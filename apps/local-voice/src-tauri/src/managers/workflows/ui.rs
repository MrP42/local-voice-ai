//! Die Schnittstelle der Oberflaeche „Automationen“ (B7): die Rechnung hinter den Kommandos
//! `workflow_*` (`commands::workflows`), ohne Tauri, damit sie ohne Programmstart pruefbar ist.
//!
//! Die Oberflaeche hat keine eigene Logik fuer Pruefung, Trockenlauf oder Rechte: sie zeigt, was
//! die Engine sagt. Darum liefern diese Funktionen
//! - den Katalog (Ausloeser, Bausteine, Felder) als Beschreibung, aus der das Formular entsteht
//!   (ein neuer Baustein aus B5/B6 erscheint ohne Aenderung der Oberflaeche),
//! - die Pruefung einer Definition mit JSON-Zeiger je Befund (`validate`),
//! - den Trockenlauf (`plan::plan_definition`, derselbe wie `--workflow-run --dry-run`),
//! - Listen, Laufprotokoll mit Schritten und Herkunft, Abbrechen und Wiederholen.
//!
//! Gelesen und geschrieben wird nur ueber die Engine (`Engine::save_workflow` usw.): die
//! Oberflaeche kann also nichts speichern, was die Engine nicht auch beim Lauf akzeptierte, und
//! sie fuehrt nie einen Baustein selbst aus. Ein manueller Start geht durch `trigger::manual`
//! und damit durch dieselbe Idempotenz, dieselben Rechte und dieselbe Einwilligung wie jeder
//! andere Ausloeser.
//!
//! Strukturierte Antworten gehen als JSON-TEXT (`*_json`), wie im Rest der App (z. B.
//! `Integration.config_json`): so braucht kein Typ der Engine einen Typ-Export, und die
//! Oberflaeche liest nur, was sie braucht.

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Map, Value};
use specta::Type;

use crate::managers::provenance::ProvenanceEntry;

use super::catalog::{self, FieldKind, FieldSpec, NeedsSpec};
use super::engine::{CancelResult, Engine};
use super::model::{code, RunState, StepState, MAX_STEPS, SCHEMA_ID};
use super::plan;
use super::store::{self, RunRow, StepRow, WorkflowError, WorkflowRow};
use super::templates;
use super::trigger::manual;
use super::validate::{self, Issue};

/// Hoechstzahl Laeufe in der Liste „Laeufe“.
pub const MAX_RUNS_LISTED: i64 = 200;
/// Groesste Importdatei in Bytes (wie eine Definition, `model::MAX_DEFINITION_BYTES`).
pub const MAX_IMPORT_BYTES: usize = super::model::MAX_DEFINITION_BYTES;

// ---------------------------------------------------------------------------
// Typen fuer die Oberflaeche
// ---------------------------------------------------------------------------

/// Ein Befund der Pruefung: wo (JSON-Zeiger, leer = ganze Definition) und was (deutscher Satz).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Type)]
pub struct WorkflowIssue {
    pub path: String,
    pub message: String,
}

impl From<&Issue> for WorkflowIssue {
    fn from(i: &Issue) -> Self {
        Self {
            path: i.path.clone(),
            message: i.message.clone(),
        }
    }
}

fn issues_of(list: &[Issue]) -> Vec<WorkflowIssue> {
    list.iter().map(WorkflowIssue::from).collect()
}

/// Kurzfassung eines Laufs fuer Listen.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowRunSummary {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    /// Herkunft des Starts: `trigger`, `manual` oder `agent`.
    pub origin: String,
    pub state: RunState,
    pub dry_run: bool,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub error: Option<String>,
    pub error_code: Option<String>,
    pub wait_reason: Option<String>,
    pub cancel_requested: bool,
}

impl From<&RunRow> for WorkflowRunSummary {
    fn from(r: &RunRow) -> Self {
        Self {
            id: r.id.clone(),
            workflow_id: r.workflow_id.clone(),
            workflow_name: r.workflow_name.clone(),
            origin: r.origin.as_str().to_string(),
            state: r.state,
            dry_run: r.dry_run,
            created_at: r.created_at,
            started_at: r.started_at,
            ended_at: r.ended_at,
            error: r.error.clone(),
            error_code: r.error_code.clone(),
            wait_reason: r.wait_reason.clone(),
            cancel_requested: r.cancel_requested,
        }
    }
}

