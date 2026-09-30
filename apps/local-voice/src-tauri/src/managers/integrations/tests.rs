//! Migration Index 5 und Uebernahme der Kalenderquellen (AK1): Fixture mit
//! zwei lebenden und einer entfernten Quelle, zweiter Start ohne Dubletten,
//! Abbruch mitten in der Migration, gleichzeitiges Oeffnen.

use std::path::PathBuf;

use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};

use super::model::{Caller, Capability, Direction, GrantMode, Kind};
use super::schema::INTEGRATIONS_MIGRATION;
use super::store;
use crate::managers::calendar::model::CalendarKind;
use crate::managers::meetings::store::{
    backup_before_migration, MeetingStore, ReadOnlyOpenError, SyncMark, MIGRATIONS,
};

const FIXTURE: &str = include_str!("../../../tests/fixtures/integrations/calendar_sources.sql");
const ICS_ID: &str = "01K8Z3Q6M2V7N4T9X5B1C8D0EF";
const GRAPH_ID: &str = "graph-3fa9c1d27be04a58";
const REMOVED_ID: &str = "01K8Z3R0AA0000000000REMOVD";

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// Eine Datenbank auf dem Stand vor A1 (Index 0 bis 4) mit den Zeilen der Fixture.
fn legacy_db_with_fixture() -> (PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meetings.db");
    let mut conn = Connection::open(&path).unwrap();
    Migrations::new(MIGRATIONS[..5].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    assert_eq!(user_version(&conn), 5, "Stand vor A1: Index 0 bis 4");
    conn.execute_batch(FIXTURE).unwrap();
    (path, dir)
}

#[test]
fn migration_5_is_the_next_index_and_one_step() {
    // Index 5 ist die A1-Migration: nach den fuenf Schritten bis M5 (Kalender).
    assert!(MIGRATIONS.len() >= 6, "Index 5 muss es geben");
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    assert_eq!(
        user_version(&store.get_connection().unwrap()),
        MIGRATIONS.len() as i64
    );
}

#[test]
fn migration_5_adopts_the_calendar_sources_with_the_same_ids() {
    let (path, _dir) = legacy_db_with_fixture();
    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();

    let all = store::list(&conn).unwrap();
    let ids: Vec<&str> = all.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![ICS_ID, GRAPH_ID],
        "zwei lebende Quellen, die entfernte nicht"
    );

    let ics = store::get(&conn, ICS_ID).unwrap().unwrap();
    assert_eq!(ics.kind, Kind::Ics);
    assert_eq!(ics.label, "Outlook privat");
    assert_eq!(ics.direction, Direction::Read);
    assert!(ics.enabled);
    assert_eq!(ics.account_hint.as_deref(), Some("outlook.office365.com"));
    assert_eq!(ics.last_ok_at, Some(1_790_000_000_000));
    assert_eq!(ics.created_at, 1_789_000_000_000);
    assert_eq!(ics.config_json, "{}");
    assert!(ics.last_error.is_none());

    let graph = store::get(&conn, GRAPH_ID).unwrap().unwrap();
    assert_eq!(graph.kind, Kind::Graph);
    assert_eq!(graph.label, "Microsoft 365 (Arbeit)");
    assert!(store::get(&conn, REMOVED_ID).unwrap().is_none());

    // Es gelten die Vorgaben: keine gespeicherten Rechte noetig.
    assert!(store::list_grants(&conn, ICS_ID).unwrap().is_empty());
    assert_eq!(
        store::effective(
            &conn,
            ICS_ID,
            Capability::CalendarRead,
            Caller::Workflow,
            None
        )
        .unwrap(),
        GrantMode::Allow
    );
    assert_eq!(
        store::effective(
            &conn,
            ICS_ID,
            Capability::CalendarRead,
            Caller::AgentExternal,
            None
        )
        .unwrap(),
        GrantMode::Off
    );
}

