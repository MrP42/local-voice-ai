//! Phonon (Gradium) über das offizielle CLI `ptts` (Paket P2, Goal tts-provider).
//!
//! Der Adapter startet je Satz `ptts --model <m> --lang <l> [--quant q8]
//! [--voice <v>] [--threads <n>] "<text>" -o <tmp>/out.wav`, liest die WAV-Datei
//! und räumt auf. Quelle der Optionen: `docs/cli.md` des Repos
//! `gradium-ai/xn-ptts` (v0.4.2, abgerufen 26.10.2026). Streaming: `Sentence`
//! (das CLI schreibt nur Dateien). Gerät: CPU — laut `cli.md` laufen die
//! fertigen Downloads nur auf der CPU und `--quant q8` immer; CUDA wird nicht
//! behauptet.
//!
//! ## Entscheidung: `std::process` mit eigenem Kill-on-Drop statt `tokio::process`
//!
//! Der Systemschutz der App verlangt für jeden Kindprozess das Job-Objekt aus
//! `process_guard` (RAM-/CPU-Deckel, Kill beim Beenden der App).
//! `ProcessGuard::attach` nimmt ein `std::process::Child`; ein
//! `tokio::process::Child` ließe sich dort nicht einhängen. Darum startet der
//! Adapter mit `std::process::Command`, wartet asynchron per `try_wait`-Schleife
//! (4 bis 40 ms Takt) und kapselt den Prozess in [`ChildGuard`], dessen `Drop`
//! ihn beendet und einsammelt. Fallen `guarded` (Abbruch/Frist) oder ein
//! Task-Abbruch das Future, endet der Prozess sofort — ohne neue
//! Cargo-Abhängigkeit oder tokio-Feature. Der Start selbst ist ein kurzer,
//! blockierender Aufruf (Millisekunden); es gibt keinen `await` zwischen
//! `spawn` und der Übergabe an den Wächter, ein Abbruch kann also keinen
//! Prozess verwaisen lassen.
//!
//! stdout/stderr gehen in Dateien des Arbeitsordners (keine Pipes, also kein
//! Verklemmen an vollen Puffern und keine Lesethreads); gelesen werden nur die
//! letzten 4 KiB.
//!
//! ## Text als Argument
//!
//! Kein Shell-Aufruf, `Command::arg` quotet nach den Windows-Regeln
//! (`CommandLineToArgvW`); Anführungszeichen, Backslashes, Umlaute und Emojis
//! kommen unverändert an (Test `text_kommt_unveraendert_an`). Steuerzeichen
//! (auch Zeilenumbrüche, NUL) werden zu Leerzeichen — NUL würde den Start
//! scheitern lassen. `docs/cli.md` sagt nichts zu `--`; darum beginnt ein Text,
//! der mit `-` oder `@` anfängt, mit EINEM Leerzeichen. Das ist für jedes CLI
//! sicher (kein Optionsanfang, kein `@datei`) und für die Sprachausgabe
//! unhörbar. Höchstens 8000 Zeichen (Kommandozeilen-Grenze 32 767 Zeichen).
//!
//! ## Datenschutz und Downloads
//!
//! Standard: der Kindprozess bekommt `HF_HUB_OFFLINE=1` (und `HF_HOME`, falls
//! konfiguriert) — keine stillen Downloads. `docs/cli.md` dokumentiert
//! `HF_HUB_OFFLINE` NICHT; darum verlässt sich der Adapter nicht darauf: bei
//! einem `hf://`-Modell prüft er VOR dem Start, ob der Hugging-Face-Cache
//! (`HF_HUB_CACHE`, sonst `<HF_HOME>/hub`, sonst `~/.cache/huggingface/hub`)
//! den Schnappschuss des Modells enthält, und endet sonst mit `Unavailable`,
//! ohne `ptts` zu starten. Erst `allow_network = true` entfernt die Sperre.
//! Das ist eine Annahme über das Cache-Layout (Python-`huggingface_hub`,
//! `models--OWNER--NAME/snapshots/<commit>`), die für das Rust-Tool nicht
//! belegt ist; trifft sie nicht zu, bleibt das Ergebnis sicher (Modell
//! „fehlt“), nie ein stiller Download.
//!
//! ## Fehlerfälle und ihre Absicherung
//!
//! | Fall | Absicherung |
//! |---|---|
//! | Gleichzeitige Aufträge | je Auftrag eigener Arbeitsordner und Kindprozess, kein gemeinsamer veränderlicher Zustand; Test `parallele_auftraege_…` |
//! | Abbruch/Frist mitten im Lauf | `guarded` lässt das Future fallen, [`ChildGuard`] beendet den Prozess, Arbeitsordner verschwindet; Tests `abbruch_beendet_den_kindprozess…`, `frist_…`, Marker-Datei des Fakes |
//! | Voller Speicher | Start-Gate `process_guard::check_ram_for_start` → `Unavailable` ohne Start; Job-Objekt-Deckel (CPU) via `ProcessGuard`; Ausgabedatei über 256 MiB → `Backend`; Test `ram_gate_…`, `zu_grosse_ausgabe_…` |
//! | Voller Datenträger | Arbeitsordner nicht anlegbar → `Backend` vor dem Start; abgeschnittene WAV → `Backend`; Test `arbeitsordner_…` |
//! | Fehlendes Programm/Modell | `Unavailable` mit Grund und Hinweis, ohne Prozessstart; Tests `fehlende_exe_…`, `modellpruefung_…` |
//! | Absturz/Fehlercode des Kindprozesses | Exit ≠ 0 → `Backend` (mit stderr-Auszug), gesperrtes Modell → `LicenseBlocked`, fehlendes Modell → `Unavailable`; Exit 0 ohne/mit kaputter/leerer Datei → `Backend`; Test `fehlschlaege_…` |
//! | Hänger des Kindprozesses | Frist (`opts.timeout`) → `Timeout` und Kill; `ptts --version` im Health mit 5-s-Frist |
//! | Stille Downloads | `HF_HUB_OFFLINE=1`, Cache-Vorprüfung, `allow_network` standardmäßig aus; Tests `offline_…` |
//! | Stille Downloads über `--voice` | ohne `allow_network` nur `default`, Stimmen aus `<Modell>/voices/` und vorhandene `.safetensors`-Pfade; jeder andere Name endet vor dem Start mit `Unavailable` und Hinweis (die Gesundheit meldet es für die konfigurierte Stimme früh); Tests `ohne_netzfreigabe_sind_nur_default_…` |
//! | Katalog hängt | `--list-voices` hat 7 s (unter den 10 s der Oberfläche); danach kommt die lokale Teilliste; Test `haengt_der_stimmenkatalog_…` |

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use specta::Type;
use tempfile::TempDir;

use super::contract::{
    guarded, language_matches, AudioClip, AudioEncoding, CancelToken, Device, LicenseInfo,
    ProviderCaps, ProviderDescriptor, ProviderHealth, ProviderTier, StreamingMode,
    SynthesisOptions, TtsError, TtsErrorKind, TtsProvider, UseSubject, VoiceDescriptor,
};
use crate::managers::tts::phonon_models;
use crate::managers::tts::phonon_voices;
use crate::process_guard::{self, ProcessGuard};

pub const PHONON_ID: &str = "phonon";
/// Frist für `ptts --version` im Health-Check.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
/// Frist für `ptts --list-voices` (Katalog aus dem Netz). Sie liegt UNTER der
/// äußeren Frist der Stimmenliste (Oberfläche und Agentenbrücke: je 10 s; ein
/// Test hält das fest): läuft sie ab, liefert `list_voices` die lokale Teilliste,
/// statt dass die äußere Frist die ganze Antwort verwirft.
pub(crate) const LIST_VOICES_TIMEOUT: Duration = Duration::from_secs(7);
/// Obergrenze des Texts je Aufruf (Zeichen).
const MAX_TEXT_CHARS: usize = 8000;
/// Obergrenze der Ausgabedatei (ein Satz ist nie so groß).
const MAX_WAV_BYTES: u64 = 256 * 1024 * 1024;
/// So viel Ausgabe des Kindprozesses wird gelesen (Ende der Datei).
const LOG_TAIL_BYTES: u64 = 4096;
/// Grobe RAM-Schätzung für einen ptts-Lauf (Modell 75 bis 150 MB plus Laufzeit),
/// großzügig; das Start-Gate addiert die Systemreserve.
const RAM_NEED_MB: u64 = 1024;
const LANGS: [&str; 5] = ["de", "en", "fr", "es", "pt"];
const REPO_URL: &str = "https://github.com/gradium-ai/xn-ptts";
/// Datum der Recherche (`docs/tts/provider-research.md`).
const VERIFIED_ON: &str = "2026-10-26";

/// Stabiler, von der Rust-Version unabhängiger Hash (FNV-1a, 64 Bit) für
/// Cache-Schlüssel und Dateinamen.
pub(crate) fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ---- Konfiguration -----------------------------------------------------------

/// Einstellungen des Phonon-Adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct PhononConfig {
    /// Pfad zur `ptts`-Programmdatei (oder ein Name im `PATH`).
    pub exe_path: String,
    /// Lokaler Modellordner (mit `config.json`, `tokenizer.json`, Gewichten)
    /// oder `hf://OWNER/MODEL[@REVISION]`.
    pub model: String,
    /// `None` (f32) oder `"q8"` (q8-GGUF, läuft immer auf der CPU).
    pub quant: Option<String>,
    /// Sprache der Textnormalisierung: de, en, fr, es, pt oder none.
    pub lang: String,
    /// Stimme (Name, `.safetensors`-Pfad); `None` = Standardstimme des Modells.
    pub voice: Option<String>,
    /// Rechen-Threads (`--threads`); `None` = Vorgabe des Programms.
    pub threads: Option<u32>,
    /// Ort des Hugging-Face-Caches (`HF_HOME`); `None` = Umgebung/Standard.
    pub hf_home: Option<String>,
    /// Downloads vom Hugging Face erlauben (Standard: nein).
    pub allow_network: bool,
}

impl Default for PhononConfig {
    fn default() -> Self {
        Self {
            exe_path: String::new(),
            model: String::new(),
            quant: None,
            lang: "de".to_string(),
            voice: None,
            threads: None,
            hf_home: None,
            allow_network: false,
        }
    }
}

fn invalid(msg: impl Into<String>) -> TtsError {
    TtsError::invalid_config(msg)
}

/// Gültiger Teil eines Hugging-Face-Namens (Besitzer oder Modell).
fn hf_name_ok(s: &str) -> bool {
    !s.is_empty()
        && !s.chars().all(|c| c == '.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Das Modell als Ordner oder Hugging-Face-Verweis.
#[derive(Debug, PartialEq, Eq)]
enum ModelSpec<'a> {
    Local(&'a str),
    Hf {
        owner: &'a str,
        name: &'a str,
        rev: Option<&'a str>,
    },
}

fn parse_model(model: &str) -> Result<ModelSpec<'_>, TtsError> {
    let m = model.trim();
    if m.is_empty() {
        return Err(invalid(
            "model fehlt: auf der Modellseite (Vorlesen) ein Phonon-Modell installieren, einen \
             lokalen Modellordner oder hf://OWNER/MODEL@COMMIT angeben",
        ));
    }
    if m.chars().any(char::is_control) {
        return Err(invalid("model enthält Steuerzeichen"));
    }
    if let Some(rest) = m.strip_prefix("hf://") {
        let (repo, rev) = match rest.split_once('@') {
            Some((r, v)) => (r, Some(v)),
            None => (rest, None),
        };
        let mut parts = repo.split('/');
        let (owner, name, extra) = (parts.next(), parts.next(), parts.next());
        let (Some(owner), Some(name), None) = (owner, name, extra) else {
            return Err(invalid(format!(
                "model '{m}' ist kein gültiger Verweis: erwartet hf://OWNER/MODEL oder hf://OWNER/MODEL@COMMIT"
            )));
        };
        if !hf_name_ok(owner) || !hf_name_ok(name) {
            return Err(invalid(format!(
                "model '{m}': Besitzer und Modellname dürfen nur Buchstaben, Ziffern, '.', '_' und '-' enthalten"
            )));
        }
        if let Some(v) = rev {
            let ok = !v.is_empty()
                && !v.contains("..")
                && !v.starts_with('/')
                && !v.ends_with('/')
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
            if !ok {
                return Err(invalid(format!(
                    "model '{m}': die Revision hinter '@' ist ungültig"
                )));
            }
        }
        return Ok(ModelSpec::Hf { owner, name, rev });
    }
    if m.starts_with('-') {
        return Err(invalid(
            "model darf nicht mit '-' beginnen (würde als Option gelesen)",
        ));
    }
    Ok(ModelSpec::Local(m))
}

impl PhononConfig {
    /// Prüft den GESPEICHERTEN Stand: leeres `exe_path`/`model` ist erlaubt und heißt „die von der
    /// App eingerichtete Installation“ (wird beim Auflösen aus [`super::policy::InstalledPaths`]
    /// gefüllt). Gesetzte Werte werden wie bei [`Self::validate`] geprüft.
    pub fn validate_stored(&self) -> Result<(), TtsError> {
        let mut probe = self.clone();
        if probe.exe_path.trim().is_empty() {
            probe.exe_path = "ptts".into();
        }
        if probe.model.trim().is_empty() {
            probe.model = "hf://app/installiert".into();
        }
        probe.validate()
    }

    /// Prüft die Einstellungen; die Meldungen sind für den Nutzer gedacht.
    pub fn validate(&self) -> Result<(), TtsError> {
        let exe = self.exe_path.trim();
        if exe.is_empty() {
            return Err(invalid(
                "Phonon ist nicht installiert: auf der Modellseite (Vorlesen) das Phonon-Programm \
                 installieren oder in exe_path den Pfad zur ptts-Programmdatei eintragen",
            ));
        }
        if exe.chars().any(char::is_control) {
            return Err(invalid("exe_path enthält Steuerzeichen"));
        }
        parse_model(&self.model)?;
        if let Some(q) = self.quant.as_deref() {
            let q = q.trim();
            if !q.is_empty() && !q.eq_ignore_ascii_case("q8") {
                return Err(invalid(format!(
                    "quant '{q}' wird nicht unterstützt: nur leer (f32) oder \"q8\""
                )));
            }
        }
        let lang = self.lang.trim().to_ascii_lowercase();
        if lang != "none" && !LANGS.contains(&lang.as_str()) {
            return Err(invalid(format!(
                "lang '{}' wird nicht unterstützt: de, en, fr, es, pt oder none",
                self.lang
            )));
        }
        if let Some(v) = self.voice.as_deref() {
            check_voice(v).map_err(|e| invalid(format!("voice: {}", e.message)))?;
        }
        if let Some(n) = self.threads {
            if !(1..=256).contains(&n) {
                return Err(invalid(format!("threads {n} liegt nicht in 1 bis 256")));
            }
        }
        if let Some(h) = self.hf_home.as_deref() {
            if h.trim().is_empty() || h.chars().any(char::is_control) {
                return Err(invalid("hf_home ist leer oder enthält Steuerzeichen"));
            }
        }
        Ok(())
    }

    fn quant_arg(&self) -> Option<&'static str> {
        self.quant
            .as_deref()
            .is_some_and(|q| q.trim().eq_ignore_ascii_case("q8"))
            .then_some("q8")
    }

    fn lang_arg(&self) -> String {
        self.lang.trim().to_ascii_lowercase()
    }

    /// Konfigurierte Stimme; leer und `default` heißen „Standardstimme“.
    fn configured_voice(&self) -> Option<&str> {
        self.voice
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty() && *v != "default")
    }
}

/// Stimmenname für die Kommandozeile prüfen: kein Optionsanfang, keine
/// Steuerzeichen.
fn check_voice(v: &str) -> Result<(), TtsError> {
    let v = v.trim();
    if v.is_empty() {
        return Err(TtsError::bad_request("Stimme ist leer"));
    }
    if v.starts_with('-') {
        return Err(TtsError::bad_request(
            "Stimme darf nicht mit '-' beginnen (würde als Option gelesen)",
        ));
    }
    if v.chars().any(char::is_control) {
        return Err(TtsError::bad_request("Stimme enthält Steuerzeichen"));
    }
    Ok(())
}

// ---- Stimmenkatalog des Modellordners (voices/SOURCE.json) -------------------

/// Obergrenze für `voices/SOURCE.json` (ein Dutzend Stimmen sind wenige KiB).
const SOURCE_JSON_MAX_BYTES: u64 = 512 * 1024;
/// Länge der Kurzbeschreibung in der Stimmenbeschriftung.
const VOICE_BLURB_CHARS: usize = 48;

