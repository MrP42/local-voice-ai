//! EPUB 3 aus dem Text einer Fassung (lva#236).
//!
//! Ein Buch, eine Übersetzung oder eine Kurzversion wird als EPUB weitergegeben,
//! das Apple Books, Kindle (Send to Kindle), Tolino und Calibre öffnen. Kapitel
//! entstehen an den Überschriften der Ebenen 1 und 2; Inhaltsverzeichnis als
//! EPUB-3-Navigation und zusätzlich als NCX (ältere Lesegeräte). Das Cover der
//! Bibliothek kommt mit, wenn es eins gibt.
//!
//! Aus Markdown wird nur, was Bücher brauchen: Überschriften, Absätze, Listen,
//! Zitate, Trenner, fett und kursiv. Rein (nur `zip`), ohne App-Build prüfbar.

use std::io::Write;

/// Angaben im Kopf des EPUB.
pub struct EpubMeta<'a> {
    /// Eindeutige Kennung (z. B. `urn:ulid:…`).
    pub id: &'a str,
    pub title: &'a str,
    pub authors: &'a [String],
    /// BCP-47-Sprachcode (`de`, `en`); leer: `und`.
    pub language: &'a str,
    /// `dcterms:modified`, Format `2026-11-03T20:00:00Z`.
    pub modified: &'a str,
}

/// Cover als Bild (`image/jpeg` oder `image/png`).
pub struct EpubCover<'a> {
    pub bytes: &'a [u8],
    pub media_type: &'a str,
}

/// Ein Kapitel: Titel fürs Inhaltsverzeichnis und XHTML-Körper.
#[derive(Debug, PartialEq)]
pub struct Chapter {
    pub title: String,
    pub body: String,
}

/// EPUB-Datei als Bytes.
pub fn build(
    meta: &EpubMeta<'_>,
    markdown: &str,
    cover: Option<&EpubCover<'_>>,
) -> Result<Vec<u8>, String> {
    let chapters = chapters(markdown, meta.title);
    let lang = if meta.language.trim().is_empty() {
        "und"
    } else {
        meta.language.trim()
    };
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let stored: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut put = |name: &str, bytes: &[u8], opts| -> Result<(), String> {
            zip.start_file(name, opts)
                .map_err(|e| format!("epub: {name}: {e}"))?;
            zip.write_all(bytes)
                .map_err(|e| format!("epub: {name}: {e}"))
        };
        // Die Spezifikation verlangt `mimetype` zuerst und unkomprimiert.
        put("mimetype", b"application/epub+zip", stored)?;
        put("META-INF/container.xml", CONTAINER.as_bytes(), deflated)?;
        put("OEBPS/style.css", STYLE.as_bytes(), deflated)?;
        let cover_ext = cover.map(|c| {
            if c.media_type == "image/png" {
                "png"
            } else {
                "jpg"
            }
        });
        if let (Some(c), Some(ext)) = (cover, cover_ext) {
            put(&format!("OEBPS/images/cover.{ext}"), c.bytes, stored)?;
            put(
                "OEBPS/cover.xhtml",
                page(lang, meta.title, &format!(
                    "<section epub:type=\"cover\" class=\"cover\"><img src=\"images/cover.{ext}\" alt=\"{}\"/></section>",
                    esc(meta.title)
                ))
                .as_bytes(),
                deflated,
            )?;
        }
        for (i, ch) in chapters.iter().enumerate() {
            put(
                &format!("OEBPS/text/{}", chapter_file(i)),
                page(lang, &ch.title, &ch.body).as_bytes(),
                deflated,
            )?;
        }
        put(
            "OEBPS/nav.xhtml",
            nav(lang, meta.title, &chapters).as_bytes(),
            deflated,
        )?;
        put("OEBPS/toc.ncx", ncx(meta, &chapters).as_bytes(), deflated)?;
        put(
            "OEBPS/content.opf",
            opf(
                meta,
                lang,
                &chapters,
                cover.map(|c| c.media_type).zip(cover_ext),
            )
            .as_bytes(),
            deflated,
        )?;
        zip.finish().map_err(|e| format!("epub: {e}"))?;
    }
    Ok(buf.into_inner())
}

fn chapter_file(i: usize) -> String {
    format!("ch{:03}.xhtml", i + 1)
}

const CONTAINER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#;

