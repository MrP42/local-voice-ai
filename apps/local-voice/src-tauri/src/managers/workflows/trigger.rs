//! Ausloeser der Workflow-Engine (B2): Kalender „Termin beginnt/endet“, Besprechungs-
//! ereignisse, Zeitplan und manueller Start.
//!
//! Ein Ausloeser ist hier eine REINE Entscheidung ("fuer diesen Ablauf ist jetzt etwas
//! faellig") plus ein Aufruf von `RunSink::enqueue`. Die Idempotenz liegt in der Engine
//! (`UNIQUE (workflow_id, trigger_key)`): jeder Ausloeser bildet einen stabilen Schluessel
//! (`calendar_start:<termin>`, `meeting:<besprechung>:<stufe>`, `schedule:<zeitpunkt>`), und
//! derselbe Schluessel ergibt nie einen zweiten Lauf, auch nicht ueber einen Neustart
//! oder zwei Threads. Kein Ausloeser startet einen eigenen Thread oder Zeitgeber:
//! - Kalender und Zeitplan haengen am Takt von `calendar::service` (alle 15 s, derselbe
//!   Takt wie die Erinnerung `calendar::reminder`, KEIN zweiter Poller),
//! - Besprechungsereignisse sind Aufrufe aus den vorhandenen Ereignissen
//!   (`TranscriptFinal`, KI-Notizen und Protokoll fertig),
//! - manuell ist ein Aufruf der Oberflaeche.
//!
//! Neue Ablaeufe sind im Trockenlauf (B1): ein Ausloeser reiht dann einen Lauf ein, der nur
//! plant. Erst „scharf schalten“ laesst ihn wirken; die Aufnahme verlangt zusaetzlich
//! immer die Einwilligung (`recording`).

use serde_json::{Map, Value};

use super::engine::{Engine, EnqueueRequest, Enqueued};
use super::model::WorkflowDef;
use super::store::{WorkflowError, WorkflowRow};
use super::validate::{self, Issue};

pub mod calendar;
pub mod manual;
pub mod meeting_events;
pub mod schedule;

/// Das, was die Ausloeser von der Engine brauchen. Tests setzen eine Attrappe dazwischen
/// (Fehler beim Einreihen), die Anwendung die Engine selbst.
pub trait RunSink: Send + Sync {
    fn workflows(&self) -> Result<Vec<WorkflowRow>, WorkflowError>;
    fn enqueue(&self, req: &EnqueueRequest) -> Result<Enqueued, WorkflowError>;
}

impl RunSink for Engine {
    fn workflows(&self) -> Result<Vec<WorkflowRow>, WorkflowError> {
        Engine::workflows(self)
    }

    fn enqueue(&self, req: &EnqueueRequest) -> Result<Enqueued, WorkflowError> {
        Engine::enqueue(self, req)
    }
}

/// Ein eingeschalteter Ablauf mit gelesener Definition.
pub struct Armed {
    pub row: WorkflowRow,
    pub def: WorkflowDef,
}

/// Alle EINGESCHALTETEN Ablaeufe, deren Ausloeser eine der Arten `kinds` ist.
/// Unlesbare Definitionen werden uebergangen (der Lauf selbst haette sie abgelehnt) und
/// im Log genannt. Ein Fehler beim Lesen der Liste ist ein Fehler des Taktes.
pub fn enabled_with(sink: &dyn RunSink, kinds: &[&str]) -> Result<Vec<Armed>, WorkflowError> {
    let mut out = Vec::new();
    for row in sink.workflows()? {
        if !row.enabled {
            continue;
        }
        match validate::parse_definition_str(&row.definition_json) {
            Ok(def) if kinds.contains(&def.trigger.kind.as_str()) => out.push(Armed { row, def }),
            Ok(_) => {}
            Err(_) => log::warn!(
                "workflows: Definition von {} ist ungueltig, der Ausloeser wird uebergangen",
                row.id
            ),
        }
    }
    Ok(out)
}

