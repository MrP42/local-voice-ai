//! M8 meetings: the recorder manager that orchestrates a live meeting —
//! dual capture (mic + system loopback), streaming WAV writing, the live
//! chunk -> transcription -> delta pipeline, the consent gate, crash
//! recovery and the recording indicator (tray + overlay).
//!
//! Everything that can be decided without I/O lives in the free functions at
//! the top (`consent_gate`, `may_start`, `apply_pause`) so the state rules are
//! testable without Tauri, audio hardware or a model.
//!
//! #537: `start_playback_capture` nimmt NUR den Systemton auf (kein Mikrofon) und
//! schreibt ihn in eine vorhandene YouTube-Besprechung (`playback.rs`): der Nutzer
//! spielt das Video im eingebetteten Player ab, die Aufnahme laeuft wie jede andere
//! (`system.wav`, Live-Transkript, Stopp, Enddurchlauf). Dasselbe Geruest wie die
//! Besprechung, nur mit [`CaptureMode::PlaybackOnly`]: kein `mic.wav`, kein
//! Mikrofon-Capture, kein Echo, `RecordingSession.mic_*` leer.
//!
//! Fehlerfaelle des Systemton-only-Modus und ihre Absicherung (Stand lva#538):
//! - **Kein Ausgabegeraet / Loopback scheitert**: der Start schlaegt mit
//!   `playback_capture_failed` fehl, nie still "nur Mikrofon" (es gibt keins); das Ziel
//!   wird aus dem gespeicherten Vorzustand zurueckgesetzt (`restore_playback_target`),
//!   seine Dateien entfernt. Test: `a_failing_start_reports_its_error_at_once`,
//!   Rueckfall in `playback.rs`.
//! - **Treiber haengt (Frist)**: `playback_capture_timeout`. Die Frist setzt den
//!   Abbruchmerker des Kanals (`silenced`): kehrt der Treiber spaeter zurueck und ruft
//!   den Callback, schreibt der nichts mehr, und der Start schliesst den WAV-Writer
//!   selbst, bevor er die Dateien entfernt (offene Handles verhindern das Loeschen).
//!   Tests: `a_wedged_start_times_out...`, `a_late_callback_after_the_timeout_...`.
//! - **Stopp/Abbruch waehrend des Starts**: `RunCore::begin_stop` setzt den Abbruchmerker
//!   des laufenden Starts, `publish_started` veroeffentlicht dann nichts mehr (beides
//!   unter denselben Sperren: nie eine Geisteraufnahme). Der Start endet mit
//!   `start_cancelled`, der Eintrag ist wie vorher; `stop()` kehrt erst danach zurueck.
//!   Tests: `a_stop_during_a_start_...`, `a_stop_racing_the_publish_...`.
//! - **Loeschen waehrend Start oder Aufnahme**: `abort_for_delete` (vor dem Loeschen):
//!   der Start endet mit `meeting_not_found`, eine laufende Aufnahme wird beendet, ohne
//!   Enddurchlauf. Vor dem Veroeffentlichen prueft der Start, dass die Zeile noch da ist;
//!   das Speichern der Audiopfade ist kein "nur loggen" mehr. Tests: `a_delete_...`.
//! - **Absturz mitten im Start**: Vorzustand steht in der Zeile (`playback.rs`),
//!   `recover_orphans` setzt einen Mitschnitt ohne Ton zurueck (`reset_unrecorded_capture`).
//! - **Loopback stirbt mitten in der Aufnahme**: der Waechter meldet
//!   `playback_capture_died` (nicht den Mikrofon-Text der Besprechung); die Aufnahme
//!   bleibt stehen, der Nutzer stoppt, das bis dahin Gehoerte bleibt.
//! - **Mikrofon-Waechter**: Kanal 0 liefert nie, ein Standard-Waechter meldete nach 8 s
//!   `NoData` (`WatchConfig::off()`, Tests in `signal_watch.rs`/`dsp.rs`); Stille am
//!   Systemton fragt der Waechter nach 30 s nach (`WatchConfig::playback()`), auch ohne
//!   ein einziges Paket (WASAPI-Loopback liefert bei Stille keine; Test in `dsp.rs`).
//! - **Warten auf den Mikrofonkanal**: der DSP-Thread, `drain()` und der
//!   Enddurchlauf behandeln jede Spur einzeln; ein nie beschickter Kanal blockiert
//!   nichts (Test `a_system_only_feed_...`, `dsp.rs`).
//! - **Null-WAV-Leichen**: es wird nur `system.wav` angelegt; auch ein gescheiterter
//!   Start entfernt nur, was er selbst angelegt hat (`remove_capture_files`); ein
//!   Mitschnitt, der ohne einen Ton endet, setzt den Eintrag zurueck (`stop`).
//! - **Voller Datentraeger / Schreibfehler** (gemeinsamer Pfad, auch Live-Aufnahmen):
//!   der Callback verwirft den WAV-Writer und setzt ein Merkzeichen (`WriteFlags`, nur
//!   Atomics); ein Waechter-Thread und `stop()` schreiben es DAUERHAFT als
//!   `metadata_json.audio_incomplete` und melden `audio_write_failed`. Der Enddurchlauf
//!   ersetzt das Live-Transkript dann nie aus der unvollstaendigen Aufnahme
//!   (`KeepReason::AudioIncomplete`, auch ohne Merker, wenn die Aufnahme kuerzer ist als
//!   das Transkript). Tests: `a_write_error_...`, `final_pass.rs`.
//! - **Absturz der App**: die Pfade stehen ab dem Start in der Datenbank,
//!   `recover_orphans` repariert `system.wav` und holt den Rest nach
//!   (`runs_final_pass`: auch fuer eine YouTube-Besprechung mit eigener Aufnahme).
//! - **Zwei Starts gleichzeitig**: `start_guard`, `begin_start` und die atomare
//!   Umwandlung in `begin_playback_capture` (nur einer gewinnt).
//! - **Echtzeit**: der Callback ist derselbe wie beim Systemton der Besprechung
//!   (WAV-Append, `try_send`, Pegel). Neu darin: nur Atomics (Abbruch, Schreibfehler),
//!   keine Allokation, keine Datenbank. Ein Ueberlauf der DSP-Queue wird gezaehlt und
//!   als `Health { QueueOverflow }` gemeldet (nicht verschluckt). Vorbestehend und
//!   unveraendert: das WAV-Append selbst (Datei-I/O) und die Sperre des Sinks laufen im
//!   Callback-Thread.

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
/// Wie oft das Warten auf den Capture-Start den Abbruchmerker prueft.
const START_POLL: Duration = Duration::from_millis(25);
/// Wie lange ein Stopp bzw. Loeschen auf das Ende eines abgebrochenen Starts (oder einer
/// laufenden Aufnahme-Anzeige) wartet.
const START_CANCEL_WAIT: Duration = Duration::from_secs(12);
/// Takt, in dem solch ein Warten den Zustand neu liest.
const SETTLE_POLL: Duration = Duration::from_millis(5);
/// Wie oft der Schreib-Waechter die Merkzeichen der WAV-Writer liest.
const WRITE_WATCH_INTERVAL: Duration = Duration::from_millis(500);
/// Fehlercode: der Start wurde durch einen Stopp abgebrochen.
pub const START_CANCELLED: &str = "start_cancelled";
/// Fehlercode: die Besprechung ist (mittlerweile) geloescht.
const MEETING_NOT_FOUND: &str = "meeting_not_found";
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
/// des ganzen Starts (Stopp, Loeschen). Endet es ohne Capture (Frist, Abbruch), setzt es
/// `silenced`: der Callback dieser Capture schreibt dann nichts mehr, auch wenn der
/// Treiber spaeter doch noch zurueckkehrt und den Strom startet (lva#538). Das Ergebnis,
/// das nach der Frist kommt, faellt beim Senden aus dem Kanal und wird dabei verworfen
/// (`Drop` stoppt eine Capture).
pub(crate) fn start_bounded<T: Send + 'static>(
    thread_name: &str,
    timeout: Duration,
    cancel: &AtomicBool,
    silenced: &AtomicBool,
    start: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, LoopbackStartError> {
    if cancel.load(Ordering::Acquire) {
        silenced.store(true, Ordering::Release);
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
                // Ein Abbruch, der gleichzeitig kam: die Capture faellt hier (Drop stoppt sie).
                if cancel.load(Ordering::Acquire) {
                    silenced.store(true, Ordering::Release);
                    return Err(LoopbackStartError::Cancelled);
                }
                return Ok(started);
            }
            Ok(Err(e)) => return Err(LoopbackStartError::Failed(e)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::Acquire) {
                    silenced.store(true, Ordering::Release);
                    return Err(LoopbackStartError::Cancelled);
                }
                if Instant::now() >= deadline {
                    silenced.store(true, Ordering::Release);
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
    remove_capture_files(dir);
    if let Err(e) = store.set_audio_paths(meeting_id, None, None, None) {
        warn!("meetings: failed start of {meeting_id}: audio paths not cleared: {e}");
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

/// Der Start, der gerade laeuft (zwischen `begin_start` und dem Ende seines Tickets).
struct StartInfo {
    /// Abbruchmerker: Stopp und Loeschen setzen ihn; `publish_started` veroeffentlicht dann
    /// nichts mehr, und jeder Schritt des Starts prueft ihn.
    cancel: Arc<AtomicBool>,
    /// Gesetzt, wenn der Abbruch vom Loeschen der Besprechung kam (Fehlercode
    /// `meeting_not_found` statt `start_cancelled`).
    deleted: Arc<AtomicBool>,
    /// Die Besprechung, in die der Start aufnimmt, sobald bekannt (Loeschen vergleicht).
    meeting_id: Option<String>,
}

/// Die Zustandsregeln des Recorders mit der Sitzung als Typparameter, damit sie
/// ohne Mikrofon, Modell und `AppHandle` pruefbar sind. Reihenfolge der Sperren
/// ueberall: erst `state`, dann `session`, dann `start`.
///
/// Der Zustand wechselt NACH dem Capture-Start auf `Recording` und erst NACH dem
/// Capture-Stopp zurueck auf `Idle` (#15): solange ein Mikrofon offen ist, gilt
/// die Aufnahme als laufend, fuer Diktat-Hotkey, neuen Start und Pause.
///
/// lva#538: ein Stopp oder Loeschen WAEHREND des Starts (noch keine Sitzung) wird nicht
/// mehr als "nicht aufgenommen" abgewiesen, sondern setzt den Abbruchmerker des Starts;
/// `publish_started` prueft ihn unter denselben Sperren wie `begin_stop`: entweder sieht
/// der Stopp die Sitzung, oder die Veroeffentlichung sieht den Abbruch.
pub(crate) struct RunCore<S> {
    state: Mutex<MeetingRunState>,
    session: Mutex<Option<S>>,
    starting: AtomicBool,
    start: Mutex<Option<StartInfo>>,
}

/// Was ein Stopp (oder das Loeschen) vorfindet.
pub(crate) enum StopStep<'a, S> {
    /// Eine laufende Aufnahme: ihre Sitzung und das Ticket (`Idle` erst mit dessen Ende).
    Running(S, StopTicket<'a, S>),
    /// Es lief nur ein Start (noch keine Sitzung): sein Abbruchmerker ist gesetzt, er
    /// raeumt selbst auf und endet mit einem Fehlercode.
    CancelledStart,
    /// Die Aufnahme ist veroeffentlicht, ihr Start raeumt aber noch auf (Anzeige,
    /// Ereignis), oder ein anderer Stopp laeuft: kurz warten und erneut fragen. Sonst
    /// schaltete der Start die Anzeige NACH dem Stopp wieder ein.
    Settling,
}

impl<S> RunCore<S> {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(MeetingRunState::Idle),
            session: Mutex::new(None),
            starting: AtomicBool::new(false),
            start: Mutex::new(None),
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
    /// Fehlerweg) und traegt den Abbruchmerker dieses Starts.
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
        let cancel = Arc::new(AtomicBool::new(false));
        let deleted = Arc::new(AtomicBool::new(false));
        *lock_recovering(&self.start) = Some(StartInfo {
            cancel: Arc::clone(&cancel),
            deleted: Arc::clone(&deleted),
            meeting_id: None,
        });
        Ok(StartingFlag {
            starting: &self.starting,
            info: &self.start,
            cancel,
            deleted,
        })
    }

    /// Wurde der laufende Start abgebrochen?
    fn start_cancelled(&self) -> bool {
        lock_recovering(&self.start)
            .as_ref()
            .is_some_and(|info| info.cancel.load(Ordering::Acquire))
    }

    /// Setzt den Abbruchmerker des laufenden Starts, wenn es einen gibt (und, mit
    /// `only_for`, wenn er in diese Besprechung aufnimmt). `true`, wenn gesetzt.
    fn cancel_start(&self, only_for: Option<&str>) -> bool {
        let start = lock_recovering(&self.start);
        let Some(info) = start.as_ref() else {
            return false;
        };
        if only_for.is_some_and(|id| info.meeting_id.as_deref() != Some(id)) {
            return false;
        }
        if only_for.is_some() {
            info.deleted.store(true, Ordering::Release);
        }
        info.cancel.store(true, Ordering::Release);
        true
    }

    /// Wartet, bis kein Start mehr laeuft (`false` nach `max`: er haengt noch).
    pub(crate) fn wait_start_end(&self, max: Duration) -> bool {
        let deadline = Instant::now() + max;
        while self.starting.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(SETTLE_POLL);
        }
        true
    }

    /// Die Capture laeuft: Zustand UND Sitzung unter beiden Sperren setzen. `Err`
    /// gibt die Sitzung zurueck, wenn der Start inzwischen abgebrochen wurde (Stopp,
    /// Loeschen): dann wird nichts veroeffentlicht, der Aufrufer raeumt sie ab.
    pub(crate) fn publish_started(&self, meeting_id: &str, session: S) -> Result<(), S> {
        // Beide Sperren zusammen: ein Stopp dazwischen sah vorher die Sitzung, aber
        // noch `Idle`, stellte `Idle` ein, und der Start setzte danach `Recording`
        // ohne Sitzung (Geisteraufnahme).
        let mut state = lock_recovering(&self.state);
        let mut slot = lock_recovering(&self.session);
        if self.start_cancelled() {
            return Err(session);
        }
        *state = MeetingRunState::Recording {
            meeting_id: meeting_id.to_string(),
            paused: false,
        };
        *slot = Some(session);
        Ok(())
    }

    /// Stopp beginnen: die Sitzung herausnehmen. Der Zustand bleibt `Recording`,
    /// bis das Ticket beendet wird (`finish`) oder faellt (auch bei Panik). Ohne
    /// Sitzung, aber mit laufendem Start: der Start wird abgebrochen (lva#538).
    pub(crate) fn begin_stop(&self) -> Result<StopStep<'_, S>, String> {
        let _state = lock_recovering(&self.state);
        let mut slot = lock_recovering(&self.session);
        if slot.is_some() && self.starting.load(Ordering::Acquire) {
            return Ok(StopStep::Settling);
        }
        if let Some(session) = slot.take() {
            return Ok(StopStep::Running(session, StopTicket { core: self }));
        }
        if self.cancel_start(None) {
            return Ok(StopStep::CancelledStart);
        }
        Err("not_recording".to_string())
    }

    /// Loeschen der Besprechung `meeting_id`: gilt nur, wenn DIESE Besprechung gerade
    /// aufgenommen wird oder ein Start in sie aufnimmt; `None`, wenn nichts davon
    /// zutrifft (dann gibt es nichts zu stoppen).
    pub(crate) fn begin_delete(&self, meeting_id: &str) -> Option<StopStep<'_, S>> {
        let state = lock_recovering(&self.state);
        let mut slot = lock_recovering(&self.session);
        let recording_it = matches!(
            &*state,
            MeetingRunState::Recording { meeting_id: id, .. } if id == meeting_id
        );
        if recording_it {
            if slot.is_none() {
                // Ein Stopp hat die Sitzung schon genommen und laeuft noch.
                return Some(StopStep::Settling);
            }
            if self.starting.load(Ordering::Acquire) {
                return Some(StopStep::Settling);
            }
            return slot
                .take()
                .map(|session| StopStep::Running(session, StopTicket { core: self }));
        }
        self.cancel_start(Some(meeting_id))
            .then_some(StopStep::CancelledStart)
    }

    /// Pause/Weiter: nur mit laufender Sitzung. `apply` setzt das Merkzeichen der
    /// Capture-Callbacks unter denselben Sperren.
    pub(crate) fn set_paused(
        &self,
        paused: bool,
        apply: impl FnOnce(&S),
    ) -> Result<String, String> {
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

/// Everything a running meeting owns. Dropped as a unit by `stop()`.
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
}

