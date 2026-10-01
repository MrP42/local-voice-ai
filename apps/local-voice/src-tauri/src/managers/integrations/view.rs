//! Ansicht des Registers fuer die Seite „Integrationen“ (A4).
//!
//! Die Oberflaeche bekommt hier fertige Zeilen: je Integration die Faehigkeiten
//! mit dem gespeicherten und dem WIRKSAMEN Recht je Aufrufer (`grants::explain`,
//! also dieselbe Regel wie beim Tor, nie eine zweite Rechnung im Browser), dazu
//! die Richtungen, die die Art kennt. Alle Schreibwege des Nutzers laufen ueber
//! diese Datei und stehen im Audit (Aufrufer `user`): eine Rechteaenderung ist
//! nachvollziehbar, eine Freigabe-Entscheidung ebenso.
//!
//! Sicherheitsannahmen:
//! - Eine Freigabe entscheidet nur der Nutzer in der Oberflaeche: `decide_approval`
//!   ist ein Tauri-Kommando des Webviews, es gibt keinen Weg ueber MCP oder Pipe
//!   (B1).
//! - Der Audit-Eintrag zu einer Nutzeraktion ist Best Effort: scheitert er (Platte
//!   voll), geht die Aktion des Nutzers trotzdem durch und die Warnung steht im Log.
//!   Das Tor (`gate`) verlangt dagegen den Eintrag VOR jeder Aktion eines Agenten.
//! - Anlegen aus der Oberflaeche ist auf Arten beschraenkt, die schon ohne Konto
//!   funktionieren (`UI_CREATABLE`). Konten und Ziele mit Geheimnis folgen mit
//!   ihren eigenen Paketen und ihren Assistenten.
//! - Ein Ordner wird beim Anlegen geprueft (absolut, vorhanden, ein Ordner); die
//!   Sandbox gegen `..` und Junctions gehoert dem Ordner-Adapter (A6).

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::approvals::{self, ApprovalError};
use super::audit;
use super::grants::{default_mode, explain, GrantSet};
use super::model::{
    Access, Approval, AuditEntry, AuditOutcome, Caller, Capability, Direction, GrantMode,
    Integration, IntegrationError, IntegrationPatch, Kind, NewAudit, NewIntegration,
};
use super::{secrets, store};

/// Arten, die die Oberflaeche selbst anlegen darf (alle ohne Konto/Geheimnis).
pub const UI_CREATABLE: [Kind; 1] = [Kind::Folder];

/// Aufrufer-Spalten der Rechte-Matrix, in Anzeigereihenfolge.
pub const MATRIX_CALLERS: [Caller; 3] =
    [Caller::Workflow, Caller::AgentExternal, Caller::AgentLocal];

/// Das Recht EINES Aufrufers fuer EINE Faehigkeit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CallerMode {
    pub caller: Caller,
    /// Die gespeicherte Zeile; `None`: es gilt die Vorgabe (E3).
    pub stored: Option<GrantMode>,
    /// Die Vorgabe (E3), wenn keine Zeile gespeichert ist; auch dann sichtbar, wenn
    /// die Richtung die Faehigkeit sperrt und `effective` deshalb „aus“ ist.
    pub default_mode: GrantMode,
    /// Was tatsaechlich gilt (Richtung, Schalter und Sperren eingerechnet).
    pub effective: GrantMode,
    /// Maschinenlesbarer Grund, wenn `effective` = aus (`grant_off`,
    /// `direction_blocks`, `integration_disabled`, ...).
    pub off_reason: Option<String>,
}

/// Eine Zeile der Rechte-Matrix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CapabilityView {
    pub capability: Capability,
    /// Veraendert diese Faehigkeit etwas (Mail senden, Datei schreiben)?
    pub writes: bool,
    /// Nie dauerhaft erlaubt (Aufnahme starten): die Oberflaeche bietet „erlaubt“ nicht an.
    pub never_allow: bool,
    /// Erlaubt die Richtung der Integration diese Faehigkeit ueberhaupt?
    pub direction_allows: bool,
    pub modes: Vec<CallerMode>,
}

/// Ein Fach fuer ein Geheimnis und sein Zustand (`present`, `missing`, `broken`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SecretSlotView {
    pub slot: String,
    pub status: String,
}

/// Eine Integration mit allem, was die Seite zeigt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct IntegrationView {
    pub integration: Integration,
    /// Richtungen, die diese Art kennt (ICS nur lesen, SMTP nur schreiben).
    pub directions: Vec<Direction>,
    pub capabilities: Vec<CapabilityView>,
    pub secrets: Vec<SecretSlotView>,
    /// Name und Schalter gehoeren dem Kalender (Karte „Kalender“).
    pub calendar_managed: bool,
    /// Offene Freigaben zu dieser Integration.
    pub pending_approvals: u32,
}

