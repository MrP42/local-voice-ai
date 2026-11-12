//! lva#522: Loeschauftraege des Zuschneidens.
//!
//! Die alten WAVs (und ihre `mic_aec.wav`) loescht der Zuschnitt erst nach dem
//! Commit. Damit ein Absturz oder eine gesperrte Datei (Windows: Player,
//! Virenscanner) sie nicht fuer immer liegen laesst, schreibt dieselbe
//! Transaktion wie der Zuschnitt die Pfade als Auftraege in
//! `meetings.metadata_json["trim_cleanup"]` (ohne Migration). Danach kommt
//! sofort ein Loeschversuch ([`run_cleanup`]); was geloescht ist (oder nie
//! existierte oder ungueltig war), verlaesst die Liste in einer eigenen kleinen
//! Transaktion, der Rest bleibt, und der Start ([`sweep_leftovers`]) versucht es
//! wieder.
//!
//! Die Liste steht in der Datenbank, ist aber Eingabe von aussen (Datei, Sicherung,
//! Fremdprogramm): vor JEDER Loeschung (Sofortversuch wie Start) gilt
//!
//! - die Datei ist per `symlink_metadata` eine gewoehnliche Datei, kein Symlink,
//!   keine Junction, kein Reparse-Point (Windows: `FILE_ATTRIBUTE_REPARSE_POINT`);
//! - der Pfad liegt lexikalisch im Besprechungsordner, jeder Ordner dazwischen
//!   ist ein echtes Verzeichnis (kein Verweis), und der kanonische Elternpfad
//!   liegt im kanonischen Besprechungsordner;
//! - die Datei wird von der aktuellen Zeile nicht gebraucht (`mic`/`system`,
//!   deren `mic_aec.wav`, Dateien im aktuell benutzten `trim-`Ordner), verglichen
//!   nach Kanonisierung, unter Windows ohne Gross-/Kleinschreibung;
//! - unlesbare Metadaten oder ein unbekannter Besprechungsordner: nichts wird geloescht.
//!
//! Ungueltige Eintraege werden verworfen (Log ohne Inhalte, nur die Zahl).

use std::collections::HashSet;
use std::fs::Metadata;
use std::path::{Component, Path, PathBuf};

use log::{info, warn};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::Value;

use super::{is_trim_dir, meeting_folder};
use crate::managers::meetings::store::MeetingStore;

/// `metadata_json`-Schluessel der Loeschauftraege (Liste von Pfaden als Text).
pub(super) const CLEANUP_KEY: &str = "trim_cleanup";

/// Die Auftraege in `metadata_json`.
pub(super) fn jobs_of(root: &Value) -> Vec<String> {
    jobs_of_value(root.get(CLEANUP_KEY))
}

fn jobs_of_value(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Haengt `paths` an die Auftraege an (ohne Doppelte). `true`, wenn sich etwas aenderte.
pub(super) fn push_jobs(root: &mut Value, paths: &[String]) -> bool {
    let Some(map) = root.as_object_mut() else {
        return false;
    };
    let before = jobs_of_value(map.get(CLEANUP_KEY));
    let mut list = before.clone();
    for p in paths {
        if !list.contains(p) {
            list.push(p.clone());
        }
    }
    let changed = list != before;
    if changed {
        map.insert(
            CLEANUP_KEY.to_string(),
            Value::Array(list.into_iter().map(Value::String).collect()),
        );
    }
    changed
}

// ---------------------------------------------------------------------------
// Pruefungen
// ---------------------------------------------------------------------------

/// Verweis? Symlink, unter Windows auch Junction und jeder andere Reparse-Point.
pub(super) fn is_link(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Ein echtes Verzeichnis (kein Verweis).
pub(super) fn plain_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !is_link(&m))
}

/// Kanonischer Pfad; fehlt die Datei selbst, der kanonische Elternpfad mit ihrem Namen.
fn canonical(path: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(path) {
        return c;
    }
    match (
        path.parent().and_then(|p| std::fs::canonicalize(p).ok()),
        path.file_name(),
    ) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
}

/// Vergleichsschluessel: kanonisch, unter Windows ohne Gross-/Kleinschreibung.
pub(super) fn key(path: &Path) -> String {
    let text = canonical(path).to_string_lossy().to_string();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// Was die aktuelle Zeile der Besprechung braucht und nie geloescht werden darf.
#[derive(Default)]
struct Referenced {
    files: HashSet<String>,
    dirs: HashSet<String>,
}

impl Referenced {
    fn of(mic: Option<&str>, system: Option<&str>) -> Self {
        let mut me = Self::default();
        for path in [mic, system].into_iter().flatten() {
            me.files.insert(key(Path::new(path)));
            for derived in crate::managers::meetings::derived_audio_paths(path) {
                me.files.insert(key(Path::new(&derived)));
            }
            if let Some(dir) = Path::new(path).parent().filter(|d| is_trim_dir(d)) {
                me.dirs.insert(key(dir));
            }
        }
        me
    }
}

/// Was mit einem Auftrag geschieht.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Loeschen.
    Delete(PathBuf),
    /// Es gibt sie nicht (mehr): Auftrag erledigt.
    Gone,
    /// Ungueltig oder gefaehrlich: nichts loeschen, Auftrag verwerfen.
    Discard,
    /// Nicht entscheidbar (Lesefehler): Auftrag bleibt.
    Keep,
}

