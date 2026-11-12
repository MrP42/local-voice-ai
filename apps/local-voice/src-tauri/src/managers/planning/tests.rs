//! Tests der Besprechungsplanung in der App (P3): Migrationskette, Fremdschluessel, Befehle und
//! der Abgleich-Lauf von Adapter C mit synthetischen Terminen (Domains `example.*`).
//!
//! Jeder Test legt eine eigene Datenbank in einem Temp-Verzeichnis an; produktive Daten werden nie
//! beruehrt.

use besprechungsplanung::consent::StartModus;
use besprechungsplanung::invitation::{Einladung, Gestaltung};
use besprechungsplanung::model::{
    AdapterKind, AgendaItem, Consent, ConsentChannel, ConsentState, GoalType, Hints, Plan,
    PlanInput, PlanLink, PlanStatus, Roles, SyncState,
};
use besprechungsplanung::store as plan_store;
use besprechungsplanung::sync::Ereignis;
use rusqlite::types::Value;
use rusqlite::Connection;
use rusqlite_migration::Migrations;

use super::adapter_c::{self, abgleich_lauf, klartext, termin_waehlen, AbgleichBericht};
use super::*;
use crate::managers::calendar::model::{event_key, Attendee, CalEvent, CalendarKind};
use crate::managers::meetings::store::{MeetingStore, SyncMeta, MIGRATIONS};

const NOW: i64 = 1_790_000_000_000;
const H: i64 = 3_600_000;
const D: i64 = 24 * H;
const SRC: &str = "src1";

struct Fix {
    _dir: tempfile::TempDir,
    store: MeetingStore,
}

fn fix() -> Fix {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    store
        .calendar_source_add(SRC, CalendarKind::Ics, "Kalender", None, NOW)
        .unwrap();
    Fix { _dir: dir, store }
}

fn eingabe() -> PlanInput {
    PlanInput {
        title: "Abstimmung Posteingang".into(),
        goal_type: GoalType::Entscheiden,
        goal_text: "Wir entscheiden, ob der Pilot im Januar startet.".into(),
        expected_result: Some("Entscheidung mit Termin".into()),
        duration_min: 25,
        classification: "intern".into(),
        agenda: vec![AgendaItem {
            minutes: 25,
            topic: "Optionen".into(),
            owner: None,
        }],
        roles: Roles::default(),
        materials: vec![],
        hints: Hints::default(),
        transcription_planned: true,
        minutes_template_id: None,
    }
}

/// Planung im Status `uebergeben` samt Einladungstext.
fn uebergeben(s: &MeetingStore) -> (Plan, Einladung) {
    let p = plan_anlegen(s, &eingabe(), NOW).unwrap();
    let e = einladung_erzeugen(s, &p.id, &Gestaltung::default(), NOW).unwrap();
    (plan_laden(s, &p.id).unwrap().unwrap(), e)
}

/// So steht der Block im Termin: eigener Text davor, Teams-Hinweis dahinter.
fn beschreibung(e: &Einladung) -> String {
    format!("Hallo zusammen,\n\n{}\n\nTeilnahme per Teams.", e.text)
}

fn person(email: &str) -> Attendee {
    Attendee {
        email: Some(email.into()),
        name: None,
        organizer: false,
        is_self: false,
        partstat: None,
    }
}

fn termin(uid: &str, start: i64, beschreibung: Option<String>) -> CalEvent {
    CalEvent {
        key: event_key(SRC, uid, start),
        source_id: SRC.into(),
        uid: uid.into(),
        title: "Abstimmung".into(),
        starts_at: start,
        ends_at: start + H,
        all_day: false,
        cancelled: false,
        location: None,
        join_url: None,
        description: beschreibung,
        instance_start: None,
        attendees: vec![],
    }
}

fn instanz(uid: &str, start: i64, original: i64, beschreibung: Option<String>) -> CalEvent {
    CalEvent {
        instance_start: Some(original),
        ..termin(uid, start, beschreibung)
    }
}

fn cache(s: &MeetingStore, events: &[CalEvent]) {
    s.calendar_replace_events(
        SRC,
        events,
        &SyncMeta {
            has_attendee_data: true,
            etag: None,
            last_modified: None,
            now_ms: NOW,
        },
    )
    .unwrap();
}

fn lauf(s: &MeetingStore) -> AbgleichBericht {
    abgleich_lauf(s, NOW).unwrap()
}

fn status(s: &MeetingStore, id: &str) -> PlanStatus {
    plan_laden(s, id).unwrap().unwrap().status
}

fn link(s: &MeetingStore, id: &str) -> PlanLink {
    verknuepfung_laden(s, id).unwrap().unwrap()
}

fn arten(b: &AbgleichBericht) -> Vec<Ereignis> {
    b.ereignisse.iter().map(|e| e.ereignis.clone()).collect()
}

fn personen(s: &MeetingStore, id: &str) -> Vec<(String, ConsentState)> {
    einwilligungen(s, id)
        .unwrap()
        .into_iter()
        .map(|c| (c.person, c.state))
        .collect()
}

/// Eine verknuepfte Planung: Termin `uid` bei `NOW + 2 Tage`, Marke im Text.
fn verknuepft(f: &Fix, uid: &str) -> (Plan, CalEvent) {
    let (p, e) = uebergeben(&f.store);
    let t = termin(uid, NOW + 2 * D, Some(beschreibung(&e)));
    cache(&f.store, std::slice::from_ref(&t));
    assert_eq!(lauf(&f.store).geaendert, 1);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);
    (p, t)
}

// ---------------------------------------------------------------------------
// Migrationskette
// ---------------------------------------------------------------------------

/// Die Kette haengt jeden Schritt des Crates an (Index 16 ff.). Waechst das Crate um einen Schritt,
/// muss er in `store.rs` `MIGRATIONS` ergaenzt werden; dieser Test erinnert daran.
#[test]
fn planning_steps_follow_the_crate() {
    assert_eq!(
        besprechungsplanung::store::MIGRATIONS.len(),
        1,
        "neuer Schritt im Crate: in managers/meetings/store.rs MIGRATIONS hinter dem letzten \
         Planungsschritt anhaengen und diese Zahl anheben"
    );
    assert!(
        MIGRATIONS.len() >= 18,
        "Index 16 = Planung, 17 = Kalender-Instanz"
    );
}

