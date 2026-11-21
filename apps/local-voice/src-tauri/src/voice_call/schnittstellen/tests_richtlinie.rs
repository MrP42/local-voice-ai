use std::cell::Cell;

use super::super::richtlinie::{
    fuer_modell, werkzeugdefinitionen_fuer_modell, Richtlinie, Verweigernd,
};
use super::{
    aufruf, eintraege, fuehre_aus, protokoll, Abbruch, Antwortrunde, Aufrufergebnis, Dns, Netz,
    Wert,
};

struct Politik {
    netz: bool,
    geschuetzt: bool,
    cloud: bool,
    widerruf_ab: usize,
    pruefungen: Cell<usize>,
}
impl Politik {
    fn neu() -> Self {
        Self {
            netz: true,
            geschuetzt: false,
            cloud: true,
            widerruf_ab: usize::MAX,
            pruefungen: Cell::new(0),
        }
    }
}
impl Richtlinie for Politik {
    fn netzabfluss_erlaubt(&self, kennung: &str) -> bool {
        assert_eq!(kennung, "test.api");
        self.netz
    }
    fn widerrufen(&self, kennung: &str) -> bool {
        assert_eq!(kennung, "test.api");
        let anzahl = self.pruefungen.get() + 1;
        self.pruefungen.set(anzahl);
        anzahl >= self.widerruf_ab
    }
    fn cloud_werkzeugdefinitionen_erlaubt(&self) -> bool {
        self.cloud
    }
    fn cloud_ergebnis_erlaubt(&self, kennung: &str) -> bool {
        assert_eq!(kennung, "test.api");
        self.cloud
    }
    fn geschuetzt(&self) -> bool {
        self.geschuetzt
    }
}

fn mit_richtlinie(
    e: &[Wert],
    r: &mut Antwortrunde,
    dns: &mut Dns,
    netz: &mut Netz,
    politik: &impl Richtlinie,
) -> Aufrufergebnis {
    fuehre_aus(
        e,
        &["test.api".into()],
        "test_dapi",
        "{}",
        r,
        &Abbruch::neu(),
        None,
        politik,
        dns,
        netz,
        "test",
        "de",
        "2026-11-21T12:00:00Z",
    )
}

fn ohne_verbrauch(e: &[Wert], politik: &impl Richtlinie) {
    let mut r = Antwortrunde::neu();
    let mut dns = Dns::default();
    let mut netz = Netz::default();
    let a = mit_richtlinie(e, &mut r, &mut dns, &mut netz, politik);
    assert_eq!(a.ergebnis, "abgelehnt");
    assert!(a.daten.is_none());
    assert!(a.protokollzeile.is_empty());
    assert_eq!((dns.0, netz.aufbauten), (0, 0));
    assert!(!r.laeuft());
    // Gegenprobe: jede Ablehnung laesst noch alle fuenf echten Aufrufe zu.
    let ids = vec!["test.api".into()];
    for _ in 0..5 {
        let a = aufruf(e, &ids, "test_dapi", "{}", &mut r, &mut dns, &mut netz);
        assert_eq!(a.ergebnis, "ok");
        assert!(a.daten.is_some());
        protokoll(&a);
    }
    let a = aufruf(e, &ids, "test_dapi", "{}", &mut r, &mut dns, &mut netz);
    assert_eq!(a.ergebnis, "abgelehnt");
    assert_eq!((dns.0, netz.aufbauten), (5, 5));
}

#[test]
fn frei_allein_ist_keine_netzfreigabe() {
    let p = Politik {
        netz: false,
        ..Politik::neu()
    };
    assert!(!p.geschuetzt());
    ohne_verbrauch(&eintraege(), &p);
}

#[test]
fn lokal_braucht_keine_netzfreigabe_aber_beachtet_widerruf() {
    let mut e = eintraege();
    e[0].setzen("abfluss", Wert::from("lokal"));
    let p = Politik {
        netz: false,
        ..Politik::neu()
    };
    let a = mit_richtlinie(
        &e,
        &mut Antwortrunde::neu(),
        &mut Dns::default(),
        &mut Netz::default(),
        &p,
    );
    assert_eq!(a.ergebnis, "ok");
    protokoll(&a);
    ohne_verbrauch(
        &e,
        &Politik {
            widerruf_ab: 1,
            ..Politik::neu()
        },
    );
}

