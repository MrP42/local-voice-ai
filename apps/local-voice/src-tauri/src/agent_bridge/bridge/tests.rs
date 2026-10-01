//! Tests der Entscheidungen der Agentenbruecke: Anmeldung, Liste, Aufruf, Freigabe, Audit.

use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::agent_bridge::catalog::STATUS_TOOL;
use crate::agent_bridge::protocol::code;
use crate::agent_bridge::testkit::{fast_config, Behavior, Fixture, ALL_TOOLS, NOW};
use crate::managers::integrations::approvals;

fn names(list: &[Value]) -> Vec<String> {
    list.iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

fn wait_id(step: CallStep) -> String {
    match step {
        CallStep::Wait(id) => id,
        other => panic!("Freigabe erwartet, bekommen: {other:?}"),
    }
}

fn err(r: Result<CallStep, BridgeError>) -> BridgeError {
    r.expect_err("Fehler erwartet")
}

// --- Anmelden ---------------------------------------------------------------

#[test]
fn a_valid_token_logs_in_and_marks_the_use() {
    let f = Fixture::new();
    let ctx = f.bridge.authenticate(&f.token).unwrap();
    assert_eq!(ctx, f.ctx());
    let row = clients::get(&f.conn(), &f.client.id).unwrap().unwrap();
    assert_eq!(row.last_used_at, Some(NOW));
}

#[test]
fn an_invalid_token_is_refused_and_audited() {
    let f = Fixture::new();
    let e = f.bridge.authenticate(&clients::generate_token()).unwrap_err();
    assert_eq!(e.code, code::TOKEN_INVALID);
    let rows = f.audit();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].caller, "agent_external");
    assert_eq!(rows[0].outcome, "denied");
    assert!(rows[0].detail_json.as_deref().unwrap().contains("token_invalid"));
    // Weder das erfundene Token noch ein Teil davon steht im Audit.
    assert!(rows[0].target.is_none());
}

#[test]
fn a_garbage_token_is_refused_without_touching_the_clients() {
    let f = Fixture::new();
    let e = f.bridge.authenticate("kein token").unwrap_err();
    assert_eq!(e.code, code::TOKEN_INVALID);
    let e = f.bridge.authenticate("").unwrap_err();
    assert_eq!(e.code, code::TOKEN_INVALID);
}

#[test]
fn a_revoked_token_is_refused_by_name_and_audited() {
    let f = Fixture::new();
    clients::revoke(&f.conn(), &f.client.id, NOW + 1).unwrap();
    let e = f.bridge.authenticate(&f.token).unwrap_err();
    assert_eq!(e.code, code::TOKEN_REVOKED);
    let rows = f.audit();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "denied");
    assert!(rows[0].detail_json.as_deref().unwrap().contains("token_revoked"));
    assert!(
        rows[0].target.as_deref().unwrap().contains(&f.client.id),
        "das Audit nennt den Zugang"
    );
    assert_eq!(rows[0].integration_id.as_deref(), Some("agents"));
}

#[test]
fn failed_logins_are_limited_and_the_limit_is_audited_once() {
    let cfg = crate::agent_bridge::Config {
        auth_failures_per_minute: 3,
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    for _ in 0..3 {
        assert_eq!(
            f.bridge.authenticate(&clients::generate_token()).unwrap_err().code,
            code::TOKEN_INVALID
        );
    }
    for _ in 0..10 {
        assert_eq!(
            f.bridge.authenticate(&clients::generate_token()).unwrap_err().code,
            code::RATE_LIMITED
        );
    }
    // Auch das richtige Token kommt in dieser Minute nicht mehr durch? Doch: nur Fehlversuche
    // zaehlen, ein gueltiges Token wird nicht ausgesperrt (sonst waere der Nutzer ausgesperrt).
    assert!(f.bridge.authenticate(&f.token).is_ok());
    let rows = f.audit();
    let limited = rows
        .iter()
        .filter(|r| r.detail_json.as_deref().is_some_and(|d| d.contains("auth_rate_limited")))
        .count();
    assert_eq!(limited, 1, "die Flut steht als EINE Zeile im Audit");
}

#[test]
fn refresh_cuts_off_a_client_revoked_or_removed_mid_connection() {
    let f = Fixture::new();
    assert!(f.bridge.refresh(&f.client.id).is_ok());
    clients::revoke(&f.conn(), &f.client.id, NOW + 1).unwrap();
    assert_eq!(f.bridge.refresh(&f.client.id).unwrap_err().code, code::TOKEN_REVOKED);
    clients::delete(&f.conn(), &f.client.id).unwrap();
    assert_eq!(f.bridge.refresh(&f.client.id).unwrap_err().code, code::TOKEN_INVALID);
}

// --- Status -------------------------------------------------------------------

#[test]
fn the_anonymous_status_shows_only_the_basics() {
    let f = Fixture::new();
    let v = f.bridge.anonymous_status();
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["authenticated"], json!(false));
    assert_eq!(v["app"], json!("local-voice-ai"));
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    for secret in ["client", "tools", "pending_approvals"] {
        assert!(!keys.contains(&secret), "{secret} gehoert nicht in den anonymen Stand");
    }
}

