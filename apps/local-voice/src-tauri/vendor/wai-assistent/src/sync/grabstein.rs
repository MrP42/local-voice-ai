//! Grabsteine (ADR-0028 E3.2): bekannte Loeschziele eines Geraets.
//!
//! Ein Grabstein schlaegt jedes Objekt seines Ziels, auch ein offline wieder auftauchendes; es gibt
//! keine Wiederbelebung. Grabsteine bleiben dauerhaft und werden mit dem Geraetezustand gesichert (E3.8).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::kennung::Hex128;
use super::umschlag::{Ziel, ZielTyp};

/// Menge der dem Geraet bekannten Loeschziele.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grabsteine {
    unterhaltungen: BTreeSet<Hex128>,
    objekte: BTreeSet<Hex128>,
}

impl Grabsteine {
    /// Merkt ein Ziel; `true`, wenn es neu war.
    pub fn merken(&mut self, ziel: &Ziel) -> bool {
        match ziel.typ {
            ZielTyp::Unterhaltung => self.unterhaltungen.insert(ziel.wert.clone()),
            ZielTyp::Objekt => self.objekte.insert(ziel.wert.clone()),
        }
    }

    /// Schlaegt ein Grabstein dieses Objekt (per Unterhaltung oder per Objekt-ID)?
    pub fn schlaegt(&self, unterhaltung_ref: &Hex128, objekt_id: &Hex128) -> bool {
        self.unterhaltungen.contains(unterhaltung_ref) || self.objekte.contains(objekt_id)
    }

    pub fn unterhaltung_geloescht(&self, unterhaltung_ref: &Hex128) -> bool {
        self.unterhaltungen.contains(unterhaltung_ref)
    }

    pub fn len(&self) -> usize {
        self.unterhaltungen.len() + self.objekte.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(n: u8) -> Hex128 {
        Hex128::parse(&format!("{n:02x}").repeat(16)).unwrap()
    }

    #[test]
    fn unterhaltungsgrabstein_schlaegt_alle_objekte_der_unterhaltung() {
        let mut g = Grabsteine::default();
        assert!(g.merken(&Ziel {
            typ: ZielTyp::Unterhaltung,
            wert: h(1)
        }));
        assert!(
            !g.merken(&Ziel {
                typ: ZielTyp::Unterhaltung,
                wert: h(1)
            }),
            "idempotent"
        );
        assert!(g.schlaegt(&h(1), &h(7)));
        assert!(!g.schlaegt(&h(2), &h(7)));
        assert!(g.unterhaltung_geloescht(&h(1)));
    }

    #[test]
    fn objektgrabstein_schlaegt_nur_das_objekt() {
        let mut g = Grabsteine::default();
        g.merken(&Ziel {
            typ: ZielTyp::Objekt,
            wert: h(7),
        });
        assert!(g.schlaegt(&h(1), &h(7)));
        assert!(!g.schlaegt(&h(1), &h(8)));
        assert!(!g.unterhaltung_geloescht(&h(1)));
        assert_eq!(g.len(), 1);
    }
}
