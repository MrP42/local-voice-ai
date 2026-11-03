//! Bibliothek (lva#152): Bücher und Hörbücher als **ein** Eintrag je Werk.
//!
//! Ein Buch hat eine Textfassung (eine Vorlesen-Seite, vertonbar), eine
//! Audiofassung (ein importiertes Hörbuch, transkribierbar) oder beide. Kommt
//! eine zweite Fassung desselben Werks dazu, wird sie an das vorhandene Buch
//! gehängt ([`same_work`]); es entsteht nie ein zweiter Eintrag.
//!
//! Ablage unter `<appdata>/library/`:
//! - `<id>/buch.json` – Metadaten und Fassungen,
//! - `<id>/cover.jpg|png` – Cover aus Open Library, der EPUB oder einer Datei,
//! - `<id>/audio/` – das importierte Hörbuch,
//! - `fortschritt.json` – Fortschritt aller Bücher, Seiten und abgeleiteten
//!   Dokumente (Schlüssel → JSON-Text, Schema gehört der Oberfläche).
//!
//! Netz nur auf Knopfdruck: Metadaten und Cover kommen aus Open Library, dabei
//! gehen allein ISBN oder Titel und Autor hinaus, nie Buchtext. Ohne Netz
//! zeichnet die Oberfläche ein Cover aus Titel und Autor.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tauri::AppHandle;

use crate::commands::pages::write_atomic;

static LIBRARY_LOCK: Mutex<()> = Mutex::new(());
fn lock() -> MutexGuard<'static, ()> {
    LIBRARY_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

const OPEN_LIBRARY_SEARCH: &str = "https://openlibrary.org/search.json";
const OPEN_LIBRARY_COVERS: &str = "https://covers.openlibrary.org/b";
const OFFLINE: &str =
    "Open Library ist gerade nicht erreichbar. Prüfe die Internetverbindung und versuche es noch einmal.";
/// Größer ist kein Cover, sondern ein Irrläufer.
const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROGRESS_KEY: usize = 200;
const MAX_PROGRESS_VALUE: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct TextVersion {
    /// Vorlesen-Seite mit dem Text. Sie bleibt die Quelle, damit Leseposition,
    /// Lesezeichen und Seitenmarken erhalten bleiben.
    pub page_id: String,
    /// `document` (EPUB, PDF, DOCX, TXT, MD) oder `transcript` (aus dem Hörbuch).
    pub origin: String,
    /// Dateiname der Quelle, nur zur Anzeige.
    pub source_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct AudioVersion {
    /// Dateiname unter `<id>/audio/`. Bei mehreren Teilen der erste Teil.
    pub file: String,
    pub duration_ms: Option<f64>,
    pub narrator: Option<String>,
    /// Laufende Transkription (Import-Warteschlange), bis der Text übernommen
    /// ist. Gilt für Altbestand mit einer Datei; mehrteilige Hörbücher führen
    /// den Auftrag je Teil in [`AudioTrack::transcript_job`].
    pub transcript_job: Option<String>,
    /// Teile eines mehrteiligen Hörbuchs in Hörreihenfolge. Leer bedeutet:
    /// ein Teil, nämlich `file` ([`tracks_of`]).
    #[serde(default)]
    pub tracks: Vec<AudioTrack>,
    /// Zeitachse Text ↔ Audio aus dem Transkript (Absatzbeginn → Gesamt-ms).
    #[serde(default)]
    pub sync: Vec<SyncPoint>,
}

/// Ein Teil (Datei) eines Hörbuchs.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct AudioTrack {
    /// Dateiname unter `<id>/audio/` (`teil_001.mp3`, …).
    pub file: String,
    /// Anzeigename, beim Import der Dateiname ohne Endung.
    pub title: String,
    pub duration_ms: Option<f64>,
    /// Laufende Transkription dieses Teils (Import-Warteschlange).
    pub transcript_job: Option<String>,
    /// Lange Einzeldatei in Abschnitten (lva#208): ein Auftrag je Zeitabschnitt.
    /// Nicht leer: ersetzt `transcript_job`. Leer: wie bisher ein Auftrag je Teil.
    #[serde(default)]
    pub transcript_jobs: Vec<TranscriptPart>,
}

/// Ein Zeitabschnitt eines Teils mit eigenem Transkriptions-Auftrag (lva#208).
/// Der Auftrag liest nur `start_ms` bis `end_ms` der Datei; seine Segmentzeiten
/// sind relativ zu `start_ms`. Die Abschnitte überlappen an den Rändern, damit
/// kein Wort zerschnitten wird; beim Zusammenführen gehört jedes Segment dem
/// Abschnitt, in dessen Bereich `from_ms`…`to_ms` seine Mitte liegt.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct TranscriptPart {
    /// Hilfs-Besprechung der Import-Warteschlange.
    pub job: String,
    /// Beginn des dekodierten Bereichs in der Datei (ms); Versatz der Segmente.
    pub start_ms: f64,
    /// Ende des dekodierten Bereichs (ms); `None` = bis zum Dateiende.
    pub end_ms: Option<f64>,
    /// Ab hier gehören die Segmente diesem Abschnitt (ms in der Datei).
    pub from_ms: f64,
    /// Bis hier (ausschließlich); `None` beim letzten Abschnitt.
    pub to_ms: Option<f64>,
}

/// Stützpunkt der Zeitachse: ab dieser Zeichenstelle des Textes ist das
/// Hörbuch bei `ms` (Millisekunden ab Beginn des ersten Teils).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct SyncPoint {
    pub chars: u32,
    pub ms: f64,
}

/// Die Teile einer Audiofassung. Altbestand ohne `tracks` hat genau einen
/// Teil: die Datei `file` mit Dauer und Auftrag der Fassung.
pub fn tracks_of(audio: &AudioVersion) -> Vec<AudioTrack> {
    if !audio.tracks.is_empty() {
        return audio.tracks.clone();
    }
    if audio.file.is_empty() {
        return Vec::new();
    }
    vec![AudioTrack {
        file: audio.file.clone(),
        title: Path::new(&audio.file)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        duration_ms: audio.duration_ms,
        transcript_job: audio.transcript_job.clone(),
        transcript_jobs: Vec::new(),
    }]
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct LibraryBook {
    /// Leer beim Anlegen; das Backend vergibt `book_<ms>`.
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub isbn: Option<String>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub pages: Option<u32>,
    pub description: Option<String>,
    /// Woher die Metadaten stammen: `openlibrary`, `epub`, `manual`.
    pub metadata_source: Option<String>,
    /// Woher das Cover stammt: `openlibrary`, `epub`, `file` oder `generated`.
    pub cover_source: String,
    pub text: Option<TextVersion>,
    pub audio: Option<AudioVersion>,
    pub added_ms: f64,
    pub finished: bool,
    /// Favorit: steht in der Bibliothek vor den übrigen Büchern. Gehört zum
    /// Buch (nicht zur Anzeige) und reist deshalb in der Sync-Sammlung `book` mit.
    #[serde(default)]
    pub favorite: bool,
    /// Nur zur Anzeige, wird beim Auflisten gesetzt: absoluter Pfad des Covers
    /// samt Änderungszeit (`?v=`), damit ein neues Cover sofort erscheint.
    pub cover_path: Option<String>,
    /// Nur zur Anzeige: absoluter Pfad des importierten Hörbuchs (erster Teil).
    pub audio_path: Option<String>,
    /// Nur zur Anzeige: absolute Pfade aller Teile in Hörreihenfolge. Ein
    /// fehlender Teil steht als leerer Text an seiner Stelle; ist keiner da,
    /// ist die Liste leer.
    #[serde(default)]
    pub audio_paths: Vec<String>,
}

fn generated() -> String {
    "generated".to_string()
}

/// Ein Treffer aus Open Library, als Vorschlag für den Dialog.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct MetadataCandidate {
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub first_publish_year: Option<u32>,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
    pub language: Option<String>,
    pub pages: Option<u32>,
    /// Open-Library-Cover-Kennung (`cover_i`).
    pub cover_id: Option<i64>,
    /// Werk-Schlüssel (`/works/OL…W`) für den Klappentext.
    pub work_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ProgressEntry {
    pub key: String,
    /// `None` löscht den Eintrag.
    pub value: Option<String>,
}

// ---------------------------------------------------------------------------
// Pfade
// ---------------------------------------------------------------------------

fn library_root(app: &AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let base = crate::portable::data_dir()
        .cloned()
        .or_else(|| app.path().app_local_data_dir().ok())
        .ok_or("Kein Datenverzeichnis verfügbar")?;
    let root = base.join("library");
    std::fs::create_dir_all(&root).map_err(|e| format!("could not create library dir: {e}"))?;
    Ok(root)
}

/// Kennungen entstehen nur hier; alles andere ist ein manipulierter Aufruf.
fn checked_id(id: &str) -> Result<&str, String> {
    let valid = id
        .strip_prefix("book_")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric()));
    if valid {
        Ok(id)
    } else {
        Err(format!("Ungültige Buch-Kennung: {id}"))
    }
}

fn book_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    Ok(root.join(checked_id(id)?))
}

fn fresh_id(root: &Path) -> String {
    let mut ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    // Zwei Importe in derselben Millisekunde dürfen sich nicht überschreiben.
    while root.join(format!("book_{ms}")).exists() {
        ms += 1;
    }
    format!("book_{ms}")
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or_default()
}

