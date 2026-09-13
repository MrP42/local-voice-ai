//! Sprachmodell-Verbindungen und freigegebene Modelle (Schema 2).
//!
//! Die Oberflaeche verwaltet Verbindungen (ein Konto bei einem Anbieter, eine
//! lokale Laufzeit) und gibt je Verbindung einzelne Modelle frei. Was hier
//! gespeichert wird, spiegelt `AppSettings::sync_legacy_from_llm` in die
//! aelteren Felder, von denen Nachbearbeitung, Zusammenfassung, Protokoll und
//! Tagging heute noch lesen -- so wechseln alle Funktionen gemeinsam.

use tauri::AppHandle;

use crate::settings::{self, LlmConnection, LlmModelConfig, PostProcessProvider};

/// Verbindung anlegen oder ersetzen (nach `id`). Eine leere `id` oder eine
/// unbekannte Vorlage wird abgewiesen: eine Verbindung ohne Vorlage haette
/// keine Endpunkte, mit denen die App sprechen koennte.
#[tauri::command]
#[specta::specta]
pub fn llm_upsert_connection(app: AppHandle, connection: LlmConnection) -> Result<(), String> {
    if connection.id.trim().is_empty() {
        return Err("Verbindung ohne Kennung".to_string());
    }
    let mut s = settings::get_settings(&app);
    if s.post_process_provider(&connection.kind).is_none() {
        return Err(format!("Unbekannte Anbieter-Vorlage: {}", connection.kind));
    }
    match s.llm_connections.iter_mut().find(|c| c.id == connection.id) {
        Some(existing) => *existing = connection,
        None => s.llm_connections.push(connection),
    }
    s.sync_legacy_from_llm();
    settings::write_settings(&app, s);
    Ok(())
}

/// Verbindung samt ihrer Modelle entfernen. War eines davon aktiv, ist danach
/// nichts aktiv -- die alten Felder bleiben, wie sie sind, bis der Nutzer neu
/// waehlt.
#[tauri::command]
#[specta::specta]
pub fn llm_remove_connection(app: AppHandle, id: String) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    s.llm_connections.retain(|c| c.id != id);
    s.llm_models.retain(|m| m.connection_id != id);
    if let Some(active) = s.llm_active_model_id.clone() {
        if !s.llm_models.iter().any(|m| m.id == active) {
            s.llm_active_model_id = None;
        }
    }
    settings::write_settings(&app, s);
    Ok(())
}

/// Modell freigeben oder seine Limits/Preise aendern. Die `id` folgt aus
/// Verbindung und Modellname (`LlmModelConfig::make_id`), damit ein Modell je
/// Verbindung nur einmal freigegeben sein kann.
#[tauri::command]
#[specta::specta]
pub fn llm_upsert_model(app: AppHandle, mut model: LlmModelConfig) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    if !s.llm_connections.iter().any(|c| c.id == model.connection_id) {
        return Err(format!("Unbekannte Verbindung: {}", model.connection_id));
    }
    if model.remote_id.trim().is_empty() {
        return Err("Modell ohne Namen".to_string());
    }
    model.id = LlmModelConfig::make_id(&model.connection_id, &model.remote_id);
    if model.label.trim().is_empty() {
        model.label = model.remote_id.clone();
    }
    match s.llm_models.iter_mut().find(|m| m.id == model.id) {
        Some(existing) => *existing = model,
        None => s.llm_models.push(model),
    }
    s.sync_legacy_from_llm();
    settings::write_settings(&app, s);
    Ok(())
}

/// Freigabe zuruecknehmen. War es das aktive Modell, ist danach keines aktiv.
#[tauri::command]
#[specta::specta]
pub fn llm_remove_model(app: AppHandle, id: String) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    s.llm_models.retain(|m| m.id != id);
    if s.llm_active_model_id.as_deref() == Some(id.as_str()) {
        s.llm_active_model_id = None;
    }
    settings::write_settings(&app, s);
    Ok(())
}

/// Das aktive Modell setzen. Nur ein freigegebenes Modell einer
/// eingeschalteten Verbindung ist waehlbar -- alles andere waere ein Verweis
/// ins Leere, und die Funktionen liefen gegen die Wand.
#[tauri::command]
#[specta::specta]
pub fn llm_set_active_model(app: AppHandle, id: Option<String>) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    if let Some(ref wanted) = id {
        let selectable = s.llm_models.iter().any(|m| {
            m.id == *wanted
                && m.enabled
                && s.llm_connections
                    .iter()
                    .any(|c| c.id == m.connection_id && c.enabled)
        });
        if !selectable {
            return Err(format!("Modell nicht waehlbar: {wanted}"));
        }
    }
    s.llm_active_model_id = id;
    s.sync_legacy_from_llm();
    settings::write_settings(&app, s);
    Ok(())
}

/// Die Modelle, die ein Anbieter ueber seine Verbindung anbietet -- zum
/// Freigeben. Spricht die Vorlage der Verbindung an, aber mit deren Adresse
/// und Schluessel, damit zwei Konten derselben Art getrennt bleiben.
#[tauri::command]
#[specta::specta]
pub async fn llm_list_remote_models(
    app: AppHandle,
    connection_id: String,
) -> Result<Vec<String>, String> {
    let s = settings::get_settings(&app);
    let connection = s
        .llm_connections
        .iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("Unbekannte Verbindung: {connection_id}"))?;
    let template = s
        .llm_template(connection)
        .ok_or_else(|| format!("Unbekannte Anbieter-Vorlage: {}", connection.kind))?;
    let provider = PostProcessProvider {
        id: connection.id.clone(),
        label: connection.label.clone(),
        base_url: connection.base_url.clone(),
        allow_base_url_edit: template.allow_base_url_edit,
        models_endpoint: template.models_endpoint.clone(),
        supports_structured_output: template.supports_structured_output,
    };
    let api_key = s
        .post_process_api_keys
        .get(&connection.id)
        .cloned()
        .unwrap_or_default();
    crate::llm_client::fetch_models(&provider, api_key).await
}

/// Schluessel einer Verbindung setzen. Abgelegt unter der Verbindungs-`id`,
/// nicht unter der Vorlage -- zwei Konten derselben Art brauchen zwei
/// Schluessel. Der aeltere Befehl prueft gegen die Vorlagen und wuerde eine
/// frei benannte Verbindung abweisen.
#[tauri::command]
#[specta::specta]
pub fn llm_set_api_key(app: AppHandle, connection_id: String, api_key: String) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    if !s.llm_connections.iter().any(|c| c.id == connection_id) {
        return Err(format!("Unbekannte Verbindung: {connection_id}"));
    }
    s.post_process_api_keys.insert(connection_id, api_key);
    settings::write_settings(&app, s);
    Ok(())
}
