//! D2 (Spec §9): Fixtures je Format mit Ankern, Grenzen (ZIP-Bombe, XXE, übergroßes XLSX, Endlos-PDF,
//! Zeit, Speicher) im echten Unterprozess, Extraktorwechsel → neue `textrev`, Suchtreffer mit Anker,
//! Neuaufbau idempotent, Papierkorb → 0 Treffer. Alle Fixtures entstehen hier im Code, nur synthetische
//! Inhalte.
//!
//! Der Unterprozess ist in Tests das Testprogramm selbst: [`kind_einstieg`] läuft nur mit gesetztem
//! `LVA_EXTRAKT_KIND` und ruft dann `extrakt::haupt` mit den letzten drei Argumenten.

use super::extrakt::{self, Format};
use super::index;
use super::speicher;
use super::speicher::{Ablage, AblageArt, AblageImportStand, TextAblage};
use super::textfassung;
use super::textlauf::{self, Grenzen, Programm};
use super::zusammenfuehren::Text;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const KONTROLLE: &str = "Zebrafink";

// ---------------------------------------------------------------------------
// Unterprozess
// ---------------------------------------------------------------------------

#[test]
fn kind_einstieg() {
    if std::env::var_os("LVA_EXTRAKT_KIND").is_none() {
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    let n = args.len();
    std::process::exit(extrakt::haupt(&args[n - 3..]));
}

fn kind(test: Option<&str>) -> Programm {
    let mut umgebung = vec![("LVA_EXTRAKT_KIND".into(), "1".into())];
    if let Some(t) = test {
        umgebung.push(("LVA_EXTRAKT_TEST".into(), t.into()));
    }
    Programm {
        pfad: std::env::current_exe().unwrap(),
        vorweg: vec![
            "ablage::tests_d2::kind_einstieg".into(),
            "--exact".into(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ],
        umgebung,
    }
}

/// Kurze Grenzen für Tests (die Standardwerte gelten in der App).
const TEST_GRENZEN: Grenzen = Grenzen {
    zeit: Duration::from_secs(20),
    speicher_mb: 1024,
};

fn sha(b: &[u8]) -> String {
    use sha2::Digest;
    extrakt::hex(&sha2::Sha256::digest(b))
}

struct Datei {
    _d: tempfile::TempDir,
    pfad: PathBuf,
    sha: String,
}

fn datei(name: &str, bytes: &[u8]) -> Datei {
    let d = tempfile::tempdir().unwrap();
    let pfad = d.path().join(name);
    fs::write(&pfad, bytes).unwrap();
    Datei {
        _d: d,
        pfad,
        sha: sha(bytes),
    }
}

fn im_kind(
    name: &str,
    typ: &str,
    bytes: &[u8],
    test: Option<&str>,
    g: Grenzen,
) -> Result<extrakt::Textfassung, String> {
    let d = datei(name, bytes);
    textlauf::ausfuehren(
        &kind(test),
        d._d.path(),
        &d.pfad,
        &d.sha,
        typ,
        g,
        &AtomicBool::new(false),
    )
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn zip_bauen(eintraege: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opt = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (n, b) in eintraege {
            z.start_file(*n, opt).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn docx_xml(absaetze: &[(Option<&str>, &str)]) -> String {
    let mut body = String::new();
    for (stil, t) in absaetze {
        body.push_str("<w:p>");
        if let Some(s) = stil {
            body.push_str(&format!("<w:pPr><w:pStyle w:val=\"{s}\"/></w:pPr>"));
        }
        body.push_str(&format!(
            "<w:r><w:t xml:space=\"preserve\">{t}</w:t></w:r></w:p>"
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
         <w:body>{body}</w:body></w:document>"
    )
}

fn docx(absaetze: &[(Option<&str>, &str)]) -> Vec<u8> {
    zip_bauen(&[
        ("[Content_Types].xml", b"<?xml version=\"1.0\"?><Types/>"),
        ("word/document.xml", docx_xml(absaetze).as_bytes()),
    ])
}

fn vertrag_docx() -> Vec<u8> {
    docx(&[
        (Some("Heading1"), "Rahmenvertrag Beispiel"),
        (None, "Zwischen der Example GmbH und der Muster AG."),
        (Some("Heading2"), "§ 4 Laufzeit"),
        (
            None,
            &format!("Die Kündigungsfrist beträgt drei Monate. Kontrollwort {KONTROLLE}."),
        ),
    ])
}

fn odt() -> Vec<u8> {
    let content = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
        <office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
        xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text>\
        <text:h text:outline-level=\"1\">Protokoll</text:h>\
        <text:p>Erster<text:s/>Punkt mit <text:span>Übergabe</text:span>.</text:p>\
        <text:h text:outline-level=\"2\">Beschlüsse</text:h>\
        <text:p>Die Größe wird geprüft.</text:p>\
        </office:text></office:body></office:document-content>";
    zip_bauen(&[
        ("mimetype", b"application/vnd.oasis.opendocument.text"),
        ("content.xml", content.as_bytes()),
    ])
}

fn folie_xml(t: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><p:sld xmlns:p=\"p\" xmlns:a=\"a\"><p:cSld><p:spTree><p:sp><p:txBody>\
         <a:p><a:r><a:t>{t}</a:t></a:r></a:p><a:p><a:r><a:t>zweite Zeile</a:t></a:r></a:p>\
         </p:txBody></p:sp></p:spTree></p:cSld></p:sld>"
    )
}

/// Zwei Folien; die Präsentation zeigt `slide2.xml` zuerst.
fn pptx() -> Vec<u8> {
    let pres = "<?xml version=\"1.0\"?><p:presentation xmlns:p=\"p\" xmlns:r=\"r\"><p:sldIdLst>\
        <p:sldId id=\"256\" r:id=\"rId3\"/><p:sldId id=\"257\" r:id=\"rId2\"/></p:sldIdLst></p:presentation>";
    let rels = "<?xml version=\"1.0\"?><Relationships>\
        <Relationship Id=\"rId2\" Type=\"slide\" Target=\"slides/slide1.xml\"/>\
        <Relationship Id=\"rId3\" Type=\"slide\" Target=\"slides/slide2.xml\"/></Relationships>";
    let f1 = folie_xml("Folie Drache");
    let f2 = folie_xml("Folie Einhorn");
    zip_bauen(&[
        ("ppt/presentation.xml", pres.as_bytes()),
        ("ppt/_rels/presentation.xml.rels", rels.as_bytes()),
        ("ppt/slides/slide1.xml", f1.as_bytes()),
        ("ppt/slides/slide2.xml", f2.as_bytes()),
    ])
}

fn odp() -> Vec<u8> {
    let content = "<?xml version=\"1.0\"?><office:document-content xmlns:office=\"o\" xmlns:draw=\"d\" \
        xmlns:text=\"t\"><office:body><office:presentation>\
        <draw:page draw:name=\"a\"><draw:frame><draw:text-box><text:p>Begrüßung</text:p></draw:text-box></draw:frame></draw:page>\
        <draw:page draw:name=\"b\"><draw:frame><draw:text-box><text:p>Ausblick Mondfahrt</text:p></draw:text-box></draw:frame></draw:page>\
        </office:presentation></office:body></office:document-content>";
    zip_bauen(&[
        (
            "mimetype",
            b"application/vnd.oasis.opendocument.presentation",
        ),
        ("content.xml", content.as_bytes()),
    ])
}

fn xlsx_mit(sheet_xml: &str) -> Vec<u8> {
    let ct = "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
        <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
        </Types>";
    let rels = "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>";
    let wb = "<?xml version=\"1.0\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
        xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
        <sheets><sheet name=\"Kosten 2026\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>";
    let wbrels = "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>";
    zip_bauen(&[
        ("[Content_Types].xml", ct.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("xl/workbook.xml", wb.as_bytes()),
        ("xl/_rels/workbook.xml.rels", wbrels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet_xml.as_bytes()),
    ])
}

fn zelle(r: &str, t: &str) -> String {
    format!("<c r=\"{r}\" t=\"inlineStr\"><is><t>{t}</t></is></c>")
}

fn xlsx() -> Vec<u8> {
    let mut zeilen = format!(
        "<row r=\"1\">{}{}</row>",
        zelle("A1", "Posten"),
        zelle("B1", "Betrag")
    );
    for i in 2..=25 {
        let posten = if i == 23 {
            format!("Posten {KONTROLLE}")
        } else {
            format!("Posten {i}")
        };
        zeilen.push_str(&format!(
            "<row r=\"{i}\">{}<c r=\"B{i}\"><v>{}</v></c></row>",
            zelle(&format!("A{i}"), &posten),
            i * 10
        ));
    }
    xlsx_mit(&format!(
        "<?xml version=\"1.0\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <sheetData>{zeilen}</sheetData></worksheet>"
    ))
}

/// Zwei Zellen weit auseinander: calamine legt die volle Fläche (17 Mrd. Zellen) an.
fn xlsx_riesig() -> Vec<u8> {
    xlsx_mit(&format!(
        "<?xml version=\"1.0\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <sheetData><row r=\"1\">{}</row><row r=\"1048576\"><c r=\"XFD1048576\"><v>1</v></c></row></sheetData></worksheet>",
        zelle("A1", "x")
    ))
}

fn eml() -> Vec<u8> {
    format!(
        "From: Anna Beispiel <anna@example.org>\r\nTo: bernd@example.com\r\nSubject: Angebot Fähre\r\n\
         Date: Mon, 2 Nov 2026 10:00:00 +0100\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n\
         Hallo Bernd,\r\nanbei das Angebot {KONTROLLE}.\r\nGruß Anna\r\n"
    )
    .into_bytes()
}

/// Minimales PDF (Helvetica, WinAnsi) mit einer Seite je Text; `extra` hängt ein zusätzliches Objekt an.
fn pdf(seiten: &[&str]) -> Vec<u8> {
    pdf_mit(
        seiten
            .iter()
            .map(|t| format!("BT /F1 12 Tf 72 720 Td ({t}) Tj ET").into_bytes())
            .collect(),
    )
}

fn pdf_mit(inhalte: Vec<Vec<u8>>) -> Vec<u8> {
    let n = inhalte.len();
    // Objekte: 1 Katalog, 2 Seiten, 3 Schrift, dann je Seite (Seite, Inhalt).
    let mut objekte: Vec<Vec<u8>> = Vec::new();
    objekte.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objekte.push(format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).into_bytes());
    objekte.push(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    );
    for (i, inhalt) in inhalte.iter().enumerate() {
        objekte.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                5 + 2 * i
            )
            .into_bytes(),
        );
        let mut s = format!("<< /Length {} >>\nstream\n", inhalt.len()).into_bytes();
        s.extend_from_slice(inhalt);
        s.extend_from_slice(b"\nendstream");
        objekte.push(s);
    }
    pdf_aus_objekten(&objekte)
}

fn pdf_aus_objekten(objekte: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objekte.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objekte.len() + 1).as_bytes(),
    );
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objekte.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// PDF, dessen Inhaltsstrom entpackt 1,2 GiB groß ist (gepackt gut 1 MiB).
fn pdf_riesig() -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let block = vec![b' '; 1024 * 1024];
    for _ in 0..1200 {
        enc.write_all(&block).unwrap();
    }
    let daten = enc.finish().unwrap();
    let mut objekte: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>".to_vec(),
    ];
    let mut s = format!(
        "<< /Length {} /Filter /FlateDecode >>\nstream\n",
        daten.len()
    )
    .into_bytes();
    s.extend_from_slice(&daten);
    s.extend_from_slice(b"\nendstream");
    objekte.push(s);
    pdf_aus_objekten(&objekte)
}

// ---------------------------------------------------------------------------
// Formate und Anker (im Prozess, schnell)
// ---------------------------------------------------------------------------

fn anker(a: &[extrakt::Abschnitt]) -> Vec<&str> {
    a.iter().map(|x| x.anker.as_str()).collect()
}

#[test]
fn docx_absaetze_mit_ueberschriftenpfad() {
    let a = extrakt::abschnitte(&vertrag_docx(), Format::Docx).unwrap();
    assert_eq!(anker(&a), ["absatz=1", "absatz=2", "absatz=3", "absatz=4"]);
    assert!(a[0].pfad.is_empty());
    assert_eq!(a[1].pfad, ["Rahmenvertrag Beispiel"]);
    assert_eq!(a[3].pfad, ["Rahmenvertrag Beispiel", "§ 4 Laufzeit"]);
    assert!(a[3].text.contains("Kündigungsfrist") && a[3].text.contains(KONTROLLE));
}

#[test]
fn odt_absaetze_und_leerzeichen() {
    let a = extrakt::abschnitte(&odt(), Format::Odt).unwrap();
    assert_eq!(anker(&a), ["absatz=1", "absatz=2", "absatz=3", "absatz=4"]);
    assert_eq!(a[1].text, "Erster Punkt mit Übergabe.");
    assert_eq!(a[3].pfad, ["Protokoll", "Beschlüsse"]);
}

#[test]
fn pptx_folien_in_praesentationsreihenfolge() {
    let a = extrakt::abschnitte(&pptx(), Format::Pptx).unwrap();
    assert_eq!(anker(&a), ["folie=1", "folie=2"]);
    assert!(a[0].text.starts_with("Folie Einhorn"));
    assert!(a[1].text.starts_with("Folie Drache"));
}

#[test]
fn odp_folien() {
    let a = extrakt::abschnitte(&odp(), Format::Odp).unwrap();
    assert_eq!(anker(&a), ["folie=1", "folie=2"]);
    assert_eq!(a[1].text, "Ausblick Mondfahrt");
}

#[test]
fn xlsx_zeilen_als_spalte_wert() {
    let a = extrakt::abschnitte(&xlsx(), Format::Tabelle).unwrap();
    assert_eq!(
        anker(&a),
        [
            "blatt=Kosten 2026&zeilen=1-20",
            "blatt=Kosten 2026&zeilen=21-25"
        ]
    );
    assert!(a[0]
        .text
        .starts_with("Posten; Betrag\nPosten: Posten 2; Betrag: 20"));
    assert!(a[1]
        .text
        .contains(&format!("Posten: Posten {KONTROLLE}; Betrag: 230")));
    assert_eq!(a[1].pfad, ["Kosten 2026"]);
}

#[test]
fn text_zeilenbloecke() {
    let a = extrakt::abschnitte(
        "Titel\nZeile zwei\n\n\nAbsatz ab Zeile fünf\n".as_bytes(),
        Format::Text,
    )
    .unwrap();
    assert_eq!(anker(&a), ["zeile=1", "zeile=5"]);
    // Latin-1 statt UTF-8 wird nicht verworfen.
    let l = extrakt::abschnitte(b"Gr\xf6\xdfe", Format::Text).unwrap();
    assert_eq!(l[0].text, "Größe");
}

#[test]
fn html_ohne_skripte_mit_ueberschriften() {
    let html = "<html><head><title>T</title><script>var geheim='SKRIPT';</script></head><body>\
        <h1>Hausordnung</h1><p>Ruhezeiten <b>ab</b> 22&nbsp;Uhr.</p><script>alert('X')</script>\
        <style>p{color:red}</style><h2>Müll</h2><p>Tonnen &amp; Säcke</p><img src=\"https://example.org/x.png\"></body></html>";
    let a = extrakt::abschnitte(html.as_bytes(), Format::Html).unwrap();
    let alles: String = a
        .iter()
        .map(|x| x.text.clone())
        .collect::<Vec<_>>()
        .join("|");
    assert!(!alles.contains("SKRIPT") && !alles.contains("alert") && !alles.contains("color"));
    assert_eq!(anker(&a), ["absatz=1", "absatz=2", "absatz=3", "absatz=4"]);
    assert_eq!(
        a[1].text,
        "Ruhezeiten ab 22\u{a0}Uhr.".replace('\u{a0}', " ")
    );
    assert_eq!(a[3].pfad, ["Hausordnung", "Müll"]);
    assert_eq!(a[3].text, "Tonnen & Säcke");
}

#[test]
fn eml_kopf_und_text() {
    let a = extrakt::abschnitte(&eml(), Format::Eml).unwrap();
    assert_eq!(anker(&a), ["teil=kopf", "teil=text"]);
    assert!(a[0].text.contains("Betreff: Angebot Fähre"));
    assert!(a[0].text.contains("Von: Anna Beispiel <anna@example.org>"));
    assert!(a[1].text.contains(KONTROLLE));
}

#[test]
fn pdf_seiten() {
    let a = extrakt::abschnitte(
        &pdf(&["Erste Seite Alpha", "", "Dritte Seite Gamma"]),
        Format::Pdf,
    )
    .unwrap();
    assert_eq!(anker(&a), ["seite=1", "seite=3"]);
    assert!(a[1].text.contains("Gamma"));
}

#[test]
fn xxe_und_doctype_abgewiesen() {
    let xxe = "<?xml version=\"1.0\"?><!DOCTYPE w [<!ENTITY xxe SYSTEM \"file:///etc/passwd\">]>\
        <w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:t>&xxe;</w:t></w:r></w:p></w:body></w:document>";
    let b = zip_bauen(&[("word/document.xml", xxe.as_bytes())]);
    assert_eq!(
        extrakt::abschnitte(&b, Format::Docx).unwrap_err().code,
        "xml_doctype"
    );
    // Ohne DOCTYPE, aber mit unbekannter Entität (auch im Attribut).
    let ent = "<w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:t>&xxe;</w:t></w:r></w:p></w:body></w:document>";
    let b = zip_bauen(&[("word/document.xml", ent.as_bytes())]);
    assert_eq!(
        extrakt::abschnitte(&b, Format::Docx).unwrap_err().code,
        "xml_entitaet"
    );
    let ent = "<w:document xmlns:w=\"w\"><w:body><w:p><w:pPr><w:pStyle w:val=\"&xxe;\"/></w:pPr></w:p></w:body></w:document>";
    let b = zip_bauen(&[("word/document.xml", ent.as_bytes())]);
    assert_eq!(
        extrakt::abschnitte(&b, Format::Docx).unwrap_err().code,
        "xml_entitaet"
    );
    // Vordefinierte Entitäten und Zeichenreferenzen bleiben erlaubt.
    let ok = "<w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:t>A &amp; B &#252;</w:t></w:r></w:p></w:body></w:document>";
    let b = zip_bauen(&[("word/document.xml", ok.as_bytes())]);
    assert_eq!(
        extrakt::abschnitte(&b, Format::Docx).unwrap()[0].text,
        "A & B ü"
    );
}

#[test]
fn zip_grenzen() {
    // Verhältnis: 8 MiB Leerzeichen packen sich auf wenige KiB.
    let bombe = vec![b' '; 8 * 1024 * 1024];
    let b = zip_bauen(&[("word/document.xml", &bombe)]);
    assert_eq!(
        extrakt::abschnitte(&b, Format::Docx).unwrap_err().code,
        "zip_grenze"
    );
    assert_eq!(
        extrakt::abschnitte(&b, Format::Tabelle).unwrap_err().code,
        "zip_grenze"
    );
    // Zu viele Einträge.
    let namen: Vec<String> = (0..10_001).map(|i| format!("x/{i}.xml")).collect();
    let eintraege: Vec<(&str, &[u8])> = namen.iter().map(|n| (n.as_str(), &b"<a/>"[..])).collect();
    let b = zip_bauen(&eintraege);
    assert_eq!(
        extrakt::abschnitte(&b, Format::Pptx).unwrap_err().code,
        "zip_grenze"
    );
    // Kein ZIP.
    assert_eq!(
        extrakt::abschnitte(b"kein zip", Format::Odt)
            .unwrap_err()
            .code,
        "zip"
    );
}

#[test]
fn textrev_haengt_an_extraktorversion() {
    let a = extrakt::textrev("ab", "pdf-extract", "0.12.0+lva1", 1);
    assert_eq!(a.len(), 64);
    assert_ne!(a, extrakt::textrev("ab", "pdf-extract", "0.12.0+lva2", 1));
    assert_ne!(a, extrakt::textrev("ab", "pdf-extract", "0.12.0+lva1", 2));
    assert_ne!(a, extrakt::textrev("ac", "pdf-extract", "0.12.0+lva1", 1));
    assert_eq!(a, extrakt::textrev("ab", "pdf-extract", "0.12.0+lva1", 1));
}

#[test]
fn stuecke_mit_ueberlappung() {
    let wort = "Wort ";
    let text = wort.repeat(100);
    let s = index::stuecke(&text, index::STUECK, index::UEBERLAPPUNG);
    assert!(s.len() >= 3);
    assert!(s.iter().all(|x| x.chars().count() <= index::STUECK));
    assert!(s.iter().all(|x| x.starts_with("Wort")));
    // Überlappung: Gesamtlänge der Stücke größer als der Text.
    let summe: usize = s.iter().map(|x| x.chars().count()).sum();
    assert!(summe > text.trim().chars().count());
    assert_eq!(index::stuecke("kurz", 220, 40), ["kurz"]);
}

// ---------------------------------------------------------------------------
// Unterprozess mit Grenzen
// ---------------------------------------------------------------------------

#[test]
fn unterprozess_liefert_markenformat_v1() {
    let b = vertrag_docx();
    let t = im_kind("v.docx", "docx", &b, None, TEST_GRENZEN).unwrap();
    assert_eq!(t.marken_version, 1);
    assert_eq!(t.original_sha256, sha(&b));
    assert_eq!(t.extraktor, "lva-office");
    assert_eq!(t.textrev, extrakt::textrev(&sha(&b), "lva-office", "1", 1));
    assert_eq!(t.abschnitte.len(), 4);
    let json = serde_json::to_value(&t).unwrap();
    assert_eq!(json["abschnitte"][3]["anker"], "absatz=4");
    assert_eq!(json["abschnitte"][3]["pfad"][1], "§ 4 Laufzeit");
}

#[test]
fn unterprozess_prueft_pruefsumme() {
    let d = datei("v.txt", b"Hallo");
    let r = textlauf::ausfuehren(
        &kind(None),
        d._d.path(),
        &d.pfad,
        &sha(b"anders"),
        "txt",
        TEST_GRENZEN,
        &AtomicBool::new(false),
    );
    assert_eq!(r.unwrap_err(), "extrakt.pruefsumme");
}

#[test]
fn unterprozess_zeitgrenze_beendet() {
    let start = std::time::Instant::now();
    let g = Grenzen {
        zeit: Duration::from_secs(2),
        speicher_mb: 1024,
    };
    let r = im_kind("e.txt", "txt", b"x", Some("endlos"), g);
    assert_eq!(r.unwrap_err(), "extrakt.zeit");
    assert!(start.elapsed() < Duration::from_secs(15));
}

#[test]
fn unterprozess_abbrechbar() {
    let d = datei("e.txt", b"x");
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        s2.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let r = textlauf::ausfuehren(
        &kind(Some("endlos")),
        d._d.path(),
        &d.pfad,
        &d.sha,
        "txt",
        TEST_GRENZEN,
        &stop,
    );
    t.join().unwrap();
    assert_eq!(r.unwrap_err(), "extrakt.abgebrochen");
}

#[cfg(unix)]
#[test]
fn unterprozess_speichergrenze_beendet() {
    assert!(im_kind("s.txt", "txt", b"x", None, TEST_GRENZEN).is_ok());
    let r = im_kind("s.txt", "txt", b"x", Some("speicher"), TEST_GRENZEN);
    assert_eq!(r.unwrap_err(), "extrakt.abbruch");
}

#[test]
fn uebergrosses_xlsx_endet_an_der_grenze() {
    // Vorab aus der Blatt-XML erkannt, bevor calamine den Range anlegt.
    let r = im_kind("r.xlsx", "xlsx", &xlsx_riesig(), None, TEST_GRENZEN);
    assert_eq!(r.unwrap_err(), "extrakt.zu_gross");
}

#[test]
fn zip_bombe_im_unterprozess() {
    let bombe = vec![b' '; 8 * 1024 * 1024];
    let b = zip_bauen(&[("word/document.xml", &bombe)]);
    let r = im_kind("b.docx", "docx", &b, None, TEST_GRENZEN);
    assert_eq!(r.unwrap_err(), "extrakt.zip_grenze");
}

#[cfg(unix)]
#[test]
fn riesiges_pdf_endet_an_der_grenze() {
    let g = Grenzen {
        zeit: Duration::from_secs(10),
        speicher_mb: 1024,
    };
    // Gegenprobe: unter denselben Grenzen läuft ein gewöhnliches PDF durch (der Abbruch unten kommt
    // also vom Inhalt, nicht vom Start des Kindes).
    assert!(im_kind("k.pdf", "pdf", &pdf(&["Klein"]), None, g).is_ok());
    let r = im_kind("r.pdf", "pdf", &pdf_riesig(), None, g);
    let code = r.unwrap_err();
    assert!(
        code == "extrakt.abbruch" || code == "extrakt.zeit",
        "unerwartet: {code}"
    );
}

#[test]
fn alle_formate_im_unterprozess() {
    for (name, typ, b, erster) in [
        ("a.odt", "odt", odt(), "absatz=1"),
        ("a.pptx", "pptx", pptx(), "folie=1"),
        ("a.odp", "odp", odp(), "folie=1"),
        ("a.xlsx", "xlsx", xlsx(), "blatt=Kosten 2026&zeilen=1-20"),
        ("a.eml", "eml", eml(), "teil=kopf"),
        ("a.pdf", "pdf", pdf(&["Seite Alpha"]), "seite=1"),
        ("a.md", "md", b"# Titel\nText".to_vec(), "zeile=1"),
        ("a.html", "html", b"<p>Hallo</p>".to_vec(), "absatz=1"),
    ] {
        let t = im_kind(name, typ, &b, None, TEST_GRENZEN).unwrap_or_else(|e| panic!("{typ}: {e}"));
        assert_eq!(t.abschnitte[0].anker, erster, "{typ}");
    }
}

// ---------------------------------------------------------------------------
// Warteschlange, Index, Suche
// ---------------------------------------------------------------------------

struct Env {
    d: tempfile::TempDir,
    a: Ablage,
}

fn env() -> Env {
    let d = tempfile::tempdir().unwrap();
    let a = Ablage::neu(d.path().join("ablage"));
    assert_eq!(a.wartung(), None);
    Env { d, a }
}

impl Env {
    fn standard(&self) -> String {
        self.a.ansicht().unwrap().ordner[0].id.clone()
    }

    fn ablegen(&self, name: &str, bytes: &[u8], ordner: &str) -> String {
        let q = self.d.path().join("q").join(name);
        fs::create_dir_all(q.parent().unwrap()).unwrap();
        fs::write(&q, bytes).unwrap();
        let z = self.a.importieren(&q, ordner, None).unwrap();
        assert_eq!(z[0].stand, AblageImportStand::Abgelegt, "{:?}", z[0]);
        z[0].dat.clone().unwrap()
    }

    fn lauf(&self) -> textfassung::Lauf {
        textfassung::lauf(
            &self.a,
            &kind(None),
            TEST_GRENZEN,
            &AtomicBool::new(false),
            &|| {},
        )
        .unwrap()
    }

    fn textstand(&self, dat: &str) -> (Option<String>, Option<String>) {
        let s = self.a.ansicht().unwrap();
        let d = s.dateien.iter().find(|d| d.id == dat).unwrap();
        (d.text.clone(), d.text_code.clone())
    }

    fn treffer(&self, q: &str) -> Vec<index::AblageSuchtreffer> {
        index::suchen(&self.a, q).unwrap()
    }
}

#[test]
fn warteschlange_textstand_und_suche_mit_anker() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("Vertrag.docx", &vertrag_docx(), &o);
    let bild = e.ablegen("Foto.png", b"\x89PNG keine Textfassung", &o);
    let bombe = zip_bauen(&[("word/document.xml", &vec![b' '; 8 * 1024 * 1024])]);
    let kaputt = e.ablegen("Bombe.docx", &bombe, &o);
    assert_eq!(e.textstand(&dat).0.as_deref(), Some("ausstehend"));
    assert_eq!(e.textstand(&bild).0, None);

    let l = e.lauf();
    assert_eq!((l.ok, l.fehler, l.abgebrochen), (1, 1, false));
    assert_eq!(e.textstand(&dat).0.as_deref(), Some("ok"));
    assert_eq!(
        e.textstand(&kaputt),
        (Some("fehler".into()), Some("extrakt.zip_grenze".into()))
    );
    // Die Textfassung liegt atomar geschrieben unter text/<dat>/<textrev>.json.
    let rev = extrakt::textrev(&sha(&vertrag_docx()), "lva-office", "1", 1);
    let json: extrakt::Textfassung =
        serde_json::from_slice(&fs::read(e.a.text_pfad(&dat, &rev)).unwrap()).unwrap();
    assert_eq!(json.textrev, rev);
    // Ein zweiter Lauf hat nichts mehr zu tun (auch die Bombe wird nicht wiederholt).
    assert_eq!(e.lauf(), textfassung::Lauf::default());

    let t = e.treffer(KONTROLLE);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].dat, dat);
    assert_eq!(t[0].name, "Vertrag.docx");
    assert_eq!(t[0].stellen[0].anker, "absatz=4");
    assert_eq!(
        t[0].stellen[0].pfad,
        ["Rahmenvertrag Beispiel", "§ 4 Laufzeit"]
    );
    assert!(t[0].stellen[0].auszug.contains(KONTROLLE));
    // Umlaute und Groß/Klein (FTS5 unicode61).
    assert_eq!(e.treffer("kündigungsfrist").len(), 1);
    assert!(e.treffer("Nichtvorhanden").is_empty());
    assert!(e.treffer("  ").is_empty());
}

