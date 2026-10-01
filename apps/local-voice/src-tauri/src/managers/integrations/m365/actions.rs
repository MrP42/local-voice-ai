//! Die Aktionen des Kontos hinter dem Tor (A5): Mail senden, Datei ablegen, Notiz am
//! Termin. Jede laeuft durch `gate::run` bzw. `gate::run_approved` (B1: kein
//! Adapter ohne Tor).
//!
//! - Aufrufer `User` (die Oberflaeche): braucht keine Freigabe; die Aktion steht
//!   trotzdem im Audit (Best Effort, wie jede Nutzeraktion in `view.rs`).
//! - Aufrufer `Workflow`/`AgentLocal`/`AgentExternal`: Vorgabe „fragen“ fuer alles
//!   Schreibende. `gate::run` legt die Freigabe an und liefert `Pending`; nach der
//!   Entscheidung des Nutzers fuehrt derselbe Aufruf mit `approval_id` die Aktion
//!   aus, einmalig und nur mit denselben Argumenten (`gate::run_approved`).
//! - Vorher und nachher stehen `pending`/`ok`/`error` im Audit; ohne Audit-Zeile
//!   laeuft nichts (fail closed, siehe `gate.rs`).
//!
//! Das Tor ist synchron (SQLite), die Aktionen sind asynchron: der Aufruf geht in
//! einen Blockier-Thread (`spawn_blocking`), der die Aktion mit `Handle::block_on`
//! auf der Laufzeit des Aufrufers ausfuehrt. Es wird nie ein SQLite-Handle ueber ein
//! `await` gehalten.

use std::cell::RefCell;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::runtime::Handle;

use super::drive::{UploadRequest, Uploaded};
use super::error::M365Error;
use super::event::{EventRef, NoteOutcome};
use super::mail::MailMessage;
use super::service::{Acct, M365Service};
use crate::managers::integrations::gate::{self, GateOutcome, Request};
use crate::managers::integrations::model::{
    AuditOutcome, Caller, Capability, IntegrationError, NewAudit,
};
use crate::managers::integrations::{audit, store};
use crate::managers::meetings::store::MeetingStore;

/// Zeitstempel in Millisekunden UTC.
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn store_err(e: impl std::fmt::Display) -> IntegrationError {
    IntegrationError::Store(e.to_string())
}

/// Der Nutzer hat etwas getan, das nach aussen wirkt: ins Audit (Best Effort).
fn audit_user(
    conn: &rusqlite::Connection,
    integration_id: &str,
    cap: Capability,
    target: &str,
    outcome: AuditOutcome,
    detail: Value,
    now: i64,
) {
    let entry = NewAudit {
        caller: Caller::User.as_str().to_string(),
        integration_id: Some(integration_id.to_string()),
        capability: Some(cap.as_str().to_string()),
        target: Some(target.to_string()),
        outcome,
        detail: Some(detail),
    };
    if let Err(e) = audit::record_at(conn, &entry, now) {
        log::warn!("m365: Audit der Nutzeraktion nicht geschrieben: {e}");
    }
}

/// Ein Schritt des Nutzers an einem Konto (Anlegen, Einstellungen, Anmelden,
/// Abmelden) ins Audit (Best Effort, wie in `view.rs`).
pub fn audit_user_event(conn: &rusqlite::Connection, integration_id: &str, detail: Value) {
    let entry = NewAudit {
        caller: Caller::User.as_str().to_string(),
        integration_id: Some(integration_id.to_string()),
        capability: None,
        target: None,
        outcome: AuditOutcome::Ok,
        detail: Some(detail),
    };
    if let Err(e) = audit::record_at(conn, &entry, now_ms()) {
        log::warn!("m365: Audit des Schritts nicht geschrieben: {e}");
    }
}

