use super::super::aeusserung::{bewerte, vorschau_text, Entscheidung, Entscheidung::*, Rueckfrage};
use super::super::bestaetigung::Aeusserung;
use super::{Arc, AtomicI64, Freigabe, Ordering, Testuhr};

fn antwort(r: &Rueckfrage<'_>, f: &Freigabe, a: &Aeusserung, id: &str, t: &str) -> Entscheidung {
    r.aeussere(f, "c", id, a, true, t, "de")
}

#[test]
fn regelwerk_ist_exakt_und_begrenzt() {
    use super::super::aeusserung::Urteil::*;
    for t in [
        "ja",
        "ja bitte",
        "mach das",
        "ausfuehren",
        "ja ausfuehren",
        "okay mach",
    ] {
        assert_eq!(bewerte(t, "de"), Ja);
    }
    for t in ["yes", "yes please", "do it", "go ahead"] {
        assert_eq!(bewerte(t, "en"), Ja);
    }
    for t in [
        "nein",
        "nein danke",
        "nicht",
        "stopp",
        "stop",
        "abbrechen",
        "no",
        "no thanks",
        "cancel",
    ] {
        assert_eq!(bewerte(t, "de"), Nein);
    }
    for t in [
        "",
        "ja aber",
        "ja, aber nicht",
        "nicht ja",
        "ja nicht",
        "nein ja",
        "ja nein",
        "ja stop",
        "yes",
        "ja bitte eins zwei drei vier fuenf",
        "vielleicht",
    ] {
        assert_eq!(bewerte(t, "de"), Unklar);
    }
    assert_eq!(bewerte("yes no", "en"), Unklar);
    assert_eq!(bewerte("ja", "fr"), Unklar);
    assert_eq!(bewerte("\u{2003}„JA   BITTE!“\u{2003}", "de"), Ja);
    assert_eq!(bewerte(&format!("{}ja", "\u{2003}".repeat(198)), "de"), Ja);
    assert_eq!(bewerte(&format!("{}ja", " ".repeat(199)), "de"), Unklar);
}

#[test]
fn zuordnung_finalitaet_und_verbrauch() {
    let f = Freigabe::default();
    let frueh = f.aeusserungsbeginn();
    for id in ["", "ungueltig/", &"a".repeat(65)] {
        assert!(Rueckfrage::neu(&f, "bindung", id, "q").is_err());
        assert!(Rueckfrage::neu(&f, "bindung", "c", id).is_err());
    }
    let r = Rueckfrage::neu(&f, "bindung", "c", "q").unwrap();
    let a = f.aeusserungsbeginn();
    assert_eq!(
        r.aeussere(&f, "falsch", "u", &a, true, "ja", "de"),
        Ungueltig
    );
    assert_eq!(antwort(&r, &f, &frueh, "frueh", "ja"), Ungueltig);
    assert_eq!(antwort(&r, &f, &a, "ungueltig/", "ja"), Ungueltig);
    assert_eq!(r.aeussere(&f, "c", "u", &a, false, "ja", "de"), Wartet);
    assert_eq!(antwort(&r, &Freigabe::default(), &a, "u", "ja"), Ungueltig);
    assert_eq!(antwort(&r, &f, &a, "u", "ja"), Freigegeben);
    assert!(f.bereit(r.ereignis().generation, "bindung").is_ok());
    assert_eq!(antwort(&r, &f, &a, "u", "ja"), Ungueltig);
    assert_eq!(antwort(&r, &f, &a, "neu", "nein"), Ungueltig);
    assert_eq!(r.ereignis().state, "freigegeben");
}

#[test]
fn rueckfragen_behalten_frist_und_verraten_keine_inhalte() {
    for zeit in [29_999, 30_000, 31_000] {
        let uhr = Arc::new(Testuhr(AtomicI64::new(0)));
        let f = Freigabe::mit_uhr(uhr.clone());
        let r = Rueckfrage::neu(&f, "GEHEIM_SENTINEL", "c", "q").unwrap();
        let generation = r.ereignis().generation;
        let a = f.aeusserungsbeginn();
        assert_eq!(r.ereignis().text_key, "bestaetigung_frage");
        for id in ["eins", "zwei", "drei", "vier"] {
            assert_eq!(antwort(&r, &f, &a, id, "GEHEIM_SENTINEL"), Rueckfrage);
        }
        assert_eq!(antwort(&r, &f, &a, "eins", "ja"), Ungueltig);
        let e = r.ereignis();
        assert_eq!((e.question_id, e.generation), ("q", generation));
        assert_eq!((e.state, e.timeout_ms), ("rueckfrage", 30_000));
        assert_eq!(e.text_key, "bestaetigung_rueckfrage");
        assert!(!format!("{e:?}").contains("GEHEIM_SENTINEL"));
        assert!(f.bereit(generation, "GEHEIM_SENTINEL").is_err());
        uhr.0.store(zeit, Ordering::SeqCst);
        let erwartet = if zeit < 30_000 {
            Freigegeben
        } else {
            Abgelaufen
        };
        assert_eq!(antwort(&r, &f, &a, "ende", "ja"), erwartet);
        assert_eq!(antwort(&r, &f, &a, "danach", "ja"), Ungueltig);
    }
}

#[test]
fn neue_generation_ende_nein_und_id_limit() {
    let f = Freigabe::default();
    for fall in ["generation", "ende", "nein", "limit"] {
        let r = Rueckfrage::neu(&f, "bindung", "c", "q").unwrap();
        let a = f.aeusserungsbeginn();
        if fall == "generation" {
            f.frage("neu");
            let neu = f.aeusserungsbeginn();
            assert_eq!(antwort(&r, &f, &neu, "neu", "ja"), Abgelaufen);
        } else if fall == "ende" {
            r.beende();
            assert_eq!(antwort(&r, &f, &a, "neu", "ja"), Ungueltig);
        } else if fall == "nein" {
            assert_eq!(antwort(&r, &f, &a, "nein", "nein"), Abgelehnt);
            assert_eq!(antwort(&r, &f, &a, "ja", "ja"), Ungueltig);
        } else {
            for id in 0..32 {
                assert_eq!(antwort(&r, &f, &a, &id.to_string(), "?"), Rueckfrage);
            }
            assert_eq!(antwort(&r, &f, &a, "neu", "ja"), Ungueltig);
        }
        assert!(f.bereit(r.ereignis().generation, "bindung").is_err());
    }
}

#[test]
fn vorschau_validiert_den_namen() {
    assert_eq!(Rueckfrage::RUECKFRAGE, "Bitte antworte mit Ja oder Nein.");
    for (sprache, schreibend, erwartet) in [
        ("de", true, "Soll ich Test ausfuehren? Das aendert Daten."),
        ("en", true, "Shall I run Test? This changes data."),
        ("en", false, "Shall I run Test?"),
    ] {
        assert_eq!(
            vorschau_text("Test", schreibend, sprache).as_deref(),
            Some(erwartet)
        );
    }
    for name in [
        "",
        " ",
        "a\nb",
        "a\0b",
        "a\u{2028}b",
        "\"ja\"",
        "'ja'",
        "„ja“",
        &"a".repeat(61),
    ] {
        assert!(vorschau_text(name, false, "de").is_none());
    }
    assert!(vorschau_text(&"ä".repeat(60), false, "de").is_some());
    assert!(vorschau_text("Test", false, "fr").is_none());
}