#[test]
fn the_authenticated_status_counts_own_pending_approvals_and_visible_tools() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let ctx = f.ctx();
    wait_id(f.bridge.call(&ctx, "transcribe_file", &json!({"path": "C:/a.wav"}), None).unwrap());
    let (other, _) = f.second_client("Codex");
    f.grant_for(&other.id, "transcribe_file", GrantMode::Ask);
    wait_id(f.bridge.call(&other, "transcribe_file", &json!({"path": "C:/b.wav"}), None).unwrap());
    let s = f.bridge.status(&ctx).unwrap();
    assert_eq!(s["authenticated"], json!(true));
    assert_eq!(s["client"]["label"], json!("Claude Code"));
    assert_eq!(s["pending_approvals"], json!(1), "nur die eigenen");
    assert_eq!(s["tools"], json!(2), "get_action_status + transcribe_file");
}

// --- Werkzeugliste ------------------------------------------------------------

#[test]
fn a_new_client_sees_only_the_status_tool() {
    let f = Fixture::new();
    let list = f.bridge.tools_list(&f.ctx()).unwrap();
    assert_eq!(names(&list), vec![STATUS_TOOL.to_string()]);
}

#[test]
fn a_tool_set_to_off_is_missing_and_one_set_to_ask_or_allow_is_listed_with_its_mode() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    f.grant("create_meeting", GrantMode::Allow);
    f.grant("tts_page_create", GrantMode::Off);
    let list = f.bridge.tools_list(&f.ctx()).unwrap();
    let n = names(&list);
    assert!(n.contains(&"transcribe_file".to_string()));
    assert!(n.contains(&"create_meeting".to_string()));
    assert!(!n.contains(&"tts_page_create".to_string()), "aus: fehlt in tools/list");
    assert!(!n.contains(&"start_recording".to_string()), "nie freigegeben: fehlt");
    let mode = |t: &str| list.iter().find(|x| x["name"] == t).unwrap()["mode"].clone();
    assert_eq!(mode("transcribe_file"), json!("ask"));
    assert_eq!(mode("create_meeting"), json!("allow"));
}

#[test]
fn the_integration_ceiling_hides_a_tool_even_when_the_client_allows_it() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Allow);
    f.ceiling("transcribe_file", GrantMode::Off);
    let list = f.bridge.tools_list(&f.ctx()).unwrap();
    assert!(!names(&list).contains(&"transcribe_file".to_string()));
}

#[test]
fn the_client_right_hides_a_tool_even_when_the_ceiling_allows_it() {
    let f = Fixture::new();
    f.ceiling("transcribe_file", GrantMode::Allow);
    // Kein Recht des Zugangs: aus.
    assert!(!names(&f.bridge.tools_list(&f.ctx()).unwrap()).contains(&"transcribe_file".to_string()));
    // Ein ANDERER Zugang mit Recht aendert daran nichts.
    let (other, _) = f.second_client("Codex");
    f.grant_for(&other.id, "transcribe_file", GrantMode::Allow);
    assert!(!names(&f.bridge.tools_list(&f.ctx()).unwrap()).contains(&"transcribe_file".to_string()));
    assert!(names(&f.bridge.tools_list(&other).unwrap()).contains(&"transcribe_file".to_string()));
}

