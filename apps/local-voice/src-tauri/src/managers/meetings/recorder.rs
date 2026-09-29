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

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use super::chunker::Chunk;
use super::dsp::{
    transcript_final_event, ChannelFeed, DspConfig, DspControl, DspNotice, EchoSetup,
    LivePipeline, MeetingTimeline, PcmSink, VadFactory, WavFileSink,
};
use super::mic_capture::MeetingMicCapture;
use super::store::{Meeting, MeetingSource, MeetingStatus, MeetingStore, StoredSegment};
use crate::audio_toolkit::audio::wav_writer::repair_orphan_wav;
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
    /// The live transcript is complete and stored (sent at the end of `stop()`).
    /// `epoch` is the generation of the segments (`segment_epoch`); `model` the
    /// engine that produced them. Consumers that build on the transcript (AI
    /// notes) start here, not at `stop()`.
    #[serde(rename = "transcript_final")]
    TranscriptFinal {
        meeting_id: String,
        epoch: u32,
        model: Option<String>,
    },
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
                mic_feed,
                Arc::clone(&levels),
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
                channel_callback(
                    CHANNEL_SYSTEM,
                    Arc::clone(sink),
                    Arc::clone(&paused),
                    feed,
                    Arc::clone(&levels),
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

        *self.session.lock().unwrap() = Some(RecordingSession {
            meeting_id: meeting_id.clone(),
            paused,
            mic_capture: Some(mic_capture),
            loopback,
            mic_sink,
            system_sink,
            mic_path,
            system_path,
            pipeline: Some(pipeline),
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
        callback: impl FnMut(&[i16], Option<u64>) + Send + 'static,
        dsp_control: Option<DspControl>,
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

        // Until Health events exist (P2e) the notices are log-only.
        let notice_meeting = meeting_id.to_string();
        let notice: super::dsp::NoticeFn = Arc::new(move |n: DspNotice| {
            // TODO(P2e): map to MeetingEvent::Health { meeting_id, channel, state }.
            debug!("meetings: dsp notice for {notice_meeting}: {n:?}");
        });

        let mut cfg = DspConfig::new(vad_factory, notice);
        if let Some(echo) = echo {
            cfg = cfg.with_echo(echo);
        }

        LivePipeline::start(
            meeting_id.to_string(),
            Arc::clone(&self.store),
            cfg,
            move |chunk: &Chunk| {
                super::import::transcribe_chunk_resilient(&app, &transcription, chunk)
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
            // deliberate and consistent: transcript offsets come from the DSP
            // thread, which counts the same samples (it also closes the open
            // segment when the callback reports the pause).
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
            pipeline,
            ..
        } = session;

        self.set_indicator(false);

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

        // The engine that produced the live transcript, read BEFORE the
        // dictation model is restored below.
        let live_model = self.transcription.get_current_model();

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
        // Every live segment is stored: consumers may build on the transcript now.
        // P2d moves this behind the final pass.
        let _ = transcript_final_event(&self.store, &meeting_id, live_model).emit(&self.app);
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
                // M2-P2c2: auch die entechote Kopie (mic_aec.wav), sonst bliebe
                // ihr Header auf dem Stand der letzten Sekunde vor dem Absturz.
                let derived: Vec<String> = meeting
                    .mic_audio_path
                    .as_deref()
                    .map(super::derived_audio_paths)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| std::path::Path::new(p).exists())
                    .collect();
                for path in [
                    meeting.mic_audio_path.clone(),
                    meeting.system_audio_path.clone(),
                ]
                .into_iter()
                .flatten()
                .chain(derived)
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

    // M1-P1c (Beruehrpunkt B2): Audioposition fuer den Notizblock.
    /// The running meeting's id and the microphone audio position in
    /// milliseconds, or `None` when nothing is being recorded. Same timeline
    /// as `StoredSegment.start_ms` (paused stretches are compressed).
    pub fn position_ms(&self) -> Option<(String, u64)> {
        let session = self.session.lock().ok()?;
        session.as_ref().and_then(Self::session_position)
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
