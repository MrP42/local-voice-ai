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
    for i in 0..MAX_PENDING {
        create(&conn, &new(&format!("h{i}")), T0).unwrap();
    }
    let err = create(&conn, &new("one-too-many"), T0).unwrap_err();
    assert!(
        matches!(err, IntegrationError::Invalid(ref m) if m.contains("Zu viele")),
        "{err}"
    );
    // Nach einer Entscheidung ist wieder Platz.
    let first = list_pending(&conn, T0).unwrap().remove(0);
    decide(&conn, &first.id, false, T0 + 1).unwrap();
    create(&conn, &new("jetzt-wieder"), T0 + 2).unwrap();
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
