//! I1 (Integration 0.20.11): die Migrationskette nach dem Zusammenfuehren von
//! A1 (Register, Index 5), A3 (Fassungen, Index 6) und U7 (Warteschlange,
//! Index 7). Geprueft wird der Sprung vom Stand 0.20.9/0.20.10 (Index 0 bis 4,
//! `user_version` 5) ueber alle drei Schritte mit vorhandenen Altdaten, die
//! Wiederholung, der Abbruch im letzten Schritt und die Angleichung einer
//! Datenbank aus einem reinen U7-Build (dort war U7 Index 5).
//!
//! Die Schritte 0 bis 4 sind seit 0.20.9 unveraendert (`git diff` ueber
//! `store.rs` zeigt nur angehaengte Schritte): `MIGRATIONS[..5]` IST die
//! Datenbank von 0.20.9 und 0.20.10.

use rusqlite::types::Value;
use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};

use super::store::{MeetingStore, MIGRATIONS};

const CALENDAR_FIXTURE: &str =
    include_str!("../../../tests/fixtures/integrations/calendar_sources.sql");

/// Die Spalten der Tabellen im Altstand (ohne `meetings.description` aus U7).
const OLD_TABLES: [(&str, &str); 6] = [
    (
        "meetings",
        "id, title, status, source, source_path, started_at, ended_at, language, duration_ms, \
         created_at, updated_at, deleted_at",
    ),
    ("transcripts", "id, meeting_id, model, segments_json, created_at"),
    ("meeting_notes", "meeting_id, blocks_json, revision"),
    ("humans", "id, name, email"),
    ("action_items", "id, meeting_id, text, status, source, kind"),
    ("calendar_sources", "id, kind, label, enabled, deleted_at"),
];

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
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