#[test]
fn a_tool_without_a_handler_is_not_listed_even_when_allowed() {
    let f = Fixture::with(fast_config(), &["create_meeting"]);
    f.grant("transcribe_file", GrantMode::Allow);
    f.grant("create_meeting", GrantMode::Allow);
    let n = names(&f.bridge.tools_list(&f.ctx()).unwrap());
    assert!(n.contains(&"create_meeting".to_string()));
    assert!(!n.contains(&"transcribe_file".to_string()));
}

#[test]
fn switching_the_agent_integration_off_empties_the_list() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    let conn = f.conn();
    store::update(
        &conn,
        &f.client.integration_id,
        &crate::managers::integrations::model::IntegrationPatch {
            enabled: Some(false),
            ..Default::default()
        },
        NOW,
    )
    .unwrap();
    assert_eq!(names(&f.bridge.tools_list(&f.ctx()).unwrap()), vec![STATUS_TOOL.to_string()]);
    assert_eq!(
        err(f.bridge.call(&f.ctx(), "create_meeting", &json!({}), None)).code,
        code::TOOL_OFF
    );
}

// --- Aufruf: aus / erlaubt ----------------------------------------------------

#[test]
fn a_call_to_a_tool_that_is_off_is_refused_audited_and_never_runs() {
    let f = Fixture::new();
    let e = err(f.bridge.call(&f.ctx(), "transcribe_file", &json!({"path": "C:/a.wav"}), None));
    assert_eq!(e.code, code::TOOL_OFF);
    assert!(e.data.as_ref().unwrap()["reason"].is_string());
    assert_eq!(f.calls.len(), 0);
    let rows = f.audit();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "denied");
    assert_eq!(rows[0].capability.as_deref(), Some("transcribe.file"));
    assert_eq!(rows[0].caller, "agent_external");
    assert_eq!(f.count("SELECT COUNT(*) FROM approvals"), 0, "aus: keine Freigabe");
}

#[test]
fn a_call_is_refused_when_only_the_clients_own_right_is_off() {
    let f = Fixture::new();
    // Die Obergrenze der Integration erlaubt es, dieser Zugang hat aber kein Recht.
    f.ceiling("create_meeting", GrantMode::Allow);
    let e = err(f.bridge.call(&f.ctx(), "create_meeting", &json!({}), None));
    assert_eq!(e.code, code::TOOL_OFF);
    assert_eq!(e.data.as_ref().unwrap()["reason"], json!("tool_off"));
    assert_eq!(f.calls.len(), 0);
    // Dasselbe mit ausdruecklich „aus“ gesetztem Recht des Zugangs.
    f.grant("create_meeting", GrantMode::Off);
    assert_eq!(err(f.bridge.call(&f.ctx(), "create_meeting", &json!({}), None)).code, code::TOOL_OFF);
    assert_eq!(f.calls.len(), 0);
}

#[test]
fn an_allowed_call_runs_once_and_is_audited_from_start_to_end() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    let step = f.bridge.call(&f.ctx(), "create_meeting", &json!({"title": "Test"}), None).unwrap();
    assert_eq!(step, CallStep::Done(json!({"ran": "create_meeting", "client": "Claude Code"})));
    assert_eq!(f.calls.all(), vec![("create_meeting".to_string(), json!({"title": "Test"}), false)]);
    let rows = f.audit();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "ok");
    assert_eq!(rows[0].capability.as_deref(), Some("meeting.create"));
    let target = rows[0].target.as_deref().unwrap();
    assert!(target.starts_with("create_meeting · Claude Code ("), "{target}");
    assert!(target.contains(&f.client.id));
}

#[test]
fn a_tool_error_is_reported_and_audited_as_error() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    *f.behavior.lock().unwrap() = Behavior::Err("Datei nicht gefunden".into());
    let e = err(f.bridge.call(&f.ctx(), "create_meeting", &json!({}), None));
    assert_eq!(e.code, code::FAILED);
    assert!(e.message.contains("Datei nicht gefunden"));
    assert_eq!(f.audit()[0].outcome, "error");
}

