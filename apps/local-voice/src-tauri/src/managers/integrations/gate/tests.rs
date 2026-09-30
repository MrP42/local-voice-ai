use std::cell::Cell;

use super::*;
use crate::managers::integrations::audit::{self, AuditFilter};
use crate::managers::integrations::model::{ApprovalState, Kind};
use crate::managers::integrations::test_support::{folder, of_kind, Fx};

const T0: i64 = 10_000_000;

fn req<'a>(caller: Caller, integration_id: &'a str, cap: Capability) -> Request<'a> {
    Request {
        caller,
        integration_id,
        capability: cap,
        target: Some("Protokoll.md"),
        args: None,
        tool_mode: None,
    }
}

fn audit_rows(conn: &Connection) -> Vec<crate::managers::integrations::model::AuditEntry> {
    audit::list(conn, &AuditFilter::default(), 100).unwrap()
}

#[test]
fn the_user_needs_no_approval_and_leaves_no_audit_row() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let ran = Cell::new(0);
    let out = run(
        &conn,
        &req(Caller::User, &f.id, Capability::FilesWrite),
        T0,
        || {
            ran.set(ran.get() + 1);
            Ok::<_, String>(7)
        },
    )
    .unwrap();
    assert_eq!(out, GateOutcome::Done(7));
    assert_eq!(ran.get(), 1);
    assert!(audit_rows(&conn).is_empty());
}

#[test]
fn a_switched_off_integration_denies_even_the_user_with_the_reason() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    crate::managers::integrations::store::update(
        &conn,
        &f.id,
        &crate::managers::integrations::model::IntegrationPatch {
            enabled: Some(false),
            ..Default::default()
        },
        T0,
    )
    .unwrap();
    let ran = Cell::new(false);
    let out = run(
        &conn,
        &req(Caller::User, &f.id, Capability::FilesRead),
        T0,
        || {
            ran.set(true);
            Ok::<_, String>(())
        },
    )
    .unwrap();
    match out {
        GateOutcome::Denied { code, reason } => {
            assert_eq!(code, "integration_disabled");
            assert!(reason.contains("ausgeschaltet"));
        }
        other => panic!("{other:?}"),
    }
    assert!(!ran.get());
}

#[test]
fn an_external_agent_is_denied_by_default_and_the_denial_is_audited() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let ran = Cell::new(false);
    let out = run(
        &conn,
        &req(Caller::AgentExternal, &f.id, Capability::FilesRead),
        T0,
        || {
            ran.set(true);
            Ok::<_, String>(())
        },
    )
    .unwrap();
    assert!(
        matches!(
            out,
            GateOutcome::Denied {
                code: "grant_off",
                ..
            }
        ),
        "{out:?}"
    );
    assert!(!ran.get(), "die Aktion lief nicht");
    let rows = audit_rows(&conn);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "denied");
    assert_eq!(rows[0].caller, "agent_external");
    assert_eq!(rows[0].integration_id.as_deref(), Some(f.id.as_str()));
    assert_eq!(rows[0].capability.as_deref(), Some("files.read"));
    assert!(rows[0]
        .detail_json
        .as_deref()
        .unwrap()
        .contains("grant_off"));
}

#[test]
fn an_unknown_integration_is_denied_and_audited() {
    let fx = Fx::new();
    let conn = fx.conn();
    let out = run(
        &conn,
        &req(Caller::Workflow, "nope", Capability::FilesRead),
        T0,
        || Ok::<_, String>(()),
    )
    .unwrap();
    assert!(matches!(
        out,
        GateOutcome::Denied {
            code: "unknown_integration",
            ..
        }
    ));
    assert_eq!(audit_rows(&conn)[0].outcome, "denied");
}

#[test]
fn ask_creates_one_approval_and_a_pending_audit_row_and_does_not_run() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let ran = Cell::new(false);
    let out = run(
        &conn,
        &req(Caller::Workflow, &f.id, Capability::FilesWrite),
        T0,
        || {
            ran.set(true);
            Ok::<_, String>(())
        },
    )
    .unwrap();
    let GateOutcome::Pending { approval_id } = out else {
        panic!("erwartet Pending");
    };
    assert!(!ran.get());
    let approval = approvals::get(&conn, &approval_id).unwrap().unwrap();
    assert_eq!(approval.state, ApprovalState::Pending);
    assert_eq!(approval.caller, "workflow");
    assert_eq!(approval.tool_or_capability, "files.write");
    assert!(approval.args_preview.unwrap().contains("Protokoll.md"));
    let rows = audit_rows(&conn);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "pending");
    assert!(rows[0]
        .detail_json
        .as_deref()
        .unwrap()
        .contains(&approval_id));
}

#[test]
fn an_allowed_action_is_audited_before_it_runs_and_ok_after() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let during = Cell::new(String::new());
    let out = run(
        &conn,
        &req(Caller::Workflow, &f.id, Capability::FilesRead),
        T0,
        || {
            // Mitten in der Aktion steht der Eintrag schon als „pending“ da.
            let rows = audit::list(&fx.conn(), &AuditFilter::default(), 10).unwrap();
            during.set(rows[0].outcome.clone());
            Ok::<_, String>("fertig")
        },
    )
    .unwrap();
    assert_eq!(out, GateOutcome::Done("fertig"));
    assert_eq!(during.take(), "pending");
    let rows = audit_rows(&conn);
    assert_eq!(rows.len(), 1, "ein Eintrag, nicht zwei");
    assert_eq!(rows[0].outcome, "ok");
}

