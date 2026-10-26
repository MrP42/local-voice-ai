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
//! [`ProviderEngine::guard`] ruft [`check_use`] gegen den Einstellungsstand,
//! mit dem die Engine gebaut wurde (der Manager baut sie vor jedem Auftrag
//! neu), und läuft in `fetch_wav` VOR dem Cache-Lookup: auch bereits
//! synthetisiertes Audio eines inzwischen gesperrten Anbieters wird nicht
//! mehr ausgeliefert.
//!
//! ## Abbruch
//!
//! `synthesize` hört auf das Abbruch-Flag des laufenden Auftrags und lässt bei
//! gesetztem Flag das Future des Anbieters fallen (Subprozess-Adapter setzen
//! `kill_on_drop`) — der Abbruch wirkt also auch MITTEN im Satz, anders als
//! bei Fish (HTTP-Anfrage läuft zu Ende) und Piper.
//!
//! Kein `settings::get_settings(&AppHandle)` hier.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::contract::{
    guarded, AudioEncoding, CancelToken, ProviderDescriptor, StreamingMode, SynthesisOptions,
    TtsError, TtsProvider,
};
use super::policy::{check_use, TtsProvidersSettings, UseSubject};
use super::registry::{with_retry, DeliveryTracker, ProviderRegistry, RetryPolicy};
use crate::managers::tts::engine::EngineCaps;
use crate::managers::tts::lang::detect_language;

/// Frist je Satz. Ein Satz dauert Sekunden; die Zahl fängt hängende Anbieter ab.
pub const SENTENCE_TIMEOUT: Duration = Duration::from_secs(120);
/// Frist der Bereitschaftsprüfung vor einem Auftrag.
pub const READY_TIMEOUT: Duration = Duration::from_secs(8);
/// Wie oft das Abbruch-Flag während der Synthese angesehen wird.
const CANCEL_POLL: Duration = Duration::from_millis(25);
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
/// * `voice_switching`: nie — Sprechermarker im Skript beziehen sich auf
///   Fish-Referenzstimmen; ein Anbieter spricht in EINER gewählten Stimme.
/// * `needs_gpu`: nie — die App kennt die Grafikkartennutzung eines fremden
///   Anbieters nicht, und das Flag steuert, ob die Fußleiste „Server läuft“
///   zeigt und ob Übersetzung die GPU meiden muss (nur bei Fish zutreffend).
/// * `export_formats`: nur WAV (der Anbieter liefert WAV; andere Formate
///   encodiert der Fish-Server).
pub fn engine_caps(desc: &ProviderDescriptor) -> EngineCaps {
    EngineCaps {
        style_tags: desc.caps.style_tags,
        cloning: desc.caps.cloning,
        voice_switching: false,
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
    subject: UseSubject,
    settings: Arc<TtsProvidersSettings>,
    ready_timeout: Duration,
}

impl ProviderEngine {
    /// Engine für `id` aus der Registry, mit gewählter Stimme, wirksamer
    /// Konfiguration (für die Guardrails) und dem Einstellungsstand der
    /// Freigaben.
    pub fn new(
        registry: Arc<ProviderRegistry>,
        id: &str,
        voice: Option<String>,
        subject: UseSubject,
        settings: Arc<TtsProvidersSettings>,
    ) -> Self {
        let provider = registry.get(id);
        let descriptor = provider.as_ref().map(|p| p.descriptor());
        Self {
            id: id.to_string(),
            registry,
            provider,
            descriptor,
            voice,
            subject,
            settings,
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

    pub fn caps(&self) -> EngineCaps {
        self.descriptor
            .as_ref()
            .map(engine_caps)
            .unwrap_or(UNKNOWN_CAPS)
    }

    /// Anteil am Cache-Schlüssel: der des Anbieters (`"<id>:<modell>:<stimme>"`).
    /// Die Fish-Referenzstimme, die der Kern mitreicht, zählt hier nicht.
    pub fn cache_tag(&self) -> String {
        match &self.provider {
            Some(p) => p.cache_tag(self.voice.as_deref()),
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

    /// Die Guardrails dieser Engine ([`check_use`]).
    pub fn guard(&self) -> Result<(), TtsError> {
        let Some(desc) = &self.descriptor else {
            return Err(self.unknown());
        };
        check_use(desc, &self.subject, &self.settings)
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

    /// Einen Satz synthetisieren; Rückgabe sind WAV-Bytes. `cancel` ist das
    /// Abbruch-Flag des laufenden Auftrags (siehe Moduldoku).
    pub async fn synthesize(
        &self,
        text: &str,
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
            voice: self.voice.clone(),
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
                    r = work => r,
                }
            }
            None => work.await,
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
    use crate::managers::tts::providers::policy::license_fingerprint;
    use crate::managers::tts::state::TtsPhase;
    use crate::managers::tts::{EngineImpl, TtsCore};
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

    fn engine_for(
        reg: &Arc<ProviderRegistry>,
        id: &str,
        settings: Arc<TtsProvidersSettings>,
    ) -> ProviderEngine {
        ProviderEngine::new(reg.clone(), id, None, UseSubject::default(), settings)
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
        assert!(!c.voice_switching && !c.needs_gpu);
        assert_eq!(c.export_formats, &["wav"]);
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
        let engine = ProviderEngine::new(
            reg,
            "zzz",
            Some("mock-de".to_string()),
            UseSubject::default(),
            settings_with(&["zzz"]),
        );
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
        assert_eq!(core.engine_cache_tag(Some("fish-referenz")), "rec:mock-de");
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
    }

    #[tokio::test]
    async fn nichtkommerziell_laeuft_erst_mit_zustimmung_zum_aktuellen_modell() {
        let nc = Nc(MockProvider::new().with_id("zzz"));
        let desc = nc.descriptor();
        let reg = registry_with(Arc::new(nc));
        let subj = UseSubject {
            model: "modell-a".into(),
            base_url: None,
        };
        let mut settings = TtsProvidersSettings::default();
        settings.set_enabled("zzz", true);
        let mk = |s: &TtsProvidersSettings, subj: &UseSubject| {
            ProviderEngine::new(reg.clone(), "zzz", None, subj.clone(), Arc::new(s.clone()))
        };
        // Ohne Zustimmung gesperrt.
        let core = core_with(mk(&settings, &subj));
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
        let core = core_with(mk(&settings, &subj));
        assert!(core.speak_core(TEXT).await.is_ok());
        // Modellwechsel: die alte Zustimmung gilt nicht mehr.
        let other = UseSubject {
            model: "modell-b".into(),
            base_url: None,
        };
        let core = core_with(mk(&settings, &other));
        assert!(core
            .speak_core(TEXT)
            .await
            .unwrap_err()
            .contains("geändert"));
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
