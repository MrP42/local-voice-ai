//! Tests der lesenden Besprechungs-Werkzeuge: echte Datenbank, nur synthetische Besprechungen.

use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::agent_bridge::catalog::{self, CallContext, ToolHandler};
use crate::agent_bridge::testkit::NOW;
use crate::agent_bridge::tools::{Host, PageRef, QueuedImport, Rendered};
use crate::managers::integrations::model::{Access, Capability};
use crate::managers::meetings::search::index::tests::{ready_meeting, tmp_store};
use crate::managers::meetings::store::{MeetingStore, StoredSegment, TranscriptDelta};
use crate::managers::workflows::recording::RecordingControl;
use crate::managers::youtube::source::AddOptions;

/// Ein Host, der nur die Freigabe des Transkripts kennt.
struct MeetHost {
    transcript: bool,
}

impl Host for MeetHost {
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

    fn transcript_shared(&self) -> bool {
        self.transcript
    }
}

fn segment(index: u32, start_ms: u64, text: &str) -> StoredSegment {
    StoredSegment {
        segment_index: index,
        text: text.to_string(),
        start_ms,
        end_ms: start_ms + 1_000,
        channel: 0,
        speaker_index: None,
        words: None,
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    tools: AppTools,
    meeting: String,
}

fn rig(transcript: bool) -> Rig {
    let (dir, store) = tmp_store();
    let meeting = ready_meeting(&store, "Jour fixe Vertrieb", 1_790_000_000).id;
    store
        .append_delta(
            &meeting,
            &TranscriptDelta {
                new_segments: vec![
                    segment(0, 0, "Wir starten mit dem Quartalsbericht."),
                    segment(1, 5_000, "Der Umsatz liegt bei einer Million."),
                ],
            },
        )
        .unwrap();
    let store: Arc<MeetingStore> = Arc::new(store);
    Rig {
        _dir: dir,
        tools: AppTools::new(store, Arc::new(MeetHost { transcript })),
        meeting,
    }
}

fn ctx() -> CallContext {
    CallContext {
        client_id: "C1".to_string(),
        client_label: "Sprachgespraech".to_string(),
        approved: false,
        now_ms: NOW,
    }
}

impl Rig {
    fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        self.tools.call(&ctx(), tool, &args)
    }
}

#[test]
fn the_meeting_tools_are_read_only_and_in_the_catalog() {
    let names = ["meetings_list", "meetings_search", "meeting_transcript"];
    let offered: Vec<String> = specs().into_iter().map(|s| s.name).collect();
    for name in names {
        let entry = catalog::find(name).expect("im Katalog");
        assert_eq!(entry.capability, Capability::MeetingRead, "{name}");
        assert_eq!(entry.capability.access(), Access::Read, "{name}");
        assert!(offered.iter().any(|n| n == name), "{name}");
    }
    assert_eq!(offered.len(), names.len());
    // Kein Name gehoert dem lokalen MCP-Server (sonst leitet der Proxy dessen Aufruf um).
    for s in specs() {
        assert!(!mcp_tools::is_known(&s.name), "{}", s.name);
        assert_eq!(s.input_schema["additionalProperties"], json!(false));
    }
}

#[test]
fn a_meeting_is_listed_and_searched() {
    let r = rig(true);
    let list = r.call("meetings_list", json!({})).unwrap();
    assert_eq!(list["returned"], json!(1));
    assert_eq!(list["meetings"][0]["title"], json!("Jour fixe Vertrieb"));
    let list = r
        .call("meetings_list", json!({"from": "2030-01-01"}))
        .unwrap();
    assert_eq!(list["returned"], json!(0));
    let found = r
        .call("meetings_search", json!({"query": "Jour fixe"}))
        .unwrap();
    // Ob ein Treffer kommt, haengt vom Such-Index ab (den baut die App im Hintergrund); die Form gilt immer.
    assert!(found["meetings"].is_array(), "{found}");
}

#[test]
fn arguments_are_checked_strictly() {
    let r = rig(true);
    for (tool, args) in [
        ("meetings_list", json!({"person": "Anna"})),
        ("meetings_list", json!({"folder": "x"})),
        ("meetings_list", json!({"limit": 0})),
        ("meetings_list", json!({"limit": 51})),
        ("meetings_list", json!({"limit": "zehn"})),
        ("meetings_list", json!({"from": 5})),
        ("meetings_search", json!({})),
        ("meetings_search", json!({"query": "   "})),
        ("meetings_search", json!({"query": "a".repeat(501)})),
        ("meetings_search", json!({"query": "x", "limit": 21})),
        ("meeting_transcript", json!({})),
        ("meeting_transcript", json!({"meeting_id": "../../etc"})),
        ("meeting_transcript", json!({"meeting_id": "a b"})),
        ("meeting_transcript", json!({"meeting_id": 7})),
        (
            "meeting_transcript",
            json!({"meeting_id": "x", "cursor": -1}),
        ),
        (
            "meeting_transcript",
            json!({"meeting_id": "x", "path": "C:\\a"}),
        ),
    ] {
        assert!(r.call(tool, args.clone()).is_err(), "{tool} {args}");
    }
    assert!(r.call("meetings_list", json!("kein Objekt")).is_err());
}

#[test]
fn the_transcript_is_read_page_wise_with_the_release() {
    let r = rig(true);
    let page = r
        .call("meeting_transcript", json!({"meeting_id": r.meeting}))
        .unwrap();
    let text = page["text"].as_str().unwrap();
    assert!(text.contains("Quartalsbericht"), "{page}");
    assert!(page["next_cursor"].is_null());
    let unknown = r.call("meeting_transcript", json!({"meeting_id": "gibt-es-nicht"}));
    assert!(unknown.unwrap_err().contains("nicht gefunden"));
}

#[test]
fn the_transcript_needs_the_users_release() {
    let r = rig(false);
    let err = r
        .call("meeting_transcript", json!({"meeting_id": r.meeting}))
        .unwrap_err();
    assert!(err.contains("nicht freigegeben"), "{err}");
    // Die Suche zeigt dann keinen Treffer, der nur im Transkript steht (die Filterung selbst
    // sichern die Tests des MCP-Servers mit gebautem Index).
    let found = r
        .call("meetings_search", json!({"query": "Quartalsbericht"}))
        .unwrap();
    assert_eq!(found["returned"], json!(0), "{found}");
    // Titel und Liste bleiben lesbar.
    assert_eq!(
        r.call("meetings_list", json!({})).unwrap()["returned"],
        json!(1)
    );
}
