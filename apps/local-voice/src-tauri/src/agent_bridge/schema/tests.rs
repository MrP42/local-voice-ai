//! Tests der Migration Index 8 (A7): vorwaerts, mit Altdaten, wiederholt, bei Abbruch.

use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};

use super::{AGENT_BRIDGE_MIGRATION, MIGRATION_INDEX};
use crate::managers::meetings::store::{MeetingStore, MIGRATIONS};

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
}

fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// Inhalt der Tabellen der Kette bis Index 7 als Text (zum Vergleich vorher/nachher).
fn snapshot(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let mut tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'agent_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // FTS-Hilfstabellen aendern sich nicht durch uns, ihr Inhalt ist nicht greifbar.
    tables.retain(|t| !t.contains("_fts") && !t.ends_with("_config") && !t.ends_with("_docsize"));
    // Das Oeffnen legt die mitgelieferten Vorlagen an (unabhaengig von diesem Schritt).
    tables.retain(|t| t != "meeting_templates");
    tables
        .into_iter()
        .map(|t| {
            let mut stmt = conn.prepare(&format!("SELECT * FROM \"{t}\"")).unwrap();
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
            (t, rows)
        })
        .collect()
}

/// Eine Datenbank auf dem Stand vor A7 (Index 0 bis 7) mit etwas Inhalt.
fn db_before_a7(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("meetings.db");
    let mut conn = Connection::open(&path).unwrap();
    Migrations::new(MIGRATIONS[..MIGRATION_INDEX].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    assert_eq!(user_version(&conn), MIGRATION_INDEX as i64, "Stand vor A7");
    conn.execute_batch(
        "INSERT INTO meetings (id, title, status, source, created_at, updated_at)
           VALUES ('M1', 'Wochenbesprechung', 'ready', 'live', 1790000000, 1790000000);
         INSERT INTO integrations (id, kind, label, enabled, direction, config_json, created_at, updated_at)
           VALUES ('ordner1', 'folder', 'Ablage', 1, 'both', '{}', 1790000000, 1790000000);
         INSERT INTO integration_grants (integration_id, capability, caller, mode)
           VALUES ('ordner1', 'files.write', 'workflow', 'allow');
         INSERT INTO audit_log (ts, caller, integration_id, capability, outcome)
           VALUES (1790000000, 'workflow', 'ordner1', 'files.write', 'ok');",
    )
    .unwrap();
    path
}

#[test]
fn the_chain_ends_with_the_agent_bridge_step() {
    let mut conn = Connection::open_in_memory().unwrap();
    Migrations::new(MIGRATIONS[..MIGRATION_INDEX].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    for t in ["agent_clients", "agent_tool_grants", "agent_approvals"] {
        assert!(!has_table(&conn, t), "{t} ist vor dem Schritt noch nicht da");
    }
    let mut conn = Connection::open_in_memory().unwrap();
    Migrations::new(MIGRATIONS[..=MIGRATION_INDEX].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    assert_eq!(user_version(&conn), MIGRATION_INDEX as i64 + 1);
    for t in ["agent_clients", "agent_tool_grants", "agent_approvals"] {
        assert!(has_table(&conn, t), "{t} fehlt nach dem Schritt");
    }
    let mut conn = Connection::open_in_memory().unwrap();
    Migrations::new(MIGRATIONS.to_vec())
        .to_latest(&mut conn)
        .unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    for t in ["agent_clients", "agent_tool_grants", "agent_approvals"] {
        assert!(has_table(&conn, t), "{t} fehlt nach der ganzen Kette");
    }
}

#[test]
fn a_database_from_before_a7_keeps_all_its_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = db_before_a7(&dir);
    let before = snapshot(&Connection::open(&path).unwrap());
    assert!(!before.is_empty());

    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    assert_eq!(snapshot(&conn), before, "Altdaten unveraendert");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM agent_clients"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM agent_tool_grants"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM agent_approvals"), 0);
    // Das Register und seine Rechte aus A1 sind unveraendert benutzbar.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM integration_grants WHERE integration_id = 'ordner1'"
        ),
        1
    );
}

#[test]
fn a_second_start_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = db_before_a7(&dir);
    {
        let store = MeetingStore::open_at(&path).unwrap();
        let conn = store.get_connection().unwrap();
        conn.execute(
            "INSERT INTO agent_clients (id, label, integration_id, token_hash, created_at)
             VALUES ('C1', 'Claude Code', 'agents', 'abc', 1790000000)",
            [],
        )
        .unwrap();
    }
    let again = MeetingStore::open_at(&path).unwrap();
    let conn = again.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM agent_clients"),
        1,
        "ein zweiter Start laesst den Zugang stehen und legt nichts doppelt an"
    );
}

#[test]
fn an_abort_in_the_step_leaves_the_database_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let path = db_before_a7(&dir);
    let before = snapshot(&Connection::open(&path).unwrap());

    // Der eigene Schritt bricht am Ende ab: nichts darf halb angelegt zurueckbleiben.
    let broken: &'static str =
        Box::leak(format!("{AGENT_BRIDGE_MIGRATION} SELECT no_such_function();").into_boxed_str());
    let mut steps = MIGRATIONS[..MIGRATION_INDEX].to_vec();
    steps.push(M::up(broken));
    let mut conn = Connection::open(&path).unwrap();
    assert!(Migrations::new(steps).to_latest(&mut conn).is_err());

    assert_eq!(user_version(&conn), MIGRATION_INDEX as i64, "Version unveraendert");
    for t in ["agent_clients", "agent_tool_grants", "agent_approvals"] {
        assert!(!has_table(&conn, t), "{t} darf nicht halb angelegt sein");
    }
    assert_eq!(snapshot(&conn), before, "Altdaten unveraendert");
    drop(conn);

    // Mit dem richtigen Stand startet die App danach ganz normal.
    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    assert!(has_table(&conn, "agent_clients"));
    assert_eq!(snapshot(&conn), before);
}

#[test]
fn the_tool_mode_column_only_accepts_the_three_modes() {
    let mut conn = Connection::open_in_memory().unwrap();
    Migrations::new(MIGRATIONS.to_vec())
        .to_latest(&mut conn)
        .unwrap();
    conn.execute(
        "INSERT INTO agent_clients (id, label, integration_id, token_hash, created_at)
         VALUES ('C1', 'x', 'agents', 'h', 1)",
        [],
    )
    .unwrap();
    let bad = conn.execute(
        "INSERT INTO agent_tool_grants (client_id, tool, mode) VALUES ('C1', 'transcribe_file', 'sometimes')",
        [],
    );
    assert!(bad.is_err());
    conn.execute(
        "INSERT INTO agent_tool_grants (client_id, tool, mode) VALUES ('C1', 'transcribe_file', 'ask')",
        [],
    )
    .unwrap();
}

#[test]
fn two_clients_cannot_share_one_token_hash() {
    let mut conn = Connection::open_in_memory().unwrap();
    Migrations::new(MIGRATIONS.to_vec())
        .to_latest(&mut conn)
        .unwrap();
    let insert = |id: &str| {
        conn.execute(
            "INSERT INTO agent_clients (id, label, integration_id, token_hash, created_at)
             VALUES (?1, 'x', 'agents', 'same', 1)",
            params![id],
        )
    };
    insert("C1").unwrap();
    assert!(insert("C2").is_err(), "token_hash ist eindeutig");
}
