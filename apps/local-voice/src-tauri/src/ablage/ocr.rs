//! D7 (Spec §8): lokale Texterkennung für Bild-PDFs und Bilder der Ablage.
//!
//! * Schnittstelle [`OcrLeser`]: ein Bild oder die Seiten eines PDFs lesen. Läuft nur im begrenzten
//!   Unterprozess `lva-extrakt` (Aufruf mit `typ = ocr:<typ>`, siehe [`super::extrakt::haupt`]); die Eingabe
//!   ist dieselbe geprüfte Datei wie bei der Textfassung (Prüfsumme vor dem Lesen).
//! * Windows: `Windows.Media.Ocr` über die schon vorhandene Folien-OCR
//!   (`managers::meetings::slides::ocr::WindowsOcr`, Sprachwahl de → en → Benutzersprachen). Bilder über
//!   `BitmapDecoder` (zu große Bilder werden auf die Grenze der Engine verkleinert), Bild-PDFs über
//!   `Windows.Data.Pdf` (jede Seite in doppelter Größe gerendert, höchstens [`MAX_SEITEN`] Seiten).
//! * macOS: Apple Vision (`VNRecognizeTextRequest`) bräuchte `objc2-vision` als neue Abhängigkeit; bis dahin
//!   meldet die Plattform „nicht verfügbar“ ([`extraktor`] = `None`), Bild-PDFs bleiben `kein_text`.
//! * Kennung und Version des Extraktors gehen wie bei jedem Extraktor in die `textrev` ein; der Elternprozess
//!   kennt sie vorab über [`extraktor`].
//!
//! Nie Inhalt im Log; Fehler tragen nur Codes (`ocr_nicht_verfuegbar`, `ocr_bild`, `ocr`, `pdf`).

use super::extrakt::{self, Abschnitt, ExtraktFehler, Textfassung};
use std::path::Path;

/// Präfix des Typs, mit dem der Elternprozess die OCR im Unterprozess anfordert (`ocr:pdf`, `ocr:png`).
pub const PRAEFIX: &str = "ocr:";
/// Mehr Seiten eines Bild-PDFs werden nicht gelesen (Zeit- und Speichergrenze des Unterprozesses).
pub const MAX_SEITEN: u32 = 200;
/// Bildtypen, die die OCR liest (Endung der Datei, klein).
pub const BILDTYPEN: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "gif"];

/// Kennung und Version eines Extraktors (`windows-ocr`, `1`).
pub type Kennung = (&'static str, &'static str);

pub fn ist_bild(typ: &str) -> bool {
    BILDTYPEN.contains(&typ.to_ascii_lowercase().as_str())
}

/// Kommt `typ` (Endung) für die OCR in Frage: Bilder sofort, PDFs, wenn sie keinen Text enthalten.
pub fn ocr_typ(typ: &str) -> bool {
    ist_bild(typ) || typ.eq_ignore_ascii_case("pdf")
}

/// Ein Texterkenner. Liest Dateien, die schon geprüft sind (Prüfsumme, keine Verknüpfung).
pub trait OcrLeser {
    fn kennung(&self) -> Kennung;
    /// Text eines Bildes (leer, wenn das Bild keinen Text zeigt).
    fn bild(&self, pfad: &Path) -> Result<String, ExtraktFehler>;
    /// Text je Seite eines PDFs (höchstens `max_seiten`).
    fn pdf(&self, pfad: &Path, max_seiten: u32) -> Result<Vec<String>, ExtraktFehler>;
}

