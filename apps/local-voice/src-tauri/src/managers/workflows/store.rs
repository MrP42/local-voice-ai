//! Lesen und Schreiben der Workflow-Tabellen (B1). Jede Funktion arbeitet auf einer
//! `&Connection` (Tests und Engine holen sie aus der Datenbank der Besprechungen).
//!
//! Regeln, auf die sich die Engine stuetzt:
//! - **Idempotenz in der Datenbank**: `insert_run` ist ein einziges
//!   `INSERT ... ON CONFLICT DO NOTHING` gegen `UNIQUE (workflow_id, trigger_key)`;
//!   zwei Threads oder Prozesse mit demselben Ausloeser ergeben eine Zeile, der zweite
//!   bekommt die vorhandene zurueck.
//! - **Genau ein Arbeiter je Lauf**: `claim_next` ist EIN `UPDATE ... RETURNING` mit
//!   Bedingung `state = 'queued'`; wer 0 Zeilen aendert, hat nichts.
//! - **Zaun (Fencing)**: jede Schreibung eines Arbeiters (Schrittzeile, Lauf-Zustand)
//!   verlangt `lease_owner = ich AND state = 'running'`; hat ein anderer den Lauf nach
//!   Ablauf des Mietvertrags uebernommen, scheitert sie mit `LeaseLost`, und der
//!   alte Arbeiter hoert auf, bevor er Weiteres anrichtet.
//! - **Begrenzt**: je Workflow hoechstens [`MAX_QUEUED_PER_WORKFLOW`] wartende Laeufe,
//!   Aufbewahrung der beendeten Laeufe gedeckelt ([`prune`]).

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use serde::Serialize;
use ulid::Ulid;

use super::model::{Origin, RunState, StepState, WorkflowDef, SCHEMA_VERSION};
use super::validate::Issue;

/// Hoechstzahl wartender (nicht beendeter) Laeufe je Workflow.
pub const MAX_QUEUED_PER_WORKFLOW: i64 = 500;
/// Aufbewahrung beendeter Laeufe je Workflow (die juengsten bleiben).
pub const MAX_RUNS_PER_WORKFLOW: i64 = 200;
/// Aufbewahrung beendeter Laeufe insgesamt.
pub const MAX_RUNS_TOTAL: i64 = 5_000;
/// Groesste Ausloeserdaten-Angabe eines Laufs (JSON-Text, Bytes).
pub const MAX_CONTEXT_BYTES: usize = 64 * 1024;
/// Laengster Schluessel eines Ausloesers.
pub const MAX_TRIGGER_KEY_CHARS: usize = 300;

/// Fehler des Workflow-Moduls. `Display` ist Klartext fuer die Oberflaeche.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowError {
    /// Die Definition ist ungueltig (alle Befunde).
    Invalid(Vec<Issue>),
    /// Eine Eingabe ist ungueltig (Text auf Deutsch).
    BadInput(String),
    NotFound(String),
    /// Der Ablauf ist ausgeschaltet.
    Disabled(String),
    /// Zu viele wartende Laeufe.
    QueueFull(String),
    /// Der Arbeiter hat den Lauf verloren (Mietvertrag abgelaufen, anderer Arbeiter).
    LeaseLost,
    /// Gerade nicht moeglich (z. B. Ablauf mit aktiven Laeufen loeschen).
    Busy(String),
    /// Datenbank (gesperrt, voll, defekt).
    Store(String),
    /// Nur in Tests: absichtlicher Abbruch an einer Stelle (Absturz-Simulation).
    #[cfg(test)]
    SimulatedCrash(&'static str),
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkflowError::Invalid(issues) => {
                let first = issues
                    .first()
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "unbekannt".to_string());
                if issues.len() > 1 {
                    write!(
                        f,
                        "Die Definition ist ungültig: {first} (und {} weitere Befunde).",
                        issues.len() - 1
                    )
                } else {
                    write!(f, "Die Definition ist ungültig: {first}")
                }
            }
            WorkflowError::BadInput(m) | WorkflowError::Busy(m) | WorkflowError::QueueFull(m) => {
                write!(f, "{m}")
            }
            WorkflowError::NotFound(id) => write!(f, "Nicht gefunden: {id}"),
            WorkflowError::Disabled(m) => write!(f, "{m}"),
            WorkflowError::LeaseLost => {
                write!(
                    f,
                    "Der Lauf gehört einem anderen Arbeiter (Mietvertrag verloren)."
                )
            }
            WorkflowError::Store(m) => write!(f, "Speicherfehler: {m}"),
            #[cfg(test)]
            WorkflowError::SimulatedCrash(at) => write!(f, "simulierter Absturz bei {at}"),
        }
    }
}

