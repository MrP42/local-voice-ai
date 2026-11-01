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
use std::sync::LazyLock;

/// Einmal kompilierte Muster; ein großes Kapitel ruft sie zehntausendfach auf.
macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new($pat).expect("static regex"));
    };
}
re!(
    ATTR_RE,
    r#"([A-Za-z_][\w:.-]*)\s*=\s*(?:"([^"]*)"|'([^']*)')"#
);
re!(ROOTFILE_RE, r"(?is)<(?:\w+:)?rootfile\b[^>]*>");
re!(ITEM_RE, r"(?is)<(?:\w+:)?item\b[^>]*>");
re!(ITEMREF_RE, r"(?is)<(?:\w+:)?itemref\b[^>]*>");
re!(
    ENTITY_RE,
    r"&(#[xX][0-9a-fA-F]+|#[0-9]+|[A-Za-z][A-Za-z0-9]*);"
);
re!(
    HIDDEN_RE,
    r"(?is)<(head|script|style|svg|math|nav|aside)\b(?:[^>]*/>|.*?</(head|script|style|svg|math|nav|aside)\s*>)"
);
re!(
    COMMENT_RE,
    r"(?s)<!--.*?-->|<\?.*?\?>|<!\[CDATA\[.*?\]\]>|<!DOCTYPE[^>]*>"
);
re!(LIST_LINE_RE, r"^(?:[•\-–*]\s|\d{1,3}[.)]\s)");
re!(
    SPEAKER_LINE_RE,
    r"^(?:<[^<>]{1,40}>|[A-ZÄÖÜ][\w .'-]{0,30}:\s)"
);
re!(BODY_RE, r"(?is)<body\b[^>]*>(.*)</body\s*>");
re!(DC_TITLE_RE, r"(?is)<dc:title\b[^>]*>(.*?)</dc:title\s*>");
re!(
    DC_CREATOR_RE,
    r"(?is)<dc:creator\b([^>]*)>(.*?)</dc:creator\s*>"
);
re!(
    ROLE_META_RE,
    r#"(?is)<meta\b[^>]*\brefines\s*=\s*["']#([^"']+)["'][^>]*\bproperty\s*=\s*["']role["'][^>]*>\s*([^<\s]+)\s*</meta>"#
);
re!(
    ENC_ALGO_RE,
    r#"(?is)<(?:\w+:)?EncryptionMethod\b[^>]*\bAlgorithm\s*=\s*["']([^"']*)["']"#
);
re!(
    TAG_RE,
    r#"(?s)<(/?)([A-Za-z][\w:-]*)((?:[^>"']|"[^"]*"|'[^']*')*?)(/?)>"#
);

/// Höchstgröße eines einzelnen Eintrags im EPUB (Schutz vor Zip-Bomben).
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;

fn read_entry<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String, String> {
    // Manche EPUBs schreiben den Pfad im Manifest anders groß als im Archiv.
    let actual = if archive.index_for_name(name).is_some() {
        name.to_string()
    } else {
        archive
            .file_names()
            .find(|n| n.eq_ignore_ascii_case(name))
            .map(str::to_string)
            .ok_or_else(|| format!("EPUB: Eintrag „{name}“ fehlt"))?
    };
    let entry = archive
        .by_name(&actual)
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
    ATTR_RE
        .captures_iter(tag)
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
    reject_drm(archive)?;
    let container = read_entry(archive, "META-INF/container.xml")?;
    let opf_path = ROOTFILE_RE
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

    let mut manifest: HashMap<String, (String, String)> = HashMap::new();
    for m in ITEM_RE.find_iter(&opf) {
        let a = attributes(m.as_str());
        // Das EPUB3-Inhaltsverzeichnis ist kein Lesetext.
        if a.get("properties")
            .is_some_and(|p| p.split_whitespace().any(|x| x == "nav"))
        {
            continue;
        }
        if let (Some(id), Some(href)) = (a.get("id"), a.get("href")) {
            let media = a.get("media-type").cloned().unwrap_or_default();
            manifest.insert(id.clone(), (resolve_path(base_dir, href), media));
        }
    }
    let spine: Vec<String> = ITEMREF_RE
        .find_iter(&opf)
        .map(|m| attributes(m.as_str()))
        // linear="no": Anhänge, Pop-ups — nicht in der Lesereihenfolge.
        .filter(|a| a.get("linear").is_none_or(|l| l != "no"))
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
    Ok(with_title_page(&opf, text))
}

