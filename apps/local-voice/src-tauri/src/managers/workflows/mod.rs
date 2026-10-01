//! Workflow-Automation (Goal „Workflow-Automation“, Issue #67, Paket B1): die
//! Engine-Kerne fuer lokal definierte Ablaeufe aus Ausloeser, Bedingungen und
//! Bausteinen, ohne Abo und ohne fremde Laufzeit.
//!
//! Bausteine dieses Pakets:
//! - `model`: Definition `lva-workflow@1` (Ausloeser, Variablen, lineare Schritte),
//!   Zustaende von Lauf und Schritt.
//! - `catalog`: bekannte Ausloeser und Bausteine, Felder, Rechte, Beispieldaten;
//!   `jsonschema` erzeugt daraus das JSON-Schema.
//! - `expr`: sichere Ausdruecke und Vorlagen-Variablen, ohne Code-Ausfuehrung.
//! - `validate`: Pruefung einer Definition mit Pfad und deutschem Satz je Befund.
//! - `schema` / `store`: Migration Index 8 (Tabellen) und alle Zugriffe auf sie.
//! - `action`: das `Action`-Trait, `RunCtx`, Fehlerklassen, Register der Bausteine.
//! - `heavy`: das Tor fuer schwere Schritte (seriell, RAM-Tor).
//! - `engine`: Warteschlange, Ausfuehrung, Wiederholung, Wiederaufnahme, Freigaben.
//! - `plan` / `cli`: der Trockenlauf (`--workflow-run <datei> --dry-run`).
//! - `builtin`: der eine eingebaute Baustein `wait`.
//!
//! Was dieses Paket NICHT tut: keine Ausloeser (B2/B3), keine App-Bausteine ausser
//! `wait` (B3 bis B6), keine Oberflaeche (B7), keine Agentenbruecke (B8), kein
//! Eingehaengtwerden in den App-Start (B2). Es liefert die Schnittstellen, an denen sie
//! einrasten.
//!
//! # Fehlerfaelle (B1) und ihre Absicherung
//!
//! - **Nebenlaeufigkeit** (zwei Threads/Prozesse, derselbe Ausloeser; zwei Arbeiter,
//!   derselbe Lauf): `UNIQUE (workflow_id, trigger_key)` mit `ON CONFLICT DO NOTHING`
//!   (`engine::tests::the_same_trigger_twice_*`, `...from_many_threads_*`,
//!   `store::tests::the_same_trigger_from_many_threads_*`); ein Lauf wird
//!   mit EINEM bedingten UPDATE geholt (`claim_next`), jede Schreibung des Arbeiters ist
//!   gezaeunt (`store::tests::a_worker_that_lost_the_lease_cannot_write`).
//! - **Abbruch mitten im Vorgang** (App stirbt vor/nach dem Baustein, vor/nach dem
//!   Journal): Journal `running` -> `done`; Wiederaufnahme nach Ablauf des
//!   Mietvertrags uebernimmt nur Erledigtes, Reines und Wiederholbares von selbst,
//!   Schritte mit Aussenwirkung nie ohne Beleg (`engine::tests::a_crash_*`, `a_step_that_keeps_killing_the_app_*`,
//!   `the_effect_can_be_confirmed_after_a_crash_*`).
//! - **Voller Datentraeger / gesperrte Datenbank**: jede Schreibung ist ein Statement
//!   oder eine Transaktion; scheitert das Journal NACH dem Baustein, bleibt der Schritt
//!   `running` und der Vertrag laeuft ab, danach greift die Wiederaufnahme
//!   (`engine::tests::a_failing_journal_write_after_the_action_*`). Die Migration ist eine
//!   Transaktion (`store::tests::an_abort_inside_the_workflow_migration_*`).
//! - **Fehlendes Geraet / Audio-Echtzeitpfad**: nicht beteiligt (die Engine fasst weder
//!   Audio noch Geraete an; ein fehlendes Mikrofon ist ein `Permanent` des Bausteins
//!   `recording.start`, B2). Es gibt in diesem Modul keinen Audio-Callback.
//! - **Absturz eines Kindprozesses**: ein Baustein meldet ihn als `Transient` (nichts
//!   geschrieben) oder `Unknown` (unklar), die Engine wiederholt bzw. haelt an
//!   (`engine::tests::a_transient_failure_is_retried_*`, `retries_stop_*`,
//!   `an_unknown_failure_*`, `a_panicking_action_*`); ein Kindprozess startet nur
//!   ueber `process_guard` (Vertrag in `action`).
//! - **Voller Arbeitsspeicher**: schwere Schritte warten am RAM-Tor, statt zu scheitern
//!   (`heavy::tests::*`, `engine::tests::a_heavy_step_waits_*`).
//! - **Warteschlange und Speicher**: je Ablauf hoechstens 500 wartende Laeufe
//!   (abgelehnt und gezaehlt), Aufbewahrung beendeter Laeufe gedeckelt (200 je Ablauf,
//!   5000 gesamt), Ausgaben je Schritt hoechstens 64 KiB
//!   (`store::tests::*`, `engine::tests::*`).

#![allow(dead_code)]

pub mod action;
pub mod builtin;
pub mod catalog;
pub mod cli;
pub mod engine;
pub mod expr;
pub mod heavy;
pub mod jsonschema;
pub mod model;
pub mod plan;
pub mod schema;
pub mod store;
pub mod templates;
pub mod validate;

#[allow(unused_imports)]
pub use action::{Action, ActionRegistry, EffectKind, StepError, StepOutput};
#[allow(unused_imports)]
pub use engine::{Engine, EngineConfig, EnqueueRequest, RunOutcome};
#[allow(unused_imports)]
pub use model::{RunState, StepState, WorkflowDef};
#[allow(unused_imports)]
pub use store::WorkflowError;

#[cfg(test)]
pub(crate) mod test_support;
