use super::*;
use crate::managers::integrations::test_support::Fx;

const T0: i64 = 1_000_000;

fn new<'a>(hash: &'a str) -> NewApproval<'a> {
    NewApproval {
        caller: "agent_external",
        integration_id: Some("i-1"),
        capability: "mail.send",
        args_preview: Some("anna@example.invalid — Angebot"),
        args_hash: Some(hash),
    }
}

fn expect<'a>(hash: &'a str) -> Expect<'a> {
    Expect {
        caller: "agent_external",
        integration_id: Some("i-1"),
        capability: "mail.send",
        args_hash: Some(hash),
    }
}

#[test]
fn a_new_request_is_pending_and_its_preview_is_redacted() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = create(
        &conn,
        &NewApproval {
            args_preview: Some("an https://anna:geheim123@mail.example mit password=hunter2"),
            ..new("h1")
        },
        T0,
    )
    .unwrap();
    assert_eq!(a.state, ApprovalState::Pending);
    assert_eq!(a.created_at, T0);
    assert_eq!(a.decided_at, None);
    let preview = a.args_preview.unwrap();
    assert!(
        !preview.contains("geheim123") && !preview.contains("hunter2"),
        "{preview}"
    );
    assert_eq!(list_pending(&conn, T0 + 1).unwrap().len(), 1);
}

#[test]
fn the_user_can_approve_or_deny_and_the_time_is_kept() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = create(&conn, &new("h1"), T0).unwrap();
    let b = create(&conn, &new("h2"), T0).unwrap();
    let approved = decide(&conn, &a.id, true, T0 + 10).unwrap();
    let denied = decide(&conn, &b.id, false, T0 + 20).unwrap();
    assert_eq!(
        (approved.state, approved.decided_at),
        (ApprovalState::Approved, Some(T0 + 10))
    );
    assert_eq!(
        (denied.state, denied.decided_at),
        (ApprovalState::Denied, Some(T0 + 20))
    );
    assert!(list_pending(&conn, T0 + 30).unwrap().is_empty());
}

#[test]
fn a_decided_request_cannot_be_decided_again() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = create(&conn, &new("h1"), T0).unwrap();
    decide(&conn, &a.id, false, T0 + 1).unwrap();
    // Nachtraeglich „genehmigen“ geht nicht: eine Ablehnung ist endgueltig.
    assert_eq!(
        decide(&conn, &a.id, true, T0 + 2).unwrap_err(),
        ApprovalError::WrongState(ApprovalState::Denied)
    );
    assert_eq!(
        decide(&conn, "gibt-es-nicht", true, T0).unwrap_err(),
        ApprovalError::NotFound
    );
}

#[test]
fn only_one_of_two_concurrent_deciders_wins() {
    let fx = Fx::new();
    let a = create(&fx.conn(), &new("h1"), T0).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|n| {
            let (path, id, barrier) = (fx.db_path.clone(), a.id.clone(), barrier.clone());
            std::thread::spawn(move || {
                let conn = rusqlite::Connection::open(&path).unwrap();
                barrier.wait();
                decide(&conn, &id, n % 2 == 0, T0 + 5).is_ok()
            })
        })
        .collect();
    let winners = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .filter(|won| *won)
        .count();
    assert_eq!(winners, 1, "genau ein Entscheider gewinnt");
    let final_state = get(&fx.conn(), &a.id).unwrap().unwrap().state;
    assert!(matches!(
        final_state,
        ApprovalState::Approved | ApprovalState::Denied
    ));
}

#[test]
fn a_request_expires_after_the_ttl_and_cannot_be_approved() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = create(&conn, &new("h1"), T0).unwrap();
    assert_eq!(list_pending(&conn, T0 + TTL_MS - 1).unwrap().len(), 1);
    assert!(list_pending(&conn, T0 + TTL_MS).unwrap().is_empty());
    assert_eq!(
        decide(&conn, &a.id, true, T0 + TTL_MS).unwrap_err(),
        ApprovalError::Expired
    );
    assert_eq!(
        get(&conn, &a.id).unwrap().unwrap().state,
        ApprovalState::Expired
    );
}

