//! P8a (Issue #59): Fortschritt, Pause und Stopp fuer die Verarbeitung einer
//! Besprechung.
//!
//! Jede laengere Verarbeitung (Datei-Import, Enddurchlauf nach dem Stopp,
//! Neu-Transkription, Sprechertrennung, Nachholen nach einem Absturz, KI-
//! Notizen) meldet hier ihren Fortschritt als Anteil der verarbeiteten
//! Audiodauer und kann angehalten oder gestoppt werden.
//!
//! Drei Bausteine, jeder ohne Tauri, Modell oder Geraet testbar:
//!
//! - [`ProgressTracker`]: Anteil, Laufzeit (ohne Pausen) und geschaetzte
//!   Restdauer aus der bisherigen Geschwindigkeit (exponentiell geglaettet,
//!   erst nach kurzer Anlaufzeit).
//! - [`Control`]: die Zustandsmaschine Laufen / Pausieren / Stoppen. Rein, ohne
//!   Sperren; [`JobHandle`] legt Mutex und Condvar darum.
//! - [`MeetingJobs`]: das Verzeichnis der laufenden Auftraege je Besprechung
//!   (ein Auftrag je Besprechung), aus dem die Befehle der Oberflaeche pausieren,
//!   fortsetzen und stoppen.
//!
//! Pause und Stopp sind kooperativ: der Auftrag fragt zwischen zwei Bloecken
//! ([`JobHandle::checkpoint`]) nach. Ein laufender Modellaufruf wird nicht
//! unterbrochen; die Reaktionszeit ist hoechstens ein Block (Import: 60 s Audio,
//! Enddurchlauf: hoechstens 25 s). Es wird nie ein Prozess angehalten oder
//! eingefroren: Pause heisst "kein neuer Block", die Kindprozesse (ffmpeg) sind
//! dann schon beendet oder gehoeren zur Phase, die nicht pausierbar ist. Nichts
//! hier laeuft im Audio-Callback: Aufrufer sind Job-Threads.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;

use super::recorder::MeetingEvent;

/// Hoechstens so oft geht ein Fortschrittsereignis je Auftrag hinaus (2 / s).
pub const MIN_EMIT_INTERVAL: Duration = Duration::from_millis(500);
/// Erst nach so viel Laufzeit (ohne Pausen) gibt es eine Restdauer ...
pub const ETA_WARMUP_MS: u64 = 8_000;
/// ... und erst, wenn mindestens dieser Anteil geschafft ist.
pub const ETA_MIN_FRACTION: f64 = 0.02;
/// Fortschritt wird zu Geschwindigkeitsproben von mindestens dieser Laenge
/// zusammengefasst: viele kleine Schritte in kurzer Zeit rauschen sonst.
const RATE_SAMPLE_MS: u64 = 1_000;
/// Gewicht der neuesten Probe im geglaetteten Wert (0 = nie, 1 = nur sie).
const RATE_ALPHA: f64 = 0.2;
/// So oft prueft ein angehaltener Auftrag, ob er weiter soll (Stopp kommt
/// ueber die Condvar sofort; nur "neue Aufnahme" braucht diese Abfrage).
const PAUSE_POLL: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------------
// Phasen, Zustand, Ereignis
// ---------------------------------------------------------------------------

/// Was ein Auftrag gerade tut. Die Oberflaeche benennt daran den Fortschritt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobPhase {
    /// Audio lesen und dekodieren (Import); Groesse noch unbekannt.
    Prepare,
    /// Transkription in Bloecken (Import, Neu-Transkription, Nachholen).
    Transcription,
    /// Enddurchlauf nach dem Stopp einer Aufnahme.
    FinalPass,
    /// Sprechertrennung (ein Modelllauf je Kanal).
    Speakers,
    /// KI-Notizen (Schritte statt Audiodauer).
    Notes,
    /// Protokoll (Bloecke statt Audiodauer).
    Minutes,
}

impl JobPhase {
    /// Nur Phasen aus vielen kleinen Bloecken lassen sich anhalten. Die
    /// Sprechertrennung ist ein einziger Modelllauf je Kanal, das Lesen der
    /// Datei ein Kindprozess: dort waere "Pause" ein leeres Versprechen.
    /// Notizen und Protokoll sind es nur, wenn sie in Bloecken laufen; das
    /// meldet der Lauf selbst ([`JobHandle::begin_phase_ex`]).
    pub fn pausable(self) -> bool {
        matches!(
            self,
            JobPhase::Transcription | JobPhase::FinalPass | JobPhase::Notes | JobPhase::Minutes
        )
    }
}

/// Zustand des Auftrags fuer die Anzeige.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobRunState {
    Running,
    /// Pause ist verlangt, der laufende Block wird noch fertig.
    Pausing,
    Paused,
    /// Stopp ist verlangt, der laufende Block wird noch fertig.
    Stopping,
}

/// Fortschritt eines Auftrags: Ereignis und Abfrage haben dieselben Felder.
/// `done`/`total` zaehlen Millisekunden Audio, in den Phasen `notes` und
/// `minutes` Schritte (Bloecke).
/// `total == 0`: Groesse (noch) unbekannt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct JobProgress {
    pub meeting_id: String,
    pub phase: JobPhase,
    pub done: u64,
    pub total: u64,
    /// Laufzeit des ganzen Auftrags ohne Pausen.
    pub elapsed_ms: u64,
    /// Geschaetzte Restdauer der Phase; `None` in der Anlaufzeit.
    pub eta_ms: Option<u64>,
    pub state: JobRunState,
    pub pausable: bool,
}

impl JobProgress {
    pub fn into_event(self) -> MeetingEvent {
        MeetingEvent::Progress {
            meeting_id: self.meeting_id,
            phase: self.phase,
            done: self.done,
            total: self.total,
            elapsed_ms: self.elapsed_ms,
            eta_ms: self.eta_ms,
            state: self.state,
            pausable: self.pausable,
        }
    }
}

/// Fehler der Steuerbefehle; `code()` geht als Text an die Oberflaeche.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobError {
    /// Fuer diese Besprechung laeuft nichts (mehr).
    NoJob,
    /// Die aktuelle Phase laesst sich nicht anhalten.
    NotPausable,
    /// Der Auftrag wird schon gestoppt.
    Stopping,
    /// Fuer diese Besprechung laeuft schon ein Auftrag.
    Busy,
}

impl JobError {
    pub fn code(self) -> &'static str {
        match self {
            JobError::NoJob => "no_job",
            JobError::NotPausable => "not_pausable",
            JobError::Stopping => "job_stopping",
            JobError::Busy => "job_busy",
        }
    }
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

// ---------------------------------------------------------------------------
// Fortschrittsrechnung (rein)
// ---------------------------------------------------------------------------

/// Anteil, Laufzeit und Restdauer einer Phase. Die Zeit kommt von aussen
/// (`now`), damit Tests ohne Warten rechnen.
#[derive(Clone, Debug)]
pub struct ProgressTracker {
    total: u64,
    done: u64,
    started: Instant,
    paused_total: Duration,
    paused_since: Option<Instant>,
    /// Laufzeit ohne Pausen beim letzten Fortschritt.
    last_update_ms: u64,
    /// Laufzeit und Stand der letzten Geschwindigkeitsprobe.
    sample_ms: u64,
    sample_done: u64,
    /// Geglaettete Geschwindigkeit in Einheiten je ms.
    rate: Option<f64>,
}