#[test]
fn a_panicking_tool_is_contained_and_the_bridge_keeps_working() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    *f.behavior.lock().unwrap() = Behavior::Panic;
    let e = err(f.bridge.call(&f.ctx(), "create_meeting", &json!({}), None));
    assert_eq!(e.code, code::FAILED);
    assert!(!e.message.contains("absichtlich"), "keine internen Texte nach aussen");
    assert_eq!(f.audit()[0].outcome, "error");
    *f.behavior.lock().unwrap() = Behavior::Ok;
    assert!(matches!(
        f.bridge.call(&f.ctx(), "create_meeting", &json!({}), None).unwrap(),
        CallStep::Done(_)
    ));
}

#[test]
fn an_oversized_answer_is_replaced_but_the_call_still_counts_as_done() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    *f.behavior.lock().unwrap() = Behavior::Huge;
    let CallStep::Done(v) = f.bridge.call(&f.ctx(), "create_meeting", &json!({}), None).unwrap() else {
        panic!("done erwartet");
    };
    assert_eq!(v["truncated"], json!(true));
    assert!(v.to_string().len() < 1000);
    assert_eq!(f.audit()[0].outcome, "ok");
}

#[test]
fn an_unknown_tool_is_refused_and_audited() {
    let f = Fixture::new();
    let e = err(f.bridge.call(&f.ctx(), "format_disk", &json!({}), None));
    assert_eq!(e.code, code::UNKNOWN_TOOL);
    let rows = f.audit();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "denied");
    assert!(rows[0].detail_json.as_deref().unwrap().contains("unknown_tool"));
    assert_eq!(f.calls.len(), 0);
}

#[test]
fn an_allowed_tool_without_a_handler_says_unavailable_and_is_audited() {
    let f = Fixture::with(fast_config(), &["create_meeting"]);
    f.grant("transcribe_file", GrantMode::Allow);
    let e = err(f.bridge.call(&f.ctx(), "transcribe_file", &json!({}), None));
    assert_eq!(e.code, code::TOOL_UNAVAILABLE);
    assert!(f.audit().iter().any(|r| r.detail_json.as_deref().is_some_and(|d| d.contains("tool_unavailable"))));
}

#[test]
fn arguments_must_be_an_object() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    for bad in [json!("text"), json!([1]), json!(5), json!(true)] {
        let e = err(f.bridge.call(&f.ctx(), "create_meeting", &bad, None));
        assert_eq!(e.code, code::BAD_REQUEST, "{bad}");
    }
    assert_eq!(f.calls.len(), 0);
    // null ist „keine Argumente“.
    assert!(matches!(
        f.bridge.call(&f.ctx(), "create_meeting", &Value::Null, None).unwrap(),
        CallStep::Done(_)
    ));
    assert_eq!(f.calls.all()[0].1, json!({}));
}

// --- Aufruf: fragen -------------------------------------------------------------

#[test]
fn ask_creates_an_approval_with_a_readable_preview_and_does_not_run() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let id = wait_id(
        f.bridge
            .call(&f.ctx(), "transcribe_file", &json!({"path": "C:/Aufnahmen/a.wav", "language": "de"}), None)
            .unwrap(),
    );
    assert_eq!(f.calls.len(), 0, "fragen: noch nichts ausgefuehrt");
    let a = approvals::get(&f.conn(), &id).unwrap().unwrap();
    assert_eq!(a.caller, "agent_external");
    assert_eq!(a.integration_id.as_deref(), Some("agents"));
    assert_eq!(a.tool_or_capability, "transcribe.file");
    assert_eq!(a.state, ApprovalState::Pending);
    let preview = a.args_preview.unwrap();
    assert!(preview.starts_with("Ziel: transcribe_file · Claude Code ("), "{preview}");
    assert!(preview.contains("C:/Aufnahmen/a.wav"), "der Pfad steht vollstaendig da: {preview}");
    // Audit: Freigabe angefragt (pending).
    let rows = f.audit();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "pending");
    assert!(rows[0].detail_json.as_deref().unwrap().contains(&id));
}

