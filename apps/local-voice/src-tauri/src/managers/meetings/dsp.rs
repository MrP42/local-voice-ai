//! M2 / P2a: der Live-Pfad einer Besprechung zwischen Capture und Ablage.
//!
//! ```text
//! Capture-Callback (je Kanal) ─ ChannelFeed ─ bounded Queue ─► "meeting-dsp"-Thread
//!     WAV wird VOR der Uebergabe roh geschrieben       VAD-Segmentierer je Kanal
//!                                                      (oder 20-s-Chunker als Rueckfall)
//!                                                                │ WorkItem
//!                                                                ▼
//!                                          "meeting-transcribe"-Worker (EIN Worker, FIFO)
//!                                          STT -> Halluzinationsfilter -> append_delta -> Event
//! ```
//!
//! Echtzeitregeln (`mic_capture.rs:7-11`): der Capture-Callback ruft nur
//! `ChannelFeed::push`. Das ist ein `try_send` auf eine begrenzte Queue: kein
//! Warten, kein Lock, kein I/O, kein VAD. Ist die Queue voll, werden die Samples
//! verworfen, GEZAEHLT und mit der naechsten erfolgreichen Nachricht als Luecke
//! (`gap_before`) gemeldet, damit die Segment-Zeitachse mit der WAV im Gleichlauf
//! bleibt. Die WAV selbst ist davon nicht betroffen (sie wird vorher geschrieben).
//! Gemeldet wird auf dem DSP-Thread (Log + `DspNotice`), nie im Callback.
//!
//! Der Thread ist absturzfest: eine Panik im Segmentierer (z. B. im ONNX-Laufzeit)
//! stellt den Kanal auf den 20-s-Chunker um, statt das Transkript stumm enden zu
//! lassen, waehrend die Aufnahme weiterlaeuft.
//!
//! M2 / P2c2 – Echo-Unterdrueckung: Ist `DspConfig::echo` gesetzt (Systemton wird
//! aufgenommen, Einstellung nicht `off`), laeuft Kanal 0 vor dem VAD durch die
//! [`EchoStage`]: Aligner legt den Systemton per QPC-Stempel auf die
//! Mikrofon-Achse, AEC3 (`echo.rs`) nimmt das Echo heraus, das Ergebnis geht an
//! den Segmentierer UND nach `mic_aec.wav` (gleiche Laenge und Achse wie die
//! rohe `mic.wav`, die unveraendert bleibt). Ein Mikrofon-Frame wartet hoechstens
//! [`MAX_REF_WAIT_MS`] Audiozeit auf seine Referenz, danach laeuft er mit
//! Stille als Referenz weiter (Loopback spaet, still oder weg).

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};
use serde::Serialize;

use super::chunker::{ChannelChunker, Chunk};
use super::echo::{self, AlignEvent, Aligner, EchoCanceller, Pulled, FRAME_SAMPLES};
use super::hallucination::{self, BlockFacts, Reason};
use super::recorder::MeetingEvent;
use super::segmenter::{Segment, SegmenterConfig, SegmenterStats, VadSegmenter};
use super::signal_watch::{BlockStats, HealthState, SignalWatch, WatchConfig};
use super::store::{MeetingStore, StoredSegment, TranscriptDelta};
use crate::audio_toolkit::audio::StreamingWavWriter;
use crate::audio_toolkit::VoiceActivityDetector;
use crate::managers::transcription::{TimedSegment, WordTime};

/// Kanaele: 0 = Mikrofon, 1 = Systemton.
pub const CHANNEL_COUNT: usize = 2;
/// Nachrichten (je ~30 ms Audio) in der Queue zum DSP-Thread, beide Kanaele
/// zusammen: rund 30 s Rueckstand je Kanal, ~2 MB.
pub const QUEUE_CAPACITY: usize = 2_048;
/// Blocklaenge des Rueckfall-Chunkers (der heutige Live-Pfad).
pub const FALLBACK_CHUNK_MS: u64 = 20_000;
/// Ab so vielen unverarbeiteten Segmenten im Worker wird gewarnt: die STT
/// kommt dann dauerhaft nicht hinterher.
const BACKLOG_WARN: usize = 50;
const SAMPLES_PER_MS: u64 = 16;

// ---------------------------------------------------------------------------
// Typen
// ---------------------------------------------------------------------------

/// Was der Segmentierer ueber ein Segment weiss (fuer Filter und Latenzmessung).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentMeta {
    /// Ende der Aeusserung (Kanal-Zeitachse, ms).
    pub vad_end_ms: u64,
    /// Zeitpunkt der Entscheidung (Audiozeit, ms).
    pub decided_at_ms: u64,
    pub speech_ms: u64,
    pub span_ms: u64,
}

/// Auftrag an den Transkriptions-Worker.
pub enum WorkItem {
    /// Block des Rueckfall-Chunkers (kein VAD).
    Chunk(u8, Chunk),
    /// Sprachsegment des VAD-Segmentierers.
    Segment(u8, Chunk, SegmentMeta),
    /// Ende: alles davor ist (FIFO) schon abgearbeitet.
    Shutdown,
}

/// Nachricht vom Capture-Callback an den DSP-Thread.
pub enum DspMsg {
    Samples {
        channel: u8,
        samples: Vec<i16>,
        /// Samples, die davor wegen voller Queue verworfen wurden.
        gap_before: u64,
        /// QPC-Zeitstempel (100 ns) des ersten Samples, falls das Geraet einen
        /// liefert (P2c2: gemeinsame Zeitachse fuer die Echo-Unterdrueckung).
        qpc: Option<u64>,
    },
    /// Pause: offenes Segment beenden, VAD zuruecksetzen.
    Boundary {
        channel: u8,
        gap_before: u64,
    },
    /// Die Systemton-Referenz ist endgueltig weg (Loopback startet nicht oder
    /// ist gestorben): Echo-Unterdrueckung aus, Mikrofon laeuft unbearbeitet.
    ReferenceLost,
    Shutdown,
}

/// Meldungen des DSP-Threads. Der Recorder macht daraus `MeetingEvent::Health`
/// (`health_event`): `Health` (Ausfallwaechter, nur Zustandswechsel) und
/// `VadUnavailable` (einmal je Kanal). Der Rest ist Log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DspNotice {
    /// Zustandswechsel des Ausfallwaechters (`signal_watch.rs`).
    Health {
        channel: u8,
        state: HealthState,
    },
    VadUnavailable { channel: u8 },
    Overflow { channel: u8, skipped_ms: u64 },
    DspPanic { channel: u8 },
    /// Panik in der Echo-Unterdrueckung; Kanal 0 laeuft ab da unbearbeitet.
    AecPanic,
}

pub type VadFactory = Arc<dyn Fn() -> anyhow::Result<Box<dyn VoiceActivityDetector>> + Send + Sync>;
pub type NoticeFn = Arc<dyn Fn(DspNotice) + Send + Sync>;

pub struct DspConfig {
    /// Erzeugt einen Detektor je Kanal. `None` oder `Err` = Rueckfall-Chunker.
    pub vad_factory: Option<VadFactory>,
    pub segmenter: SegmenterConfig,
    pub fallback_chunk_ms: u64,
    pub notice: NoticeFn,
    /// Echo-Unterdrueckung fuer Kanal 0 (P2c2). `None` = aus (kein Systemton,
    /// Einstellung `off`); dann entsteht auch keine `mic_aec.wav`.
    pub echo: Option<EchoSetup>,
    /// Ausfallwaechter je Kanal (P2e) und sein Wanduhr-Takt fuer `NoData`.
    pub watch_mic: WatchConfig,
    pub watch_loopback: WatchConfig,
    pub watch_tick: Duration,
}

/// Takt, in dem der DSP-Thread ohne Nachricht nachsieht, ob ein Kanal
/// verstummt ist (`NoData` wird also auf ~0,5 s genau gemeldet).
pub const WATCH_TICK: Duration = Duration::from_millis(500);

impl DspConfig {
    pub fn new(vad_factory: Option<VadFactory>, notice: NoticeFn) -> Self {
        Self {
            vad_factory,
            segmenter: SegmenterConfig::default(),
            fallback_chunk_ms: FALLBACK_CHUNK_MS,
            notice,
            echo: None,
            watch_mic: WatchConfig::mic(),
            watch_loopback: WatchConfig::loopback(),
            watch_tick: WATCH_TICK,
        }
    }

    pub fn with_echo(mut self, echo: EchoSetup) -> Self {
        self.echo = Some(echo);
        self
    }
}

/// Zaehler einer Besprechung. Nur Zahlen, nie Inhalte.
#[derive(Default)]
pub struct DspStats {
    pub overflow_samples: [AtomicU64; CHANNEL_COUNT],
    pub lost_samples: AtomicU64,
    pub segments_emitted: AtomicU64,
    pub dropped_short: AtomicU64,
    pub dropped_silent: AtomicU64,
    pub vad_errors: AtomicU64,
    pub cut_at_max: AtomicU64,
    pub hallu_known_phrase: AtomicU64,
    pub hallu_repetition: AtomicU64,
    pub hallu_too_dense: AtomicU64,
    pub hallu_low_speech_share: AtomicU64,
    pub dsp_panics: AtomicU64,
    pub vad_fallback: AtomicBool,
    pub backlog: AtomicUsize,
    pub backlog_peak: AtomicUsize,
    /// P2c2: Echo-Unterdrueckung war fuer diese Besprechung eingerichtet.
    pub aec_enabled: AtomicBool,
    /// Ich-Segmente, die das Sicherheitsnetz als Echo verworfen hat.
    pub echo_dropped: AtomicU64,
    /// Versatz `mic_qpc0 - sys_qpc0` (ms), sobald bekannt (fuer das
    /// Sicherheitsnetz: Systemzeiten auf die Mikrofon-Achse legen).
    pub timeline_known: AtomicBool,
    pub timeline_offset_ms: AtomicI64,
    /// Abschlussbericht der Echo-Unterdrueckung, gesetzt am Ende des DSP-Threads.
    pub echo: Mutex<Option<EchoSummary>>,
}

impl DspStats {
    fn add_segmenter(&self, s: SegmenterStats) {
        self.dropped_short
            .fetch_add(s.dropped_short, Ordering::Relaxed);
        self.dropped_silent
            .fetch_add(s.dropped_silent, Ordering::Relaxed);
        self.vad_errors.fetch_add(s.vad_errors, Ordering::Relaxed);
        self.cut_at_max.fetch_add(s.cut_at_max, Ordering::Relaxed);
    }

    fn count_drop(&self, reason: Reason) {
        let counter = match reason {
            Reason::KnownPhrase => &self.hallu_known_phrase,
            Reason::Repetition => &self.hallu_repetition,
            Reason::TooDense => &self.hallu_too_dense,
            Reason::LowSpeechShare => &self.hallu_low_speech_share,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Eine Logzeile mit Zahlen, ohne Inhalt.
    pub fn summary(&self) -> String {
        let n = |c: &AtomicU64| c.load(Ordering::Relaxed);
        format!(
            "segments={} short={} silent={} vad_errors={} max_cuts={} hallu[phrase={} repeat={} dense={} share={}] overflow_ms[mic={} sys={}] lost_ms={} panics={} vad_fallback={} backlog_peak={} aec={} echo_dropped={}",
            n(&self.segments_emitted),
            n(&self.dropped_short),
            n(&self.dropped_silent),
            n(&self.vad_errors),
            n(&self.cut_at_max),
            n(&self.hallu_known_phrase),
            n(&self.hallu_repetition),
            n(&self.hallu_too_dense),
            n(&self.hallu_low_speech_share),
            n(&self.overflow_samples[0]) / SAMPLES_PER_MS,
            n(&self.overflow_samples[1]) / SAMPLES_PER_MS,
            n(&self.lost_samples) / SAMPLES_PER_MS,
            n(&self.dsp_panics),
            self.vad_fallback.load(Ordering::Relaxed),
            self.backlog_peak.load(Ordering::Relaxed),
            self.aec_enabled.load(Ordering::Relaxed),
            n(&self.echo_dropped),
        )
    }

    /// Kopie des Abschlussberichts der Echo-Unterdrueckung (nach `drain`).
    pub fn echo_summary(&self) -> Option<EchoSummary> {
        self.echo.lock().ok().and_then(|g| g.clone())
    }
}

// ---------------------------------------------------------------------------
// Callback-Seite
// ---------------------------------------------------------------------------

/// Die Uebergabestelle eines Kanals. Lebt im Capture-Callback; `push` und
/// `pause` warten nie und machen kein I/O.
pub struct ChannelFeed {
    channel: u8,
    tx: SyncSender<DspMsg>,
    stats: Arc<DspStats>,
    pending_gap: u64,
    was_paused: bool,
}

impl ChannelFeed {
    pub fn new(channel: u8, tx: SyncSender<DspMsg>, stats: Arc<DspStats>) -> Self {
        Self {
            channel,
            tx,
            stats,
            pending_gap: 0,
            was_paused: false,
        }
    }

    fn slot(&self) -> usize {
        (self.channel as usize).min(CHANNEL_COUNT - 1)
    }

    /// Liefert einen Block ohne Zeitstempel (siehe [`Self::push_stamped`]).
    #[cfg(test)]
    pub fn push(&mut self, samples: &[i16]) {
        self.push_stamped(samples, None);
    }

    /// Liefert einen Block samt QPC-Stempel seines ersten Samples. Voll oder
    /// DSP-Thread weg: Samples zaehlen, nicht blockieren.
    pub fn push_stamped(&mut self, samples: &[i16], qpc: Option<u64>) {
        self.was_paused = false;
        let msg = DspMsg::Samples {
            channel: self.channel,
            samples: samples.to_vec(),
            gap_before: self.pending_gap,
            qpc,
        };
        match self.tx.try_send(msg) {
            Ok(()) => self.pending_gap = 0,
            Err(TrySendError::Full(_)) => {
                self.pending_gap += samples.len() as u64;
                self.stats.overflow_samples[self.slot()]
                    .fetch_add(samples.len() as u64, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.stats
                    .lost_samples
                    .fetch_add(samples.len() as u64, Ordering::Relaxed);
            }
        }
    }

    /// Nur fuer Dateiquellen (Simulation, nie im Audio-Callback): wartet bei
    /// voller Queue, statt zu verwerfen. `false`, wenn der DSP-Thread weg ist.
    pub fn push_blocking(&mut self, samples: &[i16], qpc: Option<u64>) -> bool {
        self.was_paused = false;
        let msg = DspMsg::Samples {
            channel: self.channel,
            samples: samples.to_vec(),
            gap_before: std::mem::take(&mut self.pending_gap),
            qpc,
        };
        self.tx.send(msg).is_ok()
    }

    /// Meldet den Beginn einer Pause genau einmal. Aufrufen, solange pausiert
    /// ist; schlaegt das Senden fehl (Queue voll), klappt es beim naechsten Mal.
    pub fn pause(&mut self) {
        if self.was_paused {
            return;
        }
        let msg = DspMsg::Boundary {
            channel: self.channel,
            gap_before: self.pending_gap,
        };
        if self.tx.try_send(msg).is_ok() {
            self.was_paused = true;
            self.pending_gap = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// Echo-Unterdrueckung der Ich-Spur (P2c2)
// ---------------------------------------------------------------------------

/// So lange (Audiozeit) wartet ein Mikrofon-Frame hoechstens auf seine
/// Referenz. Normal liegen beide Kanaele < 60 ms auseinander (30-ms-Bloecke
/// hinter je einem Resampler); laenger heisst: Loopback spaet, still oder weg.
pub const MAX_REF_WAIT_MS: u64 = 250;
const MAX_REF_WAIT_SAMPLES: usize = (MAX_REF_WAIT_MS * SAMPLES_PER_MS) as usize;
/// Referenzbloecke (je ~30 ms), die auf den Mikrofon-Anker warten duerfen.
const HELD_REF_BLOCKS: usize = 100;
/// Groesster glaubwuerdiger |mic_qpc0 - sys_qpc0|. Der Loopback startet
/// hoechstens 5 s nach dem Mikrofon (`recorder.rs`); mehr heisst: die Stempel
/// stammen aus verschiedenen Uhren (anderer cpal-Host) -> Ankunftszeit.
pub const MAX_PLAUSIBLE_OFFSET_MS: f64 = 30_000.0;
/// 100-ns-QPC-Einheiten je Sample bei 16 kHz.
const QPC_PER_SAMPLE: i128 = 625;
const QPC_PER_MS: f64 = 10_000.0;
/// Header von `mic_aec.wav` jede Sekunde nachziehen (wie `mic.wav`).
const AEC_FLUSH_EVERY: usize = 16_000;
static SILENCE: [i16; 480] = [0; 480];

/// Ziel der entechoten Mikrofonspur. Trait, damit Tests Speicher- oder
/// scheiternde Senken einsetzen koennen.
pub trait PcmSink: Send {
    fn append(&mut self, samples: &[i16]) -> std::io::Result<()>;
    fn flush_header(&mut self) -> std::io::Result<()>;
    /// Schliesst die Datei ab; liefert die Zahl der Samples. Scheitert das,
    /// entfernt die Senke die Datei selbst.
    fn finalize(self: Box<Self>) -> std::io::Result<u64>;
    /// Unbrauchbar (nie Referenz, Schreibfehler): Datei entfernen.
    fn discard(self: Box<Self>);
}

/// `mic_aec.wav` auf der Platte.
pub struct WavFileSink {
    writer: StreamingWavWriter,
    path: PathBuf,
}

impl WavFileSink {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        Ok(Self {
            writer: StreamingWavWriter::create(path, 16_000)?,
            path: path.to_path_buf(),
        })
    }
}

impl PcmSink for WavFileSink {
    fn append(&mut self, samples: &[i16]) -> std::io::Result<()> {
        self.writer.append(samples)
    }

    fn flush_header(&mut self) -> std::io::Result<()> {
        self.writer.flush_header()
    }

    fn finalize(self: Box<Self>) -> std::io::Result<u64> {
        let frames = self.writer.frames_written();
        let path = self.path;
        match self.writer.finalize() {
            Ok(_) => Ok(frames),
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                Err(e)
            }
        }
    }

    fn discard(self: Box<Self>) {
        let path = self.path;
        drop(self.writer);
        if let Err(e) = std::fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!("meetings: mic_aec.wav not removed: {e}");
            }
        }
    }
}

/// Worauf die gemeinsame Zeitachse beruht.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineBasis {
    /// Beide Geraete lieferten QPC-Stempel derselben Uhr.
    Qpc,
    /// Keine brauchbaren Mikrofon-Stempel: der erste Referenzblock gilt als
    /// gleichzeitig mit dem gerade eingetroffenen Mikrofon (Fehler ~Zehntel-
    /// sekunden, AEC3 gleicht bis ~500 ms selbst aus).
    Arrival,
}

/// Anker der gemeinsamen Zeitachse (`meetings.metadata_json.timeline`).
/// `offset_ms = mic_qpc0 - sys_qpc0`: negativ, wenn der Loopback nach dem
/// Mikrofon startete. Bei `basis = arrival` ist er 0.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MeetingTimeline {
    pub mic_qpc0: Option<u64>,
    pub sys_qpc0: Option<u64>,
    pub offset_ms: f64,
    pub basis: TimelineBasis,
}

