//! Ansichten und Nutzeraktionen fuer die Tauri-Commands (`commands/agent_bridge.rs`, A7):
//! Zugaenge anlegen, zurueckziehen, entfernen, Werkzeugrechte setzen, Liste mit
//! WIRKSAMEN Rechten.
//!
//! Das sind Schritte des Nutzers in der Oberflaeche. Es gibt keinen Weg dorthin ueber Pipe,
//! MCP oder `ctl`. Jede Aenderung steht im Audit (Aufrufer `user`), Best Effort: scheitert der
//! Eintrag (Platte voll), geht die Aktion des Nutzers trotzdem durch und die Warnung steht im Log
//! (anders als beim Tor, das fuer Agenten fail-closed ist).
//!
//! Die Liste rechnet die wirksamen Rechte mit derselben Funktion wie der Aufruf
//! (`bridge::tool_rights`): die Oberflaeche zeigt nie etwas anderes, als das Tor entscheidet. Sie
//! nennt auch den Grund, wenn ein Recht „aus“ ist (z. B. Obergrenze der Agent-Integration „aus“).

use std::collections::HashSet;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;

use super::bridge::tool_rights;
use super::catalog::CATALOG;
use super::clients::{self, AgentClient};
use crate::managers::integrations::audit;
use crate::managers::integrations::model::{
    AuditOutcome, Capability, GrantMode, IntegrationError, NewAudit,
};

/// Ein Werkzeug aus Sicht eines Zugangs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct ToolView {
    pub name: String,
    pub title: String,
    pub description: String,
    pub capability: Capability,
    /// Recht des Zugangs (ohne Zeile: `off`).
    pub client_mode: GrantMode,
    /// Was das Tor tatsaechlich entscheidet (Obergrenze der Agent-Integration eingerechnet).
    pub effective_mode: GrantMode,
    /// Warum `effective_mode` „aus“ ist (`grant_off`, `tool_off`, `integration_disabled`, ...).
    pub off_reason: Option<String>,
    /// Hat diese App-Version das Werkzeug? Ohne Handler fehlt es in `tools/list`, auch wenn erlaubt.
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentClientView {
    pub client: AgentClient,
    pub tools: Vec<ToolView>,
}

/// Ergebnis von `create`: der Zugang und sein Token (nur dieses eine Mal sichtbar).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct NewAgentClient {
    pub client: AgentClient,
    pub token: String,
}

fn audit_user(
    conn: &Connection,
    client: &AgentClient,
    capability: Option<&str>,
    detail: serde_json::Value,
    now_ms: i64,
) {
    let entry = NewAudit {
        caller: "user".to_string(),
        integration_id: Some(client.integration_id.clone()),
        capability: capability.map(str::to_string),
        target: Some(format!("{} ({})", client.label, client.id)),
        outcome: AuditOutcome::Ok,
        detail: Some(detail),
    };
    if let Err(e) = audit::record_at(conn, &entry, now_ms) {
        log::warn!("agent_bridge: Audit der Nutzeraktion nicht geschrieben: {e}");
    }
}

pub fn view_of(
    conn: &Connection,
    client: &AgentClient,
    available: &HashSet<String>,
) -> Result<AgentClientView, IntegrationError> {
    let stored = clients::tool_modes(conn, &client.id)?;
    let mut tools = Vec::with_capacity(CATALOG.len());
    for entry in CATALOG {
        let (effective, reason) = tool_rights(conn, client, entry)?;
        tools.push(ToolView {
            name: entry.name.to_string(),
            title: entry.title.to_string(),
            description: entry.description.to_string(),
            capability: entry.capability,
            client_mode: stored.get(entry.name).copied().unwrap_or(GrantMode::Off),
            effective_mode: effective,
            off_reason: reason.map(|r| r.as_str().to_string()),
            available: available.contains(entry.name),
        });
    }
    Ok(AgentClientView {
        client: client.clone(),
        tools,
    })
}

/// Alle Zugaenge (auch zurueckgezogene) mit ihren Werkzeugen.
pub fn client_views(
    conn: &Connection,
    available: &HashSet<String>,
) -> Result<Vec<AgentClientView>, IntegrationError> {
    clients::list(conn)?
        .iter()
        .map(|c| view_of(conn, c, available))
        .collect()
}

pub fn create_client(
    conn: &Connection,
    label: &str,
    integration_id: Option<&str>,
    now_ms: i64,
) -> Result<NewAgentClient, IntegrationError> {
    let (client, token) = clients::create(conn, label, integration_id, now_ms)?;
    audit_user(conn, &client, None, json!({ "phase": "agent_client_created" }), now_ms);
    Ok(NewAgentClient { client, token })
}

pub fn revoke_client(
    conn: &Connection,
    id: &str,
    now_ms: i64,
) -> Result<AgentClient, IntegrationError> {
    let changed = clients::revoke(conn, id, now_ms)?;
    let client = clients::get(conn, id)?.ok_or_else(|| IntegrationError::NotFound(id.to_string()))?;
    if changed {
        audit_user(conn, &client, None, json!({ "phase": "agent_client_revoked" }), now_ms);
    }
    Ok(client)
}

pub fn delete_client(conn: &Connection, id: &str, now_ms: i64) -> Result<(), IntegrationError> {
    let client = clients::get(conn, id)?.ok_or_else(|| IntegrationError::NotFound(id.to_string()))?;
    clients::delete(conn, id)?;
    audit_user(conn, &client, None, json!({ "phase": "agent_client_deleted" }), now_ms);
    Ok(())
}

pub fn set_tool_mode(
    conn: &Connection,
    client_id: &str,
    tool: &str,
    mode: GrantMode,
    available: &HashSet<String>,
    now_ms: i64,
) -> Result<AgentClientView, IntegrationError> {
    clients::set_tool_mode(conn, client_id, tool, mode)?;
    let client =
        clients::get(conn, client_id)?.ok_or_else(|| IntegrationError::NotFound(client_id.to_string()))?;
    let capability = super::catalog::find(tool).map(|e| e.capability.as_str());
    audit_user(
        conn,
        &client,
        capability,
        json!({ "phase": "agent_tool_mode", "tool": tool, "mode": mode.as_str() }),
        now_ms,
    );
    view_of(conn, &client, available)
}

#[cfg(test)]
mod tests;