/// Angaben zu EINER Stimme aus `voices/SOURCE.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct VoiceInfo {
    name: String,
    voice_id: String,
    /// Dateiname ohne Endung (`voices/<voice_id>.safetensors` → `<voice_id>`).
    file_stem: String,
    gender: Option<String>,
    age: Option<String>,
    description: Option<String>,
    /// Herkunft einer EIGENEN Stimme (`import`, `gradium-clone`); fehlt bei mitgelieferten.
    origin: Option<String>,
}

/// Der Inhalt von `<Modellordner>/voices/SOURCE.json`: woher die Stimmen stammen,
/// unter welcher Lizenz, und was man über sie weiß. Die Datei legt der Nutzer mit
/// den Stimmen ab (z. B. aus `hf://gradium/phonon-voices`); sie ist NICHT von der
/// App geprüft und deshalb nie als „verifiziert“ gemeldet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct VoiceSource {
    license: Option<String>,
    source: Option<String>,
    voices: Vec<VoiceInfo>,
}

impl VoiceSource {
    /// Die Angaben zur Stimmendatei `stem` (Name, Kennung oder Dateiname). Ein Name mit Leerzeichen
    /// („Captain Silas Blackwater“) liegt als `Captain-Silas-Blackwater.safetensors` auf der Platte.
    fn find(&self, stem: &str) -> Option<&VoiceInfo> {
        self.voices.iter().find(|v| {
            [&v.name, &v.voice_id, &v.file_stem].iter().any(|k| {
                !k.is_empty()
                    && (k.eq_ignore_ascii_case(stem)
                        || k.replace(' ', "-").eq_ignore_ascii_case(stem))
            })
        })
    }

    /// Lizenz der Stimmen: das Feld `license`, Quelle aus `source`.
    fn license_info(&self) -> Option<LicenseInfo> {
        let license = self.license.as_deref()?.trim();
        if license.is_empty() {
            return None;
        }
        let low = license.to_ascii_lowercase();
        // Nur bekannte freizügige Lizenzen gelten als kommerziell nutzbar; alles
        // andere (auch „…-nc…“ und Unbekanntes) vorsorglich als nichtkommerziell.
        let permissive = matches!(
            low.as_str(),
            "apache-2.0" | "mit" | "cc-by-4.0" | "cc0-1.0" | "bsd-3-clause" | "bsd-2-clause"
        );
        let source_url = self
            .source
            .as_deref()
            .and_then(|s| s.trim().strip_prefix("hf://"))
            .map(|rest| rest.split('@').next().unwrap_or(rest))
            .filter(|repo| !repo.is_empty())
            .map(|repo| format!("https://huggingface.co/{repo}"))
            .unwrap_or_default();
        Some(LicenseInfo {
            code: license.to_string(),
            weights: license.to_string(),
            non_commercial: !permissive,
            verified: false,
            source_url,
            verified_on: None,
            note: format!(
                "Angabe aus voices/SOURCE.json des Modellordners{}; von der App nicht geprüft.",
                self.source
                    .as_deref()
                    .map(|s| format!(" (Quelle {})", s.trim()))
                    .unwrap_or_default()
            ),
        })
    }
}

/// `<Modellordner>/voices/SOURCE.json` tolerant lesen. Fehlt die Datei, ist sie zu
/// groß oder kein JSON, gibt es keine Anreicherung — die Stimmenliste bleibt die
/// schlichte Dateiliste.
fn read_voice_source(model_dir: &Path) -> Option<VoiceSource> {
    let path = model_dir.join("voices").join("SOURCE.json");
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > SOURCE_JSON_MAX_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let text = |v: &serde_json::Value, key: &str| {
        v.get(key)
            .and_then(|x| x.as_str())
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
    };
    let voices = value
        .get("voices")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .map(|e| VoiceInfo {
                    name: text(e, "name").unwrap_or_default(),
                    voice_id: text(e, "voice_id").unwrap_or_default(),
                    file_stem: text(e, "filename")
                        .and_then(|f| {
                            Path::new(&f)
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .map(String::from)
                        })
                        .unwrap_or_default(),
                    gender: text(e, "gender").map(|g| g.to_ascii_lowercase()),
                    age: text(e, "age"),
                    description: text(e, "description"),
                    origin: text(e, "origin"),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(VoiceSource {
        license: text(&value, "license"),
        source: text(&value, "source"),
        voices,
    })
}

/// Kurzfassung eines Beschreibungstextes: höchstens `max` Zeichen, an einer
/// Wortgrenze, mit „…“ wenn gekürzt.
fn short_blurb(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max {
        return text.trim_end_matches('.').to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!("{}…", cut.trim_end_matches([' ', ',', ';', ':', '.', '-']))
}

/// Beschriftung einer Stimme aus ihren Angaben: `Name – Kurzbeschreibung`
/// (ohne Beschreibung der Alterseindruck, sonst nur der Name).
fn voice_label(info: &VoiceInfo, fallback: &str) -> String {
    let name = if info.name.is_empty() {
        fallback
    } else {
        info.name.as_str()
    };
    match (&info.description, &info.age) {
        (Some(d), _) => format!("{name} – {}", short_blurb(d, VOICE_BLURB_CHARS)),
        (None, Some(a)) => format!("{name} – {a}"),
        (None, None) => name.to_string(),
    }
}

/// Was ein Modellordner an Gewichten enthält (nur Dateisystem, kein Prozess).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ModelFolderInfo {
    /// Der Pfad ist ein vorhandener Ordner.
    pub is_dir: bool,
    /// `model.safetensors` (f32-Gewichte) liegt vor.
    pub has_f32: bool,
    /// `model.q8.gguf` (8-Bit-Gewichte) liegt vor.
    pub has_q8: bool,
}

/// Modellordner ansehen, damit die Oberfläche `q8` vorschlagen kann: liegt dort nur
/// `model.q8.gguf`, scheitert der f32-Lauf (Vorgabe) am Fehlen der Gewichte.
pub fn inspect_model_folder(path: &str) -> ModelFolderInfo {
    let path = path.trim();
    let dir = Path::new(path);
    let is_dir = !path.is_empty()
        && !path.starts_with("hf://")
        && !path.chars().any(char::is_control)
        && dir.is_dir();
    ModelFolderInfo {
        is_dir,
        has_f32: is_dir && dir.join("model.safetensors").is_file(),
        has_q8: is_dir && dir.join("model.q8.gguf").is_file(),
    }
}

// ---- Programm und Modell finden ---------------------------------------------

/// `exe` als Datei auflösen: Pfad (mit Trenner) direkt, sonst im `PATH`.
fn resolve_exe(exe: &str) -> Option<PathBuf> {
    let direct = Path::new(exe);
    if direct.is_absolute() || exe.contains('/') || exe.contains('\\') {
        return direct.is_file().then(|| direct.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let plain = dir.join(exe);
        if plain.is_file() {
            return Some(plain);
        }
        if cfg!(windows) && Path::new(exe).extension().is_none() {
            let with_ext = dir.join(format!("{exe}.exe"));
            if with_ext.is_file() {
                return Some(with_ext);
            }
        }
    }
    None
}

fn real_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// Ort des Hugging-Face-Hub-Caches, so wie ihn der Kindprozess auflöst:
/// `HF_HUB_CACHE`, sonst `<HF_HOME>/hub` (der konfigurierte Wert überstimmt
/// die Umgebung, weil er dem Kindprozess als `HF_HOME` mitgegeben wird),
/// sonst `$XDG_CACHE_HOME/huggingface/hub`, sonst `~/.cache/huggingface/hub`.
pub(crate) fn hub_cache_dir(
    hf_home_cfg: Option<&str>,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    for k in ["HF_HUB_CACHE", "HUGGINGFACE_HUB_CACHE"] {
        if let Some(v) = get(k) {
            return Some(PathBuf::from(v));
        }
    }
    let home = hf_home_cfg
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| get("HF_HOME"));
    if let Some(h) = home {
        return Some(PathBuf::from(h).join("hub"));
    }
    if let Some(x) = get("XDG_CACHE_HOME") {
        return Some(PathBuf::from(x).join("huggingface").join("hub"));
    }
    let home = get("HOME").or_else(|| get("USERPROFILE"))?;
    Some(
        PathBuf::from(home)
            .join(".cache")
            .join("huggingface")
            .join("hub"),
    )
}

/// Liegt ein Schnappschuss des Modells im Cache? `rev`: voller Commit (40
/// Hex-Zeichen) oder ein Verweis unter `refs/` (Standard `main`).
pub(crate) fn snapshot_present(cache: &Path, owner: &str, name: &str, rev: Option<&str>) -> bool {
    let repo = cache.join(format!("models--{owner}--{name}"));
    let rev = rev.unwrap_or("main");
    let commit = if rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()) {
        rev.to_string()
    } else {
        match std::fs::read_to_string(repo.join("refs").join(rev)) {
            Ok(s) if !s.trim().is_empty() && !s.trim().contains(['/', '\\', '.']) => {
                s.trim().to_string()
            }
            _ => return false,
        }
    };
    std::fs::read_dir(repo.join("snapshots").join(commit))
        .map(|mut d| d.next().is_some())
        .unwrap_or(false)
}

// ---- Kindprozess -------------------------------------------------------------

/// Hält den Kindprozess; `Drop` beendet und sammelt ihn ein (Kill-on-Drop).
/// Das Job-Objekt (Windows) beendet zusätzlich den ganzen Prozessbaum und
/// den Prozess beim Ende der App.
struct ChildGuard {
    child: std::process::Child,
    _job: Option<ProcessGuard>,
}

impl ChildGuard {
    /// Asynchron auf das Ende warten (kein Blockieren des Runtimes).
    async fn wait(&mut self) -> Result<ExitStatus, TtsError> {
        let mut delay = Duration::from_millis(4);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => {}
                Err(e) => {
                    return Err(TtsError::backend(format!(
                        "Status von ptts nicht lesbar: {e}"
                    )))
                }
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_millis(40));
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
        }
        // Einsammeln (kein Zombie); nach dem Kill kehrt das sofort zurück.
        let _ = self.child.wait();
    }
}

/// Ergebnis eines beendeten `ptts`-Laufs. `dir` hält den Arbeitsordner am
/// Leben (Ausgabedatei) und räumt beim Drop auf.
struct Run {
    dir: TempDir,
    out: PathBuf,
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn read_tail(path: &Path, max: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = File::open(path) else {
        return String::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len > max {
        let _ = f.seek(SeekFrom::Start(len - max));
    }
    let mut buf = Vec::new();
    let _ = f.take(max).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).trim().to_string()
}

/// Auf eine Zeile zusammenziehen und auf das ENDE kürzen (dort steht bei
/// Programmfehlern die Ursache).
fn tail_line(s: &str, max_chars: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let n = flat.chars().count();
    if n <= max_chars {
        return flat;
    }
    let tail: String = flat.chars().skip(n - max_chars).collect();
    format!("…{tail}")
}

fn status_text(st: &ExitStatus) -> String {
    match st.code() {
        Some(c) => format!("Code {c} (0x{:08X})", c as u32),
        None => "einem Signal".to_string(),
    }
}

fn spawn_error(exe: &Path, e: std::io::Error) -> TtsError {
    match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
            TtsError::unavailable(format!(
                "ptts ({}) lässt sich nicht starten: {e}",
                exe.display()
            ))
        }
        _ => TtsError::backend(format!(
            "ptts ({}) lässt sich nicht starten: {e}",
            exe.display()
        )),
    }
}

/// Steht etwas in der Ausgabe, das auf ein gesperrtes (gated) Modell deutet?
fn looks_gated(low: &str) -> bool {
    [
        "gated",
        "unauthorized",
        "forbidden",
        "accept its terms",
        "hf_token",
    ]
    .iter()
    .any(|k| low.contains(k))
}

fn looks_missing(low: &str) -> bool {
    [
        "no such file",
        "not found",
        "cannot find",
        "does not exist",
        "offline",
        "no snapshot",
        "failed to load",
        "os error 2",
        "config.json",
        "tokenizer.json",
    ]
    .iter()
    .any(|k| low.contains(k))
}

const GATED_HINT: &str = "Auf der Hugging-Face-Seite des Modells die Bedingungen annehmen und \
    HF_TOKEN setzen. Die Phonon-40m-Modelle (de, multilingual) stehen unter CC BY-NC 4.0 \
    (nicht kommerziell).";

// ---- Provider ----------------------------------------------------------------

/// Ein weiteres installiertes Modell einer anderen Sprache (Modellordner und Sprache).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhononModelRef {
    /// Sprache des Modells laut Katalog (`en`, `de`, ...).
    pub lang: String,
    /// Modellordner.
    pub model: String,
}

/// Trennzeichen zwischen Sprache und Stimme bei Stimmen eines weiteren Modells (`en:Zoey`).
const SIBLING_SEP: char = ':';

/// Phonon über `ptts` (siehe Moduldoku).
#[derive(Clone)]
pub struct PhononProvider {
    cfg: PhononConfig,
    ram_gate: fn(u64) -> Result<u64, String>,
    version_timeout: Duration,
    temp_base: Option<PathBuf>,
    max_wav_bytes: u64,
    env_get: fn(&str) -> Option<String>,
    list_voices_timeout: Duration,
    /// Weitere Modelle anderer Sprachen; ihre Stimmen heißen `<sprache>:<stimme>`.
    siblings: Vec<PhononModelRef>,
}

/// Warum der Anbieter nicht bereit ist: Fehler plus Hinweis für die Anzeige.
struct Problem {
    error: TtsError,
    hint: Option<String>,
}

fn unavailable(msg: impl Into<String>, hint: impl Into<String>) -> Problem {
    Problem {
        error: TtsError::unavailable(msg),
        hint: Some(hint.into()),
    }
}

impl PhononProvider {
    /// Anbieter aus einer Konfiguration. Prüft nicht selbst: `health_check`
    /// und die Synthese melden eine ungültige Konfiguration mit Grund
    /// (`InvalidConfig`); wer früher prüfen will, ruft [`PhononConfig::validate`].
    pub fn new(cfg: PhononConfig) -> Self {
        Self {
            cfg,
            ram_gate: process_guard::check_ram_for_start,
            version_timeout: VERSION_TIMEOUT,
            temp_base: None,
            max_wav_bytes: MAX_WAV_BYTES,
            env_get: real_env,
            list_voices_timeout: LIST_VOICES_TIMEOUT,
            siblings: Vec::new(),
        }
    }

    /// Weitere installierte Modelle anderer Sprachen (die Stufe „installiert“, z. B. das englische
    /// neben dem deutschen). Ihre Stimmen erscheinen als `<sprache>:<stimme>`; die Synthese nimmt
    /// dafür das Modell und die Sprache dieses Modells. Modelle der eigenen Sprache, unbekannte
    /// Sprachen und doppelte Sprachen fallen weg.
    pub fn with_siblings(mut self, siblings: Vec<PhononModelRef>) -> Self {
        let own = self.cfg.lang_arg();
        let mut kept: Vec<PhononModelRef> = Vec::new();
        for s in siblings {
            let lang = s.lang.trim().to_ascii_lowercase();
            if lang == own
                || !LANGS.contains(&lang.as_str())
                || kept.iter().any(|k| k.lang == lang)
                || s.model.trim().is_empty()
            {
                continue;
            }
            kept.push(PhononModelRef {
                lang,
                model: s.model,
            });
        }
        self.siblings = kept;
        self
    }

    /// Gehört die Stimme zu einem weiteren Modell (`<sprache>:<stimme>`)? Dann der Anbieter für
    /// dieses Modell und der Stimmenname darin (`default` = Standardstimme des Modells).
    fn sibling_for(&self, voice: &str) -> Option<(PhononProvider, String)> {
        let (lang, rest) = voice.trim().split_once(SIBLING_SEP)?;
        let rest = rest.trim();
        if rest.is_empty() {
            return None;
        }
        let sibling = self.siblings.iter().find(|s| s.lang == lang)?;
        let mut p = self.clone();
        p.cfg.model = sibling.model.clone();
        p.cfg.lang = sibling.lang.clone();
        p.cfg.voice = None;
        p.siblings = Vec::new();
        Some((p, rest.to_string()))
    }

    /// Die Stimmen im Ordner `voices/` des (lokalen) Modells, ohne Netz.
    fn local_voices(&self, make: &dyn Fn(&str, &str) -> VoiceDescriptor) -> Vec<VoiceDescriptor> {
        let mut voices = Vec::new();
        let Ok(ModelSpec::Local(dir)) = parse_model(&self.cfg.model) else {
            return voices;
        };
        // Mit `voices/SOURCE.json` tragen die Stimmen Geschlecht, Kurzbeschreibung
        // und Lizenz; ohne sie bleibt es bei den Dateinamen.
        let source = read_voice_source(Path::new(dir));
        let license = source.as_ref().and_then(VoiceSource::license_info);
        let mut names: Vec<String> = std::fs::read_dir(Path::new(dir).join("voices"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                (p.extension().is_some_and(|x| x == "safetensors"))
                    .then(|| p.file_stem()?.to_str().map(String::from))
                    .flatten()
            })
            .collect();
        names.sort();
        for n in names {
            let mut voice = make(&n, &n);
            if let Some(info) = source.as_ref().and_then(|s| s.find(&n)) {
                voice.label = voice_label(info, &n);
                voice.gender = info.gender.clone();
                // Eigene Stimmen (Import, Klon) haben eigene Rechte: die Lizenz des
                // Modells gilt nicht fuer sie.
                voice.license = match info.origin.as_deref() {
                    Some(o) if phonon_voices::origin_is_own(o) => {
                        Some(phonon_voices::own_voice_license(o))
                    }
                    _ => license.clone(),
                };
            }
            voices.push(voice);
        }
        voices
    }