fn dump(conn: &Connection, table: &str) -> Vec<String> {
    let mut st = conn
        .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
        .unwrap();
    let n = st.column_count();
    st.query_map([], |row| {
        let mut parts = Vec::new();
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

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

/// Eine Datenbank auf dem Stand VOR der Planung (Index 0 bis 15) mit Altdaten bekommt die
/// Planungstabellen und die Instanzspalte; jede Altzeile bleibt byte-gleich, nichts geht verloren.
#[test]
fn the_planning_migration_is_appended_and_keeps_old_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meetings.db");
    let alt = 16;
    {
        let mut conn = Connection::open(&path).unwrap();
        Migrations::new(MIGRATIONS[..alt].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        assert_eq!(user_version(&conn), alt as i64);
        assert!(!table_exists(&conn, "meeting_plans"));
        conn.execute(
            "INSERT INTO meetings (id, title, status, source, created_at, updated_at)
             VALUES ('m1', 'Altbestand', 'ready', 'live', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO calendar_sources (id, kind, label, created_at, updated_at)
             VALUES ('s1', 'ics', 'Kalender', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO calendar_events (key, source_id, uid, title, starts_at, ends_at, description,
                fetched_at)
             VALUES ('s1:u1:5', 's1', 'u1', 'Alt', 5, 6, 'Text', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO calendar_event_attendees (event_key, email, name)
             VALUES ('s1:u1:5', 'anna@example.com', '')",
            [],
        )
        .unwrap();
    }
    let vorher: Vec<(&str, Vec<String>)> = {
        let conn = Connection::open(&path).unwrap();
        ["meetings", "calendar_sources", "calendar_event_attendees"]
            .iter()
            .map(|t| (*t, dump(&conn, t)))
            .collect()
    };
    let spalten_alt = {
        let conn = Connection::open(&path).unwrap();
        dump(&conn, "calendar_events")
    };

    let s = MeetingStore::open_at(&path).unwrap();
    let conn = s.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    for t in [
        "meeting_plans",
        "meeting_plan_links",
        "meeting_plan_consents",
        "meeting_plan_consent_log",
    ] {
        assert!(table_exists(&conn, t), "{t}");
        assert_eq!(dump(&conn, t), Vec::<String>::new(), "{t} startet leer");
    }
    for (t, zeilen) in &vorher {
        assert_eq!(&dump(&conn, t), zeilen, "{t} bleibt unveraendert");
    }
    // calendar_events: dieselben Werte, dazu die neue Spalte (NULL).
    let nach = dump(&conn, "calendar_events");
    assert_eq!(nach.len(), spalten_alt.len());
    assert_eq!(nach[0], format!("{}|Null", spalten_alt[0]));
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_cal_events_uid'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(conn);

    // Zweiter Start: nichts mehr zu tun, nichts aendert sich.
    drop(s);
    let s2 = MeetingStore::open_at(&path).unwrap();
    let conn = s2.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    assert_eq!(dump(&conn, "calendar_events"), nach);
}

/// Der SQL-Text kommt aus dem Crate: die Tabellen entsprechen seinem Schema (Spalten je Tabelle).
#[test]
fn a_fresh_database_has_the_crate_schema() {
    let f = fix();
    let conn = f.store.get_connection().unwrap();
    let ref_conn = Connection::open_in_memory().unwrap();
    plan_store::migrieren(&ref_conn).unwrap();
    for t in [
        "meeting_plans",
        "meeting_plan_links",
        "meeting_plan_consents",
        "meeting_plan_consent_log",
    ] {
        let sql = |c: &Connection| -> String {
            c.query_row("SELECT sql FROM sqlite_master WHERE name = ?1", [t], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(sql(&conn), sql(&ref_conn), "{t}");
    }
}

// ---------------------------------------------------------------------------
// Fremdschluessel
// ---------------------------------------------------------------------------

/// Das gebuendelte SQLite hat die Vorgabe `foreign_keys = ON` (`SQLITE_DEFAULT_FOREIGN_KEYS=1` in
/// libsqlite3-sys): auch eine Verbindung ohne eigenen PRAGMA hat sie. `open_connection` setzt sie
/// trotzdem ausdruecklich, damit die Zusage nicht an der SQLite-Fassung haengt. Fuer die
/// bestehenden Tabellen ist das keine Aenderung.
#[test]
fn foreign_keys_are_on_by_default_and_on_every_store_connection() {
    let plain = Connection::open_in_memory().unwrap();
    let default: i64 = plain
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(default, 1, "gebuendeltes SQLite: Vorgabe an");

    let f = fix();
    let conn = verbindung(&f.store).unwrap();
    let an: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(an, 1);
    conn.pragma_update(None, "foreign_keys", false).unwrap();
    assert_eq!(
        fremdschluessel_pruefen(&conn).unwrap_err(),
        "foreign_keys_nicht_aktiv"
    );
}

/// Endgueltiges Loeschen einer Planung nimmt Verknuepfung, Einwilligungen und Verlauf mit
/// (`ON DELETE CASCADE`): Personendaten bleiben nicht liegen.
#[test]
fn deleting_a_plan_row_cascades_to_links_consents_and_log() {
    let f = fix();
    let (p, e) = uebergeben(&f.store);
    cache(
        &f.store,
        &[CalEvent {
            attendees: vec![person("anna@example.com")],
            ..termin("u1", NOW + D, Some(beschreibung(&e)))
        }],
    );
    lauf(&f.store);
    assert_eq!(personen(&f.store, &p.id).len(), 1);

    let conn = verbindung(&f.store).unwrap();
    let zaehlen = |t: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(zaehlen("meeting_plan_links"), 1);
    assert_eq!(zaehlen("meeting_plan_consents"), 1);
    assert_eq!(zaehlen("meeting_plan_consent_log"), 1);
    conn.execute("DELETE FROM meeting_plans WHERE id = ?1", [&p.id])
        .unwrap();
    assert_eq!(zaehlen("meeting_plan_links"), 0);
    assert_eq!(zaehlen("meeting_plan_consents"), 0);
    assert_eq!(zaehlen("meeting_plan_consent_log"), 0);
}

// ---------------------------------------------------------------------------
// Planung, Status, Einladung
// ---------------------------------------------------------------------------

#[test]
fn plans_can_be_created_changed_listed_and_deleted() {
    let f = fix();
    let p = plan_anlegen(&f.store, &eingabe(), NOW).unwrap();
    assert_eq!(p.status, PlanStatus::Entwurf);
    assert!(besprechungsplanung::store::gueltige_id(&p.id));

    let mut neu = eingabe();
    neu.title = "Geaendert".into();
    let q = plan_aendern(&f.store, &p.id, &neu, NOW + 1).unwrap();
    assert_eq!(q.title, "Geaendert");
    assert_eq!(plan_liste(&f.store).unwrap().len(), 1);

    let mut kaputt = eingabe();
    kaputt.title = "  ".into();
    assert_eq!(
        plan_anlegen(&f.store, &kaputt, NOW).unwrap_err(),
        "titel_leer"
    );
    assert_eq!(
        plan_aendern(&f.store, "gibt-es-nicht", &eingabe(), NOW).unwrap_err(),
        "plan_nicht_gefunden"
    );

    plan_loeschen(&f.store, &p.id, NOW + 2).unwrap();
    assert!(plan_laden(&f.store, &p.id).unwrap().is_none());
    assert!(plan_liste(&f.store).unwrap().is_empty());
    assert_eq!(
        plan_loeschen(&f.store, &p.id, NOW + 3).unwrap_err(),
        "plan_nicht_gefunden"
    );
}

#[test]
fn the_status_is_set_by_hand_only_where_the_calendar_does_not_decide() {
    let f = fix();
    let p = plan_anlegen(&f.store, &eingabe(), NOW).unwrap();
    for s in [
        PlanStatus::Verschoben,
        PlanStatus::Abgesagt,
        PlanStatus::Verknuepft,
    ] {
        assert_eq!(
            status_setzen(&f.store, &p.id, s, NOW).unwrap_err(),
            "status_nur_durch_abgleich"
        );
    }
    status_setzen(&f.store, &p.id, PlanStatus::Uebergeben, NOW).unwrap();
    assert_eq!(status(&f.store, &p.id), PlanStatus::Uebergeben);
    status_setzen(&f.store, &p.id, PlanStatus::Durchgefuehrt, NOW).unwrap();
    // Nach der Durchfuehrung nur vorwaerts.
    assert_eq!(
        status_setzen(&f.store, &p.id, PlanStatus::Uebergeben, NOW).unwrap_err(),
        "status_uebergang_ungueltig"
    );
    status_setzen(&f.store, &p.id, PlanStatus::Protokolliert, NOW).unwrap();
    assert_eq!(
        status_setzen(&f.store, &p.id, PlanStatus::Durchgefuehrt, NOW).unwrap_err(),
        "status_uebergang_ungueltig"
    );
    assert_eq!(
        status_setzen(&f.store, "gibt-es-nicht", PlanStatus::Entwurf, NOW).unwrap_err(),
        "plan_nicht_gefunden"
    );
}

/// Der Statuswechsel prueft im UPDATE, dass der Status noch der gelesene ist.
#[test]
fn a_status_change_checks_the_status_it_read() {
    let f = fix();
    let p = plan_anlegen(&f.store, &eingabe(), NOW).unwrap();
    let conn = verbindung(&f.store).unwrap();
    // Gelesen "uebergeben", tatsaechlich "entwurf": ein anderer hat es geaendert.
    assert_eq!(
        status_wechseln(
            &conn,
            &p.id,
            PlanStatus::Uebergeben,
            PlanStatus::Verknuepft,
            NOW
        )
        .unwrap_err(),
        "status_geaendert"
    );
    assert_eq!(status(&f.store, &p.id), PlanStatus::Entwurf);
    status_wechseln(
        &conn,
        &p.id,
        PlanStatus::Entwurf,
        PlanStatus::Uebergeben,
        NOW,
    )
    .unwrap();
    assert_eq!(status(&f.store, &p.id), PlanStatus::Uebergeben);
    // Ein abgeschlossener Status ist hier nie Ausgangspunkt.
    assert_eq!(
        status_wechseln(
            &conn,
            &p.id,
            PlanStatus::Durchgefuehrt,
            PlanStatus::Uebergeben,
            NOW
        )
        .unwrap_err(),
        "status_uebergang_ungueltig"
    );
}

#[test]
fn the_invitation_hands_the_plan_over_and_is_repeatable() {
    let f = fix();
    let p = plan_anlegen(&f.store, &eingabe(), NOW).unwrap();
    assert!(verknuepfung_laden(&f.store, &p.id).unwrap().is_none());
    let e = einladung_erzeugen(&f.store, &p.id, &Gestaltung::default(), NOW).unwrap();
    assert!(e.text.contains(&format!("lva-plan:{}", p.id)));
    assert_eq!(status(&f.store, &p.id), PlanStatus::Uebergeben);
    let l = link(&f.store, &p.id);
    assert_eq!(l.adapter, AdapterKind::IcsMarker);
    assert_eq!(l.body_hash.as_deref(), Some(e.block_hash.as_str()));
    assert!(!hat_termin(&l));

    // Wiederholt: dasselbe, und eine erkannte Verknuepfung bleibt.
    let e2 = einladung_erzeugen(&f.store, &p.id, &Gestaltung::default(), NOW + 1).unwrap();
    assert_eq!(e2, e);
    cache(&f.store, &[termin("u1", NOW + D, Some(beschreibung(&e)))]);
    lauf(&f.store);
    let gebunden = link(&f.store, &p.id);
    assert!(hat_termin(&gebunden));
    einladung_erzeugen(&f.store, &p.id, &Gestaltung::default(), NOW + 2).unwrap();
    assert_eq!(link(&f.store, &p.id), gebunden);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);

    assert_eq!(
        einladung_erzeugen(&f.store, "gibt-es-nicht", &Gestaltung::default(), NOW).unwrap_err(),
        "plan_nicht_gefunden"
    );
    status_setzen(&f.store, &p.id, PlanStatus::Durchgefuehrt, NOW).unwrap();
    assert_eq!(
        einladung_erzeugen(&f.store, &p.id, &Gestaltung::default(), NOW).unwrap_err(),
        "plan_abgeschlossen"
    );
}

// ---------------------------------------------------------------------------
// Adapter C: Erstverknuepfung
// ---------------------------------------------------------------------------

#[test]
fn a_calendar_event_with_the_marker_is_linked_and_its_attendees_become_open_consents() {
    let f = fix();
    let (p, e) = uebergeben(&f.store);
    let mut t = termin("u1", NOW + 2 * D, Some(beschreibung(&e)));
    t.location = Some("Raum 2.14".into());
    t.attendees = vec![
        person("anna@example.com"),
        person("Bert@Example.org"),
        Attendee {
            email: None,
            name: Some("Chris Ohne Adresse".into()),
            organizer: false,
            is_self: false,
            partstat: None,
        },
    ];
    cache(&f.store, &[t.clone()]);

    let b = lauf(&f.store);
    assert_eq!((b.geprueft, b.geaendert), (1, 1));
    assert_eq!(arten(&b), [Ereignis::Verknuepft]);
    assert!(b.fehler.is_empty() && b.mehrdeutig.is_empty());

    let l = link(&f.store, &p.id);
    assert_eq!(l.remote_id.as_deref(), Some("u1"));
    assert_eq!(l.ical_uid.as_deref(), Some("u1"));
    assert_eq!(l.source_id.as_deref(), Some(SRC));
    assert_eq!(l.starts_at, Some(t.starts_at));
    assert_eq!(l.ends_at, Some(t.ends_at));
    assert_eq!(l.location.as_deref(), Some("Raum 2.14"));
    assert_eq!(l.instanz, None);
    assert_eq!(l.sync_state, SyncState::Ok);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);
    // Nur Personen mit Adresse; ein Name allein ist keine Identitaet.
    assert_eq!(
        personen(&f.store, &p.id),
        [
            ("anna@example.com".to_string(), ConsentState::Offen),
            ("bert@example.org".to_string(), ConsentState::Offen)
        ]
    );

    // Ein zweiter Lauf aendert nichts und schreibt nichts.
    let vorher = plan_laden(&f.store, &p.id).unwrap().unwrap();
    let b2 = lauf(&f.store);
    assert_eq!((b2.geprueft, b2.geaendert), (1, 0));
    assert!(b2.ereignisse.is_empty());
    assert_eq!(plan_laden(&f.store, &p.id).unwrap().unwrap(), vorher);
    assert_eq!(link(&f.store, &p.id), l);
}

#[test]
fn without_the_marker_nothing_is_linked() {
    let f = fix();
    let (p, e) = uebergeben(&f.store);
    // Kein Block; Marke einer anderen Planung; Marke ausserhalb eines Blocks (nur die Zeile).
    cache(
        &f.store,
        &[
            termin("a", NOW + D, Some("Agenda folgt".into())),
            termin(
                "b",
                NOW + 2 * D,
                Some(beschreibung(&Einladung {
                    text: e.text.replace(&p.id, "ANDEREPLANUNG"),
                    ..e.clone()
                })),
            ),
            termin("c", NOW + 3 * D, Some(format!("lva-plan:{}", p.id))),
            termin("d", NOW + 4 * D, None),
        ],
    );
    let b = lauf(&f.store);
    assert_eq!((b.geprueft, b.geaendert, b.uebersprungen), (1, 0, 1));
    assert!(!hat_termin(&link(&f.store, &p.id)));
    assert_eq!(status(&f.store, &p.id), PlanStatus::Uebergeben);
}

#[test]
fn the_marker_is_read_from_html_descriptions_as_plain_text() {
    let f = fix();
    let (p, e) = uebergeben(&f.store);
    let html: String = e
        .text
        .lines()
        .map(|z| {
            format!(
                "<p>{}</p>",
                z.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            )
        })
        .collect();
    assert!(html.contains("<p>"), "Test-Aufbau");
    cache(
        &f.store,
        &[termin(
            "u1",
            NOW + D,
            Some(format!("<html><body>{html}</body></html>")),
        )],
    );
    assert_eq!(lauf(&f.store).geaendert, 1);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);
}

#[test]
fn only_html_looking_descriptions_are_converted() {
    // Gewoehnlicher Text mit < und > bleibt, wie er ist.
    let t = "Budget < 5 Mio und > 2 Mio\nlva-plan:ABC";
    assert_eq!(klartext(t), t);
    assert_eq!(
        klartext("<div>Zeile 1</div><div>lva-plan:ABC</div>"),
        "Zeile 1\nlva-plan:ABC"
    );
    // Zeichenreferenzen nur im HTML-Zweig: Dezimal, hexadezimal und benannt.
    assert_eq!(klartext("<p>lva&#45;plan&#58;ABC</p>"), "lva-plan:ABC");
    assert_eq!(klartext("<p>lva&#x2d;plan&#x3A;ABC</p>"), "lva-plan:ABC");
    assert_eq!(klartext("<p>lva&hyphen;plan&colon;ABC</p>"), "lva-plan:ABC");
    assert_eq!(klartext("<p>x&#99999999;y</p>"), "x&#99999999;y");
    assert_eq!(klartext("lva&#58;ABC"), "lva&#58;ABC");
}

#[test]
fn an_ambiguous_marker_links_nothing_until_the_user_chooses() {
    let f = fix();
    let (p, e) = uebergeben(&f.store);
    let text = beschreibung(&e);
    let a = termin("serie-a", NOW + D, Some(text.clone()));
    let b = termin("serie-b", NOW + 2 * D, Some(text.clone()));
    cache(&f.store, &[a.clone(), b.clone()]);

    let bericht = lauf(&f.store);
    assert_eq!(bericht.mehrdeutig, std::slice::from_ref(&p.id));
    assert_eq!(bericht.geaendert, 0);
    assert!(!hat_termin(&link(&f.store, &p.id)));
    assert_eq!(status(&f.store, &p.id), PlanStatus::Uebergeben);

    // Auswahl: nur ein Termin mit DIESER Marke, der keiner anderen Planung gehoert.
    assert_eq!(
        termin_waehlen(&f.store, &p.id, "gibt-es-nicht", NOW).unwrap_err(),
        "termin_nicht_gefunden"
    );
    let ohne = termin("ohne", NOW + 3 * D, Some("kein Block".into()));
    cache(&f.store, &[a.clone(), b.clone(), ohne.clone()]);
    assert_eq!(
        termin_waehlen(&f.store, &p.id, &ohne.key, NOW).unwrap_err(),
        "marke_fehlt"
    );
    let ereignisse = termin_waehlen(&f.store, &p.id, &b.key, NOW).unwrap();
    assert_eq!(ereignisse, [Ereignis::Verknuepft]);
    assert_eq!(link(&f.store, &p.id).remote_id.as_deref(), Some("serie-b"));
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);

    // Danach bleibt es bei dieser Wahl, auch wenn der andere Termin weiter die Marke traegt.
    let b2 = lauf(&f.store);
    assert!(b2.ereignisse.is_empty() && b2.mehrdeutig.is_empty());
    assert_eq!(link(&f.store, &p.id).remote_id.as_deref(), Some("serie-b"));

    // Ein Termin, den schon eine andere Planung hat, ist nicht waehlbar.
    let q = plan_anlegen(&f.store, &eingabe(), NOW).unwrap();
    let eq = einladung_erzeugen(&f.store, &q.id, &Gestaltung::default(), NOW).unwrap();
    cache(
        &f.store,
        &[
            a.clone(),
            termin("serie-b", NOW + 2 * D, Some(beschreibung(&eq))),
        ],
    );
    assert_eq!(
        termin_waehlen(&f.store, &q.id, &b.key, NOW).unwrap_err(),
        "termin_belegt"
    );
    // ... und die Erstverknuepfung ueber die Marke nimmt ihn auch nicht weg.
    lauf(&f.store);
    assert!(!hat_termin(&link(&f.store, &q.id)));
}

// ---------------------------------------------------------------------------
// Adapter C: Verschiebung, Absage, Marke weg, Termin weg
// ---------------------------------------------------------------------------

#[test]
fn a_moved_event_updates_the_plan_until_the_move_is_confirmed() {
    let f = fix();
    let (p, t) = verknuepft(&f, "u1");
    let (alt, neu) = (t.starts_at, t.starts_at + D);
    cache(&f.store, &[termin("u1", neu, t.description.clone())]);

    let b = lauf(&f.store);
    assert_eq!(
        arten(&b),
        [Ereignis::Verschoben {
            alt_beginn: Some(alt),
            neu_beginn: neu
        }]
    );
    let l = link(&f.store, &p.id);
    assert_eq!(l.starts_at, Some(neu));
    assert!(l.verschiebung_offen);
    assert_eq!(l.sync_state, SyncState::Verschoben);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verschoben);

    // Noch unbestaetigt: ein weiterer Lauf meldet nichts Neues.
    assert!(lauf(&f.store).ereignisse.is_empty());
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verschoben);

    let l = verschiebung_bestaetigen(&f.store, &p.id, NOW).unwrap();
    assert!(!l.verschiebung_offen);
    assert_eq!(l.sync_state, SyncState::Ok);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);
    assert!(lauf(&f.store).ereignisse.is_empty());
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);
}