/// Inhalt einer Tabelle (die genannten Spalten) als Text, in Einfuegereihenfolge.
fn dump(conn: &Connection, table: &str, cols: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {cols} FROM {table} ORDER BY rowid"))
        .unwrap();
    let n = stmt.column_count();
    stmt.query_map([], |row| {
        let mut parts = Vec::with_capacity(n);
        for i in 0..n {
            let v: Value = row.get(i)?;
            parts.push(format!("{v:?}"));
        }
        Ok(parts.join("|"))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn dump_all(conn: &Connection) -> Vec<(&'static str, Vec<String>)> {
    OLD_TABLES
        .iter()
        .map(|(table, cols)| (*table, dump(conn, table, cols)))
        .collect()
}

/// Altdaten, wie 0.20.9 sie hinterlaesst: eine Live-Aufnahme, ein Import, ein
/// Untertitel-Import, eine leere (gescheiterte) Aufnahme, eine geloeschte, dazu
/// Notizen, Personen, Aufgaben und die drei Kalenderquellen der Fixture.
fn seed_0_20_9_data(conn: &Connection) {
    conn.execute_batch(CALENDAR_FIXTURE).unwrap();
    conn.execute_batch(
        "INSERT INTO meetings (id, title, status, source, source_path, started_at, ended_at,
                               language, duration_ms, created_at, updated_at, deleted_at) VALUES
           ('L1', 'Wochenbesprechung', 'ready', 'live', NULL, 1790000000, 1790003600, 'de', 3600000,
            1790000000, 1790003600, NULL),
           ('I1', 'Import Vortrag', 'ready', 'import', 'C:/Aufnahmen/vortrag.wav', NULL, NULL, 'de',
            1800000, 1790100000, 1790100000, NULL),
           ('S1', 'Interview (Untertitel)', 'ready', 'subtitle', 'interview.vtt', NULL, NULL, 'de',
            NULL, 1790200000, 1790200000, NULL),
           ('E1', 'Leere Aufnahme', 'failed', 'live', NULL, NULL, NULL, NULL, NULL,
            1790300000, 1790300000, NULL),
           ('D1', 'Geloeschte Besprechung', 'ready', 'live', NULL, NULL, NULL, 'de', NULL,
            1790400000, 1790400000, 1790500000);
         INSERT INTO meeting_notes (meeting_id, blocks_json, revision, created_at, updated_at)
           VALUES ('L1', '[{\"text\":\"Budget freigeben\"}]', 3, 1790000000, 1790003600);
         INSERT INTO humans (id, name, email, created_at, updated_at)
           VALUES ('H1', 'Anna Beispiel', 'anna@example.invalid', 1790000000, 1790000000);
         INSERT INTO action_items (id, meeting_id, text, status, source, kind, created_at, updated_at)
           VALUES ('A1', 'L1', 'Angebot schicken', 'todo', 'manual', 'action', 1790000000, 1790000000);",
    )
    .unwrap();
    let segments = r#"[{"segment_index":0,"text":"Guten Morgen zusammen","start_ms":0,"end_ms":1800,"channel":2,"speaker_index":null},{"segment_index":1,"text":"Das Budget steht","start_ms":1800,"end_ms":3600,"channel":2,"speaker_index":null}]"#;
    for (tid, mid, json) in [
        ("T1", "L1", segments),
        ("T2", "I1", segments),
        ("T3", "S1", segments),
        ("T4", "E1", "[]"),
    ] {
        conn.execute(
            "INSERT INTO transcripts (id, meeting_id, model, segments_json, created_at, updated_at)
             VALUES (?1, ?2, 'whisper', ?3, 1790000000, 1790000000)",
            params![tid, mid, json],
        )
        .unwrap();
    }
}

/// Eine Datenbank auf dem Stand 0.20.9/0.20.10 samt Altdaten.
fn legacy_db(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("meetings.db");
    let mut conn = Connection::open(&path).unwrap();
    Migrations::new(MIGRATIONS[..5].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    assert_eq!(user_version(&conn), 5, "Stand 0.20.9: Index 0 bis 4");
    seed_0_20_9_data(&conn);
    path
}

/// Werkzeug fuer den Rauchtest der Release-EXE (kein Test): schreibt eine
/// Datenbank im Stand 0.20.9 samt Altdaten nach `LVA_SMOKE_LEGACY_DB`; die EXE
/// migriert sie dann in einer Sandbox (`LVA_MEETINGS_DIR`).
///
/// LVA_SMOKE_LEGACY_DB=<pfad>/meetings.db cargo test --lib write_legacy_db -- --ignored
#[test]
#[ignore = "Werkzeug: schreibt die Datei aus LVA_SMOKE_LEGACY_DB"]
fn write_legacy_db_for_the_release_smoke_test() {
    let target = std::env::var("LVA_SMOKE_LEGACY_DB").expect("LVA_SMOKE_LEGACY_DB setzen");
    let target = std::path::PathBuf::from(target);
    assert!(!target.exists(), "die Datei darf noch nicht existieren");
    let mut conn = Connection::open(&target).unwrap();
    Migrations::new(MIGRATIONS[..5].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    seed_0_20_9_data(&conn);
    println!("LEGACY_DB={}", target.display());
}

/// Wie jede Stufe der Kette aussieht: Version und Tabellen.
#[test]
fn the_chain_is_register_then_variants_then_queue() {
    assert_eq!(MIGRATIONS.len(), 9, "A1 = 5, A3 = 6, U7 = 7, A7 = 8");
    let steps: [(usize, &[&str], &[&str]); 5] = [
        (5, &[], &["integrations", "transcript_variants", "import_queue"]),
        (
            6,
            &["integrations", "provenance"],
            &["transcript_variants", "import_queue"],
        ),
        (
            7,
            &["integrations", "transcript_variants"],
            &["import_queue"],
        ),
        (
            8,
            &["integrations", "transcript_variants", "import_queue"],
            &["agent_clients"],
        ),
        (
            9,
            &[
                "integrations",
                "transcript_variants",
                "import_queue",
                "agent_clients",
                "agent_tool_grants",
                "agent_approvals",
            ],
            &[],
        ),
    ];
    for (upto, present, absent) in steps {
        let mut conn = Connection::open_in_memory().unwrap();
        Migrations::new(MIGRATIONS[..upto].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        assert_eq!(user_version(&conn), upto as i64);
        for t in present {
            assert!(has_table(&conn, t), "{t} fehlt nach Index {}", upto - 1);
        }
        for t in absent {
            assert!(!has_table(&conn, t), "{t} ist zu frueh da (Stufe {upto})");
        }
    }
}

#[test]
fn a_database_of_0_20_9_with_data_migrates_through_all_three_steps() {
    let dir = tempfile::tempdir().unwrap();
    let path = legacy_db(&dir);
    let before = {
        let conn = Connection::open(&path).unwrap();
        dump_all(&conn)
    };
    assert_eq!(before[0].1.len(), 5, "fuenf Besprechungen im Altstand");

    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    assert_eq!(MIGRATIONS.len(), 9);

    // Nichts der Altdaten ging verloren oder wurde veraendert.
    assert_eq!(dump_all(&conn), before, "Altdaten unveraendert");

    // A1: die zwei lebenden Kalenderquellen stehen im Register, die entfernte nicht.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM integrations"), 2);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM provenance"), 0);
    // A3: jedes nicht leere Transkript ist Fassung 1 (aktiv), das leere keine.
    let variants = dump(
        &conn,
        "transcript_variants",
        "meeting_id, kind, number, active",
    );
    assert_eq!(variants.len(), 3, "L1, I1, S1; das leere E1 nicht: {variants:?}");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM transcript_variants WHERE number = 1 AND active = 1"
        ),
        3
    );
    // U7: Beschreibung leer, Warteschlange leer, alle Besprechungen wie zuvor.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM meetings WHERE description IS NOT NULL"
        ),
        0
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM import_queue"), 0);
    assert!(store.queue_rows().unwrap().is_empty());
    let l1 = store.get_meeting("L1").unwrap().unwrap();
    assert_eq!(l1.title, "Wochenbesprechung");
    assert_eq!(l1.description, None);
    assert_eq!(store.get_segments("L1").unwrap().len(), 2);

    // Die Sicherung des alten Standes liegt daneben und ist der Altstand.
    let bak = path.with_file_name("meetings.db.bak-v5");
    assert!(bak.is_file(), "Sicherung vor der Migration fehlt");
    let backup = Connection::open(&bak).unwrap();
    assert_eq!(user_version(&backup), 5);
    assert_eq!(dump_all(&backup), before, "die Sicherung ist der Altstand");
    assert!(!has_table(&backup, "import_queue"));
    drop(backup);
    drop(conn);
    drop(store);

    // Wiederholung (zweiter und dritter Start): nichts doppelt, nichts geaendert.
    for _ in 0..2 {
        let again = MeetingStore::open_at(&path).unwrap();
        let conn = again.get_connection().unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
        assert_eq!(dump_all(&conn), before);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM transcript_variants"), 3);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM integrations"), 2);
    }
}

