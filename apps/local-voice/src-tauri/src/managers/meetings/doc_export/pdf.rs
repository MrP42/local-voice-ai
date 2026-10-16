//! PDF mit Briefkopf ueber `krilla` (MIT OR Apache-2.0): reines Rust, kein Browser, kein
//! Fenster -- dieselbe Ausgabe auf Windows und macOS und in Tests pruefbar. Der
//! Besprechungs-Export ueber WebView2 (`meetings/pdf.rs`) bleibt davon unberuehrt.
//!
//! Satz in zwei Durchgaengen: erst werden alle Seiten als Zeichenbefehle ([`Op`])
//! umbrochen (Zeilen-, Seiten- und Tabellenumbruch, Kopfzeile der Tabelle wird auf der
//! Folgeseite wiederholt), dann gezeichnet -- erst dann ist "Seite x von y" bekannt.
//! Zeilenumbruch: Flattersatz an Leerzeichen, Breiten aus den Vorschubwerten der Schrift
//! (`ttf-parser`); ein ueberlanges Wort wird zeichenweise getrennt. Keine Silbentrennung.
//!
//! Schriften (eingebettet, OFL-1.1, `assets/documents/`): IBM Plex Sans Regular, Bold,
//! Italic fuer den Text, Archivo Bold fuer Titel und Ueberschriften (Medienregel des
//! Design-Systems: Markenschriften dort, wo sie eingebettet werden).
//!
//! Ausgabe als PDF/A-2b; lehnt die Pruefung von krilla ab (z. B. ein Zeichen ohne Glyphe),
//! wird ein gewoehnliches PDF erzeugt und das geloggt -- der Export scheitert daran nicht.

use krilla::color::rgb;
use krilla::configure::{Archival, ConfigurationBuilder};
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::image::Image;
use krilla::metadata::{DateTime, Metadata};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::Fill;
use krilla::surface::Surface;
use krilla::text::{Font, TextDirection};
use krilla::{Document, SerializeSettings};

use super::{plain, title_of, Design, Item};
use crate::managers::letterhead::{parse_color, ImageKind, LogoPosition};
use crate::managers::meetings::export::Span;

const REGULAR: &[u8] = include_bytes!("../../../../assets/documents/IBMPlexSans-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../../assets/documents/IBMPlexSans-Bold.ttf");
const ITALIC: &[u8] = include_bytes!("../../../../assets/documents/IBMPlexSans-Italic.ttf");
const HEADING: &[u8] = include_bytes!("../../../../assets/documents/Archivo-Bold.ttf");

/// A4 in Punkt.
const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
const MM: f32 = 72.0 / 25.4;
const MARGIN_X: f32 = 22.0 * MM;
const CONTENT_W: f32 = PAGE_W - 2.0 * MARGIN_X;
/// Erste Textzeile unter dem Kopf mit Firma oder Logo ...
const CONTENT_TOP: f32 = 122.0;
/// ... und ohne beides (Briefkopf "Neutral"): kein leerer Kopfbereich.
const CONTENT_TOP_PLAIN: f32 = 64.0;

/// Linie unter dem Kopf und erste Textzeile: abhaengig davon, ob der Kopf etwas zeigt.
fn header_geometry(design: &Design<'_>) -> (f32, f32) {
    let lh = design.letterhead;
    let shows =
        design.logo.is_some() || !lh.firma.name.trim().is_empty() || !lh.firma.zeilen.is_empty();
    if shows {
        (96.0, CONTENT_TOP)
    } else {
        (40.0, CONTENT_TOP_PLAIN)
    }
}
/// Unterkante des Satzspiegels (darunter der Fuss).
const CONTENT_BOTTOM: f32 = PAGE_H - 74.0;

const BODY_SIZE: f32 = 10.0;
const BODY_LEADING: f32 = 14.5;

type Rgb = (u8, u8, u8);

/// Schriftschnitt eines Textstuecks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Face {
    Regular,
    Bold,
    Italic,
    Heading,
}

/// Ein Zeichenbefehl in Seitenkoordinaten (Ursprung oben links, y nach unten).
#[derive(Clone, Debug)]
enum Op {
    Text {
        x: f32,
        baseline: f32,
        face: Face,
        size: f32,
        color: Rgb,
        text: String,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Rgb,
    },
}

