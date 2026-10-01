use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::managers::integrations::approvals;
use crate::managers::integrations::audit::{self, AuditFilter};
use crate::managers::integrations::model::{ApprovalState, Capability, GrantMode, Kind};
use crate::managers::workflows::action::HeavyNeed;
use crate::managers::workflows::heavy::{LocalHeavyGate, MEMORY_RETRY_MS, SLOT_RETRY_MS};
use crate::managers::workflows::test_support::{
    armed_workflow, def, engine, engine_with, note, register, roomy_gate, set_grant, step,
    FakeClock, Fx, Scripted, T0,
};

// ---------------------------------------------------------------------------
// Aufbau
// ---------------------------------------------------------------------------

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
    notify: Arc<Scripted>,
}

fn world() -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    let notify = Scripted::new("notify.local", EffectKind::Idempotent);
    engine.register_action(notify.clone());
    World {
        fx,
        clock,
        engine,
        notify,
    }
}

impl World {
    fn conn(&self) -> Connection {
        self.fx.conn()
    }

    /// Ein zweiter Prozess: neue Engine auf derselben Datenbank (eigene Kennung).
    fn restart(&self) -> Engine {
        let e = engine_with(&self.fx, &self.clock, roomy_gate());
        e.register_action(self.notify.clone());
        e
    }

    fn run(&self, id: &str) -> RunRow {
        store::get_run(&self.conn(), id).unwrap().unwrap()
    }

    fn steps(&self, id: &str) -> Vec<StepRow> {
        store::steps_for(&self.conn(), id).unwrap()
    }

    /// Scharfer Ablauf und ein neuer Lauf dazu.
    fn live(&self, definition: &Value) -> (String, String) {
        let wf = armed_workflow(&self.engine, definition);
        let run = self.engine.enqueue(&EnqueueRequest::manual(&wf)).unwrap();
        assert!(!run.dry_run);
        (wf, run.run_id)
    }

    fn audit(&self) -> Vec<crate::managers::integrations::model::AuditEntry> {
        audit::list(&self.conn(), &AuditFilter::default(), 100).unwrap()
    }
}

fn mail_step(id: &str) -> Value {
    step(
        id,
        "mail.send",
        json!({"via": "smtp-1", "to": "me", "subject": "Protokoll"}),
    )
}

fn mail_action() -> Arc<Scripted> {
    Scripted::new("mail.send", EffectKind::External)
}

/// Registriert `smtp-1` mit dem Recht `mode` fuer `mail.send`.
fn smtp(w: &World, mode: Option<GrantMode>) {
    let conn = w.conn();
    register(&conn, Kind::Smtp, "smtp-1");
    if let Some(m) = mode {
        set_grant(&conn, "smtp-1", Capability::MailSend, m);
    }
}

fn outcome_of(report: &TickReport, run: &str) -> RunOutcome {
    report
        .outcomes
        .iter()
        .find(|(id, _)| id == run)
        .unwrap_or_else(|| panic!("kein Ergebnis fuer {run}: {report:?}"))
        .1
        .clone()
}

// ---------------------------------------------------------------------------
// Grundablauf, Trockenlauf, Idempotenz
// ---------------------------------------------------------------------------

#[test]
fn a_run_executes_its_steps_in_order_and_hands_results_forward() {
    let w = world();
    w.notify
        .push_ok(json!({"path": "C:/Ablage/Protokoll.docx"}));
    let d = def(vec![
        step("a", "notify.local", json!({"title": "A"})),
        step(
            "b",
            "notify.local",
            json!({"title": "Ablage: {{steps.a.path}}"}),
        ),
    ]);
    let (_, run) = w.live(&d);
    let report = w.engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    let calls = w.notify.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].params["title"], "Ablage: C:/Ablage/Protokoll.docx");
    let rows = w.steps(&run);
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|r| r.state == StepState::Done && r.attempt == 1));
    assert!(rows[0]
        .output_json
        .as_deref()
        .unwrap()
        .contains("Protokoll.docx"));
    let r = w.run(&run);
    assert_eq!(r.state, RunState::Done);
    assert!(r.lease_owner.is_none() && r.ended_at.is_some());
    assert_eq!(r.error, None);
}

#[test]
fn a_workflow_that_is_not_armed_only_plans_and_never_runs_an_action() {
    let w = world();
    let row = w
        .engine
        .save_workflow(None, &def(vec![note("a"), note("b")]))
        .unwrap();
    w.engine.set_enabled(&row.id, true).unwrap();
    // Nicht scharf: ein Lauf plant nur, auch wenn er nicht ausdruecklich "trocken" ist.
    let queued = w.engine.enqueue(&EnqueueRequest::manual(&row.id)).unwrap();
    assert!(
        queued.dry_run,
        "ein nicht scharf geschalteter Ablauf plant nur"
    );
    let report = w.engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &queued.run_id), RunOutcome::Done);
    assert_eq!(w.notify.call_count(), 0, "kein Baustein ist gelaufen");
    let rows = w.steps(&queued.run_id);
    assert!(rows.iter().all(|r| r.state == StepState::Planned));
    let plan: Value = serde_json::from_str(rows[0].output_json.as_deref().unwrap()).unwrap();
    assert_eq!(plan["status"], "planned");
    assert!(plan["effect"].as_str().unwrap().contains("Mitteilung"));
    assert!(w.run(&queued.run_id).dry_run);
    // Auch Audit und Freigaben bleiben leer.
    assert!(w.audit().is_empty());

    // Ein scharfer Ablauf laesst sich trotzdem als Trockenlauf anfordern.
    w.engine.set_armed(&row.id, true).unwrap();
    let mut req = EnqueueRequest::manual(&row.id);
    req.force_dry_run = true;
    let dry = w.engine.enqueue(&req).unwrap();
    assert!(dry.dry_run);
    w.engine.tick().unwrap();
    assert_eq!(w.notify.call_count(), 0);
}

#[test]
fn a_new_workflow_is_refused_for_triggers_until_it_is_switched_on_and_never_runs_live_unarmed() {
    let w = world();
    let row = w.engine.save_workflow(None, &def(vec![note("a")])).unwrap();
    // Ausgeschaltet: ein Ausloeser darf nichts einreihen, ein Trockenlauf von Hand schon.
    let mut by_trigger = EnqueueRequest::manual(&row.id);
    by_trigger.origin = Origin::Trigger;
    by_trigger.trigger_key = "calendar:1".into();
    assert!(matches!(
        w.engine.enqueue(&by_trigger),
        Err(WorkflowError::Disabled(_))
    ));
    assert!(
        w.engine
            .enqueue(&EnqueueRequest::manual(&row.id))
            .unwrap()
            .dry_run
    );
    // Eingeschaltet, aber nicht scharf: der Ausloeser startet nur einen Plan.
    w.engine.set_enabled(&row.id, true).unwrap();
    let planned = w.engine.enqueue(&by_trigger).unwrap();
    assert!(planned.dry_run);
    // Ausgeschaltet nach dem Scharfschalten: wieder abgelehnt, auch fuer Handstart.
    w.engine.set_armed(&row.id, true).unwrap();
    w.engine.set_enabled(&row.id, false).unwrap();
    assert!(matches!(
        w.engine.enqueue(&EnqueueRequest::manual(&row.id)),
        Err(WorkflowError::Disabled(_))
    ));
    // Unbekannter Ablauf.
    assert!(matches!(
        w.engine.enqueue(&EnqueueRequest::manual("gibt-es-nicht")),
        Err(WorkflowError::NotFound(_))
    ));
    // Ungueltiger Schluessel.
    let mut bad = EnqueueRequest::manual(&row.id);
    bad.trigger_key = "  ".into();
    assert!(matches!(
        w.engine.enqueue(&bad),
        Err(WorkflowError::BadInput(_))
    ));
}