impl ProgressTracker {
    pub fn new(total: u64, now: Instant) -> Self {
        Self {
            total,
            done: 0,
            started: now,
            paused_total: Duration::ZERO,
            paused_since: None,
            last_update_ms: 0,
            sample_ms: 0,
            sample_done: 0,
            rate: None,
        }
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn done(&self) -> u64 {
        self.done
    }

    /// Die Groesse wird erst unterwegs bekannt (Nachholen, Sprecher je Kanal).
    pub fn set_total(&mut self, total: u64) {
        self.total = total;
        if total > 0 {
            self.done = self.done.min(total);
        }
    }

    /// Laufzeit ohne die Zeit in der Pause.
    pub fn active_ms(&self, now: Instant) -> u64 {
        let mut paused = self.paused_total;
        if let Some(since) = self.paused_since {
            paused += now.saturating_duration_since(since);
        }
        now.saturating_duration_since(self.started)
            .saturating_sub(paused)
            .as_millis() as u64
    }

    /// Neuer Stand. Nie rueckwaerts, nie ueber `total`.
    pub fn update(&mut self, done: u64, now: Instant) {
        let done = if self.total > 0 {
            done.min(self.total)
        } else {
            done
        };
        if done < self.done {
            return;
        }
        let active = self.active_ms(now);
        self.done = done;
        self.last_update_ms = active;
        let dt = active.saturating_sub(self.sample_ms);
        if dt >= RATE_SAMPLE_MS && done > self.sample_done {
            let instant = (done - self.sample_done) as f64 / dt as f64;
            self.rate = Some(match self.rate {
                None => instant,
                Some(previous) => previous + RATE_ALPHA * (instant - previous),
            });
            self.sample_ms = active;
            self.sample_done = done;
        }
    }

    /// Ab jetzt zaehlt die Zeit nicht mehr (idempotent).
    pub fn pause(&mut self, now: Instant) {
        if self.paused_since.is_none() {
            self.paused_since = Some(now);
        }
    }

    /// Die Zeit laeuft wieder (idempotent).
    pub fn resume(&mut self, now: Instant) {
        if let Some(since) = self.paused_since.take() {
            self.paused_total += now.saturating_duration_since(since);
        }
    }

    /// Geschaetzte Restdauer in ms. `None`: Groesse unbekannt oder noch in der
    /// Anlaufzeit (weniger als [`ETA_WARMUP_MS`] Laufzeit oder weniger als
    /// [`ETA_MIN_FRACTION`] geschafft); danach aus der geglaetteten
    /// Geschwindigkeit, abzueglich der Zeit seit dem letzten Fortschritt.
    pub fn eta_ms(&self, now: Instant) -> Option<u64> {
        if self.total == 0 {
            return None;
        }
        if self.done >= self.total {
            return Some(0);
        }
        let active = self.active_ms(now);
        if active < ETA_WARMUP_MS || (self.done as f64) < ETA_MIN_FRACTION * self.total as f64 {
            return None;
        }
        let rate = self
            .rate
            .or_else(|| (active > 0 && self.done > 0).then(|| self.done as f64 / active as f64))?;
        if rate.is_nan() || rate <= 0.0 {
            return None;
        }
        let remaining = (self.total - self.done) as f64 / rate;
        let since = active.saturating_sub(self.last_update_ms) as f64;
        Some((remaining - since).max(0.0) as u64)
    }
}

/// Begrenzt Ereignisse auf [`MIN_EMIT_INTERVAL`].
#[derive(Debug, Default)]
pub struct EmitGate {
    last: Option<Instant>,
}

impl EmitGate {
    /// `true` (und merkt sich den Zeitpunkt), wenn ein Ereignis hinausgehen darf.
    pub fn allow(&mut self, now: Instant) -> bool {
        match self.last {
            Some(last) if now.saturating_duration_since(last) < MIN_EMIT_INTERVAL => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }

    /// Ein Ereignis geht in jedem Fall hinaus (Phase, Pause, Ende): Zeitpunkt merken.
    pub fn mark(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

// ---------------------------------------------------------------------------
// Zustandsmaschine (rein)
// ---------------------------------------------------------------------------

/// Was ein Aufrufer am Kontrollpunkt tun soll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Weiter mit dem naechsten Block; `resumed`: eben aus einer Pause.
    Go { resumed: bool },
    /// Der Nutzer hat gestoppt.
    Stopped,
    /// Von aussen abgebrochen (eine neue Aufnahme will die Engine).
    Cancelled,
}

/// Ein Schritt der Zustandsmaschine am Kontrollpunkt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Done(Gate),
    /// Warten; `announce`: der Uebergang nach "pausiert" ist neu (Ereignis).
    Wait { announce: bool },
}

/// Pause / Stopp eines Auftrags ohne Sperren und ohne Zeit.
///
/// Uebergaenge: Pause und Fortsetzen sind idempotent; Stopp gewinnt gegen
/// alles (auch aus der Pause) und ist selbst idempotent; nach einem Stopp
/// lehnt Pause und Fortsetzen ab; Fortsetzen ohne Pause tut nichts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Control {
    pub pause_requested: bool,
    /// Der Auftrag steht tatsaechlich an einem Kontrollpunkt.
    pub blocked: bool,
    pub stop: bool,
}

impl Control {
    pub fn run_state(&self) -> JobRunState {
        if self.stop {
            JobRunState::Stopping
        } else if self.blocked {
            JobRunState::Paused
        } else if self.pause_requested {
            JobRunState::Pausing
        } else {
            JobRunState::Running
        }
    }

    pub fn request_pause(&mut self, pausable: bool) -> Result<(), JobError> {
        if self.stop {
            return Err(JobError::Stopping);
        }
        if self.pause_requested {
            return Ok(());
        }
        if !pausable {
            return Err(JobError::NotPausable);
        }
        self.pause_requested = true;
        Ok(())
    }

    pub fn request_resume(&mut self) -> Result<(), JobError> {
        if self.stop {
            return Err(JobError::Stopping);
        }
        self.pause_requested = false;
        Ok(())
    }

    pub fn request_stop(&mut self) {
        self.stop = true;
    }

    /// Ein Kontrollpunkt: stoppen, abbrechen, weiterlaufen oder warten.
    pub fn step(&mut self, external_cancel: bool) -> Step {
        if self.stop {
            self.blocked = false;
            return Step::Done(Gate::Stopped);
        }
        if external_cancel {
            self.blocked = false;
            return Step::Done(Gate::Cancelled);
        }
        if !self.pause_requested {
            let resumed = self.blocked;
            self.blocked = false;
            return Step::Done(Gate::Go { resumed });
        }
        let announce = !self.blocked;
        self.blocked = true;
        Step::Wait { announce }
    }
}

// ---------------------------------------------------------------------------
// Auftrag
// ---------------------------------------------------------------------------

/// Wohin Ereignisse gehen: in der App `MeetingEvent::emit`, in Tests ein Puffer.
pub type EmitFn = Arc<dyn Fn(MeetingEvent) + Send + Sync>;

/// Der Weg der App: jedes Ereignis an die Fenster.
pub fn app_emit(app: &tauri::AppHandle) -> EmitFn {
    use tauri_specta::Event;
    let app = app.clone();
    Arc::new(move |event: MeetingEvent| {
        let _ = event.emit(&app);
    })
}

struct Ctl {
    phase: JobPhase,
    /// Ob die laufende Phase sich anhalten laesst (Voreinstellung der Phase,
    /// vom Lauf ueberschreibbar: Notizen im Einzeldurchlauf sind ein Aufruf).
    pausable: bool,
    tracker: ProgressTracker,
    /// Laufzeit der abgeschlossenen Phasen.
    elapsed_before_ms: u64,
    control: Control,
    gate: EmitGate,
}

/// Ein laufender Auftrag: Fortschritt, Pause, Stopp. Wird als `Arc` zwischen
/// dem Job-Thread und den Befehlen geteilt.
pub struct JobHandle {
    meeting_id: String,
    emit: EmitFn,
    ctl: Mutex<Ctl>,
    cvar: Condvar,
    /// Spiegel von `Control::stop` fuer Verbraucher, die nur ein Flag kennen
    /// (der Diarisierer beendet damit einen laufenden Modelllauf).
    stop_flag: Arc<AtomicBool>,
    /// Weckt asynchrone Wartende (`select!` gegen [`JobHandle::stopped`]).
    stop_notify: tokio::sync::Notify,
}

impl JobHandle {
    /// Neuer Auftrag in der Phase `Prepare` ohne bekannte Groesse. Meldet sich
    /// nicht selbst: das erste Ereignis kommt mit [`JobHandle::begin_phase`].
    pub fn new(meeting_id: &str, emit: EmitFn) -> Arc<Self> {
        Arc::new(Self {
            meeting_id: meeting_id.to_string(),
            emit,
            ctl: Mutex::new(Ctl {
                phase: JobPhase::Prepare,
                pausable: JobPhase::Prepare.pausable(),
                tracker: ProgressTracker::new(0, Instant::now()),
                elapsed_before_ms: 0,
                control: Control::default(),
                gate: EmitGate::default(),
            }),
            cvar: Condvar::new(),
            stop_flag: Arc::new(AtomicBool::new(false)),
            stop_notify: tokio::sync::Notify::new(),
        })
    }

