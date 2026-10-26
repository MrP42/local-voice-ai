//! Benchmark-Schleife der Anbieterschicht (Goal tts-provider, Paket P3b).
//!
//! Das ist der Rust-Teil der Schnittstelle zum Python-Harness
//! (`scripts/bench/tts/schema/README.md`): je Konfiguration ein Aufruf
//! (`--tts-bench`), der `rows.jsonl` (eine Zeile je Synthese, auch
//! Fehlschläge), `env.json` und die WAVs schreibt. Das Modul ist REIN: kein
//! `AppHandle`, keine Einstellungen — die Registry kommt über [`BenchBackend`]
//! herein, damit die Schleife mit dem Mock-Anbieter testbar ist (die
//! App-Anbindung steht in `tts_cli.rs`).
//!
//! ## Messung
//!
//! Jede Synthese läuft über `stream_synthesize` der Registry — derselbe Weg
//! wie die Probestimme, aber OHNE Wiederholung (ein Fehlschlag wird als Zeile
//! gemeldet, nicht durch einen zweiten Versuch verdeckt). Der Verbraucher
//! stempelt jeden Chunk bei der Ankunft:
//!
//! * `ttfa_ms` = Ankunft des ersten Chunks, `total_ms` = Ankunft des letzten,
//! * `audio_ms` = Frames aller Chunks / Abtastrate (aus dem zusammengesetzten
//!   Audio, nicht aus dem Text), `rtf = total_ms / audio_ms`,
//! * `chunk_gaps_ms` = Abstände der Ankünfte (Länge `chunk_count − 1`).
//!
//! Das Chunk-Format folgt den Fähigkeiten des Anbieters ([`pick_format`]):
//! Fish und Piper melden nur `Wav`, also liefern sie einen vollständigen
//! WAV-Container je Satz.
//!
//! ## Ablauf
//!
//! `cold`: je Satz und Lauf zuerst [`BenchBackend::cold_reset`] (Fish: der
//! SELBST gestartete Server wird beendet und neu gestartet, `process_start_ms`
//! ist die Startzeit; ein fremder Server bleibt unberührt und wird im
//! `cold_reset`-Feld der Zeile vermerkt), dann eine frisch gebaute Registry,
//! dann die Messung. `warm`: eine Aufwärm-Synthese ohne Zeile, dann alle Läufe.
//! Strikt sequentiell — es läuft nie mehr als EINE Synthese.
//!
//! WAV: der erste Lauf (`run_idx` 0) jedes Satzes, bei `warm > 0` der warme,
//! sonst der kalte, nach `wav/<utterance_id>.wav` (`wav_path` ist relativ zum
//! Ausgabeordner). Zusätzliche Zeilenfelder (`cold_reset`) sind laut Schema
//! erlaubt.
//!
//! ## Fehlerfälle
//!
//! | Fall | Verhalten |
//! |---|---|
//! | Anbieter fehlt/nicht bereit | Gesundheit VOR der ersten Zeile; `Unavailable` mit Grund und Hinweis, keine Zeile |
//! | Einzelne Synthese scheitert (Timeout, Backend, Kindprozess stürzt ab) | Zeile `ok=false` mit `error_kind`/`error`, der Lauf geht weiter |
//! | Fünf Fehlschläge in Folge | Abbruch (`Outcome::Aborted`), die Zeilen bleiben; sonst liefe ein totes System Stunden in Fristen |
//! | Abbruch (Strg+C) mitten in der Synthese | der Auftrag wird fallengelassen (Subprozess-Adapter setzen `kill_on_drop`), kein weiteres Audio, `Outcome::Interrupted` |
//! | Abbruch mitten im Schreiben | `rows.jsonl` wird zeilenweise geschrieben und geleert (nie eine halbe Zeile außer bei hartem Kill); WAV und `env.json` über `.part` + Umbenennen |
//! | Datenträger voll / Ordner nicht schreibbar | der Ordner wird VOR der ersten Synthese angelegt (Fehler sofort); ein späterer Schreibfehler beendet den Lauf mit `Io`, die bis dahin geschriebenen Zeilen bleiben |
//! | Anbieter liefert endlos Audio / riesiges Audio | je Synthese höchstens [`MAX_AUDIO_BYTES`]; darüber `bad_request` (Zeile), Chunk-Kanal begrenzt (Rückstau statt Wachstum; `sink_stall_count` steht in der Zusammenfassung) |
//! | Doppelte oder gefährliche Satz-IDs im Korpus | [`parse_corpus`] lehnt sie ab (WAV-Dateinamen kommen aus den IDs) |
//!
//! Kein `settings::get_settings(&AppHandle)` hier.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::contract::{
    sink_stall_count, AudioChunk, AudioEncoding, CancelToken, ProviderCaps, SynthesisOptions,
    TtsError,
};
use super::pipeline::next_request_id;
use super::registry::ProviderRegistry;
use super::wav::{parse_wav, wav_from_pcm};

pub const ROW_SCHEMA: &str = "tts-bench-row/1";
pub const ENV_SCHEMA: &str = "tts-bench-env/1";

/// Höchstens so viele Audio-Bytes je Synthese (≈ 23 Minuten bei 24 kHz mono PCM16).
pub const MAX_AUDIO_BYTES: usize = 64 * 1024 * 1024;
/// Fehlschläge in Folge, nach denen der Lauf abbricht.
pub const MAX_CONSECUTIVE_FAILURES: u32 = 5;
/// Frist je Synthese (Kaltstarts großer Modelle brauchen Zeit).
pub const SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(180);
/// Frist der Bereitschaftsprüfung vor dem Lauf.
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(10);
/// Kapazität des Stream-Kanals (wie bei der Probestimme).
const CHUNK_QUEUE: usize = 8;
/// Größte Korpusdatei, die gelesen wird.
const MAX_CORPUS_BYTES: u64 = 8 * 1024 * 1024;
/// Längster Satz im Korpus (Zeichen).
const MAX_SENTENCE_CHARS: usize = 5000;

// ---- Korpus ---------------------------------------------------------------------------

/// Ein Satz des Korpus (`corpus/README.md`: weitere Felder werden ignoriert).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusItem {
    pub id: String,
    pub text: String,
    pub smoke: bool,
}

