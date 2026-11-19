//! Reine, synchron testbare Logik der Medienübersetzung.

pub mod befehle;
pub mod cache;
pub mod events;
pub mod planner;
pub mod sentences;
pub mod sitzung;

pub use crate::media::zeitachse::{Segment, Wort, ZeitGuete};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuellenFehler(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UebersetzungsFehler(pub String);

pub trait MediaSegmentSource {
    fn segmente(
        &mut self,
        von_ms: u64,
        bis_ms: u64,
    ) -> Result<(Vec<Segment>, ZeitGuete), QuellenFehler>;
}

pub trait Uebersetzer {
    fn uebersetzen(&mut self, satz: &str) -> Result<String, UebersetzungsFehler>;

    fn kennung(&self) -> String {
        String::new()
    }
}

/// Fest definierter FNV-1a statt eines pro Prozess zufälligen Hashers.
pub(crate) fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |wert, byte| {
        (wert ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
