//! Kettenpruefung veraenderlicher Objekte (Kopf, Lease) gegen den lokalen Hoechststand (ADR-0028 E3.8,
//! Must-fix 1 der zweiten Runde).
//!
//! Jede Fassung traegt ein signiertes [`Glied`] `(version, vorgaenger)` und hat einen Inhaltshash. Regeln:
//! - **Genesis:** Version 0 gibt es nicht; Version 1 hat den Null-Vorgaenger.
//! - **Abstammung:** Eine Fassung wird nur uebernommen, wenn sie lueckenlos vom lokalen Hoechststand abstammt
//!   (direkter Nachfolger oder eine Kette gepruefter Zwischenfassungen) oder, ohne Hoechststand, lueckenlos
//!   von der Genesis. Ein Sprung ohne Zwischenbelege wird zurueckgehalten ([`Einordnung::Luecke`]), nie
//!   geglaubt.
//! - **Gabelung:** Zwei gepruefte Ketten, die sich widersprechen, sind eine Gabelung. Gewinner ist
//!   reihenfolgeunabhaengig die hoechste Version, bei Gleichstand der kleinere Hash; die Gabelung selbst
//!   haelt der Aufrufer dauerhaft fest.
//! - **Rollback:** Eine kleinere Version als der Hoechststand wird nie uebernommen. Sie wird aber gegen die
//!   belegte Kette des Hoechststands geprueft: Weicht sie an ihrer Stelle ab und ist sie selbst lueckenlos
//!   belegt, ist sie eine Gabelung (die der Hoechststand gewinnt), kein stiller Rollback.
//!
//! Zwischenbelege liefert der Hub ueber [`super::hub::HubSpeicher::fassungen_lesen`]; jeder Beleg ist vorher
//! wie jedes Objekt geoeffnet und signaturgeprueft. Fehlen sie, bleibt es bei `Luecke`.

use std::collections::BTreeMap;

use super::abgleich::Hoechststand;
use super::umschlag::Glied;

/// Gepruefte Fassung: signiertes Glied und Inhaltshash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Beleg {
    pub glied: Glied,
    pub hash: [u8; 32],
}

/// Wie eine Fassung zum lokalen Hoechststand steht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Einordnung {
    /// Dieselbe Fassung wie der Hoechststand.
    Gleich,
    /// Stammt lueckenlos vom Hoechststand (bzw. der Genesis) ab: uebernehmen.
    Nachfolger,
    /// Kleinere Version als der Hoechststand.
    Rollback,
    /// Gepruefte, aber widerspruechliche Kette. `gewinnt`: hoehere Version bzw. kleinerer Hash.
    Gabelung { gewinnt: bool },
    /// Abstammung (noch) nicht belegt: zurueckhalten.
    Luecke,
    /// Verletzt die Genesisregel.
    Ungueltig,
}

const NULL: [u8; 32] = [0; 32];

fn genesis_ok(b: &Beleg) -> bool {
    match b.glied.version {
        0 => false,
        1 => b.glied.vorgaenger == NULL,
        _ => true,
    }
}

/// Ergebnis des Rueckwaertslaufs von einer Fassung.
enum Lauf {
    /// Erreicht den Hoechststand (gleiche Version, gleicher Hash).
    Hoechststand,
    /// Erreicht die Genesis, ohne den Hoechststand zu treffen (bei vorhandenem Hoechststand: Gabelung).
    Genesis,
    /// Ein Zwischenglied fehlt.
    Luecke,
    Ungueltig,
}