#[test]
fn the_same_trigger_twice_gives_one_run_and_one_execution() {
    let w = world();
    let wf = armed_workflow(&w.engine, &def(vec![note("a")]));
    let mut req = EnqueueRequest::manual(&wf);
    req.trigger_key = "calendar:ev-1:2026-10-02T10:00".into();
    let first = w.engine.enqueue(&req).unwrap();
    let second = w.engine.enqueue(&req).unwrap();
    assert!(first.created && !second.created);
    assert_eq!(first.run_id, second.run_id);
    w.engine.tick().unwrap();
    assert_eq!(w.notify.call_count(), 1);
    // Auch nach Abschluss und nach einem Neustart loest derselbe Ausloeser nichts neu aus.
    let third = w.engine.enqueue(&req).unwrap();
    assert!(!third.created);
    let restarted = w.restart();
    let report = restarted.tick().unwrap();
    assert!(report.outcomes.is_empty());
    assert_eq!(w.notify.call_count(), 1, "genau eine Ausfuehrung");
}

#[test]
fn the_same_trigger_from_many_threads_gives_one_run() {
    let w = world();
    let wf = armed_workflow(&w.engine, &def(vec![note("a")]));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let engine = w.engine.clone();
            let wf = wf.clone();
            std::thread::spawn(move || {
                let mut req = EnqueueRequest::manual(&wf);
                req.trigger_key = "datei:abc123".into();
                engine.enqueue(&req).unwrap()
            })
        })
        .collect();
    let results: Vec<Enqueued> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.created).count(), 1);
    w.engine.tick().unwrap();
    assert_eq!(w.notify.call_count(), 1);
}

// ---------------------------------------------------------------------------
// Wiederholung
// ---------------------------------------------------------------------------

#[test]
fn a_transient_failure_is_retried_with_growing_delay_and_the_same_key() {
    let w = world();
    w.notify.push(Err(StepError::Transient("Netz weg".into())));
    w.notify.push(Err(StepError::Transient("Netz weg".into())));
    let mut d = def(vec![note("a")]);
    d["steps"][0]["retry"] = json!({"max_attempts": 3, "backoff_ms": 1000});
    let (_, run) = w.live(&d);

    let r1 = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&r1, &run),
        RunOutcome::Parked {
            reason: "retry".into()
        }
    );
    let parked = w.run(&run);
    assert_eq!(parked.state, RunState::Queued);
    assert_eq!(
        parked.next_run_at,
        Some(T0 + 1_000),
        "erste Wartezeit = backoff"
    );
    assert!(
        w.engine.tick().unwrap().outcomes.is_empty(),
        "noch nicht faellig"
    );

    w.clock.advance(1_000);
    let r2 = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&r2, &run),
        RunOutcome::Parked {
            reason: "retry".into()
        }
    );
    assert_eq!(
        w.run(&run).next_run_at,
        Some(T0 + 1_000 + 2_000),
        "verdoppelt"
    );

    w.clock.advance(2_000);
    let r3 = w.engine.tick().unwrap();
    assert_eq!(outcome_of(&r3, &run), RunOutcome::Done);
    let calls = w.notify.calls();
    assert_eq!(calls.len(), 3);
    assert!(
        calls.iter().all(|c| c.key == format!("{run}:a")),
        "derselbe Schluessel"
    );
    assert_eq!(
        calls.iter().map(|c| c.attempt).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    let states: Vec<StepState> = w.steps(&run).iter().map(|r| r.state).collect();
    assert_eq!(
        states,
        vec![StepState::Retrying, StepState::Retrying, StepState::Done]
    );
}

#[test]
fn retries_stop_when_exhausted_and_a_permanent_error_is_never_retried() {
    let w = world();
    for _ in 0..5 {
        w.notify
            .push(Err(StepError::Transient("noch immer weg".into())));
    }
    let mut d = def(vec![note("a")]);
    d["steps"][0]["retry"] = json!({"max_attempts": 2, "backoff_ms": 100});
    let (_, run) = w.live(&d);
    w.engine.tick().unwrap();
    w.clock.advance(100);
    let report = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "retries_exhausted".into()
        }
    );
    assert_eq!(w.notify.call_count(), 2, "genau max_attempts Versuche");
    let r = w.run(&run);
    assert_eq!(r.state, RunState::Failed);
    assert!(r
        .error
        .as_deref()
        .unwrap()
        .contains("Aufgegeben nach 2 Versuchen"));
    assert_eq!(w.steps(&run).last().unwrap().state, StepState::Failed);

    // Dauerhafter Fehler: ein Versuch, keine Wiederholung.
    let w2 = world();
    w2.notify
        .push(Err(StepError::Permanent("Datei gibt es nicht".into())));
    let (_, run2) = w2.live(&def(vec![note("a"), note("b")]));
    let report = w2.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run2),
        RunOutcome::Failed {
            code: "permanent".into()
        }
    );
    assert_eq!(w2.notify.call_count(), 1);
    assert_eq!(
        w2.steps(&run2).len(),
        1,
        "der zweite Schritt wurde nie begonnen"
    );
    w2.clock.advance(3_600_000);
    assert!(w2.engine.tick().unwrap().outcomes.is_empty());
}

#[test]
fn an_unknown_failure_stops_an_external_step_but_an_idempotent_one_is_repeated() {
    let w = world();
    smtp(&w, Some(GrantMode::Allow));
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    mail.push(Err(StepError::Unknown(
        "Zeitüberschreitung nach dem Senden".into(),
    )));
    let (_, run) = w.live(&def(vec![mail_step("m")]));
    let report = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "effect_uncertain".into()
        }
    );
    assert_eq!(w.steps(&run)[0].state, StepState::Uncertain);
    w.clock.advance(3_600_000);
    w.engine.tick().unwrap();
    assert_eq!(
        mail.call_count(),
        1,
        "die Mail wird nie von selbst ein zweites Mal gesendet"
    );

    // Dasselbe bei einem wiederholbaren Schritt: wird wiederholt.
    let w2 = world();
    w2.notify.push(Err(StepError::Unknown("unklar".into())));
    let (_, run2) = w2.live(&def(vec![note("a")]));
    let r = w2.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&r, &run2),
        RunOutcome::Parked {
            reason: "retry".into()
        }
    );
    w2.clock.advance(2_000);
    w2.engine.tick().unwrap();
    assert_eq!(w2.run(&run2).state, RunState::Done);
    assert_eq!(w2.notify.call_count(), 2);
}

