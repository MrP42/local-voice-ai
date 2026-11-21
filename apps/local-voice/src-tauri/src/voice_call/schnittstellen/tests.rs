use std::collections::BTreeSet;

use once_cell::sync::Lazy;

use super::adressen;
use super::anfrage;
use super::bestaetigung::Bestaetigung;
use super::bindung;
use super::kodierung::{kanonisch as kodiere, lies_json, Wert};
use super::rahmung;
use super::verzeichnis;
use super::werkzeug;
use super::{ergebnis, objekt};

#[path = "tests_anbindung.rs"]
mod anbindungsproben;
#[path = "tests_mutationen.rs"]
mod mutationen;
#[path = "tests_schema.rs"]
mod schema;

static DATEN: Lazy<Wert> = Lazy::new(|| {
    let text = include_str!("../../../../../shared/assistenten/schnittstellen-pruefstaende.json")
        .replace("\r\n", "\n");
    super::kodierung::lies_pruefstaende(&text)
        .expect("Pruefstandsdatei: strenges JSON, keine Duplikate/Surrogate")
});
const ABSCHNITTE: &[(&str, usize)] = &[
    ("verzeichnis_ok", 37),
    ("verzeichnis_fehler", 278),
    ("adresse", 159),
    ("ip_klasse", 132),
    ("netz", 40),
    ("anfrage", 248),
    ("kanonisch", 27),
    ("bindung", 35),
    ("bestaetigung", 90),
    ("rahmung", 71),
    ("schnittstellenblock", 11),
    ("meldung", 57),
    ("protokoll", 11),
    ("werkzeug", 18),
    ("werkzeugname", 60),
];
fn faelle(abschnitt: &str) -> &'static [Wert] {
    DATEN[abschnitt].liste()
}
fn eingabe(f: &Wert) -> Wert {
    match &f["eingabe"] {
        Wert::Text(s) => lies_json(s).unwrap_or(Wert::Null),
        w => w.clone(),
    }
}
fn argumente(f: &Wert) -> Wert {
    let mut a = if let Some(roh) = f.get("argumente_roh") {
        lies_json(roh.s()).unwrap_or(Wert::Null)
    } else {
        f["argumente"].clone()
    };
    if let Some(fuellen) = f.get("fuellen") {
        a.setzen(
            fuellen["parameter"].s(),
            fuellen["text"]
                .s()
                .repeat(fuellen["anzahl"].ganz().unwrap() as usize),
        );
    }
    a
}
fn bytes(f: &Wert) -> Vec<u8> {
    if let Some(hex) = f.get("hex") {
        let hex: String = hex
            .s()
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        hex.as_bytes()
            .chunks(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect()
    } else {
        f["text"].s().as_bytes().to_vec()
    }
}
fn folge(f: &Wert) -> Wert {
    let mut b = Bestaetigung::default();
    Wert::Liste(
        f["folge"]
            .liste()
            .iter()
            .map(|ev| {
                let t = ev["t"].ganz().unwrap();
                Wert::from(match ev["ereignis"].s() {
                    "frage" => b.frage(ev["bindung"].s(), t),
                    "ja" => b.ja(t),
                    "nein" => b.nein(t),
                    "einloesen" => b.einloesen(ev["bindung"].s(), t),
                    _ => panic!("unbekanntes Ereignis"),
                })
            })
            .collect(),
    )
}
fn auswerten(abschnitt: &str, f: &Wert) -> Wert {
    match abschnitt {
        "verzeichnis_ok" | "verzeichnis_fehler" => {
            match verzeichnis::kodiertes_verzeichnis(&eingabe(f)) {
                Ok(s) => objekt([("kodiert", s.into())]),
                Err(e) => objekt([("fehler", e)]),
            }
        }
        "adresse" => Wert::from(
            if adressen::pruefe_adresse(f["text"].s(), f["art"].s(), f["netz"].s()) {
                "ok"
            } else {
                "adresse"
            },
        ),
        "ip_klasse" => {
            let (k, b) = adressen::klasse(f["ip"].s()).expect("IP-Pruefstand");
            objekt([("klasse", k.into()), ("bereich", b.into())])
        }
        "netz" => adressen::pruefe_netz(
            f["netz"].s(),
            &f["adressen"]
                .liste()
                .iter()
                .map(|a| a.s().to_owned())
                .collect::<Vec<_>>(),
        )
        .into(),
        "anfrage" => ergebnis(anfrage::baue_anfrage(
            &f["eintrag"],
            &argumente(f),
            f.get("geheimniswert"),
        )),
        "kanonisch" => {
            let w = if let Some(roh) = f.get("eingabe_roh") {
                lies_json(roh.s())
            } else {
                Ok(f["eingabe"].clone())
            };
            match w.and_then(|w| kodiere(&w)) {
                Ok(s) => s.into(),
                Err(c) => objekt([("fehler", c.into())]),
            }
        }
        "bindung" => {
            if f.hat("teile") {
                let t = f["teile"].liste();
                bindung::bindung(t[0].s(), t[1].s(), t[2].s(), t[3].s()).into()
            } else {
                ergebnis(bindung::fuer_aufruf(&f["eintrag"], &argumente(f)))
            }
        }
        "bestaetigung" => folge(f),
        "rahmung" => match rahmung::rahme(
            f["id"].s(),
            &bytes(f),
            f["limit"].ganz().unwrap() as usize,
            f["marke"].s(),
        ) {
            Ok(w) => w,
            Err(c) => objekt([("fehler", c.into())]),
        },
        "schnittstellenblock" => rahmung::schnittstellenblock(f["sprache"].s()).into(),
        "meldung" => rahmung::meldung(
            f["ergebnis"].s(),
            f["sprache"].s(),
            f["untercode"].text(),
            f["parameter"].text(),
        )
        .map(Wert::from)
        .unwrap_or(Wert::Null),
        "protokoll" => match rahmung::protokoll(&f["felder"]) {
            Ok(s) => objekt([("zeile", s.into())]),
            Err(c) => objekt([("fehler", c.into())]),
        },
        "werkzeug" => werkzeug::werkzeug(&f["eintrag"])
            .expect("Werkzeugdefinition")
            .into(),
        "werkzeugname" => {
            if f.hat("id") {
                werkzeug::werkzeugname(f["id"].s()).into()
            } else if f.hat("ids") {
                let ids = f["ids"]
                    .liste()
                    .iter()
                    .map(|v| v.s().to_owned())
                    .collect::<Vec<_>>();
                match werkzeug::aufloesen(&ids, f["werkzeugname"].s()) {
                    Some(id) => objekt([("id", id.into())]),
                    None => "nicht_vorhanden".into(),
                }
            } else {
                werkzeug::kennung(f["werkzeugname"].s())
                    .map(Wert::from)
                    .unwrap_or(Wert::Null)
            }
        }
        _ => panic!("unbekannter Abschnitt {abschnitt}"),
    }
}

fn pruefe(abschnitt: &str) {
    let liste = faelle(abschnitt);
    let anzahl = ABSCHNITTE.iter().find(|(a, _)| *a == abschnitt).unwrap().1;
    assert_eq!(liste.len(), anzahl, "Fallzahl {abschnitt}");
    let mut namen = BTreeSet::new();
    let mut abweichungen = Vec::new();
    for f in liste {
        let name = f["name"].s();
        assert!(namen.insert(name), "Fallname doppelt [{abschnitt}] {name}");
        // Jeder Fall wird auch nach einer Abweichung weiter ausgewertet.
        match std::panic::catch_unwind(|| auswerten(abschnitt, f)) {
            Ok(ist) if ist == f["erwartet"] => {}
            Ok(ist) => abweichungen.push(format!(
                "[{abschnitt}] {name}: {ist:?} statt {:?}",
                f["erwartet"]
            )),
            Err(_) => abweichungen.push(format!("[{abschnitt}] {name}: Abbruch")),
        }
    }
    assert!(abweichungen.is_empty(), "{}", abweichungen.join("\n"));
}

#[test]
fn pruefstandsbestand_1274() {
    assert_eq!(ABSCHNITTE.iter().map(|(_, n)| n).sum::<usize>(), 1274);
    let vorhanden: BTreeSet<_> = DATEN
        .map()
        .unwrap()
        .keys()
        .filter(|k| k.as_str() != "hinweis")
        .map(String::as_str)
        .collect();
    assert_eq!(vorhanden, ABSCHNITTE.iter().map(|(a, _)| *a).collect());
    assert_eq!(
        faelle("bestaetigung")
            .iter()
            .filter(|f| f["name"].s().starts_with("Matrix:"))
            .count(),
        60
    );
}
macro_rules! abschnitttest {
    ($($name:ident),+ $(,)?) => { $(#[test] fn $name() { pruefe(stringify!($name)); })+ };
}
abschnitttest!(
    verzeichnis_ok,
    verzeichnis_fehler,
    adresse,
    ip_klasse,
    netz,
    anfrage,
    kanonisch,
    bindung,
    bestaetigung,
    rahmung,
    schnittstellenblock,
    meldung,
    protokoll,
    werkzeug,
    werkzeugname
);

#[test]
fn verzeichnis_idempotent_und_schema() {
    let schema = schema::laden();
    for f in faelle("verzeichnis_ok") {
        let name = f["name"].s();
        let w = eingabe(f);
        assert!(
            schema::passt(&schema, &w, &schema),
            "Schema Eingabe: {name}"
        );
        let text = verzeichnis::kodiertes_verzeichnis(&w).unwrap();
        let gelesen = lies_json(&text).unwrap();
        assert_eq!(
            verzeichnis::kodiertes_verzeichnis(&gelesen)
                .unwrap()
                .as_bytes(),
            text.as_bytes(),
            "zweite Kodierung: {name}"
        );
        assert!(
            schema::passt(&schema, &gelesen, &schema),
            "Schema kodiert: {name}"
        );
    }
    for f in faelle("verzeichnis_fehler")
        .iter()
        .filter(|f| f["schema_lehnt_ab"].bool() == Some(true))
    {
        assert!(
            !schema::passt(&schema, &eingabe(f), &schema),
            "Schema muss ablehnen: {}",
            f["name"].s()
        );
    }
}

#[test]
fn bindung_bytegleich_zu_approvals() {
    for f in faelle("bindung") {
        let ist = auswerten("bindung", f);
        assert_eq!(ist, f["erwartet"], "{}", f["name"].s());
        if f.hat("teile") {
            continue;
        }
        if let Some(ok) = ist.get("ok") {
            let hash = crate::managers::integrations::approvals::args_hash(
                f["eintrag"]["id"].s(),
                ok["faehigkeit"].s(),
                ok["ziel"].s(),
                ok["argumente"].s(),
            );
            assert_eq!(
                hash.as_bytes(),
                ok["bindung"].s().as_bytes(),
                "{}",
                f["name"].s()
            );
        }
    }
}

fn adress_teile(a: &str) -> (&str, &str) {
    let (_, rest) = a.split_once("://").unwrap();
    let slash = a.len() - rest.len() + rest.find('/').unwrap();
    (&a[..slash], a[slash..].split('?').next().unwrap())
}
#[test]
fn anfrageeigenschaften_und_geheimnisfreiheit_der_werkzeuge() {
    for f in faelle("anfrage") {
        let a = argumente(f);
        let e = &f["eintrag"];
        let Ok(gebaut) = anfrage::baue_anfrage(e, &a, f.get("geheimniswert")) else {
            continue;
        };
        let name = f["name"].s();
        let werte = anfrage::pruefe_argumente(e, &a).unwrap();
        let vorlage = if e["art"].s() == "web" {
            let text = kodiere(&a).unwrap();
            assert!(!text.contains("://"), "{name}");
            for s in e["web"]["seiten"].liste() {
                let host = adress_teile(s["adresse"].s())
                    .0
                    .split("://")
                    .nth(1)
                    .unwrap()
                    .split(':')
                    .next()
                    .unwrap();
                assert!(!text.contains(host), "Host in Modellargumenten: {name}");
            }
            e["web"]["seiten"]
                .liste()
                .iter()
                .find(|s| s["kennung"] == a["seite"])
                .unwrap()["adresse"]
                .s()
        } else {
            e["http"]["adresse"].s()
        };
        let (kopf_v, pfad_v) = adress_teile(vorlage);
        let (kopf, pfad) = adress_teile(gebaut["adresse"].s());
        assert_eq!(kopf, kopf_v, "Autoritaet: {name}");
        assert!(!pfad.contains("//"), "{name}");
        assert!(!pfad.split('/').any(|s| s == "." || s == ".."), "{name}");
        let seg_v: Vec<_> = pfad_v.split('/').collect();
        let seg: Vec<_> = pfad.split('/').collect();
        assert_eq!(seg.len(), seg_v.len(), "{name}");
        for (v, s) in seg_v.iter().zip(&seg) {
            if v.starts_with('{') {
                let wert = &werte[&v[1..v.len() - 1]];
                let text = wert
                    .text()
                    .map(str::to_owned)
                    .unwrap_or_else(|| kodiere(wert).unwrap());
                assert_eq!(s.as_bytes(), text.as_bytes(), "Pfadwert: {name}");
            } else {
                assert_eq!(v, s, "festes Segment: {name}");
            }
        }
        assert_eq!(
            pfad.matches('%').count(),
            pfad_v.matches('%').count(),
            "{name}"
        );
    }
    for f in faelle("werkzeug") {
        let text = werkzeug::werkzeug(&f["eintrag"]).unwrap();
        assert!(!text.contains("://"), "{}", f["name"].s());
        let e = &f["eintrag"];
        let adressen: Vec<_> = if e["art"].s() == "web" {
            e["web"]["seiten"]
                .liste()
                .iter()
                .map(|s| s["adresse"].s())
                .collect()
        } else {
            vec![e["http"]["adresse"].s()]
        };
        for a in adressen {
            let host = adress_teile(a)
                .0
                .split("://")
                .nth(1)
                .unwrap()
                .split(':')
                .next()
                .unwrap();
            assert!(!text.contains(host), "Host im Werkzeug: {}", f["name"].s());
        }
    }
    for f in faelle("protokoll") {
        if let Ok(zeile) = rahmung::protokoll(&f["felder"]) {
            for v in f["verboten"].liste() {
                assert!(!zeile.contains(v.s()), "{}", f["name"].s());
            }
        }
    }
}

#[test]
fn transportgrenze_status_und_zufallsmarken() {
    assert_eq!(rahmung::MAX_TRANSPORT, 1_048_576);
    for s in 100..=599 {
        let erwartet = if (200..300).contains(&s) {
            "ok"
        } else if (300..400).contains(&s) {
            "umleitung"
        } else {
            "http_fehler"
        };
        assert_eq!(rahmung::status_ergebnis(s), erwartet);
    }
    assert_eq!(rahmung::antwort_limit(500, 20000), 2048);
    assert_eq!(rahmung::antwort_limit(400, 1000), 1000);
    assert_eq!(rahmung::antwort_limit(200, 20000), 20000);
    let mut marken = BTreeSet::new();
    for _ in 0..64 {
        let marke = rahmung::neue_marke().unwrap();
        assert_eq!(marke.len(), 16);
        assert!(marke
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert!(marken.insert(marke));
    }
}

#[test]
fn transportgrenze_wird_beim_lesen_geprueft() {
    use std::io::Read;
    struct Zaehler {
        gelesen: usize,
        laenge: usize,
    }
    impl Read for Zaehler {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.laenge - self.gelesen);
            buf[..n].fill(b'x');
            self.gelesen += n;
            Ok(n)
        }
    }
    for laenge in [0, 1, rahmung::MAX_TRANSPORT] {
        let mut q = Zaehler { gelesen: 0, laenge };
        assert_eq!(rahmung::lese_begrenzt(&mut q).unwrap().len(), laenge);
    }
    let mut q = Zaehler {
        gelesen: 0,
        laenge: 100 * rahmung::MAX_TRANSPORT,
    };
    assert_eq!(rahmung::lese_begrenzt(&mut q), Err("zu_gross"));
    assert_eq!(q.gelesen, rahmung::MAX_TRANSPORT + 1);
}
