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

use super::bench::ColdReset;
use super::builtin::SynthFn;
use super::contract::TtsProvider;
use super::contract::{AudioClip, CancelToken, ProviderDescriptor, SynthesisOptions, TtsError};
use super::policy::{
    check_use, process_env, resolve_provider_configs_with, use_state, LiveApprovals,
    ProviderOverrides, ResolvedConfigs, TtsProvidersSettings, UseState, UseSubject, FISH_ID,
    PIPER_ID,
};
use super::preview::{PreviewRequest, PreviewService, TtsProviderPreview};
use super::registry::ProviderRegistry;
use super::setup::ProviderPaths;
use crate::managers::tts::engine::{self, TtsEngineKind};
use crate::managers::tts::phonon_models;
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

/// Die Fish-Synthese für den BENCHMARK: wie [`fish_synth`], aber ohne
/// `refresh_from_settings` und ohne `ensure_server` je Satz. Server und
/// Einstellungen macht der Aufrufer VORHER einmal bereit
/// ([`TtsManager::bench_prepare`]; Kaltläufe über `bench_cold_reset`, dessen
/// Startzeit als `process_start_ms` getrennt gemessen wird) — in der gemessenen
/// Zeit liegt dann nur der HTTP-Aufruf der Synthese, kein Gesundheitsaufruf.
/// Der Vorlese-Pfad der App benutzt weiter [`fish_synth`].
pub fn fish_bench_synth(manager: Weak<TtsManager>) -> SynthFn {
    Arc::new(move |text, opts, cancel| {
        let manager = manager.clone();
        Box::pin(async move {
            let manager = manager
                .upgrade()
                .ok_or_else(|| TtsError::unavailable("Die App wird beendet"))?;
            manager
                .fish_provider_synthesize_prepared(&text, &opts, &cancel)
                .await
        })
    })
}

/// Probestimme: Player und Regler des Kerns, Ereignis an die Oberfläche; die
/// Guardrails prüft sie gegen den gemeinsamen, aktuellen Stand der Freigaben.
pub fn preview_service(
    app: &tauri::AppHandle,
    core: &Arc<TtsCore>,
    registry: &Arc<ProviderRegistry>,
    approvals: &LiveApprovals,
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
        approvals.clone(),
    )
}

