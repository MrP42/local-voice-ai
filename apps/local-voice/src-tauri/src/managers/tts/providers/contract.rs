//! Typen und Vertrag der Anbieterschicht (Überblick: `providers/mod.rs`).
//!
//! Alles, was die UI, die CLI oder die Agentenbrücke je über einen Anbieter
//! erfährt, steht hier: Deskriptor, Fähigkeiten, Lizenz, Gesundheit, Stimmen,
//! Fehlerklassen. Die Typen, die über IPC gehen, tragen serde + `specta::Type`
//! (Konvention des Projekts: Felder `snake_case`, Enum-Werte `snake_case`).
//! `AudioChunk`/`AudioClip` tragen rohe Audiobytes und gehen NIE über IPC —
//! sie sind bewusst ohne serde.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, Notify};

/// Wie verbindlich ein Anbieter angeboten wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTier {
    /// Teil des Standardumfangs (Fish, Piper).
    Supported,
    /// Nur nach ausdrücklichem Opt-in sichtbar.
    Experimental,
    /// Nur für Benchmark/Tests (Mock).
    BenchmarkOnly,
    /// Bekannt, aber hier nicht benutzbar (nur Statuszeile).
    Unavailable,
}

/// Wie ein Anbieter Audio liefert. Ehrlich gemeint: nur ECHTE Teil-Audios
/// vor dem Ende der Synthese sind `Chunk`; ein Anbieter, der erst den ganzen
/// Satz berechnet, ist `Sentence`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum StreamingMode {
    None,
    Sentence,
    Chunk,
}

/// Wo gerechnet wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    Cpu,
    Cuda,
    Metal,
    Remote,
}

/// Kodierung von Audiodaten: rohes PCM (16 Bit little endian bzw. 32 Bit
/// Float) oder ein vollständiger WAV-Container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AudioEncoding {
    // Ausdrücklich benannt: serde schreibt `pcm16_le`, specta würde aus
    // `snake_case` sonst `pcm_16_le` für die TypeScript-Typen machen — JSON und
    // Typ liefen auseinander (vom Export-Test gefunden).
    #[serde(rename = "pcm16_le")]
    Pcm16Le,
    #[serde(rename = "f32_le")]
    F32Le,
    Wav,
}

impl AudioEncoding {
    /// Bytes je Einzelwert; `None` beim Container (dort steht es im Header).
    pub fn bytes_per_sample(self) -> Option<u16> {
        match self {
            Self::Pcm16Le => Some(2),
            Self::F32Le => Some(4),
            Self::Wav => None,
        }
    }
}

/// Format von Audiodaten. Bei `encoding == Wav` beschreiben Rate und Kanäle
/// das Audio IM Container (sie müssen zum Header passen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub encoding: AudioEncoding,
}

/// Ein Teil-Audio eines Streams. `seq` zählt ab 0 lückenlos, genau der
/// letzte Chunk trägt `is_last`. `request_id` stammt aus dem [`CancelToken`]
/// des Auftrags. `elapsed_ms` ist die Zeit seit Beginn des Auftrags (der
/// erste Chunk ergibt damit die Zeit bis zum ersten Ton).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    pub request_id: String,
    pub seq: u32,
    pub format: AudioFormat,
    pub data: Vec<u8>,
    pub is_last: bool,
    pub elapsed_ms: u64,
}

/// Ein fertig synthetisierter Clip. `data` ist IMMER ein vollständiges WAV
/// (`format.encoding == Wav`), `duration_ms` aus dem Header berechnet.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioClip {
    pub format: AudioFormat,
    pub data: Vec<u8>,
    pub duration_ms: u64,
}

/// Lizenzangaben. `verified` heißt: Quelle UND Datum der Prüfung sind
/// bekannt (`source_url`, `verified_on`). Alles andere ist `false` und
/// braucht eine `note`, die sagt, was noch zu prüfen ist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct LicenseInfo {
    /// Lizenz des Codes (SPDX-Kennung bzw. Klartext).
    pub code: String,
    /// Lizenz der Gewichte/Stimmen.
    pub weights: String,
    pub non_commercial: bool,
    pub verified: bool,
    pub source_url: String,
    /// Datum der Prüfung (`JJJJ-MM-TT`), nur zusammen mit `verified`.
    pub verified_on: Option<String>,
    pub note: String,
}

