//! M2-P2c2: headless Messweg `--simulate-meeting` fuer die Echo-Unterdrueckung
//! (AK6). Schiebt zwei WAV-Dateien (Mikrofon, Systemton) durch DENSELBEN
//! DSP-Thread und Transkriptions-Worker wie eine Live-Besprechung – nur die
//! Quelle ist eine Datei statt cpal/WASAPI. Die QPC-Stempel sind synthetisch:
//! beide Spuren beginnen gleichzeitig, oder der Systemton `system_delay_ms`
//! spaeter (Loopback startet nach dem Mikrofon, negativer Versatz).
//!
//! Ergebnis: ein JSON-Objekt mit Ich-/Gegenseite-Transkript, dem Bericht der
//! Echo-Unterdrueckung, der Laenge von `mic_aec.wav` im Vergleich zu `mic.wav`
//! und – wenn Referenztexte fuer beide Seiten vorliegen –
//! `ich_far_word_leak`: Anteil der NUR in der Gegenseite vorkommenden Woerter,
//! die im Ich-Transkript stehen (Spike-Metrik, Konzept §2).
//!
//! Schreibt nur in den Besprechungsordner, den der Aufrufer vorgibt; der
//! CLI-Pfad verlangt dafür die Sandbox `LVA_MEETINGS_DIR`.
//!
//! M2-P2b2 (AK5): `realtime` speist die Dateien im Wanduhr-Takt ein. Je
//! Segment-Event stehen `vad_end_ms` (Ende der Aeusserung, Audiozeit auf der
//! Mikrofon-Achse) und `emitted_at_ms` (Wanduhr ab Einspeisebeginn) fest;
//! Latenz = emitted − vad_end, dazu p50/p95/max. Mit Referenztexten je Seite
//! (`near_text`/`far_text`, z. B. aus `reference.json` der Benchmark-Szenen)
//! kommt die Live-WER ueber `selftest::SelfTestResult::build` dazu.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::chunker::Chunk;
use super::dsp::{
    DspConfig, DspNotice, EchoSetup, LivePipeline, NoticeFn, PcmSink, VadFactory, WavFileSink,
};
use super::recorder::{store_echo_summary, timeline_writer, MeetingEvent};
use super::store::{MeetingSource, MeetingStore};
use crate::audio_toolkit::audio::StreamingWavWriter;
use crate::managers::transcription::TimedSegment;
use crate::selftest::{normalize_word, SelfTestResult};

/// Blocklaenge wie die Capture-Pfade (30 ms bei 16 kHz).
const BLOCK: usize = 480;
/// Synthetischer QPC-Nullpunkt (1000 s), damit auch Stempel vor dem Start positiv bleiben.
const QPC_BASE: u64 = 10_000_000_000;
const QPC_PER_SAMPLE: u64 = 625;