#[test]
fn papierkorb_null_treffer_wiederherstellen_und_loeschen() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen(
        "Notiz.txt",
        format!("Erste Zeile\nDas {KONTROLLE} singt.").as_bytes(),
        &o,
    );
    e.lauf();
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    assert_eq!(e.treffer(KONTROLLE)[0].stellen[0].anker, "zeile=1");

    e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    index::abgleichen(&e.a).unwrap();
    assert!(e.treffer(KONTROLLE).is_empty());
    // Auch ohne Abgleich filtert die Suche gegen den Stand (Ordner im Papierkorb).
    e.a.wiederherstellen(AblageArt::Datei, &dat).unwrap();
    index::abgleichen(&e.a).unwrap();
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    e.a.in_papierkorb(AblageArt::Ordner, &o).unwrap();
    assert!(e.treffer(KONTROLLE).is_empty());
    e.a.wiederherstellen(AblageArt::Ordner, &o).unwrap();
    index::abgleichen(&e.a).unwrap();
    assert_eq!(e.treffer(KONTROLLE).len(), 1);

    e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    e.a.endgueltig_loeschen(AblageArt::Datei, &dat).unwrap();
    let b = index::abgleichen(&e.a).unwrap();
    assert_eq!((b.entfernt, b.stuecke), (1, 0));
    assert!(e.treffer(KONTROLLE).is_empty());
}

