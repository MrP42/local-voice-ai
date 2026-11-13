//! Tests der Sperren des Besprechungsassistenten (G9a). Nur synthetische Adressen
//! (`example.org`, `example.com`).

use crate::assistent::*;

fn eigene() -> Vec<String> {
    vec!["example.org".into()]
}

fn liste(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

/// Anfrage, bei der lokal und Cloud erlaubt sind (interne Liste, bestätigt).
fn offen_intern(quelle: Quelle) -> Anfrage {
    let teilnehmende = liste(&["a@example.org", "b@example.org"]);
    let einstufung = einstufen(&teilnehmende, &eigene(), true);
    let bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Intern,
        fingerabdruck: einstufung.fingerabdruck.clone(),
    });
    Anfrage {
        assistent_an: true,
        aufbewahrung_festgelegt: true,
        freigabe: Freigabe::Erteilt,
        quelle,
        pfad: Pfad::LiveChat,
        kundenprofil_nur_lokal: false,
        profil_nur_lokal: false,
        anbieter_freigegeben: true,
        cloud_fuer_besprechung: true,
        cloud_bei_externen: false,
        teilnehmende,
        liste_aus_termin: true,
        eigene_domaenen: eigene(),
        bestaetigung,
    }
}

/// Fingerabdruck der Liste in `a` (wie die App ihn bei der Bestätigung speichert).
fn fp(a: &Anfrage) -> String {
    einstufen(&a.teilnehmende, &a.eigene_domaenen, a.liste_aus_termin).fingerabdruck
}

fn gruende(e: &Entscheidung) -> Vec<Sperrgrund> {
    match e {
        Entscheidung::Erlaubt { .. } => Vec::new(),
        Entscheidung::Gesperrt { gruende } => gruende.clone(),
    }
}

// -- Einstufung ---------------------------------------------------------------

#[test]
fn alle_aus_eigenen_domaenen_sind_intern() {
    let e = einstufen(&liste(&["a@example.org", "B@Example.ORG"]), &eigene(), true);
    assert_eq!(e.einstufung, Einstufung::Intern);
    assert!(e.externe.is_empty());
}

#[test]
fn subdomaene_nur_mit_stern_eintrag_und_nie_aehnlicher_name() {
    let genau = einstufen(&liste(&["a@team.example.org"]), &eigene(), true);
    assert_eq!(genau.einstufung, Einstufung::Extern);
    let stern = liste(&["*.example.org"]);
    let intern = einstufen(&liste(&["a@team.example.org"]), &stern, true);
    assert_eq!(intern.einstufung, Einstufung::Intern);
    let fremd = einstufen(&liste(&["a@badexample.org"]), &stern, true);
    assert_eq!(fremd.einstufung, Einstufung::Extern);
    // "badexample.org" endet auf "example.org", ist aber keine Subdomäne.
    let fremd = einstufen(&liste(&["a@badexample.org"]), &eigene(), true);
    assert_eq!(fremd.einstufung, Einstufung::Extern);
}

#[test]
fn eine_externe_adresse_macht_die_besprechung_extern() {
    let e = einstufen(
        &liste(&["a@example.org", "x@example.com", "x@example.com"]),
        &eigene(),
        true,
    );
    assert_eq!(e.einstufung, Einstufung::Extern);
    assert_eq!(e.externe, vec!["x@example.com".to_string()]);
}

#[test]
fn extern_geht_vor_unbekannt() {
    let e = einstufen(&liste(&["Alex", "x@example.com"]), &eigene(), true);
    assert_eq!(e.einstufung, Einstufung::Extern);
    assert_eq!(e.ohne_adresse, 1);
}

#[test]
fn person_ohne_adresse_macht_unbekannt() {
    let e = einstufen(&liste(&["a@example.org", "Alex"]), &eigene(), true);
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
    assert_eq!(e.ohne_adresse, 1);
}

#[test]
fn personen_id_ohne_domaene_ist_unbekannt() {
    let e = einstufen(&liste(&["id:01HABC"]), &eigene(), true);
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
}

