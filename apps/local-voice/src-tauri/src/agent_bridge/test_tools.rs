//! Test-Werkzeuge fuer die headless Sandbox-Instanz (`--agent-bridge-serve`, nur mit
//! `LVA_MEETINGS_DIR` UND `LVA_AGENT_TEST_TOOLS=1`): sie tun nichts ausser die Anfrage
//! zurueckzugeben. Damit lassen sich Rechte, Freigaben und `ctl`-Exit-Codes gegen die echte EXE
//! belegen, ohne dass ein echtes Werkzeug (A8) laufen muss. In der App nie registriert.

use serde_json::{json, Value};

use super::catalog::{find, CallContext, ToolHandler, ToolSpec};

pub struct EchoTools;

const TOOLS: [&str; 2] = ["transcribe_file", "create_meeting"];

impl ToolHandler for EchoTools {
    fn specs(&self) -> Vec<ToolSpec> {
        TOOLS
            .iter()
            .filter_map(|n| {
                let e = find(n)?;
                Some(ToolSpec {
                    name: (*n).to_string(),
                    title: e.title.to_string(),
                    description: format!("{} (Testwerkzeug: gibt die Anfrage zurück)", e.description),
                    input_schema: json!({ "type": "object", "additionalProperties": true }),
                })
            })
            .collect()
    }

    fn call(&self, ctx: &CallContext, tool: &str, args: &Value) -> Result<Value, String> {
        Ok(json!({
            "echo": args,
            "tool": tool,
            "client": ctx.client_label,
            "approved": ctx.approved,
        }))
    }
}

/// Ist der Testbetrieb gewuenscht? Nur mit Sandbox und ausdruecklicher Umgebungsvariable.
pub fn test_tools_enabled() -> bool {
    let sandbox = std::env::var(crate::managers::meetings::MEETINGS_DIR_ENV)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    sandbox && std::env::var("LVA_AGENT_TEST_TOOLS").as_deref() == Ok("1")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::agent_bridge::catalog::ToolRegistry;

    #[test]
    fn the_echo_tools_are_catalog_tools_and_register_cleanly() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(EchoTools)).unwrap();
        assert!(r.has("transcribe_file") && r.has("create_meeting"));
        let ctx = CallContext {
            client_id: "C1".into(),
            client_label: "Test".into(),
            approved: true,
            now_ms: 0,
        };
        let v = EchoTools.call(&ctx, "create_meeting", &json!({"a": 1})).unwrap();
        assert_eq!(v["echo"], json!({"a": 1}));
        assert_eq!(v["approved"], json!(true));
    }
}
