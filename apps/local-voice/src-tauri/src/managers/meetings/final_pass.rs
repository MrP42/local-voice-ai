//! M2-P2d: Enddurchlauf nach dem Stopp einer Besprechung.
//!
//! ```text
//! stop(): Live-Worker leer ─► Plan (Einstellung `meeting_final_model`, GPU, RAM, VRAM)
//!    Keep ─► ready + TranscriptFinal sofort (Live = Ende)
//!    Run  ─► Job-Thread: [Nachholen (Recovery)] ─► Endmodell laden ─► je Kanal
//!            WAV streamen ─► VAD-Segmente (max 25 s, Qwen 18 s) ─► STT ─► Filter
//!            ─► transcript_live.json (Temp + Rename) ─► replace_segments (EINE
//!            Transaktion, Epoche + 1) ─► Reset + Segments ─► ready ─► TranscriptFinal
//! ```
//!
//! Grundsatz: das Live-Transkript ist immer gueltig. Jeder Fehler (Modell
//! fehlt, zu wenig Speicher, Abbruch, Konflikt mit einer Korrektur von Hand,
//! volle Platte, Panik) laesst es unveraendert stehen; die Besprechung wird
//! trotzdem `ready`, und `TranscriptFinal` kommt (dann mit `model: None` und
//! dem Hinweis `final_pass_skipped`). Ersetzt wird nur in einer einzigen
//! DB-Transaktion, und erst nachdem die Live-Kopie sicher auf der Platte liegt.
//!
//! Recovery (Konzept 3.6): eine Besprechung, die beim Absturz auf `recording`
//! oder `processing` stand, wird hier nachgeholt: je Kanal der Rest ab dem
//! letzten gespeicherten Segment, dann der Enddurchlauf (falls aktiv).
//!
//! M3-P3b: Sprechertrennung. Vor dem End-STT (ein grosses Modell zur Zeit)
//! liefert `speakers::collect_turns` die Turns je Kanal (Gegenseite,
//! Import, optional "mehrere Personen am Mikrofon"); nach dem End-STT ordnet
//! `diarize::assign` jedes Wort einem Sprecher zu und teilt Segmente an
//! Sprecherwechseln, danach schreibt EINE Transaktion Segmente, Turns
//! (`speaker_hints_json`) und `speakers`-Zeilen. Bleibt das Live-Transkript
//! stehen (Einstellung `off`, nur CPU, Fehler), gilt dieselbe Zuordnung fuer
//! das Live-Transkript (`speakers::apply_to_stored`). Ein Fehler im Sprecher-
//! Schritt (Modell fehlt, wenig RAM, Panik) laesst das Transkript, wie es ist:
//! die Besprechung wird `ready`, die Labels bleiben "Ich" / "Gegenseite".
//!
//! Alles, was Tauri, das Modell oder die Hardware braucht, steckt hinter
//! [`FinalEnv`]; die Ablaeufe selbst sind ohne Geraet und ohne Modell testbar.

use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use log::{error, info, warn};
use serde::Serialize;

use super::chunker::{ChannelChunker, Chunk};
use super::diarize::assign::assign_segments;
use super::dsp::{live_segments, DspStats, SegmentMeta, VadFactory, FALLBACK_CHUNK_MS};
use super::recorder::{MeetingEvent, CHANNEL_MIC, CHANNEL_SYSTEM};
use super::segmenter::{Segment, SegmenterConfig, VadSegmenter};
use super::speakers::{self, ApplyOutcome, ChannelDiarizer, StepReport, TurnSet};
use super::store::{
    Meeting, MeetingStatus, MeetingStore, ReplaceError, StoredSegment, TranscriptDelta,
};
use crate::managers::transcription::TimedSegment;

/// Werte der Einstellung `meeting_final_model` neben einer Modell-ID.
pub const FINAL_AUTO: &str = "auto";
pub const FINAL_OFF: &str = "off";
/// Hoechstlaenge eines Segments im Enddurchlauf (Konzept 3.2).
pub const FINAL_MAX_SEGMENT_MS: u64 = 25_000;
/// Qwen3-ASR scheitert an langen Bloecken (256-Token-Deckel in transcribe-cpp).
pub const QWEN_MAX_SEGMENT_MS: u64 = 18_000;
/// Mindestens so viel freier Grafikspeicher, bevor ein Endmodell auf die GPU geht.
pub const MIN_FREE_VRAM_MB: u64 = 4 * 1024;
/// Die Live-Kopie vor dem Ersatz, im Besprechungsordner (keine DB-Spalte, B5).
pub const LIVE_TRANSCRIPT_FILE: &str = "transcript_live.json";
/// `metadata_json`-Schluessel des Abschlussberichts.
pub const REPORT_KEY: &str = "final_pass";
/// Hinweis-Code (MeetingEvent::Error) fuer einen uebersprungenen Enddurchlauf.
pub const SKIPPED_CODE: &str = "final_pass_skipped";
/// Import-Spur ohne Kanaltrennung (wie `retranscribe.rs`).
const CHANNEL_MIXED: u8 = 2;
/// `remap_sources`: ohne Ueberlappung zaehlt das naechste Segment, wenn es
/// hoechstens so weit entfernt liegt.
const REMAP_MAX_GAP_MS: u64 = 2_000;
/// Blockgroesse beim Lesen einer WAV (1 s): der Speicher bleibt bei einem
/// Segment plus einer Sekunde, egal wie lang die Besprechung ist.
const READ_BLOCK: usize = 16_000;
const SAMPLES_PER_MS: u64 = 16;
/// Wie oft ein Segment versucht wird, bevor der Enddurchlauf aufgibt.
const TRANSCRIBE_ATTEMPTS: u32 = 3;

/// Endmodelle fuer `auto` mit GPU, in dieser Reihenfolge (Befund B1):
/// Whisper large-v3 (4,65 % WER, RTF ~20 auf der 4090), sonst Qwen3-ASR 1.7B.
/// (Repo-Praefix, bevorzugte Datei): jede Quantisierung des Repos zaehlt,
/// die bevorzugte zuerst.
pub const AUTO_GPU_CANDIDATES: &[(&str, &str)] = &[
    (
        "handy-computer/whisper-large-v3-gguf/",
        "whisper-large-v3-Q5_K_M.gguf",
    ),
    (
        "handy-computer/Qwen3-ASR-1.7B-gguf/",
        "Qwen3-ASR-1.7B-Q5_K_M.gguf",
    ),
];

// ---------------------------------------------------------------------------
// Modellwahl (rein)
// ---------------------------------------------------------------------------

/// Die Einstellung `meeting_final_model`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinalChoice {
    Auto,
    Off,
    Model(String),
}

impl FinalChoice {
    /// Leer gilt als `auto` (aeltere settings.json ohne den Schluessel).
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "" | FINAL_AUTO => FinalChoice::Auto,
            FINAL_OFF => FinalChoice::Off,
            id => FinalChoice::Model(id.to_string()),
        }
    }
}

/// Was der Rechner fuer den Enddurchlauf hergibt.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hardware {
    /// transcribe-cpp hat ein GPU-Geraet und die Einstellung erlaubt es.
    pub gpu: bool,
    /// Freier Grafikspeicher (MB); `None` = nicht messbar.
    pub free_vram_mb: Option<u64>,
    /// Freier Arbeitsspeicher (MB); `None` = nicht messbar.
    pub free_ram_mb: Option<u64>,
}

/// Warum das Live-Transkript das Endtranskript bleibt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepReason {
    /// Einstellung `off`.
    Off,
    /// `auto` ohne GPU: das Live-Transkript ist schon das beste erreichbare.
    CpuOnly,
    /// `auto` mit GPU, aber keines der Endmodelle ist installiert.
    NoModel,
    /// Das eingestellte Modell ist nicht installiert.
    NotInstalled,
    LowRam,
    LowVram,
    /// Keine lesbare Aufnahme (geloescht, Aufbewahrung abgelaufen).
    NoAudio,
    LoadFailed,
    TranscribeFailed,
    /// Das Endmodell lieferte nichts, das Live-Transkript hatte Text.
    EmptyResult,
    /// Das Transkript wurde waehrend des Laufs geaendert (Korrektur von Hand).
    Conflict,
    /// `transcript_live.json` liess sich nicht schreiben (z. B. Platte voll).
    BackupFailed,
    StoreFailed,
    /// Eine neue Aufnahme hat den Lauf abgebrochen.
    Cancelled,
    Panic,
    /// Der Enddurchlauf lief schon (Absturz nach dem Ersatz, vor `ready`).
    AlreadyFinal,
    /// Nur Recovery: Import/Untertitel bekommen keinen Enddurchlauf.
    NotLive,
}