impl std::error::Error for WorkflowError {}

impl From<rusqlite::Error> for WorkflowError {
    fn from(e: rusqlite::Error) -> Self {
        WorkflowError::Store(e.to_string())
    }
}

type Result<T> = std::result::Result<T, WorkflowError>;

// ---------------------------------------------------------------------------
// Zeilen
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkflowRow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// `true`: noch nicht scharf geschaltet, jeder Lauf plant nur.
    pub dry_run: bool,
    pub schema_version: i64,
    pub definition_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RunRow {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub trigger_key: String,
    pub origin: Origin,
    pub state: RunState,
    pub dry_run: bool,
    pub definition_json: String,
    pub context_json: String,
    pub next_run_at: Option<i64>,
    pub wait_reason: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_until: Option<i64>,
    pub cancel_requested: bool,
    pub error: Option<String>,
    pub error_code: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StepRow {
    pub run_id: String,
    pub step_id: String,
    pub attempt: u32,
    pub ordinal: u32,
    pub action: String,
    pub state: StepState,
    pub error_class: Option<String>,
    pub input_json: Option<String>,
    pub output_json: Option<String>,
    pub error: Option<String>,
    pub approval_id: Option<String>,
    pub wake_at: Option<i64>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

fn enum_error(column: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("unbekannter Wert in {column}: {value}").into(),
    )
}

fn map_workflow(r: &Row<'_>) -> rusqlite::Result<WorkflowRow> {
    Ok(WorkflowRow {
        id: r.get("id")?,
        name: r.get("name")?,
        enabled: r.get::<_, i64>("enabled")? != 0,
        dry_run: r.get::<_, i64>("dry_run")? != 0,
        schema_version: r.get("schema_version")?,
        definition_json: r.get("definition_json")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

fn map_run(r: &Row<'_>) -> rusqlite::Result<RunRow> {
    let origin: String = r.get("origin")?;
    let state: String = r.get("state")?;
    Ok(RunRow {
        id: r.get("id")?,
        workflow_id: r.get("workflow_id")?,
        workflow_name: r.get("workflow_name")?,
        trigger_key: r.get("trigger_key")?,
        origin: Origin::parse(&origin).ok_or_else(|| enum_error("origin", &origin))?,
        state: RunState::parse(&state).ok_or_else(|| enum_error("state", &state))?,
        dry_run: r.get::<_, i64>("dry_run")? != 0,
        definition_json: r.get("definition_json")?,
        context_json: r.get("context_json")?,
        next_run_at: r.get("next_run_at")?,
        wait_reason: r.get("wait_reason")?,
        lease_owner: r.get("lease_owner")?,
        lease_until: r.get("lease_until")?,
        cancel_requested: r.get::<_, i64>("cancel_requested")? != 0,
        error: r.get("error")?,
        error_code: r.get("error_code")?,
        created_at: r.get("created_at")?,
        started_at: r.get("started_at")?,
        ended_at: r.get("ended_at")?,
        updated_at: r.get("updated_at")?,
    })
}

fn map_step(r: &Row<'_>) -> rusqlite::Result<StepRow> {
    let state: String = r.get("state")?;
    Ok(StepRow {
        run_id: r.get("run_id")?,
        step_id: r.get("step_id")?,
        attempt: r.get::<_, i64>("attempt")?.max(0) as u32,
        ordinal: r.get::<_, i64>("ordinal")?.max(0) as u32,
        action: r.get("action")?,
        state: StepState::parse(&state).ok_or_else(|| enum_error("state", &state))?,
        error_class: r.get("error_class")?,
        input_json: r.get("input_json")?,
        output_json: r.get("output_json")?,
        error: r.get("error")?,
        approval_id: r.get("approval_id")?,
        wake_at: r.get("wake_at")?,
        started_at: r.get("started_at")?,
        ended_at: r.get("ended_at")?,
    })
}

const WORKFLOW_COLS: &str =
    "id, name, enabled, dry_run, schema_version, definition_json, created_at, updated_at";
const RUN_COLS: &str = "id, workflow_id, workflow_name, trigger_key, origin, state, dry_run,
    definition_json, context_json, next_run_at, wait_reason, lease_owner, lease_until,
    cancel_requested, error, error_code, created_at, started_at, ended_at, updated_at";
const STEP_COLS: &str = "run_id, step_id, attempt, ordinal, action, state, error_class, input_json,
    output_json, error, approval_id, wake_at, started_at, ended_at";

// ---------------------------------------------------------------------------
// Workflows
// ---------------------------------------------------------------------------

pub fn get_workflow(conn: &Connection, id: &str) -> Result<Option<WorkflowRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {WORKFLOW_COLS} FROM workflows WHERE id = ?1"),
            params![id],
            map_workflow,
        )
        .optional()?)
}

pub fn list_workflows(conn: &Connection) -> Result<Vec<WorkflowRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {WORKFLOW_COLS} FROM workflows ORDER BY name COLLATE NOCASE, id"
    ))?;
    let rows = stmt
        .query_map([], map_workflow)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Legt einen Ablauf an oder ersetzt seine Definition. Ein NEUER Ablauf ist