#[test]
fn verschieben_aktualisiert_ordnerkette_inkrementell() {
    let e = env();
    let o = e.standard();
    let ziel = e.a.ordner_anlegen(Some(&o), "Unterordner").unwrap();
    let dat = e.ablegen("a.md", b"Inhalt Wal", &o);
    e.ablegen("b.md", b"Inhalt Hai", &o);
    e.lauf();
    assert_eq!(index::abgleichen(&e.a).unwrap().geschrieben, 0);
    e.a.datei_verschieben(&dat, &ziel).unwrap();
    let b = index::abgleichen(&e.a).unwrap();
    assert_eq!(b.geschrieben, 1);
    let c = rusqlite::Connection::open(e.a.dir().join(index::INDEX)).unwrap();
    let kette: String = c
        .query_row(
            "SELECT ordnerkette FROM ablage_dok WHERE dat = ?1",
            [&dat],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kette, format!("{ziel}/{o}"));
    assert_eq!(e.treffer("Wal")[0].dat, dat);
}

#[test]
fn neuaufbau_idempotent() {
    let e = env();
    let o = e.standard();
    e.ablegen("v.docx", &vertrag_docx(), &o);
    e.ablegen("t.xlsx", &xlsx(), &o);
    e.ablegen("m.eml", &eml(), &o);
    e.lauf();
    let vorher = e.treffer(KONTROLLE);
    assert_eq!(vorher.len(), 3);
    let erst = index::neu_aufbauen(&e.a).unwrap();
    let dann = index::neu_aufbauen(&e.a).unwrap();
    assert_eq!(erst, dann);
    assert_eq!(erst.geschrieben, 3);
    assert_eq!(index::abgleichen(&e.a).unwrap().geschrieben, 0);
    let nachher = e.treffer(KONTROLLE);
    let ids = |t: &[index::AblageSuchtreffer]| {
        let mut v: Vec<(String, String)> = t
            .iter()
            .map(|x| (x.dat.clone(), x.stellen[0].anker.clone()))
            .collect();
        v.sort();
        v
    };
    assert_eq!(ids(&vorher), ids(&nachher));
    let anker: Vec<String> = nachher.iter().map(|t| t.stellen[0].anker.clone()).collect();
    assert!(anker.contains(&"blatt=Kosten 2026&zeilen=21-25".to_string()));
    assert!(anker.contains(&"teil=text".to_string()));
    // Ein beschädigter Index wird durch den Neuaufbau ersetzt.
    fs::write(e.a.dir().join(index::INDEX), b"kaputt").unwrap();
    assert!(index::suchen(&e.a, KONTROLLE).is_err());
    index::neu_aufbauen(&e.a).unwrap();
    assert_eq!(e.treffer(KONTROLLE).len(), 3);
}

