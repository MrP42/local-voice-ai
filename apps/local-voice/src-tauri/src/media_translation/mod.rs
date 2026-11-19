//! Reine, synchron testbare Logik der Medienübersetzung.

pub mod cache;
pub mod events;
pub mod planner;
pub mod sentences;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZeitGuete {
    Wort,
    Satz,
    Grob,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wort {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Alle Zeiten sind absolut. Ein Quellenadapter rechnet lokale Offsets einmalig um.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub woerter: Option<Vec<Wort>>,
}

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
}

/// Fest definierter FNV-1a statt eines pro Prozess zufälligen Hashers.
pub(crate) fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |wert, byte| {
        (wert ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
