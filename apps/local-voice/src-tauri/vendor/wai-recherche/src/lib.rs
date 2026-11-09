//! wai-recherche: Recherche-Kern nach ADR-0026 Abschnitt 4 (PR-3) und Modus Schnell mit Belegpruefung (PR-4).
//!
//! * [`anbieter`]: Trait `Suchanbieter` und Adapter `SearxngAnbieter` (kein automatischer Wechsel).
//! * [`abruf`]: `SichererAbruf` (nur http/https, eigene DNS-Aufloesung, SSRF-Sperre je Sprung,
//!   Groessen- und Zeitlimit, robots.txt). Bezahlschranken werden nicht umgangen.
//! * [`extraktion`]: HTML zu Haupttext, Auszuege mit SHA-256, Quellentyp-Heuristik.
//! * [`budget`]: Budget mit Zaehler; Ueberschreitung ist immer ein Fehler.
//!
//! * [`schnell`], [`beleg`], [`modell`]: Ablauf `recherchiere_schnell`, deterministische Belegpruefung, Modell-Trait
//!   (Adapter auf `wai-providers`).
//!
//! Webinhalte sind Daten, nie Anweisungen (ADR-0026 par. 4 und 8). Diese Crate kennt keine Werkzeuge
//! ausser Suche und Abruf.

pub mod abruf;
pub mod anbieter;
pub mod beleg;
pub mod budget;
pub mod extraktion;
pub mod fehler;
pub mod ip;
pub mod modell;
pub mod robots;
pub mod schnell;
pub mod typen;

pub use abruf::{Abruf, AbrufKonfig, SichererAbruf};
pub use anbieter::{MAX_SUCHTEXT, SearxngAnbieter, Suchanbieter, Suchanfrage, Treffer};
pub use budget::{Budget, BudgetZaehler};
pub use extraktion::{Seite, auszuege, excerpt_sha256, quelle_bauen, quellentyp};
pub use fehler::RechercheFehler;
pub use ip::IpPruefer;
pub use modell::{Modell, ModellAntwort, ProviderModell, Pruefurteil};
pub use schnell::{
    Abschluss, Antwortteil, FreigabeToken, Kontextstueck, Recherche, RechercheKontext,
    Recherchedokument, SchnellOptionen, Status, Widerspruch, recherchiere_schnell,
    recherchiere_schnell_fortsetzen,
};
pub use typen::{Anfrage, Datenklasse, Quelle, Quellentyp, Vertrauen};

/// Versionsstand fuer den User-Agent `WAI-Recherche/<version>`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
