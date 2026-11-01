//! Bücher als Vorlesetext importieren (lva#149).
//!
//! - **EPUB:** `META-INF/container.xml` → OPF → Spine-Reihenfolge; jedes
//!   XHTML-Kapitel wird zur Markdown-Teilmenge des Lesedokuments (lva#148):
//!   `#`/`##`/`###` für Überschriften, Absätze durch Leerzeilen, `*kursiv*`,
//!   `**fett**`, `> ` für Zitate, `- ` für Listen, `---` für Trenner.
//! - **PDF-Reflow:** pdf-extract liefert harte Zeilenumbrüche je Druckzeile.
//!   Die werden zu Absätzen zusammengeführt (Silbentrennung aufgehoben,
//!   Seitenzahlen entfernt), damit der Vorleser keine Pausen mitten im Satz
//!   macht.
//!
//! Text, der im Lesedokument eine Bedeutung hätte, wird entschärft: `<…>`
//! wäre ein Sprecher, `[…]` ein Tag — im Buchtext werden daraus `‹…›` und
//! `(…)`. Reine Funktionen ohne Tauri, damit sie für sich testbar sind.

use std::collections::HashMap;
use std::io::{Read, Seek};

/// Höchstgröße eines einzelnen Eintrags im EPUB (Schutz vor Zip-Bomben).
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;

fn read_entry<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String, String> {
    let entry = archive
        .by_name(name)
        .map_err(|_| format!("EPUB: Eintrag „{name}“ fehlt"))?;
    let mut bytes = Vec::new();
    entry
        .take(MAX_ENTRY_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("EPUB: „{name}“ nicht lesbar: {e}"))?;
    Ok(String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).into_owned())
}

