//! Sprachmodell-Verbindungen und freigegebene Modelle (Schema 2).
//!
//! Die Oberflaeche verwaltet Verbindungen (ein Konto bei einem Anbieter, eine
//! lokale Laufzeit) und gibt je Verbindung einzelne Modelle frei. Was hier
//! gespeichert wird, spiegelt `AppSettings::sync_legacy_from_llm` in die
//! aelteren Felder, von denen Nachbearbeitung, Zusammenfassung, Protokoll und
//! Tagging heute noch lesen -- so wechseln alle Funktionen gemeinsam.

use tauri::AppHandle;

use crate::managers::llm::updates::{self, ModelUpdate};
use crate::settings::{self, AppSettings, LlmConnection, LlmModelConfig, PostProcessProvider};

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
        // Regelwerk: ein gesperrtes Modell ist nicht waehlbar -- sonst liefe
        // jede Funktion gegen die Sperre in `llm_client`.
        if let Some(c) = s
            .llm_models
            .iter()
            .find(|m| m.id == *wanted)
            .and_then(|m| s.llm_connections.iter().find(|c| c.id == m.connection_id))
        {
            let a = crate::commands::compliance::assess_connection(&s, c);
            if a.verdict == crate::managers::compliance::Verdict::Blocked {
                return Err(format!(
                    "{}: {}",
                    crate::managers::compliance::CODE_COMPLIANCE_BLOCKED,
                    a.reasons.join(", ")
                ));
            }
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
pub(crate) async fn offered_models(
    s: &AppSettings,
    connection: &LlmConnection,
) -> Result<Vec<String>, String> {
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
    offered_models(&s, connection).await
}

// ---- Neue Modelle der Anbieter (managers::llm::updates) -------------------

/// Ergebnis einer Pruefung auf neue Modelle.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, specta::Type)]
pub struct ModelCheck {
    /// Einstellung `llm_auto_update_models` zum Zeitpunkt der Pruefung.
    pub mode: String,
    /// Ohne Rueckfrage uebernommen (nur bei `on`).
    pub applied: Vec<ModelUpdate>,
    /// Warten auf die Antwort des Nutzers (nur bei `ask`).
    pub pending: Vec<ModelUpdate>,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Uebernimmt Verlaufseintraege in die frischen Einstellungen -- ohne eine
/// Entscheidung zu ueberschreiben, die der Nutzer zwischenzeitlich getroffen hat.
fn merge_history(s: &mut AppSettings, entries: &[settings::LlmModelSeen]) {
    for e in entries {
        let decided = e.status == updates::STATUS_ABSENT
            && s.llm_model_history
                .iter()
                .any(|h| h.key == e.key && h.status != updates::STATUS_ABSENT);
        if !decided {
            updates::set_history(
                &mut s.llm_model_history,
                &e.key,
                &e.status,
                e.at,
                e.replaced_by.clone(),
            );
        }
    }
}

static MODEL_CHECK_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

struct RunningGuard;
impl Drop for RunningGuard {
    fn drop(&mut self) {
        MODEL_CHECK_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Fragt die eingeschalteten, nicht gesperrten Verbindungen nach ihren Modellen
/// (das Claude-Abo durch Ausprobieren der naheliegenden Nachfolger), vermerkt
/// Neues im Verlauf und uebernimmt es je nach Einstellung. Still bei Fehlern.
#[tauri::command]
#[specta::specta]
pub async fn llm_check_new_models(app: AppHandle) -> Result<ModelCheck, String> {
    use crate::managers::llm::cli::{self, Cli};
    use std::sync::atomic::Ordering;
    let first = !MODEL_CHECK_RUNNING.swap(true, Ordering::SeqCst);
    // Laeuft schon eine Pruefung, antwortet dieser Aufruf nur mit dem Stand.
    let _guard = first.then_some(RunningGuard);
    let now = unix_now();
    let s0 = settings::get_settings(&app);
    updates::restore_claude_extras(&s0);
    let mut touched: Vec<settings::LlmModelSeen> = Vec::new();

    if first {
        // Arbeitskopie, damit die zweite Runde sieht, was die erste vermerkt hat.
        let mut work = s0.clone();
        let connections: Vec<LlmConnection> =
            s0.llm_connections.iter().filter(|c| c.enabled).cloned().collect();
        for c in connections {
            if crate::commands::compliance::assess_connection(&work, &c).verdict
                == crate::managers::compliance::Verdict::Blocked
            {
                continue;
            }
            let Some(template) = work.llm_template(&c).cloned() else {
                continue;
            };
            if crate::managers::llm::is_local(&template) {
                continue;
            }
            let Ok(offered) = offered_models(&work, &c).await else {
                log::debug!("Modellpruefung: {} nicht erreichbar", c.label);
                continue;
            };
            // Erste Runde: das Angebot des Anbieters. Sie setzt auch die
            // Grundlinie, bevor ein Probelauf etwas "Neues" finden kann.
            let seen = updates::record_offered(&work, &c, &offered, c.kind == "ollama", now);
            merge_history(&mut work, &seen);
            touched.extend(seen);

            if Cli::from_base_url(&c.base_url) != Some(Cli::Claude) {
                continue;
            }
            let mut known = offered.clone();
            known.extend(
                work.llm_models
                    .iter()
                    .filter(|m| m.connection_id == c.id)
                    .map(|m| m.remote_id.clone()),
            );
            let due = updates::due_candidates(&work, &c.id, &known, now);
            if due.is_empty() {
                continue;
            }
            let Some(binary) = cli::locate(Cli::Claude) else {
                continue;
            };
            let run = tokio::task::spawn_blocking(move || {
                updates::run_probes(&due, updates::PROBE_BUDGET, |m| {
                    updates::probe_claude(&binary, m)
                })
            })
            .await
            .unwrap_or_default();
            for name in run.absent {
                let e = settings::LlmModelSeen {
                    key: LlmModelConfig::make_id(&c.id, &name),
                    status: updates::STATUS_ABSENT.to_string(),
                    at: now,
                    replaced_by: None,
                };
                merge_history(&mut work, std::slice::from_ref(&e));
                touched.push(e);
            }
            // Zweite Runde: was der Probelauf gefunden hat, ist neu.
            let seen = updates::record_offered(&work, &c, &run.available, false, now);
            merge_history(&mut work, &seen);
            touched.extend(seen);
            updates::restore_claude_extras(&work);
        }
    }

    let mut s = settings::get_settings(&app);
    merge_history(&mut s, &touched);
    updates::restore_claude_extras(&s);
    let mode = s.llm_auto_update_models.clone();
    let mut applied = Vec::new();
    let mut pending = updates::pending_updates(&s);
    if mode == updates::MODE_ON {
        for u in std::mem::take(&mut pending) {
            match updates::apply(&mut s, &u, now) {
                Ok(_) => applied.push(u),
                Err(e) => log::warn!("Modell {} nicht uebernommen: {e}", u.remote_id),
            }
        }
    } else if mode == updates::MODE_OFF {
        pending.clear();
    }
    if !touched.is_empty() || !applied.is_empty() {
        settings::write_settings(&app, s);
    }
    Ok(ModelCheck {
        mode,
        applied,
        pending,
    })
}

/// Antwort auf die Frage nach neuen Modellen: `always` (uebernehmen und kuenftig
/// ohne Rueckfrage), `once` (nur dieses Mal) oder `never` (nicht uebernehmen,
/// kuenftig selbst waehlen).
#[tauri::command]
#[specta::specta]
pub fn llm_answer_model_updates(
    app: AppHandle,
    answer: String,
) -> Result<Vec<ModelUpdate>, String> {
    let mut s = settings::get_settings(&app);
    let now = unix_now();
    let mut applied = Vec::new();
    match answer.as_str() {
        "always" | "once" => {
            if answer == "always" {
                s.llm_auto_update_models = updates::MODE_ON.to_string();
            }
            for u in updates::pending_updates(&s) {
                match updates::apply(&mut s, &u, now) {
                    Ok(_) => applied.push(u),
                    Err(e) => log::warn!("Modell {} nicht uebernommen: {e}", u.remote_id),
                }
            }
        }
        "never" => s.llm_auto_update_models = updates::MODE_OFF.to_string(),
        other => return Err(format!("Unbekannte Antwort: {other}")),
    }
    settings::write_settings(&app, s);
    Ok(applied)
}

/// Erlaubt oder verbietet, dass die App die Codex-CLI selbst aktualisiert.
#[tauri::command]
#[specta::specta]
pub fn llm_set_cli_auto_update(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    s.cli_auto_update = enabled;
    settings::write_settings(&app, s);
    Ok(())
}

/// Stellt die Einstellung `llm_auto_update_models` (`ask`, `on`, `off`).
#[tauri::command]
#[specta::specta]
pub fn llm_set_auto_update_models(app: AppHandle, mode: String) -> Result<(), String> {
    if !updates::valid_mode(&mode) {
        return Err(format!("Unbekannte Einstellung: {mode}"));
    }
    let mut s = settings::get_settings(&app);
    s.llm_auto_update_models = mode;
    settings::write_settings(&app, s);
    Ok(())
}

/// Effort-Stufen, die ein Modell einer Verbindung annimmt (Abo ueber die CLI:
/// Claude `--effort`, Codex laut eigenem Katalog). Leer: nicht einstellbar.
#[tauri::command]
#[specta::specta]
pub fn llm_model_efforts(app: AppHandle, connection_id: String, remote_id: String) -> Vec<String> {
    let s = settings::get_settings(&app);
    s.llm_connections
        .iter()
        .find(|c| c.id == connection_id)
        .and_then(|c| crate::managers::llm::cli::Cli::from_base_url(&c.base_url))
        .map(|cli| cli.efforts(&remote_id))
        .unwrap_or_default()
}

/// Bietet ein Modell einer Verbindung den Fast-Modus an? (Codex laut Katalog)
#[tauri::command]
#[specta::specta]
pub fn llm_model_offers_fast(app: AppHandle, connection_id: String, remote_id: String) -> bool {
    let s = settings::get_settings(&app);
    s.llm_connections
        .iter()
        .find(|c| c.id == connection_id)
        .and_then(|c| crate::managers::llm::cli::Cli::from_base_url(&c.base_url))
        .is_some_and(|cli| cli.offers_fast(&remote_id))
}

/// Standard-Effort fuer Abo-Modelle ohne eigenen Effort (`low` … `max`).
#[tauri::command]
#[specta::specta]
pub fn llm_set_default_effort(app: AppHandle, effort: String) -> Result<(), String> {
    if !crate::managers::llm::cli::valid_effort(&effort) {
        return Err(format!("Unbekannte Effort-Stufe: {effort}"));
    }
    let mut s = settings::get_settings(&app);
    s.llm_default_effort = effort;
    s.sync_legacy_from_llm();
    settings::write_settings(&app, s);
    Ok(())
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
pub async fn llm_local_list(app: AppHandle) -> Vec<LlmDownloadInfo> {
    // Nicht auf dem Hauptthread: der erste Aufruf liest die Koepfe aller
    // Modelle in den Modellordnern (Ollama: zwei Dutzend Dateien).
    let manager = app.state::<Arc<LlmRuntimeManager>>().inner().clone();
    tokio::task::spawn_blocking(move || manager.list_downloads())
        .await
        .unwrap_or_default()
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
pub async fn llm_local_stop(app: AppHandle) -> Result<(), String> {
    // Nicht auf dem Hauptthread: `stop` wartet auf taskkill und Prozessende.
    let server = app.state::<Arc<LocalLlmServer>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || server.stop())
        .await
        .map_err(|e| format!("Server-Stopp abgebrochen: {e}"))?;
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
    if let Some(reason) = runtime.external_incompatibility(&model_id) {
        return Err(format!(
            "{}: {reason}",
            crate::managers::llm::CODE_EXTERNAL_INCOMPATIBLE
        ));
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
                training_opt_out: false,
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
            effort: None,
            fast: false,
        });
    } else if let Some(m) = s.llm_models.iter_mut().find(|m| m.id == id) {
        m.enabled = true;
    }
    s.llm_active_model_id = Some(id);
    s.sync_legacy_from_llm();
    settings::write_settings(&app, s);
    Ok(())
}

// ---- Modellordner (fremde GGUF-Dateien, Ollama-Speicher) ------------------

/// Die Modellordner setzen und sofort neu einlesen. Leere und doppelte
/// Eintraege fallen weg; die Reihenfolge bleibt die des Nutzers.
#[tauri::command]
#[specta::specta]
pub async fn llm_set_model_dirs(app: AppHandle, dirs: Vec<String>) -> Result<(), String> {
    let mut cleaned: Vec<String> = Vec::new();
    for d in dirs {
        let d = d.trim().to_string();
        if !d.is_empty() && !cleaned.iter().any(|c| c.eq_ignore_ascii_case(&d)) {
            cleaned.push(d);
        }
    }
    let mut s = settings::get_settings(&app);
    s.llm_model_dirs = cleaned.clone();
    settings::write_settings(&app, s);
    app.state::<Arc<LlmRuntimeManager>>()
        .set_model_dirs(cleaned);
    llm_rescan_model_dirs(app).await
}

/// Ordner, die hier wahrscheinlich Modelle enthalten: der Ollama-Speicher
/// und die Ablage von LM Studio -- als Vorschlag, eingetragen wird nichts.
#[tauri::command]
#[specta::specta]
pub fn llm_suggest_model_dirs() -> Vec<String> {
    use crate::managers::llm::external;
    external::detect_ollama_store()
        .into_iter()
        .chain(external::detect_lmstudio_dir())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Modellordner neu durchsuchen (nach `ollama pull` oder einem neuen Download
/// in LM Studio).
#[tauri::command]
#[specta::specta]
pub async fn llm_rescan_model_dirs(app: AppHandle) -> Result<(), String> {
    let manager = app.state::<Arc<LlmRuntimeManager>>().inner().clone();
    let scanner = manager.clone();
    tokio::task::spawn_blocking(move || scanner.rescan_external())
        .await
        .map_err(|e| e.to_string())?;
    // Moegliche Ersatzmodelle fuer App-Kopien gleich pruefen, damit die
    // Liste das Aufraeumen anbieten kann.
    manager.probe_duplicate_candidates().await;
    Ok(())
}

/// Ladeversuch fuer ein Modell aus einem Modellordner: laedt es mit der
/// Laufzeit der App? Das Ergebnis wird gemerkt.
#[tauri::command]
#[specta::specta]
pub async fn llm_probe_external(
    app: AppHandle,
    id: String,
) -> Result<crate::managers::llm::external::Compat, String> {
    let manager = app.state::<Arc<LlmRuntimeManager>>().inner().clone();
    manager.probe_external(&id).await
}

/// RAM und GPU-Speicherbudget fuer die Fussleiste, dazu der Anteil der App
/// (samt Kindprozessen). Auf einem Blocking-Thread: DXGI und die
/// Prozessliste sind schnell, aber nicht async; die App-Messung ist auf eine
/// je 5 s gedrosselt.
#[tauri::command]
#[specta::specta]
pub async fn system_memory(_app: AppHandle) -> Result<crate::managers::llm::SystemMemory, String> {
    tokio::task::spawn_blocking(crate::managers::llm::resources::system_memory_with_app)
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
