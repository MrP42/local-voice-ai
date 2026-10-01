use rusqlite::params;

use super::*;
use crate::managers::integrations::model::{Caller, Capability, Direction, GrantMode, Kind};
use crate::managers::integrations::store;
use crate::managers::integrations::test_support::{add_calendar_source, folder, Fx};

fn label_of(conn: &Connection, id: &str) -> Option<String> {
    conn.query_row(
        "SELECT label FROM integrations WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )
    .ok()
}

#[test]
fn a_clean_database_needs_no_changes() {
    let fx = Fx::new();
    let conn = fx.conn();
    add_calendar_source(&conn, "src-1", "ics", "Outlook", 100);
    add_calendar_source(&conn, "src-2", "graph", "Arbeit", 200);
    let report = reconcile(&conn).unwrap();
    assert_eq!(report, AdoptReport::default());
    assert_eq!(report.changed(), 0);
}

#[test]
fn a_missing_integration_is_added_again_with_the_same_id() {
    let fx = Fx::new();
    let conn = fx.conn();
    add_calendar_source(&conn, "src-1", "ics", "Outlook", 100);
    conn.execute("DELETE FROM integrations WHERE id = 'src-1'", [])
        .unwrap();
    let report = reconcile(&conn).unwrap();
    assert_eq!((report.inserted, report.updated, report.removed), (1, 0, 0));
    let i = store::get(&conn, "src-1").unwrap().unwrap();
    assert_eq!(
        (i.kind, i.label.as_str(), i.direction),
        (Kind::Ics, "Outlook", Direction::Read)
    );
    // Ein zweiter Lauf findet nichts mehr.
    assert_eq!(reconcile(&conn).unwrap().changed(), 0);
}

#[test]
fn a_diverged_mirror_is_corrected_but_register_only_fields_stay() {
    let fx = Fx::new();
    let conn = fx.conn();
    add_calendar_source(&conn, "src-g", "graph", "Arbeit", 100);
    // Register-eigene Felder, die `reconcile` nie anfassen darf.
    conn.execute(
        "UPDATE integrations SET direction = 'both', config_json = '{\"zone\":\"Europe/Berlin\"}',
                data_class = 'confidential', label = 'Falsch', enabled = 0 WHERE id = 'src-g'",
        [],
    )
    .unwrap();
    store::set_grant(
        &conn,
        "src-g",
        Capability::CalendarWrite,
        Caller::Workflow,
        GrantMode::Allow,
    )
    .unwrap();
    let report = reconcile(&conn).unwrap();
    assert_eq!((report.inserted, report.updated, report.removed), (0, 1, 0));
    let i = store::get(&conn, "src-g").unwrap().unwrap();
    assert_eq!(i.label, "Arbeit", "Name vom Kalender");
    assert!(i.enabled, "Schalter vom Kalender");
    assert_eq!(
        i.direction,
        Direction::Both,
        "Richtung gehoert dem Register"
    );
    assert!(i.config_json.contains("Europe/Berlin"));
    assert_eq!(i.data_class.as_deref(), Some("confidential"));
    assert_eq!(store::list_grants(&conn, "src-g").unwrap().len(), 1);
}

#[test]
fn an_orphan_calendar_integration_is_removed_with_its_grants() {
    let fx = Fx::new();
    let conn = fx.conn();
    // Eine Kalender-Integration ohne Quelle (z. B. von Hand geschrieben).
    conn.execute(
        "INSERT INTO integrations (id, kind, label, enabled, direction, created_at, updated_at)
         VALUES ('waise', 'ics', 'Waise', 1, 'read', 1, 1)",
        [],
    )
    .unwrap();
    store::set_grant(
        &conn,
        "waise",
        Capability::CalendarRead,
        Caller::Workflow,
        GrantMode::Off,
    )
    .unwrap();
    let report = reconcile(&conn).unwrap();
    assert_eq!((report.inserted, report.updated, report.removed), (0, 0, 1));
    assert!(store::get(&conn, "waise").unwrap().is_none());
    assert!(store::list_grants(&conn, "waise").unwrap().is_empty());
}

#[test]
fn a_soft_deleted_source_is_never_adopted_and_its_integration_is_dropped() {
    let fx = Fx::new();
    let conn = fx.conn();
    add_calendar_source(&conn, "src-old", "ics", "Alt", 100);
    // Entfernen, als der Trigger noch nicht da war: Quelle weich geloescht, Integration blieb.
    conn.execute_batch("DROP TRIGGER calendar_sources_mirror_au")
        .unwrap();
    conn.execute(
        "UPDATE calendar_sources SET deleted_at = 5, enabled = 0 WHERE id = 'src-old'",
        [],
    )
    .unwrap();
    assert!(
        store::get(&conn, "src-old").unwrap().is_some(),
        "Trigger fehlt: Waise"
    );
    let report = reconcile(&conn).unwrap();
    assert_eq!(report.removed, 1);
    assert!(store::get(&conn, "src-old").unwrap().is_none());
    // Und sie kommt auch nicht zurueck.
    assert_eq!(reconcile(&conn).unwrap().changed(), 0);
}

#[test]
fn integrations_of_other_kinds_are_left_alone() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    store::set_grant(
        &conn,
        &f.id,
        Capability::FilesWrite,
        Caller::Workflow,
        GrantMode::Allow,
    )
    .unwrap();
    add_calendar_source(&conn, "src-1", "ics", "Outlook", 100);
    let report = reconcile(&conn).unwrap();
    assert_eq!(report.changed(), 0);
    assert!(store::get(&conn, &f.id).unwrap().is_some());
    assert_eq!(store::list_grants(&conn, &f.id).unwrap().len(), 1);
}