pub type TimelineFn = Arc<dyn Fn(&MeetingTimeline) + Send + Sync>;

/// Was die Echo-Unterdrueckung beim Start mitbekommt.
pub struct EchoSetup {
    /// `mic_aec.wav`; `None` = nur fuer die Transkription entechoen.
    pub sink: Option<Box<dyn PcmSink>>,
    /// Einmal gerufen, sobald die Zeitachse feststeht (vom DSP-Thread).
    pub on_timeline: Option<TimelineFn>,
}

/// Abschlussbericht (Log, `metadata_json.aec`, Simulation). Nur Zahlen.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct EchoSummary {
    pub basis: Option<TimelineBasis>,
    pub offset_ms: Option<f64>,
    /// Mindestens ein Frame bekam echte Referenzdaten.
    pub reference_seen: bool,
    pub reference_lost: bool,
    /// Durch AEC3 gelaufene 10-ms-Frames.
    pub frames: u64,
    /// Samples, die nach `ReferenceLost`/Panik unbearbeitet durchliefen.
    pub passthrough_samples: u64,
    /// Frames, deren Referenz nach `MAX_REF_WAIT_MS` noch fehlte.
    pub ref_late_frames: u64,
    /// Neustarts wegen Pause/Unterbrechung des Systemtons.
    pub resets: u64,
    /// Neuverankerungen wegen |Drift| > 500 ms (`aec_realign`).
    pub realigns: u64,
    pub panics: u64,
    /// Referenz, die verworfen wurde (Warteschlange voll, Pause).
    pub ref_dropped_samples: u64,
    pub erl_db: Option<f32>,
    pub erle_db: Option<f32>,
    pub delay_ms: Option<i32>,
    pub aligner_underruns: u64,
    pub aligner_overruns: u64,
    pub aligner_jumps: u64,
    pub aligner_max_drift_ms: i64,
    /// Samples in `mic_aec.wav`, wenn sie behalten wurde.
    pub wav_samples: Option<u64>,
    pub wav_kept: bool,
    pub wav_failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EchoMode {
    Active,
    /// Referenz weg oder Panik: Mikrofon unbearbeitet durchreichen.
    Passthrough,
}

/// Aligner + AEC3 fuer Kanal 0, laeuft im DSP-Thread (kein Callback).
///
/// Achsen: `next_out_pos` = Index des naechsten auszugebenden Mikrofon-Samples
/// (Achse von `mic.wav`), `pending` die Samples dahinter, die auf ihre Referenz
/// warten. Ausgabe sammelt sich in `out` und geht vom Aufrufer an Segmentierer
/// und `mic_aec.wav` – so bleibt die Datei Sample fuer Sample auf der Achse von
/// `mic.wav`, auch ueber Luecken, Pausen und das Ende hinweg.
///
/// Referenz-Zeit je Block (`t_ref_ms` im Sinne von [`Aligner`]):
/// * `Qpc`: Mikrofon-Anker `(qpc, pos)` aus dem juengsten Mikrofonblock,
///   `t_ref = pos/16 + (q_ref - q_anker)/10^4 + offset_ms`. Damit wirken Drift
///   der Mikrofon-Uhr und die Pause (Achse gestaucht) automatisch mit.
/// * `Arrival`: erster Block nach (Neu-)Start = aktuelle Mikrofon-Position,
///   danach QPC-Abstand zum ersten Block.
struct EchoStage {
    aligner: Aligner,
    canceller: EchoCanceller,
    sink: Option<Box<dyn PcmSink>>,
    on_timeline: Option<TimelineFn>,
    stats: Arc<DspStats>,
    mode: EchoMode,
    pending: VecDeque<i16>,
    next_out_pos: u64,
    out: Vec<i16>,
    mic_frame: [i16; FRAME_SAMPLES],
    ref_frame: [i16; FRAME_SAMPLES],
    out_frame: [i16; FRAME_SAMPLES],
    mic_seen: bool,
    mic_qpc0: Option<u64>,
    sys_qpc0: Option<u64>,
    /// Referenz-Samples, die bisher kamen (inkl. Luecken).
    ref_pos: u64,
    basis: Option<TimelineBasis>,
    offset_ms: f64,
    /// (QPC, Mikrofon-Position) des juengsten gestempelten Mikrofonblocks.
    mic_anchor: Option<(u64, u64)>,
    /// `Arrival`: (QPC, Mikrofon-Achse in ms) des ersten Blocks nach Start.
    arrival_base: Option<(Option<u64>, f64)>,
    /// Referenzbloecke, die noch nicht auf die Achse gelegt werden koennen.
    held: VecDeque<(Option<u64>, Vec<i16>)>,
    /// Der Aligner hat seit dem letzten Reset einen Block bekommen.
    ref_anchored: bool,
    /// Systemton-Pause gesehen: beim naechsten Block neu aufsetzen.
    ref_interrupted: bool,
    /// Mikrofon-Pause gesehen, noch kein neuer Mikrofonblock: die Pause des
    /// Systemtons ist dann schon mit abgedeckt (ein Reset je Pause).
    mic_paused: bool,
    /// Seit dem letzten Reset nichts verarbeitet (Reset waere ein No-op).
    fresh: bool,
    samples_since_flush: usize,
    summary: EchoSummary,
    #[cfg(test)]
    panic_at_frame: Option<u64>,
}

impl EchoStage {
    fn new(setup: EchoSetup, stats: Arc<DspStats>) -> Self {
        Self {
            aligner: Aligner::new(),
            canceller: EchoCanceller::new(),
            sink: setup.sink,
            on_timeline: setup.on_timeline,
            stats,
            mode: EchoMode::Active,
            // 1 s Platz: mehr als die Wartezeit plus ein grosser Block.
            pending: VecDeque::with_capacity(16_000),
            next_out_pos: 0,
            out: Vec::with_capacity(16_000),
            mic_frame: [0; FRAME_SAMPLES],
            ref_frame: [0; FRAME_SAMPLES],
            out_frame: [0; FRAME_SAMPLES],
            mic_seen: false,
            mic_qpc0: None,
            sys_qpc0: None,
            ref_pos: 0,
            basis: None,
            offset_ms: 0.0,
            mic_anchor: None,
            arrival_base: None,
            held: VecDeque::with_capacity(HELD_REF_BLOCKS),
            ref_anchored: false,
            ref_interrupted: false,
            mic_paused: false,
            fresh: true,
            samples_since_flush: 0,
            summary: EchoSummary::default(),
            #[cfg(test)]
            panic_at_frame: None,
        }
    }

    /// Mikrofon-Position der bisher eingetroffenen Samples.
    fn mic_in_pos(&self) -> u64 {
        self.next_out_pos + self.pending.len() as u64
    }

    fn on_mic(&mut self, samples: &[i16], qpc: Option<u64>) {
        if self.mode == EchoMode::Passthrough {
            self.out.extend_from_slice(samples);
            self.next_out_pos += samples.len() as u64;
            self.summary.passthrough_samples += samples.len() as u64;
            return;
        }
        let pos = self.mic_in_pos();
        // Zuerst ablegen: was danach auch passiert, das Sample geht nicht verloren.
        self.pending.extend(samples.iter().copied());
        self.mic_seen = true;
        self.mic_paused = false;
        if let Some(q) = qpc {
            if self.mic_qpc0.is_none() {
                self.mic_qpc0 = qpc_minus_samples(q, pos);
            }
            self.mic_anchor = Some((q, pos));
        }
        self.release_held();
        self.process_ready(false);
    }

    fn on_ref(&mut self, samples: Vec<i16>, qpc: Option<u64>) {
        let pos = self.ref_pos;
        self.ref_pos += samples.len() as u64;
        if self.mode == EchoMode::Passthrough {
            return;
        }
        if let (Some(q), None) = (qpc, self.sys_qpc0) {
            self.sys_qpc0 = qpc_minus_samples(q, pos);
        }
        if self.ref_interrupted {
            self.ref_interrupted = false;
            self.reset_aec();
        }
        if !self.can_map() {
            self.hold(qpc, samples);
            return;
        }
        self.release_held();
        self.push_ref(qpc, &samples);
        self.process_ready(false);
    }

    /// Queue-Ueberlauf auf dem Mikrofon: alles davor geht jetzt raus (die
    /// Luecke schreibt der Aufrufer als Stille, siehe [`Self::write_silence`]).
    fn before_mic_gap(&mut self) {
        self.flush_all();
    }

    /// Queue-Ueberlauf auf dem Systemton: die fehlende Referenz ist Stille,
    /// dann bleibt die Zaehlung auf der Achse (auch ohne Zeitstempel).
    fn on_ref_gap(&mut self, gap: u64) {
        self.ref_pos += gap;
        if self.mode == EchoMode::Passthrough || !self.ref_anchored {
            return;
        }
        let mut left = gap;
        while left > 0 {
            let n = left.min(SILENCE.len() as u64) as usize;
            let _ = self.aligner.push_reference(f64::NAN, &SILENCE[..n]);
            left -= n as u64;
        }
    }

    /// Pause (Mikrofon): offene Frames abarbeiten, dann alles vergessen, was
    /// an der alten Zeit haengt. Nach dem Fortsetzen verankert der naechste
    /// gestempelte Mikrofonblock die Achse neu.
    fn mic_boundary(&mut self) {
        if self.mode == EchoMode::Passthrough {
            return;
        }
        self.flush_all();
        self.reset_aec();
        self.mic_anchor = None;
        self.arrival_base = None;
        self.ref_interrupted = false;
        self.mic_paused = true;
    }

    /// Pause (Systemton): gehaltene Referenz stammt von vor der Pause.
    fn ref_boundary(&mut self) {
        if self.mode == EchoMode::Passthrough {
            return;
        }
        self.drop_held();
        self.ref_interrupted = !self.mic_paused;
    }

    /// Loopback startet nicht oder ist gestorben: was schon Referenz hat,
    /// noch bearbeiten, danach unbearbeitet weiter.
    fn reference_lost(&mut self) {
        if self.mode == EchoMode::Passthrough {
            return;
        }
        self.summary.reference_lost = true;
        self.process_ready(false);
        self.enter_passthrough();
    }

    /// Ende der Aufnahme: alles ausgeben, auch den angebrochenen Frame.
    fn finish(&mut self) {
        self.flush_all();
    }

    fn enter_passthrough(&mut self) {
        self.mode = EchoMode::Passthrough;
        let n = self.pending.len() as u64;
        self.out.extend(self.pending.drain(..));
        self.next_out_pos += n;
        self.summary.passthrough_samples += n;
        self.drop_held();
    }

    fn flush_all(&mut self) {
        if self.mode == EchoMode::Passthrough {
            return;
        }
        self.process_ready(true);
        if !self.pending.is_empty() {
            self.process_frame(self.pending.len());
        }
    }