#[test]
fn a_cancelled_event_marks_the_plan_cancelled_and_a_restore_is_recognised() {
    let f = fix();
    let (p, t) = verknuepft(&f, "u1");
    cache(
        &f.store,
        &[CalEvent {
            cancelled: true,
            ..t.clone()
        }],
    );
    assert_eq!(arten(&lauf(&f.store)), [Ereignis::Abgesagt]);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Abgesagt);
    assert!(link(&f.store, &p.id).abgesagt);

    cache(&f.store, &[t]);
    assert_eq!(arten(&lauf(&f.store)), [Ereignis::Wiederhergestellt]);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);
}

#[test]
fn a_removed_marker_orphans_the_link_and_a_returning_marker_relinks_it() {
    let f = fix();
    let (p, t) = verknuepft(&f, "u1");
    // Marke weg; zugleich kommt eine neue Teilnehmerin. Fuer einen verwaisten Termin entsteht
    // keine Einwilligung.
    cache(
        &f.store,
        &[CalEvent {
            description: Some("Text ohne Block".into()),
            attendees: vec![person("dora@example.com")],
            ..t.clone()
        }],
    );
    assert_eq!(arten(&lauf(&f.store)), [Ereignis::Verwaist]);
    assert_eq!(link(&f.store, &p.id).sync_state, SyncState::Verwaist);
    assert!(personen(&f.store, &p.id).is_empty());

    // Marke zurueck: wieder verknuepft, jetzt mit der Einwilligung.
    cache(
        &f.store,
        &[CalEvent {
            attendees: vec![person("dora@example.com")],
            ..t
        }],
    );
    assert_eq!(arten(&lauf(&f.store)), [Ereignis::Wiederverknuepft]);
    assert_eq!(link(&f.store, &p.id).sync_state, SyncState::Ok);
    assert_eq!(
        personen(&f.store, &p.id),
        [("dora@example.com".to_string(), ConsentState::Offen)]
    );
}

