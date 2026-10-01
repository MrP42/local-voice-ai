//! Tests der Werkzeuge der Automationen (B8): echte Engine auf einer Test-Datenbank, echte Bruecke
//! fuer Rechte, Freigaben und Audit.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::agent_bridge::bridge::{Bridge, BridgeError, CallStep, ClientCtx};
use crate::agent_bridge::catalog::{self, CallContext, ToolHandler, ToolRegistry};
use crate::agent_bridge::clients;
use crate::agent_bridge::protocol::code;
use crate::agent_bridge::testkit::{fast_config, NOW};
use crate::agent_bridge::tools::{Host, PageRef, QueuedImport, Rendered};
use crate::managers::integrations::audit::{self, AuditFilter};
use crate::managers::integrations::model::{Access, Caller, Capability, GrantMode, Kind};
use crate::managers::integrations::store as register;
use crate::managers::integrations::test_support::Fx;
use crate::managers::meetings::store::MeetingStore;
use crate::managers::workflows::engine::{Engine, EngineConfig, EnqueueRequest};
use crate::managers::workflows::recording::RecordingControl;
use crate::managers::workflows::store::{self, RunFilter};
use crate::managers::workflows::test_support::{armed_workflow, def, note, roomy_gate, FakeClock};
use crate::managers::youtube::source::AddOptions;

/// Ein Host, der nur die Engine kennt: alles andere gibt es hier nicht.
struct WfHost {
    engine: Option<Engine>,
}

impl Host for WfHost {
    fn enqueue_import(&self, _t: &str, _p: &str) -> Result<QueuedImport, String> {
        Err("nicht in diesem Test".to_string())
    }

    fn create_page(&self, _t: &str, _x: &str) -> Result<PageRef, String> {
        Err("nicht in diesem Test".to_string())
    }

    fn page_text(&self, _id: &str) -> Result<String, String> {
        Err("nicht in diesem Test".to_string())
    }

    fn render_audio(&self, _p: &str, _t: &str, _f: &str) -> Result<Rendered, String> {
        Err("nicht in diesem Test".to_string())
    }

    fn recording(&self) -> Option<Arc<dyn RecordingControl>> {
        None
    }

    fn youtube_options(&self) -> AddOptions {
        AddOptions::production()
    }

    fn workflow_engine(&self) -> Option<Engine> {
        self.engine.clone()
    }
}

struct Rig {
    fx: Fx,
    engine: Engine,
    tools: Arc<AppTools>,
    bridge: Bridge,
    client: ClientCtx,
}

fn rig_with_db(fx: Fx, engine: Option<Engine>) -> Rig {
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let tools = Arc::new(AppTools::new(
        store.clone(),
        Arc::new(WfHost {
            engine: engine.clone(),
        }),
    ));
    let mut registry = ToolRegistry::new();
    registry.register(tools.clone()).unwrap();
    let bridge = Bridge::new(store, registry, fast_config()).with_clock(Arc::new(|| NOW));
    let (client, _) = clients::create(&fx.conn(), "Claude Code", None, NOW).unwrap();
    let engine = engine.unwrap_or_else(|| {
        Engine::new(
            PathBuf::from("unused"),
            roomy_gate(),
            FakeClock::new(),
            EngineConfig::default(),
        )
    });
    Rig {
        fx,
        engine,
        tools,
        bridge,
        client: ClientCtx::from(&client),
    }
}

fn rig() -> Rig {
    let fx = Fx::new();
    let engine = Engine::new(
        fx.db_path.clone(),
        roomy_gate(),
        FakeClock::new(),
        EngineConfig::default(),
    );
    rig_with_db(fx, Some(engine))
}

fn ctx() -> CallContext {
    CallContext {
        client_id: "C1".to_string(),
        client_label: "Claude Code".to_string(),
        approved: false,
        now_ms: NOW,
    }
}

