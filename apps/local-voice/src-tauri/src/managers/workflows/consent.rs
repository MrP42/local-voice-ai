//! Die Bitte um Einwilligung zur Aufnahme (B2, AK4): welche Freigaben des Bausteins
//! `recording.start` offen sind, zu welchem Ablauf und Termin sie gehoeren, und die
//! Entscheidung des Nutzers.
//!
//! Das Hinweisfenster (`meeting_prompt`) zeigt die Bitte und ruft `decide`. `decide`
//! entscheidet NUR Freigaben dieser einen Art (Faehigkeit `recording.start`, Aufrufer `workflow`
//! oder, A8, ein externer Agent mit dem Werkzeug `start_recording`): ein Fenster, das eine
//! beliebige Freigabe-Kennung schickt, kann damit keine Mail genehmigen und kein Beenden einer
//! Aufnahme freigeben.
//!
//! A8: Bittet ein externer Agent ueber die Agentenbruecke um eine Aufnahme (`start_recording`),
//! erscheint dieselbe Bitte im selben Fenster mit demselben Haekchen („Alle Beteiligten haben
//! zugestimmt“); ohne Klick geschieht nichts, und der Agent bekommt nach 30 s `pending`. Die Engine merkt die Entscheidung beim naechsten Takt
//! (`poll_approvals`) und setzt den Lauf fort; `Engine::wake` beschleunigt das.
//!
//! Verfallene, zurueckgezogene (Lauf abgebrochen) und schon entschiedene Freigaben sind
//! nicht mehr offen: `decide` meldet dann `NotPending`, das Fenster schliesst sich.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::agent_bridge::clients as agent_clients;
use crate::managers::integrations::approvals::{self, ApprovalError};
use crate::managers::integrations::model::Approval;

use super::model::RunState;
use super::store;

pub const CALLER: &str = "workflow";
/// A8: ein externer Agent ueber die Agentenbruecke.
pub const AGENT_CALLER: &str = "agent_external";
/// Das Werkzeug der Agentenbruecke, dessen Freigabe eine Bitte um Einwilligung ist.
pub const AGENT_START_TOOL: &str = "start_recording";
pub const CAPABILITY: &str = "recording.start";

/// Eine offene Bitte um Einwilligung samt dem, was das Fenster zeigt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consent {
    pub approval_id: String,
    /// Wann die Freigabe angelegt wurde (ms UTC).
    pub created_at: i64,
    pub run_id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    /// Kalendertermin des Ausloesers, falls es einer ist.
    pub event_key: Option<String>,
    /// Titel des Termins bzw. der Besprechung aus den Ausloeserdaten (beim Agenten: sein Titel).
    pub title: Option<String>,
    /// A8: die Bitte kommt von einem externen Agenten; `workflow_name` ist dann sein Name,
    /// `workflow_id` die Kennung des Zugangs und `run_id` leer.
    pub agent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsentError {
    /// Es gibt keine Freigabe mit dieser Kennung.
    NotFound,
    /// Die Freigabe gehoert nicht zu einer Aufnahme (andere Faehigkeit oder anderer Aufrufer).
    NotAConsent,
    /// Schon entschieden, verfallen oder zurueckgezogen.
    NotPending,
    Store(String),
}

impl ConsentError {
    /// Code fuer die Oberflaeche (`"<code>"`, siehe Fehlerkonvention der Befehle).
    pub fn code(&self) -> &'static str {
        match self {
            ConsentError::NotFound | ConsentError::NotPending => "consent_not_pending",
            ConsentError::NotAConsent => "consent_invalid",
            ConsentError::Store(_) => "store_failed",
        }
    }
}

impl std::fmt::Display for ConsentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsentError::NotFound | ConsentError::NotPending => {
                write!(f, "Die Anfrage ist nicht mehr offen.")
            }
            ConsentError::NotAConsent => write!(f, "Das ist keine Anfrage zur Aufnahme."),
            ConsentError::Store(m) => write!(f, "Speicherfehler: {m}"),
        }
    }
}

impl std::error::Error for ConsentError {}

fn store_err(e: impl std::fmt::Display) -> ConsentError {
    ConsentError::Store(e.to_string())
}