#[test]
fn the_same_ask_twice_gives_the_same_approval() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let a = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    let b = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    assert_eq!(a, b);
    assert_eq!(f.count("SELECT COUNT(*) FROM approvals"), 1);
    assert_eq!(f.audit().len(), 1, "keine zweite Audit-Zeile");
}

#[test]
fn concurrent_identical_asks_share_one_approval() {
    let f = Arc::new(Fixture::new());
    f.grant("transcribe_file", GrantMode::Ask);
    let threads: Vec<_> = (0..6)
        .map(|_| {
            let f = f.clone();
            std::thread::spawn(move || {
                wait_id(
                    f.bridge
                        .call(&f.ctx(), "transcribe_file", &json!({"path": "C:/a.wav"}), None)
                        .unwrap(),
                )
            })
        })
        .collect();
    let ids: Vec<String> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(ids.iter().all(|i| i == &ids[0]), "{ids:?}");
    assert_eq!(f.count("SELECT COUNT(*) FROM approvals"), 1);
}

#[test]
fn two_clients_asking_the_same_thing_get_separate_approvals() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let (other, _) = f.second_client("Codex");
    f.grant_for(&other.id, "transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let a = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    let b = wait_id(f.bridge.call(&other, "transcribe_file", &args, None).unwrap());
    assert_ne!(a, b);
    let pb = approvals::get(&f.conn(), &b).unwrap().unwrap().args_preview.unwrap();
    assert!(pb.contains("Codex"), "die Freigabe nennt den Zugang: {pb}");
}

#[test]
fn an_approved_approval_runs_the_tool_exactly_once() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    assert_eq!(f.bridge.approval_phase(&f.ctx(), &id).unwrap().phase, Phase::Pending);
    f.user_decides(&id, true);
    assert_eq!(f.bridge.approval_phase(&f.ctx(), &id).unwrap().phase, Phase::Approved);
    let step = f.bridge.call(&f.ctx(), "transcribe_file", &args, Some(&id)).unwrap();
    assert!(matches!(step, CallStep::Done(_)));
    assert_eq!(f.calls.all(), vec![("transcribe_file".to_string(), args.clone(), true)]);
    assert_eq!(f.bridge.approval_phase(&f.ctx(), &id).unwrap().phase, Phase::Used);
    // Audit: angefragt (pending) und ausgefuehrt (ok).
    let outcomes: Vec<String> = f.audit().iter().map(|r| r.outcome.clone()).collect();
    assert_eq!(outcomes, vec!["pending", "ok"]);
}

#[test]
fn an_approval_cannot_be_used_twice() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    f.user_decides(&id, true);
    f.bridge.call(&f.ctx(), "transcribe_file", &args, Some(&id)).unwrap();
    let e = err(f.bridge.call(&f.ctx(), "transcribe_file", &args, Some(&id)));
    assert_eq!(e.code, code::APPROVAL_USED);
    assert_eq!(f.calls.len(), 1);
}

#[test]
fn an_approval_used_by_two_threads_runs_once() {
    let f = Arc::new(Fixture::new());
    f.grant("transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    f.user_decides(&id, true);
    let threads: Vec<_> = (0..5)
        .map(|_| {
            let (f, id, args) = (f.clone(), id.clone(), args.clone());
            std::thread::spawn(move || f.bridge.call_approved(&f.ctx(), "transcribe_file", &args, &id))
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "{results:?}");
    assert_eq!(f.calls.len(), 1);
}

#[test]
fn a_denied_approval_never_runs() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    f.user_decides(&id, false);
    assert_eq!(f.bridge.approval_phase(&f.ctx(), &id).unwrap().phase, Phase::Denied);
    let e = err(f.bridge.call(&f.ctx(), "transcribe_file", &args, Some(&id)));
    assert_eq!(e.code, code::APPROVAL_DENIED);
    assert_eq!(f.calls.len(), 0);
}

#[test]
fn an_approval_expires_after_an_hour() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    f.advance(approvals::TTL_MS + 1);
    assert_eq!(f.bridge.approval_phase(&f.ctx(), &id).unwrap().phase, Phase::Expired);
    let e = err(f.bridge.call(&f.ctx(), "transcribe_file", &args, Some(&id)));
    assert_eq!(e.code, code::APPROVAL_EXPIRED);
    assert_eq!(f.calls.len(), 0);
}