#[test]
fn on_error_continue_lets_the_run_go_on_and_later_steps_can_react() {
    let w = world();
    w.notify.push(Err(StepError::Permanent("kaputt".into())));
    let mut d = def(vec![
        note("a"),
        json!({"id": "b", "action": "notify.local", "when": "steps.a.ok", "params": {"title": "b"}}),
        json!({"id": "c", "action": "notify.local", "when": "not steps.a.ok",
               "params": {"title": "Fehler: {{steps.a.error}}"}}),
    ]);
    d["steps"][0]["on_error"] = json!("continue");
    let (_, run) = w.live(&d);
    let report = w.engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    let states: Vec<(String, StepState)> = w
        .steps(&run)
        .iter()
        .map(|r| (r.step_id.clone(), r.state))
        .collect();
    assert_eq!(
        states,
        vec![
            ("a".to_string(), StepState::Failed),
            ("b".to_string(), StepState::Skipped),
            ("c".to_string(), StepState::Done)
        ]
    );
    assert_eq!(w.notify.call_count(), 2, "a (gescheitert) und c");
    assert_eq!(w.notify.calls()[1].params["title"], "Fehler: kaputt");
}

#[test]
fn a_panicking_action_does_not_take_the_engine_down() {
    let w = world();
    smtp(&w, Some(GrantMode::Allow));
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    mail.panic_once();
    let (_, run1) = w.live(&def(vec![mail_step("m")]));
    let wf2 = armed_workflow(&w.engine, &def(vec![note("a")]));
    let run2 = w
        .engine
        .enqueue(&EnqueueRequest::manual(&wf2))
        .unwrap()
        .run_id;
    let report = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run1),
        RunOutcome::Failed {
            code: "effect_uncertain".into()
        },
        "ein Absturz mit Aussenwirkung ist unklar, nicht wiederholbar"
    );
    assert!(w
        .run(&run1)
        .error
        .as_deref()
        .unwrap()
        .contains("abgestürzt"));
    assert_eq!(
        outcome_of(&report, &run2),
        RunOutcome::Done,
        "der naechste Lauf laeuft"
    );
}

#[test]
fn an_output_that_is_too_large_fails_the_step_without_truncating() {
    let w = world();
    w.notify.push_ok(json!({"text": "x".repeat(70 * 1024)}));
    let (_, run) = w.live(&def(vec![note("a")]));
    let report = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "permanent".into()
        }
    );
    let r = w.run(&run);
    assert!(
        r.error.as_deref().unwrap().contains("zu groß"),
        "{:?}",
        r.error
    );
    assert_eq!(
        w.steps(&run)[0].output_json,
        None,
        "nichts Halbes gespeichert"
    );
}

// ---------------------------------------------------------------------------
// Rechte und Freigaben
// ---------------------------------------------------------------------------

#[test]
fn a_denied_step_ends_the_run_cleanly_with_an_audit_row_and_can_be_retried_after_the_fix() {
    let w = world();
    smtp(&w, Some(GrantMode::Off));
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    let (_, run) = w.live(&def(vec![mail_step("m"), note("danach")]));
    let report = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "denied".into()
        }
    );
    assert_eq!(mail.call_count(), 0, "ohne Recht laeuft nichts");
    assert_eq!(w.notify.call_count(), 0, "danach laeuft auch nichts");
    let rows = w.steps(&run);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, StepState::Denied);
    assert!(rows[0].error.as_deref().unwrap().contains("aus"));
    let r = w.run(&run);
    assert_eq!(r.state, RunState::Failed);
    assert!(
        r.lease_owner.is_none(),
        "der Lauf endet sauber, nichts bleibt gemietet"
    );
    let audit = w.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, "denied");
    assert_eq!(audit[0].caller, "workflow");
    assert_eq!(audit[0].capability.as_deref(), Some("mail.send"));
    assert_eq!(audit[0].integration_id.as_deref(), Some("smtp-1"));

    // Der Nutzer erlaubt es und wiederholt den Lauf (kein Doppelungsrisiko: nichts lief).
    set_grant(&w.conn(), "smtp-1", Capability::MailSend, GrantMode::Allow);
    w.engine.retry_run(&run, false).unwrap();
    let report = w.engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    assert_eq!(mail.call_count(), 1);
    assert_eq!(w.notify.call_count(), 1);
}

#[test]
fn an_action_without_a_register_capability_is_denied_and_audited() {
    let w = world();
    let d = def(vec![step(
        "hook",
        "webhook.post",
        json!({"url": "http://127.0.0.1:5678/webhook/x"}),
    )]);
    let (_, run) = w.live(&d);
    let report = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "denied".into()
        }
    );
    assert_eq!(w.steps(&run)[0].state, StepState::Denied);
    let audit = w.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, "denied");
    assert!(audit[0]
        .detail_json
        .as_deref()
        .unwrap()
        .contains("capability_not_modeled"));
}

#[test]
fn ask_opens_an_approval_with_a_preview_and_the_run_waits_then_runs_once_after_the_click() {
    let w = world();
    smtp(&w, None); // Vorgabe: fragen
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    let (_, run) = w.live(&def(vec![mail_step("m"), note("danach")]));
    let report = w.engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::AwaitingApproval);
    assert_eq!(mail.call_count(), 0, "ohne Freigabe geht nichts hinaus");
    assert_eq!(w.run(&run).state, RunState::AwaitingApproval);

    let conn = w.conn();
    let pending = approvals::list_pending(&conn, T0).unwrap();
    assert_eq!(pending.len(), 1);
    let preview = pending[0].args_preview.as_deref().unwrap();
    assert!(
        preview.contains("to: me"),
        "Empfaengerregel steht in der Vorschau: {preview}"
    );
    assert!(preview.contains("Protokoll"), "{preview}");
    assert_eq!(pending[0].caller, "workflow");
    assert_eq!(pending[0].integration_id.as_deref(), Some("smtp-1"));
    let step_row = &w.steps(&run)[0];
    assert_eq!(step_row.state, StepState::AwaitingApproval);
    assert_eq!(
        step_row.approval_id.as_deref(),
        Some(pending[0].id.as_str())
    );

    // Ohne Entscheidung aendert ein weiterer Takt nichts (keine zweite Freigabe).
    let again = w.engine.tick().unwrap();
    assert!(again.outcomes.is_empty() && again.approvals_released == 0);
    assert_eq!(approvals::list_pending(&conn, T0).unwrap().len(), 1);

    // Der Nutzer klickt.
    approvals::decide(&conn, &pending[0].id, true, T0).unwrap();
    let report = w.engine.tick().unwrap();
    assert_eq!(report.approvals_released, 1);
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    assert_eq!(mail.call_count(), 1, "genau einmal");
    assert_eq!(w.notify.call_count(), 1);
    assert_eq!(
        approvals::get(&conn, &pending[0].id)
            .unwrap()
            .unwrap()
            .state,
        ApprovalState::Used,
        "die Freigabe ist eingeloest"
    );
    // Audit: Nachfrage, dann der Lauf der Aktion.
    let outcomes: Vec<String> = w.audit().iter().map(|a| a.outcome.clone()).collect();
    assert!(outcomes.contains(&"pending".to_string()));
    assert!(outcomes.contains(&"ok".to_string()));
    assert!(w.engine.tick().unwrap().outcomes.is_empty());
    assert_eq!(mail.call_count(), 1);
}

