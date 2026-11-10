//! Schluesselmaterial: ein 256-Bit-Schluessel je Epoche plus Kontosalz fuer abgeleitete Kennungen.
//!
//! Epochenschluessel sind unabhaengig zufaellig (nicht voneinander ableitbar): Ein widerrufenes Geraet
//! kennt nur die Epochen bis zu seinem Widerruf und kann spaetere nicht berechnen (E3.1). Die Verteilung
//! an Geraete laeuft als gewrappter Schluessel je Empfaenger-Geraeteschluessel (`paket`, E3.8).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::fehler::SyncFehler;
use super::kennung::{GeheimHex, zufall};

/// Vergleich zweier Geheimnisse in konstanter Zeit (`subtle`, E3.8 Must-fix 7).
pub(crate) fn gleich(a: &[u8; 32], b: &[u8; 32]) -> bool {
    use subtle::ConstantTimeEq;
    bool::from(a.ct_eq(b))
}

/// 256-Bit-Geheimnis; wird beim Freigeben ueberschrieben, `Debug` zeigt nichts.
#[derive(Clone)]
pub struct Schluessel([u8; 32]);

impl Schluessel {
    pub fn zufall() -> Result<Self, SyncFehler> {
        Ok(Self(zufall::<32>()?))
    }

    pub fn aus_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Rohbytes, nur fuer die Kopplung (QR) und Tests.
    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Drop for Schluessel {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Serialize for Schluessel {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        GeheimHex(Zeroizing::new(self.0)).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Schluessel {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Schluessel(*GeheimHex::deserialize(d)?.0))
    }
}

impl fmt::Debug for Schluessel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Schluessel(***)")
    }
}

/// Alle Epochenschluessel, die ein Geraet kennt, und das Kontosalz.
#[derive(Clone, Debug)]
pub struct Schluesselbund {
    id_salz: Schluessel,
    schluessel: BTreeMap<u64, Schluessel>,
    aktuell: u64,
}

impl Schluesselbund {
    /// Bund mit genau einer Epoche.
    pub fn neu(id_salz: Schluessel, epoche: u64, schluessel: Schluessel) -> Self {
        Self {
            id_salz,
            schluessel: BTreeMap::from([(epoche, schluessel)]),
            aktuell: epoche,
        }
    }

    /// Frischer Bund: zufaelliges Salz, Epoche 1 mit zufaelligem Schluessel.
    pub fn zufaellig() -> Result<Self, SyncFehler> {
        Ok(Self::neu(Schluessel::zufall()?, 1, Schluessel::zufall()?))
    }

    pub fn aktuelle_epoche(&self) -> u64 {
        self.aktuell
    }

    pub fn schluessel(&self, epoche: u64) -> Option<&Schluessel> {
        self.schluessel.get(&epoche)
    }

    pub fn id_salz(&self) -> &[u8] {
        self.id_salz.bytes()
    }

    pub(crate) fn salz(&self) -> &[u8; 32] {
        self.id_salz.bytes()
    }

    /// Bekannte Epochennummern, aufsteigend.
    pub fn epochen(&self) -> impl Iterator<Item = u64> + '_ {
        self.schluessel.keys().copied()
    }

    /// Nimmt einen verteilten Epochenschluessel auf; die hoechste Epoche wird zur aktuellen.
    ///
    /// Ist die Epochennummer schon mit einem anderen Schluessel belegt, kommt
    /// [`SyncFehler::EpocheKollision`] und nichts wird ueberschrieben. Derselbe Schluessel ist ein No-op.
    pub fn epoche_aufnehmen(
        &mut self,
        epoche: u64,
        schluessel: Schluessel,
    ) -> Result<(), SyncFehler> {
        if let Some(vorhanden) = self.schluessel.get(&epoche) {
            return if gleich(vorhanden.bytes(), schluessel.bytes()) {
                Ok(())
            } else {
                Err(SyncFehler::EpocheKollision(epoche))
            };
        }
        self.schluessel.insert(epoche, schluessel);
        self.aktuell = self.aktuell.max(epoche);
        Ok(())
    }
}

/// Sicherungsform; geheime Felder als [`GeheimHex`] (auch bei abgebrochenem Lesen ueberschrieben).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundGesichert {
    salz: GeheimHex,
    schluessel: Vec<(u64, GeheimHex)>,
}

/// Sicherung des Bundes (Host-Speicher, E3.8): enthaelt Geheimnisse, der Host legt sie geschuetzt ab.
impl Serialize for Schluesselbund {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        BundGesichert {
            salz: GeheimHex(Zeroizing::new(*self.id_salz.bytes())),
            schluessel: self
                .schluessel
                .iter()
                .map(|(e, k)| (*e, GeheimHex(Zeroizing::new(*k.bytes()))))
                .collect(),
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Schluesselbund {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let g = BundGesichert::deserialize(d)?;
        let id_salz = Schluessel(*g.salz.0);
        let schluessel: BTreeMap<u64, Schluessel> = g
            .schluessel
            .iter()
            .map(|(e, k)| (*e, Schluessel(*k.0)))
            .collect();
        let aktuell = *schluessel
            .keys()
            .next_back()
            .ok_or_else(|| D::Error::custom("bund ohne epoche"))?;
        Ok(Self {
            id_salz,
            schluessel,
            aktuell,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sicherung_rundlauf() {
        let mut b = Schluesselbund::zufaellig().unwrap();
        b.epoche_aufnehmen(2, Schluessel::zufall().unwrap())
            .unwrap();
        let j = serde_json::to_string(&b).unwrap();
        let z: Schluesselbund = serde_json::from_str(&j).unwrap();
        assert_eq!(z.aktuelle_epoche(), 2);
        assert_eq!(
            z.schluessel(1).unwrap().bytes(),
            b.schluessel(1).unwrap().bytes()
        );
        assert_eq!(z.id_salz(), b.id_salz());
    }

    #[test]
    fn debug_zeigt_kein_geheimnis() {
        let b = Schluesselbund::neu(
            Schluessel::aus_bytes([7; 32]),
            1,
            Schluessel::aus_bytes([9; 32]),
        );
        let s = format!("{b:?}");
        assert!(s.contains("***"));
        assert!(!s.contains("[9, 9") && !s.contains("[7, 7"));
    }

    #[test]
    fn epoche_aufnehmen_hebt_aktuelle() {
        let mut b = Schluesselbund::zufaellig().unwrap();
        b.epoche_aufnehmen(4, Schluessel::zufall().unwrap())
            .unwrap();
        assert_eq!(b.aktuelle_epoche(), 4);
        b.epoche_aufnehmen(2, Schluessel::zufall().unwrap())
            .unwrap();
        assert_eq!(b.aktuelle_epoche(), 4);
    }

    #[test]
    fn kollision_ueberschreibt_nichts() {
        let mut b = Schluesselbund::zufaellig().unwrap();
        let alt = b.schluessel(1).unwrap().clone();
        assert_eq!(
            b.epoche_aufnehmen(1, Schluessel::aus_bytes([9; 32])),
            Err(SyncFehler::EpocheKollision(1))
        );
        assert_eq!(b.schluessel(1).unwrap().bytes(), alt.bytes());
        assert_eq!(
            b.epoche_aufnehmen(1, alt),
            Ok(()),
            "gleicher Schluessel ist idempotent"
        );
    }
}