/// Klartext eines Metadatenfelds (Tags, Entities, Leerraum bereinigt).
fn metadata_text(raw: &str) -> String {
    let plain = TAG_RE.replace_all(raw, "");
    neutralize(&decode_entities(&plain))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Titel und Autor aus dem OPF voranstellen (lva#149) — als Überschrift, damit
/// Inhaltsverzeichnis und Seitentitel sie kennen. Nicht, wenn das Buch schon
/// mit dem Titel beginnt (eigene Titelseite).
fn with_title_page(opf: &str, text: String) -> String {
    let Some(title) = DC_TITLE_RE
        .captures(opf)
        .map(|c| metadata_text(&c[1]))
        .filter(|t| !t.is_empty())
    else {
        return text;
    };
    // Eigene Titelseite: eine der ersten Zeilen IST der Titel. Ein bloßes
    // Vorkommen genügt nicht — kurze Titel („1984“) stehen fast überall.
    let norm = |l: &str| {
        l.trim_start_matches(['#', '>', ' '])
            .trim_matches(['*', ' '])
            .to_lowercase()
    };
    let wanted = title.to_lowercase();
    if text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(3)
        .any(|l| norm(l) == wanted)
    {
        return text;
    }
    // Nur Autorinnen und Autoren, nicht Übersetzung oder Illustration
    // (EPUB 2: opf:role, EPUB 3: <meta refines property="role">).
    let other_roles: Vec<String> = ROLE_META_RE
        .captures_iter(opf)
        .filter(|c| !c[2].eq_ignore_ascii_case("aut"))
        .map(|c| c[1].to_string())
        .collect();
    let authors: Vec<String> = DC_CREATOR_RE
        .captures_iter(opf)
        .filter(|c| {
            let attrs = attributes(&format!("<x{}>", &c[1]));
            let role_ok = attrs
                .iter()
                .find(|(k, _)| k.ends_with("role"))
                .is_none_or(|(_, v)| v.eq_ignore_ascii_case("aut"));
            let id_ok = attrs
                .get("id")
                .is_none_or(|id| !other_roles.iter().any(|o| o == id));
            role_ok && id_ok
        })
        .map(|c| metadata_text(&c[2]))
        .filter(|a| !a.is_empty())
        .collect();
    if authors.is_empty() {
        format!("# {title}\n\n{text}")
    } else {
        format!("# {title}\n\nvon {}\n\n{text}", authors.join(", "))
    }
}

/// Kopiergeschützte EPUBs (Adobe ADEPT, Apple FairPlay, Readium LCP u. a.)
/// ablehnen, mit einer verständlichen Meldung statt Zeichensalat. Erlaubt
/// bleibt die Schriftverschleierung nach IDPF/Adobe — die betrifft nur Fonts.
fn reject_drm<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<(), String> {
    const DRM: &str = "Dieses E-Book ist kopiergeschützt (DRM) und kann nicht \
         vorgelesen werden. Bitte eine DRM-freie Ausgabe verwenden.";
    let has = |archive: &zip::ZipArchive<R>, name: &str| {
        archive.file_names().any(|n| n.eq_ignore_ascii_case(name))
    };
    if has(archive, "META-INF/rights.xml")
        || has(archive, "META-INF/sinf.xml")
        || has(archive, "META-INF/license.lcpl")
    {
        return Err(DRM.into());
    }
    if !has(archive, "META-INF/encryption.xml") {
        return Ok(());
    }
    let enc = read_entry(archive, "META-INF/encryption.xml")?;
    let font_only = ENC_ALGO_RE.captures_iter(&enc).all(|c| {
        let algo = c[1].trim();
        algo == "http://www.idpf.org/2008/embedding" || algo == "http://ns.adobe.com/pdf/enc#RC"
    });
    if font_only {
        Ok(())
    } else {
        Err(DRM.into())
    }
}

/// Benannte und numerische Entities auflösen (die üblichen in E-Books).
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    ENTITY_RE
        .replace_all(s, |c: &regex::Captures| {
            let name = &c[1];
            let ch = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X"))
            {
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
                    "aacute" => Some('á'),
                    "agrave" => Some('à'),
                    "acirc" => Some('â'),
                    "ecirc" => Some('ê'),
                    "iacute" => Some('í'),
                    "oacute" => Some('ó'),
                    "uacute" => Some('ú'),
                    "ccedil" => Some('ç'),
                    "ntilde" => Some('ñ'),
                    "oslash" => Some('ø'),
                    "aring" => Some('å'),
                    "aelig" => Some('æ'),
                    "middot" => Some('·'),
                    "bull" => Some('•'),
                    "euro" => Some('€'),
                    "sect" => Some('§'),
                    "deg" => Some('°'),
                    "times" => Some('×'),
                    "thinsp" | "ensp" | "emsp" => Some(' '),
                    "zwnj" | "zwj" => Some('\u{200B}'),
                    _ => None,
                }
            };
            // Unbekannte Entities nicht als „und aacute Semikolon“ vorlesen.
            ch.map_or_else(String::new, |ch| ch.to_string())
        })
        .into_owned()
}

