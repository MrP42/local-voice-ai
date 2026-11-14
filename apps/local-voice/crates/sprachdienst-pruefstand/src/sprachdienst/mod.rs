//! Spiegel von `src-tauri/src/sprachdienst/mod.rs`: dieselben Module, aus den Originaldateien.
//! Nicht eingebunden sind `app` (Tauri-Anbindung) und `pipe` (Windows). Der Test
//! `waechter::module_folgen_dem_dienst` hält diese Liste mit dem Original gleich.

#[path = "../../../../src-tauri/src/sprachdienst/dienst.rs"]
pub mod dienst;
#[path = "../../../../src-tauri/src/sprachdienst/engine.rs"]
pub mod engine;
#[path = "../../../../src-tauri/src/sprachdienst/kontext.rs"]
pub mod kontext;
#[path = "../../../../src-tauri/src/sprachdienst/kopplung.rs"]
pub mod kopplung;
#[path = "../../../../src-tauri/src/sprachdienst/unix.rs"]
pub mod unix;
#[path = "../../../../src-tauri/src/sprachdienst/verbindung.rs"]
pub mod verbindung;
#[path = "../../../../src-tauri/src/sprachdienst/wav.rs"]
pub mod wav;
#[path = "../../../../src-tauri/src/sprachdienst/zugang.rs"]
pub mod zugang;

#[cfg(test)]
#[path = "../../../../src-tauri/src/sprachdienst/tests.rs"]
mod tests;
