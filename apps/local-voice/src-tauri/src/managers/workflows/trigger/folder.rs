//! Ausloeser „Datei im Ordner“ (B3, AK5): eine neue Datei in einem Ordner (lokal oder im
//! OneDrive-Sync-Ordner) startet einen Lauf, wenn sie FERTIG ist.
//!
//! # Takt-Scan statt Dateiueberwachung (Entscheidung)
//!
//! Der Ausloeser liest den Ordner im Takt der Erinnerung (alle 15 s, `WorkflowHub::on_tick`) und
//! hat keine eigene Ueberwachung (`notify`/`ReadDirectoryChangesW`). Gruende:
//! - Eine Ereignisueberwachung muesste ohnehin entprellen und die Stabilitaet pruefen
//!   (Dateien entstehen in Teilen); das braucht Beobachtungen ueber die Zeit, also einen Takt.
//! - Ereignisse auf Sync-Ordnern (OneDrive) sind unzuverlaessig: Platzhalter, Umbenennungen
//!   beim Synchronisieren, verlorene Ereignisse bei vollem Puffer. Der Scan sieht immer den
//!   tatsaechlichen Stand und holt nach einem Neustart alles nach.
//! - Kein neuer Thread, keine Abhaengigkeit, kein Dauerbetrieb bei ungenutztem Ablauf: ohne
//!   eingeschalteten Ablauf mit diesem Ausloeser liest nichts den Datentraeger.
//!   Der Scan kostet je Ordner ein `read_dir` ohne Unterordner (hoechstens
//!   [`MAX_ENTRIES`] Eintraege).
//! Preis: Reaktionszeit 15 s bis `stable_seconds` + 15 s statt Millisekunden; fuer
//! Eingangsordner ist das ohne Belang.
//!
//! # Wann eine Datei „fertig“ ist
//!
//! 1. **Stabil**: Groesse UND Aenderungszeit sind ueber mindestens ZWEI Takte und
//!    `stable_seconds` (Vorgabe 20 s, 2 bis 300) unveraendert. Eine Datei, die in Teilen
//!    geschrieben oder kopiert wird (oder ein OneDrive-Download, der wachsend eintrifft),
//!    setzt die Uhr bei jeder Aenderung zurueck.
//! 2. **Nicht gesperrt**: ein anderer Prozess hat sie nicht zum Schreiben geoeffnet
//!    (Windows: Oeffnen mit `FILE_SHARE_READ` scheitert, solange ein Schreiber sie haelt).
//! 3. **Lokal da**: OneDrive-Platzhalter, die nur in der Cloud liegen
//!    (`RECALL_ON_DATA_ACCESS`/`RECALL_ON_OPEN`/`OFFLINE`), werden NICHT gelesen (jeder Zugriff
//!    wuerde den Download anstossen), sondern gemeldet (`TickReport::skipped` und Log, beide ohne Dateinamen, und
//!    [`State::cloud_only`] fuer die Oberflaeche). Sobald der Nutzer sie herunterlaedt
//!    („Immer auf diesem Gerät behalten“), wird sie wie jede andere Datei behandelt.
//!    Die Attribute kommen unter Windows NICHT aus `fs::metadata`/`symlink_metadata` (Rust oeffnet
//!    einen Reparse-Point, der kein Symlink ist, ohne `FILE_FLAG_OPEN_REPARSE_POINT` noch einmal und
//!    liefert dann Attribute ohne `REPARSE_POINT`/`OFFLINE`), sondern aus `ablage::eintrag::roh`
//!    (`FindFirstFileExW`, oeffnet nichts). Entschieden wird vor jedem Oeffnen, auch vor der
//!    Pfadaufloesung der Sandbox ([`entry_state`], rein). Wo der Ausloeser danach doch oeffnet
//!    (Sperrpruefung, Hash), geschieht es zweistufig (erst nur Attribute am Handle, dann der Lesezugriff;
//!    `FILE_FLAG_OPEN_NO_RECALL` allein verhindert den Abruf nicht), und das Handle wird vor dem
//!    ersten Lesen erneut geprueft ([`open_checked`]).
//! 4. **Nicht voruebergehend**: Namen wie `~$x.docx`, `.tmp`, `.part`, `.crdownload`,
//!    `desktop.ini` und Punkt-Dateien zaehlen nicht.
//! Unterordner werden nicht abgesucht (der Ordner ist der Eingang, `subfolder` waehlt einen
//! anderen); Verknuepfungen, die den Ordner verlassen, lehnt die Sandbox der Ordner-Integration
//! ab (`integrations::folder::Sandbox`).
//!
//! # Genau einmal je Inhalt
//!
//! Der Schluessel des Laufs ist `file:<sha256 des Inhalts>`, NICHT der Pfad. Deshalb ergeben
//! Umbenennen, Verschieben im Ordner und eine Konfliktkopie (OneDrive: `Name-RECHNER.wav`,
//! `Name (1).wav`) mit demselben Inhalt keinen zweiten Lauf; eine ueberschriebene Datei mit
//! neuem Inhalt dagegen einen neuen. Zwei Schichten sichern das:
//! - die Engine (`UNIQUE (workflow_id, trigger_key)`), auch ueber Neustart und zwei Threads,
//! - das Dateiledger (`ledger`): Pfad (Groesse, Zeit, Hash) und Inhalt je Ablauf. Es
//!   ueberlebt die Aufbewahrungsgrenze der Laeufe und erspart nach einem Neustart das
//!   erneute Hashen. Reihenfolge: ZUERST einreihen, DANN ins Ledger; scheitert das Einreihen
//!   (Platte voll, Datenbank gesperrt), bleibt die Datei ungesehen und der naechste Takt
//!   versucht es wieder, scheitert das Ledger, ist der naechste Versuch ein Duplikat der
//!   Engine. Nichts geht verloren, nichts laeuft doppelt.
//!
//! # Last
//!
//! Hashen liest die ganze Datei einmal (1-MiB-Bloecke, ein Kern). Je Takt und Ablauf werden
//! hoechstens [`MAX_HASH_PER_SCAN`] Dateien gehasht; der Rest folgt im naechsten Takt. Der Scan
//! laeuft nie auf dem Takt-Thread des Kalenders, sondern in `spawn_blocking` (Hub) und nie zweimal
//! gleichzeitig (`State::try_begin`).
//!
//! Der Ausloeser verlangt das Recht `files.read` am Ordner (Aufrufer „Ablauf“); „aus“ ueberspringt
//! den Ablauf mit Grund. Bei „fragen“ wird eingereiht; gefragt wird beim Import-Schritt.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::ablage::eintrag::{self, Roh, Urteil};
use crate::managers::integrations::folder::{self as sandbox_folder, FolderConfig, Sandbox};
use crate::managers::integrations::grants;
use crate::managers::integrations::model::{Caller, Capability, GrantMode, Kind};
use crate::managers::integrations::store as register;
use crate::managers::meetings::store::open_connection;

