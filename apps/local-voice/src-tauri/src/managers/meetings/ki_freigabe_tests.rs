//! Tests der Freigabe für KI-Clients (G9a-2). Nur synthetische Adressen (`example.*`).

use super::*;
use crate::managers::calendar::model::{Attendee, CalEvent, CalendarKind};
use crate::managers::meetings::store::{MeetingSource, SyncMeta};

const T1: i64 = 1_790_000_000_000;

fn store_mit_quelle() -> (tempfile::TempDir, MeetingStore) {
    let dir = tempfile::tempdir().unwrap();
    let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    s.calendar_source_add("src1", CalendarKind::Ics, "Kalender", None, 1_000)
        .unwrap();
    s.ki_richtlinie_setzen(&sperre().richtlinie().unwrap())
        .unwrap();
    (dir, s)
}

fn nachweis() -> Freigabenachweis {
    Freigabenachweis {
        informiert: true,
        rechtsgrundlage: Some(Rechtsgrundlage::OeffentlicheAufgabe),
        einwilligungen_liegen_vor: false,
    }
}

fn ohne_information() -> Freigabenachweis {
    Freigabenachweis {
        informiert: false,
        ..nachweis()
    }
}

fn rev(s: &MeetingStore) -> i64 {
    s.ki_stand().unwrap().rev
}

fn teilnehmer(email: Option<&str>, name: Option<&str>) -> Attendee {
    Attendee {
        email: email.map(str::to_string),
        name: name.map(str::to_string),
        organizer: false,
        is_self: false,
        partstat: None,
    }
}

fn termin(attendees: Vec<Attendee>) -> CalEvent {
    CalEvent {
        key: crate::managers::calendar::model::event_key("src1", "u1", T1),
        source_id: "src1".into(),
        uid: "u1".into(),
        title: "Jour fixe".into(),
        starts_at: T1,
        ends_at: T1 + 1_800_000,
        all_day: false,
        cancelled: false,
        location: None,
        join_url: None,
        description: None,
        instance_start: None,
        attendees,
    }
}

/// Abgleich nach dem Ende des Termins (Teilnehmerstand aktuell).
fn meta(has_attendee_data: bool) -> SyncMeta<'static> {
    meta_um(has_attendee_data, T1 + 3_600_000)
}

fn meta_um(has_attendee_data: bool, now_ms: i64) -> SyncMeta<'static> {
    SyncMeta {
        has_attendee_data,
        etag: None,
        last_modified: None,
        now_ms,
    }
}

/// Besprechung mit verknüpftem Termin; liefert Store und Besprechungs-Id.
fn besprechung(
    attendees: Vec<Attendee>,
    has_attendee_data: bool,
) -> (tempfile::TempDir, MeetingStore, String) {
    let (dir, s) = store_mit_quelle();
    let ev = termin(attendees);
    s.calendar_replace_events("src1", std::slice::from_ref(&ev), &meta(has_attendee_data))
        .unwrap();
    let m = s
        .create_meeting("Jour fixe", MeetingSource::Live, Some(1))
        .unwrap();
    s.link_meeting_event(&m.id, &ev, "manual", 3_000).unwrap();
    (dir, s, m.id)
}

fn intern_liste() -> Vec<Attendee> {
    vec![
        teilnehmer(Some("a@example.org"), Some("Anna")),
        teilnehmer(Some("b@example.org"), None),
    ]
}

fn sperre() -> KiSperre {
    KiSperre::Pruefen {
        kundenprofil_nur_lokal: false,
        cloud_bei_externen: false,
        eigene_domaenen: vec!["example.org".into()],
    }
}

#[test]
fn liste_kommt_aus_dem_termin_und_name_wird_nie_mit_adresse_verbunden() {
    let (_d, s, id) = besprechung(
        vec![
            teilnehmer(Some("a@example.org"), Some("x@example.com")),
            teilnehmer(None, Some("Ohne Adresse")),
            teilnehmer(None, None),
        ],
        true,
    );
    let lage = s.ki_teilnehmerlage(&id).unwrap();
    assert!(lage.liste_aus_termin);
    assert_eq!(lage.teilnehmende, vec!["a@example.org", "?", "?"]);
}