fn cover_file(dir: &Path) -> Option<PathBuf> {
    ["cover.jpg", "cover.png", "cover.webp"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

fn read_book(dir: &Path) -> Option<LibraryBook> {
    let raw = std::fs::read_to_string(dir.join("buch.json")).ok()?;
    let mut book: LibraryBook = serde_json::from_str(&raw)
        .map_err(|e| log::warn!("Bibliothek: {} unlesbar: {e}", dir.display()))
        .ok()?;
    if let Some(cover) = cover_file(dir) {
        let stamp = std::fs::metadata(&cover)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis())
            .unwrap_or_default();
        book.cover_path = Some(format!("{}?v={stamp}", cover.to_string_lossy()));
    } else {
        book.cover_source = generated();
    }
    if let Some(audio) = &book.audio {
        let paths: Vec<String> = tracks_of(audio)
            .iter()
            .map(|t| {
                let path = dir.join("audio").join(&t.file);
                if path.is_file() {
                    path.to_string_lossy().into_owned()
                } else {
                    String::new()
                }
            })
            .collect();
        book.audio_path = paths.iter().find(|p| !p.is_empty()).cloned();
        book.audio_paths = if book.audio_path.is_some() {
            paths
        } else {
            Vec::new()
        };
    }
    Some(book)
}

fn write_book(dir: &Path, book: &LibraryBook) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create book dir: {e}"))?;
    let mut stored = book.clone();
    stored.cover_path = None;
    stored.audio_path = None;
    stored.audio_paths = Vec::new();
    let json = serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?;
    write_atomic(&dir.join("buch.json"), &json)
}

fn list_books(root: &Path) -> Vec<LibraryBook> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut books: Vec<LibraryBook> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| checked_id(&e.file_name().to_string_lossy()).is_ok())
        .filter_map(|e| read_book(&e.path()))
        .collect();
    books.sort_by(|a, b| b.added_ms.total_cmp(&a.added_ms));
    books
}

/// Nur ein Dateiname im eigenen audio-Ordner, kein Pfad.
fn plain_name(name: &str) -> bool {
    let plain = Path::new(name)
        .file_name()
        .is_some_and(|n| n == std::ffi::OsStr::new(name));
    plain && !name.contains(['/', '\\', ':']) && !name.starts_with('.')
}

/// Prüft die Datei der Fassung und jeden Teil wie bisher `file`.
fn validate_audio_names(audio: &AudioVersion) -> Result<(), String> {
    if !plain_name(&audio.file) {
        return Err("Ungültiger Dateiname der Audiofassung.".into());
    }
    if audio.tracks.iter().any(|t| !plain_name(&t.file)) {
        return Err("Ungültiger Dateiname eines Hörbuch-Teils.".into());
    }
    Ok(())
}

/// Hörbuch-Formate, die der Import kopiert und die Hörleiste abspielen kann.
const AUDIO_EXTENSIONS: [&str; 9] = [
    "mp3", "m4b", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma",
];
/// Audible-Formate: DRM-geschützt, wird nicht umgangen.
const DRM_EXTENSIONS: [&str; 3] = ["aax", "aa", "aaxc"];

/// Endung einer Hördatei (klein). Audible-Dateien werden mit klarer Meldung
/// abgewiesen; der Kern entschlüsselt oder umgeht kein DRM.
fn audio_ext(source: &Path) -> Result<String, String> {
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    if DRM_EXTENSIONS.contains(&ext.as_str()) {
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Err(format!(
            "\u{201e}{name}\u{201c} ist mit Audible-DRM geschützt und kann nicht importiert werden. Verwende eine DRM-freie Fassung (MP3, M4B)."
        ));
    }
    if !AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!(
            "Kein Hörbuch-Format: .{ext} (unterstützt: {}).",
            AUDIO_EXTENSIONS.join(", ")
        ));
    }
    Ok(ext)
}

/// Nummer im Dateinamen `teil_007.mp3` → 7.
fn part_number(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("teil_")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn part_name(number: u32, ext: &str) -> String {
    format!("teil_{number:03}.{ext}")
}

/// Nächste freie Teilnummer: nach der größten Nummer, die Teile oder Dateien
/// im Ordner schon tragen.
fn next_part_number(audio_dir: &Path, tracks: &[AudioTrack]) -> u32 {
    let on_disk = std::fs::read_dir(audio_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| part_number(&e.file_name().to_string_lossy()));
    tracks
        .iter()
        .filter_map(|t| part_number(&t.file))
        .chain(on_disk)
        .max()
        .unwrap_or(0)
        + 1
}

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Gehört die Datei zu einer Audiofassung (`hoerbuch.*`, `teil_*`)?
fn is_audio_part(path: &Path) -> bool {
    path.is_file()
        && path.file_name().is_some_and(|n| {
            let n = n.to_string_lossy();
            n.starts_with("teil_") || n.starts_with("hoerbuch.")
        })
}

/// Sperre je Buch: hält die ganze Dateiphase eines Imports oder Zusammenführens
/// (Nummern vergeben, kopieren, verschieben, buch.json schreiben), damit zwei
/// Läufe auf dasselbe Buch sich nicht überholen. Immer vor der globalen Sperre
/// nehmen, bei mehreren Büchern in sortierter Reihenfolge.
fn book_locks(ids: &[&str]) -> Vec<std::sync::Arc<Mutex<()>>> {
    static LOCKS: Mutex<BTreeMap<String, std::sync::Arc<Mutex<()>>>> = Mutex::new(BTreeMap::new());
    let mut sorted: Vec<&str> = ids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut map = LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    sorted
        .into_iter()
        .map(|id| map.entry(id.to_string()).or_default().clone())
        .collect()
}

/// Kopiert die Quellen der Reihe nach als `teil_<start>.<ext>` … in den
/// Zwischenordner `stage` (jede Datei als `.part`, dann umbenannt). Der Bestand
/// bleibt unberührt.
fn stage_tracks(stage: &Path, sources: &[PathBuf], start: u32) -> Result<Vec<AudioTrack>, String> {
    std::fs::create_dir_all(stage).map_err(|e| e.to_string())?;
    let mut tracks = Vec::with_capacity(sources.len());
    for (i, source) in sources.iter().enumerate() {
        let ext = audio_ext(source)?;
        let file = part_name(start + i as u32, &ext);
        let tmp = stage.join(format!("{file}.part"));
        std::fs::copy(source, &tmp)
            .map_err(|e| format!("Kopieren fehlgeschlagen ({}): {e}", stem_of(source)))?;
        std::fs::rename(&tmp, stage.join(&file)).map_err(|e| e.to_string())?;
        tracks.push(AudioTrack {
            file,
            title: stem_of(source),
            ..Default::default()
        });
    }
    Ok(tracks)
}

/// Legt die gestagten Dateien in `audio_dir` und schreibt dann (`write`) die
/// buch.json. Bei `replace` werden die alten Teile vorher nur beiseite
/// gelegt (`.alt-…`) und erst nach erfolgreichem Schreiben gelöscht. Jeder
/// Fehler rollt zurück: neue Dateien gehen zurück in `stage`, alte an ihren
/// Platz. Ein vorhandenes Ziel wird nie überschrieben. Gibt Meldungen über
/// Aufräumfehler zurück (das Ergebnis selbst steht dann).
fn commit_staged(
    audio_dir: &Path,
    stage: &Path,
    files: &[String],
    replace: bool,
    write: impl FnOnce() -> Result<(), String>,
) -> Result<Vec<String>, String> {
    let alt = audio_dir.join(format!(".alt-{}", now_ms() as i64));
    let mut parked: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut placed: Vec<(PathBuf, PathBuf)> = Vec::new();
    let undo = |parked: &[(PathBuf, PathBuf)], placed: &[(PathBuf, PathBuf)], err: String| {
        let mut failed = Vec::new();
        for (from, to) in placed.iter().rev() {
            if let Err(e) = std::fs::rename(to, from) {
                failed.push(format!("{}: {e}", to.display()));
            }
        }
        for (orig, in_alt) in parked.iter().rev() {
            if let Err(e) = std::fs::rename(in_alt, orig) {
                failed.push(format!("{}: {e}", orig.display()));
            }
        }
        let _ = std::fs::remove_dir(&alt);
        if failed.is_empty() {
            err
        } else {
            format!("{err} (Zurückrollen unvollständig: {})", failed.join("; "))
        }
    };
    if replace {
        std::fs::create_dir_all(&alt).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(audio_dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if !is_audio_part(&path) {
                continue;
            }
            let to = alt.join(entry.file_name());
            if let Err(e) = std::fs::rename(&path, &to) {
                return Err(undo(&parked, &placed, e.to_string()));
            }
            parked.push((path, to));
        }
    }
    for file in files {
        let from = stage.join(file);
        let to = audio_dir.join(file);
        if to.exists() {
            return Err(undo(
                &parked,
                &placed,
                format!("Die Datei \u{201e}{file}\u{201c} gibt es im Hörbuch schon."),
            ));
        }
        if let Err(e) = std::fs::rename(&from, &to) {
            return Err(undo(&parked, &placed, e.to_string()));
        }
        placed.push((from, to));
    }
    if let Err(e) = write() {
        return Err(undo(&parked, &placed, e));
    }
    let mut warnings = Vec::new();
    if replace {
        if let Err(e) = std::fs::remove_dir_all(&alt) {
            warnings.push(format!("Alte Hörbuch-Dateien nicht gelöscht: {e}"));
        }
    }
    Ok(warnings)
}

/// Importiert Dateien als Teile eines Buchs (Kern von
/// `library_audio_import_tracks`): Buchsperre über die ganze Dateiphase,
/// Nummern erst unter der Sperre, alte Dateien erst nach erfolgreichem
/// Schreiben der buch.json weg. Ein Text aus Transkript fällt beim Ersetzen weg
/// (er passt nicht mehr zum neuen Hörbuch).
fn import_tracks_core(
    root: &Path,
    id: &str,
    sources: &[PathBuf],
    append: bool,
) -> Result<LibraryBook, String> {
    let dir = book_dir(root, id)?;
    let locks = book_locks(&[id]);
    let _book_guards: Vec<_> = locks
        .iter()
        .map(|l| l.lock().unwrap_or_else(|e| e.into_inner()))
        .collect();
    let existing = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    let audio_dir = dir.join("audio");
    std::fs::create_dir_all(&audio_dir).map_err(|e| e.to_string())?;
    let start = if append {
        next_part_number(
            &audio_dir,
            &existing.audio.as_ref().map(tracks_of).unwrap_or_default(),
        )
    } else {
        1
    };
    let stage = audio_dir.join(format!(".neu-{}", now_ms() as i64));
    let outcome = (|| -> Result<(LibraryBook, Vec<String>), String> {
        let tracks = stage_tracks(&stage, sources, start)?;
        let files: Vec<String> = tracks.iter().map(|t| t.file.clone()).collect();
        let _guard = lock();
        let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
        book.audio = Some(with_tracks(book.audio.take(), tracks, append));
        if !append && book.text.as_ref().is_some_and(|t| t.origin == "transcript") {
            book.text = None;
        }
        let warnings = commit_staged(&audio_dir, &stage, &files, !append, || {
            write_book(&dir, &book)
        })?;
        let fresh = read_book(&dir).ok_or_else(|| "Buch unlesbar.".to_string())?;
        Ok((fresh, warnings))
    })();
    if let Err(e) = std::fs::remove_dir_all(&stage) {
        if stage.exists() {
            log::warn!("Zwischenordner nicht gelöscht: {e}");
        }
    }
    let (book, warnings) = outcome?;
    for w in warnings {
        log::warn!("{w}");
    }
    Ok(book)
}

/// Audiofassung nach einem Import: bei `append` kommen die neuen Teile hinter
/// die vorhandenen (Altbestand mit einer Datei wird dabei Teil 1), sonst
/// ersetzen sie alles.
fn with_tracks(previous: Option<AudioVersion>, new: Vec<AudioTrack>, append: bool) -> AudioVersion {
    let previous = previous.unwrap_or_default();
    let mut tracks = if append {
        tracks_of(&previous)
    } else {
        Vec::new()
    };
    tracks.extend(new);
    AudioVersion {
        file: tracks.first().map(|t| t.file.clone()).unwrap_or_default(),
        duration_ms: None,
        narrator: previous.narrator,
        transcript_job: None,
        // Angehängte Teile ändern die Zeitachse der vorderen nicht.
        sync: if append { previous.sync } else { Vec::new() },
        tracks,
    }
}

// ---------------------------------------------------------------------------
// Dubletten
// ---------------------------------------------------------------------------

/// Nur Buchstaben und Ziffern, klein, Umlaute gefaltet: „Faust. Der
/// Tragödie erster Teil“ und „faust der tragoedie erster teil“ sind gleich.
fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            'à' | 'á' | 'â' => out.push('a'),
            'è' | 'é' | 'ê' => out.push('e'),
            'ì' | 'í' | 'î' => out.push('i'),
            'ò' | 'ó' | 'ô' => out.push('o'),
            'ù' | 'ú' | 'û' => out.push('u'),
            c if c.is_alphanumeric() => out.push(c),
            _ => {}
        }
    }
    out
}

