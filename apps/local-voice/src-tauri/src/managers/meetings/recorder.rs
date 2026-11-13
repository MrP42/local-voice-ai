//! M8 meetings: the recorder manager that orchestrates a live meeting —
//! dual capture (mic + system loopback), streaming WAV writing, the live
//! chunk -> transcription -> delta pipeline, the consent gate, crash
//! recovery and the recording indicator (tray + overlay).
//!
//! Everything that can be decided without I/O lives in the free functions at
//! the top (`consent_gate`, `may_start`, `apply_pause`) so the state rules are
//! testable without Tauri, audio hardware or a model.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use super::chunker::Chunk;
use super::dsp::{
    ChannelFeed, DspConfig, DspControl, DspNotice, EchoSetup, LivePipeline, MeetingTimeline,
    PcmSink, VadFactory, WavFileSink,
};
use super::empty::{coded_error, EmptyFill};
use super::final_pass::{self, FinalChoice, FinalPlan, JobSpec, KeepReason};
use super::mic_capture::MeetingMicCapture;
use super::signal_watch::HealthState;
use super::store::{Meeting, MeetingSource, MeetingStatus, MeetingStore, StoredSegment};
use crate::audio_toolkit::audio::{LoopbackCapture, StreamingWavWriter};
use crate::managers::transcription::TranscriptionManager;

/// Sample rate of the whole meetings pipeline (mic capture and loopback both
/// deliver 16 kHz mono i16).
const SAMPLE_RATE: u32 = 16_000;
/// Silero VAD model (bundled resource) that segments the live channels.
const VAD_MODEL_RESOURCE: &str = "resources/models/silero_vad_v4.onnx";
/// VAD decision threshold for meetings (neutral, independent of the dictation
/// microphone sensitivity).
const MEETING_VAD_THRESHOLD: f32 = 0.5;
/// WAV header rewrite cadence — one second of audio, so a crash costs at most
/// that much of the recoverable header state.
const FLUSH_EVERY_SAMPLES: usize = SAMPLE_RATE as usize;
/// Level events per channel: ~5/s.
const LEVEL_INTERVAL: Duration = Duration::from_millis(200);
/// `LoopbackCapture::start` can block indefinitely on a wedged audio driver
/// (known finding from Task 4). We give it this long, then continue mic-only.
const LOOPBACK_START_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the watchdog checks whether the loopback thread has died.
const LOOPBACK_WATCH_INTERVAL: Duration = Duration::from_millis(500);
/// How many meetings `recover_orphans` scans per page.
const RECOVERY_PAGE: u32 = 100;

/// Transcript channel ids (mirror `StoredSegment::channel`).
pub const CHANNEL_MIC: u8 = 0;
pub const CHANNEL_SYSTEM: u8 = 1;

// ---------------------------------------------------------------------------
// Pure state logic
// ---------------------------------------------------------------------------

/// What the recorder is doing right now. Deliberately tiny: everything that
/// needs cleanup lives in `RecordingSession`, this is only the rule surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeetingRunState {
    Idle,
    Recording { meeting_id: String, paused: bool },
}

/// Recording a meeting without a confirmed consent hint is refused outright —
/// the consent confirmation is a product requirement (Spec A1), not a UI nicety.
pub fn consent_gate(consent_confirmed: bool) -> Result<(), String> {
    if consent_confirmed {
        Ok(())
    } else {
        Err("consent_required".to_string())
    }
}

/// Only one meeting records at a time (one WAV pair, one worker, one indicator).
pub fn may_start(state: &MeetingRunState) -> bool {
    matches!(state, MeetingRunState::Idle)
}

/// Recovery beim Start: nur `recording` und `processing` sind Reste eines
/// Absturzes. `cancelled` (P8a, Stopp durch den Nutzer) ist ein Endzustand:
/// wer gestoppt hat, will nicht, dass die App beim naechsten Start von selbst
/// weiterrechnet.
pub fn is_orphan_status(status: &str) -> bool {
    matches!(status, "recording" | "processing")
}

/// P8a: darf eine Besprechung fortgesetzt werden? Nur nach einem Stopp
/// (`cancelled`), nur ohne laufenden Auftrag und nur, solange eine Aufnahme auf
/// der Platte liegt (`exists` prueft einen Pfad). Die Fehlercodes uebersetzt
/// die Oberflaeche.
pub fn check_can_continue(
    meeting: &Meeting,
    job_running: bool,
    exists: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    if meeting.status != "cancelled" {
        return Err("not_cancelled".to_string());
    }
    if job_running {
        return Err(super::job::JobError::Busy.to_string());
    }
    let has_audio = [&meeting.mic_audio_path, &meeting.system_audio_path]
        .into_iter()
        .flatten()
        .any(|p| exists(p));
    if !has_audio {
        return Err("audio_missing".to_string());
    }
    Ok(())
}

/// Die Zeile einer neuen Live-Aufnahme: eine neue Besprechung oder, mit `target`,
/// ein vorhandener leerer Eintrag (G1, #70), der zur Aufnahme umgewandelt wird.
/// Ohne Datenbank-Zugriff ausser dem Store: laesst sich ohne App pruefen.
pub fn begin_live_row(
    store: &MeetingStore,
    title: &str,
    consent_at: i64,
    target: Option<&str>,
) -> Result<Meeting, String> {
    match target {
        None => store
            .create_meeting(title, MeetingSource::Live, Some(consent_at))
            .map_err(|e| format!("meeting_create_failed: {e}")),
        Some(id) => {
            let mut fill = EmptyFill::new(MeetingSource::Live, MeetingStatus::Recording);
            fill.consent_confirmed_at = Some(consent_at);
            fill.title = Some(title);
            store
                .fill_empty_meeting(id, &fill)
                .map_err(|e| coded_error("meeting_create_failed", &e))
        }
    }
}

/// Aufraeumen nach einem Start, der nicht zustande kam (#15): die Aufnahmedateien
/// (`mic.wav`, `system.wav`, `mic_aec.wav` mit Null-Laenge im Kopf) bleiben sonst
/// als Debris liegen, die DB zeigt auf sie, und die Zeile bliebe `recording` und
/// wuerde beim naechsten App-Start als "Absturzrest" wiederbelebt. Hier: Zeile
/// `failed`, Dateien weg, Pfade geleert, Ordner weg, wenn danach leer. Fremde
/// Dateien im Ordner bleiben. Best effort: was sich nicht loeschen laesst (noch
/// offen), steht im Log.
pub(crate) fn discard_failed_start(store: &MeetingStore, meeting_id: &str, dir: &std::path::Path) {
    if let Err(e) = store.set_status(meeting_id, MeetingStatus::Failed) {
        warn!("meetings: failed start of {meeting_id}: status not stored: {e}");
    }
    for name in ["mic.wav", "system.wav", super::MIC_AEC_FILE] {
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // Pfad nur, nie Inhalt (Log-Datenschutz).
            Err(e) => warn!("meetings: could not remove {path:?} after a failed start: {e}"),
        }
    }
    // `remove_dir` entfernt nur einen leeren Ordner: Fremdes bleibt.
    let _ = std::fs::remove_dir(dir);
    if let Err(e) = store.set_audio_paths(meeting_id, None, None, None) {
        warn!("meetings: failed start of {meeting_id}: audio paths not cleared: {e}");
    }
}

/// Pause/resume only mean something while recording.
pub fn apply_pause(state: &mut MeetingRunState, paused: bool) -> Result<(), String> {
    match state {
        MeetingRunState::Recording { paused: p, .. } => {
            *p = paused;
            Ok(())
        }
        MeetingRunState::Idle => Err("not_recording".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Lauf-Kern: Zustand + Sitzung ohne Tauri
// ---------------------------------------------------------------------------

/// Sperre nehmen, auch wenn ein anderer Thread mit ihr abgestuerzt ist. Der
/// Zustand hier ist ein kleines Enum bzw. ein Option: ein "vergifteter" Wert ist
/// nie halb geschrieben. Wer auf dem Hotkey-Pfad `lock().unwrap()` ruft, nimmt
/// sonst das Diktat mit in die Panik (#15).
fn lock_recovering<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Die Zustandsregeln des Recorders mit der Sitzung als Typparameter, damit sie
/// ohne Mikrofon, Modell und `AppHandle` pruefbar sind. Reihenfolge der Sperren
/// ueberall: erst `state`, dann `session`.
///
/// Der Zustand wechselt NACH dem Capture-Start auf `Recording` und erst NACH dem
/// Capture-Stopp zurueck auf `Idle` (#15): solange ein Mikrofon offen ist, gilt
/// die Aufnahme als laufend, fuer Diktat-Hotkey, neuen Start und Pause.
pub(crate) struct RunCore<S> {
    state: Mutex<MeetingRunState>,
    session: Mutex<Option<S>>,
    starting: AtomicBool,
}

impl<S> RunCore<S> {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(MeetingRunState::Idle),
            session: Mutex::new(None),
            starting: AtomicBool::new(false),
        }
    }

    /// Laeuft eine Aufnahme oder gerade ein Start/Stopp? Hotkey-Pfad: darf nie
    /// panikieren.
    pub(crate) fn is_recording(&self) -> bool {
        self.starting.load(Ordering::Acquire) || !self.is_idle()
    }

    /// Nur `Idle` darf starten.
    pub(crate) fn is_idle(&self) -> bool {
        may_start(&lock_recovering(&self.state))
    }

    /// Beginnt einen Start: `already_recording`, wenn schon einer laeuft oder
    /// aufgenommen wird. Das Ticket meldet "starting" bis zum Ende (auch bei jedem
    /// Fehlerweg).
    pub(crate) fn begin_start(&self) -> Result<StartingFlag<'_>, String> {
        if !self.is_idle() {
            return Err("already_recording".to_string());
        }
        // Ein zweiter Start waehrend des ersten (der Start haelt die Sperre lange:
        // Mikrofon, Loopback-Timeout) wird abgewiesen, nicht ueberholt.
        if self
            .starting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("already_recording".to_string());
        }
        Ok(StartingFlag::set(&self.starting))
    }

    /// Die Capture laeuft: Zustand UND Sitzung unter beiden Sperren setzen.
    pub(crate) fn publish_started(&self, meeting_id: &str, session: S) {
        crate::managers::planning::ohne_tor_sperre("Zustand der Aufnahme (publish_started)");
        // Beide Sperren zusammen: ein Stopp dazwischen sah vorher die Sitzung, aber
        // noch `Idle`, stellte `Idle` ein, und der Start setzte danach `Recording`
        // ohne Sitzung (Geisteraufnahme).
        let mut state = lock_recovering(&self.state);
        let mut slot = lock_recovering(&self.session);
        *state = MeetingRunState::Recording {
            meeting_id: meeting_id.to_string(),
            paused: false,
        };
        *slot = Some(session);
    }

    /// Stopp beginnen: die Sitzung herausnehmen. Der Zustand bleibt `Recording`,
    /// bis das Ticket beendet wird (`finish`) oder faellt (auch bei Panik).
    pub(crate) fn begin_stop(&self) -> Result<(S, StopTicket<'_, S>), String> {
        crate::managers::planning::ohne_tor_sperre("Zustand der Aufnahme (begin_stop)");
        let _state = lock_recovering(&self.state);
        let mut slot = lock_recovering(&self.session);
        let session = slot.take().ok_or_else(|| "not_recording".to_string())?;
        Ok((session, StopTicket { core: self }))
    }

    /// Pause/Weiter: nur mit laufender Sitzung. `apply` setzt das Merkzeichen der
    /// Capture-Callbacks unter denselben Sperren.
    pub(crate) fn set_paused(
        &self,
        paused: bool,
        apply: impl FnOnce(&S),
    ) -> Result<String, String> {
        crate::managers::planning::ohne_tor_sperre("Zustand der Aufnahme (set_paused)");
        let mut state = lock_recovering(&self.state);
        let slot = lock_recovering(&self.session);
        // Ohne Sitzung (Startfenster, Stopp laeuft) gibt es nichts zu pausieren; ein
        // Zustandsereignis "pausiert" ohne wirkende Pause waere falsch.
        let session = slot.as_ref().ok_or_else(|| "not_recording".to_string())?;
        apply_pause(&mut state, paused)?;
        apply(session);
        match &*state {
            MeetingRunState::Recording { meeting_id, .. } => Ok(meeting_id.clone()),
            MeetingRunState::Idle => Err("not_recording".to_string()),
        }
    }

    /// Lesezugriff auf die Sitzung (Audioposition fuer den Notizblock).
    pub(crate) fn with_session<R>(&self, f: impl FnOnce(Option<&S>) -> R) -> R {
        let slot = lock_recovering(&self.session);
        f(slot.as_ref())
    }
}

/// Haelt den Zustand `Recording`, bis der Stopp die Capture wirklich beendet hat.
/// Faellt es (Panik im Stopp), wird der Zustand trotzdem frei: sonst bliebe eine
/// "Geisteraufnahme" zurueck, die Diktat und Neustart bis zum App-Neustart sperrt.
pub(crate) struct StopTicket<'a, S> {
    core: &'a RunCore<S>,
}

impl<S> StopTicket<'_, S> {
    /// Die Capture ist zu: der Zustand wird `Idle`.
    pub(crate) fn finish(self) {
        drop(self);
    }
}