#[test]
fn extraktorwechsel_ergibt_neue_textrev_alte_bleibt() {
    let e = env();
    let o = e.standard();
    let bytes = format!("Altbestand {KONTROLLE}").into_bytes();
    let dat = e.ablegen("alt.txt", &bytes, &o);
    // Eine Textfassung einer älteren Extraktorversion liegt schon vor.
    let fas = e.a.fassungen(&dat).unwrap()[0].fas.clone();
    let alt_rev = extrakt::textrev(&sha(&bytes), "lva-text", "0", 1);
    let alt = extrakt::Textfassung {
        marken_version: 1,
        textrev: alt_rev.clone(),
        original_sha256: sha(&bytes),
        extraktor: "lva-text".into(),
        extraktor_version: "0".into(),
        abschnitte: vec![extrakt::Abschnitt {
            anker: "zeile=1".into(),
            pfad: vec![],
            text: format!("Altbestand {KONTROLLE}"),
        }],
    };
    let json = serde_json::to_vec(&alt).unwrap();
    let r =
        e.a.text_ablegen(
            &dat,
            &fas,
            Some(&json),
            Text {
                textrev: alt_rev.clone(),
                blob: String::new(),
                size: 0,
                extraktor: "lva-text".into(),
                extraktor_version: "0".into(),
                marken_version: 1,
                stand: "ok".into(),
                code: None,
                versuche: 0,
            },
        )
        .unwrap();
    assert!(matches!(r, TextAblage::Eingetragen(ref t) if t.size == json.len() as u64));
    // Die alte gilt bis zum Neuaufbau: suchbar, Textstand aber „ausstehend“.
    index::abgleichen(&e.a).unwrap();
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    assert_eq!(e.textstand(&dat).0.as_deref(), Some("ausstehend"));
    let auftraege = e.a.text_auftraege().unwrap();
    assert_eq!(auftraege.len(), 1);
    assert_ne!(auftraege[0].textrev, alt_rev);

    assert_eq!(e.lauf().ok, 1);
    assert_eq!(e.textstand(&dat).0.as_deref(), Some("ok"));
    let neu_rev = extrakt::textrev(&sha(&bytes), "lva-text", "1", 1);
    assert!(e.a.text_pfad(&dat, &alt_rev).is_file());
    assert!(e.a.text_pfad(&dat, &neu_rev).is_file());
    let c = rusqlite::Connection::open(e.a.dir().join(index::INDEX)).unwrap();
    let rev: String = c
        .query_row(
            "SELECT textrev FROM ablage_dok WHERE dat = ?1",
            [&dat],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rev, neu_rev);
}

