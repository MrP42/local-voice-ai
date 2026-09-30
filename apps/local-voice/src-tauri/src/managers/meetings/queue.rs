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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

use super::final_pass::{ram_need_mb, Hardware};
use super::job::{JobPhase, MeetingJobs};
use super::queue_store::{CancelWaiting, QueueRow, STATE_RUNNING, STATE_WAITING};
use super::store::{Meeting, MeetingStore};

/// Mehr als drei gleichzeitige Transkriptionen bietet die Einstellung nicht.
pub const MAX_PARALLEL: usize = 3;
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
    ram_need_mb(size_mb) + crate::process_guard::RAM_RESERVE_MB
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
        let limit = (self.inner.deps.limit)().clamp(1, MAX_PARALLEL);
        let st = self.inner.lock();
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
                let decision = {
                    let mut gate = || self.extra_gate_open(&mut st);
                    decide(true, &occ, limit, recording, &mut gate)
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
                            Ok(true) => self.spawn_run(&mut st, &row.meeting_id, slot),
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
            let snapshot = build_snapshot(&rows, &st, limit);
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

    fn spawn_run(self: &Arc<Self>, st: &mut State, meeting_id: &str, slot: Slot) {
        let stop_requested = Arc::new(AtomicBool::new(false));
        st.running.insert(
            meeting_id.to_string(),
            RunInfo {
                slot,
                held: false,
                started: Instant::now(),
                stop_requested: Arc::clone(&stop_requested),
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
            move || inner.run_thread(id, slot, stop_requested)
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
        let request = RunRequest {
            meeting_id: meeting_id.clone(),
            source_path: source.unwrap_or_default(),
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
        slot,
        engine_ready,
        stop_requested,
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
        let threads = extra_engine_threads();
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
    let result = super::import::run_import(app, store, tm, &meeting_id, &path, job.handle());
    drop(job);
    match result {
        Ok(()) => RunOutcome::Done,
        Err(e) => RunOutcome::Failed(e),
    }
}

/// CPU-Threads einer weiteren Engine: die Haelfte der Rechen-Threads des
/// Systemschutzes (`process_guard::cpu_threads_for`), mindestens 2. Zwei
/// Engines mit allen Kernen je wuerden sich gegenseitig ausbremsen und den
/// Rechner bedienunfaehig machen.
fn extra_engine_threads() -> i32 {
    let base = crate::process_guard::cpu_threads_for(crate::process_guard::logical_cpus());
    (base / 2).max(2) as i32
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
                    clamp_limit(crate::settings::get_settings(&limit_app).meeting_import_parallel)
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
            ram_need_mb(1_600) + crate::process_guard::RAM_RESERVE_MB
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
    fn the_limit_is_one_to_three() {
        assert_eq!(clamp_limit(0), 1);
        assert_eq!(clamp_limit(1), 1);
        assert_eq!(clamp_limit(2), 2);
        assert_eq!(clamp_limit(3), 3);
        assert_eq!(clamp_limit(99), 3);
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
        gate: bool,
        /// Die Extra-Laeufe melden `engine_ready` selbst (Test des Ladens).
        manual_engine_ready: bool,
        /// Die Laeufe melden sich beim Auftragsverzeichnis mit Phase Transkription.
        register_jobs: bool,
        tick: Duration,
    }

    impl Default for RigOptions {
        fn default() -> Self {
            Self {
                limit: 1,
                gate: true,
                manual_engine_ready: false,
                register_jobs: false,
                tick: Duration::from_millis(20),
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

        let runner: Runner = {
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
            tick: options.tick,
            cooldown: Duration::from_millis(150),
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

    fn eventually(mut cond: impl FnMut() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(Instant::now() < deadline, "Zeit abgelaufen: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Ein Takt lang nichts Neues: fuer "es beginnt NICHT".
    fn settle() {
        std::thread::sleep(Duration::from_millis(250));
    }

    #[test]
    fn files_run_one_after_the_other_in_the_order_they_were_added() {
        let r = rig(RigOptions::default());
        let (a, b, c) = (r.add("a"), r.add("b"), r.add("c"));
        eventually(|| r.started() == vec![a.clone()], "a beginnt");
        settle();
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
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        settle();
        assert_eq!(r.started(), vec![a.clone(), b.clone()], "c wartet");
        assert_eq!(r.slot_of(&a), Slot::Primary);
        assert_eq!(r.slot_of(&b), Slot::Extra);
        assert_eq!(r.peak.load(Ordering::SeqCst), 2);

        // Endet die gemeinsame Engine zuerst, nimmt c sie (kein dritter Lauf).
        r.finish(&a, Release::Done);
        eventually(|| r.started().len() == 3, "c beginnt");
        assert_eq!(r.slot_of(&c), Slot::Primary);
        r.finish(&b, Release::Done);
        r.finish(&c, Release::Done);
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        settle();
        assert_eq!(r.started().len(), 3, "der vierte wartet");
        assert_eq!(r.peak.load(Ordering::SeqCst), 3);
        assert_eq!(r.queue.snapshot().waiting, vec![ids[3].clone()]);
        for id in &ids[..3] {
            r.finish(id, Release::Done);
        }
        eventually(|| r.started().len() == 4, "der vierte beginnt");
        r.finish(&ids[3], Release::Done);
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        settle();
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
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        settle();
        assert_eq!(
            r.started().len(),
            3,
            "zwei laufen noch, Limit 1: der vierte wartet"
        );
        r.finish(&ids[1], Release::Done);
        settle();
        assert_eq!(r.started().len(), 3);
        r.finish(&ids[2], Release::Done);
        eventually(|| r.started().len() == 4, "jetzt beginnt der vierte");
        r.finish(&ids[3], Release::Done);
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
    }

    #[test]
    fn a_stop_before_the_job_exists_is_not_lost() {
        // `remove` setzt das Merkmal; der Lauf sieht es, auch wenn es noch keinen Auftrag gab.
        let r = rig(RigOptions::default());
        let a = r.add("a");
        eventually(|| r.started() == vec![a.clone()], "a beginnt");
        assert_eq!(r.queue.remove(&a).unwrap(), RemoveOutcome::Stopping);
        eventually(|| r.stop_seen.lock().unwrap().contains(&a), "Stopp gesehen");
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        settle();
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
        settle();
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
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
    }

    #[test]
    fn a_recording_that_starts_before_the_first_file_holds_it_back() {
        let r = rig(RigOptions::default());
        r.recording.store(true, Ordering::SeqCst);
        let a = r.add("a");
        settle();
        assert!(
            r.started().is_empty(),
            "waehrend der Aufnahme beginnt nichts"
        );
        assert_eq!(r.queue.snapshot().blocked, Some(WaitReason::Recording));
        assert_eq!(r.status(&a), "queued");
        r.recording.store(false, Ordering::SeqCst);
        eventually(|| r.started() == vec![a.clone()], "danach beginnt sie");
        r.finish(&a, Release::Done);
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
    }

    #[test]
    fn a_foreign_job_on_the_shared_engine_holds_the_queue_back() {
        // Enddurchlauf/Nachholen/Neu-Transkription (Phase Transkription) eines
        // Auftrags, der nicht zur Warteschlange gehoert: die Engine ist belegt.
        let r = rig(RigOptions::default());
        let foreign = r.jobs.try_start("fremd", no_emit()).unwrap();
        foreign.handle().begin_phase(JobPhase::FinalPass, 1_000);
        let a = r.add("a");
        settle();
        assert!(
            r.started().is_empty(),
            "die gemeinsame Engine gehoert dem Enddurchlauf"
        );
        assert_eq!(r.queue.snapshot().blocked, Some(WaitReason::Slot));
        drop(foreign);
        eventually(|| r.started() == vec![a.clone()], "danach beginnt sie");
        r.finish(&a, Release::Done);
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
    }

    #[test]
    fn a_final_pass_that_has_no_job_yet_still_holds_the_shared_engine() {
        // Die Wiederherstellung startet einen Thread, dessen Auftrag erst einen
        // Augenblick spaeter im Verzeichnis steht: die Warteschlange darf in
        // dieser Luecke nicht auf derselben Engine beginnen.
        let r = rig(RigOptions::default());
        r.shared.store(true, Ordering::SeqCst);
        let a = r.add("a");
        settle();
        assert!(r.started().is_empty());
        r.shared.store(false, Ordering::SeqCst);
        eventually(|| r.started() == vec![a.clone()], "danach beginnt sie");
        r.finish(&a, Release::Done);
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
    }

    #[test]
    fn a_deferred_extra_engine_puts_the_file_back_in_its_place_and_retries_later() {
        let r = rig(RigOptions {
            limit: 2,
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
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
        settle();
        assert_eq!(
            r.started(),
            vec![a.clone(), b.clone()],
            "c wartet, bis b seine Engine geladen hat (sonst wuerden zwei Tore auf denselben Speicher schauen)"
        );
        // Die Engine von b ist da: jetzt darf c laden.
        // (Das Attrappen-`engine_ready` haengt am Request; wir holen es ueber ein neues Signal.)
        r.queue.inner.engine_loaded(&b);
        eventually(|| r.started().len() == 3, "c beginnt");
        assert_eq!(r.slot_of(&c), Slot::Extra);
        for id in [&a, &b, &c] {
            r.finish(id, Release::Done);
        }
        assert!(r.queue.wait_idle(Duration::from_secs(5)));
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
            tick: Duration::from_millis(20),
            cooldown: Duration::from_millis(50),
        });
        assert!(queue.wait_idle(Duration::from_secs(10)));
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
