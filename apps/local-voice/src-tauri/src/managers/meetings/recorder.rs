//! M8 meetings: the recorder manager that orchestrates a live meeting —
//! dual capture (mic + system loopback), streaming WAV writing, the live
//! chunk -> transcription -> delta pipeline, the consent gate, crash
//! recovery and the recording indicator (tray + overlay).
//!
//! Everything that can be decided without I/O lives in free functions (`consent_gate`,
//! `settle_after_stop`, ...) and in `lifecycle.rs` (der Lebenszyklus), so the state
//! rules are testable without Tauri, audio hardware or a model.
//!
//! #537: `start_playback_capture` nimmt NUR den Systemton auf (kein Mikrofon) und
//! schreibt ihn in eine vorhandene YouTube-Besprechung (`playback.rs`): der Nutzer
//! spielt das Video im eingebetteten Player ab, die Aufnahme laeuft wie jede andere
//! (`system.wav`, Live-Transkript, Stopp, Enddurchlauf). Dasselbe Geruest wie die
//! Besprechung, nur mit [`CaptureMode::PlaybackOnly`]: kein `mic.wav`, kein
//! Mikrofon-Capture, kein Echo, `RecordingSession.mic_*` leer.
//!
//! lva#538, die Architektur: EIN Lebenszyklus-Zustand unter EINER Sperre (`lifecycle::RunCore`:
//! Idle -> Starting{op_id, meeting_id, cancel} -> Recording -> Stopping, danach `Finalizing`).
//! Der Start registriert sich mit seiner `op_id` atomar, bevor er etwas anderes tut; Stopp und
//! Loeschen sprechen genau diesen Vorgang an, und ein Stopp, der VOR der Registrierung kommt,
//! wird als Grabstein gemerkt und gilt verbindlich. Abbruch und Teardown sind eine BARRIERE
//! (`RecordingSession::drop`): Sinks schliessen (Callback-Sperre), Captures stoppen und joinen,
//! DSP-Thread und Live-Worker beenden und JOINEN; erst danach folgen Rollback und Dateiloeschung.
//! Danach gibt es keine Nebenwirkung mehr (kein WAV, kein DSP, kein Pegel, kein Segment).
//!
//! Fehlerfaelle und ihre Absicherung (Stand lva#538, Fixrunde 2):
//! - **Nebenlaeufigkeit (Start, Stopp, Abbruch, Loeschen)**: ein Zustand, eine Sperre, kein
//!   Atomic daneben. Start/Stopp-Wettlauf, Abbruch vor/waehrend/nach der Registrierung, Loeschen
//!   waehrend eines Stopps: Tests in `lifecycle.rs` (`a_cancel_racing_the_registration_never_
//!   gets_lost`, `a_delete_waits_for_the_stop_to_finish_closing_the_recording`, ...). Ein Warten,
//!   das ablaeuft, ist ein FEHLER (`start_busy`, `recording_busy`), nie ein stiller Erfolg.
//! - **Abbruch mitten im Start (Frist, Stopp, Loeschen)**: `start_bounded` ruft `abandon`
//!   (= `close_sink`) auf, BEVOR es zurueckkehrt. Der Sink ist unter seiner Sperre geschlossen:
//!   ein laufender Callback ist fertig, ein spaeterer ruft weder WAV noch DSP noch Pegel.
//!   Tests: `a_wedged_start_times_out_abandons_the_sink_...`, `a_running_callback_is_finished_
//!   when_the_barrier_returns`, `a_late_callback_after_the_timeout_writes_nothing`.
//! - **Veroeffentlichung eines abgebrochenen Starts**: die Sitzung wird im Start frueh
//!   angelegt; faellt sie ohne Stopp (Abbruch, Fehler, Panik), laeuft ihr `Drop` durch die
//!   Barriere (`DSP/Worker: LivePipeline::abort`, gejoint). Danach steht kein Segment in der
//!   Datenbank, und es kommt kein Ereignis. Der Rollback der Zeile (`rollback_playback_capture`:
//!   Saetze verwerfen und zuruecksetzen in EINER Transaktion) und das Loeschen der Dateien
//!   kommen erst danach; scheitert der Rollback, bleibt die Datei stehen (Status `failed`).
//!   Tests: `a_dropped_session_leaves_no_segment_event_or_callback_effect`, `dsp.rs`
//!   (`an_abort_returns_only_when_the_worker_is_done_...`), `playback.rs` (`a_rollback_...`).
//! - **Loeschen waehrend Start, Aufnahme oder Abschluss**: `reserve_delete` haelt die Besprechung
//!   gesperrt (`delete_in_progress`, kein Start mehr in sie), beendet einen laufenden Start
//!   (`meeting_not_found`) und eine laufende Aufnahme (ohne Enddurchlauf) und wartet den
//!   Abschluss eines Stopps ab (Pipeline leeren, Writer schliessen: `Finalizing`). Die
//!   Reservierung faellt erst nach Grabstein und Dateiloeschung. Tests: `lifecycle.rs`.
//! - **Voller Datentraeger, Schreibfehler, Kopf-Flush** (gemeinsamer Pfad, auch Live-Aufnahmen):
//!   der Callback verwirft den WAV-Writer (auch bei einem Fehler des Kopf-Flush) und setzt ein
//!   Merkzeichen (`WriteFlags`, nur Atomics); ein Waechter-Thread und `stop()` schreiben es
//!   DAUERHAFT als `metadata_json.audio_incomplete` und melden `audio_write_failed`. Laesst sich
//!   der Marker nicht speichern, versucht es der Waechter weiter, und `stop()` weicht fail-closed
//!   auf `Keep(AudioIncomplete)` aus (`settle_after_stop`). Tests: `a_write_error_...`,
//!   `a_header_flush_error_...`, `a_marker_that_cannot_be_stored_keeps_the_live_transcript`,
//!   `wav_writer.rs`, `final_pass.rs` (Abdeckung je Spur).
//! - **Nicht sauberer Abschluss des Loopbacks (lva#538 Runde 5)**: `LoopbackCapture::stop`
//!   liefert `CaptureEnd {lost, error, panicked, timeline_uncertain}` (nie ein `bool`). Jedes Feld
//!   (Verlust beim Abschluss, Geraetefehler, Panik des Capture-Threads, deren `join` fehlschlaegt,
//!   unsichere Zeitachse durch ein Paket ohne Stempel, einen ignorierten Stempel oder eine ueber
//!   `MAX_GAP` gekuerzte Pause) macht den Systemkanal dauerhaft unvollstaendig:
//!   `settle_after_stop` schreibt `audio_incomplete` mit Grund, laesst `capture_open` stehen, und
//!   der Enddurchlauf ersetzt nichts. Tests: die
//!   `..._keeps_the_live_transcript_after_a_restart` in `final_pass.rs` (echter Capture-Thread ->
//!   `stop_loopback` -> `settle_after_stop` -> Store neu geoeffnet -> `run_job`).
//! - **Kein Ausgabegeraet / Loopback scheitert**: der Start schlaegt mit
//!   `playback_capture_failed` fehl, nie still "nur Mikrofon" (es gibt keins); das Ziel wird
//!   aus dem gespeicherten Vorzustand zurueckgesetzt. Test: `a_failing_start_reports_its_error_at_once`.
//! - **Treiber haengt (Frist) oder stirbt**: `playback_capture_timeout`; stirbt die Capture
//!   mitten in der Aufnahme, meldet der Waechter `playback_capture_died`, die Aufnahme bleibt
//!   stehen, der Nutzer stoppt, das bis dahin Gehoerte bleibt. Ein Treiber, der im Start
//!   abstuerzt, ist ein Fehler, kein Haenger (`a_panicking_start_is_a_failure_not_a_hang`).
//! - **Absturz eines Kindthreads (DSP, Worker)**: der Join meldet es im Log (`join_all`), die
//!   Barriere kehrt trotzdem zurueck; der Rest der Aufnahme (WAV) ist unabhaengig davon.
//! - **Absturz der App**: die Pfade stehen ab dem Start in der Datenbank, `recover_orphans`
//!   repariert `system.wav` und holt den Rest nach. Fail-closed: ein Mitschnitt wird nur
//!   zurueckgesetzt, wenn er NACHWEISLICH nie aufgenommen hat (`playback::audio_state`:
//!   `NotFound` oder nur der WAV-Kopf); ein Dateizustand, der sich nicht lesen laesst, ist
//!   `Unknown` und laesst alles stehen. Tests: `the_recovery_...`, `playback.rs`.
//! - **Warten auf den Mikrofonkanal**: der DSP-Thread, `drain()` und der
//!   Enddurchlauf behandeln jede Spur einzeln; ein nie beschickter Kanal blockiert
//!   nichts. Stille am Systemton fragt der Waechter nach 30 s nach (`WatchConfig::playback()`).
//! - **Null-WAV-Leichen**: es wird nur `system.wav` angelegt; ein Mitschnitt, der ohne einen
//!   Ton endet, setzt den Eintrag zurueck (`stop`); ein gescheiterter Start entfernt nur, was
//!   er selbst angelegt hat (`remove_capture_files`), und nur, wenn kein Satz gespeichert ist
//!   (`discard_failed_start` behaelt sonst Transkript UND Audio).
//! - **Echtzeit**: der Callback laeuft ganz unter der Sperre seines Sinks (`closed`-Pruefung,
//!   WAV-Append, Kopf-Flush, `try_send` an den DSP, Pegel): keine Datenbank, kein Warten auf
//!   etwas anderes. Ein Ueberlauf der DSP-Queue wird gezaehlt und als
//!   `Health { QueueOverflow }` gemeldet (nicht verschluckt). Vorbestehend und unveraendert:
//!   das WAV-Append selbst (Datei-I/O), die Allokation seines Byte-Puffers und die Sperre des
//!   Sinks laufen im Callback-Thread; die Sperre wird nur kurz vom Abbruch/Stopp gehalten.

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
pub use super::lifecycle::START_CANCELLED;
use super::lifecycle::{
    lock_recovering, CancelOp, DeleteStep, RunCore, StartTicket, StopStep, StopTicket,
    MEETING_NOT_FOUND, START_BUSY,
};
use super::mic_capture::MeetingMicCapture;
use super::signal_watch::HealthState;
use super::store::{Meeting, MeetingSource, MeetingStatus, MeetingStore, StoredSegment};
use crate::audio_toolkit::audio::{CaptureEnd, LoopbackCapture, StreamingWavWriter};
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
/// Wie oft das Warten auf den Capture-Start den Abbruchmerker prueft.
const START_POLL: Duration = Duration::from_millis(25);
/// Wie lange ein Stopp auf das Ende eines abgebrochenen Starts (oder einer laufenden
/// Aufnahme-Anzeige) wartet. Danach `start_busy`: ein Fehler, kein stiller Erfolg.
const START_CANCEL_WAIT: Duration = Duration::from_secs(12);
/// Wie lange das Loeschen auf eine laufende Aufnahme, einen laufenden Start oder den
/// Abschluss eines Stopps (Pipeline leeren, Writer schliessen) wartet. Danach
/// `recording_busy`: nichts wurde geloescht, ein neuer Versuch ist moeglich.
const DELETE_WAIT: Duration = Duration::from_secs(60);
/// Wie oft der Schreib-Waechter die Merkzeichen der WAV-Writer liest (und einen noch nicht
/// gespeicherten Marker erneut zu speichern versucht).
const WRITE_WATCH_INTERVAL: Duration = Duration::from_millis(500);
/// So oft versucht `stop()`, den Marker `audio_incomplete` zu speichern, bevor es
/// fail-closed auf `Keep(AudioIncomplete)` ausweicht.
const MARKER_TRIES: u32 = 3;
/// Fehlercode (Ereignis): die WAV liess sich nicht weiter schreiben.
pub const AUDIO_WRITE_FAILED: &str = "audio_write_failed";

/// Transcript channel ids (mirror `StoredSegment::channel`).
pub const CHANNEL_MIC: u8 = 0;
pub const CHANNEL_SYSTEM: u8 = 1;

/// Was eine Aufnahme aufnimmt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureMode {
    /// Die gewohnte Besprechung: Mikrofon, auf Wunsch dazu der Systemton.
    Meeting { capture_system: bool },
    /// #537: NUR der Systemton, in eine YouTube-Besprechung. Ohne Systemton gibt es
    /// nichts aufzunehmen, darum ist sein Start Pflicht (kein stilles "nur Mikrofon").
    PlaybackOnly,
}

impl CaptureMode {
    /// Gibt es einen Mikrofonkanal (Capture, `mic.wav`, Echo-Unterdrueckung)?
    pub(crate) fn mic(self) -> bool {
        matches!(self, CaptureMode::Meeting { .. })
    }

    /// Gibt es einen Systemton-Kanal (`system.wav`)?
    pub(crate) fn system(self) -> bool {
        match self {
            CaptureMode::Meeting { capture_system } => capture_system,
            CaptureMode::PlaybackOnly => true,
        }
    }

    /// Fehlercode fuer die Oberflaeche, wenn der Systemton nicht startete. In der
    /// Besprechung geht es mit dem Mikrofon weiter (die Texte sagen das), beim
    /// Mitschnitt scheitert der Start.
    fn loopback_start_code(self, failure: &LoopbackStartError) -> &'static str {
        match (self, failure) {
            (_, LoopbackStartError::Cancelled) => START_CANCELLED,
            (CaptureMode::PlaybackOnly, LoopbackStartError::Timeout) => "playback_capture_timeout",
            (CaptureMode::PlaybackOnly, _) => "playback_capture_failed",
            (_, LoopbackStartError::Timeout) => "loopback_start_timeout",
            (_, _) => "loopback_start_failed",
        }
    }

    /// Die Meldungen, wenn der Loopback NACH erfolgreichem Start stirbt. In der
    /// Besprechung: Kanalzustand plus Fehler ("laeuft nur mit dem Mikrofon weiter");
    /// beim Mitschnitt nur der eigene Fehler, denn der Mikrofon-Text waere falsch.
    fn loopback_died_events(self, meeting_id: &str) -> Vec<MeetingEvent> {
        let error = |message: &str| MeetingEvent::Error {
            meeting_id: meeting_id.to_string(),
            message: message.to_string(),
        };
        match self {
            CaptureMode::PlaybackOnly => vec![error("playback_capture_died")],
            CaptureMode::Meeting { .. } => vec![
                MeetingEvent::Health {
                    meeting_id: meeting_id.to_string(),
                    channel: CHANNEL_SYSTEM,
                    state: HealthState::LoopbackDied,
                },
                error("loopback_died"),
            ],
        }
    }
}

/// Warum ein Capture-Start nicht zustande kam.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LoopbackStartError {
    /// Der Start meldete einen Fehler (kein Geraet, Treiber).
    Failed(String),
    /// Der Treiber antwortete nicht innerhalb der Frist.
    Timeout,
    /// Stopp oder Loeschen waehrend des Starts.
    Cancelled,
}

/// Fuehrt `start` auf einem Wegwerf-Thread aus und wartet hoechstens `timeout`:
/// ein haengender Treiber (bekannter Befund aus Task 4) darf den Start nicht
/// ewig blockieren. Das Warten prueft alle [`START_POLL`] den Abbruchmerker `cancel`
/// des ganzen Starts (Stopp, Loeschen).
///
/// lva#538: Endet das Warten OHNE Capture (Frist, Abbruch), ruft es `abandon` auf, BEVOR
/// es zurueckkehrt. `abandon` schliesst den Sink dieser Capture (`close_sink`): das ist die
/// Callback-Sperre, die unter derselben Sperre wie jeder Schreibzugriff liegt. Ein
/// Callback, der gerade laeuft, ist damit fertig; ein spaeterer schreibt, speist und
/// meldet nichts mehr, auch wenn der Treiber nach der Frist doch noch zurueckkehrt. Die
/// Capture selbst, die dann aus dem Kanal faellt, stoppt ihr `Drop`.
pub(crate) fn start_bounded<T: Send + 'static>(
    thread_name: &str,
    timeout: Duration,
    cancel: &AtomicBool,
    abandon: impl FnOnce(),
    start: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, LoopbackStartError> {
    if cancel.load(Ordering::Acquire) {
        abandon();
        return Err(LoopbackStartError::Cancelled);
    }
    let (tx, rx) = mpsc::channel::<Result<T, String>>();
    std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || {
            let _ = tx.send(start());
        })
        .map_err(|e| LoopbackStartError::Failed(format!("start thread: {e}")))?;
    let deadline = Instant::now() + timeout;
    loop {
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(START_POLL);
        match rx.recv_timeout(wait) {
            Ok(Ok(started)) => {
                // Ein Abbruch, der gleichzeitig kam: erst die Sperre, dann faellt die Capture
                // (`Drop` stoppt sie), nie umgekehrt.
                if cancel.load(Ordering::Acquire) {
                    abandon();
                    return Err(LoopbackStartError::Cancelled);
                }
                return Ok(started);
            }
            Ok(Err(e)) => return Err(LoopbackStartError::Failed(e)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::Acquire) {
                    abandon();
                    return Err(LoopbackStartError::Cancelled);
                }
                if Instant::now() >= deadline {
                    abandon();
                    return Err(LoopbackStartError::Timeout);
                }
            }
            // Der Start-Thread ist ohne Ergebnis zu Ende (Panik im Treiber).
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(LoopbackStartError::Failed(
                    "start thread ended without a result".to_string(),
                ))
            }
        }
    }
}