/// Zeichen entschärfen, die im Lesedokument Marker wären, und Leerraum
/// vereinheitlichen.
fn neutralize(text: &str) -> String {
    text.chars()
        .filter(|&c| {
            !matches!(
                c,
                '\u{AD}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}'
            ) && (!c.is_control() || c.is_whitespace())
        })
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
            self.kind = BlockKind::Para;
        }
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
    let cleaned = COMMENT_RE.replace_all(xhtml, "");
    let cleaned = HIDDEN_RE.replace_all(&cleaned, "");
    let body = BODY_RE
        .captures(&cleaned)
        .map_or(cleaned.to_string(), |c| c[1].to_string());

    let mut b = Builder::new();
    let mut last = 0;
    for cap in TAG_RE.captures_iter(&body) {
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
///   üblichen Zeilen, beendet den Absatz; ebenso eine sehr kurze Zeile ohne
///   Satzzeichen vor einer groß beginnenden Zeile (Überschrift).
/// - Aufzählungen, `Name:`- und `<Name>`-Zeilen beginnen einen neuen Absatz.
/// - Trennstriche am Zeilenende vor Kleinbuchstaben werden aufgehoben, außer
///   vor „und/oder“ („Vor- und Nachteile“); vor Großbuchstaben bleibt der
///   Bindestrich ohne Leerzeichen („Rheinland-Pfalz“).
/// - Zeilen nur aus einer kurzen Zahl an einer Seitengrenze (Seitenzahlen)
///   entfallen.
///
/// Anders als beim EPUB bleiben `<…>` und `[…]` stehen: PDFs sind oft
/// eigene Skripte mit Sprechern und Tags.
pub fn reflow_plain_text(text: &str) -> String {
    let lines: Vec<(String, Option<u32>)> = text
        .replace('\u{0C}', "\n\n")
        .lines()
        .map(|l| (l.to_string(), None))
        .collect();
    reflow_lines(lines)
}

/// Seitenmarke im Seitentext: `{{S. 12}}`. Die Buchansicht zeigt sie als
/// Seitenzahl des Originals, die Synthese liest sie nicht vor.
pub fn page_mark(page: u32) -> String {
    format!("{{{{S. {page}}}}}")
}

/// Wie [`reflow_plain_text`], aber je Zeile optional die Seite des Originals,
/// die mit dieser Zeile beginnt; sie landet als [`page_mark`] im Absatz.
fn reflow_lines(raw: Vec<(String, Option<u32>)>) -> String {
    let (lines, starts): (Vec<String>, Vec<Option<u32>>) = raw
        .into_iter()
        .map(|(l, m)| (l.split_whitespace().collect::<Vec<_>>().join(" "), m))
        .unzip();
    let mut lengths: Vec<usize> = lines
        .iter()
        .map(|l| l.chars().count())
        .filter(|&n| n > 20)
        .collect();
    lengths.sort_unstable();
    let typical = lengths.get(lengths.len() * 3 / 4).copied().unwrap_or(0);

    let starts_block = |line: &str| LIST_LINE_RE.is_match(line) || SPEAKER_LINE_RE.is_match(line);
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    // Seitenanfänge im laufenden Absatz: (Byte-Position, Seite).
    let mut marks: Vec<(usize, u32)> = Vec::new();
    let mut pending: Option<u32> = None;
    let end_paragraph =
        |current: &mut String, marks: &mut Vec<(usize, u32)>, paragraphs: &mut Vec<String>| {
            let text = insert_page_marks(current, marks);
            if !text.trim().is_empty() {
                paragraphs.push(text.trim().to_string());
            }
            current.clear();
            marks.clear();
        };
    for (i, line) in lines.iter().enumerate() {
        if let Some(page) = starts[i] {
            pending = Some(page);
        }
        if line.is_empty() {
            end_paragraph(&mut current, &mut marks, &mut paragraphs);
            continue;
        }
        let at_gap = |j: Option<usize>| j.and_then(|j| lines.get(j)).is_none_or(String::is_empty);
        if line.len() <= 3
            && line.chars().all(|c| c.is_ascii_digit())
            && (at_gap(i.checked_sub(1)) || at_gap(Some(i + 1)))
        {
            continue;
        }
        if starts_block(line) {
            end_paragraph(&mut current, &mut marks, &mut paragraphs);
        }
        let first_word = line.split(' ').next().unwrap_or_default();
        if current.is_empty() {
            if let Some(page) = pending.take() {
                marks.push((0, page));
            }
            current.push_str(line);
        } else if current.ends_with('-')
            && !current.ends_with(" -")
            && current[..current.len() - 1]
                .chars()
                .next_back()
                .is_some_and(char::is_alphabetic)
        {
            let next_lower = line.chars().next().is_some_and(char::is_lowercase);
            if next_lower
                && !matches!(
                    first_word,
                    "und" | "oder" | "bzw." | "sowie" | "bis" | "als"
                )
            {
                current.pop();
            } else if next_lower {
                current.push(' ');
            }
            if let Some(page) = pending.take() {
                marks.push((current.len(), page));
            }
            current.push_str(line);
        } else {
            current.push(' ');
            if let Some(page) = pending.take() {
                marks.push((current.len(), page));
            }
            current.push_str(line);
        }
        let n = line.chars().count();
        let ends_sentence = line
            .trim_end_matches(['"', '“', '”', '«', '»', '’', '\'', ')'])
            .ends_with(['.', '!', '?', ':', '…']);
        let next_upper = lines
            .get(i + 1)
            .and_then(|l| l.chars().next())
            .is_some_and(char::is_uppercase);
        let short_heading =
            typical > 0 && n * 2 < typical && !line.ends_with([',', '-', ';']) && next_upper;
        if typical > 0 && ((ends_sentence && n * 10 < typical * 8) || short_heading) {
            end_paragraph(&mut current, &mut marks, &mut paragraphs);
        }
    }
    end_paragraph(&mut current, &mut marks, &mut paragraphs);
    paragraphs.join("\n\n")
}

/// Seitenmarken in einen Absatz setzen: vor das erste Wort der Seite, oder —
/// wenn die Seite mitten im Wort beginnt (getrenntes Wort über die
/// Seitengrenze) — hinter dieses Wort.
fn insert_page_marks(text: &str, marks: &[(usize, u32)]) -> String {
    let mut out = String::with_capacity(text.len() + marks.len() * 12);
    let mut from = 0;
    for &(at, page) in marks {
        let at = at.min(text.len());
        let mid_word = text[..at]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
            && text[at..].chars().next().is_some_and(char::is_alphanumeric);
        if mid_word {
            let end = text[at..]
                .find(char::is_whitespace)
                .map_or(text.len(), |e| at + e);
            out.push_str(&text[from..end]);
            out.push(' ');
            out.push_str(&page_mark(page));
            from = end;
        } else {
            out.push_str(&text[from..at]);
            out.push_str(&page_mark(page));
            out.push(' ');
            from = at;
        }
    }
    out.push_str(&text[from..]);
    out
}

/// Wie viele Zeilen oben und unten einer PDF-Seite höchstens als Kopf-/Fußzeile
/// in Frage kommen. Auf kurzen Seiten weniger (ein Viertel der Zeilen), damit
/// der Fließtext einer kurzen Seite nie ganz als „Rand“ gilt.
const RUNNING_LINES: usize = 3;

/// Schlüssel einer Kopf-/Fußzeile für den buchweiten Abgleich: Ziffern zählen
/// nicht („Seite 41 von 300“ und „Seite 42 von 300“ sind dieselbe Fußzeile),
/// Leerraum und Groß-/Kleinschreibung auch nicht.
fn running_key(line: &str) -> String {
    line.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect()
}

/// Nur eine Seitenzahl: „12“, „- 12 -“, „Seite 12“, „S. 12“, „12 / 300“.
/// Kein Punkt am Ende („1914.“ ist ein Satzende, „1.“ ein Listenpunkt).
fn is_page_number(line: &str) -> bool {
    let t = line.trim();
    let t = t
        .strip_prefix("Seite")
        .or_else(|| t.strip_prefix("S."))
        .or_else(|| t.strip_prefix("Page"))
        .unwrap_or(t)
        .trim();
    // „12 von 300“ / „12 of 300“ wie „12 / 300“.
    let t = &t.replace(" von ", " / ").replace(" of ", " / ");
    let digits = t.chars().filter(char::is_ascii_digit).count();
    (1..=8).contains(&digits)
        && t.chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, '-' | '–'))
        && !t.ends_with('.')
        && t.chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || matches!(c, '-' | '–' | '/' | '|'))
        && t.split(['/', '|'])
            .all(|part| part.chars().filter(char::is_ascii_digit).count() <= 4)
}