impl<S> Drop for StopTicket<'_, S> {
    fn drop(&mut self) {
        *lock_recovering(&self.core.state) = MeetingRunState::Idle;
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Typed frontend event (pattern: `HistoryUpdatePayload`). `message` on the
/// error variant carries an i18n-able code string, never a prose sentence.
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind")]
pub enum MeetingEvent {
    #[serde(rename = "state")]
    State {
        meeting_id: String,
        status: String,
        paused: bool,
    },
    #[serde(rename = "segments")]
    Segments {
        meeting_id: String,
        appended: Vec<StoredSegment>,
    },
    #[serde(rename = "levels")]
    Levels { mic: f32, system: f32 },
    #[serde(rename = "error")]
    Error { meeting_id: String, message: String },
    /// Every segment of this meeting was discarded (re-transcription started).
    /// Consumers that keep a local segment list must clear it — otherwise the
    /// new run's segments, which restart at index 0, would append to the old.
    #[serde(rename = "reset")]
    Reset { meeting_id: String },
    /// The transcript is final and stored: after the final pass (P2d), or at
    /// once when the live transcript stays (setting `off`, CPU only, skipped).
    /// `epoch` is the generation of the segments (`segment_epoch`); `model` the
    /// engine that produced them, `None` when the final pass was skipped by an
    /// error. Consumers that build on the transcript (AI notes, index) start
    /// here, not at `stop()`.
    #[serde(rename = "transcript_final")]
    TranscriptFinal {
        meeting_id: String,
        epoch: u32,
        model: Option<String>,
    },
    /// Zustandswechsel des Ausfallwaechters (M2-P2e, `signal_watch.rs`).
    /// `channel`: 0 = Mikrofon, 1 = Systemton. Nur Wechsel, nie Dauerfeuer;
    /// `recovered` nimmt die Kanalwarnung zurueck, `vad_unavailable` und
    /// `loopback_died` bleiben bis zum Ende der Besprechung stehen.
    #[serde(rename = "health")]
    Health {
        meeting_id: String,
        channel: u8,
        state: HealthState,
    },
    /// P8a: Fortschritt einer Verarbeitung (Import, Enddurchlauf, Neu-
    /// Transkription, Sprecher, Notizen, Protokoll); hoechstens 2 / s je
    /// Besprechung, Zustandswechsel (Pause, Stopp, Phase) sofort. `done` /
    /// `total` zaehlen ms Audio (Notizen und Protokoll: Bloecke), `total` 0 =
    /// Groesse unbekannt, `eta_ms` `None` = noch in der Anlaufzeit.
    #[serde(rename = "progress")]
    Progress {
        meeting_id: String,
        phase: super::job::JobPhase,
        done: u64,
        total: u64,
        elapsed_ms: u64,
        eta_ms: Option<u64>,
        state: super::job::JobRunState,
        pausable: bool,
    },
    /// P8a: der Auftrag zu `meeting_id` ist zu Ende (fertig, gestoppt oder
    /// gescheitert): eine Ansicht, die beim Ende nicht offen war, laedt ihr
    /// Ergebnis daraufhin neu und nimmt den Laufzustand zurueck.
    #[serde(rename = "job_ended")]
    JobEnded {
        meeting_id: String,
        phase: super::job::JobPhase,
        stopped: bool,
    },
}

/// Macht eine Meldung des DSP-Threads zum Oberflaechen-Ereignis. Was kein
/// Zustand des Kanals ist (Panik-Rueckfaelle, roher Ueberlauf-Zaehler), bleibt
/// im Log: der Ueberlauf kommt entprellt als `Health { QueueOverflow }`.
pub fn health_event(meeting_id: &str, notice: &DspNotice) -> Option<MeetingEvent> {
    let (channel, state) = match *notice {
        DspNotice::Health { channel, state } => (channel, state),
        DspNotice::VadUnavailable { channel } => (channel, HealthState::VadUnavailable),
        DspNotice::Overflow { .. } | DspNotice::DspPanic { .. } | DspNotice::AecPanic => {
            return None
        }
    };
    Some(MeetingEvent::Health {
        meeting_id: meeting_id.to_string(),
        channel,
        state,
    })
}

// ---------------------------------------------------------------------------
// Session internals
// ---------------------------------------------------------------------------

/// One channel's write path: the raw WAV file. Segmentation happens on the DSP
/// thread (`dsp.rs`), never here.
struct ChannelSink {
    writer: Option<StreamingWavWriter>,
    samples_since_flush: usize,
}

impl ChannelSink {
    fn new(writer: StreamingWavWriter) -> Self {
        Self {
            writer: Some(writer),
            samples_since_flush: 0,
        }
    }
}

/// Throttles level events to ~5/s per channel while still reporting both
/// channels in every event (the payload carries mic and system together).
struct LevelEmitter {
    app: AppHandle,
    inner: Mutex<LevelState>,
}

struct LevelState {
    mic: f32,
    system: f32,
    last_mic: Instant,
    last_system: Instant,
}

impl LevelEmitter {
    fn new(app: AppHandle) -> Self {
        let past = Instant::now() - LEVEL_INTERVAL;
        Self {
            app,
            inner: Mutex::new(LevelState {
                mic: 0.0,
                system: 0.0,
                last_mic: past,
                last_system: past,
            }),
        }
    }

    fn record(&self, channel: u8, rms: f32) {
        let payload = {
            let Ok(mut state) = self.inner.lock() else {
                return;
            };
            let now = Instant::now();
            let due = if channel == CHANNEL_MIC {
                state.mic = rms;
                let due = now.duration_since(state.last_mic) >= LEVEL_INTERVAL;
                if due {
                    state.last_mic = now;
                }
                due
            } else {
                state.system = rms;
                let due = now.duration_since(state.last_system) >= LEVEL_INTERVAL;
                if due {
                    state.last_system = now;
                }
                due
            };
            if !due {
                return;
            }
            MeetingEvent::Levels {
                mic: state.mic,
                system: state.system,
            }
        };
        let _ = payload.emit(&self.app);
    }
}

fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples
        .iter()
        .map(|s| {
            let v = *s as f64 / i16::MAX as f64;
            v * v
        })
        .sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// Everything a running meeting owns. Dropped as a unit by `stop()`.
struct RecordingSession {
    meeting_id: String,
    paused: Arc<AtomicBool>,
    mic_capture: Option<MeetingMicCapture>,
    loopback: Option<LoopbackCapture>,
    mic_sink: Arc<Mutex<ChannelSink>>,
    system_sink: Option<Arc<Mutex<ChannelSink>>>,
    mic_path: PathBuf,
    system_path: Option<PathBuf>,
    /// DSP thread + transcription worker of this meeting.
    pipeline: Option<LivePipeline>,
    /// lva#544: die Anmeldung dieser Aufnahme im [`FinalSlot`]. Solange sie lebt, beginnt kein
    /// Enddurchlauf-/Recovery-Auftrag mit einem Modell. `stop()` gibt sie frei, sobald der
    /// Live-Worker leer ist, und erst danach startet der Enddurchlauf dieser Aufnahme.
    live: LiveHalter,
}

/// M2-P2d: der laufende Enddurchlauf-/Recovery-Thread.
struct FinalJobHandle {
    cancel: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

/// Was der [`FinalSlot`] unter EINER Sperre haelt: den Thread und die Zahl der Aufnahmen, die
/// gerade beginnen oder laufen.
#[derive(Default)]
struct SlotState {
    job: Option<FinalJobHandle>,
    /// Angemeldete Aufnahmen ([`LiveHalter`]). Solange > 0, wird jeder neue Auftrag mit gesetztem
    /// Abbruch-Merker geboren.
    live: u32,
}

/// So lange wartet der Start einer Aufnahme darauf, dass ein laufender Enddurchlauf nach dem
/// Abbruch endet. Der Abbruch wirkt zwischen zwei Modellaufrufen (ein laufender Aufruf der
/// Spracherkennung und ein laufendes Laden lassen sich nicht unterbrechen); endet der Auftrag in
/// dieser Zeit nicht, scheitert der START (`job_busy`), statt unbegrenzt zu haengen.
pub(crate) const CANCEL_WAIT: Duration = Duration::from_secs(90);

/// Abstand, in dem der Start nach dem Ende des abgebrochenen Auftrags sieht.
const CANCEL_POLL_SLOT: Duration = Duration::from_millis(10);

/// Der Platz fuer den Enddurchlauf-/Recovery-Thread (hoechstens einer) UND die Anmeldung laufender
/// Aufnahmen: beides unter derselben Sperre, damit zwischen "Aufnahme beginnt" und "Auftrag
/// beginnt" keine Luecke bleibt (lva#544, Fixrunde 5). Ohne Tauri pruefbar.
///
/// Regel: ein Enddurchlauf-/Recovery-Auftrag laesst nie ein Modell neben einer Aufnahme laufen.
/// - Eine Aufnahme meldet sich an ([`FinalSlot::live_beginnen`]), BEVOR sie das Tor betritt: der
///   laufende Auftrag bekommt den Abbruch-Merker und wird (mit Frist) abgewartet.
/// - Ein Auftrag, der waehrend einer angemeldeten Aufnahme entsteht ([`FinalSlot::spawn`]), traegt
///   den Abbruch-Merker von Geburt an: `run_job` laedt und rechnet dann nichts
///   (`FinalEnv::cancelled` vor jedem Laden und vor jedem Modellaufruf).
pub(crate) struct FinalSlot {
    inner: Arc<Mutex<SlotState>>,
    cancel_wait: Duration,
}

impl Default for FinalSlot {
    fn default() -> Self {
        Self::mit_frist(CANCEL_WAIT)
    }
}

/// Die Anmeldung EINER Aufnahme im [`FinalSlot`] (vom Beginn des Starts bis der Live-Worker nach
/// dem Stopp leer ist). Faellt sie, zaehlt die Aufnahme nicht mehr (auch bei jedem Fehlerweg).
pub(crate) struct LiveHalter {
    inner: Arc<Mutex<SlotState>>,
}

impl Drop for LiveHalter {
    fn drop(&mut self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.live = state.live.saturating_sub(1);
    }
}

impl FinalSlot {
    pub(crate) fn mit_frist(cancel_wait: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SlotState::default())),
            cancel_wait,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, SlotState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Startet `body` in einem neuen Thread. Laeuft noch ein frueherer, bekommt `body` dessen
    /// Handle (zum Abwarten) und teilt seinen Abbruch-Merker (ein Abbruch trifft dann beide),
    /// ausser der ist schon gesetzt: der Vorgaenger endet dann von selbst, und der neue Auftrag
    /// soll nicht mit ihm sterben. Ist eine Aufnahme angemeldet, ist der Merker des neuen Auftrags
    /// von Geburt an gesetzt.
    pub(crate) fn spawn(
        &self,
        body: impl FnOnce(Option<std::thread::JoinHandle<()>>, Arc<AtomicBool>) + Send + 'static,
    ) -> std::io::Result<()> {
        let mut slot = self.state();
        let previous = slot.job.take().and_then(|p| {
            if p.handle.is_finished() {
                let _ = p.handle.join();
                None
            } else {
                Some(p)
            }
        });
        let live = slot.live > 0;
        let (cancel, previous_handle) = match previous {
            Some(p) if !live && !p.cancel.load(Ordering::Acquire) => (p.cancel, Some(p.handle)),
            Some(p) => (Arc::new(AtomicBool::new(live)), Some(p.handle)),
            None => (Arc::new(AtomicBool::new(live)), None),
        };
        if live {
            info!("meetings: a recording is live - the new final pass job starts cancelled");
        }
        let job_cancel = Arc::clone(&cancel);
        let handle = std::thread::Builder::new()
            .name("meeting-final-pass".to_string())
            .spawn(move || body(previous_handle, job_cancel))?;
        slot.job = Some(FinalJobHandle { cancel, handle });
        Ok(())
    }

    /// Ein Thread laeuft (nicht nur: ein Handle liegt da).
    pub(crate) fn active(&self) -> bool {
        self.state()
            .job
            .as_ref()
            .is_some_and(|job| !job.handle.is_finished())
    }

    /// Ist eine Aufnahme angemeldet (beginnt oder laeuft)?
    pub(crate) fn live_aktiv(&self) -> bool {
        self.state().live > 0
    }

    /// Meldet eine Aufnahme an: ab jetzt (unter der Sperre des Platzes) wird jeder neue Auftrag
    /// abgebrochen geboren, und der laufende bekommt den Abbruch-Merker. Danach wird sein Ende
    /// abgewartet, hoechstens die Frist des Platzes ([`CANCEL_WAIT`]): endet er nicht, gibt es
    /// keinen Start (`job_busy`), die Anmeldung faellt, der Auftrag bleibt abgebrochen im Platz.
    /// NIE unter der Tor-Sperre (Sperrordnung): der Auftrag nimmt sie selbst.
    pub(crate) fn live_beginnen(&self) -> Result<LiveHalter, String> {
        crate::managers::planning::ohne_tor_sperre("Abwarten des Enddurchlaufs");
        {
            let mut slot = self.state();
            slot.live += 1;
            if let Some(job) = &slot.job {
                if !job.handle.is_finished() {
                    info!("meetings: a new recording cancels the running final pass");
                }
                job.cancel.store(true, Ordering::Release);
            }
        }
        let halter = LiveHalter {
            inner: Arc::clone(&self.inner),
        };
        let frist = Instant::now() + self.cancel_wait;
        loop {
            let fertig = {
                let mut slot = self.state();
                match &slot.job {
                    None => return Ok(halter),
                    Some(job) if job.handle.is_finished() => slot.job.take(),
                    Some(job) => {
                        // Auch ein inzwischen neu entstandener Auftrag traegt den Merker schon.
                        job.cancel.store(true, Ordering::Release);
                        None
                    }
                }
            };
            if let Some(job) = fertig {
                if job.handle.join().is_err() {
                    error!("meetings: final pass thread panicked");
                }
                continue;
            }
            if Instant::now() >= frist {
                warn!("meetings: the cancelled final pass did not end in time - no start");
                return Err("job_busy: final_pass_not_cancelled".to_string());
            }
            std::thread::sleep(CANCEL_POLL_SLOT);
        }
    }

    /// Wartet auf den Thread (Headless-Laeufe), ohne ihn abzubrechen.
    pub(crate) fn wait(&self) {
        crate::managers::planning::ohne_tor_sperre("join des Enddurchlaufs");
        let job = self.state().job.take();
        if let Some(job) = job {
            if job.handle.join().is_err() {
                error!("meetings: final pass thread panicked");
            }
        }
    }

    /// Beginnt eine Aufnahme unter dem Einwilligungs-Tor, in der Reihenfolge der Sperrordnung
    /// (`managers::planning`): ERST die Aufnahme anmelden und den laufenden Auftrag abwarten
    /// (ohne Tor-Sperre, mit Frist), DANN unter der Sperre die frische Pruefung und `start`.
    /// Eine Wiederholungsschleife gibt es nicht mehr: seit der Anmeldung kann kein Auftrag mehr
    /// mit einem Modell beginnen. `nach_abbruch` laeuft zwischen Abwarten und Tor (Testhaken).
    /// Der [`LiveHalter`] gehoert der Aufnahme; faellt er, endet die Anmeldung.
    pub(crate) fn start_unter_tor<T>(
        &self,
        store: &MeetingStore,
        event: Option<&crate::managers::calendar::model::CalEvent>,
        target_meeting_id: Option<&str>,
        nach_abbruch: impl FnOnce(),
        start: impl FnOnce(&crate::managers::planning::TorNachweis) -> Result<T, String>,
    ) -> Result<(T, LiveHalter), String> {
        let live = self.live_beginnen()?;
        nach_abbruch();
        let wert = crate::managers::planning::mit_tor(store, event, target_meeting_id, start)?;
        Ok((wert, live))
    }
}

/// Setzt `starting` fuer die Dauer von `start()` (auch bei jedem Fehlerweg).
pub(crate) struct StartingFlag<'a>(&'a AtomicBool);