#[test]
fn ohne_verknuepfung_oder_ohne_teilnehmerdaten_ist_die_liste_ad_hoc() {
    let (_d, s) = store_mit_quelle();
    let m = s
        .create_meeting("Spontan", MeetingSource::Live, Some(1))
        .unwrap();
    let lage = s.ki_teilnehmerlage(&m.id).unwrap();
    assert!(!lage.liste_aus_termin);
    assert!(lage.teilnehmende.is_empty());

    let (_d2, s2, id2) = besprechung(intern_liste(), false);
    assert!(!s2.ki_teilnehmerlage(&id2).unwrap().liste_aus_termin);
}

#[test]
fn ohne_bestaetigung_gesperrt_mit_bestaetigung_sichtbar() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    assert!(!sperre().sichtbar(&s, &id));
    let st = stand(&s, &sperre(), &id).unwrap();
    assert_eq!(st.ergebnis.einstufung, Einstufung::Intern);
    assert!(st.bestaetigung.is_none());
    let st = bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &st.ergebnis.fingerabdruck,
        &nachweis(),
        5_000,
    )
    .unwrap();
    assert!(st.entscheidung.erlaubt());
    assert!(sperre().sichtbar(&s, &id));
}

#[test]
fn veralteter_fingerabdruck_und_zu_lockere_einstufung_werden_abgelehnt() {
    let (_d, s, id) = besprechung(
        vec![
            teilnehmer(Some("a@example.org"), None),
            teilnehmer(Some("x@example.com"), None),
        ],
        true,
    );
    let st = stand(&s, &sperre(), &id).unwrap();
    assert_eq!(st.ergebnis.einstufung, Einstufung::Extern);
    let e = bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Extern,
        "0",
        &nachweis(),
        5_000,
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "ki_freigabe_veraltet");
    let e = bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &st.ergebnis.fingerabdruck,
        &nachweis(),
        5_000,
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "ki_freigabe_nicht_intern");
    assert!(s.ki_bestaetigung(&id).unwrap().is_none());
}

#[test]
fn extern_bestaetigt_nur_mit_einstellung_sichtbar() {
    let (_d, s, id) = besprechung(
        vec![
            teilnehmer(Some("a@example.org"), None),
            teilnehmer(Some("x@example.com"), None),
        ],
        true,
    );
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Extern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    assert!(!sperre().sichtbar(&s, &id));
    let mit_externen = KiSperre::Pruefen {
        kundenprofil_nur_lokal: false,
        cloud_bei_externen: true,
        eigene_domaenen: vec!["example.org".into()],
    };
    assert!(mit_externen.sichtbar(&s, &id));
}

#[test]
fn neue_teilnehmende_im_termin_machen_die_bestaetigung_ungueltig() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    assert!(sperre().sichtbar(&s, &id));
    let mut neu = intern_liste();
    neu.push(teilnehmer(Some("c@example.org"), None));
    s.calendar_replace_events("src1", &[termin(neu)], &meta(true))
        .unwrap();
    assert!(!sperre().sichtbar(&s, &id), "Nachzügler: neu bestätigen");
}

#[test]
fn geaenderte_eigene_domaenen_und_kundenprofil_sperren() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    let andere = KiSperre::Pruefen {
        kundenprofil_nur_lokal: false,
        cloud_bei_externen: false,
        eigene_domaenen: vec!["example.org".into(), "example.net".into()],
    };
    assert!(!andere.sichtbar(&s, &id));
    let lokal = KiSperre::Pruefen {
        kundenprofil_nur_lokal: true,
        cloud_bei_externen: false,
        eigene_domaenen: vec!["example.org".into()],
    };
    assert!(lokal.ganz_gesperrt());
    assert!(!lokal.sichtbar(&s, &id));
}

