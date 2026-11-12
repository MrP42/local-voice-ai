//! lva#522: der Ordner des Zuschnitts und die Vermerke zu den alten Dateien.
//!
//! Der Zuschnitt LOESCHT NICHTS. Die alten Audiodateien (`mic.wav`, `system.wav`,
//! `mic_aec.wav` oder die Dateien eines frueheren Ordners `trim-<ULID>`) bleiben
//! unangetastet als Sicherung liegen. Dieselbe Transaktion wie der Zuschnitt vermerkt sie
//! dauerhaft in `meetings.metadata_json["trim_superseded"]` ([`Superseded`]: Pfad, der
//! ersetzende Zuschnitt, Spur, Zeitpunkt), damit ein spaeteres Aufraeumen (Folge-Issue)
//! weiss, was ersetzt wurde. Es gibt hier deshalb keine Loeschauftraege, keinen Sweep
//! beim Start und keine Entscheidung darueber, was loeschbar ist: die Angriffsflaeche
//! "App loescht Dateien anhand gespeicherter Pfade" entsteht gar nicht erst.
//!
//! Die NEUEN Dateien entstehen nur RELATIV zu Verzeichnis-Handles (`cap-std`):
//!
//! - Der Besprechungsordner ergibt sich aus Datenwurzel plus Besprechungs-ID
//!   ([`open_folder`]), nie aus gespeicherten Pfaden. Er wird einmal geoeffnet (kein
//!   Verweis, am Handle der Datenwurzel per `symlink_metadata` geprueft).
//! - Der Ordner `trim-<ULID>` wird ueber dieses Handle angelegt und sofort selbst als
//!   Handle festgehalten ([`NewDir`]); frisch angelegt, muss er leer sein.
//! - Jede Datei darin entsteht mit `create_new` ueber das Handle des Zuschnittsordners
//!   (nie ueberschreiben) und wird nur dort vermerkt, wo sie wirklich angelegt wurde.
//! - Die Fehlerbereinigung entfernt genau diese vermerkten Dateien ueber DASSELBE Handle
//!   und danach den (leeren) Ordner ueber das Handle des Besprechungsordners mit
//!   `remove_dir` (nie rekursiv). Wird der Ordner nach dem Oeffnen gegen einen Verweis
//!   oder einen anderen Ordner getauscht, trifft die Bereinigung weiter den festgehaltenen
//!   Ordner; unter dem Namen kann sie hoechstens einen LEEREN Ordner entfernen.

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

    fn is_listed_in(&self, list: &[Value]) -> bool {
        list.iter().any(|v| {
            v.get("path").and_then(Value::as_str) == Some(self.path.as_str())
                && v.get("channel").and_then(Value::as_str) == Some(self.channel.as_str())
        })
    }
}

/// Ist `trim_superseded` lesbar (fehlt, `null` oder eine Liste)? Alles andere sind
/// beschaedigte Metadaten: der Zuschnitt darf sie nicht ueberschreiben.
pub(super) fn superseded_readable(root: &Value) -> bool {
    matches!(
        root.get(SUPERSEDED_KEY),
        None | Some(Value::Null) | Some(Value::Array(_))
    )
}

