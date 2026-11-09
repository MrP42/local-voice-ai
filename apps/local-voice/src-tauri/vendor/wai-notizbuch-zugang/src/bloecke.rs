//! Seiteninhalt (Tiptap-JSON des Notizbuchs) → flache Blockliste für den Assistenten.
//!
//! Ein Block ist eine Textzeile mit Struktur: Absatz, Überschrift, Listenpunkt, Aufgabe, Zitat, Code,
//! Tabellenzelle oder ein Platzhalter für Bild, Datei, Zeichnung und Formel. Jeder Block trägt, falls
//! vorhanden, die Kategorie (`kategorie`), den Erledigt-Stand einer Aufgabe und die Blockzeit `zeit`
//! (ISO-Zeit der letzten inhaltlichen Änderung, Spec §5.1). Unbekannte Knoten werden über ihre Kinder
//! gelesen, nie verworfen: Text geht nicht verloren, nur weil ein Knotentyp neu ist.
//!
//! Die Funktion ist rein und begrenzt (Tiefe, Blockzahl, Zeichen), weil der Inhalt aus einer fremden
//! Datei kommt.

use serde::Serialize;
use serde_json::Value;

/// Höchste Verschachtelungstiefe, die gelesen wird.
const MAX_TIEFE: usize = 64;
/// Höchstzahl Blöcke je Seite.
pub const MAX_BLOECKE: usize = 5_000;

/// Art eines Blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockArt {
    Absatz,
    Ueberschrift,
    Aufzaehlung,
    Aufgabe,
    Zitat,
    Code,
    Tabellenzelle,
    Bild,
    Datei,
    Zeichnung,
    Formel,
}

/// Ein Block der Seite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Block {
    /// Stabile Block-ID (`bid`, Spec §5.1); `None` bei älteren Seiten.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid: Option<String>,
    pub art: BlockArt,
    pub text: String,
    /// Überschriftenebene 1–3 bzw. Listentiefe ab 0.
    pub ebene: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kategorie: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub erledigt: Option<bool>,
    /// Erste Eingabe in den Block (ISO 8601, Attribut `zeit`); `None` bei älteren Seiten.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zeit: Option<String>,
    /// Letzte inhaltliche Änderung (Attribut `zeit_geaendert`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zeit_geaendert: Option<String>,
}

fn attr<'a>(n: &'a Value, name: &str) -> Option<&'a Value> {
    n.get("attrs").and_then(|a| a.get(name))
}

fn attr_text(n: &Value, name: &str) -> Option<String> {
    attr(n, name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(sauber)
}

/// Unsichtbare oder steuernde Zeichen, die der Assistent nicht zu sehen bekommt: Steuerzeichen
/// (außer Zeilenumbruch und Tab), Format-Zeichen (Bidi, Nullbreite, Wortverbinder, BOM, Tags) und
/// Zeilen-/Absatztrenner.
pub(crate) fn unsichtbar(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
        )
}

/// Entfernt unsichtbare und steuernde Zeichen (siehe [`unsichtbar`]).
pub(crate) fn sauber(s: &str) -> String {
    s.chars().filter(|c| !unsichtbar(*c)).collect()
}

/// Inline-Text eines Knotens (Text, Zeilenumbruch, Inline-Formel).
fn inline_text(n: &Value, tiefe: usize, out: &mut String) {
    if tiefe > MAX_TIEFE {
        return;
    }
    match n.get("type").and_then(Value::as_str) {
        Some("text") => out.push_str(&sauber(n.get("text").and_then(Value::as_str).unwrap_or(""))),
        Some("hardBreak") => out.push('\n'),
        Some("nbFormelZeile") => {
            if let Some(t) = attr_text(n, "tex") {
                out.push('$');
                out.push_str(&t);
                out.push('$');
            }
        }
        _ => {
            if let Some(k) = n.get("content").and_then(Value::as_array) {
                for c in k {
                    inline_text(c, tiefe + 1, out);
                }
            }
        }
    }
}

struct Sammler {
    bloecke: Vec<Block>,
}

impl Sammler {
    fn push(&mut self, art: BlockArt, text: String, ebene: u8, n: &Value, erbt: &Erbe) {
        if self.bloecke.len() >= MAX_BLOECKE {
            return;
        }
        let text = text.trim().to_string();
        let platzhalter = matches!(
            art,
            BlockArt::Bild | BlockArt::Datei | BlockArt::Zeichnung | BlockArt::Formel
        );
        if text.is_empty() && !platzhalter {
            return;
        }
        let bid = attr_text(n, "bid")
            .filter(|b| crate::gueltige_id(b))
            .or_else(|| erbt.bid.clone());
        self.bloecke.push(Block {
            bid,
            art,
            text,
            ebene,
            kategorie: attr_text(n, "kategorie").or_else(|| erbt.kategorie.clone()),
            erledigt: erbt.erledigt,
            zeit: attr_text(n, "zeit").or_else(|| erbt.zeit.clone()),
            zeit_geaendert: attr_text(n, "zeit_geaendert").or_else(|| erbt.zeit_geaendert.clone()),
        });
    }
}

