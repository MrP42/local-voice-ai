//! lva#522: der Ordner des Zuschnitts und die Vermerke zu den alten Dateien.
//!
//! Der Zuschnitt LOESCHT NICHTS: weder die alten Audiodateien noch, wenn etwas schiefgeht,
//! die eigenen neuen. In diesem Modul und im ganzen Zuschnitt gibt es keine einzige
//! Dateisystem-Loeschung (kein `remove_*`, kein ersetzendes `rename`, nur `create_new`).
//!
//! - Die alten Audiodateien (`mic.wav`, `system.wav`, `mic_aec.wav` oder die Dateien eines
//!   frueheren Ordners `trim-<ULID>`) bleiben unangetastet als Sicherung liegen. Dieselbe
//!   Transaktion wie der Zuschnitt vermerkt sie dauerhaft in
//!   `meetings.metadata_json["trim_superseded"]` ([`Superseded`]: Pfad, der ersetzende
//!   Zuschnitt, Spur, Zeitpunkt), damit ein spaeteres Aufraeumen (#536) weiss, was ersetzt
//!   wurde.
//! - Scheitert der Zuschnitt vor dem Commit, bleibt ein unfertiger Ordner `trim-<ULID>`
//!   liegen (nur Speicher, nie referenziert; Aufraeumen ebenfalls #536). Die Datenbank
//!   bleibt unveraendert.
//!
//! Die NEUEN Dateien entstehen relativ zu Verzeichnis-Handles (`cap-std`), und vor dem
//! Commit wird bewiesen, dass die gespeicherten Pfade genau diese Dateien benennen:
//!
//! - Der Besprechungsordner ergibt sich aus Datenwurzel plus Besprechungs-ID
//!   ([`open_folder`]), nie aus gespeicherten Pfaden. `cap-std` folgt Verweisen, die in der
//!   Datenwurzel bleiben; deshalb wird er VOR und NACH dem Oeffnen als Nicht-Verweis
//!   geprueft und das geoeffnete Handle gegen den Pfad verglichen ([`names_file`], Geraet
//!   und Inode bzw. Dateiindex). Ein Tausch zwischen Pruefung und Oeffnen faellt so auf.
//! - Der Ordner `trim-<ULID>` wird ueber dieses Handle angelegt und ebenso nach dem Oeffnen
//!   gegen seinen Pfad verglichen ([`NewDir::create`]); frisch angelegt, muss er leer sein.
//! - Jede Datei darin entsteht mit `create_new` ueber das Handle des Zuschnittsordners
//!   (nie ueberschreiben); der Zuschnitt haelt ihr `File`-Handle bis zum Commit.
//! - Innerhalb der `IMMEDIATE`-Transaktion, unmittelbar vor dem `UPDATE`, wird fuer jede neue
//!   Datei geprueft, dass der zu speichernde Pfad dieselbe Datei ist wie das gehaltene Handle
//!   (`MeetingStore::apply_trim`). Weicht etwas ab, gibt es `trim_changed`, keinen Commit und
//!   keine Loeschung. Uebrig bleibt nur das Zeitfenster zwischen dieser Pruefung und dem
//!   Commit (Mikrosekunden, ohne Schaden: nichts wird geloescht).

use std::path::{Component, Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, Metadata, OpenOptions};
use log::warn;
use serde_json::{json, Value};
use ulid::Ulid;

use super::TRIM_DIR_PREFIX;
use crate::managers::meetings::store::MeetingStore;

/// `metadata_json`-Schluessel der ersetzten Audiodateien.
pub(super) const SUPERSEDED_KEY: &str = "trim_superseded";
/// So viele Vermerke haelt `trim_superseded` hoechstens; darueber fallen die aeltesten weg
/// (die Dateien selbst bleiben liegen, nur der Vermerk entfaellt).
pub(super) const SUPERSEDED_MAX: usize = 256;