/// Eine offene Freigabe mit dem Namen der Integration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PendingApproval {
    pub approval: Approval,
    pub integration_label: Option<String>,
    pub integration_kind: Option<Kind>,
}

/// Ergebnis von „Verbindung testen“: Code fuer die Oberflaeche, kein Freitext.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TestResult {
    pub ok: bool,
    /// `folder_ok`, `folder_path_not_found`, `folder_path_not_a_folder`,
    /// `folder_unreadable`, `test_not_available`.
    pub code: String,
}

/// Zeitstempel in Millisekunden UTC.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn view_of(
    conn: &Connection,
    i: &Integration,
    secret_status: &dyn Fn(&Integration, &str) -> String,
    pending: &[Approval],
) -> Result<IntegrationView, IntegrationError> {
    let grants: GrantSet = store::grants_for(conn, &i.id)?;
    let capabilities = i
        .kind
        .capabilities()
        .iter()
        .map(|cap| {
            let modes = MATRIX_CALLERS
                .iter()
                .map(|caller| {
                    let (effective, reason) = explain(i, *cap, *caller, &grants, None);
                    CallerMode {
                        caller: *caller,
                        stored: grants.get(&(*cap, *caller)).copied(),
                        default_mode: default_mode(*cap, *caller),
                        effective,
                        off_reason: reason.map(|r| r.as_str().to_string()),
                    }
                })
                .collect();
            CapabilityView {
                capability: *cap,
                writes: cap.access() == Access::Write,
                never_allow: cap.never_allow(),
                direction_allows: i.direction.permits(cap.access()),
                modes,
            }
        })
        .collect();
    let secrets = secrets::required_slots(i.kind)
        .iter()
        .map(|slot| SecretSlotView {
            slot: (*slot).to_string(),
            status: secret_status(i, slot),
        })
        .collect();
    let pending_approvals = pending
        .iter()
        .filter(|a| a.integration_id.as_deref() == Some(i.id.as_str()))
        .count() as u32;
    Ok(IntegrationView {
        integration: i.clone(),
        directions: i.kind.allowed_directions().to_vec(),
        capabilities,
        secrets,
        calendar_managed: i.kind.is_calendar_managed(),
        pending_approvals,
    })
}

/// Alle Integrationen als Ansicht, aelteste zuerst.
pub fn list_views(
    conn: &Connection,
    secret_status: &dyn Fn(&Integration, &str) -> String,
    now_ms: i64,
) -> Result<Vec<IntegrationView>, IntegrationError> {
    let pending = approvals::list_pending(conn, now_ms)?;
    store::list(conn)?
        .iter()
        .map(|i| view_of(conn, i, secret_status, &pending))
        .collect()
}

/// Eine Integration als Ansicht.
pub fn get_view(
    conn: &Connection,
    id: &str,
    secret_status: &dyn Fn(&Integration, &str) -> String,
    now_ms: i64,
) -> Result<IntegrationView, IntegrationError> {
    let Some(i) = store::get(conn, id)? else {
        return Err(IntegrationError::NotFound(id.to_string()));
    };
    let pending = approvals::list_pending(conn, now_ms)?;
    view_of(conn, &i, secret_status, &pending)
}

/// Schreibt, was der Nutzer getan hat, ins Audit (Best Effort, siehe Moduldoku).
fn audit_user(
    conn: &Connection,
    integration_id: &str,
    capability: Option<&str>,
    target: Option<&str>,
    detail: Value,
    now_ms: i64,
) {
    let entry = NewAudit {
        caller: Caller::User.as_str().to_string(),
        integration_id: Some(integration_id.to_string()),
        capability: capability.map(str::to_string),
        target: target.map(str::to_string),
        outcome: AuditOutcome::Ok,
        detail: Some(detail),
    };
    if let Err(e) = audit::record_at(conn, &entry, now_ms) {
        log::warn!("integrations: Audit der Nutzeraktion nicht geschrieben: {e}");
    }
}

/// Fehlercodes fuer die Ordner-Pruefung (die Oberflaeche uebersetzt sie).
pub const ERR_PATH_MISSING: &str = "folder_path_missing";
pub const ERR_PATH_RELATIVE: &str = "folder_path_relative";
pub const ERR_PATH_NOT_FOUND: &str = "folder_path_not_found";
pub const ERR_PATH_NOT_A_FOLDER: &str = "folder_path_not_a_folder";
pub const ERR_KIND_NOT_AVAILABLE: &str = "kind_not_available";