fn rueckwaerts(h: Option<Hoechststand>, ziel: &Beleg, belege: &[Beleg]) -> Lauf {
    let nach_hash: BTreeMap<[u8; 32], &Beleg> = belege
        .iter()
        .filter(|b| genesis_ok(b))
        .map(|b| (b.hash, b))
        .collect();
    let mut aktuell = *ziel;
    loop {
        if !genesis_ok(&aktuell) {
            return Lauf::Ungueltig;
        }
        if let Some(h) = h
            && h.version.checked_add(1) == Some(aktuell.glied.version)
            && aktuell.glied.vorgaenger == h.hash
        {
            return Lauf::Hoechststand;
        }
        if let Some(h) = h
            && aktuell.glied.version == h.version
        {
            if aktuell.hash == h.hash {
                return Lauf::Hoechststand;
            }
            if aktuell.glied.version == 1 {
                return Lauf::Genesis;
            }
        }
        if aktuell.glied.version == 1 {
            return Lauf::Genesis;
        }
        let Some(vor) = nach_hash.get(&aktuell.glied.vorgaenger) else {
            return Lauf::Luecke;
        };
        if vor.glied.version.checked_add(1) != Some(aktuell.glied.version) {
            return Lauf::Ungueltig;
        }
        aktuell = **vor;
    }
}

/// Vorfahr des Hoechststands mit Version `version`, gelaufen ueber die Belege (`None`, wenn ein Glied fehlt
/// oder die Kette bricht).
fn vorfahr(hs: Hoechststand, version: u64, belege: &[Beleg]) -> Option<Beleg> {
    let nach_hash: BTreeMap<[u8; 32], &Beleg> = belege
        .iter()
        .filter(|b| genesis_ok(b))
        .map(|b| (b.hash, b))
        .collect();
    let mut aktuell = **nach_hash.get(&hs.hash)?;
    if aktuell.glied.version != hs.version {
        return None;
    }
    while aktuell.glied.version > version {
        let vor = **nach_hash.get(&aktuell.glied.vorgaenger)?;
        if vor.glied.version.checked_add(1) != Some(aktuell.glied.version) {
            return None;
        }
        aktuell = vor;
    }
    Some(aktuell)
}

/// Aeltere Fassung als der Hoechststand (Runde 3, Must-fix 1): nie uebernehmen, aber auf Widerspruch
/// pruefen. Liegt an ihrer Stelle in der belegten Kette des Hoechststands eine andere Fassung und stammt sie
/// selbst lueckenlos von der Genesis ab, ist das eine Gabelung, die der Hoechststand (hoehere Version)
/// gewinnt. So verdeckt eine zuerst gelieferte hoehere Gegenkette die Gabelung nicht. Ohne Belege bleibt es
/// beim Rollback.
fn aelter_einordnen(hs: Hoechststand, ziel: &Beleg, belege: &[Beleg]) -> Einordnung {
    let Some(dort) = vorfahr(hs, ziel.glied.version, belege) else {
        return Einordnung::Rollback;
    };
    if dort.hash == ziel.hash {
        return Einordnung::Rollback;
    }
    match rueckwaerts(None, ziel, belege) {
        Lauf::Genesis | Lauf::Hoechststand => Einordnung::Gabelung { gewinnt: false },
        Lauf::Luecke => Einordnung::Rollback,
        Lauf::Ungueltig => Einordnung::Ungueltig,
    }
}

