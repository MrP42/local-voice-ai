//! Herkunft der Inhalte eines Modellaufrufs (lva#604, Teilpaket 1).
//!
//! [`Quellen`] sagt, aus welchen Besprechungen Text in einem Aufruf steckt. Die Darstellung
//! ist **privat**: konstruiert wird nur über benannte Konstruktoren, damit niemand eine
//! Herkunft „von Hand“ als harmlos zusammenbauen kann.
//!
//! Algebra:
//! - [`Quellen::keine`]: der Aufruf enthält nachweislich keinen Besprechungsinhalt.
//! - [`Quellen::besprechung`] / [`Quellen::besprechungen`]: genau diese Besprechungen. Eine
//!   leere Menge ist `keine()`; eine leere (oder nur aus Leerraum bestehende) ID ist
//!   `unbestimmt()` — wer keine ID nennen kann, hat keine Herkunft belegt.
//! - [`Quellen::unbestimmt`]: Besprechungsbezug möglich, aber nicht belegt. Cloud-Ziele
//!   sperren das.
//! - [`Quellen::vereinigen`]: `Unbestimmt` dominiert jede Vereinigung; `keine` ist das
//!   neutrale Element; zwei Mengen ergeben die Vereinigungsmenge.
//!
//! Es gibt bewusst **kein `Default`**: ein „leerer“ Standardwert würde als `keine()` gelesen
//! und Besprechungspfade stillschweigend umetikettieren (Codex #604, Runde 1).

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

/// Herkunft der Inhalte eines Aufrufs. Siehe Moduldoku.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quellen(Darstellung);

/// Private Darstellung. Die Varianten sind von außen nicht konstruierbar.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Darstellung {
    Keine,
    /// Nie leer, keine leere ID.
    Besprechungen(BTreeSet<String>),
    Unbestimmt,
}

impl Quellen {
    /// Kein Besprechungsinhalt (belegt, nicht bloß „unbekannt“).
    pub fn keine() -> Self {
        Self(Darstellung::Keine)
    }

    /// Besprechungsbezug ist möglich, die Herkunft aber nicht belegt.
    pub fn unbestimmt() -> Self {
        Self(Darstellung::Unbestimmt)
    }

    /// Inhalte genau dieser einen Besprechung. Leere ID: [`Quellen::unbestimmt`].
    pub fn besprechung(id: &str) -> Self {
        Self::besprechungen([id])
    }

    /// Inhalte genau dieser Besprechungen (Duplikate zählen einmal). Leere Menge:
    /// [`Quellen::keine`]. Enthält die Menge eine leere ID: [`Quellen::unbestimmt`].
    pub fn besprechungen<I, S>(ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut menge = BTreeSet::new();
        for id in ids {
            let id = id.as_ref();
            if id.trim().is_empty() {
                return Self::unbestimmt();
            }
            menge.insert(id.to_owned());
        }
        if menge.is_empty() {
            return Self::keine();
        }
        Self(Darstellung::Besprechungen(menge))
    }

    /// Vereinigung zweier Herkünfte. `Unbestimmt` dominiert; `keine` ändert nichts.
    pub fn vereinigen(self, andere: Self) -> Self {
        match (self.0, andere.0) {
            (Darstellung::Unbestimmt, _) | (_, Darstellung::Unbestimmt) => Self::unbestimmt(),
            (Darstellung::Keine, rest) | (rest, Darstellung::Keine) => Self(rest),
            (Darstellung::Besprechungen(mut a), Darstellung::Besprechungen(b)) => {
                a.extend(b);
                Self(Darstellung::Besprechungen(a))
            }
        }
    }

    /// Ist belegt, dass kein Besprechungsinhalt im Aufruf steckt?
    pub fn ist_keine(&self) -> bool {
        matches!(self.0, Darstellung::Keine)
    }

    /// Ist die Herkunft unbelegt?
    pub fn ist_unbestimmt(&self) -> bool {
        matches!(self.0, Darstellung::Unbestimmt)
    }

    /// Die belegten Besprechungs-IDs in fester Reihenfolge (aufsteigend). Leer bei `keine()`
    /// und bei `unbestimmt()`; wer prüft, fragt vorher [`Quellen::ist_unbestimmt`].
    pub fn besprechungs_ids(&self) -> impl Iterator<Item = &str> {
        let menge = match &self.0 {
            Darstellung::Besprechungen(menge) => Some(menge),
            Darstellung::Keine | Darstellung::Unbestimmt => None,
        };
        menge.into_iter().flatten().map(String::as_str)
    }

