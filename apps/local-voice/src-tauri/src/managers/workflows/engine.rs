//! Die Engine: Warteschlange, Ausfuehrung, Wiederholung, Wiederaufnahme (B1).
//!
//! Ein Lauf ist eine Zeile in `workflow_runs` (die Warteschlange) und ein Journal in
//! `workflow_run_steps`. Die Engine liest nur das Journal; im Speicher steht nichts,
//! was ein Neustart nicht aus der Datenbank wiederherstellt.
//!
//! **Ablauf eines Schritts** (`run_step`): Bedingung -> Parameter einsetzen -> (Trockenlauf:
//! planen und fertig) -> Baustein und Recht bestimmen -> Platz am `HeavyGate` -> Zeile
//! `running` ins Journal -> Tor (`integrations::gate`: Recht, Audit, Freigabe) -> Baustein
//! -> Zeile `done` mit Ausgabe -> Provenienz. Schlaegt etwas fehl, entscheidet die
//! Fehlerklasse des Bausteins (`StepError`), siehe `action`.
//!
//! **Wiederaufnahme ohne doppelte Aussenwirkung.** Jeder Arbeiter haelt den Lauf per
//! Mietvertrag (`lease_until`, vom Herzschlag verlaengert). Stirbt er, laeuft der
//! Vertrag ab, und `tick` uebernimmt: Schritte mit `done` bleiben erledigt (ihre Ausgabe
//! kommt aus dem Journal), ein Schritt im Zustand `running` ist unbestimmt:
//! - `Pure`/`Idempotent`: wird neu ausgefuehrt (`interrupted`, hoechstens
//!   [`MAX_INTERRUPTS`] Mal, danach `crash_loop`),
//! - `External`: `Action::confirm` kann die Wirkung belegen (dann gilt der Schritt als
//!   erledigt), sonst `uncertain` und Lauf `failed/effect_uncertain`. Der Nutzer
//!   entscheidet ueber `retry_run`; nie von selbst.
//!
//! Jede Schreibung des Arbeiters ist gezaeunt (`store`): hat ein anderer den Lauf
//! uebernommen, scheitert sie mit `LeaseLost`, und der alte Arbeiter hoert auf.
//! Ein Arbeiter, der an einem Fehler (Platte voll) abbricht, verlaengert seinen Vertrag
//! nicht mehr: der Lauf wird nach Ablauf wie nach einem Absturz behandelt.
//!
//! **Idempotenz.** `enqueue` ist ein `INSERT ... ON CONFLICT` auf
//! `(workflow_id, trigger_key)`: derselbe Ausloeser ergibt einen Lauf (auch ueber
//! Neustart und zwei Threads). Die Definition wird beim Einreihen in den Lauf kopiert.
//!
//! **Rechte.** Jeder Schritt mit `needs` geht durch das Tor als `Caller::Workflow`:
//! `aus` -> Schritt `denied`, Audit-Eintrag, Lauf endet `failed/denied`; `fragen` -> Freigabe
//! mit Vorschau, Lauf `awaiting_approval`; nach der Entscheidung (`tick`) laeuft der
//! Schritt mit der einmalig eingeloesten Freigabe. Neue Ablaeufe sind im Trockenlauf
//! (`workflows.dry_run`), bis der Nutzer sie scharf schaltet; ein Lauf kann nie
//! "schaerfer" angefordert werden als sein Ablauf.

use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use log::{info, warn};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Map, Value};
use ulid::Ulid;

use crate::managers::integrations::approvals;
use crate::managers::integrations::audit::{self, redact_params, sanitize_audit_text};
use crate::managers::integrations::gate::{self, GateOutcome, Request};
use crate::managers::integrations::model::{ApprovalState, AuditOutcome, Caller, NewAudit};
use crate::managers::provenance::{
    self, ActorKind, NewProvenance, ProvenanceEntry, SourceRef, SubjectKind,
};

use super::action::{
    actor_ref, output_json, Action, ActionRegistry, EffectKind, GateEnv, NeedsError, RunCtx,
    StepError, StepOutput,
};
use super::expr;
use super::heavy::{HeavyGate, LocalHeavyGate};
use super::model::{code, Origin, RunState, StepDef, StepState, WorkflowDef, MAX_BACKOFF_MS};
use super::plan;
use super::store::{
    self, RunFilter, RunRow, StepRow, StepUpdate, WorkflowError, WorkflowRow, MAX_CONTEXT_BYTES,
    MAX_TRIGGER_KEY_CHARS,
};
use super::validate::{self, Issue};

type Result<T> = std::result::Result<T, WorkflowError>;

/// Wie oft ein Schritt die App beenden darf, bevor er als Absturzschleife gilt.
pub const MAX_INTERRUPTS: usize = 3;
/// Hoechstzahl Versuche (Zeilen) eines Schritts in einem Lauf.
pub const MAX_STEP_ATTEMPTS: u32 = 200;
/// Laengste gespeicherte Eingabe eines Schritts im Journal (Bytes).
const MAX_INPUT_JOURNAL_BYTES: usize = 16 * 1024;

// ---------------------------------------------------------------------------
// Uhr, Einstellungen, Absturzpunkte
// ---------------------------------------------------------------------------

/// Die Uhr der Engine (Millisekunden UTC). Tests setzen eine feste.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Gueltigkeit des Mietvertrags eines Arbeiters.
    pub lease_ttl_ms: i64,
    /// Abstand des Herzschlags (Verlaengerung der Vertraege, Abbruchwuensche).
    pub heartbeat_ms: u64,
    /// So lange wartet der Arbeiter ohne Anlass bis zum naechsten Takt.
    pub idle_wait_ms: u64,
    /// Hoechstzahl Laeufe je Takt (Fairness gegenueber Wiederaufnahme und Freigaben).
    pub max_runs_per_tick: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            lease_ttl_ms: 60_000,
            heartbeat_ms: 15_000,
            idle_wait_ms: 2_000,
            max_runs_per_tick: 50,
        }
    }
}

/// Stellen, an denen Tests einen Absturz simulieren (nur `cfg(test)` wirkt).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashPoint {
    /// Vor dem Journal-Eintrag `running`.
    BeforeBegin,
    /// `running` steht im Journal, der Baustein ist noch nicht gelaufen.
    AfterBegin,
    /// Der Baustein ist gelaufen, das Journal kennt `done` noch nicht.
    AfterAction,
    /// `done` steht im Journal, der Lauf ist noch nicht weitergeschaltet.
    AfterFinish,
}

#[derive(Clone, Debug, Default)]
pub struct EngineStats {
    /// Wie oft `enqueue` wegen voller Warteschlange abgelehnt hat.
    pub queue_full: u64,
}

/// Meldungen der Engine an die Anwendung (B2). Die Engine kennt weder Fenster noch
/// Tauri; die Anwendung haengt hier ihr Hinweisfenster an.
pub trait EngineObserver: Send + Sync {
    /// Ein Lauf hat angehalten und wartet auf die Entscheidung des Nutzers (genau einmal
    /// je Anhalten, nicht bei jedem Takt). Aufgerufen vom Arbeiter-Thread: kurz bleiben.
    fn run_awaiting_approval(&self, run_id: &str);
}

// ---------------------------------------------------------------------------
// Oeffentliche Typen
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct EnqueueRequest {
    pub workflow_id: String,
    /// Idempotenzschluessel des Ausloesers (`calendar:<event>:<start>`, `manual:<ulid>`).
    pub trigger_key: String,
    pub origin: Origin,
    /// Daten des Ausloesers (`trigger.*`), ein JSON-Objekt.
    pub trigger: Value,
    /// Werte fuer deklarierte Variablen (manuell, Agent).
    pub vars: Map<String, Value>,
    /// Trockenlauf erzwingen (auch bei einem scharfen Ablauf).
    pub force_dry_run: bool,
}

