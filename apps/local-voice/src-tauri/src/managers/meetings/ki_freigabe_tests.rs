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
    (dir, s)
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
    assert_eq!(
        lage.teilnehmende,
        vec!["a@example.org", "Ohne Adresse", "?"]
    );
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
    let e = bestaetigen(&s, &sperre(), &id, Einstufung::Extern, "0", 5_000).unwrap_err();
    assert_eq!(e.to_string(), "ki_freigabe_veraltet");
    let e = bestaetigen(
        &s,
        &sperre(),
        &id,
        Einstufung::Intern,
        &st.ergebnis.fingerabdruck,
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
    bestaetigen(&s, &sperre(), &id, Einstufung::Extern, &fp, 5_000).unwrap();
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
    bestaetigen(&s, &sperre(), &id, Einstufung::Intern, &fp, 5_000).unwrap();
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
    bestaetigen(&s, &sperre(), &id, Einstufung::Intern, &fp, 5_000).unwrap();
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
    bestaetigen(&s, &sperre(), &id, Einstufung::Intern, &fp, 5_000).unwrap();
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
    bestaetigen(&s, &sperre(), &id, Einstufung::Intern, &fp, 5_000).unwrap();
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
            "INSERT INTO meeting_ai_confirmations VALUES (?1, 'irgendwas', 'x', 2, 1)",
            params![id],
        )
        .is_err());
}

#[test]
fn liste_ohne_abgleich_nach_dem_ende_ist_veraltet_und_gesperrt() {
    let (_d, s, id) = besprechung(intern_liste(), true);
    let fp = stand(&s, &sperre(), &id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(&s, &sperre(), &id, Einstufung::Intern, &fp, 5_000).unwrap();
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
    bestaetigen(&s, &sperre(), &id, Einstufung::Intern, &fp, 5_000).unwrap();
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
    bestaetigen(&s, &sperre(), &id, Einstufung::Intern, &fp, 5_000).unwrap();
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