impl KeepReason {
    /// Ohne Hinweis an den Nutzer: der Live-Stand ist hier gewollt das Ende.
    pub fn is_by_design(self) -> bool {
        matches!(
            self,
            KeepReason::Off
                | KeepReason::CpuOnly
                | KeepReason::NoModel
                | KeepReason::AlreadyFinal
                | KeepReason::NotLive
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinalPlan {
    Keep(KeepReason),
    Run {
        model_id: String,
        max_segment_ms: u64,
    },
}

/// RAM-Bedarf eines Endmodells (MB): Datei x 1,5 plus Arbeitspuffer.
pub fn ram_need_mb(size_mb: u64) -> u64 {
    (size_mb.saturating_mul(3) / 2 + 512).max(1_024)
}

fn is_qwen(model_id: &str) -> bool {
    model_id.to_ascii_lowercase().contains("qwen3-asr")
}

pub fn max_segment_ms_for(model_id: &str) -> u64 {
    if is_qwen(model_id) {
        QWEN_MAX_SEGMENT_MS
    } else {
        FINAL_MAX_SEGMENT_MS
    }
}

fn auto_candidate(installed: &[(String, u64)]) -> Option<&(String, u64)> {
    AUTO_GPU_CANDIDATES.iter().find_map(|(prefix, preferred)| {
        let exact = format!("{prefix}{preferred}");
        installed
            .iter()
            .find(|(id, _)| *id == exact)
            .or_else(|| installed.iter().find(|(id, _)| id.starts_with(prefix)))
    })
}

/// Welches Modell rechnet den Enddurchlauf, oder warum keins.
/// `installed`: (Modell-ID, Groesse in MB) der heruntergeladenen Modelle.
///
/// - `off` -> Keep(Off).
/// - `auto` ohne GPU -> Keep(CpuOnly): Live = Ende, kein zweiter Lauf.
/// - `auto` mit GPU -> Whisper large-v3, sonst Qwen3-ASR 1.7B; keins -> Keep(NoModel).
/// - Modell-ID -> dieses Modell, auch nur auf der CPU (bewusste Wahl).
///
/// Dann die Speicher-Tore: RAM (Bedarf + Systemreserve) und, wenn eine GPU
/// rechnet, `MIN_FREE_VRAM_MB`. Nicht Messbares blockiert nicht (wie
/// `process_guard::check_ram_for_start`).
pub fn plan_final_pass(
    choice: &FinalChoice,
    hw: &Hardware,
    installed: &[(String, u64)],
) -> FinalPlan {
    let (model_id, size_mb) = match choice {
        FinalChoice::Off => return FinalPlan::Keep(KeepReason::Off),
        FinalChoice::Auto if !hw.gpu => return FinalPlan::Keep(KeepReason::CpuOnly),
        FinalChoice::Auto => match auto_candidate(installed) {
            Some((id, size)) => (id.clone(), *size),
            None => return FinalPlan::Keep(KeepReason::NoModel),
        },
        FinalChoice::Model(id) => match installed.iter().find(|(i, _)| i == id) {
            Some((id, size)) => (id.clone(), *size),
            None => return FinalPlan::Keep(KeepReason::NotInstalled),
        },
    };
    if let Some(free) = hw.free_ram_mb {
        if free < ram_need_mb(size_mb) + crate::process_guard::RAM_RESERVE_MB {
            return FinalPlan::Keep(KeepReason::LowRam);
        }
    }
    if hw.gpu {
        if let Some(free) = hw.free_vram_mb {
            if free < MIN_FREE_VRAM_MB {
                return FinalPlan::Keep(KeepReason::LowVram);
            }
        }
    }
    FinalPlan::Run {
        max_segment_ms: max_segment_ms_for(&model_id),
        model_id,
    }
}

// ---------------------------------------------------------------------------
// Belege nachziehen (rein)
// ---------------------------------------------------------------------------

fn overlap_ms(a: &StoredSegment, b: &StoredSegment) -> u64 {
    a.end_ms.min(b.end_ms).saturating_sub(a.start_ms.max(b.start_ms))
}

fn gap_ms(a: &StoredSegment, b: &StoredSegment) -> u64 {
    if a.end_ms <= b.start_ms {
        b.start_ms - a.end_ms
    } else if b.end_ms <= a.start_ms {
        a.start_ms - b.end_ms
    } else {
        0
    }
}

/// Stabile Belege fuer M1 (Beruehrpunkt B3): bildet `segment_index`-Werte
/// der alten Epoche auf die neue ab. Je Beleg das neue Segment im selben
/// Kanal mit der groessten Zeitueberlappung; ohne Ueberlappung das naechste,
/// wenn es hoechstens 2 s entfernt liegt; sonst entfaellt der Beleg.
/// Gleichstand: kleinerer Index. Reihenfolge wie `ids`, ohne Doppelte.
// Aufrufer ist M1 (KI-Notizen ziehen ihre Belege nach `TranscriptFinal` nach).
#[allow(dead_code)]
pub fn remap_sources(old: &[StoredSegment], new: &[StoredSegment], ids: &[u32]) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(src) = old.iter().find(|s| s.segment_index == *id) else {
            continue;
        };
        let same_channel = || new.iter().filter(|n| n.channel == src.channel);
        let best_overlap = same_channel()
            .map(|n| (overlap_ms(src, n), n.segment_index))
            .filter(|(o, _)| *o > 0)
            .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let target = match best_overlap {
            Some((_, index)) => Some(index),
            None => same_channel()
                .map(|n| (gap_ms(src, n), n.segment_index))
                .filter(|(g, _)| *g <= REMAP_MAX_GAP_MS)
                .min()
                .map(|(_, index)| index),
        };
        if let Some(index) = target {
            if !out.contains(&index) {
                out.push(index);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Umgebung
// ---------------------------------------------------------------------------

/// Was die Ablaeufe von der App brauchen. Produktion: [`AppEnv`]; Tests:
/// Attrappen ohne Modell und ohne Tauri.
pub trait FinalEnv {
    /// Laedt das Modell exklusiv (blockierend) und gibt die Ladezeit in ms zurueck.
    fn load(&mut self, model_id: &str) -> Result<u64, KeepReason>;
    /// Transkribiert einen Block mit genau diesem Modell (laedt nach, falls es
    /// inzwischen verdraengt wurde).
    fn transcribe(&mut self, model_id: &str, chunk: &Chunk) -> Result<Vec<TimedSegment>, String>;
    /// VAD fuer die Offline-Segmentierung; `None` = Rueckfall-Chunker.
    fn vad(&self) -> Option<VadFactory>;
    fn emit(&mut self, event: MeetingEvent);
    /// Eine neue Aufnahme will die Engine: zwischen zwei Segmenten aufhoeren.
    fn cancelled(&self) -> bool;
    /// M3-P3b: die Sprechertrennung dieser Umgebung. Standard: keine (Attrappen
    /// ohne Diarisierer); die App liefert immer einen (auch abgeschaltet, dann
    /// nutzt der Schritt nur gespeicherte Turns).
    fn diarizer(&mut self) -> Option<&mut dyn ChannelDiarizer> {
        None
    }
}

// ---------------------------------------------------------------------------
// Offline-Segmentierung einer Kanal-WAV
// ---------------------------------------------------------------------------

/// Laenge einer 16-kHz-WAV in ms (`None`, wenn nicht lesbar).
pub fn wav_duration_ms(path: &Path) -> Option<u64> {
    hound::WavReader::open(path)
        .ok()
        .map(|r| r.duration() as u64 / SAMPLES_PER_MS)
}

fn to_chunk(seg: Segment, base_ms: u64) -> (Chunk, Option<SegmentMeta>) {
    let meta = SegmentMeta {
        vad_end_ms: base_ms + seg.vad_end_ms,
        decided_at_ms: base_ms + seg.decided_at_ms,
        speech_ms: seg.speech_ms,
        span_ms: seg.span_ms,
    };
    let samples = seg.samples.iter().map(|&s| s as f32 / 32_768.0).collect();
    (
        Chunk {
            samples,
            offset_ms: base_ms + seg.offset_ms,
        },
        Some(meta),
    )
}

enum Cutter {
    Vad(Box<VadSegmenter>),
    Chunker(ChannelChunker),
}

impl Cutter {
    fn new(vad: Option<&VadFactory>, max_segment_ms: u64) -> Self {
        if let Some(factory) = vad {
            match catch_unwind(AssertUnwindSafe(|| factory())) {
                Ok(Ok(detector)) => {
                    let cfg = SegmenterConfig {
                        max_segment_ms,
                        ..SegmenterConfig::default()
                    };
                    return Cutter::Vad(Box::new(VadSegmenter::new(detector, cfg)));
                }
                Ok(Err(e)) => warn!("meetings: final pass VAD not loaded ({e}) - fixed blocks"),
                Err(_) => error!("meetings: final pass VAD load panicked - fixed blocks"),
            }
        }
        Cutter::Chunker(ChannelChunker::new(max_segment_ms.min(FALLBACK_CHUNK_MS)))
    }

    fn push(&mut self, samples: &[i16], base_ms: u64, out: &mut Vec<(Chunk, Option<SegmentMeta>)>) {
        match self {
            Cutter::Vad(seg) => out.extend(seg.push(samples).into_iter().map(|s| to_chunk(s, base_ms))),
            Cutter::Chunker(c) => {
                if let Some(mut chunk) = c.push(samples) {
                    chunk.offset_ms += base_ms;
                    out.push((chunk, None));
                }
            }
        }
    }

    fn flush(&mut self, base_ms: u64, out: &mut Vec<(Chunk, Option<SegmentMeta>)>) {
        match self {
            Cutter::Vad(seg) => out.extend(seg.flush().into_iter().map(|s| to_chunk(s, base_ms))),
            Cutter::Chunker(c) => {
                if let Some(mut chunk) = c.flush() {
                    chunk.offset_ms += base_ms;
                    out.push((chunk, None));
                }
            }
        }
    }
}

/// Wie ein Kanal durchlaufen wurde.
#[derive(Debug, Default)]
struct TrackRun {
    /// Bis hierhin (Kanal-Achse, ms) ist alles erledigt.
    done_ms: u64,
    /// Abgebrochen (`on_segment` lieferte `false`).
    stopped: bool,
}

/// Liest eine Kanal-WAV ab `from_ms` blockweise, segmentiert offline und ruft
/// `on_segment` je Segment (Offset auf der Kanal-Achse). Liefert `on_segment`
/// `false`, endet der Lauf; `done_ms` ist dann der Anfang dieses Segments.
fn segment_wav(
    path: &Path,
    from_ms: u64,
    max_segment_ms: u64,
    vad: Option<&VadFactory>,
    on_segment: &mut dyn FnMut(Chunk, Option<SegmentMeta>) -> bool,
) -> Result<TrackRun, String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = reader.spec();
    if (spec.sample_rate, spec.channels, spec.bits_per_sample) != (16_000, 1, 16)
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err(format!("{}: not 16 kHz mono PCM16", path.display()));
    }
    let total = reader.duration() as u64;
    let start = (from_ms * SAMPLES_PER_MS).min(total);
    reader
        .seek(start as u32)
        .map_err(|e| format!("{}: {e}", path.display()))?;

    let mut cutter = Cutter::new(vad, max_segment_ms);
    let mut pending: Vec<(Chunk, Option<SegmentMeta>)> = Vec::new();
    let mut block: Vec<i16> = Vec::with_capacity(READ_BLOCK);
    let mut samples = reader.samples::<i16>();
    let mut run = TrackRun {
        done_ms: from_ms,
        stopped: false,
    };
    loop {
        block.clear();
        for s in samples.by_ref().take(READ_BLOCK) {
            // Ein kaputter Rest (Absturz mitten im Schreiben) beendet die Spur.
            match s {
                Ok(v) => block.push(v),
                Err(_) => break,
            }
        }
        let last = block.len() < READ_BLOCK;
        cutter.push(&block, from_ms, &mut pending);
        if last {
            cutter.flush(from_ms, &mut pending);
        }
        for (chunk, meta) in pending.drain(..) {
            let offset = chunk.offset_ms;
            if !on_segment(chunk, meta) {
                run.done_ms = offset;
                run.stopped = true;
                return Ok(run);
            }
        }
        if last {
            break;
        }
    }
    run.done_ms = total / SAMPLES_PER_MS;
    Ok(run)
}

// ---------------------------------------------------------------------------
// Eingaenge
// ---------------------------------------------------------------------------

/// Eine Kanal-Aufnahme, die ein Lauf liest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub path: PathBuf,
    pub channel: u8,
}

/// Eingaenge des Enddurchlaufs: Kanal 0 `mic_aec.wav` (ohne Echo, gleiche
/// Achse wie `mic.wav`), wenn sie existiert und der Bericht sie nicht als
/// unvollstaendig meldet, sonst `mic.wav`; Kanal 1 `system.wav`. Nur
/// vorhandene Dateien.
pub fn final_tracks(meeting: &Meeting, metadata: Option<&serde_json::Value>) -> Vec<Track> {
    let mut out = Vec::new();
    if let Some(mic) = meeting.mic_audio_path.as_deref() {
        let mic = PathBuf::from(mic);
        let aec = mic.with_file_name(super::MIC_AEC_FILE);
        let aec_complete = metadata
            .and_then(|m| m.get("aec"))
            .and_then(|a| a.get("wav_kept"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let path = if aec.exists() && aec_complete { aec } else { mic };
        if path.exists() {
            out.push(Track {
                path,
                channel: CHANNEL_MIC,
            });
        }
    }
    if let Some(system) = meeting.system_audio_path.as_deref() {
        let path = PathBuf::from(system);
        if path.exists() {
            out.push(Track {
                path,
                channel: CHANNEL_SYSTEM,
            });
        }
    }
    out
}

/// Eingaenge des Nachholens: die Rohspuren, mit den Kanaelen, die ihre
/// Segmente tragen (Import: eine gemischte Spur, Kanal 2).
fn catch_up_tracks(meeting: &Meeting) -> Vec<Track> {
    let mic_channel = if meeting.source == "import" {
        CHANNEL_MIXED
    } else {
        CHANNEL_MIC
    };
    [
        (meeting.mic_audio_path.as_deref(), mic_channel),
        (meeting.system_audio_path.as_deref(), CHANNEL_SYSTEM),
    ]
    .into_iter()
    .filter_map(|(path, channel)| {
        path.map(|p| Track {
            path: PathBuf::from(p),
            channel,
        })
    })
    .filter(|t| t.path.exists())
    .collect()
}

/// Ordner der Besprechung (neben ihren WAVs).
fn meeting_dir(meeting: &Meeting) -> Option<PathBuf> {
    meeting
        .mic_audio_path
        .as_deref()
        .or(meeting.system_audio_path.as_deref())
        .and_then(|p| Path::new(p).parent().map(Path::to_path_buf))
}

// ---------------------------------------------------------------------------
// Live-Kopie
// ---------------------------------------------------------------------------

#[derive(Serialize, serde::Deserialize)]
struct LiveCopy {
    format: u32,
    meeting_id: String,
    epoch: u32,
    revision: i64,
    model: Option<String>,
    written_at: i64,
    segments: Vec<StoredSegment>,
}

/// Epoche der vorhandenen Live-Kopie (`None`: keine oder unlesbar).
fn live_copy_epoch(dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(dir.join(LIVE_TRANSCRIPT_FILE)).ok()?;
    serde_json::from_str::<LiveCopy>(&text).ok().map(|c| c.epoch)
}

/// Schreibt die Live-Kopie: erst `…tmp` vollstaendig und auf die Platte,
/// dann umbenennen. Ein Absturz hinterlaesst nie eine halbe Datei.
fn write_live_copy(dir: &Path, copy: &LiveCopy) -> std::io::Result<()> {
    let tmp = dir.join(format!("{LIVE_TRANSCRIPT_FILE}.tmp"));
    let json = serde_json::to_vec_pretty(copy).map_err(std::io::Error::other)?;
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, dir.join(LIVE_TRANSCRIPT_FILE)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

// ---------------------------------------------------------------------------
// Ablaeufe
// ---------------------------------------------------------------------------

/// Ein Auftrag fuer den Job-Thread.
#[derive(Clone, Debug)]
pub struct JobSpec {
    pub meeting_id: String,
    /// Recovery: vor dem Enddurchlauf je Kanal den Rest nachholen, mit
    /// diesem Modell (dem Besprechungsmodell).
    pub catch_up_model: Option<String>,
    pub plan: FinalPlan,
    /// Modell des Live-Transkripts (fuer `TranscriptFinal`, wenn es bleibt).
    pub live_model: Option<String>,
}

/// Abschlussbericht (Log, `metadata_json.final_pass`, Simulation). Nur Zahlen.
#[derive(Clone, Debug, Default, Serialize)]
pub struct FinalReport {
    pub model: Option<String>,
    /// Gesetzt, wenn das Live-Transkript blieb.
    pub kept: Option<KeepReason>,
    pub live_epoch: u32,
    pub epoch: u32,
    pub live_segments: usize,
    pub segments: usize,
    /// Segmente, die an die STT gingen (vor dem Filter).
    pub blocks: usize,
    pub audio_ms: u64,
    pub load_ms: u64,
    pub transcribe_ms: u64,
    pub wall_ms: u64,
    /// Audiozeit / reine Rechenzeit der STT.
    pub rtf: Option<f64>,
    pub with_words: bool,
    pub catch_up_segments: usize,
    pub catch_up_gaps: usize,
    /// M3-P3b: Bericht des Sprecher-Schritts (nur wenn er lief).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speakers: Option<StepReport>,
}

/// Nachholen nach einem Absturz: je Kanal alles ab dem groessten `end_ms`
/// dieses Kanals, mit dem Besprechungsmodell, gespeichert wie der Live-Pfad
/// (`append_delta` + `Segments`). Scheitert das Laden, das Transkribieren
/// oder bricht eine neue Aufnahme ab, markiert ein Luecken-Platzhalter den
/// Rest, damit nichts stumm fehlt.
fn catch_up(
    store: &MeetingStore,
    meeting: &Meeting,
    model_id: &str,
    env: &mut dyn FinalEnv,
    report: &mut FinalReport,
) {
    let segments = match store.get_segments(&meeting.id) {
        Ok(s) => s,
        Err(e) => {
            warn!("meetings: catch-up could not read the transcript: {e}");
            return;
        }
    };
    let mut next_index = segments
        .iter()
        .map(|s| s.segment_index + 1)
        .max()
        .unwrap_or(0);
    let vad = env.vad();
    let stats = DspStats::default();
    let mut loaded = false;
    for track in catch_up_tracks(meeting) {
        let from_ms = segments
            .iter()
            .filter(|s| s.channel == track.channel)
            .map(|s| s.end_ms)
            .max()
            .unwrap_or(0);
        let Some(total_ms) = wav_duration_ms(&track.path) else {
            continue;
        };
        // Weniger als eine halbe Sekunde Rest: nichts, was ein Wort traegt.
        if from_ms + 500 >= total_ms {
            continue;
        }
        let mut failed = env.cancelled();
        if !failed && !loaded {
            match env.load(model_id) {
                Ok(_) => loaded = true,
                Err(reason) => {
                    warn!("meetings: catch-up model not loaded ({reason:?})");
                    failed = true;
                }
            }
        }
        let mut done_ms = from_ms;
        if !failed {
            let channel = track.channel;
            let mut on_segment = |chunk: Chunk, meta: Option<SegmentMeta>| -> bool {
                if env.cancelled() {
                    return false;
                }
                let timed = match env.transcribe(model_id, &chunk) {
                    Ok(t) => t,
                    Err(e) => {
                        warn!("meetings: catch-up block at {} ms failed: {e}", chunk.offset_ms);
                        return false;
                    }
                };
                let appended = live_segments(timed, &chunk, channel, meta, &mut next_index, &stats);
                if appended.is_empty() {
                    return true;
                }
                if let Err(e) = store.append_delta(
                    &meeting.id,
                    &TranscriptDelta {
                        new_segments: appended.clone(),
                    },
                ) {
                    warn!("meetings: catch-up delta not stored: {e}");
                    return false;
                }
                report.catch_up_segments += appended.len();
                env.emit(MeetingEvent::Segments {
                    meeting_id: meeting.id.clone(),
                    appended,
                });
                true
            };
            match segment_wav(&track.path, from_ms, 15_000, vad.as_ref(), &mut on_segment) {
                Ok(run) if !run.stopped => continue,
                Ok(run) => done_ms = run.done_ms,
                Err(e) => warn!("meetings: catch-up could not read a track: {e}"),
            }
        }
        // Rest als Luecke markieren (Muster `import.rs`).
        let gap = StoredSegment {
            segment_index: next_index,
            text: super::import::gap_placeholder(done_ms, total_ms),
            start_ms: done_ms,
            end_ms: total_ms,
            channel: track.channel,
            speaker_index: None,
            words: None,
        };
        match store.append_delta(
            &meeting.id,
            &TranscriptDelta {
                new_segments: vec![gap.clone()],
            },
        ) {
            Ok(_) => {
                next_index += 1;
                report.catch_up_gaps += 1;
                env.emit(MeetingEvent::Segments {
                    meeting_id: meeting.id.clone(),
                    appended: vec![gap],
                });
            }
            Err(e) => warn!("meetings: catch-up gap marker not stored: {e}"),
        }
    }
}

/// Der eigentliche Enddurchlauf. `Ok(epoch)` = ersetzt; `Err` = das
/// Live-Transkript bleibt unveraendert.
fn final_pass(
    store: &MeetingStore,
    meeting: &Meeting,
    model_id: &str,
    max_segment_ms: u64,
    env: &mut dyn FinalEnv,
    report: &mut FinalReport,
    turns: Option<&TurnSet>,
) -> Result<u32, KeepReason> {
    let metadata = store.metadata_json(&meeting.id).ok().flatten();
    let tracks = final_tracks(meeting, metadata.as_ref());
    let dir = meeting_dir(meeting).ok_or(KeepReason::NoAudio)?;
    if tracks.is_empty() {
        return Err(KeepReason::NoAudio);
    }
    let snapshot = store
        .transcript_snapshot(&meeting.id)
        .map_err(|_| KeepReason::StoreFailed)?;
    report.live_epoch = snapshot.epoch;
    report.live_segments = snapshot.segments.len();
    // Absturz nach dem Ersatz, aber vor `ready`: die Kopie haelt die alte
    // Epoche, die DB schon die neue. Nie die Live-Kopie mit dem Endtranskript
    // ueberschreiben.
    if live_copy_epoch(&dir).is_some_and(|e| e < snapshot.epoch) {
        return Err(KeepReason::AlreadyFinal);
    }
    if env.cancelled() {
        return Err(KeepReason::Cancelled);
    }
    report.load_ms = env.load(model_id)?;

    let vad = env.vad();
    let stats = DspStats::default();
    let mut fresh: Vec<StoredSegment> = Vec::new();
    let mut failure: Option<KeepReason> = None;
    for track in &tracks {
        let channel = track.channel;
        let mut on_segment = |chunk: Chunk, meta: Option<SegmentMeta>| -> bool {
            if env.cancelled() {
                failure = Some(KeepReason::Cancelled);
                return false;
            }
            let started = Instant::now();
            let timed = match env.transcribe(model_id, &chunk) {
                Ok(t) => t,
                Err(e) => {
                    warn!(
                        "meetings: final pass block at {} ms failed ({} ms audio): {e}",
                        chunk.offset_ms,
                        chunk.samples.len() as u64 / SAMPLES_PER_MS
                    );
                    failure = Some(KeepReason::TranscribeFailed);
                    return false;
                }
            };
            report.transcribe_ms += started.elapsed().as_millis() as u64;
            report.blocks += 1;
            report.audio_ms += chunk.samples.len() as u64 / SAMPLES_PER_MS;
            let mut index = 0;
            fresh.extend(live_segments(timed, &chunk, channel, meta, &mut index, &stats));
            true
        };
        match segment_wav(&track.path, 0, max_segment_ms, vad.as_ref(), &mut on_segment) {
            Ok(run) if run.stopped => {
                return Err(failure.unwrap_or(KeepReason::TranscribeFailed));
            }
            Ok(_) => {}
            Err(e) => {
                warn!("meetings: final pass could not read a track: {e}");
                return Err(KeepReason::NoAudio);
            }
        }
    }
    if env.cancelled() {
        return Err(KeepReason::Cancelled);
    }
    if fresh.is_empty() && !snapshot.segments.is_empty() {
        return Err(KeepReason::EmptyResult);
    }

    // M3-P3b: Wort -> Sprecher, Segmente an Sprecherwechseln teilen. Vor dem
    // Nummerieren, damit es EINE Epoche gibt und die Indizes der geteilten
    // Segmente stimmen.
    if let Some(ts) = turns {
        let (assigned, stats) = assign_segments(std::mem::take(&mut fresh), &ts.channels);
        fresh = assigned;
        if let Some(step) = report.speakers.as_mut() {
            step.assigned = stats.assigned;
            step.unassigned = stats.unassigned;
            step.split_added = stats.split_added;
        }
    }

    // Kanaele ineinander, zeitlich geordnet; Indizes der neuen Epoche ab 0.
    fresh.sort_by_key(|s| (s.start_ms, s.channel, s.end_ms));
    for (i, s) in fresh.iter_mut().enumerate() {
        s.segment_index = i as u32;
    }
    let with_words = fresh.iter().any(|s| s.words.is_some());
    report.with_words = with_words;

    let copy = LiveCopy {
        format: 1,
        meeting_id: meeting.id.clone(),
        epoch: snapshot.epoch,
        revision: snapshot.revision,
        model: snapshot.model.clone(),
        written_at: chrono::Utc::now().timestamp(),
        segments: snapshot.segments,
    };
    if let Err(e) = write_live_copy(&dir, &copy) {
        warn!("meetings: {LIVE_TRANSCRIPT_FILE} not written ({e}) - live transcript kept");
        return Err(KeepReason::BackupFailed);
    }

    let granularity = if with_words { "word@1" } else { "segment@1" };
    let replaced = match turns {
        // Segmente, Turns und Sprecherzeilen in EINER Transaktion.
        Some(ts) => {
            let write = speakers::speaker_write(store, &meeting.id, ts, &fresh);
            store.replace_segments_with_speakers(
                &meeting.id,
                &fresh,
                model_id,
                granularity,
                snapshot.revision,
                &write,
            )
        }
        None => store.replace_segments(
            &meeting.id,
            &fresh,
            model_id,
            granularity,
            snapshot.revision,
        ),
    };
    let epoch = replaced.map_err(|e| {
        warn!("meetings: final transcript not stored ({e}) - live transcript kept");
        match e {
            ReplaceError::Conflict { .. } => KeepReason::Conflict,
            ReplaceError::Store(_) => KeepReason::StoreFailed,
        }
    })?;
    if let Some(step) = report.speakers.as_mut() {
        step.applied = turns.is_some();
    }
    report.segments = fresh.len();
    env.emit(MeetingEvent::Reset {
        meeting_id: meeting.id.clone(),
    });
    env.emit(MeetingEvent::Segments {
        meeting_id: meeting.id.clone(),
        appended: fresh,
    });
    Ok(epoch)
}

/// M3-P3b: Turns je Kanal holen, bevor das End-STT-Modell geladen wird. Kein
/// Fehler und keine Panik dieses Schritts darf den Enddurchlauf verhindern:
/// dann `None`, und alles laeuft wie bisher.
fn turns_before_stt(
    store: &MeetingStore,
    meeting: &Meeting,
    plan: &FinalPlan,
    env: &mut dyn FinalEnv,
    report: &mut FinalReport,
) -> Option<TurnSet> {
    let metadata = store.metadata_json(&meeting.id).ok().flatten();
    let tracks = speakers::diarize_tracks(meeting, metadata.as_ref());
    if tracks.is_empty() {
        return None;
    }
    // Bleibt das Live-Transkript stehen und ist leer, gibt es nichts zuzuordnen.
    if matches!(plan, FinalPlan::Keep(_))
        && store
            .get_segments(&meeting.id)
            .map_or(true, |s| s.is_empty())
    {
        return None;
    }
    let diarizer = env.diarizer()?;
    let mut step = StepReport::default();
    let collected = catch_unwind(AssertUnwindSafe(|| {
        speakers::collect_turns(
            store,
            meeting,
            metadata.as_ref(),
            &tracks,
            true,
            diarizer,
            &mut step,
        )
    }));
    let set = match collected {
        Ok(Ok(set)) => set,
        Ok(Err(speakers::Cancelled)) => None,
        Err(_) => {
            error!(
                "meetings: speaker step panicked ({}) - transcript without speakers",
                meeting.id
            );
            step.state = "aborted".into();
            None
        }
    };
    report.speakers = Some(step);
    set
}

/// M3-P3b: das Live-Transkript bleibt (kein oder gescheiterter Enddurchlauf):
/// Sprecher darauf anwenden und die Anzeige neu laden lassen. Nie ein Fehler:
/// bei Abbruch, Konflikt oder Store-Fehler bleibt das Transkript, wie es ist.
fn apply_speakers_to_live(
    store: &MeetingStore,
    meeting: &Meeting,
    turns: Option<&TurnSet>,
    env: &mut dyn FinalEnv,
    report: &mut FinalReport,
) {
    let Some(ts) = turns else { return };
    if env.cancelled() {
        return;
    }
    let step = report.speakers.get_or_insert_with(StepReport::default);
    match speakers::apply_to_stored(store, &meeting.id, ts, step) {
        ApplyOutcome::Applied { .. } => {
            if let Ok(all) = store.get_segments(&meeting.id) {
                env.emit(MeetingEvent::Reset {
                    meeting_id: meeting.id.clone(),
                });
                env.emit(MeetingEvent::Segments {
                    meeting_id: meeting.id.clone(),
                    appended: all,
                });
            }
        }
        ApplyOutcome::Unchanged => {}
        ApplyOutcome::Conflict => {
            warn!(
                "meetings: speakers not applied ({}): transcript kept changing",
                meeting.id
            )
        }
        ApplyOutcome::Failed(e) => warn!("meetings: speakers not applied ({}): {e}", meeting.id),
    }
}

/// Ergebnis eines Auftrags.
#[derive(Clone, Debug)]
pub struct JobOutcome {
    pub report: FinalReport,
    /// Was `TranscriptFinal` meldet.
    pub epoch: u32,
    pub model: Option<String>,
}

/// Fuehrt einen Auftrag aus und schliesst die Besprechung in JEDEM Fall ab:
/// Status `ready`, `State`, bei einem unerwarteten Verbleib des
/// Live-Transkripts der Hinweis `final_pass_skipped`, dann `TranscriptFinal`
/// (danach starten KI-Notizen und Indexer). Eine Panik im Ablauf (VAD,
/// Engine) aendert daran nichts.
pub fn run_job(store: &MeetingStore, job: &JobSpec, env: &mut dyn FinalEnv) -> JobOutcome {
    let started = Instant::now();
    let mut report = FinalReport::default();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let meeting = match store.get_meeting(&job.meeting_id) {
            Ok(Some(m)) if m.deleted_at.is_none() => m,
            _ => return Err(KeepReason::StoreFailed),
        };
        if let Some(model) = &job.catch_up_model {
            catch_up(store, &meeting, model, env, &mut report);
        }
        // M3-P3b: die Turns VOR dem End-STT holen (ein grosses Modell zur Zeit).
        let turns = turns_before_stt(store, &meeting, &job.plan, env, &mut report);
        let outcome = match &job.plan {
            FinalPlan::Keep(reason) => Err(*reason),
            FinalPlan::Run {
                model_id,
                max_segment_ms,
            } => final_pass(
                store,
                &meeting,
                model_id,
                *max_segment_ms,
                env,
                &mut report,
                turns.as_ref(),
            )
            .map(|epoch| (epoch, model_id.clone())),
        };
        if outcome.is_err() {
            // Das Live-Transkript bleibt: die Sprecher kommen auf dieses.
            apply_speakers_to_live(store, &meeting, turns.as_ref(), env, &mut report);
        }
        outcome
    }));
    let result = result.unwrap_or_else(|_| {
        error!("meetings: final pass panicked ({}) - live transcript kept", job.meeting_id);
        Err(KeepReason::Panic)
    });
    report.wall_ms = started.elapsed().as_millis() as u64;
    if report.transcribe_ms > 0 {
        report.rtf = Some(report.audio_ms as f64 / report.transcribe_ms as f64);
    }

    let (epoch, model) = match result {
        Ok((epoch, model)) => {
            report.epoch = epoch;
            report.model = Some(model.clone());
            (epoch, Some(model))
        }
        Err(reason) => {
            report.kept = Some(reason);
            let epoch = store.segment_epoch(&job.meeting_id).unwrap_or(0);
            report.epoch = epoch;
            // Gewollt Live = Ende: das Live-Modell ist das Endmodell. Sonst
            // (Fehler) meldet `model: None` "Enddurchlauf uebersprungen".
            let model = reason.is_by_design().then(|| job.live_model.clone()).flatten();
            if !reason.is_by_design() {
                warn!(
                    "meetings: final pass skipped for {} ({reason:?}) - live transcript stays",
                    job.meeting_id
                );
                env.emit(MeetingEvent::Error {
                    meeting_id: job.meeting_id.clone(),
                    message: SKIPPED_CODE.to_string(),
                });
            }
            (epoch, model)
        }
    };

    if let Some(step) = report.speakers.as_ref() {
        // Endstand (Zuordnung, geschrieben ja/nein) statt der Zwischenstaende.
        speakers::write_report(store, &job.meeting_id, step);
    }
    if let Ok(value) = serde_json::to_value(&report) {
        if let Err(e) = store.set_metadata_key(&job.meeting_id, REPORT_KEY, value) {
            warn!("meetings: final pass report not stored: {e}");
        }
    }
    if let Err(e) = store.set_status(&job.meeting_id, MeetingStatus::Ready) {
        warn!("meetings: status 'ready' not stored: {e}");
    }
    env.emit(MeetingEvent::State {
        meeting_id: job.meeting_id.clone(),
        status: "ready".to_string(),
        paused: false,
    });
    // Epoche aus der DB (nach einem Ersatz die neue).
    env.emit(super::dsp::transcript_final_event(
        store,
        &job.meeting_id,
        model.clone(),
    ));
    info!(
        "meetings: final pass done ({}): kept={:?} epoch={} segments={} blocks={} audio_ms={} load_ms={} transcribe_ms={} wall_ms={} catch_up={}/{}",
        job.meeting_id,
        report.kept,
        report.epoch,
        report.segments,
        report.blocks,
        report.audio_ms,
        report.load_ms,
        report.transcribe_ms,
        report.wall_ms,
        report.catch_up_segments,
        report.catch_up_gaps
    );
    JobOutcome {
        report,
        epoch,
        model,
    }
}

// ---------------------------------------------------------------------------
// App-Anbindung
// ---------------------------------------------------------------------------

/// Heruntergeladene Modelle als (ID, Groesse MB).
pub fn installed_models(app: &tauri::AppHandle) -> Vec<(String, u64)> {
    use tauri::Manager;
    app.try_state::<Arc<crate::managers::model::ModelManager>>()
        .map(|m| {
            m.get_available_models()
                .into_iter()
                .filter(|i| i.is_downloaded)
                .map(|i| (i.id, i.size_mb))
                .collect()
        })
        .unwrap_or_default()
}

/// Misst, was der Rechner jetzt hergibt (GPU-Geraete von transcribe-cpp mit
/// frischem Speicherstand; DXGI, wenn das Backend keinen meldet).
pub fn probe_hardware(app: &tauri::AppHandle) -> Hardware {
    let settings = crate::settings::get_settings(app);
    let allowed =
        settings.transcribe_accelerator != crate::settings::TranscribeAcceleratorSetting::Cpu;
    let gpus = crate::managers::transcription::transcribe_gpu_memory();
    let gpu = allowed && !gpus.is_empty();
    let mut free_vram_mb = gpus.iter().map(|(_, free)| *free).filter(|f| *f > 0).max();
    if gpu && free_vram_mb.is_none() {
        free_vram_mb = crate::managers::llm::resources::system_memory()
            .gpus
            .iter()
            .filter(|g| !g.shared)
            .map(|g| g.budget_mb.saturating_sub(g.used_mb))
            .max();
    }
    let ram = crate::process_guard::available_ram_mb();
    Hardware {
        gpu,
        free_vram_mb,
        free_ram_mb: (ram > 0).then_some(ram),
    }
}

/// Recovery, erster Schritt je Besprechung (ohne Modell, schnell): WAV-Header
/// aus der Dateigroesse reparieren (auch `mic_aec.wav`, sonst stuende ihr
/// Header auf dem Stand der letzten Sekunde vor dem Absturz), die echte
/// `duration_ms` aus den reparierten WAVs speichern (#15) und den Status auf
/// `processing` setzen. `Err` nur, wenn der Status nicht gespeichert wurde.
pub fn prepare_orphan(store: &MeetingStore, meeting: &Meeting) -> Result<Option<u64>, String> {
    use crate::audio_toolkit::audio::wav_writer::repair_orphan_wav;
    let derived: Vec<String> = meeting
        .mic_audio_path
        .as_deref()
        .map(super::derived_audio_paths)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| Path::new(p).exists())
        .collect();
    for path in [&meeting.mic_audio_path, &meeting.system_audio_path]
        .into_iter()
        .flatten()
        .cloned()
        .chain(derived)
    {
        match repair_orphan_wav(Path::new(&path)) {
            Ok(Some(ms)) => log::debug!("meetings: repaired orphan wav ({ms} ms)"),
            Ok(None) => {}
            Err(e) => warn!("meetings: orphan wav repair failed: {e}"),
        }
    }
    let duration_ms = [&meeting.mic_audio_path, &meeting.system_audio_path]
        .into_iter()
        .flatten()
        .filter_map(|p| wav_duration_ms(Path::new(p)))
        .max()
        .filter(|ms| *ms > 0);
    if duration_ms.is_some() {
        if let Err(e) = store.set_audio_paths(
            &meeting.id,
            meeting.mic_audio_path.as_deref(),
            meeting.system_audio_path.as_deref(),
            duration_ms,
        ) {
            warn!("meetings: orphan duration not stored: {e}");
        }
    }
    store
        .set_status(&meeting.id, MeetingStatus::Processing)
        .map_err(|e| e.to_string())?;
    Ok(duration_ms)
}

/// Plan aus der Einstellung und der Hardware, fuer eine Live-Besprechung.
pub fn plan_for_app(app: &tauri::AppHandle, choice: &FinalChoice) -> FinalPlan {
    if *choice == FinalChoice::Off {
        return FinalPlan::Keep(KeepReason::Off);
    }
    plan_final_pass(choice, &probe_hardware(app), &installed_models(app))
}

/// M3-P3b: Sortformer fuer die App. Das Modell kommt aus dem Katalogordner
/// (`<app_data>/models/diarization`), die Einstellung `meeting_diarization`
/// schaltet ab, der Abbruch-Merker der Engine bricht einen laufenden Kanal ab
/// (ein Wachthund-Thread reicht ihn an `transcribe_cpp::CancelToken` weiter).
/// Das Modell laeuft im App-Prozess (FFI): ein nativer Absturz reisst die App
/// mit, deshalb die Absturzmarke in `speakers::collect_turns`. Es ist hoechstens
/// eines geladen (`diarize::engine::ExclusiveSlot`) und wird nach den Kanaelen
/// freigegeben (`release`), bevor das End-STT-Modell kommt.
pub struct AppDiarizer {
    enabled: bool,
    model_path: PathBuf,
    threads: usize,
    cancel: Arc<AtomicBool>,
    loaded: Option<super::diarize::Diarizer>,
}

/// So oft fragt der Wachthund den Abbruch-Merker ab.
const CANCEL_POLL: std::time::Duration = std::time::Duration::from_millis(100);

impl AppDiarizer {
    /// Einstellung und Modellpfad der App; `cancel` ist der Merker der Engine.
    pub fn from_app(app: &tauri::AppHandle, cancel: Arc<AtomicBool>) -> Self {
        let enabled = crate::settings::meeting_diarization_enabled(
            &crate::settings::get_settings(app).meeting_diarization,
        );
        let models_root = crate::portable::app_data_dir(app)
            .map(|d| d.join("models"))
            .unwrap_or_default();
        Self {
            enabled,
            model_path: super::diarize::resolve_model_path(&models_root, None),
            threads: super::diarize::default_threads(),
            cancel,
            loaded: None,
        }
    }
}

impl ChannelDiarizer for AppDiarizer {
    fn model_name(&self) -> String {
        speakers::HINTS_MODEL_NAME.to_string()
    }

