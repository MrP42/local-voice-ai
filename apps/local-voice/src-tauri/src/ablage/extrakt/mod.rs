//! Textfassung einer abgelegten Fassung erzeugen (Spec §4.1/§4.2, ADR-0029 E9). Läuft im eigenen
//! Unterprozess `lva-extrakt` (siehe [`super::textlauf`]); hier liegt nur die reine Umwandlung
//! Bytes → Abschnitte mit Ankern, ohne Netz, ohne Log, ohne Dateizugriff außer der einen Eingabedatei.
//!
//! Markenformat v1 (Spec §4.2):
//!
//! ```json
//! {"marken_version": 1, "textrev": "<hex>", "original_sha256": "<hex>", "extraktor": "…",
//!  "extraktor_version": "…", "abschnitte": [{"anker": "seite=7", "pfad": ["§ 4 Laufzeit"], "text": "…"}]}
//! ```
//!
//! `textrev` = SHA-256 über `original_sha256 ‖ extraktor ‖ extraktor_version ‖ marken_version` (UTF-8,
//! Felder durch ein Nullbyte getrennt, `marken_version` dezimal), klein hexadezimal. Eine neue
//! Extraktorversion ergibt damit eine neue `textrev`; die alte Textfassung bleibt bis zum Neuaufbau.

mod einfach;
mod office;
mod tabelle;
pub mod zipsicher;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MARKEN_VERSION: u32 = 1;
/// Mehr Text als das wird nicht ausgegeben (Schutz der App vor riesigen Ausgaben).
pub const MAX_TEXT: usize = 64 * 1024 * 1024;
/// Kommandozeilenschalter, mit dem das App-Programm selbst als Extraktor läuft (wenn kein eigenes
/// `lva-extrakt` neben dem Programm liegt).
pub const SCHALTER: &str = "--lva-extrakt";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Abschnitt {
    pub anker: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pfad: Vec<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Textfassung {
    pub marken_version: u32,
    pub textrev: String,
    pub original_sha256: String,
    pub extraktor: String,
    pub extraktor_version: String,
    pub abschnitte: Vec<Abschnitt>,
}

/// Fehler mit stabilem Code (nie Inhalt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraktFehler {
    pub code: &'static str,
}