#[test]
fn two_runs_with_identical_parameters_get_two_approvals_and_one_click_releases_one_run() {
    let w = world();
    smtp(&w, None);
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    let wf = armed_workflow(&w.engine, &def(vec![mail_step("m")]));
    let mut ids = Vec::new();
    for key in ["termin-1", "termin-2"] {
        let mut req = EnqueueRequest::manual(&wf);
        req.trigger_key = key.into();
        ids.push(w.engine.enqueue(&req).unwrap().run_id);
    }
    w.engine.tick().unwrap();
    let conn = w.conn();
    let pending = approvals::list_pending(&conn, T0).unwrap();
    assert_eq!(
        pending.len(),
        2,
        "je Lauf eine eigene Freigabe, auch bei gleichen Parametern"
    );
    assert!(
        pending[0].args_preview.as_deref().unwrap().contains("lauf"),
        "die Vorschau nennt den Lauf"
    );
    // Der Nutzer gibt nur die Freigabe des ersten Laufs.
    let first = w
        .steps(&ids[0])
        .iter()
        .find_map(|r| r.approval_id.clone())
        .unwrap();
    approvals::decide(&conn, &first, true, T0).unwrap();
    let report = w.engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &ids[0]), RunOutcome::Done);
    assert_eq!(
        w.run(&ids[1]).state,
        RunState::AwaitingApproval,
        "der zweite wartet weiter"
    );
    assert_eq!(mail.call_count(), 1);
}

#[test]
fn a_refused_or_expired_approval_ends_the_run_without_the_action() {
    let w = world();
    smtp(&w, None);
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    let (_, run) = w.live(&def(vec![mail_step("m")]));
    w.engine.tick().unwrap();
    let conn = w.conn();
    let id = approvals::list_pending(&conn, T0).unwrap()[0].id.clone();
    approvals::decide(&conn, &id, false, T0).unwrap();
    let report = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "denied".into()
        }
    );
    assert_eq!(w.steps(&run)[0].state, StepState::Denied);
    assert_eq!(mail.call_count(), 0);

    // Verfallen: eine Stunde ohne Klick.
    let (_, run2) = {
        let wf = armed_workflow(&w.engine, &{
            let mut d = def(vec![mail_step("m")]);
            d["name"] = json!("Zweiter");
            d
        });
        let r = w.engine.enqueue(&EnqueueRequest::manual(&wf)).unwrap();
        (wf, r.run_id)
    };
    w.engine.tick().unwrap();
    assert_eq!(w.run(&run2).state, RunState::AwaitingApproval);
    w.clock.advance(approvals::TTL_MS + 1_000);
    let report = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run2),
        RunOutcome::Failed {
            code: "approval_expired".into()
        }
    );
    assert_eq!(
        mail.call_count(),
        0,
        "eine verfallene Freigabe fuehrt nichts aus"
    );
}

#[test]
fn cancelling_a_run_that_waits_for_approval_withdraws_the_approval() {
    let w = world();
    smtp(&w, None);
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    let (_, run) = w.live(&def(vec![mail_step("m")]));
    w.engine.tick().unwrap();
    let conn = w.conn();
    let id = approvals::list_pending(&conn, T0).unwrap()[0].id.clone();
    assert_eq!(w.engine.cancel_run(&run).unwrap(), CancelResult::Cancelled);
    assert_eq!(w.run(&run).state, RunState::Cancelled);
    assert_eq!(
        approvals::get(&conn, &id).unwrap().unwrap().state,
        ApprovalState::Expired,
        "die offene Freigabe ist zurueckgezogen"
    );
    assert!(
        approvals::decide(&conn, &id, true, T0).is_err(),
        "kein Klick mehr moeglich"
    );
    assert!(w.engine.tick().unwrap().outcomes.is_empty());
    assert_eq!(mail.call_count(), 0);
    assert!(
        matches!(w.engine.cancel_run(&run), Err(WorkflowError::BadInput(_))),
        "ein beendeter Lauf laesst sich nicht noch einmal abbrechen"
    );
}

// ---------------------------------------------------------------------------
// Absturz und Wiederaufnahme
// ---------------------------------------------------------------------------

fn crash_once(w: &World, step_id: &str, point: CrashPoint) -> String {
    w.engine.crash_at(step_id, point);
    let r = w.engine.run_next();
    let Err(WorkflowError::SimulatedCrash(_)) = r else {
        panic!("der Absturz haette ausgeloest werden muessen, war: {r:?}");
    };
    w.engine.owner().to_string()
}

#[test]
fn a_crash_before_the_step_begins_resumes_with_exactly_one_execution() {
    let w = world();
    let (_, run) = w.live(&def(vec![note("a")]));
    crash_once(&w, "a", CrashPoint::BeforeBegin);
    assert_eq!(
        w.run(&run).state,
        RunState::Running,
        "der Lauf steht gemietet da"
    );
    assert!(w.steps(&run).is_empty());
    // Der Mietvertrag gilt noch: ein zweiter Prozess darf den Lauf nicht anruehren.
    let other = w.restart();
    let early = other.tick().unwrap();
    assert_eq!(early.recovered, 0);
    assert!(early.outcomes.is_empty());
    // Nach Ablauf uebernimmt er.
    w.clock.advance(61_000);
    let report = other.tick().unwrap();
    assert_eq!(report.recovered, 1);
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    assert_eq!(w.notify.call_count(), 1);
}

#[test]
fn a_crash_after_the_action_of_an_idempotent_step_reruns_it_with_the_same_key() {
    let w = world();
    let (_, run) = w.live(&def(vec![note("a"), note("b")]));
    crash_once(&w, "a", CrashPoint::AfterAction);
    assert_eq!(w.notify.call_count(), 1);
    assert_eq!(
        w.steps(&run)[0].state,
        StepState::Running,
        "das Journal kennt `done` nicht"
    );
    w.clock.advance(61_000);
    let other = w.restart();
    let report = other.tick().unwrap();
    assert_eq!(report.recovered, 1);
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    let calls = w.notify.calls();
    assert_eq!(calls.len(), 3, "a zweimal (wiederholbar), b einmal");
    assert_eq!(
        calls[0].key, calls[1].key,
        "derselbe Schluessel: das Ziel kann Doppeltes erkennen"
    );
    let a_states: Vec<StepState> = w
        .steps(&run)
        .iter()
        .filter(|r| r.step_id == "a")
        .map(|r| r.state)
        .collect();
    assert_eq!(a_states, vec![StepState::Interrupted, StepState::Done]);
}

#[test]
fn a_crash_after_the_action_of_an_external_step_is_never_repeated_by_itself() {
    let w = world();
    smtp(&w, Some(GrantMode::Allow));
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    let (_, run) = w.live(&def(vec![mail_step("m"), note("danach")]));
    crash_once(&w, "m", CrashPoint::AfterAction);
    assert_eq!(mail.call_count(), 1, "die Mail ging hinaus");
    w.clock.advance(61_000);
    let other = w.restart();
    other.register_action(mail.clone());
    let report = other.tick().unwrap();
    assert_eq!(report.recovered, 1);
    let r = w.run(&run);
    assert_eq!(r.state, RunState::Failed);
    assert_eq!(r.error_code.as_deref(), Some("effect_uncertain"));
    assert_eq!(w.steps(&run)[0].state, StepState::Uncertain);
    // Egal wie oft und wie lange: keine zweite Mail, der Folgeschritt bleibt aus.
    for _ in 0..3 {
        w.clock.advance(3_600_000);
        other.tick().unwrap();
        w.engine.tick().unwrap();
    }
    assert_eq!(mail.call_count(), 1);
    assert_eq!(w.notify.call_count(), 0);
    // Der Nutzer entscheidet: ohne ausdrueckliche Bestaetigung geht es nicht.
    assert!(matches!(
        other.retry_run(&run, false),
        Err(WorkflowError::BadInput(_))
    ));
    assert_eq!(mail.call_count(), 1);
    other.retry_run(&run, true).unwrap();
    let report = other.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    assert_eq!(mail.call_count(), 2, "nur auf ausdruecklichen Wunsch");
    assert_eq!(w.notify.call_count(), 1);
}

