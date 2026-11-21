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
        assert_eq!(plaene[0].plan, PlanKurz::von(&plan));
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
fn notizen_starten_idempotent_ohne_ueberschreiben() {
    let (_dir, store) = fix();
    let (plan, _) = testhilfe::plan_mit_termin(&store, "uid-notizen-doppelt", false);
    let meeting = notizen_anlegen(&store, &plan.id, "Notizen", testhilfe::NOW + 1).unwrap();
    let vorher = zeilenzahlen(&store);
    let nochmal = notizen_anlegen(&store, &plan.id, "Anderer Titel", testhilfe::NOW + 2).unwrap();
    assert_eq!(nochmal.id, meeting.id);
    assert_eq!(zeilenzahlen(&store), vorher);
    // Ein bestehender Eintrag braucht auch keine erneute Titelpruefung.
    let ohne_titel = notizen_anlegen(&store, &plan.id, "", testhilfe::NOW + 3).unwrap();
    assert_eq!(ohne_titel.id, meeting.id);
    assert_eq!(zeilenzahlen(&store), vorher);
    let geladen = plan_laden(&store, &plan.id).unwrap().unwrap();
    assert_eq!(geladen.meeting_id.as_deref(), Some(meeting.id.as_str()));
}

#[test]
fn notizen_ersetzen_einen_nicht_mehr_auffindbaren_bezug() {
    let (_dir, store) = fix();
    let (plan, _) = testhilfe::plan_mit_termin(&store, "uid-notizen-weg", false);
    plan_store::plan_besprechung_setzen(
        &verbindung(&store).unwrap(),
        &plan.id,
        Some("m_weg"),
        testhilfe::NOW,
    )
    .unwrap();
    assert!(store.get_meeting("m_weg").unwrap().is_none());
    let meeting = notizen_anlegen(&store, &plan.id, "Notizen", testhilfe::NOW + 1).unwrap();
    assert_ne!(meeting.id, "m_weg");
    assert!(store.get_meeting(&meeting.id).unwrap().is_some());
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
fn leerer_titel_legt_keinen_eintrag_an_und_aendert_die_zuordnung_nicht() {
    let (_dir, store) = fix();
    let (plan, _) = testhilfe::plan_mit_termin(&store, "uid-titel", false);
    let vorher = zeilenzahlen(&store);
    let fehler = notizen_anlegen(&store, &plan.id, "", testhilfe::NOW + 2)
        .err()
        .unwrap();
    assert!(fehler.starts_with("title_empty"));
    assert_eq!(zeilenzahlen(&store), vorher);
    let geladen = plan_laden(&store, &plan.id).unwrap().unwrap();
    assert!(geladen.meeting_id.is_none());
}

#[test]
fn notizen_bleiben_fuer_transkription_durch_das_tor_gesperrt() {
    let (_dir, store) = fix();
    let (plan, _) = testhilfe::plan_mit_termin(&store, "uid-n", false);
    let meeting = notizen_anlegen(&store, &plan.id, "Notizen", testhilfe::NOW + 1).unwrap();
    let fehler = start_tor(&store, None, Some(&meeting.id)).unwrap_err();
    assert!(ist_ablehnung(&fehler));
}

#[test]
fn nachgemeldete_anwesende_brauchen_zustimmung_und_widerruf_sperrt() {
    let (_dir, store) = fix();
    let meeting = store.create_empty_meeting("Test", None).unwrap();
    let (plan, _) = testhilfe::plan_fuer_besprechung(&store, &meeting.id, "uid-anwesende", true);
    let personen = vec!["carl@example.org".to_string()];
    // Gegenprobe: ohne Meldung kennt das Backend die Zusatzperson nicht; das Tor besteht.
    tor_pruefen_besprechung(&store, &meeting.id).unwrap();

    anwesende_ergaenzen(&store, &meeting.id, &plan.id, &personen).unwrap();
    let fehler = tor_pruefen_besprechung(&store, &meeting.id).unwrap_err();
    assert!(ist_ablehnung(&fehler));
    assert_eq!(fehler, "nur_notizen: offen");

    einwilligung_setzen(
        &store,
        &testhilfe::zustimmung(&plan.id, &personen[0]),
        testhilfe::NOW + 1,
    )
    .unwrap();
    tor_pruefen_besprechung(&store, &meeting.id).unwrap();

    einwilligung_setzen(
        &store,
        &testhilfe::widerruf(&plan.id, &personen[0]),
        testhilfe::NOW + 2,
    )
    .unwrap();
    let fehler = tor_pruefen_besprechung(&store, &meeting.id).unwrap_err();
    assert!(ist_ablehnung(&fehler));
    assert_eq!(fehler, "nur_notizen: abgelehnt");

    let conn = verbindung(&store).unwrap();
    let lesen = || {
        conn.query_row(
            "SELECT attendees_json FROM meeting_plan_bindings
             WHERE meeting_id = ?1 AND plan_id = ?2",
            params![meeting.id, plan.id],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
    };
    let vorher = lesen();
    anwesende_ergaenzen(&store, &meeting.id, &plan.id, &personen).unwrap();
    let nachher = lesen();
    assert_eq!(nachher, vorher);
    let anwesende: Vec<String> = serde_json::from_str(&nachher).unwrap();
    assert_eq!(anwesende.iter().filter(|p| *p == &personen[0]).count(), 1);
}

#[test]
fn anwesende_ohne_planungsbezug_schreiben_keine_bindung() {
    let (_dir, store) = fix();
    let meeting = store.create_empty_meeting("Test", None).unwrap();
    let (plan, _) =
        testhilfe::plan_fuer_besprechung(&store, &meeting.id, "uid-anwesende-fremd", true);
    tor_pruefen_besprechung(&store, &meeting.id).unwrap();
    let fremd = store.create_empty_meeting("Fremd", None).unwrap();
    let vorher = zeilenzahlen(&store);
    let fehler = anwesende_ergaenzen(
        &store,
        &fremd.id,
        &plan.id,
        &["carl@example.org".to_string()],
    )
    .unwrap_err();
    assert_eq!(fehler, "besprechung_ohne_planung");
    assert_eq!(zeilenzahlen(&store), vorher);
    assert!(bindungen(&verbindung(&store).unwrap(), &fremd.id)
        .unwrap()
        .is_empty());
}

#[test]
fn ungueltige_person_verhindert_auch_gueltige_ergaenzungen() {
    let (_dir, store) = fix();
    let meeting = store.create_empty_meeting("Test", None).unwrap();
    let (plan, _) =
        testhilfe::plan_fuer_besprechung(&store, &meeting.id, "uid-anwesende-ungueltig", true);
    tor_pruefen_besprechung(&store, &meeting.id).unwrap();
    let conn = verbindung(&store).unwrap();
    let vorher = bindungen(&conn, &meeting.id).unwrap();
    for personen in [
        vec!["kein-at".to_string()],
        vec!["carl@example.org".to_string(), "kein-at".to_string()],
    ] {
        let fehler = anwesende_ergaenzen(&store, &meeting.id, &plan.id, &personen).unwrap_err();
        assert_eq!(fehler, "person_ungueltig");
        assert_eq!(bindungen(&conn, &meeting.id).unwrap(), vorher);
    }
    tor_pruefen_besprechung(&store, &meeting.id).unwrap();
}

#[test]
fn leere_anwesendenliste_schreibt_nichts() {
    let (_dir, store) = fix();
    let vorher = zeilenzahlen(&store);
    anwesende_ergaenzen(&store, "unbekannt", "unbekannt", &[]).unwrap();
    assert_eq!(zeilenzahlen(&store), vorher);
}

#[test]
fn nachgemeldete_person_gilt_fuer_jede_planung_der_besprechung() {
    let (_dir, store) = fix();
    let meeting = store.create_empty_meeting("Test", None).unwrap();
    let (a, _) = testhilfe::plan_fuer_besprechung(&store, &meeting.id, "uid-a", true);
    let (b, _) = testhilfe::plan_mit_termin(&store, "uid-b", true);
    // Beide Planungen gelten fuer die Besprechung (zwei Planungen am selben Termin).
    let anwesende = vec![testhilfe::ANNA.to_string(), testhilfe::BERT.to_string()];
    in_tx(&store, |tx| {
        binden(
            tx,
            &meeting.id,
            &[
                (a.id.clone(), anwesende.clone()),
                (b.id.clone(), anwesende.clone()),
            ],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(tor_pruefen_besprechung(&store, &meeting.id).is_ok());
    let carl = "carl@example.org";
    anwesende_ergaenzen(&store, &meeting.id, &a.id, &[carl.to_string()]).unwrap();
    // Carl stimmt A zu, lehnt B ab: die Zustimmung zu A ersetzt nie die zu B.
    einwilligung_setzen(&store, &testhilfe::zustimmung(&a.id, carl), testhilfe::NOW).unwrap();
    einwilligung_setzen(&store, &testhilfe::widerruf(&b.id, carl), testhilfe::NOW).unwrap();
    let fehler = tor_pruefen_besprechung(&store, &meeting.id).unwrap_err();
    assert!(ist_ablehnung(&fehler), "{fehler}");
    // Gegenprobe: mit Zustimmung auch zu B besteht das Tor.
    einwilligung_setzen(
        &store,
        &testhilfe::zustimmung(&b.id, carl),
        testhilfe::NOW + 2,
    )
    .unwrap();
    assert!(tor_pruefen_besprechung(&store, &meeting.id).is_ok());
}

#[test]
fn nachgemeldete_person_sperrt_schon_die_vorpruefung_eines_vorgemerkten_teils() {
    let (_dir, store) = fix();
    let ziel = store.create_empty_meeting("Ziel", None).unwrap();
    let (a, _) = testhilfe::plan_fuer_besprechung(&store, &ziel.id, "uid-t-a", true);
    let (b, _) = testhilfe::plan_mit_termin(&store, "uid-t-b", true);
    let anwesende = vec![testhilfe::ANNA.to_string(), testhilfe::BERT.to_string()];
    in_tx(&store, |tx| {
        binden(
            tx,
            &ziel.id,
            &[
                (a.id.clone(), anwesende.clone()),
                (b.id.clone(), anwesende.clone()),
            ],
        )?;
        Ok(())
    })
    .unwrap();
    let teil = store.create_empty_meeting("Teil", None).unwrap();
    store.append_pending_add(&teil.id, &ziel.id).unwrap();
    let carl = "carl@example.org";
    anwesende_ergaenzen(&store, &ziel.id, &a.id, &[carl.to_string()]).unwrap();
    einwilligung_setzen(&store, &testhilfe::zustimmung(&a.id, carl), testhilfe::NOW).unwrap();
    einwilligung_setzen(&store, &testhilfe::widerruf(&b.id, carl), testhilfe::NOW).unwrap();
    // Schon die VORpruefung des Teils sperrt (vor jedem Modellaufruf), nicht erst die Nachpruefung.
    let fehler = tor_pruefen_besprechung(&store, &teil.id).unwrap_err();
    assert!(ist_ablehnung(&fehler), "{fehler}");
}