pub struct SimulateOptions {
    pub title: String,
    pub mic: Vec<i16>,
    pub system: Option<Vec<i16>>,
    /// Echo-Unterdrueckung an (braucht `system`).
    pub aec: bool,
    /// Der Systemton beginnt so viel spaeter als das Mikrofon; die ersten
    /// `system_delay_ms` der Systemspur fehlen (wie bei einem spaet
    /// startenden Loopback).
    pub system_delay_ms: u64,
    /// Referenztexte fuer `ich_far_word_leak` (beide noetig).
    pub far_text: Option<String>,
    pub near_text: Option<String>,
    /// M2-P2b2: im Wanduhr-Takt einspeisen (Latenzmessung AK5).
    pub realtime: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LeakResult {
    pub leak: f64,
    pub far_only_words: usize,
    pub leaked_words: Vec<String>,
}

fn word_set(text: &str) -> BTreeSet<String> {
    text.split_whitespace()
        .map(normalize_word)
        .filter(|w| !w.is_empty())
        .collect()
}

/// Anteil der nur in der Gegenseite vorkommenden Woerter (`far_ref` ohne
/// `near_ref`, normalisiert wie die WER), die im Ich-Transkript auftauchen.
/// `None`, wenn es keine solchen Woerter gibt.
pub fn far_word_leak(ich_transcript: &str, far_ref: &str, near_ref: &str) -> Option<LeakResult> {
    let near = word_set(near_ref);
    let far_only: BTreeSet<String> = word_set(far_ref).difference(&near).cloned().collect();
    if far_only.is_empty() {
        return None;
    }
    let ich = word_set(ich_transcript);
    let leaked: Vec<String> = far_only.intersection(&ich).cloned().collect();
    Some(LeakResult {
        leak: leaked.len() as f64 / far_only.len() as f64,
        far_only_words: far_only.len(),
        leaked_words: leaked,
    })
}

// ---------------------------------------------------------------------------
// M2-P2b2: Latenz, Live-WER, Benchmark-Szenen
// ---------------------------------------------------------------------------

/// Zeitmessung eines `Segments`-Events.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SegmentTiming {
    pub channel: u8,
    /// Beginn des Blocks (Kanal-Achse, ms).
    pub offset_ms: u64,
    /// Ende der Aeusserung (Mikrofon-Achse, ms). Abgeleitet aus dem Blockende
    /// minus Nachlaufpolster des Segmentierers: exakt fuer Segmente, die mit
    /// dem VAD-Nachlauf enden; bei einem Schnitt an der Hoechstlaenge und beim
    /// letzten Segment liegt der Wert bis zu 300 ms zu frueh, die Latenz also
    /// hoechstens so viel zu hoch (konservativ).
    pub vad_end_ms: u64,
    /// Wanduhr ab Einspeisebeginn (ms).
    pub emitted_at_ms: u64,
    /// `emitted_at_ms - vad_end_ms`; nur im Echtzeit-Takt aussagekraeftig.
    pub latency_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LatencySummary {
    pub count: usize,
    pub p50: i64,
    pub p95: i64,
    pub max: i64,
}

/// p50/p95/max nach dem Nearest-Rank-Verfahren (Rang = ceil(p·n)).
pub fn latency_summary(values: &[i64]) -> Option<LatencySummary> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_unstable();
    let rank = |p: f64| v[((p * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1];
    Some(LatencySummary {
        count: v.len(),
        p50: rank(0.50),
        p95: rank(0.95),
        max: v[v.len() - 1],
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChannelWer {
    pub wer: f64,
    pub errors: usize,
    pub ref_words: usize,
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiveWer {
    /// Summe der Fehler beider Seiten / Summe der Referenzwoerter.
    pub wer: f64,
    pub errors: usize,
    pub ref_words: usize,
    pub ich: Option<ChannelWer>,
    pub gegen: Option<ChannelWer>,
}

fn channel_wer(reference: &str, hypothesis: &str) -> ChannelWer {
    // Dieselbe WER-Implementierung wie Selbsttest und `--reference`.
    let r = SelfTestResult::build(reference, hypothesis, Vec::new(), 0, 0.0);
    let errors = r.substitutions + r.deletions + r.insertions;
    ChannelWer {
        wer: if r.reference_words == 0 {
            0.0
        } else {
            errors as f64 / r.reference_words as f64
        },
        errors,
        ref_words: r.reference_words,
        substitutions: r.substitutions,
        deletions: r.deletions,
        insertions: r.insertions,
    }
}

/// Live-WER je Seite und gesamt; `None` ohne jede Referenz.
pub fn live_wer(
    ich_ref: Option<&str>,
    ich_text: &str,
    gegen_ref: Option<&str>,
    gegen_text: &str,
) -> Option<LiveWer> {
    let ich = ich_ref.map(|r| channel_wer(r, ich_text));
    let gegen = gegen_ref.map(|r| channel_wer(r, gegen_text));
    if ich.is_none() && gegen.is_none() {
        return None;
    }
    let errors = ich.as_ref().map_or(0, |c| c.errors) + gegen.as_ref().map_or(0, |c| c.errors);
    let ref_words =
        ich.as_ref().map_or(0, |c| c.ref_words) + gegen.as_ref().map_or(0, |c| c.ref_words);
    Some(LiveWer {
        wer: if ref_words == 0 {
            0.0
        } else {
            errors as f64 / ref_words as f64
        },
        errors,
        ref_words,
        ich,
        gegen,
    })
}

/// Hintereinander gelegte Benchmark-Szenen (`make_corpus.py synth`).
pub struct SceneAudio {
    pub mic: Vec<i16>,
    pub system: Vec<i16>,
    /// Referenztext Ich-Seite (Mikrofon), Szenen mit Leerzeichen verbunden.
    pub near_text: String,
    /// Referenztext Gegenseite (Systemton).
    pub far_text: String,
    pub utterances: usize,
}

/// Laedt je Ordner `<mic_file>`, `system.wav` und `reference.json` und legt
/// die Szenen auf eine Zeitachse: jede Szene wird auf die laengere ihrer
/// beiden Spuren mit digitaler Stille aufgefuellt, damit Mikrofon und
/// Systemton gleichzeitig beginnen.
pub fn load_scenes(dirs: &[PathBuf], mic_file: &str) -> Result<SceneAudio, String> {
    let mut out = SceneAudio {
        mic: Vec::new(),
        system: Vec::new(),
        near_text: String::new(),
        far_text: String::new(),
        utterances: 0,
    };
    let join = |acc: &mut String, t: &str| {
        let t = t.trim();
        if !t.is_empty() {
            if !acc.is_empty() {
                acc.push(' ');
            }
            acc.push_str(t);
        }
    };
    for dir in dirs {
        let mut mic = read_pcm16_mono(&dir.join(mic_file))?;
        let mut system = read_pcm16_mono(&dir.join("system.wav"))?;
        let ref_path = dir.join("reference.json");
        let raw = std::fs::read_to_string(&ref_path)
            .map_err(|e| format!("{}: {e}", ref_path.display()))?;
        let reference: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", ref_path.display()))?;
        let text = |side: &str| {
            reference["reference_text"][side]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{}: reference_text.{side} fehlt", ref_path.display()))
        };
        join(&mut out.near_text, &text("mic")?);
        join(&mut out.far_text, &text("system")?);
        out.utterances += reference["utterances"].as_array().map_or(0, Vec::len);
        let len = mic.len().max(system.len());
        mic.resize(len, 0);
        system.resize(len, 0);
        out.mic.extend_from_slice(&mic);
        out.system.extend_from_slice(&system);
    }
    Ok(out)
}

/// Wartet bis `start + audio_ms` (Wanduhr-Takt einer Capture-Quelle: ein
/// Block ist erst nach seiner Aufnahmedauer da).
fn pace(start: Instant, audio_ms: u64) {
    let due = start + Duration::from_millis(audio_ms);
    let now = Instant::now();
    if due > now {
        std::thread::sleep(due - now);
    }
}

fn write_wav(path: &Path, samples: &[i16]) -> Result<(), String> {
    let mut w = StreamingWavWriter::create(path, 16_000)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    w.append(samples)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    w.finalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

fn wav_len(path: &Path) -> Option<u64> {
    hound::WavReader::open(path).ok().map(|r| r.len() as u64)
}

/// Liest eine 16-kHz-Mono-PCM16-WAV (das Format der Besprechungs-Pipeline).
pub fn read_pcm16_mono(path: &Path) -> Result<Vec<i16>, String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = reader.spec();
    if (spec.sample_rate, spec.channels, spec.bits_per_sample) != (16_000, 1, 16)
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err(format!(
            "{}: erwartet 16 kHz mono PCM16, ist {} Hz / {} Kanal / {} bit",
            path.display(),
            spec.sample_rate,
            spec.channels,
            spec.bits_per_sample
        ));
    }
    reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Fuehrt die Simulation aus. `meetings_dir` ist der Ordner, unter dem der
/// Besprechungsordner entsteht (wie `meetings_data_dir`).
pub fn simulate(
    store: Arc<MeetingStore>,
    meetings_dir: &Path,
    opts: SimulateOptions,
    vad_factory: Option<VadFactory>,
    mut transcribe: impl FnMut(&Chunk) -> Vec<TimedSegment> + Send + 'static,
) -> Result<serde_json::Value, String> {
    let started = Instant::now();
    let meeting = store
        .create_meeting(
            &opts.title,
            MeetingSource::Live,
            Some(chrono::Utc::now().timestamp()),
        )
        .map_err(|e| format!("create_meeting: {e}"))?;
    let id = meeting.id.clone();
    let dir = meetings_dir.join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let delay = (opts.system_delay_ms * 16) as usize;
    let system: Option<Vec<i16>> = opts
        .system
        .as_ref()
        .map(|s| s.get(delay.min(s.len())..).unwrap_or_default().to_vec());
    let mic_path = dir.join("mic.wav");
    write_wav(&mic_path, &opts.mic)?;
    let system_path = match &system {
        Some(s) => {
            let p = dir.join("system.wav");
            write_wav(&p, s)?;
            Some(p)
        }
        None => None,
    };
    store
        .set_audio_paths(
            &id,
            mic_path.to_str(),
            system_path.as_ref().and_then(|p| p.to_str()),
            Some(opts.mic.len() as u64 / 16),
        )
        .map_err(|e| format!("set_audio_paths: {e}"))?;

    let notice: NoticeFn = Arc::new(|n: DspNotice| log::info!("simulate: dsp notice {n:?}"));
    let mut cfg = DspConfig::new(vad_factory, notice);
    let aec = opts.aec && system.is_some();
    let aec_path = dir.join(super::MIC_AEC_FILE);
    if aec {
        let sink = WavFileSink::create(&aec_path).map_err(|e| format!("mic_aec.wav: {e}"))?;
        cfg = cfg.with_echo(EchoSetup {
            sink: Some(Box::new(sink) as Box<dyn PcmSink>),
            on_timeline: Some(timeline_writer(Arc::clone(&store), &id)),
        });
    }

    let segment_events = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&segment_events);

    // M2-P2b2: der Worker ist seriell (transcribe -> Event), also gehoert jedes
    // `Segments`-Event zum zuletzt transkribierten Block.
    let pad_ms = cfg.segmenter.trailing_pad_ms;
    let system_shift_ms = opts.system_delay_ms;
    let clock: Arc<std::sync::OnceLock<Instant>> = Arc::new(std::sync::OnceLock::new());
    let last_block: Arc<Mutex<Option<(u64, u64)>>> = Arc::new(Mutex::new(None));
    let timings: Arc<Mutex<Vec<SegmentTiming>>> = Arc::new(Mutex::new(Vec::new()));
    let block_in = Arc::clone(&last_block);
    let transcribe = move |c: &Chunk| {
        let end = c.offset_ms + c.samples.len() as u64 / 16;
        *block_in.lock().unwrap_or_else(|p| p.into_inner()) = Some((c.offset_ms, end));
        transcribe(c)
    };
    let (block_out, timings_in, clock_in) = (
        Arc::clone(&last_block),
        Arc::clone(&timings),
        Arc::clone(&clock),
    );
    let pipeline =
        LivePipeline::start(id.clone(), Arc::clone(&store), cfg, transcribe, move |e| {
            if let MeetingEvent::Segments { appended, .. } = &e {
                counter.fetch_add(1, Ordering::Relaxed);
                let emitted_at_ms = clock_in.get().map_or(0, |c| c.elapsed().as_millis() as u64);
                let block = *block_out.lock().unwrap_or_else(|p| p.into_inner());
                if let (Some((offset_ms, end_ms)), Some(first)) = (block, appended.first()) {
                    let channel = first.channel;
                    // Kanal 1 laeuft auf der Systemton-Achse, die `system_delay_ms`
                    // nach dem Mikrofon beginnt.
                    let shift = if channel == 1 { system_shift_ms } else { 0 };
                    let vad_end_ms = end_ms.saturating_sub(pad_ms).max(offset_ms) + shift;
                    timings_in
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(SegmentTiming {
                            channel,
                            offset_ms,
                            vad_end_ms,
                            emitted_at_ms,
                            latency_ms: emitted_at_ms as i64 - vad_end_ms as i64,
                        });
                }
            }
        })?;
    let mut mic_feed = pipeline.feed(0).ok_or("dsp_thread_failed")?;
    let mut sys_feed = pipeline.feed(1);

    // In Zeitstempel-Reihenfolge einspeisen, wie zwei Capture-Threads es tun.
    let mut m = 0usize;
    let mut s = 0usize;
    let sys = system.as_deref().unwrap_or_default();
    let clock = *clock.get_or_init(Instant::now);
    loop {
        let mic_t = (m < opts.mic.len()).then_some(m);
        let sys_t = (s < sys.len()).then_some(s + delay);
        let take_mic = match (mic_t, sys_t) {
            (None, None) => break,
            (Some(a), Some(b)) => a <= b,
            (Some(_), None) => true,
            (None, Some(_)) => false,
        };
        if take_mic {
            let end = (m + BLOCK).min(opts.mic.len());
            let stamp = QPC_BASE + m as u64 * QPC_PER_SAMPLE;
            if opts.realtime {
                pace(clock, end as u64 / 16);
            }
            if !mic_feed.push_blocking(&opts.mic[m..end], Some(stamp)) {
                return Err("dsp_thread_gone".into());
            }
            m = end;
        } else if let Some(feed) = sys_feed.as_mut() {
            let end = (s + BLOCK).min(sys.len());
            let stamp = QPC_BASE + (s + delay) as u64 * QPC_PER_SAMPLE;
            if opts.realtime {
                pace(clock, (end + delay) as u64 / 16);
            }
            if !feed.push_blocking(&sys[s..end], Some(stamp)) {
                return Err("dsp_thread_gone".into());
            }
            s = end;
        } else {
            s = sys.len();
        }
    }
    drop(mic_feed);
    drop(sys_feed);
    let stats = pipeline.drain();
    let summary = stats.echo_summary();
    if let Some(summary) = &summary {
        store_echo_summary(&store, &id, summary);
    }

    let segments = store
        .get_segments(&id)
        .map_err(|e| format!("get_segments: {e}"))?;
    let text_of = |channel: u8| {
        segments
            .iter()
            .filter(|s| s.channel == channel)
            .map(|s| s.text.trim())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let ich_text = text_of(0);
    let gegen_text = text_of(1);
    let leak = match (&opts.far_text, &opts.near_text) {
        (Some(far), Some(near)) => far_word_leak(&ich_text, far, near),
        _ => None,
    };
    let mic_aec_samples = if aec { wav_len(&aec_path) } else { None };
    let wer = live_wer(
        opts.near_text.as_deref(),
        &ich_text,
        opts.far_text.as_deref(),
        &gegen_text,
    );
    let timings = std::mem::take(&mut *timings.lock().unwrap_or_else(|p| p.into_inner()));
    let latency = if opts.realtime {
        latency_summary(&timings.iter().map(|t| t.latency_ms).collect::<Vec<_>>())
    } else {
        None
    };
    let metadata = store.metadata_json(&id).ok().flatten();

    Ok(serde_json::json!({
        "meeting_id": id,
        "aec": aec,
        "system_delay_ms": opts.system_delay_ms,
        "mic_samples": opts.mic.len(),
        "system_samples": system.as_ref().map(Vec::len),
        "mic_aec_samples": mic_aec_samples,
        "mic_aec_same_length": mic_aec_samples.map(|n| n == opts.mic.len() as u64),
        "ich_far_word_leak": leak.as_ref().map(|l| l.leak),
        "far_only_words": leak.as_ref().map(|l| l.far_only_words),
        "leaked_words": leak.as_ref().map(|l| l.leaked_words.clone()),
        "ich_text": ich_text,
        "gegen_text": gegen_text,
        "segment_events": segment_events.load(Ordering::Relaxed),
        "segments": segments,
        "echo": summary,
        "echo_dropped": stats.echo_dropped.load(Ordering::Relaxed),
        "dsp": stats.summary(),
        "timeline": metadata.as_ref().and_then(|m| m.get("timeline").cloned()),
        "wall_ms": started.elapsed().as_millis() as u64,
        // M2-P2b2
        "realtime": opts.realtime,
        "audio_ms": opts.mic.len() as u64 / 16,
        "segment_timings": timings,
        "latency_ms": latency,
        "latency_p95_ms": latency.map(|l| l.p95),
        "wer_live": wer.as_ref().map(|w| w.wer),
        "wer_live_detail": wer,
    }))
}

/// M2-P2d: Text je Kanal (0 = Ich, 1 = Gegenseite) in Segmentreihenfolge.
fn channel_text(segments: &[super::store::StoredSegment], channel: u8) -> String {
    segments
        .iter()
        .filter(|s| s.channel == channel)
        .map(|s| s.text.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// M2-P2d: der Enddurchlauf nach der Simulation, ueber denselben Job wie
/// `stop()` (`final_pass::run_job` mit der App-Umgebung). `final_model` ist
/// `auto`, `off` oder eine Modell-ID; der Plan prueft Installation, RAM und
/// VRAM wie in der App. Ergebnis: Live- und Endstand fuer das JSON.
pub fn final_pass_with_app(
    app: &tauri::AppHandle,
    store: Arc<MeetingStore>,
    tm: Arc<crate::managers::transcription::TranscriptionManager>,
    meeting_id: &str,
    final_model: &str,
) -> serde_json::Value {
    use super::final_pass::{self, FinalChoice, JobSpec};
    let live = store.transcript_snapshot(meeting_id).ok();
    let plan = final_pass::plan_for_app(app, &FinalChoice::parse(final_model));
    let _ = store.set_status(meeting_id, super::store::MeetingStatus::Processing);
    let job = JobSpec {
        meeting_id: meeting_id.to_string(),
        catch_up_model: None,
        plan: plan.clone(),
        live_model: tm.get_current_model(),
    };
    let mut env = final_pass::AppEnv::new(app, Arc::clone(&tm), Arc::new(AtomicBool::new(false)));
    let outcome = final_pass::run_job(&store, &job, &mut env);
    let after = store.transcript_snapshot(meeting_id).ok();
    let live_copy = store
        .get_meeting(meeting_id)
        .ok()
        .flatten()
        .and_then(|m| m.mic_audio_path)
        .map(|p| Path::new(&p).with_file_name(final_pass::LIVE_TRANSCRIPT_FILE));
    serde_json::json!({
        "requested": final_model,
        "plan": format!("{plan:?}"),
        "model": outcome.model,
        "live_epoch": live.as_ref().map(|s| s.epoch),
        "revision_epoch": after.as_ref().map(|s| s.epoch),
        "transcript_final_epoch": outcome.epoch,
        "live_segments": live.as_ref().map(|s| s.segments.len()),
        "segments": after.as_ref().map(|s| s.segments.len()),
        "segments_with_words": after.as_ref().map(|s| s.segments.iter().filter(|x| x.words.is_some()).count()),
        "ich_text": after.as_ref().map(|s| channel_text(&s.segments, 0)),
        "gegen_text": after.as_ref().map(|s| channel_text(&s.segments, 1)),
        "transcript_live_json": live_copy.as_ref().is_some_and(|p| p.exists()),
        "report": outcome.report,
        "status": store.get_meeting(meeting_id).ok().flatten().map(|m| m.status),
    })
}

/// App-Anbindung fuer die CLI: echter Silero-VAD aus den Ressourcen, echte
/// Transkription ueber den geladenen `TranscriptionManager`.
pub fn simulate_with_app(
    app: &tauri::AppHandle,
    store: Arc<MeetingStore>,
    tm: Arc<crate::managers::transcription::TranscriptionManager>,
    opts: SimulateOptions,
) -> Result<serde_json::Value, String> {
    let meetings_dir = super::meetings_data_dir(app).map_err(|e| format!("meetings dir: {e}"))?;
    let vad = super::recorder::meeting_vad_factory(app);
    let app = app.clone();
    simulate(store, &meetings_dir, opts, vad, move |chunk: &Chunk| {
        super::import::transcribe_chunk_resilient(&app, &tm, chunk)
    })
}

#[cfg(test)]
mod tests {
    use super::super::segmenter::test_support::EnergyVad;
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn leak_counts_only_words_the_near_talker_never_says() {
        let far = "Die Ausschreibung endet am dritten November.";
        let near = "Ich uebernehme die Abstimmung.";
        // far-only: ausschreibung endet am dritten november (die ist geteilt)
        let r = far_word_leak("ich übernehme die abstimmung", far, near).unwrap();
        assert_eq!((r.far_only_words, r.leak), (5, 0.0));
        let r = far_word_leak("Die Ausschreibung endet, ich übernehme", far, near).unwrap();
        assert_eq!(r.leaked_words, vec!["ausschreibung", "endet"]);
        assert!((r.leak - 0.4).abs() < 1e-9);
        assert!(far_word_leak("x", "gleich", "gleich").is_none());
    }

    fn fixture(name: &str) -> Vec<i16> {
        let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
            .iter()
            .collect();
        read_pcm16_mono(&path).unwrap()
    }

    fn sandbox() -> (Arc<MeetingStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap());
        (store, dir)
    }

    /// Ganze Kette mit Dateiquelle, Energie-VAD und einer Attrappe als STT: die
    /// Attrappe "hoert" die Gegenseite im Ich-Kanal, wenn dort ein Segment im
    /// Fenster der Gegenseite laut genug ist. Geprueft wird die Verdrahtung
    /// (Laengen, Zeitachse, Bericht) – die echte Wortmessung macht die CLI.
    #[test]
    fn the_file_source_runs_aec_writes_an_equal_length_track_and_stores_the_timeline() {
        let (store, dir) = sandbox();
        let mic = fixture("m2_echo_mic.wav");
        let render = fixture("m2_echo_render.wav");
        let out = simulate(
            Arc::clone(&store),
            dir.path(),
            SimulateOptions {
                title: "sim".into(),
                mic: mic.clone(),
                system: Some(render),
                aec: true,
                system_delay_ms: 700,
                far_text: None,
                near_text: None,
                realtime: false,
            },
            Some(Arc::new(|| Ok(EnergyVad::boxed()))),
            |c: &Chunk| {
                vec![TimedSegment {
                    text: format!("wort{}", c.offset_ms),
                    start_ms: 0,
                    end_ms: c.samples.len() as u64 / 16,
                    words: None,
                }]
            },
        )
        .unwrap();
        assert_eq!(out["aec"], true);
        assert_eq!(out["mic_aec_same_length"], true, "{out}");
        assert_eq!(out["mic_aec_samples"], mic.len() as u64);
        assert_eq!(out["timeline"]["basis"], "qpc");
        let offset = out["timeline"]["offset_ms"].as_f64().unwrap();
        assert!((offset + 700.0).abs() < 0.01, "Versatz {offset}");
        assert_eq!(out["echo"]["reference_seen"], true);
        assert!(out["echo"]["frames"].as_u64().unwrap() >= (mic.len() / 160) as u64);
        let meta = store
            .metadata_json(out["meeting_id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(meta["aec"]["wav_kept"], true);
        assert_eq!(meta["aec"]["file"], super::super::MIC_AEC_FILE);
    }

    // M2-P2b2 ---------------------------------------------------------------

    #[test]
    fn latency_summary_uses_the_nearest_rank_percentile() {
        let values: Vec<i64> = (1..=20).map(|i| i * 100).collect();
        let s = latency_summary(&values).unwrap();
        assert_eq!((s.count, s.p50, s.p95, s.max), (20, 1000, 1900, 2000));
        // Reihenfolge egal, ein Wert ist alles.
        let s = latency_summary(&[700, -20, 300]).unwrap();
        assert_eq!((s.p50, s.p95, s.max), (300, 700, 700));
        assert!(latency_summary(&[]).is_none());
    }

    #[test]
    fn live_wer_sums_the_errors_of_both_channels() {
        let w = live_wer(Some("a b c d"), "a x c d", Some("E f."), "e f g").unwrap();
        assert_eq!(
            (
                w.ich.as_ref().unwrap().errors,
                w.ich.as_ref().unwrap().ref_words
            ),
            (1, 4)
        );
        assert_eq!(
            (
                w.gegen.as_ref().unwrap().errors,
                w.gegen.as_ref().unwrap().ref_words
            ),
            (1, 2)
        );
        assert_eq!((w.errors, w.ref_words), (2, 6));
        assert!((w.wer - 2.0 / 6.0).abs() < 1e-9);
        // Nur eine Seite mit Referenz: nur sie zaehlt.
        let w = live_wer(Some("a b"), "a b", None, "egal").unwrap();
        assert!(w.gegen.is_none());
        assert_eq!((w.errors, w.ref_words), (0, 2));
        assert!(live_wer(None, "x", None, "y").is_none());
    }

    fn write_scene(
        dir: &Path,
        mic: &[i16],
        system: &[i16],
        ref_mic: &str,
        ref_sys: &str,
        n: usize,
    ) {
        std::fs::create_dir_all(dir).unwrap();
        write_wav(&dir.join("mic_echo.wav"), mic).unwrap();
        write_wav(&dir.join("system.wav"), system).unwrap();
        let reference = serde_json::json!({
            "utterances": vec![serde_json::json!({}); n],
            "reference_text": { "mic": ref_mic, "system": ref_sys },
        });
        std::fs::write(dir.join("reference.json"), reference.to_string()).unwrap();
    }

    #[test]
    fn scenes_are_concatenated_on_one_timeline_with_joined_references() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        // Szene a: Systemspur kuerzer als das Mikrofon -> wird mit Stille aufgefuellt.
        write_scene(&a, &[1; 1_600], &[2; 800], "Ich eins.", "Du eins.", 2);
        write_scene(&b, &[3; 320], &[4; 480], "Ich zwei.", "Du zwei.", 3);
        let s = load_scenes(&[a, b], "mic_echo.wav").unwrap();
        assert_eq!(s.mic.len(), 1_600 + 480);
        assert_eq!(s.system.len(), s.mic.len());
        assert_eq!((s.mic[1_599], s.mic[1_600], s.mic[1_600 + 320]), (1, 3, 0));
        assert_eq!((s.system[799], s.system[800], s.system[1_600]), (2, 0, 4));
        assert_eq!(s.near_text, "Ich eins. Ich zwei.");
        assert_eq!(s.far_text, "Du eins. Du zwei.");
        assert_eq!(s.utterances, 5);
        assert!(load_scenes(&[tmp.path().join("fehlt")], "mic_echo.wav").is_err());
    }

    fn opts(mic: Vec<i16>, realtime: bool) -> SimulateOptions {
        SimulateOptions {
            title: "sim".into(),
            mic,
            system: None,
            aec: false,
            system_delay_ms: 0,
            far_text: None,
            near_text: Some("wort".into()),
            realtime,
        }
    }

    /// Echtzeit-Takt: die Datei laeuft in Wanduhrzeit durch DSP-Thread und
    /// Worker; je Segment-Event stehen Aeusserungsende und Wanduhr fest. Die
    /// Latenz ist mindestens der VAD-Nachlauf (600 ms) und kein Vielfaches davon.
    #[test]
    fn realtime_paces_the_file_by_the_wall_clock_and_measures_the_latency() {
        use super::super::segmenter::test_support::{loud, quiet};
        let (store, dir) = sandbox();
        let mic: Vec<i16> = [quiet(300), loud(1_000), quiet(1_200)].concat();
        let t0 = std::time::Instant::now();
        let out = simulate(
            store,
            dir.path(),
            opts(mic, true),
            Some(Arc::new(|| Ok(EnergyVad::boxed()))),
            |_c: &Chunk| {
                vec![TimedSegment {
                    text: "Wort".into(),
                    start_ms: 0,
                    end_ms: 100,
                    words: None,
                }]
            },
        )
        .unwrap();
        assert!(
            t0.elapsed().as_millis() >= 2_450,
            "kein Echtzeit-Takt: {:?}",
            t0.elapsed()
        );
        assert_eq!(out["realtime"], true);
        let timings = out["segment_timings"].as_array().unwrap();
        assert_eq!(timings.len(), 1, "{out}");
        let t = &timings[0];
        assert_eq!(t["channel"], 0);
        let vad_end = t["vad_end_ms"].as_u64().unwrap();
        assert!((1_290..=1_350).contains(&vad_end), "vad_end {vad_end}");
        let latency = t["latency_ms"].as_i64().unwrap();
        assert!((550..=1_500).contains(&latency), "Latenz {latency}");
        assert_eq!(out["latency_ms"]["count"], 1);
        assert_eq!(out["latency_p95_ms"].as_i64(), Some(latency));
        assert_eq!(out["wer_live"].as_f64(), Some(0.0));
    }

    #[test]
    fn without_realtime_the_run_is_not_paced_and_reports_no_latency() {
        use super::super::segmenter::test_support::{loud, quiet};
        let (store, dir) = sandbox();
        let mic: Vec<i16> = [quiet(300), loud(1_000), quiet(1_200)].concat();
        let out = simulate(
            store,
            dir.path(),
            opts(mic, false),
            Some(Arc::new(|| Ok(EnergyVad::boxed()))),
            |_c: &Chunk| Vec::new(),
        )
        .unwrap();
        assert_eq!(out["realtime"], false);
        assert!(out["latency_ms"].is_null());
        assert!(out["latency_p95_ms"].is_null());
        // Referenz da, aber nichts erkannt: alles geloescht.
        assert_eq!(out["wer_live"].as_f64(), Some(1.0));
    }

    #[test]
    fn without_aec_there_is_no_echo_free_track() {
        let (store, dir) = sandbox();
        let out = simulate(
            store,
            dir.path(),
            SimulateOptions {
                title: "sim".into(),
                mic: fixture("m2_echo_mic.wav"),
                system: Some(fixture("m2_echo_render.wav")),
                aec: false,
                system_delay_ms: 0,
                far_text: None,
                near_text: None,
                realtime: false,
            },
            Some(Arc::new(|| Ok(EnergyVad::boxed()))),
            |_c: &Chunk| Vec::new(),
        )
        .unwrap();
        assert_eq!(out["aec"], false);
        assert!(out["mic_aec_samples"].is_null());
        assert!(out["echo"].is_null());
        let id = out["meeting_id"].as_str().unwrap();
        assert!(!dir.path().join(id).join(super::super::MIC_AEC_FILE).exists());
    }
}