/// Titel ohne Untertitel (vor „:“, „ – “ oder „. “) und ohne führenden Artikel.
fn core_title(title: &str) -> String {
    let head = title
        .split([':', '|', '(', '['])
        .next()
        .unwrap_or(title)
        .split(" – ")
        .next()
        .unwrap_or(title)
        .split(" - ")
        .next()
        .unwrap_or(title);
    let folded = fold(head);
    for article in ["der", "die", "das", "the", "a", "an", "le", "la", "les"] {
        if let Some(rest) = folded.strip_prefix(article) {
            // Nur echte Artikel: „Dieb“ beginnt nicht mit „die“ + Wort.
            let words: Vec<&str> = head.split_whitespace().collect();
            if words.len() > 1 && fold(words[0]) == article && !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    folded
}

/// Nachnamen der Autoren (letztes Wort, gefaltet).
fn surnames(authors: &[String]) -> Vec<String> {
    authors
        .iter()
        .flat_map(|a| a.split([',', '&', ';']))
        .flat_map(|a| a.split(" und ").map(str::to_string).collect::<Vec<_>>())
        .filter_map(|a| a.split_whitespace().last().map(fold))
        .filter(|s| s.len() > 1)
        .collect()
}

fn digits(isbn: &str) -> String {
    isbn.chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .collect::<String>()
        .to_uppercase()
}

/// Gleiches Werk? Bei passender ISBN sicher; sonst gleicher Kerntitel und,
/// wenn beide Autoren kennen, mindestens ein gemeinsamer Nachname.
pub fn same_work(
    a_title: &str,
    a_authors: &[String],
    a_isbn: Option<&str>,
    b: &LibraryBook,
) -> bool {
    if let (Some(x), Some(y)) = (a_isbn, b.isbn.as_deref()) {
        let (x, y) = (digits(x), digits(y));
        if !x.is_empty() && x == y {
            return true;
        }
    }
    let (ta, tb) = (core_title(a_title), core_title(&b.title));
    if ta.is_empty() || ta != tb {
        return false;
    }
    let (sa, sb) = (surnames(a_authors), surnames(&b.authors));
    sa.is_empty() || sb.is_empty() || sa.iter().any(|s| sb.contains(s))
}

// ---------------------------------------------------------------------------
// Open Library
// ---------------------------------------------------------------------------

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("LocalVoiceAI (Bibliothek; Metadaten und Cover)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// Treffer aus `search.json` lesen. Höchstens `limit`, ohne leere Titel.
pub fn parse_search(body: &str, limit: usize) -> Result<Vec<MetadataCandidate>, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("Open Library: unerwartete Antwort ({e})"))?;
    let docs = value["docs"].as_array().cloned().unwrap_or_default();
    let strings = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::trim).map(str::to_string))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(docs
        .iter()
        .filter_map(|d| {
            let title = d["title"].as_str()?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let isbns = strings(&d["isbn"]);
            Some(MetadataCandidate {
                title,
                subtitle: d["subtitle"].as_str().map(str::to_string),
                authors: strings(&d["author_name"]),
                first_publish_year: d["first_publish_year"].as_u64().map(|y| y as u32),
                publisher: strings(&d["publisher"]).into_iter().next(),
                // ISBN-13 bevorzugt, sie ist auf heutigen Ausgaben gedruckt.
                isbn: isbns
                    .iter()
                    .find(|i| i.len() == 13)
                    .or_else(|| isbns.first())
                    .cloned(),
                language: strings(&d["language"]).into_iter().next(),
                pages: d["number_of_pages_median"].as_u64().map(|p| p as u32),
                cover_id: d["cover_i"].as_i64().filter(|i| *i > 0),
                work_key: d["key"]
                    .as_str()
                    .filter(|k| k.starts_with("/works/"))
                    .map(str::to_string),
            })
        })
        .take(limit)
        .collect())
}

/// Klappentext aus einem Werk-Datensatz (`description` als Text oder Objekt).
pub fn parse_description(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let d = &value["description"];
    let text = d.as_str().or_else(|| d["value"].as_str())?.trim();
    // Open Library hängt gern Quellenblöcke an („----------“ und Links).
    let text = text.split("\n----------").next().unwrap_or(text).trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Bildformat aus den ersten Bytes. Nur echte Bilder werden Cover.
fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

fn store_cover(dir: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_COVER_BYTES {
        return Err("Das Bild ist zu groß für ein Cover (höchstens 8 MB).".into());
    }
    // Open Library liefert für fehlende Cover ein 1×1-GIF bzw. 43 Byte.
    if bytes.len() < 512 {
        return Err("Für diese Ausgabe gibt es kein Cover.".into());
    }
    let ext = image_ext(bytes).ok_or("Die Datei ist kein JPEG-, PNG- oder WebP-Bild.")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for old in ["cover.jpg", "cover.png", "cover.webp"] {
        let _ = std::fs::remove_file(dir.join(old));
    }
    write_atomic(&dir.join(format!("cover.{ext}")), bytes)
}

// ---------------------------------------------------------------------------
// EPUB-Cover
// ---------------------------------------------------------------------------

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let start = tag.find(&needle)? + needle.len();
    let quote = tag[start..].chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &tag[start + 1..];
    rest.find(quote).map(|end| &rest[..end])
}