/// Die wirksame Konfiguration eines Anbieters, soweit die Guardrails sie
/// brauchen. Gehört zum INSTANZ-Stand: [`TtsProvider::use_subject`] liefert sie
/// aus der Konfiguration, mit der die Instanz gebaut wurde — Prüfgegenstand und
/// Instanz können deshalb nie aus verschiedenen Ständen stammen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct UseSubject {
    /// Modell/Gewichte-Kennung (geht in den Fingerabdruck ein).
    pub model: String,
    /// Adresse, die der Anbieter anspricht (`None` = lokal/Programm).
    pub base_url: Option<String>,
}

/// Was ein Anbieter kann.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProviderCaps {
    /// Sprachkürzel (`de`, `en`, …); `und` = sprachunabhängig/mehrsprachig.
    pub languages: Vec<String>,
    pub streaming: StreamingMode,
    /// Kodierungen, die `SynthesisOptions::format` für den Stream annimmt.
    pub formats: Vec<AudioEncoding>,
    pub sample_rates: Vec<u32>,
    pub cloning: bool,
    pub devices: Vec<Device>,
    /// Plattformkennungen wie im Katalog (`windows-x64`, `macos-aarch64`, …).
    pub platforms: Vec<String>,
    pub style_tags: bool,
}

/// Statische Beschreibung eines Anbieters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProviderDescriptor {
    /// Stabile Kennung, `[a-z0-9][a-z0-9_-]*`.
    pub id: String,
    pub label: String,
    pub tier: ProviderTier,
    /// Nur nach Freischaltung durch den Nutzer anbieten (Guardrails in `commands`).
    pub opt_in: bool,
    pub caps: ProviderCaps,
    pub license: LicenseInfo,
}

/// Ergebnis einer Gesundheitsprüfung. Nicht verfügbar heißt immer: `reason`
/// sagt warum (für den Nutzer lesbar), `hint` was zu tun ist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProviderHealth {
    pub available: bool,
    pub reason: Option<String>,
    pub hint: Option<String>,
    pub device: Option<Device>,
    pub version: Option<String>,
    /// Dauer der Prüfung in Millisekunden (die Registry setzt den gemessenen Wert).
    pub checked_ms: u64,
}

impl ProviderHealth {
    pub fn ok() -> Self {
        Self {
            available: true,
            reason: None,
            hint: None,
            device: None,
            version: None,
            checked_ms: 0,
        }
    }

    pub fn unavailable(reason: impl Into<String>, hint: Option<String>) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
            hint,
            device: None,
            version: None,
            checked_ms: 0,
        }
    }
}

/// Eine wählbare Stimme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct VoiceDescriptor {
    pub id: String,
    pub label: String,
    /// Sprachkürzel; `und` = beliebige Sprache (z. B. geklonte Fish-Stimme).
    pub language: String,
    pub gender: Option<String>,
    /// Eigene Lizenz der Stimme, wenn sie von der des Anbieters abweicht.
    pub license: Option<LicenseInfo>,
}

/// Optionen EINES Syntheseauftrags.
///
/// `format` steuert nur die Kodierung der Stream-Chunks
/// ([`TtsProvider::stream_synthesize`]); `synthesize` liefert immer WAV.
/// `timeout` ist die Gesamtfrist des Auftrags (bei Streams bis zum letzten
/// Chunk), kein Leerlauf-Timeout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SynthesisOptions {
    pub language: String,
    pub voice: Option<String>,
    pub format: AudioEncoding,
    pub speed: Option<f32>,
    pub seed: Option<i64>,
    pub timeout: Duration,
}

impl Default for SynthesisOptions {
    fn default() -> Self {
        Self {
            language: "de".to_string(),
            voice: None,
            format: AudioEncoding::Wav,
            speed: None,
            seed: None,
            timeout: Duration::from_secs(60),
        }
    }
}

