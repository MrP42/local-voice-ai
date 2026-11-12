use rusqlite::Connection;

use crate::consent::{neue_einwilligungen, person_schluessel, start_modus, StartModus};
use crate::invitation::{
    block_hash, einladung, einladung_aus, finde_block, finde_marke, marke, Gestaltung, BLOCK_KOPF,
};
use crate::model::*;
use crate::rules::{pruefen, uebergabe_erlaubt, LokaleZeit, Regeln, Stufe};
use crate::store::*;
use crate::sync::{abgleichen, verschiebung_bestaetigen, Beobachtung, Ereignis};

fn eingabe() -> PlanInput {
    PlanInput {
        title: "Abstimmung Posteingang".into(),
        goal_type: GoalType::Entscheiden,
        goal_text: "Wir entscheiden, ob der Pilot im Januar startet.".into(),
        expected_result: Some("Entscheidung mit Termin".into()),
        duration_min: 25,
        classification: "intern".into(),
        agenda: vec![
            AgendaItem {
                minutes: 5,
                topic: "Stand".into(),
                owner: Some("A. Beispiel".into()),
            },
            AgendaItem {
                minutes: 15,
                topic: "Optionen".into(),
                owner: None,
            },
            AgendaItem {
                minutes: 5,
                topic: "Entscheidung".into(),
                owner: None,
            },
        ],
        roles: Roles {
            moderation: Some("A. Beispiel".into()),
            ergebnissicherung: Some("B. Muster".into()),
            entscheidung: None,
        },
        materials: vec![Material {
            label: "Vorlage".into(),
            url: Some("https://intranet.example.org/vorlage".into()),
            due_date: Some("2026-11-20".into()),
        }],
        hints: Hints {
            weiterleiten_erlaubt: Some(false),
            barrierefreiheit: None,
            ergebnisfrist: Some("2026-11-30".into()),
            ablageort: None,
        },
        transcription_planned: true,
        minutes_template_id: None,
    }
}

fn codes(h: &[crate::rules::Hinweis]) -> Vec<&str> {
    h.iter().map(|h| h.code.as_str()).collect()
}

// ---------- Regeln ----------

#[test]
fn regeln_saubere_planung_hat_keine_warnung() {
    let h = pruefen(
        &eingabe(),
        Some(LokaleZeit {
            stunde: 10,
            minute: 0,
        }),
        &Regeln::default(),
    );
    assert!(h.iter().all(|h| h.stufe == Stufe::Info), "{h:?}");
    assert!(uebergabe_erlaubt(&h));
    assert!(codes(&h).contains(&"transkription_einwilligung"));
}

#[test]
fn regeln_titel_leer_blockiert() {
    let mut p = eingabe();
    p.title = "   ".into();
    let h = pruefen(&p, None, &Regeln::default());
    assert!(codes(&h).contains(&"titel_leer"));
    assert!(!uebergabe_erlaubt(&h));
}

#[test]
fn regeln_dauer_und_agenda() {
    let mut p = eingabe();
    p.duration_min = 20;
    let h = pruefen(&p, None, &Regeln::default());
    assert!(codes(&h).contains(&"dauer_unueblich"));
    assert!(codes(&h).contains(&"agenda_zu_lang"));
    assert!(uebergabe_erlaubt(&h));

    p.duration_min = 0;
    assert!(!uebergabe_erlaubt(&pruefen(&p, None, &Regeln::default())));
    p.duration_min = 9 * 60;
    assert!(codes(&pruefen(&p, None, &Regeln::default())).contains(&"dauer_zu_lang"));
}

#[test]
fn regeln_agenda_leer_und_leerer_punkt() {
    let mut p = eingabe();
    p.agenda.clear();
    assert!(codes(&pruefen(&p, None, &Regeln::default())).contains(&"agenda_leer"));
    p.agenda.push(AgendaItem {
        minutes: 0,
        topic: " ".into(),
        owner: None,
    });
    let h = pruefen(&p, None, &Regeln::default());
    assert!(codes(&h).contains(&"agenda_punkt_leer"));
    assert!(codes(&h).contains(&"agenda_zeitbox_null"));
    assert!(!uebergabe_erlaubt(&h));
}