/// Der Lauf, dessen Schritt auf diese Freigabe wartet.
fn run_waiting_for(
    conn: &Connection,
    approval_id: &str,
) -> Result<Option<store::RunRow>, ConsentError> {
    let run_id: Option<String> = conn
        .query_row(
            "SELECT run_id FROM workflow_run_steps WHERE approval_id = ?1
             ORDER BY started_at DESC, attempt DESC LIMIT 1",
            params![approval_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_err)?;
    let Some(run_id) = run_id else {
        return Ok(None);
    };
    Ok(store::get_run(conn, &run_id)
        .map_err(store_err)?
        .filter(|r| r.state == RunState::AwaitingApproval))
}

/// Der Titel aus der Vorschau einer Freigabe (`• title: ...`), falls der Agent einen nannte.
fn preview_title(preview: Option<&str>) -> Option<String> {
    preview?
        .lines()
        .find_map(|l| l.strip_prefix("• title: "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// A8: die Bitte eines Agenten. `None`, wenn die Freigabe zu einem anderen Werkzeug gehoert
/// (Beenden einer Aufnahme ist keine Einwilligung zum Aufnehmen) oder nicht zugeordnet ist.
fn agent_consent_of(conn: &Connection, a: &Approval) -> Result<Option<Consent>, ConsentError> {
    let Some((client_id, tool)) = agent_clients::approval_link(conn, &a.id).map_err(store_err)?
    else {
        return Ok(None);
    };
    if tool != AGENT_START_TOOL {
        return Ok(None);
    }
    let label = agent_clients::get(conn, &client_id)
        .map_err(store_err)?
        .map(|c| c.label)
        .unwrap_or_else(|| client_id.clone());
    Ok(Some(Consent {
        approval_id: a.id.clone(),
        created_at: a.created_at,
        run_id: String::new(),
        workflow_id: client_id,
        workflow_name: label,
        event_key: None,
        title: preview_title(a.args_preview.as_deref()),
        agent: true,
    }))
}

fn consent_of(conn: &Connection, a: &Approval) -> Result<Option<Consent>, ConsentError> {
    if a.caller == AGENT_CALLER {
        return agent_consent_of(conn, a);
    }
    let Some(run) = run_waiting_for(conn, &a.id)? else {
        return Ok(None);
    };
    let ctx: Value = serde_json::from_str(&run.context_json).unwrap_or(Value::Null);
    let text = |key: &str| {
        ctx["trigger"][key]
            .as_str()
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    };
    Ok(Some(Consent {
        approval_id: a.id.clone(),
        created_at: a.created_at,
        run_id: run.id,
        workflow_id: run.workflow_id,
        workflow_name: run.workflow_name,
        event_key: text("event_id"),
        title: text("title"),
        agent: false,
    }))
}

/// Alle offenen Bitten um Einwilligung, aelteste zuerst. Eine Freigabe, zu der (noch) kein
/// wartender Lauf gehoert, fehlt hier: der Takt findet sie, sobald der Lauf sie vermerkt hat.
pub fn pending(conn: &Connection, now_ms: i64) -> Result<Vec<Consent>, ConsentError> {
    let mut out = Vec::new();
    for a in approvals::list_pending(conn, now_ms).map_err(store_err)? {
        if (a.caller != CALLER && a.caller != AGENT_CALLER) || a.tool_or_capability != CAPABILITY {
            continue;
        }
        if let Some(c) = consent_of(conn, &a)? {
            out.push(c);
        }
    }
    Ok(out)
}

/// Ist die Bitte noch offen?
pub fn is_pending(conn: &Connection, approval_id: &str, now_ms: i64) -> bool {
    pending(conn, now_ms)
        .map(|p| p.iter().any(|c| c.approval_id == approval_id))
        .unwrap_or(false)
}

/// Der Nutzer entscheidet: `approve = true` ist die Einwilligung („Aufnahme starten“),
/// `false` das Nein („Nicht aufnehmen“). Nur Freigaben des Bausteins `recording.start`.
pub fn decide(
    conn: &Connection,
    approval_id: &str,
    approve: bool,
    now_ms: i64,
) -> Result<(), ConsentError> {
    let Some(a) = approvals::get(conn, approval_id).map_err(store_err)? else {
        return Err(ConsentError::NotFound);
    };
    if (a.caller != CALLER && a.caller != AGENT_CALLER) || a.tool_or_capability != CAPABILITY {
        return Err(ConsentError::NotAConsent);
    }
    if a.caller == AGENT_CALLER && agent_consent_of(conn, &a)?.is_none() {
        // Nur die Bitte „Aufnahme starten“ ist eine Einwilligung, nie ein „Beenden“.
        return Err(ConsentError::NotAConsent);
    }
    approvals::decide(conn, approval_id, approve, now_ms)
        .map(|_| ())
        .map_err(|e| match e {
            ApprovalError::NotFound => ConsentError::NotFound,
            ApprovalError::Store(m) => ConsentError::Store(m),
            _ => ConsentError::NotPending,
        })
}

#[cfg(test)]
mod tests;
