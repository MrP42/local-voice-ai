//! Gemeinsamer lokaler Kontospeicher der Suite (ADR-0024, Paket 1: nur lokales Profil).
//!
//! Alle Apps lesen und schreiben denselben Ordner (`profile.json`, `bilder/<id>.png`).
//! Schreibzugriffe laufen unter einer exklusiven Dateisperre (`konto.lock`), lesen frisch,
//! erhöhen `generation` und ersetzen die Datei atomar. Es werden keine Profildaten
//! protokolliert; Fehlermeldungen nennen höchstens Profil-IDs, Feldnamen sowie Zeile,
//! Spalte und Kategorie von Formatfehlern.
//!
//! # Vertrauensgrenze
//! Alle Prozesse desselben Windows-Kontos (bzw. Benutzers) können Profildatei und Bilder
//! lesen und schreiben; `wai-konto` hat keine Zugriffskontrolle. Der Speicher gilt nur für
//! gleich vertrauenswürdige Suite-Apps (ADR-0024 A3).
//!
//! # Änderungen erkennen
//! `generation` zählt jede Schreiboperation; `sitzungsrevision` nur Profilwechsel und
//! (später) Anmelden, Erneuern, Abmelden. Der Wächter [`KontoSpeicher::beobachten`] ist nur
//! ein Hinweis; vor geschützten Aktionen gilt [`KontoSpeicher::pruefe_revision`].

mod beobachter;
mod fehler;
mod modell;
mod pruefen;
mod speicher;

pub use beobachter::Beobachter;
pub use fehler::KontoFehler;
pub use modell::{Anmeldung, Profil, ProfilAenderung, ProfilDatei, ProfilEingabe};
pub use speicher::{KontoSpeicher, standard_wurzel};

/// Version des Dateiformats von `profile.json`.
pub const DATEI_VERSION: u32 = 1;
/// Höchstgröße des Profilbilds in Bytes.
pub const BILD_MAX_BYTES: usize = 256 * 1024;
/// Anzahl der Initialenfarben (Index 0 bis 15).
pub const FARBEN: u8 = 16;
