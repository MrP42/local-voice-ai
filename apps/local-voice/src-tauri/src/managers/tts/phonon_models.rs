//! Phonon auf der Modellseite: Programm (`ptts`), Modelle und Stimmen installieren und entfernen
//! (Vorbild: `models.rs` fuer Piper; derselbe Download-Weg, dieselben Ereignisse).
//!
//! Ablage unter `<app_data>/tts/phonon/`:
//!
//! * Programm: `<plattform>/ptts(.exe)` samt Lizenzdateien, aus dem Release-Archiv von GitHub.
//! * Modell: `models/<id>/` mit genau den Dateien des Katalogeintrags (Commit fest).
//! * Stimme: `models/<modell>/voices/<Name>.safetensors` plus `voices/SOURCE.json`
//!   (`{"source", "license", "voices": [Zeilen aus voices.jsonl]}`).
//!
//! ## Stimmen gehoeren zum Modell (seit 27.10.2026)
//!
//! Die Stimmen der Sprache eines Modells (Katalog: `purpose: phonon-voice`, `parent` = Modell) sind
//! KEINE eigenen Zeilen der Oberflaeche mehr, sondern Teil ihres Modells: `install_model` laedt sie mit
//! (Groessenanzeige = Modell + Stimmen), `delete` entfernt sie mit dem Modellordner. Ein schon
//! installiertes Modell ohne (alle) Stimmen wird beim naechsten Installieren ergaenzt, ohne das Modell
//! neu zu laden (`install_missing_voices`); die Oberflaeche bietet das als „Stimmen nachladen“ an.
//! Einzeln installierte Stimmen (aeltere Staende) liegen am selben Zielpfad und bleiben gueltig.
//!
//! ## Fehlerfaelle und ihre Absicherung
//!
//! | Fall | Absicherung |
//! |---|---|
//! | Zwei Installationen derselben Kennung (Doppelklick) | `try_claim` ist atomar: die zweite wird abgewiesen; Test `eine_kennung_laesst_sich_nur_einmal_belegen` |
//! | Abbruch mitten im Vorgang | jede Datei kommt als `.partial` (fortsetzbar), wird geprueft, dann umbenannt; ein Modell entsteht erst in `<id>.installing/` und wird als Ganzes umbenannt, die Marke `.lva-install.json` ist das Letzte; Tests `abbruch_…`, `…atomar…` |
//! | Absturz/Abbruch waehrend des Entpackens | Entpackt wird nach `<ziel>.extracting`; ersetzt wird erst nach der Pruefung, mit Sicherung und Rueckweg; Tests `unvollstaendiges_archiv_…`, `ersetzen_…` |
//! | Voller Datentraeger / nicht beschreibbar | Schreibfehler kommen als Meldung zurueck (Code 112/28: eigener Text), nichts gilt als installiert (Marke fehlt); Tests `ein_nicht_beschreibbares_ziel_…` |
//! | Netzfehler, Stillstand | Wiederaufnahme per Range; fertige Dateien eines Modells werden nicht neu geladen; Test `abbruch_…` |
//! | Beschaedigte oder untergeschobene Datei | SHA-256 (bzw. Git-Blob-Kennung) je Datei; bei Abweichung wird die Datei geloescht und nichts installiert; Tests `sha_…`, `git_blob_…` |
//! | Archiv mit `..`, absoluten Pfaden, Symlinks, Riesendatei | nur vier benannte Dateien werden entpackt, unzulaessige Pfade lehnen das Archiv ab, Groessenobergrenze; Tests `zip_…`, `tar_…` |
//! | Gesperrtes Modell (gated) | Token nur aus `HF_TOKEN`/Token-Datei, nur an `https://huggingface.co`, nie an ein CDN, nie in Meldung oder Log; 401/403 und fehlendes Token nennen den Weg; Tests `gesperrtes_…`, `download::tests::…` |
//! | Katalog-IDs als Pfadbaustein | nur `[A-Za-z0-9._-]`, kein fuehrender Punkt; Test `ids_und_dateinamen_…` |

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use hf_hub::api::tokio::CancellationToken;
use serde::{Deserialize, Serialize};

use super::models::{TtsDownloadInfo, TtsDownloadKind};
use super::providers::contract::LicenseInfo;
use super::providers::policy::{InstalledPaths, InstalledPhonon};
use crate::catalog::{self, PhononVoiceMeta, Purpose, TtsCatalogEntry};
use crate::managers::model::download::{
    DownloadAuth, HttpDownloadEvent, HttpDownloadOutcome, HttpStatusError, Origin, Secret,
};
use crate::managers::model::{DownloadProgress, ModelManager};

/// Kennung der (plattformabhaengig aufgeloesten) Programmzeile in der Oberflaeche.
pub const RUNTIME_ID: &str = "phonon-runtime";
/// Marke im Ordner einer ueber die Modellseite eingerichteten Installation. Geschrieben wird sie
/// zuletzt und nur nach bestandener Pruefung: ohne sie gilt nichts als installiert.
pub(crate) const MARKER_FILE: &str = ".lva-install.json";
const SOURCE_FILE: &str = "SOURCE.json";
const ENGINE: &str = "phonon";
/// Obergrenze fuer das Programm im Archiv und fuer jede andere entpackte Datei.
const MAX_EXE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SMALL_BYTES: u64 = 4 * 1024 * 1024;
/// Modelle in der Reihenfolge, in der der Adapter sie waehlt: das deutsche, sonst das englische.
const MODEL_PREFERENCE: [&str; 2] = ["phonon-40m-de", "phonon-31m-en"];

// ---- Plattform, Pfade --------------------------------------------------------------

/// Plattformkennung des Katalogs (`phonon-runtime-<plattform>`); `None` ohne Programm fuer dieses System.
pub(crate) fn current_platform() -> Option<&'static str> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-aarch64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("macos-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("linux-aarch64")
    } else {
        None
    }
}

pub(crate) fn exe_name(platform: &str) -> &'static str {
    if platform.starts_with("windows") {
        "ptts.exe"
    } else {
        "ptts"
    }
}

/// Ein Katalog-Name als Pfadbaustein: Buchstaben, Ziffern, `.`, `_`, `-`; nie leer, nie mit Punkt am Anfang.
pub(crate) fn is_safe_component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `<app_data>/tts/phonon`.
#[derive(Debug, Clone)]
pub struct PhononDirs {
    root: PathBuf,
}