    pub fn meeting_id(&self) -> &str {
        &self.meeting_id
    }

    /// Die Phase, in der der Auftrag gerade ist.
    pub fn phase(&self) -> JobPhase {
        self.lock().phase
    }

    fn lock(&self) -> MutexGuard<'_, Ctl> {
        self.ctl.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn progress_of(&self, ctl: &Ctl, now: Instant) -> JobProgress {
        JobProgress {
            meeting_id: self.meeting_id.clone(),
            phase: ctl.phase,
            done: ctl.tracker.done(),
            total: ctl.tracker.total(),
            elapsed_ms: ctl.elapsed_before_ms + ctl.tracker.active_ms(now),
            eta_ms: ctl.tracker.eta_ms(now),
            state: ctl.control.run_state(),
            pausable: ctl.pausable,
        }
    }

    fn publish(&self, progress: JobProgress) {
        (self.emit)(progress.into_event());
    }

    /// Abfrage des aktuellen Stands (fuer eine Oberflaeche, die spaeter dazukommt).
    pub fn snapshot(&self) -> JobProgress {
        let ctl = self.lock();
        self.progress_of(&ctl, Instant::now())
    }

    /// Eine neue Phase mit ihrer Groesse (0 = unbekannt). Geht sofort hinaus.
    pub fn begin_phase(&self, phase: JobPhase, total: u64) {
        self.begin_phase_ex(phase, total, phase.pausable());
    }

    /// Wie [`JobHandle::begin_phase`], aber der Lauf sagt selbst, ob er sich
    /// anhalten laesst: Notizen und Protokoll im Einzeldurchlauf sind ein
    /// einziger Modellaufruf, dort gaebe es keinen Kontrollpunkt.
    pub fn begin_phase_ex(&self, phase: JobPhase, total: u64, pausable: bool) {
        let now = Instant::now();
        let progress = {
            let mut ctl = self.lock();
            ctl.elapsed_before_ms += ctl.tracker.active_ms(now);
            ctl.phase = phase;
            ctl.pausable = pausable;
            ctl.tracker = ProgressTracker::new(total, now);
            if ctl.control.blocked {
                ctl.tracker.pause(now);
            }
            ctl.gate.mark(now);
            self.progress_of(&ctl, now)
        };
        self.publish(progress);
    }

    /// Die Groesse der laufenden Phase wird bekannt.
    pub fn set_total(&self, total: u64) {
        let now = Instant::now();
        let progress = {
            let mut ctl = self.lock();
            ctl.tracker.set_total(total);
            ctl.gate.mark(now);
            self.progress_of(&ctl, now)
        };
        self.publish(progress);
    }

    /// Neuer Stand der Phase. Ereignisse hoechstens 2 / s; der Schritt ans
    /// Ende geht immer hinaus.
    pub fn advance(&self, done: u64) {
        let now = Instant::now();
        let progress = {
            let mut ctl = self.lock();
            ctl.tracker.update(done, now);
            let finished = ctl.tracker.total() > 0 && ctl.tracker.done() >= ctl.tracker.total();
            let due = ctl.gate.allow(now);
            if due || finished {
                ctl.gate.mark(now);
                Some(self.progress_of(&ctl, now))
            } else {
                None
            }
        };
        if let Some(progress) = progress {
            self.publish(progress);
        }
    }

    /// Eine Auswertung des Kontrollpunkts, ohne zu warten. `Some`: der Aufruf
    /// darf weiter oder muss enden; `None`: angehalten, weiter warten. Meldet
    /// den Wechsel nach "pausiert" und zurueck.
    fn checkpoint_once(&self, external_cancel: bool) -> Option<Gate> {
        let now = Instant::now();
        let (gate, progress) = {
            let mut ctl = self.lock();
            let was_blocked = ctl.control.blocked;
            match ctl.control.step(external_cancel) {
                Step::Done(gate) => {
                    // War angehalten: die Zeit laeuft wieder, die Anzeige folgt.
                    let progress = was_blocked.then(|| {
                        ctl.tracker.resume(now);
                        ctl.gate.mark(now);
                        self.progress_of(&ctl, now)
                    });
                    (Some(gate), progress)
                }
                Step::Wait { announce } => {
                    let progress = announce.then(|| {
                        ctl.tracker.pause(now);
                        ctl.gate.mark(now);
                        self.progress_of(&ctl, now)
                    });
                    (None, progress)
                }
            }
        };
        if let Some(progress) = progress {
            self.publish(progress);
        }
        gate
    }

