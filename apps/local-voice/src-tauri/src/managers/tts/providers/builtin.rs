//! Hüllen um die beiden vorhandenen Engines: [`FishProvider`] und
//! [`PiperProvider`]. Sie liefern Deskriptor, Fähigkeiten, Lizenz, Gesundheit
//! und Stimmen aus den VORHANDENEN Funktionen (`availability`, `voices`,
//! `piper`) und delegieren die Synthese — der Engine-Code selbst bleibt
//! unverändert. Streaming beider: `Sentence` (ein Chunk je Satz).
//!
//! ## Warum Fish die Synthese hereingereicht bekommt
//!
//! Die Fish-Synthese ist `TtsCore::fish_synthesize` und braucht HTTP-Client,
//! Port und einen laufenden Server (`ensure_server`). Hielte die Hülle einen
//! `Arc<TtsCore>`, entstünde genau die Besitz-Schleife, vor der `engine.rs`
//! warnt (der Kern hält später den Anbieter, der Anbieter den Kern). Darum
//! bekommt [`FishProvider`] die Synthese als [`SynthFn`] von außen (Paket P3
//! reicht eine Closure herein, die den Server sicherstellt und
//! `fish_synthesize` ruft; ein `Weak<TtsCore>` genügt). Ohne Closure endet
//! `synthesize` mit `InvalidConfig`. Piper braucht keinen Kern: der
//! Subprozess-Lauf `piper::run_piper_blocking` ist eine freie Funktion und
//! wird direkt gerufen.
//!
//! ## Lizenzangaben
//!
//! `verified` ist nur dort `true`, wo `docs/tts/provider-research.md` Quelle
//! UND Abrufdatum nennt — oder wo die lokal installierte Lizenzdatei den Beleg
//! liefert (Fish: `LICENSE` und `checkpoints/s2-pro/LICENSE.md`, Fish Audio
//! Research License vom 07.03.2026, nichtkommerziell, `verified_on` 2026-10-26).
//! Piper steht auf `false`: dort hängt die Lizenz je Stimme (siehe `note`).
//! Beide sind keine Opt-in-Anbieter (`opt_in = false`) — auch Fish mit
//! nichtkommerzieller Lizenz bleibt frei, es ist der bestehende Standard; eine Lizenz-Sperre
//! (`license_ack`) darf deshalb nicht allein an `!verified` hängen, sondern
//! nur an `opt_in && (non_commercial || !verified)`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;

use super::contract::{
    guarded, language_matches, AudioClip, AudioEncoding, CancelToken, Device, LicenseInfo,
    ProviderCaps, ProviderDescriptor, ProviderHealth, ProviderTier, StreamingMode,
    SynthesisOptions, TtsError, TtsProvider, VoiceDescriptor,
};
use crate::catalog::{self, Purpose};
use crate::managers::tts::engine::{self, TtsEngine};
use crate::managers::tts::piper::{self, PiperEngine};
use crate::managers::tts::{availability, voices};

/// Von außen hereingereichte Synthese (siehe Moduldoku): Text, Optionen und
/// Abbruchmarke rein, WAV-Clip raus.
pub type SynthFn = Arc<
    dyn Fn(String, SynthesisOptions, CancelToken) -> BoxFuture<'static, Result<AudioClip, TtsError>>
        + Send
        + Sync,
>;

/// Wie lange der Piper-Prozess über die Frist des Auftrags hinaus weiterlaufen
/// darf: `run_piper_blocking` läuft auf einem Blocking-Thread, der sich von
/// außen nicht abbrechen lässt. Die eigene Frist des Laufs (Frist + Nachlauf)
/// beendet ihn.
const PIPER_KILL_GRACE: Duration = Duration::from_secs(2);

/// Hülle um Fish Speech (externer Python-HTTP-Server, GPU).
pub struct FishProvider {
    fish_dir: PathBuf,
    platform: String,
    synth: Option<SynthFn>,
}

