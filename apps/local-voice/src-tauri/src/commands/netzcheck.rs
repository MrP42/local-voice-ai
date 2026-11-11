//! Tauri-Befehl der Netzwerkprüfung (lva#496). Dünne Hülle: Einstellungen lesen,
//! den reinen Kern in `crate::netzcheck` anwenden. Nur auf Knopfdruck; kein
//! Hintergrundlauf, keine Wiederholung, keine Übertragung von Inhalten.
//!
//! Systemschutz: kein Kindprozess, kein Modell; höchstens [`MAX_TARGETS`]
//! gleichzeitige Anfragen mit je 5 s Zeitlimit und ohne Antwortinhalt (`HEAD`),
//! also ein paar Kilobyte Speicher. Bei knappem RAM ändert sich daran nichts.
//!
//! [`MAX_TARGETS`]: crate::netzcheck::targets::MAX_TARGETS

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::managers::calendar::model::CalendarKind;
use crate::managers::meetings::store::MeetingStore;
use crate::managers::model::ModelManager;
use crate::managers::tts::providers::ollama_speech::OLLAMA_ID;
use crate::managers::tts::providers::openai_speech::DEFAULT_ID as OPENAI_DEFAULT_ID;
use crate::managers::tts::providers::phonon::PHONON_ID;
use crate::managers::tts::providers::policy::RESERVED_IDS;
use crate::netzcheck::probe::SystemProbe;
use crate::netzcheck::proxy::{self, ProxyInfo};
use crate::netzcheck::report::{self, ReportMeta};
use crate::netzcheck::targets::{derive_targets, Service, Setup};
use crate::netzcheck::{run_check, Row, PROBE_TIMEOUT};

/// Ergebnis der Prüfung: Zeilen für die Oberfläche und der Bericht zum Kopieren.
#[derive(Serialize, Debug, Clone, Type)]
pub struct NetzcheckResult {
    pub rows: Vec<Row>,
    pub proxy: ProxyInfo,
    /// Berichtstext (Deutsch) für die IT: Hostname, Port, Ergebnis,
    /// App-Version, Betriebssystem; geschwärzt.
    pub report: String,
}

/// Läuft schon eine Prüfung? Ein zweiter Klick wartet nicht, er wird abgewiesen.
static RUNNING: AtomicBool = AtomicBool::new(false);

struct RunningGuard;

impl RunningGuard {
    fn acquire() -> Option<Self> {
        RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then_some(Self)
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

/// Adressen der Update-Quelle aus der Updater-Einstellung der App
/// (`plugins.updater.endpoints`), also auch bei einer eigenen Quelle.
fn update_endpoints(app: &AppHandle) -> Vec<String> {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("endpoints"))
        .and_then(|e| e.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn collect_setup(app: &AppHandle) -> Setup {
    let settings = crate::settings::get_settings(app);

    let llm_connections = settings
        .llm_connections
        .iter()
        .map(|c| Service {
            url: c.base_url.clone(),
            enabled: c.enabled,
            allow_remote: false,
        })
        .collect();

    let tts = &settings.tts_providers;
    let openai_id = if RESERVED_IDS.contains(&tts.openai_speech.id.as_str()) {
        OPENAI_DEFAULT_ID
    } else {
        tts.openai_speech.id.as_str()
    };
    let tts_services = vec![
        Service {
            url: tts.openai_speech.base_url.clone(),
            enabled: tts.is_enabled(openai_id),
            allow_remote: tts.is_remote_allowed(openai_id),
        },
        Service {
            url: tts.ollama_speech.base_url.clone(),
            enabled: tts.is_enabled(OLLAMA_ID),
            allow_remote: tts.is_remote_allowed(OLLAMA_ID),
        },
    ];
    let huggingface_downloads =
        tts.is_enabled(PHONON_ID) && tts.is_remote_allowed(PHONON_ID) && tts.phonon.allow_network;

    let (mut calendar_graph, mut calendar_ics_hosts) = (false, Vec::new());
    if let Some(store) = app.try_state::<Arc<MeetingStore>>() {
        for source in store
            .calendar_sources()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.enabled)
        {
            match source.kind {
                CalendarKind::Graph => calendar_graph = true,
                // `account_hint` einer ICS-Quelle ist nur der Host, nie die Adresse.
                CalendarKind::Ics => calendar_ics_hosts.extend(source.account_hint),
            }
        }
    }

    let model_download_urls = app
        .try_state::<Arc<ModelManager>>()
        .map(|m| {
            m.get_available_models()
                .into_iter()
                .filter_map(|info| info.url)
                .collect()
        })
        .unwrap_or_default();

    Setup {
        sync_hub_url: crate::sync::account::load(app).map(|c| c.url),
        update_checks_enabled: settings.update_checks_enabled,
        update_endpoints: update_endpoints(app),
        llm_connections,
        tts_services,
        calendar_graph,
        calendar_ics_hosts,
        model_download_urls,
        huggingface_downloads,
    }
}

/// Prüft alle eingerichteten Gegenstellen (parallel, je 5 s) und liefert
/// Zeilen samt Bericht. Nur auf Knopfdruck.
#[tauri::command]
#[specta::specta]
pub async fn netzcheck_run(app: AppHandle) -> Result<NetzcheckResult, String> {
    let _guard = RunningGuard::acquire().ok_or_else(|| "busy".to_string())?;
    let setup = collect_setup(&app);
    let targets = derive_targets(&setup);
    let proxy_info = proxy::detect();
    let probe = SystemProbe::new(&proxy_info);
    let rows = run_check(&targets, &probe, PROBE_TIMEOUT).await;
    let meta = ReportMeta {
        app_version: app.package_info().version.to_string(),
        os: format!(
            "{} {} ({})",
            tauri_plugin_os::platform(),
            tauri_plugin_os::version(),
            tauri_plugin_os::arch()
        ),
        generated_at: chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string(),
    };
    let report = report::build(&rows, &proxy_info, &meta);
    Ok(NetzcheckResult {
        rows,
        proxy: proxy_info,
        report,
    })
}
