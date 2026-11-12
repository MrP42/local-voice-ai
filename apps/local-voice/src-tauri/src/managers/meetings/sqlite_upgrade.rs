//! RUSTSEC-2026-0459: Datenbanken, die mit rusqlite 0.37 (gebuendeltes SQLite 3.50)
//! entstanden sind, oeffnen ueber den echten Pfad der App (`MeetingStore::open_at`)
//! unter rusqlite 0.41 (SQLite 3.54): Daten lesen, zweimal starten (Migrationskette auf
//! aktuellem Stand, `to_latest` darf nichts aendern), schreiben und
//! wieder oeffnen, `integrity_check` und Fremdschluessel, dazu ein Stand mit
//! committeten, aber nicht checkpointeten WAL-Daten.
//!
//! Fixtures: `tests/fixtures/sqlite-037/` (Herkunft in `ERZEUGT_MIT.txt`).

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::managers::meetings::store::{MeetingStore, MIGRATIONS};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sqlite-037")
        .join(name)
}

/// Kopiert die Fixture (mit allen Begleitdateien) als `meetings.db` in ein frisches Verzeichnis.
fn kopie(dir: &tempfile::TempDir, stamm: &str) -> PathBuf {
    let ziel = dir.path().join("meetings.db");
    std::fs::copy(fixture(&format!("{stamm}.db")), &ziel).unwrap();
    let wal = fixture(&format!("{stamm}.db-wal"));
    if wal.exists() {
        std::fs::copy(wal, dir.path().join("meetings.db-wal")).unwrap();
    }
    ziel
}

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn ids(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT id FROM meetings ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn gesund(conn: &Connection) {
    let ok: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
    let fk: i64 = conn
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |_| Ok(()))
        .unwrap()
        .count() as i64;
    assert_eq!(fk, 0, "Fremdschluessel verletzt");
}

fn sqlite_ab_3_54(conn: &Connection) {
    let v: String = conn
        .query_row("select sqlite_version()", [], |r| r.get(0))
        .unwrap();
    let t: Vec<u32> = v.split('.').map(|x| x.parse().unwrap_or(0)).collect();
    assert!(
        (t[0], t[1]) >= (3, 54),
        "SQLite {v} ist von RUSTSEC-2026-0459 betroffen"
    );
}

/// Gemeinsamer Ablauf: oeffnen, lesen, zweiter Start, schreiben, wieder oeffnen.
fn rundlauf(stamm: &str, erwartet: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    let path = kopie(&dir, stamm);
    {
        let store = MeetingStore::open_at(&path).unwrap();
        let conn = store.get_connection().unwrap();
        sqlite_ab_3_54(&conn);
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
        assert_eq!(ids(&conn), erwartet, "Altdaten lesbar");
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM integration_grants WHERE integration_id = 'ordner1'"
            ),
            1
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM audit_log"), 1);
        gesund(&conn);
    }
    // Zweiter Start: `to_latest` laeuft erneut und aendert nichts. Die Fixtures stehen schon
    // auf MIGRATIONS.len(); ein Upgrade aelterer Schemastaende pruefen migration_chain und
    // agent_bridge::schema.
    {
        let store = MeetingStore::open_at(&path).unwrap();
        let conn = store.get_connection().unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
        assert_eq!(ids(&conn), erwartet);
        conn.execute(
            "INSERT INTO meetings (id, title, status, source, created_at, updated_at)
               VALUES ('M9', 'Nach dem Upgrade', 'ready', 'live', 1790000200, 1790000200)",
            [],
        )
        .unwrap();
    }
    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    let mut mit_neu: Vec<String> = erwartet.iter().map(|s| s.to_string()).collect();
    mit_neu.push("M9".into());
    assert_eq!(ids(&conn), mit_neu, "geschrieben und wieder geoeffnet");
    assert_eq!(
        conn.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    gesund(&conn);
}

#[test]
fn fixtures_stammen_aus_rusqlite_037() {
    let herkunft = std::fs::read_to_string(fixture("ERZEUGT_MIT.txt")).unwrap();
    assert!(
        herkunft.starts_with("rusqlite 0.37.0, SQLite 3.5"),
        "{herkunft}"
    );
    // Die Fixtures sind unter 0.37 auf dem damaligen Stand der Kette entstanden. Spaetere,
    // angehaengte Schritte (Besprechungsplanung P3: Index 16 und 17) wendet `open_at` unter 0.41 auf
    // die alte Datei an: genau der Upgradepfad, den `rundlauf` prueft (`user_version == MIGRATIONS.len()`,
    // `integrity_check`, `foreign_key_check`). Die Angabe darf also hinter der Kette zurueckbleiben,
    // aber nie vor ihr liegen (sonst waere sie nicht echt).
    let stand: usize = herkunft
        .split("MIGRATIONS.len() = ")
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("Stand der Kette fehlt in ERZEUGT_MIT.txt: {herkunft}"));
    assert!(
        (16..=MIGRATIONS.len()).contains(&stand),
        "Fixture passt nicht zur Migrationskette: {herkunft}"
    );
}

#[test]
fn checkpointeter_altbestand_oeffnet_unter_0_41() {
    rundlauf("meetings-037", &["M1"]);
}

#[test]
fn nicht_checkpointete_wal_daten_aus_0_37_bleiben_erhalten() {
    // Gegenprobe: die Hauptdatei allein kennt M1 und M2 nicht, sie liegen nur im -wal.
    let ohne_wal = tempfile::tempdir().unwrap();
    let p = ohne_wal.path().join("meetings.db");
    std::fs::copy(fixture("meetings-037-wal.db"), &p).unwrap();
    assert!(
        ids(&Connection::open(&p).unwrap()).is_empty(),
        "Fixture ist schon checkpointet"
    );
    rundlauf("meetings-037-wal", &["M1", "M2"]);
}
