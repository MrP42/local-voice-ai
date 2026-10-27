//! Eigene Phonon-Stimmen: importieren, entfernen und auf die installierten Modelle verteilen.
//!
//! Eine Phonon-Stimme ist eine kleine `.safetensors`-Datei (Tensor `speaker_wavs`, F32
//! [1, 512, 125], 256 136 Byte). Der Adapter liest sie aus `<Modell>/voices/<Name>.safetensors`;
//! die Anzeige (Name, Beschreibung, Lizenz) kommt aus `voices/SOURCE.json`. Eigene Stimmen
//! (Import, Gradium-Klon) bekommen dort eine Zeile mit `origin`, Lizenz „eigene/unbekannt“ und
//! `verified: false`.
//!
//! ## Eigener Speicher als Quelle der Wahrheit
//!
//! Die Modellseite löscht beim Entfernen eines Modells den ganzen Ordner `models/<id>/` — samt
//! `voices/`. Eine selbst erstellte Stimme (Einwilligung, Gradium-Aufruf) soll daran nicht
//! hängen. Deshalb liegt jede eigene Stimme zuerst in `<phonon>/eigene-stimmen/` (mit
//! `stimmen.json`) und wird von dort in `voices/` jedes installierten Modells KOPIERT. [`VoiceStore::sync`]
//! gleicht ab und heilt: fehlende Dateien, fehlende Zeilen, und Zeilen, deren Zusatzfelder die
//! Modellseite beim typisierten Umschreiben von `SOURCE.json` verloren hat. Wo eine fremde Datei
//! gleichen Namens liegt (andere Bytes, Zeile ohne Herkunft), wird sie NIE überschrieben,
//! sondern als Konflikt gemeldet.
//!
//! ## Fehlerfälle und ihre Absicherung
//!
//! | Fall | Absicherung |
//! |---|---|
//! | Parallele Importe / Modellinstallation | eine gemeinsame Sperre (`TtsModelManager::phonon_voices_lock`); Prüfen und Schreiben geschehen unter ihr; Tests `parallele_importe_…`, `gleicher_name_parallel_…` |
//! | Abbruch mitten im Schreiben | jede Datei erst als `.partial` (`sync_all`), dann `rename`; Stimmdatei zuerst, `SOURCE.json`-Zeile danach (auch atomar); eine Datei ohne Zeile ist eine gültige Stimme, der Abgleich ergänzt die Zeile; Test `scheitert_das_schreiben_…` |
//! | Voller Datenträger / nicht beschreibbar | Fehlertext ohne Pfad, `.partial` wird gelöscht, die bisherige Stimme bleibt; ein unbeschreibbares Modell stört die anderen nicht; Tests `scheitert_das_schreiben_…`, `ein_unbeschreibbares_modell_…` |
//! | Untergeschobene Datei | nur gültiges safetensors (Kopf, Offsets, lückenlose Daten, `speaker_wavs`), Größe vor dem Lesen begrenzt, nichts wird ausgeführt; Tests `eine_echte_form_…`, `der_dateiimport_prueft_…` |
//! | Pfadzeichen im Namen | Namen werden zu `[A-Za-z0-9_-]` gefaltet, Windows-reservierte Namen und `default` abgelehnt; Test `namen_werden_geprueft_…` |
//! | Mitgelieferte Stimme überschreiben/entfernen | nie, auch nicht mit Zustimmung; Tests `namen_der_modellstimmen_…`, `mitgelieferte_stimmen_…` |
//! | `SOURCE.json` kaputt | wird nicht angefasst (die Stimme ist trotzdem nutzbar), Test `eine_kaputte_source_json_…` |
//! | `stimmen.json` kaputt | gesichert als `.defekt`, aus den Dateien (und den Zeilen in den Modellen) neu aufgebaut; Test `ein_defekter_index_…` |

use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use specta::Type;

use super::providers::contract::LicenseInfo;

/// Herkunft: eine fertige Stimmdatei wurde importiert.
pub const ORIGIN_IMPORT: &str = "import";
/// Herkunft: mit Gradium (`scripts/clone-voice.py`-Weg) aus einer Aufnahme erstellt.
pub const ORIGIN_GRADIUM: &str = "gradium-clone";
/// Lizenzangabe eigener Stimmen: nie verifiziert, Rechte liegen beim Nutzer.
pub const OWN_LICENSE: &str = "eigene/unbekannt";
/// Ordner des eigenen Speichers unter `<app_data>/tts/phonon/`.
pub const STORE_DIR: &str = "eigene-stimmen";
const INDEX_FILE: &str = "stimmen.json";
const SOURCE_FILE: &str = "SOURCE.json";
/// Echte Stimmen: 256 136 Byte. Darunter ist es keine Stimme, darüber auch nicht.
pub const MIN_VOICE_BYTES: u64 = 16 * 1024;
pub const MAX_VOICE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_HEADER_BYTES: u64 = 64 * 1024;
/// Der Adapter liest `SOURCE.json` nur bis 512 KiB; mit 200 eigenen Zeilen (je rund 600 Byte)
/// bleibt die Datei weit darunter.
pub const MAX_OWN_VOICES: usize = 200;
const MAX_SOURCE_BYTES: u64 = 400 * 1024;
const MAX_NAME_CHARS: usize = 40;

