//! C1: Werkzeugliste -> JSON-Schema fuer die Werkzeugwahl ("Schema-Modus").
//!
//! Das Modell antwortet nicht ueber natives Tool-Calling, sondern mit einem
//! Objekt `{ "tool": <name>, "arguments": { ... } }`, das per
//! `response_format` (json_schema, Grammatik-Sampling im llama-server) an genau
//! die angebotenen Werkzeuge gebunden ist: je Werkzeug eine `oneOf`-Variante mit
//! `const`-Namen und `additionalProperties: false`. Im Spike (30.09.2026) war das
//! robuster als natives Tool-Calling (50/50 gegen 46/50).
//!
//! `no_action` ist immer dabei: der Rueckfall, wenn kein Werkzeug passt.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Name des Rueckfall-Werkzeugs ("nichts tun"), immer angeboten.
pub const NO_ACTION: &str = "no_action";

/// Ein Werkzeug, wie der Router es sieht: Name, Beschreibung fuer den Prompt
/// und das JSON-Schema der Argumente (`type: object`, `properties`, `required`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolSpec {
    /// Namen der Argumente laut Schema (fuer Prompt und Datensatzpruefung).
    pub fn param_names(&self) -> Vec<String> {
        self.parameters
            .get("properties")
            .and_then(Value::as_object)
            .map(|props| props.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Pflichtargumente laut Schema.
    pub fn required(&self) -> Vec<String> {
        self.parameters
            .get("required")
            .and_then(Value::as_array)
            .map(|req| {
                req.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Die angebotenen Werkzeuge: `allowed` (Namen) aus `all`, in der Reihenfolge
/// von `all`, `no_action` immer am Ende. `None` heisst: alle. Ein unbekannter
/// Name ist ein Fehler (Tippfehler in einer Whitelist darf nicht still ein
/// Werkzeug verschwinden lassen).
pub fn offered<'a>(
    all: &'a [ToolSpec],
    allowed: Option<&[String]>,
) -> Result<Vec<&'a ToolSpec>, String> {
    if let Some(names) = allowed {
        for name in names {
            if !all.iter().any(|t| &t.name == name) {
                return Err(format!("unbekanntes Werkzeug in der Auswahl: {name}"));
            }
        }
    }
    let no_action = all
        .iter()
        .find(|t| t.name == NO_ACTION)
        .ok_or_else(|| format!("Werkzeugliste ohne {NO_ACTION}"))?;
    let mut out: Vec<&ToolSpec> = all
        .iter()
        .filter(|t| t.name != NO_ACTION)
        .filter(|t| allowed.map_or(true, |names| names.iter().any(|n| n == &t.name)))
        .collect();
    out.push(no_action);
    Ok(out)
}

/// `oneOf`-Schema ueber die angebotenen Werkzeuge. Die Argument-Schemas
/// bekommen `additionalProperties: false`, damit das Modell keine Felder
/// erfindet.
pub fn choice_schema(tools: &[&ToolSpec]) -> Value {
    let variants: Vec<Value> = tools
        .iter()
        .map(|tool| {
            let mut params = match &tool.parameters {
                Value::Object(map) => map.clone(),
                _ => Map::new(),
            };
            params.insert("type".into(), json!("object"));
            params.insert("additionalProperties".into(), json!(false));
            params.entry("properties").or_insert_with(|| json!({}));
            json!({
                "type": "object",
                "properties": {
                    "tool": { "const": tool.name },
                    "arguments": Value::Object(params),
                },
                "required": ["tool", "arguments"],
                "additionalProperties": false,
            })
        })
        .collect();
    json!({ "oneOf": variants })
}

/// Die Werkzeugliste fuer den Systemprompt: eine Zeile je Werkzeug mit
/// Beschreibung und Argumenten (Pflichtargumente ohne Stern, optionale mit `?`).
pub fn prompt_tool_lines(tools: &[&ToolSpec]) -> String {
    tools
        .iter()
        .map(|tool| {
            let required = tool.required();
            let params: Vec<String> = tool
                .param_names()
                .into_iter()
                .map(|p| {
                    if required.contains(&p) {
                        p
                    } else {
                        format!("{p}?")
                    }
                })
                .collect();
            format!(
                "- {}: {} (Argumente: {})",
                tool.name,
                tool.description,
                if params.is_empty() {
                    "keine".to_string()
                } else {
                    params.join(", ")
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Die Wahl des Modells, wie sie im Schema steht.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ToolChoice {
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
}

/// Antworttext -> Wahl. Toleriert einen Markdown-Codezaun (manche Vorlagen
/// setzen ihn trotz Schema). Kein gueltiges Objekt -> Fehler mit kurzer
/// Klassifikation (ohne Antworttext).
pub fn parse_choice(raw: &str) -> Result<ToolChoice, String> {
    let text = crate::managers::meetings::llm_call::strip_code_fence(raw.trim());
    serde_json::from_str::<ToolChoice>(text)
        .map_err(|e| crate::managers::meetings::llm_call::describe_json_error(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, props: Value, required: &[&str]) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: format!("Beschreibung {name}"),
            parameters: json!({ "type": "object", "properties": props, "required": required }),
        }
    }

    fn tools() -> Vec<ToolSpec> {
        vec![
            tool("send_mail", json!({"to": {"type": "array"}, "subject": {"type": "string"}}), &["to"]),
            tool("obsidian_note", json!({"title": {"type": "string"}}), &["title"]),
            tool(NO_ACTION, json!({"reason": {"type": "string"}}), &["reason"]),
        ]
    }

    #[test]
    fn offered_appends_no_action_and_filters() {
        let all = tools();
        let names: Vec<_> = offered(&all, None).unwrap().iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, ["send_mail", "obsidian_note", NO_ACTION]);
        let only = vec!["obsidian_note".to_string()];
        let names: Vec<_> = offered(&all, Some(only.as_slice())).unwrap().iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, ["obsidian_note", NO_ACTION]);
    }

    #[test]
    fn offered_rejects_unknown_tool_name() {
        let all = tools();
        let bad = vec!["delete_all".to_string()];
        assert!(offered(&all, Some(bad.as_slice())).unwrap_err().contains("delete_all"));
    }

    #[test]
    fn choice_schema_binds_tool_names_and_forbids_extra_fields() {
        let all = tools();
        let schema = choice_schema(&offered(&all, None).unwrap());
        let variants = schema["oneOf"].as_array().unwrap();
        assert_eq!(variants.len(), 3);
        for v in variants {
            assert_eq!(v["additionalProperties"], false);
            assert_eq!(v["properties"]["arguments"]["additionalProperties"], false);
            assert_eq!(v["properties"]["arguments"]["type"], "object");
            assert!(v["properties"]["tool"]["const"].is_string());
        }
        assert_eq!(variants[2]["properties"]["tool"]["const"], NO_ACTION);
        assert_eq!(variants[0]["properties"]["arguments"]["required"], json!(["to"]));
    }

    #[test]
    fn prompt_lines_mark_optional_arguments() {
        let all = tools();
        let lines = prompt_tool_lines(&offered(&all, None).unwrap());
        assert!(lines.contains("- send_mail: Beschreibung send_mail (Argumente: "));
        assert!(lines.contains("subject?"));
        assert!(lines.contains("to"));
        assert!(!lines.contains("to?"));
    }

    #[test]
    fn parse_choice_accepts_fenced_json_and_rejects_garbage() {
        let c = parse_choice("```json\n{\"tool\":\"send_mail\",\"arguments\":{\"to\":[\"a@example.com\"]}}\n```").unwrap();
        assert_eq!(c.tool, "send_mail");
        assert_eq!(c.arguments["to"][0], "a@example.com");
        assert!(parse_choice("{\"tool\":\"send_mail\",\"arguments\":{").is_err());
        assert!(parse_choice("").is_err());
        assert!(parse_choice("no_action{reason:<|\"|>x}").is_err());
    }
}
