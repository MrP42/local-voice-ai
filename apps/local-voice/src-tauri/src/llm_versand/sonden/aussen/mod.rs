//! Proben von **außerhalb** von `llm_versand` (siehe [`super`]). Dieses Verzeichnis wird nur mit
//! `--cfg lva_sonde` als `crate::managers::llm::versand_sonden` eingehängt: ein Modul, das weder
//! Nachfahre von `llm_versand` ist noch dessen Sichtbarkeit teilt, am Ort der heutigen Aufrufer.
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

#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "typ"))]
mod aussen_e0599_quellen_default;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "aufloesung"))]
mod aussen_e0603_ausstellen;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "aufloesung"))]
mod aussen_e0603_freigabe;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "aufloesung"))]
mod aussen_e0603_transport;
#[cfg(any(lva_sonde = "kontrolle", lva_sonde = "aufloesung"))]
mod aussen_e0603_versand;
