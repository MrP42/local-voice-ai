//! Tests des MCP-Proxys fuer die schreibenden Werkzeuge (A8) und der zustandslosen Fassung
//! 2026-07-28: Uebersetzung auf die Agentenbruecke (Attrappe der Gegenstelle), `pending` als
//! Hinweis, Schalter, Versionen und `_meta`, dazu `get_provenance`.

use std::sync::Mutex;

use super::link::{LinkError, LinkErrorKind};
use super::protocol::{META_CLIENT_CAPABILITIES, META_PROTOCOL_VERSION, META_SERVER_INFO};
use super::testkit::*;
use super::*;
use crate::managers::provenance::{self, ActorKind, NewProvenance, SourceRef, SubjectKind};

const TOKEN: &str = "lvat_GEHEIMGEHEIMGEHEIMGEHEIMGEHEIMGEHEIMGEHEIM";

struct FakeLink {
    tools: Mutex<Result<Vec<Value>, LinkError>>,
    reply: Mutex<Result<LinkReply, LinkError>>,
    listed: Mutex<usize>,
    calls: Mutex<Vec<(String, Value, Option<String>)>>,
}

impl FakeLink {
    fn new() -> Arc<Self> {
        let tools = [
            json!({"name": "create_meeting", "title": "Besprechung anlegen", "description": "Legt an.",
                   "inputSchema": {"type": "object", "properties": {"title": {"type": "string"}},
                                   "additionalProperties": false},
                   "annotations": {"readOnlyHint": false}, "mode": "allow"}),
            json!({"name": "get_action_status", "title": "Stand", "description": "Stand.",
                   "inputSchema": {"type": "object", "properties": {"approval_id": {"type": "string"}},
                                   "required": ["approval_id"]},
                   "annotations": {"readOnlyHint": true}, "mode": "allow"}),
            json!({"name": "rm_rf", "description": "gibt es nicht", "inputSchema": {"type": "object"}}),
        ];
        Arc::new(Self {
            tools: Mutex::new(Ok(tools.iter().filter_map(link::to_mcp_tool).collect())),
            reply: Mutex::new(Ok(LinkReply::Done(json!({"meeting_id": "M1"})))),
            listed: Mutex::new(0),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn set_reply(&self, r: Result<LinkReply, LinkError>) {
        *self.reply.lock().unwrap() = r;
    }
}

impl BridgeLink for FakeLink {
    fn tools(&self) -> Result<Vec<Value>, LinkError> {
        *self.listed.lock().unwrap() += 1;
        self.tools.lock().unwrap().clone()
    }

    fn call(
        &self,
        name: &str,
        args: &Value,
        approval_id: Option<&str>,
    ) -> Result<LinkReply, LinkError> {
        self.calls.lock().unwrap().push((
            name.to_string(),
            args.clone(),
            approval_id.map(str::to_string),
        ));
        self.reply.lock().unwrap().clone()
    }
}

struct Rig {
    fx: Fx,
    server: Server,
    link: Arc<FakeLink>,
}

fn rig() -> Rig {
    let fx = Fx::new();
    let link = FakeLink::new();
    let server = Server::with_link(config_in(fx.dir.path()), link.clone());
    Rig { fx, server, link }
}

fn rpc(server: &Server, method: &str, params: Value) -> Value {
    let line = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    serde_json::from_str(&server.handle_line(&line.to_string()).expect("Antwort")).unwrap()
}

fn modern(extra: Value) -> Value {
    let mut p = json!({ "_meta": {
        META_PROTOCOL_VERSION: "2026-07-28",
        META_CLIENT_CAPABILITIES: {},
        "io.modelcontextprotocol/clientInfo": {"name": "t", "version": "1"}
    }});
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    p
}

fn names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

fn call(server: &Server, name: &str, args: Value) -> Value {
    rpc(
        server,
        "tools/call",
        json!({ "name": name, "arguments": args }),
    )
}

// ---------------------------------------------------------------------------
// Schreibende Werkzeuge ueber die Bruecke
// ---------------------------------------------------------------------------

#[test]
fn without_a_link_only_the_read_tools_exist_and_write_tools_explain_how_to_get_access() {
    let fx = Fx::new();
    let list = rpc(&fx.server, "tools/list", json!({}));
    assert_eq!(names(&list).len(), 5, "nur die lesenden Werkzeuge");
    let reply = call(&fx.server, "create_meeting", json!({}));
    assert_eq!(reply["result"]["isError"], true);
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("LVA_AGENT_TOKEN") && text.contains("claude mcp add"),
        "{text}"
    );
}

#[test]
fn with_a_link_the_write_tools_follow_the_read_tools_and_carry_the_approval_argument() {
    let r = rig();
    let list = rpc(&r.server, "tools/list", json!({}));
    let n = names(&list);
    assert_eq!(
        n,
        [
            "list_meetings",
            "search_meetings",
            "get_meeting",
            "get_transcript",
            "get_provenance",
            "create_meeting",
            "get_action_status"
        ]
    );
    let tools = list["result"]["tools"].as_array().unwrap();
    let create = tools
        .iter()
        .find(|t| t["name"] == "create_meeting")
        .unwrap();
    assert!(create.get("mode").is_none(), "das Recht gehoert der App");
    assert_eq!(create["annotations"]["readOnlyHint"], false);
    assert_eq!(
        create["inputSchema"]["properties"]["approval_id"]["type"],
        "string"
    );
    assert_eq!(
        create["inputSchema"]["properties"]["title"]["type"],
        "string"
    );
    let status = tools
        .iter()
        .find(|t| t["name"] == "get_action_status")
        .unwrap();
    assert_eq!(status["inputSchema"]["required"], json!(["approval_id"]));
    assert!(*r.link.listed.lock().unwrap() == 1);
}

#[test]
fn the_setting_off_hides_and_blocks_the_write_tools_without_touching_the_link() {
    let r = rig();
    r.fx.settings(false, true);
    let list = rpc(&r.server, "tools/list", json!({}));
    assert_eq!(names(&list).len(), 5);
    assert_eq!(
        *r.link.listed.lock().unwrap(),
        0,
        "bei AUS wird die App nicht einmal gefragt"
    );
    let reply = call(&r.server, "create_meeting", json!({"title": "x"}));
    assert_eq!(reply["result"]["isError"], true);
    assert!(reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("ausgeschaltet"));
    assert!(r.link.calls.lock().unwrap().is_empty());
}

#[test]
fn a_done_reply_is_a_text_result_with_structured_content() {
    let r = rig();
    let reply = call(&r.server, "create_meeting", json!({"title": "Neu"}));
    let result = &reply["result"];
    assert_eq!(result["isError"], false);
    assert_eq!(result["structuredContent"]["meeting_id"], "M1");
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["meeting_id"], "M1");
    let calls = r.link.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![("create_meeting".to_string(), json!({"title": "Neu"}), None)]
    );
}