#[test]
fn bestaetigung_gehoert_zur_besprechung_und_geht_mit_ihr() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    // Zweite Besprechung am selben Termin: eigene, noch fehlende Bestätigung.
    let ev = termin(intern_liste());
    let m2 = s
        .create_meeting("Jour fixe 2", MeetingSource::Live, Some(1))
        .unwrap();
    s.link_meeting_event(&m2.id, &ev, "manual", 3_000).unwrap();
    assert!(!sperre().sichtbar(&s, &m2.id));
    s.soft_delete_meeting(&id).unwrap();
    assert!(s.ki_bestaetigung(&id).unwrap().is_none());
}

#[test]
fn zuruecknehmen_sperrt_sofort() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    s.ki_bestaetigung_entfernen(&id).unwrap();
    assert!(!sperre().sichtbar(&s, &id));
}

#[test]
fn unlesbarer_wert_zaehlt_als_keine_bestaetigung() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    // Die CHECK-Bedingung lässt keinen fremden Wert zu.
    let conn = s.get_connection().unwrap();
    assert!(conn
        .execute(
            "INSERT INTO meeting_ai_confirmations VALUES (?1, 'irgendwas', 'x', 2, 1, 1)",
            params![id],
        )
        .is_err());
}

#[test]
fn liste_ohne_abgleich_nach_dem_ende_ist_veraltet_und_gesperrt() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    assert!(sperre().sichtbar(&s, &id));
    // Letzter erfolgreicher Abgleich VOR dem Ende des Termins: Stand nicht belegt.
    s.calendar_replace_events("src1", &[termin(intern_liste())], &meta_um(true, T1))
        .unwrap();
    assert_eq!(
        s.ki_teilnehmerlage(&id).unwrap().teilnehmerinfo,
        Teilnehmerinfo::Veraltet
    );
    assert!(!sperre().sichtbar(&s, &id));
    let mit_externen = KiSperre::Pruefen {
        kundenprofil_nur_lokal: false,
        cloud_bei_externen: true,
        eigene_domaenen: vec!["example.org".into()],
    };
    assert!(!mit_externen.sichtbar(&s, &id));
}

#[test]
fn liste_an_der_importgrenze_ist_unvollstaendig() {
    let viele: Vec<Attendee> = (0..TEILNEHMER_IMPORTGRENZE)
        .map(|i| teilnehmer(Some(&format!("p{i}@example.org")), None))
        .collect();
    let (_d, s, id) = besprechung(viele, true);
    assert_eq!(
        s.ki_teilnehmerlage(&id).unwrap().teilnehmerinfo,
        Teilnehmerinfo::Unvollstaendig
    );
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    assert!(!sperre().sichtbar(&s, &id));
}

#[test]
fn ohne_termin_ist_der_teilnehmerstand_unbekannt() {
    let (_d, s) = store_mit_quelle();
    let m = s
        .create_meeting("Spontan", MeetingSource::Live, Some(1))
        .unwrap();
    assert_eq!(
        s.ki_teilnehmerlage(&m.id).unwrap().teilnehmerinfo,
        Teilnehmerinfo::Unbekannt
    );
}

#[test]
fn bestaetigung_mit_anderer_schemaversion_zaehlt_nicht() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    assert!(sperre().sichtbar(&s, &id));
    let conn = s.get_connection().unwrap();
    conn.execute(
        "UPDATE meeting_ai_confirmations SET fp_version = 1 WHERE meeting_id = ?1",
        params![id],
    )
    .unwrap();
    assert!(s.ki_bestaetigung(&id).unwrap().is_none());
    assert!(!sperre().sichtbar(&s, &id));
}

#[test]
fn adressfoermiger_name_ersetzt_nie_die_fehlende_adresse() {
    // Bestätigte interne Liste …
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    assert!(sperre().sichtbar(&s, &id));
    // … die Adresse fällt weg, derselbe Wert steht nur noch als Name da.
    s.calendar_replace_events(
        "src1",
        &[termin(vec![
            teilnehmer(None, Some("a@example.org")),
            teilnehmer(Some("b@example.org"), None),
        ])],
        &meta(true),
    )
    .unwrap();
    let lage = s.ki_teilnehmerlage(&id).unwrap();
    assert_eq!(lage.teilnehmende, vec!["?", "b@example.org"]);
    let st = stand(&s, &sperre(), &id).unwrap();
    assert_eq!(st.ergebnis.einstufung, Einstufung::Unbekannt);
    assert_ne!(st.ergebnis.fingerabdruck, fp);
    assert!(!sperre().sichtbar(&s, &id));
    let mit_externen = KiSperre::Pruefen {
        kundenprofil_nur_lokal: false,
        cloud_bei_externen: true,
        eigene_domaenen: vec!["example.org".into()],
    };
    assert!(!mit_externen.sichtbar(&s, &id));
}

