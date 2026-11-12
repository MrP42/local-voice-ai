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
//! Ein Auftrag ist KEIN Pfad, sondern ein Name im Besprechungsordner plus Herkunft:
//!
//! - `{"kind":"root","name":"mic.wav","origin":{..}}`: eine Datei direkt im
//!   Besprechungsordner. Der Name ist ein einfacher `.wav`-Dateiname (`mic.wav`,
//!   `system.wav`, `mic_aec.wav`, `import.wav`, `<Name>-mic.wav` der Zusammenfuehrung),
//!   nie ein Pfad.
//! - `{"kind":"trim","dir":"trim-<ULID>","origin":{..}}`: ein Zuschnittsordner, streng
//!   `trim-` plus gueltige ULID. Er wird als Ganzes geloescht.
//! - Herkunft: `{"from":"trim","trim_id":"<ULID>","channel":"mic|system|mic_aec"}`. Ein
//!   Auftrag entsteht nur in `apply_trim`, aus den alten Pfaden DERSELBEN Zeile, und
//!   nennt den Zuschnitt (die ULID seines neuen Ordners) und die Spur, die er ersetzt
//!   hat. Ein Eintrag ohne gueltige Herkunft (auch der alte Auftrag mit freiem Pfad
//!   oder nur Name) wird verworfen, ohne etwas zu loeschen. Ein Zuschnittsordner nennt
//!   nie seinen eigenen Zuschnitt als Herkunft.
//!
//! Der Besprechungsordner ergibt sich aus Datenwurzel plus Besprechungs-ID
//! ([`open_folder`]), nie aus gespeicherten Pfaden. Geloescht wird nur RELATIV zu
//! Verzeichnis-Handles (`cap-std`): die Datenwurzel wird einmal geoeffnet, der
//! Besprechungsordner relativ dazu (kein Verweis, per `symlink_metadata` am Handle
//! geprueft, und das geoeffnete Verzeichnis ist dasselbe wie das gepruefte); Datei oder
//! Ordner darin werden mit `remove_file` bzw. `remove_dir_all` AM HANDLE entfernt. Wird
//! der Ordner oder ein Vorfahre nach der Pruefung gegen einen Verweis ausgetauscht, trifft
//! die Loeschung weiter das gepruefte Verzeichnis, nie das Ziel des Verweises. Das Ziel
//! ist selbst kein Verweis (am Handle geprueft).
//!
//! Referenzschutz ueber ALLE Besprechungen: vor jeder Loeschung werden alle Zeilen von
//! `meetings` (auch weich geloeschte, solange Dateipfade gesetzt sind) gelesen. Was eine
//! Zeile braucht (ihre `mic`/`system`-Datei, deren `mic_aec.wav`, bei einem Ordner jede
//! Datei darin; normalisiert, unter Windows ohne Gross-/Kleinschreibung), wird nicht
//! geloescht. Lesen und Loeschen laufen in EINER `IMMEDIATE`-Transaktion: niemand schreibt
//! dazwischen einen neuen Pfad. Zusaetzlich haelt der Aufrufer den Auftrag (Sperre) der
//! Besprechung und weist das mit einem `&JobGuard` nach.
//!
//! Alte Dateien, die keinen Auftrag bekommen koennen (ausserhalb des Besprechungsordners
//! oder mit nicht zulaessigem Namen), werden NIE geloescht. Der Zuschnitt haelt sie in
//! derselben Transaktion dauerhaft in `metadata_json["trim_kept"]` fest
//! (`[{"path","reason"}]`).

use std::collections::HashSet;
use std::fs::Metadata;
use std::path::{Component, Path, PathBuf, MAIN_SEPARATOR};
use std::sync::Arc;

use cap_std::ambient_authority;
use cap_std::fs::{Dir, Metadata as CapMetadata};
use log::{info, warn};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Map, Value};
use ulid::Ulid;

use super::TRIM_DIR_PREFIX;
use crate::managers::meetings::job::{self, JobGuard};
use crate::managers::meetings::store::MeetingStore;