#[test]
fn expire_stale_marks_old_pending_and_old_approved_requests() {
    let fx = Fx::new();
    let conn = fx.conn();
    let old_pending = create(&conn, &new("h1"), T0).unwrap();
    let old_approved = create(&conn, &new("h2"), T0).unwrap();
    decide(&conn, &old_approved.id, true, T0 + 1).unwrap();
    let fresh = create(&conn, &new("h3"), T0 + TTL_MS + 1).unwrap();
    // `create` raeumt selbst auf: die beiden alten sind schon verfallen.
    let state = |id: &str| get(&conn, id).unwrap().unwrap().state;
    assert_eq!(state(&old_pending.id), ApprovalState::Expired);
    assert_eq!(state(&old_approved.id), ApprovalState::Expired);
    assert_eq!(state(&fresh.id), ApprovalState::Pending);
    assert_eq!(
        expire_stale(&conn, T0 + TTL_MS).unwrap(),
        0,
        "nichts mehr zu tun"
    );
}

#[test]
fn an_approval_is_single_use() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = create(&conn, &new("h1"), T0).unwrap();
    decide(&conn, &a.id, true, T0 + 1).unwrap();
    consume(&conn, &a.id, &expect("h1"), T0 + 2).unwrap();
    assert_eq!(
        get(&conn, &a.id).unwrap().unwrap().state,
        ApprovalState::Used
    );
    assert_eq!(
        consume(&conn, &a.id, &expect("h1"), T0 + 3).unwrap_err(),
        ApprovalError::WrongState(ApprovalState::Used)
    );
}

#[test]
fn an_approval_is_bound_to_caller_integration_capability_and_arguments() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = create(&conn, &new("h1"), T0).unwrap();
    decide(&conn, &a.id, true, T0 + 1).unwrap();
    let mismatches = [
        Expect {
            args_hash: Some("anderes-argument"),
            ..expect("h1")
        },
        Expect {
            caller: "workflow",
            ..expect("h1")
        },
        Expect {
            integration_id: Some("i-2"),
            ..expect("h1")
        },
        Expect {
            capability: "files.write",
            ..expect("h1")
        },
        Expect {
            args_hash: None,
            ..expect("h1")
        },
    ];
    for m in &mismatches {
        assert_eq!(
            consume(&conn, &a.id, m, T0 + 2).unwrap_err(),
            ApprovalError::Mismatch,
            "{m:?}"
        );
    }
    // Die Genehmigung blieb unberuehrt und gilt fuer die richtige Aktion.
    assert_eq!(
        get(&conn, &a.id).unwrap().unwrap().state,
        ApprovalState::Approved
    );
    consume(&conn, &a.id, &expect("h1"), T0 + 3).unwrap();
}

#[test]
fn consuming_needs_an_approved_request_not_a_pending_or_denied_one() {
    let fx = Fx::new();
    let conn = fx.conn();
    let pending = create(&conn, &new("h1"), T0).unwrap();
    assert_eq!(
        consume(&conn, &pending.id, &expect("h1"), T0 + 1).unwrap_err(),
        ApprovalError::WrongState(ApprovalState::Pending)
    );
    let denied = create(&conn, &new("h2"), T0).unwrap();
    decide(&conn, &denied.id, false, T0 + 1).unwrap();
    assert_eq!(
        consume(&conn, &denied.id, &expect("h2"), T0 + 2).unwrap_err(),
        ApprovalError::WrongState(ApprovalState::Denied)
    );
    assert_eq!(
        consume(&conn, "nope", &expect("h1"), T0).unwrap_err(),
        ApprovalError::NotFound
    );
}

#[test]
fn an_approval_that_is_not_used_in_time_expires() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = create(&conn, &new("h1"), T0).unwrap();
    decide(&conn, &a.id, true, T0 + 10).unwrap();
    assert_eq!(
        consume(&conn, &a.id, &expect("h1"), T0 + 10 + TTL_MS).unwrap_err(),
        ApprovalError::Expired
    );
    assert_eq!(
        get(&conn, &a.id).unwrap().unwrap().state,
        ApprovalState::Expired
    );
}

