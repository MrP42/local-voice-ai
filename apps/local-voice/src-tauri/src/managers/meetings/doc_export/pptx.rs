//! PowerPoint (.pptx) aus einem Dokument (Entwurf 2026-10-13, Etappe 3): handgeschriebenes
//! OOXML ueber `zip`, im Design des Briefkopfs, deterministisch und ohne Modellaufruf.
//!
//! Folienaufbau ([`plan`]):
//! - Titelfolie: Dokumenttyp, Besprechung, Kopfzeilen des Dokuments (Datum, Dauer, ...).
//! - je Hauptabschnitt (`##`) eine Folie: Kernaussage je Punkt = sein erster Satz (nie
//!   mitten im Satz gekuerzt); Aufgaben (Punkte mit "Wer/Bis") als Tabelle Was/Wer/Bis,
//!   Abschnitte "Entscheidungen" als nummerierte Tabelle, Markdown-Tabellen als Tabelle.
//! - Was nicht auf eine Folie passt (geschaetzte Zeilen, hoechstens 6 Punkte bzw. Zeilen),
//!   geht auf eine Folgefolie "(Forts.)"; es wird nichts abgeschnitten.
//! - Schlussfolie.
//! - Sprechernotizen: der ausfuehrliche Text des Abschnitts (alle Punkte vollstaendig).
//!
//! Gestaltung: 16:9, Akzentbalken oben, Logo rechts oben, Titel in der Titelfarbe, Fuss mit
//! Fusszeile des Briefkopfs und Foliennummer; Schrift Segoe UI (wie Word).

use std::io::Write;

use super::{plain, Design, Item};
use crate::managers::letterhead::{parse_color, ImageKind};
use crate::managers::meetings::export::{xml_escape, Span};

const SLIDE_W: i64 = 12_192_000;
const SLIDE_H: i64 = 6_858_000;
const EMU_IN: i64 = 914_400;
const MARGIN_X: i64 = 548_640; // 0,6 in
const BODY_TOP: i64 = 1_463_040; // 1,6 in
const BODY_H: i64 = 4_754_880; // 5,2 in
const FONT: &str = "Segoe UI";

/// Hoechstens so viele Punkte oder Tabellenzeilen je Folie.
const MAX_POINTS: usize = 6;
/// Geschaetzte Textzeilen je Folie (18 pt auf 12 in Breite) ...
const MAX_LINES: usize = 11;
/// ... bei so vielen Zeichen je Zeile (vorsichtig geschaetzt).
const CHARS_PER_LINE: usize = 82;

/// Ein Punkt auf einer Folie: Kernaussage und vollstaendiger Text (Notizen).
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub short: String,
    pub full: String,
    /// Absatz statt Aufzaehlung (ein einzelner Fliesstext).
    pub paragraph: bool,
}

/// Was eine Folie zeigt.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Title {
        title: String,
        subtitle: String,
        lines: Vec<String>,
    },
    Points(Vec<Point>),
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Closing {
        title: String,
        lines: Vec<String>,
    },
}

/// Eine Folie samt Sprechernotiz.
#[derive(Clone, Debug, PartialEq)]
pub struct Slide {
    pub title: String,
    pub body: Body,
    pub notes: String,
}

/// Erster Satz eines Textes (bis `.`, `!`, `?` vor einem Leerzeichen), sonst der ganze Text.
pub fn first_sentence(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len().saturating_sub(1) {
        if matches!(chars[i], '.' | '!' | '?') && chars[i + 1] == ' ' {
            // "z. B." / "1. September": ein Punkt nach einer Ziffer oder einem einzelnen
            // Buchstaben beendet keinen Satz.
            let prev = if i > 0 { chars[i - 1] } else { ' ' };
            let before = if i > 1 { chars[i - 2] } else { ' ' };
            if chars[i] == '.'
                && (prev.is_ascii_digit() || (prev.is_alphabetic() && !before.is_alphabetic()))
            {
                continue;
            }
            return chars[..=i].iter().collect::<String>().trim().to_string();
        }
    }
    text.trim().to_string()
}

/// "Text (Wer: Anna, Bis: Freitag)" -> (Text, Wer, Bis).
pub fn split_task(text: &str) -> Option<(String, String, String)> {
    let text = text.trim();
    let open = text.rfind('(')?;
    let inner = text[open + 1..].strip_suffix(')')?;
    if !(inner.starts_with("Wer:") || inner.starts_with("Bis:")) {
        return None;
    }
    let (mut who, mut due) = (String::new(), String::new());
    for part in inner.split(", ") {
        if let Some(v) = part.strip_prefix("Wer:") {
            who = v.trim().to_string();
        } else if let Some(v) = part.strip_prefix("Bis:") {
            due = v.trim().to_string();
        }
    }
    Some((text[..open].trim().to_string(), who, due))
}

fn lines_of(text: &str, chars_per_line: usize) -> usize {
    text.chars().count().div_ceil(chars_per_line.max(10)).max(1)
}