/// Haengt `entries` an `metadata_json["trim_superseded"]` an (ohne Doppelte nach Pfad und
/// Spur; fremde oder aeltere Eintraege bleiben unveraendert). `true`, wenn sich etwas
/// aenderte.
pub(super) fn push_superseded(root: &mut Value, entries: &[Superseded]) -> bool {
    let Some(map) = root.as_object_mut() else {
        return false;
    };
    let mut list = match map.get(SUPERSEDED_KEY) {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    let before = list.len();
    for entry in entries {
        if !entry.is_listed_in(&list) {
            list.push(entry.to_value());
        }
    }
    if list.len() == before {
        return false;
    }
    if list.len() > SUPERSEDED_MAX {
        let excess = list.len() - SUPERSEDED_MAX;
        list.drain(..excess);
        warn!("meetings: trim_superseded is full, {excess} oldest note(s) dropped");
    }
    map.insert(SUPERSEDED_KEY.to_string(), Value::Array(list));
    true
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
    /// Das Verzeichnis selbst: alles entsteht und vergeht relativ dazu.
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
}

fn classify(e: &std::io::Error) -> Unusable {
    if e.kind() == std::io::ErrorKind::NotFound {
        Unusable::Missing
    } else {
        Unusable::Unreadable
    }
}

/// Oeffnet den Besprechungsordner relativ zur Datenwurzel (siehe Moduldoku). `cap-std`
/// folgt Verweisen, die innerhalb der Datenwurzel bleiben (etwa auf den Ordner einer
/// anderen Besprechung); deshalb wird der Besprechungsordner vorher selbst als
/// Nicht-Verweis geprueft.
pub(super) fn open_folder(store: &MeetingStore, meeting_id: &str) -> Result<Folder, Unusable> {
    let path = meeting_dir(store, meeting_id).ok_or(Unusable::Unsafe)?;
    let data_root = store.db_path().parent().ok_or(Unusable::Unsafe)?;
    let root = Dir::open_ambient_dir(data_root, ambient_authority()).map_err(|e| classify(&e))?;
    let name = Path::new(meeting_id);
    match root.symlink_metadata(name) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => {}
        Ok(_) => return Err(Unusable::Unsafe),
        Err(e) => return Err(classify(&e)),
    }
    let dir = root.open_dir(name).map_err(|e| classify(&e))?;
    Ok(Folder { path, dir })
}

// ---------------------------------------------------------------------------
// Neuer Zuschnittsordner (Handle)
// ---------------------------------------------------------------------------

/// Der frisch angelegte Ordner `trim-<ULID>`, als Handle festgehalten, mit den Namen der
/// Dateien, die der Zuschnitt selbst darin angelegt hat.
pub(super) struct NewDir {
    name: String,
    dir: Dir,
    created: Vec<&'static str>,
}

impl NewDir {
    /// Legt `name` im Besprechungsordner `home` an und haelt ihn als Handle fest. Der
    /// Ordner muss vorher gefehlt haben (`create_dir` scheitert sonst) und leer sein.
    pub(super) fn create(home: &Dir, name: &str) -> Result<Self, String> {
        let failed = |e: &dyn std::fmt::Display| format!("wav_write_failed: {e}");
        home.create_dir(name).map_err(|e| failed(&e))?;
        let dir = match home.open_dir(name) {
            Ok(dir) => dir,
            Err(e) => {
                // `remove_dir` entfernt nur einen leeren Ordner.
                let _ = home.remove_dir(name);
                return Err(failed(&e));
            }
        };
        let empty = dir.entries().map(|mut it| it.next().is_none());
        if !matches!(empty, Ok(true)) {
            drop(dir);
            let _ = home.remove_dir(name);
            return Err(failed(&"the new folder is not empty"));
        }
        Ok(Self {
            name: name.to_string(),
            dir,
            created: Vec::new(),
        })
    }

    /// Legt `name` im Zuschnittsordner NEU an (`create_new`: eine vorhandene Datei bleibt,
    /// wie sie war, und ist ein Fehler) und merkt es fuer die Bereinigung.
    pub(super) fn create_file(&mut self, name: &'static str) -> Result<std::fs::File, String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let file = self
            .dir
            .open_with(name, &options)
            .map_err(|e| format!("wav_write_failed: {e}"))?;
        self.created.push(name);
        Ok(file.into_std())
    }

    /// Entfernt eine Datei, die dieser Zuschnitt selbst angelegt hat (etwa eine halb
    /// geschriebene Kopie), ueber das Handle. Eine Datei, die er nicht angelegt hat, bleibt.
    pub(super) fn remove_created(&mut self, name: &'static str) {
        let Some(at) = self.created.iter().position(|n| *n == name) else {
            return;
        };
        self.created.swap_remove(at);
        match self.dir.remove_file(name) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!("meetings: trim: {}/{name} not removed: {e}", self.name),
        }
    }

    /// Fehlerbereinigung: die selbst angelegten Dateien weg, ueber dasselbe Handle, dann
    /// den Ordner, nur wenn er leer ist, ueber das Handle des Besprechungsordners.
    /// Best effort; Pfade stehen im Log, nie Inhalt.
    pub(super) fn discard(self, home: &Dir) {
        let Self { name, dir, created } = self;
        for file in created {
            match dir.remove_file(file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => warn!("meetings: trim: {name}/{file} not removed: {e}"),
            }
        }
        drop(dir);
        match home.remove_dir(&name) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!("meetings: trim: folder {name} not removed: {e}"),
        }
    }
}
