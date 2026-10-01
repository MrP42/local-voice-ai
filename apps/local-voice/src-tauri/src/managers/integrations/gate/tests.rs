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
    // Im Audit: Anfrage (pending), die Ausfuehrung (ok) und EINE Zeile fuer die beiden
    // gleichen Verweigerungen (zusammengefasst, Zaehler 2).
    let rows = audit_rows(&conn);
    let outcomes: Vec<&str> = rows.iter().map(|r| r.outcome.as_str()).collect();
    assert_eq!(outcomes.iter().filter(|o| **o == "ok").count(), 1);
    let denied: Vec<_> = rows.iter().filter(|r| r.outcome == "denied").collect();
    assert_eq!(denied.len(), 1);
    let detail: serde_json::Value =
        serde_json::from_str(denied[0].detail_json.as_deref().unwrap()).unwrap();
    assert_eq!(detail["count"], serde_json::json!(2));
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
    // Je Aufrufer und Integration sind nur MAX_PENDING_PER_SOURCE offen erlaubt:
    // verteilt auf mehrere Integrationen erreicht man die globale Grenze.
    let folders: Vec<_> = (0..(approvals::MAX_PENDING / approvals::MAX_PENDING_PER_SOURCE) + 1)
        .map(|i| folder(&conn, &format!("Ablage {i}")))
        .collect();
    for n in 0..approvals::MAX_PENDING {
        let f = &folders[(n / approvals::MAX_PENDING_PER_SOURCE) as usize];
        let args = serde_json::json!({ "n": n });
        let request = Request {
            args: Some(&args),
            ..req(Caller::Workflow, &f.id, Capability::FilesWrite)
        };
        assert!(matches!(
            run(&conn, &request, T0, || Ok::<_, String>(())).unwrap(),
            GateOutcome::Pending { .. }
        ));
    }
    let last = folders.last().unwrap();
    let request = req(Caller::Workflow, &last.id, Capability::FilesWrite);
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

// -- A1n: Haertung nach dem Sicherheits-Review -----------------------------------

fn mail_request<'a>(
    caller: Caller,
    integration_id: &'a str,
    args: &'a serde_json::Value,
) -> Request<'a> {
    Request {
        args: Some(args),
        target: Some("Angebot an Kunden"),
        ..req(caller, integration_id, Capability::FilesWrite)
    }
}

fn pending_approvals(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM approvals WHERE state = 'pending'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn a_long_body_cannot_hide_the_recipients_from_the_user() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    // Alphabetisch steht `body` vor `to`: die alte Vorschau schnitt nach 500 Zeichen ab.
    let args = serde_json::json!({
        "body": "Sehr geehrte Damen und Herren ".repeat(400),
        "to": "kunde@example.invalid",
        "bcc": "spion@example.invalid",
        "attachments": ["C:/Vertraulich/Gehaelter.xlsx"],
    });
    let GateOutcome::Pending { approval_id } = run(
        &conn,
        &mail_request(Caller::Workflow, &f.id, &args),
        T0,
        || Ok::<_, String>(()),
    )
    .unwrap() else {
        panic!("erwartet Pending");
    };
    let preview = approvals::get(&conn, &approval_id)
        .unwrap()
        .unwrap()
        .args_preview
        .unwrap();
    for must_show in [
        "kunde@example.invalid",
        "spion@example.invalid",
        "C:/Vertraulich/Gehaelter.xlsx",
        "Angebot an Kunden",
    ] {
        assert!(
            preview.contains(must_show),
            "{must_show} fehlt in: {preview}"
        );
    }
    assert!(
        preview.contains("gekürzt"),
        "sichtbare Kuerzungsmarke: {preview}"
    );
    assert!(preview.chars().count() <= approvals::MAX_PREVIEW_CHARS);
}

