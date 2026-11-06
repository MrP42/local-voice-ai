//! Rahmen: 12 Byte Kopf, danach die Nutzlast.
//!
//! ```text
//! 0      1      2..4       4..8          8..12
//! Magie  Art    reserviert Strom (u32 LE) Länge (u32 LE)
//! 0x57   1..=3  0          0 = Verbindung Nutzlast in Bytes
//! ```
//!
//! Art 1 = Steuer (JSON, UTF-8), 2 = Audio hinein (PCM16 LE, 16 kHz mono, App → Dienst),
//! 3 = Audio heraus (WAV-Häppchen, Dienst → App). Unbekannte Art, falsche Magie, gesetzte
//! reservierte Bytes, eine Länge über der Grenze der Art, Audio in die falsche Richtung oder
//! ungerade PCM16-Länge sind Protokollfehler: Die Verbindung wird beendet, statt zu raten.

use thiserror::Error;

use crate::grenzen;

const MAGIE: u8 = 0x57;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Art {
    Steuer = 1,
    AudioHinein = 2,
    AudioHeraus = 3,
}

impl Art {
    fn aus(b: u8) -> Option<Self> {
        match b {
            1 => Some(Self::Steuer),
            2 => Some(Self::AudioHinein),
            3 => Some(Self::AudioHeraus),
            _ => None,
        }
    }

    /// Größte erlaubte Nutzlast dieser Art.
    pub fn max(self) -> usize {
        match self {
            Self::Steuer => grenzen::STEUER_MAX,
            Self::AudioHinein | Self::AudioHeraus => grenzen::AUDIO_MAX,
        }
    }
}