use super::super::model::TriggerDef;
use super::ledger::{self, Entry};
use super::{enabled_with, fire, iso, Armed, RunSink, TickReport};

pub const KIND: &str = "folder.file_added";
/// Vorgabe fuer `stable_seconds`.
pub const DEFAULT_STABLE_SECONDS: i64 = 20;
/// So viele Eintraege liest ein Scan hoechstens aus einem Ordner.
pub const MAX_ENTRIES: usize = 500;
/// So viele Dateien hasht ein Scan je Ablauf hoechstens.
pub const MAX_HASH_PER_SCAN: usize = 2;
/// Leseblock beim Hashen.
const HASH_BLOCK: usize = 1024 * 1024;

/// Endungen, wenn der Ausloeser keine nennt: alles, was der Import kennt.
pub fn default_extensions() -> Vec<String> {
    crate::media::MEDIA_EXTENSIONS
        .iter()
        .chain(["vtt", "srt"].iter())
        .map(|e| (*e).to_string())
        .collect()
}

/// Die Felder eines Ordner-Ausloesers aus der Definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub integration: String,
    /// Relativ zur Wurzel der Integration; leer = die Wurzel selbst.
    pub subfolder: String,
    /// Klein, ohne Punkt.
    pub extensions: Vec<String>,
    pub stable_ms: i64,
}

fn clean_extension(raw: &str) -> String {
    raw.trim().trim_start_matches('.').to_lowercase()
}