/// ausgeschaltet und im Trockenlauf. Aendert sich die Definition eines bestehenden,
/// faellt er in den Trockenlauf zurueck: was der Nutzer scharf geschaltet hat, war
/// die alte Fassung, nicht die neue (R2: keine unbemerkt neuen Empfaenger).
pub fn save_workflow(
    conn: &Connection,
    id: Option<&str>,
    def: &WorkflowDef,
    definition_json: &str,
    now_ms: i64,
) -> Result<WorkflowRow> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let id = match id {
        Some(existing) => {
            let Some(old) = get_workflow(&tx, existing)? else {
                return Err(WorkflowError::NotFound(existing.to_string()));
            };
            let changed = old.definition_json != definition_json;
            tx.execute(
                "UPDATE workflows SET name = ?2, definition_json = ?3, schema_version = ?4,
                        dry_run = CASE WHEN ?5 THEN 1 ELSE dry_run END, updated_at = ?6
                 WHERE id = ?1",
                params![
                    existing,
                    def.name,
                    definition_json,
                    SCHEMA_VERSION,
                    changed,
                    now_ms
                ],
            )?;
            existing.to_string()
        }
        None => {
            let new_id = Ulid::new().to_string();
            tx.execute(
                "INSERT INTO workflows (id, name, enabled, dry_run, schema_version,
                                        definition_json, created_at, updated_at)
                 VALUES (?1, ?2, 0, 1, ?3, ?4, ?5, ?5)",
                params![new_id, def.name, SCHEMA_VERSION, definition_json, now_ms],
            )?;
            new_id
        }
    };
    let row = get_workflow(&tx, &id)?.ok_or_else(|| WorkflowError::NotFound(id.clone()))?;
    tx.commit()?;
    Ok(row)
}

pub fn set_enabled(conn: &Connection, id: &str, enabled: bool, now_ms: i64) -> Result<()> {
    let n = conn.execute(
        "UPDATE workflows SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, enabled, now_ms],
    )?;
    if n == 0 {
        return Err(WorkflowError::NotFound(id.to_string()));
    }
    Ok(())
}

/// Scharf schalten (`armed = true`): Laeufe fuehren aus. Zurueck in den Trockenlauf
/// (`false`): Laeufe planen nur.
pub fn set_armed(conn: &Connection, id: &str, armed: bool, now_ms: i64) -> Result<()> {
    let n = conn.execute(
        "UPDATE workflows SET dry_run = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, !armed, now_ms],
    )?;
    if n == 0 {
        return Err(WorkflowError::NotFound(id.to_string()));
    }
    Ok(())
}