#[test]
fn wiederanlauf_und_abbruch() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("w.txt", format!("Wiederanlauf {KONTROLLE}").as_bytes(), &o);
    // Abbruch vor dem ersten Auftrag: nichts eingetragen, Datei bleibt ausstehend.
    let stop = AtomicBool::new(true);
    let l = textfassung::lauf(&e.a, &kind(None), TEST_GRENZEN, &stop, &|| {}).unwrap();
    assert!(l.abgebrochen);
    assert_eq!(e.textstand(&dat).0.as_deref(), Some("ausstehend"));
    // „Neustart“: eine frische Ablage findet den offenen Auftrag aus dem Stand.
    let neu = Ablage::neu(e.a.dir());
    neu.prozess_neustart();
    assert_eq!(neu.text_auftraege().unwrap().len(), 1);
    let l = textfassung::lauf(
        &neu,
        &kind(None),
        TEST_GRENZEN,
        &AtomicBool::new(false),
        &|| {},
    )
    .unwrap();
    assert_eq!(l.ok, 1);
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
}

#[test]
fn verknuepfter_textordner_wird_abgewiesen() {
    #[cfg(unix)]
    {
        let e = env();
        let o = e.standard();
        let dat = e.ablegen("v.txt", b"Text", &o);
        let fremd = e.d.path().join("fremd");
        fs::create_dir_all(&fremd).unwrap();
        fs::create_dir_all(e.a.dir().join("text")).unwrap();
        std::os::unix::fs::symlink(&fremd, e.a.dir().join("text").join(&dat)).unwrap();
        let l = e.lauf();
        assert_eq!((l.ok, l.fehler), (0, 1));
        assert_eq!(fs::read_dir(&fremd).unwrap().count(), 0);
        // Nichts eingetragen: der nächste Start versucht es erneut.
        assert_eq!(e.textstand(&dat).0.as_deref(), Some("ausstehend"));
    }
}