/// Breiten aus den Vorschubwerten der Schrift.
struct Metrics {
    faces: [ttf_parser::Face<'static>; 4],
}

impl Metrics {
    fn new() -> Result<Self, String> {
        let parse = |bytes: &'static [u8]| {
            ttf_parser::Face::parse(bytes, 0).map_err(|e| format!("pdf_font: {e}"))
        };
        Ok(Self {
            faces: [
                parse(REGULAR)?,
                parse(BOLD)?,
                parse(ITALIC)?,
                parse(HEADING)?,
            ],
        })
    }

    fn face(&self, face: Face) -> &ttf_parser::Face<'static> {
        &self.faces[face as usize]
    }

    fn width(&self, face: Face, size: f32, text: &str) -> f32 {
        let f = self.face(face);
        let upem = f32::from(f.units_per_em());
        let units: f32 = text
            .chars()
            .map(|c| {
                f.glyph_index(c)
                    .and_then(|g| f.glyph_hor_advance(g))
                    .map(f32::from)
                    .unwrap_or(upem * 0.5)
            })
            .sum();
        units * size / upem
    }
}

fn span_face(span: &Span, base: Face) -> Face {
    if base == Face::Heading {
        return Face::Heading;
    }
    if span.bold || base == Face::Bold {
        Face::Bold
    } else if span.italic {
        Face::Italic
    } else {
        Face::Regular
    }
}

/// Ein Stueck einer Zeile.
#[derive(Clone, Debug, PartialEq)]
struct Run {
    dx: f32,
    face: Face,
    text: String,
}

/// Umbruch von Textstuecken auf `width`: Liste der Zeilen, je Zeile die Stuecke mit
/// ihrem Versatz. Leerzeichen am Zeilenende entfallen.
fn wrap(m: &Metrics, spans: &[Span], base: Face, size: f32, width: f32) -> Vec<Vec<Run>> {
    // Woerter mit Schnitt und der Angabe, ob davor ein Leerzeichen stand.
    let mut words: Vec<(String, Face, bool)> = Vec::new();
    let mut pending_space = false;
    for span in spans {
        let face = span_face(span, base);
        let mut current = String::new();
        for c in span.text.chars() {
            if c.is_whitespace() {
                if !current.is_empty() {
                    words.push((std::mem::take(&mut current), face, pending_space));
                }
                pending_space = true;
            } else {
                if current.is_empty() && words.is_empty() {
                    pending_space = false;
                }
                current.push(c);
            }
        }
        if !current.is_empty() {
            words.push((current, face, pending_space));
            pending_space = false;
        }
    }
    let mut lines: Vec<Vec<Run>> = vec![Vec::new()];
    let mut x = 0.0f32;
    for (word, face, space_before) in words {
        let space = if space_before && x > 0.0 {
            m.width(face, size, " ")
        } else {
            0.0
        };
        let w = m.width(face, size, &word);
        if x > 0.0 && x + space + w > width {
            lines.push(Vec::new());
            x = 0.0;
        }
        let space = if x > 0.0 { space } else { 0.0 };
        if w > width {
            // Ueberlanges Wort (URL, Kennung): zeichenweise trennen.
            let mut chunk = String::new();
            for c in word.chars() {
                let next = format!("{chunk}{c}");
                if !chunk.is_empty() && x + m.width(face, size, &next) > width {
                    push_run(
                        lines.last_mut().unwrap(),
                        x,
                        face,
                        std::mem::take(&mut chunk),
                    );
                    lines.push(Vec::new());
                    x = 0.0;
                }
                chunk.push(c);
            }
            let cw = m.width(face, size, &chunk);
            push_run(lines.last_mut().unwrap(), x, face, chunk);
            x += cw;
            continue;
        }
        let line = lines.last_mut().unwrap();
        // Gleicher Schnitt wie das letzte Stueck: anhaengen (weniger Zeichenbefehle).
        match line.last_mut() {
            Some(last) if last.face == face && x > 0.0 => {
                if space > 0.0 {
                    last.text.push(' ');
                }
                last.text.push_str(&word);
            }
            _ => line.push(Run {
                dx: x + space,
                face,
                text: word,
            }),
        }
        x += space + w;
    }
    if lines.len() > 1 && lines.last().is_some_and(Vec::is_empty) {
        lines.pop();
    }
    lines
}

fn push_run(line: &mut Vec<Run>, dx: f32, face: Face, text: String) {
    if !text.is_empty() {
        line.push(Run { dx, face, text });
    }
}