#[test]
fn a_crash_right_after_the_journal_entry_is_just_as_cautious_for_an_external_step() {
    let w = world();
    smtp(&w, Some(GrantMode::Allow));
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    let (_, run) = w.live(&def(vec![mail_step("m")]));
    crash_once(&w, "m", CrashPoint::AfterBegin);
    assert_eq!(mail.call_count(), 0, "der Baustein ist noch nicht gelaufen");
    w.clock.advance(61_000);
    let other = w.restart();
    other.register_action(mail.clone());
    other.tick().unwrap();
    // Unklar, ob die Mail schon ging (der Absturz kann auch spaeter gewesen sein):
    // sicher ist, dass nichts von selbst doppelt geschieht.
    assert_eq!(w.run(&run).error_code.as_deref(), Some("effect_uncertain"));
    assert_eq!(mail.call_count(), 0);
}

#[test]
fn a_crash_after_the_journal_says_done_resumes_with_the_next_step_only() {
    let w = world();
    let (_, run) = w.live(&def(vec![note("a"), note("b")]));
    w.notify.push_ok(json!({"wert": 7}));
    crash_once(&w, "a", CrashPoint::AfterFinish);
    assert_eq!(w.steps(&run)[0].state, StepState::Done);
    w.clock.advance(61_000);
    let other = w.restart();
    let report = other.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    let calls = w.notify.calls();
    assert_eq!(calls.len(), 2, "a einmal, b einmal");
    assert_eq!(calls[1].params["title"], "Schritt b");
}

#[test]
fn the_effect_can_be_confirmed_after_a_crash_and_then_the_step_counts_as_done() {
    let w = world();
    smtp(&w, Some(GrantMode::Allow));
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    mail.set_confirm(Some(StepOutput::with_data(
        json!({"message_id": "<abc@lva>"}),
    )));
    let (_, run) = w.live(&def(vec![mail_step("m"), note("danach")]));
    crash_once(&w, "m", CrashPoint::AfterAction);
    w.clock.advance(61_000);
    let other = w.restart();
    other.register_action(mail.clone());
    let report = other.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    assert_eq!(mail.call_count(), 1, "bestaetigt, nicht wiederholt");
    let rows = w.steps(&run);
    assert_eq!(rows[0].state, StepState::Done);
    assert!(rows[0].output_json.as_deref().unwrap().contains("abc@lva"));
    assert_eq!(w.notify.call_count(), 1, "der Lauf ging weiter");
}

#[test]
fn a_step_that_keeps_killing_the_app_is_stopped_as_a_crash_loop() {
    let w = world();
    let (_, run) = w.live(&def(vec![note("a")]));
    for round in 0..MAX_INTERRUPTS {
        w.clock.advance(61_000);
        let e = w.restart();
        e.crash_at("a", CrashPoint::AfterAction);
        let report = e.tick().unwrap();
        assert!(
            report
                .errors
                .iter()
                .any(|m| m.contains("simulierter Absturz")),
            "Runde {round}: {report:?}"
        );
    }
    assert_eq!(w.notify.call_count(), MAX_INTERRUPTS);
    w.clock.advance(61_000);
    let report = w.restart().tick().unwrap();
    assert_eq!(report.recovered, 1);
    let r = w.run(&run);
    assert_eq!(r.state, RunState::Failed);
    assert_eq!(r.error_code.as_deref(), Some("crash_loop"));
    assert_eq!(
        w.notify.call_count(),
        MAX_INTERRUPTS,
        "keine weitere Ausfuehrung"
    );
    // Nur mit ausdruecklicher Bestaetigung wieder.
    assert!(matches!(
        w.engine.retry_run(&run, false),
        Err(WorkflowError::BadInput(_))
    ));
}

#[test]
fn a_failing_journal_write_after_the_action_leaves_the_step_running_and_is_handled_after_expiry() {
    let w = world();
    smtp(&w, Some(GrantMode::Allow));
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    let (_, run) = w.live(&def(vec![mail_step("m")]));
    // "Platte voll": das Schreiben von `done` scheitert.
    w.conn()
        .execute_batch(
            "CREATE TRIGGER lva_test_disk_full BEFORE UPDATE ON workflow_run_steps
             WHEN new.state = 'done'
             BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
        )
        .unwrap();
    let err = w.engine.run_next().unwrap_err();
    assert!(matches!(err, WorkflowError::Store(_)), "{err:?}");
    assert_eq!(mail.call_count(), 1, "die Mail ging hinaus");
    assert_eq!(w.steps(&run)[0].state, StepState::Running);
    // Der Arbeiter hat aufgegeben; sein Herzschlag verlaengert nichts mehr.
    w.engine.heartbeat().unwrap();
    w.conn()
        .execute_batch("DROP TRIGGER lva_test_disk_full")
        .unwrap();
    w.clock.advance(61_000);
    let report = w.restart().tick().unwrap();
    assert_eq!(report.recovered, 1);
    assert_eq!(w.run(&run).error_code.as_deref(), Some("effect_uncertain"));
    assert_eq!(mail.call_count(), 1, "nie doppelt");
}

#[test]
fn a_worker_that_was_taken_over_cannot_finish_the_run() {
    let w = world();
    let (_, run) = w.live(&def(vec![note("a")]));
    let other = w.restart();
    // Waehrend der Baustein von A laeuft, steht die Zeit still, A haelt den Lauf zu lange
    // ohne Herzschlag; B uebernimmt und fuehrt den (wiederholbaren) Schritt selbst aus.
    let clock = w.clock.clone();
    let hook_other = other.clone();
    let fired = Arc::new(AtomicBool::new(false));
    let f = fired.clone();
    w.notify.set_hook(Box::new(move |_ctx| {
        if !f.swap(true, Ordering::SeqCst) {
            clock.advance(61_000);
            hook_other.tick().unwrap();
        }
    }));
    let result = w.engine.run_next();
    assert!(
        matches!(result, Err(WorkflowError::LeaseLost)),
        "A darf nach der Uebernahme nichts mehr schreiben: {result:?}"
    );
    let r = w.run(&run);
    assert_eq!(r.state, RunState::Done, "B hat den Lauf zu Ende gebracht");
    let rows = w.steps(&run);
    let states: Vec<StepState> = rows.iter().map(|r| r.state).collect();
    assert_eq!(states, vec![StepState::Interrupted, StepState::Done]);
}