impl ExtraktFehler {
    pub fn neu(code: &'static str) -> Self {
        Self { code }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Docx,
    Odt,
    Pptx,
    Odp,
    Tabelle,
    Text,
    Html,
    Eml,
}

/// Format nach der gespeicherten Endung (`typ` der Fassung); `None` = keine Textfassung (z. B. Bilder).
pub fn format_aus_typ(typ: &str) -> Option<Format> {
    Some(match typ.to_ascii_lowercase().as_str() {
        "pdf" => Format::Pdf,
        "docx" | "docm" => Format::Docx,
        "odt" => Format::Odt,
        "pptx" | "pptm" => Format::Pptx,
        "odp" => Format::Odp,
        "xlsx" | "xlsm" | "ods" => Format::Tabelle,
        "txt" | "md" | "markdown" | "csv" | "tsv" | "log" => Format::Text,
        "html" | "htm" | "xhtml" => Format::Html,
        "eml" => Format::Eml,
        _ => return None,
    })
}

/// Kennung und Version des Extraktors je Format. Die Version wird bei jeder Änderung der Ausgabe erhöht
/// (neue `textrev`); bei Bibliotheken steht deren Version vorn.
pub fn extraktor(f: Format) -> (&'static str, &'static str) {
    match f {
        Format::Pdf => ("pdf-extract", "0.12.0+lva1"),
        Format::Docx | Format::Odt | Format::Pptx | Format::Odp => ("lva-office", "1"),
        Format::Tabelle => ("calamine", "0.32.0+lva1"),
        Format::Text => ("lva-text", "1"),
        Format::Html => ("lva-html", "1"),
        Format::Eml => ("lva-mail", "1"),
    }
}

pub fn textrev(original_sha256: &str, extraktor: &str, version: &str, marken: u32) -> String {
    let mut h = Sha256::new();
    h.update(original_sha256.as_bytes());
    h.update([0u8]);
    h.update(extraktor.as_bytes());
    h.update([0u8]);
    h.update(version.as_bytes());
    h.update([0u8]);
    h.update(marken.to_string().as_bytes());
    hex(&h.finalize())
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Wandelt die Bytes einer Fassung in Abschnitte (leere Abschnitte fallen weg).
pub fn abschnitte(bytes: &[u8], f: Format) -> Result<Vec<Abschnitt>, ExtraktFehler> {
    let roh = match f {
        Format::Pdf => pdf(bytes)?,
        Format::Docx => office::docx(bytes)?,
        Format::Odt => office::odt(bytes)?,
        Format::Pptx => office::pptx(bytes)?,
        Format::Odp => office::odp(bytes)?,
        Format::Tabelle => tabelle::tabelle(bytes)?,
        Format::Text => einfach::text(bytes),
        Format::Html => einfach::html(bytes),
        Format::Eml => einfach::eml(bytes)?,
    };
    let mut gesamt = 0usize;
    let mut aus = Vec::with_capacity(roh.len());
    for mut a in roh {
        a.text = a.text.trim().to_string();
        if a.text.is_empty() {
            continue;
        }
        gesamt += a.text.len();
        if gesamt > MAX_TEXT {
            return Err(ExtraktFehler::neu("zu_gross"));
        }
        aus.push(a);
    }
    Ok(aus)
}

/// Vollständige Textfassung nach Markenformat v1.
pub fn textfassung(
    bytes: &[u8],
    original_sha256: &str,
    f: Format,
) -> Result<Textfassung, ExtraktFehler> {
    let (id, version) = extraktor(f);
    Ok(Textfassung {
        marken_version: MARKEN_VERSION,
        textrev: textrev(original_sha256, id, version, MARKEN_VERSION),
        original_sha256: original_sha256.to_string(),
        extraktor: id.to_string(),
        extraktor_version: version.to_string(),
        abschnitte: abschnitte(bytes, f)?,
    })
}

fn pdf(bytes: &[u8]) -> Result<Vec<Abschnitt>, ExtraktFehler> {
    // pdf-extract kann bei beschädigten Dateien in Panik geraten; das beendet hier nur den Unterprozess,
    // aber ein sauberer Fehlercode ist freundlicher.
    let seiten = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(bytes))
        .map_err(|_| ExtraktFehler::neu("pdf"))?
        .map_err(|_| ExtraktFehler::neu("pdf"))?;
    Ok(seiten
        .into_iter()
        .enumerate()
        .map(|(i, text)| Abschnitt {
            anker: format!("seite={}", i + 1),
            pfad: vec![],
            text,
        })
        .collect())
}

/// Antwort des Unterprozesses auf stdout: Textfassung oder Fehlercode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Antwort {
    Ok(Textfassung),
    Fehler { fehler: String },
}

/// Einstieg des Unterprozesses: `<pfad> <original_sha256> <typ>`. Schreibt genau ein JSON-Dokument
/// auf stdout; Rückgabe ist der Exitcode (0 = Textfassung, 2 = Fehlercode, 64 = Aufruf falsch).
/// Nie Inhalt auf stderr.
pub fn haupt(args: &[String]) -> i32 {
    let [pfad, sha, typ] = args else {
        return 64;
    };
    let antwort = match ausfuehren(std::path::Path::new(pfad), sha, typ) {
        Ok(t) => Antwort::Ok(t),
        Err(e) => Antwort::Fehler {
            fehler: e.code.to_string(),
        },
    };
    let code = if matches!(antwort, Antwort::Ok(_)) {
        0
    } else {
        2
    };
    let Ok(json) = serde_json::to_vec(&antwort) else {
        return 70;
    };
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    if out.write_all(&json).is_err() || out.flush().is_err() {
        return 74;
    }
    code
}

fn ausfuehren(pfad: &std::path::Path, sha: &str, typ: &str) -> Result<Textfassung, ExtraktFehler> {
    let f = format_aus_typ(typ).ok_or(ExtraktFehler::neu("format"))?;
    #[cfg(test)]
    tests_kind::haken(typ);
    // Keine Verknüpfung, nur eine gewöhnliche Datei (der Aufrufer hat den Pfad unter `ablage/` geprüft).
    let meta = std::fs::symlink_metadata(pfad).map_err(|_| ExtraktFehler::neu("lesen"))?;
    if super::ist_umweg(&meta) || !meta.is_file() {
        return Err(ExtraktFehler::neu("lesen"));
    }
    let bytes = std::fs::read(pfad).map_err(|_| ExtraktFehler::neu("lesen"))?;
    // Erst die Prüfsumme, dann der Inhalt: Text und Original stammen aus demselben Stand (E9).
    if hex(&Sha256::digest(&bytes)) != sha.to_ascii_lowercase() {
        return Err(ExtraktFehler::neu("pruefsumme"));
    }
    textfassung(&bytes, &sha.to_ascii_lowercase(), f)
}