impl Spec {
    pub fn from_def(t: &TriggerDef) -> Option<Self> {
        let integration = t.params.get("integration")?.as_str()?.to_string();
        let extensions: Vec<String> = match t.params.get("extensions").and_then(Value::as_array) {
            Some(list) if !list.is_empty() => list
                .iter()
                .filter_map(Value::as_str)
                .map(clean_extension)
                .filter(|e| !e.is_empty())
                .collect(),
            _ => default_extensions(),
        };
        let stable_s = t
            .params
            .get("stable_seconds")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_STABLE_SECONDS)
            .clamp(2, 300);
        Some(Self {
            integration,
            subfolder: t
                .params
                .get("subfolder")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            extensions,
            stable_ms: stable_s * 1_000,
        })
    }

    pub fn wants(&self, extension: &str) -> bool {
        let e = clean_extension(extension);
        !e.is_empty() && self.extensions.iter().any(|x| *x == e)
    }
}

/// Zusaetzliche Pruefung beim Speichern (`trigger::check_definition`): (Feld, Satz).
pub fn check(params: &Map<String, Value>) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(sub) = params.get("subfolder").and_then(Value::as_str) {
        if !sub.trim().is_empty() && sandbox_folder::check_relative(sub).is_err() {
            out.push((
                "subfolder",
                "Unterordner nicht zulässig: erwartet wird ein relativer Pfad innerhalb des Ordners (ohne „..“, Laufwerk oder Sonderzeichen)."
                    .to_string(),
            ));
        }
    }
    if let Some(list) = params.get("extensions").and_then(Value::as_array) {
        let bad = list.iter().filter_map(Value::as_str).any(|e| {
            let c = clean_extension(e);
            c.is_empty() || c.len() > 10 || !c.chars().all(|ch| ch.is_ascii_alphanumeric())
        });
        if bad {
            out.push((
                "extensions",
                "Endungen bestehen aus Buchstaben und Ziffern (z. B. „wav“ oder „.mp3“)."
                    .to_string(),
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Reine Entscheidungen
// ---------------------------------------------------------------------------

/// Namen, die nie eine fertige Datei des Nutzers sind (Zwischenstaende, Systemdateien).
pub fn is_ignored_name(name: &str) -> bool {
    let n = name.to_lowercase();
    if n.starts_with("~$") || n.starts_with('.') || n == "desktop.ini" || n == "thumbs.db" {
        return true;
    }
    [
        ".tmp",
        ".temp",
        ".part",
        ".partial",
        ".crdownload",
        ".download",
        ".filepart",
        ".lock",
        ".swp",
    ]
    .iter()
    .any(|e| n.ends_with(e))
}

/// Dateiattribute (Windows; macOS: `SF_DATALESS` als `OFFLINE`), die einen Platzhalter ohne lokalen Inhalt
/// kennzeichnen: `OFFLINE`, `RECALL_ON_OPEN`, `RECALL_ON_DATA_ACCESS` (dieselbe Regel wie in der Ablage).
pub fn attributes_cloud_only(attrs: u32) -> bool {
    eintrag::online_urteil(attrs)
}

/// Was der Takt mit einem Eintrag des Ordners tut (Entscheidung aus den Rohdaten, ohne zu oeffnen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryState {
    /// Keine Datei (Ordner, Verknuepfung, Geraet): ignorieren.
    NotAFile,
    /// Nur in der Cloud: nie oeffnen, lesen oder hashen; melden.
    CloudOnly,
    /// Verknuepfung oder unbekannter Reparse-Point: nie folgen, ignorieren.
    Detour,
    /// Lokal vorhanden: darf geprueft und gehasht werden.
    Local,
}

/// `IO_REPARSE_TAG_WOF` (Windows Overlay Filter, „compact /exe“) und `IO_REPARSE_TAG_DEDUP`: der
/// Treiber liefert die Daten beim normalen Oeffnen transparent und holt nichts aus der Cloud.
const REPARSE_TAG_WOF: u32 = 0x8000_0017;
const REPARSE_TAG_DEDUP: u32 = 0x8000_0013;

/// Ist `tag` ein datentransparentes Tag (komprimierte oder deduplizierte lokale Datei)?
pub fn reparse_tag_transparent(tag: u32) -> bool {
    tag == REPARSE_TAG_WOF || tag == REPARSE_TAG_DEDUP
}

/// Urteil des Ausloesers ueber Attribute und Tag: wie `eintrag::reparse_urteil`, doch WOF und
/// Dedup sind lokale Dateien. Beim zweistufigen Oeffnen (`eintrag::oeffnen_ohne_abruf`) oeffnet
/// Stufe 1 (nur Attribute) MIT `FILE_FLAG_OPEN_REPARSE_POINT` und urteilt ueber den Reparse-Point
/// selbst; Stufe 2 (Lesezugriff) FOLGT ihm (ohne das Flag) und liest damit den echten Inhalt. Die
/// Ablage oeffnet auch in Stufe 2 mit dem Flag und behaelt ihr strenges Urteil.
pub fn trigger_urteil(tag: u32, attribute: u32) -> Urteil {
    let u = eintrag::reparse_urteil(tag, attribute);
    if u == Urteil::Umweg
        && attribute & eintrag::ATTR_REPARSE_POINT != 0
        && reparse_tag_transparent(tag)
    {
        return Urteil::Lesbar;
    }
    u
}

/// Reine Entscheidung „ausloesen ja/nein“ aus den Rohdaten (`ablage::eintrag::roh`). Nur
/// [`EntryState::Local`] darf den Ausloeser anstossen; „nur online“ gilt unabhaengig vom Reparse-Bit.
pub fn entry_state(roh: &Roh) -> EntryState {
    if !roh.ist_datei() {
        return EntryState::NotAFile;
    }
    match trigger_urteil(roh.tag, roh.attribute) {
        Urteil::NurOnline => EntryState::CloudOnly,
        Urteil::Umweg => EntryState::Detour,
        Urteil::Lesbar => EntryState::Local,
    }
}

/// Ein Zustand einer Datei ueber mehrere Takte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub size: u64,
    pub mtime_ms: i64,
    /// Seit wann (Takt-Zeit) Groesse und Zeit unveraendert sind.
    pub since_ms: i64,
    /// Wie viele Takte sie so gesehen haben (mit dem ersten).
    pub observations: u32,
}

impl Seen {
    /// Fertig im Sinne der Stabilitaet: zwei Takte und lange genug unveraendert.
    pub fn is_stable(&self, now_ms: i64, stable_ms: i64) -> bool {
        self.observations >= 2 && now_ms.saturating_sub(self.since_ms) >= stable_ms
    }
}

/// Was das System ueber eine Datei sagt. Tests setzen eine Attrappe (Platzhalter,
/// Sperre), die Anwendung `SystemProbe`.
pub trait FileProbe: Send + Sync {
    /// Liegt die Datei nur in der Cloud? `roh` stammt aus `ablage::eintrag::roh`. Der Aufruf darf die
    /// Datei NICHT oeffnen.
    fn cloud_only(&self, path: &Path, roh: &Roh) -> bool;
    /// Haelt ein anderer Prozess die Datei zum Schreiben offen?
    fn locked(&self, path: &Path) -> bool;
}

pub struct SystemProbe;

impl FileProbe for SystemProbe {
    fn cloud_only(&self, _path: &Path, roh: &Roh) -> bool {
        // Unter Windows kommen die Attribute aus `FindFirstFileExW`, unter macOS aus `lstat`
        // (`SF_DATALESS`, siehe `ablage::eintrag`); sonst sind sie 0.
        attributes_cloud_only(roh.attribute)
    }

    #[cfg(windows)]
    fn locked(&self, path: &Path) -> bool {
        // Teilen wie ein Leser (`FILE_SHARE_READ`): Stufe 2 scheitert, solange ein anderer Prozess
        // Schreibzugriff hat. Zweistufig: ein Platzhalter erreicht den Lesezugriff nie.
        let art = eintrag::OeffnenArt {
            folgen: true,
            teilen: 1,
        };
        match eintrag::oeffnen_ohne_abruf(path, art, trigger_urteil) {
            Ok(_) => false,
            // ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33)
            Err(eintrag::OeffnenFehler::Io(e)) => matches!(e.raw_os_error(), Some(32) | Some(33)),
            Err(_) => false,
        }
    }

    #[cfg(not(windows))]
    fn locked(&self, _path: &Path) -> bool {
        false
    }
}

/// Oeffnet `path` zum Lesen ueber das zweistufige Oeffnen der Ablage ([`eintrag::oeffnen_ohne_abruf`]):
/// erst nur Attribute und Urteil am Handle, nur bei „lokal“ der Lesezugriff, dann erneut Urteil und
/// Datei-Kennung. `FILE_FLAG_OPEN_NO_RECALL` allein verhindert den Abruf NICHT; der Schutz ist das Urteil
/// vor dem Lesezugriff (Restrisiko: Auslagern im Fenster zwischen den Stufen, Millisekunden). Wird nichts
/// gelesen, ist das Ergebnis `Err` (Art `Other`, ausser bei Fehlern des Oeffnens selbst).
pub fn open_checked(path: &Path) -> std::io::Result<fs::File> {
    let art = eintrag::OeffnenArt {
        folgen: true,
        teilen: 1 | 2 | 4,
    };
    eintrag::oeffnen_ohne_abruf(path, art, trigger_urteil).map_err(|e| match e {
        eintrag::OeffnenFehler::Io(e) => e,
        eintrag::OeffnenFehler::Urteil(_) => std::io::Error::other("nur online oder Verknüpfung"),
        eintrag::OeffnenFehler::Getauscht => std::io::Error::other("Datei ausgetauscht"),
    })
}

/// Ergebnis des Hashens: Hash und Groesse/Zeit, wie das geprueft-geoeffnete Handle sie NACH dem Lesen
/// meldet (der Verzeichniseintrag hinkt auf NTFS/SMB nach).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hashed {
    pub hash: String,
    pub size: u64,
    pub mtime_ms: i64,
}

/// SHA-256 des Inhalts, in Bloecken (kein Laden der ganzen Datei), ueber ein No-Recall-Handle.
pub fn hash_file(path: &Path) -> std::io::Result<Hashed> {
    // macOS: wird die Datei nach dem Urteil doch noch ausgelagert, scheitert das Lesen (`EDEADLK`), statt
    // sie herunterzuladen; die Sperre gilt fuer diesen Faden bis zum Ende des Hashens.
    let _abrufsperre = eintrag::abruf_sperren();
    let mut file = open_checked(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; HASH_BLOCK];
    loop {
        let n = file.read(&mut buf).map_err(|e| {
            if eintrag::ist_abruf_gesperrt(&e) {
                std::io::Error::other("nur online oder Verknüpfung")
            } else {
                e
            }
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let meta = file.metadata()?;
    Ok(Hashed {
        hash: format!("{:x}", hasher.finalize()),
        size: meta.len(),
        mtime_ms: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as i64),
    })
}

/// SHA-256 des Inhalts als Kleinbuchstaben-Hex.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    hash_file(path).map(|h| h.hash)
}

/// Pfad fuer Anzeige und Parameter: ohne das `\\?\`-Praefix der kanonischen Windows-Pfade.
pub fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        let drive = rest.as_bytes();
        if drive.len() >= 2 && drive[0].is_ascii_alphabetic() && drive[1] == b':' {
            return rest.to_string();
        }
    }
    s.into_owned()
}