fn rgb_of(hex: &str) -> Rgb {
    parse_color(hex).unwrap_or((0, 0, 0))
}

/// Farben des Briefkopfs, aufgeloest.
struct Palette {
    accent: Rgb,
    accent_text: Rgb,
    text: Rgb,
    title: Rgb,
    muted: Rgb,
    line: Rgb,
}

/// Der Umbruch in Seiten.
struct Layout<'m> {
    m: &'m Metrics,
    p: Palette,
    pages: Vec<Vec<Op>>,
    y: f32,
    /// Erste Textzeile einer Seite.
    top: f32,
}

impl<'m> Layout<'m> {
    fn new(m: &'m Metrics, p: Palette, top: f32) -> Self {
        Self {
            m,
            p,
            pages: vec![Vec::new()],
            y: top,
            top,
        }
    }

    fn ops(&mut self) -> &mut Vec<Op> {
        self.pages.last_mut().expect("mindestens eine Seite")
    }

    fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = self.top;
    }

    /// Platz fuer `h` Punkt, sonst neue Seite (nie auf einer leeren Seite).
    fn ensure(&mut self, h: f32) {
        if self.y + h > CONTENT_BOTTOM && self.y > self.top {
            self.new_page();
        }
    }

    fn lines(&mut self, lines: &[Vec<Run>], x: f32, size: f32, leading: f32, color: Rgb) {
        for line in lines {
            self.ensure(leading);
            let baseline = self.y + size;
            for run in line {
                let op = Op::Text {
                    x: x + run.dx,
                    baseline,
                    face: run.face,
                    size,
                    color,
                    text: run.text.clone(),
                };
                self.ops().push(op);
            }
            self.y += leading;
        }
    }

    fn heading(&mut self, level: u8, spans: &[Span]) {
        let (size, leading, before, after, face) = match level {
            1 => (18.0, 23.0, 0.0, 8.0, Face::Heading),
            2 => (12.5, 17.0, 12.0, 3.0, Face::Heading),
            _ => (10.5, 15.0, 8.0, 2.0, Face::Bold),
        };
        let lines = wrap(self.m, spans, face, size, CONTENT_W);
        // Ueberschrift nie allein am Seitenende: Platz fuer sie und zwei Zeilen Text.
        self.ensure(before + leading * lines.len() as f32 + 2.0 * BODY_LEADING);
        if self.y > self.top {
            self.y += before;
        }
        let color = self.p.title;
        self.lines(&lines, MARGIN_X, size, leading, color);
        self.y += after;
    }

    fn paragraph(&mut self, spans: &[Span]) {
        let lines = wrap(self.m, spans, Face::Regular, BODY_SIZE, CONTENT_W);
        let color = self.p.text;
        self.lines(&lines, MARGIN_X, BODY_SIZE, BODY_LEADING, color);
        self.y += 4.0;
    }

    fn bullet(&mut self, spans: &[Span], done: Option<bool>) {
        let indent = 14.0;
        let lines = wrap(self.m, spans, Face::Regular, BODY_SIZE, CONTENT_W - indent);
        self.ensure(BODY_LEADING);
        let top = self.y;
        match done {
            None => {
                let (x, color) = (MARGIN_X + 3.0, self.p.text);
                self.ops().push(Op::Text {
                    x,
                    baseline: top + BODY_SIZE,
                    face: Face::Regular,
                    size: BODY_SIZE,
                    color,
                    text: "\u{2022}".into(),
                });
            }
            Some(checked) => {
                // Kaestchen: Rahmen aus vier Linien, erledigt gefuellt.
                let (x, y, s) = (MARGIN_X + 1.0, top + 3.0, 7.5);
                let (line, accent) = (self.p.muted, self.p.accent);
                if checked {
                    self.ops().push(Op::Rect {
                        x,
                        y,
                        w: s,
                        h: s,
                        color: accent,
                    });
                }
                for (rx, ry, rw, rh) in [
                    (x, y, s, 0.7),
                    (x, y + s - 0.7, s, 0.7),
                    (x, y, 0.7, s),
                    (x + s - 0.7, y, 0.7, s),
                ] {
                    self.ops().push(Op::Rect {
                        x: rx,
                        y: ry,
                        w: rw,
                        h: rh,
                        color: line,
                    });
                }
            }
        }
        let color = self.p.text;
        self.lines(&lines, MARGIN_X + indent, BODY_SIZE, BODY_LEADING, color);
        self.y += 2.0;
    }

    fn note(&mut self, spans: &[Span]) {
        let size = 9.0;
        let leading = 13.0;
        let lines = wrap(self.m, spans, Face::Regular, size, CONTENT_W - 12.0);
        self.y += 4.0;
        self.ensure(leading * lines.len().min(3) as f32);
        let top = self.y;
        let color = self.p.muted;
        self.lines(&lines, MARGIN_X + 12.0, size, leading, color);
        let accent = self.p.accent;
        // Der Balken gehoert zur Seite, auf der der Hinweis beginnt.
        let h = (self.y - top).max(leading);
        let page = self.pages.len() - 1;
        if self.y >= top {
            self.pages[page].push(Op::Rect {
                x: MARGIN_X,
                y: top,
                w: 3.0,
                h,
                color: accent,
            });
        }
        self.y += 4.0;
    }

    fn table(&mut self, header: &Option<Vec<Vec<Span>>>, rows: &[Vec<Vec<Span>>]) {
        let size = 9.0;
        let leading = 12.5;
        let pad = 4.0;
        let cols = header
            .as_ref()
            .map(Vec::len)
            .unwrap_or(0)
            .max(rows.iter().map(Vec::len).max().unwrap_or(0))
            .max(1);
        // Spaltenbreiten nach dem laengsten Inhalt, mindestens 45 pt, auf die Breite verteilt.
        let mut natural = vec![45.0f32; cols];
        for row in header.iter().chain(rows.iter()) {
            for (i, cell) in row.iter().enumerate().take(cols) {
                let w = self.m.width(Face::Bold, size, &plain(cell)) + 2.0 * pad;
                natural[i] = natural[i].max(w.min(CONTENT_W * 0.6));
            }
        }
        let total: f32 = natural.iter().sum();
        let widths: Vec<f32> = natural.iter().map(|w| w / total * CONTENT_W).collect();
        let layout_row = |m: &Metrics, row: &Vec<Vec<Span>>, face: Face| -> Vec<Vec<Vec<Run>>> {
            (0..cols)
                .map(|i| {
                    row.get(i)
                        .map(|c| wrap(m, c, face, size, widths[i] - 2.0 * pad))
                        .unwrap_or_default()
                })
                .collect()
        };
        let head = header.as_ref().map(|h| layout_row(self.m, h, Face::Bold));
        let height = |cells: &Vec<Vec<Vec<Run>>>| -> f32 {
            cells.iter().map(Vec::len).max().unwrap_or(1).max(1) as f32 * leading + 2.0 * pad
        };
        self.y += 2.0;
        let draw = |me: &mut Self, cells: &Vec<Vec<Vec<Run>>>, is_head: bool| {
            let h = height(cells);
            let top = me.y;
            if is_head {
                let accent = me.p.accent;
                me.ops().push(Op::Rect {
                    x: MARGIN_X,
                    y: top,
                    w: CONTENT_W,
                    h,
                    color: accent,
                });
            }
            let color = if is_head { me.p.accent_text } else { me.p.text };
            let mut x = MARGIN_X;
            for (i, lines) in cells.iter().enumerate() {
                for (n, line) in lines.iter().enumerate() {
                    for run in line {
                        let op = Op::Text {
                            x: x + pad + run.dx,
                            baseline: top + pad + size + n as f32 * leading,
                            face: run.face,
                            size,
                            color,
                            text: run.text.clone(),
                        };
                        me.ops().push(op);
                    }
                }
                x += widths[i];
            }
            let line = me.p.line;
            me.ops().push(Op::Rect {
                x: MARGIN_X,
                y: top + h - 0.6,
                w: CONTENT_W,
                h: 0.6,
                color: line,
            });
            me.y += h;
        };
        if let Some(head) = &head {
            self.ensure(height(head) + leading + 2.0 * pad);
            draw(self, head, true);
        }
        for row in rows {
            let cells = layout_row(self.m, row, Face::Regular);
            let h = height(&cells);
            if self.y + h > CONTENT_BOTTOM && self.y > self.top {
                self.new_page();
                if let Some(head) = &head {
                    draw(self, head, true);
                }
            }
            draw(self, &cells, false);
        }
        self.y += 8.0;
    }
}