/// Pfad des Coverbilds im Archiv aus dem OPF: EPUB 3 `properties="cover-image"`,
/// sonst EPUB 2 `<meta name="cover" content="id">`.
pub fn epub_cover_href(opf: &str) -> Option<String> {
    let items: Vec<&str> = opf
        .split('<')
        .filter(|t| t.starts_with("item ") || t.starts_with("opf:item "))
        .collect();
    if let Some(item) = items.iter().find(|t| {
        attr(t, "properties").is_some_and(|p| p.split_whitespace().any(|p| p == "cover-image"))
    }) {
        return attr(item, "href").map(str::to_string);
    }
    let cover_id = opf
        .split('<')
        .filter(|t| t.starts_with("meta ") || t.starts_with("opf:meta "))
        .find(|t| attr(t, "name") == Some("cover"))
        .and_then(|t| attr(t, "content"))?;
    items
        .iter()
        .find(|t| attr(t, "id") == Some(cover_id))
        .and_then(|t| attr(t, "href"))
        .map(str::to_string)
}

fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    max: usize,
) -> Option<Vec<u8>> {
    let entry = archive.by_name(name).ok()?;
    if entry.size() as usize > max {
        return None;
    }
    let mut buf = Vec::new();
    entry.take(max as u64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn epub_cover(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("kein EPUB (zip): {e}"))?;
    let container = read_zip_entry(&mut archive, "META-INF/container.xml", 64 * 1024)
        .ok_or("EPUB ohne container.xml")?;
    let container = String::from_utf8_lossy(&container);
    let opf_path = container
        .split('<')
        .find(|t| t.starts_with("rootfile "))
        .and_then(|t| attr(t, "full-path"))
        .ok_or("EPUB ohne OPF")?
        .to_string();
    let opf = read_zip_entry(&mut archive, &opf_path, 2 * 1024 * 1024).ok_or("OPF unlesbar")?;
    let href =
        epub_cover_href(&String::from_utf8_lossy(&opf)).ok_or("Die EPUB nennt kein Cover.")?;
    let base = opf_path.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
    let joined = if base.is_empty() {
        href.clone()
    } else {
        format!("{base}/{href}")
    };
    // `../` im href auflösen, ohne das Archiv zu verlassen.
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            p => parts.push(p),
        }
    }
    let name = parts.join("/");
    let name = percent_decode(&name);
    read_zip_entry(&mut archive, &name, MAX_COVER_BYTES)
        .ok_or_else(|| "Coverbild fehlt in der EPUB.".into())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Fortschritt
// ---------------------------------------------------------------------------

fn progress_path(root: &Path) -> PathBuf {
    root.join("fortschritt.json")
}

/// Gespeicherten Fortschritt lesen. Fehlt die Datei, ist er leer. Ist sie
/// unlesbar, wird sie als `fortschritt.json.defekt-<ms>` beiseitegelegt statt
/// beim nächsten Sichern überschrieben – so bleibt der alte Stand rettbar.
fn load_progress(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let path = progress_path(root);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("Fortschritt nicht lesbar: {e}")),
    };
    match serde_json::from_str(&raw) {
        Ok(map) => Ok(map),
        Err(e) => {
            let aside = root.join(format!("fortschritt.json.defekt-{}", now_ms() as i64));
            std::fs::rename(&path, &aside).map_err(|r| {
                format!("Fortschritt beschädigt ({e}), Sichern fehlgeschlagen: {r}")
            })?;
            log::warn!("fortschritt.json beschädigt ({e}), beiseitegelegt");
            Ok(BTreeMap::new())
        }
    }
}