// ---------------------------------------------------------------------------
// Zustand zwischen den Takten
// ---------------------------------------------------------------------------

/// Eine Datei, die nur in der Cloud liegt (fuer die Oberflaeche, B7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudFile {
    pub workflow_id: String,
    pub name: String,
}

#[derive(Default)]
struct Inner {
    seen: HashMap<String, Seen>,
    cloud: HashMap<String, CloudFile>,
    /// Zuletzt gemeldetes Problem je Ablauf (Ordner fehlt, Recht aus, ...): jede Aenderung wird
    /// einmal gemeldet, nicht alle 15 s dasselbe.
    problems: HashMap<String, String>,
}

/// Der fluechtige Zustand: Beobachtungen (Stabilitaet), gemeldete Platzhalter und Probleme.
/// Ein Neustart beginnt leer; das Ledger sorgt dafuer, dass nichts doppelt laeuft.
#[derive(Default)]
pub struct State {
    scanning: AtomicBool,
    inner: Mutex<Inner>,
}

/// Haelt den Scan-Platz; der Drop gibt ihn frei (auch bei Panik).
pub struct ScanGuard<'a>(&'a AtomicBool);

impl Drop for ScanGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl State {
    /// Der Scan-Platz: es laeuft nie zwei Scans gleichzeitig.
    pub fn try_begin(&self) -> Option<ScanGuard<'_>> {
        self.scanning
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| ScanGuard(&self.scanning))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn observe(&self, key: &str, size: u64, mtime_ms: i64, now_ms: i64) -> Seen {
        let mut g = self.lock();
        let entry = g.seen.entry(key.to_string()).or_insert(Seen {
            size,
            mtime_ms,
            since_ms: now_ms,
            observations: 0,
        });
        if entry.size == size && entry.mtime_ms == mtime_ms {
            entry.observations = entry.observations.saturating_add(1);
        } else {
            *entry = Seen {
                size,
                mtime_ms,
                since_ms: now_ms,
                observations: 1,
            };
        }
        entry.clone()
    }

    fn reset(&self, key: &str) {
        self.lock().seen.remove(key);
    }

    /// `true`, wenn der Platzhalter neu gemeldet werden soll.
    fn note_cloud(&self, key: &str, file: CloudFile) -> bool {
        self.lock().cloud.insert(key.to_string(), file).is_none()
    }

    fn clear_cloud(&self, key: &str) {
        self.lock().cloud.remove(key);
    }

    /// `true`, wenn das Problem neu oder geaendert ist.
    fn note_problem(&self, workflow_id: &str, message: &str) -> bool {
        let mut g = self.lock();
        match g.problems.get(workflow_id) {
            Some(old) if old == message => false,
            _ => {
                g.problems
                    .insert(workflow_id.to_string(), message.to_string());
                true
            }
        }
    }

    fn clear_problem(&self, workflow_id: &str) {
        self.lock().problems.remove(workflow_id);
    }

    /// Vergisst, was dieser Ablauf nicht mehr im Ordner hat.
    fn retain_keys(&self, workflow_id: &str, keep: &HashSet<String>) {
        let prefix = format!("{workflow_id}|");
        let mut g = self.lock();
        g.seen
            .retain(|k, _| !k.starts_with(&prefix) || keep.contains(k));
        g.cloud
            .retain(|k, _| !k.starts_with(&prefix) || keep.contains(k));
    }

    /// Vergisst alles zu Ablaeufen, die nicht mehr eingeschaltet sind.
    fn retain_workflows(&self, live: &HashSet<String>) {
        let mut g = self.lock();
        g.seen
            .retain(|k, _| k.split('|').next().is_some_and(|w| live.contains(w)));
        g.cloud.retain(|_, f| live.contains(&f.workflow_id));
        g.problems.retain(|w, _| live.contains(w));
    }

    /// Dateien, die nur in der Cloud liegen und deshalb nicht verarbeitet werden.
    pub fn cloud_only(&self) -> Vec<CloudFile> {
        let mut v: Vec<CloudFile> = self.lock().cloud.values().cloned().collect();
        v.sort_by(|a, b| (&a.workflow_id, &a.name).cmp(&(&b.workflow_id, &b.name)));
        v
    }

    /// Laeuft gerade ein Scan? (billige Frage fuer den Takt)
    pub fn is_busy(&self) -> bool {
        self.scanning.load(Ordering::Acquire)
    }

    /// Wie viele Dateien gerade auf ihre Stabilitaet warten (Tests, Anzeige).
    pub fn waiting(&self) -> usize {
        self.lock().seen.len()
    }
}