fn check_entry(folder: &Path, entry: &str, referenced: &Referenced) -> Verdict {
    let path = PathBuf::from(entry);
    let Ok(rel) = path.strip_prefix(folder) else {
        return Verdict::Discard;
    };
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            _ => return Verdict::Discard,
        }
    }
    let Some((_, dirs)) = parts.split_last() else {
        return Verdict::Discard;
    };
    // Jeder Ordner zwischen Besprechungsordner und Datei: ein echtes Verzeichnis.
    let mut current = folder.to_path_buf();
    for part in dirs {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !is_link(&meta) => {}
            Ok(_) => return Verdict::Discard,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Verdict::Gone,
            Err(_) => return Verdict::Keep,
        }
    }
    let Some(parent) = path.parent() else {
        return Verdict::Discard;
    };
    if !canonical(parent).starts_with(canonical(folder)) {
        return Verdict::Discard;
    }
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() && !is_link(&meta) => {}
        Ok(_) => return Verdict::Discard,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Verdict::Gone,
        Err(_) => return Verdict::Keep,
    }
    if referenced.files.contains(&key(&path)) || referenced.dirs.contains(&key(parent)) {
        return Verdict::Discard;
    }
    Verdict::Delete(path)
}

// ---------------------------------------------------------------------------
// Auftraege abarbeiten
// ---------------------------------------------------------------------------

type RemoveFn<'a> = &'a dyn Fn(&Path) -> std::io::Result<()>;

/// Arbeitet die Auftraege einer Besprechung ab. Gibt zurueck, wie viele bleiben.
pub fn run_cleanup(store: &MeetingStore, meeting_id: &str) -> u32 {
    run_cleanup_with(store, meeting_id, &|p| std::fs::remove_file(p))
}

/// Wie [`run_cleanup`] mit austauschbarer Loeschfunktion (Tests: gesperrte Datei).
pub(super) fn run_cleanup_with(store: &MeetingStore, meeting_id: &str, remove: RemoveFn) -> u32 {
    match cleanup_inner(store, meeting_id, remove) {
        Ok(remaining) => remaining,
        Err(e) => {
            warn!("meetings: trim cleanup of {meeting_id} failed: {e}");
            0
        }
    }
}

/// Mikrofonpfad, Systempfad, `metadata_json` einer Besprechung.
type AudioRow = (Option<String>, Option<String>, Option<String>);