/// Gültige Satz-ID: aus ihr entsteht der WAV-Dateiname, also nur
/// `[A-Za-z0-9._-]`, kein Anfang mit `.`/`-`, kein `..`, höchstens 128 Zeichen.
pub fn valid_utterance_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// JSONL-Korpus lesen. `smoke_only`: nur Sätze mit `"smoke": true`.
pub fn parse_corpus(jsonl: &str, smoke_only: bool) -> Result<Vec<CorpusItem>, String> {
    let jsonl = jsonl.strip_prefix('\u{feff}').unwrap_or(jsonl);
    let mut items: Vec<CorpusItem> = Vec::new();
    for (n, line) in jsonl.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let at = |what: &str| format!("Korpus Zeile {}: {what}", n + 1);
        let v: Value = serde_json::from_str(line).map_err(|e| at(&format!("kein JSON ({e})")))?;
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| at("Feld 'id' fehlt"))?;
        if !valid_utterance_id(id) {
            return Err(at(&format!(
                "ungültige Satz-ID '{}' (erlaubt: A-Z a-z 0-9 . _ -, höchstens 128 Zeichen)",
                id.chars().take(40).collect::<String>()
            )));
        }
        let text = v
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| at("Feld 'text' fehlt oder ist leer"))?;
        if text.chars().count() > MAX_SENTENCE_CHARS {
            return Err(at(&format!(
                "Satz '{id}' ist zu lang (höchstens {MAX_SENTENCE_CHARS} Zeichen)"
            )));
        }
        if items.iter().any(|i| i.id == id) {
            return Err(at(&format!("doppelte Satz-ID '{id}'")));
        }
        items.push(CorpusItem {
            id: id.to_string(),
            text: text.to_string(),
            smoke: v.get("smoke").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    if items.is_empty() {
        return Err("Der Korpus enthält keine Sätze".to_string());
    }
    if smoke_only {
        items.retain(|i| i.smoke);
        if items.is_empty() {
            return Err("Der Korpus enthält keine Sätze der Smoke-Teilmenge".to_string());
        }
    }
    Ok(items)
}

/// Korpusdatei lesen (höchstens 8 MiB).
pub fn load_corpus(path: &Path, smoke_only: bool) -> Result<Vec<CorpusItem>, String> {
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("Korpus '{}' nicht lesbar: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("Korpus '{}' ist keine Datei", path.display()));
    }
    if meta.len() > MAX_CORPUS_BYTES {
        return Err(format!(
            "Korpus '{}' ist zu groß (höchstens {} MiB)",
            path.display(),
            MAX_CORPUS_BYTES / (1024 * 1024)
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Korpus '{}' nicht lesbar: {e}", path.display()))?;
    parse_corpus(&text, smoke_only)
}

// ---- Optionen, Anbindung --------------------------------------------------------------

/// Parameter eines Laufs.
#[derive(Debug, Clone)]
pub struct BenchOptions {
    pub provider_id: String,
    pub voice: Option<String>,
    pub language: String,
    /// Nur für `env.json` (Modellname bzw. -datei).
    pub model: Option<String>,
    /// Läufe JE SATZ.
    pub warm: u32,
    pub cold: u32,
    pub out_dir: PathBuf,
    /// Frist je Synthese.
    pub timeout: Duration,
    pub max_audio_bytes: usize,
    pub max_consecutive_failures: u32,
    /// Feste Hinweise für `env.json`.
    pub notes: Vec<String>,
}

impl BenchOptions {
    pub fn new(provider_id: &str, language: &str, out_dir: impl Into<PathBuf>) -> Self {
        Self {
            provider_id: provider_id.to_string(),
            voice: None,
            language: language.to_string(),
            model: None,
            warm: 3,
            cold: 1,
            out_dir: out_dir.into(),
            timeout: SYNTHESIS_TIMEOUT,
            max_audio_bytes: MAX_AUDIO_BYTES,
            max_consecutive_failures: MAX_CONSECUTIVE_FAILURES,
            notes: Vec::new(),
        }
    }
}

/// Ergebnis von [`BenchBackend::cold_reset`].
#[derive(Debug, Clone, PartialEq)]
pub struct ColdReset {
    /// Zeit bis der Anbieter bereit ist; `None`, wo es keinen Start gibt, den
    /// der Benchmark kontrolliert.
    pub process_start_ms: Option<f64>,
    /// `provider_rebuilt` (nur die Registry wurde neu gebaut),
    /// `server_restarted` (der eigene Server wurde beendet und neu gestartet),
    /// `server_started` (es lief keiner, er wurde gestartet),
    /// `foreign_server_kept` (ein fremder Server läuft und bleibt: der
    /// Kaltlauf ist dann kein echter Kaltstart).
    pub kind: &'static str,
}

impl ColdReset {
    pub fn rebuilt() -> Self {
        Self {
            process_start_ms: None,
            kind: "provider_rebuilt",
        }
    }
}

/// Was die Schleife von der Umgebung braucht (siehe Moduldoku).
pub trait BenchBackend: Send + Sync {
    /// Eine frisch gebaute Registry (siehe [`bench_registry`]).
    fn fresh_registry(&self) -> Result<Arc<ProviderRegistry>, TtsError>;

    /// Vor einem Kaltlauf den Zustand des Anbieters zurücksetzen. Der
    /// Standard baut nur die Registry neu (Piper, Phonon: ein Prozess je Satz).
    fn cold_reset(&self) -> BoxFuture<'_, Result<ColdReset, TtsError>> {
        Box::pin(async { Ok(ColdReset::rebuilt()) })
    }
}

/// Registry für die Messung: OHNE Wiederholung, damit ein Fehlschlag sichtbar bleibt.
pub fn bench_registry() -> ProviderRegistry {
    ProviderRegistry::new().with_retry_policy(super::registry::RetryPolicy {
        max_attempts: 1,
        backoff: Duration::ZERO,
    })
}

/// Kodierung der Stream-Chunks: die erste der Fähigkeiten des Anbieters in der
/// Reihenfolge `Pcm16Le`, `F32Le`, `Wav` (ohne Angabe `Wav`).
pub fn pick_format(caps: &ProviderCaps) -> AudioEncoding {
    [
        AudioEncoding::Pcm16Le,
        AudioEncoding::F32Le,
        AudioEncoding::Wav,
    ]
    .into_iter()
    .find(|f| caps.formats.contains(f))
    .unwrap_or(AudioEncoding::Wav)
}

/// Schreibweise im Zeilen-Schema (`pcm16le | f32le | wav`).
pub fn encoding_name(e: AudioEncoding) -> &'static str {
    match e {
        AudioEncoding::Pcm16Le => "pcm16le",
        AudioEncoding::F32Le => "f32le",
        AudioEncoding::Wav => "wav",
    }
}

// ---- Messung einer Synthese -----------------------------------------------------------

/// Zusammengesetztes Roh-Audio (nur wenn angefordert).
#[derive(Debug, Clone)]
pub struct RawAudio {
    pub encoding: AudioEncoding,
    pub sample_rate: u32,
    pub channels: u16,
    pub pcm: Vec<u8>,
}

impl RawAudio {
    /// Als WAV-Container.
    pub fn to_wav(&self) -> Result<Vec<u8>, TtsError> {
        wav_from_pcm(self.encoding, self.sample_rate, self.channels, &self.pcm)
    }
}

/// Messwerte EINER Synthese.
#[derive(Debug, Clone)]
pub struct Measurement {
    pub ttfa_ms: f64,
    pub total_ms: f64,
    pub audio_ms: f64,
    pub sample_rate: u32,
    pub channels: u16,
    /// Format der gelieferten Chunks (Spalte `encoding`).
    pub chunk_encoding: AudioEncoding,
    pub chunk_count: usize,
    pub chunk_gaps_ms: Vec<f64>,
    pub audio: Option<RawAudio>,
}

impl Measurement {
    pub fn rtf(&self) -> f64 {
        self.total_ms / self.audio_ms
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Sammelt Chunks: Ankunftszeiten, Format, Frames, optional das Audio selbst.
struct Accumulator {
    keep_audio: bool,
    max_bytes: usize,
    arrivals: Vec<Duration>,
    /// (Rate, Kanäle, PCM-Kodierung, Kodierung der Chunks)
    format: Option<(u32, u16, AudioEncoding, AudioEncoding)>,
    frames: u64,
    bytes: usize,
    pcm: Vec<u8>,
}

impl Accumulator {
    fn new(keep_audio: bool, max_bytes: usize) -> Self {
        Self {
            keep_audio,
            max_bytes,
            arrivals: Vec::new(),
            format: None,
            frames: 0,
            bytes: 0,
            pcm: Vec::new(),
        }
    }

    fn push(&mut self, chunk: &AudioChunk, at: Duration) -> Result<(), TtsError> {
        // (Rate, Kanäle, PCM-Kodierung, Nutzdaten)
        let (rate, channels, pcm_enc, payload): (u32, u16, AudioEncoding, &[u8]) =
            match chunk.format.encoding {
                AudioEncoding::Wav => {
                    let info = parse_wav(&chunk.data)?;
                    let enc = info.encoding().ok_or_else(|| {
                        TtsError::backend("Teil-Audio hat ein nicht auswertbares WAV-Format")
                    })?;
                    (
                        info.sample_rate,
                        info.channels,
                        enc,
                        &chunk.data[info.data_offset..info.data_offset + info.data_len],
                    )
                }
                enc => (
                    chunk.format.sample_rate,
                    chunk.format.channels,
                    enc,
                    &chunk.data[..],
                ),
            };
        if rate == 0 || channels == 0 {
            return Err(TtsError::backend(
                "Teil-Audio ohne Abtastrate oder Kanalzahl",
            ));
        }
        let width = usize::from(pcm_enc.bytes_per_sample().unwrap_or(2)) * usize::from(channels);
        if payload.len() % width != 0 {
            return Err(TtsError::backend(
                "Teil-Audio endet mitten in einem Frame (abgeschnittener Strom?)",
            ));
        }
        let this = (rate, channels, pcm_enc, chunk.format.encoding);
        match self.format {
            None => self.format = Some(this),
            Some(first) if first != this => {
                return Err(TtsError::backend(
                    "Das Audioformat wechselt mitten im Strom",
                ))
            }
            Some(_) => {}
        }
        self.bytes = self.bytes.saturating_add(payload.len());
        if self.bytes > self.max_bytes {
            return Err(TtsError::bad_request(format!(
                "Der Anbieter lieferte mehr Audio als erlaubt (über {} MiB)",
                self.max_bytes / (1024 * 1024)
            )));
        }
        self.frames += (payload.len() / width) as u64;
        if self.keep_audio {
            self.pcm.extend_from_slice(payload);
        }
        self.arrivals.push(at);
        Ok(())
    }

    fn finish(self) -> Result<Measurement, TtsError> {
        let Some((rate, channels, pcm_enc, chunk_enc)) = self.format else {
            return Err(TtsError::backend("Der Anbieter lieferte kein Audio"));
        };
        if self.frames == 0 {
            return Err(TtsError::backend("Der Anbieter lieferte leeres Audio"));
        }
        let first = self.arrivals[0];
        let last = *self.arrivals.last().expect("mindestens ein Chunk");
        let gaps: Vec<f64> = self
            .arrivals
            .windows(2)
            .map(|w| ms(w[1].saturating_sub(w[0])))
            .collect();
        Ok(Measurement {
            ttfa_ms: ms(first),
            total_ms: ms(last),
            audio_ms: self.frames as f64 * 1000.0 / f64::from(rate),
            sample_rate: rate,
            channels,
            chunk_encoding: chunk_enc,
            chunk_count: self.arrivals.len(),
            chunk_gaps_ms: gaps,
            audio: self.keep_audio.then_some(RawAudio {
                encoding: pcm_enc,
                sample_rate: rate,
                channels,
                pcm: self.pcm,
            }),
        })
    }
}

/// Eine Synthese messen (Moduldoku). Fehler des Anbieters kommen unverändert
/// zurück; ein Fehler des Verbrauchers (zu viel Audio, Formatwechsel) hat
/// Vorrang, weil der Anbieter dann nur noch mit `Cancelled` antwortet.
pub async fn measure_once(
    registry: &ProviderRegistry,
    provider_id: &str,
    text: &str,
    opts: &SynthesisOptions,
    keep_audio: bool,
    max_audio_bytes: usize,
) -> Result<Measurement, TtsError> {
    let (tx, mut rx) = mpsc::channel::<AudioChunk>(CHUNK_QUEUE);
    let request_id = next_request_id("bench");
    let started = Instant::now();
    let produce = registry.stream_synthesize(provider_id, &request_id, text, opts, tx);
    let consume = async {
        let mut acc = Accumulator::new(keep_audio, max_audio_bytes);
        while let Some(chunk) = rx.recv().await {
            let at = started.elapsed();
            // Bei einem Fehler fällt `rx`: der Anbieter endet mit `Cancelled`.
            acc.push(&chunk, at)?;
        }
        Ok::<Accumulator, TtsError>(acc)
    };
    let (produced, consumed) = futures_util::future::join(produce, consume).await;
    let acc = consumed?;
    produced?;
    acc.finish()
}

// ---- Dateien --------------------------------------------------------------------------

/// Datei über `<name>.part` schreiben und umbenennen: nie eine halbe Datei unter dem Zielnamen.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".part");
    let tmp = path.with_file_name(name);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Fataler Fehler eines Laufs (einzelne Fehlschläge sind Zeilen, keine Fehler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchError {
    /// Falsche Eingabe (leerer Korpus, beide Laufzahlen 0, …).
    BadInput(String),
    /// Der Anbieter ist nicht bereit.
    Unavailable(String),
    /// Datei- oder Ordnerfehler.
    Io(String),
}

impl std::fmt::Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadInput(m) | Self::Unavailable(m) | Self::Io(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for BenchError {}

/// Wie ein Lauf endete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    /// Strg+C oder Abbruchmarke.
    Interrupted,
    /// Wiederholte Fehlschläge (Grund).
    Aborted(String),
}

#[derive(Debug, Clone)]
pub struct BenchSummary {
    pub provider: String,
    pub voice: Option<String>,
    pub language: String,
    pub rows_expected: usize,
    pub rows_written: usize,
    pub rows_ok: usize,
    pub rows_failed: usize,
    pub wavs_written: usize,
    pub outcome: Outcome,
    /// Wie oft der Stream-Kanal während des Laufs voll war (Rückstau).
    pub sink_stalls: u64,
    pub provider_version: Option<String>,
    pub device: Option<String>,
    pub out_dir: PathBuf,
}

impl BenchSummary {
    pub fn to_json(&self) -> Value {
        let (outcome, reason) = match &self.outcome {
            Outcome::Complete => ("complete", None),
            Outcome::Interrupted => ("interrupted", None),
            Outcome::Aborted(r) => ("aborted", Some(r.clone())),
        };
        json!({
            "schema": "tts-bench-summary/1",
            "provider": self.provider,
            "voice": self.voice,
            "language": self.language,
            "rows_expected": self.rows_expected,
            "rows_written": self.rows_written,
            "rows_ok": self.rows_ok,
            "rows_failed": self.rows_failed,
            "wavs_written": self.wavs_written,
            "outcome": outcome,
            "reason": reason,
            "sink_stalls": self.sink_stalls,
            "provider_version": self.provider_version,
            "device": self.device,
            "out_dir": self.out_dir.display().to_string(),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RunKind {
    Cold,
    Warm,
}

impl RunKind {
    fn name(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
        }
    }
}

enum Flow {
    Continue,
    Stop(Outcome),
}

fn utc_now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn io_err(what: &str, path: &Path, e: std::io::Error) -> BenchError {
    BenchError::Io(format!("{what} '{}': {e}", path.display()))
}

/// Ablauf EINES Laufs (Zustand, damit die Phasen ihn teilen).
struct Run<'a> {
    backend: &'a dyn BenchBackend,
    opts: &'a BenchOptions,
    items: &'a [CorpusItem],
    cancel: &'a CancelToken,
    log: &'a (dyn Fn(&str) + Sync),
    synth: SynthesisOptions,
    wav_kind: RunKind,
    rows: std::fs::File,
    total: usize,
    written: usize,
    ok: usize,
    failed: usize,
    wavs: usize,
    consecutive_failures: u32,
    notes: Vec<String>,
}

impl Run<'_> {
    fn write_row(&mut self, row: &Value) -> Result<(), BenchError> {
        let mut line = serde_json::to_string(row)
            .map_err(|e| BenchError::Io(format!("Zeile nicht darstellbar: {e}")))?;
        line.push('\n');
        let path = self.opts.out_dir.join("rows.jsonl");
        self.rows
            .write_all(line.as_bytes())
            .and_then(|()| self.rows.flush())
            .map_err(|e| io_err("rows.jsonl nicht schreibbar", &path, e))?;
        self.written += 1;
        Ok(())
    }

    fn row_base(
        &self,
        item: &CorpusItem,
        kind: RunKind,
        idx: u32,
        started_at: &str,
        reset: Option<&ColdReset>,
    ) -> serde_json::Map<String, Value> {
        let mut m = serde_json::Map::new();
        m.insert("schema".into(), json!(ROW_SCHEMA));
        m.insert("provider".into(), json!(self.opts.provider_id));
        m.insert("voice".into(), json!(self.opts.voice));
        m.insert("language".into(), json!(self.opts.language));
        m.insert("utterance_id".into(), json!(item.id));
        m.insert("run_kind".into(), json!(kind.name()));
        m.insert("run_idx".into(), json!(idx));
        m.insert("started_at".into(), json!(started_at));
        m.insert(
            "process_start_ms".into(),
            match (kind, reset.and_then(|r| r.process_start_ms)) {
                (RunKind::Cold, Some(ms)) => json!(round2(ms)),
                _ => Value::Null,
            },
        );
        if let (RunKind::Cold, Some(r)) = (kind, reset) {
            m.insert("cold_reset".into(), json!(r.kind));
        }
        m
    }

    /// Eine Messung samt Zeile (und WAV).
    async fn one(
        &mut self,
        registry: &Arc<ProviderRegistry>,
        item: &CorpusItem,
        kind: RunKind,
        idx: u32,
        reset: Option<&ColdReset>,
    ) -> Result<Flow, BenchError> {
        let started_at = utc_now();
        let keep = kind == self.wav_kind && idx == 0;
        let measured = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Ok(Flow::Stop(Outcome::Interrupted)),
            r = measure_once(
                registry,
                &self.opts.provider_id,
                &item.text,
                &self.synth,
                keep,
                self.opts.max_audio_bytes,
            ) => r,
        };
        let mut row = self.row_base(item, kind, idx, &started_at, reset);
        let ok = match measured {
            Ok(m) => {
                let wav_path = match (&m.audio, keep) {
                    (Some(audio), true) => Some(self.write_wav(&item.id, audio)?),
                    _ => None,
                };
                (self.log)(&format!(
                    "[{}/{}] {} {} #{}: ttfa {:.0} ms, gesamt {:.0} ms, rtf {:.3}",
                    self.written + 1,
                    self.total,
                    kind.name(),
                    item.id,
                    idx,
                    m.ttfa_ms,
                    m.total_ms,
                    m.rtf()
                ));
                let max_gap = m.chunk_gaps_ms.iter().copied().fold(0.0_f64, f64::max);
                row.insert("ok".into(), json!(true));
                row.insert("error_kind".into(), Value::Null);
                row.insert("error".into(), Value::Null);
                row.insert("ttfa_ms".into(), json!(round2(m.ttfa_ms)));
                row.insert("total_ms".into(), json!(round2(m.total_ms)));
                row.insert("audio_ms".into(), json!(round2(m.audio_ms)));
                row.insert(
                    "rtf".into(),
                    json!((m.rtf() * 100_000.0).round() / 100_000.0),
                );
                row.insert("sample_rate".into(), json!(m.sample_rate));
                row.insert("channels".into(), json!(m.channels));
                row.insert("encoding".into(), json!(encoding_name(m.chunk_encoding)));
                row.insert("chunk_count".into(), json!(m.chunk_count));
                row.insert(
                    "chunk_gaps_ms".into(),
                    json!(m
                        .chunk_gaps_ms
                        .iter()
                        .map(|g| round2(*g))
                        .collect::<Vec<_>>()),
                );
                row.insert("max_gap_ms".into(), json!(round2(max_gap)));
                row.insert("wav_path".into(), json!(wav_path));
                true
            }
            Err(e) => {
                (self.log)(&format!(
                    "[{}/{}] {} {} #{}: FEHLER {}: {}",
                    self.written + 1,
                    self.total,
                    kind.name(),
                    item.id,
                    idx,
                    e.kind.as_str(),
                    e.message
                ));
                self.error_fields(&mut row, &e);
                false
            }
        };
        self.write_row(&Value::Object(row))?;
        Ok(self.count(ok))
    }

    fn error_fields(&self, row: &mut serde_json::Map<String, Value>, e: &TtsError) {
        row.insert("ok".into(), json!(false));
        row.insert("error_kind".into(), json!(e.kind.as_str()));
        row.insert("error".into(), json!(e.message));
        for key in [
            "ttfa_ms",
            "total_ms",
            "audio_ms",
            "rtf",
            "sample_rate",
            "channels",
            "encoding",
            "chunk_count",
            "chunk_gaps_ms",
            "max_gap_ms",
            "wav_path",
        ] {
            row.insert(key.into(), Value::Null);
        }
    }

    /// Zählt das Ergebnis und entscheidet über den Abbruch nach Fehlschlägen in Folge.
    fn count(&mut self, ok: bool) -> Flow {
        if ok {
            self.ok += 1;
            self.consecutive_failures = 0;
            return Flow::Continue;
        }
        self.failed += 1;
        self.consecutive_failures += 1;
        if self.consecutive_failures >= self.opts.max_consecutive_failures {
            return Flow::Stop(Outcome::Aborted(format!(
                "{} Fehlschläge in Folge",
                self.consecutive_failures
            )));
        }
        Flow::Continue
    }

    fn write_wav(&mut self, utterance_id: &str, audio: &RawAudio) -> Result<String, BenchError> {
        let rel = format!("wav/{utterance_id}.wav");
        let path = self.opts.out_dir.join(&rel);
        let bytes = audio
            .to_wav()
            .map_err(|e| BenchError::Io(format!("WAV nicht erzeugbar: {}", e.message)))?;
        write_atomic(&path, &bytes).map_err(|e| io_err("WAV nicht schreibbar", &path, e))?;
        self.wavs += 1;
        Ok(rel)
    }

    /// Eine Zeile für einen Fehlschlag VOR der Messung (Zurücksetzen/Neuaufbau gescheitert).
    fn failed_before_measuring(
        &mut self,
        item: &CorpusItem,
        kind: RunKind,
        idx: u32,
        e: &TtsError,
    ) -> Result<Flow, BenchError> {
        (self.log)(&format!(
            "[{}/{}] {} {} #{}: FEHLER {} beim Zurücksetzen: {}",
            self.written + 1,
            self.total,
            kind.name(),
            item.id,
            idx,
            e.kind.as_str(),
            e.message
        ));
        let mut row = self.row_base(item, kind, idx, &utc_now(), None);
        self.error_fields(&mut row, e);
        self.write_row(&Value::Object(row))?;
        Ok(self.count(false))
    }

    async fn cold_phase(&mut self) -> Result<Option<Outcome>, BenchError> {
        let items = self.items;
        for item in items {
            for idx in 0..self.opts.cold {
                if self.cancel.is_cancelled() {
                    return Ok(Some(Outcome::Interrupted));
                }
                let reset = tokio::select! {
                    biased;
                    _ = self.cancel.cancelled() => return Ok(Some(Outcome::Interrupted)),
                    r = self.backend.cold_reset() => r,
                };
                let reset = match reset {
                    Ok(r) => r,
                    Err(e) => match self.failed_before_measuring(item, RunKind::Cold, idx, &e)? {
                        Flow::Continue => continue,
                        Flow::Stop(o) => return Ok(Some(o)),
                    },
                };
                if reset.kind == "foreign_server_kept" {
                    let note = "Ein fremd gestarteter Server lief und wurde nicht beendet: \
                                die Kaltläufe sind keine echten Kaltstarts."
                        .to_string();
                    if !self.notes.contains(&note) {
                        self.notes.push(note);
                    }
                }
                let registry = match self.backend.fresh_registry() {
                    Ok(r) => r,
                    Err(e) => match self.failed_before_measuring(item, RunKind::Cold, idx, &e)? {
                        Flow::Continue => continue,
                        Flow::Stop(o) => return Ok(Some(o)),
                    },
                };
                if let Flow::Stop(o) = self
                    .one(&registry, item, RunKind::Cold, idx, Some(&reset))
                    .await?
                {
                    return Ok(Some(o));
                }
            }
        }
        Ok(None)
    }

    async fn warm_phase(
        &mut self,
        registry: Arc<ProviderRegistry>,
    ) -> Result<Option<Outcome>, BenchError> {
        let items = self.items;
        // Aufwärmen: eine Synthese ohne Zeile (lädt Modell/Server, falls noch nicht geschehen).
        if let Some(first) = items.first() {
            let warmup = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return Ok(Some(Outcome::Interrupted)),
                r = measure_once(
                    &registry,
                    &self.opts.provider_id,
                    &first.text,
                    &self.synth,
                    false,
                    self.opts.max_audio_bytes,
                ) => r,
            };
            match warmup {
                Ok(m) => (self.log)(&format!("Aufwärmen: {:.0} ms (ohne Zeile)", m.total_ms)),
                Err(e) => (self.log)(&format!(
                    "Aufwärmen fehlgeschlagen ({}): {} — die Messung folgt trotzdem",
                    e.kind.as_str(),
                    e.message
                )),
            }
        }
        for item in items {
            for idx in 0..self.opts.warm {
                if let Flow::Stop(o) = self.one(&registry, item, RunKind::Warm, idx, None).await? {
                    return Ok(Some(o));
                }
            }
        }
        Ok(None)
    }
}

fn check_options(opts: &BenchOptions, items: &[CorpusItem]) -> Result<(), BenchError> {
    if opts.warm == 0 && opts.cold == 0 {
        return Err(BenchError::BadInput(
            "Beide Laufzahlen sind 0: es gäbe nichts zu messen".to_string(),
        ));
    }
    if items.is_empty() {
        return Err(BenchError::BadInput("Der Korpus ist leer".to_string()));
    }
    if opts.language.trim().len() < 2 || opts.language.len() > 16 {
        return Err(BenchError::BadInput(format!(
            "Ungültige Sprache '{}'",
            opts.language
        )));
    }
    if opts.max_consecutive_failures == 0 {
        return Err(BenchError::BadInput(
            "Die Zahl der erlaubten Fehlschläge in Folge muss größer als 0 sein".to_string(),
        ));
    }
    Ok(())
}

/// Schreibt `env.json` (atomar).
fn write_env(
    opts: &BenchOptions,
    version: &Option<String>,
    device: &Option<String>,
    notes: &[String],
) -> Result<(), BenchError> {
    let notes_text = (!notes.is_empty()).then(|| notes.join(" "));
    let env = json!({
        "schema": ENV_SCHEMA,
        "provider": opts.provider_id,
        "provider_version": version,
        "model": opts.model,
        "device": device,
        "voice": opts.voice,
        "language": opts.language,
        "app_version": env!("CARGO_PKG_VERSION"),
        "notes": notes_text,
    });
    let path = opts.out_dir.join("env.json");
    let bytes = serde_json::to_vec_pretty(&env)
        .map_err(|e| BenchError::Io(format!("env.json nicht darstellbar: {e}")))?;
    write_atomic(&path, &bytes).map_err(|e| io_err("env.json nicht schreibbar", &path, e))
}

/// Den Lauf ausführen (Moduldoku). Rückgabe: Zusammenfassung; `Interrupted` und
/// `Aborted` sind Ergebnisse, keine Fehler — die geschriebenen Zeilen bleiben.
pub async fn run_bench(
    backend: &dyn BenchBackend,
    opts: &BenchOptions,
    items: &[CorpusItem],
    cancel: &CancelToken,
    log: &(dyn Fn(&str) + Sync),
) -> Result<BenchSummary, BenchError> {
    check_options(opts, items)?;
    // Ordner VOR der ersten Synthese: ein nicht beschreibbarer Ort scheitert sofort.
    let wav_dir = opts.out_dir.join("wav");
    std::fs::create_dir_all(&wav_dir)
        .map_err(|e| io_err("Ausgabeordner nicht anlegbar", &opts.out_dir, e))?;

    let registry = backend
        .fresh_registry()
        .map_err(|e| BenchError::Unavailable(e.message))?;
    let provider = registry.get(&opts.provider_id).ok_or_else(|| {
        BenchError::BadInput(format!("Unbekannter Anbieter '{}'", opts.provider_id))
    })?;
    let descriptor = provider.descriptor();
    let health = registry
        .health(&opts.provider_id, HEALTH_TIMEOUT)
        .await
        .map_err(|e| BenchError::Unavailable(e.message))?;
    if !health.available {
        let reason = health
            .reason
            .clone()
            .unwrap_or_else(|| "nicht verfügbar".to_string());
        return Err(BenchError::Unavailable(match &health.hint {
            Some(h) if !h.trim().is_empty() => {
                format!("{} ist nicht bereit: {reason} — {h}", descriptor.label)
            }
            _ => format!("{} ist nicht bereit: {reason}", descriptor.label),
        }));
    }
    let device = health.device.and_then(|d| {
        serde_json::to_value(d)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
    });
    write_env(opts, &health.version, &device, &opts.notes)?;

    let rows_path = opts.out_dir.join("rows.jsonl");
    let rows = std::fs::File::create(&rows_path)
        .map_err(|e| io_err("rows.jsonl nicht anlegbar", &rows_path, e))?;
    let stalls_before = sink_stall_count();
    let mut run = Run {
        backend,
        opts,
        items,
        cancel,
        log,
        synth: SynthesisOptions {
            language: opts.language.clone(),
            voice: opts.voice.clone(),
            format: pick_format(&descriptor.caps),
            speed: None,
            seed: None,
            timeout: opts.timeout,
        },
        wav_kind: if opts.warm > 0 {
            RunKind::Warm
        } else {
            RunKind::Cold
        },
        rows,
        total: items.len() * (opts.warm + opts.cold) as usize,
        written: 0,
        ok: 0,
        failed: 0,
        wavs: 0,
        consecutive_failures: 0,
        notes: opts.notes.clone(),
    };

    let mut outcome = Outcome::Complete;
    if opts.cold > 0 {
        if let Some(o) = run.cold_phase().await? {
            outcome = o;
        }
    }
    if opts.warm > 0 && outcome == Outcome::Complete {
        if let Some(o) = run.warm_phase(registry).await? {
            outcome = o;
        }
    }

    write_env(opts, &health.version, &device, &run.notes)?;
    Ok(BenchSummary {
        provider: opts.provider_id.clone(),
        voice: opts.voice.clone(),
        language: opts.language.clone(),
        rows_expected: run.total,
        rows_written: run.written,
        rows_ok: run.ok,
        rows_failed: run.failed,
        wavs_written: run.wavs,
        outcome,
        sink_stalls: sink_stall_count().saturating_sub(stalls_before),
        provider_version: health.version,
        device,
        out_dir: opts.out_dir.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::contract::{
        AudioClip, ProviderDescriptor, ProviderHealth, TtsProvider, VoiceDescriptor,
    };
    use crate::managers::tts::providers::mock::{MockFailure, MockProvider};
    use std::sync::atomic::{AtomicU32, Ordering};

    // ---- Schnittstellenvertrag: row.schema.json aus dem Repo ----------------------------

    fn repo_file(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join(rel)
    }

    fn row_schema() -> Value {
        let path = repo_file("scripts/bench/tts/schema/row.schema.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Schema {} nicht lesbar: {e}", path.display()));
        serde_json::from_str(&text).unwrap()
    }

    fn type_ok(v: &Value, t: &str) -> bool {
        match t {
            "string" => v.is_string(),
            "number" => v.is_number(),
            "integer" => v.is_i64() || v.is_u64(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            "array" => v.is_array(),
            "object" => v.is_object(),
            _ => false,
        }
    }

    /// Die Schlüsselwörter, die `row.schema.json` benutzt (wie `ttsbench/rowschema.py`).
    fn check(value: &Value, schema: &Value, at: &str, errors: &mut Vec<String>) {
        if let Some(t) = schema.get("type") {
            let names: Vec<&str> = match t {
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                other => other.as_str().into_iter().collect(),
            };
            if !names.iter().any(|n| type_ok(value, n)) {
                errors.push(format!("{at}: Typ passt nicht zu {names:?}: {value}"));
                return;
            }
        }
        if let Some(c) = schema.get("const") {
            if value != c {
                errors.push(format!("{at}: erwartet {c}, ist {value}"));
            }
        }
        if let Some(Value::Array(options)) = schema.get("enum") {
            if !options.contains(value) {
                errors.push(format!("{at}: {value} nicht in {options:?}"));
            }
        }
        if let (Some(min), Some(n)) = (
            schema.get("minimum").and_then(Value::as_f64),
            value.as_f64(),
        ) {
            if n < min {
                errors.push(format!("{at}: {n} < {min}"));
            }
        }
        if let (Some(min), Some(s)) = (
            schema.get("minLength").and_then(Value::as_u64),
            value.as_str(),
        ) {
            if (s.chars().count() as u64) < min {
                errors.push(format!("{at}: zu kurz"));
            }
        }
        if let (Some(items), Some(list)) = (schema.get("items"), value.as_array()) {
            for (k, v) in list.iter().enumerate() {
                check(v, items, &format!("{at}[{k}]"), errors);
            }
        }
    }

    /// Fehlerliste; leer = gültig (Schema + die semantischen Regeln der Beschreibung).
    fn validate_row(row: &Value, schema: &Value) -> Vec<String> {
        let mut errors = Vec::new();
        for req in schema["required"].as_array().unwrap() {
            let key = req.as_str().unwrap();
            if row.get(key).is_none() {
                errors.push(format!("Pflichtfeld '{key}' fehlt"));
            }
        }
        for (key, sub) in schema["properties"].as_object().unwrap() {
            if let Some(v) = row.get(key) {
                check(v, sub, key, &mut errors);
            }
        }
        if !errors.is_empty() {
            return errors;
        }
        if row["ok"] == json!(true) {
            for f in [
                "ttfa_ms",
                "total_ms",
                "audio_ms",
                "rtf",
                "sample_rate",
                "channels",
                "encoding",
                "chunk_count",
            ] {
                if row.get(f).is_none_or(Value::is_null) {
                    errors.push(format!("ok=true, aber '{f}' fehlt oder ist null"));
                }
            }
            let gaps = row["chunk_gaps_ms"].as_array().map(Vec::len);
            let count = row["chunk_count"].as_u64();
            if let (Some(g), Some(c)) = (gaps, count) {
                if g as u64 != c.saturating_sub(1) {
                    errors.push("chunk_gaps_ms hat nicht chunk_count-1 Einträge".into());
                }
            }
        } else {
            for f in ["error_kind", "error"] {
                if row.get(f).and_then(Value::as_str).is_none_or(str::is_empty) {
                    errors.push(format!("ok=false, aber '{f}' fehlt"));
                }
            }
        }
        if row["run_kind"] == json!("warm") && !row["process_start_ms"].is_null() {
            errors.push("process_start_ms gibt es nur bei run_kind=cold".into());
        }
        errors
    }

    // ---- Hilfen -------------------------------------------------------------------------

    fn item(id: &str, text: &str, smoke: bool) -> CorpusItem {
        CorpusItem {
            id: id.to_string(),
            text: text.to_string(),
            smoke,
        }
    }

    fn items(n: usize) -> Vec<CorpusItem> {
        (1..=n)
            .map(|i| {
                item(
                    &format!("t-{i:03}"),
                    &format!("Dies ist Satz Nummer {i}."),
                    i == 1,
                )
            })
            .collect()
    }

    fn read_rows(dir: &Path) -> Vec<Value> {
        std::fs::read_to_string(dir.join("rows.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).expect("jede Zeile ist vollständiges JSON"))
            .collect()
    }

    fn opts(dir: &Path) -> BenchOptions {
        let mut o = BenchOptions::new("mock", "de", dir);
        o.voice = Some("mock-de".into());
        o.model = Some("sinus".into());
        o
    }

    /// Anbindung mit einem gemeinsamen Mock: „frisch“ heißt eine neue Registry.
    struct MockBackend {
        mock: Arc<dyn TtsProvider>,
        builds: AtomicU32,
        resets: AtomicU32,
        reset: ColdReset,
    }

    impl MockBackend {
        fn new(mock: Arc<dyn TtsProvider>) -> Self {
            Self {
                mock,
                builds: AtomicU32::new(0),
                resets: AtomicU32::new(0),
                reset: ColdReset {
                    process_start_ms: Some(1234.5),
                    kind: "server_restarted",
                },
            }
        }
    }

    impl BenchBackend for MockBackend {
        fn fresh_registry(&self) -> Result<Arc<ProviderRegistry>, TtsError> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            let registry = bench_registry();
            registry.register(self.mock.clone())?;
            Ok(Arc::new(registry))
        }

        fn cold_reset(&self) -> BoxFuture<'_, Result<ColdReset, TtsError>> {
            self.resets.fetch_add(1, Ordering::SeqCst);
            let reset = self.reset.clone();
            Box::pin(async move { Ok(reset) })
        }
    }

    async fn run(
        backend: &MockBackend,
        o: &BenchOptions,
        its: &[CorpusItem],
    ) -> Result<BenchSummary, BenchError> {
        run_bench(backend, o, its, &CancelToken::new("test"), &|_| {}).await
    }

    fn no_part_files(dir: &Path) {
        for sub in [dir.to_path_buf(), dir.join("wav")] {
            for entry in std::fs::read_dir(&sub).into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                assert!(!name.ends_with(".part"), "Rest einer halben Datei: {name}");
            }
        }
    }

    // ---- Zeilen, Schema, WAV ------------------------------------------------------------

    #[tokio::test]
    async fn die_zeilen_erfuellen_das_schema_des_harness_und_die_wavs_liegen_da() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new(Arc::new(
            MockProvider::new().chunked(3, Duration::from_millis(5)),
        ));
        let mut o = opts(dir.path());
        o.warm = 2;
        o.cold = 1;
        let its = items(3);
        let summary = run(&backend, &o, &its).await.unwrap();
        assert_eq!(summary.outcome, Outcome::Complete);
        assert_eq!(summary.rows_expected, 9);
        assert_eq!(summary.rows_written, 9);
        assert_eq!(summary.rows_ok, 9);

        let schema = row_schema();
        let rows = read_rows(dir.path());
        assert_eq!(rows.len(), 9);
        for row in &rows {
            let errors = validate_row(row, &schema);
            assert!(errors.is_empty(), "{errors:?} in {row}");
            assert_eq!(row["schema"], "tts-bench-row/1");
            assert_eq!(row["provider"], "mock");
            assert_eq!(row["voice"], "mock-de");
            assert_eq!(row["language"], "de");
        }
        // Reihenfolge: erst kalt (je Satz), dann warm; run_idx je (Satz, Art).
        let kinds: Vec<&str> = rows
            .iter()
            .map(|r| r["run_kind"].as_str().unwrap())
            .collect();
        assert_eq!(&kinds[..3], ["cold"; 3]);
        assert_eq!(&kinds[3..], ["warm"; 6]);
        // WAV nur für den warmen Lauf 0 jedes Satzes.
        for row in &rows {
            let expected = row["run_kind"] == "warm" && row["run_idx"] == 0;
            assert_eq!(!row["wav_path"].is_null(), expected, "{row}");
            if expected {
                let rel = row["wav_path"].as_str().unwrap();
                assert_eq!(
                    rel,
                    format!("wav/{}.wav", row["utterance_id"].as_str().unwrap())
                );
                let bytes = std::fs::read(dir.path().join(rel)).unwrap();
                let info = parse_wav(&bytes).unwrap();
                assert_eq!(
                    info.sample_rate,
                    row["sample_rate"].as_u64().unwrap() as u32
                );
                let audio_ms = row["audio_ms"].as_f64().unwrap();
                assert!((info.duration_ms as f64 - audio_ms).abs() < 1.5, "{row}");
            }
        }
        assert_eq!(summary.wavs_written, 3);
        no_part_files(dir.path());
    }

    #[tokio::test]
    async fn ohne_warmlaeufe_bekommt_der_kalte_lauf_null_die_wav() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new(Arc::new(MockProvider::new()));
        let mut o = opts(dir.path());
        o.warm = 0;
        o.cold = 2;
        let summary = run(&backend, &o, &items(2)).await.unwrap();
        assert_eq!(summary.rows_written, 4);
        let rows = read_rows(dir.path());
        for row in &rows {
            let expected = row["run_idx"] == 0;
            assert_eq!(!row["wav_path"].is_null(), expected, "{row}");
        }
        assert_eq!(summary.wavs_written, 2);
    }

    #[tokio::test]
    async fn chunk_zahlen_abstaende_und_ttfa_stimmen() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new(Arc::new(
            MockProvider::new().chunked(4, Duration::from_millis(15)),
        ));
        let mut o = opts(dir.path());
        o.warm = 1;
        o.cold = 0;
        run(&backend, &o, &items(1)).await.unwrap();
        let row = &read_rows(dir.path())[0];
        assert_eq!(row["chunk_count"], 4);
        let gaps: Vec<f64> = row["chunk_gaps_ms"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g.as_f64().unwrap())
            .collect();
        assert_eq!(gaps.len(), 3);
        assert!(gaps.iter().all(|g| *g >= 10.0), "{gaps:?}");
        let max = gaps.iter().copied().fold(0.0, f64::max);
        assert_eq!(row["max_gap_ms"].as_f64().unwrap(), max);
        let (ttfa, total) = (
            row["ttfa_ms"].as_f64().unwrap(),
            row["total_ms"].as_f64().unwrap(),
        );
        assert!(ttfa < total, "echtes Streaming: erster Chunk vor dem Ende");
        assert!(total >= 40.0, "drei Pausen à 15 ms");
        assert_eq!(row["encoding"], "pcm16le");
        let rtf = row["rtf"].as_f64().unwrap();
        assert!((rtf - total / row["audio_ms"].as_f64().unwrap()).abs() < 0.001);
    }

    #[tokio::test]
    async fn ein_einzelner_chunk_ist_kein_streaming_ttfa_gleich_gesamt() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new(Arc::new(
            MockProvider::new().latency(Duration::from_millis(20)),
        ));
        let mut o = opts(dir.path());
        o.warm = 1;
        o.cold = 0;
        run(&backend, &o, &items(1)).await.unwrap();
        let row = &read_rows(dir.path())[0];
        assert_eq!(row["chunk_count"], 1);
        assert_eq!(row["chunk_gaps_ms"], json!([]));
        assert_eq!(row["max_gap_ms"].as_f64(), Some(0.0));
        assert_eq!(row["ttfa_ms"], row["total_ms"]);
        assert!(row["ttfa_ms"].as_f64().unwrap() >= 15.0);
    }

    // ---- Kalt und warm ------------------------------------------------------------------

    #[tokio::test]
    async fn kaltlaeufe_setzen_zurueck_messen_den_start_und_bauen_den_anbieter_neu() {
        let dir = tempfile::tempdir().unwrap();
        let mock = Arc::new(MockProvider::new());
        let backend = MockBackend::new(mock.clone());
        let mut o = opts(dir.path());
        o.warm = 1;
        o.cold = 2;
        run(&backend, &o, &items(2)).await.unwrap();
        let rows = read_rows(dir.path());
        assert_eq!(rows.len(), 6);
        assert_eq!(
            backend.resets.load(Ordering::SeqCst),
            4,
            "je Kaltlauf ein Zurücksetzen"
        );
        assert_eq!(
            backend.builds.load(Ordering::SeqCst),
            1 + 4,
            "Start + je Kaltlauf frisch"
        );
        for row in &rows {
            if row["run_kind"] == "cold" {
                assert_eq!(row["process_start_ms"].as_f64(), Some(1234.5), "{row}");
                assert_eq!(row["cold_reset"], "server_restarted");
            } else {
                assert!(row["process_start_ms"].is_null(), "{row}");
                assert!(row.get("cold_reset").is_none());
            }
        }
        // 4 kalt + 1 Aufwärmen (ohne Zeile) + 2 warm.
        assert_eq!(mock.attempts(), 7);
    }

    #[tokio::test]
    async fn ein_fremder_server_wird_vermerkt() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = MockBackend::new(Arc::new(MockProvider::new()));
        backend.reset = ColdReset {
            process_start_ms: None,
            kind: "foreign_server_kept",
        };
        let mut o = opts(dir.path());
        o.warm = 0;
        o.cold = 1;
        run(&backend, &o, &items(1)).await.unwrap();
        let env: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("env.json")).unwrap())
                .unwrap();
        assert!(env["notes"]
            .as_str()
            .unwrap()
            .contains("fremd gestarteter Server"));
        let row = &read_rows(dir.path())[0];
        assert_eq!(row["cold_reset"], "foreign_server_kept");
        assert!(row["process_start_ms"].is_null());
    }

    #[tokio::test]
    async fn env_json_traegt_anbieter_version_modell_und_geraet() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new(Arc::new(MockProvider::new()));
        let mut o = opts(dir.path());
        o.warm = 1;
        o.cold = 0;
        o.notes = vec!["Hinweis eins.".into()];
        run(&backend, &o, &items(1)).await.unwrap();
        let env: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("env.json")).unwrap())
                .unwrap();
        assert_eq!(env["schema"], "tts-bench-env/1");
        assert_eq!(env["provider"], "mock");
        assert_eq!(env["provider_version"], "mock-1");
        assert_eq!(env["model"], "sinus");
        assert_eq!(env["device"], "cpu");
        assert_eq!(env["voice"], "mock-de");
        assert_eq!(env["language"], "de");
        assert_eq!(env["notes"], "Hinweis eins.");
        assert!(env["app_version"].is_string());
        no_part_files(dir.path());
    }

    // ---- Fehlschläge ----------------------------------------------------------------------

    #[tokio::test]
    async fn fehlschlaege_sind_zeilen_mit_error_kind_und_der_lauf_geht_weiter() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new(Arc::new(MockProvider::new().failing(MockFailure {
            error: TtsError::backend("Prozess abgestürzt"),
            at_chunk: 0,
            for_attempts: None,
        })));
        let mut o = opts(dir.path());
        o.warm = 2;
        o.cold = 0;
        let summary = run(&backend, &o, &items(2)).await.unwrap();
        assert_eq!(summary.outcome, Outcome::Complete);
        assert_eq!(
            (summary.rows_written, summary.rows_failed, summary.rows_ok),
            (4, 4, 0)
        );
        let schema = row_schema();
        for row in read_rows(dir.path()) {
            assert_eq!(validate_row(&row, &schema), Vec::<String>::new(), "{row}");
            assert_eq!(row["ok"], false);
            assert_eq!(row["error_kind"], "backend");
            assert_eq!(row["error"], "Prozess abgestürzt");
            assert!(row["ttfa_ms"].is_null() && row["wav_path"].is_null());
        }
    }

    #[tokio::test]
    async fn es_gibt_keine_stille_wiederholung_der_erste_fehlschlag_bleibt_sichtbar() {
        let dir = tempfile::tempdir().unwrap();
        // Ein Netzwerkfehler ist wiederholbar - die Registry der Messung wiederholt NIE.
        let backend = MockBackend::new(Arc::new(MockProvider::new().failing(MockFailure {
            error: TtsError::network("Aussetzer"),
            at_chunk: 0,
            for_attempts: Some(1),
        })));
        let mut o = opts(dir.path());
        o.warm = 0;
        o.cold = 1;
        run(&backend, &o, &items(2)).await.unwrap();
        let rows = read_rows(dir.path());
        assert_eq!(rows[0]["ok"], false);
        assert_eq!(rows[0]["error_kind"], "network");
        assert_eq!(rows[1]["ok"], true);
    }

    #[tokio::test]
    async fn fuenf_fehlschlaege_in_folge_brechen_den_lauf_ab_und_die_zeilen_bleiben() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new(Arc::new(MockProvider::new().failing(MockFailure {
            error: TtsError::timeout("keine Antwort"),
            at_chunk: 0,
            for_attempts: None,
        })));
        let mut o = opts(dir.path());
        o.warm = 1;
        o.cold = 0;
        o.max_consecutive_failures = 3;
        let summary = run(&backend, &o, &items(10)).await.unwrap();
        assert_eq!(
            summary.outcome,
            Outcome::Aborted("3 Fehlschläge in Folge".to_string())
        );
        assert_eq!(summary.rows_written, 3);
        assert_eq!(read_rows(dir.path()).len(), 3);
    }

    #[tokio::test]
    async fn ein_erfolg_setzt_den_zaehler_der_fehlschlaege_zurueck() {
        let dir = tempfile::tempdir().unwrap();
        // Der Mock scheitert nur beim 1. und 2. Versuch: 2 Fehlschläge, dann Erfolge.
        let backend = MockBackend::new(Arc::new(MockProvider::new().failing(MockFailure {
            error: TtsError::backend("kurz kaputt"),
            at_chunk: 0,
            for_attempts: Some(2),
        })));
        let mut o = opts(dir.path());
        o.warm = 0;
        o.cold = 1;
        o.max_consecutive_failures = 3;
        let summary = run(&backend, &o, &items(6)).await.unwrap();
        assert_eq!(summary.outcome, Outcome::Complete);
        assert_eq!((summary.rows_failed, summary.rows_ok), (2, 4));
    }

    #[tokio::test]
    async fn ein_nicht_bereiter_anbieter_endet_vor_der_ersten_zeile_mit_grund() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new(Arc::new(MockProvider::new().unavailable("Modell fehlt")));
        let mut o = opts(dir.path());
        o.warm = 1;
        let err = run(&backend, &o, &items(1)).await.unwrap_err();
        match err {
            BenchError::Unavailable(m) => assert!(m.contains("Modell fehlt"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(
            !dir.path().join("rows.jsonl").exists(),
            "keine Zeile, keine Datei"
        );
        assert!(!dir.path().join("env.json").exists());
    }

    #[tokio::test]
    async fn zu_viel_audio_wird_begrenzt_und_als_zeile_gemeldet() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new(Arc::new(MockProvider::new().chunked(4, Duration::ZERO)));
        let mut o = opts(dir.path());
        o.warm = 1;
        o.cold = 0;
        o.max_audio_bytes = 1000;
        run(&backend, &o, &items(1)).await.unwrap();
        let row = &read_rows(dir.path())[0];
        assert_eq!(row["ok"], false);
        assert_eq!(row["error_kind"], "bad_request");
        assert!(row["error"]
            .as_str()
            .unwrap()
            .contains("mehr Audio als erlaubt"));
    }

    // ---- Abbruch, Datenträger, Nebenläufigkeit --------------------------------------------

    #[tokio::test]
    async fn abbruch_mitten_im_lauf_laesst_vollstaendige_zeilen_und_keine_halben_dateien() {
        let dir = tempfile::tempdir().unwrap();
        let mock = Arc::new(MockProvider::new().latency(Duration::from_millis(60)));
        let backend = MockBackend::new(mock.clone());
        let mut o = opts(dir.path());
        o.warm = 4;
        o.cold = 0;
        let its = items(5);
        let cancel = CancelToken::new("test");
        let stopper = cancel.clone();
        let (result, ()) = tokio::join!(
            run_bench(&backend, &o, &its, &cancel, &|_| {}),
            async move {
                tokio::time::sleep(Duration::from_millis(250)).await;
                stopper.cancel();
            }
        );
        let summary = result.unwrap();
        assert_eq!(summary.outcome, Outcome::Interrupted);
        assert!(
            summary.rows_written > 0 && summary.rows_written < 20,
            "{}",
            summary.rows_written
        );
        assert_eq!(
            read_rows(dir.path()).len(),
            summary.rows_written,
            "nur ganze Zeilen"
        );
        no_part_files(dir.path());
        assert_eq!(
            mock.in_flight(),
            0,
            "der laufende Auftrag wurde fallengelassen"
        );
    }

    #[tokio::test]
    async fn ein_nicht_beschreibbarer_ausgabeort_scheitert_vor_der_ersten_synthese() {
        let dir = tempfile::tempdir().unwrap();
        // Der "Ordner" ist eine Datei: der Platz lässt sich nicht anlegen.
        let blocker = dir.path().join("kein-ordner");
        std::fs::write(&blocker, b"x").unwrap();
        let mock = Arc::new(MockProvider::new());
        let backend = MockBackend::new(mock.clone());
        let mut o = opts(&blocker);
        o.warm = 1;
        let err = run(&backend, &o, &items(1)).await.unwrap_err();
        assert!(matches!(err, BenchError::Io(_)), "{err:?}");
        assert_eq!(mock.attempts(), 0, "nichts synthetisiert");
    }

    /// Anbieter, der die gleichzeitig laufenden Synthesen zählt.
    struct Tracking {
        inner: MockProvider,
        current: AtomicU32,
        max: AtomicU32,
    }

    impl TtsProvider for Tracking {
        fn descriptor(&self) -> ProviderDescriptor {
            self.inner.descriptor()
        }
        fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
            self.inner.health_check()
        }
        fn list_voices<'a>(
            &'a self,
            language: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
            self.inner.list_voices(language)
        }
        fn synthesize<'a>(
            &'a self,
            text: &'a str,
            opts: &'a SynthesisOptions,
            cancel: &'a CancelToken,
        ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
            Box::pin(async move {
                let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
                self.max.fetch_max(now, Ordering::SeqCst);
                let r = self.inner.synthesize(text, opts, cancel).await;
                self.current.fetch_sub(1, Ordering::SeqCst);
                r
            })
        }
        fn cache_tag(&self, voice: Option<&str>) -> String {
            self.inner.cache_tag(voice)
        }
    }

    #[tokio::test]
    async fn die_messung_laeuft_strikt_nacheinander() {
        let dir = tempfile::tempdir().unwrap();
        let tracking = Arc::new(Tracking {
            inner: MockProvider::new().latency(Duration::from_millis(3)),
            current: AtomicU32::new(0),
            max: AtomicU32::new(0),
        });
        let backend = MockBackend::new(tracking.clone());
        let mut o = opts(dir.path());
        o.warm = 3;
        o.cold = 1;
        let summary = run(&backend, &o, &items(4)).await.unwrap();
        assert_eq!(summary.rows_ok, 16);
        assert_eq!(
            tracking.max.load(Ordering::SeqCst),
            1,
            "nie zwei Synthesen zugleich"
        );
    }

    // ---- Eingaben ---------------------------------------------------------------------------

    #[tokio::test]
    async fn unsinnige_eingaben_werden_vor_dem_start_abgelehnt() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new(Arc::new(MockProvider::new()));
        let mut o = opts(dir.path());
        o.warm = 0;
        o.cold = 0;
        assert!(matches!(
            run(&backend, &o, &items(1)).await,
            Err(BenchError::BadInput(_))
        ));
        o.warm = 1;
        assert!(matches!(
            run(&backend, &o, &[]).await,
            Err(BenchError::BadInput(_))
        ));
        o.language = "x".into();
        assert!(matches!(
            run(&backend, &o, &items(1)).await,
            Err(BenchError::BadInput(_))
        ));
        o.language = "de".into();
        o.provider_id = "gibt-es-nicht".into();
        assert!(matches!(
            run(&backend, &o, &items(1)).await,
            Err(BenchError::BadInput(_))
        ));
        assert_eq!(backend.mock.descriptor().id, "mock");
    }

    #[test]
    fn das_chunkformat_folgt_den_faehigkeiten() {
        let mut caps = MockProvider::new().descriptor().caps;
        assert_eq!(pick_format(&caps), AudioEncoding::Pcm16Le);
        caps.formats = vec![AudioEncoding::Wav];
        assert_eq!(
            pick_format(&caps),
            AudioEncoding::Wav,
            "Fish/Piper: nur Wav"
        );
        caps.formats = vec![AudioEncoding::Wav, AudioEncoding::F32Le];
        assert_eq!(pick_format(&caps), AudioEncoding::F32Le);
        caps.formats = Vec::new();
        assert_eq!(pick_format(&caps), AudioEncoding::Wav);
        assert_eq!(encoding_name(AudioEncoding::Pcm16Le), "pcm16le");
        assert_eq!(encoding_name(AudioEncoding::F32Le), "f32le");
        assert_eq!(encoding_name(AudioEncoding::Wav), "wav");
    }

    // ---- Korpus -----------------------------------------------------------------------------

    #[test]
    fn der_korpus_liest_id_text_und_smoke_und_ignoriert_den_rest() {
        let text =
            "\u{feff}{\"id\":\"a-1\",\"text\":\"Eins.\",\"smoke\":true,\"category\":\"x\"}\n\n\
                    {\"id\":\"a-2\",\"text\":\" Zwei. \"}\n";
        let all = parse_corpus(text, false).unwrap();
        assert_eq!(
            all,
            vec![item("a-1", "Eins.", true), item("a-2", "Zwei.", false)]
        );
        let smoke = parse_corpus(text, true).unwrap();
        assert_eq!(smoke, vec![item("a-1", "Eins.", true)]);
    }

    #[test]
    fn ein_schlechter_korpus_wird_mit_zeilennummer_abgelehnt() {
        let bad = |t: &str| parse_corpus(t, false).unwrap_err();
        assert!(bad("{\"text\":\"x\"}").contains("Zeile 1"));
        assert!(bad("kein json").contains("kein JSON"));
        assert!(bad("{\"id\":\"a\"}").contains("text"));
        assert!(bad("{\"id\":\"a\",\"text\":\"  \"}").contains("leer"));
        assert!(bad("").contains("keine Sätze"));
        for id in [
            "../x",
            "a/b",
            "a\\b",
            ".versteckt",
            "-x",
            "a b",
            "a..b",
            "",
            "ä",
        ] {
            let line = format!(
                "{{\"id\":{},\"text\":\"x\"}}",
                serde_json::to_string(id).unwrap()
            );
            assert!(bad(&line).contains("ungültige Satz-ID"), "{id:?}");
        }
        assert!(
            bad("{\"id\":\"a\",\"text\":\"x\"}\n{\"id\":\"a\",\"text\":\"y\"}")
                .contains("doppelte")
        );
        assert!(parse_corpus("{\"id\":\"a\",\"text\":\"x\"}", true)
            .unwrap_err()
            .contains("Smoke"));
        let long = format!(
            "{{\"id\":\"a\",\"text\":\"{}\"}}",
            "x".repeat(MAX_SENTENCE_CHARS + 1)
        );
        assert!(bad(&long).contains("zu lang"));
    }

    #[test]
    fn der_echte_korpus_de_v1_laesst_sich_lesen() {
        let path = repo_file("scripts/bench/tts/corpus/de-v1.jsonl");
        let all = load_corpus(&path, false).unwrap();
        assert!(all.len() >= 100, "{}", all.len());
        let smoke = load_corpus(&path, true).unwrap();
        assert_eq!(smoke.len(), 12);
        assert!(all.iter().all(|i| valid_utterance_id(&i.id)));
    }

    #[test]
    fn eine_fehlende_oder_zu_grosse_korpusdatei_ist_ein_fehler() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_corpus(&dir.path().join("gibt-es-nicht.jsonl"), false).is_err());
        assert!(load_corpus(dir.path(), false)
            .unwrap_err()
            .contains("keine Datei"));
    }

    #[test]
    fn die_zusammenfassung_nennt_ergebnis_und_grund() {
        let s = BenchSummary {
            provider: "mock".into(),
            voice: None,
            language: "de".into(),
            rows_expected: 4,
            rows_written: 3,
            rows_ok: 1,
            rows_failed: 2,
            wavs_written: 1,
            outcome: Outcome::Aborted("3 Fehlschläge in Folge".into()),
            sink_stalls: 0,
            provider_version: Some("1".into()),
            device: Some("cpu".into()),
            out_dir: PathBuf::from("x"),
        };
        let j = s.to_json();
        assert_eq!(j["outcome"], "aborted");
        assert_eq!(j["reason"], "3 Fehlschläge in Folge");
        assert_eq!(j["rows_written"], 3);
    }
}
