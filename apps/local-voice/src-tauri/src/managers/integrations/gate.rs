//! Das Tor: jede Aktion eines Nicht-Nutzers geht hier durch (A1).
//!
//! `check` entscheidet nach `grants::explain`, schreibt Verweigerungen und
//! Nachfragen ins Audit und legt bei „fragen“ eine Freigabe-Anfrage an. `run`
//! fuehrt eine erlaubte Aktion aus und haelt sie im Audit fest.
//!
//! Sicherheitsannahmen (fuer das Review):
//! - **Fail closed**: Laesst sich das Audit vor der Aktion nicht schreiben (Platte
//!   voll, Datenbank gesperrt), laeuft die Aktion NICHT; der Aufrufer bekommt einen
//!   Fehler. Fuer Nicht-Nutzer gibt es keine unprotokollierte Aktion.
//! - Erlaubte Aktionen schreiben VOR dem Start einen Eintrag `pending`
//!   (`phase: running`) und setzen ihn danach auf `ok` oder `error`. Bleibt ein
//!   Eintrag `pending` stehen (Absturz mitten in der Aktion), ist das sichtbar:
//!   die Aktion kann gelaufen sein.
//! - Verwirft der Nutzer eine Freigabe oder aendert er das Recht auf `off`, gilt
//!   das sofort: `run_approved` prueft das Recht erneut.
//! - Der Nutzer in der Oberflaeche (`Caller::User`) braucht keine Freigabe und
//!   erscheint nicht im Audit dieses Tores; Integration aus, fehlende Faehigkeit
//!   und gesperrte Richtung gelten aber auch fuer ihn.
//! - Kein Geheimnis im Audit: Ziel, Vorschau und Detail laufen durch die
//!   Schwaerzung (`audit::redact_*`).

use rusqlite::Connection;
use serde_json::{json, Value};

use super::approvals::{self, ApprovalError, Expect, NewApproval};
use super::audit::{self, sanitize_text};
use super::grants::{explain, OffReason};
use super::model::{
    AuditOutcome, Caller, Capability, GrantMode, Integration, IntegrationError, NewAudit,
};
use super::store;

/// Eine Anfrage an das Tor.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    pub caller: Caller,
    pub integration_id: &'a str,
    pub capability: Capability,
    /// Wohin/woran (Datei, Empfaenger, Termin): erscheint geschwaerzt im Audit.
    pub target: Option<&'a str>,
    /// Argumente, nur fuer die Bindung der Freigabe (Hash) und die Vorschau.
    pub args: Option<&'a Value>,
    /// Recht je Werkzeug bei externen Agenten (A7); sonst `None`.
    pub tool_mode: Option<GrantMode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    /// Klartext fuer Nutzer und Agent, dazu der maschinenlesbare Grund.
    Denied {
        reason: String,
        code: &'static str,
    },
    /// Der Nutzer muss in der App freigeben; `approval_id` fuer Nachfrage/Status.
    NeedsApproval {
        approval_id: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum GateOutcome<T> {
    Done(T),
    /// Die Aktion lief und scheiterte (Text geschwaerzt im Audit).
    Failed(String),
    Denied {
        reason: String,
        code: &'static str,
    },
    Pending {
        approval_id: String,
    },
}

fn caller_label(c: Caller) -> &'static str {
    c.as_str()
}

fn args_text(args: Option<&Value>) -> String {
    args.map(|a| a.to_string()).unwrap_or_default()
}

fn preview(req: &Request<'_>) -> String {
    let target = req.target.unwrap_or("");
    let args = args_text(req.args);
    if args.is_empty() || args == "null" {
        target.to_string()
    } else if target.is_empty() {
        args
    } else {
        format!("{target} — {args}")
    }
}

fn hash_of(req: &Request<'_>) -> String {
    approvals::args_hash(
        req.integration_id,
        req.capability.as_str(),
        req.target.unwrap_or(""),
        &args_text(req.args),
    )
}

fn audit_entry(req: &Request<'_>, outcome: AuditOutcome, detail: Value) -> NewAudit {
    NewAudit {
        caller: caller_label(req.caller).to_string(),
        integration_id: Some(req.integration_id.to_string()),
        capability: Some(req.capability.as_str().to_string()),
        target: req.target.map(str::to_string),
        outcome,
        detail: Some(detail),
    }
}

fn deny(
    conn: &Connection,
    req: &Request<'_>,
    code: &'static str,
    reason: &str,
    now_ms: i64,
) -> Result<Decision, IntegrationError> {
    audit::record_at(
        conn,
        &audit_entry(req, AuditOutcome::Denied, json!({ "reason": code })),
        now_ms,
    )?;
    Ok(Decision::Denied {
        reason: reason.to_string(),
        code,
    })
}

/// Entscheidung ueber eine Anfrage (siehe Moduldoku). `Err` heisst: die
/// Entscheidung liess sich nicht festhalten -- der Aufrufer darf nicht handeln.
pub fn check(
    conn: &Connection,
    req: &Request<'_>,
    now_ms: i64,
) -> Result<Decision, IntegrationError> {
    let Some(integration) = store::get(conn, req.integration_id)? else {
        return deny(
            conn,
            req,
            "unknown_integration",
            "Die Integration gibt es nicht.",
            now_ms,
        );
    };
    check_loaded(conn, &integration, req, now_ms)
}