/// Punkte auf Folien verteilen: hoechstens [`MAX_POINTS`] und [`MAX_LINES`] je Folie.
fn chunk_points(points: Vec<Point>) -> Vec<Vec<Point>> {
    let mut slides: Vec<Vec<Point>> = vec![Vec::new()];
    let mut lines = 0;
    for p in points {
        let need = lines_of(&p.short, CHARS_PER_LINE);
        let current = slides.last_mut().expect("eine Folie");
        if !current.is_empty() && (current.len() >= MAX_POINTS || lines + need > MAX_LINES) {
            slides.push(Vec::new());
            lines = 0;
        }
        slides.last_mut().expect("eine Folie").push(p);
        lines += need;
    }
    slides
}

/// Tabellenzeilen auf Folien verteilen (Kopf auf jeder Folie).
fn chunk_rows(rows: Vec<Vec<String>>, widths: &[usize]) -> Vec<Vec<Vec<String>>> {
    let mut out: Vec<Vec<Vec<String>>> = vec![Vec::new()];
    let mut lines = 1; // Kopfzeile
    for row in rows {
        let need = row
            .iter()
            .zip(widths)
            .map(|(c, w)| lines_of(c, *w))
            .max()
            .unwrap_or(1);
        let current = out.last_mut().expect("eine Folie");
        if !current.is_empty() && (current.len() >= MAX_POINTS || lines + need > MAX_LINES) {
            out.push(Vec::new());
            lines = 1;
        }
        out.last_mut().expect("eine Folie").push(row);
        lines += need;
    }
    out
}

/// Zeichen je Zeile einer Tabellenspalte nach ihrem Breitenanteil.
fn col_chars(shares: &[f64]) -> Vec<usize> {
    shares
        .iter()
        .map(|s| ((CHARS_PER_LINE as f64 * 1.15) * s).floor().max(8.0) as usize)
        .collect()
}

/// Abschnitt mit getroffenen Entscheidungen ("Entscheidungen", "Beschluesse"), nicht
/// "Entscheidungsbedarf" (offene Fragen, beobachtet am 13.10.2026).
pub fn is_decisions(title: &str) -> bool {
    let t = title.trim().to_lowercase();
    ["entscheidungen", "beschlüsse", "beschluesse"]
        .iter()
        .any(|w| t == *w || t.starts_with(&format!("{w} ")))
}

fn continued(title: &str, n: usize) -> String {
    if n == 0 {
        title.to_string()
    } else {
        format!("{title} (Forts.)")
    }
}

