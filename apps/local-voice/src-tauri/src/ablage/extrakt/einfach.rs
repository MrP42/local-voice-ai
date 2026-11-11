//! TXT/MD/CSV (`zeile=<n>`), HTML (ohne Skripte, nichts wird nachgeladen; `absatz=<n>` + Überschriften)
//! und E-Mail (`teil=kopf|text|<n>`; Anhänge werden nie still aufgenommen).

use super::{Abschnitt, ExtraktFehler};

const ZEILEN_JE_ABSCHNITT: usize = 20;

/// Windows-1252 für 0x80–0x9F (Rest wie Latin-1). Die fünf unbelegten Stellen (0x81, 0x8D, 0x8F, 0x90,
/// 0x9D) bleiben, wie im WHATWG-Standard, die gleichnamigen Steuerzeichen.
const CP1252: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// ISO-8859-15: wie Latin-1, bis auf acht Stellen (€ Š š Ž ž Œ œ Ÿ).
const LATIN9: [(u8, char); 8] = [
    (0xA4, '€'),
    (0xA6, 'Š'),
    (0xA8, 'š'),
    (0xB4, 'Ž'),
    (0xB8, 'ž'),
    (0xBC, 'Œ'),
    (0xBD, 'œ'),
    (0xBE, 'Ÿ'),
];

fn windows_1252(b: &[u8]) -> String {
    b.iter()
        .map(|&x| match x {
            0x80..=0x9F => CP1252[usize::from(x - 0x80)],
            _ => x as char,
        })
        .collect()
}

fn latin9(b: &[u8]) -> String {
    b.iter()
        .map(|&x| {
            LATIN9
                .iter()
                .find(|(c, _)| *c == x)
                .map_or(x as char, |&(_, z)| z)
        })
        .collect()
}

/// UTF-16 ohne das BOM; ein halbes Zeichen am Ende und ungepaarte Surrogate werden zu U+FFFD.
fn utf16(b: &[u8], little: bool) -> String {
    let einheiten = b.chunks_exact(2).map(|p| {
        if little {
            u16::from_le_bytes([p[0], p[1]])
        } else {
            u16::from_be_bytes([p[0], p[1]])
        }
    });
    let mut s: String = char::decode_utf16(einheiten)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    if b.len() % 2 == 1 {
        s.push(char::REPLACEMENT_CHARACTER);
    }
    s
}

/// Zeichensatz aus einer Angabe (`charset=…`); nur das, was ohne Bibliothek sicher lesbar ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zeichensatz {
    Utf8,
    /// `windows-1252` und seine Etiketten (`latin1`, `iso-8859-1`, `ascii`: im Web gilt dafür 1252).
    Windows1252,
    Latin9,
}

fn zeichensatz_aus(name: &str) -> Option<Zeichensatz> {
    let n = name
        .trim_matches(|c: char| c.is_ascii_whitespace() || c == '"' || c == '\'')
        .to_ascii_lowercase();
    Some(match n.as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => Zeichensatz::Utf8,
        "windows-1252" | "cp1252" | "x-cp1252" | "iso-8859-1" | "iso8859-1" | "iso_8859-1"
        | "latin1" | "l1" | "us-ascii" | "ascii" | "ansi_x3.4-1968" => Zeichensatz::Windows1252,
        "iso-8859-15" | "iso8859-15" | "iso_8859-15" | "latin9" | "latin-9" | "l9" => {
            Zeichensatz::Latin9
        }
        _ => return None,
    })
}

/// Ein Attribut eines Tags: Name und Wert ohne Anführungszeichen (der Kopf ist bereits klein geschrieben).
type Attribut<'a> = (&'a str, &'a str);