/// Gemeinsamer Weg aller Aktionen (siehe Moduldoku).
#[allow(clippy::too_many_arguments)]
async fn gated<T: Send + 'static>(
    svc: Arc<M365Service>,
    store: Arc<MeetingStore>,
    caller: Caller,
    integration_id: String,
    cap: Capability,
    target: String,
    args: Value,
    approval_id: Option<String>,
    op: impl FnOnce(&M365Service, Acct, &Handle) -> Result<T, M365Error> + Send + 'static,
) -> Result<GateOutcome<T>, IntegrationError> {
    let rt = Handle::current();
    tokio::task::spawn_blocking(move || {
        let conn = store.get_connection().map_err(store_err)?;
        let now = now_ms();
        let req = Request {
            caller,
            integration_id: &integration_id,
            capability: cap,
            target: Some(&target),
            args: Some(&args),
            tool_mode: None,
        };
        let failure: RefCell<Option<M365Error>> = RefCell::new(None);
        let action = || -> Result<T, String> {
            let keep = |e: M365Error| {
                let wire = e.wire();
                *failure.borrow_mut() = Some(e);
                wire
            };
            let integ = store::get(&conn, &integration_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Die Integration gibt es nicht.".to_string())?;
            let acct = Acct::from_integration(integ).map_err(keep)?;
            op(&svc, acct, &rt).map_err(keep)
        };
        let outcome = match approval_id.as_deref() {
            Some(id) => gate::run_approved(&conn, id, &req, now, action)?,
            None => gate::run(&conn, &req, now, action)?,
        };
        // Zustand am Eintrag (fuer die Oberflaeche) und Spur der Nutzeraktion.
        match &outcome {
            GateOutcome::Done(_) => {
                let _ = store::mark_ok(&conn, &integration_id, now);
                if caller == Caller::User {
                    audit_user(
                        &conn,
                        &integration_id,
                        cap,
                        &target,
                        AuditOutcome::Ok,
                        json!({ "phase": "done" }),
                        now,
                    );
                }
            }
            GateOutcome::Failed(wire) => {
                let text = failure
                    .borrow()
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| wire.clone());
                let _ = store::mark_error(&conn, &integration_id, &text, now);
                if caller == Caller::User {
                    audit_user(
                        &conn,
                        &integration_id,
                        cap,
                        &target,
                        AuditOutcome::Error,
                        json!({ "phase": "done", "error": wire }),
                        now,
                    );
                }
            }
            _ => {}
        }
        Ok(outcome)
    })
    .await
    .map_err(store_err)?
}

/// Eine Mail senden (Faehigkeit `mail.send`).
pub async fn send_mail(
    svc: Arc<M365Service>,
    store: Arc<MeetingStore>,
    caller: Caller,
    integration_id: &str,
    msg: MailMessage,
    approval_id: Option<String>,
) -> Result<GateOutcome<()>, IntegrationError> {
    let target = msg.gate_target();
    let args = msg.gate_args();
    gated(
        svc,
        store,
        caller,
        integration_id.to_string(),
        Capability::MailSend,
        target,
        args,
        approval_id,
        move |svc, acct, rt| rt.block_on(svc.send_mail(&acct, &msg)),
    )
    .await
}

/// Eine Datei in OneDrive ablegen (Faehigkeit `files.write`). Das Ziel fuer
/// Freigabe und Audit wird aus der Konfiguration des Kontos gebildet; ist der
/// Name oder Pfad unzulaessig, gibt es Fehler, bevor das Tor etwas aufzeichnet.
pub async fn upload_file(
    svc: Arc<M365Service>,
    store: Arc<MeetingStore>,
    caller: Caller,
    integration_id: &str,
    req: UploadRequest,
    approval_id: Option<String>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<GateOutcome<Uploaded>, IntegrationError> {
    let cfg = {
        let conn = store.get_connection().map_err(store_err)?;
        let integ = store::get(&conn, integration_id)?
            .ok_or_else(|| IntegrationError::NotFound(integration_id.to_string()))?;
        Acct::from_integration(integ)
            .map_err(|e| IntegrationError::Invalid(e.to_string()))?
            .cfg
    };
    let target = req
        .display_path(&cfg)
        .map_err(|e| IntegrationError::Invalid(e.to_string()))?;
    let args = req
        .gate_args(&cfg)
        .map_err(|e| IntegrationError::Invalid(e.to_string()))?;
    gated(
        svc,
        store,
        caller,
        integration_id.to_string(),
        Capability::FilesWrite,
        target,
        args,
        approval_id,
        move |svc, acct, rt| rt.block_on(svc.upload(&acct, &req, cancel.as_deref())),
    )
    .await
}

/// Eine Notiz an einen Termin haengen (Faehigkeit `calendar.write`).
pub async fn event_note(
    svc: Arc<M365Service>,
    store: Arc<MeetingStore>,
    caller: Caller,
    integration_id: &str,
    ev: EventRef,
    note: String,
    approval_id: Option<String>,
) -> Result<GateOutcome<NoteOutcome>, IntegrationError> {
    // Das Ziel ist der Termin (Zeit und Kennung), nie der Notiztext.
    let target = format!("Termin {}", ev.uid.chars().take(40).collect::<String>());
    let args = json!({ "event": ev.uid, "starts_at": ev.starts_at, "note": note });
    gated(
        svc,
        store,
        caller,
        integration_id.to_string(),
        Capability::CalendarWrite,
        target,
        args,
        approval_id,
        move |svc, acct, rt| rt.block_on(svc.add_event_note(&acct, &ev, &note)),
    )
    .await
}