#[test]
fn reconcile_is_all_or_nothing() {
    let fx = Fx::new();
    let conn = fx.conn();
    add_calendar_source(&conn, "src-1", "ics", "Outlook", 100);
    conn.execute("DELETE FROM integrations WHERE id = 'src-1'", [])
        .unwrap();
    conn.execute(
        "INSERT INTO integrations (id, kind, label, enabled, direction, created_at, updated_at)
         VALUES ('waise', 'ics', 'Waise', 1, 'read', 1, 1)",
        [],
    )
    .unwrap();
    // Das Entfernen der Waise scheitert (wie ein voller Datentraeger) ...
    conn.execute_batch(
        "CREATE TRIGGER no_delete BEFORE DELETE ON integrations
         BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
    )
    .unwrap();
    assert!(reconcile(&conn).is_err());
    // ... und das vorher eingefuegte src-1 ist mit zurueckgerollt.
    assert!(label_of(&conn, "src-1").is_none(), "kein halber Stand");
    assert!(label_of(&conn, "waise").is_some());
    // Ohne Fehlerquelle heilt der naechste Lauf alles.
    conn.execute_batch("DROP TRIGGER no_delete").unwrap();
    let report = reconcile(&conn).unwrap();
    assert_eq!((report.inserted, report.removed), (1, 1));
}

#[test]
fn the_source_id_stays_the_secret_name_of_the_calendar_namespace() {
    use crate::managers::calendar::secret::{Namespace, SecretRef};
    let fx = Fx::new();
    let conn = fx.conn();
    add_calendar_source(&conn, "graph-3fa9c1d27be04a58", "graph", "Arbeit", 100);
    let i = store::get(&conn, "graph-3fa9c1d27be04a58")
        .unwrap()
        .unwrap();
    let r = crate::managers::integrations::secrets::secret_ref(&i, "token");
    assert_eq!(r, SecretRef::calendar("graph-3fa9c1d27be04a58"));
    assert_eq!(r.ns, Namespace::Calendar);
    assert_eq!(
        r.name().unwrap(),
        "graph-3fa9c1d27be04a58",
        "derselbe Dateiname wie bisher"
    );
}
