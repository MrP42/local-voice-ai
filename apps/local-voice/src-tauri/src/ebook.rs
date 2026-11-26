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

use crate::text_aufbereiten::re;
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
re!(
    NAV_TOC_RE,
    r#"(?is)<nav\b[^>]*\btype\s*=\s*["']toc["'][^>]*>"#
);
re!(NAV_PART_RE, r"(?is)</?ol\b[^>]*>|<a\b[^>]*>.*?</a\s*>");
re!(NAV_LINK_RE, r"(?is)^<a\b([^>]*)>(.*?)</a\s*>$");
re!(
    NCX_PART_RE,
    r"(?is)</?navPoint\b[^>]*>|<text\b[^>]*>.*?</text\s*>|<content\b[^>]*>"
);
re!(
    SPINE_TOC_RE,
    r#"(?is)<spine\b[^>]*\btoc\s*=\s*["']([^"']+)["']"#
);
re!(BODY_RE, r"(?is)<body\b[^>]*>(.*)</body\s*>");
re!(DC_TITLE_RE, r"(?is)<dc:title\b[^>]*>(.*?)</dc:title\s*>");
re!(DC_DESCRIPTION_RE, r"(?is)<dc:description\b[^>]*>(.*?)</dc:description\s*>");
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
    let mut nav_path: Option<String> = None;
    for m in ITEM_RE.find_iter(&opf) {
        let a = attributes(m.as_str());
        // Das EPUB3-Inhaltsverzeichnis ist kein Lesetext.
        if a.get("properties")
            .is_some_and(|p| p.split_whitespace().any(|x| x == "nav"))
        {
            nav_path = a.get("href").map(|h| resolve_path(base_dir, h));
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

    // Inhaltsverzeichnis: EPUB-3-nav, sonst die NCX aus `<spine toc>`.
    let mut toc: Vec<TocEntry> = nav_path
        .and_then(|p| {
            let dir = p.rsplit_once('/').map_or("", |(d, _)| d).to_string();
            read_entry(archive, &p).ok().map(|x| nav_toc(&x, &dir))
        })
        .unwrap_or_default();
    if toc.is_empty() {
        let ncx = SPINE_TOC_RE
            .captures(&opf)
            .and_then(|c| manifest.get(&c[1]).map(|(p, _)| p.clone()));
        if let Some(p) = ncx {
            let dir = p.rsplit_once('/').map_or("", |(d, _)| d).to_string();
            if let Ok(x) = read_entry(archive, &p) {
                toc = ncx_toc(&x, &dir);
            }
        }
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
        let entries: Vec<&TocEntry> = toc
            .iter()
            .filter(|e| e.path.eq_ignore_ascii_case(path))
            .collect();
        let md = promote_chapter_lines(&apply_toc(&xhtml_to_markdown(&xhtml), &entries));
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

/// Ein Eintrag im Inhaltsverzeichnis eines EPUBs (nav oder NCX).
#[derive(Debug, Clone, PartialEq)]
struct TocEntry {
    /// Tiefe, 0 = oberste Ebene.
    depth: usize,
    title: String,
    /// Datei im Archiv (aufgelöst) und Sprungmarke.
    path: String,
    fragment: Option<String>,
}

/// Inhaltsverzeichnis aus einem EPUB-3-`nav` (`<nav epub:type="toc">`):
/// Verschachtelung über `<ol>`, Einträge sind die Links darin.
fn nav_toc(xhtml: &str, base_dir: &str) -> Vec<TocEntry> {
    let Some(nav) = NAV_TOC_RE.find(xhtml) else {
        return Vec::new();
    };
    let body = &xhtml[nav.end()..];
    let body = &body[..body.find("</nav").unwrap_or(body.len())];
    let mut depth = 0usize;
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(m) = NAV_PART_RE.find_at(body, pos) {
        pos = m.end();
        let tag = m.as_str();
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<ol") {
            depth += 1;
        } else if lower.starts_with("</ol") {
            depth = depth.saturating_sub(1);
        } else if let Some(c) = NAV_LINK_RE.captures(tag) {
            let href = attributes(&c[1]).get("href").cloned().unwrap_or_default();
            let title = metadata_text(&c[2]);
            push_toc(&mut out, depth.saturating_sub(1), title, &href, base_dir);
        }
    }
    out
}

/// Inhaltsverzeichnis aus einer EPUB-2-NCX-Datei (`navPoint`, verschachtelt).
fn ncx_toc(ncx: &str, base_dir: &str) -> Vec<TocEntry> {
    let mut depth = 0usize;
    let mut out = Vec::new();
    let mut title: Option<String> = None;
    for m in NCX_PART_RE.find_iter(ncx) {
        let tag = m.as_str();
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<navpoint") {
            depth += 1;
            title = None;
        } else if lower.starts_with("</navpoint") {
            depth = depth.saturating_sub(1);
        } else if lower.starts_with("<text") {
            if title.is_none() {
                title = Some(metadata_text(&TAG_RE.replace_all(tag, "")));
            }
        } else if lower.starts_with("<content") {
            let href = attributes(tag).get("src").cloned().unwrap_or_default();
            if let Some(t) = title.take() {
                push_toc(&mut out, depth.saturating_sub(1), t, &href, base_dir);
            }
        }
    }
    out
}

fn push_toc(out: &mut Vec<TocEntry>, depth: usize, title: String, href: &str, base_dir: &str) {
    if title.is_empty() || href.is_empty() || href.contains("://") {
        return;
    }
    let (file, fragment) = match href.split_once('#') {
        Some((f, frag)) => (f, Some(percent_decode(frag))),
        None => (href, None),
    };
    out.push(TocEntry {
        depth,
        title,
        path: resolve_path(base_dir, file),
        fragment: fragment.filter(|f| !f.is_empty()),
    });
}

/// Text eines Markdown-Blocks ohne Überschriften- und Betonungszeichen.
fn block_plain(block: &str) -> &str {
    block
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_matches(['*', '_', ' '])
}

/// Überschriften eines Kapitels nach dem Inhaltsverzeichnis setzen: ein
/// Absatz, der wörtlich ein Verzeichnistitel ist, wird zur Überschrift
/// (`##`, darunterliegende Ebenen `###`). Gesucht wird in Reihenfolge der
/// Einträge; steht ein Eintrag ohne Sprungmarke für die ganze Datei und
/// beginnt sie ohne Überschrift, kommt der Titel davor. Sonst bleibt der
/// Text, wie er ist — lieber keine Überschrift als eine falsche.
fn apply_toc(md: &str, entries: &[&TocEntry]) -> String {
    if entries.is_empty() {
        return md.to_string();
    }
    let mut blocks: Vec<String> = md.split("\n\n").map(str::to_string).collect();
    let mut cursor = 0usize;
    let mut prepend: Option<String> = None;
    for e in entries {
        let level = "#".repeat((e.depth + 2).min(3));
        let found = (cursor..blocks.len()).find(|&i| {
            let b = &blocks[i];
            b.chars().count() <= 200 && title_match(block_plain(b), &e.title) == Some(true)
        });
        match found {
            Some(i) => {
                if !blocks[i].trim_start().starts_with('#') {
                    blocks[i] = format!("{level} {}", block_plain(&blocks[i]));
                }
                cursor = i + 1;
            }
            None if e.fragment.is_none() && cursor == 0 && prepend.is_none() => {
                let starts_with_heading = blocks
                    .iter()
                    .find(|b| !b.trim().is_empty())
                    .is_some_and(|b| b.trim_start().starts_with('#'));
                if !starts_with_heading {
                    prepend = Some(format!("{level} {}", e.title));
                }
            }
            None => {}
        }
    }
    let text = blocks.join("\n\n");
    match prepend {
        Some(h) => format!("{h}\n\n{text}"),
        None => text,
    }
}

/// Kapitelzeilen als Überschrift: ein Absatz, der nur „Erstes Kapitel“,
/// „Kapitel 3“, „Chapter IV“, „Prolog“ … ist (auch fett), wird `##`. Folgt
/// ein kurzer Titel ohne Satzzeichen („Der Brief“), gehört er dazu.
fn promote_chapter_lines(md: &str) -> String {
    let blocks: Vec<&str> = md.split("\n\n").collect();
    let mut out: Vec<String> = Vec::with_capacity(blocks.len());
    let mut i = 0;
    while i < blocks.len() {
        let b = blocks[i];
        let plain = block_plain(b).trim_end_matches('.');
        let is_head = !b.trim_start().starts_with('#')
            && !plain.contains('\n')
            && (CHAPTER_HEAD_RE.is_match(plain) || SPECIAL_HEAD_RE.is_match(plain));
        if !is_head {
            out.push(b.to_string());
            i += 1;
            continue;
        }
        let next = blocks.get(i + 1).map(|n| block_plain(n));
        // „Vorwort“ bekommt keinen Titel angehängt (sonst erkennt der Lesestart
        // es nicht mehr), und ein Satz mit Ausruf oder Zitat ist kein Titel.
        let special = SPECIAL_HEAD_RE.is_match(plain);
        match next {
            Some(t)
                if !special
                    && !blocks[i + 1].trim_start().starts_with('#')
                    && !t.contains('\n')
                    && !t.ends_with(['!', '?', '…', '"', '“', '”', '»', '«'])
                    && looks_like_chapter_title(t, 0) =>
            {
                out.push(format!("## {plain}: {t}"));
                i += 2;
            }
            _ => {
                out.push(format!("## {plain}"));
                i += 1;
            }
        }
    }
    out.join("\n\n")
}

/// Klartext eines Metadatenfelds (Tags, Entities, Leerraum bereinigt).
pub(crate) fn opf_description(opf: &str) -> Option<String> {
    let raw = DC_DESCRIPTION_RE.captures(opf)?;
    let raw = raw[1].trim();
    let raw = raw
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
        .unwrap_or(raw);
    let plain = strip_description_tags(raw);
    let decoded = decode_entities(&decode_entities(&plain));
    let plain = strip_description_tags(&decoded);
    let text = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

/// Ein Durchlauf, begrenzter Tagname; dieselbe Bereinigung wie `BookText` am iPhone.
fn strip_description_tags(raw: &str) -> String {
    let mut out = String::new();
    let mut name = String::new();
    let mut skipped: Option<String> = None;
    let (mut in_tag, mut closing, mut name_done) = (false, false, false);
    let mut quote: Option<char> = None;
    for c in raw.chars() {
        if in_tag && quote.is_some() {
            if quote == Some(c) { quote = None; }
            continue;
        }
        if c == '<' {
            in_tag = true; name.clear(); closing = false; name_done = false;
            if skipped.is_none() { out.push(' '); }
        } else if !in_tag {
            if skipped.is_none() { out.push(c); }
        } else if c == '>' {
            if let Some(skip) = &skipped {
                if closing && &name == skip { skipped = None; }
            } else if !closing && (name == "script" || name == "style") { skipped = Some(name.clone()); }
            in_tag = false;
        } else if c == '\"' || c == '\'' {
            quote = Some(c); name_done = true;
        } else if !name_done {
            if c == '/' && name.is_empty() { closing = true; }
            else if c.is_ascii_alphabetic() {
                if name.len() < 16 { name.push(c.to_ascii_lowercase()); }
                else { name_done = true; }
            } else { name_done = true; }
        }
    }
    out
}

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

pub use crate::text_aufbereiten::struktur::pdf_book_to_text;
#[cfg(test)]
use crate::text_aufbereiten::struktur::*;
use crate::text_aufbereiten::struktur::{
    looks_like_chapter_title, title_match, CHAPTER_HEAD_RE, SPECIAL_HEAD_RE,
};

#[cfg(test)]
mod tests {
    #[test]
    fn descriptions_drop_scripts_and_entity_generated_tags() {
        for raw in ["A<script>secret</script><STYLE>hidden</STYLE>B",
                    "A&lt;script&gt;secret&lt;/script&gt;&lt;style&gt;hidden&lt;/style&gt;B",
                    "A&amp;lt;script&amp;gt;secret&amp;lt;/script&amp;gt;B",
                    "A<b title='>'>B</b>C"] {
            let expected = if raw.contains("title") { "A B C" } else { "A B" };
            assert_eq!(opf_description(&format!("<dc:description>{raw}</dc:description>")).as_deref(), Some(expected));
        }
        let raw = format!("<dc:description>A{}</dc:description>", "<".repeat(100_000));
        assert_eq!(opf_description(&raw).as_deref(), Some("A"));
    }

    #[test]
    fn descriptions_decode_markup_entities_and_cdata() {
        for raw in ["&lt;p&gt;A &amp;amp; B&lt;/p&gt;&lt;p&gt;&#xE4; &#169;&lt;/p&gt;",
                    "<![CDATA[<p>A &amp; B</p><p>ä &#169;</p>]]>",
                    "<p>A &amp; B</p><p>ä &#169;</p>"] {
            assert_eq!(opf_description(&format!("<dc:description>{raw}</dc:description>")).as_deref(), Some("A & B ä ©"));
        }
        assert_eq!(opf_description("<dc:description>  </dc:description>"), None);
        assert_eq!(opf_description("<metadata/>"), None);
    }

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
        // Der Kapitel-Lauftitel steht nur noch einmal je Kapitel, als Überschrift.
        assert_eq!(out.matches("Kapitel").count(), 2, "{out}");
        assert!(out.contains("## Kapitel 2 · Die Klippe"), "{out}");
        assert!(!out.contains("- 3 -"), "{out}");
        assert!(out.contains("vom Meer heraufkam, Nebel am Ende."), "{out}");
        // Ein Satz über die Seitengrenze bleibt zusammen (nicht an einem
        // Kapitelanfang: dort beginnt ein neuer Absatz unter der Überschrift).
        assert!(
            out.contains("läuft über die Seitengrenze in {{S. 3}} Nebel im Turm, Zeile 1"),
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
        // Der Lauftitel steht nicht mehr über jeder Seite, sondern einmal je
        // Kapitel als Überschrift.
        assert_eq!(out.matches("Die Klippe").count(), 6, "{out}");
        assert!(out.contains("## Kapitel 3 · Die Klippe"), "{out}");
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

    /// Eine Buchseite mit Fließtext; `head` steht als eigene Zeilen oben.
    fn book_page(p: usize, head: &[&str], top: Option<&str>) -> String {
        let mut page = String::new();
        if let Some(t) = top {
            page.push_str(&format!("{t}\n"));
        }
        for h in head {
            page.push_str(&format!("{h}\n"));
        }
        for z in 0..10 {
            page.push_str(&format!(
                "Satz {z} auf Seite {p}: Der Wind {} um den Turm {}, und Mara lauschte lange.\n",
                ["heulte", "pfiff", "strich", "fuhr"][(p + z) % 4],
                // Jede Zeile einmalig, auch ohne Ziffern verglichen.
                (p * 10 + z)
                    .to_string()
                    .chars()
                    .map(|d| (b'a' + d as u8 - b'0') as char)
                    .collect::<String>()
            ));
        }
        page.push_str(&format!("{}\n", p + 1));
        page
    }

    #[test]
    fn pdf_chapter_lines_at_page_top_become_headings() {
        let heads: [(usize, &[&str]); 4] = [
            (0, &["Prolog"]),
            (3, &["Kapitel 3", "Die Klippe"]),
            (6, &["Teil II"]),
            (9, &["Chapter 4: The Storm"]),
        ];
        let pages: Vec<String> = (0..12)
            .map(|p| {
                let head = heads.iter().find(|h| h.0 == p).map_or(&[][..], |h| h.1);
                let mut page = book_page(p, head, None);
                if p == 5 {
                    // Verweis mitten im Text: keine Überschrift.
                    page = page.replace("Satz 4 auf", "Kapitel 3\nSatz 4 auf");
                }
                page
            })
            .collect();
        let out = pdf_pages_to_text(&pages);
        assert!(out.starts_with("## Prolog\n\n"), "{out}");
        assert!(out.contains("\n\n## Kapitel 3: Die Klippe\n\n"), "{out}");
        assert!(out.contains("\n\n## Teil II\n\n"), "{out}");
        assert!(out.contains("\n\n## Chapter 4: The Storm\n\n"), "{out}");
        assert_eq!(out.matches("## ").count(), 4, "{out}");
        // Die Seitenmarke steht am Text unter der Überschrift, nicht darin.
        assert!(
            out.contains("## Kapitel 3: Die Klippe\n\nSatz 0 auf Seite 3: {{S. 4}}"),
            "{out}"
        );
        assert!(!out.contains("## {{S."), "{out}");
    }

    #[test]
    fn pdf_a_short_sentence_below_a_chapter_line_is_no_title() {
        // Zweite Zeile ist ein Satz mit Punkt: nur „Kapitel 2“ wird Überschrift.
        let pages: Vec<String> = (0..4)
            .map(|p| {
                let head: &[&str] = if p == 2 {
                    &["Kapitel 2", "Es regnete."]
                } else {
                    &[]
                };
                book_page(p, head, None)
            })
            .collect();
        let out = pdf_pages_to_text(&pages);
        assert!(out.contains("## Kapitel 2\n\n"), "{out}");
        assert!(!out.contains("Kapitel 2:"), "{out}");
    }

    #[test]
    fn pdf_changing_running_titles_become_headings_without_a_chapter_line() {
        // 24 Seiten, drei Kapitel à 8 Seiten mit eigenem Lauftitel, dazu der
        // Buchtitel auf jeder zweiten Seite (steht auf der halben Länge: kein Kapitel).
        let titles = ["Die Klippe", "Der Sturm", "Die Rückkehr"];
        let pages: Vec<String> = (0..24)
            .map(|p| {
                let top = if p % 2 == 0 {
                    titles[p / 8].to_string()
                } else {
                    "Der Leuchtturm".to_string()
                };
                book_page(p, &[], Some(&top))
            })
            .collect();
        let out = pdf_pages_to_text(&pages);
        for t in titles {
            assert_eq!(out.matches(&format!("## {t}")).count(), 1, "{t}: {out}");
        }
        assert!(!out.contains("Leuchtturm"), "{out}");
        assert_eq!(out.matches("## ").count(), 3, "{out}");
        // Erste Überschrift steht vor dem Text der ersten Seite.
        assert!(
            out.starts_with("## Die Klippe\n\nSatz 0 auf Seite 0: {{S. 1}}"),
            "{out}"
        );
    }

    #[test]
    fn pdf_alternating_running_titles_make_no_headings() {
        // Links Kapitel A, rechts Kapitel B im Seitentakt: ein Wechsel pro Seite
        // ist kein Kapitelwechsel.
        let pages: Vec<String> = (0..12)
            .map(|p| book_page(p, &[], Some(if p % 2 == 0 { "Mara" } else { "Wind" })))
            .collect();
        let out = pdf_pages_to_text(&pages);
        assert!(!out.contains("## "), "{out}");
    }

    #[test]
    fn pdf_without_chapter_hints_has_no_headings() {
        let pages: Vec<String> = (0..6).map(|p| book_page(p, &[], None)).collect();
        assert!(!pdf_pages_to_text(&pages).contains("## "));
    }

    #[test]
    fn reflow_without_pages_has_no_marks() {
        assert!(!reflow_plain_text("Eins.\n\nZwei.").contains("{{"));
    }
    #[test]
    fn page_marks_stay_behind_list_and_speaker_prefixes() {
        let out = reflow_lines(vec![
            ("- Erster Punkt der Liste.".into(), Some(5)),
            ("".into(), None),
            ("Mara: Hallo, wer ist da?".into(), Some(6)),
            ("".into(), None),
            ("Der Wind kam aus Süd-".into(), None),
            ("Ost, wie jeden Abend.".into(), Some(7)),
        ]);
        assert!(out.contains("- {{S. 5}} Erster"), "{out}");
        assert!(out.contains("Mara: {{S. 6}} Hallo"), "{out}");
        assert!(out.contains("Süd-Ost, {{S. 7}}"), "{out}");
    }

    #[test]
    fn page_marks_never_slice_backwards() {
        let out = insert_page_marks("Stun-de,x weiter", &[(5, 1), (4, 2)]);
        assert!(
            out.contains("{{S. 1}}") && out.contains("{{S. 2}}"),
            "{out}"
        );
    }

    // ---- Buchimport: Kopfzeilen, Überschriften, Absätze (lva#149) ----

    fn sized(pages: &[&[(&str, f32)]]) -> crate::pdf_text::PdfBook {
        crate::pdf_text::PdfBook {
            pages: pages
                .iter()
                .map(|p| {
                    p.iter()
                        .map(|(t, s)| crate::pdf_text::PdfLine {
                            text: t.to_string(),
                            size: *s,
                            gap: false,
                        })
                        .collect()
                })
                .collect(),
            outline: Vec::new(),
        }
    }

    #[test]
    fn short_pdf_drops_header_with_running_page_number() {
        // Wie die Kurzgeschichte im Screenshot: acht Seiten, Kopfzeile mit
        // mitlaufender Seitenzahl, Titel direkt darunter.
        let pages: Vec<String> = (1..=8)
            .map(|n| {
                let title = if n == 1 { "Der längste Dezember\n" } else { "" };
                format!(
                    "beispiel.example · kostenlose Geschichten zum Vorlesen · Seite {n}\n{title}\
                     Mattea zählt die Tage bis zum Abend, an dem ihr Vater wieder nach Hause kommt.\n\
                     Am Fenster liegt Schnee, und im Flur stehen schon die Stiefel bereit für morgen.\n"
                )
            })
            .collect();
        let out = pdf_pages_to_text(&pages);
        assert!(!out.contains("kostenlose Geschichten"), "{out}");
        assert!(out.contains("Der längste Dezember"), "{out}");
    }

    #[test]
    fn numbered_line_inside_a_sentence_is_no_list() {
        let out = reflow_plain_text(
            "Es liegt daran, dass ihr Vater seit August in einer anderen Stadt arbeitet und erst am\n\
             23. kommt. Die dreiundzwanzig Tage bis dahin bestehen aus Schule und Schnee.\n\
             Folgendes ist zu tun:\n\
             1. Koffer packen und den Zug am Morgen nehmen, bevor es dunkel wird.\n\
             2. Anrufen.",
        );
        assert!(out.contains("und erst am 23. kommt."), "{out}");
        assert!(out.contains("\n\n1. Koffer"), "{out}");
        assert!(out.contains("\n\n2. Anrufen"), "{out}");
    }

    #[test]
    fn letter_spaced_page_numbers_are_removed() {
        assert_eq!(despace("S E I T E   12"), "SEITE 12");
        assert_eq!(despace("K A P I T E L 3"), "KAPITEL 3");
        assert_eq!(despace("Ein Satz mit a und b"), "Ein Satz mit a und b");
        assert!(is_page_number("SEITE 12"));
        let pages: Vec<String> = (1..=4)
            .map(|n| {
                format!(
                    "Der Wind strich um den Turm, und Mara lauschte lange auf das Meer {n}.\n\
                     Sie zählte die Wellen, bis der Morgen kam und das Licht über die Klippe fiel.\n\
                     S E I T E   {n}\n"
                )
            })
            .collect();
        let out = pdf_pages_to_text(&pages);
        assert!(!out.contains("SEITE"), "{out}");
    }

    #[test]
    fn toc_entries_become_headings_in_order() {
        let line = "Es war einmal ein König, der hatte eine Tochter, die war so schön, dass";
        let mut pages: Vec<String> = vec![
            "INHALT\n• KHM 1. Der Froschkönig oder der eiserne Heinrich\n\
             • KHM 2. Katze und Maus in Gesellschaft\n• KHM 3. Marienkind\n\
             • KHM 4. Märchen von einem, der auszog das\nFürchten zu lernen\n• KHM 5. Der Wolf"
                .into(),
        ];
        for (n, head) in [
            "KHM 1. DER FROSCHKÖNIG ODER DER EISERNE HEINRICH*",
            "KHM 2. KATZE UND MAUS IN GESELLSCHAFT",
            "KHM 3. MARIENKIND",
            "KHM 4. MÄRCHEN VOM EINEM, DER AUSZOG",
            "KHM 5. DER WOLF",
        ]
        .iter()
        .enumerate()
        {
            pages.push(format!(
                "{line}\n{head}\n{line} die Sonne sich wunderte, so oft sie schien.\n{line} {n}.\n"
            ));
        }
        let out = pdf_pages_to_text(&pages);
        for h in [
            "## KHM 1. Der Froschkönig oder der eiserne Heinrich",
            "## KHM 2. Katze und Maus in Gesellschaft",
            "## KHM 3. Marienkind",
            "## KHM 4. Märchen von einem, der auszog das Fürchten zu lernen",
            "## KHM 5. Der Wolf",
        ] {
            assert!(out.contains(&format!("\n{h}\n")), "{h}: {out}");
        }
        // Im Verzeichnis selbst bleibt alles Text.
        assert!(out.contains("• KHM 3. Marienkind"), "{out}");
    }

    #[test]
    fn larger_font_lines_become_headings() {
        let body = "Der Wind strich um den Turm, und Mara lauschte lange auf das Meer.";
        let page1: Vec<(&str, f32)> = vec![
            ("Der Leuchtturm", 24.0),
            ("Eine Geschichte", 11.0),
            ("Die Ankunft", 16.0),
            (body, 11.0),
            (body, 11.0),
        ];
        let page2: Vec<(&str, f32)> = vec![(body, 11.0), ("Der Sturm", 16.0), (body, 11.0)];
        let page3: Vec<(&str, f32)> = vec![(body, 11.0), (body, 11.0), (body, 11.0)];
        let out = pdf_book_to_text(&sized(&[&page1, &page2, &page3]));
        assert!(out.starts_with("# Der Leuchtturm\n"), "{out}");
        assert!(out.contains("\n## Die Ankunft\n"), "{out}");
        assert!(out.contains("\n## Der Sturm\n"), "{out}");
        assert!(!out.contains("# Eine Geschichte"), "{out}");
    }

    #[test]
    fn many_large_lines_are_emphasis_not_headings() {
        let long = "Dieser ganze Abschnitt ist in größerer Schrift gesetzt, weil er wichtig ist";
        let page: Vec<(&str, f32)> = vec![
            (long, 14.0),
            (long, 14.0),
            (long, 14.0),
            (long, 14.0),
            (
                "Normaler Text der Seite, der weitergeht und weitergeht.",
                11.0,
            ),
        ];
        let body = "Der Wind strich um den Turm, und Mara lauschte lange auf das Meer.";
        let rest: Vec<(&str, f32)> = vec![(body, 11.0); 30];
        let out = pdf_book_to_text(&sized(&[&page, &rest]));
        assert!(!out.contains('#'), "{out}");
    }

    #[test]
    fn outline_entries_mark_headings_or_are_inserted() {
        let body = "Der Wind strich um den Turm, und Mara lauschte lange auf das Meer.";
        let mut book = sized(&[
            &[("Erstes Kapitel", 11.0), (body, 11.0)],
            &[(body, 11.0), (body, 11.0)],
            &[(body, 11.0)],
        ]);
        book.outline = vec![
            crate::pdf_text::OutlineEntry {
                level: 0,
                title: "Erstes Kapitel".into(),
                page: 0,
            },
            crate::pdf_text::OutlineEntry {
                level: 1,
                title: "Ein Abschnitt ohne Zeile".into(),
                page: 2,
            },
        ];
        let out = pdf_book_to_text(&book);
        assert!(out.starts_with("## Erstes Kapitel\n"), "{out}");
        assert!(out.contains("\n### Ein Abschnitt ohne Zeile\n"), "{out}");
    }

    #[test]
    fn unusable_outline_falls_back_and_missing_titles_wait_for_sentence_end() {
        let body = "Der Wind strich um den Turm, und Mara lauschte lange auf das Meer";
        let end = "und hörte die Wellen bis zum Morgen.";
        let entry = |title: &str, page: usize| crate::pdf_text::OutlineEntry {
            level: 0,
            title: title.into(),
            page,
        };
        // Ein Titel fehlt im Text: er kommt hinter das Satzende, nicht in den Satz.
        let mut book = sized(&[
            &[("Erstes Kapitel", 11.0), (body, 11.0)],
            &[(end, 11.0), ("Neuer Absatz beginnt hier.", 11.0)],
            &[("Zweites Kapitel", 11.0), (body, 11.0), (end, 11.0)],
        ]);
        book.outline = vec![
            entry("Erstes Kapitel", 0),
            entry("Zwischenspiel", 1),
            entry("Zweites Kapitel", 2),
        ];
        let out = pdf_book_to_text(&book);
        assert!(
            out.contains("Meer und hörte die Wellen bis zum Morgen."),
            "{out}"
        );
        assert!(out.contains("## Zwischenspiel\n\nNeuer Absatz"), "{out}");
        // Seiten-Lesezeichen eines Scanners zählen nicht; ohne brauchbare
        // Lesezeichen greifen die Kapitelzeilen.
        book.outline = vec![
            entry("Seite 1", 0),
            entry("Seite 2", 1),
            entry("Seite 3", 2),
        ];
        let out = pdf_book_to_text(&book);
        assert!(!out.contains("## Seite"), "{out}");
        assert!(out.contains("## Zweites Kapitel"), "{out}");
    }

    #[test]
    fn numbered_list_after_a_title_line_stays_a_list() {
        let out = reflow_plain_text(
            "Man braucht für den Teig nur wenige Zutaten, die es überall zu kaufen gibt\n\
             1. Mehl aus der Mühle am Fluss\n\
             2. Ei",
        );
        assert!(out.contains("\n\n1. Mehl"), "{out}");
        assert!(out.contains("\n\n2. Ei"), "{out}");
    }

    #[test]
    fn preface_heading_gets_no_title_appended() {
        let out = promote_chapter_lines("Vorwort\n\nLiebe Leserinnen und Leser!\n\nDieses Buch …");
        assert!(out.starts_with("## Vorwort\n\nLiebe Leserinnen"), "{out}");
        let out = promote_chapter_lines("Erstes Kapitel\n\n„Komm her!“\n\nSie kam.");
        assert!(out.starts_with("## Erstes Kapitel\n\n„Komm her!“"), "{out}");
    }

    #[test]
    fn paragraph_gaps_split_paragraphs_when_rare() {
        let full = "Der Wind strich um den Turm, und Mara lauschte lange auf das Meer und";
        let mut book = sized(&[&[
            (full, 11.0),
            (
                "die Wellen bis zum Morgen, als das Licht kam und alles wieder gut war.",
                11.0,
            ),
            (full, 11.0),
            (
                "die Wellen bis zum Morgen, als das Licht kam und alles wieder gut war.",
                11.0,
            ),
            (full, 11.0),
            (full, 11.0),
        ]]);
        book.pages[0][2].gap = true;
        let out = pdf_book_to_text(&book);
        assert_eq!(out.matches("\n\n").count(), 1, "{out}");
    }

    #[test]
    fn epub_chapter_lines_become_headings() {
        let md = "Effi Briest\n\nErstes Kapitel\n\nIn Front des Herrenhauses fiel heller Sonnenschein.\n\n\
                  **Zweites Kapitel.**\n\nDer Brief\n\nEs war ein Sonntag.\n\nKapitel 3 zeigt, wie es weiterging.";
        let out = promote_chapter_lines(md);
        assert!(out.contains("\n\n## Erstes Kapitel\n\n"), "{out}");
        assert!(
            out.contains("\n\n## Zweites Kapitel: Der Brief\n\n"),
            "{out}"
        );
        assert!(out.contains("Kapitel 3 zeigt"), "{out}");
        assert!(out.starts_with("Effi Briest"), "{out}");
    }

    #[test]
    fn epub_toc_from_nav_and_ncx() {
        let nav = r#"<html><body><nav epub:type="toc"><ol>
<li><a href="Text/k1.xhtml">Teil 1</a><ol><li><a href="Text/k1.xhtml#a2">Der Brief</a></li></ol></li>
<li><a href="Text/k%202.xhtml">Teil <em>2</em></a></li></ol></nav>
<nav epub:type="landmarks"><ol><li><a href="Text/k1.xhtml">Start</a></li></ol></nav></body></html>"#;
        let toc = nav_toc(nav, "OEBPS");
        assert_eq!(
            toc.iter()
                .map(|e| (
                    e.depth,
                    e.title.as_str(),
                    e.path.as_str(),
                    e.fragment.as_deref()
                ))
                .collect::<Vec<_>>(),
            vec![
                (0, "Teil 1", "OEBPS/Text/k1.xhtml", None),
                (1, "Der Brief", "OEBPS/Text/k1.xhtml", Some("a2")),
                (0, "Teil 2", "OEBPS/Text/k 2.xhtml", None),
            ]
        );
        let ncx = r#"<ncx><navMap><navPoint id="n1"><navLabel><text>Erstes Kapitel</text></navLabel>
<content src="c1.xhtml"/><navPoint id="n2"><navLabel><text>Ein Abschnitt</text></navLabel>
<content src="c1.xhtml#s"/></navPoint></navPoint></navMap></ncx>"#;
        let toc = ncx_toc(ncx, "");
        assert_eq!(toc.len(), 2);
        assert_eq!((toc[1].depth, toc[1].title.as_str()), (1, "Ein Abschnitt"));
    }

    #[test]
    fn epub_toc_titles_mark_paragraphs_as_headings() {
        let e1 = TocEntry {
            depth: 0,
            title: "Der Brief".into(),
            path: "x".into(),
            fragment: Some("a".into()),
        };
        let e2 = TocEntry {
            depth: 1,
            title: "Am Bahnhof".into(),
            path: "x".into(),
            fragment: Some("b".into()),
        };
        let md = "Vorbemerkung zum Brief.\n\n**Der Brief**\n\nText.\n\nAm Bahnhof\n\nMehr Text.";
        let out = apply_toc(md, &[&e1, &e2]);
        assert_eq!(
            out,
            "Vorbemerkung zum Brief.\n\n## Der Brief\n\nText.\n\n### Am Bahnhof\n\nMehr Text."
        );
        // Ganze Datei ohne Überschrift: Titel davor.
        let whole = TocEntry {
            fragment: None,
            ..e1.clone()
        };
        assert_eq!(
            apply_toc("Text ohne Kopf.", &[&whole]),
            "## Der Brief\n\nText ohne Kopf."
        );
        assert_eq!(apply_toc("# Kopf\n\nText.", &[&whole]), "# Kopf\n\nText.");
    }

    #[test]
    fn epub_uses_ncx_for_chapter_headings() {
        let opf = r#"<package version="2.0"><metadata><dc:title>Buch</dc:title></metadata>
<manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
<item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine toc="ncx"><itemref idref="c1"/></spine></package>"#;
        let ncx = r#"<ncx><navMap><navPoint><navLabel><text>Die Ankunft</text></navLabel><content src="c1.xhtml#k1"/></navPoint>
<navPoint><navLabel><text>Der Sturm</text></navLabel><content src="c1.xhtml#k2"/></navPoint></navMap></ncx>"#;
        let mut a = epub(&[
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", opf),
            ("OEBPS/toc.ncx", ncx),
            (
                "OEBPS/c1.xhtml",
                r#"<html><body><p id="k1" class="chapter">Die Ankunft</p><p>Sie kam am Morgen.</p>
<p id="k2" class="chapter">Der Sturm</p><p>In der Nacht wurde es laut.</p></body></html>"#,
            ),
        ]);
        let out = epub_to_markdown(&mut a).unwrap();
        assert!(
            out.contains("## Die Ankunft\n\nSie kam am Morgen."),
            "{out}"
        );
        assert!(out.contains("## Der Sturm\n\nIn der Nacht"), "{out}");
    }
}