#[test]
fn regeln_ziel_und_ergebnis() {
    let mut p = eingabe();
    p.goal_text = "".into();
    p.expected_result = Some("  ".into());
    let c = pruefen(&p, None, &Regeln::default());
    assert!(codes(&c).contains(&"ziel_fehlt"));
    assert!(codes(&c).contains(&"ergebnis_fehlt"));
}

#[test]
fn regeln_zeitrahmen() {
    let p = eingabe();
    let r = Regeln::default();
    assert!(codes(&pruefen(
        &p,
        Some(LokaleZeit {
            stunde: 5,
            minute: 30
        }),
        &r
    ))
    .contains(&"ausserhalb_rahmen"));
    // 19:40 + 25 Min. endet 20:05 -> ausserhalb, und krummer Beginn.
    let h = pruefen(
        &p,
        Some(LokaleZeit {
            stunde: 19,
            minute: 40,
        }),
        &r,
    );
    assert!(codes(&h).contains(&"ausserhalb_rahmen"));
    assert!(codes(&h).contains(&"beginn_krumm"));
    // 19:30 + 25 endet 19:55 -> im Rahmen.
    let h = pruefen(
        &p,
        Some(LokaleZeit {
            stunde: 19,
            minute: 30,
        }),
        &r,
    );
    assert!(!codes(&h).contains(&"ausserhalb_rahmen"));
}

// ---------- Einladung und Marke ----------

#[test]
fn einladung_enthaelt_alles_und_marke() {
    let e = einladung_aus("01HPLAN", &eingabe(), &Gestaltung::default());
    assert!(e.text.starts_with(BLOCK_KOPF));
    assert!(e.text.trim_end().ends_with("lva-plan:01HPLAN"));
    for teil in [
        "Abstimmung Posteingang",
        "Ziel (Entscheiden): Wir entscheiden",
        "Erwartetes Ergebnis: Entscheidung mit Termin",
        "1. Stand (5 Min.) – A. Beispiel",
        "Moderation: A. Beispiel",
        "Vorlage (bitte lesen bis 2026-11-20): https://intranet.example.org/vorlage",
        "Bitte nicht weiterleiten.",
        "nur, wenn alle Anwesenden zustimmen",
        "Klassifizierung: intern",
    ] {
        assert!(e.text.contains(teil), "fehlt {teil:?} in\n{}", e.text);
    }
    assert!(e.html.contains("lva-plan:01HPLAN"));
    assert!(e
        .html
        .contains("href=\"https://intranet.example.org/vorlage\""));
    assert_eq!(finde_marke(&e.text).as_deref(), Some("01HPLAN"));
    assert_eq!(finde_marke(&e.html).as_deref(), Some("01HPLAN"));
}

#[test]
fn einladung_escaped_html_und_verbietet_fremde_marke() {
    let mut p = eingabe();
    p.title = "<script>alert(1)</script> & Co".into();
    p.goal_text = "Ziel lva-plan:FREMD\nzweite Zeile".into();
    p.materials[0].url = Some("javascript:alert(1)".into());
    let e = einladung_aus("01HPLAN", &p, &Gestaltung::default());
    assert!(!e.html.contains("<script>"));
    assert!(e.html.contains("&lt;script&gt;"));
    assert!(!e.html.contains("javascript:"));
    // Nur die eigene Marke ist auffindbar, und der Inhalt ist einzeilig.
    assert_eq!(finde_marke(&e.text).as_deref(), Some("01HPLAN"));
    assert!(e.text.contains("Ziel lva-plan\u{2060}:FREMD zweite Zeile"));
}

