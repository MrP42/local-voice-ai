//! Ausschliesslich testseitige, absichtlich falsche Varianten. Kein Schalter im
//! Produktionsweg; ein Nachweis verlangt einen falschen Rueckgabewert, keinen Absturz.
use sha2::{Digest, Sha256};

use super::super::{anfrage, fehler, objekt, rahmung, verzeichnis};
use super::{argumente, auswerten, bytes, eingabe, ergebnis, faelle, kodiere, lies_json, Wert};

fn erkannt(abschnitt: &str, mutant: impl Fn(&Wert) -> Wert) {
    let mut abweichungen = Vec::new();
    for f in faelle(abschnitt) {
        assert_eq!(
            auswerten(abschnitt, f),
            f["erwartet"],
            "unveraenderte Basis: {}",
            f["name"].s()
        );
        if mutant(f) != f["erwartet"] {
            abweichungen.push(f["name"].s());
        }
    }
    assert!(
        !abweichungen.is_empty(),
        "Mutation ueberlebt in {abschnitt}"
    );
}

#[test]
fn verzeichnis_fehlende_standardwerte_werden_erkannt() {
    erkannt("verzeichnis_ok", |f| {
        let mut doc = verzeichnis::pruefe_verzeichnis(&eingabe(f)).unwrap();
        if let Wert::Objekt(m) = &mut doc {
            if let Some(Wert::Liste(l)) = m.get_mut("eintraege") {
                for e in l {
                    if let Wert::Objekt(e) = e {
                        e.remove("zeitlimit_ms");
                    }
                }
            }
        }
        objekt([("kodiert", kodiere(&doc).unwrap().into())])
    });
}
#[test]
fn adresse_still_normalisierte_standardports_werden_erkannt() {
    erkannt("adresse", |f| {
        let text = f["text"].s();
        let text = if text.starts_with("https://") {
            text.replace(":443/", "/")
        } else if text.starts_with("http://") {
            text.replace(":80/", "/")
        } else {
            text.to_owned()
        };
        Wert::from(
            if super::super::adressen::pruefe_adresse(&text, f["art"].s(), f["netz"].s()) {
                "ok"
            } else {
                "adresse"
            },
        )
    });
}
#[test]
fn ip_uebergangene_ipv4_mapped_klasse_wird_erkannt() {
    erkannt("ip_klasse", |f| {
        if f["ip"].s().starts_with("::ffff:") {
            objekt([("klasse", "nie".into()), ("bereich", "reserviert".into())])
        } else {
            auswerten("ip_klasse", f)
        }
    });
}
#[test]
fn netz_nur_erste_adresse_pruefen_wird_erkannt() {
    erkannt("netz", |f| {
        let erste = f["adressen"]
            .liste()
            .first()
            .map(|s| s.s().to_owned())
            .into_iter()
            .collect::<Vec<_>>();
        super::super::adressen::pruefe_netz(f["netz"].s(), &erste).into()
    });
}

fn anfrage_ohne_pfadsperre(f: &Wert) -> Wert {
    let e = &f["eintrag"];
    if e["art"].s() != "http" {
        return auswerten("anfrage", f);
    }
    let mut ohne = e.clone();
    if let Wert::Objekt(root) = &mut ohne {
        if let Some(Wert::Objekt(http)) = root.get_mut("http") {
            if let Some(Wert::Liste(params)) = http.get_mut("parameter") {
                for p in params {
                    if p["ort"].s() == "pfad" {
                        p.setzen("ort", "abfrage");
                    }
                }
            }
        }
    }
    let erg = anfrage::pruefe_argumente(&ohne, &argumente(f)).and_then(|werte| {
        let adresse = anfrage::baue_adresse(e, &werte)?;
        anfrage::aus_geprueft(e, &werte, adresse, f.get("geheimniswert"))
    });
    ergebnis(erg)
}
#[test]
fn anfrage_fehlende_pfadsperre_wird_erkannt() {
    erkannt("anfrage", anfrage_ohne_pfadsperre);
}