    /// Kontrollpunkt zwischen zwei Bloecken (Job-Thread). Ist eine Pause
    /// verlangt, wartet der Aufruf hier (Ereignis "pausiert"), bis fortgesetzt,
    /// gestoppt oder `external_cancel` wahr wird. Ein Stopp gewinnt immer.
    pub fn checkpoint(&self, external_cancel: &dyn Fn() -> bool) -> Gate {
        loop {
            if let Some(gate) = self.checkpoint_once(external_cancel()) {
                return gate;
            }
            let ctl = self.lock();
            // Ein Wecken zwischen der Auswertung und hier kostet hoechstens
            // PAUSE_POLL; die Pruefung darunter faengt den haeufigen Fall ab.
            if ctl.control.stop || !ctl.control.pause_requested {
                continue;
            }
            let _ = self
                .cvar
                .wait_timeout(ctl, PAUSE_POLL)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Wie [`JobHandle::checkpoint`] fuer asynchronen Code: wartet mit
    /// `tokio::time::sleep`, blockiert also keinen Worker der Laufzeit.
    pub async fn checkpoint_async<F: Fn() -> bool>(&self, external_cancel: F) -> Gate {
        loop {
            if let Some(gate) = self.checkpoint_once(external_cancel()) {
                return gate;
            }
            tokio::time::sleep(PAUSE_POLL).await;
        }
    }

    /// Steht der Auftrag gerade an einem Kontrollpunkt (pausiert)?
    pub fn is_paused(&self) -> bool {
        self.lock().control.blocked
    }

    fn control_changed(&self) {
        let now = Instant::now();
        let progress = {
            let mut ctl = self.lock();
            ctl.gate.mark(now);
            self.progress_of(&ctl, now)
        };
        self.publish(progress);
        self.cvar.notify_all();
    }

    /// Pause verlangen: der Auftrag haelt am naechsten Kontrollpunkt an.
    pub fn pause(&self) -> Result<(), JobError> {
        {
            let mut ctl = self.lock();
            let pausable = ctl.pausable;
            ctl.control.request_pause(pausable)?;
        }
        self.control_changed();
        Ok(())
    }

    /// Fortsetzen (auch eine noch nicht wirksame Pause zurueckziehen).
    pub fn resume(&self) -> Result<(), JobError> {
        let blocked = {
            let mut ctl = self.lock();
            ctl.control.request_resume()?;
            ctl.control.blocked
        };
        if blocked {
            // Der angehaltene Auftrag meldet "laeuft" selbst, sobald er
            // aufwacht; ein eigenes Ereignis hier wuerde noch "pausiert" sagen.
            self.cvar.notify_all();
        } else {
            self.control_changed();
        }
        Ok(())
    }

    /// Stoppen: am naechsten Kontrollpunkt (auch aus der Pause). Mehrfach
    /// aufrufen ist in Ordnung. Asynchrone Laeufe brechen sofort ab, wenn sie
    /// gegen [`JobHandle::stopped`] laufen.
    pub fn stop(&self) -> Result<(), JobError> {
        {
            let mut ctl = self.lock();
            ctl.control.request_stop();
        }
        self.stop_flag.store(true, Ordering::Release);
        self.stop_notify.notify_waiters();
        self.control_changed();
        Ok(())
    }

    pub fn is_stopped(&self) -> bool {
        self.stop_flag.load(Ordering::Acquire)
    }

    /// Wird fertig, sobald gestoppt wurde (fuer `tokio::select!` gegen den Lauf).
    pub async fn stopped(&self) {
        loop {
            // Zuerst anmelden, dann pruefen: so geht kein Stopp dazwischen verloren.
            let notified = self.stop_notify.notified();
            if self.is_stopped() {
                return;
            }
            notified.await;
        }
    }

    /// Das Stopp-Flag zum Weiterreichen (Diarisierer).
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop_flag)
    }
}

// ---------------------------------------------------------------------------
// Verzeichnis der Auftraege
// ---------------------------------------------------------------------------

/// Die laufenden Auftraege, je Besprechung hoechstens einer.
#[derive(Clone, Default)]
pub struct MeetingJobs {
    map: Arc<Mutex<HashMap<String, Arc<JobHandle>>>>,
}

/// Haelt die Eintragung eines Auftrags; beim Drop (auch nach einer Panik) ist
/// der Auftrag wieder ausgetragen, damit nie ein Geisterauftrag steht.
pub struct JobGuard {
    jobs: MeetingJobs,
    handle: Arc<JobHandle>,
}

impl JobGuard {
    pub fn handle(&self) -> &Arc<JobHandle> {
        &self.handle
    }
}

impl std::ops::Deref for JobGuard {
    type Target = JobHandle;
    fn deref(&self) -> &JobHandle {
        &self.handle
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        {
            let mut map = self.jobs.lock();
            if map
                .get(self.handle.meeting_id())
                .is_some_and(|h| Arc::ptr_eq(h, &self.handle))
            {
                map.remove(self.handle.meeting_id());
            }
        }
        // Das Ende meldet sich selbst: eine Ansicht, die beim Ende gar nicht
        // offen war (Reiterwechsel), laedt daraufhin ihr Ergebnis neu.
        (self.handle.emit)(MeetingEvent::JobEnded {
            meeting_id: self.handle.meeting_id().to_string(),
            phase: self.handle.phase(),
            stopped: self.handle.is_stopped(),
        });
    }
}

impl MeetingJobs {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Arc<JobHandle>>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Meldet einen Auftrag an. `Busy`, wenn fuer die Besprechung schon einer
    /// laeuft: zwei Verarbeitungen derselben Besprechung schreiben sonst
    /// gleichzeitig ins Transkript.
    pub fn try_start(&self, meeting_id: &str, emit: EmitFn) -> Result<JobGuard, JobError> {
        let mut map = self.lock();
        if map.contains_key(meeting_id) {
            return Err(JobError::Busy);
        }
        let handle = JobHandle::new(meeting_id, emit);
        map.insert(meeting_id.to_string(), Arc::clone(&handle));
        Ok(JobGuard {
            jobs: self.clone(),
            handle,
        })
    }

    fn get(&self, meeting_id: &str) -> Result<Arc<JobHandle>, JobError> {
        self.lock()
            .get(meeting_id)
            .cloned()
            .ok_or(JobError::NoJob)
    }

    pub fn pause(&self, meeting_id: &str) -> Result<(), JobError> {
        self.get(meeting_id)?.pause()
    }

    pub fn resume(&self, meeting_id: &str) -> Result<(), JobError> {
        self.get(meeting_id)?.resume()
    }

    pub fn stop(&self, meeting_id: &str) -> Result<(), JobError> {
        self.get(meeting_id)?.stop()
    }

    pub fn is_running(&self, meeting_id: &str) -> bool {
        self.lock().contains_key(meeting_id)
    }

    /// Stand aller laufenden Auftraege (Hydrierung der Oberflaeche).
    pub fn snapshots(&self) -> Vec<JobProgress> {
        let handles: Vec<Arc<JobHandle>> = self.lock().values().cloned().collect();
        let mut out: Vec<JobProgress> = handles.iter().map(|h| h.snapshot()).collect();
        out.sort_by(|a, b| a.meeting_id.cmp(&b.meeting_id));
        out
    }
}

/// Das Verzeichnis der App (ein Prozess, ein Verzeichnis): Befehle, Import,
/// Enddurchlauf und der Headless-Pfad finden darueber denselben Auftrag.
pub fn global() -> &'static MeetingJobs {
    static JOBS: OnceLock<MeetingJobs> = OnceLock::new();
    JOBS.get_or_init(MeetingJobs::new)
}

// ---------------------------------------------------------------------------
// Schnittstelle fuer asynchrone Laeufe (KI-Notizen, Protokoll)
// ---------------------------------------------------------------------------
//
// Ein Lauf, der schon `async` ist und tief verschachtelt Bloecke abarbeitet
// (Notizen, Protokoll), bekommt seinen Auftrag nicht durch jede Signatur
// gereicht, sondern als Task-Variable: der Aufrufer schreibt
// `job::scope(job, lauf).await`, der Lauf ruft `job::report_*` und
// `job::checkpoint().await`. Ohne Auftrag (Tests, Headless) sind alle Aufrufe
// wirkungslos und der Kontrollpunkt laesst durch.

tokio::task_local! {
    static CURRENT: Arc<JobHandle>;
}

/// Fuehrt `fut` mit `job` als aktuellem Auftrag aus.
pub async fn scope<F: std::future::Future>(job: Arc<JobHandle>, fut: F) -> F::Output {
    CURRENT.scope(job, fut).await
}

/// Der Auftrag des laufenden Tasks, falls es einen gibt.
pub fn current() -> Option<Arc<JobHandle>> {
    CURRENT.try_with(Arc::clone).ok()
}

/// Meldet eine neue Phase des aktuellen Auftrags (`total` 0 = unbekannt).
/// `pausable`: hat der Lauf Kontrollpunkte (mehrere Bloecke)?
// Schnittstelle fuer das Protokoll (P1k): bis dahin ohne Aufrufer.
#[allow(dead_code)]
pub fn report_phase(phase: JobPhase, total: u64, pausable: bool) {
    if let Some(job) = current() {
        job.begin_phase_ex(phase, total, pausable);
    }
}

/// Meldet die Groesse der laufenden Phase.
#[allow(dead_code)]
pub fn report_total(total: u64) {
    if let Some(job) = current() {
        job.set_total(total);
    }
}

/// Meldet den Stand (`done` von `total`, in Bloecken oder Schritten).
#[allow(dead_code)]
pub fn report_done(done: u64) {
    if let Some(job) = current() {
        job.advance(done);
    }
}

/// Kontrollpunkt zwischen zwei Bloecken: haelt bei Pause an, gibt bei Stopp
/// `Gate::Stopped`. Ohne Auftrag: `Go`.
pub async fn checkpoint() -> Gate {
    match current() {
        Some(job) => job.checkpoint_async(|| false).await,
        None => Gate::Go { resumed: false },
    }
}

