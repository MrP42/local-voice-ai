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
fn ad_hoc_freigabe_braucht_nachweis_und_gilt_nur_fuer_diese_aufnahme() {
    let (_d, s) = store_mit_quelle();
    let m = s.create_meeting("Spontan", MeetingSource::Live, Some(1)).unwrap();
    let andere = s.create_meeting("Andere", MeetingSource::Live, Some(1)).unwrap();
    let fp = stand(&s, &sperre(), &m.id).unwrap().ergebnis.fingerabdruck;
    assert!(!sperre().sichtbar(&s, &m.id));
    for nachweis in [
        ohne_information(),
        Freigabenachweis { rechtsgrundlage: None, ..nachweis() },
        Freigabenachweis {
            rechtsgrundlage: Some(Rechtsgrundlage::Einwilligung),
            einwilligungen_liegen_vor: false,
            ..nachweis()
        },
    ] {
        assert!(bestaetigen(&s, &sperre(), &m.id, Einstufung::Unbekannt, &fp, &nachweis, 5_000).is_err());
        assert!(s.ki_bestaetigung(&m.id).unwrap().is_none());
    }
    let vorher = rev(&s);
    let st = bestaetigen(&s, &sperre(), &m.id, Einstufung::Unbekannt, &fp, &nachweis(), 5_000).unwrap();
    assert!(st.entscheidung.erlaubt());
    assert!(rev(&s) > vorher);
    assert!(!sperre().sichtbar(&s, &andere.id));
    let lokal = KiSperre::Pruefen {
        kundenprofil_nur_lokal: true,
        cloud_bei_externen: false,
        eigene_domaenen: vec!["example.org".into()],
    };
    assert!(!lokal.sichtbar(&s, &m.id));
    let ev = termin(intern_liste());
    s.calendar_replace_events("src1", std::slice::from_ref(&ev), &meta(true)).unwrap();
    s.link_meeting_event(&m.id, &ev, "manual", 6_000).unwrap();
    assert!(!sperre().sichtbar(&s, &m.id));
    assert!(bestaetigen(&s, &sperre(), &m.id, Einstufung::Unbekannt, &fp, &nachweis(), 7_000).is_err());
    s.ki_bestaetigung_entfernen(&m.id).unwrap();
    assert!(s.ki_bestaetigung(&m.id).unwrap().is_none());
}

