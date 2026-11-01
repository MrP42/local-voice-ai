//! Tests der lesenden Werkzeuge fuer das Sprachgespraech: echtes Tor, echtes Audit, Attrappen-Handler.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::agent_bridge::catalog::{ToolHandler, ToolSpec};
use crate::managers::integrations::approvals;
use crate::managers::integrations::audit::{self, AuditFilter};
use crate::managers::integrations::model::{GrantMode, IntegrationPatch};
use crate::managers::integrations::store;
use crate::managers::integrations::test_support::Fx;
use super::knowledge::{
    ConversationInfo, ConversationView, Dictation, EntryInfo, EventInfo, MeetingInfo, PageInfo,
};

const NOW: i64 = 1_790_000_000_000;
const READING: [&str; 3] = ["meetings_list", "meetings_search", "meeting_transcript"];
const READING_OTHER: [&str; 1] = ["tts_providers_list"];
/// Lesend, aber ohne Wissensbereich: das Gespraech bietet es nicht an.
const READING_WITHOUT_AREA: [&str; 1] = ["list_workflows"];
const KNOWLEDGE: [&str; 9] = [
    "dictations_recent",
    "dictations_search",
    "conversations_list",
    "conversations_search",
    "conversation_get",
    "calendar_events",
    "day_overview",
    "tts_pages_list",
    "tts_page_text",
];
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
            .chain(READING_WITHOUT_AREA.iter())
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

/// Daten der Wissenswerkzeuge, fest verdrahtet.
struct Source;

impl knowledge::KnowledgeSource for Source {
    fn dictations(&self, _: Option<i64>, _: Option<i64>, _: usize) -> Result<Vec<Dictation>, String> {
        Ok(vec![Dictation {
            id: 1,
            at_ms: NOW,
            text: "GEHEIMES-DIKTAT".to_string(),
        }])
    }
    fn conversations(&self) -> Result<Vec<ConversationInfo>, String> {
        let one = |id: &str| ConversationInfo {
            id: id.to_string(),
            title: format!("Titel {id}"),
            updated_ms: NOW,
            entries: 2,
            last_text: "GEHEIMER-BEITRAG".to_string(),
        };
        Ok(vec![one("conv_laufend"), one("conv_alt")])
    }
    fn conversation_entries(&self, _: usize) -> Result<Vec<EntryInfo>, String> {
        Ok(Vec::new())
    }
    fn conversation(&self, _: &str) -> Result<ConversationView, String> {
        Err("nicht in diesem Test".to_string())
    }
    fn calendar(&self, _: i64, _: i64) -> Result<Vec<EventInfo>, String> {
        Ok(Vec::new())
    }
    fn meetings(&self, _: i64, _: i64, _: usize) -> Result<Vec<MeetingInfo>, String> {
        Ok(Vec::new())
    }
    fn pages(&self) -> Result<Vec<PageInfo>, String> {
        Ok(Vec::new())
    }
    fn page_text(&self, _: &str) -> Result<String, String> {
        Err("nicht in diesem Test".to_string())
    }
}

fn all_areas() -> Areas {
    Areas::from_settings(
        &["meetings", "dictations", "conversations", "calendar", "tts"].map(String::from),
        true,
    )
}

fn areas_of(keys: &[&str], transcripts: bool) -> Areas {
    Areas::from_settings(&keys.iter().map(|k| k.to_string()).collect::<Vec<_>>(), transcripts)
}

struct Rig {
    fx: Fx,
    fake: Arc<Fake>,
    registry: ToolRegistry,
    opts: VoiceOptions,
    conversation: Option<String>,
}

fn rig_with(reply: Value) -> Rig {
    let fake = Fake::all(reply);
    let mut registry = ToolRegistry::new();
    registry.register(fake.clone()).unwrap();
    Rig {
        fx: Fx::new(),
        fake,
        registry,
        opts: VoiceOptions { areas: all_areas() },
        conversation: Some("conv_laufend".to_string()),
    }
}

fn rig() -> Rig {
    rig_with(json!({"ok": true}))
}