// ---------------------------------------------------------------------------
// Takt
// ---------------------------------------------------------------------------

/// Ein Takt der Ordner-Ausloeser: durchsucht den Ordner jedes eingeschalteten Ablaufs und
/// reiht fuer jede neue, fertige Datei einen Lauf ein (siehe Moduldoku).
pub fn on_tick(
    sink: &dyn RunSink,
    state: &State,
    db_path: &Path,
    probe: &dyn FileProbe,
    now_ms: i64,
) -> TickReport {
    let mut report = TickReport::default();
    let Some(_guard) = state.try_begin() else {
        report
            .skipped
            .push("Der vorige Durchlauf der Ordnerüberwachung läuft noch.".to_string());
        return report;
    };
    let armed = match enabled_with(sink, &[KIND]) {
        Ok(a) => a,
        Err(e) => {
            report.errors.push(format!("Ablaeufe: {e}"));
            return report;
        }
    };
    let live: HashSet<String> = armed.iter().map(|a| a.row.id.clone()).collect();
    state.retain_workflows(&live);
    if armed.is_empty() {
        return report;
    }
    let conn = match open_connection(db_path) {
        Ok(c) => c,
        Err(e) => {
            report.errors.push(format!("Datenbank: {e}"));
            return report;
        }
    };
    for a in &armed {
        let Some(spec) = Spec::from_def(&a.def.trigger) else {
            continue;
        };
        scan_workflow(sink, state, probe, &conn, a, &spec, now_ms, &mut report);
    }
    report
}