/// Loescht einen Ablauf samt seinen beendeten Laeufen. Mit wartenden oder laufenden
/// Laeufen: `Busy` (erst abbrechen).
pub fn delete_workflow(conn: &Connection, id: &str) -> Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if get_workflow(&tx, id)?.is_none() {
        return Err(WorkflowError::NotFound(id.to_string()));
    }
    let active: i64 = tx.query_row(
        "SELECT COUNT(*) FROM workflow_runs
         WHERE workflow_id = ?1 AND state IN ('queued','running','awaiting_approval')",
        params![id],
        |r| r.get(0),
    )?;
    if active > 0 {
        return Err(WorkflowError::Busy(format!(
            "Der Ablauf hat noch {active} offene Läufe. Zuerst abbrechen."
        )));
    }
    tx.execute(
        "DELETE FROM workflow_run_steps WHERE run_id IN
           (SELECT id FROM workflow_runs WHERE workflow_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM workflow_runs WHERE workflow_id = ?1",
        params![id],
    )?;
    tx.execute("DELETE FROM workflows WHERE id = ?1", params![id])?;
    tx.commit()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Laeufe
// ---------------------------------------------------------------------------

pub struct NewRun<'a> {
    pub workflow: &'a WorkflowRow,
    pub trigger_key: &'a str,
    pub origin: Origin,
    pub dry_run: bool,
    pub context_json: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InsertedRun {
    pub run_id: String,
    /// `false`: der Ausloeser hatte schon einen Lauf; es ist dessen Kennung.
    pub created: bool,
}

/// Reiht einen Lauf ein (siehe Moduldoku: Idempotenz, Obergrenze).
pub fn insert_run(conn: &Connection, new: &NewRun<'_>, now_ms: i64) -> Result<InsertedRun> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM workflow_runs WHERE workflow_id = ?1 AND trigger_key = ?2",
            params![new.workflow.id, new.trigger_key],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(run_id) = existing {
        tx.commit()?;
        return Ok(InsertedRun {
            run_id,
            created: false,
        });
    }
    let waiting: i64 = tx.query_row(
        "SELECT COUNT(*) FROM workflow_runs
         WHERE workflow_id = ?1 AND state IN ('queued','running','awaiting_approval')",
        params![new.workflow.id],
        |r| r.get(0),
    )?;
    if waiting >= MAX_QUEUED_PER_WORKFLOW {
        return Err(WorkflowError::QueueFull(format!(
            "Zu viele wartende Läufe für diesen Ablauf (höchstens {MAX_QUEUED_PER_WORKFLOW})."
        )));
    }
    let run_id = Ulid::new().to_string();
    // ON CONFLICT: ein zweiter Prozess, der zwischen Pruefung und Einfuegen dasselbe
    // einreiht, hinterlaesst genau eine Zeile.
    let changed = tx.execute(
        "INSERT INTO workflow_runs (id, workflow_id, workflow_name, trigger_key, origin, state,
                dry_run, definition_json, context_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT(workflow_id, trigger_key) DO NOTHING",
        params![
            run_id,
            new.workflow.id,
            new.workflow.name,
            new.trigger_key,
            new.origin.as_str(),
            new.dry_run,
            new.workflow.definition_json,
            new.context_json,
            now_ms
        ],
    )?;
    if changed == 0 {
        let run_id: String = tx.query_row(
            "SELECT id FROM workflow_runs WHERE workflow_id = ?1 AND trigger_key = ?2",
            params![new.workflow.id, new.trigger_key],
            |r| r.get(0),
        )?;
        tx.commit()?;
        return Ok(InsertedRun {
            run_id,
            created: false,
        });
    }
    tx.commit()?;
    Ok(InsertedRun {
        run_id,
        created: true,
    })
}

pub fn get_run(conn: &Connection, id: &str) -> Result<Option<RunRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {RUN_COLS} FROM workflow_runs WHERE id = ?1"),
            params![id],
            map_run,
        )
        .optional()?)
}

#[derive(Clone, Debug, Default)]
pub struct RunFilter {
    pub workflow_id: Option<String>,
    pub state: Option<RunState>,
}