const STYLE: &str = "body { font-family: serif; line-height: 1.5; margin: 0 5%; }
h1, h2, h3 { font-family: sans-serif; line-height: 1.25; page-break-after: avoid; }
h1 { font-size: 1.6em; margin: 1.5em 0 1em; }
h2 { font-size: 1.3em; margin: 1.4em 0 0.8em; }
h3 { font-size: 1.1em; margin: 1.2em 0 0.6em; }
p { margin: 0 0 0.8em; text-align: justify; }
blockquote { margin: 1em 1.5em; font-style: italic; }
hr { border: 0; text-align: center; margin: 1.5em 0; }
hr:after { content: \"\u{2042}\"; }
.cover { text-align: center; margin: 0; }
.cover img { max-width: 100%; max-height: 100%; }
";

fn page(lang: &str, title: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{l}\" lang=\"{l}\">\n<head><meta charset=\"UTF-8\"/><title>{t}</title><link rel=\"stylesheet\" type=\"text/css\" href=\"{css}\"/></head>\n<body>\n{body}\n</body>\n</html>\n",
        l = esc(lang),
        t = esc(title),
        css = if body.contains("class=\"cover\"") {
            "style.css"
        } else {
            "../style.css"
        },
    )
}

fn nav(lang: &str, title: &str, chapters: &[Chapter]) -> String {
    let items: String = chapters
        .iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                "<li><a href=\"text/{}\">{}</a></li>",
                chapter_file(i),
                esc(&c.title)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{l}\" lang=\"{l}\">\n<head><meta charset=\"UTF-8\"/><title>{t}</title></head>\n<body>\n<nav epub:type=\"toc\" id=\"toc\"><h1>{t}</h1>\n<ol>\n{items}\n</ol></nav>\n</body>\n</html>\n",
        l = esc(lang),
        t = esc(title),
    )
}

fn ncx(meta: &EpubMeta<'_>, chapters: &[Chapter]) -> String {
    let points: String = chapters
        .iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                "<navPoint id=\"p{n}\" playOrder=\"{n}\"><navLabel><text>{t}</text></navLabel><content src=\"text/{f}\"/></navPoint>",
                n = i + 1,
                t = esc(&c.title),
                f = chapter_file(i)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n<head><meta name=\"dtb:uid\" content=\"{id}\"/></head>\n<docTitle><text>{t}</text></docTitle>\n<navMap>\n{points}\n</navMap>\n</ncx>\n",
        id = esc(meta.id),
        t = esc(meta.title),
    )
}

fn opf(
    meta: &EpubMeta<'_>,
    lang: &str,
    chapters: &[Chapter],
    cover: Option<(&str, &str)>,
) -> String {
    let creators: String = meta
        .authors
        .iter()
        .filter(|a| !a.trim().is_empty())
        .enumerate()
        .map(|(i, a)| {
            format!(
                "<dc:creator id=\"creator{}\">{}</dc:creator>",
                i + 1,
                esc(a.trim())
            )
        })
        .collect::<Vec<_>>()
        .join("\n    ");
    let mut manifest = vec![
        "<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>".to_string(),
        "<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>".to_string(),
        "<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>".to_string(),
    ];
    let mut spine = Vec::new();
    let mut cover_meta = String::new();
    if let Some((media, ext)) = cover {
        manifest.push(format!(
            "<item id=\"cover-image\" href=\"images/cover.{ext}\" media-type=\"{media}\" properties=\"cover-image\"/>"
        ));
        manifest.push(
            "<item id=\"cover\" href=\"cover.xhtml\" media-type=\"application/xhtml+xml\"/>"
                .to_string(),
        );
        spine.push("<itemref idref=\"cover\" linear=\"no\"/>".to_string());
        cover_meta = "\n    <meta name=\"cover\" content=\"cover-image\"/>".to_string();
    }
    for i in 0..chapters.len() {
        manifest.push(format!(
            "<item id=\"c{n}\" href=\"text/{f}\" media-type=\"application/xhtml+xml\"/>",
            n = i + 1,
            f = chapter_file(i)
        ));
        spine.push(format!("<itemref idref=\"c{}\"/>", i + 1));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\" xml:lang=\"{l}\">\n  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n    <dc:identifier id=\"bookid\">{id}</dc:identifier>\n    <dc:title>{t}</dc:title>\n    {creators}\n    <dc:language>{l}</dc:language>\n    <meta property=\"dcterms:modified\">{m}</meta>{cover_meta}\n  </metadata>\n  <manifest>\n    {manifest}\n  </manifest>\n  <spine toc=\"ncx\">\n    {spine}\n  </spine>\n</package>\n",
        l = esc(lang),
        id = esc(meta.id),
        t = esc(meta.title),
        m = esc(meta.modified),
        manifest = manifest.join("\n    "),
        spine = spine.join("\n    "),
    )
}