/// Was ein Listenpunkt oder eine Aufgabe an den ersten Absatz weitergibt.
#[derive(Clone, Default)]
struct Erbe {
    art: Option<BlockArt>,
    kategorie: Option<String>,
    erledigt: Option<bool>,
    zeit: Option<String>,
    zeit_geaendert: Option<String>,
    bid: Option<String>,
    liste: u8,
}

fn lesen(n: &Value, tiefe: usize, erbt: &Erbe, s: &mut Sammler) {
    if tiefe > MAX_TIEFE || s.bloecke.len() >= MAX_BLOECKE {
        return;
    }
    let typ = n.get("type").and_then(Value::as_str).unwrap_or("");
    let kinder = || {
        n.get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    };
    let text_von = |n: &Value| {
        let mut t = String::new();
        inline_text(n, tiefe, &mut t);
        t
    };
    match typ {
        "paragraph" => {
            let art = erbt.art.unwrap_or(BlockArt::Absatz);
            s.push(art, text_von(n), erbt.liste, n, erbt);
        }
        "heading" => {
            let ebene = attr(n, "level")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6) as u8;
            s.push(BlockArt::Ueberschrift, text_von(n), ebene, n, erbt);
        }
        "codeBlock" => s.push(BlockArt::Code, text_von(n), 0, n, erbt),
        "nbBild" => {
            let alt = attr_text(n, "alt").unwrap_or_default();
            s.push(BlockArt::Bild, format!("[Bild: {alt}]"), 0, n, erbt);
        }
        "nbDatei" => {
            let name = attr_text(n, "name").unwrap_or_else(|| "Datei".into());
            s.push(BlockArt::Datei, format!("[Datei: {name}]"), 0, n, erbt);
        }
        "nbZeichnung" => s.push(BlockArt::Zeichnung, "[Zeichnung]".into(), 0, n, erbt),
        "nbFormel" => {
            let tex = attr_text(n, "tex").unwrap_or_default();
            s.push(BlockArt::Formel, format!("$${tex}$$"), 0, n, erbt);
        }
        "listItem" | "taskItem" => {
            let aufgabe = typ == "taskItem";
            let mut e = Erbe {
                art: Some(if aufgabe {
                    BlockArt::Aufgabe
                } else {
                    BlockArt::Aufzaehlung
                }),
                kategorie: attr_text(n, "kategorie").or_else(|| erbt.kategorie.clone()),
                erledigt: aufgabe
                    .then(|| attr(n, "checked").and_then(Value::as_bool) == Some(true)),
                zeit: attr_text(n, "zeit").or_else(|| erbt.zeit.clone()),
                zeit_geaendert: attr_text(n, "zeit_geaendert")
                    .or_else(|| erbt.zeit_geaendert.clone()),
                bid: attr_text(n, "bid").filter(|b| crate::gueltige_id(b)),
                liste: erbt.liste,
            };
            for (i, c) in kinder().enumerate() {
                if i > 0 && c.get("type").and_then(Value::as_str) == Some("paragraph") {
                    // Weitere Absätze im selben Punkt: gleiche Art, aber kein zweites Häkchen und
                    // keine geerbte Block-ID (sonst zwei Blöcke mit derselben ID).
                    e.erledigt = None;
                    e.bid = None;
                }
                lesen(c, tiefe + 1, &e, s);
            }
        }
        "bulletList" | "orderedList" | "taskList" => {
            let e = Erbe {
                liste: if erbt.art.is_some() {
                    erbt.liste.saturating_add(1)
                } else {
                    0
                },
                ..Erbe::default()
            };
            for c in kinder() {
                lesen(c, tiefe + 1, &e, s);
            }
        }
        "blockquote" => {
            let e = Erbe {
                art: Some(BlockArt::Zitat),
                zeit: attr_text(n, "zeit").or_else(|| erbt.zeit.clone()),
                ..erbt.clone()
            };
            for c in kinder() {
                lesen(c, tiefe + 1, &e, s);
            }
        }
        "tableCell" | "tableHeader" => {
            let e = Erbe {
                art: Some(BlockArt::Tabellenzelle),
                ..Erbe::default()
            };
            for c in kinder() {
                lesen(c, tiefe + 1, &e, s);
            }
        }
        "text" | "hardBreak" | "nbFormelZeile" => {
            // Lose Inline-Knoten ohne Absatz (sollte nicht vorkommen): als Absatz lesen.
            s.push(BlockArt::Absatz, text_von(n), 0, n, erbt);
        }
        _ => {
            for c in kinder() {
                lesen(c, tiefe + 1, erbt, s);
            }
        }
    }
}

/// Liest den Seiteninhalt (`inhalt` einer Seite) als flache Blockliste.
pub fn bloecke(inhalt: &Value) -> Vec<Block> {
    let mut s = Sammler {
        bloecke: Vec::new(),
    };
    lesen(inhalt, 0, &Erbe::default(), &mut s);
    s.bloecke
}