#[test]
fn an_abort_in_the_last_step_leaves_the_0_20_9_database_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = legacy_db(&dir);
    let before = {
        let conn = Connection::open(&path).unwrap();
        dump_all(&conn)
    };

    // A1 und A3 laufen durch, U7 bricht am Ende ab: alles muss zurueckrollen
    // (eine Transaktion ueber die ganze Kette).
    let broken: &'static str = Box::leak(
        format!("{} SELECT no_such_function();", super::queue_store::QUEUE_MIGRATION)
            .into_boxed_str(),
    );
    let mut steps = MIGRATIONS[..7].to_vec();
    steps.push(M::up(broken));
    let mut conn = Connection::open(&path).unwrap();
    assert!(Migrations::new(steps).to_latest(&mut conn).is_err());

    assert_eq!(user_version(&conn), 5, "Version unveraendert");
    for t in [
        "integrations",
        "provenance",
        "transcript_variants",
        "import_queue",
    ] {
        assert!(!has_table(&conn, t), "{t} darf nicht halb angelegt sein");
    }
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'calendar_sources_mirror%'"
        ),
        0
    );
    assert_eq!(dump_all(&conn), before, "Altdaten unveraendert");
    drop(conn);

    // Mit dem richtigen Stand startet die App danach ganz normal.
    let store = MeetingStore::open_at(&path).unwrap();
    assert_eq!(
        user_version(&store.get_connection().unwrap()),
        MIGRATIONS.len() as i64
    );
    assert_eq!(dump_all(&store.get_connection().unwrap()), before);
}

#[test]
fn a_database_from_a_u7_only_build_is_adopted() {
    // Ein reiner U7-Build kannte nur Index 0 bis 4 und die Warteschlange als
    // Index 5: Version 6, mit einer wartenden Datei und einer Beschreibung.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meetings.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        let mut steps = MIGRATIONS[..5].to_vec();
        steps.push(M::up(super::queue_store::QUEUE_MIGRATION));
        Migrations::new(steps).to_latest(&mut conn).unwrap();
        assert_eq!(user_version(&conn), 6);
        seed_0_20_9_data(&conn);
        conn.execute(
            "UPDATE meetings SET description = 'Notiz zur Aufnahme' WHERE id = 'L1'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE meetings SET status = 'queued' WHERE id = 'E1'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO import_queue (meeting_id, seq, state, enqueued_at)
             VALUES ('E1', 1, 'waiting', 1790300000)",
            [],
        )
        .unwrap();
    }
    let before = {
        let conn = Connection::open(&path).unwrap();
        dump_all(&conn)
    };

    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64, "nach A1, A3, U7 und A7");
    assert_eq!(dump_all(&conn), before, "Altdaten unveraendert");
    assert!(has_table(&conn, "integrations"), "A1 nachgeholt");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM integrations"), 2);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM transcript_variants"),
        3,
        "A3 nachgeholt, Fassungen zurueckgefuellt"
    );
    assert_eq!(
        store.get_meeting("L1").unwrap().unwrap().description.as_deref(),
        Some("Notiz zur Aufnahme"),
        "die Beschreibung aus U7 bleibt"
    );
    let rows = store.queue_rows().unwrap();
    assert_eq!(rows.len(), 1, "die wartende Datei bleibt in der Warteschlange");
    assert_eq!(rows[0].meeting_id, "E1");
    drop(conn);
    drop(store);

    // Zweiter Start: nichts mehr anzugleichen, nichts doppelt.
    let again = MeetingStore::open_at(&path).unwrap();
    let conn = again.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM transcript_variants"), 3);
}

#[test]
fn a_database_of_the_merged_order_is_never_mistaken_for_a_u7_only_one() {
    // Version 6 des zusammengefuehrten Standes (nur A1) hat kein `import_queue`:
    // die Angleichung greift nicht, die Kette wendet A3 und U7 an.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meetings.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        Migrations::new(MIGRATIONS[..6].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        assert_eq!(user_version(&conn), 6);
        assert!(!has_table(&conn, "import_queue"));
    }
    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    assert!(has_table(&conn, "transcript_variants"));
    assert!(has_table(&conn, "import_queue"));
}
