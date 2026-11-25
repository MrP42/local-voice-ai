//! Fixrunde zu lva#738 (P5), Befund HOCH 1: die Migrationsreihenfolge nach dem Zusammenfuehren mit
//! lva#806 (Hoerbuch) und lva#858 (Terminfindung TF2). P5 (die Protokolltabelle der Planung) stand
//! erst als Index 21 (`user_version` 22), dann hinter dem Hoerbuch als Index 23 (`user_version`
//! 24); main belegt 21 und 22 fuer das Hoerbuch, TF2 belegt 23, und P5 steht jetzt dahinter
//! (`P5_INDEX`). Eine Datenbank aus einem frueheren Build hat die Protokolltabelle, aber nicht die
//! Schritte, die seither VOR P5 eingefuegt wurden: ohne Angleichung wuerde `to_latest` sie nie
//! ausfuehren (das Hoerbuch liefe gegen fehlende Tabellen). Geprueft wird die Angleichung
//! (`store::adopt_p5_first_database`) mit befuellten Altdaten, die Wiederholung, der Abbruch, der
//! volle Datentraeger und der gleichzeitige Start. Nur synthetische Daten.
//!
//! Solange die Terminfindung nicht auf main ist, steht an Index 23 ein No-op-Platzhalter; die
//! Tests koppeln nicht an dessen Inhalt: sie rechnen mit `P5_INDEX` und `MIGRATIONS.len()`, und
//! `h_ein_weiterer_schritt_vor_p5_wird_mitgeholt` prueft mit einem echten Probeschritt, dass die
//! Angleichung jeden Schritt zwischen alter und neuer Stelle von P5 ausfuehrt.

use rusqlite::types::Value;
use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};

use super::store::{adopt_p5_first_database, adopt_p5_in, MeetingStore, MIGRATIONS, P5_INDEX};
use crate::managers::planning::protokoll::PROTOKOLL_MIGRATION;

/// Index des ersten Hoerbuch-Schritts (`audiobook::MIGRATION`); der Claim-Schritt folgt.
const HOERBUCH_INDEX: usize = 21;
/// P5 als Index 21: ein Build von vor dem Hoerbuch (`user_version` 22).
const P5_VOR_HOERBUCH: usize = HOERBUCH_INDEX;
/// P5 als Index 23: ein Build mit dem Hoerbuch, aber von vor der Terminfindung (`user_version` 24).
const P5_VOR_TERMINFINDUNG: usize = HOERBUCH_INDEX + 2;
/// Die Stellen, an denen P5 in einem frueheren Build stand.
const ALTE_P5_STELLEN: [usize; 2] = [P5_VOR_HOERBUCH, P5_VOR_TERMINFINDUNG];

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