impl FishProvider {
    pub fn new(fish_dir: impl Into<PathBuf>) -> Self {
        Self {
            fish_dir: fish_dir.into(),
            platform: availability::current_platform_id().to_string(),
            synth: None,
        }
    }

    /// Synthese hereinreichen (siehe Moduldoku).
    pub fn with_synth(mut self, synth: SynthFn) -> Self {
        self.synth = Some(synth);
        self
    }

    /// Plattform überschreiben (Tests).
    #[cfg(test)]
    pub(crate) fn with_platform(mut self, platform: &str) -> Self {
        self.platform = platform.to_string();
        self
    }

    /// Warum Fish nicht startfähig ist: (Grund, Hinweis); `None` = bereit.
    fn problem(&self) -> Option<(String, String)> {
        if !availability::fish_supported(&self.platform) {
            return Some((
                "Fish Speech wird auf diesem System nicht unterstützt".to_string(),
                "Unter Modelle > Vorlesestimmen lässt sich stattdessen Piper einrichten."
                    .to_string(),
            ));
        }
        let missing = availability::fish_missing(&self.fish_dir, &self.platform);
        if missing.is_empty() {
            return None;
        }
        Some((
            format!(
                "Fish Speech ist nicht eingerichtet (im Ordner '{}' fehlt: {})",
                self.fish_dir.display(),
                missing.join(", ")
            ),
            "Fish Speech einrichten oder unter Modelle > Vorlesestimmen Piper wählen.".to_string(),
        ))
    }
}