/// Codex #583 Fixrunde 2: Die Revision steigt bei JEDER Änderung an der Freigabegrundlage, in
/// der App selbst (Trigger), und sinkt nie; auch A → B → A bleibt sichtbar.
#[test]
fn revision_steigt_bei_jeder_aenderung_der_freigabegrundlage() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    let r0 = rev(&s);
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    let r1 = rev(&s);
    assert!(r1 > r0, "Bestätigung");
    // Kalender A → B → A: gleicher Inhalt, höhere Revision.
    let mut neu = intern_liste();
    neu.push(teilnehmer(Some("c@example.org"), None));
    s.calendar_replace_events("src1", &[termin(neu)], &meta(true))
        .unwrap();
    let r2 = rev(&s);
    assert!(r2 > r1, "Kalender A → B");
    s.calendar_replace_events("src1", &[termin(intern_liste())], &meta(true))
        .unwrap();
    let r3 = rev(&s);
    assert!(r3 > r2, "Kalender B → A");
    assert_eq!(s.ki_teilnehmerlage(&id).unwrap().teilnehmende.len(), 2);
    // Widerruf, Richtlinie, Verknüpfung.
    s.ki_bestaetigung_entfernen(&id).unwrap();
    let r4 = rev(&s);
    assert!(r4 > r3, "Widerruf");
    let mut r = sperre().richtlinie().unwrap();
    r.cloud_bei_externen = true;
    assert!(s.ki_richtlinie_setzen(&r).unwrap());
    let r5 = rev(&s);
    assert!(r5 > r4, "Richtlinie");
    assert!(!s.ki_richtlinie_setzen(&r).unwrap(), "unverändert");
    assert_eq!(rev(&s), r5, "unverändert: keine neue Revision");
    s.unlink_meeting_event(&id).unwrap();
    assert!(rev(&s) > r5, "Verknüpfung");
}

#[test]
fn ohne_erklaerung_zur_information_keine_bestaetigung() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    let e = bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &ohne_information(),
        5_000,
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "ki_freigabe_ohne_information");
    assert!(s.ki_bestaetigung(&id).unwrap().is_none());
    let conn = s.get_connection().unwrap();
    assert!(conn
        .execute(
            "INSERT INTO meeting_ai_confirmations
               (meeting_id, einstufung, fingerabdruck, fp_version, informiert, confirmed_at,
                rechtsgrundlage)
             VALUES (?1, 'intern', 'x', 2, 0, 1, 'vertrag')",
            params![id],
        )
        .is_err());
}