fn check_loaded(
    conn: &Connection,
    integration: &Integration,
    req: &Request<'_>,
    now_ms: i64,
) -> Result<Decision, IntegrationError> {
    let grants = store::grants_for(conn, &integration.id)?;
    let (mode, reason) = explain(
        integration,
        req.capability,
        req.caller,
        &grants,
        req.tool_mode,
    );
    match mode {
        GrantMode::Allow => Ok(Decision::Allowed),
        GrantMode::Off => {
            let r = reason.unwrap_or(OffReason::GrantOff);
            if req.caller == Caller::User {
                // Der Nutzer erscheint nicht im Audit; er bekommt den Grund.
                return Ok(Decision::Denied {
                    reason: r.message().to_string(),
                    code: r.as_str(),
                });
            }
            deny(conn, req, r.as_str(), r.message(), now_ms)
        }
        GrantMode::Ask => {
            let hash = hash_of(req);
            let created = approvals::create(
                conn,
                &NewApproval {
                    caller: caller_label(req.caller),
                    integration_id: Some(req.integration_id),
                    capability: req.capability.as_str(),
                    args_preview: Some(&preview(req)),
                    args_hash: Some(&hash),
                },
                now_ms,
            );
            let approval = match created {
                Ok(a) => a,
                Err(IntegrationError::Invalid(msg)) => {
                    return deny(conn, req, "too_many_pending", &msg, now_ms);
                }
                Err(e) => return Err(e),
            };
            audit::record_at(
                conn,
                &audit_entry(
                    req,
                    AuditOutcome::Pending,
                    json!({ "phase": "approval", "approval_id": approval.id }),
                ),
                now_ms,
            )?;
            Ok(Decision::NeedsApproval {
                approval_id: approval.id,
            })
        }
    }
}

/// Prueft und fuehrt `action` aus, wenn erlaubt (siehe Moduldoku).
pub fn run<T>(
    conn: &Connection,
    req: &Request<'_>,
    now_ms: i64,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<GateOutcome<T>, IntegrationError> {
    match check(conn, req, now_ms)? {
        Decision::Allowed => execute(conn, req, now_ms, action),
        Decision::Denied { reason, code } => Ok(GateOutcome::Denied { reason, code }),
        Decision::NeedsApproval { approval_id } => Ok(GateOutcome::Pending { approval_id }),
    }
}

/// Fuehrt `action` aus, nachdem der Nutzer die Anfrage `approval_id`
/// genehmigt hat. Die Genehmigung wird eingeloest (einmalig) und muss zu
/// Aufrufer, Integration, Faehigkeit und Argumenten passen. Steht das Recht
/// inzwischen auf `off`, geschieht nichts.
pub fn run_approved<T>(
    conn: &Connection,
    approval_id: &str,
    req: &Request<'_>,
    now_ms: i64,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<GateOutcome<T>, IntegrationError> {
    let Some(integration) = store::get(conn, req.integration_id)? else {
        return deny_outcome(
            conn,
            req,
            "unknown_integration",
            "Die Integration gibt es nicht.",
            now_ms,
        );
    };
    let grants = store::grants_for(conn, &integration.id)?;
    let (mode, reason) = explain(
        &integration,
        req.capability,
        req.caller,
        &grants,
        req.tool_mode,
    );
    if mode == GrantMode::Off {
        let r = reason.unwrap_or(OffReason::GrantOff);
        return deny_outcome(conn, req, r.as_str(), r.message(), now_ms);
    }
    let hash = hash_of(req);
    let consumed = approvals::consume(
        conn,
        approval_id,
        &Expect {
            caller: caller_label(req.caller),
            integration_id: Some(req.integration_id),
            capability: req.capability.as_str(),
            args_hash: Some(&hash),
        },
        now_ms,
    );
    match consumed {
        Ok(()) => execute(conn, req, now_ms, action),
        Err(ApprovalError::Store(m)) => Err(IntegrationError::Store(m)),
        Err(e) => {
            let code = match e {
                ApprovalError::NotFound => "approval_not_found",
                ApprovalError::Expired => "approval_expired",
                ApprovalError::Mismatch => "approval_mismatch",
                _ => "approval_not_granted",
            };
            deny_outcome(conn, req, code, &e.to_string(), now_ms)
        }
    }
}

fn deny_outcome<T>(
    conn: &Connection,
    req: &Request<'_>,
    code: &'static str,
    reason: &str,
    now_ms: i64,
) -> Result<GateOutcome<T>, IntegrationError> {
    match deny(conn, req, code, reason, now_ms)? {
        Decision::Denied { reason, code } => Ok(GateOutcome::Denied { reason, code }),
        _ => unreachable!("deny liefert immer Denied"),
    }
}

fn execute<T>(
    conn: &Connection,
    req: &Request<'_>,
    now_ms: i64,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<GateOutcome<T>, IntegrationError> {
    if req.caller == Caller::User {
        return Ok(match action() {
            Ok(v) => GateOutcome::Done(v),
            Err(e) => GateOutcome::Failed(sanitize_text(&e, 300)),
        });
    }
    // Fail closed: ohne Audit-Eintrag keine Aktion.
    let id = audit::record_at(
        conn,
        &audit_entry(req, AuditOutcome::Pending, json!({ "phase": "running" })),
        now_ms,
    )?;
    let result = action();
    let (outcome, detail, out) = match result {
        Ok(v) => (
            AuditOutcome::Ok,
            json!({ "phase": "done" }),
            GateOutcome::Done(v),
        ),
        Err(e) => {
            let clean = sanitize_text(&e, 300);
            (
                AuditOutcome::Error,
                json!({ "phase": "done", "error": clean }),
                GateOutcome::Failed(clean),
            )
        }
    };
    if let Err(e) = audit::set_outcome(conn, id, outcome, Some(detail)) {
        // Die Aktion ist gelaufen; der Eintrag bleibt `pending` und faellt auf.
        log::warn!("integrations: Audit-Ergebnis nicht gesetzt (Eintrag {id}): {e}");
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
