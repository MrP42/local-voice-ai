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
/// Mehr Abschnitte als das (leere Absätze fallen vorher weg) gelten als Missbrauch: `zu_gross`.
pub const MAX_ABSCHNITTE: usize = 100_000;
/// Stack des Extraktionsthreads im Unterprozess (PDF-Rekursion); zählt zur Speichergrenze des Kindes.
pub const STACK_KIND: usize = 64 * 1024 * 1024;
/// Kommandozeilenschalter, mit dem das App-Programm selbst als Extraktor läuft (wenn kein eigenes
/// `lva-extrakt` neben dem Programm liegt).
pub const SCHALTER: &str = "--lva-extrakt";
/// Größere Eingaben werden nicht gelesen (die Ablage legt größere Dateien zwar ab, extrahiert sie aber
/// nicht; die Speichergrenze des Unterprozesses liegt bei 1 GiB).
pub const MAX_EINGABE: u64 = 512 * 1024 * 1024;
/// Umgebung des Unterprozesses (vom Elternprozess gesetzt): zufälliges Kennzeichen, das zusätzlich über
/// stdin kommen muss, und der Ablageordner, unter dem die Eingabe liegen muss.
pub const ENV_KENNZEICHEN: &str = "LVA_EXTRAKT_KENNZEICHEN";
pub const ENV_WURZEL: &str = "LVA_EXTRAKT_WURZEL";

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
    let mut aus = Vec::with_capacity(roh.len().min(MAX_ABSCHNITTE));
    for mut a in roh {
        a.text = a.text.trim().to_string();
        if a.text.is_empty() {
            continue;
        }
        // Text, Anker und Überschriftenpfad zählen zur Ausgabegröße (Pfad wird je Abschnitt wiederholt).
        gesamt = gesamt.saturating_add(
            a.text.len() + a.anker.len() + a.pfad.iter().map(|p| p.len() + 4).sum::<usize>(),
        );
        if gesamt > MAX_TEXT || aus.len() >= MAX_ABSCHNITTE {
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
/// auf stdout; Rückgabe ist der Exitcode (0 = Textfassung, 2 = Fehlercode, 64 = Aufruf falsch oder nicht
/// vom Elternprozess, dann ohne jede Ausgabe). Nie Inhalt auf stderr.
///
/// Angenommen wird der Aufruf nur, wenn [`ENV_KENNZEICHEN`] gesetzt ist, dasselbe Kennzeichen als erste
/// Zeile auf stdin kommt (ein Start über Dateizuordnung, Verknüpfung oder Kommandozeile hat beides nicht)
/// und der kanonische Pfad unter dem kanonischen [`ENV_WURZEL`] liegt.
pub fn haupt(args: &[String]) -> i32 {
    let [pfad, sha, typ] = args else {
        return 64;
    };
    let pfad = std::path::Path::new(pfad);
    if !zugelassen(pfad, &mut std::io::stdin().lock()) {
        return 64;
    }
    // Eigener Thread mit großem Stack: tief verschachtelte PDFs rekursieren sonst über den Haupt-Stack.
    // Läuft der Thread nicht an oder gerät in Panik, endet der Prozess ohne Antwort (Abbruch).
    let (p, h, t) = (pfad.to_path_buf(), sha.clone(), typ.clone());
    let Ok(thread) = std::thread::Builder::new()
        .stack_size(STACK_KIND)
        .spawn(move || ausfuehren(&p, &h, &t))
    else {
        return 71;
    };
    let Ok(ergebnis) = thread.join() else {
        return 70;
    };
    let antwort = match ergebnis {
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

fn kennzeichen_gueltig(k: &str) -> bool {
    k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Kennzeichen aus Umgebung und stdin, Pfad unter der Wurzel (beide kanonisch; die Datei selbst prüft
/// [`datei_lesen`] ohne Folgen von Verknüpfungen).
pub(crate) fn zugelassen(pfad: &std::path::Path, stdin: &mut dyn std::io::Read) -> bool {
    use std::io::Read;
    let Ok(k) = std::env::var(ENV_KENNZEICHEN) else {
        return false;
    };
    if !kennzeichen_gueltig(&k) {
        return false;
    }
    let mut zeile = Vec::new();
    if std::io::Read::take(stdin, 66)
        .read_to_end(&mut zeile)
        .is_err()
    {
        return false;
    }
    let gelesen = String::from_utf8_lossy(&zeile);
    if gelesen.trim_end_matches(['\r', '\n']) != k {
        return false;
    }
    let Some(wurzel) = std::env::var_os(ENV_WURZEL) else {
        return false;
    };
    let Ok(wurzel) = std::fs::canonicalize(wurzel) else {
        return false;
    };
    let (Some(eltern), Some(name)) = (pfad.parent(), pfad.file_name()) else {
        return false;
    };
    let Ok(eltern) = std::fs::canonicalize(eltern) else {
        return false;
    };
    eltern.starts_with(&wurzel) && eltern.join(name).starts_with(&wurzel)
}

/// Öffnet die Eingabe ohne Folgen einer Verknüpfung (Unix `O_NOFOLLOW`, Windows
/// `FILE_FLAG_OPEN_REPARSE_POINT`), prüft am offenen Handle, dass es eine gewöhnliche Datei ist, und
/// liest höchstens `max` Bytes.
pub(crate) fn datei_lesen(pfad: &std::path::Path, max: u64) -> Result<Vec<u8>, ExtraktFehler> {
    use std::io::Read;
    let lesen = || ExtraktFehler::neu("lesen");
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: die Verknüpfung selbst öffnen, nicht ihr Ziel.
        o.custom_flags(0x0020_0000);
        // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE: Löschen und Verschieben bleiben
        // während der Extraktion möglich.
        o.share_mode(0x1 | 0x2 | 0x4);
    }
    let datei = o.open(pfad).map_err(|_| lesen())?;
    let meta = datei.metadata().map_err(|_| lesen())?;
    if super::ist_umweg(&meta) || !meta.is_file() {
        return Err(lesen());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT
        if meta.file_attributes() & 0x400 != 0 {
            return Err(lesen());
        }
    }
    // Hartlink: könnte eine Datei außerhalb der Ablage sein; endgültig abgelehnt.
    if super::datei_hartverknuepft(&datei) {
        return Err(ExtraktFehler::neu("hartlink"));
    }
    if meta.len() > max {
        return Err(ExtraktFehler::neu("zu_gross"));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    datei
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| lesen())?;
    if bytes.len() as u64 > max {
        return Err(ExtraktFehler::neu("zu_gross"));
    }
    Ok(bytes)
}

fn ausfuehren(pfad: &std::path::Path, sha: &str, typ: &str) -> Result<Textfassung, ExtraktFehler> {
    // D7: `ocr:<typ>` liest Bild-PDFs und Bilder mit der lokalen Texterkennung (nach derselben Prüfung).
    if let Some(innen) = typ.strip_prefix(super::ocr::PRAEFIX) {
        if !super::ocr::ocr_typ(innen) {
            return Err(ExtraktFehler::neu("format"));
        }
        let bytes = datei_lesen(pfad, MAX_EINGABE)?;
        if hex(&Sha256::digest(&bytes)) != sha.to_ascii_lowercase() {
            return Err(ExtraktFehler::neu("pruefsumme"));
        }
        drop(bytes);
        return super::ocr::textfassung(pfad, &sha.to_ascii_lowercase(), innen);
    }
    let f = format_aus_typ(typ).ok_or(ExtraktFehler::neu("format"))?;
    let bytes = datei_lesen(pfad, MAX_EINGABE)?;
    #[cfg(test)]
    if let Some(f) = tests_kind::haken(typ) {
        return Err(f);
    }
    // Erst die Prüfsumme, dann der Inhalt: Text und Original stammen aus demselben Stand (E9).
    if hex(&Sha256::digest(&bytes)) != sha.to_ascii_lowercase() {
        return Err(ExtraktFehler::neu("pruefsumme"));
    }
    textfassung(&bytes, &sha.to_ascii_lowercase(), f)
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

/// Testhaken des Kindprozesses: simuliert einen hängenden oder speicherhungrigen Extraktor.
#[cfg(test)]
pub(crate) mod tests_kind {
    pub fn haken(typ: &str) -> Option<super::ExtraktFehler> {
        match std::env::var("LVA_EXTRAKT_TEST").as_deref() {
            // Vorübergehender Fehler im Kind (Lesen scheitert).
            Ok("lesen") if typ == "txt" => return Some(super::ExtraktFehler::neu("lesen")),
            // Braucht rund 40 MiB Stack: läuft nur im Thread mit großem Stack durch.
            Ok("stack") if typ == "txt" => {
                fn tief(n: u32) -> u8 {
                    let block = [n as u8; 1024 * 1024];
                    let r = if n == 0 { 0 } else { tief(n - 1) };
                    std::hint::black_box(&block);
                    r.wrapping_add(block[17])
                }
                std::hint::black_box(tief(40));
            }
            // Arbeitsordner des Kindes darf nicht die Ablagewurzel sein (sie wäre unter Windows
            // gegen Umbenennen und Löschen gesperrt).
            Ok("arbeitsordner") if typ == "txt" => {
                let cwd = std::env::current_dir().and_then(std::fs::canonicalize);
                let wurzel = std::env::var_os(super::ENV_WURZEL).map(std::fs::canonicalize);
                if let (Ok(c), Some(Ok(w))) = (cwd, wurzel) {
                    if c.starts_with(&w) {
                        std::process::exit(3);
                    }
                }
            }
            Ok("endlos") if typ == "txt" => loop {
                std::hint::black_box(0u64);
            },
            Ok("langsam") if typ == "txt" => {
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
            Ok("flut") if typ == "txt" => {
                use std::io::Write;
                let block = vec![b'x'; 1024 * 1024];
                let mut out = std::io::stdout().lock();
                while out.write_all(&block).is_ok() {}
                std::process::exit(0);
            }
            Ok("speicher") if typ == "txt" => {
                // Optional: eigene PID ablegen, damit ein Test nach dem Lauf prüfen kann, dass das Kind
                // weder läuft noch als Zombie liegt (lva#462).
                if let Some(p) = std::env::var_os("LVA_EXTRAKT_PIDDATEI") {
                    let _ = std::fs::write(p, std::process::id().to_string());
                }
                let mut v: Vec<Vec<u8>> = Vec::new();
                // Sicherheitsnetz für die Testmaschine: versagt die Speichergrenze des Elternprozesses,
                // endet der Haken nach 4 GiB ohne Antwort (der Test sieht dann `extrakt.programm`
                // statt eines Abbruchs und wird rot), statt den Rechner leerzufressen.
                for _ in 0..64 {
                    v.push(vec![1u8; 64 * 1024 * 1024]);
                    std::hint::black_box(&v);
                }
                std::process::exit(0);
            }
            _ => {}
        }
        None
    }
}