/// ADR-0031 E2 Nr. 7: ohne Rechtsgrundlage keine Freigabe; Einwilligung nur, wenn sie vorliegt.
/// Eine Zeile ohne Rechtsgrundlage (Altbestand vor Migration 20) zählt nicht.
#[test]
fn ohne_rechtsgrundlage_oder_ohne_vorliegende_einwilligung_keine_freigabe() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    let ohne = Freigabenachweis {
        rechtsgrundlage: None,
        ..nachweis()
    };
    let e = bestaetigen(&s, &sperre(), &id, Einstufung::Intern, &fp, &ohne, 5_000).unwrap_err();
    assert_eq!(e.to_string(), "ki_freigabe_ohne_rechtsgrundlage");
    let einwilligung = Freigabenachweis {
        rechtsgrundlage: Some(Rechtsgrundlage::Einwilligung),
        ..nachweis()
    };
    let e = bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &einwilligung,
        5_000,
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "ki_freigabe_ohne_einwilligung");
    assert!(!sperre().sichtbar(&s, &id));
    let belegt = Freigabenachweis {
        einwilligungen_liegen_vor: true,
        ..einwilligung
    };
    bestaetigen(&s, &sperre(), &id, Einstufung::Intern, &fp, &belegt, 5_000).unwrap();
    assert!(sperre().sichtbar(&s, &id));
    // Einwilligung ohne Beleg in der Zeile: zählt nicht.
    let conn = s.get_connection().unwrap();
    conn.execute(
        "UPDATE meeting_ai_confirmations SET einwilligungen_belegt = 0",
        [],
    )
    .unwrap();
    assert!(!sperre().sichtbar(&s, &id));
    // Altzeile ohne Rechtsgrundlage: zählt nicht.
    conn.execute(
        "UPDATE meeting_ai_confirmations SET rechtsgrundlage = NULL, einwilligungen_belegt = 1",
        [],
    )
    .unwrap();
    assert!(!sperre().sichtbar(&s, &id));
    conn.execute(
        "UPDATE meeting_ai_confirmations SET rechtsgrundlage = 'erfunden'",
        [],
    )
    .unwrap();
    assert!(!sperre().sichtbar(&s, &id));
}

/// Domänen A → B → A über den Spiegel (so schreiben Befehl und Einstellungspfad): Die alte
/// Freigabe kommt nicht zurück; eine andere Richtlinienänderung lässt sie stehen.
#[test]
fn domaenen_a_b_a_reaktiviert_nichts() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    let a = sperre().richtlinie().unwrap();
    let mut cloud = a.clone();
    cloud.cloud_bei_externen = true;
    s.ki_richtlinie_setzen(&cloud).unwrap();
    assert!(
        s.ki_bestaetigung(&id).unwrap().is_some(),
        "nur Cloud geändert"
    );
    s.ki_richtlinie_setzen(&a).unwrap();
    let mut b = a.clone();
    b.eigene_domaenen = vec!["example.net".into()];
    s.ki_richtlinie_setzen(&b).unwrap();
    s.ki_richtlinie_setzen(&a).unwrap();
    assert!(s.ki_bestaetigung(&id).unwrap().is_none());
    assert!(!sperre().sichtbar(&s, &id));
}

/// Bestätigung und Domänenwechsel sind über die Schreibsperre der Datenbank geordnet: Eine
/// Bestätigung, die nach dem Wechsel festgeschrieben wird, prüft gegen die NEUEN Domänen.
#[test]
fn bestaetigung_nach_domaenenwechsel_prueft_gegen_die_neuen_domaenen() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    // Die Oberfläche hat noch den alten Stand („intern“ mit example.org).
    let alt = stand(&s, &sperre(), &id).unwrap();
    assert_eq!(alt.ergebnis.einstufung, Einstufung::Intern);
    let mut b = sperre().richtlinie().unwrap();
    b.eigene_domaenen = vec!["example.net".into()];
    s.ki_richtlinie_setzen(&b).unwrap();
    let e = bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &alt.ergebnis.fingerabdruck,
        &nachweis(),
        5_000,
    )
    .unwrap_err();
    // Der Fingerabdruck bindet die Domänen: der alte Stand gilt als veraltet.
    assert_eq!(e.to_string(), "ki_freigabe_veraltet");
    assert!(s.ki_bestaetigung(&id).unwrap().is_none());
}

/// Ein Fehler beim Schreiben des Spiegels ändert nichts: Bestätigungen und Spiegel bleiben.
#[test]
fn fehler_beim_spiegeln_laesst_alles_beim_alten() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    let r0 = rev(&s);
    s.get_connection()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER kaputt BEFORE UPDATE ON meeting_ai_policy
               BEGIN SELECT RAISE(ABORT, 'Speicherfehler'); END;",
        )
        .unwrap();
    let mut b = sperre().richtlinie().unwrap();
    b.eigene_domaenen = vec!["example.net".into()];
    assert!(s.ki_richtlinie_setzen(&b).is_err());
    assert!(s.ki_bestaetigung(&id).unwrap().is_some());
    assert_eq!(s.ki_stand().unwrap().richtlinie, sperre().richtlinie());
    assert_eq!(rev(&s), r0);
}

