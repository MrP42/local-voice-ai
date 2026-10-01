//! Oberflaeche der KI-Schritte (Goal Lokaler Agent, Issue #68, Paket C5): der Werkzeugkatalog
//! fuer den Editor und die Vorschau mit Modellentscheidung.
//!
//! - [`tools`]: die Werkzeuge der Politik (`agent::policy::catalog`) mit ihren Feldern, damit der
//!   Editor die Whitelist als Mehrfachauswahl zeigt. Ein neues Werkzeug erscheint ohne Aenderung
//!   der Oberflaeche.
//! - [`preview`]: „Mit Beispieltext ausprobieren“. Der Entwurf aus dem Editor (auch ungespeichert)
//!   und die Kennung des Schritts; das Ergebnis ist die Entscheidung des Modells (gewaehltes
//!   Werkzeug, Argumente, Begruendung, Konfidenz) mit `dry_run: true` und `writes: "nothing"`.
//!
//! # Keine Wirkung
//!
//! Die Vorschau ruft ausschliesslich `agent_route::preview` bzw. `agent_actions::preview_extract`:
//! kein Lauf, kein Journal, keine Provenienz, kein Audit, keine Freigabe, kein Folgeschritt, kein
//! Schreiben in Vault oder Kalender. Sie nimmt den Platz am `HeavyGate` (belegt: Hinweis
//! `busy`, das Modell wird nicht gefragt) und startet den Server nur ueber den Modellverwalter
//! (RAM-Start-Tor, `process_guard`). Bei vollem Arbeitsspeicher meldet das Tor `busy` mit dem Grund;
//! der Rechner bleibt bedienbar. Der Aufruf bricht nach der Zeitgrenze der Laufzeit ab.
//!
//! # Ergebnis (JSON-Text)
//!
//! | Feld | Inhalt |
//! |------|--------|
//! | `kind` | `route` oder `extract` |
//! | `dry_run`, `writes` | immer `true` und `"nothing"` |
//! | `busy` | `true`: der schwere Platz ist belegt, `retry_after_ms` und `message` sagen warum |
//! | `skipped`, `reason`, `reason_text` | keine Entscheidung moeglich (Kontext haengt von einem Vorschritt ab, Beispieltext fehlt) |
//! | `outcome`, `tool`, `arguments`, `recipients`, `would_run`, `provenance` | wie die Ausgabe des Bausteins (`steps.<id>`), siehe `agent_route` |
//! | `todos`, `deadlines`, `decisions`, `counts`, `provenance` | wie die Ausgabe von `agent.extract` |

use serde::Serialize;
use serde_json::{json, Map, Value};
use specta::Type;

use crate::agent::policy::{self, ParamKind};

use super::action::StepError;
use super::agent_actions;
use super::agent_route;
use super::app_actions::AppServices;
use super::engine::Engine;
use super::expr;
use super::heavy::HeavyGate;
use super::model::StepDef;
use super::plan;
use super::validate;

// ---------------------------------------------------------------------------
// Werkzeugkatalog
// ---------------------------------------------------------------------------

/// Ein Argument eines Werkzeugs, das das Modell fuellt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Type)]
pub struct WorkflowAgentToolParam {
    pub key: String,
    /// `text` oder `date` (Zeitangabe, das Datum rechnet der Code).
    pub kind: String,
    pub required: bool,
    pub description: String,
    /// Hoechstzahl Zeichen (nur `text`).
    pub max_chars: Option<u32>,
}

/// Ein Werkzeug, das ein `agent.route`-Schritt anbieten darf.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Type)]
pub struct WorkflowAgentTool {
    pub name: String,
    /// Der Baustein, der es ausfuehrt (hat sein eigenes Recht und seine Freigabe).
    pub action: String,
    pub description: String,
    /// `true`: geht an andere; dann ist die Empfaengerregel Pflicht.
    pub sends_mail: bool,
    pub params: Vec<WorkflowAgentToolParam>,
}