    fn unavailable(&self) -> Option<&'static str> {
        if !self.enabled {
            Some("disabled")
        } else if !self.model_path.is_file() {
            Some("model_missing")
        } else {
            None
        }
    }

    fn check_ram(&mut self, need_mb: u64) -> Result<(), String> {
        crate::process_guard::check_ram_for_start(need_mb).map(|_| ())
    }

    fn diarize(
        &mut self,
        _channel: u8,
        pcm: &[f32],
    ) -> Result<Vec<super::diarize::Turn>, super::diarize::DiarizeError> {
        use super::diarize::{DiarizeError, DiarizeParams, Diarizer};
        if self.cancel.load(Ordering::Relaxed) {
            return Err(DiarizeError::Cancelled);
        }
        if self.loaded.is_none() {
            self.loaded = Some(Diarizer::load(&self.model_path, self.threads)?);
        }
        let Some(diarizer) = self.loaded.as_mut() else {
            return Err(DiarizeError::Load("diarizer not loaded".into()));
        };
        let token = transcribe_cpp::CancelToken::new();
        let mut params = DiarizeParams::new(&self.model_path);
        params.threads = self.threads;
        params.cancel = Some(token.clone());
        let done = AtomicBool::new(false);
        let cancel = &self.cancel;
        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        token.cancel();
                        break;
                    }
                    std::thread::sleep(CANCEL_POLL);
                }
            });
            let result = diarizer.diarize(pcm, &params);
            done.store(true, Ordering::Relaxed);
            result
        })
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn release(&mut self) {
        self.loaded = None;
    }
}

