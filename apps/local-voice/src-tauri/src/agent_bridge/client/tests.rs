use std::io::Cursor;

use serde_json::json;

use super::*;

/// Ein Client, dessen Antworten fest vorgegeben sind; `sent` zeigt, was er geschrieben hat.
fn canned(reply: &str) -> (Client<Cursor<Vec<u8>>, Vec<u8>>, ()) {
    (Client::new(Cursor::new(reply.as_bytes().to_vec()), Vec::new()), ())
}

#[test]
fn a_request_is_one_json_line_and_the_result_is_returned() {
    let (mut c, _) = canned("{\"id\":1,\"result\":{\"ok\":true}}\n");
    let r = c.request("status", json!({"a": 1})).unwrap();
    assert_eq!(r, json!({"ok": true}));
    let sent = String::from_utf8(c.writer.clone()).unwrap();
    assert!(sent.ends_with('\n'));
    assert_eq!(sent.matches('\n').count(), 1, "genau eine Zeile");
    let v: serde_json::Value = serde_json::from_str(sent.trim()).unwrap();
    assert_eq!(v["method"], json!("status"));
    assert_eq!(v["id"], json!(1));
    assert_eq!(v["params"], json!({"a": 1}));
}

#[test]
fn ids_count_up_per_request() {
    let (mut c, _) = canned("{\"id\":1,\"result\":1}\n{\"id\":2,\"result\":2}\n");
    assert_eq!(c.request("a", json!({})).unwrap(), json!(1));
    assert_eq!(c.request("b", json!({})).unwrap(), json!(2));
}

#[test]
fn a_remote_error_keeps_code_message_and_data() {
    let (mut c, _) = canned(
        "{\"id\":1,\"error\":{\"code\":\"tool_off\",\"message\":\"Aus.\",\"data\":{\"reason\":\"grant_off\"}}}\n",
    );
    match c.request("tools/call", json!({})).unwrap_err() {
        ClientError::Remote { code, message, data } => {
            assert_eq!(code, "tool_off");
            assert_eq!(message, "Aus.");
            assert_eq!(data.unwrap()["reason"], json!("grant_off"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_error_line_without_a_matching_id_still_counts_as_an_error() {
    // So antwortet der Server, wenn er die Verbindung sofort ablehnt (zu viele Verbindungen).
    let (mut c, _) = canned("{\"id\":null,\"error\":{\"code\":\"too_many_connections\",\"message\":\"x\"}}\n");
    assert_eq!(
        remote_code(&c.request("status", json!({})).unwrap_err()),
        Some("too_many_connections")
    );
}

#[test]
fn a_result_for_another_request_is_a_protocol_error() {
    let (mut c, _) = canned("{\"id\":99,\"result\":{}}\n");
    assert!(matches!(c.request("status", json!({})), Err(ClientError::Protocol(_))));
}

#[test]
fn a_closed_connection_is_a_transport_error() {
    let (mut c, _) = canned("");
    match c.request("status", json!({})).unwrap_err() {
        ClientError::Transport(m) => assert!(m.contains("beendet"), "{m}"),
        other => panic!("{other:?}"),
    }
    // Ende mitten in der Zeile ebenfalls.
    let (mut c, _) = canned("{\"id\":1,\"res");
    assert!(matches!(c.request("status", json!({})), Err(ClientError::Transport(_))));
}

#[test]
fn garbage_replies_are_protocol_errors() {
    for reply in ["kein json\n", "[1]\n", "{\"id\":1}\n", "\u{fffd}\n"] {
        let (mut c, _) = canned(reply);
        assert!(matches!(c.request("status", json!({})), Err(ClientError::Protocol(_))), "{reply:?}");
    }
}

#[test]
fn an_oversized_request_is_not_sent() {
    let (mut c, _) = canned("");
    let big = "x".repeat(MAX_LINE_BYTES + 1);
    assert!(matches!(c.request("tools/call", json!({"blob": big})), Err(ClientError::Protocol(_))));
    assert!(c.writer.is_empty(), "nichts wurde gesendet");
}

#[test]
fn an_endless_reply_line_is_cut_off() {
    let (mut c, _) = canned(&"x".repeat(3 * MAX_LINE_BYTES));
    assert!(matches!(c.request("status", json!({})), Err(ClientError::Protocol(_))));
}

#[test]
fn auth_errors_are_recognised_by_code() {
    let remote = |c: &str| ClientError::Remote {
        code: c.to_string(),
        message: String::new(),
        data: None,
    };
    for c in ["token_invalid", "token_revoked", "unauthenticated"] {
        assert!(is_auth_error(&remote(c)), "{c}");
    }
    for c in ["tool_off", "rate_limited", "failed"] {
        assert!(!is_auth_error(&remote(c)), "{c}");
    }
    assert!(!is_auth_error(&ClientError::NotRunning(String::new())));
}

#[test]
fn hello_sends_the_token_only_when_given() {
    let (mut c, _) = canned("{\"id\":1,\"result\":{}}\n");
    c.hello(Some("lvat_abc")).unwrap();
    assert!(String::from_utf8(c.writer.clone()).unwrap().contains("lvat_abc"));
    let (mut c, _) = canned("{\"id\":1,\"result\":{}}\n");
    c.hello(None).unwrap();
    assert!(!String::from_utf8(c.writer.clone()).unwrap().contains("token"));
}
