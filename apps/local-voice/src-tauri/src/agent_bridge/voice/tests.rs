//! Tests der lesenden Werkzeuge fuer das Sprachgespraech: echtes Tor, echtes Audit, Attrappen-Handler.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::agent_bridge::catalog::{ToolHandler, ToolSpec};
use crate::managers::integrations::audit::{self, AuditFilter};
use crate::managers::integrations::model::{Capability, IntegrationPatch};
use crate::managers::integrations::test_support::Fx;

const NOW: i64 = 1_790_000_000_000;
const READING: [&str; 3] = ["meetings_list", "meetings_search", "meeting_transcript"];
const READING_OTHER: [&str; 2] = ["tts_providers_list", "list_workflows"];
const WRITING: [&str; 4] = [
    "transcribe_file",
    "create_meeting",
    "start_recording",
    "run_workflow",
];

/// Zaehlt Aufrufe je Werkzeug; liefert ein festes Ergebnis.
struct Fake {
    names: Vec<&'static str>,
    calls: AtomicUsize,
    reply: Value,
}

impl Fake {
    fn all(reply: Value) -> Arc<Self> {
        let names = READING
            .iter()
            .chain(READING_OTHER.iter())
            .chain(WRITING.iter())
            .copied()
            .collect();
        Arc::new(Self {
            names,
            calls: AtomicUsize::new(0),
            reply,
        })
    }
}

impl ToolHandler for Fake {
    fn specs(&self) -> Vec<ToolSpec> {
        self.names
            .iter()
            .map(|n| ToolSpec {
                name: n.to_string(),
                title: n.to_string(),
                description: format!("Werkzeug {n} für den Test."),
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            })
            .collect()
    }

    fn call(&self, _ctx: &CallContext, _tool: &str, _args: &Value) -> Result<Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.reply.clone())
    }
}

struct Rig {
    fx: Fx,
    fake: Arc<Fake>,
    registry: ToolRegistry,
}

fn rig_with(reply: Value) -> Rig {
    let fake = Fake::all(reply);
    let mut registry = ToolRegistry::new();
    registry.register(fake.clone()).unwrap();
    Rig {
        fx: Fx::new(),
        fake,
        registry,
    }
}

fn rig() -> Rig {
    rig_with(json!({"ok": true}))
}

