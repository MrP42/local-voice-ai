//! Tests einer Verbindung ueber `tokio::io::duplex` (ohne echte Pipe): Protokoll,
//! Anmeldung, Wartezeit auf Freigaben, Abbruch, Grenzen.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{
    duplex, split, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf,
};
use tokio::task::JoinHandle;

use super::*;
use crate::agent_bridge::clients;
use crate::agent_bridge::testkit::{fast_config, Fixture, ALL_TOOLS};
use crate::agent_bridge::Config;
use crate::managers::integrations::approvals;
use crate::managers::integrations::model::GrantMode;

struct TestClient {
    rd: BufReader<ReadHalf<DuplexStream>>,
    wr: WriteHalf<DuplexStream>,
    next: u64,
}

impl TestClient {
    async fn send_raw(&mut self, raw: &str) {
        self.wr.write_all(raw.as_bytes()).await.unwrap();
        self.wr.write_all(b"\n").await.unwrap();
        self.wr.flush().await.unwrap();
    }

    /// Eine Zeile lesen; `None` bei Verbindungsende.
    async fn read(&mut self) -> Option<Value> {
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(10), self.rd.read_line(&mut line))
            .await
            .expect("Antwort innerhalb von 10 s")
            .unwrap();
        (n > 0).then(|| serde_json::from_str(line.trim()).unwrap())
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send_raw(&json!({"id": id, "method": method, "params": params}).to_string())
            .await;
        let v = self.read().await.expect("Antwort");
        assert_eq!(v["id"], json!(id), "die Antwort traegt die Kennung der Anfrage");
        v
    }

    async fn hello(&mut self, token: &str) -> Value {
        self.request("hello", json!({"token": token})).await
    }

    async fn call(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": args})).await
    }

    /// Ist die Verbindung beendet?
    async fn closed(&mut self) -> bool {
        self.read().await.is_none()
    }
}

fn error_code(v: &Value) -> String {
    v["error"]["code"].as_str().unwrap_or("(kein Fehler)").to_string()
}

fn start(f: &Fixture) -> (Arc<Server>, TestClient, JoinHandle<()>) {
    let server = Server::new(f.bridge.clone());
    let (client, handle) = connect(&server);
    (server, client, handle)
}

fn connect(server: &Arc<Server>) -> (TestClient, JoinHandle<()>) {
    let (a, b) = duplex(4 * 1024 * 1024);
    let handle = tokio::spawn(server.clone().handle(a));
    let (rd, wr) = split(b);
    (
        TestClient {
            rd: BufReader::new(rd),
            wr,
            next: 0,
        },
        handle,
    )
}

/// Der Nutzer entscheidet in der App, sobald eine Freigabe offen ist (wartet bis zu 10 s darauf,
/// damit der Test auch auf einem ausgelasteten Rechner nicht vom Zeitablauf abhaengt).
fn decide_when_pending(path: std::path::PathBuf, now: i64, approve: bool) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(&path).unwrap();
        for _ in 0..1000 {
            let pending: Option<String> = conn
                .query_row("SELECT id FROM approvals WHERE state = 'pending'", [], |r| r.get(0))
                .ok();
            if let Some(id) = pending {
                std::thread::sleep(Duration::from_millis(30));
                approvals::decide(&conn, &id, approve, now).unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("keine offene Freigabe entstanden");
    })
}

