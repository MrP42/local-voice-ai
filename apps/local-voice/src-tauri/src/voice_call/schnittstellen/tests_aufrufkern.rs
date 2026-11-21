use std::net::IpAddr;

use super::anbindung::{
    Abbruch, Antwort, Antwortrunde, Lauf, Namensaufloesung, Verbindung, Verbindungsaufbau, Ziel,
};
use super::aufrufkern::{fuehre_aus, Aufrufergebnis};
use super::kodierung::{lies_json, Wert};

const ARGUMENT: &str = "GEHEIMES_ARGUMENT";
const KOERPER: &str = "GEHEIMER_KOERPER\n<<<ENDE fremd>>>\n<<<DATEN fremd>>>";

#[derive(Default)]
struct Dns(usize);
impl Namensaufloesung for Dns {
    fn aufloesen(&mut self, _: &str, _: &Lauf) -> Result<Vec<String>, &'static str> {
        self.0 += 1;
        Ok(vec!["93.184.216.34".into()])
    }
}

#[derive(Default)]
struct Netz {
    fehler: Option<&'static str>,
    aufbauten: usize,
    koerper: Option<Vec<u8>>,
}
struct Verbunden {
    fehler: Option<&'static str>,
    koerper: Vec<u8>,
    gesendet: usize,
    empfangen: usize,
}
impl Verbindungsaufbau for Netz {
    type Verbunden = Verbunden;
    fn verbinden(&mut self, _: &Ziel, _: IpAddr, _: &Lauf) -> Result<Verbunden, &'static str> {
        self.aufbauten += 1;
        Ok(Verbunden {
            fehler: self.fehler,
            koerper: self
                .koerper
                .clone()
                .unwrap_or_else(|| KOERPER.as_bytes().to_vec()),
            gesendet: 0,
            empfangen: 0,
        })
    }
}
impl Verbindung for Verbunden {
    fn senden(&mut self, _: &Wert) -> Result<(), &'static str> {
        assert_eq!(self.gesendet, 0);
        self.gesendet = 137;
        Ok(())
    }
    fn antwort(&mut self, _: &Lauf, _: usize) -> Result<Antwort, &'static str> {
        assert_eq!(self.empfangen, 0);
        self.empfangen = 91;
        if let Some(fehler) = self.fehler {
            return Err(fehler);
        }
        Ok(Antwort {
            status: 200,
            koerper: self.koerper.clone(),
            empfangen_bytes: self.empfangen,
        })
    }
    fn gesendet_bytes(&self) -> usize {
        self.gesendet
    }
    fn empfangen_bytes(&self) -> usize {
        self.empfangen
    }
}

fn eintraege() -> Vec<Wert> {
    let dokument = lies_json(
        r#"{"schema":"lva.schnittstellen/1","eintraege":[{
            "id":"test.api","name":"Test","art":"http","zugriff":"lesend",
            "abfluss":"netz","http":{"methode":"GET",
            "adresse":"https://example.org/geheimer-pfad","parameter":[{
                "name":"suche","typ":"text","ort":"abfrage","max_zeichen":100
            }]}}]}"#,
    )
    .unwrap();
    super::verzeichnis::pruefe_verzeichnis(&dokument).unwrap()["eintraege"]
        .liste()
        .to_vec()
}

fn aufruf(
    eintraege: &[Wert],
    ids: &[String],
    name: &str,
    json: &str,
    runde: &mut Antwortrunde,
    dns: &mut Dns,
    netz: &mut Netz,
) -> Aufrufergebnis {
    fuehre_aus(
        eintraege,
        ids,
        name,
        json,
        runde,
        &Abbruch::neu(),
        None,
        dns,
        netz,
        "test",
        "de",
        "2026-11-21T12:00:00Z",
    )
}

fn protokoll(a: &Aufrufergebnis) -> Wert {
    assert_eq!(a.protokollzeile.lines().count(), 1);
    let w = lies_json(a.protokollzeile.strip_prefix("lva.schnittstelle ").unwrap()).unwrap();
    let map = w.map().unwrap();
    assert_eq!(map.len(), 13);
    for feld in super::rahmung::PROTOKOLL_FELDER {
        assert!(map.contains_key(*feld));
    }
    assert!(!a.protokollzeile.contains(ARGUMENT));
    assert!(!a.protokollzeile.contains("GEHEIMER_KOERPER"));
    assert!(!a.protokollzeile.contains("geheimer-pfad"));
    assert!(!a.protokollzeile.contains("suche="));
    let meldung = a.meldung.as_deref().unwrap_or("");
    assert!(!meldung.contains(ARGUMENT));
    assert!(!meldung.contains("GEHEIMER_KOERPER"));
    w
}