#[test]
fn a_missing_event_is_orphaned_only_where_the_cache_would_show_it() {
    let f = fix();
    // Plan 1: Beginn im Fenster.
    let (p1, t1) = verknuepft(&f, "u1");
    // Plan 2: Beginn weit in der Zukunft, ausserhalb des Abruffensters (+-30 Tage).
    let (p2, e2) = uebergeben(&f.store);
    let weit = termin("u2", NOW + 60 * D, Some(beschreibung(&e2)));
    cache(&f.store, &[t1, weit.clone()]);
    assert_eq!(lauf(&f.store).geaendert, 1);
    assert_eq!(link(&f.store, &p2.id).starts_at, Some(weit.starts_at));

    // Der Cache zeigt beide nicht mehr.
    cache(&f.store, &[]);
    let b = lauf(&f.store);
    assert_eq!(arten(&b), [Ereignis::Verwaist]);
    assert_eq!(b.ereignisse[0].plan_id, p1.id);
    assert_eq!(link(&f.store, &p1.id).sync_state, SyncState::Verwaist);
    assert_eq!(b.uebersprungen, 1, "Plan 2: keine Aussage");
    assert_eq!(link(&f.store, &p2.id).sync_state, SyncState::Ok);
    assert_eq!(status(&f.store, &p2.id), PlanStatus::Verknuepft);
}

