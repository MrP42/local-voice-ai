use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::managers::integrations::approvals::{self, NewApproval};
use crate::managers::integrations::model::ApprovalState;
use crate::managers::workflows::engine::{Engine, EnqueueRequest};
use crate::managers::workflows::model::Origin;
use crate::managers::workflows::recording::{ensure_carrier, CARRIER_ID};
use crate::managers::workflows::test_support::{armed_workflow, engine, FakeClock, Fx, T0};

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
}

fn world() -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    ensure_carrier(&fx.conn(), T0).unwrap();
    World { fx, clock, engine }
}

fn rec_def(name: &str) -> serde_json::Value {
    json!({
        "schema": "lva-workflow@1",
        "name": name,
        "trigger": {"type": "calendar.event_starting", "integration": "cal-1"},
        "steps": [{
            "id": "rec", "action": "recording.start",
            "params": {"via": CARRIER_ID, "title": "{{trigger.title}}"}
        }]
    })
}

impl World {
    fn start_run(&self, name: &str, key: &str) -> String {
        let wf = armed_workflow(&self.engine, &rec_def(name));
        self.engine
            .enqueue(&EnqueueRequest {
                workflow_id: wf,
                trigger_key: format!("calendar_start:{key}"),
                origin: Origin::Trigger,
                trigger: json!({
                    "event_id": format!("cal-1:uid:{key}"),
                    "title": "Jour fixe Vertrieb"
                }),
                vars: Default::default(),
                force_dry_run: false,
            })
            .unwrap()
            .run_id
    }
}

#[test]
fn a_waiting_recording_shows_up_with_workflow_event_and_title() {
    let w = world();
    let run = w.start_run("Kundentermin protokollieren", "t1");
    w.engine.tick().unwrap();
    let conn = w.fx.conn();
    let list = pending(&conn, T0).unwrap();
    assert_eq!(list.len(), 1);
    let c = &list[0];
    assert_eq!(c.run_id, run);
    assert_eq!(c.workflow_name, "Kundentermin protokollieren");
    assert_eq!(c.event_key.as_deref(), Some("cal-1:uid:t1"));
    assert_eq!(c.title.as_deref(), Some("Jour fixe Vertrieb"));
    assert!(is_pending(&conn, &c.approval_id, T0));
}

#[test]
fn only_requests_to_record_are_consents_a_mail_approval_is_not() {
    let w = world();
    w.start_run("Aufnahme", "t1");
    w.engine.tick().unwrap();
    let conn = w.fx.conn();
    // Eine Freigabe zum Mailversand desselben Aufrufers.
    let mail = approvals::create(
        &conn,
        &NewApproval {
            caller: "workflow",
            integration_id: Some("smtp-1"),
            capability: "mail.send",
            args_preview: Some("Ziel: me"),
            args_hash: Some("h1"),
        },
        T0,
    )
    .unwrap();
    // Und eine Aufnahme-Freigabe eines Agenten.
    let agent = approvals::create(
        &conn,
        &NewApproval {
            caller: "agent_external",
            integration_id: Some(CARRIER_ID),
            capability: "recording.start",
            args_preview: None,
            args_hash: Some("h2"),
        },
        T0,
    )
    .unwrap();
    let list = pending(&conn, T0).unwrap();
    assert_eq!(list.len(), 1, "nur die des Ablaufs");
    assert!(list
        .iter()
        .all(|c| c.approval_id != mail.id && c.approval_id != agent.id));
}

#[test]
fn decide_refuses_every_approval_that_is_not_a_consent() {
    let w = world();
    let conn = w.fx.conn();
    let mail = approvals::create(
        &conn,
        &NewApproval {
            caller: "workflow",
            integration_id: Some("smtp-1"),
            capability: "mail.send",
            args_preview: Some("Ziel: me"),
            args_hash: Some("h1"),
        },
        T0,
    )
    .unwrap();
    let agent = approvals::create(
        &conn,
        &NewApproval {
            caller: "agent_local",
            integration_id: Some(CARRIER_ID),
            capability: "recording.start",
            args_preview: None,
            args_hash: Some("h2"),
        },
        T0,
    )
    .unwrap();
    for id in [&mail.id, &agent.id] {
        assert_eq!(
            decide(&conn, id, true, true, T0).unwrap_err(),
            ConsentError::NotAConsent
        );
        assert_eq!(
            approvals::get(&conn, id).unwrap().unwrap().state,
            ApprovalState::Pending,
            "unberuehrt"
        );
    }
    assert_eq!(
        decide(&conn, "gibt-es-nicht", true, true, T0).unwrap_err(),
        ConsentError::NotFound
    );
}

