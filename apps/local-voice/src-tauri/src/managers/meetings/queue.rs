//! U7 (Issue #64): Import-Warteschlange und gleichzeitige Transkriptionen.
//!
//! Weitere Dateien lassen sich jederzeit hinzufuegen (Symbol, Ablage, mehrere
//! auf einmal): jede bekommt sofort ihre Besprechung (Status `queued`) und
//! wird in der Reihenfolge des Hinzufuegens abgearbeitet. Die Reihenfolge und
//! der Zustand stehen in der Datenbank (`queue_store.rs`): ein Neustart
//! verliert nichts.
//!
//! Zwei Teile:
//!
//! - **Planung** (rein, ohne Tauri, Modell oder Thread): [`decide`] sagt, ob
//!   die naechste Datei jetzt beginnt, auf welchem Platz und warum nicht;
//!   [`plan_extra_slot`] ist das Speicher-Tor fuer jeden weiteren gleichzeitigen
//!   Lauf.
//! - **Ablauf** ([`ImportQueue`]): ein Verteiler-Thread, der im Takt (und bei
//!   jeder Aenderung) die Planung ausfuehrt und je Lauf einen Thread startet.
//!   Der eigentliche Import steht als Closure ([`Runner`]) bereit, damit
//!   Reihenfolge, Gleichzeitigkeit, Stopp und Absturz ohne Modell testbar sind.
//!
//! **Plaetze.** Der erste Platz (`Primary`) nutzt die gemeinsame Engine des
//! `TranscriptionManager` wie bisher. Jeder weitere (`Extra`, nur bei der
//! Einstellung "Gleichzeitige Transkriptionen" > 1) bekommt eine EIGENE Engine:
//! transcribe-cpp 0.2.4 erlaubt auf einem Modell hoechstens einen laufenden
//! Aufruf ueber alle Sitzungen (ein Mutex je Modell), echte Gleichzeitigkeit
//! braucht also ein eigenes `Model` je Lauf, und damit das Modell noch einmal im
//! Speicher. Dafuer steht das Tor davor: ein weiterer Lauf beginnt nur, wenn
//! RAM (Bedarf plus Systemreserve) und bei GPU-Modellen auch der freie
//! Grafikspeicher reichen. Sonst wartet die Datei ("wartet auf Arbeitsspeicher").
//! Es wird immer nur EINE weitere Engine gleichzeitig geladen (zwei Tore auf
//! demselben freien Speicher waeren eine Wette).
//!
//! **Aufnahme hat Vorrang.** Waehrend eine Live-Aufnahme laeuft, beginnt nichts
//! Neues, und laufende Importe werden am naechsten Block angehalten (der
//! Auftrag bleibt, `job::pause`), danach setzt die Warteschlange sie fort.
//! Begruendung: Die gemeinsame Engine gehoert in dieser Zeit der Aufnahme (ein
//! Import-Block, der sie haelt, laesst den Live-Block scheitern), und eine
//! zweite Engine nimmt Rechenzeit und Grafikspeicher, die das Mitschreiben in
//! Echtzeit braucht. Ein Import hat Zeit, eine Aufnahme nicht.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

use super::final_pass::{ram_need_mb, Hardware};
use super::job::{JobPhase, MeetingJobs};
#[cfg(test)]
use super::queue_store::KIND_AUDIOBOOK;
use super::queue_store::{
    CancelWaiting, MsRange, QueueRow, KIND_MEETING, STATE_RUNNING, STATE_WAITING,
};
use super::store::{Meeting, MeetingStore};

/// Mehr als vier gleichzeitige Transkriptionen gibt es nicht.
pub const MAX_PARALLEL: usize = 4;
/// Reserve im Grafikspeicher ueber die Modellgroesse hinaus (MB).
pub const VRAM_HEADROOM_MB: u64 = 1_024;
/// So lange gilt ein Ergebnis des Speicher-Tors (es fragt Geraete ab).
const GATE_TTL: Duration = Duration::from_secs(2);

/// Zahl der wartenden Dateien beim letzten Takt. `import::run_import` fragt, ob
/// es sich lohnt, das Besprechungsmodell fuer die naechste Datei geladen zu lassen.
static WAITING_NOW: AtomicUsize = AtomicUsize::new(0);

/// Wartet (beim letzten Takt) noch eine weitere Datei?
pub fn more_waiting() -> bool {
    WAITING_NOW.load(Ordering::Acquire) > 0
}

/// Die Einstellung als Anzahl gleichzeitiger Laeufe: 1 bis [`MAX_PARALLEL`].
pub fn clamp_limit(value: u32) -> usize {
    (value as usize).clamp(1, MAX_PARALLEL)
}

/// Hoerbuch-Teile und -Abschnitte (lva#208) laufen bis zu so vielen gleichzeitig,
/// auch wenn die Einstellung fuer Besprechungen kleiner ist (Vorgabe 1):
/// min(4, max(1, Kerne / 2)). Das Speicher-Tor und der Aufnahme-Vorrang gelten
/// unveraendert.
pub fn bulk_parallel(cpus: usize) -> usize {
    (cpus / 2).clamp(1, MAX_PARALLEL)
}

/// Die Grenze fuer die naechste Datei: Besprechungen behalten die Einstellung,
/// Hoerbuch-Auftraege duerfen bis `bulk` ([`bulk_parallel`], mindestens die Einstellung).
pub fn limit_for(audiobook: bool, setting: usize, bulk: usize) -> usize {
    let setting = setting.clamp(1, MAX_PARALLEL);
    if audiobook {
        setting.max(bulk.clamp(1, MAX_PARALLEL))
    } else {
        setting
    }
}

/// CPU-Threads je Engine, wenn sich `engines` Engines die Rechen-Threads des
/// Systemschutzes (`process_guard::cpu_threads_for`) teilen: jede bekommt
/// ihren Anteil, mindestens 1. Zusammen nie mehr als der Systemschutz erlaubt.
pub fn threads_per_engine(cpus: usize, engines: usize) -> usize {
    (crate::process_guard::cpu_threads_for(cpus) / engines.max(1)).max(1)
}

/// Einstellungswert fuer "Automatisch": die Zahl gleichzeitiger Laeufe richtet
/// sich nach der Hardware ([`recommended_parallel`]).
pub const PARALLEL_AUTO: u32 = 0;
/// Ab so vielen logischen Kernen gibt es einen weiteren gleichzeitigen Lauf
/// (jede Engine soll mindestens vier Rechen-Threads behalten).
const CPUS_PER_RUN: usize = 4;
/// Groesse, mit der gerechnet wird, wenn das Modell unbekannt ist (MB).
const UNKNOWN_MODEL_MB: u64 = 1_024;

/// Der gespeicherte Wert der Einstellung: 0 (Automatisch) bleibt, alles
/// andere zaehlt als der naechste gueltige Wert von 1 bis [`MAX_PARALLEL`].
pub fn normalize_setting(value: u32) -> u32 {
    if value == PARALLEL_AUTO {
        PARALLEL_AUTO
    } else {
        clamp_limit(value) as u32
    }
}

/// Die wirksame Grenze: eine feste Zahl gilt, "Automatisch" nimmt die
/// Empfehlung fuer diesen Rechner (erst dann gemessen).
pub fn resolve_limit(setting: u32, recommended: impl FnOnce() -> usize) -> usize {
    if setting == PARALLEL_AUTO {
        recommended().clamp(1, MAX_PARALLEL)
    } else {
        clamp_limit(setting)
    }
}

/// Was der Rechner fuer die Empfehlung hergibt (Gesamtwerte, nicht der
/// gerade freie Speicher: den prueft das Speicher-Tor vor jedem Lauf).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HardwareCapacity {
    pub cpus: usize,
    /// Arbeitsspeicher insgesamt (MB); 0 = nicht messbar.
    pub ram_total_mb: u64,
    /// Grafikspeicher der groessten eigenen Karte (MB), nur wenn die
    /// Transkription die GPU nutzen darf; `None` = keine oder nicht messbar.
    pub vram_total_mb: Option<u64>,
}

/// Wodurch die Empfehlung begrenzt ist (fuer die Begruendung in der Oberflaeche).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ParallelLimitedBy {
    /// Die Obergrenze der App ([`MAX_PARALLEL`]).
    Maximum,
    /// Die Zahl der Prozessorkerne.
    Cpu,
    /// Der Arbeitsspeicher.
    Ram,
    /// Der Grafikspeicher.
    Vram,
}

/// Herleitung der Empfehlung: was jeder Faktor allein erlauben wuerde (ohne
/// die Obergrenze der App) und was daraus folgt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParallelEstimate {
    /// 1 bis [`MAX_PARALLEL`].
    pub recommended: usize,
    /// Der begrenzende Faktor. `Maximum` nur, wenn JEDER Faktor mehr als
    /// [`MAX_PARALLEL`] erlauben wuerde (bei Gleichstand nennt er die Hardware).
    pub limited_by: ParallelLimitedBy,
    /// Laeufe nach Kernen (ein Lauf je [`CPUS_PER_RUN`]), mindestens 1.
    pub by_cpu: usize,
    /// Laeufe nach Arbeitsspeicher; `None` = nicht messbar.
    pub by_ram: Option<usize>,
    /// Laeufe nach Grafikspeicher; `None` = CPU-Modell oder nicht messbar.
    pub by_vram: Option<usize>,
    /// Bedarf je Lauf im Arbeitsspeicher (MB, [`ram_need_mb`]).
    pub ram_per_run_mb: u64,
    /// Bedarf je Lauf im Grafikspeicher (MB, [`vram_gate_mb`]); `None` = CPU-Modell.
    pub vram_per_run_mb: Option<u64>,
}

/// Empfohlene Zahl gleichzeitiger Transkriptionen fuer diesen Rechner und
/// dieses Modell (rein, ohne Messung):
///
/// - Kerne: ein Lauf je [`CPUS_PER_RUN`] logische Kerne.
/// - Arbeitsspeicher: jede Engine laedt das Modell selbst ([`ram_need_mb`]);
///   zusammen hoechstens die Haelfte des Arbeitsspeichers abzueglich der
///   Systemreserve, der Rest bleibt fuer System und andere Programme.
/// - Grafikspeicher (nur GPU-Modell auf GPU): jede Engine braucht
///   [`vram_gate_mb`].
///
/// Das Ergebnis liegt immer zwischen 1 und [`MAX_PARALLEL`]. Was nicht messbar
/// ist, begrenzt nicht; das Speicher-Tor vor jedem weiteren Lauf bleibt.
pub fn recommended_parallel(
    hw: &HardwareCapacity,
    model_size_mb: u64,
    gpu_model: bool,
) -> ParallelEstimate {
    let size_mb = if model_size_mb == 0 {
        UNKNOWN_MODEL_MB
    } else {
        model_size_mb
    };
    let ram_per_run_mb = ram_need_mb(size_mb);
    let vram_per_run_mb = gpu_model.then(|| vram_gate_mb(size_mb));
    let by_cpu = (hw.cpus / CPUS_PER_RUN).max(1);
    let by_ram = (hw.ram_total_mb > 0).then(|| {
        let budget = hw
            .ram_total_mb
            .saturating_sub(crate::process_guard::reserve_for_total_mb(hw.ram_total_mb))
            / 2;
        ((budget / ram_per_run_mb) as usize).max(1)
    });
    let by_vram = vram_per_run_mb.and_then(|need| {
        hw.vram_total_mb
            .filter(|v| *v > 0)
            .map(|vram| ((vram / need) as usize).max(1))
    });
    let mut hardware = (by_cpu, ParallelLimitedBy::Cpu);
    for (n, by) in [
        (by_ram, ParallelLimitedBy::Ram),
        (by_vram, ParallelLimitedBy::Vram),
    ] {
        if let Some(n) = n {
            if n < hardware.0 {
                hardware = (n, by);
            }
        }
    }
    let (recommended, limited_by) = if hardware.0 > MAX_PARALLEL {
        (MAX_PARALLEL, ParallelLimitedBy::Maximum)
    } else {
        hardware
    };
    ParallelEstimate {
        recommended,
        limited_by,
        by_cpu,
        by_ram,
        by_vram,
        ram_per_run_mb,
        vram_per_run_mb,
    }
}

// ---------------------------------------------------------------------------
// Sicht fuer die Oberflaeche
// ---------------------------------------------------------------------------

/// Warum die wartenden Dateien nicht beginnen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    /// Alle erlaubten Plaetze sind belegt (eine Transkription laeuft).
    Slot,
    /// Fuer einen weiteren gleichzeitigen Lauf reicht der Arbeitsspeicher (oder
    /// Grafikspeicher) nicht: "wartet auf Arbeitsspeicher".
    Memory,
    /// Eine Live-Aufnahme laeuft und hat Vorrang.
    Recording,
}

/// Stand der Warteschlange. Reihenfolge = Reihenfolge der Abarbeitung; die
/// Position einer wartenden Datei ist ihr Platz in `waiting` (ab 1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct QueueSnapshot {
    /// Wartende Besprechungen, die naechste zuerst.
    pub waiting: Vec<String>,
    /// Laufende Besprechungen (die Reihenfolge des Starts).
    pub running: Vec<String>,
    /// Davon wegen einer Aufnahme angehalten.
    pub held: Vec<String>,
    /// Eingestellte Zahl gleichzeitiger Laeufe.
    pub limit: u32,
    /// Warum die wartenden nicht beginnen; `None`, wenn nichts wartet oder der
    /// naechste gleich beginnt.
    pub blocked: Option<WaitReason>,
}

