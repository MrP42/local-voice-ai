//! Notizbuch ↔ Besprechung zuordnen und Notizen zusammenführen (lva#396, Stufe N4a).
//!
//! Reine, deterministische Logik nach Spec `2026-11-08-notizbuch-fuer-assistent.md` §5 (ohne IO):
//!
//! - [`zeit`]: Blockzeit-Intervalle mit Güte und Abbildung auf die Zeitachse (§5.1, §5.3)
//! - [`zuordnung`]: Kandidaten, Bewertung, Einstufung (§5.2)
//! - [`verlust`]: Duplikate, Widersprüche, Verlustprüfung (§5.4)
//!
//! Verdrahtung (Schritt 2):
//!
//! - [`links`]: Speicher `meeting_note_links` im Besprechungs-Store (Migration Index 16)
//! - [`dienst`]: `zuordnen(meeting_id)` über den lesenden Zugang, nur freigegebene Abschnitte
//! - [`quelle`]: zugeordnete Seiten als zweite Notizquelle der KI-Notizen, Verlustprüfung
//! - [`schutz`]: Notizbuch-Inhalt gespeicherter KI-Notizen nur an lokale Modelle
//!
//! Inhalte aus dem Notizbuch sind hier bloße Daten: Text wird nur gefaltet, gezählt und
//! verglichen, nie interpretiert oder ausgeführt.
#![allow(dead_code)] // Oberfläche und Tor-Messung folgen in den nächsten N4a-Schritten

use std::collections::BTreeSet;
use wai_notizbuch_zugang::falte;

pub mod dienst;
pub mod links;
pub mod quelle;
pub mod schutz;
pub mod verlust;
pub mod zeit;
pub mod zuordnung;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod verdrahtung_tests;

#[allow(unused_imports)]
pub use verlust::{
    fehlende_bloecke, finde_duplikate, finde_widersprueche, pruefbloecke_aus_seite,
    unvollstaendige_seiten, Abweichung, Beziehung, Paar, PruefBlock, Quelle, Widerspruch,
};
#[allow(unused_imports)]
pub use zeit::{
    abbilden, anzeige, blockzeit, lage, parse_iso_ms, Abbild, Blockzeit, Guete, Lage, Zeitachse,
};
#[allow(unused_imports)]
pub use zuordnung::{
    bewerte, einstufen, entschaerfe_parallele, entscheide, ist_kandidat, Abschnitt, Besprechung,
    Bewertung, Einstufung, Entscheidung, Grund, Kandidat,
};

/// Füllwörter (gefaltet), die als Begriff nichts beitragen. Verneinungen stehen absichtlich
/// hier: sie werden getrennt als Merkmal erfasst.
const FUELLWOERTER: &[&str] = &[
    "nicht", "nichts", "kein", "keine", "keinen", "keinem", "keiner", "keines", "dass", "aber",
    "auch", "oder", "wenn", "dann", "noch", "sind", "wird", "werden", "wurde", "haben", "einen",
    "einem", "einer", "eines", "diese", "dieser", "dieses", "diesen", "diesem", "nach", "sein",
    "that", "this", "with", "have", "from", "will", "they", "been", "were", "there", "their",
    "what", "when", "which", "about", "would", "also", "then", "than",
];

/// Wörter des gefalteten Textes: Buchstaben und Ziffern, alles andere trennt.
pub(crate) fn woerter(text: &str) -> Vec<String> {
    falte(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Begriffe eines Textes: gefaltete Wörter ab vier Zeichen (ohne Füllwörter) sowie Zahlen
/// und Wörter mit Ziffern ab zwei Zeichen.
pub(crate) fn begriffe(text: &str) -> BTreeSet<String> {
    woerter(text)
        .into_iter()
        .filter(|w| {
            if w.chars().any(|c| c.is_ascii_digit()) {
                return w.chars().count() >= 2;
            }
            w.chars().count() >= 4 && !FUELLWOERTER.contains(&w.as_str())
        })
        .collect()
}