/// Testhaken des Kindprozesses: simuliert einen hängenden oder speicherhungrigen Extraktor.
#[cfg(test)]
pub(crate) mod tests_kind {
    pub fn haken(typ: &str) {
        match std::env::var("LVA_EXTRAKT_TEST").as_deref() {
            Ok("endlos") if typ == "txt" => loop {
                std::hint::black_box(0u64);
            },
            Ok("speicher") if typ == "txt" => {
                let mut v: Vec<Vec<u8>> = Vec::new();
                loop {
                    v.push(vec![1u8; 64 * 1024 * 1024]);
                    std::hint::black_box(&v);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// XML ohne DOCTYPE und ohne Entitäten
// ---------------------------------------------------------------------------

pub(crate) enum X {
    Start(String, Vec<(String, String)>),
    Leer(String, Vec<(String, String)>),
    Ende(String),
    Text(String),
    Eof,
}

pub(crate) struct XmlLeser<'a> {
    r: quick_xml::Reader<&'a [u8]>,
}

fn xml_fehler() -> ExtraktFehler {
    ExtraktFehler::neu("xml")
}

fn attribute(
    e: &quick_xml::events::BytesStart<'_>,
    decoder: quick_xml::encoding::Decoder,
) -> Result<Vec<(String, String)>, ExtraktFehler> {
    let mut aus = Vec::new();
    // Ohne Doppelprüfung der Attributnamen: die ist in quick-xml 0.38 quadratisch (RUSTSEC-2026-0194).
    for a in e.attributes().with_checks(false) {
        let a = a.map_err(|_| xml_fehler())?;
        let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let v = a
            .decode_and_unescape_value(decoder)
            .map_err(|_| ExtraktFehler::neu("xml_entitaet"))?
            .into_owned();
        aus.push((k, v));
    }
    Ok(aus)
}

impl<'a> XmlLeser<'a> {
    pub fn neu(bytes: &'a [u8]) -> Self {
        Self {
            r: quick_xml::Reader::from_reader(bytes),
        }
    }

    pub fn naechstes(&mut self) -> Result<X, ExtraktFehler> {
        use quick_xml::events::Event;
        loop {
            let ev = self.r.read_event().map_err(|_| xml_fehler())?;
            return Ok(match ev {
                Event::DocType(_) => return Err(ExtraktFehler::neu("xml_doctype")),
                Event::Start(e) => X::Start(
                    String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    attribute(&e, self.r.decoder())?,
                ),
                Event::Empty(e) => X::Leer(
                    String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    attribute(&e, self.r.decoder())?,
                ),
                Event::End(e) => X::Ende(String::from_utf8_lossy(e.name().as_ref()).into_owned()),
                Event::Text(t) => X::Text(t.decode().map_err(|_| xml_fehler())?.into_owned()),
                Event::CData(t) => X::Text(String::from_utf8_lossy(&t).into_owned()),
                Event::GeneralRef(r) => {
                    if r.is_char_ref() {
                        match r.resolve_char_ref().map_err(|_| xml_fehler())? {
                            Some(c) => X::Text(c.to_string()),
                            None => continue,
                        }
                    } else {
                        let name = std::str::from_utf8(&r).map_err(|_| xml_fehler())?;
                        X::Text(
                            match name {
                                "lt" => "<",
                                "gt" => ">",
                                "amp" => "&",
                                "apos" => "'",
                                "quot" => "\"",
                                _ => return Err(ExtraktFehler::neu("xml_entitaet")),
                            }
                            .to_string(),
                        )
                    }
                }
                Event::Eof => X::Eof,
                Event::Comment(_) | Event::Decl(_) | Event::PI(_) => continue,
            });
        }
    }
}

pub(crate) fn attr<'v>(attrs: &'v [(String, String)], name: &str) -> Option<&'v str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}