/// Was ein Takt eines Ausloesers getan hat. Fuer Log und Tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// Neue Laeufe (Ablauf, Lauf).
    pub started: Vec<(String, String)>,
    /// Der Ausloeser war schon bekannt (zweiter Takt, Neustart): kein neuer Lauf.
    pub duplicates: usize,
    /// Faellig, aber bewusst nicht gestartet (laufende Aufnahme, ...): Grund je Eintrag.
    pub skipped: Vec<String>,
    /// Das Einreihen scheiterte (Platte voll, gesperrt, Warteschlange voll): der naechste
    /// Takt versucht es erneut, solange der Ausloeser noch gilt.
    pub errors: Vec<String>,
}

impl TickReport {
    pub fn is_empty(&self) -> bool {
        self.started.is_empty()
            && self.duplicates == 0
            && self.skipped.is_empty()
            && self.errors.is_empty()
    }

    pub(crate) fn absorb(&mut self, other: TickReport) {
        self.started.extend(other.started);
        self.duplicates += other.duplicates;
        self.skipped.extend(other.skipped);
        self.errors.extend(other.errors);
    }
}

/// Reiht einen Lauf eines Ausloesers ein und verbucht das Ergebnis im Bericht.
pub(crate) fn fire(
    sink: &dyn RunSink,
    report: &mut TickReport,
    workflow_id: &str,
    trigger_key: String,
    trigger: Value,
) {
    let req = EnqueueRequest {
        workflow_id: workflow_id.to_string(),
        trigger_key: trigger_key.clone(),
        origin: super::model::Origin::Trigger,
        trigger,
        vars: Map::new(),
        force_dry_run: false,
    };
    match sink.enqueue(&req) {
        Ok(e) if e.created => report.started.push((workflow_id.to_string(), e.run_id)),
        Ok(_) => report.duplicates += 1,
        Err(e) => {
            log::warn!(
                "workflows: Ausloeser {trigger_key} fuer {workflow_id} nicht eingereiht: {e}"
            );
            report.errors.push(format!("{workflow_id}: {e}"));
        }
    }
}

/// Zeitangabe fuer `trigger.*`: RFC 3339 in UTC, auf die Sekunde.
pub(crate) fn iso(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// Kann dieser Ausloeser Variablen erfragen? Nur der manuelle Start und der Agent geben
/// Werte mit; alle anderen laufen ohne Mitwirkung.
pub fn is_automatic(kind: &str) -> bool {
    !matches!(kind, "manual" | "agent")
}

/// Zusaetzliche Pruefung beim Speichern (ueber `validate` hinaus), ausgerufen von
/// `Engine::save_workflow`: Felder, die der Katalog nur als „irgendwas“ kennt
/// (`schedule.at`, `schedule.weekdays`), und Variablen eines automatischen Ablaufs.
pub fn check_definition(def: &WorkflowDef) -> Vec<Issue> {
    let mut out = Vec::new();
    if def.trigger.kind == "schedule" {
        for (field, message) in schedule::check(&def.trigger.params) {
            out.push(Issue {
                path: format!("/trigger/{field}"),
                message,
            });
        }
    }
    if is_automatic(&def.trigger.kind) {
        for (name, decl) in &def.variables {
            if decl.default.is_none() {
                out.push(Issue {
                    path: format!("/variables/{name}"),
                    message: format!(
                        "Die Variable „{name}“ braucht einen Vorgabewert: ein Auslöser vom Typ „{}“ kann beim Start keinen Wert erfragen.",
                        def.trigger.kind
                    ),
                });
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod test_sink {
    //! Attrappe: reicht an die Engine durch und laesst die ersten `fail_first` Aufrufe von
    //! `enqueue` scheitern (voller Datentraeger, gesperrte Datenbank).

    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    pub struct FlakySink<'a> {
        pub engine: &'a Engine,
        pub fail_first: AtomicUsize,
        pub calls: AtomicUsize,
    }

    impl<'a> FlakySink<'a> {
        pub fn new(engine: &'a Engine, fail_first: usize) -> Self {
            Self {
                engine,
                fail_first: AtomicUsize::new(fail_first),
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl RunSink for FlakySink<'_> {
        fn workflows(&self) -> Result<Vec<WorkflowRow>, WorkflowError> {
            self.engine.workflows()
        }

        fn enqueue(&self, req: &EnqueueRequest) -> Result<Enqueued, WorkflowError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self
                .fail_first
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(WorkflowError::Store(
                    "database or disk is full (simuliert)".to_string(),
                ));
            }
            self.engine.enqueue(req)
        }
    }
}