#[test]
fn a_second_start_adds_no_duplicates_and_changes_nothing() {
    let (path, _dir) = legacy_db_with_fixture();
    let first = MeetingStore::open_at(&path).unwrap();
    let before: Vec<(String, i64)> = store::list(&first.get_connection().unwrap())
        .unwrap()
        .into_iter()
        .map(|i| (i.id, i.updated_at))
        .collect();
    drop(first);
    for _ in 0..3 {
        let again = MeetingStore::open_at(&path).unwrap();
        let conn = again.get_connection().unwrap();
        let after: Vec<(String, i64)> = store::list(&conn)
            .unwrap()
            .into_iter()
            .map(|i| (i.id, i.updated_at))
            .collect();
        assert_eq!(after, before, "kein neuer Eintrag, kein neuer Zeitstempel");
        let report = super::adopt::reconcile(&conn).unwrap();
        assert_eq!(report.changed(), 0, "{report:?}");
    }
    let conn = Connection::open(&path).unwrap();
    assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM integrations"), 2);
    assert_eq!(
        scalar(&conn, "SELECT COUNT(DISTINCT id) FROM integrations"),
        2
    );
}

#[test]
fn the_migration_leaves_rows_of_other_tables_untouched() {
    let (path, _dir) = legacy_db_with_fixture();
    let snapshot = |conn: &Connection| -> Vec<(String, String, i64, Option<i64>)> {
        let mut stmt = conn
            .prepare("SELECT id, label, enabled, deleted_at FROM calendar_sources ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    let conn = Connection::open(&path).unwrap();
    let sources_before = snapshot(&conn);
    let events_before = scalar(&conn, "SELECT COUNT(*) FROM calendar_events");
    let templates_before = scalar(&conn, "SELECT COUNT(*) FROM meeting_templates");
    drop(conn);

    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(
        snapshot(&conn),
        sources_before,
        "calendar_sources bleibt, wie sie war"
    );
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM calendar_events"),
        events_before
    );
    assert!(scalar(&conn, "SELECT COUNT(*) FROM meeting_templates") >= templates_before);
    // Und die Tabellen der Kalender-API lesen sich wie vorher.
    assert_eq!(store.calendar_sources().unwrap().len(), 2);
}

#[test]
fn migration_5_is_all_or_nothing_when_it_fails_midway() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meetings.db");
    let mut conn = Connection::open(&path).unwrap();
    Migrations::new(MIGRATIONS[..5].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    conn.execute_batch(FIXTURE).unwrap();

    // Dieselbe Migration, am Ende mit einem Fehler: alles davor muss zurueckrollen.
    let broken_sql: &'static str =
        Box::leak(format!("{INTEGRATIONS_MIGRATION} SELECT no_such_function();").into_boxed_str());
    let mut steps = MIGRATIONS[..5].to_vec();
    steps.push(M::up(broken_sql));
    assert!(Migrations::new(steps).to_latest(&mut conn).is_err());

    assert_eq!(user_version(&conn), 5, "Version unveraendert");
    for table in [
        "integrations",
        "integration_grants",
        "audit_log",
        "approvals",
        "provenance",
    ] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
                params![table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0, "{table} darf nicht halb angelegt sein");
    }
    assert_eq!(
        scalar(
            &conn,
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'calendar_sources_mirror%'"
        ),
        0,
        "keine halb angelegten Spiegel-Trigger"
    );
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM calendar_sources"),
        3,
        "Altdaten heil"
    );

    // Danach laeuft die echte Migration sauber durch (kein halber Zustand im Weg).
    let store = MeetingStore::open_at(&path).unwrap();
    assert_eq!(
        store::list(&store.get_connection().unwrap()).unwrap().len(),
        2
    );
}

#[test]
fn the_schema_has_every_table_trigger_and_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    let conn = store.get_connection().unwrap();
    let names = |kind: &str| -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = ?1")
            .unwrap();
        stmt.query_map(params![kind], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    let tables = names("table");
    for t in [
        "provenance",
        "integrations",
        "integration_grants",
        "audit_log",
        "approvals",
    ] {
        assert!(tables.iter().any(|n| n == t), "Tabelle {t} fehlt");
    }
    let triggers = names("trigger");
    for t in [
        "calendar_sources_mirror_ai",
        "calendar_sources_mirror_au",
        "calendar_sources_mirror_ad",
    ] {
        assert!(triggers.iter().any(|n| n == t), "Trigger {t} fehlt");
    }
    let indexes = names("index");
    for i in [
        "provenance_subject",
        "audit_log_ts",
        "audit_log_integration",
        "approvals_state",
        "integrations_kind",
    ] {
        assert!(indexes.iter().any(|n| n == i), "Index {i} fehlt");
    }
}