#[test]
fn a_disabled_or_removed_source_says_nothing_about_the_event() {
    let f = fix();
    let (p, t) = verknuepft(&f, "u1");
    f.store
        .calendar_source_set_enabled(SRC, false, NOW)
        .unwrap();
    let b = lauf(&f.store);
    assert_eq!((b.geprueft, b.uebersprungen), (1, 1));
    assert_eq!(link(&f.store, &p.id).sync_state, SyncState::Ok);

    f.store.calendar_source_set_enabled(SRC, true, NOW).unwrap();
    let b = lauf(&f.store);
    assert!(b.ereignisse.is_empty(), "wieder sichtbar: unveraendert");
    assert_eq!(link(&f.store, &p.id).starts_at, Some(t.starts_at));

    f.store.calendar_source_remove(SRC, NOW).unwrap();
    let b = lauf(&f.store);
    assert_eq!(b.uebersprungen, 1);
    assert_eq!(link(&f.store, &p.id).sync_state, SyncState::Ok);
}

#[test]
fn a_finished_plan_is_not_changed_by_the_calendar() {
    let f = fix();
    let (p, t) = verknuepft(&f, "u1");
    status_setzen(&f.store, &p.id, PlanStatus::Durchgefuehrt, NOW).unwrap();
    cache(
        &f.store,
        &[CalEvent {
            cancelled: true,
            ..t
        }],
    );
    let b = lauf(&f.store);
    assert_eq!(b.geprueft, 0);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Durchgefuehrt);
    assert!(!link(&f.store, &p.id).abgesagt);
}

// ---------------------------------------------------------------------------
// Adapter C: Instanzen
// ---------------------------------------------------------------------------

const I1: i64 = NOW + D;
const I2: i64 = NOW + 8 * D;
const I3: i64 = NOW + 15 * D;

fn serie(text: &str, instanzen: &[i64]) -> Vec<CalEvent> {
    instanzen
        .iter()
        .map(|i| instanz("serie", *i, *i, Some(text.to_string())))
        .collect()
}

/// Eine Planung an Instanz 2 einer Serie gebunden (Auswahl bei mehrdeutiger Marke).
fn an_instanz2(f: &Fix) -> (Plan, String) {
    let (p, e) = uebergeben(&f.store);
    let text = beschreibung(&e);
    cache(&f.store, &serie(&text, &[I1, I2, I3]));
    assert_eq!(lauf(&f.store).mehrdeutig, std::slice::from_ref(&p.id));
    termin_waehlen(&f.store, &p.id, &event_key(SRC, "serie", I2), NOW).unwrap();
    let l = link(&f.store, &p.id);
    assert_eq!((l.instanz, l.starts_at), (Some(I2), Some(I2)));
    (p, text)
}

#[test]
fn a_deleted_instance_in_a_running_series_is_orphaned_and_never_replaced_by_another() {
    let f = fix();
    let (p, text) = an_instanz2(&f);
    // Instanz 2 fehlt im Cache, die anderen sind da: Fremdinstanz melden (verworfen, einmal) und
    // Abwesenheit pruefen (verwaist) sind getrennte Schritte; die Bindung bleibt Instanz 2.
    cache(&f.store, &serie(&text, &[I1, I3]));
    let b = lauf(&f.store);
    assert_eq!(
        arten(&b),
        [
            Ereignis::AndererTermin {
                erwartet: Some(I2),
                gesehen: Some(I1)
            },
            Ereignis::Verwaist
        ]
    );
    let l = link(&f.store, &p.id);
    assert_eq!(
        (l.instanz, l.starts_at),
        (Some(I2), Some(I2)),
        "Bindung bleibt, keine andere Instanz springt ein"
    );
    assert!(l.fremd_beobachtet);
    assert_eq!(l.sync_state, SyncState::Verwaist);
    assert!(lauf(&f.store).ereignisse.is_empty(), "nur einmal gemeldet");

    // Auch ohne jede andere Instanz im Cache: verwaist (kein Treffer, nichts zu melden).
    cache(&f.store, &[]);
    assert!(lauf(&f.store).ereignisse.is_empty(), "schon verwaist");
    assert_eq!(link(&f.store, &p.id).sync_state, SyncState::Verwaist);

    // Die richtige Instanz ist wieder da: der Merker faellt, die Verknuepfung lebt wieder.
    cache(&f.store, &serie(&text, &[I1, I2, I3]));
    assert_eq!(arten(&lauf(&f.store)), [Ereignis::Wiederverknuepft]);
    let l = link(&f.store, &p.id);
    assert!(!l.fremd_beobachtet);
    assert_eq!(l.sync_state, SyncState::Ok);
}

#[test]
fn a_deleted_instance_without_other_instances_is_orphaned() {
    let f = fix();
    let (p, text) = an_instanz2(&f);
    // Nur Instanz 1 und 3 fallen weg, eine Instanz einer ANDEREN Serie bleibt im Cache.
    let andere = instanz("andere-serie", I2, I2, Some(text));
    cache(&f.store, &[andere]);
    assert_eq!(arten(&lauf(&f.store)), [Ereignis::Verwaist]);
    assert_eq!(link(&f.store, &p.id).sync_state, SyncState::Verwaist);
    assert_eq!(link(&f.store, &p.id).instanz, Some(I2));
}

#[test]
fn a_moved_instance_is_found_by_its_original_start() {
    let f = fix();
    let (p, text) = an_instanz2(&f);
    let verschoben = I2 + 3 * H;
    let mut events = serie(&text, &[I1, I3]);
    events.push(instanz("serie", verschoben, I2, Some(text)));
    cache(&f.store, &events);
    let b = lauf(&f.store);
    assert_eq!(
        arten(&b),
        [Ereignis::Verschoben {
            alt_beginn: Some(I2),
            neu_beginn: verschoben
        }]
    );
    let l = link(&f.store, &p.id);
    assert_eq!((l.instanz, l.starts_at), (Some(I2), Some(verschoben)));
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verschoben);
}

#[test]
fn a_single_event_is_not_silently_replaced_by_a_series_instance() {
    let f = fix();
    let (p, t) = verknuepft(&f, "u1");
    cache(
        &f.store,
        &[instanz(
            "u1",
            t.starts_at,
            t.starts_at,
            t.description.clone(),
        )],
    );
    // Der Einzeltermin (UID, ohne Instanz) ist weg; die Serie mit gleicher UID ersetzt ihn nie.
    assert_eq!(
        arten(&lauf(&f.store)),
        [
            Ereignis::AndererTermin {
                erwartet: None,
                gesehen: Some(t.starts_at)
            },
            Ereignis::Verwaist
        ]
    );
    assert_eq!(link(&f.store, &p.id).instanz, None);
}

#[test]
fn releasing_the_identity_of_a_series_instance_needs_a_new_choice() {
    let f = fix();
    let (p, _) = an_instanz2(&f);
    assert_eq!(
        identitaet_loesen(&f.store, "gibt-es-nicht", NOW).unwrap_err(),
        "plan_nicht_gefunden"
    );
    let l = identitaet_loesen(&f.store, &p.id, NOW).unwrap();
    assert!(!hat_termin(&l));
    assert_eq!(l.source_id, None);
    assert_eq!(l.adapter, AdapterKind::IcsMarker);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Uebergeben);

    // Wieder mehrdeutig (Serie): erst die Auswahl bindet neu, jetzt an Instanz 3.
    assert_eq!(lauf(&f.store).mehrdeutig, std::slice::from_ref(&p.id));
    termin_waehlen(&f.store, &p.id, &event_key(SRC, "serie", I3), NOW).unwrap();
    assert_eq!(link(&f.store, &p.id).instanz, Some(I3));
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);

    // Direkt auf eine andere Instanz: die Auswahl loest die alte Bindung selbst.
    termin_waehlen(&f.store, &p.id, &event_key(SRC, "serie", I1), NOW).unwrap();
    assert_eq!(link(&f.store, &p.id).instanz, Some(I1));
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);
}

