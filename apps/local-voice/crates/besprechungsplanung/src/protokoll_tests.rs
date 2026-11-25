//! Tests zu `protokoll` (P5, lva#736). Nur synthetische Daten (`example.org`).

use crate::invitation::{einladung, klassifizierungszeile, Gestaltung};
use crate::model::*;
use crate::protokoll::*;

fn plan(agenda: usize) -> Plan {
    Plan {
        id: "P1".into(),
        meeting_id: None,
        title: "Abstimmung Posteingang".into(),
        goal_type: GoalType::Entscheiden,
        goal_text: "Wir entscheiden, ob der Pilot im Januar startet.".into(),
        expected_result: Some("Entscheidung mit Termin".into()),
        duration_min: 60,
        classification: "intern".into(),
        agenda: (1..=agenda)
            .map(|i| AgendaItem {
                minutes: 5,
                topic: format!("Thema {i}"),
                owner: (i == 1).then(|| "A. Beispiel".to_string()),
            })
            .collect(),
        roles: Roles::default(),
        materials: vec![],
        hints: Hints::default(),
        transcription_planned: true,
        minutes_template_id: None,
        status: PlanStatus::Verknuepft,
        created_at: 1,
        updated_at: 1,
    }
}

fn gueltig(v: &Vorlage) {
    assert!(v.abschnitte.len() <= MAX_ABSCHNITTE);
    assert!(!v.abschnitte.is_empty());
    assert!(v.titel.chars().count() <= MAX_TITEL && !v.titel.is_empty());
    assert!(v.kontext.chars().count() <= MAX_KONTEXT);
    let mut ids: Vec<&str> = Vec::new();
    for a in &v.abschnitte {
        assert!(
            (1..=32).contains(&a.id.len())
                && a.id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{}",
            a.id
        );
        assert!(!ids.contains(&a.id.as_str()), "doppelt: {}", a.id);
        ids.push(&a.id);
        assert!(a.titel.chars().count() <= MAX_TITEL && !a.titel.trim().is_empty());
        assert!(a.anweisung.chars().count() <= MAX_ANWEISUNG);
    }
    assert_eq!(v.abschnitte.iter().filter(|a| a.aufgaben).count(), 1);
}

#[test]
fn vorlage_hat_je_agendapunkt_einen_abschnitt_und_feste_schlussabschnitte() {
    let v = vorlage(&plan(3));
    gueltig(&v);
    let ids: Vec<&str> = v.abschnitte.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "agenda_1",
            "agenda_2",
            "agenda_3",
            "ergebnisse",
            "entscheidungen",
            "aufgaben"
        ]
    );
    assert_eq!(v.abschnitte[0].titel, "1. Thema 1");
    assert!(v.abschnitte[0]
        .anweisung
        .contains("verantwortlich: A. Beispiel"));
    assert!(v.abschnitte[3]
        .anweisung
        .contains("Entscheidung mit Termin"));
    assert!(v.abschnitte[5].aufgaben);
    assert!(v
        .kontext
        .contains("Wir entscheiden, ob der Pilot im Januar startet"));
    assert!(v.kontext.contains("Entscheidung mit Termin"));
    assert!(v.titel.starts_with("Agenda: "));
}

#[test]
fn vorlage_ohne_agenda_liefert_nur_die_schlussabschnitte() {
    let v = vorlage(&plan(0));
    gueltig(&v);
    assert_eq!(v.abschnitte.len(), 3);
}

#[test]
fn vorlage_faltet_ueberzaehlige_agendapunkte_in_einen_abschnitt() {
    for n in [7, 8, 30] {
        let v = vorlage(&plan(n));
        gueltig(&v);
        if n == 7 {
            assert!(v.abschnitte.iter().all(|a| a.id != "agenda_rest"));
            assert_eq!(v.abschnitte.len(), 10);
        } else {
            assert_eq!(v.abschnitte.len(), 10);
            let rest = v.abschnitte.iter().find(|a| a.id == "agenda_rest").unwrap();
            assert!(rest.anweisung.contains("7. Thema 7"));
        }
    }
}

#[test]
fn vorlage_kuerzt_lange_und_mehrzeilige_texte() {
    let mut p = plan(1);
    p.title = "T".repeat(300);
    p.goal_text = "Ziel\nzweite Zeile ".repeat(200);
    p.expected_result = Some("E".repeat(900));
    p.agenda[0].topic = format!("Zeile eins\nZeile zwei {}", "ä".repeat(400));
    let v = vorlage(&p);
    gueltig(&v);
    assert!(!v.abschnitte[0].titel.contains('\n'));
    assert!(!v.kontext.contains('\n'));
}

#[test]
fn vorlage_laesst_das_modell_das_ziel_nicht_bewerten() {
    let v = vorlage(&plan(2));
    assert!(v.kontext.contains("bewertet ein Mensch"));
}

#[test]
fn kopf_traegt_ziel_ergebnis_erreicht_und_klassifizierung() {
    let p = plan(2);
    let md = kopf_markdown(&p, None);
    assert!(md.contains("**Ziel (Entscheiden):** Wir entscheiden"));
    assert!(md.contains("**Erwartetes Ergebnis:** Entscheidung mit Termin"));
    assert!(md.contains("**Ziel erreicht:** noch nicht bewertet"));
    assert!(md.contains("**Klassifizierung:** intern"));
    for (e, wort) in [
        (Erreicht::Ja, "ja"),
        (Erreicht::Teilweise, "teilweise"),
        (Erreicht::Nein, "nein"),
    ] {
        assert!(kopf_markdown(&p, Some(e)).contains(&format!("**Ziel erreicht:** {wort}\n")));
    }
    let mut ohne = p.clone();
    ohne.expected_result = None;
    assert!(!kopf_markdown(&ohne, None).contains("Erwartetes Ergebnis"));
}

