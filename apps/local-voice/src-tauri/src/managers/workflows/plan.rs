//! Trockenlauf (B1, AK2): was WUERDE ein Ablauf tun, und duerfte er es?
//!
//! Der Plan ist reine Rechnung: kein Baustein laeuft, keine Datei entsteht, keine Mail
//! geht, kein Modell startet, es wird nicht einmal ins Audit oder in die Freigaben
//! geschrieben. Gelesen werden nur das Register und seine Rechte
//! (`integrations::grants::explain`, dieselbe reine Regel wie beim echten Lauf).
//!
//! Je Schritt: Bedingung (wahr / falsch / unbekannt, wenn sie von einem Vorschritt
//! abhaengt), eingesetzte Parameter (Verweise auf Ergebnisse von Vorschritten bleiben
//! als `{{...}}` stehen und stehen in `unresolved`), die geplante Wirkung in einem
//! deutschen Satz, Wirkungsart, Bedarf an schwerer Arbeit und das Rechte-Ergebnis:
//! `allowed`, `needs_approval` (mit Vorschau, was der Nutzer sehen wuerde), `denied`
//! (mit Grund) oder `not_required`.
//!
//! Ausloeserdaten stammen nicht von einem echten Ereignis, sondern aus den
//! Beispieldaten des Katalogs (`trigger_sample: true` im Ergebnis): der Plan zeigt, was
//! mit solchen Daten geschaehe.
//!
//! Dieselbe Planung liefert die Schrittzeilen eines Laufs im Trockenlauf
//! (`engine`): Plan und Laufprotokoll koennen nicht auseinanderlaufen.

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::managers::integrations::grants::explain;
use crate::managers::integrations::model::{Caller, GrantMode};
use crate::managers::integrations::{preview, store as register};

use super::action::{ActionRegistry, EffectKind, NeedsError};
use super::catalog;
use super::expr::{self, Path};
use super::model::{StepDef, WorkflowDef};

/// Kennung des Plan-Formats in der Ausgabe.
pub const PLAN_SCHEMA: &str = "lva-workflow-plan@1";

fn deferred_refs(refs: &[Path]) -> Vec<String> {
    refs.iter()
        .filter(|p| p.root() == "steps")
        .map(|p| p.to_string())
        .collect()
}

/// Ausgangskontext fuer Plan und Trockenlauf: Beispieldaten des Ausloesers und
/// Vorgabewerte der Variablen (Pflichtvariablen ohne Vorgabe: ein Platzhaltertext).
pub fn base_context(def: &WorkflowDef, workflow_id: &str, run_id: &str) -> (Value, Vec<String>) {
    let trigger = catalog::trigger_spec(&def.trigger.kind)
        .map(|s| (s.sample)(&def.trigger.params))
        .unwrap_or_else(|| json!({}));
    let mut vars = Map::new();
    let mut missing = Vec::new();
    for (name, decl) in &def.variables {
        match &decl.default {
            Some(d) if !d.is_null() => {
                vars.insert(name.clone(), d.clone());
            }
            _ => {
                missing.push(name.clone());
                vars.insert(name.clone(), json!(format!("(Beispielwert für {name})")));
            }
        }
    }
    let mut ctx = json!({
        "run": {"id": run_id, "dry_run": true, "started_at": 0},
        "workflow": {"id": workflow_id, "name": def.name},
        "trigger": trigger,
        "vars": Value::Object(vars),
        "steps": {}
    });
    if let Some(m) = ctx["trigger"].get("meeting").cloned() {
        ctx["meeting"] = m;
    }
    (ctx, missing)
}

