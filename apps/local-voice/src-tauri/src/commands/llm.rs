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

// ---- Lokales Sprachmodell: Laufzeit, Modelle, Server ----------------------

use std::sync::Arc;

use tauri::Manager;

use crate::managers::llm::{LlmDownloadInfo, LlmRuntimeManager, LocalLlmServer, LocalLlmStatus};

#[tauri::command]
#[specta::specta]
pub fn llm_local_list(app: AppHandle) -> Vec<LlmDownloadInfo> {
    app.state::<Arc<LlmRuntimeManager>>().list_downloads()
}

#[tauri::command]
#[specta::specta]
pub async fn llm_local_download(app: AppHandle, id: String) -> Result<(), String> {
    let manager = app.state::<Arc<LlmRuntimeManager>>().inner().clone();
    manager.download(&id).await
}

#[tauri::command]
#[specta::specta]
pub fn llm_local_cancel(app: AppHandle, id: String) -> Result<(), String> {
    app.state::<Arc<LlmRuntimeManager>>().cancel(&id);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn llm_local_delete(app: AppHandle, id: String) -> Result<(), String> {
    // Ein Modell, das gerade bedient wird, zuerst freigeben -- sonst haelt
    // der Server die Datei offen, und das Loeschen scheitert leise.
    let server = app.state::<Arc<LocalLlmServer>>();
    if server.is_serving(&id) {
        server.stop();
    }
    app.state::<Arc<LlmRuntimeManager>>().delete(&id)
}

#[tauri::command]
#[specta::specta]
pub fn llm_local_status(app: AppHandle) -> LocalLlmStatus {
    app.state::<Arc<LocalLlmServer>>().status()
}

/// Startet den Server fuer ein geladenes Modell (oder wechselt darauf).
/// Liefert die Adresse -- die Oberflaeche braucht sie nicht, der Test schon.
#[tauri::command]
#[specta::specta]
pub async fn llm_local_start(_app: AppHandle, model_id: String) -> Result<String, String> {
    crate::managers::llm::ensure_local(&model_id).await
}

#[tauri::command]
#[specta::specta]
pub fn llm_local_stop(app: AppHandle) -> Result<(), String> {
    app.state::<Arc<LocalLlmServer>>().stop();
    Ok(())
}

/// Welches Backend der Selbsttest gewaehlt hat (oder waehlen wuerde).
#[tauri::command]
#[specta::specta]
pub async fn llm_local_backend(app: AppHandle) -> Result<String, String> {
    let manager = app.state::<Arc<LlmRuntimeManager>>().inner().clone();
    manager.resolve_runtime().await.map(|(_, backend, _)| backend)
}

/// Ein geladenes lokales Modell zum aktiven Modell machen -- in einem Zug:
/// die Verbindung "In der App" anlegen, falls sie fehlt, das Modell dort
/// freigeben, aktiv setzen, Spiegel nachziehen. Drei Klicks in der Oberflaeche
/// waeren drei Gelegenheiten, auf halbem Weg stehenzubleiben.
#[tauri::command]
#[specta::specta]
pub fn llm_local_activate(app: AppHandle, model_id: String) -> Result<(), String> {
    use crate::managers::llm::{LOCAL_PLACEHOLDER_URL, LOCAL_PROVIDER_ID};
    let runtime = app.state::<Arc<LlmRuntimeManager>>();
    let info = runtime
        .list_downloads()
        .into_iter()
        .find(|d| d.id == model_id)
        .ok_or_else(|| format!("Unbekanntes Modell: {model_id}"))?;
    if !info.is_downloaded {
        return Err(format!("{} ist noch nicht geladen", info.name));
    }
    let mut s = settings::get_settings(&app);
    let connection_id = match s
        .llm_connections
        .iter()
        .find(|c| c.kind == LOCAL_PROVIDER_ID)
        .map(|c| c.id.clone())
    {
        Some(id) => id,
        None => {
            let label = s
                .post_process_provider(LOCAL_PROVIDER_ID)
                .map(|p| p.label.clone())
                .unwrap_or_else(|| "In der App".to_string());
            s.llm_connections.push(LlmConnection {
                id: LOCAL_PROVIDER_ID.to_string(),
                kind: LOCAL_PROVIDER_ID.to_string(),
                label,
                base_url: LOCAL_PLACEHOLDER_URL.to_string(),
                enabled: true,
                monthly_budget_usd: None,
                budget_enforced: false,
            });
            LOCAL_PROVIDER_ID.to_string()
        }
    };
    if let Some(c) = s.llm_connections.iter_mut().find(|c| c.id == connection_id) {
        c.enabled = true;
    }
    let id = LlmModelConfig::make_id(&connection_id, &model_id);
    if !s.llm_models.iter().any(|m| m.id == id) {
        s.llm_models.push(LlmModelConfig {
            id: id.clone(),
            connection_id,
            remote_id: model_id.clone(),
            label: info.name.clone(),
            enabled: true,
            context_limit: Some(crate::managers::llm::DEFAULT_CONTEXT_TOKENS),
            max_input_tokens: None,
            max_output_tokens: None,
            price_input_per_mtok: Some(0.0),
            price_output_per_mtok: Some(0.0),
            tags: info.tags.clone(),
        });
    } else if let Some(m) = s.llm_models.iter_mut().find(|m| m.id == id) {
        m.enabled = true;
    }
    s.llm_active_model_id = Some(id);
    s.sync_legacy_from_llm();
    settings::write_settings(&app, s);
    Ok(())
}

/// RAM und GPU-Speicherbudget fuer die Fussleiste. Auf einem
/// Blocking-Thread: DXGI ist schnell, aber nicht async.
#[tauri::command]
#[specta::specta]
pub async fn system_memory(_app: AppHandle) -> Result<crate::managers::llm::SystemMemory, String> {
    tokio::task::spawn_blocking(crate::managers::llm::resources::system_memory)
        .await
        .map_err(|e| e.to_string())
}

/// Passt das Modell in den Speicher? Prognose gegen das freie Budget der
/// dedizierten Grafikkarte -- oder gegen den RAM, wenn keine messbar ist;
/// dann ist das Urteil "unbekannt", nicht "passt".
#[tauri::command]
#[specta::specta]
pub async fn llm_local_fit(
    app: AppHandle,
    model_id: String,
    context_tokens: Option<u32>,
) -> Result<crate::managers::llm::FitReport, String> {
    use crate::managers::llm::estimate;
    let runtime = app.state::<Arc<LlmRuntimeManager>>().inner().clone();
    let (size, url) = runtime
        .model_source(&model_id)
        .ok_or_else(|| format!("Unbekanntes Modell: {model_id}"))?;
    let context = context_tokens.unwrap_or(crate::managers::llm::DEFAULT_CONTEXT_TOKENS);
    // Metadaten: aus der Datei, wenn sie da ist; sonst aus dem Kopf der
    // entfernten Datei; sonst Daumenregel (und so ausgewiesen).
    let local = runtime.model_path(&model_id).filter(|p| p.is_file());
    let shape = match local {
        Some(path) => tokio::task::spawn_blocking(move || estimate::shape_from_file(&path))
            .await
            .ok()
            .flatten(),
        None => estimate::shape_from_url(&url).await,
    };
    let memory = tokio::task::spawn_blocking(crate::managers::llm::resources::system_memory)
        .await
        .map_err(|e| e.to_string())?;
    let gpu = memory.gpus.iter().find(|g| !g.shared);
    let (free_mb, on_gpu) = match gpu {
        Some(g) => (g.budget_mb.saturating_sub(g.used_mb), true),
        None => (memory.ram_total_mb.saturating_sub(memory.ram_used_mb), false),
    };
    let est = estimate::estimate(size, shape, context);
    let verdict = estimate::verdict(est.total_mb, free_mb, on_gpu);
    Ok(crate::managers::llm::FitReport {
        estimate: est,
        free_mb,
        on_gpu,
        verdict,
    })
}