// ---------------------------------------------------------------------------
// Test-Haken
// ---------------------------------------------------------------------------

#[cfg(test)]
type HookFn = Box<dyn Fn(&str)>;

#[cfg(test)]
thread_local! {
    static HOOK: std::cell::RefCell<Option<HookFn>> = const { std::cell::RefCell::new(None) };
}

/// Haengt in Tests eine Funktion an die benannten Stellen des Ablaufs
/// (`meeting_folder_checked`, `trim_folder_created`, `written`), etwa um dort einen Ordner
/// auszutauschen. Gilt nur fuer den aktuellen Thread und nur, solange der Rueckgabewert
/// lebt (er nimmt den Haken beim Verwerfen wieder ab, auch wenn Tests auf einem Thread laufen).
#[cfg(test)]
#[must_use = "the hook is removed again when the guard is dropped"]
pub(super) fn set_hook(f: impl Fn(&str) + 'static) -> HookGuard {
    HOOK.with(|h| *h.borrow_mut() = Some(Box::new(f)));
    HookGuard
}

#[cfg(test)]
pub(super) struct HookGuard;

#[cfg(test)]
impl Drop for HookGuard {
    fn drop(&mut self) {
        HOOK.with(|h| *h.borrow_mut() = None);
    }
}

/// Eine benannte Stelle des Ablaufs; ausserhalb von Tests ohne Wirkung.
pub(super) fn hook(point: &str) {
    #[cfg(test)]
    HOOK.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f(point);
        }
    });
    #[cfg(not(test))]
    let _ = point;
}

// ---------------------------------------------------------------------------
// Vermerke
// ---------------------------------------------------------------------------

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

/// Spur, die ein Zuschnitt ersetzt hat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Channel {
    Mic,
    System,
    /// Die entechote Kopie `mic_aec.wav` neben der `mic.wav`.
    MicAec,
}

impl Channel {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::System => "system",
            Self::MicAec => "mic_aec",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Self::Mic, Self::System, Self::MicAec]
            .into_iter()
            .find(|c| c.as_str() == text)
    }
}

/// Eine alte Audiodatei, die ein Zuschnitt ersetzt hat und die unangetastet liegen bleibt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Superseded {
    /// Pfad, wie ihn die Zeile vor dem Zuschnitt hatte (bei `mic_aec` der abgeleitete).
    pub path: String,
    /// Der Zuschnitt (ULID seines neuen Ordners), der sie ersetzt hat.
    pub trim_id: TrimId,
    pub channel: Channel,
    /// Unix-Sekunden des Zuschnitts.
    pub at: i64,
}

impl Superseded {
    fn to_value(&self) -> Value {
        json!({
            "path": self.path,
            "trim_id": self.trim_id.as_str(),
            "channel": self.channel.as_str(),
            "at": self.at,
        })
    }
}

/// Ein gueltiger Vermerk: Objekt mit nicht leerem `path` (Text), gueltiger `trim_id`
/// (ULID), `channel` aus {mic, system, mic_aec} und `at` (Zahl).
fn valid_note(value: &Value) -> bool {
    let text = |key: &str| value.get(key).and_then(Value::as_str);
    value.is_object()
        && text("path").is_some_and(|p| !p.is_empty())
        && text("trim_id").is_some_and(|id| TrimId::parse(id).is_some())
        && text("channel").is_some_and(|c| Channel::parse(c).is_some())
        && value.get("at").is_some_and(Value::is_number)
}

/// Ist `trim_superseded` lesbar: fehlt, `null` oder eine Liste, deren Elemente alle
/// gueltige Vermerke sind? Alles andere sind beschaedigte Metadaten: der Zuschnitt darf sie
/// nicht ueberschreiben (`metadata_unreadable`).
pub(super) fn superseded_readable(root: &Value) -> bool {
    match root.get(SUPERSEDED_KEY) {
        None | Some(Value::Null) => true,
        Some(Value::Array(list)) => list.iter().all(valid_note),
        Some(_) => false,
    }
}