#[test]
fn calling_again_with_a_pending_approval_waits_for_the_same_one() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    let again = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, Some(&id)).unwrap());
    assert_eq!(id, again);
}

#[test]
fn another_clients_approval_is_unusable_and_invisible() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let (other, _) = f.second_client("Codex");
    f.grant_for(&other.id, "transcribe_file", GrantMode::Allow);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    f.user_decides(&id, true);
    // Der andere Zugang sieht den Stand nicht und kann die Genehmigung nicht verbrauchen.
    assert_eq!(f.bridge.approval_phase(&other, &id).unwrap_err().code, code::APPROVAL_NOT_FOUND);
    assert_eq!(f.bridge.action_status(&other, &id).unwrap_err().code, code::APPROVAL_NOT_FOUND);
    assert_eq!(
        err(f.bridge.call(&other, "transcribe_file", &args, Some(&id))).code,
        code::APPROVAL_NOT_FOUND
    );
    assert_eq!(
        f.bridge.call_approved(&other, "transcribe_file", &args, &id).unwrap_err().code,
        code::APPROVAL_NOT_FOUND
    );
    assert_eq!(f.calls.len(), 0);
    // Der Besitzer kann sie danach noch benutzen.
    assert!(f.bridge.call(&f.ctx(), "transcribe_file", &args, Some(&id)).is_ok());
}

#[test]
fn an_approval_does_not_carry_over_to_other_arguments_or_another_tool() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    f.grant("create_meeting", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    f.user_decides(&id, true);
    // Andere Argumente: genehmigt war nur diese Datei.
    let e = err(f.bridge.call(&f.ctx(), "transcribe_file", &json!({"path": "C:/geheim.wav"}), Some(&id)));
    assert_eq!(e.code, code::APPROVAL_MISMATCH);
    // Anderes Werkzeug.
    let e = err(f.bridge.call(&f.ctx(), "create_meeting", &json!({}), Some(&id)));
    assert_eq!(e.code, code::APPROVAL_NOT_FOUND);
    assert_eq!(f.calls.len(), 0);
    // Die Genehmigung ist dabei nicht verbraucht worden.
    assert!(matches!(
        f.bridge.call(&f.ctx(), "transcribe_file", &args, Some(&id)).unwrap(),
        CallStep::Done(_)
    ));
}

#[test]
fn argument_order_does_not_change_the_approval() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let a: Value = serde_json::from_str(r#"{"path": "C:/a.wav", "language": "de", "nested": {"x": 1, "y": 2}}"#).unwrap();
    let b: Value = serde_json::from_str(r#"{"nested": {"y": 2, "x": 1}, "language": "de", "path": "C:/a.wav"}"#).unwrap();
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &a, None).unwrap());
    let same = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &b, None).unwrap());
    assert_eq!(id, same);
    f.user_decides(&id, true);
    assert!(f.bridge.call(&f.ctx(), "transcribe_file", &b, Some(&id)).is_ok());
}

#[test]
fn rights_withdrawn_after_approval_stop_the_run() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    f.user_decides(&id, true);
    f.grant("transcribe_file", GrantMode::Off);
    let e = err(f.bridge.call(&f.ctx(), "transcribe_file", &args, Some(&id)));
    assert_eq!(e.code, code::TOOL_OFF);
    assert_eq!(f.calls.len(), 0);
}

#[test]
fn a_revoked_client_cannot_use_its_approval() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    f.user_decides(&id, true);
    clients::revoke(&f.conn(), &f.client.id, f.now()).unwrap();
    let e = err(f.bridge.call(&f.ctx(), "transcribe_file", &args, Some(&id)));
    assert_eq!(e.code, code::TOKEN_REVOKED);
    assert_eq!(f.calls.len(), 0);
}