/// Ein Ablauf in der Liste.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowItem {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// `true`: nicht scharf, jeder Lauf plant nur (Trockenlauf).
    pub dry_run: bool,
    pub updated_at: i64,
    /// Die Definition als JSON-Text (gespeicherte, normalisierte Form).
    pub definition_json: String,
    /// Kennung des Ausloesers (`calendar.event_starting`) und Zahl der Schritte, ohne die
    /// Definition in der Oberflaeche parsen zu muessen.
    pub trigger_kind: String,
    pub step_count: u32,
    pub last_run: Option<WorkflowRunSummary>,
    /// Laeufe, die noch nicht beendet sind (wartend, laufend, auf Freigabe wartend).
    pub open_runs: u32,
}

/// Ergebnis von Speichern und Import: der gespeicherte Ablauf ODER alle Befunde.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowSaveResult {
    pub workflow: Option<WorkflowItem>,
    pub issues: Vec<WorkflowIssue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowStepRow {
    pub step_id: String,
    pub attempt: u32,
    pub ordinal: u32,
    pub action: String,
    /// Klartext des Bausteins aus dem Katalog.
    pub action_title: Option<String>,
    pub state: StepState,
    pub error_class: Option<String>,
    pub input_json: Option<String>,
    pub output_json: Option<String>,
    pub error: Option<String>,
    /// Freigabe, auf die der Schritt wartet (Seite „Integrationen“ / Freigabedialog).
    pub approval_id: Option<String>,
    pub wake_at: Option<i64>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

fn step_row(s: &StepRow) -> WorkflowStepRow {
    WorkflowStepRow {
        step_id: s.step_id.clone(),
        attempt: s.attempt,
        ordinal: s.ordinal,
        action: s.action.clone(),
        action_title: catalog::action_spec(&s.action).map(|a| a.title.to_string()),
        state: s.state,
        error_class: s.error_class.clone(),
        input_json: s.input_json.clone(),
        output_json: s.output_json.clone(),
        error: s.error.clone(),
        approval_id: s.approval_id.clone(),
        wake_at: s.wake_at,
        started_at: s.started_at,
        ended_at: s.ended_at,
    }
}

/// Ein Lauf mit Schritten und Herkunft.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowRunDetail {
    pub run: WorkflowRunSummary,
    /// Daten des Ausloesers und Variablen des Laufs (JSON-Text).
    pub context_json: String,
    /// Die Definition, mit der der Lauf lief (JSON-Text).
    pub definition_json: String,
    /// Alle Versuche aller Schritte, in der Reihenfolge der Schritte, dann nach Versuch.
    pub steps: Vec<WorkflowStepRow>,
    /// Herkunft der Ausgaben des Laufs (Schritt, Modell, Quellen).
    pub provenance: Vec<ProvenanceEntry>,
    /// Der Lauf ist gescheitert und laesst sich wiederholen.
    pub can_retry: bool,
    /// Eine Wiederholung braucht die ausdrueckliche Bestaetigung (Wirkung unklar).
    pub retry_needs_confirmation: bool,
    /// Der Lauf ist noch nicht beendet und laesst sich abbrechen.
    pub can_cancel: bool,
}