#[test]
fn deciding_twice_or_after_expiry_or_after_cancelling_reports_not_pending() {
    let w = world();
    let conn = w.fx.conn();
    let run1 = w.start_run("Eins", "a");
    let run2 = w.start_run("Zwei", "b");
    let run3 = w.start_run("Drei", "c");
    w.engine.tick().unwrap();
    let list = pending(&conn, T0).unwrap();
    assert_eq!(list.len(), 3);
    let of = |run: &str| {
        list.iter()
            .find(|c| c.run_id == run)
            .unwrap()
            .approval_id
            .clone()
    };
    // Zweimal entscheiden.
    decide(&conn, &of(&run1), true, true, T0).unwrap();
    assert_eq!(
        decide(&conn, &of(&run1), true, true, T0).unwrap_err(),
        ConsentError::NotPending
    );
    // Verfallen.
    assert_eq!(
        decide(&conn, &of(&run2), true, true, T0 + approvals::TTL_MS + 1).unwrap_err(),
        ConsentError::NotPending
    );
    // Lauf abgebrochen: die Freigabe ist zurueckgezogen.
    w.engine.cancel_run(&run3).unwrap();
    assert_eq!(
        decide(&conn, &of(&run3), true, true, T0).unwrap_err(),
        ConsentError::NotPending
    );
    assert!(!is_pending(&conn, &of(&run3), T0));
}

#[test]
fn a_no_ends_the_request_and_is_not_confused_with_a_yes() {
    let w = world();
    let conn = w.fx.conn();
    w.start_run("Aufnahme", "t1");
    w.engine.tick().unwrap();
    let id = pending(&conn, T0).unwrap()[0].approval_id.clone();
    decide(&conn, &id, false, true, T0).unwrap();
    assert_eq!(
        approvals::get(&conn, &id).unwrap().unwrap().state,
        ApprovalState::Denied
    );
    assert!(pending(&conn, T0).unwrap().is_empty());
}

#[test]
fn error_codes_follow_the_command_convention() {
    assert_eq!(ConsentError::NotPending.code(), "consent_not_pending");
    assert_eq!(ConsentError::NotFound.code(), "consent_not_pending");
    assert_eq!(ConsentError::NotAConsent.code(), "consent_invalid");
    assert_eq!(ConsentError::Store("x".into()).code(), "store_failed");
}

#[test]
fn allowing_a_recording_without_the_consent_confirmation_is_refused_and_changes_nothing() {
    let w = world();
    let conn = w.fx.conn();
    w.start_run("Aufnahme", "t1");
    w.engine.tick().unwrap();
    let id = pending(&conn, T0).unwrap()[0].approval_id.clone();
    assert_eq!(
        decide(&conn, &id, true, false, T0).unwrap_err(),
        ConsentError::ConsentRequired
    );
    assert_eq!(ConsentError::ConsentRequired.code(), "consent_required");
    assert_eq!(
        approvals::get(&conn, &id).unwrap().unwrap().state,
        ApprovalState::Pending,
        "die Bitte bleibt offen, es wurde nichts entschieden"
    );
    assert!(is_pending(&conn, &id, T0));
    // Mit der Bestaetigung geht es; das Nein braucht sie nie.
    decide(&conn, &id, true, true, T0).unwrap();
    assert_eq!(
        approvals::get(&conn, &id).unwrap().unwrap().state,
        ApprovalState::Approved
    );
    let w2 = world();
    let c2 = w2.fx.conn();
    w2.start_run("Aufnahme", "t2");
    w2.engine.tick().unwrap();
    let id2 = pending(&c2, T0).unwrap()[0].approval_id.clone();
    decide(&c2, &id2, false, false, T0).unwrap();
    assert_eq!(
        approvals::get(&c2, &id2).unwrap().unwrap().state,
        ApprovalState::Denied
    );
}
