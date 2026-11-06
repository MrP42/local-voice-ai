//! Strikter Ereignisvertrag der Claude-Vorschau (lva#334): `claude -p
//! --output-format stream-json --verbose` darf NUR Ereignisse liefern, die in
//! `tooling/buch-gespraech/claude-events.json` stehen. Die Datei ist die
//! gemeinsame Quelle: die App bindet sie mit `include_str!` ein, das
//! Pruefskript (`isolation-check.sh`/`.ps1`) liest dieselbe Datei und meldet
//! `event_contract` nur, wenn alle beobachteten Ereignisse darin stehen.
//!
//! Regeln (jede Verletzung sperrt, auch wenn spaeter ein Erfolg folgt):
//! * `system/init` ist das erste Ereignis und kommt genau einmal vor;
//! * genau ein `result`, und es ist das LETZTE Ereignis;
//! * jedes Ereignis ist in der Whitelist (Typ, ggf. Untertyp), sonst gesperrt;
//! * Inhaltsbloecke nur aus der Whitelist des Ereignistyps; `tool_use` (und
//!   verwandte) und `tool_result` irgendwo sperren mit eigenem Grund;
//! * kein Fehlerereignis irgendwo: `is_error: true`, ein `error`-Feld oder
//!   `api_error_status` am Ereignis oder an `message`.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde_json::Value;

/// Obergrenze fuer Zahlenfelder: 2^53-1 (JSON-sicher), gleich in `analyze-run.py` und der PS1.
pub const MAX_SAFE_INT: u64 = 9_007_199_254_740_991;

/// Die gemeinsame Whitelist-Datei.
pub const EVENTS_JSON: &str =
    include_str!("../../../../../../tooling/buch-gespraech/claude-events.json");

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Rule {
    kind: String,
    /// `None`: kein Untertyp erlaubt oder verlangt (jeder Untertyp ist dann unbekannt).
    subtypes: Option<Vec<String>>,
    blocks: Vec<String>,
}

/// Die geladene Whitelist. Unlesbare Datei: leer, also ist alles gesperrt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contract {
    rules: Vec<Rule>,
    tool_blocks: Vec<String>,
    /// Pflichtfelder je Ereignis (`typ` oder `typ/untertyp`): nicht negative Ganzzahlen.
    numeric_fields: Vec<(String, Vec<String>)>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect()
}

