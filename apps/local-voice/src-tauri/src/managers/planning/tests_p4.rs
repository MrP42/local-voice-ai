use super::*;
use besprechungsplanung::model::ConsentState;

fn fix() -> (tempfile::TempDir, MeetingStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    (dir, store)
}

// Ohne Annahmen ueber die Tabellen des MeetingStore: ein Fehler darf keine Zeile anlegen.
fn zeilenzahlen(store: &MeetingStore) -> Vec<(String, i64)> {
    let conn = verbindung(store).unwrap();
    let mut st = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .unwrap();
    let tabellen = st
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    tabellen
        .into_iter()
        .map(|name| {
            let sql = format!("SELECT COUNT(*) FROM \"{}\"", name.replace('"', "\"\""));
            let anzahl = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
            (name, anzahl)
        })
        .collect()
}

#[test]
fn termin_liefert_planung_mit_aktuellem_einwilligungsstand() {
    let (_dir, store) = fix();
    for (uid, zugestimmt, stand) in [
        ("uid-offen", false, ConsentState::Offen),
        ("uid-ja", true, ConsentState::Zugestimmt),
    ] {
        let (plan, event) = testhilfe::plan_mit_termin(&store, uid, zugestimmt);
        let vorher = zeilenzahlen(&store);
        let plaene = plaene_zum_termin(&store, &event).unwrap();
        assert_eq!(plaene.len(), 1);
        assert_eq!(plaene[0].plan, plan);
        assert_eq!(plaene[0].einwilligungen.len(), 2);
        assert!(plaene[0].einwilligungen.iter().all(|c| c.state == stand));
        assert_eq!(zeilenzahlen(&store), vorher);

        let mut fremd = event.clone();
        fremd.uid = "uid-fremd".into();
        fremd.key = "key-fremd".into();
        assert!(plaene_zum_termin(&store, &fremd).unwrap().is_empty());

        plan_loeschen(&store, &plan.id, testhilfe::NOW + 1).unwrap();
        assert!(plaene_zum_termin(&store, &event).unwrap().is_empty());
    }
}

#[test]
fn notizen_verknuepfen_einen_neuen_eintrag_mit_der_planung() {
    let (_dir, store) = fix();
    let (plan, _) = testhilfe::plan_mit_termin(&store, "uid-notizen", false);
    assert!(plan.meeting_id.is_none());
    let meeting = notizen_anlegen(&store, &plan.id, "Notizen", testhilfe::NOW + 1).unwrap();
    let geladen = plan_laden(&store, &plan.id).unwrap().unwrap();
    assert_eq!(geladen.meeting_id.as_deref(), Some(meeting.id.as_str()));
}

#[test]
fn unbekannte_planung_legt_keinen_eintrag_an() {
    let (_dir, store) = fix();
    let vorher = zeilenzahlen(&store);
    let fehler = notizen_anlegen(&store, "unbekannt", "Notizen", testhilfe::NOW)
        .err()
        .unwrap();
    assert_eq!(fehler, "plan_nicht_gefunden");
    assert_eq!(zeilenzahlen(&store), vorher);
}

#[test]
fn leerer_titel_bleibt_erkennbar_und_aendert_die_zuordnung_nicht() {
    let (_dir, store) = fix();
    let (plan, _) = testhilfe::plan_mit_termin(&store, "uid-titel", false);
    let meeting = notizen_anlegen(&store, &plan.id, "Notizen", testhilfe::NOW + 1).unwrap();
    let vorher = zeilenzahlen(&store);
    let fehler = notizen_anlegen(&store, &plan.id, "", testhilfe::NOW + 2)
        .err()
        .unwrap();
    assert!(fehler.starts_with("title_empty"));
    assert_eq!(zeilenzahlen(&store), vorher);
    let geladen = plan_laden(&store, &plan.id).unwrap().unwrap();
    assert_eq!(geladen.meeting_id.as_deref(), Some(meeting.id.as_str()));
}

#[test]
fn notizen_bleiben_fuer_transkription_durch_das_tor_gesperrt() {
    let (_dir, store) = fix();
    let (plan, _) = testhilfe::plan_mit_termin(&store, "uid-n", false);
    let meeting = notizen_anlegen(&store, &plan.id, "Notizen", testhilfe::NOW + 1).unwrap();
    let fehler = start_tor(&store, None, Some(&meeting.id)).unwrap_err();
    assert!(ist_ablehnung(&fehler));
}
