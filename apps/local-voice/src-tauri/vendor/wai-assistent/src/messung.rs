//! Messpunkte nach ADR-0028 E9. Zeiten sind Millisekunden relativ zum Start (Sprachende bzw. Senden);
//! keine Inhalte, nur Zeiten und Merkmale.

use serde::{Deserialize, Serialize};

use crate::einstufung::Stufe;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weg {
    Lokal,
    Relay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Messung {
    pub beitrag_id: String,
    /// Startzeit in ms auf der Uhr des Aufrufers (monoton); alle weiteren Werte sind relativ dazu.
    pub start: u64,
    pub erste_ansage_ms: Option<u64>,
    pub erster_inhalt_ms: Option<u64>,
    pub fertig_ms: Option<u64>,
    pub warm: bool,
    pub weg: Weg,
    pub stufe: Stufe,
}

impl Messung {
    pub fn neu(
        beitrag_id: impl Into<String>,
        start: u64,
        warm: bool,
        weg: Weg,
        stufe: Stufe,
    ) -> Self {
        Self {
            beitrag_id: beitrag_id.into(),
            start,
            erste_ansage_ms: None,
            erster_inhalt_ms: None,
            fertig_ms: None,
            warm,
            weg,
            stufe,
        }
    }

    fn relativ(&self, t: u64) -> u64 {
        t.saturating_sub(self.start)
    }

    /// Erste hoerbare/sichtbare Ansage; nur der erste Wert zaehlt.
    pub fn ansage(&mut self, t: u64) {
        let r = self.relativ(t);
        self.erste_ansage_ms.get_or_insert(r);
    }

    /// Erster Inhalt (erster Audio-Frame bzw. erstes sichtbares Zeichen); nur der erste Wert zaehlt.
    pub fn inhalt(&mut self, t: u64) {
        let r = self.relativ(t);
        self.erster_inhalt_ms.get_or_insert(r);
    }

    /// Abschluss; nur der erste Wert zaehlt.
    pub fn fertig(&mut self, t: u64) {
        let r = self.relativ(t);
        self.fertig_ms.get_or_insert(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nur_erster_wert_zaehlt_und_relativ() {
        let mut m = Messung::neu("b1", 1_000, true, Weg::Lokal, Stufe::S1);
        m.ansage(1_700);
        m.ansage(1_900);
        m.inhalt(4_000);
        m.inhalt(5_000);
        m.fertig(6_000);
        m.fertig(7_000);
        assert_eq!(m.erste_ansage_ms, Some(700));
        assert_eq!(m.erster_inhalt_ms, Some(3_000));
        assert_eq!(m.fertig_ms, Some(5_000));
    }

    #[test]
    fn nie_negativ() {
        let mut m = Messung::neu("b", 500, false, Weg::Relay, Stufe::S0);
        m.ansage(100);
        assert_eq!(m.erste_ansage_ms, Some(0));
    }

    #[test]
    fn serialisiert_ohne_inhalte() {
        let mut m = Messung::neu("b-7", 0, true, Weg::Relay, Stufe::S2);
        m.inhalt(1_200);
        let j = serde_json::to_value(&m).unwrap();
        assert_eq!(j["weg"], "relay");
        assert_eq!(j["stufe"], "S2");
        assert_eq!(j["erster_inhalt_ms"], 1200);
        assert!(j["erste_ansage_ms"].is_null());
        let zurueck: Messung = serde_json::from_value(j).unwrap();
        assert_eq!(zurueck, m);
    }
}
