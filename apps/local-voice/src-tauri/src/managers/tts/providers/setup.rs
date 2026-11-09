//! Aufbau der [`ProviderRegistry`] aus den aufgelösten Konfigurationen
//! (Goal tts-provider, Paket P3a).
//!
//! * Fish- und Piper-Hülle sind IMMER registriert.
//! * phonon, openai-speech und ollama-speech sind IMMER registriert, auch
//!   unkonfiguriert und ohne Freigabe: so lassen sie sich listen und prüfen
//!   (Gesundheit zeigt „nicht konfiguriert“ mit Grund). BENUTZT werden dürfen
//!   sie erst nach den Guardrails ([`super::policy::check_use`]) — die gelten
//!   an der Nutzung, nicht an der Registrierung.
//! * Die Registry lebt LANGE: [`ProviderHost::sync`] ersetzt Anbieter an Ort
//!   und Stelle (`register` behält die Reihenfolge) und trägt nicht mehr
//!   vorhandene aus. Die Auftragstabelle der Registry bleibt dabei stehen —
//!   ein Neuaufbau mitten in einer laufenden Probestimme bricht sie nicht ab
//!   (der Auftrag hält seinen alten Anbieter), und `tts_provider_cancel` findet
//!   sie weiterhin.
//!
//! Kein `settings::get_settings(&AppHandle)` hier: Werte reicht der Aufrufer herein.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use super::builtin::{FishProvider, PiperProvider, SynthFn};
use super::contract::{TtsError, TtsProvider};
use super::ollama_speech::OllamaSpeechProvider;
use super::openai_speech::OpenAiSpeechProvider;
use super::phonon::PhononProvider;
use super::policy::{LiveApprovals, ResolvedConfigs, UseGate};
use super::registry::ProviderRegistry;

/// Orte und Einstellungen, die die Hüllen für Fish und Piper brauchen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderPaths {
    pub fish_dir: PathBuf,
    /// Datenordner der App (dort liegt `tts/piper/…`).
    pub app_data: Option<PathBuf>,
    /// Gewählte Piper-Stimme (`tts_piper_voice`).
    pub piper_voice: Option<String>,
    /// Piper: Sprache je Satz erkennen (`tts_piper_auto_language`).
    pub piper_auto_language: bool,
}

/// Alle Anbieter dieser Fassung in fester Reihenfolge. `gate`: die Prüfung der
/// Guardrails für Anbieter mit Nebenwirkung (Probe des Ollama-Anbieters); ohne
/// sie verweigert dieser Anbieter die Probe.
fn build_providers(
    resolved: &ResolvedConfigs,
    paths: &ProviderPaths,
    fish_synth: Option<&SynthFn>,
    gate: Option<&UseGate>,
) -> Vec<Arc<dyn TtsProvider>> {
    let mut fish = FishProvider::new(paths.fish_dir.clone());
    if let Some(synth) = fish_synth {
        fish = fish.with_synth(synth.clone());
    }
    let piper = PiperProvider::new(paths.app_data.clone())
        .with_voice(paths.piper_voice.clone())
        .with_auto_language(paths.piper_auto_language);
    let mut ollama = OllamaSpeechProvider::new(resolved.ollama_speech.clone());
    if let Some(gate) = gate {
        ollama = ollama.with_use_gate(gate.clone());
    }
    vec![
        Arc::new(fish),
        Arc::new(piper),
        Arc::new(phonon_provider(resolved)),
        Arc::new(OpenAiSpeechProvider::new(resolved.openai_speech.clone())),
        Arc::new(ollama),
    ]
}

/// Phonon mit der wirksamen Konfiguration. Stammt das Modell aus der Stufe „installiert“, kommen
/// die weiteren installierten Modelle anderer Sprachen dazu (das englische neben dem deutschen);
/// ein selbst eingetragenes Modell bleibt allein.
fn phonon_provider(resolved: &ResolvedConfigs) -> PhononProvider {
    let installed = &resolved.installed().phonon;
    let from_installed = installed
        .model
        .as_deref()
        .is_some_and(|m| m.trim() == resolved.phonon.model.trim());
    let provider = PhononProvider::new(resolved.phonon.clone());
    if from_installed {
        provider.with_siblings(installed.others.clone())
    } else {
        provider
    }
}