/// Inhalt einer Abfrage als Text, eine Zeile je Datensatz.
fn dump(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
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

/// Alles, was eine Angleichung nicht veraendern darf, wenn sie nichts tut oder scheitert: das
/// Schema (Name, Art, SQL), die `user_version` und die Daten der beteiligten Tabellen.
fn zustand(conn: &Connection) -> Vec<String> {
    let mut out = vec![format!("user_version={}", user_version(conn))];
    out.extend(dump(
        conn,
        "SELECT type, name, sql FROM sqlite_master ORDER BY type, name",
    ));
    for table in [
        "meetings",
        "import_part",
        "meeting_plans",
        "meeting_plan_protocols",
        "meeting_plan_consents",
    ] {
        out.push(format!("== {table}"));
        out.extend(dump(conn, &format!("SELECT * FROM {table} ORDER BY rowid")));
    }
    for table in ["audiobook_control", "audiobook_chunk", "audiobook_adoption"] {
        if has_table(conn, table) {
            out.push(format!("== {table}"));
            out.extend(dump(conn, &format!("SELECT * FROM {table} ORDER BY rowid")));
        }
    }
    out
}

/// Altdaten: eine Besprechung mit Planung und Einwilligung, ein Hoerbuch-Teil in der
/// Warteschlange (kommt aus der Zeit vor dem Hoerbuch-Schritt) und eine normale Besprechung.
const ALTDATEN: &str = "
INSERT INTO meetings (id, title, status, source, created_at, updated_at)
    VALUES ('M1', 'Abstimmung', 'ready', 'live', 1790000000, 1790000000),
           ('B1', 'Hoerbuch synthetisch', 'queued', 'import', 1790000000, 1790000000);
INSERT INTO import_part (meeting_id, kind, range_start_ms, range_end_ms)
    VALUES ('B1', 'audiobook', 1000, 2000);
INSERT INTO meeting_plans (id, meeting_id, title, goal_type, goal_text, duration_min, status,
                           created_at, updated_at)
    VALUES ('plan-1', 'M1', 'Abstimmung Posteingang', 'entscheiden', 'Wir entscheiden',
            25, 'durchgefuehrt', 1790000000, 1790000000);
INSERT INTO meeting_plan_consents (plan_id, person, state, channel, recorded_at)
    VALUES ('plan-1', 'anna@example.com', 'zugestimmt', 'einladung', 1790000000);
";

/// Die Protokollzeile der Planung (nur P5-Stand).
const PROTOKOLL_ZEILE: &str = "
INSERT INTO meeting_plan_protocols (plan_id, template_id, goal_reached, goal_reached_at,
                                    approved_document_id, approved_hash, approved_at, updated_at)
    VALUES ('plan-1', 'builtin:allgemein', 'ja', 1790000100, 'doc-1', 'abc123', 1790000200,
            1790000300);
";

/// Eine Datenbank mit den ersten `schritte` Migrationen der heutigen Kette und Altdaten.
fn stand(dir: &tempfile::TempDir, schritte: usize) -> std::path::PathBuf {
    let path = dir.path().join("meetings.db");
    let mut conn = Connection::open(&path).unwrap();
    Migrations::new(MIGRATIONS[..schritte].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    assert_eq!(user_version(&conn), schritte as i64);
    conn.execute_batch(ALTDATEN).unwrap();
    path
}

/// Die Datenbank eines frueheren Builds: die ersten `p5_index` Schritte der heutigen Kette, danach
/// P5 als Schritt `p5_index` (`user_version` `p5_index + 1`), mit Altdaten und Protokollzeile.
fn alter_p5_stand(dir: &tempfile::TempDir, p5_index: usize) -> std::path::PathBuf {
    let path = stand(dir, p5_index);
    let conn = Connection::open(&path).unwrap();
    assert!(!has_table(&conn, "meeting_plan_protocols"));
    conn.execute_batch(PROTOKOLL_MIGRATION).unwrap();
    conn.execute_batch(PROTOKOLL_ZEILE).unwrap();
    conn.pragma_update(None, "user_version", p5_index as i64 + 1)
        .unwrap();
    path
}

fn hoerbuch_tabellen_da(conn: &Connection) {
    for table in ["audiobook_control", "audiobook_chunk", "audiobook_adoption"] {
        assert!(has_table(conn, table), "{table} fehlt");
    }
    let trigger: i64 = count(
        conn,
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'audiobook_part_%'",
    );
    assert_eq!(trigger, 2, "beide Hoerbuch-Trigger");
}

#[test]
fn p5_index_zeigt_auf_die_p5_migration() {
    // Haelt `HOERBUCH_INDEX` und `store::P5_INDEX` an der Kette fest: wer einen Schritt vor P5
    // einfuegt (TF2, ein weiterer), muss `P5_INDEX` anpassen und scheitert sonst hier.
    assert!(
        P5_INDEX >= HOERBUCH_INDEX + 2,
        "P5 steht hinter dem Hoerbuch"
    );
    assert!(P5_INDEX < MIGRATIONS.len());
    let dir = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(dir.path().join("meetings.db")).unwrap();
    let bis = |conn: &mut Connection, n: usize| {
        Migrations::new(MIGRATIONS[..n].to_vec())
            .to_latest(conn)
            .unwrap()
    };
    bis(&mut conn, HOERBUCH_INDEX);
    assert!(!has_table(&conn, "audiobook_control"), "vor dem Hoerbuch");
    bis(&mut conn, HOERBUCH_INDEX + 1);
    assert!(
        has_table(&conn, "audiobook_control"),
        "Index 21 ist Hoerbuch"
    );
    assert!(!has_table(&conn, "audiobook_adoption"));
    bis(&mut conn, HOERBUCH_INDEX + 2);
    assert!(
        has_table(&conn, "audiobook_adoption"),
        "Index 22 ist Hoerbuch"
    );
    bis(&mut conn, P5_INDEX);
    assert!(
        !has_table(&conn, "meeting_plan_protocols"),
        "P5 ist nicht Teil der ersten P5_INDEX Schritte"
    );
    bis(&mut conn, P5_INDEX + 1);
    assert!(
        has_table(&conn, "meeting_plan_protocols"),
        "Index P5_INDEX ist P5"
    );
    assert_eq!(user_version(&conn), P5_INDEX as i64 + 1);
}

/// Gemeinsame Pruefung fuer einen alten P5-Stand nach dem Start: alles aktuell, Hoerbuch da,
/// Protokollzeilen und Planungsdaten unveraendert, Sicherung des alten Standes vorhanden.
fn pruefe_angeglichen(dir: &tempfile::TempDir, path: &std::path::Path, p5_index: usize) {
    let alte_version = p5_index as i64 + 1;
    let vorher = {
        let conn = Connection::open(path).unwrap();
        assert_eq!(user_version(&conn), alte_version);
        dump(
            &conn,
            "SELECT * FROM meeting_plan_protocols ORDER BY plan_id",
        )
    };
    assert_eq!(vorher.len(), 1);

    let store = MeetingStore::open_at(path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    hoerbuch_tabellen_da(&conn);
    assert_eq!(
        dump(
            &conn,
            "SELECT * FROM meeting_plan_protocols ORDER BY plan_id"
        ),
        vorher,
        "Protokollzeilen unveraendert"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM meeting_plans"), 1);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM meeting_plan_consents"),
        1
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM meetings"), 2);
    drop(conn);
    drop(store);
    // Die Sicherung des Standes vor der Angleichung ist da und kennt die Protokolltabelle.
    let bak = dir.path().join(format!("meetings.db.bak-v{alte_version}"));
    assert!(bak.exists(), "Sicherung vor dem Nachziehen");
    let alt = Connection::open(&bak).unwrap();
    assert_eq!(user_version(&alt), alte_version);
    assert!(has_table(&alt, "meeting_plan_protocols"));
}

#[test]
fn a_ein_alter_p5_stand_vor_dem_hoerbuch_bekommt_es_und_behaelt_die_protokolle() {
    let dir = tempfile::tempdir().unwrap();
    let path = alter_p5_stand(&dir, P5_VOR_HOERBUCH);
    {
        let conn = Connection::open(&path).unwrap();
        assert!(!has_table(&conn, "audiobook_control"), "vor dem Hoerbuch");
    }
    pruefe_angeglichen(&dir, &path, P5_VOR_HOERBUCH);

    let store = MeetingStore::open_at(&path).unwrap();
    // Der Hoerbuch-Teil aus der Zeit davor bekommt seine Steuerzeile (Rueckfuellung des Schritts).
    let ctl = store.audiobook_control("B1").unwrap().unwrap();
    assert_eq!(
        (ctl.desired_state.as_str(), ctl.generation, ctl.next_sample),
        ("running", 0, None)
    );
    // Das Hoerbuch laeuft auf den nachgezogenen Tabellen: Pause, Claim, Freigabe.
    store
        .audiobook_set_state(&["B1".to_string()], "paused", true)
        .unwrap();
    assert_eq!(
        store
            .audiobook_control("B1")
            .unwrap()
            .unwrap()
            .desired_state,
        "paused"
    );
}

#[test]
fn a2_ein_alter_p5_stand_vor_der_terminfindung_behaelt_hoerbuch_und_protokolle() {
    // P5 als Index 23, `user_version` 24: das Hoerbuch ist da (mit Pause und Blockzeiger), nur die
    // Schritte zwischen dieser Stelle und `P5_INDEX` fehlen.
    let dir = tempfile::tempdir().unwrap();
    let path = alter_p5_stand(&dir, P5_VOR_TERMINFINDUNG);
    {
        let conn = Connection::open(&path).unwrap();
        hoerbuch_tabellen_da(&conn);
        conn.execute(
            "UPDATE audiobook_control SET desired_state = 'paused', generation = 3,
             next_sample = 160 WHERE meeting_id = 'B1'",
            [],
        )
        .unwrap();
    }
    pruefe_angeglichen(&dir, &path, P5_VOR_TERMINFINDUNG);

    let store = MeetingStore::open_at(&path).unwrap();
    let ctl = store.audiobook_control("B1").unwrap().unwrap();
    assert_eq!(
        (ctl.desired_state.as_str(), ctl.generation, ctl.next_sample),
        ("paused", 3, Some(160)),
        "Pause und Blockzeiger bleiben"
    );
}

#[test]
fn b_der_regulaere_main_stand_22_wird_ohne_angleichung_aktuell() {
    // `MIGRATIONS[..22]`: main nach lva#806 P1a (Hoerbuch-Steuerung, noch ohne Claim-Tabelle).
    let dir = tempfile::tempdir().unwrap();
    let path = stand(&dir, HOERBUCH_INDEX + 1);
    {
        let conn = Connection::open(&path).unwrap();
        assert!(has_table(&conn, "audiobook_control"));
        assert!(!has_table(&conn, "audiobook_adoption"));
        conn.execute(
            "UPDATE audiobook_control SET desired_state = 'paused', generation = 3,
             next_sample = 160 WHERE meeting_id = 'B1'",
            [],
        )
        .unwrap();
    }
    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    hoerbuch_tabellen_da(&conn);
    assert!(has_table(&conn, "meeting_plan_protocols"), "P5 nachgezogen");
    let ctl = store.audiobook_control("B1").unwrap().unwrap();
    assert_eq!(
        (ctl.desired_state.as_str(), ctl.generation, ctl.next_sample),
        ("paused", 3, Some(160)),
        "Pause und Blockzeiger aus main bleiben"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM meeting_plans"), 1);
}

#[test]
fn b2_der_volle_main_stand_vor_p5_bekommt_nur_p5() {
    let dir = tempfile::tempdir().unwrap();
    let path = stand(&dir, P5_INDEX);
    {
        let conn = Connection::open(&path).unwrap();
        assert!(has_table(&conn, "audiobook_adoption"));
        assert!(!has_table(&conn, "meeting_plan_protocols"));
    }
    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    assert!(has_table(&conn, "meeting_plan_protocols"));
    assert!(store.audiobook_control("B1").unwrap().is_some());
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM meeting_plan_protocols"),
        0
    );
}

#[test]
fn c_aeltere_staende_laufen_durch_alle_schritte() {
    for schritte in [20, HOERBUCH_INDEX] {
        let dir = tempfile::tempdir().unwrap();
        let path = stand(&dir, schritte);
        let store = MeetingStore::open_at(&path).unwrap();
        let conn = store.get_connection().unwrap();
        assert_eq!(
            user_version(&conn),
            MIGRATIONS.len() as i64,
            "Stand {schritte}"
        );
        hoerbuch_tabellen_da(&conn);
        assert!(
            has_table(&conn, "meeting_plan_protocols"),
            "Stand {schritte}"
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM meeting_plans"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM meetings"), 2);
        let ctl = store.audiobook_control("B1").unwrap().unwrap();
        assert_eq!(ctl.desired_state, "running", "Stand {schritte}");
    }
}

#[test]
fn d_der_zweite_start_aendert_nichts() {
    for p5_index in ALTE_P5_STELLEN {
        let dir = tempfile::tempdir().unwrap();
        let path = alter_p5_stand(&dir, p5_index);
        drop(MeetingStore::open_at(&path).unwrap());
        let nach_dem_ersten = {
            let conn = Connection::open(&path).unwrap();
            zustand(&conn)
        };
        drop(MeetingStore::open_at(&path).unwrap());
        let nach_dem_zweiten = {
            let conn = Connection::open(&path).unwrap();
            zustand(&conn)
        };
        assert_eq!(nach_dem_zweiten, nach_dem_ersten, "P5 als Index {p5_index}");
        assert_eq!(
            nach_dem_zweiten[0],
            format!("user_version={}", MIGRATIONS.len()),
            "P5 als Index {p5_index}"
        );
    }
}

#[test]
fn e_ein_fehler_beim_nachziehen_laesst_die_datenbank_unveraendert() {
    let dir = tempfile::tempdir().unwrap();
    let path = alter_p5_stand(&dir, P5_VOR_HOERBUCH);
    let alte_version = P5_VOR_HOERBUCH as i64 + 1;
    // Ein hinderliches Objekt: `audiobook_chunk` gibt es schon, der Schritt scheitert NACH
    // `CREATE TABLE audiobook_control`. Beides haengt in EINER Transaktion.
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE audiobook_chunk (hindernis TEXT);")
            .unwrap();
    }
    let vorher = {
        let conn = Connection::open(&path).unwrap();
        zustand(&conn)
    };
    // Der Start scheitert laut: weiterzumachen haette `to_latest` auf der alten Zaehlung laufen
    // lassen und die Hoerbuch-Tabellen fuer immer uebersprungen (stiller Schaden).
    let fehler = match MeetingStore::open_at(&path) {
        Ok(_) => panic!("ein gescheiterter Nachzug darf den Start nicht durchlassen"),
        Err(e) => e,
    };
    assert!(
        fehler.to_string().contains("audiobook_chunk"),
        "der Fehler nennt die Ursache: {fehler}"
    );
    let nachher = {
        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn), alte_version, "Version unveraendert");
        assert!(
            !has_table(&conn, "audiobook_control"),
            "nichts halb angelegt"
        );
        assert!(!has_table(&conn, "audiobook_adoption"));
        zustand(&conn)
    };
    assert_eq!(nachher, vorher, "Datenbank unveraendert");

    // Ist das Hindernis weg, klappt der naechste Start und alles ist da.
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE audiobook_chunk;").unwrap();
    }
    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    hoerbuch_tabellen_da(&conn);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM meeting_plan_protocols"),
        1
    );
}