/// `metadata_json`-Schluessel der Loeschauftraege.
pub(super) const CLEANUP_KEY: &str = "trim_cleanup";
/// `metadata_json`-Schluessel der alten Dateien, die der Zuschnitt nie loescht.
pub(super) const KEPT_KEY: &str = "trim_kept";
/// Grund: die alte Datei liegt nicht direkt im Besprechungsordner (oder in einem
/// `trim-<ULID>` darin).
pub(super) const REASON_OUTSIDE: &str = "outside_folder";
/// Grund: die alte Datei liegt im Ordner, ihr Name ist aber nicht zulaessig.
pub(super) const REASON_NAME: &str = "invalid_name";

// ---------------------------------------------------------------------------
// Auftraege
// ---------------------------------------------------------------------------

/// Spur, die ein Zuschnitt ersetzt hat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Channel {
    Mic,
    System,
    /// Die entechote Kopie `mic_aec.wav` neben der `mic.wav`.
    MicAec,
}

impl Channel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::System => "system",
            Self::MicAec => "mic_aec",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "mic" => Some(Self::Mic),
            "system" => Some(Self::System),
            "mic_aec" => Some(Self::MicAec),
            _ => None,
        }
    }
}

/// ULID des neuen Ordners eines Zuschnitts (klein geschrieben, 26 Zeichen).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TrimId(String);

impl TrimId {
    /// Gueltige ULID (26 Zeichen, Crockford-Base32), gross oder klein.
    pub(super) fn parse(text: &str) -> Option<Self> {
        (text.len() == 26 && Ulid::from_string(text).is_ok())
            .then(|| Self(text.to_ascii_lowercase()))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }

    /// Name des Ordners `trim-<ULID>`.
    pub(super) fn dir_name(&self) -> String {
        format!("{TRIM_DIR_PREFIX}{}", self.0)
    }
}

/// Herkunft eines Auftrags: der Zuschnitt und die Spur, die er ersetzt hat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Origin {
    pub trim_id: TrimId,
    pub channel: Channel,
}

impl Origin {
    fn to_value(&self) -> Value {
        json!({
            "from": "trim",
            "trim_id": self.trim_id.as_str(),
            "channel": self.channel.as_str(),
        })
    }

    fn from_value(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let text = |k: &str| obj.get(k).and_then(Value::as_str);
        if text("from")? != "trim" {
            return None;
        }
        Some(Self {
            trim_id: TrimId::parse(text("trim_id")?)?,
            channel: Channel::parse(text("channel")?)?,
        })
    }
}

/// Datei oder Ordner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    File,
    Dir,
}

/// Ein Loeschauftrag (siehe Moduldoku).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Job {
    /// `File`: Datei direkt im Besprechungsordner; `Dir`: Zuschnittsordner darin.
    pub kind: Kind,
    /// Einfacher Name im Besprechungsordner.
    pub name: String,
    pub origin: Origin,
}

impl Job {
    /// Datei im Besprechungsordner; `None`, wenn der Name nicht gueltig ist.
    pub(super) fn root(name: &str, origin: Origin) -> Option<Self> {
        valid_root_name(name).then(|| Self {
            kind: Kind::File,
            name: name.to_string(),
            origin,
        })
    }

    /// Zuschnittsordner; `None`, wenn der Name nicht streng `trim-<ULID>` ist oder der
    /// Ordner der eigene neue Ordner des Zuschnitts waere.
    pub(super) fn trim(dir: &str, origin: Origin) -> Option<Self> {
        let own = origin.trim_id.dir_name();
        (is_trim_name(dir) && !dir.eq_ignore_ascii_case(&own)).then(|| Self {
            kind: Kind::Dir,
            name: dir.to_string(),
            origin,
        })
    }

    pub(super) fn to_value(&self) -> Value {
        let origin = self.origin.to_value();
        match self.kind {
            Kind::File => json!({"kind": "root", "name": self.name, "origin": origin}),
            Kind::Dir => json!({"kind": "trim", "dir": self.name, "origin": origin}),
        }
    }

    /// Ein gespeicherter Eintrag; `None` bei jedem anderen Format (auch dem alten freien
    /// Pfad) und ohne gueltige Herkunft.
    pub(super) fn from_value(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let text = |k: &str| obj.get(k).and_then(Value::as_str);
        let origin = Origin::from_value(obj.get("origin")?)?;
        match text("kind")? {
            "root" => Self::root(text("name")?, origin),
            "trim" => Self::trim(text("dir")?, origin),
            _ => None,
        }
    }