/// Prueft den Pfad einer Ordner-Integration und gibt den bereinigten Text zurueck.
fn check_folder_path(raw: &str) -> Result<String, &'static str> {
    let path = raw.trim();
    if path.is_empty() {
        return Err(ERR_PATH_MISSING);
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(ERR_PATH_RELATIVE);
    }
    match std::fs::metadata(p) {
        Err(_) => Err(ERR_PATH_NOT_FOUND),
        Ok(m) if !m.is_dir() => Err(ERR_PATH_NOT_A_FOLDER),
        Ok(_) => Ok(path.to_string()),
    }
}

/// Legt eine Integration an (nur `UI_CREATABLE`). Fehler: ein Code (`folder_*`,
/// `kind_not_available`) oder der Klartext des Registers.
pub fn create_from_ui(
    conn: &Connection,
    kind: Kind,
    label: &str,
    direction: Option<Direction>,
    config: Value,
    now_ms: i64,
) -> Result<Integration, String> {
    if !UI_CREATABLE.contains(&kind) {
        return Err(ERR_KIND_NOT_AVAILABLE.to_string());
    }
    let mut n = NewIntegration::new(kind, label);
    n.direction = direction;
    n.config = match kind {
        Kind::Folder => {
            let raw = config.get("path").and_then(Value::as_str).unwrap_or("");
            let path = check_folder_path(raw).map_err(str::to_string)?;
            json!({ "path": path })
        }
        _ => config,
    };
    let created = store::create(conn, &n, now_ms).map_err(|e| e.to_string())?;
    audit_user(
        conn,
        &created.id,
        None,
        None,
        json!({ "phase": "created", "kind": kind.as_str(), "direction": created.direction.as_str() }),
        now_ms,
    );
    Ok(created)
}

/// Aendert Name, Schalter oder Richtung. Wird die Richtung enger, bleiben die
/// gespeicherten Rechte stehen, wirken aber nicht mehr (`direction_blocks`).
pub fn update_from_ui(
    conn: &Connection,
    id: &str,
    label: Option<String>,
    enabled: Option<bool>,
    direction: Option<Direction>,
    now_ms: i64,
) -> Result<Integration, IntegrationError> {
    let before = store::get(conn, id)?.ok_or_else(|| IntegrationError::NotFound(id.to_string()))?;
    let patch = IntegrationPatch {
        label,
        enabled,
        direction,
        ..Default::default()
    };
    let after = store::update(conn, id, &patch, now_ms)?;
    let mut changes = serde_json::Map::new();
    if after.direction != before.direction {
        changes.insert("direction".into(), json!(after.direction.as_str()));
    }
    if after.enabled != before.enabled {
        changes.insert("enabled".into(), json!(after.enabled));
    }
    if after.label != before.label {
        changes.insert("renamed".into(), json!(true));
    }
    if !changes.is_empty() {
        let mut detail = json!({ "phase": "updated" });
        detail["changes"] = Value::Object(changes);
        audit_user(conn, id, None, None, detail, now_ms);
    }
    Ok(after)
}

/// Entfernt eine Integration samt Rechten. Gibt sie zurueck, damit der Aufrufer
/// ihre Geheimnisse wegraeumen kann.
pub fn delete_from_ui(
    conn: &Connection,
    id: &str,
    now_ms: i64,
) -> Result<Integration, IntegrationError> {
    let before = store::get(conn, id)?.ok_or_else(|| IntegrationError::NotFound(id.to_string()))?;
    store::delete(conn, id)?;
    audit_user(
        conn,
        id,
        None,
        None,
        json!({ "phase": "deleted", "kind": before.kind.as_str() }),
        now_ms,
    );
    Ok(before)
}

/// Setzt ein Recht; `mode = None` entfernt die gespeicherte Zeile (zurueck auf
/// die Vorgabe).
pub fn set_grant_from_ui(
    conn: &Connection,
    id: &str,
    cap: Capability,
    caller: Caller,
    mode: Option<GrantMode>,
    now_ms: i64,
) -> Result<(), IntegrationError> {
    match mode {
        Some(m) => store::set_grant(conn, id, cap, caller, m)?,
        None => {
            if store::get(conn, id)?.is_none() {
                return Err(IntegrationError::NotFound(id.to_string()));
            }
            store::clear_grant(conn, id, cap, caller)?;
        }
    }
    audit_user(
        conn,
        id,
        Some(cap.as_str()),
        None,
        json!({
            "phase": "grant_changed",
            "for": caller.as_str(),
            "mode": mode.map(GrantMode::as_str).unwrap_or("default"),
        }),
        now_ms,
    );
    Ok(())
}