/// Ein Abschnitt (`##` bis zum naechsten `##`) als Folien.
fn section_slides(title: &str, items: &[&Item]) -> Vec<Slide> {
    let notes = items
        .iter()
        .filter_map(|i| match i {
            Item::Paragraph(s) | Item::Note(s) => Some(plain(s)),
            Item::Bullet { spans, .. } => Some(format!("- {}", plain(spans))),
            Item::Heading { spans, .. } => Some(plain(spans)),
            Item::Table { header, rows } => Some(
                header
                    .iter()
                    .chain(rows.iter())
                    .map(|r| r.iter().map(|c| plain(c)).collect::<Vec<_>>().join(" | "))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let bullets: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            Item::Bullet { spans, .. } => Some(plain(spans)),
            _ => None,
        })
        .collect();
    let mut slides = Vec::new();
    let mut push = |body: Body, n: usize| {
        slides.push(Slide {
            title: continued(title, n),
            body,
            notes: notes.clone(),
        })
    };

    // Aufgaben: Punkte mit "Wer/Bis".
    let tasks: Vec<(String, String, String)> =
        bullets.iter().filter_map(|b| split_task(b)).collect();
    if !tasks.is_empty() {
        let mut rows: Vec<Vec<String>> = Vec::new();
        for b in &bullets {
            match split_task(b) {
                Some((what, who, due)) => rows.push(vec![what, who, due]),
                None => rows.push(vec![b.clone(), String::new(), String::new()]),
            }
        }
        let widths = col_chars(&[0.62, 0.2, 0.18]);
        for (n, chunk) in chunk_rows(rows, &widths).into_iter().enumerate() {
            push(
                Body::Table {
                    header: vec!["Was".into(), "Wer".into(), "Bis".into()],
                    rows: chunk,
                },
                n,
            );
        }
        return slides;
    }
    // Entscheidungen: nummerierte Tabelle.
    if is_decisions(title) && !bullets.is_empty() {
        let rows: Vec<Vec<String>> = bullets
            .iter()
            .enumerate()
            .map(|(i, b)| vec![(i + 1).to_string(), b.clone()])
            .collect();
        let widths = col_chars(&[0.08, 0.92]);
        for (n, chunk) in chunk_rows(rows, &widths).into_iter().enumerate() {
            push(
                Body::Table {
                    header: vec!["#".into(), "Entscheidung".into()],
                    rows: chunk,
                },
                n,
            );
        }
        return slides;
    }
    // Markdown-Tabellen und Punkte in Reihenfolge.
    let mut n = 0;
    let mut points: Vec<Point> = Vec::new();
    let flush = |points: &mut Vec<Point>, n: &mut usize, push: &mut dyn FnMut(Body, usize)| {
        if points.is_empty() {
            return;
        }
        for chunk in chunk_points(std::mem::take(points)) {
            push(Body::Points(chunk), *n);
            *n += 1;
        }
    };
    for item in items {
        match item {
            Item::Bullet { spans, .. } => {
                let full = plain(spans);
                points.push(Point {
                    short: first_sentence(&full),
                    full,
                    paragraph: false,
                });
            }
            Item::Paragraph(spans) | Item::Heading { spans, .. } => {
                let full = plain(spans);
                points.push(Point {
                    short: full.clone(),
                    full,
                    paragraph: true,
                });
            }
            Item::Table { header, rows } => {
                flush(&mut points, &mut n, &mut push);
                let header: Vec<String> = header
                    .as_ref()
                    .map(|h| h.iter().map(|c| plain(c)).collect())
                    .unwrap_or_default();
                let rows: Vec<Vec<String>> = rows
                    .iter()
                    .map(|r| r.iter().map(|c| plain(c)).collect())
                    .collect();
                let cols = header
                    .len()
                    .max(rows.iter().map(Vec::len).max().unwrap_or(1))
                    .max(1);
                let widths = col_chars(&vec![1.0 / cols as f64; cols]);
                for chunk in chunk_rows(rows, &widths) {
                    push(
                        Body::Table {
                            header: header.clone(),
                            rows: chunk,
                        },
                        n,
                    );
                    n += 1;
                }
            }
            Item::Note(_) => {}
        }
    }
    flush(&mut points, &mut n, &mut push);
    slides
}

/// Das Dokument als Folienplan (rein, ohne Datei).
pub fn plan(items: &[Item]) -> Vec<Slide> {
    let heading = super::title_of(items).unwrap_or_else(|| "Dokument".to_string());
    let (title, subtitle) = match heading.split_once(": ") {
        Some((kind, meeting)) => (kind.trim().to_string(), meeting.trim().to_string()),
        None => (heading.clone(), String::new()),
    };
    let first_section = items
        .iter()
        .position(|i| matches!(i, Item::Heading { level: 2, .. }))
        .unwrap_or(items.len());
    let head_lines: Vec<String> = items[..first_section]
        .iter()
        .filter_map(|i| match i {
            Item::Paragraph(s) => Some(plain(s)),
            _ => None,
        })
        .collect();
    let mut slides = vec![Slide {
        title: title.clone(),
        body: Body::Title {
            title: title.clone(),
            subtitle: subtitle.clone(),
            lines: head_lines.clone(),
        },
        notes: head_lines.join("\n"),
    }];
    let mut i = first_section;
    while i < items.len() {
        let Item::Heading { spans, .. } = &items[i] else {
            i += 1;
            continue;
        };
        let section_title = plain(spans);
        let end = items[i + 1..]
            .iter()
            .position(|it| matches!(it, Item::Heading { level: 1 | 2, .. }))
            .map(|p| i + 1 + p)
            .unwrap_or(items.len());
        let body: Vec<&Item> = items[i + 1..end].iter().collect();
        if !body.is_empty() {
            slides.extend(section_slides(&section_title, &body));
        }
        i = end;
    }
    let hints: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            Item::Note(s) => Some(plain(s)),
            _ => None,
        })
        .collect();
    slides.push(Slide {
        title: "Vielen Dank".into(),
        body: Body::Closing {
            title: "Vielen Dank".into(),
            lines: [subtitle, title]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect(),
        },
        notes: hints.join("\n"),
    });
    slides
}

// ---------------------------------------------------------------------------
// OOXML
// ---------------------------------------------------------------------------

const NS: &str = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";

fn hex(value: &str) -> String {
    match parse_color(value) {
        Some((r, g, b)) => format!("{r:02X}{g:02X}{b:02X}"),
        None => "000000".to_string(),
    }
}

fn solid(color: &str) -> String {
    format!("<a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill>")
}

fn run(text: &str, size_pt: u32, bold: bool, color: &str) -> String {
    format!(
        "<a:r><a:rPr lang=\"de-DE\" sz=\"{}\" b=\"{}\" dirty=\"0\">{}<a:latin typeface=\"{FONT}\"/><a:cs typeface=\"{FONT}\"/></a:rPr><a:t>{}</a:t></a:r>",
        size_pt * 100,
        u8::from(bold),
        solid(color),
        xml_escape(text)
    )
}

/// Ein Textfeld. `paras`: fertige `<a:p>`-Elemente.
fn text_box(
    id: u32,
    name: &str,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    anchor: &str,
    paras: &str,
) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr>\
<p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr>\
<p:txBody><a:bodyPr wrap=\"square\" lIns=\"0\" rIns=\"0\" tIns=\"0\" bIns=\"0\" anchor=\"{anchor}\"><a:normAutofit/></a:bodyPr><a:lstStyle/>{paras}</p:txBody></p:sp>"
    )
}

fn rect(id: u32, x: i64, y: i64, w: i64, h: i64, color: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Fläche {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>\
<p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>{}<a:ln><a:noFill/></a:ln></p:spPr></p:sp>",
        solid(color)
    )
}

fn para(content: &str, space_after_pt: u32) -> String {
    format!(
        "<a:p><a:pPr><a:spcAft><a:spcPts val=\"{}\"/></a:spcAft></a:pPr>{content}</a:p>",
        space_after_pt * 100
    )
}

struct Theme {
    accent: String,
    accent_text: String,
    text: String,
    title: String,
    muted: String,
    line: String,
}