/// Klartext der Blöcke (eine Zeile je Block, Aufgaben mit `[ ]`/`[x]`), höchstens `max` Zeichen.
pub fn klartext(bloecke: &[Block], max: usize) -> String {
    let mut out = String::new();
    let mut zahl = 0usize;
    for b in bloecke {
        let zeile = match b.art {
            BlockArt::Ueberschrift => format!("{} {}", "#".repeat(b.ebene.max(1) as usize), b.text),
            BlockArt::Aufzaehlung => format!("{}- {}", "  ".repeat(b.ebene as usize), b.text),
            BlockArt::Aufgabe => {
                let h = if b.erledigt == Some(true) {
                    "[x]"
                } else {
                    "[ ]"
                };
                format!("{}- {h} {}", "  ".repeat(b.ebene as usize), b.text)
            }
            BlockArt::Zitat => format!("> {}", b.text),
            _ => b.text.clone(),
        };
        let laenge = zeile.chars().count() + 1;
        if zahl.saturating_add(laenge) > max {
            break;
        }
        zahl += laenge;
        out.push_str(&zeile);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn absatz(t: &str) -> Value {
        json!({"type": "paragraph", "content": [{"type": "text", "text": t}]})
    }

    #[test]
    fn struktur_kategorie_zeit_und_aufgaben() {
        let dok = json!({"type": "doc", "content": [
            {"type": "heading", "attrs": {"level": 2, "bid": "b1", "zeit": "2026-11-08T10:00:00Z", "zeit_geaendert": "2026-11-08T10:01:00Z"}, "content": [{"type": "text", "text": "Haushalt"}]},
            {"type": "paragraph", "attrs": {"kategorie": "wichtig", "zeit": "2026-11-08T10:05:00Z"},
             "content": [{"type": "text", "text": "Budget "}, {"type": "text", "marks": [{"type": "bold"}], "text": "kürzen"}]},
            {"type": "taskList", "content": [
                {"type": "taskItem", "attrs": {"checked": true, "bid": "t1"}, "content": [absatz("Angebot prüfen"), absatz("Zusatz")]},
                {"type": "taskItem", "attrs": {"checked": false}, "content": [absatz("Rückruf"),
                    {"type": "taskList", "content": [{"type": "taskItem", "attrs": {"checked": false}, "content": [absatz("Unterpunkt")]}]}]}
            ]},
            {"type": "bulletList", "content": [{"type": "listItem", "content": [absatz("Punkt")]}]},
            {"type": "nbBild", "attrs": {"anhangId": "a1", "alt": "Skizze"}},
            {"type": "nbDatei", "attrs": {"anhangId": "a2", "name": "plan.pdf"}},
            {"type": "unbekannt", "content": [absatz("nicht verlieren")]}
        ]});
        let mut b = bloecke(&dok);
        assert_eq!(b[0].art, BlockArt::Ueberschrift);
        assert_eq!(b[0].ebene, 2);
        assert_eq!(b[0].zeit.as_deref(), Some("2026-11-08T10:00:00Z"));
        assert_eq!(b[0].zeit_geaendert.as_deref(), Some("2026-11-08T10:01:00Z"));
        assert_eq!(b[0].bid.as_deref(), Some("b1"));
        assert_eq!(b[2].bid.as_deref(), Some("t1"));
        let zusatz = b.remove(3);
        assert_eq!(
            (zusatz.text.as_str(), zusatz.bid, zusatz.erledigt),
            ("Zusatz", None, None)
        );
        assert_eq!(b[1].text, "Budget kürzen");
        assert_eq!(b[1].kategorie.as_deref(), Some("wichtig"));
        assert_eq!((b[2].art, b[2].erledigt), (BlockArt::Aufgabe, Some(true)));
        assert_eq!(
            (b[3].text.as_str(), b[3].erledigt),
            ("Rückruf", Some(false))
        );
        assert_eq!((b[4].text.as_str(), b[4].ebene), ("Unterpunkt", 1));
        assert_eq!(b[5].art, BlockArt::Aufzaehlung);
        assert_eq!(b[6].text, "[Bild: Skizze]");
        assert_eq!(b[7].text, "[Datei: plan.pdf]");
        assert_eq!(b[8].text, "nicht verlieren");
        let t = klartext(&b, 10_000);
        assert!(t.contains("## Haushalt\n"));
        assert!(t.contains("- [x] Angebot prüfen\n"));
        assert!(t.contains("  - [ ] Unterpunkt\n"));
    }

    #[test]
    fn steuerzeichen_tiefe_und_grenzen() {
        let mut n = absatz("tief");
        for _ in 0..200 {
            n = json!({"type": "blockquote", "content": [n]});
        }
        assert!(bloecke(&n).is_empty());
        let b = bloecke(&absatz("a\u{202E}b\u{0007}c"));
        assert_eq!(b[0].text, "abc");
        assert_eq!(klartext(&b, 2), "");
        assert!(bloecke(&json!("kein objekt")).is_empty());
    }
}
