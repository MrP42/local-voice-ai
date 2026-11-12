use rusqlite::Connection;

use crate::consent::{neue_einwilligungen, person_schluessel, start_modus, StartModus};
use crate::invitation::{
    block_hash, einladung, einladung_aus, finde_block, finde_marke, marke, Gestaltung, BLOCK_KOPF,
};
use crate::model::*;
use crate::rules::{pruefen, uebergabe_erlaubt, LokaleZeit, Regeln, Stufe};
use crate::store::*;
use crate::sync::{abgleichen, verschiebung_bestaetigen, Beobachtung, Ereignis, Feld};

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
    let p = plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1000).unwrap();
    assert_eq!(p.status, PlanStatus::Entwurf);
    assert_eq!(p.agenda.len(), 3);
    assert_eq!(p.roles.moderation.as_deref(), Some("A. Beispiel"));
    assert!(p.transcription_planned);

    let mut neu = eingabe();
    neu.title = "Neu".into();
    neu.transcription_planned = false;
    let q = plan_aendern(&c, "P1", &neu, &Regeln::default(), 2000).unwrap();
    assert_eq!(q.title, "Neu");
    assert!(!q.transcription_planned);
    assert_eq!(q.created_at, 1000);
    assert_eq!(q.updated_at, 2000);

    plan_status_setzen(&c, "P1", PlanStatus::Versendet, 3000).unwrap();
    assert_eq!(
        plan_aendern(&c, "P1", &neu, &Regeln::default(), 4000).unwrap_err(),
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
            plan_anlegen(&c, id, &eingabe(), &Regeln::default(), 1).unwrap_err(),
            "plan_id_ungueltig"
        );
    }
}

#[test]
fn store_liste_sortiert() {
    let c = db();
    plan_anlegen(&c, "A", &eingabe(), &Regeln::default(), 1).unwrap();
    plan_anlegen(&c, "B", &eingabe(), &Regeln::default(), 2).unwrap();
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
        verschiebung_offen: false,
        block_konflikt: false,
        abgesagt: false,
        instanz: None,
        fremd_beobachtet: false,
    }
}

#[test]
fn store_links_und_einwilligungen() {
    let c = db();
    plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    plan_anlegen(&c, "P2", &eingabe(), &Regeln::default(), 1).unwrap();
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
    einwilligung_setzen(&c, &e, 9).unwrap();
    e.state = ConsentState::Zugestimmt;
    e.channel = Some(ConsentChannel::MuendlichBeiStart);
    e.recorded_at = Some(5);
    einwilligung_setzen(&c, &e, 9).unwrap();
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
        instanz: None,
        starts_at: start,
        ends_at: start + 1500,
        location: Feld::Wert("Raum 1".into()),
        join_url: Feld::Leer,
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
    // Adapter C schreibt nie (Konzept 3.4).
    assert!(!a.schreiben_erlaubt);
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
    let p = plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    let e = einladung(&p, &Gestaltung::default());
    let mut l = link("P1");
    l.body_hash = Some(e.block_hash.clone());
    l.adapter = AdapterKind::Graph;

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
        channel: Some(ConsentChannel::MuendlichBeiStart),
        recorded_at: Some(1),
    }
}

