//! Tests von `ctl`: Eingaben, Exit-Codes (rein) und gegen eine laufende Bruecke ueber die
//! echte Named Pipe (Windows).

use clap::Parser;
use serde_json::json;

use super::*;

/// Nur um `CtlArgs` ueber clap zu parsen, ohne die ganze App-CLI.
#[derive(Parser, Debug)]
struct Wrapper {
    #[command(flatten)]
    ctl: CtlArgs,
}

fn parse(args: &[&str]) -> CtlArgs {
    let mut v = vec!["ctl"];
    v.extend(args);
    Wrapper::try_parse_from(v).unwrap().ctl
}

// --- Eingaben -----------------------------------------------------------------------

#[test]
fn the_verbs_and_global_flags_parse_in_any_order() {
    let a = parse(&["status", "--json"]);
    assert!(a.json && matches!(a.verb, Verb::Status));
    let a = parse(&["--json", "--out", "x.json", "status"]);
    assert!(a.json && a.out.as_deref() == Some(Path::new("x.json")));
    let a = parse(&["call", "transcribe_file", "--args", "{}", "--approval", "A1", "--json"]);
    match a.verb {
        Verb::Call { tool, args, approval } => {
            assert_eq!(tool, "transcribe_file");
            assert_eq!(args.as_deref(), Some("{}"));
            assert_eq!(approval.as_deref(), Some("A1"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(parse(&["approval", "01ABC"]).verb, Verb::Approval { .. }));
    assert!(matches!(parse(&["tools", "--token-file", "t.txt"]).verb, Verb::Tools));
}

#[test]
fn unknown_verbs_and_missing_arguments_are_refused_by_the_parser() {
    assert!(Wrapper::try_parse_from(["ctl"]).is_err());
    assert!(Wrapper::try_parse_from(["ctl", "frobnicate"]).is_err());
    assert!(Wrapper::try_parse_from(["ctl", "call"]).is_err());
    assert!(Wrapper::try_parse_from(["ctl", "approval"]).is_err());
    // Ein Token als Argument gibt es absichtlich nicht.
    assert!(Wrapper::try_parse_from(["ctl", "status", "--token", "lvat_x"]).is_err());
}

#[test]
fn the_token_comes_from_a_file_first_then_the_environment_and_is_cleaned() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.txt");
    std::fs::write(&file, "\u{feff}lvat_aaa  \r\nzweite Zeile\r\n").unwrap();
    assert_eq!(
        read_token(Some(&file), Some("lvat_env".into())).unwrap().as_deref(),
        Some("lvat_aaa"),
        "Datei hat Vorrang, BOM und Rest der Datei fallen weg"
    );
    assert_eq!(read_token(None, Some("  lvat_env \n".into())).unwrap().as_deref(), Some("lvat_env"));
    assert_eq!(read_token(None, None).unwrap(), None);
    assert_eq!(read_token(None, Some("   ".into())).unwrap(), None);
    assert!(read_token(Some(&dir.path().join("fehlt.txt")), None).is_err());
    let empty = dir.path().join("leer.txt");
    std::fs::write(&empty, "\n").unwrap();
    assert!(read_token(Some(&empty), None).is_err());
}

#[test]
fn call_arguments_must_be_a_json_object_inline_or_from_a_file() {
    assert_eq!(parse_args(r#"{"path": "C:/a.wav"}"#).unwrap(), json!({"path": "C:/a.wav"}));
    assert!(parse_args("[1]").is_err());
    assert!(parse_args("kein json").is_err());
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.json");
    std::fs::write(&f, "\u{feff}{\"x\": 1}").unwrap();
    assert_eq!(parse_args(&format!("@{}", f.display())).unwrap(), json!({"x": 1}));
    assert!(parse_args("@/gibt/es/nicht.json").is_err());
}

#[test]
fn error_codes_map_to_the_documented_exit_codes() {
    for (c, exit) in [
        ("token_invalid", EXIT_AUTH),
        ("token_revoked", EXIT_AUTH),
        ("unauthenticated", EXIT_AUTH),
        ("tool_off", EXIT_DENIED),
        ("denied", EXIT_DENIED),
        ("approval_denied", EXIT_DENIED),
        ("shutting_down", EXIT_NOT_RUNNING),
        ("too_many_connections", EXIT_NOT_RUNNING),
        ("unknown_tool", EXIT_ERROR),
        ("tool_unavailable", EXIT_ERROR),
        ("failed", EXIT_ERROR),
        ("rate_limited", EXIT_ERROR),
        ("approval_expired", EXIT_ERROR),
        ("bad_request", EXIT_ERROR),
        ("etwas_neues", EXIT_ERROR),
    ] {
        assert_eq!(exit_for_remote(c), exit, "{c}");
    }
    assert_eq!((EXIT_OK, EXIT_ERROR, EXIT_NOT_RUNNING, EXIT_DENIED, EXIT_AUTH, EXIT_PENDING), (0, 1, 2, 3, 4, 5));
}

#[test]
fn client_errors_map_to_exit_codes() {
    assert_eq!(from_error(&ClientError::NotRunning("x".into())).exit, EXIT_NOT_RUNNING);
    assert_eq!(from_error(&ClientError::UntrustedServer("x".into())).exit, EXIT_NOT_RUNNING);
    assert_eq!(from_error(&ClientError::Transport("x".into())).exit, EXIT_NOT_RUNNING);
    assert_eq!(from_error(&ClientError::Protocol("x".into())).exit, EXIT_ERROR);
    let r = from_error(&ClientError::Remote {
        code: "tool_off".into(),
        message: "Aus.".into(),
        data: None,
    });
    assert_eq!(r.exit, EXIT_DENIED);
    assert_eq!(r.json["error"]["code"], json!("tool_off"));
    assert_eq!(r.json["ok"], json!(false));
}

#[test]
fn bad_inputs_fail_before_any_connection_is_made() {
    // Die Pipe gibt es nicht: waere verbunden worden, kaeme Exit 2.
    let env = |token: Option<&str>| CtlEnv {
        pipe_name: Ok(r"\\.\pipe\gibt-es-nicht".to_string()),
        token: Ok(token.map(str::to_string)),
        deadline: Duration::from_secs(5),
    };
    let bad_args = parse(&["call", "transcribe_file", "--args", "[1]"]);
    assert_eq!(execute(&bad_args, &env(Some("lvat_x"))).exit, EXIT_ERROR);
    let no_token = parse(&["tools"]);
    let r = execute(&no_token, &env(None));
    assert_eq!(r.exit, EXIT_AUTH);
    assert_eq!(r.json["error"]["code"], json!("no_token"));
    let broken = CtlEnv {
        pipe_name: Ok(r"\\.\pipe\x".into()),
        token: Err("kaputt".into()),
        deadline: Duration::from_secs(5),
    };
    assert_eq!(execute(&parse(&["status"]), &broken).exit, EXIT_ERROR);
}

#[test]
fn a_pipe_name_error_is_exit_1_with_the_reason() {
    let env = CtlEnv {
        pipe_name: Err("LVA_AGENT_PIPE ungueltig".into()),
        token: Ok(None),
        deadline: Duration::from_secs(5),
    };
    let r = execute(&parse(&["status"]), &env);
    assert_eq!(r.exit, EXIT_ERROR);
    assert!(r.stderr.contains("ungueltig"));
}

// --- Gegen eine laufende Bruecke (Windows) -----------------------------------------------

#[cfg(windows)]
mod live {
    use std::sync::Arc;

    use tokio::sync::oneshot;
    use ulid::Ulid;

    use super::*;
    use crate::agent_bridge::clients;
    use crate::agent_bridge::server::Server;
    use crate::agent_bridge::testkit::{fast_config, Fixture, ALL_TOOLS};
    use crate::agent_bridge::Config;
    use crate::managers::integrations::model::GrantMode;

    fn unique_name() -> String {
        format!(r"\\.\pipe\lva-ctl-test-{}", Ulid::new())
    }

    async fn start(f: &Fixture, name: &str) -> Arc<Server> {
        let server = Server::new(f.bridge.clone());
        let (tx, rx) = oneshot::channel();
        let (s2, n2) = (server.clone(), name.to_string());
        tokio::spawn(async move { pipe::serve(s2, &n2, Some(tx)).await });
        rx.await.unwrap().expect("Pipe angelegt");
        server
    }

    fn env(name: &str, token: Option<&str>) -> CtlEnv {
        CtlEnv {
            pipe_name: Ok(name.to_string()),
            token: Ok(token.map(str::to_string)),
            deadline: Duration::from_secs(20),
        }
    }

    async fn ctl(args: &[&str], env: CtlEnv) -> CtlResult {
        let parsed = parse(args);
        tokio::task::spawn_blocking(move || execute(&parsed, &env)).await.unwrap()
    }

    #[tokio::test]
    async fn status_without_a_token_is_exit_0() {
        let f = Fixture::new();
        let name = unique_name();
        let _s = start(&f, &name).await;
        let r = ctl(&["status", "--json"], env(&name, None)).await;
        assert_eq!(r.exit, EXIT_OK, "{r:?}");
        assert_eq!(r.json["ok"], json!(true));
        assert_eq!(r.json["result"]["ok"], json!(true));
        assert_eq!(r.json["result"]["authenticated"], json!(false));
        assert!(r.text.contains("läuft"), "{}", r.text);
    }

    #[tokio::test]
    async fn status_with_the_right_token_says_who_is_calling() {
        let f = Fixture::new();
        let name = unique_name();
        let _s = start(&f, &name).await;
        let r = ctl(&["status"], env(&name, Some(&f.token))).await;
        assert_eq!(r.exit, EXIT_OK);
        assert_eq!(r.json["result"]["client"]["label"], json!("Claude Code"));
        assert!(r.text.contains("Claude Code"));
    }

    #[tokio::test]
    async fn no_app_is_exit_2() {
        let r = ctl(&["status", "--json"], env(&unique_name(), None)).await;
        assert_eq!(r.exit, EXIT_NOT_RUNNING);
        assert_eq!(r.json["error"]["code"], json!("not_running"));
        assert!(r.stderr.contains("läuft nicht"));
    }

    #[tokio::test]
    async fn an_invalid_or_revoked_token_is_exit_4() {
        let f = Fixture::new();
        let name = unique_name();
        let _s = start(&f, &name).await;
        let bad = clients::generate_token();
        assert_eq!(ctl(&["status"], env(&name, Some(&bad))).await.exit, EXIT_AUTH);
        assert_eq!(ctl(&["tools"], env(&name, None)).await.exit, EXIT_AUTH);
        clients::revoke(&f.conn(), &f.client.id, f.now()).unwrap();
        let r = ctl(&["call", "create_meeting"], env(&name, Some(&f.token))).await;
        assert_eq!(r.exit, EXIT_AUTH);
        assert_eq!(r.json["error"]["code"], json!("token_revoked"));
        assert_eq!(f.calls.len(), 0);
    }

    #[tokio::test]
    async fn a_tool_that_is_off_is_exit_3() {
        let f = Fixture::new();
        let name = unique_name();
        let _s = start(&f, &name).await;
        let r = ctl(&["call", "transcribe_file", "--json"], env(&name, Some(&f.token))).await;
        assert_eq!(r.exit, EXIT_DENIED, "{r:?}");
        assert_eq!(r.json["error"]["code"], json!("tool_off"));
        assert_eq!(f.calls.len(), 0);
        // Es fehlt auch in der Liste.
        let list = ctl(&["tools", "--json"], env(&name, Some(&f.token))).await;
        assert_eq!(list.exit, EXIT_OK);
        let names: Vec<&str> = list.json["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(!names.contains(&"transcribe_file"));
    }

    #[tokio::test]
    async fn an_allowed_tool_runs_and_prints_its_result() {
        let f = Fixture::new();
        f.grant("create_meeting", GrantMode::Allow);
        let name = unique_name();
        let _s = start(&f, &name).await;
        let r = ctl(&["call", "create_meeting", "--args", r#"{"title":"T"}"#, "--json"], env(&name, Some(&f.token))).await;
        assert_eq!(r.exit, EXIT_OK, "{r:?}");
        assert_eq!(r.json["result"]["status"], json!("done"));
        assert_eq!(r.json["result"]["result"]["ran"], json!("create_meeting"));
        assert_eq!(f.calls.all()[0].1, json!({"title": "T"}));
        let tools = ctl(&["tools"], env(&name, Some(&f.token))).await;
        assert!(tools.text.contains("create_meeting"));
    }

    #[tokio::test]
    async fn ask_without_an_answer_is_exit_5_and_the_approval_can_be_followed_up() {
        let cfg = Config {
            approval_wait: Duration::from_millis(200),
            ..fast_config()
        };
        let f = Fixture::with(cfg, &ALL_TOOLS);
        f.grant("transcribe_file", GrantMode::Ask);
        let name = unique_name();
        let _s = start(&f, &name).await;
        let token = f.token.clone();

        let r = ctl(&["call", "transcribe_file", "--args", r#"{"path":"C:/a.wav"}"#, "--json"], env(&name, Some(&token))).await;
        assert_eq!(r.exit, EXIT_PENDING, "{r:?}");
        assert_eq!(r.json["ok"], json!(false));
        let id = r.json["result"]["approval_id"].as_str().unwrap().to_string();
        assert_eq!(f.calls.len(), 0);

        // Noch offen: Exit 5.
        let s = ctl(&["approval", &id, "--json"], env(&name, Some(&token))).await;
        assert_eq!(s.exit, EXIT_PENDING);
        // Der Nutzer gibt frei: Exit 0 ...
        f.user_decides(&id, true);
        let s = ctl(&["approval", &id], env(&name, Some(&token))).await;
        assert_eq!(s.exit, EXIT_OK);
        // ... und der Aufruf mit der Freigabe laeuft.
        let r = ctl(
            &["call", "transcribe_file", "--args", r#"{"path":"C:/a.wav"}"#, "--approval", &id, "--json"],
            env(&name, Some(&token)),
        )
        .await;
        assert_eq!(r.exit, EXIT_OK, "{r:?}");
        assert_eq!(f.calls.len(), 1);
    }

    #[tokio::test]
    async fn a_denied_approval_is_exit_3() {
        let cfg = Config {
            approval_wait: Duration::from_millis(100),
            ..fast_config()
        };
        let f = Fixture::with(cfg, &ALL_TOOLS);
        f.grant("transcribe_file", GrantMode::Ask);
        let name = unique_name();
        let _s = start(&f, &name).await;
        let r = ctl(&["call", "transcribe_file", "--args", r#"{"path":"C:/a.wav"}"#, "--json"], env(&name, Some(&f.token))).await;
        let id = r.json["result"]["approval_id"].as_str().unwrap().to_string();
        f.user_decides(&id, false);
        let s = ctl(&["approval", &id], env(&name, Some(&f.token))).await;
        assert_eq!(s.exit, EXIT_DENIED);
        let r = ctl(
            &["call", "transcribe_file", "--args", r#"{"path":"C:/a.wav"}"#, "--approval", &id],
            env(&name, Some(&f.token)),
        )
        .await;
        assert_eq!(r.exit, EXIT_DENIED);
        assert_eq!(f.calls.len(), 0);
    }

    #[tokio::test]
    async fn the_app_ending_mid_call_is_exit_2() {
        let cfg = Config {
            approval_wait: Duration::from_secs(10),
            ..fast_config()
        };
        let f = Fixture::with(cfg, &ALL_TOOLS);
        f.grant("transcribe_file", GrantMode::Ask);
        let name = unique_name();
        let server = start(&f, &name).await;
        let (n2, t2) = (name.clone(), f.token.clone());
        let call = tokio::task::spawn_blocking(move || {
            execute(&parse(&["call", "transcribe_file", "--args", r#"{"path":"C:/a.wav"}"#]), &CtlEnv {
                pipe_name: Ok(n2),
                token: Ok(Some(t2)),
                deadline: Duration::from_secs(20),
            })
        });
        // Warten, bis die Freigabe angelegt ist, dann endet die App.
        for _ in 0..200 {
            if f.count("SELECT COUNT(*) FROM approvals") == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        server.shutdown();
        let r = call.await.unwrap();
        assert_eq!(r.exit, EXIT_NOT_RUNNING, "{r:?}");
        assert_eq!(f.calls.len(), 0);
        // Der Audit-Eintrag der Anfrage bleibt sichtbar (pending).
        assert!(f.audit().iter().any(|a| a.outcome == "pending"));
    }

    #[tokio::test]
    async fn a_server_that_hangs_up_abruptly_is_exit_2_and_one_that_never_answers_times_out() {
        use tokio::net::windows::named_pipe::ServerOptions;
        // 1) Der Server nimmt an und legt sofort auf.
        let name = unique_name();
        let srv = ServerOptions::new().create(&name).unwrap();
        let hangup = tokio::spawn(async move {
            srv.connect().await.unwrap();
            drop(srv);
        });
        let r = ctl(&["status"], env(&name, None)).await;
        hangup.await.unwrap();
        assert_eq!(r.exit, EXIT_NOT_RUNNING, "{r:?}");

        // 2) Der Server nimmt an und antwortet nie: Gesamtfrist.
        let name = unique_name();
        let srv = ServerOptions::new().create(&name).unwrap();
        let silent = tokio::spawn(async move {
            srv.connect().await.unwrap();
            tokio::time::sleep(Duration::from_secs(3)).await;
            drop(srv);
        });
        let mut e = env(&name, None);
        e.deadline = Duration::from_millis(400);
        let r = ctl(&["status", "--json"], e).await;
        assert_eq!(r.exit, EXIT_ERROR);
        assert_eq!(r.json["error"]["code"], json!("timeout"));
        silent.abort();
    }
}

#[test]
fn finish_writes_the_output_file_in_the_chosen_form() {
    let dir = tempfile::tempdir().unwrap();
    let result = CtlResult::failure(EXIT_NOT_RUNNING, "not_running", "Local Voice AI läuft nicht.");
    // JSON
    let out = dir.path().join("ctl.json");
    let args = CtlArgs {
        json: true,
        out: Some(out.clone()),
        token_file: None,
        verb: Verb::Status,
    };
    assert_eq!(finish(&args, &result), EXIT_NOT_RUNNING);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(v["exit"], json!(2));
    assert_eq!(v["error"]["code"], json!("not_running"));
    // Text: der Fehlertext
    let out = dir.path().join("ctl.txt");
    let args = CtlArgs {
        json: false,
        out: Some(out.clone()),
        token_file: None,
        verb: Verb::Status,
    };
    finish(&args, &result);
    assert!(std::fs::read_to_string(&out).unwrap().contains("läuft nicht"));
    // Eine nicht schreibbare Datei aendert den Exit-Code nicht.
    let args = CtlArgs {
        json: true,
        out: Some(dir.path().join("fehlt").join("x.json")),
        token_file: None,
        verb: Verb::Status,
    };
    assert_eq!(finish(&args, &result), EXIT_NOT_RUNNING);
}