#[test]
fn e2_eine_volle_datenbank_rollt_das_nachziehen_zurueck() {
    // SQLITE_FULL statt eines erfundenen Fehlers: die Datenbank darf nicht mehr wachsen.
    let dir = tempfile::tempdir().unwrap();
    let path = alter_p5_stand(&dir, P5_VOR_HOERBUCH);
    let vorher = {
        let conn = Connection::open(&path).unwrap();
        zustand(&conn)
    };
    let mut conn = Connection::open(&path).unwrap();
    let seiten: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    conn.pragma_update(None, "max_page_count", seiten).unwrap();
    let fehler = adopt_p5_first_database(&mut conn).expect_err("kein Platz fuer neue Tabellen");
    assert!(
        fehler.to_string().to_lowercase().contains("full"),
        "SQLITE_FULL erwartet: {fehler}"
    );
    drop(conn);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(zustand(&conn), vorher, "Datenbank unveraendert");
}

#[test]
fn f_gleichzeitige_starts_ziehen_genau_einmal_nach() {
    for p5_index in ALTE_P5_STELLEN {
        let dir = tempfile::tempdir().unwrap();
        let path = alter_p5_stand(&dir, p5_index);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    MeetingStore::open_at(&path)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                })
            })
            .collect();
        for t in threads {
            t.join()
                .unwrap()
                .unwrap_or_else(|e| panic!("jeder Start gelingt (P5 als Index {p5_index}): {e}"));
        }
        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
        hoerbuch_tabellen_da(&conn);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM meeting_plan_protocols"),
            1
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM audiobook_control"), 1);
    }
}

