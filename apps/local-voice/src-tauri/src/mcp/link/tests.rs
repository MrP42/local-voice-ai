use serde_json::json;

use super::*;

const TOKEN: &str = "lvat_GEHEIMGEHEIMGEHEIMGEHEIMGEHEIMGEHEIMGEHEIM";

#[test]
fn the_bridge_tools_are_the_catalog_plus_the_status_tool() {
    for name in [
        "add_youtube_source",
        "start_recording",
        "stop_recording",
        "transcribe_file",
        "create_session",
        "create_meeting",
        "tts_page_create",
        "tts_render_audio",
        "get_action_status",
    ] {
        assert!(is_bridge_tool(name), "{name}");
    }
    for name in [
        "list_meetings",
        "get_transcript",
        "get_provenance",
        "delete_meeting",
        "",
    ] {
        assert!(!is_bridge_tool(name), "{name}");
    }
}

#[test]
fn a_bridge_definition_becomes_an_mcp_tool_without_the_mode() {
    let t = json!({
        "name": "create_meeting", "title": "Besprechung anlegen", "description": "Legt an.",
        "inputSchema": {"type": "object", "properties": {"title": {"type": "string"}}},
        "annotations": {"readOnlyHint": false}, "mode": "ask"
    });
    let tool = to_mcp_tool(&t).unwrap();
    assert_eq!(tool["name"], "create_meeting");
    assert_eq!(tool["inputSchema"]["properties"]["title"]["type"], "string");
    assert_eq!(tool["annotations"]["readOnlyHint"], false);
    assert!(
        tool.get("mode").is_none(),
        "das Recht gehoert der App, nicht dem Agenten"
    );
    // Unbekannte oder namenlose Eintraege der Gegenstelle werden nie weitergereicht.
    assert!(to_mcp_tool(&json!({"name": "rm_rf"})).is_none());
    assert!(to_mcp_tool(&json!({"description": "x"})).is_none());
    // Ohne Schema gibt es ein leeres Objektschema.
    assert_eq!(
        to_mcp_tool(&json!({"name": "stop_recording"})).unwrap()["inputSchema"]["type"],
        "object"
    );
}

#[test]
fn call_replies_are_done_pending_or_a_failure() {
    assert_eq!(
        parse_call_reply(&json!({"status": "done", "result": {"meeting_id": "M1"}})).unwrap(),
        LinkReply::Done(json!({"meeting_id": "M1"}))
    );
    match parse_call_reply(&json!({"status": "pending", "approval_id": "A1", "message": "Warten."}))
        .unwrap()
    {
        LinkReply::Pending {
            approval_id,
            message,
        } => {
            assert_eq!(approval_id, "A1");
            assert_eq!(message, "Warten.");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        parse_call_reply(&json!({"status": "komisch"}))
            .unwrap_err()
            .kind,
        LinkErrorKind::Failed
    );
    assert_eq!(
        parse_call_reply(&json!({})).unwrap_err().kind,
        LinkErrorKind::Failed
    );
}

#[test]
fn errors_of_the_bridge_become_readable_hints_that_never_contain_the_token() {
    let remote = |c: &str, m: &str| ClientError::Remote {
        code: c.to_string(),
        message: m.to_string(),
        data: None,
    };
    let cases = [
        (
            ClientError::NotRunning("x".into()),
            LinkErrorKind::NotRunning,
        ),
        (
            ClientError::UntrustedServer("x".into()),
            LinkErrorKind::NotRunning,
        ),
        (
            ClientError::Transport("x".into()),
            LinkErrorKind::NotRunning,
        ),
        (
            ClientError::Protocol("kaputt".into()),
            LinkErrorKind::Failed,
        ),
        (
            remote("token_invalid", "Das Token ist ungültig."),
            LinkErrorKind::Auth,
        ),
        (
            remote("token_revoked", "zurückgezogen"),
            LinkErrorKind::Auth,
        ),
        (
            remote("unauthenticated", "Nicht angemeldet"),
            LinkErrorKind::Auth,
        ),
        (
            remote("tool_off", "Das Werkzeug ist aus."),
            LinkErrorKind::Denied,
        ),
        (
            remote("approval_denied", "Abgelehnt"),
            LinkErrorKind::Denied,
        ),
        (remote("failed", "Es ging schief"), LinkErrorKind::Failed),
        (
            remote("rate_limited", "Zu viele Anfragen"),
            LinkErrorKind::Failed,
        ),
    ];
    for (e, kind) in cases {
        let h = explain(&e);
        assert_eq!(h.kind, kind, "{e:?}");
        assert!(!h.text.is_empty());
        assert!(!h.text.contains(TOKEN));
    }
    assert!(explain(&remote("token_invalid", "m"))
        .text
        .contains("LVA_AGENT_TOKEN"));
    assert!(explain(&ClientError::NotRunning("x".into()))
        .text
        .contains("App starten"));
}

#[test]
fn without_a_token_there_is_no_link() {
    // Die Umgebung dieses Tests hat kein Token (Tests setzen es nie): kein Link, keine Pipe.
    if std::env::var(TOKEN_ENV).is_err() {
        assert!(PipeLink::from_env().is_none());
    }
}

#[test]
fn a_missing_pipe_is_not_running_and_fast() {
    let link = PipeLink::with(TOKEN, r"\\.\pipe\lva-mcp-test-gibt-es-nicht");
    let started = std::time::Instant::now();
    let e = link.tools().unwrap_err();
    assert_eq!(e.kind, LinkErrorKind::NotRunning, "{e:?}");
    assert!(!e.text.contains(TOKEN));
    let e = link.call("create_meeting", &json!({}), None).unwrap_err();
    assert_eq!(e.kind, LinkErrorKind::NotRunning);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "keine Wartezeit ohne App"
    );
}