    fn can_map(&self) -> bool {
        match self.basis {
            Some(TimelineBasis::Qpc) => self.mic_anchor.is_some(),
            Some(TimelineBasis::Arrival) | None => self.mic_seen,
        }
    }

    fn hold(&mut self, qpc: Option<u64>, samples: Vec<i16>) {
        if self.held.len() >= HELD_REF_BLOCKS {
            if let Some((_, old)) = self.held.pop_front() {
                self.summary.ref_dropped_samples += old.len() as u64;
            }
        }
        self.held.push_back((qpc, samples));
    }

    fn drop_held(&mut self) {
        for (_, block) in self.held.drain(..) {
            self.summary.ref_dropped_samples += block.len() as u64;
        }
    }

    fn release_held(&mut self) {
        if self.held.is_empty() || !self.can_map() {
            return;
        }
        while let Some((qpc, block)) = self.held.pop_front() {
            self.push_ref(qpc, &block);
        }
    }

    /// Legt die Zeitachse beim ersten Referenzblock fest (einmal je Besprechung).
    fn determine_basis(&mut self) {
        let (basis, offset) = match (self.mic_qpc0, self.sys_qpc0) {
            (Some(mic), Some(sys)) => {
                let offset = (mic as i128 - sys as i128) as f64 / QPC_PER_MS;
                if offset.abs() <= MAX_PLAUSIBLE_OFFSET_MS {
                    (TimelineBasis::Qpc, offset)
                } else {
                    warn!(
                        "meetings: AEC-Zeitstempel unplausibel (Versatz {offset:.0} ms) - Rueckfall auf Ankunftszeit"
                    );
                    (TimelineBasis::Arrival, 0.0)
                }
            }
            _ => (TimelineBasis::Arrival, 0.0),
        };
        self.basis = Some(basis);
        self.offset_ms = offset;
        self.aligner.set_offset_ms(offset);
        self.stats
            .timeline_offset_ms
            .store(offset.round() as i64, Ordering::Relaxed);
        self.stats.timeline_known.store(true, Ordering::Relaxed);
        info!("meetings: AEC-Zeitachse {basis:?}, Versatz Mikrofon-Loopback {offset:.1} ms");
        if let Some(cb) = &self.on_timeline {
            cb(&MeetingTimeline {
                mic_qpc0: self.mic_qpc0,
                sys_qpc0: self.sys_qpc0,
                offset_ms: offset,
                basis,
            });
        }
    }

    /// `t_ref_ms` des Blocks mit Stempel `qpc` (siehe Typ-Doku). NaN = nach
    /// Sample-Zahl anhaengen.
    fn t_ref_ms(&mut self, qpc: Option<u64>) -> f64 {
        let now_ms = self.mic_in_pos() as f64 / SAMPLES_PER_MS as f64;
        match self.basis {
            Some(TimelineBasis::Qpc) => match (qpc, self.mic_anchor) {
                (Some(q), Some((qa, pa))) => {
                    pa as f64 / SAMPLES_PER_MS as f64
                        + (q as i128 - qa as i128) as f64 / QPC_PER_MS
                        + self.offset_ms
                }
                // Erster Block ohne Stempel: "jetzt" statt Achsen-Nullpunkt.
                _ if !self.ref_anchored => now_ms + self.offset_ms,
                _ => f64::NAN,
            },
            Some(TimelineBasis::Arrival) | None => {
                if !self.ref_anchored || self.arrival_base.is_none() {
                    self.arrival_base = Some((qpc, now_ms));
                    return now_ms;
                }
                match (qpc, self.arrival_base) {
                    (Some(q), Some((Some(qb), base_ms))) => {
                        base_ms + (q as i128 - qb as i128) as f64 / QPC_PER_MS
                    }
                    _ => f64::NAN,
                }
            }
        }
    }

    fn push_ref(&mut self, qpc: Option<u64>, samples: &[i16]) {
        if self.basis.is_none() {
            self.determine_basis();
        }
        let t = self.t_ref_ms(qpc);
        if let AlignEvent::Reset { drift_ms } = self.aligner.push_reference(t, samples) {
            warn!("meetings: aec_realign - Systemton um {drift_ms} ms versetzt, AEC schwingt neu ein");
            self.summary.realigns += 1;
            self.canceller.reset();
        }
        self.ref_anchored = true;
        self.fresh = false;
    }

    fn reset_aec(&mut self) {
        if self.fresh {
            return;
        }
        self.aligner.reset();
        self.canceller.reset();
        self.ref_anchored = false;
        self.fresh = true;
        self.summary.resets += 1;
    }

    /// Bearbeitet alle Frames, deren Referenz da ist – oder, wenn `force` bzw.
    /// die Wartezeit ueberschritten ist, auch ohne.
    fn process_ready(&mut self, force: bool) {
        if self.mode == EchoMode::Passthrough {
            return;
        }
        while self.pending.len() >= FRAME_SAMPLES {
            let ready = self.aligner.ready_for(self.next_out_pos, FRAME_SAMPLES);
            if !ready && !force && self.pending.len() < MAX_REF_WAIT_SAMPLES + FRAME_SAMPLES {
                break;
            }
            if !ready {
                self.summary.ref_late_frames += 1;
            }
            self.process_frame(FRAME_SAMPLES);
        }
    }

    /// Ein 10-ms-Frame mit `n` (<= 160) echten Samples vorn aus `pending`.
    /// Reihenfolge fuer die Panik-Sicherheit: erst rechnen, dann `out`
    /// verlaengern, zuletzt aus `pending` entfernen.
    fn process_frame(&mut self, n: usize) {
        let n = n.min(FRAME_SAMPLES).min(self.pending.len());
        let mut it = self.pending.iter().copied();
        for slot in self.mic_frame[..n].iter_mut() {
            *slot = it.next().unwrap_or(0);
        }
        self.mic_frame[n..].fill(0);
        #[cfg(test)]
        if self.panic_at_frame == Some(self.summary.frames) {
            panic!("simulierte Panik im AEC-Pfad");
        }
        if self.aligner.pull(self.next_out_pos, &mut self.ref_frame) == Pulled::Data {
            self.summary.reference_seen = true;
        }
        if self
            .canceller
            .process(&self.mic_frame, &self.ref_frame, &mut self.out_frame)
            .is_err()
        {
            // Kann bei festen 160er-Frames nicht passieren; wenn doch: roh.
            self.out_frame.copy_from_slice(&self.mic_frame);
        }
        self.out.extend_from_slice(&self.out_frame[..n]);
        self.pending.drain(..n);
        self.next_out_pos += n as u64;
        self.summary.frames += 1;
        self.fresh = false;
    }

    /// Schreibt `out` in die Senke (vor der Uebergabe an den Segmentierer).
    fn write_out(&mut self) {
        let Some(sink) = self.sink.as_mut() else {
            return;
        };
        if let Err(e) = sink.append(&self.out) {
            self.sink_failed(e);
            return;
        }
        self.samples_since_flush += self.out.len();
        if self.samples_since_flush >= AEC_FLUSH_EVERY {
            self.samples_since_flush = 0;
            if let Err(e) = sink.flush_header() {
                self.sink_failed(e);
            }
        }
    }

    /// Luecke (Queue-Ueberlauf) auf dem Mikrofon: in der Datei als Stille,
    /// damit sie gleich lang bleibt wie `mic.wav`.
    fn write_silence(&mut self, gap: u64) {
        self.next_out_pos += gap;
        let mut left = gap;
        while left > 0 {
            let n = left.min(SILENCE.len() as u64) as usize;
            if let Some(sink) = self.sink.as_mut() {
                if let Err(e) = sink.append(&SILENCE[..n]) {
                    self.sink_failed(e);
                    return;
                }
            }
            left -= n as u64;
        }
    }

    fn sink_failed(&mut self, e: std::io::Error) {
        warn!("meetings: mic_aec.wav stopped ({e}) - file discarded, transcription continues");
        if let Some(sink) = self.sink.take() {
            sink.discard();
        }
        self.summary.wav_failed = true;
    }

    /// Schliesst `mic_aec.wav` ab (oder verwirft sie, wenn nie Referenz kam)
    /// und liefert den Bericht.
    fn finalize(&mut self) -> EchoSummary {
        if let Some(sink) = self.sink.take() {
            if self.summary.reference_seen {
                match sink.finalize() {
                    Ok(samples) => {
                        self.summary.wav_samples = Some(samples);
                        self.summary.wav_kept = true;
                    }
                    Err(e) => {
                        warn!("meetings: mic_aec.wav not finalized ({e}) - file discarded");
                        self.summary.wav_failed = true;
                    }
                }
            } else {
                info!("meetings: mic_aec.wav discarded - no system audio reference ever arrived");
                sink.discard();
            }
        }
        let ec = self.canceller.stats();
        let al = *self.aligner.stats();
        let s = &mut self.summary;
        s.basis = self.basis;
        s.offset_ms = self.basis.map(|_| self.offset_ms);
        s.erl_db = ec.erl_db;
        s.erle_db = ec.erle_db;
        s.delay_ms = ec.delay_ms;
        s.panics += ec.panics;
        s.aligner_underruns = al.underruns;
        s.aligner_overruns = al.overruns;
        s.aligner_jumps = al.jumps;
        s.aligner_max_drift_ms = al.max_abs_drift_ms;
        s.clone()
    }
}

/// QPC des Samples 0 aus dem Stempel eines spaeteren Samples `pos`.
fn qpc_minus_samples(qpc: u64, pos: u64) -> Option<u64> {
    u64::try_from(qpc as i128 - pos as i128 * QPC_PER_SAMPLE).ok()
}

/// Fuehrt einen Schritt der Echo-Stufe aus. Panikt er, laeuft Kanal 0 ab da
/// unbearbeitet weiter; kein Sample geht verloren oder doppelt (siehe
/// `process_frame`).
fn guard_echo(stage: &mut EchoStage, cfg: &DspConfig, f: impl FnOnce(&mut EchoStage)) {
    if catch_unwind(AssertUnwindSafe(|| f(&mut *stage))).is_err() {
        error!("meetings: panic in echo cancellation - microphone continues unprocessed");
        stage.summary.panics += 1;
        (cfg.notice)(DspNotice::AecPanic);
        stage.enter_passthrough();
    }
}

/// Gibt die fertige Ausgabe der Echo-Stufe an `mic_aec.wav` und Kanal 0.
fn deliver_mic(
    stage: &mut EchoStage,
    lane: &mut Option<Lane>,
    lane_pos: &mut u64,
    cfg: &DspConfig,
    stats: &DspStats,
    out: &mut Vec<WorkItem>,
) {
    if stage.out.is_empty() {
        return;
    }
    stage.write_out();
    let samples = &stage.out;
    apply(lane, 0, *lane_pos, cfg, stats, out, |l, o| l.push(0, samples, o));
    *lane_pos += samples.len() as u64;
    stage.out.clear();
}

// ---------------------------------------------------------------------------
// DSP-Thread
// ---------------------------------------------------------------------------

/// Rueckfall: der heutige 20-s-Chunker, auf die Zeitachse des Kanals gesetzt.
struct ChunkerLane {
    chunker: ChannelChunker,
    base_ms: u64,
}

impl ChunkerLane {
    fn new(chunk_ms: u64, base_ms: u64) -> Self {
        Self {
            chunker: ChannelChunker::new(chunk_ms),
            base_ms,
        }
    }

    fn shift(&self, mut chunk: Chunk) -> Chunk {
        chunk.offset_ms += self.base_ms;
        chunk
    }
}

/// Der Weg eines Kanals durch den DSP-Thread.
enum Lane {
    Vad(Box<VadSegmenter>),
    Chunker(ChunkerLane),
}

impl Lane {
    fn push(&mut self, channel: u8, samples: &[i16], out: &mut Vec<WorkItem>) {
        match self {
            Lane::Vad(seg) => out.extend(
                seg.push(samples)
                    .into_iter()
                    .map(|s| segment_item(channel, s)),
            ),
            Lane::Chunker(c) => {
                if let Some(chunk) = c.chunker.push(samples) {
                    out.push(WorkItem::Chunk(channel, c.shift(chunk)));
                }
            }
        }
    }

    /// Pause. Der Chunker macht wie heute einfach weiter (die Pause steht
    /// nicht in der Zeitachse).
    fn boundary(&mut self, channel: u8, out: &mut Vec<WorkItem>) {
        if let Lane::Vad(seg) = self {
            out.extend(seg.boundary().into_iter().map(|s| segment_item(channel, s)));
        }
    }

    fn skip(&mut self, channel: u8, samples: u64, out: &mut Vec<WorkItem>) {
        match self {
            Lane::Vad(seg) => out.extend(
                seg.skip(samples)
                    .into_iter()
                    .map(|s| segment_item(channel, s)),
            ),
            Lane::Chunker(c) => {
                // Stille einschieben haelt die Offsets richtig.
                let mut left = samples;
                let zeros = vec![0i16; 16_000];
                while left > 0 {
                    let n = left.min(zeros.len() as u64) as usize;
                    if let Some(chunk) = c.chunker.push(&zeros[..n]) {
                        out.push(WorkItem::Chunk(channel, c.shift(chunk)));
                    }
                    left -= n as u64;
                }
            }
        }
    }

    fn flush(&mut self, channel: u8, out: &mut Vec<WorkItem>) {
        match self {
            Lane::Vad(seg) => out.extend(seg.flush().into_iter().map(|s| segment_item(channel, s))),
            Lane::Chunker(c) => {
                if let Some(chunk) = c.chunker.flush() {
                    out.push(WorkItem::Chunk(channel, c.shift(chunk)));
                }
            }
        }
    }
}

fn segment_item(channel: u8, seg: Segment) -> WorkItem {
    let meta = SegmentMeta {
        vad_end_ms: seg.vad_end_ms,
        decided_at_ms: seg.decided_at_ms,
        speech_ms: seg.speech_ms,
        span_ms: seg.span_ms,
    };
    let samples = seg.samples.iter().map(|&s| s as f32 / 32_768.0).collect();
    WorkItem::Segment(
        channel,
        Chunk {
            samples,
            offset_ms: seg.offset_ms,
        },
        meta,
    )
}

fn make_lane(channel: u8, cfg: &DspConfig, stats: &DspStats) -> Lane {
    if let Some(factory) = &cfg.vad_factory {
        // Auch eine Panik beim Laden (ONNX-Laufzeit) darf den Thread nicht
        // beenden: dann laeuft der Kanal im Rueckfall.
        match catch_unwind(AssertUnwindSafe(|| factory())) {
            Ok(Ok(vad)) => {
                return Lane::Vad(Box::new(VadSegmenter::new(vad, cfg.segmenter.clone())))
            }
            Ok(Err(e)) => warn!("meetings: VAD fuer Kanal {channel} nicht geladen ({e}) - Rueckfall auf 20-s-Bloecke"),
            Err(_) => error!("meetings: VAD-Laden fuer Kanal {channel} ist abgestuerzt - Rueckfall auf 20-s-Bloecke"),
        }
    } else {
        warn!("meetings: kein VAD-Modell fuer Kanal {channel} - Rueckfall auf 20-s-Bloecke");
    }
    stats.vad_fallback.store(true, Ordering::Relaxed);
    // Der Recorder macht daraus `Health { VadUnavailable }` (`health_event`).
    (cfg.notice)(DspNotice::VadUnavailable { channel });
    Lane::Chunker(ChunkerLane::new(cfg.fallback_chunk_ms, 0))
}