impl Rig {
    fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        self.tools.call(&ctx(), tool, &args)
    }

    fn grant(&self, tool: &str, mode: GrantMode) {
        let conn = self.fx.conn();
        let entry = catalog::find(tool).unwrap();
        register::set_grant(
            &conn,
            "agents",
            entry.capability,
            Caller::AgentExternal,
            GrantMode::Allow,
        )
        .unwrap();
        clients::set_tool_mode(&conn, &self.client.id, tool, mode).unwrap();
    }

    fn through_bridge(
        &self,
        tool: &str,
        args: Value,
        approval: Option<&str>,
    ) -> Result<CallStep, BridgeError> {
        self.bridge.call(&self.client, tool, &args, approval)
    }

    fn runs(&self) -> Vec<store::RunRow> {
        store::list_runs(&self.fx.conn(), &RunFilter::default(), 100).unwrap()
    }

    fn armed(&self) -> String {
        armed_workflow(&self.engine, &def(vec![note("a"), note("b")]))
    }
}

fn done(step: CallStep) -> Value {
    match step {
        CallStep::Done(v) => v,
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Katalog
// ---------------------------------------------------------------------------

#[test]
fn the_workflow_tools_sit_behind_their_own_rights() {
    for (name, cap, access) in [
        ("list_workflows", Capability::WorkflowRead, Access::Read),
        ("get_run", Capability::WorkflowRead, Access::Read),
        ("run_workflow", Capability::WorkflowRun, Access::Write),
    ] {
        let e = catalog::find(name).unwrap_or_else(|| panic!("{name} fehlt im Katalog"));
        assert_eq!(e.capability, cap, "{name}");
        assert_eq!(cap.access(), access, "{name}");
        assert!(Kind::Agent.capabilities().contains(&cap), "{name}");
        assert!(!cap.never_allow(), "{name}");
    }
    assert_eq!(
        Capability::parse("workflow.read"),
        Some(Capability::WorkflowRead)
    );
    assert_eq!(
        Capability::parse("workflow.run"),
        Some(Capability::WorkflowRun)
    );
    let read = catalog::annotations(catalog::find("list_workflows").unwrap());
    assert_eq!(read["readOnlyHint"], json!(true));
    let run = catalog::annotations(catalog::find("run_workflow").unwrap());
    assert_eq!(run["readOnlyHint"], json!(false));
}

#[test]
fn the_specs_are_strict_and_describe_the_dry_run_default() {
    let specs = specs();
    let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["list_workflows", "run_workflow", "get_run"]);
    for s in &specs {
        assert_eq!(
            s.input_schema["additionalProperties"],
            json!(false),
            "{}",
            s.name
        );
    }
    let run = specs.iter().find(|s| s.name == "run_workflow").unwrap();
    assert!(run.description.contains("TROCKENLAUF"));
    assert_eq!(
        run.input_schema["properties"]["live"]["default"],
        json!(false)
    );
    assert_eq!(run.input_schema["required"], json!(["workflow_id"]));
}

// ---------------------------------------------------------------------------
// list_workflows
// ---------------------------------------------------------------------------

#[test]
fn list_workflows_shows_state_and_variables_but_not_the_definition() {
    let r = rig();
    let mut d = def(vec![note("a")]);
    d["description"] = json!("Fasst den Tag zusammen");
    d["variables"] = json!({
        "thema": { "type": "string" },
        "tage": { "type": "number", "default": 3 }
    });
    let armed = armed_workflow(&r.engine, &d);
    let draft = r.engine.save_workflow(None, &def(vec![note("x")])).unwrap();

    let v = r.call("list_workflows", json!({})).unwrap();
    assert_eq!(v["count"], 2);
    assert_eq!(v["total"], 2);
    let list = v["workflows"].as_array().unwrap();
    let a = list.iter().find(|w| w["id"] == armed.as_str()).unwrap();
    assert_eq!(a["enabled"], true);
    assert_eq!(a["armed"], true);
    assert_eq!(a["trigger"], "manual");
    assert_eq!(a["steps"], 1);
    assert_eq!(a["description"], "Fasst den Tag zusammen");
    assert_eq!(
        a["variables"],
        json!([
            { "name": "tage", "type": "number", "required": false },
            { "name": "thema", "type": "string", "required": true }
        ])
    );
    assert!(a["last_run"].is_null());
    let b = list.iter().find(|w| w["id"] == draft.id.as_str()).unwrap();
    assert_eq!(b["armed"], false, "neue Abläufe sind nicht scharf");
    let text = v.to_string();
    assert!(!text.contains("definition"), "keine Definition: {text}");
    assert!(
        !text.contains("params"),
        "keine Parameter der Schritte: {text}"
    );
    assert!(r.call("list_workflows", json!({"x": 1})).is_err());
}

// ---------------------------------------------------------------------------
// run_workflow und get_run
// ---------------------------------------------------------------------------

#[test]
fn a_dry_run_is_planned_and_readable_through_get_run() {
    let r = rig();
    let id = r.armed();
    // Der Standard ist der Trockenlauf, auch bei einem scharfen Ablauf.
    let started = r
        .call("run_workflow", json!({ "workflow_id": id }))
        .unwrap();
    assert_eq!(started["dry_run"], true);
    assert_eq!(started["created"], true);
    assert_eq!(started["origin"], "agent");
    assert_eq!(started["workflow_id"], id.as_str());
    let run_id = started["run_id"].as_str().unwrap().to_string();

    let row = store::get_run(&r.fx.conn(), &run_id).unwrap().unwrap();
    assert_eq!(row.origin.as_str(), "agent");
    assert!(row.dry_run);
    assert!(
        row.trigger_key.starts_with("agent:C1:"),
        "{}",
        row.trigger_key
    );

    // Vor dem Takt wartet der Lauf; danach ist er fertig und das Protokoll hat die Felder.
    let waiting = r.call("get_run", json!({ "run_id": run_id })).unwrap();
    assert_eq!(waiting["run"]["state"], "queued");
    assert_eq!(waiting["finished"], false);
    r.engine.tick().unwrap();
    let log = r.call("get_run", json!({ "run_id": run_id })).unwrap();
    assert_eq!(log["run"]["state"], "done", "{log}");
    assert_eq!(log["run"]["dry_run"], true);
    assert_eq!(log["run"]["origin"], "agent");
    assert_eq!(log["run"]["workflow_name"], "Testablauf");
    assert_eq!(log["finished"], true);
    assert_eq!(log["steps_total"], 2);
    let steps = log["steps"].as_array().unwrap();
    assert_eq!(steps[0]["step_id"], "a");
    assert_eq!(steps[0]["action"], "notify.local");
    assert!(steps[0]["title"].is_string());
    assert_eq!(steps[1]["step_id"], "b");
    assert!(log["run"]["ended_at"].is_i64());

    // list_workflows kennt den Lauf jetzt als letzten.
    let list = r.call("list_workflows", json!({})).unwrap();
    assert_eq!(list["workflows"][0]["last_run"]["run_id"], run_id.as_str());
    assert_eq!(list["workflows"][0]["last_run"]["origin"], "agent");
}

#[test]
fn live_needs_an_armed_and_enabled_workflow() {
    let r = rig();
    // Neu gespeichert und eingeschaltet, aber nicht scharf.
    let unarmed = r
        .engine
        .save_workflow(None, &def(vec![note("a")]))
        .unwrap()
        .id;
    r.engine.set_enabled(&unarmed, true).unwrap();
    let e = r
        .call(
            "run_workflow",
            json!({ "workflow_id": unarmed, "live": true }),
        )
        .unwrap_err();
    assert!(e.contains("nicht scharf geschaltet"), "{e}");
    assert!(r.runs().is_empty(), "es entsteht KEIN Lauf");

    // Scharf, aber ausgeschaltet.
    let armed = r.armed();
    r.engine.set_enabled(&armed, false).unwrap();
    let e = r
        .call(
            "run_workflow",
            json!({ "workflow_id": armed, "live": true }),
        )
        .unwrap_err();
    assert!(e.contains("ausgeschaltet"), "{e}");
    assert!(r.runs().is_empty());

    // Eingeschaltet und scharf: der Lauf ist echt (nicht Trockenlauf).
    r.engine.set_enabled(&armed, true).unwrap();
    let v = r
        .call(
            "run_workflow",
            json!({ "workflow_id": armed, "live": true }),
        )
        .unwrap();
    assert_eq!(v["dry_run"], false);
    let row = store::get_run(&r.fx.conn(), v["run_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert!(!row.dry_run);
    assert_eq!(row.origin.as_str(), "agent");
    // `live: false` ist dasselbe wie nichts: Trockenlauf.
    let v = r
        .call(
            "run_workflow",
            json!({ "workflow_id": armed, "live": false }),
        )
        .unwrap();
    assert_eq!(v["dry_run"], true);
}

#[test]
fn live_is_refused_for_event_triggered_workflows() {
    let r = rig();
    let mut d = def(vec![note("a")]);
    d["trigger"] = json!({ "type": "schedule", "every": "daily", "at": "09:00" });
    let timed = armed_workflow(&r.engine, &d);
    let e = r
        .call(
            "run_workflow",
            json!({ "workflow_id": timed, "live": true }),
        )
        .unwrap_err();
    assert!(e.contains("Ereignis"), "{e}");
    assert!(r.runs().is_empty());
    // Der Trockenlauf geht (er plant nur).
    assert_eq!(
        r.call("run_workflow", json!({ "workflow_id": timed }))
            .unwrap()["dry_run"],
        true
    );
    // „Durch einen Agenten starten“ und „Von Hand“ dürfen scharf laufen.
    let mut d = def(vec![note("a")]);
    d["trigger"] = json!({ "type": "agent" });
    let by_agent = armed_workflow(&r.engine, &d);
    assert_eq!(
        r.call(
            "run_workflow",
            json!({ "workflow_id": by_agent, "live": true })
        )
        .unwrap()["dry_run"],
        false
    );
    let list = r.call("list_workflows", json!({})).unwrap();
    let can = |id: &str| {
        list["workflows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["id"] == id)
            .map(|w| w["can_run_live"].clone())
            .unwrap()
    };
    assert_eq!(can(&timed), false);
    assert_eq!(can(&by_agent), true);
}

#[test]
fn a_disabled_workflow_allows_only_the_dry_run() {
    let r = rig();
    let id = r.armed();
    r.engine.set_enabled(&id, false).unwrap();
    let v = r
        .call("run_workflow", json!({ "workflow_id": id }))
        .unwrap();
    assert_eq!(v["dry_run"], true, "zum Ausprobieren");
}

#[test]
fn the_same_request_id_is_the_same_run() {
    let r = rig();
    let id = r.armed();
    let a = r
        .call(
            "run_workflow",
            json!({ "workflow_id": id, "request_id": "abbruch-1" }),
        )
        .unwrap();
    let b = r
        .call(
            "run_workflow",
            json!({ "workflow_id": id, "request_id": "abbruch-1" }),
        )
        .unwrap();
    assert_eq!(a["run_id"], b["run_id"]);
    assert_eq!(a["created"], true);
    assert_eq!(b["created"], false);
    assert_eq!(r.runs().len(), 1);
    let c = r
        .call(
            "run_workflow",
            json!({ "workflow_id": id, "request_id": "abbruch-2" }),
        )
        .unwrap();
    assert_ne!(a["run_id"], c["run_id"]);
    assert_eq!(r.runs().len(), 2);
}

#[test]
fn without_a_request_id_every_call_is_a_run() {
    let r = rig();
    let id = r.armed();
    let a = r
        .call("run_workflow", json!({ "workflow_id": id }))
        .unwrap();
    let b = r
        .call("run_workflow", json!({ "workflow_id": id }))
        .unwrap();
    assert_ne!(a["run_id"], b["run_id"]);
    assert_eq!(r.runs().len(), 2);
}

#[test]
fn variables_are_passed_and_checked() {
    let r = rig();
    let mut d = def(vec![note("a")]);
    d["variables"] = json!({ "thema": { "type": "string" } });
    let id = armed_workflow(&r.engine, &d);
    let e = r
        .call("run_workflow", json!({ "workflow_id": id }))
        .unwrap_err();
    assert!(e.contains("thema"), "{e}");
    let e = r
        .call(
            "run_workflow",
            json!({ "workflow_id": id, "vars": { "thema": 5 } }),
        )
        .unwrap_err();
    assert!(e.contains("thema"), "{e}");
    let e = r
        .call(
            "run_workflow",
            json!({ "workflow_id": id, "vars": { "fremd": "x" } }),
        )
        .unwrap_err();
    assert!(e.contains("fremd"), "{e}");
    assert!(r.runs().is_empty());
    let v = r
        .call(
            "run_workflow",
            json!({ "workflow_id": id, "vars": { "thema": "Budget" } }),
        )
        .unwrap();
    let row = store::get_run(&r.fx.conn(), v["run_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert!(row.context_json.contains("Budget"));
}

#[test]
fn arguments_are_checked_strictly() {
    let r = rig();
    let id = r.armed();
    let cases = [
        ("run_workflow", json!({})),
        ("run_workflow", json!({ "workflow_id": 5 })),
        ("run_workflow", json!({ "workflow_id": "../x" })),
        ("run_workflow", json!({ "workflow_id": "a b" })),
        ("run_workflow", json!({ "workflow_id": id, "live": "ja" })),
        ("run_workflow", json!({ "workflow_id": id, "vars": ["a"] })),
        (
            "run_workflow",
            json!({ "workflow_id": id, "vars": { "x": "y".repeat(MAX_VARS_BYTES) } }),
        ),
        (
            "run_workflow",
            json!({ "workflow_id": id, "request_id": "a/b" }),
        ),
        (
            "run_workflow",
            json!({ "workflow_id": id, "dry_run": false }),
        ),
        ("run_workflow", json!("text")),
        ("get_run", json!({})),
        ("get_run", json!({ "run_id": ["x"] })),
        ("get_run", json!({ "run_id": "x", "full": true })),
        ("get_run", json!({ "run_id": "has space" })),
        ("list_workflows", json!({ "limit": 5 })),
    ];
    for (tool, args) in cases {
        assert!(r.call(tool, args.clone()).is_err(), "{tool} {args}");
    }
    assert!(r.runs().is_empty(), "nichts wurde eingereiht");
    // Unbekannte Kennungen sind ein Klartext, kein Absturz.
    let e = r
        .call("get_run", json!({ "run_id": "01XXXXXXXXXXXXXXXXXXXXXXXX" }))
        .unwrap_err();
    assert!(e.contains("gibt es nicht"), "{e}");
    let e = r
        .call("run_workflow", json!({ "workflow_id": "gibtsnicht" }))
        .unwrap_err();
    assert!(e.contains("gibt es nicht"), "{e}");
}

// ---------------------------------------------------------------------------
// Rechte ueber die Bruecke
// ---------------------------------------------------------------------------

#[test]
fn run_workflow_without_the_right_is_refused_and_no_run_exists() {
    let r = rig();
    let id = r.armed();
    // Ein neuer Zugang hat kein Werkzeugrecht: das Werkzeug fehlt in der Liste, ein Aufruf ist `tool_off`.
    let names: Vec<String> = r
        .bridge
        .tools_list(&r.client)
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(!names
        .iter()
        .any(|n| n == "run_workflow" || n == "list_workflows" || n == "get_run"));
    let e = r
        .through_bridge("run_workflow", json!({ "workflow_id": id }), None)
        .unwrap_err();
    assert_eq!(e.code, code::TOOL_OFF);
    assert!(r.runs().is_empty());
    let e = r
        .through_bridge("list_workflows", json!({}), None)
        .unwrap_err();
    assert_eq!(e.code, code::TOOL_OFF);

    // Nur Lesen: Liste und Protokoll gehen, Starten nicht.
    r.grant("list_workflows", GrantMode::Allow);
    r.grant("get_run", GrantMode::Allow);
    let v = done(r.through_bridge("list_workflows", json!({}), None).unwrap());
    assert_eq!(v["count"], 1);
    let e = r
        .through_bridge("run_workflow", json!({ "workflow_id": id }), None)
        .unwrap_err();
    assert_eq!(e.code, code::TOOL_OFF);
    assert!(r.runs().is_empty());

    // Das Recht auf „aus“ zuruecksetzen: weg.
    r.grant("list_workflows", GrantMode::Off);
    assert_eq!(
        r.through_bridge("list_workflows", json!({}), None)
            .unwrap_err()
            .code,
        code::TOOL_OFF
    );
    // Die Verweigerungen stehen im Audit, ohne dass ein Handler lief.
    let rows = audit::list(&r.fx.conn(), &AuditFilter::default(), 100).unwrap();
    assert!(rows.iter().any(|a| a.outcome == "denied"));
}

#[test]
fn ask_binds_the_approval_to_live() {
    let r = rig();
    let id = r.armed();
    r.grant("run_workflow", GrantMode::Ask);
    // Fragen: nichts laeuft, bis der Nutzer entscheidet.
    let aid = match r
        .through_bridge(
            "run_workflow",
            json!({ "workflow_id": id, "live": true }),
            None,
        )
        .unwrap()
    {
        CallStep::Wait(a) => a,
        other => panic!("{other:?}"),
    };
    assert!(r.runs().is_empty());
    crate::managers::integrations::approvals::decide(&r.fx.conn(), &aid, true, NOW).unwrap();
    // Die Freigabe fuer „scharf“ taugt nicht fuer den Trockenlauf und umgekehrt ...
    let e = r
        .through_bridge("run_workflow", json!({ "workflow_id": id }), Some(&aid))
        .unwrap_err();
    assert_eq!(e.code, code::APPROVAL_MISMATCH, "{e}");
    assert!(r.runs().is_empty());
    // ... mit denselben Argumenten laeuft der Aufruf, genau einmal.
    let v = done(
        r.through_bridge(
            "run_workflow",
            json!({ "workflow_id": id, "live": true }),
            Some(&aid),
        )
        .unwrap(),
    );
    assert_eq!(v["dry_run"], false);
    assert_eq!(r.runs().len(), 1);
    let e = r
        .through_bridge(
            "run_workflow",
            json!({ "workflow_id": id, "live": true }),
            Some(&aid),
        )
        .unwrap_err();
    assert_eq!(e.code, code::APPROVAL_USED);
    assert_eq!(r.runs().len(), 1);
}

#[test]
fn a_started_run_is_attributed_to_the_agent_in_the_audit() {
    let r = rig();
    let id = r.armed();
    r.grant("run_workflow", GrantMode::Allow);
    let v = done(
        r.through_bridge("run_workflow", json!({ "workflow_id": id }), None)
            .unwrap(),
    );
    let run = store::get_run(&r.fx.conn(), v["run_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    // Der Schluessel nennt den Zugang (nicht den Namen: der kann sich aendern).
    assert!(
        run.trigger_key.contains(&r.client.id),
        "{}",
        run.trigger_key
    );
    let rows = audit::list(&r.fx.conn(), &AuditFilter::default(), 100).unwrap();
    assert!(rows.iter().any(|a| a.caller == "agent_external"
        && a.capability.as_deref() == Some("workflow.run")
        && a.outcome == "ok"
        && a.target
            .as_deref()
            .is_some_and(|t| t.contains("Claude Code"))));
}

// ---------------------------------------------------------------------------
// Protokoll ohne Geheimnisse
// ---------------------------------------------------------------------------

#[test]
fn get_run_hides_secrets_and_trigger_data() {
    let r = rig();
    let id = r.armed();
    let enq = r
        .engine
        .enqueue(&EnqueueRequest {
            workflow_id: id.clone(),
            trigger_key: "test:geheim".to_string(),
            origin: crate::managers::workflows::model::Origin::Trigger,
            trigger: json!({ "password": "hunter2", "mail_text": "VERTRAULICHER-INHALT" }),
            vars: Default::default(),
            force_dry_run: true,
        })
        .unwrap();
    r.engine.tick().unwrap();
    let log = r.call("get_run", json!({ "run_id": enq.run_id })).unwrap();
    let text = log.to_string();
    for forbidden in [
        "hunter2",
        "VERTRAULICHER-INHALT",
        "context",
        "definition",
        "input",
        "params",
    ] {
        assert!(
            !text.contains(forbidden),
            "{forbidden} im Protokoll: {text}"
        );
    }
    assert_eq!(log["run"]["origin"], "trigger");
    assert_eq!(log["run"]["state"], "done");
}

#[test]
fn step_outputs_and_errors_are_scrubbed_and_bounded() {
    let out = step_output(Some(
        r#"{"ok":true,"token":"abc123","url":"https://example.org/zugang/geheimerpfad?sig=1","text":"Hallo","nested":{"password":"x","list":[1,2,3]}}"#,
    ));
    assert_eq!(out["ok"], true);
    assert_eq!(out["token"], "***");
    assert_eq!(out["nested"]["password"], "***");
    assert_eq!(out["nested"]["list"], json!([1, 2, 3]));
    let url = out["url"].as_str().unwrap();
    assert!(url.starts_with("https://example.org"), "{url}");
    assert!(!url.contains("geheimerpfad"), "{url}");
    assert_eq!(out["text"], "Hallo");
    // Riesige Ausgabe: nur eine Notiz mit der Groesse.
    let big = json!({ "a": "x".repeat(150), "b": "y".repeat(150), "c": "z".repeat(150), "d": "w".repeat(150),
                      "e": "v".repeat(150), "f": "u".repeat(150), "g": "t".repeat(150), "h": "s".repeat(150),
                      "i": "r".repeat(150), "j": "q".repeat(150), "k": "p".repeat(150), "l": "o".repeat(150),
                      "m": "n".repeat(150), "n": "m".repeat(150), "o": "l".repeat(150) })
    .to_string();
    let out = step_output(Some(&big));
    assert_eq!(out["truncated"], true);
    assert!(out["bytes"].as_u64().unwrap() > MAX_STEP_OUTPUT_BYTES as u64);
    assert_eq!(step_output(None), Value::Null);
    // Die eingesetzten Parameter eines Plans bleiben beim Nutzer.
    let plan = step_output(Some(
        r#"{"status":"planned","params":{"to":"a@b.de"},"effect":"Mail senden"}"#,
    ));
    assert_eq!(
        plan,
        json!({ "status": "planned", "effect": "Mail senden" })
    );
    // Kein JSON: als Text behandelt, auch geschwaerzt.
    let text = step_output(Some("Bearer abcdefghijklmnop"));
    assert!(!text.to_string().contains("abcdefghijklmnop"), "{text}");
}

// ---------------------------------------------------------------------------
// Fehlerfaelle der Umgebung
// ---------------------------------------------------------------------------

#[test]
fn without_an_engine_the_tools_say_so() {
    let r = rig_with_db(Fx::new(), None);
    for (tool, args) in [
        ("list_workflows", json!({})),
        ("run_workflow", json!({ "workflow_id": "w" })),
        ("get_run", json!({ "run_id": "r" })),
    ] {
        let e = r.call(tool, args).unwrap_err();
        assert!(e.contains("nicht verfügbar"), "{tool}: {e}");
    }
}

#[test]
fn a_store_failure_is_a_plain_message() {
    let fx = Fx::new();
    let broken = Engine::new(
        fx.db_path
            .parent()
            .unwrap()
            .join("gibt-es-nicht")
            .join("x.db"),
        roomy_gate(),
        FakeClock::new(),
        EngineConfig::default(),
    );
    let r = rig_with_db(fx, Some(broken));
    let e = r.call("list_workflows", json!({})).unwrap_err();
    assert!(e.contains("nicht verfügbar"), "{e}");
    assert!(!e.contains("gibt-es-nicht"), "kein Pfad im Text: {e}");
    let e = r
        .call("run_workflow", json!({ "workflow_id": "w" }))
        .unwrap_err();
    assert!(
        e.contains("nicht verfügbar") || e.contains("nichts gestartet"),
        "{e}"
    );
}

#[test]
fn engine_errors_become_plain_german_messages() {
    use crate::managers::workflows::store::WorkflowError;
    let queue = wf_error(&WorkflowError::QueueFull(
        "zu viele wartende Läufe".to_string(),
    ));
    assert!(queue.contains("wartende"), "{queue}");
    let store = wf_error(&WorkflowError::Store(
        "disk I/O error at C:\\secret\\db".to_string(),
    ));
    assert!(!store.contains("secret"), "{store}");
    assert!(wf_error(&WorkflowError::NotFound("x".to_string())).contains("gibt es nicht"));
    assert!(wf_error(&WorkflowError::Disabled(
        "Der Ablauf ist ausgeschaltet.".to_string()
    ))
    .contains("ausgeschaltet"));
}
