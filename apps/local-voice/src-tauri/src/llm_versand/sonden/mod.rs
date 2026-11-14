//! Proben für die Compile-Zeit-Zusicherungen (lva#604, Teilpaket 4). **Nur** mit `--cfg lva_sonde`;
//! ein normaler Build, `cargo test` und `cargo clippy` sehen dieses Verzeichnis nie.
//!
//! Jede Datei enthält **genau einen Verstoß**, der der Compiler mit einem bestimmten Fehlercode
//! abweisen muss, und daneben denselben Code **ohne** Verstoß (`#[cfg(lva_sonde = "kontrolle")]`),
//! der übersetzen muss. Gestartet und ausgewertet werden die Läufe vom Test
//! `zusicherungen::compile_zeit_zusicherungen` (siehe dort und die Moduldoku von
//! [`llm_versand`](super)). Der Fehlercode jeder Probe steht in der Tabelle dieses Tests, nicht
//! hier: eine Probe, die aus einem anderen Grund scheitert, besteht nicht.
//!
//! Läufe (`--cfg lva_sonde="<lauf>"`):
//!
//! | Lauf | Proben | Erwartung |
//! |---|---|---|
//! | `kontrolle` | alle, ohne Verstoß | übersetzt fehlerfrei |
//! | `aufloesung` | E0603 | genau diese Fehler, je Probe in ihrer Datei |
//! | `typ` | E0599, E0624 | dito |
//! | `feld` | E0451 | dito |
//! | `ausleihe` | E0382 | dito (der Borrowchecker läuft erst nach fehlerfreier Typprüfung) |
//!
//! Die Proben in diesem Verzeichnis liegen **innerhalb** von `llm_versand` (sehen also alles, was
//! `pub(super)` ist); die Proben von **außerhalb** liegen in `aussen/` und werden als
//! `crate::managers::llm::versand_sonden` eingehängt.
#![allow(
    unexpected_cfgs,
    dead_code,
    unused_imports,
    unused_variables,
    unused_must_use
)]

/// Kontrollmarke: der Lauf `kontrolle` meldet sie als Warnung (veraltet) in der Datei der Probe.
/// Das belegt, dass die Probe tatsächlich übersetzt wurde und nicht nur nie eingebunden war.
#[cfg(lva_sonde = "kontrolle")]
#[deprecated(note = "Kontrollmarke")]
const fn kontrollmarke() {}

#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "ausleihe"))]
mod innen_e0382_kopie;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "ausleihe"))]
mod innen_e0382_zweimal;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "feld"))]
mod innen_e0451_literal;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "typ"))]
mod innen_e0599_default;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "typ"))]
mod innen_e0599_klon;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "aufloesung"))]
mod innen_e0603_cli;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "aufloesung"))]
mod innen_e0603_pruefer_app;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "typ"))]
mod innen_e0624_ausstellen;
