//! Word (.docx) mit Briefkopf: handgeschriebenes OOXML ueber `zip`, wie der bisherige
//! Protokoll-Export (`export::write_docx`), ergaenzt um Kopf- und Fusszeile, Logo,
//! Tabellen und die Farben des Briefkopfs. Keine Abhaengigkeit, keine Office-Automation.
//!
//! Schrift: Segoe UI (Medienregel des Design-Systems: Empfaenger haben die Markenschriften
//! nicht; Word auf dem Mac ersetzt sie durch eine aehnliche Schrift).

use std::io::Write;

use super::{Design, Item};
use crate::managers::letterhead::{parse_color, ImageKind, LogoPosition};
use crate::managers::meetings::export::{xml_escape, Span};

/// Schrift in Word.
pub const OFFICE_FONT: &str = "Segoe UI";
const EMU_PER_MM: f64 = 36_000.0;
/// Satzspiegel A4 in Twips (1/20 pt): Breite 11906, Raender links/rechts je 1247 (22 mm).
const TEXT_WIDTH_TWIPS: u32 = 11906 - 2 * 1247;

const NS: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"";

/// `#RRGGBB` -> `RRGGBB` fuer `w:color`; ungueltig -> Schwarz (die Pruefung des
/// Briefkopfs laesst das nicht zu, das hier ist nur der Riegel).
fn hex(value: &str) -> String {
    match parse_color(value) {
        Some((r, g, b)) => format!("{r:02X}{g:02X}{b:02X}"),
        None => "000000".to_string(),
    }
}

fn run(
    span_text: &str,
    bold: bool,
    italic: bool,
    color: Option<&str>,
    size_half_pt: Option<u32>,
) -> String {
    let mut props = String::new();
    if bold {
        props.push_str("<w:b/>");
    }
    if italic {
        props.push_str("<w:i/>");
    }
    if let Some(c) = color {
        props.push_str(&format!("<w:color w:val=\"{c}\"/>"));
    }
    if let Some(sz) = size_half_pt {
        props.push_str(&format!("<w:sz w:val=\"{sz}\"/>"));
    }
    let props = if props.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{props}</w:rPr>")
    };
    format!(
        "<w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
        xml_escape(span_text)
    )
}

fn runs(spans: &[Span], color: Option<&str>, force_bold: bool) -> String {
    spans
        .iter()
        .map(|s| run(&s.text, s.bold || force_bold, s.italic, color, None))
        .collect()
}

fn table_xml(
    header: &Option<Vec<Vec<Span>>>,
    rows: &[Vec<Vec<Span>>],
    design: &Design<'_>,
) -> String {
    let f = &design.letterhead.farben;
    let line = hex(&f.linie);
    let cols = header
        .as_ref()
        .map(Vec::len)
        .unwrap_or(0)
        .max(rows.iter().map(Vec::len).max().unwrap_or(0))
        .max(1);
    let width = TEXT_WIDTH_TWIPS / cols as u32;
    let grid: String = (0..cols)
        .map(|_| format!("<w:gridCol w:w=\"{width}\"/>"))
        .collect();
    let cell = |spans: Option<&Vec<Span>>, head: bool| -> String {
        let shade = if head {
            format!(
                "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
                hex(&f.akzent)
            )
        } else {
            String::new()
        };
        let color = head.then(|| hex(&f.akzent_text));
        let content = spans
            .map(|s| runs(s, color.as_deref(), head))
            .unwrap_or_default();
        format!(
            "<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/>{shade}</w:tcPr>\
             <w:p><w:pPr><w:spacing w:before=\"40\" w:after=\"40\"/></w:pPr>{content}</w:p></w:tc>"
        )
    };
    let row = |cells: &Vec<Vec<Span>>, head: bool| -> String {
        let tr_pr = if head {
            "<w:trPr><w:tblHeader/></w:trPr>"
        } else {
            ""
        };
        let body: String = (0..cols).map(|i| cell(cells.get(i), head)).collect();
        format!("<w:tr>{tr_pr}{body}</w:tr>")
    };
    let mut xml = format!(
        "<w:tbl><w:tblPr><w:tblW w:w=\"5000\" w:type=\"pct\"/>\
         <w:tblBorders>\
         <w:top w:val=\"single\" w:sz=\"4\" w:color=\"{line}\"/>\
         <w:bottom w:val=\"single\" w:sz=\"4\" w:color=\"{line}\"/>\
         <w:insideH w:val=\"single\" w:sz=\"4\" w:color=\"{line}\"/>\
         </w:tblBorders>\
         <w:tblCellMar><w:left w:w=\"100\" w:type=\"dxa\"/><w:right w:w=\"100\" w:type=\"dxa\"/></w:tblCellMar>\
         </w:tblPr><w:tblGrid>{grid}</w:tblGrid>"
    );
    if let Some(h) = header {
        xml.push_str(&row(h, true));
    }
    for r in rows {
        xml.push_str(&row(r, false));
    }
    xml.push_str("</w:tbl><w:p><w:pPr><w:spacing w:after=\"60\"/></w:pPr></w:p>");
    xml
}