/// Fehlerklassen. Die Klasse entscheidet über Wiederholung, Anzeige und
/// Abbruchverhalten — nicht der Meldungstext.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TtsErrorKind {
    Unavailable,
    InvalidConfig,
    LicenseBlocked,
    PrivacyBlocked,
    Timeout,
    Cancelled,
    BadRequest,
    Backend,
    Network,
}

impl TtsErrorKind {
    /// Stabile Kennung für Logs und Agentenbrücke.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::InvalidConfig => "invalid_config",
            Self::LicenseBlocked => "license_blocked",
            Self::PrivacyBlocked => "privacy_blocked",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::BadRequest => "bad_request",
            Self::Backend => "backend",
            Self::Network => "network",
        }
    }
}

/// Fehler eines Anbieters. `retryable`: ein erneuter Versuch kann gelingen
/// (Netzwerkaussetzer, 5xx). Abbruch, Fristablauf, Konfigurations-, Lizenz-
/// und Anfragefehler sind NIE wiederholbar — dafür sorgen die Konstruktoren;
/// `with_retryable` ist nur für `Network`/`Backend` gedacht.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TtsError {
    pub kind: TtsErrorKind,
    pub retryable: bool,
    pub message: String,
}

impl TtsError {
    /// `retryable` folgt der Klasse: nur `Network` ist von sich aus wiederholbar.
    pub fn new(kind: TtsErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            retryable: matches!(kind, TtsErrorKind::Network),
            message: message.into(),
        }
    }

    /// Wiederholbarkeit überschreiben. Bei Klassen, die nie wiederholbar sein
    /// dürfen, bleibt `false` bestehen.
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        let never = matches!(
            self.kind,
            TtsErrorKind::Cancelled
                | TtsErrorKind::Timeout
                | TtsErrorKind::InvalidConfig
                | TtsErrorKind::LicenseBlocked
                | TtsErrorKind::PrivacyBlocked
                | TtsErrorKind::BadRequest
                | TtsErrorKind::Unavailable
        );
        self.retryable = retryable && !never;
        self
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(TtsErrorKind::Unavailable, message)
    }
    pub fn invalid_config(message: impl Into<String>) -> Self {
        Self::new(TtsErrorKind::InvalidConfig, message)
    }
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(TtsErrorKind::BadRequest, message)
    }
    pub fn backend(message: impl Into<String>) -> Self {
        Self::new(TtsErrorKind::Backend, message)
    }
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(TtsErrorKind::Network, message)
    }
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(TtsErrorKind::Timeout, message)
    }
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(TtsErrorKind::Cancelled, message)
    }
}

impl std::fmt::Display for TtsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TtsError {}

/// Brücke zu den `String`-Fehlern des bestehenden `TtsEngine`-Pfads.
impl From<TtsError> for String {
    fn from(e: TtsError) -> String {
        e.message
    }
}

struct CancelInner {
    request_id: String,
    cancelled: AtomicBool,
    notify: Notify,
}