fn utf16(w: &Wert) -> String {
    match w {
        Wert::Objekt(m) => {
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort_by_key(|k| k.encode_utf16().collect::<Vec<_>>());
            format!(
                "{{{}}}",
                keys.iter()
                    .map(|k| format!(
                        "{}:{}",
                        kodiere(&Wert::from(k.as_str())).unwrap(),
                        utf16(&m[*k])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Wert::Liste(l) => format!("[{}]", l.iter().map(utf16).collect::<Vec<_>>().join(",")),
        _ => kodiere(w).unwrap(),
    }
}
#[test]
fn kodierung_sortierung_nach_utf16_wird_erkannt() {
    erkannt("kanonisch", |f| {
        let w = if let Some(roh) = f.get("eingabe_roh") {
            lies_json(roh.s())
        } else {
            Ok(f["eingabe"].clone())
        };
        match w {
            Ok(w) => utf16(&w).into(),
            Err(c) => objekt([("fehler", c.into())]),
        }
    });
}
#[test]
fn bindung_ohne_laengenpraefix_wird_erkannt() {
    erkannt("bindung", |f| {
        if !f.hat("teile") {
            return auswerten("bindung", f);
        }
        let mut h = Sha256::new();
        for t in f["teile"].liste() {
            h.update(t.s().as_bytes());
        }
        Wert::from(
            h.finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        )
    });
}

/// Zweiter, nur im Test vorhandener Automat fuer die gezielte TTL-Mutation.
fn folge_mit_ttl(f: &Wert, ttl: i64) -> Wert {
    let mut frage: Option<(String, i64, bool)> = None;
    let mut out = Vec::new();
    for ev in f["folge"].liste() {
        let t = ev["t"].ganz().unwrap();
        let gueltig = |erstellt| (0..ttl).contains(&(t - erstellt));
        let erg = match ev["ereignis"].s() {
            "frage" => {
                frage = Some((ev["bindung"].s().into(), t, false));
                "offen"
            }
            "ja" => match &mut frage {
                Some((_, erstellt, bestaetigt)) if !*bestaetigt => {
                    if gueltig(*erstellt) {
                        *bestaetigt = true;
                        "bestaetigt"
                    } else {
                        frage = None;
                        "abgelaufen"
                    }
                }
                _ => "keine_frage",
            },
            "nein" => {
                if frage.take().is_some() {
                    "abgelehnt"
                } else {
                    "keine_frage"
                }
            }
            "einloesen" => {
                if frage.as_ref().is_some_and(|(_, _, b)| *b) {
                    let (bindung, erstellt, _) = frage.take().unwrap();
                    if bindung != ev["bindung"].s() {
                        "bindung_falsch"
                    } else if !gueltig(erstellt) {
                        "abgelaufen"
                    } else {
                        "ok"
                    }
                } else {
                    "keine_frage"
                }
            }
            _ => panic!("Ereignis"),
        };
        out.push(Wert::from(erg));
    }
    Wert::Liste(out)
}
#[test]
fn bestaetigung_frist_30001_wird_erkannt() {
    for f in faelle("bestaetigung") {
        assert_eq!(
            folge_mit_ttl(f, 30_000),
            f["erwartet"],
            "Kontrollautomat: {}",
            f["name"].s()
        );
    }
    erkannt("bestaetigung", |f| folge_mit_ttl(f, 30_001));
}
#[test]
fn rahmung_ausgeschaltete_markenkollision_wird_erkannt() {
    erkannt("rahmung", |f| {
        // NUL wird aus Daten entfernt, kann also nie kollidieren. Nach dem Rahmen
        // wird nur in den erzeugten Rahmenzeilen die echte Marke eingesetzt.
        match rahmung::rahme(
            f["id"].s(),
            &bytes(f),
            f["limit"].ganz().unwrap() as usize,
            "\0",
        ) {
            Ok(mut w) => {
                let text = w["text"].s().replace('\0', f["marke"].s());
                w.setzen("text", text);
                w
            }
            Err(c) => objekt([("fehler", c.into())]),
        }
    });
}
#[test]
fn werkzeugname_alte_kollidierende_kodierung_wird_erkannt() {
    erkannt("werkzeugname", |f| {
        if f.hat("id") {
            f["id"].s().replace('.', "_").into()
        } else {
            auswerten("werkzeugname", f)
        }
    });
}
#[test]
fn protokoll_zusatzfelder_werden_erkannt() {
    erkannt("protokoll", |f| {
        if rahmung::protokoll(&f["felder"]).is_err() {
            auswerten("protokoll", f)
        } else {
            objekt([("zeile", kodiere(&f["felder"]).unwrap().into())])
        }
    });
}
#[test]
fn meldung_reflektierter_unbekannter_schluessel_wird_erkannt() {
    erkannt("meldung", |f| {
        if f["ergebnis"].text() == Some("abgelehnt")
            && f["untercode"].text() == Some("argument_unbekannt")
        {
            let parameter = f["parameter"].text();
            rahmung::meldung(
                "abgelehnt",
                f["sprache"].s(),
                Some("argument_fehlt"),
                parameter,
            )
            .map(Wert::from)
            .unwrap_or(Wert::Null)
        } else {
            auswerten("meldung", f)
        }
    });
}
#[test]
fn strenger_leser_duplikate_surrogate_und_tiefe_sind_keine_teilerfolge() {
    for roh in [
        r#"{"a":1,"\u0061":2}"#,
        r#"{"x":"\ud800"}"#,
        r#"{"\udc00":0}"#,
    ] {
        assert!(lies_json(roh).is_err());
    }
    for tiefe in [65, 600, 5000] {
        let roh = format!("{}0{}", "[".repeat(tiefe), "]".repeat(tiefe));
        assert!(lies_json(&roh).is_err(), "Tiefe {tiefe}");
    }
    assert!(lies_json(&format!("{}0{}", "[".repeat(64), "]".repeat(64))).is_ok());
    assert_eq!(
        anfrage::pruefe_argumente(&faelle("anfrage")[0]["eintrag"], &Wert::Null),
        Err(fehler("argumente_typ", None))
    );
}
