//! lva#522: Loeschauftraege des Zuschneidens.
//!
//! Die alten Dateien loescht der Zuschnitt erst nach dem Commit. Damit ein Absturz
//! oder eine gesperrte Datei (Windows: Player, Virenscanner) sie nicht fuer immer
//! liegen laesst, schreibt dieselbe Transaktion wie der Zuschnitt Auftraege in
//! `meetings.metadata_json["trim_cleanup"]` (ohne Migration). Danach kommt sofort ein
//! Loeschversuch ([`run_cleanup`]); was erledigt ist, verlaesst die Liste in einer
//! eigenen kleinen Transaktion, der Rest bleibt, und der Start ([`sweep_leftovers`])
//! versucht es wieder.
//!
//! Ein Auftrag ist KEIN Pfad, sondern ein Name im Besprechungsordner:
//!
//! - `{"kind":"root","name":"mic.wav"}`: eine Datei direkt im Besprechungsordner. Der
//!   Name ist ein einfacher `.wav`-Dateiname (`mic.wav`, `system.wav`, `mic_aec.wav`,
//!   `import.wav`, `<Name>-mic.wav` der Zusammenfuehrung), nie ein Pfad.
//! - `{"kind":"trim","dir":"trim-<ULID>"}`: ein Zuschnittsordner, streng `trim-` plus
//!   gueltige ULID. Er wird als Ganzes geloescht.
//!
//! Der Besprechungsordner ergibt sich aus Datenwurzel plus Besprechungs-ID
//! ([`meeting_dir`]), nie aus gespeicherten Pfaden. Vor JEDER Loeschung gilt: der
//! Ordner ist ein echtes Verzeichnis (kein Symlink, keine Junction, kein Reparse-Point,
//! per `symlink_metadata`), kanonisch ein direktes Kind der kanonischen Datenwurzel;
//! das Ziel ist selbst kein Verweis und kanonisch ein direktes Kind des Ordners; und
//! es gehoert nicht zur aktuellen Zeile (Referenzschutz: nie `mic`/`system`, deren
//! `mic_aec.wav` oder der Ordner, in dem sie liegen). Alles Ungueltige, auch ein altes
//! Auftragsformat (freier Pfad), wird verworfen, ohne etwas zu loeschen. Pruefung UND
//! Loeschung laufen unter dem Auftrag (Sperre) der Besprechung: der Aufrufer weist
//! das mit einem `&JobGuard` nach.

use std::collections::HashSet;
use std::fs::Metadata;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use log::{info, warn};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use ulid::Ulid;

use super::{is_trim_dir, TRIM_DIR_PREFIX};
use crate::managers::meetings::job::{self, JobGuard};
use crate::managers::meetings::store::MeetingStore;

/// `metadata_json`-Schluessel der Loeschauftraege.
pub(super) const CLEANUP_KEY: &str = "trim_cleanup";

// ---------------------------------------------------------------------------
// Auftraege
// ---------------------------------------------------------------------------

/// Ein Loeschauftrag (siehe Moduldoku).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Job {
    /// Datei direkt im Besprechungsordner.
    Root(String),
    /// Zuschnittsordner `trim-<ULID>` im Besprechungsordner.
    Trim(String),
}

impl Job {
    /// Datei im Besprechungsordner; `None`, wenn der Name nicht gueltig ist.
    pub(super) fn root(name: &str) -> Option<Self> {
        valid_root_name(name).then(|| Self::Root(name.to_string()))
    }

    /// Zuschnittsordner; `None`, wenn der Name nicht streng `trim-<ULID>` ist.
    pub(super) fn trim(dir: &str) -> Option<Self> {
        is_trim_name(dir).then(|| Self::Trim(dir.to_string()))
    }

    pub(super) fn to_value(&self) -> Value {
        match self {
            Self::Root(name) => json!({"kind": "root", "name": name}),
            Self::Trim(dir) => json!({"kind": "trim", "dir": dir}),
        }
    }

