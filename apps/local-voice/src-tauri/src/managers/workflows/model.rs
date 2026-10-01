//! Datentypen des Workflow-Moduls (B1): die Definition `lva-workflow@1` und die
//! Zustaende von Lauf und Schritt.
//!
//! Die Definition ist reines JSON ohne Code: Ausloeser, Variablen, eine lineare
//! Liste von Schritten, je Schritt Bedingung (`when`), Parameter, Fehlerregel und
//! Wiederholung. Pruefung auf Gueltigkeit macht `validate`, nicht serde allein:
//! serde faengt Form und unbekannte Felder ab, `validate` Bezuege, Grenzen und den
//! Katalog der Bausteine.
//!
//! Zeitstempel sind Millisekunden UTC.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Kennung des Schemas in `"schema"` jeder Definition.
pub const SCHEMA_ID: &str = "lva-workflow@1";
/// Schemaversion in der Spalte `workflows.schema_version`.
pub const SCHEMA_VERSION: i64 = 1;

/// Hoechstzahl Schritte in einem Ablauf (lineare Abfolge, kein Graph).
pub const MAX_STEPS: usize = 50;
/// Hoechstzahl Variablen.
pub const MAX_VARIABLES: usize = 32;
/// Groesste Definition in Bytes (Text als gespeichert).
pub const MAX_DEFINITION_BYTES: usize = 256 * 1024;
/// Groesste Parameterangabe eines Schritts in Bytes.
pub const MAX_PARAMS_BYTES: usize = 64 * 1024;
pub const MAX_NAME_CHARS: usize = 120;
pub const MAX_TEXT_CHARS: usize = 500;

/// Wiederholungsvorgabe: bei voruebergehenden Fehlern drei Versuche, 2 s, 4 s, ...
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;
pub const DEFAULT_BACKOFF_MS: u64 = 2_000;
pub const MAX_ATTEMPTS_LIMIT: u32 = 10;
pub const MIN_BACKOFF_MS: u64 = 100;
pub const MAX_BACKOFF_MS: u64 = 3_600_000;
/// Mehr als eine Viertelstunde wartet eine Wiederholung nie, egal wie oft sie schon lief.
pub const BACKOFF_CAP_MS: u64 = 15 * 60 * 1000;

// ---------------------------------------------------------------------------
// Definition
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDef {
    pub schema: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub trigger: TriggerDef,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variables: BTreeMap<String, VariableDecl>,
    pub steps: Vec<StepDef>,
}

/// Ausloeser: `type` plus die Felder, die der Katalog (`catalog::trigger_spec`) fuer
/// diese Art kennt. Unbekannte Felder lehnt `validate` mit Pfad ab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriggerDef {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub params: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepDef {
    pub id: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Bedingung (`expr`): Schritt laeuft nur, wenn sie wahr ist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
    #[serde(default, skip_serializing_if = "OnError::is_default")]
    pub on_error: OnError,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
}

impl StepDef {
    /// Die wirksame Wiederholungsregel (Vorgabe, wenn keine angegeben ist).
    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry.clone().unwrap_or_default()
    }
}

/// Was nach einem endgueltig gescheiterten (oder abgelehnten) Schritt geschieht.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnError {
    /// Der Lauf endet als gescheitert (Vorgabe).
    #[default]
    Fail,
    /// Der Schritt gilt als erledigt-mit-Fehler, der Lauf geht weiter
    /// (`steps.<id>.ok` ist dann `false`).
    Continue,
}

impl OnError {
    pub fn is_default(&self) -> bool {
        *self == OnError::Fail
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    /// Versuche insgesamt bei voruebergehenden Fehlern (1 = keine Wiederholung).
    #[serde(default = "default_attempts")]
    pub max_attempts: u32,
    /// Wartezeit vor dem zweiten Versuch; jede weitere verdoppelt sich (gedeckelt).
    #[serde(default = "default_backoff")]
    pub backoff_ms: u64,
}

fn default_attempts() -> u32 {
    DEFAULT_MAX_ATTEMPTS
}

fn default_backoff() -> u64 {
    DEFAULT_BACKOFF_MS
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            backoff_ms: DEFAULT_BACKOFF_MS,
        }
    }
}

