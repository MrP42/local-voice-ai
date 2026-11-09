//! Schluesselmaterial: ein 256-Bit-Schluessel je Epoche plus Kontosalz fuer abgeleitete Kennungen.
//!
//! Epochenschluessel sind unabhaengig zufaellig (nicht voneinander ableitbar): Ein widerrufenes Geraet
//! kennt nur die Epochen bis zu seinem Widerruf und kann spaetere nicht berechnen (E3.1). Die Verteilung
//! an Geraete laeuft ausserhalb dieses Moduls ueber die Kopplung (QR).

use std::collections::BTreeMap;
use std::fmt;

use zeroize::Zeroize;

use super::fehler::SyncFehler;
use super::kennung::zufall;

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

    /// Startet eine neue Epoche mit frischem Zufallsschluessel (Gerätewiderruf, E3.1).
    pub fn neue_epoche(&mut self) -> Result<u64, SyncFehler> {
        let neu = self
            .aktuell
            .checked_add(1)
            .ok_or(SyncFehler::Vertrag("epoche ueberlaeuft"))?;
        self.schluessel.insert(neu, Schluessel::zufall()?);
        self.aktuell = neu;
        Ok(neu)
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
            return if vorhanden.bytes() == schluessel.bytes() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neue_epoche_ist_unabhaengig() {
        let mut b = Schluesselbund::zufaellig().unwrap();
        let alt = b.schluessel(1).unwrap().bytes().to_owned();
        assert_eq!(b.neue_epoche().unwrap(), 2);
        assert_eq!(b.aktuelle_epoche(), 2);
        assert_ne!(b.schluessel(2).unwrap().bytes(), &alt);
        assert!(b.schluessel(1).is_some(), "alte Epochen bleiben lesbar");
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
