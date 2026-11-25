use super::*;

fn umfrage() -> Umfrage {
    Umfrage {
        id: "T-A1B2".into(),
        plan_id: None,
        titel: "Prüfung & Übung".into(),
        titel_an_externe: false,
        dauer_min: 60,
        frist: 1_798_790_400_000,
        status: Status::Offen,
        vorschlaege: (1..=3)
            .map(|nr| Vorschlag {
                nr,
                start: i64::from(nr) * 86_400_000,
                ende: i64::from(nr) * 86_400_000 + 3_600_000,
                ortszeit: Some(Ortszeit {
                    jahr: 2026,
                    monat: 12,
                    tag: nr,
                    wochentag: nr + 1,
                    stunde: 10,
                    minute: 0,
                }),
            })
            .collect(),
        teilnehmende: vec![
            Person {
                adresse: "pflicht@example.org".into(),
                name: "Beispiel".into(),
                intern: true,
                pflicht: true,
            },
            Person {
                adresse: "optional@example.org".into(),
                name: "Muster".into(),
                intern: false,
                pflicht: false,
            },
        ],
    }
}

#[test]
fn antwortvarianten_mit_gegenproben() {
    use Wert::{Offen as O, Passt as P, PasstNicht as N};
    let u = umfrage();
    // Freitext bleibt auch bei eindeutiger Erkennung ein Bestaetigungsvorschlag.
    let faelle = [
        ("Alle Termine passen", true, vec![P, P, P]),
        ("All dates work!", true, vec![P, P, P]),
        ("Alle Termine passen\n-- \nBeispiel", true, vec![P, P, P]),
        ("Vorschlag 2 passt", true, vec![O, P, O]),
        ("Option 3 works", true, vec![O, O, P]),
        ("Keiner passt", true, vec![N, N, N]),
        ("None work", true, vec![N, N, N]),
        (
            "[x] 1 Di 01.12. 10:00\n[ ] 2 Mi 02.12. 10:00\n[X] 3 Do 03.12.",
            true,
            vec![P, N, P],
        ),
        ("[ ] 1\n[x] 2\n[ ] 3", true, vec![N, P, N]),
        ("[x] 1 Di 01.12. 10:00–11:00\n[ ] 2", true, vec![P, N, O]),
        ("1 und 3", false, vec![P, O, P]),
        ("1 and 3", false, vec![P, O, P]),
        ("nur 2", false, vec![N, P, N]),
        ("only 2", false, vec![N, P, N]),
        ("ja", false, vec![P, P, P]),
        ("yes", false, vec![P, P, P]),
        ("geht nicht", false, vec![N, N, N]),
        ("no", false, vec![N, N, N]),
        ("Mittwoch 02.12.2026", false, vec![O, P, O]),
        ("Wednesday 02.12.2026 10:00", false, vec![O, P, O]),
        ("02.12.2026", false, vec![O, P, O]),
        ("Alle Termine passen\n> Keiner passt", true, vec![P, P, P]),
        (
            "Keiner passt\nVon: beispiel@example.org\nAlle Termine passen",
            true,
            vec![N, N, N],
        ),
        (
            "None work\nFrom: example@example.org\nAll dates work",
            true,
            vec![N, N, N],
        ),
        (
            "ja\nAm Dienstag schrieb Beispiel:\n> Keiner passt",
            false,
            vec![P, P, P],
        ),
        (
            "yes\nOn Tuesday, Example wrote:\n> None work",
            false,
            vec![P, P, P],
        ),
        (
            "Keiner passt\n-----Original Message-----\nAlle Termine passen",
            true,
            vec![N, N, N],
        ),
        (
            "Alle Termine passen\n________________\nKeiner passt",
            true,
            vec![P, P, P],
        ),
    ];
    for (text, sicher, erwartet) in faelle {
        let e = lese_antwort(text, &u);
        assert_eq!(e.sicher, sicher, "{text}");
        assert_eq!(e.art, ErgebnisArt::Antwort, "{text}");
        assert_eq!(
            e.werte.iter().map(|w| w.wert).collect::<Vec<_>>(),
            erwartet,
            "{text}"
        );
    }
    for text in [
        "vielleicht",
        "10:00",
        "ja aber nein",
        "1 und 9",
        "nur 0",
        "[x] 1\n[x] 1",
        "[ ] 1\n[ ] 2\n[ ] 3",
        "[x] 9",
        "[x] 1\naber nicht 1",
        "[x] 1 passt nicht",
        "Alle Termine passen\nKeiner passt",
        "Mittwoch 03.12.2026",
        "02.12.2025",
        "02.12.2026 11:00",
        "Vorschlag 9 passt",
        "Alle Termine passen vielleicht",
        "[x] 1 10:00–12:00",
        "Vorschlag +1 passt",
        "only +2",
        "[x] +1",
        "Weitergeleitete Nachricht\nAlle Termine passen",
        "Forwarded message\nAll dates work",
        "Fwd: All dates work",
        "AW: ja",
    ] {
        let e = lese_antwort(text, &u);
        assert!(!e.sicher, "{text}");
        assert!(e.werte.is_empty(), "{text}");
    }
    for text in [
        "Abwesenheitsnotiz: bin nicht da",
        "Automatic reply: Out of office",
        "Automatische Antwort: Alle Termine passen",
        "Auto-reply: yes",
    ] {
        assert_eq!(lese_antwort(text, &u).art, ErgebnisArt::Ignorieren);
    }
    assert!(lese_antwort(&"ä".repeat(MAX_ANTWORT_BYTES), &u)
        .werte
        .is_empty());
    let mut einer = u.clone();
    einer.vorschlaege.truncate(1);
    assert_eq!(lese_antwort("10:00", &einer).werte[0].wert, P);
    assert!(!lese_antwort("10:00", &einer).sicher);
    einer.vorschlaege[0].ortszeit = None;
    assert!(lese_antwort("10:00", &einer).werte.is_empty());
}