fn body_xml(items: &[Item], design: &Design<'_>) -> String {
    let f = &design.letterhead.farben;
    let muted = hex(&f.gedaempft);
    let mut body = String::new();
    for item in items {
        match item {
            Item::Heading { level, spans } => {
                let style = match level {
                    1 => "Title",
                    2 => "Heading1",
                    _ => "Heading2",
                };
                body.push_str(&format!(
                    "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>{}</w:p>",
                    runs(spans, None, false)
                ));
            }
            Item::Paragraph(spans) => {
                body.push_str(&format!("<w:p>{}</w:p>", runs(spans, None, false)));
            }
            Item::Bullet { spans, done } => {
                let mark = match done {
                    None => "\u{2022}",
                    Some(false) => "\u{2610}",
                    Some(true) => "\u{2611}",
                };
                body.push_str(&format!(
                    "<w:p><w:pPr><w:spacing w:after=\"60\"/><w:ind w:left=\"360\" w:hanging=\"240\"/></w:pPr>\
                     <w:r><w:t xml:space=\"preserve\">{mark}\t</w:t></w:r>{}</w:p>",
                    runs(spans, None, false)
                ));
            }
            Item::Table { header, rows } => body.push_str(&table_xml(header, rows, design)),
            Item::Note(spans) => {
                body.push_str(&format!(
                    "<w:p><w:pPr><w:pBdr><w:left w:val=\"single\" w:sz=\"18\" w:space=\"8\" w:color=\"{}\"/></w:pBdr>\
                     <w:ind w:left=\"200\"/><w:spacing w:before=\"120\"/></w:pPr>{}</w:p>",
                    hex(&f.akzent),
                    runs(spans, Some(&muted), false)
                ));
            }
        }
    }
    body
}

fn document_xml(items: &[Item], design: &Design<'_>) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document {NS}><w:body>{}\
         <w:sectPr><w:headerReference w:type=\"default\" r:id=\"rIdHeader\"/>\
         <w:footerReference w:type=\"default\" r:id=\"rIdFooter\"/>\
         <w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
         <w:pgMar w:top=\"1701\" w:right=\"1247\" w:bottom=\"1418\" w:left=\"1247\" w:header=\"567\" w:footer=\"567\" w:gutter=\"0\"/>\
         </w:sectPr></w:body></w:document>",
        body_xml(items, design)
    )
}

