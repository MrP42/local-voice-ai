//! Register der Integrationen (Goal „Integrationen“, Issue #66, Paket A1):
//! Kalender, Mail, Speicher, Wissen und Agenten als Verbindungen mit Richtung
//! und einem Recht je Faehigkeit (aus / fragen / erlaubt).
//!
//! Bausteine:
//! - `schema`: Migration Index 5 (Register, Rechte, Audit, Freigaben, Provenienz).
//! - `model`: Arten, Richtungen, Faehigkeiten, Aufrufer, Rechte.
//! - `store`: Anlegen, Aendern, Entfernen, Rechte setzen (Konfiguration ohne Geheimnisse).
//! - `grants`: `effective_mode`, die reine Rechteregel (E3: externe Agenten aus,
//!   Schreibendes fragen, Aufnahme nie dauerhaft erlaubt).
//! - `audit`, `approvals`, `gate`: jede Aktion eines Nicht-Nutzers ist
//!   protokolliert, „fragen“ erzeugt eine einmalige, an die Ausfuehrung
//!   gebundene Freigabe, ohne Audit keine Aktion.
//! - `adopt`: Uebernahme der Kalenderquellen unter gleicher ID (idempotent).
//! - `secrets`: Geheimnis-Namensraum ueber dem DPAPI-Speicher.
//! - `dump`: `--integrations-dump` (JSON, nur in der Sandbox).
//!
//! Was dieses Paket NICHT tut: keine Oberflaeche (A4), keine Konten und Ziele
//! (A5/A6), keine Pipe/Token fuer Agenten (A7). Es liefert das Fundament, auf
//! dem sie aufsetzen.
//!
//! Fehlerfaelle (A1) und ihre Absicherung:
//! - **Migration mit Altdaten**: Fixture mit zwei Kalenderquellen (ICS, Graph) und
//!   einer entfernten -> zwei Integrationen gleicher ID; zweiter Start ohne
//!   Dubletten (`tests::migration_*`, `adopt::tests`).
//! - **Abbruch mitten in der Migration**: eine Transaktion, `user_version` bleibt
//!   (`tests::migration_5_is_all_or_nothing_when_it_fails_midway`).
//! - **Zwei Schreiber / Nebenlaeufigkeit**: Oeffnen aus mehreren Threads und
//!   parallele Freigabe-Entscheidungen (`tests::opening_from_several_threads_*`,
//!   `approvals::tests::only_one_of_two_concurrent_deciders_wins`), Audit-Schreiber
//!   (`audit::tests::concurrent_writers_keep_every_row`).
//! - **Voller Datentraeger**: alle Schreibwege sind einzelne Statements oder
//!   Transaktionen; `gate::run` bricht ohne Audit-Eintrag ab
//!   (`gate::tests::a_failing_audit_write_blocks_the_action`).
//! - **Geheimnis fehlt oder ist kaputt**: `secrets::status*` meldet
//!   `missing`/`broken` mit Klartext, nie Inhalt (`secrets::tests`); der Dump zeigt
//!   nur den Zustand.
//! - **Kindprozess / fehlendes Geraet / Audio**: nicht beteiligt (Datenbank und
//!   Dateien); entfaellt.
//! - **Speicher**: Konfiguration 64 KiB, Audit 20 000 Zeilen, Freigaben 50 offen,
//!   Quellen je Provenienz 200 (Tests in den Modulen).

#![allow(dead_code)]

pub mod adopt;
pub mod approvals;
pub mod audit;
pub mod dump;
pub mod gate;
pub mod grants;
pub mod model;
pub mod schema;
pub mod secrets;
pub mod store;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