/// Jede Aenderung der Warteschlange geht als Ereignis mit dem vollen Stand
/// hinaus (verlustfrei: ein verpasstes Ereignis wird vom naechsten ersetzt).
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
pub struct ImportQueueEvent {
    pub snapshot: QueueSnapshot,
}

// ---------------------------------------------------------------------------
// Planung (rein)
// ---------------------------------------------------------------------------

/// Auf welchem Platz ein Lauf rechnet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Die gemeinsame Engine des `TranscriptionManager`.
    Primary,
    /// Eine eigene Engine (weiterer gleichzeitiger Lauf).
    Extra,
}

/// Wer gerade welchen Platz haelt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Occupancy {
    /// Ein Lauf der Warteschlange nutzt die gemeinsame Engine.
    pub queue_primary: bool,
    /// Ein anderer Auftrag (Enddurchlauf, Nachholen, Neu-Transkription) nutzt sie.
    pub foreign_primary: bool,
    /// Laeufe der Warteschlange mit eigener Engine.
    pub extras: usize,
    /// Eine eigene Engine wird gerade geladen.
    pub extra_loading: bool,
}

/// Was der Verteiler mit der naechsten wartenden Datei tut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Nichts wartet.
    Idle,
    Start(Slot),
    Wait(WaitReason),
}

/// Die Planung. `extra_allowed` ist das Speicher-Tor; es wird nur gefragt, wenn
/// ein weiterer Lauf wirklich eine eigene Engine braeuchte.
///
/// - Aufnahme laeuft: nichts beginnt.
/// - Plaetze (Laeufe der Warteschlange plus fremder Auftrag an der gemeinsamen
///   Engine) sind `limit` oder mehr: warten.
/// - Die gemeinsame Engine ist frei: sie nimmt die Datei.
/// - Sonst eine eigene Engine, wenn keine andere gerade laedt und das Tor offen ist.
pub fn decide(
    has_waiting: bool,
    occ: &Occupancy,
    limit: usize,
    recording: bool,
    extra_allowed: &mut dyn FnMut() -> bool,
) -> Decision {
    if !has_waiting {
        return Decision::Idle;
    }
    if recording {
        return Decision::Wait(WaitReason::Recording);
    }
    let used = usize::from(occ.queue_primary) + usize::from(occ.foreign_primary) + occ.extras;
    if used >= limit.max(1) {
        return Decision::Wait(WaitReason::Slot);
    }
    if !occ.queue_primary && !occ.foreign_primary {
        return Decision::Start(Slot::Primary);
    }
    if occ.extra_loading {
        return Decision::Wait(WaitReason::Slot);
    }
    if extra_allowed() {
        Decision::Start(Slot::Extra)
    } else {
        Decision::Wait(WaitReason::Memory)
    }
}

/// RAM-Bedarf einer eigenen Engine mit diesem Modell: wie das Live-Modell
/// (`final_pass::ram_need_mb`), plus die Systemreserve.
pub fn ram_gate_mb(size_mb: u64) -> u64 {
    ram_need_mb(size_mb) + crate::process_guard::ram_reserve_mb()
}

/// Bedarf im Grafikspeicher: das Modell plus ein Viertel (Arbeitsspeicher der
/// Sitzung) plus feste Reserve.
pub fn vram_gate_mb(size_mb: u64) -> u64 {
    size_mb.saturating_mul(5) / 4 + VRAM_HEADROOM_MB
}

/// Woran ein weiterer Lauf scheitert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtraBlock {
    Ram { need_mb: u64, free_mb: u64 },
    Vram { need_mb: u64, free_mb: u64 },
}

/// Speicher-Tor fuer jeden weiteren gleichzeitigen Lauf. `size_mb`: Groesse des
/// Besprechungsmodells (Katalog); `gpu_model`: transcribe-cpp-Modell (nur das
/// laeuft auf der GPU). Nicht Messbares (`None`) blockiert nie, wie
/// `process_guard::check_ram_for_start`.
pub fn plan_extra_slot(size_mb: u64, gpu_model: bool, hw: &Hardware) -> Result<(), ExtraBlock> {
    if let Some(free_mb) = hw.free_ram_mb {
        let need_mb = ram_gate_mb(size_mb);
        if free_mb < need_mb {
            return Err(ExtraBlock::Ram { need_mb, free_mb });
        }
    }
    if gpu_model && hw.gpu {
        if let Some(free_mb) = hw.free_vram_mb {
            let need_mb = vram_gate_mb(size_mb);
            if free_mb < need_mb {
                return Err(ExtraBlock::Vram { need_mb, free_mb });
            }
        }
    }
    Ok(())
}

/// Untertiteldatei (VTT/SRT)? Die braucht keine Transkription und geht nicht
/// durch die Warteschlange (wie `import::SUBTITLE_EXTENSIONS`).
pub fn is_subtitle(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_lowercase().as_str(), "vtt" | "srt"))
}

/// Titel einer importierten Datei: der Dateiname ohne Endung (wie `import.rs`).
pub fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Import")
        .to_string()
}

/// Bereich eines Hoerbuch-Abschnitts: ohne Grenzen die ganze Datei, nur mit
/// Anfang bis zum Dateiende (letzter Abschnitt), mit Ende hinter dem Anfang
/// (`range_invalid`; ein Ende ohne Anfang ist ungueltig).
pub fn part_range(start_ms: Option<u64>, end_ms: Option<u64>) -> Result<Option<MsRange>, String> {
    match (start_ms, end_ms) {
        (None, None) => Ok(None),
        (Some(start), None) => Ok(Some((start, None))),
        (Some(start), Some(end)) if end > start => Ok(Some((start, Some(end)))),
        _ => Err("range_invalid".to_string()),
    }
}

/// Titel der Hilfs-Besprechung eines Hoerbuch-Teils: der Dateiname, bei einem
/// Abschnitt mit dem Beginn ("Name (ab 40:00)").
pub fn part_title(path: &Path, range: Option<MsRange>) -> String {
    let base = title_from_path(path);
    let Some((start_ms, _)) = range else {
        return base;
    };
    let secs = start_ms / 1_000;
    let (h, m, s) = (secs / 3_600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{base} (ab {h}:{m:02}:{s:02})")
    } else {
        format!("{base} (ab {m}:{s:02})")
    }
}

/// Vor dem Start: ist die Quelldatei noch da? `Some(Fehlercode)`, wenn nicht.
pub fn source_problem(source: Option<&str>) -> Option<&'static str> {
    match source {
        None => Some("import_source_missing"),
        Some(path) if !Path::new(path).is_file() => Some("import_source_missing"),
        Some(_) => None,
    }
}

// ---------------------------------------------------------------------------
// Ablauf
// ---------------------------------------------------------------------------

/// Was ein Lauf bekommt.
pub struct RunRequest {
    pub meeting_id: String,
    pub source_path: String,
    /// `queue_store::KIND_MEETING` oder `KIND_AUDIOBOOK`.
    pub kind: String,
    /// Nur dieser Bereich der Datei (ms), `None` = ganze Datei (lva#208).
    pub range: Option<MsRange>,
    /// Auf so viele Engines verteilen sich die CPU-Threads (die Grenze, die fuer
    /// diesen Auftrag gilt).
    pub engines: usize,
    pub slot: Slot,
    /// Der Lauf meldet, sobald seine eigene Engine geladen ist (oder keine
    /// gebraucht wird): erst dann laedt die Warteschlange die naechste.
    pub engine_ready: EngineReady,
    /// Der Nutzer hat diesen Lauf gestoppt (kann vor dem Auftrag kommen).
    pub stop_requested: Arc<AtomicBool>,
}

/// Wie ein Lauf endete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// Fertig oder vom Nutzer gestoppt: der Lauf hat den Endzustand selbst
    /// geschrieben.
    Done,
    /// Gescheitert: der Lauf hat die Besprechung auf `failed` gesetzt (die
    /// Warteschlange tut es sicherheitshalber noch einmal) und der Text geht ins Log.
    Failed(String),
    /// Der Lauf konnte nicht beginnen (keine eigene Engine ladbar): die Datei
    /// wartet wieder an ihrer Stelle, die Warteschlange wartet kurz.
    Deferred(String),
}

/// Meldet das Laden der eigenen Engine.
#[derive(Clone)]
pub struct EngineReady {
    inner: Arc<Inner>,
    meeting_id: String,
}

impl EngineReady {
    pub fn signal(&self) {
        self.inner.engine_loaded(&self.meeting_id);
    }
}

/// Der eigentliche Import einer Datei; blockiert bis zum Ende.
pub type Runner = Arc<dyn Fn(RunRequest) -> RunOutcome + Send + Sync>;

/// Alles, was die Warteschlange von aussen braucht.
pub struct QueueDeps {
    pub store: Arc<MeetingStore>,
    pub jobs: MeetingJobs,
    /// Eingestellte Zahl gleichzeitiger Laeufe (wird je Takt gelesen).
    pub limit: Arc<dyn Fn() -> usize + Send + Sync>,
    /// Laeuft eine Live-Aufnahme?
    pub recording: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Haelt etwas ausserhalb des Auftragsverzeichnisses die gemeinsame Engine
    /// (Enddurchlauf oder Wiederherstellung nach einem Absturz, schon bevor ihr
    /// Auftrag erscheint)?
    pub shared_busy: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Speicher-Tor fuer einen weiteren Lauf ([`plan_extra_slot`] mit dem Stand
    /// des Rechners).
    pub extra_gate: Arc<dyn Fn() -> bool + Send + Sync>,
    pub runner: Runner,
    /// Neuer Stand der Warteschlange (Ereignis an die Oberflaeche).
    pub publish: Arc<dyn Fn(&QueueSnapshot) + Send + Sync>,
    /// Zustandswechsel, den die Warteschlange selbst schreibt: (Besprechung, Status).
    pub state_event: Arc<dyn Fn(&str, &str) + Send + Sync>,
    /// Fehler einer Besprechung: (Besprechung, Code).
    pub error_event: Arc<dyn Fn(&str, &str) + Send + Sync>,
    /// Grenze fuer Hoerbuch-Auftraege ([`bulk_parallel`]).
    pub bulk_limit: usize,
    /// Takt des Verteilers (Aufnahme und Speicher werden so oft geprueft).
    pub tick: Duration,
    /// So lange wartet die Warteschlange nach einem verschobenen Lauf.
    pub cooldown: Duration,
}

struct RunInfo {
    slot: Slot,
    /// Wegen einer Aufnahme angehalten.
    held: bool,
    started: Instant,
    stop_requested: Arc<AtomicBool>,
    /// Hoerbuch-Teil oder -Abschnitt (zaehlt fuer die wirksame Grenze).
    audiobook: bool,
}

#[derive(Default)]
struct State {
    running: HashMap<String, RunInfo>,
    /// Eine eigene Engine wird gerade geladen (`engine_loaded` hebt es auf).
    loading: HashSet<String>,
    /// Die Aufnahme wurde beim letzten Takt als laufend gesehen.
    recording_seen: bool,
    blocked: Option<WaitReason>,
    /// Nach einem verschobenen Lauf: bis dahin ist das Speicher-Tor zu.
    cooldown_until: Option<Instant>,
    gate_cache: Option<(Instant, bool)>,
    last_published: Option<QueueSnapshot>,
}

struct Inner {
    deps: QueueDeps,
    state: Mutex<State>,
    /// `true`: es gibt etwas zu tun (Aenderung oder Ende eines Laufs).
    dirty: Mutex<bool>,
    wake: Condvar,
    shutdown: AtomicBool,
    /// Zahl der beendeten Takte des Verteilers. Tests warten auf sie statt auf
    /// Schlafzeiten (`settle`): "der Verteiler hat den Stand gesehen" ist ein
    /// Ereignis, keine Frage der Rechnerlast.
    #[cfg_attr(not(test), allow(dead_code))]
    ticks: AtomicU64,
}

/// Die Warteschlange. Das Loeschen beendet den Verteiler (laufende Laeufe
/// enden von selbst).
pub struct ImportQueue {
    inner: Arc<Inner>,
}

impl Drop for ImportQueue {
    fn drop(&mut self) {
        self.inner.shutdown.store(true, Ordering::Release);
        self.inner.notify();
    }
}

/// Wie eine Datei aus der Warteschlange genommen wurde.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// Sie wartete: aus der Warteschlange, Besprechung `cancelled`.
    Cancelled,
    /// Sie lief: der Auftrag wird gestoppt (Endzustand `cancelled` wie beim Stopp).
    Stopping,
    /// Weder wartend noch laufend.
    NotInQueue,
}

impl ImportQueue {
    pub fn new(deps: QueueDeps) -> Self {
        let inner = Arc::new(Inner {
            deps,
            state: Mutex::new(State::default()),
            dirty: Mutex::new(true),
            wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
            ticks: AtomicU64::new(0),
        });
        let dispatcher = Arc::clone(&inner);
        let spawned = std::thread::Builder::new()
            .name("meeting-import-queue".to_string())
            .spawn(move || dispatcher.dispatch_loop());
        if let Err(e) = spawned {
            // Ohne Verteiler laeuft nichts von selbst; die Dateien bleiben
            // wartend (und beim naechsten Start dran). Sichtbar im Log.
            error!("meetings: queue dispatcher not started: {e}");
        }
        Self { inner }
    }

