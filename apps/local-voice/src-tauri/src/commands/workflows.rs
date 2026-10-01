//! Kommandos der Oberflaeche „Automationen“ (B7, Goal Workflow-Automation, Issue #67).
//!
//! Die Rechnung steht in `managers::workflows::ui`; hier nur Argumente, der Zugriff auf die
//! Engine (`WorkflowHub`) und die Fehlerabbildung. Eine ungueltige Definition ist KEIN Fehler,
//! sondern ein Ergebnis mit allen Befunden (`WorkflowSaveResult.issues`); Fehler sind Klartext.
//!
//! Was die Oberflaeche hier NICHT kann: einen Baustein selbst ausfuehren, ein Recht erteilen,
//! eine Freigabe entscheiden (das bleibt der Freigabedialog auf der Seite „Integrationen“) oder
//! die Einwilligung zur Aufnahme geben (nur das Hinweisfenster oder dieser Dialog). Ein Start
//! von Hand geht durch dieselbe `enqueue` wie jeder Ausloeser.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value};
use tauri::State;

use crate::managers::meetings::store::MeetingStore;
use crate::managers::workflows::hub::WorkflowHub;
use crate::managers::workflows::ui::{
    self, WorkflowCatalog, WorkflowItem, WorkflowRunDetail, WorkflowRunSummary, WorkflowSaveResult,
    WorkflowStarted, WorkflowStatus, WorkflowTemplate,
};

type Hub<'a> = State<'a, Arc<WorkflowHub>>;

fn text(e: crate::managers::workflows::store::WorkflowError) -> String {
    ui::error_text(&e)
}

/// Alle Ablaeufe mit letztem Lauf und Zahl der offenen Laeufe.
#[tauri::command]
#[specta::specta]
pub async fn workflow_list(hub: Hub<'_>) -> Result<Vec<WorkflowItem>, String> {
    ui::list(hub.engine()).map_err(text)
}

/// Katalog der Ausloeser und Bausteine mit ihren Feldern (Grundlage des Formulars).
#[tauri::command]
#[specta::specta]
pub async fn workflow_catalog() -> Result<WorkflowCatalog, String> {
    Ok(ui::catalog_view())
}

/// Die mitgelieferten Vorlagen.
#[tauri::command]
#[specta::specta]
pub async fn workflow_templates() -> Result<Vec<WorkflowTemplate>, String> {
    Ok(ui::template_list())
}

/// Prueft einen Definitionstext, ohne zu speichern (Fehler mit JSON-Zeiger).
#[tauri::command]
#[specta::specta]
pub async fn workflow_validate(
    hub: Hub<'_>,
    definition_json: String,
) -> Result<Vec<ui::WorkflowIssue>, String> {
    Ok(ui::validate_text(hub.engine(), &definition_json))
}

/// Speichert einen Ablauf (`id = None`: neu, ausgeschaltet, im Trockenlauf).
#[tauri::command]
#[specta::specta]
pub async fn workflow_save(
    hub: Hub<'_>,
    id: Option<String>,
    definition_json: String,
) -> Result<WorkflowSaveResult, String> {
    ui::save(hub.engine(), id.as_deref(), &definition_json).map_err(text)
}

#[tauri::command]
#[specta::specta]
pub async fn workflow_delete(hub: Hub<'_>, id: String) -> Result<(), String> {
    ui::delete(hub.engine(), &id).map_err(text)
}

#[tauri::command]
#[specta::specta]
pub async fn workflow_set_enabled(
    hub: Hub<'_>,
    id: String,
    enabled: bool,
) -> Result<WorkflowItem, String> {
    ui::set_enabled(hub.engine(), &id, enabled).map_err(text)
}

/// Scharf schalten (`armed = true`) oder zurueck in den Trockenlauf.
#[tauri::command]
#[specta::specta]
pub async fn workflow_set_armed(
    hub: Hub<'_>,
    id: String,
    armed: bool,
) -> Result<WorkflowItem, String> {
    ui::set_armed(hub.engine(), &id, armed).map_err(text)
}

/// Trockenlauf einer Definition (auch einer noch nicht gespeicherten) als JSON-Text:
/// jeder Schritt mit Bedingung, geplanter Wirkung und Rechte-Ergebnis. Schreibt nichts.
#[tauri::command]
#[specta::specta]
pub async fn workflow_plan(
    hub: Hub<'_>,
    store: State<'_, Arc<MeetingStore>>,
    definition_json: String,
    workflow_id: Option<String>,
) -> Result<String, String> {
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    Ok(ui::plan_text(
        hub.engine(),
        &conn,
        &definition_json,
        workflow_id.as_deref(),
    ))
}