/// Fuehrt eine Operation auf dem Kanal aus. Panikt sie, laeuft der Kanal ab
/// dieser Position mit dem 20-s-Chunker weiter (gepufferte Sprache des
/// Segmentierers, hoechstens 15 s, geht verloren; die WAV nicht).
#[allow(clippy::too_many_arguments)]
fn apply(
    slot: &mut Option<Lane>,
    channel: u8,
    pos_samples: u64,
    cfg: &DspConfig,
    stats: &DspStats,
    out: &mut Vec<WorkItem>,
    mut op: impl FnMut(&mut Lane, &mut Vec<WorkItem>),
) {
    let lane = slot.get_or_insert_with(|| make_lane(channel, cfg, stats));
    let mut items = Vec::new();
    match catch_unwind(AssertUnwindSafe(|| op(lane, &mut items))) {
        Ok(()) => {
            if let Lane::Vad(seg) = lane {
                stats.add_segmenter(seg.take_stats());
            }
            out.extend(items);
        }
        Err(_) => {
            error!("meetings: DSP-Panik auf Kanal {channel} - Rueckfall auf 20-s-Bloecke");
            stats.dsp_panics.fetch_add(1, Ordering::Relaxed);
            (cfg.notice)(DspNotice::DspPanic { channel });
            let mut fresh = Lane::Chunker(ChunkerLane::new(
                cfg.fallback_chunk_ms,
                pos_samples / SAMPLES_PER_MS,
            ));
            let mut items = Vec::new();
            op(&mut fresh, &mut items);
            out.extend(items);
            *slot = Some(fresh);
        }
    }
}

fn dispatch(items: Vec<WorkItem>, work_tx: &Sender<WorkItem>, stats: &DspStats) {
    for item in items {
        let is_segment = matches!(item, WorkItem::Segment(..));
        let depth = stats.backlog.fetch_add(1, Ordering::Relaxed) + 1;
        stats.backlog_peak.fetch_max(depth, Ordering::Relaxed);
        if depth == BACKLOG_WARN {
            warn!("meetings: {BACKLOG_WARN} Segmente warten auf die Transkription - die STT kommt nicht hinterher");
        }
        if work_tx.send(item).is_err() {
            stats.backlog.fetch_sub(1, Ordering::Relaxed);
        } else if is_segment {
            stats.segments_emitted.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Meldet einen Zustandswechsel des Ausfallwaechters (Log + Notice). Nur auf
/// dem DSP-Thread, nie im Capture-Callback.
fn report_health(cfg: &DspConfig, channel: u8, state: HealthState) {
    if state == HealthState::Recovered {
        info!("meetings: Kanal {channel} liefert wieder Signal");
    } else {
        warn!("meetings: Kanal {channel} auffaellig: {state:?}");
    }
    (cfg.notice)(DspNotice::Health { channel, state });
}

fn dsp_main(
    rx: Receiver<DspMsg>,
    mut cfg: DspConfig,
    work_tx: Sender<WorkItem>,
    stats: Arc<DspStats>,
) {
    let mut lanes: [Option<Lane>; CHANNEL_COUNT] = [None, None];
    // Samples, die der Segmentierer je Kanal bekommen hat (inkl. Luecken).
    // Kanal 0 laeuft mit Echo-Unterdrueckung der Eingabe etwas hinterher.
    let mut lane_pos: [u64; CHANNEL_COUNT] = [0; CHANNEL_COUNT];
    let mut echo = cfg
        .echo
        .take()
        .map(|setup| EchoStage::new(setup, Arc::clone(&stats)));
    stats.aec_enabled.store(echo.is_some(), Ordering::Relaxed);

    // Ausfallwaechter (P2e): sieht die ROHEN Bloecke beider Kanaele (vor der
    // Echo-Unterdrueckung, die ein totes Mikrofon nur noch stiller machte).
    let started = Instant::now();
    let wall_ms = || started.elapsed().as_millis() as u64;
    let mut watches = [
        SignalWatch::new(cfg.watch_mic.clone(), 0),
        SignalWatch::new(cfg.watch_loopback.clone(), 0),
    ];

    // Ende ohne `Shutdown` (alle Sender weg) flusht ebenfalls: nichts verlieren.
    loop {
        let msg = match rx.recv_timeout(cfg.watch_tick) {
            Ok(msg) => msg,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Keine Nachricht: ein verstummter Kanal faellt nur hier auf.
                let now = wall_ms();
                for (ch, watch) in watches.iter_mut().enumerate() {
                    if let Some(state) = watch.on_tick(now) {
                        report_health(&cfg, ch as u8, state);
                    }
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let mut out = Vec::new();
        let (channel, gap, samples) = match msg {
            DspMsg::Shutdown => break,
            DspMsg::ReferenceLost => {
                if let Some(stage) = echo.as_mut() {
                    info!("meetings: system audio reference lost - echo cancellation off, microphone unprocessed");
                    guard_echo(stage, &cfg, |s| s.reference_lost());
                    deliver_mic(stage, &mut lanes[0], &mut lane_pos[0], &cfg, &stats, &mut out);
                }
                dispatch(out, &work_tx, &stats);
                continue;
            }
            DspMsg::Samples {
                channel,
                samples,
                gap_before,
                qpc,
            } => (channel, gap_before, Some((samples, qpc))),
            DspMsg::Boundary {
                channel,
                gap_before,
            } => (channel, gap_before, None),
        };
        let ch = channel as usize;
        if ch >= CHANNEL_COUNT {
            continue;
        }

        if gap > 0 {
            let skipped_ms = gap / SAMPLES_PER_MS;
            warn!("meetings: DSP-Queue uebergelaufen - {skipped_ms} ms auf Kanal {channel} nicht segmentiert (WAV vollstaendig)");
            watches[ch].on_overflow();
            (cfg.notice)(DspNotice::Overflow {
                channel,
                skipped_ms,
            });
            match (ch, echo.as_mut()) {
                (0, Some(stage)) => {
                    guard_echo(stage, &cfg, |s| s.before_mic_gap());
                    deliver_mic(stage, &mut lanes[0], &mut lane_pos[0], &cfg, &stats, &mut out);
                    stage.write_silence(gap);
                }
                (1, Some(stage)) => guard_echo(stage, &cfg, |s| s.on_ref_gap(gap)),
                _ => {}
            }
            apply(
                &mut lanes[ch],
                channel,
                lane_pos[ch],
                &cfg,
                &stats,
                &mut out,
                |l, o| l.skip(channel, gap, o),
            );
            lane_pos[ch] += gap;
        }
        let health = match &samples {
            Some((samples, _)) => watches[ch].on_block(wall_ms(), &BlockStats::of(samples)),
            None => watches[ch].on_pause(),
        };
        if let Some(state) = health {
            report_health(&cfg, channel, state);
        }
        match samples {
            Some((samples, qpc)) => match (ch, echo.as_mut()) {
                (0, Some(stage)) => {
                    guard_echo(stage, &cfg, |s| s.on_mic(&samples, qpc));
                    deliver_mic(stage, &mut lanes[0], &mut lane_pos[0], &cfg, &stats, &mut out);
                }
                (1, Some(stage)) => {
                    let len = samples.len() as u64;
                    apply(
                        &mut lanes[1],
                        channel,
                        lane_pos[1],
                        &cfg,
                        &stats,
                        &mut out,
                        |l, o| l.push(channel, &samples, o),
                    );
                    lane_pos[1] += len;
                    // Neue Referenz kann wartende Mikrofon-Frames freigeben.
                    guard_echo(stage, &cfg, move |s| s.on_ref(samples, qpc));
                    deliver_mic(stage, &mut lanes[0], &mut lane_pos[0], &cfg, &stats, &mut out);
                }
                _ => {
                    apply(
                        &mut lanes[ch],
                        channel,
                        lane_pos[ch],
                        &cfg,
                        &stats,
                        &mut out,
                        |l, o| l.push(channel, &samples, o),
                    );
                    lane_pos[ch] += samples.len() as u64;
                }
            },
            None => {
                match (ch, echo.as_mut()) {
                    (0, Some(stage)) => {
                        guard_echo(stage, &cfg, |s| s.mic_boundary());
                        deliver_mic(stage, &mut lanes[0], &mut lane_pos[0], &cfg, &stats, &mut out);
                    }
                    (1, Some(stage)) => guard_echo(stage, &cfg, |s| s.ref_boundary()),
                    _ => {}
                }
                apply(
                    &mut lanes[ch],
                    channel,
                    lane_pos[ch],
                    &cfg,
                    &stats,
                    &mut out,
                    |l, o| l.boundary(channel, o),
                );
            }
        }
        dispatch(out, &work_tx, &stats);
    }

    // Echo-Stufe zuerst: ihr Rest gehoert noch in Kanal 0, bevor der flusht.
    if let Some(stage) = echo.as_mut() {
        let mut out = Vec::new();
        guard_echo(stage, &cfg, |s| s.finish());
        deliver_mic(stage, &mut lanes[0], &mut lane_pos[0], &cfg, &stats, &mut out);
        dispatch(out, &work_tx, &stats);
        let summary = stage.finalize();
        info!(
            "meetings: aec summary basis={:?} offset_ms={:?} frames={} erle_db={:?} delay_ms={:?} resets={} realigns={} late_frames={} ref_dropped={} passthrough={} panics={} wav_kept={} wav_samples={:?}",
            summary.basis,
            summary.offset_ms,
            summary.frames,
            summary.erle_db,
            summary.delay_ms,
            summary.resets,
            summary.realigns,
            summary.ref_late_frames,
            summary.ref_dropped_samples,
            summary.passthrough_samples,
            summary.panics,
            summary.wav_kept,
            summary.wav_samples,
        );
        if let Ok(mut slot) = stats.echo.lock() {
            *slot = Some(summary);
        }
    }

    for ch in 0..CHANNEL_COUNT {
        let mut out = Vec::new();
        if lanes[ch].is_some() {
            apply(
                &mut lanes[ch],
                ch as u8,
                lane_pos[ch],
                &cfg,
                &stats,
                &mut out,
                |l, o| l.flush(ch as u8, o),
            );
        }
        dispatch(out, &work_tx, &stats);
    }
    debug!("meetings: DSP-Thread beendet");
}

pub struct DspHandle {
    tx: SyncSender<DspMsg>,
    join: Option<JoinHandle<()>>,
    stats: Arc<DspStats>,
}

impl DspHandle {
    pub fn spawn(
        cfg: DspConfig,
        work_tx: Sender<WorkItem>,
        queue_capacity: usize,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<DspMsg>(queue_capacity);
        let stats = Arc::new(DspStats::default());
        let thread_stats = Arc::clone(&stats);
        let join = std::thread::Builder::new()
            .name("meeting-dsp".to_string())
            .spawn(move || dsp_main(rx, cfg, work_tx, thread_stats))?;
        Ok(Self {
            tx,
            join: Some(join),
            stats,
        })
    }

    pub fn feed(&self, channel: u8) -> ChannelFeed {
        ChannelFeed::new(channel, self.tx.clone(), Arc::clone(&self.stats))
    }

    pub fn control(&self) -> DspControl {
        DspControl {
            tx: self.tx.clone(),
        }
    }

    pub fn stats(&self) -> Arc<DspStats> {
        Arc::clone(&self.stats)
    }

    /// Wartet, bis der Thread die Queue leer und alle Segmentierer geflusht hat.
    /// `send` blockiert nur, solange die Queue voll ist und der Thread lebt.
    pub fn finish(mut self) {
        let _ = self.tx.send(DspMsg::Shutdown);
        if let Some(join) = self.join.take() {
            if join.join().is_err() {
                error!("meetings: DSP-Thread ist abgestuerzt");
            }
        }
    }
}

impl Drop for DspHandle {
    /// Nicht blockierend: ein Start, der scheitert, darf den Thread nicht
    /// haengen lassen.
    fn drop(&mut self) {
        if self.join.is_some() {
            let _ = self.tx.try_send(DspMsg::Shutdown);
        }
    }
}

/// Steuerung von aussen (Recorder, Loopback-Watchdog), nie aus einem
/// Audio-Callback.
#[derive(Clone)]
pub struct DspControl {
    tx: SyncSender<DspMsg>,
}

impl DspControl {
    /// Die Systemton-Referenz ist endgueltig weg: Echo-Unterdrueckung aus.
    /// Wartet hoechstens, solange die Queue voll ist und der Thread lebt.
    pub fn reference_lost(&self) {
        let _ = self.tx.send(DspMsg::ReferenceLost);
    }
}

// ---------------------------------------------------------------------------
// Worker-Seite
// ---------------------------------------------------------------------------

/// Baut aus dem STT-Ergebnis eines Blocks die zu speichernden Segmente.
/// Der Halluzinationsfilter greift hier; ein Luecken-Platzhalter (Block
/// endgueltig nicht transkribiert) wird nie gefiltert.
pub(super) fn live_segments(
    timed: Vec<TimedSegment>,
    chunk: &Chunk,
    channel: u8,
    meta: Option<SegmentMeta>,
    next_index: &mut u32,
    stats: &DspStats,
) -> Vec<StoredSegment> {
    let audio_ms = chunk.samples.len() as u64 / SAMPLES_PER_MS;
    let placeholder = super::import::gap_placeholder(chunk.offset_ms, chunk.offset_ms + audio_ms);
    let is_gap = timed.iter().any(|s| s.text == placeholder);
    let mut kept: Vec<TimedSegment> = timed
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();

    if !is_gap {
        kept.retain(|s| match hallucination::check_text(&s.text) {
            Some(reason) => {
                stats.count_drop(reason);
                false
            }
            None => true,
        });
        if !kept.is_empty() {
            let joined = kept
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let facts = BlockFacts {
                audio_ms,
                speech_ms: meta.map(|m| m.speech_ms),
                span_ms: meta.map(|m| m.span_ms),
            };
            if let Some(reason) = hallucination::check_block(&joined, &facts) {
                stats.count_drop(reason);
                kept.clear();
            }
        }
    }

    kept.into_iter()
        .map(|s| {
            let segment = StoredSegment {
                segment_index: *next_index,
                text: s.text,
                start_ms: chunk.offset_ms + s.start_ms,
                end_ms: chunk.offset_ms + s.end_ms,
                channel,
                speaker_index: None,
                // M2-P2d: Wortzeiten auf die Kanal-Achse (wie start_ms).
                words: s.words.map(|ws| {
                    ws.into_iter()
                        .map(|w| WordTime {
                            text: w.text,
                            start_ms: chunk.offset_ms + w.start_ms,
                            end_ms: chunk.offset_ms + w.end_ms,
                        })
                        .collect()
                }),
            };
            *next_index += 1;
            segment
        })
        .collect()
}

/// Suchfenster des Sicherheitsnetzes um ein Ich-Segment (Konzept §3.3).
const ECHO_NET_WINDOW_MS: i64 = 2_000;
/// So viele Gegenseite-Segmente merkt sich das Netz.
const ECHO_NET_HISTORY: usize = 64;

/// RMS in dBFS fuer die f32-Samples eines Blocks (wie `echo::rms_dbfs`).
fn rms_dbfs_f32(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -120.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    let rms = (sum / samples.len() as f64).sqrt();
    (20.0 * rms.max(1e-6).log10()) as f32
}

struct FarSegment {
    /// Mikrofon-Achse (ms).
    start_ms: i64,
    end_ms: i64,
    words: Vec<String>,
    rms_db: f32,
}

/// Sicherheitsnetz (Konzept §3.3): verwirft ein Ich-Segment, das eine leise
/// Kopie der Gegenseite ist (>= 80 % der Woerter in einem Gegenseite-Segment
/// +-2 s und >= 15 dB leiser). Greift fuer Gegenseite-Segmente, die VOR dem
/// Ich-Segment transkribiert wurden; das ist der Normalfall, weil Kanal 0 mit
/// Echo-Unterdrueckung der Referenz hinterherlaeuft.
struct EchoNet {
    far: VecDeque<FarSegment>,
}

impl EchoNet {
    fn new() -> Self {
        Self {
            far: VecDeque::with_capacity(ECHO_NET_HISTORY),
        }
    }

    fn words(segs: &[StoredSegment]) -> Vec<String> {
        segs.iter()
            .flat_map(|s| s.text.split_whitespace().map(str::to_string))
            .collect()
    }

    fn span_ms(chunk: &Chunk) -> (i64, i64) {
        let start = chunk.offset_ms as i64;
        (start, start + (chunk.samples.len() as u64 / SAMPLES_PER_MS) as i64)
    }

    /// Merkt sich ein gespeichertes Gegenseite-Segment. Systemzeit -> Mikrofon-
    /// Achse: `t_mic = t_sys - offset_ms` (offset = mic_qpc0 - sys_qpc0).
    fn remember_far(&mut self, chunk: &Chunk, segs: &[StoredSegment], stats: &DspStats) {
        let shift = if stats.timeline_known.load(Ordering::Relaxed) {
            -stats.timeline_offset_ms.load(Ordering::Relaxed)
        } else {
            0
        };
        let (start, end) = Self::span_ms(chunk);
        if self.far.len() >= ECHO_NET_HISTORY {
            self.far.pop_front();
        }
        self.far.push_back(FarSegment {
            start_ms: start + shift,
            end_ms: end + shift,
            words: Self::words(segs),
            rms_db: rms_dbfs_f32(&chunk.samples),
        });
    }

    fn is_echo(&self, chunk: &Chunk, segs: &[StoredSegment]) -> bool {
        let (start, end) = Self::span_ms(chunk);
        let (lo, hi) = (start - ECHO_NET_WINDOW_MS, end + ECHO_NET_WINDOW_MS);
        let mut far_words: Vec<&str> = Vec::new();
        let mut far_rms = f32::NEG_INFINITY;
        for f in self.far.iter().filter(|f| f.end_ms >= lo && f.start_ms <= hi) {
            far_words.extend(f.words.iter().map(String::as_str));
            far_rms = far_rms.max(f.rms_db);
        }
        if far_words.is_empty() {
            return false;
        }
        let ich_words = Self::words(segs);
        echo::should_drop_duplicate(&ich_words, &far_words, far_rms - rms_dbfs_f32(&chunk.samples))
    }
}

/// Die Schleife des Transkriptions-Worker (ein Thread je Besprechung, FIFO).
/// Ein gescheiterter Block beendet die Besprechung nicht.
fn run_worker(
    rx: Receiver<WorkItem>,
    meeting_id: &str,
    store: &MeetingStore,
    stats: &DspStats,
    mut echo_net: Option<EchoNet>,
    transcribe: &mut dyn FnMut(&Chunk) -> Vec<TimedSegment>,
    emit: &mut dyn FnMut(MeetingEvent),
) {
    let mut next_index: u32 = 0;
    while let Ok(item) = rx.recv() {
        let (channel, chunk, meta) = match item {
            WorkItem::Chunk(channel, chunk) => (channel, chunk, None),
            WorkItem::Segment(channel, chunk, meta) => (channel, chunk, Some(meta)),
            WorkItem::Shutdown => break,
        };
        let _ = stats
            .backlog
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(1))
            });

        let timed = transcribe(&chunk);
        let mut index = next_index;
        let appended = live_segments(timed, &chunk, channel, meta, &mut index, stats);
        if appended.is_empty() {
            continue;
        }
        if let Some(net) = echo_net.as_mut() {
            if channel == 1 {
                net.remember_far(&chunk, &appended, stats);
            } else if net.is_echo(&chunk, &appended) {
                // Nur Zahlen ins Log, nie Inhalte.
                stats.echo_dropped.fetch_add(1, Ordering::Relaxed);
                debug!("meetings: echo safety net dropped a {} ms segment", chunk.samples.len() as u64 / SAMPLES_PER_MS);
                continue;
            }
        }
        let delta = TranscriptDelta {
            new_segments: appended.clone(),
        };
        if let Err(e) = store.append_delta(meeting_id, &delta) {
            error!("meetings: delta not stored: {e}");
            emit(MeetingEvent::Error {
                meeting_id: meeting_id.to_string(),
                message: "delta_store_failed".to_string(),
            });
            continue;
        }
        next_index = index;
        emit(MeetingEvent::Segments {
            meeting_id: meeting_id.to_string(),
            appended,
        });
    }
    debug!("meetings: worker for {meeting_id} finished");
}

/// DSP-Thread + Worker einer Besprechung als Einheit.
pub struct LivePipeline {
    dsp: Option<DspHandle>,
    work_tx: Sender<WorkItem>,
    worker: Option<JoinHandle<()>>,
    stats: Arc<DspStats>,
}

impl LivePipeline {
    pub fn start(
        meeting_id: String,
        store: Arc<MeetingStore>,
        cfg: DspConfig,
        mut transcribe: impl FnMut(&Chunk) -> Vec<TimedSegment> + Send + 'static,
        mut emit: impl FnMut(MeetingEvent) + Send + 'static,
    ) -> Result<Self, String> {
        let (work_tx, work_rx) = mpsc::channel::<WorkItem>();
        // Das Sicherheitsnetz gehoert zur Echo-Unterdrueckung (Einstellung
        // `off` oder ohne Systemton: aus).
        let echo_net = cfg.echo.is_some().then(EchoNet::new);
        let dsp = DspHandle::spawn(cfg, work_tx.clone(), QUEUE_CAPACITY)
            .map_err(|e| format!("dsp_thread_failed: {e}"))?;
        let stats = dsp.stats();
        let worker_stats = Arc::clone(&stats);
        let worker = std::thread::Builder::new()
            .name("meeting-transcribe".to_string())
            .spawn(move || {
                run_worker(
                    work_rx,
                    &meeting_id,
                    &store,
                    &worker_stats,
                    echo_net,
                    &mut transcribe,
                    &mut emit,
                )
            })
            .map_err(|e| format!("worker_thread_failed: {e}"))?;
        Ok(Self {
            dsp: Some(dsp),
            work_tx,
            worker: Some(worker),
            stats,
        })
    }

    pub fn feed(&self, channel: u8) -> Option<ChannelFeed> {
        self.dsp.as_ref().map(|d| d.feed(channel))
    }

    pub fn control(&self) -> Option<DspControl> {
        self.dsp.as_ref().map(DspHandle::control)
    }

    /// Ende der Aufnahme. Voraussetzung: die Captures sind gestoppt (kein
    /// Callback ruft `ChannelFeed::push` mehr). Reihenfolge: DSP leeren und
    /// Segmentierer flushen, dann Worker beenden. Wenn das hier zurueckkehrt,
    /// sind alle Segmente gespeichert und alle `Segments`-Events gesendet.
    pub fn drain(mut self) -> Arc<DspStats> {
        if let Some(dsp) = self.dsp.take() {
            dsp.finish();
        }
        // FIFO: alles, was der DSP-Thread eben abgeliefert hat, wird vor diesem
        // Shutdown transkribiert.
        let _ = self.work_tx.send(WorkItem::Shutdown);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                error!("meetings: Transkriptions-Worker ist abgestuerzt");
            }
        }
        info!("meetings: live pipeline drained ({})", self.stats.summary());
        Arc::clone(&self.stats)
    }
}