/// Beschreibung eines Felds fuer das Formular.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowFieldSpec {
    pub name: String,
    /// `text`, `id`, `int`, `bool`, `text_list`, `choice` oder `any`.
    pub kind: String,
    pub required: bool,
    /// Nur feste Werte, nie `{{...}}` (Empfaenger, Adressen).
    pub literal: bool,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub options: Vec<String>,
    /// Bei Feldern vom Typ `id`: welche Faehigkeit die Integration haben muss (zum Filtern der Auswahl).
    pub capability: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowTriggerSpec {
    pub id: String,
    pub title: String,
    pub fields: Vec<WorkflowFieldSpec>,
    /// Felder, die der Ausloeser unter `trigger.<feld>` liefert (fuer die Hilfe zu Variablen).
    pub provides: Vec<String>,
    /// Startet von selbst (Kalender, Zeitplan, Datei, Kanal) und nicht auf Zuruf.
    pub automatic: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowActionSpec {
    pub id: String,
    pub title: String,
    pub effect_text: String,
    pub fields: Vec<WorkflowFieldSpec>,
    /// `pure`, `idempotent` oder `external`.
    pub effect: String,
    pub heavy_label: Option<String>,
    /// Faehigkeit der Integration, die der Baustein braucht (`mail.send`), sonst leer.
    pub capability: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowCatalog {
    pub schema: String,
    pub max_steps: u32,
    pub triggers: Vec<WorkflowTriggerSpec>,
    pub actions: Vec<WorkflowActionSpec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Die Vorlage als JSON-Text.
    pub definition_json: String,
}

/// Datei, die nur in der Cloud liegt (OneDrive-Platzhalter).
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowCloudFile {
    pub workflow_id: String,
    pub workflow_name: String,
    pub name: String,
}

/// Stand eines beobachteten YouTube-Kanals.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowChannelStatus {
    pub workflow_id: String,
    pub workflow_name: String,
    pub channel_id: String,
    pub last_ok_ms: Option<i64>,
    pub failures: u32,
    pub outage: bool,
    pub last_error: Option<String>,
    pub next_fetch_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowStatus {
    pub cloud_only: Vec<WorkflowCloudFile>,
    pub channels: Vec<WorkflowChannelStatus>,
}

/// Antwort auf einen Start von Hand.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct WorkflowStarted {
    pub run_id: String,
    pub created: bool,
    pub dry_run: bool,
}

// ---------------------------------------------------------------------------
// Fehler
// ---------------------------------------------------------------------------

/// Klartext fuer die Oberflaeche. Befunde einer ungueltigen Definition gehen NICHT hierueber,
/// sondern als `WorkflowSaveResult.issues`.
pub fn error_text(e: &WorkflowError) -> String {
    e.to_string()
}

// ---------------------------------------------------------------------------
// Katalog und Vorlagen
// ---------------------------------------------------------------------------

fn field_spec(f: &FieldSpec, capability: Option<&str>) -> WorkflowFieldSpec {
    let (kind, min, max, options): (&str, Option<i64>, Option<i64>, Vec<String>) = match f.kind {
        FieldKind::Text => ("text", None, None, vec![]),
        FieldKind::Id => ("id", None, None, vec![]),
        FieldKind::Int { min, max } => ("int", Some(min), Some(max), vec![]),
        FieldKind::Bool => ("bool", None, None, vec![]),
        FieldKind::TextList => ("text_list", None, None, vec![]),
        FieldKind::Choice(options) => (
            "choice",
            None,
            None,
            options.iter().map(|o| o.to_string()).collect(),
        ),
        FieldKind::Any => ("any", None, None, vec![]),
    };
    WorkflowFieldSpec {
        name: f.name.to_string(),
        kind: kind.to_string(),
        required: f.required,
        literal: f.literal,
        min,
        max,
        options,
        capability: if matches!(f.kind, FieldKind::Id) {
            capability.map(str::to_string)
        } else {
            None
        },
    }
}

/// Welche Faehigkeit eine Integration fuer das Feld `integration` eines Ausloesers haben muss.
fn trigger_capability(trigger_id: &str) -> Option<&'static str> {
    match trigger_id {
        "calendar.event_starting" | "calendar.event_ended" => Some("calendar.read"),
        "folder.file_added" => Some("files.read"),
        "youtube.channel_new_video" => Some("media.fetch"),
        _ => None,
    }
}

/// Der Katalog fuer das Formular. Reine Beschreibung der statischen Tabellen in `catalog`.
pub fn catalog_view() -> WorkflowCatalog {
    let triggers = catalog::triggers()
        .iter()
        .map(|t| WorkflowTriggerSpec {
            id: t.id.to_string(),
            title: t.title.to_string(),
            fields: t
                .fields
                .iter()
                .map(|f| field_spec(f, trigger_capability(t.id)))
                .collect(),
            provides: t.provides.iter().map(|p| p.to_string()).collect(),
            automatic: super::trigger::is_automatic(t.id),
        })
        .collect();
    let actions = catalog::actions()
        .iter()
        .map(|a| {
            let (cap, via) = match a.needs {
                NeedsSpec::Cap {
                    capability, via, ..
                } => (Some(capability.as_str()), Some(via)),
                _ => (None, None),
            };
            WorkflowActionSpec {
                id: a.id.to_string(),
                title: a.title.to_string(),
                effect_text: a.effect_text.to_string(),
                fields: a
                    .fields
                    .iter()
                    .map(|f| field_spec(f, if Some(f.name) == via { cap } else { None }))
                    .collect(),
                effect: a.effect.as_str().to_string(),
                heavy_label: a.heavy.map(|h| h.label.to_string()),
                capability: cap.map(str::to_string),
            }
        })
        .collect();
    WorkflowCatalog {
        schema: SCHEMA_ID.to_string(),
        max_steps: MAX_STEPS as u32,
        triggers,
        actions,
    }
}

