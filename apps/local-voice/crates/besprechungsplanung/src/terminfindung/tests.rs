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
                start: 1_796_115_600_000 + i64::from(nr - 1) * 86_400_000,
                ende: 1_796_119_200_000 + i64::from(nr - 1) * 86_400_000,
                ortszeit: Some(Ortszeit {
                    jahr: 2026,
                    monat: 12,
                    tag: nr,
                    wochentag: nr + 1,
                    stunde: 10,
                    minute: 0,
                    utc_versatz_min: 60,
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
fn inline_antworten_und_signaturen_verdecken_keinen_widerspruch() {
    let u = umfrage();
    // Gegenprobe: Ein Rueckfall auf Abbruch bei '>' oder '--' macht diese
    // widerspruechlichen Antworten wieder sicher.
    for text in [
        "Vorschlag 2 passt\n> Ursprüngliche Auswahl: Vorschlag 2\nKorrektur: Vorschlag 2 passt nicht.",
        "All dates work\n> Old reply\n>> Older reply\n\nOption 2 does not work.",
        "[x] 2\n> Auswahl\n[ ] 2",
        "Alle Termine passen\n-- \nVorschlag 2 passt nicht",
        "Vorschlag 2 passt\n-- \nBeispiel\nKorrektur: Vorschlag 2 passt nicht.",
        "[x] 2\n-- \nNein, 2 geht nicht",
        "Alle Termine passen\n________________\nKeiner passt",
        "Vorschlag 2 passt\n-----\nVorschlag 2 passt nicht",
        "Alle Termine passen\nKeiner passt",
        "Vorschlag 2 passt, aber nicht am Mittwoch",
        "[x] 2\nNein, 2 geht nicht",
        "[x] 2 passt nicht",
    ] {
        let e = lese_antwort(text, &u);
        assert_eq!(e.art, ErgebnisArt::Pruefen, "{text}");
        assert!(!e.sicher, "{text}");
        assert!(e.werte.is_empty(), "{text}");
    }
    // Zitatzeilen duerfen auch vor und zwischen eigener Auswahl stehen.
    for text in [
        "> Keiner passt\nAlle Termine passen",
        "Alle Termine\n> Keiner passt\n>> alte Antwort\npassen",
        "[x] 2\n> [ ] 2\n[ ] 1",
        "Alle Termine passen\n-- ",
    ] {
        let e = lese_antwort(text, &u);
        assert_eq!(e.art, ErgebnisArt::Antwort, "{text}");
        assert!(e.sicher, "{text}");
        assert_eq!(e.werte[1].wert, Wert::Passt, "{text}");
    }
    for trennung in [
        "Von: beispiel@example.org",
        "From: example@example.org",
        "Am Dienstag schrieb Beispiel:",
        "Am Dienstag schrieb Beispiel <beispiel@example.org>:",
        "On Tuesday, Example wrote:",
        "-----Original Message-----",
    ] {
        let text = format!("Vorschlag 2 passt\n> Auswahl\n{trennung}\nVorschlag 2 passt nicht");
        let e = lese_antwort(&text, &u);
        assert_eq!(e.art, ErgebnisArt::Antwort, "{trennung}");
        assert!(e.sicher, "{trennung}");
        assert_eq!(e.werte[1].wert, Wert::Passt);
    }
}

#[test]
fn signaturen_ohne_antwortgehalt_bleiben_sicher() {
    let u = umfrage();
    // Ohne Signaturbehandlung werden diese Knopf- und Auswahlantworten unklar.
    for signatur in [
        "-- \nBeispiel\nMuster GmbH\nTelefon: +49 30 1 23\nMusterweg 2\n12345 Musterstadt",
        "Mit freundlichen Grüßen\nBeispiel\nTel. 1 23\nKeine Haftung für Übertragungsfehler.",
        "Mit freundlichen Gruessen,\nBeispiel\nMuster GmbH\nStand: 02.12.2026",
        "Viele Grüße\nBeispiel\nMusterstraße 1 23\n12345 Musterstadt",
        "Beste Gruesse\nBeispiel\n1 23\n2\n02.12.2026",
        "Best regards,\nExample\nExample Ltd\nPhone: +44 20 1 23\nNo liability for transmission errors.",
        "Kind regards\nExample\n1 Sample Road\nThis email is confidential. Please delete if received in error.",
        "Gesendet von meinem Smartphone\nTel. 1 23",
        "Sent from my phone\nExample Ltd\n02.12.2026",
        "________________\nBeispiel\nMuster GmbH\n1 23",
        "-----\nExample\nExample Ltd\n2",
    ] {
        for (antworttext, erwartet) in [
            ("Alle Termine passen", vec![Wert::Passt; 3]),
            ("[ ] 1\n[x] 2", vec![Wert::PasstNicht, Wert::Passt, Wert::Offen]),
        ] {
            let text = format!("{antworttext}\n{signatur}");
            let e = lese_antwort(&text, &u);
            assert_eq!(e.art, ErgebnisArt::Antwort, "{text}");
            assert!(e.sicher, "{text}");
            assert_eq!(e.werte.iter().map(|w| w.wert).collect::<Vec<_>>(), erwartet);
        }
    }
    // Auch Firmennamen unterliegen der Sperre. Vor der Grenze bleiben
    // selbst harmlose Zusaetze streng geprueft.
    for text in [
        // Works ist ein Sperrwort, auch in einem Firmennamen.
        "Alle Termine passen\nKind regards\nExample\n02.12.2026\nExample Works Ltd\nWe do not accept liability for transmission errors.\nI cannot accept liability for damage.",
        "Alle Termine passen\nBeispiel\nViele Gruesse",
        "[x] 2\nTelefon: 1 23\nBest regards",
        "Alle Termine passen\nViele Gruesse, aber Mittwoch geht nicht",
    ] {
        let e = lese_antwort(text, &u);
        assert_eq!(e.art, ErgebnisArt::Pruefen);
        assert!(!e.sicher);
        assert!(e.werte.is_empty());
    }
}

#[test]
fn antwortgehalt_in_signaturen_bleibt_pruefpflichtig() {
    let u = umfrage();
    // Gegenprobe zu blindem Abschneiden: Auch nach der Grenze alles pruefen,
    // einschliesslich mehrzeiliger Antwortmuster und unbekannter Nummern.
    for zusatz in [
        "Beispiel\nKeiner passt",
        "Example\nOption 2 does not work.",
        "Beispiel\n[x] 2\n[ ] 1",
        "Beispiel\nNein, Mittwoch geht nicht",
        "Beispiel\nKorrektur: doch erst spaeter",
        "Example\nCorrection: please call me",
        "Beispiel\nVorschlag 2\npasst nicht",
        "Beispiel\nVorschlag 9 passt",
        "Beispiel\nnur 2",
        "Example\n1 and 3",
        "Example\n1, 3",
        "Example\nyes",
        "Beispiel\n2 notfalls",
        "Beispiel\nAlle Termine passen",
        "Example\ndoes not work",
        "Beispiel\npasst nicht",
    ] {
        for grenze in ["-- ", "Viele Grüße", "Best regards", "Sent from my phone", "-----"] {
            let text = format!("Vorschlag 2 passt\n{grenze}\n{zusatz}");
            let e = lese_antwort(&text, &u);
            assert_eq!(e.art, ErgebnisArt::Pruefen, "{text}");
            assert!(!e.sicher, "{text}");
            assert!(e.werte.is_empty(), "{text}");
        }
    }
    let e = lese_antwort("Alle Termine passen\nSent from my phone; no dates work", &u);
    assert_eq!(e.art, ErgebnisArt::Pruefen);
    assert!(!e.sicher);
}

#[test]
fn unbekannter_text_nach_signaturgrenze_bleibt_pruefpflichtig() {
    let u = umfrage();
    // Gegenprobe: Die fruehere Absagemusterliste macht diese Texte sicher.
    for zusatz in [
        "PS: Ich kann\nnicht teilnehmen.",
        "PS: Ich muss absagen.",
        "PS: cannot make it",
        "PS: cannot\nmake it",
        "Ich kann\nnicht teilnehmen.",
        "Wir muessen verschieben",
        "Leider verhindert",
        "Bitte nochmals melden",
        "Ich Muss Absagen",
        "Max Beispiel.",
        "max beispiel",
        "Telefon: +49 30 123 Ich muss absagen",
        "https://example.org PS: cannot make it",
        "Gesendet von meinem Smartphone; Ich muss absagen.",
        "This email is confidential.\nPS: cannot make it",
        "This email is confidential.\nWe cannot\nmake it.",
        "Keine Haftung für Übertragungsfehler.\nWir koennen am Termin nicht teilnehmen.",
        "We do not accept liability for transmission errors.\nI must cancel.",
    ] {
        for antworttext in ["Alle Termine passen", "[ ] 1\n[x] 2"] {
            let text = format!("{antworttext}\nViele Grüße\n{zusatz}");
            let e = lese_antwort(&text, &u);
            assert_eq!(e.art, ErgebnisArt::Pruefen, "{text}");
            assert!(!e.sicher, "{text}");
            assert!(e.werte.is_empty(), "{text}");
        }
    }
}

#[test]
fn eindeutige_signaturbestandteile_bleiben_sicher() {
    let u = umfrage();
    for signatur in [
        "Dr. Anna von Beispiel\nLeitung Entwicklung",
        "Fax: +49 (30) 123-4\nMobil: +49 170 123\nE-Mail: anna@example.org",
        "https://example.org\nwww.example.org\n12345 Musterstadt\nMusterweg 12",
        "Handelsregister: HRB 12345\nUSt-IdNr.: DE123456789",
        "This email is confidential.\nPlease delete this message if received in error.",
        "Keine Haftung für Übertragungsfehler.\nBitte informieren Sie den Absender.",
    ] {
        let text = format!("Alle Termine passen\nViele Grüße\n{signatur}");
        let e = lese_antwort(&text, &u);
        assert_eq!(e.art, ErgebnisArt::Antwort, "{text}");
        assert!(e.sicher, "{text}");
        assert!(e.werte.iter().all(|w| w.wert == Wert::Passt), "{text}");
    }
}

#[test]
fn signatursperre_hat_vorrang_vor_positivregeln() {
    let u = umfrage();
    // Ohne globale Sperre werden Grossschreibung, Kontakt und Hinweis zum Schlupfloch.
    for zusatz in [
        "This email is confidential.\nMittwoch passt nicht.",
        "Korrektur\nNur Mittwoch",
        "This email is confidential. Mittwoch passt nicht.",
        "No liability for transmission errors.\nNur Mittwoch",
        "We do not accept liability for transmission errors.\nWon’t\nAttend",
        "I cannot accept liability for damage.\nCan\nT",
        "We do not accept liability for transmission errors. aber",
        "We do not accept liability for transmission errors",
        "We do not accept liability for transmission errors. Instead",
        "Zusatz We do not accept liability for transmission errors.",
        "Nicht bekannte Haftung.\nThis email is confidential.",
        "Keine Haftung für Übertragungsfehler bitte.",
        "Diese E-Mail ist nicht vertraulich.\nDoch",
        "https://example.org/only",
        "no@example.org",
        "Anna Beispiel\nMuster GmbH\nKorrektur",
        "Anna Beispiel\nMuster GmbH\nBeliebiger Text",
        "Dr. Anna von Beispiel\nLeitung Entwicklung\nBeispiel AG",
        "Anna Beispiel\n\nMuster GmbH\n\nBeliebiger Text",
        "Anna Beispiel\nThis email is confidential.\nBeliebiger Text",
    ] {
        for kopf in ["Alle Termine passen", "[ ] 1\n[x] 2"] {
            let text = format!("{kopf}\nViele Grüße\n{zusatz}");
            let e = lese_antwort(&text, &u);
            assert_eq!(e.art, ErgebnisArt::Pruefen, "{text}");
            assert!(!e.sicher, "{text}");
            assert!(e.werte.is_empty(), "{text}");
        }
    }
    for wort in [
        "Montag", "Di.", "Mittwoch", "Do", "Freitag", "Sa", "Sonntag",
        "Monday", "Tue", "Wednesday", "Thu", "Friday", "Sat", "Sunday",
        "Januar", "Februar", "März", "Maerz", "April", "Mai", "Juni", "Juli",
        "August", "September", "Oktober", "November", "Dezember", "January",
        "February", "March", "May", "June", "July", "October", "December",
        "Termin", "Vorschlag", "Slot", "Option", "Date", "Time", "Passt",
        "Passen", "Geht", "Klappt", "Works", "Fits", "Nicht", "Kein", "Keine",
        "Keiner", "Nein", "No", "Not", "Cannot", "Can’t", "Won't", "Nur",
        "Only", "Just", "Korrektur", "Korrigiere", "Correction", "Instead",
        "Statt", "Stattdessen", "Sondern", "Doch", "Aber", "But", "However",
        "Absage", "Absagen", "Verhindert", "Unavailable", "Decline", "Cancel",
        "Leider", "Unfortunately", "Ja", "Yes", "Zusage", "Ich", "Wir", "I",
        "We", "Mir", "Uns", "PS",
    ] {
        let text = format!("Alle Termine passen\nViele Grüße\n{wort}");
        let e = lese_antwort(&text, &u);
        assert_eq!(e.art, ErgebnisArt::Pruefen, "{wort}");
        assert!(!e.sicher, "{wort}");
        assert!(e.werte.is_empty(), "{wort}");
    }
}

#[test]
fn nur_vollstaendige_hinweissaetze_werden_einzeln_ausgenommen() {
    let u = umfrage();
    // Gegenprobe: Ohne Satzausnahme greifen nicht/not/cannot und die Pronomen.
    for hinweis in [
        "Diese E-Mail ist nicht für die Öffentlichkeit bestimmt.",
        "Diese E-Mail ist nicht fuer die Oeffentlichkeit bestimmt.",
        "This email is not intended for public disclosure.",
        "We do not accept liability for transmission errors.",
        "I cannot accept liability for damage.",
        "WE DO NOT ACCEPT\nLIABILITY FOR TRANSMISSION ERRORS.",
        "This email is confidential. We do not accept liability for transmission errors.",
    ] {
        let text = format!("Alle Termine passen\nViele Grüße\nAnna Beispiel\n{hinweis}\nTel. 123");
        let e = lese_antwort(&text, &u);
        assert_eq!(e.art, ErgebnisArt::Antwort, "{hinweis}");
        assert!(e.sicher, "{hinweis}");
        assert!(e.werte.iter().all(|w| w.wert == Wert::Passt));
        for rest in ["Mittwoch passt nicht.", "Korrektur\nNur Mittwoch", "Unbekannter Text"] {
            let e = lese_antwort(&format!("{text}\n{rest}"), &u);
            assert_eq!(e.art, ErgebnisArt::Pruefen, "{hinweis}: {rest}");
            assert!(!e.sicher);
            assert!(e.werte.is_empty());
        }
    }
}

#[test]
fn ortszeit_muss_mit_start_und_versatz_uebereinstimmen() {
    let u = umfrage();
    assert!(u.validiere().is_ok());
    // Ohne Konsistenzpruefung bleiben diese einzeln gueltigen Ortszeiten
    // trotz eines anderen UTC-Termins akzeptiert.
    for delta in [-1, 1, 60_000, 86_400_000, -1_796_029_200_000] {
        let mut falsch = u.clone();
        falsch.vorschlaege[0].start += delta;
        falsch.vorschlaege[0].ende += delta;
        assert_eq!(falsch.validiere(), Err("vorschlag_daten"));
        let e = lese_antwort("01.12.2026 10:00", &falsch);
        assert_eq!(e.art, ErgebnisArt::Pruefen);
        assert!(!e.sicher);
        assert!(anfrage_text(&falsch, true).is_err());
        assert!(antwort_links("beispiel@example.org", &falsch, "AB12").is_err());
    }
    for versatz in [0, 120] {
        let mut falsch = u.clone();
        falsch.vorschlaege[0]
            .ortszeit
            .as_mut()
            .unwrap()
            .utc_versatz_min = versatz;
        assert_eq!(falsch.validiere(), Err("vorschlag_daten"));
    }
    // Trotz passender Rechnung sind Versatze ausserhalb +/-14 Stunden ungueltig.
    for versatz in [-841, 841, i16::MIN, i16::MAX] {
        let mut falsch = u.clone();
        let v = &mut falsch.vorschlaege[0];
        v.ortszeit.as_mut().unwrap().utc_versatz_min = versatz;
        let delta = (60 - i64::from(versatz)) * 60_000;
        v.start += delta;
        v.ende += delta;
        assert_eq!(falsch.validiere(), Err("vorschlag_daten"));
    }
    // Auch ein fehlender Versatz darf nicht stillschweigend UTC bedeuten.
    let mut json = serde_json::to_value(&u).unwrap();
    json["vorschlaege"][0]["ortszeit"]
        .as_object_mut()
        .unwrap()
        .remove("utc_versatz_min");
    assert!(serde_json::from_value::<Umfrage>(json).is_err());

    // Unabhaengige UTC-Vektoren: Schaltjahr, Tageswechsel, vor Unix-Epoche.
    for (start, jahr, monat, tag, wochentag, stunde, minute, utc_versatz_min) in [
        (0, 1970, 1, 1, 4, 0, 0, 0),
        (-60_000, 1970, 1, 1, 4, 0, 59, 60),
        (951_782_400_000, 2000, 2, 28, 1, 23, 0, -60),
        (0, 1970, 1, 1, 4, 14, 0, 840),
        (0, 1969, 12, 31, 3, 10, 0, -840),
        (-62_135_596_800_000, 1, 1, 1, 1, 0, 0, 0),
    ] {
        let mut richtig = u.clone();
        richtig.vorschlaege[0].start = start;
        richtig.vorschlaege[0].ende = start + 3_600_000;
        richtig.vorschlaege[0].ortszeit = Some(Ortszeit {
            jahr,
            monat,
            tag,
            wochentag,
            stunde,
            minute,
            utc_versatz_min,
        });
        assert!(richtig.validiere().is_ok(), "{start}");
    }
    let mut ohne = u.clone();
    ohne.vorschlaege[0].ortszeit = None;
    assert!(ohne.validiere().is_ok());
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
    assert!(text.contains("01.12.2026 09:00 UTC"));
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