#[test]
fn modell_validierung_und_serde() {
    let mut u = umfrage();
    assert!(u.validiere().is_ok());
    assert_eq!(
        serde_json::from_str::<Umfrage>(&serde_json::to_string(&u).unwrap()).unwrap(),
        u
    );
    for id in ["", "T-ab12", "T-ABCDE", "T-Ä123", "T-A1B2/", "T-A1B2\n"] {
        assert!(!gueltige_id(id));
    }
    u.vorschlaege[1].nr = 1;
    assert!(u.validiere().is_err());
    u = umfrage();
    u.vorschlaege[0].ende = u.vorschlaege[0].start;
    assert!(u.validiere().is_err());
    u = umfrage();
    u.teilnehmende[1].adresse = " PFLICHT@example.org ".into();
    assert!(u.validiere().is_err());
    u = umfrage();
    u.vorschlaege[0].ortszeit.as_mut().unwrap().tag = 32;
    assert!(u.validiere().is_err());
}

#[test]
fn codes_und_marken() {
    let key = [7_u8; 32];
    let code = persoenlicher_code(&key, "T-A1B2", " Person@EXAMPLE.org ").unwrap();
    assert_eq!(
        code,
        persoenlicher_code(&key, "T-A1B2", "person@example.org").unwrap()
    );
    assert_eq!(code.len(), 4);
    // Gegen unabhaengig mit Python hmac/hashlib berechneten Vektor pruefen.
    assert_eq!(code, "7ZYN");
    assert!(pruefe_code(&key, "T-A1B2", "person@example.org", &code));
    assert!(!pruefe_code(&key, "T-A1B2", "other@example.org", &code));
    assert!(!pruefe_code(
        &[8; 32],
        "T-A1B2",
        "person@example.org",
        &code
    ));
    assert!(!pruefe_code(&key, "T-ZZZZ", "person@example.org", &code));
    for bad in ["", "abc", "ABCDE", "!!!!", "Ä123"] {
        assert!(!pruefe_code(&key, "T-A1B2", "person@example.org", bad));
    }
    assert!(persoenlicher_code(b"short", "T-A1B2", "person@example.org").is_err());
    assert!(persoenlicher_code(&key, "bad", "person@example.org").is_err());
    let marke = betreff_marke("T-A1B2", &code).unwrap();
    assert_eq!(
        finde_marke(&format!("Re: AW: {marke}")),
        Some(("T-A1B2".into(), code))
    );
    assert!(finde_marke(&format!("{marke} {marke}")).is_none());
    assert!(finde_marke("[Terminfindung T-A1B2-AAAAA]").is_none());
    assert!(finde_marke("[Terminfindung T-a1b2-AAAA]").is_none());
    assert!(finde_marke(&"x".repeat(MAX_BETREFF_BYTES + 1)).is_none());
}

fn antwort(person: &str, nr: u8, wert: Wert, zeit: i64) -> Antwort {
    Antwort {
        person: person.into(),
        vorschlag_nr: nr,
        wert,
        quelle: Quelle::Knopf,
        sicher: true,
        eingegangen: zeit,
    }
}