/// M2-P2d: der laufende Enddurchlauf-/Recovery-Thread.
struct FinalJobHandle {
    cancel: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

/// Was ein Start an laufenden Captures und der Pipeline zusammenbringt.
struct Captures {
    mic_capture: Option<MeetingMicCapture>,
    loopback: Option<LoopbackCapture>,
    pipeline: LivePipeline,
}

/// Setzt `starting` fuer die Dauer von `start()` (auch bei jedem Fehlerweg) und traegt
/// den Abbruchmerker dieses Starts.
pub(crate) struct StartingFlag<'a> {
    starting: &'a AtomicBool,
    info: &'a Mutex<Option<StartInfo>>,
    cancel: Arc<AtomicBool>,
    deleted: Arc<AtomicBool>,
}

impl StartingFlag<'_> {
    /// Der Abbruchmerker: bis in die Capture-Callbacks und das Warten auf den Treiber.
    pub(crate) fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Wurde der Start abgebrochen (Stopp, Loeschen)?
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// Der Fehlercode eines abgebrochenen Starts: das Loeschen der Besprechung heisst
    /// `meeting_not_found`, ein Stopp `start_cancelled`.
    pub(crate) fn cancel_code(&self) -> &'static str {
        if self.deleted.load(Ordering::Acquire) {
            MEETING_NOT_FOUND
        } else {
            START_CANCELLED
        }
    }

    /// `Err(code)`, wenn der Start abgebrochen wurde: vor jedem Schritt, der etwas anlegt.
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err(self.cancel_code().to_string())
        } else {
            Ok(())
        }
    }

    /// Merkt die Besprechung, in die dieser Start aufnimmt (das Loeschen sucht sie).
    pub(crate) fn set_target(&self, meeting_id: &str) {
        if let Some(info) = lock_recovering(self.info).as_mut() {
            info.meeting_id = Some(meeting_id.to_string());
        }
    }
}

