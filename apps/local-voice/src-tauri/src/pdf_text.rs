//! PDF-Text mit Schriftgröße je Zeile und Lesezeichen (lva#149, Buchimport).
//!
//! `pdf_extract::extract_text_by_pages` liefert nur Text. Für Überschriften
//! braucht der Import die Schriftgröße jeder Zeile und das Lesezeichen-
//! Verzeichnis (Outline) des PDFs. Die Zeilenbildung hier folgt Zeichen für
//! Zeichen der von `pdf_extract::PlainTextOutput` (gleiche Schwellen), damit
//! der Text derselbe bleibt wie bisher; dazu kommt je Zeile die größte
//! Schriftgröße in Punkt.

use pdf_extract::{MediaBox, OutputDev, OutputError, Transform};

/// Eine Druckzeile: Text und größte Schriftgröße (pt, nach Transformation).
#[derive(Debug, Clone, PartialEq)]
pub struct PdfLine {
    pub text: String,
    pub size: f32,
    /// Vor der Zeile ein Abstand von mehr als anderthalb Zeilen (wie die
    /// Leerzeile in `PlainTextOutput`): Hinweis auf einen neuen Absatz.
    pub gap: bool,
}

/// Ein Eintrag im Lesezeichen-Verzeichnis: Tiefe (0 = oberste Ebene), Titel
/// und Seite (0-basiert).
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineEntry {
    pub level: usize,
    pub title: String,
    pub page: usize,
}

/// Ein PDF als Zeilen je Seite samt Lesezeichen.
#[derive(Debug, Clone, Default)]
pub struct PdfBook {
    pub pages: Vec<Vec<PdfLine>>,
    pub outline: Vec<OutlineEntry>,
}

/// Sammelt die Zeilen einer Seite (Logik wie `PlainTextOutput`).
struct SizedLines {
    lines: Vec<PdfLine>,
    current: String,
    current_size: f64,
    current_gap: bool,
    next_gap: bool,
    last_end: f64,
    last_y: f64,
    first_char: bool,
    flip_ctm: Transform,
}

impl SizedLines {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            current: String::new(),
            current_size: 0.0,
            current_gap: false,
            next_gap: false,
            last_end: 100000.,
            last_y: 0.,
            first_char: false,
            flip_ctm: Transform::identity(),
        }
    }

    fn break_line(&mut self) {
        let text = std::mem::take(&mut self.current);
        if !text.trim().is_empty() {
            self.lines.push(PdfLine {
                text,
                size: self.current_size as f32,
                gap: self.current_gap,
            });
        }
        self.current_size = 0.0;
        self.current_gap = std::mem::take(&mut self.next_gap);
    }
}

impl OutputDev for SizedLines {
    fn begin_page(
        &mut self,
        _page_num: u32,
        media_box: &MediaBox,
        _art_box: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), OutputError> {
        self.flip_ctm = Transform::row_major(1., 0., 0., -1., 0., media_box.ury - media_box.lly);
        Ok(())
    }
    fn end_page(&mut self) -> Result<(), OutputError> {
        self.break_line();
        Ok(())
    }
    fn output_character(
        &mut self,
        trm: &Transform,
        width: f64,
        _spacing: f64,
        font_size: f64,
        ch: &str,
    ) -> Result<(), OutputError> {
        let position = trm.post_transform(&self.flip_ctm);
        // Wie `transform_vector(vec2(font_size, font_size))` in pdf-extract.
        let vx = font_size * trm.m11 + font_size * trm.m21;
        let vy = font_size * trm.m12 + font_size * trm.m22;
        // Für die Zeilenschwellen exakt wie dort (auch NaN bei Spiegelung),
        // für die gemeldete Größe der Betrag.
        let size = (vx * vy).sqrt();
        let shown = (vx * vy).abs().sqrt();
        let (x, y) = (position.m31, position.m32);
        if self.first_char {
            let gap = (y - self.last_y).abs() > size * 1.5;
            let new_line = gap || (x < self.last_end && (y - self.last_y).abs() > size * 0.5);
            if new_line {
                self.next_gap = gap;
                self.break_line();
            } else if x > self.last_end + size * 0.1 {
                self.current.push(' ');
            }
        }
        // Leerzeichen zählen nicht für die Größe (manche PDFs setzen sie groß).
        if !ch.trim().is_empty() && shown.is_finite() && shown > self.current_size {
            self.current_size = shown;
        }
        self.current.push_str(ch);
        self.first_char = false;
        self.last_y = y;
        self.last_end = x + width * size;
        Ok(())
    }
    fn begin_word(&mut self) -> Result<(), OutputError> {
        self.first_char = true;
        Ok(())
    }
    fn end_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
    fn end_line(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
}

/// Höchstens so viele Lesezeichen und so tief: kaputte PDFs mit Ringen oder
/// endlosen Ketten dürfen den Import nicht aufhalten.
const MAX_OUTLINE: usize = 5000;
const MAX_OUTLINE_DEPTH: usize = 32;
const MAX_NAME_TREE: usize = 20_000;

/// PDFDocEncoding 0x80–0xA0 (sonst wie Latin-1), PDF 1.7 Anhang D.
const PDF_DOC_HIGH: [char; 33] = [
    '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚', '™',
    'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{FFFD}', '€',
];

/// Textstring eines PDFs: UTF-16BE oder UTF-8 mit BOM, sonst PDFDocEncoding
/// (Voreinstellung etwa von LaTeX/hyperref; lopdf liest das als UTF-8 und
/// macht aus „Einführung“ „Einf�hrung“).
pub(crate) fn pdf_text_string(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0xA0 => PDF_DOC_HIGH[(b - 0x80) as usize],
            _ => b as char,
        })
        .collect()
}

