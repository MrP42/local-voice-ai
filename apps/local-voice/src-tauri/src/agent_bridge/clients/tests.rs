use std::collections::HashSet;

use super::*;
use crate::managers::integrations::model::{Kind, NewIntegration};
use crate::managers::integrations::test_support::Fx;

const NOW: i64 = 1_790_000_000_000;

fn table_dump(conn: &Connection, table: &str) -> String {
    let mut stmt = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
    let n = stmt.column_count();
    let rows: Vec<String> = stmt
        .query_map([], |row| {
            let mut parts = Vec::new();
            for i in 0..n {
                let v: rusqlite::types::Value = row.get(i)?;
                parts.push(format!("{v:?}"));
            }
            Ok(parts.join("|"))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows.join("\n")
}

// --- Token ---------------------------------------------------------------

#[test]
fn a_token_has_the_prefix_the_length_and_enough_randomness() {
    let t = generate_token();
    assert!(t.starts_with(TOKEN_PREFIX));
    assert_eq!(t.len(), TOKEN_LEN);
    assert!(token_looks_valid(&t));
    let all: HashSet<String> = (0..200).map(|_| generate_token()).collect();
    assert_eq!(all.len(), 200, "keine Wiederholung");
}

#[test]
fn only_well_formed_tokens_pass_the_shape_check() {
    let good = generate_token();
    assert!(token_looks_valid(&good));
    for bad in [
        "",
        "lvat_",
        "lvat_short",
        &good[1..],
        &format!("{good}x"),
        &good.replace("lvat_", "xxxx_"),
        &format!("lvat_{}'--", "a".repeat(40)),
        &format!("lvat_{}", "ä".repeat(21)),
    ] {
        assert!(!token_looks_valid(bad), "{bad:?}");
    }
}

#[test]
fn the_hash_is_sha256_hex_and_differs_per_token() {
    assert_eq!(
        hash_token("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_ne!(hash_token(&generate_token()), hash_token(&generate_token()));
}

#[test]
fn the_token_is_stored_only_as_a_hash() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (client, token) = create(&conn, "Claude Code", None, NOW).unwrap();
    // Keine Spalte der Tabelle enthaelt das Token.
    for table in ["agent_clients", "agent_tool_grants", "agent_approvals", "audit_log", "integrations"] {
        assert!(
            !table_dump(&conn, table).contains(&token),
            "Token im Klartext in {table}"
        );
    }
    let stored: String = conn
        .query_row(
            "SELECT token_hash FROM agent_clients WHERE id = ?1",
            params![client.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, hash_token(&token));
    // Auch in der Datenbankdatei (nach dem Zurueckschreiben) taucht es nicht auf.
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
    drop(conn);
    let bytes = std::fs::read(&fx.db_path).unwrap();
    let needle = token.as_bytes();
    assert!(
        !bytes.windows(needle.len()).any(|w| w == needle),
        "Token im Klartext in der Datenbankdatei"
    );
    // Der Hash steht dagegen drin.
    let hash = hash_token(&token);
    assert!(bytes.windows(hash.len()).any(|w| w == hash.as_bytes()));
}

// --- Anlegen, Anmelden, Zurueckziehen ---------------------------------------

#[test]
fn creating_makes_the_default_agent_integration_once() {
    let fx = Fx::new();
    let conn = fx.conn();
    assert!(store::list(&conn).unwrap().iter().all(|i| i.kind != Kind::Agent));
    let (a, _) = create(&conn, "Claude Code", None, NOW).unwrap();
    let (b, _) = create(&conn, "Codex", None, NOW + 1).unwrap();
    assert_eq!(a.integration_id, DEFAULT_INTEGRATION_ID);
    assert_eq!(b.integration_id, DEFAULT_INTEGRATION_ID);
    let agents: Vec<_> = store::list(&conn)
        .unwrap()
        .into_iter()
        .filter(|i| i.kind == Kind::Agent)
        .collect();
    assert_eq!(agents.len(), 1, "keine Dublette");
    assert_eq!(agents[0].label, DEFAULT_INTEGRATION_LABEL);
    assert_eq!(agents[0].direction, Direction::Both);
}

#[test]
fn a_client_can_belong_to_a_chosen_agent_integration_but_not_to_another_kind() {
    let fx = Fx::new();
    let conn = fx.conn();
    let other = store::create(&conn, &NewIntegration::new(Kind::Agent, "Zweiter Zugang"), NOW).unwrap();
    let (c, _) = create(&conn, "Skript", Some(&other.id), NOW).unwrap();
    assert_eq!(c.integration_id, other.id);

    let folder = store::create(&conn, &NewIntegration::new(Kind::Folder, "Ablage"), NOW).unwrap();
    assert!(matches!(
        create(&conn, "Falsch", Some(&folder.id), NOW),
        Err(IntegrationError::Invalid(_))
    ));
    assert!(matches!(
        create(&conn, "Falsch", Some("gibt-es-nicht"), NOW),
        Err(IntegrationError::NotFound(_))
    ));
}

#[test]
fn the_issued_token_authenticates_its_client() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (client, token) = create(&conn, "Claude Code", None, NOW).unwrap();
    assert_eq!(authenticate(&conn, &token).unwrap(), client);
}

#[test]
fn an_unknown_or_malformed_token_is_invalid() {
    let fx = Fx::new();
    let conn = fx.conn();
    create(&conn, "Claude Code", None, NOW).unwrap();
    assert_eq!(authenticate(&conn, &generate_token()), Err(AuthError::Invalid));
    assert_eq!(authenticate(&conn, ""), Err(AuthError::Invalid));
    assert_eq!(authenticate(&conn, "lvat_' OR 1=1 --"), Err(AuthError::Invalid));
}

#[test]
fn a_revoked_token_is_reported_as_revoked_and_stays_unusable() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (client, token) = create(&conn, "Claude Code", None, NOW).unwrap();
    assert!(revoke(&conn, &client.id, NOW + 5).unwrap());
    match authenticate(&conn, &token) {
        Err(AuthError::Revoked(c)) => {
            assert_eq!(c.id, client.id);
            assert_eq!(c.revoked_at, Some(NOW + 5));
        }
        other => panic!("{other:?}"),
    }
    // Zweites Zurueckziehen aendert nichts und ist kein Fehler.
    assert!(!revoke(&conn, &client.id, NOW + 99).unwrap());
    assert_eq!(get(&conn, &client.id).unwrap().unwrap().revoked_at, Some(NOW + 5));
    assert!(matches!(revoke(&conn, "gibt-es-nicht", NOW), Err(IntegrationError::NotFound(_))));
}

#[test]
fn a_deleted_client_leaves_nothing_and_its_token_is_invalid() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (client, token) = create(&conn, "Claude Code", None, NOW).unwrap();
    set_tool_mode(&conn, &client.id, "transcribe_file", GrantMode::Ask).unwrap();
    link_approval(&conn, "A1", &client.id, "transcribe_file", NOW).unwrap();
    delete(&conn, &client.id).unwrap();
    assert_eq!(authenticate(&conn, &token), Err(AuthError::Invalid));
    assert!(tool_modes(&conn, &client.id).unwrap().is_empty());
    assert!(approval_link(&conn, "A1").unwrap().is_none());
    assert!(matches!(delete(&conn, &client.id), Err(IntegrationError::NotFound(_))));
}

#[test]
fn labels_are_checked() {
    let fx = Fx::new();
    let conn = fx.conn();
    for bad in ["", "   ", &"x".repeat(MAX_LABEL_CHARS + 1), "zwei\nZeilen", "tab\there"] {
        assert!(
            matches!(create(&conn, bad, None, NOW), Err(IntegrationError::Invalid(_))),
            "{bad:?}"
        );
    }
    let (c, _) = create(&conn, "  Claude Code  ", None, NOW).unwrap();
    assert_eq!(c.label, "Claude Code", "Leerraum an den Raendern faellt weg");
}

#[test]
fn the_number_of_active_clients_is_capped_and_revoking_frees_a_slot() {
    let fx = Fx::new();
    let conn = fx.conn();
    let mut first = None;
    for i in 0..MAX_ACTIVE_CLIENTS {
        let (c, _) = create(&conn, &format!("Zugang {i}"), None, NOW + i).unwrap();
        first.get_or_insert(c.id);
    }
    assert!(matches!(
        create(&conn, "Einer zu viel", None, NOW + 100),
        Err(IntegrationError::Invalid(_))
    ));
    revoke(&conn, &first.unwrap(), NOW + 101).unwrap();
    create(&conn, "Jetzt geht es", None, NOW + 102).unwrap();
}

#[test]
fn last_use_is_written_at_most_once_a_minute() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (c, _) = create(&conn, "Claude Code", None, NOW).unwrap();
    touch(&conn, &c.id, NOW + 1).unwrap();
    assert_eq!(get(&conn, &c.id).unwrap().unwrap().last_used_at, Some(NOW + 1));
    touch(&conn, &c.id, NOW + 1_000).unwrap();
    assert_eq!(
        get(&conn, &c.id).unwrap().unwrap().last_used_at,
        Some(NOW + 1),
        "innerhalb der Minute nicht neu geschrieben"
    );
    touch(&conn, &c.id, NOW + 1 + TOUCH_INTERVAL_MS).unwrap();
    assert_eq!(
        get(&conn, &c.id).unwrap().unwrap().last_used_at,
        Some(NOW + 1 + TOUCH_INTERVAL_MS)
    );
}