/// Die Werkzeuge der Politik fuer den Editor.
pub fn tools() -> Vec<WorkflowAgentTool> {
    policy::catalog()
        .iter()
        .map(|t| WorkflowAgentTool {
            name: t.name.to_string(),
            action: t.action.to_string(),
            description: t.description.to_string(),
            sends_mail: t.sends_mail,
            params: t
                .params
                .iter()
                .map(|p| {
                    let (kind, max_chars) = match p.kind {
                        ParamKind::Text { max, .. } => ("text", Some(max as u32)),
                        ParamKind::Date { .. } => ("date", None),
                    };
                    WorkflowAgentToolParam {
                        key: p.key.to_string(),
                        kind: kind.to_string(),
                        required: p.required,
                        description: p.description.to_string(),
                        max_chars,
                    }
                })
                .collect(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Vorschau
// ---------------------------------------------------------------------------

/// Die Parameter des Schritts so, wie der Trockenlauf des Plans sie zeigt: Verweise auf Ausloeser
/// und Variablen eingesetzt (Beispieldaten), Verweise auf Vorschritte bleiben als `{{...}}` stehen.
fn render_params(step: &StepDef, ctx: &Value) -> Result<Value, String> {
    let mut params = Map::new();
    for (key, value) in &step.params {
        let refs = expr::collect_value_refs(value).map_err(|(_, e)| format!("{key}: {e}"))?;
        let later = refs.iter().any(|p| p.root() == "steps");
        if later {
            params.insert(key.clone(), value.clone());
        } else {
            let rendered = expr::render_value(value, ctx).map_err(|e| format!("{key}: {e}"))?;
            params.insert(key.clone(), rendered);
        }
    }
    Ok(Value::Object(params))
}

/// Der Hinweis „Platz belegt“ als Ergebnis (kein Fehler: der Nutzer versucht es gleich nochmal).
fn busy(kind: &str, retry_after_ms: u64, reason: &str) -> Value {
    json!({
        "kind": kind,
        "dry_run": true,
        "writes": "nothing",
        "busy": true,
        "retry_after_ms": retry_after_ms,
        "message": reason,
    })
}

/// Woher der Entwurf kommt: der Text aus dem Editor oder ein gespeicherter Ablauf.
pub struct Target<'a> {
    pub workflow_id: Option<&'a str>,
    pub definition_json: Option<&'a str>,
    pub step_id: &'a str,
}

/// Die Modellentscheidung eines `agent.route`-/`agent.extract`-Schritts zum Ausprobieren, als
/// JSON-Text (siehe Moduldoku). Fehler sind Klartext: unbekannter Schritt, kein KI-Schritt,
/// ungueltige Parameter, Modell nicht eingerichtet, Server nicht erreichbar.
#[allow(clippy::too_many_arguments)]
pub fn preview(
    engine: &Engine,
    services: &dyn AppServices,
    gate: &dyn HeavyGate,
    target: &Target<'_>,
    sample_text: Option<&str>,
    now_ms: i64,
    cancel: &dyn Fn() -> bool,
) -> Result<String, String> {
    let text = match (target.definition_json, target.workflow_id) {
        (Some(t), _) if !t.trim().is_empty() => t.to_string(),
        (_, Some(id)) => engine
            .workflow(id)
            .map(|w| w.definition_json)
            .map_err(|e| e.to_string())?,
        _ => return Err("Es fehlt der Ablauf oder sein Entwurf.".to_string()),
    };
    let def = validate::parse_definition_str(&text).map_err(|issues| {
        issues
            .iter()
            .map(|i| i.message.clone())
            .collect::<Vec<_>>()
            .join(" ")
    })?;
    let index = def
        .steps
        .iter()
        .position(|s| s.id == target.step_id)
        .ok_or_else(|| {
            format!(
                "Der Schritt „{}“ ist im Entwurf nicht vorhanden.",
                target.step_id
            )
        })?;
    let step = &def.steps[index];
    if step.action != "agent.route" && step.action != "agent.extract" {
        return Err(
            "Die Vorschau gibt es nur für die KI-Schritte „Werkzeug wählen“ und „Extrahieren“."
                .to_string(),
        );
    }

    // Kontext wie im Plan: Beispieldaten des Ausloesers, Vorschritte nur als „geplant“.
    let (mut ctx, _) =
        plan::base_context(&def, target.workflow_id.unwrap_or("(Entwurf)"), "preview");
    for earlier in &def.steps[..index] {
        ctx["steps"][earlier.id.as_str()] = json!({"status": "planned", "ok": true});
    }
    let params = render_params(step, &ctx)?;
    let sample = sample_text.map(str::trim).filter(|s| !s.is_empty());

    let outcome = if step.action == "agent.route" {
        agent_route::preview(services, gate, &ctx, &params, now_ms, sample, cancel).map(|mut v| {
            v["kind"] = json!("route");
            v
        })
    } else {
        agent_actions::preview_extract(services, gate, &params, sample, now_ms, cancel)
    };
    let kind = if step.action == "agent.route" {
        "route"
    } else {
        "extract"
    };
    match outcome {
        Ok(v) => Ok(v.to_string()),
        Err(StepError::Defer {
            retry_after_ms,
            reason,
        }) => Ok(busy(kind, retry_after_ms, &reason).to_string()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests;