fn styles_xml(design: &Design<'_>) -> String {
    let f = &design.letterhead.farben;
    let (text, title) = (hex(&f.text), hex(&f.titel));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{OFFICE_FONT}\" w:hAnsi=\"{OFFICE_FONT}\" w:cs=\"{OFFICE_FONT}\" w:eastAsia=\"{OFFICE_FONT}\"/>\
<w:color w:val=\"{text}\"/><w:sz w:val=\"20\"/><w:lang w:val=\"de-DE\"/></w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"100\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/>\
<w:pPr><w:spacing w:before=\"0\" w:after=\"160\"/></w:pPr><w:rPr><w:b/><w:color w:val=\"{title}\"/><w:sz w:val=\"36\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/>\
<w:pPr><w:keepNext/><w:outlineLvl w:val=\"0\"/><w:spacing w:before=\"280\" w:after=\"100\"/></w:pPr><w:rPr><w:b/><w:color w:val=\"{title}\"/><w:sz w:val=\"26\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/>\
<w:pPr><w:keepNext/><w:outlineLvl w:val=\"1\"/><w:spacing w:before=\"200\" w:after=\"80\"/></w:pPr><w:rPr><w:b/><w:color w:val=\"{title}\"/><w:sz w:val=\"22\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Header\"><w:name w:val=\"header\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:after=\"0\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Footer\"><w:name w:val=\"footer\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:after=\"0\"/></w:pPr><w:rPr><w:sz w:val=\"16\"/></w:rPr></w:style>\
</w:styles>"
    )
}

/// Das Logo als Inline-Grafik (`rIdLogo` in den Beziehungen der Kopfzeile).
fn logo_xml(design: &Design<'_>) -> Option<String> {
    let logo = design.logo?;
    let spec = design.letterhead.logo.as_ref()?;
    let cx = (f64::from(spec.breite_mm) * EMU_PER_MM).round() as u64;
    let cy =
        (cx as f64 * f64::from(logo.height_px) / f64::from(logo.width_px.max(1))).round() as u64;
    Some(format!(
        "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
<wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"1\" name=\"Logo\"/>\
<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
<pic:pic><pic:nvPicPr><pic:cNvPr id=\"0\" name=\"Logo\"/><pic:cNvPicPr/></pic:nvPicPr>\
<pic:blipFill><a:blip r:embed=\"rIdLogo\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr>\
</pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
    ))
}

/// Kopfzeile: eine randlose Tabelle (Logo | Firma), darunter eine Linie in der Akzentfarbe.
fn header_xml(design: &Design<'_>) -> String {
    let lh = design.letterhead;
    let f = &lh.farben;
    let muted = hex(&f.gedaempft);
    let mut firm = String::new();
    if !lh.firma.name.trim().is_empty() {
        firm.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"Header\"/>{{ALIGN}}</w:pPr>{}</w:p>",
            run(&lh.firma.name, true, false, Some(&hex(&f.titel)), Some(22))
        ));
    }
    for line in &lh.firma.zeilen {
        firm.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"Header\"/>{{ALIGN}}</w:pPr>{}</w:p>",
            run(line, false, false, Some(&muted), Some(16))
        ));
    }
    let right = lh
        .logo
        .as_ref()
        .is_some_and(|l| l.position == LogoPosition::Rechts);
    let logo = logo_xml(design);
    let mut content = String::new();
    match logo {
        Some(logo) => {
            let logo_cell = format!(
                "<w:tc><w:tcPr><w:tcW w:w=\"1500\" w:type=\"dxa\"/><w:vAlign w:val=\"center\"/></w:tcPr>\
                 <w:p><w:pPr><w:pStyle w:val=\"Header\"/>{}</w:pPr>{logo}</w:p></w:tc>",
                if right { "<w:jc w:val=\"right\"/>" } else { "" }
            );
            let align = if right { "" } else { "" };
            let firm_cell = format!(
                "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/><w:vAlign w:val=\"center\"/></w:tcPr>{}</w:tc>",
                TEXT_WIDTH_TWIPS - 1500,
                if firm.is_empty() {
                    "<w:p/>".to_string()
                } else {
                    firm.replace("{ALIGN}", align)
                }
            );
            let cells = if right {
                format!("{firm_cell}{logo_cell}")
            } else {
                format!("{logo_cell}{firm_cell}")
            };
            content.push_str(&format!(
                "<w:tbl><w:tblPr><w:tblW w:w=\"5000\" w:type=\"pct\"/><w:tblBorders>\
                 <w:top w:val=\"nil\"/><w:left w:val=\"nil\"/><w:bottom w:val=\"nil\"/><w:right w:val=\"nil\"/>\
                 <w:insideH w:val=\"nil\"/><w:insideV w:val=\"nil\"/></w:tblBorders></w:tblPr>\
                 <w:tblGrid><w:gridCol w:w=\"1500\"/><w:gridCol w:w=\"{}\"/></w:tblGrid><w:tr>{cells}</w:tr></w:tbl>",
                TEXT_WIDTH_TWIPS - 1500
            ));
        }
        None => content.push_str(&firm.replace("{ALIGN}", "")),
    }
    // Akzentlinie als eigener Absatz mit unterer Kante: in Word sichtbar wie im PDF.
    content.push_str(&format!(
        "<w:p><w:pPr><w:pStyle w:val=\"Header\"/><w:pBdr><w:bottom w:val=\"single\" w:sz=\"18\" w:space=\"1\" w:color=\"{}\"/></w:pBdr></w:pPr></w:p>",
        hex(&f.akzent)
    ));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:hdr {NS}>{content}</w:hdr>"
    )
}