impl<'a> StartingFlag<'a> {
    fn set(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::Release);
        Self(flag)
    }
}

impl Drop for StartingFlag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Laesst `inner` erst arbeiten, wenn die Aufnahme WIRKSAM begonnen hat
/// (`planning::Gebunden::wirksam`, unter der Tor-Sperre). Vorher fallen die Samples weg: bis zur
/// letzten Pruefung des Tors wird nichts aufgenommen und nichts an die Transkription gegeben.
fn erst_wenn_wirksam(
    wirksam: Arc<AtomicBool>,
    mut inner: impl FnMut(&[i16], Option<u64>) + Send + 'static,
) -> impl FnMut(&[i16], Option<u64>) + Send + 'static {
    move |samples: &[i16], qpc: Option<u64>| {
        if wirksam.load(Ordering::Acquire) {
            inner(samples, qpc);
        }
    }
}

/// EIN Block der Live-Transkription hinter dem Einwilligungs-Tor (lva#534, lva#544). `call`
/// rechnet den Block; JEDER Modellversuch darin steht fuer sich hinter dem Tor
/// (`import::transcribe_chunk_resilient` -> `planning::modell_lauf`) und meldet eine Ablehnung als
/// `Err`. Dann liefert der Block KEINE Segmente (auch keine Luecken-Platzhalter); die Aufnahme
/// selbst laeuft weiter. `abgelehnt` meldet das einmal je Aufnahme an die Oberflaeche.
pub(crate) fn live_block_gated(
    meeting_id: &str,
    told: &AtomicBool,
    call: impl FnOnce() -> Result<Vec<crate::managers::transcription::TimedSegment>, String>,
    abgelehnt: impl FnOnce(),
) -> Vec<crate::managers::transcription::TimedSegment> {
    match call() {
        Ok(segments) => segments,
        Err(code) => {
            warn!("meetings: Block nicht transkribiert ({meeting_id}, {code})");
            if !told.swap(true, Ordering::Relaxed) {
                abgelehnt();
            }
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

pub struct MeetingRecorderManager {
    app: AppHandle,
    store: Arc<MeetingStore>,
    transcription: Arc<TranscriptionManager>,
    /// Zustand, Sitzung und "Start laeuft" (M2-P2d: zaehlt als "Aufnahme aktiv",
    /// damit ein `TranscriptFinal` des eben abgebrochenen Enddurchlaufs keine
    /// lokalen KI-Notizen neben der neuen Aufnahme startet) samt der Regeln, wie
    /// sie wechseln (#15, `RunCore`).
    core: RunCore<RecordingSession>,
    /// Held for the whole of `start()`. The run state only flips to
    /// `Recording` once the captures are up, so without this a double-click
    /// could get two starts past `may_start` and leave one orphaned.
    start_guard: Mutex<()>,
    /// M2-P2d: Enddurchlauf bzw. Recovery im Hintergrund (hoechstens einer).
    finals: FinalSlot,
}

impl MeetingRecorderManager {
    pub fn new(
        app: &AppHandle,
        store: Arc<MeetingStore>,
        transcription: Arc<TranscriptionManager>,
    ) -> Self {
        Self {
            app: app.clone(),
            store,
            transcription,
            core: RunCore::new(),
            start_guard: Mutex::new(()),
            finals: FinalSlot::default(),
        }
    }

    pub fn is_recording(&self) -> bool {
        // Hotkey-Pfad des Diktats: vergiftete Sperren werden uebergangen, nie
        // zur Panik (#15).
        self.core.is_recording()
    }

    /// M2-P2d: startet Enddurchlauf-/Recovery-Auftraege in EINEM Thread, der
    /// sie nacheinander abarbeitet (eine Engine, nie zwei grosse Modelle).
    /// Laeuft noch ein frueherer Thread, wartet der neue auf ihn und teilt
    /// seinen Abbruch-Merker. Danach kommt das Diktatmodell zurueck
    /// (Muster `stop()`), ausser eine neue Aufnahme hat abgebrochen.
    fn spawn_final_jobs(&self, jobs: Vec<JobSpec>) {
        if jobs.is_empty() {
            return;
        }
        let app = self.app.clone();
        let store = Arc::clone(&self.store);
        let tm = Arc::clone(&self.transcription);
        let thread_jobs = jobs.clone();
        let spawned = self.finals.spawn(move |previous, job_cancel| {
            if let Some(previous) = previous {
                let _ = previous.join();
            }
            let mut env = final_pass::AppEnv::new(&app, Arc::clone(&tm), Arc::clone(&job_cancel));
            for job in &thread_jobs {
                // Das Tor entscheidet `run_job` selbst (frisch, unter der Sperre).
                final_pass::run_job(&store, job, &mut env);
            }
            if !job_cancel.load(Ordering::Relaxed) {
                let dictation_model = crate::settings::get_settings(&app).selected_model;
                tm.initiate_model_load_target(&dictation_model);
            }
        });
        if let Err(e) = spawned {
            // Ohne Thread kein Enddurchlauf, aber auch kein haengendes
            // `processing`: jede Besprechung wird hier abgeschlossen.
            warn!("meetings: final pass thread not started ({e}) - live transcripts stay");
            // lva#544: dieser Notweg laeuft im Aufrufer und steht NICHT im `FinalSlot`: eine
            // Aufnahme, die jetzt beginnt, koennte ihn weder abbrechen noch abwarten. Er rechnet
            // deshalb nie ein Modell (Abbruch-Merker von Anfang an: kein Laden, keine
            // Spracherkennung, kein Sprecher-Modell) und schliesst die Besprechungen nur ab.
            let mut env = final_pass::AppEnv::new(
                &self.app,
                Arc::clone(&self.transcription),
                Arc::new(AtomicBool::new(true)),
            );
            for mut job in jobs {
                job.plan = FinalPlan::Keep(KeepReason::LoadFailed);
                job.catch_up_model = None;
                final_pass::run_job(&self.store, &job, &mut env);
            }
        }
    }

    /// U7: laeuft der Enddurchlauf-/Recovery-Thread noch? Die Import-Warteschlange
    /// beginnt dann nicht auf der gemeinsamen Engine (auch nicht in der Luecke,
    /// bevor sein Auftrag im Verzeichnis steht).
    pub fn final_jobs_active(&self) -> bool {
        self.finals.active()
    }

    /// M2-P2d: wartet auf den Enddurchlauf-/Recovery-Thread (Headless-Laeufe,
    /// deren Prozess sonst vor dem Ende des Auftrags endete).
    pub fn wait_final_jobs(&self) {
        self.finals.wait();
    }

    /// Starts a live meeting: creates the row, the folder and both WAV
    /// writers, wires the capture callbacks into the chunk pipeline and turns
    /// the recording indicator on.
    ///
    /// G1 (#70): auf Wunsch in einen vorhandenen LEEREN Eintrag
    /// (`target_meeting_id`) statt in eine neue Besprechung: Notizen,
    /// Projekte und Id bleiben, der Titel ist der im Startdialog bestaetigte.
    /// Die Einwilligung gilt zuerst, noch vor jeder Zielpruefung. Ist das Ziel
    /// nicht (mehr) leer, kommt `target_not_empty` und nichts startet. Scheitert
    /// der Start danach (Mikrofon, Datei), wird der Eintrag wieder leer, statt als
    /// "fehlgeschlagene Aufnahme" ohne Ton stehenzubleiben.
    pub fn start_into(
        &self,
        title: String,
        consent_confirmed: bool,
        capture_system: bool,
        target_meeting_id: Option<&str>,
    ) -> Result<Meeting, String> {
        self.start_for_event(
            title,
            consent_confirmed,
            capture_system,
            target_meeting_id,
            None,
        )
    }

    /// [`Self::start_into`] mit dem Termin, zu dem die Besprechung gehoert (`event`; sonst
    /// wird er ueber die Verknuepfung des Zieleintrags gesucht). Gehoert der Termin zu einer
    /// Planung (lva#534), entscheidet das Einwilligungs-Tor `planning::start_tor` hier,
    /// unmittelbar vor dem Start, aus dem aktuellen Stand der Datenbank: ohne Zustimmung aller
    /// Anwesenden startet weder Aufnahme noch Transkription (`nur_notizen: <grund>`).
    pub fn start_for_event(
        &self,
        title: String,
        consent_confirmed: bool,
        capture_system: bool,
        target_meeting_id: Option<&str>,
        event: Option<&crate::managers::calendar::model::CalEvent>,
    ) -> Result<Meeting, String> {
        consent_gate(consent_confirmed)?;
        // `Mutex<()>` traegt keine Daten: eine Vergiftung hiesse sonst, dass nie
        // wieder eine Aufnahme startet.
        let _start_guard = lock_recovering(&self.start_guard);
        let _starting = self.core.begin_start()?;

        // A dictation and a meeting would fight over the microphone and the
        // overlay; the meeting yields to the dictation already in progress
        // (the reverse direction is guarded in `actions.rs`).
        if let Some(rm) = self
            .app
            .try_state::<Arc<crate::managers::audio::AudioRecordingManager>>()
        {
            if rm.is_recording() {
                return Err("dictation_active".to_string());
            }
        }

        // Einwilligungs-Tor der Besprechungsplanung (lva#534, lva#544), in drei Schritten:
        // 1. Die Aufnahme meldet sich im `FinalSlot` an (M2-P2d: ein laufender Enddurchlauf gibt
        //    die Engine frei) - VOR dem Tor und ohne dessen Sperre; ab hier beginnt kein
        //    Enddurchlauf-/Recovery-Auftrag mehr mit einem Modell.
        // 2. Unter der Tor-Sperre, nur Datenbank: frische Pruefung, Zeile anlegen und die
        //    geprueften Planungen an die Besprechung BINDEN (`meeting_plan_bindings`). Ab der
        //    Bindung prueft jeder Modellaufruf und jedes Schreiben das Tor mit diesen Planungen,
        //    auch wenn die Verknuepfung mit dem Termin erst spaeter entsteht oder ersetzt wird.
        // 3. `start_row` baut Dateien, Pipeline und Captures OHNE Tor-Sperre auf (Sperrordnung:
        //    keine Sperre von Recorder, Modell-Manager oder Audiotreiber unter dem Tor) und
        //    schaltet die Aufnahme erst nach einer zweiten Pruefung unter der Sperre wirksam.
        // Ohne Planung zum Termin bzw. zur Zielbesprechung: unveraendert.
        let consent_at = chrono::Utc::now().timestamp();
        let ((meeting, gebunden), live) = self.finals.start_unter_tor(
            &self.store,
            event,
            target_meeting_id,
            || {},
            |tor| {
                let meeting = begin_live_row(&self.store, &title, consent_at, target_meeting_id)?;
                match tor.binden(&self.store, &meeting.id) {
                    Ok(gebunden) => Ok((meeting, gebunden)),
                    Err(e) => {
                        self.failed_start_cleanup(target_meeting_id, &meeting.id, &e);
                        Err(e)
                    }
                }
            },
        )?;
        let meeting_id = meeting.id.clone();
        let started = self.start_row(meeting, capture_system, &gebunden, live);
        if let Err(e) = &started {
            self.failed_start_cleanup(target_meeting_id, &meeting_id, e);
            // Nichts wurde aufgenommen: was DIESER Start gebunden hat, faellt wieder weg.
            crate::managers::planning::bindung_verwerfen(&self.store, gebunden);
        }
        started
    }

    /// Ein Start kam nicht zustande: der Zieleintrag wird wieder leer bzw. die neue Besprechung
    /// als `failed` aufgeraeumt. Nur Datenbank und Dateien (darf unter der Tor-Sperre laufen).
    fn failed_start_cleanup(&self, target_meeting_id: Option<&str>, meeting_id: &str, e: &str) {
        match target_meeting_id {
            Some(target) => {
                // Nichts wurde aufgenommen: der Eintrag wird wieder leer.
                match self.store.restore_empty_meeting(target) {
                    Ok(true) => {
                        info!(
                            "meetings: Start gescheitert ({e}), Eintrag {target} ist wieder leer"
                        );
                        if let Ok(dir) = super::meetings_data_dir(&self.app) {
                            let _ = std::fs::remove_dir_all(dir.join(target));
                        }
                    }
                    Ok(false) => {}
                    Err(err) => {
                        warn!("meetings: Eintrag {target} nicht wiederhergestellt: {err}")
                    }
                }
            }
            // #15: eine neue Besprechung bleibt als `failed` stehen, ohne die
            // Null-Header-WAVs und ohne dass der naechste App-Start sie als
            // "Absturzrest" wiederbelebt.
            None => match super::meetings_data_dir(&self.app) {
                Ok(base) => {
                    info!("meetings: Start gescheitert ({e}), {meeting_id} wird aufgeraeumt");
                    discard_failed_start(&self.store, meeting_id, &base.join(meeting_id));
                }
                Err(err) => {
                    warn!("meetings: Start gescheitert, Datenordner unbekannt ({err})");
                    let _ = self.store.set_status(meeting_id, MeetingStatus::Failed);
                }
            },
        }
    }

    /// Der Teil des Starts nach dem Anlegen der Zeile und der Bindung der Planungen. Laeuft OHNE
    /// Tor-Sperre; der wirksame Beginn (ab dem Ton gespeichert und transkribiert wird) ist die
    /// zweite Pruefung unter der Sperre (`Gebunden::wirksam`). `gebunden` beweist, dass das Tor
    /// bestanden und die Planungen an die Besprechung gebunden sind (Compiler statt Absprache).
    fn start_row(
        &self,
        meeting: Meeting,
        capture_system: bool,
        gebunden: &crate::managers::planning::Gebunden,
        live: LiveHalter,
    ) -> Result<Meeting, String> {
        crate::managers::planning::ohne_tor_sperre("Aufbau der Aufnahme (start_row)");
        let meeting_id = meeting.id.clone();
        debug_assert_eq!(gebunden.meeting_id(), meeting_id);

        let dir = super::meetings_data_dir(&self.app)
            .map_err(|e| format!("app_data_dir_failed: {e}"))?
            .join(&meeting_id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("meeting_dir_failed: {e}"))?;

        let mic_path = dir.join("mic.wav");
        let mic_writer = StreamingWavWriter::create(&mic_path, SAMPLE_RATE)
            .map_err(|e| format!("mic_wav_failed: {e}"))?;
        let system_path = capture_system.then(|| dir.join("system.wav"));
        let system_writer = match &system_path {
            Some(path) => Some(
                StreamingWavWriter::create(path, SAMPLE_RATE)
                    .map_err(|e| format!("system_wav_failed: {e}"))?,
            ),
            None => None,
        };

        // Written now, not at stop(): crash recovery can only repair WAVs whose
        // paths it knows, and a crash is exactly the case where stop() never ran.
        if let Err(e) = self.store.set_audio_paths(
            &meeting_id,
            mic_path.to_str(),
            system_path.as_ref().and_then(|p| p.to_str()),
            None,
        ) {
            warn!("meetings: could not persist audio paths: {e}");
        }

        let mic_sink = Arc::new(Mutex::new(ChannelSink::new(mic_writer)));
        let system_sink = system_writer.map(|w| Arc::new(Mutex::new(ChannelSink::new(w))));
        let paused = Arc::new(AtomicBool::new(false));
        // lva#544: bis zum wirksamen Beginn (unten) verwerfen die Capture-Callbacks alles.
        let wirksam = Arc::new(AtomicBool::new(false));
        let levels = Arc::new(LevelEmitter::new(self.app.clone()));

        // M2-P2c2: Echo-Unterdrueckung der Ich-Spur (braucht den Systemton als
        // Referenz). Das Ergebnis geht an die Transkription und nach
        // mic_aec.wav; mic.wav bleibt roh.
        let echo_mode = crate::settings::get_settings(&self.app).meeting_echo_cancellation;
        let echo = echo_mode
            .enabled(capture_system)
            .then(|| self.echo_setup(&meeting_id, &dir.join(super::MIC_AEC_FILE)));

        let pipeline = self.start_pipeline(&meeting_id, echo).inspect_err(|_| {
            let _ = self.store.set_status(&meeting_id, MeetingStatus::Failed);
        })?;
        let mic_feed = pipeline.feed(CHANNEL_MIC).ok_or("dsp_thread_failed")?;
        let system_feed = pipeline.feed(CHANNEL_SYSTEM);
        let dsp_control = pipeline.control();

        // Load the meeting model (dedicated `meeting_model` or the dictation
        // model as fallback); transcribe_segments waits on the load condvar.
        self.transcription
            .initiate_meeting_model_load(&crate::settings::get_settings(&self.app));

        // #15: was die begrenzte Queue des Mikrofons verwirft, zaehlt auf demselben
        // Zaehler wie der Ueberlauf der DSP-Queue (`overflow_ms[mic=..]` im Log).
        let overflow_stats = pipeline.stats();
        let mic_overflow: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |samples| {
            overflow_stats.overflow_samples[CHANNEL_MIC as usize]
                .fetch_add(samples, Ordering::Relaxed);
        });
        let mic_capture = MeetingMicCapture::start(
            crate::settings::get_settings(&self.app).selected_microphone,
            mic_overflow,
            erst_wenn_wirksam(
                Arc::clone(&wirksam),
                channel_callback(
                    CHANNEL_MIC,
                    Arc::clone(&mic_sink),
                    Arc::clone(&paused),
                    mic_feed,
                    Arc::clone(&levels),
                ),
            ),
        )
        .map_err(|e| {
            // The meeting row exists but nothing was captured — mark it failed
            // rather than leaving a phantom "recording" row behind.
            let _ = self.store.set_status(&meeting_id, MeetingStatus::Failed);
            format!("mic_start_failed: {e}")
        })?;

        let loopback = match (&system_sink, system_feed, capture_system) {
            (Some(sink), Some(feed), true) => self.start_loopback(
                &meeting_id,
                erst_wenn_wirksam(
                    Arc::clone(&wirksam),
                    channel_callback(
                        CHANNEL_SYSTEM,
                        Arc::clone(sink),
                        Arc::clone(&paused),
                        feed,
                        Arc::clone(&levels),
                    ),
                ),
                dsp_control.clone(),
            ),
            _ => None,
        };
        if loopback.is_none() {
            // Ohne Referenz keine Echo-Unterdrueckung (No-op, wenn sie aus ist).
            if let Some(control) = &dsp_control {
                control.reference_lost();
            }
        }

        // lva#544: der WIRKSAME Beginn. Unter der Tor-Sperre wird das Tor ein zweites Mal frisch
        // geprueft (ein Widerruf seit der Bindung gewinnt) und nur der Merker umgelegt: erst ab
        // jetzt speichern die Callbacks Ton und speisen die Transkription. Scheitert die Pruefung,
        // wurde nichts aufgenommen: Captures zu, der Start gilt als gescheitert.
        if let Err(code) = gebunden.wirksam(&self.store, || {
            wirksam.store(true, Ordering::Release);
        }) {
            info!("meetings: Start nicht wirksam ({meeting_id}, {code})");
            mic_capture.stop();
            if let Some(capture) = loopback {
                capture.stop();
            }
            drop(pipeline);
            let _ = self.store.set_status(&meeting_id, MeetingStatus::Failed);
            return Err(code);
        }

        // Zustand und Sitzung zusammen und erst jetzt, wo die Captures laufen (#15).
        self.core.publish_started(
            &meeting_id,
            RecordingSession {
                meeting_id: meeting_id.clone(),
                paused,
                mic_capture: Some(mic_capture),
                loopback,
                mic_sink,
                system_sink,
                mic_path,
                system_path,
                pipeline: Some(pipeline),
                live,
            },
        );

        self.set_indicator(true);
        self.emit_state(&meeting_id, "recording", false);
        info!("meetings: recording started ({meeting_id})");
        Ok(meeting)
    }

    /// Runs `LoopbackCapture::start` on a throwaway thread with a bounded
    /// wait: a wedged audio driver must cost the meeting its system channel,
    /// not the whole recording (known Task-4 finding). On timeout the started
    /// capture — if it ever arrives — is dropped by the send error, which
    /// stops it.
    fn start_loopback(
        &self,
        meeting_id: &str,
        callback: impl FnMut(&[i16], Option<u64>) + Send + 'static,
        dsp_control: Option<DspControl>,
    ) -> Option<LoopbackCapture> {
        crate::managers::planning::ohne_tor_sperre("Start des Systemtons");
        let (tx, rx) = mpsc::channel::<Result<LoopbackCapture, String>>();
        std::thread::Builder::new()
            .name("meeting-loopback-start".to_string())
            .spawn(move || {
                let result = LoopbackCapture::start(callback).map_err(|e| format!("{e:#}"));
                let _ = tx.send(result);
            })
            .ok()?;

        match rx.recv_timeout(LOOPBACK_START_TIMEOUT) {
            Ok(Ok(capture)) => {
                self.watch_loopback(meeting_id, &capture, dsp_control);
                Some(capture)
            }
            Ok(Err(e)) => {
                warn!("meetings: loopback start failed ({e}) — continuing mic-only");
                self.emit_error(meeting_id, "loopback_start_failed");
                None
            }
            Err(_) => {
                warn!("meetings: loopback start timed out — continuing mic-only");
                self.emit_error(meeting_id, "loopback_start_timeout");
                None
            }
        }
    }

    /// Worst failure mode of the system channel: the loopback thread dies
    /// AFTER a successful start (endpoint removed, driver error). The callback
    /// simply goes quiet, the meeting keeps recording and half the minutes are
    /// missing without anyone noticing. This watchdog turns that silence into
    /// an error event; it ends with the capture (stop flag) or right after it
    /// reported the failure.
    fn watch_loopback(
        &self,
        meeting_id: &str,
        capture: &LoopbackCapture,
        dsp_control: Option<DspControl>,
    ) {
        let (stopped, failed) = capture.watch_flags();
        let app = self.app.clone();
        let meeting_id = meeting_id.to_string();
        let _ = std::thread::Builder::new()
            .name("meeting-loopback-watch".to_string())
            .spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    if failed.load(Ordering::Relaxed) {
                        warn!(
                            "meetings: loopback capture died mid-meeting ({meeting_id})                              - system audio is gone, the meeting continues mic-only"
                        );
                        // M2-P2c2: ohne Referenz Echo-Unterdrueckung aus.
                        if let Some(control) = &dsp_control {
                            control.reference_lost();
                        }
                        let _ = (MeetingEvent::Health {
                            meeting_id: meeting_id.clone(),
                            channel: CHANNEL_SYSTEM,
                            state: HealthState::LoopbackDied,
                        })
                        .emit(&app);
                        let _ = (MeetingEvent::Error {
                            meeting_id,
                            message: "loopback_died".to_string(),
                        })
                        .emit(&app);
                        return;
                    }
                    std::thread::sleep(LOOPBACK_WATCH_INTERVAL);
                }
            });
    }

    /// DSP thread + transcription worker of one meeting. The worker is a single
    /// thread on purpose (FIFO, one engine): a failed block is logged by length
    /// only (never content) and does not end the meeting.
    fn start_pipeline(
        &self,
        meeting_id: &str,
        echo: Option<EchoSetup>,
    ) -> Result<LivePipeline, String> {
        let app = self.app.clone();
        let transcription = Arc::clone(&self.transcription);
        let emit_app = self.app.clone();

        // VAD model: bundled resource. If it is missing or does not load, the
        // DSP thread falls back to the 20-s chunker per channel.
        let vad_factory = meeting_vad_factory(&self.app);

        // Channel health goes to the UI (P2e). This runs on the DSP thread, not
        // in a capture callback, and only on state changes.
        let notice_meeting = meeting_id.to_string();
        let notice_app = self.app.clone();
        let notice: super::dsp::NoticeFn = Arc::new(move |n: DspNotice| {
            debug!("meetings: dsp notice for {notice_meeting}: {n:?}");
            if let Some(event) = health_event(&notice_meeting, &n) {
                let _ = event.emit(&notice_app);
            }
        });

        let mut cfg = DspConfig::new(vad_factory, notice);
        if let Some(echo) = echo {
            cfg = cfg.with_echo(echo);
        }

        let gate_store = Arc::clone(&self.store);
        let gate_meeting = meeting_id.to_string();
        let refusal_told = AtomicBool::new(false);
        let refusal_app = self.app.clone();
        LivePipeline::start(
            meeting_id.to_string(),
            Arc::clone(&self.store),
            cfg,
            move |chunk: &Chunk| {
                live_block_gated(
                    &gate_meeting,
                    &refusal_told,
                    || {
                        super::import::transcribe_chunk_resilient(
                            &app,
                            &transcription,
                            &gate_store,
                            &gate_meeting,
                            chunk,
                        )
                    },
                    || {
                        let _ = (MeetingEvent::Error {
                            meeting_id: gate_meeting.clone(),
                            message: "nur_notizen".to_string(),
                        })
                        .emit(&refusal_app);
                    },
                )
            },
            move |event| {
                let _ = event.emit(&emit_app);
            },
        )
    }

    /// M2-P2c2: `mic_aec.wav` + Ablage der Zeitachse in `metadata_json`.
    /// Laesst sich die Datei nicht anlegen (Platte voll), laeuft die
    /// Echo-Unterdrueckung trotzdem fuer die Transkription.
    fn echo_setup(&self, meeting_id: &str, aec_path: &std::path::Path) -> EchoSetup {
        let sink = match WavFileSink::create(aec_path) {
            Ok(sink) => Some(Box::new(sink) as Box<dyn PcmSink>),
            Err(e) => {
                warn!("meetings: mic_aec.wav not created ({e}) - echo cancellation for the transcript only");
                None
            }
        };
        EchoSetup {
            sink,
            on_timeline: Some(timeline_writer(Arc::clone(&self.store), meeting_id)),
        }
    }

    pub fn pause(&self) -> Result<(), String> {
        self.set_paused(true)
    }

    pub fn resume(&self) -> Result<(), String> {
        self.set_paused(false)
    }

    fn set_paused(&self, paused: bool) -> Result<(), String> {
        let meeting_id = self.core.set_paused(paused, |session| {
            // Paused means "discard samples"; wall-clock keeps running, so the
            // WAV timeline compresses the pause instead of padding it. That is
            // deliberate and consistent: transcript offsets come from the DSP
            // thread, which counts the same samples (it also closes the open
            // segment when the callback reports the pause).
            session.paused.store(paused, Ordering::Relaxed);
        })?;
        self.emit_state(&meeting_id, "recording", paused);
        Ok(())
    }

    /// Stops capture, drains the tail chunks (status `processing` while that
    /// runs) and finalizes both WAVs. Then the final pass (P2d): when it runs,
    /// the meeting stays `processing` and a background job replaces the live
    /// transcript, marks it `ready` and sends `TranscriptFinal`; when the live
    /// transcript stays (setting `off`, CPU only, no model, not enough memory),
    /// that happens here at once. Returns as soon as the live worker is empty.
    /// Blocking — call it off the UI thread.
    pub fn stop(&self) -> Result<String, String> {
        // #15: der Zustand bleibt `Recording`, bis die Captures wirklich zu sind
        // (das Ticket stellt ihn auf `Idle`, auch bei einer Panik): vorher galt die
        // Aufnahme schon als beendet, waehrend das Mikrofon noch offen war, und ein
        // Diktat-Hotkey oder ein neuer Start konnte dazwischen greifen.
        let (session, stopping) = self.core.begin_stop()?;
        let RecordingSession {
            meeting_id,
            mic_capture,
            loopback,
            mic_sink,
            system_sink,
            mic_path,
            system_path,
            pipeline,
            live,
            ..
        } = session;

        // Captures first: once they are stopped no callback can touch the
        // sinks or the DSP queue any more, so draining and finalizing below is
        // race-free.
        if let Some(capture) = mic_capture {
            if capture.had_error() {
                self.emit_error(&meeting_id, "mic_stream_error");
            }
            capture.stop();
        }
        if let Some(capture) = loopback {
            // The watchdog above has already reported this while the meeting
            // was running; the log line here is what makes it visible in the
            // evidence of a finished meeting.
            if capture.had_error() {
                warn!("meetings: loopback capture had ended with an error ({meeting_id})");
            }
            capture.stop();
        }
        // Mikrofon und Loopback sind zu: jetzt ist die Maschine nicht mehr in
        // Aufnahme. ZUERST der Zustand, DANACH der Indikator: `hide_recording_overlay`
        // fragt den Zustand und stellte den Indikator sonst wieder auf.
        stopping.finish();
        self.set_indicator(false);

        if let Err(e) = self
            .store
            .set_status(&meeting_id, MeetingStatus::Processing)
        {
            warn!("meetings: status 'processing' not stored: {e}");
        }
        self.emit_state(&meeting_id, "processing", false);

        // DSP thread first (flushes the open segments), then the worker: FIFO,
        // everything queued is transcribed and stored before this returns.
        if let Some(pipeline) = pipeline {
            let stats = pipeline.drain();
            // M2-P2c2: Bericht der Echo-Unterdrueckung (Zahlen, ob mic_aec.wav
            // gueltig ist) fuer den Enddurchlauf und die Fehlersuche.
            if let Some(summary) = stats.echo_summary() {
                store_echo_summary(&self.store, &meeting_id, &summary);
            }
        }
        // lva#544: der Live-Worker ist leer, diese Aufnahme laesst kein Modell mehr laufen: die
        // Anmeldung im `FinalSlot` faellt, BEVOR der Enddurchlauf dieser Aufnahme entsteht.
        drop(live);

        // The engine that produced the live transcript, read BEFORE the
        // dictation model is restored below.
        let live_model = self.transcription.get_current_model();

        // M2-P2d: which model runs the final pass, if any (setting, GPU, RAM,
        // VRAM). Only when none does, the dictation model comes back right
        // away; otherwise the job restores it after the final model.
        let settings = crate::settings::get_settings(&self.app);
        let plan = final_pass::plan_for_app(
            &self.app,
            &FinalChoice::parse(&settings.meeting_final_model),
        );
        if matches!(plan, FinalPlan::Keep(_)) {
            // Restore the dictation model so the next hotkey dictation does not
            // silently run on the meeting model (no-op when they are the same).
            self.transcription
                .initiate_model_load_target(&settings.selected_model);
        }

        let mic_ms = finalize_sink(&mic_sink).unwrap_or(0);
        let system_ms = system_sink.as_ref().and_then(finalize_sink).unwrap_or(0);
        let duration_ms = mic_ms.max(system_ms);

        if let Err(e) = self.store.set_audio_paths(
            &meeting_id,
            mic_path.to_str(),
            system_path.as_ref().and_then(|p| p.to_str()),
            Some(duration_ms),
        ) {
            warn!("meetings: audio paths not stored: {e}");
        }
        // Retention starts counting from the moment the meeting ends; no
        // minutes document exists yet at this point. `ended_at` is recorded
        // here so later recomputations (e.g. after minutes generation) stay
        // anchored to this moment instead of drifting to whenever they run.
        let now = chrono::Utc::now().timestamp();
        if let Err(e) = self.store.set_ended_at(&meeting_id, now) {
            warn!("meetings: ended_at not stored: {e}");
        }
        let policy = crate::settings::get_meeting_audio_retention(&self.app);
        let until = super::retention::retention_until(&policy, now, now, false);
        if let Err(e) = self.store.set_retention_until(&meeting_id, until) {
            warn!("meetings: retention_until not stored: {e}");
        }
        info!("meetings: recording stopped ({meeting_id}, {duration_ms} ms, final pass {plan:?})");
        // `ready` + `TranscriptFinal` come from the job: at once when the live
        // transcript stays, after the final pass otherwise.
        let job = JobSpec {
            meeting_id: meeting_id.clone(),
            catch_up_model: None,
            plan,
            live_model,
            recover_from: None,
        };
        // M3-P3b: die Sprechertrennung kann Minuten dauern (CPU: ~2 min je Stunde
        // Audio), also nie im Aufruf von `stop()`: mit ihr laeuft der Auftrag
        // immer im Hintergrund-Thread, auch wenn das Live-Transkript bleibt.
        let diarization =
            crate::settings::meeting_diarization_enabled(&settings.meeting_diarization);
        if matches!(job.plan, FinalPlan::Keep(_)) && !diarization {
            let mut env = final_pass::AppEnv::new(
                &self.app,
                Arc::clone(&self.transcription),
                Arc::new(AtomicBool::new(false)),
            );
            final_pass::run_job(&self.store, &job, &mut env);
        } else {
            self.spawn_final_jobs(vec![job]);
        }
        Ok(meeting_id)
    }

    /// App start: a meeting still marked `recording` or `processing` means the
    /// app died mid recording or mid final pass. Its WAV headers may claim
    /// zero length, so repair them from the file size, store the real
    /// `duration_ms`, and hand the meeting to a background job (status
    /// `processing`): it transcribes each channel's rest after the last stored
    /// segment, runs the final pass when one applies, then marks it `ready`
    /// and sends `TranscriptFinal` (M2-P2d, concept 3.6). Stored segments are
    /// durable — every delta was committed as it arrived.
    pub fn recover_orphans(&self) {
        let settings = crate::settings::get_settings(&self.app);
        let choice = FinalChoice::parse(&settings.meeting_final_model);
        let catch_up_model = self.transcription.meeting_model_target(&settings);
        let mut jobs: Vec<JobSpec> = Vec::new();
        let mut offset = 0u32;
        let mut recovered: Vec<String> = Vec::new();
        loop {
            let page = match self.store.list_meetings(offset, RECOVERY_PAGE) {
                Ok(page) => page,
                Err(e) => {
                    warn!("meetings: orphan scan failed: {e}");
                    return;
                }
            };
            let page_len = page.len() as u32;
            for meeting in page {
                if !is_orphan_status(&meeting.status) {
                    continue;
                }
                // lva#523: WAV-Header, Dauer und `processing` erst unter dem Auftrag
                // (`run_job`, `recover_from`): eine Ablehnung aendert nichts.
                let plan = if meeting.source == "live" {
                    final_pass::plan_for_app(&self.app, &choice)
                } else {
                    FinalPlan::Keep(KeepReason::NotLive)
                };
                jobs.push(JobSpec {
                    meeting_id: meeting.id.clone(),
                    catch_up_model: Some(catch_up_model.clone()),
                    plan,
                    live_model: None,
                    recover_from: Some(meeting.status.clone()),
                });
                recovered.push(meeting.id);
            }
            if page_len < RECOVERY_PAGE {
                break;
            }
            offset += page_len;
        }
        if !recovered.is_empty() {
            info!(
                "meetings: recovered {} orphan(s): {:?}",
                recovered.len(),
                recovered
            );
        }
        self.spawn_final_jobs(jobs);
    }

    /// P8a: "Fortsetzen" nach einem Stopp durch den Nutzer: holt den Rest einer
    /// abgebrochenen Verarbeitung nach, mit derselben Logik wie die
    /// Wiederherstellung nach einem Absturz (`recover_orphans`): je Kanal ab dem
    /// letzten gespeicherten Segment, danach der Plan der Besprechung. Laeuft im
    /// Hintergrund-Thread mit Fortschritt, Pause und Stopp. Blockierend
    /// (Hardware-Messung fuer den Plan): nicht im UI-Thread rufen.
    pub fn continue_processing(&self, meeting_id: &str) -> Result<(), String> {
        let meeting = self
            .store
            .get_meeting(meeting_id)
            .map_err(|e| format!("meeting_lookup_failed: {e}"))?
            .ok_or_else(|| "meeting_not_found".to_string())?;
        check_can_continue(
            &meeting,
            super::job::global().is_running(meeting_id),
            &|p| std::path::Path::new(p).exists(),
        )?;
        // lva#544: neben einer Aufnahme laeuft kein Nachholen (der Auftrag wuerde abgebrochen
        // geboren); der Nutzer erfaehrt es hier.
        if self.finals.live_aktiv() {
            return Err("already_recording".to_string());
        }
        // lva#534: ohne Zustimmung gibt es nichts fortzusetzen: der Nutzer erfaehrt es hier, der
        // Auftrag (`run_job`) entscheidet das Tor selbst noch einmal.
        crate::managers::planning::tor_pruefen_besprechung(&self.store, meeting_id)?;
        let settings = crate::settings::get_settings(&self.app);
        let catch_up_model = self.transcription.meeting_model_target(&settings);
        let plan = if meeting.source == "live" {
            final_pass::plan_for_app(
                &self.app,
                &FinalChoice::parse(&settings.meeting_final_model),
            )
        } else {
            FinalPlan::Keep(KeepReason::NotLive)
        };
        // lva#523: WAV-Header, Dauer und `processing` (samt Anzeige) erst unter dem
        // Auftrag (`run_job`, `recover_from`); eine Ablehnung aendert nichts.
        info!("meetings: continuing a stopped meeting ({meeting_id})");
        self.spawn_final_jobs(vec![JobSpec {
            meeting_id: meeting_id.to_string(),
            catch_up_model: Some(catch_up_model),
            plan,
            live_model: None,
            recover_from: Some(meeting.status.clone()),
        }]);
        Ok(())
    }

    /// Spec A1: while a meeting records, the machine must show it — tray icon
    /// plus an overlay notice that is visible even at `overlay_style: none`.
    fn set_indicator(&self, on: bool) {
        if on {
            crate::tray::change_tray_icon(&self.app, crate::tray::TrayIconState::Recording);
            crate::overlay::show_persistent_notice(
                &self.app,
                crate::overlay::MEETING_RECORDING_NOTICE_KEY,
            );
        } else {
            crate::overlay::hide_recording_overlay(&self.app);
            crate::tray::change_tray_icon(&self.app, crate::tray::TrayIconState::Idle);
        }
    }

    fn emit_state(&self, meeting_id: &str, status: &str, paused: bool) {
        let _ = (MeetingEvent::State {
            meeting_id: meeting_id.to_string(),
            status: status.to_string(),
            paused,
        })
        .emit(&self.app);
    }

    fn emit_error(&self, meeting_id: &str, message: &str) {
        let _ = (MeetingEvent::Error {
            meeting_id: meeting_id.to_string(),
            message: message.to_string(),
        })
        .emit(&self.app);
    }

    // M1-P1c (Beruehrpunkt B2): Audioposition fuer den Notizblock.
    /// The running meeting's id and the microphone audio position in
    /// milliseconds, or `None` when nothing is being recorded. Same timeline
    /// as `StoredSegment.start_ms` (paused stretches are compressed).
    pub fn position_ms(&self) -> Option<(String, u64)> {
        self.core
            .with_session(|session| session.and_then(Self::session_position))
    }

    /// `None` once the mic WAV writer is gone (RIFF limit reached): a wrong
    /// position would be worse than none, the notepad then stores no timestamp.
    fn session_position(session: &RecordingSession) -> Option<(String, u64)> {
        let sink = session.mic_sink.lock().ok()?;
        let ms = sink.writer.as_ref()?.position_ms();
        Some((session.meeting_id.clone(), ms))
    }
}