    /// Ein gespeicherter Eintrag; `None` bei jedem anderen Format (auch dem alten freien Pfad).
    pub(super) fn from_value(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let text = |k: &str| obj.get(k).and_then(Value::as_str);
        match text("kind")? {
            "root" => Self::root(text("name")?),
            "trim" => Self::trim(text("dir")?),
            _ => None,
        }
    }
}

/// `trim-` plus gueltige ULID (26 Zeichen, Crockford-Base32), gross oder klein.
pub(super) fn is_trim_name(name: &str) -> bool {
    name.strip_prefix(TRIM_DIR_PREFIX)
        .is_some_and(|ulid| ulid.len() == 26 && Ulid::from_string(ulid).is_ok())
}

/// Einfacher `.wav`-Dateiname: ASCII-Buchstaben, Ziffern, `.`, `_`, `-`; kein Pfad,
/// kein Stream (`:`), kein Geraetename (Windows: `NUL.wav`).
fn valid_root_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix(".wav").filter(|s| !s.is_empty()) else {
        return false;
    };
    let device = stem.split('.').next().unwrap_or("");
    let reserved = matches!(device, "con" | "prn" | "aux" | "nul")
        || ["com", "lpt"].iter().any(|p| {
            device
                .strip_prefix(p)
                .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
        });
    name.len() <= 128
        && !reserved
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Die rohen Eintraege in `metadata_json`.
fn raw_jobs(root: &Value) -> Vec<Value> {
    root.get(CLEANUP_KEY)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Haengt `jobs` an die Auftraege an (ohne Doppelte). `true`, wenn sich etwas aenderte.
pub(super) fn push_jobs(root: &mut Value, jobs: &[Job]) -> bool {
    let Some(map) = root.as_object_mut() else {
        return false;
    };
    let before = raw_jobs(&Value::Object(map.clone()));
    let mut list = before.clone();
    for job in jobs {
        let value = job.to_value();
        if !list.contains(&value) {
            list.push(value);
        }
    }
    if list == before {
        return false;
    }
    map.insert(CLEANUP_KEY.to_string(), Value::Array(list));
    true
}

/// Die Auftraege fuer alte Audiodateien einer Besprechung: Datei direkt im Ordner ->
/// `root`, Datei in einem `trim-<ULID>` direkt im Ordner -> `trim`. Gibt zusaetzlich
/// zurueck, wie viele der Hauptdateien (`paths`) so NICHT loeschbar sind (liegen
/// anderswo): die bleiben liegen.
pub(super) fn jobs_for_old(folder: &Path, paths: &[String]) -> (Vec<Job>, u32) {
    let mut jobs: Vec<Job> = Vec::new();
    let mut unreachable = 0u32;
    for path in paths {
        let derived = crate::managers::meetings::derived_audio_paths(path);
        for (i, candidate) in std::iter::once(path).chain(derived.iter()).enumerate() {
            match job_for(folder, candidate) {
                Some(job) if !jobs.contains(&job) => jobs.push(job),
                Some(_) => {}
                None if i == 0 => unreachable += 1,
                None => {}
            }
        }
    }
    (jobs, unreachable)
}

fn job_for(folder: &Path, path: &str) -> Option<Job> {
    let p = Path::new(path);
    let dir = p.parent()?;
    if key(dir) == key(folder) {
        return Job::root(p.file_name()?.to_str()?);
    }
    if key(dir.parent()?) == key(folder) {
        return Job::trim(dir.file_name()?.to_str()?);
    }
    None
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

/// Ist `child` kanonisch ein DIREKTES Kind von `parent`?
fn direct_child(child: &Path, parent: &Path) -> bool {
    match (std::fs::canonicalize(child), std::fs::canonicalize(parent)) {
        (Ok(c), Ok(p)) => c.parent() == Some(p.as_path()),
        _ => false,
    }
}

/// Der Besprechungsordner: Datenwurzel (Ordner der Datenbank) plus Besprechungs-ID,
/// nie aus gespeicherten Pfaden. `None` bei einer ID, die kein einzelner Name ist.
pub(super) fn meeting_dir(store: &MeetingStore, meeting_id: &str) -> Option<PathBuf> {
    let mut parts = Path::new(meeting_id).components();
    let single = matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(n)), None) if n == std::ffi::OsStr::new(meeting_id)
    );
    (single && !meeting_id.contains(':'))
        .then(|| store.db_path().parent().map(|r| r.join(meeting_id)))?
}

