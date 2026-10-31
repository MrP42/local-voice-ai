//! Anbindung des Sprachdienstes an die laufende App: Engines, Ereignisse, Fenster.
//!
//! Alles, was Tauri braucht, steht hier und in `commands/sprachdienst.rs`; die Bausteine
//! daneben sind davon unabhängig und einzeln getestet. Keine Inhalte im Log.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use wai_voice_dienst::msg::Engines;

use super::dienst::Sprachdienst;
use super::engine::{pcm_zu_f32, EngineFehler, Sprache};
use super::kontext::{Konfig, Kontext, Meldung};
use super::zugang::ZugangStore;
use crate::managers::transcription::TranscriptionManager;
use crate::managers::tts::engine::provider_id_from_setting;
use crate::managers::tts::providers::is_loopback_url;
use crate::managers::tts::providers::policy::{
    process_env, resolve_provider_configs, ProviderOverrides,
};
use crate::managers::tts::TtsManager;
use crate::settings::{get_settings, AppSettings};

/// Ereignis: eine Kopplungsanfrage wartet (die Oberfläche holt Name und Code per Befehl).
pub const EREIGNIS_KOPPLUNG: &str = "sprachdienst-kopplung";
/// Ereignis: die Liste der Zugänge hat sich geändert.
pub const EREIGNIS_ZUGAENGE: &str = "sprachdienst-zugaenge";

/// So lange darf ein Auftrag auf das Laden des Erkennungsmodells warten.
const MODELL_WARTEN: Duration = Duration::from_secs(90);

/// Läuft die gewählte Stimme auf diesem Rechner? Nur ein Anbieter mit einer Adresse außerhalb
/// von Loopback schickt den Text fort; alle anderen Engines sind lokal.
pub fn stimme_ist_lokal(s: &AppSettings) -> bool {
    let Some(id) = provider_id_from_setting(&s.tts_engine) else {
        return true;
    };
    let konfig = resolve_provider_configs(
        &s.tts_providers,
        &process_env,
        &ProviderOverrides::default(),
    );
    match konfig.subject(&id).base_url {
        None => true,
        Some(url) => is_loopback_url(&url),
    }
}

pub struct AppSprache {
    app: AppHandle,
}

impl AppSprache {
    pub fn neu(app: AppHandle) -> Self {
        Self { app }
    }

    fn erkennung(&self) -> Result<Arc<TranscriptionManager>, EngineFehler> {
        self.app
            .try_state::<Arc<TranscriptionManager>>()
            .map(|t| t.inner().clone())
            .ok_or_else(|| EngineFehler::fehlt("Die Spracherkennung ist nicht bereit."))
    }

    fn stimmen(&self) -> Result<Arc<TtsManager>, EngineFehler> {
        self.app
            .try_state::<Arc<TtsManager>>()
            .map(|t| t.inner().clone())
            .ok_or_else(|| EngineFehler::fehlt("Die Sprachausgabe ist nicht bereit."))
    }
}

impl Sprache for AppSprache {
    fn engines(&self) -> Engines {
        let s = get_settings(&self.app);
        let erkennung = self
            .erkennung()
            .ok()
            .and_then(|t| t.get_current_model())
            .unwrap_or_else(|| "keine".to_string());
        Engines {
            erkennung,
            stimme: s.tts_engine.clone(),
            lokal: stimme_ist_lokal(&s),
        }
    }

    fn bereit(&self) -> bool {
        self.erkennung()
            .ok()
            .is_some_and(|t| t.get_current_model().is_some())
    }

    fn erkennen(
        &self,
        pcm: &[u8],
        _bereich: &str,
        abbruch: &Arc<AtomicBool>,
    ) -> Result<String, EngineFehler> {
        let abgebrochen = || abbruch.load(Ordering::Acquire);
        if abgebrochen() {
            return Err(EngineFehler::abgebrochen());
        }
        let tm = self.erkennung()?;
        if tm.get_current_model().is_none() {
            return Err(EngineFehler::fehlt("Es ist kein Erkennungsmodell gewählt."));
        }
        if !tm.is_model_loaded() {
            tm.initiate_model_load();
            let bis = Instant::now() + MODELL_WARTEN;
            while !tm.is_model_loaded() {
                if abgebrochen() {
                    return Err(EngineFehler::abgebrochen());
                }
                if Instant::now() >= bis {
                    return Err(EngineFehler::fehlgeschlagen(
                        "Das Erkennungsmodell wurde nicht rechtzeitig geladen.",
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        if abgebrochen() {
            return Err(EngineFehler::abgebrochen());
        }
        // Nur die Art des Fehlers ins Log, nie sein Text: er könnte Inhalte enthalten.
        tm.transcribe(pcm_zu_f32(pcm)).map_err(|_| {
            log::warn!("sprachdienst: Erkennung fehlgeschlagen");
            EngineFehler::fehlgeschlagen("Die Erkennung ist fehlgeschlagen.")
        })
    }

    fn sprechen(
        &self,
        text: &str,
        stimme: Option<&str>,
        abbruch: &Arc<AtomicBool>,
    ) -> Result<Vec<Vec<u8>>, EngineFehler> {
        if abbruch.load(Ordering::Acquire) {
            return Err(EngineFehler::abgebrochen());
        }
        let tts = self.stimmen()?;
        // Eine fremde App darf nur Stimmen wählen, die es gibt; sonst gilt die eingestellte.
        let stimme = stimme.filter(|s| tts.list_voice_ids().iter().any(|v| v.as_str() == *s));
        let text = text.to_string();
        let stimme = stimme.map(str::to_string);
        let abbruch = abbruch.clone();
        tauri::async_runtime::block_on(async move {
            tts.synthesize_wavs(&text, stimme.as_deref(), &abbruch)
                .await
        })
        .map_err(|e| {
            // Nur die Art des Fehlers ins Log, nie sein Text: er könnte Inhalte enthalten.
            if e == crate::managers::tts::providers::pipeline::CANCELLED_MESSAGE {
                return EngineFehler::abgebrochen();
            }
            log::warn!("sprachdienst: Sprachausgabe fehlgeschlagen");
            EngineFehler::fehlgeschlagen("Die Sprachausgabe ist fehlgeschlagen.")
        })
    }
}

fn jetzt_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn fenster_zeigen(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Legt den Dienst an und startet ihn nur, wenn der Nutzer ihn eingeschaltet hat (Opt-in).
pub fn init(app: &AppHandle) -> Arc<Sprachdienst> {
    let speicher = match crate::portable::app_data_dir(app) {
        Ok(dir) => Arc::new(ZugangStore::oeffnen(dir.join("sprachdienst.json"))),
        Err(e) => {
            log::error!("sprachdienst: Datenordner nicht verfügbar: {e}");
            Arc::new(ZugangStore::im_speicher())
        }
    };
    let melder = app.clone();
    let kontext = Kontext::new(
        Konfig::standard(env!("CARGO_PKG_VERSION")),
        speicher.clone(),
        Arc::new(AppSprache::neu(app.clone())),
        jetzt_ms,
        Box::new(move |m| match m {
            Meldung::KopplungOffen => {
                fenster_zeigen(&melder);
                let _ = melder.emit(EREIGNIS_KOPPLUNG, ());
            }
            Meldung::ZugaengeGeaendert => {
                let _ = melder.emit(EREIGNIS_ZUGAENGE, ());
            }
        }),
    );
    let dienst = Sprachdienst::neu(kontext);
    if speicher.aktiv() {
        let d = dienst.clone();
        tauri::async_runtime::spawn(async move {
            let _ = d.starten().await;
        });
    }
    dienst
}