/// Startet einen Ablauf von Hand. `dry_run = true`: er plant nur. `vars_json`: Werte fuer die
/// deklarierten Variablen als JSON-Objekt.
#[tauri::command]
#[specta::specta]
pub async fn workflow_run_start(
    hub: Hub<'_>,
    id: String,
    dry_run: bool,
    vars_json: Option<String>,
) -> Result<WorkflowStarted, String> {
    let vars: Map<String, Value> = match vars_json.as_deref().filter(|s| !s.trim().is_empty()) {
        None => Map::new(),
        Some(s) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Object(m)) => m,
            _ => return Err("Die Werte der Variablen müssen ein JSON-Objekt sein.".to_string()),
        },
    };
    ui::start(hub.engine(), &id, vars, dry_run).map_err(text)
}

/// Laeufe, neueste zuerst; optional eines Ablaufs und nur offene.
#[tauri::command]
#[specta::specta]
pub async fn workflow_runs(
    hub: Hub<'_>,
    workflow_id: Option<String>,
    open_only: bool,
    limit: Option<i64>,
) -> Result<Vec<WorkflowRunSummary>, String> {
    ui::runs(
        hub.engine(),
        workflow_id.as_deref(),
        open_only,
        limit.unwrap_or(50),
    )
    .map_err(text)
}

/// Ein Lauf mit allen Schritten, Versuchen und der Herkunft der Ausgaben.
#[tauri::command]
#[specta::specta]
pub async fn workflow_run_detail(
    hub: Hub<'_>,
    run_id: String,
) -> Result<WorkflowRunDetail, String> {
    ui::run_detail(hub.engine(), &run_id).map_err(text)
}

/// Bricht einen Lauf ab. `true`: sofort; `false`: er endet am naechsten Schrittwechsel.
#[tauri::command]
#[specta::specta]
pub async fn workflow_run_cancel(hub: Hub<'_>, run_id: String) -> Result<bool, String> {
    ui::cancel_run(hub.engine(), &run_id).map_err(text)
}

/// Wiederholt einen gescheiterten Lauf ab dem gescheiterten Schritt. Bei unklarer Wirkung
/// (`retry_needs_confirmation`) nur mit `accept_uncertain = true`.
#[tauri::command]
#[specta::specta]
pub async fn workflow_run_retry(
    hub: Hub<'_>,
    run_id: String,
    accept_uncertain: bool,
) -> Result<(), String> {
    ui::retry_run(hub.engine(), &run_id, accept_uncertain).map_err(text)
}

/// Nach einer Entscheidung im Freigabedialog: Engine wecken und das Hinweisfenster der
/// naechsten Bitte zeigen, damit der Lauf ohne Wartezeit weitergeht.
#[tauri::command]
#[specta::specta]
pub async fn workflow_approvals_changed(hub: Hub<'_>) -> Result<(), String> {
    hub.after_decision();
    Ok(())
}

/// Die Definition eines Ablaufs als lesbarer JSON-Text.
#[tauri::command]
#[specta::specta]
pub async fn workflow_export(hub: Hub<'_>, id: String) -> Result<String, String> {
    ui::export(hub.engine(), &id).map_err(text)
}

/// Schreibt den Export in eine vom Nutzer gewaehlte Datei.
#[tauri::command]
#[specta::specta]
pub async fn workflow_export_file(hub: Hub<'_>, id: String, path: String) -> Result<(), String> {
    ui::export_to_file(hub.engine(), &id, Path::new(&path))
}

/// Liest eine Importdatei (hoechstens 256 KiB, UTF-8) und gibt den Text zurueck; importiert
/// wird erst mit `workflow_import` nach der Pruefung durch den Nutzer.
#[tauri::command]
#[specta::specta]
pub async fn workflow_read_file(path: String) -> Result<String, String> {
    ui::read_import_file(Path::new(&path))
}

/// Importiert eine Definition als neuen Ablauf (ausgeschaltet, im Trockenlauf).
#[tauri::command]
#[specta::specta]
pub async fn workflow_import(
    hub: Hub<'_>,
    definition_json: String,
) -> Result<WorkflowSaveResult, String> {
    ui::import(hub.engine(), &definition_json).map_err(text)
}

/// Ordner-Platzhalter (nur in der Cloud) und Kanal-Ausfaelle.
#[tauri::command]
#[specta::specta]
pub async fn workflow_status(hub: Hub<'_>) -> Result<WorkflowStatus, String> {
    Ok(ui::status(
        hub.engine(),
        hub.cloud_only_files(),
        hub.channel_status(),
    ))
}