#[test]
fn einladung_gestaltung_absender_und_farbe() {
    let g = Gestaltung {
        absender: Some("Fachbereich Digitalisierung".into()),
        akzentfarbe: "#123ABC".into(),
    };
    let e = einladung_aus("X1", &eingabe(), &g);
    assert!(e.text.contains("Fachbereich Digitalisierung"));
    assert!(e.html.contains("#123ABC"));
    let schlecht = Gestaltung {
        absender: None,
        akzentfarbe: "red;background:url(x)".into(),
    };
    let e = einladung_aus("X1", &eingabe(), &schlecht);
    assert!(!e.html.contains("url(x)"));
    assert!(e.html.contains(crate::invitation::AKZENT_NEUTRAL));
}

#[test]
fn block_hash_ignoriert_formatierung_aber_nicht_inhalt() {
    let e = einladung_aus("P1", &eingabe(), &Gestaltung::default());
    // Outlook-Text: andere Aufzaehlungszeichen, CRLF, Tabs.
    let outlook = e.text.replace("• ", "*\t").replace('\n', "\r\n");
    let fremd = format!("Einwahl: 123\r\n\r\n{outlook}\r\n\r\nGruesse");
    let block = finde_block(&fremd, "P1").expect("Block gefunden");
    assert_eq!(block_hash(&block), e.block_hash);
    let geaendert = fremd.replace("Optionen", "Varianten");
    let block = finde_block(&geaendert, "P1").unwrap();
    assert_ne!(block_hash(&block), e.block_hash);
}

#[test]
fn finde_block_ohne_kopf_oder_marke() {
    assert!(finde_block("nur lva-plan:P1", "P1").is_none());
    assert!(finde_block(&format!("{BLOCK_KOPF}\nohne Marke"), "P1").is_none());
    assert!(finde_marke("kein Bezug").is_none());
    assert!(finde_marke("lva-plan: ").is_none());
}

// ---------- Speicher ----------

fn db() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrieren(&c).unwrap();
    c
}

#[test]
fn store_anlegen_laden_aendern_loeschen() {
    let c = db();
    let p = plan_anlegen(&c, "P1", &eingabe(), 1000).unwrap();
    assert_eq!(p.status, PlanStatus::Entwurf);
    assert_eq!(p.agenda.len(), 3);
    assert_eq!(p.roles.moderation.as_deref(), Some("A. Beispiel"));
    assert!(p.transcription_planned);

    let mut neu = eingabe();
    neu.title = "Neu".into();
    neu.transcription_planned = false;
    let q = plan_aendern(&c, "P1", &neu, 2000).unwrap();
    assert_eq!(q.title, "Neu");
    assert!(!q.transcription_planned);
    assert_eq!(q.created_at, 1000);
    assert_eq!(q.updated_at, 2000);

    plan_status_setzen(&c, "P1", PlanStatus::Versendet, 3000).unwrap();
    assert_eq!(
        plan_aendern(&c, "P1", &neu, 4000).unwrap_err(),
        "plan_abgeschlossen"
    );

    plan_besprechung_setzen(&c, "P1", Some("M1"), 5000).unwrap();
    assert_eq!(
        plan_laden(&c, "P1").unwrap().unwrap().meeting_id.as_deref(),
        Some("M1")
    );

    plan_loeschen(&c, "P1", 6000).unwrap();
    assert!(plan_laden(&c, "P1").unwrap().is_none());
    assert!(plan_liste(&c).unwrap().is_empty());
    assert_eq!(
        plan_loeschen(&c, "P1", 7000).unwrap_err(),
        "plan_nicht_gefunden"
    );
}

#[test]
fn store_id_wird_geprueft() {
    let c = db();
    for id in ["", "a b", "x\"y", &"z".repeat(65)] {
        assert_eq!(
            plan_anlegen(&c, id, &eingabe(), 1).unwrap_err(),
            "plan_id_ungueltig"
        );
    }
}