#[test]
fn leere_liste_ad_hoc_ist_unbekannt() {
    let e = einstufen(&[], &eigene(), true);
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
}

#[test]
fn ohne_eigene_domaenen_ist_nichts_intern() {
    let e = einstufen(&liste(&["a@example.org"]), &[], true);
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
    let ungueltig = einstufen(&liste(&["a@example.org"]), &liste(&["kein-punkt"]), true);
    assert_eq!(ungueltig.einstufung, Einstufung::Unbekannt);
}

#[test]
fn domaenen_werden_normalisiert() {
    assert_eq!(
        domaene_normalisieren(" @Example.ORG "),
        Some("example.org".into())
    );
    assert_eq!(domaene_normalisieren("example..org"), None);
    assert_eq!(domaene_normalisieren(".example.org"), None);
    assert_eq!(domaene_normalisieren("exa mple.org"), None);
}

#[test]
fn vorschlag_aus_eigenen_adressen() {
    let v = eigene_domaenen_vorschlag(&liste(&[
        "Ich <ich@Example.org>",
        "zweit@example.org",
        "kaputt",
    ]));
    assert_eq!(v, vec!["example.org".to_string()]);
}

#[test]
fn fingerabdruck_ist_reihenfolge_und_schreibweise_egal() {
    let a = fingerabdruck(&liste(&["a@example.org", "B@example.org"]), &eigene(), true);
    let b = fingerabdruck(
        &liste(&["mailto:b@example.org", "A@Example.org"]),
        &eigene(),
        true,
    );
    assert_eq!(a, b);
}

#[test]
fn fingerabdruck_aendert_sich_bei_neuer_person_auch_ohne_adresse() {
    let basis = fingerabdruck(&liste(&["a@example.org"]), &eigene(), true);
    assert_ne!(
        basis,
        fingerabdruck(&liste(&["a@example.org", "b@example.org"]), &eigene(), true)
    );
    let ein_gast = fingerabdruck(&liste(&["a@example.org", "Gast"]), &eigene(), true);
    let zwei_gaeste = fingerabdruck(&liste(&["a@example.org", "Gast", "Gast"]), &eigene(), true);
    assert_ne!(basis, ein_gast);
    assert_ne!(ein_gast, zwei_gaeste);
}

// -- Prüfung -------------------------------------------------------------------

#[test]
fn intern_bestaetigt_erlaubt_lokal_und_cloud() {
    assert!(pruefen(&offen_intern(Quelle::Lokal)).erlaubt());
    assert_eq!(
        pruefen(&offen_intern(Quelle::Cloud)),
        Entscheidung::Erlaubt {
            einstufung: Einstufung::Intern
        }
    );
}

#[test]
fn grundsperren_gelten_auch_lokal_und_werden_alle_genannt() {
    let mut a = offen_intern(Quelle::Lokal);
    a.assistent_an = false;
    a.aufbewahrung_festgelegt = false;
    a.freigabe = Freigabe::Offen;
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![
            Sperrgrund::AssistentAus,
            Sperrgrund::AufbewahrungOffen,
            Sperrgrund::FreigabeOffen
        ]
    );
    a = offen_intern(Quelle::Lokal);
    a.freigabe = Freigabe::Abgelehnt;
    assert_eq!(gruende(&pruefen(&a)), vec![Sperrgrund::FreigabeAbgelehnt]);
}

#[test]
fn lokal_braucht_keine_einstufung() {
    let mut a = offen_intern(Quelle::Lokal);
    a.teilnehmende = liste(&["x@example.com"]);
    a.bestaetigung = None;
    a.anbieter_freigegeben = false;
    a.kundenprofil_nur_lokal = true;
    a.profil_nur_lokal = true;
    a.cloud_fuer_besprechung = false;
    assert!(pruefen(&a).erlaubt());
}