/// Rand einer Seite: `Some(true)` oben, `Some(false)` unten, `None` Fließtext.
fn page_zone(len: usize, i: usize) -> Option<bool> {
    let k = RUNNING_LINES.min(len / 4);
    if i < k {
        Some(true)
    } else if i + k >= len {
        Some(false)
    } else {
        None
    }
}

/// PDF-Seiten (je Seite der Text) zu einem Text zusammenführen und dabei
/// Kopf- und Fußzeilen entfernen sowie Seitenzahlen am Seitenrand:
///
/// - buchweit: eine Randzeile, die (ohne Ziffern verglichen) auf mindestens
///   einem Drittel der Seiten an derselben Stelle (oben/unten) steht — Buchtitel,
///   „Seite 12 von 300“; erst ab zehn Seiten, sonst wären „Abschnitt 1“,
///   „Abschnitt 2“ … oben auf jeder Seite eines kurzen Texts Kopfzeilen;
/// - kapitelweit: eine wortgleiche Randzeile auf mindestens drei Seiten mit
///   höchstens einer Seite Abstand — „Kapitel 5 · Die Klippe“ über die Seiten des
///   Kapitels. Wortgleich, damit „Abschnitt 1“, „Abschnitt 2“ … bleiben;
/// - Seitenzahl: nur die erste oder letzte Zeile einer Seite.
///
/// Danach [`reflow_plain_text`]. Die Seiten hängen mit einem einfachen Umbruch
/// aneinander — wie bisher bei `extract_text`, das auch keine Seitengrenze
/// markiert —, damit ein Absatz über eine Seitengrenze ein Absatz bleibt.
pub fn pdf_pages_to_text(pages: &[String]) -> String {
    use std::collections::{HashMap, HashSet};
    let page_lines: Vec<Vec<&str>> = pages
        .iter()
        .map(|p| p.lines().filter(|l| !l.trim().is_empty()).collect())
        .collect();
    let exact_key = |line: &str| line.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut masked: HashMap<(bool, String), usize> = HashMap::new();
    let mut exact: HashMap<(bool, String), Vec<usize>> = HashMap::new();
    for (p, lines) in page_lines.iter().enumerate() {
        let mut seen = HashSet::new();
        for (i, line) in lines.iter().enumerate() {
            let Some(top) = page_zone(lines.len(), i) else {
                continue;
            };
            if seen.insert((top, running_key(line))) {
                *masked.entry((top, running_key(line))).or_default() += 1;
            }
            let pages_of = exact.entry((top, exact_key(line))).or_default();
            if pages_of.last() != Some(&p) {
                pages_of.push(p);
            }
        }
    }
    let with_text = page_lines.iter().filter(|l| !l.is_empty()).count();
    let threshold = with_text.div_ceil(3).max(3);
    let chapter_run = |pages_of: &[usize]| {
        let mut run = 1;
        pages_of.windows(2).any(|w| {
            run = if w[1] - w[0] <= 2 { run + 1 } else { 1 };
            run >= 3
        })
    };
    let running = |top: bool, line: &str| {
        (with_text >= 10
            && masked
                .get(&(top, running_key(line)))
                .is_some_and(|&n| n >= threshold))
            || exact
                .get(&(top, exact_key(line)))
                .is_some_and(|p| chapter_run(p))
    };

    // Gedruckte Seitenzahl = Seitenindex + Versatz (Vorspann ohne Zahl,
    // Kapitelanfänge ohne Zahl): der häufigste Versatz der erkannten Zahlen.
    let mut offsets: HashMap<i64, usize> = HashMap::new();
    for (p, lines) in page_lines.iter().enumerate() {
        let printed = [lines.first(), lines.last()]
            .into_iter()
            .flatten()
            .find(|l| is_page_number(l))
            .and_then(|l| first_number(l));
        if let Some(n) = printed {
            *offsets.entry(n as i64 - p as i64).or_default() += 1;
        }
    }
    let offset = offsets
        .into_iter()
        .max_by_key(|&(o, n)| (n, -o.abs()))
        .map_or(1, |(o, _)| o);

    let mut out: Vec<(String, Option<u32>)> = Vec::new();
    for (p, lines) in page_lines.iter().enumerate() {
        // Ohne zweite Seite keine Marken: ein Einseiter braucht keine Seitenzahl.
        let mut mark = (pages.len() > 1)
            .then(|| u32::try_from(p as i64 + offset).ok())
            .flatten()
            .filter(|&n| n > 0);
        for (i, line) in lines.iter().enumerate() {
            let at_edge = i == 0 || i + 1 == lines.len();
            let drop = match page_zone(lines.len(), i) {
                Some(top) => running(top, line),
                None => false,
            } || (at_edge && is_page_number(line));
            if !drop {
                out.push((line.to_string(), mark.take()));
            }
        }
    }
    reflow_lines(out)
}