#[test]
fn zweiter_widerruf_verhindert_lauf_und_netz() {
    for lokal in [false, true] {
        let mut e = eintraege();
        if lokal {
            e[0].setzen("abfluss", Wert::from("lokal"));
        }
        let p = Politik {
            widerruf_ab: 2,
            ..Politik::neu()
        };
        ohne_verbrauch(&e, &p);
        assert_eq!(p.pruefungen.get(), 2);
    }
}

fn ergebnis() -> Aufrufergebnis {
    Aufrufergebnis {
        ergebnis: "http_fehler",
        daten: Some(Wert::from("INHALTS_SENTINEL")),
        meldung: Some("MELDUNGS_SENTINEL".into()),
        protokollzeile: "bereits protokolliert".into(),
    }
}

#[test]
fn cloud_filter_entfernt_inhalte_und_erhaelt_status_und_protokoll() {
    for (geschuetzt, cloud) in [(true, true), (false, false), (true, false)] {
        let p = Politik {
            geschuetzt,
            cloud,
            ..Politik::neu()
        };
        for sprache in ["de", "en", "en-US"] {
            let a = fuer_modell(ergebnis(), true, "test.api", &p, sprache);
            assert!(a.daten.is_none());
            assert_eq!(a.ergebnis, "http_fehler");
            assert_eq!(a.protokollzeile, "bereits protokolliert");
            let erwartet = if sprache == "de" {
                "Das Ergebnis darf nicht an ein Cloud-Modell gegeben werden, es wird nicht genannt."
            } else {
                "The result must not be given to a cloud model and will not be stated."
            };
            assert_eq!(a.meldung.as_deref(), Some(erwartet));
            assert!(!a.meldung.as_deref().unwrap().contains("SENTINEL"));
        }
        let a = fuer_modell(ergebnis(), false, "test.api", &p, "de");
        assert_eq!(a.daten.unwrap().text(), Some("INHALTS_SENTINEL"));
        assert_eq!(a.meldung.as_deref(), Some("MELDUNGS_SENTINEL"));
    }
    let a = fuer_modell(ergebnis(), true, "test.api", &Politik::neu(), "de");
    assert_eq!(a.daten.unwrap().text(), Some("INHALTS_SENTINEL"));
    assert_eq!(a.meldung.as_deref(), Some("MELDUNGS_SENTINEL"));
}

#[test]
fn definitionen_werden_nur_mit_cloud_freigabe_weitergereicht() {
    let definitionen = vec![super::super::werkzeug::werkzeug(&eintraege()[0]).unwrap()];
    for (geschuetzt, cloud) in [(true, true), (false, false), (true, false), (false, true)] {
        let p = Politik {
            geschuetzt,
            cloud,
            ..Politik::neu()
        };
        assert_eq!(
            werkzeugdefinitionen_fuer_modell(&definitionen, false, &p),
            definitionen
        );
        let gefiltert = werkzeugdefinitionen_fuer_modell(&definitionen, true, &p);
        if geschuetzt || !cloud {
            assert!(gefiltert.is_empty());
        } else {
            assert_eq!(gefiltert, definitionen);
        }
    }
}

#[test]
fn fehlende_konfiguration_verweigert_auch_lokal_und_cloud() {
    assert!(!Verweigernd.netzabfluss_erlaubt("test.api"));
    assert!(Verweigernd.widerrufen("test.api"));
    assert!(!Verweigernd.cloud_werkzeugdefinitionen_erlaubt());
    assert!(!Verweigernd.cloud_ergebnis_erlaubt("test.api"));
    assert!(Verweigernd.geschuetzt());
    let mut e = eintraege();
    ohne_verbrauch(&e, &Verweigernd);
    e[0].setzen("abfluss", Wert::from("lokal"));
    ohne_verbrauch(&e, &Verweigernd);
    let a = fuer_modell(ergebnis(), true, "test.api", &Verweigernd, "de");
    assert!(a.daten.is_none());
    assert!(!a.meldung.as_deref().unwrap().contains("SENTINEL"));
    assert!(
        werkzeugdefinitionen_fuer_modell(&["Definition".into()], true, &Verweigernd).is_empty()
    );
}
