use super::*;
use crate::managers::integrations::test_support::Fx;
use serde_json::json;

fn entry(outcome: AuditOutcome) -> NewAudit {
    NewAudit {
        caller: "agent_external".into(),
        integration_id: Some("i-1".into()),
        capability: Some("mail.send".into()),
        target: Some("anna@example.invalid".into()),
        outcome,
        detail: None,
    }
}

#[test]
fn secret_looking_keys_are_recognised_in_every_spelling() {
    for key in [
        "password",
        "Password",
        "smtp_password",
        "api-key",
        "apiKey",
        "API_KEY",
        "access_token",
        "refreshToken",
        "client_secret",
        "Authorization",
        "private_key",
        "Passwort",
    ] {
        assert!(is_secret_key(key), "{key}");
    }
    for key in [
        "path",
        "host",
        "label",
        "port",
        "username",
        "tokens_used_note_ok",
    ] {
        // `tokens_used_note_ok` enthaelt "token": bewusst streng geschwaerzt.
        let expected = key.contains("token");
        assert_eq!(is_secret_key(key), expected, "{key}");
    }
}

#[test]
fn redact_text_removes_credentials_from_addresses_tokens_and_assignments() {
    let cases = [
        ("https://anna:geheim123@mail.example/x", "geheim123"),
        (
            "Authorization: Bearer abcdef0123456789xyz",
            "abcdef0123456789xyz",
        ),
        ("password=hunter2", "hunter2"),
        ("token: \"abc123secret\"", "abc123secret"),
        ("refresh_token=0.AAAAxyz", "0.AAAAxyz"),
        (
            "jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghij",
            "eyJhbGciOiJIUzI1NiJ9",
        ),
    ];
    for (input, secret) in cases {
        let out = redact_text(input);
        assert!(!out.contains(secret), "{input} -> {out}");
        assert!(out.contains("***"), "{input} -> {out}");
    }
    // Harmloser Text bleibt unberuehrt.
    assert_eq!(
        redact_text("Datei abgelegt unter C:/Ablage/x.md"),
        "Datei abgelegt unter C:/Ablage/x.md"
    );
}

#[test]
fn redact_value_masks_secret_keys_at_any_depth_and_bounds_size() {
    let v = json!({
        "to": "anna@example.invalid",
        "auth": { "password": "p1", "nested": { "apiKey": "k1", "ok": 1 } },
        "list": [ { "token": "t1" }, "https://u:pw@h/" ],
    });
    let out = redact_value(&v);
    let text = out.to_string();
    for secret in ["p1", "k1", "t1", "u:pw@"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    assert_eq!(out["auth"]["nested"]["ok"], json!(1));
    assert_eq!(out["to"], json!("anna@example.invalid"));

    // Listen und Tiefe sind begrenzt.
    let many = Value::Array((0..500).map(|i| json!(i)).collect());
    assert_eq!(redact_value(&many).as_array().unwrap().len(), MAX_ARRAY);
    let mut deep = json!("blatt");
    for _ in 0..20 {
        deep = json!({ "a": deep });
    }
    assert!(redact_value(&deep).to_string().contains('…'));
}

#[test]
fn records_list_newest_first_and_filter() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = record_at(&conn, &entry(AuditOutcome::Denied), 1_000).unwrap();
    let mut other = entry(AuditOutcome::Ok);
    other.caller = "workflow".into();
    other.integration_id = Some("i-2".into());
    let b = record_at(&conn, &other, 2_000).unwrap();
    let c = record_at(&conn, &entry(AuditOutcome::Pending), 3_000).unwrap();
    assert!(a < b && b < c);

    let all = list(&conn, &AuditFilter::default(), 50).unwrap();
    assert_eq!(all.iter().map(|e| e.id).collect::<Vec<_>>(), vec![c, b, a]);
    let by_integration = list(
        &conn,
        &AuditFilter {
            integration_id: Some("i-2".into()),
            ..Default::default()
        },
        50,
    )
    .unwrap();
    assert_eq!(by_integration.len(), 1);
    assert_eq!(by_integration[0].caller, "workflow");
    let denied = list(
        &conn,
        &AuditFilter {
            outcome: Some("denied".into()),
            ..Default::default()
        },
        50,
    )
    .unwrap();
    assert_eq!(denied.len(), 1);
    let recent = list(
        &conn,
        &AuditFilter {
            since_ms: Some(2_500),
            ..Default::default()
        },
        50,
    )
    .unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(list(&conn, &AuditFilter::default(), 1).unwrap().len(), 1);
    // Die Obergrenze je Abfrage.
    assert_eq!(list(&conn, &AuditFilter::default(), 0).unwrap().len(), 1);
}

#[test]
fn the_audit_never_stores_a_secret_from_target_or_detail() {
    let fx = Fx::new();
    let conn = fx.conn();
    let mut e = entry(AuditOutcome::Error);
    e.target = Some("https://anna:geheim123@mail.example/send".into());
    e.detail = Some(json!({ "password": "hunter2", "note": "Bearer abcdef0123456789xyz" }));
    let id = record_at(&conn, &e, 1).unwrap();
    let raw: (String, String) = conn
        .query_row(
            "SELECT target, detail_json FROM audit_log WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    for secret in ["geheim123", "hunter2", "abcdef0123456789xyz"] {
        assert!(
            !raw.0.contains(secret) && !raw.1.contains(secret),
            "{secret}: {raw:?}"
        );
    }
}