#[test]
fn releasing_the_identity_of_a_single_event_relinks_it_through_the_marker() {
    let f = fix();
    let (p, t) = verknuepft(&f, "einzeln");
    identitaet_loesen(&f.store, &p.id, NOW).unwrap();
    assert_eq!(status(&f.store, &p.id), PlanStatus::Uebergeben);
    let b = lauf(&f.store);
    assert_eq!(arten(&b), [Ereignis::Verknuepft]);
    assert_eq!(link(&f.store, &p.id).starts_at, Some(t.starts_at));
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);
}

// ---------------------------------------------------------------------------
// Transaktion
// ---------------------------------------------------------------------------

/// Scheitert das Speichern einer Planung mitten im Abgleich (hier: Einwilligung), rollt ALLES
/// dieser Planung zurueck: Verknuepfung, Planstatus, Einwilligungen und Verlauf. Die anderen
/// Planungen werden trotzdem abgeglichen.
#[test]
fn a_failing_save_rolls_back_the_whole_plan_and_leaves_the_others_alone() {
    let f = fix();
    let (p1, e1) = uebergeben(&f.store);
    let (p2, e2) = uebergeben(&f.store);
    cache(
        &f.store,
        &[
            CalEvent {
                attendees: vec![person("anna@example.com")],
                ..termin("u1", NOW + D, Some(beschreibung(&e1)))
            },
            termin("u2", NOW + 2 * D, Some(beschreibung(&e2))),
        ],
    );
    {
        let conn = verbindung(&f.store).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER p3_boom BEFORE INSERT ON meeting_plan_consents
             BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();
    }
    let b = lauf(&f.store);
    assert_eq!(b.geprueft, 2);
    assert_eq!(b.fehler.len(), 1);
    assert_eq!(b.fehler[0].plan_id, p1.id);
    assert!(b.fehler[0].code.contains("boom"), "{}", b.fehler[0].code);

    // Plan 1: nichts davon steht in der Datenbank.
    assert!(!hat_termin(&link(&f.store, &p1.id)));
    assert_eq!(status(&f.store, &p1.id), PlanStatus::Uebergeben);
    assert!(personen(&f.store, &p1.id).is_empty());
    assert!(einwilligungs_verlauf(&f.store, &p1.id).unwrap().is_empty());
    // Plan 2 ist verknuepft.
    assert_eq!(status(&f.store, &p2.id), PlanStatus::Verknuepft);
    assert_eq!(b.geaendert, 1);

    // Ohne den Fehler holt der naechste Lauf es nach.
    {
        let conn = verbindung(&f.store).unwrap();
        conn.execute_batch("DROP TRIGGER p3_boom;").unwrap();
    }
    let b = lauf(&f.store);
    assert!(b.fehler.is_empty());
    assert_eq!(status(&f.store, &p1.id), PlanStatus::Verknuepft);
    assert_eq!(personen(&f.store, &p1.id).len(), 1);
}

/// Aendert eine andere Verbindung den Status, nachdem der Abgleich ihn gelesen hat, schreibt der
/// Abgleich nicht darueber: `anwenden` mit einem veralteten Plan scheitert an der Pruefung im UPDATE
/// und rollt zurueck.
#[test]
fn applying_a_stale_status_fails_and_changes_nothing() {
    let f = fix();
    let (p, t) = verknuepft(&f, "u1");
    let stale = plan_laden(&f.store, &p.id).unwrap().unwrap();
    // Andere Verbindung: Absage von Hand ist nicht erlaubt; stattdessen Status zurueck auf uebergeben.
    {
        let conn = verbindung(&f.store).unwrap();
        status_wechseln(
            &conn,
            &p.id,
            PlanStatus::Verknuepft,
            PlanStatus::Uebergeben,
            NOW + 1,
        )
        .unwrap();
    }
    let l = link(&f.store, &p.id);
    let mut conn = verbindung(&f.store).unwrap();
    let tx = schreib_tx(&mut conn).unwrap();
    let neu = adapter_c_test_termin(&t, true);
    let err = adapter_c::anwenden(&tx, &stale, &l, Some(&neu), NOW + 2)
        .err()
        .expect("veralteter Status");
    assert_eq!(err, "status_geaendert");
    drop(tx);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Uebergeben);
    assert!(
        !link(&f.store, &p.id).abgesagt,
        "die Verknuepfung ist nicht halb geschrieben"
    );
}

fn adapter_c_test_termin(t: &CalEvent, abgesagt: bool) -> adapter_c::Termin {
    adapter_c::Termin {
        key: t.key.clone(),
        source_id: t.source_id.clone(),
        uid: t.uid.clone(),
        starts_at: t.starts_at,
        ends_at: t.ends_at,
        cancelled: abgesagt,
        location: None,
        join_url: None,
        text: t.description.clone().unwrap_or_default(),
        instance_start: t.instance_start,
        teilnehmende: vec![],
    }
}

// ---------------------------------------------------------------------------
// Kalender-Cache: Instanz
// ---------------------------------------------------------------------------

#[test]
fn the_instance_is_stored_in_the_calendar_cache() {
    let f = fix();
    cache(
        &f.store,
        &[
            instanz("serie", NOW + 3 * H, NOW, None),
            termin("einzeln", NOW + D, None),
        ],
    );
    let events = f.store.calendar_events_between(0, i64::MAX, true).unwrap();
    let von = |uid: &str| events.iter().find(|e| e.uid == uid).unwrap().instance_start;
    assert_eq!(von("serie"), Some(NOW));
    assert_eq!(von("einzeln"), None);
    let e = f
        .store
        .calendar_event(&event_key(SRC, "serie", NOW + 3 * H))
        .unwrap()
        .unwrap();
    assert_eq!(e.instance_start, Some(NOW));
}

// ---------------------------------------------------------------------------
// Einwilligungen und Startmodus
// ---------------------------------------------------------------------------

#[test]
fn consents_need_proof_and_keep_a_history() {
    let f = fix();
    let p = plan_anlegen(&f.store, &eingabe(), NOW).unwrap();
    let zustimmung = |person: &str, nachweis: bool| Consent {
        plan_id: p.id.clone(),
        person: person.into(),
        state: ConsentState::Zugestimmt,
        channel: nachweis.then_some(ConsentChannel::Einladung),
        recorded_at: nachweis.then_some(NOW),
    };
    assert_eq!(
        einwilligung_setzen(&f.store, &zustimmung("anna@example.com", false), NOW).unwrap_err(),
        "einwilligung_nachweis_fehlt"
    );
    assert_eq!(
        einwilligung_setzen(&f.store, &zustimmung("Anzeigename", true), NOW).unwrap_err(),
        "person_ohne_identitaet"
    );
    einwilligung_setzen(&f.store, &zustimmung("Anna@Example.com", true), NOW).unwrap();
    // Widerruf: neuer Eintrag, der Verlauf bleibt.
    einwilligung_setzen(
        &f.store,
        &Consent {
            state: ConsentState::Abgelehnt,
            ..zustimmung("anna@example.com", true)
        },
        NOW + 5,
    )
    .unwrap();
    assert_eq!(
        personen(&f.store, &p.id),
        [("anna@example.com".to_string(), ConsentState::Abgelehnt)]
    );
    let verlauf = einwilligungs_verlauf(&f.store, &p.id).unwrap();
    assert_eq!(verlauf.len(), 2);
    assert_eq!(verlauf[0].einwilligung.state, ConsentState::Zugestimmt);
    assert_eq!(verlauf[1].festgehalten_am, NOW + 5);
}