/// Objekt auflösen (Verweise folgen, höchstens ein paar Stufen).
fn resolve<'a>(
    doc: &'a pdf_extract::Document,
    mut obj: &'a pdf_extract::Object,
) -> Option<&'a pdf_extract::Object> {
    for _ in 0..8 {
        match obj {
            pdf_extract::Object::Reference(id) => obj = doc.get_object(*id).ok()?,
            other => return Some(other),
        }
    }
    None
}

fn dict_get<'a>(
    doc: &'a pdf_extract::Document,
    dict: &'a pdf_extract::Dictionary,
    key: &[u8],
) -> Option<&'a pdf_extract::Object> {
    dict.get(key).ok().and_then(|o| resolve(doc, o))
}

/// Ziel eines benannten Sprungs im Namensbaum (`/Names /Dests`) oder im
/// alten `/Dests`-Wörterbuch.
fn named_dest<'a>(doc: &'a pdf_extract::Document, name: &[u8]) -> Option<&'a pdf_extract::Object> {
    let catalog = doc.catalog().ok()?;
    if let Some(pdf_extract::Object::Dictionary(dests)) = dict_get(doc, catalog, b"Dests") {
        if let Some(d) = dict_get(doc, dests, name) {
            return Some(d);
        }
    }
    let names = match dict_get(doc, catalog, b"Names")? {
        pdf_extract::Object::Dictionary(d) => d,
        _ => return None,
    };
    let mut stack = vec![dict_get(doc, names, b"Dests")?];
    let mut seen = 0;
    while let Some(node) = stack.pop() {
        seen += 1;
        if seen > MAX_NAME_TREE {
            return None;
        }
        let pdf_extract::Object::Dictionary(node) = node else {
            continue;
        };
        if let Some(pdf_extract::Object::Array(pairs)) = dict_get(doc, node, b"Names") {
            for pair in pairs.chunks_exact(2) {
                if resolve(doc, &pair[0]).and_then(|k| k.as_str().ok()) == Some(name) {
                    return resolve(doc, &pair[1]);
                }
            }
        }
        if let Some(pdf_extract::Object::Array(kids)) = dict_get(doc, node, b"Kids") {
            stack.extend(kids.iter().filter_map(|k| resolve(doc, k)));
        }
    }
    None
}

/// Seite (0-basiert) eines Lesezeichens aus `/Dest` oder `/A` (GoTo).
fn outline_page(
    doc: &pdf_extract::Document,
    node: &pdf_extract::Dictionary,
    pages: &std::collections::HashMap<pdf_extract::ObjectId, usize>,
) -> Option<usize> {
    let mut dest = dict_get(doc, node, b"Dest");
    if dest.is_none() {
        if let Some(pdf_extract::Object::Dictionary(action)) = dict_get(doc, node, b"A") {
            if dict_get(doc, action, b"S").and_then(|s| s.as_name().ok())
                == Some(b"GoTo".as_slice())
            {
                dest = dict_get(doc, action, b"D");
            }
        }
    }
    let mut dest = dest?;
    for _ in 0..3 {
        match dest {
            pdf_extract::Object::Array(a) => {
                return match a.first()? {
                    pdf_extract::Object::Reference(id) => pages.get(id).copied(),
                    _ => None,
                };
            }
            pdf_extract::Object::Dictionary(d) => dest = dict_get(doc, d, b"D")?,
            pdf_extract::Object::Name(n) | pdf_extract::Object::String(n, _) => {
                dest = named_dest(doc, n)?
            }
            _ => return None,
        }
    }
    None
}

