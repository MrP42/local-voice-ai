use std::net::IpAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::super::anbindung::{
    protokollzeile, versende, Abbruch, Antwort, Antwortrunde, Lauf, Namensaufloesung,
    Schnappschuss, Verbindung, Verbindungsaufbau, Ziel, MAX_AUFRUFE_JE_ANTWORT,
};
use super::super::bestaetigung::{Freigabe, MonotoneUhr, Uhr};
use super::super::kodierung::{lies_json, Wert, MAX_JSON_BYTES};

#[derive(Default)]
struct TestUhr(AtomicI64);
impl Uhr for TestUhr {
    fn jetzt_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
type Aktion = Option<Box<dyn FnOnce()>>;
#[derive(Default)]
struct Dns {
    anzahl: usize,
    aktion: Aktion,
}
impl Namensaufloesung for Dns {
    fn aufloesen(&mut self, host: &str, _lauf: &Lauf) -> Result<Vec<String>, &'static str> {
        assert_eq!(host, "api.example.org");
        self.anzahl += 1;
        if let Some(a) = self.aktion.take() {
            a();
        }
        // Jede unerlaubte zweite Aufloesung wuerde eine gesperrte Adresse liefern.
        Ok(vec![if self.anzahl == 1 {
            "8.8.8.8"
        } else {
            "169.254.169.254"
        }
        .into()])
    }
}
#[derive(Default)]
struct Beobachtung {
    ziele: Vec<(Ziel, IpAddr)>,
    anfragen: Vec<Wert>,
    alt_svc: Vec<String>,
}
#[derive(Default)]
struct Aufbau {
    aufrufe: usize,
    beobachtung: Arc<Mutex<Beobachtung>>,
    tls_aktion: Aktion,
    sende_aktion: Aktion,
    sende_fehler: bool,
    antwort_ergebnis: Option<Result<Antwort, &'static str>>,
}
struct Kanal {
    beobachtung: Arc<Mutex<Beobachtung>>,
    sende_aktion: Aktion,
    sende_fehler: bool,
    antwort_ergebnis: Option<Result<Antwort, &'static str>>,
    gesendet: usize,
}
impl Verbindungsaufbau for Aufbau {
    type Verbunden = Kanal;
    fn verbinden(&mut self, ziel: &Ziel, ip: IpAddr, _lauf: &Lauf) -> Result<Kanal, &'static str> {
        self.aufrufe += 1;
        self.beobachtung
            .lock()
            .unwrap()
            .ziele
            .push((ziel.clone(), ip));
        if let Some(a) = self.tls_aktion.take() {
            a();
        }
        Ok(Kanal {
            beobachtung: self.beobachtung.clone(),
            sende_aktion: self.sende_aktion.take(),
            sende_fehler: self.sende_fehler,
            antwort_ergebnis: self.antwort_ergebnis.take(),
            gesendet: 0,
        })
    }
}
impl Verbindung for Kanal {
    fn senden(&mut self, anfrage: &Wert) -> Result<(), &'static str> {
        if let Some(a) = self.sende_aktion.take() {
            a();
        }
        let mut b = self.beobachtung.lock().unwrap();
        self.gesendet = super::super::kodierung::kanonisch(anfrage).unwrap().len();
        b.anfragen.push(anfrage.clone());
        // Eine Antwort bietet ein alternatives Ziel an. Es bleibt nur Antwortinhalt;
        // weder die Ablaufhuelle noch ein spaeterer Aufruf liest oder speichert eine Route daraus.
        b.alt_svc.push("h3=\"8.8.8.8:8443\"; ma=86400".into());
        if self.sende_fehler {
            Err("Abriss")
        } else {
            Ok(())
        }
    }
    fn antwort(&mut self, lauf: &Lauf, _max_bytes: usize) -> Result<Antwort, &'static str> {
        lauf.pruefen()?;
        self.antwort_ergebnis.take().unwrap_or(Ok(Antwort {
            status: 200,
            koerper: b"{}".to_vec(),
            empfangen_bytes: 2,
        }))
    }
    fn gesendet_bytes(&self) -> usize {
        self.gesendet
    }
}
fn eintrag(schreibend: bool) -> Wert {
    let mut e = lies_json(
        r#"{
        "id":"probe", "name":"Probe", "beschreibung":"", "art":"http",
        "zugriff":"schreibend", "abfluss":"netz", "zeitlimit_ms":15000,
        "http":{"methode":"POST", "adresse":"https://api.example.org/send",
        "netz":"oeffentlich", "antwort_max_bytes":20480,
        "parameter":[{"name":"text", "beschreibung":"", "typ":"text", "ort":"koerper",
        "pflicht":true, "max_zeichen":500}], "kopfzeilen":[], "geheimnis":null}
    }"#,
    )
    .unwrap();
    if !schreibend {
        e.setzen("zugriff", "lesend");
    }
    assert!(
        super::super::verzeichnis::pruefe_verzeichnis(&super::super::objekt([
            ("schema", "lva.schnittstellen/1".into()),
            ("eintraege", Wert::Liste(vec![e.clone()])),
        ]))
        .is_ok()
    );
    e
}
fn snapshot(schreibend: bool) -> Schnappschuss {
    Schnappschuss::neu(
        &eintrag(schreibend),
        &lies_json(r#"{"text":"einmal"}"#).unwrap(),
        None,
    )
    .unwrap()
}
fn bestaetigt(f: &Freigabe, s: &Schnappschuss, uhr: &TestUhr) -> u64 {
    uhr.0.store(0, Ordering::SeqCst);
    let generation = f.frage(s.bindung());
    uhr.0.store(1, Ordering::SeqCst);
    assert_eq!(f.aeusserung(f.aeusserungsbeginn(), true), "bestaetigt");
    generation
}

#[test]
fn nein_nach_ja_waehrend_dns_oder_tls_verhindert_versand() {
    for waehrend_dns in [true, false] {
        let uhr = Arc::new(TestUhr::default());
        let f = Arc::new(Freigabe::mit_uhr(uhr.clone()));
        let s = snapshot(true);
        let generation = bestaetigt(&f, &s, &uhr);
        let mut dns = Dns::default();
        let mut aufbau = Aufbau::default();
        let widerruf = {
            let f = f.clone();
            let uhr = uhr.clone();
            Some(Box::new(move || {
                let neue_aeusserung = f.aeusserungsbeginn();
                uhr.0.store(2, Ordering::SeqCst);
                assert_eq!(f.aeusserung(neue_aeusserung, false), "abgelehnt");
            }) as Box<dyn FnOnce()>)
        };
        if waehrend_dns {
            dns.aktion = widerruf;
        } else {
            aufbau.tls_aktion = widerruf;
        }
        uhr.0.store(0, Ordering::SeqCst);
        assert_eq!(
            versende(
                &s,
                Some((&f, generation)),
                &mut dns,
                &mut aufbau,
                &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
            )
            .ergebnis,
            "bestaetigung_abgelehnt"
        );
        assert!(aufbau.beobachtung.lock().unwrap().anfragen.is_empty());
    }
}
#[test]
fn aeusserung_vor_frage_bestaetigt_nichts() {
    let uhr = Arc::new(TestUhr::default());
    let f = Freigabe::mit_uhr(uhr.clone());
    let a = f.aeusserungsbeginn();
    let s = snapshot(true);
    uhr.0.store(1, Ordering::SeqCst);
    let generation = f.frage(s.bindung());
    uhr.0.store(2, Ordering::SeqCst);
    assert_eq!(f.aeusserung(a, true), "keine_frage");
    let mut dns = Dns::default();
    let mut aufbau = Aufbau::default();
    uhr.0.store(3, Ordering::SeqCst);
    assert_eq!(
        versende(
            &s,
            Some((&f, generation)),
            &mut dns,
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
        )
        .ergebnis,
        "bestaetigung_abgelaufen"
    );
    assert_eq!(dns.anzahl, 0);
    assert!(aufbau.beobachtung.lock().unwrap().anfragen.is_empty());
    uhr.0.store(4, Ordering::SeqCst);
    assert_eq!(f.aeusserung(f.aeusserungsbeginn(), true), "bestaetigt");
}
#[test]
fn aeusserung_zu_a_laesst_ersatzfrage_b_unberuehrt() {
    for ja in [true, false] {
        let uhr = Arc::new(TestUhr::default());
        let f = Freigabe::mit_uhr(uhr.clone());
        uhr.0.store(0, Ordering::SeqCst);
        f.frage("A");
        let a = f.aeusserungsbeginn();
        uhr.0.store(1, Ordering::SeqCst);
        let generation_b = f.frage("B");
        uhr.0.store(2, Ordering::SeqCst);
        assert_eq!(f.aeusserung(a, ja), "keine_frage");
        uhr.0.store(2, Ordering::SeqCst);
        assert_eq!(
            f.absenden(generation_b, "B", || ()),
            Err("bestaetigung_abgelaufen")
        );
        uhr.0.store(3, Ordering::SeqCst);
        assert_eq!(f.aeusserung(f.aeusserungsbeginn(), true), "bestaetigt");
        uhr.0.store(3, Ordering::SeqCst);
        assert_eq!(f.absenden(generation_b, "B", || ()), Ok(()));
    }
}
#[test]
fn alt_svc_aendert_host_und_port_auch_beim_zweiten_aufruf_nicht() {
    let s = snapshot(false);
    let mut aufbau = Aufbau::default();
    for _ in 0..2 {
        let mut dns = Dns::default();
        assert_eq!(
            versende(
                &s,
                None,
                &mut dns,
                &mut aufbau,
                &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
            )
            .ergebnis,
            "ok"
        );
        assert_eq!(dns.anzahl, 1);
    }
    let b = aufbau.beobachtung.lock().unwrap();
    assert_eq!(b.alt_svc.len(), 2);
    assert_eq!(b.ziele.len(), 2);
    for (z, ip) in &b.ziele {
        assert_eq!(
            z,
            &Ziel {
                host: "api.example.org".into(),
                port: 443,
                tls: true
            }
        );
        assert_eq!(*ip, "8.8.8.8".parse::<IpAddr>().unwrap());
    }
}
#[test]
fn genau_einmal_aufloesen_und_an_gepruefte_adresse_binden() {
    let s = snapshot(false);
    let mut dns = Dns::default();
    let mut aufbau = Aufbau::default();
    assert_eq!(
        versende(
            &s,
            None,
            &mut dns,
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
        )
        .ergebnis,
        "ok"
    );
    assert_eq!(dns.anzahl, 1);
    let b = aufbau.beobachtung.lock().unwrap();
    assert_eq!(b.ziele.len(), 1);
    assert_eq!(b.ziele[0].1, "8.8.8.8".parse::<IpAddr>().unwrap());
    assert_eq!(b.anfragen.len(), 1);
}
#[test]
fn fristablauf_waehrend_tls_verhindert_versand() {
    let uhr = Arc::new(TestUhr::default());
    let f = Freigabe::mit_uhr(uhr.clone());
    let s = snapshot(true);
    let generation = bestaetigt(&f, &s, &uhr);
    let u = uhr.clone();
    let mut aufbau = Aufbau {
        tls_aktion: Some(Box::new(move || u.0.store(30_000, Ordering::SeqCst))),
        ..Aufbau::default()
    };
    uhr.0.store(0, Ordering::SeqCst);
    assert_eq!(
        versende(
            &s,
            Some((&f, generation)),
            &mut Dns::default(),
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
        )
        .ergebnis,
        "bestaetigung_abgelaufen"
    );
    assert!(aufbau.beobachtung.lock().unwrap().anfragen.is_empty());
}
#[test]
fn einloesen_und_absenden_sind_unteilbar_und_nicht_wiederholbar() {
    let uhr = Arc::new(TestUhr::default());
    let f = Arc::new(Freigabe::mit_uhr(uhr.clone()));
    let s = snapshot(true);
    let generation = bestaetigt(&f, &s, &uhr);
    let f_im_versand = f.clone();
    let mut aufbau = Aufbau {
        sende_aktion: Some(Box::new(move || {
            assert!(f_im_versand.entscheidung_gesperrt())
        })),
        ..Aufbau::default()
    };
    uhr.0.store(29_999, Ordering::SeqCst);
    assert_eq!(
        versende(
            &s,
            Some((&f, generation)),
            &mut Dns::default(),
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
        )
        .ergebnis,
        "ok"
    );
    assert!(!f.entscheidung_gesperrt());
    uhr.0.store(29_999, Ordering::SeqCst);
    assert_eq!(
        versende(
            &s,
            Some((&f, generation)),
            &mut Dns::default(),
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
        )
        .ergebnis,
        "bestaetigung_abgelaufen"
    );
    assert_eq!(aufbau.beobachtung.lock().unwrap().anfragen.len(), 1);
}
#[test]
fn schnappschuss_bleibt_nach_frage_bytegleich_und_schreibfehler_wird_nie_wiederholt() {
    let uhr = Arc::new(TestUhr::default());
    let f = Freigabe::mit_uhr(uhr.clone());
    let e = eintrag(true);
    let mut argumente = lies_json(r#"{"text":"urspruenglich"}"#).unwrap();
    let s = Schnappschuss::neu(&e, &argumente, None).unwrap();
    let generation = bestaetigt(&f, &s, &uhr);
    argumente.setzen("text", "veraendert");
    let mut aufbau = Aufbau {
        sende_fehler: true,
        ..Aufbau::default()
    };
    uhr.0.store(0, Ordering::SeqCst);
    assert_eq!(
        versende(
            &s,
            Some((&f, generation)),
            &mut Dns::default(),
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
        )
        .ergebnis,
        "unklar"
    );
    let b = aufbau.beobachtung.lock().unwrap();
    assert_eq!(b.anfragen.len(), 1);
    assert_eq!(b.anfragen[0]["koerper"].s(), r#"{"text":"urspruenglich"}"#);
    assert_ne!(
        Schnappschuss::neu(&e, &argumente, None).unwrap().bindung(),
        s.bindung()
    );
}
#[test]
fn alte_sender_duerfen_ersatzfrage_mit_identischer_bindung_nicht_einloesen() {
    let uhr = Arc::new(TestUhr::default());
    let f = Freigabe::mit_uhr(uhr.clone());
    let s = snapshot(true);
    let a = bestaetigt(&f, &s, &uhr);
    let b = bestaetigt(&f, &s, &uhr);
    assert_ne!(a, b);
    uhr.0.store(0, Ordering::SeqCst);
    assert_eq!(
        f.absenden(a, s.bindung(), || ()),
        Err("bestaetigung_abgelaufen")
    );
    uhr.0.store(0, Ordering::SeqCst);
    assert_eq!(f.absenden(b, s.bindung(), || ()), Ok(()));
}

#[test]
fn app_und_mcp_ergeben_typisierten_fehler_statt_http_anfrage() {
    for art in ["app", "mcp"] {
        let e = super::faelle("verzeichnis_ok")
            .iter()
            .flat_map(|f| f["eingabe"]["eintraege"].liste())
            .find(|e| e["art"].text() == Some(art))
            .unwrap();
        for roh in [r#"{}"#, r#"{"x":1}"#] {
            let a = lies_json(roh).unwrap();
            let erwartet = super::super::fehler("art_nicht_unterstuetzt", None);
            assert_eq!(
                Schnappschuss::neu(e, &a, None).err(),
                Some(erwartet.clone())
            );
            assert_eq!(
                super::super::anfrage::baue_anfrage(e, &a, None),
                Err(erwartet.clone())
            );
            assert_eq!(super::super::anfrage::baue_adresse(e, &a), Err(erwartet));
        }
    }
}

struct FesteDns {
    adressen: Vec<String>,
    anzahl: usize,
}
impl Namensaufloesung for FesteDns {
    fn aufloesen(&mut self, host: &str, _lauf: &Lauf) -> Result<Vec<String>, &'static str> {
        assert_eq!(host, "api.example.org");
        self.anzahl += 1;
        Ok(self.adressen.clone())
    }
}

#[test]
fn ssrf_tor_prueft_alle_adressen_und_den_netzmodus_vor_verbindungsaufbau() {
    for (netz, adressen, erwartet) in [
        (
            "oeffentlich",
            vec!["169.254.169.254"],
            Err("adresse_abgewiesen"),
        ),
        (
            "oeffentlich",
            vec!["8.8.8.8", "169.254.169.254"],
            Err("adresse_abgewiesen"),
        ),
        ("oeffentlich", vec![], Err("netzfehler")),
        ("privat", vec!["10.1.2.3"], Ok(())),
        ("privat", vec!["10.1.2.3", "10.4.5.6"], Ok(())),
        ("privat", vec!["8.8.8.8"], Err("adresse_abgewiesen")),
        ("privat", vec!["fc00::1%eth0"], Err("adresse_abgewiesen")),
    ] {
        let mut e = eintrag(false);
        let mut h = e["http"].clone();
        h.setzen("netz", netz);
        e.setzen("http", h);
        let s = Schnappschuss::neu(&e, &lies_json(r#"{"text":"einmal"}"#).unwrap(), None).unwrap();
        let mut dns = FesteDns {
            adressen: adressen.iter().map(|a| (*a).into()).collect(),
            anzahl: 0,
        };
        let mut aufbau = Aufbau::default();
        assert_eq!(
            versende(
                &s,
                None,
                &mut dns,
                &mut aufbau,
                &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
            )
            .ergebnis,
            erwartet.map_or_else(|art| art, |()| "ok")
        );
        assert_eq!(dns.anzahl, 1);
        let b = aufbau.beobachtung.lock().unwrap();
        if erwartet.is_ok() {
            assert_eq!(b.ziele.len(), 1);
            assert_eq!(b.ziele[0].1, "10.1.2.3".parse::<IpAddr>().unwrap());
            assert_eq!(b.anfragen.len(), 1);
        } else {
            assert!(b.ziele.is_empty());
            assert!(b.anfragen.is_empty());
        }
    }
}

#[test]
fn ziel_behaelt_expliziten_port_und_lokales_http_ohne_dns() {
    for (adresse, netz, host, port, tls, dns_anzahl) in [
        (
            "https://api.example.org:8443/send",
            "oeffentlich",
            "api.example.org",
            8443,
            true,
            1,
        ),
        ("http://127.0.0.1/send", "geraet", "127.0.0.1", 80, false, 0),
    ] {
        let mut e = eintrag(false);
        let mut h = e["http"].clone();
        h.setzen("adresse", adresse);
        h.setzen("netz", netz);
        e.setzen("http", h);
        if netz == "geraet" {
            e.setzen("abfluss", "lokal");
        }
        let s = Schnappschuss::neu(&e, &lies_json(r#"{"text":"einmal"}"#).unwrap(), None).unwrap();
        let mut dns = Dns::default();
        let mut aufbau = Aufbau::default();
        assert_eq!(
            versende(
                &s,
                None,
                &mut dns,
                &mut aufbau,
                &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
            )
            .ergebnis,
            "ok"
        );
        assert_eq!(dns.anzahl, dns_anzahl);
        let b = aufbau.beobachtung.lock().unwrap();
        assert_eq!(b.ziele.len(), 1);
        assert_eq!(
            b.ziele[0].0,
            Ziel {
                host: host.into(),
                port,
                tls
            }
        );
        let ip = if tls { "8.8.8.8" } else { "127.0.0.1" };
        assert_eq!(b.ziele[0].1, ip.parse::<IpAddr>().unwrap());
        assert_eq!(b.anfragen.len(), 1);
    }
}

#[test]
fn schnappschuss_bindung_ist_bytegleich_zu_fuer_aufruf() {
    let mut anzahl = 0;
    for f in super::faelle("anfrage")
        .iter()
        .filter(|f| f["erwartet"].hat("ok"))
    {
        let e = &f["eintrag"];
        let a = super::argumente(f);
        let s = Schnappschuss::neu(e, &a, f.get("geheimniswert")).unwrap();
        let b = super::super::bindung::fuer_aufruf(e, &a).unwrap();
        assert_eq!(
            s.bindung().as_bytes(),
            b["bindung"].s().as_bytes(),
            "{}",
            f["name"].s()
        );
        anzahl += 1;
    }
    assert!(anzahl > 0);
}

#[test]
fn nein_vor_versende_verhindert_schon_dns() {
    let uhr = Arc::new(TestUhr::default());
    let f = Freigabe::mit_uhr(uhr.clone());
    let s = snapshot(true);
    let generation = bestaetigt(&f, &s, &uhr);
    uhr.0.store(2, Ordering::SeqCst);
    assert_eq!(f.aeusserung(f.aeusserungsbeginn(), false), "abgelehnt");
    let mut dns = Dns::default();
    let mut aufbau = Aufbau::default();
    uhr.0.store(0, Ordering::SeqCst);
    assert_eq!(
        versende(
            &s,
            Some((&f, generation)),
            &mut dns,
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
        )
        .ergebnis,
        "bestaetigung_abgelehnt"
    );
    assert_eq!(dns.anzahl, 0);
    let b = aufbau.beobachtung.lock().unwrap();
    assert!(b.ziele.is_empty());
    assert!(b.anfragen.is_empty());
}

#[test]
fn freigabe_bleibt_nach_panic_im_versand_verwendbar_ohne_erneutes_einloesen() {
    let uhr = Arc::new(TestUhr::default());
    let f = Freigabe::mit_uhr(uhr.clone());
    let s = snapshot(true);
    let generation = bestaetigt(&f, &s, &uhr);
    uhr.0.store(0, Ordering::SeqCst);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = f.absenden(generation, s.bindung(), || panic!("Versandabbruch"));
    }))
    .is_err());
    uhr.0.store(0, Ordering::SeqCst);
    assert_eq!(
        f.bereit(generation, s.bindung()),
        Err("bestaetigung_abgelaufen")
    );
    uhr.0.store(0, Ordering::SeqCst);
    assert_eq!(
        f.absenden(generation, s.bindung(), || ()),
        Err("bestaetigung_abgelaufen")
    );
    uhr.0.store(1, Ordering::SeqCst);
    assert_eq!(f.aeusserung(f.aeusserungsbeginn(), true), "keine_frage");
    let neu = bestaetigt(&f, &s, &uhr);
    uhr.0.store(0, Ordering::SeqCst);
    assert_eq!(f.bereit(neu, s.bindung()), Ok(()));
    uhr.0.store(0, Ordering::SeqCst);
    assert_eq!(f.absenden(neu, s.bindung(), || ()), Ok(()));
}

#[test]
fn debug_der_gebauten_anfrage_verdeckt_auch_verschachtelte_kopfzeilen() {
    let mut e = eintrag(false);
    let mut h = e["http"].clone();
    h.setzen(
        "geheimnis",
        lies_json(r#"{"kopf":"Authorization","praefix":"Bearer ","schluessel":"probe"}"#).unwrap(),
    );
    e.setzen("http", h);
    let anfrage = super::super::anfrage::baue_anfrage(
        &e,
        &lies_json(r#"{"text":"einmal"}"#).unwrap(),
        Some(&Wert::from("streng-geheim")),
    )
    .unwrap();
    assert_eq!(
        anfrage["kopfzeilen"].liste().last().unwrap().liste()[1].s(),
        "Bearer streng-geheim"
    );
    for w in [anfrage.clone(), super::super::objekt([("ok", anfrage)])] {
        let debug = format!("{w:?}");
        assert!(!debug.contains("streng-geheim"));
        assert!(!debug.contains("Bearer"));
        assert!(debug.contains("[REDACTED]"));
    }
}

#[test]
fn freigabe_nutzt_ihre_eigene_uhr() {
    let uhr = Arc::new(TestUhr::default());
    let f = Freigabe::mit_uhr(uhr.clone());
    uhr.0.store(0, Ordering::SeqCst);
    let generation = f.frage("probe");
    let beginn = f.aeusserungsbeginn();
    uhr.0.store(29_999, Ordering::SeqCst);
    assert_eq!(f.aeusserung(beginn, true), "bestaetigt");
    uhr.0.store(30_000, Ordering::SeqCst);
    assert_eq!(
        f.bereit(generation, "probe"),
        Err("bestaetigung_abgelaufen")
    );
    uhr.0.store(30_000, Ordering::SeqCst);
    let generation = f.frage("probe");
    let beginn = f.aeusserungsbeginn();
    uhr.0.store(59_999, Ordering::SeqCst);
    assert_eq!(f.aeusserung(beginn, true), "bestaetigt");
    uhr.0.store(59_999, Ordering::SeqCst);
    assert_eq!(f.absenden(generation, "probe", || ()), Ok(()));
    uhr.0.store(60_000, Ordering::SeqCst);
    f.frage("probe");
    let beginn = f.aeusserungsbeginn();
    uhr.0.store(90_000, Ordering::SeqCst);
    assert_eq!(f.aeusserung(beginn, true), "abgelaufen");
}

#[test]
fn monotone_uhr_steigt_und_faellt_nie() {
    let uhr = MonotoneUhr::default();
    let erste = uhr.jetzt_ms();
    let zweite = uhr.jetzt_ms();
    assert!(zweite >= erste);
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(uhr.jetzt_ms() > zweite);
}

#[test]
fn lies_json_kappt_die_eingabe_vor_dem_parsen() {
    let genau = format!("\"{}\"", "a".repeat(MAX_JSON_BYTES - 2));
    assert_eq!(genau.len(), MAX_JSON_BYTES);
    assert!(lies_json(&genau).is_ok());
    let zu_gross = format!("\"{}\"", "a".repeat(MAX_JSON_BYTES - 1));
    assert_eq!(zu_gross.len(), MAX_JSON_BYTES + 1);
    assert_eq!(lies_json(&zu_gross), Err("json"));
}

#[test]
fn zeitlimit_vor_dns() {
    // Ohne Fristpruefung vor DNS wuerden Aufloesung oder Aufbau bereits stattfinden.
    for schreibend in [false, true] {
        let s = snapshot(schreibend);
        let uhr = Arc::new(TestUhr::default());
        let f = Freigabe::mit_uhr(uhr.clone());
        let generation = bestaetigt(&f, &s, &uhr);
        let mut dns = Dns::default();
        let mut aufbau = Aufbau::default();
        let ausgang = versende(
            &s,
            schreibend.then_some((&f, generation)),
            &mut dns,
            &mut aufbau,
            &Lauf::fuer_test(Duration::ZERO, Abbruch::neu()),
        );
        assert_eq!(ausgang.ergebnis, "zeitlimit");
        assert_eq!(dns.anzahl, 0);
        assert_eq!(aufbau.aufrufe, 0);
        assert!(aufbau.beobachtung.lock().unwrap().anfragen.is_empty());
        assert_eq!(ausgang.gesendet_bytes, 0);
    }
}

#[test]
fn abbruch_waehrend_dns_verhindert_verbindung() {
    // Ohne Abbruchpruefung nach DNS wuerde der Verbindungszaehler steigen.
    for schreibend in [false, true] {
        let s = snapshot(schreibend);
        let uhr = Arc::new(TestUhr::default());
        let f = Freigabe::mit_uhr(uhr.clone());
        let generation = bestaetigt(&f, &s, &uhr);
        let abbruch = Abbruch::neu();
        let ausloeser = abbruch.clone();
        let mut dns = Dns {
            aktion: Some(Box::new(move || ausloeser.ausloesen())),
            ..Dns::default()
        };
        let mut aufbau = Aufbau::default();
        let ausgang = versende(
            &s,
            schreibend.then_some((&f, generation)),
            &mut dns,
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), abbruch),
        );
        assert_eq!(ausgang.ergebnis, "abgebrochen");
        assert_eq!(dns.anzahl, 1);
        assert_eq!(aufbau.aufrufe, 0);
        assert!(aufbau.beobachtung.lock().unwrap().anfragen.is_empty());
        assert_eq!(ausgang.gesendet_bytes, 0);
    }
}

#[test]
fn abbruch_oder_frist_nach_aufbau_verhindert_senden() {
    // Ohne Pruefung nach TLS wuerde trotz Abbruch oder Fristablauf gesendet.
    for schreibend in [false, true] {
        for fristablauf in [false, true] {
            let s = snapshot(schreibend);
            let uhr = Arc::new(TestUhr::default());
            let f = Freigabe::mit_uhr(uhr.clone());
            let generation = bestaetigt(&f, &s, &uhr);
            let abbruch = Abbruch::neu();
            let ausloeser = abbruch.clone();
            let mut aufbau = Aufbau {
                tls_aktion: Some(Box::new(move || {
                    if fristablauf {
                        std::thread::sleep(Duration::from_millis(120));
                    } else {
                        ausloeser.ausloesen();
                    }
                })),
                ..Aufbau::default()
            };
            let frist = if fristablauf {
                Duration::from_millis(60)
            } else {
                Duration::from_secs(30)
            };
            let ausgang = versende(
                &s,
                schreibend.then_some((&f, generation)),
                &mut Dns::default(),
                &mut aufbau,
                &Lauf::fuer_test(frist, abbruch),
            );
            assert_eq!(
                ausgang.ergebnis,
                if fristablauf {
                    "zeitlimit"
                } else {
                    "abgebrochen"
                }
            );
            assert_eq!(aufbau.aufrufe, 1);
            assert!(aufbau.beobachtung.lock().unwrap().anfragen.is_empty());
            assert_eq!(ausgang.gesendet_bytes, 0);
        }
    }
}

#[test]
fn zu_gross_bleibt_zu_gross_auch_beim_schreiben() {
    // Eine pauschale Abbildung auf unklar oder netzfehler verliert die Groessengrenze.
    for schreibend in [false, true] {
        let s = snapshot(schreibend);
        let uhr = Arc::new(TestUhr::default());
        let f = Freigabe::mit_uhr(uhr.clone());
        let generation = bestaetigt(&f, &s, &uhr);
        let mut aufbau = Aufbau {
            antwort_ergebnis: Some(Err("zu_gross")),
            ..Aufbau::default()
        };
        let ausgang = versende(
            &s,
            schreibend.then_some((&f, generation)),
            &mut Dns::default(),
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
        );
        assert_eq!(ausgang.ergebnis, "zu_gross");
        assert_eq!(aufbau.beobachtung.lock().unwrap().anfragen.len(), 1);
    }
}

#[test]
fn schreibend_nach_dem_absenden_ist_jede_andere_stoerung_unklar() {
    // Ohne zugriffsabhaengige Fehlerabbildung gehen Unsicherheit oder Fehlerart verloren.
    for schreibend in [false, true] {
        for (fehler, lesend) in [
            ("zeitlimit", "zeitlimit"),
            ("abgebrochen", "abgebrochen"),
            ("netzfehler", "netzfehler"),
            ("antwort_ungueltig", "antwort_ungueltig"),
            ("voellig_unbekannt", "netzfehler"),
        ] {
            let s = snapshot(schreibend);
            let uhr = Arc::new(TestUhr::default());
            let f = Freigabe::mit_uhr(uhr.clone());
            let generation = bestaetigt(&f, &s, &uhr);
            let mut aufbau = Aufbau {
                antwort_ergebnis: Some(Err(fehler)),
                ..Aufbau::default()
            };
            let ausgang = versende(
                &s,
                schreibend.then_some((&f, generation)),
                &mut Dns::default(),
                &mut aufbau,
                &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
            );
            assert_eq!(ausgang.ergebnis, if schreibend { "unklar" } else { lesend });
            assert_eq!(aufbau.beobachtung.lock().unwrap().anfragen.len(), 1);
        }
    }
}

#[test]
fn abbruch_nach_dem_absenden_wirkt_nur_lesend() {
    // Ohne Entkopplung des Abbruchs beim Schreiben wuerde die Antwort unklar statt ok.
    for schreibend in [false, true] {
        let s = snapshot(schreibend);
        let uhr = Arc::new(TestUhr::default());
        let f = Freigabe::mit_uhr(uhr.clone());
        let generation = bestaetigt(&f, &s, &uhr);
        let abbruch = Abbruch::neu();
        let ausloeser = abbruch.clone();
        let mut aufbau = Aufbau {
            sende_aktion: Some(Box::new(move || ausloeser.ausloesen())),
            ..Aufbau::default()
        };
        let ausgang = versende(
            &s,
            schreibend.then_some((&f, generation)),
            &mut Dns::default(),
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), abbruch.clone()),
        );
        assert!(abbruch.ausgeloest());
        assert_eq!(
            ausgang.ergebnis,
            if schreibend { "ok" } else { "abgebrochen" }
        );
        assert_eq!(ausgang.status, schreibend.then_some(200));
        assert_eq!(aufbau.beobachtung.lock().unwrap().anfragen.len(), 1);
    }
}