/// Die mitgelieferten Vorlagen (gueltige Definitionen; Kennungen von Integrationen sind
/// Platzhalter, die der Editor waehlen laesst).
pub fn template_list() -> Vec<WorkflowTemplate> {
    templates::all()
        .into_iter()
        .filter_map(|(id, text)| {
            let v: Value = serde_json::from_str(text).ok()?;
            Some(WorkflowTemplate {
                id: id.to_string(),
                name: v["name"].as_str().unwrap_or(id).to_string(),
                description: v["description"].as_str().unwrap_or("").to_string(),
                definition_json: text.to_string(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Ablaeufe
// ---------------------------------------------------------------------------

fn trigger_and_steps(row: &WorkflowRow) -> (String, u32) {
    match serde_json::from_str::<Value>(&row.definition_json) {
        Ok(v) => (
            v["trigger"]["type"].as_str().unwrap_or("").to_string(),
            v["steps"].as_array().map(|a| a.len()).unwrap_or(0) as u32,
        ),
        Err(_) => (String::new(), 0),
    }
}

fn item_of(engine: &Engine, row: &WorkflowRow) -> Result<WorkflowItem, WorkflowError> {
    let (trigger_kind, step_count) = trigger_and_steps(row);
    let filter = store::RunFilter {
        workflow_id: Some(row.id.clone()),
        state: None,
    };
    // Die jeweils juengsten Laeufe genuegen: der letzte und die Zahl der offenen darunter.
    let runs = engine.runs(&filter, 50)?;
    let open_runs = runs.iter().filter(|r| !r.state.is_terminal()).count() as u32;
    Ok(WorkflowItem {
        id: row.id.clone(),
        name: row.name.clone(),
        enabled: row.enabled,
        dry_run: row.dry_run,
        updated_at: row.updated_at,
        definition_json: row.definition_json.clone(),
        trigger_kind,
        step_count,
        last_run: runs.first().map(WorkflowRunSummary::from),
        open_runs,
    })
}

pub fn list(engine: &Engine) -> Result<Vec<WorkflowItem>, WorkflowError> {
    engine
        .workflows()?
        .iter()
        .map(|row| item_of(engine, row))
        .collect()
}

pub fn get(engine: &Engine, id: &str) -> Result<WorkflowItem, WorkflowError> {
    item_of(engine, &engine.workflow(id)?)
}

/// Prueft einen Definitionstext ohne zu speichern: alle Befunde mit JSON-Zeiger.
pub fn validate_text(engine: &Engine, definition_json: &str) -> Vec<WorkflowIssue> {
    if definition_json.len() > MAX_IMPORT_BYTES {
        return vec![WorkflowIssue {
            path: String::new(),
            message: format!(
                "Die Definition ist zu groß (höchstens {} KiB).",
                MAX_IMPORT_BYTES / 1024
            ),
        }];
    }
    let value: Value = match serde_json::from_str(definition_json) {
        Ok(v) => v,
        Err(e) => {
            return vec![WorkflowIssue {
                path: String::new(),
                message: format!(
                    "Kein gültiges JSON (Zeile {}, Spalte {}).",
                    e.line(),
                    e.column()
                ),
            }]
        }
    };
    match engine.check_definition(&value) {
        Ok(_) => vec![],
        Err(issues) => issues_of(&issues),
    }
}

/// Speichert (`id = None`: neuer Ablauf, ausgeschaltet und im Trockenlauf) oder ersetzt.
/// Eine ungueltige Definition ist KEIN Fehler, sondern ein Ergebnis mit allen Befunden.
pub fn save(
    engine: &Engine,
    id: Option<&str>,
    definition_json: &str,
) -> Result<WorkflowSaveResult, WorkflowError> {
    let issues = validate_text(engine, definition_json);
    if !issues.is_empty() {
        return Ok(WorkflowSaveResult {
            workflow: None,
            issues,
        });
    }
    let value: Value = serde_json::from_str(definition_json)
        .map_err(|e| WorkflowError::BadInput(format!("Kein gültiges JSON: {e}")))?;
    match engine.save_workflow(id, &value) {
        Ok(row) => Ok(WorkflowSaveResult {
            workflow: Some(item_of(engine, &row)?),
            issues: vec![],
        }),
        Err(WorkflowError::Invalid(list)) => Ok(WorkflowSaveResult {
            workflow: None,
            issues: issues_of(&list),
        }),
        Err(e) => Err(e),
    }
}

/// Die Definition eines Ablaufs als lesbarer JSON-Text zum Weitergeben (Export). Der Rundlauf
/// Export -> Import ergibt dieselbe Definition (die gespeicherte Form ist schon normalisiert).
pub fn export(engine: &Engine, id: &str) -> Result<String, WorkflowError> {
    let row = engine.workflow(id)?;
    let value: Value = serde_json::from_str(&row.definition_json)
        .map_err(|e| WorkflowError::Store(format!("Definition nicht lesbar: {e}")))?;
    let mut text = serde_json::to_string_pretty(&value)
        .map_err(|e| WorkflowError::Store(format!("Definition nicht lesbar: {e}")))?;
    text.push('\n');
    Ok(text)
}

/// Importiert eine Definition als NEUEN Ablauf (ausgeschaltet, Trockenlauf). Dieselbe Pruefung
/// wie beim Speichern: ein importierter Ablauf kann nichts, was ein selbst gebauter nicht kann.
pub fn import(engine: &Engine, text: &str) -> Result<WorkflowSaveResult, WorkflowError> {
    save(engine, None, text)
}

/// Liest eine Importdatei, hoechstens `MAX_IMPORT_BYTES` (ein groesserer Brocken wird gar nicht
/// erst gelesen) und nur gueltiges UTF-8.
pub fn read_import_file(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let file =
        std::fs::File::open(path).map_err(|e| format!("Die Datei lässt sich nicht öffnen: {e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Die Datei lässt sich nicht lesen: {e}"))?;
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(format!(
            "Die Datei ist zu groß (höchstens {} KiB).",
            MAX_IMPORT_BYTES / 1024
        ));
    }
    String::from_utf8(bytes).map_err(|_| "Die Datei ist kein Text in UTF-8.".to_string())
}

/// Schreibt den Export in eine Datei, die der Nutzer im Speichern-Dialog gewaehlt hat.
pub fn export_to_file(engine: &Engine, id: &str, path: &std::path::Path) -> Result<(), String> {
    let text = export(engine, id).map_err(|e| error_text(&e))?;
    std::fs::write(path, text).map_err(|e| format!("Die Datei lässt sich nicht schreiben: {e}"))
}

pub fn set_enabled(
    engine: &Engine,
    id: &str,
    enabled: bool,
) -> Result<WorkflowItem, WorkflowError> {
    engine.set_enabled(id, enabled)?;
    get(engine, id)
}

pub fn set_armed(engine: &Engine, id: &str, armed: bool) -> Result<WorkflowItem, WorkflowError> {
    engine.set_armed(id, armed)?;
    get(engine, id)
}

pub fn delete(engine: &Engine, id: &str) -> Result<(), WorkflowError> {
    engine.delete_workflow(id)
}

// ---------------------------------------------------------------------------
// Trockenlauf und Start
// ---------------------------------------------------------------------------

/// Trockenlauf einer Definition (auch einer noch nicht gespeicherten): was wuerde jeder Schritt
/// tun, und duerfte er es? Rein rechnerisch, schreibt nichts. Ein JSON-Text:
/// `{"valid": true, ...plan}` oder `{"valid": false, "issues": [...]}`.
pub fn plan_text(
    engine: &Engine,
    conn: &Connection,
    definition_json: &str,
    workflow_id: Option<&str>,
) -> String {
    let issues = validate_text(engine, definition_json);
    if !issues.is_empty() {
        return json!({
            "schema": plan::PLAN_SCHEMA,
            "valid": false,
            "dry_run": true,
            "writes": "nothing",
            "issues": issues
                .iter()
                .map(|i| json!({"path": i.path, "message": i.message}))
                .collect::<Vec<_>>(),
        })
        .to_string();
    }
    let def = match validate::parse_definition_str(definition_json) {
        Ok(d) => d,
        Err(list) => {
            return json!({
                "schema": plan::PLAN_SCHEMA,
                "valid": false,
                "dry_run": true,
                "writes": "nothing",
                "issues": list
                    .iter()
                    .map(|i| json!({"path": i.path, "message": i.message}))
                    .collect::<Vec<_>>(),
            })
            .to_string()
        }
    };
    let registry = engine.registry();
    let mut payload = plan::plan_definition(conn, &registry, &def, workflow_id);
    payload["valid"] = json!(true);
    payload.to_string()
}

/// Startet einen Ablauf von Hand. `dry_run = true`: er plant nur (auch bei einem scharfen
/// Ablauf, auch bei einem ausgeschalteten). Ein echter Lauf verlangt einen eingeschalteten,
/// scharfen Ablauf; sonst weist die Engine ihn ab (`Disabled`) oder plant nur.
pub fn start(
    engine: &Engine,
    workflow_id: &str,
    vars: Map<String, Value>,
    dry_run: bool,
) -> Result<WorkflowStarted, WorkflowError> {
    let e = manual::start(engine, workflow_id, vars, dry_run)?;
    Ok(WorkflowStarted {
        run_id: e.run_id,
        created: e.created,
        dry_run: e.dry_run,
    })
}

// ---------------------------------------------------------------------------
// Laeufe
// ---------------------------------------------------------------------------

pub fn runs(
    engine: &Engine,
    workflow_id: Option<&str>,
    open_only: bool,
    limit: i64,
) -> Result<Vec<WorkflowRunSummary>, WorkflowError> {
    let filter = store::RunFilter {
        workflow_id: workflow_id.map(str::to_string),
        state: None,
    };
    let limit = limit.clamp(1, MAX_RUNS_LISTED);
    // Mit dem Filter „offen“ mehr lesen, damit nach dem Aussortieren noch genug bleibt.
    let rows = engine.runs(&filter, if open_only { 1_000 } else { limit })?;
    Ok(rows
        .iter()
        .filter(|r| !open_only || !r.state.is_terminal())
        .take(limit as usize)
        .map(WorkflowRunSummary::from)
        .collect())
}

pub fn run_detail(engine: &Engine, run_id: &str) -> Result<WorkflowRunDetail, WorkflowError> {
    let d = engine.run_detail(run_id)?;
    let needs_confirmation = matches!(
        d.run.error_code.as_deref(),
        Some(code::EFFECT_UNCERTAIN) | Some(code::CRASH_LOOP)
    );
    let mut steps: Vec<WorkflowStepRow> = d.steps.iter().map(step_row).collect();
    steps.sort_by_key(|s| (s.ordinal, s.attempt));
    Ok(WorkflowRunDetail {
        run: WorkflowRunSummary::from(&d.run),
        context_json: d.run.context_json.clone(),
        definition_json: d.run.definition_json.clone(),
        steps,
        provenance: d.provenance,
        can_retry: d.run.state == RunState::Failed,
        retry_needs_confirmation: needs_confirmation,
        can_cancel: !d.run.state.is_terminal(),
    })
}

/// `true`: sofort abgebrochen; `false`: der Abbruch ist vermerkt, der Lauf endet am naechsten
/// Schrittwechsel.
pub fn cancel_run(engine: &Engine, run_id: &str) -> Result<bool, WorkflowError> {
    Ok(matches!(
        engine.cancel_run(run_id)?,
        CancelResult::Cancelled
    ))
}

pub fn retry_run(
    engine: &Engine,
    run_id: &str,
    accept_uncertain: bool,
) -> Result<(), WorkflowError> {
    engine.retry_run(run_id, accept_uncertain)
}

// ---------------------------------------------------------------------------
// Stand der Ausloeser
// ---------------------------------------------------------------------------

/// Ordner-Platzhalter und Kanal-Ausfaelle fuer die Anzeige. Die Namen der Ablaeufe kommen aus
/// der Engine; ein inzwischen geloeschter Ablauf erscheint mit seiner Kennung.
pub fn status(
    engine: &Engine,
    cloud: Vec<super::trigger::folder::CloudFile>,
    channels: Vec<super::trigger::youtube_channel::ChannelStatus>,
) -> WorkflowStatus {
    let name = |id: &str| {
        engine
            .workflow(id)
            .map(|w| w.name)
            .unwrap_or_else(|_| id.to_string())
    };
    WorkflowStatus {
        cloud_only: cloud
            .into_iter()
            .map(|c| WorkflowCloudFile {
                workflow_name: name(&c.workflow_id),
                workflow_id: c.workflow_id,
                name: c.name,
            })
            .collect(),
        channels: channels
            .into_iter()
            .map(|c| WorkflowChannelStatus {
                workflow_name: name(&c.workflow_id),
                workflow_id: c.workflow_id,
                channel_id: c.channel_id,
                last_ok_ms: c.last_ok_ms,
                failures: c.failures,
                outage: c.outage,
                last_error: c.last_error,
                next_fetch_ms: c.next_fetch_ms,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