#[test]
fn opening_from_several_threads_adopts_once() {
    let (path, _dir) = legacy_db_with_fixture();
    let barrier = std::sync::Barrier::new(6);
    let results: Vec<anyhow::Result<()>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..6)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    MeetingStore::open_at(&path).map(|_| ())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for r in &results {
        assert!(
            r.is_ok(),
            "gleichzeitiges Oeffnen darf nicht scheitern: {r:?}"
        );
    }
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM integrations"), 2);
    assert_eq!(
        scalar(&conn, "SELECT COUNT(DISTINCT id) FROM integrations"),
        2
    );
}

#[test]
fn the_read_only_open_of_the_mcp_server_waits_for_the_migration() {
    let (path, _dir) = legacy_db_with_fixture();
    let busy = std::time::Duration::from_millis(200);
    // Vor dem ersten Start der neuen App: Schema zu alt, der Server migriert nie.
    match MeetingStore::open_read_only(&path, busy) {
        Err(ReadOnlyOpenError::SchemaOlder { found: 5, known })
            if known == MIGRATIONS.len() as i64 => {}
        other => panic!(
            "erwartet SchemaOlder(5, bekannt), war {:?}",
            other.map(|_| ())
        ),
    }
    MeetingStore::open_at(&path).unwrap();
    let ro = MeetingStore::open_read_only(&path, busy).expect("nach der Migration lesbar");
    // Lesend sieht der Server das Register; schreiben kann er nicht.
    let conn = ro.get_connection().unwrap();
    assert_eq!(store::list(&conn).unwrap().len(), 2);
    assert!(conn.execute("DELETE FROM integrations", []).is_err());
}

#[test]
fn a_source_added_later_is_mirrored_in_the_same_transaction() {
    let (path, _dir) = legacy_db_with_fixture();
    let store_ = MeetingStore::open_at(&path).unwrap();
    store_
        .calendar_source_add(
            "01K900NEWSOURCE0000000000A",
            CalendarKind::Ics,
            "Neu",
            Some("host.invalid"),
            1_791_000_000_000,
        )
        .unwrap();
    let conn = store_.get_connection().unwrap();
    let mirrored = store::get(&conn, "01K900NEWSOURCE0000000000A")
        .unwrap()
        .unwrap();
    assert_eq!((mirrored.kind, mirrored.label.as_str()), (Kind::Ics, "Neu"));
    assert_eq!(mirrored.created_at, 1_791_000_000_000);
    assert_eq!(store::list(&conn).unwrap().len(), 3);
}

#[test]
fn renaming_disabling_and_syncing_a_source_is_mirrored_and_keeps_the_grants() {
    let (path, _dir) = legacy_db_with_fixture();
    let store_ = MeetingStore::open_at(&path).unwrap();
    let conn = store_.get_connection().unwrap();
    store::set_grant(
        &conn,
        ICS_ID,
        Capability::CalendarRead,
        Caller::AgentExternal,
        GrantMode::Allow,
    )
    .unwrap();

    store_
        .calendar_source_set_enabled(ICS_ID, false, 1_791_000_000_000)
        .unwrap();
    store_
        .calendar_source_mark_sync(ICS_ID, 1_791_000_100_000, SyncMark::Failed("HTTP 500"))
        .unwrap();
    let i = store::get(&conn, ICS_ID).unwrap().unwrap();
    assert!(!i.enabled, "Schalter gespiegelt");
    assert_eq!(i.last_error.as_deref(), Some("HTTP 500"));
    assert_eq!(i.updated_at, 1_791_000_100_000);
    assert_eq!(
        store::list_grants(&conn, ICS_ID).unwrap().len(),
        1,
        "ein Abruf loescht keine Rechte"
    );

    store_
        .calendar_source_mark_sync(ICS_ID, 1_791_000_200_000, SyncMark::NotModified)
        .unwrap();
    let i = store::get(&conn, ICS_ID).unwrap().unwrap();
    assert_eq!(
        (i.last_ok_at, i.last_error),
        (Some(1_791_000_200_000), None)
    );
}