#[test]
fn the_number_of_open_requests_is_capped() {
    let fx = Fx::new();
    let conn = fx.conn();
    // Je Integration hoechstens MAX_PENDING_PER_SOURCE: verteilt auf mehrere Integrationen
    // laesst sich die globale Grenze erreichen.
    for i in 0..MAX_PENDING {
        let integration = format!("i-{}", i / MAX_PENDING_PER_SOURCE);
        create(
            &conn,
            &new_for("agent_external", &integration, &format!("h{i}")),
            T0,
        )
        .unwrap();
    }
    let err = open(
        &conn,
        &new_for("agent_external", "i-neu", "one-too-many"),
        T0,
    )
    .unwrap_err();
    assert_eq!(err, OpenError::TooManyPending);
    assert_eq!(err.code(), "too_many_pending");
    let err = create(
        &conn,
        &new_for("agent_external", "i-neu", "one-too-many"),
        T0,
    )
    .unwrap_err();
    assert!(
        matches!(err, IntegrationError::Invalid(ref m) if m.contains("Zu viele")),
        "{err}"
    );
    // Nach einer Entscheidung ist wieder Platz.
    let first = list_pending(&conn, T0).unwrap().remove(0);
    decide(&conn, &first.id, false, T0 + 1).unwrap();
    create(
        &conn,
        &new_for("agent_external", "i-neu", "jetzt-wieder"),
        T0 + 2,
    )
    .unwrap();
}

#[test]
fn the_args_hash_separates_its_parts_and_is_stable() {
    let h = args_hash("i-1", "mail.send", "anna", "{\"a\":1}");
    assert_eq!(h, args_hash("i-1", "mail.send", "anna", "{\"a\":1}"));
    assert_eq!(h.len(), 64);
    // Verschobene Grenzen ergeben einen anderen Wert ("ab"+"c" gegen "a"+"bc").
    assert_ne!(args_hash("ab", "c", "", ""), args_hash("a", "bc", "", ""));
    assert_ne!(h, args_hash("i-1", "mail.send", "bert", "{\"a\":1}"));
    assert_ne!(h, args_hash("i-1", "mail.send", "anna", "{\"a\":2}"));
    assert_ne!(h, args_hash("i-2", "mail.send", "anna", "{\"a\":1}"));
}

// -- A1n: Haertung nach dem Sicherheits-Review -----------------------------------

fn new_for<'a>(caller: &'a str, integration: &'a str, hash: &'a str) -> NewApproval<'a> {
    NewApproval {
        caller,
        integration_id: Some(integration),
        ..new(hash)
    }
}

#[test]
fn an_identical_open_request_is_reused_and_not_duplicated() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = create(&conn, &new("h1"), T0).unwrap();
    let b = create(&conn, &new("h1"), T0 + 5).unwrap();
    assert_eq!(a.id, b.id, "dieselbe offene Freigabe");
    assert_eq!(list_pending(&conn, T0 + 6).unwrap().len(), 1);
    // Jede Abweichung ist eine eigene Anfrage.
    let other_hash = create(&conn, &new("h2"), T0).unwrap();
    let other_caller = create(&conn, &new_for("workflow", "i-1", "h1"), T0).unwrap();
    let other_integration = create(&conn, &new_for("agent_external", "i-2", "h1"), T0).unwrap();
    let other_cap = create(
        &conn,
        &NewApproval {
            capability: "files.write",
            ..new("h1")
        },
        T0,
    )
    .unwrap();
    let ids: std::collections::HashSet<_> = [
        &a.id,
        &other_hash.id,
        &other_caller.id,
        &other_integration.id,
        &other_cap.id,
    ]
    .into_iter()
    .collect();
    assert_eq!(ids.len(), 5);
    // Ohne Hash laesst sich nichts zusammenfassen.
    let no_hash = NewApproval {
        args_hash: None,
        ..new("x")
    };
    let n1 = create(&conn, &no_hash, T0).unwrap();
    let n2 = create(&conn, &no_hash, T0).unwrap();
    assert_ne!(n1.id, n2.id);
}