/// `TranscriptFinal` (Planer-Entscheidung B4): der Live-Stand ist gespeichert.
/// `epoch` ist die Generation der Segmente; P2d verschiebt das Senden hinter
/// den Enddurchlauf.
pub fn transcript_final_event(
    store: &MeetingStore,
    meeting_id: &str,
    model: Option<String>,
) -> MeetingEvent {
    let epoch = store.segment_epoch(meeting_id).unwrap_or_else(|e| {
        warn!("meetings: segment epoch not readable: {e}");
        0
    });
    MeetingEvent::TranscriptFinal {
        meeting_id: meeting_id.to_string(),
        epoch,
        model,
    }
}

#[cfg(test)]
mod tests {
    use super::super::segmenter::test_support::*;
    use super::super::store::MeetingSource;
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    fn energy_factory() -> VadFactory {
        Arc::new(|| Ok(EnergyVad::boxed()))
    }

    fn notices() -> (NoticeFn, Arc<Mutex<Vec<DspNotice>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        (Arc::new(move |n| sink.lock().unwrap().push(n)), log)
    }

    fn cat(parts: &[Vec<i16>]) -> Vec<i16> {
        parts.iter().flatten().copied().collect()
    }

    /// DSP-Thread ohne Worker: Ausgang direkt in einen Kanal.
    fn dsp_only(
        vad: Option<VadFactory>,
    ) -> (DspHandle, Receiver<WorkItem>, Arc<Mutex<Vec<DspNotice>>>) {
        let (notice, log) = notices();
        let (tx, rx) = mpsc::channel();
        let h = DspHandle::spawn(DspConfig::new(vad, notice), tx, QUEUE_CAPACITY).unwrap();
        (h, rx, log)
    }

    fn feed_blocks(feed: &mut ChannelFeed, audio: &[i16]) {
        for block in audio.chunks(480) {
            feed.push(block);
        }
    }

    fn items(rx: &Receiver<WorkItem>) -> Vec<WorkItem> {
        rx.try_iter().collect()
    }

    fn seg_offsets(items: &[WorkItem]) -> Vec<(u8, u64, usize)> {
        items
            .iter()
            .filter_map(|i| match i {
                WorkItem::Segment(ch, c, _) | WorkItem::Chunk(ch, c) => {
                    Some((*ch, c.offset_ms, c.samples.len()))
                }
                WorkItem::Shutdown => None,
            })
            .collect()
    }

    // ---- Segmentierung im Thread ----------------------------------------

    #[test]
    fn segments_from_both_channels_keep_their_own_timeline() {
        let (h, rx, _) = dsp_only(Some(energy_factory()));
        let mut mic = h.feed(0);
        let mut sys = h.feed(1);
        feed_blocks(&mut mic, &cat(&[quiet(1_000), loud(2_000), quiet(2_000)]));
        feed_blocks(&mut sys, &cat(&[quiet(4_000), loud(1_500), quiet(2_000)]));
        h.finish();
        let got = seg_offsets(&items(&rx));
        assert_eq!(got.len(), 2, "{got:?}");
        let mic_seg = got.iter().find(|g| g.0 == 0).unwrap();
        let sys_seg = got.iter().find(|g| g.0 == 1).unwrap();
        assert!((690..=720).contains(&mic_seg.1), "mic {}", mic_seg.1);
        assert!((3_690..=3_720).contains(&sys_seg.1), "sys {}", sys_seg.1);
    }

    #[test]
    fn shutdown_flushes_the_open_segment_of_every_channel() {
        let (h, rx, _) = dsp_only(Some(energy_factory()));
        let mut mic = h.feed(0);
        let mut sys = h.feed(1);
        feed_blocks(&mut mic, &loud(2_000));
        feed_blocks(&mut sys, &loud(1_000));
        h.finish();
        let got = items(&rx);
        assert_eq!(got.len(), 2, "beide offenen Segmente muessen kommen");
        assert!(got.iter().all(|i| matches!(i, WorkItem::Segment(..))));
    }

    #[test]
    fn pause_ends_the_segment_and_resume_starts_a_new_one() {
        let (h, rx, _) = dsp_only(Some(energy_factory()));
        let mut mic = h.feed(0);
        feed_blocks(&mut mic, &loud(2_000));
        mic.pause();
        mic.pause(); // zweiter Aufruf waehrend der Pause: kein zweites Signal
                     // Nach dem Fortsetzen: neue Sprache. Sonst haetten beide Teile ein Segment.
        feed_blocks(&mut mic, &cat(&[loud(2_000), quiet(1_500)]));
        h.finish();
        let got = seg_offsets(&items(&rx));
        assert_eq!(got.len(), 2, "{got:?}");
        // Zwei Sekunden vor der Pause, Achse laeuft danach bei 2 s weiter.
        assert!(got[0].1 < 100);
        assert!(got[1].1 >= 2_000, "zweites Segment bei {} ms", got[1].1);
    }

    #[test]
    fn a_gap_from_a_full_queue_moves_the_timeline_instead_of_shifting_it() {
        let (tx, rx) = mpsc::sync_channel::<DspMsg>(2);
        let stats = Arc::new(DspStats::default());
        let mut feed = ChannelFeed::new(0, tx, Arc::clone(&stats));
        for _ in 0..5 {
            feed.push(&[1i16; 480]);
        }
        // 2 angekommen, 3 gezaehlt und verworfen.
        assert_eq!(stats.overflow_samples[0].load(Ordering::Relaxed), 3 * 480);
        assert_eq!(rx.try_iter().count(), 2);
        // Der naechste Block traegt die Luecke.
        feed.push(&[1i16; 480]);
        match rx.try_recv().unwrap() {
            DspMsg::Samples { gap_before, .. } => assert_eq!(gap_before, 3 * 480),
            _ => panic!("Samples erwartet"),
        }
        feed.push(&[1i16; 480]);
        match rx.try_recv().unwrap() {
            DspMsg::Samples { gap_before, .. } => assert_eq!(gap_before, 0),
            _ => panic!("Samples erwartet"),
        }
    }

    #[test]
    fn an_overflow_gap_is_reported_and_advances_the_segment_offsets() {
        let (h, rx, log) = dsp_only(Some(energy_factory()));
        let tx = h.tx.clone();
        // 1 s Stille, dann meldet der Callback 4 s verworfene Samples, dann Sprache.
        tx.send(DspMsg::Samples {
            channel: 0,
            samples: quiet(1_000),
            gap_before: 0,
            qpc: None,
        })
        .unwrap();
        tx.send(DspMsg::Samples {
            channel: 0,
            samples: cat(&[loud(2_000), quiet(1_500)]),
            gap_before: 4 * 16_000,
            qpc: None,
        })
        .unwrap();
        h.finish();
        let got = seg_offsets(&items(&rx));
        assert_eq!(got.len(), 1);
        assert!((4_990..=5_040).contains(&got[0].1), "offset {}", got[0].1);
        assert_eq!(
            log.lock().unwrap().as_slice(),
            &[
                DspNotice::Overflow {
                    channel: 0,
                    skipped_ms: 4_000
                },
                // P2e: derselbe Ueberlauf als Zustandswechsel fuer die Oberflaeche.
                DspNotice::Health {
                    channel: 0,
                    state: HealthState::QueueOverflow
                }
            ]
        );
    }