/// Wartet, bis eine offene Freigabe in der Datenbank steht, und liefert ihre Kennung.
async fn pending_id(f: &Fixture) -> String {
    for _ in 0..1000 {
        if let Ok(id) = f
            .conn()
            .query_row("SELECT id FROM approvals WHERE state = 'pending'", [], |r| r.get::<_, String>(0))
        {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("keine offene Freigabe entstanden");
}

// --- Anmeldung -----------------------------------------------------------------

#[tokio::test]
async fn hello_with_a_valid_token_logs_in() {
    let f = Fixture::new();
    let (_s, mut c, _) = start(&f);
    let v = c.hello(&f.token).await;
    assert_eq!(v["result"]["authenticated"], json!(true));
    assert_eq!(v["result"]["client"]["label"], json!("Claude Code"));
    assert_eq!(v["result"]["protocol"], json!(1));
}

#[tokio::test]
async fn without_a_token_only_status_works() {
    let f = Fixture::new();
    let (_s, mut c, _) = start(&f);
    let v = c.request("status", json!({})).await;
    assert_eq!(v["result"]["ok"], json!(true));
    assert_eq!(v["result"]["authenticated"], json!(false));
    for m in ["tools/list", "tools/call", "approval/status"] {
        let v = c.request(m, json!({"name": "create_meeting"})).await;
        assert_eq!(error_code(&v), "unauthenticated", "{m}");
    }
    // Auch nach einem hello ohne Token bleibt die Verbindung anonym.
    let v = c.request("hello", json!({})).await;
    assert_eq!(v["result"]["authenticated"], json!(false));
    assert_eq!(error_code(&c.request("tools/list", json!({})).await), "unauthenticated");
}

#[tokio::test]
async fn an_invalid_token_is_refused_and_three_failures_close_the_connection() {
    let f = Fixture::new();
    let (_s, mut c, handle) = start(&f);
    let bad = clients::generate_token();
    assert_eq!(error_code(&c.hello(&bad).await), "token_invalid");
    assert_eq!(error_code(&c.hello(&bad).await), "token_invalid");
    // Dazwischen ein richtiger Versuch setzt den Zaehler zurueck.
    assert_eq!(c.hello(&f.token).await["result"]["authenticated"], json!(true));
    assert_eq!(error_code(&c.hello(&bad).await), "token_invalid");
    assert_eq!(error_code(&c.hello(&bad).await), "token_invalid");
    assert_eq!(error_code(&c.hello(&bad).await), "token_invalid");
    // Sofort zu (nicht erst durch die Leerlauf-Frist von 5 s).
    let closed = tokio::time::timeout(Duration::from_secs(1), c.closed()).await;
    assert_eq!(closed, Ok(true), "nach drei Fehlversuchen ist Schluss");
    handle.await.unwrap();
}

#[tokio::test]
async fn a_failed_login_drops_an_earlier_login_of_the_same_connection() {
    let f = Fixture::new();
    let (_s, mut c, _) = start(&f);
    assert_eq!(c.hello(&f.token).await["result"]["authenticated"], json!(true));
    assert_eq!(error_code(&c.hello(&clients::generate_token()).await), "token_invalid");
    assert_eq!(error_code(&c.request("tools/list", json!({})).await), "unauthenticated");
}

#[tokio::test]
async fn a_revoked_token_is_refused() {
    let f = Fixture::new();
    clients::revoke(&f.conn(), &f.client.id, f.now()).unwrap();
    let (_s, mut c, _) = start(&f);
    assert_eq!(error_code(&c.hello(&f.token).await), "token_revoked");
}

#[tokio::test]
async fn a_revoked_client_is_cut_off_mid_connection() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    assert!(c.call("create_meeting", json!({})).await["result"].is_object());
    clients::revoke(&f.conn(), &f.client.id, f.now()).unwrap();
    assert_eq!(error_code(&c.call("create_meeting", json!({})).await), "token_revoked");
    assert_eq!(f.calls.len(), 1, "nach dem Zurueckziehen lief nichts mehr");
    // Auch nicht, wenn der Agent es weiter versucht.
    assert_eq!(error_code(&c.request("tools/list", json!({})).await), "unauthenticated");
}

// --- Liste und Aufruf -------------------------------------------------------------

#[tokio::test]
async fn tools_list_shows_only_what_the_client_may_use() {
    let f = Fixture::new();
    f.grant("transcribe_file", GrantMode::Ask);
    f.grant("create_meeting", GrantMode::Off);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    let v = c.request("tools/list", json!({})).await;
    let names: Vec<&str> = v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"transcribe_file"));
    assert!(names.contains(&"get_action_status"));
    assert!(!names.contains(&"create_meeting"), "aus: fehlt in tools/list");
}

