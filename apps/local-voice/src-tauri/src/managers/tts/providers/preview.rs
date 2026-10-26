//! Probestimme: ein Anbieter spricht einen kurzen Text, das Audio wird
//! abgespielt, SOBALD die ersten Teile eintreffen (Goal tts-provider, Paket P3a).
//!
//! ## Ablauf
//!
//! `start` prüft zuerst alles Synchrone (Text, Anbieter bekannt, Guardrails —
//! [`check_use`], wie bei jeder Nutzung) und liefert dann die `request_id`;
//! alles Weitere läuft in einer Aufgabe:
//!
//! ```text
//! Anbieter ──stream_synthesize──▶ Chunks ──▶ PcmSegment ──▶ Player::play_stream
//!   (begrenzter Kanal, Retry nur            (f32)           (ein Ausgabestrom,
//!    vor dem ersten Chunk)                                   Stücke werden angehängt)
//! ```
//!
//! Ereignisse ([`TtsProviderPreview`], Name `tts-provider-preview`):
//! `started` → `first_audio` (mit `ttfa_ms`: Zeit bis zum ersten Chunk) →
//! `done` | `cancelled` | `error` (mit `error_kind` und `message`). Genau EIN
//! Abschlussereignis je Auftrag. Das Ereignis `started` kann vor der Antwort
//! des Befehls bei der Oberfläche ankommen — sie ordnet Ereignisse über die
//! `request_id` zu und muss ein erstes unbekanntes Ereignis puffern.
//!
//! ## Streaming-Verhalten (ehrlich)
//!
//! Echtes Streaming gibt es nur bei Anbietern mit `StreamingMode::Chunk`
//! (mehrere Teile vor dem Ende der Synthese). Ein `Sentence`-Anbieter liefert
//! EIN Teil nach der fertigen Berechnung; dann beginnt die Wiedergabe eben erst
//! dann. Mit dem [`RodioPlayer`](crate::managers::tts::player::RodioPlayer)
//! läuft alles über EINEN Ausgabestrom (kein Spalt zwischen den Teilen);
//! ein Player ohne eigene Stream-Fähigkeit spielt gesammelt am Ende.
//!
//! ## Widerruf der Freigabe
//!
//! Der Dienst hält den gemeinsamen Stand der Freigaben ([`LiveApprovals`]) und
//! prüft ihn nicht nur beim Start, sondern auch je eintreffendem Teil. Nach einer
//! Änderung der Freigaben ruft der Manager [`PreviewService::enforce`]: ist die
//! laufende Probestimme jetzt gesperrt, endet sie sofort (Synthese und
//! Wiedergabe) mit `error` und `license_blocked`/`privacy_blocked`.
//!
//! ## Abbruch
//!
//! `cancel` setzt das Abbruch-Flag der Wiedergabe (Player stoppt binnen 50 ms,
//! verwirft den Rest) UND die Marke der Synthese (Subprozess wird beendet,
//! HTTP-Strom abgebrochen). Nach `cancel` wird kein weiteres Audio an den
//! Player gereicht. Ein neuer Start bricht einen laufenden ab („letzter
//! gewinnt“, wie beim Vorlesen).
//!
//! ## Grenzen (gegen volle Speicher/Endlosläufe)
//!
//! Text höchstens [`MAX_PREVIEW_CHARS`] Zeichen; Audio höchstens
//! [`MAX_PREVIEW_SAMPLES`] Abtastwerte (≈ 5,5 Minuten bei 24 kHz mono) —
//! darüber endet der Auftrag mit `bad_request`; Gesamtfrist
//! [`PREVIEW_TIMEOUT`]; Stream-Kanal begrenzt (Rückstau statt Wachstum).
//! Der Puffer zum Player ist ungebunden, aber durch die Sample-Grenze gedeckelt.
//!
//! ## Vorlesen und Probestimme
//!
//! Die Probestimme ändert nie den Zustand des Vorlesens (Phase, Generation,
//! Abbruch-Flag, Cache). Gegen doppeltes Audio entscheidet der Aufrufer
//! (`TtsManager`): läuft das Vorlesen, wird die Probestimme ABGELEHNT; startet
//! das Vorlesen, wird eine laufende Probestimme abgebrochen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;
use tokio::sync::mpsc;

use super::contract::{
    AudioChunk, AudioEncoding, SynthesisOptions, TtsError, TtsErrorKind, TtsProvider,
};
use super::pipeline::next_request_id;
use super::policy::LiveApprovals;
use super::registry::{stream_with_retry, ProviderRegistry, RequestGuard, RetryPolicy};
use super::wav::parse_wav;
use crate::managers::tts::lang::detect_language;
use crate::managers::tts::player::{PcmSegment, PlaybackControls, Player};

/// Längster Probetext in Zeichen.
pub const MAX_PREVIEW_CHARS: usize = 1000;
/// Höchste Zahl Abtastwerte (über alle Kanäle) je Probestimme.
pub const MAX_PREVIEW_SAMPLES: u64 = 8_000_000;
/// Gesamtfrist einer Probestimme.
pub const PREVIEW_TIMEOUT: Duration = Duration::from_secs(120);
/// Kapazität des Stream-Kanals zwischen Anbieter und Wiedergabe.
const CHUNK_QUEUE: usize = 8;

/// Phase im Ereignis [`TtsProviderPreview`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PreviewPhase {
    Started,
    FirstAudio,
    Done,
    Cancelled,
    Error,
}