/// Fusszeile: Zeilen des Briefkopfs links, "Seite x von y" rechts (Felder PAGE/NUMPAGES).
fn footer_xml(design: &Design<'_>) -> String {
    let lh = design.letterhead;
    let muted = hex(&lh.farben.gedaempft);
    let field = |name: &str| {
        format!(
            "<w:r><w:rPr><w:color w:val=\"{muted}\"/></w:rPr><w:fldChar w:fldCharType=\"begin\"/></w:r>\
             <w:r><w:rPr><w:color w:val=\"{muted}\"/></w:rPr><w:instrText xml:space=\"preserve\"> {name} </w:instrText></w:r>\
             <w:r><w:rPr><w:color w:val=\"{muted}\"/></w:rPr><w:fldChar w:fldCharType=\"separate\"/></w:r>\
             <w:r><w:rPr><w:color w:val=\"{muted}\"/></w:rPr><w:t>1</w:t></w:r>\
             <w:r><w:rPr><w:color w:val=\"{muted}\"/></w:rPr><w:fldChar w:fldCharType=\"end\"/></w:r>"
        )
    };
    let mut xml = String::new();
    let tab = format!("<w:tabs><w:tab w:val=\"right\" w:pos=\"{TEXT_WIDTH_TWIPS}\"/></w:tabs>");
    let lines: Vec<&String> = lh.fusszeile.zeilen.iter().collect();
    let count = lines.len().max(1);
    for i in 0..count {
        let text = lines
            .get(i)
            .map(|l| run(l, false, false, Some(&muted), None))
            .unwrap_or_default();
        let page = if i == 0 && lh.fusszeile.seitenzahl {
            format!(
                "<w:r><w:tab/></w:r>{}{}{}{}",
                run("Seite ", false, false, Some(&muted), None),
                field("PAGE"),
                run(" von ", false, false, Some(&muted), None),
                field("NUMPAGES")
            )
        } else {
            String::new()
        };
        xml.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"Footer\"/>{tab}</w:pPr>{text}{page}</w:p>"
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:ftr {NS}>{xml}</w:ftr>"
    )
}

fn content_types(design: &Design<'_>) -> String {
    let image = match design.logo.map(|l| l.kind) {
        Some(ImageKind::Png) => "<Default Extension=\"png\" ContentType=\"image/png\"/>",
        Some(ImageKind::Jpeg) => "<Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>",
        None => "",
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>{image}\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/header1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>\
<Override PartName=\"/word/footer1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml\"/>\
</Types>"
    )
}

const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";

const DOCUMENT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
<Relationship Id=\"rIdHeader\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header1.xml\"/>\
<Relationship Id=\"rIdFooter\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer\" Target=\"footer1.xml\"/>\
</Relationships>";