// ---- Fehler ------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceError {
    /// Der Name ist unbrauchbar (Meldung sagt warum).
    InvalidName(String),
    /// Es gibt schon eine eigene Stimme dieses Namens; Überschreiben braucht Zustimmung.
    Exists {
        id: String,
    },
    /// Der Name gehört zu einer mitgelieferten Stimme des Modells.
    TakenByModel {
        id: String,
    },
    /// Die Datei ist keine gültige Phonon-Stimme.
    InvalidFile(String),
    /// Keine eigene Stimme (entfernen).
    NotOwn(String),
    TooMany,
    Io(String),
}

impl fmt::Display for VoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(m) | Self::InvalidFile(m) | Self::Io(m) => f.write_str(m),
            Self::Exists { id } => write!(
                f,
                "Es gibt schon eine eigene Stimme mit diesem Namen („{id}“)."
            ),
            Self::TakenByModel { id } => write!(
                f,
                "Der Name „{id}“ gehört zu einer mitgelieferten Stimme des Modells; \
                 bitte einen anderen Namen wählen."
            ),
            Self::NotOwn(id) => write!(f, "„{id}“ ist keine eigene Stimme."),
            Self::TooMany => write!(
                f,
                "Es sind höchstens {MAX_OWN_VOICES} eigene Stimmen möglich."
            ),
        }
    }
}

impl std::error::Error for VoiceError {}

fn io_err(e: &std::io::Error) -> VoiceError {
    // Windows ERROR_DISK_FULL (112) und POSIX ENOSPC (28); sonst nur die Art, nie ein Pfad.
    let text = match (e.raw_os_error(), e.kind()) {
        (Some(112 | 28), _) => "Der Datenträger ist voll.".to_string(),
        (_, std::io::ErrorKind::PermissionDenied) => {
            "Kein Schreibzugriff auf den Stimmenordner.".to_string()
        }
        _ => format!("Der Stimmenordner lässt sich nicht beschreiben ({e})."),
    };
    VoiceError::Io(text)
}

/// Ist `origin` die Herkunft einer eigenen Stimme (Import, Klon)?
pub fn origin_is_own(origin: &str) -> bool {
    matches!(origin, ORIGIN_IMPORT | ORIGIN_GRADIUM)
}

/// Lizenzangabe einer eigenen Stimme fuer die Stimmenliste: eigene/unbekannt, nie verifiziert,
/// vorsorglich nichtkommerziell (wie jede ungeprueft eingebrachte Stimme, siehe
/// `docs/tts/licenses.md`). Die Lizenz des Modells gilt nicht fuer sie.
pub fn own_voice_license(origin: &str) -> LicenseInfo {
    let what = if origin == ORIGIN_GRADIUM {
        "mit Gradium aus einer Aufnahme erstellt; zusätzlich gelten die Bedingungen von Gradium"
    } else {
        "importiert"
    };
    LicenseInfo {
        code: OWN_LICENSE.to_string(),
        weights: OWN_LICENSE.to_string(),
        non_commercial: true,
        verified: false,
        source_url: String::new(),
        verified_on: None,
        note: format!(
            "Eigene Stimme ({what}). Rechte an der Stimme und die Einwilligung der sprechenden \
             Person liegen beim Nutzer; die App prüft sie nicht."
        ),
    }
}

pub fn check_capacity(count: usize, replacing: bool) -> Result<(), VoiceError> {
    if !replacing && count >= MAX_OWN_VOICES {
        return Err(VoiceError::TooMany);
    }
    Ok(())
}

// ---- Namen ---------------------------------------------------------------------------------

/// Anzeigename (frei) und Kennung (Dateiname ohne Endung; zugleich die Stimmenkennung im Adapter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceName {
    pub display: String,
    pub id: String,
}

fn fold(c: char) -> Option<&'static str> {
    Some(match c {
        'ä' => "ae",
        'Ä' => "Ae",
        'ö' => "oe",
        'Ö' => "Oe",
        'ü' => "ue",
        'Ü' => "Ue",
        'ß' => "ss",
        'æ' => "ae",
        'Æ' => "Ae",
        'œ' => "oe",
        'Œ' => "Oe",
        'à' | 'á' | 'â' | 'ã' | 'å' | 'ā' => "a",
        'À' | 'Á' | 'Â' | 'Ã' | 'Å' => "A",
        'è' | 'é' | 'ê' | 'ë' => "e",
        'È' | 'É' | 'Ê' | 'Ë' => "E",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'Ì' | 'Í' | 'Î' | 'Ï' => "I",
        'ò' | 'ó' | 'ô' | 'õ' | 'ø' => "o",
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ø' => "O",
        'ù' | 'ú' | 'û' => "u",
        'Ù' | 'Ú' | 'Û' => "U",
        'ý' | 'ÿ' => "y",
        'Ý' => "Y",
        'ñ' => "n",
        'Ñ' => "N",
        'ç' => "c",
        'Ç' => "C",
        _ => return None,
    })
}