/// Der Schluessel eines Vermerks zum Deduplizieren: Pfad und Spur.
fn note_key(value: &Value) -> Option<(&str, &str)> {
    Some((
        value.get("path")?.as_str()?,
        value.get("channel")?.as_str()?,
    ))
}

/// Haengt `entries` an `metadata_json["trim_superseded"]` an und bringt die RESULTIERENDE
/// Liste in Form: ohne Doppelte nach Pfad und Spur (der erste bleibt), hoechstens
/// [`SUPERSEDED_MAX`] Eintraege (die aeltesten fallen weg). Das gilt auch dann, wenn nichts
/// Neues hinzukommt. Eine nicht lesbare Liste ([`superseded_readable`]) bleibt unangetastet.
/// `true`, wenn sich etwas aenderte.
pub(super) fn push_superseded(root: &mut Value, entries: &[Superseded]) -> bool {
    // Nie etwas Beschaedigtes umschreiben: der Zuschnitt lehnt das vorher ab.
    if !superseded_readable(root) {
        return false;
    }
    let Some(map) = root.as_object_mut() else {
        return false;
    };
    let before: Vec<Value> = match map.get(SUPERSEDED_KEY) {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    let mut list: Vec<Value> = Vec::with_capacity(before.len() + entries.len());
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let added = entries.iter().map(Superseded::to_value);
    for value in before.iter().cloned().chain(added) {
        let Some((path, channel)) = note_key(&value) else {
            continue;
        };
        if seen.insert((path.to_string(), channel.to_string())) {
            list.push(value);
        }
    }
    if list.len() > SUPERSEDED_MAX {
        let excess = list.len() - SUPERSEDED_MAX;
        list.drain(..excess);
        warn!("meetings: trim_superseded is full, {excess} oldest note(s) dropped");
    }
    if list == before {
        return false;
    }
    map.insert(SUPERSEDED_KEY.to_string(), Value::Array(list));
    true
}

// ---------------------------------------------------------------------------
// Identitaet
// ---------------------------------------------------------------------------

/// Ist `path` (jetzt) genau die Datei oder der Ordner, den `file` haelt (Geraet und Inode,
/// unter Windows Volume und Dateiindex)? Verweise werden aufgeloest; wer sie ausschliessen
/// will, prueft vorher separat. Jeder Fehler ist ein Nein.
pub(super) fn names_file(path: &Path, file: &std::fs::File) -> bool {
    let (Ok(clone), Ok(named)) = (file.try_clone(), same_file::Handle::from_path(path)) else {
        return false;
    };
    same_file::Handle::from_file(clone).is_ok_and(|held| held == named)
}

/// Wie [`names_file`], fuer ein Verzeichnis-Handle.
pub(super) fn names_dir(path: &Path, dir: &Dir) -> bool {
    dir.try_clone()
        .is_ok_and(|clone| names_file(path, &clone.into_std_file()))
}

// ---------------------------------------------------------------------------
// Besprechungsordner (Handle)
// ---------------------------------------------------------------------------

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

/// Verweis? Symlink, unter Windows auch Junction und jeder andere Reparse-Point.
fn is_link(meta: &Metadata) -> bool {
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

/// Der geoeffnete Besprechungsordner: ein echtes Verzeichnis direkt in der Datenwurzel.
pub(super) struct Folder {
    /// Pfad (Datenwurzel plus ID), zum Benennen der neuen Dateien in der Datenbank.
    pub path: PathBuf,
    /// Das Verzeichnis selbst: alles entsteht relativ dazu.
    pub dir: Dir,
}

/// Warum sich der Besprechungsordner nicht oeffnen laesst.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Unusable {
    /// Gibt es nicht (mehr).
    Missing,
    /// Verweis, kein Verzeichnis oder keine einfache ID: nichts anlegen.
    Unsafe,
    /// Nicht lesbar: nicht entscheidbar.
    Unreadable,
    /// Zwischen Pruefung und Oeffnen ausgetauscht: das Handle ist nicht der Ordner unter
    /// dem Pfad.
    Changed,
}

fn classify(e: &std::io::Error) -> Unusable {
    if e.kind() == std::io::ErrorKind::NotFound {
        Unusable::Missing
    } else {
        Unusable::Unreadable
    }
}

/// `name` direkt in `parent` ist ein Verzeichnis und kein Verweis.
fn require_plain_dir(parent: &Dir, name: &Path) -> Result<(), Unusable> {
    match parent.symlink_metadata(name) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => Ok(()),
        Ok(_) => Err(Unusable::Unsafe),
        Err(e) => Err(classify(&e)),
    }
}