impl PhononDirs {
    pub fn new(app_data: &Path) -> Self {
        Self {
            root: app_data.join("tts").join("phonon"),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn runtime_dir(&self, platform: &str) -> PathBuf {
        self.root.join(platform)
    }

    pub(crate) fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    pub(crate) fn model_dir(&self, id: &str) -> PathBuf {
        self.models_dir().join(id)
    }

    fn staging_dir(&self, id: &str) -> PathBuf {
        self.models_dir().join(format!("{id}.installing"))
    }

    fn voices_dir(&self, model_id: &str) -> PathBuf {
        self.model_dir(model_id).join("voices")
    }
}

/// Unterordner fuer die Stimmen in einem Modellordner (auch im Zwischenordner).
const VOICES_SUBDIR: &str = "voices";

fn file_has_size(path: &Path, size: u64) -> bool {
    fs::metadata(path)
        .map(|m| m.is_file() && m.len() == size)
        .unwrap_or(false)
}

fn ensure_safe_entry(entry: &TtsCatalogEntry) -> Result<(), String> {
    let ok = is_safe_component(&entry.id)
        && entry.files.iter().all(|f| is_safe_component(&f.filename))
        && entry.parent.as_deref().map_or(true, is_safe_component);
    if ok {
        Ok(())
    } else {
        Err(format!(
            "Katalogeintrag {} hat einen unzulässigen Namen.",
            entry.id
        ))
    }
}

// ---- Katalog-Zugriff ------------------------------------------------------------------

fn runtime_entry(platform: &str) -> Option<TtsCatalogEntry> {
    let id = format!("phonon-runtime-{platform}");
    catalog::tts_entries(Purpose::PhononRuntime)
        .into_iter()
        .find(|e| e.id == id)
}

fn model_entries() -> Vec<TtsCatalogEntry> {
    catalog::tts_entries(Purpose::PhononModel)
}

fn voice_entries() -> Vec<TtsCatalogEntry> {
    catalog::tts_entries(Purpose::PhononVoice)
}

fn model_entry(id: &str) -> Option<TtsCatalogEntry> {
    model_entries().into_iter().find(|e| e.id == id)
}

/// Die Stimmen, die mit dem Modell `model_id` geliefert werden (Katalogreihenfolge).
pub(crate) fn bundled_voices(model_id: &str) -> Vec<TtsCatalogEntry> {
    voice_entries()
        .into_iter()
        .filter(|e| e.parent.as_deref() == Some(model_id))
        .collect()
}

/// Gehoert die Kennung zu Phonon (Programmzeile, Modell oder Stimme)?
pub(crate) fn is_phonon_id(id: &str) -> bool {
    id == RUNTIME_ID || model_entry(id).is_some() || voice_entries().iter().any(|e| e.id == id)
}

// ---- Marke ------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Marker {
    catalog_id: String,
    revision: Option<String>,
    source: Option<String>,
    installed_at_unix: u64,
}

pub(crate) fn write_marker(dir: &Path, entry: &TtsCatalogEntry) -> Result<()> {
    let marker = Marker {
        catalog_id: entry.id.clone(),
        revision: entry.revision.clone(),
        source: entry.source.clone(),
        installed_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };
    let tmp = dir.join(format!("{MARKER_FILE}.tmp"));
    fs::write(&tmp, serde_json::to_vec_pretty(&marker)?)?;
    fs::rename(&tmp, dir.join(MARKER_FILE))?;
    Ok(())
}

fn read_marker(dir: &Path) -> Option<Marker> {
    let file = fs::File::open(dir.join(MARKER_FILE)).ok()?;
    let mut buf = Vec::new();
    file.take(64 * 1024).read_to_end(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn marker_matches(dir: &Path, entry: &TtsCatalogEntry) -> bool {
    read_marker(dir).is_some_and(|m| m.catalog_id == entry.id && m.revision == entry.revision)
}

// ---- Zustand auf der Platte ---------------------------------------------------------------

fn runtime_installed(dirs: &PhononDirs, platform: &str, entry: &TtsCatalogEntry) -> bool {
    let dir = dirs.runtime_dir(platform);
    dir.join(exe_name(platform)).is_file() && marker_matches(&dir, entry)
}

/// Ein Modell gilt nur als installiert, wenn die Marke zum Katalogstand passt UND jede Datei in
/// der verlangten Groesse da ist (ein haengengebliebener Ordner liest sich nie als installiert).
fn model_complete(dirs: &PhononDirs, entry: &TtsCatalogEntry) -> bool {
    let dir = dirs.model_dir(&entry.id);
    marker_matches(&dir, entry)
        && entry
            .files
            .iter()
            .all(|f| file_has_size(&dir.join(&f.filename), f.size_bytes))
}

fn voice_file(dirs: &PhononDirs, entry: &TtsCatalogEntry) -> Option<PathBuf> {
    let parent = entry.parent.as_deref()?;
    let file = entry.files.first()?;
    Some(dirs.voices_dir(parent).join(&file.filename))
}

fn voice_installed(dirs: &PhononDirs, entry: &TtsCatalogEntry) -> bool {
    match (voice_file(dirs, entry), entry.files.first()) {
        (Some(path), Some(f)) => file_has_size(&path, f.size_bytes),
        _ => false,
    }
}

/// Steht die Stimme in `voices/SOURCE.json` (Name, Geschlecht, Beschreibung fuer die Anzeige)?
fn voice_listed(source: Option<&VoiceSource>, entry: &TtsCatalogEntry) -> bool {
    match (source, entry.voice.as_ref()) {
        (Some(doc), Some(meta)) => doc.voices.iter().any(|v| v.name == meta.name),
        _ => false,
    }
}

/// Wie viele der mitgelieferten Stimmen sind vollstaendig da (Datei in richtiger Groesse UND
/// Eintrag in `SOURCE.json`)? Fehlt eines von beiden, gilt die Stimme als fehlend und wird beim
/// Nachladen ergaenzt (Dateien, die schon stimmen, werden nicht neu geladen).
fn voices_present(dirs: &PhononDirs, model_id: &str, voices: &[TtsCatalogEntry]) -> usize {
    let source = read_voice_source(&dirs.voices_dir(model_id));
    voices
        .iter()
        .filter(|v| voice_installed(dirs, v) && voice_listed(source.as_ref(), v))
        .count()
}

fn files_bytes(entry: &TtsCatalogEntry) -> u64 {
    entry.files.iter().map(|f| f.size_bytes).sum()
}

// ---- Liste fuer die Oberflaeche -------------------------------------------------------------

fn base_info(
    entry: &TtsCatalogEntry,
    id: &str,
    kind: TtsDownloadKind,
    requires: Option<String>,
) -> TtsDownloadInfo {
    let size_bytes: u64 = entry.files.iter().map(|f| f.size_bytes).sum();
    TtsDownloadInfo {
        id: id.to_string(),
        kind,
        name: entry.name.clone(),
        description: entry.description.clone(),
        language: entry.languages.first().cloned(),
        size_mb: size_bytes / (1024 * 1024),
        is_downloaded: false,
        is_downloading: false,
        is_usable: false,
        unsupported_reason: entry.unsupported_reason.clone(),
        license: entry.license.clone(),
        license_url: entry.license_url.clone(),
        license_non_commercial: entry.license_non_commercial,
        engine: ENGINE.to_string(),
        size_bytes,
        gated: entry.gated,
        requires,
        source: entry.source.clone(),
        sha256: if entry.files.len() == 1 {
            entry.files[0].sha256.clone()
        } else {
            None
        },
        license_note: entry.license_note.clone(),
        bundled_voices: 0,
        bundled_voices_present: 0,
    }
}

/// Programmzeile, dann Modelle. Die Stimmen sind Teil ihres Modells (Groesse, Zaehler), keine Zeilen. `is_downloading` fragt den laufenden Zustand
/// (er braucht den Manager; hier wird er hereingereicht, damit die Liste ohne App testbar ist).
pub(crate) fn build_downloads(
    dirs: &PhononDirs,
    platform: Option<&str>,
    is_downloading: impl Fn(&str) -> bool,
) -> Vec<TtsDownloadInfo> {
    let mut out = Vec::new();
    let runtime = platform.and_then(|p| runtime_entry(p).map(|e| (p, e)));
    let runtime_ready = runtime
        .as_ref()
        .is_some_and(|(p, e)| runtime_installed(dirs, p, e));
    if let Some((_, entry)) = &runtime {
        let mut info = base_info(entry, RUNTIME_ID, TtsDownloadKind::Runtime, None);
        info.is_downloaded = runtime_ready;
        info.is_usable = runtime_ready;
        info.is_downloading = is_downloading(RUNTIME_ID);
        out.push(info);
    }
    let runtime_id = runtime.as_ref().map(|_| RUNTIME_ID.to_string());

    for entry in model_entries() {
        let complete = model_complete(dirs, &entry);
        let voices = bundled_voices(&entry.id);
        let mut info = base_info(
            &entry,
            &entry.id,
            TtsDownloadKind::Model,
            runtime_id.clone(),
        );
        // Anzeige: Modell plus Stimmen, so gross wie die ganze Installation.
        info.size_bytes += voices.iter().map(files_bytes).sum::<u64>();
        info.size_mb = info.size_bytes / (1024 * 1024);
        info.bundled_voices = voices.len() as u32;
        info.bundled_voices_present = voices_present(dirs, &entry.id, &voices) as u32;
        info.is_downloaded = complete;
        info.is_usable = complete && runtime_ready;
        info.is_downloading = is_downloading(&entry.id);
        out.push(info);
    }
    out
}

// ---- Aufloesung fuer die Anbieter-Einstellungen ("installiert") --------------------------------

/// Pfade der ueber die Modellseite installierten Teile; leer, wenn nichts (vollstaendig) installiert ist.
/// Das Programm nur mit Marke, das Modell: das deutsche, sonst das englische.
pub(crate) fn installed_paths(app_data: &Path) -> InstalledPaths {
    let dirs = PhononDirs::new(app_data);
    let exe_path = current_platform().and_then(|platform| {
        let entry = runtime_entry(platform)?;
        runtime_installed(&dirs, platform, &entry).then(|| {
            dirs.runtime_dir(platform)
                .join(exe_name(platform))
                .to_string_lossy()
                .into_owned()
        })
    });
    let models = model_entries();
    let model = MODEL_PREFERENCE.iter().find_map(|id| {
        let entry = models.iter().find(|e| e.id == *id)?;
        model_complete(&dirs, entry).then(|| dirs.model_dir(id).to_string_lossy().into_owned())
    });
    InstalledPaths {
        phonon: InstalledPhonon { exe_path, model },
    }
}

/// Kennung und Ordner aller vollstaendig ueber die Modellseite installierten Katalog-Modelle
/// (Katalogreihenfolge). Dorthin kopiert `phonon_voices` die eigenen Stimmen.
pub(crate) fn installed_model_dirs(dirs: &PhononDirs) -> Vec<(String, PathBuf)> {
    model_entries()
        .into_iter()
        .filter(|entry| model_complete(dirs, entry))
        .map(|entry| (entry.id.clone(), dirs.model_dir(&entry.id)))
        .collect()
}

/// Lizenz eines ueber die Modellseite installierten Katalog-Modells (aus dem Katalog, mit Quelle und
/// Pruefdatum). `None` fuer alles andere: `hf://`-Verweise, fremde Ordner und Ordner ohne passende
/// Marke (ein Ordner gleichen Namens mit fremden Gewichten bekommt so keine Lizenz zugesprochen).
pub(crate) fn installed_model_license(model: &str) -> Option<LicenseInfo> {
    if model.contains("://") {
        return None;
    }
    let path = Path::new(model.trim());
    let id = path.file_name()?.to_str()?;
    let models = path.parent()?;
    let phonon = models.parent()?;
    let tts = phonon.parent()?;
    let named = |p: &Path, n: &str| p.file_name().and_then(|s| s.to_str()) == Some(n);
    if !(named(models, "models") && named(phonon, "phonon") && named(tts, "tts")) {
        return None;
    }
    let entry = model_entry(id)?;
    if !marker_matches(path, &entry) {
        return None;
    }
    let rev = entry
        .revision
        .as_deref()
        .map(|r| r.chars().take(8).collect::<String>())
        .unwrap_or_default();
    let license = entry.license.clone()?;
    Some(LicenseInfo {
        code: "MIT OR Apache-2.0".to_string(),
        weights: format!("{license} ({}, über die Modellseite installiert)", entry.name),
        non_commercial: entry.license_non_commercial,
        verified: true,
        source_url: entry.license_url.clone()?,
        verified_on: entry.license_verified_on.clone(),
        note: format!(
            "{} Installiert über die Modellseite aus {} (Commit {rev}); Größe und Prüfsumme jeder Datei wurden beim Laden geprüft.",
            entry.license_note.clone().unwrap_or_default(),
            entry.source.clone().unwrap_or_default()
        ),
    })
}

// ---- Token (gated Modelle) ------------------------------------------------------------------

/// Hugging-Face-Token aus `HF_TOKEN`, sonst aus der Token-Datei (`$HF_HOME/token`, sonst
/// `$XDG_CACHE_HOME/huggingface/token`, sonst `~/.cache/huggingface/token`). `env` liefert die
/// Umgebung (Produktion: [`process_env`]). Der Wert wird nie ausgegeben.
pub(crate) fn hf_token(env: &dyn Fn(&str) -> Option<String>) -> Option<Secret> {
    if let Some(token) = env("HF_TOKEN").as_deref().and_then(Secret::new) {
        return Some(token);
    }
    let non_empty = |k: &str| {
        env(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let path = if let Some(home) = non_empty("HF_HOME") {
        PathBuf::from(home).join("token")
    } else if let Some(cache) = non_empty("XDG_CACHE_HOME") {
        PathBuf::from(cache).join("huggingface").join("token")
    } else {
        let home = non_empty("HOME").or_else(|| non_empty("USERPROFILE"))?;
        PathBuf::from(home)
            .join(".cache")
            .join("huggingface")
            .join("token")
    };
    let mut buf = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(4096)
        .read_to_end(&mut buf)
        .ok()?;
    let text = String::from_utf8(buf).ok()?;
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .and_then(Secret::new)
}

pub(crate) fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn hf_repo(entry: &TtsCatalogEntry) -> String {
    entry
        .source
        .as_deref()
        .and_then(|s| s.strip_prefix("hf://"))
        .map(|s| s.split('@').next().unwrap_or(s).to_string())
        .unwrap_or_else(|| entry.name.clone())
}

fn no_token_message(entry: &TtsCatalogEntry) -> String {
    format!(
        "{} ist gesperrt, und es wurde kein Hugging-Face-Token gefunden. Bedingungen auf huggingface.co/{} annehmen und anmelden (hf auth login) oder die Umgebungsvariable HF_TOKEN setzen.",
        entry.name,
        hf_repo(entry)
    )
}

fn denied_message(entry: &TtsCatalogEntry) -> String {
    format!(
        "{} ist gesperrt: der Server verweigert den Zugriff. Bedingungen auf huggingface.co/{} annehmen und anmelden (hf auth login).",
        entry.name,
        hf_repo(entry)
    )
}

/// Meldung zu einem Download-Fehler. Nennt weder Token noch signierte Adressen.
fn describe_error(entry: &TtsCatalogEntry, err: &anyhow::Error) -> String {
    if let Some(HttpStatusError(status)) = err.downcast_ref::<HttpStatusError>() {
        let code = status.as_u16();
        return match code {
            401 | 403 if entry.gated => denied_message(entry),
            401 | 403 => format!(
                "Der Server verweigert den Zugriff auf {} (HTTP {code}).",
                entry.name
            ),
            404 => format!(
                "{} wurde an der Quelle nicht gefunden (HTTP 404); die Quelle hat sich vermutlich verändert.",
                entry.name
            ),
            _ => format!(
                "Download von {} fehlgeschlagen: der Server antwortete mit HTTP {code}.",
                entry.name
            ),
        };
    }
    if let Some(io) = err.downcast_ref::<std::io::Error>() {
        return io_message(&entry.name, io);
    }
    format!(
        "Download von {} fehlgeschlagen: {}",
        entry.name,
        scrub_urls(&err.to_string())
    )
}

/// Ersetzt Webadressen im Text: eine signierte CDN-Adresse waere selbst ein Zugang auf Zeit und
/// gehoert in keine Meldung (Fehler der Netzbibliothek nennen sie gern).
fn scrub_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = ["https://", "http://"]
        .iter()
        .filter_map(|prefix| rest.find(prefix))
        .min()
    {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '>'))
            .unwrap_or(tail.len());
        out.push_str("<Adresse>");
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn io_message(what: &str, e: &std::io::Error) -> String {
    // 112 = ERROR_DISK_FULL (Windows), 28 = ENOSPC (Unix).
    if matches!(e.raw_os_error(), Some(112) | Some(28)) {
        format!(
            "Für {what} ist nicht genug Speicherplatz frei. Platz schaffen und erneut versuchen."
        )
    } else {
        format!("{what} lässt sich nicht schreiben: {e}")
    }
}

// ---- SHA-1 fuer Git-Blob-Kennungen ----------------------------------------------------------

/// SHA-1 (RFC 3174), nur fuer Git-Blob-Kennungen der kleinen Dateien ohne LFS, bei denen die
/// Hugging-Face-API kein SHA-256 nennt. Keine Sicherheitsfunktion allein: sie ergaenzt den festen
/// Commit; die grossen Dateien tragen SHA-256.
struct Sha1 {
    h: [u32; 5],
    buf: [u8; 64],
    buf_len: usize,
    total: u64,
}

impl Sha1 {
    fn new() -> Self {
        Self {
            h: [
                0x6745_2301,
                0xEFCD_AB89,
                0x98BA_DCFE,
                0x1032_5476,
                0xC3D2_E1F0,
            ],
            buf: [0; 64],
            buf_len: 0,
            total: 0,
        }
    }

    fn block(h: &mut [u32; 5], block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for (i, chunk) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = *h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total += data.len() as u64;
        if self.buf_len > 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                Self::block(&mut self.h, &block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            Self::block(&mut self.h, &block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn finish(mut self) -> String {
        let bit_len = self.total.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        while (self.buf_len + pad.len()) % 64 != 56 {
            pad.push(0);
        }
        pad.extend_from_slice(&bit_len.to_be_bytes());
        let total = self.total;
        self.update(&pad);
        self.total = total;
        self.h.iter().map(|w| format!("{w:08x}")).collect()
    }
}

/// Git-Blob-Kennung einer Datei: SHA-1 ueber `blob <laenge>\0<inhalt>`.
fn git_blob_sha1(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut sha = Sha1::new();
    sha.update(format!("blob {len}\0").as_bytes());
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
    }
    Ok(sha.finish())
}

/// Prueft die Git-Blob-Kennung; bei Abweichung wird die Datei geloescht (naechster Versuch beginnt sauber).
fn verify_git_blob(path: &Path, expected: &str, name: &str) -> Result<(), String> {
    match git_blob_sha1(path) {
        Ok(actual) if actual.eq_ignore_ascii_case(expected) => Ok(()),
        Ok(_) => {
            let _ = fs::remove_file(path);
            Err(format!(
                "Die Datei {name} ist beschädigt (Prüfsumme weicht ab). Bitte erneut versuchen."
            ))
        }
        Err(e) => {
            let _ = fs::remove_file(path);
            Err(format!("Die Datei {name} lässt sich nicht prüfen: {e}"))
        }
    }
}

// ---- Entpacken des Programms ----------------------------------------------------------------

/// Zerlegt einen Archivpfad in Bausteine. `Err` bei allem, was aus dem Zielordner herausfuehren koennte:
/// absolute Pfade, Laufwerks- oder Stream-Angaben (`:`), `..`.
fn entry_parts(name: &str) -> Result<Vec<String>> {
    let norm = name.replace('\\', "/");
    if norm.starts_with('/') || norm.contains(':') {
        return Err(anyhow!("Das Archiv enthält einen unzulässigen Pfad."));
    }
    let mut parts = Vec::new();
    for p in norm.split('/') {
        match p {
            "" | "." => {}
            ".." => return Err(anyhow!("Das Archiv enthält einen unzulässigen Pfad.")),
            other => parts.push(other.to_string()),
        }
    }
    Ok(parts)
}

/// Dateien, die aus dem Archiv uebernommen werden: das Programm und seine Lizenzen.
fn is_wanted_file(name: &str, exe: &str) -> bool {
    name == exe || matches!(name, "LICENSE-MIT" | "LICENSE-APACHE" | "README.md")
}

/// Kopiert hoechstens `limit` Bytes; mehr ist ein Fehler (Zip-Bombe).
fn copy_limited(reader: &mut dyn Read, out: &mut fs::File, limit: u64) -> Result<u64> {
    let mut limited = reader.take(limit + 1);
    let n = std::io::copy(&mut limited, out)?;
    if n > limit {
        return Err(anyhow!("Eine Datei im Archiv ist unplausibel groß."));
    }
    Ok(n)
}

fn extract_zip(archive: &Path, out_dir: &Path, exe: &str) -> Result<()> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut seen: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let parts = entry_parts(entry.name())?;
        if entry.is_dir() || parts.is_empty() || parts.len() > 2 {
            continue;
        }
        let name = parts[parts.len() - 1].clone();
        if !is_wanted_file(&name, exe) {
            continue;
        }
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(anyhow!(
                "Das Archiv enthält eine Verknüpfung statt einer Datei."
            ));
        }
        if seen.contains(&name) {
            return Err(anyhow!("Das Archiv enthält {name} doppelt."));
        }
        let limit = if name == exe {
            MAX_EXE_BYTES
        } else {
            MAX_SMALL_BYTES
        };
        let mut out = fs::File::create(out_dir.join(&name))?;
        copy_limited(&mut entry, &mut out, limit)?;
        seen.push(name);
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, out_dir: &Path, exe: &str) -> Result<()> {
    let file = fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut seen: Vec<String> = Vec::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        let raw = entry.path()?.to_string_lossy().into_owned();
        let parts = entry_parts(&raw)?;
        let kind = entry.header().entry_type();
        if parts.is_empty() || parts.len() > 2 || kind.is_dir() {
            continue;
        }
        let name = parts[parts.len() - 1].clone();
        if !is_wanted_file(&name, exe) {
            continue;
        }
        if !kind.is_file() {
            return Err(anyhow!(
                "Das Archiv enthält eine Verknüpfung statt einer Datei."
            ));
        }
        if seen.contains(&name) {
            return Err(anyhow!("Das Archiv enthält {name} doppelt."));
        }
        let limit = if name == exe {
            MAX_EXE_BYTES
        } else {
            MAX_SMALL_BYTES
        };
        let mut out = fs::File::create(out_dir.join(&name))?;
        copy_limited(&mut entry, &mut out, limit)?;
        seen.push(name);
    }
    Ok(())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("phonon");
    path.with_file_name(format!("{name}{suffix}"))
}

/// Ersetzt `dest` durch `new_dir`: die alte Fassung wird zuerst beiseite gestellt und kommt bei
/// einem Fehler zurueck. Nie bleibt `dest` halb ersetzt.
fn replace_dir(new_dir: &Path, dest: &Path) -> Result<()> {
    let backup = sibling(dest, ".old");
    if backup.exists() {
        fs::remove_dir_all(&backup)?;
    }
    let had_old = dest.exists();
    if had_old {
        fs::rename(dest, &backup)?;
    }
    if let Err(e) = fs::rename(new_dir, dest) {
        if had_old {
            let _ = fs::rename(&backup, dest);
        }
        return Err(e.into());
    }
    if had_old {
        let _ = fs::remove_dir_all(&backup);
    }
    Ok(())
}

/// Entpackt das Programm-Archiv nach `dest_dir`: nur `ptts(.exe)` und die Lizenzdateien, keine
/// Pfade ausserhalb, keine Verknuepfungen. Erst pruefen, dann ersetzen: ein unvollstaendiges Archiv
/// laesst eine vorhandene Installation unberuehrt.
pub(crate) fn extract_ptts_archive(
    archive: &Path,
    dest_dir: &Path,
    is_zip: bool,
    platform: &str,
    entry: &TtsCatalogEntry,
) -> Result<()> {
    let exe = exe_name(platform);
    let temp = sibling(dest_dir, ".extracting");
    if temp.exists() {
        fs::remove_dir_all(&temp)?;
    }
    fs::create_dir_all(&temp)?;
    let result = (|| -> Result<()> {
        if is_zip {
            extract_zip(archive, &temp, exe)?;
        } else {
            extract_tar_gz(archive, &temp, exe)?;
        }
        let exe_path = temp.join(exe);
        if !fs::metadata(&exe_path)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
        {
            return Err(anyhow!("Das Archiv enthält {exe} nicht."));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe_path, fs::Permissions::from_mode(0o755))?;
        }
        write_marker(&temp, entry)?;
        replace_dir(&temp, dest_dir)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temp);
    }
    result
}

// ---- Stimmenliste SOURCE.json ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VoiceSource {
    source: String,
    license: String,
    voices: Vec<PhononVoiceMeta>,
}

fn read_voice_source(voices_dir: &Path) -> Option<VoiceSource> {
    let file = fs::File::open(voices_dir.join(SOURCE_FILE)).ok()?;
    let mut buf = Vec::new();
    file.take(1024 * 1024).read_to_end(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn write_voice_source(voices_dir: &Path, doc: &VoiceSource) -> Result<()> {
    let tmp = voices_dir.join(format!("{SOURCE_FILE}.tmp"));
    fs::write(&tmp, serde_json::to_vec_pretty(doc)?)?;
    fs::rename(&tmp, voices_dir.join(SOURCE_FILE))?;
    Ok(())
}

/// Traegt die Zeilen der Stimmen in `base` ein (ersetzt gleichnamige; Reihenfolge wie im Katalog).
fn merge_voice_rows(base: Option<VoiceSource>, entries: &[TtsCatalogEntry]) -> VoiceSource {
    let mut doc = base.unwrap_or(VoiceSource {
        source: String::new(),
        license: String::new(),
        voices: Vec::new(),
    });
    for entry in entries {
        let Some(meta) = entry.voice.clone() else {
            continue;
        };
        doc.source = entry.source.clone().unwrap_or_default();
        doc.license = entry
            .license
            .clone()
            .unwrap_or_default()
            .to_ascii_lowercase();
        doc.voices.retain(|v| v.name != meta.name);
        doc.voices.push(meta);
    }
    doc
}

/// Traegt die Zeile der Stimme in `SOURCE.json` ein (ersetzt eine gleichnamige).
fn add_voice_row(voices_dir: &Path, entry: &TtsCatalogEntry) -> Result<()> {
    if entry.voice.is_none() {
        return Ok(());
    }
    let doc = merge_voice_rows(read_voice_source(voices_dir), std::slice::from_ref(entry));
    write_voice_source(voices_dir, &doc)
}

/// Nimmt die Zeile der Stimme heraus; ohne verbleibende Stimmen verschwindet die Datei.
fn remove_voice_row(voices_dir: &Path, name: &str) -> Result<()> {
    let Some(mut doc) = read_voice_source(voices_dir) else {
        return Ok(());
    };
    doc.voices.retain(|v| v.name != name);
    if doc.voices.is_empty() {
        fs::remove_file(voices_dir.join(SOURCE_FILE))?;
        Ok(())
    } else {
        write_voice_source(voices_dir, &doc)
    }
}

// ---- Installieren -------------------------------------------------------------------------------

/// Ausgang eines Installationsaufrufs. Abbruch ist ein Ergebnis, kein Fehler (wie beim Piper-Weg).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Installed,
    Cancelled,
}

/// Alles, was ein Installationsaufruf von aussen braucht (ohne Tauri, damit er testbar ist).
pub(crate) struct InstallEnv<'a> {
    pub dirs: &'a PhononDirs,
    pub platform: Option<&'a str>,
    pub cancel: &'a CancellationToken,
    /// Fortschritt und Pruefereignisse; die Kennung ist die der Oberflaeche (`event_id`).
    pub emit: &'a (dyn Fn(HttpDownloadEvent<'_>) + Send + Sync),
    pub event_id: &'a str,
    /// Liefert das Token erst, wenn ein gesperrtes Modell es braucht.
    pub token: &'a (dyn Fn() -> Option<Secret> + Send + Sync),
    /// Der Ursprung, an den das Token gehen darf (`Origin::huggingface()`).
    pub auth_origin: Origin,
    /// Serialisiert die Aenderungen an `SOURCE.json`.
    pub voices_lock: &'a Mutex<()>,
}