    /// SHA-256 über die Herkunft: stabil, unabhängig von Einfüge-Reihenfolge und Duplikaten.
    /// Dient der Bindung einer [`Versandfreigabe`](super::pruefer) an genau diese Quellen.
    ///
    /// Eingang: ein Markerbyte je Art (`0` keine, `1` Besprechungen, `2` unbestimmt), bei
    /// Besprechungen die Anzahl (u64, big endian) und je ID (aufsteigend sortiert) Länge
    /// (u64, big endian) plus Bytes. Die Längenpräfixe verhindern, dass `["ab","c"]` und
    /// `["a","bc"]` zusammenfallen.
    pub fn fingerabdruck(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        match &self.0 {
            Darstellung::Keine => {
                h.update([0u8]);
                h.update(0u64.to_be_bytes());
            }
            Darstellung::Unbestimmt => {
                h.update([2u8]);
                h.update(0u64.to_be_bytes());
            }
            Darstellung::Besprechungen(menge) => {
                h.update([1u8]);
                h.update((menge.len() as u64).to_be_bytes());
                for id in menge {
                    h.update((id.len() as u64).to_be_bytes());
                    h.update(id.as_bytes());
                }
            }
        }
        h.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: [u8; 32]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn leere_menge_ist_keine() {
        let leer: [&str; 0] = [];
        assert_eq!(Quellen::besprechungen(leer), Quellen::keine());
        assert!(Quellen::besprechungen(Vec::<String>::new()).ist_keine());
    }

    #[test]
    fn leere_id_ist_unbestimmt() {
        assert_eq!(Quellen::besprechung(""), Quellen::unbestimmt());
        assert_eq!(Quellen::besprechung("   "), Quellen::unbestimmt());
        assert_eq!(Quellen::besprechung("\t\n"), Quellen::unbestimmt());
        // Eine einzige leere ID macht die ganze Menge unbestimmt, auch hinter gültigen.
        assert_eq!(
            Quellen::besprechungen(["a", "b", ""]),
            Quellen::unbestimmt()
        );
        assert_eq!(Quellen::besprechungen(["", "a"]), Quellen::unbestimmt());
    }

    #[test]
    fn einzelne_und_mehrere_besprechungen() {
        let q = Quellen::besprechungen(["b", "a", "b"]);
        assert!(!q.ist_keine() && !q.ist_unbestimmt());
        assert_eq!(q.besprechungs_ids().collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(Quellen::besprechung("a"), Quellen::besprechungen(["a"]));
    }

    #[test]
    fn unbestimmt_dominiert_jede_vereinigung() {
        let faelle = [
            Quellen::keine(),
            Quellen::besprechung("m1"),
            Quellen::besprechungen(["m1", "m2"]),
            Quellen::unbestimmt(),
        ];
        for q in &faelle {
            assert_eq!(
                Quellen::unbestimmt().vereinigen(q.clone()),
                Quellen::unbestimmt()
            );
            assert_eq!(
                q.clone().vereinigen(Quellen::unbestimmt()),
                Quellen::unbestimmt()
            );
        }
    }

    #[test]
    fn keine_ist_neutral_und_mengen_vereinigen_sich() {
        let m = Quellen::besprechung("m1");
        assert_eq!(Quellen::keine().vereinigen(m.clone()), m);
        assert_eq!(m.clone().vereinigen(Quellen::keine()), m);
        assert_eq!(
            Quellen::keine().vereinigen(Quellen::keine()),
            Quellen::keine()
        );
        assert_eq!(
            Quellen::besprechung("m1").vereinigen(Quellen::besprechung("m2")),
            Quellen::besprechungen(["m2", "m1"])
        );
        // Idempotent und kommutativ.
        assert_eq!(m.clone().vereinigen(m.clone()), m);
        assert_eq!(
            Quellen::besprechung("a").vereinigen(Quellen::besprechung("b")),
            Quellen::besprechung("b").vereinigen(Quellen::besprechung("a"))
        );
    }

    #[test]
    fn ids_von_keine_und_unbestimmt_sind_leer() {
        assert_eq!(Quellen::keine().besprechungs_ids().count(), 0);
        assert_eq!(Quellen::unbestimmt().besprechungs_ids().count(), 0);
    }

    #[test]
    fn fingerabdruck_ist_stabil_und_ordnungsunabhaengig() {
        let a = Quellen::besprechungen(["m3", "m1", "m2"]);
        let b = Quellen::besprechungen(["m1", "m2", "m3"]);
        let c = Quellen::besprechungen(["m2", "m2", "m3", "m1", "m1"]);
        assert_eq!(a.fingerabdruck(), b.fingerabdruck());
        assert_eq!(a.fingerabdruck(), c.fingerabdruck());
        // Über Vereinigungen in anderer Reihenfolge ebenso.
        let v = Quellen::besprechung("m2")
            .vereinigen(Quellen::besprechung("m3"))
            .vereinigen(Quellen::besprechung("m1"));
        assert_eq!(a.fingerabdruck(), v.fingerabdruck());
        // Wiederholte Berechnung liefert dasselbe.
        assert_eq!(a.fingerabdruck(), a.fingerabdruck());
    }

    #[test]
    fn fingerabdruck_unterscheidet_arten_und_inhalte() {
        let alle = [
            Quellen::keine(),
            Quellen::unbestimmt(),
            Quellen::besprechung("a"),
            Quellen::besprechung("b"),
            Quellen::besprechungen(["a", "b"]),
            Quellen::besprechungen(["ab"]),
            Quellen::besprechungen(["ab", "c"]),
            Quellen::besprechungen(["a", "bc"]),
        ];
        let mut hashes: Vec<[u8; 32]> = alle.iter().map(Quellen::fingerabdruck).collect();
        hashes.sort_unstable();
        hashes.dedup();
        assert_eq!(hashes.len(), alle.len(), "Kollision zwischen Herkünften");
    }

    /// Festgeschriebene Werte (unabhängig mit `hashlib` berechnet): ändert jemand die
    /// Kodierung, brechen gespeicherte oder verglichene Fingerabdrücke.
    #[test]
    fn fingerabdruck_hat_festgeschriebene_werte() {
        assert_eq!(
            hex(Quellen::keine().fingerabdruck()),
            "3e7077fd2f66d689e0cee6a7cf5b37bf2dca7c979af356d0a31cbc5c85605c7d"
        );
        assert_eq!(
            hex(Quellen::unbestimmt().fingerabdruck()),
            "4322fd2bc0a137d1375b37b3b2e2b4715b3d3dd7ca9682438d4fea0f8437fad3"
        );
        assert_eq!(
            hex(Quellen::besprechungen(["bc", "a"]).fingerabdruck()),
            "7a7e56c5609b3211d368094e35a01d72fbb5d1bed042f614b232bc2b783e0fc4"
        );
    }
}