#[test]
fn a_failing_action_is_audited_as_error_without_the_secret() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let out = run(
        &conn,
        &req(Caller::Workflow, &f.id, Capability::FilesRead),
        T0,
        || Err::<(), _>("Zugriff verweigert: https://anna:geheim123@drive.example/x".to_string()),
    )
    .unwrap();
    let GateOutcome::Failed(message) = out else {
        panic!("erwartet Failed");
    };
    assert!(!message.contains("geheim123"), "{message}");
    let rows = audit_rows(&conn);
    assert_eq!(rows[0].outcome, "error");
    assert!(!rows[0]
        .detail_json
        .as_deref()
        .unwrap()
        .contains("geheim123"));
}

#[test]
fn a_failing_audit_write_blocks_the_action() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    conn.execute_batch(
        "CREATE TRIGGER audit_full BEFORE INSERT ON audit_log
         BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
    )
    .unwrap();
    let ran = Cell::new(false);
    let result = run(
        &conn,
        &req(Caller::Workflow, &f.id, Capability::FilesRead),
        T0,
        || {
            ran.set(true);
            Ok::<_, String>(())
        },
    );
    assert!(result.is_err(), "ohne Audit keine Aktion");
    assert!(!ran.get(), "die Aktion lief NICHT");
    // Auch Verweigerungen und Nachfragen lassen sich nicht verschweigen.
    let denied = run(
        &conn,
        &req(Caller::AgentExternal, &f.id, Capability::FilesRead),
        T0,
        || Ok::<_, String>(()),
    );
    assert!(denied.is_err());
    // Der Nutzer bleibt davon unberuehrt: er erscheint ohnehin nicht im Audit.
    let user = run(
        &conn,
        &req(Caller::User, &f.id, Capability::FilesRead),
        T0,
        || Ok::<_, String>(1),
    )
    .unwrap();
    assert_eq!(user, GateOutcome::Done(1));
}

#[test]
fn an_approved_request_runs_exactly_once() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let request = req(Caller::Workflow, &f.id, Capability::FilesWrite);
    let GateOutcome::Pending { approval_id } =
        run(&conn, &request, T0, || Ok::<_, String>(())).unwrap()
    else {
        panic!("erwartet Pending");
    };
    // Vor der Entscheidung laeuft nichts.
    let early = run_approved(&conn, &approval_id, &request, T0 + 1, || Ok::<_, String>(1)).unwrap();
    assert!(
        matches!(
            early,
            GateOutcome::Denied {
                code: "approval_not_granted",
                ..
            }
        ),
        "{early:?}"
    );

    approvals::decide(&conn, &approval_id, true, T0 + 2).unwrap();
    let runs = Cell::new(0);
    let first = run_approved(&conn, &approval_id, &request, T0 + 3, || {
        runs.set(runs.get() + 1);
        Ok::<_, String>("abgelegt")
    })
    .unwrap();
    assert_eq!(first, GateOutcome::Done("abgelegt"));
    // Zweites Mal mit derselben Genehmigung: verweigert.
    let second = run_approved(&conn, &approval_id, &request, T0 + 4, || {
        runs.set(runs.get() + 1);
        Ok::<_, String>("nochmal")
    })
    .unwrap();
    assert!(
        matches!(
            second,
            GateOutcome::Denied {
                code: "approval_not_granted",
                ..
            }
        ),
        "{second:?}"
    );
    assert_eq!(runs.get(), 1);
    // Im Audit: Anfrage (pending), zwei Verweigerungen, die Ausfuehrung (ok).
    let outcomes: Vec<String> = audit_rows(&conn).into_iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes.iter().filter(|o| o.as_str() == "ok").count(), 1);
    assert_eq!(
        outcomes.iter().filter(|o| o.as_str() == "denied").count(),
        2
    );
}

#[test]
fn an_approval_does_not_carry_over_to_other_arguments() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let args = serde_json::json!({ "to": "anna@example.invalid" });
    let asked = Request {
        args: Some(&args),
        ..req(Caller::Workflow, &f.id, Capability::FilesWrite)
    };
    let GateOutcome::Pending { approval_id } =
        run(&conn, &asked, T0, || Ok::<_, String>(())).unwrap()
    else {
        panic!("erwartet Pending");
    };
    approvals::decide(&conn, &approval_id, true, T0 + 1).unwrap();
    // Dieselbe Genehmigung fuer einen ANDEREN Empfaenger: verweigert, nichts laeuft.
    let other_args = serde_json::json!({ "to": "boese@example.invalid" });
    let swapped = Request {
        args: Some(&other_args),
        ..asked.clone()
    };
    let ran = Cell::new(false);
    let out = run_approved(&conn, &approval_id, &swapped, T0 + 2, || {
        ran.set(true);
        Ok::<_, String>(())
    })
    .unwrap();
    assert!(
        matches!(
            out,
            GateOutcome::Denied {
                code: "approval_mismatch",
                ..
            }
        ),
        "{out:?}"
    );
    assert!(!ran.get());
    // Fuer die genehmigte Aktion gilt sie weiterhin.
    let ok = run_approved(&conn, &approval_id, &asked, T0 + 3, || Ok::<_, String>(())).unwrap();
    assert_eq!(ok, GateOutcome::Done(()));
}