#[test]
fn pending_is_a_hint_and_not_an_error() {
    let r = rig();
    r.link.set_reply(Ok(LinkReply::Pending {
        approval_id: "01ABCAPPROVAL".to_string(),
        message: "Die App wartet auf die Freigabe des Nutzers.".to_string(),
    }));
    let reply = call(&r.server, "create_meeting", json!({}));
    let result = &reply["result"];
    assert!(reply.get("error").is_none(), "kein Protokollfehler");
    assert_eq!(
        result["isError"], false,
        "pending ist ein Hinweis, kein Fehler"
    );
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("01ABCAPPROVAL")
            && text.contains("kein Fehler")
            && text.contains("approval_id"),
        "{text}"
    );
    assert_eq!(
        result["structuredContent"],
        json!({"status": "pending", "approval_id": "01ABCAPPROVAL"})
    );
}

#[test]
fn bridge_errors_become_tool_errors_with_a_hint_and_never_leak_the_token() {
    let r = rig();
    let cases = [
        LinkError {
            kind: LinkErrorKind::NotRunning,
            text: "Local Voice AI läuft nicht.".into(),
        },
        LinkError {
            kind: LinkErrorKind::Auth,
            text: "Der Zugang wurde nicht angenommen.".into(),
        },
        LinkError {
            kind: LinkErrorKind::Denied,
            text: "Nicht erlaubt: aus.".into(),
        },
        LinkError {
            kind: LinkErrorKind::Timeout,
            text: "Zeitüberschreitung.".into(),
        },
        LinkError {
            kind: LinkErrorKind::Failed,
            text: "Es ging schief.".into(),
        },
    ];
    for e in cases {
        let expected = e.text.clone();
        r.link.set_reply(Err(e));
        let reply = call(&r.server, "create_meeting", json!({}));
        assert!(reply.get("error").is_none());
        assert_eq!(reply["result"]["isError"], true);
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(text, expected);
        assert!(!text.contains(TOKEN) && !reply.to_string().contains("lvat_"));
    }
}