impl Contract {
    pub fn parse(json: &str) -> Self {
        let Ok(v) = serde_json::from_str::<Value>(json) else {
            return Self::default();
        };
        if v.get("schema").and_then(Value::as_u64) != Some(1) {
            return Self::default();
        }
        let rules = v
            .get("events")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some(Rule {
                    kind: e.get("type")?.as_str()?.to_string(),
                    subtypes: e.get("subtypes").map(|s| strings(Some(s))),
                    blocks: strings(e.get("content_blocks")),
                })
            })
            .collect();
        let numeric_fields = v
            .get("numeric_fields")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(label, fields)| (label.clone(), strings(Some(fields))))
            .collect();
        Self {
            rules,
            tool_blocks: strings(v.get("tool_blocks")),
            numeric_fields,
        }
    }

    /// Die eingebaute Whitelist.
    pub fn builtin() -> &'static Self {
        static C: OnceLock<Contract> = OnceLock::new();
        C.get_or_init(|| Self::parse(EVENTS_JSON))
    }

    /// Typ und Untertyp als `typ` bzw. `typ/untertyp`.
    pub fn label(ev: &Value) -> String {
        let kind = ev.get("type").and_then(Value::as_str).unwrap_or("?");
        match ev.get("subtype").and_then(Value::as_str) {
            Some(s) => format!("{kind}/{s}"),
            None => kind.to_string(),
        }
    }

    /// Alle beobachteten Typen (`typ` bzw. `typ/untertyp`) eines Laufs.
    pub fn observed(events: &[Value]) -> BTreeSet<String> {
        events.iter().map(Self::label).collect()
    }

    fn rule(&self, ev: &Value) -> Option<&Rule> {
        let kind = ev.get("type").and_then(Value::as_str)?;
        let sub = ev.get("subtype").and_then(Value::as_str);
        let rule = self.rules.iter().find(|r| r.kind == kind)?;
        match (&rule.subtypes, sub) {
            (None, None) => Some(rule),
            (None, Some(_)) => None,
            (Some(list), Some(s)) if list.iter().any(|x| x == s) => Some(rule),
            _ => None,
        }
    }

    /// Ist jedes Ereignis in der Whitelist? Vorbedingung des Skripts fuer
    /// `event_contract` (die App prueft zusaetzlich mit [`Self::check`]).
    pub fn all_known(&self, events: &[Value]) -> bool {
        events
            .iter()
            .all(|e| self.rule(e).is_some() && self.fields_ok(e))
    }

    /// `numeric_fields`: jedes Pflichtfeld des Ereignisses ist eine nicht negative
    /// Ganzzahl (kein String, keine Kommazahl, kein Bool, nicht negativ).
    fn fields_ok(&self, ev: &Value) -> bool {
        let label = Self::label(ev);
        self.numeric_fields
            .iter()
            .filter(|(l, _)| *l == label)
            .flat_map(|(_, fields)| fields)
            .all(|f| {
                ev.get(f)
                    .and_then(Value::as_u64)
                    .is_some_and(|n| n <= MAX_SAFE_INT)
            })
    }

    /// Prueft einen vollstaendigen Lauf. `Err` ist der feste Grund fuers Audit.
    pub fn check(&self, events: &[Value]) -> Result<(), &'static str> {
        let Some(first) = events.first() else {
            return Err("event_order");
        };
        if Self::label(first) != "system/init" {
            return Err("event_order");
        }
        let results = events
            .iter()
            .filter(|e| e.get("type").and_then(Value::as_str) == Some("result"))
            .count();
        let inits = events
            .iter()
            .filter(|e| Self::label(e) == "system/init")
            .count();
        if inits != 1 {
            return Err("event_order");
        }
        for (i, ev) in events.iter().enumerate() {
            let is_result = ev.get("type").and_then(Value::as_str) == Some("result");
            if results > 0 && i + 1 < events.len() && is_result {
                return Err("event_after_result");
            }
            if let Some(reason) = self.tool_use(ev) {
                return Err(reason);
            }
            let Some(rule) = self.rule(ev).filter(|_| self.fields_ok(ev)) else {
                return Err("event_unknown");
            };
            if is_error(ev) {
                return Err("error_event");
            }
            for block in blocks_of(ev) {
                if !rule.blocks.iter().any(|b| *b == block) {
                    return Err("content_unknown");
                }
            }
        }
        if results != 1 {
            return Err("event_order");
        }
        Ok(())
    }

    /// `tool_use`/`tool_result` als Inhaltsblock oder als Ereignistyp.
    fn tool_use(&self, ev: &Value) -> Option<&'static str> {
        let tool = |name: &str| self.tool_blocks.iter().any(|t| t == name);
        let kind = ev.get("type").and_then(Value::as_str).unwrap_or("");
        if tool(kind) || blocks_of(ev).iter().any(|b| tool(b)) {
            return Some("tool_use");
        }
        None
    }
}

/// Fehlerereignis: `is_error: true`, ein nicht leeres `error` oder
/// `api_error_status` am Ereignis oder an seiner `message`.
fn is_error(ev: &Value) -> bool {
    let flagged = |v: &Value| {
        v.get("is_error").and_then(Value::as_bool) == Some(true)
            || v.get("error").is_some_and(|e| !e.is_null())
            || v.get("api_error_status").is_some_and(|e| !e.is_null())
    };
    flagged(ev) || ev.get("message").is_some_and(flagged)
}