#[tokio::test]
async fn an_allowed_call_answers_done_with_the_result() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    let v = c.call("create_meeting", json!({"title": "T"})).await;
    assert_eq!(v["result"]["status"], json!("done"));
    assert_eq!(v["result"]["result"]["ran"], json!("create_meeting"));
}

#[tokio::test]
async fn a_call_to_a_tool_that_is_off_is_an_error_with_a_stable_code() {
    let f = Fixture::new();
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    let v = c.call("transcribe_file", json!({"path": "C:/a.wav"})).await;
    assert_eq!(error_code(&v), "tool_off");
    assert!(v["error"]["message"].as_str().unwrap().len() > 10);
    assert_eq!(f.calls.len(), 0);
    let v = c.call("gibt_es_nicht", json!({})).await;
    assert_eq!(error_code(&v), "unknown_tool");
}

#[tokio::test]
async fn ask_approved_within_the_wait_runs_the_tool_and_answers_in_the_same_reply() {
    let cfg = Config {
        approval_wait: Duration::from_secs(10),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    f.grant("transcribe_file", GrantMode::Ask);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    // Der Nutzer entscheidet in der App, waehrend der Agent wartet.
    let user = decide_when_pending(f.fx.db_path.clone(), f.now(), true);
    let v = c.call("transcribe_file", json!({"path": "C:/a.wav"})).await;
    user.await.unwrap();
    assert_eq!(v["result"]["status"], json!("done"), "{v}");
    assert_eq!(f.calls.len(), 1);
    assert!(f.calls.all()[0].2, "mit eingeloester Freigabe ausgefuehrt");
}

#[tokio::test]
async fn ask_denied_within_the_wait_answers_denied_and_never_runs() {
    let cfg = Config {
        approval_wait: Duration::from_secs(10),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    f.grant("transcribe_file", GrantMode::Ask);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    let user = decide_when_pending(f.fx.db_path.clone(), f.now(), false);
    let v = c.call("transcribe_file", json!({"path": "C:/a.wav"})).await;
    user.await.unwrap();
    assert_eq!(error_code(&v), "approval_denied");
    assert_eq!(f.calls.len(), 0);
}

#[tokio::test]
async fn thirty_seconds_without_an_answer_give_pending_with_an_id_that_can_be_asked_later() {
    // Die 30 s sind hier auf 250 ms verkuerzt; die Vorgabe `Config::default()` ist 30 s.
    assert_eq!(Config::default().approval_wait, Duration::from_secs(30));
    let cfg = Config {
        approval_wait: Duration::from_millis(250),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    f.grant("transcribe_file", GrantMode::Ask);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    let args = json!({"path": "C:/a.wav"});
    let started = std::time::Instant::now();
    let v = c.call("transcribe_file", args.clone()).await;
    assert!(started.elapsed() >= Duration::from_millis(240), "gewartet wurde");
    assert_eq!(v["result"]["status"], json!("pending"), "{v}");
    let id = v["result"]["approval_id"].as_str().unwrap().to_string();
    assert_eq!(f.calls.len(), 0);

    // Spaeter abfragbar: noch offen ...
    let s = c.request("approval/status", json!({"approval_id": id})).await;
    assert_eq!(s["result"]["state"], json!("pending"));
    // ... der Nutzer gibt frei ...
    f.user_decides(&id, true);
    let s = c.request("approval/status", json!({"approval_id": id})).await;
    assert_eq!(s["result"]["state"], json!("approved"));
    // ... und der Agent holt die Ausfuehrung mit derselben approval_id nach.
    let v = c
        .request("tools/call", json!({"name": "transcribe_file", "arguments": args, "approval_id": id}))
        .await;
    assert_eq!(v["result"]["status"], json!("done"), "{v}");
    assert_eq!(f.calls.len(), 1);
    let s = c.request("approval/status", json!({"approval_id": id})).await;
    assert_eq!(s["result"]["state"], json!("used"));
}

#[tokio::test]
async fn the_status_of_an_approval_can_also_be_asked_through_the_status_tool() {
    let cfg = Config {
        approval_wait: Duration::from_millis(100),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    f.grant("transcribe_file", GrantMode::Ask);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    let v = c.call("transcribe_file", json!({"path": "C:/a.wav"})).await;
    let id = v["result"]["approval_id"].as_str().unwrap().to_string();
    let v = c.call("get_action_status", json!({"approval_id": id})).await;
    assert_eq!(v["result"]["result"]["state"], json!("pending"));
}

#[tokio::test]
async fn a_client_that_leaves_while_waiting_never_runs_the_tool() {
    let cfg = Config {
        approval_wait: Duration::from_secs(5),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    f.grant("transcribe_file", GrantMode::Ask);
    let (_s, mut c, handle) = start(&f);
    c.hello(&f.token).await;
    c.send_raw(
        &json!({"id": 99, "method": "tools/call", "params": {"name": "transcribe_file", "arguments": {"path": "C:/a.wav"}}})
            .to_string(),
    )
    .await;
    // Warten, bis die Freigabe angelegt ist, dann geht der Agent weg.
    let id = pending_id(&f).await;
    drop(c);
    // Der Server bemerkt das Ende und gibt die Verbindung auf, lange vor den 5 s.
    tokio::time::timeout(Duration::from_secs(3), handle)
        .await
        .expect("die Verbindung endet, ohne die ganze Wartezeit abzuwarten")
        .unwrap();
    // Der Nutzer genehmigt danach: es passiert nichts, die Freigabe bleibt unverbraucht.
    f.user_decides(&id, true);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(f.calls.len(), 0, "kein Agent mehr da: nichts ausgefuehrt");
    let state: String = f
        .conn()
        .query_row("SELECT state FROM approvals WHERE id = ?1", [&id], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "approved");
}

#[tokio::test]
async fn there_is_no_way_to_decide_an_approval_over_the_pipe() {
    let cfg = Config {
        approval_wait: Duration::from_millis(80),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    f.grant("transcribe_file", GrantMode::Ask);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    let v = c.call("transcribe_file", json!({"path": "C:/a.wav"})).await;
    let id = v["result"]["approval_id"].as_str().unwrap().to_string();
    for method in ["approval/decide", "approval/approve", "approvals/decide", "grants/set", "clients/create", "approval_decide"] {
        let v = c.request(method, json!({"approval_id": id, "approve": true, "id": id})).await;
        assert_eq!(error_code(&v), "unknown_method", "{method}");
    }
    // Als Werkzeug gibt es es ebenfalls nicht.
    for tool in ["approval_decide", "approve", "set_grant", "client_create"] {
        let v = c.call(tool, json!({"approval_id": id, "approve": true})).await;
        assert_eq!(error_code(&v), "unknown_tool", "{tool}");
    }
    // Zusatzfelder bei der Statusabfrage aendern nichts.
    let s = c.request("approval/status", json!({"approval_id": id, "approve": true, "state": "approved"})).await;
    assert_eq!(s["result"]["state"], json!("pending"));
    let state: String = f
        .conn()
        .query_row("SELECT state FROM approvals WHERE id = ?1", [&id], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "pending");
    assert_eq!(f.calls.len(), 0);
}

// --- Fehlerhafte Eingaben ------------------------------------------------------------

#[tokio::test]
async fn malformed_requests_get_errors_and_the_connection_stays_usable() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    for raw in ["das ist kein json", "[1,2,3]", r#"{"id":1}"#, r#"{"id":1,"method":"tools/call","params":[1]}"#] {
        c.send_raw(raw).await;
        let v = c.read().await.unwrap();
        assert_eq!(error_code(&v), "bad_request", "{raw}");
    }
    assert_eq!(error_code(&c.request("tools/call", json!({})).await), "bad_request");
    assert_eq!(error_code(&c.request("tools/call", json!({"name": 5})).await), "bad_request");
    assert_eq!(
        error_code(&c.request("tools/call", json!({"name": "create_meeting", "arguments": "x"})).await),
        "bad_request"
    );
    assert_eq!(
        error_code(&c.request("tools/call", json!({"name": "create_meeting", "approval_id": 7})).await),
        "bad_request"
    );
    assert_eq!(error_code(&c.request("nonsense", json!({})).await), "unknown_method");
    assert_eq!(error_code(&c.request("approval/status", json!({})).await), "bad_request");
    // Leerzeilen werden ueberlesen, danach geht alles weiter.
    c.send_raw("").await;
    assert_eq!(c.call("create_meeting", json!({})).await["result"]["status"], json!("done"));
}

#[tokio::test]
async fn invalid_utf8_gets_an_error_and_the_connection_goes_on() {
    let f = Fixture::new();
    let (_s, mut c, _) = start(&f);
    c.wr.write_all(b"\xff\xfe\n").await.unwrap();
    assert_eq!(error_code(&c.read().await.unwrap()), "bad_request");
    assert_eq!(c.request("status", json!({})).await["result"]["ok"], json!(true));
}

#[tokio::test]
async fn an_overlong_line_is_answered_and_the_connection_is_closed() {
    let f = Fixture::new();
    let (_s, mut c, handle) = start(&f);
    let big = "x".repeat(MAX_LINE_BYTES + 100);
    let writer = tokio::spawn(async move {
        // Das Schreiben kann scheitern, sobald der Server die Verbindung schliesst.
        let _ = c.wr.write_all(big.as_bytes()).await;
        let _ = c.wr.write_all(b"\n").await;
        c
    });
    let mut c = writer.await.unwrap();
    let v = c.read().await.unwrap();
    assert_eq!(error_code(&v), "line_too_long");
    assert!(c.closed().await);
    handle.await.unwrap();
}

// --- Fristen und Grenzen ----------------------------------------------------------------

#[tokio::test]
async fn a_connection_that_says_nothing_is_closed_after_the_handshake_timeout() {
    let cfg = Config {
        handshake_timeout: Duration::from_millis(300),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    let (s, mut c, handle) = start(&f);
    let started = std::time::Instant::now();
    assert!(c.closed().await);
    assert!(started.elapsed() < Duration::from_secs(3));
    handle.await.unwrap();
    assert_eq!(s.free_slots(), 8, "der Platz ist wieder frei");
}

#[tokio::test]
async fn an_anonymous_connection_that_goes_idle_is_closed() {
    let cfg = Config {
        anonymous_idle_timeout: Duration::from_millis(300),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    let (_s, mut c, handle) = start(&f);
    assert_eq!(c.request("status", json!({})).await["result"]["ok"], json!(true));
    assert!(c.closed().await, "anonym und still: nach der kurzen Frist zu");
    handle.await.unwrap();
}

#[tokio::test]
async fn the_number_of_connections_is_capped_and_a_free_slot_is_reused() {
    let cfg = Config {
        max_connections: 2,
        handshake_timeout: Duration::from_secs(5),
        anonymous_idle_timeout: Duration::from_secs(5),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    let server = Server::new(f.bridge.clone());
    let (mut a, _ha) = connect(&server);
    let (mut b, _hb) = connect(&server);
    assert_eq!(a.request("status", json!({})).await["result"]["ok"], json!(true));
    assert_eq!(b.request("status", json!({})).await["result"]["ok"], json!(true));
    // Die dritte wird abgewiesen: Fehler, dann Ende.
    let (mut c3, h3) = connect(&server);
    let v = c3.read().await.unwrap();
    assert_eq!(error_code(&v), "too_many_connections");
    assert!(c3.closed().await);
    h3.await.unwrap();
    // Eine geht, eine neue passt wieder hinein.
    drop(a);
    for _ in 0..100 {
        if server.free_slots() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (mut d, _hd) = connect(&server);
    assert_eq!(d.request("status", json!({})).await["result"]["ok"], json!(true));
}

#[tokio::test]
async fn calls_are_limited_per_minute_per_client() {
    let cfg = Config {
        calls_per_minute: 2,
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    f.grant("create_meeting", GrantMode::Allow);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    assert_eq!(c.call("create_meeting", json!({})).await["result"]["status"], json!("done"));
    assert_eq!(c.call("create_meeting", json!({})).await["result"]["status"], json!("done"));
    assert_eq!(error_code(&c.call("create_meeting", json!({})).await), "rate_limited");
    assert_eq!(f.calls.len(), 2);
    // tools/list und status zaehlen nicht als Aufruf.
    assert!(c.request("tools/list", json!({})).await["result"].is_object());
}

#[tokio::test]
async fn an_anonymous_status_flood_is_limited() {
    let cfg = Config {
        requests_per_minute: 3,
        anonymous_idle_timeout: Duration::from_secs(5),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    let (_s, mut c, _) = start(&f);
    for _ in 0..3 {
        assert_eq!(c.request("status", json!({})).await["result"]["ok"], json!(true));
    }
    assert_eq!(error_code(&c.request("status", json!({})).await), "rate_limited");
}

#[tokio::test]
async fn shutdown_ends_waiting_connections_without_running_anything() {
    let cfg = Config {
        approval_wait: Duration::from_secs(5),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    f.grant("transcribe_file", GrantMode::Ask);
    let (s, mut c, handle) = start(&f);
    c.hello(&f.token).await;
    c.send_raw(
        &json!({"id": 5, "method": "tools/call", "params": {"name": "transcribe_file", "arguments": {"path": "C:/a.wav"}}})
            .to_string(),
    )
    .await;
    pending_id(&f).await;
    s.shutdown();
    tokio::time::timeout(Duration::from_secs(3), handle).await.unwrap().unwrap();
    assert_eq!(f.calls.len(), 0);
    // Neue Verbindungen werden abgewiesen.
    let (mut late, _) = connect(&s);
    assert_eq!(error_code(&late.read().await.unwrap()), "shutting_down");
}

#[tokio::test]
async fn an_idle_authenticated_connection_survives_longer_than_an_anonymous_one() {
    let cfg = Config {
        anonymous_idle_timeout: Duration::from_millis(300),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    let (_s, mut c, _) = start(&f);
    c.hello(&f.token).await;
    tokio::time::sleep(Duration::from_millis(500)).await; // laenger als die anonyme Frist (300 ms)
    assert_eq!(c.request("status", json!({})).await["result"]["authenticated"], json!(true));
}

#[tokio::test]
async fn two_connections_of_one_client_work_side_by_side() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    let server = Server::new(f.bridge.clone());
    let (mut a, _) = connect(&server);
    let (mut b, _) = connect(&server);
    a.hello(&f.token).await;
    b.hello(&f.token).await;
    let (ra, rb) = tokio::join!(a.call("create_meeting", json!({"n": 1})), b.call("create_meeting", json!({"n": 2})));
    assert_eq!(ra["result"]["status"], json!("done"));
    assert_eq!(rb["result"]["status"], json!("done"));
    assert_eq!(f.calls.len(), 2);
}

#[tokio::test]
async fn two_connections_asking_the_same_get_one_approval_and_it_runs_once() {
    let cfg = Config {
        approval_wait: Duration::from_secs(5),
        ..fast_config()
    };
    let f = Fixture::with(cfg, &ALL_TOOLS);
    f.grant("transcribe_file", GrantMode::Ask);
    let server = Server::new(f.bridge.clone());
    let (mut a, _) = connect(&server);
    let (mut b, _) = connect(&server);
    a.hello(&f.token).await;
    b.hello(&f.token).await;
    let args = json!({"path": "C:/a.wav"});
    let msg = |id: u64| {
        json!({"id": id, "method": "tools/call", "params": {"name": "transcribe_file", "arguments": args}}).to_string()
    };
    a.send_raw(&msg(1)).await;
    b.send_raw(&msg(2)).await;
    let id = pending_id(&f).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(f.count("SELECT COUNT(*) FROM approvals"), 1, "eine gemeinsame Freigabe");
    f.user_decides(&id, true);
    let (ra, rb) = tokio::join!(a.read(), b.read());
    let ok = [ra.unwrap(), rb.unwrap()]
        .iter()
        .filter(|v| v["result"]["status"] == json!("done"))
        .count();
    assert_eq!(ok, 1, "die Genehmigung gilt fuer genau eine Ausfuehrung");
    assert_eq!(f.calls.len(), 1);
}
