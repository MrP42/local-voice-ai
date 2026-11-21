use super::kodierung::{kanonisch, Wert};
use super::objekt;
use super::verzeichnis::id_ok;

pub fn werkzeugname(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            '_' => "__".into(),
            '.' => "_d".into(),
            c => c.to_string(),
        })
        .collect()
}
pub fn kennung(name: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        match c {
            '.' => return None,
            '_' => out.push(match chars.next()? {
                '_' => '_',
                'd' => '.',
                _ => return None,
            }),
            c => out.push(c),
        }
    }
    id_ok(&out).then_some(out)
}
pub fn aufloesen(ids: &[String], name: &str) -> Option<String> {
    kennung(name).filter(|id| ids.contains(id))
}
pub fn werkzeug(e: &Wert) -> Result<String, &'static str> {
    let mut eigenschaften = objekt(Vec::<(String, Wert)>::new());
    let mut pflicht = Vec::new();
    match e["art"].s() {
        "web" => {
            eigenschaften.setzen(
                "seite",
                objekt([
                    ("type", "string".into()),
                    (
                        "enum",
                        Wert::Liste(
                            e["web"]["seiten"]
                                .liste()
                                .iter()
                                .map(|s| s["kennung"].clone())
                                .collect(),
                        ),
                    ),
                ]),
            );
            pflicht.push("seite".into());
        }
        "http" => {
            for p in e["http"]["parameter"].liste() {
                let mut s = match p["typ"].s() {
                    "text" => objekt([
                        ("type", "string".into()),
                        ("maxLength", p["max_zeichen"].clone()),
                    ]),
                    "ganzzahl" => {
                        let mut s = objekt([("type", "integer".into())]);
                        if p.hat("min") {
                            s.setzen("minimum", p["min"].clone());
                        }
                        if p.hat("max") {
                            s.setzen("maximum", p["max"].clone());
                        }
                        s
                    }
                    "wahrheitswert" => objekt([("type", "boolean".into())]),
                    _ => objekt([("type", "string".into()), ("enum", p["optionen"].clone())]),
                };
                if !p["beschreibung"].s().is_empty() {
                    s.setzen("description", p["beschreibung"].clone());
                }
                eigenschaften.setzen(p["name"].s(), s);
                if p["pflicht"].bool() == Some(true) {
                    pflicht.push(p["name"].clone());
                }
            }
        }
        _ => return Err("art"),
    }
    kanonisch(&objekt([
        ("name", werkzeugname(e["id"].s()).into()),
        (
            "description",
            if e["beschreibung"].s().is_empty() {
                e["name"].clone()
            } else {
                e["beschreibung"].clone()
            },
        ),
        (
            "parameters",
            objekt([
                ("type", "object".into()),
                ("properties", eigenschaften),
                ("required", Wert::Liste(pflicht)),
                ("additionalProperties", false.into()),
            ]),
        ),
    ]))
}