    /// Stimmen der weiteren Modelle (`<sprache>:<stimme>`), nur aus deren Ordnern.
    fn sibling_voices(&self) -> Vec<VoiceDescriptor> {
        let mut out = Vec::new();
        for s in &self.siblings {
            let Some((p, _)) = self.sibling_for(&format!("{}{SIBLING_SEP}default", s.lang)) else {
                continue;
            };
            if p.check_model().is_err() {
                continue;
            }
            let lang = p.voice_language();
            let make = |id: &str, label: &str| VoiceDescriptor {
                id: format!("{}{SIBLING_SEP}{id}", s.lang),
                label: label.to_string(),
                language: lang.clone(),
                gender: None,
                license: None,
            };
            let mut default = make("default", "Standardstimme des Modells");
            default.label = format!("Standardstimme des Modells ({})", s.lang);
            default.license = Some(p.license());
            out.push(default);
            let model_license = p.license();
            for mut v in p.local_voices(&make) {
                if v.license.is_none() {
                    v.license = Some(model_license.clone());
                }
                out.push(v);
            }
        }
        out
    }

    /// Wie [`new`](Self::new), aber mit sofortiger Prüfung.
    pub fn try_new(cfg: PhononConfig) -> Result<Self, TtsError> {
        cfg.validate()?;
        Ok(Self::new(cfg))
    }

    pub fn config(&self) -> &PhononConfig {
        &self.cfg
    }

    /// Start-Gate austauschen (Tests).
    #[cfg(test)]
    pub(crate) fn with_ram_gate(mut self, gate: fn(u64) -> Result<u64, String>) -> Self {
        self.ram_gate = gate;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_version_timeout(mut self, t: Duration) -> Self {
        self.version_timeout = t;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_temp_base(mut self, base: PathBuf) -> Self {
        self.temp_base = Some(base);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_max_wav_bytes(mut self, n: u64) -> Self {
        self.max_wav_bytes = n;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_env(mut self, get: fn(&str) -> Option<String>) -> Self {
        self.env_get = get;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_list_voices_timeout(mut self, t: Duration) -> Self {
        self.list_voices_timeout = t;
        self
    }

    fn voice_language(&self) -> String {
        let l = self.cfg.lang_arg();
        if l == "none" {
            "und".to_string()
        } else {
            l
        }
    }

    /// Lizenz der Gewichte nach dem Modell. `verified` nur für die
    /// ausdrücklich recherchierten Gradium-Repos (`hf://gradium/phonon-…`);
    /// alles andere gilt vorsorglich als nichtkommerziell und ungeprüft.
    fn license(&self) -> LicenseInfo {
        // Ein ueber die Modellseite installiertes Katalog-Modell kennt seine Lizenz aus dem
        // Katalog (Quelle und Pruefdatum dort); fremde Ordner gleichen Namens nicht (Marke).
        if let Some(info) = phonon_models::installed_model_license(&self.cfg.model) {
            return info;
        }
        let code = "MIT OR Apache-2.0".to_string();
        let unknown = |model: &str| {
            LicenseInfo {
            code: code.clone(),
            weights: "unbekannt: Model Card des Checkpoints prüfen (Phonon 31m: Apache-2.0, Phonon 40m: CC BY-NC 4.0)".to_string(),
            non_commercial: true,
            verified: false,
            source_url: REPO_URL.to_string(),
            verified_on: None,
            note: format!(
                "Die Code-Lizenz (MIT OR Apache-2.0) ist belegt (README von gradium-ai/xn-ptts, 26.10.2026); \
                 {which} und wird vorsorglich als nichtkommerziell behandelt. Stimmen haben \
                 eigene Lizenzen (docs/cli.md).",
                which = if model.is_empty() {
                    "es ist kein Modell eingetragen, die Lizenz der Gewichte lässt sich ohne \
                     Modell nicht belegen"
                        .to_string()
                } else {
                    format!("die Lizenz der Gewichte von '{model}' ist nicht belegt")
                }
            ),
        }
        };
        let model = self.cfg.model.trim();
        if let Ok(ModelSpec::Hf {
            owner: "gradium",
            name,
            ..
        }) = parse_model(model)
        {
            let low = name.to_ascii_lowercase();
            if low.starts_with("phonon-31m") {
                return LicenseInfo {
                    code,
                    weights: "Apache-2.0 (Phonon 31m)".to_string(),
                    non_commercial: false,
                    verified: true,
                    source_url: "https://huggingface.co/gradium/phonon-31m-en".to_string(),
                    verified_on: Some(VERIFIED_ON.to_string()),
                    note:
                        "Code MIT OR Apache-2.0 (README), Gewichte 31m Apache-2.0 (README-Tabelle, \
                           HF-Karte), geprüft 26.10.2026 (docs/tts/provider-research.md, A). Das \
                           31m-Modell spricht nur Englisch."
                            .to_string(),
                };
            }
            if low.starts_with("phonon-40m") {
                return LicenseInfo {
                    code,
                    weights: "CC BY-NC 4.0 (Phonon 40m, gated)".to_string(),
                    non_commercial: true,
                    verified: true,
                    source_url: "https://huggingface.co/gradium/phonon-40m-de".to_string(),
                    verified_on: Some(VERIFIED_ON.to_string()),
                    note: "Gewichte der 40m-Modelle (de, multilingual): CC BY-NC 4.0, keine \
                           kommerzielle Nutzung; gated: Bedingungen auf Hugging Face annehmen und \
                           HF_TOKEN setzen (README, HF-Karte, geprüft 26.10.2026, \
                           docs/tts/provider-research.md, A)."
                        .to_string(),
                };
            }
        }
        unknown(model)
    }

    /// Bereit zum Start? Prüft Konfiguration, Programm und Modell — ohne
    /// einen Prozess zu starten.
    fn check_ready(&self) -> Result<PathBuf, Problem> {
        self.cfg.validate().map_err(|error| Problem {
            hint: Some(
                if self.cfg.exe_path.trim().is_empty() || self.cfg.model.trim().is_empty() {
                    "Auf der Modellseite (Vorlesen) das Phonon-Programm und ein Phonon-Modell \
                     installieren; exe_path und model bleiben dann leer und werden automatisch \
                     gefunden."
                } else {
                    "Phonon-Einstellungen prüfen (exe_path, model, lang)."
                }
                .to_string(),
            ),
            error,
        })?;
        let exe_text = self.cfg.exe_path.trim();
        let Some(exe) = resolve_exe(exe_text) else {
            return Err(unavailable(
                format!("ptts wurde nicht gefunden: {exe_text}"),
                "Auf der Modellseite (Vorlesen) das Phonon-Programm installieren. Alternativ das \
                 offizielle ptts-CLI (v0.4.x) von github.com/gradium-ai/xn-ptts/releases \
                 herunterladen und den Pfad zur Programmdatei eintragen. Ohne ausdrücklichen \
                 Klick lädt die App nichts.",
            ));
        };
        self.check_model()?;
        Ok(exe)
    }

    /// Groesse und Aenderungszeit der Stimmdatei `<Modellordner>/voices/<stimme>.safetensors`
    /// (nur fuer einen lokalen Modellordner und einen schlichten Dateinamen).
    fn voice_file_stamp(&self, voice: &str) -> Option<String> {
        let plain = !voice.is_empty()
            && !voice.starts_with('.')
            && !voice.contains(['/', '\\', ':'])
            && voice != "default";
        if !plain {
            return None;
        }
        let ModelSpec::Local(dir) = parse_model(&self.cfg.model).ok()? else {
            return None;
        };
        let meta = std::fs::metadata(
            Path::new(dir)
                .join("voices")
                .join(format!("{voice}.safetensors")),
        )
        .ok()?;
        let secs = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(format!("{:x}{:x}", meta.len(), secs))
    }

    /// Darf `ptts` diese Stimme OHNE Netzfreigabe laden? `None` = ja; sonst der
    /// Grund für den Nutzer. Ein Katalogname (`--voice Zoey`) ließe `ptts` die
    /// Stimme von Hugging Face holen, und `HF_HUB_OFFLINE` ist für `ptts` nicht
    /// dokumentiert — darum ist ohne `allow_network` nur erlaubt:
    /// * `default` (kein `--voice`, das prüft der Aufrufer),
    /// * der Pfad einer vorhandenen `.safetensors`-Datei,
    /// * ein schlichter Name (kein Pfadtrenner, kein `..`), zu dem es in
    ///   `<Modellordner>/voices/` eine `.safetensors`-Datei gibt.
    fn voice_problem(&self, voice: &str) -> Option<String> {
        if self.cfg.allow_network || voice == "default" {
            return None;
        }
        let is_file = |p: &Path| p.is_file();
        let low = voice.to_ascii_lowercase();
        // Ein Netzwerkpfad (UNC) ist selbst ein Zugriff über das Netz: nicht einmal
        // nachsehen, ob es die Datei gibt.
        let network_path = voice.starts_with("\\\\") || voice.starts_with("//");
        if !network_path && low.ends_with(".safetensors") && is_file(Path::new(voice)) {
            return None;
        }
        let plain = !voice.contains(['/', '\\']) && !voice.contains("..");
        if plain {
            if let Ok(ModelSpec::Local(dir)) = parse_model(&self.cfg.model) {
                let file = if low.ends_with(".safetensors") {
                    voice.to_string()
                } else {
                    format!("{voice}.safetensors")
                };
                if is_file(&Path::new(dir).join("voices").join(file)) {
                    return None;
                }
            }
        }
        Some(format!(
            "Die Stimme '{voice}' ist ohne Netzfreigabe nicht ladbar: ptts würde sie von Hugging \
             Face holen. Erlaubt sind 'default', eine Stimme aus dem Ordner voices/ des lokalen \
             Modellordners oder der Pfad einer vorhandenen .safetensors-Datei — oder allow_network \
             aktivieren."
        ))
    }

    /// Modell prüfen; `Ok(Some(hinweis))` = bereit, mit Hinweis für die Anzeige.
    fn check_model(&self) -> Result<Option<String>, Problem> {
        let spec = parse_model(&self.cfg.model).map_err(|error| Problem { error, hint: None })?;
        match spec {
            ModelSpec::Local(dir) => {
                let dir = Path::new(dir);
                if !dir.is_dir() {
                    return Err(unavailable(
                        format!("Modellordner fehlt: {}", dir.display()),
                        "Auf der Modellseite (Vorlesen) ein Modell installieren, oder den \
                         Modellordner (z. B. von huggingface.co/gradium) herunterladen und den \
                         Pfad eintragen, oder ein hf://-Modell mit allow_network laden.",
                    ));
                }
                for f in ["config.json", "tokenizer.json"] {
                    if !dir.join(f).is_file() {
                        return Err(unavailable(
                            format!("Im Modellordner {} fehlt {f}", dir.display()),
                            "Der Ordner ist kein vollständiger Phonon-Checkpoint (erwartet: \
                             config.json, tokenizer.json, Gewichte).",
                        ));
                    }
                }
                Ok(None)
            }
            ModelSpec::Hf { owner, name, rev } => {
                if self.cfg.allow_network {
                    return Ok(Some(
                        "Downloads sind erlaubt: fehlt das Modell im Cache, lädt ptts es beim \
                         ersten Lauf aus dem Netz."
                            .to_string(),
                    ));
                }
                let get = self.env_get;
                let Some(cache) = hub_cache_dir(self.cfg.hf_home.as_deref(), &get) else {
                    return Err(unavailable(
                        "Der Hugging-Face-Cache ist nicht bestimmbar (HOME/HF_HOME fehlen)",
                        "hf_home eintragen oder ein lokales Modellverzeichnis verwenden.",
                    ));
                };
                if snapshot_present(&cache, owner, name, rev) {
                    Ok(Some(format!(
                        "Offline: das Modell wird nur aus dem Hugging-Face-Cache geladen ({}).",
                        cache.display()
                    )))
                } else {
                    Err(unavailable(
                        format!(
                            "Das Modell hf://{owner}/{name} liegt nicht im Hugging-Face-Cache ({}) \
                             und Downloads sind aus",
                            cache.display()
                        ),
                        format!(
                            "Einmal laden (allow_network aktivieren) oder einen lokalen \
                             Modellordner angeben. {GATED_HINT}"
                        ),
                    ))
                }
            }
        }
    }

    fn apply_env(&self, cmd: &mut std::process::Command) {
        if self.cfg.allow_network {
            // Ausdrückliche Freigabe schlägt eine vererbte Sperre.
            cmd.env_remove("HF_HUB_OFFLINE");
        } else {
            cmd.env("HF_HUB_OFFLINE", "1");
        }
        if let Some(h) = self
            .cfg
            .hf_home
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            cmd.env("HF_HOME", h);
        }
    }

    /// `ptts` mit den Argumenten aus `make_args` (bekommt den Pfad der
    /// Ausgabedatei) starten und auf das Ende warten. Fällt das Future vorher
    /// (Abbruch/Frist), beendet [`ChildGuard`] den Prozess; der Arbeitsordner
    /// verschwindet danach (er ist VOR dem Wächter angelegt und wird deshalb
    /// NACH ihm freigegeben).
    async fn run(
        &self,
        exe: &Path,
        make_args: impl FnOnce(&Path) -> Vec<OsString>,
    ) -> Result<Run, TtsError> {
        let builder = tempfile::Builder::new()
            .prefix("lva-ptts-")
            .tempdir_in(self.temp_base.clone().unwrap_or_else(std::env::temp_dir));
        let dir = builder.map_err(|e| {
            TtsError::backend(format!(
                "Kein Arbeitsordner für ptts anlegbar (Datenträger voll oder gesperrt?): {e}"
            ))
        })?;
        let out = dir.path().join("out.wav");
        let stdout_path = dir.path().join("stdout.log");
        let stderr_path = dir.path().join("stderr.log");
        let create = |p: &Path| {
            File::create(p).map_err(|e| {
                TtsError::backend(format!("Protokolldatei für ptts nicht anlegbar: {e}"))
            })
        };
        let (stdout_file, stderr_file) = (create(&stdout_path)?, create(&stderr_path)?);

        // Nie eine Platzhalter-/kaputte .exe starten (Windows zeigte sonst einen modalen Dialog).
        process_guard::check_launchable(exe).map_err(TtsError::unavailable)?;
        let mut cmd = std::process::Command::new(exe);
        cmd.args(make_args(&out))
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));
        self.apply_env(&mut cmd);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW (Muster: piper.rs)
        }
        let child = cmd.spawn().map_err(|e| spawn_error(exe, e))?;
        drop(cmd); // schließt unsere Kopien der Protokolldateien
        let job = ProcessGuard::attach(&child, None, process_guard::CPU_CAP_PERCENT);
        let mut guard = ChildGuard { child, _job: job };
        let status = guard.wait().await?;
        drop(guard);
        Ok(Run {
            stdout: read_tail(&stdout_path, LOG_TAIL_BYTES),
            stderr: read_tail(&stderr_path, LOG_TAIL_BYTES),
            dir,
            out,
            status,
        })
    }

    /// `ptts --version` → Versionsangabe.
    async fn query_version(&self, exe: &Path) -> Result<String, TtsError> {
        let run = self.run(exe, |_| vec![OsString::from("--version")]).await?;
        if !run.status.success() {
            return Err(TtsError::unavailable(format!(
                "`ptts --version` endete mit {}: {}",
                status_text(&run.status),
                tail_line(&run.stderr, 200)
            )));
        }
        let text = if run.stdout.trim().is_empty() {
            &run.stderr
        } else {
            &run.stdout
        };
        let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let token = line
            .split_whitespace()
            .map(|t| t.trim_start_matches(['v', 'V']))
            .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()));
        Ok(token
            .map(str::to_string)
            .unwrap_or_else(|| tail_line(line, 60)))
    }

    /// Stimme für den Aufruf: `opts.voice` vor der Konfiguration; `default`
    /// heißt „Standardstimme des Modells“ (kein `--voice`).
    fn effective_voice(&self, from_opts: Option<&str>) -> Result<Option<String>, TtsError> {
        match from_opts.map(str::trim).filter(|v| !v.is_empty()) {
            Some("default") => Ok(None),
            Some(v) => {
                check_voice(v)?;
                Ok(Some(v.to_string()))
            }
            None => Ok(self.cfg.configured_voice().map(String::from)),
        }
    }

    fn synth_args(&self, out: &Path, text: &str, voice: Option<&str>) -> Vec<OsString> {
        let mut a: Vec<OsString> = vec![
            "--model".into(),
            self.cfg.model.trim().into(),
            "--lang".into(),
            self.cfg.lang_arg().into(),
        ];
        if let Some(q) = self.cfg.quant_arg() {
            a.push("--quant".into());
            a.push(q.into());
        }
        if let Some(v) = voice {
            a.push("--voice".into());
            a.push(v.into());
        }
        if let Some(n) = self.cfg.threads {
            a.push("--threads".into());
            a.push(n.to_string().into());
        }
        a.push(text.into());
        a.push("-o".into());
        a.push(out.as_os_str().to_owned());
        a
    }

    /// Fehlschlag des Programms nach Klasse einordnen.
    fn classify_failure(&self, run: &Run) -> TtsError {
        let low = run.stderr.to_lowercase();
        let detail = tail_line(&run.stderr, 300);
        if looks_gated(&low) {
            return TtsError::new(
                TtsErrorKind::LicenseBlocked,
                format!("Phonon-Modell gesperrt (gated): {detail}. {GATED_HINT}"),
            );
        }
        if looks_missing(&low) {
            return TtsError::unavailable(format!(
                "ptts findet das Modell nicht: {detail}. Falls das Modell auf Hugging Face \
                 gesperrt ist: {GATED_HINT}"
            ));
        }
        TtsError::backend(format!(
            "ptts endete mit {}: {}",
            status_text(&run.status),
            if detail.is_empty() {
                "keine Meldung".to_string()
            } else {
                detail
            }
        ))
    }

    async fn synth_once(
        &self,
        exe: &Path,
        text: &str,
        voice: Option<&str>,
    ) -> Result<AudioClip, TtsError> {
        let run = self
            .run(exe, |out| self.synth_args(out, text, voice))
            .await?;
        if !run.status.success() {
            return Err(self.classify_failure(&run));
        }
        let Ok(meta) = std::fs::metadata(&run.out) else {
            return Err(TtsError::backend(
                "ptts meldete Erfolg, schrieb aber keine Ausgabedatei",
            ));
        };
        if meta.len() == 0 {
            return Err(TtsError::backend("ptts schrieb eine leere Ausgabedatei"));
        }
        if meta.len() > self.max_wav_bytes {
            return Err(TtsError::backend(format!(
                "ptts-Ausgabe ist unplausibel groß ({} MiB, Grenze {} MiB)",
                meta.len() / (1024 * 1024),
                self.max_wav_bytes / (1024 * 1024)
            )));
        }
        let bytes = std::fs::read(&run.out)
            .map_err(|e| TtsError::backend(format!("Ausgabedatei nicht lesbar: {e}")))?;
        let clip = AudioClip::from_wav(bytes).map_err(|e| {
            TtsError::backend(format!("ptts lieferte kein brauchbares WAV: {}", e.message))
        })?;
        if clip.duration_ms == 0 {
            return Err(TtsError::backend("ptts lieferte ein leeres Audio"));
        }
        Ok(clip)
    }
}