#[test]
fn bewertung_erklaert_punkte_ausschluss_und_gleichstand() {
    let u = umfrage();
    let a = vec![
        antwort("pflicht@example.org", 1, Wert::PasstNicht, 1),
        antwort("pflicht@example.org", 2, Wert::Passt, 1),
        antwort("optional@example.org", 2, Wert::Notfalls, 1),
        antwort("pflicht@example.org", 3, Wert::Passt, 1),
    ];
    let f = FreiBelegt {
        eintraege: vec![],
        kernzeit_vorschlaege: vec![3],
    };
    let r = bewerte(&u, &a, &f);
    assert_eq!(r.iter().map(|r| r.nr).collect::<Vec<_>>(), vec![2, 3, 1]);
    assert_eq!(r[0].punkte, 7);
    assert_eq!(r[1].punkte, 7);
    assert!(r[2].ausgeschlossen);
    assert!(r[0].begruendung.contains("1 Pflicht"));
    assert!(r[0].begruendung.contains("6 + 1 + 0 = 7"));
    assert!(r[1].begruendung.contains("1 offen"));
    let mut umgekehrt = u.clone();
    umgekehrt.vorschlaege.reverse();
    assert_eq!(bewerte(&umgekehrt, &a, &f), r);
}

#[test]
fn bewertung_juengste_antwort_keine_doppelzaehlung_kein_unsicherer_import() {
    let u = umfrage();
    let f = FreiBelegt {
        eintraege: vec![KalenderWert {
            person: "pflicht@example.org".into(),
            vorschlag_nr: 1,
            wert: Verfuegbarkeit::Frei,
        }],
        kernzeit_vorschlaege: vec![],
    };
    let mut a = vec![
        antwort("pflicht@example.org", 1, Wert::Passt, 1),
        antwort(" PFLICHT@example.org ", 1, Wert::PasstNicht, 2),
    ];
    let r = bewerte(&u, &a, &f);
    assert!(r.iter().find(|r| r.nr == 1).unwrap().ausgeschlossen);
    a[1].sicher = false;
    a.push(antwort("fremd@example.org", 1, Wert::Passt, 3));
    assert_eq!(
        bewerte(&u, &a, &f)
            .iter()
            .find(|r| r.nr == 1)
            .unwrap()
            .punkte,
        0
    );
    a[1].wert = Wert::Passt;
    a[1].sicher = true;
    assert_eq!(bewerte(&u, &a, &f)[0].punkte, 6);
    a[1].wert = Wert::PasstNicht;
    a[1].eingegangen = 1;
    assert_eq!(
        bewerte(&u, &a, &f)
            .iter()
            .find(|r| r.nr == 1)
            .unwrap()
            .punkte,
        0
    );
    assert!(bewerte(&u, &[], &f)[0].punkte == 6);
    let mut extern_frei = f.clone();
    extern_frei.eintraege[0].person = "optional@example.org".into();
    assert_eq!(bewerte(&u, &[], &extern_frei)[0].punkte, 0);
}

#[test]
fn mailto_und_externe_datenminimierung() {
    let u = umfrage();
    let text = anfrage_text(&u, true).unwrap();
    assert!(!text.contains(&u.titel));
    assert!(!text.contains("example.org"));
    assert!(anfrage_text(&u, false).unwrap().contains(&u.titel));
    let links = antwort_links("einladung@example.org", &u, "AB12").unwrap();
    assert_eq!(links.len(), 6);
    assert!(links[0]
        .url
        .contains("subject=%5BTerminfindung%20T-A1B2-AB12%5D"));
    assert!(links.iter().any(|l| l.url.contains("%0D%0A")));
    let url = mailto_link(
        "einladung@example.org",
        "Prüfung & Übung?",
        "Grüße + 50%\nJa",
    )
    .unwrap();
    assert_eq!(url, "mailto:einladung@example.org?subject=Pr%C3%BCfung%20%26%20%C3%9Cbung%3F&body=Gr%C3%BC%C3%9Fe%20%2B%2050%25%0D%0AJa");
    assert!(mailto_link("a@example.org?bcc=b@example.org", "x", "x").is_err());
    assert!(mailto_link("a@example.org", "x\r\nBcc: b@example.org", "x").is_err());
    assert!(mailto_link("a@example.org,b@example.org", "x", "x").is_err());
    // Jeder vorbereitete Knopf muss sich mit dem Parser wieder einlesen lassen.
    for link in links {
        let e = lese_antwort(&link.body, &u);
        if link.label == "Auswahl beantworten" {
            assert_eq!(e.art, ErgebnisArt::Pruefen);
            assert!(!e.sicher);
            let ausgewaehlt = lese_antwort(&link.body.replacen("[ ]", "[x]", 1), &u);
            assert!(ausgewaehlt.sicher);
            assert_eq!(ausgewaehlt.werte[0].wert, Wert::Passt);
            assert_eq!(ausgewaehlt.werte[1].wert, Wert::PasstNicht);
        } else {
            assert_eq!(e.art, ErgebnisArt::Antwort);
            assert!(e.sicher);
        }
    }
}