#[test]
fn the_start_mode_follows_the_consents_of_this_plan() {
    let f = fix();
    let p = plan_anlegen(&f.store, &eingabe(), NOW).unwrap();
    let anwesend = vec!["anna@example.com".to_string()];
    assert!(matches!(
        start_modus(&f.store, &p.id, &anwesend).unwrap(),
        StartModus::NurNotizen { ref offen, .. } if offen == &["anna@example.com".to_string()]
    ));
    einwilligung_setzen(
        &f.store,
        &Consent {
            plan_id: p.id.clone(),
            person: "anna@example.com".into(),
            state: ConsentState::Zugestimmt,
            channel: Some(ConsentChannel::MuendlichBeiStart),
            recorded_at: Some(NOW),
        },
        NOW,
    )
    .unwrap();
    assert_eq!(
        start_modus(&f.store, &p.id, &anwesend).unwrap(),
        StartModus::MitTranskription
    );
    assert_eq!(
        start_modus(&f.store, "gibt-es-nicht", &anwesend).unwrap_err(),
        "plan_nicht_gefunden"
    );
}

// ---------------------------------------------------------------------------
// Marke in kodiertem HTML (Vorauswahl nach der Normalisierung)
// ---------------------------------------------------------------------------

#[test]
fn a_marker_with_html_entities_is_found_after_normalising_the_description() {
    let f = fix();
    let (p, e) = uebergeben(&f.store);
    let html: String = e
        .text
        .lines()
        .map(|z| {
            // Der Doppelpunkt und der Bindestrich der Marke kommen kodiert an: `lva&#45;plan&#58;ID`.
            format!(
                "<p>{}</p>",
                z.replace(':', "&#58;").replace("lva-plan", "lva&#45;plan")
            )
        })
        .collect();
    assert!(
        !html.contains("lva-plan"),
        "Test-Aufbau: roh steht keine Marke im Text"
    );
    assert!(html.contains("&#58;"), "Test-Aufbau");
    cache(
        &f.store,
        &[termin(
            "u1",
            NOW + D,
            Some(format!("<html><body>{html}</body></html>")),
        )],
    );
    assert_eq!(lauf(&f.store).geaendert, 1);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);

    // Nur der Doppelpunkt kodiert (Rohtext traegt noch `lva-plan`): ebenso.
    let f = fix();
    let (p, e) = uebergeben(&f.store);
    let html: String = e
        .text
        .lines()
        .map(|z| format!("<p>{}</p>", z.replace(':', "&#58;")))
        .collect();
    cache(
        &f.store,
        &[termin("u2", NOW + D, Some(format!("<div>{html}</div>")))],
    );
    assert_eq!(lauf(&f.store).geaendert, 1);
    assert_eq!(status(&f.store, &p.id), PlanStatus::Verknuepft);
}

// ---------------------------------------------------------------------------
// Einwilligung gilt je Termin und Person
// ---------------------------------------------------------------------------

fn zustimmen(f: &Fix, plan_id: &str, email: &str) {
    einwilligung_setzen(
        &f.store,
        &Consent {
            plan_id: plan_id.into(),
            person: email.into(),
            state: ConsentState::Zugestimmt,
            channel: Some(ConsentChannel::Einladung),
            recorded_at: Some(NOW),
        },
        NOW,
    )
    .unwrap();
}

/// Eine Planung an Termin `uid` mit zwei Teilnehmenden (Einwilligungen `offen`).
fn mit_teilnehmenden(f: &Fix, uid: &str) -> (Plan, CalEvent) {
    let (p, e) = uebergeben(&f.store);
    let mut t = termin(uid, NOW + 2 * D, Some(beschreibung(&e)));
    t.attendees = vec![person("anna@example.com"), person("bert@example.org")];
    cache(&f.store, std::slice::from_ref(&t));
    assert_eq!(lauf(&f.store).geaendert, 1);
    assert_eq!(personen(&f.store, &p.id).len(), 2);
    (p, t)
}

#[test]
fn a_consent_for_one_event_does_not_carry_over_when_the_identity_is_released() {
    let f = fix();
    let (p, _) = mit_teilnehmenden(&f, "termin-a");
    zustimmen(&f, &p.id, "anna@example.com");
    zustimmen(&f, &p.id, "bert@example.org");
    let anwesend = vec![
        "anna@example.com".to_string(),
        "bert@example.org".to_string(),
    ];
    assert_eq!(
        start_modus(&f.store, &p.id, &anwesend).unwrap(),
        StartModus::MitTranskription
    );
    let verlauf_vorher = einwilligungs_verlauf(&f.store, &p.id).unwrap().len();

    identitaet_loesen(&f.store, &p.id, NOW + 10).unwrap();

    // Alle offen, ohne Nachweis; je Person eine Verlaufszeile, die alten Nachweise bleiben im Log.
    let aktuell = einwilligungen(&f.store, &p.id).unwrap();
    assert_eq!(aktuell.len(), 2);
    assert!(aktuell
        .iter()
        .all(|c| c.state == ConsentState::Offen && c.channel.is_none() && c.recorded_at.is_none()));
    let verlauf = einwilligungs_verlauf(&f.store, &p.id).unwrap();
    assert_eq!(verlauf.len(), verlauf_vorher + 2);
    assert!(verlauf
        .iter()
        .any(|v| v.einwilligung.state == ConsentState::Zugestimmt
            && v.einwilligung.recorded_at == Some(NOW)));
    assert_eq!(
        verlauf
            .iter()
            .filter(|v| v.festgehalten_am == NOW + 10 && v.einwilligung.state == ConsentState::Offen)
            .count(),
        2
    );
    assert!(matches!(
        start_modus(&f.store, &p.id, &anwesend).unwrap(),
        StartModus::NurNotizen { .. }
    ));
}

#[test]
fn a_consent_for_event_a_does_not_hold_after_choosing_event_b() {
    let f = fix();
    let (p, a) = mit_teilnehmenden(&f, "termin-a");
    zustimmen(&f, &p.id, "anna@example.com");
    zustimmen(&f, &p.id, "bert@example.org");
    let anwesend = vec![
        "anna@example.com".to_string(),
        "bert@example.org".to_string(),
    ];

    // Termin B traegt dieselbe Marke (Einladung weitergeleitet): die Nutzerin waehlt ihn.
    let mut b = termin("termin-b", NOW + 5 * D, a.description.clone());
    b.attendees = vec![person("anna@example.com"), person("bert@example.org")];
    cache(&f.store, &[a.clone(), b.clone()]);
    termin_waehlen(&f.store, &p.id, &b.key, NOW + 20).unwrap();
    assert_eq!(link(&f.store, &p.id).remote_id.as_deref(), Some("termin-b"));
    assert!(personen(&f.store, &p.id)
        .iter()
        .all(|(_, s)| *s == ConsentState::Offen));
    assert!(matches!(
        start_modus(&f.store, &p.id, &anwesend).unwrap(),
        StartModus::NurNotizen { .. }
    ));
    // Der Nachweis fuer Termin A bleibt im Verlauf.
    assert!(einwilligungs_verlauf(&f.store, &p.id)
        .unwrap()
        .iter()
        .any(|v| v.einwilligung.state == ConsentState::Zugestimmt));

    // Fuer Termin B neu zugestimmt; dieselbe Wahl noch einmal entwertet das nicht.
    zustimmen(&f, &p.id, "anna@example.com");
    termin_waehlen(&f.store, &p.id, &b.key, NOW + 30).unwrap();
    assert!(personen(&f.store, &p.id)
        .contains(&("anna@example.com".to_string(), ConsentState::Zugestimmt)));
}

#[test]
fn a_consent_for_one_series_instance_does_not_hold_for_another() {
    let f = fix();
    let (p, text) = an_instanz2(&f);
    let mut i3 = instanz("serie", I3, I3, Some(text));
    i3.attendees = vec![person("anna@example.com")];
    zustimmen(&f, &p.id, "anna@example.com");
    cache(&f.store, &[i3.clone()]);
    termin_waehlen(&f.store, &p.id, &i3.key, NOW + 5).unwrap();
    assert_eq!(link(&f.store, &p.id).instanz, Some(I3));
    assert!(personen(&f.store, &p.id)
        .iter()
        .all(|(_, s)| *s == ConsentState::Offen));
}

