//! Tests des Assistent-Moduls (ADR-0028 A2): feste Uhr, Schalter, Ansage, Messung ohne Inhalte, Belegregel.

use chrono::NaiveDate;
use wai_assistent::{
    Beleg, FesteUhr, Grund, QuellenMap, Satz, SatzArt, Sprache, Wochentag, Zeitpunkt,
};

use super::uhr::{zeitpunkt_aus, zeitpunkt_lokal, SystemUhr};
use super::*;

fn feste_uhr() -> FesteUhr {
    FesteUhr(Zeitpunkt {
        jahr: 2026,
        monat: 11,
        tag: 8,
        stunde: 14,
        minute: 5,
        wochentag: Wochentag::Sonntag,
    })
}

fn einstufen_text(text: &str, sprache: Sprache) -> Option<EinstufungDto> {
    einstufen_mit(true, text, sprache, tabelle(), &feste_uhr(), None).expect("keine Panne")
}

#[test]
fn s0_uhrzeit_aus_fester_uhr_ohne_modell() {
    let e = einstufen_text("Wie spät ist es?", Sprache::De).expect("eingestuft");
    assert_eq!(e.stufe, StufeDto::S0);
    assert_eq!(e.absicht, "uhrzeit");
    assert_eq!(e.ansage, None, "S0 hat keine Ansage, nur die Antwort");
    let antwort = e.antwort.expect("Vorlagenantwort");
    assert!(antwort.contains("14:05"), "Uhrzeit aus der Uhr: {antwort}");
}

#[test]
fn s0_datum_englisch() {
    let e = einstufen_text("what time is it", Sprache::En).expect("eingestuft");
    assert_eq!(e.stufe, StufeDto::S0);
    assert!(e.antwort.is_some());
}

#[test]
fn s1_mail_hat_ansage_und_keine_antwort() {
    let e = einstufen_text("Habe ich neue Mails?", Sprache::De).expect("eingestuft");
    assert_eq!(e.stufe, StufeDto::S1);
    assert_eq!(e.antwort, None, "ab S1 kein Vorlagentext als Antwort");
    assert_eq!(e.ansage.as_deref(), Some("Ich schaue in deine Mails."));
}

#[test]
fn s3_auftrag_hat_ansage() {
    let e = einstufen_text("Entwirf eine Zusage an den Kunden", Sprache::De).expect("eingestuft");
    assert_eq!(e.stufe, StufeDto::S3);
    assert!(e.ansage.is_some());
}

#[test]
fn s0_nur_bei_exakter_gleichheit() {
    let e = einstufen_text(
        "Wie spät ist es in Tokio und was kostet ein Flug?",
        Sprache::De,
    )
    .expect("eingestuft");
    assert_ne!(
        e.stufe,
        StufeDto::S0,
        "Falle: beginnt wie S0, ist aber mehr"
    );
    assert_eq!(e.antwort, None);
}

#[test]
fn schalter_aus_stuft_nicht_ein() {
    let e = einstufen_mit(
        false,
        "Wie spät ist es?",
        Sprache::De,
        tabelle(),
        &feste_uhr(),
        None,
    );
    assert_eq!(e, Ok(None));
}

#[test]
fn leerer_oder_zu_langer_text_wird_nicht_eingestuft() {
    assert_eq!(einstufen_text("   ", Sprache::De), None);
    assert_eq!(
        einstufen_text(&"a ".repeat(MAX_TEXT_ZEICHEN), Sprache::De),
        None
    );
}

#[test]
fn sprachcode_zu_sprache() {
    for code in ["en", "en-US", "en-GB", "EN_gb", " en "] {
        assert_eq!(sprache_aus(code), Sprache::En, "{code:?}");
    }
    for code in ["de", "de-DE", "de-AT", "de-CH", "de_ch"] {
        assert_eq!(sprache_aus(code), Sprache::De, "{code:?}");
    }
    // Unbekannt (auch „englisch aussehende“ Codes): Deutsch.
    for code in ["fr", "fr-FR", "english", "ent", "en-AU", "", "xx"] {
        assert_eq!(sprache_aus(code), Sprache::De, "{code:?}");
    }
}

#[test]
fn standard_einstellung_ist_an_und_alte_datei_bleibt_an() {
    assert!(crate::settings::get_default_settings().assistent_einstufung_aktiv);
    let alt: crate::settings::AppSettings =
        serde_json::from_value(serde_json::json!({ "meeting_language": "de" })).unwrap();
    assert!(alt.assistent_einstufung_aktiv, "fehlender Schluessel = an");
}

#[test]
fn zeitpunkt_aus_chrono_mit_wochentag() {
    // 8.11.2026 ist ein Sonntag.
    let z = zeitpunkt_aus(
        NaiveDate::from_ymd_opt(2026, 11, 8)
            .unwrap()
            .and_hms_opt(14, 5, 0)
            .unwrap(),
    );
    assert_eq!(z, feste_uhr().0);
}