/// Wer empfängt: Der Dienst nimmt nur `AudioHinein` an, die App nur `AudioHeraus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rolle {
    Dienst,
    App,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rahmen {
    pub art: Art,
    /// 0 = Verbindung (Handschlag, Status), sonst der Strom einer Operation.
    pub strom: u32,
    pub nutzlast: Vec<u8>,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RahmenFehler {
    #[error("falsche Magie")]
    Magie,
    #[error("unbekannte Rahmenart {0}")]
    Art(u8),
    #[error("reservierte Bytes gesetzt")]
    Reserviert,
    #[error("Nutzlast {laenge} > {max} Bytes")]
    ZuGross { laenge: usize, max: usize },
    #[error("Audio auf Strom 0")]
    AudioOhneStrom,
    #[error("Audio in falscher Richtung")]
    Richtung,
    #[error("PCM16 mit ungerader Länge")]
    UngeradePcm,
}

impl Rahmen {
    pub fn steuer(strom: u32, json: Vec<u8>) -> Self {
        Self {
            art: Art::Steuer,
            strom,
            nutzlast: json,
        }
    }

    /// Kodiert den Rahmen; prüft dieselben Grenzen wie der Decoder.
    pub fn kodieren(&self) -> Result<Vec<u8>, RahmenFehler> {
        pruefen(self.art, self.strom, self.nutzlast.len())?;
        let mut out = Vec::with_capacity(grenzen::KOPF + self.nutzlast.len());
        out.push(MAGIE);
        out.push(self.art as u8);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&self.strom.to_le_bytes());
        out.extend_from_slice(&(self.nutzlast.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.nutzlast);
        Ok(out)
    }
}

fn pruefen(art: Art, strom: u32, laenge: usize) -> Result<(), RahmenFehler> {
    if laenge > art.max() {
        return Err(RahmenFehler::ZuGross {
            laenge,
            max: art.max(),
        });
    }
    if art != Art::Steuer && strom == 0 {
        return Err(RahmenFehler::AudioOhneStrom);
    }
    if art == Art::AudioHinein && !laenge.is_multiple_of(2) {
        return Err(RahmenFehler::UngeradePcm);
    }
    Ok(())
}

/// Decoder für einen Bytestrom, der in beliebigen Stücken ankommt.
///
/// Der Puffer hält höchstens einen Rahmen (Kopf plus größte Nutzlast). `einspeisen` nimmt nur
/// so viel an, wie hineinpasst, und meldet die Zahl der übernommenen Bytes; der Aufrufer holt
/// mit [`Decoder::naechster`] Rahmen ab und speist den Rest nach. Ein Fehler ist endgültig:
/// Danach liefert jeder Aufruf denselben Fehler, die Verbindung ist zu schließen.
#[derive(Debug)]
pub struct Decoder {
    rolle: Rolle,
    puffer: Vec<u8>,
    fehler: Option<RahmenFehler>,
}

const PUFFER_MAX: usize = grenzen::KOPF + grenzen::STEUER_MAX + grenzen::AUDIO_MAX;

impl Decoder {
    pub fn new(rolle: Rolle) -> Self {
        Self {
            rolle,
            puffer: Vec::new(),
            fehler: None,
        }
    }

    fn scheitern(&mut self, f: RahmenFehler) -> RahmenFehler {
        self.fehler = Some(f.clone());
        self.puffer.clear();
        f
    }

    /// Übernimmt höchstens so viele Bytes, wie in den Puffer passen; Rückgabe: übernommen.
    pub fn einspeisen(&mut self, bytes: &[u8]) -> Result<usize, RahmenFehler> {
        if let Some(f) = &self.fehler {
            return Err(f.clone());
        }
        let n = bytes.len().min(PUFFER_MAX - self.puffer.len());
        self.puffer.extend_from_slice(&bytes[..n]);
        // Kopf früh prüfen, damit eine falsche Länge nicht erst nach dem Puffern auffällt.
        if self.puffer.len() >= grenzen::KOPF
            && let Err(f) = self.kopf()
        {
            return Err(self.scheitern(f));
        }
        Ok(n)
    }

    fn kopf(&self) -> Result<(Art, u32, usize), RahmenFehler> {
        let k = &self.puffer[..grenzen::KOPF];
        if k[0] != MAGIE {
            return Err(RahmenFehler::Magie);
        }
        let art = Art::aus(k[1]).ok_or(RahmenFehler::Art(k[1]))?;
        if k[2] != 0 || k[3] != 0 {
            return Err(RahmenFehler::Reserviert);
        }
        let falsch = matches!(
            (self.rolle, art),
            (Rolle::Dienst, Art::AudioHeraus) | (Rolle::App, Art::AudioHinein)
        );
        if falsch {
            return Err(RahmenFehler::Richtung);
        }
        let strom = u32::from_le_bytes([k[4], k[5], k[6], k[7]]);
        let laenge = u32::from_le_bytes([k[8], k[9], k[10], k[11]]) as usize;
        pruefen(art, strom, laenge)?;
        Ok((art, strom, laenge))
    }

    /// Nächster vollständiger Rahmen, falls vorhanden. Ein fehlerhafter Folgekopf wird erst beim
    /// nächsten Aufruf gemeldet, der fertige Rahmen geht nicht verloren.
    pub fn naechster(&mut self) -> Result<Option<Rahmen>, RahmenFehler> {
        if let Some(f) = &self.fehler {
            return Err(f.clone());
        }
        if self.puffer.len() < grenzen::KOPF {
            return Ok(None);
        }
        let (art, strom, laenge) = match self.kopf() {
            Ok(k) => k,
            Err(f) => return Err(self.scheitern(f)),
        };
        let ende = grenzen::KOPF + laenge;
        if self.puffer.len() < ende {
            return Ok(None);
        }
        let nutzlast = self.puffer[grenzen::KOPF..ende].to_vec();
        self.puffer.drain(..ende);
        Ok(Some(Rahmen {
            art,
            strom,
            nutzlast,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alles(d: &mut Decoder, mut bytes: &[u8]) -> Result<Vec<Rahmen>, RahmenFehler> {
        let mut raus = Vec::new();
        while !bytes.is_empty() {
            let n = d.einspeisen(bytes)?;
            bytes = &bytes[n..];
            while let Some(r) = d.naechster()? {
                raus.push(r);
            }
        }
        Ok(raus)
    }

    #[test]
    fn hin_und_zurueck_in_stuecken() {
        let a = Rahmen::steuer(0, br#"{"t":"status"}"#.to_vec());
        let b = Rahmen {
            art: Art::AudioHinein,
            strom: 7,
            nutzlast: vec![1, 2, 3, 4],
        };
        let mut bytes = a.kodieren().unwrap();
        bytes.extend(b.kodieren().unwrap());
        let mut d = Decoder::new(Rolle::Dienst);
        let mut raus = Vec::new();
        for stueck in bytes.chunks(5) {
            raus.extend(alles(&mut d, stueck).unwrap());
        }
        assert_eq!(raus, vec![a, b]);
    }

    #[test]
    fn grosser_lesevorgang_mit_vielen_rahmen() {
        let mut bytes = Vec::new();
        for i in 0..200u32 {
            bytes.extend(
                Rahmen {
                    art: Art::AudioHinein,
                    strom: 1,
                    nutzlast: vec![0; 640],
                }
                .kodieren()
                .unwrap(),
            );
            bytes.extend(
                Rahmen::steuer(0, format!("{i}").into_bytes())
                    .kodieren()
                    .unwrap(),
            );
        }
        let mut d = Decoder::new(Rolle::Dienst);
        assert_eq!(alles(&mut d, &bytes).unwrap().len(), 400);
    }

    #[test]
    fn grenzen_beim_kodieren() {
        let r = Rahmen::steuer(0, vec![b' '; grenzen::STEUER_MAX + 1]);
        assert!(matches!(r.kodieren(), Err(RahmenFehler::ZuGross { .. })));
        let r = Rahmen {
            art: Art::AudioHinein,
            strom: 0,
            nutzlast: vec![0; 2],
        };
        assert_eq!(r.kodieren(), Err(RahmenFehler::AudioOhneStrom));
        let r = Rahmen {
            art: Art::AudioHinein,
            strom: 1,
            nutzlast: vec![0; 3],
        };
        assert_eq!(r.kodieren(), Err(RahmenFehler::UngeradePcm));
    }

    #[test]
    fn falscher_kopf_ist_endgueltig() {
        let mut d = Decoder::new(Rolle::Dienst);
        assert_eq!(d.einspeisen(&[0x00; 12]), Err(RahmenFehler::Magie));
        assert_eq!(
            d.einspeisen(&Rahmen::steuer(0, vec![]).kodieren().unwrap()),
            Err(RahmenFehler::Magie)
        );
        assert_eq!(d.naechster(), Err(RahmenFehler::Magie));
        let mut d = Decoder::new(Rolle::Dienst);
        assert_eq!(
            d.einspeisen(&[MAGIE, 9, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]),
            Err(RahmenFehler::Art(9))
        );
        let mut d = Decoder::new(Rolle::Dienst);
        assert_eq!(
            d.einspeisen(&[MAGIE, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(RahmenFehler::Reserviert)
        );
    }

    #[test]
    fn richtung_wird_geprueft() {
        let heraus = Rahmen {
            art: Art::AudioHeraus,
            strom: 1,
            nutzlast: vec![1],
        }
        .kodieren()
        .unwrap();
        assert_eq!(
            Decoder::new(Rolle::Dienst).einspeisen(&heraus),
            Err(RahmenFehler::Richtung)
        );
        assert!(Decoder::new(Rolle::App).einspeisen(&heraus).is_ok());
        let hinein = Rahmen {
            art: Art::AudioHinein,
            strom: 1,
            nutzlast: vec![1, 2],
        }
        .kodieren()
        .unwrap();
        assert_eq!(
            Decoder::new(Rolle::App).einspeisen(&hinein),
            Err(RahmenFehler::Richtung)
        );
    }

    #[test]
    fn guter_rahmen_vor_schlechtem_kopf_wird_geliefert() {
        let mut bytes = Rahmen::steuer(0, b"ok".to_vec()).kodieren().unwrap();
        bytes.extend_from_slice(&[0x00; 12]);
        let mut d = Decoder::new(Rolle::Dienst);
        // Der schlechte Folgekopf fällt schon beim Einspeisen auf: Verbindung endet.
        assert_eq!(d.einspeisen(&bytes[..14]), Ok(14));
        assert_eq!(d.naechster().unwrap().unwrap().nutzlast, b"ok");
        assert_eq!(d.einspeisen(&bytes[14..]), Err(RahmenFehler::Magie));
    }

    #[test]
    fn zu_lange_laenge_faellt_vor_dem_puffern_auf() {
        let mut d = Decoder::new(Rolle::Dienst);
        let mut kopf = vec![MAGIE, 2, 0, 0, 1, 0, 0, 0];
        kopf.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            d.einspeisen(&kopf),
            Err(RahmenFehler::ZuGross { .. })
        ));
    }
}
