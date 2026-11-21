//! Prüfstand für den Sprachdienst: führt dessen Tests aus, ohne die App zu bauen.
//!
//! Dieses Crate enthält **keinen** Dienstcode. Es bindet die Originalquellen aus
//! `src-tauri/src/sprachdienst` per `#[path]` ein, unter demselben Modulpfad wie in der App
//! (`crate::sprachdienst::…`, `crate::testlog`). Was hier grün ist, ist derselbe Quelltext.
//!
//! Abweichungen vom echten Crate, siehe README: ohne `app.rs` (Tauri-Anbindung) und ohne
//! `pipe.rs` (Windows), nur unter Unix, eigene Abhängigkeitsliste. Maßgeblich bleibt das Rust-Tor
//! im App-Crate.
//!
//! Diese Datei enthält nur die drei Modulzeilen unten; `waechter::wurzel_ist_unveraendert` prüft
//! das. Alles Weitere steht in `sprachdienst/mod.rs` (Spiegel) und `waechter.rs`.
#![cfg(unix)]

#[cfg(test)]
#[path = "../../../src-tauri/src/testlog.rs"]
mod testlog;

pub mod sprachdienst;

#[cfg(test)]
mod waechter;