#[test]
fn store_liste_sortiert() {
    let c = db();
    plan_anlegen(&c, "A", &eingabe(), 1).unwrap();
    plan_anlegen(&c, "B", &eingabe(), 2).unwrap();
    let ids: Vec<_> = plan_liste(&c).unwrap().into_iter().map(|p| p.id).collect();
    assert_eq!(ids, ["B", "A"]);
}

fn link(plan_id: &str) -> PlanLink {
    PlanLink {
        plan_id: plan_id.into(),
        adapter: AdapterKind::IcsMarker,
        source_id: Some("S1".into()),
        remote_id: None,
        ical_uid: Some("uid-1".into()),
        change_key: None,
        starts_at: Some(10_000),
        ends_at: Some(11_500),
        location: Some("Raum 1".into()),
        join_url: None,
        body_hash: None,
        last_seen_at: None,
        last_change_at: None,
        sync_state: SyncState::Ok,
    }
}

#[test]
fn store_links_und_einwilligungen() {
    let c = db();
    plan_anlegen(&c, "P1", &eingabe(), 1).unwrap();
    plan_anlegen(&c, "P2", &eingabe(), 1).unwrap();
    let mut l = link("P1");
    link_speichern(&c, &l).unwrap();
    l.sync_state = SyncState::Verschoben;
    l.starts_at = Some(20_000);
    link_speichern(&c, &l).unwrap();
    assert_eq!(link_laden(&c, "P1").unwrap().unwrap(), l);
    link_speichern(&c, &link("P2")).unwrap();
    plan_status_setzen(&c, "P2", PlanStatus::Protokolliert, 2).unwrap();
    let offen: Vec<_> = links_offen(&c)
        .unwrap()
        .into_iter()
        .map(|l| l.plan_id)
        .collect();
    assert_eq!(offen, ["P1"]);

    let mut e = Consent {
        plan_id: "P1".into(),
        person: "a@example.org".into(),
        state: ConsentState::Offen,
        channel: None,
        recorded_at: None,
    };
    einwilligung_setzen(&c, &e).unwrap();
    e.state = ConsentState::Zugestimmt;
    e.channel = Some(ConsentChannel::MuendlichBeiStart);
    e.recorded_at = Some(5);
    einwilligung_setzen(&c, &e).unwrap();
    assert_eq!(einwilligungen(&c, "P1").unwrap(), vec![e]);

    // Ohne Planung keine Verknuepfung (Fremdschluessel).
    assert!(link_speichern(&c, &link("FEHLT")).is_err());
}

#[test]
fn migration_ist_wiederholbar() {
    let c = db();
    migrieren(&c).unwrap();
}

// ---------- Abgleich ----------

fn beob(start: i64, beschreibung: Option<String>) -> Beobachtung {
    Beobachtung {
        remote_id: None,
        ical_uid: Some("uid-1".into()),
        change_key: None,
        starts_at: start,
        ends_at: start + 1500,
        location: Some("Raum 1".into()),
        join_url: None,
        cancelled: false,
        description: beschreibung,
        attendees: vec!["a@example.org".into()],
    }
}

#[test]
fn sync_erstverknuepfung() {
    let mut l = link("P1");
    l.starts_at = None;
    l.ends_at = None;
    let a = abgleichen(PlanStatus::Uebergeben, &l, Some(&beob(10_000, None)), 99);
    assert_eq!(a.plan_status, Some(PlanStatus::Verknuepft));
    assert_eq!(a.link.sync_state, SyncState::Ok);
    assert_eq!(a.ereignisse, vec![Ereignis::Verknuepft]);
    assert!(a.schreiben_erlaubt);
    assert_eq!(a.link.last_seen_at, Some(99));
}