#[test]
fn the_heartbeat_keeps_a_long_step_from_being_taken_over() {
    let w = world();
    let (_, run) = w.live(&def(vec![note("a")]));
    let other = w.restart();
    let engine = w.engine.clone();
    let clock = w.clock.clone();
    let hook_other = other.clone();
    let taken = Arc::new(AtomicUsize::new(0));
    let t = taken.clone();
    w.notify.set_hook(Box::new(move |_ctx| {
        // Zweimal knapp unter dem Ablauf: ohne Herzschlag waere der Lauf weg.
        for _ in 0..2 {
            clock.advance(59_000);
            engine.heartbeat().unwrap();
            t.fetch_add(hook_other.tick().unwrap().recovered, Ordering::SeqCst);
        }
    }));
    let (id, outcome) = w.engine.run_next().unwrap().unwrap();
    assert_eq!((id.as_str(), outcome), (run.as_str(), RunOutcome::Done));
    assert_eq!(taken.load(Ordering::SeqCst), 0, "nichts uebernommen");
    assert_eq!(w.notify.call_count(), 1);
}

// ---------------------------------------------------------------------------
// Schwere Schritte (QG5)
// ---------------------------------------------------------------------------

fn heavy_action() -> Arc<Scripted> {
    Scripted::heavy(
        "meeting.minutes",
        EffectKind::Idempotent,
        HeavyNeed {
            ram_mb: 6_000,
            label: "Sprachmodell",
        },
    )
}

fn minutes_def() -> Value {
    def(vec![step("p", "meeting.minutes", json!({}))])
}

#[test]
fn a_heavy_step_waits_when_the_slot_is_taken_and_costs_no_attempt() {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let gate = roomy_gate();
    let engine = engine_with(&fx, &clock, gate.clone());
    let heavy = heavy_action();
    engine.register_action(heavy.clone());
    let wf = armed_workflow(&engine, &minutes_def());
    let run = engine.enqueue(&EnqueueRequest::manual(&wf)).unwrap().run_id;
    // Ein anderer schwerer Schritt (z. B. eine Transkription) haelt den Platz.
    let held = gate
        .try_enter(&HeavyNeed {
            ram_mb: 1,
            label: "fremd",
        })
        .map_err(|_| "Platz muss frei sein")
        .unwrap();
    let report = engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Parked {
            reason: "heavy_slot".into()
        }
    );
    let parked = store::get_run(&fx.conn(), &run).unwrap().unwrap();
    assert_eq!(parked.state, RunState::Queued);
    assert_eq!(parked.wait_reason.as_deref(), Some("heavy_slot"));
    assert_eq!(parked.next_run_at, Some(T0 + SLOT_RETRY_MS as i64));
    assert!(
        store::steps_for(&fx.conn(), &run).unwrap().is_empty(),
        "kein Versuch verbraucht"
    );
    assert_eq!(heavy.call_count(), 0);

    drop(held);
    clock.advance(SLOT_RETRY_MS as i64);
    let report = engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    let rows = store::steps_for(&fx.conn(), &run).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].attempt, 1);
    assert!(!gate.is_busy(), "der Platz ist wieder frei");
}

#[test]
fn a_heavy_step_waits_for_memory_instead_of_failing() {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let enough = Arc::new(AtomicBool::new(false));
    let probe_flag = enough.clone();
    let gate = Arc::new(LocalHeavyGate::with_probe(Box::new(move |need| {
        if probe_flag.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(format!("zu wenig freier Arbeitsspeicher fuer {need} MB"))
        }
    })));
    let engine = engine_with(&fx, &clock, gate.clone());
    let heavy = heavy_action();
    engine.register_action(heavy.clone());
    let wf = armed_workflow(&engine, &minutes_def());
    let run = engine.enqueue(&EnqueueRequest::manual(&wf)).unwrap().run_id;
    let report = engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Parked {
            reason: "heavy_memory".into()
        }
    );
    let parked = store::get_run(&fx.conn(), &run).unwrap().unwrap();
    assert_eq!(
        parked.state,
        RunState::Queued,
        "wartet, ist nicht gescheitert"
    );
    assert_eq!(parked.next_run_at, Some(T0 + MEMORY_RETRY_MS as i64));
    assert!(
        !gate.is_busy(),
        "ein abgewiesener Schritt haelt keinen Platz"
    );
    assert_eq!(
        heavy.call_count(),
        0,
        "kein Modellstart bei vollem Speicher"
    );
    // Mehrere Takte bei weiterhin vollem Speicher: immer noch kein Fehler.
    for _ in 0..3 {
        clock.advance(MEMORY_RETRY_MS as i64);
        engine.tick().unwrap();
    }
    assert_eq!(
        store::get_run(&fx.conn(), &run).unwrap().unwrap().state,
        RunState::Queued
    );
    enough.store(true, Ordering::SeqCst);
    clock.advance(MEMORY_RETRY_MS as i64);
    let report = engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    assert_eq!(heavy.call_count(), 1);
}