/// Abbruchmarke eines Auftrags. Klonbar (alle Klone teilen den Zustand),
/// trägt die `request_id` des Auftrags und lässt sich sowohl abfragen
/// ([`is_cancelled`](Self::is_cancelled), für Blocking-Code) als auch
/// erwarten ([`cancelled`](Self::cancelled), für `select!`).
#[derive(Clone)]
pub struct CancelToken {
    inner: Arc<CancelInner>,
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelToken")
            .field("request_id", &self.inner.request_id)
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancelToken {
    pub fn new(request_id: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(CancelInner {
                request_id: request_id.into(),
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    /// Marke ohne Registry-Eintrag (Tests, einmalige Aufrufe).
    pub fn detached() -> Self {
        Self::new("detached")
    }

    pub fn request_id(&self) -> &str {
        &self.inner.request_id
    }

    /// Abbruch auslösen; idempotent.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Wird fertig, sobald abgebrochen wurde (sofort, wenn schon geschehen).
    pub async fn cancelled(&self) {
        loop {
            // Erst registrieren, dann prüfen: ein `cancel` zwischen Prüfung
            // und Warten ginge sonst verloren (`Notified` empfängt
            // `notify_waiters` ab der Erzeugung, auch ungepollt).
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }

    /// Dieselbe Marke (nicht nur gleiche Kennung)?
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Gültige Anbieterkennung: `[a-z0-9][a-z0-9_-]*`.
pub fn is_valid_provider_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Passt eine Stimme der Sprache `voice_language` zum Filter `wanted`?
/// Verglichen wird nur das Hauptkürzel (`de_DE`, `de-AT` → `de`);
/// `und` (sprachunabhängig) passt immer, kein Filter passt alles.
pub fn language_matches(voice_language: &str, wanted: Option<&str>) -> bool {
    let Some(wanted) = wanted else {
        return true;
    };
    let primary = |s: &str| {
        s.split(['_', '-'])
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    };
    let voice = primary(voice_language);
    voice == "und" || voice == primary(wanted)
}

/// Führt `fut` unter Abbruch- und Zeitgrenze aus. Abbruch ergibt
/// `Cancelled`, Fristablauf `Timeout`; in beiden Fällen wird `fut`
/// fallengelassen — Subprozess-Adapter setzen `kill_on_drop`, damit dabei
/// kein Restprozess bleibt. Schon abgebrochene Marken starten `fut` gar nicht.
/// Alle Adapter benutzen diese Funktion für `synthesize`, damit Abbruch und
/// Frist überall gleich wirken.
pub async fn guarded<T>(
    cancel: &CancelToken,
    timeout: Duration,
    what: &str,
    fut: impl Future<Output = Result<T, TtsError>>,
) -> Result<T, TtsError> {
    if cancel.is_cancelled() {
        return Err(TtsError::cancelled(format!("{what}: abgebrochen")));
    }
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(TtsError::cancelled(format!("{what}: abgebrochen"))),
        r = tokio::time::timeout(timeout, fut) => match r {
            Ok(inner) => inner,
            Err(_) => Err(TtsError::timeout(format!(
                "{what}: keine Antwort nach {} ms",
                timeout.as_millis()
            ))),
        },
    }
}

/// Wie oft ein Stream warten musste, weil der Empfänger voll war
/// (Rückstau). Überlauf wird gezählt statt verschluckt: kein Chunk geht
/// verloren, aber ein dauerhaft wachsender Wert zeigt einen zu langsamen
/// Abnehmer.
static SINK_STALLS: AtomicU64 = AtomicU64::new(0);

pub fn sink_stall_count() -> u64 {
    SINK_STALLS.load(Ordering::Relaxed)
}

/// Einen Chunk in den Stream-Kanal legen. Der Kanal ist BEGRENZT; ist er
/// voll, wartet der Anbieter (Rückstau, `sink_stall_count` zählt mit) — aber
/// nur, bis abgebrochen wird. Ist der Empfänger weg, endet der Auftrag mit
/// `Cancelled`. Prüft den Abbruch auch bei freiem Kanal: so reagiert jeder
/// Stream zwischen zwei Chunks. Alle Anbieter senden NUR über diese Funktion.
pub async fn send_chunk(
    sink: &mpsc::Sender<AudioChunk>,
    chunk: AudioChunk,
    cancel: &CancelToken,
) -> Result<(), TtsError> {
    if cancel.is_cancelled() {
        return Err(TtsError::cancelled("Stream abgebrochen"));
    }
    match sink.try_send(chunk) {
        Ok(()) => Ok(()),
        Err(TrySendError::Closed(_)) => Err(TtsError::cancelled("Empfänger des Streams ist weg")),
        Err(TrySendError::Full(chunk)) => {
            SINK_STALLS.fetch_add(1, Ordering::Relaxed);
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(TtsError::cancelled("Stream abgebrochen")),
                sent = sink.send(chunk) => sent
                    .map_err(|_| TtsError::cancelled("Empfänger des Streams ist weg")),
            }
        }
    }
}

/// Der Anbietervertrag. Dyn-fähig (`Arc<dyn TtsProvider>`) über `BoxFuture`
/// statt `async-trait` (keine neue Dependency); Implementierungen schreiben
/// `Box::pin(async move { … })`.
///
/// Pflichten jeder Implementierung (prüft `contract_tests::contract_suite`):
/// * `synthesize`/`stream_synthesize` beachten `cancel` und `opts.timeout`
///   (am einfachsten über [`guarded`] und [`send_chunk`]);
/// * Fehler tragen die richtige [`TtsErrorKind`]; leerer Text ist `BadRequest`;
/// * Streams liefern `seq` ab 0 lückenlos, genau EINEN Chunk mit `is_last`
///   als letzten, und im Format `opts.format`;
/// * nicht verfügbar → `health_check` nennt einen `reason`, `synthesize`
///   endet mit `Unavailable` (ohne Prozessstart).
pub trait TtsProvider: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;

    fn health_check(&self) -> BoxFuture<'_, ProviderHealth>;

    /// Stimmen, optional auf eine Sprache eingegrenzt (`de`, `de_DE` → `de`).
    /// Stimmen mit Sprache `und` passen immer.
    fn list_voices<'a>(
        &'a self,
        language: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>>;

    /// Einen Text komplett synthetisieren; Ergebnis ist ein WAV-Clip.
    fn synthesize<'a>(
        &'a self,
        text: &'a str,
        opts: &'a SynthesisOptions,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<AudioClip, TtsError>>;

    /// Streamen. Standard: `synthesize` aufrufen und EIN Chunk mit
    /// `is_last = true` liefern — korrekt für Anbieter mit Modus `Sentence`
    /// bzw. `None`. Nur Anbieter mit echten Teil-Audios überschreiben das
    /// (und deklarieren dann `StreamingMode::Chunk`).
    fn stream_synthesize<'a>(
        &'a self,
        text: &'a str,
        opts: &'a SynthesisOptions,
        cancel: &'a CancelToken,
        sink: mpsc::Sender<AudioChunk>,
    ) -> BoxFuture<'a, Result<(), TtsError>> {
        Box::pin(async move {
            let started = Instant::now();
            let clip = self.synthesize(text, opts, cancel).await?;
            let (format, data) = super::wav::chunk_payload(clip, opts.format)?;
            let chunk = AudioChunk {
                request_id: cancel.request_id().to_string(),
                seq: 0,
                format,
                data,
                is_last: true,
                elapsed_ms: started.elapsed().as_millis() as u64,
            };
            send_chunk(&sink, chunk, cancel).await
        })
    }

    /// Anteil des Anbieters am Cache-Schlüssel. Neue Anbieter:
    /// `"<id>:<modell>:<stimme>"`; Fish behält den leeren Tag (Legacy-Regel,
    /// siehe `engine::fish_cache_tag`), Piper `"piper/<stimme>"`.
    fn cache_tag(&self, voice: Option<&str>) -> String;

    /// Wohin diese INSTANZ spricht und mit welchem Modell: der Gegenstand der
    /// Guardrails ([`super::policy::check_use`]). Kommt aus der Konfiguration,
    /// mit der die Instanz gebaut wurde — nie aus einem anderen Stand. Vorgabe:
    /// leer (Fish, Piper: kein Modell, keine Adresse).
    fn use_subject(&self) -> UseSubject {
        UseSubject::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fehlerklassen_sind_nur_dort_wiederholbar_wo_es_sinn_hat() {
        assert!(TtsError::network("x").retryable);
        assert!(!TtsError::backend("x").retryable);
        assert!(TtsError::backend("x").with_retryable(true).retryable);
        // Diese Klassen lassen sich nicht wiederholbar machen.
        for e in [
            TtsError::cancelled("x"),
            TtsError::timeout("x"),
            TtsError::bad_request("x"),
            TtsError::invalid_config("x"),
            TtsError::unavailable("x"),
            TtsError::new(TtsErrorKind::LicenseBlocked, "x"),
            TtsError::new(TtsErrorKind::PrivacyBlocked, "x"),
        ] {
            assert!(!e.clone().with_retryable(true).retryable, "{:?}", e.kind);
        }
    }

    #[test]
    fn fehler_konvertiert_nach_string_mit_der_meldung() {
        let s: String = TtsError::backend("kaputt").into();
        assert_eq!(s, "kaputt");
        assert_eq!(TtsError::backend("kaputt").to_string(), "kaputt");
    }

    #[test]
    fn serde_werte_sind_snake_case() {
        assert_eq!(
            serde_json::to_string(&AudioEncoding::Pcm16Le).unwrap(),
            "\"pcm16_le\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderTier::BenchmarkOnly).unwrap(),
            "\"benchmark_only\""
        );
        assert_eq!(
            serde_json::to_string(&TtsErrorKind::LicenseBlocked).unwrap(),
            "\"license_blocked\""
        );
        assert_eq!(TtsErrorKind::InvalidConfig.as_str(), "invalid_config");
    }

    /// Alle IPC-Typen lassen sich nach TypeScript exportieren (u64/i64 als number),
    /// damit P3 sie ohne Nacharbeit in Befehle einhängen kann.
    #[test]
    fn ipc_typen_lassen_sich_nach_typescript_exportieren() {
        let mut types = specta::TypeCollection::default();
        types.register::<ProviderDescriptor>();
        types.register::<ProviderHealth>();
        types.register::<VoiceDescriptor>();
        types.register::<SynthesisOptions>();
        types.register::<TtsError>();
        types.register::<AudioFormat>();
        types.register::<AudioEncoding>();
        let ts = specta_typescript::Typescript::default()
            .bigint(specta_typescript::BigIntExportBehavior::Number)
            .export(&types)
            .unwrap();
        for needle in [
            "ProviderDescriptor",
            "checked_ms: number",
            "verified_on: string | null",
        ] {
            assert!(
                ts.contains(needle),
                "{needle} fehlt in:
{ts}"
            );
        }
        // Jeder Enum-Wert steht im TypeScript-Typ GENAU so, wie serde ihn schreibt.
        fn json<T: Serialize>(values: &[T]) -> Vec<String> {
            values
                .iter()
                .map(|v| serde_json::to_string(v).unwrap())
                .collect()
        }
        let mut expected = json(&[
            AudioEncoding::Pcm16Le,
            AudioEncoding::F32Le,
            AudioEncoding::Wav,
        ]);
        expected.extend(json(&[
            ProviderTier::Supported,
            ProviderTier::Experimental,
            ProviderTier::BenchmarkOnly,
            ProviderTier::Unavailable,
        ]));
        expected.extend(json(&[
            StreamingMode::None,
            StreamingMode::Sentence,
            StreamingMode::Chunk,
        ]));
        expected.extend(json(&[
            Device::Cpu,
            Device::Cuda,
            Device::Metal,
            Device::Remote,
        ]));
        expected.extend(json(&[
            TtsErrorKind::Unavailable,
            TtsErrorKind::InvalidConfig,
            TtsErrorKind::LicenseBlocked,
            TtsErrorKind::PrivacyBlocked,
            TtsErrorKind::Timeout,
            TtsErrorKind::Cancelled,
            TtsErrorKind::BadRequest,
            TtsErrorKind::Backend,
            TtsErrorKind::Network,
        ]));
        for value in expected {
            assert!(
                ts.contains(&value),
                "serde-Wert {value} fehlt im TypeScript:
{ts}"
            );
        }
    }

    #[test]
    fn kennungen_und_sprachfilter() {
        for ok in ["fish", "piper", "openai-speech", "xtts_v2", "0abc"] {
            assert!(is_valid_provider_id(ok), "{ok}");
        }
        for bad in ["", "Fish", "-x", "_x", "a b", "ä", "a/b"] {
            assert!(!is_valid_provider_id(bad), "{bad:?}");
        }
        assert!(language_matches("de", None));
        assert!(language_matches("de", Some("de_DE")));
        assert!(language_matches("de_DE", Some("DE-at")));
        assert!(!language_matches("en", Some("de")));
        assert!(language_matches("und", Some("fr")), "und passt immer");
        assert!(
            !language_matches("de", Some("")),
            "leerer Filter passt nicht zu de"
        );
    }

    #[test]
    fn optionen_haben_vernuenftige_vorgaben() {
        let o = SynthesisOptions::default();
        assert_eq!(o.language, "de");
        assert_eq!(o.format, AudioEncoding::Wav);
        assert!(o.timeout >= Duration::from_secs(1));
    }

    #[tokio::test]
    async fn cancel_token_weckt_wartende_und_ist_idempotent() {
        let t = CancelToken::new("r1");
        assert_eq!(t.request_id(), "r1");
        assert!(!t.is_cancelled());
        let t2 = t.clone();
        let waiter = tokio::spawn(async move { t2.cancelled().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        t.cancel();
        t.cancel();
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("Wartender wurde nicht geweckt")
            .unwrap();
        assert!(t.is_cancelled());
        // Schon abgebrochen: sofort fertig, ohne Wartezeit.
        tokio::time::timeout(Duration::from_millis(200), t.cancelled())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn guarded_liefert_cancelled_und_timeout_und_startet_bei_abbruch_nicht() {
        // Frist.
        let t = CancelToken::detached();
        let r: Result<(), _> = guarded(&t, Duration::from_millis(30), "x", async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(())
        })
        .await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Timeout);

        // Abbruch mitten im Warten.
        let t = CancelToken::detached();
        let t2 = t.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            t2.cancel();
        });
        let r: Result<(), _> = guarded(&t, Duration::from_secs(30), "x", async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(())
        })
        .await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Cancelled);