/// Hat der Nutzer den aktuellen Auftrag gestoppt? (Abbruch-Merker fuer Code,
/// der kein Future abwarten kann.)
#[allow(dead_code)]
pub fn stop_requested() -> bool {
    current().is_some_and(|job| job.is_stopped())
}

/// Das Zeitlimit ist abgelaufen.
#[derive(Debug, PartialEq, Eq)]
pub struct TimedOut;

/// Wie `tokio::time::timeout`, aber Zeit in der Pause zaehlt nicht mit: ein
/// Lauf, der eine Stunde angehalten war, ist nicht "zu lang" gelaufen.
pub async fn timeout_excluding_pauses<F: std::future::Future>(
    limit: Duration,
    fut: F,
) -> Result<F::Output, TimedOut> {
    let Some(job) = current() else {
        return tokio::time::timeout(limit, fut).await.map_err(|_| TimedOut);
    };
    tokio::pin!(fut);
    let mut active = Duration::ZERO;
    let mut tick = tokio::time::interval(PAUSE_POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            out = &mut fut => return Ok(out),
            _ = tick.tick() => {
                if !job.is_paused() {
                    active += PAUSE_POLL;
                    if active >= limit {
                        return Err(TimedOut);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    // ---- Fortschrittsrechnung ------------------------------------------------

    /// 60 Minuten Audio, die Maschine schafft 60 s Audio je Sekunde (RTF 60).
    const HOUR_MS: u64 = 3_600_000;

    #[test]
    fn constant_speed_gives_the_exact_remaining_time() {
        let t0 = Instant::now();
        let mut t = ProgressTracker::new(HOUR_MS, t0);
        for s in 1..=30u64 {
            t.update(s * 60_000, at(t0, s * 1_000));
        }
        // Nach 30 s stehen 30 min: die Haelfte ist noch offen, also 30 s.
        let eta = t.eta_ms(at(t0, 30_000)).expect("nach der Anlaufzeit");
        assert!((29_000..=31_000).contains(&eta), "eta {eta}");
    }

    #[test]
    fn there_is_no_eta_during_the_warm_up() {
        let t0 = Instant::now();
        let mut t = ProgressTracker::new(HOUR_MS, t0);
        t.update(60_000 * 5, at(t0, 5_000));
        assert_eq!(t.eta_ms(at(t0, 5_000)), None, "unter 8 s Laufzeit");
        t.update(60_000 * 9, at(t0, 9_000));
        assert!(t.eta_ms(at(t0, 9_000)).is_some(), "danach ja");
    }

    #[test]
    fn there_is_no_eta_below_two_percent_even_after_the_warm_up() {
        let t0 = Instant::now();
        let mut t = ProgressTracker::new(HOUR_MS, t0);
        // 10 s Laufzeit, aber erst 1 % geschafft (sehr langsam anlaufend).
        t.update(HOUR_MS / 100, at(t0, 10_000));
        assert_eq!(t.eta_ms(at(t0, 10_000)), None);
    }

    #[test]
    fn unknown_size_has_no_eta_and_a_finished_phase_has_zero() {
        let t0 = Instant::now();
        let mut unknown = ProgressTracker::new(0, t0);
        unknown.update(50_000, at(t0, 20_000));
        assert_eq!(unknown.eta_ms(at(t0, 20_000)), None);

        let mut done = ProgressTracker::new(1_000, t0);
        done.update(1_000, at(t0, 100));
        assert_eq!(done.eta_ms(at(t0, 100)), Some(0));
    }

    #[test]
    fn pause_time_counts_neither_as_runtime_nor_as_slow_speed() {
        let t0 = Instant::now();
        let mut t = ProgressTracker::new(HOUR_MS, t0);
        for s in 1..=10u64 {
            t.update(s * 60_000, at(t0, s * 1_000));
        }
        let eta_before = t.eta_ms(at(t0, 10_000)).unwrap();
        // Fuenf Minuten Pause.
        t.pause(at(t0, 10_000));
        t.pause(at(t0, 12_000)); // doppelt: kein neuer Beginn
        assert_eq!(t.active_ms(at(t0, 310_000)), 10_000, "Uhr steht");
        t.resume(at(t0, 310_000));
        t.resume(at(t0, 311_000)); // doppelt: nichts
        assert_eq!(t.active_ms(at(t0, 310_000)), 10_000);
        // Danach geht es mit derselben Geschwindigkeit weiter.
        for s in 11..=20u64 {
            t.update(s * 60_000, at(t0, 310_000 + (s - 10) * 1_000));
        }
        let eta_after = t.eta_ms(at(t0, 320_000)).unwrap();
        assert_eq!(t.active_ms(at(t0, 320_000)), 20_000);
        // 20 min stehen: noch 40 min = 40 s. Ohne Pausenabzug waeren es Stunden.
        assert!((38_000..=42_000).contains(&eta_after), "eta {eta_after}");
        assert!(eta_before > eta_after);
    }

    #[test]
    fn smoothing_softens_a_sudden_slowdown() {
        let t0 = Instant::now();
        let mut t = ProgressTracker::new(HOUR_MS, t0);
        // 20 s mit 60 s Audio je Sekunde.
        for s in 1..=20u64 {
            t.update(s * 60_000, at(t0, s * 1_000));
        }
        let fast = t.eta_ms(at(t0, 20_000)).unwrap();
        // Eine Sekunde spaeter nur noch halbes Tempo (30 s Audio je Sekunde).
        t.update(20 * 60_000 + 30_000, at(t0, 21_000));
        let after_one = t.eta_ms(at(t0, 21_000)).unwrap();
        // Ungeglaettet waere die Schaetzung auf den doppelten Wert gesprungen.
        let remaining = HOUR_MS - (20 * 60_000 + 30_000);
        let raw_new = (remaining as f64 / 30.0) as u64; // ms bei 30 ms Audio je ms
        assert!(after_one > fast - 1_000, "geht nicht ploetzlich runter");
        assert!(
            after_one < raw_new,
            "eine langsame Probe zieht die Schaetzung nur ein Stueck: {after_one} < {raw_new}"
        );
        // Bleibt es langsam, naehert sie sich dem neuen Tempo.
        for s in 22..=60u64 {
            t.update(20 * 60_000 + (s - 20) * 30_000, at(t0, s * 1_000));
        }
        let slow = t.eta_ms(at(t0, 60_000)).unwrap();
        let remaining = HOUR_MS - (20 * 60_000 + 40 * 30_000);
        let steady = (remaining as f64 / 30.0) as u64;
        assert!(
            (steady as f64 * 0.9..steady as f64 * 1.1).contains(&(slow as f64)),
            "slow {slow} steady {steady}"
        );
    }

    #[test]
    fn alternating_fast_and_slow_blocks_keep_the_eta_calm() {
        // Stille Bloecke sind schnell, Sprache langsam: die Schaetzung darf
        // nicht im Takt der Bloecke zappeln.
        let t0 = Instant::now();
        let total = 70 * 60_000u64;
        let mut t = ProgressTracker::new(total, t0);
        let (mut now_ms, mut done) = (0u64, 0u64);
        let mut etas = Vec::new();
        for i in 0..60u64 {
            // Ein Block = 60 s Audio: 1 s (still) oder 5 s (Sprache).
            now_ms += if i % 2 == 0 { 1_000 } else { 5_000 };
            done += 60_000;
            t.update(done, at(t0, now_ms));
            if i >= 10 {
                etas.push(t.eta_ms(at(t0, now_ms)).unwrap() as f64);
            }
        }
        let steady = etas.last().copied().unwrap();
        for pair in etas.windows(2) {
            // Von einem Schritt zum naechsten aendert sich die Schaetzung um
            // hoechstens die Zeit des Schritts plus 25 % Schwankung.
            let jump = (pair[1] - pair[0]).abs();
            assert!(jump < 0.25 * pair[0] + 6_000.0, "sprunghaft: {pair:?}");
        }
        assert!(steady < etas[0], "sinkt insgesamt");
    }

    #[test]
    fn progress_is_clamped_to_the_total_and_never_runs_backwards() {
        let t0 = Instant::now();
        let mut t = ProgressTracker::new(1_000, t0);
        t.update(2_500, at(t0, 100));
        assert_eq!(t.done(), 1_000, "nie ueber das Ende");
        t.update(400, at(t0, 200));
        assert_eq!(t.done(), 1_000, "nie rueckwaerts");
    }

    #[test]
    fn the_estimate_shrinks_with_the_time_since_the_last_update() {
        let t0 = Instant::now();
        let mut t = ProgressTracker::new(HOUR_MS, t0);
        for s in 1..=20u64 {
            t.update(s * 60_000, at(t0, s * 1_000));
        }
        let now = t.eta_ms(at(t0, 20_000)).unwrap();
        let later = t.eta_ms(at(t0, 25_000)).unwrap();
        assert_eq!(now - later, 5_000);
        assert_eq!(t.eta_ms(at(t0, 2_000_000)), Some(0), "nie negativ");
    }

    #[test]
    fn set_total_after_the_start_keeps_the_progress() {
        let t0 = Instant::now();
        let mut t = ProgressTracker::new(0, t0);
        t.update(30_000, at(t0, 1_000));
        t.set_total(100_000);
        assert_eq!((t.done(), t.total()), (30_000, 100_000));
    }

    // ---- Ereignisdrossel -------------------------------------------------------

    #[test]
    fn events_go_out_at_most_twice_a_second() {
        let t0 = Instant::now();
        let mut gate = EmitGate::default();
        let allowed: Vec<u64> = [0u64, 100, 400, 499, 500, 700, 999, 1_000, 1_001, 1_600]
            .into_iter()
            .filter(|ms| gate.allow(at(t0, *ms)))
            .collect();
        assert_eq!(allowed, vec![0, 500, 1_000, 1_600]);
        // Ueber eine Minute nie mehr als 120.
        let mut gate = EmitGate::default();
        let n = (0..60_000u64).filter(|ms| gate.allow(at(t0, *ms))).count();
        assert!(n <= 120, "{n}");
    }

    // ---- Zustandsmaschine ------------------------------------------------------

    #[test]
    fn control_pause_resume_are_idempotent_and_resume_without_pause_is_a_noop() {
        let mut c = Control::default();
        assert_eq!(c.run_state(), JobRunState::Running);
        assert_eq!(c.request_resume(), Ok(()), "ohne Pause: nichts");
        assert_eq!(c.run_state(), JobRunState::Running);
        assert_eq!(c.request_pause(true), Ok(()));
        assert_eq!(c.request_pause(true), Ok(()), "doppelt");
        assert_eq!(c.run_state(), JobRunState::Pausing);
        assert_eq!(c.step(false), Step::Wait { announce: true });
        assert_eq!(c.run_state(), JobRunState::Paused);
        assert_eq!(c.step(false), Step::Wait { announce: false }, "bleibt stehen");
        assert_eq!(c.request_resume(), Ok(()));
        assert_eq!(c.step(false), Step::Done(Gate::Go { resumed: true }));
        assert_eq!(c.run_state(), JobRunState::Running);
        assert_eq!(c.step(false), Step::Done(Gate::Go { resumed: false }));
    }

    #[test]
    fn control_a_pause_can_be_withdrawn_before_it_takes_effect() {
        let mut c = Control::default();
        c.request_pause(true).unwrap();
        assert_eq!(c.run_state(), JobRunState::Pausing);
        c.request_resume().unwrap();
        assert_eq!(c.step(false), Step::Done(Gate::Go { resumed: false }));
    }

    #[test]
    fn control_stop_wins_over_everything_including_a_running_pause() {
        let mut c = Control::default();
        c.request_pause(true).unwrap();
        assert_eq!(c.step(false), Step::Wait { announce: true });
        c.request_stop();
        assert_eq!(c.run_state(), JobRunState::Stopping);
        assert_eq!(c.step(false), Step::Done(Gate::Stopped));
        assert!(!c.blocked, "nicht mehr angehalten");
        // Doppelter Stopp: derselbe Zustand, kein Fehler.
        c.request_stop();
        assert_eq!(c.step(false), Step::Done(Gate::Stopped));
        // Danach lehnen Pause und Fortsetzen ab.
        assert_eq!(c.request_pause(true), Err(JobError::Stopping));
        assert_eq!(c.request_resume(), Err(JobError::Stopping));
    }

    #[test]
    fn control_a_stop_beats_an_outside_cancel_and_both_beat_a_pause() {
        let mut c = Control::default();
        c.request_pause(true).unwrap();
        assert_eq!(c.step(true), Step::Done(Gate::Cancelled), "neue Aufnahme in der Pause");
        let mut c = Control::default();
        c.request_stop();
        assert_eq!(c.step(true), Step::Done(Gate::Stopped));
    }

    #[test]
    fn control_refuses_to_start_a_pause_in_a_phase_that_cannot_pause() {
        let mut c = Control::default();
        assert_eq!(c.request_pause(false), Err(JobError::NotPausable));
        assert_eq!(c.run_state(), JobRunState::Running);
        // Eine schon verlangte Pause bleibt auch ueber eine solche Phase hinweg
        // gueltig (sie wirkt am naechsten Kontrollpunkt).
        c.request_pause(true).unwrap();
        assert_eq!(c.request_pause(false), Ok(()));
    }

    #[test]
    fn phases_declare_what_can_be_controlled() {
        assert!(JobPhase::Transcription.pausable());
        assert!(JobPhase::FinalPass.pausable());
        assert!(JobPhase::Notes.pausable());
        assert!(JobPhase::Minutes.pausable());
        assert!(!JobPhase::Prepare.pausable());
        assert!(!JobPhase::Speakers.pausable());
    }

    // ---- JobHandle mit Threads ---------------------------------------------------

    type Events = Arc<Mutex<Vec<MeetingEvent>>>;

    fn collector() -> (EmitFn, Events) {
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        (
            Arc::new(move |e| sink.lock().unwrap().push(e)),
            events,
        )
    }

    fn progress_events(events: &Events) -> Vec<(JobPhase, u64, JobRunState)> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                MeetingEvent::Progress {
                    phase, done, state, ..
                } => Some((*phase, *done, *state)),
                _ => None,
            })
            .collect()
    }

    /// Wartet, bis der Zustand erreicht ist (der Job-Thread laeuft nebenher).
    fn wait_state(job: &JobHandle, want: JobRunState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while job.snapshot().state != want {
            assert!(Instant::now() < deadline, "Zustand {want:?} nie erreicht");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Ein Job-Thread, der je Nachricht einen Kontrollpunkt durchlaeuft.
    fn stepper(job: &Arc<JobHandle>) -> (mpsc::Sender<()>, mpsc::Receiver<Gate>, thread::JoinHandle<()>) {
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let (gate_tx, gate_rx) = mpsc::channel::<Gate>();
        let job = Arc::clone(job);
        let handle = thread::spawn(move || {
            while go_rx.recv().is_ok() {
                let gate = job.checkpoint(&|| false);
                let _ = gate_tx.send(gate);
            }
        });
        (go_tx, gate_rx, handle)
    }

    #[test]
    fn a_paused_job_waits_at_the_checkpoint_until_it_is_resumed() {
        let (emit, events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase(JobPhase::Transcription, 100_000);
        let (go, gate, worker) = stepper(&job);

        go.send(()).unwrap();
        assert_eq!(gate.recv_timeout(Duration::from_secs(2)), Ok(Gate::Go { resumed: false }));

        job.pause().unwrap();
        assert_eq!(job.snapshot().state, JobRunState::Pausing, "noch nicht angehalten");
        go.send(()).unwrap();
        assert!(
            gate.recv_timeout(Duration::from_millis(300)).is_err(),
            "der Kontrollpunkt haelt"
        );
        assert_eq!(job.snapshot().state, JobRunState::Paused);

        job.resume().unwrap();
        assert_eq!(gate.recv_timeout(Duration::from_secs(2)), Ok(Gate::Go { resumed: true }));
        assert_eq!(job.snapshot().state, JobRunState::Running);

        drop(go);
        worker.join().unwrap();
        // Ereignisse: Phase, Pause verlangt, angehalten, fortgesetzt.
        let states: Vec<JobRunState> = progress_events(&events).iter().map(|e| e.2).collect();
        assert_eq!(
            states,
            vec![
                JobRunState::Running,
                JobRunState::Pausing,
                JobRunState::Paused,
                JobRunState::Running
            ]
        );
    }

    #[test]
    fn stopping_a_paused_job_wakes_it_and_a_second_stop_is_harmless() {
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase(JobPhase::FinalPass, 100_000);
        let (go, gate, worker) = stepper(&job);
        job.pause().unwrap();
        go.send(()).unwrap();
        wait_state(&job, JobRunState::Paused);

        job.stop().unwrap();
        assert_eq!(gate.recv_timeout(Duration::from_secs(2)), Ok(Gate::Stopped));
        assert!(job.is_stopped());
        assert!(job.stop_flag().load(Ordering::Acquire));
        assert_eq!(job.stop(), Ok(()), "doppelter Stopp");
        assert_eq!(job.pause(), Err(JobError::Stopping));
        assert_eq!(job.resume(), Err(JobError::Stopping));
        assert_eq!(job.snapshot().state, JobRunState::Stopping);

        // Auch jeder weitere Kontrollpunkt meldet den Stopp.
        go.send(()).unwrap();
        assert_eq!(gate.recv_timeout(Duration::from_secs(2)), Ok(Gate::Stopped));
        drop(go);
        worker.join().unwrap();
    }

    #[test]
    fn an_outside_cancel_frees_a_paused_job_a_new_recording_never_hangs_on_a_pause() {
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase(JobPhase::FinalPass, 100_000);
        let cancel = Arc::new(AtomicBool::new(false));
        job.pause().unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = {
            let job = Arc::clone(&job);
            let cancel = Arc::clone(&cancel);
            thread::spawn(move || {
                let gate = job.checkpoint(&|| cancel.load(Ordering::Relaxed));
                tx.send(gate).unwrap();
            })
        };
        wait_state(&job, JobRunState::Paused);
        cancel.store(true, Ordering::Relaxed); // kein notify: die Abfrage reicht
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)), Ok(Gate::Cancelled));
        worker.join().unwrap();
    }

    #[test]
    fn a_phase_that_cannot_pause_refuses_pause_but_accepts_stop() {
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase(JobPhase::Speakers, 60_000);
        assert_eq!(job.pause(), Err(JobError::NotPausable));
        assert_eq!(job.snapshot().state, JobRunState::Running);
        assert!(!job.snapshot().pausable);
        assert_eq!(job.stop(), Ok(()));
    }

    #[test]
    fn a_run_in_one_call_declares_itself_not_pausable_for_notes_and_minutes() {
        // Einzeldurchlauf: ein Modellaufruf, kein Kontrollpunkt.
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase_ex(JobPhase::Notes, 1, false);
        assert_eq!(job.pause(), Err(JobError::NotPausable));
        // In Bloecken geht es.
        job.begin_phase_ex(JobPhase::Notes, 7, true);
        assert!(job.snapshot().pausable);
        assert_eq!(job.pause(), Ok(()));
        // Protokoll: dieselbe Regel.
        let (emit, _events) = collector();
        let minutes = JobHandle::new("m2", emit);
        minutes.begin_phase_ex(JobPhase::Minutes, 1, false);
        assert_eq!(minutes.pause(), Err(JobError::NotPausable));
        assert_eq!(minutes.stop(), Ok(()), "Stopp geht immer");
    }

    #[test]
    fn a_pause_requested_in_a_block_phase_holds_at_the_next_block_phase() {
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase(JobPhase::Transcription, 100_000);
        job.pause().unwrap();
        job.begin_phase(JobPhase::Speakers, 100_000); // nicht pausierbar
        assert_eq!(job.snapshot().state, JobRunState::Pausing);
        assert_eq!(job.resume(), Ok(()), "zurueckziehen geht immer");
        assert_eq!(job.snapshot().state, JobRunState::Running);
    }

    #[test]
    fn progress_events_are_throttled_but_phase_changes_and_the_end_always_go_out() {
        let (emit, events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase(JobPhase::Transcription, 1_000_000);
        // 500 Fortschritte in wenigen ms: nur das erste geht hinaus.
        for i in 1..=500u64 {
            job.advance(i * 1_000);
        }
        let n = progress_events(&events).len();
        assert!(n <= 3, "gedrosselt statt 500 Ereignisse: {n}");
        job.advance(1_000_000); // Ende: immer
        let last = *progress_events(&events).last().unwrap();
        assert_eq!((last.0, last.1), (JobPhase::Transcription, 1_000_000));
        job.begin_phase(JobPhase::Speakers, 10);
        assert_eq!(progress_events(&events).last().unwrap().0, JobPhase::Speakers);
    }

    #[test]
    fn elapsed_time_carries_over_between_phases() {
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase(JobPhase::Transcription, 10);
        thread::sleep(Duration::from_millis(60));
        job.begin_phase(JobPhase::Speakers, 10);
        assert!(job.snapshot().elapsed_ms >= 50, "Laufzeit geht nicht auf 0 zurueck");
    }

    #[test]
    fn the_progress_event_has_a_flat_snake_case_wire_format() {
        let json = serde_json::to_value(
            JobProgress {
                meeting_id: "m1".into(),
                phase: JobPhase::FinalPass,
                done: 30_000,
                total: 60_000,
                elapsed_ms: 12_000,
                eta_ms: Some(9_000),
                state: JobRunState::Paused,
                pausable: true,
            }
            .into_event(),
        )
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "progress",
                "meeting_id": "m1",
                "phase": "final_pass",
                "done": 30000,
                "total": 60000,
                "elapsed_ms": 12000,
                "eta_ms": 9000,
                "state": "paused",
                "pausable": true
            })
        );
    }

    // ---- Verzeichnis --------------------------------------------------------------

    #[test]
    fn the_registry_allows_one_job_per_meeting_and_frees_the_slot_on_drop() {
        let jobs = MeetingJobs::new();
        let (emit, _e) = collector();
        let guard = jobs.try_start("m1", emit.clone()).unwrap();
        assert!(jobs.is_running("m1"));
        assert_eq!(jobs.try_start("m1", emit.clone()).err(), Some(JobError::Busy));
        assert!(jobs.try_start("m2", emit.clone()).is_ok(), "andere Besprechung");
        drop(guard);
        assert!(!jobs.is_running("m1"));
        assert!(jobs.try_start("m1", emit).is_ok(), "danach wieder frei");
    }

    #[test]
    fn commands_on_an_unknown_meeting_say_no_job() {
        let jobs = MeetingJobs::new();
        assert_eq!(jobs.pause("x"), Err(JobError::NoJob));
        assert_eq!(jobs.resume("x"), Err(JobError::NoJob));
        assert_eq!(jobs.stop("x"), Err(JobError::NoJob));
        assert!(jobs.snapshots().is_empty());
    }

    #[test]
    fn the_registry_steers_the_job_and_lists_it() {
        let jobs = MeetingJobs::new();
        let (emit, _e) = collector();
        let guard = jobs.try_start("m1", emit).unwrap();
        guard.begin_phase(JobPhase::Transcription, 5_000);
        jobs.pause("m1").unwrap();
        let snaps = jobs.snapshots();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].state, JobRunState::Pausing);
        assert_eq!(snaps[0].phase, JobPhase::Transcription);
        jobs.resume("m1").unwrap();
        jobs.stop("m1").unwrap();
        assert!(guard.is_stopped());
    }

    #[test]
    fn a_job_dropped_by_a_panic_still_leaves_the_registry() {
        let jobs = MeetingJobs::new();
        let (emit, _e) = collector();
        let jobs2 = jobs.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = jobs2.try_start("m1", emit).unwrap();
            panic!("Modell explodiert");
        }));
        assert!(result.is_err());
        assert!(!jobs.is_running("m1"), "kein Geisterauftrag");
    }

    #[test]
    fn pause_resume_stop_from_many_threads_never_deadlock() {
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase(JobPhase::Transcription, 1_000_000);
        let worker = {
            let job = Arc::clone(&job);
            thread::spawn(move || {
                let mut blocks = 0u32;
                loop {
                    match job.checkpoint(&|| false) {
                        Gate::Stopped | Gate::Cancelled => return blocks,
                        Gate::Go { .. } => {
                            blocks += 1;
                            job.advance(u64::from(blocks) * 100);
                            thread::sleep(Duration::from_micros(200));
                        }
                    }
                }
            })
        };
        let controllers: Vec<_> = (0..4)
            .map(|k| {
                let job = Arc::clone(&job);
                thread::spawn(move || {
                    for i in 0..150 {
                        let _ = if (i + k) % 2 == 0 { job.pause() } else { job.resume() };
                        thread::sleep(Duration::from_micros(100));
                    }
                })
            })
            .collect();
        for c in controllers {
            c.join().unwrap();
        }
        // Ein Stopp beendet den Job in jedem Zustand.
        job.stop().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || tx.send(worker.join()).unwrap());
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().is_ok(), "haengt nicht");
    }

    // ---- JobEnded, asynchrone Schnittstelle ---------------------------------------

    fn ended(events: &Events) -> Vec<(JobPhase, bool)> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                MeetingEvent::JobEnded { phase, stopped, .. } => Some((*phase, *stopped)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_finished_job_announces_its_end_so_a_closed_view_can_reload() {
        let jobs = MeetingJobs::new();
        let (emit, events) = collector();
        let guard = jobs.try_start("m1", emit.clone()).unwrap();
        guard.begin_phase(JobPhase::Minutes, 3);
        drop(guard);
        assert_eq!(ended(&events), vec![(JobPhase::Minutes, false)]);

        let guard = jobs.try_start("m1", emit).unwrap();
        guard.begin_phase(JobPhase::Notes, 3);
        guard.stop().unwrap();
        drop(guard);
        assert_eq!(
            ended(&events),
            vec![(JobPhase::Minutes, false), (JobPhase::Notes, true)],
            "gestoppt wird mitgeteilt"
        );
    }

    #[test]
    fn the_end_event_has_a_flat_wire_format() {
        let json = serde_json::to_value(MeetingEvent::JobEnded {
            meeting_id: "m1".into(),
            phase: JobPhase::Minutes,
            stopped: true,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "job_ended", "meeting_id": "m1", "phase": "minutes", "stopped": true
            })
        );
    }

    #[tokio::test]
    async fn the_async_checkpoint_pauses_without_blocking_and_resumes() {
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase(JobPhase::Minutes, 5);
        job.pause().unwrap();
        let runner = {
            let job = Arc::clone(&job);
            tokio::spawn(async move { job.checkpoint_async(|| false).await })
        };
        // Die Laufzeit bleibt bedienbar, waehrend der Lauf angehalten ist.
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(!runner.is_finished());
        assert!(job.is_paused());
        job.resume().unwrap();
        assert_eq!(runner.await.unwrap(), Gate::Go { resumed: true });
    }

    #[tokio::test]
    async fn stopped_resolves_for_a_stop_before_and_after_the_wait_began() {
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        let waiter = {
            let job = Arc::clone(&job);
            tokio::spawn(async move { job.stopped().await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiter.is_finished());
        job.stop().unwrap();
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("wacht auf")
            .unwrap();
        // Schon gestoppt: sofort fertig.
        tokio::time::timeout(Duration::from_millis(200), job.stopped())
            .await
            .expect("sofort");
    }

    #[tokio::test]
    async fn a_run_stopped_through_select_is_dropped_and_frees_what_it_holds() {
        struct Held(Arc<AtomicBool>);
        impl Drop for Held {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        let released = Arc::new(AtomicBool::new(false));
        let run = {
            let released = Arc::clone(&released);
            async move {
                let _held = Held(released);
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        };
        let stopper = {
            let job = Arc::clone(&job);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                job.stop().unwrap();
            })
        };
        let outcome = tokio::select! {
            _ = run => "done",
            _ = job.stopped() => "stopped",
        };
        stopper.await.unwrap();
        assert_eq!(outcome, "stopped");
        assert!(released.load(Ordering::SeqCst), "Wache und Verbindung gehen frei (Drop)");
    }

    #[tokio::test]
    async fn the_scope_gives_nested_async_code_its_job_and_without_one_everything_is_a_noop() {
        // Ohne Auftrag: kein Fehler, der Kontrollpunkt laesst durch.
        report_total(9);
        report_done(1);
        assert!(!stop_requested());
        assert_eq!(checkpoint().await, Gate::Go { resumed: false });

        let (emit, events) = collector();
        let job = JobHandle::new("m1", emit);
        scope(Arc::clone(&job), async {
            report_phase(JobPhase::Minutes, 4, true);
            report_done(2);
            assert!(!stop_requested());
            job.stop().unwrap();
            assert!(stop_requested());
            assert_eq!(checkpoint().await, Gate::Stopped);
        })
        .await;
        let snap = job.snapshot();
        assert_eq!((snap.phase, snap.total, snap.done), (JobPhase::Minutes, 4, 2));
        assert!(!progress_events(&events).is_empty());
    }

    #[tokio::test]
    async fn the_time_limit_ignores_time_spent_in_a_pause() {
        // Ohne Auftrag: ein gewoehnliches Zeitlimit.
        let slow = tokio::time::sleep(Duration::from_millis(600));
        assert_eq!(
            timeout_excluding_pauses(Duration::from_millis(200), slow).await,
            Err(TimedOut)
        );
        // Mit Auftrag, dauerhaft angehalten: das Limit laeuft nicht ab.
        let (emit, _events) = collector();
        let job = JobHandle::new("m1", emit);
        job.begin_phase(JobPhase::Notes, 5);
        job.pause().unwrap();
        let holder = {
            let job = Arc::clone(&job);
            tokio::spawn(async move { job.checkpoint_async(|| false).await })
        };
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(job.is_paused());
        let result = scope(Arc::clone(&job), async {
            timeout_excluding_pauses(
                Duration::from_millis(200),
                tokio::time::sleep(Duration::from_millis(700)),
            )
            .await
        })
        .await;
        assert!(result.is_ok(), "700 ms in der Pause zaehlen nicht gegen 200 ms");
        job.stop().unwrap();
        holder.await.unwrap();
        // Laeuft der Auftrag, gilt das Limit.
        let (emit, _events) = collector();
        let running = JobHandle::new("m2", emit);
        let result = scope(running, async {
            timeout_excluding_pauses(
                Duration::from_millis(300),
                tokio::time::sleep(Duration::from_millis(900)),
            )
            .await
        })
        .await;
        assert_eq!(result, Err(TimedOut));
    }
}