impl Rig {
    fn list(&self) -> Vec<String> {
        let conn = self.fx.conn();
        list(&conn, &self.registry, NOW)
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    fn call(&self, name: &str, args: &str) -> Result<String, String> {
        let conn = self.fx.conn();
        call(&conn, &self.registry, name, args, NOW)
    }

    fn integration_id(&self) -> String {
        clients::ensure_agent_integration(&self.fx.conn(), NOW)
            .unwrap()
            .id
    }

    fn set_right(&self, cap: Capability, mode: GrantMode) {
        let id = self.integration_id();
        store::set_grant(&self.fx.conn(), &id, cap, Caller::AgentLocal, mode).unwrap();
    }

    fn audit(&self) -> Vec<crate::managers::integrations::model::AuditEntry> {
        audit::list(&self.fx.conn(), &AuditFilter::default(), 100).unwrap()
    }
}

#[test]
fn read_only_follows_the_catalog_annotation() {
    for entry in catalog::CATALOG {
        let hint = catalog::annotations(entry)["readOnlyHint"]
            .as_bool()
            .unwrap();
        assert_eq!(is_read_only(entry), hint, "{}", entry.name);
    }
    for name in READING.iter().chain(READING_OTHER.iter()) {
        assert!(is_read_only(catalog::find(name).unwrap()), "{name}");
    }
    for name in WRITING {
        assert!(!is_read_only(catalog::find(name).unwrap()), "{name}");
    }
}

#[test]
fn writing_tools_are_never_listed_or_run() {
    let r = rig();
    let listed = r.list();
    for name in WRITING {
        assert!(!listed.iter().any(|n| n == name), "{name} in der Liste");
        let err = r.call(name, "{}").unwrap_err();
        assert!(err.contains("nur lesende"), "{name}: {err}");
    }
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0, "Handler gerufen");
    for row in r.audit() {
        let target = row.target.as_deref().unwrap_or("");
        assert!(
            WRITING.iter().all(|w| !target.starts_with(w)),
            "ein abgelehnter Aufruf hinterlässt keinen Werkzeug-Eintrag: {target}"
        );
    }
    // Die lesenden dagegen laufen.
    for name in READING.iter().chain(READING_OTHER.iter()) {
        assert!(listed.iter().any(|n| n == name), "{name} fehlt");
    }
    assert_eq!(r.call("meetings_list", "{}").unwrap(), r#"{"ok":true}"#);
}

#[test]
fn unknown_and_odd_names_are_refused() {
    let r = rig();
    for name in [
        "",
        "Meetings_List",
        "meetings_list ",
        " meetings_list",
        "../meetings_list",
        "meetings_list\u{0}",
        "get_action_status",
        "nicht_im_katalog",
        &"a".repeat(200),
    ] {
        assert!(r.call(name, "{}").is_err(), "{name:?}");
    }
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_catalog_tool_without_a_handler_is_unavailable() {
    let fake = Arc::new(Fake {
        names: vec!["meetings_list"],
        calls: AtomicUsize::new(0),
        reply: json!({}),
    });
    let mut registry = ToolRegistry::new();
    registry.register(fake).unwrap();
    let fx = Fx::new();
    let conn = fx.conn();
    assert!(call(&conn, &registry, "tts_providers_list", "{}", NOW)
        .unwrap_err()
        .contains("nicht verfügbar"));
    assert_eq!(list(&conn, &registry, NOW).unwrap().len(), 1);
}

#[test]
fn the_list_follows_the_rights() {
    let r = rig();
    assert_eq!(r.list().len(), READING.len() + READING_OTHER.len());
    // Besprechungen „aus“: nur diese fehlen.
    r.set_right(Capability::MeetingRead, GrantMode::Off);
    let listed = r.list();
    assert!(READING.iter().all(|n| !listed.iter().any(|l| l == n)));
    assert!(READING_OTHER.iter().all(|n| listed.iter().any(|l| l == n)));
    // „fragen“ gilt im Gespraech wie „aus“.
    r.set_right(Capability::MeetingRead, GrantMode::Ask);
    assert!(READING.iter().all(|n| !r.list().iter().any(|l| l == n)));
    // Wieder erlaubt.
    r.set_right(Capability::MeetingRead, GrantMode::Allow);
    assert_eq!(r.list().len(), READING.len() + READING_OTHER.len());
    // Integration aus: nichts mehr.
    let id = r.integration_id();
    let patch = IntegrationPatch {
        enabled: Some(false),
        ..IntegrationPatch::default()
    };
    store::update(&r.fx.conn(), &id, &patch, NOW).unwrap();
    assert!(r.list().is_empty());
    assert!(r.call("meetings_list", "{}").is_err());
}

#[test]
fn a_right_of_off_blocks_the_call_and_is_audited() {
    let r = rig();
    r.set_right(Capability::MeetingRead, GrantMode::Off);
    let err = r.call("meetings_list", "{}").unwrap_err();
    assert!(err.contains("aus"), "{err}");
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0);
    assert!(r
        .audit()
        .iter()
        .any(|a| a.outcome == "denied" && a.caller == "agent_local"));
}

#[test]
fn a_right_of_ask_runs_nothing() {
    let r = rig();
    r.set_right(Capability::MeetingRead, GrantMode::Ask);
    let err = r.call("meetings_list", "{}").unwrap_err();
    assert!(err.contains("Freigabe"), "{err}");
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn arguments_must_be_a_small_object() {
    let r = rig();
    for args in ["[]", "\"x\"", "42", "nicht json", "{\"a\":"] {
        assert!(r.call("meetings_list", args).is_err(), "{args}");
    }
    let big = format!("{{\"q\":\"{}\"}}", "a".repeat(MAX_ARGS_BYTES));
    assert!(r
        .call("meetings_list", &big)
        .unwrap_err()
        .contains("zu groß"));
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0);
    // Leer und null sind ein leeres Objekt.
    assert!(r.call("meetings_list", "").is_ok());
    assert!(r.call("meetings_list", "null").is_ok());
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn long_results_are_cut_at_a_character_boundary() {
    assert_eq!(clamp_result("kurz".to_string(), 10), "kurz");
    assert_eq!(clamp_result("ääää".to_string(), 4), "ääää");
    let cut = clamp_result("äöüß!".to_string(), 4);
    assert!(cut.starts_with("äöüß"), "{cut}");
    assert!(cut.ends_with("[gekürzt]"), "{cut}");
    let r = rig_with(json!({"text": "ü".repeat(MAX_RESULT_CHARS * 2)}));
    let out = r.call("meetings_list", "{}").unwrap();
    assert!(
        out.chars().count() <= MAX_RESULT_CHARS + 20,
        "{}",
        out.len()
    );
    assert!(out.ends_with("[gekürzt]"));
}

#[test]
fn every_call_is_audited_without_content() {
    let r = rig_with(json!({"title": "GEHEIMES-ERGEBNIS"}));
    r.call("meetings_search", r#"{"query":"GEHEIME-FRAGE"}"#)
        .unwrap();
    let rows = r.audit();
    let mine: Vec<_> = rows
        .iter()
        .filter(|a| a.target.as_deref() == Some("meetings_search · voice_orchestrator"))
        .collect();
    assert!(!mine.is_empty(), "{rows:?}");
    assert!(mine.iter().any(|a| a.outcome == "ok"));
    for a in &rows {
        assert_eq!(a.caller, "agent_local");
        assert_eq!(a.capability.as_deref(), Some("meeting.read"));
        let all = format!("{:?}", a);
        assert!(!all.contains("GEHEIM"), "{all}");
    }
}

#[test]
fn a_failing_handler_is_an_error_and_audited() {
    struct Failing;
    impl ToolHandler for Failing {
        fn specs(&self) -> Vec<ToolSpec> {
            vec![ToolSpec {
                name: "meetings_list".to_string(),
                title: "t".to_string(),
                description: "Werkzeug für den Test.".to_string(),
                input_schema: json!({"type": "object"}),
            }]
        }
        fn call(&self, _c: &CallContext, _t: &str, _a: &Value) -> Result<Value, String> {
            Err("Die Besprechungsdatenbank ist gesperrt.".to_string())
        }
    }
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(Failing)).unwrap();
    let fx = Fx::new();
    let conn = fx.conn();
    let err = call(&conn, &registry, "meetings_list", "{}", NOW).unwrap_err();
    assert!(err.contains("gesperrt"));
    let rows = audit::list(&conn, &AuditFilter::default(), 10).unwrap();
    assert!(rows.iter().any(|a| a.outcome == "error"));
}