/// Schreibzugriffe auf die Einstellungen der Anbieter nacheinander (die Befehle
/// in `commands::tts_providers`) UND Abgleich des gemeinsamen Stands der
/// Freigaben mit der Datei: wer den Stand aus der Datei übernimmt, tut es unter
/// derselben Sperre, damit ein veralteter Lesezugriff einen frischen Widerruf
/// nie überschreibt.
pub(crate) static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    /// Gegenstand der Guardrails für `id`: aus der INSTANZ der Registry
    /// ([`TtsProvider::use_subject`]), nicht aus `resolved` — die Registry lebt
    /// lange und kann zwischen dem Auflösen der Konfiguration und diesem Aufruf
    /// neu gebaut worden sein. Nur eine unbekannte Kennung fällt auf die
    /// aufgelöste Konfiguration zurück.
    pub fn subject(&self, id: &str) -> UseSubject {
        match self.registry.get(id) {
            Some(provider) => provider.use_subject(),
            None => self.resolved.subject(id),
        }
    }

    /// Beschreibung UND Freigabezustand EINER Instanz, beides aus demselben
    /// Zugriff (Gegenstand aus der Instanz selbst).
    pub fn use_state_of(&self, provider: &dyn TtsProvider) -> (ProviderDescriptor, UseState) {
        let descriptor = provider.descriptor();
        let state = use_state(&descriptor, &provider.use_subject(), &self.settings);
        (descriptor, state)
    }

    /// Freigabezustand von `desc` (Gegenstand aus der Instanz, siehe [`subject`](Self::subject)).
    pub fn use_state(&self, desc: &ProviderDescriptor) -> UseState {
        use_state(desc, &self.subject(&desc.id), &self.settings)
    }

    /// Anbieter nachschlagen UND die Guardrails prüfen ([`check_use`]);
    /// Beschreibung und Gegenstand stammen aus derselben Instanz.
    pub fn checked(&self, id: &str) -> Result<ProviderDescriptor, TtsError> {
        let provider = self
            .registry
            .get(id)
            .ok_or_else(|| TtsError::bad_request(format!("Unbekannter Anbieter '{id}'")))?;
        let desc = provider.descriptor();
        check_use(&desc, &provider.use_subject(), &self.settings)?;
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
    /// Die Fish-Synthese für die EIGENE Registry der CLI (`--tts-test`,
    /// `--tts-bench` mit `fish`; sie hat ihre eigene Konfiguration mit Overrides
    /// und berührt die langlebige Registry des Managers nicht), siehe
    /// [`fish_bench_synth`]: kein `refresh_from_settings`/`ensure_server` je
    /// Satz — der Aufrufer macht beides einmal VOR der gemessenen Zeit
    /// ([`bench_prepare`](Self::bench_prepare), `bench_cold_reset`). Die
    /// Registry des Managers (Vorlesen, Probestimme) nimmt weiter [`fish_synth`].
    pub(crate) fn fish_bench_synth_fn(self: &Arc<Self>) -> SynthFn {
        fish_bench_synth(Arc::downgrade(self))
    }

    /// Einmal VOR der Messschleife: Einstellungen lesen und den Fish-Server
    /// sicherstellen (RAM-Start-Gate und Job-Objekt wie immer in
    /// `ensure_server`). Danach gibt es je Synthese nur noch den HTTP-Aufruf
    /// ([`fish_provider_synthesize_prepared`](Self::fish_provider_synthesize_prepared)).
    pub(crate) async fn bench_prepare(&self) -> Result<(), TtsError> {
        self.refresh_from_settings();
        self.ensure_server().await.map_err(TtsError::unavailable)
    }

    /// Gemeinsamen Stand der Freigaben aus der Datei übernehmen — unter der
    /// Sperre der Befehle, damit ein veralteter Lesezugriff einen frischen
    /// Widerruf nicht überschreibt.
    pub(crate) fn sync_live_approvals(&self) {
        let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let providers = crate::settings::get_settings(&self.app).tts_providers;
        self.provider_host.approvals().set(providers);
    }

    /// Nach jeder Änderung der Freigaben oder der Konfiguration eines Anbieters
    /// (die Befehle der Oberfläche rufen es nach dem Speichern): Registry und Engine
    /// auf den neuen Stand bringen und eine laufende Probestimme beenden, wenn
    /// ihr Anbieter jetzt gesperrt ist. Laufende Aufträge des Vorlesens, des
    /// Exports und des Vorwärmens prüfen den gemeinsamen Stand je Satz selbst
    /// (`ProviderEngine::guard`) — hier kommt nur der Neuaufbau der Engine dazu,
    /// damit auch Modell und Adresse der neuen Konfiguration gelten.
    pub(crate) fn providers_changed(&self) {
        // Die Sprecher des Anbieters (Stimmenliste, Auswahl, gewaehlte Stimme) koennen
        // sich mit geaendert haben: beim naechsten Zerlegen des Textes neu lesen.
        self.invalidate_provider_roster();
        self.refresh_from_settings();
        if self.provider_preview.enforce() {
            log::info!("Probestimme beendet: der Anbieter ist nicht mehr freigegeben");
        }
    }

    /// Der gemeinsame Stand der Freigaben (die Befehle setzen ihn nach dem Speichern).
    pub(crate) fn live_approvals(&self) -> &LiveApprovals {
        self.provider_host.approvals()
    }

    /// Kaltstart des Fish-Servers für den Benchmark. Beendet NUR einen
    /// SELBST gestarteten Server (er ist dann der Kaltstart-Fall) und startet
    /// ihn neu; die Zeit bis „bereit“ ist `process_start_ms`. Läuft ein
    /// fremd gestarteter Server, bleibt er unberührt (`foreign_server_kept`):
    /// der Kaltlauf ist dann keiner, und der Aufrufer vermerkt das. Der Start
    /// läuft über `ensure_server` — also mit RAM-Start-Gate und Job-Objekt.
    pub(crate) async fn bench_cold_reset(&self) -> Result<ColdReset, TtsError> {
        self.refresh_from_settings();
        let owned = self.core.owns_server();
        if owned {
            self.stop_owned_server();
        } else {
            let port = *self.core.port.lock().unwrap();
            if self.core.health_ok(port).await {
                return Ok(ColdReset {
                    process_start_ms: None,
                    kind: "foreign_server_kept",
                });
            }
        }
        let started = std::time::Instant::now();
        self.ensure_server().await.map_err(TtsError::unavailable)?;
        Ok(ColdReset {
            process_start_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
            kind: if owned {
                "server_restarted"
            } else {
                "server_started"
            },
        })
    }

    /// Beendet AUSSCHLIESSLICH den selbst gestarteten Fish-Server (samt
    /// Prozessbaum); ein fremder bleibt unberührt. `true`: es gab einen eigenen.
    pub(crate) fn stop_owned_server(&self) -> bool {
        let owned = self.core.owns_server();
        if owned {
            self.kill_owned_child();
            self.core
                .owns_server
                .store(false, std::sync::atomic::Ordering::Release);
            self.core.set_phase(TtsPhase::Stopped, None);
        }
        owned
    }

    /// Registry auf den Stand der Einstellungen bringen und die wirksame
    /// Konfiguration liefern (Umgebung eingerechnet, keine Overrides — die
    /// gibt es nur für die CLI, die ihre eigene Registry baut: `tts_cli`, P3b).
    /// Billig, wenn nichts geändert ist.
    pub(crate) fn sync_providers(
        &self,
        providers: &TtsProvidersSettings,
        fish_dir: &str,
        piper_voice: Option<&str>,
        piper_auto_language: bool,
    ) -> Arc<ResolvedConfigs> {
        let paths = ProviderPaths {
            fish_dir: PathBuf::from(fish_dir),
            app_data: self.data_base_dir(),
            piper_voice: piper_voice.map(String::from),
            piper_auto_language,
        };
        // Stufe "installiert": was die Modellseite eingerichtet hat, gilt, solange nichts
        // eingetragen ist. Nur Dateisystem (ein paar `is_file`), billig bei jedem Abgleich;
        // ein neu installiertes Modell baut den Anbieter beim naechsten Abgleich neu.
        let installed = paths
            .app_data
            .as_deref()
            .map(phonon_models::installed_paths)
            .unwrap_or_default();
        let resolved = Arc::new(resolve_provider_configs_with(
            providers,
            &process_env,
            &ProviderOverrides::default(),
            &installed,
        ));
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
        // Die Freigaben sind die der Datei JETZT (der Dienst prüft gegen den
        // gemeinsamen Stand und danach je Teil weiter).
        self.sync_live_approvals();
        let snapshot = self.provider_snapshot();
        let voice = voice.or_else(|| snapshot.resolved.voice_for(id));
        let device = self.core.output_device.lock().unwrap().clone();
        self.provider_preview.start(PreviewRequest {
            provider_id: id.to_string(),
            text: text.to_string(),
            voice,
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
        cancel: &CancelToken,
    ) -> Result<AudioClip, TtsError> {
        self.refresh_from_settings();
        self.ensure_server().await.map_err(TtsError::unavailable)?;
        self.fish_provider_synthesize_prepared(text, opts, cancel)
            .await
    }

    /// Nur der HTTP-Pfad der Synthese: Server und Einstellungen müssen schon
    /// bereit sein ([`bench_prepare`](Self::bench_prepare)). Ein inzwischen
    /// gestorbener Server ist dann ein Fehlschlag der Messung, kein heimlicher
    /// Neustart innerhalb der gemessenen Zeit.
    pub(crate) async fn fish_provider_synthesize_prepared(
        &self,
        text: &str,
        opts: &SynthesisOptions,
        _cancel: &CancelToken,
    ) -> Result<AudioClip, TtsError> {
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