/// Neueste zuerst.
pub fn list_runs(conn: &Connection, filter: &RunFilter, limit: i64) -> Result<Vec<RunRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLS} FROM workflow_runs
         WHERE (?1 IS NULL OR workflow_id = ?1) AND (?2 IS NULL OR state = ?2)
         ORDER BY created_at DESC, id DESC LIMIT ?3"
    ))?;
    let rows = stmt
        .query_map(
            params![
                filter.workflow_id,
                filter.state.map(RunState::as_str),
                limit.clamp(1, 1_000)
            ],
            map_run,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Holt den naechsten faelligen Lauf und gibt ihn dem Arbeiter `owner` (Mietvertrag
/// bis `now + ttl`). `None`: nichts faellig.
pub fn claim_next(
    conn: &Connection,
    owner: &str,
    now_ms: i64,
    lease_ttl_ms: i64,
) -> Result<Option<RunRow>> {
    let id: Option<String> = conn
        .query_row(
            "UPDATE workflow_runs
                SET state = 'running', lease_owner = ?1, lease_until = ?2, next_run_at = NULL,
                    wait_reason = NULL, started_at = COALESCE(started_at, ?3), updated_at = ?3
              WHERE state = 'queued'
                AND id = (SELECT id FROM workflow_runs
                           WHERE state = 'queued' AND (next_run_at IS NULL OR next_run_at <= ?3)
                           ORDER BY COALESCE(next_run_at, created_at), created_at, id LIMIT 1)
              RETURNING id",
            params![owner, now_ms + lease_ttl_ms, now_ms],
            |r| r.get(0),
        )
        .optional()?;
    match id {
        Some(id) => get_run(conn, &id),
        None => Ok(None),
    }
}

/// Verlaengert den Mietvertrag der genannten Laeufe von `owner` (nur die, die der
/// Arbeiter wirklich noch bearbeitet: ein aufgegebener Lauf verfaellt). Gibt die
/// Zahl zurueck.
pub fn renew_leases(
    conn: &Connection,
    owner: &str,
    run_ids: &[String],
    now_ms: i64,
    lease_ttl_ms: i64,
) -> Result<usize> {
    if run_ids.is_empty() {
        return Ok(0);
    }
    let json = serde_json::to_string(run_ids).unwrap_or_else(|_| "[]".to_string());
    Ok(conn.execute(
        "UPDATE workflow_runs SET lease_until = ?2
         WHERE lease_owner = ?1 AND state = 'running'
           AND id IN (SELECT value FROM json_each(?3))",
        params![owner, now_ms + lease_ttl_ms, json],
    )?)
}

/// Laeufe im Zustand `running`, deren Mietvertrag abgelaufen ist (Absturz).
pub fn list_expired(conn: &Connection, now_ms: i64) -> Result<Vec<RunRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLS} FROM workflow_runs
         WHERE state = 'running' AND (lease_until IS NULL OR lease_until < ?1)
         ORDER BY created_at, id"
    ))?;
    let rows = stmt
        .query_map(params![now_ms], map_run)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Laeufe, die auf eine Freigabe warten.
pub fn list_awaiting(conn: &Connection) -> Result<Vec<RunRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLS} FROM workflow_runs WHERE state = 'awaiting_approval'
         ORDER BY created_at, id"
    ))?;
    let rows = stmt
        .query_map([], map_run)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn fenced(changed: usize) -> Result<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err(WorkflowError::LeaseLost)
    }
}

/// Der Arbeiter gibt den Lauf zurueck: wartend (`Queued` mit Zeitpunkt und Grund)
/// oder auf Freigabe wartend.
pub fn park_run(
    conn: &Connection,
    run_id: &str,
    owner: &str,
    state: RunState,
    next_run_at: Option<i64>,
    wait_reason: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    debug_assert!(matches!(
        state,
        RunState::Queued | RunState::AwaitingApproval
    ));
    fenced(conn.execute(
        "UPDATE workflow_runs SET state = ?3, next_run_at = ?4, wait_reason = ?5,
                lease_owner = NULL, lease_until = NULL, updated_at = ?6
         WHERE id = ?1 AND lease_owner = ?2 AND state = 'running'",
        params![
            run_id,
            owner,
            state.as_str(),
            next_run_at,
            wait_reason,
            now_ms
        ],
    )?)
}

