//! Eingangsvertrag v1 (Spec §4.1): Anfragen anderer Apps an das Notizbuch.
//!
//! Local Voice schreibt eine Anfrage als JSON-Datei in `wai/eingang/notizbuch/` (`wai-eingang`); der
//! Notizbuch-Host liest sie, prüft sie mit [`pruefen`] und übernimmt sie. Maßgeblich ist allein diese
//! Prüfung im Host; der Absender nutzt dieselbe Funktion nur, um früh abzulehnen. Freigabe und Ziel
//! (Abschnitt schreibfreigegeben, existiert, aktiv) prüft der Host gegen seine Ablage.
//!
//! Grundsätze: nur Anlegen und Anhängen, kein Überschreiben, kein Löschen; nur Klartext; Größe
//! begrenzt; unbekannte Felder werden abgelehnt (strenges Schema).

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const MAX_BLOECKE: usize = 200;
pub const MAX_ZEICHEN: usize = 64_000;
pub const MAX_TITEL: usize = 200;
pub const MAX_QUELLEN: usize = 20;
/// Größe einer Anfragedatei (wie `wai_eingang::MAX_DATEI`).
pub const MAX_BYTES: usize = 64 * 1024;
/// Höchstalter der Bestätigung für `abschnitt_anlegen` in Sekunden (Spec §4.2).
pub const MAX_BESTAETIGUNG_S: i64 = 600;

/// Anfrage wie im Eingang abgelegt (strenges Schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anfrage {
    pub v: u32,
    pub app: String,
    /// ULID; Idempotenz über (app, id).
    pub id: String,
    pub aktion: AktionArt,
    pub ziel: Ziel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub titel: Option<String>,
    #[serde(default)]
    pub bloecke: Vec<NeuerBlock>,
    pub herkunft: Herkunft,
    /// Nur bei `abschnitt_anlegen`: Bestätigung der Karte im Assistenten (Spec §4.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bestaetigung: Option<Bestaetigung>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AktionArt {
    SeiteAnlegen,
    BloeckeAnhaengen,
    AbschnittAnlegen,
    Abbrechen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
///
/// Das Format prüft nur Form und Alter. Der empfangende Host MUSS jede `karte_id` höchstens einmal
/// verbrauchen (Wiederholungsschutz), sonst entstehen mehrere „Assistent (n)“ aus einer Karte.
pub struct Bestaetigung {
    pub karte_id: String,
    /// Unix-Sekunden der Bestätigung.
    pub zeit: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ziel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abschnitt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seite_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notizbuch_id: Option<String>,
    /// Bei `abbrechen`: Id der zurückzuziehenden Anfrage desselben Absenders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anfrage_id: Option<String>,
}

/// Geprüfte Aktion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aktion {
    SeiteAnlegen {
        abschnitt_id: String,
        titel: String,
    },
    BloeckeAnhaengen {
        seite_id: String,
    },
    /// Legt den Abschnitt samt Schreibfreigabe atomar an (Sonderregel §4.1). `name` ist „Assistent“
    /// oder „Assistent (n)“.
    AbschnittAnlegen {
        notizbuch_id: String,
        name: String,
        bestaetigung: Bestaetigung,
    },
    Abbrechen {
        anfrage_id: String,
    },
}

/// Geprüfte, bereinigte Anfrage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gepruefte {
    pub app: String,
    pub id: String,
    pub aktion: Aktion,
    pub bloecke: Vec<NeuerBlock>,
    pub herkunft: Herkunft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeueArt {
    Absatz,
    Ueberschrift,
    Aufzaehlung,
    Aufgabe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeuerBlock {
    pub art: NeueArt,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kategorie: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub erledigt: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Herkunft {
    /// Derzeit nur `assistent`.
    pub art: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unterhaltung: Option<String>,
    /// Verweise (`wai://…`), etwa auf die Besprechung.
    #[serde(default)]
    pub quellen: Vec<String>,
}

/// Ergebnis, das der Host als Statusdatei ablegt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    /// `ziel_id` ist die Seite bzw. bei `abschnitt_anlegen` der neue Abschnitt.
    Uebernommen {
        id: String,
        ziel_id: String,
    },
    Abgelehnt {
        id: String,
        code: String,
    },
    /// Anfrage `id` wurde vor der Übernahme zurückgezogen.
    Abgebrochen {
        id: String,
    },
}

/// Ablehnungsgründe (Codes für Oberfläche und Status; ohne Inhalte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Abweisung {
    #[error("eingang.format")]
    Format,
    #[error("eingang.version")]
    Version,
    #[error("eingang.kennung")]
    Kennung,
    #[error("eingang.leer")]
    Leer,
    #[error("eingang.zu_gross")]
    ZuGross,
    #[error("eingang.kategorie")]
    Kategorie,
    #[error("eingang.herkunft")]
    Herkunft,
    #[error("eingang.bestaetigung")]
    Bestaetigung,
}