#[test]
fn the_approval_id_is_passed_on_as_its_own_field_not_as_an_argument() {
    let r = rig();
    let reply = call(
        &r.server,
        "create_meeting",
        json!({"title": "T", "approval_id": "A1"}),
    );
    assert_eq!(reply["result"]["isError"], false);
    // Das Statuswerkzeug behaelt sein Argument.
    call(&r.server, "get_action_status", json!({"approval_id": "A2"}));
    let calls = r.link.calls.lock().unwrap().clone();
    assert_eq!(
        calls[0],
        (
            "create_meeting".to_string(),
            json!({"title": "T"}),
            Some("A1".to_string())
        )
    );
    assert_eq!(
        calls[1],
        (
            "get_action_status".to_string(),
            json!({"approval_id": "A2"}),
            None
        )
    );
    // Ein falscher Typ wird nicht weitergereicht.
    let bad = call(&r.server, "create_meeting", json!({"approval_id": 5}));
    assert_eq!(bad["result"]["isError"], true);
    assert_eq!(r.link.calls.lock().unwrap().len(), 2);
}

#[test]
fn unknown_tools_stay_protocol_errors_and_the_other_catalog_names_go_to_the_bridge() {
    let r = rig();
    let reply = call(&r.server, "delete_meeting", json!({}));
    assert_eq!(reply["error"]["code"], INVALID_PARAMS);
    // Auch ein Katalogwerkzeug, das die App dem Zugang nicht anbietet, geht zur App: sie entscheidet.
    let reply = call(&r.server, "start_recording", json!({}));
    assert_eq!(reply["result"]["isError"], false);
    assert_eq!(r.link.calls.lock().unwrap()[0].0, "start_recording");
}

// ---------------------------------------------------------------------------
// Zustandslose Fassung 2026-07-28
// ---------------------------------------------------------------------------

#[test]
fn a_modern_request_gets_result_type_server_info_and_cache_hints() {
    let r = rig();
    let list = rpc(&r.server, "tools/list", modern(json!({})));
    let result = &list["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(
        result["ttlMs"], 0,
        "die Liste haengt von Schalter und App ab: sofort veraltet"
    );
    assert_eq!(result["cacheScope"], "private");
    assert_eq!(result["_meta"][META_SERVER_INFO]["name"], "local-voice-ai");
    assert_eq!(names(&list).len(), 7);
    let tool = rpc(
        &r.server,
        "tools/call",
        modern(json!({"name": "create_meeting", "arguments": {}})),
    );
    assert_eq!(tool["result"]["resultType"], "complete");
    assert_eq!(tool["result"]["isError"], false);
    let ping = rpc(&r.server, "ping", modern(json!({})));
    assert_eq!(ping["result"]["resultType"], "complete");
}