#[cfg(test)]
mod position_tests {
    use super::*;

    fn session_with(dir: &std::path::Path, writer: Option<StreamingWavWriter>) -> RecordingSession {
        let mut sink = ChannelSink::new(
            StreamingWavWriter::create(&dir.join("placeholder.wav"), 16_000).unwrap(),
        );
        sink.writer = writer;
        RecordingSession {
            meeting_id: "m-pos".to_string(),
            paused: Arc::new(AtomicBool::new(false)),
            mic_capture: None,
            loopback: None,
            mic_sink: Arc::new(Mutex::new(sink)),
            system_sink: None,
            mic_path: PathBuf::new(),
            system_path: None,
            pipeline: None,
            live: FinalSlot::default().live_beginnen().unwrap(),
        }
    }

    #[test]
    fn position_tracks_the_mic_writer_of_a_running_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = StreamingWavWriter::create(&dir.path().join("mic.wav"), 16_000).unwrap();
        w.append(&vec![0i16; 48_000]).unwrap(); // 3 s
        let session = session_with(dir.path(), Some(w));
        assert_eq!(
            MeetingRecorderManager::session_position(&session),
            Some(("m-pos".to_string(), 3_000))
        );
        // Weiterschreiben ueber den Sink: die Position laeuft mit.
        session
            .mic_sink
            .lock()
            .unwrap()
            .writer
            .as_mut()
            .unwrap()
            .append(&vec![0i16; 16_000])
            .unwrap();
        assert_eq!(
            MeetingRecorderManager::session_position(&session),
            Some(("m-pos".to_string(), 4_000))
        );
    }

    #[test]
    fn position_is_none_without_a_mic_writer() {
        let dir = tempfile::tempdir().unwrap();
        let session = session_with(dir.path(), None);
        assert_eq!(MeetingRecorderManager::session_position(&session), None);
    }
}

