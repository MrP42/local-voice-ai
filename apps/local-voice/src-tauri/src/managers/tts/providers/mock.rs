//! Deterministischer Testanbieter: Sinus oder Stille, 24 kHz mono PCM16.
//!
//! Dient drei Zwecken: Vertragstests ([`super::contract_tests`]), Tests der
//! Registry und späterer Pakete, und als „Anbieter ohne Engine“ für
//! Trockenläufe des Benchmarks. Verhalten ist frei einstellbar: Latenz bis zum
//! ersten Ton, echtes Chunk-Streaming mit `n` Teilen, Pausen zwischen den
//! Teilen, gezielte Fehler (auch mitten im Stream) und Gesundheitszustand.
//!
//! Dauer des erzeugten Signals: 50 ms je Zeichen, mindestens 100 ms, höchstens
//! 20 s — der Speicherbedarf bleibt damit auch bei riesigem Text begrenzt.
//! Das Signal ist eine reine Funktion von Text und Einstellung; alle Chunks
//! zusammen sind byte-gleich mit dem Clip aus `synthesize`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use tokio::sync::mpsc;

use super::contract::{
    guarded, language_matches, send_chunk, AudioChunk, AudioClip, AudioEncoding, AudioFormat,
    CancelToken, Device, LicenseInfo, ProviderCaps, ProviderDescriptor, ProviderHealth,
    ProviderTier, StreamingMode, SynthesisOptions, TtsError, TtsProvider, VoiceDescriptor,
};
use super::wav::wav_from_pcm;

pub const MOCK_SAMPLE_RATE: u32 = 24_000;
const MS_PER_CHAR: u64 = 50;
const MIN_MS: u64 = 100;
const MAX_MS: u64 = 20_000;
const AMPLITUDE: f64 = 0.25;

/// Welches Signal der Mock erzeugt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockTone {
    Sine { hz: u32 },
    Silence,
}

