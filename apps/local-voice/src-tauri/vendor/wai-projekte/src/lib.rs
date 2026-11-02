//! wai-projekte: Suite-Projektverzeichnis (ADR-0026, PR-2).
//!
//! Ein Projekt hat eine Kennung ueber alle Module; Eintraege sind Verweise (`wai://`) auf Inhalte der
//! Herkunftsapp, keine Kopien. Mandant kommt nur aus dem `Principal`; fremde Datensaetze sind "nicht
//! gefunden". Die Crate ist keine Sicherheitsgrenze gegen direkten Dateizugriff (ADR-0026 E1).

mod fehler;
mod hilfen;
mod modell;
mod verzeichnis;

pub use fehler::{Ergebnis, ProjektFehler};
pub use modell::*;
pub use verzeichnis::{
    Herkunftsbestand, Herkunftsrecht, MAX_EREIGNISSE, Projektverzeichnis, SCHEMA_STAND, Vorhanden,
};