#[test]
fn a_request_whose_recipients_cannot_be_shown_in_full_is_refused_not_clipped() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let many: Vec<String> = (0..60)
        .map(|i| format!("empfaenger-{i:03}@example.invalid"))
        .collect();
    let args = serde_json::json!({ "to": many, "body": "Hallo" });
    let ran = Cell::new(false);
    let out = run(
        &conn,
        &mail_request(Caller::Workflow, &f.id, &args),
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
                code: "preview_unsafe",
                ..
            }
        ),
        "{out:?}"
    );
    assert!(!ran.get());
    assert_eq!(pending_approvals(&conn), 0, "nichts liegt zur Freigabe vor");
    let rows = audit_rows(&conn);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "denied");
    assert!(rows[0]
        .detail_json
        .as_deref()
        .unwrap()
        .contains("preview_unsafe"));
    // Ein ueberlanges Ziel ist genauso unzulaessig.
    let long_target = "C:/".to_string() + &"ordner/".repeat(80) + "datei.txt";
    let too_long = Request {
        target: Some(&long_target),
        ..req(Caller::Workflow, &f.id, Capability::FilesWrite)
    };
    assert!(matches!(
        run(&conn, &too_long, T0, || Ok::<_, String>(())).unwrap(),
        GateOutcome::Denied {
            code: "preview_unsafe",
            ..
        }
    ));
    assert_eq!(pending_approvals(&conn), 0);
}

#[test]
fn a_looping_agent_leaves_one_counted_denial_instead_of_one_row_per_call() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    for i in 0..200 {
        let out = run(
            &conn,
            &req(Caller::AgentExternal, &f.id, Capability::FilesRead),
            T0 + i,
            || Ok::<_, String>(()),
        )
        .unwrap();
        assert!(matches!(out, GateOutcome::Denied { .. }));
    }
    let rows = audit_rows(&conn);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let detail: serde_json::Value =
        serde_json::from_str(rows[0].detail_json.as_deref().unwrap()).unwrap();
    assert_eq!(detail["count"], serde_json::json!(200));
    assert_eq!(detail["reason"], serde_json::json!("grant_off"));
}

#[test]
fn a_hostile_integration_id_reaches_the_audit_only_as_a_placeholder() {
    let fx = Fx::new();
    let conn = fx.conn();
    for hostile in [
        "x\nFaelschung: ok".to_string(),
        format!("token=geheim123{}", "a".repeat(300)),
        "../../etc/passwd".to_string(),
        String::new(),
    ] {
        let out = run(
            &conn,
            &req(Caller::AgentExternal, &hostile, Capability::FilesRead),
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
    }
    let rows = audit_rows(&conn);
    assert!(!rows.is_empty());
    for row in &rows {
        let id = row.integration_id.as_deref().unwrap();
        assert_eq!(id, "(ungültig)", "{row:?}");
    }
    // Eine wohlgeformte, aber unbekannte Kennung bleibt zur Diagnose lesbar.
    run(
        &conn,
        &req(
            Caller::AgentExternal,
            "gibt-es-nicht_1",
            Capability::FilesRead,
        ),
        T0,
        || Ok::<_, String>(()),
    )
    .unwrap();
    assert!(audit_rows(&conn)
        .iter()
        .any(|r| r.integration_id.as_deref() == Some("gibt-es-nicht_1")));
}

#[test]
fn asking_again_for_the_same_action_reuses_the_open_approval() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let args = serde_json::json!({ "to": "kunde@example.invalid" });
    let request = mail_request(Caller::Workflow, &f.id, &args);
    let mut ids = Vec::new();
    for i in 0..5 {
        match run(&conn, &request, T0 + i, || Ok::<_, String>(())).unwrap() {
            GateOutcome::Pending { approval_id } => ids.push(approval_id),
            other => panic!("{other:?}"),
        }
    }
    assert!(ids.iter().all(|i| *i == ids[0]), "{ids:?}");
    assert_eq!(pending_approvals(&conn), 1);
    let pending_rows = audit_rows(&conn)
        .into_iter()
        .filter(|r| r.outcome == "pending")
        .count();
    assert_eq!(pending_rows, 1, "nur die erste Anfrage steht im Audit");
    // Eine andere Aktion bekommt ihre eigene Freigabe.
    let other_args = serde_json::json!({ "to": "anderer@example.invalid" });
    let other = run(
        &conn,
        &mail_request(Caller::Workflow, &f.id, &other_args),
        T0 + 9,
        || Ok::<_, String>(()),
    )
    .unwrap();
    assert!(matches!(&other, GateOutcome::Pending { approval_id } if *approval_id != ids[0]));
    assert_eq!(pending_approvals(&conn), 2);
    // Nach der Entscheidung gilt eine neue Anfrage wieder als neue Freigabe.
    approvals::decide(&conn, &ids[0], false, T0 + 10).unwrap();
    let again = run(&conn, &request, T0 + 11, || Ok::<_, String>(())).unwrap();
    assert!(matches!(&again, GateOutcome::Pending { approval_id } if *approval_id != ids[0]));
}