fn theme_of(design: &Design<'_>) -> Theme {
    let f = &design.letterhead.farben;
    Theme {
        accent: hex(&f.akzent),
        accent_text: hex(&f.akzent_text),
        text: hex(&f.text),
        title: hex(&f.titel),
        muted: hex(&f.gedaempft),
        line: hex(&f.linie),
    }
}

fn logo_pic(design: &Design<'_>, id: u32) -> String {
    let (Some(logo), Some(_)) = (design.logo, design.letterhead.logo.as_ref()) else {
        return String::new();
    };
    let h = (0.5 * EMU_IN as f64) as i64;
    let w = (h as f64 * f64::from(logo.width_px) / f64::from(logo.height_px.max(1))) as i64;
    let x = SLIDE_W - MARGIN_X - w;
    format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Logo\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr>\
<p:blipFill><a:blip r:embed=\"rIdLogo\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
<p:spPr><a:xfrm><a:off x=\"{x}\" y=\"320040\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>"
    )
}

/// Akzentbalken, Logo, Fuss mit Foliennummer (ausser auf der Titelfolie).
fn chrome(design: &Design<'_>, t: &Theme, number: Option<usize>) -> String {
    let mut xml = rect(2, 0, 0, SLIDE_W, 91_440, &t.accent);
    xml.push_str(&logo_pic(design, 3));
    if let Some(n) = number {
        let lh = design.letterhead;
        let footer = lh
            .fusszeile
            .zeilen
            .first()
            .cloned()
            .unwrap_or_else(|| lh.firma.name.clone());
        xml.push_str(&rect(
            4,
            MARGIN_X,
            6_309_360,
            SLIDE_W - 2 * MARGIN_X,
            9_525,
            &t.line,
        ));
        xml.push_str(&text_box(
            5,
            "Fußzeile",
            MARGIN_X,
            6_400_800,
            SLIDE_W - 2 * MARGIN_X - 1_000_000,
            274_320,
            "t",
            &para(&run(&footer, 10, false, &t.muted), 0),
        ));
        xml.push_str(&text_box(
            6,
            "Foliennummer",
            SLIDE_W - MARGIN_X - 900_000,
            6_400_800,
            900_000,
            274_320,
            "t",
            &format!(
                "<a:p><a:pPr algn=\"r\"/><a:fld id=\"{{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}}\" type=\"slidenum\"><a:rPr lang=\"de-DE\" sz=\"1000\">{}<a:latin typeface=\"{FONT}\"/></a:rPr><a:t>{n}</a:t></a:fld></a:p>",
                solid(&t.muted)
            ),
        ));
    }
    xml
}

fn table_xml(header: &[String], rows: &[Vec<String>], t: &Theme) -> String {
    let cols = header
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(1))
        .max(1);
    let width = SLIDE_W - 2 * MARGIN_X;
    let shares: Vec<f64> = match cols {
        3 if header.first().map(String::as_str) == Some("Was") => vec![0.62, 0.2, 0.18],
        2 if header.first().map(String::as_str) == Some("#") => vec![0.08, 0.92],
        _ => vec![1.0 / cols as f64; cols],
    };
    let grid: String = shares
        .iter()
        .map(|s| format!("<a:gridCol w=\"{}\"/>", (width as f64 * s) as i64))
        .collect();
    let border = |side: &str| {
        format!(
            "<a:{side} w=\"9525\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></a:{side}>",
            t.line
        )
    };
    let cell = |text: &str, head: bool| {
        let fill = if head {
            solid(&t.accent)
        } else {
            "<a:noFill/>".to_string()
        };
        let color = if head { &t.accent_text } else { &t.text };
        format!(
            "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p>{}</a:p></a:txBody><a:tcPr marL=\"91440\" marR=\"91440\" marT=\"45720\" marB=\"45720\">{}{}{}</a:tcPr></a:tc>",
            run(text, 16, head, color),
            border("lnT"),
            border("lnB"),
            fill
        )
    };
    let row = |cells: &[String], head: bool| {
        let body: String = (0..cols)
            .map(|i| cell(cells.get(i).map(String::as_str).unwrap_or(""), head))
            .collect();
        format!("<a:tr h=\"370840\">{body}</a:tr>")
    };
    let mut rows_xml = String::new();
    if !header.is_empty() {
        rows_xml.push_str(&row(header, true));
    }
    for r in rows {
        rows_xml.push_str(&row(r, false));
    }
    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"8\" name=\"Tabelle\"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr>\
<p:xfrm><a:off x=\"{MARGIN_X}\" y=\"{BODY_TOP}\"/><a:ext cx=\"{width}\" cy=\"{}\"/></p:xfrm>\
<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr firstRow=\"1\"/><a:tblGrid>{grid}</a:tblGrid>{rows_xml}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>",
        370_840 * (rows.len() as i64 + 1)
    )
}