// ---------------------------------------------------------------------------
// Nachbesserungen aus dem Review (Regressionstests)
// ---------------------------------------------------------------------------

/// Setzt die unkomprimierte Größe jedes Eintrags (lokaler Kopf und Zentralverzeichnis) auf `n`.
fn groesse_luegen(mut b: Vec<u8>, n: u32) -> Vec<u8> {
    let le = n.to_le_bytes();
    let mut i = 0;
    while i + 4 <= b.len() {
        let off = match &b[i..i + 4] {
            b"PK\x03\x04" => Some(22),
            b"PK\x01\x02" => Some(24),
            _ => None,
        };
        if let Some(o) = off {
            b[i + o..i + o + 4].copy_from_slice(&le);
            i += 4;
        } else {
            i += 1;
        }
    }
    b
}

#[test]
fn gelogene_groesse_im_zip_bricht_ab() {
    let mut inhalt = String::from("<?xml version=\"1.0\"?><worksheet><sheetData>");
    for i in 0..4000 {
        inhalt.push_str(&format!("<row r=\"{}\"/>", i + 1));
    }
    inhalt.push_str("</sheetData></worksheet>");
    let x = groesse_luegen(xlsx_mit(&inhalt), 10);
    let langer: Vec<(Option<&str>, &str)> = vec![(None, "Absatz mit Text"); 2000];
    let d = groesse_luegen(docx(&langer), 10);
    for (b, f) in [(x, Format::Tabelle), (d, Format::Docx)] {
        let code = extrakt::abschnitte(&b, f).unwrap_err().code;
        assert!(code == "zip_grenze" || code == "zip", "{f:?}: {code}");
    }
    // Direkt am Leser: nie mehr als angegeben + 1 Byte.
    let roh = groesse_luegen(zip_bauen(&[("a.xml", &vec![b'a'; 50_000])]), 10);
    let mut z = extrakt::zipsicher::ZipLeser::oeffnen(&roh).unwrap();
    assert!(z.lesen("a.xml").is_err());
}

#[test]
fn xlsx_a1_und_xfd1048576_vorab_abgewiesen() {
    let start = std::time::Instant::now();
    assert_eq!(
        extrakt::abschnitte(&xlsx_riesig(), Format::Tabelle)
            .unwrap_err()
            .code,
        "zu_gross"
    );
    // Ohne Range-Aufbau: sofort, ohne Gigabytes.
    assert!(start.elapsed() < Duration::from_secs(5));
    // Leere Zellen weit draußen (ohne Wert) zählen nicht.
    let ok = xlsx_mit(&format!(
        "<?xml version=\"1.0\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <sheetData><row r=\"1\">{}</row><row r=\"1048576\"><c r=\"XFD1048576\" s=\"1\"/></row></sheetData></worksheet>",
        zelle("A1", "Kopf")
    ));
    assert_eq!(
        extrakt::abschnitte(&ok, Format::Tabelle).unwrap()[0].text,
        "Kopf"
    );
}