#[test]
fn cloud_sperren_aus_profil_anbieter_und_wahl() {
    let mut a = offen_intern(Quelle::Cloud);
    a.kundenprofil_nur_lokal = true;
    a.profil_nur_lokal = true;
    a.anbieter_freigegeben = false;
    a.cloud_fuer_besprechung = false;
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![
            Sperrgrund::KundenprofilNurLokal,
            Sperrgrund::ProfilNurLokal,
            Sperrgrund::AnbieterNichtFreigegeben,
            Sperrgrund::CloudNichtGewaehlt
        ]
    );
}

#[test]
fn cloud_ohne_bestaetigung_gesperrt() {
    let mut a = offen_intern(Quelle::Cloud);
    a.bestaetigung = None;
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![Sperrgrund::EinstufungUnbestaetigt]
    );
}

#[test]
fn nachzuegler_macht_bestaetigung_ungueltig_und_sperrt_cloud() {
    let mut a = offen_intern(Quelle::Cloud);
    // Eine weitere interne Person kommt hinzu: Liste geändert, Bestätigung veraltet.
    a.teilnehmende = liste(&["a@example.org", "b@example.org", "c@example.org"]);
    assert_eq!(gruende(&pruefen(&a)), vec![Sperrgrund::EinstufungVeraltet]);
    // Eine externe Person kommt hinzu: zusätzlich extern.
    a.teilnehmende = liste(&["a@example.org", "b@example.org", "x@example.com"]);
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![
            Sperrgrund::EinstufungVeraltet,
            Sperrgrund::TeilnehmendeExtern
        ]
    );
}

#[test]
fn unbekannt_sperrt_cloud_auch_mit_einstellung_fuer_externe() {
    let mut a = offen_intern(Quelle::Cloud);
    a.cloud_bei_externen = true;
    a.teilnehmende = liste(&["a@example.org", "Gast"]);
    // Der Nutzer versucht, „intern“ zu bestätigen: zählt nicht (nur verschärfen).
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Intern,
        fingerabdruck: fp(&a),
    });
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![
            Sperrgrund::EinstufungUnbestaetigt,
            Sperrgrund::TeilnehmendeUnbekannt
        ]
    );
}

#[test]
fn extern_nur_mit_einstellung_und_bestaetigung() {
    let mut a = offen_intern(Quelle::Cloud);
    a.teilnehmende = liste(&["a@example.org", "x@example.com"]);
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Extern,
        fingerabdruck: fp(&a),
    });
    assert_eq!(gruende(&pruefen(&a)), vec![Sperrgrund::TeilnehmendeExtern]);
    a.cloud_bei_externen = true;
    assert_eq!(
        pruefen(&a),
        Entscheidung::Erlaubt {
            einstufung: Einstufung::Extern
        }
    );
}

#[test]
fn nutzer_kann_auf_extern_verschaerfen() {
    let mut a = offen_intern(Quelle::Cloud);
    let fingerabdruck = fp(&a);
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Extern,
        fingerabdruck,
    });
    assert_eq!(gruende(&pruefen(&a)), vec![Sperrgrund::TeilnehmendeExtern]);
}

#[test]
fn veraltete_bestaetigung_zaehlt_nicht() {
    let mut a = offen_intern(Quelle::Cloud);
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Intern,
        fingerabdruck: "0".repeat(64),
    });
    assert_eq!(gruende(&pruefen(&a)), vec![Sperrgrund::EinstufungVeraltet]);
}

#[test]
fn geaenderte_eigene_domaenen_machen_bestaetigung_ungueltig() {
    let mut a = offen_intern(Quelle::Cloud);
    a.eigene_domaenen = liste(&["example.org", "*.example.org"]);
    assert_eq!(gruende(&pruefen(&a)), vec![Sperrgrund::EinstufungVeraltet]);
}

#[test]
fn ungueltige_domaenen_sind_nie_intern() {
    for adresse in [
        "x@evil.com?.example.org",
        "x@evil.com/.example.org",
        "x@evil.com\0.example.org",
        "x@a..example.org",
        "x@-a.example.org",
        "x@example.org:443",
        "x@exampl\u{0435}.org",
        "x@example\u{3002}org",
    ] {
        let e = einstufen(&liste(&[adresse]), &liste(&["*.example.org"]), true);
        assert_ne!(e.einstufung, Einstufung::Intern, "{adresse:?}");
    }
}