/// Produktionsumgebung: der `TranscriptionManager` der App.
pub struct AppEnv {
    app: tauri::AppHandle,
    tm: Arc<crate::managers::transcription::TranscriptionManager>,
    cancel: Arc<AtomicBool>,
    vad: Option<VadFactory>,
    diarizer: AppDiarizer,
}

/// So lange wartet ein Laden auf einen laufenden fremden Ladevorgang.
const LOAD_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

impl AppEnv {
    pub fn new(
        app: &tauri::AppHandle,
        tm: Arc<crate::managers::transcription::TranscriptionManager>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            app: app.clone(),
            tm,
            diarizer: AppDiarizer::from_app(app, Arc::clone(&cancel)),
            cancel,
            vad: super::recorder::meeting_vad_factory(app),
        }
    }

    fn size_mb(&self, model_id: &str) -> u64 {
        installed_models(&self.app)
            .into_iter()
            .find(|(id, _)| id == model_id)
            .map(|(_, size)| size)
            .unwrap_or(0)
    }

    fn is_current(&self, model_id: &str) -> bool {
        self.tm.get_current_model().as_deref() == Some(model_id)
    }
}

impl FinalEnv for AppEnv {
    /// RAM-Tor, dann exklusiv laden: der Lade-Merker der Engine wird gehalten,
    /// damit kein anderer Pfad gleichzeitig ein zweites Modell laedt und
    /// `transcribe_segments` auf das Ende wartet. Das alte Modell gibt
    /// `load_model` vor dem neuen frei (nie zwei grosse Modelle im Speicher).
    fn load(&mut self, model_id: &str) -> Result<u64, KeepReason> {
        let started = Instant::now();
        if let Err(e) = crate::process_guard::check_ram_for_start(ram_need_mb(self.size_mb(model_id))) {
            warn!("meetings: final model not loaded, RAM gate: {e}");
            return Err(KeepReason::LowRam);
        }
        let guard = loop {
            if let Some(guard) = self.tm.try_start_loading() {
                break guard;
            }
            if self.cancelled() {
                return Err(KeepReason::Cancelled);
            }
            if started.elapsed() > LOAD_WAIT {
                warn!("meetings: final model not loaded, another load never finished");
                return Err(KeepReason::LoadFailed);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let result = self.tm.load_model(model_id);
        drop(guard);
        match result {
            Ok(()) if self.is_current(model_id) => {
                // P2f: auf der GPU legt der erste Lauf nach dem Laden erst
                // seine Pipelines an. Eine Sekunde Stille vorweg, damit das in
                // der Ladezeit steht und nicht im ersten Segment (Ergebnis
                // verworfen).
                let gpu = self
                    .tm
                    .current_backend()
                    .is_some_and(|b| !b.eq_ignore_ascii_case("cpu") && b != "onnx");
                if gpu {
                    let _ = self.tm.transcribe_segments(vec![0.0; 16_000]);
                }
                Ok(started.elapsed().as_millis() as u64)
            }
            Ok(()) => Err(KeepReason::LoadFailed),
            Err(e) => {
                warn!("meetings: final model '{model_id}' not loaded: {e}");
                Err(KeepReason::LoadFailed)
            }
        }
    }

    /// Bis zu drei Versuche. Vor und nach jedem Versuch muss genau dieses
    /// Modell geladen sein: ein Diktat oder Import kann es zwischen zwei
    /// Segmenten verdraengen, dann wird nachgeladen und wiederholt.
    fn transcribe(&mut self, model_id: &str, chunk: &Chunk) -> Result<Vec<TimedSegment>, String> {
        let mut last = String::from("model changed during transcription");
        for _ in 0..TRANSCRIBE_ATTEMPTS {
            if self.cancelled() {
                return Err("cancelled".into());
            }
            if !self.is_current(model_id) {
                self.load(model_id)
                    .map_err(|r| format!("model reload failed: {r:?}"))?;
            }
            match self.tm.transcribe_segments(chunk.samples.clone()) {
                Ok(timed) if self.is_current(model_id) => return Ok(timed),
                Ok(_) => {}
                Err(e) => last = e.to_string(),
            }
        }
        Err(last)
    }

    fn vad(&self) -> Option<VadFactory> {
        self.vad.clone()
    }

    fn emit(&mut self, event: MeetingEvent) {
        use tauri_specta::Event;
        let _ = event.emit(&self.app);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn diarizer(&mut self) -> Option<&mut dyn ChannelDiarizer> {
        Some(&mut self.diarizer)
    }
}

#[cfg(test)]
mod tests {
    use super::super::diarize::{DiarizeError, Turn};
    use super::super::segmenter::test_support::{loud, quiet, EnergyVad};
    use super::super::store::MeetingSource;
    use super::*;
    use crate::audio_toolkit::audio::StreamingWavWriter;
    use crate::managers::transcription::WordTime;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    fn seg(index: u32, channel: u8, start_ms: u64, end_ms: u64) -> StoredSegment {
        StoredSegment {
            segment_index: index,
            text: format!("s{index}"),
            start_ms,
            end_ms,
            channel,
            speaker_index: None,
            words: None,
        }
    }

    // ---- Modellwahl --------------------------------------------------------

    const LARGE: &str = "handy-computer/whisper-large-v3-gguf/whisper-large-v3-Q5_K_M.gguf";
    const QWEN: &str = "handy-computer/Qwen3-ASR-1.7B-gguf/Qwen3-ASR-1.7B-Q5_K_M.gguf";
    const TURBO: &str = "handy-computer/whisper-large-v3-turbo-gguf/whisper-large-v3-turbo-Q8_0.gguf";

    fn installed(ids: &[&str]) -> Vec<(String, u64)> {
        ids.iter().map(|id| (id.to_string(), 1_100)).collect()
    }

    fn gpu() -> Hardware {
        Hardware {
            gpu: true,
            free_vram_mb: Some(20_000),
            free_ram_mb: Some(40_000),
        }
    }

    fn cpu() -> Hardware {
        Hardware {
            gpu: false,
            free_vram_mb: None,
            free_ram_mb: Some(40_000),
        }
    }

    #[test]
    fn the_setting_parses_auto_off_and_model_ids() {
        assert_eq!(FinalChoice::parse("auto"), FinalChoice::Auto);
        assert_eq!(FinalChoice::parse(""), FinalChoice::Auto, "aeltere Einstellungen");
        assert_eq!(FinalChoice::parse(" off "), FinalChoice::Off);
        assert_eq!(FinalChoice::parse(QWEN), FinalChoice::Model(QWEN.into()));
    }

    #[test]
    fn auto_with_a_gpu_prefers_large_v3_then_qwen() {
        let all = installed(&[TURBO, QWEN, LARGE]);
        assert_eq!(
            plan_final_pass(&FinalChoice::Auto, &gpu(), &all),
            FinalPlan::Run {
                model_id: LARGE.into(),
                max_segment_ms: 25_000
            }
        );
        let no_large = installed(&[TURBO, QWEN]);
        assert_eq!(
            plan_final_pass(&FinalChoice::Auto, &gpu(), &no_large),
            FinalPlan::Run {
                model_id: QWEN.into(),
                max_segment_ms: 18_000
            },
            "Qwen nur mit Segmenten <= 18 s"
        );
        // Eine andere Quantisierung desselben Repos zaehlt, turbo nicht.
        let q8 = installed(&[TURBO, "handy-computer/whisper-large-v3-gguf/whisper-large-v3-Q8_0.gguf"]);
        assert!(matches!(
            plan_final_pass(&FinalChoice::Auto, &gpu(), &q8),
            FinalPlan::Run { model_id, .. } if model_id.ends_with("large-v3-Q8_0.gguf")
        ));
        assert_eq!(
            plan_final_pass(&FinalChoice::Auto, &gpu(), &installed(&[TURBO])),
            FinalPlan::Keep(KeepReason::NoModel)
        );
    }

    #[test]
    fn auto_on_the_cpu_keeps_the_live_transcript_but_an_explicit_model_runs() {
        let all = installed(&[LARGE, QWEN]);
        assert_eq!(
            plan_final_pass(&FinalChoice::Auto, &cpu(), &all),
            FinalPlan::Keep(KeepReason::CpuOnly)
        );
        assert_eq!(
            plan_final_pass(&FinalChoice::Model(QWEN.into()), &cpu(), &all),
            FinalPlan::Run {
                model_id: QWEN.into(),
                max_segment_ms: 18_000
            }
        );
        assert_eq!(
            plan_final_pass(&FinalChoice::Off, &gpu(), &all),
            FinalPlan::Keep(KeepReason::Off)
        );
        assert_eq!(
            plan_final_pass(&FinalChoice::Model(TURBO.into()), &gpu(), &all),
            FinalPlan::Keep(KeepReason::NotInstalled)
        );
    }

    #[test]
    fn memory_gates_turn_the_final_pass_off_instead_of_crashing() {
        let all = installed(&[LARGE]);
        let low_ram = Hardware {
            free_ram_mb: Some(4_000),
            ..gpu()
        };
        assert_eq!(
            plan_final_pass(&FinalChoice::Auto, &low_ram, &all),
            FinalPlan::Keep(KeepReason::LowRam)
        );
        let low_vram = Hardware {
            free_vram_mb: Some(2_000),
            ..gpu()
        };
        assert_eq!(
            plan_final_pass(&FinalChoice::Auto, &low_vram, &all),
            FinalPlan::Keep(KeepReason::LowVram)
        );
        // Nicht messbar blockiert nicht.
        let unknown = Hardware {
            gpu: true,
            free_vram_mb: None,
            free_ram_mb: None,
        };
        assert!(matches!(
            plan_final_pass(&FinalChoice::Auto, &unknown, &all),
            FinalPlan::Run { .. }
        ));
        assert!(!KeepReason::LowRam.is_by_design());
        assert!(KeepReason::CpuOnly.is_by_design());
    }

    // ---- remap_sources -----------------------------------------------------

    #[test]
    fn remap_follows_the_largest_overlap_in_the_same_channel() {
        let old = vec![seg(0, 0, 0, 4_000), seg(1, 1, 3_000, 6_000), seg(2, 0, 7_000, 9_000)];
        let new = vec![
            seg(0, 0, 0, 1_000),
            seg(1, 0, 1_000, 5_000), // 3 s Ueberlappung mit alt 0
            seg(2, 1, 2_500, 6_500), // Kanal 1
            seg(3, 0, 5_500, 9_500),
        ];
        assert_eq!(remap_sources(&old, &new, &[0]), vec![1]);
        assert_eq!(remap_sources(&old, &new, &[1]), vec![2], "nie ueber Kanaele");
        assert_eq!(remap_sources(&old, &new, &[2, 0, 2]), vec![3, 1], "Reihenfolge, keine Doppelten");
        assert!(remap_sources(&old, &new, &[42]).is_empty(), "unbekannter Beleg");
    }

    #[test]
    fn remap_without_overlap_takes_the_nearest_close_segment_or_drops_it() {
        let old = vec![seg(5, 0, 10_000, 11_000), seg(6, 0, 40_000, 41_000)];
        let new = vec![seg(0, 0, 11_500, 13_000), seg(1, 0, 7_000, 9_500), seg(2, 0, 20_000, 21_000)];
        assert_eq!(remap_sources(&old, &new, &[5]), vec![0], "500 ms vor 500 ms: kleinerer Index");
        assert!(remap_sources(&old, &new, &[6]).is_empty(), "nichts in 2 s Naehe");
    }

    // ---- Ablaeufe mit Attrappe ---------------------------------------------

    /// Attrappe der Sprechertrennung: feste Turns je Kanal, Ablaufprotokoll
    /// gemeinsam mit der Umgebung (Reihenfolge Diarisierer -> End-STT).
    struct FakeDiar {
        turns: BTreeMap<u8, Vec<Turn>>,
        log: Rc<RefCell<Vec<String>>>,
        unavailable: Option<&'static str>,
        panic: bool,
        runs: Rc<RefCell<usize>>,
    }

    impl ChannelDiarizer for FakeDiar {
        fn model_name(&self) -> String {
            "fake-diar".into()
        }
        fn unavailable(&self) -> Option<&'static str> {
            self.unavailable
        }
        fn check_ram(&mut self, _: u64) -> Result<(), String> {
            Ok(())
        }
        fn diarize(&mut self, channel: u8, pcm: &[f32]) -> Result<Vec<Turn>, DiarizeError> {
            assert!(!pcm.is_empty());
            self.log.borrow_mut().push(format!("diarize:{channel}"));
            *self.runs.borrow_mut() += 1;
            if self.panic {
                panic!("diarizer exploded");
            }
            Ok(self.turns.get(&channel).cloned().unwrap_or_default())
        }
        fn release(&mut self) {
            self.log.borrow_mut().push("release".into());
        }
    }

    struct FakeEnv {
        loads: Vec<String>,
        events: Vec<MeetingEvent>,
        cancel_after: Option<usize>,
        fail_load: Option<KeepReason>,
        fail_transcribe: bool,
        panic_transcribe: bool,
        empty: bool,
        calls: usize,
        on_call: Option<Box<dyn FnMut(usize)>>,
        /// Jeder Block liefert 4 Woerter zu je 500 ms (statt einem).
        many_words: bool,
        log: Rc<RefCell<Vec<String>>>,
        diar: Option<FakeDiar>,
    }

    impl FakeEnv {
        fn new() -> Self {
            Self {
                loads: Vec::new(),
                events: Vec::new(),
                cancel_after: None,
                fail_load: None,
                fail_transcribe: false,
                panic_transcribe: false,
                empty: false,
                calls: 0,
                on_call: None,
                many_words: false,
                log: Rc::new(RefCell::new(Vec::new())),
                diar: None,
            }
        }

        /// Sprechertrennung mit festen Turns (Kanal -> Turns); Zaehler der Laeufe.
        fn with_turns(mut self, turns: &[(u8, Vec<Turn>)]) -> (Self, Rc<RefCell<usize>>) {
            let runs = Rc::new(RefCell::new(0));
            self.diar = Some(FakeDiar {
                turns: turns.iter().cloned().collect(),
                log: Rc::clone(&self.log),
                unavailable: None,
                panic: false,
                runs: Rc::clone(&runs),
            });
            (self, runs)
        }

        fn kinds(&self) -> Vec<&'static str> {
            self.events
                .iter()
                .map(|e| match e {
                    MeetingEvent::State { .. } => "state",
                    MeetingEvent::Segments { .. } => "segments",
                    MeetingEvent::Levels { .. } => "levels",
                    MeetingEvent::Error { .. } => "error",
                    MeetingEvent::Reset { .. } => "reset",
                    MeetingEvent::TranscriptFinal { .. } => "final",
                    MeetingEvent::Health { .. } => "health",
                })
                .collect()
        }

        fn final_event(&self) -> (u32, Option<String>) {
            match self.events.last() {
                Some(MeetingEvent::TranscriptFinal { epoch, model, .. }) => (*epoch, model.clone()),
                other => panic!("TranscriptFinal zuletzt erwartet, war {other:?}"),
            }
        }
    }

    impl FinalEnv for FakeEnv {
        fn load(&mut self, model_id: &str) -> Result<u64, KeepReason> {
            if let Some(r) = self.fail_load {
                return Err(r);
            }
            self.loads.push(model_id.to_string());
            self.log.borrow_mut().push(format!("load:{model_id}"));
            Ok(7)
        }
        fn diarizer(&mut self) -> Option<&mut dyn ChannelDiarizer> {
            self.diar.as_mut().map(|d| d as &mut dyn ChannelDiarizer)
        }
        fn transcribe(&mut self, model_id: &str, chunk: &Chunk) -> Result<Vec<TimedSegment>, String> {
            self.calls += 1;
            if let Some(f) = self.on_call.as_mut() {
                f(self.calls);
            }
            if self.panic_transcribe {
                panic!("engine exploded");
            }
            if self.fail_transcribe {
                return Err("boom".into());
            }
            if self.empty {
                return Ok(Vec::new());
            }
            let ms = chunk.samples.len() as u64 / 16;
            if self.many_words {
                let words: Vec<WordTime> = (0..4u64)
                    .map(|i| WordTime {
                        text: format!("wort{i}"),
                        start_ms: i * 500,
                        end_ms: (i + 1) * 500,
                    })
                    .collect();
                return Ok(vec![TimedSegment {
                    text: "wort0 wort1 wort2 wort3".into(),
                    start_ms: 0,
                    end_ms: ms,
                    words: Some(words),
                }]);
            }
            Ok(vec![TimedSegment {
                text: format!("{} bei {}", model_id.rsplit('/').next().unwrap(), chunk.offset_ms),
                start_ms: 0,
                end_ms: ms,
                words: Some(vec![crate::managers::transcription::WordTime {
                    text: "wort".into(),
                    start_ms: 100,
                    end_ms: 400,
                }]),
            }])
        }
        fn vad(&self) -> Option<VadFactory> {
            Some(Arc::new(|| Ok(EnergyVad::boxed())))
        }
        fn emit(&mut self, event: MeetingEvent) {
            self.events.push(event);
        }
        fn cancelled(&self) -> bool {
            self.cancel_after.is_some_and(|n| self.calls >= n)
        }
    }

    /// Besprechung mit mic.wav (+ system.wav) und einem Live-Transkript.
    struct Fixture {
        _dir: tempfile::TempDir,
        store: MeetingStore,
        id: String,
        meeting_dir: PathBuf,
    }

    fn write_wav(path: &Path, samples: &[i16]) {
        let mut w = StreamingWavWriter::create(path, 16_000).unwrap();
        w.append(samples).unwrap();
        w.finalize().unwrap();
    }

    /// 2 x (2 s Sprache, 1 s Pause) je Kanal = 6 s.
    fn speech() -> Vec<i16> {
        [loud(2_000), quiet(1_000), loud(2_000), quiet(1_000)].concat()
    }

    fn fixture(with_system: bool, status: MeetingStatus) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        let m = store.create_meeting("t", MeetingSource::Live, Some(0)).unwrap();
        let meeting_dir = dir.path().join(&m.id);
        std::fs::create_dir_all(&meeting_dir).unwrap();
        let mic = meeting_dir.join("mic.wav");
        write_wav(&mic, &speech());
        let system = with_system.then(|| {
            let p = meeting_dir.join("system.wav");
            write_wav(&p, &speech());
            p
        });
        store
            .set_audio_paths(
                &m.id,
                mic.to_str(),
                system.as_ref().and_then(|p| p.to_str()),
                Some(6_000),
            )
            .unwrap();
        store
            .append_delta(
                &m.id,
                &TranscriptDelta {
                    new_segments: vec![seg(0, 0, 0, 2_500), seg(1, 1, 0, 2_400)],
                },
            )
            .unwrap();
        store.set_status(&m.id, status).unwrap();
        Fixture {
            _dir: dir,
            store,
            id: m.id,
            meeting_dir,
        }
    }

