//! Regelwerk und Compliance-Schild fuer die Oberflaeche.

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use crate::managers::compliance::{
    self, Assessment, ComplianceProfile, ShieldInputs, ShieldStatus,
};
use crate::settings::{self, AppSettings, LlmConnection, PostProcessProvider};

/// Der Anbieter einer Verbindung, wie `llm_client` ihn sieht: Vorlagenkennung
/// als `id`, die Adresse der Verbindung.
pub fn provider_for_connection(connection: &LlmConnection) -> PostProcessProvider {
    PostProcessProvider {
        id: connection.kind.clone(),
        label: connection.label.clone(),
        base_url: connection.base_url.clone(),
        allow_base_url_edit: false,
        models_endpoint: None,
        supports_structured_output: false,
    }
}

/// Bewertung eines freigegebenen Modells (`LlmModelConfig::id`).
#[derive(Serialize, Debug, Clone, Type)]
pub struct ModelCompliance {
    pub model_id: String,
    pub assessment: Assessment,
}

pub fn assess_connection(s: &AppSettings, connection: &LlmConnection) -> Assessment {
    compliance::assess(
        s.compliance_profile,
        &provider_for_connection(connection),
        connection.training_opt_out,
        &compliance::today(),
    )
}

/// Bewertung aller freigegebenen Modelle unter dem eingestellten Regelwerk.
#[tauri::command]
#[specta::specta]
pub fn compliance_assess_models(app: AppHandle) -> Vec<ModelCompliance> {
    let s = settings::get_settings(&app);
    s.llm_models
        .iter()
        .filter_map(|m| {
            let c = s.llm_connections.iter().find(|c| c.id == m.connection_id)?;
            Some(ModelCompliance {
                model_id: m.id.clone(),
                assessment: assess_connection(&s, c),
            })
        })
        .collect()
}

#[tauri::command]
#[specta::specta]
pub fn compliance_set_profile(app: AppHandle, profile: ComplianceProfile) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    s.compliance_profile = profile;
    settings::write_settings(&app, s);
    Ok(())
}

/// Bestaetigung "Training im Konto abgeschaltet" fuer eine Verbindung.
#[tauri::command]
#[specta::specta]
pub fn compliance_set_training_opt_out(
    app: AppHandle,
    connection_id: String,
    opt_out: bool,
) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    let c = s
        .llm_connections
        .iter_mut()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("Unbekannte Verbindung: {connection_id}"))?;
    c.training_opt_out = opt_out;
    settings::write_settings(&app, s);
    Ok(())
}

/// Ampel und Erklaerung fuer das Schild in der Fussleiste.
#[tauri::command]
#[specta::specta]
pub fn compliance_status(app: AppHandle) -> ShieldStatus {
    let s = settings::get_settings(&app);
    let active = s.active_llm_model().map(|(c, m)| {
        (
            m.label.clone(),
            provider_for_connection(c),
            c.training_opt_out,
        )
    });
    // Was wirklich unverschluesselt in der Datei steht -- geladen sind die
    // Schluessel immer im Klartext.
    let plaintext_keys = settings::stored_plaintext_key_count(&app);
    let now = chrono::Utc::now().timestamp();
    let cloud_errors_24h = crate::managers::usage::ledger()
        .and_then(|l| l.events(200, 0).ok())
        .map(|events| {
            events
                .iter()
                .filter(|e| now - e.ts < 86_400 && !e.ok)
                .filter(|e| !matches!(e.connection_kind.as_str(), "local" | "ollama" | "vllm"))
                .count()
        })
        .unwrap_or(0);
    let inputs = ShieldInputs {
        plaintext_keys,
        blocks_24h: compliance::recent_blocks()
            .into_iter()
            .map(|(_, p)| p)
            .collect(),
        cloud_errors_24h,
    };
    compliance::shield(
        s.compliance_profile,
        active.as_ref().map(|(l, p, o)| (l.as_str(), p, *o)),
        &inputs,
        &compliance::today(),
    )
}