/// Gilt fuer diese Besprechung der Plan des Enddurchlaufs (Recovery, "Fortsetzen")?
/// Eine Live-Aufnahme immer; eine YouTube-Besprechung nur, wenn sie eine eigene
/// Aufnahme hat (#537, Systemton): ohne Aufnahme gibt es nichts zu verarbeiten.
/// Import und Untertitel bekommen keinen Enddurchlauf (`KeepReason::NotLive`).
pub(crate) fn runs_final_pass(meeting: &Meeting) -> bool {
    match meeting.source.as_str() {
        "live" => true,
        "youtube" => meeting.mic_audio_path.is_some() || meeting.system_audio_path.is_some(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Pure rules
// ---------------------------------------------------------------------------

/// Recording a meeting without a confirmed consent hint is refused outright —
/// the consent confirmation is a product requirement (Spec A1), not a UI nicety.
pub fn consent_gate(consent_confirmed: bool) -> Result<(), String> {
    if consent_confirmed {
        Ok(())
    } else {
        Err("consent_required".to_string())
    }
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
    // lva#538, fail-closed: hat die Pipeline vor dem Abbruch schon Saetze gespeichert (oder
    // laesst sich das nicht pruefen), bleiben Transkript UND Audio stehen. Geloescht wird
    // nur, was nachweislich leer ist.
    match store.get_segments(meeting_id) {
        Ok(segments) if segments.is_empty() => {}
        Ok(_) | Err(_) => {
            warn!("meetings: failed start of {meeting_id}: a transcript exists or is unreadable - audio kept");
            return;
        }
    }
    remove_capture_files(dir);
    if let Err(e) = store.set_audio_paths(meeting_id, None, None, None) {
        warn!("meetings: failed start of {meeting_id}: audio paths not cleared: {e}");
    }
    // Es gibt kein Audio mehr, das der Marker schuetzen koennte.
    if let Err(e) = store.end_capture(meeting_id) {
        warn!("meetings: failed start of {meeting_id}: capture marker not cleared: {e}");
    }
}

/// Entfernt, was ein Aufnahmestart im Ordner angelegt haben kann (`mic.wav`,
/// `system.wav`, `mic_aec.wav`), und den Ordner, wenn er danach leer ist. Fremdes
/// (Folien, Notizen einer YouTube-Besprechung) bleibt. Best effort: was sich nicht
/// loeschen laesst, steht im Log.
pub(crate) fn remove_capture_files(dir: &std::path::Path) {
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
    /// lva#538: die Callback-Sperre. Gesetzt unter der Sperre dieses Sinks (`close_sink`,
    /// `finalize_sink`), gelesen vom Callback unter derselben Sperre: ein laufender Callback
    /// ist damit fertig, ein spaeterer tut nichts mehr (kein WAV, keine DSP-Queue, kein
    /// Pegel). Dauerhaft: ein geschlossener Sink oeffnet nie wieder.
    closed: bool,
}

impl ChannelSink {
    fn new(writer: StreamingWavWriter) -> Self {
        Self {
            writer: Some(writer),
            samples_since_flush: 0,
            closed: false,
        }
    }
}

/// Wohin die Pegel der Capture-Callbacks gehen (die App sendet Ereignisse, ein Test
/// zaehlt mit). Nur `&self`: der Callback ruft es ohne Sperre mit Wartezeit.
trait LevelSink: Send + Sync {
    fn level(&self, channel: u8, rms: f32);
}

impl LevelSink for LevelEmitter {
    fn level(&self, channel: u8, rms: f32) {
        self.record(channel, rms);
    }
}

/// lva#538: ein Schreibfehler der WAV je Kanal. Der Capture-Callback setzt nur das
/// Merkzeichen (Atomic, keine Allokation, kein Lock); der Schreib-Waechter und `stop()`
/// machen daraus den dauerhaften Marker `metadata_json.audio_incomplete` und die Meldung.
#[derive(Default)]
struct WriteFlags {
    mic: AtomicBool,
    system: AtomicBool,
    reported: AtomicBool,
}

impl WriteFlags {
    fn flag(&self, channel: u8) -> &AtomicBool {
        if channel == CHANNEL_MIC {
            &self.mic
        } else {
            &self.system
        }
    }

    /// Die Kanaele, deren WAV nicht vollstaendig geschrieben wurde.
    fn failed_channels(&self) -> Vec<u8> {
        [CHANNEL_MIC, CHANNEL_SYSTEM]
            .into_iter()
            .filter(|c| self.flag(*c).load(Ordering::Acquire))
            .collect()
    }

    /// `true` fuer den ersten Aufruf: die Meldung an die Oberflaeche geht nur einmal raus.
    fn first_report(&self) -> bool {
        !self.reported.swap(true, Ordering::AcqRel)
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

/// Everything a running meeting owns.
///
/// lva#538: das Fallenlassen ist die BARRIERE (`Drop`): Sinks schliessen (Callback-Sperre),
/// Captures stoppen und joinen, DSP-Thread und Live-Worker beenden und joinen. Danach hat die
/// Sitzung keine Nebenwirkung mehr (kein WAV, kein DSP, kein Pegel, kein Segment). Jeder
/// Weg ohne ordentlichen Stopp (Abbruch des Starts, Fehler, Panik) laeuft hier durch; der
/// Aufrufer raeumt Zeile und Dateien erst DANACH auf. `stop()` und das Loeschen nehmen die
/// Teile mit [`RecordingSession::into_parts`] heraus und schliessen sie selbst, in ihrer
/// Reihenfolge.
struct RecordingSession {
    meeting_id: String,
    mode: CaptureMode,
    paused: Arc<AtomicBool>,
    mic_capture: Option<MeetingMicCapture>,
    loopback: Option<LoopbackCapture>,
    /// `None` nur beim Mitschnitt des Systemtons (#537): dann gibt es kein Mikrofon.
    mic_sink: Option<Arc<Mutex<ChannelSink>>>,
    system_sink: Option<Arc<Mutex<ChannelSink>>>,
    mic_path: Option<PathBuf>,
    system_path: Option<PathBuf>,
    /// DSP thread + transcription worker of this meeting.
    pipeline: Option<LivePipeline>,
    /// Schreibfehler der WAV-Writer (lva#538), vom Callback gesetzt.
    write_flags: Arc<WriteFlags>,
    /// Beendet den Schreib-Waechter mit der Aufnahme.
    watch_stop: Arc<AtomicBool>,
    /// lva#544: die Anmeldung dieser Aufnahme im [`FinalSlot`]. Solange sie lebt, beginnt kein
    /// Enddurchlauf-/Recovery-Auftrag mit einem Modell. Sie faellt mit der Sitzung (nach der
    /// Barriere) oder in `stop()`, sobald der Live-Worker leer ist (`SessionParts::live`).
    live: Option<LiveHalter>,
}

/// Die Teile einer Sitzung, nachdem `stop()` oder das Loeschen sie uebernommen haben.
struct SessionParts {
    meeting_id: String,
    mode: CaptureMode,
    mic_capture: Option<MeetingMicCapture>,
    loopback: Option<LoopbackCapture>,
    mic_sink: Option<Arc<Mutex<ChannelSink>>>,
    system_sink: Option<Arc<Mutex<ChannelSink>>>,
    mic_path: Option<PathBuf>,
    system_path: Option<PathBuf>,
    pipeline: Option<LivePipeline>,
    write_flags: Arc<WriteFlags>,
    watch_stop: Arc<AtomicBool>,
    live: Option<LiveHalter>,
}

impl RecordingSession {
    fn new(
        meeting_id: &str,
        mode: CaptureMode,
        mic_sink: Option<Arc<Mutex<ChannelSink>>>,
        system_sink: Option<Arc<Mutex<ChannelSink>>>,
        mic_path: Option<PathBuf>,
        system_path: Option<PathBuf>,
        live: LiveHalter,
    ) -> Self {
        Self {
            meeting_id: meeting_id.to_string(),
            mode,
            paused: Arc::new(AtomicBool::new(false)),
            mic_capture: None,
            loopback: None,
            mic_sink,
            system_sink,
            mic_path,
            system_path,
            pipeline: None,
            write_flags: Arc::new(WriteFlags::default()),
            watch_stop: Arc::new(AtomicBool::new(false)),
            live: Some(live),
        }
    }

    /// Gibt die Teile ab; was in `self` zurueckbleibt, ist leer (das `Drop` tut dann nichts).
    fn into_parts(mut self) -> SessionParts {
        SessionParts {
            meeting_id: std::mem::take(&mut self.meeting_id),
            mode: self.mode,
            mic_capture: self.mic_capture.take(),
            loopback: self.loopback.take(),
            mic_sink: self.mic_sink.take(),
            system_sink: self.system_sink.take(),
            mic_path: self.mic_path.take(),
            system_path: self.system_path.take(),
            pipeline: self.pipeline.take(),
            write_flags: Arc::clone(&self.write_flags),
            watch_stop: Arc::clone(&self.watch_stop),
            live: self.live.take(),
        }
    }
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        self.watch_stop.store(true, Ordering::Release);
        // 1. Callback-Sperre: ab hier schreibt, speist und meldet kein Callback mehr.
        close_sink(self.mic_sink.as_ref());
        close_sink(self.system_sink.as_ref());
        // 2. Die Captures: stoppen und joinen.
        if let Some(capture) = self.mic_capture.take() {
            capture.stop();
        }
        if let Some(capture) = self.loopback.take() {
            capture.stop();
        }
        // 3. DSP-Thread und Worker: beenden und joinen, den Rest abwerfen.
        if let Some(pipeline) = self.pipeline.take() {
            pipeline.abort();
        }
    }
}

/// lva#538: die Reservierung zum Loeschen einer Besprechung (`reserve_delete`). Solange sie
/// lebt, nimmt kein Start in diese Besprechung auf. Der Aufrufer laesst sie erst fallen,
/// wenn Tombstone und Dateien weg sind (oder das Loeschen scheiterte).
pub struct DeleteReservation {
    recorder: Arc<MeetingRecorderManager>,
    meeting_id: String,
}

impl Drop for DeleteReservation {
    fn drop(&mut self) {
        self.recorder.core.release_delete(&self.meeting_id);
    }
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
    ///
    /// Entsteht der Thread nicht (Fehler), bleibt der Vorgaenger MIT seinem Merker im Platz: der
    /// Start einer Aufnahme bricht ihn weiter ab und wartet ihn ab ([`FinalSlot::live_beginnen`]).
    pub(crate) fn spawn(
        &self,
        body: impl FnOnce(Option<std::thread::JoinHandle<()>>, Arc<AtomicBool>) + Send + 'static,
    ) -> std::io::Result<()> {
        self.spawn_mit(body, |lauf| {
            std::thread::Builder::new()
                .name("meeting-final-pass".to_string())
                .spawn(lauf)
        })
    }

    /// [`FinalSlot::spawn`] mit dem Threadstart als Parameter (Tests lassen ihn scheitern).
    fn spawn_mit(
        &self,
        body: impl FnOnce(Option<std::thread::JoinHandle<()>>, Arc<AtomicBool>) + Send + 'static,
        starten: impl FnOnce(
            Box<dyn FnOnce() + Send + 'static>,
        ) -> std::io::Result<std::thread::JoinHandle<()>>,
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
        let cancel = match &previous {
            Some(p) if !live && !p.cancel.load(Ordering::Acquire) => Arc::clone(&p.cancel),
            _ => Arc::new(AtomicBool::new(live)),
        };
        if live {
            info!("meetings: a recording is live - the new final pass job starts cancelled");
        }
        // Der Vorgaenger geht NICHT in die Closure des neuen Threads (scheiterte der Start, fiele
        // er mit ihr und liefe unverfolgt weiter): sein Handle liegt in einer Uebergabe, aus der
        // es der neue Thread als Erstes nimmt, und sein Merker bleibt hier.
        let (previous_cancel, previous_handle) = match previous {
            Some(p) => (Some(p.cancel), Some(p.handle)),
            None => (None, None),
        };
        let uebergabe = Arc::new(Mutex::new(previous_handle));
        let im_thread = Arc::clone(&uebergabe);
        let job_cancel = Arc::clone(&cancel);
        let gestartet = starten(Box::new(move || {
            let vorher = im_thread.lock().unwrap_or_else(|e| e.into_inner()).take();
            body(vorher, job_cancel)
        }));
        match gestartet {
            Ok(handle) => {
                slot.job = Some(FinalJobHandle { cancel, handle });
                Ok(())
            }
            Err(e) => {
                let vorher = uebergabe.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let (Some(handle), Some(cancel)) = (vorher, previous_cancel) {
                    slot.job = Some(FinalJobHandle { cancel, handle });
                }
                Err(e)
            }
        }
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

/// Was die Anlage der Zeile unter dem Einwilligungs-Tor ergab (lva#544): die Zeile samt Bindung
/// der Planungen, oder die Zeile ohne Bindung (sie ist aufzuraeumen, und zwar AUSSERHALB der
/// Tor-Sperre: das Aufraeumen meldet Ereignisse).
enum Angelegt {
    Gebunden(Meeting, crate::managers::planning::Gebunden),
    BindungFehlt(Meeting, String),
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

pub struct MeetingRecorderManager {
    app: AppHandle,
    store: Arc<MeetingStore>,
    transcription: Arc<TranscriptionManager>,
    /// Der Lebenszyklus (Start, Abbruch, Stopp, Loeschen) als EIN Zustand unter EINER Sperre
    /// (lva#538, `lifecycle.rs`). Ein laufender Start zaehlt als "Aufnahme aktiv" (M2-P2d:
    /// ein `TranscriptFinal` des eben abgebrochenen Enddurchlaufs startet keine lokalen
    /// KI-Notizen neben der neuen Aufnahme).
    core: RunCore<RecordingSession>,
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
        // Der Start registriert sich atomar im Lebenszyklus, bevor er irgendetwas anlegt: ein
        // zweiter Start, ein Stopp und ein Loeschen sehen ihn (lva#538).
        let starting = self.core.begin_start(None, target_meeting_id)?;
        if let Some(target) = target_meeting_id {
            starting.set_target(target)?;
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
        starting.check()?;

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
        let (angelegt, live) = self.finals.start_unter_tor(
            &self.store,
            event,
            target_meeting_id,
            || {},
            |tor| {
                let meeting = begin_live_row(&self.store, &title, consent_at, target_meeting_id)?;
                Ok(match tor.binden(&self.store, &meeting.id) {
                    Ok(gebunden) => Angelegt::Gebunden(meeting, gebunden),
                    Err(e) => Angelegt::BindungFehlt(meeting, e),
                })
            },
        )?;
        let (meeting, gebunden) = match angelegt {
            Angelegt::Gebunden(meeting, gebunden) => (meeting, gebunden),
            // Die Zeile steht, die Bindung nicht: aufraeumen, ausserhalb der Tor-Sperre
            // (das Aufraeumen meldet Ereignisse).
            Angelegt::BindungFehlt(meeting, e) => {
                self.undo_live_start(target_meeting_id, &meeting.id, &e);
                return Err(e);
            }
        };
        let meeting_id = meeting.id.clone();
        starting.set_target(&meeting_id)?;
        // `start_row` kehrt erst zurueck, wenn seine Sitzung die Barriere durchlaufen hat
        // (Captures, DSP und Worker sind zu): erst DANACH wird die Zeile zurueckgesetzt.
        let started = self.start_row(
            meeting,
            CaptureMode::Meeting { capture_system },
            &gebunden,
            live,
            &starting,
        );
        if let Err(e) = &started {
            self.undo_live_start(target_meeting_id, &meeting_id, e);
            // Nichts wurde aufgenommen: was DIESER Start gebunden hat, faellt wieder weg.
            crate::managers::planning::bindung_verwerfen(&self.store, gebunden);
        }
        started
    }

    /// Ein Start einer Besprechung kam nicht zustande (nach dem Anlegen der Zeile): der Zieleintrag
    /// wird wieder leer bzw. die neue Besprechung als `failed` aufgeraeumt. NIE unter der
    /// Tor-Sperre (meldet Ereignisse).
    fn undo_live_start(&self, target_meeting_id: Option<&str>, meeting_id: &str, e: &str) {
        match target_meeting_id {
            Some(target) => self.undo_live_target_start(target, e),
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

    /// Der Start in einen leeren Eintrag kam nicht zustande: der Eintrag wird wieder leer, die
    /// Aufnahmedateien erst danach entfernt. Fail-closed: laesst er sich nicht zuruecksetzen
    /// (die Pipeline hat schon Saetze gespeichert, die Datenbank antwortet nicht), steht er auf
    /// `failed`, mit Transkript und Audio.
    fn undo_live_target_start(&self, target: &str, error: &str) {
        match self.store.restore_empty_meeting(target) {
            Ok(true) => {
                info!("meetings: Start gescheitert ({error}), Eintrag {target} ist wieder leer");
                if let Ok(dir) = super::meetings_data_dir(&self.app) {
                    let _ = std::fs::remove_dir_all(dir.join(target));
                }
                self.emit_restored(target);
            }
            Ok(false) => {
                warn!("meetings: Eintrag {target} nicht wiederherstellbar, wird `failed` (Audio bleibt)");
                let _ = self.store.set_status(target, MeetingStatus::Failed);
                self.emit_state(target, "failed", false);
            }
            Err(err) => {
                warn!("meetings: Eintrag {target} nicht wiederhergestellt: {err}");
                let _ = self.store.set_status(target, MeetingStatus::Failed);
                self.emit_state(target, "failed", false);
            }
        }
    }

    /// #537: nimmt den Systemton (NUR den, kein Mikrofon) in die vorhandene
    /// YouTube-Besprechung `meeting_id` auf, solange der Nutzer das Video im
    /// eingebetteten Player abspielt. Titel, Quelle, Notizen und Projekte bleiben;
    /// Transkript, Zeitachse und Stopp wie bei jeder Aufnahme. Die Einwilligung gilt
    /// zuerst (der Nutzer bestaetigt im Dialog, dass er das Video mitschneiden darf).
    ///
    /// `op` ist die Vorgangs-Id des Frontends: `cancel_start(op)` adressiert genau diesen
    /// Start, auch wenn der Abbruch-Befehl den Start-Befehl ueberholt (lva#538).
    ///
    /// Fehler: `consent_required`, `already_recording`, `dictation_active`,
    /// `meeting_not_found` (auch: waehrend des Starts geloescht), `target_not_empty`
    /// (kein YouTube-Eintrag ohne Transkript und ohne Aufnahme: nichts startet, nichts
    /// aendert sich), `playback_capture_failed` / `playback_capture_timeout` (der
    /// Systemton liess sich nicht oeffnen), `start_cancelled` (ein Stopp oder Abbruch kam
    /// dazwischen), `nur_notizen: <grund>` (das Einwilligungs-Tor der Besprechungsplanung
    /// sperrt, lva#544). Nach jedem Fehler nach der Umwandlung ist der Eintrag wieder wie
    /// vorher (`undo_playback_start`).
    pub fn start_playback_capture(
        &self,
        meeting_id: &str,
        consent_confirmed: bool,
        op: Option<&str>,
    ) -> Result<Meeting, String> {
        consent_gate(consent_confirmed)?;
        let starting = self.core.begin_start(op, Some(meeting_id))?;

        if let Some(rm) = self
            .app
            .try_state::<Arc<crate::managers::audio::AudioRecordingManager>>()
        {
            if rm.is_recording() {
                return Err("dictation_active".to_string());
            }
        }
        starting.check()?;

        // Einwilligungs-Tor der Besprechungsplanung (lva#534, lva#544), dieselbe Reihenfolge wie bei
        // `start_for_event` (Sperrordnung in `managers::planning`): ERST die Aufnahme im
        // `FinalSlot` anmelden (ein laufender Enddurchlauf wird abgebrochen und abgewartet,
        // ohne Tor-Sperre), DANN unter der Tor-Sperre das Tor frisch pruefen (die Planungen der
        // YouTube-Besprechung, sofern sie einer zugeordnet ist), die Zeile atomar umwandeln
        // (samt Vorzustand) und die geprueften Planungen BINDEN. Ohne Zustimmung aller
        // Anwesenden bleibt der Eintrag unveraendert (`nur_notizen: <grund>`).
        // Folge der Reihenfolge: auch eine Anfrage, die danach an der Zielpruefung scheitert
        // (`target_not_empty`), hat den Enddurchlauf einer anderen Besprechung abgebrochen.
        let consent_at = chrono::Utc::now().timestamp();
        let (angelegt, live) = self.finals.start_unter_tor(
            &self.store,
            None,
            Some(meeting_id),
            || {},
            |tor| {
                let meeting = self
                    .store
                    .begin_playback_capture(meeting_id, consent_at)
                    .map_err(|e| coded_error("meeting_create_failed", &e))?;
                Ok(match tor.binden(&self.store, &meeting.id) {
                    Ok(gebunden) => Angelegt::Gebunden(meeting, gebunden),
                    Err(e) => Angelegt::BindungFehlt(meeting, e),
                })
            },
        )?;
        let (meeting, gebunden) = match angelegt {
            Angelegt::Gebunden(meeting, gebunden) => (meeting, gebunden),
            Angelegt::BindungFehlt(_, e) => {
                info!("meetings: Mitschnitt gescheitert ({e}), {meeting_id} wird zurueckgesetzt");
                self.undo_playback_start(meeting_id);
                return Err(e);
            }
        };

        let started = self.start_row(
            meeting,
            CaptureMode::PlaybackOnly,
            &gebunden,
            live,
            &starting,
        );
        if let Err(e) = &started {
            info!("meetings: Mitschnitt gescheitert ({e}), {meeting_id} wird zurueckgesetzt");
            self.undo_playback_start(meeting_id);
            // Nichts wurde aufgenommen: was DIESER Start gebunden hat, faellt wieder weg.
            crate::managers::planning::bindung_verwerfen(&self.store, gebunden);
        }
        started
    }

    /// Der Mitschnitt kam nicht zustande: die Zeile ist wieder die YouTube-Besprechung von
    /// vorher (aus dem gespeicherten Vorzustand), danach erst werden die Dateien entfernt.
    /// Voraussetzung: die Barriere der Sitzung ist durchlaufen (nichts schreibt mehr).
    ///
    /// Saetze, die die Pipeline vor dem Abbruch gespeichert hat, gehoeren zu diesem nie
    /// veroeffentlichten Start (vorher hatte der Eintrag keine, `begin_playback_capture`
    /// prueft das): `rollback_playback_capture` verwirft sie in DERSELBEN Transaktion wie das
    /// Zuruecksetzen. Fail-closed: laesst sich die Zeile nicht zuruecksetzen, bleiben Dateien
    /// und Transkript, die Zeile steht auf `failed` (mit dem Merker; der naechste App-Start
    /// setzt sie zurueck, sofern nachweislich nichts aufgenommen wurde), nie auf `recording`
    /// (er belebt sie sonst als Absturzrest).
    fn undo_playback_start(&self, meeting_id: &str) {
        match self.store.rollback_playback_capture(meeting_id) {
            Ok(true) => {
                match super::meetings_data_dir(&self.app) {
                    Ok(base) => remove_capture_files(&base.join(meeting_id)),
                    Err(err) => {
                        warn!("meetings: Mitschnitt gescheitert, Datenordner unbekannt ({err})")
                    }
                }
                self.emit_restored(meeting_id);
            }
            Ok(false) | Err(_) => {
                warn!("meetings: Eintrag {meeting_id} nicht wiederhergestellt, wird `failed`");
                let _ = self.store.set_status(meeting_id, MeetingStatus::Failed);
                self.emit_state(meeting_id, "failed", false);
            }
        }
    }

    /// lva#538: der Befehl "Start abbrechen" mit der Vorgangs-Id des Frontends. Der Abbruch geht
    /// nie verloren: kommt er VOR der Registrierung des Starts (der Befehl ueberholt den
    /// Start-Befehl), wird die Id vermerkt, und der Start endet bei der Registrierung mit
    /// `start_cancelled`. Laeuft der Start, kehrt dieser Aufruf erst zurueck, wenn er
    /// aufgeraeumt hat (`start_busy`, wenn er in einem Treiber haengt). Hat er schon
    /// veroeffentlicht, ist der Abbruch ein Stopp genau dieser Aufnahme. Blockierend.
    pub fn cancel_start(&self, op: &str) -> Result<(), String> {
        match self.core.cancel_op(op) {
            CancelOp::Remembered | CancelOp::Finished => Ok(()),
            // Zu viele Abbrueche warten: dieser ist NICHT bestaetigt. Wie ein haengender Abbruch
            // ein Fehler mit neuem Versuch, nie ein stiller Erfolg.
            CancelOp::Refused => Err(START_BUSY.to_string()),
            CancelOp::Starting => {
                if self
                    .core
                    .wait_start_end(op, Instant::now() + START_CANCEL_WAIT)
                {
                    Ok(())
                } else {
                    Err(START_BUSY.to_string())
                }
            }
            CancelOp::Running => match self.stop_op(Some(op)) {
                Ok(_) => Ok(()),
                // Inzwischen von jemand anderem gestoppt: das Ziel ist erreicht.
                Err(e) if e == "not_recording" => Ok(()),
                Err(e) => Err(e),
            },
        }
    }

    /// Der Teil des Starts nach dem Anlegen bzw. Umwandeln der Zeile.
    ///
    /// lva#538: der Start ist abbrechbar. Er prueft den Abbruchmerker (Stopp, Loeschen) vor
    /// jedem Schritt, der etwas anlegt; das Warten auf den Treiber prueft ihn im Takt;
    /// `publish_started` entscheidet unter der Sperre des Lebenszyklus. Die Sitzung entsteht
    /// FRUEH (mit den Sinks) und traegt die Barriere in ihrem `Drop`: jeder Fehler- und
    /// Abbruchweg schliesst Sinks, Captures, DSP und Worker, bevor diese Funktion
    /// zurueckkehrt. Der Aufrufer raeumt Zeile und Dateien erst danach auf.
    ///
    /// lva#544: laeuft OHNE Tor-Sperre. `gebunden` beweist, dass das Einwilligungs-Tor bestanden
    /// und die Planungen an die Besprechung gebunden sind; der wirksame Beginn (ab dem Ton
    /// gespeichert und transkribiert wird) ist die zweite Pruefung unter der Sperre
    /// (`Gebunden::wirksam`), kurz vor der Veroeffentlichung. Bis dahin verwerfen die
    /// Capture-Callbacks alles (`erst_wenn_wirksam`). `live` ist die Anmeldung der Aufnahme im
    /// `FinalSlot`; sie gehoert ab der Sitzung der Aufnahme (und faellt mit jedem Fehlerweg).
    fn start_row(
        &self,
        meeting: Meeting,
        mode: CaptureMode,
        gebunden: &crate::managers::planning::Gebunden,
        live: LiveHalter,
        starting: &StartTicket<'_, RecordingSession>,
    ) -> Result<Meeting, String> {
        crate::managers::planning::ohne_tor_sperre("Aufbau der Aufnahme (start_row)");
        let meeting_id = meeting.id.clone();
        debug_assert_eq!(gebunden.meeting_id(), meeting_id);
        starting.check()?;

        let dir = super::meetings_data_dir(&self.app)
            .map_err(|e| format!("app_data_dir_failed: {e}"))?
            .join(&meeting_id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("meeting_dir_failed: {e}"))?;

        // Nur die Dateien des Modus: beim Mitschnitt entsteht keine `mic.wav`
        // (sonst bliebe eine Null-Spur liegen, auf die der Enddurchlauf wartet).
        let mic_path = mode.mic().then(|| dir.join("mic.wav"));
        let mic_writer = mic_path
            .as_ref()
            .map(|path| {
                StreamingWavWriter::create(path, SAMPLE_RATE)
                    .map_err(|e| format!("mic_wav_failed: {e}"))
            })
            .transpose()?;
        let system_path = mode.system().then(|| dir.join("system.wav"));
        let system_writer = system_path
            .as_ref()
            .map(|path| {
                StreamingWavWriter::create(path, SAMPLE_RATE)
                    .map_err(|e| format!("system_wav_failed: {e}"))
            })
            .transpose()?;
        let mic_sink = mic_writer.map(|w| Arc::new(Mutex::new(ChannelSink::new(w))));
        let system_sink = system_writer.map(|w| Arc::new(Mutex::new(ChannelSink::new(w))));
        // Ab hier traegt die Sitzung die Barriere (`Drop`).
        let mut session = RecordingSession::new(
            &meeting_id,
            mode,
            mic_sink,
            system_sink,
            mic_path,
            system_path,
            live,
        );

        // Written now, not at stop(): crash recovery can only repair WAVs whose
        // paths it knows, and a crash is exactly the case where stop() never ran.
        // lva#538: ein Fehler hier ist ein Fehler des Starts (nicht mehr "nur loggen"):
        // ohne Pfade repariert die Wiederherstellung nichts, und fuer eine inzwischen
        // geloeschte Zeile gibt es keine Aufnahme.
        // lva#538 (Runde 3): die Pfade und der pessimistische Marker `capture_open` stehen in
        // DERSELBEN Transaktion. Er faellt erst, wenn alle Writer sauber zu sind (`stop`); ein
        // Absturz laesst ihn stehen, und das Audio gilt dann als unvollstaendig.
        self.store
            .begin_capture(
                &meeting_id,
                session.mic_path.as_deref().and_then(|p| p.to_str()),
                session.system_path.as_deref().and_then(|p| p.to_str()),
            )
            .map_err(|e| audio_paths_error(&e))?;

        // lva#544: bis zum wirksamen Beginn (unten) verwerfen die Capture-Callbacks alles.
        let wirksam = Arc::new(AtomicBool::new(false));
        self.open_captures(&mut session, starting, &dir, &wirksam)?;

        // Der letzte Augenblick vor der Veroeffentlichung: abgebrochen, oder die Zeile ist
        // inzwischen geloescht? Dann wird nichts daraus (die Sitzung faellt durch die Barriere).
        starting
            .check()
            .and_then(|()| self.ensure_present(&meeting_id))?;
        let write_flags = Arc::clone(&session.write_flags);
        let watch_stop = Arc::clone(&session.watch_stop);
        // lva#544: der WIRKSAME Beginn. Unter der Tor-Sperre wird das Tor ein zweites Mal frisch
        // geprueft (ein Widerruf seit der Bindung gewinnt) und nur der Merker umgelegt: erst ab
        // jetzt speichern die Callbacks Ton und speisen die Transkription. Scheitert die Pruefung,
        // wurde nichts aufgenommen: die Sitzung faellt durch die Barriere (Captures zu), der Start
        // gilt als gescheitert und der Aufrufer raeumt auf.
        if let Err(code) = gebunden.wirksam(&self.store, || {
            wirksam.store(true, Ordering::Release);
        }) {
            info!("meetings: Start nicht wirksam ({meeting_id}, {code})");
            return Err(code);
        }
        // Zustand und Sitzung zusammen unter der Sperre des Lebenszyklus und erst jetzt, wo die
        // Captures laufen (#15). Ein Stopp, der zwischen der Pruefung und hier kam, hat den
        // Abbruchmerker gesetzt: die Sitzung kommt zurueck und faellt durch die Barriere.
        if let Err(session) = self.core.publish_started(starting, &meeting_id, session) {
            drop(session);
            return Err(starting.cancel_code().to_string());
        }

        self.set_indicator(true);
        self.emit_state(&meeting_id, "recording", false);
        self.watch_writes(&meeting_id, &write_flags, &watch_stop);
        info!("meetings: recording started ({meeting_id})");
        Ok(meeting)
    }

    /// Pipeline und Captures einer Sitzung; jeder Teil kommt sofort in `session` (dort faengt
    /// ihn die Barriere ab), nie in eine lokale Variable, die bei einem Fehler verloren ginge.
    fn open_captures(
        &self,
        session: &mut RecordingSession,
        starting: &StartTicket<'_, RecordingSession>,
        dir: &std::path::Path,
        wirksam: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        let meeting_id = session.meeting_id.clone();
        let mode = session.mode;
        starting.check()?;
        // M2-P2c2: Echo-Unterdrueckung der Ich-Spur (braucht den Systemton als
        // Referenz). Das Ergebnis geht an die Transkription und nach
        // mic_aec.wav; mic.wav bleibt roh. Ohne Mikrofon (#537) gibt es kein Echo.
        let echo_mode = crate::settings::get_settings(&self.app).meeting_echo_cancellation;
        let echo = (mode.mic() && echo_mode.enabled(mode.system()))
            .then(|| self.echo_setup(&meeting_id, &dir.join(super::MIC_AEC_FILE)));

        let pipeline = self
            .start_pipeline(&meeting_id, echo, mode)
            .inspect_err(|_| {
                let _ = self.store.set_status(&meeting_id, MeetingStatus::Failed);
            })?;
        let system_feed = pipeline.feed(CHANNEL_SYSTEM);
        let mic_feed = pipeline.feed(CHANNEL_MIC);
        let dsp_control = pipeline.control();
        let overflow_stats = pipeline.stats();
        session.pipeline = Some(pipeline);

        // Load the meeting model (dedicated `meeting_model` or the dictation
        // model as fallback); transcribe_segments waits on the load condvar.
        self.transcription
            .initiate_meeting_model_load(&crate::settings::get_settings(&self.app));

        let levels: Arc<dyn LevelSink> = Arc::new(LevelEmitter::new(self.app.clone()));
        // #15: was die begrenzte Queue des Mikrofons verwirft, zaehlt auf demselben
        // Zaehler wie der Ueberlauf der DSP-Queue (`overflow_ms[mic=..]` im Log).
        let mic_overflow: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |samples| {
            overflow_stats.overflow_samples[CHANNEL_MIC as usize]
                .fetch_add(samples, Ordering::Relaxed);
        });
        if let Some(sink) = session.mic_sink.clone() {
            let mic_feed = mic_feed.ok_or("dsp_thread_failed")?;
            starting.check()?;
            let callback = erst_wenn_wirksam(
                Arc::clone(wirksam),
                channel_callback(
                    CHANNEL_MIC,
                    sink,
                    Arc::clone(&session.paused),
                    Arc::clone(&session.write_flags),
                    mic_feed,
                    Arc::clone(&levels),
                ),
            );
            let capture = MeetingMicCapture::start(
                crate::settings::get_settings(&self.app).selected_microphone,
                mic_overflow,
                callback,
            )
            .map_err(|e| {
                // The meeting row exists but nothing was captured — mark it failed
                // rather than leaving a phantom "recording" row behind.
                let _ = self.store.set_status(&meeting_id, MeetingStatus::Failed);
                format!("mic_start_failed: {e}")
            })?;
            session.mic_capture = Some(capture);
        }
        starting.check()?;

        let loopback_parts = match (mode, session.system_sink.clone(), system_feed) {
            (
                CaptureMode::Meeting {
                    capture_system: false,
                },
                _,
                _,
            ) => None,
            (CaptureMode::Meeting { .. }, None, _) => None,
            // Mitschnitt: der Systemton ist alles, was es gibt.
            (CaptureMode::PlaybackOnly, None, _) | (CaptureMode::PlaybackOnly, _, None) => {
                return Err("dsp_thread_failed".to_string())
            }
            (_, Some(sink), Some(feed)) => Some((sink, feed)),
            (_, Some(_), None) => None,
        };
        if let Some((sink, feed)) = loopback_parts {
            let callback = erst_wenn_wirksam(
                Arc::clone(wirksam),
                channel_callback(
                    CHANNEL_SYSTEM,
                    Arc::clone(&sink),
                    Arc::clone(&session.paused),
                    Arc::clone(&session.write_flags),
                    feed,
                    Arc::clone(&levels),
                ),
            );
            let started = self.start_loopback(
                &meeting_id,
                mode,
                starting,
                &sink,
                callback,
                dsp_control.clone(),
            );
            match (mode, started) {
                (_, Ok(capture)) => session.loopback = Some(capture),
                // Mitschnitt: scheitert der Systemton, scheitert der Start (der Aufrufer
                // setzt den Eintrag zurueck).
                (CaptureMode::PlaybackOnly, Err(e)) => return Err(e),
                // Besprechung: es geht mit dem Mikrofon weiter. Ein Abbruch des Starts bleibt
                // aber ein Abbruch.
                (_, Err(e)) if starting.cancelled() => return Err(e),
                (_, Err(_)) => {}
            }
        }
        if session.loopback.is_none() {
            // Ohne Referenz keine Echo-Unterdrueckung (No-op, wenn sie aus ist).
            if let Some(control) = &dsp_control {
                control.reference_lost();
            }
        }
        Ok(())
    }

    /// Ist die Zeile noch da? (Geloescht waehrend des Starts: `meeting_not_found`.)
    fn ensure_present(&self, meeting_id: &str) -> Result<(), String> {
        match self.store.get_meeting(meeting_id) {
            Ok(None) => Err(MEETING_NOT_FOUND.to_string()),
            Ok(Some(_)) | Err(_) => Ok(()),
        }
    }

    /// Starts the loopback capture with a bounded wait (`start_bounded`): a wedged
    /// audio driver must not block the start forever (known Task-4 finding).
    ///
    /// Besprechung: scheitert er, laeuft die Aufnahme mit dem Mikrofon weiter
    /// (Warnung im Log, Fehlerereignis `loopback_start_failed`/`_timeout`; der
    /// Aufrufer verwirft das `Err`). Mitschnitt (#537): scheitert er, scheitert der
    /// Start mit `playback_capture_failed`/`_timeout` (kein Ereignis, der Befehl
    /// meldet den Fehler selbst). Das `Err` traegt immer den Code. Ein Abbruch des Starts
    /// (Stopp, Loeschen) gibt den Code des Abbruchs zurueck, ohne Ereignis.
    fn start_loopback(
        &self,
        meeting_id: &str,
        mode: CaptureMode,
        starting: &StartTicket<'_, RecordingSession>,
        sink: &Arc<Mutex<ChannelSink>>,
        callback: impl FnMut(&[i16], Option<u64>) + Send + 'static,
        dsp_control: Option<DspControl>,
    ) -> Result<LoopbackCapture, String> {
        crate::managers::planning::ohne_tor_sperre("Start des Systemtons");
        let cancel = starting.cancel_flag();
        let started = start_bounded(
            "meeting-loopback-start",
            LOOPBACK_START_TIMEOUT,
            &cancel,
            || close_sink(Some(sink)),
            move || LoopbackCapture::start(callback).map_err(|e| format!("{e:#}")),
        );
        match started {
            Ok(capture) => {
                self.watch_loopback(meeting_id, &capture, mode, dsp_control);
                Ok(capture)
            }
            Err(LoopbackStartError::Cancelled) => Err(starting.cancel_code().to_string()),
            Err(failure) => {
                let code = mode.loopback_start_code(&failure);
                match (&failure, mode) {
                    // Nur der Code und die Art des Fehlers, nie Inhalt.
                    (LoopbackStartError::Failed(e), CaptureMode::PlaybackOnly) => {
                        warn!("meetings: playback capture could not start ({e})");
                        Err(format!("{code}: {e}"))
                    }
                    (_, CaptureMode::PlaybackOnly) => {
                        warn!("meetings: playback capture start timed out");
                        Err(code.to_string())
                    }
                    (LoopbackStartError::Failed(e), _) => {
                        warn!("meetings: loopback start failed ({e}) - continuing mic-only");
                        self.emit_error(meeting_id, code);
                        Err(code.to_string())
                    }
                    (_, _) => {
                        warn!("meetings: loopback start timed out - continuing mic-only");
                        self.emit_error(meeting_id, code);
                        Err(code.to_string())
                    }
                }
            }
        }
    }

    /// Worst failure mode of the system channel: the loopback thread dies
    /// AFTER a successful start (endpoint removed, driver error). The callback
    /// simply goes quiet, the meeting keeps recording and half the minutes are
    /// missing without anyone noticing. This watchdog turns that silence into
    /// an error event; it ends with the capture (stop flag) or right after it
    /// reported the failure. Welche Ereignisse das sind, haengt vom Modus ab
    /// (`CaptureMode::loopback_died_events`).
    fn watch_loopback(
        &self,
        meeting_id: &str,
        capture: &LoopbackCapture,
        mode: CaptureMode,
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
                            "meetings: loopback capture died mid-recording ({meeting_id}) - system audio is gone"
                        );
                        // M2-P2c2: ohne Referenz Echo-Unterdrueckung aus.
                        if let Some(control) = &dsp_control {
                            control.reference_lost();
                        }
                        for event in mode.loopback_died_events(&meeting_id) {
                            let _ = event.emit(&app);
                        }
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
        mode: CaptureMode,
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
        if mode == CaptureMode::PlaybackOnly {
            // Ohne Mikrofon: kein Echo, kein Mikrofon-Waechter (#537).
            cfg = cfg.playback_only();
        } else if let Some(echo) = echo {
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
        self.stop_op(None)
    }

    /// `stop`, mit `op` nur fuer genau diesen Vorgang (`cancel_start`).
    fn stop_op(&self, op: Option<&str>) -> Result<String, String> {
        // #15: der Zustand bleibt `Stopping`, bis die Captures wirklich zu sind (das Ticket
        // stellt ihn auf `Idle`, auch bei einer Panik): vorher galt die Aufnahme schon als
        // beendet, waehrend das Mikrofon noch offen war, und ein Diktat-Hotkey oder ein neuer
        // Start konnte dazwischen greifen.
        let deadline = Instant::now() + START_CANCEL_WAIT;
        let (session, stopping) = match self.core.begin_stop(op, deadline)? {
            StopStep::Running(session, ticket) => (session, ticket),
            // lva#538: ein Stopp WAEHREND des Starts bricht ihn ab (verbindlich); zurueck kehrt er
            // erst, wenn der Start aufgeraeumt hat (der Eintrag ist dann wie vorher). Haengt der
            // Start noch in einem Treiber, ist das ein Fehler, kein stiller Erfolg.
            StopStep::CancelledStart(start_op) => {
                return if self.core.wait_start_end(&start_op, deadline) {
                    Ok(String::new())
                } else {
                    warn!("meetings: the cancelled start did not end in time");
                    Err(START_BUSY.to_string())
                };
            }
        };
        let SessionParts {
            meeting_id,
            mode,
            mic_capture,
            loopback,
            mic_sink,
            system_sink,
            mic_path,
            system_path,
            pipeline,
            write_flags,
            watch_stop,
            live,
        } = session.into_parts();
        watch_stop.store(true, Ordering::Release);

        // Captures first: once they are stopped no callback can touch the
        // sinks or the DSP queue any more, so draining and finalizing below is
        // race-free.
        let capture_end = self.stop_captures(&meeting_id, mic_capture, loopback);
        // Mikrofon und Loopback sind zu: jetzt ist die Maschine nicht mehr in
        // Aufnahme. ZUERST der Zustand, DANACH der Indikator: `hide_recording_overlay`
        // fragt den Zustand und stellte den Indikator sonst wieder auf. Der Abschluss
        // (Pipeline leeren, Writer schliessen) laeuft weiter und haelt `Finalizing` bis zum
        // Ende dieser Funktion (alle Rueckgabewege): ein Loeschen dieser Besprechung wartet
        // darauf, statt Dateien unter einem offenen Writer zu loeschen (lva#538).
        let _finalizing = stopping.finish();
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

        let mic_ms = mic_sink
            .as_ref()
            .and_then(|sink| finalize_sink(sink, write_flags.flag(CHANNEL_MIC)))
            .unwrap_or(0);
        let system_ms = system_sink
            .as_ref()
            .and_then(|sink| finalize_sink(sink, write_flags.flag(CHANNEL_SYSTEM)))
            .unwrap_or(0);
        let duration_ms = mic_ms.max(system_ms);

        // lva#538: ein Schreibfehler (auch ein Fehler beim Kopf-Flush oder beim Abschluss) und
        // jeder nicht saubere Abschluss des Loopbacks (Runde 5: Verlust, Geraetefehler, Panik,
        // unsichere Zeitachse) machen die Aufnahme unvollstaendig.
        let (failed, plan) = settle_after_stop(
            &self.store,
            &meeting_id,
            write_flags.failed_channels(),
            &capture_end,
            plan,
            MARKER_TRIES,
        );
        if !failed.is_empty() && write_flags.first_report() {
            self.emit_error(&meeting_id, AUDIO_WRITE_FAILED);
        }

        // #537: ein Mitschnitt ohne einen einzigen Ton (das Video stumm, der Treiber
        // lieferte nichts) laesst keine leere Aufnahme zurueck, die einen neuen Mitschnitt
        // fuer immer verhindert: der Eintrag ist wieder wie vorher (nur ohne Saetze, das
        // prueft der Rueckfall selbst).
        if mode == CaptureMode::PlaybackOnly
            && duration_ms == 0
            && failed.is_empty()
            && self.undo_empty_capture(&meeting_id, system_path.as_deref())
        {
            self.transcription
                .initiate_model_load_target(&settings.selected_model);
            self.emit_state(&meeting_id, "ready", false);
            return Ok(meeting_id);
        }
        if mode == CaptureMode::PlaybackOnly && duration_ms > 0 {
            let _ = self.store.discard_playback_prior(&meeting_id);
        }

        if let Err(e) = self.store.set_audio_paths(
            &meeting_id,
            mic_path.as_ref().and_then(|p| p.to_str()),
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

    /// Beendet die Captures einer Aufnahme (Mikrofon, Loopback) und meldet, was sie
    /// unterwegs zu sagen hatten. Danach ruft kein Callback mehr in Sinks oder DSP-Queue.
    /// Rueckgabe: wie der Loopback endete (lva#538 Runde 5): jeder nicht saubere Abschluss macht
    /// den Systemkanal unvollstaendig.
    fn stop_captures(
        &self,
        meeting_id: &str,
        mic_capture: Option<MeetingMicCapture>,
        loopback: Option<LoopbackCapture>,
    ) -> CaptureEnd {
        if let Some(capture) = mic_capture {
            if capture.had_error() {
                self.emit_error(meeting_id, "mic_stream_error");
            }
            capture.stop();
        }
        loopback
            .map(|capture| stop_loopback(meeting_id, capture))
            .unwrap_or_default()
    }

    /// Der Mitschnitt hat keinen einzigen Ton aufgenommen: Eintrag wie vorher, leere
    /// Capture-Dateien weg. `true`, wenn zurueckgesetzt.
    fn undo_empty_capture(&self, meeting_id: &str, system_path: Option<&std::path::Path>) -> bool {
        match self.store.restore_playback_target(meeting_id) {
            Ok(true) => {
                if let Some(dir) = system_path.and_then(std::path::Path::parent) {
                    remove_capture_files(dir);
                }
                info!(
                    "meetings: playback capture without any audio ({meeting_id}) - entry restored"
                );
                true
            }
            Ok(false) => false,
            Err(e) => {
                warn!("meetings: empty playback capture not reset ({meeting_id}): {e}");
                false
            }
        }
    }

    /// lva#538: VOR dem Loeschen der Besprechung `meeting_id` (`meetings_delete`). Reserviert das
    /// Loeschen und bringt die Besprechung in einen Zustand, in dem ihre Dateien weg duerfen:
    /// ein laufender Start in sie endet mit `meeting_not_found` und hat aufgeraeumt, eine
    /// laufende Aufnahme wird beendet (ohne Enddurchlauf), ein laufender Stopp samt Abschluss
    /// (Pipeline leeren, Writer schliessen) wird abgewartet. Die Reservierung haelt bis der
    /// Aufrufer sie fallen laesst, also NACH dem Tombstone und dem Entfernen der Dateien: bis
    /// dahin nimmt kein Start mehr in diese Besprechung auf.
    ///
    /// Fehler: `delete_in_progress` (schon ein Loeschen), `recording_busy` (das Warten lief ab:
    /// nichts wurde geloescht, ein neuer Versuch ist moeglich) - nie ein stiller Erfolg.
    /// Blockierend.
    pub fn reserve_delete(self: &Arc<Self>, meeting_id: &str) -> Result<DeleteReservation, String> {
        self.core.reserve_delete(meeting_id)?;
        // Faellt die Reservierung (auch bei jedem Fehler unten), gibt sie sich frei.
        let reservation = DeleteReservation {
            recorder: Arc::clone(self),
            meeting_id: meeting_id.to_string(),
        };
        let deadline = Instant::now() + DELETE_WAIT;
        loop {
            match self.core.delete_step(meeting_id, deadline)? {
                DeleteStep::Free => return Ok(reservation),
                DeleteStep::Running(session, ticket) => self.discard_running(session, ticket),
            }
        }
    }

    /// Beendet eine laufende Aufnahme, deren Besprechung geloescht wird: Captures zu, DSP und
    /// Worker beenden und JOINEN (der Rest der Warteschlange wird abgeworfen, nichts mehr
    /// gespeichert), WAV-Writer zu, kein Status, kein Enddurchlauf. Die Dateien loescht der
    /// Aufrufer danach; `Finalizing` haelt das Loeschen bis hierher zurueck.
    fn discard_running(
        &self,
        session: RecordingSession,
        stopping: StopTicket<'_, RecordingSession>,
    ) {
        let SessionParts {
            meeting_id,
            mic_capture,
            loopback,
            mic_sink,
            system_sink,
            pipeline,
            write_flags,
            watch_stop,
            // Die Anmeldung faellt erst am Ende: bis dahin laeuft kein Enddurchlauf mit Modell.
            live: _live,
            ..
        } = session.into_parts();
        watch_stop.store(true, Ordering::Release);
        self.stop_captures(&meeting_id, mic_capture, loopback);
        let _finalizing = stopping.finish();
        self.set_indicator(false);
        if let Some(pipeline) = pipeline {
            pipeline.abort();
        }
        if let Some(sink) = &mic_sink {
            finalize_sink(sink, write_flags.flag(CHANNEL_MIC));
        }
        if let Some(sink) = &system_sink {
            finalize_sink(sink, write_flags.flag(CHANNEL_SYSTEM));
        }
        let selected = crate::settings::get_settings(&self.app).selected_model;
        self.transcription.initiate_model_load_target(&selected);
        self.emit_state(&meeting_id, "cancelled", false);
        info!("meetings: recording ended because its meeting is deleted ({meeting_id})");
    }

    /// lva#538: der Waechter der WAV-Writer. Ein Schreibfehler (Platte voll, RIFF-Grenze,
    /// Kopf-Flush) setzt im Callback nur ein Merkzeichen; hier wird daraus, noch waehrend der
    /// Aufnahme, der dauerhafte Marker `audio_incomplete` (ein Absturz danach verliert ihn
    /// nicht) und eine Meldung an den Nutzer. Laesst sich der Marker nicht speichern, versucht
    /// der Waechter es im Takt weiter (die Meldung geht trotzdem sofort raus). Endet mit der
    /// Aufnahme (`stop`) oder, wenn Marker und Meldung stehen.
    fn watch_writes(&self, meeting_id: &str, flags: &Arc<WriteFlags>, stop: &Arc<AtomicBool>) {
        let store = Arc::clone(&self.store);
        let app = self.app.clone();
        let flags = Arc::clone(flags);
        let stop = Arc::clone(stop);
        let meeting_id = meeting_id.to_string();
        let _ = std::thread::Builder::new()
            .name("meeting-write-watch".to_string())
            .spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let failed = flags.failed_channels();
                    if !failed.is_empty() {
                        warn!(
                            "meetings: the recording could not be written any more ({meeting_id})"
                        );
                        if flags.first_report() {
                            let _ = (MeetingEvent::Error {
                                meeting_id: meeting_id.clone(),
                                message: AUDIO_WRITE_FAILED.to_string(),
                            })
                            .emit(&app);
                        }
                        if persist_audio_incomplete(&store, &meeting_id, &failed, 1) {
                            return;
                        }
                    }
                    std::thread::sleep(WRITE_WATCH_INTERVAL);
                }
            });
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
        let data_dir = super::meetings_data_dir(&self.app).ok();
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
                // lva#538: ein Mitschnitt, der nie etwas aufgenommen hat (Absturz im Start),
                // wird wieder die YouTube-Besprechung von vorher, statt als leere
                // Aufnahme stehenzubleiben. Auch eine `failed`-Zeile mit dem Merker.
                if reset_unrecorded_capture(&self.store, &meeting, data_dir.as_deref()) {
                    info!(
                        "meetings: unrecorded playback capture reset ({})",
                        meeting.id
                    );
                    continue;
                }
                if !is_orphan_status(&meeting.status) {
                    continue;
                }
                // lva#523: WAV-Header, Dauer und `processing` erst unter dem Auftrag
                // (`run_job`, `recover_from`): eine Ablehnung aendert nichts.
                let plan = if runs_final_pass(&meeting) {
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
        let plan = if runs_final_pass(&meeting) {
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

    /// lva#538 (Runde 3): ein abgebrochener oder gescheiterter Start hat die Zeile zurueckgesetzt
    /// (Eintrag wieder wie vorher, Saetze verworfen). Die Oberflaeche hat die Saetze, die die
    /// Pipeline VOR dem Abbruch schon gemeldet hatte, als Ereignisse uebernommen: `Reset` leert
    /// ihre Liste, der Zustand danach (aus der Zeile gelesen) laesst sie Segmente und Eintrag neu
    /// laden. Ohne das blieben Phantomsaetze stehen und der erneute Mitschnitt verschwaende.
    fn emit_restored(&self, meeting_id: &str) {
        let status = self
            .store
            .get_meeting(meeting_id)
            .ok()
            .flatten()
            .map(|m| m.status)
            .unwrap_or_else(|| "ready".to_string());
        let _ = (MeetingEvent::Reset {
            meeting_id: meeting_id.to_string(),
        })
        .emit(&self.app);
        self.emit_state(meeting_id, &status, false);
    }

    // M1-P1c (Beruehrpunkt B2): Audioposition fuer den Notizblock.
    /// The running meeting's id and the microphone audio position in
    /// milliseconds, or `None` when nothing is being recorded. Same timeline
    /// as `StoredSegment.start_ms` (paused stretches are compressed). Beim
    /// Mitschnitt des Systemtons (#537) ohne Mikrofon zaehlt die Systemton-Datei.
    pub fn position_ms(&self) -> Option<(String, u64)> {
        self.core
            .with_session(|session| session.and_then(Self::session_position))
    }

    /// `None` once the mic WAV writer is gone (RIFF limit reached): a wrong
    /// position would be worse than none, the notepad then stores no timestamp.
    fn session_position(session: &RecordingSession) -> Option<(String, u64)> {
        let sink = session.mic_sink.as_ref().or(session.system_sink.as_ref())?;
        let sink = sink.lock().ok()?;
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
        RecordingSession::new(
            "m-pos",
            CaptureMode::Meeting {
                capture_system: false,
            },
            Some(Arc::new(Mutex::new(sink))),
            None,
            None,
            None,
            FinalSlot::default().live_beginnen().unwrap(),
        )
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
            .as_ref()
            .unwrap()
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

    /// #537: ohne Mikrofon (Mitschnitt des Systemtons) zaehlt die Systemton-Datei:
    /// der Notizblock bekommt die Position, an der das Transkript wirklich steht.
    #[test]
    fn position_follows_the_system_writer_when_there_is_no_microphone() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = StreamingWavWriter::create(&dir.path().join("system.wav"), 16_000).unwrap();
        w.append(&vec![0i16; 32_000]).unwrap(); // 2 s
        let mut session = session_with(dir.path(), None);
        session.mic_sink = None;
        session.system_sink = Some(Arc::new(Mutex::new(ChannelSink::new(w))));
        assert_eq!(
            MeetingRecorderManager::session_position(&session),
            Some(("m-pos".to_string(), 2_000))
        );
        // Kein Writer mehr (Platte voll, RIFF-Grenze): lieber keine als eine falsche Zeit.
        session.system_sink.as_ref().unwrap().lock().unwrap().writer = None;
        assert_eq!(MeetingRecorderManager::session_position(&session), None);
        // Gar keine Spur: ebenfalls keine Position.
        session.system_sink = None;
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

/// Die Meldung zu einem Fehler beim Speichern der Audiopfade: eine geloeschte Zeile
/// heisst `meeting_not_found`, alles andere ein Fehler des Starts.
fn audio_paths_error(error: &anyhow::Error) -> String {
    let text = error.to_string();
    if text.ends_with("not found") {
        MEETING_NOT_FOUND.to_string()
    } else {
        format!("audio_paths_failed: {text}")
    }
}

/// The per-channel capture callback: WAV append (with 1-s header flush), hand-off
/// to the DSP thread, and the throttled level readout. The hand-off is a
/// non-blocking `try_send` (see `dsp::ChannelFeed`); segmentation and VAD never
/// run here. The second argument is the QPC stamp of the block's first sample
/// (P2c2), passed through untouched.
///
/// lva#538: der GANZE Callback laeuft unter der Sperre seines Sinks (WAV, DSP-Queue, Pegel,
/// Pause). `closed` (`close_sink`, `finalize_sink`) wird unter derselben Sperre gesetzt: ein
/// Callback, der gerade laeuft, ist fertig, wenn das Schliessen zurueckkehrt, und jeder spaetere
/// ruft nichts mehr an, auch wenn der Treiber nach einer Frist oder einem Abbruch doch noch
/// zurueckkehrt. Ein Frist-Merkzeichen nur fuer das WAV (wie vorher) liess DSP und Pegel
/// durch. Ein Schreibfehler der WAV, auch beim Kopf-Flush, setzt das Merkzeichen in
/// `write_flags` (Atomic) und verwirft den Writer; Aufnahme und Transkription laufen weiter,
/// der Fehler wird gemeldet und dauerhaft markiert, nicht verschluckt.
fn channel_callback(
    channel: u8,
    sink: Arc<Mutex<ChannelSink>>,
    paused: Arc<AtomicBool>,
    write_flags: Arc<WriteFlags>,
    mut feed: ChannelFeed,
    levels: Arc<dyn LevelSink>,
) -> impl FnMut(&[i16], Option<u64>) + Send + 'static {
    move |samples: &[i16], qpc: Option<u64>| {
        let mut guard = lock_recovering(&sink);
        let sink = &mut *guard;
        if sink.closed {
            return;
        }
        if paused.load(Ordering::Relaxed) {
            feed.pause();
            return;
        }
        let mut writer_failed = false;
        if let Some(writer) = sink.writer.as_mut() {
            match writer.append(samples) {
                // Also the RIFF 4-GB limit: stop writing this file rather
                // than corrupting it; capture and transcription continue.
                Err(e) => {
                    warn!("meetings: wav append stopped on channel {channel}: {e}");
                    writer_failed = true;
                }
                Ok(()) => {
                    sink.samples_since_flush += samples.len();
                    if sink.samples_since_flush >= FLUSH_EVERY_SAMPLES {
                        sink.samples_since_flush = 0;
                        // Ein Fehler des Kopf-Flush ist ein Schreibfehler: die Datei laesst sich
                        // nach einem Absturz nicht mehr reparieren, und der Writer ist danach
                        // unbrauchbar (Position unbekannt).
                        if let Err(e) = writer.flush_header() {
                            warn!("meetings: wav header flush failed on channel {channel}: {e}");
                            writer_failed = true;
                        }
                    }
                }
            }
        }
        if writer_failed {
            sink.writer = None;
            write_flags.flag(channel).store(true, Ordering::Release);
        }
        feed.push_stamped(samples, qpc);
        levels.level(channel, rms(samples));
    }
}

/// Die Callback-Sperre fuer einen Sink, dessen Datei nicht mehr gebraucht wird (Abbruch des
/// Starts, Loeschen): ab hier ruft kein Callback mehr etwas an, ein laufender ist fertig. Der
/// Writer wird dabei abgeschlossen, damit eine Datei, die ein Fehlerweg behaelt (Fail-closed),
/// ein gueltiger Kopf hat und nicht auf "0 Laenge" steht.
fn close_sink(sink: Option<&Arc<Mutex<ChannelSink>>>) {
    if let Some(sink) = sink {
        let unused = AtomicBool::new(false);
        let _ = finalize_sink(sink, &unused);
    }
}

/// Schliesst einen Kanal fuer immer (Callback-Sperre) und liefert seine Dauer in
/// Millisekunden. Ein Fehler beim Abschluss setzt dasselbe Merkzeichen wie ein Schreibfehler
/// (die Aufnahme ist dann nicht vollstaendig).
fn finalize_sink(sink: &Arc<Mutex<ChannelSink>>, failed: &AtomicBool) -> Option<u64> {
    let writer = {
        let mut sink = lock_recovering(sink);
        sink.closed = true;
        sink.writer.take()?
    };
    match writer.finalize() {
        Ok(ms) => Some(ms),
        Err(e) => {
            warn!("meetings: wav finalize failed: {e}");
            failed.store(true, Ordering::Release);
            None
        }
    }
}

/// lva#538: macht aus dem Merkzeichen einen dauerhaften Marker (`metadata_json.
/// audio_incomplete`). Ab dann ersetzt der Enddurchlauf das Live-Transkript nie mehr aus
/// dieser Aufnahme, auch nach einem Absturz oder wenn spaeter wieder Platz da ist.
/// Mehrfaches Schreiben ist unschaedlich (derselbe Schluessel). Ein Fehler wird gemeldet,
/// nicht nur geloggt: der Aufrufer weicht darauf fail-closed aus.
#[cfg(test)]
pub(crate) fn mark_audio_incomplete(
    store: &MeetingStore,
    meeting_id: &str,
    channels: &[u8],
) -> Result<(), String> {
    mark_audio_incomplete_because(store, meeting_id, channels, "write_failed")
}

/// Wie [`mark_audio_incomplete`], mit dem Grund (`reason`: `write_failed`, `capture_lost`,
/// `capture_error`, `capture_panicked`, `timeline_uncertain`, kommagetrennt bei mehreren).
pub(crate) fn mark_audio_incomplete_because(
    store: &MeetingStore,
    meeting_id: &str,
    channels: &[u8],
    reason: &str,
) -> Result<(), String> {
    let value = serde_json::json!({
        "channels": channels,
        "reason": reason,
        "at": chrono::Utc::now().timestamp(),
    });
    store
        .set_metadata_key(meeting_id, final_pass::AUDIO_INCOMPLETE_KEY, value)
        .map_err(|e| {
            error!("meetings: audio_incomplete marker not stored ({meeting_id}): {e}");
            e.to_string()
        })
}

/// Speichert den Marker mit bis zu `tries` Versuchen; `true`, wenn er steht.
fn persist_audio_incomplete(
    store: &MeetingStore,
    meeting_id: &str,
    channels: &[u8],
    tries: u32,
) -> bool {
    persist_audio_incomplete_because(store, meeting_id, channels, "write_failed", tries)
}

fn persist_audio_incomplete_because(
    store: &MeetingStore,
    meeting_id: &str,
    channels: &[u8],
    reason: &str,
    tries: u32,
) -> bool {
    for attempt in 0..tries.max(1) {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
        if mark_audio_incomplete_because(store, meeting_id, channels, reason).is_ok() {
            return true;
        }
    }
    false
}

/// lva#538 (Runde 3): laesst den Marker `capture_open` fallen, mit bis zu `tries` Versuchen.
/// `true`, wenn er nicht (mehr) steht. Scheitert es, bleibt der Marker (fail-closed): das Audio
/// gilt weiter als unvollstaendig und der Enddurchlauf ersetzt das Live-Transkript nicht.
pub(crate) fn close_capture_marker(store: &MeetingStore, meeting_id: &str, tries: u32) -> bool {
    for attempt in 0..tries.max(1) {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
        match store.end_capture(meeting_id) {
            Ok(_) => return true,
            Err(e) => warn!("meetings: capture marker of {meeting_id} not cleared: {e}"),
        }
    }
    false
}

/// lva#538 (Runde 5): beendet den Loopback und wertet aus, wie er endete. Die Auswertung ist
/// `LoopbackCapture::stop`: Verlust beim Abschluss, Gerätefehler, Panik des Threads (das `join`
/// schlug fehl) und unsichere Zeitachse. Nichts davon wird nur protokolliert.
pub(crate) fn stop_loopback(meeting_id: &str, capture: LoopbackCapture) -> CaptureEnd {
    // The watchdog above has already reported an error while the meeting was running; the log
    // line here is what makes it visible in the evidence of a finished meeting.
    if capture.had_error() {
        warn!("meetings: loopback capture had ended with an error ({meeting_id})");
    }
    let end = capture.stop();
    if !end.is_clean() {
        warn!(
            "meetings: loopback capture did not end cleanly ({meeting_id}): {}",
            end.reasons().join(",")
        );
    }
    end
}

/// lva#538: das Ende des Stopps am Marker. Unvollstaendig ist ein Kanal, dessen WAV nicht
/// vollstaendig geschrieben wurde (`write_failed`), und der Systemkanal bei JEDEM nicht sauberen
/// Abschluss des Loopbacks (`end`, Runde 5). Der Marker `audio_incomplete` steht VOR dem Auftrag,
/// mit Grund (`reason`), damit der Enddurchlauf das Live-Transkript nicht aus der WAV ersetzt
/// (fail-closed: laesst er sich nicht speichern, gilt fuer DIESEN Lauf trotzdem
/// `Keep(AudioIncomplete)`). Nur eine sauber geschlossene Aufnahme ohne jeden Verlust laesst den
/// pessimistischen Marker `capture_open` fallen; bei einem Fehler bleibt er (zusaetzlich zu
/// `audio_incomplete`), laesst er sich nicht entfernen, bleibt er ebenfalls: das ist die sichere
/// Seite. Rueckgabe: die unvollstaendigen Kanaele und der Plan.
pub(crate) fn settle_after_stop(
    store: &MeetingStore,
    meeting_id: &str,
    write_failed: Vec<u8>,
    end: &CaptureEnd,
    plan: FinalPlan,
    tries: u32,
) -> (Vec<u8>, FinalPlan) {
    let mut failed = write_failed;
    let mut reasons: Vec<&str> = Vec::new();
    if !failed.is_empty() {
        reasons.push("write_failed");
    }
    reasons.extend(end.reasons());
    if !end.is_clean() && !failed.contains(&CHANNEL_SYSTEM) {
        failed.push(CHANNEL_SYSTEM);
    }
    let plan = plan_after_incomplete(store, meeting_id, &failed, &reasons.join(","), plan, tries);
    if failed.is_empty() {
        close_capture_marker(store, meeting_id, tries);
    }
    (failed, plan)
}

/// lva#538, fail-closed: der Plan des Enddurchlaufs nach den Schreibfehlern (`failed` = die
/// Kanaele, deren WAV nicht vollstaendig geschrieben wurde). Ohne Fehler bleibt der Plan. Mit
/// Fehler steht der Marker VOR dem Auftrag; laesst er sich nicht speichern, gilt fuer diesen
/// Lauf trotzdem `Keep(AudioIncomplete)`: das Live-Transkript bleibt, auch wenn die Datenbank
/// den Marker verweigert. (Ein spaeterer Lauf ohne Marker faengt `final_pass` ueber die
/// Abdeckung der Spuren ab.)
#[cfg(test)]
pub(crate) fn plan_after_write_failures(
    store: &MeetingStore,
    meeting_id: &str,
    failed: &[u8],
    plan: FinalPlan,
    tries: u32,
) -> FinalPlan {
    plan_after_incomplete(store, meeting_id, failed, "write_failed", plan, tries)
}

/// Wie [`plan_after_write_failures`], mit dem Grund, der im Marker steht.
pub(crate) fn plan_after_incomplete(
    store: &MeetingStore,
    meeting_id: &str,
    failed: &[u8],
    reason: &str,
    plan: FinalPlan,
    tries: u32,
) -> FinalPlan {
    if failed.is_empty()
        || persist_audio_incomplete_because(store, meeting_id, failed, reason, tries)
    {
        return plan;
    }
    FinalPlan::Keep(KeepReason::AudioIncomplete)
}

/// lva#538: nach einem Absturz oder Abbruch des Mitschnitts. Ein YouTube-Eintrag, der
/// nie etwas aufgenommen hat (Start bis `recording` gekommen, WAV leer oder fehlt, keine
/// Saetze), wird wieder die YouTube-Besprechung von vorher: Status, Einwilligung und
/// Dauer aus dem gespeicherten Vorzustand, keine Audiopfade, die leeren Capture-Dateien
/// weg. `true`, wenn zurueckgesetzt (dann gibt es nichts wiederherzustellen). Mit Ton
/// (auch ohne erkannte Sprache) bleibt es bei der normalen Wiederherstellung; der Merker
/// des Vorzustands wird dann nicht mehr gebraucht.
pub(crate) fn reset_unrecorded_capture(
    store: &MeetingStore,
    meeting: &Meeting,
    data_dir: Option<&std::path::Path>,
) -> bool {
    reset_unrecorded_capture_with(store, meeting, data_dir, super::playback::audio_state)
}

/// Wie [`reset_unrecorded_capture`], mit dem Dateizustand als Parameter (Test: ein Zugriffsfehler
/// laesst sich einspielen, ohne die Platte zu bemuehen).
fn reset_unrecorded_capture_with(
    store: &MeetingStore,
    meeting: &Meeting,
    data_dir: Option<&std::path::Path>,
    audio_state: impl Fn(&Meeting) -> super::playback::AudioState,
) -> bool {
    let reachable = is_orphan_status(&meeting.status) || meeting.status == "failed";
    if meeting.source != super::playback::SOURCE_YOUTUBE || !reachable {
        return false;
    }
    let has_prior = store.playback_prior(&meeting.id).ok().flatten().is_some();
    let state = audio_state(meeting);
    if !(has_prior && state == super::playback::AudioState::Empty) {
        // Nur NACHGEWIESENER Ton erledigt den Vorzustand. Laesst sich der Dateizustand nicht
        // lesen, bleibt der Merker (und alles andere) stehen: im Zweifel behalten.
        if has_prior
            && is_orphan_status(&meeting.status)
            && state == super::playback::AudioState::Present
        {
            let _ = store.discard_playback_prior(&meeting.id);
        }
        return false;
    }
    match store.restore_playback_target(&meeting.id) {
        Ok(true) => {
            let mut dirs: Vec<PathBuf> =
                data_dir.map(|d| d.join(&meeting.id)).into_iter().collect();
            dirs.extend(
                [&meeting.mic_audio_path, &meeting.system_audio_path]
                    .into_iter()
                    .flatten()
                    .filter_map(|p| std::path::Path::new(p).parent().map(PathBuf::from)),
            );
            dirs.dedup();
            for dir in dirs {
                remove_capture_files(&dir);
            }
            true
        }
        Ok(false) => false,
        Err(e) => {
            warn!("meetings: unrecorded capture {} not reset: {e}", meeting.id);
            false
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

    /// lva#538, fail-closed: hat die Pipeline vor dem Fehler schon Saetze gespeichert, bleiben
    /// Transkript UND Audio stehen (nur der Status ist `failed`). Geloescht wird nur, was
    /// nachweislich leer ist.
    #[test]
    fn a_failed_start_keeps_the_audio_when_the_transcript_already_has_segments() {
        let (_dir, store, id, folder) = failed_start_fixture();
        store
            .append_delta(
                &id,
                &crate::managers::meetings::store::TranscriptDelta {
                    new_segments: vec![StoredSegment {
                        segment_index: 0,
                        start_ms: 0,
                        end_ms: 1_000,
                        text: "Schon gesagt".to_string(),
                        channel: 0,
                        speaker_index: None,
                        words: None,
                    }],
                },
            )
            .unwrap();

        discard_failed_start(&store, &id, &folder);

        assert!(folder.join("mic.wav").exists(), "das Audio blieb");
        assert!(folder.join("system.wav").exists());
        let meeting = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(meeting.status, "failed");
        assert!(
            meeting.mic_audio_path.is_some() && meeting.system_audio_path.is_some(),
            "die Pfade bleiben, das Audio gehoert zum Transkript"
        );
        assert_eq!(store.get_segments(&id).unwrap().len(), 1);
    }

    // ---- #537: Mitschnitt des Systemtons -----------------------------------------

    #[test]
    fn a_playback_capture_has_a_system_channel_and_no_microphone() {
        let meeting = CaptureMode::Meeting {
            capture_system: true,
        };
        let mic_only = CaptureMode::Meeting {
            capture_system: false,
        };
        assert!(meeting.mic() && meeting.system());
        assert!(mic_only.mic() && !mic_only.system());
        let playback = CaptureMode::PlaybackOnly;
        assert!(!playback.mic(), "kein Mikrofon, kein mic.wav, kein Echo");
        assert!(playback.system());
    }

    /// Der Mitschnitt scheitert laut, wenn der Systemton nicht startet (er ist die
    /// einzige Quelle); die Besprechung sagt weiterhin "nur Mikrofon".
    #[test]
    fn a_loopback_that_does_not_start_has_its_own_code_per_mode() {
        let failed = LoopbackStartError::Failed("kein Geraet".into());
        let timeout = LoopbackStartError::Timeout;
        let playback = CaptureMode::PlaybackOnly;
        assert_eq!(
            playback.loopback_start_code(&failed),
            "playback_capture_failed"
        );
        assert_eq!(
            playback.loopback_start_code(&timeout),
            "playback_capture_timeout"
        );
        let meeting = CaptureMode::Meeting {
            capture_system: true,
        };
        assert_eq!(
            meeting.loopback_start_code(&failed),
            "loopback_start_failed"
        );
        assert_eq!(
            meeting.loopback_start_code(&timeout),
            "loopback_start_timeout"
        );
    }

    /// Stirbt der Systemton mitten im Mitschnitt, ist der Mikrofon-Text der
    /// Besprechung ("laeuft nur mit dem Mikrofon weiter") falsch: es kommt nur der
    /// eigene Fehler, kein Kanalzustand `loopback_died`.
    #[test]
    fn a_dead_loopback_is_reported_with_the_text_of_its_mode() {
        let json = |events: Vec<MeetingEvent>| {
            events
                .iter()
                .map(|e| serde_json::to_value(e).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            json(CaptureMode::PlaybackOnly.loopback_died_events("m1")),
            vec![serde_json::json!({
                "kind": "error", "meeting_id": "m1", "message": "playback_capture_died"
            })]
        );
        let meeting = CaptureMode::Meeting {
            capture_system: true,
        };
        assert_eq!(
            json(meeting.loopback_died_events("m1")),
            vec![
                serde_json::json!({
                    "kind": "health", "meeting_id": "m1", "channel": 1, "state": "loopback_died"
                }),
                serde_json::json!({
                    "kind": "error", "meeting_id": "m1", "message": "loopback_died"
                }),
            ]
        );
    }

    use std::sync::atomic::AtomicUsize;

    /// Frischer Abbruchmerker eines Starts.
    fn cancel_flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    /// Ein `abandon` fuer `start_bounded`, das mitzaehlt, wie oft es lief.
    fn abandon_counter() -> (Arc<AtomicUsize>, impl FnOnce()) {
        let count = Arc::new(AtomicUsize::new(0));
        let hit = Arc::clone(&count);
        (count, move || {
            hit.fetch_add(1, Ordering::SeqCst);
        })
    }

    #[test]
    fn a_bounded_start_returns_the_started_capture() {
        let cancel = cancel_flag();
        let (abandoned, abandon) = abandon_counter();
        let got = start_bounded("t-ok", Duration::from_secs(5), &cancel, abandon, || {
            Ok::<_, String>(42u8)
        });
        assert_eq!(got, Ok(42));
        assert_eq!(
            abandoned.load(Ordering::SeqCst),
            0,
            "ein Erfolg gibt den Kanal nicht auf"
        );
    }

    /// Kein Ausgabegeraet (oder es laesst sich nicht oeffnen): der Fehler des
    /// Treibers kommt an, nicht nach einer Frist, sondern sofort.
    #[test]
    fn a_failing_start_reports_its_error_at_once() {
        let cancel = cancel_flag();
        let started = Instant::now();
        let got = start_bounded(
            "t-err",
            Duration::from_secs(30),
            &cancel,
            || {},
            || Err::<u8, _>("kein Wiedergabegeraet".to_string()),
        );
        assert_eq!(
            got,
            Err(LoopbackStartError::Failed("kein Wiedergabegeraet".into()))
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// Haengt der Treiber, endet das Warten nach der Frist, und eine Capture, die
    /// spaeter doch noch aufgeht, wird verworfen (ihr `Drop` stoppt sie): sie laeuft
    /// nie unbemerkt weiter. Der Kanal ist ab der Frist gesperrt (lva#538): `abandon` lief,
    /// BEVOR das Warten zurueckkehrte.
    #[test]
    fn a_wedged_start_times_out_abandons_the_sink_and_drops_the_late_capture() {
        struct Capture(mpsc::Sender<()>);
        impl Drop for Capture {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let cancel = cancel_flag();
        let (abandoned, abandon) = abandon_counter();
        let (dropped_tx, dropped_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let got = start_bounded(
            "t-wedged",
            Duration::from_millis(50),
            &cancel,
            abandon,
            move || {
                // Der Treiber haengt, bis der Test ihn freigibt: die Frist laeuft sicher ab.
                let _ = release_rx.recv();
                Ok::<_, String>(Capture(dropped_tx))
            },
        );
        assert!(matches!(got, Err(LoopbackStartError::Timeout)));
        assert_eq!(
            abandoned.load(Ordering::SeqCst),
            1,
            "nach der Frist darf der Callback nichts mehr anrufen"
        );
        release_tx.send(()).unwrap();
        dropped_rx
            .recv_timeout(Duration::from_secs(60))
            .expect("die spaete Capture lief weiter");
    }

    /// lva#538: ein Stopp waehrend des Wartens beendet es sofort (nicht erst nach der
    /// Frist von dreissig Sekunden) und sperrt den Kanal; die spaete Capture wird verworfen.
    /// Der Abbruch kommt erst, wenn der Start-Thread nachweislich im Treiber steht.
    #[test]
    fn a_cancel_ends_the_wait_at_once_and_abandons_the_sink() {
        let cancel = cancel_flag();
        let (abandoned, abandon) = abandon_counter();
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let canceller = Arc::clone(&cancel);
        let cancel_thread = std::thread::spawn(move || {
            entered_rx.recv().unwrap();
            canceller.store(true, Ordering::SeqCst);
        });
        let got = start_bounded(
            "t-cancel",
            Duration::from_secs(300),
            &cancel,
            abandon,
            move || {
                entered_tx.send(()).unwrap();
                let _ = release_rx.recv();
                Ok::<_, String>(1u8)
            },
        );
        // Ohne den Abbruch laege hier erst nach 300 s `Timeout`: der Test belegt die Reihenfolge,
        // nicht die Zeit.
        assert_eq!(got, Err(LoopbackStartError::Cancelled));
        assert_eq!(abandoned.load(Ordering::SeqCst), 1);
        cancel_thread.join().unwrap();
        release_tx.send(()).unwrap();
    }

    /// Schon abgebrochen, bevor das Warten beginnt: es startet nicht einmal den Treiber.
    #[test]
    fn a_start_that_was_cancelled_before_it_began_never_touches_the_driver() {
        let cancel = cancel_flag();
        cancel.store(true, Ordering::SeqCst);
        let (abandoned, abandon) = abandon_counter();
        let touched = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&touched);
        // Der Wachposten meldet, wann der Start-Aufruf verbraucht oder verworfen ist.
        struct Sentinel(mpsc::Sender<()>);
        impl Drop for Sentinel {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (gone_tx, gone_rx) = mpsc::channel::<()>();
        let sentinel = Sentinel(gone_tx);
        let got = start_bounded(
            "t-early",
            Duration::from_secs(5),
            &cancel,
            abandon,
            move || {
                let _keep = &sentinel;
                flag.store(true, Ordering::SeqCst);
                Ok::<_, String>(())
            },
        );
        assert_eq!(got, Err(LoopbackStartError::Cancelled));
        gone_rx
            .recv_timeout(Duration::from_secs(60))
            .expect("der Start-Aufruf wurde nie verworfen");
        assert!(!touched.load(Ordering::SeqCst));
        assert_eq!(abandoned.load(Ordering::SeqCst), 1);
    }

    /// Ein Start, der GLEICHZEITIG mit dem Abbruch fertig wird, liefert keine Capture mehr aus:
    /// der Kanal ist gesperrt, bevor die Capture faellt, und der Start meldet den Abbruch.
    #[test]
    fn a_capture_that_arrives_together_with_the_cancel_is_abandoned_not_published() {
        let cancel = cancel_flag();
        let (abandoned, abandon) = abandon_counter();
        let racing = Arc::clone(&cancel);
        let got = start_bounded(
            "t-race",
            Duration::from_secs(5),
            &cancel,
            abandon,
            move || {
                racing.store(true, Ordering::SeqCst);
                Ok::<_, String>(7u8)
            },
        );
        assert_eq!(got, Err(LoopbackStartError::Cancelled));
        assert_eq!(abandoned.load(Ordering::SeqCst), 1);
    }

    /// Ein Treiber, der im Start abstuerzt, ist ein Fehler, kein Haenger.
    #[test]
    fn a_panicking_start_is_a_failure_not_a_hang() {
        let cancel = cancel_flag();
        let started = Instant::now();
        let got = start_bounded(
            "t-panic",
            Duration::from_secs(30),
            &cancel,
            || {},
            || -> Result<u8, String> { panic!("treiber") },
        );
        assert!(matches!(got, Err(LoopbackStartError::Failed(_))));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    // ---- lva#538: der echte Callback hinter der Sperre, Schreibfehler ------------------------

    /// lva#538 (Runde 3): ein Tor haelt einen Test-Thread an einer Stelle fest, bis der Test es
    /// oeffnet. `wait_entered` kehrt erst zurueck, wenn der Thread DA ist: keine Schlafzeit,
    /// kein Raten. Die Frist ist nur ein Netz gegen ein Haengen im Fehlerfall.
    #[derive(Default)]
    struct Gate {
        state: Mutex<(bool, bool)>,
        changed: std::sync::Condvar,
    }

    impl Gate {
        /// Vom festgehaltenen Thread: ich bin da, jetzt warte ich auf das Oeffnen.
        fn pass(&self) {
            let mut state = self.state.lock().unwrap();
            state.0 = true;
            self.changed.notify_all();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !state.1 {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "das Tor wurde nie geoeffnet");
                state = self.changed.wait_timeout(state, left).unwrap().0;
            }
        }

        /// Vom Test: wartet, bis ein Thread am Tor steht.
        fn wait_entered(&self) {
            let mut state = self.state.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !state.0 {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "niemand kam ans Tor");
                state = self.changed.wait_timeout(state, left).unwrap().0;
            }
        }

        fn open(&self) {
            self.state.lock().unwrap().1 = true;
            self.changed.notify_all();
        }
    }

    /// Wartet (ohne Schlafzeit, nur mit Yield) auf eine Bedingung, die ein anderer Thread
    /// herstellt; die Frist ist ein Netz gegen ein Haengen.
    fn until(what: &str, condition: impl Fn() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "nie eingetreten: {what}"
            );
            std::thread::yield_now();
        }
    }

    /// Der Haken fuer den Schliessversuch: meldet, dass er gleich die Sperre des Sinks nimmt.
    fn close_sink_hooked(sink: Option<&Arc<Mutex<ChannelSink>>>, on_attempt: impl FnOnce()) {
        on_attempt();
        close_sink(sink);
    }

    /// Pegel, die ein Test mitzaehlt statt ein Ereignis zu senden. Mit einem `gate` bleibt der
    /// Callback im Pegelpfad (also unter der Sperre seines Sinks) stehen, bis der Test es oeffnet.
    #[derive(Default)]
    struct CountingLevels {
        calls: AtomicUsize,
        finished: AtomicUsize,
        gate: Mutex<Option<Arc<Gate>>>,
    }

    impl LevelSink for CountingLevels {
        fn level(&self, _channel: u8, _rms: f32) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let gate = self.gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.pass();
            }
            self.finished.fetch_add(1, Ordering::SeqCst);
        }
    }

    type RigCallback = Box<dyn FnMut(&[i16], Option<u64>) + Send>;

    struct CallbackRig {
        _dir: tempfile::TempDir,
        path: PathBuf,
        sink: Arc<Mutex<ChannelSink>>,
        flags: Arc<WriteFlags>,
        levels: Arc<CountingLevels>,
        pipeline: LivePipeline,
        callback: RigCallback,
    }

    fn callback_rig() -> CallbackRig {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system.wav");
        let sink = Arc::new(Mutex::new(ChannelSink::new(
            StreamingWavWriter::create(&path, SAMPLE_RATE).unwrap(),
        )));
        let store = Arc::new(MeetingStore::open_at(&dir.path().join("m.db")).unwrap());
        let pipeline = LivePipeline::start(
            "m-rig".to_string(),
            store,
            DspConfig::new(None, Arc::new(|_| {})).playback_only(),
            |_chunk: &Chunk| Vec::new(),
            |_event| {},
        )
        .unwrap();
        let flags = Arc::new(WriteFlags::default());
        let levels = Arc::new(CountingLevels::default());
        let callback = channel_callback(
            CHANNEL_SYSTEM,
            Arc::clone(&sink),
            Arc::new(AtomicBool::new(false)),
            Arc::clone(&flags),
            pipeline.feed(CHANNEL_SYSTEM).unwrap(),
            Arc::clone(&levels) as Arc<dyn LevelSink>,
        );
        CallbackRig {
            _dir: dir,
            path,
            sink,
            flags,
            levels,
            pipeline,
            callback: Box::new(callback),
        }
    }

    fn written_samples(rig: &CallbackRig) -> Option<u64> {
        lock_recovering(&rig.sink)
            .writer
            .as_ref()
            .map(StreamingWavWriter::frames_written)
    }

    /// lva#538, "verspaetete echte Callback-Aktivitaet nach Timeout": der Treiber haengt
    /// ueber die Frist, kehrt dann zurueck und beginnt, den ECHTEN Callback zu rufen. Der
    /// Start hat laengst gemeldet, dass er gescheitert ist; es darf nichts mehr geschrieben
    /// werden (weder in die WAV noch an den DSP noch als Pegel), und der Start kann die
    /// Datei danach schliessen und entfernen.
    #[test]
    fn a_late_callback_after_the_timeout_writes_nothing() {
        let mut rig = callback_rig();
        let cancel = cancel_flag();
        let mut callback = std::mem::replace(&mut rig.callback, Box::new(|_, _| {}));
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let barrier_sink = Arc::clone(&rig.sink);
        let got = start_bounded(
            "t-late-callback",
            Duration::from_millis(50),
            &cancel,
            move || close_sink(Some(&barrier_sink)),
            move || {
                // Der Treiber haengt, bis der Test ihn freigibt; dann liefert er, wie ein
                // laufender Strom, Bloecke.
                let _ = release_rx.recv();
                for _ in 0..5 {
                    callback(&vec![500i16; 1_600], Some(1));
                }
                let _ = done_tx.send(());
                Ok::<_, String>(())
            },
        );
        assert!(matches!(got, Err(LoopbackStartError::Timeout)));
        release_tx.send(()).unwrap();
        done_rx
            .recv_timeout(Duration::from_secs(60))
            .expect("Voraussetzung: der Callback lief");

        assert_eq!(
            written_samples(&rig),
            None,
            "nach der Frist ist der Writer zu und es wurde nichts geschrieben"
        );
        assert_eq!(
            rig.levels.calls.load(Ordering::SeqCst),
            0,
            "auch kein Pegel"
        );
        assert!(rig.flags.failed_channels().is_empty());
        let stats = rig.pipeline.stats();
        assert_eq!(
            stats.overflow_samples[CHANNEL_SYSTEM as usize].load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            std::fs::metadata(&rig.path).unwrap().len(),
            wav_header_len(),
            "die WAV enthaelt keinen einzigen Ton"
        );

        // Der Start kann die Datei danach entfernen.
        remove_capture_files(rig.path.parent().unwrap());
        assert!(!rig.path.exists(), "die Datei liess sich entfernen");
    }

    /// Laenge des WAV-Kopfes (44 Byte, `wav_writer::HEADER_LEN`).
    fn wav_header_len() -> u64 {
        44
    }

    /// Das Gegenstueck: solange der Sink offen ist, schreibt derselbe Callback, zaehlt Pegel
    /// und erzeugt keinen Fehler. (Sonst bewiese der Test oben nichts.)
    #[test]
    fn the_same_callback_writes_while_the_sink_is_open() {
        let mut rig = callback_rig();
        (rig.callback)(&vec![500i16; 1_600], Some(1));
        assert_eq!(written_samples(&rig), Some(1_600));
        assert_eq!(rig.levels.calls.load(Ordering::SeqCst), 1);
        assert!(rig.flags.failed_channels().is_empty());
        close_sink(Some(&rig.sink));
        (rig.callback)(&vec![500i16; 1_600], Some(2));
        assert_eq!(
            rig.levels.calls.load(Ordering::SeqCst),
            1,
            "geschlossen = nichts mehr, auch kein Pegel"
        );
        assert_eq!(written_samples(&rig), None);
    }

    /// lva#538 Befund 1: der Abbruch ist eine BARRIERE. Ein Callback, der gerade laeuft (hier:
    /// im Pegelpfad, unter der Sperre seines Sinks), ist fertig, wenn `close_sink` zurueckkehrt;
    /// jeder spaetere tut nichts. Zwei Kanaele: der zweite wird unabhaengig vom ersten
    /// geschlossen. Deterministisch: der Callback steht nachweislich im Pegelpfad (Tor), der
    /// Schliessversuch laeuft nachweislich (Haken), erst dann wird der Callback freigegeben;
    /// danach WAV, DSP und Pegel unveraendert.
    #[test]
    fn a_running_callback_is_finished_when_the_barrier_returns() {
        let mut held = callback_rig();
        let mut other = callback_rig();
        let gate = Arc::new(Gate::default());
        *held.levels.gate.lock().unwrap() = Some(Arc::clone(&gate));
        let mut callback = std::mem::replace(&mut held.callback, Box::new(|_, _| {}));
        let in_flight = std::thread::spawn(move || {
            callback(&vec![500i16; 1_600], Some(1));
            callback
        });
        gate.wait_entered();
        assert!(
            held.sink.try_lock().is_err(),
            "Voraussetzung: der Callback haelt die Sperre seines Sinks"
        );
        assert_eq!(
            held.levels.finished.load(Ordering::SeqCst),
            0,
            "Voraussetzung: er ist noch nicht fertig"
        );

        // Der andere Kanal laesst sich unabhaengig schliessen (die Sperren sind je Kanal).
        close_sink(Some(&other.sink));
        assert_eq!(written_samples(&other), None);

        let (attempt_tx, attempt_rx) = mpsc::channel::<()>();
        let closing_sink = Arc::clone(&held.sink);
        let closing_levels = Arc::clone(&held.levels);
        let closer = std::thread::spawn(move || {
            close_sink_hooked(Some(&closing_sink), || attempt_tx.send(()).unwrap());
            // Was der Callback bei der Rueckkehr der Barriere geschafft hat.
            closing_levels.finished.load(Ordering::SeqCst)
        });
        attempt_rx.recv().unwrap();
        gate.open();
        let finished_at_return = closer.join().unwrap();
        assert_eq!(
            finished_at_return, 1,
            "die Barriere kehrte zurueck, waehrend ein Callback noch lief"
        );
        let mut callback = in_flight.join().unwrap();
        let stats = held.pipeline.stats();
        let overflow = stats.overflow_samples[CHANNEL_SYSTEM as usize].load(Ordering::Relaxed);
        callback(&vec![500i16; 1_600], Some(2));
        let mut other_callback = std::mem::replace(&mut other.callback, Box::new(|_, _| {}));
        other_callback(&vec![500i16; 1_600], Some(3));
        assert_eq!(held.levels.calls.load(Ordering::SeqCst), 1, "danach nichts");
        assert_eq!(
            other.levels.calls.load(Ordering::SeqCst),
            0,
            "auch der andere nicht"
        );
        assert_eq!(
            stats.overflow_samples[CHANNEL_SYSTEM as usize].load(Ordering::Relaxed),
            overflow,
            "und nichts mehr an den DSP"
        );
        assert_eq!(written_samples(&held), None);
        assert_eq!(written_samples(&other), None);

        let bytes = std::fs::read(&held.path).unwrap();
        assert_eq!(bytes.len() as u64, wav_header_len() + 1_600 * 2);
        let data_len = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        assert_eq!(data_len, 1_600 * 2, "der Kopf der behaltenen Datei stimmt");
        assert_eq!(
            std::fs::metadata(&other.path).unwrap().len(),
            wav_header_len(),
            "der zweite Kanal bekam keinen Ton"
        );
    }

    /// lva#538, Schreibfehler: der Callback setzt NUR das Merkzeichen (Atomic), verwirft den
    /// Writer und liefert weiter an den DSP (Transkription laeuft). Hier ueber die
    /// RIFF-Grenze, den deterministischen Schreibfehler.
    #[test]
    fn a_write_error_sets_the_flag_and_the_transcription_keeps_its_feed() {
        let mut rig = callback_rig();
        lock_recovering(&rig.sink)
            .writer
            .as_mut()
            .unwrap()
            .set_frames_for_test(u32::MAX as u64 / 2);

        (rig.callback)(&vec![500i16; 1_600], Some(1));

        assert_eq!(rig.flags.failed_channels(), vec![CHANNEL_SYSTEM]);
        assert!(
            lock_recovering(&rig.sink).writer.is_none(),
            "der Writer ist verworfen"
        );
        assert_eq!(
            rig.levels.calls.load(Ordering::SeqCst),
            1,
            "die Aufnahme laeuft fuer die Transkription weiter"
        );
        // Weitere Bloecke bleiben ohne neuen Fehler und ohne Schreiben.
        (rig.callback)(&vec![500i16; 1_600], Some(2));
        assert_eq!(rig.levels.calls.load(Ordering::SeqCst), 2);
        // Das Merkzeichen meldet nur einmal.
        assert!(rig.flags.first_report());
        assert!(!rig.flags.first_report());
    }

    /// lva#538 Befund 5: ein Fehler beim Kopf-Flush ist ein Schreibfehler wie jeder andere. Der
    /// Append davor ging durch, der Kopf nicht: die Datei laesst sich nach einem Absturz nicht
    /// mehr reparieren. Das Merkzeichen steht, der Writer ist verworfen, die Transkription
    /// laeuft weiter.
    #[test]
    fn a_header_flush_error_sets_the_flag_and_drops_the_writer() {
        let mut rig = callback_rig();
        lock_recovering(&rig.sink)
            .writer
            .as_mut()
            .unwrap()
            .fail_header_for_test();

        // Eine Sekunde Audio loest den Kopf-Flush aus.
        (rig.callback)(&vec![500i16; FLUSH_EVERY_SAMPLES], Some(1));

        assert_eq!(
            rig.flags.failed_channels(),
            vec![CHANNEL_SYSTEM],
            "der Fehler des Kopf-Flush versandete im Callback"
        );
        assert!(lock_recovering(&rig.sink).writer.is_none());
        assert_eq!(rig.levels.calls.load(Ordering::SeqCst), 1);
    }

    /// Ohne Fehler bleibt das Merkzeichen leer, auch ueber einen Kopf-Flush hinweg.
    #[test]
    fn a_healthy_header_flush_sets_no_flag() {
        let mut rig = callback_rig();
        (rig.callback)(&vec![500i16; FLUSH_EVERY_SAMPLES], Some(1));
        assert!(rig.flags.failed_channels().is_empty());
        assert_eq!(written_samples(&rig), Some(FLUSH_EVERY_SAMPLES as u64));
    }

    /// Ein Schreiben mit einem ECHTEN I/O-Fehler (die Datei laesst sich nicht mehr beschreiben,
    /// wie bei einem vollen Datentraeger): Merkzeichen und verworfener Writer, die Aufnahme
    /// laeuft fuer die Transkription weiter.
    #[test]
    fn a_real_io_error_is_flagged_and_does_not_stop_the_feed() {
        let mut rig = callback_rig();
        lock_recovering(&rig.sink)
            .writer
            .as_mut()
            .unwrap()
            .break_file_for_test(&rig.path);
        (rig.callback)(&vec![500i16; 1_600], Some(1));
        assert_eq!(rig.flags.failed_channels(), vec![CHANNEL_SYSTEM]);
        assert_eq!(rig.levels.calls.load(Ordering::SeqCst), 1);
    }

    // ---- lva#538: die Sitzung faellt = Barriere ---------------------------------------------

    /// Eine Sitzung mit echter Pipeline (langsame Engine) und dem echten Callback, ohne
    /// Capture-Geraet: der Test spielt den Treiber.
    struct SessionRig {
        _dir: tempfile::TempDir,
        store: Arc<MeetingStore>,
        meeting_id: String,
        path: PathBuf,
        session: Option<RecordingSession>,
        callback: RigCallback,
        levels: Arc<CountingLevels>,
        events: Arc<Mutex<Vec<MeetingEvent>>>,
        flags: Arc<WriteFlags>,
        calls: Arc<AtomicUsize>,
        witness: Arc<()>,
    }

    /// Die Engine der Sitzung bleibt am Tor `gate` stehen (mitten in einem Block), bis der Test es
    /// oeffnet: kein Warten auf Zeit, die Reihenfolge entscheidet.
    fn session_rig(gate: Arc<Gate>) -> SessionRig {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MeetingStore::open_at(&dir.path().join("m.db")).unwrap());
        let meeting = store
            .create_meeting("Abbruch", MeetingSource::Youtube, None)
            .unwrap();
        let path = dir.path().join("system.wav");
        let sink = Arc::new(Mutex::new(ChannelSink::new(
            StreamingWavWriter::create(&path, SAMPLE_RATE).unwrap(),
        )));
        let events = Arc::new(Mutex::new(Vec::new()));
        let event_sink = Arc::clone(&events);
        let calls = Arc::new(AtomicUsize::new(0));
        let call_count = Arc::clone(&calls);
        let witness = Arc::new(());
        let held = Arc::clone(&witness);
        let pipeline = LivePipeline::start(
            meeting.id.clone(),
            Arc::clone(&store),
            DspConfig::new(None, Arc::new(|_| {})).playback_only(),
            move |chunk: &Chunk| {
                let _keep = &held;
                call_count.fetch_add(1, Ordering::SeqCst);
                gate.pass();
                // Ein Wort je Sekunde Audio, damit weder Dichte noch Wiederholung greift.
                let secs = (chunk.samples.len() / 16_000).max(1);
                let text = (0..secs)
                    .map(|i| format!("wort{}", i as u64 + chunk.offset_ms))
                    .collect::<Vec<_>>()
                    .join(" ");
                vec![crate::managers::transcription::TimedSegment {
                    text,
                    start_ms: 0,
                    end_ms: chunk.samples.len() as u64 / 16,
                    words: None,
                }]
            },
            move |event| event_sink.lock().unwrap().push(event),
        )
        .unwrap();
        let levels = Arc::new(CountingLevels::default());
        let flags = Arc::new(WriteFlags::default());
        let callback = channel_callback(
            CHANNEL_SYSTEM,
            Arc::clone(&sink),
            Arc::new(AtomicBool::new(false)),
            Arc::clone(&flags),
            pipeline.feed(CHANNEL_SYSTEM).unwrap(),
            Arc::clone(&levels) as Arc<dyn LevelSink>,
        );
        let mut session = RecordingSession::new(
            &meeting.id,
            CaptureMode::PlaybackOnly,
            None,
            Some(sink),
            None,
            Some(path.clone()),
            FinalSlot::default().live_beginnen().unwrap(),
        );
        session.pipeline = Some(pipeline);
        SessionRig {
            _dir: dir,
            store,
            meeting_id: meeting.id,
            path,
            session: Some(session),
            callback: Box::new(callback),
            levels,
            events,
            flags,
            calls,
            witness,
        }
    }

    /// Spielt `seconds` Sekunden Ton durch den Callback (wie ein Treiber, 100-ms-Bloecke).
    fn drive(rig: &mut SessionRig, seconds: usize) {
        for _ in 0..seconds * 10 {
            (rig.callback)(&vec![3_000i16; 1_600], Some(1));
        }
    }

    /// lva#538 Befund 3: ein abgebrochener Start veroeffentlicht kein Transkript mehr. Die
    /// Engine steht mitten in einem Block (Tor), als die Sitzung faellt. Reihenfolge, nicht
    /// Zeit: der Abbruch ist gesetzt, das Fallenlassen haengt am Join des Workers; erst wenn die
    /// Engine freigegeben ist, kehrt es zurueck (der Zeuge ist weg). Dann steht kein Segment in der
    /// Datenbank, es kam kein Ereignis, der Rollback stellt den Eintrag wieder her, und ein
    /// Treiber, der danach noch Callbacks ruft, bewirkt nichts mehr (DB, Ereignisse, WAV, Pegel).
    #[test]
    fn a_dropped_session_leaves_no_segment_event_or_callback_effect() {
        let gate = Arc::new(Gate::default());
        let mut rig = session_rig(Arc::clone(&gate));
        let before = rig.store.get_meeting(&rig.meeting_id).unwrap().unwrap();
        rig.store
            .begin_playback_capture(&rig.meeting_id, 1)
            .unwrap();
        // 20 s Ton fuellen den ersten Block des Notfall-Chunkers; die Engine beginnt.
        drive(&mut rig, 21);
        gate.wait_entered();
        assert_eq!(
            Arc::strong_count(&rig.witness),
            2,
            "Voraussetzung: Worker lebt"
        );

        let discard = rig
            .session
            .as_ref()
            .unwrap()
            .pipeline
            .as_ref()
            .unwrap()
            .discard_handle();
        let session = rig.session.take().unwrap();
        let dropper = std::thread::spawn(move || drop(session));
        until("Abbruch gesetzt", || discard.load(Ordering::Acquire));
        assert!(
            !dropper.is_finished(),
            "das Fallenlassen kehrte zurueck, waehrend die Engine noch mitten im Block stand"
        );
        assert_eq!(
            Arc::strong_count(&rig.witness),
            2,
            "der Worker ist noch nicht gejoint"
        );

        gate.open();
        dropper.join().unwrap();
        assert_eq!(
            Arc::strong_count(&rig.witness),
            1,
            "das Fallenlassen kehrte zurueck, bevor der Worker gejoint war"
        );
        assert!(
            rig.store.get_segments(&rig.meeting_id).unwrap().is_empty(),
            "nach dem Abbruch steht ein Segment in der Datenbank"
        );
        let event_count = rig.events.lock().unwrap().len();
        assert!(
            !rig.events
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, MeetingEvent::Segments { .. })),
            "nach dem Abbruch kam ein Segmente-Ereignis"
        );

        // Erst NACH dem Join: der Rollback stellt den Eintrag her.
        assert!(rig
            .store
            .rollback_playback_capture(&rig.meeting_id)
            .unwrap());
        let restored = rig.store.get_meeting(&rig.meeting_id).unwrap().unwrap();
        assert_eq!(restored.status, before.status);
        assert!(restored.system_audio_path.is_none());
        let calls = rig.calls.load(Ordering::SeqCst);
        let levels = rig.levels.calls.load(Ordering::SeqCst);
        let size = std::fs::metadata(&rig.path).unwrap().len();

        // Ein spaeter Treiber: nichts davon darf noch eine Wirkung haben.
        drive(&mut rig, 3);
        assert_eq!(
            rig.levels.calls.load(Ordering::SeqCst),
            levels,
            "kein Pegel"
        );
        assert_eq!(rig.calls.load(Ordering::SeqCst), calls, "keine Engine");
        assert_eq!(
            std::fs::metadata(&rig.path).unwrap().len(),
            size,
            "kein WAV"
        );
        assert!(rig.store.get_segments(&rig.meeting_id).unwrap().is_empty());
        assert_eq!(
            rig.events.lock().unwrap().len(),
            event_count,
            "kein Ereignis mehr nach dem Rollback"
        );
    }

    /// Das Gegenstueck: ohne Abbruch landet derselbe Block als Segment in der Datenbank
    /// (`drain` statt `abort`). Sonst bewiese der Test oben nichts.
    #[test]
    fn the_same_session_stores_its_segment_when_it_is_drained_not_aborted() {
        let gate = Arc::new(Gate::default());
        gate.open();
        let mut rig = session_rig(Arc::clone(&gate));
        drive(&mut rig, 21);
        gate.wait_entered();
        let parts = rig.session.take().unwrap().into_parts();
        close_sink(parts.system_sink.as_ref());
        parts.pipeline.unwrap().drain();
        assert!(
            !rig.store.get_segments(&rig.meeting_id).unwrap().is_empty(),
            "Voraussetzung: der Block wurde transkribiert"
        );
    }

    /// lva#538 Befund 5, Runde 3: der Zugriff auf den Kopf scheitert gezielt NACH einem
    /// erfolgreichen Append. Das Merkzeichen steht, der Writer ist verworfen, weitere Appends
    /// werden abgelehnt (die Datei waechst nicht mehr), und der DSP wird weiter beschickt (die
    /// Engine bekommt noch ihren Block).
    #[test]
    fn a_failing_header_after_a_good_append_stops_the_writes_but_not_the_dsp_feed() {
        let gate = Arc::new(Gate::default());
        gate.open();
        let mut rig = session_rig(Arc::clone(&gate));
        let sink = Arc::clone(rig.session.as_ref().unwrap().system_sink.as_ref().unwrap());
        (rig.callback)(&vec![3_000i16; 1_600], Some(1));
        assert_eq!(
            lock_recovering(&sink)
                .writer
                .as_ref()
                .map(StreamingWavWriter::frames_written),
            Some(1_600),
            "Voraussetzung: der Append davor ging durch"
        );
        lock_recovering(&sink)
            .writer
            .as_mut()
            .unwrap()
            .fail_header_for_test();
        (rig.callback)(&vec![3_000i16; FLUSH_EVERY_SAMPLES], Some(2));
        assert_eq!(rig.flags.failed_channels(), vec![CHANNEL_SYSTEM]);
        assert!(lock_recovering(&sink).writer.is_none());
        let size = std::fs::metadata(&rig.path).unwrap().len();

        // Weiter: 21 s Ton. Die WAV waechst nicht mehr, die Engine kommt trotzdem dran.
        drive(&mut rig, 21);
        gate.wait_entered();
        assert!(
            rig.calls.load(Ordering::SeqCst) >= 1,
            "der DSP wurde beschickt"
        );
        assert_eq!(
            std::fs::metadata(&rig.path).unwrap().len(),
            size,
            "weitere Appends wurden abgelehnt"
        );
    }

    // ---- lva#538 Befund 6: der Marker und der Plan, fail-closed ---------------------------------

    /// Das dauerhafte Merkzeichen: einmal geschrieben, bleibt es (auch nach einem Absturz);
    /// zweimal schreiben schadet nicht; andere Schluessel bleiben.
    #[test]
    fn the_incomplete_audio_marker_is_durable_and_leaves_other_keys_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
        let meeting = store
            .create_meeting("Montag", MeetingSource::Live, Some(1))
            .unwrap();
        store
            .set_metadata_key(&meeting.id, "timeline", serde_json::json!({"basis": "qpc"}))
            .unwrap();

        mark_audio_incomplete(&store, &meeting.id, &[CHANNEL_SYSTEM]).unwrap();
        mark_audio_incomplete(&store, &meeting.id, &[CHANNEL_MIC, CHANNEL_SYSTEM]).unwrap();

        let meta = store.metadata_json(&meeting.id).unwrap().unwrap();
        assert_eq!(
            meta[final_pass::AUDIO_INCOMPLETE_KEY]["channels"],
            serde_json::json!([0, 1])
        );
        assert_eq!(meta["timeline"]["basis"], "qpc");
    }

    /// Ein Marker, der sich nicht speichern laesst, ist ein FEHLER (nicht nur ein Logeintrag):
    /// der Aufrufer weicht darauf fail-closed aus.
    #[test]
    fn a_marker_that_cannot_be_stored_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
        // Eine Zeile, die es nicht gibt (geloescht).
        assert!(mark_audio_incomplete(&store, "gibt-es-nicht", &[CHANNEL_MIC]).is_err());
        assert!(!persist_audio_incomplete(
            &store,
            "gibt-es-nicht",
            &[CHANNEL_MIC],
            2
        ));
        let meeting = store
            .create_meeting("Da", MeetingSource::Live, Some(1))
            .unwrap();
        assert!(persist_audio_incomplete(
            &store,
            &meeting.id,
            &[CHANNEL_MIC],
            1
        ));
    }

    /// lva#538 Befund 6, fail-closed: laesst sich der Marker nicht speichern, bleibt das
    /// Live-Transkript trotzdem (`Keep(AudioIncomplete)`), egal was der Plan vorher war. Steht er,
    /// bleibt der Plan unberuehrt; ohne Schreibfehler wird der Marker gar nicht erst angefasst.
    #[test]
    fn a_marker_that_cannot_be_stored_keeps_the_live_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
        let meeting = store
            .create_meeting("Da", MeetingSource::Live, Some(1))
            .unwrap();
        // Ein Plan, der das Live-Transkript ersetzen wuerde.
        let replace = || FinalPlan::Run {
            model_id: "endmodell".to_string(),
            max_segment_ms: 30_000,
        };

        // Kein Fehler: der Plan bleibt, kein Marker.
        let plan = plan_after_write_failures(&store, &meeting.id, &[], replace(), 1);
        assert_eq!(plan, replace());
        assert!(store
            .metadata_json(&meeting.id)
            .unwrap()
            .is_none_or(|m| m.get(final_pass::AUDIO_INCOMPLETE_KEY).is_none()));

        // Fehler, Marker steht: der Plan bleibt, der Marker ist da.
        let plan = plan_after_write_failures(&store, &meeting.id, &[CHANNEL_SYSTEM], replace(), 1);
        assert_eq!(plan, replace());
        let meta = store.metadata_json(&meeting.id).unwrap().unwrap();
        assert!(meta.get(final_pass::AUDIO_INCOMPLETE_KEY).is_some());

        // Fehler, Marker laesst sich nicht speichern: fail-closed.
        let plan =
            plan_after_write_failures(&store, "gibt-es-nicht", &[CHANNEL_SYSTEM], replace(), 2);
        assert_eq!(
            plan,
            FinalPlan::Keep(KeepReason::AudioIncomplete),
            "ohne Marker darf der Enddurchlauf das Transkript nicht ersetzen"
        );
    }

    /// Das Speichern der Audiopfade ist ein Fehler des Starts: fuer eine geloeschte Zeile
    /// `meeting_not_found` (keine Aufnahme fuer einen geloeschten Eintrag).
    #[test]
    fn failing_to_store_the_audio_paths_fails_the_start_with_the_right_code() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
        let meeting = store
            .create_meeting("Weg", MeetingSource::Youtube, None)
            .unwrap();
        store.soft_delete_meeting(&meeting.id).unwrap();
        let error = store
            .set_audio_paths(&meeting.id, None, Some("C:/m/system.wav"), None)
            .unwrap_err();
        assert_eq!(audio_paths_error(&error), "meeting_not_found");
        let other = anyhow::anyhow!("database is locked");
        assert_eq!(
            audio_paths_error(&other),
            "audio_paths_failed: database is locked"
        );
    }

    fn with_source(source: &str, mic: Option<&str>, system: Option<&str>) -> Meeting {
        let mut m = meeting_with("recording", mic, system);
        m.source = source.into();
        m
    }

    /// Recovery und "Fortsetzen": Live immer, YouTube nur mit eigener Aufnahme.
    #[test]
    fn only_recordings_get_the_plan_of_the_final_pass() {
        assert!(runs_final_pass(&with_source("live", Some("m.wav"), None)));
        assert!(runs_final_pass(&with_source(
            "youtube",
            None,
            Some("system.wav")
        )));
        assert!(
            !runs_final_pass(&with_source("youtube", None, None)),
            "ohne Aufnahme gibt es nichts zu verarbeiten"
        );
        for source in ["import", "subtitle", "empty"] {
            assert!(
                !runs_final_pass(&with_source(source, Some("a.wav"), None)),
                "{source}"
            );
        }
    }

    /// Ein gescheiterter Mitschnitt entfernt nur `system.wav` und was er sonst
    /// angelegt hat; die Folien der YouTube-Besprechung bleiben samt Ordner.
    #[test]
    fn a_failed_playback_start_removes_only_its_own_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("m1");
        std::fs::create_dir_all(&folder).unwrap();
        drop(StreamingWavWriter::create(&folder.join("system.wav"), SAMPLE_RATE).unwrap());
        std::fs::write(folder.join("folien.pdf"), b"gehoert dem Nutzer").unwrap();

        remove_capture_files(&folder);

        assert!(!folder.join("system.wav").exists());
        assert!(folder.join("folien.pdf").exists(), "Fremdes bleibt");
        assert!(folder.exists());
        assert!(
            !folder.join("mic.wav").exists(),
            "beim Mitschnitt entsteht nie eine Mikrofonspur"
        );
    }

    #[test]
    fn a_failed_playback_start_removes_the_folder_when_nothing_else_is_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("m2");
        std::fs::create_dir_all(&folder).unwrap();
        drop(StreamingWavWriter::create(&folder.join("system.wav"), SAMPLE_RATE).unwrap());
        remove_capture_files(&folder);
        assert!(!folder.exists());
        // Ein nie angelegter Ordner ist kein Fehler.
        remove_capture_files(&dir.path().join("gibt-es-nicht"));
    }

    /// Das Zusammenspiel von Zielpruefung und Rueckfall wie `start_playback_capture` es
    /// nutzt: Start scheitert nach der Umwandlung -> Eintrag wie vorher, Dateien weg,
    /// nichts, was der naechste App-Start als Absturz belebt.
    #[test]
    fn a_playback_start_that_fails_leaves_the_entry_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        let entry = store
            .create_meeting("Vortrag", MeetingSource::Youtube, None)
            .unwrap();
        let folder = dir.path().join("meetings").join(&entry.id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("folien.pdf"), b"x").unwrap();

        let started = store.begin_playback_capture(&entry.id, 7).unwrap();
        assert_eq!(started.status, "recording");
        // `start_row`: Datei und Pfad stehen, dann scheitert der Systemton.
        let system = folder.join("system.wav");
        drop(StreamingWavWriter::create(&system, SAMPLE_RATE).unwrap());
        store
            .set_audio_paths(&entry.id, None, system.to_str(), None)
            .unwrap();

        remove_capture_files(&folder);
        assert!(store.restore_playback_target(&entry.id).unwrap());

        let after = store.get_meeting(&entry.id).unwrap().unwrap();
        assert_eq!(
            (after.source.as_str(), after.status.as_str()),
            ("youtube", "ready")
        );
        assert_eq!(after.system_audio_path, None);
        assert!(!is_orphan_status(&after.status));
        assert!(folder.join("folien.pdf").exists());
        assert!(!system.exists());
    }

    // ---- lva#538: Recovery eines Mitschnitts, der nie etwas aufgenommen hat -------------

    fn crashed_capture(
        dir: &std::path::Path,
        with_audio: bool,
        write_paths: bool,
    ) -> (MeetingStore, Meeting, PathBuf) {
        let store = MeetingStore::open_at(&dir.join("meetings.db")).unwrap();
        let entry = store
            .create_meeting("Vortrag", MeetingSource::Youtube, None)
            .unwrap();
        store
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE meetings SET status = 'ready', duration_ms = 1805000 WHERE id = ?1",
                rusqlite::params![entry.id],
            )
            .unwrap();
        let folder = dir.join("meetings").join(&entry.id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("folien.pdf"), b"x").unwrap();
        store.begin_playback_capture(&entry.id, 7).unwrap();
        let system = folder.join("system.wav");
        let mut writer = StreamingWavWriter::create(&system, SAMPLE_RATE).unwrap();
        if with_audio {
            writer.append(&vec![300i16; 32_000]).unwrap();
        }
        drop(writer); // Absturz: weder finalize noch Kopfaktualisierung
        if write_paths {
            store
                .set_audio_paths(&entry.id, None, system.to_str(), None)
                .unwrap();
        }
        let meeting = store.get_meeting(&entry.id).unwrap().unwrap();
        (store, meeting, system)
    }

    /// Crash NACH dem Speichern der Pfade, vor dem ersten Ton: der naechste App-Start
    /// setzt den Eintrag zurueck (Dauer des Players, `ready`, keine Pfade), entfernt die
    /// leere WAV und laesst die Folien des Nutzers stehen. Danach ist er erneut startbar.
    #[test]
    fn the_recovery_resets_a_capture_that_crashed_before_any_audio() {
        let dir = tempfile::tempdir().unwrap();
        let (store, meeting, system) = crashed_capture(dir.path(), false, true);
        assert_eq!(meeting.status, "recording", "Voraussetzung: Absturzrest");
        assert!(system.exists());

        assert!(reset_unrecorded_capture(
            &store,
            &meeting,
            Some(&dir.path().join("meetings"))
        ));

        let after = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(
            (
                after.status.as_str(),
                after.duration_ms,
                after.consent_confirmed_at
            ),
            ("ready", Some(1_805_000), None)
        );
        assert_eq!(after.system_audio_path, None);
        assert!(!system.exists(), "die leere WAV ist weg");
        assert!(system.parent().unwrap().join("folien.pdf").exists());
        assert!(
            store.begin_playback_capture(&meeting.id, 8).is_ok(),
            "kein target_not_empty"
        );
    }

    /// Crash VOR dem Speichern der Pfade (Datei angelegt, Zeile steht auf `recording`).
    #[test]
    fn the_recovery_resets_a_capture_that_crashed_before_the_paths_were_stored() {
        let dir = tempfile::tempdir().unwrap();
        let (store, meeting, system) = crashed_capture(dir.path(), false, false);
        assert_eq!(meeting.system_audio_path, None);
        assert!(system.exists());

        assert!(reset_unrecorded_capture(
            &store,
            &meeting,
            Some(&dir.path().join("meetings"))
        ));

        let after = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(
            (after.status.as_str(), after.duration_ms),
            ("ready", Some(1_805_000))
        );
        assert!(
            !system.exists(),
            "auch die Datei ohne Pfad in der Datenbank"
        );
    }

    /// lva#538 Runde 4: der echte Start setzt den Marker `capture_open` (`begin_capture`), BEVOR
    /// die erste Probe geschrieben ist. Ein Absturz vor dem ersten Ton laesst ihn stehen; das
    /// Zuruecksetzen nimmt ihn mit. Sonst machte der naechste, echte Mitschnitt daraus ein
    /// dauerhaftes `audio_incomplete` (`capture_not_closed`) und der Enddurchlauf ersetzte nie.
    #[test]
    fn the_recovery_drops_the_capture_marker_that_was_set_before_the_first_audio() {
        let dir = tempfile::tempdir().unwrap();
        let (store, meeting, system) = crashed_capture(dir.path(), false, false);
        store
            .begin_capture(&meeting.id, None, system.to_str())
            .unwrap();
        let meeting = store.get_meeting(&meeting.id).unwrap().unwrap();
        let meta = store.metadata_json(&meeting.id).unwrap().unwrap();
        assert!(
            meta.get(super::super::store::CAPTURE_OPEN_KEY).is_some(),
            "Voraussetzung: der Marker steht vor dem ersten Ton"
        );

        assert!(reset_unrecorded_capture(
            &store,
            &meeting,
            Some(&dir.path().join("meetings"))
        ));

        let meta = store.metadata_json(&meeting.id).unwrap().unwrap();
        assert!(
            meta.get(super::super::store::CAPTURE_OPEN_KEY).is_none()
                && meta.get(final_pass::AUDIO_INCOMPLETE_KEY).is_none(),
            "{meta}"
        );
        // Der naechste Start erbt nichts.
        store.begin_playback_capture(&meeting.id, 8).unwrap();
        store
            .begin_capture(&meeting.id, None, system.to_str())
            .unwrap();
        let meta = store.metadata_json(&meeting.id).unwrap().unwrap();
        assert!(
            meta.get(final_pass::AUDIO_INCOMPLETE_KEY).is_none(),
            "{meta}"
        );
    }

    /// Mit Ton auf der Platte laeuft die normale Wiederherstellung: nichts wird
    /// zurueckgesetzt, die Datei bleibt, der Vorzustand ist erledigt.
    #[test]
    fn the_recovery_keeps_a_capture_that_has_audio() {
        let dir = tempfile::tempdir().unwrap();
        let (store, meeting, system) = crashed_capture(dir.path(), true, true);

        assert!(!reset_unrecorded_capture(
            &store,
            &meeting,
            Some(&dir.path().join("meetings"))
        ));

        let after = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(
            after.status, "recording",
            "die Wiederherstellung uebernimmt"
        );
        assert!(after.system_audio_path.is_some());
        assert!(system.exists(), "der Ton bleibt");
        assert_eq!(store.playback_prior(&meeting.id).unwrap(), None);
        assert!(
            runs_final_pass(&after),
            "und sie bekommt ihren Enddurchlauf"
        );
    }

    /// lva#538 Befund 7, fail-closed: laesst sich der Dateizustand nicht lesen (hier ein Pfad,
    /// den das Dateisystem ablehnt), setzt die Wiederherstellung nichts zurueck und loescht
    /// nichts, auch den Merker des Vorzustands nicht.
    #[test]
    fn the_recovery_keeps_a_capture_whose_audio_state_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let (store, meeting, system) = crashed_capture(dir.path(), false, true);
        let mut unreadable = meeting.clone();
        unreadable.system_audio_path = Some("C:/m/sys\0tem.wav".to_string());

        assert!(!reset_unrecorded_capture(
            &store,
            &unreadable,
            Some(&dir.path().join("meetings"))
        ));

        let after = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(after.status, "recording", "nichts zurueckgesetzt");
        assert!(after.system_audio_path.is_some());
        assert!(system.exists(), "nichts geloescht");
        assert!(
            store.playback_prior(&meeting.id).unwrap().is_some(),
            "der Weg zurueck bleibt"
        );
    }

    /// lva#538 Befund 7, Runde 3: ein EINGESPIELTER Zugriffsfehler (Zugriff verweigert, E/A-Fehler,
    /// Zeitueberschreitung) ist nicht "leer": die Wiederherstellung aendert weder die Pfade noch
    /// den Merker des Vorzustands noch das Audio. Ein bestaetigtes `NotFound` dagegen setzt zurueck.
    #[test]
    fn an_injected_access_error_keeps_paths_prior_marker_and_audio() {
        use super::super::playback::{audio_state_of, AudioState};
        use std::io::{Error, ErrorKind};
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::Other,
            ErrorKind::TimedOut,
            ErrorKind::Interrupted,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (store, meeting, system) = crashed_capture(dir.path(), false, true);
            let size = std::fs::metadata(&system).unwrap().len();
            let reset = reset_unrecorded_capture_with(
                &store,
                &meeting,
                Some(&dir.path().join("meetings")),
                |_| audio_state_of([Err(Error::from(kind))]),
            );
            assert!(!reset, "{kind:?}: ein unlesbarer Zustand ist nicht leer");
            let after = store.get_meeting(&meeting.id).unwrap().unwrap();
            assert_eq!(after.status, "recording", "{kind:?}: nichts zurueckgesetzt");
            assert_eq!(
                after.system_audio_path, meeting.system_audio_path,
                "{kind:?}"
            );
            assert_eq!(std::fs::metadata(&system).unwrap().len(), size, "{kind:?}");
            assert!(
                store.playback_prior(&meeting.id).unwrap().is_some(),
                "{kind:?}: der Weg zurueck bleibt"
            );
        }
        // Gegenprobe: ein bestaetigtes Fehlen (NotFound) setzt zurueck.
        let dir = tempfile::tempdir().unwrap();
        let (store, meeting, system) = crashed_capture(dir.path(), false, true);
        assert!(reset_unrecorded_capture_with(
            &store,
            &meeting,
            Some(&dir.path().join("meetings")),
            |_| audio_state_of([Err(Error::from(ErrorKind::NotFound))]),
        ));
        assert!(!system.exists());
        assert_eq!(
            audio_state_of([Err(Error::from(ErrorKind::NotFound))]),
            AudioState::Empty
        );
    }

    /// Altdaten: ein Absturzrest OHNE Merker (Zeile aus der ersten Fassung) und Zeilen
    /// anderer Quellen oder Zustaende bleiben unangetastet.
    #[test]
    fn the_recovery_leaves_rows_without_the_marker_and_other_sources_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (store, meeting, _) = crashed_capture(dir.path(), false, true);
        store.discard_playback_prior(&meeting.id).unwrap();
        assert!(!reset_unrecorded_capture(&store, &meeting, None));
        assert_eq!(
            store.get_meeting(&meeting.id).unwrap().unwrap().status,
            "recording"
        );
        let mut live = meeting.clone();
        live.source = "live".into();
        assert!(!reset_unrecorded_capture(&store, &live, None));
        let mut ready = meeting.clone();
        ready.status = "ready".into();
        assert!(!reset_unrecorded_capture(&store, &ready, None));
    }

    /// Der Rueckfall des Starts selbst scheiterte (Zeile `failed` mit Merker): der naechste
    /// App-Start setzt auch sie zurueck.
    #[test]
    fn the_recovery_resets_a_failed_row_that_still_has_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let (store, meeting, _) = crashed_capture(dir.path(), false, true);
        store
            .set_status(&meeting.id, MeetingStatus::Failed)
            .unwrap();
        let failed = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert!(reset_unrecorded_capture(&store, &failed, None));
        assert_eq!(
            store.get_meeting(&meeting.id).unwrap().unwrap().status,
            "ready"
        );
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
            let starting = core.begin_start(None, Some("m")).unwrap();
            let text = unter_tor(&|| {
                let _ = core.publish_started(&starting, "m", 1);
            });
            assert!(text.contains("publish_started"), "{text}");
            drop(starting);
            assert!(core.is_idle(), "nichts veroeffentlicht");
            let text = unter_tor(&|| {
                let _ = core.begin_stop(None, std::time::Instant::now());
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

        /// R5-1: scheitert der Threadstart eines neuen Auftrags, bleibt der laufende Vorgaenger im
        /// Platz (mit seinem Abbruch-Merker). Der Start einer Aufnahme bricht ihn ab und wartet
        /// sein Ende ab, bevor das Tor betreten wird und die Aufnahme beginnt; vorher laeuft
        /// nichts von ihr. Der Vorgaenger haengt an einem Kanal (wie ein Modellaufruf, der sich
        /// nicht unterbrechen laesst); die Reihenfolge steht im Protokoll, keine Zeitmessung.
        #[test]
        fn a_failed_thread_start_keeps_the_running_final_pass_tracked() {
            let (_dir, store) = store();
            let slot = Arc::new(FinalSlot::default());
            let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
            let (weiter_tx, weiter_rx) = mpsc::channel::<()>();
            let (merker_tx, merker_rx) = mpsc::channel::<Arc<AtomicBool>>();
            let l = Arc::clone(&log);
            slot.spawn(move |_vorher, cancel| {
                let _ = merker_tx.send(Arc::clone(&cancel));
                let _ = weiter_rx.recv();
                l.lock().unwrap().push(if cancel.load(Ordering::Acquire) {
                    "vorgaenger_abgebrochen_beendet"
                } else {
                    "vorgaenger_ohne_abbruch_beendet"
                });
            })
            .unwrap();
            let merker = merker_rx.recv().unwrap();

            // Der naechste Auftrag entsteht nicht: der Threadstart scheitert.
            let nie = Arc::new(AtomicBool::new(false));
            let lief = Arc::clone(&nie);
            let r = slot.spawn_mit(
                move |_vorher, _cancel| lief.store(true, Ordering::Release),
                |_lauf| Err(std::io::Error::other("kein Thread (injiziert)")),
            );
            assert!(r.is_err());
            assert!(
                slot.active(),
                "der Vorgaenger steht weiter im Platz (sonst liefe er unverfolgt neben der Aufnahme)"
            );
            assert!(
                !merker.load(Ordering::Acquire),
                "der gescheiterte Start bricht nichts ab"
            );

            // Start einer Aufnahme: bricht den Vorgaenger ab und wartet auf sein Ende.
            let (fertig_tx, fertig_rx) = mpsc::channel();
            let (s2, st2, l1, l2) = (
                Arc::clone(&slot),
                Arc::clone(&store),
                Arc::clone(&log),
                Arc::clone(&log),
            );
            let start = std::thread::spawn(move || {
                let r = s2.start_unter_tor(
                    &st2,
                    None,
                    None,
                    move || l1.lock().unwrap().push("abbruch_abgewartet"),
                    move |_tor| {
                        l2.lock().unwrap().push("aufnahme_beginnt");
                        Ok(())
                    },
                );
                let _ = fertig_tx.send(r.map(|((), live)| live));
            });
            // Der Start hat den Vorgaenger gefunden: dessen Merker ist gesetzt. (Die Frist ist nur
            // ein Netz gegen ein Haengen, kein Beweis.)
            let netz = Instant::now() + Duration::from_secs(60);
            while !merker.load(Ordering::Acquire) {
                assert!(
                    Instant::now() < netz,
                    "der Start sieht den Vorgaenger nicht"
                );
                std::thread::yield_now();
            }
            // Solange der Vorgaenger haengt, steht er im Platz und der Start wartet: nichts von
            // der Aufnahme ist gelaufen.
            assert!(slot.active());
            assert!(log.lock().unwrap().is_empty(), "{:?}", log.lock().unwrap());

            weiter_tx.send(()).unwrap();
            let live = fertig_rx
                .recv_timeout(Duration::from_secs(60))
                .expect("der Start endet nicht")
                .expect("der Start gelingt");
            start.join().unwrap();
            assert_eq!(
                *log.lock().unwrap(),
                [
                    "vorgaenger_abgebrochen_beendet",
                    "abbruch_abgewartet",
                    "aufnahme_beginnt"
                ]
            );
            assert!(
                !nie.load(Ordering::Acquire),
                "der gescheiterte Auftrag lief nie"
            );
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