#[test]
fn two_heavy_runs_started_together_run_one_after_the_other() {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let gate = roomy_gate();
    let e1 = engine_with(&fx, &clock, gate.clone());
    let e2 = engine_with(&fx, &clock, gate.clone());
    let heavy = heavy_action();
    e1.register_action(heavy.clone());
    e2.register_action(heavy.clone());
    let current = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    {
        let (c, m) = (current.clone(), most.clone());
        heavy.set_hook(Box::new(move |_| {
            let now = c.fetch_add(1, Ordering::SeqCst) + 1;
            m.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(80));
            c.fetch_sub(1, Ordering::SeqCst);
        }));
    }
    let wf = armed_workflow(&e1, &minutes_def());
    let mut ids = Vec::new();
    for k in ["gleichzeitig-1", "gleichzeitig-2"] {
        let mut req = EnqueueRequest::manual(&wf);
        req.trigger_key = k.into();
        ids.push(e1.enqueue(&req).unwrap().run_id);
    }
    let db = fx.db_path.clone();
    let all_done = {
        let ids = ids.clone();
        move || {
            let conn = Connection::open(&db).unwrap();
            ids.iter()
                .all(|id| store::get_run(&conn, id).unwrap().unwrap().state == RunState::Done)
        }
    };
    let handles: Vec<_> = [e1.clone(), e2.clone()]
        .into_iter()
        .map(|e| {
            let clock = clock.clone();
            let done = all_done.clone();
            std::thread::spawn(move || {
                for _ in 0..400 {
                    e.tick().unwrap();
                    if done() {
                        return;
                    }
                    clock.advance(SLOT_RETRY_MS as i64 + 100);
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert!(all_done(), "beide Laeufe sind fertig");
    assert_eq!(heavy.call_count(), 2);
    assert_eq!(
        most.load(Ordering::SeqCst),
        1,
        "nie zwei schwere Schritte zugleich"
    );
    assert!(!gate.is_busy());
}

// ---------------------------------------------------------------------------
// Warten, Abbruch, Variablen, Grenzen
// ---------------------------------------------------------------------------

#[test]
fn the_wait_step_parks_the_run_and_survives_a_restart_without_stretching_the_wait() {
    let w = world();
    let d = def(vec![
        step("pause", "wait", json!({"minutes": 5})),
        note("danach"),
    ]);
    let (_, run) = w.live(&d);
    let report = w.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Parked {
            reason: "defer".into()
        }
    );
    assert_eq!(w.run(&run).next_run_at, Some(T0 + 5 * 60_000));
    assert_eq!(w.steps(&run)[0].state, StepState::Waiting);
    // Neustart (neue Engine), 4 Minuten spaeter: noch nichts.
    let other = w.restart();
    w.clock.advance(4 * 60_000);
    assert!(other.tick().unwrap().outcomes.is_empty());
    // Nach 5 Minuten ab dem ERSTEN Beginn ist es soweit.
    w.clock.advance(60_000);
    let report = other.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    let rows = w.steps(&run);
    assert_eq!(
        rows.iter().filter(|r| r.step_id == "pause").count(),
        1,
        "ein Versuch, nicht je Aufwachen"
    );
    assert_eq!(rows[0].state, StepState::Done);
    assert_eq!(w.notify.call_count(), 1);
}

#[test]
fn a_queued_run_can_be_cancelled_and_a_running_one_stops_between_steps() {
    let w = world();
    let wf = armed_workflow(&w.engine, &def(vec![note("a")]));
    let queued = w
        .engine
        .enqueue(&EnqueueRequest::manual(&wf))
        .unwrap()
        .run_id;
    assert_eq!(
        w.engine.cancel_run(&queued).unwrap(),
        CancelResult::Cancelled
    );
    assert!(w.engine.tick().unwrap().outcomes.is_empty());
    assert_eq!(w.notify.call_count(), 0);

    // Laufend: der Baustein von Schritt a bricht den eigenen Lauf ab.
    let wf2 = armed_workflow(&w.engine, &{
        let mut d = def(vec![note("a"), note("b")]);
        d["name"] = json!("Mit Abbruch");
        d
    });
    let run = w
        .engine
        .enqueue(&EnqueueRequest::manual(&wf2))
        .unwrap()
        .run_id;
    let engine = w.engine.clone();
    w.notify.set_hook(Box::new(move |ctx| {
        assert_eq!(
            engine.cancel_run(ctx.run_id).unwrap(),
            CancelResult::Requested
        );
    }));
    let report = w.engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Cancelled);
    assert_eq!(w.notify.call_count(), 1, "b lief nicht mehr");
    assert_eq!(w.run(&run).state, RunState::Cancelled);
    assert_eq!(w.steps(&run).len(), 1);
}

#[test]
fn variables_are_declared_typed_and_checked_when_a_run_is_queued() {
    let w = world();
    let mut d = def(vec![step(
        "a",
        "notify.local",
        json!({"title": "Hallo {{vars.name}}"}),
    )]);
    d["variables"] = json!({
        "name": {"type": "string"},
        "anzahl": {"type": "number", "default": 3}
    });
    let wf = armed_workflow(&w.engine, &d);
    // Pflichtvariable ohne Wert.
    let err = w.engine.enqueue(&EnqueueRequest::manual(&wf)).unwrap_err();
    assert!(err.to_string().contains("„name“"), "{err}");
    // Falsche Art, unbekannter Name.
    let mut bad = EnqueueRequest::manual(&wf);
    bad.vars.insert("name".into(), json!(5));
    assert!(matches!(
        w.engine.enqueue(&bad),
        Err(WorkflowError::BadInput(_))
    ));
    let mut unknown = EnqueueRequest::manual(&wf);
    unknown.vars.insert("name".into(), json!("x"));
    unknown.vars.insert("fremd".into(), json!("y"));
    assert!(matches!(
        w.engine.enqueue(&unknown),
        Err(WorkflowError::BadInput(_))
    ));
    // Mit Wert; die Vorgabe der anderen gilt.
    let mut ok = EnqueueRequest::manual(&wf);
    ok.vars.insert("name".into(), json!("Welt"));
    let run = w.engine.enqueue(&ok).unwrap().run_id;
    w.engine.tick().unwrap();
    assert_eq!(w.notify.calls()[0].params["title"], "Hallo Welt");
    let ctx: Value = serde_json::from_str(&w.run(&run).context_json).unwrap();
    assert_eq!(ctx["vars"]["anzahl"], 3);
}

#[test]
fn template_syntax_in_trigger_data_is_never_evaluated_and_a_missing_value_fails_the_step() {
    let w = world();
    let d = def(vec![step(
        "a",
        "notify.local",
        json!({"title": "{{trigger.titel}}"}),
    )]);
    let wf = armed_workflow(&w.engine, &d);
    let mut req = EnqueueRequest::manual(&wf);
    req.trigger = json!({"titel": "{{vars.geheim}} {{run.id}}"});
    w.engine.enqueue(&req).unwrap();
    w.engine.tick().unwrap();
    assert_eq!(
        w.notify.calls()[0].params["title"],
        "{{vars.geheim}} {{run.id}}",
        "Daten bleiben Text"
    );

    // Fehlender Wert: der Schritt scheitert dauerhaft, mit dem Namen, ohne Baustein.
    let w2 = world();
    let d2 = def(vec![step(
        "a",
        "notify.local",
        json!({"title": "{{trigger.fehlt}}"}),
    )]);
    let (_, run) = w2.live(&d2);
    let report = w2.engine.tick().unwrap();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "permanent".into()
        }
    );
    assert!(w2
        .run(&run)
        .error
        .as_deref()
        .unwrap()
        .contains("trigger.fehlt"));
    assert_eq!(w2.notify.call_count(), 0);
}

#[test]
fn the_queue_of_a_workflow_is_bounded_and_the_overflow_is_counted() {
    let w = world();
    let row = w.engine.save_workflow(None, &def(vec![note("a")])).unwrap();
    w.engine.set_enabled(&row.id, true).unwrap();
    for i in 0..store::MAX_QUEUED_PER_WORKFLOW {
        let mut req = EnqueueRequest::manual(&row.id);
        req.trigger_key = format!("k{i}");
        w.engine.enqueue(&req).unwrap();
    }
    let mut over = EnqueueRequest::manual(&row.id);
    over.trigger_key = "zu-viel".into();
    assert!(matches!(
        w.engine.enqueue(&over),
        Err(WorkflowError::QueueFull(_))
    ));
    assert!(matches!(
        w.engine.enqueue(&over),
        Err(WorkflowError::QueueFull(_))
    ));
    assert_eq!(
        w.engine.stats().queue_full,
        2,
        "abgelehnte Laeufe werden gezaehlt"
    );
}

// ---------------------------------------------------------------------------
// Herkunft, Hintergrund
// ---------------------------------------------------------------------------