/// Ohne Spiegel (Datenbank vor dem ersten Start nach dem Update) keine Bestätigung.
#[test]
fn ohne_richtlinienspiegel_keine_bestaetigung() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    s.get_connection()
        .unwrap()
        .execute("DELETE FROM meeting_ai_policy", [])
        .unwrap();
    let e = bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "ki_freigabe_richtlinie_fehlt");
}

/// Abgleich: nur bei gleichem Spiegel gilt die Sperre, sonst keine Ausgabe.
#[test]
fn sperre_gilt_nur_mit_gleichem_spiegel() {
    let r = sperre().richtlinie().unwrap();
    assert_eq!(sperre().abgleichen(Some(&r)), Some(sperre()));
    assert_eq!(sperre().abgleichen(None), None);
    let mut anders = r.clone();
    anders.nur_lokal = true;
    assert_eq!(sperre().abgleichen(Some(&anders)), None);
}

/// Codex #583 Fixrunde 3: Upgrade einer belegten Datenbank vom Stand nach Migration 19 (Bestätigung
/// ohne Rechtsgrundlage, Termin, Verknüpfung) auf Migration 20. Die Daten bleiben, die alte
/// Bestätigung zählt aber nicht mehr (fail closed), bis sie mit Nachweis erneuert wird; Revision
/// und Trigger arbeiten ab dem Upgrade.
#[test]
fn upgrade_von_stand_19_behaelt_daten_und_sperrt_die_alte_bestaetigung() {
    use crate::managers::meetings::store::MIGRATIONS;
    let (dir, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    let pfad = dir.path().join("meetings.db");
    drop(s);
    // Stand 19 herstellen: alles aus Migration 20 entfernen, alte Bestätigung wie damals.
    {
        let conn = rusqlite::Connection::open(&pfad).unwrap();
        let trigger: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'ai_rev_%'",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(trigger.len(), 18);
        for t in &trigger {
            conn.execute_batch(&format!("DROP TRIGGER {t};")).unwrap();
        }
        conn.execute_batch(
            "DROP TABLE meeting_ai_revision;
             DROP TABLE meeting_ai_policy;
             ALTER TABLE meeting_ai_confirmations DROP COLUMN einwilligungen_belegt;
             ALTER TABLE meeting_ai_confirmations DROP COLUMN rechtsgrundlage;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meeting_ai_confirmations
               (meeting_id, einstufung, fingerabdruck, fp_version, informiert, confirmed_at)
             VALUES (?1, 'intern', ?2, ?3, 1, 4000)",
            params![id, fp, i64::from(FINGERABDRUCK_VERSION)],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", (MIGRATIONS.len() - 1) as i64)
            .unwrap();
    }
    let s = MeetingStore::open_at(&pfad).unwrap();
    let conn = s.get_connection().unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, MIGRATIONS.len() as i64);
    // Daten bleiben: Besprechung, Verknüpfung, Bestätigungszeile (ohne Rechtsgrundlage).
    let (zeilen, grundlage): (i64, Option<String>) = conn
        .query_row(
            "SELECT COUNT(*), MAX(rechtsgrundlage) FROM meeting_ai_confirmations WHERE meeting_id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((zeilen, grundlage), (1, None));
    assert!(s.get_meeting(&id).unwrap().is_some());
    // Ohne Spiegel keine Ausgabe; mit Spiegel zählt die alte Bestätigung trotzdem nicht.
    assert!(s.ki_stand().unwrap().richtlinie.is_none());
    s.ki_richtlinie_setzen(&sperre().richtlinie().unwrap())
        .unwrap();
    assert!(s.ki_bestaetigung(&id).unwrap().is_none());
    assert!(!sperre().sichtbar(&s, &id));
    // Revision und Trigger arbeiten: erneuern mit Nachweis erhöht sie und gibt frei.
    let vorher = rev(&s);
    bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &fp,
        &nachweis(),
        5_000,
    )
    .unwrap();
    assert!(rev(&s) > vorher);
    assert!(sperre().sichtbar(&s, &id));
}
