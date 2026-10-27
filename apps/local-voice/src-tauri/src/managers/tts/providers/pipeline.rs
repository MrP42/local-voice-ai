//! Brücke zwischen der Anbieterschicht und der Vorlese-Pipeline
//! (Goal tts-provider, Paket P3a).
//!
//! [`ProviderEngine`] ist der Inhalt der Variante `EngineImpl::Provider` des
//! Kerns (`managers::tts`): sie nimmt EINEN Satz, ruft nach den Guardrails
//! `synthesize` des Anbieters und liefert die WAV-Bytes — die Satz-Pipeline
//! (Satz N+1 während Satz N spielt), der Satz-Cache, der Abbruch über den
//! Generationszähler und der Player bleiben unverändert.
//!
//! ## Auswahl
//!
//! Die Wahl steht im bestehenden Einstellungsfeld `tts_engine` als
//! `"provider:<id>"`. Ältere App-Fassungen kennen den Wert nicht und fallen
//! (wie bei jedem unbekannten Wert) auf Fish zurück. `provider:fish` und
//! `provider:piper` sind Schreibweisen der eingebauten Engines und laufen
//! über deren unveränderte Pfade (Fish behält den leeren Cache-Tag).
//!
//! ## Guardrails
//!
//! [`ProviderEngine::guard`] ruft [`check_use`] gegen den AKTUELLEN Stand der
//! Freigaben ([`LiveApprovals`]: der Manager und die Befehle der Oberfläche
//! halten ihn gemeinsam, jede Prüfung liest ihn neu — kein beim Auftragsstart
//! eingefrorener Stand), und läuft in `fetch_wav` VOR dem Cache-Lookup: auch
//! bereits synthetisiertes Audio eines inzwischen gesperrten Anbieters wird
//! nicht mehr ausgeliefert. Ein Widerruf (Opt-in aus, Fernzugriff zurück,
//! Zustimmung entfallen) wirkt damit ab dem nächsten Satz, und `synthesize`
//! bricht auch einen gerade laufenden Satz binnen [`REVOKE_POLL`] ab.
//!
//! Der Gegenstand der Prüfung (Modell, Adresse) stammt aus derselben Instanz,
//! die synthetisiert ([`TtsProvider::use_subject`]); die Engine holt Instanz und
//! Gegenstand in einem Zug aus der Registry (kein Auseinanderlaufen, wenn die
//! Registry zwischendurch neu gebaut wird).
//!
//! ## Abbruch
//!
//! `synthesize` hört auf das Abbruch-Flag des laufenden Auftrags und lässt bei
//! gesetztem Flag das Future des Anbieters fallen (Subprozess-Adapter setzen
//! `kill_on_drop`) — der Abbruch wirkt also auch MITTEN im Satz, anders als
//! bei Fish (HTTP-Anfrage läuft zu Ende) und Piper.
//!
//! ## Stimmen
//!
//! Die Engine trägt EINE Stimme (die gewählte bzw. die der Konfiguration). Hat der
//! Anbieter [`ProviderCaps::voice_switching`](super::contract::ProviderCaps) —
//! Phonon, generischer Sprachserver, Ollama —, kann ein Satz zusätzlich eine
//! eigene Stimme mitbringen (Sprecher aus dem Skript, `<Nele>`): `synthesize_voiced`
//! und `cache_tag_for` nehmen sie statt der eigenen, `SynthesisOptions::voice` und
//! der Cache-Tag tragen sie. Welche Stimme ein Satz bekommt, entscheidet der Kern je
//! Satz (`TtsCore::utterance_voice`); die Namen liefert [`super::speakers`].
//!
//! Kein `settings::get_settings(&AppHandle)` hier.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::contract::{
    guarded, AudioEncoding, CancelToken, ProviderDescriptor, StreamingMode, SynthesisOptions,
    TtsError, TtsProvider, VoiceDescriptor,
};
use super::policy::{check_use, LiveApprovals, UseSubject};
use super::registry::{with_retry, DeliveryTracker, ProviderRegistry, RetryPolicy};
use crate::managers::tts::engine::EngineCaps;
use crate::managers::tts::lang::detect_language;

/// Frist je Satz. Ein Satz dauert Sekunden; die Zahl fängt hängende Anbieter ab.
pub const SENTENCE_TIMEOUT: Duration = Duration::from_secs(120);
/// Frist der Bereitschaftsprüfung vor einem Auftrag.
pub const READY_TIMEOUT: Duration = Duration::from_secs(8);
/// Frist der Stimmenliste für die Sprecher. Unter den 10 s der Befehle der Oberfläche
/// (`VOICES_LIMIT`), damit eine Teilliste noch ankommt.
pub const VOICES_TIMEOUT: Duration = Duration::from_secs(8);
/// Wie oft das Abbruch-Flag während der Synthese angesehen wird.
const CANCEL_POLL: Duration = Duration::from_millis(25);
/// Wie oft während eines laufenden Satzes geprüft wird, ob die Nutzung inzwischen
/// gesperrt ist (Widerruf der Freigabe).
const REVOKE_POLL: Duration = Duration::from_millis(200);
/// Sicherheitsnetz über der Auftragsfrist (wie in der Registry).
const SAFETY_GRACE: Duration = Duration::from_secs(2);

/// Meldung, wenn der Abbruch gewirkt hat (der Kern erkennt den Abbruch am Flag).
pub const CANCELLED_MESSAGE: &str = "abgebrochen";