/// Plant einen Schritt gegen den Kontext `ctx`.
pub fn plan_step(
    conn: &Connection,
    registry: &ActionRegistry,
    step: &StepDef,
    index: usize,
    ctx: &Value,
) -> Value {
    let spec = catalog::action_spec(&step.action);
    let mut out = json!({
        "index": index,
        "id": step.id,
        "action": step.action,
        "title": spec.map(|s| s.title).unwrap_or("(unbekannter Baustein)"),
        "label": step.label,
        "status": "planned",
    });

    // Bedingung
    let mut condition = json!({"expression": step.when, "result": "true"});
    if let Some(when) = &step.when {
        match expr::parse_condition(when) {
            Err(e) => {
                condition["result"] = json!("error");
                condition["message"] = json!(e.to_string());
                out["status"] = json!("invalid");
            }
            Ok(e) => {
                if !deferred_refs(&e.refs()).is_empty() {
                    condition["result"] = json!("unknown");
                    condition["message"] =
                        json!("Hängt vom Ergebnis eines Vorschritts ab und lässt sich erst beim Lauf entscheiden.");
                } else {
                    match e.eval_bool(ctx) {
                        Ok(true) => condition["result"] = json!("true"),
                        Ok(false) => {
                            condition["result"] = json!("false");
                            out["status"] = json!("skipped");
                        }
                        Err(err) => {
                            condition["result"] = json!("error");
                            condition["message"] = json!(err.to_string());
                            out["status"] = json!("invalid");
                        }
                    }
                }
            }
        }
    }
    out["condition"] = condition;

    // Parameter: je Feld einsetzen; Verweise auf Vorschritte bleiben stehen.
    let mut params = Map::new();
    let mut unresolved: Vec<String> = Vec::new();
    let mut param_errors: Vec<String> = Vec::new();
    for (key, value) in &step.params {
        match expr::collect_value_refs(value) {
            Ok(refs) => {
                let later = deferred_refs(&refs);
                if later.is_empty() {
                    match expr::render_value(value, ctx) {
                        Ok(v) => {
                            params.insert(key.clone(), v);
                        }
                        Err(e) => {
                            param_errors.push(format!("{key}: {e}"));
                            params.insert(key.clone(), value.clone());
                        }
                    }
                } else {
                    unresolved.extend(later);
                    params.insert(key.clone(), value.clone());
                }
            }
            Err((_, e)) => {
                param_errors.push(format!("{key}: {e}"));
                params.insert(key.clone(), value.clone());
            }
        }
    }
    unresolved.sort();
    unresolved.dedup();
    let params = Value::Object(params);
    out["params"] = params.clone();
    out["unresolved"] = json!(unresolved);
    if !param_errors.is_empty() {
        out["status"] = json!("invalid");
        out["param_errors"] = json!(param_errors);
    }

    // Baustein: Wirkung, Art, Bedarf, Recht
    let Some(action) = registry.get(&step.action) else {
        out["status"] = json!("invalid");
        out["effect"] = json!("Unbekannter Baustein.");
        out["permission"] = json!({"required": false, "result": "invalid",
            "message": "Unbekannter Baustein."});
        return out;
    };
    out["effect"] = json!(action.describe(&params));
    out["effect_kind"] = json!(action.effect().as_str());
    out["heavy"] = match action.heavy(&params) {
        Some(h) => json!({"label": h.label, "ram_mb": h.ram_mb}),
        None => Value::Null,
    };
    out["permission"] = permission_of(conn, action.as_ref(), &params);
    out
}

fn permission_of(conn: &Connection, action: &dyn super::action::Action, params: &Value) -> Value {
    let needs = match action.needs(params) {
        Ok(None) => {
            return json!({"required": false, "result": "not_required"});
        }
        Ok(Some(n)) => n,
        Err(NeedsError::Invalid(m)) => {
            return json!({"required": true, "result": "invalid", "message": m});
        }
        Err(NeedsError::Unmodeled(m)) => {
            return json!({"required": true, "result": "denied",
                "reason": "capability_not_modeled", "message": m});
        }
    };
    let mut perm = json!({
        "required": true,
        "integration": needs.integration_id,
        "capability": needs.capability.as_str(),
        "target": needs.target,
    });
    let integration = match register::get(conn, &needs.integration_id) {
        Ok(Some(i)) => i,
        Ok(None) => {
            perm["result"] = json!("denied");
            perm["reason"] = json!("unknown_integration");
            perm["message"] = json!("Die Integration gibt es nicht.");
            return perm;
        }
        Err(e) => {
            perm["result"] = json!("invalid");
            perm["message"] = json!(e.to_string());
            return perm;
        }
    };
    let grants = match register::grants_for(conn, &integration.id) {
        Ok(g) => g,
        Err(e) => {
            perm["result"] = json!("invalid");
            perm["message"] = json!(e.to_string());
            return perm;
        }
    };
    let (mode, reason) = explain(&integration, needs.capability, Caller::Workflow, &grants, None);
    perm["mode"] = json!(mode.as_str());
    match mode {
        GrantMode::Allow => perm["result"] = json!("allowed"),
        GrantMode::Off => {
            perm["result"] = json!("denied");
            if let Some(r) = reason {
                perm["reason"] = json!(r.as_str());
                perm["message"] = json!(r.message());
            }
        }
        GrantMode::Ask => match preview::build(needs.target.as_deref(), Some(params)) {
            Ok(text) => {
                perm["result"] = json!("needs_approval");
                perm["preview"] = json!(text);
            }
            Err(e) => {
                perm["result"] = json!("denied");
                perm["reason"] = json!("preview_unsafe");
                perm["message"] = json!(e.to_string());
            }
        },
    }
    perm
}