#[test]
fn recording_is_never_run_without_asking_even_when_the_database_says_allow() {
    let f = Fixture::new();
    // Der Weg ueber die Befehle verbietet `allow`; hier wird die Datenbank von Hand verfaelscht.
    let conn = f.conn();
    conn.execute(
        "INSERT INTO integration_grants (integration_id, capability, caller, mode)
         VALUES ('agents', 'recording.start', 'agent_external', 'allow')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agent_tool_grants (client_id, tool, mode) VALUES (?1, 'start_recording', 'allow')",
        rusqlite::params![f.client.id],
    )
    .unwrap();
    let step = f.bridge.call(&f.ctx(), "start_recording", &json!({}), None).unwrap();
    assert!(matches!(step, CallStep::Wait(_)), "Aufnahme: nie ohne Freigabe: {step:?}");
    assert_eq!(f.calls.len(), 0);
    let listed = f.bridge.tools_list(&f.ctx()).unwrap();
    assert_eq!(
        listed.iter().find(|t| t["name"] == "start_recording").unwrap()["mode"],
        json!("ask")
    );
}

#[test]
fn one_client_cannot_flood_the_open_approvals() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let mut open = 0;
    let mut refused = None;
    for i in 0..(approvals::MAX_PENDING_PER_SOURCE + 3) {
        match f.bridge.call(&f.ctx(), "transcribe_file", &json!({"path": format!("C:/{i}.wav")}), None) {
            Ok(CallStep::Wait(_)) => open += 1,
            Ok(other) => panic!("{other:?}"),
            Err(e) => {
                refused = Some(e);
                break;
            }
        }
    }
    assert_eq!(open, approvals::MAX_PENDING_PER_SOURCE);
    let e = refused.expect("die Grenze greift");
    assert_eq!(e.code, code::DENIED);
    assert_eq!(e.data.unwrap()["reason"], json!("too_many_pending_caller"));
}

#[test]
fn a_request_that_cannot_be_shown_in_full_is_refused_not_clipped() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let long = format!("C:/{}.wav", "x".repeat(400));
    let e = err(f.bridge.call(&f.ctx(), "transcribe_file", &json!({"path": long}), None));
    assert_eq!(e.code, code::DENIED);
    assert_eq!(e.data.unwrap()["reason"], json!("preview_unsafe"));
    assert_eq!(f.count("SELECT COUNT(*) FROM approvals"), 0);
}

// --- Fail closed ----------------------------------------------------------------------

#[test]
fn a_failing_audit_write_blocks_the_tool() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    f.conn()
        .execute_batch(
            "CREATE TRIGGER block_audit BEFORE INSERT ON audit_log BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
        )
        .unwrap();
    let e = err(f.bridge.call(&f.ctx(), "create_meeting", &json!({}), None));
    assert_eq!(e.code, code::STORE_UNAVAILABLE);
    assert_eq!(f.calls.len(), 0, "ohne Protokoll keine Aktion");
    assert!(!e.message.contains("disk full"), "keine Datenbanktexte nach aussen");
}

#[test]
fn a_failing_audit_write_leaves_no_open_approval_behind() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    f.conn()
        .execute_batch(
            "CREATE TRIGGER block_audit BEFORE INSERT ON audit_log BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
        )
        .unwrap();
    let e = err(f.bridge.call(&f.ctx(), "transcribe_file", &json!({"path": "C:/a.wav"}), None));
    assert_eq!(e.code, code::STORE_UNAVAILABLE);
    assert_eq!(
        f.count("SELECT COUNT(*) FROM approvals WHERE state = 'pending'"),
        0,
        "eine Freigabe ohne Audit-Zeile darf nicht offen bleiben"
    );
}

#[test]
fn a_briefly_locked_database_makes_the_call_wait_and_then_go_through() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    // Ein anderer Schreiber haelt die Datenbank kurz gesperrt. Die Bruecke oeffnet ihre Verbindungen
    // ueber `meetings::store::open_connection` (Wartezeit 30 s, G8) und wartet, statt zu scheitern.
    let blocker = f.conn();
    blocker.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        blocker.execute_batch("ROLLBACK;").unwrap();
    });
    let started = std::time::Instant::now();
    let step = f.bridge.call(&f.ctx(), "create_meeting", &json!({}), None).unwrap();
    release.join().unwrap();
    assert!(matches!(step, CallStep::Done(_)));
    assert!(started.elapsed() >= std::time::Duration::from_millis(400), "es wurde gewartet");
    assert_eq!(f.calls.len(), 1, "genau einmal ausgefuehrt");
}