/// Eindeutige Auftragskennung `"<prefix>-<n>"` (prozessweit).
pub fn next_request_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    format!("{prefix}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Fähigkeiten eines Anbieters als Engine-Fähigkeiten.
///
/// * `voice_switching`: die Angabe des Anbieters
///   ([`ProviderCaps::voice_switching`](super::contract::ProviderCaps)): wer je
///   Auftrag eine andere Stimme sprechen kann, bekommt Sprechermarker im Skript
///   (`<Nele>`), die Satz für Satz die Stimme wählen. Die Namen sind die Stimmen
///   seiner Liste, nicht Fish-Referenzstimmen ([`super::speakers`]).
/// * `needs_gpu`: nie — die App kennt die Grafikkartennutzung eines fremden
///   Anbieters nicht, und das Flag steuert, ob die Fußleiste „Server läuft“
///   zeigt und ob Übersetzung die GPU meiden muss (nur bei Fish zutreffend).
/// * `export_formats`: nur WAV (der Anbieter liefert WAV; andere Formate
///   encodiert der Fish-Server).
pub fn engine_caps(desc: &ProviderDescriptor) -> EngineCaps {
    EngineCaps {
        style_tags: desc.caps.style_tags,
        cloning: desc.caps.cloning,
        voice_switching: desc.caps.voice_switching,
        streaming: desc.caps.streaming == StreamingMode::Chunk,
        needs_gpu: false,
        export_formats: &["wav"],
    }
}

/// Engine-Fähigkeiten, solange der Anbieter unbekannt ist: nichts versprechen.
const UNKNOWN_CAPS: EngineCaps = EngineCaps {
    style_tags: false,
    cloning: false,
    voice_switching: false,
    streaming: false,
    needs_gpu: false,
    export_formats: &["wav"],
};

/// Eine ausdrücklich genannte Stimme: getrimmt, leer zählt als keine.
fn clean_voice(voice: Option<&str>) -> Option<&str> {
    voice.map(str::trim).filter(|v| !v.is_empty())
}

async fn wait_flag(flag: &AtomicBool) {
    while !flag.load(Ordering::Acquire) {
        tokio::time::sleep(CANCEL_POLL).await;
    }
}

/// Ein Registry-Anbieter als Engine der Vorlese-Pipeline. `Clone`: der Kern
/// zieht je Auftrag einen Schnappschuss (siehe `EngineImpl`).
#[derive(Clone)]
pub struct ProviderEngine {
    id: String,
    registry: Arc<ProviderRegistry>,
    /// `None` = die Kennung ist nicht (mehr) registriert.
    provider: Option<Arc<dyn TtsProvider>>,
    descriptor: Option<ProviderDescriptor>,
    voice: Option<String>,
    /// Aus DERSELBEN Instanz wie `provider` ([`TtsProvider::use_subject`]),
    /// im selben Zug geholt: Prüfgegenstand und Instanz stammen nie aus
    /// verschiedenen Ständen.
    subject: UseSubject,
    /// Gemeinsamer, aktueller Stand der Freigaben — je Satz neu gelesen.
    approvals: LiveApprovals,
    ready_timeout: Duration,
}

impl ProviderEngine {
    /// Engine für `id` aus der Registry, mit gewählter Stimme und dem
    /// gemeinsamen Stand der Freigaben. Der Gegenstand der Guardrails (Modell,
    /// Adresse) kommt aus der gefundenen Instanz selbst.
    pub fn new(
        registry: Arc<ProviderRegistry>,
        id: &str,
        voice: Option<String>,
        approvals: LiveApprovals,
    ) -> Self {
        // EIN Zugriff auf die Registry: Instanz, Beschreibung und Gegenstand
        // der Prüfung gehören zusammen.
        let provider = registry.get(id);
        let descriptor = provider.as_ref().map(|p| p.descriptor());
        let subject = provider
            .as_ref()
            .map(|p| p.use_subject())
            .unwrap_or_default();
        Self {
            id: id.to_string(),
            registry,
            provider,
            descriptor,
            voice,
            subject,
            approvals,
            ready_timeout: READY_TIMEOUT,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_ready_timeout(mut self, t: Duration) -> Self {
        self.ready_timeout = t;
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Die eigene (gewählte bzw. konfigurierte) Stimme dieser Engine.
    pub fn voice(&self) -> Option<&str> {
        self.voice.as_deref()
    }

    pub fn caps(&self) -> EngineCaps {
        self.descriptor
            .as_ref()
            .map(engine_caps)
            .unwrap_or(UNKNOWN_CAPS)
    }

    /// Anteil am Cache-Schlüssel: der des Anbieters (`"<id>:<modell>:<stimme>"`).
    /// Die Fish-Referenzstimme, die der Kern mitreicht, zählt hier nicht.
    pub fn cache_tag(&self) -> String {
        self.cache_tag_for(None)
    }

    /// Wie [`Self::cache_tag`] für einen Satz mit eigener Stimme `voice` (Sprecher aus
    /// dem Skript): sie ersetzt die der Engine, so teilen sich zwei Stimmen nie einen
    /// Cache-Eintrag.
    pub fn cache_tag_for(&self, voice: Option<&str>) -> String {
        let voice = clean_voice(voice).or(self.voice.as_deref());
        match &self.provider {
            Some(p) => p.cache_tag(voice),
            None => format!("provider:{}:unbekannt", self.id),
        }
    }

    fn unknown(&self) -> TtsError {
        TtsError::unavailable(format!(
            "Der Anbieter '{}' ist nicht (mehr) vorhanden. In den Einstellungen unter \
             Sprachausgabe-Anbieter einen anderen wählen.",
            self.id
        ))
    }

    /// Die Guardrails dieser Engine ([`check_use`]) gegen den AKTUELLEN Stand
    /// der Freigaben — jeder Aufruf liest ihn neu.
    pub fn guard(&self) -> Result<(), TtsError> {
        let Some(desc) = &self.descriptor else {
            return Err(self.unknown());
        };
        self.approvals.check(desc, &self.subject)
    }

    /// Wartet, bis die Nutzung gesperrt ist (Widerruf mitten im Satz), und
    /// liefert den Sperrgrund. Endet nie, solange die Nutzung erlaubt bleibt.
    async fn wait_revoked(&self) -> TtsError {
        loop {
            tokio::time::sleep(REVOKE_POLL).await;
            if let Err(e) = self.guard() {
                return e;
            }
        }
    }

    /// Bereit für einen Auftrag? Guardrails, dann Gesundheit mit Frist; ein
    /// nicht verfügbarer Anbieter meldet Grund und Hinweis.
    pub async fn ensure_ready(&self) -> Result<(), String> {
        self.guard().map_err(|e| e.message)?;
        let health = self
            .registry
            .health(&self.id, self.ready_timeout)
            .await
            .map_err(|e| e.message)?;
        if health.available {
            return Ok(());
        }
        let reason = health
            .reason
            .unwrap_or_else(|| "nicht verfügbar".to_string());
        Err(match health.hint {
            Some(hint) if !hint.trim().is_empty() => format!("{reason} — {hint}"),
            _ => reason,
        })
    }

    /// Die Stimmenliste des Anbieters (Grundlage der Sprecher, siehe
    /// [`super::speakers`]), mit Frist [`VOICES_TIMEOUT`]. Gilt als Nutzung: ein
    /// gesperrter Anbieter antwortet mit dem Sperrgrund, ohne dass er gerufen wird.
    pub async fn list_voices(&self) -> Result<Vec<VoiceDescriptor>, TtsError> {
        self.guard()?;
        let provider = self.provider.clone().ok_or_else(|| self.unknown())?;
        match tokio::time::timeout(VOICES_TIMEOUT, provider.list_voices(None)).await {
            Ok(result) => result,
            Err(_) => Err(TtsError::timeout(format!(
                "Die Stimmenliste kam nicht innerhalb von {} s",
                VOICES_TIMEOUT.as_secs()
            ))),
        }
    }

    /// Einen Satz in der Stimme der Engine synthetisieren; Rückgabe sind WAV-Bytes.
    /// `cancel` ist das Abbruch-Flag des laufenden Auftrags (siehe Moduldoku).
    pub async fn synthesize(
        &self,
        text: &str,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        self.synthesize_voiced(text, None, cancel).await
    }

    /// Wie [`Self::synthesize`], aber mit einer eigenen Stimme für diesen Satz
    /// (`voice`, Sprecher aus dem Skript); `None` = die Stimme der Engine. Die
    /// Stimme geht unverändert als `SynthesisOptions::voice` an den Anbieter.
    pub async fn synthesize_voiced(
        &self,
        text: &str,
        voice: Option<&str>,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<Vec<u8>, String> {
        self.guard().map_err(|e| e.message)?;
        let provider = self
            .provider
            .clone()
            .ok_or_else(|| self.unknown().message)?;
        let token = CancelToken::new(next_request_id("read"));
        let opts = SynthesisOptions {
            language: detect_language(text).unwrap_or("de").to_string(),
            voice: clean_voice(voice)
                .map(String::from)
                .or_else(|| self.voice.clone()),
            format: AudioEncoding::Wav,
            // Tempo regelt die Wiedergabe (siehe `SynthesisRequest::speed`).
            speed: None,
            seed: None,
            timeout: SENTENCE_TIMEOUT,
        };
        let safety_net = opts.timeout.saturating_add(SAFETY_GRACE);
        let tracker = DeliveryTracker::default();
        let policy = RetryPolicy::default();
        let work = with_retry(&policy, &token, &tracker, |_attempt| {
            guarded(
                &token,
                safety_net,
                &self.id,
                provider.synthesize(text, &opts, &token),
            )
        });
        let result = match cancel {
            Some(flag) => {
                tokio::select! {
                    biased;
                    _ = wait_flag(flag) => {
                        // Das Future des Anbieters fällt mit `work`; die Marke
                        // weckt alles, was noch auf sie hört.
                        token.cancel();
                        return Err(CANCELLED_MESSAGE.to_string());
                    }
                    blocked = self.wait_revoked() => {
                        // Widerruf mitten im Satz: nichts mehr ausliefern.
                        token.cancel();
                        return Err(blocked.message);
                    }
                    r = work => r,
                }
            }
            None => {
                tokio::select! {
                    biased;
                    blocked = self.wait_revoked() => {
                        token.cancel();
                        return Err(blocked.message);
                    }
                    r = work => r,
                }
            }
        };
        result.map(|clip| clip.data).map_err(|e| e.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::engine::{self, TtsEngineKind};
    use crate::managers::tts::providers::contract::{
        AudioClip, ProviderHealth, TtsErrorKind, VoiceDescriptor,
    };
    use crate::managers::tts::providers::mock::{MockFailure, MockProvider};
    use crate::managers::tts::providers::policy::{license_fingerprint, TtsProvidersSettings};
    use crate::managers::tts::providers::speakers;
    use crate::managers::tts::state::TtsPhase;
    use crate::managers::tts::{split_utterances, EngineImpl, TtsCore};
    use futures_util::future::BoxFuture;
    use std::sync::Mutex;
    use std::time::Instant;

    const TEXT: &str = "Der erste Satz ist lang genug. Der zweite Satz ist es ebenfalls.";

    fn registry_with(mock: Arc<dyn TtsProvider>) -> Arc<ProviderRegistry> {
        let reg = Arc::new(ProviderRegistry::new());
        reg.register(mock).unwrap();
        reg
    }

    fn settings_with(ids: &[&str]) -> Arc<TtsProvidersSettings> {
        let mut s = TtsProvidersSettings::default();
        for id in ids {
            s.set_enabled(id, true);
        }
        Arc::new(s)
    }

    /// Engine mit einem FESTEN Stand der Freigaben (für Tests ohne Widerruf).
    fn engine_for(
        reg: &Arc<ProviderRegistry>,
        id: &str,
        settings: Arc<TtsProvidersSettings>,
    ) -> ProviderEngine {
        ProviderEngine::new(
            reg.clone(),
            id,
            None,
            LiveApprovals::new(settings.as_ref().clone()),
        )
    }

    fn live_with(ids: &[&str]) -> LiveApprovals {
        LiveApprovals::new(settings_with(ids).as_ref().clone())
    }

    fn core_with(engine: ProviderEngine) -> TtsCore {
        let core = TtsCore::for_test(1);
        *core.engine.lock().unwrap() = EngineImpl::Provider(engine);
        core
    }

    /// Anbieter, der die Optionen seines letzten Auftrags festhält.
    struct Recording {
        inner: MockProvider,
        last: Mutex<Option<SynthesisOptions>>,
    }

    impl TtsProvider for Recording {
        fn descriptor(&self) -> ProviderDescriptor {
            self.inner.descriptor()
        }
        fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
            self.inner.health_check()
        }
        fn list_voices<'a>(
            &'a self,
            l: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
            self.inner.list_voices(l)
        }
        fn synthesize<'a>(
            &'a self,
            t: &'a str,
            o: &'a SynthesisOptions,
            c: &'a CancelToken,
        ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
            *self.last.lock().unwrap() = Some(o.clone());
            self.inner.synthesize(t, o, c)
        }
        fn cache_tag(&self, v: Option<&str>) -> String {
            format!("rec:{}", v.unwrap_or("-"))
        }
    }

    /// Anbieter mit Latenz, der von JEDEM Auftrag Text und Stimme festhält -- beim
    /// BEGINN, auch wenn der Auftrag danach fallengelassen wird.
    struct VoiceLog {
        inner: MockProvider,
        calls: Mutex<Vec<(String, Option<String>)>>,
    }

    impl TtsProvider for VoiceLog {
        fn descriptor(&self) -> ProviderDescriptor {
            self.inner.descriptor()
        }
        fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
            self.inner.health_check()
        }
        fn list_voices<'a>(
            &'a self,
            l: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
            self.inner.list_voices(l)
        }
        fn synthesize<'a>(
            &'a self,
            t: &'a str,
            o: &'a SynthesisOptions,
            c: &'a CancelToken,
        ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
            self.calls
                .lock()
                .unwrap()
                .push((t.to_string(), o.voice.clone()));
            self.inner.synthesize(t, o, c)
        }
        fn cache_tag(&self, v: Option<&str>) -> String {
            self.inner.cache_tag(v)
        }
    }

    // ---- Engine-Wahl --------------------------------------------------------------

    #[test]
    fn die_engine_wahl_provider_id_wird_erkannt() {
        assert_eq!(
            TtsEngineKind::from_setting("provider:phonon"),
            TtsEngineKind::Provider
        );
        assert_eq!(
            TtsEngineKind::from_setting(" Provider:Ollama-Speech "),
            TtsEngineKind::Provider
        );
        assert_eq!(
            engine::provider_id_from_setting("provider:phonon").as_deref(),
            Some("phonon")
        );
        assert_eq!(
            engine::provider_id_from_setting("Provider:OpenAI-Speech").as_deref(),
            Some("openai-speech")
        );
        // Eingebaute Engines behalten ihre unveränderten Wege.
        assert_eq!(
            TtsEngineKind::from_setting("provider:fish"),
            TtsEngineKind::Fish
        );
        assert_eq!(
            TtsEngineKind::from_setting("provider:piper"),
            TtsEngineKind::Piper
        );
        assert_eq!(engine::provider_id_from_setting("provider:fish"), None);
        assert_eq!(engine::provider_id_from_setting("provider:piper"), None);
        // Kaputtes und Fremdes fällt wie bisher auf Fish zurück.
        for bad in [
            "provider:",
            "provider:Bad Id",
            "provider:-x",
            "kokoro",
            "",
            "provider",
        ] {
            assert_eq!(
                TtsEngineKind::from_setting(bad),
                TtsEngineKind::Fish,
                "{bad:?}"
            );
            assert_eq!(engine::provider_id_from_setting(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn die_einstellung_zu_einer_kennung_ist_umkehrbar() {
        assert_eq!(engine::engine_setting_for("fish"), "fish");
        assert_eq!(engine::engine_setting_for("piper"), "piper");
        assert_eq!(engine::engine_setting_for("phonon"), "provider:phonon");
        for id in ["phonon", "openai-speech", "ollama-speech", "kokoro"] {
            let s = engine::engine_setting_for(id);
            assert_eq!(engine::provider_id_from_setting(&s).as_deref(), Some(id));
        }
    }

    // ---- Fähigkeiten ----------------------------------------------------------------

    #[test]
    fn die_faehigkeiten_werden_auf_engine_caps_abgebildet() {
        let chunked = MockProvider::new().chunked(3, Duration::ZERO);
        let mut d = chunked.descriptor();
        d.caps.cloning = true;
        d.caps.style_tags = true;
        let c = engine_caps(&d);
        assert!(c.streaming && c.cloning && c.style_tags);
        assert!(!c.needs_gpu);
        assert_eq!(c.export_formats, &["wav"]);
        // Den Stimmwechsel meldet der Anbieter selbst (der Mock hat zwei Stimmen) ...
        assert!(c.voice_switching);
        // ... ohne die Angabe gibt es keine Sprecher im Skript.
        d.caps.voice_switching = false;
        assert!(!engine_caps(&d).voice_switching);
        let single = MockProvider::new().descriptor();
        assert!(
            !engine_caps(&single).streaming,
            "Sentence ist kein Chunk-Streaming"
        );
    }

    #[test]
    fn eine_unbekannte_kennung_verspricht_nichts() {
        let reg = Arc::new(ProviderRegistry::new());
        let e = engine_for(&reg, "gibt-es-nicht", settings_with(&[]));
        let c = e.caps();
        assert!(!c.streaming && !c.cloning && !c.style_tags && !c.needs_gpu);
        assert_eq!(e.guard().unwrap_err().kind, TtsErrorKind::Unavailable);
        assert!(e.cache_tag().contains("gibt-es-nicht"));
    }

    // ---- Durch die Satz-Pipeline ----------------------------------------------------

    #[tokio::test]
    async fn ein_mock_anbieter_traegt_die_satz_pipeline_ohne_jeden_server() {
        let mock = Arc::new(MockProvider::new().with_id("zzz"));
        let reg = registry_with(mock.clone());
        let core = core_with(engine_for(&reg, "zzz", settings_with(&["zzz"])));
        assert_eq!(core.engine_kind(), TtsEngineKind::Provider);
        assert!(!core.engine_caps().needs_gpu);

        let played = core.speak_core(TEXT).await.unwrap();

        assert!(
            played > 2 * 1024,
            "beide WAVs kamen beim Player an: {played}"
        );
        assert_eq!(mock.attempts(), 2, "ein Aufruf je Satz");
        assert_eq!(core.phase(), TtsPhase::Ready);
    }

    #[tokio::test]
    async fn der_cache_trennt_anbieter_von_fish_und_trifft_beim_zweiten_lauf() {
        let mock = Arc::new(MockProvider::new().with_id("zzz"));
        let reg = registry_with(mock.clone());
        let core = core_with(engine_for(&reg, "zzz", settings_with(&["zzz"])));
        let text = "Dieser Satz wird nur ein einziges Mal synthetisiert.";
        core.speak_core(text).await.unwrap();
        core.speak_core(text).await.unwrap();
        assert_eq!(mock.attempts(), 1, "der zweite Lauf kommt aus dem Cache");
        assert!(core.has_cached(text));
        assert!(
            core.engine_cache_tag(Some("patrick")).starts_with("zzz"),
            "der Tag kommt vom Anbieter: {}",
            core.engine_cache_tag(None)
        );
        // Fish sieht den Treffer des Anbieters nicht — und behält den leeren Tag.
        *core.engine.lock().unwrap() = EngineImpl::Fish;
        assert!(!core.has_cached(text));
        assert_eq!(core.engine_cache_tag(Some("patrick")), "");
    }

    #[tokio::test]
    async fn die_stimme_kommt_vom_anbieter_nicht_von_fish() {
        let rec = Arc::new(Recording {
            inner: MockProvider::new().with_id("zzz"),
            last: Mutex::new(None),
        });
        let reg = registry_with(rec.clone());
        let engine =
            ProviderEngine::new(reg, "zzz", Some("mock-de".to_string()), live_with(&["zzz"]));
        let core = core_with(engine);
        // Die Fish-Referenzstimme des Kerns darf nicht beim Anbieter landen.
        *core.voice.lock().unwrap() = Some("fish-referenz".to_string());
        core.speak_core("Ein einzelner ausreichend langer Satz zum Sprechen.")
            .await
            .unwrap();
        let opts = rec.last.lock().unwrap().clone().unwrap();
        assert_eq!(opts.voice.as_deref(), Some("mock-de"));
        assert_eq!(opts.format, AudioEncoding::Wav);
        assert_eq!(opts.language, "de");
        // Auch der Schluessel des Caches kennt die Fish-Stimme nicht.
        assert_eq!(core.synth_identity(None).voice, None);
        assert_eq!(core.synth_identity(None).engine_tag, "rec:mock-de");
    }

    /// Stimmwechsel mitten im Vorlesen mit einem Anbieter (etwa Phonon): die
    /// Berechnung mit der alten Anbieter-Stimme wird fallengelassen (der
    /// Subprozess-Adapter stirbt mit seinem Future), die neue beginnt sofort, und
    /// der Cache haelt beide Stimmen auseinander -- der Tag des Anbieters traegt
    /// die Stimme.
    #[tokio::test]
    async fn ein_wechsel_der_anbieter_stimme_mitten_in_der_berechnung_gilt_sofort() {
        let log = Arc::new(VoiceLog {
            inner: MockProvider::new()
                .with_id("zzz")
                .latency(Duration::from_millis(400)),
            calls: Mutex::new(Vec::new()),
        });
        let reg = registry_with(log.clone());
        let approvals = live_with(&["zzz"]);
        let engine = |voice: &str| {
            ProviderEngine::new(
                reg.clone(),
                "zzz",
                Some(voice.to_string()),
                approvals.clone(),
            )
        };
        let core = Arc::new(core_with(engine("anna")));

        let run = {
            let core = Arc::clone(&core);
            tokio::spawn(async move { core.speak_core(TEXT).await })
        };
        // Der erste Satz wird mit "anna" berechnet ...
        for _ in 0..400 {
            if !log.calls.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(log.calls.lock().unwrap().len(), 1);
        // ... und mittendrin wechselt die Stimme.
        core.set_provider_engine(engine("ben"));
        core.mark_voice_changed();
        let played = tokio::time::timeout(Duration::from_secs(10), run)
            .await
            .expect("die Sitzung endet nicht")
            .unwrap()
            .unwrap();
        assert!(played > 2 * 1024, "beide Saetze wurden gespielt: {played}");

        let voices: Vec<Option<String>> = log
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, voice)| voice.clone())
            .collect();
        assert_eq!(
            voices,
            vec![
                Some("anna".to_string()),
                Some("ben".to_string()),
                Some("ben".to_string())
            ],
            "die fallengelassene Berechnung mit anna, dann beide Saetze mit ben"
        );

        // Cache je Stimme: nur ben ist fertig, anna wurde nie fertig.
        let first = "Der erste Satz ist lang genug.";
        assert!(core.has_cached_for(first, None), "ben liegt im Cache");
        let ben_tag = core.engine_cache_tag(None);
        core.set_provider_engine(engine("anna"));
        assert!(
            !core.has_cached_for(first, None),
            "anna wurde fallengelassen, ihr Eintrag existiert nicht"
        );
        assert_ne!(
            core.engine_cache_tag(None),
            ben_tag,
            "der Tag traegt die Anbieter-Stimme"
        );
    }

    // ---- Mehrere Sprecher: je Satz die Stimme des Sprechers -----------------------------

    fn voice(id: &str, label: &str) -> VoiceDescriptor {
        VoiceDescriptor {
            id: id.to_string(),
            label: label.to_string(),
            language: "de".to_string(),
            gender: None,
            license: None,
        }
    }

    /// Die Sprecher einer Stimmenliste wie die von Phonon: Standardstimme, Nele, Anton.
    fn nele_und_anton() -> Vec<crate::managers::tts::protocol::KnownSpeaker> {
        let voices = [
            voice("default", "Standardstimme des Modells"),
            voice("nele-id", "Nele – hell"),
            voice("anton-id", "Anton – tief"),
        ];
        speakers::build_roster(&voices, None, None)
            .iter()
            .map(|s| s.known("zzz"))
            .collect()
    }

    fn voice_log() -> Arc<VoiceLog> {
        Arc::new(VoiceLog {
            inner: MockProvider::new().with_id("zzz"),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls_of(log: &VoiceLog) -> Vec<(String, Option<String>)> {
        log.calls.lock().unwrap().clone()
    }

    const SKRIPT: &str = "<Nele> Der erste Satz ist lang genug. \
        <Anton> Der zweite Satz ist auch lang genug. \
        <Bob> Der dritte Satz ist ebenfalls lang genug.";

    /// Der Kern des Pakets: jeder Abschnitt geht mit der Stimme SEINES Sprechers an den
    /// Anbieter (`SynthesisOptions::voice`); ein unbekannter Marker wird gestrichen und
    /// liest in der eingestellten Stimme; kein Marker erreicht je den Anbieter als Text.
    #[tokio::test]
    async fn jeder_satz_geht_mit_der_stimme_seines_sprechers_an_den_anbieter() {
        let log = voice_log();
        let reg = registry_with(log.clone());
        let core = core_with(engine_for(&reg, "zzz", settings_with(&["zzz"])));
        assert!(core.engine_caps().voice_switching);

        let utterances = split_utterances(SKRIPT, &nele_und_anton(), true);
        assert_eq!(utterances.len(), 3, "{utterances:?}");
        core.speak_sentence_run(utterances, 0, None, None)
            .await
            .unwrap();

        let calls = calls_of(&log);
        assert_eq!(
            calls.iter().map(|(_, v)| v.as_deref()).collect::<Vec<_>>(),
            [Some("nele-id"), Some("anton-id"), None],
            "{calls:?}"
        );
        assert!(
            calls.iter().all(|(text, _)| !text.contains('<')),
            "kein Marker wird vorgelesen: {calls:?}"
        );
    }

    #[tokio::test]
    async fn der_cache_haelt_die_stimmen_auseinander_und_trifft_beim_zweiten_lauf() {
        let log = voice_log();
        let reg = registry_with(log.clone());
        let core = core_with(engine_for(&reg, "zzz", settings_with(&["zzz"])));
        let satz = "Derselbe Satz klingt je Sprecher anders.";
        let nele = speakers::speaker_key("zzz", "nele-id");
        let anton = speakers::speaker_key("zzz", "anton-id");
        let run = || {
            vec![
                (satz.to_string(), Some(nele.clone())),
                (satz.to_string(), Some(anton.clone())),
            ]
        };
        core.speak_sentence_run(run(), 0, None, None).await.unwrap();
        assert_eq!(calls_of(&log).len(), 2, "zwei Stimmen, zwei Berechnungen");

        assert!(core.has_cached_for(satz, Some(&nele)));
        assert!(core.has_cached_for(satz, Some(&anton)));
        assert!(
            !core.has_cached_for(satz, None),
            "die eigene Stimme des Anbieters ist eine dritte"
        );
        assert_ne!(
            core.engine_cache_tag(Some("nele-id")),
            core.engine_cache_tag(Some("anton-id")),
            "der Tag traegt die Stimme des Sprechers"
        );

        core.speak_sentence_run(run(), 0, None, None).await.unwrap();
        assert_eq!(
            calls_of(&log).len(),
            2,
            "der zweite Lauf kommt aus dem Cache"
        );
    }

    #[tokio::test]
    async fn text_ohne_marker_und_unbekannte_marker_lesen_in_der_eingestellten_stimme() {
        let log = voice_log();
        let reg = registry_with(log.clone());
        let engine =
            ProviderEngine::new(reg, "zzz", Some("mock-de".to_string()), live_with(&["zzz"]));
        let core = core_with(engine);
        let text = "Der erste Satz ohne Marker ist lang genug. \
                    <Nele> Der zweite Satz gehoert Nele und ist lang. \
                    <Bob> Der dritte Satz gehoert niemandem und ist lang.";
        let utterances = split_utterances(text, &nele_und_anton(), true);
        core.speak_sentence_run(utterances, 0, None, None)
            .await
            .unwrap();
        assert_eq!(
            calls_of(&log)
                .iter()
                .map(|(_, v)| v.as_deref())
                .collect::<Vec<_>>(),
            [Some("mock-de"), Some("nele-id"), Some("mock-de")]
        );
    }

    /// Stil-Tags `[…]` setzt ein Anbieter ohne `style_tags` (Phonon) nicht um und
    /// würde sie wörtlich vorlesen: sie erreichen ihn nie. Pausen erzeugt die App selbst
    /// (Stille), der Satz geht in Teilen an den Anbieter; der Stil hinter dem Sprecher
    /// (`<Anton:leise>`) wird vom Parser entfernt.
    #[tokio::test]
    async fn stil_tags_erreichen_einen_anbieter_ohne_stil_tags_nicht() {
        let log = voice_log();
        let reg = registry_with(log.clone());
        let core = core_with(engine_for(&reg, "zzz", settings_with(&["zzz"])));
        assert!(!core.engine_caps().style_tags);
        let text = "<Anton:leise> Das ist [laughing] ein Satz mit [whisper] Tags. [short pause] Und danach geht es weiter.";
        let utterances = split_utterances(text, &nele_und_anton(), true);
        core.speak_sentence_run(utterances, 0, None, None)
            .await
            .unwrap();
        let calls = calls_of(&log);
        assert!(!calls.is_empty());
        for (spoken, voice) in &calls {
            assert!(
                !spoken.contains('[') && !spoken.contains(']') && !spoken.contains('<'),
                "dem Anbieter darf kein Tag und kein Marker erreichen: {spoken:?}"
            );
            assert_eq!(voice.as_deref(), Some("anton-id"));
        }
        assert!(
            calls.iter().any(|(t, _)| t.contains("Das ist")),
            "der Text selbst bleibt: {calls:?}"
        );
    }

    /// Eine gewaehlte feste Stimme liest alles; die Sprecher im Skript gelten dann nur
    /// bei "Skript mit Sprechern" (wie bei Fish).
    #[tokio::test]
    async fn eine_feste_stimme_liest_alles_und_uebergeht_die_sprecher() {
        let log = voice_log();
        let reg = registry_with(log.clone());
        let engine =
            ProviderEngine::new(reg, "zzz", Some("mock-de".to_string()), live_with(&["zzz"]));
        let core = core_with(engine);
        core.fixed_voice
            .store(true, std::sync::atomic::Ordering::Release);
        // Der Text wird ohne Marker zerlegt (wie `utterances` bei fester Stimme) ...
        let plain = split_utterances(SKRIPT, &nele_und_anton(), false);
        assert!(plain
            .iter()
            .all(|(text, voice)| voice.is_none() && !text.contains('<')));
        // ... und selbst ein Satz, der noch einen Sprecher traegt (Sitzung von vorher),
        // liest in der festen Stimme.
        let with_speakers = split_utterances(SKRIPT, &nele_und_anton(), true);
        core.speak_sentence_run(with_speakers, 0, None, None)
            .await
            .unwrap();
        assert!(
            calls_of(&log)
                .iter()
                .all(|(_, v)| v.as_deref() == Some("mock-de")),
            "{:?}",
            calls_of(&log)
        );
    }

    /// Anbieter ohne Stimmwechsel: Sprecher gelten nicht, alles liest die eine Stimme.
    struct Flat(Arc<VoiceLog>);

    impl TtsProvider for Flat {
        fn descriptor(&self) -> ProviderDescriptor {
            let mut d = self.0.descriptor();
            d.caps.voice_switching = false;
            d
        }
        fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
            self.0.health_check()
        }
        fn list_voices<'a>(
            &'a self,
            l: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
            self.0.list_voices(l)
        }
        fn synthesize<'a>(
            &'a self,
            t: &'a str,
            o: &'a SynthesisOptions,
            c: &'a CancelToken,
        ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
            self.0.synthesize(t, o, c)
        }
        fn cache_tag(&self, v: Option<&str>) -> String {
            self.0.cache_tag(v)
        }
    }

    #[tokio::test]
    async fn ein_anbieter_ohne_stimmwechsel_liest_alles_in_seiner_stimme() {
        let log = voice_log();
        let reg = registry_with(Arc::new(Flat(log.clone())));
        let engine =
            ProviderEngine::new(reg, "zzz", Some("mock-de".to_string()), live_with(&["zzz"]));
        let core = core_with(engine);
        assert!(!core.engine_caps().voice_switching);
        let utterances = split_utterances(SKRIPT, &nele_und_anton(), true);
        core.speak_sentence_run(utterances, 0, None, None)
            .await
            .unwrap();
        assert!(calls_of(&log)
            .iter()
            .all(|(_, v)| v.as_deref() == Some("mock-de")));
    }

    /// Die Stimme eines Satzes ist an seine Welt gebunden: ein Schluessel eines anderen
    /// Anbieters oder eine Fish-Kennung erreicht den Anbieter nicht (der Satz liest in
    /// der eigenen Stimme), und Fish bekommt nie einen Anbieter-Schluessel.
    #[tokio::test]
    async fn fremde_stimmen_erreichen_weder_den_anbieter_noch_fish() {
        let log = voice_log();
        let reg = registry_with(log.clone());
        let core = core_with(engine_for(&reg, "zzz", settings_with(&["zzz"])));
        let sentences = vec![
            (
                "Ein Satz mit der Fish-Stimme aus einer alten Sitzung.".to_string(),
                Some("erzaehlerin".to_string()),
            ),
            (
                "Ein Satz mit der Stimme eines anderen Anbieters.".to_string(),
                Some(speakers::speaker_key("andere", "nele-id")),
            ),
            (
                "Ein Satz mit der Stimme dieses Anbieters selbst.".to_string(),
                Some(speakers::speaker_key("zzz", "nele-id")),
            ),
        ];
        core.speak_sentence_run(sentences, 0, None, None)
            .await
            .unwrap();
        assert_eq!(
            calls_of(&log)
                .iter()
                .map(|(_, v)| v.as_deref())
                .collect::<Vec<_>>(),
            [None, None, Some("nele-id")]
        );

        let own = speakers::speaker_key("zzz", "nele-id");
        let fish = EngineImpl::Fish;
        assert_eq!(TtsCore::translate_voice(&fish, Some(&own)), None);
        assert_eq!(
            TtsCore::translate_voice(&fish, Some("erzaehlerin")).as_deref(),
            Some("erzaehlerin"),
            "Fish behaelt seine Kennungen unveraendert"
        );
        // Die gewaehlte Fish-Stimme (`tts_voice`) bleibt fuer Fish der Rueckfall ...
        *core.voice.lock().unwrap() = Some("fish-referenz".to_string());
        assert_eq!(
            core.effective_voice(&fish, None).as_deref(),
            Some("fish-referenz")
        );
        // ... fuer einen Anbieter aber nie.
        let provider = EngineImpl::Provider(engine_for(&reg, "zzz", settings_with(&["zzz"])));
        assert_eq!(core.effective_voice(&provider, None), None);
        assert_eq!(
            core.effective_voice(&provider, Some(&own)).as_deref(),
            Some("nele-id")
        );
    }

    /// Ein Wechsel von "Skript mit Sprechern" zu einer festen Stimme (oder zurueck)
    /// mitten im Vorlesen gilt fuer den Folgesatz: die Berechnung mit der alten Stimme
    /// wird verworfen (die Identitaet des Satzes aendert sich).
    #[tokio::test]
    async fn der_wechsel_zur_festen_stimme_aendert_die_identitaet_des_satzes() {
        let log = voice_log();
        let reg = registry_with(log.clone());
        let engine =
            ProviderEngine::new(reg, "zzz", Some("mock-de".to_string()), live_with(&["zzz"]));
        let core = core_with(engine);
        let nele = speakers::speaker_key("zzz", "nele-id");
        let speaker = Some(nele.clone());
        let before = core.synth_identity(core.utterance_voice(speaker.as_ref()).as_deref());
        assert_eq!(before.voice.as_deref(), Some("nele-id"));
        assert!(core.voice_unchanged(&before, speaker.as_ref()));

        core.fixed_voice
            .store(true, std::sync::atomic::Ordering::Release);
        assert!(
            !core.voice_unchanged(&before, speaker.as_ref()),
            "feste Stimme: der Satz muss mit \"mock-de\" neu berechnet werden"
        );
        let after = core.synth_identity(core.utterance_voice(speaker.as_ref()).as_deref());
        assert_eq!(
            after.voice, None,
            "die eigene Stimme steckt im Tag des Anbieters"
        );
        assert!(
            before.engine_tag.ends_with("nele-id"),
            "{}",
            before.engine_tag
        );
        assert!(
            after.engine_tag.ends_with("mock-de"),
            "{}",
            after.engine_tag
        );
    }

    #[tokio::test]
    async fn ein_anbieterfehler_endet_im_fehlerzustand_mit_der_meldung() {
        let mock = Arc::new(MockProvider::new().with_id("zzz").failing(MockFailure {
            error: TtsError::backend("Server kaputt"),
            at_chunk: 0,
            for_attempts: None,
        }));
        let reg = registry_with(mock);
        let core = core_with(engine_for(&reg, "zzz", settings_with(&["zzz"])));
        let err = core.speak_core(TEXT).await.unwrap_err();
        assert!(err.contains("Server kaputt"), "{err}");
        assert_eq!(core.phase(), TtsPhase::Error);
    }

    // ---- Guardrails in der Pipeline -------------------------------------------------

    #[tokio::test]
    async fn ohne_freischaltung_synthetisiert_der_anbieter_nichts() {
        let mock = Arc::new(MockProvider::new().with_id("zzz"));
        let reg = registry_with(mock.clone());
        let core = core_with(engine_for(&reg, "zzz", settings_with(&[])));
        let err = core.speak_core(TEXT).await.unwrap_err();
        assert!(err.contains("nicht freigeschaltet"), "{err}");
        assert_eq!(mock.attempts(), 0, "kein Aufruf ohne Opt-in");
        assert_eq!(core.phase(), TtsPhase::Error);
    }

    #[tokio::test]
    async fn die_sperre_gilt_auch_fuer_bereits_gecachte_saetze() {
        let mock = Arc::new(MockProvider::new().with_id("zzz"));
        let reg = registry_with(mock.clone());
        let text = "Dieser Satz liegt danach im Cache des Anbieters.";
        let core = core_with(engine_for(&reg, "zzz", settings_with(&["zzz"])));
        core.speak_core(text).await.unwrap();
        assert!(core.has_cached(text));
        // Der Nutzer nimmt die Freischaltung zurück: dieselbe Engine, neuer Stand.
        *core.engine.lock().unwrap() =
            EngineImpl::Provider(engine_for(&reg, "zzz", settings_with(&[])));
        let err = core.speak_core(text).await.unwrap_err();
        assert!(err.contains("nicht freigeschaltet"), "{err}");
        let fetched = core.fetch_wav(1, 42, text, None).await;
        assert!(fetched.is_err(), "auch der direkte Weg liefert nichts");
    }

    /// Anbieter mit nichtkommerzieller Gewichts-Lizenz (der Mock hat eine feste).
    struct Nc(MockProvider);

    impl TtsProvider for Nc {
        fn descriptor(&self) -> ProviderDescriptor {
            let mut d = self.0.descriptor();
            d.license.non_commercial = true;
            d
        }
        fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
            self.0.health_check()
        }
        fn list_voices<'a>(
            &'a self,
            l: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
            self.0.list_voices(l)
        }
        fn synthesize<'a>(
            &'a self,
            t: &'a str,
            o: &'a SynthesisOptions,
            c: &'a CancelToken,
        ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
            self.0.synthesize(t, o, c)
        }
        fn cache_tag(&self, v: Option<&str>) -> String {
            self.0.cache_tag(v)
        }
        fn use_subject(&self) -> UseSubject {
            self.0.use_subject()
        }
    }

    #[tokio::test]
    async fn nichtkommerziell_laeuft_erst_mit_zustimmung_zum_aktuellen_modell() {
        // Das Modell gehört zur Instanz (`use_subject`), nicht zu einem
        // getrennt mitgereichten Stand.
        let nc_for = |model: &str| Nc(MockProvider::new().with_id("zzz").with_subject(model, None));
        let nc_a = nc_for("modell-a");
        let desc = nc_a.descriptor();
        let subj = nc_a.use_subject();
        assert_eq!(subj.model, "modell-a");
        let mut settings = TtsProvidersSettings::default();
        settings.set_enabled("zzz", true);
        let mk = |provider: Nc, s: &TtsProvidersSettings| {
            ProviderEngine::new(
                registry_with(Arc::new(provider)),
                "zzz",
                None,
                LiveApprovals::new(s.clone()),
            )
        };
        // Ohne Zustimmung gesperrt.
        let core = core_with(mk(nc_for("modell-a"), &settings));
        assert!(core
            .speak_core(TEXT)
            .await
            .unwrap_err()
            .contains("bestätigt"));
        // Mit Zustimmung zum aktuellen Modell frei.
        settings.record_ack(
            "zzz",
            &license_fingerprint(&desc, &subj),
            "2026-10-26T00:00:00Z",
        );
        let core = core_with(mk(nc_for("modell-a"), &settings));
        assert!(core.speak_core(TEXT).await.is_ok());
        // Modellwechsel: die alte Zustimmung gilt nicht mehr.
        let core = core_with(mk(nc_for("modell-b"), &settings));
        assert!(core
            .speak_core(TEXT)
            .await
            .unwrap_err()
            .contains("geändert"));
    }

    // ---- Widerruf wirkt auf laufende Aufträge (Befund 1) ------------------------------

    /// Hörbuch über einen Anbieter, dem der Nutzer mitten im Auftrag die
    /// Freischaltung nimmt: der nächste Satz scheitert, der Anbieter wird nicht
    /// mehr gerufen — und die Engine wurde dafür NICHT neu gebaut.
    #[tokio::test]
    async fn ein_entzogenes_opt_in_stoppt_den_naechsten_satz_ohne_neue_engine() {
        let mock = Arc::new(MockProvider::new().with_id("zzz"));
        let reg = registry_with(mock.clone());
        let live = live_with(&["zzz"]);
        let core = core_with(ProviderEngine::new(reg.clone(), "zzz", None, live.clone()));
        let first = "Der erste Satz kommt noch durch.";
        let second = "Der zweite Satz darf nicht mehr durch.";
        core.fetch_wav(1, 42, first, None)
            .await
            .expect("vor dem Widerruf ist der Anbieter frei");
        assert_eq!(mock.attempts(), 1);

        live.set(TtsProvidersSettings::default());

        let err = core.fetch_wav(1, 42, second, None).await.unwrap_err();
        assert!(err.contains("nicht freigeschaltet"), "{err}");
        assert_eq!(mock.attempts(), 1, "kein weiterer Aufruf des Anbieters");
        // Schon Synthetisiertes (liegt im Cache) wird ebenfalls nicht mehr ausgeliefert.
        assert!(core.fetch_wav(1, 42, first, None).await.is_err());
        // Nach erneuter Freigabe geht es ohne Neuaufbau wieder.
        live.set(settings_with(&["zzz"]).as_ref().clone());
        core.fetch_wav(1, 42, second, None).await.unwrap();
        assert_eq!(mock.attempts(), 2);
    }

    /// Die Fernzugriff-Zusicherung: Hörbuch über einen LAN-Server, der Nutzer
    /// nimmt den Fernzugriff zurück → ab dem nächsten Satz geht nichts mehr hinaus.
    #[tokio::test]
    async fn ein_zurueckgenommener_fernzugriff_stoppt_ein_hoerbuch_ueber_den_lan_server() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .with_subject("m", Some("http://192.168.1.20:8880/v1")),
        );
        let reg = registry_with(mock.clone());
        let mut allowed = TtsProvidersSettings::default();
        allowed.set_enabled("zzz", true);
        allowed.set_remote_allowed("zzz", true);
        let live = LiveApprovals::new(allowed.clone());
        let engine = ProviderEngine::new(reg, "zzz", None, live.clone());
        let core = core_with(engine.clone());
        core.fetch_wav(1, 42, "Der erste Satz geht ins Netz.", None)
            .await
            .expect("mit Fernzugriff frei");
        assert!(engine.guard().is_ok());

        let mut withdrawn = allowed;
        withdrawn.set_remote_allowed("zzz", false);
        live.set(withdrawn);

        assert_eq!(
            engine.guard().unwrap_err().kind,
            TtsErrorKind::PrivacyBlocked
        );
        let err = core
            .fetch_wav(1, 42, "Der zweite Satz bleibt zu Hause.", None)
            .await
            .unwrap_err();
        assert!(err.contains("außerhalb dieses Rechners"), "{err}");
        assert_eq!(mock.attempts(), 1, "nach dem Entzug nichts mehr gesendet");
    }

    /// Auch ein GERADE LAUFENDER Satz endet beim Widerruf (nicht erst der nächste).
    #[tokio::test]
    async fn ein_widerruf_beendet_auch_den_gerade_laufenden_satz() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .latency(Duration::from_secs(30)),
        );
        let reg = registry_with(mock.clone());
        let live = live_with(&["zzz"]);
        let engine = ProviderEngine::new(reg, "zzz", None, live.clone());
        let runner = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.synthesize(TEXT, None).await })
        };
        let started = Instant::now();
        while mock.in_flight() == 0 && started.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(mock.in_flight(), 1, "die Synthese läuft");

        live.set(TtsProvidersSettings::default());

        let result = tokio::time::timeout(Duration::from_secs(3), runner)
            .await
            .expect("der Widerruf wirkt binnen Sekunden, nicht erst nach 30 s")
            .unwrap();
        assert!(result.unwrap_err().contains("nicht freigeschaltet"));
        assert_eq!(
            mock.in_flight(),
            0,
            "das Anbieter-Future wurde fallengelassen"
        );
    }

    /// Wer den Stand VOR dem Auftrag fest einbaut, behält das alte Verhalten
    /// (Regression: ein fester Stand bleibt fest).
    #[tokio::test]
    async fn ein_fester_stand_aendert_sich_nicht_von_selbst() {
        let mock = Arc::new(MockProvider::new().with_id("zzz"));
        let reg = registry_with(mock.clone());
        let engine = engine_for(&reg, "zzz", settings_with(&["zzz"]));
        assert!(engine.guard().is_ok());
        assert!(engine.synthesize(TEXT, None).await.is_ok());
        let blocked = engine_for(&reg, "zzz", settings_with(&[]));
        assert!(blocked.guard().is_err());
        assert!(blocked.synthesize(TEXT, None).await.is_err());
        assert_eq!(mock.attempts(), 1);
    }

    // ---- Instanz und Gegenstand der Prüfung kommen zusammen (Befund 2) ----------------

    /// Die Engine hält die Instanz UND deren Gegenstand: wird die Registry
    /// zwischendurch mit einer anderen Konfiguration neu gebaut, prüft die alte
    /// Engine weiter, was ihre Instanz wirklich anspricht — und die neue Engine,
    /// was die neue Instanz anspricht. Nichts wird aus einem dritten Stand gemischt.
    #[tokio::test]
    async fn der_gegenstand_der_pruefung_stammt_aus_der_instanz_die_synthetisiert() {
        let remote = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .with_subject("m", Some("https://tts.example.org/v1")),
        );
        let reg = registry_with(remote.clone());
        let live = live_with(&["zzz"]);
        let old_engine = ProviderEngine::new(reg.clone(), "zzz", None, live.clone());
        // Kein Aufrufer reicht einen Gegenstand herein: die Sperre kommt allein aus der Instanz.
        assert_eq!(
            old_engine.guard().unwrap_err().kind,
            TtsErrorKind::PrivacyBlocked
        );

        // Neuaufbau der Registry: dieselbe Kennung, jetzt eine Loopback-Instanz.
        let local = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .with_subject("m", Some("http://127.0.0.1:8880/v1")),
        );
        reg.register(local.clone()).unwrap();
        let new_engine = ProviderEngine::new(reg.clone(), "zzz", None, live.clone());

        // Die alte Engine spricht weiter die ALTE (entfernte) Instanz an und bleibt gesperrt;
        // sie fällt nie auf den Gegenstand der neuen zurück und umgeht so die Sperre nicht.
        assert_eq!(
            old_engine.guard().unwrap_err().kind,
            TtsErrorKind::PrivacyBlocked
        );
        assert!(old_engine.synthesize(TEXT, None).await.is_err());
        assert_eq!(
            remote.attempts(),
            0,
            "die entfernte Instanz wurde nie gerufen"
        );
        // Die neue Engine passt zu ihrer Instanz.
        assert!(new_engine.guard().is_ok());
        assert!(new_engine.synthesize(TEXT, None).await.is_ok());
        assert_eq!(local.attempts(), 1);
    }

    // ---- Abbruch, Bereitschaft ------------------------------------------------------

    #[tokio::test]
    async fn abbruch_mitten_in_der_synthese_beendet_den_satz_sofort() {
        let mock = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .latency(Duration::from_secs(30)),
        );
        let reg = registry_with(mock.clone());
        let core = Arc::new(core_with(engine_for(&reg, "zzz", settings_with(&["zzz"]))));
        let runner = {
            let core = core.clone();
            tokio::spawn(async move { core.speak_core(TEXT).await })
        };
        // Warten, bis die Synthese läuft.
        let started = Instant::now();
        while mock.in_flight() == 0 && started.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(mock.in_flight(), 1, "die Synthese läuft");
        let t0 = Instant::now();
        core.cancel_core();
        let result = tokio::time::timeout(Duration::from_secs(3), runner)
            .await
            .expect("der Abbruch wirkt binnen Sekunden, nicht erst nach 30 s")
            .unwrap();
        assert!(t0.elapsed() < Duration::from_secs(3));
        // Ein Abbruch ist kein Fehler des Vorlesens.
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            mock.in_flight(),
            0,
            "das Anbieter-Future wurde fallengelassen"
        );
        assert_eq!(mock.attempts(), 1, "es lief kein zweiter Satz mehr an");
        assert_ne!(core.phase(), TtsPhase::Error);
    }

    #[tokio::test]
    async fn ensure_ready_meldet_grund_und_haelt_die_frist() {
        let down = Arc::new(
            MockProvider::new()
                .with_id("zzz")
                .unavailable("Programm fehlt"),
        );
        let reg = registry_with(down);
        let e = engine_for(&reg, "zzz", settings_with(&["zzz"]));
        let err = e.ensure_ready().await.unwrap_err();
        assert!(err.contains("Programm fehlt"), "{err}");
        assert!(
            err.contains("Nur im Test einstellbar"),
            "Hinweis fehlt: {err}"
        );

        let hang = Arc::new(MockProvider::new().with_id("zzz").hang_health());
        let reg = registry_with(hang);
        let e = engine_for(&reg, "zzz", settings_with(&["zzz"]))
            .with_ready_timeout(Duration::from_millis(150));
        let t0 = Instant::now();
        let err = e.ensure_ready().await.unwrap_err();
        assert!(t0.elapsed() < Duration::from_secs(3));
        assert!(err.contains("Zeit"), "{err}");

        let ok = Arc::new(MockProvider::new().with_id("zzz"));
        let reg = registry_with(ok);
        assert!(engine_for(&reg, "zzz", settings_with(&["zzz"]))
            .ensure_ready()
            .await
            .is_ok());
        // Ohne Opt-in nicht einmal die Prüfung.
        assert!(engine_for(&reg, "zzz", settings_with(&[]))
            .ensure_ready()
            .await
            .is_err());
    }
}
