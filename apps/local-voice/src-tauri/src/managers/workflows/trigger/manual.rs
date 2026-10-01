//! Ausloeser „Von Hand“ (B2): ein Klick in der Oberflaeche startet den Ablauf jetzt.
//!
//! Der manuelle Start geht durch dieselbe `enqueue` wie jeder Ausloeser, mit zwei
//! Unterschieden: jeder Start hat einen EIGENEN Schluessel (`manual:<ulid>`, also nie
//! ein Duplikat: wer zweimal klickt, will zwei Laeufe), und er darf Werte fuer die
//! deklarierten Variablen mitgeben. Ein Trockenlauf (`dry_run`) geht auch bei einem
//! ausgeschalteten Ablauf (zum Ausprobieren); ein echter Lauf nur bei einem eingeschalteten,
//! scharfen. Die Einwilligung zur Aufnahme gilt wie ueberall (`recording`).
//!
//! Die Oberflaeche (B7) und die Agentenbruecke (B8, Herkunft `agent`) rufen `start`.

use serde_json::{Map, Value};

use super::super::engine::{EnqueueRequest, Enqueued};
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
    let mut req = EnqueueRequest::manual(workflow_id);
    req.vars = vars;
    req.force_dry_run = dry_run;
    sink.enqueue(&req)
}

#[cfg(test)]
mod tests;