impl TtsProvider for FishProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "fish".to_string(),
            label: "Fish Speech".to_string(),
            tier: ProviderTier::Supported,
            opt_in: false,
            caps: ProviderCaps {
                // Mehrsprachig; eine feste Sprachliste führt der Kern nicht.
                languages: vec!["und".to_string()],
                streaming: StreamingMode::Sentence,
                formats: vec![AudioEncoding::Wav],
                // Annahme des Kerns, wenn der Header keine Rate nennt (mod.rs).
                sample_rates: vec![44_100],
                cloning: engine::FISH_CAPS.cloning,
                devices: vec![Device::Cuda],
                platforms: ["windows-x64", "macos-aarch64", "linux-x64"]
                    .map(String::from)
                    .to_vec(),
                style_tags: engine::FISH_CAPS.style_tags,
            },
            license: LicenseInfo {
                code: "Fish Audio Research License (installierte Fassung, Stand 07.03.2026)"
                    .to_string(),
                weights: "s2-pro: Fish Audio Research License".to_string(),
                non_commercial: true,
                verified: true,
                source_url: "https://huggingface.co/fishaudio/s2-pro".to_string(),
                verified_on: Some("2026-10-26".to_string()),
                note: "Belegt durch die lokal installierte LICENSE (Repo-Code) und \
                       checkpoints/s2-pro/LICENSE.md (Gewichte): beide stehen unter der Fish \
                       Audio Research License, zuletzt geändert am 07.03.2026; die Model Card \
                       nennt license: other / fish-audio-research-license. Erlaubt sind \
                       Forschung und nichtkommerzielle Nutzung; kommerzielle Nutzung nur mit \
                       separater Lizenz von Fish Audio. Fish bleibt der bestehende Standard \
                       und wird deshalb nicht gesperrt (kein Opt-in)."
                    .to_string(),
            },
        }
    }

    fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
        Box::pin(async move {
            // Nur Dateisystem: ob der Server schon läuft, ist Sache des
            // Managers — er startet ihn beim ersten Vorlesen selbst.
            let mut health = match self.problem() {
                None => ProviderHealth::ok(),
                Some((reason, hint)) => ProviderHealth::unavailable(reason, Some(hint)),
            };
            health.device = Some(Device::Cuda);
            health
        })
    }

    fn list_voices<'a>(
        &'a self,
        language: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
        Box::pin(async move {
            Ok(voices::list_voices(&self.fish_dir)
                .into_iter()
                // Geklonte Stimmen sind sprachunabhängig (`und`).
                .filter(|_| language_matches("und", language))
                .map(|id| VoiceDescriptor {
                    label: id.clone(),
                    id,
                    language: "und".to_string(),
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
            if let Some((reason, _)) = self.problem() {
                return Err(TtsError::unavailable(reason));
            }
            if text.trim().is_empty() {
                return Err(TtsError::bad_request("Leerer Text"));
            }
            let Some(synth) = &self.synth else {
                return Err(TtsError::invalid_config(
                    "Fish: keine Synthese angebunden (Aufruf über den TtsManager fehlt)",
                ));
            };
            guarded(
                cancel,
                opts.timeout,
                "Fish",
                synth(text.to_string(), opts.clone(), cancel.clone()),
            )
            .await
        })
    }

    /// Fish behält den leeren Tag (Legacy-Regel, bestehende Caches bleiben gültig).
    fn cache_tag(&self, voice: Option<&str>) -> String {
        engine::fish_cache_tag(voice)
    }
}

/// Hülle um Piper (CPU-Subprozess je Satz).
pub struct PiperProvider {
    app_data: Option<PathBuf>,
    voice: Option<String>,
    auto_language: bool,
}

impl PiperProvider {
    /// `app_data`: Datenordner der App (dort liegt `tts/piper/…`); `None`
    /// heißt „noch nicht bekannt“ und ergibt „Programm fehlt“.
    pub fn new(app_data: Option<PathBuf>) -> Self {
        Self {
            app_data,
            voice: None,
            auto_language: false,
        }
    }

    /// Gewählte Stimme (`tts_piper_voice`).
    pub fn with_voice(mut self, voice: Option<String>) -> Self {
        self.voice = voice;
        self
    }

    /// Sprache je Satz erkennen (`tts_piper_auto_language`).
    pub fn with_auto_language(mut self, auto: bool) -> Self {
        self.auto_language = auto;
        self
    }

    fn engine_for(&self, voice: Option<&str>) -> PiperEngine {
        PiperEngine::resolve_auto(
            self.app_data.as_deref(),
            voice.or(self.voice.as_deref()),
            self.auto_language,
        )
    }

    /// Warum Piper nicht einsatzbereit ist (Grund, Hinweis); `None` = bereit.
    fn problem(&self, voice: Option<&str>) -> Option<(String, String)> {
        if !availability::piper_supported(piper::platform_subdir()) {
            return Some((
                "Piper wird auf diesem System nicht unterstützt".to_string(),
                "Unter Modelle > Vorlesestimmen gibt es hier keine Piper-Laufzeit.".to_string(),
            ));
        }
        self.engine_for(voice).unavailable_reason().map(|reason| {
            (
                reason.to_string(),
                "Unter Modelle > Vorlesestimmen Piper und eine Stimme laden.".to_string(),
            )
        })
    }
}

/// Lizenz einer Piper-Stimme aus dem App-Katalog (`catalog.json`, Prüfung K1);
/// unbekannte Stimmen haben keine eigene Angabe. Nie `verified`: der Katalog
/// nennt weder Prüfdatum noch eine unabhängige Quelle.
fn piper_voice_license(voice_id: &str) -> Option<LicenseInfo> {
    let entry = catalog::tts_entries(Purpose::TtsVoice)
        .into_iter()
        .find(|e| e.id == voice_id)?;
    Some(LicenseInfo {
        code: "GPL-3.0".to_string(),
        weights: entry.license.unwrap_or_else(|| "unbekannt".to_string()),
        non_commercial: entry.license_non_commercial,
        verified: false,
        source_url: entry.license_url.unwrap_or_default(),
        verified_on: None,
        note: "Lizenz laut App-Katalog (nicht unabhängig geprüft); bei kommerzieller Nutzung \
               Model Card der Stimme prüfen."
            .to_string(),
    })
}

impl TtsProvider for PiperProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        // Sprachen: alles, wofür der Katalog eine Stimme anbietet.
        let mut languages: Vec<String> = catalog::tts_entries(Purpose::TtsVoice)
            .iter()
            .filter_map(|e| piper::language_of_voice_id(&e.id))
            .collect();
        languages.sort();
        languages.dedup();
        if languages.is_empty() {
            languages = vec!["de".to_string(), "en".to_string()];
        }
        ProviderDescriptor {
            id: "piper".to_string(),
            label: "Piper".to_string(),
            tier: ProviderTier::Supported,
            opt_in: false,
            caps: ProviderCaps {
                languages,
                streaming: StreamingMode::Sentence,
                formats: vec![AudioEncoding::Wav],
                // x_low/low-Stimmen 16 kHz, medium/high 22,05 kHz.
                sample_rates: vec![16_000, 22_050],
                cloning: piper::PIPER_CAPS.cloning,
                devices: vec![Device::Cpu],
                platforms: ["windows-x64", "macos-x64", "macos-aarch64"]
                    .map(String::from)
                    .to_vec(),
                style_tags: piper::PIPER_CAPS.style_tags,
            },
            license: LicenseInfo {
                code: "GPL-3.0".to_string(),
                weights: "je Stimme (siehe Stimmenliste)".to_string(),
                non_commercial: false,
                verified: false,
                source_url: "https://github.com/OHF-Voice/piper1-gpl".to_string(),
                verified_on: None,
                note: "Code-Lizenz GPL-3.0 (provider-research.md, 26.10.2026); läuft als \
                       separater Prozess, die Auswirkung auf die App-Lizenz ist juristisch \
                       offen. Stimmenlizenzen gelten je Stimme und sind nicht alle geprüft."
                    .to_string(),
            },
        }
    }

    fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
        Box::pin(async move {
            let mut health = match self.problem(None) {
                None => ProviderHealth::ok(),
                Some((reason, hint)) => ProviderHealth::unavailable(reason, Some(hint)),
            };
            health.device = Some(Device::Cpu);
            health
        })
    }

    fn list_voices<'a>(
        &'a self,
        language: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
        Box::pin(async move {
            let Some(app_data) = &self.app_data else {
                return Ok(Vec::new());
            };
            let catalog_names: std::collections::HashMap<String, String> =
                catalog::tts_entries(Purpose::TtsVoice)
                    .into_iter()
                    .map(|e| (e.id, e.name))
                    .collect();
            Ok(piper::installed_voices(app_data)
                .into_iter()
                .filter(|(_, lang, _)| language_matches(lang, language))
                .map(|(id, lang, _)| VoiceDescriptor {
                    label: catalog_names
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| id.clone()),
                    license: piper_voice_license(&id),
                    id,
                    language: lang,
                    gender: None,
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
            if let Some((reason, _)) = self.problem(opts.voice.as_deref()) {
                return Err(TtsError::unavailable(reason));
            }
            if text.trim().is_empty() {
                return Err(TtsError::bad_request("Leerer Text"));
            }
            let paths = self
                .engine_for(opts.voice.as_deref())
                .paths_for(text)
                .map_err(TtsError::unavailable)?;
            let (owned_text, speed) = (text.to_string(), opts.speed);
            let kill_after = opts.timeout + PIPER_KILL_GRACE;
            let job = tokio::task::spawn_blocking(move || {
                piper::run_piper_blocking(&paths, &owned_text, speed, kill_after)
            });
            let wav = guarded(cancel, opts.timeout, "Piper", async {
                match job.await {
                    Ok(Ok(bytes)) => Ok(bytes),
                    // Startfehler des Programms ist „nicht verfügbar“, jeder
                    // andere Fehler (Exit-Code, kein WAV) ein Backend-Fehler.
                    Ok(Err(msg)) if msg.starts_with(piper::ERR_BINARY_MISSING) => {
                        Err(TtsError::unavailable(msg))
                    }
                    Ok(Err(msg)) => Err(TtsError::backend(msg)),
                    Err(e) => Err(TtsError::backend(format!("Piper-Lauf abgebrochen: {e}"))),
                }
            })
            .await?;
            AudioClip::from_wav(wav)
        })
    }

    /// Wie `PiperEngine::cache_tag`: `"piper/<stimme>"` — bestehende Caches
    /// bleiben gültig.
    fn cache_tag(&self, voice: Option<&str>) -> String {
        self.engine_for(voice).cache_tag(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::contract::TtsErrorKind;
    use crate::managers::tts::providers::contract_tests::{contract_suite, ContractExpect};
    use crate::managers::tts::providers::registry::ProviderRegistry;
    use std::path::Path;
    use tempfile::TempDir;

    fn touch(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// Vollständige (falsche, aber vorhandene) Fish-Installation.
    fn fish_fixture(dir: &Path) {
        touch(&availability::fish_python(dir, "windows-x64"), b"fixture");
        touch(&dir.join("tools").join("api_server.py"), b"fixture");
        touch(&dir.join("checkpoints").join("model.bin"), b"fixture");
    }

    /// Fish-Hülle mit hereingereichter Fake-Synthese (`delay` bis zum Ergebnis).
    fn fish_with_fake(dir: &Path, delay: Duration) -> FishProvider {
        let synth: SynthFn = Arc::new(move |text, _opts, _cancel| {
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                // 50 ms je Zeichen Stille bei 44,1 kHz.
                let frames = 44_100 * text.chars().count() * 50 / 1000;
                AudioClip::from_pcm(AudioEncoding::Pcm16Le, 44_100, 1, &vec![0u8; frames * 2])
            })
        });
        FishProvider::new(dir)
            .with_platform("windows-x64")
            .with_synth(synth)
    }

    #[tokio::test]
    async fn fish_ohne_installation_besteht_den_vertrag_als_nicht_verfuegbar() {
        let dir = TempDir::new().unwrap();
        let fish = FishProvider::new(dir.path().join("gibt-es-nicht"));
        let health = fish.health_check().await;
        assert!(!health.available);
        assert_eq!(health.device, Some(Device::Cuda));
        let reason = health.reason.clone().unwrap();
        assert!(reason.contains("nicht eingerichtet") || reason.contains("nicht unterstützt"));
        contract_suite(Arc::new(fish), ContractExpect::unavailable()).await;
    }

    #[tokio::test]
    async fn fish_auf_unbekannter_plattform_meldet_nicht_unterstuetzt() {
        let dir = TempDir::new().unwrap();
        fish_fixture(dir.path());
        let fish = FishProvider::new(dir.path()).with_platform("windows-arm64");
        let h = fish.health_check().await;
        assert!(!h.available);
        assert!(h.reason.unwrap().contains("nicht unterstützt"));
    }

    #[tokio::test]
    async fn fish_mit_installation_und_fake_synthese_besteht_den_ganzen_vertrag() {
        let dir = TempDir::new().unwrap();
        fish_fixture(dir.path());
        // Eine Stimme mit WAV + .lab (Gültigkeitsregel von voices::list_voices).
        let voice = voices::voice_dir(dir.path(), "patrick");
        touch(&voice.join("a.wav"), b"x");
        touch(&voice.join("a.lab"), b"x");

        let fish = Arc::new(fish_with_fake(dir.path(), Duration::ZERO));
        assert!(fish.health_check().await.available);
        let v = fish.list_voices(Some("de")).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(
            (v[0].id.as_str(), v[0].language.as_str()),
            ("patrick", "und")
        );

        contract_suite(
            fish,
            ContractExpect::available()
                .with_sample_rate(44_100)
                .with_slow(Arc::new(fish_with_fake(
                    dir.path(),
                    Duration::from_secs(20),
                ))),
        )
        .await;
    }

    #[tokio::test]
    async fn fish_ohne_angebundene_synthese_ist_ein_konfigurationsfehler() {
        let dir = TempDir::new().unwrap();
        fish_fixture(dir.path());
        let fish = FishProvider::new(dir.path()).with_platform("windows-x64");
        let e = fish
            .synthesize(
                "Hallo",
                &SynthesisOptions::default(),
                &CancelToken::detached(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::InvalidConfig);
    }

    #[test]
    fn fish_beschreibung_ist_ehrlich_ueber_lizenz_und_streaming() {
        let d = FishProvider::new("x").descriptor();
        assert_eq!(d.id, "fish");
        assert_eq!(d.tier, ProviderTier::Supported);
        assert!(!d.opt_in);
        assert_eq!(d.caps.streaming, StreamingMode::Sentence);
        assert!(d.caps.cloning && d.caps.style_tags);
        assert_eq!(d.caps.devices, vec![Device::Cuda]);
        // Lizenz belegt (Befund 11): Fish Audio Research License, nichtkommerziell,
        // lokale LICENSE/LICENSE.md und Model Card als Quelle.
        let l = &d.license;
        assert!(
            l.non_commercial,
            "Fish-Lizenz erlaubt nur Forschung/nichtkommerziell"
        );
        assert!(l.verified);
        assert_eq!(l.verified_on.as_deref(), Some("2026-10-26"));
        assert_eq!(l.source_url, "https://huggingface.co/fishaudio/s2-pro");
        assert!(l.code.contains("Fish Audio Research License"), "{}", l.code);
        assert!(!l.code.contains("Apache"), "{}", l.code);
        assert!(
            l.weights.contains("Fish Audio Research License"),
            "{}",
            l.weights
        );
        assert!(l.note.contains("LICENSE.md"), "{}", l.note);
        assert!(l.note.contains("separate"), "{}", l.note);
        // Legacy-Regel: leerer Cache-Tag, egal welche Stimme.
        assert_eq!(FishProvider::new("x").cache_tag(Some("patrick")), "");
    }

    /// Die korrigierte Lizenz sperrt Fish NICHT: Fish ist der bestehende Standard
    /// (`opt_in = false`); über eine Sperre entscheidet der Owner, nicht diese Angabe.
    #[test]
    fn die_nichtkommerzielle_fish_lizenz_sperrt_fish_nicht() {
        use crate::managers::tts::providers::policy::{
            check_use, license_needs_ack, use_state, TtsProvidersSettings, UseSubject,
        };
        let d = FishProvider::new("x").descriptor();
        assert!(d.license.non_commercial && !d.opt_in);
        assert_eq!(d.tier, ProviderTier::Supported);
        assert!(
            !license_needs_ack(&d),
            "keine Zustimmung nötig, kein Opt-in"
        );
        let settings = TtsProvidersSettings::default();
        assert!(check_use(&d, &UseSubject::default(), &settings).is_ok());
        let state = use_state(&d, &UseSubject::default(), &settings);
        assert!(state.enabled && !state.license_needs_ack && state.blocked.is_none());
    }

    // ---- Piper ------------------------------------------------------------

    fn piper_dir(app_data: &Path) -> PathBuf {
        app_data
            .join("tts")
            .join("piper")
            .join(piper::platform_subdir())
    }

    fn piper_voice_files(app_data: &Path, id: &str) {
        let (model, config) = piper::piper_voice_paths(app_data, id);
        touch(&model, b"fixture");
        touch(&config, b"{}");
    }

    #[tokio::test]
    async fn piper_ohne_laufzeit_besteht_den_vertrag_als_nicht_verfuegbar() {
        let dir = TempDir::new().unwrap();
        for app_data in [None, Some(dir.path().to_path_buf())] {
            let p = PiperProvider::new(app_data);
            let h = p.health_check().await;
            assert!(!h.available);
            assert_eq!(h.device, Some(Device::Cpu));
            assert!(h.reason.is_some() && h.hint.is_some());
            contract_suite(Arc::new(p), ContractExpect::unavailable()).await;
        }
        // Konkreter Grund: Programm fehlt.
        let p = PiperProvider::new(Some(dir.path().to_path_buf()));
        if availability::piper_supported(piper::platform_subdir()) {
            assert_eq!(
                p.health_check().await.reason.as_deref(),
                Some(piper::ERR_BINARY_MISSING)
            );
        }
    }

    #[tokio::test]
    async fn piper_mit_laufzeit_aber_ohne_stimme_meldet_stimme_fehlt() {
        if !availability::piper_supported(piper::platform_subdir()) {
            return;
        }
        let dir = TempDir::new().unwrap();
        availability::write_piper_fixture(&piper_dir(dir.path()), piper::platform_subdir());
        let p = PiperProvider::new(Some(dir.path().to_path_buf()));
        assert_eq!(
            p.health_check().await.reason.as_deref(),
            Some(piper::ERR_VOICE_MISSING)
        );

        // Mit Stimme: bereit; Stimmenliste mit Sprache und Katalog-Lizenz.
        piper_voice_files(dir.path(), "de_DE-thorsten-high");
        piper_voice_files(dir.path(), "en_US-ryan-high");
        let p = PiperProvider::new(Some(dir.path().to_path_buf()))
            .with_voice(Some("de_DE-thorsten-high".to_string()));
        assert!(p.health_check().await.available);
        let all = p.list_voices(None).await.unwrap();
        assert_eq!(all.len(), 2);
        let de = p.list_voices(Some("de_DE")).await.unwrap();
        assert_eq!(de.len(), 1);
        assert_eq!(de[0].id, "de_DE-thorsten-high");
        assert_eq!(de[0].language, "de");
        let lic = de[0].license.clone().expect("Katalog kennt thorsten");
        assert!(!lic.verified && lic.verified_on.is_none());
        assert!(
            lic.non_commercial,
            "Katalog (K1) führt thorsten als nicht-kommerziell"
        );
        assert!(!lic.note.is_empty());
        // Cache-Tag: identisch zur Engine (bestehende Caches bleiben gültig).
        assert_eq!(
            p.cache_tag(Some("de_DE-thorsten-high")),
            "piper/de_DE-thorsten-high"
        );
        assert_eq!(
            p.cache_tag(Some("en_US-ryan-high")),
            PiperEngine::resolve(Some(dir.path()), Some("en_US-ryan-high")).cache_tag(None)
        );
        // Falsches Programm im Fixture: Start schlägt fehl -> Unavailable, kein Hänger.
        let e = p
            .synthesize(
                "Hallo Welt",
                &SynthesisOptions::default(),
                &CancelToken::detached(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(e.kind, TtsErrorKind::Unavailable | TtsErrorKind::Backend),
            "{e:?}"
        );
    }

    #[test]
    fn piper_beschreibung_nennt_cpu_sentence_und_ungeprueften_lizenzstand() {
        let d = PiperProvider::new(None).descriptor();
        assert_eq!(d.id, "piper");
        assert_eq!(d.tier, ProviderTier::Supported);
        assert!(!d.opt_in);
        assert_eq!(d.caps.streaming, StreamingMode::Sentence);
        assert_eq!(d.caps.devices, vec![Device::Cpu]);
        assert!(!d.caps.cloning && !d.caps.style_tags);
        assert!(d.caps.languages.iter().any(|l| l == "de"));
        assert!(!d.license.verified);
        assert!(d.license.code.contains("GPL"));
        assert!(d.license.source_url.contains("piper1-gpl"));
    }

    #[test]
    fn piper_stimmenlizenz_unbekannter_stimmen_ist_leer() {
        assert!(piper_voice_license("xx_XX-gibtsnicht-low").is_none());
        let l = piper_voice_license("en_US-ryan-high").unwrap();
        assert!(l.non_commercial);
    }

    #[tokio::test]
    async fn beide_huellen_lassen_sich_in_der_registry_zusammen_verwalten() {
        let dir = TempDir::new().unwrap();
        let reg = ProviderRegistry::new();
        reg.register(Arc::new(FishProvider::new(dir.path().join("fish"))))
            .unwrap();
        reg.register(Arc::new(PiperProvider::new(Some(dir.path().to_path_buf()))))
            .unwrap();
        let ids: Vec<_> = reg.descriptors().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, ["fish", "piper"]);
        let health = reg.health_all(Duration::from_secs(2)).await;
        assert_eq!(health.len(), 2);
        assert!(health
            .iter()
            .all(|(_, h)| !h.available && h.reason.is_some()));
    }
}