#[test]
fn ad_hoc_bestaetigung_bindet_den_transkriptstand() {
    use crate::managers::meetings::store::{StoredSegment, TranscriptDelta};
    let (_d, s) = store_mit_quelle();
    let m = s.create_meeting("A", MeetingSource::Live, Some(1)).unwrap();
    s.append_delta(&m.id, &TranscriptDelta {
        new_segments: vec![StoredSegment {
            segment_index: 0, text: "Synthetisch".into(), start_ms: 0, end_ms: 100,
            channel: 0, speaker_index: None, words: None,
        }],
    }).unwrap();
    let fp = stand(&s, &sperre(), &m.id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(&s, &sperre(), &m.id, Einstufung::Unbekannt, &fp, &nachweis(), 5_000).unwrap();
    assert!(sperre().sichtbar(&s, &m.id));
    s.update_segment_text(&m.id, 0, "Synthetische Korrektur").unwrap();
    assert!(!sperre().sichtbar(&s, &m.id));
    assert_eq!(bestaetigen(&s, &sperre(), &m.id, Einstufung::Unbekannt, &fp, &nachweis(), 6_000)
        .unwrap_err().to_string(), "ki_freigabe_veraltet");
    let neu = stand(&s, &sperre(), &m.id).unwrap().ergebnis.fingerabdruck;
    assert_ne!(fp, neu);
    bestaetigen(&s, &sperre(), &m.id, Einstufung::Unbekannt, &neu, &nachweis(), 7_000).unwrap();
    assert!(sperre().sichtbar(&s, &m.id));
}

#[test]
fn transkript_schreibwege_erhoehen_die_ki_revision() {
    use crate::managers::meetings::store::{SpeakerWrite, StoredSegment, TranscriptDelta};
    for art in ["anlegen", "ersetzen", "in_place", "delta", "text", "name", "sprecher", "merge", "unmerge", "leeren"] {
        let (_d, s) = store_mit_quelle();
        let m = s.create_meeting("A", MeetingSource::Live, Some(1)).unwrap();
        let segments = vec![StoredSegment {
            segment_index: 0, text: "Synthetisch".into(), start_ms: 0, end_ms: 100,
            channel: 0, speaker_index: Some(1), words: None,
        }];
        if art != "anlegen" {
            s.replace_segments(&m.id, &segments, "synthetic", "segment", 0).unwrap();
        }
        if matches!(art, "merge" | "unmerge") {
            s.set_speaker_name(&m.id, 0, 2, Some("Synthetisch zwei")).unwrap();
        }
        let merge_id = if art == "unmerge" {
            Some(s.merge_speakers(&m.id, 0, 1, 2, &[]).unwrap())
        } else {
            None
        };
        let vorher = rev(&s);
        let snapshot = s.transcript_snapshot(&m.id).unwrap();
        match art {
            "anlegen" | "ersetzen" => {
                s.replace_segments(&m.id, &segments, "synthetic", "segment", snapshot.revision).unwrap();
            }
            "in_place" => {
                s.update_segments_with_speakers(&m.id, &segments, snapshot.revision, false, &SpeakerWrite {
                    hints_json: "{}".into(), present: vec![(0, 1)], fresh: Default::default(),
                }).unwrap();
            }
            "delta" => { s.append_delta(&m.id, &TranscriptDelta { new_segments: segments }).unwrap(); }
            "text" => s.update_segment_text(&m.id, 0, "Synthetisch neu").unwrap(),
            "name" => { s.set_speaker_name(&m.id, 0, 1, Some("Synthetisch")).unwrap(); }
            "sprecher" => s.set_segment_speaker(&m.id, 0, snapshot.epoch, Some(2)).unwrap(),
            "merge" => { s.merge_speakers(&m.id, 0, 1, 2, &[]).unwrap(); }
            "unmerge" => s.unmerge_speakers(&m.id, merge_id.as_deref().unwrap()).unwrap(),
            "leeren" => s.clear_segments(&m.id).unwrap(),
            _ => unreachable!(),
        }
        assert!(rev(&s) > vorher, "{art}: KI-Abschlusspruefung muss den Wechsel erkennen");
    }
}

#[test]
fn fehlgeschlagener_ki_revisionshub_rollt_textaenderung_zurueck() {
    use crate::managers::meetings::store::{StoredSegment, TranscriptDelta};
    let (_d, s) = store_mit_quelle();
    let m = s.create_meeting("A", MeetingSource::Live, Some(1)).unwrap();
    s.append_delta(&m.id, &TranscriptDelta {
        new_segments: vec![StoredSegment {
            segment_index: 0, text: "Synthetisch".into(), start_ms: 0, end_ms: 100,
            channel: 0, speaker_index: None, words: None,
        }],
    }).unwrap();
    let vorher = rev(&s);
    let snapshot = s.transcript_snapshot(&m.id).unwrap();
    // Nur in der synthetischen Testdatenbank: Schreibfehler beim zweiten Schritt.
    s.get_connection().unwrap().execute_batch(
        "CREATE TRIGGER test_revision_fehler BEFORE UPDATE ON meeting_ai_revision
         BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;",
    ).unwrap();
    assert!(s.update_segment_text(&m.id, 0, "Synthetisch neu").is_err());
    assert_eq!(rev(&s), vorher);
    let nachher = s.transcript_snapshot(&m.id).unwrap();
    assert_eq!(nachher.revision, snapshot.revision);
    assert_eq!(nachher.segments[0].text, "Synthetisch");
}

#[test]
fn alte_ad_hoc_bestaetigung_ohne_aufnahmebindung_sperrt() {
    let (_d, s) = store_mit_quelle();
    let m = s.create_meeting("A", MeetingSource::Live, Some(1)).unwrap();
    let fp = stand(&s, &sperre(), &m.id).unwrap().ergebnis.fingerabdruck;
    bestaetigen(&s, &sperre(), &m.id, Einstufung::Unbekannt, &fp, &nachweis(), 5_000).unwrap();
    assert!(sperre().sichtbar(&s, &m.id));
    // Historischer Stand: gültige Schemaversion und Nachweis, aber nur Teilnehmerbindung.
    let alt = einstufen(&[], &["example.org".into()], false).fingerabdruck;
    s.get_connection().unwrap().execute(
        "UPDATE meeting_ai_confirmations SET fingerabdruck = ?1 WHERE meeting_id = ?2",
        params![alt, m.id],
    ).unwrap();
    assert!(s.ki_bestaetigung(&m.id).unwrap().is_none());
    assert!(!sperre().sichtbar(&s, &m.id));
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
    let dir = tempfile::tempdir().unwrap();
    let id = "synthetic-meeting";
    let ev = termin(intern_liste());
    let fp = einstufen(
        &["a@example.org".into(), "b@example.org".into()],
        &["example.org".into()],
        true,
    )
    .fingerabdruck;
    let pfad = dir.path().join("meetings.db");
    // Historischen Stand direkt aufbauen, ohne spaetere Tabellen zurueckzubauen.
    {
        let mut conn = rusqlite::Connection::open(&pfad).unwrap();
        rusqlite_migration::Migrations::new(MIGRATIONS[..20].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO meetings (id, title, status, source, created_at, updated_at)
             VALUES ('synthetic-meeting', 'Jour fixe', 'ready', 'live', 1, 1);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO calendar_sources
               (id, kind, label, has_attendee_data, last_ok_at, created_at, updated_at)
             VALUES ('src1', 'ics', 'Kalender', 1, ?1, 1000, 1000)",
            params![T1 + 3_600_000],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO calendar_events
               (key, source_id, uid, title, starts_at, ends_at, fetched_at)
             VALUES (?1, 'src1', 'u1', 'Jour fixe', ?2, ?3, ?4)",
            params![ev.key, ev.starts_at, ev.ends_at, T1 + 3_600_000],
        )
        .unwrap();
        for attendee in &ev.attendees {
            conn.execute(
                "INSERT INTO calendar_event_attendees (event_key, email, name) VALUES (?1, ?2, ?3)",
                params![ev.key, attendee.email, attendee.name],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO meeting_calendar_links
               (meeting_id, event_key, source_id, uid, event_start, event_title, linked_by, created_at)
             VALUES (?1, ?2, 'src1', 'u1', ?3, 'Jour fixe', 'manual', 3000)",
            params![id, ev.key, ev.starts_at],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meeting_ai_confirmations
               (meeting_id, einstufung, fingerabdruck, fp_version, informiert, confirmed_at)
             VALUES (?1, 'intern', ?2, ?3, 1, 4000)",
            params![id, fp, i64::from(FINGERABDRUCK_VERSION)],
        )
        .unwrap();
        let spaetere_tabellen: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
                 AND (name LIKE 'audiobook_%' OR name IN ('meeting_ai_revision', 'meeting_ai_policy'))",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(spaetere_tabellen, 0);
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
    assert_eq!(s.ki_teilnehmerlage(&id).unwrap().teilnehmende.len(), 2);
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

/// Gegenprobe: eine beim Rueckbau vergessene Claim-Tabelle verhindert das Upgrade.
#[test]
fn upgrade_von_stand_19_mit_spaeterer_claim_tabelle_scheitert() {
    use crate::managers::meetings::store::MIGRATIONS;
    let dir = tempfile::tempdir().unwrap();
    let pfad = dir.path().join("meetings.db");
    let mut conn = rusqlite::Connection::open(&pfad).unwrap();
    rusqlite_migration::Migrations::new(MIGRATIONS[..20].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    // Genau der Rest des alten Rueckbaus; SQLite erlaubt den noch fehlenden FK-Parent.
    conn.execute_batch(
        "CREATE TABLE audiobook_adoption (
           meeting_id TEXT PRIMARY KEY REFERENCES audiobook_control(meeting_id) ON DELETE CASCADE,
           book_id TEXT NOT NULL, token TEXT NOT NULL);",
    )
    .unwrap();
    drop(conn);
    let error = MeetingStore::open_at(&pfad).err().unwrap();
    assert!(error.to_string().contains("audiobook_adoption"));
}