#[test]
fn a_denied_or_expired_approval_never_runs() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let request = req(Caller::Workflow, &f.id, Capability::FilesWrite);
    let GateOutcome::Pending {
        approval_id: denied,
    } = run(&conn, &request, T0, || Ok::<_, String>(())).unwrap()
    else {
        panic!()
    };
    approvals::decide(&conn, &denied, false, T0 + 1).unwrap();
    assert!(matches!(
        run_approved(&conn, &denied, &request, T0 + 2, || Ok::<_, String>(())).unwrap(),
        GateOutcome::Denied { .. }
    ));
    let GateOutcome::Pending { approval_id: stale } =
        run(&conn, &request, T0 + 3, || Ok::<_, String>(())).unwrap()
    else {
        panic!()
    };
    approvals::decide(&conn, &stale, true, T0 + 4).unwrap();
    let late = run_approved(&conn, &stale, &request, T0 + 4 + approvals::TTL_MS, || {
        Ok::<_, String>(())
    })
    .unwrap();
    assert!(
        matches!(
            late,
            GateOutcome::Denied {
                code: "approval_expired",
                ..
            }
        ),
        "{late:?}"
    );
}

#[test]
fn revoking_the_grant_after_approval_wins() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let request = req(Caller::Workflow, &f.id, Capability::FilesWrite);
    let GateOutcome::Pending { approval_id } =
        run(&conn, &request, T0, || Ok::<_, String>(())).unwrap()
    else {
        panic!()
    };
    approvals::decide(&conn, &approval_id, true, T0 + 1).unwrap();
    // Der Nutzer schaltet das Recht danach aus.
    crate::managers::integrations::store::set_grant(
        &conn,
        &f.id,
        Capability::FilesWrite,
        Caller::Workflow,
        GrantMode::Off,
    )
    .unwrap();
    let ran = Cell::new(false);
    let out = run_approved(&conn, &approval_id, &request, T0 + 2, || {
        ran.set(true);
        Ok::<_, String>(())
    })
    .unwrap();
    assert!(
        matches!(
            out,
            GateOutcome::Denied {
                code: "grant_off",
                ..
            }
        ),
        "{out:?}"
    );
    assert!(!ran.get());
    // Die Genehmigung bleibt unverbraucht (kein Verlust, aber auch keine Wirkung).
    assert_eq!(
        approvals::get(&conn, &approval_id).unwrap().unwrap().state,
        ApprovalState::Approved
    );
}

#[test]
fn too_many_open_requests_turn_into_a_denial_not_an_error() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let request = req(Caller::Workflow, &f.id, Capability::FilesWrite);
    for _ in 0..approvals::MAX_PENDING {
        assert!(matches!(
            run(&conn, &request, T0, || Ok::<_, String>(())).unwrap(),
            GateOutcome::Pending { .. }
        ));
    }
    let out = run(&conn, &request, T0, || Ok::<_, String>(())).unwrap();
    assert!(
        matches!(
            out,
            GateOutcome::Denied {
                code: "too_many_pending",
                ..
            }
        ),
        "{out:?}"
    );
}

#[test]
fn recording_is_never_allowed_outright_even_with_a_hand_written_grant() {
    let fx = Fx::new();
    let conn = fx.conn();
    let agent = of_kind(&conn, Kind::Agent, "Claude Code");
    conn.execute(
        "INSERT INTO integration_grants VALUES (?1, 'recording.start', 'agent_external', 'allow')",
        rusqlite::params![agent.id],
    )
    .unwrap();
    let ran = Cell::new(false);
    let out = run(
        &conn,
        &req(Caller::AgentExternal, &agent.id, Capability::RecordingStart),
        T0,
        || {
            ran.set(true);
            Ok::<_, String>(())
        },
    )
    .unwrap();
    assert!(matches!(out, GateOutcome::Pending { .. }), "{out:?}");
    assert!(!ran.get());
}

#[test]
fn the_tool_level_of_an_external_agent_can_switch_a_capability_off() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    crate::managers::integrations::store::set_grant(
        &conn,
        &f.id,
        Capability::FilesRead,
        Caller::AgentExternal,
        GrantMode::Allow,
    )
    .unwrap();
    let mut request = req(Caller::AgentExternal, &f.id, Capability::FilesRead);
    assert!(matches!(
        check(&conn, &request, T0).unwrap(),
        Decision::Allowed
    ));
    request.tool_mode = Some(GrantMode::Off);
    assert!(matches!(
        check(&conn, &request, T0).unwrap(),
        Decision::Denied {
            code: "tool_off",
            ..
        }
    ));
}