/// Plant einen ganzen Ablauf (CLI `--workflow-run --dry-run`).
pub fn plan_definition(
    conn: &Connection,
    registry: &ActionRegistry,
    def: &WorkflowDef,
    workflow_id: Option<&str>,
) -> Value {
    let (mut ctx, missing_vars) = base_context(def, workflow_id.unwrap_or("(Datei)"), "dry-run");
    let mut steps = Vec::new();
    for (i, step) in def.steps.iter().enumerate() {
        let planned = plan_step(conn, registry, step, i, &ctx);
        // Spaetere Schritte sehen den Vorschritt als "geplant", ohne Ergebnisfelder.
        let status = planned["status"].as_str().unwrap_or("planned").to_string();
        ctx["steps"][step.id.as_str()] = json!({"status": status, "ok": status == "planned"});
        steps.push(planned);
    }
    let count = |key: &str, val: &str| {
        steps
            .iter()
            .filter(|s| s[key].as_str() == Some(val))
            .count()
    };
    let perm_count = |val: &str| {
        steps
            .iter()
            .filter(|s| s["status"].as_str() != Some("skipped"))
            .filter(|s| s["permission"]["result"].as_str() == Some(val))
            .count()
    };
    let denied = perm_count("denied");
    let invalid = count("status", "invalid") + perm_count("invalid");
    let summary = json!({
        "steps": steps.len(),
        "planned": count("status", "planned"),
        "skipped": count("status", "skipped"),
        "invalid": invalid,
        "allowed": perm_count("allowed"),
        "needs_approval": perm_count("needs_approval"),
        "denied": denied,
        "not_required": perm_count("not_required"),
        "heavy": steps.iter().filter(|s| !s["heavy"].is_null()).count(),
        "external_effects": steps
            .iter()
            .filter(|s| s["effect_kind"].as_str() == Some(EffectKind::External.as_str()))
            .count(),
        "would_run_without_intervention": denied == 0 && invalid == 0,
    });
    json!({
        "schema": PLAN_SCHEMA,
        "dry_run": true,
        "writes": "nothing",
        "workflow": {
            "id": workflow_id,
            "name": def.name,
            "description": def.description,
            "schema": def.schema,
        },
        "trigger": {
            "type": def.trigger.kind,
            "config": def.trigger.params,
        },
        "trigger_sample": true,
        "variables_without_default": missing_vars,
        "steps": steps,
        "summary": summary,
    })
}

/// Lesbare Fassung des Plans (CLI ohne `--json`).
pub fn format_table(plan: &Value) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Trockenlauf: {}\n",
        plan["workflow"]["name"].as_str().unwrap_or("?")
    ));
    out.push_str(&format!(
        "Auslöser: {} (Beispieldaten)\n\n",
        plan["trigger"]["type"].as_str().unwrap_or("?")
    ));
    for s in plan["steps"].as_array().into_iter().flatten() {
        let perm = &s["permission"];
        out.push_str(&format!(
            "{:>2}. {:<14} {:<10} Recht: {:<15} {}\n",
            s["index"].as_u64().unwrap_or(0) + 1,
            s["id"].as_str().unwrap_or("?"),
            s["status"].as_str().unwrap_or("?"),
            perm["result"].as_str().unwrap_or("?"),
            s["effect"].as_str().unwrap_or("")
        ));
    }
    let sum = &plan["summary"];
    out.push_str(&format!(
        "\n{} Schritte: {} erlaubt, {} fragen, {} abgelehnt, {} ungültig\n",
        sum["steps"],
        sum["allowed"],
        sum["needs_approval"],
        sum["denied"],
        sum["invalid"]
    ));
    out
}

#[cfg(test)]
mod tests;