    /// Reiht eine Datei hinten ein: die Besprechung (Status `queued`) gibt es
    /// sofort, die Datei wird nicht gelesen.
    pub fn enqueue(
        &self,
        title: &str,
        source_path: &str,
        consent_confirmed_at: Option<i64>,
    ) -> Result<Meeting, String> {
        let meeting = self
            .inner
            .deps
            .store
            .queue_enqueue(title, source_path, consent_confirmed_at)
            .map_err(|e| format!("queue_enqueue_failed: {e}"))?;
        self.inner.notify();
        Ok(meeting)
    }

    /// lva#208: reiht einen Hoerbuch-Teil (ganze Datei) oder einen Abschnitt davon
    /// (`range`, ms) ein. Hoerbuch-Auftraege laufen bis zu [`BULK_PARALLEL`]
    /// gleichzeitig; ihre Segmentzeiten sind relativ zum Bereich.
    pub fn enqueue_part(
        &self,
        title: &str,
        source_path: &str,
        range: Option<MsRange>,
    ) -> Result<Meeting, String> {
        let meeting = self
            .inner
            .deps
            .store
            // Ein Hoerbuch, keine Gespraechsaufnahme: keine Einwilligung noetig.
            .queue_enqueue_part(
                title,
                source_path,
                Some(chrono::Utc::now().timestamp()),
                range,
            )
            .map_err(|e| format!("queue_enqueue_failed: {e}"))?;
        self.inner.notify();
        Ok(meeting)
    }

    /// G1 (#70): reiht die Datei ein und fuellt damit einen vorhandenen LEEREN
    /// Eintrag statt eine neue Besprechung anzulegen (Fehler: `target_not_empty`,
    /// `meeting_not_found`).
    pub fn enqueue_into(
        &self,
        target_id: &str,
        title: &str,
        source_path: &str,
        consent_confirmed_at: Option<i64>,
    ) -> Result<Meeting, String> {
        let meeting = self
            .inner
            .deps
            .store
            .queue_enqueue_into(target_id, title, source_path, consent_confirmed_at)
            .map_err(|e| super::empty::coded_error("queue_enqueue_failed", &e))?;
        self.inner.notify();
        Ok(meeting)
    }

    /// Reiht eine gestoppte Datei ohne Audio wieder ein (`Fortsetzen`).
    pub fn requeue(&self, meeting_id: &str) -> Result<bool, String> {
        let queued = self
            .inner
            .deps
            .store
            .queue_requeue(meeting_id)
            .map_err(|e| format!("queue_requeue_failed: {e}"))?;
        if queued {
            self.inner.notify();
        }
        Ok(queued)
    }

    /// Nimmt eine wartende Datei heraus oder stoppt die laufende.
    pub fn remove(&self, meeting_id: &str) -> Result<RemoveOutcome, String> {
        let deps = &self.inner.deps;
        let outcome = match deps
            .store
            .queue_cancel_waiting(meeting_id)
            .map_err(|e| format!("queue_remove_failed: {e}"))?
        {
            CancelWaiting::Cancelled => {
                (deps.state_event)(meeting_id, "cancelled");
                RemoveOutcome::Cancelled
            }
            CancelWaiting::NotWaiting => {
                let flag = self
                    .inner
                    .lock()
                    .running
                    .get(meeting_id)
                    .map(|info| Arc::clone(&info.stop_requested));
                match flag {
                    Some(flag) => {
                        // Der Auftrag kann noch fehlen (der Lauf startet gerade):
                        // dann greift das Merkmal, sobald er da ist.
                        flag.store(true, Ordering::Release);
                        let _ = deps.jobs.stop(meeting_id);
                        RemoveOutcome::Stopping
                    }
                    None => RemoveOutcome::NotInQueue,
                }
            }
        };
        self.inner.notify();
        Ok(outcome)
    }

    /// Zieht eine wartende Datei an die erste Stelle der wartenden.
    pub fn move_to_front(&self, meeting_id: &str) -> Result<bool, String> {
        let moved = self
            .inner
            .deps
            .store
            .queue_move_to_front(meeting_id)
            .map_err(|e| format!("queue_move_failed: {e}"))?;
        self.inner.notify();
        Ok(moved)
    }

    /// Eine Besprechung wurde geloescht: aus der Warteschlange, ein laufender
    /// Lauf wird gestoppt (vor dem Loeschen der Audiodateien).
    pub fn forget(&self, meeting_id: &str) {
        let flag = self
            .inner
            .lock()
            .running
            .get(meeting_id)
            .map(|info| Arc::clone(&info.stop_requested));
        if let Some(flag) = flag {
            flag.store(true, Ordering::Release);
            let _ = self.inner.deps.jobs.stop(meeting_id);
        }
        self.inner.notify();
    }

    /// Weckt den Verteiler (z. B. nach einer geaenderten Einstellung).
    pub fn notify(&self) {
        self.inner.notify();
    }

    /// Der aktuelle Stand (aus der Datenbank und dem letzten Takt).
    pub fn snapshot(&self) -> QueueSnapshot {
        let rows = self.inner.deps.store.queue_rows().unwrap_or_default();
        let setting = (self.inner.deps.limit)().clamp(1, MAX_PARALLEL);
        let st = self.inner.lock();
        let limit = effective_limit(&rows, &st, setting, self.inner.deps.bulk_limit);
        build_snapshot(&rows, &st, limit)
    }