/// Trägt alle Anbieter in `registry` ein (vorhandene werden ersetzt) und
/// liefert deren Kennungen. Entfernt wird hier nichts: fremde Einträge
/// (Testanbieter) bleiben, Aufräumen ist Sache des [`ProviderHost`]. Ohne
/// Prüffunktion: der Ollama-Anbieter macht keine Probe (siehe
/// [`sync_registry_gated`]).
pub fn sync_registry(
    registry: &ProviderRegistry,
    resolved: &ResolvedConfigs,
    paths: &ProviderPaths,
    fish_synth: Option<&SynthFn>,
) -> Result<Vec<String>, TtsError> {
    sync_registry_gated(registry, resolved, paths, fish_synth, None)
}

/// Wie [`sync_registry`], mit der Prüffunktion der Guardrails: Anbieter mit
/// Nebenwirkung (die Probe des Ollama-Anbieters lädt ein Modell) fragen sie,
/// bevor sie etwas tun — gegen den Stand, den der Aufrufer hält (die App:
/// [`LiveApprovals::gate`], die CLI: die Einstellungen der Datei).
pub fn sync_registry_gated(
    registry: &ProviderRegistry,
    resolved: &ResolvedConfigs,
    paths: &ProviderPaths,
    fish_synth: Option<&SynthFn>,
    gate: Option<&UseGate>,
) -> Result<Vec<String>, TtsError> {
    let providers = build_providers(resolved, paths, fish_synth, gate);
    let wanted: Vec<String> = providers.iter().map(|p| p.descriptor().id).collect();
    for provider in providers {
        registry.register(provider)?;
    }
    Ok(wanted)
}

/// Frische Registry (Tests, CLI): [`sync_registry`] auf eine leere.
pub fn build_registry(
    resolved: &ResolvedConfigs,
    paths: &ProviderPaths,
    fish_synth: Option<&SynthFn>,
) -> Result<ProviderRegistry, TtsError> {
    let registry = ProviderRegistry::new();
    sync_registry(&registry, resolved, paths, fish_synth)?;
    Ok(registry)
}

/// Woran der Host merkt, dass er neu bauen muss.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BuildKey {
    resolved: ResolvedConfigs,
    paths: ProviderPaths,
}

/// Stand des letzten Aufbaus: Schlüssel plus die dabei eingetragenen Kennungen.
struct Built {
    key: BuildKey,
    ids: Vec<String>,
}

/// Besitzer der langlebigen Registry (lebt im `TtsManager`) und des
/// gemeinsamen, aktuellen Stands der Freigaben ([`LiveApprovals`]), gegen den
/// jede Nutzung (Engine, Probestimme, Probe des Ollama-Anbieters) prüft.
pub struct ProviderHost {
    registry: Arc<ProviderRegistry>,
    fish_synth: Option<SynthFn>,
    approvals: LiveApprovals,
    gate: UseGate,
    last: Mutex<Option<Built>>,
}

impl ProviderHost {
    pub fn new(fish_synth: Option<SynthFn>) -> Self {
        let approvals = LiveApprovals::default();
        Self {
            registry: Arc::new(ProviderRegistry::new()),
            fish_synth,
            gate: approvals.gate(),
            approvals,
            last: Mutex::new(None),
        }
    }

    pub fn registry(&self) -> &Arc<ProviderRegistry> {
        &self.registry
    }

    /// Der gemeinsame Stand der Freigaben. Wer die Einstellungen der Anbieter
    /// speichert, setzt ihn danach neu (`TtsManager::providers_changed`).
    pub fn approvals(&self) -> &LiveApprovals {
        &self.approvals
    }