#[test]
fn oeffentliche_mail_domaenen_sind_nie_eigen() {
    assert!(eigene_domaenen_vorschlag(&liste(&["ich@gmail.com", "ich@web.de"])).is_empty());
    let e = einstufen(
        &liste(&["x@gmail.com"]),
        &liste(&["gmail.com", "*.co.uk"]),
        true,
    );
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
}

#[test]
fn zweite_adresse_im_anzeigenamen_zaehlt_nicht() {
    // Zwei Adressen in einem Eintrag sind nie eindeutig: Person ohne Adresse.
    let e = einstufen(&liste(&["a@example.org <x@example.com>"]), &eigene(), true);
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
}

#[test]
fn doppelte_personen_zaehlen_einmal() {
    let a = fingerabdruck(
        &liste(&["a@example.org", "A@EXAMPLE.ORG", "mailto:a@example.org"]),
        &eigene(),
        true,
    );
    assert_eq!(
        a,
        fingerabdruck(&liste(&["a@example.org"]), &eigene(), true)
    );
}

#[test]
fn strenge_reihenfolge_ist_festgelegt() {
    assert!(Einstufung::Intern < Einstufung::Unbekannt);
    assert!(Einstufung::Unbekannt < Einstufung::Extern);
}

#[test]
fn entscheidung_ist_als_json_stabil() {
    let gesperrt = Entscheidung::Gesperrt {
        gruende: vec![Sperrgrund::TeilnehmendeExtern],
    };
    assert_eq!(
        serde_json::to_string(&gesperrt).unwrap(),
        r#"{"ergebnis":"gesperrt","gruende":["teilnehmende_extern"]}"#
    );
}

// -- Fixrunde 2 (Prüfung der Vertretung) ---------------------------------------

#[test]
fn mehrere_adressen_in_einem_eintrag_sind_unbekannt() {
    for eintrag in [
        "Anna <a@example.org>; x@example.com",
        "<a@example.org>,x@example.com",
        "a@example.org, x@example.com",
        "a@example.org;x@example.com",
        "Anna <a@example.org> Bert",
        "<a@example.org> <b@example.org>",
        "a@example.org x@example.com",
        "<a@example.org,x@example.com>",
    ] {
        let e = einstufen(&liste(&[eintrag]), &eigene(), true);
        assert_eq!(e.einstufung, Einstufung::Unbekannt, "{eintrag:?}");
        assert_eq!(e.ohne_adresse, 1, "{eintrag:?}");
    }
    // Komma im Anzeigenamen ist kein zweiter Eintrag.
    let e = einstufen(&liste(&["Muster, Anna <a@example.org>"]), &eigene(), true);
    assert_eq!(e.einstufung, Einstufung::Intern);
}

#[test]
fn mehrfachadresse_sperrt_cloud_trotz_bestaetigung() {
    let mut a = offen_intern(Quelle::Cloud);
    a.teilnehmende = liste(&["Anna <a@example.org>; x@example.com"]);
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Intern,
        fingerabdruck: fp(&a),
    });
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![
            Sperrgrund::EinstufungUnbestaetigt,
            Sperrgrund::TeilnehmendeUnbekannt
        ]
    );
}

#[test]
fn nicht_ascii_in_der_adresse_ist_unbekannt() {
    // Kelvin-Zeichen U+212A würde beim Kleinschreiben zu „k“.
    for eintrag in ["a@\u{212A}example.org", "\u{212A}@example.org"] {
        let e = einstufen(
            &liste(&[eintrag]),
            &liste(&["kexample.org", "example.org"]),
            true,
        );
        assert_eq!(e.einstufung, Einstufung::Unbekannt, "{eintrag:?}");
    }
}