#[test]
fn sync_unveraendert_ist_ruhig() {
    let a = abgleichen(
        PlanStatus::Verknuepft,
        &link("P1"),
        Some(&beob(10_000, None)),
        99,
    );
    assert!(a.ereignisse.is_empty());
    assert_eq!(a.plan_status, None);
    assert_eq!(a.link.last_change_at, None);
}

#[test]
fn sync_verschiebung_bleibt_bis_bestaetigt() {
    let a = abgleichen(
        PlanStatus::Verknuepft,
        &link("P1"),
        Some(&beob(50_000, None)),
        99,
    );
    assert_eq!(a.link.sync_state, SyncState::Verschoben);
    assert_eq!(a.link.starts_at, Some(50_000));
    assert_eq!(a.plan_status, Some(PlanStatus::Verschoben));
    assert_eq!(
        a.ereignisse,
        vec![Ereignis::Verschoben {
            alt_beginn: Some(10_000),
            neu_beginn: 50_000
        }]
    );
    // Naechster Tick ohne Aenderung: bleibt verschoben.
    let b = abgleichen(
        PlanStatus::Verschoben,
        &a.link,
        Some(&beob(50_000, None)),
        100,
    );
    assert_eq!(b.link.sync_state, SyncState::Verschoben);
    assert!(b.ereignisse.is_empty());
    // Bestaetigt -> ok, Planung wieder verknuepft.
    let ok = verschiebung_bestaetigen(&b.link);
    let c = abgleichen(PlanStatus::Verschoben, &ok, Some(&beob(50_000, None)), 101);
    assert_eq!(c.link.sync_state, SyncState::Ok);
    assert_eq!(c.plan_status, Some(PlanStatus::Verknuepft));
}

#[test]
fn sync_absage_und_wiederherstellung() {
    let mut b = beob(10_000, None);
    b.cancelled = true;
    let a = abgleichen(PlanStatus::Verknuepft, &link("P1"), Some(&b), 1);
    assert_eq!(a.link.sync_state, SyncState::Abgesagt);
    assert_eq!(a.plan_status, Some(PlanStatus::Abgesagt));
    assert!(a.ereignisse.contains(&Ereignis::Abgesagt));
    assert!(!a.schreiben_erlaubt);
    let w = abgleichen(PlanStatus::Abgesagt, &a.link, Some(&beob(10_000, None)), 2);
    assert!(w.ereignisse.contains(&Ereignis::Wiederhergestellt));
    assert_eq!(w.plan_status, Some(PlanStatus::Verknuepft));
}

#[test]
fn sync_abgeschlossene_planung_bleibt() {
    let mut b = beob(10_000, None);
    b.cancelled = true;
    let a = abgleichen(PlanStatus::Protokolliert, &link("P1"), Some(&b), 1);
    assert_eq!(a.plan_status, None);
}

#[test]
fn sync_geloescht_oder_marke_weg_ist_verwaist() {
    let a = abgleichen(PlanStatus::Verknuepft, &link("P1"), None, 1);
    assert_eq!(a.link.sync_state, SyncState::Verwaist);
    assert_eq!(a.ereignisse, vec![Ereignis::Verwaist]);
    assert!(!a.schreiben_erlaubt);

    let b = abgleichen(
        PlanStatus::Verknuepft,
        &link("P1"),
        Some(&beob(10_000, Some("Text ohne Marke".into()))),
        1,
    );
    assert_eq!(b.link.sync_state, SyncState::Verwaist);
    // Fremde Marke zaehlt nicht.
    let c = abgleichen(
        PlanStatus::Verknuepft,
        &link("P1"),
        Some(&beob(10_000, Some(marke("P2")))),
        1,
    );
    assert_eq!(c.link.sync_state, SyncState::Verwaist);
}