    fn lock(&self) -> MutexGuard<'_, Option<Built>> {
        self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Bringt die Registry auf den Stand der Konfiguration. Billig, wenn sich
    /// nichts geändert hat (ein Vergleich); `true` = es wurde neu gebaut.
    /// Gleichzeitige Aufrufe laufen nacheinander (kein `await` unter der Sperre).
    pub fn sync(&self, resolved: &ResolvedConfigs, paths: &ProviderPaths) -> bool {
        let key = BuildKey {
            resolved: resolved.clone(),
            paths: paths.clone(),
        };
        let mut last = self.lock();
        if last.as_ref().is_some_and(|b| b.key == key) {
            return false;
        }
        match sync_registry_gated(
            &self.registry,
            resolved,
            paths,
            self.fish_synth.as_ref(),
            Some(&self.gate),
        ) {
            Ok(ids) => {
                // Was der vorige Aufbau eingetragen hat und jetzt nicht mehr
                // existiert (z. B. eine geänderte openai-speech-Kennung), geht.
                if let Some(previous) = last.as_ref() {
                    for old in previous.ids.iter().filter(|id| !ids.contains(id)) {
                        self.registry.unregister(old);
                    }
                }
                *last = Some(Built { key, ids });
                true
            }
            Err(e) => {
                // Nicht merken: der nächste Aufruf versucht es erneut.
                log::error!(
                    "TTS-Anbieter konnten nicht registriert werden: {}",
                    e.message
                );
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::contract::SynthesisOptions;
    use crate::managers::tts::providers::mock::MockProvider;
    use crate::managers::tts::providers::policy::{
        resolve_provider_configs, ProviderOverrides, TtsProvidersSettings,
    };
    use std::time::Duration;

    fn resolved(settings: &TtsProvidersSettings) -> ResolvedConfigs {
        resolve_provider_configs(settings, &|_| None, &ProviderOverrides::default())
    }

    fn ids(registry: &ProviderRegistry) -> Vec<String> {
        registry.descriptors().into_iter().map(|d| d.id).collect()
    }

    fn paths() -> ProviderPaths {
        ProviderPaths {
            fish_dir: PathBuf::from("C:/gibt/es/nicht"),
            ..Default::default()
        }
    }

    #[test]
    fn alle_fuenf_anbieter_sind_immer_registriert_auch_unkonfiguriert() {
        let reg =
            build_registry(&resolved(&TtsProvidersSettings::default()), &paths(), None).unwrap();
        assert_eq!(
            ids(&reg),
            ["fish", "piper", "phonon", "openai-speech", "ollama-speech"]
        );
    }

    #[tokio::test]
    async fn unkonfigurierte_anbieter_lassen_sich_pruefen_und_melden_einen_grund() {
        let reg =
            build_registry(&resolved(&TtsProvidersSettings::default()), &paths(), None).unwrap();
        let all = reg.health_all(Duration::from_secs(5)).await;
        let phonon = all.iter().find(|(id, _)| id == "phonon").unwrap();
        assert!(
            !phonon.1.available,
            "ohne ptts/Modell ist phonon nicht bereit"
        );
        assert!(phonon.1.reason.is_some(), "mit Grund");
        let fish = all.iter().find(|(id, _)| id == "fish").unwrap();
        assert!(!fish.1.available);
        assert!(fish.1.reason.is_some());
    }

    #[test]
    fn die_deskriptoren_tragen_die_erwarteten_opt_in_flags() {
        let reg =
            build_registry(&resolved(&TtsProvidersSettings::default()), &paths(), None).unwrap();
        for d in reg.descriptors() {
            let expect_opt_in = !matches!(d.id.as_str(), "fish" | "piper");
            assert_eq!(d.opt_in, expect_opt_in, "{}", d.id);
        }
    }

    #[test]
    fn die_konfiguration_erreicht_den_anbieter() {
        let mut s = TtsProvidersSettings::default();
        s.ollama_speech.model = "phonon:de".into();
        s.openai_speech.model = "kokoro".into();
        let reg = build_registry(&resolved(&s), &paths(), None).unwrap();
        assert!(reg
            .get("ollama-speech")
            .unwrap()
            .cache_tag(None)
            .contains("phonon:de"));
        assert!(reg
            .get("openai-speech")
            .unwrap()
            .cache_tag(None)
            .contains("kokoro"));
    }

    /// Befund 2: jede Instanz meldet als Gegenstand der Guardrails genau das,
    /// was `ResolvedConfigs::subject` für ihre Konfiguration liefert — die zwei
    /// Wege können nie auseinanderlaufen.
    #[test]
    fn die_instanz_meldet_denselben_gegenstand_wie_die_aufgeloeste_konfiguration() {
        let mut s = TtsProvidersSettings::default();
        s.phonon.model = "  hf://gradium/phonon-31m-en ".into();
        s.openai_speech.model = " kokoro ".into();
        s.openai_speech.base_url = " http://127.0.0.1:8880/v1 ".into();
        s.ollama_speech.model = " phonon:de ".into();
        s.ollama_speech.base_url = " http://127.0.0.1:11434 ".into();
        let check = |s: &TtsProvidersSettings| {
            let r = resolved(s);
            let reg = build_registry(&r, &paths(), None).unwrap();
            for d in reg.descriptors() {
                assert_eq!(
                    reg.get(&d.id).unwrap().use_subject(),
                    r.subject(&d.id),
                    "{}",
                    d.id
                );
            }
            reg
        };
        let reg = check(&s);
        assert_eq!(
            reg.get("phonon").unwrap().use_subject().model,
            "hf://gradium/phonon-31m-en"
        );
        assert_eq!(
            reg.get("ollama-speech")
                .unwrap()
                .use_subject()
                .base_url
                .as_deref(),
            Some("http://127.0.0.1:11434")
        );
        // Auch mit eigener Kennung des OpenAI-Anbieters.
        s.openai_speech.id = "kokoro".into();
        let reg = check(&s);
        assert_eq!(reg.get("kokoro").unwrap().use_subject().model, "kokoro");
    }

    /// Befund 3: der Host reicht den gemeinsamen Stand der Freigaben an die
    /// Probe des Ollama-Anbieters — ohne Freigabe kein Modellaufruf, nach der
    /// Freigabe läuft die Probe.
    #[tokio::test]
    async fn der_host_gibt_die_freigaben_an_die_probe_des_ollama_anbieters_weiter() {
        use crate::managers::tts::providers::ollama_speech::OLLAMA_ID;
        use crate::managers::tts::providers::policy::license_fingerprint;
        use crate::managers::tts::providers::test_http::{speech_reply, FakeServer, Reply, Req};
        if std::env::consts::OS == "macos" {
            return; // dort entfällt die Probe (Plattform wird unterstützt)
        }
        let server =
            FakeServer::start(|req: &Req| match (req.method.as_str(), req.path.as_str()) {
                ("GET", "/api/version") => {
                    Reply::json(200, serde_json::json!({"version": "0.40.11"}))
                }
                ("GET", "/api/tags") => Reply::json(
                    200,
                    serde_json::json!({"models": [{"name": "phonon:latest"}]}),
                ),
                ("POST", "/v1/audio/speech") => speech_reply(req, Duration::from_millis(1)),
                _ => Reply::json(404, serde_json::json!({"error": "not found"})),
            })
            .await;
        let host = ProviderHost::new(None);
        let mut s = TtsProvidersSettings::default();
        s.ollama_speech.base_url = server.root.clone();
        s.ollama_speech.probe_speech = true;
        host.sync(&resolved(&s), &paths());

        let health = || async {
            host.registry()
                .health("ollama-speech", Duration::from_secs(4))
                .await
                .unwrap()
        };
        let h = health().await;
        assert!(!h.available, "{h:?}");
        assert_eq!(
            server.count("/v1/audio/speech"),
            0,
            "ohne Freigabe lädt die Probe nichts in Ollama"
        );

        let provider = host.registry().get("ollama-speech").unwrap();
        let mut approved = TtsProvidersSettings::default();
        approved.set_enabled(OLLAMA_ID, true);
        approved.record_ack(
            OLLAMA_ID,
            &license_fingerprint(&provider.descriptor(), &provider.use_subject()),
            "2026-10-26T00:00:00Z",
        );
        host.approvals().set(approved);
        let h = health().await;
        assert!(h.available, "{h:?}");
        assert_eq!(server.count("/v1/audio/speech"), 1);
    }

    #[test]
    fn sync_baut_nur_bei_aenderung_neu() {
        let host = ProviderHost::new(None);
        let s = TtsProvidersSettings::default();
        assert!(host.sync(&resolved(&s), &paths()), "erster Aufruf baut");
        assert!(
            !host.sync(&resolved(&s), &paths()),
            "unverändert: nichts tun"
        );
        let mut s2 = s.clone();
        s2.openai_speech.model = "anderes".into();
        assert!(host.sync(&resolved(&s2), &paths()));
        assert!(host
            .registry()
            .get("openai-speech")
            .unwrap()
            .cache_tag(None)
            .contains("anderes"));
        // Auch ein geänderter Pfad zählt.
        let mut p = paths();
        p.piper_voice = Some("de_DE-thorsten-high".into());
        assert!(host.sync(&resolved(&s2), &p));
        assert_eq!(
            host.registry().get("piper").unwrap().cache_tag(None),
            "piper/de_DE-thorsten-high".to_string()
        );
    }

    #[test]
    fn eine_geaenderte_openai_kennung_tauscht_den_eintrag_aus() {
        let host = ProviderHost::new(None);
        let mut s = TtsProvidersSettings::default();
        s.openai_speech.id = "kokoro".into();
        host.sync(&resolved(&s), &paths());
        assert!(host.registry().get("kokoro").is_some());
        assert!(host.registry().get("openai-speech").is_none());
        s.openai_speech.id = "openai-speech".into();
        host.sync(&resolved(&s), &paths());
        assert!(host.registry().get("openai-speech").is_some());
        assert!(host.registry().get("kokoro").is_none(), "kein Rest");
        assert_eq!(host.registry().list().len(), 5);
    }

    /// Nebenläufigkeit: der Neuaufbau während eines laufenden Auftrags lässt
    /// dessen Eintrag stehen, und der Auftrag lässt sich weiter abbrechen.
    #[tokio::test]
    async fn ein_neuaufbau_waehrend_eines_auftrags_laesst_ihn_abbrechbar() {
        let host = ProviderHost::new(None);
        let s = TtsProvidersSettings::default();
        host.sync(&resolved(&s), &paths());
        let reg = host.registry().clone();
        // Ein Testanbieter mit langer Latenz läuft unter einer Kennung.
        reg.register(Arc::new(
            MockProvider::new()
                .with_id("langsam")
                .latency(Duration::from_secs(30)),
        ))
        .unwrap();
        let reg2 = reg.clone();
        let job = tokio::spawn(async move {
            let opts = SynthesisOptions {
                timeout: Duration::from_secs(60),
                ..Default::default()
            };
            reg2.synthesize("langsam", "auftrag-1", "Hallo Welt", &opts)
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(reg.active_requests(), 1);
        // Einstellungsänderung → Neuaufbau mitten im Auftrag.
        let mut s2 = s.clone();
        s2.openai_speech.model = "neu".into();
        assert!(host.sync(&resolved(&s2), &paths()));
        assert_eq!(reg.active_requests(), 1, "die Auftragstabelle bleibt");
        assert!(reg.get("langsam").is_some(), "fremde Testanbieter bleiben");
        assert!(reg.cancel("auftrag-1"), "der Auftrag ist weiter abbrechbar");
        let r = job.await.unwrap();
        assert_eq!(
            r.unwrap_err().kind,
            crate::managers::tts::providers::contract::TtsErrorKind::Cancelled
        );
        assert_eq!(reg.active_requests(), 0);
    }
}