#[test]
fn ad_hoc_liste_ist_nie_intern_und_sperrt_cloud() {
    let e = einstufen(&liste(&["a@example.org"]), &eigene(), false);
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
    let mut a = offen_intern(Quelle::Cloud);
    a.liste_aus_termin = false;
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Intern,
        fingerabdruck: fp(&a),
    });
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![
            Sperrgrund::EinstufungUnbestaetigt,
            Sperrgrund::ListeOhneTermin
        ]
    );
    // Auch mit „Cloud bei Externen“ und Externen in der Ad-hoc-Liste.
    a.cloud_bei_externen = true;
    a.teilnehmende = liste(&["a@example.org", "x@example.com"]);
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Extern,
        fingerabdruck: fp(&a),
    });
    assert_eq!(gruende(&pruefen(&a)), vec![Sperrgrund::ListeOhneTermin]);
    // Lokal bleibt erlaubt.
    a.quelle = Quelle::Lokal;
    assert!(pruefen(&a).erlaubt());
}

#[test]
fn herkunft_der_liste_aendert_den_fingerabdruck() {
    let l = liste(&["a@example.org"]);
    assert_ne!(
        fingerabdruck(&l, &eigene(), true),
        fingerabdruck(&l, &eigene(), false)
    );
}

#[test]
fn mcp_wird_immer_wie_cloud_geprueft() {
    let mut a = offen_intern(Quelle::Lokal);
    a.pfad = Pfad::Mcp;
    a.teilnehmende = liste(&["x@example.com"]);
    a.bestaetigung = None;
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![
            Sperrgrund::EinstufungUnbestaetigt,
            Sperrgrund::TeilnehmendeExtern
        ]
    );
    // Kundenprofil „nur lokal“ sperrt MCP ganz.
    let mut b = offen_intern(Quelle::Lokal);
    b.pfad = Pfad::Mcp;
    b.kundenprofil_nur_lokal = true;
    assert_eq!(
        gruende(&pruefen(&b)),
        vec![Sperrgrund::KundenprofilNurLokal]
    );
    // Die anderen Pfade lokal: keine Cloud-Prüfung.
    for pfad in [Pfad::LiveChat, Pfad::Stand, Pfad::Auswertung] {
        let mut c = offen_intern(Quelle::Lokal);
        c.pfad = pfad;
        c.teilnehmende = liste(&["x@example.com"]);
        assert!(pruefen(&c).erlaubt(), "{pfad:?}");
    }
}

#[test]
fn extern_und_unbekannt_gemischt_sperrt_trotz_einstellung() {
    let mut a = offen_intern(Quelle::Cloud);
    a.cloud_bei_externen = true;
    a.teilnehmende = liste(&["x@example.com", "Alex"]);
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Extern,
        fingerabdruck: fp(&a),
    });
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![Sperrgrund::TeilnehmendeUnbekannt]
    );
    a.teilnehmende = liste(&["Alex"]);
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Extern,
        fingerabdruck: fp(&a),
    });
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![Sperrgrund::TeilnehmendeUnbekannt]
    );
}

#[test]
fn leere_liste_auf_extern_bestaetigt_bleibt_gesperrt() {
    let mut a = offen_intern(Quelle::Cloud);
    a.cloud_bei_externen = true;
    a.teilnehmende = Vec::new();
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Extern,
        fingerabdruck: fp(&a),
    });
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![Sperrgrund::TeilnehmendeUnbekannt]
    );
}

#[test]
fn freemail_marken_unter_jeder_landesendung_sind_nie_eigen() {
    let v = eigene_domaenen_vorschlag(&liste(&[
        "ich@gmx.com",
        "ich@yahoo.fr",
        "ich@yahoo.co.uk",
        "ich@pm.me",
        "ich@tutanota.de",
        "ich@outlook.at",
        "ich@example.org",
    ]));
    assert_eq!(v, vec!["example.org".to_string()]);
    for eigen in ["gmx.com", "*.gmx.com", "yahoo.fr", "pm.me", "*.github.io"] {
        let e = einstufen(&liste(&["fremd@gmx.com"]), &liste(&[eigen]), true);
        assert_ne!(e.einstufung, Einstufung::Intern, "{eigen:?}");
    }
    // Firmen, deren Name nur mit einer Marke beginnt, bleiben zulässig.
    let e = einstufen(
        &liste(&["a@mail.example.org"]),
        &liste(&["mail.example.org"]),
        true,
    );
    assert_eq!(e.einstufung, Einstufung::Intern);
}

