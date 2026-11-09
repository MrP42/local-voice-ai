//! ZIP-Container (DOCX, PPTX, XLSX, ODT, ODP, ODS) mit Grenzen öffnen (Spec §4.1, ADR-0029 E9).
//!
//! * höchstens [`MAX_EINTRAEGE`] Einträge,
//! * höchstens [`MAX_ENTPACKT`] entpackt (Summe der angegebenen Größen und tatsächlich gelesene Bytes),
//! * Verhältnis entpackt : gepackt höchstens [`MAX_VERHAELTNIS`] (für das ganze Archiv und je Eintrag ab
//!   1 MiB; kleine XML-Teile packen sich legitim besser).
//!
//! Die angegebenen Größen im Archiv sind nicht vertrauenswürdig: gelesen wird höchstens die angegebene
//! Größe + 1 Byte, ein Eintrag, der mehr liefert, wird abgewiesen.

use super::ExtraktFehler;
use std::io::{Cursor, Read};

pub const MAX_EINTRAEGE: usize = 10_000;
pub const MAX_ENTPACKT: u64 = 200 * 1024 * 1024;
pub const MAX_VERHAELTNIS: u64 = 100;
const VERHAELTNIS_AB: u64 = 1024 * 1024;

pub struct ZipLeser<'a> {
    archiv: zip::ZipArchive<Cursor<&'a [u8]>>,
    gelesen: u64,
}

fn grenze() -> ExtraktFehler {
    ExtraktFehler::neu("zip_grenze")
}

impl<'a> ZipLeser<'a> {
    pub fn oeffnen(bytes: &'a [u8]) -> Result<Self, ExtraktFehler> {
        let mut archiv =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| ExtraktFehler::neu("zip"))?;
        if archiv.len() > MAX_EINTRAEGE {
            return Err(grenze());
        }
        let (mut summe, mut gepackt) = (0u64, 0u64);
        for i in 0..archiv.len() {
            let e = archiv
                .by_index_raw(i)
                .map_err(|_| ExtraktFehler::neu("zip"))?;
            let (gross, klein) = (e.size(), e.compressed_size());
            summe = summe.saturating_add(gross);
            gepackt = gepackt.saturating_add(klein);
            if gross >= VERHAELTNIS_AB && gross / klein.max(1) > MAX_VERHAELTNIS {
                return Err(grenze());
            }
        }
        if summe > MAX_ENTPACKT
            || (summe >= VERHAELTNIS_AB && summe / gepackt.max(1) > MAX_VERHAELTNIS)
        {
            return Err(grenze());
        }
        Ok(Self { archiv, gelesen: 0 })
    }

    pub fn namen(&self) -> Vec<String> {
        self.archiv.file_names().map(str::to_string).collect()
    }

    /// Liest einen Eintrag vollständig (mit Grenzen); `None`, wenn er fehlt.
    pub fn lesen(&mut self, name: &str) -> Result<Option<Vec<u8>>, ExtraktFehler> {
        let mut e = match self.archiv.by_name(name) {
            Ok(e) => e,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(_) => return Err(ExtraktFehler::neu("zip")),
        };
        let angegeben = e.size();
        let rest = MAX_ENTPACKT.saturating_sub(self.gelesen);
        if angegeben > rest {
            return Err(grenze());
        }
        let mut aus = Vec::with_capacity(angegeben.min(16 * 1024 * 1024) as usize);
        (&mut e)
            .take(angegeben + 1)
            .read_to_end(&mut aus)
            .map_err(|_| ExtraktFehler::neu("zip"))?;
        if aus.len() as u64 > angegeben {
            return Err(grenze());
        }
        self.gelesen += aus.len() as u64;
        Ok(Some(aus))
    }
}
