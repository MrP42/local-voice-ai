use serde_json::json;
use tokio::io::BufReader;

use super::*;

fn req(line: &str) -> (Value, String, Value) {
    match parse_request(line) {
        Incoming::Request { id, method, params } => (id, method, params),
        other => panic!("{other:?}"),
    }
}

fn bad(line: &str) -> String {
    match parse_request(line) {
        Incoming::Invalid { message, .. } => message,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_request_is_split_into_id_method_and_params() {
    let (id, m, p) = req(r#"{"id": 7, "method": "tools/call", "params": {"name": "x"}}"#);
    assert_eq!((id, m.as_str(), p), (json!(7), "tools/call", json!({"name": "x"})));
    let (id, _, p) = req(r#"{"id": "a-1", "method": "status"}"#);
    assert_eq!((id, p), (json!("a-1"), json!({})));
    let (id, _, p) = req(r#"{"method": "status", "params": null}"#);
    assert_eq!((id, p), (Value::Null, json!({})));
}

#[test]
fn malformed_requests_say_why() {
    assert!(bad("das ist kein json").contains("JSON"));
    assert!(bad("[1,2]").contains("Objekt"));
    assert!(bad(r#"{"id": 1}"#).contains("Methode"));
    assert!(bad(r#"{"id": 1, "method": 5}"#).contains("Methode"));
    assert!(bad(r#"{"id": 1, "method": ""}"#).contains("leer"));
    assert!(bad(&format!(r#"{{"id": 1, "method": "{}"}}"#, "x".repeat(65))).contains("lang"));
    assert!(bad(r#"{"id": 1, "method": "status", "params": [1]}"#).contains("params"));
    assert!(bad(r#"{"id": [1], "method": "status"}"#).contains("Kennung"));
    assert!(bad(r#"{"id": {"a":1}, "method": "status"}"#).contains("Kennung"));
}

#[test]
fn an_invalid_request_keeps_the_id_when_it_has_a_usable_one() {
    match parse_request(r#"{"id": 9, "method": 5}"#) {
        Incoming::Invalid { id, .. } => assert_eq!(id, json!(9)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn replies_are_single_json_lines_with_the_id_echoed() {
    let ok = ok_line(&json!("x"), json!({"a": 1}));
    assert!(!ok.contains('\n'));
    assert_eq!(serde_json::from_str::<Value>(&ok).unwrap(), json!({"id": "x", "result": {"a": 1}}));
    let err = err_line(&json!(3), code::TOOL_OFF, "Das Werkzeug ist aus.", Some(&json!({"reason": "grant_off"})));
    let v: Value = serde_json::from_str(&err).unwrap();
    assert_eq!(v["id"], json!(3));
    assert_eq!(v["error"]["code"], json!("tool_off"));
    assert_eq!(v["error"]["data"]["reason"], json!("grant_off"));
    let no_data: Value = serde_json::from_str(&err_line(&Value::Null, "x", "y", None)).unwrap();
    assert!(no_data["error"].get("data").is_none());
}

async fn read_all(input: &[u8], max: usize) -> Vec<LineRead> {
    let mut r = BufReader::with_capacity(8, input);
    let mut out = Vec::new();
    loop {
        let l = read_line_capped(&mut r, max).await.unwrap();
        let end = matches!(l, LineRead::Eof | LineRead::TooLong);
        out.push(l);
        if end {
            return out;
        }
    }
}

#[tokio::test]
async fn lines_are_read_one_by_one_across_small_buffers() {
    let got = read_all(b"{\"a\":1}\r\nzweite Zeile\n\ndritte", 100).await;
    assert_eq!(
        got,
        vec![
            LineRead::Line("{\"a\":1}".into()),
            LineRead::Line("zweite Zeile".into()),
            LineRead::Line(String::new()),
            LineRead::Line("dritte".into()),
            LineRead::Eof,
        ]
    );
}

#[tokio::test]
async fn an_overlong_line_stops_reading_instead_of_buffering() {
    assert_eq!(read_all(b"kurz\n", 4).await.last(), Some(&LineRead::Eof));
    let got = read_all(&[b'a'; 50], 10).await;
    assert_eq!(got, vec![LineRead::TooLong]);
    let got = read_all(b"abcdefghijklmnop\nrest", 10).await;
    assert_eq!(got, vec![LineRead::TooLong]);
    // Genau an der Grenze geht noch.
    assert_eq!(read_all(b"abcdefghij\n", 10).await[0], LineRead::Line("abcdefghij".into()));
}

#[tokio::test]
async fn invalid_utf8_is_reported_not_panicked_on() {
    let got = read_all(b"\xff\xfe\nok\n", 100).await;
    assert_eq!(got[0], LineRead::InvalidUtf8);
    assert_eq!(got[1], LineRead::Line("ok".into()));
}