fn fill(surface: &mut Surface<'_>, color: Rgb) {
    surface.set_fill(Some(Fill {
        paint: rgb::Color::new(color.0, color.1, color.2).into(),
        opacity: NormalizedF32::ONE,
        rule: Default::default(),
    }));
}

fn rect(surface: &mut Surface<'_>, x: f32, y: f32, w: f32, h: f32, color: Rgb) {
    let Some(r) = Rect::from_xywh(x, y, w.max(0.1), h.max(0.1)) else {
        return;
    };
    let mut pb = PathBuilder::new();
    pb.push_rect(r);
    if let Some(path) = pb.finish() {
        fill(surface, color);
        surface.draw_path(&path);
    }
}

struct Fonts {
    faces: [Font; 4],
}

impl Fonts {
    fn new() -> Result<Self, String> {
        let load = |bytes: &'static [u8]| {
            Font::new(bytes.into(), 0).ok_or_else(|| "pdf_font: unreadable".to_string())
        };
        Ok(Self {
            faces: [load(REGULAR)?, load(BOLD)?, load(ITALIC)?, load(HEADING)?],
        })
    }
}

fn text(
    surface: &mut Surface<'_>,
    fonts: &Fonts,
    face: Face,
    size: f32,
    x: f32,
    baseline: f32,
    color: Rgb,
    s: &str,
) {
    fill(surface, color);
    surface.draw_text(
        Point::from_xy(x, baseline),
        fonts.faces[face as usize].clone(),
        size,
        s,
        false,
        TextDirection::Auto,
    );
}

