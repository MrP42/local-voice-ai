//! wai-assistent: Assistent-Kern fuer ADR-0028 (Stufe A2).
//!
//! - [`regeltabelle`]: Vertragstypen `assistent-einstufung` 0.1.6 (E4.1) mit eigener Pruefung.
//! - [`einstufung`]: lokale Einstufung S0-S3, S0 nur bei exakter Gleichheit, Eskalation ohne Rueckstufung.
//! - [`belegregel`]: Belegtor E4.2 (Quelle, Herleitung, Empfehlung); Saetze ohne Beleg werden verworfen.
//! - [`messung`]: Messpunkte E9, ohne Inhalte serialisierbar.
//! - [`sync`]: geraeteseitiger Sync-Kern (Stufe A1, E3): Umschlag, Lease, Zusammenfuehren, Grabstein, Migration.
//!
//! Keine Systemuhr im Kern: Uhrzeit und Datum kommen ueber [`einstufung::Uhr`].

pub mod belegregel;
pub mod einstufung;
pub mod messung;
pub mod regeltabelle;
pub mod sync;

pub use belegregel::{
    Ansagen, Beleg, Dezimal, Grund, Herleitung, Pruefergebnis, Quellen, QuellenMap, Satz, SatzArt,
    kein_beleg, keine_freie_zeit, keine_quelle_angebot, pruefen,
};
pub use einstufung::{
    Einstufung, FesteUhr, Sprache, Stufe, Uhr, Wochentag, Zeitpunkt, einstufen, eskalieren,
};
pub use messung::{Messung, Weg};
pub use regeltabelle::{Fehler, Regeltabelle};