    fn same_target(&self, other: &Self) -> bool {
        self.kind == other.kind && self.name == other.name
    }
}

/// `trim-` plus gueltige ULID (26 Zeichen, Crockford-Base32), gross oder klein.
pub(super) fn is_trim_name(name: &str) -> bool {
    name.strip_prefix(TRIM_DIR_PREFIX)
        .is_some_and(|ulid| TrimId::parse(ulid).is_some())
}

/// Einfacher `.wav`-Dateiname: ASCII-Buchstaben, Ziffern, `.`, `_`, `-`; kein Pfad,
/// kein Stream (`:`), kein Geraetename (Windows: `NUL.wav`), und keiner, der wie ein
/// Zuschnittsordner beginnt (`trim-...`, auch `trim-<ULID>.wav`): so bleibt ein Name im
/// Besprechungsordner nie zweideutig.
///
/// Die Namen, die die App selbst vergibt, erfuellen das alle: `mic.wav`, `system.wav`,
/// `mic_aec.wav` (Aufnahme), `import.wav` (Import: der Originalname der Datei wird nur
/// Titel, nie Dateiname), `teile-<ulid>-mic.wav` und `teile-<ulid>-system.wav`
/// (Zusammenfuehrung). Leerzeichen und Nicht-ASCII kommen darin nie vor; solche Namen
/// stammen von aussen und gehen den Weg `trim_kept` (nie loeschen).
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
        && !lower.starts_with(TRIM_DIR_PREFIX)
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

/// Eine alte Datei, die der Zuschnitt nie loescht (siehe Moduldoku).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Kept {
    pub path: String,
    pub reason: &'static str,
}

/// Haengt `kept` an `metadata_json["trim_kept"]` an (ohne Doppelte nach Pfad).
/// `true`, wenn sich etwas aenderte.
pub(super) fn push_kept(root: &mut Value, kept: &[Kept]) -> bool {
    let Some(map) = root.as_object_mut() else {
        return false;
    };
    let mut list = map
        .get(KEPT_KEY)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let before = list.len();
    for entry in kept {
        let known = list
            .iter()
            .any(|v| v.get("path").and_then(Value::as_str) == Some(entry.path.as_str()));
        if !known {
            list.push(json!({"path": entry.path, "reason": entry.reason}));
        }
    }
    if list.len() == before {
        return false;
    }
    map.insert(KEPT_KEY.to_string(), Value::Array(list));
    true
}

/// Die Auftraege fuer die alten Audiodateien einer Besprechung (`files`: Spur und Pfad
/// laut Zeile): Datei direkt im Ordner -> `root`, Datei in einem `trim-<ULID>` direkt
/// im Ordner -> `trim`; die abgeleitete `mic_aec.wav` zaehlt als eigene Spur. Alles
/// andere (anderer Ort, nicht zulaessiger Name) bekommt keinen Auftrag und kommt als
/// [`Kept`] zurueck: es bleibt liegen und wird festgehalten. Abgeleitete Dateien, die
/// es gar nicht gibt, bleiben ungenannt.
pub(super) fn jobs_for_old(
    folder: &Path,
    trim_id: &TrimId,
    files: &[(Channel, &str)],
) -> (Vec<Job>, Vec<Kept>) {
    let mut jobs: Vec<Job> = Vec::new();
    let mut kept: Vec<Kept> = Vec::new();
    for (channel, path) in files {
        let derived = crate::managers::meetings::derived_audio_paths(path);
        let candidates = std::iter::once((*channel, (*path).to_string()))
            .chain(derived.into_iter().map(|d| (Channel::MicAec, d)));
        for (i, (channel, candidate)) in candidates.enumerate() {
            match job_for(folder, &candidate, trim_id, channel) {
                Ok(job) if !jobs.iter().any(|j| j.same_target(&job)) => jobs.push(job),
                Ok(_) => {}
                Err(reason) => {
                    let exists = i == 0 || std::fs::symlink_metadata(&candidate).is_ok();
                    if exists && !kept.iter().any(|k| k.path == candidate) {
                        kept.push(Kept {
                            path: candidate,
                            reason,
                        });
                    }
                }
            }
        }
    }
    (jobs, kept)
}