#[test]
fn einwilligung_startmodus() {
    let alle = vec![
        zu("a@example.org", ConsentState::Zugestimmt),
        zu("b@example.org", ConsentState::Zugestimmt),
    ];
    let anw = vec!["a@example.org".to_string(), "B@example.org".to_string()];
    assert_eq!(
        start_modus("P1", true, &anw, &alle),
        StartModus::MitTranskription
    );

    // Nicht geplant -> nur Notizen.
    assert!(matches!(
        start_modus("P1", false, &anw, &alle),
        StartModus::NurNotizen {
            nicht_geplant: true,
            ..
        }
    ));
    // Niemand erfasst -> keine Transkription.
    assert!(matches!(
        start_modus("P1", true, &[], &alle),
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
        start_modus("P1", true, &anw, &gemischt),
        StartModus::NurNotizen {
            nicht_geplant: false,
            offen: vec!["b@example.org".into()],
            abgelehnt: vec!["c@example.org".into()],
        }
    );
}

// ---------- Fixrunde Review ----------

#[test]
fn marke_in_url_wird_nicht_als_marke_gelesen() {
    let mut p = eingabe();
    p.materials[0].url = Some("https://x.example.org/lva-plan:abc".into());
    let e = einladung_aus("P1", &p, &Gestaltung::default());
    assert_eq!(finde_marke(&e.text).as_deref(), Some("P1"));
    assert_eq!(finde_marke(&e.html).as_deref(), Some("P1"));
    assert!(!e
        .html
        .contains("href=\"https://x.example.org/lva-plan:abc\""));
    // Auch eine fremde URL im Outlook-Teil des Termins zaehlt nicht.
    assert_eq!(
        finde_marke(&format!(
            "siehe https://x/lva-plan:abc\n{BLOCK_KOPF}\nlva-plan:P1"
        ))
        .as_deref(),
        Some("P1")
    );
}

#[test]
fn konflikt_ereignis_nur_einmal_auch_bei_absage() {
    let c = db();
    let p = plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    let e = einladung(&p, &Gestaltung::default());
    let mut l = link("P1");
    l.body_hash = Some(e.block_hash.clone());
    l.adapter = AdapterKind::Graph;
    let mut b = beob(10_000, Some(e.text.replace("Optionen", "Varianten")));
    b.cancelled = true;
    let a1 = abgleichen(PlanStatus::Verknuepft, &l, Some(&b), 1);
    assert_eq!(a1.link.sync_state, SyncState::Abgesagt);
    assert!(a1.ereignisse.contains(&Ereignis::Konflikt));
    let a2 = abgleichen(PlanStatus::Abgesagt, &a1.link, Some(&b), 2);
    assert!(a2.ereignisse.is_empty(), "{:?}", a2.ereignisse);
}

#[test]
fn verschiebung_ueberlebt_konflikt() {
    let c = db();
    let p = plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    let e = einladung(&p, &Gestaltung::default());
    let mut l = link("P1");
    l.body_hash = Some(e.block_hash.clone());
    l.adapter = AdapterKind::Graph;
    // Verschoben und Block geaendert zugleich.
    let geaendert = e.text.replace("Optionen", "Varianten");
    let a = abgleichen(
        PlanStatus::Verknuepft,
        &l,
        Some(&beob(50_000, Some(geaendert))),
        1,
    );
    assert_eq!(a.link.sync_state, SyncState::Konflikt);
    assert!(a.link.verschiebung_offen);
    assert_eq!(a.plan_status, Some(PlanStatus::Verschoben));
    // Konflikt aufgeloest -> Verschiebung weiter sichtbar.
    let b = abgleichen(
        PlanStatus::Verschoben,
        &a.link,
        Some(&beob(50_000, Some(e.text.clone()))),
        2,
    );
    assert_eq!(b.link.sync_state, SyncState::Verschoben);
    assert!(b.ereignisse.contains(&Ereignis::KonfliktAufgeloest));
}

#[test]
fn rueckkehr_aus_verwaist_wird_gemeldet() {
    let v = abgleichen(PlanStatus::Verknuepft, &link("P1"), None, 1);
    let w = abgleichen(
        PlanStatus::Verknuepft,
        &v.link,
        Some(&beob(10_000, None)),
        2,
    );
    assert!(w.ereignisse.contains(&Ereignis::Wiederverknuepft));
    assert_eq!(w.link.sync_state, SyncState::Ok);
}

#[test]
fn regeln_ueberlauf_ohne_panik() {
    let mut p = eingabe();
    p.duration_min = u32::MAX;
    p.agenda[0].minutes = u32::MAX;
    p.agenda[1].minutes = u32::MAX;
    let h = pruefen(
        &p,
        Some(LokaleZeit {
            stunde: 10,
            minute: 0,
        }),
        &Regeln::default(),
    );
    assert!(codes(&h).contains(&"dauer_zu_lang"));
    let mut q = eingabe();
    q.agenda[0].minutes = u32::MAX;
    q.agenda[1].minutes = u32::MAX;
    assert!(codes(&pruefen(&q, None, &Regeln::default())).contains(&"agenda_zu_lang"));
}

#[test]
fn store_erzwingt_regeln_und_status() {
    let c = db();
    let mut leer = eingabe();
    leer.title = " ".into();
    assert_eq!(
        plan_anlegen(&c, "P1", &leer, &Regeln::default(), 1).unwrap_err(),
        "titel_leer"
    );
    plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    let mut null = eingabe();
    null.duration_min = 0;
    assert_eq!(
        plan_aendern(&c, "P1", &null, &Regeln::default(), 2).unwrap_err(),
        "dauer_null"
    );

    plan_status_setzen(&c, "P1", PlanStatus::Durchgefuehrt, 3).unwrap();
    assert_eq!(
        plan_aendern(&c, "P1", &eingabe(), &Regeln::default(), 4).unwrap_err(),
        "plan_abgeschlossen"
    );
    assert_eq!(
        plan_status_setzen(&c, "P1", PlanStatus::Entwurf, 5).unwrap_err(),
        "status_uebergang_ungueltig"
    );
    plan_status_setzen(&c, "P1", PlanStatus::Protokolliert, 6).unwrap();
    plan_status_setzen(&c, "P1", PlanStatus::Versendet, 7).unwrap();
    assert!(plan_status_setzen(&c, "P1", PlanStatus::Protokolliert, 8).is_err());
}

#[test]
fn store_prueft_planung_und_normalisiert_person() {
    let c = db();
    plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    plan_anlegen(&c, "P2", &eingabe(), &Regeln::default(), 1).unwrap();
    let e = Consent {
        plan_id: "P1".into(),
        person: "Anna Beispiel <Anna@Example.org>".into(),
        state: ConsentState::Zugestimmt,
        channel: Some(ConsentChannel::MuendlichBeiStart),
        recorded_at: Some(1),
    };
    einwilligung_setzen(&c, &e, 9).unwrap();
    let alle = einwilligungen(&c, "P1").unwrap();
    assert_eq!(alle[0].person, "anna@example.org");
    assert_eq!(
        start_modus("P1", true, &["ANNA@example.org".into()], &alle),
        StartModus::MitTranskription
    );
    let mut fehlt = e.clone();
    fehlt.plan_id = "FEHLT".into();
    assert_eq!(
        einwilligung_setzen(&c, &fehlt, 9).unwrap_err(),
        "plan_nicht_gefunden"
    );

    // Ein Termin gehoert hoechstens einer Planung.
    let mut l1 = link("P1");
    l1.remote_id = Some("R1".into());
    link_speichern(&c, &l1).unwrap();
    let mut l2 = link("P2");
    l2.remote_id = Some("R1".into());
    assert!(link_speichern(&c, &l2).is_err());
}

// ---------- Fixrunde Codex (lva#535) ----------

#[test]
fn codex1_einwilligung_gilt_nur_je_planung_und_sperrt_bei_widerspruch() {
    let mut fremd = zu("a@example.org", ConsentState::Zugestimmt);
    fremd.plan_id = "P2".into();
    let anw = vec!["a@example.org".to_string()];
    // Zustimmung aus P2 erlaubt keinen Start von P1.
    assert!(matches!(
        start_modus("P1", true, &anw, &[fremd.clone()]),
        StartModus::NurNotizen { .. }
    ));
    // ... und legt in P1 trotzdem einen offenen Eintrag an.
    assert_eq!(neue_einwilligungen("P1", &[fremd], &anw).len(), 1);
    // Widerspruch in P1: Ablehnung gewinnt, unabhaengig von der Reihenfolge.
    for liste in [
        vec![
            zu("a@example.org", ConsentState::Zugestimmt),
            zu("A@example.org", ConsentState::Abgelehnt),
        ],
        vec![
            zu("a@example.org", ConsentState::Abgelehnt),
            zu("a@example.org", ConsentState::Zugestimmt),
        ],
    ] {
        assert_eq!(
            start_modus("P1", true, &anw, &liste),
            StartModus::NurNotizen {
                nicht_geplant: false,
                offen: vec![],
                abgelehnt: vec!["a@example.org".into()],
            }
        );
    }
    // Zustimmung plus offener Eintrag: offen.
    let gemischt = vec![
        zu("a@example.org", ConsentState::Zugestimmt),
        zu("a@example.org", ConsentState::Offen),
    ];
    assert!(matches!(
        start_modus("P1", true, &anw, &gemischt),
        StartModus::NurNotizen { .. }
    ));
}

#[test]
fn codex1_zustimmung_ohne_nachweis_zaehlt_nicht() {
    let mut ohne = zu("a@example.org", ConsentState::Zugestimmt);
    ohne.recorded_at = None;
    assert!(matches!(
        start_modus("P1", true, &["a@example.org".into()], &[ohne.clone()]),
        StartModus::NurNotizen { .. }
    ));
    let c = db();
    plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    assert_eq!(
        einwilligung_setzen(&c, &ohne, 2).unwrap_err(),
        "einwilligung_nachweis_fehlt"
    );
}

#[test]
fn codex2_unbekannte_anwesende_sperren() {
    let alle = vec![zu("a@example.org", ConsentState::Zugestimmt)];
    assert_eq!(
        start_modus("P1", true, &["a@example.org".into(), " ".into()], &alle),
        StartModus::NurNotizen {
            nicht_geplant: false,
            offen: vec![crate::consent::UNBEKANNT.into()],
            abgelehnt: vec![],
        }
    );
}

fn graph_link_mit_block(c: &Connection) -> (PlanLink, String) {
    let p = plan_anlegen(c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    let e = einladung(&p, &Gestaltung::default());
    let mut l = link("P1");
    l.adapter = AdapterKind::Graph;
    l.body_hash = Some(e.block_hash.clone());
    (l, e.text)
}

#[test]
fn codex3_ics_adapter_schreibt_nie() {
    let c = db();
    let (mut l, text) = graph_link_mit_block(&c);
    l.adapter = AdapterKind::IcsMarker;
    let a = abgleichen(
        PlanStatus::Verknuepft,
        &l,
        Some(&beob(10_000, Some(text))),
        1,
    );
    assert_eq!(a.link.sync_state, SyncState::Ok);
    assert!(!a.schreiben_erlaubt);
    assert!(!crate::sync::schreibender_adapter(AdapterKind::IcsMarker));
}

#[test]
fn codex4_ohne_geprueften_stand_keine_schreibfreigabe() {
    let c = db();
    let (l, text) = graph_link_mit_block(&c);
    // Kein Termintext: nicht pruefbar.
    let a = abgleichen(PlanStatus::Verknuepft, &l, Some(&beob(10_000, None)), 1);
    assert!(!a.schreiben_erlaubt);
    // Keine gespeicherte Pruefsumme: nicht pruefbar.
    let mut ohne = l.clone();
    ohne.body_hash = None;
    let b = abgleichen(
        PlanStatus::Verknuepft,
        &ohne,
        Some(&beob(10_000, Some(text.clone()))),
        1,
    );
    assert!(!b.schreiben_erlaubt);
    // Geprueft und gleich: erlaubt.
    let g = abgleichen(
        PlanStatus::Verknuepft,
        &l,
        Some(&beob(10_000, Some(text))),
        1,
    );
    assert!(g.schreiben_erlaubt);
}

#[test]
fn codex5_pruefsumme_sieht_inhalt_aber_nicht_formatierung() {
    let basis =
        "— Besprechungsplanung —\n• Dauer 1,5 Stunden\n• https://x.example.org/a-b\nlva-plan:P1";
    let h = block_hash(basis);
    for anders in [
        basis.replace("1,5", "15"),
        basis.replace("a-b", "ab"),
        basis.replace("Dauer", "dauer"),
        basis.replace("Stunden", "Stunden."),
        // Nur echte Listenzeichen sind Formatierung, `-` und `o` sind Inhalt.
        basis.replace("• ", "- "),
        basis.replace("• Dauer", "Dauer"),
    ] {
        assert_ne!(block_hash(&anders), h, "{anders}");
    }
    for gleich in [
        basis.replace('\n', "\r\n"),
        basis.replace("• ", "*\t"),
        basis.replace("• ", "· "),
        basis.replace("\n", "\n\n"),
        basis.replace("Dauer 1,5", "Dauer   1,5"),
    ] {
        assert_eq!(block_hash(&gleich), h, "{gleich:?}");
    }
}

#[test]
fn codex6_unbekannter_ort_loescht_nichts() {
    let mut b = beob(10_000, None);
    b.location = Feld::Unbekannt;
    b.join_url = Feld::Unbekannt;
    let mut l = link("P1");
    l.join_url = Some("https://meet.example.org/1".into());
    let a = abgleichen(PlanStatus::Verknuepft, &l, Some(&b), 1);
    assert_eq!(a.link.location.as_deref(), Some("Raum 1"));
    assert_eq!(
        a.link.join_url.as_deref(),
        Some("https://meet.example.org/1")
    );
    assert!(!a.ereignisse.contains(&Ereignis::OrtGeaendert));
    // Ausdruecklich leer: geloescht, mit Ereignis.
    b.location = Feld::Leer;
    let c = abgleichen(PlanStatus::Verknuepft, &l, Some(&b), 2);
    assert_eq!(c.link.location, None);
    assert!(c.ereignisse.contains(&Ereignis::OrtGeaendert));
}

#[test]
fn codex9_absage_ueberlebt_verwaist() {
    let mut abg = beob(10_000, None);
    abg.cancelled = true;
    let a = abgleichen(PlanStatus::Verknuepft, &link("P1"), Some(&abg), 1);
    assert!(a.link.abgesagt);
    let v = abgleichen(PlanStatus::Abgesagt, &a.link, None, 2);
    assert!(v.link.abgesagt);
    // Weiter abgesagt: kein neues Absage-Ereignis.
    let w = abgleichen(PlanStatus::Abgesagt, &v.link, Some(&abg), 3);
    assert!(!w.ereignisse.contains(&Ereignis::Abgesagt));
    assert_eq!(w.link.sync_state, SyncState::Abgesagt);
    // Wiederhergestellt nach verwaist: gemeldet.
    let r = abgleichen(PlanStatus::Abgesagt, &v.link, Some(&beob(10_000, None)), 4);
    assert!(r.ereignisse.contains(&Ereignis::Wiederhergestellt));
    assert_eq!(r.plan_status, Some(PlanStatus::Verknuepft));
}

#[test]
fn codex10_beschaedigte_daten_werden_gemeldet() {
    let c = db();
    plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    c.execute("UPDATE meeting_plans SET agenda_json = '{kaputt'", [])
        .unwrap();
    assert_eq!(plan_laden(&c, "P1").unwrap_err(), "daten_beschaedigt");
    c.execute(
        "UPDATE meeting_plans SET agenda_json = '[]', status = 'irgendwas'",
        [],
    )
    .unwrap();
    assert_eq!(plan_laden(&c, "P1").unwrap_err(), "daten_beschaedigt");
    c.execute("UPDATE meeting_plans SET status = 'entwurf'", [])
        .unwrap();
    link_speichern(&c, &link("P1")).unwrap();
    c.execute("UPDATE meeting_plan_links SET adapter = 'fax'", [])
        .unwrap();
    assert_eq!(link_laden(&c, "P1").unwrap_err(), "daten_beschaedigt");
}

#[test]
fn codex8_einwilligungsverlauf_bleibt_erhalten() {
    let c = db();
    plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    let mut e = zu("a@example.org", ConsentState::Zugestimmt);
    einwilligung_setzen(&c, &e, 10).unwrap();
    e.state = ConsentState::Abgelehnt;
    e.recorded_at = Some(20);
    einwilligung_setzen(&c, &e, 20).unwrap();
    let v = einwilligungs_verlauf(&c, "P1").unwrap();
    assert_eq!(v.len(), 2);
    assert_eq!(v[0].0.state, ConsentState::Zugestimmt);
    assert_eq!(v[1].0.state, ConsentState::Abgelehnt);
    assert_eq!(
        einwilligungen(&c, "P1").unwrap()[0].state,
        ConsentState::Abgelehnt
    );
}

#[test]
fn codex12_uhrzeit_regeln_und_instanz() {
    let h = pruefen(
        &eingabe(),
        Some(LokaleZeit {
            stunde: 24,
            minute: 0,
        }),
        &Regeln::default(),
    );
    assert!(codes(&h).contains(&"beginn_ungueltig"));
    assert!(!uebergabe_erlaubt(&h));
    // Store nutzt die uebergebenen Regeln.
    let c = db();
    let streng = Regeln {
        max_dauer_min: 20,
        ..Regeln::default()
    };
    assert_eq!(
        plan_anlegen(&c, "P1", &eingabe(), &streng, 1).unwrap_err(),
        "dauer_zu_lang"
    );
    // Serieninstanz wird gespeichert.
    plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    let mut b = beob(10_000, None);
    b.instanz = Some(9_000);
    // Erstverknuepfung (Zeiten noch unbekannt) uebernimmt die Instanz.
    let mut neu = link("P1");
    neu.starts_at = None;
    neu.ends_at = None;
    let a = abgleichen(PlanStatus::Verknuepft, &neu, Some(&b), 1);
    link_speichern(&c, &a.link).unwrap();
    assert_eq!(link_laden(&c, "P1").unwrap().unwrap().instanz, Some(9_000));
}

#[test]
fn abgeschlossene_planung_bleibt_auch_im_link_ruhig() {
    let mut b = beob(50_000, None);
    b.cancelled = true;
    let a = abgleichen(PlanStatus::Versendet, &link("P1"), Some(&b), 1);
    assert_eq!(a.plan_status, None);
    // Der Link spiegelt den Termin weiter, der Planstatus bleibt.
    assert!(a.link.abgesagt);
}

// ---------- Fixrunde 2 Codex (lva#535) ----------

#[test]
fn codex2b_anzeigenamen_sind_keine_identitaet() {
    use crate::consent::{identitaet, UNBEKANNT};
    assert_eq!(identitaet("Alex"), None);
    assert_eq!(identitaet("alex@"), None);
    assert_eq!(identitaet("a@b"), None);
    assert_eq!(identitaet("a b@example.org"), None);
    assert_eq!(
        identitaet("Alex <ALEX@example.org>").as_deref(),
        Some("alex@example.org")
    );
    assert_eq!(identitaet("id:01HXYZ").as_deref(), Some("id:01HXYZ"));
    assert_eq!(identitaet("id:"), None);
    // Zwei Personen namens Alex: beide sperren einzeln, keine Zustimmung kann gelten.
    let mut z = zu("a@example.org", ConsentState::Zugestimmt);
    z.person = "alex".into();
    assert_eq!(
        start_modus("P1", true, &["Alex".into(), "alex".into()], &[z]),
        StartModus::NurNotizen {
            nicht_geplant: false,
            offen: vec![UNBEKANNT.into(), UNBEKANNT.into()],
            abgelehnt: vec![],
        }
    );
    // Personen-Id der App funktioniert.
    let id = zu("id:P-7", ConsentState::Zugestimmt);
    assert_eq!(
        start_modus("P1", true, &["id:P-7".into()], &[id]),
        StartModus::MitTranskription
    );
    // Speicher lehnt Anzeigenamen ab.
    let c = db();
    plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    let mut n = zu("x", ConsentState::Zugestimmt);
    n.person = "Alex".into();
    assert_eq!(
        einwilligung_setzen(&c, &n, 1).unwrap_err(),
        "person_ohne_identitaet"
    );
    // Neue Teilnehmende ohne Identitaet bekommen keinen Eintrag.
    assert!(neue_einwilligungen("P1", &[], &["Alex".into()]).is_empty());
}

#[test]
fn codex5b_inhalt_mit_listenzeichen_bleibt_inhalt() {
    let mut p = eingabe();
    p.title = "* Dringend".into();
    let a = einladung_aus("P1", &p, &Gestaltung::default());
    p.title = "• Dringend".into();
    let b = einladung_aus("P1", &p, &Gestaltung::default());
    p.title = "Dringend".into();
    let c = einladung_aus("P1", &p, &Gestaltung::default());
    assert_ne!(a.block_hash, b.block_hash);
    assert_ne!(a.block_hash, c.block_hash);
    assert_ne!(b.block_hash, c.block_hash);
    // Outlook-Listenformat fuer echte Punkte bleibt toleriert.
    let outlook = a.text.replace("\n• ", "\n*\t");
    assert_eq!(block_hash(&outlook), a.block_hash);
}

#[test]
fn codex15_andere_serieninstanz_ersetzt_nichts() {
    let c = db();
    let (mut l, text) = graph_link_mit_block(&c);
    l.instanz = Some(9_000);
    let mut b = beob(18_000, Some(text));
    b.instanz = Some(18_000);
    let a = abgleichen(PlanStatus::Verknuepft, &l, Some(&b), 1);
    assert_eq!(
        a.link,
        PlanLink {
            fremd_beobachtet: true,
            ..l.clone()
        }
    );
    assert!(!a.schreiben_erlaubt);
    assert_eq!(a.plan_status, None);
    assert_eq!(
        a.ereignisse,
        vec![Ereignis::AndererTermin {
            erwartet: Some(9_000),
            gesehen: Some(18_000)
        }]
    );
    // Ohne Instanzangabe ebenfalls nicht uebernehmen.
    b.instanz = None;
    assert!(!abgleichen(PlanStatus::Verknuepft, &l, Some(&b), 1).schreiben_erlaubt);
}

#[test]
fn codex15b_keine_instanz_wird_nicht_still_zur_instanz() {
    let c = db();
    let (l, text) = graph_link_mit_block(&c);
    assert_eq!(l.instanz, None);
    assert!(l.starts_at.is_some());
    let mut b = beob(18_000, Some(text.clone()));
    b.instanz = Some(18_000);
    let a = abgleichen(PlanStatus::Verknuepft, &l, Some(&b), 1);
    assert!(!a.schreiben_erlaubt);
    assert_eq!(a.link.instanz, None);
    assert_eq!(a.link.starts_at, l.starts_at);
    assert!(a.link.fremd_beobachtet);
    // Anderer Termin (andere Id) ebenso.
    let mut l2 = l.clone();
    l2.remote_id = Some("A".into());
    let mut b2 = beob(10_000, Some(text.clone()));
    b2.remote_id = Some("B".into());
    let a2 = abgleichen(PlanStatus::Verknuepft, &l2, Some(&b2), 1);
    assert!(!a2.schreiben_erlaubt);
    assert_eq!(a2.link.remote_id.as_deref(), Some("A"));
    // Bestaetigter Wechsel: danach Erstverknuepfung mit der neuen Instanz.
    let frei = crate::sync::identitaet_loesen(&l);
    let a3 = abgleichen(PlanStatus::Verknuepft, &frei, Some(&b), 2);
    assert_eq!(a3.ereignisse, vec![Ereignis::Verknuepft]);
    assert_eq!(a3.link.instanz, Some(18_000));
    assert!(a3.schreiben_erlaubt);
}

#[test]
fn codex22_abgewiesene_instanz_meldet_einmal() {
    let c = db();
    let (mut l, text) = graph_link_mit_block(&c);
    l.instanz = Some(9_000);
    let mut b = beob(18_000, Some(text.clone()));
    b.instanz = Some(18_000);
    let a1 = abgleichen(PlanStatus::Verknuepft, &l, Some(&b), 1);
    assert_eq!(a1.ereignisse.len(), 1);
    let a2 = abgleichen(PlanStatus::Verknuepft, &a1.link, Some(&b), 2);
    assert!(a2.ereignisse.is_empty());
    assert_eq!(a2.link, a1.link);
    // Passende Beobachtung setzt den Merker zurueck.
    let mut passend = beob(10_000, Some(text));
    passend.instanz = Some(9_000);
    let a3 = abgleichen(PlanStatus::Verknuepft, &a2.link, Some(&passend), 3);
    assert!(!a3.link.fremd_beobachtet);
    assert!(a3.schreiben_erlaubt);
}

#[test]
fn codex21_blockende_nur_als_vollstaendige_marke() {
    let c = db();
    let (l, text) = graph_link_mit_block(&c);
    let manipuliert = format!(
        "lva-plan:P1\n{}",
        text.replace("lva-plan:P1", "lva-plan:P1X")
    );
    assert!(finde_block(&manipuliert, "P1").is_none());
    assert_eq!(finde_marke(&manipuliert).as_deref(), Some("P1X"));
    let a = abgleichen(
        PlanStatus::Verknuepft,
        &l,
        Some(&beob(10_000, Some(manipuliert))),
        1,
    );
    assert!(!a.schreiben_erlaubt);
    assert_eq!(a.link.sync_state, SyncState::Verwaist);
    // Markenzeile mit Zusatz ist kein gueltiges Blockende.
    let zusatz = text.replace("lva-plan:P1", "lva-plan:P1 Zusatz");
    assert!(finde_block(&zusatz, "P1").is_none());
    assert!(finde_marke(&zusatz).is_none());
    let a = abgleichen(
        PlanStatus::Verknuepft,
        &l,
        Some(&beob(10_000, Some(zusatz))),
        1,
    );
    assert!(!a.schreiben_erlaubt);
    let davor = text.replace("lva-plan:P1", "Text lva-plan:P1");
    assert!(finde_block(&davor, "P1").is_none());
    // Leerraum und CRLF um die Markenzeile bleiben toleriert.
    let crlf = text
        .replace("lva-plan:P1", "  lva-plan:P1\t")
        .replace('\n', "\r\n");
    assert_eq!(finde_marke(&crlf).as_deref(), Some("P1"));
    assert!(finde_block(&crlf, "P1").is_some());
    // Marke ausserhalb eines Blocks zaehlt nicht.
    assert!(finde_marke("lva-plan:P1").is_none());
}

#[test]
fn codex16_eindeutigkeit_je_instanz() {
    let c = db();
    plan_anlegen(&c, "P1", &eingabe(), &Regeln::default(), 1).unwrap();
    plan_anlegen(&c, "P2", &eingabe(), &Regeln::default(), 1).unwrap();
    plan_anlegen(&c, "P3", &eingabe(), &Regeln::default(), 1).unwrap();
    let mut l1 = link("P1");
    l1.remote_id = Some("SERIE".into());
    l1.instanz = Some(1);
    link_speichern(&c, &l1).unwrap();
    let mut l2 = link("P2");
    l2.remote_id = Some("SERIE".into());
    l2.instanz = Some(2);
    link_speichern(&c, &l2).unwrap();
    let mut l3 = link("P3");
    l3.remote_id = Some("SERIE".into());
    l3.instanz = Some(2);
    assert!(link_speichern(&c, &l3).is_err());
    // Fehlende Instanz faellt mit keinem Zeitwert zusammen, auch nicht mit -1.
    l3.instanz = None;
    link_speichern(&c, &l3).unwrap();
    l2.instanz = Some(-1);
    link_speichern(&c, &l2).unwrap();
    let mut l4 = link("P2");
    l4.remote_id = Some("SERIE".into());
    l4.instanz = None;
    assert!(link_speichern(&c, &l4).is_err());
    assert_eq!(link_laden(&c, "P2").unwrap().unwrap().instanz, Some(-1));
}

#[test]
fn codex18_ics_wertet_blockinhalt_nicht_aus() {
    let c = db();
    let (mut l, text) = graph_link_mit_block(&c);
    l.adapter = AdapterKind::IcsMarker;
    let geaendert = text.replace("Optionen", "Varianten");
    let a = abgleichen(
        PlanStatus::Verknuepft,
        &l,
        Some(&beob(10_000, Some(geaendert))),
        1,
    );
    assert_eq!(a.link.sync_state, SyncState::Ok);
    assert!(!a.link.block_konflikt);
    assert!(a.ereignisse.is_empty());
    assert!(!a.schreiben_erlaubt);
}