impl Abweisung {
    pub fn code(self) -> &'static str {
        match self {
            Abweisung::Format => "eingang.format",
            Abweisung::Version => "eingang.version",
            Abweisung::Kennung => "eingang.kennung",
            Abweisung::Leer => "eingang.leer",
            Abweisung::ZuGross => "eingang.zu_gross",
            Abweisung::Kategorie => "eingang.kategorie",
            Abweisung::Herkunft => "eingang.herkunft",
            Abweisung::Bestaetigung => "eingang.bestaetigung",
        }
    }
}

fn kennung_ok(s: &str) -> bool {
    crate::gueltige_id(s)
}

fn app_ok(s: &str) -> bool {
    (1..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}

fn kategorie_ok(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Klartext: Steuer- und Bidi-Zeichen weg (Zeilenumbruch bleibt), Ränder getrimmt.
pub fn klartext(s: &str) -> String {
    s.chars()
        .filter(|c| !crate::bloecke::unsichtbar(*c) && *c != '\t')
        .collect::<String>()
        .trim()
        .to_string()
}

/// Gültiger Abschnittsname für `abschnitt_anlegen`: „Assistent“ oder „Assistent (n)“, n = 2…99.
pub fn assistent_name_ok(name: &str) -> bool {
    name == "Assistent"
        || name
            .strip_prefix("Assistent (")
            .and_then(|r| r.strip_suffix(')'))
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (2..=99).contains(&n) && !name.contains("(0"))
}

/// Liest und prüft eine Anfrage. Gibt sie mit bereinigtem Text zurück. `jetzt` (Unix-Sekunden)
/// dient nur der Altersprüfung der Bestätigung bei `abschnitt_anlegen`.
pub fn pruefen(json: &[u8], jetzt: i64) -> Result<Gepruefte, Abweisung> {
    if json.len() > MAX_BYTES {
        return Err(Abweisung::ZuGross);
    }
    let mut a: Anfrage = serde_json::from_slice(json).map_err(|_| Abweisung::Format)?;
    if a.v != VERSION {
        return Err(Abweisung::Version);
    }
    if !app_ok(&a.app) || !kennung_ok(&a.id) {
        return Err(Abweisung::Kennung);
    }
    let z = std::mem::take(&mut a.ziel);
    let nur_bloecke = |a: &Anfrage| a.bloecke.is_empty();
    let aktion = match (
        a.aktion,
        z.abschnitt_id,
        z.seite_id,
        a.titel.take(),
        z.notizbuch_id,
        z.anfrage_id,
    ) {
        (AktionArt::SeiteAnlegen, Some(abschnitt_id), None, Some(titel), None, None) => {
            if !kennung_ok(&abschnitt_id) {
                return Err(Abweisung::Kennung);
            }
            let titel = klartext(&titel.replace('\n', " "));
            if titel.chars().count() > MAX_TITEL {
                return Err(Abweisung::ZuGross);
            }
            if titel.is_empty() {
                return Err(Abweisung::Leer);
            }
            Aktion::SeiteAnlegen {
                abschnitt_id,
                titel,
            }
        }
        (AktionArt::BloeckeAnhaengen, None, Some(seite_id), None, None, None) => {
            if !kennung_ok(&seite_id) {
                return Err(Abweisung::Kennung);
            }
            Aktion::BloeckeAnhaengen { seite_id }
        }
        (AktionArt::AbschnittAnlegen, None, None, Some(name), Some(notizbuch_id), None) => {
            if !kennung_ok(&notizbuch_id) {
                return Err(Abweisung::Kennung);
            }
            if !nur_bloecke(&a) {
                return Err(Abweisung::Format);
            }
            let name = klartext(&name);
            if !assistent_name_ok(&name) {
                return Err(Abweisung::Format);
            }
            let b = a.bestaetigung.take().ok_or(Abweisung::Bestaetigung)?;
            let alter = jetzt.checked_sub(b.zeit).ok_or(Abweisung::Bestaetigung)?;
            if !kennung_ok(&b.karte_id) || !(0..=MAX_BESTAETIGUNG_S).contains(&alter) {
                return Err(Abweisung::Bestaetigung);
            }
            Aktion::AbschnittAnlegen {
                notizbuch_id,
                name,
                bestaetigung: b,
            }
        }
        (AktionArt::Abbrechen, None, None, None, None, Some(anfrage_id)) => {
            if !kennung_ok(&anfrage_id) || !nur_bloecke(&a) {
                return Err(Abweisung::Kennung);
            }
            Aktion::Abbrechen { anfrage_id }
        }
        _ => return Err(Abweisung::Format),
    };
    if a.bestaetigung.is_some() {
        // Eine Bestätigung gehört nur zu `abschnitt_anlegen`; anderswo ist sie ein Formfehler.
        return Err(Abweisung::Format);
    }
    if a.herkunft.art != "assistent" || a.herkunft.quellen.len() > MAX_QUELLEN {
        return Err(Abweisung::Herkunft);
    }
    if a.herkunft
        .unterhaltung
        .as_deref()
        .is_some_and(|u| !kennung_ok(u))
        || a.herkunft.quellen.iter().any(|q| {
            !q.starts_with("wai://") || q.len() > 512 || !q.bytes().all(|b| b.is_ascii_graphic())
        })
    {
        return Err(Abweisung::Herkunft);
    }
    if a.bloecke.len() > MAX_BLOECKE {
        return Err(Abweisung::ZuGross);
    }
    let mut summe = 0;
    for b in &mut a.bloecke {
        b.text = klartext(&b.text);
        summe += b.text.chars().count();
        if b.kategorie.as_deref().is_some_and(|k| !kategorie_ok(k)) {
            return Err(Abweisung::Kategorie);
        }
        if b.art != NeueArt::Aufgabe {
            b.erledigt = None;
        }
    }
    if summe > MAX_ZEICHEN {
        return Err(Abweisung::ZuGross);
    }
    a.bloecke.retain(|b| !b.text.is_empty());
    if a.bloecke.is_empty() && matches!(aktion, Aktion::BloeckeAnhaengen { .. }) {
        return Err(Abweisung::Leer);
    }
    Ok(Gepruefte {
        app: a.app,
        id: a.id,
        aktion,
        bloecke: a.bloecke,
        herkunft: a.herkunft,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn anfrage() -> serde_json::Value {
        json!({"v": 1, "app": "local-voice", "id": "01JC0000000000000000000000",
            "aktion": "seite_anlegen", "ziel": {"abschnitt_id": "abs-1"}, "titel": "Rückruf\u{202E} Frau Beispiel",
            "bloecke": [{"art": "aufgabe", "text": " Förderantrag klären ", "kategorie": "aufgabe", "erledigt": false},
                        {"art": "absatz", "text": "x", "erledigt": true}],
            "herkunft": {"art": "assistent", "unterhaltung": "u1", "quellen": ["wai://local-voice/besprechung/b1"]}})
    }

    const JETZT: i64 = 1_762_640_000;

    fn mit(f: impl FnOnce(&mut serde_json::Value)) -> Result<Gepruefte, Abweisung> {
        let mut v = anfrage();
        f(&mut v);
        pruefen(&serde_json::to_vec(&v).unwrap(), JETZT)
    }

    #[test]
    fn gueltig_und_bereinigt() {
        let a = mit(|_| {}).unwrap();
        assert_eq!(
            a.aktion,
            Aktion::SeiteAnlegen {
                abschnitt_id: "abs-1".into(),
                titel: "Rückruf Frau Beispiel".into()
            }
        );
        assert_eq!(a.bloecke[0].text, "Förderantrag klären");
        assert_eq!(a.bloecke[1].erledigt, None);
        let roh: Anfrage = serde_json::from_value(anfrage()).unwrap();
        let rund: Anfrage = serde_json::from_str(&serde_json::to_string(&roh).unwrap()).unwrap();
        assert_eq!(rund, roh);
    }

    #[test]
    fn abschnitt_anlegen_und_abbrechen() {
        let basis = |name: &str, zeit: i64| {
            mit(|v| {
                v["aktion"] = json!("abschnitt_anlegen");
                v["ziel"] = json!({"notizbuch_id": "nb1"});
                v["titel"] = json!(name);
                v["bloecke"] = json!([]);
                v["bestaetigung"] = json!({"karte_id": "k1", "zeit": zeit});
            })
        };
        assert!(matches!(
            basis("Assistent", JETZT - 5),
            Ok(Gepruefte {
                aktion: Aktion::AbschnittAnlegen { .. },
                ..
            })
        ));
        assert!(basis("Assistent (2)", JETZT).is_ok());
        assert_eq!(basis("Privat", JETZT), Err(Abweisung::Format));
        assert_eq!(basis("Assistent (02)", JETZT), Err(Abweisung::Format));
        assert_eq!(basis("Assistent (+2)", JETZT), Err(Abweisung::Format));
        assert_eq!(basis("Assistent", i64::MIN), Err(Abweisung::Bestaetigung));
        assert_eq!(
            basis("Assistent", JETZT - MAX_BESTAETIGUNG_S - 1),
            Err(Abweisung::Bestaetigung)
        );
        assert_eq!(basis("Assistent", JETZT + 60), Err(Abweisung::Bestaetigung));
        let ohne = mit(|v| {
            v["aktion"] = json!("abschnitt_anlegen");
            v["ziel"] = json!({"notizbuch_id": "nb1"});
            v["titel"] = json!("Assistent");
            v["bloecke"] = json!([]);
        });
        assert_eq!(ohne, Err(Abweisung::Bestaetigung));
        // Bestätigung bei anderer Aktion ist ein Formfehler.
        assert_eq!(
            mit(|v| v["bestaetigung"] = json!({"karte_id": "k1", "zeit": JETZT})),
            Err(Abweisung::Format)
        );
        let ab = mit(|v| {
            v["aktion"] = json!("abbrechen");
            v["ziel"] = json!({"anfrage_id": "01JC0000000000000000000001"});
            v.as_object_mut().unwrap().remove("titel");
            v["bloecke"] = json!([]);
        })
        .unwrap();
        assert_eq!(
            ab.aktion,
            Aktion::Abbrechen {
                anfrage_id: "01JC0000000000000000000001".into()
            }
        );
        assert_eq!(
            pruefen(&vec![b' '; MAX_BYTES + 1], JETZT),
            Err(Abweisung::ZuGross)
        );
    }

    #[test]
    fn unsichtbare_zeichen_entfernt() {
        let g = mit(|v| {
            v["titel"] = json!("Pro\u{200B}to\u{E0041}koll\u{2028}");
            v["bloecke"][0]["text"] = json!("a\u{FEFF}b\u{2060}c\u{202E}d");
        })
        .unwrap();
        assert!(matches!(&g.aktion, Aktion::SeiteAnlegen { titel, .. } if titel == "Protokoll"));
        assert_eq!(g.bloecke[0].text, "abcd");
    }

    #[test]
    fn ablehnungen() {
        assert_eq!(pruefen(b"{", JETZT), Err(Abweisung::Format));
        assert_eq!(mit(|v| v["v"] = json!(2)), Err(Abweisung::Version));
        assert_eq!(mit(|v| v["fremd"] = json!(1)), Err(Abweisung::Format));
        assert_eq!(
            mit(|v| v["aktion"] = json!("seite_loeschen")),
            Err(Abweisung::Format)
        );
        assert_eq!(
            mit(|v| v["ziel"]["abschnitt_id"] = json!("../x")),
            Err(Abweisung::Kennung)
        );
        assert_eq!(mit(|v| v["titel"] = json!("  ")), Err(Abweisung::Leer));
        assert_eq!(
            mit(|v| v["ziel"]["seite_id"] = json!("s1")),
            Err(Abweisung::Format)
        );
        assert_eq!(
            mit(|v| v["herkunft"]["art"] = json!("mail")),
            Err(Abweisung::Herkunft)
        );
        assert_eq!(
            mit(|v| v["herkunft"]["quellen"] = json!(["https://x"])),
            Err(Abweisung::Herkunft)
        );
        assert_eq!(
            mit(|v| v["herkunft"]["quellen"] = json!(["wai://x y\nz"])),
            Err(Abweisung::Herkunft)
        );
        assert_eq!(
            mit(|v| v["bloecke"][0]["kategorie"] = json!("<b>")),
            Err(Abweisung::Kategorie)
        );
        assert_eq!(
            mit(|v| v["bloecke"][0]["text"] = json!("a".repeat(MAX_ZEICHEN + 1))),
            Err(Abweisung::ZuGross)
        );
        assert_eq!(
            mit(|v| {
                v["aktion"] = json!("bloecke_anhaengen");
                v["ziel"] = json!({"seite_id": "s1"});
                v.as_object_mut().unwrap().remove("titel");
                v["bloecke"] = json!([]);
            }),
            Err(Abweisung::Leer)
        );
    }
}