fn is_reserved(id: &str) -> bool {
    let up = id.to_ascii_uppercase();
    matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "DEFAULT")
        || ((up.starts_with("COM") || up.starts_with("LPT"))
            && up.len() == 4
            && up.as_bytes()[3].is_ascii_digit())
}

/// Name prüfen und zur Kennung falten (siehe Moduldoku).
pub fn parse_name(raw: &str) -> Result<VoiceName, VoiceError> {
    let bad = |m: &str| Err(VoiceError::InvalidName(m.to_string()));
    let raw = raw.trim();
    if raw.is_empty() {
        return bad("Bitte einen Namen eingeben.");
    }
    if raw.chars().any(char::is_control) {
        return bad("Der Name enthält Steuerzeichen.");
    }
    let display = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if display.chars().count() > MAX_NAME_CHARS {
        return bad(&format!(
            "Der Name darf höchstens {MAX_NAME_CHARS} Zeichen lang sein."
        ));
    }
    if !display
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '\'' | '(' | ')'))
    {
        return bad(
            "Der Name enthält unzulässige Zeichen (erlaubt: Buchstaben, Ziffern, Leerzeichen und - _ . ' ( )).",
        );
    }
    if display.starts_with('.') || display.ends_with('.') {
        return bad("Der Name darf nicht mit einem Punkt beginnen oder enden.");
    }
    let mut id = String::new();
    for c in display.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            id.push(c);
        } else if let Some(f) = fold(c) {
            id.push_str(f);
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id: String = id
        .trim_matches(|c| c == '-' || c == '_')
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    let id = id.trim_matches(|c| c == '-' || c == '_').to_string();
    if id.is_empty() {
        return bad("Der Name braucht mindestens einen Buchstaben (A–Z) oder eine Ziffer.");
    }
    if is_reserved(&id) {
        return bad(
            "Dieser Name ist vom System oder von Phonon belegt; bitte einen anderen wählen.",
        );
    }
    Ok(VoiceName { display, id })
}

// ---- Prüfung der Stimmdatei ----------------------------------------------------------------------

fn dtype_size(dtype: &str) -> Option<u64> {
    Some(match dtype {
        "F64" | "I64" | "U64" => 8,
        "F32" | "I32" | "U32" => 4,
        "F16" | "BF16" | "I16" | "U16" => 2,
        "I8" | "U8" | "BOOL" | "F8_E4M3" | "F8_E5M2" => 1,
        _ => return None,
    })
}

/// Ist das eine gültige Phonon-Stimme? Es wird nur der Kopf gelesen und gerechnet, nie etwas
/// ausgeführt: safetensors-Kopf lesbar, jeder Tensor mit gültigen Offsets und passender Größe,
/// die Daten lückenlos ohne Anhang, und der Tensor `speaker_wavs` in der Form [1, 512, T].
pub fn validate_voice_bytes(bytes: &[u8]) -> Result<(), VoiceError> {
    let bad = |m: &str| Err(VoiceError::InvalidFile(m.to_string()));
    let len = bytes.len() as u64;
    if len < MIN_VOICE_BYTES {
        return bad("Die Datei ist zu klein für eine Phonon-Stimme (erwartet etwa 256 KB).");
    }
    if len > MAX_VOICE_BYTES {
        return bad("Die Datei ist zu groß für eine Phonon-Stimme (erwartet etwa 256 KB).");
    }
    let mut first = [0u8; 8];
    first.copy_from_slice(&bytes[..8]);
    let header_len = u64::from_le_bytes(first);
    if header_len < 2 || header_len > MAX_HEADER_BYTES || 8 + header_len > len {
        return bad("Das ist keine safetensors-Datei (Kopf unlesbar).");
    }
    let header = &bytes[8..8 + header_len as usize];
    let Ok(Value::Object(tensors)) = serde_json::from_slice::<Value>(header) else {
        return bad("Das ist keine safetensors-Datei (Kopf ist kein JSON-Objekt).");
    };
    let data_len = len - 8 - header_len;
    let mut spans: Vec<(u64, u64)> = Vec::new();
    let mut has_speaker = false;
    for (key, tensor) in &tensors {
        if key == "__metadata__" {
            let ok = tensor
                .as_object()
                .is_some_and(|m| m.values().all(Value::is_string));
            if !ok {
                return bad("Die Metadaten der Datei sind ungültig.");
            }
            continue;
        }
        let Some(t) = tensor.as_object() else {
            return bad("Ein Tensor-Eintrag der Datei ist ungültig.");
        };
        let Some(size) = t.get("dtype").and_then(Value::as_str).and_then(dtype_size) else {
            return bad("Die Datei enthält einen unbekannten Datentyp.");
        };
        let shape: Option<Vec<u64>> = t
            .get("shape")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(Value::as_u64).collect::<Option<Vec<_>>>())
            .unwrap_or(None);
        let offsets: Option<Vec<u64>> = t
            .get("data_offsets")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(Value::as_u64).collect::<Option<Vec<_>>>())
            .unwrap_or(None);
        let (Some(shape), Some(offsets)) = (shape, offsets) else {
            return bad("Ein Tensor-Eintrag der Datei ist ungültig.");
        };
        if offsets.len() != 2 {
            return bad("Ein Tensor-Eintrag der Datei ist ungültig.");
        }
        let (begin, end) = (offsets[0], offsets[1]);
        if begin > end || end > data_len {
            return bad("Die Datei ist unvollständig oder beschädigt (Daten außerhalb der Datei).");
        }
        let numel = shape.iter().try_fold(1u64, |acc, &d| acc.checked_mul(d));
        if numel.and_then(|n| n.checked_mul(size)) != Some(end - begin) {
            return bad("Die Datei ist beschädigt (Größe passt nicht zur Form).");
        }
        spans.push((begin, end));
        if key == "speaker_wavs" {
            let float = matches!(
                t.get("dtype").and_then(Value::as_str),
                Some("F32" | "F16" | "BF16" | "F64")
            );
            if !float || shape.len() != 3 || shape[0] != 1 || shape[1] != 512 || shape[2] == 0 {
                return bad(
                    "Der Tensor „speaker_wavs“ hat nicht die Form einer Phonon-Stimme ([1, 512, …]).",
                );
            }
            has_speaker = true;
        }
    }
    if !has_speaker {
        return bad("Der Tensor „speaker_wavs“ fehlt – das ist keine Phonon-Stimme.");
    }
    spans.sort_unstable();
    let mut expected = 0u64;
    for (begin, end) in spans {
        if begin != expected {
            return bad("Die Datei ist beschädigt (Lücke oder Überlappung in den Daten).");
        }
        expected = end;
    }
    if expected != data_len {
        return bad("Die Datei enthält zusätzliche, unerwartete Daten.");
    }
    Ok(())
}