    fn health_of(log: &Mutex<Vec<DspNotice>>) -> Vec<(u8, HealthState)> {
        log.lock()
            .unwrap()
            .iter()
            .filter_map(|n| match n {
                DspNotice::Health { channel, state } => Some((*channel, *state)),
                _ => None,
            })
            .collect()
    }

    fn dsp_with_watch(
        mic: WatchConfig,
    ) -> (DspHandle, Receiver<WorkItem>, Arc<Mutex<Vec<DspNotice>>>) {
        let (notice, log) = notices();
        let (tx, rx) = mpsc::channel();
        let mut cfg = DspConfig::new(Some(energy_factory()), notice);
        cfg.watch_mic = mic;
        cfg.watch_tick = Duration::from_millis(20);
        (DspHandle::spawn(cfg, tx, QUEUE_CAPACITY).unwrap(), rx, log)
    }

    #[test]
    fn a_muted_mic_reports_digital_zero_once_and_recovered_when_sound_returns() {
        let (h, _rx, log) = dsp_only(Some(energy_factory()));
        let mut mic = h.feed(0);
        feed_blocks(&mut mic, &cat(&[loud(1_000), zeros(12_000), loud(1_000)]));
        h.finish();
        assert_eq!(
            health_of(&log),
            vec![(0, HealthState::DigitalZero), (0, HealthState::Recovered)]
        );
    }

    #[test]
    fn the_loopback_channel_reports_no_signal_health() {
        // Stille der Gegenseite ist normal: Kanal 1 meldet trotz 40 s Nullen nichts.
        let (h, _rx, log) = dsp_only(Some(energy_factory()));
        let mut sys = h.feed(1);
        feed_blocks(&mut sys, &zeros(40_000));
        h.finish();
        assert!(health_of(&log).is_empty(), "{:?}", health_of(&log));
    }

    #[test]
    fn a_stalled_mic_reports_no_data_from_the_tick_and_recovers_with_the_next_block() {
        let cfg = WatchConfig {
            no_data_ms: Some(150),
            start_grace_ms: 0,
            ..WatchConfig::mic()
        };
        let (h, _rx, log) = dsp_with_watch(cfg);
        let mut mic = h.feed(0);
        feed_blocks(&mut mic, &loud(500));
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(health_of(&log), vec![(0, HealthState::NoData)]);
        feed_blocks(&mut mic, &loud(60));
        h.finish();
        assert_eq!(
            health_of(&log),
            vec![(0, HealthState::NoData), (0, HealthState::Recovered)]
        );
    }

    #[test]
    fn a_pause_stops_the_no_data_watch() {
        let cfg = WatchConfig {
            no_data_ms: Some(100),
            start_grace_ms: 0,
            ..WatchConfig::mic()
        };
        let (h, _rx, log) = dsp_with_watch(cfg);
        let mut mic = h.feed(0);
        feed_blocks(&mut mic, &loud(500));
        mic.pause();
        std::thread::sleep(Duration::from_millis(400));
        h.finish();
        assert!(health_of(&log).is_empty(), "{:?}", health_of(&log));
    }

    #[test]
    fn many_overflows_in_a_row_are_one_health_event() {
        let (h, _rx, log) = dsp_only(Some(energy_factory()));
        let tx = h.tx.clone();
        for _ in 0..20 {
            tx.send(DspMsg::Samples {
                channel: 0,
                samples: loud(100),
                gap_before: 16_000,
                qpc: None,
            })
            .unwrap();
        }
        h.finish();
        assert_eq!(health_of(&log), vec![(0, HealthState::QueueOverflow)]);
    }

    #[test]
    fn a_dead_dsp_thread_makes_push_count_lost_samples_and_never_block() {
        let (tx, rx) = mpsc::sync_channel::<DspMsg>(4);
        drop(rx);
        let stats = Arc::new(DspStats::default());
        let mut feed = ChannelFeed::new(1, tx, Arc::clone(&stats));
        feed.push(&[0i16; 480]);
        feed.pause();
        assert_eq!(stats.lost_samples.load(Ordering::Relaxed), 480);
    }

    // ---- Rueckfall ohne VAD --------------------------------------------------

    #[test]
    fn without_a_vad_model_the_20_second_chunker_runs_and_the_notice_fires() {
        let (h, rx, log) = dsp_only(None);
        let stats = h.stats();
        let mut mic = h.feed(0);
        // 45 s laut: zwei 20-s-Bloecke + Rest beim Flush.
        feed_blocks(&mut mic, &loud(45_000));
        h.finish();
        let got = items(&rx);
        assert!(got.iter().all(|i| matches!(i, WorkItem::Chunk(..))));
        let offs = seg_offsets(&got);
        assert_eq!(offs.len(), 3, "{offs:?}");
        assert_eq!(offs[0].1, 0);
        assert!(offs[1].1 >= 15_000 && offs[1].1 <= 20_100);
        assert_eq!(
            log.lock().unwrap().as_slice(),
            &[DspNotice::VadUnavailable { channel: 0 }]
        );
        assert!(stats.vad_fallback.load(Ordering::Relaxed));
        // Kein Audio verloren.
        let total: usize = offs.iter().map(|o| o.2).sum();
        assert_eq!(total, 45 * 16_000);
    }

    #[test]
    fn a_vad_that_fails_to_load_falls_back_per_channel() {
        let factory: VadFactory = Arc::new(|| anyhow::bail!("Modell fehlt"));
        let (h, rx, log) = dsp_only(Some(factory));
        let mut sys = h.feed(1);
        feed_blocks(&mut sys, &loud(25_000));
        h.finish();
        let got = items(&rx);
        assert!(!got.is_empty());
        assert!(got.iter().all(|i| matches!(i, WorkItem::Chunk(1, _))));
        assert_eq!(
            log.lock().unwrap().as_slice(),
            &[DspNotice::VadUnavailable { channel: 1 }]
        );
    }

    #[test]
    fn fallback_chunker_keeps_the_timeline_across_a_gap() {
        let (h, rx, _) = dsp_only(None);
        let tx = h.tx.clone();
        tx.send(DspMsg::Samples {
            channel: 0,
            samples: loud(1_000),
            gap_before: 0,
            qpc: None,
        })
        .unwrap();
        tx.send(DspMsg::Samples {
            channel: 0,
            samples: loud(1_000),
            gap_before: 3 * 16_000,
            qpc: None,
        })
        .unwrap();
        h.finish();
        let offs = seg_offsets(&items(&rx));
        // 1 s + 3 s Luecke + 1 s = ein Block ab 0 mit 5 s.
        assert_eq!(offs.len(), 1);
        assert_eq!(offs[0], (0, 0, 5 * 16_000));
    }

    #[test]
    fn a_vad_that_panics_while_loading_falls_back_too() {
        let factory: VadFactory = Arc::new(|| panic!("ort init"));
        let (h, rx, log) = dsp_only(Some(factory));
        let mut mic = h.feed(0);
        feed_blocks(&mut mic, &loud(3_000));
        h.finish();
        let got = items(&rx);
        assert_eq!(got.len(), 1);
        assert!(matches!(got[0], WorkItem::Chunk(0, _)));
        assert_eq!(
            log.lock().unwrap().as_slice(),
            &[DspNotice::VadUnavailable { channel: 0 }]
        );
    }