/// The meetings VAD (bundled Silero v4) as a factory for the DSP thread, or
/// `None` when the resource path does not resolve (DSP falls back to the
/// 20-s chunker). Shared by the live recorder and `--simulate-meeting`.
pub(crate) fn meeting_vad_factory(app: &AppHandle) -> Option<VadFactory> {
    match app
        .path()
        .resolve(VAD_MODEL_RESOURCE, tauri::path::BaseDirectory::Resource)
    {
        Ok(path) => Some(Arc::new(move || {
            let vad = crate::audio_toolkit::SileroVad::new(&path, MEETING_VAD_THRESHOLD)?;
            Ok(Box::new(vad) as Box<dyn crate::audio_toolkit::VoiceActivityDetector>)
        })),
        Err(e) => {
            warn!("meetings: VAD model path not resolved: {e}");
            None
        }
    }
}

/// M2-P2c2: legt die Zeitachse (`{mic_qpc0, sys_qpc0, offset_ms, basis}`)
/// in `meetings.metadata_json.timeline` ab, sobald der DSP-Thread sie kennt –
/// waehrend der Aufnahme, damit sie auch einen Absturz ueberlebt.
pub(crate) fn timeline_writer(
    store: Arc<MeetingStore>,
    meeting_id: &str,
) -> super::dsp::TimelineFn {
    let meeting_id = meeting_id.to_string();
    Arc::new(move |timeline: &MeetingTimeline| {
        let value = serde_json::to_value(timeline).unwrap_or_default();
        if let Err(e) = store.set_metadata_key(&meeting_id, "timeline", value) {
            warn!("meetings: timeline not stored: {e}");
        }
    })
}