/// Datei lesen und prüfen. Die Größe wird vor dem Lesen geprüft (nie eine riesige Datei laden).
fn read_voice_file(path: &Path) -> Result<Vec<u8>, VoiceError> {
    let bad = |m: &str| Err(VoiceError::InvalidFile(m.to_string()));
    let is_st = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("safetensors"));
    if !is_st {
        return bad("Bitte eine Datei mit der Endung .safetensors wählen.");
    }
    let Ok(meta) = fs::metadata(path) else {
        return bad("Die Datei lässt sich nicht öffnen.");
    };
    if !meta.is_file() {
        return bad("Das ist keine Datei.");
    }
    if meta.len() > MAX_VOICE_BYTES {
        return bad("Die Datei ist zu groß für eine Phonon-Stimme (erwartet etwa 256 KB).");
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    let read = fs::File::open(path).and_then(|f| f.take(MAX_VOICE_BYTES + 1).read_to_end(&mut buf));
    if read.is_err() {
        return bad("Die Datei lässt sich nicht lesen.");
    }
    validate_voice_bytes(&buf)?;
    Ok(buf)
}

// ---- Dateien ---------------------------------------------------------------------------------------

fn partial_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dest.with_file_name(format!("{name}.partial"))
}

/// Erst in `<ziel>.partial`, `sync_all`, dann umbenennen: ein Abbruch hinterlässt den alten
/// Stand oder nichts, nie eine halbe Datei. Bei einem Fehler wird der Rest gelöscht.
fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<(), VoiceError> {
    let tmp = partial_path(dest);
    let result = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, dest)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(io_err(&e));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> Option<String> {
    let mut buf = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_VOICE_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    Some(sha256_hex(&buf))
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---- Datensätze -----------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    id: String,
    name: String,
    origin: String,
    #[serde(default)]
    voice_id: String,
    #[serde(default)]
    language: String,
    created_at: String,
    #[serde(default)]
    consent_at: Option<String>,
    #[serde(default)]
    source_note: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    voices: Vec<Record>,
}

/// Eine eigene Stimme für die Oberfläche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct OwnVoice {
    /// Dateiname ohne Endung: die Stimmenkennung im Adapter.
    pub id: String,
    /// Anzeigename, wie eingegeben.
    pub name: String,
    /// `import` oder `gradium-clone`.
    pub origin: String,
    /// Kennung bei Gradium (leer beim Import).
    pub voice_id: String,
    /// Sprache der Aufnahme (`de`, `en`, …); leer beim Import.
    pub language: String,
    pub created_at: String,
    /// Zeitpunkt der bestätigten Einwilligung (Gradium-Klon).
    pub consent_at: Option<String>,
    /// Woraus die Stimme entstand (z. B. „Fish-Stimme: erzaehler“).
    pub source_note: Option<String>,
    /// Immer „eigene/unbekannt“: Rechte und Einwilligung liegen beim Nutzer.
    pub license: String,
    /// Immer `false`: die App prüft keine Lizenz eigener Stimmen.
    pub verified: bool,
    /// Kennungen der Modelle, in denen die Datei liegt.
    pub in_models: Vec<String>,
}

/// Eine neue Stimme (Bytes sind noch ungeprüft).
pub struct NewVoice {
    pub name: String,
    pub bytes: Vec<u8>,
    pub origin: &'static str,
    pub voice_id: String,
    pub language: String,
    pub consent_at: Option<String>,
    pub source_note: Option<String>,
}

