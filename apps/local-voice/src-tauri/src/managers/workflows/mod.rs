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
//! Bausteine von Paket B3 (Ordner, YouTube-Kanal, Import):
//! - `trigger::folder`: „Datei im Ordner“ (Takt-Scan, Stabilitaet, Dateiledger, OneDrive-
//!   Platzhalter), `trigger::youtube_channel`: „Neues Video im Kanal“ (RSS, Intervall, Ledger,
//!   Ausfall), `trigger::ledger`: die Tabelle `workflow_file_ledger`.
//! - `import`: die Bausteine `meeting.import` (Datei -> Besprechung ueber die vorhandene
//!   Import-Warteschlange) und `youtube.add_source` (Video -> Quelle ueber den Weg aus A2).
//! - `queue_gate`: das Tor der schweren Schritte, das Aufnahme und Import-Warteschlange kennt;
//!   `import_app`: der Kleber an die App (Warteschlange, Speicher, `AppHandle`).
//!
//! Was die Pakete NICHT tun: keine App-Bausteine ausser `wait`, Aufnahme und Import (B4 bis B6),
//! kein `youtube.transcript` (B6), keine Oberflaeche (B7), keine Agentenbruecke (B8).
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

//! # Fehlerfaelle (B3) und ihre Absicherung
//!
//! - **Nebenlaeufigkeit**: dieselbe Datei/dasselbe Video aus zwei Takten, zwei Threads oder nach
//!   einem Neustart ergibt einen Lauf (Schluessel `file:<sha256>` bzw. `yt:<kanal>:<video>` +
//!   `UNIQUE`, dazu das Ledger): `trigger::folder::tests::the_ledger_*`, `without_a_ledger_*`,
//!   `trigger::youtube_channel::tests::ak11_*`, `a_restart_*`. Zwei Scans/Abrufrunden zugleich
//!   gibt es nie (`State::try_begin`, `a_second_scan_*`, `a_second_round_*`). Zwei Ablaeufe auf
//!   demselben Ordner/Kanal bekommen je ihre Laeufe (`two_workflows_*`). Dieselbe Datei in der
//!   Import-Warteschlange zweimal verhindert die Suche nach der Quelldatei seit Schrittbeginn
//!   (`import::tests::a_repeated_step_reuses_*`).
//! - **Abbruch mitten im Vorgang**: Einreihen zuerst, Ledger danach; scheitert das Einreihen,
//!   bleibt die Datei/das Video ungesehen und der naechste Takt/Abruf holt es nach, scheitert das
//!   Ledger, ist der naechste Versuch ein Duplikat der Engine (`a_failed_enqueue_*` je Ausloeser).
//!   Eine Datei, die waehrend des Hashens waechst, zaehlt nicht als fertig. Nach einem Absturz
//!   zwischen Einreihen in die Import-Warteschlange und Journal belegt `confirm` die Besprechung
//!   (`External` wird nie blind wiederholt).
//! - **Voller Datentraeger / gesperrte Datenbank**: jeder Ledger-Zugriff ist ein Statement; ein
//!   Fehler steht im Bericht (`TickReport::errors`), nichts geht verloren. Das Ledger ist
//!   gedeckelt (20 000 Dateien, 50 000 Videos, aelteste zuerst).
//! - **Fehlendes Geraet / OneDrive**: Ordner nicht eingehaengt oder Integration geloescht: ein
//!   Hinweis je Aenderung statt alle 15 s (`a_missing_integration_or_folder_*`). Platzhalter
//!   (nur in der Cloud) werden nie gelesen oder heruntergeladen, einmal gemeldet
//!   (`a_cloud_only_placeholder_*`); als Import-Quelle ein dauerhafter Fehler mit Hinweis.
//! - **Netz**: YouTube-Feed nicht erreichbar/404/429: Zaehler, nach 3 Fehlschlaegen in Folge
//!   EIN Ausfallbericht plus Audit-Eintrag, Wiederholung nach 5 min mal Fehlerzahl (hoechstens
//!   `poll_minutes`), Rueckkehr wird gemeldet (`repeated_failures_*`). Zeitlimit 10 s, Antwort
//!   hoechstens 1 MiB, keine Umleitung, DTD abgelehnt (`hostile_or_broken_feeds_*`,
//!   `the_http_fetcher_*`).
//! - **Absturz eines Kindprozesses**: B3 startet keinen Prozess (Hashen und Abruf laufen im
//!   Prozess; Transkription und `yt-dlp` gehoeren der Import-Warteschlange bzw. A3).
//! - **Voller Arbeitsspeicher**: Hashen liest in 1-MiB-Bloecken (kein Laden der Datei),
//!   hoechstens zwei Dateien je Takt und Ablauf. Die Transkription steht hinter dem RAM-Tor der
//!   Import-Warteschlange; das Tor der Engine (`queue_gate`) laesst nichts Schweres beginnen,
//!   solange eine Aufnahme laeuft oder die Warteschlange arbeitet, danach gilt das RAM-Start-Tor
//!   von `process_guard`: bei vollem RAM WARTET der Schritt (Rueckstau), der Rechner bleibt
//!   bedienbar (`queue_gate::tests::*`).
//! - **Echtzeit-Audiopfad**: unberuehrt. Kein Code von B3 laeuft im Audio-Callback; waehrend einer
//!   Aufnahme beginnt kein schwerer Schritt, und die Import-Warteschlange haelt selbst an.
//! - **Feindliche Eingaben**: Pfade nur unter der Wurzel der Ordner-Integration (Sandbox, kein
//!   `..`, keine Verknuepfung nach aussen), Feed-Texte untrusted (Steuerzeichen, Laenge, nur
//!   `watch?v=<ID>`-Adressen aus gueltigen IDs), Kanal-Kennung streng (`UC` + 22 Zeichen).

//! Bausteine von Paket B4 (App-Bausteine): `app_actions` (KI-Notizen, Protokoll, Zusammenfassung,
//! Ablegen in einen Ordner, Vorlesen als Audiodatei, lokale Mitteilung) hinter dem Trait
//! `AppServices`; `app_services` ist dessen Umsetzung in der App (eingehaengt in `hub::start`),
//! `toast` die Windows-Mitteilung. Vorlage `eingangsordner-word` in `templates`.
//! Fehlerfaelle, Idempotenz und Rechte stehen im Kopf von `app_actions`.
//!
//! Bausteine von Paket B5 (Integrations-Bausteine): `integration_actions` (`mail.send` ueber
//! Microsoft 365 oder SMTP mit Empfaengerregeln und Anhaengen, `calendar.note` am Termin,
//! `webhook.post` an eine Webhook-Integration, n8n-Bruecke). Neu in den Schnittstellen von B1:
//! `Action::gate_view` (der Baustein bildet aus den Laufdaten die vollstaendige Ansicht, die das
//! Tor sieht und die Freigabe bindet) und `GateView::max_mode` (Obergrenze fuer das Recht, E3).
//! Im Register gibt es die Art `webhook` mit der Faehigkeit `webhook.post`. Vorlage
//! `termin-protokoll-mail` in `templates`. Fehlerfaelle und Rechte stehen im Kopf von
//! `integration_actions`.
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
pub mod import; // B3
pub mod import_app; // B3
pub mod integration_actions; // B5
pub mod jsonschema;
pub mod model;
pub mod plan;
pub mod queue_gate; // B3
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