/// Erste Zahl einer Seitenzahl-Zeile („Seite 12 von 300“ → 12).
fn first_number(line: &str) -> Option<u32> {
    line.split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
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
            "# Der Leuchtturm\n\n\
             # Kapitel 1\n\n\
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
             Nach der Seite geht es weiter mit [Anmerkung] Text."
        );
    }

    #[test]
    fn hyphens_keep_compounds_and_dashes() {
        let pdf = "Ein Satz mit einem Gedankenstrich -\nund weiter geht es in derselben Zeile mit Vor-\nund Nachteilen aus Rheinland-\nPfalz und dem Nord-\nSüd-Gefälle bis hier.";
        let out = reflow_plain_text(pdf);
        assert!(out.contains("Gedankenstrich - und weiter"), "{out}");
        assert!(out.contains("Vor- und Nachteilen"), "{out}");
        assert!(out.contains("Rheinland-Pfalz"), "{out}");
        assert!(out.contains("Nord-Süd-Gefälle"), "{out}");
    }

    #[test]
    fn lists_headings_speakers_and_years_survive_the_reflow() {
        let pdf = "Kapitel 3\n\
                   Der Aufbruch begann früh am Morgen, als die Sonne noch tief über dem Meer stand und\n\
                   niemand an Bord ahnte, was kommen würde in den nächsten Stunden.\n\
                   • Erster Punkt der Liste mit etwas Text\n\
                   • Zweiter Punkt\n\
                   Anna: Hallo zusammen, wie geht es euch heute?\n\
                   <Ben> Gut, danke.\n\
                   \n\
                   1945\n\
                   \n\
                   Ein Jahr, das alles änderte, für alle Menschen in diesem kleinen Ort am Meer.";
        let out = reflow_plain_text(pdf);
        let paras: Vec<&str> = out.split("\n\n").collect();
        assert_eq!(paras[0], "Kapitel 3", "{out}");
        assert!(
            paras.contains(&"• Erster Punkt der Liste mit etwas Text"),
            "{out}"
        );
        assert!(paras.contains(&"• Zweiter Punkt"), "{out}");
        assert!(paras.iter().any(|p| p.starts_with("Anna: Hallo")), "{out}");
        assert!(paras.contains(&"<Ben> Gut, danke."), "{out}");
        assert!(paras.contains(&"1945"), "{out}");
    }

    #[test]
    fn xhtml_edge_cases() {
        let md = xhtml_to_markdown(
            "<body><p>Text<script src='a.js'/>mehr</p><p title=\"a>b\">Echt &aacute; &unbekannt; x\u{200B}y</p>\
             <ul><li><p>Punkt</p></li></ul></body>",
        );
        assert_eq!(md, "Textmehr\n\nEcht á x y\n\n- Punkt".replace("x y", "xy"));
    }

    fn simple_epub(
        opf_meta: &str,
        extra: &[(&str, &str)],
    ) -> zip::ZipArchive<std::io::Cursor<Vec<u8>>> {
        let opf = format!(
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">{opf_meta}</metadata>
  <manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#
        );
        let mut files: Vec<(&str, &str)> = vec![
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            (
                "OEBPS/c1.xhtml",
                "<html><body><h1>Erstes Kapitel</h1><p>Es war einmal.</p></body></html>",
            ),
        ];
        files.extend_from_slice(extra);
        files.push(("OEBPS/content.opf", &opf));
        epub(&files)
    }

    #[test]
    fn epub_title_and_authors_open_the_book() {
        let mut a = simple_epub(
            r#"<dc:title id="t">Die Leuchtturm&#173;wärterin</dc:title>
               <dc:creator opf:role="aut">Mara Beispiel</dc:creator>
               <dc:creator>Jonas &amp; Co</dc:creator>"#,
            &[],
        );
        let md = epub_to_markdown(&mut a).unwrap();
        assert!(
            md.starts_with(
                "# Die Leuchtturmwärterin\n\nvon Mara Beispiel, Jonas & Co\n\n# Erstes Kapitel"
            ),
            "{md}"
        );
        // Ohne Titel bleibt der Text unverändert; ein vorhandener Titel wird nicht doppelt gesetzt.
        let mut b = simple_epub("", &[]);
        assert!(epub_to_markdown(&mut b)
            .unwrap()
            .starts_with("# Erstes Kapitel"));
        let mut c = simple_epub("<dc:title>Erstes Kapitel</dc:title>", &[]);
        assert!(epub_to_markdown(&mut c)
            .unwrap()
            .starts_with("# Erstes Kapitel"));
        // Ein kurzer Titel, der nur im Text vorkommt, wird trotzdem gesetzt;
        // Übersetzung und Illustration sind keine Autoren.
        let mut d = simple_epub(
            r##"<dc:title>Es</dc:title>
               <dc:creator id="a1">Anna Autorin</dc:creator>
               <dc:creator id="t1">Tom Übersetzer</dc:creator>
               <meta refines="#a1" property="role" scheme="marc:relators">aut</meta>
               <meta refines="#t1" property="role" scheme="marc:relators">trl</meta>
               <dc:creator opf:role="ill">Ida Illustratorin</dc:creator>"##,
            &[],
        );
        let md = epub_to_markdown(&mut d).unwrap();
        assert!(
            md.starts_with("# Es\n\nvon Anna Autorin\n\n# Erstes Kapitel"),
            "{md}"
        );
    }

    #[test]
    fn drm_is_rejected_but_font_obfuscation_is_fine() {
        let fonts = r#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container"
  xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
  <enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
  <enc:CipherData><enc:CipherReference URI="OEBPS/font.otf"/></enc:CipherData></enc:EncryptedData>
</encryption>"#;
        let mut ok = simple_epub("", &[("META-INF/encryption.xml", fonts)]);
        assert!(epub_to_markdown(&mut ok).is_ok());

        let aes = r#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#">
  <EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/>
  <CipherData><CipherReference URI="OEBPS/c1.xhtml"/></CipherData></EncryptedData>
</encryption>"#;
        let mut drm = simple_epub("", &[("META-INF/encryption.xml", aes)]);
        let err = epub_to_markdown(&mut drm).unwrap_err();
        assert!(err.contains("kopiergeschützt"), "{err}");

        let mut adept = simple_epub("", &[("META-INF/rights.xml", "<rights/>")]);
        assert!(epub_to_markdown(&mut adept).unwrap_err().contains("DRM"));
    }
    #[test]
    fn pdf_running_headers_and_page_numbers_are_dropped() {
        // Fließtext ist von Seite zu Seite verschieden (Wörter, nicht nur Zahlen).
        let worte = ["Möwen", "Wellen", "Laterne", "Treppe", "Nebel", "Stürme"];
        let pages: Vec<String> = (1..=6usize)
            .map(|n| {
                let mut page = format!("Der Leuchtturm\nKapitel {} · Die Klippe\n", (n - 1) / 3 + 1);
                for z in 1..=10 {
                    let w = worte[(n + z) % worte.len()];
                    page.push_str(&format!(
                        "{w} im Turm, Zeile {z}: Die Stunden vergingen zäh, und Mara lauschte auf jedes\n"
                    ));
                }
                let w = worte[n % worte.len()];
                page.push_str(&format!(
                    "Geräusch, das vom Meer heraufkam, {w} am Ende.\nDanach kam {w}, und der Satz läuft über die Seitengrenze in\n- {n} -\n"
                ));
                page
            })
            .collect();
        let out = pdf_pages_to_text(&pages);
        assert!(!out.contains("Der Leuchtturm"), "{out}");
        assert!(!out.contains("Kapitel"), "{out}");
        assert!(!out.contains("- 3 -"), "{out}");
        assert!(out.contains("vom Meer heraufkam, Nebel am Ende."), "{out}");
        // Ein Satz über die Seitengrenze bleibt zusammen.
        assert!(
            out.contains("läuft über die Seitengrenze in {{S. 4}} Stürme im Turm, Zeile 1"),
            "{out}"
        );
    }

    #[test]
    fn pdf_with_few_pages_keeps_repeated_lines() {
        let pages: Vec<String> = (1..=3)
            .map(|n| format!("Einleitung\nText der Seite {n} ist hier.\n"))
            .collect();
        let out = pdf_pages_to_text(&pages);
        assert_eq!(out.matches("Einleitung").count(), 3, "{out}");
    }

    #[test]
    fn page_number_shapes() {
        for yes in [
            "12",
            "- 12 -",
            "Seite 12",
            "S. 12",
            "12 / 300",
            "– 7 –",
            "Seite 3 von 6",
            "Page 4 of 9",
        ] {
            assert!(is_page_number(yes), "{yes}");
        }
        for no in [
            "12 Stufen",
            "Kapitel 3",
            "",
            "2026-11-01 war ein Sonntag",
            "1914.",
            "1.",
            "1 000 000",
            "0170 12345",
        ] {
            assert!(!is_page_number(no), "{no}");
        }
    }

    #[test]
    fn pdf_short_documents_keep_numbered_headings() {
        // Sechs kurze Seiten, oben je „Abschnitt n“: das sind Überschriften,
        // keine Kopfzeile, obwohl sie ohne Ziffern gleich aussehen.
        let pages: Vec<String> = (1..=6)
            .map(|n| {
                format!(
                    "Abschnitt {n}\nDer Wind drehte über der Klippe, Teil {n} beginnt hier.\nMara stieg die Treppe hinauf und sah nach Westen.\nDas Licht fiel schräg durch das Fenster der Laterne.\nEnde der Seite {n} kommt jetzt.\n"
                )
            })
            .collect();
        let out = pdf_pages_to_text(&pages);
        for n in 1..=6 {
            assert!(out.contains(&format!("Abschnitt {n}")), "{out}");
        }
    }

    #[test]
    fn pdf_chapter_running_header_is_dropped_in_long_books() {
        // 30 Seiten, Kapitelkopf nur auf den Seiten seines Kapitels (je 5).
        let pages: Vec<String> = (0..30)
            .map(|p| {
                let mut page = format!("Kapitel {} · Die Klippe\n", p / 5 + 1);
                for z in 0..12 {
                    page.push_str(&format!(
                        "Satz {z} auf Seite {p}: Der Wind {} um den Turm, und Mara lauschte lange.\n",
                        ["heulte", "pfiff", "strich", "fuhr"][(p + z) % 4]
                    ));
                }
                page.push_str(&format!("{}\n", p + 1));
                page
            })
            .collect();
        let out = pdf_pages_to_text(&pages);
        assert!(!out.contains("Die Klippe"), "{out}");
        assert!(out.contains("Satz 0 auf Seite 7"), "{out}");
    }
    #[test]
    fn pdf_page_marks_follow_printed_numbers_and_split_words() {
        // Vorspann: zwei Seiten ohne Zahl, dann Seite 1, 2, 3 … gedruckt unten.
        let line = "Der Wind strich um den Turm, und Mara lauschte lange auf das Meer";
        let pages: Vec<String> = (0..6)
            .map(|p| {
                let mut page = String::new();
                for z in 0..6 {
                    page.push_str(&format!("{line} {p}{z} und weiter\n"));
                }
                if p == 2 {
                    page.push_str("und dann kam die Stun-\n");
                }
                if p >= 2 {
                    page.push_str(&format!("{}\n", p - 1));
                }
                page
            })
            .map(|mut page| {
                if page.starts_with(line) && page.contains(" 30 ") {
                    page = format!("de der Wahrheit am Ende\n{page}");
                }
                page
            })
            .collect();
        let out = pdf_pages_to_text(&pages);
        // Versatz aus den gedruckten Zahlen: Seite mit Index 3 ist „2“.
        assert!(out.contains("Stunde {{S. 2}} der Wahrheit"), "{out}");
        assert!(!out.contains("{{S. 0}}"), "{out}");
        assert!(out.starts_with(line), "{out}");
    }

    #[test]
    fn reflow_without_pages_has_no_marks() {
        assert!(!reflow_plain_text("Eins.\n\nZwei.").contains("{{"));
    }
}