fn header_rels(design: &Design<'_>) -> String {
    let image = design
        .logo
        .map(|l| {
            format!(
                "<Relationship Id=\"rIdLogo\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/logo.{}\"/>",
                l.kind.extension()
            )
        })
        .unwrap_or_default();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{image}</Relationships>"
    )
}

/// Die .docx-Datei als Bytes.
pub fn render(items: &[Item], design: &Design<'_>) -> Result<Vec<u8>, String> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut parts: Vec<(String, Vec<u8>)> = vec![
            (
                "[Content_Types].xml".into(),
                content_types(design).into_bytes(),
            ),
            ("_rels/.rels".into(), ROOT_RELS.as_bytes().to_vec()),
            (
                "word/_rels/document.xml.rels".into(),
                DOCUMENT_RELS.as_bytes().to_vec(),
            ),
            (
                "word/_rels/header1.xml.rels".into(),
                header_rels(design).into_bytes(),
            ),
            ("word/styles.xml".into(), styles_xml(design).into_bytes()),
            ("word/header1.xml".into(), header_xml(design).into_bytes()),
            ("word/footer1.xml".into(), footer_xml(design).into_bytes()),
            (
                "word/document.xml".into(),
                document_xml(items, design).into_bytes(),
            ),
        ];
        if let Some(logo) = design.logo {
            parts.push((
                format!("word/media/logo.{}", logo.kind.extension()),
                logo.bytes.clone(),
            ));
        }
        for (name, bytes) in parts {
            zip.start_file(name.as_str(), options)
                .map_err(|e| format!("docx part {name} failed: {e}"))?;
            zip.write_all(&bytes)
                .map_err(|e| format!("docx part {name} failed: {e}"))?;
        }
        zip.finish().map_err(|e| format!("docx failed: {e}"))?;
    }
    Ok(buffer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;
    use crate::managers::letterhead::{builtins, Store};
    use std::io::Read;

    fn part(bytes: &[u8], name: &str) -> String {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut file = zip.by_name(name).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        text
    }

    const MD: &str = "# Management Summary: Q&A <Test>\n\n## Aufgaben\n\n- Angebot _(Wer: Anna)_\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n> **Hinweis:** kurz\n";

    #[test]
    fn a_branded_docx_has_header_logo_footer_fields_and_a_real_table() {
        let lh = builtins().into_iter().find(|b| b.id == "wai").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let logo = Store::new(dir.path().to_path_buf()).logo(&lh).unwrap();
        let design = Design {
            letterhead: &lh,
            logo: logo.as_ref(),
        };
        let bytes = render(&parse(MD), &design).unwrap();
        let doc = part(&bytes, "word/document.xml");
        assert!(doc.contains("Q&amp;A &lt;Test&gt;"), "Maskierung");
        assert!(doc.contains("<w:tbl>"));
        assert!(doc.contains("w:pStyle w:val=\"Title\""));
        assert!(doc.contains("r:id=\"rIdHeader\""));
        let header = part(&bytes, "word/header1.xml");
        assert!(header.contains("r:embed=\"rIdLogo\""));
        assert!(header.contains("Wolff Applied AI"));
        assert!(header.contains("w:color=\"FFDD00\""), "Akzentlinie");
        let footer = part(&bytes, "word/footer1.xml");
        assert!(footer.contains(" PAGE ") && footer.contains(" NUMPAGES "));
        assert!(part(&bytes, "word/_rels/header1.xml.rels").contains("media/logo.png"));
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        assert!(zip.by_name("word/media/logo.png").is_ok());
        assert!(part(&bytes, "word/styles.xml").contains("Segoe UI"));
    }

    #[test]
    fn without_logo_there_is_no_image_part() {
        let lh = builtins().remove(0);
        let design = Design {
            letterhead: &lh,
            logo: None,
        };
        let bytes = render(&parse(MD), &design).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        assert!(zip.by_name("word/media/logo.png").is_err());
        drop(zip);
        assert!(!part(&bytes, "word/header1.xml").contains("rIdLogo"));
        assert!(!part(&bytes, "[Content_Types].xml").contains("image/png"));
    }
}
