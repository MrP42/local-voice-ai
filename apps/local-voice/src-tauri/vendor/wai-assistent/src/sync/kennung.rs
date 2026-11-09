//! 128-Bit-Kennungen (32 Hexzeichen) und Zufall.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::fehler::SyncFehler;

/// Kennung aus 32 kleinen Hexzeichen (`Hex128` des Vertrags `assistent-sync-object`).
///
/// Zufaellig erzeugt oder per [`Hex128::abgeleitet`] stabil aus einer vorhandenen Kennung.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hex128(String);

impl Hex128 {
    /// Prueft Form (genau 32 Zeichen aus `0-9a-f`).
    pub fn parse(s: &str) -> Result<Self, SyncFehler> {
        let gueltig = s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if gueltig {
            Ok(Self(s.to_owned()))
        } else {
            Err(SyncFehler::UngueltigeKennung)
        }
    }

    /// Neue zufaellige Kennung (128 Bit).
    pub fn zufall() -> Result<Self, SyncFehler> {
        Ok(Self::aus_bytes(&zufall::<16>()?))
    }

    /// Stabile, schluesselabhaengige Kennung: `SHA-256(salz, zweck, eingabe)`, auf 128 Bit gekuerzt.
    ///
    /// Gleiche Eingabe und gleiches Salz ergeben auf jedem Geraet dieselbe Kennung (Migration, E11);
    /// ohne das Kontosalz ist kein Bezug zur Eingabe herstellbar.
    pub fn abgeleitet(salz: &[u8], zweck: &str, eingabe: &str) -> Self {
        let mut h = Sha256::new();
        h.update(b"wai-assistent-sync/id/v1");
        for teil in [salz, zweck.as_bytes(), eingabe.as_bytes()] {
            h.update((teil.len() as u32).to_be_bytes());
            h.update(teil);
        }
        Self::aus_bytes(&h.finalize()[..16])
    }

    fn aus_bytes(b: &[u8]) -> Self {
        Self(b.iter().map(|x| format!("{x:02x}")).collect())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Hex128 {
    type Error = SyncFehler;
    fn try_from(s: String) -> Result<Self, SyncFehler> {
        Self::parse(&s)
    }
}

impl From<Hex128> for String {
    fn from(h: Hex128) -> String {
        h.0
    }
}

impl fmt::Display for Hex128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Hex128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hex128({})", self.0)
    }
}

/// `N` Zufallsbytes aus dem Betriebssystem.
pub(crate) fn zufall<const N: usize>() -> Result<[u8; N], SyncFehler> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|_| SyncFehler::Zufall)?;
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prueft_form() {
        assert!(Hex128::parse("a1b2c3d4e5f60718293a4b5c6d7e8f90").is_ok());
        assert!(Hex128::parse("A1b2c3d4e5f60718293a4b5c6d7e8f90").is_err());
        assert!(Hex128::parse("abc").is_err());
        assert!(Hex128::parse("g1b2c3d4e5f60718293a4b5c6d7e8f90").is_err());
    }

    #[test]
    fn zufall_ist_gueltig_und_verschieden() {
        let (a, b) = (Hex128::zufall().unwrap(), Hex128::zufall().unwrap());
        assert_ne!(a, b);
        assert!(Hex128::parse(a.as_str()).is_ok());
    }

    #[test]
    fn abgeleitet_ist_stabil_und_getrennt() {
        let a = Hex128::abgeleitet(b"salz", "beitrag", "msg_1");
        assert_eq!(a, Hex128::abgeleitet(b"salz", "beitrag", "msg_1"));
        assert_ne!(a, Hex128::abgeleitet(b"salz", "beitrag", "msg_2"));
        assert_ne!(a, Hex128::abgeleitet(b"salz", "unterhaltung", "msg_1"));
        assert_ne!(a, Hex128::abgeleitet(b"anderes", "beitrag", "msg_1"));
        // Laengenpraefix verhindert Grenzverschiebung zwischen den Teilen.
        assert_ne!(
            Hex128::abgeleitet(b"s", "ab", "c"),
            Hex128::abgeleitet(b"s", "a", "bc")
        );
    }

    #[test]
    fn serde_prueft_beim_lesen() {
        let ok: Hex128 = serde_json::from_str("\"a1b2c3d4e5f60718293a4b5c6d7e8f90\"").unwrap();
        assert_eq!(ok.as_str().len(), 32);
        assert!(serde_json::from_str::<Hex128>("\"zz\"").is_err());
    }
}