fn slide_xml(slide: &Slide, number: usize, design: &Design<'_>, t: &Theme) -> String {
    let mut shapes = String::new();
    match &slide.body {
        Body::Title {
            title,
            subtitle,
            lines,
        } => {
            shapes.push_str(&chrome(design, t, None));
            shapes.push_str(&rect(7, MARGIN_X, 2_286_000, 91_440, 1_600_200, &t.accent));
            let mut paras = para(&run(title, 40, true, &t.title), 6);
            if !subtitle.is_empty() {
                paras.push_str(&para(&run(subtitle, 24, false, &t.text), 12));
            }
            for line in lines {
                paras.push_str(&para(&run(line, 14, false, &t.muted), 2));
            }
            shapes.push_str(&text_box(
                9,
                "Titel",
                MARGIN_X + 320_040,
                2_286_000,
                SLIDE_W - 2 * MARGIN_X - 320_040,
                2_560_320,
                "t",
                &paras,
            ));
            let firm = &design.letterhead.firma.name;
            if !firm.trim().is_empty() {
                shapes.push_str(&text_box(
                    10,
                    "Firma",
                    MARGIN_X,
                    6_217_920,
                    SLIDE_W - 2 * MARGIN_X,
                    320_040,
                    "t",
                    &para(&run(firm, 12, true, &t.muted), 0),
                ));
            }
        }
        Body::Closing { title, lines } => {
            shapes.push_str(&chrome(design, t, Some(number)));
            let mut paras = para(&run(title, 36, true, &t.title), 8);
            for line in lines {
                paras.push_str(&para(&run(line, 18, false, &t.muted), 2));
            }
            shapes.push_str(&text_box(
                9,
                "Schluss",
                MARGIN_X,
                2_560_320,
                SLIDE_W - 2 * MARGIN_X,
                1_828_800,
                "ctr",
                &paras,
            ));
        }
        Body::Points(points) => {
            shapes.push_str(&chrome(design, t, Some(number)));
            shapes.push_str(&title_box(&slide.title, t));
            let size = if points.len() <= 4 { 20 } else { 18 };
            let paras: String = points
                .iter()
                .map(|p| {
                    if p.paragraph {
                        format!("<a:p><a:pPr><a:spcAft><a:spcPts val=\"1000\"/></a:spcAft></a:pPr>{}</a:p>", run(&p.short, size, false, &t.text))
                    } else {
                        format!(
                            "<a:p><a:pPr marL=\"342900\" indent=\"-342900\"><a:spcAft><a:spcPts val=\"1000\"/></a:spcAft><a:buClr><a:srgbClr val=\"{}\"/></a:buClr><a:buFont typeface=\"Arial\"/><a:buChar char=\"&#8226;\"/></a:pPr>{}</a:p>",
                            t.title,
                            run(&p.short, size, false, &t.text)
                        )
                    }
                })
                .collect();
            shapes.push_str(&text_box(
                9,
                "Inhalt",
                MARGIN_X,
                BODY_TOP,
                SLIDE_W - 2 * MARGIN_X,
                BODY_H,
                "t",
                &paras,
            ));
        }
        Body::Table { header, rows } => {
            shapes.push_str(&chrome(design, t, Some(number)));
            shapes.push_str(&title_box(&slide.title, t));
            shapes.push_str(&table_xml(header, rows, t));
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sld {NS}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    )
}

fn title_box(title: &str, t: &Theme) -> String {
    text_box(
        7,
        "Titel",
        MARGIN_X,
        457_200,
        SLIDE_W - 2 * MARGIN_X - 1_400_000,
        822_960,
        "b",
        &para(&run(title, 28, true, &t.title), 0),
    )
}

fn notes_xml(slide: &Slide, number: usize) -> String {
    let paras: String = slide
        .notes
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            format!(
                "<a:p><a:r><a:rPr lang=\"de-DE\" dirty=\"0\"/><a:t>{}</a:t></a:r></a:p>",
                xml_escape(l)
            )
        })
        .collect();
    let paras = if paras.is_empty() {
        "<a:p><a:endParaRPr lang=\"de-DE\"/></a:p>".to_string()
    } else {
        paras
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:notes {NS}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>\
<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Folienbild {number}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\" noRot=\"1\" noChangeAspect=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"sldImg\"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp>\
<p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"Notizen {number}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{paras}</p:txBody></p:sp>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"
    )
}

const CLR_MAP: &str = "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>";

fn theme_xml(t: &Theme) -> String {
    let clr = |name: &str, val: &str| format!("<a:{name}><a:srgbClr val=\"{val}\"/></a:{name}>");
    let fills = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>".repeat(3);
    let line =
        "<a:ln w=\"9525\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>".repeat(3);
    let effects = "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Briefkopf\"><a:themeElements>\
<a:clrScheme name=\"Briefkopf\">{}{}{}{}{}{}{}{}{}{}{}{}</a:clrScheme>\
<a:fontScheme name=\"Briefkopf\"><a:majorFont><a:latin typeface=\"{FONT}\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"{FONT}\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme>\
<a:fmtScheme name=\"Briefkopf\"><a:fillStyleLst>{fills}</a:fillStyleLst><a:lnStyleLst>{line}</a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst><a:bgFillStyleLst>{fills}</a:bgFillStyleLst></a:fmtScheme>\
</a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>",
        clr("dk1", "000000"),
        clr("lt1", "FFFFFF"),
        clr("dk2", &t.title),
        clr("lt2", "F8F9FB"),
        clr("accent1", &t.accent),
        clr("accent2", &t.title),
        clr("accent3", &t.muted),
        clr("accent4", &t.line),
        clr("accent5", &t.text),
        clr("accent6", &t.accent_text),
        clr("hlink", &t.title),
        clr("folHlink", &t.muted),
    )
}