/// Lesezeichen des PDFs, wenn vorhanden (Seiten 0-basiert, nur gültige).
/// Eigener Durchlauf statt `get_toc()`: lopdf folgt `/Next` ohne
/// Ringerkennung (ein kaputtes PDF hängt dann endlos), führt gleiche Titel
/// zusammen und liest Titel ohne BOM als UTF-8.
fn outline_of(doc: &pdf_extract::Document) -> Vec<OutlineEntry> {
    let pages: std::collections::HashMap<pdf_extract::ObjectId, usize> = doc
        .get_pages()
        .into_iter()
        .map(|(n, id)| (id, n as usize - 1))
        .collect();
    let Some(first) = doc
        .catalog()
        .ok()
        .and_then(|c| dict_get(doc, c, b"Outlines"))
        .and_then(|o| o.as_dict().ok())
        .and_then(|o| o.get(b"First").ok())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut visited = std::collections::HashSet::new();
    // (Knoten, Tiefe); Geschwister kommen in Reihenfolge, Kinder davor.
    let mut stack: Vec<(&pdf_extract::Object, usize)> = vec![(first, 0)];
    while let Some((obj, level)) = stack.pop() {
        if out.len() >= MAX_OUTLINE || visited.len() >= MAX_OUTLINE * 2 {
            break;
        }
        if let pdf_extract::Object::Reference(id) = obj {
            if !visited.insert(*id) {
                continue;
            }
        }
        let Some(pdf_extract::Object::Dictionary(node)) = resolve(doc, obj) else {
            continue;
        };
        if let Ok(next) = node.get(b"Next") {
            stack.push((next, level));
        }
        if level + 1 < MAX_OUTLINE_DEPTH {
            if let Ok(child) = node.get(b"First") {
                stack.push((child, level + 1));
            }
        }
        let title = dict_get(doc, node, b"Title")
            .and_then(|t| t.as_str().ok())
            .map(pdf_text_string)
            .unwrap_or_default();
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if !title.chars().any(char::is_alphanumeric) {
            continue;
        }
        if let Some(page) = outline_page(doc, node, &pages) {
            out.push(OutlineEntry { level, title, page });
        }
    }
    out
}

