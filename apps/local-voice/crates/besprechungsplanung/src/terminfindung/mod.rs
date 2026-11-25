//! Terminfindung (lva#845): reine Kernlogik ohne Postfach-, Kalender- oder App-Zugriff.
//!
//! Nur strukturierte Antworten duerfen automatisch gelten. Freitext und
//! Weiterleitungen brauchen Bestaetigung; der Adapter prueft zusaetzlich Absender
//! und Code. Kurzcodes sind keine Authentisierung (siehe [`code`]).

pub mod bewertung;
pub mod code;
pub mod modell;
pub mod parser;
pub mod texte;

pub use bewertung::*;
pub use code::*;
pub use modell::*;
pub use parser::*;
pub use texte::*;

#[cfg(test)]
mod tests;