#[test]
fn laender_suffixe_sind_nie_eigen_auch_nicht_mit_stern() {
    for (eigen, adresse) in [
        ("*.co.at", "fremd@firma.co.at"),
        ("co.at", "fremd@co.at"),
        ("*.com.au", "fremd@firma.com.au"),
        ("*.or.at", "fremd@verein.or.at"),
        ("*.co.uk", "fremd@firma.co.uk"),
    ] {
        let e = einstufen(&liste(&[adresse]), &liste(&[eigen]), true);
        assert_eq!(e.einstufung, Einstufung::Unbekannt, "{eigen:?}");
    }
    // Eine Firma unter einem Länder-Suffix ist zulässig.
    let e = einstufen(
        &liste(&["a@team.example.co.at"]),
        &liste(&["*.example.co.at"]),
        true,
    );
    assert_eq!(e.einstufung, Einstufung::Intern);
}

#[test]
fn stern_eintrag_gilt_auch_fuer_die_basisdomaene() {
    let e = einstufen(&liste(&["a@example.org"]), &liste(&["*.example.org"]), true);
    assert_eq!(e.einstufung, Einstufung::Intern);
}

// -- Fixrunde 3 ------------------------------------------------------------------

#[test]
fn bestaetigung_unbekannt_sperrt_cloud_und_mcp() {
    let mut a = offen_intern(Quelle::Cloud);
    a.teilnehmende = liste(&["a@example.org"]);
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Unbekannt,
        fingerabdruck: fp(&a),
    });
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![Sperrgrund::TeilnehmendeUnbekannt]
    );
    a.quelle = Quelle::Lokal;
    a.pfad = Pfad::Mcp;
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![Sperrgrund::TeilnehmendeUnbekannt]
    );
    // Neben Externen mit „Cloud bei Externen“: weiter gesperrt.
    let mut b = offen_intern(Quelle::Cloud);
    b.cloud_bei_externen = true;
    b.teilnehmende = liste(&["a@example.org", "x@example.com"]);
    b.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Unbekannt,
        fingerabdruck: fp(&b),
    });
    assert_eq!(
        gruende(&pruefen(&b)),
        vec![Sperrgrund::TeilnehmendeUnbekannt]
    );
}

#[test]
fn bestaetigung_lockerer_als_berechnet_gilt_als_unbestaetigt() {
    let mut a = offen_intern(Quelle::Cloud);
    a.cloud_bei_externen = true;
    a.teilnehmende = liste(&["a@example.org", "x@example.com"]);
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Intern,
        fingerabdruck: fp(&a),
    });
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![Sperrgrund::EinstufungUnbestaetigt]
    );
}

#[test]
fn weitere_oeffentliche_domaenen_und_suffixe_sind_nie_eigen() {
    let v = eigene_domaenen_vorschlag(&liste(&[
        "ich@bluewin.ch",
        "ich@email.de",
        "ich@fastmail.com",
        "ich@qq.com",
        "ich@rocketmail.com",
    ]));
    assert!(v.is_empty(), "{v:?}");
    for (eigen, adresse) in [
        ("*.me.uk", "fremd@x.me.uk"),
        ("*.sch.uk", "fremd@x.sch.uk"),
        ("*.uk.com", "fremd@x.uk.com"),
        ("*.co.com", "fremd@x.co.com"),
        ("*.eu.org", "fremd@x.eu.org"),
        ("*.web.app", "fremd@x.web.app"),
        ("*.amazonaws.com", "fremd@x.amazonaws.com"),
    ] {
        let e = einstufen(&liste(&[adresse]), &liste(&[eigen]), true);
        assert_ne!(e.einstufung, Einstufung::Intern, "{eigen:?}");
    }
}

#[test]
fn nicht_ascii_in_eigenen_domaenen_ist_ungueltig() {
    assert_eq!(domaene_normalisieren("\u{212A}example.org"), None);
}