    fn run_plan(model: &str) -> FinalPlan {
        FinalPlan::Run {
            model_id: model.into(),
            max_segment_ms: 25_000,
        }
    }

    fn job(f: &Fixture, plan: FinalPlan) -> JobSpec {
        JobSpec {
            meeting_id: f.id.clone(),
            catch_up_model: None,
            plan,
            live_model: Some("live-model".into()),
        }
    }

    #[test]
    fn the_final_pass_replaces_the_transcript_atomically_and_keeps_a_live_copy() {
        let f = fixture(true, MeetingStatus::Processing);
        let mut env = FakeEnv::new();
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);

        // Epoche + 1, neue Segmente je Kanal, Indizes ab 0 in Zeitordnung.
        assert_eq!(out.epoch, 1);
        assert_eq!(out.model.as_deref(), Some(LARGE));
        let snap = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(snap.epoch, 1);
        assert_eq!(snap.model.as_deref(), Some(LARGE));
        assert_eq!(snap.segments.len(), 4, "2 Aeusserungen x 2 Kanaele");
        let idx: Vec<u32> = snap.segments.iter().map(|s| s.segment_index).collect();
        assert_eq!(idx, vec![0, 1, 2, 3]);
        assert!(snap.segments.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
        assert_eq!(snap.segments.iter().filter(|s| s.channel == 1).count(), 2);
        assert!(snap.segments.iter().all(|s| s.text.contains("large-v3")));
        // Wortzeiten auf der Kanal-Achse.
        let w = &snap.segments[2].words.as_ref().unwrap()[0];
        assert_eq!(w.start_ms, snap.segments[2].start_ms + 100);

        // Live-Kopie mit der alten Epoche und den alten Segmenten.
        let copy: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(f.meeting_dir.join(LIVE_TRANSCRIPT_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(copy["epoch"], 0);
        assert_eq!(copy["segments"].as_array().unwrap().len(), 2);
        assert!(!f.meeting_dir.join("transcript_live.json.tmp").exists());

        // Reihenfolge: Reset, Segments, State ready, TranscriptFinal zuletzt.
        assert_eq!(env.kinds(), vec!["reset", "segments", "state", "final"]);
        assert_eq!(env.final_event(), (1, Some(LARGE.to_string())));
        assert_eq!(f.store.get_meeting(&f.id).unwrap().unwrap().status, "ready");
        let meta = f.store.metadata_json(&f.id).unwrap().unwrap();
        assert_eq!(meta[REPORT_KEY]["segments"], 4);
        assert_eq!(meta[REPORT_KEY]["with_words"], true);
        assert_eq!(env.loads, vec![LARGE.to_string()]);
    }

    #[test]
    fn keep_by_design_sends_transcript_final_at_once_with_the_live_model_and_no_hint() {
        let f = fixture(false, MeetingStatus::Processing);
        let mut env = FakeEnv::new();
        let out = run_job(&f.store, &job(&f, FinalPlan::Keep(KeepReason::CpuOnly)), &mut env);
        assert_eq!(out.epoch, 0);
        assert_eq!(env.kinds(), vec!["state", "final"], "kein Hinweis, kein Laden");
        assert_eq!(env.final_event(), (0, Some("live-model".to_string())));
        assert!(env.loads.is_empty());
        assert_eq!(f.store.get_segments(&f.id).unwrap().len(), 2, "Live bleibt");
        assert!(!f.meeting_dir.join(LIVE_TRANSCRIPT_FILE).exists());
        assert_eq!(f.store.get_meeting(&f.id).unwrap().unwrap().status, "ready");
    }

    /// Jeder Fehlerfall: Live-Transkript unveraendert, Epoche 0, ready,
    /// Hinweis, TranscriptFinal mit `model: None`.
    fn assert_kept(f: &Fixture, env: &FakeEnv, out: &JobOutcome, reason: KeepReason) {
        assert_eq!(out.report.kept, Some(reason));
        let snap = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(snap.epoch, 0);
        let texts: Vec<&str> = snap.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["s0", "s1"], "Live-Transkript unveraendert");
        assert_eq!(f.store.get_meeting(&f.id).unwrap().unwrap().status, "ready");
        assert!(env.kinds().contains(&"error"), "Hinweis final_pass_skipped");
        assert!(!env.kinds().contains(&"reset"));
        assert_eq!(env.final_event(), (0, None));
    }

    #[test]
    fn a_failing_model_load_keeps_the_live_transcript() {
        let f = fixture(true, MeetingStatus::Processing);
        let mut env = FakeEnv::new();
        env.fail_load = Some(KeepReason::LowRam);
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_kept(&f, &env, &out, KeepReason::LowRam);
    }

    #[test]
    fn a_failing_block_aborts_without_touching_the_transcript() {
        let f = fixture(true, MeetingStatus::Processing);
        let mut env = FakeEnv::new();
        env.fail_transcribe = true;
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_kept(&f, &env, &out, KeepReason::TranscribeFailed);
        assert!(!f.meeting_dir.join(LIVE_TRANSCRIPT_FILE).exists());
    }

    #[test]
    fn a_panicking_engine_still_ends_in_ready_and_transcript_final() {
        let f = fixture(true, MeetingStatus::Processing);
        let mut env = FakeEnv::new();
        env.panic_transcribe = true;
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_kept(&f, &env, &out, KeepReason::Panic);
    }

    #[test]
    fn a_new_recording_cancels_between_segments() {
        let f = fixture(true, MeetingStatus::Processing);
        let mut env = FakeEnv::new();
        env.cancel_after = Some(1);
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_kept(&f, &env, &out, KeepReason::Cancelled);
        assert_eq!(env.calls, 1, "nach dem laufenden Segment Schluss");
    }

    #[test]
    fn an_empty_final_result_never_wipes_a_live_transcript() {
        let f = fixture(true, MeetingStatus::Processing);
        let mut env = FakeEnv::new();
        env.empty = true;
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_kept(&f, &env, &out, KeepReason::EmptyResult);
    }

    #[test]
    fn a_hand_correction_during_the_run_wins_over_the_final_pass() {
        let f = fixture(true, MeetingStatus::Processing);
        let db = f.store.db_path().to_path_buf();
        let id = f.id.clone();
        let mut env = FakeEnv::new();
        env.on_call = Some(Box::new(move |n| {
            if n == 1 {
                let other = MeetingStore::open_at(&db).unwrap();
                other.update_segment_text(&id, 0, "von Hand").unwrap();
            }
        }));
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_eq!(out.report.kept, Some(KeepReason::Conflict));
        let snap = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(snap.epoch, 0);
        assert_eq!(snap.segments[0].text, "von Hand", "Korrektur bleibt");
        assert_eq!(env.final_event(), (0, None));
    }

    #[test]
    fn an_unwritable_live_copy_means_no_replacement() {
        let f = fixture(true, MeetingStatus::Processing);
        // Platte voll/gesperrt nachgestellt: an der Stelle der Kopie liegt ein
        // Ordner, das Umbenennen scheitert.
        std::fs::create_dir_all(f.meeting_dir.join(LIVE_TRANSCRIPT_FILE).join("x")).unwrap();
        let mut env = FakeEnv::new();
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_kept(&f, &env, &out, KeepReason::BackupFailed);
        assert!(!f.meeting_dir.join("transcript_live.json.tmp").exists(), "kein Rest");
    }

    #[test]
    fn missing_audio_keeps_the_live_transcript() {
        let f = fixture(true, MeetingStatus::Processing);
        std::fs::remove_file(f.meeting_dir.join("mic.wav")).unwrap();
        std::fs::remove_file(f.meeting_dir.join("system.wav")).unwrap();
        let mut env = FakeEnv::new();
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_kept(&f, &env, &out, KeepReason::NoAudio);
    }

    #[test]
    fn a_second_run_after_a_crash_never_overwrites_the_live_copy() {
        let f = fixture(true, MeetingStatus::Processing);
        let mut env = FakeEnv::new();
        run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        let before = std::fs::read_to_string(f.meeting_dir.join(LIVE_TRANSCRIPT_FILE)).unwrap();
        // Absturz nach dem Ersatz, vor `ready`: der Lauf beginnt erneut.
        f.store.set_status(&f.id, MeetingStatus::Processing).unwrap();
        let mut env = FakeEnv::new();
        let out = run_job(&f.store, &job(&f, run_plan(QWEN)), &mut env);
        assert_eq!(out.report.kept, Some(KeepReason::AlreadyFinal));
        assert_eq!(out.epoch, 1, "bleibt beim Endtranskript");
        assert_eq!(
            std::fs::read_to_string(f.meeting_dir.join(LIVE_TRANSCRIPT_FILE)).unwrap(),
            before,
            "Live-Kopie unveraendert"
        );
        assert!(!env.kinds().contains(&"error"), "gewollt, kein Hinweis");
    }

    #[test]
    fn the_echo_free_track_is_preferred_unless_it_is_incomplete() {
        let f = fixture(true, MeetingStatus::Processing);
        let aec = f.meeting_dir.join(super::super::MIC_AEC_FILE);
        write_wav(&aec, &speech());
        let meeting = f.store.get_meeting(&f.id).unwrap().unwrap();
        let tracks = final_tracks(&meeting, None);
        assert_eq!(tracks[0], Track { path: aec.clone(), channel: 0 });
        assert_eq!(tracks[1].channel, 1);
        let incomplete = serde_json::json!({ "aec": { "wav_kept": false } });
        let tracks = final_tracks(&meeting, Some(&incomplete));
        assert_eq!(tracks[0].path, f.meeting_dir.join("mic.wav"));
    }

    #[test]
    fn catch_up_transcribes_the_lost_tail_then_runs_the_final_pass() {
        // Absturz: Live-Transkript endet bei 2,5 s (Kanal 0) bzw. 2,4 s (Kanal 1).
        let f = fixture(true, MeetingStatus::Processing);
        let mut env = FakeEnv::new();
        let mut spec = job(&f, FinalPlan::Keep(KeepReason::CpuOnly));
        spec.catch_up_model = Some("meeting-model".into());
        let out = run_job(&f.store, &spec, &mut env);
        assert_eq!(out.report.catch_up_segments, 2, "je Kanal die zweite Aeusserung");
        let segs = f.store.get_segments(&f.id).unwrap();
        assert_eq!(segs.len(), 4);
        assert!(segs[2].start_ms >= 2_500 && segs[2].text.contains("meeting-model"));
        assert_eq!(segs.iter().map(|s| s.segment_index).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert_eq!(env.loads, vec!["meeting-model".to_string()]);
        assert_eq!(f.store.get_meeting(&f.id).unwrap().unwrap().status, "ready");
        assert_eq!(env.final_event(), (0, Some("live-model".to_string())));
    }

    /// AK Recovery: eine Besprechung, die beim Absturz auf `processing` stand,
    /// mit ungepatchtem WAV-Header (Dauer 0) -> Header repariert,
    /// `duration_ms > 0`, Rest nachgeholt, `ready`, `TranscriptFinal`.
    #[test]
    fn a_processing_orphan_is_repaired_caught_up_and_marked_ready() {
        let f = fixture(false, MeetingStatus::Processing);
        let mic = f.meeting_dir.join("mic.wav");
        // Absturz: WAV geschrieben, aber nie finalisiert (Header sagt 0).
        std::fs::remove_file(&mic).unwrap();
        {
            let mut w = StreamingWavWriter::create(&mic, 16_000).unwrap();
            w.append(&speech()).unwrap();
        }
        f.store
            .set_audio_paths(&f.id, mic.to_str(), None, None)
            .unwrap();
        assert_eq!(wav_duration_ms(&mic), Some(0), "Header vor der Reparatur");

        let meeting = f.store.get_meeting(&f.id).unwrap().unwrap();
        assert_eq!(meeting.status, "processing");
        assert_eq!(prepare_orphan(&f.store, &meeting).unwrap(), Some(6_000));
        let meeting = f.store.get_meeting(&f.id).unwrap().unwrap();
        assert_eq!(meeting.duration_ms, Some(6_000));
        assert_eq!(meeting.status, "processing");

        let mut env = FakeEnv::new();
        let mut spec = job(&f, FinalPlan::Keep(KeepReason::CpuOnly));
        spec.catch_up_model = Some("meeting-model".into());
        spec.live_model = None;
        run_job(&f.store, &spec, &mut env);
        let meeting = f.store.get_meeting(&f.id).unwrap().unwrap();
        assert_eq!(meeting.status, "ready");
        assert!(meeting.duration_ms.unwrap() > 0);
        let segs = f.store.get_segments(&f.id).unwrap();
        let last_mic = segs.iter().filter(|s| s.channel == 0).map(|s| s.end_ms).max().unwrap();
        assert!(last_mic >= 5_000, "Segmente bis zum Ende: {last_mic}");
        assert_eq!(env.kinds().last(), Some(&"final"));
    }

    // ---- M3-P3b: Sprecher im Enddurchlauf ------------------------------------

    fn turn(start_ms: u64, end_ms: u64, speaker: u32) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    /// Kanal 1: Sprecher 1 in der ersten Sekunde, danach Sprecher 2.
    fn two_speakers() -> Vec<(u8, Vec<Turn>)> {
        vec![(1, vec![turn(0, 1_000, 1), turn(1_000, 9_000, 2)])]
    }

    fn channel_speakers(snap: &[StoredSegment], channel: u8) -> Vec<Option<u32>> {
        snap.iter()
            .filter(|s| s.channel == channel)
            .map(|s| s.speaker_index)
            .collect()
    }

    #[test]
    fn the_final_pass_assigns_speakers_before_the_stt_model_and_writes_one_epoch() {
        let f = fixture(true, MeetingStatus::Processing);
        let (mut env, runs) = FakeEnv::new().with_turns(&two_speakers());
        env.many_words = true;
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);

        // Reihenfolge: erst der Diarisierer (und frei), dann das End-STT-Modell.
        let log = env.log.borrow().clone();
        assert_eq!(
            log,
            vec![
                "diarize:1",
                "release",
                "load:handy-computer/whisper-large-v3-gguf/whisper-large-v3-Q5_K_M.gguf"
            ]
        );
        assert_eq!(
            *runs.borrow(),
            1,
            "nur die Gegenseite, das Mikrofon ist 'Ich'"
        );

        // EINE Epoche, Kanal 1 in zwei Sprecher geteilt, Kanal 0 ohne Sprecher.
        assert_eq!(out.epoch, 1);
        let snap = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(snap.epoch, 1);
        let speakers = channel_speakers(&snap.segments, 1);
        assert!(speakers.iter().all(Option::is_some), "{speakers:?}");
        assert!(
            speakers.contains(&Some(1)) && speakers.contains(&Some(2)),
            "{speakers:?}"
        );
        assert!(
            speakers.len() >= 3,
            "der erste Block wurde geteilt: {speakers:?}"
        );
        assert!(channel_speakers(&snap.segments, 0)
            .iter()
            .all(Option::is_none));
        assert_eq!(
            snap.segments
                .iter()
                .map(|s| s.segment_index)
                .collect::<Vec<_>>(),
            (0..snap.segments.len() as u32).collect::<Vec<_>>(),
            "Indizes lueckenlos ab 0"
        );
        // Turns, Modell und Zeilen stehen in derselben Transaktion.
        let hints: serde_json::Value =
            serde_json::from_str(&f.store.speaker_hints(&f.id).unwrap().unwrap()).unwrap();
        assert_eq!(hints["model"], "fake-diar");
        assert_eq!(hints["channels"]["1"][0], serde_json::json!([0, 1_000, 1]));
        let rows = f.store.speaker_rows(&f.id).unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| (r.channel, r.speaker_index))
                .collect::<Vec<_>>(),
            vec![(1, 1), (1, 2)]
        );
        // Anzeige neu laden, dann fertig; der Bericht nennt die Zahlen.
        assert_eq!(env.kinds(), vec!["reset", "segments", "state", "final"]);
        assert_eq!(env.final_event(), (1, Some(LARGE.to_string())));
        assert_eq!(out.report.speakers.as_ref().unwrap().state, "done");
        assert!(out.report.speakers.as_ref().unwrap().applied);
        let meta = f.store.metadata_json(&f.id).unwrap().unwrap();
        assert_eq!(
            meta["final_pass"]["speakers"]["split_added"],
            out.report.speakers.as_ref().unwrap().split_added
        );
        assert_eq!(meta["diarize"]["state"], "done");
    }

    #[test]
    fn a_kept_live_transcript_still_gets_its_speakers_without_a_second_epoch() {
        let f = fixture(true, MeetingStatus::Processing);
        let (mut env, runs) = FakeEnv::new().with_turns(&two_speakers());
        let out = run_job(
            &f.store,
            &job(&f, FinalPlan::Keep(KeepReason::CpuOnly)),
            &mut env,
        );
        assert_eq!(*runs.borrow(), 1);
        // Live = Ende: keine neue Epoche (die Segmente wurden nicht geteilt), aber Sprecher.
        assert_eq!(out.epoch, 0);
        let snap = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(snap.epoch, 0);
        assert_eq!(
            channel_speakers(&snap.segments, 1),
            vec![Some(2)],
            "0..2400 ms: Sprecher 1 hat 1000, Sprecher 2 1400 ms"
        );
        assert_eq!(snap.segments.len(), 2);
        assert_eq!(snap.segments[0].text, "s0", "Text unveraendert");
        // Die Anzeige laedt neu, TranscriptFinal mit dem Live-Modell zuletzt.
        assert_eq!(env.kinds(), vec!["reset", "segments", "state", "final"]);
        assert_eq!(env.final_event(), (0, Some("live-model".to_string())));
        assert_eq!(f.store.get_meeting(&f.id).unwrap().unwrap().status, "ready");
        assert!(env.loads.is_empty(), "kein STT-Modell");
    }

    #[test]
    fn a_failed_final_pass_still_applies_the_speakers_to_the_live_transcript() {
        let f = fixture(true, MeetingStatus::Processing);
        let (mut env, _) = FakeEnv::new().with_turns(&two_speakers());
        env.fail_transcribe = true;
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_eq!(out.report.kept, Some(KeepReason::TranscribeFailed));
        let snap = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(snap.epoch, 0);
        let texts: Vec<&str> = snap.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["s0", "s1"],
            "Live-Transkript unveraendert im Text"
        );
        assert_eq!(channel_speakers(&snap.segments, 1), vec![Some(2)]);
        assert_eq!(env.final_event(), (0, None));
    }

    #[test]
    fn a_missing_model_or_a_panicking_diarizer_never_blocks_the_transcript() {
        // Modell fehlt: Enddurchlauf laeuft wie ohne Sprecher, Bericht nennt den Grund.
        let f = fixture(true, MeetingStatus::Processing);
        let (mut env, runs) = FakeEnv::new().with_turns(&two_speakers());
        env.diar.as_mut().unwrap().unavailable = Some("model_missing");
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_eq!(out.epoch, 1);
        assert_eq!(*runs.borrow(), 0);
        assert_eq!(f.store.speaker_hints(&f.id).unwrap(), None);
        let step = out.report.speakers.as_ref().unwrap();
        assert_eq!(step.state, "skipped");
        assert_eq!(step.channels[0].skipped, Some("model_missing"));
        assert!(!step.applied);
        assert_eq!(env.kinds(), vec!["reset", "segments", "state", "final"]);

        // Panik im Diarisierer: das End-STT laeuft trotzdem, ohne Sprecher.
        let f = fixture(true, MeetingStatus::Processing);
        let (mut env, _) = FakeEnv::new().with_turns(&two_speakers());
        env.diar.as_mut().unwrap().panic = true;
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_eq!(out.epoch, 1, "Enddurchlauf trotz Panik im Sprecher-Schritt");
        assert_eq!(out.report.kept, None);
        assert_eq!(out.report.speakers.as_ref().unwrap().state, "aborted");
        assert_eq!(f.store.speaker_hints(&f.id).unwrap(), None);
        assert_eq!(f.store.get_meeting(&f.id).unwrap().unwrap().status, "ready");
        // Die Absturzmarke steht nicht auf running (Panik-Wache) und der Lauf zaehlt nicht mit.
        let meta = f.store.metadata_json(&f.id).unwrap().unwrap();
        assert_ne!(meta["diarize"]["state"], "running");
    }

    #[test]
    fn a_recovery_run_after_the_replace_reuses_the_stored_turns_and_changes_nothing() {
        let f = fixture(true, MeetingStatus::Processing);
        let (mut env, runs) = FakeEnv::new().with_turns(&two_speakers());
        env.many_words = true;
        run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert_eq!(*runs.borrow(), 1);
        let first = f.store.transcript_snapshot(&f.id).unwrap();
        // Absturz nach dem Ersatz, vor `ready`: der Lauf beginnt erneut.
        f.store
            .set_status(&f.id, MeetingStatus::Processing)
            .unwrap();
        let (mut env, runs2) = FakeEnv::new().with_turns(&two_speakers());
        let out = run_job(&f.store, &job(&f, run_plan(QWEN)), &mut env);
        assert_eq!(out.report.kept, Some(KeepReason::AlreadyFinal));
        assert_eq!(*runs2.borrow(), 0, "gespeicherte Turns, kein Modelllauf");
        let second = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(
            (second.epoch, second.revision),
            (first.epoch, first.revision)
        );
        assert_eq!(second.segments, first.segments);
        assert_eq!(env.kinds(), vec!["state", "final"], "nichts neu zu laden");
    }

    #[test]
    fn the_microphone_is_diarized_only_with_the_flag_or_without_a_system_track() {
        let both = vec![
            (0u8, vec![turn(0, 1_000, 1), turn(1_000, 9_000, 2)]),
            (1u8, vec![turn(0, 9_000, 1)]),
        ];
        // Online mit Systemton, ohne Haekchen: nur die Gegenseite.
        let f = fixture(true, MeetingStatus::Processing);
        let (mut env, runs) = FakeEnv::new().with_turns(&both);
        let out = run_job(
            &f.store,
            &job(&f, FinalPlan::Keep(KeepReason::CpuOnly)),
            &mut env,
        );
        assert_eq!(*runs.borrow(), 1);
        assert_eq!(
            channel_speakers(&f.store.get_segments(&f.id).unwrap(), 0),
            vec![None]
        );
        assert!(
            !out.report.speakers.as_ref().unwrap().mic_without_aec,
            "das Mikrofon wird nicht diarisiert, also kein Hinweis"
        );

        // Mit "Mehrere Personen am Mikrofon": beide Kanaele, "Ich" wird zu "Raum n".
        let f = fixture(true, MeetingStatus::Processing);
        f.store
            .set_metadata_key(&f.id, "diarize_mic", serde_json::json!(true))
            .unwrap();
        let (mut env, runs) = FakeEnv::new().with_turns(&both);
        let out = run_job(
            &f.store,
            &job(&f, FinalPlan::Keep(KeepReason::CpuOnly)),
            &mut env,
        );
        assert_eq!(*runs.borrow(), 2);
        assert!(
            out.report.speakers.as_ref().unwrap().mic_without_aec,
            "ohne mic_aec.wav rat die Anzeige zu Kopfhoerern oder Echo-Unterdrueckung"
        );
        let segs = f.store.get_segments(&f.id).unwrap();
        assert_eq!(
            channel_speakers(&segs, 0),
            vec![Some(2)],
            "0..2500 ms: Sprecher 2 hat 1500, Sprecher 1 1000 ms"
        );
        let dir = super::super::speakers::SpeakerDirectory::load(&f.store, &f.id);
        assert_eq!(dir.label(&segs[0]), "Raum 2");

        // Praesenz (kein Systemton): das Mikrofon ist die einzige Spur, "Person n".
        let f = fixture(false, MeetingStatus::Processing);
        f.store.clear_segments(&f.id).unwrap();
        f.store
            .append_delta(
                &f.id,
                &TranscriptDelta {
                    new_segments: vec![seg(0, 0, 0, 2_500)],
                },
            )
            .unwrap();
        let (mut env, runs) = FakeEnv::new().with_turns(&both);
        run_job(
            &f.store,
            &job(&f, FinalPlan::Keep(KeepReason::CpuOnly)),
            &mut env,
        );
        assert_eq!(*runs.borrow(), 1);
        let segs = f.store.get_segments(&f.id).unwrap();
        let dir = super::super::speakers::SpeakerDirectory::load(&f.store, &f.id);
        assert_eq!(channel_speakers(&segs, 0), vec![Some(2)]);
        assert_eq!(dir.label(&segs[0]), "Person 2");
    }

    fn app_diarizer(enabled: bool, model: &Path, cancelled: bool) -> AppDiarizer {
        AppDiarizer {
            enabled,
            model_path: model.to_path_buf(),
            threads: 1,
            cancel: Arc::new(AtomicBool::new(cancelled)),
            loaded: None,
        }
    }

    #[test]
    fn the_app_diarizer_says_why_it_cannot_run_and_honours_a_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("diar.gguf");
        // Ausgeschaltet hat Vorrang, dann fehlt das Modell, sonst ist es bereit.
        assert_eq!(
            app_diarizer(false, &model, false).unavailable(),
            Some("disabled")
        );
        assert_eq!(
            app_diarizer(true, &model, false).unavailable(),
            Some("model_missing")
        );
        std::fs::write(&model, b"x").unwrap();
        assert_eq!(app_diarizer(true, &model, false).unavailable(), None);
        assert_eq!(
            app_diarizer(true, &model, false).model_name(),
            "sortformer-4spk-v2.1-q8"
        );

        // Neue Aufnahme vor dem Lauf: es wird kein Modell geladen.
        let mut d = app_diarizer(true, &model, true);
        assert!(d.cancelled());
        assert!(matches!(
            d.diarize(1, &[0.0; 16]),
            Err(DiarizeError::Cancelled)
        ));
        assert!(d.loaded.is_none());

        // Fehlt die Datei, ist es ein Fehler und keine Panik.
        let mut d = app_diarizer(true, &dir.path().join("weg.gguf"), false);
        assert!(matches!(
            d.diarize(1, &[0.0; 16]),
            Err(DiarizeError::ModelMissing(_))
        ));
        d.release();
        assert!(d.loaded.is_none());
    }

    #[test]
    fn without_a_diarizer_or_tracks_nothing_changes() {
        // Umgebung ohne Diarisierer (alle bisherigen Tests): kein Sprecher-Bericht.
        let f = fixture(true, MeetingStatus::Processing);
        let mut env = FakeEnv::new();
        let out = run_job(&f.store, &job(&f, run_plan(LARGE)), &mut env);
        assert!(out.report.speakers.is_none());
        assert_eq!(f.store.speaker_hints(&f.id).unwrap(), None);
        // Keep-Plan und leeres Live-Transkript: nichts zuzuordnen, also kein Modelllauf.
        let g = fixture(true, MeetingStatus::Processing);
        g.store.clear_segments(&g.id).unwrap();
        let (mut env, runs) = FakeEnv::new().with_turns(&two_speakers());
        run_job(
            &g.store,
            &job(&g, FinalPlan::Keep(KeepReason::CpuOnly)),
            &mut env,
        );
        assert_eq!(*runs.borrow(), 0);
    }

    #[test]
    fn a_failed_catch_up_leaves_a_gap_marker_instead_of_silence() {
        let f = fixture(false, MeetingStatus::Recording);
        let mut env = FakeEnv::new();
        env.fail_load = Some(KeepReason::LoadFailed);
        let mut spec = job(&f, FinalPlan::Keep(KeepReason::CpuOnly));
        spec.catch_up_model = Some("meeting-model".into());
        let out = run_job(&f.store, &spec, &mut env);
        assert_eq!(out.report.catch_up_gaps, 1);
        let segs = f.store.get_segments(&f.id).unwrap();
        let gap = segs.iter().find(|s| s.channel == 0 && s.start_ms == 2_500).unwrap();
        assert_eq!(gap.end_ms, 6_000);
        assert!(gap.text.starts_with("[Nicht transkribiert"));
        assert_eq!(f.store.get_meeting(&f.id).unwrap().unwrap().status, "ready");
    }
}