/// Meldet ein Problem einmal je Aenderung (`skipped` mit dem Namen des Ablaufs).
fn problem(state: &State, report: &mut TickReport, a: &Armed, text: &str) {
    if state.note_problem(&a.row.id, text) {
        log::warn!("workflows: Ordner-Ausloeser „{}“: {text}", a.row.name);
        report.skipped.push(format!("{}: {text}", a.row.name));
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_workflow(
    sink: &dyn RunSink,
    state: &State,
    probe: &dyn FileProbe,
    conn: &rusqlite::Connection,
    a: &Armed,
    spec: &Spec,
    now_ms: i64,
    report: &mut TickReport,
) {
    let wf = a.row.id.as_str();
    // ---- Integration, Recht, Ordner -------------------------------------------------
    let integration = match register::get(conn, &spec.integration) {
        Ok(Some(i)) => i,
        Ok(None) => {
            problem(
                state,
                report,
                a,
                &format!(
                    "Die Ordner-Integration „{}“ gibt es nicht (mehr).",
                    spec.integration
                ),
            );
            return;
        }
        Err(e) => {
            report.errors.push(format!("{}: Register: {e}", a.row.name));
            return;
        }
    };
    if integration.kind != Kind::Folder {
        problem(
            state,
            report,
            a,
            &format!(
                "„{}“ ist keine Ordner-Integration; der Ausloeser „Datei im Ordner“ braucht eine.",
                integration.label
            ),
        );
        return;
    }
    let granted = match register::grants_for(conn, &integration.id) {
        Ok(g) => g,
        Err(e) => {
            report.errors.push(format!("{}: Rechte: {e}", a.row.name));
            return;
        }
    };
    let (mode, why) = grants::explain(
        &integration,
        Capability::FilesRead,
        Caller::Workflow,
        &granted,
        None,
    );
    if mode == GrantMode::Off {
        let reason = why.map(|r| r.message()).unwrap_or("Das Recht ist aus.");
        problem(
            state,
            report,
            a,
            &format!("Der Ordner wird nicht überwacht: {reason}"),
        );
        return;
    }
    let config = match FolderConfig::from_config_json(&integration.config_json) {
        Ok(c) => c,
        Err(e) => {
            problem(state, report, a, &e.to_string());
            return;
        }
    };
    let sandbox = match Sandbox::open(&config.path) {
        Ok(s) => s,
        Err(e) => {
            problem(state, report, a, &e.to_string());
            return;
        }
    };
    let dir = match sandbox.resolve_dir(&spec.subfolder, false) {
        Ok(d) => d,
        Err(e) => {
            problem(
                state,
                report,
                a,
                &format!("Der überwachte Ordner ist nicht erreichbar: {e}"),
            );
            return;
        }
    };
    let entries = match fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) => {
            problem(
                state,
                report,
                a,
                &format!("Der Ordner ließ sich nicht lesen ({})", e.kind()),
            );
            return;
        }
    };
    state.clear_problem(wf);

    // ---- Dateien --------------------------------------------------------------------
    let mut keep: HashSet<String> = HashSet::new();
    let mut hashed = 0usize;
    let mut wrote = false;
    for entry in entries.flatten().take(MAX_ENTRIES) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_ignored_name(&name) {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_string();
        if !spec.wants(&extension) {
            continue;
        }
        // Rohdaten OHNE zu oeffnen (unter Windows `FindFirstFileExW`; `fs::metadata` liefert fuer
        // OneDrive-Platzhalter falsche Attribute). Vor allem anderen entscheiden: schon die
        // Pfadaufloesung der Sandbox oeffnet die Datei.
        let Ok(roh) = eintrag::roh(&path) else {
            continue;
        };
        let kind = entry_state(&roh);
        if matches!(kind, EntryState::NotAFile | EntryState::Detour) {
            continue;
        }
        let rel = sandbox.rel_of(&path);
        if rel.is_empty() {
            continue;
        }

        // Nur in der Cloud: nicht anfassen, melden.
        if kind == EntryState::CloudOnly || probe.cloud_only(&path, &roh) {
            let key = format!("{wf}|{}", rel.to_lowercase());
            keep.insert(key.clone());
            let first = state.note_cloud(
                &key,
                CloudFile {
                    workflow_id: wf.to_string(),
                    name: name.clone(),
                },
            );
            if first {
                log::info!(
                    "workflows: „{}“: eine Datei liegt nur in OneDrive und wird nicht heruntergeladen",
                    a.row.name
                );
                // Kein Dateiname: `skipped` geht ins Log; der Name erreicht die Oberflaeche nur ueber
                // `State::cloud_only`.
                report.skipped.push(format!(
                    "{}: eine Datei liegt nur in der Cloud (OneDrive) und wird nicht heruntergeladen; \"Immer auf diesem Gerät behalten\" holt sie.",
                    a.row.name
                ));
            }
            state.reset(&key);
            continue;
        }
        if sandbox.resolve_file(&rel, false).is_err() {
            continue;
        }
        let key = format!("{wf}|{}", rel.to_lowercase());
        keep.insert(key.clone());
        state.clear_cloud(&key);

        let size = roh.groesse;
        let modified = roh.zeit;
        let seen = state.observe(&key, size, modified, now_ms);
        if !seen.is_stable(now_ms, spec.stable_ms) {
            continue;
        }

        // Schon verarbeitet und unveraendert (auch nach einem Neustart): nichts zu tun.
        let pkey = ledger::path_key(wf, &spec.integration, &rel);
        match ledger::get(conn, &pkey) {
            Ok(Some(e)) if e.size == Some(size as i64) && e.mtime == Some(modified) => continue,
            Ok(_) => {}
            Err(e) => {
                report.errors.push(format!("{}: Ledger: {e}", a.row.name));
                continue;
            }
        }
        if hashed >= MAX_HASH_PER_SCAN {
            continue; // der naechste Takt
        }
        if probe.locked(&path) {
            state.reset(&key);
            continue;
        }
        hashed += 1;
        let (hash, at_hash) = match hash_file(&path) {
            Ok(h) => (h.hash.clone(), h),
            Err(e) => {
                // Gesperrt oder verschwunden: beim naechsten Takt erneut.
                log::debug!("workflows: Datei nicht gelesen ({})", e.kind());
                state.reset(&key);
                continue;
            }
        };
        // Waehrend des Hashens veraendert? Dann war sie nicht fertig: zuerst nach dem Handle, dann
        // nach dem Verzeichniseintrag.
        if at_hash.size != size || at_hash.mtime_ms != modified {
            state.reset(&key);
            continue;
        }
        match eintrag::roh(&path) {
            Ok(r)
                if entry_state(&r) == EntryState::Local
                    && r.groesse == size
                    && r.zeit == modified => {}
            _ => {
                state.reset(&key);
                continue;
            }
        }

        // ---- Inhalt schon bekannt (Umbenennen, Konfliktkopie, Verschieben)? ---------
        let ckey = ledger::content_key(wf, &hash);
        match ledger::get(conn, &ckey) {
            Ok(Some(known)) => {
                let mut e = Entry::new(pkey, now_ms);
                e.size = Some(size as i64);
                e.mtime = Some(modified);
                e.content_hash = Some(hash);
                e.run_id = known.run_id;
                match ledger::put(conn, &e) {
                    Ok(()) => wrote = true,
                    Err(err) => report.errors.push(format!("{}: Ledger: {err}", a.row.name)),
                }
                report.duplicates += 1;
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                report.errors.push(format!("{}: Ledger: {e}", a.row.name));
                continue;
            }
        }

        // ---- Neu: einreihen, dann ins Ledger ------------------------------------------
        let trigger = json!({
            "integration": spec.integration,
            "path": display_path(&path),
            "name": name,
            "extension": extension.to_lowercase(),
            "size": size,
            "content_hash": hash,
            "modified": iso(modified),
        });
        let Some(run_id) = fire(sink, report, wf, format!("file:{hash}"), trigger) else {
            continue; // Fehler im Bericht; ungesehen, der naechste Takt versucht es wieder
        };
        let mut content = Entry::new(ckey, now_ms);
        content.content_hash = Some(hash.clone());
        content.run_id = Some(run_id.clone());
        let mut path_entry = Entry::new(pkey, now_ms);
        path_entry.size = Some(size as i64);
        path_entry.mtime = Some(modified);
        path_entry.content_hash = Some(hash);
        path_entry.run_id = Some(run_id);
        for e in [content, path_entry] {
            match ledger::put(conn, &e) {
                Ok(()) => wrote = true,
                Err(err) => {
                    log::warn!("workflows: Ledger nicht geschrieben: {err}");
                    report.errors.push(format!("{}: Ledger: {err}", a.row.name));
                }
            }
        }
    }
    state.retain_keys(wf, &keep);
    if wrote {
        for family in ["p:", "h:"] {
            if let Err(e) = ledger::prune_family(conn, family, ledger::FILE_CAP) {
                log::warn!("workflows: Ledger nicht bereinigt: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests;