    #[test]
    fn a_panicking_vad_switches_the_channel_to_the_chunker_and_keeps_going() {
        struct Bomb {
            frames: u32,
        }
        impl VoiceActivityDetector for Bomb {
            fn push_frame<'a>(
                &'a mut self,
                frame: &'a [f32],
            ) -> anyhow::Result<crate::audio_toolkit::vad::VadFrame<'a>> {
                self.frames += 1;
                if self.frames == 40 {
                    panic!("ort abgestuerzt");
                }
                Ok(crate::audio_toolkit::vad::VadFrame::Speech(frame))
            }
        }
        let factory: VadFactory = Arc::new(|| Ok(Box::new(Bomb { frames: 0 })));
        let (h, rx, log) = dsp_only(Some(factory));
        let stats = h.stats();
        let mut mic = h.feed(0);
        feed_blocks(&mut mic, &loud(50_000));
        h.finish();
        let got = items(&rx);
        assert_eq!(stats.dsp_panics.load(Ordering::Relaxed), 1);
        assert!(log
            .lock()
            .unwrap()
            .contains(&DspNotice::DspPanic { channel: 0 }));
        // Nach der Panik kommen weiter Bloecke, und ihre Achse stimmt: der erste
        // Chunker-Block beginnt dort, wo der Segmentierer die Verarbeitung verlor.
        let chunks: Vec<_> = got
            .iter()
            .filter_map(|i| match i {
                WorkItem::Chunk(_, c) => Some((c.offset_ms, c.samples.len())),
                _ => None,
            })
            .collect();
        assert!(!chunks.is_empty(), "kein Rueckfall-Block");
        assert!(chunks[0].0 >= 1_100 && chunks[0].0 <= 1_300, "{chunks:?}");
        let last = chunks.last().unwrap();
        let end_ms = last.0 + (last.1 as u64) / 16;
        assert!((49_900..=50_100).contains(&end_ms), "Ende {end_ms} ms");
    }

    // ---- Filter + Worker ---------------------------------------------------

    fn timed(text: &str, start: u64, end: u64) -> TimedSegment {
        TimedSegment {
            text: text.to_string(),
            start_ms: start,
            end_ms: end,
            words: None,
        }
    }

    fn chunk(offset_ms: u64, ms: usize) -> Chunk {
        Chunk {
            samples: vec![0.1; ms * 16],
            offset_ms,
        }
    }

    fn meta(speech_ms: u64) -> SegmentMeta {
        SegmentMeta {
            vad_end_ms: 0,
            decided_at_ms: 0,
            speech_ms,
            span_ms: speech_ms,
        }
    }

    #[test]
    fn live_segments_apply_the_chunk_offset_and_continuous_indices() {
        let stats = DspStats::default();
        let mut idx = 7;
        let out = live_segments(
            vec![
                timed("Erster Satz.", 0, 1_000),
                timed("Zweiter Satz.", 1_000, 2_500),
            ],
            &chunk(60_000, 3_000),
            1,
            Some(meta(2_500)),
            &mut idx,
            &stats,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(
            (out[0].segment_index, out[0].start_ms, out[0].channel),
            (7, 60_000, 1)
        );
        assert_eq!((out[1].segment_index, out[1].end_ms), (8, 62_500));
        assert_eq!(idx, 9);
    }

    #[test]
    fn live_segments_drop_a_silence_hallucination_and_count_it() {
        let stats = DspStats::default();
        let mut idx = 0;
        let out = live_segments(
            vec![timed("Thank you.", 0, 1_000)],
            &chunk(0, 1_500),
            0,
            Some(meta(500)),
            &mut idx,
            &stats,
        );
        assert!(out.is_empty());
        assert_eq!(idx, 0);
        assert_eq!(stats.hallu_known_phrase.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn live_segments_drop_text_when_the_vad_saw_almost_no_speech() {
        let stats = DspStats::default();
        let mut idx = 0;
        let out = live_segments(
            vec![timed("Das klingt nach einem ganz normalen Satz", 0, 8_000)],
            &chunk(0, 10_000),
            0,
            // Sprachframes streuen ueber 9 s, zusammen nur 1 s (Rauschen).
            Some(SegmentMeta {
                span_ms: 9_000,
                ..meta(1_000)
            }),
            &mut idx,
            &stats,
        );
        assert!(out.is_empty());
        assert_eq!(stats.hallu_low_speech_share.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_gap_placeholder_is_never_filtered() {
        let stats = DspStats::default();
        let mut idx = 0;
        let c = chunk(120_000, 500);
        // Kurzer Block, langer Platzhaltertext: waere sonst "zu dicht".
        let placeholder = super::super::import::gap_placeholder(120_000, 120_500);
        let out = live_segments(
            vec![timed(&placeholder, 0, 500)],
            &c,
            0,
            Some(meta(450)),
            &mut idx,
            &stats,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, placeholder);
    }

    #[test]
    fn the_chunker_fallback_path_filters_text_but_has_no_vad_facts() {
        let stats = DspStats::default();
        let mut idx = 0;
        let out = live_segments(
            vec![timed(
                "Wir starten mit dem ersten Punkt der Tagesordnung.",
                0,
                6_000,
            )],
            &chunk(0, 20_000),
            0,
            None,
            &mut idx,
            &stats,
        );
        assert_eq!(out.len(), 1);
    }

    // ---- Ganze Pipeline mit echtem Store ---------------------------------------

    fn store() -> Arc<MeetingStore> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let s = MeetingStore::open_at(&path).unwrap();
        std::mem::forget(dir);
        Arc::new(s)
    }

    struct Rig {
        pipeline: LivePipeline,
        store: Arc<MeetingStore>,
        meeting_id: String,
        events: Arc<Mutex<Vec<MeetingEvent>>>,
        notices: Arc<Mutex<Vec<DspNotice>>>,
    }

    fn rig(vad: Option<VadFactory>, delay: Duration) -> Rig {
        let store = store();
        let meeting = store
            .create_meeting("Test", MeetingSource::Live, Some(1))
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let (notice, notices) = notices();
        let pipeline = LivePipeline::start(
            meeting.id.clone(),
            Arc::clone(&store),
            DspConfig::new(vad, notice),
            move |chunk: &Chunk| {
                std::thread::sleep(delay);
                // Ein Wort je Sekunde Audio, damit weder Dichte noch Wiederholung greift.
                let secs = (chunk.samples.len() / 16_000).max(1);
                let text = (0..secs)
                    .map(|i| format!("wort{}", i as u64 + chunk.offset_ms))
                    .collect::<Vec<_>>()
                    .join(" ");
                vec![TimedSegment {
                    text,
                    start_ms: 0,
                    end_ms: chunk.samples.len() as u64 / 16,
                    words: None,
                }]
            },
            move |e| sink.lock().unwrap().push(e),
        )
        .unwrap();
        Rig {
            pipeline,
            store,
            meeting_id: meeting.id,
            events,
            notices,
        }
    }

    #[test]
    fn transcript_final_comes_after_the_last_segments_event() {
        let r = rig(Some(energy_factory()), Duration::from_millis(30));
        let mut mic = r.pipeline.feed(0).unwrap();
        // Zwei Aeusserungen; die zweite ist beim Stop noch offen (Flush).
        feed_blocks(&mut mic, &cat(&[loud(1_500), quiet(1_000), loud(2_000)]));
        let _stats = r.pipeline.drain();
        // Genau die Reihenfolge aus recorder::stop(): erst drain, dann Final.
        let final_event = transcript_final_event(&r.store, &r.meeting_id, Some("modell-x".into()));
        r.events.lock().unwrap().push(final_event);

        let events = r.events.lock().unwrap();
        let seg_events: Vec<usize> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, MeetingEvent::Segments { .. }))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(seg_events.len(), 2, "beide Segmente muessen gesendet sein");
        let last = events.len() - 1;
        assert!(seg_events.iter().all(|i| *i < last));
        match &events[last] {
            MeetingEvent::TranscriptFinal {
                meeting_id,
                epoch,
                model,
            } => {
                assert_eq!(meeting_id, &r.meeting_id);
                assert_eq!(*epoch, 0);
                assert_eq!(model.as_deref(), Some("modell-x"));
            }
            other => panic!("TranscriptFinal erwartet, war {other:?}"),
        }
        assert_eq!(events.len(), 3);
        // Alles, was gesendet wurde, steht auch im Store.
        let stored = r.store.get_segments(&r.meeting_id).unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(
            stored.iter().map(|s| s.segment_index).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn stop_waits_for_a_slow_worker_before_the_final_event() {
        // Segment ist erst nach 300 ms transkribiert; drain() darf nicht vorher
        // zurueckkehren.
        let r = rig(Some(energy_factory()), Duration::from_millis(300));
        let mut mic = r.pipeline.feed(0).unwrap();
        feed_blocks(&mut mic, &loud(2_000));
        let started = std::time::Instant::now();
        r.pipeline.drain();
        assert!(started.elapsed() >= Duration::from_millis(280));
        assert_eq!(r.store.get_segments(&r.meeting_id).unwrap().len(), 1);
    }

    #[test]
    fn the_fallback_pipeline_stores_chunks_when_vad_is_missing() {
        let r = rig(None, Duration::from_millis(1));
        let mut mic = r.pipeline.feed(0).unwrap();
        feed_blocks(&mut mic, &loud(25_000));
        r.pipeline.drain();
        assert_eq!(r.notices.lock().unwrap().len(), 1);
        let stored = r.store.get_segments(&r.meeting_id).unwrap();
        assert_eq!(stored.len(), 2, "20 s + 5 s");
        assert_eq!(stored[0].start_ms, 0);
    }

    #[test]
    fn dropping_the_pipeline_without_drain_ends_both_threads() {
        // Start scheitert nach dem Anlegen der Pipeline (z. B. Mikrofon): keine
        // haengenden Threads. Der Zeuge ist ein Arc, den die Worker-Closure haelt.
        let witness = Arc::new(());
        let held = Arc::clone(&witness);
        let store = store();
        let meeting = store
            .create_meeting("Abbruch", MeetingSource::Live, Some(1))
            .unwrap();
        let (notice, _) = notices();
        let pipeline = LivePipeline::start(
            meeting.id,
            store,
            DspConfig::new(Some(energy_factory()), notice),
            move |_c: &Chunk| {
                let _keep = &held;
                Vec::new()
            },
            |_e| {},
        )
        .unwrap();
        assert_eq!(Arc::strong_count(&witness), 2);
        drop(pipeline);
        for _ in 0..200 {
            if Arc::strong_count(&witness) == 1 {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("Worker-Thread lebt nach dem Drop weiter");
    }

    #[test]
    fn stats_summary_contains_counts_only() {
        let stats = DspStats::default();
        stats.count_drop(Reason::KnownPhrase);
        stats.overflow_samples[0].fetch_add(16_000, Ordering::Relaxed);
        let line = stats.summary();
        assert!(line.contains("phrase=1"));
        assert!(line.contains("mic=1000"));
    }

    // =======================================================================
    // P2c2: Echo-Unterdrueckung im DSP-Thread
    // =======================================================================

    use super::super::echo::rms_dbfs;

    /// Synthetischer QPC-Nullpunkt und Stempel je Sample-Position (16 kHz).
    const Q0: u64 = 10_000_000_000;
    fn stamp(pos: u64) -> Option<u64> {
        Some(Q0 + pos * 625)
    }

    /// Zeitplan der Fixtures (siehe echo.rs / make-m2-fixtures.py).
    const FAR_ONLY: (f64, f64) = (6.0, 9.8);
    const NEAR_ONLY: (f64, f64) = (11.5, 13.9);

    fn region(x: &[i16], r: (f64, f64)) -> &[i16] {
        &x[(r.0 * 16_000.0) as usize..(r.1 * 16_000.0) as usize]
    }

    fn fixture(name: &str) -> Vec<i16> {
        let path: std::path::PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
            .iter()
            .collect();
        super::super::simulate::read_pcm16_mono(&path).unwrap()
    }

    /// Senke im Speicher; `fail_after` simuliert eine volle Platte.
    #[derive(Clone, Default)]
    struct MemSink {
        data: Arc<Mutex<Vec<i16>>>,
        state: Arc<Mutex<&'static str>>,
        fail_after: Option<usize>,
    }

    impl PcmSink for MemSink {
        fn append(&mut self, samples: &[i16]) -> std::io::Result<()> {
            let mut data = self.data.lock().unwrap();
            if let Some(limit) = self.fail_after {
                if data.len() + samples.len() > limit {
                    return Err(std::io::Error::other("Platte voll"));
                }
            }
            data.extend_from_slice(samples);
            Ok(())
        }
        fn flush_header(&mut self) -> std::io::Result<()> {
            Ok(())
        }
        fn finalize(self: Box<Self>) -> std::io::Result<u64> {
            *self.state.lock().unwrap() = "finalized";
            Ok(self.data.lock().unwrap().len() as u64)
        }
        fn discard(self: Box<Self>) {
            *self.state.lock().unwrap() = "discarded";
        }
    }

    type Timelines = Arc<Mutex<Vec<MeetingTimeline>>>;

    fn echo_setup(sink: Option<&MemSink>) -> (EchoSetup, Timelines) {
        let timelines: Timelines = Arc::new(Mutex::new(Vec::new()));
        let t = Arc::clone(&timelines);
        (
            EchoSetup {
                sink: sink.map(|s| Box::new(s.clone()) as Box<dyn PcmSink>),
                on_timeline: Some(Arc::new(move |tl: &MeetingTimeline| {
                    t.lock().unwrap().push(tl.clone())
                })),
            },
            timelines,
        )
    }

    fn stage(sink: Option<&MemSink>) -> (EchoStage, Arc<DspStats>, Timelines) {
        let stats = Arc::new(DspStats::default());
        let (setup, timelines) = echo_setup(sink);
        (EchoStage::new(setup, Arc::clone(&stats)), stats, timelines)
    }

    // ---- EchoStage direkt (synchron) -----------------------------------------

    #[test]
    fn a_mic_frame_waits_at_most_250_ms_for_its_reference() {
        let (mut st, _, _) = stage(None);
        st.on_mic(&vec![100i16; 32_000], stamp(0));
        let n = st.out.len();
        assert!(
            (32_000 - MAX_REF_WAIT_SAMPLES - FRAME_SAMPLES..=32_000 - MAX_REF_WAIT_SAMPLES)
                .contains(&n),
            "{n} Samples ausgegeben"
        );
        assert!(st.summary.ref_late_frames > 0, "ohne Referenz gezaehlt");
        // Referenz kommt (gleiche Zeit): alles Wartende ist jetzt bereit.
        st.on_ref(vec![0; 32_000], stamp(0));
        assert_eq!(st.out.len(), 32_000);
    }

    #[test]
    fn a_late_loopback_sets_a_negative_offset_once_and_does_not_hold_up_the_mic() {
        let (mut st, stats, timelines) = stage(None);
        st.on_mic(&vec![0; 4_800], stamp(0));
        // Loopback startet 1,2 s nach dem Mikrofon.
        st.on_ref(vec![0; 480], stamp(1_200 * 16));
        st.on_ref(vec![0; 480], stamp(1_200 * 16 + 480));
        let t = timelines.lock().unwrap().clone();
        assert_eq!(t.len(), 1, "Zeitachse genau einmal gemeldet");
        assert_eq!(t[0].basis, TimelineBasis::Qpc);
        assert_eq!(t[0].mic_qpc0, Some(Q0));
        assert_eq!(t[0].sys_qpc0, Some(Q0 + 1_200 * 10_000));
        assert!((t[0].offset_ms + 1_200.0).abs() < 1e-9);
        assert_eq!(stats.timeline_offset_ms.load(Ordering::Relaxed), -1_200);
        // Die 0,3 s Mikrofon liegen vor dem Beginn der Referenz: sofort fertig.
        assert_eq!(st.out.len(), 4_800);
    }

    #[test]
    fn implausible_or_missing_mic_stamps_fall_back_to_arrival_time() {
        let (mut st, _, timelines) = stage(None);
        st.on_mic(&vec![0; 1_600], stamp(0));
        st.on_ref(vec![0; 480], Some(Q0 + 40 * 10_000_000)); // 40 s: andere Uhr
        assert_eq!(timelines.lock().unwrap()[0].basis, TimelineBasis::Arrival);
        assert_eq!(timelines.lock().unwrap()[0].offset_ms, 0.0);

        let (mut st, _, timelines) = stage(None);
        st.on_mic(&vec![0; 1_600], None);
        st.on_ref(vec![0; 480], stamp(0));
        assert_eq!(timelines.lock().unwrap()[0].basis, TimelineBasis::Arrival);
        // Ankunftszeit: der erste Block liegt an der aktuellen Mikrofon-Position.
        st.on_mic(&vec![0; 480], None);
        assert!(st.summary.reference_seen);
    }

    #[test]
    fn reference_that_arrives_before_the_first_mic_block_is_held_not_lost() {
        let (mut st, _, _) = stage(None);
        st.on_ref(vec![3; 480], stamp(0));
        assert_eq!(st.held.len(), 1);
        st.on_mic(&vec![0; 480], stamp(0));
        assert!(st.held.is_empty());
        assert_eq!(st.summary.ref_dropped_samples, 0);
        assert!(st.summary.reference_seen, "gehaltene Referenz wurde benutzt");
    }

    #[test]
    fn a_drift_jump_over_500_ms_realigns_and_resets_the_canceller() {
        let (mut st, _, _) = stage(None);
        st.on_mic(&vec![0; 16_000], stamp(0));
        for k in 0..20u64 {
            st.on_ref(vec![0; 480], stamp(k * 480));
        }
        assert_eq!(st.summary.realigns, 0);
        st.on_ref(vec![0; 480], stamp(20 * 480 + 700 * 16));
        assert_eq!(st.summary.realigns, 1, "aec_realign");
        assert_eq!(st.canceller.stats().resets, 1, "EchoCanceller::reset()");
    }

    #[test]
    fn a_pause_resets_once_and_the_next_stamped_block_reanchors_without_realign() {
        for sys_first in [true, false] {
            let (mut st, _, _) = stage(None);
            st.on_mic(&vec![0; 16_000], stamp(0));
            st.on_ref(vec![0; 16_000], stamp(0));
            assert_eq!(st.out.len(), 16_000);
            if sys_first {
                st.ref_boundary();
                st.mic_boundary();
            } else {
                st.mic_boundary();
                st.ref_boundary();
            }
            // 5 s Pause: QPC laeuft weiter, die Mikrofon-Achse nicht (gestaucht).
            let resume = stamp(16_000 + 5 * 16_000);
            st.on_mic(&vec![0; 4_800], resume);
            st.on_ref(vec![0; 4_800], resume);
            assert_eq!(st.summary.resets, 1, "ein Reset je Pause (sys_first={sys_first})");
            assert_eq!(st.summary.realigns, 0, "kein falsches Realign");
            assert_eq!(st.out.len(), 16_000 + 4_800);
            // Nach dem Fortsetzen liegt die Referenz wieder auf der Achse.
            let mut r = [0i16; FRAME_SAMPLES];
            assert!(st.aligner.ready_for(16_000 + 4_640, 160));
            assert_eq!(st.aligner.pull(16_000 + 4_640, &mut r), Pulled::Data);
        }
    }

    #[test]
    fn pre_pause_reference_still_held_at_the_pause_is_dropped_and_counted() {
        let (mut st, _, _) = stage(None);
        st.on_mic(&vec![0; 1_600], stamp(0));
        st.on_ref(vec![0; 480], stamp(0)); // Zeitachse steht (Qpc)
        st.mic_boundary(); // Mikrofon pausiert, Anker weg
        st.on_ref(vec![0; 480], stamp(480)); // Referenz von vor der Pause
        assert_eq!(st.held.len(), 1);
        st.ref_boundary();
        assert!(st.held.is_empty());
        assert_eq!(st.summary.ref_dropped_samples, 480);
    }

    #[test]
    fn reference_lost_passes_the_microphone_through_bit_exact_and_discards_the_track() {
        let sink = MemSink::default();
        let (mut st, _, _) = stage(Some(&sink));
        st.on_mic(&vec![500; 8_000], stamp(0));
        st.reference_lost();
        let raw: Vec<i16> = (0..4_801).map(|i| (i % 2_000) as i16 - 1_000).collect();
        st.on_mic(&raw, stamp(8_000));
        st.finish();
        assert_eq!(st.out.len(), 8_000 + 4_801);
        assert_eq!(&st.out[8_000..], &raw[..], "nach dem Verlust unveraendert");
        st.write_out();
        let summary = st.finalize();
        assert!(summary.reference_lost && !summary.reference_seen);
        assert!(!summary.wav_kept);
        assert_eq!(*sink.state.lock().unwrap(), "discarded", "nie Referenz: keine mic_aec.wav");
    }

    #[test]
    fn a_panic_in_the_aec_path_loses_and_duplicates_nothing() {
        let (notice, log) = notices();
        let cfg = DspConfig::new(None, notice);
        let (mut st, _, _) = stage(None);
        st.panic_at_frame = Some(5);
        let mic: Vec<i16> = (0..3_200).map(|i| (i % 1_000) as i16 + 1).collect();
        guard_echo(&mut st, &cfg, |s| s.on_ref(vec![0; 3_200], stamp(0)));
        guard_echo(&mut st, &cfg, |s| s.on_mic(&mic, stamp(0)));
        assert_eq!(st.out.len(), 3_200, "nichts verloren, nichts doppelt");
        assert_eq!(&st.out[5 * 160..], &mic[5 * 160..], "ab der Panik roh");
        assert_eq!(st.summary.panics, 1);
        assert!(log.lock().unwrap().contains(&DspNotice::AecPanic));
        // Weiter geht es roh.
        guard_echo(&mut st, &cfg, |s| s.on_mic(&[7; 100], stamp(3_200)));
        assert_eq!(&st.out[3_200..], &[7; 100]);
    }

    #[test]
    fn a_full_disk_drops_the_track_but_not_the_transcription_input() {
        let sink = MemSink {
            fail_after: Some(4_000),
            ..Default::default()
        };
        let (mut st, _, _) = stage(Some(&sink));
        st.on_ref(vec![0; 9_600], stamp(0));
        st.on_mic(&vec![0; 9_600], stamp(0));
        st.write_out(); // 9600 > 4000: Schreibfehler
        assert_eq!(*sink.state.lock().unwrap(), "discarded");
        assert!(st.sink.is_none());
        assert_eq!(st.out.len(), 9_600, "Segmentierer bekommt trotzdem alles");
        let summary = st.finalize();
        assert!(summary.wav_failed && !summary.wav_kept);
    }

    // ---- durch den DSP-Thread ------------------------------------------------

    fn dsp_with_echo(
        sink: &MemSink,
        vad: Option<VadFactory>,
    ) -> (DspHandle, Receiver<WorkItem>, Timelines, Arc<Mutex<Vec<DspNotice>>>) {
        let (notice, log) = notices();
        let (setup, timelines) = echo_setup(Some(sink));
        let (tx, rx) = mpsc::channel();
        let h = DspHandle::spawn(
            DspConfig::new(vad, notice).with_echo(setup),
            tx,
            QUEUE_CAPACITY,
        )
        .unwrap();
        (h, rx, timelines, log)
    }

    /// Speist Mikrofon und Systemton in Zeitstempel-Reihenfolge ein, wie zwei
    /// Capture-Threads. Der Systemton beginnt `sys_start` Samples spaeter.
    fn feed_both(h: &DspHandle, mic: &[i16], sys: &[i16], sys_start: usize) {
        let mut mf = h.feed(0);
        let mut sf = h.feed(1);
        let (mut m, mut s) = (0usize, 0usize);
        while m < mic.len() || s < sys.len() {
            let take_mic = m < mic.len() && (s >= sys.len() || m <= s + sys_start);
            if take_mic {
                let end = (m + 480).min(mic.len());
                assert!(mf.push_blocking(&mic[m..end], stamp(m as u64)));
                m = end;
            } else {
                let end = (s + 480).min(sys.len());
                assert!(sf.push_blocking(&sys[s..end], stamp((s + sys_start) as u64)));
                s = end;
            }
        }
    }

    #[test]
    fn the_dsp_thread_cancels_the_echo_of_a_late_loopback_and_writes_an_equal_length_track() {
        let mic = fixture("m2_echo_mic.wav");
        let render = fixture("m2_echo_render.wav");
        let late = 1_200 * 16;
        let sink = MemSink::default();
        let (h, rx, timelines, _) = dsp_with_echo(&sink, Some(energy_factory()));
        let stats = h.stats();
        feed_both(&h, &mic, &render[late..], late);
        h.finish();

        let out = sink.data.lock().unwrap().clone();
        assert_eq!(out.len(), mic.len(), "mic_aec gleich lang wie mic");
        assert_eq!(*sink.state.lock().unwrap(), "finalized");
        let erle = rms_dbfs(region(&mic, FAR_ONLY)) - rms_dbfs(region(&out, FAR_ONLY));
        let near = rms_dbfs(region(&out, NEAR_ONLY)) - rms_dbfs(region(&mic, NEAR_ONLY));
        let summary = stats.echo_summary().unwrap();
        println!(
            "dsp-aec: erle_far_only={erle:.1} dB near_delta={near:.1} dB | {summary:?}"
        );
        assert!(erle >= 20.0, "ERLE {erle:.1} dB");
        assert!(near >= -3.0, "Nahsprache {near:.1} dB");
        let t = timelines.lock().unwrap()[0].clone();
        assert_eq!(t.basis, TimelineBasis::Qpc);
        assert!((t.offset_ms + 1_200.0).abs() < 1e-6);
        assert_eq!((summary.realigns, summary.panics), (0, 0));
        assert!(summary.reference_seen && summary.wav_kept);
        assert_eq!(summary.wav_samples, Some(mic.len() as u64));
        assert!(stats.aec_enabled.load(Ordering::Relaxed));
        // Kanal 0 hat Segmente aus dem entechoten Signal bekommen.
        assert!(items(&rx)
            .iter()
            .any(|i| matches!(i, WorkItem::Segment(0, ..))));
    }

    #[test]
    fn the_aec_converges_again_after_a_pause() {
        // Pause nach 3 s, 5 s lang (QPC springt, Achse gestaucht).
        let mic = fixture("m2_echo_mic.wav");
        let render = fixture("m2_echo_render.wav");
        let cut = 3 * 16_000;
        let sink = MemSink::default();
        let (h, _rx, _, _) = dsp_with_echo(&sink, Some(energy_factory()));
        let stats = h.stats();
        let tx = h.tx.clone();
        let (mut mf, mut sf) = (h.feed(0), h.feed(1));
        for k in (0..cut).step_by(480) {
            assert!(mf.push_blocking(&mic[k..k + 480], stamp(k as u64)));
            assert!(sf.push_blocking(&render[k..k + 480], stamp(k as u64)));
        }
        for ch in [0u8, 1] {
            tx.send(DspMsg::Boundary {
                channel: ch,
                gap_before: 0,
            })
            .unwrap();
        }
        let pause = 5 * 16_000;
        for k in (cut..mic.len()).step_by(480) {
            let q = stamp((k + pause) as u64);
            let end = (k + 480).min(mic.len());
            assert!(mf.push_blocking(&mic[k..end], q));
            assert!(sf.push_blocking(&render[k..end], q));
        }
        drop((mf, sf, tx));
        h.finish();
        let out = sink.data.lock().unwrap().clone();
        assert_eq!(out.len(), mic.len());
        let window = (7.5, 9.8);
        let erle = rms_dbfs(region(&mic, window)) - rms_dbfs(region(&out, window));
        let summary = stats.echo_summary().unwrap();
        println!("dsp-aec nach Pause: erle({window:?})={erle:.1} dB | resets={} realigns={}", summary.resets, summary.realigns);
        assert_eq!((summary.resets, summary.realigns), (1, 0));
        assert!(erle >= 15.0, "ERLE nach der Pause {erle:.1} dB");
    }

    #[test]
    fn mic_aec_keeps_the_length_of_mic_across_gap_pause_partial_frames_and_a_lost_reference() {
        let sink = MemSink::default();
        let (h, _rx, _, log) = dsp_with_echo(&sink, None);
        let stats = h.stats();
        let tx = h.tx.clone();
        let send = |channel: u8, n: usize, gap: u64, pos: u64| {
            tx.send(DspMsg::Samples {
                channel,
                samples: vec![300; n],
                gap_before: gap,
                qpc: stamp(pos),
            })
            .unwrap();
        };
        send(0, 1_001, 0, 0);
        send(1, 1_440, 0, 0);
        send(0, 999, 3_200, 1_001 + 3_200); // 200 ms Ueberlauf davor
        send(1, 5_000, 0, 1_440);
        tx.send(DspMsg::Boundary { channel: 0, gap_before: 0 }).unwrap();
        tx.send(DspMsg::Boundary { channel: 1, gap_before: 0 }).unwrap();
        send(0, 777, 0, 200_000);
        tx.send(DspMsg::ReferenceLost).unwrap();
        send(0, 555, 0, 200_777);
        send(1, 480, 0, 200_000); // nach dem Verlust: ignoriert
        drop(tx);
        h.finish();
        let total = 1_001 + 3_200 + 999 + 777 + 555;
        assert_eq!(sink.data.lock().unwrap().len(), total);
        let summary = stats.echo_summary().unwrap();
        assert!(summary.reference_lost && summary.wav_kept);
        assert_eq!(summary.wav_samples, Some(total as u64));
        assert!(log
            .lock()
            .unwrap()
            .contains(&DspNotice::Overflow { channel: 0, skipped_ms: 200 }));
        // Die Luecke steht als Stille in der Datei, an ihrer Stelle.
        let data = sink.data.lock().unwrap();
        assert!(data[1_001..1_001 + 3_200].iter().all(|&s| s == 0));
    }

    /// Kanal-0-Samples, die der Rueckfall-Chunker bekommen hat (ohne VAD).
    fn chunked_mic(items: &[WorkItem]) -> Vec<f32> {
        items
            .iter()
            .filter_map(|i| match i {
                WorkItem::Chunk(0, c) => Some(c.samples.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn with_echo_cancellation_off_the_microphone_reaches_the_lane_untouched() {
        let (h, rx, _) = dsp_only(None);
        let stats = h.stats();
        let raw: Vec<i16> = (0..32_000).map(|i| ((i * 7) % 3_000) as i16 - 1_500).collect();
        let mut mic = h.feed(0);
        let mut sys = h.feed(1);
        feed_blocks(&mut mic, &raw);
        feed_blocks(&mut sys, &raw);
        h.finish();
        let got = chunked_mic(&items(&rx));
        let want: Vec<f32> = raw.iter().map(|&s| s as f32 / 32_768.0).collect();
        assert_eq!(got, want, "Einstellung off: kein Eingriff");
        assert!(!stats.aec_enabled.load(Ordering::Relaxed));
        assert!(stats.echo_summary().is_none());
    }

    #[test]
    fn a_loopback_that_never_starts_turns_aec_off_and_leaves_no_track() {
        let sink = MemSink::default();
        let (h, rx, _, _) = dsp_with_echo(&sink, None);
        let stats = h.stats();
        h.control().reference_lost(); // wie recorder.rs bei Startfehler/Timeout
        let raw: Vec<i16> = (0..24_000).map(|i| ((i * 13) % 4_000) as i16 - 2_000).collect();
        let mut mic = h.feed(0);
        for (k, block) in raw.chunks(480).enumerate() {
            assert!(mic.push_blocking(block, stamp(k as u64 * 480)));
        }
        drop(mic);
        h.finish();
        let got = chunked_mic(&items(&rx));
        let want: Vec<f32> = raw.iter().map(|&s| s as f32 / 32_768.0).collect();
        assert_eq!(got, want, "ohne Loopback unbearbeitet");
        let summary = stats.echo_summary().unwrap();
        assert!(summary.reference_lost && !summary.reference_seen);
        assert_eq!(summary.frames, 0);
        assert_eq!(*sink.state.lock().unwrap(), "discarded");
    }

    // ---- Sicherheitsnetz im Worker ---------------------------------------------

    fn tone_chunk(offset_ms: u64, ms: usize, amp: f32) -> Chunk {
        Chunk {
            samples: (0..ms * 16)
                .map(|i| if i % 2 == 0 { amp } else { -amp })
                .collect(),
            offset_ms,
        }
    }

    fn seg_item(channel: u8, chunk: Chunk) -> WorkItem {
        let ms = chunk.samples.len() as u64 / 16;
        WorkItem::Segment(
            channel,
            chunk,
            SegmentMeta {
                vad_end_ms: 0,
                decided_at_ms: 0,
                speech_ms: ms,
                span_ms: ms,
            },
        )
    }

    /// Worker mit Sicherheitsnetz; die Attrappen-STT liefert fuer jeden Block
    /// denselben Satz (das Echo "hoert" dieselben Woerter).
    fn run_net(items_in: Vec<WorkItem>, stats: &DspStats) -> Vec<StoredSegment> {
        let store = store();
        let meeting = store
            .create_meeting("Netz", MeetingSource::Live, Some(1))
            .unwrap();
        let (tx, rx) = mpsc::channel();
        for i in items_in {
            tx.send(i).unwrap();
        }
        tx.send(WorkItem::Shutdown).unwrap();
        let mut transcribe = |c: &Chunk| {
            vec![TimedSegment {
                text: "Bitte schicken Sie mir die Kalkulation bis Donnerstag".into(),
                start_ms: 0,
                end_ms: c.samples.len() as u64 / 16,
                words: None,
            }]
        };
        run_worker(
            rx,
            &meeting.id,
            &store,
            stats,
            Some(EchoNet::new()),
            &mut transcribe,
            &mut |_e| {},
        );
        store.get_segments(&meeting.id).unwrap()
    }

    #[test]
    fn the_safety_net_drops_a_quiet_copy_of_the_far_end() {
        let stats = DspStats::default();
        let stored = run_net(
            vec![
                seg_item(1, tone_chunk(10_000, 3_000, 0.3)),  // Gegenseite, laut
                seg_item(0, tone_chunk(10_100, 3_000, 0.01)), // Ich, 30 dB leiser
            ],
            &stats,
        );
        assert_eq!(stats.echo_dropped.load(Ordering::Relaxed), 1);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].channel, 1);
    }

    #[test]
    fn the_safety_net_keeps_loud_near_speech_and_far_away_copies() {
        // Gleiche Woerter, aber der Nutzer spricht selbst laut: behalten.
        let stats = DspStats::default();
        let stored = run_net(
            vec![
                seg_item(1, tone_chunk(10_000, 3_000, 0.3)),
                seg_item(0, tone_chunk(10_100, 3_000, 0.2)),
            ],
            &stats,
        );
        assert_eq!(stats.echo_dropped.load(Ordering::Relaxed), 0);
        assert_eq!(stored.len(), 2);
        // Leise, aber 10 s entfernt: kein Echo dieses Segments.
        let stats = DspStats::default();
        let stored = run_net(
            vec![
                seg_item(1, tone_chunk(0, 3_000, 0.3)),
                seg_item(0, tone_chunk(10_000, 3_000, 0.01)),
            ],
            &stats,
        );
        assert_eq!((stats.echo_dropped.load(Ordering::Relaxed), stored.len()), (0, 2));
    }

    #[test]
    fn the_safety_net_puts_system_times_on_the_mic_axis() {
        // Loopback startete 5 s nach dem Mikrofon: Systemzeit 0 = Mikrofonzeit 5 s.
        let stats = DspStats::default();
        stats.timeline_known.store(true, Ordering::Relaxed);
        stats.timeline_offset_ms.store(-5_000, Ordering::Relaxed);
        let stored = run_net(
            vec![
                seg_item(1, tone_chunk(0, 3_000, 0.3)),
                seg_item(0, tone_chunk(5_000, 3_000, 0.01)),
            ],
            &stats,
        );
        assert_eq!(stats.echo_dropped.load(Ordering::Relaxed), 1);
        assert_eq!(stored.len(), 1);
    }

    #[test]
    fn the_echo_stage_allocates_nothing_per_frame_beyond_sonora() {
        // Gleichgewicht: ein Block Mikrofon + Referenz, Ausgabe abholen. Die
        // Stufe selbst darf nichts allokieren, was sonora nicht schon tut.
        use super::super::echo::alloc_probe::count_allocs;
        let (mut st, _, _) = stage(None);
        let block = vec![100i16; 480];
        for k in 0..200u64 {
            st.on_mic(&block, stamp(k * 480));
            st.on_ref(block.clone(), stamp(k * 480));
            st.out.clear();
        }
        let refs: Vec<Vec<i16>> = (0..100).map(|_| block.clone()).collect();
        let mut refs = refs.into_iter();
        let ((), allocs, _) = count_allocs(|| {
            for k in 200..300u64 {
                st.on_mic(&block, stamp(k * 480));
                st.on_ref(refs.next().unwrap(), stamp(k * 480));
                st.out.clear();
            }
        });
        let frames = 100 * 3;
        println!("echo-stage alloc-messung: {allocs} Allokationen fuer {frames} Frames (sonora ~35 je Frame)");
        assert!(allocs / frames <= 64, "{} je Frame", allocs / frames);
    }
}