/// Einträge einpflegen (`None` löscht). Ungültige Schlüssel oder übergroße
/// Werte werden verworfen statt die ganze Datei zu gefährden.
pub fn merge_progress(map: &mut BTreeMap<String, String>, entries: Vec<ProgressEntry>) {
    for entry in entries {
        if entry.key.is_empty() || entry.key.len() > MAX_PROGRESS_KEY {
            log::warn!("Fortschritts-Schlüssel ungültig, verworfen");
            continue;
        }
        match entry.value {
            Some(v) if v.len() <= MAX_PROGRESS_VALUE => {
                map.insert(entry.key, v);
            }
            Some(_) => log::warn!(
                "Fortschritt zu groß, verworfen ({} Zeichen Schlüssel)",
                entry.key.len()
            ),
            None => {
                map.remove(&entry.key);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Befehle
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub fn library_list(app: AppHandle) -> Result<Vec<LibraryBook>, String> {
    let root = library_root(&app)?;
    Ok(list_books(&root))
}

/// Anlegen (leere Kennung) oder ändern. Gibt das gespeicherte Buch zurück.
#[tauri::command]
#[specta::specta]
pub fn library_save(app: AppHandle, book: LibraryBook) -> Result<LibraryBook, String> {
    let _guard = lock();
    let root = library_root(&app)?;
    let mut book = book;
    book.title = book.title.trim().to_string();
    if book.title.is_empty() {
        return Err("Ein Buch braucht einen Titel.".into());
    }
    if book.id.is_empty() {
        book.id = fresh_id(&root);
        if book.added_ms <= 0.0 {
            book.added_ms = now_ms();
        }
    }
    let dir = book_dir(&root, &book.id)?;
    if let Some(audio) = &book.audio {
        validate_audio_names(audio)?;
    }
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch nach dem Speichern unlesbar.".into())
}

/// Buch aus der Bibliothek entfernen, samt Cover und importiertem Hörbuch.
/// Die Vorlesen-Seite der Textfassung bleibt (sie löscht die Oberfläche auf
/// Wunsch selbst), ebenso der Fortschritt bis zum nächsten Aufräumen.
#[tauri::command]
#[specta::specta]
pub fn library_delete(app: AppHandle, id: String) -> Result<(), String> {
    let _guard = lock();
    let root = library_root(&app)?;
    let dir = book_dir(&root, &id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("could not delete book: {e}"))?;
    }
    Ok(())
}

/// Gibt es das Werk schon? Dann soll die neue Fassung angehängt werden.
#[tauri::command]
#[specta::specta]
pub fn library_find_duplicate(
    app: AppHandle,
    title: String,
    authors: Vec<String>,
    isbn: Option<String>,
) -> Result<Option<LibraryBook>, String> {
    let root = library_root(&app)?;
    Ok(list_books(&root)
        .into_iter()
        .find(|b| same_work(&title, &authors, isbn.as_deref(), b)))
}

/// Ausgaben bei Open Library suchen: per ISBN oder per Titel und Autor.
#[tauri::command]
#[specta::specta]
pub async fn library_metadata_search(
    title: String,
    author: String,
    isbn: Option<String>,
) -> Result<Vec<MetadataCandidate>, String> {
    const FIELDS: &str = "key,title,subtitle,author_name,first_publish_year,publisher,isbn,cover_i,language,number_of_pages_median";
    let mut query: Vec<(&str, String)> = vec![("fields", FIELDS.into()), ("limit", "8".into())];
    match isbn.as_deref().and_then(crate::isbn::normalize_isbn) {
        Some(isbn) => query.push(("isbn", isbn)),
        None => {
            let title = title.trim();
            if title.is_empty() {
                return Ok(Vec::new());
            }
            query.push(("title", title.chars().take(200).collect()));
            let author = author.trim();
            if !author.is_empty() {
                query.push(("author", author.chars().take(120).collect()));
            }
        }
    }
    let response = http()?
        .get(OPEN_LIBRARY_SEARCH)
        .query(&query)
        .send()
        .await
        .map_err(|e| {
            log::warn!("Bibliothek: Open Library nicht erreichbar: {e}");
            OFFLINE.to_string()
        })?;
    if !response.status().is_success() {
        return Err(format!("Open Library antwortete mit {}", response.status()));
    }
    parse_search(&response.text().await.map_err(|e| e.to_string())?, 6)
}

/// Klappentext eines Werks (`/works/OL…W`).
#[tauri::command]
#[specta::specta]
pub async fn library_description(work_key: String) -> Result<Option<String>, String> {
    let valid = work_key
        .strip_prefix("/works/OL")
        .and_then(|r| r.strip_suffix('W'))
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    if !valid {
        return Err("Ungültiger Werk-Schlüssel.".into());
    }
    let response = http()?
        .get(format!("https://openlibrary.org{work_key}.json"))
        .send()
        .await
        .map_err(|_| OFFLINE.to_string())?;
    if !response.status().is_success() {
        return Ok(None);
    }
    Ok(parse_description(
        &response.text().await.map_err(|e| e.to_string())?,
    ))
}

/// Cover aus Open Library laden (per Cover-Kennung oder ISBN) und ablegen.
#[tauri::command]
#[specta::specta]
pub async fn library_cover_fetch(
    app: AppHandle,
    id: String,
    cover_id: Option<i64>,
    isbn: Option<String>,
) -> Result<LibraryBook, String> {
    let root = library_root(&app)?;
    let dir = book_dir(&root, &id)?;
    let url = match (
        cover_id.filter(|c| *c > 0),
        isbn.as_deref().and_then(crate::isbn::normalize_isbn),
    ) {
        (Some(c), _) => format!("{OPEN_LIBRARY_COVERS}/id/{c}-L.jpg"),
        (None, Some(i)) => format!("{OPEN_LIBRARY_COVERS}/isbn/{i}-L.jpg?default=false"),
        _ => return Err("Ohne Cover-Kennung oder ISBN lässt sich kein Cover laden.".into()),
    };
    let response = http()?
        .get(url)
        .send()
        .await
        .map_err(|_| OFFLINE.to_string())?;
    if !response.status().is_success() {
        return Err("Für diese Ausgabe gibt es kein Cover.".into());
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_COVER_BYTES as u64)
    {
        return Err("Das Cover ist zu groß.".into());
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_COVER_BYTES {
            return Err("Das Cover ist zu groß.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let _guard = lock();
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    store_cover(&dir, &bytes)?;
    book.cover_source = "openlibrary".into();
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Cover aus einer Bilddatei oder dem Coverbild einer EPUB übernehmen.
#[tauri::command]
#[specta::specta]
pub async fn library_cover_from_file(
    app: AppHandle,
    id: String,
    path: String,
) -> Result<LibraryBook, String> {
    let root = library_root(&app)?;
    let dir = book_dir(&root, &id)?;
    let source = PathBuf::from(&path);
    let is_epub = source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("epub"));
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        if is_epub {
            epub_cover(&source)
        } else {
            let meta = std::fs::metadata(&source).map_err(|e| e.to_string())?;
            if meta.len() as usize > MAX_COVER_BYTES {
                return Err("Das Bild ist zu groß für ein Cover (höchstens 8 MB).".into());
            }
            std::fs::read(&source).map_err(|e| e.to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    let _guard = lock();
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    store_cover(&dir, &bytes)?;
    book.cover_source = if is_epub { "epub" } else { "file" }.into();
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Eigenes Cover entfernen: die Oberfläche zeichnet wieder das generierte.
#[tauri::command]
#[specta::specta]
pub fn library_cover_clear(app: AppHandle, id: String) -> Result<LibraryBook, String> {
    let _guard = lock();
    let root = library_root(&app)?;
    let dir = book_dir(&root, &id)?;
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    for old in ["cover.jpg", "cover.png", "cover.webp"] {
        let _ = std::fs::remove_file(dir.join(old));
    }
    book.cover_source = generated();
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Hörbuch in die Bibliothek kopieren und als Audiofassung eintragen. Die
/// Kopie macht das Buch unabhängig davon, ob die Quelldatei liegen bleibt.
#[tauri::command]
#[specta::specta]
pub async fn library_audio_import(
    app: AppHandle,
    id: String,
    path: String,
) -> Result<LibraryBook, String> {
    let source = PathBuf::from(&path);
    let ext = audio_ext(&source)?;
    let root = library_root(&app)?;
    let dir = book_dir(&root, &id)?;
    if read_book(&dir).is_none() {
        return Err("Buch nicht gefunden.".into());
    }
    let audio_dir = dir.join("audio");
    let file = format!("hoerbuch.{ext}");
    let target = audio_dir.join(&file);
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        std::fs::create_dir_all(&audio_dir).map_err(|e| e.to_string())?;
        // Erst vollständig kopieren, dann umbenennen: ein Abbruch hinterlässt
        // keine halbe Datei unter dem endgültigen Namen.
        // Altes Hörbuch erst entfernen, wenn das neue an seinem Platz ist.
        let tmp = audio_dir.join(format!(
            "{}.part",
            target.file_name().unwrap_or_default().to_string_lossy()
        ));
        std::fs::copy(&source, &tmp).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Kopieren fehlgeschlagen: {e}")
        })?;
        std::fs::rename(&tmp, &target).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })?;
        for entry in std::fs::read_dir(&audio_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            let p = entry.path();
            if p != target && p.file_stem().is_some_and(|s| s == "hoerbuch") {
                let _ = std::fs::remove_file(p);
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    let _guard = lock();
    let Some(mut book) = read_book(&dir) else {
        // Während des Kopierens gelöscht: keinen verwaisten Ordner zurücklassen.
        if !dir.join("buch.json").exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        return Err("Buch nicht gefunden.".into());
    };
    let previous = book.audio.take().unwrap_or_default();
    // Ein Text aus dem alten Hörbuch passt nicht zum neuen.
    if book.text.as_ref().is_some_and(|t| t.origin == "transcript") {
        book.text = None;
    }
    // Eine einzelne Datei ersetzt alle Teile; die Zeitachse passt nicht mehr.
    book.audio = Some(AudioVersion {
        file,
        narrator: previous.narrator,
        ..Default::default()
    });
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Mehrere Dateien als Teile **einer** Audiofassung importieren, in der
/// gegebenen Reihenfolge (`teil_001.<ext>` …). `append` hängt hinter die
/// vorhandenen Teile, sonst ersetzen die neuen alle alten.
#[tauri::command]
#[specta::specta]
pub async fn library_audio_import_tracks(
    app: AppHandle,
    id: String,
    paths: Vec<String>,
    append: bool,
) -> Result<LibraryBook, String> {
    if paths.is_empty() {
        return Err("Keine Dateien zum Importieren.".into());
    }
    let sources: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    for source in &sources {
        audio_ext(source)?;
    }
    let root = library_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || import_tracks_core(&root, &id, &sources, append))
        .await
        .map_err(|e| e.to_string())?
}

/// Bücher mit je einem Teil zu **einem** Hörbuch zusammenführen: die Teile der
/// Quellen wandern in der gegebenen Reihenfolge hinter die des Ziels, samt
/// Dauer und Transkriptions-Auftrag. Die Quellbücher verschwinden. Mit
/// `replace` ersetzen sie das vorhandene Hörbuch des Ziels (dessen Dateien
/// gehen erst nach erfolgreichem Schreiben weg, Fehler rollen alles zurück).
#[tauri::command]
#[specta::specta]
pub fn library_merge_audio(
    app: AppHandle,
    target_id: String,
    source_ids: Vec<String>,
    replace: bool,
) -> Result<LibraryBook, String> {
    let root = library_root(&app)?;
    merge_audio_in(&root, &target_id, &source_ids, replace)
}

/// Verschiebt eine Datei; über Laufwerksgrenzen kopieren und löschen. Klappt
/// das Löschen nach dem Kopieren nicht, verschwindet die Kopie wieder.
fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| format!("Verschieben fehlgeschlagen: {e}"))?;
    if let Err(e) = std::fs::remove_file(from) {
        let _ = std::fs::remove_file(to);
        return Err(format!("Verschieben fehlgeschlagen: {e}"));
    }
    Ok(())
}

fn merge_audio_in(
    root: &Path,
    target_id: &str,
    source_ids: &[String],
    replace: bool,
) -> Result<LibraryBook, String> {
    let ids: Vec<&str> = std::iter::once(target_id)
        .chain(source_ids.iter().map(String::as_str))
        .collect();
    let locks = book_locks(&ids);
    let _book_guards: Vec<_> = locks
        .iter()
        .map(|l| l.lock().unwrap_or_else(|e| e.into_inner()))
        .collect();
    let _guard = lock();
    let target_dir = book_dir(root, target_id)?;
    let mut target = read_book(&target_dir).ok_or("Zielbuch nicht gefunden.")?;
    if source_ids.is_empty() {
        return Err("Keine Quellbücher zum Zusammenführen.".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut sources = Vec::with_capacity(source_ids.len());
    for id in source_ids {
        let dir = book_dir(root, id)?;
        if id == target_id || !seen.insert(id.clone()) {
            return Err(format!("Buch doppelt oder gleich dem Ziel: {id}"));
        }
        let book = read_book(&dir).ok_or_else(|| format!("Quellbuch nicht gefunden: {id}"))?;
        if book.audio.is_none() {
            return Err(format!("\u{201e}{}\u{201c} hat kein Hörbuch.", book.title));
        }
        if book.text.is_some() {
            return Err(format!(
                "\u{201e}{}\u{201c} hat eine Textfassung und lässt sich nicht einfügen.",
                book.title
            ));
        }
        sources.push((dir, book));
    }

    // Ohne Audiofassung ist alles, was im Ordner liegt, Rest einer früheren.
    let replace = replace || target.audio.is_none();
    let target_audio = if replace { None } else { target.audio.take() };
    let previous_narrator = target.audio.take().and_then(|a| a.narrator);
    let target_audio_dir = target_dir.join("audio");
    std::fs::create_dir_all(&target_audio_dir).map_err(|e| e.to_string())?;
    let mut tracks = target_audio.as_ref().map(tracks_of).unwrap_or_default();
    let mut number = if replace {
        1
    } else {
        next_part_number(&target_audio_dir, &tracks)
    };
    let stage = target_audio_dir.join(format!(".neu-{}", now_ms() as i64));
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let rollback = |moved: &[(PathBuf, PathBuf)]| {
        for (from, to) in moved.iter().rev() {
            if let Err(e) = move_file(to, from) {
                log::warn!("Zurückrollen von {} fehlgeschlagen: {e}", from.display());
            }
        }
        let _ = std::fs::remove_dir_all(&stage);
    };
    let mut added: Vec<AudioTrack> = Vec::new();
    for (dir, book) in &sources {
        let audio = book.audio.as_ref().expect("oben geprüft");
        for track in tracks_of(audio) {
            let from = dir.join("audio").join(&track.file);
            if !from.is_file() {
                rollback(&moved);
                return Err(format!(
                    "Die Datei \u{201e}{}\u{201c} von \u{201e}{}\u{201c} fehlt.",
                    track.file, book.title
                ));
            }
            let ext = Path::new(&track.file)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp3")
                .to_lowercase();
            let file = part_name(number, &ext);
            number += 1;
            let to = stage.join(&file);
            if let Err(e) = move_file(&from, &to) {
                rollback(&moved);
                return Err(e);
            }
            moved.push((from, to));
            added.push(AudioTrack {
                file,
                // Altbestand kennt den Dateinamen nicht mehr: der Buchtitel
                // (aus dem Dateinamen entstanden) ist der Teiltitel.
                title: if audio.tracks.is_empty() {
                    book.title.clone()
                } else {
                    track.title
                },
                duration_ms: track.duration_ms,
                transcript_job: track.transcript_job,
                transcript_jobs: track.transcript_jobs,
            });
        }
    }
    let files: Vec<String> = added.iter().map(|t| t.file.clone()).collect();
    tracks.extend(added);
    let previous = target_audio.unwrap_or_default();
    target.audio = Some(AudioVersion {
        file: tracks.first().map(|t| t.file.clone()).unwrap_or_default(),
        duration_ms: None,
        narrator: previous.narrator.or(previous_narrator).or_else(|| {
            sources
                .iter()
                .find_map(|(_, b)| b.audio.as_ref().and_then(|a| a.narrator.clone()))
        }),
        transcript_job: None,
        sync: previous.sync,
        tracks,
    });
    // Ein Text aus Transkript gehört zum ersetzten Hörbuch und fällt weg.
    if replace
        && target
            .text
            .as_ref()
            .is_some_and(|t| t.origin == "transcript")
    {
        target.text = None;
    }
    let warnings = match commit_staged(&target_audio_dir, &stage, &files, replace, || {
        write_book(&target_dir, &target)
    }) {
        Ok(w) => w,
        Err(e) => {
            rollback(&moved);
            return Err(e);
        }
    };
    let _ = std::fs::remove_dir_all(&stage);
    for w in &warnings {
        log::warn!("{w}");
    }
    let mut leftovers = Vec::new();
    for (dir, book) in &sources {
        if let Err(e) = std::fs::remove_dir_all(dir) {
            leftovers.push(format!("\u{201e}{}\u{201c}: {e}", book.title));
        }
    }
    if !leftovers.is_empty() {
        return Err(format!(
            "Zusammengeführt, aber diese Quellbücher ließen sich nicht entfernen: {}",
            leftovers.join("; ")
        ));
    }
    read_book(&target_dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Gesamter gespeicherter Fortschritt (Schlüssel → JSON-Text).
#[tauri::command]
#[specta::specta]
pub fn library_progress_load(app: AppHandle) -> Result<BTreeMap<String, String>, String> {
    let root = library_root(&app)?;
    let _guard = lock();
    load_progress(&root)
}

/// Fortschritt sichern: Einträge einpflegen und die Datei atomar ersetzen.
#[tauri::command]
#[specta::specta]
pub fn library_progress_put(app: AppHandle, entries: Vec<ProgressEntry>) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    let _guard = lock();
    let root = library_root(&app)?;
    let mut map = load_progress(&root)?;
    merge_progress(&mut map, entries);
    let json = serde_json::to_vec(&map).map_err(|e| e.to_string())?;
    write_atomic(&progress_path(&root), &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(title: &str, authors: &[&str], isbn: Option<&str>) -> LibraryBook {
        LibraryBook {
            id: "book_1".into(),
            title: title.into(),
            authors: authors.iter().map(|a| a.to_string()).collect(),
            isbn: isbn.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn same_work_by_isbn_title_and_author() {
        let effi = book(
            "Effi Briest",
            &["Theodor Fontane"],
            Some("978-3-15-006961-6"),
        );
        assert!(same_work("Irgendwas", &[], Some("9783150069616"), &effi));
        assert!(same_work(
            "effi briest",
            &["Fontane, Theodor".into()],
            None,
            &effi
        ));
        assert!(same_work(
            "Effi Briest: Roman",
            &["T. Fontane".into()],
            None,
            &effi
        ));
        assert!(
            same_work("Effi Briest", &[], None, &effi),
            "ohne Autor zählt der Titel"
        );
        assert!(!same_work(
            "Effi Briest",
            &["Thomas Mann".into()],
            None,
            &effi
        ));
        assert!(!same_work(
            "Frau Jenny Treibel",
            &["Theodor Fontane".into()],
            None,
            &effi
        ));
    }

    #[test]
    fn titles_fold_umlauts_articles_and_subtitles() {
        let faust = book("Faust. Der Tragödie erster Teil", &["Goethe"], None);
        assert!(same_work(
            "FAUST. DER TRAGOEDIE ERSTER TEIL",
            &[],
            None,
            &faust
        ));
        let verw = book("Die Verwandlung", &["Franz Kafka"], None);
        assert!(same_work("Verwandlung", &["Kafka".into()], None, &verw));
        assert!(same_work(
            "Die Verwandlung – Erzählung",
            &["Franz Kafka".into()],
            None,
            &verw
        ));
        let dieb = book("Diebe im Haus", &[], None);
        assert!(
            !same_work("Be im Haus", &[], None, &dieb),
            "„Diebe“ ist kein Artikel"
        );
    }

    #[test]
    fn parses_search_results() {
        let body = r#"{"numFound":2,"docs":[
          {"key":"/works/OL123W","title":"Effi Briest","author_name":["Theodor Fontane"],
           "first_publish_year":1895,"publisher":["Reclam"],"isbn":["3150069618","9783150069616"],
           "cover_i":8231856,"language":["ger"],"number_of_pages_median":336},
          {"key":"/works/OL9W","title":"  ","cover_i":-1},
          {"key":"/books/OL1M","title":"Effi","cover_i":-1}]}"#;
        let hits = parse_search(body, 6).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].isbn.as_deref(), Some("9783150069616"));
        assert_eq!(hits[0].cover_id, Some(8231856));
        assert_eq!(hits[0].work_key.as_deref(), Some("/works/OL123W"));
        assert_eq!(hits[0].pages, Some(336));
        assert_eq!(hits[1].cover_id, None);
        assert_eq!(hits[1].work_key, None);
        assert!(parse_search("kein json", 6).is_err());
    }

    #[test]
    fn parses_descriptions() {
        assert_eq!(
            parse_description(r#"{"description":"Ein Roman."}"#).as_deref(),
            Some("Ein Roman.")
        );
        assert_eq!(
            parse_description(
                r#"{"description":{"type":"/type/text","value":"Text.\n----------\nSiehe auch"}}"#
            )
            .as_deref(),
            Some("Text.")
        );
        assert_eq!(parse_description(r#"{"title":"x"}"#), None);
    }

    #[test]
    fn finds_epub_cover_in_opf() {
        let epub3 = r#"<manifest><item id="c" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/></manifest>"#;
        assert_eq!(epub_cover_href(epub3).as_deref(), Some("images/cover.jpg"));
        let epub2 = r#"<metadata><meta name="cover" content="img1"/></metadata><manifest><item id="x" href="a.html"/><item id="img1" href="Cover%20Bild.png" media-type="image/png"/></manifest>"#;
        assert_eq!(epub_cover_href(epub2).as_deref(), Some("Cover%20Bild.png"));
        assert_eq!(percent_decode("Cover%20Bild.png"), "Cover Bild.png");
        assert_eq!(epub_cover_href("<manifest/>"), None);
    }

    #[test]
    fn covers_must_be_images() {
        let dir = std::env::temp_dir().join(format!("lv-lib-test-{}", std::process::id()));
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpg.resize(2048, 0);
        assert!(store_cover(&dir, &jpg).is_ok());
        assert!(dir.join("cover.jpg").is_file());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(2048, 0);
        assert!(store_cover(&dir, &png).is_ok());
        assert!(!dir.join("cover.jpg").exists(), "altes Cover ersetzt");
        assert!(store_cover(&dir, &[0u8; 2048]).is_err());
        assert!(
            store_cover(&dir, &[0xFF, 0xD8, 0xFF]).is_err(),
            "Platzhalter-Bild"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_foreign_ids() {
        assert!(checked_id("book_123").is_ok());
        assert!(checked_id("book_../x").is_err());
        assert!(checked_id("page_1").is_err());
        assert!(checked_id("book_").is_err());
    }

    #[test]
    fn progress_merge_inserts_replaces_and_deletes() {
        let mut map = BTreeMap::new();
        merge_progress(
            &mut map,
            vec![
                ProgressEntry {
                    key: "a".into(),
                    value: Some("1".into()),
                },
                ProgressEntry {
                    key: "b".into(),
                    value: Some("2".into()),
                },
                ProgressEntry {
                    key: String::new(),
                    value: Some("x".into()),
                },
                ProgressEntry {
                    key: "big".into(),
                    value: Some("x".repeat(MAX_PROGRESS_VALUE + 1)),
                },
            ],
        );
        merge_progress(
            &mut map,
            vec![
                ProgressEntry {
                    key: "a".into(),
                    value: Some("3".into()),
                },
                ProgressEntry {
                    key: "b".into(),
                    value: None,
                },
            ],
        );
        assert_eq!(map.len(), 1);
        assert_eq!(map["a"], "3");
    }

    #[test]
    fn books_roundtrip_without_display_paths() {
        let root = std::env::temp_dir().join(format!("lv-lib-rt-{}", std::process::id()));
        let dir = root.join("book_42");
        let mut b = book("Woyzeck", &["Georg Büchner"], None);
        b.id = "book_42".into();
        b.cover_path = Some("/x".into());
        b.favorite = true;
        b.audio = Some(AudioVersion {
            file: "hoerbuch.mp3".into(),
            ..Default::default()
        });
        write_book(&dir, &b).unwrap();
        let raw = std::fs::read_to_string(dir.join("buch.json")).unwrap();
        assert!(!raw.contains("cover_path\": \"/x"));
        let listed = list_books(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].cover_source, "generated");
        assert_eq!(listed[0].audio_path, None, "Hörbuchdatei fehlt");
        assert!(
            raw.contains("\"favorite\": true"),
            "Favorit steht in buch.json"
        );
        assert!(listed[0].favorite);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn broken_progress_file_is_kept_aside_not_overwritten() {
        let root = std::env::temp_dir().join(format!("lv-lib-pr-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            load_progress(&root).unwrap().is_empty(),
            "fehlende Datei = leer"
        );
        std::fs::write(progress_path(&root), "{ kaputt").unwrap();
        assert!(load_progress(&root).unwrap().is_empty());
        let aside: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("fortschritt.json.defekt-")
            })
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(
            std::fs::read_to_string(aside[0].path()).unwrap(),
            "{ kaputt"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn percent_decode_survives_multibyte_after_percent() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("x%é.jpg"), "x%é.jpg");
        assert_eq!(percent_decode("Cover%C3%A4.png"), "Coverä.png");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lv-lib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn audio_book(root: &Path, id: &str, title: &str, audio: AudioVersion) -> LibraryBook {
        let mut b = book(title, &["Robert Kiyosaki"], None);
        b.id = id.into();
        b.audio = Some(audio);
        write_book(&root.join(id), &b).unwrap();
        b
    }

    fn legacy(file: &str, job: Option<&str>, ms: Option<f64>) -> AudioVersion {
        AudioVersion {
            file: file.into(),
            duration_ms: ms,
            transcript_job: job.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn legacy_audio_is_one_track() {
        let old =
            r#"{"file":"hoerbuch.m4b","duration_ms":1000.0,"narrator":null,"transcript_job":"m7"}"#;
        let audio: AudioVersion = serde_json::from_str(old).unwrap();
        assert!(audio.tracks.is_empty() && audio.sync.is_empty());
        let tracks = tracks_of(&audio);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].file, "hoerbuch.m4b");
        assert_eq!(tracks[0].title, "hoerbuch");
        assert_eq!(tracks[0].duration_ms, Some(1000.0));
        assert_eq!(tracks[0].transcript_job.as_deref(), Some("m7"));
        assert!(tracks_of(&AudioVersion::default()).is_empty());
        let multi = AudioVersion {
            file: "teil_001.mp3".into(),
            tracks: vec![AudioTrack {
                file: "teil_001.mp3".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(tracks_of(&multi), multi.tracks);
    }

    #[test]
    fn old_book_json_without_new_fields_still_reads() {
        let root = scratch("compat");
        let dir = root.join("book_9");
        touch(
            &dir.join("buch.json"),
            r#"{"id":"book_9","title":"Alt","subtitle":null,"authors":[],"language":null,"isbn":null,"publisher":null,"publish_date":null,"pages":null,"description":null,"metadata_source":null,"cover_source":"generated","text":null,"audio":{"file":"hoerbuch.mp3","duration_ms":null,"narrator":null,"transcript_job":null},"added_ms":1.0,"finished":false,"cover_path":null,"audio_path":null}"#,
        );
        touch(&dir.join("audio/hoerbuch.mp3"), "x");
        let b = read_book(&dir).unwrap();
        assert!(!b.favorite, "ohne Feld kein Favorit");
        assert_eq!(b.audio_paths.len(), 1);
        assert_eq!(
            b.audio_path.as_deref(),
            b.audio_paths.first().map(String::as_str)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_track_without_section_jobs_still_reads_and_sections_round_trip() {
        // Altbestand (lva#204) kennt `transcript_jobs` nicht.
        let old: AudioTrack = serde_json::from_str(
            r#"{"file":"teil_001.mp3","title":"T","duration_ms":null,"transcript_job":"m1"}"#,
        )
        .unwrap();
        assert!(old.transcript_jobs.is_empty());
        let sectioned = AudioTrack {
            file: "teil_001.mp3".into(),
            transcript_jobs: vec![
                TranscriptPart {
                    job: "a".into(),
                    start_ms: 0.0,
                    end_ms: Some(1_202_000.0),
                    from_ms: 0.0,
                    to_ms: Some(1_200_000.0),
                },
                TranscriptPart {
                    job: "b".into(),
                    start_ms: 1_198_000.0,
                    end_ms: None,
                    from_ms: 1_200_000.0,
                    to_ms: None,
                },
            ],
            ..Default::default()
        };
        let json = serde_json::to_string(&sectioned).unwrap();
        assert_eq!(
            serde_json::from_str::<AudioTrack>(&json).unwrap(),
            sectioned
        );
    }

    #[test]
    fn audio_names_are_checked_for_every_track() {
        let mut audio = legacy("teil_001.mp3", None, None);
        audio.tracks = vec![
            AudioTrack {
                file: "teil_001.mp3".into(),
                ..Default::default()
            },
            AudioTrack {
                file: "../x.mp3".into(),
                ..Default::default()
            },
        ];
        assert!(validate_audio_names(&audio).is_err());
        audio.tracks[1].file = "C:evil.mp3".into();
        assert!(validate_audio_names(&audio).is_err());
        audio.tracks[1].file = ".versteckt".into();
        assert!(validate_audio_names(&audio).is_err());
        audio.tracks[1].file = "teil_002.mp3".into();
        assert!(validate_audio_names(&audio).is_ok());
        assert!(validate_audio_names(&legacy("a/b.mp3", None, None)).is_err());
    }

    #[test]
    fn audible_files_are_refused_with_a_clear_message() {
        for name in ["Buch.aax", "Buch.AA", "Buch.aaxc"] {
            let err = audio_ext(Path::new(name)).unwrap_err();
            assert!(err.contains("Audible-DRM"), "{err}");
            assert!(err.contains("DRM-freie Fassung (MP3, M4B)"), "{err}");
            assert!(err.contains(name), "{err}");
        }
        assert_eq!(audio_ext(Path::new("a/B.MP3")).unwrap(), "mp3");
        assert!(audio_ext(Path::new("a.pdf"))
            .unwrap_err()
            .contains("Kein Hörbuch-Format"));
    }

    #[test]
    fn imports_numbered_parts_replaces_and_appends() {
        let root = scratch("import");
        let src = root.join("quelle");
        for n in ["b - 02", "a - 01", "c - 03"] {
            touch(&src.join(format!("{n}.mp3")), n);
        }
        let mut b = book("Ziel", &["A B"], None);
        b.id = "book_1".into();
        write_book(&root.join("book_1"), &b).unwrap();
        let audio_dir = root.join("book_1/audio");
        touch(&audio_dir.join("hoerbuch.m4b"), "alt");
        let sources = vec![
            src.join("a - 01.mp3"),
            src.join("b - 02.mp3"),
            src.join("c - 03.mp3"),
        ];
        let done = import_tracks_core(&root, "book_1", &sources, false).unwrap();
        let tracks = done.audio.unwrap().tracks;
        let files: Vec<_> = tracks.iter().map(|t| t.file.as_str()).collect();
        assert_eq!(files, ["teil_001.mp3", "teil_002.mp3", "teil_003.mp3"]);
        assert_eq!(tracks[0].title, "a - 01");
        assert_eq!(
            std::fs::read_to_string(audio_dir.join("teil_002.mp3")).unwrap(),
            "b - 02"
        );
        assert!(!audio_dir.join("hoerbuch.m4b").exists(), "alte Datei weg");
        let leftovers: Vec<_> = std::fs::read_dir(&audio_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "Zwischenordner aufgeräumt");

        let more = import_tracks_core(&root, "book_1", &[src.join("a - 01.mp3")], true).unwrap();
        let files: Vec<_> = more
            .audio
            .unwrap()
            .tracks
            .iter()
            .map(|t| t.file.clone())
            .collect();
        assert_eq!(files.last().unwrap(), "teil_004.mp3");
        assert!(
            audio_dir.join("teil_001.mp3").is_file(),
            "Anhängen behält alles"
        );

        // Ersetzen: ein Fehler (fehlende Quelle, DRM) lässt Bestand und buch.json unberührt.
        assert!(
            import_tracks_core(&root, "book_1", &[src.join("gibt-es-nicht.mp3")], false).is_err()
        );
        assert!(audio_dir.join("teil_001.mp3").is_file());
        assert!(import_tracks_core(&root, "book_1", &[src.join("x.aax")], false).is_err());
        assert!(audio_dir.join("teil_004.mp3").is_file());
        assert_eq!(
            read_book(&root.join("book_1"))
                .unwrap()
                .audio
                .unwrap()
                .tracks
                .len(),
            4
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn replacing_keeps_old_files_when_writing_fails_and_drops_transcript_text() {
        let root = scratch("replace-fail");
        let dir = root.join("book_1/audio");
        touch(&dir.join("teil_001.mp3"), "alt1");
        touch(&dir.join("teil_002.mp3"), "alt2");
        let stage = dir.join(".neu-1");
        touch(&stage.join("teil_001.mp3"), "neu1");
        let err = commit_staged(&dir, &stage, &["teil_001.mp3".to_string()], true, || {
            Err("Schreiben kaputt".into())
        })
        .unwrap_err();
        assert!(err.contains("Schreiben kaputt"));
        assert_eq!(
            std::fs::read_to_string(dir.join("teil_001.mp3")).unwrap(),
            "alt1"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("teil_002.mp3")).unwrap(),
            "alt2"
        );
        assert_eq!(
            std::fs::read_to_string(stage.join("teil_001.mp3")).unwrap(),
            "neu1"
        );
        // Erfolg: Alte Dateien erst danach weg.
        let ok = commit_staged(&dir, &stage, &["teil_001.mp3".to_string()], true, || {
            assert!(dir.join("teil_001.mp3").is_file(), "Neue Datei liegt schon");
            Ok(())
        })
        .unwrap();
        assert!(ok.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.join("teil_001.mp3")).unwrap(),
            "neu1"
        );
        assert!(!dir.join("teil_002.mp3").exists());
        // Ohne Ersetzen wird nie überschrieben.
        touch(&stage.join("teil_001.mp3"), "dritte");
        assert!(commit_staged(
            &dir,
            &stage,
            &["teil_001.mp3".to_string()],
            false,
            || Ok(())
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("teil_001.mp3")).unwrap(),
            "neu1"
        );

        // Ersetzen verwirft einen Transkript-Text, behält aber einen Dokumenttext.
        let src = root.join("q");
        touch(&src.join("n.mp3"), "n");
        for (origin, kept) in [("transcript", false), ("document", true)] {
            let mut b = book("T", &["A B"], None);
            b.id = "book_9".into();
            b.text = Some(TextVersion {
                page_id: "p".into(),
                origin: origin.into(),
                source_name: None,
            });
            b.audio = Some(legacy("hoerbuch.mp3", None, None));
            write_book(&root.join("book_9"), &b).unwrap();
            touch(&root.join("book_9/audio/hoerbuch.mp3"), "x");
            let r = import_tracks_core(&root, "book_9", &[src.join("n.mp3")], false).unwrap();
            assert_eq!(r.text.is_some(), kept, "{origin}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_replace_swaps_target_audio_only_after_success() {
        let root = scratch("merge-replace");
        audio_book(
            &root,
            "book_1",
            "Ziel",
            legacy("hoerbuch.mp3", Some("m1"), None),
        );
        touch(&root.join("book_1/audio/hoerbuch.mp3"), "alt");
        audio_book(
            &root,
            "book_2",
            "Q1",
            legacy("hoerbuch.mp3", None, Some(5.0)),
        );
        touch(&root.join("book_2/audio/hoerbuch.mp3"), "q1");
        audio_book(&root, "book_3", "Q2", legacy("hoerbuch.mp3", None, None));
        // book_3: Datei fehlt -> Fehler, nichts verändert (auch Quelle 1 zurück).
        let err = merge_audio_in(&root, "book_1", &["book_2".into(), "book_3".into()], true);
        assert!(err.is_err());
        assert!(root.join("book_1/audio/hoerbuch.mp3").is_file());
        assert!(root.join("book_2/audio/hoerbuch.mp3").is_file());
        assert_eq!(
            tracks_of(&read_book(&root.join("book_1")).unwrap().audio.unwrap())[0].file,
            "hoerbuch.mp3"
        );
        touch(&root.join("book_3/audio/hoerbuch.mp3"), "q2");
        let merged =
            merge_audio_in(&root, "book_1", &["book_2".into(), "book_3".into()], true).unwrap();
        let audio = merged.audio.unwrap();
        let files: Vec<_> = audio.tracks.iter().map(|t| t.file.as_str()).collect();
        assert_eq!(files, ["teil_001.mp3", "teil_002.mp3"]);
        assert!(!root.join("book_1/audio/hoerbuch.mp3").exists());
        assert_eq!(
            audio.tracks[0].transcript_job, None,
            "Auftrag des alten Hörbuchs weg"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("book_1/audio/teil_001.mp3")).unwrap(),
            "q1"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn with_tracks_appends_after_a_legacy_file() {
        let previous = legacy("hoerbuch.mp3", Some("m1"), Some(5.0));
        let new = vec![AudioTrack {
            file: "teil_001.mp3".into(),
            ..Default::default()
        }];
        let appended = with_tracks(Some(previous.clone()), new.clone(), true);
        assert_eq!(appended.tracks.len(), 2);
        assert_eq!(appended.tracks[0].file, "hoerbuch.mp3");
        assert_eq!(appended.tracks[0].transcript_job.as_deref(), Some("m1"));
        assert_eq!(appended.file, "hoerbuch.mp3");
        let replaced = with_tracks(Some(previous), new, false);
        assert_eq!(replaced.tracks.len(), 1);
        assert_eq!(replaced.file, "teil_001.mp3");
        assert_eq!(replaced.transcript_job, None);
    }

    #[test]
    fn merge_moves_parts_in_order_and_removes_sources() {
        let root = scratch("merge");
        // Ziel: Buch mit einem Teil; Quellen: Altbestand (hoerbuch.mp3) und ein
        // Buch mit zwei Teilen und Auftrag.
        audio_book(
            &root,
            "book_1",
            "Rich Dad",
            legacy("hoerbuch.mp3", Some("m1"), Some(10.0)),
        );
        touch(&root.join("book_1/audio/hoerbuch.mp3"), "t1");
        audio_book(
            &root,
            "book_2",
            "Rich Dad Track 02",
            legacy("hoerbuch.mp3", Some("m2"), Some(20.0)),
        );
        touch(&root.join("book_2/audio/hoerbuch.mp3"), "t2");
        let multi = AudioVersion {
            file: "teil_001.mp3".into(),
            tracks: vec![
                AudioTrack {
                    file: "teil_001.mp3".into(),
                    title: "Track 03".into(),
                    duration_ms: Some(30.0),
                    transcript_job: Some("m3".into()),
                    ..Default::default()
                },
                AudioTrack {
                    file: "teil_002.m4a".into(),
                    title: "Track 04".into(),
                    duration_ms: None,
                    transcript_job: None,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        audio_book(&root, "book_3", "Rich Dad Track 03", multi);
        touch(&root.join("book_3/audio/teil_001.mp3"), "t3");
        touch(&root.join("book_3/audio/teil_002.m4a"), "t4");

        let merged = merge_audio_in(
            &root,
            "book_1",
            &["book_2".to_string(), "book_3".to_string()],
            false,
        )
        .unwrap();
        let audio = merged.audio.unwrap();
        let files: Vec<_> = audio.tracks.iter().map(|t| t.file.as_str()).collect();
        assert_eq!(
            files,
            [
                "hoerbuch.mp3",
                "teil_001.mp3",
                "teil_002.mp3",
                "teil_003.m4a"
            ]
        );
        assert_eq!(audio.file, "hoerbuch.mp3");
        let jobs: Vec<_> = audio
            .tracks
            .iter()
            .map(|t| t.transcript_job.as_deref())
            .collect();
        assert_eq!(jobs, [Some("m1"), Some("m2"), Some("m3"), None]);
        let durations: Vec<_> = audio.tracks.iter().map(|t| t.duration_ms).collect();
        assert_eq!(durations, [Some(10.0), Some(20.0), Some(30.0), None]);
        assert_eq!(
            audio.tracks[1].title, "Rich Dad Track 02",
            "Altbestand: Buchtitel"
        );
        assert_eq!(audio.tracks[3].title, "Track 04");
        assert_eq!(merged.audio_paths.len(), 4);
        assert!(merged.audio_paths.iter().all(|p| Path::new(p).is_file()));
        assert_eq!(
            std::fs::read_to_string(root.join("book_1/audio/teil_003.m4a")).unwrap(),
            "t4"
        );
        assert!(!root.join("book_2").exists() && !root.join("book_3").exists());

        // Fehlerfälle: Ziel als Quelle, doppelt, ohne Audio, fehlende Datei.
        audio_book(&root, "book_5", "X", legacy("hoerbuch.mp3", None, None));
        assert!(merge_audio_in(&root, "book_1", &["book_1".into()], false).is_err());
        assert!(
            merge_audio_in(&root, "book_1", &["book_5".into(), "book_5".into()], false).is_err()
        );
        assert!(merge_audio_in(&root, "book_1", &[], false).is_err());
        assert!(
            merge_audio_in(&root, "book_1", &["book_5".into()], false).is_err(),
            "Datei fehlt"
        );
        assert!(root.join("book_5").exists(), "Quelle bleibt bei Fehler");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_into_a_target_without_audio_clears_stale_files() {
        let root = scratch("merge-stale");
        let mut target = book("Ziel", &["A B"], None);
        target.id = "book_1".into();
        write_book(&root.join("book_1"), &target).unwrap();
        touch(&root.join("book_1/audio/hoerbuch.mp3"), "alt");
        audio_book(&root, "book_2", "Q", legacy("hoerbuch.mp3", None, None));
        touch(&root.join("book_2/audio/hoerbuch.mp3"), "neu");
        let merged = merge_audio_in(&root, "book_1", &["book_2".into()], false).unwrap();
        let files: Vec<_> = merged
            .audio
            .unwrap()
            .tracks
            .iter()
            .map(|t| t.file.clone())
            .collect();
        assert_eq!(files, ["teil_001.mp3"]);
        assert!(!root.join("book_1/audio/hoerbuch.mp3").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