fn read_row(store: &MeetingStore, meeting_id: &str) -> Result<Option<AudioRow>, String> {
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT mic_audio_path, system_audio_path, metadata_json FROM meetings WHERE id = ?1",
        params![meeting_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn cleanup_inner(store: &MeetingStore, meeting_id: &str, remove: RemoveFn) -> Result<u32, String> {
    let Some((mic, system, metadata)) = read_row(store, meeting_id)? else {
        return Ok(0);
    };
    // Unlesbare Metadaten: nichts loeschen.
    let Some(root) = metadata
        .as_deref()
        .and_then(|m| serde_json::from_str::<Value>(m).ok())
    else {
        return Ok(0);
    };
    let jobs = jobs_of(&root);
    if jobs.is_empty() {
        return Ok(0);
    }
    let count = u32::try_from(jobs.len()).unwrap_or(u32::MAX);
    // Ohne bekannten Besprechungsordner laesst sich nichts pruefen.
    let Some(folder) = mic
        .as_deref()
        .or(system.as_deref())
        .map(|p| meeting_folder(Path::new(p)))
    else {
        return Ok(count);
    };
    let referenced = Referenced::of(mic.as_deref(), system.as_deref());

    let mut done: Vec<String> = Vec::new();
    let mut discarded = 0u32;
    for entry in &jobs {
        match check_entry(&folder, entry, &referenced) {
            Verdict::Gone => done.push(entry.clone()),
            Verdict::Discard => {
                discarded += 1;
                done.push(entry.clone());
            }
            Verdict::Keep => {}
            Verdict::Delete(path) => match remove(&path) {
                Ok(()) => done.push(entry.clone()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => done.push(entry.clone()),
                Err(e) => warn!("meetings: old audio file not deleted ({e}); kept for retry"),
            },
        }
    }
    if discarded > 0 {
        warn!("meetings: {discarded} invalid trim cleanup entr(y/ies) discarded for {meeting_id}");
    }
    if done.is_empty() {
        return Ok(count);
    }
    let remaining = forget_jobs(store, meeting_id, &done)?;
    Ok(remaining)
}

/// Eigene kleine Transaktion: die erledigten Auftraege aus der Liste nehmen
/// (frisch gelesen, damit nichts verloren geht, was inzwischen dazukam).
fn forget_jobs(store: &MeetingStore, meeting_id: &str, done: &[String]) -> Result<u32, String> {
    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let metadata: Option<String> = tx
        .query_row(
            "SELECT metadata_json FROM meetings WHERE id = ?1",
            params![meeting_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let Some(mut root) = metadata
        .as_deref()
        .and_then(|m| serde_json::from_str::<Value>(m).ok())
    else {
        return Ok(0);
    };
    let left: Vec<String> = jobs_of(&root)
        .into_iter()
        .filter(|p| !done.contains(p))
        .collect();
    let remaining = u32::try_from(left.len()).unwrap_or(u32::MAX);
    if let Some(map) = root.as_object_mut() {
        if left.is_empty() {
            map.remove(CLEANUP_KEY);
        } else {
            map.insert(
                CLEANUP_KEY.to_string(),
                Value::Array(left.into_iter().map(Value::String).collect()),
            );
        }
    }
    tx.execute(
        "UPDATE meetings SET metadata_json = ?1 WHERE id = ?2",
        params![root.to_string(), meeting_id],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(remaining)
}

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

/// Was der Aufraeumlauf beim Start geschafft hat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Verwaiste `trim-*`-Ordner (von keiner Besprechung mehr genutzt), die weg sind.
    pub orphan_dirs_removed: u32,
    /// Auftraege und Ordner, die weiter bleiben.
    pub remaining: u32,
}

/// Beim Start: die Loeschauftraege aller Besprechungen aus der Datenbank abarbeiten
/// und verwaiste `trim-*`-Ordner entfernen (Abbruch zwischen Schneiden und Commit).
/// Ein Ordner gilt als verwaist, wenn er ein echtes Verzeichnis ist, weder
/// `mic_audio_path` noch `system_audio_path` einer Besprechung in ihm liegt und
/// er nur gewoehnliche Dateien enthaelt (ein Verweis oder ein Unterordner: stehen
/// lassen). Ist die Datenbank nicht lesbar, wird nichts als verwaist geloescht.
pub fn sweep_leftovers(store: &MeetingStore, meetings_dir: &Path) -> SweepReport {
    let mut report = SweepReport::default();
    for id in meetings_with_jobs(store) {
        report.remaining += run_cleanup(store, &id);
    }
    let Some(referenced) = referenced_audio_dirs(store) else {
        return report;
    };
    let Ok(entries) = std::fs::read_dir(meetings_dir) else {
        return report;
    };
    for folder in entries.flatten().map(|e| e.path()).filter(|p| plain_dir(p)) {
        let Ok(children) = std::fs::read_dir(&folder) else {
            continue;
        };
        for dir in children
            .flatten()
            .map(|e| e.path())
            .filter(|p| is_trim_dir(p) && plain_dir(p))
        {
            if referenced.contains(&key(&dir)) {
                continue;
            }
            match remove_orphan_dir(&dir) {
                Orphan::Removed => report.orphan_dirs_removed += 1,
                Orphan::Left => {}
                Orphan::Stuck => report.remaining += 1,
            }
        }
    }
    if report.orphan_dirs_removed > 0 {
        info!(
            "meetings: {} orphaned trim folder(s) removed",
            report.orphan_dirs_removed
        );
    }
    report
}

enum Orphan {
    Removed,
    /// Enthaelt Verweise oder Unterordner: unberuehrt.
    Left,
    /// Eine Datei liess sich nicht loeschen.
    Stuck,
}

fn remove_orphan_dir(dir: &Path) -> Orphan {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Orphan::Left;
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && !is_link(&meta) => files.push(path),
            _ => {
                warn!("meetings: orphaned trim folder left alone (contains more than plain files)");
                return Orphan::Left;
            }
        }
    }
    for file in files {
        if let Err(e) = std::fs::remove_file(&file) {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!("meetings: orphaned trim file not deleted: {e}");
                return Orphan::Stuck;
            }
        }
    }
    match std::fs::remove_dir(dir) {
        Ok(()) => Orphan::Removed,
        Err(e) => {
            warn!("meetings: orphaned trim folder not removed: {e}");
            Orphan::Stuck
        }
    }
}

/// Besprechungen, deren Metadaten Loeschauftraege tragen.
fn meetings_with_jobs(store: &MeetingStore) -> Vec<String> {
    let Ok(conn) = store.get_connection() else {
        return Vec::new();
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT id FROM meetings WHERE metadata_json LIKE '%\"trim_cleanup\"%'")
    else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

/// Die Ordner der Audiodateien aller Besprechungen (auch geloeschter), als
/// Vergleichsschluessel. `None`, wenn die Datenbank nicht lesbar ist.
fn referenced_audio_dirs(store: &MeetingStore) -> Option<HashSet<String>> {
    let conn = store.get_connection().ok()?;
    let mut stmt = conn
        .prepare("SELECT mic_audio_path, system_audio_path FROM meetings")
        .ok()?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        })
        .ok()?;
    let mut dirs = HashSet::new();
    for row in rows {
        let (mic, system) = row.ok()?;
        dirs.extend(
            [mic, system]
                .into_iter()
                .flatten()
                .filter_map(|p| Path::new(&p).parent().map(key)),
        );
    }
    Some(dirs)
}
