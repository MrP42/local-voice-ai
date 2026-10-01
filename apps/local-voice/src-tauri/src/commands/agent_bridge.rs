//! Kommandos der Agentenbruecke (A7): Zugaenge anlegen, zurueckziehen, entfernen,
//! Werkzeugrechte setzen, Zustand der Pipe.
//!
//! Die Logik steht in `crate::agent_bridge::view`; hier nur Argumente und Fehlerabbildung. Jedes
//! Kommando ist ein Schritt des Nutzers in der Oberflaeche; es gibt keinen Weg dorthin ueber MCP,
//! Pipe oder `ctl` (B1). Freigaben entscheidet `approval_decide` (A4), nicht dieses Modul.

use std::collections::HashSet;
use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::agent_bridge::clients::AgentClient;
use crate::agent_bridge::runtime::{AppBridge, BridgeStatus};
use crate::agent_bridge::view::{self, AgentClientView, NewAgentClient};
use crate::managers::integrations::model::GrantMode;
use crate::managers::meetings::store::MeetingStore;

fn conn(store: &MeetingStore) -> Result<rusqlite::Connection, String> {
    store.get_connection().map_err(|e| e.to_string())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Werkzeuge, die diese App-Version ausfuehren kann (leer, wenn die Bruecke nicht laeuft).
fn available(app: &AppHandle) -> HashSet<String> {
    app.try_state::<Arc<AppBridge>>()
        .map(|b| b.available_tools())
        .unwrap_or_default()
}

/// Legt einen Zugang an. Das Token steht NUR in dieser Antwort (gespeichert wird sein Hash);
/// der Nutzer traegt es in den Agenten ein. `integration_id` `None`: die Agent-Integration
/// „Externe Agenten“ (wird bei Bedarf angelegt).
#[tauri::command]
#[specta::specta]
pub async fn agent_client_create(
    store: State<'_, Arc<MeetingStore>>,
    label: String,
    integration_id: Option<String>,
) -> Result<NewAgentClient, String> {
    let conn = conn(&store)?;
    view::create_client(&conn, &label, integration_id.as_deref(), now_ms()).map_err(|e| e.to_string())
}

/// Alle Zugaenge (auch zurueckgezogene) mit ihren Werkzeugen: Recht des Zugangs, wirksames
/// Recht und Grund, wenn es „aus“ ist.
#[tauri::command]
#[specta::specta]
pub async fn agent_client_list(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
) -> Result<Vec<AgentClientView>, String> {
    let conn = conn(&store)?;
    view::client_views(&conn, &available(&app)).map_err(|e| e.to_string())
}

/// Zieht einen Zugang zurueck: sein Token gilt sofort nicht mehr, auch nicht in einer
/// laufenden Verbindung.
#[tauri::command]
#[specta::specta]
pub async fn agent_client_revoke(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<AgentClient, String> {
    let conn = conn(&store)?;
    view::revoke_client(&conn, &id, now_ms()).map_err(|e| e.to_string())
}

/// Entfernt einen Zugang samt seinen Werkzeugrechten.
#[tauri::command]
#[specta::specta]
pub async fn agent_client_delete(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<(), String> {
    let conn = conn(&store)?;
    view::delete_client(&conn, &id, now_ms()).map_err(|e| e.to_string())
}

/// Setzt das Recht eines Zugangs fuer ein Werkzeug (`off`, `ask`, `allow`). „Aufnahme starten“
/// nie `allow`. Die Antwort ist die aktualisierte Ansicht des Zugangs.
#[tauri::command]
#[specta::specta]
pub async fn agent_client_set_tool_mode(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    client_id: String,
    tool: String,
    mode: GrantMode,
) -> Result<AgentClientView, String> {
    let conn = conn(&store)?;
    view::set_tool_mode(&conn, &client_id, &tool, mode, &available(&app), now_ms())
        .map_err(|e| e.to_string())
}

/// Laeuft die Pipe der Agentenbruecke? Sonst warum nicht.
#[tauri::command]
#[specta::specta]
pub async fn agent_bridge_status(app: AppHandle) -> BridgeStatus {
    app.try_state::<Arc<AppBridge>>()
        .map(|b| b.status())
        .unwrap_or_default()
}