/// Ein eingebauter Fehler: `at_chunk` Teile werden geliefert, danach kommt
/// `error` (0 = vor dem ersten Teil). `for_attempts`: nur in den ersten n
/// Versuchen (`None` = immer) — so lässt sich „Aussetzer, dann klappt es“
/// nachstellen.
#[derive(Debug, Clone)]
pub struct MockFailure {
    pub error: TtsError,
    pub at_chunk: u32,
    pub for_attempts: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MockHealth {
    Ok,
    Unavailable(String),
    /// Die Prüfung kommt nie zurück (Test der Zeitgrenze).
    Hang,
}

/// Zählt laufende Aufträge; zählt beim Drop herunter, auch wenn das Future
/// mitten im Warten fallengelassen wird (Beweis, dass Abbruch aufräumt).
struct InFlight(Arc<AtomicU32>);

impl InFlight {
    fn enter(counter: &Arc<AtomicU32>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter.clone())
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct MockProvider {
    id: String,
    latency: Duration,
    chunks: u32,
    chunk_gap: Duration,
    tone: MockTone,
    failure: Option<MockFailure>,
    health: MockHealth,
    attempts: AtomicU32,
    sent: AtomicU32,
    in_flight: Arc<AtomicU32>,
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Der Plan eines Auftrags: die PCM-Teile und ein eventueller Fehlerpunkt.
struct Plan {
    chunks: Vec<Vec<u8>>,
    failure: Option<MockFailure>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            id: "mock".to_string(),
            latency: Duration::ZERO,
            chunks: 1,
            chunk_gap: Duration::ZERO,
            tone: MockTone::Sine { hz: 440 },
            failure: None,
            health: MockHealth::Ok,
            attempts: AtomicU32::new(0),
            sent: AtomicU32::new(0),
            in_flight: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn with_id(mut self, id: &str) -> Self {
        self.id = id.to_string();
        self
    }

    /// Wartezeit vor dem ersten Ton.
    pub fn latency(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    /// `n` echte Teil-Audios mit `gap` Pause dazwischen (`n >= 2` ergibt
    /// Streaming-Modus `Chunk`).
    pub fn chunked(mut self, n: u32, gap: Duration) -> Self {
        self.chunks = n.max(1);
        self.chunk_gap = gap;
        self
    }

    pub fn silence(mut self) -> Self {
        self.tone = MockTone::Silence;
        self
    }

    pub fn failing(mut self, failure: MockFailure) -> Self {
        self.failure = Some(failure);
        self
    }

    /// Nicht verfügbar, mit Grund (Gesundheit meldet ihn, Synthese endet mit
    /// `Unavailable`).
    pub fn unavailable(mut self, reason: &str) -> Self {
        self.health = MockHealth::Unavailable(reason.to_string());
        self
    }

    /// Die Gesundheitsprüfung antwortet nie.
    pub fn hang_health(mut self) -> Self {
        self.health = MockHealth::Hang;
        self
    }

    /// Begonnene Aufträge (auch fehlgeschlagene und wiederholte).
    pub fn attempts(&self) -> u32 {
        self.attempts.load(Ordering::SeqCst)
    }

    /// Aktuell laufende Aufträge.
    pub fn in_flight(&self) -> u32 {
        self.in_flight.load(Ordering::SeqCst)
    }

    /// An Empfänger gelieferte Chunks (über alle Aufträge).
    pub fn chunks_sent(&self) -> u32 {
        self.sent.load(Ordering::SeqCst)
    }

    /// Das erwartete Signal als ein Stück PCM16 (für Gleichheitsprüfungen).
    pub fn expected_pcm(&self, text: &str) -> Vec<u8> {
        self.pcm_chunks(text).concat()
    }

    fn pcm_chunks(&self, text: &str) -> Vec<Vec<u8>> {
        let chars = text.trim().chars().count() as u64;
        let ms = (chars * MS_PER_CHAR).clamp(MIN_MS, MAX_MS);
        let total = (MOCK_SAMPLE_RATE as u64 * ms / 1000) as usize;
        let n = (self.chunks.max(1) as usize).min(total.max(1));
        let (base, rem) = (total / n, total % n);
        let mut out = Vec::with_capacity(n);
        let mut index = 0usize;
        for part in 0..n {
            let len = base + usize::from(part < rem);
            let mut bytes = Vec::with_capacity(len * 2);
            for i in index..index + len {
                let sample: i16 = match self.tone {
                    MockTone::Silence => 0,
                    MockTone::Sine { hz } => {
                        let phase = 2.0 * std::f64::consts::PI * hz as f64 * i as f64
                            / MOCK_SAMPLE_RATE as f64;
                        (AMPLITUDE * i16::MAX as f64 * phase.sin()) as i16
                    }
                };
                bytes.extend(sample.to_le_bytes());
            }
            index += len;
            out.push(bytes);
        }
        out
    }

    /// Gemeinsamer Auftragsbeginn: Verfügbarkeit, Eingabeprüfung, Versuch
    /// zählen, Signal und Fehlerpunkt festlegen.
    fn begin(&self, text: &str) -> Result<Plan, TtsError> {
        if let MockHealth::Unavailable(reason) = &self.health {
            return Err(TtsError::unavailable(reason.clone()));
        }
        if text.trim().is_empty() {
            return Err(TtsError::bad_request("Leerer Text"));
        }
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
        let failure = self
            .failure
            .clone()
            .filter(|f| f.for_attempts.is_none_or(|k| attempt <= k));
        Ok(Plan {
            chunks: self.pcm_chunks(text),
            failure,
        })
    }
}

impl TtsProvider for MockProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.id.clone(),
            label: "Mock (Test)".to_string(),
            tier: ProviderTier::BenchmarkOnly,
            opt_in: true,
            caps: ProviderCaps {
                languages: vec!["de".to_string(), "en".to_string()],
                streaming: if self.chunks > 1 {
                    StreamingMode::Chunk
                } else {
                    StreamingMode::Sentence
                },
                formats: vec![AudioEncoding::Pcm16Le, AudioEncoding::Wav],
                sample_rates: vec![MOCK_SAMPLE_RATE],
                cloning: false,
                devices: vec![Device::Cpu],
                platforms: ["windows-x64", "macos-x64", "macos-aarch64", "linux-x64"]
                    .map(String::from)
                    .to_vec(),
                style_tags: false,
            },
            license: LicenseInfo {
                code: "MIT".to_string(),
                weights: "keine (synthetisches Signal)".to_string(),
                non_commercial: false,
                verified: true,
                source_url: "https://github.com/MrP42/local-voice-ai".to_string(),
                verified_on: Some("2026-10-26".to_string()),
                note: "Teil dieser App, enthält keine Modelle.".to_string(),
            },
        }
    }

    fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
        Box::pin(async move {
            match &self.health {
                MockHealth::Ok => {
                    let mut h = ProviderHealth::ok();
                    h.device = Some(Device::Cpu);
                    h.version = Some("mock-1".to_string());
                    h
                }
                MockHealth::Unavailable(reason) => ProviderHealth::unavailable(
                    reason.clone(),
                    Some("Nur im Test einstellbar.".to_string()),
                ),
                MockHealth::Hang => std::future::pending().await,
            }
        })
    }

    fn list_voices<'a>(
        &'a self,
        language: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
        Box::pin(async move {
            if let MockHealth::Unavailable(reason) = &self.health {
                return Err(TtsError::unavailable(reason.clone()));
            }
            Ok([
                ("mock-de", "Mock Deutsch", "de"),
                ("mock-en", "Mock English", "en"),
            ]
            .into_iter()
            .filter(|(_, _, lang)| language_matches(lang, language))
            .map(|(id, label, lang)| VoiceDescriptor {
                id: id.to_string(),
                label: label.to_string(),
                language: lang.to_string(),
                gender: None,
                license: None,
            })
            .collect())
        })
    }

    fn synthesize<'a>(
        &'a self,
        text: &'a str,
        opts: &'a SynthesisOptions,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
        Box::pin(async move {
            guarded(cancel, opts.timeout, "Mock", async {
                let _running = InFlight::enter(&self.in_flight);
                let plan = self.begin(text)?;
                tokio::time::sleep(self.latency).await;
                let mut pcm = Vec::new();
                for (i, part) in plan.chunks.iter().enumerate() {
                    if i > 0 && !self.chunk_gap.is_zero() {
                        tokio::time::sleep(self.chunk_gap).await;
                    }
                    if let Some(f) = plan.failure.as_ref().filter(|f| f.at_chunk as usize == i) {
                        return Err(f.error.clone());
                    }
                    pcm.extend_from_slice(part);
                }
                AudioClip::from_pcm(AudioEncoding::Pcm16Le, MOCK_SAMPLE_RATE, 1, &pcm)
            })
            .await
        })
    }

    fn stream_synthesize<'a>(
        &'a self,
        text: &'a str,
        opts: &'a SynthesisOptions,
        cancel: &'a CancelToken,
        sink: mpsc::Sender<AudioChunk>,
    ) -> BoxFuture<'a, Result<(), TtsError>> {
        Box::pin(async move {
            guarded(cancel, opts.timeout, "Mock", async {
                let _running = InFlight::enter(&self.in_flight);
                let started = Instant::now();
                if !matches!(opts.format, AudioEncoding::Pcm16Le | AudioEncoding::Wav) {
                    return Err(TtsError::bad_request(format!(
                        "Format {:?} wird vom Mock nicht geliefert",
                        opts.format
                    )));
                }
                let plan = self.begin(text)?;
                tokio::time::sleep(self.latency).await;
                let n = plan.chunks.len();
                for (i, part) in plan.chunks.iter().enumerate() {
                    if i > 0 && !self.chunk_gap.is_zero() {
                        tokio::time::sleep(self.chunk_gap).await;
                    }
                    if let Some(f) = plan.failure.as_ref().filter(|f| f.at_chunk as usize == i) {
                        return Err(f.error.clone());
                    }
                    let (encoding, data) = match opts.format {
                        // Jedes Wav-Chunk ist eine in sich abspielbare WAV-Datei.
                        AudioEncoding::Wav => (
                            AudioEncoding::Wav,
                            wav_from_pcm(AudioEncoding::Pcm16Le, MOCK_SAMPLE_RATE, 1, part)?,
                        ),
                        _ => (AudioEncoding::Pcm16Le, part.clone()),
                    };
                    let chunk = AudioChunk {
                        request_id: cancel.request_id().to_string(),
                        seq: i as u32,
                        format: AudioFormat {
                            sample_rate: MOCK_SAMPLE_RATE,
                            channels: 1,
                            encoding,
                        },
                        data,
                        is_last: i + 1 == n,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    };
                    send_chunk(&sink, chunk, cancel).await?;
                    self.sent.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            })
            .await
        })
    }

    fn cache_tag(&self, voice: Option<&str>) -> String {
        format!("{}:synthetic:{}", self.id, voice.unwrap_or("default"))
    }
}