#[test]
fn a_decided_or_expired_request_is_not_reused() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = create(&conn, &new("h1"), T0).unwrap();
    decide(&conn, &a.id, false, T0 + 1).unwrap();
    let again = create(&conn, &new("h1"), T0 + 2).unwrap();
    assert_ne!(a.id, again.id);
    let stale = create(&conn, &new("h1"), T0 + 2 + TTL_MS).unwrap();
    assert_ne!(
        again.id, stale.id,
        "die verfallene wird nicht wiederverwendet"
    );
    assert_eq!(
        get(&conn, &again.id).unwrap().unwrap().state,
        ApprovalState::Expired
    );
}

#[test]
fn reuse_is_reported_by_open() {
    let fx = Fx::new();
    let conn = fx.conn();
    let first = open(&conn, &new("h1"), T0).unwrap();
    let second = open(&conn, &new("h1"), T0 + 1).unwrap();
    assert!(!first.reused);
    assert!(second.reused);
    assert_eq!(first.approval.id, second.approval.id);
}

#[test]
fn one_caller_on_one_integration_is_capped_below_the_global_limit() {
    let fx = Fx::new();
    let conn = fx.conn();
    for i in 0..MAX_PENDING_PER_SOURCE {
        create(&conn, &new(&format!("h{i}")), T0).unwrap();
    }
    let err = open(&conn, &new("eine-zu-viel"), T0).unwrap_err();
    assert_eq!(err, OpenError::TooManyForSource);
    assert_eq!(err.code(), "too_many_pending_caller");
    // `create` meldet dasselbe als Klartext-Fehler.
    assert!(matches!(
        create(&conn, &new("noch-eine"), T0).unwrap_err(),
        IntegrationError::Invalid(_)
    ));
    // Andere Aufrufer und Integrationen sind nicht betroffen.
    open(&conn, &new_for("workflow", "i-1", "x1"), T0).unwrap();
    open(&conn, &new_for("agent_external", "i-2", "x2"), T0).unwrap();
    // Nach einer Entscheidung ist wieder Platz.
    let first = list_pending(&conn, T0)
        .unwrap()
        .into_iter()
        .find(|a| a.caller == "agent_external" && a.integration_id.as_deref() == Some("i-1"))
        .unwrap();
    decide(&conn, &first.id, false, T0 + 1).unwrap();
    open(&conn, &new("jetzt-wieder"), T0 + 2).unwrap();
}

#[test]
fn a_preview_that_is_too_long_is_refused_and_never_clipped() {
    let fx = Fx::new();
    let conn = fx.conn();
    let long = "x".repeat(MAX_PREVIEW_CHARS + 1);
    let err = open(
        &conn,
        &NewApproval {
            args_preview: Some(&long),
            ..new("h1")
        },
        T0,
    )
    .unwrap_err();
    assert_eq!(err, OpenError::PreviewTooLong);
    assert_eq!(err.code(), "preview_unsafe");
    assert!(
        list_pending(&conn, T0).unwrap().is_empty(),
        "nichts gespeichert"
    );
    // Genau an der Grenze wird sie vollstaendig gespeichert.
    let exact = "x".repeat(MAX_PREVIEW_CHARS);
    let a = open(
        &conn,
        &NewApproval {
            args_preview: Some(&exact),
            ..new("h2")
        },
        T0,
    )
    .unwrap()
    .approval;
    assert_eq!(a.args_preview.unwrap().chars().count(), MAX_PREVIEW_CHARS);
}

#[test]
fn concurrent_identical_requests_end_up_as_one_row() {
    let fx = Fx::new();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let (path, barrier) = (fx.db_path.clone(), barrier.clone());
            std::thread::spawn(move || {
                let conn = rusqlite::Connection::open(&path).unwrap();
                conn.busy_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
                barrier.wait();
                open(&conn, &new("gleich"), T0).unwrap()
            })
        })
        .collect();
    let opened: Vec<Opened> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    let fresh = opened.iter().filter(|o| !o.reused).count();
    assert_eq!(fresh, 1, "genau einer legt an");
    let ids: std::collections::HashSet<_> = opened.iter().map(|o| o.approval.id.clone()).collect();
    assert_eq!(ids.len(), 1);
    assert_eq!(list_pending(&fx.conn(), T0).unwrap().len(), 1);
}