#[test]
fn target_and_detail_are_bounded() {
    let fx = Fx::new();
    let conn = fx.conn();
    let mut e = entry(AuditOutcome::Ok);
    e.target = Some("x".repeat(5_000));
    // Jeder Text wird auf 1000 Zeichen gekuerzt; erst viele Felder sprengen die 4 KiB.
    e.detail = Some(json!({
        "a": "y".repeat(50_000), "b": "y".repeat(50_000), "c": "y".repeat(50_000),
        "d": "y".repeat(50_000), "e": "y".repeat(50_000), "f": "y".repeat(50_000),
    }));
    let id = record_at(&conn, &e, 1).unwrap();
    let row = list(&conn, &AuditFilter::default(), 1).unwrap().remove(0);
    assert_eq!(row.id, id);
    assert!(row.target.unwrap().chars().count() <= MAX_TARGET_CHARS + 1);
    assert_eq!(row.detail_json.as_deref(), Some(r#"{"truncated":true}"#));
}

#[test]
fn an_empty_caller_is_refused() {
    let fx = Fx::new();
    let conn = fx.conn();
    let mut e = entry(AuditOutcome::Ok);
    e.caller = "  ".into();
    assert!(matches!(
        record_at(&conn, &e, 1),
        Err(IntegrationError::Invalid(_))
    ));
    assert_eq!(count(&conn).unwrap(), 0);
}

#[test]
fn set_outcome_turns_a_pending_row_into_its_result() {
    let fx = Fx::new();
    let conn = fx.conn();
    let id = record_at(&conn, &entry(AuditOutcome::Pending), 1).unwrap();
    set_outcome(
        &conn,
        id,
        AuditOutcome::Error,
        Some(json!({ "error": "Server 500" })),
    )
    .unwrap();
    let row = list(&conn, &AuditFilter::default(), 1).unwrap().remove(0);
    assert_eq!(row.outcome, "error");
    assert!(row.detail_json.unwrap().contains("Server 500"));
    // Ohne neues Detail bleibt das alte stehen.
    let id2 = record_at(
        &conn,
        &NewAudit {
            detail: Some(json!({ "phase": "running" })),
            ..entry(AuditOutcome::Pending)
        },
        2,
    )
    .unwrap();
    set_outcome(&conn, id2, AuditOutcome::Ok, None).unwrap();
    let row = list(&conn, &AuditFilter::default(), 1).unwrap().remove(0);
    assert_eq!((row.id, row.outcome.as_str()), (id2, "ok"));
    assert!(row.detail_json.unwrap().contains("running"));
}

#[test]
fn retention_keeps_at_most_max_rows_and_drops_the_oldest() {
    let fx = Fx::new();
    let conn = fx.conn();
    // Fast bis zur Grenze mit Rohdaten auffuellen (schnell, eine Transaktion) ...
    conn.execute_batch("BEGIN").unwrap();
    {
        let mut stmt = conn
            .prepare("INSERT INTO audit_log (ts, caller, outcome) VALUES (?1, 'workflow', 'ok')")
            .unwrap();
        for i in 0..MAX_ROWS {
            stmt.execute(params![i]).unwrap();
        }
    }
    conn.execute_batch("COMMIT").unwrap();
    assert_eq!(count(&conn).unwrap(), MAX_ROWS);
    // ... dann schreiben drei Aufrufe ueber die Grenze: die aeltesten fallen weg.
    for i in 0..3 {
        record_at(&conn, &entry(AuditOutcome::Denied), MAX_ROWS + i).unwrap();
    }
    assert_eq!(count(&conn).unwrap(), MAX_ROWS);
    let oldest: i64 = conn
        .query_row("SELECT MIN(ts) FROM audit_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(oldest, 3, "die drei aeltesten Zeilen sind weg");
    let newest = list(&conn, &AuditFilter::default(), 1).unwrap().remove(0);
    assert_eq!(newest.outcome, "denied");
}

#[test]
fn concurrent_writers_keep_every_row() {
    let fx = Fx::new();
    let path = fx.db_path.clone();
    let threads: Vec<_> = (0..4)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                let conn = rusqlite::Connection::open(&path).unwrap();
                for i in 0..25 {
                    let mut e = entry(AuditOutcome::Ok);
                    e.caller = format!("workflow-{t}");
                    record_at(&conn, &e, i).unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let conn = fx.conn();
    assert_eq!(count(&conn).unwrap(), 100);
    let ids: std::collections::HashSet<i64> = list(&conn, &AuditFilter::default(), 500)
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids.len(), 100, "jede Zeile hat ihre eigene Nummer");
}

#[test]
fn a_full_disk_leaves_no_half_written_entry() {
    let fx = Fx::new();
    let conn = fx.conn();
    record_at(&conn, &entry(AuditOutcome::Ok), 1).unwrap();
    // Wie ein voller Datentraeger: ein Trigger bricht das Einfuegen ab.
    conn.execute_batch(
        "CREATE TRIGGER audit_full BEFORE INSERT ON audit_log
         BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
    )
    .unwrap();
    let err = record_at(&conn, &entry(AuditOutcome::Ok), 2).unwrap_err();
    assert!(err.to_string().contains("disk is full"), "{err}");
    assert_eq!(count(&conn).unwrap(), 1, "der alte Stand bleibt");
}