#[test]
fn every_executed_step_leaves_a_provenance_entry_with_workflow_and_run() {
    let w = world();
    let (wf, run) = w.live(&def(vec![note("a"), note("b")]));
    w.engine.tick().unwrap();
    let detail = w.engine.run_detail(&run).unwrap();
    assert_eq!(detail.run.state, RunState::Done);
    assert_eq!(detail.steps.len(), 2);
    assert_eq!(detail.provenance.len(), 2);
    for (i, p) in detail.provenance.iter().enumerate() {
        assert_eq!(p.subject_id, run);
        assert_eq!(p.operation, "notify_local");
        assert_eq!(p.subject_revision, Some(i as i64 + 1));
        assert_eq!(
            p.actor_ref.as_deref(),
            Some(format!("{wf}/{run}/{}", ["a", "b"][i]).as_str())
        );
        assert!(p.sources.iter().any(|s| s.kind == "trigger"));
        assert!(p
            .params_json
            .as_deref()
            .unwrap()
            .contains("\"action\":\"notify.local\""));
    }
    // Ein Trockenlauf schreibt keine Herkunft (er erzeugt nichts).
    let w2 = world();
    let row = w2
        .engine
        .save_workflow(None, &def(vec![note("a")]))
        .unwrap();
    w2.engine.set_enabled(&row.id, true).unwrap();
    let r = w2.engine.enqueue(&EnqueueRequest::manual(&row.id)).unwrap();
    w2.engine.tick().unwrap();
    assert!(w2
        .engine
        .run_detail(&r.run_id)
        .unwrap()
        .provenance
        .is_empty());
}

#[test]
fn a_broken_provenance_table_never_fails_a_step() {
    let w = world();
    let (_, run) = w.live(&def(vec![note("a")]));
    w.conn().execute_batch("DROP TABLE provenance").unwrap();
    let report = w.engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    assert_eq!(w.steps(&run)[0].state, StepState::Done);
}

#[test]
fn the_background_worker_runs_the_queue_and_stops_cleanly() {
    let fx = Fx::new();
    let cfg = EngineConfig {
        idle_wait_ms: 20,
        heartbeat_ms: 50,
        ..EngineConfig::default()
    };
    let engine = Engine::new(fx.db_path.clone(), roomy_gate(), Arc::new(SystemClock), cfg);
    let notify = Scripted::new("notify.local", EffectKind::Idempotent);
    engine.register_action(notify.clone());
    let wf = armed_workflow(&engine, &def(vec![note("a"), note("b")]));
    let handle = engine.spawn();
    let run = engine.enqueue(&EnqueueRequest::manual(&wf)).unwrap().run_id;
    let mut done = false;
    for _ in 0..300 {
        if engine.run_detail(&run).unwrap().run.state == RunState::Done {
            done = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(done, "der Arbeiter hat den Lauf nicht beendet");
    assert_eq!(notify.call_count(), 2);
    handle.stop();
}

// ---------------------------------------------------------------------------
// B2: Beobachter und Beenden mit Frist
// ---------------------------------------------------------------------------

struct CountingObserver {
    runs: std::sync::Mutex<Vec<String>>,
    panic: AtomicBool,
}

impl EngineObserver for CountingObserver {
    fn run_awaiting_approval(&self, run_id: &str) {
        self.runs.lock().unwrap().push(run_id.to_string());
        if self.panic.load(Ordering::SeqCst) {
            panic!("absichtlicher Absturz des Beobachters");
        }
    }
}

#[test]
fn the_observer_hears_once_each_time_a_run_parks_for_an_approval() {
    let w = world();
    smtp(&w, None);
    let mail = mail_action();
    w.engine.register_action(mail.clone());
    let obs = Arc::new(CountingObserver {
        runs: Default::default(),
        panic: AtomicBool::new(false),
    });
    w.engine.set_observer(obs.clone());
    let (wf, first) = w.live(&def(vec![mail_step("m")]));
    w.engine.tick().unwrap();
    assert_eq!(*obs.runs.lock().unwrap(), vec![first.clone()]);
    // Weitere Takte ohne Aenderung melden nichts.
    for _ in 0..3 {
        w.engine.tick().unwrap();
    }
    assert_eq!(obs.runs.lock().unwrap().len(), 1);
    // Ein zweiter Lauf parkt: eine zweite Meldung.
    let mut req = EnqueueRequest::manual(&wf);
    req.trigger_key = "zweiter".into();
    let second = w.engine.enqueue(&req).unwrap().run_id;
    w.engine.tick().unwrap();
    assert_eq!(*obs.runs.lock().unwrap(), vec![first.clone(), second]);
    // Nach der Freigabe laeuft der Lauf zu Ende: keine weitere Meldung.
    let conn = w.conn();
    for a in approvals::list_pending(&conn, T0).unwrap() {
        approvals::decide(&conn, &a.id, true, T0).unwrap();
    }
    w.engine.tick().unwrap();
    assert_eq!(obs.runs.lock().unwrap().len(), 2);
    assert_eq!(mail.call_count(), 2);
}

#[test]
fn a_panicking_observer_does_not_take_the_engine_down() {
    let w = world();
    smtp(&w, None);
    w.engine.register_action(mail_action());
    let obs = Arc::new(CountingObserver {
        runs: Default::default(),
        panic: AtomicBool::new(true),
    });
    w.engine.set_observer(obs.clone());
    let (_, run) = w.live(&def(vec![mail_step("m")]));
    let report = w.engine.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::AwaitingApproval);
    assert_eq!(obs.runs.lock().unwrap().len(), 1);
    assert_eq!(
        w.run(&run).state,
        RunState::AwaitingApproval,
        "der Lauf ist unberuehrt"
    );
}

fn fast_engine(fx: &Fx) -> Engine {
    let cfg = EngineConfig {
        idle_wait_ms: 20,
        heartbeat_ms: 50,
        ..EngineConfig::default()
    };
    Engine::new(fx.db_path.clone(), roomy_gate(), Arc::new(SystemClock), cfg)
}

#[test]
fn stop_within_ends_an_idle_engine_at_once() {
    let fx = Fx::new();
    let engine = fast_engine(&fx);
    let handle = engine.spawn();
    let t = std::time::Instant::now();
    assert!(handle.stop_within(Duration::from_secs(5)));
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

#[test]
fn stop_within_does_not_wait_for_a_long_step_and_the_run_is_not_cancelled() {
    let fx = Fx::new();
    let engine = fast_engine(&fx);
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let slow = Scripted::new("notify.local", EffectKind::Idempotent);
    {
        let (entered, release) = (entered.clone(), release.clone());
        slow.set_hook(Box::new(move |_| {
            entered.store(true, Ordering::SeqCst);
            let t = std::time::Instant::now();
            while !release.load(Ordering::SeqCst) && t.elapsed() < Duration::from_secs(30) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }));
    }
    engine.register_action(slow.clone());
    let wf = armed_workflow(&engine, &def(vec![note("a")]));
    let handle = engine.spawn();
    let run = engine.enqueue(&EnqueueRequest::manual(&wf)).unwrap().run_id;
    for _ in 0..500 {
        if entered.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        entered.load(Ordering::SeqCst),
        "der Schritt hat nie begonnen"
    );
    // Beenden mit Frist: der Arbeiter steckt im Schritt, die App darf trotzdem enden.
    let t = std::time::Instant::now();
    assert!(!handle.stop_within(Duration::from_millis(150)));
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    // Der Lauf ist NICHT abgebrochen: er laeuft weiter (hier: Schritt ist fertig, wenn er loskommt).
    assert_eq!(
        engine.run_detail(&run).unwrap().run.state,
        RunState::Running
    );
    release.store(true, Ordering::SeqCst);
    let mut state = RunState::Running;
    for _ in 0..500 {
        state = engine.run_detail(&run).unwrap().run.state;
        if state != RunState::Running {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_ne!(
        state,
        RunState::Cancelled,
        "ein Lauf wird beim Beenden nie abgebrochen"
    );
}
