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

/// Lesezeichen des PDFs, wenn vorhanden (Seiten 0-basiert, nur gültige).
fn outline_of(doc: &pdf_extract::Document, page_count: usize) -> Vec<OutlineEntry> {
    let Ok(toc) = doc.get_toc() else {
        return Vec::new();
    };
    toc.toc
        .into_iter()
        .filter(|t| t.page >= 1 && t.page <= page_count)
        .map(|t| OutlineEntry {
            level: t.level.saturating_sub(1),
            title: t.title.split_whitespace().collect::<Vec<_>>().join(" "),
            page: t.page - 1,
        })
        .filter(|t| t.title.chars().any(char::is_alphanumeric))
        .collect()
}

/// PDF laden: Zeilen mit Schriftgröße je Seite und Lesezeichen. pdf-extract
/// kann bei kaputten PDFs in Panik geraten; das wird zur Fehlermeldung.
pub fn read_pdf(bytes: &[u8]) -> Result<PdfBook, String> {
    let bytes = bytes.to_vec();
    std::panic::catch_unwind(move || -> Result<PdfBook, String> {
        let mut doc = pdf_extract::Document::load_mem(&bytes).map_err(|e| e.to_string())?;
        if doc.is_encrypted() {
            // Nur „offene“ PDFs mit leerem Benutzerpasswort (wie pdf-extract).
            doc.decrypt("").map_err(|e| e.to_string())?;
        }
        let page_count = doc.get_pages().len();
        let mut pages = Vec::with_capacity(page_count);
        for n in 1..=page_count as u32 {
            let mut out = SizedLines::new();
            // Eine unlesbare Seite bleibt leer, statt das Buch zu verwerfen.
            if pdf_extract::output_doc_page(&doc, &mut out, n).is_err() {
                pages.push(Vec::new());
                continue;
            }
            out.break_line();
            pages.push(out.lines);
        }
        let outline = outline_of(&doc, page_count);
        Ok(PdfBook { pages, outline })
    })
    .unwrap_or_else(|_| Err("PDF konnte nicht gelesen werden".into()))
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
