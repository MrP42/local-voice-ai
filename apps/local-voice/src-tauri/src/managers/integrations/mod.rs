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
//! - `preview`: gegliederte Vorschau einer Freigabe (Ziel und sicherheitsrelevante
//!   Felder vollstaendig, sonst Ablehnung; A1n).
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
//! - **Speicher**: Konfiguration 64 KiB, Audit 20 000 Zeilen (Verweigerungen zuerst
//!   verdraengt und je Schluessel zusammengefasst), Freigaben 50 offen (je Aufrufer und
//!   Integration 10, gleiche Anfragen wiederverwendet), Quellen je Provenienz 200
//!   (Tests in den Modulen).
//!
//! Haertung A1n (Sicherheits-Review B2) und ihre Absicherung:
//! - **Unsichtbare Empfaenger** (langer Text vor `to`/`bcc`): `preview::tests::*`,
//!   `gate::tests::a_long_body_cannot_hide_the_recipients_from_the_user`; nicht
//!   darstellbar -> `preview_unsafe`, nichts liegt zur Freigabe vor
//!   (`gate::tests::a_request_whose_recipients_cannot_be_shown_in_full_is_refused_not_clipped`).
//! - **Audit-Flutung**: `audit::tests::repeated_denials_within_the_window_*`,
//!   `a_full_log_drops_denied_rows_first_*`.
//! - **Freigabe-Flutung und gleichzeitige gleiche Anfragen**: `approvals::tests::*`,
//!   `gate::tests::concurrent_identical_asks_share_one_approval` (eine `IMMEDIATE`-
//!   Transaktion je `open`).
//! - **Platte voll nach dem Anlegen der Freigabe**: die Freigabe wird zurueckgezogen
//!   (`gate::tests::a_failing_audit_write_leaves_no_open_approval_behind`).
//! - **Fremde Kennungen, Zugangsschluessel im Pfad, Kalender-Trigger, unbekannte Art
//!   im Register**: `gate::tests::a_hostile_integration_id_*`,
//!   `audit::tests::an_address_in_the_audit_*`, `tests::a_calendar_update_does_not_*`,
//!   `store::tests::list_skips_a_row_*`.

#![allow(dead_code)]

pub mod adopt;
pub mod approvals;
pub mod audit;
pub mod dump;
pub mod gate;
pub mod grants;
pub mod model;
pub mod preview;
pub mod schema;
pub mod secrets;
pub mod store;
pub mod view; // A4

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