/// Text für die Kommandozeile: Steuerzeichen werden Leerzeichen; ein Text,
/// der mit `-` oder `@` beginnt, bekommt ein Leerzeichen davor (siehe
/// Moduldoku).
fn prepare_text(text: &str) -> Result<String, TtsError> {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let t = flat.trim();
    if t.is_empty() {
        return Err(TtsError::bad_request("Leerer Text"));
    }
    if t.chars().count() > MAX_TEXT_CHARS {
        return Err(TtsError::bad_request(format!(
            "Text zu lang für einen ptts-Aufruf ({} von höchstens {MAX_TEXT_CHARS} Zeichen)",
            t.chars().count()
        )));
    }
    if t.starts_with('-') || t.starts_with('@') {
        Ok(format!(" {t}"))
    } else {
        Ok(t.to_string())
    }
}

/// Stimmen aus der Ausgabe von `ptts --list-voices` lesen. Das Format ist in
/// `docs/cli.md` nicht beschrieben (nur „listet die Stimmen mit Sprache
/// auf“); darum tolerant: je Zeile das erste Wort als Kennung, eine
/// Sprachangabe (`de`, `de-DE`, `(de)`) irgendwo in der Zeile.
fn parse_voice_catalog(out: &str, default_lang: &str) -> Vec<VoiceDescriptor> {
    let mut voices = Vec::new();
    for line in out.lines() {
        let mut toks = line.split_whitespace().peekable();
        while toks
            .peek()
            .is_some_and(|t| matches!(*t, "-" | "*" | "•" | ">"))
        {
            toks.next();
        }
        let Some(first) = toks.next() else { continue };
        if first.ends_with(':') {
            continue; // Überschrift
        }
        let ok = first.len() <= 100
            && first
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/'))
            && !first.starts_with('-');
        if !ok {
            continue;
        }
        let lang = toks
            .find_map(|t| {
                let t = t.trim_matches(|c: char| "()[],:;".contains(c));
                let primary = t
                    .split(['-', '_'])
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                LANGS.contains(&primary.as_str()).then_some(primary)
            })
            .unwrap_or_else(|| default_lang.to_string());
        voices.push(VoiceDescriptor {
            id: first.to_string(),
            label: first.to_string(),
            language: lang,
            gender: None,
            license: None,
        });
    }
    voices
}