/// Ordnet `ziel` gegen den Hoechststand `h` ein. `belege` sind gepruefte Zwischenfassungen (darf leer sein:
/// dann ergibt jeder Sprung `Luecke`, und der Aufrufer holt die Belege nach).
pub(super) fn einordnen(h: Option<Hoechststand>, ziel: &Beleg, belege: &[Beleg]) -> Einordnung {
    if !genesis_ok(ziel) {
        return Einordnung::Ungueltig;
    }
    let Some(hs) = h else {
        return match rueckwaerts(None, ziel, belege) {
            Lauf::Genesis | Lauf::Hoechststand => Einordnung::Nachfolger,
            Lauf::Luecke => Einordnung::Luecke,
            Lauf::Ungueltig => Einordnung::Ungueltig,
        };
    };
    let v = ziel.glied.version;
    if v < hs.version {
        return aelter_einordnen(hs, ziel, belege);
    }
    if v == hs.version && ziel.hash == hs.hash {
        return Einordnung::Gleich;
    }
    if hs.version.checked_add(1) == Some(v) && ziel.glied.vorgaenger == hs.hash {
        return Einordnung::Nachfolger;
    }
    match rueckwaerts(Some(hs), ziel, belege) {
        Lauf::Hoechststand => Einordnung::Nachfolger,
        Lauf::Genesis => Einordnung::Gabelung {
            gewinnt: (v, std::cmp::Reverse(ziel.hash)) > (hs.version, std::cmp::Reverse(hs.hash)),
        },
        Lauf::Luecke => Einordnung::Luecke,
        Lauf::Ungueltig => Einordnung::Ungueltig,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: u64, vor: [u8; 32], h: u8) -> Beleg {
        Beleg {
            glied: Glied {
                version: v,
                vorgaenger: vor,
            },
            hash: [h; 32],
        }
    }

    fn hs(b: &Beleg) -> Hoechststand {
        Hoechststand {
            version: b.glied.version,
            hash: b.hash,
            revision: None,
        }
    }

    #[test]
    fn genesis_abstammung_luecke_und_gabelung() {
        let g1 = b(1, NULL, 1);
        let g2 = b(2, [1; 32], 2);
        let g3 = b(3, [2; 32], 3);
        assert_eq!(einordnen(None, &g1, &[]), Einordnung::Nachfolger);
        assert_eq!(
            einordnen(None, &b(1, [9; 32], 1), &[]),
            Einordnung::Ungueltig
        );
        assert_eq!(einordnen(None, &b(0, NULL, 1), &[]), Einordnung::Ungueltig);
        // Sprung ohne Belege: zurueckhalten, mit Belegen: uebernehmen.
        assert_eq!(einordnen(None, &g3, &[]), Einordnung::Luecke);
        assert_eq!(einordnen(None, &g3, &[g1, g2]), Einordnung::Nachfolger);
        assert_eq!(einordnen(Some(hs(&g1)), &g3, &[]), Einordnung::Luecke);
        assert_eq!(einordnen(Some(hs(&g1)), &g3, &[g2]), Einordnung::Nachfolger);
        assert_eq!(einordnen(Some(hs(&g1)), &g2, &[]), Einordnung::Nachfolger);
        assert_eq!(einordnen(Some(hs(&g3)), &g2, &[]), Einordnung::Rollback);
        assert_eq!(einordnen(Some(hs(&g2)), &g2, &[]), Einordnung::Gleich);
        // Gabelung: andere Version 2 mit gepruefter Kette bis zur Genesis.
        let x2 = b(2, [1; 32], 0);
        let x3 = b(3, [0; 32], 7);
        assert_eq!(
            einordnen(Some(hs(&g2)), &x2, &[g1]),
            Einordnung::Gabelung { gewinnt: true }
        );
        assert_eq!(
            einordnen(Some(hs(&x2)), &g2, &[g1]),
            Einordnung::Gabelung { gewinnt: false }
        );
        assert_eq!(
            einordnen(Some(hs(&g2)), &x3, &[g1, x2]),
            Einordnung::Gabelung { gewinnt: true }
        );
        // Unbelegter Gabelungsast: nur Luecke, kein Gewinner.
        assert_eq!(einordnen(Some(hs(&g2)), &x3, &[]), Einordnung::Luecke);
        // Aeltere Fassung: echte Vorfassung ist Rollback, eine abweichende, belegte ist Gabelung (Runde 3).
        assert_eq!(
            einordnen(Some(hs(&g3)), &g2, &[g1, g2, g3]),
            Einordnung::Rollback
        );
        assert_eq!(
            einordnen(Some(hs(&g3)), &x2, &[g1, g2, g3]),
            Einordnung::Gabelung { gewinnt: false }
        );
        // Ohne Belege der eigenen Kette oder der Gegenfassung: nur Rollback, keine Behauptung.
        assert_eq!(einordnen(Some(hs(&g3)), &x2, &[]), Einordnung::Rollback);
        assert_eq!(
            einordnen(Some(hs(&g3)), &x2, &[g2, g3]),
            Einordnung::Rollback
        );
        // Ein Beleg mit falscher Version bricht die Kette.
        assert_eq!(
            einordnen(None, &g3, &[g1, b(5, [1; 32], 2)]),
            Einordnung::Ungueltig
        );
    }
}