impl Drop for StartingFlag<'_> {
    fn drop(&mut self) {
        *lock_recovering(self.info) = None;
        self.starting.store(false, Ordering::Release);
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
    final_job: Mutex<Option<FinalJobHandle>>,
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
            final_job: Mutex::new(None),
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
        let mut slot = self.final_job.lock().unwrap_or_else(|e| e.into_inner());
        let previous = slot.take();
        let cancel = match &previous {
            Some(p) if !p.handle.is_finished() => Arc::clone(&p.cancel),
            _ => Arc::new(AtomicBool::new(false)),
        };
        let app = self.app.clone();
        let store = Arc::clone(&self.store);
        let tm = Arc::clone(&self.transcription);
        let job_cancel = Arc::clone(&cancel);
        let thread_jobs = jobs.clone();
        let spawned = std::thread::Builder::new()
            .name("meeting-final-pass".to_string())
            .spawn(move || {
                if let Some(previous) = previous {
                    let _ = previous.handle.join();
                }
                let mut env =
                    final_pass::AppEnv::new(&app, Arc::clone(&tm), Arc::clone(&job_cancel));
                for job in &thread_jobs {
                    final_pass::run_job(&store, job, &mut env);
                }
                if !job_cancel.load(Ordering::Relaxed) {
                    let dictation_model = crate::settings::get_settings(&app).selected_model;
                    tm.initiate_model_load_target(&dictation_model);
                }
            });
        match spawned {
            Ok(handle) => *slot = Some(FinalJobHandle { cancel, handle }),
            Err(e) => {
                drop(slot);
                // Ohne Thread kein Enddurchlauf, aber auch kein haengendes
                // `processing`: jede Besprechung wird hier abgeschlossen.
                warn!("meetings: final pass thread not started ({e}) - live transcripts stay");
                let mut env = final_pass::AppEnv::new(
                    &self.app,
                    Arc::clone(&self.transcription),
                    Arc::new(AtomicBool::new(false)),
                );
                for mut job in jobs {
                    job.plan = FinalPlan::Keep(KeepReason::LoadFailed);
                    job.catch_up_model = None;
                    final_pass::run_job(&self.store, &job, &mut env);
                }
            }
        }
    }

    /// M2-P2d: eine neue Aufnahme braucht die Engine. Der Enddurchlauf hoert
    /// nach dem laufenden Segment auf (das Live-Transkript bleibt, die
    /// Besprechung wird `ready`), und erst dann geht es weiter.
    fn cancel_final_jobs(&self) {
        let job = self
            .final_job
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(job) = job {
            if !job.handle.is_finished() {
                info!("meetings: a new recording cancels the running final pass");
            }
            job.cancel.store(true, Ordering::Relaxed);
            if job.handle.join().is_err() {
                error!("meetings: final pass thread panicked");
            }
        }
    }

    /// U7: laeuft der Enddurchlauf-/Recovery-Thread noch? Die Import-Warteschlange
    /// beginnt dann nicht auf der gemeinsamen Engine (auch nicht in der Luecke,
    /// bevor sein Auftrag im Verzeichnis steht).
    pub fn final_jobs_active(&self) -> bool {
        self.final_job
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|job| !job.handle.is_finished())
    }

    /// M2-P2d: wartet auf den Enddurchlauf-/Recovery-Thread (Headless-Laeufe,
    /// deren Prozess sonst vor dem Ende des Auftrags endete).
    pub fn wait_final_jobs(&self) {
        let job = self
            .final_job
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(job) = job {
            if job.handle.join().is_err() {
                error!("meetings: final pass thread panicked");
            }
        }
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
        consent_gate(consent_confirmed)?;
        // `Mutex<()>` traegt keine Daten: eine Vergiftung hiesse sonst, dass nie
        // wieder eine Aufnahme startet.
        let _start_guard = lock_recovering(&self.start_guard);
        let starting = self.core.begin_start()?;
        if let Some(target) = target_meeting_id {
            starting.set_target(target);
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

        // M2-P2d: ein laufender Enddurchlauf gibt die Engine vorher frei.
        self.cancel_final_jobs();

        let consent_at = chrono::Utc::now().timestamp();
        let meeting = begin_live_row(&self.store, &title, consent_at, target_meeting_id)?;
        let meeting_id = meeting.id.clone();
        starting.set_target(&meeting_id);
        let started = self.start_row(meeting, CaptureMode::Meeting { capture_system }, &starting);
        if let Err(e) = &started {
            match target_meeting_id {
                Some(target) => {
                    // Nichts wurde aufgenommen: der Eintrag wird wieder leer.
                    match self.store.restore_empty_meeting(target) {
                        Ok(true) => {
                            info!("meetings: Start gescheitert ({e}), Eintrag {target} ist wieder leer");
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
                        discard_failed_start(&self.store, &meeting_id, &base.join(&meeting_id));
                    }
                    Err(err) => {
                        warn!("meetings: Start gescheitert, Datenordner unbekannt ({err})");
                        let _ = self.store.set_status(&meeting_id, MeetingStatus::Failed);
                    }
                },
            }
        }
        started
    }

    /// #537: nimmt den Systemton (NUR den, kein Mikrofon) in die vorhandene
    /// YouTube-Besprechung `meeting_id` auf, solange der Nutzer das Video im
    /// eingebetteten Player abspielt. Titel, Quelle, Notizen und Projekte bleiben;
    /// Transkript, Zeitachse und Stopp wie bei jeder Aufnahme. Die Einwilligung gilt
    /// zuerst (der Nutzer bestaetigt im Dialog, dass er das Video mitschneiden darf).
    ///
    /// Fehler: `consent_required`, `already_recording`, `dictation_active`,
    /// `meeting_not_found` (auch: waehrend des Starts geloescht), `target_not_empty`
    /// (kein YouTube-Eintrag ohne Transkript und ohne Aufnahme: nichts startet, nichts
    /// aendert sich), `playback_capture_failed` / `playback_capture_timeout` (der
    /// Systemton liess sich nicht oeffnen), `start_cancelled` (ein Stopp kam
    /// dazwischen). Nach jedem Fehler nach der Umwandlung ist der Eintrag wieder wie
    /// vorher (`undo_playback_start`).
    pub fn start_playback_capture(
        &self,
        meeting_id: &str,
        consent_confirmed: bool,
    ) -> Result<Meeting, String> {
        consent_gate(consent_confirmed)?;
        let _start_guard = lock_recovering(&self.start_guard);
        let starting = self.core.begin_start()?;
        starting.set_target(meeting_id);

        if let Some(rm) = self
            .app
            .try_state::<Arc<crate::managers::audio::AudioRecordingManager>>()
        {
            if rm.is_recording() {
                return Err("dictation_active".to_string());
            }
        }
        starting.check()?;

        // ERST die Zeile pruefen und umwandeln (atomar, samt Vorzustand), DANN den
        // Enddurchlauf einer anderen Besprechung abbrechen: eine abgelehnte Anfrage soll
        // nichts anrichten.
        let consent_at = chrono::Utc::now().timestamp();
        let meeting = self
            .store
            .begin_playback_capture(meeting_id, consent_at)
            .map_err(|e| coded_error("meeting_create_failed", &e))?;
        self.cancel_final_jobs();

        let started = self.start_row(meeting, CaptureMode::PlaybackOnly, &starting);
        if let Err(e) = &started {
            info!("meetings: Mitschnitt gescheitert ({e}), {meeting_id} wird zurueckgesetzt");
            self.undo_playback_start(meeting_id);
        }
        started
    }

    /// Der Mitschnitt kam nicht zustande: Dateien weg, Zeile wieder die YouTube-
    /// Besprechung von vorher (aus dem gespeicherten Vorzustand). Laesst sich die Zeile
    /// nicht zuruecksetzen, steht sie auf `failed` (mit dem Merker; der naechste
    /// App-Start setzt sie zurueck), nie auf `recording` (er belebt sie sonst als
    /// Absturzrest).
    fn undo_playback_start(&self, meeting_id: &str) {
        match super::meetings_data_dir(&self.app) {
            Ok(base) => remove_capture_files(&base.join(meeting_id)),
            Err(err) => warn!("meetings: Mitschnitt gescheitert, Datenordner unbekannt ({err})"),
        }
        match self.store.restore_playback_target(meeting_id) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                warn!("meetings: Eintrag {meeting_id} nicht wiederhergestellt, wird `failed`");
                let _ = self.store.set_status(meeting_id, MeetingStatus::Failed);
            }
        }
    }

    /// Der Teil des Starts nach dem Anlegen bzw. Umwandeln der Zeile.
    ///
    /// lva#538: der Start ist abbrechbar. Er prueft den Abbruchmerker (Stopp, Loeschen) vor
    /// jedem Schritt, der etwas anlegt; das Warten auf den Treiber prueft ihn im Takt;
    /// `publish_started` entscheidet unter den Sperren des Stopps. Scheitert oder endet er
    /// vor der Veroeffentlichung, schliesst er die WAV-Writer selbst (ein spaeter
    /// zurueckkehrender Treiber haelt die Datei sonst offen, das Loeschen scheiterte) und
    /// ersetzt die Callbacks durch stumme; der Aufrufer raeumt Zeile und Dateien auf.
    fn start_row(
        &self,
        meeting: Meeting,
        mode: CaptureMode,
        starting: &StartingFlag<'_>,
    ) -> Result<Meeting, String> {
        let meeting_id = meeting.id.clone();
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

        // Written now, not at stop(): crash recovery can only repair WAVs whose
        // paths it knows, and a crash is exactly the case where stop() never ran.
        // lva#538: ein Fehler hier ist ein Fehler des Starts (nicht mehr "nur loggen"):
        // ohne Pfade repariert die Wiederherstellung nichts, und fuer eine inzwischen
        // geloeschte Zeile gibt es keine Aufnahme.
        self.store
            .set_audio_paths(
                &meeting_id,
                mic_path.as_ref().and_then(|p| p.to_str()),
                system_path.as_ref().and_then(|p| p.to_str()),
                None,
            )
            .map_err(|e| audio_paths_error(&e))?;

        let paused = Arc::new(AtomicBool::new(false));
        let levels: Arc<dyn LevelSink> = Arc::new(LevelEmitter::new(self.app.clone()));
        let write_flags = Arc::new(WriteFlags::default());
        let watch_stop = Arc::new(AtomicBool::new(false));
        let mic_silenced = Arc::new(AtomicBool::new(false));
        let system_silenced = Arc::new(AtomicBool::new(false));

        let captured = (|| -> Result<Captures, String> {
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
            let mic_capture = match &mic_sink {
                Some(sink) => {
                    let mic_feed = pipeline.feed(CHANNEL_MIC).ok_or("dsp_thread_failed")?;
                    starting.check()?;
                    Some(
                        MeetingMicCapture::start(
                            crate::settings::get_settings(&self.app).selected_microphone,
                            mic_overflow,
                            channel_callback(
                                CHANNEL_MIC,
                                Arc::clone(sink),
                                Arc::clone(&paused),
                                Arc::clone(&mic_silenced),
                                Arc::clone(&write_flags),
                                mic_feed,
                                Arc::clone(&levels),
                            ),
                        )
                        .map_err(|e| {
                            // The meeting row exists but nothing was captured — mark it failed
                            // rather than leaving a phantom "recording" row behind.
                            let _ = self.store.set_status(&meeting_id, MeetingStatus::Failed);
                            format!("mic_start_failed: {e}")
                        })?,
                    )
                }
                None => None,
            };
            starting.check()?;

            let system_callback = |sink: &Arc<Mutex<ChannelSink>>, feed: ChannelFeed| {
                channel_callback(
                    CHANNEL_SYSTEM,
                    Arc::clone(sink),
                    Arc::clone(&paused),
                    Arc::clone(&system_silenced),
                    Arc::clone(&write_flags),
                    feed,
                    Arc::clone(&levels),
                )
            };
            let loopback = match (mode, &system_sink, system_feed) {
                // Mitschnitt: der Systemton ist alles, was es gibt. Scheitert er, scheitert
                // der Start (der Aufrufer setzt den Eintrag zurueck).
                (CaptureMode::PlaybackOnly, Some(sink), Some(feed)) => Some(self.start_loopback(
                    &meeting_id,
                    mode,
                    starting,
                    &system_silenced,
                    system_callback(sink, feed),
                    dsp_control.clone(),
                )?),
                (CaptureMode::PlaybackOnly, _, _) => return Err("dsp_thread_failed".to_string()),
                (
                    CaptureMode::Meeting {
                        capture_system: true,
                    },
                    Some(sink),
                    Some(feed),
                ) => self
                    .start_loopback(
                        &meeting_id,
                        mode,
                        starting,
                        &system_silenced,
                        system_callback(sink, feed),
                        dsp_control.clone(),
                    )
                    .ok(),
                _ => None,
            };
            if loopback.is_none() {
                // Ohne Referenz keine Echo-Unterdrueckung (No-op, wenn sie aus ist).
                if let Some(control) = &dsp_control {
                    control.reference_lost();
                }
            }
            Ok(Captures {
                mic_capture,
                loopback,
                pipeline,
            })
        })();
        let Captures {
            mic_capture,
            loopback,
            pipeline,
        } = match captured {
            Ok(captures) => captures,
            Err(e) => {
                for flag in [&mic_silenced, &system_silenced] {
                    flag.store(true, Ordering::Release);
                }
                close_sink(mic_sink.as_ref());
                close_sink(system_sink.as_ref());
                return Err(e);
            }
        };

        let session = RecordingSession {
            meeting_id: meeting_id.clone(),
            mode,
            paused,
            mic_capture,
            loopback,
            mic_sink,
            system_sink,
            mic_path,
            system_path,
            pipeline: Some(pipeline),
            write_flags: Arc::clone(&write_flags),
            watch_stop: Arc::clone(&watch_stop),
        };
        // Der letzte Augenblick vor der Veroeffentlichung: abgebrochen, oder die Zeile ist
        // inzwischen geloescht? Dann wird nichts daraus.
        if let Err(e) = starting
            .check()
            .and_then(|()| self.ensure_present(&meeting_id))
        {
            Self::teardown(session);
            return Err(e);
        }
        // Zustand und Sitzung zusammen und erst jetzt, wo die Captures laufen (#15). Ein
        // Stopp, der zwischen der Pruefung und hier kam, hat den Abbruchmerker gesetzt.
        if let Err(session) = self.core.publish_started(&meeting_id, session) {
            Self::teardown(session);
            return Err(starting.cancel_code().to_string());
        }

        self.set_indicator(true);
        self.emit_state(&meeting_id, "recording", false);
        self.watch_writes(&meeting_id, &write_flags, &watch_stop);
        info!("meetings: recording started ({meeting_id})");
        Ok(meeting)
    }

    /// Ist die Zeile noch da? (Geloescht waehrend des Starts: `meeting_not_found`.)
    fn ensure_present(&self, meeting_id: &str) -> Result<(), String> {
        match self.store.get_meeting(meeting_id) {
            Ok(None) => Err(MEETING_NOT_FOUND.to_string()),
            Ok(Some(_)) | Err(_) => Ok(()),
        }
    }

    /// Eine Sitzung, die nie veroeffentlicht wurde (Abbruch im letzten Augenblick): Captures
    /// zu, WAV-Writer zu (das Entfernen der Dateien folgt), Pipeline fallen lassen.
    fn teardown(session: RecordingSession) {
        let RecordingSession {
            mic_capture,
            loopback,
            mic_sink,
            system_sink,
            pipeline,
            watch_stop,
            ..
        } = session;
        watch_stop.store(true, Ordering::Release);
        if let Some(capture) = mic_capture {
            capture.stop();
        }
        if let Some(capture) = loopback {
            capture.stop();
        }
        close_sink(mic_sink.as_ref());
        close_sink(system_sink.as_ref());
        drop(pipeline);
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
        starting: &StartingFlag<'_>,
        silenced: &Arc<AtomicBool>,
        callback: impl FnMut(&[i16], Option<u64>) + Send + 'static,
        dsp_control: Option<DspControl>,
    ) -> Result<LoopbackCapture, String> {
        let cancel = starting.cancel_flag();
        let started = start_bounded(
            "meeting-loopback-start",
            LOOPBACK_START_TIMEOUT,
            &cancel,
            silenced,
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
        let deadline = Instant::now() + START_CANCEL_WAIT;
        let (session, stopping) = loop {
            match self.core.begin_stop()? {
                StopStep::Running(session, ticket) => break (session, ticket),
                // lva#538: ein Stopp WAEHREND des Starts bricht ihn ab; zurueck kehrt er erst,
                // wenn der Start aufgeraeumt hat (der Eintrag ist dann wie vorher).
                StopStep::CancelledStart => {
                    if !self.core.wait_start_end(START_CANCEL_WAIT) {
                        warn!("meetings: the cancelled start did not end in time");
                    }
                    return Ok(String::new());
                }
                StopStep::Settling if Instant::now() < deadline => std::thread::sleep(SETTLE_POLL),
                StopStep::Settling => return Err("start_busy".to_string()),
            }
        };
        let RecordingSession {
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
            ..
        } = session;
        watch_stop.store(true, Ordering::Release);

        // Captures first: once they are stopped no callback can touch the
        // sinks or the DSP queue any more, so draining and finalizing below is
        // race-free.
        self.stop_captures(&meeting_id, mic_capture, loopback);
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

        // lva#538: ein Schreibfehler macht die Aufnahme unvollstaendig. Der Marker steht
        // VOR dem Auftrag, damit der Enddurchlauf das Live-Transkript nicht aus ihr ersetzt.
        let failed = write_flags.failed_channels();
        if !failed.is_empty() {
            mark_audio_incomplete(&self.store, &meeting_id, &failed);
            if write_flags.first_report() {
                self.emit_error(&meeting_id, AUDIO_WRITE_FAILED);
            }
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
    fn stop_captures(
        &self,
        meeting_id: &str,
        mic_capture: Option<MeetingMicCapture>,
        loopback: Option<LoopbackCapture>,
    ) {
        if let Some(capture) = mic_capture {
            if capture.had_error() {
                self.emit_error(meeting_id, "mic_stream_error");
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

    /// lva#538: VOR dem Loeschen der Besprechung `meeting_id` (`meetings_delete`). Laeuft ein
    /// Start in sie, endet er mit `meeting_not_found` und raeumt auf (dieser Aufruf kehrt
    /// danach zurueck); wird sie gerade aufgenommen, wird die Aufnahme beendet, ohne
    /// Enddurchlauf (die Besprechung verschwindet gleich). Fuer jede andere Besprechung
    /// ist es ein leerer Aufruf. Blockierend.
    pub fn abort_for_delete(&self, meeting_id: &str) {
        let deadline = Instant::now() + START_CANCEL_WAIT;
        loop {
            match self.core.begin_delete(meeting_id) {
                None => return,
                Some(StopStep::CancelledStart) => {
                    if !self.core.wait_start_end(START_CANCEL_WAIT) {
                        warn!("meetings: the start of a deleted meeting did not end in time");
                    }
                    return;
                }
                Some(StopStep::Running(session, ticket)) => {
                    self.discard_running(session, ticket);
                    return;
                }
                Some(StopStep::Settling) if Instant::now() < deadline => {
                    std::thread::sleep(SETTLE_POLL)
                }
                Some(StopStep::Settling) => {
                    warn!("meetings: a recording being deleted did not settle in time");
                    return;
                }
            }
        }
    }

    /// Beendet eine laufende Aufnahme, deren Besprechung geloescht wird: Captures zu, die
    /// Pipeline leert ihre Queue (solange die Zeile noch da ist, sonst meldete der
    /// Worker spaeter einen Speicherfehler), WAV-Writer zu (die Dateien loescht der
    /// Aufrufer gleich danach), kein Status, kein Enddurchlauf.
    fn discard_running(
        &self,
        session: RecordingSession,
        stopping: StopTicket<'_, RecordingSession>,
    ) {
        let RecordingSession {
            meeting_id,
            mic_capture,
            loopback,
            mic_sink,
            system_sink,
            pipeline,
            write_flags,
            watch_stop,
            ..
        } = session;
        watch_stop.store(true, Ordering::Release);
        self.stop_captures(&meeting_id, mic_capture, loopback);
        stopping.finish();
        self.set_indicator(false);
        if let Some(pipeline) = pipeline {
            pipeline.drain();
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

    /// lva#538: der Waechter der WAV-Writer. Ein Schreibfehler (Platte voll, RIFF-Grenze)
    /// setzt im Callback nur ein Merkzeichen; hier wird daraus, noch waehrend der
    /// Aufnahme, der dauerhafte Marker `audio_incomplete` (ein Absturz danach verliert ihn
    /// nicht) und eine Meldung an den Nutzer. Endet mit der Aufnahme (`stop`) oder nach der
    /// ersten Meldung.
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
                        mark_audio_incomplete(&store, &meeting_id, &failed);
                        if flags.first_report() {
                            let _ = (MeetingEvent::Error {
                                meeting_id: meeting_id.clone(),
                                message: AUDIO_WRITE_FAILED.to_string(),
                            })
                            .emit(&app);
                        }
                        return;
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
        RecordingSession {
            meeting_id: "m-pos".to_string(),
            mode: CaptureMode::Meeting {
                capture_system: false,
            },
            paused: Arc::new(AtomicBool::new(false)),
            mic_capture: None,
            loopback: None,
            mic_sink: Some(Arc::new(Mutex::new(sink))),
            system_sink: None,
            mic_path: None,
            system_path: None,
            pipeline: None,
            write_flags: Arc::new(WriteFlags::default()),
            watch_stop: Arc::new(AtomicBool::new(false)),
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
/// lva#538: `silenced` (Abbruch des Starts, Frist des Treibers) macht den Callback stumm,
/// bevor er etwas anruehrt: ein Treiber, der nach der Frist doch noch zurueckkehrt und
/// Daten liefert, schreibt nichts mehr. Ein Schreibfehler der WAV setzt das Merkzeichen
/// in `write_flags` (Atomic) und verwirft den Writer; Aufnahme und Transkription laufen
/// weiter, der Fehler wird gemeldet und dauerhaft markiert, nicht verschluckt.
fn channel_callback(
    channel: u8,
    sink: Arc<Mutex<ChannelSink>>,
    paused: Arc<AtomicBool>,
    silenced: Arc<AtomicBool>,
    write_flags: Arc<WriteFlags>,
    mut feed: ChannelFeed,
    levels: Arc<dyn LevelSink>,
) -> impl FnMut(&[i16], Option<u64>) + Send + 'static {
    move |samples: &[i16], qpc: Option<u64>| {
        if silenced.load(Ordering::Acquire) {
            return;
        }
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
                write_flags.flag(channel).store(true, Ordering::Release);
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
        levels.level(channel, rms(samples));
    }
}

/// Schliesst den WAV-Writer eines Kanals, ohne den Kopf zu schreiben: der Start, der
/// nicht zustande kam, braucht die Datei nicht mehr, und ein offenes Handle haelt ihr
/// Entfernen (Windows) auf.
fn close_sink(sink: Option<&Arc<Mutex<ChannelSink>>>) {
    if let Some(sink) = sink {
        lock_recovering(sink).writer = None;
    }
}

/// Finalizes a channel's WAV and returns its duration in milliseconds. Ein Fehler beim
/// Abschluss setzt dasselbe Merkzeichen wie ein Schreibfehler (die Aufnahme ist dann nicht
/// vollstaendig).
fn finalize_sink(sink: &Arc<Mutex<ChannelSink>>, failed: &AtomicBool) -> Option<u64> {
    let writer = sink.lock().ok()?.writer.take()?;
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
/// Mehrfaches Schreiben ist unschaedlich (derselbe Schluessel).
pub(crate) fn mark_audio_incomplete(store: &MeetingStore, meeting_id: &str, channels: &[u8]) {
    let value = serde_json::json!({
        "channels": channels,
        "at": chrono::Utc::now().timestamp(),
    });
    if let Err(e) = store.set_metadata_key(meeting_id, final_pass::AUDIO_INCOMPLETE_KEY, value) {
        error!("meetings: audio_incomplete marker not stored ({meeting_id}): {e}");
    }
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
    let reachable = is_orphan_status(&meeting.status) || meeting.status == "failed";
    if meeting.source != super::playback::SOURCE_YOUTUBE || !reachable {
        return false;
    }
    let has_prior = store.playback_prior(&meeting.id).ok().flatten().is_some();
    if !super::playback::never_recorded(meeting, has_prior) {
        if has_prior && is_orphan_status(&meeting.status) {
            // Es gibt Ton: der Vorzustand ist erledigt.
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

    /// Ein Stopp, der eine laufende Aufnahme vorfindet (sonst schlaegt der Test fehl).
    fn running<'a>(core: &'a Core) -> (&'static str, StopTicket<'a, &'static str>) {
        match core.begin_stop() {
            Ok(StopStep::Running(session, ticket)) => (session, ticket),
            Ok(StopStep::CancelledStart) => panic!("kein Start, sondern eine Aufnahme erwartet"),
            Ok(StopStep::Settling) => panic!("der Start raeumt noch auf"),
            Err(e) => panic!("Stopp abgewiesen: {e}"),
        }
    }

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
        core.publish_started("m1", "sitzung").unwrap();
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
        core.publish_started("m1", "sitzung").unwrap();
        let (session, ticket) = running(&core);
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
        core.publish_started("m1", "sitzung").unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (_session, _ticket) = running(&core);
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
        let starting = core.begin_start().unwrap();
        assert_eq!(
            core.set_paused(true, |_| {}),
            Err("not_recording".to_string())
        );
        core.publish_started("m1", "sitzung").unwrap();
        drop(starting);
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
        let (_s, ticket) = running(&core);
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
                let _ = starter.publish_started("m1", "sitzung");
            });
            let stop = std::thread::spawn(move || {
                // Mehrere Versuche, damit mindestens einer nach dem Start liegt.
                for _ in 0..50 {
                    if let Ok(StopStep::Running(_s, ticket)) = stopper.begin_stop() {
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
                let (_s, ticket) = running(&core);
                ticket.finish();
            }
            assert!(
                !core.is_recording(),
                "Runde {round}: Zustand Recording ohne Sitzung (Geisteraufnahme)"
            );
        }
    }

    // ---- lva#538: Stopp und Loeschen waehrend des Starts ----------------------------

    /// Ein Stopp, der waehrend des Starts kommt (noch keine Sitzung), wird nicht mehr als
    /// "nicht aufgenommen" abgewiesen: er setzt den Abbruchmerker, und die Veroeffentlichung
    /// danach gibt die Sitzung zurueck, statt eine Aufnahme zu veroeffentlichen, die der
    /// Nutzer schon abgebrochen hat.
    #[test]
    fn a_stop_during_a_start_cancels_it_and_nothing_is_published() {
        let core = Core::new();
        let starting = core.begin_start().unwrap();
        assert!(!starting.cancelled());

        assert!(matches!(core.begin_stop(), Ok(StopStep::CancelledStart)));
        assert!(starting.cancelled(), "der Start sieht den Abbruch");
        assert_eq!(starting.check(), Err(START_CANCELLED.to_string()));

        // Der Start war schon bei der Veroeffentlichung: er bekommt seine Sitzung zurueck.
        assert_eq!(core.publish_started("m1", "sitzung"), Err("sitzung"));
        assert!(
            core.with_session(|s| s.is_none()),
            "keine Sitzung veroeffentlicht"
        );
        assert!(
            core.is_idle(),
            "der Zustand blieb Idle: keine Geisteraufnahme"
        );

        drop(starting);
        assert!(!core.is_recording());
        // Ohne Start und ohne Aufnahme bleibt es bei `not_recording`.
        assert_eq!(core.begin_stop().err(), Some("not_recording".to_string()));
        // Der naechste Start ist unberuehrt vom alten Abbruch.
        let next = core.begin_start().unwrap();
        assert!(!next.cancelled());
        assert!(core.publish_started("m2", "neu").is_ok());
    }

    /// Stopp und Veroeffentlichung im Wettlauf (lva#538): entweder sieht der Stopp die
    /// Sitzung (und kann sie beenden), oder die Veroeffentlichung sieht den Abbruch. Nie
    /// laeuft eine Aufnahme weiter, obwohl der Stopp "erfolgreich" war.
    #[test]
    fn a_stop_racing_the_publish_either_stops_the_recording_or_cancels_the_start() {
        for round in 0..300 {
            let core = Arc::new(Core::new());
            let starting = core.begin_start().unwrap();
            let stopper = Arc::clone(&core);
            let stop = std::thread::spawn(move || {
                matches!(stopper.begin_stop(), Ok(StopStep::CancelledStart))
            });
            let published = core.publish_started("m1", "sitzung").is_ok();
            let cancelled_start = stop.join().unwrap();
            drop(starting);
            if published {
                // Der Stopp kam NACH der Veroeffentlichung: die Sitzung ist da und stoppbar.
                assert!(!cancelled_start, "Runde {round}");
                let (_s, ticket) = running(&core);
                ticket.finish();
            } else {
                assert!(cancelled_start, "Runde {round}: abgelehnt ohne Abbruch");
            }
            assert!(!core.is_recording(), "Runde {round}: Geisteraufnahme");
        }
    }

    /// Eine Aufnahme, deren Start noch aufraeumt (Anzeige, Ereignis), wird nicht schon
    /// gestoppt: sonst schaltete der Start die Anzeige NACH dem Stopp wieder ein.
    #[test]
    fn a_stop_right_after_the_publish_waits_for_the_start_to_finish() {
        let core = Core::new();
        let starting = core.begin_start().unwrap();
        core.publish_started("m1", "sitzung").unwrap();
        assert!(matches!(core.begin_stop(), Ok(StopStep::Settling)));
        assert!(matches!(core.begin_delete("m1"), Some(StopStep::Settling)));
        assert!(
            core.with_session(|s| s.is_some()),
            "die Sitzung blieb stehen"
        );
        drop(starting);
        let (_s, ticket) = running(&core);
        ticket.finish();
    }

    #[test]
    fn a_delete_cancels_only_the_start_of_that_meeting() {
        let core = Core::new();
        let starting = core.begin_start().unwrap();
        starting.set_target("a");

        assert!(core.begin_delete("b").is_none(), "eine fremde Besprechung");
        assert!(!starting.cancelled());

        assert!(matches!(
            core.begin_delete("a"),
            Some(StopStep::CancelledStart)
        ));
        assert!(starting.cancelled());
        assert_eq!(
            starting.cancel_code(),
            "meeting_not_found",
            "geloescht, nicht gestoppt"
        );
        assert_eq!(core.publish_started("a", "sitzung"), Err("sitzung"));
        drop(starting);
        assert!(core.begin_delete("a").is_none(), "nichts mehr zu tun");
    }

    #[test]
    fn a_stop_gives_the_start_the_code_start_cancelled_not_meeting_not_found() {
        let core = Core::new();
        let starting = core.begin_start().unwrap();
        starting.set_target("a");
        assert!(matches!(core.begin_stop(), Ok(StopStep::CancelledStart)));
        assert_eq!(starting.cancel_code(), START_CANCELLED);
    }

    #[test]
    fn a_delete_takes_the_running_session_of_that_meeting_only() {
        let core = Core::new();
        core.publish_started("m1", "sitzung").unwrap();
        assert!(
            core.begin_delete("m2").is_none(),
            "andere Besprechung: nichts"
        );
        assert!(core.with_session(|s| s.is_some()));

        let Some(StopStep::Running(session, ticket)) = core.begin_delete("m1") else {
            panic!("die laufende Aufnahme gehoert zu m1");
        };
        assert_eq!(session, "sitzung");
        // Bis das Ticket endet, gilt die Aufnahme als laufend (Captures noch offen).
        assert!(core.is_recording());
        // Ein zweites Loeschen waehrend des Stopps wartet, statt zu stoerren.
        assert!(matches!(core.begin_delete("m1"), Some(StopStep::Settling)));
        ticket.finish();
        assert!(!core.is_recording());
        assert!(core.begin_delete("m1").is_none());
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

    /// Frische Merker fuer einen Start: (Abbruch des ganzen Starts, Stummschaltung des Kanals).
    fn flags() -> (Arc<AtomicBool>, Arc<AtomicBool>) {
        (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[test]
    fn a_bounded_start_returns_the_started_capture() {
        let (cancel, silenced) = flags();
        let got = start_bounded("t-ok", Duration::from_secs(5), &cancel, &silenced, || {
            Ok::<_, String>(42u8)
        });
        assert_eq!(got, Ok(42));
        assert!(
            !silenced.load(Ordering::SeqCst),
            "ein Erfolg schweigt nicht"
        );
    }

    /// Kein Ausgabegeraet (oder es laesst sich nicht oeffnen): der Fehler des
    /// Treibers kommt an, nicht nach einer Frist, sondern sofort.
    #[test]
    fn a_failing_start_reports_its_error_at_once() {
        let (cancel, silenced) = flags();
        let started = Instant::now();
        let got = start_bounded("t-err", Duration::from_secs(30), &cancel, &silenced, || {
            Err::<u8, _>("kein Wiedergabegeraet".to_string())
        });
        assert_eq!(
            got,
            Err(LoopbackStartError::Failed("kein Wiedergabegeraet".into()))
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// Haengt der Treiber, endet das Warten nach der Frist, und eine Capture, die
    /// spaeter doch noch aufgeht, wird verworfen (ihr `Drop` stoppt sie): sie laeuft
    /// nie unbemerkt weiter. Der Kanal ist ab der Frist stumm (lva#538).
    #[test]
    fn a_wedged_start_times_out_and_the_late_capture_is_dropped() {
        struct Capture(Arc<AtomicBool>);
        impl Drop for Capture {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let (cancel, silenced) = flags();
        let dropped = Arc::new(AtomicBool::new(false));
        let late = Arc::clone(&dropped);
        let got = start_bounded(
            "t-wedged",
            Duration::from_millis(50),
            &cancel,
            &silenced,
            move || {
                std::thread::sleep(Duration::from_millis(400));
                Ok::<_, String>(Capture(late))
            },
        );
        assert!(matches!(got, Err(LoopbackStartError::Timeout)));
        assert!(
            silenced.load(Ordering::SeqCst),
            "nach der Frist darf der Callback nichts mehr schreiben"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dropped.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            dropped.load(Ordering::SeqCst),
            "die spaete Capture lief weiter"
        );
    }

    /// lva#538: ein Stopp waehrend des Wartens beendet es sofort (nicht erst nach der
    /// Frist von fuenf Sekunden) und macht den Kanal stumm; die spaete Capture wird
    /// verworfen.
    #[test]
    fn a_cancel_ends_the_wait_at_once_and_silences_the_late_capture() {
        let (cancel, silenced) = flags();
        let canceller = Arc::clone(&cancel);
        let _ = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            canceller.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let got = start_bounded(
            "t-cancel",
            Duration::from_secs(30),
            &cancel,
            &silenced,
            || {
                std::thread::sleep(Duration::from_millis(600));
                Ok::<_, String>(1u8)
            },
        );
        assert_eq!(got, Err(LoopbackStartError::Cancelled));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "der Abbruch wartet nicht auf den Treiber ({:?})",
            started.elapsed()
        );
        assert!(silenced.load(Ordering::SeqCst));
    }

    /// Schon abgebrochen, bevor das Warten beginnt: es startet nicht einmal den Treiber.
    #[test]
    fn a_start_that_was_cancelled_before_it_began_never_touches_the_driver() {
        let (cancel, silenced) = flags();
        cancel.store(true, Ordering::SeqCst);
        let touched = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&touched);
        let got = start_bounded(
            "t-early",
            Duration::from_secs(5),
            &cancel,
            &silenced,
            move || {
                flag.store(true, Ordering::SeqCst);
                Ok::<_, String>(())
            },
        );
        assert_eq!(got, Err(LoopbackStartError::Cancelled));
        std::thread::sleep(Duration::from_millis(100));
        assert!(!touched.load(Ordering::SeqCst));
        assert!(silenced.load(Ordering::SeqCst));
    }

    /// Ein Treiber, der im Start abstuerzt, ist ein Fehler, kein Haenger.
    #[test]
    fn a_panicking_start_is_a_failure_not_a_hang() {
        let (cancel, silenced) = flags();
        let started = Instant::now();
        let got = start_bounded(
            "t-panic",
            Duration::from_secs(30),
            &cancel,
            &silenced,
            || -> Result<u8, String> { panic!("treiber") },
        );
        assert!(matches!(got, Err(LoopbackStartError::Failed(_))));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    // ---- lva#538: der echte Callback nach der Frist, Schreibfehler -----------------------

    /// Pegel, die ein Test mitzaehlt statt ein Ereignis zu senden.
    #[derive(Default)]
    struct CountingLevels(std::sync::atomic::AtomicUsize);

    impl LevelSink for CountingLevels {
        fn level(&self, _channel: u8, _rms: f32) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    type RigCallback = Box<dyn FnMut(&[i16], Option<u64>) + Send>;

    struct CallbackRig {
        _dir: tempfile::TempDir,
        path: PathBuf,
        sink: Arc<Mutex<ChannelSink>>,
        silenced: Arc<AtomicBool>,
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
        let silenced = Arc::new(AtomicBool::new(false));
        let flags = Arc::new(WriteFlags::default());
        let levels = Arc::new(CountingLevels::default());
        let callback = channel_callback(
            CHANNEL_SYSTEM,
            Arc::clone(&sink),
            Arc::new(AtomicBool::new(false)),
            Arc::clone(&silenced),
            Arc::clone(&flags),
            pipeline.feed(CHANNEL_SYSTEM).unwrap(),
            Arc::clone(&levels) as Arc<dyn LevelSink>,
        );
        CallbackRig {
            _dir: dir,
            path,
            sink,
            silenced,
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
        let (cancel, _) = flags();
        let mut callback = std::mem::replace(&mut rig.callback, Box::new(|_, _| {}));
        let late = Arc::new(Mutex::new(Vec::<usize>::new()));
        let seen = Arc::clone(&late);
        let got = start_bounded(
            "t-late-callback",
            Duration::from_millis(50),
            &cancel,
            &rig.silenced,
            move || {
                // Der Treiber kommt zurueck und liefert, wie ein laufender Strom, Bloecke.
                std::thread::sleep(Duration::from_millis(300));
                for _ in 0..5 {
                    callback(&vec![500i16; 1_600], Some(1));
                    seen.lock().unwrap().push(1_600);
                }
                Ok::<_, String>(())
            },
        );
        assert!(matches!(got, Err(LoopbackStartError::Timeout)));
        // Warten, bis die spaete Aktivitaet wirklich gelaufen ist.
        let deadline = Instant::now() + Duration::from_secs(5);
        while late.lock().unwrap().len() < 5 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            late.lock().unwrap().len(),
            5,
            "Voraussetzung: der Callback lief"
        );

        assert_eq!(
            written_samples(&rig),
            Some(0),
            "nach der Frist wurde in die WAV geschrieben"
        );
        assert_eq!(rig.levels.0.load(Ordering::SeqCst), 0, "auch kein Pegel");
        assert!(rig.flags.failed_channels().is_empty());
        let stats = rig.pipeline.stats();
        assert_eq!(
            stats.overflow_samples[CHANNEL_SYSTEM as usize].load(Ordering::Relaxed),
            0
        );

        // Der Start schliesst die Datei selbst und kann sie dann entfernen.
        close_sink(Some(&rig.sink));
        assert!(lock_recovering(&rig.sink).writer.is_none());
        remove_capture_files(rig.path.parent().unwrap());
        assert!(!rig.path.exists(), "die Datei liess sich entfernen");
    }

    /// Das Gegenstueck: ohne Abbruch schreibt derselbe Callback, zaehlt Pegel und
    /// erzeugt keinen Fehler. (Sonst bewiese der Test oben nichts.)
    #[test]
    fn the_same_callback_writes_when_it_is_not_silenced() {
        let mut rig = callback_rig();
        (rig.callback)(&vec![500i16; 1_600], Some(1));
        assert_eq!(written_samples(&rig), Some(1_600));
        assert_eq!(rig.levels.0.load(Ordering::SeqCst), 1);
        assert!(rig.flags.failed_channels().is_empty());
        rig.silenced.store(true, Ordering::SeqCst);
        (rig.callback)(&vec![500i16; 1_600], Some(2));
        assert_eq!(written_samples(&rig), Some(1_600), "stumm = nichts mehr");
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
            rig.levels.0.load(Ordering::SeqCst),
            1,
            "die Aufnahme laeuft fuer die Transkription weiter"
        );
        // Weitere Bloecke bleiben ohne neuen Fehler und ohne Schreiben.
        (rig.callback)(&vec![500i16; 1_600], Some(2));
        assert_eq!(rig.levels.0.load(Ordering::SeqCst), 2);
        // Das Merkzeichen meldet nur einmal.
        assert!(rig.flags.first_report());
        assert!(!rig.flags.first_report());
    }

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

        mark_audio_incomplete(&store, &meeting.id, &[CHANNEL_SYSTEM]);
        mark_audio_incomplete(&store, &meeting.id, &[CHANNEL_MIC, CHANNEL_SYSTEM]);

        let meta = store.metadata_json(&meeting.id).unwrap().unwrap();
        assert_eq!(
            meta[final_pass::AUDIO_INCOMPLETE_KEY]["channels"],
            serde_json::json!([0, 1])
        );
        assert_eq!(meta["timeline"]["basis"], "qpc");
        // Fuer eine geloeschte Zeile: kein Absturz.
        mark_audio_incomplete(&store, "gibt-es-nicht", &[CHANNEL_MIC]);
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
}