#[test]
fn doctype_in_tabellen_xml_abgewiesen() {
    let blatt = "<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY a \"b\">]><worksheet><sheetData/></worksheet>";
    assert_eq!(
        extrakt::abschnitte(&xlsx_mit(blatt), Format::Tabelle)
            .unwrap_err()
            .code,
        "xml_doctype"
    );
    // Auch in Nebenteilen (hier Arbeitsmappe, über den Inhaltstyp-Teil gleich mit).
    let b = zip_bauen(&[
        (
            "xl/workbook.xml",
            b"<?xml version=\"1.0\"?><!DOCTYPE w SYSTEM \"http://example.org/w.dtd\"><workbook/>",
        ),
        ("xl/worksheets/sheet1.xml", b"<worksheet/>"),
    ]);
    assert_eq!(
        extrakt::abschnitte(&b, Format::Tabelle).unwrap_err().code,
        "xml_doctype"
    );
    let ods = ods_mit("<?xml version=\"1.0\"?><!DOCTYPE c><office:document-content/>");
    assert_eq!(
        extrakt::abschnitte(&ods, Format::Tabelle).unwrap_err().code,
        "xml_doctype"
    );
}

fn ods_mit(content: &str) -> Vec<u8> {
    let manifest = "<?xml version=\"1.0\"?><manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\">\
        <manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.spreadsheet\"/>\
        <manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>";
    zip_bauen(&[
        (
            "mimetype",
            b"application/vnd.oasis.opendocument.spreadsheet",
        ),
        ("META-INF/manifest.xml", manifest.as_bytes()),
        ("content.xml", content.as_bytes()),
    ])
}

fn ods_tabelle(zeilen: &str) -> Vec<u8> {
    ods_mit(&format!(
        "<?xml version=\"1.0\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
         xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\">\
         <office:body><office:spreadsheet><table:table table:name=\"Liste\">{zeilen}</table:table>\
         </office:spreadsheet></office:body></office:document-content>"
    ))
}

fn ods_zelle(t: &str) -> String {
    format!(
        "<table:table-cell office:value-type=\"string\"><text:p>{t}</text:p></table:table-cell>"
    )
}

#[test]
fn ods_wiederholungen_vorab_begrenzt() {
    // Üblich: leere Wiederholungen am Ende (LibreOffice) bleiben erlaubt.
    let ok = ods_tabelle(&format!(
        "<table:table-row>{}{}<table:table-cell table:number-columns-repeated=\"16382\"/></table:table-row>\
         <table:table-row>{}{}<table:table-cell table:number-columns-repeated=\"16382\"/></table:table-row>\
         <table:table-row table:number-rows-repeated=\"1048574\"><table:table-cell table:number-columns-repeated=\"16384\"/></table:table-row>",
        ods_zelle("Name"),
        ods_zelle("Ort"),
        ods_zelle("Anna"),
        ods_zelle("Ulm")
    ));
    let a = extrakt::abschnitte(&ok, Format::Tabelle).unwrap();
    assert_eq!(anker(&a), ["blatt=Liste&zeilen=1-2"]);
    assert!(a[0].text.contains("Name: Anna; Ort: Ulm"));
    // Belegte Zelle 20 000-fach in 1 000 wiederholten Zeilen: 20 Mio. Zellen.
    let breit = ods_tabelle(
        "<table:table-row table:number-rows-repeated=\"1000\"><table:table-cell office:value-type=\"float\" \
         office:value=\"1\" table:number-columns-repeated=\"20000\"><text:p>1</text:p></table:table-cell></table:table-row>",
    );
    assert_eq!(
        extrakt::abschnitte(&breit, Format::Tabelle)
            .unwrap_err()
            .code,
        "zu_gross"
    );
    // Zwei Werte weit auseinander, dazwischen eine Million leerer Zeilen: calamine füllt auf.
    let weit = ods_tabelle(&format!(
        "<table:table-row>{}</table:table-row>\
         <table:table-row table:number-rows-repeated=\"1000000\"><table:table-cell/></table:table-row>\
         <table:table-row><table:table-cell table:number-columns-repeated=\"100\"/>{}</table:table-row>",
        ods_zelle("a"),
        ods_zelle("b")
    ));
    assert_eq!(
        extrakt::abschnitte(&weit, Format::Tabelle)
            .unwrap_err()
            .code,
        "zu_gross"
    );
}

#[test]
fn zu_grosse_ausgabe_als_zu_gross() {
    let start = std::time::Instant::now();
    let r = im_kind("f.txt", "txt", b"x", Some("flut"), TEST_GRENZEN);
    assert_eq!(r.unwrap_err(), "extrakt.zu_gross");
    // Sofort erkannt, nicht erst an der Zeitgrenze.
    assert!(start.elapsed() < TEST_GRENZEN.zeit);
}

#[test]
fn eingabe_ohne_verknuepfung_und_mit_groessengrenze() {
    let d = datei("g.txt", b"Hallo");
    assert_eq!(extrakt::datei_lesen(&d.pfad, 100).unwrap(), b"Hallo");
    assert_eq!(
        extrakt::datei_lesen(&d.pfad, 3).unwrap_err().code,
        "zu_gross"
    );
    #[cfg(unix)]
    {
        let link = d._d.path().join("l.txt");
        std::os::unix::fs::symlink(&d.pfad, &link).unwrap();
        assert_eq!(extrakt::datei_lesen(&link, 100).unwrap_err().code, "lesen");
        // Auch über den Unterprozess.
        let r = textlauf::ausfuehren(
            &kind(None),
            d._d.path(),
            &link,
            &d.sha,
            "txt",
            TEST_GRENZEN,
            &AtomicBool::new(false),
        );
        assert_eq!(r.unwrap_err(), "extrakt.lesen");
    }
}

#[test]
fn extraktor_nur_mit_kennzeichen_und_unter_der_wurzel() {
    let d = datei("k.txt", b"Kennzeichen");
    let p = kind(None);
    let lauf = |kennung: Option<&str>, stdin: &[u8]| {
        let mut c = std::process::Command::new(&p.pfad);
        c.args(&p.vorweg)
            .arg(&d.pfad)
            .arg(&d.sha)
            .arg("txt")
            .env("LVA_EXTRAKT_KIND", "1")
            .env(extrakt::ENV_WURZEL, d._d.path())
            .env_remove(extrakt::ENV_KENNZEICHEN)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        if let Some(k) = kennung {
            c.env(extrakt::ENV_KENNZEICHEN, k);
        }
        let mut kind = c.spawn().unwrap();
        kind.stdin.take().unwrap().write_all(stdin).unwrap();
        let aus = kind.wait_with_output().unwrap();
        (aus.status.code(), aus.stdout)
    };
    let k = "ab".repeat(32);
    // Ohne Kennzeichen (z. B. Start über eine Dateizuordnung): keine Ausgabe, Exitcode 64.
    let (code, aus) = lauf(None, format!("{k}\n").as_bytes());
    assert_eq!(code, Some(64));
    assert!(!aus.contains(&b'{'));
    // Kennzeichen nur in der Umgebung, nicht auf stdin.
    let (code, aus) = lauf(Some(&k), b"falsch\n");
    assert_eq!(code, Some(64));
    assert!(!aus.contains(&b'{'));
    // Beides: angenommen.
    let (code, aus) = lauf(Some(&k), format!("{k}\n").as_bytes());
    assert_eq!(code, Some(0));
    assert!(aus.contains(&b'{'));
    // Pfad außerhalb der Wurzel: keine Antwort (für den Elternprozess ein Abbruch).
    let andere = tempfile::tempdir().unwrap();
    let r = textlauf::ausfuehren(
        &p,
        andere.path(),
        &d.pfad,
        &d.sha,
        "txt",
        TEST_GRENZEN,
        &AtomicBool::new(false),
    );
    assert_eq!(r.unwrap_err(), "extrakt.abbruch");
}