#[test]
fn removing_a_source_removes_its_integration_and_grants() {
    let (path, _dir) = legacy_db_with_fixture();
    let store_ = MeetingStore::open_at(&path).unwrap();
    let conn = store_.get_connection().unwrap();
    store::set_grant(
        &conn,
        GRAPH_ID,
        Capability::CalendarWrite,
        Caller::Workflow,
        GrantMode::Allow,
    )
    .unwrap();
    store::set_grant(
        &conn,
        ICS_ID,
        Capability::CalendarRead,
        Caller::Workflow,
        GrantMode::Off,
    )
    .unwrap();

    store_
        .calendar_source_remove(GRAPH_ID, 1_791_000_000_000)
        .unwrap();
    assert!(store::get(&conn, GRAPH_ID).unwrap().is_none());
    assert!(store::list_grants(&conn, GRAPH_ID).unwrap().is_empty());
    // Die andere Quelle bleibt samt Recht.
    assert!(store::get(&conn, ICS_ID).unwrap().is_some());
    assert_eq!(store::list_grants(&conn, ICS_ID).unwrap().len(), 1);
    // Ein zweiter Start bringt die entfernte Quelle nicht zurueck.
    drop(conn);
    drop(store_);
    let again = MeetingStore::open_at(&path).unwrap();
    assert!(store::get(&again.get_connection().unwrap(), GRAPH_ID)
        .unwrap()
        .is_none());
}

#[test]
fn a_hard_delete_of_a_source_row_also_removes_its_integration() {
    let (path, _dir) = legacy_db_with_fixture();
    let store_ = MeetingStore::open_at(&path).unwrap();
    let conn = store_.get_connection().unwrap();
    store::set_grant(
        &conn,
        ICS_ID,
        Capability::CalendarRead,
        Caller::Workflow,
        GrantMode::Off,
    )
    .unwrap();
    conn.execute(
        "DELETE FROM calendar_sources WHERE id = ?1",
        params![ICS_ID],
    )
    .unwrap();
    assert!(store::get(&conn, ICS_ID).unwrap().is_none());
    assert!(store::list_grants(&conn, ICS_ID).unwrap().is_empty());
}

// -- Sicherung vor der Migration (R6) ---------------------------------------------

#[test]
fn the_old_database_is_backed_up_once_before_the_migration() {
    let (path, dir) = legacy_db_with_fixture();
    let backup = dir.path().join("meetings.db.bak-v5");
    assert!(!backup.exists());
    MeetingStore::open_at(&path).unwrap();

    // Die Sicherung ist der Stand VOR Index 5: Schema 5, Kalenderquellen, kein Register.
    assert!(backup.exists(), "Sicherung angelegt");
    let old = Connection::open(&backup).unwrap();
    assert_eq!(user_version(&old), 5);
    assert_eq!(scalar(&old, "SELECT COUNT(*) FROM calendar_sources"), 3);
    assert_eq!(
        scalar(
            &old,
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'integrations'"
        ),
        0
    );
    drop(old);
    // Keine Temp-Reste neben der Datenbank.
    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // Ein zweiter Start ueberschreibt sie nicht und legt keine neue an.
    let before = std::fs::read(&backup).unwrap();
    MeetingStore::open_at(&path).unwrap();
    assert_eq!(std::fs::read(&backup).unwrap(), before);
    assert!(!dir.path().join("meetings.db.bak-v6").exists());
}

#[test]
fn a_fresh_or_current_database_gets_no_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meetings.db");
    // Gibt es nicht / leere Datei: nichts zu sichern.
    assert_eq!(backup_before_migration(&path).unwrap(), None);
    std::fs::write(&path, b"").unwrap();
    assert_eq!(backup_before_migration(&path).unwrap(), None);
    std::fs::remove_file(&path).unwrap();
    // Neu angelegt und migriert: keine Sicherung.
    MeetingStore::open_at(&path).unwrap();
    // Aktuell: nichts mehr zu tun.
    assert_eq!(backup_before_migration(&path).unwrap(), None);
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|n| !n.contains(".bak-")), "{names:?}");
}

#[test]
fn a_failing_backup_never_blocks_the_migration() {
    let (path, dir) = legacy_db_with_fixture();
    // Das Ziel der Sicherung ist schon ein Ordner: das Umbenennen scheitert.
    std::fs::create_dir(dir.path().join("meetings.db.bak-v5")).unwrap();
    // Der Helfer meldet den Fehler (oder, je nach Plattform, ueberspringt) ...
    let direct = backup_before_migration(&path);
    assert!(
        matches!(direct, Ok(None)),
        "vorhandenes Ziel zaehlt als gesichert: {direct:?}"
    );
    // ... und der Start migriert trotzdem.
    let store = MeetingStore::open_at(&path).unwrap();
    assert_eq!(
        store::list(&store.get_connection().unwrap()).unwrap().len(),
        2
    );

    // Die Temp-Datei der Sicherung ist blockiert (ein Ordner gleichen Namens): das
    // Schreiben der Sicherung scheitert wirklich, die Migration laeuft trotzdem.
    let (path2, dir2) = legacy_db_with_fixture();
    let blocked = dir2
        .path()
        .join(format!("meetings.db.bak-v5.{}.tmp", std::process::id()));
    std::fs::create_dir(&blocked).unwrap();
    assert!(
        backup_before_migration(&path2).is_err(),
        "der Helfer meldet den Fehler"
    );
    let store2 = MeetingStore::open_at(&path2).unwrap();
    assert_eq!(
        store::list(&store2.get_connection().unwrap())
            .unwrap()
            .len(),
        2
    );
    assert!(
        !dir2.path().join("meetings.db.bak-v5").exists(),
        "keine halbe Sicherung unter dem Endnamen"
    );
}