    /// Wartet, bis nichts mehr wartet oder laeuft (Headless-Lauf, Tests).
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let busy = {
                let rows = self.inner.deps.store.queue_rows().unwrap_or_default();
                let st = self.inner.lock();
                !rows.is_empty() || !st.running.is_empty()
            };
            if !busy {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

/// Die wirksame Grenze: die Einstellung, solange nur Besprechungen warten oder
/// laufen; sonst die hoehere Grenze der Hoerbuch-Auftraege.
fn effective_limit(rows: &[QueueRow], st: &State, setting: usize, bulk: usize) -> usize {
    let any_bulk =
        rows.iter().any(|r| r.is_audiobook()) || st.running.values().any(|i| i.audiobook);
    limit_for(any_bulk, setting, bulk)
}

fn build_snapshot(rows: &[QueueRow], st: &State, limit: usize) -> QueueSnapshot {
    let waiting: Vec<String> = rows
        .iter()
        .filter(|r| r.state == STATE_WAITING)
        .map(|r| r.meeting_id.clone())
        .collect();
    let mut running: Vec<(&String, &RunInfo)> = st.running.iter().collect();
    running.sort_by_key(|(_, info)| info.started);
    QueueSnapshot {
        blocked: if waiting.is_empty() { None } else { st.blocked },
        waiting,
        held: running
            .iter()
            .filter(|(_, info)| info.held)
            .map(|(id, _)| (*id).clone())
            .collect(),
        running: running.iter().map(|(id, _)| (*id).clone()).collect(),
        limit: limit as u32,
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn notify(&self) {
        *self.dirty.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.wake.notify_all();
    }

    fn dispatch_loop(self: Arc<Self>) {
        while !self.shutdown.load(Ordering::Acquire) {
            // Eine Panik im Takt (z. B. in einer Closure) darf den Verteiler
            // nicht beenden: sonst stuende die Warteschlange fuer immer.
            if catch_unwind(AssertUnwindSafe(|| self.tick())).is_err() {
                error!("meetings: queue tick panicked - continuing");
            }
            self.ticks.fetch_add(1, Ordering::Release);
            let mut dirty = self.dirty.lock().unwrap_or_else(|e| e.into_inner());
            if !*dirty {
                dirty = self
                    .wake
                    .wait_timeout(dirty, self.deps.tick)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            *dirty = false;
        }
    }

    /// Jemand anderes (Enddurchlauf, Nachholen, Neu-Transkription) transkribiert
    /// gerade auf der gemeinsamen Engine?
    fn foreign_primary_busy(&self, own: &HashMap<String, RunInfo>) -> bool {
        self.deps.jobs.snapshots().iter().any(|p| {
            !own.contains_key(&p.meeting_id)
                && matches!(p.phase, JobPhase::Transcription | JobPhase::FinalPass)
        }) || (self.deps.shared_busy)()
    }

    /// Aufnahme beginnt/endet: laufende Importe anhalten bzw. fortsetzen.
    fn apply_recording(&self, st: &mut State, recording: bool) {
        if recording {
            // Jeden Takt neu versuchen, bis es klappt: in der Vorbereitung
            // (Dekodieren) laesst sich nicht anhalten, danach schon.
            for (id, info) in st.running.iter_mut() {
                if !info.held && self.deps.jobs.pause(id).is_ok() {
                    info.held = true;
                }
            }
            st.recording_seen = true;
        } else if st.recording_seen {
            for (id, info) in st.running.iter_mut() {
                if info.held {
                    let _ = self.deps.jobs.resume(id);
                    info.held = false;
                }
            }
            st.recording_seen = false;
        }
    }

    fn extra_gate_open(&self, st: &mut State) -> bool {
        let now = Instant::now();
        if st.cooldown_until.is_some_and(|until| now < until) {
            return false;
        }
        if let Some((at, open)) = st.gate_cache {
            if now.duration_since(at) < self.deps.gate_ttl() {
                return open;
            }
        }
        let open = (self.deps.extra_gate)();
        st.gate_cache = Some((now, open));
        open
    }

    fn tick(self: &Arc<Self>) {
        let store = &self.deps.store;
        if let Err(e) = store.queue_prune() {
            warn!("meetings: queue prune failed: {e}");
        }
        let rows = match store.queue_rows() {
            Ok(rows) => rows,
            Err(e) => {
                warn!("meetings: queue not readable: {e}");
                return;
            }
        };
        let recording = (self.deps.recording)();
        let limit = (self.deps.limit)().clamp(1, MAX_PARALLEL);
        let snapshot = {
            let mut st = self.lock();
            // Zeilen `running`, zu denen kein Lauf mehr gehoert (das Ende liess
            // sich nicht speichern): aufraeumen, sonst bliebe die Zeile stehen.
            for row in rows.iter().filter(|r| r.state == STATE_RUNNING) {
                if !st.running.contains_key(&row.meeting_id) {
                    let _ = store.queue_finish(&row.meeting_id);
                }
            }
            self.apply_recording(&mut st, recording);

            let waiting: Vec<&QueueRow> =
                rows.iter().filter(|r| r.state == STATE_WAITING).collect();
            WAITING_NOW.store(waiting.len(), Ordering::Release);
            let mut next = 0;
            let mut blocked = None;
            while next < waiting.len() {
                let occ = Occupancy {
                    queue_primary: st.running.values().any(|i| i.slot == Slot::Primary),
                    foreign_primary: self.foreign_primary_busy(&st.running),
                    extras: st
                        .running
                        .values()
                        .filter(|i| i.slot == Slot::Extra)
                        .count(),
                    extra_loading: !st.loading.is_empty(),
                };
                // Das Tor fragt nur, wenn die Planung eine eigene Engine braeuchte.
                let row_limit =
                    limit_for(waiting[next].is_audiobook(), limit, self.deps.bulk_limit);
                let decision = {
                    let mut gate = || self.extra_gate_open(&mut st);
                    decide(true, &occ, row_limit, recording, &mut gate)
                };
                match decision {
                    Decision::Idle => break,
                    Decision::Wait(reason) => {
                        blocked = Some(reason);
                        break;
                    }
                    Decision::Start(slot) => {
                        let row = waiting[next];
                        next += 1;
                        match store.queue_mark_running(&row.meeting_id) {
                            Ok(true) => self.spawn_run(
                                &mut st,
                                &row.meeting_id,
                                slot,
                                row.is_audiobook(),
                                limit_for(row.is_audiobook(), limit, self.deps.bulk_limit),
                            ),
                            // Inzwischen entfernt oder geloescht: die naechste.
                            Ok(false) => continue,
                            Err(e) => {
                                warn!("meetings: queue could not claim {}: {e}", row.meeting_id);
                                break;
                            }
                        }
                    }
                }
            }
            st.blocked = blocked;
            let rows = store.queue_rows().unwrap_or(rows);
            let shown = effective_limit(&rows, &st, limit, self.deps.bulk_limit);
            let snapshot = build_snapshot(&rows, &st, shown);
            if st.last_published.as_ref() == Some(&snapshot) {
                None
            } else {
                st.last_published = Some(snapshot.clone());
                Some(snapshot)
            }
        };
        if let Some(snapshot) = snapshot {
            (self.deps.publish)(&snapshot);
        }
    }

    fn spawn_run(
        self: &Arc<Self>,
        st: &mut State,
        meeting_id: &str,
        slot: Slot,
        audiobook: bool,
        engines: usize,
    ) {
        let stop_requested = Arc::new(AtomicBool::new(false));
        st.running.insert(
            meeting_id.to_string(),
            RunInfo {
                slot,
                held: false,
                started: Instant::now(),
                stop_requested: Arc::clone(&stop_requested),
                audiobook,
            },
        );
        if slot == Slot::Extra {
            st.loading.insert(meeting_id.to_string());
        }
        let inner = Arc::clone(self);
        let id = meeting_id.to_string();
        let name = format!("meeting-import-{}", &id[id.len().saturating_sub(6)..]);
        let spawned = std::thread::Builder::new().name(name).spawn({
            let id = id.clone();
            move || inner.run_thread(id, slot, engines, stop_requested)
        });
        if let Err(e) = spawned {
            // Kein Thread: die Datei wartet wieder, der naechste Takt versucht es.
            warn!("meetings: import thread not started ({id}): {e}");
            st.running.remove(&id);
            st.loading.remove(&id);
            let _ = self.deps.store.queue_mark_waiting(&id);
            st.cooldown_until = Some(Instant::now() + self.deps.cooldown);
        }
    }

    /// Der Lauf einer Datei im eigenen Thread. Endet immer mit einem
    /// Endzustand der Besprechung und freiem Platz, auch nach einer Panik.
    fn run_thread(
        self: Arc<Self>,
        meeting_id: String,
        slot: Slot,
        engines: usize,
        stop_requested: Arc<AtomicBool>,
    ) {
        let _guard = RunGuard {
            inner: Arc::clone(&self),
            meeting_id: meeting_id.clone(),
        };
        let deps = &self.deps;
        let source = deps.store.queue_source(&meeting_id).ok().flatten();
        if let Some(code) = source_problem(source.as_deref()) {
            // Datei geloescht oder verschoben, bevor sie dran war: kein Import,
            // sondern ein sichtbarer Fehler; die Warteschlange laeuft weiter.
            warn!("meetings: import source missing ({meeting_id}): {code}");
            self.fail(&meeting_id, code);
            return;
        }
        // Ein Lesefehler darf nie zur Besprechung ohne Bereich werden: der
        // Hoerbuch-Abschnitt wuerde sonst als ganze Datei transkribiert.
        let (kind, range) = match deps.store.queue_part(&meeting_id) {
            Ok(part) => part,
            Err(e) => {
                warn!("meetings: queue part unreadable ({meeting_id}): {e}");
                self.fail(&meeting_id, "queue_part_unreadable");
                return;
            }
        };
        let request = RunRequest {
            meeting_id: meeting_id.clone(),
            source_path: source.unwrap_or_default(),
            kind,
            range,
            engines,
            slot,
            engine_ready: EngineReady {
                inner: Arc::clone(&self),
                meeting_id: meeting_id.clone(),
            },
            stop_requested,
        };
        match catch_unwind(AssertUnwindSafe(|| (deps.runner)(request))) {
            Ok(RunOutcome::Done) => {
                let _ = deps.store.queue_finish(&meeting_id);
            }
            Ok(RunOutcome::Failed(reason)) => {
                warn!("meetings: queued import failed ({meeting_id}): {reason}");
                let _ = deps.store.queue_finish(&meeting_id);
                // Sicherheitsnetz: nie `processing` zuruecklassen.
                if deps.store.queue_fail_meeting(&meeting_id).unwrap_or(false) {
                    (deps.state_event)(&meeting_id, "failed");
                }
            }
            Ok(RunOutcome::Deferred(reason)) => {
                info!("meetings: queued import deferred ({meeting_id}): {reason}");
                if let Err(e) = deps.store.queue_mark_waiting(&meeting_id) {
                    warn!("meetings: deferred import not re-queued ({meeting_id}): {e}");
                    self.fail(&meeting_id, "queue_failed");
                } else {
                    (deps.state_event)(&meeting_id, "queued");
                    self.lock().cooldown_until = Some(Instant::now() + deps.cooldown);
                }
            }
            Err(_) => {
                error!("meetings: queued import panicked ({meeting_id})");
                self.fail(&meeting_id, "import_panicked");
            }
        }
    }

    /// Endzustand `failed` mit Fehlermeldung; die Zeile entfaellt.
    fn fail(&self, meeting_id: &str, code: &str) {
        let deps = &self.deps;
        let _ = deps.store.queue_finish(meeting_id);
        if deps.store.queue_fail_meeting(meeting_id).unwrap_or(false) {
            (deps.state_event)(meeting_id, "failed");
        }
        (deps.error_event)(meeting_id, code);
    }

    fn engine_loaded(&self, meeting_id: &str) {
        let removed = self.lock().loading.remove(meeting_id);
        if removed {
            self.notify();
        }
    }
}

impl QueueDeps {
    fn gate_ttl(&self) -> Duration {
        // Bei kurzem Takt (Tests) gilt der Takt selbst: ein Ergebnis, das laenger
        // lebt als der Takt, machte eine geaenderte Lage unsichtbar.
        GATE_TTL.min(self.tick)
    }
}

/// Gibt den Platz frei, auch wenn der Lauf in einer Panik endet.
struct RunGuard {
    inner: Arc<Inner>,
    meeting_id: String,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        {
            let mut st = self.inner.lock();
            st.running.remove(&self.meeting_id);
            st.loading.remove(&self.meeting_id);
        }
        self.inner.notify();
    }
}

// ---------------------------------------------------------------------------
// Anbindung an die App
// ---------------------------------------------------------------------------

use std::path::PathBuf;

use tauri::AppHandle;

use super::job::{self, JobGuard};
use super::recorder::MeetingEvent;
use crate::managers::transcription::{bind_extra_engine, TranscriptionManager};

/// Takt des Verteilers in der App.
const APP_TICK: Duration = Duration::from_secs(1);
/// So lange wartet die App nach einer Engine, die sich nicht laden liess.
const APP_COOLDOWN: Duration = Duration::from_secs(30);

fn emit_state(app: &AppHandle, meeting_id: &str, status: &str) {
    let _ = (MeetingEvent::State {
        meeting_id: meeting_id.to_string(),
        status: status.to_string(),
        paused: false,
    })
    .emit(app);
}

fn emit_error(app: &AppHandle, meeting_id: &str, message: &str) {
    let _ = (MeetingEvent::Error {
        meeting_id: meeting_id.to_string(),
        message: message.to_string(),
    })
    .emit(app);
}

/// Der Import einer wartenden Datei mit den Bausteinen der App: eigene Engine
/// (nur auf Extra-Plaetzen), Auftrag mit Fortschritt (P8a) und die Pipeline aus
/// `import.rs` (`run_import`, unveraendert).
fn run_request(
    app: &AppHandle,
    store: &Arc<MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    request: RunRequest,
) -> RunOutcome {
    let RunRequest {
        meeting_id,
        source_path,
        range,
        engines,
        slot,
        engine_ready,
        stop_requested,
        ..
    } = request;

    // Der Auftrag zuerst: waehrend eine eigene Engine laedt (Sekunden), zeigt die
    // Oberflaeche schon "Vorbereitung" statt einer Besprechung ohne Lebenszeichen.
    let job: JobGuard = match job::global().try_start(&meeting_id, job::app_emit(app)) {
        Ok(job) => job,
        Err(e) => {
            emit_error(app, &meeting_id, "import_failed");
            return RunOutcome::Failed(e.to_string());
        }
    };
    emit_state(app, &meeting_id, "processing");
    if stop_requested.load(Ordering::Acquire) {
        let _ = job.handle().stop();
    }

    // Eigene Engine nur fuer weitere gleichzeitige Laeufe. Gelingt das Laden
    // nicht (Speicher), beginnt der Lauf gar nicht: der Auftrag endet, die Datei
    // wartet wieder an ihrer Stelle.
    let _binding = if slot == Slot::Extra {
        job.handle().begin_phase(JobPhase::Prepare, 0);
        let settings = crate::settings::get_settings(app);
        let model_id = tm.meeting_model_target(&settings);
        let threads = threads_per_engine(crate::process_guard::logical_cpus(), engines) as i32;
        let loaded = tm.load_extra_engine(&model_id, threads);
        engine_ready.signal();
        match loaded {
            Ok(engine) => Some(bind_extra_engine(engine)),
            Err(e) => {
                drop(job);
                return RunOutcome::Deferred(format!("engine: {e}"));
            }
        }
    } else {
        engine_ready.signal();
        None
    };
    let path = PathBuf::from(&source_path);
    let result = super::import::run_import(app, store, tm, &meeting_id, &path, range, job.handle());
    drop(job);
    match result {
        Ok(()) => {
            // Teil einer Besprechung aus mehreren Dateien: ist jetzt alles fertig,
            // werden die Teile zusammengefuehrt (`parts.rs`).
            super::parts::after_import(app, store, &meeting_id);
            RunOutcome::Done
        }
        Err(e) => RunOutcome::Failed(e),
    }
}

/// Empfehlung fuer diesen Rechner, wie die Einstellungen sie zeigen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ParallelRecommendation {
    /// So viele Transkriptionen laufen bei "Automatisch" gleichzeitig.
    pub recommended: u32,
    /// Hoechster einstellbarer Wert.
    pub max: u32,
    pub limited_by: ParallelLimitedBy,
    /// Laeufe, die jeder Faktor allein erlauben wuerde (ohne Obergrenze).
    pub by_cpu: u32,
    pub by_ram: Option<u32>,
    pub by_vram: Option<u32>,
    /// Bedarf je weiterem Lauf (das Modell wird je Lauf neu geladen).
    pub ram_per_run_mb: u64,
    pub vram_per_run_mb: Option<u64>,
    pub cpus: u32,
    pub ram_total_mb: u64,
    /// `None`: keine GPU fuer die Transkription oder nicht messbar.
    pub vram_total_mb: Option<u64>,
    /// Groesse des Besprechungsmodells (MB), mit der gerechnet wurde.
    pub model_size_mb: u64,
    /// Das Besprechungsmodell rechnet auf der GPU.
    pub gpu_model: bool,
}

/// So lange gilt eine gemessene Empfehlung (der Verteiler fragt im Takt).
const RECOMMENDATION_TTL: Duration = Duration::from_secs(30);
/// Zwischenspeicher: (Zeitpunkt, GPU fuer die Transkription erlaubt, Empfehlung).
static RECOMMENDATION_CACHE: Mutex<Option<(Instant, bool, ParallelRecommendation)>> =
    Mutex::new(None);

fn gpu_allowed(settings: &crate::settings::AppSettings) -> bool {
    settings.transcribe_accelerator != crate::settings::TranscribeAcceleratorSetting::Cpu
}

/// Gesamtwerte des Rechners fuer [`recommended_parallel`]. Nur der
/// Arbeitsspeicher wird je Aufruf gemessen; die GPU-Kapazitaet stammt aus dem
/// Start-Zwischenspeicher der Geraete.
pub fn probe_capacity(gpu_allowed: bool) -> HardwareCapacity {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    HardwareCapacity {
        cpus: crate::process_guard::logical_cpus(),
        ram_total_mb: sys.total_memory() / (1024 * 1024),
        vram_total_mb: if gpu_allowed {
            crate::managers::transcription::transcribe_gpu_total_mb()
        } else {
            None
        },
    }
}

/// Die Empfehlung fuer das aktuelle Besprechungsmodell. Der Verteiler nutzt
/// den Zwischenspeicher (30 s, neu bei geaendertem Beschleuniger) und fragt
/// dann weder Modell noch Geraete ab; `fresh` (die Einstellungsseite) misst
/// immer neu, etwa nach einem Modellwechsel. Gemessen wird ohne gehaltene Sperre.
pub fn recommendation_for_app(
    app: &AppHandle,
    tm: &TranscriptionManager,
    fresh: bool,
) -> ParallelRecommendation {
    let settings = crate::settings::get_settings(app);
    let allowed = gpu_allowed(&settings);
    if !fresh {
        let cache = RECOMMENDATION_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((at, cached_allowed, rec)) = *cache {
            if cached_allowed == allowed && at.elapsed() < RECOMMENDATION_TTL {
                return rec;
            }
        }
    }
    let (model_size_mb, gpu_model) = tm.meeting_model_footprint(&settings);
    let hw = probe_capacity(allowed);
    let gpu_model = gpu_model && hw.vram_total_mb.is_some();
    let est = recommended_parallel(&hw, model_size_mb, gpu_model);
    let rec = ParallelRecommendation {
        recommended: est.recommended as u32,
        max: MAX_PARALLEL as u32,
        limited_by: est.limited_by,
        by_cpu: est.by_cpu as u32,
        by_ram: est.by_ram.map(|n| n as u32),
        by_vram: est.by_vram.map(|n| n as u32),
        ram_per_run_mb: est.ram_per_run_mb,
        vram_per_run_mb: est.vram_per_run_mb,
        cpus: hw.cpus as u32,
        ram_total_mb: hw.ram_total_mb,
        vram_total_mb: hw.vram_total_mb,
        model_size_mb,
        gpu_model,
    };
    *RECOMMENDATION_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((Instant::now(), allowed, rec));
    rec
}

impl ImportQueue {
    /// Die Warteschlange der App: Einstellung `meeting_import_parallel`,
    /// Aufnahme-Vorrang ueber `recording`, Speicher-Tor ueber
    /// `final_pass::probe_hardware`.
    pub fn for_app(
        app: &AppHandle,
        store: Arc<MeetingStore>,
        tm: Arc<TranscriptionManager>,
        recording: Arc<dyn Fn() -> bool + Send + Sync>,
        shared_busy: Arc<dyn Fn() -> bool + Send + Sync>,
        limit_override: Option<usize>,
    ) -> Self {
        let limit_app = app.clone();
        let limit_tm = Arc::clone(&tm);
        let gate_app = app.clone();
        let gate_tm = Arc::clone(&tm);
        let run_app = app.clone();
        let run_store = Arc::clone(&store);
        let run_tm = Arc::clone(&tm);
        let publish_app = app.clone();
        let state_app = app.clone();
        let error_app = app.clone();
        Self::new(QueueDeps {
            store,
            jobs: job::global().clone(),
            limit: Arc::new(move || match limit_override {
                // Nur der Headless-Nachweis: gilt fuer diesen Lauf, schreibt nichts.
                Some(limit) => limit.clamp(1, MAX_PARALLEL),
                None => {
                    let setting = crate::settings::get_settings(&limit_app).meeting_import_parallel;
                    resolve_limit(setting, || {
                        recommendation_for_app(&limit_app, &limit_tm, false).recommended as usize
                    })
                }
            }),
            recording,
            shared_busy,
            extra_gate: Arc::new(move || {
                let settings = crate::settings::get_settings(&gate_app);
                let (size_mb, gpu_model) = gate_tm.meeting_model_footprint(&settings);
                let hw = super::final_pass::probe_hardware(&gate_app);
                match plan_extra_slot(size_mb, gpu_model, &hw) {
                    Ok(()) => true,
                    Err(block) => {
                        info!("meetings: weiterer gleichzeitiger Import wartet auf Speicher: {block:?}");
                        false
                    }
                }
            }),
            runner: Arc::new(move |request| run_request(&run_app, &run_store, &run_tm, request)),
            publish: Arc::new(move |snapshot| {
                let _ = ImportQueueEvent {
                    snapshot: snapshot.clone(),
                }
                .emit(&publish_app);
            }),
            state_event: Arc::new(move |id, status| emit_state(&state_app, id, status)),
            error_event: Arc::new(move |id, code| emit_error(&error_app, id, code)),
            bulk_limit: bulk_parallel(crate::process_guard::logical_cpus()),
            tick: APP_TICK,
            cooldown: APP_COOLDOWN,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::job::EmitFn;
    use crate::managers::meetings::job::JobPhase;
    use crate::managers::meetings::recorder::MeetingEvent;
    use crate::managers::meetings::search::index::tests::tmp_store;

    /// Grenze, ab der eine Warteschlange, die nicht leer wird, als haengend gilt.
    /// Grosszuegig: ein Lauf mit paralleler Last braucht Sekunden, ein Fehler Minuten.
    const IDLE: Duration = Duration::from_secs(60);

    // ---- Planung -----------------------------------------------------------

    fn occ(queue_primary: bool, foreign: bool, extras: usize) -> Occupancy {
        Occupancy {
            queue_primary,
            foreign_primary: foreign,
            extras,
            extra_loading: false,
        }
    }

    fn decide_with(o: Occupancy, limit: usize, recording: bool, gate: bool) -> (Decision, bool) {
        let mut asked = false;
        let decision = decide(true, &o, limit, recording, &mut || {
            asked = true;
            gate
        });
        (decision, asked)
    }

    #[test]
    fn nothing_waiting_is_idle_and_never_asks_the_gate() {
        let mut asked = false;
        let d = decide(false, &occ(false, false, 0), 3, false, &mut || {
            asked = true;
            true
        });
        assert_eq!(d, Decision::Idle);
        assert!(!asked);
    }

    #[test]
    fn with_one_slot_the_next_file_waits_until_the_running_one_is_done() {
        assert_eq!(
            decide_with(occ(false, false, 0), 1, false, true).0,
            Decision::Start(Slot::Primary)
        );
        let (d, asked) = decide_with(occ(true, false, 0), 1, false, true);
        assert_eq!(d, Decision::Wait(WaitReason::Slot));
        assert!(!asked, "bei Limit 1 gibt es nichts zu fragen");
    }

    #[test]
    fn a_foreign_job_on_the_shared_engine_counts_as_a_running_transcription() {
        // Enddurchlauf / Neu-Transkription an der gemeinsamen Engine, Limit 1: warten.
        assert_eq!(
            decide_with(occ(false, true, 0), 1, false, true).0,
            Decision::Wait(WaitReason::Slot)
        );
        // Limit 2: eine eigene Engine (das Tor entscheidet).
        assert_eq!(
            decide_with(occ(false, true, 0), 2, false, true).0,
            Decision::Start(Slot::Extra)
        );
        assert_eq!(
            decide_with(occ(false, true, 0), 2, false, false).0,
            Decision::Wait(WaitReason::Memory)
        );
    }

    #[test]
    fn parallel_limits_one_two_three() {
        // Limit 2: der zweite Lauf ist ein Extra, der dritte wartet.
        assert_eq!(
            decide_with(occ(true, false, 0), 2, false, true).0,
            Decision::Start(Slot::Extra)
        );
        assert_eq!(
            decide_with(occ(true, false, 1), 2, false, true).0,
            Decision::Wait(WaitReason::Slot)
        );
        // Limit 3: zwei Extras, der vierte wartet.
        assert_eq!(
            decide_with(occ(true, false, 1), 3, false, true).0,
            Decision::Start(Slot::Extra)
        );
        assert_eq!(
            decide_with(occ(true, false, 2), 3, false, true).0,
            Decision::Wait(WaitReason::Slot)
        );
        // Wird der erste Platz frei, nimmt die naechste Datei ihn (kein dritter Extra).
        assert_eq!(
            decide_with(occ(false, false, 2), 3, false, true).0,
            Decision::Start(Slot::Primary)
        );
        // Limit 1 bei zwei eigenen Engines (Einstellung zurueckgenommen): warten.
        assert_eq!(
            decide_with(occ(false, false, 2), 1, false, true).0,
            Decision::Wait(WaitReason::Slot)
        );
    }

    #[test]
    fn a_closed_memory_gate_keeps_the_next_file_waiting_for_memory() {
        let (d, asked) = decide_with(occ(true, false, 0), 3, false, false);
        assert_eq!(d, Decision::Wait(WaitReason::Memory));
        assert!(asked);
        // Auch die erste Datei fragt das Tor nie: die gemeinsame Engine ist schon geladen.
        let (d, asked) = decide_with(occ(false, false, 0), 3, false, false);
        assert_eq!(d, Decision::Start(Slot::Primary));
        assert!(!asked);
    }

    #[test]
    fn only_one_engine_is_loaded_at_a_time() {
        let loading = Occupancy {
            queue_primary: true,
            extra_loading: true,
            ..Occupancy::default()
        };
        let mut asked = false;
        let d = decide(true, &loading, 3, false, &mut || {
            asked = true;
            true
        });
        assert_eq!(d, Decision::Wait(WaitReason::Slot));
        assert!(
            !asked,
            "zwei Tore auf demselben freien Speicher waeren eine Wette"
        );
    }

    #[test]
    fn a_recording_beats_every_other_rule() {
        for o in [
            occ(false, false, 0),
            occ(true, false, 0),
            occ(true, false, 2),
        ] {
            assert_eq!(
                decide_with(o, 3, true, true).0,
                Decision::Wait(WaitReason::Recording)
            );
        }
    }

    fn hw(ram: Option<u64>, gpu: bool, vram: Option<u64>) -> Hardware {
        Hardware {
            gpu,
            free_vram_mb: vram,
            free_ram_mb: ram,
        }
    }

    #[test]
    fn the_ram_gate_needs_the_model_plus_the_system_reserve() {
        let need = ram_gate_mb(1_600);
        assert_eq!(
            need,
            ram_need_mb(1_600) + crate::process_guard::ram_reserve_mb()
        );
        assert!(
            plan_extra_slot(1_600, false, &hw(Some(need), false, None)).is_ok(),
            "genau auf der Grenze: ja"
        );
        assert_eq!(
            plan_extra_slot(1_600, false, &hw(Some(need - 1), false, None)),
            Err(ExtraBlock::Ram {
                need_mb: need,
                free_mb: need - 1
            })
        );
    }

    #[test]
    fn the_vram_gate_applies_to_gpu_models_on_a_gpu_only() {
        let ram = Some(64_000);
        let need = vram_gate_mb(1_600);
        assert!(plan_extra_slot(1_600, true, &hw(ram, true, Some(need))).is_ok());
        assert_eq!(
            plan_extra_slot(1_600, true, &hw(ram, true, Some(need - 1))),
            Err(ExtraBlock::Vram {
                need_mb: need,
                free_mb: need - 1
            })
        );
        // ONNX-Modell oder CPU-Rechner: der Grafikspeicher ist kein Thema.
        assert!(plan_extra_slot(1_600, false, &hw(ram, true, Some(0))).is_ok());
        assert!(plan_extra_slot(1_600, true, &hw(ram, false, Some(0))).is_ok());
    }

    #[test]
    fn what_cannot_be_measured_never_blocks() {
        assert!(plan_extra_slot(50_000, true, &hw(None, true, None)).is_ok());
    }

    #[test]
    fn auto_setting_stays_auto_and_resolves_to_the_recommendation() {
        assert_eq!(normalize_setting(0), PARALLEL_AUTO);
        assert_eq!(normalize_setting(3), 3);
        assert_eq!(normalize_setting(99), 4);
        assert_eq!(resolve_limit(0, || 3), 3);
        assert_eq!(resolve_limit(0, || 0), 1);
        assert_eq!(resolve_limit(0, || 99), MAX_PARALLEL);
        assert_eq!(resolve_limit(2, || panic!("fest: keine Messung")), 2);
    }

    fn cap(cpus: usize, ram_gb: u64, vram_gb: Option<u64>) -> HardwareCapacity {
        HardwareCapacity {
            cpus,
            ram_total_mb: ram_gb * 1024,
            vram_total_mb: vram_gb.map(|g| g * 1024),
        }
    }

    fn rec(hw: HardwareCapacity, size: u64, gpu: bool) -> (usize, ParallelLimitedBy) {
        let e = recommended_parallel(&hw, size, gpu);
        (e.recommended, e.limited_by)
    }

    #[test]
    fn recommendation_follows_cores_ram_and_vram() {
        use ParallelLimitedBy::*;
        // 16 Threads, 32 GB, 12 GB GPU, Nemotron 750 MB: Kerne erlauben genau 4.
        assert_eq!(rec(cap(16, 32, Some(12)), 750, true), (4, Cpu));
        // Patricks PC (32 Threads, 64 GB, RTX 4090 24 GB): jede Hardware erlaubt
        // mehr als 4 -> ehrlich die Obergrenze der App.
        let e = recommended_parallel(&cap(32, 64, Some(24)), 750, true);
        assert_eq!((e.recommended, e.limited_by), (4, Maximum));
        assert_eq!(e.by_cpu, 8);
        assert!(e.by_ram.unwrap() > 4 && e.by_vram.unwrap() > 4);
        assert_eq!(e.ram_per_run_mb, ram_need_mb(750));
        assert_eq!(e.vram_per_run_mb, Some(vram_gate_mb(750)));
        // Wenige Kerne.
        assert_eq!(rec(cap(8, 32, None), 750, false), (2, Cpu));
        assert_eq!(rec(cap(2, 8, None), 750, false), (1, Cpu));
        // Wenig Arbeitsspeicher: (16 GB - 6 GB Reserve) / 2 = 5 GB, je Engine ~4,5 GB.
        assert_eq!(rec(cap(32, 16, None), 2_900, false), (1, Ram));
        // Grosses Modell auf kleiner Karte: der Grafikspeicher begrenzt.
        assert_eq!(rec(cap(32, 64, Some(8)), 3_000, true), (1, Vram));
        assert_eq!(rec(cap(32, 64, Some(16)), 3_000, true), (3, Vram));
        // CPU-Modell: der Grafikspeicher zaehlt nicht.
        let cpu_model = recommended_parallel(&cap(32, 64, Some(4)), 3_000, false);
        assert_eq!(cpu_model.recommended, 4);
        assert_eq!((cpu_model.by_vram, cpu_model.vram_per_run_mb), (None, None));
        // Nicht messbar begrenzt nicht; unbekanntes Modell = 1 GB.
        let unknown = recommended_parallel(&cap(16, 0, None), 0, true);
        assert_eq!((unknown.recommended, unknown.limited_by), (4, Cpu));
        assert_eq!((unknown.by_ram, unknown.by_vram), (None, None));
        // Nie unter 1, nie ueber MAX_PARALLEL.
        for cpus in [0, 1, 3, 4, 100] {
            for ram in [0, 1, 4, 512] {
                let (n, _) = rec(cap(cpus, ram, Some(1)), 5_000, true);
                assert!((1..=MAX_PARALLEL).contains(&n));
            }
        }
    }

    #[test]
    fn the_limit_is_one_to_four() {
        assert_eq!(clamp_limit(0), 1);
        assert_eq!(clamp_limit(1), 1);
        assert_eq!(clamp_limit(2), 2);
        assert_eq!(clamp_limit(3), 3);
        assert_eq!(clamp_limit(4), 4);
        assert_eq!(clamp_limit(99), 4);
    }

    #[test]
    fn audiobook_parts_get_up_to_four_slots_meetings_keep_their_setting() {
        for setting in 1..=4 {
            assert_eq!(
                limit_for(false, setting, 4),
                setting,
                "Besprechung: Einstellung"
            );
            assert_eq!(
                limit_for(true, setting, 4),
                4,
                "Hoerbuch: max(Einstellung, 4)"
            );
        }
        assert_eq!(limit_for(false, 0, 4), 1);
        assert_eq!(limit_for(false, 99, 4), 4);
        assert_eq!(limit_for(true, 99, 4), 4);
        // Wenige Kerne: Hoerbuch-Grenze kleiner, die Einstellung gilt mindestens.
        assert_eq!(limit_for(true, 1, 2), 2);
        assert_eq!(limit_for(true, 3, 2), 3);
        for (cpus, want) in [(1, 1), (2, 1), (4, 2), (6, 3), (8, 4), (64, 4)] {
            assert_eq!(bulk_parallel(cpus), want, "{cpus} Kerne");
        }
        // Threads teilen sich auf die Engines, zusammen nie mehr als der Systemschutz.
        let total = crate::process_guard::cpu_threads_for(16);
        assert_eq!(threads_per_engine(16, 1), total);
        assert_eq!(threads_per_engine(16, 4), total / 4);
        assert!(threads_per_engine(16, 4) * 4 <= total);
        assert_eq!(threads_per_engine(2, 99), 1);
        // Die Planung selbst: Limit 4 = vier Plaetze (Primary plus drei Extras).
        assert_eq!(
            decide_with(occ(true, false, 2), 4, false, true).0,
            Decision::Start(Slot::Extra)
        );
        assert_eq!(
            decide_with(occ(true, false, 3), 4, false, true).0,
            Decision::Wait(WaitReason::Slot)
        );
    }

    #[test]
    fn a_part_title_names_the_section_start() {
        let p = Path::new("C:/b/Buch.mp3");
        assert_eq!(part_title(p, None), "Buch");
        assert_eq!(part_title(p, Some((0, Some(60_000)))), "Buch (ab 0:00)");
        assert_eq!(
            part_title(p, Some((2_405_000, Some(3_600_000)))),
            "Buch (ab 40:05)"
        );
        assert_eq!(part_title(p, Some((3_723_000, None))), "Buch (ab 1:02:03)");
    }

    #[test]
    fn a_range_needs_a_start_and_an_end_behind_it_when_given() {
        assert_eq!(part_range(None, None), Ok(None));
        assert_eq!(part_range(Some(0), Some(10)), Ok(Some((0, Some(10)))));
        assert_eq!(
            part_range(Some(7), None),
            Ok(Some((7, None))),
            "offenes Ende"
        );
        for bad in [(None, Some(5)), (Some(5), Some(5)), (Some(9), Some(1))] {
            assert_eq!(part_range(bad.0, bad.1), Err("range_invalid".to_string()));
        }
    }

    #[test]
    fn subtitles_skip_the_queue_and_titles_come_from_the_file_name() {
        assert!(is_subtitle(Path::new("C:/in/Talk.VTT")));
        assert!(is_subtitle(Path::new("talk.srt")));
        assert!(!is_subtitle(Path::new("talk.mp3")));
        assert!(!is_subtitle(Path::new("srt")));
        assert_eq!(
            title_from_path(Path::new("C:/in/Kick-off 2026.m4a")),
            "Kick-off 2026"
        );
        assert_eq!(title_from_path(Path::new("")), "Import");
    }

    #[test]
    fn a_missing_source_file_is_reported_before_the_import_starts() {
        assert_eq!(source_problem(None), Some("import_source_missing"));
        assert_eq!(
            source_problem(Some("Z:/gibt/es/nicht.wav")),
            Some("import_source_missing")
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.wav");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(source_problem(Some(file.to_str().unwrap())), None);
        assert_eq!(
            source_problem(Some(dir.path().to_str().unwrap())),
            Some("import_source_missing"),
            "ein Ordner ist keine Datei"
        );
    }

    // ---- Ablauf mit Attrappen ---------------------------------------------

    /// Ein Lauf, den der Test freigibt. Merkt Start, Platz und Gleichzeitigkeit.
    struct Rig {
        _dir: tempfile::TempDir,
        store: Arc<MeetingStore>,
        jobs: MeetingJobs,
        queue: ImportQueue,
        starts: Arc<Mutex<Vec<(String, Slot)>>>,
        running_now: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        release: Arc<Mutex<HashMap<String, Slotbox>>>,
        limit: Arc<AtomicUsize>,
        recording: Arc<AtomicBool>,
        gate: Arc<AtomicBool>,
        shared: Arc<AtomicBool>,
        events: Arc<Mutex<Vec<(String, String)>>>,
        errors: Arc<Mutex<Vec<(String, String)>>>,
        stop_seen: Arc<Mutex<Vec<String>>>,
        /// Art und Bereich je gestartetem Lauf (lva#208).
        parts: Arc<Mutex<HashMap<String, (String, Option<MsRange>)>>>,
        files: tempfile::TempDir,
    }

    /// Die Freigabe eines Laufs: der Test legt sie hinein, der Lauf nimmt sie.
    type Slotbox = Arc<Mutex<Option<Release>>>;

    fn no_emit() -> EmitFn {
        Arc::new(|_event: MeetingEvent| {})
    }

    #[derive(Clone, Debug)]
    enum Release {
        Done,
        Fail,
        Panic,
        Defer,
    }

    struct RigOptions {
        limit: usize,
        /// Grenze fuer Hoerbuch-Auftraege.
        bulk: usize,
        gate: bool,
        /// Die Extra-Laeufe melden `engine_ready` selbst (Test des Ladens).
        manual_engine_ready: bool,
        /// Die Laeufe melden sich beim Auftragsverzeichnis mit Phase Transkription.
        register_jobs: bool,
        tick: Duration,
        /// Abkuehlzeit nach einem verschobenen Lauf.
        cooldown: Duration,
    }

    impl Default for RigOptions {
        fn default() -> Self {
            Self {
                limit: 1,
                bulk: 4,
                gate: true,
                manual_engine_ready: false,
                register_jobs: false,
                tick: Duration::from_millis(20),
                cooldown: Duration::from_millis(150),
            }
        }
    }

    fn rig(options: RigOptions) -> Rig {
        let (dir, store) = tmp_store();
        let store = Arc::new(store);
        let jobs = MeetingJobs::new();
        let starts = Arc::new(Mutex::new(Vec::new()));
        let running_now = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let release: Arc<Mutex<HashMap<String, Slotbox>>> = Arc::default();
        let limit = Arc::new(AtomicUsize::new(options.limit));
        let recording = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(AtomicBool::new(options.gate));
        let events: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let errors: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let stop_seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let parts_outer: Arc<Mutex<HashMap<String, (String, Option<MsRange>)>>> = Arc::default();

        let runner: Runner = {
            let parts = parts_outer.clone();
            let (starts, running_now, peak, release, jobs, store, stop_seen) = (
                starts.clone(),
                running_now.clone(),
                peak.clone(),
                release.clone(),
                jobs.clone(),
                store.clone(),
                stop_seen.clone(),
            );
            let manual = options.manual_engine_ready;
            let register = options.register_jobs;
            Arc::new(move |request: RunRequest| {
                let slot: Slotbox = Arc::default();
                release
                    .lock()
                    .unwrap()
                    .insert(request.meeting_id.clone(), Arc::clone(&slot));
                parts.lock().unwrap().insert(
                    request.meeting_id.clone(),
                    (request.kind.clone(), request.range),
                );
                starts
                    .lock()
                    .unwrap()
                    .push((request.meeting_id.clone(), request.slot));
                let now = running_now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                if !(manual && request.slot == Slot::Extra) {
                    request.engine_ready.signal();
                }
                let job = register.then(|| {
                    let job = jobs
                        .try_start(&request.meeting_id, no_emit())
                        .expect("Auftrag");
                    job.handle().begin_phase(JobPhase::Transcription, 1_000);
                    job
                });
                // Wartet auf die Freigabe des Tests oder einen Stopp; ein
                // angehaltener Lauf (Pause) wacht fuer die Freigabe auf.
                let outcome = loop {
                    if request.stop_requested.load(Ordering::Acquire) {
                        stop_seen.lock().unwrap().push(request.meeting_id.clone());
                        break Release::Done;
                    }
                    if let Some(r) = slot.lock().unwrap().take() {
                        break r;
                    }
                    if let Some(job) = &job {
                        use crate::managers::meetings::job::Gate;
                        let released = || slot.lock().unwrap().is_some();
                        if let Gate::Stopped = job.checkpoint(&released) {
                            stop_seen.lock().unwrap().push(request.meeting_id.clone());
                            break Release::Done;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(5));
                };
                running_now.fetch_sub(1, Ordering::SeqCst);
                drop(job);
                match outcome {
                    Release::Done => {
                        // Wie der echte Lauf: Endzustand `ready` schreiben.
                        let _ = store.set_status(
                            &request.meeting_id,
                            crate::managers::meetings::store::MeetingStatus::Ready,
                        );
                        RunOutcome::Done
                    }
                    Release::Fail => RunOutcome::Failed("kaputt".into()),
                    Release::Panic => panic!("Absturz im Lauf"),
                    Release::Defer => RunOutcome::Deferred("kein Speicher".into()),
                }
            })
        };

        let events_c = events.clone();
        let errors_c = errors.clone();
        let (limit_c, recording_c, gate_c) = (limit.clone(), recording.clone(), gate.clone());
        let shared = Arc::new(AtomicBool::new(false));
        let shared_c = shared.clone();
        let queue = ImportQueue::new(QueueDeps {
            store: store.clone(),
            jobs: jobs.clone(),
            limit: Arc::new(move || limit_c.load(Ordering::SeqCst)),
            recording: Arc::new(move || recording_c.load(Ordering::SeqCst)),
            shared_busy: Arc::new(move || shared_c.load(Ordering::SeqCst)),
            extra_gate: Arc::new(move || gate_c.load(Ordering::SeqCst)),
            runner,
            publish: Arc::new(|_| {}),
            state_event: Arc::new(move |id, status| {
                events_c
                    .lock()
                    .unwrap()
                    .push((id.to_string(), status.to_string()))
            }),
            error_event: Arc::new(move |id, code| {
                errors_c
                    .lock()
                    .unwrap()
                    .push((id.to_string(), code.to_string()))
            }),
            bulk_limit: options.bulk,
            tick: options.tick,
            cooldown: options.cooldown,
        });
        Rig {
            _dir: dir,
            store,
            jobs,
            queue,
            starts,
            running_now,
            peak,
            release,
            limit,
            recording,
            gate,
            shared,
            events,
            errors,
            stop_seen,
            parts: parts_outer,
            files: tempfile::tempdir().unwrap(),
        }
    }

    impl Rig {
        /// Legt die Quelldatei an und reiht sie ein.
        fn add(&self, name: &str) -> String {
            let path = self.files.path().join(format!("{name}.wav"));
            std::fs::write(&path, b"RIFF").unwrap();
            self.queue
                .enqueue(name, path.to_str().unwrap(), Some(1))
                .unwrap()
                .id
        }

        /// Reiht einen Hoerbuch-Teil (lva#208) ein.
        fn add_part(&self, name: &str, range: Option<MsRange>) -> String {
            let path = self.files.path().join(format!("{name}.mp3"));
            std::fs::write(&path, b"ID3").unwrap();
            self.queue
                .enqueue_part(name, path.to_str().unwrap(), range)
                .unwrap()
                .id
        }

        fn started(&self) -> Vec<String> {
            self.starts
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| id.clone())
                .collect()
        }

        fn slot_of(&self, id: &str) -> Slot {
            self.starts
                .lock()
                .unwrap()
                .iter()
                .find(|(m, _)| m == id)
                .unwrap()
                .1
        }

        fn finish(&self, id: &str, how: Release) {
            eventually(
                || self.release.lock().unwrap().contains_key(id),
                "Lauf gestartet",
            );
            let slot = self.release.lock().unwrap().remove(id).unwrap();
            *slot.lock().unwrap() = Some(how);
        }

        fn status(&self, id: &str) -> String {
            self.store.get_meeting(id).unwrap().unwrap().status
        }
    }

    /// Wartet auf ein Ereignis. Die Frist ist nur die Grenze, ab der ein
    /// wirklich haengender Verteiler als Fehler gilt; sie ist grosszuegig, weil
    /// parallele Builds den Rechner minutenlang auslasten koennen.
    fn eventually(mut cond: impl FnMut() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !cond() {
            assert!(Instant::now() < deadline, "Zeit abgelaufen: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// "Es beginnt NICHT": wartet, bis der Verteiler nach diesem Aufruf zwei
    /// volle Takte beendet hat. Der erste kann vor dem Aufruf begonnen und noch
    /// einen aelteren Stand der Datenbank gesehen haben (zuletzt eingereihte
    /// Dateien, ein eben beendeter Lauf); der zweite hat den heutigen Stand sicher
    /// gesehen. Ein Ereignis statt einer Schlafzeit: unter Last dauern Takte
    /// laenger, aber es genuegen immer zwei.
    fn settle(r: &Rig) {
        let seen = r.queue.inner.ticks.load(Ordering::Acquire);
        eventually(
            || r.queue.inner.ticks.load(Ordering::Acquire) >= seen + 2,
            "zwei volle Takte des Verteilers",
        );
    }

    /// Die gestarteten Dateien ohne Reihenfolge. Zwei gleichzeitig beginnende
    /// Laeufe sind zwei Threads: wer zuerst in die Liste schreibt, entscheidet
    /// der Thread-Planer, nicht die Warteschlange (deren Entscheidung steht in
    /// `slot_of`).
    fn started_set(r: &Rig) -> HashSet<String> {
        r.started().into_iter().collect()
    }

    #[test]
    fn files_run_one_after_the_other_in_the_order_they_were_added() {
        let r = rig(RigOptions::default());
        let (a, b, c) = (r.add("a"), r.add("b"), r.add("c"));
        eventually(|| r.started() == vec![a.clone()], "a beginnt");
        settle(&r);
        assert_eq!(
            r.started(),
            vec![a.clone()],
            "b und c warten, solange a laeuft"
        );
        let snap = r.queue.snapshot();
        assert_eq!(snap.running, vec![a.clone()]);
        assert_eq!(
            snap.waiting,
            vec![b.clone(), c.clone()],
            "Positionen 1 und 2"
        );
        assert_eq!(snap.blocked, Some(WaitReason::Slot));
        assert_eq!(r.status(&b), "queued");

        r.finish(&a, Release::Done);
        eventually(|| r.started() == vec![a.clone(), b.clone()], "b beginnt");
        r.finish(&b, Release::Done);
        eventually(
            || r.started() == vec![a.clone(), b.clone(), c.clone()],
            "c beginnt",
        );
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
        assert_eq!(r.peak.load(Ordering::SeqCst), 1, "nie zwei gleichzeitig");
        for id in [&a, &b, &c] {
            assert_eq!(r.status(id), "ready");
        }
        assert_eq!(
            r.queue.snapshot(),
            QueueSnapshot {
                limit: 1,
                ..QueueSnapshot::default()
            }
        );
    }

    #[test]
    fn with_two_slots_two_files_run_at_once_the_second_on_its_own_engine() {
        let r = rig(RigOptions {
            limit: 2,
            ..RigOptions::default()
        });
        let (a, b, c) = (r.add("a"), r.add("b"), r.add("c"));
        eventually(|| r.started().len() == 2, "zwei beginnen");
        settle(&r);
        assert_eq!(
            started_set(&r),
            HashSet::from([a.clone(), b.clone()]),
            "c wartet"
        );
        assert_eq!(r.slot_of(&a), Slot::Primary);
        assert_eq!(r.slot_of(&b), Slot::Extra);
        assert_eq!(r.peak.load(Ordering::SeqCst), 2);

        // Endet die gemeinsame Engine zuerst, nimmt c sie (kein dritter Lauf).
        r.finish(&a, Release::Done);
        eventually(|| r.started().len() == 3, "c beginnt");
        assert_eq!(r.slot_of(&c), Slot::Primary);
        r.finish(&b, Release::Done);
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
        assert_eq!(r.peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn with_three_slots_three_files_run_at_once() {
        let r = rig(RigOptions {
            limit: 3,
            ..RigOptions::default()
        });
        let ids: Vec<String> = ["a", "b", "c", "d"].iter().map(|n| r.add(n)).collect();
        eventually(|| r.started().len() == 3, "drei beginnen");
        settle(&r);
        assert_eq!(r.started().len(), 3, "der vierte wartet");
        assert_eq!(r.peak.load(Ordering::SeqCst), 3);
        assert_eq!(r.queue.snapshot().waiting, vec![ids[3].clone()]);
        for id in &ids[..3] {
            r.finish(id, Release::Done);
        }
        eventually(|| r.started().len() == 4, "der vierte beginnt");
        r.finish(&ids[3], Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_closed_memory_gate_makes_the_second_file_wait_for_memory_until_it_opens() {
        let r = rig(RigOptions {
            limit: 2,
            gate: false,
            ..RigOptions::default()
        });
        let (a, b) = (r.add("a"), r.add("b"));
        eventually(|| r.started() == vec![a.clone()], "a beginnt");
        settle(&r);
        assert_eq!(r.started(), vec![a.clone()], "kein Speicher: b wartet");
        assert_eq!(r.queue.snapshot().blocked, Some(WaitReason::Memory));
        assert_eq!(
            r.status(&b),
            "queued",
            "sichtbar wartend, nicht gescheitert"
        );

        // Speicher wird frei: b beginnt auf einer eigenen Engine.
        r.gate.store(true, Ordering::SeqCst);
        eventually(|| r.started().len() == 2, "b beginnt");
        assert_eq!(r.slot_of(&b), Slot::Extra);
        assert_eq!(r.queue.snapshot().blocked, None);
        r.finish(&a, Release::Done);
        r.finish(&b, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_lowered_limit_lets_the_extra_runs_finish_and_starts_nothing_above_it() {
        let r = rig(RigOptions {
            limit: 3,
            ..RigOptions::default()
        });
        let ids: Vec<String> = ["a", "b", "c", "d"].iter().map(|n| r.add(n)).collect();
        eventually(|| r.started().len() == 3, "drei beginnen");
        r.limit.store(1, Ordering::SeqCst);
        r.queue.notify();
        r.finish(&ids[0], Release::Done);
        settle(&r);
        assert_eq!(
            r.started().len(),
            3,
            "zwei laufen noch, Limit 1: der vierte wartet"
        );
        r.finish(&ids[1], Release::Done);
        settle(&r);
        assert_eq!(r.started().len(), 3);
        r.finish(&ids[2], Release::Done);
        eventually(|| r.started().len() == 4, "jetzt beginnt der vierte");
        r.finish(&ids[3], Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn audiobook_parts_run_four_at_once_with_the_meeting_setting_at_one() {
        let r = rig(RigOptions {
            limit: 1,
            ..RigOptions::default()
        });
        let ids: Vec<String> = (0..6)
            .map(|i| r.add_part(&format!("teil{i}"), None))
            .collect();
        eventually(|| r.started().len() == 4, "vier Teile beginnen");
        settle(&r);
        assert_eq!(r.started().len(), 4, "der fuenfte und sechste warten");
        assert_eq!(r.peak.load(Ordering::SeqCst), 4);
        assert_eq!(r.queue.snapshot().waiting, ids[4..].to_vec());
        assert_eq!(r.queue.snapshot().blocked, Some(WaitReason::Slot));
        assert_eq!(r.slot_of(&ids[0]), Slot::Primary);
        for id in &ids[1..4] {
            assert_eq!(r.slot_of(id), Slot::Extra);
        }
        for id in &ids[..4] {
            r.finish(id, Release::Done);
        }
        eventually(|| r.started().len() == 6, "die letzten beiden beginnen");
        for id in &ids[4..] {
            r.finish(id, Release::Done);
        }
        assert!(r.queue.wait_idle(IDLE));
        assert_eq!(r.peak.load(Ordering::SeqCst), 4, "nie mehr als vier");
    }

    #[test]
    fn a_small_machine_limits_audiobook_parts_and_the_snapshot_shows_the_effective_limit() {
        let r = rig(RigOptions {
            limit: 1,
            bulk: 2,
            ..RigOptions::default()
        });
        let m = r.add("besprechung");
        eventually(|| r.started() == vec![m.clone()], "Besprechung beginnt");
        assert_eq!(
            r.queue.snapshot().limit,
            1,
            "nur Besprechungen: die Einstellung"
        );
        r.finish(&m, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
        let ids: Vec<String> = (0..3).map(|i| r.add_part(&format!("t{i}"), None)).collect();
        eventually(|| r.started().len() == 3, "zwei Teile beginnen");
        settle(&r);
        assert_eq!(r.started().len(), 3, "der dritte Teil wartet (Grenze 2)");
        assert_eq!(r.peak.load(Ordering::SeqCst), 2);
        assert_eq!(r.queue.snapshot().limit, 2, "wirksame Grenze mit Hoerbuch");
        for id in &ids {
            r.finish(id, Release::Done);
        }
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn meetings_keep_their_limit_while_parts_run_and_the_gate_still_applies() {
        let r = rig(RigOptions {
            limit: 1,
            gate: false,
            ..RigOptions::default()
        });
        let a = r.add_part("teil0", None);
        let b = r.add_part("teil1", None);
        eventually(|| r.started() == vec![a.clone()], "Teil 0 beginnt");
        settle(&r);
        assert_eq!(
            r.started(),
            vec![a.clone()],
            "ohne Speicher keine zweite Engine, auch nicht fuer Teile"
        );
        assert_eq!(r.queue.snapshot().blocked, Some(WaitReason::Memory));
        r.gate.store(true, Ordering::SeqCst);
        eventually(|| r.started().len() == 2, "Tor offen: Teil 1 beginnt");
        // Eine Besprechung dahinter wartet bei Einstellung 1, solange Teile laufen.
        let m = r.add("besprechung");
        settle(&r);
        assert_eq!(r.started().len(), 2);
        assert_eq!(r.status(&m), "queued");
        r.finish(&a, Release::Done);
        settle(&r);
        assert_eq!(r.started().len(), 2, "ein Platz frei, aber Grenze 1 gilt");
        r.finish(&b, Release::Done);
        eventually(|| r.started().len() == 3, "die Besprechung beginnt");
        r.finish(&m, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_recording_holds_back_audiobook_parts_too() {
        let r = rig(RigOptions::default());
        r.recording.store(true, Ordering::SeqCst);
        let a = r.add_part("teil0", None);
        settle(&r);
        assert!(r.started().is_empty());
        assert_eq!(r.queue.snapshot().blocked, Some(WaitReason::Recording));
        r.recording.store(false, Ordering::SeqCst);
        eventually(|| r.started() == vec![a.clone()], "nach der Aufnahme");
        r.finish(&a, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_run_gets_its_kind_and_section_and_a_meeting_gets_neither() {
        let r = rig(RigOptions {
            limit: 4,
            ..RigOptions::default()
        });
        let part = r.add_part("buch", Some((1_198_000, Some(2_402_000))));
        let whole = r.add_part("ganz", None);
        let meeting = r.add("besprechung");
        eventually(|| r.started().len() == 3, "alle drei beginnen");
        let parts = r.parts.lock().unwrap().clone();
        assert_eq!(
            parts[&part],
            (
                KIND_AUDIOBOOK.to_string(),
                Some((1_198_000, Some(2_402_000)))
            )
        );
        assert_eq!(parts[&whole], (KIND_AUDIOBOOK.to_string(), None));
        assert_eq!(parts[&meeting], (KIND_MEETING.to_string(), None));
        for id in [&part, &whole, &meeting] {
            r.finish(id, Release::Done);
        }
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn an_unreadable_part_row_fails_the_run_instead_of_importing_the_whole_file() {
        let r = rig(RigOptions::default());
        // Die Aufnahme haelt den Auftrag zurueck, bis die Zeile beschaedigt ist.
        r.recording.store(true, Ordering::SeqCst);
        let id = r.add_part("kaputt", Some((1_000, Some(9_000))));
        // Beschaedigter Bereich (Ende vor dem Anfang), bevor der Lauf beginnt.
        r.store
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE import_part SET range_end_ms = 5 WHERE meeting_id = ?1",
                [&id],
            )
            .unwrap();
        r.recording.store(false, Ordering::SeqCst);
        eventually(|| r.status(&id) == "failed", "Auftrag scheitert");
        assert!(r.started().is_empty(), "nie als Besprechung gestartet");
        // Die Fehlermeldung folgt dem Status `failed` (`fail`): darauf warten,
        // nicht im selben Augenblick erwarten.
        eventually(
            || {
                r.errors
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(m, c)| m == &id && c == "queue_part_unreadable")
            },
            "Fehlermeldung queue_part_unreadable",
        );
    }

    #[test]
    fn a_broken_range_is_refused_and_enqueues_nothing() {
        let r = rig(RigOptions::default());
        let path = r.files.path().join("x.mp3");
        std::fs::write(&path, b"ID3").unwrap();
        let err = r
            .queue
            .enqueue_part("x", path.to_str().unwrap(), Some((10, Some(10))));
        assert!(err.is_err());
        assert!(r.store.queue_rows().unwrap().is_empty());
    }

    #[test]
    fn a_removed_waiting_file_never_starts_and_ends_as_cancelled() {
        let r = rig(RigOptions::default());
        let (a, b, c) = (r.add("a"), r.add("b"), r.add("c"));
        eventually(|| r.started() == vec![a.clone()], "a beginnt");
        assert_eq!(r.queue.remove(&b).unwrap(), RemoveOutcome::Cancelled);
        assert_eq!(r.status(&b), "cancelled");
        assert_eq!(r.queue.remove(&b).unwrap(), RemoveOutcome::NotInQueue);
        assert!(r
            .events
            .lock()
            .unwrap()
            .contains(&(b.clone(), "cancelled".into())));
        assert_eq!(
            r.queue.snapshot().waiting,
            vec![c.clone()],
            "c rueckt auf Position 1"
        );
        r.finish(&a, Release::Done);
        eventually(|| r.started() == vec![a.clone(), c.clone()], "c statt b");
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
        assert!(!r.started().contains(&b));
    }

    #[test]
    fn moving_a_file_to_the_front_starts_it_before_the_ones_ahead() {
        let r = rig(RigOptions::default());
        let (a, b, c, d) = (r.add("a"), r.add("b"), r.add("c"), r.add("d"));
        eventually(|| r.started() == vec![a.clone()], "a beginnt");
        assert!(r.queue.move_to_front(&d).unwrap());
        assert_eq!(
            r.queue.snapshot().waiting,
            vec![d.clone(), b.clone(), c.clone()]
        );
        r.finish(&a, Release::Done);
        eventually(|| r.started().len() == 2, "naechste beginnt");
        assert_eq!(r.started()[1], d, "d zuerst");
        r.finish(&d, Release::Done);
        eventually(|| r.started().len() == 3, "b");
        r.finish(&b, Release::Done);
        eventually(|| r.started().len() == 4, "c");
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
        assert_eq!(r.started(), vec![a, d, b, c]);
    }

    #[test]
    fn stopping_one_running_file_leaves_the_others_running() {
        let r = rig(RigOptions {
            limit: 2,
            register_jobs: true,
            ..RigOptions::default()
        });
        let (a, b, c) = (r.add("a"), r.add("b"), r.add("c"));
        eventually(|| r.started().len() == 2, "zwei beginnen");
        eventually(|| r.jobs.is_running(&b), "Auftrag von b");
        assert_eq!(r.queue.remove(&b).unwrap(), RemoveOutcome::Stopping);
        eventually(|| r.stop_seen.lock().unwrap().contains(&b), "b gestoppt");
        // b endet, c ruecken nach; a laeuft unbehelligt weiter.
        eventually(|| r.started().len() == 3, "c beginnt");
        assert!(!r.stop_seen.lock().unwrap().contains(&a));
        assert_eq!(r.queue.snapshot().running.len(), 2);
        r.finish(&a, Release::Done);
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_stop_before_the_job_exists_is_not_lost() {
        // `remove` setzt das Merkmal; der Lauf sieht es, auch wenn es noch keinen Auftrag gab.
        let r = rig(RigOptions::default());
        let a = r.add("a");
        eventually(|| r.started() == vec![a.clone()], "a beginnt");
        assert_eq!(r.queue.remove(&a).unwrap(), RemoveOutcome::Stopping);
        eventually(|| r.stop_seen.lock().unwrap().contains(&a), "Stopp gesehen");
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_recording_pauses_running_imports_blocks_new_ones_and_resumes_afterwards() {
        let r = rig(RigOptions {
            limit: 2,
            register_jobs: true,
            ..RigOptions::default()
        });
        let (a, b) = (r.add("a"), r.add("b"));
        eventually(|| r.started().len() == 2, "zwei laufen");
        eventually(
            || r.jobs.is_running(&a) && r.jobs.is_running(&b),
            "Auftraege da",
        );
        let c = r.add("c");
        settle(&r);
        assert_eq!(r.started().len(), 2, "c wartet auf einen freien Platz");

        r.recording.store(true, Ordering::SeqCst);
        eventually(
            || r.queue.snapshot().held.len() == 2,
            "beide wegen der Aufnahme angehalten",
        );
        let snap = r.queue.snapshot();
        assert_eq!(snap.blocked, Some(WaitReason::Recording));
        assert_eq!(snap.waiting, vec![c.clone()]);
        // Die Laeufe stehen am Kontrollpunkt (Pause), nicht abgebrochen.
        eventually(
            || {
                r.jobs.snapshots().iter().all(|p| {
                    matches!(
                        p.state,
                        crate::managers::meetings::job::JobRunState::Paused
                            | crate::managers::meetings::job::JobRunState::Pausing
                    )
                })
            },
            "Auftraege pausiert",
        );

        // Ein Lauf endet waehrend der Aufnahme: nichts Neues beginnt an seiner Stelle.
        r.finish(&a, Release::Done);
        settle(&r);
        assert_eq!(
            r.started().len(),
            2,
            "keine neue Datei waehrend der Aufnahme"
        );

        r.recording.store(false, Ordering::SeqCst);
        eventually(|| r.started().len() == 3, "c beginnt nach der Aufnahme");
        eventually(|| r.queue.snapshot().held.is_empty(), "fortgesetzt");
        r.finish(&b, Release::Done);
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_recording_that_starts_before_the_first_file_holds_it_back() {
        let r = rig(RigOptions::default());
        r.recording.store(true, Ordering::SeqCst);
        let a = r.add("a");
        settle(&r);
        assert!(
            r.started().is_empty(),
            "waehrend der Aufnahme beginnt nichts"
        );
        assert_eq!(r.queue.snapshot().blocked, Some(WaitReason::Recording));
        assert_eq!(r.status(&a), "queued");
        r.recording.store(false, Ordering::SeqCst);
        eventually(|| r.started() == vec![a.clone()], "danach beginnt sie");
        r.finish(&a, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_foreign_job_on_the_shared_engine_holds_the_queue_back() {
        // Enddurchlauf/Nachholen/Neu-Transkription (Phase Transkription) eines
        // Auftrags, der nicht zur Warteschlange gehoert: die Engine ist belegt.
        let r = rig(RigOptions::default());
        let foreign = r.jobs.try_start("fremd", no_emit()).unwrap();
        foreign.handle().begin_phase(JobPhase::FinalPass, 1_000);
        let a = r.add("a");
        settle(&r);
        assert!(
            r.started().is_empty(),
            "die gemeinsame Engine gehoert dem Enddurchlauf"
        );
        assert_eq!(r.queue.snapshot().blocked, Some(WaitReason::Slot));
        drop(foreign);
        eventually(|| r.started() == vec![a.clone()], "danach beginnt sie");
        r.finish(&a, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_final_pass_that_has_no_job_yet_still_holds_the_shared_engine() {
        // Die Wiederherstellung startet einen Thread, dessen Auftrag erst einen
        // Augenblick spaeter im Verzeichnis steht: die Warteschlange darf in
        // dieser Luecke nicht auf derselben Engine beginnen.
        let r = rig(RigOptions::default());
        r.shared.store(true, Ordering::SeqCst);
        let a = r.add("a");
        settle(&r);
        assert!(r.started().is_empty());
        r.shared.store(false, Ordering::SeqCst);
        eventually(|| r.started() == vec![a.clone()], "danach beginnt sie");
        r.finish(&a, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_failing_or_crashing_run_never_stalls_the_queue() {
        let r = rig(RigOptions::default());
        let (a, b, c) = (r.add("a"), r.add("b"), r.add("c"));
        eventually(|| r.started() == vec![a.clone()], "a beginnt");
        r.finish(&a, Release::Fail);
        eventually(|| r.started().len() == 2, "b beginnt nach dem Fehler");
        assert_eq!(r.status(&a), "failed", "nie `processing` zuruecklassen");
        r.finish(&b, Release::Panic);
        eventually(|| r.started().len() == 3, "c beginnt nach der Panik");
        assert_eq!(r.status(&b), "failed");
        assert!(r
            .errors
            .lock()
            .unwrap()
            .contains(&(b.clone(), "import_panicked".into())));
        assert!(r
            .events
            .lock()
            .unwrap()
            .contains(&(b.clone(), "failed".into())));
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
        assert_eq!(r.status(&c), "ready");
        assert_eq!(r.running_now.load(Ordering::SeqCst), 0);
        assert!(r.queue.snapshot().running.is_empty(), "der Platz ist frei");
    }

    #[test]
    fn a_file_deleted_before_its_turn_fails_visibly_and_the_queue_moves_on() {
        let r = rig(RigOptions::default());
        let (a, b, c) = (r.add("a"), r.add("b"), r.add("c"));
        eventually(|| r.started() == vec![a.clone()], "a beginnt");
        // Die Datei von b verschwindet, waehrend a noch laeuft.
        std::fs::remove_file(r.files.path().join("b.wav")).unwrap();
        r.finish(&a, Release::Done);
        eventually(
            || r.started() == vec![a.clone(), c.clone()],
            "b wird uebersprungen, c beginnt",
        );
        assert_eq!(r.status(&b), "failed");
        assert!(r
            .errors
            .lock()
            .unwrap()
            .contains(&(b.clone(), "import_source_missing".into())));
        assert!(
            !r.started().contains(&b),
            "kein Import einer fehlenden Datei"
        );
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_meeting_deleted_while_waiting_is_skipped() {
        let r = rig(RigOptions::default());
        let (a, b, c) = (r.add("a"), r.add("b"), r.add("c"));
        eventually(|| r.started() == vec![a.clone()], "a beginnt");
        r.store.soft_delete_meeting(&b).unwrap();
        r.queue.forget(&b);
        assert_eq!(r.queue.snapshot().waiting, vec![c.clone()]);
        r.finish(&a, Release::Done);
        eventually(|| r.started() == vec![a.clone(), c.clone()], "c statt b");
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_deferred_extra_engine_puts_the_file_back_in_its_place_and_retries_later() {
        // Abkuehlzeit lang genug, dass der Stand "b wartet wieder" auch auf einem
        // ausgelasteten Rechner geprueft werden kann, bevor der zweite Versuch
        // beginnt (150 ms reichten im Windows-Tor nicht).
        let r = rig(RigOptions {
            limit: 2,
            cooldown: Duration::from_secs(2),
            ..RigOptions::default()
        });
        let (a, b, c) = (r.add("a"), r.add("b"), r.add("c"));
        eventually(|| r.started().len() == 2, "a und b");
        // b konnte seine Engine nicht laden.
        r.finish(&b, Release::Defer);
        eventually(|| r.status(&b) == "queued", "b wartet wieder");
        assert_eq!(
            r.queue.snapshot().waiting,
            vec![b.clone(), c.clone()],
            "b behaelt seinen Platz vor c"
        );
        assert!(r
            .events
            .lock()
            .unwrap()
            .contains(&(b.clone(), "queued".into())));
        // Nach der Abkuehlzeit versucht die Warteschlange es erneut (Tor offen).
        eventually(
            || r.started().iter().filter(|id| **id == b).count() == 2,
            "zweiter Versuch",
        );
        r.finish(&b, Release::Done);
        r.finish(&a, Release::Done);
        eventually(|| r.started().contains(&c), "c");
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn a_second_engine_is_not_loaded_while_the_first_one_is_still_loading() {
        let r = rig(RigOptions {
            limit: 3,
            manual_engine_ready: true,
            ..RigOptions::default()
        });
        let (a, b, c) = (r.add("a"), r.add("b"), r.add("c"));
        eventually(
            || r.started().len() == 2,
            "a (gemeinsam) und b (Extra) beginnen",
        );
        settle(&r);
        assert_eq!(
            started_set(&r),
            HashSet::from([a.clone(), b.clone()]),
            "c wartet, bis b seine Engine geladen hat (sonst wuerden zwei Tore auf denselben Speicher schauen)"
        );
        assert_eq!(r.slot_of(&a), Slot::Primary);
        assert_eq!(r.slot_of(&b), Slot::Extra);
        // Die Engine von b ist da: jetzt darf c laden.
        // (Das Attrappen-`engine_ready` haengt am Request; wir holen es ueber ein neues Signal.)
        r.queue.inner.engine_loaded(&b);
        eventually(|| r.started().len() == 3, "c beginnt");
        assert_eq!(r.slot_of(&c), Slot::Extra);
        for id in [&a, &b, &c] {
            r.finish(id, Release::Done);
        }
        assert!(r.queue.wait_idle(IDLE));
    }

    #[test]
    fn the_queue_resumes_after_a_restart_in_the_stored_order_without_a_second_start() {
        // Zustand wie nach einem Absturz: drei wartende Dateien (eine vorgezogen)
        // und eine, die beim Absturz lief (ohne Audio) -> `queue_recover`.
        let (dir, store) = tmp_store();
        let store = Arc::new(store);
        let files = tempfile::tempdir().unwrap();
        let add = |name: &str| {
            let path = files.path().join(format!("{name}.wav"));
            std::fs::write(&path, b"RIFF").unwrap();
            store
                .queue_enqueue(name, path.to_str().unwrap(), Some(1))
                .unwrap()
                .id
        };
        let (a, b, c, d) = (add("a"), add("b"), add("c"), add("d"));
        store.queue_mark_running(&a).unwrap();
        store.queue_move_to_front(&d).unwrap();
        assert_eq!(store.queue_recover().unwrap().requeued, vec![a.clone()]);

        // Neustart: neue Warteschlange auf derselben Datenbank.
        let starts: Arc<Mutex<Vec<String>>> = Arc::default();
        let starts_c = starts.clone();
        let store_c = store.clone();
        let queue = ImportQueue::new(QueueDeps {
            store: store.clone(),
            jobs: MeetingJobs::new(),
            limit: Arc::new(|| 1),
            recording: Arc::new(|| false),
            shared_busy: Arc::new(|| false),
            extra_gate: Arc::new(|| true),
            runner: Arc::new(move |request| {
                starts_c.lock().unwrap().push(request.meeting_id.clone());
                request.engine_ready.signal();
                let _ = store_c.set_status(
                    &request.meeting_id,
                    crate::managers::meetings::store::MeetingStatus::Ready,
                );
                RunOutcome::Done
            }),
            publish: Arc::new(|_| {}),
            state_event: Arc::new(|_, _| {}),
            error_event: Arc::new(|_, _| {}),
            bulk_limit: 4,
            tick: Duration::from_millis(20),
            cooldown: Duration::from_millis(50),
        });
        assert!(queue.wait_idle(IDLE));
        // Der abgestuerzte Lauf a behaelt seine Stelle vor der vorgezogenen d (er
        // lief ja schon), danach d, dann b und c in der Reihenfolge des Hinzufuegens.
        assert_eq!(*starts.lock().unwrap(), vec![a, d, b, c]);
        drop(queue);
        drop(dir);
    }

    #[test]
    fn dropping_the_queue_stops_the_dispatcher() {
        let r = rig(RigOptions::default());
        let inner = Arc::clone(&r.queue.inner);
        drop(r.queue);
        eventually(
            || inner.shutdown.load(Ordering::Acquire),
            "Verteiler beendet",
        );
        // Ein Weckruf nach dem Ende ist harmlos.
        inner.notify();
    }
}
