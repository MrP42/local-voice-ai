//! Lokaler Kalender ohne Konto (M5, F15; `entwurf/m5-m6-kalender-export.md`).
//!
//! P5a legt das Fundament: Datentypen (`model`), ICS-Parser mit Serienexpansion
//! (`ics`, rein), Abruf (`fetch`) und DPAPI-Geheimnisse (`secret`). Cache und
//! Verknuepfungen liegen als `impl MeetingStore` in `meetings/store.rs`. Der
//! Sync-Dienst, die Erinnerung und die Oberflaeche folgen in P5b; deshalb ist
//! vieles hier noch ungenutzt.
//!
//! Fehlerfaelle (P5a) und ihre Absicherung:
//! - Zwei Schreiber (App und Headless, zwei Sync-Laeufe): `replace_source_events`
//!   und Migrationen laufen in `IMMEDIATE`-Transaktionen mit Busy-Timeout; Test
//!   `concurrent_resyncs_leave_a_consistent_source`, `opening_..._from_several_threads`.
//! - Abbruch mitten im Vorgang: Migration 4 und `replace_source_events` sind je
//!   EINE Transaktion (Tests `migration_4_is_all_or_nothing_...`,
//!   `replace_is_all_or_nothing_...`); Geheimnisse erst Temp-Datei, dann atomar
//!   umbenennen (`secret::tests::a_failed_write_leaves_no_temp_file_...`).
//! - Voller Datentraeger: derselbe Weg (Fehler statt halber Stand; der Trigger-Test
//!   simuliert `database or disk is full`); Geheimnis-Schreibfehler werden gemeldet.
//! - Kein Netz, Login-Seite, zu gross, Timeout, Umleitungsschleife: `fetch::tests`;
//!   der Cache bleibt beim Fehler unberuehrt (Aufrufer schreibt nur bei Erfolg).
//! - Speicher: Abruf bei 20 MB abgebrochen (auch ohne Content-Length), Expansion
//!   je Serie auf 20 000 Instanzen und gesamt auf 2 Mio., Termine je Quelle auf
//!   20 000 begrenzt (`ics::tests::old_daily_series_is_truncated_with_warning`).
//! - Kindprozess: keiner (Abruf und Parser laufen im Prozess); entfaellt.
//! - Fremdes Konto / kaputte Geheimnisdatei: `secret_get` meldet Klartextfehler,
//!   die Quelle zeigt dann „Adresse neu eingeben“ (`secret::tests`).

#![allow(dead_code)]

pub mod dump;
pub mod fetch;
pub mod ics;
pub mod model;
pub mod secret;