/// Oeffnet den Besprechungsordner relativ zur Datenwurzel (siehe Moduldoku): vor dem
/// Oeffnen als Nicht-Verweis geprueft, danach noch einmal, und das Handle muss derselbe
/// Ordner sein wie der unter dem Pfad.
pub(super) fn open_folder(store: &MeetingStore, meeting_id: &str) -> Result<Folder, Unusable> {
    let path = meeting_dir(store, meeting_id).ok_or(Unusable::Unsafe)?;
    let data_root = store.db_path().parent().ok_or(Unusable::Unsafe)?;
    let root = Dir::open_ambient_dir(data_root, ambient_authority()).map_err(|e| classify(&e))?;
    let name = Path::new(meeting_id);
    require_plain_dir(&root, name)?;
    hook("meeting_folder_checked");
    let dir = root.open_dir(name).map_err(|e| classify(&e))?;
    require_plain_dir(&root, name).map_err(|_| Unusable::Changed)?;
    if !names_dir(&path, &dir) {
        return Err(Unusable::Changed);
    }
    Ok(Folder { path, dir })
}

// ---------------------------------------------------------------------------
// Neuer Zuschnittsordner (Handle)
// ---------------------------------------------------------------------------

/// Der frisch angelegte Ordner `trim-<ULID>`, als Handle festgehalten.
pub(super) struct NewDir {
    dir: Dir,
}

impl NewDir {
    /// Legt `name` im Besprechungsordner `home` an und haelt ihn als Handle fest. Der
    /// Ordner muss vorher gefehlt haben (`create_dir` scheitert sonst), nach dem Oeffnen
    /// kein Verweis und derselbe Ordner wie der unter seinem Pfad sein (sonst `trim_changed`)
    /// und leer. Scheitert etwas davon, bleibt der Ordner liegen: es wird nichts geloescht.
    pub(super) fn create(home: &Folder, name: &str) -> Result<Self, String> {
        let failed = |e: &dyn std::fmt::Display| format!("wav_write_failed: {e}");
        home.dir.create_dir(name).map_err(|e| failed(&e))?;
        hook("trim_folder_created");
        let dir = home.dir.open_dir(name).map_err(|e| failed(&e))?;
        let plain = require_plain_dir(&home.dir, Path::new(name)).is_ok();
        if !plain || !names_dir(&home.path.join(name), &dir) {
            return Err("trim_changed".to_string());
        }
        let empty = dir.entries().map(|mut it| it.next().is_none());
        if !matches!(empty, Ok(true)) {
            return Err(failed(&"the new folder is not empty"));
        }
        Ok(Self { dir })
    }

    /// Legt `name` im Zuschnittsordner NEU an (`create_new`: eine vorhandene Datei bleibt,
    /// wie sie war, und ist ein Fehler). Der Aufrufer haelt das Handle bis zum Commit.
    pub(super) fn create_file(&self, name: &str) -> Result<std::fs::File, String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        self.dir
            .open_with(name, &options)
            .map(cap_std::fs::File::into_std)
            .map_err(|e| format!("wav_write_failed: {e}"))
    }
}