#[test]
fn sync_konflikt_wenn_block_geaendert() {
    let c = db();
    let p = plan_anlegen(&c, "P1", &eingabe(), 1).unwrap();
    let e = einladung(&p, &Gestaltung::default());
    let mut l = link("P1");
    l.body_hash = Some(e.block_hash.clone());

    let gleich = format!("Einwahl\n{}\nGruss", e.text);
    let a = abgleichen(
        PlanStatus::Verknuepft,
        &l,
        Some(&beob(10_000, Some(gleich))),
        1,
    );
    assert_eq!(a.link.sync_state, SyncState::Ok);
    assert!(a.schreiben_erlaubt);

    let geaendert = e.text.replace("Optionen", "Varianten");
    let b = abgleichen(
        PlanStatus::Verknuepft,
        &l,
        Some(&beob(10_000, Some(geaendert))),
        2,
    );
    assert_eq!(b.link.sync_state, SyncState::Konflikt);
    assert!(b.ereignisse.contains(&Ereignis::Konflikt));
    assert!(!b.schreiben_erlaubt);
    // Planung bleibt verknuepft, nur der Block ist gesperrt.
    assert_eq!(b.plan_status, None);

    // Block zurueckgesetzt -> aufgeloest.
    let r = abgleichen(
        PlanStatus::Verknuepft,
        &b.link,
        Some(&beob(10_000, Some(e.text.clone()))),
        3,
    );
    assert_eq!(r.link.sync_state, SyncState::Ok);
    assert!(r.ereignisse.contains(&Ereignis::KonfliktAufgeloest));
}

// ---------- Einwilligung ----------

#[test]
fn einwilligung_person_schluessel() {
    assert_eq!(
        person_schluessel(" MAILTO:A.B@Example.org "),
        "a.b@example.org"
    );
    assert_eq!(person_schluessel("mailto:x@y.z"), "x@y.z");
}

#[test]
fn einwilligung_neue_teilnehmende_offen() {
    let bestehend = vec![Consent {
        plan_id: "P1".into(),
        person: "a@example.org".into(),
        state: ConsentState::Zugestimmt,
        channel: None,
        recorded_at: None,
    }];
    let neu = neue_einwilligungen(
        "P1",
        &bestehend,
        &[
            "A@example.org".into(),
            "b@example.org".into(),
            "mailto:B@example.org".into(),
            " ".into(),
        ],
    );
    assert_eq!(neu.len(), 1);
    assert_eq!(neu[0].person, "b@example.org");
    assert_eq!(neu[0].state, ConsentState::Offen);
}

fn zu(p: &str, s: ConsentState) -> Consent {
    Consent {
        plan_id: "P1".into(),
        person: p.into(),
        state: s,
        channel: None,
        recorded_at: None,
    }
}

#[test]
fn einwilligung_startmodus() {
    let alle = vec![
        zu("a@example.org", ConsentState::Zugestimmt),
        zu("b@example.org", ConsentState::Zugestimmt),
    ];
    let anw = vec!["a@example.org".to_string(), "B@example.org".to_string()];
    assert_eq!(start_modus(true, &anw, &alle), StartModus::MitTranskription);

    // Nicht geplant -> nur Notizen.
    assert!(matches!(
        start_modus(false, &anw, &alle),
        StartModus::NurNotizen {
            nicht_geplant: true,
            ..
        }
    ));
    // Niemand erfasst -> keine Transkription.
    assert!(matches!(
        start_modus(true, &[], &alle),
        StartModus::NurNotizen {
            nicht_geplant: false,
            ..
        }
    ));
    // Eine Person ohne Eintrag, eine mit Ablehnung.
    let gemischt = vec![
        zu("a@example.org", ConsentState::Zugestimmt),
        zu("c@example.org", ConsentState::Abgelehnt),
    ];
    let anw = vec![
        "a@example.org".to_string(),
        "b@example.org".to_string(),
        "c@example.org".to_string(),
    ];
    assert_eq!(
        start_modus(true, &anw, &gemischt),
        StartModus::NurNotizen {
            nicht_geplant: false,
            offen: vec!["b@example.org".into()],
            abgelehnt: vec!["c@example.org".into()],
        }
    );
}