/// Kopf (Akzentbalken, Logo, Firma, Linie) und Fuss (Zeilen, Seitenzahl) einer Seite.
fn chrome(
    surface: &mut Surface<'_>,
    fonts: &Fonts,
    m: &Metrics,
    design: &Design<'_>,
    p: &Palette,
    logo: Option<&Image>,
    page: usize,
    pages: usize,
) {
    let lh = design.letterhead;
    rect(surface, 0.0, 0.0, PAGE_W, 7.0, p.accent);
    let right = lh
        .logo
        .as_ref()
        .is_some_and(|l| l.position == LogoPosition::Rechts);
    let mut text_x = MARGIN_X;
    if let (Some(image), Some(spec), Some(data)) = (logo, lh.logo.as_ref(), design.logo) {
        let w = spec.breite_mm * MM;
        let h = w * data.height_px as f32 / data.width_px.max(1) as f32;
        let h = h.min(52.0);
        let w = h * data.width_px as f32 / data.height_px.max(1) as f32;
        let x = if right {
            PAGE_W - MARGIN_X - w
        } else {
            MARGIN_X
        };
        let y = 30.0 + (52.0 - h) / 2.0;
        if let Some(size) = Size::from_wh(w, h) {
            surface.push_transform(&Transform::from_translate(x, y));
            surface.draw_image(image.clone(), size);
            surface.pop();
        }
        if !right {
            text_x = MARGIN_X + w + 12.0;
        }
    }
    let mut baseline = 50.0;
    if !lh.firma.name.trim().is_empty() {
        text(
            surface,
            fonts,
            Face::Heading,
            13.0,
            text_x,
            baseline,
            p.title,
            &lh.firma.name,
        );
        baseline += 14.0;
    }
    for line in &lh.firma.zeilen {
        text(
            surface,
            fonts,
            Face::Regular,
            8.5,
            text_x,
            baseline,
            p.muted,
            line,
        );
        baseline += 11.0;
    }
    rect(
        surface,
        MARGIN_X,
        header_geometry(design).0,
        CONTENT_W,
        0.8,
        p.line,
    );

    // Fuss
    rect(surface, MARGIN_X, PAGE_H - 56.0, CONTENT_W, 0.6, p.line);
    let mut fy = PAGE_H - 42.0;
    for line in &lh.fusszeile.zeilen {
        text(
            surface,
            fonts,
            Face::Regular,
            8.0,
            MARGIN_X,
            fy,
            p.muted,
            line,
        );
        fy += 10.0;
    }
    if lh.fusszeile.seitenzahl {
        let label = format!("Seite {page} von {pages}");
        let w = m.width(Face::Regular, 8.0, &label);
        text(
            surface,
            fonts,
            Face::Regular,
            8.0,
            PAGE_W - MARGIN_X - w,
            PAGE_H - 42.0,
            p.muted,
            &label,
        );
    }
}