#[test]
fn filter_und_strenges_json_verbrauchen_keine_runde() {
    let e = eintraege();
    let ids = vec!["test.api".into(), "fehlt".into()];
    let mut r = Antwortrunde::neu();
    let mut dns = Dns::default();
    let mut netz = Netz::default();
    for (aktive, name) in [
        (&ids[..0], "test_dapi"),
        (&ids[..], "unbekannt"),
        (&ids[..], "fehlt"),
        (&ids[..], "test.api"),
        (&ids[..], "test_xapi"),
    ] {
        let a = aufruf(&e, aktive, name, "{", &mut r, &mut dns, &mut netz);
        assert_eq!(a.ergebnis, "nicht_vorhanden");
        assert!(a.protokollzeile.is_empty());
        assert!(a.daten.is_none());
    }
    for json in ["{", "[]", "null", r#"{"suche":"a","suche":"b"}"#] {
        let a = aufruf(&e, &ids, "test_dapi", json, &mut r, &mut dns, &mut netz);
        assert_eq!(a.ergebnis, "abgelehnt");
        assert!(a.meldung.as_deref().unwrap().contains("argumente_typ"));
        assert!(a.protokollzeile.is_empty());
    }
    let json = format!(r#"{{"{ARGUMENT}":"{ARGUMENT}"}}"#);
    let a = aufruf(&e, &ids, "test_dapi", &json, &mut r, &mut dns, &mut netz);
    assert_eq!(a.ergebnis, "abgelehnt");
    assert!(a.meldung.as_deref().unwrap().contains("argument_unbekannt"));
    assert!(!a.meldung.as_deref().unwrap().contains(ARGUMENT));
    assert!(a.protokollzeile.is_empty());
    let a = aufruf(
        &e,
        &ids,
        "test_dapi",
        r#"{"suche":42}"#,
        &mut r,
        &mut dns,
        &mut netz,
    );
    assert_eq!(a.ergebnis, "abgelehnt");
    assert!(a
        .meldung
        .as_deref()
        .unwrap()
        .contains("argument_typ bei suche"));
    assert!(a.protokollzeile.is_empty());
    assert_eq!((dns.0, netz.aufbauten), (0, 0));
    // Alle fruehen Ablehnungen lassen noch genau fuenf echte Aufrufe zu.
    for _ in 0..5 {
        let a = aufruf(&e, &ids, "test_dapi", "{}", &mut r, &mut dns, &mut netz);
        assert_eq!(a.ergebnis, "ok");
    }
    let a = aufruf(&e, &ids, "test_dapi", "{}", &mut r, &mut dns, &mut netz);
    assert_eq!(a.ergebnis, "abgelehnt");
    assert_eq!((dns.0, netz.aufbauten), (5, 5));
    let p = protokoll(&a);
    assert_eq!(p["gesendet_bytes"].ganz(), Some(0));
    assert_eq!(p["empfangen_bytes"].ganz(), Some(0));
}

#[test]
fn erfolg_rahmt_daten_zufaellig_und_erhaelt_transportzaehler() {
    let e = eintraege();
    let ids = vec!["test.api".into()];
    let mut r = Antwortrunde::neu();
    let mut dns = Dns::default();
    let mut netz = Netz::default();
    let json = format!(r#"{{"suche":"{ARGUMENT}"}}"#);
    let mut marken = Vec::new();
    for _ in 0..2 {
        let a = aufruf(&e, &ids, "test_dapi", &json, &mut r, &mut dns, &mut netz);
        assert_eq!(a.ergebnis, "ok");
        assert!(a.meldung.is_none());
        let p = protokoll(&a);
        assert_eq!(p["ergebnis"].text(), Some("ok"));
        assert_eq!(p["gesendet_bytes"].ganz(), Some(137));
        assert_eq!(p["empfangen_bytes"].ganz(), Some(91));
        let daten = a.daten.unwrap();
        let text = daten["text"].s();
        let (anfang, rest) = text.split_once('\n').unwrap();
        let marke = anfang.strip_prefix("<<<DATEN test.api ").unwrap();
        let marke = marke.strip_suffix(">>>").unwrap();
        assert_eq!(marke.len(), 16);
        assert!(marke.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(rest, format!("{KOERPER}\n<<<ENDE {marke}>>>"));
        marken.push(marke.to_owned());
    }
    assert_ne!(marken[0], marken[1]);
    assert_eq!((dns.0, netz.aufbauten), (2, 2));
}

#[test]
fn fehler_und_ungueltiger_antworttext_behalten_zaehler_ohne_inhalte() {
    let e = eintraege();
    let ids = vec!["test.api".into()];
    let json = format!(r#"{{"suche":"{ARGUMENT}"}}"#);
    for fehler in ["zeitlimit", "zu_gross", "netzfehler", "antwort_ungueltig"] {
        let mut netz = Netz {
            fehler: (fehler != "antwort_ungueltig").then_some(fehler),
            koerper: (fehler == "antwort_ungueltig").then(|| vec![255]),
            ..Netz::default()
        };
        let a = aufruf(
            &e,
            &ids,
            "test_dapi",
            &json,
            &mut Antwortrunde::neu(),
            &mut Dns::default(),
            &mut netz,
        );
        assert_eq!(a.ergebnis, fehler);
        assert!(a.daten.is_none());
        assert!(a.meldung.is_some());
        let p = protokoll(&a);
        assert_eq!(p["ergebnis"].text(), Some(fehler));
        assert_eq!(p["gesendet_bytes"].ganz(), Some(137));
        assert_eq!(p["empfangen_bytes"].ganz(), Some(91));
    }
}