/// Attribute eines Start-Tags (`id="x" href='y'`) als Tabelle, Namen klein.
fn attributes(tag: &str) -> HashMap<String, String> {
    let re = regex::Regex::new(r#"([A-Za-z_][\w:.-]*)\s*=\s*(?:"([^"]*)"|'([^']*)')"#)
        .expect("static regex");
    re.captures_iter(tag)
        .map(|c| {
            let value = c.get(2).or_else(|| c.get(3)).map_or("", |m| m.as_str());
            (c[1].to_ascii_lowercase(), decode_entities(value))
        })
        .collect()
}

/// `%20` und Co. in Pfaden auflösen (EPUB-hrefs sind URL-kodiert).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Relativen Pfad gegen das Verzeichnis der OPF-Datei auflösen.
fn resolve_path(base_dir: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or_default();
    let mut parts: Vec<&str> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    percent_decode(&parts.join("/"))
}

/// Ein EPUB (als Zip-Archiv) in den Vorlesetext mit Markdown-Teilmenge wandeln.
pub fn epub_to_markdown<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<String, String> {
    let container = read_entry(archive, "META-INF/container.xml")?;
    let rootfile_re = regex::Regex::new(r"(?is)<rootfile\b[^>]*>").expect("static regex");
    let opf_path = rootfile_re
        .find_iter(&container)
        .map(|m| attributes(m.as_str()))
        .find(|a| {
            a.get("media-type")
                .is_none_or(|t| t == "application/oebps-package+xml")
        })
        .and_then(|a| a.get("full-path").cloned())
        .ok_or("EPUB: kein Inhaltsverzeichnis (rootfile) gefunden")?;
    let opf = read_entry(archive, &opf_path)?;
    let base_dir = opf_path.rsplit_once('/').map_or("", |(d, _)| d);

    let item_re = regex::Regex::new(r"(?is)<(?:opf:)?item\b[^>]*>").expect("static regex");
    let mut manifest: HashMap<String, (String, String)> = HashMap::new();
    for m in item_re.find_iter(&opf) {
        let a = attributes(m.as_str());
        if let (Some(id), Some(href)) = (a.get("id"), a.get("href")) {
            let media = a.get("media-type").cloned().unwrap_or_default();
            manifest.insert(id.clone(), (resolve_path(base_dir, href), media));
        }
    }
    let itemref_re = regex::Regex::new(r"(?is)<(?:opf:)?itemref\b[^>]*>").expect("static regex");
    let spine: Vec<String> = itemref_re
        .find_iter(&opf)
        .map(|m| attributes(m.as_str()))
        .filter_map(|a| a.get("idref").cloned())
        .collect();
    if spine.is_empty() {
        return Err("EPUB: leere Lesereihenfolge (spine)".into());
    }

    let mut chapters: Vec<String> = Vec::new();
    for idref in spine {
        let Some((path, media)) = manifest.get(&idref) else {
            continue;
        };
        let is_html = media.contains("html")
            || path.ends_with(".xhtml")
            || path.ends_with(".html")
            || path.ends_with(".htm");
        if !is_html {
            continue;
        }
        let Ok(xhtml) = read_entry(archive, path) else {
            continue;
        };
        let md = xhtml_to_markdown(&xhtml);
        if !md.is_empty() {
            chapters.push(md);
        }
    }
    let text = chapters.join("\n\n");
    if text.trim().is_empty() {
        return Err("Das EPUB enthält keinen lesbaren Text (evtl. DRM-geschützt)".into());
    }
    Ok(text)
}

/// Benannte und numerische Entities auflösen (die üblichen in E-Books).
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let re = regex::Regex::new(r"&(#[xX][0-9a-fA-F]+|#[0-9]+|[A-Za-z]+);").expect("static regex");
    re.replace_all(s, |c: &regex::Captures| {
        let name = &c[1];
        let ch = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = name.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "shy" => Some('\u{AD}'),
                "ndash" => Some('–'),
                "mdash" => Some('—'),
                "hellip" => Some('…'),
                "laquo" => Some('«'),
                "raquo" => Some('»'),
                "lsquo" => Some('‘'),
                "rsquo" => Some('’'),
                "sbquo" => Some('‚'),
                "ldquo" => Some('“'),
                "rdquo" => Some('”'),
                "bdquo" => Some('„'),
                "auml" => Some('ä'),
                "ouml" => Some('ö'),
                "uuml" => Some('ü'),
                "Auml" => Some('Ä'),
                "Ouml" => Some('Ö'),
                "Uuml" => Some('Ü'),
                "szlig" => Some('ß'),
                "eacute" => Some('é'),
                "egrave" => Some('è'),
                "copy" => Some('©'),
                _ => None,
            }
        };
        ch.map_or_else(|| c[0].to_string(), |ch| ch.to_string())
    })
    .into_owned()
}