#[test]
fn statusabbildung() {
    // Falsche Statusklassen oder durchgereichte Umleitungskoerper lassen die Gegenprobe scheitern.
    for (status, erwartet) in [
        (200, "ok"),
        (204, "ok"),
        (302, "umleitung"),
        (404, "http_fehler"),
        (500, "http_fehler"),
        (101, "http_fehler"),
    ] {
        let s = snapshot(false);
        let koerper = b"Antwort".to_vec();
        let mut aufbau = Aufbau {
            antwort_ergebnis: Some(Ok(Antwort {
                status,
                koerper: koerper.clone(),
                empfangen_bytes: koerper.len(),
            })),
            ..Aufbau::default()
        };
        let ausgang = versende(
            &s,
            None,
            &mut Dns::default(),
            &mut aufbau,
            &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
        );
        assert_eq!(ausgang.ergebnis, erwartet);
        assert_eq!(ausgang.status, Some(status));
        if status == 302 {
            assert!(ausgang.koerper.is_empty());
        } else {
            assert_eq!(ausgang.koerper, koerper);
        }
    }
}

#[test]
fn aufruflimit_je_antwort() {
    // Ohne Rundengrenze scheitert die Ablehnung; ohne Eintragsfrist scheitern die Restzeiten.
    assert_eq!(MAX_AUFRUFE_JE_ANTWORT, 5);
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut runde = Antwortrunde::neu();
    for _ in 0..MAX_AUFRUFE_JE_ANTWORT {
        let lauf = runde.naechster_aufruf(&s, &abbruch).unwrap();
        let rest = lauf.rest().unwrap();
        assert!(rest <= Duration::from_secs(15));
        assert!(rest > Duration::from_secs(14));
    }
    assert_eq!(
        runde.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    let mut neue_runde = Antwortrunde::neu();
    for _ in 0..MAX_AUFRUFE_JE_ANTWORT {
        assert!(neue_runde.naechster_aufruf(&s, &abbruch).is_ok());
    }
    assert_eq!(
        neue_runde.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    let mut e = eintrag(false);
    e.setzen("zeitlimit_ms", Wert::from(1000_i64));
    let kurz = Schnappschuss::neu(&e, &lies_json(r#"{"text":"einmal"}"#).unwrap(), None).unwrap();
    let lauf = Antwortrunde::neu()
        .naechster_aufruf(&kurz, &abbruch)
        .unwrap();
    assert!(lauf.rest().unwrap() <= Duration::from_secs(1));
}

#[test]
fn ausgang_zaehlt_bytes_und_dauer() {
    // Fehlende Byteuebernahme oder eine absolute statt verstrichener Zeit wird sichtbar.
    let s = snapshot(false);
    let mut aufbau = Aufbau {
        sende_aktion: Some(Box::new(|| std::thread::sleep(Duration::from_millis(20)))),
        antwort_ergebnis: Some(Ok(Antwort {
            status: 200,
            koerper: b"1234567".to_vec(),
            empfangen_bytes: 7,
        })),
        ..Aufbau::default()
    };
    let ausgang = versende(
        &s,
        None,
        &mut Dns::default(),
        &mut aufbau,
        &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
    );
    assert_eq!(ausgang.ergebnis, "ok");
    assert_eq!(ausgang.empfangen_bytes, 7);
    let erwartet = super::super::kodierung::kanonisch(s.anfrage())
        .unwrap()
        .len();
    assert!(erwartet > 0);
    assert_eq!(ausgang.gesendet_bytes, erwartet);
    assert!(ausgang.dauer_ms >= 20);
    assert!(ausgang.dauer_ms < 1000);
    let ausgang = versende(
        &s,
        None,
        &mut Dns::default(),
        &mut Aufbau::default(),
        &Lauf::fuer_test(Duration::ZERO, Abbruch::neu()),
    );
    assert_eq!(ausgang.ergebnis, "zeitlimit");
    assert_eq!(ausgang.gesendet_bytes, 0);
    assert_eq!(ausgang.empfangen_bytes, 0);
}

#[test]
fn protokollzeile_hat_genau_diese_felder_und_keine_inhalte() {
    // Jedes zusaetzliche Feld und jeder uebernommene Anfrageinhalt verletzt das Protokollschema.
    let e = lies_json(
        r#"{
        "id":"probe", "name":"Probe", "beschreibung":"", "art":"http",
        "zugriff":"schreibend", "abfluss":"netz", "zeitlimit_ms":15000,
        "http":{"methode":"POST", "adresse":"https://api.example.org/send/{kennung}",
        "netz":"oeffentlich", "antwort_max_bytes":20480,
        "parameter":[{"name":"text", "beschreibung":"", "typ":"text", "ort":"koerper",
        "pflicht":true, "max_zeichen":500},
        {"name":"kennung", "beschreibung":"", "typ":"text", "ort":"pfad",
        "pflicht":true, "max_zeichen":20}], "kopfzeilen":[],
        "geheimnis":{"kopf":"authorization", "praefix":"Bearer ",
        "schluessel":"probe-schluessel"}}
    }"#,
    )
    .unwrap();
    assert!(
        super::super::verzeichnis::pruefe_verzeichnis(&super::super::objekt([
            ("schema", "lva.schnittstellen/1".into()),
            ("eintraege", Wert::Liste(vec![e.clone()])),
        ]))
        .is_ok()
    );
    let s = Schnappschuss::neu(
        &e,
        &lies_json(r#"{"kennung":"abc123","text":"einmal"}"#).unwrap(),
        Some(&Wert::from("GEHEIM-12345")),
    )
    .unwrap();
    let uhr = Arc::new(TestUhr::default());
    let f = Freigabe::mit_uhr(uhr.clone());
    let generation = bestaetigt(&f, &s, &uhr);
    let ausgang = versende(
        &s,
        Some((&f, generation)),
        &mut Dns::default(),
        &mut Aufbau::default(),
        &Lauf::fuer_test(Duration::from_secs(30), Abbruch::neu()),
    );
    assert_eq!(ausgang.ergebnis, "ok");
    let zeile = protokollzeile(&s, &ausgang);
    let erwartet = super::super::objekt([
        ("kennung", "probe".into()),
        ("art", "http".into()),
        ("zugriff", "schreibend".into()),
        ("ergebnis", "ok".into()),
        (
            "dauer_ms",
            Wert::from(i64::try_from(ausgang.dauer_ms).unwrap()),
        ),
        (
            "gesendet_bytes",
            Wert::from(i64::try_from(ausgang.gesendet_bytes).unwrap()),
        ),
        (
            "empfangen_bytes",
            Wert::from(i64::try_from(ausgang.empfangen_bytes).unwrap()),
        ),
    ]);
    // Objektgleichheit prueft auch, dass keine weiteren Schluessel vorhanden sind.
    assert!(lies_json(&zeile).unwrap() == erwartet);
    for inhalt in [
        "abc123",
        "/send",
        "GEHEIM-12345",
        "Bearer",
        "einmal",
        "api.example.org",
        s.bindung(),
    ] {
        assert!(!zeile.contains(inhalt));
    }
}