impl Rig {
    fn list(&self) -> Vec<String> {
        list(&self.registry, self.opts)
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    fn call(&self, name: &str, args: &str) -> Result<String, String> {
        let conn = self.fx.conn();
        let session = Session {
            registry: &self.registry,
            source: &Source,
            conversation: self.conversation.as_deref(),
        };
        call(&conn, &session, self.opts, name, args, NOW)
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

/// Aufruf ohne Rig: nur ein Handler, alle Bereiche frei.
fn call_with(registry: &ToolRegistry, name: &str) -> Result<String, String> {
    let fx = Fx::new();
    let conn = fx.conn();
    let session = Session {
        registry,
        source: &Source,
        conversation: None,
    };
    let opts = VoiceOptions { areas: all_areas() };
    call(&conn, &session, opts, name, "{}", NOW)
}

#[test]
fn read_only_follows_the_catalog_annotation() {
    for entry in catalog::CATALOG {
        let hint = catalog::annotations(entry)["readOnlyHint"]
            .as_bool()
            .unwrap();
        assert_eq!(is_read_only(entry), hint, "{}", entry.name);
    }
    for name in READING
        .iter()
        .chain(READING_OTHER.iter())
        .chain(READING_WITHOUT_AREA.iter())
    {
        assert!(is_read_only(catalog::find(name).unwrap()), "{name}");
    }
    for name in WRITING {
        assert!(!is_read_only(catalog::find(name).unwrap()), "{name}");
    }
}

#[test]
fn knowledge_tools_are_not_in_the_bridge_catalog() {
    for name in KNOWLEDGE {
        assert!(catalog::find(name).is_none(), "{name} gehört nicht in den Katalog");
        assert!(knowledge::find(name).is_some(), "{name}");
    }
    assert_eq!(knowledge::specs().len(), KNOWLEDGE.len());
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
    // Die lesenden mit Bereich dagegen laufen.
    for name in READING.iter().chain(READING_OTHER.iter()) {
        assert!(listed.iter().any(|n| n == name), "{name} fehlt");
    }
    assert_eq!(r.call("meetings_list", "{}").unwrap(), r#"{"ok":true}"#);
}

#[test]
fn reading_tools_without_an_area_are_not_offered() {
    let r = rig();
    for name in READING_WITHOUT_AREA {
        assert!(!r.list().iter().any(|n| n == name), "{name}");
        let err = r.call(name, "{}").unwrap_err();
        assert!(err.contains("nicht zur Verfügung"), "{err}");
    }
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0);
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
        "dictations_recent ",
        "Dictations_Recent",
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
    assert!(call_with(&registry, "tts_providers_list")
        .unwrap_err()
        .contains("nicht verfügbar"));
    let opts = VoiceOptions { areas: all_areas() };
    let names: Vec<String> = list(&registry, opts).into_iter().map(|t| t.name).collect();
    assert!(names.contains(&"meetings_list".to_string()));
    assert!(!names.contains(&"tts_providers_list".to_string()));
}

#[test]
fn nothing_is_offered_by_default() {
    let mut r = rig();
    r.opts = VoiceOptions::default();
    assert!(r.opts.areas.is_empty());
    assert!(r.list().is_empty());
    for name in READING.iter().chain(READING_OTHER.iter()).chain(KNOWLEDGE.iter()) {
        let err = r.call(name, "{}").unwrap_err();
        assert!(err.contains("nicht freigegeben"), "{name}: {err}");
        assert!(err.contains("Gesprächseinstellungen → Wissen"), "{name}: {err}");
    }
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0);
    assert!(r.audit().is_empty(), "ein abgelehnter Aufruf schreibt kein Werkzeug-Audit");
}

#[test]
fn the_list_follows_the_areas() {
    let mut r = rig();
    let listed = |r: &Rig| r.list();
    // Alles frei.
    let all = listed(&r);
    assert_eq!(all.len(), READING.len() + READING_OTHER.len() + KNOWLEDGE.len());
    // Nur Besprechungen: die zwei Besprechungswerkzeuge und der Tagesüberblick.
    r.opts.areas = areas_of(&["meetings"], false);
    assert_eq!(listed(&r), vec!["meetings_list", "meetings_search", "day_overview"]);
    // Mit Transkripten kommt `meeting_transcript` hinzu.
    r.opts.areas = areas_of(&["meetings"], true);
    assert!(listed(&r).contains(&"meeting_transcript".to_string()));
    // Der Unterschalter allein öffnet nichts.
    r.opts.areas = areas_of(&["dictations"], true);
    assert!(!listed(&r).contains(&"meeting_transcript".to_string()));
    assert_eq!(listed(&r), vec!["dictations_recent", "dictations_search", "day_overview"]);
    r.opts.areas = areas_of(&["conversations"], false);
    assert_eq!(
        listed(&r),
        vec!["conversations_list", "conversations_search", "conversation_get", "day_overview"]
    );
    r.opts.areas = areas_of(&["calendar"], false);
    assert_eq!(listed(&r), vec!["calendar_events", "day_overview"]);
    // „Vorlesen“: die lesenden Anbieter-Werkzeuge und die Seiten, kein Tagesüberblick.
    r.opts.areas = areas_of(&["tts"], false);
    assert_eq!(listed(&r), vec!["tts_providers_list", "tts_pages_list", "tts_page_text"]);
}

#[test]
fn every_listed_tool_carries_a_parsable_schema_and_a_capability() {
    let r = rig();
    for t in list(&r.registry, r.opts) {
        let schema: Value = serde_json::from_str(&t.parameters_json).unwrap();
        assert_eq!(schema["type"], "object", "{}", t.name);
        assert!(!t.description.is_empty() && !t.title.is_empty(), "{}", t.name);
        assert!(t.capability.contains('.'), "{}: {}", t.name, t.capability);
    }
}

#[test]
fn a_tool_outside_its_area_runs_nothing() {
    let mut r = rig();
    r.opts.areas = areas_of(&["dictations"], false);
    let err = r.call("meetings_list", "{}").unwrap_err();
    assert!(err.contains("Besprechungen") && err.contains("nicht freigegeben"), "{err}");
    let err = r.call("calendar_events", "{}").unwrap_err();
    assert!(err.contains("Termine"), "{err}");
    assert!(r.call("dictations_recent", "{}").is_ok());
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0);
    let rows = r.audit();
    assert!(rows
        .iter()
        .all(|a| a.target.as_deref() == Some("dictations_recent · voice_orchestrator")));
}

#[test]
fn the_agent_rights_do_not_decide_for_the_call() {
    let r = rig();
    // Rechte der Agentenbrücke „aus“ und „fragen“ ändern am Gespräch nichts.
    r.set_right(Capability::MeetingRead, GrantMode::Off);
    r.set_right(Capability::TtsRead, GrantMode::Ask);
    assert!(r.list().iter().any(|n| n == "meetings_list"));
    assert!(r.call("meetings_list", "{}").is_ok());
    assert!(r.call("tts_providers_list", "{}").is_ok());
    // Auch eine ausgeschaltete Agent-Integration sperrt das Gespräch nicht.
    let id = r.integration_id();
    let patch = IntegrationPatch {
        enabled: Some(false),
        ..IntegrationPatch::default()
    };
    store::update(&r.fx.conn(), &id, &patch, NOW).unwrap();
    assert!(!r.list().is_empty());
    assert!(r.call("meetings_list", "{}").is_ok());
    assert!(approvals::list_pending(&r.fx.conn(), NOW).unwrap().is_empty());
}

#[test]
fn the_transcript_needs_the_voice_release_in_list_and_call() {
    for (transcripts, listed) in [(false, false), (true, true)] {
        let mut r = rig();
        r.opts = VoiceOptions {
            areas: areas_of(&["meetings"], transcripts),
        };
        assert_eq!(
            r.list().iter().any(|n| n == "meeting_transcript"),
            listed,
            "Liste bei {transcripts}"
        );
        // Die anderen lesenden Werkzeuge bleiben in jedem Fall.
        assert!(r.list().iter().any(|n| n == "meetings_list"));
        let result = r.call("meeting_transcript", r#"{"meeting_id":"x"}"#);
        assert_eq!(result.is_ok(), listed, "Aufruf bei {transcripts}");
        if !listed {
            assert!(result.unwrap_err().contains("Transkripte"));
            assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0);
        }
    }
}

#[test]
fn the_rate_limit_allows_eight_calls_a_minute() {
    let mut l = RateLimiter::default();
    for i in 0..RateLimiter::MAX_CALLS {
        assert!(l.allow(NOW + i as i64), "Aufruf {i}");
    }
    assert!(!l.allow(NOW + 1_000), "der neunte in derselben Minute");
    assert!(!l.allow(NOW + 59_000));
    assert!(l.allow(NOW + 60_000), "nach einer Minute wieder frei");
    // Eine rueckwaerts springende Uhr sperrt nicht dauerhaft.
    assert!(l.allow(NOW - 10_000_000));
}

#[test]
fn arguments_must_be_a_small_object() {
    let r = rig();
    for args in ["[]", "\"x\"", "42", "nicht json", "{\"a\":"] {
        assert!(r.call("meetings_list", args).is_err(), "{args}");
        assert!(r.call("dictations_recent", args).is_err(), "{args}");
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
fn long_results_are_refused_not_cut() {
    assert_eq!(clamp_result("kurz".to_string(), 10).unwrap(), "kurz");
    assert_eq!(clamp_result("ääää".to_string(), 4).unwrap(), "ääää");
    let err = clamp_result("äöüß!".to_string(), 4).unwrap_err();
    assert!(err.contains("kleinere Seite"), "{err}");
    let r = rig_with(json!({"text": "ü".repeat(MAX_RESULT_CHARS * 2)}));
    let err = r.call("meetings_list", "{}").unwrap_err();
    assert!(err.contains("zu lang"), "{err}");
    assert!(!err.contains("üüüü"), "keine Teilausgabe");
    // Knapp darunter geht durch, unverkürzt.
    let fits = "ü".repeat(MAX_RESULT_CHARS - 20);
    let r = rig_with(json!({ "text": fits }));
    assert!(r.call("meetings_list", "{}").unwrap().contains(&fits));
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
fn knowledge_calls_go_through_the_gate_with_audit_and_without_content() {
    let r = rig();
    let out = r.call("dictations_recent", r#"{"limit":3}"#).unwrap();
    assert!(out.contains("GEHEIMES-DIKTAT"), "das Ergebnis geht an das Modell");
    r.call("calendar_events", "{}").unwrap();
    let rows = r.audit();
    let capability_of = |target: &str| {
        rows.iter()
            .find(|a| a.target.as_deref() == Some(target))
            .and_then(|a| a.capability.clone())
    };
    assert_eq!(
        capability_of("dictations_recent · voice_orchestrator").as_deref(),
        Some("knowledge.read")
    );
    assert_eq!(
        capability_of("calendar_events · voice_orchestrator").as_deref(),
        Some("calendar.read")
    );
    assert!(rows.iter().any(|a| a.outcome == "ok"));
    for a in &rows {
        assert_eq!(a.caller, "agent_local");
        let all = format!("{:?}", a);
        assert!(!all.contains("GEHEIM") && !all.contains("limit"), "{all}");
    }
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0, "kein Katalog-Handler beteiligt");
}

#[test]
fn the_running_conversation_is_hidden_from_the_conversation_tools() {
    let mut r = rig();
    let out = r.call("conversations_list", "{}").unwrap();
    assert!(out.contains("conv_alt") && !out.contains("conv_laufend"), "{out}");
    r.conversation = None;
    assert!(r.call("conversations_list", "{}").unwrap().contains("conv_laufend"));
}

#[test]
fn a_failing_audit_blocks_the_call() {
    let r = rig();
    r.fx.conn()
        .execute_batch(
            "CREATE TRIGGER audit_full BEFORE INSERT ON audit_log
             BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
        )
        .unwrap();
    for name in ["meetings_list", "dictations_recent"] {
        let err = r.call(name, "{}").unwrap_err();
        assert!(err.contains("Datenbank"), "{name}: {err}");
    }
    assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0, "ohne Audit keine Aktion");
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
    let session = Session {
        registry: &registry,
        source: &Source,
        conversation: None,
    };
    let opts = VoiceOptions { areas: all_areas() };
    let err = call(&conn, &session, opts, "meetings_list", "{}", NOW).unwrap_err();
    assert!(err.contains("gesperrt"));
    let rows = audit::list(&conn, &AuditFilter::default(), 10).unwrap();
    assert!(rows.iter().any(|a| a.outcome == "error"));
}