/// Zeichen entschärfen, die im Lesedokument Marker wären, und Leerraum
/// vereinheitlichen.
fn neutralize(text: &str) -> String {
    text.chars()
        .filter(|&c| c != '\u{AD}')
        .map(|c| match c {
            '<' => '‹',
            '>' => '›',
            '[' => '(',
            ']' => ')',
            c if c.is_whitespace() => ' ',
            c => c,
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Default)]
struct Style {
    bold: bool,
    italic: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum BlockKind {
    Para,
    Heading(u8),
    Item,
}

struct Builder {
    blocks: Vec<String>,
    segs: Vec<(String, Style)>,
    kind: BlockKind,
    quote: usize,
    bold: usize,
    italic: usize,
}

impl Builder {
    fn new() -> Self {
        Self {
            blocks: Vec::new(),
            segs: Vec::new(),
            kind: BlockKind::Para,
            quote: 0,
            bold: 0,
            italic: 0,
        }
    }

    fn text(&mut self, raw: &str) {
        let t = neutralize(&decode_entities(raw));
        if t.is_empty() {
            return;
        }
        let style = Style {
            bold: self.bold > 0,
            italic: self.italic > 0,
        };
        match self.segs.last_mut() {
            Some((s, st)) if *st == style => s.push_str(&t),
            _ => self.segs.push((t, style)),
        }
    }

    /// Aktuellen Block abschließen und als Markdown-Zeile ablegen.
    fn flush(&mut self) {
        let segs = std::mem::take(&mut self.segs);
        let mut line = String::new();
        for (text, style) in segs {
            // Mehrfachen Leerraum verdichten, Ränder außerhalb der Marker halten.
            let collapsed = text
                .split(' ')
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let lead = text.starts_with(' ');
            let trail = text.ends_with(' ') && !collapsed.is_empty();
            if collapsed.is_empty() {
                if text.contains(' ') && !line.ends_with(' ') {
                    line.push(' ');
                }
                continue;
            }
            if lead && !line.is_empty() && !line.ends_with(' ') {
                line.push(' ');
            }
            let marker = match (style.bold, style.italic) {
                (true, true) => "***",
                (true, false) => "**",
                (false, true) => "*",
                (false, false) => "",
            };
            line.push_str(marker);
            line.push_str(&collapsed);
            line.push_str(marker);
            if trail {
                line.push(' ');
            }
        }
        let line = line.trim();
        if !line.is_empty() {
            let prefix = match self.kind {
                BlockKind::Heading(1) => "# ".to_string(),
                BlockKind::Heading(2) => "## ".to_string(),
                BlockKind::Heading(_) => "### ".to_string(),
                BlockKind::Item => "- ".to_string(),
                BlockKind::Para => String::new(),
            };
            let quote = if self.quote > 0 && !matches!(self.kind, BlockKind::Heading(_)) {
                "> "
            } else {
                ""
            };
            self.blocks.push(format!("{quote}{prefix}{line}"));
        }
        self.kind = BlockKind::Para;
    }

    fn rule(&mut self) {
        self.flush();
        if self.blocks.last().is_some_and(|b| b != "---") {
            self.blocks.push("---".into());
        }
    }
}

/// Ein XHTML-Kapitel in die Markdown-Teilmenge des Lesedokuments wandeln.
pub fn xhtml_to_markdown(xhtml: &str) -> String {
    let hidden = regex::Regex::new(
        r"(?is)<(head|script|style|svg|math|nav|aside)\b.*?</(head|script|style|svg|math|nav|aside)\s*>",
    )
    .expect("static regex");
    let comments =
        regex::Regex::new(r"(?s)<!--.*?-->|<\?.*?\?>|<!\[CDATA\[.*?\]\]>|<!DOCTYPE[^>]*>")
            .expect("static regex");
    let cleaned = comments.replace_all(xhtml, "");
    let cleaned = hidden.replace_all(&cleaned, "");
    let body = regex::Regex::new(r"(?is)<body\b[^>]*>(.*)</body\s*>")
        .expect("static regex")
        .captures(&cleaned)
        .map_or(cleaned.to_string(), |c| c[1].to_string());

    let tag_re =
        regex::Regex::new(r"(?s)<(/?)([A-Za-z][\w:-]*)([^>]*?)(/?)>").expect("static regex");
    let mut b = Builder::new();
    let mut last = 0;
    for cap in tag_re.captures_iter(&body) {
        let whole = cap.get(0).expect("match");
        b.text(&body[last..whole.start()]);
        last = whole.end();
        let closing = !cap[1].is_empty();
        let self_closing = !cap[4].is_empty();
        let name = cap[2].to_ascii_lowercase();
        let name = name.rsplit(':').next().unwrap_or(&name).to_string();
        match name.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                b.flush();
                if !closing {
                    b.kind = BlockKind::Heading(name.as_bytes()[1] - b'0');
                }
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "figure" | "figcaption"
            | "table" | "tr" | "dt" | "dd" | "pre" => b.flush(),
            "li" => {
                b.flush();
                if !closing {
                    b.kind = BlockKind::Item;
                }
            }
            "blockquote" => {
                b.flush();
                if closing {
                    b.quote = b.quote.saturating_sub(1);
                } else if !self_closing {
                    b.quote += 1;
                }
            }
            "br" => b.text(" "),
            "td" | "th" => b.text(" "),
            "hr" => b.rule(),
            "em" | "i" | "cite" | "dfn" if !self_closing => {
                if closing {
                    b.italic = b.italic.saturating_sub(1);
                } else {
                    b.italic += 1;
                }
            }
            "strong" | "b" if !self_closing => {
                if closing {
                    b.bold = b.bold.saturating_sub(1);
                } else {
                    b.bold += 1;
                }
            }
            // Hochgestelltes (Fußnotenziffern) vom Wort davor trennen.
            "sup" if !closing && !self_closing => b.text(" "),
            _ => {}
        }
    }
    b.text(&body[last..]);
    b.flush();
    while b.blocks.last().is_some_and(|l| l == "---") {
        b.blocks.pop();
    }
    b.blocks.join("\n\n")
}

/// PDF-Text: Druckzeilen zu Absätzen zusammenführen.
///
/// - Leerzeilen trennen Absätze.
/// - Eine Zeile, die mit Satzzeichen endet und deutlich kürzer ist als die
///   üblichen Zeilen, beendet den Absatz.
/// - Trennstriche am Zeilenende vor Kleinbuchstaben werden aufgehoben.
/// - Zeilen nur aus einer Zahl (Seitenzahlen) und Seitenvorschübe entfallen.
pub fn reflow_plain_text(text: &str) -> String {
    let lines: Vec<String> = text
        .replace('\u{0C}', "\n")
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    let mut lengths: Vec<usize> = lines
        .iter()
        .map(|l| l.chars().count())
        .filter(|&n| n > 20)
        .collect();
    lengths.sort_unstable();
    let typical = lengths.get(lengths.len() * 3 / 4).copied().unwrap_or(0);

    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    let end_paragraph = |current: &mut String, paragraphs: &mut Vec<String>| {
        if !current.trim().is_empty() {
            paragraphs.push(current.trim().to_string());
        }
        current.clear();
    };
    for line in &lines {
        if line.is_empty() {
            end_paragraph(&mut current, &mut paragraphs);
            continue;
        }
        if line.chars().all(|c| c.is_ascii_digit()) && line.len() <= 4 {
            continue;
        }
        if current.is_empty() {
            current.push_str(line);
        } else if current.ends_with('-')
            && !current.ends_with(" -")
            && line.chars().next().is_some_and(char::is_lowercase)
        {
            current.pop();
            current.push_str(line);
        } else {
            current.push(' ');
            current.push_str(line);
        }
        let n = line.chars().count();
        let ends_sentence = line
            .trim_end_matches(['"', '“', '”', '«', '»', '’', '\'', ')'])
            .ends_with(['.', '!', '?', ':', '…']);
        if typical > 0 && ends_sentence && n * 10 < typical * 8 {
            end_paragraph(&mut current, &mut paragraphs);
        }
    }
    end_paragraph(&mut current, &mut paragraphs);
    paragraphs
        .iter()
        .map(|p| neutralize(p))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn epub(files: &[(&str, &str)]) -> zip::ZipArchive<std::io::Cursor<Vec<u8>>> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, body) in files {
                w.start_file(*name, opts).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        buf.set_position(0);
        zip::ZipArchive::new(buf).unwrap()
    }

    const CONTAINER: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

    const OPF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata><dc:title>Der Leuchtturm</dc:title></metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="c2" href="Text/kapitel%202.xhtml" media-type="application/xhtml+xml"/>
    <item id="c1" href="Text/kapitel1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img" href="Images/cover.jpg" media-type="image/jpeg"/>
  </manifest>
  <spine><itemref idref="c1"/><itemref idref="img"/><itemref idref="c2"/></spine>
</package>"#;

    #[test]
    fn epub_follows_the_spine_and_keeps_formatting() {
        let mut a = epub(&[
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", OPF),
            (
                "OEBPS/Text/kapitel1.xhtml",
                r#"<html><head><title>x</title><style>p{}</style></head><body>
<h1>Kapitel 1</h1>
<p>Der Leuchtturm stand <em>seit über</em> hundert Jahren auf der <strong>Klippe</strong>.</p>
<p>Er war ca.&#160;42&nbsp;Meter hoch &amp; „schön“.</p>
<blockquote><p>Ein Zitat.</p></blockquote>
<hr/>
<ul><li>Eins</li><li>Zwei</li></ul>
</body></html>"#,
            ),
            (
                "OEBPS/Text/kapitel 2.xhtml",
                r#"<html><body><h2 class="c">Zwei</h2><p>Sie sagte <i>leise</i>: [Anm.] &lt;Mara&gt;</p></body></html>"#,
            ),
        ]);
        let md = epub_to_markdown(&mut a).unwrap();
        assert_eq!(
            md,
            "# Kapitel 1\n\n\
             Der Leuchtturm stand *seit über* hundert Jahren auf der **Klippe**.\n\n\
             Er war ca. 42 Meter hoch & „schön“.\n\n\
             > Ein Zitat.\n\n\
             ---\n\n\
             - Eins\n\n\
             - Zwei\n\n\
             ## Zwei\n\n\
             Sie sagte *leise*: (Anm.) ‹Mara›"
        );
    }

    #[test]
    fn epub_without_container_is_an_error() {
        let mut a = epub(&[("mimetype", "application/epub+zip")]);
        assert!(epub_to_markdown(&mut a).is_err());
    }

    #[test]
    fn emphasis_keeps_spaces_outside_the_markers() {
        let md =
            xhtml_to_markdown("<body><p>Ein<em> kursives </em>Wort und <b>fett</b>.</p></body>");
        assert_eq!(md, "Ein *kursives* Wort und **fett**.");
    }

    #[test]
    fn empty_emphasis_and_nested_blocks_vanish() {
        let md = xhtml_to_markdown(
            "<body><div><p>A<em> </em>B</p><div><p></p></div><p>C<br/>D</p></div></body>",
        );
        assert_eq!(md, "A B\n\nC D");
    }

    #[test]
    fn paths_resolve_relative_to_the_opf() {
        assert_eq!(
            resolve_path("OEBPS", "Text/a%20b.xhtml#x"),
            "OEBPS/Text/a b.xhtml"
        );
        assert_eq!(resolve_path("OEBPS/Text", "../c.xhtml"), "OEBPS/c.xhtml");
        assert_eq!(resolve_path("", "c.xhtml"), "c.xhtml");
    }

    #[test]
    fn pdf_lines_become_paragraphs() {
        let pdf = "Der Leuchtturm stand seit über hundert Jahren auf der Klip-\n\
                   pe und trotzte jedem Sturm, der vom Meer heraufzog, seit\n\
                   Generationen.\n\
                   Ein neuer Absatz beginnt hier und läuft über die ganze\n\
                   Breite der Seite, bis er nach zwei Zeilen endet. Ende.\n\
                   \n\
                   12\n\
                   \u{0C}Nach der Seite geht es weiter mit [Anmerkung] Text.";
        assert_eq!(
            reflow_plain_text(pdf),
            "Der Leuchtturm stand seit über hundert Jahren auf der Klippe und trotzte jedem Sturm, der vom Meer heraufzog, seit Generationen.\n\n\
             Ein neuer Absatz beginnt hier und läuft über die ganze Breite der Seite, bis er nach zwei Zeilen endet. Ende.\n\n\
             Nach der Seite geht es weiter mit (Anmerkung) Text."
        );
    }

    #[test]
    fn hyphen_before_capital_or_dash_stays() {
        let pdf = "Ein Satz mit einem Gedankenstrich -\nund weiter geht es in derselben Zeile.\nNord-\nSüd-Achse bleibt.";
        let out = reflow_plain_text(pdf);
        assert!(out.contains("Gedankenstrich - und weiter"), "{out}");
        assert!(out.contains("Nord- Süd-Achse"), "{out}");
    }
}