/// Ereignis `tts-provider-preview` (siehe Moduldoku).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Event)]
pub struct TtsProviderPreview {
    pub request_id: String,
    pub phase: PreviewPhase,
    /// Nur bei `first_audio`: Millisekunden vom Start bis zum ersten Teil.
    pub ttfa_ms: Option<u64>,
    /// Nur bei `error`.
    pub error_kind: Option<TtsErrorKind>,
    /// Nur bei `error`: Text für den Nutzer.
    pub message: Option<String>,
}

pub type EmitFn = Arc<dyn Fn(TtsProviderPreview) + Send + Sync>;

/// Auftrag für [`PreviewService::start`]. Die Guardrails prüft der Dienst
/// selbst: Gegenstand aus der Instanz ([`TtsProvider::use_subject`]), Freigaben
/// aus dem gemeinsamen, aktuellen Stand ([`LiveApprovals`]).
pub struct PreviewRequest {
    pub provider_id: String,
    pub text: String,
    pub voice: Option<String>,
    /// Ausgabegerät (`None` = Standard).
    pub device: Option<String>,
}

/// Prüfung der Guardrails für EINE laufende Probestimme: Instanz und
/// Gegenstand sind die des Starts, die Freigaben immer die aktuellen.
type GuardFn = Arc<dyn Fn() -> Result<(), TtsError> + Send + Sync>;

