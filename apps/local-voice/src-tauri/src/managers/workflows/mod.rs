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
//! - `schema` / `store`: Migration Index 11 (Tabellen) und alle Zugriffe auf sie.
//! - `action`: das `Action`-Trait, `RunCtx`, Fehlerklassen, Register der Bausteine.
//! - `heavy`: das Tor fuer schwere Schritte (seriell, RAM-Tor).
//! - `engine`: Warteschlange, Ausfuehrung, Wiederholung, Wiederaufnahme, Freigaben.
//! - `plan` / `cli`: der Trockenlauf (`--workflow-run <datei> --dry-run`).
//! - `builtin`: der eine eingebaute Baustein `wait`.
//!
//! Bausteine von Paket B2 (Ausloeser und Einwilligung):
//! - `trigger`: Kalender „Termin beginnt/endet“ (`trigger::calendar`, am Takt der Erinnerung,
//!   kein zweiter Poller), Besprechungsereignisse (`trigger::meeting_events`), Zeitplan
//!   (`trigger::schedule`, feste Uhr testbar), manuell (`trigger::manual`). Jeder bildet einen
//!   stabilen Schluessel; die Engine macht daraus hoechstens einen Lauf.
//! - `recording`: die Bausteine `recording.start`/`recording.stop`, der Traeger des Rechts
//!   (Integration `app-automation`, Entscheidung dort dokumentiert) und die vier Sperren, die
//!   „ohne Klick keine Aufnahme“ sichern.
//! - `consent`: die offenen Bitten um Einwilligung und ihre Entscheidung (Hinweisfenster).
//! - `hub`: der Kleber an die App: Engine starten und beenden, Takt, Hoerer an den
//!   Besprechungsereignissen, Hinweisfenster. Eingehaengt in `lib.rs`
//!   (`initialize_core_logic`, `RunEvent::Exit`) und in `CalendarService::remind_tick`.
//!
//! Was die Pakete NICHT tun: keine Ordner-/YouTube-Ausloeser (B3), keine App-Bausteine ausser
//! `wait` und der Aufnahme (B3 bis B6), keine Oberflaeche (B7), keine Agentenbruecke (B8).
//! Sie liefern die Schnittstellen, an denen die naechsten einrasten.
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
//!
//! # Fehlerfaelle (B2) und ihre Absicherung
//!
//! - **Nebenlaeufigkeit**: derselbe Termin/Zeitpunkt/dasselbe Ereignis aus zwei Takten, zwei
//!   Threads oder nach einem Neustart ergibt einen Lauf (Schluessel + `UNIQUE`):
//!   `trigger::calendar::tests::it_fires_exactly_once_*`, `trigger::schedule::tests::two_schedules_*`,
//!   `trigger::meeting_events::tests::the_same_event_twice_*`. Zwei Ablaeufe, die denselben Termin
//!   aufnehmen wollen: zwei Bitten, EINE Aufnahme (`recording::tests::two_runs_for_the_same_event_*`).
//! - **Abbruch mitten im Vorgang**: Absturz nach dem Aufnahmestart wird ueber den Recorder
//!   bestaetigt oder ist `effect_uncertain`, nie ein zweiter Start (`recording::tests::a_crash_*`);
//!   die Entscheidung des Nutzers ueberlebt einen Neustart (`an_app_restart_between_*`); das
//!   Beenden der App bricht Laeufe nicht ab (`engine::tests::stop_within_*`).
//! - **Voller Datentraeger / gesperrte Datenbank**: ein gescheitertes Einreihen steht im Bericht und
//!   wird vom naechsten Takt wiederholt, solange der Ausloeser gilt (`a_failed_enqueue_*` je Ausloeser,
//!   `a_full_queue_*`); eine vergessene Aufnahme hat immer ein Ende (Sicherheitsnetz 480 min).
//! - **Fehlendes Geraet**: Mikrofon/Systemton fehlen -> `Permanent`, der Lauf fragt nicht erneut
//!   (`recording::tests::a_missing_microphone_*`).
//! - **Absturz eines Kindprozesses**: B2 startet keinen Prozess (der Recorder laeuft im Prozess).
//! - **Echtzeit-Audiopfad**: unberuehrt. Kein Code der Ausloeser oder Bausteine laeuft im
//!   Audio-Callback; der Start ist derselbe Aufruf wie von Hand.
//! - **Voller Arbeitsspeicher**: `recording.start` braucht kein Tor (leichter Mitschnitt); Live-
//!   Transkript und Enddurchlauf haben die vorhandenen Tore, schwere Schritte danach das `HeavyGate`.
//! - **Einwilligung**: ohne Klick, nach „Nein“, nach Verfall (1 h), bei „aus“, im Trockenlauf und
//!   bei einer Freigabe fuer einen beendeten Termin startet keine Aufnahme (`recording::tests::*`,
//!   `consent::tests::*`, Playwright `workflow-consent.spec.ts`).

//! Bausteine von Paket B4 (App-Bausteine): `app_actions` (KI-Notizen, Protokoll, Zusammenfassung,
//! Ablegen in einen Ordner, Vorlesen als Audiodatei, lokale Mitteilung) hinter dem Trait
//! `AppServices`; `app_services` ist dessen Umsetzung in der App (eingehaengt in `hub::start`),
//! `toast` die Windows-Mitteilung. Vorlage `eingangsordner-word` in `templates`.
//! Fehlerfaelle, Idempotenz und Rechte stehen im Kopf von `app_actions`.
//!
#![allow(dead_code)]

pub mod action;
pub mod app_actions; // B4
pub mod app_services; // B4
pub mod builtin;
pub mod catalog;
pub mod cli;
pub mod consent; // B2
pub mod engine;
pub mod expr;
pub mod heavy;
pub mod hub; // B2
pub mod jsonschema;
pub mod model;
pub mod plan;
pub mod recording; // B2
pub mod schema;
pub mod store;
pub mod templates;
pub mod toast; // B4
pub mod trigger; // B2
pub mod validate;

#[allow(unused_imports)]
pub use action::{Action, ActionRegistry, EffectKind, StepError, StepOutput};
#[allow(unused_imports)]
pub use engine::{Engine, EngineConfig, EngineObserver, EnqueueRequest, RunOutcome};
#[allow(unused_imports)]
pub use model::{RunState, StepState, WorkflowDef};
#[allow(unused_imports)]
pub use store::WorkflowError;

#[cfg(test)]
pub(crate) mod test_support;