        // Vorab abgebrochen: die Arbeit beginnt nie.
        let started = Arc::new(AtomicBool::new(false));
        let s2 = started.clone();
        let r: Result<(), _> = guarded(&t, Duration::from_secs(30), "x", async move {
            s2.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Cancelled);
        assert!(!started.load(Ordering::SeqCst));

        // Erfolg und Fehler des Inneren kommen unverändert durch.
        let t = CancelToken::detached();
        assert_eq!(
            guarded(&t, Duration::from_secs(5), "x", async { Ok(7) })
                .await
                .unwrap(),
            7
        );
        let e = guarded::<()>(&t, Duration::from_secs(5), "x", async {
            Err(TtsError::backend("innen"))
        })
        .await
        .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);
    }

    fn chunk(seq: u32) -> AudioChunk {
        AudioChunk {
            request_id: "r".into(),
            seq,
            format: AudioFormat {
                sample_rate: 24_000,
                channels: 1,
                encoding: AudioEncoding::Pcm16Le,
            },
            data: vec![0; 4],
            is_last: false,
            elapsed_ms: 0,
        }
    }

    #[tokio::test]
    async fn send_chunk_meldet_geschlossenen_empfaenger_als_cancelled() {
        let (tx, rx) = mpsc::channel(2);
        drop(rx);
        let e = send_chunk(&tx, chunk(0), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Cancelled);
    }

    #[tokio::test]
    async fn send_chunk_zaehlt_rueckstau_und_bricht_bei_abbruch_ab() {
        let (tx, mut rx) = mpsc::channel(1);
        let cancel = CancelToken::detached();
        send_chunk(&tx, chunk(0), &cancel).await.unwrap();
        // Kanal voll: der zweite Chunk wartet. Wir lesen verzögert -> kommt an.
        let before = sink_stall_count();
        let tx2 = tx.clone();
        let c2 = cancel.clone();
        let sender = tokio::spawn(async move { send_chunk(&tx2, chunk(1), &c2).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(sink_stall_count() > before, "Rückstau wurde nicht gezählt");
        assert_eq!(rx.recv().await.unwrap().seq, 0);
        sender.await.unwrap().unwrap();
        assert_eq!(rx.recv().await.unwrap().seq, 1);

        // Wieder voll, niemand liest: Abbruch löst den wartenden Sender.
        send_chunk(&tx, chunk(2), &cancel).await.unwrap();
        let tx3 = tx.clone();
        let c3 = cancel.clone();
        let blocked = tokio::spawn(async move { send_chunk(&tx3, chunk(3), &c3).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel.cancel();
        let e = tokio::time::timeout(Duration::from_secs(2), blocked)
            .await
            .expect("Sender hängt trotz Abbruch")
            .unwrap()
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Cancelled);
    }
}
