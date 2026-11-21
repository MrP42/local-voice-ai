//! Testseitiger Pruefer genau der im gemeinsamen Dokument benutzten Schema-Teilmenge.
use super::super::kodierung::{als_ganzzahl, lies_json, Wert};

pub(super) fn laden() -> Wert {
    lies_json(
        &include_str!("../../../../../shared/assistenten/schnittstellen.schema.json")
            .replace("\r\n", "\n"),
    )
    .unwrap()
}
fn typ(t: &str, w: &Wert) -> bool {
    match t {
        "object" => w.map().is_some(),
        "array" => matches!(w, Wert::Liste(_)),
        "string" => w.text().is_some(),
        "boolean" => w.bool().is_some(),
        "null" => *w == Wert::Null,
        "integer" => als_ganzzahl(w).is_some(),
        "number" => matches!(w, Wert::Zahl(_)),
        _ => panic!("Schema-Typ {t}"),
    }
}
pub(super) fn passt(s: &Wert, w: &Wert, wurzel: &Wert) -> bool {
    if let Some(b) = s.bool() {
        return b;
    }
    if let Some(r) = s.get("$ref") {
        let mut ziel = wurzel;
        for teil in r.s().trim_start_matches("#/").split('/') {
            ziel = &ziel[teil];
        }
        return passt(ziel, w, wurzel);
    }
    if let Some(t) = s.get("type") {
        let ok = if let Some(t) = t.text() {
            typ(t, w)
        } else {
            t.liste().iter().any(|t| typ(t.s(), w))
        };
        if !ok {
            return false;
        }
    }
    if s.get("const").is_some_and(|c| c != w) {
        return false;
    }
    if s.get("enum").is_some_and(|a| !a.liste().contains(w)) {
        return false;
    }
    if let Some(text) = w.text() {
        let n = text.chars().count() as i64;
        if s["minLength"].ganz().is_some_and(|min| n < min)
            || s["maxLength"].ganz().is_some_and(|max| n > max)
        {
            return false;
        }
        if let Some(p) = s.get("pattern") {
            let p = p.s();
            let p = if let Some(p) = p.strip_suffix('$') {
                format!("{p}\\z")
            } else {
                p.to_owned()
            };
            if !regex::Regex::new(&p).expect("Schema-Muster").is_match(text) {
                return false;
            }
        }
    }
    if let Wert::Zahl(token) = w {
        let n = token.parse::<f64>().unwrap_or(f64::INFINITY);
        if s["minimum"].ganz().is_some_and(|min| n < min as f64)
            || s["maximum"].ganz().is_some_and(|max| n > max as f64)
        {
            return false;
        }
    }
    if let Wert::Liste(l) = w {
        let n = l.len() as i64;
        if s["minItems"].ganz().is_some_and(|min| n < min)
            || s["maxItems"].ganz().is_some_and(|max| n > max)
        {
            return false;
        }
        if s["uniqueItems"].bool() == Some(true)
            && l.iter().enumerate().any(|(i, v)| l[..i].contains(v))
        {
            return false;
        }
        if s.get("items")
            .is_some_and(|s| !l.iter().all(|w| passt(s, w, wurzel)))
        {
            return false;
        }
    }
    if let Some(m) = w.map() {
        if !s["required"].liste().iter().all(|k| m.contains_key(k.s())) {
            return false;
        }
        if let Some(props) = s["properties"].map() {
            for (k, s) in props {
                if m.get(k).is_some_and(|w| !passt(s, w, wurzel)) {
                    return false;
                }
            }
        }
    }
    if !s["allOf"].liste().iter().all(|s| passt(s, w, wurzel)) {
        return false;
    }
    if s.get("not").is_some_and(|s| passt(s, w, wurzel)) {
        return false;
    }
    if let Some(bedingung) = s.get("if") {
        let zweig = if passt(bedingung, w, wurzel) {
            "then"
        } else {
            "else"
        };
        if s.get(zweig).is_some_and(|s| !passt(s, w, wurzel)) {
            return false;
        }
    }
    true
}