fn fas_von(e: &Env, dat: &str) -> String {
    e.a.fassungen(dat).unwrap()[0].fas.clone()
}

fn kaputtes_programm() -> Programm {
    Programm {
        pfad: PathBuf::from("/nicht/vorhanden/lva-extrakt"),
        vorweg: vec![],
        umgebung: vec![],
    }
}

#[test]
fn voruebergehende_fehler_begrenzt_wiederholt() {
    let e = env();
    let o = e.standard();
    let gut = e.ablegen("gut.txt", format!("Später {KONTROLLE}").as_bytes(), &o);
    let fas = fas_von(&e, &gut);
    let schlecht = |e: &Env| {
        textfassung::lauf(
            &e.a,
            &kaputtes_programm(),
            TEST_GRENZEN,
            &AtomicBool::new(false),
            &|| {},
        )
        .unwrap()
    };
    // Erster Versuch: vorübergehender Fehler, vermerkt mit Zähler.
    assert_eq!(schlecht(&e).fehler, 1);
    let (stand, code) = e.textstand(&gut);
    assert_eq!(stand.as_deref(), Some("fehler"));
    let code = code.unwrap();
    assert!(speicher::VORUEBERGEHEND.contains(&code.as_str()), "{code}");
    // Im selben Prozess nicht sofort wieder (Rückstellung), auch kein Wiederanlauf.
    assert_eq!(schlecht(&e), textfassung::Lauf::default());
    assert!(!textfassung::offen(&e.a));
    // Nächster Start: erneut offen, mit dem bisherigen Zähler; jetzt klappt es.
    textfassung::rueckstellung_vergessen(&fas);
    let auf = e.a.text_auftraege().unwrap();
    assert_eq!((auf.len(), auf[0].versuche), (1, 1));
    assert!(textfassung::offen(&e.a));
    assert_eq!(e.lauf().ok, 1);
    assert_eq!(e.textstand(&gut), (Some("ok".into()), None));
    assert_eq!(e.treffer(KONTROLLE).len(), 1);

    // Höchstens drei Versuche, dann endgültig.
    let dat = e.ablegen("nie.txt", b"Nie", &o);
    let fas = fas_von(&e, &dat);
    for versuch in 1..=speicher::MAX_VERSUCHE {
        textfassung::rueckstellung_vergessen(&fas);
        assert_eq!(schlecht(&e).fehler, 1, "Versuch {versuch}");
    }
    textfassung::rueckstellung_vergessen(&fas);
    assert!(e.a.text_auftraege().unwrap().is_empty());
    assert_eq!(e.textstand(&dat).0.as_deref(), Some("fehler"));

    // Ein Fehler am Inhalt ist sofort endgültig.
    let bombe = zip_bauen(&[("word/document.xml", &vec![b' '; 8 * 1024 * 1024])]);
    let b = e.ablegen("b.docx", &bombe, &o);
    e.lauf();
    textfassung::rueckstellung_vergessen(&fas_von(&e, &b));
    assert!(e.a.text_auftraege().unwrap().is_empty());
}

#[test]
fn loeschen_waehrend_extraktion_hinterlaesst_nichts() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("weg.txt", format!("Flüchtig {KONTROLLE}").as_bytes(), &o);
    let auf = e.a.text_auftraege().unwrap().remove(0);
    // Der Extraktor braucht 1,5 s; währenddessen wird die Datei endgültig gelöscht.
    let dir = e.a.dir().to_path_buf();
    let d2 = dat.clone();
    let loescher = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        let a = Ablage::neu(dir);
        a.in_papierkorb(AblageArt::Datei, &d2).unwrap();
        a.endgueltig_loeschen(AblageArt::Datei, &d2).unwrap();
    });
    let aus = textfassung::bearbeiten(
        &e.a,
        &auf,
        &kind(Some("langsam")),
        TEST_GRENZEN,
        &AtomicBool::new(false),
    )
    .unwrap();
    loescher.join().unwrap();
    assert_eq!(aus, textfassung::Ausgang::Verworfen);
    assert!(!e.a.dir().join("text").join(&dat).exists());
    let b = index::abgleichen(&e.a).unwrap();
    assert_eq!(b.stuecke, 0);
    assert!(e.treffer(KONTROLLE).is_empty());

    // Wartung räumt einen liegengebliebenen Textordner ohne Datei weg (nicht den einer lebenden Datei).
    let lebend = e.ablegen("da.txt", b"Da", &o);
    e.lauf();
    let rest =
        e.a.dir()
            .join("text")
            .join(super::neue_id(super::PRAEFIX_DATEI));
    fs::create_dir_all(&rest).unwrap();
    fs::write(rest.join("x.json"), b"{}").unwrap();
    assert_eq!(e.a.wartung(), None);
    assert!(!rest.exists());
    assert!(e.a.dir().join("text").join(&lebend).is_dir());

    // Verwaiste Stücke und Dokumente im Index fallen beim Abgleich weg.
    let c = rusqlite::Connection::open(e.a.dir().join(index::INDEX)).unwrap();
    c.execute(
        "INSERT INTO ablage_stueck(dat, seite, abschnitt, anker, pfad) VALUES('dat_waise', 1, 0, 'zeile=1', '[]')",
        [],
    )
    .unwrap();
    let vorher: i64 = c
        .query_row("SELECT COUNT(*) FROM ablage_stueck", [], |r| r.get(0))
        .unwrap();
    let b = index::abgleichen(&e.a).unwrap();
    assert_eq!(b.stuecke as i64, vorher - 1);
}

#[test]
fn abbruch_bleibt_bis_zum_ausdruecklichen_anstoss() {
    let s = textfassung::Steuerung::default();
    let (a, b, c) = (
        PathBuf::from("/a"),
        PathBuf::from("/b"),
        PathBuf::from("/c"),
    );
    let ab = |s: &textfassung::Steuerung| s.abbrechen.load(std::sync::atomic::Ordering::Relaxed);
    s.anstossen(a.clone());
    assert_eq!(s.nehmen(), Some(a));
    // Lauf läuft; Abbruch durch die Bedienung.
    s.abbrechen();
    assert!(ab(&s) && s.angehalten());
    // Neuladen der Seite startet nicht neu.
    assert!(!s.wiederanlauf(b.clone()));
    assert_eq!(s.nehmen(), None);
    // Ein ausdrücklicher Anstoß während des abbrechenden Laufs setzt dessen Abbruch nicht zurück …
    s.anstossen(c.clone());
    assert!(ab(&s));
    assert!(!s.angehalten());
    // … erst der nächste Lauf beginnt ohne Abbruch.
    assert_eq!(s.nehmen(), Some(c));
    assert!(!ab(&s));
    assert!(s.wiederanlauf(b.clone()));
    assert_eq!(s.nehmen(), Some(b));
}
