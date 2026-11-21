use std::cell::Cell;

use super::super::richtlinie::{
    fuer_modell, werkzeugdefinitionen_fuer_modell, Richtlinie, Verweigernd,
};
use super::{
    aufruf, eintraege, fuehre_aus, lies_json, protokoll, Abbruch, Antwort, Antwortrunde,
    Aufrufergebnis, Dns, Freigabe, IpAddr, Lauf, Namensaufloesung, Netz, Schnappschuss, Verbindung,
    Verbindungsaufbau, Wert, Ziel,
};

struct Politik {
    netz: bool,
    geschuetzt: bool,
    cloud: bool,
    widerruf_ab: usize,
    pruefungen: Cell<usize>,
    widerrufen: Cell<bool>,
}
impl Politik {
    fn neu() -> Self {
        Self {
            netz: true,
            geschuetzt: false,
            cloud: true,
            widerruf_ab: usize::MAX,
            pruefungen: Cell::new(0),
            widerrufen: Cell::new(false),
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
        self.widerrufen.get() || anzahl >= self.widerruf_ab
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

struct WiderrufsDns<'a> {
    dns: Dns,
    widerrufen: &'a Cell<bool>,
    ausloesen: bool,
}

impl Namensaufloesung for WiderrufsDns<'_> {
    fn aufloesen(&mut self, host: &str, lauf: &Lauf) -> Result<Vec<String>, &'static str> {
        let adressen = self.dns.aufloesen(host, lauf)?;
        if self.ausloesen {
            self.widerrufen.set(true);
        }
        Ok(adressen)
    }
}

struct WiderrufsNetz<'a> {
    netz: Netz,
    widerrufen: &'a Cell<bool>,
    ausloesen: bool,
    sendungen: &'a Cell<usize>,
}

struct GezaehlteVerbindung<'a> {
    verbindung: super::Verbunden,
    sendungen: &'a Cell<usize>,
}

impl<'a> Verbindungsaufbau for WiderrufsNetz<'a> {
    type Verbunden = GezaehlteVerbindung<'a>;

    fn verbinden(
        &mut self,
        ziel: &Ziel,
        ip: IpAddr,
        lauf: &Lauf,
    ) -> Result<Self::Verbunden, &'static str> {
        let verbindung = self.netz.verbinden(ziel, ip, lauf)?;
        if self.ausloesen {
            self.widerrufen.set(true);
        }
        Ok(GezaehlteVerbindung {
            verbindung,
            sendungen: self.sendungen,
        })
    }
}

impl Verbindung for GezaehlteVerbindung<'_> {
    fn senden(&mut self, anfrage: &Wert) -> Result<(), &'static str> {
        self.sendungen.set(self.sendungen.get() + 1);
        self.verbindung.senden(anfrage)
    }

    fn antwort(&mut self, lauf: &Lauf, max_bytes: usize) -> Result<Antwort, &'static str> {
        self.verbindung.antwort(lauf, max_bytes)
    }

    fn gesendet_bytes(&self) -> usize {
        self.verbindung.gesendet_bytes()
    }

    fn empfangen_bytes(&self) -> usize {
        self.verbindung.empfangen_bytes()
    }
}

fn widerruf_waehrend_aufbau(im_dns: bool) {
    for schreibend in [false, true] {
        // Gegenprobe ohne Widerruf: derselbe Adapter muss weiterhin genau einmal senden.
        for widerruf in [true, false] {
            let mut e = eintraege();
            if schreibend {
                e[0].setzen("zugriff", Wert::from("schreibend"));
                let mut http = e[0]["http"].clone();
                http.setzen("methode", Wert::from("POST"));
                e[0].setzen("http", http);
            }
            let json = format!(r#"{{"suche":"{}"}}"#, super::ARGUMENT);
            let s = Schnappschuss::neu(&e[0], &lies_json(&json).unwrap(), None).unwrap();
            let freigabe = Freigabe::mit_uhr(std::sync::Arc::new(super::Testuhr(
                std::sync::atomic::AtomicI64::new(0),
            )));
            let generation = freigabe.frage(s.bindung());
            let aeusserung = freigabe.aeusserungsbeginn();
            assert_eq!(freigabe.aeusserung(aeusserung, true), "bestaetigt");
            let p = Politik::neu();
            let sendungen = Cell::new(0);
            let mut dns = WiderrufsDns {
                dns: Dns::default(),
                widerrufen: &p.widerrufen,
                ausloesen: widerruf && im_dns,
            };
            let mut netz = WiderrufsNetz {
                netz: Netz::default(),
                widerrufen: &p.widerrufen,
                ausloesen: widerruf && !im_dns,
                sendungen: &sendungen,
            };
            let a = fuehre_aus(
                &e,
                &["test.api".into()],
                "test_dapi",
                &json,
                &mut Antwortrunde::neu(),
                &Abbruch::neu(),
                schreibend.then_some((&freigabe, generation)),
                &p,
                &mut dns,
                &mut netz,
                "test",
                "de",
                "2026-11-21T12:00:00Z",
            );
            let erwartet = if widerruf { "abgebrochen" } else { "ok" };
            assert_eq!(a.ergebnis, erwartet);
            assert_eq!(sendungen.get(), if widerruf { 0 } else { 1 });
            assert_eq!((dns.dns.0, netz.netz.aufbauten), (1, 1));
            assert_eq!(a.daten.is_none(), widerruf);
            let log = protokoll(&a);
            assert_eq!(log["ergebnis"].text(), Some(erwartet));
            assert_eq!(log["bestaetigt"].bool(), Some(schreibend && !widerruf));
            assert_eq!(
                log["gesendet_bytes"].ganz(),
                Some(if widerruf { 0 } else { 137 })
            );
            assert_eq!(
                log["empfangen_bytes"].ganz(),
                Some(if widerruf { 0 } else { 91 })
            );
            if schreibend {
                assert!(freigabe.bereit(generation, s.bindung()).is_err());
            }
        }
    }
}

#[test]
fn widerruf_im_dns_verhindert_lesenden_und_schreibenden_versand() {
    widerruf_waehrend_aufbau(true);
}

#[test]
fn widerruf_beim_verbinden_verhindert_lesenden_und_schreibenden_versand() {
    widerruf_waehrend_aufbau(false);
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
