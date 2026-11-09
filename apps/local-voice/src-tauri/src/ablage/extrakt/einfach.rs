//! TXT/MD/CSV (`zeile=<n>`), HTML (ohne Skripte, nichts wird nachgeladen; `absatz=<n>` + Überschriften)
//! und E-Mail (`teil=kopf|text|<n>`; Anhänge werden nie still aufgenommen).

use super::{Abschnitt, ExtraktFehler};

const ZEILEN_JE_ABSCHNITT: usize = 20;

/// UTF-8 (mit oder ohne BOM), sonst Windows-1252/Latin-1 Byte für Byte.
pub fn dekodieren(bytes: &[u8]) -> String {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&x| x as char).collect(),
    }
}

/// Zeilen in Blöcken (Leerzeile oder höchstens 20 Zeilen); Anker ist die erste Zeile des Blocks.
pub fn text(bytes: &[u8]) -> Vec<Abschnitt> {
    let s = dekodieren(bytes);
    let mut aus = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut fertig = |block: &mut Vec<&str>, start: usize| {
        if !block.is_empty() {
            aus.push(Abschnitt {
                anker: format!("zeile={start}"),
                pfad: vec![],
                text: block.join("\n"),
            });
            block.clear();
        }
    };
    for (i, zeile) in s.lines().enumerate() {
        if zeile.trim().is_empty() {
            fertig(&mut block, start);
            continue;
        }
        if block.is_empty() {
            start = i + 1;
        }
        block.push(zeile);
        if block.len() >= ZEILEN_JE_ABSCHNITT {
            fertig(&mut block, start);
        }
    }
    fertig(&mut block, start);
    aus
}

const AUSGELASSEN: &[&str] = &[
    "script", "style", "noscript", "template", "iframe", "object", "embed", "svg", "head",
];
const BLOCK: &[&str] = &[
    "p",
    "div",
    "li",
    "tr",
    "td",
    "th",
    "section",
    "article",
    "blockquote",
    "pre",
    "table",
    "ul",
    "ol",
    "dd",
    "dt",
    "header",
    "footer",
    "main",
    "nav",
    "aside",
    "figure",
    "figcaption",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
];

struct HtmlSammler {
    n: u32,
    pfad: Vec<String>,
    puffer: String,
    aus: Vec<Abschnitt>,
}

impl HtmlSammler {
    fn absatz(&mut self, ebene: Option<usize>) {
        let text = self.puffer.split_whitespace().collect::<Vec<_>>().join(" ");
        self.puffer.clear();
        if text.is_empty() {
            return;
        }
        self.n += 1;
        self.aus.push(Abschnitt {
            anker: format!("absatz={}", self.n),
            pfad: self.pfad.clone(),
            text: text.clone(),
        });
        if let Some(e) = ebene {
            self.pfad.truncate(e - 1);
            self.pfad.push(text.chars().take(200).collect());
        }
    }

    fn laufen(&mut self, el: scraper::ElementRef<'_>, tiefe: usize) {
        use scraper::Node;
        if tiefe > 256 {
            return;
        }
        for kind in el.children() {
            match kind.value() {
                Node::Text(t) => self.puffer.push_str(t),
                Node::Element(e) => {
                    let name = e.name().to_ascii_lowercase();
                    if AUSGELASSEN.contains(&name.as_str()) {
                        continue;
                    }
                    if name == "br" {
                        self.puffer.push(' ');
                        continue;
                    }
                    let Some(k) = scraper::ElementRef::wrap(kind) else {
                        continue;
                    };
                    let ebene = name
                        .strip_prefix('h')
                        .and_then(|r| r.parse::<usize>().ok())
                        .filter(|e| (1..=6).contains(e));
                    if BLOCK.contains(&name.as_str()) {
                        self.absatz(None);
                        self.laufen(k, tiefe + 1);
                        self.absatz(ebene);
                    } else {
                        self.laufen(k, tiefe + 1);
                    }
                }
                _ => {}
            }
        }
    }
}

pub fn html(bytes: &[u8]) -> Vec<Abschnitt> {
    let s = dekodieren(bytes);
    let doc = scraper::Html::parse_document(&s);
    let mut h = HtmlSammler {
        n: 0,
        pfad: vec![],
        puffer: String::new(),
        aus: vec![],
    };
    let wurzel = doc.root_element();
    // Nur der Körper (der Kopf enthält Skripte, Stile, Metadaten).
    let sel = scraper::Selector::parse("body").ok();
    match sel.as_ref().and_then(|s| doc.select(s).next()) {
        Some(body) => h.laufen(body, 0),
        None => h.laufen(wurzel, 0),
    }
    h.absatz(None);
    h.aus
}

pub fn eml(bytes: &[u8]) -> Result<Vec<Abschnitt>, ExtraktFehler> {
    let m = mail_parser::MessageParser::default()
        .parse(bytes)
        .ok_or(ExtraktFehler::neu("mail"))?;
    let adressen = |a: Option<&mail_parser::Address<'_>>| -> String {
        a.map(|a| {
            a.iter()
                .map(|x| match (x.name(), x.address()) {
                    (Some(n), Some(ad)) => format!("{n} <{ad}>"),
                    (None, Some(ad)) => ad.to_string(),
                    (Some(n), None) => n.to_string(),
                    (None, None) => String::new(),
                })
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
    };
    let mut kopf = Vec::new();
    for (k, v) in [
        ("Von", adressen(m.from())),
        ("An", adressen(m.to())),
        ("Cc", adressen(m.cc())),
        ("Betreff", m.subject().unwrap_or_default().to_string()),
        (
            "Datum",
            m.date().map(|d| d.to_rfc3339()).unwrap_or_default(),
        ),
    ] {
        if !v.trim().is_empty() {
            kopf.push(format!("{k}: {v}"));
        }
    }
    let mut aus = vec![Abschnitt {
        anker: "teil=kopf".into(),
        pfad: vec![],
        text: kopf.join("\n"),
    }];
    for i in 0..m.text_body_count() {
        let Some(t) = m.body_text(i) else { continue };
        aus.push(Abschnitt {
            anker: if i == 0 {
                "teil=text".into()
            } else {
                format!("teil={}", i + 1)
            },
            pfad: vec![],
            text: t.into_owned(),
        });
    }
    Ok(aus)
}