/// M2-P2c2: Abschlussbericht der Echo-Unterdrueckung nach `metadata_json.aec`.
pub(crate) fn store_echo_summary(
    store: &MeetingStore,
    meeting_id: &str,
    summary: &super::dsp::EchoSummary,
) {
    let mut value = serde_json::to_value(summary).unwrap_or_default();
    if summary.wav_kept {
        value["file"] = serde_json::json!(super::MIC_AEC_FILE);
    }
    if let Err(e) = store.set_metadata_key(meeting_id, "aec", value) {
        warn!("meetings: aec summary not stored: {e}");
    }
}

/// The per-channel capture callback: WAV append (with 1-s header flush), hand-off
/// to the DSP thread, and the throttled level readout. The hand-off is a
/// non-blocking `try_send` (see `dsp::ChannelFeed`); segmentation and VAD never
/// run here. The second argument is the QPC stamp of the block's first sample
/// (P2c2), passed through untouched.
fn channel_callback(
    channel: u8,
    sink: Arc<Mutex<ChannelSink>>,
    paused: Arc<AtomicBool>,
    mut feed: ChannelFeed,
    levels: Arc<LevelEmitter>,
) -> impl FnMut(&[i16], Option<u64>) + Send + 'static {
    move |samples: &[i16], qpc: Option<u64>| {
        if paused.load(Ordering::Relaxed) {
            feed.pause();
            return;
        }
        {
            let Ok(mut sink) = sink.lock() else {
                return;
            };
            let mut writer_failed = false;
            if let Some(writer) = sink.writer.as_mut() {
                if let Err(e) = writer.append(samples) {
                    // Also the RIFF 4-GB limit: stop writing this file rather
                    // than corrupting it; capture and transcription continue.
                    warn!("meetings: wav append stopped on channel {channel}: {e}");
                    writer_failed = true;
                }
            }
            if writer_failed {
                sink.writer = None;
            } else {
                sink.samples_since_flush += samples.len();
                if sink.samples_since_flush >= FLUSH_EVERY_SAMPLES {
                    sink.samples_since_flush = 0;
                    if let Some(writer) = sink.writer.as_mut() {
                        let _ = writer.flush_header();
                    }
                }
            }
        }
        feed.push_stamped(samples, qpc);
        levels.record(channel, rms(samples));
    }
}