struct Active {
    request_id: String,
    playback_cancel: Arc<AtomicBool>,
    guard: GuardFn,
    /// Gesetzt, wenn ein Widerruf die Probestimme beendet hat (der Abschluss
    /// meldet dann `error` mit dem Sperrgrund statt `cancelled`).
    revoked: Arc<Mutex<Option<TtsError>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Spielt Probestimmen von Registry-Anbietern (siehe Moduldoku).
pub struct PreviewService {
    registry: Arc<ProviderRegistry>,
    player: Arc<dyn Player>,
    controls: Arc<PlaybackControls>,
    emit: EmitFn,
    approvals: LiveApprovals,
    active: Arc<Mutex<Option<Active>>>,
    max_samples: u64,
}

impl PreviewService {
    pub fn new(
        registry: Arc<ProviderRegistry>,
        player: Arc<dyn Player>,
        controls: Arc<PlaybackControls>,
        emit: EmitFn,
        approvals: LiveApprovals,
    ) -> Self {
        Self {
            registry,
            player,
            controls,
            emit,
            approvals,
            active: Arc::new(Mutex::new(None)),
            max_samples: MAX_PREVIEW_SAMPLES,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_max_samples(mut self, n: u64) -> Self {
        self.max_samples = n;
        self
    }

    /// Läuft gerade eine Probestimme?
    pub fn is_active(&self) -> bool {
        lock(&self.active).is_some()
    }

    /// Probestimme starten; Rückgabe ist die `request_id`. Fehler vor dem
    /// Start (leerer/zu langer Text, unbekannter Anbieter, Guardrails)
    /// kommen synchron als `Err`, ohne Ereignis und ohne Aufruf des Anbieters.
    pub fn start(&self, req: PreviewRequest) -> Result<String, TtsError> {
        let text = req.text.trim().to_string();
        if text.is_empty() {
            return Err(TtsError::bad_request("Leerer Probetext"));
        }
        if text.chars().count() > MAX_PREVIEW_CHARS {
            return Err(TtsError::bad_request(format!(
                "Der Probetext ist zu lang (höchstens {MAX_PREVIEW_CHARS} Zeichen)"
            )));
        }
        let provider = self.registry.get(&req.provider_id).ok_or_else(|| {
            TtsError::bad_request(format!("Unbekannter Anbieter '{}'", req.provider_id))
        })?;
        // Die Guardrails: wie bei jeder Nutzung, vor dem ersten Aufruf. Instanz,
        // Beschreibung und Gegenstand kommen aus EINEM Zugriff; die Freigaben
        // sind die aktuellen und werden während der Probe weiter geprüft.
        let descriptor = provider.descriptor();
        let subject = provider.use_subject();
        self.approvals.check(&descriptor, &subject)?;
        let guard: GuardFn = {
            let approvals = self.approvals.clone();
            Arc::new(move || approvals.check(&descriptor, &subject))
        };

        let opts = SynthesisOptions {
            language: detect_language(&text).unwrap_or("de").to_string(),
            voice: req.voice.clone().filter(|v| !v.trim().is_empty()),
            // Rohes PCM: kürzester Weg zur Wiedergabe, ohne WAV-Kopf je Teil.
            format: AudioEncoding::Pcm16Le,
            speed: None,
            seed: None,
            timeout: PREVIEW_TIMEOUT,
        };
        let playback_cancel = Arc::new(AtomicBool::new(false));
        let revoked: Arc<Mutex<Option<TtsError>>> = Arc::new(Mutex::new(None));

        // „Letzter gewinnt“: Vorgänger abbrechen und den neuen eintragen unter
        // EINER Sperre, damit zwei gleichzeitige Starts nie beide aktiv bleiben.
        let (request_id, request_guard) = {
            let mut slot = lock(&self.active);
            if let Some(previous) = slot.take() {
                previous.playback_cancel.store(true, Ordering::Release);
                self.registry.cancel(&previous.request_id);
            }
            let request_id = next_request_id("preview");
            let request_guard = self.registry.begin_request(&request_id)?;
            *slot = Some(Active {
                request_id: request_id.clone(),
                playback_cancel: playback_cancel.clone(),
                guard: guard.clone(),
                revoked: revoked.clone(),
            });
            (request_id, request_guard)
        };

        let run = Run {
            provider,
            request_id: request_id.clone(),
            text,
            opts,
            guard: request_guard,
            use_guard: guard,
            revoked,
            playback_cancel,
            device: req.device,
            controls: self.controls.clone(),
            player: self.player.clone(),
            emit: self.emit.clone(),
            active: self.active.clone(),
            max_samples: self.max_samples,
        };
        tauri::async_runtime::spawn(run.execute());
        Ok(request_id)
    }

    /// Eine Probestimme abbrechen (Synthese UND Wiedergabe). `false`, wenn
    /// sie nicht (mehr) läuft.
    pub fn cancel(&self, request_id: &str) -> bool {
        let hit = {
            let slot = lock(&self.active);
            match slot.as_ref().filter(|a| a.request_id == request_id) {
                Some(active) => {
                    active.playback_cancel.store(true, Ordering::Release);
                    true
                }
                None => false,
            }
        };
        self.registry.cancel(request_id) || hit
    }

    /// Eine laufende Probestimme abbrechen (z. B. weil das Vorlesen beginnt).
    pub fn cancel_active(&self) -> bool {
        let id = lock(&self.active).as_ref().map(|a| a.request_id.clone());
        id.is_some_and(|id| self.cancel(&id))
    }

    /// Nach einer Änderung der Freigaben: ist die laufende Probestimme jetzt
    /// gesperrt, wird sie sofort beendet (Synthese UND Wiedergabe) und meldet
    /// `error` mit dem Sperrgrund (`license_blocked`/`privacy_blocked`). `true`:
    /// sie wurde beendet. Ohne laufende Probestimme oder bei weiterhin erlaubter
    /// Nutzung geschieht nichts.
    pub fn enforce(&self) -> bool {
        let blocked = {
            let slot = lock(&self.active);
            let Some(active) = slot.as_ref() else {
                return false;
            };
            match (active.guard)() {
                Ok(()) => return false,
                Err(e) => {
                    // Der Grund steht VOR dem Abbruch-Flag da, damit der Abschluss
                    // ihn sieht.
                    *lock(&active.revoked) = Some(e);
                    (active.request_id.clone(), active.playback_cancel.clone())
                }
            }
        };
        blocked.1.store(true, Ordering::Release);
        self.registry.cancel(&blocked.0);
        true
    }
}

/// Alles, was die Aufgabe einer Probestimme braucht.
struct Run {
    provider: Arc<dyn TtsProvider>,
    request_id: String,
    text: String,
    opts: SynthesisOptions,
    guard: RequestGuard,
    /// Guardrails (aktuelle Freigaben), je Teil des Streams neu geprüft.
    use_guard: GuardFn,
    revoked: Arc<Mutex<Option<TtsError>>>,
    playback_cancel: Arc<AtomicBool>,
    device: Option<String>,
    controls: Arc<PlaybackControls>,
    player: Arc<dyn Player>,
    emit: EmitFn,
    active: Arc<Mutex<Option<Active>>>,
    max_samples: u64,
}

/// Ein Teil Audio des Anbieters als abspielbares Stück. Prüft Rate, Kanäle und
/// ganze Frames; der Anbietervertrag verspricht sie, die Wiedergabe verlässt
/// sich nicht darauf.
fn to_segment(chunk: &AudioChunk) -> Result<PcmSegment, TtsError> {
    let (rate, channels, encoding, data): (u32, u16, AudioEncoding, &[u8]) =
        match chunk.format.encoding {
            AudioEncoding::Wav => {
                let info = parse_wav(&chunk.data)?;
                let encoding = info.encoding().ok_or_else(|| {
                    TtsError::backend("Teil-Audio hat ein nicht abspielbares WAV-Format")
                })?;
                (
                    info.sample_rate,
                    info.channels,
                    encoding,
                    &chunk.data[info.data_offset..info.data_offset + info.data_len],
                )
            }
            encoding => (
                chunk.format.sample_rate,
                chunk.format.channels,
                encoding,
                &chunk.data[..],
            ),
        };
    if rate == 0 || channels == 0 {
        return Err(TtsError::backend(
            "Teil-Audio ohne Abtastrate oder Kanalzahl",
        ));
    }
    let (width, to_f32): (usize, fn(&[u8]) -> f32) = match encoding {
        AudioEncoding::Pcm16Le => (2, |b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0),
        AudioEncoding::F32Le => (4, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        AudioEncoding::Wav => unreachable!("Wav wurde oben aufgelöst"),
    };
    let frame = width * usize::from(channels);
    if data.len() % frame != 0 {
        return Err(TtsError::backend(
            "Teil-Audio endet mitten in einem Frame (abgeschnittener Strom?)",
        ));
    }
    Ok(PcmSegment {
        sample_rate: rate,
        channels,
        samples: data.chunks_exact(width).map(to_f32).collect(),
    })
}

/// Wie die Aufgabe endete.
enum Outcome {
    Done,
    Cancelled,
    Error(TtsError),
}

impl Run {
    fn emit(&self, phase: PreviewPhase, ttfa_ms: Option<u64>, error: Option<&TtsError>) {
        (self.emit)(TtsProviderPreview {
            request_id: self.request_id.clone(),
            phase,
            ttfa_ms,
            error_kind: error.map(|e| e.kind),
            message: error.map(|e| e.message.clone()),
        });
    }

    async fn execute(self) {
        self.emit(PreviewPhase::Started, None, None);
        let started = Instant::now();
        let token = self.guard.token().clone();
        let (chunk_tx, mut chunk_rx) = mpsc::channel::<AudioChunk>(CHUNK_QUEUE);
        let (pcm_tx, pcm_rx) = std::sync::mpsc::channel::<PcmSegment>();

        let play = {
            let player = self.player.clone();
            let controls = self.controls.clone();
            let cancel = self.playback_cancel.clone();
            let device = self.device.clone();
            tauri::async_runtime::spawn_blocking(move || {
                player.play_stream(pcm_rx, device, controls, cancel)
            })
        };

        let policy = RetryPolicy::default();
        let produce = stream_with_retry(
            &*self.provider,
            &self.text,
            &self.opts,
            &token,
            chunk_tx,
            &policy,
        );
        // `async move`: `pcm_tx` muss am Ende dieses Blocks fallen, damit die
        // Wiedergabe den Rest spielt und endet. `this` ist nur eine Referenz.
        let this = &self;
        let consume_token = token.clone();
        let consume = async move {
            let mut first: Option<u64> = None;
            let mut samples_total: u64 = 0;
            while let Some(chunk) = chunk_rx.recv().await {
                // Nach einem Abbruch geht kein weiteres Audio an den Player.
                if this.playback_cancel.load(Ordering::Acquire) {
                    break;
                }
                // Widerruf der Freigabe mitten im Strom: nichts mehr abspielen.
                if let Err(blocked) = (this.use_guard)() {
                    consume_token.cancel();
                    return Err(blocked);
                }
                let segment = match to_segment(&chunk) {
                    Ok(s) => s,
                    Err(e) => {
                        consume_token.cancel();
                        return Err(e);
                    }
                };
                if first.is_none() {
                    let ttfa = started.elapsed().as_millis() as u64;
                    first = Some(ttfa);
                    this.emit(PreviewPhase::FirstAudio, Some(ttfa), None);
                }
                samples_total += segment.samples.len() as u64;
                if samples_total > this.max_samples {
                    consume_token.cancel();
                    return Err(TtsError::bad_request(
                        "Die Probestimme ist zu lang und wurde beendet",
                    ));
                }
                if pcm_tx.send(segment).is_err() {
                    // Die Wiedergabe ist weg (z. B. kein Ausgabegerät); den
                    // Grund liefert ihr Ergebnis.
                    consume_token.cancel();
                    return Err(TtsError::unavailable("Die Wiedergabe wurde beendet"));
                }
            }
            Ok(())
        };
        let (produced, consumed) = futures_util::future::join(produce, consume).await;

        // Fehler: sofort still werden, bevor wir auf das Ende der Wiedergabe warten.
        let failed = consumed.is_err()
            || produced
                .as_ref()
                .is_err_and(|e| e.kind != TtsErrorKind::Cancelled);
        if failed {
            self.playback_cancel.store(true, Ordering::Release);
        }
        let played = play.await;
        // Ein Abbruch von außen setzt das Flag (cancel, letzter gewinnt).
        let cancelled_outside = !failed && self.playback_cancel.load(Ordering::Acquire);

        let play_error = match played {
            Ok(Ok(())) => None,
            Ok(Err(msg)) => Some(msg),
            Err(join) => Some(join.to_string()),
        };
        // Ein Widerruf (Befehl der Oberfläche) hat Vorrang vor „abgebrochen“:
        // der Nutzer soll erfahren, WARUM es still wurde.
        let revoked = lock(&self.revoked).take();
        let outcome = if let Some(blocked) = revoked {
            Outcome::Error(blocked)
        } else if cancelled_outside {
            Outcome::Cancelled
        } else if let Some(msg) = play_error {
            Outcome::Error(TtsError::unavailable(format!(
                "Wiedergabe nicht möglich: {msg}"
            )))
        } else if let Err(e) = consumed {
            Outcome::Error(e)
        } else {
            match produced {
                Ok(()) => Outcome::Done,
                Err(e) if e.kind == TtsErrorKind::Cancelled => Outcome::Cancelled,
                Err(e) => Outcome::Error(e),
            }
        };

        // Austragen (nur den eigenen Eintrag), dann EIN Abschlussereignis.
        {
            let mut slot = lock(&self.active);
            if slot
                .as_ref()
                .is_some_and(|a| a.request_id == self.request_id)
            {
                *slot = None;
            }
        }
        match &outcome {
            Outcome::Done => self.emit(PreviewPhase::Done, None, None),
            Outcome::Cancelled => self.emit(PreviewPhase::Cancelled, None, None),
            Outcome::Error(e) => self.emit(PreviewPhase::Error, None, Some(e)),
        }
        // `guard` (Auftragstabelle der Registry) fällt mit `self`.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::contract::{StreamingMode, TtsProvider};
    use crate::managers::tts::providers::mock::{MockFailure, MockProvider, MOCK_SAMPLE_RATE};
    use crate::managers::tts::providers::policy::TtsProvidersSettings;

    /// Player, der jedes Stück samt Ankunftszeit festhält (und wie der echte
    /// auf `cancelled` hört).
    struct StreamRecorder {
        segments: Mutex<Vec<(Instant, PcmSegment)>>,
        fail: Option<String>,
        started: AtomicBool,
    }

    impl StreamRecorder {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                segments: Mutex::new(Vec::new()),
                fail: None,
                started: AtomicBool::new(false),
            })
        }

        fn failing(msg: &str) -> Arc<Self> {
            Arc::new(Self {
                segments: Mutex::new(Vec::new()),
                fail: Some(msg.to_string()),
                started: AtomicBool::new(false),
            })
        }

        fn count(&self) -> usize {
            lock(&self.segments).len()
        }

        fn samples(&self) -> usize {
            lock(&self.segments)
                .iter()
                .map(|(_, s)| s.samples.len())
                .sum()
        }
    }

    impl Player for StreamRecorder {
        fn play(
            &self,
            _wav: Vec<u8>,
            _device: Option<String>,
            _gain: f32,
            _controls: Arc<PlaybackControls>,
            _cancelled: Arc<AtomicBool>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn play_stream(
            &self,
            segments: std::sync::mpsc::Receiver<PcmSegment>,
            _device: Option<String>,
            _controls: Arc<PlaybackControls>,
            cancelled: Arc<AtomicBool>,
        ) -> Result<(), String> {
            use std::sync::mpsc::RecvTimeoutError;
            self.started.store(true, Ordering::SeqCst);
            if let Some(msg) = &self.fail {
                return Err(msg.clone());
            }
            loop {
                if cancelled.load(Ordering::Acquire) {
                    return Ok(());
                }
                match segments.recv_timeout(Duration::from_millis(10)) {
                    Ok(seg) => lock(&self.segments).push((Instant::now(), seg)),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return Ok(()),
                }
            }
        }
    }

    type Events = Arc<Mutex<Vec<TtsProviderPreview>>>;

    /// Dienst mit freigeschaltetem Anbieter `zzz`.
    fn service(
        mock: Arc<MockProvider>,
        player: Arc<StreamRecorder>,
    ) -> (PreviewService, Events, Arc<ProviderRegistry>) {
        let (svc, events, registry, _) = service_live(mock, player);
        (svc, events, registry)
    }

    /// Wie [`service`], mit dem Zugriff auf den gemeinsamen Stand der Freigaben.
    fn service_live(
        mock: Arc<MockProvider>,
        player: Arc<StreamRecorder>,
    ) -> (PreviewService, Events, Arc<ProviderRegistry>, LiveApprovals) {
        let registry = Arc::new(ProviderRegistry::new());
        registry.register(mock).unwrap();
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let emit: EmitFn = Arc::new(move |e| lock(&sink).push(e));
        let mut enabled = TtsProvidersSettings::default();
        enabled.set_enabled("zzz", true);
        let approvals = LiveApprovals::new(enabled);
        let svc = PreviewService::new(
            registry.clone(),
            player,
            Arc::new(PlaybackControls::default()),
            emit,
            approvals.clone(),
        );
        (svc, events, registry, approvals)
    }

    fn request(id: &str, text: &str) -> PreviewRequest {
        PreviewRequest {
            provider_id: id.to_string(),
            text: text.to_string(),
            voice: None,
            device: None,
        }
    }

    fn phases(events: &Events, id: &str) -> Vec<PreviewPhase> {
        lock(events)
            .iter()
            .filter(|e| e.request_id == id)
            .map(|e| e.phase)
            .collect()
    }

    async fn wait_for<F: Fn() -> bool>(what: &str, secs: u64, cond: F) {
        let t0 = Instant::now();
        while !cond() {
            assert!(
                t0.elapsed() < Duration::from_secs(secs),
                "Zeitüberschreitung beim Warten auf: {what}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn finished(events: &Events, id: &str) {
        wait_for("Abschlussereignis", 10, || {
            phases(events, id).last().is_some_and(|p| {
                matches!(
                    p,
                    PreviewPhase::Done | PreviewPhase::Cancelled | PreviewPhase::Error
                )
            })
        })
        .await;
    }

    const TEXT: &str = "Hallo, das ist eine kurze Probe der Stimme.";

    #[tokio::test]
    async fn eine_probestimme_laeuft_von_started_bis_done() {
        let mock = Arc::new(MockProvider::new().with_id("zzz"));
        let player = StreamRecorder::new();
        let (svc, events, reg) = service(mock.clone(), player.clone());
        let id = svc.start(request("zzz", TEXT)).unwrap();
        finished(&events, &id).await;

        assert_eq!(
            phases(&events, &id),
            [
                PreviewPhase::Started,
                PreviewPhase::FirstAudio,
                PreviewPhase::Done
            ]
        );
        let ev = lock(&events);
        let first = ev
            .iter()
            .find(|e| e.phase == PreviewPhase::FirstAudio)
            .unwrap();
        assert!(first.ttfa_ms.is_some());
        assert!(ev.iter().all(|e| e.error_kind.is_none()));
        // Das ganze Signal kam an, mit Rate und Kanalzahl des Anbieters.
        let expected = mock.expected_pcm(TEXT).len() / 2;
        assert_eq!(player.samples(), expected);
        assert_eq!(lock(&player.segments)[0].1.sample_rate, MOCK_SAMPLE_RATE);
        assert_eq!(lock(&player.segments)[0].1.channels, 1);
        drop(ev);
        wait_for("Auftrag ausgetragen", 5, || reg.active_requests() == 0).await;
        assert!(!svc.is_active());
    }

    #[tokio::test]
    async fn chunk_anbieter_werden_waehrend_der_synthese_abgespielt() {
        // 4 Teile mit 120 ms Pause: der erste Ton darf nicht auf das Ende warten.
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .chunked(4, Duration::from_millis(120)),
        );
        assert_eq!(mock.descriptor().caps.streaming, StreamingMode::Chunk);
        let player = StreamRecorder::new();
        let (svc, events, _reg) = service(mock, player.clone());
        let id = svc.start(request("zzz", TEXT)).unwrap();
        finished(&events, &id).await;

        let seg = lock(&player.segments);
        assert_eq!(seg.len(), 4, "alle vier Teile einzeln");
        let spread = seg[3].0.duration_since(seg[0].0);
        assert!(
            spread >= Duration::from_millis(250),
            "Teile kamen nacheinander an, nicht gesammelt: {spread:?}"
        );
        let ev = lock(&events);
        let ttfa = ev
            .iter()
            .find(|e| e.phase == PreviewPhase::FirstAudio)
            .unwrap()
            .ttfa_ms
            .unwrap();
        assert!(ttfa < 300, "erstes Audio lange vor dem Ende: {ttfa} ms");
    }

    #[tokio::test]
    async fn abbruch_mitten_im_strom_meldet_cancelled_und_liefert_kein_audio_mehr() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .chunked(40, Duration::from_millis(40)),
        );
        let player = StreamRecorder::new();
        let (svc, events, reg) = service(mock.clone(), player.clone());
        let id = svc.start(request("zzz", TEXT)).unwrap();
        wait_for("erstes Audio beim Player", 5, || player.count() >= 1).await;

        assert!(
            svc.cancel(&id),
            "ein laufender Auftrag lässt sich abbrechen"
        );
        finished(&events, &id).await;

        assert_eq!(
            phases(&events, &id).last(),
            Some(&PreviewPhase::Cancelled),
            "{:?}",
            phases(&events, &id)
        );
        assert!(!phases(&events, &id).contains(&PreviewPhase::Done));
        assert!(!phases(&events, &id).contains(&PreviewPhase::Error));
        let after = player.count();
        assert!(after < 40, "nicht alle Teile: {after}");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            player.count(),
            after,
            "nach dem Abbruch kommt kein Audio mehr an"
        );
        assert_eq!(
            phases(&events, &id)
                .iter()
                .filter(|p| **p == PreviewPhase::Cancelled)
                .count(),
            1,
            "genau ein Abschlussereignis"
        );
        wait_for("Anbieter beendet", 5, || mock.in_flight() == 0).await;
        wait_for("Auftrag ausgetragen", 5, || reg.active_requests() == 0).await;
        assert!(
            !svc.cancel(&id),
            "ein beendeter Auftrag lässt sich nicht noch einmal abbrechen"
        );
    }