impl EnqueueRequest {
    pub fn manual(workflow_id: &str) -> Self {
        Self {
            workflow_id: workflow_id.to_string(),
            trigger_key: format!("manual:{}", Ulid::new()),
            origin: Origin::Manual,
            trigger: json!({}),
            vars: Map::new(),
            force_dry_run: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Enqueued {
    pub run_id: String,
    /// `false`: der Ausloeser hatte schon einen Lauf; `run_id` ist dessen Kennung.
    pub created: bool,
    pub dry_run: bool,
}

/// Wie ein Lauf in diesem Durchgang endete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    Done,
    Failed {
        code: String,
    },
    Cancelled,
    /// Zurueck in die Warteschlange (Wiederholung, Warten, kein Platz).
    Parked {
        reason: String,
    },
    AwaitingApproval,
}

#[derive(Clone, Debug, Default)]
pub struct TickReport {
    pub outcomes: Vec<(String, RunOutcome)>,
    /// Laeufe, die nach Ablauf des Mietvertrags uebernommen wurden.
    pub recovered: usize,
    /// Laeufe, deren Freigabe entschieden ist und die weiterlaufen koennen.
    pub approvals_released: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CancelResult {
    /// Sofort abgebrochen (wartete oder wartete auf Freigabe).
    Cancelled,
    /// Der Lauf laeuft; er endet am naechsten Schrittwechsel.
    Requested,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunDetail {
    pub run: RunRow,
    pub steps: Vec<StepRow>,
    /// Herkunft der Ausgaben des Laufs (`run_output`, je Schritt ein Eintrag).
    pub provenance: Vec<ProvenanceEntry>,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

struct Inner {
    db_path: PathBuf,
    registry: RwLock<ActionRegistry>,
    heavy: Arc<dyn HeavyGate>,
    clock: Arc<dyn Clock>,
    owner: String,
    config: EngineConfig,
    /// Abbruch-Flags der Laeufe, die dieser Arbeiter gerade haelt.
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
    queue_full: AtomicU64,
    wake: (Mutex<bool>, Condvar),
    observer: RwLock<Option<Arc<dyn EngineObserver>>>,
    #[cfg(test)]
    crash: Mutex<Option<(String, CrashPoint)>>,
}

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

/// Haelt den Eintrag eines laufenden Laufs in `active`; raeumt beim Verlassen auf,
/// auch bei Fehler und Panik (der Herzschlag verlaengert dann nichts mehr).
struct ActiveGuard<'a> {
    engine: &'a Engine,
    run_id: String,
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.engine
            .inner
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.run_id);
    }
}

/// Alles, was der Ausfuehrer eines Laufs braucht.
struct RunScope<'a> {
    conn: &'a Connection,
    run: &'a RunRow,
    ctx: Value,
    journal: Vec<StepRow>,
    cancel: Arc<AtomicBool>,
}

/// Was nach einem Schritt mit dem Lauf geschieht.
enum StepFlow {
    Next,
    Park {
        state: RunState,
        next_run_at: Option<i64>,
        reason: String,
    },
    Stop {
        error: String,
        code: &'static str,
    },
}

fn db_err(e: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::Store(e.to_string())
}

impl Engine {
    pub fn new(
        db_path: PathBuf,
        heavy: Arc<dyn HeavyGate>,
        clock: Arc<dyn Clock>,
        config: EngineConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                db_path,
                registry: RwLock::new(ActionRegistry::with_catalog()),
                heavy,
                clock,
                owner: format!("eng-{}-{}", std::process::id(), Ulid::new()),
                config,
                active: Mutex::new(HashMap::new()),
                queue_full: AtomicU64::new(0),
                wake: (Mutex::new(false), Condvar::new()),
                observer: RwLock::new(None),
                #[cfg(test)]
                crash: Mutex::new(None),
            }),
        }
    }

    /// Mit Systemuhr, dem prozessweiten schweren Platz und den Standardwerten.
    pub fn with_defaults(db_path: PathBuf) -> Self {
        Self::new(
            db_path,
            LocalHeavyGate::shared(),
            Arc::new(SystemClock),
            EngineConfig::default(),
        )
    }

    pub fn owner(&self) -> &str {
        &self.inner.owner
    }

    pub fn config(&self) -> &EngineConfig {
        &self.inner.config
    }

    pub fn stats(&self) -> EngineStats {
        EngineStats {
            queue_full: self.inner.queue_full.load(Ordering::Relaxed),
        }
    }

    fn now(&self) -> i64 {
        self.inner.clock.now_ms()
    }

    fn conn(&self) -> Result<Connection> {
        // Zentraler Oeffnungsweg (WAL, `BUSY_TIMEOUT` 30 s), nie ein blankes `Connection::open`.
        crate::managers::meetings::store::open_connection(&self.inner.db_path).map_err(db_err)
    }

    /// Ersetzt oder ergaenzt einen Baustein (B2 bis B6, Goal C).
    pub fn register_action(&self, action: Arc<dyn Action>) {
        self.inner
            .registry
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .register(action);
    }

    /// Das Tor fuer schwere Schritte (C5: die Vorschau der Agent-Bausteine nimmt denselben
    /// Platz wie ein Lauf).
    pub fn heavy_gate(&self) -> Arc<dyn HeavyGate> {
        self.inner.heavy.clone()
    }

    pub fn registry(&self) -> ActionRegistry {
        self.inner
            .registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn action(&self, id: &str) -> Option<Arc<dyn Action>> {
        self.inner
            .registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Weckt den Arbeiter (nach einer Entscheidung des Nutzers, einem neuen Lauf).
    pub fn wake(&self) {
        let (m, cv) = &self.inner.wake;
        *m.lock().unwrap_or_else(|e| e.into_inner()) = true;
        cv.notify_all();
    }

    /// Haengt die Anwendung an (ersetzt einen frueheren Beobachter).
    pub fn set_observer(&self, observer: Arc<dyn EngineObserver>) {
        *self
            .inner
            .observer
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(observer);
    }

    fn notify_awaiting(&self, run_id: &str) {
        let observer = self
            .inner
            .observer
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(o) = observer {
            // Ein Fehler der Anwendung darf den Arbeiter nie beenden.
            if catch_unwind(AssertUnwindSafe(|| o.run_awaiting_approval(run_id))).is_err() {
                warn!(
                    "workflows: der Beobachter ist abgestuerzt (Panik), der Arbeiter laeuft weiter"
                );
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn crash_at(&self, step_id: &str, point: CrashPoint) {
        *self.inner.crash.lock().unwrap() = Some((step_id.to_string(), point));
    }

    #[cfg(test)]
    fn crash_check(&self, step_id: &str, point: CrashPoint) -> Result<()> {
        let mut slot = self.inner.crash.lock().unwrap();
        if let Some((s, p)) = slot.as_ref() {
            if s == step_id && *p == point {
                *slot = None;
                return Err(WorkflowError::SimulatedCrash(match point {
                    CrashPoint::BeforeBegin => "before_begin",
                    CrashPoint::AfterBegin => "after_begin",
                    CrashPoint::AfterAction => "after_action",
                    CrashPoint::AfterFinish => "after_finish",
                }));
            }
        }
        Ok(())
    }

    #[cfg(not(test))]
    #[inline(always)]
    fn crash_check(&self, _step_id: &str, _point: CrashPoint) -> Result<()> {
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Definitionen
    // -----------------------------------------------------------------------

    /// Prueft und speichert eine Definition. `id = None` legt einen NEUEN Ablauf an
    /// (ausgeschaltet, im Trockenlauf); sonst wird seine Definition ersetzt (eine
    /// geaenderte Definition faellt in den Trockenlauf zurueck).
    pub fn save_workflow(&self, id: Option<&str>, definition: &Value) -> Result<WorkflowRow> {
        let def = self
            .check_definition(definition)
            .map_err(WorkflowError::Invalid)?;
        let text = serde_json::to_string(&def).map_err(db_err)?;
        store::save_workflow(&self.conn()?, id, &def, &text, self.now())
    }

    /// Prueft eine Definition vollstaendig, ohne zu speichern (B7: Pruefung beim Tippen im
    /// Editor): Katalog, Bezuege, die Pruefung der echten Bausteine, Zeitplan-Felder. Gibt die
    /// Definition als Typ oder ALLE Befunde mit JSON-Zeiger.
    pub fn check_definition(
        &self,
        definition: &Value,
    ) -> std::result::Result<WorkflowDef, Vec<Issue>> {
        let def = validate::parse_definition(definition)?;
        let mut issues: Vec<Issue> = Vec::new();
        for (i, step) in def.steps.iter().enumerate() {
            if let Some(a) = self.action(&step.action) {
                if let Err(m) = a.validate(&step.params) {
                    issues.push(Issue {
                        path: format!("/steps/{i}/params"),
                        message: m,
                    });
                }
            }
        }
        // B2: Zeitplan-Felder und Variablen automatischer Ausloeser.
        issues.extend(super::trigger::check_definition(&def));
        if issues.is_empty() {
            Ok(def)
        } else {
            Err(issues)
        }
    }

    pub fn workflows(&self) -> Result<Vec<WorkflowRow>> {
        store::list_workflows(&self.conn()?)
    }

    pub fn workflow(&self, id: &str) -> Result<WorkflowRow> {
        store::get_workflow(&self.conn()?, id)?
            .ok_or_else(|| WorkflowError::NotFound(id.to_string()))
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        store::set_enabled(&self.conn()?, id, enabled, self.now())
    }

    /// Scharf schalten (`true`) oder zurueck in den Trockenlauf (`false`).
    pub fn set_armed(&self, id: &str, armed: bool) -> Result<()> {
        store::set_armed(&self.conn()?, id, armed, self.now())
    }

    pub fn delete_workflow(&self, id: &str) -> Result<()> {
        store::delete_workflow(&self.conn()?, id)
    }

    // -----------------------------------------------------------------------
    // Einreihen
    // -----------------------------------------------------------------------

    /// Reiht einen Lauf ein (idempotent ueber `trigger_key`).
    pub fn enqueue(&self, req: &EnqueueRequest) -> Result<Enqueued> {
        let key = req.trigger_key.trim();
        if key.is_empty()
            || key.chars().count() > MAX_TRIGGER_KEY_CHARS
            || key.chars().any(char::is_control)
        {
            return Err(WorkflowError::BadInput(format!(
                "Der Schlüssel des Auslösers muss 1 bis {MAX_TRIGGER_KEY_CHARS} Zeichen ohne Steuerzeichen haben."
            )));
        }
        if !req.trigger.is_object() {
            return Err(WorkflowError::BadInput(
                "Die Daten des Auslösers müssen ein JSON-Objekt sein.".to_string(),
            ));
        }
        let conn = self.conn()?;
        let wf = store::get_workflow(&conn, &req.workflow_id)?
            .ok_or_else(|| WorkflowError::NotFound(req.workflow_id.clone()))?;
        let def =
            validate::parse_definition_str(&wf.definition_json).map_err(WorkflowError::Invalid)?;
        let vars = resolve_vars(&def, &req.vars)?;
        // Nie "scharfer" als der Ablauf: ein nicht scharf geschalteter Ablauf plant nur.
        let dry_run = wf.dry_run || req.force_dry_run;
        if !wf.enabled && (req.origin == Origin::Trigger || !dry_run) {
            return Err(WorkflowError::Disabled(format!(
                "Der Ablauf „{}“ ist ausgeschaltet.",
                wf.name
            )));
        }
        let context = json!({"trigger": req.trigger, "vars": Value::Object(vars)});
        let context_json = context.to_string();
        if context_json.len() > MAX_CONTEXT_BYTES {
            return Err(WorkflowError::BadInput(format!(
                "Die Daten des Auslösers sind zu groß (höchstens {} KiB).",
                MAX_CONTEXT_BYTES / 1024
            )));
        }
        let inserted = match store::insert_run(
            &conn,
            &store::NewRun {
                workflow: &wf,
                trigger_key: key,
                origin: req.origin,
                dry_run,
                context_json: &context_json,
            },
            self.now(),
        ) {
            Ok(i) => i,
            Err(e) => {
                if matches!(e, WorkflowError::QueueFull(_)) {
                    self.inner.queue_full.fetch_add(1, Ordering::Relaxed);
                    warn!("workflows: Warteschlange voll fuer {} (abgelehnt)", wf.id);
                }
                return Err(e);
            }
        };
        if inserted.created {
            if let Err(e) = store::prune(&conn) {
                warn!("workflows: Aufbewahrung nicht bereinigt: {e}");
            }
            self.wake();
        }
        // Gab es den Lauf schon, gilt SEIN Modus (er kann vor einem Scharfschalten entstanden sein).
        let dry_run = if inserted.created {
            dry_run
        } else {
            store::get_run(&conn, &inserted.run_id)?
                .map(|r| r.dry_run)
                .unwrap_or(dry_run)
        };
        Ok(Enqueued {
            run_id: inserted.run_id,
            created: inserted.created,
            dry_run,
        })
    }

    // -----------------------------------------------------------------------
    // Abfragen und Eingriffe
    // -----------------------------------------------------------------------

    pub fn runs(&self, filter: &RunFilter, limit: i64) -> Result<Vec<RunRow>> {
        store::list_runs(&self.conn()?, filter, limit)
    }

    pub fn run_detail(&self, run_id: &str) -> Result<RunDetail> {
        let conn = self.conn()?;
        let run = store::get_run(&conn, run_id)?
            .ok_or_else(|| WorkflowError::NotFound(run_id.to_string()))?;
        let steps = store::steps_for(&conn, run_id)?;
        let provenance = provenance::list(&conn, SubjectKind::RunOutput, run_id)
            .map_err(|e| WorkflowError::Store(e.to_string()))?;
        Ok(RunDetail {
            run,
            steps,
            provenance,
        })
    }

    pub fn cancel_run(&self, run_id: &str) -> Result<CancelResult> {
        let conn = self.conn()?;
        let run = store::get_run(&conn, run_id)?
            .ok_or_else(|| WorkflowError::NotFound(run_id.to_string()))?;
        let now = self.now();
        match run.state {
            RunState::Queued | RunState::AwaitingApproval => {
                // Eine noch offene Freigabe dieses Laufs wird zurueckgezogen.
                for row in store::steps_for(&conn, run_id)? {
                    if row.state == StepState::AwaitingApproval {
                        if let Some(a) = &row.approval_id {
                            if let Err(e) = approvals::withdraw(&conn, a, now) {
                                warn!("workflows: Freigabe {a} nicht zurueckgezogen: {e}");
                            }
                        }
                    }
                }
                if store::cancel_idle(&conn, run_id, now)? {
                    return Ok(CancelResult::Cancelled);
                }
                // Zwischen Lesen und Schreiben hat ein Arbeiter ihn geholt.
                store::mark_cancel_requested(&conn, run_id, now)?;
                self.flag_cancel(run_id);
                Ok(CancelResult::Requested)
            }
            RunState::Running => {
                store::mark_cancel_requested(&conn, run_id, now)?;
                self.flag_cancel(run_id);
                Ok(CancelResult::Requested)
            }
            _ => Err(WorkflowError::BadInput(
                "Der Lauf ist schon beendet.".to_string(),
            )),
        }
    }

    fn flag_cancel(&self, run_id: &str) {
        if let Some(flag) = self
            .inner
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(run_id)
        {
            flag.store(true, Ordering::Release);
        }
    }

    /// Wiederholt einen gescheiterten Lauf ab dem gescheiterten Schritt (derselbe Lauf,
    /// derselbe Ausloeser-Schluessel). Ein Lauf, der wegen unklarer Aussenwirkung oder
    /// einer Absturzschleife endete, braucht `accept_uncertain = true`: der Nutzer
    /// bestaetigt, dass er die moegliche Doppelung (z. B. eine zweite Mail) in Kauf nimmt.
    pub fn retry_run(&self, run_id: &str, accept_uncertain: bool) -> Result<()> {
        let conn = self.conn()?;
        let run = store::get_run(&conn, run_id)?
            .ok_or_else(|| WorkflowError::NotFound(run_id.to_string()))?;
        if run.state != RunState::Failed {
            return Err(WorkflowError::BadInput(
                "Nur gescheiterte Läufe lassen sich wiederholen.".to_string(),
            ));
        }
        let needs_confirmation = matches!(
            run.error_code.as_deref(),
            Some(code::EFFECT_UNCERTAIN) | Some(code::CRASH_LOOP)
        );
        if needs_confirmation && !accept_uncertain {
            return Err(WorkflowError::BadInput(
                "Es ist unklar, ob der Schritt schon Wirkung hatte (z. B. eine Mail ging schon hinaus). Eine Wiederholung kann sie doppeln: bitte ausdrücklich bestätigen."
                    .to_string(),
            ));
        }
        if !store::requeue(&conn, run_id, &[RunState::Failed], None, self.now())? {
            return Err(WorkflowError::BadInput(
                "Der Lauf ließ sich nicht wiederholen (Zustand hat sich geändert).".to_string(),
            ));
        }
        self.wake();
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Takt
    // -----------------------------------------------------------------------

    /// Ein Durchgang: abgelaufene Mietvertraege uebernehmen, entschiedene Freigaben
    /// freigeben, faellige Laeufe nacheinander ausfuehren (hoechstens
    /// `max_runs_per_tick`). Fehler eines Laufs stehen im Bericht und stoppen den Takt.
    pub fn tick(&self) -> Result<TickReport> {
        let conn = self.conn()?;
        let mut report = TickReport::default();
        let now = self.now();
        match store::list_expired(&conn, now) {
            Ok(expired) => {
                for run in expired {
                    // Ein Lauf, den dieser Arbeiter selbst gerade ausfuehrt, ist nicht
                    // abgestuerzt, nur sein Herzschlag fehlt (Aufrufer ohne `spawn`):
                    // er wird nicht unter ihm weggenommen.
                    if self
                        .inner
                        .active
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .contains_key(&run.id)
                    {
                        continue;
                    }
                    match self.recover_run(&conn, &run, now) {
                        Ok(true) => report.recovered += 1,
                        Ok(false) => {}
                        Err(e) => report
                            .errors
                            .push(format!("Wiederaufnahme {}: {e}", run.id)),
                    }
                }
            }
            Err(e) => report.errors.push(format!("Wiederaufnahme: {e}")),
        }
        match self.poll_approvals(&conn, now) {
            Ok(n) => report.approvals_released = n,
            Err(e) => report.errors.push(format!("Freigaben: {e}")),
        }
        for _ in 0..self.inner.config.max_runs_per_tick {
            match self.run_next() {
                Ok(Some((id, outcome))) => {
                    if outcome == RunOutcome::AwaitingApproval {
                        self.notify_awaiting(&id);
                    }
                    report.outcomes.push((id, outcome));
                }
                Ok(None) => break,
                Err(WorkflowError::LeaseLost) => continue,
                Err(e) => {
                    report.errors.push(e.to_string());
                    break;
                }
            }
        }
        Ok(report)
    }

    /// Holt den naechsten faelligen Lauf und fuehrt ihn aus.
    pub fn run_next(&self) -> Result<Option<(String, RunOutcome)>> {
        let conn = self.conn()?;
        let Some(run) = store::claim_next(
            &conn,
            &self.inner.owner,
            self.now(),
            self.inner.config.lease_ttl_ms,
        )?
        else {
            return Ok(None);
        };
        let id = run.id.clone();
        let outcome = self.execute_run(&conn, run)?;
        Ok(Some((id, outcome)))
    }

    /// Verlaengert die Mietvertraege der Laeufe dieses Arbeiters und liest
    /// Abbruchwuensche. Vom Herzschlag-Thread gerufen.
    pub fn heartbeat(&self) -> Result<()> {
        let conn = self.conn()?;
        let ids: Vec<String> = self
            .inner
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        store::renew_leases(
            &conn,
            &self.inner.owner,
            &ids,
            self.now(),
            self.inner.config.lease_ttl_ms,
        )?;
        for id in ids {
            if store::cancel_requested(&conn, &id)? {
                self.flag_cancel(&id);
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Freigaben
    // -----------------------------------------------------------------------

    /// Gibt Laeufe frei, deren Freigabe entschieden ist (genehmigt, verweigert,
    /// verfallen); der Ausfuehrer wertet die Entscheidung aus.
    fn poll_approvals(&self, conn: &Connection, now: i64) -> Result<usize> {
        if let Err(e) = approvals::expire_stale(conn, now) {
            warn!("workflows: Freigaben nicht bereinigt: {e}");
        }
        let mut released = 0;
        for run in store::list_awaiting(conn)? {
            let rows = store::steps_for(conn, &run.id)?;
            let aid = rows
                .iter()
                .rev()
                .find(|r| r.state == StepState::AwaitingApproval)
                .and_then(|r| r.approval_id.clone());
            let decided = match aid {
                None => true,
                Some(a) => match approvals::get(conn, &a).map_err(db_err)? {
                    Some(ap) => ap.state != ApprovalState::Pending,
                    None => true,
                },
            };
            if decided && store::requeue(conn, &run.id, &[RunState::AwaitingApproval], None, now)? {
                released += 1;
            }
        }
        Ok(released)
    }

    // -----------------------------------------------------------------------
    // Wiederaufnahme
    // -----------------------------------------------------------------------

    /// Uebernimmt einen Lauf, dessen Mietvertrag abgelaufen ist (siehe Moduldoku).
    /// `Ok(false)`: hat zwischenzeitlich ein anderer getan.
    fn recover_run(&self, conn: &Connection, run: &RunRow, now: i64) -> Result<bool> {
        let rows = store::steps_for(conn, &run.id)?;
        let latest: Vec<&StepRow> = store::latest_per_step(&rows);
        let inflight = latest
            .iter()
            .find(|r| r.state == StepState::Running)
            .copied();
        let Some(row) = inflight else {
            return store::recover_requeue(conn, &run.id, now);
        };
        let def = validate::parse_definition_str(&run.definition_json).ok();
        let step = def
            .as_ref()
            .and_then(|d| d.steps.iter().find(|s| s.id == row.step_id));
        let action = self.action(&row.action);
        // Unbekannter Baustein oder unlesbare Definition: wie External behandeln.
        let effect = action
            .as_ref()
            .map(|a| a.effect())
            .unwrap_or(EffectKind::External);
        let interrupts = rows
            .iter()
            .filter(|r| r.step_id == row.step_id && r.state == StepState::Interrupted)
            .count();
        let note = "Die App endete mitten in diesem Schritt.";
        match effect {
            EffectKind::Pure | EffectKind::Idempotent if interrupts + 1 < MAX_INTERRUPTS => {
                store::recover_step_and_requeue(
                    conn,
                    &run.id,
                    row,
                    StepState::Interrupted,
                    Some("interrupted"),
                    None,
                    Some(&format!("{note} Er wird neu ausgeführt.")),
                    now,
                )
            }
            EffectKind::Pure | EffectKind::Idempotent => store::recover_step_and_fail(
                conn,
                &run.id,
                row,
                StepState::Failed,
                Some("permanent"),
                &format!(
                    "{note} Er hat die App {MAX_INTERRUPTS} Mal beendet und wird nicht mehr automatisch wiederholt."
                ),
                code::CRASH_LOOP,
                now,
            ),
            EffectKind::External => {
                // Kann der Baustein belegen, dass die Wirkung eingetreten ist?
                let confirmed = match (&def, step, &action) {
                    (Some(def), Some(step), Some(action)) => {
                        self.confirm_effect(conn, run, def, step, row, action.as_ref())
                    }
                    _ => None,
                };
                match confirmed {
                    Some(out) => {
                        let text = output_json(&out).unwrap_or_else(|_| "{}".to_string());
                        store::recover_step_and_requeue(
                            conn,
                            &run.id,
                            row,
                            StepState::Done,
                            None,
                            Some(&text),
                            Some("Wirkung nach dem Neustart bestätigt."),
                            now,
                        )
                    }
                    None => store::recover_step_and_fail(
                        conn,
                        &run.id,
                        row,
                        StepState::Uncertain,
                        Some("unknown"),
                        &format!(
                            "{note} Es ist unklar, ob die Wirkung (z. B. eine Mail) schon eingetreten ist; der Schritt wird nicht von selbst wiederholt."
                        ),
                        code::EFFECT_UNCERTAIN,
                        now,
                    ),
                }
            }
        }
    }

    fn confirm_effect(
        &self,
        conn: &Connection,
        run: &RunRow,
        def: &WorkflowDef,
        step: &StepDef,
        row: &StepRow,
        action: &dyn Action,
    ) -> Option<StepOutput> {
        let rows = store::steps_for(conn, &run.id).ok()?;
        let ctx = build_ctx(run, def, &rows);
        let params = expr::render_value(&Value::Object(step.params.clone()), &ctx).ok()?;
        let cancel = AtomicBool::new(false);
        let rctx = RunCtx {
            workflow_id: &run.workflow_id,
            run_id: &run.id,
            step_id: &step.id,
            attempt: row.attempt,
            idempotency_key: format!("{}:{}", run.id, step.id),
            context: &ctx,
            step_started_at: row.started_at.unwrap_or(0),
            approved: false,
            gate_args: None,
            cancel: &cancel,
            clock: &*self.inner.clock,
            db_path: &self.inner.db_path,
        };
        catch_unwind(AssertUnwindSafe(|| action.confirm(&rctx, &params)))
            .ok()
            .flatten()
    }

    // -----------------------------------------------------------------------
    // Ausfuehrung eines Laufs
    // -----------------------------------------------------------------------

    fn execute_run(&self, conn: &Connection, run: RunRow) -> Result<RunOutcome> {
        let owner = self.inner.owner.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.inner
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(run.id.clone(), cancel.clone());
        let _guard = ActiveGuard {
            engine: self,
            run_id: run.id.clone(),
        };

        let def = match validate::parse_definition_str(&run.definition_json) {
            Ok(d) => d,
            Err(issues) => {
                let msg = issues
                    .first()
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "unbekannt".to_string());
                return self.finish(
                    conn,
                    &run,
                    RunState::Failed,
                    Some(&format!("Die gespeicherte Definition ist ungültig: {msg}")),
                    Some(code::INVALID_DEFINITION),
                );
            }
        };
        let journal = store::steps_for(conn, &run.id)?;
        let ctx = build_ctx(&run, &def, &journal);
        let mut sc = RunScope {
            conn,
            run: &run,
            ctx,
            journal,
            cancel: cancel.clone(),
        };

        for (index, step) in def.steps.iter().enumerate() {
            let latest = latest_row(&sc.journal, &step.id).cloned();
            if let Some(row) = &latest {
                if is_complete(row, step) {
                    continue;
                }
            }
            if cancel.load(Ordering::Acquire) || store::cancel_requested(conn, &run.id)? {
                return self.finish(
                    conn,
                    &run,
                    RunState::Cancelled,
                    Some("Vom Nutzer abgebrochen."),
                    None,
                );
            }
            // Vertrag verlaengern, bevor ein moeglicherweise langer Schritt beginnt.
            store::renew_leases(
                conn,
                &owner,
                std::slice::from_ref(&run.id),
                self.now(),
                self.inner.config.lease_ttl_ms,
            )?;
            match self.run_step(&mut sc, index, step, latest)? {
                StepFlow::Next => {}
                StepFlow::Park {
                    state,
                    next_run_at,
                    reason,
                } => {
                    store::park_run(
                        conn,
                        &run.id,
                        &owner,
                        state,
                        next_run_at,
                        Some(&reason),
                        self.now(),
                    )?;
                    return Ok(if state == RunState::AwaitingApproval {
                        RunOutcome::AwaitingApproval
                    } else {
                        RunOutcome::Parked { reason }
                    });
                }
                StepFlow::Stop { error, code } => {
                    return self.finish(conn, &run, RunState::Failed, Some(&error), Some(code));
                }
            }
        }
        self.finish(conn, &run, RunState::Done, None, None)
    }

    fn finish(
        &self,
        conn: &Connection,
        run: &RunRow,
        state: RunState,
        error: Option<&str>,
        error_code: Option<&str>,
    ) -> Result<RunOutcome> {
        let clean = error.map(|e| sanitize_audit_text(e, 500));
        store::finish_run(
            conn,
            &run.id,
            &self.inner.owner,
            state,
            clean.as_deref(),
            error_code,
            self.now(),
        )?;
        if let Err(e) = store::prune(conn) {
            warn!("workflows: Aufbewahrung nicht bereinigt: {e}");
        }
        Ok(match state {
            RunState::Done => RunOutcome::Done,
            RunState::Cancelled => RunOutcome::Cancelled,
            _ => RunOutcome::Failed {
                code: error_code.unwrap_or(code::PERMANENT).to_string(),
            },
        })
    }

    // -----------------------------------------------------------------------
    // Ein Schritt
    // -----------------------------------------------------------------------

    fn run_step(
        &self,
        sc: &mut RunScope<'_>,
        index: usize,
        step: &StepDef,
        latest: Option<StepRow>,
    ) -> Result<StepFlow> {
        let conn = sc.conn;
        let run = sc.run;
        let owner = self.inner.owner.as_str();
        let ordinal = index as u32;
        let now = self.now();
        let attempt_after = |l: &Option<StepRow>| l.as_ref().map(|r| r.attempt + 1).unwrap_or(1);

        // ---- Trockenlauf: nur planen -----------------------------------------
        if run.dry_run {
            let planned = plan::plan_step(conn, &self.registry(), step, index, &sc.ctx);
            let skipped = planned["status"].as_str() == Some("skipped");
            let state = if skipped {
                StepState::Skipped
            } else {
                StepState::Planned
            };
            let mut row = new_row(&run.id, step, ordinal, attempt_after(&latest), state);
            row.output_json = Some(planned.to_string());
            row.started_at = Some(now);
            row.ended_at = Some(now);
            store::insert_step(conn, owner, &row)?;
            set_result(
                &mut sc.ctx,
                &step.id,
                state_word(state),
                !skipped,
                None,
                None,
            );
            return Ok(StepFlow::Next);
        }

        // ---- Bedingung ---------------------------------------------------------
        if let Some(when) = &step.when {
            match expr::parse_condition(when).and_then(|e| e.eval_bool(&sc.ctx)) {
                Ok(true) => {}
                Ok(false) => {
                    let mut row = new_row(
                        &run.id,
                        step,
                        ordinal,
                        attempt_after(&latest),
                        StepState::Skipped,
                    );
                    row.started_at = Some(now);
                    row.ended_at = Some(now);
                    store::insert_step(conn, owner, &row)?;
                    set_result(&mut sc.ctx, &step.id, "skipped", false, None, None);
                    return Ok(StepFlow::Next);
                }
                Err(e) => {
                    return self.fail_step(
                        sc,
                        step,
                        ordinal,
                        attempt_after(&latest),
                        false,
                        StepState::Failed,
                        "permanent",
                        &format!("Bedingung: {e}"),
                        code::PERMANENT,
                        None,
                    );
                }
            }
        }

        // ---- Parameter einsetzen ----------------------------------------------
        let params = match expr::render_value(&Value::Object(step.params.clone()), &sc.ctx) {
            Ok(p) => p,
            Err(e) => {
                return self.fail_step(
                    sc,
                    step,
                    ordinal,
                    attempt_after(&latest),
                    false,
                    StepState::Failed,
                    "permanent",
                    &format!("Parameter: {e}"),
                    code::PERMANENT,
                    None,
                );
            }
        };
        let input_json = journal_input(&params);

        // ---- Baustein und Recht ------------------------------------------------
        let Some(action) = self.action(&step.action) else {
            return self.fail_step(
                sc,
                step,
                ordinal,
                attempt_after(&latest),
                false,
                StepState::Failed,
                "permanent",
                &format!("Der Baustein „{}“ ist nicht eingebaut.", step.action),
                code::NOT_AVAILABLE,
                Some(input_json),
            );
        };
        let needs = match action.needs(&params) {
            Ok(n) => n,
            Err(NeedsError::Invalid(m)) => {
                return self.fail_step(
                    sc,
                    step,
                    ordinal,
                    attempt_after(&latest),
                    false,
                    StepState::Failed,
                    "permanent",
                    &m,
                    code::PERMANENT,
                    Some(input_json),
                );
            }
            Err(NeedsError::Unmodeled(m)) => {
                // Fail closed, und auch das gehoert ins Audit.
                let entry = NewAudit {
                    caller: Caller::Workflow.as_str().to_string(),
                    integration_id: None,
                    capability: None,
                    target: Some(step.action.clone()),
                    outcome: AuditOutcome::Denied,
                    detail: Some(json!({
                        "reason": "capability_not_modeled",
                        "workflow": run.workflow_id,
                        "run": run.id,
                        "step": step.id
                    })),
                };
                if let Err(e) = audit::record_at(conn, &entry, now) {
                    warn!("workflows: Audit der Ablehnung nicht geschrieben: {e}");
                }
                return self.fail_step(
                    sc,
                    step,
                    ordinal,
                    attempt_after(&latest),
                    false,
                    StepState::Denied,
                    "denied",
                    &m,
                    code::DENIED,
                    Some(input_json),
                );
            }
        };

        // ---- Schwerer Schritt: Platz und Arbeitsspeicher ------------------------
        let _permit = match action.heavy(&params) {
            Some(need) => match self.inner.heavy.try_enter(&need) {
                Ok(p) => Some(p),
                Err(wait) => {
                    return Ok(StepFlow::Park {
                        state: RunState::Queued,
                        next_run_at: Some(now.saturating_add(wait.retry_after_ms as i64)),
                        reason: wait.reason.code().to_string(),
                    });
                }
            },
            None => None,
        };

        // ---- Journal: der Schritt beginnt ---------------------------------------
        self.crash_check(&step.id, CrashPoint::BeforeBegin)?;
        let reuse = matches!(
            &latest,
            Some(r) if matches!(r.state, StepState::Waiting | StepState::AwaitingApproval)
        );
        let attempt = match &latest {
            Some(r) if reuse => r.attempt,
            other => attempt_after(other),
        };
        if attempt > MAX_STEP_ATTEMPTS {
            return self.fail_step(
                sc,
                step,
                ordinal,
                attempt,
                false,
                StepState::Failed,
                "permanent",
                "Zu viele Versuche für diesen Schritt.",
                code::PERMANENT,
                Some(input_json),
            );
        }
        let awaiting = latest
            .as_ref()
            .filter(|r| r.state == StepState::AwaitingApproval)
            .and_then(|r| r.approval_id.clone());
        let step_started_at = if reuse {
            latest.as_ref().and_then(|r| r.started_at).unwrap_or(now)
        } else {
            now
        };
        if reuse {
            store::reopen_step(conn, owner, &run.id, &step.id, attempt)?;
        } else {
            let mut row = new_row(&run.id, step, ordinal, attempt, StepState::Running);
            row.input_json = Some(input_json.clone());
            row.started_at = Some(now);
            store::insert_step(conn, owner, &row)?;
        }
        self.crash_check(&step.id, CrashPoint::AfterBegin)?;

        // ---- Tor und Baustein ----------------------------------------------------
        // Die Freigabe gilt fuer genau DIESEN Schritt dieses Laufs und steht so in der
        // Vorschau (`lauf`): Gleiche Parameter in zwei Laeufen teilten sonst EINE
        // Freigabe (das Register fasst gleiche offene Anfragen zusammen), und der
        // zweite Lauf scheiterte an der schon eingeloesten. Die Bindung ist stabil:
        // Lauf und Schritt aendern sich beim Wiederaufnehmen nicht.
        let policy = step.retry_policy();
        // Hangt die Wirkung von Daten des Laufs ab (Empfaenger einer Mail), bildet der Baustein
        // die vollstaendige Ansicht fuer das Tor: SIE bindet die Freigabe und steht in der Vorschau.
        let view = match &needs {
            Some(_) => match action.gate_view(
                &GateEnv {
                    conn,
                    context: &sc.ctx,
                    planning: false,
                },
                &params,
            ) {
                Ok(v) => v,
                Err(e) => {
                    return self.handle_error(
                        sc,
                        step,
                        ordinal,
                        attempt,
                        action.effect(),
                        e,
                        &policy,
                    );
                }
            },
            None => None,
        };
        let gate_target: Option<String> = view
            .as_ref()
            .and_then(|v| v.target.clone())
            .or_else(|| needs.as_ref().and_then(|n| n.target.clone()));
        let gate_cap = view.as_ref().and_then(|v| v.max_mode);
        let mut gate_args = view
            .as_ref()
            .map(|v| v.args.clone())
            .unwrap_or_else(|| params.clone());
        if let Some(obj) = gate_args.as_object_mut() {
            obj.insert("lauf".to_string(), json!(format!("{}/{}", run.id, step.id)));
        }
        let stash: RefCell<Option<std::result::Result<StepOutput, StepError>>> = RefCell::new(None);
        enum Gated {
            Outcome(
                Box<
                    std::result::Result<
                        GateOutcome<()>,
                        crate::managers::integrations::model::IntegrationError,
                    >,
                >,
            ),
            StillPending,
        }
        let gated = {
            let rctx = RunCtx {
                workflow_id: &run.workflow_id,
                run_id: &run.id,
                step_id: &step.id,
                attempt,
                idempotency_key: format!("{}:{}", run.id, step.id),
                context: &sc.ctx,
                step_started_at,
                approved: awaiting.is_some(),
                gate_args: Some(&gate_args),
                cancel: &sc.cancel,
                clock: &*self.inner.clock,
                db_path: &self.inner.db_path,
            };
            let work = || -> std::result::Result<(), String> {
                let result = catch_unwind(AssertUnwindSafe(|| action.run(&rctx, &params)))
                    .unwrap_or_else(|p| {
                        Err(StepError::Unknown(format!(
                            "Der Baustein ist abgestürzt: {}",
                            panic_text(&p)
                        )))
                    });
                match result {
                    Ok(out) => {
                        *stash.borrow_mut() = Some(Ok(out));
                        Ok(())
                    }
                    // Verschieben ist kein Fehler (kein `error` im Audit).
                    Err(e @ StepError::Defer { .. }) => {
                        *stash.borrow_mut() = Some(Err(e));
                        Ok(())
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        *stash.borrow_mut() = Some(Err(e));
                        Err(msg)
                    }
                }
            };
            match &needs {
                None => Gated::Outcome(Box::new(Ok(match work() {
                    Ok(()) => GateOutcome::Done(()),
                    Err(m) => GateOutcome::Failed(m),
                }))),
                Some(n) => {
                    let req = Request {
                        caller: Caller::Workflow,
                        integration_id: &n.integration_id,
                        capability: n.capability,
                        target: gate_target.as_deref(),
                        args: Some(&gate_args),
                        tool_mode: gate_cap,
                    };
                    match &awaiting {
                        None => Gated::Outcome(Box::new(gate::run(conn, &req, now, work))),
                        Some(aid) => {
                            let state = approvals::get(conn, aid).map_err(db_err)?.map(|a| a.state);
                            match state {
                                Some(ApprovalState::Approved) => Gated::Outcome(Box::new(
                                    gate::run_approved(conn, aid, &req, now, work),
                                )),
                                Some(ApprovalState::Pending) => Gated::StillPending,
                                Some(ApprovalState::Denied) => {
                                    Gated::Outcome(Box::new(Ok(GateOutcome::Denied {
                                        reason: "Die Freigabe wurde verweigert.".to_string(),
                                        code: "approval_denied",
                                    })))
                                }
                                Some(ApprovalState::Expired) => {
                                    Gated::Outcome(Box::new(Ok(GateOutcome::Denied {
                                        reason: "Die Freigabe ist abgelaufen.".to_string(),
                                        code: "approval_expired",
                                    })))
                                }
                                Some(ApprovalState::Used) => {
                                    Gated::Outcome(Box::new(Ok(GateOutcome::Denied {
                                        reason: "Die Freigabe wurde schon eingelöst.".to_string(),
                                        code: "approval_used",
                                    })))
                                }
                                None => Gated::Outcome(Box::new(Ok(GateOutcome::Denied {
                                    reason: "Die Freigabe gibt es nicht mehr.".to_string(),
                                    code: "approval_not_found",
                                }))),
                            }
                        }
                    }
                }
            }
        };
        let result = stash.into_inner();

        match gated {
            Gated::StillPending => {
                // Zurueck auf `awaiting_approval` (die Zeile wurde oben geoeffnet).
                store::update_step(
                    conn,
                    owner,
                    &StepUpdate {
                        run_id: &run.id,
                        step_id: &step.id,
                        attempt,
                        state: StepState::AwaitingApproval,
                        error_class: None,
                        output_json: None,
                        error: None,
                        approval_id: None,
                        wake_at: None,
                        ended_at: None,
                    },
                )?;
                Ok(StepFlow::Park {
                    state: RunState::AwaitingApproval,
                    next_run_at: None,
                    reason: "approval".to_string(),
                })
            }
            Gated::Outcome(outcome) => match *outcome {
                Err(e) => {
                    // Das Tor konnte nicht entscheiden (Audit/Freigabe nicht schreibbar):
                    // fail closed, es ist NICHTS passiert -> voruebergehend.
                    self.handle_error(
                        sc,
                        step,
                        ordinal,
                        attempt,
                        action.effect(),
                        StepError::Transient(format!("Das Register ist nicht erreichbar: {e}")),
                        &policy,
                    )
                }
                Ok(GateOutcome::Done(())) => match result {
                    Some(Ok(out)) => {
                        self.complete_step(sc, step, ordinal, attempt, step_started_at, out)
                    }
                    Some(Err(e)) => {
                        self.handle_error(sc, step, ordinal, attempt, action.effect(), e, &policy)
                    }
                    None => self.handle_error(
                        sc,
                        step,
                        ordinal,
                        attempt,
                        action.effect(),
                        StepError::Permanent(
                            "Der Baustein hat kein Ergebnis geliefert.".to_string(),
                        ),
                        &policy,
                    ),
                },
                Ok(GateOutcome::Failed(msg)) => {
                    let e = match result {
                        Some(Err(e)) => e,
                        _ => StepError::Permanent(msg),
                    };
                    self.handle_error(sc, step, ordinal, attempt, action.effect(), e, &policy)
                }
                Ok(GateOutcome::Denied { reason, code: c }) => {
                    let run_code = if c == "approval_expired" {
                        code::APPROVAL_EXPIRED
                    } else {
                        code::DENIED
                    };
                    self.fail_step(
                        sc,
                        step,
                        ordinal,
                        attempt,
                        true,
                        StepState::Denied,
                        "denied",
                        &reason,
                        run_code,
                        None,
                    )
                }
                Ok(GateOutcome::Pending { approval_id }) => {
                    store::update_step(
                        conn,
                        owner,
                        &StepUpdate {
                            run_id: &run.id,
                            step_id: &step.id,
                            attempt,
                            state: StepState::AwaitingApproval,
                            error_class: None,
                            output_json: None,
                            error: None,
                            approval_id: Some(&approval_id),
                            wake_at: None,
                            ended_at: None,
                        },
                    )?;
                    Ok(StepFlow::Park {
                        state: RunState::AwaitingApproval,
                        next_run_at: None,
                        reason: "approval".to_string(),
                    })
                }
            },
        }
    }

    /// Der Baustein hat geliefert: Ausgabe ins Journal, Ergebnis in den Kontext,
    /// Provenienz.
    fn complete_step(
        &self,
        sc: &mut RunScope<'_>,
        step: &StepDef,
        ordinal: u32,
        attempt: u32,
        started_at: i64,
        out: StepOutput,
    ) -> Result<StepFlow> {
        let conn = sc.conn;
        let run = sc.run;
        let owner = self.inner.owner.as_str();
        let text = match output_json(&out) {
            Ok(t) => t,
            Err(m) => {
                return self.fail_step(
                    sc,
                    step,
                    ordinal,
                    attempt,
                    true,
                    StepState::Failed,
                    "permanent",
                    &m,
                    code::PERMANENT,
                    None,
                );
            }
        };
        self.crash_check(&step.id, CrashPoint::AfterAction)?;
        let now = self.now();
        store::update_step(
            conn,
            owner,
            &StepUpdate {
                run_id: &run.id,
                step_id: &step.id,
                attempt,
                state: StepState::Done,
                error_class: None,
                output_json: Some(&text),
                error: None,
                approval_id: None,
                wake_at: None,
                ended_at: Some(now),
            },
        )?;
        self.crash_check(&step.id, CrashPoint::AfterFinish)?;
        let stored = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({}));
        set_result(&mut sc.ctx, &step.id, "done", true, None, Some(&stored));
        self.record_provenance(conn, run, step, ordinal, attempt, started_at, &out, now);
        Ok(StepFlow::Next)
    }

    #[allow(clippy::too_many_arguments)]
    fn record_provenance(
        &self,
        conn: &Connection,
        run: &RunRow,
        step: &StepDef,
        ordinal: u32,
        attempt: u32,
        started_at: i64,
        out: &StepOutput,
        now: i64,
    ) {
        let operation: String = step
            .action
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .take(48)
            .collect();
        let mut entry = NewProvenance::new(
            SubjectKind::RunOutput,
            &run.id,
            &operation,
            ActorKind::Workflow,
        );
        entry.subject_revision = Some(i64::from(ordinal) + 1);
        entry.actor_ref = Some(actor_ref(&run.workflow_id, &run.id, &step.id));
        entry.duration_ms = Some(now.saturating_sub(started_at).max(0) as u64);
        entry.confidence = out.confidence;
        entry.sources = out.sources.clone();
        if entry.sources.len() < provenance::MAX_SOURCES {
            entry
                .sources
                .push(SourceRef::new("trigger", &run.trigger_key, None));
        }
        entry.params = Some(json!({
            "workflow": run.workflow_id,
            "workflow_name": run.workflow_name,
            "step": step.id,
            "action": step.action,
            "attempt": attempt,
            "origin": run.origin.as_str(),
        }));
        // Eine fehlende Herkunft darf den Schritt nie scheitern lassen.
        if let Err(e) = provenance::record_at(conn, &entry, now) {
            warn!(
                "workflows: Provenienz fuer {}/{} nicht geschrieben: {e}",
                run.id, step.id
            );
        }
    }

    /// Fehlerklassen -> Journal und Lauf (siehe `action`).
    #[allow(clippy::too_many_arguments)]
    fn handle_error(
        &self,
        sc: &mut RunScope<'_>,
        step: &StepDef,
        ordinal: u32,
        attempt: u32,
        effect: EffectKind,
        err: StepError,
        policy: &super::model::RetryPolicy,
    ) -> Result<StepFlow> {
        let conn = sc.conn;
        let run = sc.run;
        let owner = self.inner.owner.as_str();
        let now = self.now();
        match err {
            StepError::Unknown(m) if effect == EffectKind::External => self.fail_step(
                sc,
                step,
                ordinal,
                attempt,
                true,
                StepState::Uncertain,
                "unknown",
                &format!("Unklar, ob die Wirkung eingetreten ist: {m}"),
                code::EFFECT_UNCERTAIN,
                None,
            ),
            StepError::Transient(m) | StepError::Unknown(m) => {
                let so_far = transient_failures(&sc.journal, &step.id) as u32 + 1;
                if so_far < policy.max_attempts {
                    let delay = policy.delay_before(so_far + 1).min(MAX_BACKOFF_MS);
                    let clean = sanitize_audit_text(&m, 300);
                    store::update_step(
                        conn,
                        owner,
                        &StepUpdate {
                            run_id: &run.id,
                            step_id: &step.id,
                            attempt,
                            state: StepState::Retrying,
                            error_class: Some("transient"),
                            output_json: None,
                            error: Some(&clean),
                            approval_id: None,
                            wake_at: Some(now + delay as i64),
                            ended_at: Some(now),
                        },
                    )?;
                    Ok(StepFlow::Park {
                        state: RunState::Queued,
                        next_run_at: Some(now + delay as i64),
                        reason: "retry".to_string(),
                    })
                } else {
                    self.fail_step(
                        sc,
                        step,
                        ordinal,
                        attempt,
                        true,
                        StepState::Failed,
                        "transient",
                        &format!("Aufgegeben nach {so_far} Versuchen: {m}"),
                        code::RETRIES_EXHAUSTED,
                        None,
                    )
                }
            }
            StepError::Permanent(m) => self.fail_step(
                sc,
                step,
                ordinal,
                attempt,
                true,
                StepState::Failed,
                "permanent",
                &m,
                code::PERMANENT,
                None,
            ),
            StepError::NotAvailable(m) => self.fail_step(
                sc,
                step,
                ordinal,
                attempt,
                true,
                StepState::Failed,
                "permanent",
                &m,
                code::NOT_AVAILABLE,
                None,
            ),
            StepError::Denied(m) => self.fail_step(
                sc,
                step,
                ordinal,
                attempt,
                true,
                StepState::Denied,
                "denied",
                &m,
                code::DENIED,
                None,
            ),
            StepError::Defer {
                retry_after_ms,
                reason,
            } => {
                let wake = now.saturating_add(retry_after_ms as i64);
                let clean = sanitize_audit_text(&reason, 300);
                store::update_step(
                    conn,
                    owner,
                    &StepUpdate {
                        run_id: &run.id,
                        step_id: &step.id,
                        attempt,
                        state: StepState::Waiting,
                        error_class: None,
                        output_json: None,
                        error: Some(&clean),
                        approval_id: None,
                        wake_at: Some(wake),
                        ended_at: None,
                    },
                )?;
                Ok(StepFlow::Park {
                    state: RunState::Queued,
                    next_run_at: Some(wake),
                    reason: "defer".to_string(),
                })
            }
        }
    }

    /// Schreibt den endgueltigen Misserfolg eines Schritts. Mit `on_error: continue`
    /// geht der Lauf weiter (ausser bei unklarer Aussenwirkung: die haelt ihn immer an).
    #[allow(clippy::too_many_arguments)]
    fn fail_step(
        &self,
        sc: &mut RunScope<'_>,
        step: &StepDef,
        ordinal: u32,
        attempt: u32,
        row_exists: bool,
        state: StepState,
        class: &str,
        message: &str,
        run_code: &'static str,
        input_json: Option<String>,
    ) -> Result<StepFlow> {
        let conn = sc.conn;
        let run = sc.run;
        let owner = self.inner.owner.as_str();
        let now = self.now();
        let clean = sanitize_audit_text(message, 500);
        if row_exists {
            store::update_step(
                conn,
                owner,
                &StepUpdate {
                    run_id: &run.id,
                    step_id: &step.id,
                    attempt,
                    state,
                    error_class: Some(class),
                    output_json: None,
                    error: Some(&clean),
                    approval_id: None,
                    wake_at: None,
                    ended_at: Some(now),
                },
            )?;
        } else {
            let mut row = new_row(&run.id, step, ordinal, attempt, state);
            row.error_class = Some(class.to_string());
            row.error = Some(clean.clone());
            row.input_json = input_json;
            row.started_at = Some(now);
            row.ended_at = Some(now);
            store::insert_step(conn, owner, &row)?;
        }
        let may_continue =
            step.on_error == super::model::OnError::Continue && state != StepState::Uncertain;
        if may_continue {
            set_result(
                &mut sc.ctx,
                &step.id,
                state_word(state),
                false,
                Some(&clean),
                None,
            );
            Ok(StepFlow::Next)
        } else {
            info!(
                "workflows: Lauf {} endet bei Schritt {} ({}): {clean}",
                run.id,
                step.id,
                state.as_str()
            );
            Ok(StepFlow::Stop {
                error: format!("Schritt „{}“: {clean}", step.id),
                code: run_code,
            })
        }
    }

    // -----------------------------------------------------------------------
    // Arbeiter-Threads
    // -----------------------------------------------------------------------

    /// Startet den Arbeiter (Takt) und den Herzschlag. In B1 noch nicht in die App
    /// eingehaengt; B2 startet ihn beim Programmstart.
    pub fn spawn(&self) -> EngineHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        {
            let engine = self.clone();
            let stop = stop.clone();
            threads.push(
                std::thread::Builder::new()
                    .name("workflow-worker".to_string())
                    .spawn(move || engine.worker_loop(&stop))
                    .expect("workflow worker thread"),
            );
        }
        {
            let engine = self.clone();
            let stop = stop.clone();
            threads.push(
                std::thread::Builder::new()
                    .name("workflow-heartbeat".to_string())
                    .spawn(move || engine.heartbeat_loop(&stop))
                    .expect("workflow heartbeat thread"),
            );
        }
        EngineHandle {
            engine: self.clone(),
            stop,
            threads,
        }
    }

    fn worker_loop(&self, stop: &AtomicBool) {
        while !stop.load(Ordering::Acquire) {
            match catch_unwind(AssertUnwindSafe(|| self.tick())) {
                Ok(Ok(report)) => {
                    for e in &report.errors {
                        warn!("workflows: {e}");
                    }
                }
                Ok(Err(e)) => warn!("workflows: Takt gescheitert: {e}"),
                Err(_) => warn!("workflows: Takt abgestuerzt (Panik), der Arbeiter laeuft weiter"),
            }
            let (m, cv) = &self.inner.wake;
            let mut flag = m.lock().unwrap_or_else(|e| e.into_inner());
            if !*flag && !stop.load(Ordering::Acquire) {
                let wait = Duration::from_millis(self.inner.config.idle_wait_ms);
                flag = cv
                    .wait_timeout(flag, wait)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            *flag = false;
        }
    }

    fn heartbeat_loop(&self, stop: &AtomicBool) {
        let interval = Duration::from_millis(self.inner.config.heartbeat_ms.max(50));
        let step = Duration::from_millis(50);
        while !stop.load(Ordering::Acquire) {
            let mut waited = Duration::ZERO;
            while waited < interval && !stop.load(Ordering::Acquire) {
                std::thread::sleep(step);
                waited += step;
            }
            if stop.load(Ordering::Acquire) {
                break;
            }
            if let Err(e) = self.heartbeat() {
                warn!("workflows: Herzschlag gescheitert: {e}");
            }
        }
    }
}

/// Haelt die Arbeiter-Threads; beendet und wartet beim Loslassen.
pub struct EngineHandle {
    engine: Engine,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl EngineHandle {
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    /// Beenden beim Schliessen der App (B2): wartet hoechstens `limit` auf die Threads.
    /// Steckt der Arbeiter in einem langen Schritt (Transkription, Modell), bleibt er
    /// zurueck und stirbt mit dem Prozess; sein Lauf wird beim naechsten Start ueber den
    /// abgelaufenen Mietvertrag wieder aufgenommen (`recover_run`), ohne doppelte
    /// Aussenwirkung. `true`: alle Threads sind beendet.
    pub fn stop_within(mut self, limit: Duration) -> bool {
        self.stop.store(true, Ordering::Release);
        self.engine.wake();
        let deadline = std::time::Instant::now() + limit;
        loop {
            if self.threads.iter().all(|t| t.is_finished()) {
                for t in self.threads.drain(..) {
                    let _ = t.join();
                }
                return true;
            }
            if std::time::Instant::now() >= deadline {
                // Nicht auf den Thread warten (auch nicht beim Loslassen): er sieht das
                // Stoppzeichen nach dem laufenden Schritt.
                self.threads.clear();
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.engine.wake();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Hilfen
// ---------------------------------------------------------------------------

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unbekannt".to_string()
    }
}

fn new_row(run_id: &str, step: &StepDef, ordinal: u32, attempt: u32, state: StepState) -> StepRow {
    StepRow {
        run_id: run_id.to_string(),
        step_id: step.id.clone(),
        attempt,
        ordinal,
        action: step.action.clone(),
        state,
        error_class: None,
        input_json: None,
        output_json: None,
        error: None,
        approval_id: None,
        wake_at: None,
        started_at: None,
        ended_at: None,
    }
}

fn state_word(s: StepState) -> &'static str {
    match s {
        StepState::Done => "done",
        StepState::Failed => "failed",
        StepState::Denied => "denied",
        StepState::Skipped => "skipped",
        StepState::Planned => "planned",
        _ => "pending",
    }
}

/// Die Eingabe fuers Journal: Geheimnisse geschwaerzt, nie laenger als
/// [`MAX_INPUT_JOURNAL_BYTES`].
fn journal_input(params: &Value) -> String {
    let text = redact_params(params).to_string();
    if text.len() <= MAX_INPUT_JOURNAL_BYTES {
        text
    } else {
        json!({"truncated": true, "bytes": text.len()}).to_string()
    }
}

fn latest_row<'a>(journal: &'a [StepRow], step_id: &str) -> Option<&'a StepRow> {
    journal.iter().rfind(|r| r.step_id == step_id)
}

/// Ist der Schritt abgeschlossen (nichts mehr zu tun)?
fn is_complete(row: &StepRow, step: &StepDef) -> bool {
    match row.state {
        StepState::Done | StepState::Skipped | StepState::Planned => true,
        StepState::Failed | StepState::Denied => step.on_error == super::model::OnError::Continue,
        _ => false,
    }
}

/// Vorubergehende Fehlschlaege seit dem letzten endgueltigen Ergebnis des Schritts.
fn transient_failures(journal: &[StepRow], step_id: &str) -> usize {
    let mut n = 0;
    for r in journal.iter().filter(|r| r.step_id == step_id) {
        match r.state {
            StepState::Retrying => n += 1,
            StepState::Failed | StepState::Denied | StepState::Uncertain | StepState::Done => n = 0,
            _ => {}
        }
    }
    n
}

/// Schreibt das Ergebnis eines Schritts in `steps.<id>` (und `meeting`, falls die
/// Ausgabe eines liefert). `status`, `ok` und `error` setzt die Engine.
fn set_result(
    ctx: &mut Value,
    step_id: &str,
    status: &str,
    ok: bool,
    error: Option<&str>,
    output: Option<&Value>,
) {
    let mut obj: Map<String, Value> = output
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    if let Some(meeting) = obj.get("meeting").filter(|m| m.is_object()).cloned() {
        ctx["meeting"] = meeting;
    }
    obj.insert("status".to_string(), json!(status));
    obj.insert("ok".to_string(), json!(ok));
    if let Some(e) = error {
        obj.insert("error".to_string(), json!(e));
    }
    if !ctx["steps"].is_object() {
        ctx["steps"] = json!({});
    }
    ctx["steps"][step_id] = Value::Object(obj);
}

/// Der Laufkontext: `run`, `workflow`, `trigger`, `vars`, `meeting`, `steps`. Wird aus
/// dem Journal aufgebaut, nie aus dem Speicher: nach Neustart ist er derselbe.
fn build_ctx(run: &RunRow, def: &WorkflowDef, journal: &[StepRow]) -> Value {
    let base: Value = serde_json::from_str(&run.context_json).unwrap_or_else(|_| json!({}));
    let mut ctx = json!({
        "run": {
            "id": run.id,
            "dry_run": run.dry_run,
            "started_at": run.started_at.unwrap_or(run.created_at)
        },
        "workflow": {"id": run.workflow_id, "name": run.workflow_name},
        "trigger": base.get("trigger").cloned().unwrap_or_else(|| json!({})),
        "vars": base.get("vars").cloned().unwrap_or_else(|| json!({})),
        "steps": {}
    });
    if let Some(m) = ctx["trigger"]
        .get("meeting")
        .filter(|m| m.is_object())
        .cloned()
    {
        ctx["meeting"] = m;
    }
    for step in &def.steps {
        let Some(row) = latest_row(journal, &step.id) else {
            continue;
        };
        if !is_complete(row, step) {
            continue;
        }
        let output = row
            .output_json
            .as_deref()
            .and_then(|t| serde_json::from_str::<Value>(t).ok());
        let ok = matches!(row.state, StepState::Done | StepState::Planned);
        // Im Trockenlauf steht in `output_json` der Plan, kein Ergebnis.
        let output = if row.state == StepState::Planned || row.state == StepState::Skipped {
            None
        } else {
            output
        };
        set_result(
            &mut ctx,
            &step.id,
            state_word(row.state),
            ok,
            row.error.as_deref(),
            output.as_ref(),
        );
    }
    ctx
}

/// Variablen fuer einen Lauf: Vorgabewerte der Definition, ueberschrieben durch die
/// angegebenen (nur deklarierte, passende Werte). Eine Variable ohne Vorgabe und ohne
/// Angabe ist ein Fehler.
fn resolve_vars(def: &WorkflowDef, given: &Map<String, Value>) -> Result<Map<String, Value>> {
    for (name, value) in given {
        let Some(decl) = def.variables.get(name) else {
            return Err(WorkflowError::BadInput(format!(
                "Die Variable „{name}“ ist im Ablauf nicht deklariert."
            )));
        };
        if !decl.ty.accepts(value) {
            return Err(WorkflowError::BadInput(format!(
                "Die Variable „{name}“ erwartet eine Angabe der Art „{}“.",
                decl.ty.as_str()
            )));
        }
    }
    let mut out = Map::new();
    for (name, decl) in &def.variables {
        match given.get(name).or(decl.default.as_ref()) {
            Some(v) if !v.is_null() => {
                out.insert(name.clone(), v.clone());
            }
            _ => {
                return Err(WorkflowError::BadInput(format!(
                    "Die Variable „{name}“ hat keinen Vorgabewert und muss angegeben werden."
                )));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
