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
//! P2b baut daraus den vollen Harness (Echtzeit-Takt, Latenz, Enddurchlauf).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use serde::Serialize;

use super::chunker::Chunk;
use super::dsp::{
    DspConfig, DspNotice, EchoSetup, LivePipeline, NoticeFn, PcmSink, VadFactory, WavFileSink,
};
use super::recorder::{store_echo_summary, timeline_writer, MeetingEvent};
use super::store::{MeetingSource, MeetingStore};
use crate::audio_toolkit::audio::StreamingWavWriter;
use crate::managers::transcription::TimedSegment;
use crate::selftest::normalize_word;

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
    transcribe: impl FnMut(&Chunk) -> Vec<TimedSegment> + Send + 'static,
) -> Result<serde_json::Value, String> {
    let started = std::time::Instant::now();
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
    let pipeline = LivePipeline::start(
        id.clone(),
        Arc::clone(&store),
        cfg,
        transcribe,
        move |e| {
            if matches!(e, MeetingEvent::Segments { .. }) {
                counter.fetch_add(1, Ordering::Relaxed);
            }
        },
    )?;
    let mut mic_feed = pipeline.feed(0).ok_or("dsp_thread_failed")?;
    let mut sys_feed = pipeline.feed(1);

    // In Zeitstempel-Reihenfolge einspeisen, wie zwei Capture-Threads es tun.
    let mut m = 0usize;
    let mut s = 0usize;
    let sys = system.as_deref().unwrap_or_default();
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
            if !mic_feed.push_blocking(&opts.mic[m..end], Some(stamp)) {
                return Err("dsp_thread_gone".into());
            }
            m = end;
        } else if let Some(feed) = sys_feed.as_mut() {
            let end = (s + BLOCK).min(sys.len());
            let stamp = QPC_BASE + (s + delay) as u64 * QPC_PER_SAMPLE;
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