// --- Grenzen --------------------------------------------------------------------------------

#[test]
fn calls_are_limited_per_client_and_the_refusal_is_audited_once() {
    let cfg = crate::agent_bridge::Config {
        calls_per_minute: 3,
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    let ctx = f.ctx();
    for _ in 0..3 {
        f.bridge.rate_check(&ctx, true).unwrap();
    }
    for _ in 0..20 {
        assert_eq!(f.bridge.rate_check(&ctx, true).unwrap_err().code, code::RATE_LIMITED);
    }
    let limited: Vec<_> = f
        .audit()
        .into_iter()
        .filter(|r| r.detail_json.as_deref().is_some_and(|d| d.contains("rate_limited")))
        .collect();
    assert_eq!(limited.len(), 1, "die Flut ist EINE Zeile");
    // Ein anderer Zugang ist nicht betroffen, ebenso Anfragen, die keine Aufrufe sind.
    let (other, _) = f.second_client("Codex");
    assert!(f.bridge.rate_check(&other, true).is_ok());
    assert!(f.bridge.rate_check(&ctx, false).is_ok());
}

#[test]
fn all_requests_are_limited_too() {
    let cfg = crate::agent_bridge::Config {
        requests_per_minute: 5,
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    for _ in 0..5 {
        f.bridge.rate_check(&f.ctx(), false).unwrap();
    }
    assert_eq!(f.bridge.rate_check(&f.ctx(), false).unwrap_err().code, code::RATE_LIMITED);
}

// --- Status einer Freigabe ----------------------------------------------------------------

#[test]
fn the_status_tool_answers_for_own_approvals_only() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let args = json!({"path": "C:/a.wav"});
    let id = wait_id(f.bridge.call(&f.ctx(), "transcribe_file", &args, None).unwrap());
    let CallStep::Done(v) = f.bridge.call(&f.ctx(), STATUS_TOOL, &json!({"approval_id": id}), None).unwrap() else {
        panic!()
    };
    assert_eq!(v["state"], json!("pending"));
    assert_eq!(v["tool"], json!("transcribe_file"));
    f.user_decides(&id, true);
    let v = f.bridge.action_status(&f.ctx(), &id).unwrap();
    assert_eq!(v["state"], json!("approved"));
    assert!(v["hint"].as_str().unwrap().contains("erneut aufrufen"));
    let (other, _) = f.second_client("Codex");
    assert_eq!(
        err(f.bridge.call(&other, STATUS_TOOL, &json!({"approval_id": id}), None)).code,
        code::APPROVAL_NOT_FOUND
    );
    assert_eq!(
        err(f.bridge.call(&f.ctx(), STATUS_TOOL, &json!({}), None)).code,
        code::BAD_REQUEST
    );
    assert_eq!(
        f.bridge.action_status(&f.ctx(), "gibt-es-nicht").unwrap_err().code,
        code::APPROVAL_NOT_FOUND
    );
}

#[test]
fn rejected_connections_are_audited_without_a_client() {
    let f = Fixture::new();
    f.bridge
        .audit_rejected_connection("peer_not_current_user", json!({"pid": 4242}));
    let rows = f.audit();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "denied");
    assert!(rows[0].detail_json.as_deref().unwrap().contains("peer_not_current_user"));
}

#[test]
fn the_agent_never_sees_secrets_in_audit_or_preview() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    let secret = "Bearer abcdefghijklmnop123456";
    let id = wait_id(
        f.bridge
            .call(
                &f.ctx(),
                "transcribe_file",
                &json!({"path": "C:/a.wav", "authorization": secret, "password": "geheim123"}),
                None,
            )
            .unwrap(),
    );
    let preview = approvals::get(&f.conn(), &id).unwrap().unwrap().args_preview.unwrap();
    assert!(!preview.contains("abcdefghijklmnop"), "{preview}");
    assert!(!preview.contains("geheim123"), "{preview}");
    for row in f.audit() {
        let text = format!("{:?}", row);
        assert!(!text.contains("geheim123") && !text.contains("abcdefghijklmnop"), "{text}");
    }
}
