use super::adressen::{pfadwert_ok, prozent_kodiere};
use super::kodierung::{als_ganzzahl, kanonisch, Wert};
use super::verzeichnis::ascii_text;
use super::{fehler, objekt};

pub const UNTERCODES: &[&str] = &[
    "argumente_typ",
    "argument_unbekannt",
    "argument_fehlt",
    "argument_typ",
    "argument_laenge",
    "argument_wert",
    "anfrage_zu_lang",
    "anfrage_zu_gross",
    "geheimnis_fehlt",
    "geheimnis_ungueltig",
];

pub fn pruefe_argumente(e: &Wert, a: &Wert) -> Result<Wert, Wert> {
    let Some(map) = a.map() else {
        return Err(fehler("argumente_typ", None));
    };
    let web = e["art"].s() == "web";
    let params = e["http"]["parameter"].liste();
    for k in map.keys() {
        let bekannt = if web {
            k == "seite"
        } else {
            params.iter().any(|p| p["name"].s() == k)
        };
        if !bekannt {
            return Err(fehler("argument_unbekannt", Some(k)));
        }
    }
    if web {
        let v = a
            .get("seite")
            .ok_or_else(|| fehler("argument_fehlt", Some("seite")))?;
        let s = v
            .text()
            .ok_or_else(|| fehler("argument_typ", Some("seite")))?;
        if !e["web"]["seiten"]
            .liste()
            .iter()
            .any(|p| p["kennung"].s() == s)
        {
            return Err(fehler("argument_wert", Some("seite")));
        }
        return Ok(objekt([("seite", v.clone())]));
    }
    let mut werte = objekt(Vec::<(String, Wert)>::new());
    for p in params {
        let n = p["name"].s();
        let err = |code| fehler(code, Some(n));
        let Some(v) = a.get(n) else {
            if p["pflicht"].bool() == Some(true) {
                return Err(err("argument_fehlt"));
            }
            continue;
        };
        let wert = match p["typ"].s() {
            "text" => {
                let s = v.text().ok_or_else(|| err("argument_typ"))?;
                if s.chars().count() as i64 > p["max_zeichen"].ganz().expect("gueltiger Parameter")
                {
                    return Err(err("argument_laenge"));
                }
                if s.chars().any(|c| {
                    (c <= '\u{1f}' || c == '\u{7f}')
                        && !(p["ort"].s() == "koerper" && (c == '\n' || c == '\t'))
                }) {
                    return Err(err("argument_wert"));
                }
                v.clone()
            }
            "ganzzahl" => {
                let z = als_ganzzahl(v).ok_or_else(|| err("argument_typ"))?;
                if p["min"].ganz().is_some_and(|min| z < min)
                    || p["max"].ganz().is_some_and(|max| z > max)
                {
                    return Err(err("argument_wert"));
                }
                Wert::from(z)
            }
            "wahrheitswert" => Wert::Bool(v.bool().ok_or_else(|| err("argument_typ"))?),
            "auswahl" => {
                v.text().ok_or_else(|| err("argument_typ"))?;
                if !p["optionen"].liste().contains(v) {
                    return Err(err("argument_wert"));
                }
                v.clone()
            }
            _ => return Err(err("argument_typ")),
        };
        if p["ort"].s() == "pfad" && wert.text().is_some_and(|s| !pfadwert_ok(s)) {
            return Err(err("argument_wert"));
        }
        werte.setzen(n, wert);
    }
    Ok(werte)
}
fn wert_text(v: &Wert) -> String {
    if let Some(s) = v.text() {
        s.to_owned()
    } else {
        kanonisch(v).expect("normalisierter Skalar")
    }
}
pub fn baue_adresse(e: &Wert, werte: &Wert) -> Result<String, Wert> {
    if e["art"].s() == "web" {
        return Ok(e["web"]["seiten"]
            .liste()
            .iter()
            .find(|s| s["kennung"] == werte["seite"])
            .expect("gepruefte Seitenkennung")["adresse"]
            .s()
            .to_owned());
    }
    let h = &e["http"];
    let mut adresse = h["adresse"].s().to_owned();
    let mut query = Vec::new();
    for p in h["parameter"].liste() {
        let name = p["name"].s();
        let Some(v) = werte.get(name) else {
            continue;
        };
        match p["ort"].s() {
            "pfad" => adresse = adresse.replace(&format!("{{{name}}}"), &wert_text(v)),
            "abfrage" => query.push(format!("{name}={}", prozent_kodiere(&wert_text(v)))),
            _ => {}
        }
    }
    if !query.is_empty() {
        adresse.push('?');
        adresse.push_str(&query.join("&"));
    }
    if adresse.chars().count() > 2048 {
        return Err(fehler("anfrage_zu_lang", None));
    }
    Ok(adresse)
}
fn kopf(name: &str, wert: &str) -> Wert {
    Wert::Liste(vec![name.into(), wert.into()])
}

pub fn baue_anfrage(e: &Wert, a: &Wert, geheimniswert: Option<&Wert>) -> Result<Wert, Wert> {
    let werte = pruefe_argumente(e, a)?;
    let adresse = baue_adresse(e, &werte)?;
    aus_geprueft(e, &werte, adresse, geheimniswert)
}

pub(super) fn aus_geprueft(
    e: &Wert,
    werte: &Wert,
    adresse: String,
    geheimniswert: Option<&Wert>,
) -> Result<Wert, Wert> {
    if e["art"].s() == "web" {
        return Ok(objekt([
            ("methode", "GET".into()),
            ("adresse", adresse.into()),
            (
                "kopfzeilen",
                Wert::Liste(vec![kopf("Accept", "text/html, text/plain;q=0.9")]),
            ),
            ("koerper", Wert::Null),
        ]));
    }
    let h = &e["http"];
    let koerper_params: Vec<_> = h["parameter"]
        .liste()
        .iter()
        .filter(|p| p["ort"].s() == "koerper")
        .collect();
    let koerper = if koerper_params.is_empty() {
        Wert::Null
    } else {
        let obj = objekt(koerper_params.iter().filter_map(|p| {
            let name = p["name"].s();
            werte.get(name).map(|w| (name, w.clone()))
        }));
        let text = kanonisch(&obj).expect("normalisierte Argumente");
        if text.len() > 262144 {
            return Err(fehler("anfrage_zu_gross", None));
        }
        text.into()
    };
    let mut koepfe = vec![kopf("Accept", "application/json")];
    if koerper != Wert::Null {
        koepfe.push(kopf("Content-Type", "application/json"));
    }
    for k in h["kopfzeilen"].liste() {
        koepfe.push(kopf(k["name"].s(), k["wert"].s()));
    }
    if h["geheimnis"] != Wert::Null {
        let geheimnis = geheimniswert
            .filter(|g| **g != Wert::Null)
            .ok_or_else(|| fehler("geheimnis_fehlt", None))?;
        let s = geheimnis
            .text()
            .filter(|s| (1..=4096).contains(&s.len()) && ascii_text(s))
            .ok_or_else(|| fehler("geheimnis_ungueltig", None))?;
        let g = &h["geheimnis"];
        koepfe.push(kopf(g["kopf"].s(), &format!("{}{s}", g["praefix"].s())));
    }
    Ok(objekt([
        ("methode", h["methode"].clone()),
        ("adresse", adresse.into()),
        ("kopfzeilen", Wert::Liste(koepfe)),
        ("koerper", koerper),
    ]))
}