/// Der Arbeiter beendet den Lauf (`Done`, `Failed`, `Cancelled`).
pub fn finish_run(
    conn: &Connection,
    run_id: &str,
    owner: &str,
    state: RunState,
    error: Option<&str>,
    error_code: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    debug_assert!(state.is_terminal());
    fenced(conn.execute(
        "UPDATE workflow_runs SET state = ?3, error = ?4, error_code = ?5, ended_at = ?6,
                lease_owner = NULL, lease_until = NULL, next_run_at = NULL, wait_reason = NULL,
                updated_at = ?6
         WHERE id = ?1 AND lease_owner = ?2 AND state = 'running'",
        params![run_id, owner, state.as_str(), error, error_code, now_ms],
    )?)
}

/// Setzt ein beendetes oder wartendes Lauf auf `queued` zurueck (Wiederholung durch den
/// Nutzer, Freigabe erteilt). Nur aus den genannten Zustaenden.
pub fn requeue(
    conn: &Connection,
    run_id: &str,
    from: &[RunState],
    next_run_at: Option<i64>,
    now_ms: i64,
) -> Result<bool> {
    let states: Vec<&str> = from.iter().map(|s| s.as_str()).collect();
    let json = serde_json::to_string(&states).unwrap_or_else(|_| "[]".to_string());
    let n = conn.execute(
        "UPDATE workflow_runs SET state = 'queued', next_run_at = ?2, wait_reason = NULL,
                error = NULL, error_code = NULL, ended_at = NULL, cancel_requested = 0,
                lease_owner = NULL, lease_until = NULL, updated_at = ?3
         WHERE id = ?1 AND state IN (SELECT value FROM json_each(?4))",
        params![run_id, next_run_at, now_ms, json],
    )?;
    Ok(n == 1)
}

/// Merkt den Abbruchwunsch eines laufenden Laufs vor (der Arbeiter liest ihn).
pub fn mark_cancel_requested(conn: &Connection, run_id: &str, now_ms: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE workflow_runs SET cancel_requested = 1, updated_at = ?2
         WHERE id = ?1 AND state = 'running'",
        params![run_id, now_ms],
    )? == 1)
}

/// Bricht einen NICHT laufenden Lauf sofort ab (wartend oder auf Freigabe).
pub fn cancel_idle(conn: &Connection, run_id: &str, now_ms: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE workflow_runs SET state = 'cancelled', ended_at = ?2, updated_at = ?2,
                next_run_at = NULL, wait_reason = NULL, error_code = NULL,
                error = 'Vom Nutzer abgebrochen.'
         WHERE id = ?1 AND state IN ('queued','awaiting_approval')",
        params![run_id, now_ms],
    )? == 1)
}

/// Liest `cancel_requested` frisch aus der Datenbank.
pub fn cancel_requested(conn: &Connection, run_id: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT cancel_requested FROM workflow_runs WHERE id = ?1",
            params![run_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0)
        != 0)
}

// ---------------------------------------------------------------------------
// Wiederaufnahme (Mietvertrag abgelaufen)
// ---------------------------------------------------------------------------

const EXPIRED: &str = "state = 'running' AND (lease_until IS NULL OR lease_until < ?2)";

/// Nimmt einen Lauf mit abgelaufenem Vertrag zurueck in die Warteschlange. Ein
/// bedingtes UPDATE: bei zwei gleichzeitigen Uebernehmern gewinnt genau einer.
pub fn recover_requeue(conn: &Connection, run_id: &str, now_ms: i64) -> Result<bool> {
    Ok(conn.execute(
        &format!(
            "UPDATE workflow_runs SET state = 'queued', lease_owner = NULL, lease_until = NULL,
                    next_run_at = NULL, wait_reason = NULL, updated_at = ?2
             WHERE id = ?1 AND {EXPIRED}"
        ),
        params![run_id, now_ms],
    )? == 1)
}

