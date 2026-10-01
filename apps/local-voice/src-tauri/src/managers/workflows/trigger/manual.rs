//! Ausloeser „Von Hand“ (B2): ein Klick in der Oberflaeche startet den Ablauf jetzt.
//!
//! Der manuelle Start geht durch dieselbe `enqueue` wie jeder Ausloeser, mit zwei
//! Unterschieden: jeder Start hat einen EIGENEN Schluessel (`manual:<ulid>`, also nie
//! ein Duplikat: wer zweimal klickt, will zwei Laeufe), und er darf Werte fuer die
//! deklarierten Variablen mitgeben. Ein Trockenlauf (`dry_run`) geht auch bei einem
//! ausgeschalteten Ablauf (zum Ausprobieren); ein echter Lauf nur bei einem eingeschalteten,
//! scharfen. Die Einwilligung zur Aufnahme gilt wie ueberall (`recording`).
//!
//! Die Oberflaeche (B7) ruft `start`, die Agentenbruecke (B8) `start_as` mit der Herkunft
//! `agent` (Schluessel `agent:<zugang>:<anfrage>`; mit einer Anfragekennung des Agenten ist
//! ein wiederholter Aufruf derselbe Lauf, ohne sie ist jeder Aufruf ein eigener).

use serde_json::{Map, Value};
use ulid::Ulid;

use super::super::engine::{EnqueueRequest, Enqueued};
use super::super::model::Origin;
use super::super::store::WorkflowError;
use super::RunSink;

/// Startet `workflow_id` von Hand. `dry_run = true` erzwingt den Trockenlauf auch bei einem
/// scharfen Ablauf.
pub fn start(
    sink: &dyn RunSink,
    workflow_id: &str,
    vars: Map<String, Value>,
    dry_run: bool,
) -> Result<Enqueued, WorkflowError> {
    start_as(sink, workflow_id, vars, dry_run, Origin::Manual, None, None)
}

/// Wie `start`, mit Herkunft. `actor`: wer startet (Kennung des Agentenzugangs), `request_id`:
/// Idempotenzschluessel des Aufrufers (gleiche Angabe = gleicher Lauf). Fuer `Origin::Trigger`
/// gibt es diesen Weg nicht: ein Ausloeser bildet seinen Schluessel selbst.
pub fn start_as(
    sink: &dyn RunSink,
    workflow_id: &str,
    vars: Map<String, Value>,
    dry_run: bool,
    origin: Origin,
    actor: Option<&str>,
    request_id: Option<&str>,
) -> Result<Enqueued, WorkflowError> {
    if origin == Origin::Trigger {
        return Err(WorkflowError::BadInput(
            "Ein Auslöser startet nicht von Hand.".to_string(),
        ));
    }
    let mut req = EnqueueRequest::manual(workflow_id);
    req.origin = origin;
    if origin != Origin::Manual {
        let unique = request_id.map_or_else(|| Ulid::new().to_string(), str::to_string);
        req.trigger_key = format!("{}:{}:{unique}", origin.as_str(), actor.unwrap_or("-"));
    }
    req.vars = vars;
    req.force_dry_run = dry_run;
    sink.enqueue(&req)
}

#[cfg(test)]
mod tests;