fn palette(design: &Design<'_>) -> Palette {
    let f = &design.letterhead.farben;
    Palette {
        accent: rgb_of(&f.akzent),
        accent_text: rgb_of(&f.akzent_text),
        text: rgb_of(&f.text),
        title: rgb_of(&f.titel),
        muted: rgb_of(&f.gedaempft),
        line: rgb_of(&f.linie),
    }
}

/// Seiten umbrechen.
fn layout(items: &[Item], m: &Metrics, p: Palette, top: f32) -> Vec<Vec<Op>> {
    let mut l = Layout::new(m, p, top);
    for item in items {
        match item {
            Item::Heading { level, spans } => l.heading(*level, spans),
            Item::Paragraph(spans) => l.paragraph(spans),
            Item::Bullet { spans, done } => l.bullet(spans, *done),
            Item::Table { header, rows } => l.table(header, rows),
            Item::Note(spans) => l.note(spans),
        }
    }
    l.pages
}

fn build(items: &[Item], design: &Design<'_>, archival: bool) -> Result<Vec<u8>, String> {
    let m = Metrics::new()?;
    let fonts = Fonts::new()?;
    let pages = layout(items, &m, palette(design), header_geometry(design).1);
    let logo = match design.logo {
        Some(data) => {
            let bytes: Vec<u8> = data.bytes.clone();
            let image = match data.kind {
                ImageKind::Png => Image::from_png(bytes.into(), false),
                ImageKind::Jpeg => Image::from_jpeg(bytes.into(), false),
            }
            .map_err(|e| format!("pdf_logo: {e}"))?;
            Some(image)
        }
        None => None,
    };
    let mut settings = SerializeSettings::default();
    if archival {
        settings.configuration = ConfigurationBuilder::new()
            .with_archival_validator(Archival::A2_B)
            .finish()
            .map_err(|e| format!("pdf_config: {e:?}"))?;
    }
    let mut document = Document::new_with(settings);
    let title = title_of(items).unwrap_or_else(|| "Dokument".to_string());
    document.set_metadata(
        Metadata::new()
            .title(title)
            .language("de".to_string())
            .creator("Local Voice AI".to_string())
            .producer("Local Voice AI (krilla)".to_string())
            // PDF/A verlangt ein Erstellungsdatum (UTC).
            .creation_date(now_utc()),
    );
    let total = pages.len();
    let p = palette(design);
    for (n, ops) in pages.iter().enumerate() {
        let settings = PageSettings::from_wh(PAGE_W, PAGE_H).ok_or("pdf_page")?;
        let mut page = document.start_page_with(settings);
        let mut surface = page.surface();
        chrome(
            &mut surface,
            &fonts,
            &m,
            design,
            &p,
            logo.as_ref(),
            n + 1,
            total,
        );
        for op in ops {
            match op {
                Op::Text {
                    x,
                    baseline,
                    face,
                    size,
                    color,
                    text: s,
                } => text(&mut surface, &fonts, *face, *size, *x, *baseline, *color, s),
                Op::Rect { x, y, w, h, color } => rect(&mut surface, *x, *y, *w, *h, *color),
            }
        }
        surface.finish();
        page.finish();
    }
    document.finish().map_err(|e| format!("pdf_failed: {e:?}"))
}

fn now_utc() -> DateTime {
    use chrono::{Datelike, Timelike};
    let now = chrono::Utc::now();
    DateTime::new(now.year().clamp(0, 9999) as u16)
        .month(now.month() as u8)
        .day(now.day() as u8)
        .hour(now.hour() as u8)
        .minute(now.minute() as u8)
        .second(now.second() as u8)
        .utc_offset_hour(0)
        .utc_offset_minute(0)
}