#[allow(clippy::too_many_arguments)]
fn recover_step(
    tx: &Connection,
    run_id: &str,
    row: &StepRow,
    state: StepState,
    class: Option<&str>,
    output_json: Option<&str>,
    error: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    tx.execute(
        "UPDATE workflow_run_steps
            SET state = ?4, error_class = ?5, output_json = ?6, error = ?7, ended_at = ?8
          WHERE run_id = ?1 AND step_id = ?2 AND attempt = ?3 AND state = 'running'",
        params![
            run_id,
            row.step_id,
            row.attempt,
            state.as_str(),
            class,
            output_json,
            error,
            now_ms
        ],
    )?;
    Ok(())
}

/// Uebernimmt den Lauf UND schreibt den unterbrochenen Schritt um (eine Transaktion):
/// der Lauf wartet wieder, der Schritt steht auf `state` (z. B. `interrupted`,
/// oder `done`, wenn die Wirkung belegt ist). `false`: ein anderer war schneller.
#[allow(clippy::too_many_arguments)]
pub fn recover_step_and_requeue(
    conn: &Connection,
    run_id: &str,
    row: &StepRow,
    state: StepState,
    class: Option<&str>,
    output_json: Option<&str>,
    error: Option<&str>,
    now_ms: i64,
) -> Result<bool> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if !recover_requeue(&tx, run_id, now_ms)? {
        return Ok(false);
    }
    recover_step(&tx, run_id, row, state, class, output_json, error, now_ms)?;
    tx.commit()?;
    Ok(true)
}

/// Wie `recover_step_and_requeue`, aber der Lauf endet `failed` mit `error_code`.
#[allow(clippy::too_many_arguments)]
pub fn recover_step_and_fail(
    conn: &Connection,
    run_id: &str,
    row: &StepRow,
    state: StepState,
    class: Option<&str>,
    message: &str,
    error_code: &str,
    now_ms: i64,
) -> Result<bool> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        &format!(
            "UPDATE workflow_runs SET state = 'failed', error = ?3, error_code = ?4, ended_at = ?2,
                    lease_owner = NULL, lease_until = NULL, next_run_at = NULL, wait_reason = NULL,
                    updated_at = ?2
             WHERE id = ?1 AND {EXPIRED}"
        ),
        params![run_id, now_ms, message, error_code],
    )?;
    if changed != 1 {
        return Ok(false);
    }
    recover_step(&tx, run_id, row, state, class, None, Some(message), now_ms)?;
    tx.commit()?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Schrittjournal
// ---------------------------------------------------------------------------

/// Alle Versuche aller Schritte eines Laufs, nach Position und Versuch.
pub fn steps_for(conn: &Connection, run_id: &str) -> Result<Vec<StepRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {STEP_COLS} FROM workflow_run_steps WHERE run_id = ?1 ORDER BY ordinal, attempt"
    ))?;
    let rows = stmt
        .query_map(params![run_id], map_step)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Schreibt eine Schrittzeile (jeder Zustand), nur wenn `owner` den Lauf noch haelt.
pub fn insert_step(conn: &Connection, owner: &str, s: &StepRow) -> Result<()> {
    fenced(conn.execute(
        "INSERT INTO workflow_run_steps (run_id, step_id, attempt, ordinal, action, state,
                error_class, input_json, output_json, error, approval_id, wake_at,
                started_at, ended_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
         WHERE EXISTS (SELECT 1 FROM workflow_runs
                        WHERE id = ?1 AND lease_owner = ?15 AND state = 'running')",
        params![
            s.run_id,
            s.step_id,
            s.attempt,
            s.ordinal,
            s.action,
            s.state.as_str(),
            s.error_class,
            s.input_json,
            s.output_json,
            s.error,
            s.approval_id,
            s.wake_at,
            s.started_at,
            s.ended_at,
            owner
        ],
    )?)
}

/// Aendert Zustand und Ergebnis einer vorhandenen Schrittzeile (fenced).
pub struct StepUpdate<'a> {
    pub run_id: &'a str,
    pub step_id: &'a str,
    pub attempt: u32,
    pub state: StepState,
    pub error_class: Option<&'a str>,
    pub output_json: Option<&'a str>,
    pub error: Option<&'a str>,
    pub approval_id: Option<&'a str>,
    pub wake_at: Option<i64>,
    pub ended_at: Option<i64>,
}