// --- Werkzeugrechte -------------------------------------------------------

#[test]
fn a_new_client_has_no_tool_right() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (c, _) = create(&conn, "Claude Code", None, NOW).unwrap();
    for e in catalog::CATALOG {
        assert_eq!(tool_mode(&conn, &c.id, e.name).unwrap(), None, "{}", e.name);
    }
    assert!(tool_modes(&conn, &c.id).unwrap().is_empty());
}

#[test]
fn tool_rights_roundtrip_and_overwrite() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (c, _) = create(&conn, "Claude Code", None, NOW).unwrap();
    set_tool_mode(&conn, &c.id, "transcribe_file", GrantMode::Ask).unwrap();
    assert_eq!(tool_mode(&conn, &c.id, "transcribe_file").unwrap(), Some(GrantMode::Ask));
    set_tool_mode(&conn, &c.id, "transcribe_file", GrantMode::Allow).unwrap();
    assert_eq!(tool_mode(&conn, &c.id, "transcribe_file").unwrap(), Some(GrantMode::Allow));
    set_tool_mode(&conn, &c.id, "create_meeting", GrantMode::Off).unwrap();
    let all = tool_modes(&conn, &c.id).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all["create_meeting"], GrantMode::Off);
}

#[test]
fn a_tool_right_needs_a_catalog_tool_and_a_known_client() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (c, _) = create(&conn, "Claude Code", None, NOW).unwrap();
    assert!(matches!(
        set_tool_mode(&conn, &c.id, "format_disk", GrantMode::Allow),
        Err(IntegrationError::Invalid(_))
    ));
    assert!(matches!(
        set_tool_mode(&conn, "gibt-es-nicht", "transcribe_file", GrantMode::Ask),
        Err(IntegrationError::NotFound(_))
    ));
}