#[test]
fn erreicht_wird_nachgeschrieben_und_zeilenenden_bleiben() {
    let p = plan(1);
    let md = format!(
        "# Protokoll: X\r\n{}\r\n## A\r\n\r\ntext\r\n",
        kopf_markdown(&p, None).replace('\n', "\r\n")
    );
    let neu = erreicht_ersetzen(&md, Some(Erreicht::Teilweise)).unwrap();
    assert!(neu.contains("**Ziel erreicht:** teilweise\r\n"));
    assert!(!neu.contains("noch nicht bewertet"));
    assert_eq!(neu.matches('\n').count(), md.matches('\n').count());
    assert_eq!(erreicht_lesen(&neu), Some(Some(Erreicht::Teilweise)));
    let zurueck = erreicht_ersetzen(&neu, None).unwrap();
    assert_eq!(erreicht_lesen(&zurueck), Some(None));
    assert!(erreicht_ersetzen("# Protokoll\n\n## A\n", Some(Erreicht::Ja)).is_none());
    assert_eq!(erreicht_lesen("# Protokoll\n"), None);
}

#[test]
fn redeanteile_gehen_nicht_in_den_text() {
    let md = "# Protokoll: X\n\n## A\n\ntext\n\n## Sprecher & Redeanteile\n\n| a | b |\n|---|---|\n| X | 60 % |\n\n## Danach\n\nrest\n";
    let o = ohne_redeanteile(md);
    assert!(!o.contains("Redeanteile") && !o.contains("60 %"));
    assert!(o.contains("## A") && o.contains("## Danach") && o.contains("rest"));
    assert_eq!(ohne_redeanteile("# P\n\n## A\n"), "# P\n\n## A\n");
}

#[test]
fn empfaenger_normalisiert_entdoppelt_und_zaehlt_fehlende_adressen() {
    let e = empfaenger(
        &[
            Some("Anna@Example.org"),
            Some(" anna@example.org "),
            Some("mailto:bert@example.org"),
            Some("Cleo <CLEO@example.org>"),
            None,
            Some(""),
            Some("Nur Name"),
            Some("id:p1"),
            Some("ich@example.org"),
            Some("dora@example.org"),
        ],
        &["Ich@Example.org".to_string()],
    );
    assert_eq!(
        e.adressen,
        [
            "anna@example.org",
            "bert@example.org",
            "cleo@example.org",
            "dora@example.org"
        ]
    );
    assert_eq!(e.ohne_adresse, 4);
}

#[test]
fn empfaenger_ohne_teilnehmende_ist_leer() {
    assert_eq!(empfaenger(&[], &[]), Empfaenger::default());
}

#[test]
fn betreff_hat_titel_und_datum() {
    assert_eq!(
        betreff("Abstimmung Posteingang", "2026-11-12"),
        "Protokoll: Abstimmung Posteingang (2026-11-12)"
    );
    assert_eq!(
        betreff("Zwei\nZeilen", "2026-11-12"),
        "Protokoll: Zwei Zeilen (2026-11-12)"
    );
}

#[test]
fn klassifizierungszeile_ist_die_der_einladung() {
    let p = plan(2);
    let e = einladung(&p, &Gestaltung::default());
    let zeile = klassifizierungszeile(&p.classification);
    assert_eq!(zeile, "Klassifizierung: intern");
    assert!(e.text.lines().any(|l| l == zeile));
}

#[test]
fn entwurfstext_setzt_die_klassifizierung_genau_einmal() {
    let ohne = entwurfstext("intern", "Protokoll: X\r\n\r\nInhalt\r\n");
    assert_eq!(ohne, "Klassifizierung: intern\n\nProtokoll: X\n\nInhalt");
    let mit = entwurfstext("intern", "Protokoll: X\nKlassifizierung: intern\nInhalt");
    assert_eq!(mit.matches("Klassifizierung: intern").count(), 1);
    assert!(mit.starts_with("Protokoll: X"));
    // Eine andere Klassifizierung im Text ersetzt die der Planung nicht.
    let anders = entwurfstext("vertraulich", "Klassifizierung: intern\nInhalt");
    assert!(anders.starts_with("Klassifizierung: vertraulich"));
}

#[test]
fn text_hash_ignoriert_zeilenenden_nicht_den_inhalt() {
    assert_eq!(text_hash("a\r\nb\r\n"), text_hash("a\nb"));
    assert_ne!(text_hash("a\nb"), text_hash("a\nc"));
    assert_eq!(text_hash("").len(), 64);
}

#[test]
fn erreicht_wertetabelle() {
    for e in [Erreicht::Ja, Erreicht::Teilweise, Erreicht::Nein] {
        assert_eq!(Erreicht::parse(e.as_str()), Some(e));
    }
    assert_eq!(Erreicht::parse("vielleicht"), None);
    assert_eq!(
        serde_json::to_string(&Erreicht::Teilweise).unwrap(),
        "\"teilweise\""
    );
}