/// Extraktor der Plattform, wenn sie eine OCR hat (Windows); sonst `None` (macOS, Linux).
pub fn extraktor() -> Option<Kennung> {
    #[cfg(windows)]
    {
        Some(windows_impl::KENNUNG)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Der Leser dieser Plattform (nur im Unterprozess). Ohne OCR-Sprache oder auf anderen Plattformen
/// `ocr_nicht_verfuegbar`.
fn leser() -> Result<Box<dyn OcrLeser>, ExtraktFehler> {
    #[cfg(test)]
    if std::env::var("LVA_EXTRAKT_TEST").as_deref() == Ok("ocr") {
        return Ok(Box::new(TestLeser));
    }
    #[cfg(windows)]
    {
        windows_impl::WindowsLeser::neu().map(|l| Box::new(l) as Box<dyn OcrLeser>)
    }
    #[cfg(not(windows))]
    {
        Err(ExtraktFehler::neu("ocr_nicht_verfuegbar"))
    }
}

/// Textfassung per OCR (im Unterprozess, nach der Prüfsumme). `typ` ohne Präfix (`pdf`, `png`, …).
pub fn textfassung(pfad: &Path, sha: &str, typ: &str) -> Result<Textfassung, ExtraktFehler> {
    let l = leser()?;
    textfassung_mit(l.as_ref(), pfad, sha, typ)
}

pub fn textfassung_mit(
    l: &dyn OcrLeser,
    pfad: &Path,
    sha: &str,
    typ: &str,
) -> Result<Textfassung, ExtraktFehler> {
    let roh: Vec<String> = if typ.eq_ignore_ascii_case("pdf") {
        l.pdf(pfad, MAX_SEITEN)?
    } else if ist_bild(typ) {
        vec![l.bild(pfad)?]
    } else {
        return Err(ExtraktFehler::neu("format"));
    };
    let (id, version) = l.kennung();
    let mut abschnitte = Vec::new();
    let mut gesamt = 0usize;
    for (i, text) in roh.into_iter().enumerate() {
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        gesamt = gesamt.saturating_add(text.len());
        if gesamt > extrakt::MAX_TEXT || abschnitte.len() >= extrakt::MAX_ABSCHNITTE {
            return Err(ExtraktFehler::neu("zu_gross"));
        }
        abschnitte.push(Abschnitt {
            anker: format!("seite={}", i + 1),
            pfad: vec![],
            text,
        });
    }
    Ok(Textfassung {
        marken_version: extrakt::MARKEN_VERSION,
        textrev: extrakt::textrev(sha, id, version, extrakt::MARKEN_VERSION),
        original_sha256: sha.to_string(),
        extraktor: id.to_string(),
        extraktor_version: version.to_string(),
        abschnitte,
    })
}

/// Erwartete `textrev` einer OCR-Textfassung.
pub fn textrev(sha: &str, k: Kennung) -> String {
    extrakt::textrev(sha, k.0, k.1, extrakt::MARKEN_VERSION)
}

/// Test: Kennung des Test-Lesers (für den Elternprozess).
#[cfg(test)]
pub const TEST_KENNUNG: Kennung = ("test-ocr", "1");

/// Test-Leser: „erkennt“ den Text hinter `OCRTEXT:` bis `)` (Seiten durch `|` getrennt). Nur in Tests, über
/// `LVA_EXTRAKT_TEST=ocr` im Unterprozess.
#[cfg(test)]
pub(crate) struct TestLeser;

#[cfg(test)]
impl OcrLeser for TestLeser {
    fn kennung(&self) -> Kennung {
        TEST_KENNUNG
    }

    fn bild(&self, pfad: &Path) -> Result<String, ExtraktFehler> {
        Ok(self.pdf(pfad, 1)?.join(" "))
    }

    fn pdf(&self, pfad: &Path, max_seiten: u32) -> Result<Vec<String>, ExtraktFehler> {
        let b = std::fs::read(pfad).map_err(|_| ExtraktFehler::neu("lesen"))?;
        let t = String::from_utf8_lossy(&b);
        let Some((_, rest)) = t.split_once("OCRTEXT:") else {
            return Ok(vec![]);
        };
        Ok(rest
            .split(')')
            .next()
            .unwrap_or("")
            .split('|')
            .take(max_seiten as usize)
            .map(str::to_string)
            .collect())
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::{ExtraktFehler, Kennung, OcrLeser};
    use crate::managers::meetings::slides::ocr::{WindowsOcr, PREFERRED_LANGUAGES};
    use std::path::Path;
    use windows::core::HSTRING;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat,
        BitmapTransform, ColorManagementMode, ExifOrientationMode, SoftwareBitmap,
    };
    use windows::Storage::Streams::{IRandomAccessStream, InMemoryRandomAccessStream};
    use windows::Storage::{FileAccessMode, StorageFile};

    pub const KENNUNG: Kennung = ("windows-ocr", "1");

    fn bild_fehler(_: windows::core::Error) -> ExtraktFehler {
        ExtraktFehler::neu("ocr_bild")
    }

    fn pdf_fehler(_: windows::core::Error) -> ExtraktFehler {
        ExtraktFehler::neu("pdf")
    }

    pub struct WindowsLeser {
        ocr: WindowsOcr,
        grenze: u32,
    }

    impl WindowsLeser {
        pub fn neu() -> Result<Self, ExtraktFehler> {
            let ocr = WindowsOcr::new(PREFERRED_LANGUAGES)
                .map_err(|_| ExtraktFehler::neu("ocr_nicht_verfuegbar"))?;
            let grenze = WindowsOcr::max_dimension()
                .map_err(|_| ExtraktFehler::neu("ocr_nicht_verfuegbar"))?
                .max(256);
            Ok(Self { ocr, grenze })
        }

        fn datei(pfad: &Path) -> Result<StorageFile, ExtraktFehler> {
            // `GetFileFromPathAsync` verlangt einen absoluten Pfad ohne `\\?\`.
            let abs = std::path::absolute(pfad).map_err(|_| ExtraktFehler::neu("lesen"))?;
            StorageFile::GetFileFromPathAsync(&HSTRING::from(abs.as_os_str()))
                .and_then(|op| op.get())
                .map_err(|_| ExtraktFehler::neu("lesen"))
        }

        /// Dekodiert ein Bild; ist es größer als die Grenze der Engine, wird es verkleinert.
        fn bitmap(&self, strom: &IRandomAccessStream) -> Result<SoftwareBitmap, ExtraktFehler> {
            let decoder = BitmapDecoder::CreateAsync(strom)
                .and_then(|op| op.get())
                .map_err(bild_fehler)?;
            let w = decoder.PixelWidth().map_err(bild_fehler)?;
            let h = decoder.PixelHeight().map_err(bild_fehler)?;
            if w == 0 || h == 0 {
                return Err(ExtraktFehler::neu("ocr_bild"));
            }
            if w <= self.grenze && h <= self.grenze {
                return decoder
                    .GetSoftwareBitmapAsync()
                    .and_then(|op| op.get())
                    .map_err(bild_fehler);
            }
            let f = f64::from(self.grenze) / f64::from(w.max(h));
            let t = BitmapTransform::new().map_err(bild_fehler)?;
            t.SetScaledWidth(((f64::from(w) * f) as u32).clamp(1, self.grenze))
                .map_err(bild_fehler)?;
            t.SetScaledHeight(((f64::from(h) * f) as u32).clamp(1, self.grenze))
                .map_err(bild_fehler)?;
            t.SetInterpolationMode(BitmapInterpolationMode::Fant)
                .map_err(bild_fehler)?;
            decoder
                .GetSoftwareBitmapTransformedAsync(
                    BitmapPixelFormat::Bgra8,
                    BitmapAlphaMode::Premultiplied,
                    &t,
                    ExifOrientationMode::RespectExifOrientation,
                    ColorManagementMode::DoNotColorManage,
                )
                .and_then(|op| op.get())
                .map_err(bild_fehler)
        }

        fn lesen(&self, b: &SoftwareBitmap) -> Result<String, ExtraktFehler> {
            self.ocr
                .recognize_bitmap(b)
                .map(|t| t.text)
                .map_err(|_| ExtraktFehler::neu("ocr"))
        }
    }

    impl OcrLeser for WindowsLeser {
        fn kennung(&self) -> Kennung {
            KENNUNG
        }

        fn bild(&self, pfad: &Path) -> Result<String, ExtraktFehler> {
            let datei = Self::datei(pfad)?;
            let strom: IRandomAccessStream = datei
                .OpenAsync(FileAccessMode::Read)
                .and_then(|op| op.get())
                .map_err(bild_fehler)?;
            let b = self.bitmap(&strom)?;
            self.lesen(&b)
        }

        fn pdf(&self, pfad: &Path, max_seiten: u32) -> Result<Vec<String>, ExtraktFehler> {
            let datei = Self::datei(pfad)?;
            let doc = PdfDocument::LoadFromFileAsync(&datei)
                .and_then(|op| op.get())
                .map_err(pdf_fehler)?;
            let seiten = doc.PageCount().map_err(pdf_fehler)?.min(max_seiten);
            let mut aus = Vec::with_capacity(seiten as usize);
            for i in 0..seiten {
                let seite = doc.GetPage(i).map_err(pdf_fehler)?;
                let groesse = seite.Size().map_err(pdf_fehler)?;
                // Doppelte Größe (rund 192 dpi) für die Erkennung, höchstens die Grenze der Engine.
                let (w, h) = (
                    f64::from(groesse.Width.max(1.0)) * 2.0,
                    f64::from(groesse.Height.max(1.0)) * 2.0,
                );
                let f = (f64::from(self.grenze) / w.max(h)).min(1.0);
                let optionen = PdfPageRenderOptions::new().map_err(pdf_fehler)?;
                optionen
                    .SetDestinationWidth(((w * f) as u32).max(1))
                    .map_err(pdf_fehler)?;
                optionen
                    .SetDestinationHeight(((h * f) as u32).max(1))
                    .map_err(pdf_fehler)?;
                let speicher = InMemoryRandomAccessStream::new().map_err(pdf_fehler)?;
                seite
                    .RenderWithOptionsToStreamAsync(&speicher, &optionen)
                    .and_then(|op| op.get())
                    .map_err(pdf_fehler)?;
                speicher.Seek(0).map_err(pdf_fehler)?;
                let strom: IRandomAccessStream = speicher.into();
                let b = self.bitmap(&strom)?;
                aus.push(self.lesen(&b)?);
            }
            Ok(aus)
        }
    }
}