/// Typen der Inhaltsbloecke von `message.content`; ein String zaehlt als `text`.
fn blocks_of(ev: &Value) -> Vec<String> {
    match ev.get("message").and_then(|m| m.get("content")) {
        Some(Value::String(_)) => vec!["text".to_string()],
        Some(Value::Array(a)) => a
            .iter()
            .map(|b| {
                b.get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string()
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Alle Modell-IDs eines Laufs: `system/init.model`, `assistant.message.model`
/// und die Schluessel von `modelUsage` im Ergebnis.
pub fn models_of(events: &[Value]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for ev in events {
        let kind = ev.get("type").and_then(Value::as_str).unwrap_or("");
        let direct = match kind {
            "system" => ev.get("model"),
            "assistant" => ev.get("message").and_then(|m| m.get("model")),
            _ => None,
        };
        if let Some(m) = direct.and_then(Value::as_str) {
            out.insert(m.trim().to_string());
        }
        if kind == "result" {
            if let Some(map) = ev.get("modelUsage").and_then(Value::as_object) {
                out.extend(map.keys().map(|k| k.trim().to_string()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evs(lines: &[&str]) -> Vec<Value> {
        lines
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    const INIT: &str = r#"{"type":"system","subtype":"init","model":"m","tools":[]}"#;
    const ASSIST: &str =
        r#"{"type":"assistant","message":{"model":"m","content":[{"type":"text","text":"x"}]}}"#;
    const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"x","modelUsage":{"m":{}}}"#;

    #[test]
    fn the_shared_file_parses_and_a_clean_run_passes() {
        let c = Contract::builtin();
        assert!(!c.rules.is_empty(), "Whitelist-Datei nicht lesbar");
        assert_eq!(c.check(&evs(&[INIT, ASSIST, RESULT])), Ok(()));
        assert!(c.all_known(&evs(&[INIT, ASSIST, RESULT])));
        assert_eq!(Contract::parse("kaputt"), Contract::default());
        assert_eq!(
            Contract::default().check(&evs(&[INIT, ASSIST, RESULT])),
            Err("event_unknown")
        );
    }

    #[test]
    fn unknown_events_blocks_and_order_violations_are_rejected() {
        let c = Contract::builtin();
        let unknown = r#"{"type":"stream_event"}"#;
        assert_eq!(
            c.check(&evs(&[INIT, unknown, ASSIST, RESULT])),
            Err("event_unknown")
        );
        let sub = r#"{"type":"system","subtype":"hook_started"}"#;
        assert_eq!(
            c.check(&evs(&[INIT, sub, ASSIST, RESULT])),
            Err("event_unknown")
        );
        assert_eq!(c.check(&evs(&[ASSIST, INIT, RESULT])), Err("event_order"));
        assert_eq!(c.check(&evs(&[INIT, ASSIST])), Err("event_order"));
        assert_eq!(
            c.check(&evs(&[INIT, ASSIST, RESULT, RESULT])),
            Err("event_after_result")
        );
        assert_eq!(
            c.check(&evs(&[INIT, RESULT, ASSIST])),
            Err("event_after_result")
        );
        assert_eq!(c.check(&evs(&[INIT, INIT, RESULT])), Err("event_order"));
        assert_eq!(c.check(&[]), Err("event_order"));
        assert_eq!(
            c.all_known(&evs(&[INIT, unknown])),
            false,
            "das Skript meldet den Vertrag nur bei lauter bekannten Ereignissen"
        );
    }

    #[test]
    fn tools_content_blocks_and_errors_are_rejected_anywhere() {
        let c = Contract::builtin();
        let tool_use =
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#;
        let tool_result =
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#;
        let weird = r#"{"type":"assistant","message":{"content":[{"type":"image"}]}}"#;
        assert_eq!(
            c.check(&evs(&[INIT, tool_use, ASSIST, RESULT])),
            Err("tool_use")
        );
        assert_eq!(
            c.check(&evs(&[INIT, tool_result, ASSIST, RESULT])),
            Err("tool_use")
        );
        assert_eq!(
            c.check(&evs(&[INIT, weird, ASSIST, RESULT])),
            Err("content_unknown")
        );
        for err in [
            r#"{"type":"assistant","error":"rate_limit","message":{"content":[{"type":"text","text":"x"}]}}"#,
            r#"{"type":"assistant","is_error":true,"message":{"content":[]}}"#,
            r#"{"type":"assistant","message":{"error":"x","content":[]}}"#,
            r#"{"type":"assistant","api_error_status":529,"message":{"content":[]}}"#,
        ] {
            assert_eq!(
                c.check(&evs(&[INIT, err, ASSIST, RESULT])),
                Err("error_event"),
                "{err}"
            );
        }
        let user_ok = r#"{"type":"user","message":{"content":"Frage"}}"#;
        assert_eq!(c.check(&evs(&[INIT, user_ok, ASSIST, RESULT])), Ok(()));
    }

    fn fixture(text: &str) -> Vec<Value> {
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn measured_events_are_allowed_but_hooks_and_late_events_still_block() {
        // Beleg: Claude Code 2.1.281, Windows, 06.11.2026 (claude-events.json, `evidence`).
        let c = Contract::builtin();
        let ok = fixture(include_str!(
            "../../../../../../tooling/buch-gespraech/tests/fixtures/ok_with_rate_limit_and_summary.jsonl"
        ));
        // rate_limit_event, system/post_turn_summary, system/commands_changed und neue
        // Felder am result (terminal_reason, fast_mode_state, ttft_ms) sind kein Problem.
        assert_eq!(c.check(&ok), Ok(()));
        assert!(c.all_known(&ok));
        let hook = fixture(include_str!(
            "../../../../../../tooling/buch-gespraech/tests/fixtures/hook_event.jsonl"
        ));
        assert_eq!(c.check(&hook), Err("event_unknown"));
        for sub in ["hook_started", "hook_response", "hook_progress"] {
            let ev = format!(r#"{{"type":"system","subtype":"{sub}"}}"#);
            assert_eq!(
                c.check(&evs(&[INIT, &ev, ASSIST, RESULT])),
                Err("event_unknown")
            );
        }
        let late = fixture(include_str!(
            "../../../../../../tooling/buch-gespraech/tests/fixtures/rate_limit_after_result.jsonl"
        ));
        assert_eq!(c.check(&late), Err("event_after_result"));
        // Mit Untertyp bleibt rate_limit_event unbekannt.
        let sub = r#"{"type":"rate_limit_event","subtype":"x"}"#;
        assert_eq!(
            c.check(&evs(&[INIT, sub, ASSIST, RESULT])),
            Err("event_unknown")
        );
    }

    #[test]
    fn thinking_tokens_need_non_negative_integer_fields_and_stay_before_the_result() {
        let c = Contract::builtin();
        let ok = fixture(include_str!(
            "../../../../../../tooling/buch-gespraech/tests/fixtures/ok_with_thinking_tokens.jsonl"
        ));
        assert_eq!(c.check(&ok), Ok(()));
        assert!(c.all_known(&ok));
        // Genau die Obergrenze 2^53-1 ist noch gueltig.
        let max = fixture(include_str!(
            "../../../../../../tooling/buch-gespraech/tests/fixtures/thinking_tokens_max_ok.jsonl"
        ));
        assert_eq!(c.check(&max), Ok(()));
        for (name, text, want) in [
            (
                "negativ",
                include_str!("../../../../../../tooling/buch-gespraech/tests/fixtures/thinking_tokens_negative.jsonl"),
                "event_unknown",
            ),
            (
                "fehlt",
                include_str!("../../../../../../tooling/buch-gespraech/tests/fixtures/thinking_tokens_missing_field.jsonl"),
                "event_unknown",
            ),
            (
                "String",
                include_str!("../../../../../../tooling/buch-gespraech/tests/fixtures/thinking_tokens_not_a_number.jsonl"),
                "event_unknown",
            ),
            (
                "Kommazahl",
                include_str!("../../../../../../tooling/buch-gespraech/tests/fixtures/thinking_tokens_float.jsonl"),
                "event_unknown",
            ),
            (
                "ueber 2^53-1",
                include_str!("../../../../../../tooling/buch-gespraech/tests/fixtures/thinking_tokens_over_max.jsonl"),
                "event_unknown",
            ),
            (
                "ueber Int64",
                include_str!("../../../../../../tooling/buch-gespraech/tests/fixtures/thinking_tokens_huge.jsonl"),
                "event_unknown",
            ),
            (
                "nach dem Ergebnis",
                include_str!("../../../../../../tooling/buch-gespraech/tests/fixtures/thinking_tokens_after_result.jsonl"),
                "event_after_result",
            ),
        ] {
            assert_eq!(c.check(&fixture(text)), Err(want), "{name}");
        }
    }

    #[test]
    fn all_models_of_a_run_are_collected() {
        let other = r#"{"type":"assistant","message":{"model":"anders","content":[]}}"#;
        let set = models_of(&evs(&[INIT, other, RESULT]));
        assert_eq!(set.into_iter().collect::<Vec<_>>(), vec!["anders", "m"]);
        assert_eq!(models_of(&evs(&[INIT, ASSIST, RESULT])).len(), 1);
    }
}