/// Attribute ab `s` (direkt hinter dem Tagnamen) bis zum schließenden `>`, wie im Vorabscan des
/// HTML-Standards („get an attribute“): Werte in `"…"`/`'…'` dürfen `>` und `<` enthalten.
/// Liefert die Attribute und den Index hinter dem `>` (ohne `>` das Textende).
fn attribute(s: &str) -> (Vec<Attribut<'_>>, usize) {
    let b = s.as_bytes();
    let leer = |c: u8| c.is_ascii_whitespace();
    let mut i = 0;
    let mut attr = Vec::new();
    loop {
        while i < b.len() && (leer(b[i]) || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() {
            return (attr, b.len());
        }
        if b[i] == b'>' {
            return (attr, i + 1);
        }
        // Der Name beginnt mit beliebigem Zeichen (auch `=`) und endet vor Leerraum, `/`, `>` oder `=`.
        let n0 = i;
        i += 1;
        while i < b.len() && !leer(b[i]) && !matches!(b[i], b'/' | b'>' | b'=') {
            i += 1;
        }
        let name = &s[n0..i];
        while i < b.len() && leer(b[i]) {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            attr.push((name, ""));
            continue;
        }
        i += 1;
        while i < b.len() && leer(b[i]) {
            i += 1;
        }
        if i < b.len() && matches!(b[i], b'"' | b'\'') {
            let q = b[i];
            let v0 = i + 1;
            i = v0;
            while i < b.len() && b[i] != q {
                i += 1;
            }
            attr.push((name, &s[v0..i]));
            i = (i + 1).min(b.len());
        } else {
            let v0 = i;
            while i < b.len() && !leer(b[i]) && b[i] != b'>' {
                i += 1;
            }
            attr.push((name, &s[v0..i]));
        }
    }
}

/// `charset=…` aus dem Wert von `content` („extract a character encoding from a meta element“).
fn charset_aus_content(content: &str) -> Option<&str> {
    let mut pos = 0;
    while let Some(i) = content[pos..].find("charset") {
        let nach = pos + i + "charset".len();
        let Some(wert) = content[nach..].trim_start().strip_prefix('=') else {
            pos = nach;
            continue;
        };
        let wert = wert.trim_start();
        return match wert.chars().next() {
            Some(q @ ('"' | '\'')) => wert[1..].find(q).map(|e| &wert[1..1 + e]),
            Some(_) => {
                let e = wert
                    .find(|c: char| c.is_ascii_whitespace() || c == ';')
                    .unwrap_or(wert.len());
                Some(&wert[..e])
            }
            None => None,
        };
    }
    None
}

/// Zeichensatz eines `<meta>` aus seinen Attributen: `charset` nur als Attributname, `content` nur
/// zusammen mit `http-equiv=content-type`. Bei doppelten Attributen gilt das erste.
fn meta_zeichensatz(attr: &[Attribut<'_>]) -> Option<Zeichensatz> {
    let erstes = |n: &str| attr.iter().find(|(name, _)| *name == n).map(|&(_, w)| w);
    if let Some(cs) = erstes("charset") {
        return zeichensatz_aus(cs);
    }
    if erstes("http-equiv").map(str::trim) != Some("content-type") {
        return None;
    }
    erstes("content")
        .and_then(charset_aus_content)
        .and_then(zeichensatz_aus)
}

/// Zeichensatz aus `<meta charset=…>` oder `<meta http-equiv=Content-Type content="…; charset=…">` in den
/// ersten 1024 Bytes, nach dem Vorabscan des HTML-Standards (Groß-/Kleinschreibung egal, es wird nichts
/// ausgeführt). Jedes Tag wird mit seinen Attributen gelesen; Kommentare sowie `<script>`- und
/// `<style>`-Inhalte zählen nicht. Ein unbekannter Name (z. B. ein Mehrbytesatz) wird übergangen.
/// Elemente, deren Inhalt der HTML-Parser als Text liest; ein `<meta>` darin ist kein Element.
const ROH_TEXT: [&str; 8] = [
    "script", "style", "title", "textarea", "xmp", "iframe", "noembed", "noframes",
];

fn html_zeichensatz(b: &[u8]) -> Option<Zeichensatz> {
    let kopf: String = b[..b.len().min(1024)]
        .iter()
        .map(|&x| {
            if x.is_ascii() {
                char::from(x.to_ascii_lowercase())
            } else {
                '?'
            }
        })
        .collect();
    let grenze = |c: char| c.is_ascii_whitespace() || matches!(c, '>' | '/');
    let mut pos = 0;
    while let Some(rel) = kopf[pos..].find('<') {
        let start = pos + rel;
        let rest = &kopf[start..];
        if rest.starts_with("<!--") {
            pos = rest.find("-->").map_or(kopf.len(), |e| start + e + 3);
            continue;
        }
        let (end_tag, n0) = if rest.starts_with("</") {
            (true, 2)
        } else {
            (false, 1)
        };
        if !rest[n0..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            // `<!…`, `<?…` und `</` ohne Buchstaben enden am nächsten `>`; sonst ist `<` nur Text.
            pos = if rest[1..].starts_with(['!', '?', '/']) {
                rest.find('>').map_or(kopf.len(), |e| start + e + 1)
            } else {
                start + 1
            };
            continue;
        }
        let name_ende = rest[n0..].find(grenze).map_or(rest.len(), |e| n0 + e);
        let name = &rest[n0..name_ende];
        let (attr, ende) = attribute(&rest[name_ende..]);
        pos = start + name_ende + ende;
        if end_tag {
            continue;
        }
        if name == "meta" {
            if let Some(z) = meta_zeichensatz(&attr) {
                return Some(z);
            }
            continue;
        }
        if name == "plaintext" {
            // Alles Folgende ist Text.
            return None;
        }
        if ROH_TEXT.contains(&name) {
            // Inhalt ist Text (RAWTEXT/RCDATA): erst hinter dem vollständigen Starttag suchen; endet nur an
            // `</name` mit Namensgrenze.
            let zu = format!("</{name}");
            let mut such = pos;
            pos = kopf.len();
            while let Some(e) = kopf[such..].find(&zu) {
                let nach = such + e + zu.len();
                if kopf[nach..].chars().next().is_none_or(grenze) {
                    // Das Endtag samt zitierter Attribute bis `>` überspringen.
                    pos = nach + attribute(&kopf[nach..]).1;
                    break;
                }
                such = nach;
            }
        }
    }
    None
}

fn dekodieren_als(b: &[u8], z: Zeichensatz) -> String {
    match z {
        Zeichensatz::Utf8 => match std::str::from_utf8(b) {
            Ok(s) => s.to_string(),
            Err(_) => windows_1252(b),
        },
        Zeichensatz::Windows1252 => windows_1252(b),
        Zeichensatz::Latin9 => latin9(b),
    }
}

/// Bytes → Text. Ein BOM entscheidet (UTF-8, UTF-16 LE/BE); sonst `vorgabe` (aus einer Angabe im
/// Dokument), sonst gilt gültiges UTF-8 als UTF-8 und alles andere als Windows-1252.
fn dekodieren_mit(bytes: &[u8], vorgabe: Option<Zeichensatz>) -> String {
    if let Some(b) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return dekodieren_als(b, Zeichensatz::Utf8);
    }
    if let Some(b) = bytes.strip_prefix(b"\xFF\xFE") {
        return utf16(b, true);
    }
    if let Some(b) = bytes.strip_prefix(b"\xFE\xFF") {
        return utf16(b, false);
    }
    dekodieren_als(bytes, vorgabe.unwrap_or(Zeichensatz::Utf8))
}

/// Text- und CSV-Dateien: UTF-8 (mit oder ohne BOM), UTF-16 mit BOM, sonst Windows-1252.
pub fn dekodieren(bytes: &[u8]) -> String {
    dekodieren_mit(bytes, None)
}

/// HTML: wie [`dekodieren`], dazu die Angabe `charset` im Kopf der Datei.
pub fn html_dekodieren(bytes: &[u8]) -> String {
    dekodieren_mit(bytes, html_zeichensatz(bytes))
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
    let s = html_dekodieren(bytes);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16_bytes(s: &str, little: bool, bom: bool) -> Vec<u8> {
        let mut b = Vec::new();
        if bom {
            b.extend(if little { [0xFF, 0xFE] } else { [0xFE, 0xFF] });
        }
        for u in s.encode_utf16() {
            b.extend(if little {
                u.to_le_bytes()
            } else {
                u.to_be_bytes()
            });
        }
        b
    }

    /// lva#508: 0x80–0x9F sind in Windows-1252 Satzzeichen (€, „ “ – …), nicht U+0080–U+009F.
    #[test]
    fn windows_1252_tabelle_fuer_0x80_bis_0x9f() {
        let alle: Vec<u8> = (0x80..=0x9F).collect();
        let s = dekodieren(&alle);
        assert_eq!(s.chars().count(), 32);
        assert_eq!(dekodieren(b"Preis 5\x80"), "Preis 5€");
        assert_eq!(
            dekodieren(b"\x93Zitat\x94 \x96 \x85 \x99 \x8A\x9A \x8C\x9C \x9F"),
            "“Zitat” – … ™ Šš Œœ Ÿ"
        );
        assert_eq!(dekodieren(b"\x84Hallo\x93"), "„Hallo“");
        // Unbelegte Stellen bleiben Steuerzeichen (WHATWG), Latin-1-Bereich unverändert.
        assert_eq!(
            dekodieren(b"\x81\x8D\x8F\x90\x9D"),
            "\u{81}\u{8D}\u{8F}\u{90}\u{9D}"
        );
        assert_eq!(dekodieren(b"K\xFCndigung \xDF \xE4\xF6"), "Kündigung ß äö");
        // Kein Steuerzeichen in der Ausgabe für die belegten Stellen.
        assert!(!s
            .chars()
            .any(|c| ('\u{80}'..='\u{9F}').contains(&c)
                && !"\u{81}\u{8D}\u{8F}\u{90}\u{9D}".contains(c)));
    }

    #[test]
    fn utf8_bleibt_utf8_auch_mit_bom() {
        assert_eq!(dekodieren("Größe €".as_bytes()), "Größe €");
        assert_eq!(dekodieren(b"\xEF\xBB\xBFGr\xC3\xB6\xC3\x9Fe"), "Größe");
    }

    #[test]
    fn utf16_mit_bom_wird_gelesen() {
        let t = "Kündigung zum Quartal – 5 €\n𝄞 zweite Zeile";
        assert_eq!(dekodieren(&utf16_bytes(t, true, true)), t);
        assert_eq!(dekodieren(&utf16_bytes(t, false, true)), t);
        // Halbes Zeichen am Ende und ungepaartes Surrogat: kein Absturz, U+FFFD.
        let mut kaputt = utf16_bytes("ab", true, true);
        kaputt.push(0x41);
        assert_eq!(dekodieren(&kaputt), "ab\u{FFFD}");
        let mut halb = vec![0xFF, 0xFE, 0x00, 0xD8, 0x41, 0x00];
        assert_eq!(dekodieren(&halb), "\u{FFFD}A");
        halb.truncate(2);
        assert_eq!(dekodieren(&halb), "");
        // Als Textabschnitt (Zeilen) kommt der Inhalt an.
        let a = text(&utf16_bytes("Zeile eins\nZeile zwei", true, true));
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].text, "Zeile eins\nZeile zwei");
    }

    #[test]
    fn html_charset_aus_dem_kopf() {
        let mit = |meta: &str, rumpf: &[u8]| {
            let mut b = format!("<html><head>{meta}<title>t</title></head><body><p>").into_bytes();
            b.extend_from_slice(rumpf);
            b.extend_from_slice(b"</p></body></html>");
            b
        };
        let text_von = |b: &[u8]| {
            html(b)
                .into_iter()
                .map(|a| a.text)
                .collect::<Vec<_>>()
                .join("|")
        };
        for meta in [
            r#"<meta charset="windows-1252">"#,
            "<META CHARSET=Windows-1252>",
            r#"<meta http-equiv="Content-Type" content="text/html; charset=ISO-8859-1">"#,
            "<meta charset='latin1'>",
        ] {
            let t = text_von(&mit(meta, b"5\x80 \x93zu\x94 K\xFChl"));
            assert_eq!(t, "5€ “zu” Kühl", "{meta}");
        }
        // Latin-9: 0xA4 ist €, nicht ¤.
        assert_eq!(
            text_von(&mit("<meta charset=iso-8859-15>", b"5\xA4 \xBD")),
            "5€ œ"
        );
        // Deklariert UTF-8 und ist UTF-8.
        assert_eq!(
            text_von(&mit("<meta charset=utf-8>", "Größe €".as_bytes())),
            "Größe €"
        );
        // Ohne Angabe: gültiges UTF-8 bleibt, sonst Windows-1252.
        assert_eq!(text_von(&mit("", "Größe €".as_bytes())), "Größe €");
        assert_eq!(text_von(&mit("", b"5\x80")), "5€");
        // „charset“ außerhalb von <meta> zählt nicht.
        let b = b"<html><body><p>charset=windows-1252 \xC3\xA4</p></body></html>";
        assert_eq!(text_von(b), "charset=windows-1252 ä");
        // UTF-16 mit BOM im HTML.
        let u = utf16_bytes("<html><body><p>Größe €</p></body></html>", true, true);
        assert_eq!(text_von(&u), "Größe €");
    }

    /// Codex-Befund 5: `<meta>` in Kommentaren, `<script>` und `<style>` zählt nicht.
    #[test]
    fn html_charset_ignoriert_kommentar_script_und_style() {
        let text_von = |b: &[u8]| {
            html(b)
                .into_iter()
                .map(|a| a.text)
                .collect::<Vec<_>>()
                .join("|")
        };
        let utf8 = "ä".as_bytes();
        for kopf in [
            "<!-- <meta charset=windows-1252> -->",
            "<script>var s = '<meta charset=windows-1252>';</script>",
            "<SCRIPT type=text/javascript>x='<meta charset=windows-1252>'</SCRIPT>",
            "<style>/* <meta charset=windows-1252> */</style>",
            "<script>let s=\"</scriptx><meta charset=windows-1252>\";</script>",
            "<metadata charset=windows-1252></metadata>",
            "<metadata title=\"<meta charset=windows-1252>\"></metadata>",
            "<div data-x='<meta charset=windows-1252>'></div>",
            "</div title=\"<meta charset=windows-1252>\">",
            // Codex Runde 4: Endtag-Suche erst hinter dem Starttag; `charset` nur als Attributname;
            // `content` nur mit `http-equiv=content-type`.
            "<script title=\"</script><meta charset=windows-1252>\"></script>",
            "<style title='</style><meta charset=windows-1252>'></style>",
            "<meta title=\"charset=windows-1252\">",
            "<meta data-charset=windows-1252>",
            "<meta content=\"text/html; charset=windows-1252\">",
            "<meta http-equiv=refresh content=\"0; charset=windows-1252\">",
            "<meta name=x content='charset=windows-1252'>",
            "<script></script title=\"<meta charset=windows-1252>\">",
            "<style></style title='<meta charset=windows-1252>'>",
            "<title><meta charset=windows-1252></title>",
            "<textarea><meta charset=windows-1252></textarea>",
            "<xmp><meta charset=windows-1252></xmp>",
            "<iframe><meta charset=windows-1252></iframe>",
            "<noembed><meta charset=windows-1252></noembed>",
            "<noframes><meta charset=windows-1252></noframes>",
            "<plaintext></plaintext><meta charset=windows-1252>",
        ] {
            let mut b = kopf.as_bytes().to_vec();
            b.extend_from_slice(b"<p>");
            b.extend_from_slice(utf8);
            b.extend_from_slice(b"</p>");
            // Rohtext-Inhalte (z. B. `<textarea>`) dürfen als Text erscheinen; entscheidend ist UTF-8.
            let t = text_von(&b);
            assert!(t.contains('ä') && !t.contains('Ã'), "{kopf}: {t}");
        }
        // Ein echtes `<meta>` hinter einem Kommentar gilt weiterhin.
        let b = b"<!-- x --><meta charset=windows-1252><p>5\x80</p>";
        assert_eq!(text_von(b), "5€");
        // Zitiertes `>` vor einem echten `<meta>` beendet das vorige Tag nicht vorzeitig.
        let b = b"<div title=\"a>b\"></div><meta charset=windows-1252><p>5\x80</p>";
        assert_eq!(text_von(b), "5€");
        // Echte Angaben mit weiteren Attributen gelten.
        for kopf in [
            "<meta name=x charset=windows-1252>",
            "<meta content=\"text/html; charset=windows-1252\" http-equiv=Content-Type>",
            "<meta http-equiv='content-type' content='text/html;charset=\"windows-1252\"'>",
            "<script src=a></script><meta charset=windows-1252>",
            "<meta charset=shift_jis><meta charset=windows-1252>",
        ] {
            let mut b = kopf.as_bytes().to_vec();
            b.extend_from_slice(b"<p>5\x80</p>");
            assert_eq!(text_von(&b), "5€", "{kopf}");
        }
    }

    #[test]
    fn unbekannter_zeichensatz_faellt_auf_die_vorgabe_zurueck() {
        let b = b"<html><head><meta charset=\"shift_jis\"></head><body><p>5\x80</p></body></html>";
        assert_eq!(html(b)[0].text, "5€");
    }
}