fn master_xml() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sldMaster {NS}><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld>{CLR_MAP}\
<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst>\
<p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz=\"2800\"/></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr><a:defRPr sz=\"1800\"/></a:lvl1pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr sz=\"1800\"/></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"
    )
}

fn layout_xml() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sldLayout {NS} preserve=\"1\"><p:cSld name=\"Leer\"><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
    )
}

fn notes_master_xml() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:notesMaster {NS}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>\
<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Folienbild\"/><p:cNvSpPr><a:spLocks noGrp=\"1\" noRot=\"1\" noChangeAspect=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"sldImg\" idx=\"2\"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"381000\" y=\"685800\"/><a:ext cx=\"6096000\" cy=\"3429000\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:sp>\
<p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"Notizen\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"body\" sz=\"quarter\" idx=\"3\"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"685800\" y=\"4343400\"/><a:ext cx=\"5486400\" cy=\"4114800\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"de-DE\"/></a:p></p:txBody></p:sp>\
</p:spTree></p:cSld>{CLR_MAP}<p:notesStyle><a:lvl1pPr><a:defRPr sz=\"1200\"/></a:lvl1pPr></p:notesStyle></p:notesMaster>"
    )
}

fn rels(entries: &[(&str, &str, String)]) -> String {
    let body: String = entries
        .iter()
        .map(|(id, kind, target)| {
            format!("<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{target}\"/>")
        })
        .collect();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{body}</Relationships>")
}

fn content_types(count: usize, logo: Option<ImageKind>) -> String {
    let image = match logo {
        Some(ImageKind::Png) => "<Default Extension=\"png\" ContentType=\"image/png\"/>",
        Some(ImageKind::Jpeg) => "<Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>",
        None => "",
    };
    let pml = "application/vnd.openxmlformats-officedocument.presentationml";
    let mut parts = format!(
        "<Override PartName=\"/ppt/presentation.xml\" ContentType=\"{pml}.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"{pml}.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"{pml}.slideLayout+xml\"/>\
<Override PartName=\"/ppt/notesMasters/notesMaster1.xml\" ContentType=\"{pml}.notesMaster+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\
<Override PartName=\"/ppt/theme/theme2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\
<Override PartName=\"/ppt/presProps.xml\" ContentType=\"{pml}.presProps+xml\"/>\
<Override PartName=\"/ppt/tableStyles.xml\" ContentType=\"{pml}.tableStyles+xml\"/>"
    );
    for n in 1..=count {
        parts.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{n}.xml\" ContentType=\"{pml}.slide+xml\"/>\
<Override PartName=\"/ppt/notesSlides/notesSlide{n}.xml\" ContentType=\"{pml}.notesSlide+xml\"/>"
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>{image}{parts}</Types>"
    )
}

fn presentation_xml(count: usize) -> String {
    let ids: String = (1..=count)
        .map(|n| format!("<p:sldId id=\"{}\" r:id=\"rIdSlide{n}\"/>", 255 + n))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:presentation {NS} saveSubsetFonts=\"1\">\
<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rIdMaster\"/></p:sldMasterIdLst>\
<p:notesMasterIdLst><p:notesMasterId r:id=\"rIdNotesMaster\"/></p:notesMasterIdLst>\
<p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"{SLIDE_W}\" cy=\"{SLIDE_H}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/>\
<p:defaultTextStyle><a:lvl1pPr><a:defRPr lang=\"de-DE\"/></a:lvl1pPr></p:defaultTextStyle></p:presentation>"
    )
}

