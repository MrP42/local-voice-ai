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
    let teilnehmende = liste(&["a@example.org", "b@team.example.org"]);
    let einstufung = einstufen(&teilnehmende, &eigene());
    let bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Intern,
        fingerabdruck: einstufung.fingerabdruck.clone(),
    });
    Anfrage {
        assistent_an: true,
        aufbewahrung_festgelegt: true,
        freigabe: Freigabe::Erteilt,
        quelle,
        kundenprofil_nur_lokal: false,
        profil_nur_lokal: false,
        anbieter_freigegeben: true,
        cloud_fuer_besprechung: true,
        cloud_bei_externen: false,
        einstufung,
        bestaetigung,
    }
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
    let e = einstufen(&liste(&["a@example.org", "B@Example.ORG"]), &eigene());
    assert_eq!(e.einstufung, Einstufung::Intern);
    assert!(e.externe.is_empty());
}

#[test]
fn subdomaene_zaehlt_als_eigen_aber_nicht_aehnlicher_name() {
    let intern = einstufen(&liste(&["a@team.example.org"]), &eigene());
    assert_eq!(intern.einstufung, Einstufung::Intern);
    // "badexample.org" endet auf "example.org", ist aber keine Subdomäne.
    let fremd = einstufen(&liste(&["a@badexample.org"]), &eigene());
    assert_eq!(fremd.einstufung, Einstufung::Extern);
}

#[test]
fn eine_externe_adresse_macht_die_besprechung_extern() {
    let e = einstufen(
        &liste(&["a@example.org", "x@example.com", "x@example.com"]),
        &eigene(),
    );
    assert_eq!(e.einstufung, Einstufung::Extern);
    assert_eq!(e.externe, vec!["x@example.com".to_string()]);
}

#[test]
fn extern_geht_vor_unbekannt() {
    let e = einstufen(&liste(&["Alex", "x@example.com"]), &eigene());
    assert_eq!(e.einstufung, Einstufung::Extern);
    assert_eq!(e.ohne_adresse, 1);
}

#[test]
fn person_ohne_adresse_macht_unbekannt() {
    let e = einstufen(&liste(&["a@example.org", "Alex"]), &eigene());
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
    assert_eq!(e.ohne_adresse, 1);
}

#[test]
fn personen_id_ohne_domaene_ist_unbekannt() {
    let e = einstufen(&liste(&["id:01HABC"]), &eigene());
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
}

#[test]
fn leere_liste_ad_hoc_ist_unbekannt() {
    let e = einstufen(&[], &eigene());
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
}

#[test]
fn ohne_eigene_domaenen_ist_nichts_intern() {
    let e = einstufen(&liste(&["a@example.org"]), &[]);
    assert_eq!(e.einstufung, Einstufung::Unbekannt);
    let ungueltig = einstufen(&liste(&["a@example.org"]), &liste(&["kein-punkt"]));
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
    let a = fingerabdruck(&liste(&["a@example.org", "B@example.org"]));
    let b = fingerabdruck(&liste(&["mailto:b@example.org", "A@Example.org"]));
    assert_eq!(a, b);
}

#[test]
fn fingerabdruck_aendert_sich_bei_neuer_person_auch_ohne_adresse() {
    let basis = fingerabdruck(&liste(&["a@example.org"]));
    assert_ne!(
        basis,
        fingerabdruck(&liste(&["a@example.org", "b@example.org"]))
    );
    let ein_gast = fingerabdruck(&liste(&["a@example.org", "Gast"]));
    let zwei_gaeste = fingerabdruck(&liste(&["a@example.org", "Gast", "Gast"]));
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
    a.einstufung = einstufen(&liste(&["x@example.com"]), &eigene());
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
    a.einstufung = einstufen(
        &liste(&["a@example.org", "b@team.example.org", "c@example.org"]),
        &eigene(),
    );
    assert_eq!(gruende(&pruefen(&a)), vec![Sperrgrund::EinstufungVeraltet]);
    // Eine externe Person kommt hinzu: zusätzlich extern.
    a.einstufung = einstufen(
        &liste(&["a@example.org", "b@team.example.org", "x@example.com"]),
        &eigene(),
    );
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
    a.einstufung = einstufen(&liste(&["a@example.org", "Gast"]), &eigene());
    // Der Nutzer versucht, „intern“ zu bestätigen: zählt nicht (nur verschärfen).
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Intern,
        fingerabdruck: a.einstufung.fingerabdruck.clone(),
    });
    assert_eq!(
        gruende(&pruefen(&a)),
        vec![Sperrgrund::TeilnehmendeUnbekannt]
    );
}

#[test]
fn extern_nur_mit_einstellung_und_bestaetigung() {
    let mut a = offen_intern(Quelle::Cloud);
    a.einstufung = einstufen(&liste(&["a@example.org", "x@example.com"]), &eigene());
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Extern,
        fingerabdruck: a.einstufung.fingerabdruck.clone(),
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
    let fp = a.einstufung.fingerabdruck.clone();
    a.bestaetigung = Some(Bestaetigung {
        einstufung: Einstufung::Extern,
        fingerabdruck: fp,
    });
    assert_eq!(gruende(&pruefen(&a)), vec![Sperrgrund::TeilnehmendeExtern]);
}

#[test]
fn veraltete_bestaetigung_verschaerft_nicht_und_lockert_nicht() {
    let e = einstufen(&liste(&["a@example.org"]), &eigene());
    let fremd = Bestaetigung {
        einstufung: Einstufung::Extern,
        fingerabdruck: "0000000000000000".into(),
    };
    assert_eq!(wirksame_einstufung(&e, Some(&fremd)), Einstufung::Intern);
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