/// Zustand des Besprechungsordners.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Folder {
    /// Echtes Verzeichnis direkt in der Datenwurzel.
    Safe(PathBuf),
    /// Gibt es nicht (mehr).
    Missing,
    /// Verweis, kein Verzeichnis oder ausserhalb der Datenwurzel: nichts anfassen.
    Unsafe,
    /// Nicht lesbar: nicht entscheidbar.
    Unreadable,
}

pub(super) fn check_folder(store: &MeetingStore, meeting_id: &str) -> Folder {
    let (Some(folder), Some(root)) = (meeting_dir(store, meeting_id), store.db_path().parent())
    else {
        return Folder::Unsafe;
    };
    match std::fs::symlink_metadata(&folder) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => {}
        Ok(_) => return Folder::Unsafe,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Folder::Missing,
        Err(_) => return Folder::Unreadable,
    }
    if direct_child(&folder, root) {
        Folder::Safe(folder)
    } else {
        Folder::Unsafe
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
    Delete(PathBuf, Kind),
    /// Es gibt es nicht (mehr): Auftrag erledigt.
    Gone,
    /// Ungueltig, gefaehrlich oder in Gebrauch: nichts loeschen, Auftrag verwerfen.
    Discard,
    /// Nicht entscheidbar (Lesefehler): Auftrag bleibt.
    Keep,
}

/// Datei oder Ordner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    File,
    Dir,
}

fn check_job(folder: &Path, job: &Job, referenced: &Referenced) -> Verdict {
    let (name, kind) = match job {
        Job::Root(name) => (name, Kind::File),
        Job::Trim(dir) => (dir, Kind::Dir),
    };
    let path = folder.join(name);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Verdict::Gone,
        Err(_) => return Verdict::Keep,
    };
    let right_type = match kind {
        Kind::File => meta.is_file(),
        Kind::Dir => meta.is_dir(),
    };
    if !right_type || is_link(&meta) || !direct_child(&path, folder) {
        return Verdict::Discard;
    }
    let used = match kind {
        Kind::File => referenced.files.contains(&key(&path)),
        Kind::Dir => referenced.dirs.contains(&key(&path)),
    };
    if used {
        return Verdict::Discard;
    }
    Verdict::Delete(path, kind)
}

// ---------------------------------------------------------------------------
// Auftraege abarbeiten
// ---------------------------------------------------------------------------

/// Was ein Aufraeumlauf uebrig laesst.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cleanup {
    /// So viele Auftraege bleiben (Datei gesperrt, nicht lesbar), beim Fehler mindestens 1.
    pub remaining: u32,
    /// Der Lauf selbst scheiterte (Datenbank, Metadaten, Ordner nicht lesbar): ob alte
    /// Dateien bleiben, ist unbekannt und wird vorsichtig als Rest gemeldet.
    pub uncertain: bool,
}

impl Cleanup {
    fn failed() -> Self {
        Self {
            remaining: 1,
            uncertain: true,
        }
    }
}

type RemoveFn<'a> = &'a dyn Fn(&Path, Kind) -> std::io::Result<()>;

/// Loescht eine Datei oder einen ganzen Ordner.
fn remove_target(path: &Path, kind: Kind) -> std::io::Result<()> {
    match kind {
        Kind::File => std::fs::remove_file(path),
        // `remove_dir_all` der Standardbibliothek ist seit Rust 1.58.1 gegen den Wettlauf
        // CVE-2022-21658 gehaertet (Verweise im Ordner werden entfernt, nie verfolgt;
        // vorher konnte ein zwischen Pruefung und Loeschung ausgetauschter Symlink fremde
        // Dateien treffen). Deshalb nie eine eigene Rekursion.
        Kind::Dir => std::fs::remove_dir_all(path),
    }
}