    #[tokio::test]
    async fn abbruch_waehrend_der_latenz_vor_dem_ersten_ton() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .latency(Duration::from_secs(30)),
        );
        let player = StreamRecorder::new();
        let (svc, events, _reg) = service(mock.clone(), player.clone());
        let id = svc.start(request("zzz", TEXT)).unwrap();
        wait_for("Anbieter gestartet", 5, || mock.in_flight() == 1).await;
        let t0 = Instant::now();
        assert!(svc.cancel(&id));
        finished(&events, &id).await;
        assert!(t0.elapsed() < Duration::from_secs(3));
        assert_eq!(
            phases(&events, &id),
            [PreviewPhase::Started, PreviewPhase::Cancelled]
        );
        assert_eq!(player.count(), 0);
        wait_for("Anbieter beendet", 5, || mock.in_flight() == 0).await;
    }

    #[tokio::test]
    async fn die_guardrails_gelten_auch_fuer_die_probestimme() {
        let mock = Arc::new(MockProvider::new().with_id("zzz"));
        let (svc, events, reg, approvals) = service_live(mock.clone(), StreamRecorder::new());
        // Nicht freigeschaltet: kein Auftrag, kein Ereignis, kein Anbieteraufruf.
        approvals.set(TtsProvidersSettings::default());
        let e = svc.start(request("zzz", TEXT)).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::LicenseBlocked);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(lock(&events).is_empty());
        assert_eq!(mock.attempts(), 0);
        assert_eq!(reg.active_requests(), 0);

        // Entfernte Adresse ohne Freigabe: der Gegenstand kommt aus der Instanz.
        let remote = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .with_subject("m", Some("https://tts.example.org/v1")),
        );
        let (svc, events, reg, _) = service_live(remote.clone(), StreamRecorder::new());
        let e = svc.start(request("zzz", TEXT)).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::PrivacyBlocked);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(lock(&events).is_empty());
        assert_eq!(remote.attempts(), 0);
        assert_eq!(reg.active_requests(), 0);
    }

    /// Widerruf mitten in der Probestimme (Befund 1): der Befehl der
    /// Oberfläche ruft `enforce`; die Probe endet sofort mit `error` und dem
    /// Sperrgrund, das Anbieter-Future fällt, kein weiteres Audio kommt an.
    #[tokio::test]
    async fn ein_widerruf_beendet_die_laufende_probestimme_mit_dem_sperrgrund() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .chunked(40, Duration::from_millis(40)),
        );
        let player = StreamRecorder::new();
        let (svc, events, reg, approvals) = service_live(mock.clone(), player.clone());
        let id = svc.start(request("zzz", TEXT)).unwrap();
        wait_for("erstes Audio beim Player", 5, || player.count() >= 1).await;

        // Solange nichts widerrufen ist, tut `enforce` nichts.
        assert!(!svc.enforce());
        assert!(svc.is_active());

        approvals.set(TtsProvidersSettings::default());
        assert!(svc.enforce(), "die Probestimme wird beendet");
        finished(&events, &id).await;

        let ev = lock(&events);
        let last = ev.last().unwrap();
        assert_eq!(last.phase, PreviewPhase::Error, "{:?}", phases_of(&ev, &id));
        assert_eq!(last.error_kind, Some(TtsErrorKind::LicenseBlocked));
        assert!(last
            .message
            .as_deref()
            .unwrap()
            .contains("nicht freigeschaltet"));
        assert!(!ev.iter().any(|e| e.phase == PreviewPhase::Done));
        drop(ev);
        let after = player.count();
        assert!(after < 40, "nicht alle Teile: {after}");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(player.count(), after, "danach kommt kein Audio mehr an");
        wait_for("Anbieter beendet", 5, || mock.in_flight() == 0).await;
        wait_for("Auftrag ausgetragen", 5, || reg.active_requests() == 0).await;
        assert!(!svc.is_active());
    }

    /// Auch ohne `enforce` (z. B. weil nur die Datei geändert wurde): das
    /// nächste Teil des Streams prüft die aktuellen Freigaben.
    #[tokio::test]
    async fn die_probestimme_prueft_die_freigaben_je_teil_auch_ohne_enforce() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .chunked(40, Duration::from_millis(40)),
        );
        let player = StreamRecorder::new();
        let (svc, events, _reg, approvals) = service_live(mock.clone(), player.clone());
        let id = svc.start(request("zzz", TEXT)).unwrap();
        wait_for("erstes Audio beim Player", 5, || player.count() >= 1).await;
        approvals.set(TtsProvidersSettings::default());
        finished(&events, &id).await;
        let ev = lock(&events);
        let last = ev.last().unwrap();
        assert_eq!(last.phase, PreviewPhase::Error);
        assert_eq!(last.error_kind, Some(TtsErrorKind::LicenseBlocked));
        drop(ev);
        wait_for("Anbieter beendet", 5, || mock.in_flight() == 0).await;
        assert!(mock.chunks_sent() < 40, "die Synthese lief nicht zu Ende");
    }

    /// Fernzugriff zurückgenommen: dieselbe Wirkung, mit `privacy_blocked`.
    #[tokio::test]
    async fn der_entzug_des_fernzugriffs_beendet_die_probestimme_mit_privacy_blocked() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .chunked(40, Duration::from_millis(40))
                .with_subject("m", Some("http://192.168.1.20:8880/v1")),
        );
        let player = StreamRecorder::new();
        let (svc, events, _reg, approvals) = service_live(mock.clone(), player.clone());
        let mut allowed = TtsProvidersSettings::default();
        allowed.set_enabled("zzz", true);
        allowed.set_remote_allowed("zzz", true);
        approvals.set(allowed.clone());
        let id = svc.start(request("zzz", TEXT)).unwrap();
        wait_for("erstes Audio beim Player", 5, || player.count() >= 1).await;
        allowed.set_remote_allowed("zzz", false);
        approvals.set(allowed);
        assert!(svc.enforce());
        finished(&events, &id).await;
        let ev = lock(&events);
        assert_eq!(
            ev.last().unwrap().error_kind,
            Some(TtsErrorKind::PrivacyBlocked)
        );
    }

    fn phases_of(ev: &[TtsProviderPreview], id: &str) -> Vec<PreviewPhase> {
        ev.iter()
            .filter(|e| e.request_id == id)
            .map(|e| e.phase)
            .collect()
    }

    #[tokio::test]
    async fn eingabefehler_kommen_synchron() {
        let mock = Arc::new(MockProvider::new().with_id("zzz"));
        let (svc, events, _reg) = service(mock.clone(), StreamRecorder::new());
        assert_eq!(
            svc.start(request("zzz", "   ")).unwrap_err().kind,
            TtsErrorKind::BadRequest
        );
        assert_eq!(
            svc.start(request("gibt-es-nicht", TEXT)).unwrap_err().kind,
            TtsErrorKind::BadRequest
        );
        let long = "a".repeat(MAX_PREVIEW_CHARS + 1);
        assert_eq!(
            svc.start(request("zzz", &long)).unwrap_err().kind,
            TtsErrorKind::BadRequest
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(lock(&events).is_empty());
        assert_eq!(mock.attempts(), 0);
    }

    #[tokio::test]
    async fn ein_anbieterfehler_mitten_im_strom_stoppt_die_wiedergabe() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .chunked(6, Duration::from_millis(30))
                .failing(MockFailure {
                    error: TtsError::backend("Server abgestürzt"),
                    at_chunk: 3,
                    for_attempts: None,
                }),
        );
        let player = StreamRecorder::new();
        let (svc, events, reg) = service(mock, player.clone());
        let id = svc.start(request("zzz", TEXT)).unwrap();
        finished(&events, &id).await;
        let ev = lock(&events);
        let last = ev.last().unwrap();
        assert_eq!(last.phase, PreviewPhase::Error);
        assert_eq!(last.error_kind, Some(TtsErrorKind::Backend));
        assert!(last.message.as_deref().unwrap().contains("abgestürzt"));
        drop(ev);
        assert!(player.count() <= 3, "nach dem Fehler kein weiteres Audio");
        wait_for("Auftrag ausgetragen", 5, || reg.active_requests() == 0).await;
    }

    #[tokio::test]
    async fn ohne_ausgabegeraet_meldet_error_und_stoppt_die_synthese() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .chunked(30, Duration::from_millis(30)),
        );
        let player = StreamRecorder::failing("kein Ausgabegerät gefunden");
        let (svc, events, _reg) = service(mock.clone(), player);
        let id = svc.start(request("zzz", TEXT)).unwrap();
        finished(&events, &id).await;
        let ev = lock(&events);
        let last = ev.last().unwrap();
        assert_eq!(last.phase, PreviewPhase::Error);
        assert_eq!(last.error_kind, Some(TtsErrorKind::Unavailable));
        assert!(
            last.message
                .as_deref()
                .unwrap()
                .contains("kein Ausgabegerät"),
            "{:?}",
            last.message
        );
        drop(ev);
        wait_for("Anbieter beendet", 5, || mock.in_flight() == 0).await;
        assert!(mock.chunks_sent() < 30, "die Synthese lief nicht zu Ende");
    }

    #[tokio::test]
    async fn zu_langes_audio_wird_begrenzt() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .chunked(10, Duration::from_millis(10)),
        );
        let player = StreamRecorder::new();
        let (svc, events, _reg) = service(mock, player.clone());
        let svc = svc.with_max_samples(2_000);
        let id = svc.start(request("zzz", TEXT)).unwrap();
        finished(&events, &id).await;
        let ev = lock(&events);
        let last = ev.last().unwrap();
        assert_eq!(last.phase, PreviewPhase::Error);
        assert_eq!(last.error_kind, Some(TtsErrorKind::BadRequest));
        drop(ev);
        assert!(player.samples() <= 2_000, "{}", player.samples());
    }

    #[tokio::test]
    async fn ein_neuer_start_bricht_den_laufenden_ab() {
        let slow = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .chunked(40, Duration::from_millis(50)),
        );
        let player = StreamRecorder::new();
        let (svc, events, reg) = service(slow, player);
        let a = svc.start(request("zzz", TEXT)).unwrap();
        wait_for("A läuft", 5, || phases(&events, &a).len() >= 2).await;
        let b = svc.start(request("zzz", TEXT)).unwrap();
        assert_ne!(a, b);
        finished(&events, &a).await;
        assert_eq!(phases(&events, &a).last(), Some(&PreviewPhase::Cancelled));
        assert!(svc.cancel(&b));
        finished(&events, &b).await;
        wait_for("alles ausgetragen", 5, || reg.active_requests() == 0).await;
        assert!(!svc.is_active());
    }

    #[tokio::test]
    async fn cancel_einer_unbekannten_kennung_ist_harmlos() {
        let mock = Arc::new(MockProvider::new().with_id("zzz"));
        let (svc, _events, _reg) = service(mock, StreamRecorder::new());
        assert!(!svc.cancel("preview-gibt-es-nicht"));
        assert!(!svc.cancel_active());
    }

    #[test]
    fn teile_werden_in_abspielbare_stuecke_umgerechnet() {
        use crate::managers::tts::providers::contract::AudioFormat;
        let chunk = |encoding, data: Vec<u8>, channels| AudioChunk {
            request_id: "r".into(),
            seq: 0,
            format: AudioFormat {
                sample_rate: 24_000,
                channels,
                encoding,
            },
            data,
            is_last: true,
            elapsed_ms: 0,
        };
        let pcm: Vec<u8> = [0i16, 16384, -32768]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let seg = to_segment(&chunk(AudioEncoding::Pcm16Le, pcm.clone(), 1)).unwrap();
        assert_eq!(seg.samples, vec![0.0, 0.5, -1.0]);
        // Zwei Kanäle: ein halber Frame ist abgeschnittenes Audio.
        assert!(to_segment(&chunk(AudioEncoding::Pcm16Le, pcm, 2)).is_err());
        let f32s: Vec<u8> = [0.25f32, -0.5]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let seg = to_segment(&chunk(AudioEncoding::F32Le, f32s, 2)).unwrap();
        assert_eq!(seg.samples, vec![0.25, -0.5]);
        assert_eq!(seg.channels, 2);
        assert!(to_segment(&chunk(AudioEncoding::Pcm16Le, vec![0, 0], 0)).is_err());
        // WAV-Container wird ausgepackt.
        let wav = crate::managers::tts::providers::wav::wav_from_pcm(
            AudioEncoding::Pcm16Le,
            16_000,
            1,
            &8192i16.to_le_bytes(),
        )
        .unwrap();
        let seg = to_segment(&chunk(AudioEncoding::Wav, wav, 1)).unwrap();
        assert_eq!((seg.sample_rate, seg.samples), (16_000, vec![0.25]));
        // Müll als WAV: Fehler, keine Panik.
        assert!(to_segment(&chunk(AudioEncoding::Wav, vec![1, 2, 3], 1)).is_err());
    }

    #[test]
    fn die_standard_stream_wiedergabe_sammelt_und_spielt_ein_wav() {
        // Player ohne eigene Stream-Fähigkeit: Standardumsetzung des Traits.
        struct Plain(Mutex<Vec<usize>>);
        impl Player for Plain {
            fn play(
                &self,
                wav: Vec<u8>,
                _d: Option<String>,
                _g: f32,
                _c: Arc<PlaybackControls>,
                _x: Arc<AtomicBool>,
            ) -> Result<(), String> {
                lock(&self.0).push(wav.len());
                Ok(())
            }
        }
        let player = Plain(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..3 {
            tx.send(PcmSegment {
                sample_rate: 24_000,
                channels: 1,
                samples: vec![0.1; 100],
            })
            .unwrap();
        }
        drop(tx);
        player
            .play_stream(
                rx,
                None,
                Arc::new(PlaybackControls::default()),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        // EIN WAV: 44 Byte Kopf + 300 Abtastwerte à 4 Byte (F32).
        assert_eq!(*lock(&player.0), vec![44 + 300 * 4]);
    }
}
