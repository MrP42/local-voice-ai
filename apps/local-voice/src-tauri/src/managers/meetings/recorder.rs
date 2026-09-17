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
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use super::chunker::{ChannelChunker, Chunk};
use super::mic_capture::MeetingMicCapture;
use super::store::{
    Meeting, MeetingSource, MeetingStatus, MeetingStore, StoredSegment, TranscriptDelta,
};
use crate::audio_toolkit::audio::wav_writer::repair_orphan_wav;
use crate::audio_toolkit::audio::{LoopbackCapture, StreamingWavWriter};
use crate::managers::transcription::TranscriptionManager;

/// Sample rate of the whole meetings pipeline (mic capture and loopback both
/// deliver 16 kHz mono i16).
const SAMPLE_RATE: u32 = 16_000;
/// Block length handed to the transcription engine.
const CHUNK_TARGET_MS: u64 = 20_000;
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
}

// ---------------------------------------------------------------------------
// Session internals
// ---------------------------------------------------------------------------

/// What the capture callbacks hand to the transcription worker. `Shutdown`
/// exists because "all senders dropped" is not a reliable end signal here: a
/// loopback start that timed out may still be holding a sender clone in a
/// thread we have given up on, and `stop()` must not block on it.
enum WorkItem {
    Chunk(u8, Chunk),
    Shutdown,
}

/// One channel's write path: WAV file plus the chunker that feeds the worker.
struct ChannelSink {
    writer: Option<StreamingWavWriter>,
    chunker: ChannelChunker,
    samples_since_flush: usize,
}