/// Die .pptx-Datei als Bytes.
pub fn render(items: &[Item], design: &Design<'_>) -> Result<Vec<u8>, String> {
    let slides = plan(items);
    let t = theme_of(design);
    let count = slides.len();
    let logo_ext = design.logo.map(|l| l.kind.extension());
    let mut parts: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), content_types(count, design.logo.map(|l| l.kind)).into_bytes()),
        ("_rels/.rels".into(), rels(&[("rId1", "officeDocument", "ppt/presentation.xml".into())]).into_bytes()),
        ("ppt/presentation.xml".into(), presentation_xml(count).into_bytes()),
        ("ppt/presProps.xml".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:presentationPr {NS}/>").into_bytes()),
        ("ppt/tableStyles.xml".into(), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<a:tblStyleLst xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" def=\"{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}\"/>".as_bytes().to_vec()),
        ("ppt/slideMasters/slideMaster1.xml".into(), master_xml().into_bytes()),
        ("ppt/slideMasters/_rels/slideMaster1.xml.rels".into(), rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml".into()), ("rId2", "theme", "../theme/theme1.xml".into())]).into_bytes()),
        ("ppt/slideLayouts/slideLayout1.xml".into(), layout_xml().into_bytes()),
        ("ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(), rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml".into())]).into_bytes()),
        ("ppt/notesMasters/notesMaster1.xml".into(), notes_master_xml().into_bytes()),
        ("ppt/notesMasters/_rels/notesMaster1.xml.rels".into(), rels(&[("rId1", "theme", "../theme/theme2.xml".into())]).into_bytes()),
        ("ppt/theme/theme1.xml".into(), theme_xml(&t).into_bytes()),
        ("ppt/theme/theme2.xml".into(), theme_xml(&t).into_bytes()),
    ];
    let mut pres_rels: Vec<(String, &str, String)> = vec![
        (
            "rIdMaster".into(),
            "slideMaster",
            "slideMasters/slideMaster1.xml".into(),
        ),
        (
            "rIdNotesMaster".into(),
            "notesMaster",
            "notesMasters/notesMaster1.xml".into(),
        ),
        ("rIdTheme".into(), "theme", "theme/theme1.xml".into()),
        ("rIdPresProps".into(), "presProps", "presProps.xml".into()),
        (
            "rIdTableStyles".into(),
            "tableStyles",
            "tableStyles.xml".into(),
        ),
    ];
    for (i, slide) in slides.iter().enumerate() {
        let n = i + 1;
        pres_rels.push((
            format!("rIdSlide{n}"),
            "slide",
            format!("slides/slide{n}.xml"),
        ));
        parts.push((
            format!("ppt/slides/slide{n}.xml"),
            slide_xml(slide, n, design, &t).into_bytes(),
        ));
        let mut slide_rels = vec![
            (
                "rId1",
                "slideLayout",
                "../slideLayouts/slideLayout1.xml".to_string(),
            ),
            (
                "rIdNotes",
                "notesSlide",
                format!("../notesSlides/notesSlide{n}.xml"),
            ),
        ];
        if let Some(ext) = logo_ext {
            slide_rels.push(("rIdLogo", "image", format!("../media/logo.{ext}")));
        }
        parts.push((
            format!("ppt/slides/_rels/slide{n}.xml.rels"),
            rels(&slide_rels).into_bytes(),
        ));
        parts.push((
            format!("ppt/notesSlides/notesSlide{n}.xml"),
            notes_xml(slide, n).into_bytes(),
        ));
        parts.push((
            format!("ppt/notesSlides/_rels/notesSlide{n}.xml.rels"),
            rels(&[
                (
                    "rId1",
                    "notesMaster",
                    "../notesMasters/notesMaster1.xml".into(),
                ),
                ("rId2", "slide", format!("../slides/slide{n}.xml")),
            ])
            .into_bytes(),
        ));
    }
    let pres_rels_ref: Vec<(&str, &str, String)> = pres_rels
        .iter()
        .map(|(a, b, c)| (a.as_str(), *b, c.clone()))
        .collect();
    parts.push((
        "ppt/_rels/presentation.xml.rels".into(),
        rels(&pres_rels_ref).into_bytes(),
    ));
    if let (Some(logo), Some(ext)) = (design.logo, logo_ext) {
        parts.push((format!("ppt/media/logo.{ext}"), logo.bytes.clone()));
    }

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in parts {
            zip.start_file(name.as_str(), options)
                .map_err(|e| format!("pptx part {name} failed: {e}"))?;
            zip.write_all(&bytes)
                .map_err(|e| format!("pptx part {name} failed: {e}"))?;
        }
        zip.finish().map_err(|e| format!("pptx failed: {e}"))?;
    }
    Ok(buffer.into_inner())
}

