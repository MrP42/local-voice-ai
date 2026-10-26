//! Der `TtsManager` als Gastgeber der Anbieterschicht (Goal tts-provider,
//! Paket P3a): Fish-Synthese für die Registry, Momentaufnahme für die
//! Tauri-Befehle, Probestimme.
//!
//! Dieses Modul liegt unter `managers::tts` und darf deshalb die privaten
//! Felder des Managers lesen; die Regeln selbst stehen in den reinen Modulen
//! ([`super::policy`], [`super::setup`], [`super::pipeline`],
//! [`super::preview`]).
//!
//! **Bewusst ohne Tests in diesem Modul:** `fish_synth` und
//! `fish_provider_synthesize` erreichen `settings::get_settings(&AppHandle)`.
//! Sobald ein Test sie auch nur referenziert, bindet der Linker diesen Pfad in
//! die Test-Exe ein, und sie startet nicht mehr (`0xc0000139`,
//! STATUS_ENTRYPOINT_NOT_FOUND). Die Regeln stehen deshalb in den reinen
//! Modulen und sind dort getestet; die Fish-Hülle selbst ist in `builtin`
//! mit einer Fake-`SynthFn` geprüft.

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use tauri_specta::Event;

use super::builtin::SynthFn;
use super::contract::{AudioClip, CancelToken, ProviderDescriptor, SynthesisOptions, TtsError};
use super::policy::{
    check_use, process_env, resolve_provider_configs, use_state, ProviderOverrides,
    ResolvedConfigs, TtsProvidersSettings, UseState, FISH_ID, PIPER_ID,
};
use super::preview::{PreviewRequest, PreviewService, TtsProviderPreview};
use super::registry::ProviderRegistry;
use super::setup::ProviderPaths;
use crate::managers::tts::engine::{self, TtsEngineKind};
use crate::managers::tts::state::TtsPhase;
use crate::managers::tts::{TtsCore, TtsManager};

/// Die Synthese der Fish-Hülle: Server sicherstellen, dann der BESTEHENDE
/// HTTP-Pfad des Kerns. Der schwache Verweis hält den Manager nicht am Leben
/// (und die Registry, die der Manager besitzt, keinen Kreis).
pub fn fish_synth(manager: Weak<TtsManager>) -> SynthFn {
    Arc::new(move |text, opts, cancel| {
        let manager = manager.clone();
        Box::pin(async move {
            let manager = manager
                .upgrade()
                .ok_or_else(|| TtsError::unavailable("Die App wird beendet"))?;
            manager
                .fish_provider_synthesize(&text, &opts, &cancel)
                .await
        })
    })
}

/// Probestimme: Player und Regler des Kerns, Ereignis an die Oberfläche.
pub fn preview_service(
    app: &tauri::AppHandle,
    core: &Arc<TtsCore>,
    registry: &Arc<ProviderRegistry>,
) -> PreviewService {
    let app = app.clone();
    PreviewService::new(
        registry.clone(),
        core.player.clone(),
        core.controls.clone(),
        Arc::new(move |event: TtsProviderPreview| {
            if let Err(e) = event.emit(&app) {
                log::warn!("Ereignis tts-provider-preview nicht gesendet: {e}");
            }
        }),
    )
}

/// Momentaufnahme für die Befehle: Einstellungen, wirksame Konfiguration,
/// Registry und gewählter Anbieter — alles aus EINEM Lesen der Einstellungen.
pub struct ProviderSnapshot {
    pub settings: Arc<TtsProvidersSettings>,
    pub resolved: Arc<ResolvedConfigs>,
    pub registry: Arc<ProviderRegistry>,
    /// Kennung des gewählten Anbieters: `fish`, `piper` oder `<id>` aus `provider:<id>`.
    pub selected: String,
    /// Gewählte Stimme der eingebauten Engines (`tts_voice`, `tts_piper_voice`).
    pub fish_voice: Option<String>,
    pub piper_voice: Option<String>,
}

impl ProviderSnapshot {
    pub fn descriptor(&self, id: &str) -> Result<ProviderDescriptor, TtsError> {
        self.registry
            .get(id)
            .map(|p| p.descriptor())
            .ok_or_else(|| TtsError::bad_request(format!("Unbekannter Anbieter '{id}'")))
    }

    /// Freigabezustand von `desc` unter der wirksamen Konfiguration.
    pub fn use_state(&self, desc: &ProviderDescriptor) -> UseState {
        use_state(desc, &self.resolved.subject(&desc.id), &self.settings)
    }

    /// Anbieter nachschlagen UND die Guardrails prüfen ([`check_use`]).
    pub fn checked(&self, id: &str) -> Result<ProviderDescriptor, TtsError> {
        let desc = self.descriptor(id)?;
        check_use(&desc, &self.resolved.subject(id), &self.settings)?;
        Ok(desc)
    }