/// Offene Freigaben, aelteste zuerst, mit dem Namen der Integration.
pub fn pending_approvals(
    conn: &Connection,
    now_ms: i64,
) -> Result<Vec<PendingApproval>, IntegrationError> {
    let _ = approvals::expire_stale(conn, now_ms);
    approvals::list_pending(conn, now_ms)?
        .into_iter()
        .map(|approval| {
            let integration = match approval.integration_id.as_deref() {
                Some(id) => store::get(conn, id)?,
                None => None,
            };
            Ok(PendingApproval {
                integration_label: integration.as_ref().map(|i| i.label.clone()),
                integration_kind: integration.map(|i| i.kind),
                approval,
            })
        })
        .collect()
}

/// Fehlertext fuer die Oberflaeche: ein Code (`approval_*`) oder Klartext.
fn approval_error_code(e: &ApprovalError) -> String {
    match e {
        ApprovalError::NotFound => "approval_not_found".to_string(),
        ApprovalError::Expired => "approval_expired".to_string(),
        ApprovalError::WrongState(_) => "approval_already_decided".to_string(),
        other => other.to_string(),
    }
}

/// Der Nutzer entscheidet eine Freigabe. Die Entscheidung steht im Audit.
pub fn decide_approval(
    conn: &Connection,
    id: &str,
    approve: bool,
    now_ms: i64,
) -> Result<Approval, String> {
    let decided =
        approvals::decide(conn, id, approve, now_ms).map_err(|e| approval_error_code(&e))?;
    audit_user(
        conn,
        decided.integration_id.as_deref().unwrap_or(""),
        Some(&decided.tool_or_capability),
        None,
        json!({
            "phase": "approval_decided",
            "approval_id": decided.id,
            "decision": if approve { "approved" } else { "denied" },
            "requested_by": decided.caller,
        }),
        now_ms,
    );
    Ok(decided)
}

/// Klartext zu einem Ordner-Fehlercode (steht als „letzter Fehler“ am Eintrag).
fn folder_error_text(code: &str) -> &'static str {
    match code {
        ERR_PATH_NOT_A_FOLDER => "Der Pfad ist kein Ordner.",
        "folder_unreadable" => "Der Ordner lässt sich nicht lesen.",
        _ => "Der Ordner wurde nicht gefunden.",
    }
}

/// Probiert eine Verbindung aus. Heute kann nur der Ordner getestet werden;
/// das Ergebnis steht auch am Eintrag (`last_ok_at`/`last_error`).
pub fn test_integration(
    conn: &Connection,
    id: &str,
    now_ms: i64,
) -> Result<TestResult, IntegrationError> {
    let i = store::get(conn, id)?.ok_or_else(|| IntegrationError::NotFound(id.to_string()))?;
    if i.kind != Kind::Folder {
        return Ok(TestResult {
            ok: false,
            code: "test_not_available".to_string(),
        });
    }
    let cfg: Value = serde_json::from_str(&i.config_json).unwrap_or(Value::Null);
    let path = cfg.get("path").and_then(Value::as_str).unwrap_or("");
    let code = match check_folder_path(path) {
        Err(c) if c == ERR_PATH_NOT_FOUND || c == ERR_PATH_NOT_A_FOLDER => c,
        Err(_) => ERR_PATH_NOT_FOUND,
        Ok(p) => match std::fs::read_dir(&p) {
            Ok(_) => "folder_ok",
            Err(_) => "folder_unreadable",
        },
    };
    let ok = code == "folder_ok";
    if ok {
        store::mark_ok(conn, id, now_ms)?;
    } else {
        store::mark_error(conn, id, folder_error_text(code), now_ms)?;
    }
    Ok(TestResult {
        ok,
        code: code.to_string(),
    })
}

/// Eintraege des Protokolls fuer die Seite (neueste zuerst).
pub fn audit_entries(
    conn: &Connection,
    integration_id: Option<String>,
    outcome: Option<String>,
    caller: Option<String>,
    limit: u32,
) -> Result<Vec<AuditEntry>, IntegrationError> {
    audit::list(
        conn,
        &audit::AuditFilter {
            integration_id,
            caller,
            outcome,
            since_ms: None,
        },
        limit,
    )
}

#[cfg(test)]
mod tests;