/// Das PDF als Bytes (PDF/A-2b, sonst gewoehnliches PDF).
pub fn render(items: &[Item], design: &Design<'_>) -> Result<Vec<u8>, String> {
    match build(items, design, true) {
        Ok(bytes) => Ok(bytes),
        Err(e) => {
            log::warn!(
                "PDF/A-2b nicht moeglich ({}), gewoehnliches PDF",
                e.chars().take(200).collect::<String>()
            );
            build(items, design, false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;
    use crate::managers::letterhead::{builtins, Store};

    fn spans(text: &str) -> Vec<Span> {
        crate::managers::meetings::export::parse_spans(text)
    }

    #[test]
    fn wrapping_respects_the_width_and_breaks_overlong_words() {
        let m = Metrics::new().unwrap();
        let text = "Das ist ein Satz, der deutlich breiter ist als die schmale Spalte.";
        let lines = wrap(&m, &spans(text), Face::Regular, 10.0, 120.0);
        assert!(lines.len() >= 3, "{lines:?}");
        for line in &lines {
            let end = line
                .last()
                .map(|r| r.dx + m.width(r.face, 10.0, &r.text))
                .unwrap();
            assert!(end <= 120.5, "Zeile zu breit: {end}");
        }
        let joined: String = lines
            .iter()
            .map(|l| {
                l.iter()
                    .map(|r| r.text.clone())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(joined, text);
        let long = wrap(&m, &spans(&"x".repeat(200)), Face::Regular, 10.0, 100.0);
        assert!(long.len() > 1);
        // Fett und normal in einer Zeile bleiben getrennte Stuecke.
        let mixed = wrap(&m, &spans("**Datum:** heute"), Face::Regular, 10.0, 300.0);
        assert_eq!(mixed[0].len(), 2);
        assert_eq!(mixed[0][0].face, Face::Bold);
    }

    fn extract(bytes: &[u8]) -> String {
        pdf_extract::extract_text_from_mem(bytes).unwrap()
    }

    #[test]
    fn a_branded_pdf_has_text_header_footer_and_page_numbers() {
        let lh = builtins().into_iter().find(|b| b.id == "wai").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let logo = Store::new(dir.path().to_path_buf()).logo(&lh).unwrap();
        let design = Design {
            letterhead: &lh,
            logo: logo.as_ref(),
        };
        let mut md = String::from(
            "# Management Summary: Jour fixe\n\n**Datum:** 2026-10-13\n\n## Kernergebnisse\n\n",
        );
        for i in 0..80 {
            md.push_str(&format!(
                "- Ergebnis {i}: Go-Live am 1. September, Budget bleibt unverändert.\n"
            ));
        }
        md.push_str("\n| Sprecher | Anteil |\n|---|---|\n| Ich | 60 % |\n\n> **Hinweis:** Länger als vorgesehen.\n");
        // Direkt als PDF/A-2b: scheitert die Pruefung, nennt der Test den Grund.
        let bytes = build(&parse(&md), &design, true).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        let text = extract(&bytes);
        assert!(text.contains("Management Summary"), "{text}");
        assert!(text.contains("Ergebnis 79"));
        assert!(text.contains("Wolff Applied AI"));
        assert!(text.contains("Seite 1 von"));
        assert!(text.contains("Seite 2 von"), "mehrseitig");
        assert!(text.contains("unverändert"), "Umlaute");
        let raw = String::from_utf8_lossy(&bytes);
        assert!(
            raw.contains("FontFile2") || raw.contains("FontFile3"),
            "Schrift eingebettet"
        );
        assert!(raw.contains("pdfaid"), "PDF/A-Kennung");
    }

    #[test]
    fn a_long_table_continues_on_the_next_page_with_its_header() {
        let lh = builtins().remove(0);
        let design = Design {
            letterhead: &lh,
            logo: None,
        };
        let mut md = String::from("# T\n\n| Was | Wer | Bis |\n|---|---|---|\n");
        for i in 0..90 {
            md.push_str(&format!(
                "| Aufgabe {i} mit etwas Text | Anna | Freitag |\n"
            ));
        }
        let items = parse(&md);
        let m = Metrics::new().unwrap();
        let pages = layout(&items, &m, palette(&design), header_geometry(&design).1);
        assert!(pages.len() >= 2);
        let heads_on_page_2 = pages[1]
            .iter()
            .filter(|op| matches!(op, Op::Text { text, .. } if text == "Was"))
            .count();
        assert_eq!(
            heads_on_page_2, 1,
            "Tabellenkopf auf der Folgeseite wiederholt"
        );
        for page in &pages {
            for op in page {
                if let Op::Text { baseline, .. } = op {
                    assert!(
                        *baseline <= CONTENT_BOTTOM + 1.0,
                        "Text im Fuss: {baseline}"
                    );
                }
            }
        }
        assert!(render(&items, &design).unwrap().starts_with(b"%PDF-"));
    }
}