/// Belegt `id`, wenn sie frei ist; sonst `None` (zweiter Start derselben Installation).
pub(crate) fn try_claim(
    flags: &Mutex<HashMap<String, CancellationToken>>,
    id: &str,
) -> Option<CancellationToken> {
    let mut flags = flags.lock().unwrap_or_else(|e| e.into_inner());
    if flags.contains_key(id) {
        return None;
    }
    let token = CancellationToken::new();
    flags.insert(id.to_string(), token.clone());
    Some(token)
}

/// Ein Fortschritt ueber alle Dateien: `base` Bytes sind schon fertig, `total` ist die Summe.
fn aggregated<'a>(
    env: &'a InstallEnv<'a>,
    base: u64,
    total: u64,
) -> impl Fn(HttpDownloadEvent<'_>) + Send + Sync + 'a {
    move |event| match event {
        HttpDownloadEvent::Progress(p) => {
            let downloaded = (base + p.downloaded).min(total);
            let progress = DownloadProgress {
                model_id: env.event_id.to_string(),
                downloaded,
                total,
                percentage: if total > 0 {
                    downloaded as f64 / total as f64 * 100.0
                } else {
                    0.0
                },
            };
            (env.emit)(HttpDownloadEvent::Progress(&progress));
        }
        HttpDownloadEvent::VerificationStarted => {
            (env.emit)(HttpDownloadEvent::VerificationStarted)
        }
        HttpDownloadEvent::VerificationCompleted => {
            (env.emit)(HttpDownloadEvent::VerificationCompleted)
        }
    }
}

/// Eine Datei laden, pruefen und an ihren Platz legen (`dest`). Eine schon vorhandene, richtig grosse
/// Datei wird uebersprungen (Wiederaufnahme nach Abbruch).
async fn fetch_file(
    env: &InstallEnv<'_>,
    entry: &TtsCatalogEntry,
    file: &catalog::TtsCatalogFile,
    dest: &Path,
    base: u64,
    total: u64,
    auth: Option<&DownloadAuth>,
) -> Result<Outcome, String> {
    if file_has_size(dest, file.size_bytes) {
        return Ok(Outcome::Installed);
    }
    let partial = sibling(dest, ".partial");
    let emit = aggregated(env, base, total);
    let outcome = ModelManager::download_http_resumable_authed(
        env.event_id,
        &file.url,
        &partial,
        Some(file.size_bytes),
        file.sha256.as_deref(),
        env.cancel,
        &emit,
        auth,
    )
    .await
    .map_err(|e| describe_error(entry, &e))?;
    match outcome {
        HttpDownloadOutcome::Cancelled => Ok(Outcome::Cancelled),
        HttpDownloadOutcome::Completed => {
            if let Some(blob) = entry.file_git_blobs.get(&file.filename) {
                verify_git_blob(&partial, blob, &file.filename)?;
            }
            fs::rename(&partial, dest).map_err(|e| io_message(&file.filename, &e))?;
            Ok(Outcome::Installed)
        }
    }
}

fn auth_for(env: &InstallEnv<'_>, entry: &TtsCatalogEntry) -> Result<Option<DownloadAuth>, String> {
    if !entry.gated {
        // Oeffentliche Quellen bekommen nie ein Token.
        return Ok(None);
    }
    match (env.token)() {
        Some(token) => Ok(Some(DownloadAuth::new(token, env.auth_origin.clone()))),
        None => Err(no_token_message(entry)),
    }
}

pub(crate) async fn install_runtime(env: &InstallEnv<'_>) -> Result<Outcome, String> {
    let platform = env
        .platform
        .ok_or_else(|| "Für dieses System gibt es kein Phonon-Programm.".to_string())?;
    let entry = runtime_entry(platform)
        .ok_or_else(|| format!("Kein Katalogeintrag für phonon-runtime-{platform}."))?;
    install_runtime_entry(env, platform, &entry).await
}

/// Das Programm eines Katalogeintrags laden und einrichten (getrennt, damit Tests einen
/// Eintrag mit Fake-Adresse nehmen koennen).
async fn install_runtime_entry(
    env: &InstallEnv<'_>,
    platform: &str,
    entry: &TtsCatalogEntry,
) -> Result<Outcome, String> {
    ensure_safe_entry(entry)?;
    if runtime_installed(env.dirs, platform, entry) {
        return Ok(Outcome::Installed);
    }
    let file = entry
        .files
        .first()
        .ok_or_else(|| format!("{}: Katalogeintrag ohne Datei", entry.id))?;
    fs::create_dir_all(env.dirs.root()).map_err(|e| io_message("Phonon", &e))?;
    let archive = env.dirs.root().join(format!("{platform}.download"));
    let emit = aggregated(env, 0, file.size_bytes);
    let outcome = ModelManager::download_http_resumable_authed(
        env.event_id,
        &file.url,
        &archive,
        Some(file.size_bytes),
        file.sha256.as_deref(),
        env.cancel,
        &emit,
        None,
    )
    .await
    .map_err(|e| describe_error(entry, &e))?;
    if outcome_is_cancelled(&outcome) {
        return Ok(Outcome::Cancelled);
    }
    let is_zip = file.filename.to_ascii_lowercase().ends_with(".zip");
    let dest = env.dirs.runtime_dir(platform);
    let extracted = extract_ptts_archive(&archive, &dest, is_zip, platform, entry);
    let _ = fs::remove_file(&archive);
    extracted.map_err(|e| format!("Das Phonon-Programm lässt sich nicht einrichten: {e}"))?;
    Ok(Outcome::Installed)
}

fn outcome_is_cancelled(o: &HttpDownloadOutcome) -> bool {
    matches!(o, HttpDownloadOutcome::Cancelled)
}

/// Installiert das Modell mit den Stimmen seiner Sprache (siehe Moduldoku). Ist das Modell schon
/// vollstaendig da und fehlen nur Stimmen, kommen nur diese dazu: das Modell wird nicht neu geladen.
pub(crate) async fn install_model(
    env: &InstallEnv<'_>,
    entry: &TtsCatalogEntry,
) -> Result<Outcome, String> {
    install_model_with_voices(env, entry, &bundled_voices(&entry.id)).await
}

/// Wie [`install_model`], mit gegebenen Stimmen (getrennt, damit Tests Stimmen mit Fake-Adresse nehmen).
async fn install_model_with_voices(
    env: &InstallEnv<'_>,
    entry: &TtsCatalogEntry,
    voices: &[TtsCatalogEntry],
) -> Result<Outcome, String> {
    ensure_safe_entry(entry)?;
    for voice in voices {
        ensure_safe_entry(voice)?;
    }
    if model_complete(env.dirs, entry) {
        return install_missing_voices(env, entry, voices).await;
    }
    // Ohne Token gar nicht erst anfangen (gesperrtes Modell): der Hinweis kommt sofort.
    let auth = auth_for(env, entry)?;
    let staging = env.dirs.staging_dir(&entry.id);
    fs::create_dir_all(&staging).map_err(|e| io_message(&entry.name, &e))?;
    // Stimmen, die schon am Zielort liegen (aeltere Einzelinstallation, Reparatur), werden nicht neu geladen.
    let missing: Vec<&TtsCatalogEntry> = voices
        .iter()
        .filter(|v| !voice_installed(env.dirs, v))
        .collect();
    let total: u64 = files_bytes(entry) + missing.iter().map(|v| files_bytes(v)).sum::<u64>();
    let done = AtomicU64::new(0);
    for file in &entry.files {
        let dest = staging.join(&file.filename);
        let outcome = fetch_file(
            env,
            entry,
            file,
            &dest,
            done.load(Ordering::Relaxed),
            total,
            auth.as_ref(),
        )
        .await?;
        if outcome == Outcome::Cancelled {
            return Ok(Outcome::Cancelled); // fertige Dateien bleiben fuer die Wiederaufnahme
        }
        done.fetch_add(file.size_bytes, Ordering::Relaxed);
    }
    // Die Stimmen sind oeffentlich (Apache-2.0): nie ein Token, auch nicht beim gesperrten Modell.
    let staging_voices = staging.join(VOICES_SUBDIR);
    if !missing.is_empty() {
        fs::create_dir_all(&staging_voices).map_err(|e| io_message(&entry.name, &e))?;
    }
    for voice in &missing {
        let file = voice
            .files
            .first()
            .ok_or_else(|| format!("{}: Katalogeintrag ohne Datei", voice.id))?;
        let outcome = fetch_file(
            env,
            voice,
            file,
            &staging_voices.join(&file.filename),
            done.load(Ordering::Relaxed),
            total,
            None,
        )
        .await?;
        if outcome == Outcome::Cancelled {
            return Ok(Outcome::Cancelled);
        }
        done.fetch_add(file.size_bytes, Ordering::Relaxed);
    }
    // Ab hier nur noch Dateisystem: SOURCE.json, Marke (zuletzt) und Einsetzen unter der Sperre.
    let _guard = env.voices_lock.lock().unwrap_or_else(|e| e.into_inner());
    if !voices.is_empty() {
        fs::create_dir_all(&staging_voices).map_err(|e| io_message(&entry.name, &e))?;
        let base = read_voice_source(&env.dirs.voices_dir(&entry.id));
        write_voice_source(&staging_voices, &merge_voice_rows(base, voices))
            .map_err(|e| format!("{}: {e}", entry.name))?;
    }
    write_marker(&staging, entry).map_err(|e| format!("{}: {e}", entry.name))?;
    commit_model(env.dirs, entry)
        .map_err(|e| format!("{} lässt sich nicht einsetzen: {e}", entry.name))?;
    Ok(Outcome::Installed)
}

/// Das Modell ist da, es fehlen Stimmen (oder ihre Eintraege in `SOURCE.json`): nur diese laden.
/// Die Marke des Modells und seine Dateien bleiben unberuehrt.
async fn install_missing_voices(
    env: &InstallEnv<'_>,
    entry: &TtsCatalogEntry,
    voices: &[TtsCatalogEntry],
) -> Result<Outcome, String> {
    if voices_present(env.dirs, &entry.id, voices) == voices.len() {
        return Ok(Outcome::Installed);
    }
    let missing: Vec<&TtsCatalogEntry> = voices
        .iter()
        .filter(|v| !voice_installed(env.dirs, v))
        .collect();
    let voices_dir = env.dirs.voices_dir(&entry.id);
    fs::create_dir_all(&voices_dir).map_err(|e| io_message(&entry.name, &e))?;
    let total: u64 = missing.iter().map(|v| files_bytes(v)).sum();
    let mut done = 0u64;
    for voice in &missing {
        let file = voice
            .files
            .first()
            .ok_or_else(|| format!("{}: Katalogeintrag ohne Datei", voice.id))?;
        let dest = voice_file(env.dirs, voice)
            .ok_or_else(|| format!("{}: kein Modell angegeben", voice.id))?;
        let outcome = fetch_file(env, voice, file, &dest, done, total, None).await?;
        if outcome == Outcome::Cancelled {
            return Ok(Outcome::Cancelled); // fertige Stimmen bleiben
        }
        done += file.size_bytes;
    }
    // Die Eintraege sind eine Zugabe: scheitern sie, bleiben die Stimmen benutzbar.
    let _guard = env.voices_lock.lock().unwrap_or_else(|e| e.into_inner());
    let doc = merge_voice_rows(read_voice_source(&voices_dir), voices);
    if let Err(e) = write_voice_source(&voices_dir, &doc) {
        log::warn!("SOURCE.json der Phonon-Stimmen nicht aktualisiert: {e}");
    }
    Ok(Outcome::Installed)
}

/// Setzt den geprueften Zwischenordner als Modellordner ein. Gibt es den Ordner schon (Reparatur, schon
/// installierte Stimmen), kommen die Dateien einzeln hinein: erst die Stimmen samt `SOURCE.json`, dann
/// die Modelldateien, die Marke zuletzt; vorhandene Stimmen bleiben.
fn commit_model(dirs: &PhononDirs, entry: &TtsCatalogEntry) -> Result<()> {
    let staging = dirs.staging_dir(&entry.id);
    let dest = dirs.model_dir(&entry.id);
    if !dest.exists() {
        fs::rename(&staging, &dest)?;
        return Ok(());
    }
    let staged_voices = staging.join(VOICES_SUBDIR);
    if staged_voices.is_dir() {
        let dest_voices = dest.join(VOICES_SUBDIR);
        fs::create_dir_all(&dest_voices)?;
        for item in fs::read_dir(&staged_voices)? {
            let item = item?;
            let name = item.file_name();
            if name.to_string_lossy().ends_with(".partial") {
                continue;
            }
            fs::rename(item.path(), dest_voices.join(&name))?;
        }
    }
    for name in entry
        .files
        .iter()
        .map(|f| f.filename.as_str())
        .chain(std::iter::once(MARKER_FILE))
    {
        fs::rename(staging.join(name), dest.join(name))?;
    }
    let _ = fs::remove_dir_all(&staging);
    Ok(())
}

pub(crate) async fn install_voice(
    env: &InstallEnv<'_>,
    entry: &TtsCatalogEntry,
    parent: &TtsCatalogEntry,
) -> Result<Outcome, String> {
    ensure_safe_entry(entry)?;
    if !model_complete(env.dirs, parent) {
        return Err(format!(
            "Das Modell {} ist nicht installiert. Zuerst das Modell installieren, dann die Stimme.",
            parent.name
        ));
    }
    let file = entry
        .files
        .first()
        .ok_or_else(|| format!("{}: Katalogeintrag ohne Datei", entry.id))?;
    let dest = voice_file(env.dirs, entry)
        .ok_or_else(|| format!("{}: kein Modell angegeben", entry.id))?;
    let voices_dir = env.dirs.voices_dir(&parent.id);
    fs::create_dir_all(&voices_dir).map_err(|e| io_message(&entry.name, &e))?;
    let outcome = fetch_file(env, entry, file, &dest, 0, file.size_bytes, None).await?;
    if outcome == Outcome::Cancelled {
        return Ok(Outcome::Cancelled);
    }
    // Die Metadaten sind eine Zugabe: scheitern sie, bleibt die Stimme benutzbar.
    let _guard = env.voices_lock.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = add_voice_row(&voices_dir, entry) {
        log::warn!("SOURCE.json der Phonon-Stimmen nicht aktualisiert: {e}");
    }
    Ok(Outcome::Installed)
}

/// Installiert, was zur Kennung gehoert (Programmzeile, Modell oder Stimme).
pub(crate) async fn install(env: &InstallEnv<'_>, id: &str) -> Result<Outcome, String> {
    if id == RUNTIME_ID {
        return install_runtime(env).await;
    }
    if let Some(entry) = model_entry(id) {
        return install_model(env, &entry).await;
    }
    if let Some(entry) = voice_entries().into_iter().find(|e| e.id == id) {
        let parent = entry
            .parent
            .as_deref()
            .and_then(model_entry)
            .ok_or_else(|| format!("{id}: das zugehörige Modell fehlt im Katalog"))?;
        return install_voice(env, &entry, &parent).await;
    }
    Err(format!("Unbekannter Phonon-Eintrag: {id}"))
}

// ---- Entfernen ------------------------------------------------------------------------------------

/// Entfernt Programm, Modell (samt Stimmen) oder Stimme.
pub(crate) fn delete(
    dirs: &PhononDirs,
    platform: Option<&str>,
    voices_lock: &Mutex<()>,
    id: &str,
) -> Result<(), String> {
    if id == RUNTIME_ID {
        let platform = platform
            .ok_or_else(|| "Für dieses System gibt es kein Phonon-Programm.".to_string())?;
        let dir = dirs.runtime_dir(platform);
        // Auch ein liegengebliebener Ordner (unterbrochene Einrichtung) darf weg.
        if !dir.is_dir() {
            return Err("Das Phonon-Programm ist nicht installiert.".to_string());
        }
        fs::remove_dir_all(&dir)
            .map_err(|e| format!("Das Programm lässt sich nicht löschen: {e}"))?;
        return Ok(());
    }
    if let Some(entry) = model_entry(id) {
        ensure_safe_entry(&entry)?;
        let (dir, staging) = (dirs.model_dir(id), dirs.staging_dir(id));
        if !dir.exists() && !staging.exists() {
            return Err(format!("{} ist nicht installiert.", entry.name));
        }
        for d in [&dir, &staging] {
            if d.exists() {
                fs::remove_dir_all(d)
                    .map_err(|e| format!("{} lässt sich nicht löschen: {e}", entry.name))?;
            }
        }
        return Ok(());
    }
    if let Some(entry) = voice_entries().into_iter().find(|e| e.id == id) {
        ensure_safe_entry(&entry)?;
        let path =
            voice_file(dirs, &entry).ok_or_else(|| format!("{id}: kein Modell angegeben"))?;
        if !path.is_file() {
            return Err(format!("Die Stimme {} ist nicht installiert.", entry.name));
        }
        fs::remove_file(&path)
            .map_err(|e| format!("Die Stimme {} lässt sich nicht löschen: {e}", entry.name))?;
        let _guard = voices_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let (Some(dir), Some(voice)) = (path.parent(), entry.voice.as_ref()) {
            if let Err(e) = remove_voice_row(dir, &voice.name) {
                log::warn!("SOURCE.json der Phonon-Stimmen nicht aktualisiert: {e}");
            }
            let _ = fs::remove_dir(dir); // nur, wenn leer
        }
        return Ok(());
    }
    Err(format!("Unbekannter Phonon-Eintrag: {id}"))
}

// ---- Tests ----------------------------------------------------------------------------------------

#[cfg(test)]
mod tests;