fn job_for(
    folder: &Path,
    path: &str,
    trim_id: &TrimId,
    channel: Channel,
) -> Result<Job, &'static str> {
    let origin = Origin {
        trim_id: trim_id.clone(),
        channel,
    };
    let p = Path::new(path);
    let dir = p.parent().ok_or(REASON_OUTSIDE)?;
    let name = p.file_name().and_then(|n| n.to_str()).ok_or(REASON_NAME)?;
    if key(dir) == key(folder) {
        return Job::root(name, origin).ok_or(REASON_NAME);
    }
    let in_folder = dir
        .parent()
        .is_some_and(|parent| key(parent) == key(folder));
    let trim_dir = dir
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| is_trim_name(n));
    match trim_dir {
        Some(trim_dir) if in_folder => Job::trim(trim_dir, origin).ok_or(REASON_OUTSIDE),
        _ => Err(REASON_OUTSIDE),
    }
}

// ---------------------------------------------------------------------------
// Pfadvergleiche
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

/// Wie [`is_link`], fuer die Metadaten am Verzeichnis-Handle.
fn is_link_cap(meta: &CapMetadata) -> bool {
    if meta.is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if cap_std::fs::MetadataExt::file_attributes(meta) & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Ist das geoeffnete Verzeichnis dasselbe wie das vorher gepruefte (Geraet und Inode)?
/// Unter Windows liefert `cap-std` keine stabile Kennung: dort schuetzt allein, dass
/// `cap-std` Verweisen aus dem Baum der Datenwurzel heraus nie folgt.
#[cfg(unix)]
fn same_dir(seen: &CapMetadata, opened: &CapMetadata) -> bool {
    use cap_std::fs::MetadataExt;
    seen.dev() == opened.dev() && seen.ino() == opened.ino()
}

#[cfg(not(unix))]
fn same_dir(_seen: &CapMetadata, _opened: &CapMetadata) -> bool {
    true
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

/// Windows vergleicht Namen ohne Gross-/Kleinschreibung.
pub(super) fn fold(text: String, case_insensitive: bool) -> String {
    if case_insensitive {
        text.to_lowercase()
    } else {
        text
    }
}

/// Vergleichsschluessel: kanonisch (Verweise aufgeloest), unter Windows klein.
fn key(path: &Path) -> String {
    fold(canonical(path).to_string_lossy().to_string(), cfg!(windows))
}

/// Vergleichsschluessel ohne Dateizugriff (nur der Text), unter Windows klein.
fn norm(path: &Path) -> String {
    fold(path.to_string_lossy().to_string(), cfg!(windows))
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

// ---------------------------------------------------------------------------
// Besprechungsordner (Handle)
// ---------------------------------------------------------------------------

/// Der geoeffnete Besprechungsordner: ein echtes Verzeichnis direkt in der Datenwurzel.
pub(super) struct Folder {
    /// Pfad (Datenwurzel plus ID), zum Anlegen neuer Dateien.
    pub path: PathBuf,
    /// Kanonischer Pfad der Datenwurzel plus ID, zum Vergleichen mit den Pfaden der
    /// Zeilen; einmal beim Oeffnen berechnet, spaetere Austausche aendern ihn nicht.
    canon: PathBuf,
    /// Das Verzeichnis selbst: alles wird relativ dazu geloescht.
    pub dir: Dir,
}

/// Warum sich der Besprechungsordner nicht oeffnen laesst.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Unusable {
    /// Gibt es nicht (mehr).
    Missing,
    /// Verweis, kein Verzeichnis oder ausserhalb der Datenwurzel: nichts anfassen.
    Unsafe,
    /// Nicht lesbar (oder zwischen Pruefung und Oeffnen veraendert): nicht entscheidbar.
    Unreadable,
}

fn classify(e: &std::io::Error) -> Unusable {
    if e.kind() == std::io::ErrorKind::NotFound {
        Unusable::Missing
    } else {
        Unusable::Unreadable
    }
}

/// Oeffnet den Besprechungsordner relativ zur Datenwurzel (siehe Moduldoku).
pub(super) fn open_folder(store: &MeetingStore, meeting_id: &str) -> Result<Folder, Unusable> {
    let path = meeting_dir(store, meeting_id).ok_or(Unusable::Unsafe)?;
    let data_root = store.db_path().parent().ok_or(Unusable::Unsafe)?;
    // Ueber den Pfad: Verweise jeder Art (unter Windows auch Reparse-Points) abweisen.
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => {}
        Ok(_) => return Err(Unusable::Unsafe),
        Err(e) => return Err(classify(&e)),
    }
    let canon_root = std::fs::canonicalize(data_root).map_err(|e| classify(&e))?;
    let root = Dir::open_ambient_dir(data_root, ambient_authority()).map_err(|e| classify(&e))?;
    let name = Path::new(meeting_id);
    // Und am Handle der Datenwurzel: der Besprechungsordner ist kein Verweis.
    let seen = match root.symlink_metadata(name) {
        Ok(meta) if meta.is_dir() && !is_link_cap(&meta) => meta,
        Ok(_) => return Err(Unusable::Unsafe),
        Err(e) => return Err(classify(&e)),
    };
    let dir = root.open_dir(name).map_err(|e| classify(&e))?;
    let opened = dir.dir_metadata().map_err(|e| classify(&e))?;
    if !opened.is_dir() || !same_dir(&seen, &opened) {
        return Err(Unusable::Unreadable);
    }
    Ok(Folder {
        path,
        canon: canon_root.join(meeting_id),
        dir,
    })
}

// ---------------------------------------------------------------------------
// Referenzen aller Besprechungen
// ---------------------------------------------------------------------------

/// Was irgendeine Zeile von `meetings` braucht und nie geloescht werden darf.
#[derive(Default)]
struct Referenced {
    /// Schluessel jeder Datei, die eine Zeile nennt (auch `mic_aec.wav` daneben).
    files: HashSet<String>,
}

impl Referenced {
    /// Alle Zeilen, auch weich geloeschte, solange Dateipfade gesetzt sind.
    fn scan(conn: &Connection) -> Result<Self, String> {
        let mut stmt = conn
            .prepare(
                "SELECT mic_audio_path, system_audio_path FROM meetings
                 WHERE mic_audio_path IS NOT NULL OR system_audio_path IS NOT NULL",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut me = Self::default();
        for row in rows {
            let (mic, system) = row.map_err(|e| e.to_string())?;
            for path in [mic, system].into_iter().flatten() {
                me.add(&path);
            }
        }
        Ok(me)
    }

    /// Eine Datei samt der abgeleiteten; kanonisch UND als roher Text, damit auch ein
    /// Pfad auffaellt, der sich nicht aufloesen laesst.
    fn add(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        let derived = crate::managers::meetings::derived_audio_paths(path);
        for p in std::iter::once(path.to_string()).chain(derived) {
            let p = Path::new(&p);
            self.files.insert(key(p));
            self.files.insert(norm(p));
        }
    }

    /// Braucht eine Zeile das Ziel (`target`: kanonischer Besprechungsordner plus Name)?
    /// Bei einem Ordner: irgendeine Datei darin.
    fn protects(&self, target: &Path, kind: Kind) -> bool {
        let key = norm(target);
        match kind {
            Kind::File => self.files.contains(&key),
            Kind::Dir => {
                let prefix = format!("{key}{MAIN_SEPARATOR}");
                self.files.iter().any(|f| f.starts_with(&prefix))
            }
        }
    }
}

/// Was mit einem Auftrag geschieht.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Loeschen.
    Delete,
    /// Es gibt es nicht (mehr): Auftrag erledigt.
    Gone,
    /// Ungueltig, gefaehrlich oder in Gebrauch: nichts loeschen, Auftrag verwerfen.
    Discard,
    /// Nicht entscheidbar (Lesefehler): Auftrag bleibt.
    Keep,
}

fn check_job(folder: &Folder, job: &Job, referenced: &Referenced) -> Verdict {
    let meta = match folder.dir.symlink_metadata(&job.name) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Verdict::Gone,
        Err(_) => return Verdict::Keep,
    };
    let right_type = match job.kind {
        Kind::File => meta.is_file(),
        Kind::Dir => meta.is_dir(),
    };
    if !right_type || is_link_cap(&meta) {
        return Verdict::Discard;
    }
    if referenced.protects(&folder.canon.join(&job.name), job.kind) {
        return Verdict::Discard;
    }
    Verdict::Delete
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

/// Loescht Datei oder Ordner `name` im Besprechungsordner (Tests: gesperrte Datei,
/// Austausch des Ordners vor der Loeschung).
type RemoveFn<'a> = &'a dyn Fn(&Dir, &str, Kind) -> std::io::Result<()>;

/// Loescht eine Datei oder einen ganzen Ordner RELATIV zum Handle. `cap-std` folgt keinem
/// Verweis aus dem Baum heraus; ein Verweis als Ziel oder darin wird selbst entfernt,
/// nie sein Ziel.
pub(super) fn remove_target(dir: &Dir, name: &str, kind: Kind) -> std::io::Result<()> {
    match kind {
        Kind::File => dir.remove_file(name),
        Kind::Dir => dir.remove_dir_all(name),
    }
}

/// Arbeitet die Auftraege einer Besprechung ab. Der Aufrufer haelt den Auftrag (Sperre)
/// der Besprechung: Pruefung und Loeschung laufen darunter.
pub fn run_cleanup(store: &MeetingStore, meeting_id: &str, lock: &JobGuard) -> Cleanup {
    run_cleanup_with(store, meeting_id, &remove_target, lock)
}

/// Wie [`run_cleanup`] mit austauschbarer Loeschfunktion.
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

/// `metadata_json` einer Besprechung: keine Zeile -> `None`, sonst der Text (auch `NULL`).
fn read_metadata(store: &MeetingStore, meeting_id: &str) -> Result<Option<Option<String>>, String> {
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT metadata_json FROM meetings WHERE id = ?1",
        params![meeting_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// `metadata_json` als Objekt: kein Eintrag -> leer, kaputt oder kein Objekt -> Fehler.
fn parse_root(metadata: Option<&str>) -> Result<Value, String> {
    match metadata {
        None => Ok(Value::Object(Map::new())),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(v) if v.is_object() => Ok(v),
            _ => Err("metadata_unreadable".to_string()),
        },
    }
}

/// Was aus einem Auftrag wurde.
enum Settled {
    /// Erledigt: geloescht oder gab es nicht mehr.
    Done,
    /// Verworfen (ungueltig, Verweis, in Gebrauch): nichts geloescht, Auftrag weg.
    Discarded,
    /// Bleibt (gesperrt, nicht lesbar).
    Keep,
}

/// Ein Auftrag unter EINER `IMMEDIATE`-Transaktion: alle Zeilen lesen, pruefen, loeschen.
/// Waehrenddessen kann keine andere Besprechung einen Pfad in diese Datei schreiben.
fn settle(
    conn: &mut Connection,
    folder: &Folder,
    job: &Job,
    remove: RemoveFn,
) -> Result<Settled, String> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let referenced = Referenced::scan(&tx)?;
    Ok(match check_job(folder, job, &referenced) {
        Verdict::Gone => Settled::Done,
        Verdict::Discard => Settled::Discarded,
        Verdict::Keep => Settled::Keep,
        Verdict::Delete => match remove(&folder.dir, &job.name, job.kind) {
            Ok(()) => Settled::Done,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settled::Done,
            Err(e) => {
                warn!("meetings: old audio not deleted ({e}); kept for retry");
                Settled::Keep
            }
        },
    })
}

fn cleanup_inner(
    store: &MeetingStore,
    meeting_id: &str,
    remove: RemoveFn,
) -> Result<Cleanup, String> {
    let Some(metadata) = read_metadata(store, meeting_id)? else {
        return Ok(Cleanup::default());
    };
    let root = parse_root(metadata.as_deref())?;
    let raw = raw_jobs(&root);
    if raw.is_empty() {
        return Ok(Cleanup::default());
    }
    let total = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    let folder = match open_folder(store, meeting_id) {
        Ok(folder) => folder,
        // Es gibt den Ordner nicht mehr: nichts zu loeschen, die Auftraege sind erledigt.
        Err(Unusable::Missing) => {
            forget_jobs(store, meeting_id, &raw)?;
            return Ok(Cleanup::default());
        }
        Err(Unusable::Unsafe) => {
            warn!("meetings: folder of {meeting_id} is not a plain data folder; nothing deleted");
            forget_jobs(store, meeting_id, &raw)?;
            return Ok(Cleanup::default());
        }
        Err(Unusable::Unreadable) => {
            return Ok(Cleanup {
                remaining: total,
                uncertain: true,
            })
        }
    };

    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    let mut done: Vec<Value> = Vec::new();
    let mut discarded = 0u32;
    for entry in &raw {
        let Some(job) = Job::from_value(entry) else {
            discarded += 1;
            done.push(entry.clone());
            continue;
        };
        match settle(&mut conn, &folder, &job, remove)? {
            Settled::Done => done.push(entry.clone()),
            Settled::Discarded => {
                discarded += 1;
                done.push(entry.clone());
            }
            Settled::Keep => {}
        }
    }
    drop(conn);
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
/// Besprechungsordner, in dem KEINE Zeile irgendeiner Besprechung (auch weich
/// geloeschte) eine Datei nennt. Es wird ganz entfernt. Ist der Ordner oder die
/// Datenbank nicht sicher lesbar, wird nichts als verwaist geloescht.
pub fn sweep_leftovers(store: &MeetingStore) -> SweepReport {
    let mut report = SweepReport::default();
    for (id, has_jobs) in meetings_to_tidy(store) {
        if meeting_dir(store, &id).is_none() {
            continue;
        }
        let has_orphans = open_folder(store, &id)
            .map(|folder| !orphan_candidates(&folder).is_empty())
            .unwrap_or(false);
        if !has_jobs && !has_orphans {
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

/// Namen der Zuschnittsordner im Besprechungsordner mit streng gueltigem Namen
/// (nur echte Verzeichnisse, am Handle geprueft).
fn orphan_candidates(folder: &Folder) -> Vec<String> {
    let Ok(entries) = folder.dir.entries() else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| is_trim_name(name))
        .filter(|name| {
            folder
                .dir
                .symlink_metadata(name)
                .is_ok_and(|m| m.is_dir() && !is_link_cap(&m))
        })
        .collect()
}

/// Verwaiste Zuschnittsordner einer Besprechung entfernen (unter ihrem Auftrag).
/// Gibt zurueck: entfernt, haengengeblieben.
fn sweep_orphans(store: &MeetingStore, meeting_id: &str, _lock: &JobGuard) -> (u32, u32) {
    let Ok(folder) = open_folder(store, meeting_id) else {
        return (0, 0);
    };
    let candidates = orphan_candidates(&folder);
    let all = u32::try_from(candidates.len()).unwrap_or(u32::MAX);
    if all == 0 {
        return (0, 0);
    }
    let Ok(mut conn) = store.get_connection() else {
        return (0, all);
    };
    let (mut removed, mut stuck) = (0, 0);
    for name in candidates {
        match sweep_orphan(&mut conn, &folder, &name) {
            Ok(true) => removed += 1,
            Ok(false) => {}
            Err(e) => {
                warn!("meetings: orphaned trim folder not removed: {e}");
                stuck += 1;
            }
        }
    }
    (removed, stuck)
}

/// Ein Zuschnittsordner unter EINER `IMMEDIATE`-Transaktion: alle Zeilen lesen, nur
/// loeschen, wenn keine eine Datei darin nennt. `true`, wenn er weg ist.
fn sweep_orphan(conn: &mut Connection, folder: &Folder, name: &str) -> Result<bool, String> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let referenced = Referenced::scan(&tx)?;
    if referenced.protects(&folder.canon.join(name), Kind::Dir) {
        return Ok(false);
    }
    match folder.dir.symlink_metadata(name) {
        Ok(meta) if meta.is_dir() && !is_link_cap(&meta) => {}
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    }
    match remove_target(&folder.dir, name, Kind::Dir) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e.to_string()),
    }
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