#[test]
fn one_caller_on_one_integration_cannot_fill_all_approval_slots() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let g = folder(&conn, "Zweite");
    for i in 0..approvals::MAX_PENDING_PER_SOURCE {
        let args = serde_json::json!({ "n": i });
        assert!(matches!(
            run(
                &conn,
                &mail_request(Caller::Workflow, &f.id, &args),
                T0,
                || Ok::<_, String>(())
            )
            .unwrap(),
            GateOutcome::Pending { .. }
        ));
    }
    let args = serde_json::json!({ "n": "elf" });
    let out = run(
        &conn,
        &mail_request(Caller::Workflow, &f.id, &args),
        T0,
        || Ok::<_, String>(()),
    )
    .unwrap();
    assert!(
        matches!(
            out,
            GateOutcome::Denied {
                code: "too_many_pending_caller",
                ..
            }
        ),
        "{out:?}"
    );
    // Andere Aufrufer und andere Integrationen bleiben bedienbar.
    for (caller, id) in [
        (Caller::Workflow, &g.id),
        (Caller::AgentLocal, &g.id),
        (Caller::AgentLocal, &f.id),
    ] {
        assert!(matches!(
            run(&conn, &mail_request(caller, id, &args), T0, || Ok::<
                _,
                String,
            >(
                ()
            ))
            .unwrap(),
            GateOutcome::Pending { .. }
        ));
    }
}

#[test]
fn a_failing_audit_write_leaves_no_open_approval_behind() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    conn.execute_batch(
        "CREATE TRIGGER audit_full BEFORE INSERT ON audit_log
         BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
    )
    .unwrap();
    let request = req(Caller::Workflow, &f.id, Capability::FilesWrite);
    assert!(run(&conn, &request, T0, || Ok::<_, String>(())).is_err());
    assert_eq!(
        pending_approvals(&conn),
        0,
        "keine Freigabe ohne Audit-Zeile"
    );
    // Ist das Audit wieder da, entsteht genau eine Freigabe samt Zeile.
    conn.execute_batch("DROP TRIGGER audit_full").unwrap();
    let GateOutcome::Pending { .. } = run(&conn, &request, T0 + 1, || Ok::<_, String>(())).unwrap()
    else {
        panic!("erwartet Pending");
    };
    assert_eq!(pending_approvals(&conn), 1);
    assert_eq!(audit_rows(&conn).len(), 1);
}

#[test]
fn concurrent_identical_asks_share_one_approval() {
    let fx = Fx::new();
    let f = folder(&fx.conn(), "Ablage");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let (conn, id, barrier) = (fx.conn(), f.id.clone(), barrier.clone());
            std::thread::spawn(move || {
                let args = serde_json::json!({ "to": "kunde@example.invalid" });
                let request = mail_request(Caller::Workflow, &id, &args);
                barrier.wait();
                match run(&conn, &request, T0, || Ok::<_, String>(())).unwrap() {
                    GateOutcome::Pending { approval_id } => approval_id,
                    other => panic!("{other:?}"),
                }
            })
        })
        .collect();
    let ids: std::collections::HashSet<String> =
        threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(ids.len(), 1, "{ids:?}");
    assert_eq!(pending_approvals(&fx.conn()), 1);
}

#[test]
fn an_error_message_of_an_action_loses_the_path_of_an_address() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let out = run(
        &conn,
        &req(Caller::Workflow, &f.id, Capability::FilesRead),
        T0,
        || {
            Err::<(), _>(
                "Abruf https://outlook.office365.com/owa/calendar/x/SCHLUESSEL123/calendar.ics: 403"
                    .to_string(),
            )
        },
    )
    .unwrap();
    let GateOutcome::Failed(message) = out else {
        panic!("erwartet Failed");
    };
    assert!(!message.contains("SCHLUESSEL123"), "{message}");
    assert!(message.contains("outlook.office365.com"), "{message}");
    let detail = audit_rows(&conn)[0].detail_json.clone().unwrap();
    assert!(!detail.contains("SCHLUESSEL123"), "{detail}");
}