/// Markdown in Kapitel zerlegen: jede Überschrift der Ebene 1 oder 2 beginnt
/// ein neues. Text vor der ersten Überschrift wird ein eigenes Kapitel mit dem
/// Buchtitel. Ohne Überschriften ist das ganze Buch ein Kapitel.
pub fn chapters(markdown: &str, title: &str) -> Vec<Chapter> {
    let mut out: Vec<(String, Vec<&str>)> = Vec::new();
    for line in markdown.lines() {
        if let Some((level, text)) = heading(line) {
            if level <= 2 {
                out.push((text.to_string(), vec![line]));
                continue;
            }
        }
        match out.last_mut() {
            Some((_, lines)) => lines.push(line),
            None => out.push((title.to_string(), vec![line])),
        }
    }
    out.retain(|(_, lines)| lines.iter().any(|l| !l.trim().is_empty()));
    if out.is_empty() {
        out.push((title.to_string(), vec![]));
    }
    out.into_iter()
        .map(|(t, lines)| Chapter {
            title: plain_inline(&t),
            body: blocks_to_xhtml(&lines),
        })
        .collect()
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let level = t.chars().take_while(|&c| c == '#').count();
    if !(1..=3).contains(&level) {
        return None;
    }
    let rest = &t[level..];
    if !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim();
    (!text.is_empty()).then_some((level, text))
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t == "⁂"
        || t == "* * *"
        || (t.len() >= 3
            && (t.chars().all(|c| c == '-')
                || t.chars().all(|c| c == '*')
                || t.chars().all(|c| c == '_')))
}

fn bullet(line: &str) -> Option<&str> {
    let t = line.trim_start();
    for p in ["- ", "* ", "• "] {
        if let Some(rest) = t.strip_prefix(p) {
            return Some(rest.trim());
        }
    }
    None
}

/// Zeilen eines Kapitels zu XHTML.
fn blocks_to_xhtml(lines: &[&str]) -> String {
    let mut html: Vec<String> = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut list: Vec<&str> = Vec::new();
    let mut quote: Vec<&str> = Vec::new();
    fn flush_para(html: &mut Vec<String>, para: &mut Vec<&str>) {
        if !para.is_empty() {
            let text = para.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
            html.push(format!("<p>{}</p>", inline(&text)));
            para.clear();
        }
    }
    fn flush_list(html: &mut Vec<String>, list: &mut Vec<&str>) {
        if !list.is_empty() {
            let items: String = list
                .iter()
                .map(|l| format!("<li>{}</li>", inline(l)))
                .collect();
            html.push(format!("<ul>{items}</ul>"));
            list.clear();
        }
    }
    fn flush_quote(html: &mut Vec<String>, quote: &mut Vec<&str>) {
        if !quote.is_empty() {
            let text = quote.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
            html.push(format!("<blockquote><p>{}</p></blockquote>", inline(&text)));
            quote.clear();
        }
    }
    for line in lines {
        if line.trim().is_empty() {
            flush_para(&mut html, &mut para);
            flush_list(&mut html, &mut list);
            flush_quote(&mut html, &mut quote);
            continue;
        }
        if let Some((level, text)) = heading(line) {
            flush_para(&mut html, &mut para);
            flush_list(&mut html, &mut list);
            flush_quote(&mut html, &mut quote);
            html.push(format!("<h{level}>{}</h{level}>", inline(text)));
            continue;
        }
        if is_rule(line) {
            flush_para(&mut html, &mut para);
            flush_list(&mut html, &mut list);
            flush_quote(&mut html, &mut quote);
            html.push("<hr/>".to_string());
            continue;
        }
        if let Some(item) = bullet(line) {
            flush_para(&mut html, &mut para);
            flush_quote(&mut html, &mut quote);
            list.push(item);
            continue;
        }
        if let Some(q) = line.trim_start().strip_prefix('>') {
            flush_para(&mut html, &mut para);
            flush_list(&mut html, &mut list);
            quote.push(q.trim_start());
            continue;
        }
        flush_list(&mut html, &mut list);
        flush_quote(&mut html, &mut quote);
        para.push(line);
    }
    flush_para(&mut html, &mut para);
    flush_list(&mut html, &mut list);
    flush_quote(&mut html, &mut quote);
    html.join("\n")
}

/// `**fett**` und `*kursiv*`/`_kursiv_` (an Wortgrenzen) zu XHTML; alles
/// andere maskiert. Offene Auszeichnungen werden am Ende geschlossen.
fn inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 16);
    let mut bold = false;
    let mut italic = false;
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '*' && chars.get(i + 1) == Some(&'*') {
            out.push_str(if bold { "</strong>" } else { "<strong>" });
            bold = !bold;
            i += 2;
            continue;
        }
        if c == '*' || c == '_' {
            let prev = if i > 0 { chars.get(i - 1) } else { None };
            let next = chars.get(i + 1);
            let opens = !italic && !word(prev) && next.is_some_and(|n| !n.is_whitespace());
            let closes = italic && !word(next) && prev.is_some_and(|p| !p.is_whitespace());
            if opens || closes {
                out.push_str(if italic { "</em>" } else { "<em>" });
                italic = !italic;
                i += 1;
                continue;
            }
        }
        push_esc(&mut out, c);
        i += 1;
    }
    if italic {
        out.push_str("</em>");
    }
    if bold {
        out.push_str("</strong>");
    }
    out
}