/// Finalizes a channel's WAV and returns its duration in milliseconds.
fn finalize_sink(sink: &Arc<Mutex<ChannelSink>>) -> Option<u64> {
    let writer = sink.lock().ok()?.writer.take()?;
    match writer.finalize() {
        Ok(ms) => Some(ms),
        Err(e) => {
            warn!("meetings: wav finalize failed: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_channel_states_become_health_events() {
        let ev = |n| health_event("m1", &n);
        match ev(DspNotice::Health {
            channel: 0,
            state: HealthState::Silent,
        }) {
            Some(MeetingEvent::Health {
                meeting_id,
                channel: 0,
                state: HealthState::Silent,
            }) => assert_eq!(meeting_id, "m1"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            ev(DspNotice::VadUnavailable { channel: 1 }),
            Some(MeetingEvent::Health {
                channel: 1,
                state: HealthState::VadUnavailable,
                ..
            })
        ));
        // Roher Ueberlauf-Zaehler und Panik-Rueckfaelle bleiben im Log.
        assert!(ev(DspNotice::Overflow {
            channel: 0,
            skipped_ms: 5
        })
        .is_none());
        assert!(ev(DspNotice::DspPanic { channel: 0 }).is_none());
        assert!(ev(DspNotice::AecPanic).is_none());
    }

    #[test]
    fn the_health_event_wire_format_is_kind_health_with_snake_case_state() {
        let json = serde_json::to_value(MeetingEvent::Health {
            meeting_id: "m1".into(),
            channel: 1,
            state: HealthState::LoopbackDied,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "health",
                "meeting_id": "m1",
                "channel": 1,
                "state": "loopback_died"
            })
        );
    }

    #[test]
    fn start_without_consent_is_refused() {
        assert_eq!(consent_gate(false), Err("consent_required".to_string()));
        assert!(consent_gate(true).is_ok());
    }

    #[test]
    fn only_one_meeting_records_at_a_time() {
        let s = MeetingRunState::Recording {
            meeting_id: "m1".into(),
            paused: false,
        };
        assert!(!may_start(&s));
        assert!(may_start(&MeetingRunState::Idle));
    }

    #[test]
    fn pause_and_resume_toggle_only_in_recording() {
        let mut s = MeetingRunState::Recording {
            meeting_id: "m".into(),
            paused: false,
        };
        assert!(apply_pause(&mut s, true).is_ok());
        assert!(matches!(
            &s,
            MeetingRunState::Recording { paused: true, .. }
        ));
        let mut idle = MeetingRunState::Idle;
        assert!(apply_pause(&mut idle, true).is_err());
    }

    #[test]
    fn resume_clears_the_pause_flag() {
        let mut s = MeetingRunState::Recording {
            meeting_id: "m".into(),
            paused: true,
        };
        assert!(apply_pause(&mut s, false).is_ok());
        assert!(matches!(
            &s,
            MeetingRunState::Recording { paused: false, .. }
        ));
    }

    #[test]
    fn rms_of_silence_is_zero_and_full_scale_is_one() {
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(rms(&[0, 0, 0, 0]), 0.0);
        assert!((rms(&[i16::MAX, -i16::MAX]) - 1.0).abs() < 1e-4);
    }

    /// P8a: der Start nimmt nur Reste eines Absturzes wieder auf. Ein Stopp
    /// durch den Nutzer (`cancelled`) bleibt gestoppt.
    #[test]
    fn only_crash_leftovers_are_recovered_and_a_stopped_meeting_stays_stopped() {
        assert!(is_orphan_status("recording"));
        assert!(is_orphan_status("processing"));
        assert!(!is_orphan_status("cancelled"));
        assert!(!is_orphan_status("ready"));
        assert!(!is_orphan_status("failed"));
    }

    /// G1 (#70): die Aufnahme laeuft in einem leeren Eintrag, wenn ein Ziel
    /// angegeben ist: dieselbe Besprechung (Projekt, Notizen), Status
    /// `recording`, die Einwilligung am Eintrag. Ein nicht leeres Ziel wird
    /// abgelehnt, ohne etwas zu aendern.
    #[test]
    fn a_recording_can_start_in_an_empty_entry_but_not_in_anything_else() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        let folder = store.folder_save(None, "Kunde", None).unwrap().id;
        let target = store
            .create_empty_meeting("Neue Besprechung", Some(&folder))
            .unwrap();

        let started = begin_live_row(&store, "Montag", 99, Some(&target.id)).unwrap();
        assert_eq!(started.id, target.id, "kein neuer Eintrag");
        assert_eq!(
            (started.source.as_str(), started.status.as_str()),
            ("live", "recording")
        );
        assert_eq!(started.title, "Montag");
        assert_eq!(started.consent_confirmed_at, Some(99));
        assert_eq!(store.meeting_folder_ids(&target.id).unwrap(), vec![folder]);
        assert_eq!(store.list_meetings(0, 10).unwrap().len(), 1);

        // Ein zweiter Start in dasselbe Ziel: nicht mehr leer.
        assert_eq!(
            begin_live_row(&store, "Nochmal", 100, Some(&target.id)).unwrap_err(),
            "target_not_empty"
        );
        assert_eq!(
            store.get_meeting(&target.id).unwrap().unwrap().title,
            "Montag"
        );
        assert_eq!(
            begin_live_row(&store, "x", 1, Some("gibt-es-nicht")).unwrap_err(),
            "meeting_not_found"
        );

        // Ohne Ziel bleibt es eine neue Besprechung.
        let fresh = begin_live_row(&store, "Neu", 5, None).unwrap();
        assert_ne!(fresh.id, target.id);
        assert_eq!(store.list_meetings(0, 10).unwrap().len(), 2);
    }

    /// Ohne bestaetigte Einwilligung startet nichts, auch nicht in einen leeren
    /// Eintrag: die Pflicht gilt vor jeder Zielpruefung.
    #[test]
    fn the_consent_gate_comes_before_any_target() {
        assert_eq!(consent_gate(false), Err("consent_required".to_string()));
    }

    fn meeting_with(status: &str, mic: Option<&str>, system: Option<&str>) -> Meeting {
        Meeting {
            id: "m1".into(),
            title: "t".into(),
            status: status.into(),
            source: "import".into(),
            started_at: None,
            ended_at: None,
            language: None,
            mic_audio_path: mic.map(str::to_string),
            system_audio_path: system.map(str::to_string),
            duration_ms: None,
            consent_confirmed_at: None,
            audio_retention_until: None,
            source_path: None,
            description: None,
            created_at: 0,
            deleted_at: None,
        }
    }

    #[test]
    fn continuing_needs_a_stopped_meeting_no_running_job_and_audio_on_disk() {
        let exists = |p: &str| p == "C:/m/import.wav";
        let ok = meeting_with("cancelled", Some("C:/m/import.wav"), None);
        assert_eq!(check_can_continue(&ok, false, &exists), Ok(()));
        // Nur die zweite Spur genuegt.
        let system_only = meeting_with("cancelled", Some("C:/weg.wav"), Some("C:/m/import.wav"));
        assert_eq!(check_can_continue(&system_only, false, &exists), Ok(()));
        for status in ["ready", "failed", "processing", "recording"] {
            let m = meeting_with(status, Some("C:/m/import.wav"), None);
            assert_eq!(
                check_can_continue(&m, false, &exists),
                Err("not_cancelled".to_string()),
                "{status}"
            );
        }
        assert_eq!(
            check_can_continue(&ok, true, &exists),
            Err("job_busy".to_string()),
            "ein zweiter Klick startet keinen zweiten Auftrag"
        );
        let gone = meeting_with("cancelled", Some("C:/weg.wav"), None);
        assert_eq!(
            check_can_continue(&gone, false, &exists),
            Err("audio_missing".to_string())
        );
        let none = meeting_with("cancelled", None, None);
        assert_eq!(
            check_can_continue(&none, false, &exists),
            Err("audio_missing".to_string())
        );
    }

    // ---- #15: Zustandsfenster, Vergiftung, Geisteraufnahme ---------------------------

    type Core = RunCore<&'static str>;

    #[test]
    fn a_poisoned_state_lock_does_not_take_the_hotkey_path_down() {
        let core = Arc::new(Core::new());
        let poisoner = Arc::clone(&core);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.state.lock().unwrap();
            panic!("Absturz mit gehaltener Sperre");
        })
        .join();
        assert!(core.state.is_poisoned(), "Voraussetzung des Tests");
        // Der Diktat-Hotkey fragt genau das: keine Panik, richtige Antwort.
        assert!(!core.is_recording());
        assert!(core.is_idle());
        assert!(core.begin_start().is_ok(), "ein Start bleibt moeglich");
    }

    #[test]
    fn a_poisoned_session_lock_does_not_break_pause_or_position() {
        let core = Arc::new(Core::new());
        core.publish_started("m1", "sitzung");
        let poisoner = Arc::clone(&core);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.session.lock().unwrap();
            panic!("Absturz mit gehaltener Sperre");
        })
        .join();
        assert_eq!(core.with_session(|s| s.copied()), Some("sitzung"));
        assert_eq!(core.set_paused(true, |_| {}), Ok("m1".to_string()));
    }

    #[test]
    fn the_state_stays_recording_until_the_stop_has_closed_the_capture() {
        let core = Core::new();
        core.publish_started("m1", "sitzung");
        let (session, ticket) = core.begin_stop().unwrap();
        assert_eq!(session, "sitzung");
        // Das Mikrofon ist noch offen: Diktat-Hotkey und neuer Start sehen es.
        assert!(
            core.is_recording(),
            "Fenster 'Zustand Idle vor Capture-Stopp' ist zu"
        );
        assert_eq!(
            core.begin_start().err(),
            Some("already_recording".to_string())
        );
        // Ein zweiter Stopp hat nichts mehr zu stoppen.
        assert_eq!(core.begin_stop().err(), Some("not_recording".to_string()));
        ticket.finish();
        assert!(!core.is_recording());
        assert!(core.begin_start().is_ok());
    }

    #[test]
    fn a_panic_while_stopping_still_frees_the_state() {
        let core = Core::new();
        core.publish_started("m1", "sitzung");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (_session, _ticket) = core.begin_stop().unwrap();
            panic!("Capture-Stopp stuerzt ab");
        }));
        assert!(result.is_err());
        assert!(
            !core.is_recording(),
            "keine Geisteraufnahme, die Diktat und Neustart sperrt"
        );
    }

    #[test]
    fn a_start_in_progress_counts_as_recording_and_refuses_a_second_start() {
        let core = Core::new();
        let ticket = core.begin_start().unwrap();
        assert!(
            core.is_recording(),
            "waehrend des Starts gilt es schon als Aufnahme"
        );
        assert_eq!(
            core.begin_start().err(),
            Some("already_recording".to_string())
        );
        drop(ticket);
        assert!(
            !core.is_recording(),
            "auch ein gescheiterter Start gibt es frei"
        );
    }

    #[test]
    fn pause_needs_a_running_session_not_just_a_state() {
        let core = Core::new();
        // Weder vor dem Start noch im Startfenster (Capture laeuft, Sitzung fehlt noch).
        assert_eq!(
            core.set_paused(true, |_| {}),
            Err("not_recording".to_string())
        );
        let _starting = core.begin_start().unwrap();
        assert_eq!(
            core.set_paused(true, |_| {}),
            Err("not_recording".to_string())
        );
        core.publish_started("m1", "sitzung");
        let applied = std::cell::Cell::new(false);
        assert_eq!(
            core.set_paused(true, |s| {
                assert_eq!(*s, "sitzung");
                applied.set(true);
            }),
            Ok("m1".to_string())
        );
        assert!(applied.get());
        assert!(matches!(
            &*lock_recovering(&core.state),
            MeetingRunState::Recording { paused: true, .. }
        ));
        // Nach dem Stopp-Beginn gibt es nichts mehr zu pausieren (kein falsches Ereignis).
        let (_s, ticket) = core.begin_stop().unwrap();
        assert_eq!(
            core.set_paused(false, |_| {}),
            Err("not_recording".to_string())
        );
        ticket.finish();
    }

    /// Start und Stopp im Wettlauf: nie darf `Recording` ohne Sitzung zurueckbleiben
    /// (die Reihenfolge "Sitzung setzen, dann Zustand" liess einen Stopp dazwischen
    /// den Zustand auf `Idle` setzen, bevor der Start ihn auf `Recording` stellte).
    #[test]
    fn start_and_stop_racing_never_leave_a_ghost_recording() {
        for round in 0..200 {
            let core = Arc::new(Core::new());
            let starter = Arc::clone(&core);
            let stopper = Arc::clone(&core);
            let start = std::thread::spawn(move || {
                starter.publish_started("m1", "sitzung");
            });
            let stop = std::thread::spawn(move || {
                // Mehrere Versuche, damit mindestens einer nach dem Start liegt.
                for _ in 0..50 {
                    if let Ok((_s, ticket)) = stopper.begin_stop() {
                        ticket.finish();
                        return true;
                    }
                    std::thread::yield_now();
                }
                false
            });
            start.join().unwrap();
            let stopped = stop.join().unwrap();
            if !stopped {
                // Der Stopp kam nie dazwischen: aufraeumen wie der echte Stopp.
                let (_s, ticket) = core.begin_stop().unwrap();
                ticket.finish();
            }
            assert!(
                !core.is_recording(),
                "Runde {round}: Zustand Recording ohne Sitzung (Geisteraufnahme)"
            );
        }
    }

    // ---- #15: gescheiterter Start hinterlaesst keine Debris ---------------------------

    fn failed_start_fixture() -> (tempfile::TempDir, MeetingStore, String, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        let meeting = store
            .create_meeting("Montag", MeetingSource::Live, Some(1))
            .unwrap();
        let folder = dir.path().join("meetings").join(&meeting.id);
        std::fs::create_dir_all(&folder).unwrap();
        // Wie `start_row`: beide WAVs mit Kopf, Pfade vorab in die DB.
        let mic = folder.join("mic.wav");
        let system = folder.join("system.wav");
        drop(StreamingWavWriter::create(&mic, SAMPLE_RATE).unwrap());
        drop(StreamingWavWriter::create(&system, SAMPLE_RATE).unwrap());
        store
            .set_audio_paths(&meeting.id, mic.to_str(), system.to_str(), None)
            .unwrap();
        (dir, store, meeting.id, folder)
    }

    #[test]
    fn a_failed_start_removes_the_zero_header_wavs_clears_the_paths_and_marks_the_row_failed() {
        let (_dir, store, id, folder) = failed_start_fixture();
        assert!(
            folder.join("mic.wav").exists(),
            "Voraussetzung: Debris liegt da"
        );

        discard_failed_start(&store, &id, &folder);

        assert!(!folder.exists(), "Ordner weg, wenn er danach leer ist");
        let meeting = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(meeting.status, "failed");
        assert_eq!(
            (meeting.mic_audio_path, meeting.system_audio_path),
            (None, None)
        );
        assert!(
            !is_orphan_status(&meeting.status),
            "beim naechsten App-Start darf daraus keine 'wiederhergestellte' Aufnahme werden"
        );
    }

    #[test]
    fn a_failed_start_keeps_files_it_did_not_create() {
        let (_dir, store, id, folder) = failed_start_fixture();
        std::fs::write(folder.join("notizen.txt"), b"gehoert dem Nutzer").unwrap();
        // mic_aec.wav entsteht in der Echo-Stufe der Pipeline und gehoert dazu.
        std::fs::write(
            folder.join(crate::managers::meetings::MIC_AEC_FILE),
            b"RIFF",
        )
        .unwrap();

        discard_failed_start(&store, &id, &folder);

        assert!(folder.join("notizen.txt").exists(), "Fremdes bleibt");
        assert!(!folder.join("mic.wav").exists());
        assert!(!folder.join("system.wav").exists());
        assert!(!folder
            .join(crate::managers::meetings::MIC_AEC_FILE)
            .exists());
        assert!(
            folder.exists(),
            "der Ordner bleibt, solange etwas Fremdes darin liegt"
        );
    }

    #[test]
    fn discarding_is_safe_when_nothing_was_created() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        let meeting = store
            .create_meeting("Leer", MeetingSource::Live, Some(1))
            .unwrap();
        // Der Ordner wurde nie angelegt (z. B. `meeting_dir_failed`).
        discard_failed_start(&store, &meeting.id, &dir.path().join("gibt-es-nicht"));
        assert_eq!(
            store.get_meeting(&meeting.id).unwrap().unwrap().status,
            "failed"
        );
        // Und eine unbekannte Besprechung bringt nichts zum Absturz.
        discard_failed_start(&store, "gibt-es-nicht", &dir.path().join("x"));
    }

    // -- lva#534 Fixrunde 3: Reihenfolge von Abbruch, Tor und Beginn beim Start einer Aufnahme --

    mod start {
        use super::*;
        use crate::managers::planning::testhilfe as th;
        use std::sync::mpsc;

        fn store() -> (tempfile::TempDir, Arc<MeetingStore>) {
            let dir = tempfile::tempdir().unwrap();
            let store = Arc::new(MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap());
            (dir, store)
        }

        /// Ein "Enddurchlauf", der erst weitermacht, wenn der Start ihn abbricht (der Abbruch-Merker
        /// ist gesetzt, der Start wartet also auf ihn), und DANN das Tor betritt (wie `run_job`).
        /// Wartete der Start unter der Tor-Sperre, koennte er nie enden.
        fn haengender_enddurchlauf(
            slot: &FinalSlot,
            store: &Arc<MeetingStore>,
            meeting_id: &str,
            log: &Arc<Mutex<Vec<&'static str>>>,
        ) {
            let (store, meeting_id, log) =
                (Arc::clone(store), meeting_id.to_string(), Arc::clone(log));
            slot.spawn(move |_previous, cancel| {
                while !cancel.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                let _ = crate::managers::planning::tor_pruefen_besprechung(&store, &meeting_id);
                log.lock().unwrap().push("enddurchlauf_fertig");
            })
            .unwrap();
        }

        /// Fuehrt einen Start in einem Thread aus und wartet (nur als Netz gegen ein Haengen) auf
        /// sein Ende. Gibt das Ergebnis des Starts und das Protokoll zurueck.
        fn starten(
            slot: Arc<FinalSlot>,
            store: Arc<MeetingStore>,
            ziel: Option<String>,
            log: Arc<Mutex<Vec<&'static str>>>,
        ) -> Result<(), String> {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let (l1, l2) = (Arc::clone(&log), Arc::clone(&log));
                let r = slot.start_unter_tor(
                    &store,
                    None,
                    ziel.as_deref(),
                    move || l1.lock().unwrap().push("abbruch_abgewartet"),
                    move |_tor| {
                        l2.lock().unwrap().push("start");
                        Ok(())
                    },
                );
                let _ = tx.send(r.map(|((), _live)| ()));
            });
            rx.recv_timeout(Duration::from_secs(60))
                .expect("Deadlock: der Start endet nicht (Abbruch unter der Tor-Sperre?)")
        }

        /// Blocker 1, ohne Planung: Start einer Aufnahme, waehrend ein Enddurchlauf noch vor dem
        /// Tor steht. Beide enden; erst der Enddurchlauf, dann der Start.
        #[test]
        fn a_live_start_does_not_deadlock_with_a_final_pass_before_the_gate() {
            let (_dir, store) = store();
            let slot = Arc::new(FinalSlot::default());
            let log = Arc::new(Mutex::new(Vec::new()));
            let m = store.create_empty_meeting("A", None).unwrap();
            haengender_enddurchlauf(&slot, &store, &m.id, &log);
            let r = starten(
                Arc::clone(&slot),
                Arc::clone(&store),
                None,
                Arc::clone(&log),
            );
            assert_eq!(r, Ok(()));
            assert_eq!(
                *log.lock().unwrap(),
                ["enddurchlauf_fertig", "abbruch_abgewartet", "start"]
            );
            assert!(!slot.live_aktiv(), "die Anmeldung fiel mit dem Halter");
        }

        /// Blocker 1 mit Planung: Zustimmung liegt vor -> der Start laeuft; Ablehnung -> der Start
        /// laeuft nie. In beiden Faellen endet auch der Enddurchlauf (kein Deadlock).
        #[test]
        fn the_same_holds_with_a_plan_that_allows_or_refuses() {
            for zugestimmt in [true, false] {
                let (_dir, store) = store();
                let slot = Arc::new(FinalSlot::default());
                let log = Arc::new(Mutex::new(Vec::new()));
                let m = store.create_empty_meeting("B", None).unwrap();
                th::plan_fuer_besprechung(&store, &m.id, "u-r", zugestimmt);
                haengender_enddurchlauf(&slot, &store, &m.id, &log);
                let r = starten(
                    Arc::clone(&slot),
                    Arc::clone(&store),
                    Some(m.id.clone()),
                    Arc::clone(&log),
                );
                let log = log.lock().unwrap().clone();
                if zugestimmt {
                    assert_eq!(r, Ok(()));
                    assert_eq!(log, ["enddurchlauf_fertig", "abbruch_abgewartet", "start"]);
                } else {
                    assert_eq!(r, Err("nur_notizen: offen".to_string()));
                    assert_eq!(log, ["enddurchlauf_fertig", "abbruch_abgewartet"]);
                }
                assert!(
                    !slot.live_aktiv(),
                    "auch ein abgelehnter Start meldet sich ab"
                );
            }
        }

        fn paniktext(f: impl FnOnce()) -> String {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            let text = r.expect_err("muss gemeldet werden");
            text.downcast_ref::<String>()
                .cloned()
                .or_else(|| text.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default()
        }

        /// Sperrordnung, in Debug-Laeufen geprueft: unter der Tor-Sperre wird weder auf den
        /// Enddurchlauf gewartet noch der Zustand der Aufnahme (`RunCore`) angefasst noch ein
        /// Audiotreiber gestartet. Jeder Verstoss meldet sich sofort, statt zu haengen.
        #[test]
        fn the_lock_order_check_covers_the_slot_the_run_state_and_the_microphone() {
            let (_dir, store) = store();
            let slot = FinalSlot::default();
            let unter_tor = |was: &dyn Fn()| {
                paniktext(|| {
                    let _ = crate::managers::planning::mit_tor(&store, None, None, |_| {
                        was();
                        Ok(())
                    });
                })
            };
            let text = unter_tor(&|| {
                let _ = slot.live_beginnen();
            });
            assert!(
                text.contains("Abwarten des Enddurchlaufs unter der Tor-Sperre"),
                "{text}"
            );
            assert!(!slot.live_aktiv(), "vor der Anmeldung gemeldet");
            let text = unter_tor(&|| slot.wait());
            assert!(
                text.contains("join des Enddurchlaufs unter der Tor-Sperre"),
                "{text}"
            );

            let core: RunCore<u8> = RunCore::new();
            let text = unter_tor(&|| core.publish_started("m", 1));
            assert!(text.contains("publish_started"), "{text}");
            assert!(core.is_idle(), "nichts veroeffentlicht");
            let text = unter_tor(&|| {
                let _ = core.begin_stop();
            });
            assert!(text.contains("begin_stop"), "{text}");
            let text = unter_tor(&|| {
                let _ = core.set_paused(true, |_| {});
            });
            assert!(text.contains("set_paused"), "{text}");

            let text = unter_tor(&|| {
                let _ = MeetingMicCapture::start(None, Arc::new(|_| {}), |_, _| {});
            });
            assert!(
                text.contains("Start des Mikrofons unter der Tor-Sperre"),
                "{text}"
            );
            assert!(!crate::managers::planning::unter_tor_sperre());
        }

        /// R4-1 (Platz): ab der Anmeldung einer Aufnahme wird jeder Auftrag mit gesetztem
        /// Abbruch-Merker geboren, auch einer, der IM Start-Rueckruf unter dem Tor entsteht (also
        /// nach jeder Pruefung des Starts). Nach dem Ende der Aufnahme nicht mehr.
        #[test]
        fn a_job_spawned_while_a_recording_is_registered_is_born_cancelled() {
            let (_dir, store) = store();
            let slot = FinalSlot::default();
            let (tx, rx) = mpsc::channel::<bool>();
            let auftrag = |tx: mpsc::Sender<bool>| {
                move |vorher: Option<std::thread::JoinHandle<()>>, cancel: Arc<AtomicBool>| {
                    if let Some(vorher) = vorher {
                        let _ = vorher.join();
                    }
                    let _ = tx.send(cancel.load(Ordering::Acquire));
                }
            };
            // Vor jeder Aufnahme: ein Auftrag laeuft normal.
            slot.spawn(auftrag(tx.clone())).unwrap();
            assert!(!rx.recv().unwrap());

            let ((), live) = slot
                .start_unter_tor(
                    &store,
                    None,
                    None,
                    || {},
                    |_tor| {
                        slot.spawn(auftrag(tx.clone())).unwrap();
                        Ok(())
                    },
                )
                .unwrap();
            assert!(
                rx.recv().unwrap(),
                "im Start-Rueckruf entstanden: abgebrochen geboren"
            );
            assert!(slot.live_aktiv());
            slot.spawn(auftrag(tx.clone())).unwrap();
            assert!(
                rx.recv().unwrap(),
                "waehrend der Aufnahme entstanden: abgebrochen geboren"
            );
            slot.wait();

            drop(live);
            assert!(!slot.live_aktiv());
            slot.spawn(auftrag(tx)).unwrap();
            assert!(!rx.recv().unwrap(), "nach der Aufnahme: normal");
            slot.wait();
        }

        /// Hinweis "cancel() wartet unbegrenzt": das Abwarten hat eine Frist. Endet der
        /// abgebrochene Auftrag nicht, scheitert der START (`job_busy`), die Anmeldung faellt, und
        /// der Auftrag bleibt abgebrochen. Ein spaeterer Auftrag erbt den gesetzten Merker nicht.
        /// Ohne Zeitmessung: die Frist ist null, der Auftrag haengt an einem Kanal.
        #[test]
        fn waiting_for_a_cancelled_final_pass_is_bounded() {
            let slot = FinalSlot::mit_frist(Duration::ZERO);
            let (weiter_tx, weiter_rx) = mpsc::channel::<()>();
            let (gesehen_tx, gesehen_rx) = mpsc::channel::<bool>();
            let erster = gesehen_tx.clone();
            slot.spawn(move |_vorher, cancel| {
                // Ein Modellaufruf, der sich nicht unterbrechen laesst.
                let _ = weiter_rx.recv();
                let _ = erster.send(cancel.load(Ordering::Acquire));
            })
            .unwrap();
            let r = slot.live_beginnen();
            assert_eq!(
                r.err(),
                Some("job_busy: final_pass_not_cancelled".to_string())
            );
            assert!(
                !slot.live_aktiv(),
                "kein Start: die Anmeldung ist wieder weg"
            );
            assert!(slot.active(), "der Auftrag laeuft noch");

            // Ein neuer Auftrag (keine Aufnahme angemeldet) wartet auf den alten und laeuft dann
            // normal: er teilt den gesetzten Merker NICHT.
            slot.spawn(move |vorher, cancel| {
                if let Some(vorher) = vorher {
                    let _ = vorher.join();
                }
                let _ = gesehen_tx.send(cancel.load(Ordering::Acquire));
            })
            .unwrap();
            weiter_tx.send(()).unwrap();
            assert!(
                gesehen_rx.recv().unwrap(),
                "der alte Auftrag sah den Abbruch"
            );
            assert!(!gesehen_rx.recv().unwrap(), "der neue nicht");
            slot.wait();

            // Mit einem Auftrag, der auf den Abbruch hoert, gelingt der Start.
            let slot = FinalSlot::default();
            slot.spawn(|_vorher, cancel| {
                while !cancel.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            })
            .unwrap();
            let live = slot.live_beginnen().unwrap();
            assert!(!slot.active() && slot.live_aktiv());
            drop(live);
        }

        /// Bis zum wirksamen Beginn verwerfen die Capture-Callbacks alles.
        #[test]
        fn capture_callbacks_drop_everything_until_the_start_is_effective() {
            let wirksam = Arc::new(AtomicBool::new(false));
            let gesehen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let zaehler = Arc::clone(&gesehen);
            let mut callback = erst_wenn_wirksam(Arc::clone(&wirksam), move |samples, _qpc| {
                zaehler.fetch_add(samples.len(), Ordering::SeqCst);
            });
            callback(&[1, 2, 3], None);
            assert_eq!(gesehen.load(Ordering::SeqCst), 0);
            wirksam.store(true, Ordering::Release);
            callback(&[1, 2], Some(7));
            assert_eq!(gesehen.load(Ordering::SeqCst), 2);
        }

        fn ein_segment() -> Vec<crate::managers::transcription::TimedSegment> {
            vec![crate::managers::transcription::TimedSegment {
                text: "hallo".into(),
                start_ms: 0,
                end_ms: 100,
                words: None,
            }]
        }

        /// Der echte Block des Live-Workers ohne Modell: `live_block_gated` um
        /// `import::block_mit_wiederholung` (drei Versuche, jeder hinter dem Tor), im Abschnitt
        /// `Modellarbeit` wie `dsp::run_worker`. Gibt die Segmente zurueck und zaehlt Aufrufe.
        fn live_block(
            store: &MeetingStore,
            meeting_id: &str,
            told: &AtomicBool,
            aufrufe: &std::sync::atomic::AtomicUsize,
            meldungen: &std::sync::atomic::AtomicUsize,
        ) -> Vec<crate::managers::transcription::TimedSegment> {
            let _arbeit = crate::managers::planning::Modellarbeit::beginnen(meeting_id);
            let chunk = Chunk {
                samples: vec![0.1; 16_000],
                offset_ms: 0,
            };
            live_block_gated(
                meeting_id,
                told,
                || {
                    super::super::super::import::block_mit_wiederholung(
                        store,
                        meeting_id,
                        &chunk,
                        &mut |frei, _chunk| {
                            frei.einloesen()?;
                            aufrufe.fetch_add(1, Ordering::SeqCst);
                            Ok(ein_segment())
                        },
                        &mut || {},
                    )
                },
                || {
                    meldungen.fetch_add(1, Ordering::SeqCst);
                },
            )
        }

        /// R4-6 (a): der Start hat geprueft und die Zeile angelegt, `finish_start` (Verknuepfung
        /// mit dem Termin) lief noch NICHT. Der Widerruf faellt genau hierhin. Der Live-Block ruft
        /// kein Modell, der wirksame Beginn wird verweigert, und der Speicher nimmt kein
        /// Modell-Ergebnis an: die Planung haengt seit dem Start als Bindung an der Besprechung.
        #[test]
        fn a_revocation_before_finish_start_stops_the_live_block() {
            let (_dir, store) = store();
            let (p, termin) = th::plan_mit_termin(&store, "u-6a", true);
            let slot = FinalSlot::default();
            let ((meeting, gebunden), _live) = slot
                .start_unter_tor(
                    &store,
                    Some(&termin),
                    None,
                    || {},
                    |tor| {
                        let meeting = begin_live_row(&store, "Abstimmung", 1, None)?;
                        let gebunden = tor.binden(&store, &meeting.id)?;
                        Ok((meeting, gebunden))
                    },
                )
                .unwrap();
            assert!(
                store.meeting_calendar_link(&meeting.id).unwrap().is_none(),
                "Voraussetzung: noch keine Verknuepfung mit dem Termin"
            );
            let (told, aufrufe, meldungen) = (
                AtomicBool::new(false),
                std::sync::atomic::AtomicUsize::new(0),
                std::sync::atomic::AtomicUsize::new(0),
            );
            // Gegenprobe: vor dem Widerruf laeuft der Block.
            assert_eq!(
                live_block(&store, &meeting.id, &told, &aufrufe, &meldungen).len(),
                1
            );
            assert_eq!(aufrufe.load(Ordering::SeqCst), 1);

            crate::managers::planning::einwilligung_setzen(
                &store,
                &th::widerruf(&p.id, th::BERT),
                th::NOW + 3,
            )
            .unwrap();
            assert!(live_block(&store, &meeting.id, &told, &aufrufe, &meldungen).is_empty());
            assert!(live_block(&store, &meeting.id, &told, &aufrufe, &meldungen).is_empty());
            assert_eq!(
                aufrufe.load(Ordering::SeqCst),
                1,
                "kein weiterer Modellaufruf"
            );
            assert_eq!(meldungen.load(Ordering::SeqCst), 1, "einmal gemeldet");
            assert_eq!(
                gebunden.wirksam(&store, || panic!("darf nie schalten")),
                Err("nur_notizen: abgelehnt".to_string())
            );
            // Schreibebene: ein Ergebnis, das der Worker schon in der Hand haelt, wird verworfen.
            let _arbeit = crate::managers::planning::Modellarbeit::beginnen(&meeting.id);
            let delta = super::super::super::store::TranscriptDelta {
                new_segments: vec![StoredSegment {
                    segment_index: 0,
                    text: "hallo".into(),
                    start_ms: 0,
                    end_ms: 100,
                    channel: 0,
                    speaker_index: None,
                    words: None,
                }],
            };
            assert!(store.append_delta(&meeting.id, &delta).is_err());
            assert!(store.get_segments(&meeting.id).unwrap().is_empty());
        }

        /// R4-6 (b): ZWEI zunaechst erlaubte Quellen (uebergebener Termin mit Planung A, der
        /// Zieleintrag ist mit dem Termin von Planung B verknuepft). Nach dem Start ersetzt
        /// `finish_start` die Verknuepfung des Ziels durch den uebergebenen Termin (`INSERT OR
        /// REPLACE`). Der Widerruf der URSPRUENGLICHEN Zielplanung B verhindert trotzdem jeden
        /// weiteren Modellaufruf, der von A ebenso.
        #[test]
        fn both_sources_of_a_start_keep_blocking_after_finish_start_replaced_the_link() {
            for widerrufen in ["ziel", "termin"] {
                let (_dir, store) = store();
                let ziel = store.create_empty_meeting("Ziel", None).unwrap();
                let (b, termin_b) = th::plan_mit_termin(&store, "u-6b-ziel", true);
                store
                    .link_meeting_event(&ziel.id, &termin_b, "prompt", th::NOW)
                    .unwrap();
                let (a, termin_a) = th::plan_mit_termin(&store, "u-6b-termin", true);
                let slot = FinalSlot::default();
                let ((meeting, _gebunden), _live) = slot
                    .start_unter_tor(
                        &store,
                        Some(&termin_a),
                        Some(&ziel.id),
                        || {},
                        |tor| {
                            let meeting = begin_live_row(&store, "Ziel", 1, Some(&ziel.id))?;
                            let gebunden = tor.binden(&store, &meeting.id)?;
                            Ok((meeting, gebunden))
                        },
                    )
                    .unwrap();
                assert_eq!(meeting.id, ziel.id);

                let plan = crate::managers::calendar::service::StartPlan {
                    title: "Ziel".into(),
                    event: Some(termin_a.clone()),
                    template_id: None,
                };
                let probleme = crate::managers::calendar::service::finish_start(
                    &store,
                    &meeting.id,
                    &plan,
                    "prompt",
                    th::NOW + 1,
                );
                assert!(probleme.is_empty(), "{probleme:?}");
                assert_eq!(
                    store
                        .meeting_calendar_link(&meeting.id)
                        .unwrap()
                        .unwrap()
                        .uid,
                    termin_a.uid,
                    "die Verknuepfung des Ziels zeigt jetzt auf den uebergebenen Termin"
                );

                let (told, aufrufe, meldungen) = (
                    AtomicBool::new(false),
                    std::sync::atomic::AtomicUsize::new(0),
                    std::sync::atomic::AtomicUsize::new(0),
                );
                assert_eq!(
                    live_block(&store, &meeting.id, &told, &aufrufe, &meldungen).len(),
                    1,
                    "beide zugestimmt: der Block laeuft"
                );
                let plan_id = if widerrufen == "ziel" { &b.id } else { &a.id };
                crate::managers::planning::einwilligung_setzen(
                    &store,
                    &th::widerruf(plan_id, th::BERT),
                    th::NOW + 3,
                )
                .unwrap();
                assert!(
                    live_block(&store, &meeting.id, &told, &aufrufe, &meldungen).is_empty(),
                    "Widerruf der Planung '{widerrufen}'"
                );
                assert_eq!(aufrufe.load(Ordering::SeqCst), 1, "{widerrufen}");
                assert_eq!(meldungen.load(Ordering::SeqCst), 1);
            }
        }

        /// Der Block der Live-Transkription: nach einer Ablehnung kein Modellaufruf, keine
        /// Luecken-Platzhalter, die Meldung nur einmal; ein Widerruf WAEHREND des Blocks verwirft
        /// ihn, und der Block versucht es danach kein zweites Mal (drei Versuche sind erlaubt).
        #[test]
        fn the_live_block_asks_the_gate_around_the_model_and_reports_a_refusal_once() {
            let (_dir, store) = store();
            let m = store.create_empty_meeting("L", None).unwrap();
            let (p, _) = th::plan_fuer_besprechung(&store, &m.id, "u-l", true);
            let told = AtomicBool::new(false);
            let aufrufe = std::sync::atomic::AtomicUsize::new(0);
            let meldungen = std::sync::atomic::AtomicUsize::new(0);
            let _arbeit = crate::managers::planning::Modellarbeit::beginnen(&m.id);
            let chunk = Chunk {
                samples: vec![0.1; 16_000],
                offset_ms: 0,
            };
            let block = |widerruf_im_aufruf: bool| {
                live_block_gated(
                    &m.id,
                    &told,
                    || {
                        super::super::super::import::block_mit_wiederholung(
                            &store,
                            &m.id,
                            &chunk,
                            &mut |frei, _chunk| {
                                frei.einloesen()?;
                                aufrufe.fetch_add(1, Ordering::SeqCst);
                                if widerruf_im_aufruf {
                                    crate::managers::planning::einwilligung_setzen(
                                        &store,
                                        &th::widerruf(&p.id, th::BERT),
                                        th::NOW + 3,
                                    )
                                    .unwrap();
                                    // Der widerrufene Versuch scheitert auch noch: trotzdem
                                    // kein zweiter Versuch und kein Platzhalter.
                                    return Err("kein Modell geladen".into());
                                }
                                Ok(ein_segment())
                            },
                            &mut || {},
                        )
                    },
                    || {
                        meldungen.fetch_add(1, Ordering::SeqCst);
                    },
                )
            };
            assert_eq!(block(false).len(), 1, "erlaubt: Segmente");
            assert!(block(true).is_empty(), "Widerruf im Aufruf: verworfen");
            assert_eq!(aufrufe.load(Ordering::SeqCst), 2, "kein zweiter Versuch");
            assert!(block(false).is_empty(), "danach kein Modellaufruf mehr");
            assert!(block(false).is_empty());
            assert_eq!(aufrufe.load(Ordering::SeqCst), 2, "kein weiterer Aufruf");
            assert_eq!(meldungen.load(Ordering::SeqCst), 1, "einmal gemeldet");
        }
    }
}