#[test]
fn a_bad_pipe_name_is_a_failure_with_the_reason() {
    let link = PipeLink {
        token: TOKEN.to_string(),
        pipe_name: Err("LVA_AGENT_PIPE ungültig".to_string()),
    };
    let e = link.tools().unwrap_err();
    assert_eq!(e.kind, LinkErrorKind::Failed);
    assert!(e.text.contains("ungültig"));
}

#[cfg(windows)]
mod live {
    use std::sync::Arc;

    use tokio::sync::oneshot;
    use ulid::Ulid;

    use super::*;
    use crate::agent_bridge::clients;
    use crate::agent_bridge::server::Server;
    use crate::agent_bridge::testkit::Fixture;
    use crate::managers::integrations::model::GrantMode;

    fn unique_name() -> String {
        format!(r"\\.\pipe\lva-mcp-test-{}", Ulid::new())
    }

    async fn start(f: &Fixture, name: &str) -> Arc<Server> {
        let server = Server::new(f.bridge.clone());
        let (tx, rx) = oneshot::channel();
        let (s2, n2) = (server.clone(), name.to_string());
        tokio::spawn(async move { pipe::serve(s2, &n2, Some(tx)).await });
        rx.await.unwrap().expect("Pipe angelegt");
        server
    }

    async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        tokio::task::spawn_blocking(f).await.unwrap()
    }

    #[tokio::test]
    async fn the_list_shows_only_what_the_user_allowed_and_calls_run_through_the_rights() {
        let f = Fixture::new();
        let name = unique_name();
        let _s = start(&f, &name).await;
        f.grant("create_meeting", GrantMode::Allow);
        f.grant("transcribe_file", GrantMode::Ask);
        let link = PipeLink::with(&f.token, &name);

        let l = link;
        let (tools, done, pending, off) = blocking(move || {
            let tools = l.tools().unwrap();
            let done = l.call("create_meeting", &json!({"title": "Direkt"}), None);
            let pending = l.call("transcribe_file", &json!({"path": "C:/a.wav"}), None);
            let off = l.call("start_recording", &json!({}), None);
            (tools, done, pending, off)
        })
        .await;
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"create_meeting") && names.contains(&"transcribe_file"));
        assert!(names.contains(&"get_action_status"), "{names:?}");
        assert!(
            !names.contains(&"start_recording"),
            "ausgeschaltet: fehlt in der Liste"
        );
        assert!(tools.iter().all(|t| t.get("mode").is_none()));
        match done.unwrap() {
            LinkReply::Done(v) => assert_eq!(v["ran"], "create_meeting"),
            other => panic!("{other:?}"),
        }
        match pending.unwrap() {
            LinkReply::Pending {
                approval_id,
                message,
            } => {
                assert!(!approval_id.is_empty());
                assert!(message.contains("Freigabe"));
            }
            other => panic!("{other:?}"),
        }
        let e = off.unwrap_err();
        assert_eq!(e.kind, LinkErrorKind::Denied, "{e:?}");
        // Das Werkzeug lief nur fuer `create_meeting`, nie fuer die offene Freigabe.
        let calls = f.calls.all();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "create_meeting");
    }

    #[tokio::test]
    async fn a_wrong_or_revoked_token_is_an_auth_error_and_runs_nothing() {
        let f = Fixture::new();
        let name = unique_name();
        let _s = start(&f, &name).await;
        f.grant("create_meeting", GrantMode::Allow);
        let bad = PipeLink::with(&clients::generate_token(), &name);
        let e = blocking(move || bad.tools()).await.unwrap_err();
        assert_eq!(e.kind, LinkErrorKind::Auth);
        clients::revoke(&f.conn(), &f.client.id, f.now()).unwrap();
        let revoked = PipeLink::with(&f.token, &name);
        let e = blocking(move || revoked.call("create_meeting", &json!({}), None))
            .await
            .unwrap_err();
        assert_eq!(e.kind, LinkErrorKind::Auth);
        assert_eq!(f.calls.len(), 0);
    }
}