/// Reiner Text einer Spans-Liste (fuer Tests).
#[cfg(test)]
fn text(spans: &[Span]) -> String {
    plain(spans)
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;
    use crate::managers::letterhead::{builtins, Store};
    use std::io::Read;

    const MD: &str = "# Management Summary: Jour fixe\n\n**Datum:** 2026-10-13 · **Dauer:** 30:00\n\n\
## Anlass und Ziel\n\nKlärung des Go-Live.\n\n\
## Kernergebnisse\n\n- Der Go-Live ist am 1. September. Davor läuft die Testphase bis Ende August.\n- Budget bleibt.\n\n\
## Entscheidungen\n\n- Go-Live am 1. September\n- Kein Umbau des Kerns\n\n\
## Nächste Schritte\n\n- Release-Notes schreiben _(Wer: Anna, Bis: Freitag)_\n- Abnahme planen _(Wer: Ben)_\n\n\
> **Hinweis:** Länger als vorgesehen.\n";

    #[test]
    fn sentences_and_tasks_are_split_without_cutting_words() {
        assert_eq!(
            first_sentence("Der Go-Live ist am 1. September. Davor läuft die Testphase."),
            "Der Go-Live ist am 1. September."
        );
        assert_eq!(first_sentence("z. B. ohne Ende"), "z. B. ohne Ende");
        assert_eq!(
            split_task("Release-Notes schreiben (Wer: Anna, Bis: Freitag)"),
            Some((
                "Release-Notes schreiben".into(),
                "Anna".into(),
                "Freitag".into()
            ))
        );
        assert_eq!(split_task("Abnahme (Bis: Mai)").unwrap().2, "Mai");
        assert_eq!(split_task("Text (siehe oben)"), None);
        assert!(is_decisions("Entscheidungen"));
        assert!(is_decisions("Beschlüsse und Aufträge"));
        assert!(!is_decisions("Entscheidungsbedarf"));
        assert_eq!(text(&[]), "");
    }

    #[test]
    fn the_plan_has_title_sections_tables_and_a_closing_slide() {
        let slides = plan(&parse(MD));
        let titles: Vec<&str> = slides.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            vec![
                "Management Summary",
                "Anlass und Ziel",
                "Kernergebnisse",
                "Entscheidungen",
                "Nächste Schritte",
                "Vielen Dank"
            ]
        );
        assert!(
            matches!(&slides[0].body, Body::Title { subtitle, lines, .. }
            if subtitle == "Jour fixe" && lines[0].starts_with("Datum:"))
        );
        match &slides[2].body {
            Body::Points(p) => {
                assert_eq!(p[0].short, "Der Go-Live ist am 1. September.");
                assert!(p[0].full.ends_with("Ende August."));
            }
            other => panic!("{other:?}"),
        }
        assert!(slides[2]
            .notes
            .contains("Davor läuft die Testphase bis Ende August."));
        assert!(
            matches!(&slides[3].body, Body::Table { header, rows } if header[1] == "Entscheidung" && rows.len() == 2)
        );
        match &slides[4].body {
            Body::Table { header, rows } => {
                assert_eq!(header, &vec!["Was".to_string(), "Wer".into(), "Bis".into()]);
                assert_eq!(
                    rows[0],
                    vec![
                        "Release-Notes schreiben".to_string(),
                        "Anna".into(),
                        "Freitag".into()
                    ]
                );
                assert_eq!(rows[1][2], "");
            }
            other => panic!("{other:?}"),
        }
        assert!(slides
            .last()
            .unwrap()
            .notes
            .contains("Länger als vorgesehen"));
    }

    #[test]
    fn long_sections_continue_on_further_slides_and_nothing_is_dropped() {
        let mut md = String::from("# Statusbericht: Projekt\n\n## Erreicht\n\n");
        for i in 0..15 {
            md.push_str(&format!("- Punkt {i} mit einer Aussage, die eine ganze Zeile füllen kann, damit die Folie sich füllt.\n"));
        }
        md.push_str("\n## Aufgaben\n\n");
        for i in 0..14 {
            md.push_str(&format!("- Aufgabe {i} _(Wer: Anna, Bis: Freitag)_\n"));
        }
        let slides = plan(&parse(&md));
        let reached: Vec<&Slide> = slides
            .iter()
            .filter(|s| s.title.starts_with("Erreicht"))
            .collect();
        assert!(reached.len() >= 3, "{}", reached.len());
        assert_eq!(reached[1].title, "Erreicht (Forts.)");
        let total: usize = reached
            .iter()
            .map(|s| match &s.body {
                Body::Points(p) => {
                    assert!(p.len() <= MAX_POINTS);
                    p.len()
                }
                _ => 0,
            })
            .sum();
        assert_eq!(total, 15, "kein Punkt verloren");
        let task_rows: usize = slides
            .iter()
            .filter(|s| s.title.starts_with("Aufgaben"))
            .map(|s| match &s.body {
                Body::Table { rows, .. } => {
                    assert!(rows.len() <= MAX_POINTS);
                    rows.len()
                }
                _ => 0,
            })
            .sum();
        assert_eq!(task_rows, 14);
    }

    fn part(bytes: &[u8], name: &str) -> String {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut file = zip.by_name(name).unwrap_or_else(|_| panic!("{name} fehlt"));
        let mut s = String::new();
        file.read_to_string(&mut s).unwrap();
        s
    }

    #[test]
    fn the_package_has_all_parts_notes_logo_and_escaped_text() {
        let lh = builtins().into_iter().find(|b| b.id == "wai").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let logo = Store::new(dir.path().to_path_buf()).logo(&lh).unwrap();
        let design = Design {
            letterhead: &lh,
            logo: logo.as_ref(),
        };
        let md = MD.replace("Jour fixe", "Q&A <Runde>");
        let bytes = render(&parse(&md), &design).unwrap();
        let count = plan(&parse(&md)).len();
        let pres = part(&bytes, "ppt/presentation.xml");
        assert_eq!(pres.matches("<p:sldId ").count(), count);
        for n in 1..=count {
            part(&bytes, &format!("ppt/slides/slide{n}.xml"));
            assert!(part(&bytes, &format!("ppt/slides/_rels/slide{n}.xml.rels"))
                .contains("media/logo.png"));
            part(&bytes, &format!("ppt/notesSlides/notesSlide{n}.xml"));
        }
        assert!(part(&bytes, "ppt/slides/slide1.xml").contains("Q&amp;A &lt;Runde&gt;"));
        assert!(part(&bytes, "ppt/notesSlides/notesSlide3.xml").contains("Ende August"));
        assert!(part(&bytes, "ppt/slides/slide5.xml").contains("<a:tbl>"));
        assert!(part(&bytes, "ppt/theme/theme1.xml").contains("FFDD00"));
        // 13.10.2026: PowerPoint lehnte die Datei mit leerem `p:normalViewPr` als beschaedigt
        // ab (0x80070570); viewProps ist optional und entfaellt.
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        assert!(zip.by_name("ppt/viewProps.xml").is_err());
        drop(zip);
        let table = part(&bytes, "ppt/slides/slide5.xml");
        assert!(table.find("<a:lnT").unwrap() < table.find("<a:lnB").unwrap());
        let types = part(&bytes, "[Content_Types].xml");
        assert!(!types.contains("viewProps"));
        assert!(types.contains("image/png") && types.contains("notesSlide+xml"));
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        assert!(zip.by_name("ppt/media/logo.png").is_ok());
    }
}