impl RetryPolicy {
    /// Wartezeit vor Versuch `next_attempt` (>= 2): `backoff * 2^(n-2)`, gedeckelt.
    pub fn delay_before(&self, next_attempt: u32) -> u64 {
        let shifts = next_attempt.saturating_sub(2).min(20);
        self.backoff_ms
            .saturating_mul(1u64 << shifts)
            .min(BACKOFF_CAP_MS)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VarType {
    String,
    Number,
    Bool,
    List,
}

impl VarType {
    pub fn as_str(self) -> &'static str {
        match self {
            VarType::String => "string",
            VarType::Number => "number",
            VarType::Bool => "bool",
            VarType::List => "list",
        }
    }

    /// Passt der Wert zur Art? `list` nimmt nur Listen aus Text, Zahlen und Ja/Nein.
    pub fn accepts(self, v: &Value) -> bool {
        match (self, v) {
            (VarType::String, Value::String(_)) => true,
            (VarType::Number, Value::Number(_)) => true,
            (VarType::Bool, Value::Bool(_)) => true,
            (VarType::List, Value::Array(items)) => items
                .iter()
                .all(|i| matches!(i, Value::String(_) | Value::Number(_) | Value::Bool(_))),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariableDecl {
    #[serde(rename = "type")]
    pub ty: VarType,
    /// Fehlt der Vorgabewert, muss ihn jeder Start (manuell, Agent) mitgeben.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Zustaende
// ---------------------------------------------------------------------------

/// Zustand eines Laufs (Spalte `workflow_runs.state`, CHECK in der Migration).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Wartet auf einen freien Platz oder (mit `next_run_at`) auf einen Zeitpunkt.
    Queued,
    /// Ein Arbeiter haelt den Lauf (Mietvertrag `lease_until`).
    Running,
    /// Wartet auf die Entscheidung des Nutzers.
    AwaitingApproval,
    Done,
    Failed,
    Cancelled,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Queued => "queued",
            RunState::Running => "running",
            RunState::AwaitingApproval => "awaiting_approval",
            RunState::Done => "done",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => RunState::Queued,
            "running" => RunState::Running,
            "awaiting_approval" => RunState::AwaitingApproval,
            "done" => RunState::Done,
            "failed" => RunState::Failed,
            "cancelled" => RunState::Cancelled,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunState::Done | RunState::Failed | RunState::Cancelled
        )
    }
}

/// Zustand eines Schrittversuchs (Spalte `workflow_run_steps.state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    /// Der Baustein laeuft oder lief, als die App endete: Wirkung unbestimmt.
    Running,
    Done,
    /// Endgueltig gescheitert (nicht mehr wiederholt).
    Failed,
    /// Vorubergehend gescheitert, ein weiterer Versuch ist eingeplant.
    Retrying,
    /// Das Recht fehlt (aus) oder die Freigabe wurde verweigert/ist verfallen.
    Denied,
    /// Bedingung falsch.
    Skipped,
    /// Trockenlauf: geplant, nichts getan.
    Planned,
    AwaitingApproval,
    /// Der Baustein hat auf spaeter verschoben (`wake_at`).
    Waiting,
    /// Absturz oder Zeitueberschreitung mit Aussenwirkung: unklar, ob sie eintrat.
    Uncertain,
    /// Die App endete mitten im Schritt; er wurde neu eingeplant (ohne Aussenwirkung).
    Interrupted,
}

impl StepState {
    pub fn as_str(self) -> &'static str {
        match self {
            StepState::Running => "running",
            StepState::Done => "done",
            StepState::Failed => "failed",
            StepState::Retrying => "retrying",
            StepState::Denied => "denied",
            StepState::Skipped => "skipped",
            StepState::Planned => "planned",
            StepState::AwaitingApproval => "awaiting_approval",
            StepState::Waiting => "waiting",
            StepState::Uncertain => "uncertain",
            StepState::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "running" => StepState::Running,
            "done" => StepState::Done,
            "failed" => StepState::Failed,
            "retrying" => StepState::Retrying,
            "denied" => StepState::Denied,
            "skipped" => StepState::Skipped,
            "planned" => StepState::Planned,
            "awaiting_approval" => StepState::AwaitingApproval,
            "waiting" => StepState::Waiting,
            "uncertain" => StepState::Uncertain,
            "interrupted" => StepState::Interrupted,
            _ => return None,
        })
    }
}

/// Maschinenlesbarer Grund, warum ein Lauf scheiterte (Spalte `error_code`).
pub mod code {
    /// Das Recht fehlt oder die Freigabe wurde verweigert.
    pub const DENIED: &str = "denied";
    pub const APPROVAL_EXPIRED: &str = "approval_expired";
    /// Dauerhafter Fehler des Bausteins (oder eine nicht aufloesbare Variable).
    pub const PERMANENT: &str = "permanent";
    /// Alle Versuche bei voruebergehenden Fehlern verbraucht.
    pub const RETRIES_EXHAUSTED: &str = "retries_exhausted";
    /// Aussenwirkung unklar (Absturz/Zeitueberschreitung): nie von selbst wiederholt.
    pub const EFFECT_UNCERTAIN: &str = "effect_uncertain";
    /// Derselbe Schritt hat die App mehrfach beendet.
    pub const CRASH_LOOP: &str = "crash_loop";
    /// Die gespeicherte Definition ist nicht mehr gueltig.
    pub const INVALID_DEFINITION: &str = "invalid_definition";
    /// Der Baustein ist (noch) nicht eingebaut.
    pub const NOT_AVAILABLE: &str = "not_available";
}

/// Woher ein Lauf kam.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Trigger,
    Manual,
    Agent,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Trigger => "trigger",
            Origin::Manual => "manual",
            Origin::Agent => "agent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "trigger" => Origin::Trigger,
            "manual" => Origin::Manual,
            "agent" => Origin::Agent,
            _ => return None,
        })
    }
}
