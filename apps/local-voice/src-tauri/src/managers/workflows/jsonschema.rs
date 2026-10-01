//! JSON-Schema (Draft 2020-12) der Definition `lva-workflow@1`, aus dem Katalog
//! erzeugt (B1).
//!
//! So kann es nie vom Katalog abweichen: eine neue Art oder ein neuer Baustein
//! erscheint hier ohne weitere Pflege. Die eingecheckte Datei
//! `schema/lva-workflow-1.schema.json` ist die Abgleichkopie fuer Editoren und
//! externe Agenten; ein Test vergleicht sie mit dem Erzeugten
//! (`LVA_UPDATE_SCHEMA=1 cargo test --lib workflows::jsonschema` schreibt sie neu).
//!
//! Das Schema beschreibt die FORM. Bezuege (`steps.<id>` nur auf fruehere Schritte,
//! deklarierte Variablen, parsbare Ausdruecke) prueft `validate`; kein JSON-Schema
//! kann das.

use serde_json::{json, Map, Value};

use super::catalog::{self, FieldKind, FieldSpec};
use super::model::{
    MAX_ATTEMPTS_LIMIT, MAX_BACKOFF_MS, MAX_NAME_CHARS, MAX_STEPS, MAX_TEXT_CHARS, MAX_VARIABLES,
    MIN_BACKOFF_MS, SCHEMA_ID,
};

const IDENT_PATTERN: &str = "^[a-z][a-z0-9_]{0,31}$";
const REGISTER_ID_PATTERN: &str = "^[A-Za-z0-9_-]{1,40}$";
const TEMPLATE_PATTERN: &str = "\\{\\{";

fn template_alt() -> Value {
    json!({"type": "string", "pattern": TEMPLATE_PATTERN})
}

/// Schema eines Felds. `templates`: bei Bausteinen darf an jeder Stelle statt des
/// Werts ein Text mit `{{...}}` stehen (wird erst beim Lauf eingesetzt).
fn field_schema(f: &FieldSpec, templates: bool) -> Value {
    let base = match f.kind {
        FieldKind::Text => json!({"type": "string", "maxLength": 4000}),
        FieldKind::Id => json!({"type": "string", "pattern": REGISTER_ID_PATTERN}),
        FieldKind::Int { min, max } => json!({"type": "integer", "minimum": min, "maximum": max}),
        FieldKind::Bool => json!({"type": "boolean"}),
        FieldKind::TextList => {
            json!({"type": "array", "maxItems": 100, "items": {"type": "string"}})
        }
        FieldKind::Choice(options) => json!({"enum": options}),
        FieldKind::Any => return json!({}),
    };
    if templates && !f.literal && !matches!(f.kind, FieldKind::Text) {
        json!({"anyOf": [base, template_alt()]})
    } else {
        base
    }
}

fn object_schema(fields: &[FieldSpec], templates: bool, extra: Map<String, Value>) -> Value {
    let mut props = extra;
    let mut required: Vec<Value> = Vec::new();
    for f in fields {
        props.insert(f.name.to_string(), field_schema(f, templates));
        if f.required {
            required.push(json!(f.name));
        }
    }
    json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false
    })
}

fn trigger_schema() -> Value {
    let branches: Vec<Value> = catalog::triggers()
        .iter()
        .map(|t| {
            let mut extra = Map::new();
            extra.insert("type".to_string(), json!({"const": t.id}));
            let mut s = object_schema(t.fields, false, extra);
            if let Some(Value::Array(req)) = s.get_mut("required") {
                req.insert(0, json!("type"));
            }
            s["title"] = json!(t.title);
            s
        })
        .collect();
    json!({"oneOf": branches})
}

fn step_schema() -> Value {
    let branches: Vec<Value> = catalog::actions()
        .iter()
        .map(|a| {
            let params = object_schema(a.fields, true, Map::new());
            json!({
                "title": a.title,
                "type": "object",
                "properties": {
                    "id": {"type": "string", "pattern": IDENT_PATTERN},
                    "action": {"const": a.id},
                    "label": {"type": "string", "maxLength": MAX_NAME_CHARS},
                    "when": {"type": "string", "maxLength": super::expr::MAX_EXPR_CHARS},
                    "params": params,
                    "on_error": {"enum": ["fail", "continue"]},
                    "retry": {
                        "type": "object",
                        "properties": {
                            "max_attempts": {"type": "integer", "minimum": 1, "maximum": MAX_ATTEMPTS_LIMIT},
                            "backoff_ms": {"type": "integer", "minimum": MIN_BACKOFF_MS, "maximum": MAX_BACKOFF_MS}
                        },
                        "additionalProperties": false
                    }
                },
                "required": ["id", "action"],
                "additionalProperties": false
            })
        })
        .collect();
    json!({"oneOf": branches})
}

/// Das vollstaendige Schema.
pub fn json_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:local-voice-ai:workflow:1",
        "title": SCHEMA_ID,
        "description": "Ablauf mit Ausloeser, Variablen und linearen Schritten. Bezuege (steps.<id> nur auf fruehere Schritte, deklarierte Variablen, gueltige Ausdruecke) prueft die App beim Speichern.",
        "type": "object",
        "properties": {
            "schema": {"const": SCHEMA_ID},
            "name": {"type": "string", "minLength": 1, "maxLength": MAX_NAME_CHARS},
            "description": {"type": "string", "maxLength": MAX_TEXT_CHARS},
            "trigger": trigger_schema(),
            "variables": {
                "type": "object",
                "maxProperties": MAX_VARIABLES,
                "propertyNames": {"pattern": IDENT_PATTERN},
                "additionalProperties": {
                    "type": "object",
                    "properties": {
                        "type": {"enum": ["string", "number", "bool", "list"]},
                        "default": {},
                        "description": {"type": "string", "maxLength": MAX_TEXT_CHARS}
                    },
                    "required": ["type"],
                    "additionalProperties": false
                }
            },
            "steps": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_STEPS,
                "items": step_schema()
            }
        },
        "required": ["schema", "name", "trigger", "steps"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests;