impl TtsProvider for PhononProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        let lang = self.cfg.lang_arg();
        let languages = if LANGS.contains(&lang.as_str()) {
            let mut l = vec![lang];
            l.extend(self.siblings.iter().map(|s| s.lang.clone()));
            l
        } else {
            LANGS.map(String::from).to_vec()
        };
        ProviderDescriptor {
            id: PHONON_ID.to_string(),
            label: "Phonon".to_string(),
            tier: ProviderTier::Experimental,
            opt_in: true,
            caps: ProviderCaps {
                languages,
                streaming: StreamingMode::Sentence,
                formats: vec![AudioEncoding::Wav],
                // „natural 24 kHz speech“ (HF-Karte phonon-40m-de, 26.10.2026)
                sample_rates: vec![24_000],
                // Klonen laut Recherche nur über Gradium Studio/API (Cloud).
                cloning: false,
                devices: vec![Device::Cpu],
                // docs/cli.md: Linux x86_64/ARM64, macOS Apple Silicon/Intel, Windows x64.
                platforms: [
                    "windows-x64",
                    "linux-x64",
                    "linux-aarch64",
                    "macos-aarch64",
                    "macos-x64",
                ]
                .map(String::from)
                .to_vec(),
                style_tags: false,
                // `--voice` je Aufruf; die Liste ist der Ordner `voices/` des Modells.
                voice_switching: true,
            },
            license: self.license(),
        }
    }

    fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
        Box::pin(async move {
            let fail = |reason: String, hint: Option<String>| {
                let mut h = ProviderHealth::unavailable(reason, hint);
                h.device = Some(Device::Cpu);
                h
            };
            let exe = match self.check_ready() {
                Ok(exe) => exe,
                Err(p) => return fail(p.error.message, p.hint),
            };
            let model_hint = match self.check_model() {
                Ok(hint) => hint,
                Err(p) => return fail(p.error.message, p.hint),
            };
            if let Some(why) = self
                .cfg
                .configured_voice()
                .and_then(|v| match self.sibling_for(v) {
                    Some((p, name)) if name == "default" => {
                        p.check_model().err().map(|e| e.error.message)
                    }
                    Some((p, name)) => p
                        .check_model()
                        .err()
                        .map(|e| e.error.message)
                        .or_else(|| p.voice_problem(&name)),
                    None => self.voice_problem(v),
                })
            {
                return fail(
                    why,
                    Some(
                        "Eine Stimme aus dem Ordner voices/ des Modellordners eintragen, die \
                         Stimme leer lassen (Standardstimme) oder allow_network aktivieren."
                            .to_string(),
                    ),
                );
            }
            let version =
                match tokio::time::timeout(self.version_timeout, self.query_version(&exe)).await {
                    Err(_) => {
                        return fail(
                            format!(
                                "`ptts --version` antwortet nicht innerhalb von {} s",
                                self.version_timeout.as_secs()
                            ),
                            Some(
                                "Prüfen, ob die Datei wirklich ptts ist und zur CPU passt \
                                 (Windows: x86-64-v3)."
                                    .to_string(),
                            ),
                        )
                    }
                    Ok(Err(e)) => {
                        return fail(
                            e.message,
                            Some(
                                "Die ptts-Datei ist beschädigt oder passt nicht zu diesem System; \
                                 neu herunterladen (SHA256SUMS prüfen)."
                                    .to_string(),
                            ),
                        )
                    }
                    Ok(Ok(v)) => v,
                };
            let mut h = ProviderHealth::ok();
            h.device = Some(Device::Cpu);
            h.version = Some(version);
            h.hint = model_hint;
            h
        })
    }

    fn list_voices<'a>(
        &'a self,
        language: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
        Box::pin(async move {
            let exe = self.check_ready().map_err(|p| p.error)?;
            let lang = self.voice_language();
            let make = |id: &str, label: &str| VoiceDescriptor {
                id: id.to_string(),
                label: label.to_string(),
                language: lang.clone(),
                gender: None,
                license: None,
            };
            let mut voices = vec![make("default", "Standardstimme des Modells")];
            // Eine konfigurierte Stimme eines weiteren Modells steht unten mit Sprache und Lizenz.
            if let Some(v) = self
                .cfg
                .configured_voice()
                .filter(|v| self.sibling_for(v).is_none())
            {
                voices.push(make(v, v));
            }
            // Offline nur, was im Modellordner liegt.
            voices.extend(self.local_voices(&make));
            // Dazu die Stimmen weiterer installierter Modelle anderer Sprachen.
            voices.extend(self.sibling_voices());
            // Den Hugging-Face-Katalog nur mit Netzfreigabe.
            if self.cfg.allow_network {
                let listed = tokio::time::timeout(
                    self.list_voices_timeout,
                    self.run(&exe, |_| vec![OsString::from("--list-voices")]),
                )
                .await;
                if let Ok(Ok(run)) = listed {
                    if run.status.success() {
                        voices.extend(parse_voice_catalog(&run.stdout, &lang));
                    }
                }
            }
            let mut seen = HashSet::new();
            voices.retain(|v| seen.insert(v.id.clone()));
            voices.retain(|v| language_matches(&v.language, language));
            Ok(voices)
        })
    }

    fn synthesize<'a>(
        &'a self,
        text: &'a str,
        opts: &'a SynthesisOptions,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
        Box::pin(async move {
            let voice = self.effective_voice(opts.voice.as_deref())?;
            // Stimme eines weiteren Modells (`en:Zoey`): dessen Modell und Sprache vorlesen lassen.
            if let Some((p, name)) = voice.as_deref().and_then(|v| self.sibling_for(v)) {
                let mut o = opts.clone();
                o.voice = Some(name);
                return p.synthesize(text, &o, cancel).await;
            }
            let exe = self.check_ready().map_err(|p| p.error)?;
            let text = prepare_text(text)?;
            // Ohne Netzfreigabe lädt `ptts` keine Stimme nach: Katalognamen scheitern
            // hier, ohne Prozessstart.
            if let Some(v) = voice.as_deref() {
                if let Some(why) = self.voice_problem(v) {
                    return Err(TtsError::unavailable(why));
                }
            }
            // Start-Gate: bei knappem RAM startet ptts gar nicht erst.
            (self.ram_gate)(RAM_NEED_MB).map_err(TtsError::unavailable)?;
            guarded(
                cancel,
                opts.timeout,
                "Phonon",
                self.synth_once(&exe, &text, voice.as_deref()),
            )
            .await
        })
    }

    /// `"phonon:<ordnername>~<hash>:<stimme>"`: der Hash deckt Modell, Quant
    /// und Sprache ab (anderes Modell oder `q8` klingt anders, verschiedene
    /// Ordner gleichen Namens dürfen sich nicht überschneiden).
    fn cache_tag(&self, voice: Option<&str>) -> String {
        let chosen = voice
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .or_else(|| self.cfg.configured_voice());
        if let Some((p, name)) = chosen.and_then(|v| self.sibling_for(v)) {
            return p.cache_tag(Some(&name));
        }
        let model = self.cfg.model.trim();
        let base = model
            .rsplit(['/', '\\'])
            .find(|s| !s.is_empty())
            .unwrap_or("modell")
            .split('@')
            .next()
            .unwrap_or("modell");
        let base: String = base
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let hash = fnv1a64(&format!(
            "{model}|{}|{}",
            self.cfg.quant_arg().unwrap_or("f32"),
            self.cfg.lang_arg()
        ));
        let voice = voice
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .or_else(|| self.cfg.configured_voice())
            .unwrap_or("default");
        // Wird eine Stimmdatei ersetzt (eigene Stimme neu erstellt), muss schon zwischengespeichertes
        // Vorlesen mit der alten Stimme ungueltig werden: Groesse und Aenderungszeit der Datei
        // gehoeren in den Tag.
        let stamp = self
            .voice_file_stamp(voice)
            .map(|s| format!("~{s}"))
            .unwrap_or_default();
        format!(
            "{PHONON_ID}:{base}~{:08x}:{voice}{stamp}",
            hash & 0xffff_ffff
        )
    }

    /// Modell dieser Instanz; `ptts` ist ein lokales Programm (keine Adresse).
    fn use_subject(&self) -> UseSubject {
        UseSubject {
            model: self.cfg.model.trim().to_string(),
            base_url: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::contract_tests::{contract_suite, ContractExpect};
    use crate::managers::tts::providers::phonon_fake::{fake_ptts, FakeOpts, FakePtts};
    use std::sync::Arc;
    use std::time::Instant;

    /// Ein vollständiger lokaler Modellordner (Dateien nur als Platzhalter).
    fn model_dir(with_voices: &[&str]) -> TempDir {
        let d = TempDir::new().unwrap();
        for f in ["config.json", "tokenizer.json", "model.safetensors"] {
            std::fs::write(d.path().join(f), b"{}").unwrap();
        }
        if !with_voices.is_empty() {
            std::fs::create_dir_all(d.path().join("voices")).unwrap();
            for v in with_voices {
                std::fs::write(
                    d.path().join("voices").join(format!("{v}.safetensors")),
                    b"x",
                )
                .unwrap();
            }
        }
        d
    }

    fn cfg(fake: &FakePtts, model: &TempDir) -> PhononConfig {
        PhononConfig {
            exe_path: fake.exe.display().to_string(),
            model: model.path().display().to_string(),
            ..PhononConfig::default()
        }
    }

    fn provider(fake: &FakePtts, model: &TempDir) -> PhononProvider {
        PhononProvider::new(cfg(fake, model)).with_ram_gate(|_| Ok(64 * 1024))
    }

    fn opts() -> SynthesisOptions {
        SynthesisOptions {
            timeout: Duration::from_secs(30),
            ..Default::default()
        }
    }

    // ---- Vertrag ------------------------------------------------------------

    #[tokio::test]
    async fn besteht_den_vertrag_mit_fake_ptts_inklusive_abbruch_und_frist() {
        let model = model_dir(&[]);
        let fast = fake_ptts(FakeOpts::default());
        let slow = fake_ptts(FakeOpts::slow());
        contract_suite(
            Arc::new(provider(&fast, &model)),
            ContractExpect::available()
                .with_sample_rate(24_000)
                .with_slow(Arc::new(provider(&slow, &model))),
        )
        .await;
        // Die langsamen Läufe wurden abgebrochen/ausgezeitet: der Fake schläft
        // 1,5 s und schreibt den Marker erst danach. Jeder dieser Läufe endete
        // spätestens jetzt; wartet man die volle Schlafzeit ab, darf der Marker
        // nie entstehen — alle Prozesse wurden wirklich beendet.
        tokio::time::sleep(Duration::from_millis(1800)).await;
        assert!(
            !slow.marker.exists(),
            "der langsame Fake lief nach Abbruch/Frist weiter"
        );
        // Dass die Prozesse wirklich gestartet wurden (und nicht nur nie liefen),
        // beweisen die Einzeltests `abbruch_beendet_den_kindprozess…` und
        // `frist_beendet_den_kindprozess…` mit längerer Vorlaufzeit.
    }

    #[tokio::test]
    async fn fehlende_exe_besteht_den_vertrag_als_nicht_verfuegbar() {
        let model = model_dir(&[]);
        let p = PhononProvider::new(PhononConfig {
            exe_path: "Z:/gibt/es/nicht/ptts.exe".to_string(),
            model: model.path().display().to_string(),
            ..PhononConfig::default()
        })
        .with_ram_gate(|_| Ok(64 * 1024));
        let h = p.health_check().await;
        assert!(!h.available);
        assert_eq!(h.device, Some(Device::Cpu));
        assert!(h.reason.as_deref().unwrap().contains("nicht gefunden"));
        assert!(h.hint.as_deref().unwrap().contains("xn-ptts"));
        assert!(h.hint.as_deref().unwrap().contains("Modellseite"));
        contract_suite(Arc::new(p), ContractExpect::unavailable()).await;
    }

    // ---- Beschreibung und Lizenz --------------------------------------------

    #[test]
    fn beschreibung_ist_ehrlich_ueber_geraet_streaming_und_opt_in() {
        let d = PhononProvider::new(PhononConfig::default()).descriptor();
        assert_eq!(d.id, "phonon");
        assert_eq!(d.tier, ProviderTier::Experimental);
        assert!(d.opt_in);
        assert_eq!(d.caps.streaming, StreamingMode::Sentence);
        assert_eq!(
            d.caps.devices,
            vec![Device::Cpu],
            "CUDA wird nicht behauptet"
        );
        assert_eq!(d.caps.sample_rates, vec![24_000]);
        assert_eq!(d.caps.formats, vec![AudioEncoding::Wav]);
        assert!(!d.caps.cloning);
        assert_eq!(d.caps.languages, vec!["de".to_string()]);
        assert!(d.caps.platforms.iter().any(|p| p == "windows-x64"));
        let multi = PhononProvider::new(PhononConfig {
            lang: "none".into(),
            ..Default::default()
        })
        .descriptor();
        assert_eq!(multi.caps.languages.len(), 5);
    }

    #[test]
    fn ein_installiertes_katalogmodell_kennt_seine_lizenz_aus_dem_katalog() {
        use crate::catalog::{self, Purpose};
        let lic = |base: &Path, id: &str, marker: bool| -> LicenseInfo {
            let dir = base.join("tts").join("phonon").join("models").join(id);
            std::fs::create_dir_all(&dir).unwrap();
            if marker {
                let entry = catalog::tts_entries(Purpose::PhononModel)
                    .into_iter()
                    .find(|e| e.id == id)
                    .unwrap();
                phonon_models::write_marker(&dir, &entry).unwrap();
            }
            PhononProvider::new(PhononConfig {
                model: dir.display().to_string(),
                ..Default::default()
            })
            .descriptor()
            .license
        };
        let tmp = TempDir::new().unwrap();
        let de = lic(tmp.path(), "phonon-40m-de", true);
        assert!(de.verified && de.non_commercial);
        assert_eq!(de.verified_on.as_deref(), Some("2026-10-27"));
        assert!(de.source_url.contains("gradium/phonon-40m-de"));
        assert!(de.weights.contains("CC-BY-NC-4.0"));
        assert_eq!(de.code, "MIT OR Apache-2.0");
        let en = lic(tmp.path(), "phonon-31m-en", true);
        assert!(en.verified && !en.non_commercial);
        assert!(en.weights.contains("Apache-2.0"));

        // Derselbe Ordnername ohne Marke (von Hand abgelegte Gewichte): ungeprueft, vorsorglich nichtkommerziell.
        let other = TempDir::new().unwrap();
        let unmarked = lic(other.path(), "phonon-31m-en", false);
        assert!(
            !unmarked.verified && unmarked.non_commercial,
            "{unmarked:?}"
        );
    }

    #[tokio::test]
    async fn ist_phonon_nicht_installiert_nennt_der_hinweis_die_modellseite() {
        // Nichts eingetragen und nichts installiert: der Grund sagt es, der Hinweis nennt den Weg.
        let p = PhononProvider::new(PhononConfig::default()).with_ram_gate(|_| Ok(64 * 1024));
        let h = p.health_check().await;
        assert!(!h.available);
        let reason = h.reason.unwrap();
        assert!(
            reason.contains("nicht installiert") && reason.contains("Modellseite"),
            "{reason}"
        );
        assert!(!reason.contains("exe_path fehlt"), "{reason}");
        assert!(h.hint.unwrap().contains("Modellseite"));

        // Programm da, kein Modell eingetragen: dasselbe fuer das Modell.
        let fake = fake_ptts(FakeOpts::default());
        let p = PhononProvider::new(PhononConfig {
            exe_path: fake.exe.display().to_string(),
            ..PhononConfig::default()
        })
        .with_ram_gate(|_| Ok(64 * 1024));
        let h = p.health_check().await;
        let reason = h.reason.unwrap();
        assert!(
            reason.contains("model fehlt") && reason.contains("Modellseite"),
            "{reason}"
        );

        // Eingetragenes Programm fehlt: der Hinweis nennt die Modellseite und weiter den Release.
        let model = model_dir(&[]);
        let p = PhononProvider::new(PhononConfig {
            exe_path: "Z:/gibt/es/nicht/ptts.exe".to_string(),
            model: model.path().display().to_string(),
            ..PhononConfig::default()
        });
        let hint = p.health_check().await.hint.unwrap();
        assert!(
            hint.contains("Modellseite") && hint.contains("xn-ptts"),
            "{hint}"
        );
    }

    #[test]
    fn lizenz_ist_nur_fuer_die_recherchierten_gradium_modelle_verifiziert() {
        let lic = |model: &str| {
            PhononProvider::new(PhononConfig {
                model: model.into(),
                ..Default::default()
            })
            .descriptor()
            .license
        };
        let nc = lic("hf://gradium/phonon-40m-de@0123456789abcdef0123456789abcdef01234567");
        assert!(nc.verified && nc.non_commercial);
        assert_eq!(nc.verified_on.as_deref(), Some("2026-10-26"));
        assert!(nc.weights.contains("CC BY-NC 4.0") && nc.note.contains("HF_TOKEN"));
        assert!(nc.source_url.starts_with("https://huggingface.co/gradium/"));
        assert_eq!(nc.code, "MIT OR Apache-2.0");

        let ml = lic("hf://gradium/phonon-40m-multilingual");
        assert!(ml.verified && ml.non_commercial);

        let apache = lic("hf://gradium/phonon-31m-en");
        assert!(apache.verified && !apache.non_commercial);
        assert!(apache.weights.contains("Apache-2.0"));

        // Alles andere: ungeprüft und vorsorglich nichtkommerziell.
        for model in [
            "C:/Modelle/phonon-31m-en",
            "hf://fremd/phonon-31m-en",
            "hf://gradium/etwas-anderes",
            "",
        ] {
            let l = lic(model);
            assert!(!l.verified, "{model}");
            assert!(l.verified_on.is_none() && l.non_commercial, "{model}");
            assert!(!l.note.is_empty() && !l.source_url.is_empty(), "{model}");
        }
    }

    #[test]
    fn cache_tag_unterscheidet_modell_quant_sprache_und_stimme() {
        let tag = |model: &str, quant: Option<&str>, lang: &str, voice: Option<&str>| {
            PhononProvider::new(PhononConfig {
                model: model.into(),
                quant: quant.map(String::from),
                lang: lang.into(),
                ..Default::default()
            })
            .cache_tag(voice)
        };
        let base = tag("hf://gradium/phonon-40m-de", None, "de", Some("a"));
        assert!(base.starts_with("phonon:phonon-40m-de~"), "{base}");
        assert!(base.ends_with(":a"));
        assert_eq!(
            base,
            tag("hf://gradium/phonon-40m-de", None, "de", Some("a"))
        );
        assert_ne!(
            base,
            tag("hf://gradium/phonon-40m-de", None, "de", Some("b"))
        );
        assert_ne!(
            base,
            tag("hf://gradium/phonon-40m-de", Some("q8"), "de", Some("a"))
        );
        assert_ne!(
            base,
            tag("hf://gradium/phonon-40m-de", None, "en", Some("a"))
        );
        assert_ne!(
            tag("C:/a/modell", None, "de", None),
            tag("C:/b/modell", None, "de", None),
            "gleicher Ordnername, anderer Ort"
        );
        assert!(tag("C:\\Pfad mit Leerzeichen\\m", None, "de", None).starts_with("phonon:m~"));
    }

    // ---- Konfiguration --------------------------------------------------------

    #[test]
    fn konfiguration_hat_sichere_vorgaben_und_liest_teilweise_json() {
        let d = PhononConfig::default();
        assert!(!d.allow_network && d.quant.is_none() && d.threads.is_none());
        assert_eq!(d.lang, "de");
        let c: PhononConfig =
            serde_json::from_str(r#"{"exe_path":"C:/p/ptts.exe","model":"hf://a/b"}"#).unwrap();
        assert_eq!(c.lang, "de");
        assert!(!c.allow_network);
        let c: PhononConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c, PhononConfig::default());
        // Standard ist ungültig (nichts konfiguriert) und sagt, was fehlt.
        let e = PhononConfig::default().validate().unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::InvalidConfig);
        assert!(e.message.contains("exe_path"));
    }

    #[test]
    fn validate_nennt_den_konkreten_fehler() {
        let ok = PhononConfig {
            exe_path: "ptts".into(),
            model: "hf://gradium/phonon-40m-de".into(),
            quant: Some("q8".into()),
            voice: Some("Ava".into()),
            threads: Some(4),
            hf_home: Some("D:/hf".into()),
            ..Default::default()
        };
        ok.validate().unwrap();
        let bad = |mutate: &dyn Fn(&mut PhononConfig), needle: &str| {
            let mut c = ok.clone();
            mutate(&mut c);
            let e = c.validate().expect_err(needle);
            assert_eq!(e.kind, TtsErrorKind::InvalidConfig, "{needle}: {e:?}");
            assert!(!e.retryable);
            assert!(
                e.message.contains(needle),
                "{needle} fehlt in {:?}",
                e.message
            );
        };
        bad(&|c| c.exe_path = "  ".into(), "exe_path");
        bad(&|c| c.exe_path = "a\0b".into(), "exe_path");
        bad(&|c| c.model = "".into(), "model fehlt");
        bad(&|c| c.model = "-x".into(), "model");
        bad(
            &|c| c.model = "hf://nur-ein-teil".into(),
            "hf://OWNER/MODEL",
        );
        bad(&|c| c.model = "hf://a/b/c".into(), "hf://OWNER/MODEL");
        bad(&|c| c.model = "hf://a/b@".into(), "Revision");
        bad(&|c| c.model = "hf://a/b@../x".into(), "Revision");
        bad(&|c| c.model = "hf://../b".into(), "Besitzer");
        bad(&|c| c.quant = Some("q4".into()), "quant");
        bad(&|c| c.lang = "xx".into(), "lang");
        bad(&|c| c.voice = Some("-evil".into()), "voice");
        bad(&|c| c.voice = Some("  ".into()), "voice");
        bad(&|c| c.threads = Some(0), "threads");
        bad(&|c| c.threads = Some(1000), "threads");
        bad(&|c| c.hf_home = Some(" ".into()), "hf_home");
        // Leeres quant heißt f32; Groß-/Kleinschreibung bei lang/quant egal.
        let mut c = ok.clone();
        c.quant = Some("".into());
        c.validate().unwrap();
        assert_eq!(c.quant_arg(), None);
        c.quant = Some("Q8".into());
        c.lang = "DE".into();
        c.validate().unwrap();
        assert_eq!(c.quant_arg(), Some("q8"));
        assert_eq!(c.lang_arg(), "de");
    }

    #[test]
    fn modellverweis_wird_zerlegt() {
        assert_eq!(
            parse_model("hf://gradium/phonon-40m-de@abc").unwrap(),
            ModelSpec::Hf {
                owner: "gradium",
                name: "phonon-40m-de",
                rev: Some("abc")
            }
        );
        assert_eq!(
            parse_model(" hf://a/b ").unwrap(),
            ModelSpec::Hf {
                owner: "a",
                name: "b",
                rev: None
            }
        );
        assert_eq!(
            parse_model("C:\\Modelle\\x").unwrap(),
            ModelSpec::Local("C:\\Modelle\\x")
        );
        assert!(matches!(
            parse_model("hf://a/b@refs/pr/1"),
            Ok(ModelSpec::Hf { .. })
        ));
    }

    #[test]
    fn serde_und_typexport_funktionieren() {
        let mut types = specta::TypeCollection::default();
        types.register::<PhononConfig>();
        let ts = specta_typescript::Typescript::default()
            .bigint(specta_typescript::BigIntExportBehavior::Number)
            .export(&types)
            .unwrap();
        for needle in ["PhononConfig", "exe_path: string", "allow_network: boolean"] {
            assert!(ts.contains(needle), "{needle} fehlt in:\n{ts}");
        }
    }

    // ---- Aufruf des Programms -------------------------------------------------

    #[tokio::test]
    async fn aufruf_enthaelt_alle_optionen_in_der_vorgesehenen_reihenfolge() {
        // Ohne Netzfreigabe sind nur Stimmen aus `<Modell>/voices/` erlaubt.
        let model = model_dir(&["Ava", "Ben"]);
        let fake = fake_ptts(FakeOpts::default());
        let mut c = cfg(&fake, &model);
        c.quant = Some("q8".into());
        c.voice = Some("Ava".into());
        c.threads = Some(3);
        c.lang = "de".into();
        let p = PhononProvider::new(c).with_ram_gate(|_| Ok(64 * 1024));
        p.synthesize("Hallo Welt.", &opts(), &CancelToken::detached())
            .await
            .unwrap();
        let calls = fake.synth_calls();
        assert_eq!(calls.len(), 1);
        let a = &calls[0].args;
        let model_arg = model.path().display().to_string();
        assert_eq!(a[0..2], ["--model".to_string(), model_arg]);
        assert_eq!(a[2..4], ["--lang", "de"]);
        assert_eq!(a[4..6], ["--quant", "q8"]);
        assert_eq!(a[6..8], ["--voice", "Ava"]);
        assert_eq!(a[8..10], ["--threads", "3"]);
        assert_eq!(a[10], "Hallo Welt.");
        assert_eq!(a[11], "-o");
        assert!(a[12].ends_with("out.wav"));
        assert_eq!(a.len(), 13);
        // Der Arbeitsordner ist weg.
        assert!(!Path::new(&a[12]).parent().unwrap().exists());

        // Ohne Stimme/Quant/Threads fehlen die Optionen; opts.voice schlägt die Konfiguration,
        // "default" heißt Standardstimme.
        let p = provider(&fake, &model);
        p.synthesize("Eins.", &opts(), &CancelToken::detached())
            .await
            .unwrap();
        let mut o = opts();
        o.voice = Some("Ben".into());
        p.synthesize("Zwei.", &o, &CancelToken::detached())
            .await
            .unwrap();
        let mut c2 = cfg(&fake, &model);
        c2.voice = Some("Ava".into());
        let p2 = PhononProvider::new(c2).with_ram_gate(|_| Ok(64 * 1024));
        o.voice = Some("default".into());
        p2.synthesize("Drei.", &o, &CancelToken::detached())
            .await
            .unwrap();
        let calls = fake.synth_calls();
        assert!(!calls[1].has("--voice") && !calls[1].has("--quant") && !calls[1].has("--threads"));
        assert_eq!(calls[2].value_of("--voice"), Some("Ben"));
        assert!(!calls[3].has("--voice"), "default = kein --voice");
    }

    #[tokio::test]
    async fn text_kommt_unveraendert_an() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts::default());
        let p = provider(&fake, &model);
        let cases: Vec<(&str, &str)> = vec![
            (
                r#"Sie sagte "Hallo" und 'Tschüss'."#,
                r#"Sie sagte "Hallo" und 'Tschüss'."#,
            ),
            (
                r#"Pfad C:\Daten\ und \\Server\Freigabe\"#,
                r#"Pfad C:\Daten\ und \\Server\Freigabe\"#,
            ),
            (
                r#"Endet mit Backslash und Anführung \""#,
                r#"Endet mit Backslash und Anführung \""#,
            ),
            (
                "Größe: 50 % – äöüß ÄÖÜ, 日本語 und 😀",
                "Größe: 50 % – äöüß ÄÖÜ, 日本語 und 😀",
            ),
            ("-12 Grad sind kalt.", " -12 Grad sind kalt."),
            ("--help", " --help"),
            ("-o", " -o"),
            ("@datei.txt lesen", " @datei.txt lesen"),
            ("  umgeben von Leerraum \n", "umgeben von Leerraum"),
            (
                "Zeile eins\nZeile zwei\tTab\r\nDrei",
                "Zeile eins Zeile zwei Tab  Drei",
            ),
            ("NUL\u{0}mitten", "NUL mitten"),
            (
                "a & b | c > d < e ^ f %PATH% $(x) `y`",
                "a & b | c > d < e ^ f %PATH% $(x) `y`",
            ),
        ];
        for (input, _) in &cases {
            p.synthesize(input, &opts(), &CancelToken::detached())
                .await
                .unwrap_or_else(|e| panic!("{input:?}: {e:?}"));
        }
        let calls = fake.synth_calls();
        assert_eq!(calls.len(), cases.len());
        for ((input, expected), call) in cases.iter().zip(&calls) {
            assert_eq!(call.text(), Some(*expected), "Eingabe {input:?}");
        }
    }

    #[tokio::test]
    async fn zu_langer_text_ist_bad_request_ohne_prozessstart() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts::default());
        let p = provider(&fake, &model);
        let e = p
            .synthesize(
                &"a".repeat(MAX_TEXT_CHARS + 1),
                &opts(),
                &CancelToken::detached(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
        assert!(fake.calls().is_empty());
        // Auch die Stimme aus opts wird geprüft.
        let mut o = opts();
        o.voice = Some("-x".into());
        let e = p
            .synthesize("Hallo", &o, &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
        assert!(fake.calls().is_empty());
    }

    // ---- Datenschutz ----------------------------------------------------------

    #[tokio::test]
    async fn offline_ist_standard_und_allow_network_hebt_die_sperre_auf() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts::default());
        let p = provider(&fake, &model);
        p.synthesize("Hallo.", &opts(), &CancelToken::detached())
            .await
            .unwrap();
        let c = &fake.synth_calls()[0];
        assert_eq!(c.env["HF_HUB_OFFLINE"], "1");
        assert_eq!(c.env["HF_HOME"], "unset");

        let mut cfg2 = cfg(&fake, &model);
        cfg2.hf_home = Some(model.path().join("hf").display().to_string());
        cfg2.allow_network = true;
        let p = PhononProvider::new(cfg2.clone()).with_ram_gate(|_| Ok(64 * 1024));
        p.synthesize("Hallo.", &opts(), &CancelToken::detached())
            .await
            .unwrap();
        let c = &fake.synth_calls()[1];
        assert_ne!(
            c.env["HF_HUB_OFFLINE"], "1",
            "allow_network hebt die Sperre auf"
        );
        assert_eq!(c.env["HF_HOME"], cfg2.hf_home.unwrap());

        // Offline mit hf_home: HF_HOME kommt trotzdem an.
        let mut cfg3 = cfg(&fake, &model);
        cfg3.hf_home = Some("D:/hf-cache".into());
        let p = PhononProvider::new(cfg3).with_ram_gate(|_| Ok(64 * 1024));
        p.synthesize("Hallo.", &opts(), &CancelToken::detached())
            .await
            .unwrap();
        let c = &fake.synth_calls()[2];
        assert_eq!(c.env["HF_HUB_OFFLINE"], "1");
        assert_eq!(c.env["HF_HOME"], "D:/hf-cache");
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn hf_cache_with(owner: &str, name: &str, commit: &str, refs: &[(&str, &str)]) -> TempDir {
        let hf = TempDir::new().unwrap();
        let repo = hf
            .path()
            .join("hub")
            .join(format!("models--{owner}--{name}"));
        std::fs::create_dir_all(repo.join("snapshots").join(commit)).unwrap();
        std::fs::write(
            repo.join("snapshots").join(commit).join("config.json"),
            b"{}",
        )
        .unwrap();
        for (r, target) in refs {
            let p = repo.join("refs").join(r);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, target).unwrap();
        }
        hf
    }

    #[test]
    fn cache_ort_und_schnappschuss_werden_wie_beim_kindprozess_aufgeloest() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        let dir = |hf: Option<&str>, e: &dyn Fn(&str) -> Option<String>| hub_cache_dir(hf, e);
        assert_eq!(
            dir(Some("D:/cfg"), &env(&[("HF_HOME", "E:/env")])),
            Some(PathBuf::from("D:/cfg").join("hub")),
            "Konfiguration schlägt HF_HOME"
        );
        assert_eq!(
            dir(None, &env(&[("HF_HOME", "E:/env")])),
            Some(PathBuf::from("E:/env").join("hub"))
        );
        assert_eq!(
            dir(Some("D:/cfg"), &env(&[("HF_HUB_CACHE", "F:/c")])),
            Some(PathBuf::from("F:/c")),
            "HF_HUB_CACHE schlägt alles (der Kindprozess erbt es)"
        );
        assert_eq!(
            dir(None, &env(&[("HOME", "/home/x")])),
            Some(PathBuf::from("/home/x/.cache/huggingface/hub"))
        );
        assert_eq!(
            dir(None, &env(&[("USERPROFILE", "C:/Users/x")])),
            Some(PathBuf::from("C:/Users/x/.cache/huggingface/hub"))
        );
        assert_eq!(dir(None, &env(&[])), None);

        let commit = "0123456789abcdef0123456789abcdef01234567";
        let hf = hf_cache_with(
            "gradium",
            "phonon-40m-de",
            commit,
            &[("main", commit), ("v1", commit)],
        );
        let cache = hf.path().join("hub");
        assert!(snapshot_present(&cache, "gradium", "phonon-40m-de", None));
        assert!(snapshot_present(
            &cache,
            "gradium",
            "phonon-40m-de",
            Some(commit)
        ));
        assert!(snapshot_present(
            &cache,
            "gradium",
            "phonon-40m-de",
            Some("v1")
        ));
        assert!(!snapshot_present(
            &cache,
            "gradium",
            "phonon-40m-de",
            Some("v2")
        ));
        assert!(!snapshot_present(
            &cache,
            "gradium",
            "phonon-40m-de",
            Some("ffffffffffffffffffffffffffffffffffffffff")
        ));
        assert!(!snapshot_present(&cache, "gradium", "anderes", None));
        // Ein Ref, der aus dem Cache hinauszeigt, gilt nicht.
        let evil = hf_cache_with("a", "b", "x", &[("main", "../../etc")]);
        assert!(!snapshot_present(&evil.path().join("hub"), "a", "b", None));
    }

    #[tokio::test]
    async fn offline_ohne_cache_startet_nie_den_prozess() {
        let fake = fake_ptts(FakeOpts::default());
        let hf = TempDir::new().unwrap();
        let mk = |model: &str, allow: bool, hf_home: &Path| {
            PhononProvider::new(PhononConfig {
                exe_path: fake.exe.display().to_string(),
                model: model.into(),
                hf_home: Some(hf_home.display().to_string()),
                allow_network: allow,
                ..Default::default()
            })
            .with_ram_gate(|_| Ok(64 * 1024))
            .with_env(no_env)
        };
        let model = "hf://gradium/phonon-40m-de";

        // Kein Cache, Downloads aus: nicht verfügbar, mit Hinweis, kein Start.
        let p = mk(model, false, hf.path());
        let h = p.health_check().await;
        assert!(!h.available);
        assert!(h.reason.unwrap().contains("nicht im Hugging-Face-Cache"));
        let hint = h.hint.unwrap();
        assert!(hint.contains("HF_TOKEN") && hint.contains("CC BY-NC"));
        let e = p
            .synthesize("Hallo.", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Unavailable);
        assert!(
            fake.calls().is_empty(),
            "kein Prozessstart ohne Modell im Cache"
        );

        // Mit Cache: bereit, Hinweis nennt den Offline-Betrieb.
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let cached = hf_cache_with("gradium", "phonon-40m-de", commit, &[("main", commit)]);
        let p = mk(model, false, cached.path());
        let h = p.health_check().await;
        assert!(h.available, "{h:?}");
        assert!(h.hint.unwrap().contains("nur aus dem Hugging-Face-Cache"));
        assert_eq!(h.version.as_deref(), Some("0.4.2"));

        // Netzfreigabe: bereit, auch ohne Cache; der Hinweis sagt, dass geladen wird.
        let p = mk(model, true, hf.path());
        let h = p.health_check().await;
        assert!(h.available);
        assert!(h.hint.unwrap().contains("aus dem Netz"));
    }

    // ---- Stimmen ohne Netzfreigabe (Befund 4) ---------------------------------

    /// Ohne `allow_network` darf `--voice` nichts nachladen: erlaubt sind
    /// `default`, ein Name aus `<Modell>/voices/` und ein existierender
    /// `.safetensors`-Pfad. Jeder Katalogname könnte `ptts` sonst von Hugging
    /// Face holen lassen (`HF_HUB_OFFLINE` ist für ptts nicht dokumentiert).
    #[tokio::test]
    async fn ohne_netzfreigabe_sind_nur_default_modellstimmen_und_pfade_erlaubt() {
        let model = model_dir(&["Ava"]);
        let fake = fake_ptts(FakeOpts::default());
        let p = provider(&fake, &model);
        let say = |voice: Option<&str>| {
            let mut o = opts();
            o.voice = voice.map(String::from);
            let p = &p;
            async move { p.synthesize("Hallo.", &o, &CancelToken::detached()).await }
        };

        // Ein Katalogname: nicht verfügbar, ohne Prozessstart, mit Hinweis.
        let e = say(Some("Zoey")).await.unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Unavailable, "{e:?}");
        assert!(e.message.contains("Zoey"), "{}", e.message);
        assert!(
            e.message.contains("allow_network") && e.message.contains("voices"),
            "der Hinweis nennt den Ausweg: {}",
            e.message
        );
        assert!(fake.calls().is_empty(), "kein Aufruf von ptts");

        // Pfadtricks sind kein Modellname: weder `../` noch Unterordner.
        for tricky in [
            "../Ava",
            "sub/Ava",
            "..\\Ava",
            "Ava.safetensors/..",
            // Netzwerkpfade sind selbst ein Zugriff über das Netz.
            "\\\\server\\freigabe\\Ava.safetensors",
            "//server/freigabe/Ava.safetensors",
        ] {
            let e = say(Some(tricky)).await.unwrap_err();
            assert_eq!(e.kind, TtsErrorKind::Unavailable, "{tricky}: {e:?}");
        }
        // Ein `.safetensors`-Pfad, den es nicht gibt.
        let missing = model.path().join("gibt-es-nicht.safetensors");
        let e = say(Some(&missing.display().to_string())).await.unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Unavailable, "{e:?}");
        assert!(fake.calls().is_empty(), "weiterhin kein Aufruf von ptts");

        // Erlaubt: Standardstimme, Stimme des Modellordners, vorhandene Datei.
        say(Some("default")).await.unwrap();
        say(Some("Ava")).await.unwrap();
        let file = model.path().join("voices").join("Ava.safetensors");
        say(Some(&file.display().to_string())).await.unwrap();
        say(None).await.unwrap();
        let calls = fake.synth_calls();
        assert_eq!(calls.len(), 4);
        assert!(!calls[0].has("--voice"), "default = kein --voice");
        assert_eq!(calls[1].value_of("--voice"), Some("Ava"));
        assert_eq!(
            calls[2].value_of("--voice"),
            Some(file.display().to_string().as_str())
        );
        assert!(!calls[3].has("--voice"));
    }

    /// Die konfigurierte Stimme folgt derselben Regel; die Gesundheit sagt es früh.
    #[tokio::test]
    async fn eine_konfigurierte_katalogstimme_ohne_netzfreigabe_ist_nicht_verfuegbar() {
        let model = model_dir(&["Ava"]);
        let fake = fake_ptts(FakeOpts::default());
        let mut c = cfg(&fake, &model);
        c.voice = Some("Zoey".into());
        let p = PhononProvider::new(c.clone()).with_ram_gate(|_| Ok(64 * 1024));
        let e = p
            .synthesize("Hallo.", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Unavailable);
        assert!(fake.calls().is_empty());
        let h = p.health_check().await;
        assert!(!h.available);
        assert!(h.reason.unwrap().contains("Zoey"));
        // Eine Stimme des Modellordners ist in Ordnung (auch in der Gesundheit).
        c.voice = Some("Ava".into());
        let p = PhononProvider::new(c).with_ram_gate(|_| Ok(64 * 1024));
        assert!(p.health_check().await.available);
        p.synthesize("Hallo.", &opts(), &CancelToken::detached())
            .await
            .unwrap();
    }

    /// Mit `allow_network` bleibt es wie bisher: der Katalog darf laden.
    /// Bei einem `hf://`-Modell ohne Netzfreigabe gibt es keinen Modellordner —
    /// dort nur `default` und Dateipfade.
    #[tokio::test]
    async fn mit_netzfreigabe_darf_jede_stimme_ohne_sie_nicht_bei_einem_hf_modell() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts::default());
        let mut c = cfg(&fake, &model);
        c.allow_network = true;
        let p = PhononProvider::new(c).with_ram_gate(|_| Ok(64 * 1024));
        let mut o = opts();
        o.voice = Some("Zoey".into());
        p.synthesize("Hallo.", &o, &CancelToken::detached())
            .await
            .unwrap();
        assert_eq!(fake.synth_calls()[0].value_of("--voice"), Some("Zoey"));

        // hf://-Modell, im Cache vorhanden, Downloads aus.
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let cached = hf_cache_with("gradium", "phonon-40m-de", commit, &[("main", commit)]);
        let offline = PhononProvider::new(PhononConfig {
            exe_path: fake.exe.display().to_string(),
            model: "hf://gradium/phonon-40m-de".into(),
            hf_home: Some(cached.path().display().to_string()),
            ..Default::default()
        })
        .with_ram_gate(|_| Ok(64 * 1024))
        .with_env(no_env);
        let before = fake.synth_calls().len();
        let e = offline
            .synthesize("Hallo.", &o, &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Unavailable, "{e:?}");
        assert_eq!(fake.synth_calls().len(), before, "kein Prozessstart");
        let mut d = opts();
        d.voice = Some("default".into());
        offline
            .synthesize("Hallo.", &d, &CancelToken::detached())
            .await
            .unwrap();
        assert_eq!(fake.synth_calls().len(), before + 1);
    }

    // ---- Katalog-Frist und Lizenznotiz (Befunde 7, 10) -----------------------

    /// Befund 7: hängt der Katalog (`--list-voices`), kommt nach der INNEREN
    /// Frist die lokale Teilliste — nicht erst nichts nach der äußeren.
    #[tokio::test]
    async fn haengt_der_stimmenkatalog_kommt_die_lokale_teilliste_nach_der_inneren_frist() {
        let model = model_dir(&["Lokal"]);
        let fake = fake_ptts(FakeOpts {
            voices_out: Some("Ava de\nBen en\n".to_string()),
            voices_delay_ms: Some(4000),
            ..Default::default()
        });
        let mut c = cfg(&fake, &model);
        c.allow_network = true;
        let p = PhononProvider::new(c)
            .with_ram_gate(|_| Ok(64 * 1024))
            .with_list_voices_timeout(Duration::from_millis(300));
        let started = Instant::now();
        let v = p.list_voices(None).await.unwrap();
        let took = started.elapsed();
        assert!(
            took < Duration::from_secs(3),
            "die innere Frist greift: {took:?}"
        );
        // Der Katalog wurde versucht: die Liste wartete die innere Frist ab. (Das
        // Protokoll des Fakes taugt dafür nicht — auf einem ausgelasteten Rechner
        // startet der Prozess manchmal erst NACH der 300-ms-Frist.)
        assert!(
            took >= Duration::from_millis(250),
            "die Liste wartete die innere Frist ab: {took:?}"
        );
        let ids: Vec<&str> = v.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["default", "Lokal"], "nur die lokale Teilliste");
    }

    /// Befund 10: ohne Modell steht in der Lizenznotiz nicht „von '' …“.
    #[test]
    fn die_lizenznotiz_ohne_modell_sagt_kein_modell_eingetragen() {
        for model in ["", "   "] {
            let l = PhononProvider::new(PhononConfig {
                model: model.into(),
                ..Default::default()
            })
            .descriptor()
            .license;
            assert!(
                !l.note.contains("''"),
                "leeres Modell in Anführungszeichen: {}",
                l.note
            );
            assert!(l.note.contains("kein Modell eingetragen"), "{}", l.note);
            assert!(!l.verified && l.non_commercial);
        }
        // Mit Modell bleibt der Name in der Notiz.
        let l = PhononProvider::new(PhononConfig {
            model: "C:/Modelle/x".into(),
            ..Default::default()
        })
        .descriptor()
        .license;
        assert!(l.note.contains("C:/Modelle/x"), "{}", l.note);
    }

    /// Befund 2: der Gegenstand der Guardrails ist das Modell dieser Instanz.
    #[test]
    fn die_instanz_meldet_ihr_modell_als_gegenstand_der_pruefung() {
        let p = PhononProvider::new(PhononConfig {
            model: "  hf://gradium/phonon-31m-en  ".into(),
            ..Default::default()
        });
        assert_eq!(
            p.use_subject(),
            UseSubject {
                model: "hf://gradium/phonon-31m-en".into(),
                base_url: None
            }
        );
    }

    // ---- Modell und Gesundheit ------------------------------------------------

    #[tokio::test]
    async fn modellpruefung_nennt_fehlenden_ordner_und_dateien() {
        let fake = fake_ptts(FakeOpts::default());
        let mk = |model: String| {
            PhononProvider::new(PhononConfig {
                exe_path: fake.exe.display().to_string(),
                model,
                ..Default::default()
            })
            .with_ram_gate(|_| Ok(64 * 1024))
        };
        let h = mk("Z:/gibt/es/nicht".into()).health_check().await;
        assert!(h.reason.unwrap().contains("Modellordner fehlt"));

        let leer = TempDir::new().unwrap();
        let h = mk(leer.path().display().to_string()).health_check().await;
        assert!(h.reason.unwrap().contains("config.json"));
        std::fs::write(leer.path().join("config.json"), "{}").unwrap();
        let h = mk(leer.path().display().to_string()).health_check().await;
        assert!(h.reason.unwrap().contains("tokenizer.json"));

        let voll = model_dir(&[]);
        let h = mk(voll.path().display().to_string()).health_check().await;
        assert!(h.available, "{h:?}");
        assert_eq!(h.device, Some(Device::Cpu));
        assert_eq!(h.version.as_deref(), Some("0.4.2"));
        assert!(h.reason.is_none());
        assert!(
            fake.synth_calls().is_empty(),
            "Health startet keine Synthese"
        );
    }

    #[tokio::test]
    async fn health_liest_die_version_tolerant() {
        let model = model_dir(&[]);
        let version_of = |out: &str| {
            let fake = fake_ptts(FakeOpts {
                version_out: Some(out.to_string()),
                ..Default::default()
            });
            let p = provider(&fake, &model);
            async move { (p.health_check().await, fake) }
        };
        let (h, _f) = version_of("ptts-cli v0.4.2 (abc123)").await;
        assert!(h.available, "{h:?}");
        assert_eq!(h.version.as_deref(), Some("0.4.2"));
        // Ohne Ziffern: die erste Zeile wörtlich.
        let (h, _f) = version_of("kaputt").await;
        assert!(h.available);
        assert_eq!(h.version.as_deref(), Some("kaputt"));
    }

    #[tokio::test]
    async fn health_meldet_defekte_programme() {
        let model = model_dir(&[]);
        // `--version` endet mit einem Fehlercode.
        let fake = fake_ptts(FakeOpts {
            version_code: Some(3),
            ..Default::default()
        });
        let h = provider(&fake, &model).health_check().await;
        assert!(!h.available);
        assert!(
            h.reason.as_deref().unwrap().contains("endete mit Code 3"),
            "{h:?}"
        );
        assert!(h.hint.is_some());

        // Keine ausführbare Datei (eine Textdatei).
        let text_file = model.path().join("config.json");
        let p = PhononProvider::new(PhononConfig {
            exe_path: text_file.display().to_string(),
            model: model.path().display().to_string(),
            ..Default::default()
        });
        let h = p.health_check().await;
        assert!(!h.available);
        assert!(h.reason.is_some() && h.hint.is_some());
        assert_eq!(h.device, Some(Device::Cpu));
    }

    #[tokio::test]
    async fn health_beendet_ein_haengendes_version_programm_nach_der_frist() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts {
            version_delay_ms: Some(2500),
            ..Default::default()
        });
        let p = provider(&fake, &model).with_version_timeout(Duration::from_millis(300));
        let started = Instant::now();
        let h = p.health_check().await;
        assert!(!h.available);
        assert!(
            h.reason
                .as_deref()
                .unwrap()
                .contains("antwortet nicht innerhalb"),
            "{h:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        tokio::time::sleep(Duration::from_millis(2600)).await;
        assert!(
            !fake.marker.exists(),
            "hängendes ptts --version lief weiter"
        );
    }

    // ---- Fehlschläge des Programms --------------------------------------------

    #[tokio::test]
    async fn fehlschlaege_werden_nach_klasse_eingeordnet() {
        let model = model_dir(&[]);
        let run = |opts: FakeOpts| {
            let model = &model;
            async move {
                let fake = fake_ptts(opts);
                let p = provider(&fake, model);
                let e = p
                    .synthesize("Hallo.", &opts_default(), &CancelToken::detached())
                    .await
                    .expect_err("Fehlschlag erwartet");
                (e, fake)
            }
        };
        fn opts_default() -> SynthesisOptions {
            SynthesisOptions {
                timeout: Duration::from_secs(30),
                ..Default::default()
            }
        }

        // Gesperrtes Modell.
        let (e, _f) = run(FakeOpts::failing(
            1,
            "Error: 403 Forbidden: gated repo, please accept the terms",
        ))
        .await;
        assert_eq!(e.kind, TtsErrorKind::LicenseBlocked);
        assert!(!e.retryable);
        assert!(e.message.contains("HF_TOKEN") && e.message.contains("CC BY-NC 4.0"));

        // Fehlendes Modell.
        let (e, _f) = run(FakeOpts::failing(
            1,
            "Error: model not found: config.json (os error 2)",
        ))
        .await;
        assert_eq!(e.kind, TtsErrorKind::Unavailable);
        assert!(e.message.contains("config.json"));

        // Sonstiger Fehler: Backend, mit Code und Auszug vom ENDE der Meldung.
        let long = format!("{} Ursache am Ende: Speicher alle", "x ".repeat(400));
        let (e, _f) = run(FakeOpts::failing(7, &long)).await;
        assert_eq!(e.kind, TtsErrorKind::Backend);
        assert!(!e.retryable);
        assert!(e.message.contains("Code 7"), "{}", e.message);
        assert!(e.message.contains("Speicher alle"), "{}", e.message);
        assert!(e.message.chars().count() < 450, "Auszug ist gekürzt");

        // Ohne Meldung.
        let (e, _f) = run(FakeOpts::failing(9, "")).await;
        assert_eq!(e.kind, TtsErrorKind::Backend);
        assert!(e.message.contains("keine Meldung"));

        // Erfolg gemeldet, aber nichts Brauchbares geschrieben.
        for (mode, needle) in [
            ("no_output", "keine Ausgabedatei"),
            ("bad_wav", "kein brauchbares WAV"),
            ("empty_wav", "leeres Audio"),
        ] {
            let (e, f) = run(FakeOpts::mode(mode)).await;
            assert_eq!(e.kind, TtsErrorKind::Backend, "{mode}");
            assert!(e.message.contains(needle), "{mode}: {}", e.message);
            assert_eq!(f.synth_calls().len(), 1);
        }
    }

    #[tokio::test]
    async fn streaming_header_der_ausgabe_wird_verstanden() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts::mode("streaming_header"));
        let clip = provider(&fake, &model)
            .synthesize("Zehn Zeich", &opts(), &CancelToken::detached())
            .await
            .unwrap();
        // 10 Zeichen mal 40 ms.
        assert_eq!(clip.duration_ms, 400);
        assert_eq!(clip.format.sample_rate, 24_000);
    }

    #[tokio::test]
    async fn zu_grosse_ausgabe_wird_abgelehnt() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts::default());
        let p = provider(&fake, &model).with_max_wav_bytes(1000);
        let e = p
            .synthesize(
                "Ein Satz, der mehr als 1000 Bytes Audio ergibt.",
                &opts(),
                &CancelToken::detached(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);
        assert!(e.message.contains("unplausibel groß"));
    }

    // ---- Systemschutz ---------------------------------------------------------

    #[tokio::test]
    async fn ram_gate_verhindert_den_start_bei_knappem_speicher() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts::default());
        let p = PhononProvider::new(cfg(&fake, &model))
            .with_ram_gate(|_| Err("Zu wenig freier Arbeitsspeicher (Test)".to_string()));
        let e = p
            .synthesize("Hallo.", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Unavailable);
        assert!(e.message.contains("Arbeitsspeicher"));
        assert!(
            fake.calls().is_empty(),
            "bei vollem RAM darf nichts starten"
        );
        // Das echte Gate ist standardmäßig angeschlossen.
        let real = PhononProvider::new(cfg(&fake, &model));
        let gate: fn(u64) -> Result<u64, String> = process_guard::check_ram_for_start;
        assert_eq!(real.ram_gate as *const (), gate as *const ());
    }

    #[tokio::test]
    async fn arbeitsordner_nicht_anlegbar_ist_ein_backend_fehler_vor_dem_start() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts::default());
        let p = provider(&fake, &model).with_temp_base(PathBuf::from("Z:/gibt/es/nicht/temp"));
        let e = p
            .synthesize("Hallo.", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);
        assert!(e.message.contains("Arbeitsordner"), "{}", e.message);
        assert!(fake.calls().is_empty());
        // Die Gesundheit braucht ebenfalls einen Arbeitsordner und meldet das klar.
        let h = p.health_check().await;
        assert!(!h.available && h.reason.unwrap().contains("Arbeitsordner"));
    }

    #[tokio::test]
    async fn abbruch_beendet_den_kindprozess_und_raeumt_auf() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts {
            delay_ms: Some(2500),
            ..Default::default()
        });
        let p = Arc::new(provider(&fake, &model));
        let cancel = CancelToken::new("kill-test");
        let c2 = cancel.clone();
        let started = Instant::now();
        let canceller = async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            c2.cancel();
        };
        let (r, ()) =
            futures_util::future::join(p.synthesize("Ein Satz.", &opts(), &cancel), canceller)
                .await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "Abbruch kam zu spät"
        );
        // Gestartet wurde der Prozess (Protokoll vorhanden) ...
        let calls = fake.synth_calls();
        assert_eq!(calls.len(), 1, "der Fake muss gelaufen sein");
        let out = PathBuf::from(calls[0].value_of("-o").unwrap());
        // ... aber er wurde beendet, bevor er den Marker schrieb, und der
        // Arbeitsordner ist weg.
        tokio::time::sleep(Duration::from_millis(2600)).await;
        assert!(
            !fake.marker.exists(),
            "Kindprozess lief nach dem Abbruch weiter"
        );
        assert!(
            !out.parent().unwrap().exists(),
            "Arbeitsordner blieb liegen"
        );
    }

    #[tokio::test]
    async fn frist_beendet_den_kindprozess_und_ergibt_timeout() {
        // Großzügige Abstände: unter Last (voller Testlauf) startet der Fake-Prozess spät.
        // Frist 1,5 s, Fake schläft 6 s; die Prüfung bleibt scharf (Frist << Fake-Dauer).
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts {
            delay_ms: Some(6000),
            ..Default::default()
        });
        let p = provider(&fake, &model);
        let mut o = opts();
        o.timeout = Duration::from_millis(1500);
        let started = Instant::now();
        let e = p
            .synthesize("Ein Satz.", &o, &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Timeout);
        assert!(!e.retryable);
        assert!(started.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(6200)).await;
        assert!(
            !fake.marker.exists(),
            "Kindprozess lief nach der Frist weiter"
        );
        // Den Aufruf hat der Fake nur protokolliert, wenn er vor der Frist überhaupt anlief.
        if let Some(call) = fake.synth_calls().first() {
            let out = PathBuf::from(call.value_of("-o").unwrap());
            assert!(!out.parent().unwrap().exists());
        }
    }

    #[tokio::test]
    async fn abgebrochenes_future_beendet_ebenfalls_den_prozess() {
        // Task-Abbruch statt Token (z. B. App-Ende): Drop des Futures genügt.
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts {
            delay_ms: Some(2500),
            ..Default::default()
        });
        let p = Arc::new(provider(&fake, &model));
        let p2 = p.clone();
        let task = tokio::spawn(async move {
            p2.synthesize("Ein Satz.", &opts(), &CancelToken::detached())
                .await
        });
        tokio::time::sleep(Duration::from_millis(400)).await;
        task.abort();
        let _ = task.await;
        tokio::time::sleep(Duration::from_millis(2600)).await;
        assert!(!fake.marker.exists());
    }

    #[tokio::test]
    async fn parallele_auftraege_teilen_keinen_zustand() {
        let model = model_dir(&[]);
        let fake = fake_ptts(FakeOpts::default());
        let p = Arc::new(provider(&fake, &model));
        let lens = [10usize, 25, 40, 55];
        let mut tasks = Vec::new();
        for (i, n) in lens.iter().copied().enumerate() {
            let p = p.clone();
            tasks.push(tokio::spawn(async move {
                let cancel = CancelToken::new(format!("par-{i}"));
                p.synthesize(&"x".repeat(n), &opts(), &cancel).await
            }));
        }
        for (task, n) in tasks.into_iter().zip(lens) {
            let clip = task.await.unwrap().unwrap();
            assert_eq!(clip.duration_ms, n as u64 * 40, "Auftrag mit {n} Zeichen");
        }
        let calls = fake.synth_calls();
        assert_eq!(calls.len(), 4);
        let dirs: HashSet<_> = calls
            .iter()
            .map(|c| c.value_of("-o").unwrap().to_string())
            .collect();
        assert_eq!(
            dirs.len(),
            4,
            "jeder Auftrag hat einen eigenen Arbeitsordner"
        );
    }

    // ---- Stimmen --------------------------------------------------------------

    #[tokio::test]
    async fn stimmen_offline_nur_aus_dem_modellordner() {
        let model = model_dir(&["Zoe", "Ava"]);
        let fake = fake_ptts(FakeOpts::default());
        let mut c = cfg(&fake, &model);
        c.voice = Some("Konfiguriert".into());
        let p = PhononProvider::new(c).with_ram_gate(|_| Ok(64 * 1024));
        let v = p.list_voices(None).await.unwrap();
        let ids: Vec<&str> = v.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["default", "Konfiguriert", "Ava", "Zoe"]);
        assert!(v.iter().all(|v| v.language == "de"));
        assert!(
            fake.calls().is_empty(),
            "offline: kein --list-voices, kein Prozess"
        );
        // Sprachfilter: eine englische Anfrage findet die deutschen Stimmen nicht.
        assert!(p.list_voices(Some("en")).await.unwrap().is_empty());
        assert_eq!(p.list_voices(Some("de_DE")).await.unwrap().len(), 4);
        // Doppelt konfigurierte Standardstimme erscheint einmal.
        let mut c = cfg(&fake, &model);
        c.voice = Some("default".into());
        let p = PhononProvider::new(c);
        assert_eq!(p.list_voices(None).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn stimmenkatalog_nur_mit_netzfreigabe_und_tolerant_gelesen() {
        let model = model_dir(&["Lokal"]);
        let fake = fake_ptts(FakeOpts {
            voices_out: Some(
                "Voices:\n- Ava (de)\n* Ben en-US\nChloe fr\nOhneSprache\nnur\u{a0}Text mit ungültig!!\n  \n"
                    .to_string(),
            ),
            ..Default::default()
        });
        let mut c = cfg(&fake, &model);
        c.allow_network = true;
        let p = PhononProvider::new(c).with_ram_gate(|_| Ok(64 * 1024));
        let v = p.list_voices(None).await.unwrap();
        let by_id: std::collections::HashMap<_, _> = v
            .iter()
            .map(|v| (v.id.as_str(), v.language.as_str()))
            .collect();
        assert_eq!(by_id["Ava"], "de");
        assert_eq!(by_id["Ben"], "en");
        assert_eq!(by_id["Chloe"], "fr");
        assert_eq!(
            by_id["OhneSprache"], "de",
            "ohne Angabe gilt die konfigurierte Sprache"
        );
        assert!(by_id.contains_key("Lokal") && by_id.contains_key("default"));
        assert!(!by_id.contains_key("Voices:"));
        let de = p.list_voices(Some("de")).await.unwrap();
        assert!(de.iter().all(|v| v.language == "de"));
        assert!(!de.iter().any(|v| v.id == "Ben"));
        assert!(fake.calls().iter().any(|c| c.has("--list-voices")));
    }

    /// `voices/SOURCE.json` wie bei Patrick (Auszug): Dateinamen nach dem Namen der
    /// Stimme, `voice_id` und `filename` aus dem Hugging-Face-Katalog.
    const SOURCE_JSON: &str = r#"{
 "source": "hf://gradium/phonon-voices@0ee17205f254b5abc43b266c8a0f215565c352f7",
 "license": "apache-2.0",
 "voices": [
  {"filename": "voices/2A2fFf8SkDEJ8nzx.safetensors", "name": "Nele", "voice_id": "2A2fFf8SkDEJ8nzx",
   "language": "de", "gender": "female", "age": "Middle-age",
   "description": "Rounded, open tone with gentle lift at the ends of phrases and an easy, flowing rhythm."},
  {"filename": "voices/AAAA.safetensors", "name": "Anton", "voice_id": "AAAA",
   "language": "de", "gender": "Male", "age": "Adult", "description": "Calm."},
  {"filename": "voices/xyz789.safetensors", "name": "Zoe", "voice_id": "xyz789",
   "gender": "female"}
 ]
}"#;

    /// Figurenstimmen des englischen Katalogs: Name mit Leerzeichen, Datei mit `-`, ohne Geschlecht.
    #[tokio::test]
    async fn figurenstimmen_mit_leerzeichen_im_namen_finden_ihre_angaben() {
        let model = model_dir(&["Captain-Silas-Blackwater", "Everly"]);
        std::fs::write(
            model.path().join("voices").join("SOURCE.json"),
            r#"{"source": "hf://gradium/phonon-voices@0ee17205", "license": "apache-2.0", "voices": [
 {"filename": "voices/xPDpsV3F592srkjA.safetensors", "name": "Captain Silas Blackwater",
  "voice_id": "xPDpsV3F592srkjA", "language": "en", "locale": "characters", "gender": null, "age": null,
  "description": "An old British pirate captain from the West Country, around sixty."},
 {"filename": "voices/vjFyp2BcCyTlUmYb.safetensors", "name": "Everly", "voice_id": "vjFyp2BcCyTlUmYb",
  "language": "en", "locale": "en-us", "gender": "female", "age": "Middle-age", "description": "Rounded."}]}"#,
        )
        .unwrap();
        let fake = fake_ptts(FakeOpts::default());
        let v = provider(&fake, &model).list_voices(None).await.unwrap();
        let captain = v
            .iter()
            .find(|x| x.id == "Captain-Silas-Blackwater")
            .unwrap();
        assert!(
            captain
                .label
                .starts_with("Captain Silas Blackwater – An old British pirate"),
            "{}",
            captain.label
        );
        assert_eq!(captain.gender, None, "null im Katalog: kein Geschlecht");
        assert!(captain.license.is_some());
        let everly = v.iter().find(|x| x.id == "Everly").unwrap();
        assert_eq!(everly.gender.as_deref(), Some("female"));
    }

    #[tokio::test]
    async fn stimmen_werden_mit_source_json_angereichert() {
        let model = model_dir(&["Nele", "Anton", "xyz789", "Ohne"]);
        std::fs::write(model.path().join("voices").join("SOURCE.json"), SOURCE_JSON).unwrap();
        let fake = fake_ptts(FakeOpts::default());
        let p = provider(&fake, &model);
        let v = p.list_voices(None).await.unwrap();
        let by_id = |id: &str| {
            v.iter()
                .find(|x| x.id == id)
                .unwrap_or_else(|| panic!("{id}"))
        };

        // Die Kennung bleibt der Dateiname; Beschriftung, Geschlecht und Lizenz kommen aus der Datei.
        let nele = by_id("Nele");
        assert_eq!(nele.gender.as_deref(), Some("female"));
        assert_eq!(
            nele.label,
            "Nele – Rounded, open tone with gentle lift at the ends…"
        );
        let lic = nele.license.as_ref().expect("Lizenz der Stimmen");
        assert_eq!(lic.weights, "apache-2.0");
        assert!(!lic.non_commercial);
        assert!(
            !lic.verified,
            "die Datei ist vom Nutzer, nicht von der App geprüft"
        );
        assert_eq!(
            lic.source_url,
            "https://huggingface.co/gradium/phonon-voices"
        );
        assert!(lic.note.contains("hf://gradium/phonon-voices"));

        // Geschlecht klein geschrieben; kurze Beschreibung bleibt ganz, ohne Schlusspunkt.
        let anton = by_id("Anton");
        assert_eq!(anton.gender.as_deref(), Some("male"));
        assert_eq!(anton.label, "Anton – Calm");
        // Zuordnung auch über die Kennung (Dateiname = voice_id), Name als Beschriftung.
        let zoe = by_id("xyz789");
        assert_eq!(zoe.label, "Zoe");
        assert_eq!(zoe.gender.as_deref(), Some("female"));
        // Ohne Eintrag: schlicht wie bisher.
        let ohne = by_id("Ohne");
        assert_eq!(
            (
                ohne.label.as_str(),
                ohne.gender.clone(),
                ohne.license.clone()
            ),
            ("Ohne", None, None)
        );
        // Die Standardstimme trägt keine Angaben aus der Datei.
        assert!(by_id("default").license.is_none());
        assert!(fake.calls().is_empty(), "offline: kein Prozess");
    }

    /// Eigene Stimmen (Import, Klon) tragen ihre eigene, unverifizierte Lizenz; die Lizenz der
    /// mitgelieferten Stimmen in derselben Datei gilt nicht fuer sie.
    #[tokio::test]
    async fn eigene_stimmen_tragen_eigene_unbekannte_lizenz_nicht_die_des_modells() {
        let model = model_dir(&["Nele", "Eigene-Stimme"]);
        std::fs::write(
            model.path().join("voices").join("SOURCE.json"),
            r#"{"source": "hf://gradium/phonon-voices@0ee17205", "license": "apache-2.0", "voices": [
 {"filename": "voices/AAAA.safetensors", "name": "Nele", "voice_id": "AAAA", "language": "de",
  "locale": "de-de", "gender": "female", "age": "Adult", "description": "Rounded."},
 {"filename": "voices/Eigene-Stimme.safetensors", "name": "Eigene Stimme", "voice_id": "vox_1",
  "language": "de", "locale": "", "gender": null, "age": null,
  "description": "Eigene Stimme (mit Gradium erstellt)", "origin": "gradium-clone",
  "license": "eigene/unbekannt", "verified": false}]}"#,
        )
        .unwrap();
        let fake = fake_ptts(FakeOpts::default());
        let v = provider(&fake, &model).list_voices(None).await.unwrap();
        let own = v.iter().find(|x| x.id == "Eigene-Stimme").unwrap();
        assert_eq!(
            own.label,
            "Eigene Stimme – Eigene Stimme (mit Gradium erstellt)"
        );
        let lic = own.license.as_ref().expect("eigene Lizenz");
        assert_eq!(lic.weights, "eigene/unbekannt");
        assert!(lic.non_commercial && !lic.verified && lic.verified_on.is_none());
        assert!(lic.note.contains("Gradium"), "{}", lic.note);
        // Die mitgelieferte Stimme behaelt die Lizenz der Datei.
        let nele = v.iter().find(|x| x.id == "Nele").unwrap();
        assert_eq!(nele.license.as_ref().unwrap().weights, "apache-2.0");
    }

    /// Wird eine Stimmdatei ersetzt, aendert sich der Cache-Tag; sonst kaeme schon
    /// zwischengespeichertes Vorlesen mit der alten Stimme zurueck.
    #[test]
    fn der_cache_tag_folgt_der_stimmdatei_im_modellordner() {
        let model = model_dir(&["Anna"]);
        let p = PhononProvider::new(PhononConfig {
            model: model.path().to_string_lossy().into_owned(),
            ..Default::default()
        });
        let file = model.path().join("voices").join("Anna.safetensors");
        let first = p.cache_tag(Some("Anna"));
        assert_eq!(
            first,
            p.cache_tag(Some("Anna")),
            "stabil, solange die Datei gleich bleibt"
        );

        // Andere Groesse.
        std::fs::write(&file, b"viel laenger als vorher").unwrap();
        let second = p.cache_tag(Some("Anna"));
        assert_ne!(first, second);

        // Gleiche Groesse, andere Aenderungszeit.
        let f = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
        f.set_modified(std::time::SystemTime::now() + Duration::from_secs(120))
            .unwrap();
        drop(f);
        assert_ne!(second, p.cache_tag(Some("Anna")));

        // Standardstimme, Pfade und unbekannte Stimmen: kein Stempel, kein Zugriff ausserhalb.
        for voice in [
            Some("default"),
            Some("../Anna"),
            Some("Gibt-es-nicht"),
            None,
        ] {
            let tag = p.cache_tag(voice);
            assert!(tag.matches('~').count() == 1, "{tag}");
        }
    }

    #[tokio::test]
    async fn defekte_oder_zu_grosse_source_json_laesst_die_dateiliste_unberuehrt() {
        let fake = fake_ptts(FakeOpts::default());
        for bad in [
            "kein json".to_string(),
            "[1,2,3]".to_string(),
            format!(
                "{{\"voices\": [], \"pad\": \"{}\"}}",
                "x".repeat(600 * 1024)
            ),
        ] {
            let model = model_dir(&["Nele"]);
            std::fs::write(model.path().join("voices").join("SOURCE.json"), bad).unwrap();
            let v = provider(&fake, &model).list_voices(None).await.unwrap();
            let nele = v.iter().find(|x| x.id == "Nele").unwrap();
            assert_eq!(nele.label, "Nele");
            assert!(nele.gender.is_none() && nele.license.is_none());
        }
        // Lizenz unbekannt/nicht freizügig: vorsorglich nichtkommerziell.
        let model = model_dir(&["Nele"]);
        std::fs::write(
            model.path().join("voices").join("SOURCE.json"),
            r#"{"license": "cc-by-nc-4.0", "voices": [{"name": "Nele"}]}"#,
        )
        .unwrap();
        let v = provider(&fake, &model).list_voices(None).await.unwrap();
        let lic = v
            .iter()
            .find(|x| x.id == "Nele")
            .unwrap()
            .license
            .clone()
            .unwrap();
        assert!(lic.non_commercial);
        assert_eq!(lic.source_url, "");
    }

    #[test]
    fn modellordner_zeigt_welche_gewichte_vorliegen() {
        // f32 (Fixture von model_dir), kein q8.
        let f32_only = model_dir(&[]);
        let info = inspect_model_folder(&f32_only.path().display().to_string());
        assert_eq!(
            info,
            ModelFolderInfo {
                is_dir: true,
                has_f32: true,
                has_q8: false
            }
        );
        // Nur q8 — wie der Modellordner phonon-40m-de bei Patrick.
        let q8_only = TempDir::new().unwrap();
        std::fs::write(q8_only.path().join("model.q8.gguf"), b"x").unwrap();
        let info = inspect_model_folder(&q8_only.path().display().to_string());
        assert_eq!(
            info,
            ModelFolderInfo {
                is_dir: true,
                has_f32: false,
                has_q8: true
            }
        );
        // Kein Ordner, hf://-Verweis, leer: keine Gewichte, kein Fehler.
        for none in ["", "   ", "hf://gradium/phonon-40m-de", "Z:/gibt/es/nicht"] {
            assert_eq!(
                inspect_model_folder(none),
                ModelFolderInfo {
                    is_dir: false,
                    has_f32: false,
                    has_q8: false
                },
                "{none:?}"
            );
        }
    }

    #[tokio::test]
    async fn stimmenliste_ohne_programm_ist_nicht_verfuegbar() {
        let model = model_dir(&[]);
        let p = PhononProvider::new(PhononConfig {
            exe_path: "Z:/weg/ptts.exe".into(),
            model: model.path().display().to_string(),
            ..Default::default()
        });
        let e = p.list_voices(None).await.unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Unavailable);
    }

    #[test]
    fn programm_im_pfad_wird_gefunden_und_unbekanntes_nicht() {
        assert!(resolve_exe("gibt-es-sicher-nicht-ptts-xyz").is_none());
        assert!(resolve_exe("Z:/weg/ptts.exe").is_none());
        let fake = fake_ptts(FakeOpts::default());
        assert_eq!(
            resolve_exe(&fake.exe.display().to_string()),
            Some(fake.exe.clone())
        );
    }

    // ---- Weitere Modelle anderer Sprachen (englische Stimmen neben dem deutschen Modell) ----

    fn with_english(fake: &FakePtts, de: &TempDir, en: &TempDir) -> PhononProvider {
        provider(fake, de).with_siblings(vec![PhononModelRef {
            lang: "en".into(),
            model: en.path().display().to_string(),
        }])
    }

    #[tokio::test]
    async fn stimmen_des_englischen_modells_erscheinen_als_en_doppelpunkt_name() {
        let de = model_dir(&["Nele"]);
        let en = model_dir(&["Zoey", "Hugh"]);
        let fake = fake_ptts(FakeOpts::default());
        let p = with_english(&fake, &de, &en);
        assert_eq!(p.descriptor().caps.languages, vec!["de", "en"]);
        let v = p.list_voices(None).await.unwrap();
        let ids: Vec<(&str, &str)> = v
            .iter()
            .map(|v| (v.id.as_str(), v.language.as_str()))
            .collect();
        assert!(ids.contains(&("Nele", "de")), "{ids:?}");
        assert!(ids.contains(&("en:default", "en")), "{ids:?}");
        assert!(ids.contains(&("en:Zoey", "en")), "{ids:?}");
        assert!(ids.contains(&("en:Hugh", "en")), "{ids:?}");
        // Sprachfilter: nur die englischen.
        let only_en = p.list_voices(Some("en")).await.unwrap();
        assert!(!only_en.is_empty());
        assert!(
            only_en.iter().all(|v| v.id.starts_with("en:")),
            "{only_en:?}"
        );
    }

    #[tokio::test]
    async fn eine_englische_stimme_liest_mit_dem_englischen_modell_und_lang_en() {
        let de = model_dir(&["Nele"]);
        let en = model_dir(&["Zoey"]);
        let fake = fake_ptts(FakeOpts::default());
        let p = with_english(&fake, &de, &en);
        for voice in ["en:Zoey", "en:default", "Nele"] {
            let mut o = opts();
            o.voice = Some(voice.into());
            p.synthesize("Hello.", &o, &CancelToken::detached())
                .await
                .unwrap();
        }
        let calls = fake.synth_calls();
        let en_dir = en.path().display().to_string();
        let de_dir = de.path().display().to_string();
        assert_eq!(calls[0].value_of("--model"), Some(en_dir.as_str()));
        assert_eq!(calls[0].value_of("--lang"), Some("en"));
        assert_eq!(calls[0].value_of("--voice"), Some("Zoey"));
        assert_eq!(calls[1].value_of("--model"), Some(en_dir.as_str()));
        assert!(!calls[1].has("--voice"), "en:default = Standardstimme");
        assert_eq!(calls[2].value_of("--model"), Some(de_dir.as_str()));
        assert_eq!(calls[2].value_of("--lang"), Some("de"));
        assert_eq!(calls[2].value_of("--voice"), Some("Nele"));
    }

    #[tokio::test]
    async fn ohne_weiteres_modell_bleibt_en_doppelpunkt_ein_gewoehnlicher_name() {
        let de = model_dir(&["Nele"]);
        let fake = fake_ptts(FakeOpts::default());
        let p = provider(&fake, &de);
        let mut o = opts();
        o.voice = Some("en:Zoey".into());
        // Keine englische Installation: ohne Netzfreigabe nicht ladbar, kein Prozessstart.
        let err = p
            .synthesize("Hello.", &o, &CancelToken::detached())
            .await
            .unwrap_err();
        assert!(err.message.contains("en:Zoey"), "{}", err.message);
        assert!(fake.synth_calls().is_empty());
    }

    #[tokio::test]
    async fn eine_konfigurierte_englische_stimme_steht_einmal_mit_sprache_en_in_der_liste() {
        let de = model_dir(&["Nele"]);
        let en = model_dir(&["Zoey"]);
        let fake = fake_ptts(FakeOpts::default());
        let mut c = cfg(&fake, &de);
        c.voice = Some("en:Zoey".into());
        let p = PhononProvider::new(c)
            .with_ram_gate(|_| Ok(64 * 1024))
            .with_siblings(vec![PhononModelRef {
                lang: "en".into(),
                model: en.path().display().to_string(),
            }]);
        let all = p.list_voices(None).await.unwrap();
        let zoey: Vec<_> = all.iter().filter(|v| v.id == "en:Zoey").collect();
        assert_eq!(zoey.len(), 1, "{all:?}");
        assert_eq!(zoey[0].language, "en");
        let only_en = p.list_voices(Some("en")).await.unwrap();
        assert!(only_en.iter().any(|v| v.id == "en:Zoey"), "{only_en:?}");
        assert!(p.health_check().await.available);
    }

    #[test]
    fn weitere_modelle_der_eigenen_oder_unbekannten_sprache_fallen_weg() {
        let de = model_dir(&[]);
        let fake = fake_ptts(FakeOpts::default());
        let p = provider(&fake, &de).with_siblings(vec![
            PhononModelRef {
                lang: "de".into(),
                model: "x".into(),
            },
            PhononModelRef {
                lang: "xx".into(),
                model: "y".into(),
            },
            PhononModelRef {
                lang: "EN".into(),
                model: "en1".into(),
            },
            PhononModelRef {
                lang: "en".into(),
                model: "en2".into(),
            },
            PhononModelRef {
                lang: "fr".into(),
                model: " ".into(),
            },
        ]);
        assert_eq!(
            p.siblings,
            vec![PhononModelRef {
                lang: "en".into(),
                model: "en1".into()
            }]
        );
    }

    #[test]
    fn der_cache_tag_einer_englischen_stimme_ist_der_des_englischen_modells() {
        let de = model_dir(&["Zoey"]);
        let en = model_dir(&["Zoey"]);
        let fake = fake_ptts(FakeOpts::default());
        let p = with_english(&fake, &de, &en);
        let alone = PhononProvider::new(PhononConfig {
            lang: "en".into(),
            ..cfg(&fake, &en)
        });
        assert_eq!(p.cache_tag(Some("en:Zoey")), alone.cache_tag(Some("Zoey")));
        assert_ne!(p.cache_tag(Some("en:Zoey")), p.cache_tag(Some("Zoey")));
    }
}