impl ChannelSink {
    fn new(writer: StreamingWavWriter) -> Self {
        Self {
            writer: Some(writer),
            chunker: ChannelChunker::new(CHUNK_TARGET_MS),
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
    work_tx: Option<mpsc::Sender<WorkItem>>,
    worker: Option<JoinHandle<()>>,
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

pub struct MeetingRecorderManager {
    app: AppHandle,
    store: Arc<MeetingStore>,
    transcription: Arc<TranscriptionManager>,
    state: Mutex<MeetingRunState>,
    session: Mutex<Option<RecordingSession>>,
    /// Held for the whole of `start()`. The run state only flips to
    /// `Recording` once the captures are up, so without this a double-click
    /// could get two starts past `may_start` and leave one orphaned.
    start_guard: Mutex<()>,
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
            state: Mutex::new(MeetingRunState::Idle),
            session: Mutex::new(None),
            start_guard: Mutex::new(()),
        }
    }

    pub fn is_recording(&self) -> bool {
        !may_start(&self.state.lock().unwrap())
    }

    /// Starts a live meeting: creates the row, the folder and both WAV
    /// writers, wires the capture callbacks into the chunk pipeline and turns
    /// the recording indicator on.
    pub fn start(
        &self,
        title: String,
        consent_confirmed: bool,
        capture_system: bool,
    ) -> Result<Meeting, String> {
        consent_gate(consent_confirmed)?;
        let _start_guard = self.start_guard.lock().map_err(|_| "recorder_poisoned")?;

        {
            let state = self.state.lock().unwrap();
            if !may_start(&state) {
                return Err("already_recording".to_string());
            }
        }

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

        let consent_at = chrono::Utc::now().timestamp();
        let meeting = self
            .store
            .create_meeting(&title, MeetingSource::Live, Some(consent_at))
            .map_err(|e| format!("meeting_create_failed: {e}"))?;
        let meeting_id = meeting.id.clone();

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
        let levels = Arc::new(LevelEmitter::new(self.app.clone()));

        let (work_tx, work_rx) = mpsc::channel::<WorkItem>();
        let worker = self.spawn_worker(meeting_id.clone(), work_rx);

        // Load the meeting model (dedicated `meeting_model` or the dictation
        // model as fallback); transcribe_segments waits on the load condvar.
        let target = crate::managers::transcription::TranscriptionManager::meeting_model_target(
            &crate::settings::get_settings(&self.app),
        );
        self.transcription.initiate_model_load_target(&target);

        let mic_capture = MeetingMicCapture::start(
            crate::settings::get_settings(&self.app).selected_microphone,
            channel_callback(
                CHANNEL_MIC,
                Arc::clone(&mic_sink),
                Arc::clone(&paused),
                work_tx.clone(),
                Arc::clone(&levels),
            ),
        )
        .map_err(|e| {
            // The meeting row exists but nothing was captured — mark it failed
            // rather than leaving a phantom "recording" row behind.
            let _ = self.store.set_status(&meeting_id, MeetingStatus::Failed);
            format!("mic_start_failed: {e}")
        })?;

        let loopback = match (&system_sink, capture_system) {
            (Some(sink), true) => self.start_loopback(
                &meeting_id,
                channel_callback(
                    CHANNEL_SYSTEM,
                    Arc::clone(sink),
                    Arc::clone(&paused),
                    work_tx.clone(),
                    Arc::clone(&levels),
                ),
            ),
            _ => None,
        };

        *self.session.lock().unwrap() = Some(RecordingSession {
            meeting_id: meeting_id.clone(),
            paused,
            mic_capture: Some(mic_capture),
            loopback,
            mic_sink,
            system_sink,
            mic_path,
            system_path,
            work_tx: Some(work_tx),
            worker: Some(worker),
        });
        *self.state.lock().unwrap() = MeetingRunState::Recording {
            meeting_id: meeting_id.clone(),
            paused: false,
        };

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
        callback: impl FnMut(&[i16]) + Send + 'static,
    ) -> Option<LoopbackCapture> {
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
                self.watch_loopback(meeting_id, &capture);
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
    fn watch_loopback(&self, meeting_id: &str, capture: &LoopbackCapture) {
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

    /// One background thread per meeting: chunk in, segments out. A failed
    /// chunk is logged by length only (never content) and does not end the
    /// meeting.
    fn spawn_worker(
        &self,
        meeting_id: String,
        work_rx: mpsc::Receiver<WorkItem>,
    ) -> JoinHandle<()> {
        let app = self.app.clone();
        let store = Arc::clone(&self.store);
        let transcription = Arc::clone(&self.transcription);
        std::thread::Builder::new()
            .name("meeting-transcribe".to_string())
            .spawn(move || {
                let mut next_index: u32 = 0;
                while let Ok(item) = work_rx.recv() {
                    let (channel, chunk) = match item {
                        WorkItem::Chunk(channel, chunk) => (channel, chunk),
                        WorkItem::Shutdown => break,
                    };
                    // Nie einen Block verlieren: Wiederholung, sonst Luecke
                    // mit Zeitraum (siehe import.rs::transcribe_chunk_resilient).
                    let timed =
                        super::import::transcribe_chunk_resilient(&app, &transcription, &chunk);
                    {
                        {
                            let appended: Vec<StoredSegment> = timed
                                .into_iter()
                                .filter(|s| !s.text.trim().is_empty())
                                .map(|s| {
                                    let segment = StoredSegment {
                                        segment_index: next_index,
                                        text: s.text,
                                        start_ms: chunk.offset_ms + s.start_ms,
                                        end_ms: chunk.offset_ms + s.end_ms,
                                        channel,
                                        speaker_index: None,
                                    };
                                    next_index += 1;
                                    segment
                                })
                                .collect();
                            if appended.is_empty() {
                                continue;
                            }
                            let delta = TranscriptDelta {
                                new_segments: appended.clone(),
                            };
                            if let Err(e) = store.append_delta(&meeting_id, &delta) {
                                error!("meetings: delta not stored: {e}");
                                let _ = (MeetingEvent::Error {
                                    meeting_id: meeting_id.clone(),
                                    message: "delta_store_failed".to_string(),
                                })
                                .emit(&app);
                                continue;
                            }
                            let _ = (MeetingEvent::Segments {
                                meeting_id: meeting_id.clone(),
                                appended,
                            })
                            .emit(&app);
                        }
                    }
                }
                debug!("meetings: worker for {meeting_id} finished");
            })
            .expect("failed to spawn meeting transcription worker")
    }

    pub fn pause(&self) -> Result<(), String> {
        self.set_paused(true)
    }

    pub fn resume(&self) -> Result<(), String> {
        self.set_paused(false)
    }

    fn set_paused(&self, paused: bool) -> Result<(), String> {
        let meeting_id = {
            let mut state = self.state.lock().unwrap();
            apply_pause(&mut state, paused)?;
            match &*state {
                MeetingRunState::Recording { meeting_id, .. } => meeting_id.clone(),
                MeetingRunState::Idle => unreachable!("apply_pause rejects Idle"),
            }
        };
        if let Some(session) = self.session.lock().unwrap().as_ref() {
            // Paused means "discard samples"; wall-clock keeps running, so the
            // WAV timeline compresses the pause instead of padding it. That is
            // deliberate and consistent: transcript offsets come from the
            // chunkers, which skip the same samples.
            session.paused.store(paused, Ordering::Relaxed);
        }
        self.emit_state(&meeting_id, "recording", paused);
        Ok(())
    }

    /// Stops capture, drains the tail chunks (status `processing` while that
    /// runs), finalizes both WAVs and marks the meeting `ready`. Blocking —
    /// call it off the UI thread.
    pub fn stop(&self) -> Result<String, String> {
        let session = self
            .session
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| "not_recording".to_string())?;
        *self.state.lock().unwrap() = MeetingRunState::Idle;
        let RecordingSession {
            meeting_id,
            mic_capture,
            loopback,
            mic_sink,
            system_sink,
            mic_path,
            system_path,
            work_tx,
            worker,
            ..
        } = session;

        self.set_indicator(false);

        // Captures first: once they are stopped no callback can touch the
        // sinks any more, so flushing and finalizing below is race-free.
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

        if let Err(e) = self
            .store
            .set_status(&meeting_id, MeetingStatus::Processing)
        {
            warn!("meetings: status 'processing' not stored: {e}");
        }
        self.emit_state(&meeting_id, "processing", false);

        if let Some(tx) = &work_tx {
            flush_sink(CHANNEL_MIC, &mic_sink, tx);
            if let Some(sink) = &system_sink {
                flush_sink(CHANNEL_SYSTEM, sink, tx);
            }
            // FIFO: everything queued above is transcribed before the worker
            // sees this and stops.
            let _ = tx.send(WorkItem::Shutdown);
        }
        drop(work_tx);
        if let Some(handle) = worker {
            let _ = handle.join();
        }

        // Restore the dictation model so the next hotkey dictation does not
        // silently run on the meeting model (no-op when they are the same).
        let dictation_model = crate::settings::get_settings(&self.app).selected_model;
        self.transcription
            .initiate_model_load_target(&dictation_model);

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
        if let Err(e) = self.store.set_status(&meeting_id, MeetingStatus::Ready) {
            warn!("meetings: status 'ready' not stored: {e}");
        }
        self.emit_state(&meeting_id, "ready", false);
        info!("meetings: recording stopped ({meeting_id}, {duration_ms} ms)");
        Ok(meeting_id)
    }

    /// App start: a meeting still marked `recording` means the app died mid
    /// recording. Its WAV headers claim zero length, so repair them from the
    /// file size and hand the meeting back to the user as `ready` (segments
    /// are already durable — every delta was committed as it arrived).
    pub fn recover_orphans(&self) {
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
                if meeting.status != "recording" {
                    continue;
                }
                for path in [
                    meeting.mic_audio_path.clone(),
                    meeting.system_audio_path.clone(),
                ]
                .into_iter()
                .flatten()
                {
                    match repair_orphan_wav(std::path::Path::new(&path)) {
                        Ok(Some(ms)) => debug!("meetings: repaired orphan wav ({ms} ms)"),
                        Ok(None) => {}
                        Err(e) => warn!("meetings: orphan wav repair failed: {e}"),
                    }
                }
                if let Err(e) = self.store.set_status(&meeting.id, MeetingStatus::Ready) {
                    warn!("meetings: orphan status not stored: {e}");
                    continue;
                }
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
}

/// The per-channel capture callback: WAV append (with 1-s header flush),
/// chunking for the worker, and the throttled level readout.
fn channel_callback(
    channel: u8,
    sink: Arc<Mutex<ChannelSink>>,
    paused: Arc<AtomicBool>,
    work_tx: mpsc::Sender<WorkItem>,
    levels: Arc<LevelEmitter>,
) -> impl FnMut(&[i16]) + Send + 'static {
    move |samples: &[i16]| {
        if paused.load(Ordering::Relaxed) {
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
            if let Some(chunk) = sink.chunker.push(samples) {
                let _ = work_tx.send(WorkItem::Chunk(channel, chunk));
            }
        }
        levels.record(channel, rms(samples));
    }
}

fn flush_sink(channel: u8, sink: &Arc<Mutex<ChannelSink>>, work_tx: &mpsc::Sender<WorkItem>) {
    if let Ok(mut sink) = sink.lock() {
        if let Some(chunk) = sink.chunker.flush() {
            let _ = work_tx.send(WorkItem::Chunk(channel, chunk));
        }
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
}