#[test]
fn recording_can_never_be_set_to_allow() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (c, _) = create(&conn, "Claude Code", None, NOW).unwrap();
    for tool in ["start_recording", "stop_recording"] {
        assert!(matches!(
            set_tool_mode(&conn, &c.id, tool, GrantMode::Allow),
            Err(IntegrationError::Invalid(_))
        ));
        set_tool_mode(&conn, &c.id, tool, GrantMode::Ask).unwrap();
    }
}

#[test]
fn an_unreadable_stored_mode_counts_as_no_right() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (c, _) = create(&conn, "Claude Code", None, NOW).unwrap();
    // Die CHECK-Einschraenkung verhindert unbekannte Werte; eine neuere Version koennte sie lockern.
    conn.execute_batch(
        "CREATE TABLE agent_tool_grants_new (client_id TEXT NOT NULL, tool TEXT NOT NULL, mode TEXT NOT NULL, PRIMARY KEY (client_id, tool));
         DROP TABLE agent_tool_grants;
         ALTER TABLE agent_tool_grants_new RENAME TO agent_tool_grants;",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agent_tool_grants (client_id, tool, mode) VALUES (?1, 'transcribe_file', 'root')",
        params![c.id],
    )
    .unwrap();
    assert_eq!(tool_mode(&conn, &c.id, "transcribe_file").unwrap(), None);
    assert!(tool_modes(&conn, &c.id).unwrap().is_empty());
}

// --- Freigabe-Zuordnung ---------------------------------------------------

#[test]
fn an_approval_belongs_to_one_client_and_tool_and_old_links_are_pruned() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (c, _) = create(&conn, "Claude Code", None, NOW).unwrap();
    link_approval(&conn, "A1", &c.id, "transcribe_file", NOW).unwrap();
    assert_eq!(
        approval_link(&conn, "A1").unwrap(),
        Some((c.id.clone(), "transcribe_file".to_string()))
    );
    // Eine zweite Zuordnung derselben Freigabe aendert den Besitzer nicht.
    link_approval(&conn, "A1", "anderer", "create_meeting", NOW + 1).unwrap();
    assert_eq!(approval_link(&conn, "A1").unwrap().unwrap().0, c.id);
    // Nach der doppelten Lebensdauer einer Freigabe verschwindet die Zuordnung.
    link_approval(&conn, "A2", &c.id, "transcribe_file", NOW + APPROVAL_LINK_TTL_MS + 5).unwrap();
    assert!(approval_link(&conn, "A1").unwrap().is_none());
    assert!(approval_link(&conn, "A2").unwrap().is_some());
}