// -- A1n: die Spiegel-Trigger beruehren nur Eintraege der Kalenderarten ----------

/// Ein Register-Eintrag anderer Art, dessen Kennung zufaellig auch in
/// `calendar_sources` vorkommt (so, wie ihn nur ein Altbestand oder ein
/// Schreiber am Register vorbei erzeugen kann).
fn folder_with_the_id_of_a_calendar_source(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO integrations (id, kind, label, enabled, direction, config_json,
                                   created_at, updated_at)
         VALUES (?1, 'folder', 'Mein Ordner', 1, 'both', '{}', 10, 10)",
        params![id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO integration_grants VALUES (?1, 'files.write', 'workflow', 'allow')",
        params![id],
    )
    .unwrap();
}

fn folder_is_untouched(conn: &Connection, id: &str) {
    let i = store::get(conn, id).unwrap().expect("Eintrag fehlt");
    assert_eq!(i.kind, Kind::Folder);
    assert_eq!(i.label, "Mein Ordner");
    assert!(i.enabled);
    assert_eq!(
        store::list_grants(conn, id).unwrap().len(),
        1,
        "Recht fehlt"
    );
}

#[test]
fn a_calendar_update_does_not_rewrite_or_remove_a_register_entry_of_another_kind() {
    let dir = tempfile::tempdir().unwrap();
    let store_ = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    let conn = store_.get_connection().unwrap();
    folder_with_the_id_of_a_calendar_source(&conn, "gleiche-id");
    crate::managers::integrations::test_support::add_calendar_source(
        &conn,
        "gleiche-id",
        "ics",
        "Kalender",
        20,
    );
    folder_is_untouched(&conn, "gleiche-id");
    // Umbenennen und Ausschalten der Kalenderquelle ...
    conn.execute(
        "UPDATE calendar_sources SET label = 'Umbenannt', enabled = 0, updated_at = 30
         WHERE id = 'gleiche-id'",
        [],
    )
    .unwrap();
    folder_is_untouched(&conn, "gleiche-id");
    // ... ihr Entfernen (weiches Loeschen) ...
    conn.execute(
        "UPDATE calendar_sources SET deleted_at = 40 WHERE id = 'gleiche-id'",
        [],
    )
    .unwrap();
    folder_is_untouched(&conn, "gleiche-id");
    // ... und das harte Loeschen der Zeile.
    conn.execute("DELETE FROM calendar_sources WHERE id = 'gleiche-id'", [])
        .unwrap();
    folder_is_untouched(&conn, "gleiche-id");
}

#[test]
fn the_triggers_still_mirror_and_remove_the_entries_of_the_calendar_kinds() {
    // Gegenprobe: mit dem Kind-Filter bleibt der Normalfall unveraendert.
    let (path, _dir) = legacy_db_with_fixture();
    let store_ = MeetingStore::open_at(&path).unwrap();
    let conn = store_.get_connection().unwrap();
    store::set_grant(
        &conn,
        GRAPH_ID,
        Capability::CalendarWrite,
        Caller::Workflow,
        GrantMode::Allow,
    )
    .unwrap();
    conn.execute(
        "UPDATE calendar_sources SET label = 'Anders' WHERE id = ?1",
        params![ICS_ID],
    )
    .unwrap();
    assert_eq!(store::get(&conn, ICS_ID).unwrap().unwrap().label, "Anders");
    conn.execute(
        "UPDATE calendar_sources SET deleted_at = 99 WHERE id = ?1",
        params![GRAPH_ID],
    )
    .unwrap();
    assert!(store::get(&conn, GRAPH_ID).unwrap().is_none());
    assert!(store::list_grants(&conn, GRAPH_ID).unwrap().is_empty());
}