/// Titel ohne Auszeichnung (Inhaltsverzeichnis).
fn plain_inline(text: &str) -> String {
    text.replace("**", "").trim().to_string()
}

fn push_esc(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        c => out.push(c),
    }
}

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        push_esc(&mut out, c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn meta<'a>(authors: &'a [String]) -> EpubMeta<'a> {
        EpubMeta {
            id: "urn:ulid:01TEST",
            title: "Effi & Briest",
            authors,
            language: "de",
            modified: "2026-11-03T20:00:00Z",
        }
    }

    #[test]
    fn chapters_split_at_level_one_and_two() {
        let md = "Vorwort ohne Titel.\n\n# Erstes\n\nText *eins*.\n\n### Unter\n\nmehr\n\n## Zweites\n\n- a\n- b\n";
        let ch = chapters(md, "Buch");
        assert_eq!(ch.len(), 3);
        assert_eq!(ch[0].title, "Buch");
        assert_eq!(ch[1].title, "Erstes");
        assert!(ch[1].body.contains("<h1>Erstes</h1>"));
        assert!(ch[1].body.contains("<p>Text <em>eins</em>.</p>"));
        assert!(ch[1].body.contains("<h3>Unter</h3>"));
        assert_eq!(
            ch[2].body,
            "<h2>Zweites</h2>\n<ul><li>a</li><li>b</li></ul>"
        );
    }

    #[test]
    fn inline_escapes_and_marks() {
        assert_eq!(
            inline("a < b & **c** snake_case 3*4"),
            "a &lt; b &amp; <strong>c</strong> snake_case 3*4"
        );
        assert_eq!(inline("**offen"), "<strong>offen</strong>");
    }

    #[test]
    fn paragraph_lines_join_and_quote_and_rule() {
        let ch = chapters("Zeile eins\nZeile zwei\n\n> Zitat\n\n---\n\nEnde", "T");
        assert_eq!(
            ch[0].body,
            "<p>Zeile eins Zeile zwei</p>\n<blockquote><p>Zitat</p></blockquote>\n<hr/>\n<p>Ende</p>"
        );
    }

    #[test]
    fn empty_text_still_has_one_chapter() {
        let ch = chapters("", "Leer");
        assert_eq!(ch.len(), 1);
        assert_eq!(ch[0].title, "Leer");
    }

    #[test]
    fn package_layout_is_valid_epub() {
        let authors = vec!["Theodor Fontane".to_string()];
        let cover = EpubCover {
            bytes: &[0xFF, 0xD8, 0xFF],
            media_type: "image/jpeg",
        };
        let bytes = build(&meta(&authors), "# Eins\n\nA\n\n# Zwei\n\nB", Some(&cover)).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        // mimetype zuerst, unkomprimiert
        {
            let mut first = zip.by_index(0).unwrap();
            assert_eq!(first.name(), "mimetype");
            assert_eq!(first.compression(), zip::CompressionMethod::Stored);
            let mut s = String::new();
            first.read_to_string(&mut s).unwrap();
            assert_eq!(s, "application/epub+zip");
        }
        let mut opf = String::new();
        zip.by_name("OEBPS/content.opf")
            .unwrap()
            .read_to_string(&mut opf)
            .unwrap();
        assert!(opf.contains("<dc:title>Effi &amp; Briest</dc:title>"));
        assert!(opf.contains("<dc:creator id=\"creator1\">Theodor Fontane</dc:creator>"));
        assert!(opf.contains("properties=\"cover-image\""));
        assert!(opf.contains("<itemref idref=\"c2\"/>"));
        for name in [
            "META-INF/container.xml",
            "OEBPS/nav.xhtml",
            "OEBPS/toc.ncx",
            "OEBPS/style.css",
            "OEBPS/cover.xhtml",
            "OEBPS/images/cover.jpg",
            "OEBPS/text/ch001.xhtml",
            "OEBPS/text/ch002.xhtml",
        ] {
            assert!(zip.by_name(name).is_ok(), "{name} fehlt");
        }
        let mut ch = String::new();
        zip.by_name("OEBPS/text/ch002.xhtml")
            .unwrap()
            .read_to_string(&mut ch)
            .unwrap();
        assert!(ch.contains("<h1>Zwei</h1>"));
        assert!(ch.contains("href=\"../style.css\""));
    }
}
