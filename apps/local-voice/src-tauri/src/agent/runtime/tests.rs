//! Laufzeit-Tests gegen eine llama-server-Attrappe (HTTP, `/v1/chat/completions`).

use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use super::*;
use crate::agent::test_support::{
    body, closed_port, cut, endpoint, mock, ok, user_of, R,
};
use crate::managers::meetings::llm_call::test_support::{spawn_llm_mock_with, MockReply};

fn schema() -> Value {
    json!({ "type": "object", "properties": { "ok": { "type": "boolean" } }, "required": ["ok"] })
}

fn req<'a>(schema: &'a Value) -> Request<'a> {
    Request {
        purpose: Purpose::Extract,
        system: "Systemprompt",
        user: "Nutzerprompt",
        schema_name: "test_schema",
        schema,
        hint_invalid: HINT_INVALID,
        hint_truncated: HINT_TRUNCATED,
    }
}

/// Gueltig ist ein Objekt mit `ok: true`.
fn parse_ok(raw: &str) -> Result<bool, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    match v["ok"].as_bool() {
        Some(true) => Ok(true),
        _ => Err("ok fehlt".to_string()),
    }
}

// -- Anfrage -----------------------------------------------------------------------------

#[tokio::test]
async fn the_request_carries_the_schema_turns_thinking_off_and_caps_tokens() {
    let m = mock(vec![ok(r#"{"ok":true}"#)]).await;
    let schema = schema();
    let answer = m.runtime().ask(&req(&schema), &parse_ok).await.unwrap();
    assert!(answer.value);

    let sent = m.requests();
    assert_eq!(sent.len(), 1);
    let r = &sent[0];
    assert_eq!(r["response_format"]["type"], "json_schema");
    assert_eq!(r["response_format"]["json_schema"]["name"], "test_schema");
    assert_eq!(r["response_format"]["json_schema"]["strict"], true);
    assert_eq!(r["response_format"]["json_schema"]["schema"], schema);
    assert_eq!(r["chat_template_kwargs"]["enable_thinking"], false);
    assert_eq!(r["temperature"], 0);
    assert_eq!(r["seed"], SEED);
    assert_eq!(r["max_tokens"], DEFAULT_MAX_TOKENS);
    assert_eq!(r["model"], "llm-test");
    assert_eq!(r["messages"][0]["role"], "system");
    assert_eq!(r["messages"][0]["content"], "Systemprompt");
    assert_eq!(r["messages"][1]["role"], "user");
    assert_eq!(user_of(r), "Nutzerprompt");
}

#[tokio::test]
async fn the_token_cap_is_configurable_and_always_sent() {
    let m = mock(vec![ok(r#"{"ok":true}"#)]).await;
    let schema = schema();
    let rt = m.runtime().with_config(AgentConfig {
        max_tokens: 77,
        ..AgentConfig::default()
    });
    rt.ask(&req(&schema), &parse_ok).await.unwrap();
    assert_eq!(m.requests()[0]["max_tokens"], 77);
}

#[tokio::test]
async fn a_valid_answer_costs_one_request_and_reports_model_tokens_and_time() {
    let m = mock(vec![ok(r#"{"ok":true}"#)]).await;
    let schema = schema();
    let a = m.runtime().ask(&req(&schema), &parse_ok).await.unwrap();
    assert_eq!(m.count(), 1);
    assert_eq!(a.attempts, 1);
    assert!(a.duration_ms < 60_000);
    assert_eq!(
        a.usage,
        Usage {
            prompt_tokens: 100,
            completion_tokens: 20
        }
    );
}

#[tokio::test]
async fn a_code_fence_or_think_residue_does_not_break_parsing_in_the_caller() {
    let m = mock(vec![ok("<think>ich überlege</think>{\"ok\":true}")]).await;
    let schema = schema();
    assert!(m.runtime().ask(&req(&schema), &parse_ok).await.unwrap().value);
}

// -- Wiederholversuch und Rueckfall ----------------------------------------------------------

#[tokio::test]
async fn invalid_json_gets_exactly_one_retry_with_a_hint_and_can_then_succeed() {
    let m = mock(vec![ok("das ist kein JSON"), ok(r#"{"ok":true}"#)]).await;
    let schema = schema();
    let a = m.runtime().ask(&req(&schema), &parse_ok).await.unwrap();
    assert_eq!(a.attempts, 2);
    assert_eq!(m.count(), 2);
    let sent = m.requests();
    assert_eq!(user_of(&sent[0]), "Nutzerprompt");
    assert!(user_of(&sent[1]).starts_with("Nutzerprompt"));
    assert!(user_of(&sent[1]).contains(HINT_INVALID));
    // Beide Versuche zaehlen im Verbrauch.
    assert_eq!(a.usage.prompt_tokens, 200);
    assert_eq!(a.usage.completion_tokens, 40);
}

#[tokio::test]
async fn invalid_json_twice_is_schema_invalid_after_exactly_two_requests() {
    let m = mock(vec![ok("{kaputt"), ok("auch kaputt")]).await;
    let schema = schema();
    let err = m
        .runtime()
        .ask(&req(&schema), &parse_ok)
        .await
        .unwrap_err();
    assert_eq!(m.count(), 2, "kein dritter Versuch");
    match err {
        AgentError::SchemaInvalid {
            truncated,
            raw_len,
            ref detail,
            usage,
            ..
        } => {
            assert!(!truncated);
            assert_eq!(raw_len, "auch kaputt".chars().count());
            assert!(!detail.contains("kaputt"), "kein Antworttext im Fehler");
            assert_eq!(usage.prompt_tokens, 200);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(err.code(), "schema_invalid");
}

#[tokio::test]
async fn a_truncated_answer_is_retried_once_with_the_shorten_hint() {
    let m = mock(vec![cut(r#"{"ok":"#), ok(r#"{"ok":true}"#)]).await;
    let schema = schema();
    let a = m.runtime().ask(&req(&schema), &parse_ok).await.unwrap();
    assert_eq!(a.attempts, 2);
    let sent = m.requests();
    assert!(user_of(&sent[1]).contains(HINT_TRUNCATED));
    assert!(!user_of(&sent[1]).contains(HINT_INVALID));
}

#[tokio::test]
async fn a_truncated_answer_twice_is_reported_as_truncated() {
    let m = mock(vec![cut(r#"{"ok":"#)]).await;
    let schema = schema();
    let err = m
        .runtime()
        .ask(&req(&schema), &parse_ok)
        .await
        .unwrap_err();
    assert_eq!(m.count(), 2);
    assert!(matches!(
        err,
        AgentError::SchemaInvalid {
            truncated: true,
            ..
        }
    ));
    assert_eq!(err.code(), "truncated");
}

#[tokio::test]
async fn a_valid_json_the_caller_rejects_counts_as_invalid() {
    // Gueltiges JSON, aber ohne `ok: true`: die Fachpruefung des Aufrufers verwirft es.
    let m = mock(vec![ok(r#"{"ok":false}"#), ok(r#"{"ok":true}"#)]).await;
    let schema = schema();
    let a = m.runtime().ask(&req(&schema), &parse_ok).await.unwrap();
    assert_eq!(a.attempts, 2);
}

#[tokio::test]
async fn empty_content_and_missing_choices_are_invalid_never_a_panic() {
    for reply in [
        R::Ok(r#"{"choices":[]}"#.to_string()),
        R::Ok(r#"{"choices":[{"message":{"content":null}}]}"#.to_string()),
        R::Ok(r#"{}"#.to_string()),
        R::Ok(body("", "stop", 1, 0)),
    ] {
        let m = mock(vec![reply]).await;
        let schema = schema();
        let err = m
            .runtime()
            .ask(&req(&schema), &parse_ok)
            .await
            .unwrap_err();
        assert!(
            matches!(err, AgentError::SchemaInvalid { .. }),
            "{err:?}"
        );
        assert_eq!(m.count(), 2);
    }
}

// -- Server nicht verfuegbar --------------------------------------------------------------------

#[tokio::test]
async fn a_closed_port_is_unavailable_without_an_internal_retry() {
    let port = closed_port().await;
    let schema = schema();
    let err = endpoint(port)
        .ask(&req(&schema), &parse_ok)
        .await
        .unwrap_err();
    assert!(matches!(err, AgentError::Unavailable(_)), "{err:?}");
    assert_eq!(err.code(), "unavailable");
}

#[tokio::test]
async fn a_dropped_connection_mid_request_is_unavailable() {
    // Der Server stirbt, nachdem er die Verbindung angenommen hat.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            drop(socket);
        }
    });
    let schema = schema();
    let err = endpoint(port)
        .ask(&req(&schema), &parse_ok)
        .await
        .unwrap_err();
    assert!(matches!(err, AgentError::Unavailable(_)), "{err:?}");
}

#[tokio::test]
async fn a_hanging_server_times_out_within_the_limit_and_is_not_retried() {
    let m = mock(vec![R::Hang]).await;
    let schema = schema();
    let rt = m.runtime().with_config(AgentConfig {
        request_timeout: Duration::from_millis(300),
        ..AgentConfig::default()
    });
    let started = Instant::now();
    let err = rt.ask(&req(&schema), &parse_ok).await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "blockiert laenger als die Grenze"
    );
    assert!(matches!(err, AgentError::Timeout { .. }), "{err:?}");
    assert_eq!(m.count(), 1, "ein Zeitfehler wiederholt nicht von selbst");
}

#[tokio::test]
async fn a_busy_server_is_busy_with_a_wait_not_a_failure() {
    for status in [503u16, 429] {
        let m = mock(vec![R::Status(status)]).await;
        let schema = schema();
        let err = m
            .runtime()
            .ask(&req(&schema), &parse_ok)
            .await
            .unwrap_err();
        match err {
            AgentError::Busy { retry_after_ms, .. } => {
                assert_eq!(retry_after_ms, SERVER_BUSY_RETRY_MS)
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(m.count(), 1);
    }
}

#[tokio::test]
async fn context_overflow_is_reported_so_the_caller_can_split() {
    let m = mock(vec![R::StatusBody(
        400,
        r#"{"error":{"code":400,"message":"the request exceeds the available context size, try increasing it","type":"exceed_context_size_error"}}"#
            .to_string(),
    )])
    .await;
    let schema = schema();
    let err = m
        .runtime()
        .ask(&req(&schema), &parse_ok)
        .await
        .unwrap_err();
    assert_eq!(err, AgentError::ContextExceeded);
    assert_eq!(m.count(), 1);
}

#[tokio::test]
async fn other_http_errors_are_classified_without_leaking_the_body() {
    let m = mock(vec![R::StatusBody(500, "Transkript: geheim".to_string())]).await;
    let schema = schema();
    let err = m
        .runtime()
        .ask(&req(&schema), &parse_ok)
        .await
        .unwrap_err();
    assert!(matches!(err, AgentError::Unavailable(_)));
    assert!(!err.describe().contains("geheim"));

    let m = mock(vec![R::StatusBody(401, "nope".to_string())]).await;
    let err = m
        .runtime()
        .ask(&req(&schema), &parse_ok)
        .await
        .unwrap_err();
    assert_eq!(err, AgentError::Rejected { status: 401 });
}

#[test]
fn manager_errors_map_to_busy_unavailable_or_not_configured() {
    assert!(matches!(
        from_manager_error("Zu wenig freier Arbeitsspeicher (3000 MB frei, 8000 MB nötig)"),
        AgentError::Busy {
            retry_after_ms: MEMORY_RETRY_MS,
            ..
        }
    ));
    assert!(matches!(
        from_manager_error("memory_low: zu wenig RAM"),
        AgentError::Busy { .. }
    ));
    assert!(matches!(
        from_manager_error("server_crashed: Das lokale Sprachmodell ist abgestürzt."),
        AgentError::Busy {
            retry_after_ms: CRASH_RETRY_MS,
            ..
        }
    ));
    assert!(matches!(
        from_manager_error("Modell nicht geladen: llm-x"),
        AgentError::NotConfigured(_)
    ));
    assert!(matches!(
        from_manager_error("Lokales Sprachmodell nicht initialisiert"),
        AgentError::NotConfigured(_)
    ));
    // Unbekanntes: wiederholbar, ohne den Fehlertext (Pfade) weiterzureichen.
    let e = from_manager_error(r"Start gescheitert in C:\Users\x\modell.gguf");
    assert!(matches!(e, AgentError::Unavailable(_)));
    assert!(!e.describe().contains("gguf"));
}

#[tokio::test]
async fn the_local_target_without_a_manager_is_not_configured_and_starts_nothing() {
    // In Tests gibt es keinen Modellverwalter (`install_globals` laeuft nur in der App).
    let rt = AgentRuntime::new(Target::Local {
        model: "llm-gemma4-e4b-q4".into(),
    });
    let schema = schema();
    let err = rt.ask(&req(&schema), &parse_ok).await.unwrap_err();
    assert!(matches!(err, AgentError::NotConfigured(_)), "{err:?}");
    assert!(rt.is_local());
}

// -- Gleichzeitigkeit ----------------------------------------------------------------------------

#[tokio::test]
async fn parallel_asks_do_not_share_state() {
    // Die Attrappe antwortet je nach Prompt; vier Aufrufe gleichzeitig muessen je ihre
    // eigene Antwort bekommen.
    let port = spawn_llm_mock_with(|request| {
        let v: Value = serde_json::from_str(request).unwrap();
        let user = v["messages"][1]["content"].as_str().unwrap_or_default();
        let n: u64 = user.trim_start_matches("Nutzer ").parse().unwrap_or(0);
        MockReply::Body(body(&format!(r#"{{"ok":true,"n":{n}}}"#), "stop", n, n))
    })
    .await;
    let rt = Arc::new(endpoint(port));
    let schema = schema();
    let parse = |raw: &str| -> Result<u64, String> {
        let v: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        v["n"].as_u64().ok_or_else(|| "n fehlt".to_string())
    };
    let run = |n: u64| {
        let rt = rt.clone();
        let schema = schema.clone();
        async move {
            let user = format!("Nutzer {n}");
            let request = Request {
                user: &user,
                ..req(&schema)
            };
            let a = rt.ask(&request, &parse).await.unwrap();
            (n, a.value, a.usage.prompt_tokens)
        }
    };
    let results = tokio::join!(run(1), run(2), run(3), run(4));
    for (asked, got, tokens) in [results.0, results.1, results.2, results.3] {
        assert_eq!(asked, got);
        assert_eq!(asked, tokens);
    }
}

// -- Werkzeugwahl mit Rueckfall no_action ------------------------------------------------------------

fn tools() -> Vec<ToolSpec> {
    let tool = |name: &str, props: Value, required: &[&str]| ToolSpec {
        name: name.into(),
        description: format!("Beschreibung {name}"),
        parameters: json!({ "type": "object", "properties": props, "required": required }),
    };
    vec![
        tool("create_task", json!({ "title": { "type": "string" } }), &["title"]),
        tool("send_mail", json!({ "to": { "type": "string" } }), &["to"]),
        tool(NO_ACTION, json!({ "reason": { "type": "string" } }), &["reason"]),
    ]
}

#[tokio::test]
async fn choose_returns_the_tool_and_sends_a_one_of_schema_of_the_offered_tools_only() {
    let m = mock(vec![ok(
        r#"{"tool":"create_task","arguments":{"title":"Angebot senden"}}"#,
    )])
    .await;
    let all = tools();
    let offered = schema::offered(&all, Some(&["create_task".to_string()])).unwrap();
    let chosen = m
        .runtime()
        .choose("System", "Anfrage", &offered)
        .await
        .unwrap();
    assert_eq!(chosen.choice.tool, "create_task");
    assert_eq!(chosen.choice.arguments["title"], "Angebot senden");
    assert!(chosen.fallback.is_none());
    assert_eq!(chosen.attempts, 1);

    let sent = m.requests();
    let variants = sent[0]["response_format"]["json_schema"]["schema"]["oneOf"]
        .as_array()
        .unwrap();
    let names: Vec<_> = variants
        .iter()
        .map(|v| v["properties"]["tool"]["const"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["create_task", NO_ACTION], "send_mail wurde nicht angeboten");
    assert_eq!(sent[0]["chat_template_kwargs"]["enable_thinking"], false);
}

#[tokio::test]
async fn choose_falls_back_to_no_action_after_two_unusable_answers() {
    for replies in [
        vec![ok("{kaputt"), ok("noch kaputt")],
        vec![cut(r#"{"tool":"crea"#)],
        vec![ok(r#"{"tool":"create_task"}"#)],
    ] {
        let m = mock(replies).await;
        let all = tools();
        let offered = schema::offered(&all, None).unwrap();
        let chosen = m.runtime().choose("S", "U", &offered).await.unwrap();
        assert_eq!(chosen.choice.tool, NO_ACTION);
        assert!(chosen.choice.arguments["reason"]
            .as_str()
            .is_some_and(|r| !r.is_empty()));
        assert!(matches!(
            chosen.fallback,
            Some(AgentError::SchemaInvalid { .. })
        ));
        assert_eq!(m.count(), 2, "genau ein Wiederholversuch");
        assert_eq!(chosen.usage.prompt_tokens, 200, "der Verbrauch bleibt sichtbar");
    }
}

#[tokio::test]
async fn choose_never_returns_a_tool_outside_the_offered_list_even_if_the_model_names_it() {
    let m = mock(vec![ok(
        r#"{"tool":"send_mail","arguments":{"to":"alle@example.com"}}"#,
    )])
    .await;
    let all = tools();
    let offered = schema::offered(&all, Some(&["create_task".to_string()])).unwrap();
    let chosen = m.runtime().choose("S", "U", &offered).await.unwrap();
    assert_eq!(chosen.choice.tool, NO_ACTION);
    assert!(chosen.fallback.is_some());
}

#[tokio::test]
async fn choose_can_recover_on_the_retry() {
    let m = mock(vec![
        ok("{kaputt"),
        ok(r#"{"tool":"no_action","arguments":{"reason":"nichts zu tun"}}"#),
    ])
    .await;
    let all = tools();
    let offered = schema::offered(&all, None).unwrap();
    let chosen = m.runtime().choose("S", "U", &offered).await.unwrap();
    assert_eq!(chosen.attempts, 2);
    assert!(chosen.fallback.is_none());
    assert_eq!(chosen.choice.arguments["reason"], "nichts zu tun");
}

#[tokio::test]
async fn choose_passes_server_failures_through_instead_of_hiding_them_as_no_action() {
    let m = mock(vec![R::Status(503)]).await;
    let all = tools();
    let offered = schema::offered(&all, None).unwrap();
    let err = m.runtime().choose("S", "U", &offered).await.unwrap_err();
    assert!(matches!(err, AgentError::Busy { .. }));
}

#[test]
fn no_action_carries_a_reason() {
    let c = no_action("weil");
    assert_eq!(c.tool, NO_ACTION);
    assert_eq!(c.arguments["reason"], "weil");
}

#[test]
fn error_texts_are_german_sentences_without_prompt_content() {
    let errors = [
        AgentError::NotConfigured("x.".into()),
        AgentError::Timeout { after_ms: 180_000 },
        AgentError::ContextExceeded,
        AgentError::Rejected { status: 400 },
        AgentError::SchemaInvalid {
            truncated: true,
            raw_len: 5,
            detail: "d".into(),
            usage: Usage::default(),
            duration_ms: 1,
        },
    ];
    for e in errors {
        assert!(!e.describe().is_empty());
        assert!(!e.code().is_empty());
    }
    assert!(AgentError::Timeout { after_ms: 180_000 }
        .describe()
        .contains("180 Sekunden"));
}