#[test]
fn freitext_notfalls_bleibt_unsicher() {
    for text in ["2 notfalls", "2 if necessary"] {
        let e = lese_antwort(text, &umfrage());
        assert!(!e.sicher);
        assert_eq!(e.werte[1].wert, Wert::Notfalls);
        assert_eq!(e.werte[0].wert, Wert::Offen);
    }
    assert!(lese_antwort("2 notfalls aber nicht sicher", &umfrage())
        .werte
        .is_empty());
}

#[test]
fn kalender_gewichte_und_alle_pflicht_konflikte() {
    for (wert, punkte, ausgeschlossen) in [
        (Verfuegbarkeit::Frei, 6, false),
        (Verfuegbarkeit::Anderswo, 6, false),
        (Verfuegbarkeit::Vorlaeufig, 1, false),
        (Verfuegbarkeit::Belegt, 0, true),
        (Verfuegbarkeit::Abwesend, 0, true),
        (Verfuegbarkeit::Unbekannt, 0, false),
    ] {
        let f = FreiBelegt {
            eintraege: vec![KalenderWert {
                person: "pflicht@example.org".into(),
                vorschlag_nr: 1,
                wert,
            }],
            kernzeit_vorschlaege: vec![],
        };
        let r = bewerte(&umfrage(), &[], &f);
        let r = r.iter().find(|r| r.nr == 1).unwrap();
        assert_eq!((r.punkte, r.ausgeschlossen), (punkte, ausgeschlossen));
    }
    let u = umfrage();
    let a: Vec<_> = u
        .vorschlaege
        .iter()
        .map(|v| antwort("pflicht@example.org", v.nr, Wert::PasstNicht, 1))
        .collect();
    let r = bewerte(&u, &a, &FreiBelegt::default());
    assert!(r.iter().all(|r| r.ausgeschlossen));
    assert_eq!(r.iter().map(|r| r.nr).collect::<Vec<_>>(), vec![1, 2, 3]);
    let r = bewerte(
        &u,
        &[antwort("optional@example.org", 1, Wert::Passt, 1)],
        &FreiBelegt::default(),
    );
    assert_eq!(r[0].punkte, 3);
}

#[test]
fn utc_text_ohne_rechner_zeitzone() {
    use super::texte::utc_text;
    assert_eq!(utc_text(0), "01.01.1970 00:00 UTC");
    assert_eq!(utc_text(-1), "31.12.1969 23:59 UTC");
    assert_eq!(utc_text(951_782_400_000), "29.02.2000 00:00 UTC");
    assert_eq!(utc_text(-2_203_891_200_000), "01.03.1900 00:00 UTC");
    assert_eq!(utc_text(-62_135_596_800_000), "01.01.0001 00:00 UTC");
    assert!(utc_text(i64::MAX).contains("außerhalb"));
    let mut u = umfrage();
    u.vorschlaege[0].ortszeit = None;
    let text = anfrage_text(&u, true).unwrap();
    assert!(text.contains("02.01.1970 00:00 UTC"));
    assert!(!text.contains("Millisekunden"));
}

#[test]
fn kalenderquelle_nur_intern_und_explizite_antwort_geht_vor() {
    let u = umfrage();
    let mut kalender = antwort("optional@example.org", 1, Wert::Passt, 10);
    kalender.quelle = Quelle::Kalender;
    assert_eq!(
        bewerte(&u, std::slice::from_ref(&kalender), &FreiBelegt::default())[0].punkte,
        0
    );
    kalender.person = "pflicht@example.org".into();
    assert_eq!(
        bewerte(&u, std::slice::from_ref(&kalender), &FreiBelegt::default())[0].punkte,
        6
    );
    let a = vec![
        kalender,
        antwort("pflicht@example.org", 1, Wert::PasstNicht, 1),
    ];
    assert!(
        bewerte(&u, &a, &FreiBelegt::default())
            .iter()
            .find(|r| r.nr == 1)
            .unwrap()
            .ausgeschlossen
    );
}