#[test]
fn a_legacy_request_stays_exactly_as_before() {
    let r = rig();
    let list = rpc(&r.server, "tools/list", json!({}));
    for key in ["resultType", "ttlMs", "cacheScope", "_meta"] {
        assert!(
            list["result"].get(key).is_none(),
            "{key} gehoert nicht in die alte Fassung"
        );
    }
    let init = rpc(
        &r.server,
        "initialize",
        json!({"protocolVersion": "2025-11-25"}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    assert!(init["result"].get("resultType").is_none());
    // Auch ein Handshake mit der neuen Versionsnummer waehlt den alten Ablauf.
    let init = rpc(
        &r.server,
        "initialize",
        json!({"protocolVersion": "2026-07-28"}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    // Eine alte Version in `_meta` ist ebenfalls alt.
    let list = rpc(
        &r.server,
        "tools/list",
        json!({"_meta": {META_PROTOCOL_VERSION: "2025-06-18"}}),
    );
    assert!(list["result"].get("resultType").is_none());
}

#[test]
fn server_discover_names_versions_capabilities_and_identity() {
    let r = rig();
    for params in [modern(json!({})), json!({})] {
        let d = rpc(&r.server, "server/discover", params);
        let result = &d["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(
            result["supportedVersions"],
            json!(["2026-07-28", "2025-11-25", "2025-06-18"])
        );
        assert!(result["capabilities"]["tools"].is_object());
        assert_eq!(result["_meta"][META_SERVER_INFO]["name"], "local-voice-ai");
        assert_eq!(
            result["_meta"][META_SERVER_INFO]["version"],
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(result["ttlMs"], 0);
        assert_eq!(result["cacheScope"], "private");
        assert!(!result["instructions"].as_str().unwrap().is_empty());
    }
}

#[test]
fn an_unsupported_version_is_refused_with_the_supported_list_on_every_method() {
    let r = rig();
    let bad = |extra: Value| {
        let mut p = modern(extra);
        p["_meta"][META_PROTOCOL_VERSION] = json!("1900-01-01");
        p
    };
    for method in ["tools/list", "tools/call", "server/discover", "ping"] {
        let reply = rpc(&r.server, method, bad(json!({"name": "create_meeting"})));
        assert_eq!(reply["error"]["code"], -32022, "{method}");
        assert_eq!(
            reply["error"]["data"]["supported"],
            json!(["2026-07-28", "2025-11-25", "2025-06-18"])
        );
        assert_eq!(reply["error"]["data"]["requested"], "1900-01-01");
    }
    assert!(r.link.calls.lock().unwrap().is_empty(), "nichts lief");
}

#[test]
fn a_modern_request_without_client_capabilities_is_invalid_params() {
    let r = rig();
    let mut p = modern(json!({"name": "create_meeting", "arguments": {}}));
    p["_meta"]
        .as_object_mut()
        .unwrap()
        .remove(META_CLIENT_CAPABILITIES);
    let reply = rpc(&r.server, "tools/call", p);
    assert_eq!(reply["error"]["code"], INVALID_PARAMS);
    assert!(reply["error"]["message"]
        .as_str()
        .unwrap()
        .contains("clientCapabilities"));
    assert!(r.link.calls.lock().unwrap().is_empty());
}

#[test]
fn notifications_with_meta_still_get_no_answer() {
    let r = rig();
    let line = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": modern(json!({"requestId": 1}))});
    assert_eq!(r.server.handle_line(&line.to_string()), None);
}

// ---------------------------------------------------------------------------
// get_provenance (lesend)
// ---------------------------------------------------------------------------

fn record(fx: &Fx, kind: SubjectKind, id: &str, op: &str) {
    let conn = fx.store.get_connection().unwrap();
    let mut e = NewProvenance::new(kind, id, op, ActorKind::AgentExternal);
    e.actor_ref = Some("C1".into());
    e.provider = Some("lokal".into());
    e.model_label = Some("Gemma 4 E4B".into());
    e.prompt_tokens = Some(1200);
    e.completion_tokens = Some(300);
    e.duration_ms = Some(4200);
    e.confidence = Some(0.8);
    e.sources = vec![SourceRef::new("youtube", "dQw4w9WgXcQ", Some("Ein Video"))];
    e.params = Some(json!({"geheim": "darf nicht heraus", "token": "lvat_x"}));
    provenance::record(&conn, &e).unwrap();
}

#[test]
fn get_provenance_names_model_tokens_sources_and_actor_but_no_content_or_params() {
    let fx = Fx::new();
    record(&fx, SubjectKind::Transcript, &fx.a, "stt");
    let out = fx.tool_json("get_provenance", json!({"id": fx.a, "part": "transcript"}));
    assert_eq!(out["meeting_id"], fx.a.as_str());
    let entries = out["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{out}");
    let e = &entries[0];
    assert_eq!(e["part"], "transcript");
    assert_eq!(e["operation"], "stt");
    assert_eq!(e["actor"], "agent_external");
    assert_eq!(e["model"], "Gemma 4 E4B");
    assert_eq!(e["prompt_tokens"], 1200);
    assert_eq!(e["completion_tokens"], 300);
    assert_eq!(e["duration_ms"], 4200);
    assert_eq!(e["sources"][0]["kind"], "youtube");
    assert_eq!(e["sources"][0]["title"], "Ein Video");
    assert_eq!(e["origin"], "recorded");
    let text = out.to_string();
    assert!(
        !text.contains("geheim") && !text.contains("lvat_"),
        "keine Parameter: {text}"
    );
    assert!(e.get("params").is_none() && e.get("params_json").is_none());
    // Ohne `part`: alle drei Inhalte (hier nur das Transkript hat einen Eintrag, das Protokoll
    // leitet seine Herkunft aus den Altdaten ab, falls es welche hat).
    let all = fx.tool_json("get_provenance", json!({"id": fx.a}));
    assert!(all["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["part"] == "transcript"));
}

#[test]
fn get_provenance_refuses_unknown_deleted_and_unfinished_meetings_and_bad_arguments() {
    let fx = Fx::new();
    for id in ["01GIBTESNICHT", fx.deleted.as_str()] {
        let (text, is_error) = fx.tool("get_provenance", json!({"id": id}));
        assert!(is_error && text.contains("nicht gefunden"), "{id}: {text}");
    }
    let (text, is_error) = fx.tool("get_provenance", json!({"id": fx.recording}));
    assert!(is_error && text.contains("noch nicht fertig"), "{text}");
    let (text, is_error) = fx.tool("get_provenance", json!({"id": fx.a, "part": "alles"}));
    assert!(is_error && text.contains("part"), "{text}");
    let (_, is_error) = fx.tool("get_provenance", json!({}));
    assert!(is_error);
}

#[test]
fn get_provenance_on_a_missing_database_is_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), true, true);
    let server = Server::new(config_in(dir.path()));
    let reply = call(&server, "get_provenance", json!({"id": "x"}));
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(v["entries"], json!([]));
}
