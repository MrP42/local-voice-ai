use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::managers::integrations::model::Kind;

struct Fake {
    names: Vec<&'static str>,
}

impl ToolHandler for Fake {
    fn specs(&self) -> Vec<ToolSpec> {
        self.names
            .iter()
            .map(|n| ToolSpec {
                name: n.to_string(),
                title: n.to_string(),
                description: "test".to_string(),
                input_schema: json!({"type": "object"}),
            })
            .collect()
    }

    fn call(&self, _ctx: &CallContext, _tool: &str, _args: &Value) -> Result<Value, String> {
        Ok(json!({}))
    }
}

#[test]
fn every_catalog_capability_is_offered_by_the_agent_integration() {
    for e in CATALOG {
        assert!(
            Kind::Agent.capabilities().contains(&e.capability),
            "{} braucht eine Faehigkeit, die die Art agent nicht kennt",
            e.name
        );
        assert!(valid_tool_name(e.name), "{}", e.name);
    }
}

#[test]
fn catalog_names_are_unique_and_the_status_tool_is_not_in_it() {
    let mut names: Vec<&str> = CATALOG.iter().map(|e| e.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), CATALOG.len());
    assert!(find(STATUS_TOOL).is_none(), "das eingebaute Werkzeug hat kein Recht");
}

#[test]
fn recording_stays_behind_the_recording_right() {
    assert_eq!(find("start_recording").unwrap().capability, Capability::RecordingStart);
    assert_eq!(find("stop_recording").unwrap().capability, Capability::RecordingStart);
    assert!(Capability::RecordingStart.never_allow());
}

#[test]
fn a_registry_takes_catalog_tools_and_knows_them() {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(Fake {
        names: vec!["transcribe_file", "create_meeting"],
    }))
    .unwrap();
    assert!(r.has("transcribe_file"));
    assert!(r.has("create_meeting"));
    assert!(!r.has("start_recording"));
    assert_eq!(r.names(), vec!["create_meeting", "transcribe_file"]);
    assert!(r.handler("transcribe_file").is_some());
}

#[test]
fn a_tool_outside_the_catalog_is_refused_and_nothing_of_its_batch_is_taken() {
    let mut r = ToolRegistry::new();
    let err = r
        .register(Arc::new(Fake {
            names: vec!["transcribe_file", "delete_everything"],
        }))
        .unwrap_err();
    assert_eq!(err, RegisterError::NotInCatalog("delete_everything".to_string()));
    assert!(!r.has("transcribe_file"), "ein Werkzeug der Gruppe scheiterte: keines wird aufgenommen");
}

#[test]
fn the_status_tool_cannot_be_registered_as_a_gated_tool() {
    let mut r = ToolRegistry::new();
    let err = r
        .register(Arc::new(Fake {
            names: vec![STATUS_TOOL],
        }))
        .unwrap_err();
    assert!(matches!(err, RegisterError::NotInCatalog(_)));
}

#[test]
fn duplicates_are_refused_across_and_within_handlers() {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(Fake {
        names: vec!["create_meeting"],
    }))
    .unwrap();
    assert_eq!(
        r.register(Arc::new(Fake {
            names: vec!["create_meeting"],
        }))
        .unwrap_err(),
        RegisterError::Duplicate("create_meeting".to_string())
    );
    assert!(matches!(
        r.register(Arc::new(Fake {
            names: vec!["tts_render_audio", "tts_render_audio"],
        }))
        .unwrap_err(),
        RegisterError::Duplicate(_)
    ));
}

#[test]
fn tool_names_are_checked_for_shape() {
    for ok in ["a", "start_recording", "x1_y2"] {
        assert!(valid_tool_name(ok), "{ok}");
    }
    for bad in ["", "Start", "a b", "a-b", "a/b", &"x".repeat(65)] {
        assert!(!valid_tool_name(bad), "{bad}");
    }
}

#[test]
fn writing_tools_are_not_marked_read_only() {
    let a = annotations(find("transcribe_file").unwrap());
    assert_eq!(a["readOnlyHint"], json!(false));
}