/// Ein installiertes Modell, in dessen `voices/` die eigenen Stimmen kopiert werden.
#[derive(Debug, Clone)]
pub struct ModelTarget {
    pub id: String,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameCheck {
    pub id: String,
    pub display: String,
    /// Eine eigene Stimme dieses Namens gibt es schon (und wird ersetzt).
    pub replaces: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Dateien, die neu ins Modell kopiert wurden.
    pub copied: usize,
    /// Zeilen in `SOURCE.json`, die ergänzt oder wiederhergestellt wurden.
    pub rows_healed: usize,
    /// „<Modell>: <Kennung>“: dort liegt eine fremde Datei gleichen Namens; unberührt gelassen.
    pub conflicts: Vec<String>,
}

fn row_for(r: &Record) -> Value {
    let description = if r.origin == ORIGIN_GRADIUM {
        "Eigene Stimme (mit Gradium erstellt)"
    } else {
        "Eigene Stimme (importiert)"
    };
    // Die Pflichtfelder von `PhononVoiceMeta` (Modellseite) stehen alle drin; die Zusatzfelder
    // liest nur der Adapter und dieses Modul.
    json!({
        "filename": format!("voices/{}.safetensors", r.id),
        "name": r.name,
        "voice_id": r.voice_id,
        "language": r.language,
        "locale": "",
        "gender": null,
        "age": null,
        "description": description,
        "origin": r.origin,
        "license": OWN_LICENSE,
        "verified": false,
        "created_at": r.created_at,
        "consent_at": r.consent_at,
        "source_note": r.source_note,
    })
}

fn row_stem(row: &Value) -> Option<String> {
    let f = row.get("filename")?.as_str()?;
    Path::new(f)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
}

fn row_is_own(row: &Value) -> bool {
    matches!(
        row.get("origin").and_then(Value::as_str),
        Some(ORIGIN_IMPORT | ORIGIN_GRADIUM)
    )
}

fn row_matches(row: &Value, id: &str) -> bool {
    row_stem(row).is_some_and(|s| s.eq_ignore_ascii_case(id))
}

fn read_source(path: &Path) -> Result<Option<Value>, VoiceError> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(&e)),
    };
    if !meta.is_file() || meta.len() > MAX_SOURCE_BYTES {
        return Err(VoiceError::Io(
            "Die Stimmenliste (SOURCE.json) ist unlesbar oder zu groß.".to_string(),
        ));
    }
    let raw = fs::read(path).map_err(|e| io_err(&e))?;
    serde_json::from_slice(&raw)
        .map(Some)
        .map_err(|_| VoiceError::Io("Die Stimmenliste (SOURCE.json) ist nicht lesbar.".to_string()))
}

/// Zeile der Stimme `id` in `voices/SOURCE.json` (falls lesbar und vorhanden).
fn find_row(voices_dir: &Path, id: &str) -> Option<Value> {
    let doc = read_source(&voices_dir.join(SOURCE_FILE)).ok()??;
    doc.get("voices")?
        .as_array()?
        .iter()
        .find(|r| row_matches(r, id))
        .cloned()
}

/// Zeile eintragen oder ersetzen; `true`, wenn sich die Datei geändert hat. Alles andere in der
/// Datei (Kopffelder, mitgelieferte Zeilen, Zusatzfelder) bleibt, wie es war.
fn upsert_row(voices_dir: &Path, rec: &Record) -> Result<bool, VoiceError> {
    let path = voices_dir.join(SOURCE_FILE);
    let mut doc =
        read_source(&path)?.unwrap_or_else(|| json!({"source": "", "license": "", "voices": []}));
    let bad = || {
        VoiceError::Io("Die Stimmenliste (SOURCE.json) hat ein unerwartetes Format.".to_string())
    };
    let obj = doc.as_object_mut().ok_or_else(bad)?;
    let voices = obj
        .entry("voices")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(bad)?;
    let row = row_for(rec);
    match voices.iter().position(|r| row_matches(r, &rec.id)) {
        Some(i) => {
            if voices[i] == row {
                return Ok(false);
            }
            voices[i] = row;
        }
        None => voices.push(row),
    }
    let bytes = serde_json::to_vec_pretty(&doc).map_err(|_| bad())?;
    write_atomic(&path, &bytes)?;
    Ok(true)
}

/// Zeile entfernen; mit `only_own` nur, wenn sie Herkunft „eigen“ trägt.
fn remove_row(voices_dir: &Path, id: &str, only_own: bool) -> Result<bool, VoiceError> {
    let path = voices_dir.join(SOURCE_FILE);
    let Some(mut doc) = read_source(&path)? else {
        return Ok(false);
    };
    let Some(voices) = doc.get_mut("voices").and_then(Value::as_array_mut) else {
        return Ok(false);
    };
    let before = voices.len();
    voices.retain(|r| !(row_matches(r, id) && (!only_own || row_is_own(r))));
    if voices.len() == before {
        return Ok(false);
    }
    let bytes = serde_json::to_vec_pretty(&doc)
        .map_err(|_| VoiceError::Io("Die Stimmenliste lässt sich nicht schreiben.".to_string()))?;
    write_atomic(&path, &bytes)?;
    Ok(true)
}