/// Panik als Fehler: pdf-extract und lopdf haben ungeschützte Zugriffe.
fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// PDF laden: Zeilen mit Schriftgröße je Seite und Lesezeichen. Eine Panik
/// in pdf-extract kostet nur die Seite (bzw. die Lesezeichen), nicht das Buch.
pub fn read_pdf(bytes: &[u8]) -> Result<PdfBook, String> {
    let doc = guarded(|| -> Result<pdf_extract::Document, String> {
        let mut doc = pdf_extract::Document::load_mem(bytes).map_err(|e| e.to_string())?;
        if doc.is_encrypted() {
            // Nur „offene“ PDFs mit leerem Benutzerpasswort (wie pdf-extract).
            doc.decrypt("").map_err(|e| e.to_string())?;
        }
        Ok(doc)
    })
    .unwrap_or_else(|| Err("PDF konnte nicht gelesen werden".into()))?;
    let page_count = guarded(|| doc.get_pages().len()).unwrap_or(0);
    if page_count == 0 {
        return Err("PDF konnte nicht gelesen werden".into());
    }
    let mut pages = Vec::with_capacity(page_count);
    for n in 1..=page_count as u32 {
        // Eine unlesbare Seite bleibt leer, statt das Buch zu verwerfen.
        let lines = guarded(|| {
            let mut out = SizedLines::new();
            pdf_extract::output_doc_page(&doc, &mut out, n).ok()?;
            out.break_line();
            Some(out.lines)
        })
        .flatten()
        .unwrap_or_default();
        pages.push(lines);
    }
    let outline = guarded(|| outline_of(&doc)).unwrap_or_default();
    Ok(PdfBook { pages, outline })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PdfBook {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/buchimport")
            .join(name);
        read_pdf(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn reads_font_sizes_and_paragraph_gaps() {
        let book = fixture("kapitel-schriftgroesse.pdf");
        let first = &book.pages[0];
        let title = first
            .iter()
            .find(|l| l.text.trim() == "Der längste Dezember")
            .unwrap();
        let body = first
            .iter()
            .find(|l| l.text.starts_with("Im Jahr"))
            .unwrap();
        assert!((title.size - 20.0).abs() < 0.5, "{title:?}");
        assert!((body.size - 11.0).abs() < 0.5, "{body:?}");
        assert!(first.iter().any(|l| l.gap), "{first:?}");
        assert!(book.outline.is_empty());
        // Gleicher Text wie pdf-extract (die Zeilenbildung folgt ihm).
        let plain = pdf_extract::extract_text_from_mem_by_pages(
            &std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/buchimport/kapitel-schriftgroesse.pdf"),
            )
            .unwrap(),
        )
        .unwrap();
        let ours: Vec<String> = first.iter().map(|l| l.text.trim().to_string()).collect();
        let theirs: Vec<String> = plain[0]
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn reads_the_outline() {
        let book = fixture("kapitel-lesezeichen.pdf");
        let titles: Vec<&str> = book.outline.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Erstes Kapitel: Der Brief",
                "Zweites Kapitel: Der Bahnhof",
                "Drittes Kapitel: Heimkehr"
            ]
        );
        assert_eq!(book.outline[0].page, 0);
    }

    #[test]
    fn imports_the_short_story_like_the_screenshot() {
        let text = crate::ebook::pdf_book_to_text(&fixture("kurzgeschichte-kopfzeile.pdf"));
        assert!(text.starts_with("# Der längste Dezember\n"), "{text}");
        assert!(!text.contains("kostenlose Geschichten"), "{text}");
        assert!(text.contains("und erst am 23. kommt."), "{text}");
        let chapters = crate::ebook::pdf_book_to_text(&fixture("kapitel-schriftgroesse.pdf"));
        for h in ["\n## Der Brief\n", "\n## Der Bahnhof\n", "\n## Heimkehr\n"] {
            assert!(chapters.contains(h), "{h}: {chapters}");
        }
    }

    #[test]
    fn outline_ring_latin1_and_duplicates() {
        // Ring über /Next: endet, statt zu hängen; Titel ohne BOM in PDFDocEncoding.
        for name in ["lesezeichen-ring.pdf", "lesezeichen-latin1.pdf"] {
            let book = fixture(name);
            let titles: Vec<&str> = book.outline.iter().map(|o| o.title.as_str()).collect();
            assert_eq!(titles, ["Einführung", "Hauptteil"], "{name}");
            let text = crate::ebook::pdf_book_to_text(&book);
            assert!(text.contains("## Einführung\n"), "{name}: {text}");
            assert!(!text.contains('\u{FFFD}'), "{name}: {text}");
        }
        // Gleiche Titel bleiben einzeln (lopdf führt sie zusammen).
        let dup = fixture("lesezeichen-doppelt.pdf");
        let titles: Vec<(&str, usize)> = dup
            .outline
            .iter()
            .map(|o| (o.title.as_str(), o.page))
            .collect();
        assert_eq!(
            titles,
            [("Anmerkungen", 0), ("Anmerkungen", 1), ("Hauptteil", 1)]
        );
    }

    #[test]
    fn pdf_doc_encoding() {
        assert_eq!(
            pdf_text_string(b"Einf\xfchrung \x84 \x93"),
            "Einführung — ﬁ"
        );
        assert_eq!(pdf_text_string(b"\xfe\xff\x00\xe4\x00\xdf"), "äß");
        assert_eq!(pdf_text_string(b"\xef\xbb\xbf\xc3\xa4"), "ä");
    }

    #[test]
    fn paragraph_continues_across_the_page_break() {
        let text = crate::ebook::pdf_book_to_text(&fixture("kapitel-lesezeichen.pdf"));
        assert!(!text.contains("obwohl\n\n"), "{text}");
    }

    #[test]
    fn broken_pdf_is_an_error_not_a_panic() {
        assert!(read_pdf(b"%PDF-1.4 kaputt").is_err());
    }

    /// Echte Bücher (`tests/fixtures/buchimport/korpus_laden.py`), nur von Hand:
    /// `LVA_BUCHKORPUS=<ordner> cargo test --lib pdf_text::tests::korpus -- --ignored`.
    #[test]
    #[ignore]
    fn korpus() {
        let Ok(dir) = std::env::var("LVA_BUCHKORPUS") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let count =
            |text: &str, prefix: &str| text.lines().filter(|l| l.starts_with(prefix)).count();
        let grimm = crate::ebook::pdf_book_to_text(
            &read_pdf(&std::fs::read(dir.join("grimm.pdf")).unwrap()).unwrap(),
        );
        // 200 KHM, 10 Kinderlegenden, 40 entfernte Märchen.
        let tales = count(&grimm, "## KHM") + count(&grimm, "## KL") + count(&grimm, "## 18");
        assert!(tales >= 240, "nur {tales} Märchentitel");
        assert!(!grimm.contains("S E I T E"));
        let epub = |name: &str| {
            let f = std::fs::File::open(dir.join(name)).unwrap();
            crate::ebook::epub_to_markdown(&mut zip::ZipArchive::new(f).unwrap()).unwrap()
        };
        assert!(count(&epub("effi.epub"), "## ") >= 36);
        assert!(count(&epub("grimm-gutenberg.epub"), "## ") >= 62);
        assert!(count(&epub("kafka-verwandlung.epub"), "## ") >= 3);
    }
}