pub fn update_step(conn: &Connection, owner: &str, u: &StepUpdate<'_>) -> Result<()> {
    fenced(conn.execute(
        "UPDATE workflow_run_steps
            SET state = ?4, error_class = ?5, output_json = ?6, error = ?7,
                approval_id = COALESCE(?8, approval_id), wake_at = ?9, ended_at = ?10
          WHERE run_id = ?1 AND step_id = ?2 AND attempt = ?3
            AND EXISTS (SELECT 1 FROM workflow_runs
                         WHERE id = ?1 AND lease_owner = ?11 AND state = 'running')",
        params![
            u.run_id,
            u.step_id,
            u.attempt,
            u.state.as_str(),
            u.error_class,
            u.output_json,
            u.error,
            u.approval_id,
            u.wake_at,
            u.ended_at,
            owner
        ],
    )?)
}

/// Aus `waiting` oder `awaiting_approval` zurueck auf `running` (fenced); behaelt
/// `started_at` (die Frist von `wait` rechnet ab dem ersten Beginn).
pub fn reopen_step(
    conn: &Connection,
    owner: &str,
    run_id: &str,
    step_id: &str,
    attempt: u32,
) -> Result<()> {
    fenced(conn.execute(
        "UPDATE workflow_run_steps SET state = 'running', wake_at = NULL, ended_at = NULL
          WHERE run_id = ?1 AND step_id = ?2 AND attempt = ?3
            AND state IN ('waiting','awaiting_approval')
            AND EXISTS (SELECT 1 FROM workflow_runs
                         WHERE id = ?1 AND lease_owner = ?4 AND state = 'running')",
        params![run_id, step_id, attempt, owner],
    )?)
}

/// Die jeweils letzte Zeile je Schritt, in der Reihenfolge der Schritte.
pub fn latest_per_step(rows: &[StepRow]) -> Vec<&StepRow> {
    let mut out: Vec<&StepRow> = Vec::new();
    for r in rows {
        match out.last_mut() {
            Some(last) if last.step_id == r.step_id => *last = r,
            _ => out.push(r),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Aufbewahrung
// ---------------------------------------------------------------------------

/// Loescht beendete Laeufe jenseits der Obergrenzen (je Workflow
/// [`MAX_RUNS_PER_WORKFLOW`], insgesamt [`MAX_RUNS_TOTAL`]), die aeltesten zuerst.
/// Offene Laeufe bleiben immer. Gibt die Zahl geloeschter Laeufe zurueck.
pub fn prune(conn: &Connection) -> Result<usize> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let mut victims: Vec<String> = Vec::new();
    {
        let mut stmt = tx.prepare(
            "WITH ranked AS (
               SELECT id, ROW_NUMBER() OVER (PARTITION BY workflow_id
                                             ORDER BY created_at DESC, id DESC) AS rn
                 FROM workflow_runs WHERE state IN ('done','failed','cancelled'))
             SELECT id FROM ranked WHERE rn > ?1",
        )?;
        let rows = stmt.query_map(params![MAX_RUNS_PER_WORKFLOW], |r| r.get::<_, String>(0))?;
        for id in rows {
            victims.push(id?);
        }
    }
    {
        let mut stmt = tx.prepare(
            "SELECT id FROM workflow_runs WHERE state IN ('done','failed','cancelled')
             ORDER BY created_at DESC, id DESC LIMIT -1 OFFSET ?1",
        )?;
        let rows = stmt.query_map(params![MAX_RUNS_TOTAL], |r| r.get::<_, String>(0))?;
        for id in rows {
            let id = id?;
            if !victims.contains(&id) {
                victims.push(id);
            }
        }
    }
    if victims.is_empty() {
        tx.commit()?;
        return Ok(0);
    }
    let json = serde_json::to_string(&victims).unwrap_or_else(|_| "[]".to_string());
    tx.execute(
        "DELETE FROM workflow_run_steps WHERE run_id IN (SELECT value FROM json_each(?1))",
        params![json],
    )?;
    tx.execute(
        "DELETE FROM workflow_runs WHERE id IN (SELECT value FROM json_each(?1))",
        params![json],
    )?;
    tx.commit()?;
    Ok(victims.len())
}

#[cfg(test)]
mod tests;