#[derive(Debug, PartialEq, Eq)]
enum TargetState {
    /// Keine Datei dieses Namens im Modell.
    Missing,
    /// Die Datei gehört zu uns (Zeile mit Herkunft „eigen“ oder Bytes wie im eigenen Speicher).
    Ours,
    /// Eine fremde Datei (mitgeliefert oder von woanders): nie anfassen.
    Foreign,
}

// ---- Der Speicher ------------------------------------------------------------------------------------------------

/// Eigener Speicher plus Zielmodelle. `lock` ist die Sperre, die auch die Modellseite für ihre
/// Änderungen an `voices/SOURCE.json` nimmt.
pub struct VoiceStore<'a> {
    /// `<app_data>/tts/phonon`
    pub root: &'a Path,
    pub targets: &'a [ModelTarget],
    pub lock: &'a Mutex<()>,
}

impl VoiceStore<'_> {
    fn guard(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn store_dir(&self) -> PathBuf {
        self.root.join(STORE_DIR)
    }

    fn master_path(&self, id: &str) -> PathBuf {
        self.store_dir().join(format!("{id}.safetensors"))
    }

    fn index_path(&self) -> PathBuf {
        self.store_dir().join(INDEX_FILE)
    }

    fn voices_dir(t: &ModelTarget) -> PathBuf {
        t.dir.join("voices")
    }

    fn voice_file(t: &ModelTarget, id: &str) -> PathBuf {
        Self::voices_dir(t).join(format!("{id}.safetensors"))
    }

    // -- Index

    fn read_index(&self) -> Result<Index, VoiceError> {
        let path = self.index_path();
        let raw = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Index::default()),
            Err(e) => return Err(io_err(&e)),
        };
        match serde_json::from_slice::<Index>(&raw) {
            Ok(idx) => Ok(idx),
            Err(_) => self.rebuild_index(&path),
        }
    }

    /// Ein unlesbarer Index wird gesichert (`.defekt`) und aus den Dateien des eigenen Speichers
    /// neu aufgebaut; Name, Herkunft und Gradium-Kennung kommen, wo möglich, aus den Zeilen in den
    /// Modellen.
    fn rebuild_index(&self, path: &Path) -> Result<Index, VoiceError> {
        let backup = path.with_file_name(format!("{INDEX_FILE}.defekt"));
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup).map_err(|e| io_err(&e))?;
        let mut voices = Vec::new();
        for entry in fs::read_dir(self.store_dir())
            .into_iter()
            .flatten()
            .flatten()
        {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("safetensors") {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let row = self
                .targets
                .iter()
                .find_map(|t| find_row(&Self::voices_dir(t), stem).filter(row_is_own));
            let text = |key: &str| {
                row.as_ref()
                    .and_then(|r| r.get(key))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            voices.push(Record {
                id: stem.to_string(),
                name: text("name").unwrap_or_else(|| stem.to_string()),
                origin: text("origin").unwrap_or_else(|| ORIGIN_IMPORT.to_string()),
                voice_id: text("voice_id").unwrap_or_default(),
                language: text("language").unwrap_or_default(),
                created_at: text("created_at").unwrap_or_else(now_iso),
                consent_at: text("consent_at"),
                source_note: text("source_note"),
            });
        }
        voices.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
        let idx = Index { version: 1, voices };
        self.write_index(&idx)?;
        Ok(idx)
    }

    fn write_index(&self, idx: &Index) -> Result<(), VoiceError> {
        fs::create_dir_all(self.store_dir()).map_err(|e| io_err(&e))?;
        let bytes = serde_json::to_vec_pretty(&Index {
            version: 1,
            voices: idx.voices.clone(),
        })
        .map_err(|_| VoiceError::Io("Die Stimmenliste lässt sich nicht schreiben.".to_string()))?;
        write_atomic(&self.index_path(), &bytes)
    }

    fn view(&self, r: &Record) -> OwnVoice {
        let in_models = self
            .targets
            .iter()
            .filter(|t| Self::voice_file(t, &r.id).is_file())
            .map(|t| t.id.clone())
            .collect();
        OwnVoice {
            id: r.id.clone(),
            name: r.name.clone(),
            origin: r.origin.clone(),
            voice_id: r.voice_id.clone(),
            language: r.language.clone(),
            created_at: r.created_at.clone(),
            consent_at: r.consent_at.clone(),
            source_note: r.source_note.clone(),
            license: OWN_LICENSE.to_string(),
            verified: false,
            in_models,
        }
    }

    // -- Zielmodelle

    fn target_state(&self, t: &ModelTarget, id: &str, master_hash: Option<&str>) -> TargetState {
        let dest = Self::voice_file(t, id);
        if !dest.exists() {
            return TargetState::Missing;
        }
        if find_row(&Self::voices_dir(t), id).is_some_and(|r| row_is_own(&r)) {
            return TargetState::Ours;
        }
        if let Some(h) = master_hash {
            if sha256_file(&dest).as_deref() == Some(h) {
                return TargetState::Ours;
            }
        }
        TargetState::Foreign
    }

    /// Datei und Zeile in EIN Modell schreiben (Datei zuerst).
    fn write_to_target(
        &self,
        t: &ModelTarget,
        rec: &Record,
        bytes: &[u8],
    ) -> Result<(), VoiceError> {
        fs::create_dir_all(Self::voices_dir(t)).map_err(|e| io_err(&e))?;
        write_atomic(&Self::voice_file(t, &rec.id), bytes)?;
        upsert_row(&Self::voices_dir(t), rec)?;
        Ok(())
    }

    // -- Öffentliche Schnittstelle

    /// Alle eigenen Stimmen (nach Namen sortiert) mit den Modellen, in denen sie liegen.
    pub fn list(&self) -> Vec<OwnVoice> {
        let _g = self.guard();
        match self.read_index() {
            Ok(idx) => {
                let mut out: Vec<OwnVoice> = idx.voices.iter().map(|r| self.view(r)).collect();
                out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                out
            }
            Err(e) => {
                log::warn!("eigene Phonon-Stimmen nicht lesbar: {e}");
                Vec::new()
            }
        }
    }

    fn check_locked(
        &self,
        raw: &str,
        overwrite: bool,
        idx: &Index,
    ) -> Result<NameCheck, VoiceError> {
        let name = parse_name(raw)?;
        if let Some(r) = idx
            .voices
            .iter()
            .find(|r| r.id.eq_ignore_ascii_case(&name.id))
        {
            if !overwrite {
                return Err(VoiceError::Exists { id: r.id.clone() });
            }
            return Ok(NameCheck {
                id: r.id.clone(),
                display: name.display,
                replaces: true,
            });
        }
        // Neu: der Name darf in keinem Modell schon belegt sein (Datei oder fremde Zeile).
        for t in self.targets {
            let file_taken = Self::voice_file(t, &name.id).exists();
            let row_taken = find_row(&Self::voices_dir(t), &name.id).is_some();
            if file_taken || row_taken {
                return Err(VoiceError::TakenByModel { id: name.id });
            }
        }
        Ok(NameCheck {
            id: name.id,
            display: name.display,
            replaces: false,
        })
    }

    /// Namensprobe vor dem Netzzugriff oder Kopieren: gültig, frei oder (mit `overwrite`)
    /// ersetzbar? Verändert nichts.
    pub fn check_name(&self, raw: &str, overwrite: bool) -> Result<NameCheck, VoiceError> {
        let _g = self.guard();
        let idx = self.read_index()?;
        self.check_locked(raw, overwrite, &idx)
    }

    /// Eine neue eigene Stimme ablegen: erst prüfen, dann in den eigenen Speicher, dann in jedes
    /// Modell. Ein Modell, das sich nicht beschreiben lässt, wird übersprungen (die Stimme bleibt
    /// im eigenen Speicher; `in_models` zeigt, wo sie liegt).
    pub fn add(&self, new: NewVoice, overwrite: bool) -> Result<OwnVoice, VoiceError> {
        validate_voice_bytes(&new.bytes)?;
        let _g = self.guard();
        let mut idx = self.read_index()?;
        let check = self.check_locked(&new.name, overwrite, &idx)?;
        check_capacity(idx.voices.len(), check.replaces)?;
        let id = check.id;
        let master = self.master_path(&id);
        let previous = check.replaces.then(|| sha256_file(&master)).flatten();

        fs::create_dir_all(self.store_dir()).map_err(|e| io_err(&e))?;
        write_atomic(&master, &new.bytes)?;
        let rec = Record {
            id: id.clone(),
            name: check.display,
            origin: new.origin.to_string(),
            voice_id: new.voice_id,
            language: new.language,
            created_at: now_iso(),
            consent_at: new.consent_at,
            source_note: new.source_note,
        };
        idx.voices.retain(|r| !r.id.eq_ignore_ascii_case(&id));
        idx.voices.push(rec.clone());
        if let Err(e) = self.write_index(&idx) {
            if !check.replaces {
                let _ = fs::remove_file(&master);
            }
            return Err(e);
        }
        for t in self.targets {
            match self.target_state(t, &id, previous.as_deref()) {
                TargetState::Foreign => log::warn!(
                    "Phonon-Stimme {id}: im Modell {} liegt eine fremde Datei gleichen Namens; unberührt",
                    t.id
                ),
                _ => {
                    if let Err(e) = self.write_to_target(t, &rec, &new.bytes) {
                        log::warn!("Phonon-Stimme {id} nicht in Modell {} abgelegt: {e}", t.id);
                    }
                }
            }
        }
        Ok(self.view(&rec))
    }

    /// Stimmdatei lesen, prüfen und als eigene Stimme ablegen (Import).
    pub fn import_file(
        &self,
        path: &Path,
        raw_name: &str,
        overwrite: bool,
    ) -> Result<OwnVoice, VoiceError> {
        let bytes = read_voice_file(path)?;
        self.add(
            NewVoice {
                name: raw_name.to_string(),
                bytes,
                origin: ORIGIN_IMPORT,
                voice_id: String::new(),
                language: String::new(),
                consent_at: None,
                source_note: None,
            },
            overwrite,
        )
    }

    /// Eine EIGENE Stimme entfernen (Datei und Zeile in jedem Modell, dann der eigene Speicher).
    /// Mitgelieferte Stimmen und unbekannte Kennungen lehnt das mit `NotOwn` ab, ohne einen Pfad
    /// zu bilden.
    pub fn remove(&self, id: &str) -> Result<(), VoiceError> {
        let _g = self.guard();
        let mut idx = self.read_index()?;
        let Some(rec) = idx
            .voices
            .iter()
            .find(|r| r.id.eq_ignore_ascii_case(id.trim()))
            .cloned()
        else {
            return Err(VoiceError::NotOwn(id.to_string()));
        };
        let master = self.master_path(&rec.id);
        let hash = sha256_file(&master);
        let mut first_err: Option<VoiceError> = None;
        let mut note = |e: VoiceError| {
            first_err.get_or_insert(e);
        };
        for t in self.targets {
            match self.target_state(t, &rec.id, hash.as_deref()) {
                TargetState::Foreign => {}
                TargetState::Missing => {
                    if let Err(e) = remove_row(&Self::voices_dir(t), &rec.id, true) {
                        note(e);
                    }
                }
                TargetState::Ours => {
                    match fs::remove_file(Self::voice_file(t, &rec.id)) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => note(io_err(&e)),
                    }
                    if let Err(e) = remove_row(&Self::voices_dir(t), &rec.id, false) {
                        note(e);
                    }
                }
            }
        }
        if let Some(e) = first_err {
            // Der Eintrag im eigenen Speicher bleibt: ein zweiter Versuch räumt auf.
            return Err(e);
        }
        idx.voices.retain(|r| !r.id.eq_ignore_ascii_case(&rec.id));
        self.write_index(&idx)?;
        match fs::remove_file(&master) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err(&e)),
        }
    }

    /// Eigenen Speicher und Modelle abgleichen (idempotent): fehlende Dateien und Zeilen werden
    /// ergänzt, Zeilen mit verlorenen Zusatzfeldern wiederhergestellt, Fremdes bleibt unberührt.
    pub fn sync(&self) -> SyncReport {
        let _g = self.guard();
        let mut report = SyncReport::default();
        let Ok(idx) = self.read_index() else {
            return report;
        };
        for rec in &idx.voices {
            let Some(bytes) = fs::read(self.master_path(&rec.id))
                .ok()
                .filter(|b| b.len() as u64 <= MAX_VOICE_BYTES)
            else {
                log::warn!(
                    "eigene Phonon-Stimme {}: Datei im eigenen Speicher fehlt",
                    rec.id
                );
                continue;
            };
            let hash = sha256_hex(&bytes);
            for t in self.targets {
                match self.target_state(t, &rec.id, Some(&hash)) {
                    TargetState::Foreign => report.conflicts.push(format!("{}: {}", t.id, rec.id)),
                    TargetState::Missing => match self.write_to_target(t, rec, &bytes) {
                        Ok(()) => report.copied += 1,
                        Err(e) => log::warn!("Abgleich {} in {}: {e}", rec.id, t.id),
                    },
                    TargetState::Ours => {
                        let same = sha256_file(&Self::voice_file(t, &rec.id)).as_deref()
                            == Some(hash.as_str());
                        if !same {
                            match write_atomic(&Self::voice_file(t, &rec.id), &bytes) {
                                Ok(()) => report.copied += 1,
                                Err(e) => {
                                    log::warn!("Abgleich {} in {}: {e}", rec.id, t.id);
                                    continue;
                                }
                            }
                        }
                        match upsert_row(&Self::voices_dir(t), rec) {
                            Ok(true) => report.rows_healed += 1,
                            Ok(false) => {}
                            Err(e) => log::warn!("Abgleich {} in {}: {e}", rec.id, t.id),
                        }
                    }
                }
            }
        }
        report
    }
}

/// Eine Stimmdatei in der Form der echten (`speaker_wavs` F32 [1, 512, 125], 256 136 Byte); `seed`
/// macht die Bytes verschieden. Nur fuer Tests (auch die des Gradium-Ablaufs).
#[cfg(test)]
pub(crate) fn sample_voice_bytes(seed: u32) -> Vec<u8> {
    let t = 125usize;
    let n = 512 * t;
    let header = format!(
        r#"{{"speaker_wavs":{{"dtype":"F32","shape":[1,512,{t}],"data_offsets":[0,{}]}},"__metadata__":{{"sig":"ba7a5a45","epoch":"300"}}}}"#,
        n * 4
    );
    let mut h = header.into_bytes();
    while h.len() % 8 != 0 {
        h.push(b' ');
    }
    let mut out = (h.len() as u64).to_le_bytes().to_vec();
    out.extend(h);
    for i in 0..n {
        out.extend(((seed as f32 * 7.0 + i as f32 * 0.001).sin()).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests;