/// Arbeitet die Auftraege einer Besprechung ab. Der Aufrufer haelt den Auftrag (Sperre)
/// der Besprechung: Pruefung und Loeschung laufen darunter.
pub fn run_cleanup(store: &MeetingStore, meeting_id: &str, lock: &JobGuard) -> Cleanup {
    run_cleanup_with(store, meeting_id, &remove_target, lock)
}

/// Wie [`run_cleanup`] mit austauschbarer Loeschfunktion (Tests: gesperrte Datei).
pub(super) fn run_cleanup_with(
    store: &MeetingStore,
    meeting_id: &str,
    remove: RemoveFn,
    _lock: &JobGuard,
) -> Cleanup {
    match cleanup_inner(store, meeting_id, remove) {
        Ok(done) => done,
        Err(e) => {
            warn!("meetings: trim cleanup of {meeting_id} failed: {e}");
            Cleanup::failed()
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

/// `metadata_json` als Objekt: kein Eintrag -> leer, kaputt oder kein Objekt -> Fehler.
fn parse_root(metadata: Option<&str>) -> Result<Value, String> {
    match metadata {
        None => Ok(Value::Object(serde_json::Map::new())),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(v) if v.is_object() => Ok(v),
            _ => Err("metadata_unreadable".to_string()),
        },
    }
}

fn cleanup_inner(
    store: &MeetingStore,
    meeting_id: &str,
    remove: RemoveFn,
) -> Result<Cleanup, String> {
    let Some((mic, system, metadata)) = read_row(store, meeting_id)? else {
        return Ok(Cleanup::default());
    };
    let root = parse_root(metadata.as_deref())?;
    let raw = raw_jobs(&root);
    if raw.is_empty() {
        return Ok(Cleanup::default());
    }
    let total = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    let folder = match check_folder(store, meeting_id) {
        Folder::Safe(folder) => folder,
        // Es gibt den Ordner nicht mehr: nichts zu loeschen, die Auftraege sind erledigt.
        Folder::Missing => {
            forget_jobs(store, meeting_id, &raw)?;
            return Ok(Cleanup::default());
        }
        Folder::Unsafe => {
            warn!("meetings: folder of {meeting_id} is not a plain data folder; nothing deleted");
            forget_jobs(store, meeting_id, &raw)?;
            return Ok(Cleanup::default());
        }
        Folder::Unreadable => {
            return Ok(Cleanup {
                remaining: total,
                uncertain: true,
            })
        }
    };
    let referenced = Referenced::of(mic.as_deref(), system.as_deref());

    let mut done: Vec<Value> = Vec::new();
    let mut discarded = 0u32;
    for entry in &raw {
        let Some(job) = Job::from_value(entry) else {
            discarded += 1;
            done.push(entry.clone());
            continue;
        };
        match check_job(&folder, &job, &referenced) {
            Verdict::Gone => done.push(entry.clone()),
            Verdict::Discard => {
                discarded += 1;
                done.push(entry.clone());
            }
            Verdict::Keep => {}
            Verdict::Delete(path, kind) => match remove(&path, kind) {
                Ok(()) => done.push(entry.clone()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => done.push(entry.clone()),
                Err(e) => warn!("meetings: old audio not deleted ({e}); kept for retry"),
            },
        }
    }
    if discarded > 0 {
        warn!("meetings: {discarded} invalid trim cleanup entr(y/ies) discarded for {meeting_id}");
    }
    if done.is_empty() {
        return Ok(Cleanup {
            remaining: total,
            uncertain: false,
        });
    }
    let remaining = forget_jobs(store, meeting_id, &done)?;
    Ok(Cleanup {
        remaining,
        uncertain: false,
    })
}

/// Eigene kleine Transaktion: die erledigten Auftraege aus der Liste nehmen
/// (frisch gelesen, damit nichts verloren geht, was inzwischen dazukam).
fn forget_jobs(store: &MeetingStore, meeting_id: &str, done: &[Value]) -> Result<u32, String> {
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
    let mut root = parse_root(metadata.as_deref())?;
    let left: Vec<Value> = raw_jobs(&root)
        .into_iter()
        .filter(|entry| !done.contains(entry))
        .collect();
    let remaining = u32::try_from(left.len()).unwrap_or(u32::MAX);
    if let Some(map) = root.as_object_mut() {
        if left.is_empty() {
            map.remove(CLEANUP_KEY);
        } else {
            map.insert(CLEANUP_KEY.to_string(), Value::Array(left));
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
    /// Verwaiste `trim-<ULID>`-Ordner (von keiner Zeile mehr benutzt), die weg sind.
    pub orphan_dirs_removed: u32,
    /// Auftraege und Ordner, die weiter bleiben.
    pub remaining: u32,
}

/// Beim Start: je Besprechung (unter ihrem Auftrag; laeuft schon einer, bleibt sie
/// unberuehrt, ihre Auftraege zaehlen als Rest) die Loeschauftraege abarbeiten und
/// verwaiste `trim-<ULID>`-Ordner entfernen (Abbruch zwischen Schneiden und Commit).
/// Verwaist heisst: ein echtes Verzeichnis mit streng gueltigem Namen direkt im
/// Besprechungsordner, in dem weder `mic_audio_path` noch `system_audio_path` der
/// Zeile liegt. Es wird ganz entfernt. Ist die Zeile oder der Ordner nicht sicher
/// lesbar, wird nichts als verwaist geloescht.
pub fn sweep_leftovers(store: &MeetingStore) -> SweepReport {
    let mut report = SweepReport::default();
    for (id, has_jobs) in meetings_to_tidy(store) {
        let Some(folder) = meeting_dir(store, &id) else {
            continue;
        };
        if !has_jobs && orphan_candidates(&folder).is_empty() {
            continue;
        }
        let Ok(guard) = job::global().try_start(&id, Arc::new(|_| {})) else {
            // Es laeuft etwas (zum Beispiel ein Zuschnitt, dessen neuer Ordner noch
            // keiner Zeile gehoert): nichts anfassen.
            report.remaining += u32::from(has_jobs);
            continue;
        };
        if has_jobs {
            report.remaining += run_cleanup(store, &id, &guard).remaining;
        }
        let (removed, stuck) = sweep_orphans(store, &id, &guard);
        report.orphan_dirs_removed += removed;
        report.remaining += stuck;
    }
    if report.orphan_dirs_removed > 0 {
        info!(
            "meetings: {} orphaned trim folder(s) removed",
            report.orphan_dirs_removed
        );
    }
    report
}

/// Zuschnittsordner im Besprechungsordner mit streng gueltigem Namen (nur echte Verzeichnisse).
fn orphan_candidates(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(is_trim_name))
        .map(|e| e.path())
        .filter(|p| plain_dir(p))
        .collect()
}

/// Verwaiste Zuschnittsordner einer Besprechung entfernen (unter ihrem Auftrag).
/// Gibt zurueck: entfernt, haengengeblieben.
fn sweep_orphans(store: &MeetingStore, meeting_id: &str, _lock: &JobGuard) -> (u32, u32) {
    let Ok(Some((mic, system, _))) = read_row(store, meeting_id) else {
        return (0, 0);
    };
    let Folder::Safe(folder) = check_folder(store, meeting_id) else {
        return (0, 0);
    };
    let referenced = Referenced::of(mic.as_deref(), system.as_deref());
    let (mut removed, mut stuck) = (0, 0);
    for dir in orphan_candidates(&folder) {
        if !direct_child(&dir, &folder) || referenced.dirs.contains(&key(&dir)) {
            continue;
        }
        match remove_target(&dir, Kind::Dir) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => removed += 1,
            Err(e) => {
                warn!("meetings: orphaned trim folder not removed: {e}");
                stuck += 1;
            }
        }
    }
    (removed, stuck)
}

/// Alle Besprechungen (auch geloeschte: ihr Ordner kann noch liegen) und ob ihre
/// Metadaten Loeschauftraege tragen.
fn meetings_to_tidy(store: &MeetingStore) -> Vec<(String, bool)> {
    let Ok(conn) = store.get_connection() else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn
        .prepare("SELECT id, COALESCE(metadata_json LIKE '%\"trim_cleanup\"%', 0) FROM meetings")
    else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))
    }) else {
        return Vec::new();
    };
    rows.flatten().collect()
}