#[test]
fn g_die_angeglichene_datenbank_hat_dasselbe_schema_wie_eine_frische() {
    // Sicherheitsnetz: holt die Angleichung einen Schritt nicht mit (ein neuer Schritt vor P5,
    // ein vergessenes `P5_INDEX`), unterscheidet sich das Schema von dem einer frischen Datenbank.
    let frisch = {
        let dir = tempfile::tempdir().unwrap();
        drop(MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap());
        let conn = Connection::open(dir.path().join("meetings.db")).unwrap();
        zustand_ohne_daten(&conn)
    };
    assert_eq!(frisch[0], format!("user_version={}", MIGRATIONS.len()));
    for p5_index in ALTE_P5_STELLEN {
        let dir = tempfile::tempdir().unwrap();
        let path = alter_p5_stand(&dir, p5_index);
        drop(MeetingStore::open_at(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        assert_eq!(zustand_ohne_daten(&conn), frisch, "P5 als Index {p5_index}");
    }
}

#[test]
fn h_ein_weiterer_schritt_vor_p5_wird_mitgeholt() {
    // Eine Kette mit einem echten Probeschritt direkt vor P5 (so setzt ihn die Terminfindung ein,
    // sobald sie auf main ist): P5 rueckt um eins weiter. Jede alte Stelle von P5 muss ihn
    // mitbekommen, samt den Hoerbuch-Schritten dazwischen.
    let mut kette = MIGRATIONS[..P5_INDEX].to_vec();
    kette.push(M::up(
        "CREATE TABLE tf_probe (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
         INSERT INTO tf_probe (name) VALUES ('angelegt');",
    ));
    kette.push(MIGRATIONS[P5_INDEX].clone());
    let neuer_p5_index = P5_INDEX + 1;
    for p5_index in [P5_VOR_HOERBUCH, P5_VOR_TERMINFINDUNG, P5_INDEX] {
        let dir = tempfile::tempdir().unwrap();
        let path = alter_p5_stand(&dir, p5_index);
        let mut conn = Connection::open(&path).unwrap();
        let protokolle = dump(&conn, "SELECT * FROM meeting_plan_protocols");
        assert!(
            adopt_p5_in(&mut conn, &kette, neuer_p5_index).unwrap(),
            "P5 als Index {p5_index}"
        );
        assert_eq!(
            user_version(&conn),
            kette.len() as i64,
            "P5 als Index {p5_index}"
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tf_probe"), 1);
        hoerbuch_tabellen_da(&conn);
        assert_eq!(
            dump(&conn, "SELECT * FROM meeting_plan_protocols"),
            protokolle,
            "P5 laeuft nicht noch einmal, die Zeilen bleiben"
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM meeting_plans"), 1);
        assert!(
            !adopt_p5_in(&mut conn, &kette, neuer_p5_index).unwrap(),
            "der zweite Lauf findet nichts mehr"
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tf_probe"), 1);
    }
}

#[test]
fn die_angleichung_fasst_keine_andere_datenbank_an() {
    // Nur "Version im Bereich UND Protokolltabelle da" ist ein alter P5-Stand. Jeder Stand der
    // heutigen Kette (auch der letzte, mit P5) bleibt unberuehrt.
    let dir = tempfile::tempdir().unwrap();
    for schritte in 20..=MIGRATIONS.len() {
        let path = dir.path().join(format!("m{schritte}.db"));
        let mut conn = Connection::open(&path).unwrap();
        Migrations::new(MIGRATIONS[..schritte].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        let vorher = zustand_ohne_daten(&conn);
        assert!(
            !adopt_p5_first_database(&mut conn).unwrap(),
            "Stand {schritte}"
        );
        assert_eq!(zustand_ohne_daten(&conn), vorher, "Stand {schritte}");
    }
}

/// Wie [`zustand`], aber nur Schema und Version (die Tabellen der Daten gibt es nicht in jedem Stand).
fn zustand_ohne_daten(conn: &Connection) -> Vec<String> {
    let mut out = vec![format!("user_version={}", user_version(conn))];
    out.extend(dump(
        conn,
        "SELECT type, name, sql FROM sqlite_master ORDER BY type, name",
    ));
    out
}
