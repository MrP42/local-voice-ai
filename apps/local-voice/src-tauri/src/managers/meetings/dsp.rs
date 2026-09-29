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

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;

use log::{debug, error, info, warn};

use super::chunker::{ChannelChunker, Chunk};
use super::hallucination::{self, BlockFacts, Reason};
use super::recorder::MeetingEvent;
use super::segmenter::{Segment, SegmenterConfig, SegmenterStats, VadSegmenter};
use super::store::{MeetingStore, StoredSegment, TranscriptDelta};
use crate::audio_toolkit::VoiceActivityDetector;
use crate::managers::transcription::TimedSegment;

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
    },
    /// Pause: offenes Segment beenden, VAD zuruecksetzen.
    Boundary {
        channel: u8,
        gap_before: u64,
    },
    Shutdown,
}

/// Meldungen des DSP-Threads. Heute nur Log; das Ziel ist `MeetingEvent::Health`
/// (Paket P2e).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DspNotice {
    VadUnavailable { channel: u8 },
    Overflow { channel: u8, skipped_ms: u64 },
    DspPanic { channel: u8 },
}

pub type VadFactory = Arc<dyn Fn() -> anyhow::Result<Box<dyn VoiceActivityDetector>> + Send + Sync>;
pub type NoticeFn = Arc<dyn Fn(DspNotice) + Send + Sync>;

pub struct DspConfig {
    /// Erzeugt einen Detektor je Kanal. `None` oder `Err` = Rueckfall-Chunker.
    pub vad_factory: Option<VadFactory>,
    pub segmenter: SegmenterConfig,
    pub fallback_chunk_ms: u64,
    pub notice: NoticeFn,
}

impl DspConfig {
    pub fn new(vad_factory: Option<VadFactory>, notice: NoticeFn) -> Self {
        Self {
            vad_factory,
            segmenter: SegmenterConfig::default(),
            fallback_chunk_ms: FALLBACK_CHUNK_MS,
            notice,
        }
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
            "segments={} short={} silent={} vad_errors={} max_cuts={} hallu[phrase={} repeat={} dense={} share={}] overflow_ms[mic={} sys={}] lost_ms={} panics={} vad_fallback={} backlog_peak={}",
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
        )
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

    /// Liefert einen Block. Voll oder DSP-Thread weg: Samples zaehlen, nicht
    /// blockieren.
    pub fn push(&mut self, samples: &[i16]) {
        self.was_paused = false;
        let msg = DspMsg::Samples {
            channel: self.channel,
            samples: samples.to_vec(),
            gap_before: self.pending_gap,
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
    // TODO(P2e): hier MeetingEvent::Health { state: VadUnavailable } senden.
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

fn dsp_main(rx: Receiver<DspMsg>, cfg: DspConfig, work_tx: Sender<WorkItem>, stats: Arc<DspStats>) {
    let mut lanes: [Option<Lane>; CHANNEL_COUNT] = [None, None];
    let mut pos: [u64; CHANNEL_COUNT] = [0; CHANNEL_COUNT];

    // Ende ohne `Shutdown` (alle Sender weg) flusht ebenfalls: nichts verlieren.
    while let Ok(msg) = rx.recv() {
        let (channel, gap, samples) = match msg {
            DspMsg::Shutdown => break,
            DspMsg::Samples {
                channel,
                samples,
                gap_before,
            } => (channel, gap_before, Some(samples)),
            DspMsg::Boundary {
                channel,
                gap_before,
            } => (channel, gap_before, None),
        };
        let ch = channel as usize;
        if ch >= CHANNEL_COUNT {
            continue;
        }
        let mut out = Vec::new();

        if gap > 0 {
            let skipped_ms = gap / SAMPLES_PER_MS;
            warn!("meetings: DSP-Queue uebergelaufen - {skipped_ms} ms auf Kanal {channel} nicht segmentiert (WAV vollstaendig)");
            // TODO(P2e): hier MeetingEvent::Health senden.
            (cfg.notice)(DspNotice::Overflow {
                channel,
                skipped_ms,
            });
            apply(
                &mut lanes[ch],
                channel,
                pos[ch],
                &cfg,
                &stats,
                &mut out,
                |l, o| l.skip(channel, gap, o),
            );
            pos[ch] += gap;
        }
        match samples {
            Some(samples) => {
                apply(
                    &mut lanes[ch],
                    channel,
                    pos[ch],
                    &cfg,
                    &stats,
                    &mut out,
                    |l, o| l.push(channel, &samples, o),
                );
                pos[ch] += samples.len() as u64;
            }
            None => apply(
                &mut lanes[ch],
                channel,
                pos[ch],
                &cfg,
                &stats,
                &mut out,
                |l, o| l.boundary(channel, o),
            ),
        }
        dispatch(out, &work_tx, &stats);
    }

    for ch in 0..CHANNEL_COUNT {
        let mut out = Vec::new();
        if lanes[ch].is_some() {
            apply(
                &mut lanes[ch],
                ch as u8,
                pos[ch],
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
            };
            *next_index += 1;
            segment
        })
        .collect()
}

/// Die Schleife des Transkriptions-Worker (ein Thread je Besprechung, FIFO).
/// Ein gescheiterter Block beendet die Besprechung nicht.
fn run_worker(
    rx: Receiver<WorkItem>,
    meeting_id: &str,
    store: &MeetingStore,
    stats: &DspStats,
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
        })
        .unwrap();
        tx.send(DspMsg::Samples {
            channel: 0,
            samples: cat(&[loud(2_000), quiet(1_500)]),
            gap_before: 4 * 16_000,
        })
        .unwrap();
        h.finish();
        let got = seg_offsets(&items(&rx));
        assert_eq!(got.len(), 1);
        assert!((4_990..=5_040).contains(&got[0].1), "offset {}", got[0].1);
        assert_eq!(
            log.lock().unwrap().as_slice(),
            &[DspNotice::Overflow {
                channel: 0,
                skipped_ms: 4_000
            }]
        );
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
        })
        .unwrap();
        tx.send(DspMsg::Samples {
            channel: 0,
            samples: loud(1_000),
            gap_before: 3 * 16_000,
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
}
