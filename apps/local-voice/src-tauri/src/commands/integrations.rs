//! Kommandos der Seite „Integrationen“ (A4, Goal „Integrationen“, Issue #66).
//!
//! Die Logik steht in `crate::managers::integrations::view`; hier nur Argumente
//! und Fehlerabbildung. Jedes Kommando ist ein Schritt des Nutzers in der
//! Oberflaeche: Rechte aendern, Freigaben entscheiden und Integrationen
//! anlegen oder entfernen gibt es nur hier, nie ueber MCP oder die Pipe der
//! Agenten (B1). Fehler gehen als Code oder Klartext an die Oberflaeche.

use std::sync::Arc;

use tauri::State;

use crate::managers::integrations::model::{
    Approval, AuditEntry, Caller, Capability, Direction, GrantMode, Integration, Kind,
};
use crate::managers::integrations::secrets;
use crate::managers::integrations::view::{self, IntegrationView, PendingApproval, TestResult};
use crate::managers::meetings::store::MeetingStore;

fn conn(store: &MeetingStore) -> Result<rusqlite::Connection, String> {
    store.get_connection().map_err(|e| e.to_string())
}

fn secret_state(i: &Integration, slot: &str) -> String {
    secrets::status_label(&secrets::status(i, slot)).to_string()
}

/// Alle Integrationen mit Rechte-Matrix, Richtungen und Zustand der Geheimnisse.
#[tauri::command]
#[specta::specta]
pub async fn integrations_list(
    store: State<'_, Arc<MeetingStore>>,
) -> Result<Vec<IntegrationView>, String> {
    let conn = conn(&store)?;
    view::list_views(&conn, &secret_state, view::now_ms()).map_err(|e| e.to_string())
}

/// Legt eine Integration an. Heute nur ein Ordner (`kind = folder`); Fehler sind
/// Codes (`folder_path_missing`, `folder_path_relative`, `folder_path_not_found`,
/// `folder_path_not_a_folder`, `kind_not_available`) oder der Klartext des Registers.
#[tauri::command]
#[specta::specta]
pub async fn integration_create(
    store: State<'_, Arc<MeetingStore>>,
    kind: Kind,
    label: String,
    direction: Option<Direction>,
    path: Option<String>,
) -> Result<IntegrationView, String> {
    let conn = conn(&store)?;
    let now = view::now_ms();
    let config = serde_json::json!({ "path": path.unwrap_or_default() });
    let created = view::create_from_ui(&conn, kind, &label, direction, config, now)?;
    view::get_view(&conn, &created.id, &secret_state, now).map_err(|e| e.to_string())
}

/// Aendert Name, Schalter oder Richtung. Kalenderquellen: Name und Schalter gehoeren
/// dem Kalender.
#[tauri::command]
#[specta::specta]
pub async fn integration_update(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    label: Option<String>,
    enabled: Option<bool>,
    direction: Option<Direction>,
) -> Result<IntegrationView, String> {
    let conn = conn(&store)?;
    let now = view::now_ms();
    view::update_from_ui(&conn, &id, label, enabled, direction, now).map_err(|e| e.to_string())?;
    view::get_view(&conn, &id, &secret_state, now).map_err(|e| e.to_string())
}

/// Entfernt eine Integration samt Rechten und Geheimnissen. Das Audit-Log bleibt.
#[tauri::command]
#[specta::specta]
pub async fn integration_delete(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<(), String> {
    let conn = conn(&store)?;
    let gone = view::delete_from_ui(&conn, &id, view::now_ms()).map_err(|e| e.to_string())?;
    secrets::delete_all(&gone);
    Ok(())
}

/// Setzt das Recht eines Aufrufers fuer eine Faehigkeit (`mode = None`: zurueck
/// auf die Vorgabe). „Aufnahme starten“ lehnt `allow` ab.
#[tauri::command]
#[specta::specta]
pub async fn integration_set_grant(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    capability: Capability,
    caller: Caller,
    mode: Option<GrantMode>,
) -> Result<IntegrationView, String> {
    let conn = conn(&store)?;
    let now = view::now_ms();
    view::set_grant_from_ui(&conn, &id, capability, caller, mode, now)
        .map_err(|e| e.to_string())?;
    view::get_view(&conn, &id, &secret_state, now).map_err(|e| e.to_string())
}

/// Probiert die Verbindung aus (heute: der Ordner).
#[tauri::command]
#[specta::specta]
pub async fn integration_test(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<TestResult, String> {
    let conn = conn(&store)?;
    view::test_integration(&conn, &id, view::now_ms()).map_err(|e| e.to_string())
}

/// Das Protokoll: die neuesten Eintraege zuerst, hoechstens `limit` (1 bis 500).
#[tauri::command]
#[specta::specta]
pub async fn integrations_audit_list(
    store: State<'_, Arc<MeetingStore>>,
    integration_id: Option<String>,
    outcome: Option<String>,
    caller: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<AuditEntry>, String> {
    let conn = conn(&store)?;
    view::audit_entries(&conn, integration_id, outcome, caller, limit.unwrap_or(200))
        .map_err(|e| e.to_string())
}

/// Offene Freigaben („fragen“) mit dem Namen der Integration.
#[tauri::command]
#[specta::specta]
pub async fn approvals_pending(
    store: State<'_, Arc<MeetingStore>>,
) -> Result<Vec<PendingApproval>, String> {
    let conn = conn(&store)?;
    view::pending_approvals(&conn, view::now_ms()).map_err(|e| e.to_string())
}

/// Der Nutzer entscheidet eine Freigabe. Fehler: `approval_not_found`,
/// `approval_expired`, `approval_already_decided`.
#[tauri::command]
#[specta::specta]
pub async fn approval_decide(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    approve: bool,
) -> Result<Approval, String> {
    let conn = conn(&store)?;
    view::decide_approval(&conn, &id, approve, view::now_ms())
}