    /// Die gewählte Stimme eines Anbieters (für die Anzeige und die Probe).
    pub fn voice_of(&self, id: &str) -> Option<String> {
        match id {
            "fish" => self.fish_voice.clone(),
            "piper" => self.piper_voice.clone(),
            _ => self.resolved.voice_for(id),
        }
    }
}

impl TtsManager {
    /// Registry auf den Stand der Einstellungen bringen und die wirksame
    /// Konfiguration liefern (Umgebung eingerechnet, keine Overrides — die
    /// gibt es nur für die CLI, Paket P3b). Billig, wenn nichts geändert ist.
    pub(crate) fn sync_providers(
        &self,
        providers: &TtsProvidersSettings,
        fish_dir: &str,
        piper_voice: Option<&str>,
        piper_auto_language: bool,
    ) -> Arc<ResolvedConfigs> {
        let resolved = Arc::new(resolve_provider_configs(
            providers,
            &process_env,
            &ProviderOverrides::default(),
        ));
        let paths = ProviderPaths {
            fish_dir: PathBuf::from(fish_dir),
            app_data: self.data_base_dir(),
            piper_voice: piper_voice.map(String::from),
            piper_auto_language,
        };
        self.provider_host.sync(&resolved, &paths);
        resolved
    }

    /// Momentaufnahme für die Befehle. Liest die Einstellungen einmal und
    /// gleicht die Registry ab; verändert sonst nichts am Kern (ein
    /// Engine-Wechsel geschieht erst in `refresh_from_settings`).
    pub(crate) fn provider_snapshot(&self) -> ProviderSnapshot {
        let settings = crate::settings::get_settings(&self.app);
        let resolved = self.sync_providers(
            &settings.tts_providers,
            &settings.tts_fish_dir,
            settings.tts_piper_voice.as_deref(),
            settings.tts_piper_auto_language,
        );
        let selected = match engine::provider_id_from_setting(&settings.tts_engine) {
            Some(id) => id,
            None => match TtsEngineKind::from_setting(&settings.tts_engine) {
                TtsEngineKind::Piper => PIPER_ID.to_string(),
                _ => FISH_ID.to_string(),
            },
        };
        ProviderSnapshot {
            settings: Arc::new(settings.tts_providers.clone()),
            resolved,
            registry: self.provider_host.registry().clone(),
            selected,
            fish_voice: settings.tts_voice.clone(),
            piper_voice: settings.tts_piper_voice.clone(),
        }
    }

    /// Probestimme starten. Entscheidung (dokumentiert): läuft das Vorlesen,
    /// wird sie ABGELEHNT — zwei gleichzeitige Audioströme sind nie gewollt,
    /// und das Vorlesen soll der Nutzer selbst anhalten. Umgekehrt beendet ein
    /// startendes Vorlesen eine laufende Probestimme
    /// (`run_speak_session`). Die Probestimme berührt Phase, Generation und
    /// Cache des Vorlesens nicht.
    pub(crate) fn start_provider_preview(
        &self,
        id: &str,
        text: &str,
        voice: Option<String>,
    ) -> Result<String, TtsError> {
        if self.core.phase() == TtsPhase::Speaking {
            return Err(TtsError::bad_request(
                "Das Vorlesen läuft gerade. Erst anhalten, dann die Probestimme abspielen.",
            ));
        }
        let snapshot = self.provider_snapshot();
        let voice = voice.or_else(|| snapshot.resolved.voice_for(id));
        let device = self.core.output_device.lock().unwrap().clone();
        self.provider_preview.start(PreviewRequest {
            provider_id: id.to_string(),
            text: text.to_string(),
            voice,
            subject: snapshot.resolved.subject(id),
            settings: snapshot.settings.clone(),
            device,
        })
    }

    /// Probestimme abbrechen (Synthese und Wiedergabe).
    pub(crate) fn cancel_provider_preview(&self, request_id: &str) -> bool {
        self.provider_preview.cancel(request_id)
    }

    /// Fish über die Registry (Probestimme, später CLI-Benchmark): den Server
    /// sicherstellen und den bestehenden HTTP-Pfad des Kerns nehmen. Stimme:
    /// die der Optionen, sonst die eingestellte (`tts_voice`).
    pub(crate) async fn fish_provider_synthesize(
        &self,
        text: &str,
        opts: &SynthesisOptions,
        _cancel: &CancelToken,
    ) -> Result<AudioClip, TtsError> {
        self.refresh_from_settings();
        self.ensure_server().await.map_err(TtsError::unavailable)?;
        let port = *self.core.port.lock().unwrap();
        let seed = opts.seed.unwrap_or(*self.core.seed.lock().unwrap());
        let voice = opts
            .voice
            .clone()
            .or_else(|| self.core.voice.lock().unwrap().clone());
        let req = engine::SynthesisRequest {
            text,
            voice: voice.as_deref(),
            seed,
            speed: None,
        };
        let bytes = self
            .core
            .fish_synthesize(port, req)
            .await
            .map_err(TtsError::backend)?;
        AudioClip::from_wav(bytes)
    }
}