#[test]
fn sommerzeit_ist_im_versatz_der_zone_enthalten() {
    use chrono::{FixedOffset, TimeZone, Utc};
    // 12:05 UTC: in Deutschland im Sommer (MESZ, +2) 14:05, im Winter (MEZ, +1) 13:05.
    let sommer = FixedOffset::east_opt(2 * 3600).unwrap();
    let winter = FixedOffset::east_opt(3600).unwrap();
    let juli = Utc.with_ymd_and_hms(2026, 7, 1, 12, 5, 0).unwrap();
    let januar = Utc.with_ymd_and_hms(2026, 1, 15, 12, 5, 0).unwrap();
    assert_eq!(zeitpunkt_lokal(juli.with_timezone(&sommer)).stunde, 14);
    assert_eq!(zeitpunkt_lokal(januar.with_timezone(&winter)).stunde, 13);
    let z = zeitpunkt_lokal(juli.with_timezone(&sommer));
    let e = einstufen_mit(
        true,
        "Wie spät ist es?",
        Sprache::De,
        tabelle(),
        &FesteUhr(z),
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(e.antwort.as_deref(), Some("Es ist 14:05 Uhr."));
    // Tagesgrenze der Zone: 23:30 UTC ist im Sommer schon der naechste Tag.
    let spaet = Utc.with_ymd_and_hms(2026, 7, 1, 23, 30, 0).unwrap();
    let z = zeitpunkt_lokal(spaet.with_timezone(&sommer));
    assert_eq!((z.tag, z.stunde, z.minute), (2, 1, 30));
}

#[test]
fn system_uhr_liefert_gueltige_werte() {
    let z = SystemUhr.jetzt();
    assert!((1..=12).contains(&z.monat));
    assert!((1..=31).contains(&z.tag));
    assert!(z.stunde < 24 && z.minute < 60);
}

#[test]
fn gemeinsame_tabelle_ist_nur_einmal_geladen() {
    assert!(std::ptr::eq(tabelle(), tabelle()));
    assert!(std::ptr::eq(ansagen(), ansagen()));
}

fn messung(id: &str) -> MessungDto {
    MessungDto {
        beitrag_id: id.into(),
        erste_ansage_ms: Some(120),
        erster_inhalt_ms: Some(900),
        fertig_ms: None,
        warm: true,
        weg: WegDto::Lokal,
        stufe: StufeDto::S1,
    }
}

#[test]
fn messung_traegt_nur_zeiten_und_merkmale() {
    let m = messung("turn-01HX.abc").pruefen().expect("gueltig");
    assert_eq!(m.erste_ansage_ms, Some(120));
    assert_eq!(m.erster_inhalt_ms, Some(900));
    assert_eq!(m.fertig_ms, None);
    let zeile = messung_zeile(&m);
    assert!(
        zeile.contains("ansage_ms=120") && zeile.contains("fertig_ms=-"),
        "{zeile}"
    );
}

#[test]
fn messung_lehnt_unschlichte_kennung_ab() {
    for id in [
        "",
        "mit leerzeichen",
        "Wie spät ist es",
        &"x".repeat(MAX_ID_ZEICHEN + 1),
    ] {
        assert!(messung(id).pruefen().is_err(), "{id:?}");
    }
}

#[test]
fn belegregel_verwirft_satz_ohne_beleg_und_gibt_belegte_aus() {
    let quellen = QuellenMap::new().mit("mail-1", "Der Termin ist am 12.11.2026 um 10:00 Uhr.");
    let belegt = Satz {
        text: "Der Termin ist am 12.11.2026 um 10:00 Uhr.".into(),
        art: SatzArt::Aussage,
        beleg: Some(Beleg::Quelle {
            quelle_id: "mail-1".into(),
            auszug: "Der Termin ist am 12.11.2026 um 10:00 Uhr.".into(),
        }),
    };
    let ohne = Satz {
        text: "Der Kunde ist sehr zufrieden.".into(),
        art: SatzArt::Aussage,
        beleg: None,
    };
    let ansage = Satz {
        text: "Ich schaue in deine Mails.".into(),
        art: SatzArt::Ansage,
        beleg: None,
    };
    let angebot = Satz {
        text: wai_assistent::keine_quelle_angebot(Sprache::De).into(),
        art: SatzArt::Ansage,
        beleg: None,
    };
    let erfunden = Satz {
        text: "Ich bin eine erfundene Ansage.".into(),
        art: SatzArt::Ansage,
        beleg: None,
    };
    let erg = belege_pruefen(
        &[
            belegt.clone(),
            ohne.clone(),
            ansage.clone(),
            angebot.clone(),
            erfunden.clone(),
        ],
        &quellen,
    );
    assert_eq!(erg.ausgegeben, vec![belegt, ansage, angebot]);
    assert_eq!(erg.verworfen.len(), 2);
    assert_eq!(erg.verworfen[0], (ohne, Grund::OhneBeleg));
    assert_eq!(erg.verworfen[1].0, erfunden);
}