// ---------------------------------------------------------------------------
// Einwilligungs-Tor am Start der Besprechung
// ---------------------------------------------------------------------------

fn tor(f: &Fix, event: &CalEvent) -> Result<(), String> {
    start_tor(&f.store, Some(event), None)
}

#[test]
fn without_a_plan_the_start_is_unchanged() {
    let f = fix();
    let frei = termin("ohne-planung", NOW + D, None);
    assert_eq!(start_tor(&f.store, Some(&frei), None), Ok(()));
    assert_eq!(start_tor(&f.store, None, None), Ok(()));
    // Eine Besprechung ohne Terminverknuepfung als Ziel: ebenfalls unveraendert.
    let m = f.store.create_empty_meeting("Leer", None).unwrap();
    assert_eq!(start_tor(&f.store, None, Some(&m.id)), Ok(()));
}

#[test]
fn open_or_refused_consents_block_the_start() {
    let f = fix();
    let (p, t) = mit_teilnehmenden(&f, "u1");
    // Offen (noch niemand gefragt).
    assert_eq!(tor(&f, &t), Err("nur_notizen: offen".to_string()));
    // Eine Person stimmt zu, die andere lehnt ab: die Ablehnung sperrt.
    zustimmen(&f, &p.id, "anna@example.com");
    einwilligung_setzen(
        &f.store,
        &Consent {
            plan_id: p.id.clone(),
            person: "bert@example.org".into(),
            state: ConsentState::Abgelehnt,
            channel: Some(ConsentChannel::Einladung),
            recorded_at: Some(NOW),
        },
        NOW,
    )
    .unwrap();
    assert_eq!(tor(&f, &t), Err("nur_notizen: abgelehnt".to_string()));
}

#[test]
fn a_revocation_after_a_positive_query_blocks_at_the_start() {
    let f = fix();
    let (p, t) = mit_teilnehmenden(&f, "u1");
    zustimmen(&f, &p.id, "anna@example.com");
    zustimmen(&f, &p.id, "bert@example.org");
    let anwesend = vec![
        "anna@example.com".to_string(),
        "bert@example.org".to_string(),
    ];
    // Die Abfrage sagt "mit Transkription" ...
    assert_eq!(
        start_modus(&f.store, &p.id, &anwesend).unwrap(),
        StartModus::MitTranskription
    );
    // ... bis Bert widerruft; der Start liest den aktuellen Stand und sperrt.
    einwilligung_setzen(
        &f.store,
        &Consent {
            plan_id: p.id.clone(),
            person: "bert@example.org".into(),
            state: ConsentState::Abgelehnt,
            channel: Some(ConsentChannel::MuendlichBeiStart),
            recorded_at: Some(NOW + 1),
        },
        NOW + 1,
    )
    .unwrap();
    assert_eq!(tor(&f, &t), Err("nur_notizen: abgelehnt".to_string()));
}

#[test]
fn an_unknown_person_blocks_the_start() {
    let f = fix();
    let (p, mut t) = mit_teilnehmenden(&f, "u1");
    zustimmen(&f, &p.id, "anna@example.com");
    zustimmen(&f, &p.id, "bert@example.org");
    assert_eq!(
        tor(&f, &t),
        Ok(()),
        "Aufbau: beide bekannten Personen stimmen zu"
    );
    // Eine Teilnehmerin nur mit Namen (keine Adresse): kann nicht zugestimmt haben.
    t.attendees.push(Attendee {
        email: None,
        name: Some("Chris Ohne Adresse".into()),
        organizer: false,
        is_self: false,
        partstat: None,
    });
    assert_eq!(tor(&f, &t), Err("nur_notizen: offen".to_string()));
    // Auch eine Planung ohne erfasste Anwesende erlaubt keine Transkription.
    t.attendees.clear();
    assert_eq!(tor(&f, &t), Err("nur_notizen: offen".to_string()));
}

#[test]
fn all_consents_with_proof_start_with_transcription() {
    let f = fix();
    let (p, t) = mit_teilnehmenden(&f, "u1");
    zustimmen(&f, &p.id, "anna@example.com");
    zustimmen(&f, &p.id, "bert@example.org");
    assert_eq!(tor(&f, &t), Ok(()));
    // Eine geplante Transkription ist Voraussetzung.
    let mut eing = eingabe();
    eing.transcription_planned = false;
    plan_aendern(&f.store, &p.id, &eing, NOW + 1).unwrap();
    assert_eq!(tor(&f, &t), Err("nur_notizen: nicht_geplant".to_string()));
}

#[test]
fn the_gate_finds_the_plan_through_the_meeting_link_of_the_target_entry() {
    let f = fix();
    let (p, t) = mit_teilnehmenden(&f, "u1");
    let m = f.store.create_empty_meeting("Ziel", None).unwrap();
    let im_cache = f.store.calendar_event(&t.key).unwrap().unwrap();
    f.store
        .link_meeting_event(&m.id, &im_cache, "prompt", NOW)
        .unwrap();
    // Ohne Termin im Aufruf: der Zieleintrag fuehrt ueber `meeting_calendar_links` zur Planung.
    assert_eq!(
        start_tor(&f.store, None, Some(&m.id)),
        Err("nur_notizen: offen".to_string())
    );
    zustimmen(&f, &p.id, "anna@example.com");
    zustimmen(&f, &p.id, "bert@example.org");
    assert_eq!(start_tor(&f.store, None, Some(&m.id)), Ok(()));

    // Der Termin ist aus dem Cache gefallen: nur noch die UID ist bekannt, Anwesende fehlen: sperrt.
    cache(&f.store, &[]);
    assert_eq!(
        start_tor(&f.store, None, Some(&m.id)),
        Err("nur_notizen: offen".to_string())
    );
}

#[test]
fn the_gate_is_per_series_instance() {
    let f = fix();
    let (p, text) = an_instanz2(&f);
    let mut i1 = instanz("serie", I1, I1, Some(text.clone()));
    let mut i2 = instanz("serie", I2, I2, Some(text));
    i1.attendees = vec![person("anna@example.com")];
    i2.attendees = vec![person("anna@example.com")];
    // Die Planung gehoert zu Instanz 2; Instanz 1 hat keine Planung und startet wie bisher.
    assert_eq!(tor(&f, &i1), Ok(()));
    assert_eq!(tor(&f, &i2), Err("nur_notizen: offen".to_string()));
    zustimmen(&f, &p.id, "anna@example.com");
    assert_eq!(tor(&f, &i2), Ok(()));
}

/// Die Recorder-Methode ruft das Tor vor dem Anlegen der Zeile auf; ein Umbau, der es
/// uebergeht, faellt hier auf. Alle drei Startwege laufen durch `start_for_event`.
#[test]
fn the_recorder_asks_the_gate_before_anything_starts() {
    let recorder = include_str!("../meetings/recorder.rs");
    let produktiv = recorder.split("#[cfg(test)]").next().unwrap();
    let fn_start = produktiv
        .find("pub fn start_for_event")
        .expect("start_for_event");
    let rumpf = &produktiv[fn_start..];
    let tor = rumpf.find("planning::start_tor(").expect("Tor im Start");
    for spaeter in ["begin_live_row(", "cancel_final_jobs()", "start_row("] {
        assert!(
            tor < rumpf.find(spaeter).expect(spaeter),
            "das Tor muss vor {spaeter} stehen"
        );
    }
    assert!(produktiv.contains("self.start_for_event(\n            title,"));
    let befehle = include_str!("../../commands/meetings.rs");
    assert!(befehle.contains("recorder.start_for_event("));
    let hub = include_str!("../workflows/hub.rs");
    assert!(hub.contains("recorder.start_for_event("));
}
