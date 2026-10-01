//! Tests des Webhook-Ziels (B5): Adressregeln, Senden, Grenzen, Einordnung der Fehler.
//! Der „Webhook“ ist ein lokaler Server auf Loopback; kein Netz, keine Fenster.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use super::*;

// ---------------------------------------------------------------------------
// Test-Server
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Seen {
    head: String,
    body: String,
}

struct Server {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn url(&self, path: &str) -> url::Url {
        parse_url(&format!("{}{path}", self.base)).unwrap()
    }
}

/// Startet einen Server, der jede Verbindung mit `reply(anzahl)` beantwortet (Rohbytes der
/// Antwort). `None`: nie antworten (Verbindung offen lassen).
fn serve(reply: impl Fn(usize) -> Option<Vec<u8>> + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    std::thread::spawn(move || {
        for (n, stream) in listener.incoming().enumerate() {
            let Ok(mut s) = stream else { return };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let end = loop {
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break p;
                }
                match s.read(&mut tmp) {
                    Ok(0) | Err(_) => break buf.len(),
                    Ok(k) => buf.extend_from_slice(&tmp[..k]),
                }
            };
            let head = String::from_utf8_lossy(&buf[..end.min(buf.len())]).to_string();
            let len: usize = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse().ok())
                })
                .unwrap_or(0);
            let mut body = buf.get(end + 4..).unwrap_or_default().to_vec();
            while body.len() < len {
                match s.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(k) => body.extend_from_slice(&tmp[..k]),
                }
            }
            seen2.lock().unwrap().push(Seen {
                head,
                body: String::from_utf8_lossy(&body).to_string(),
            });
            match reply(n) {
                Some(bytes) => {
                    let _ = s.write_all(&bytes);
                    let _ = s.flush();
                }
                None => {
                    // Antwort schuldig bleiben: Verbindung eine Weile offen halten.
                    std::thread::sleep(Duration::from_secs(3));
                }
            }
        }
    });
    Server { base, seen }
}

fn http(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn fast() -> HttpOpts {
    HttpOpts {
        connect_timeout: Duration::from_millis(800),
        timeout: Duration::from_millis(900),
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

// ---------------------------------------------------------------------------
// Adresse
// ---------------------------------------------------------------------------

#[test]
fn only_https_and_loopback_http_are_accepted_and_the_rest_says_why() {
    assert!(parse_url("https://n8n.example.org/webhook/abc-123").is_ok());
    assert!(parse_url("http://127.0.0.1:5678/webhook/x").is_ok());
    assert!(parse_url("http://localhost:5678/webhook/x").is_ok());
    assert!(parse_url("http://[::1]:5678/webhook/x").is_ok());
    for bad in [
        "http://n8n.example.org/webhook/x",
        "ftp://n8n.example.org/x",
        "file:///C:/x",
        "https://user:pw@n8n.example.org/x",
        "https://n8n.example.org/ x",
        "n8n.example.org/webhook",
        "https://",
    ] {
        assert!(
            matches!(parse_url(bad), Err(WebhookError::Config(_))),
            "{bad}"
        );
    }
    assert_eq!(parse_url("  "), Err(WebhookError::UrlMissing));
    let long = format!("https://example.org/{}", "a".repeat(MAX_URL_CHARS));
    assert!(matches!(parse_url(&long), Err(WebhookError::Config(_))));
    // Ein Fragment gehoert nicht zur Anfrage.
    let u = parse_url("https://example.org/h#frag").unwrap();
    assert_eq!(u.fragment(), None);
}

#[test]
fn the_shown_host_never_carries_path_or_query() {
    let u = parse_url("https://n8n.example.org:8443/webhook/GEHEIM-1234?sig=abc").unwrap();
    let host = host_of(&u);
    assert_eq!(host, "n8n.example.org:8443");
    let cfg = config_for(&u).to_string();
    assert!(!cfg.contains("GEHEIM") && !cfg.contains("sig="), "{cfg}");
    assert_eq!(host_from_config(&cfg), "n8n.example.org:8443");
    assert_eq!(host_from_config("kaputt"), "");
}

// ---------------------------------------------------------------------------
// Senden
// ---------------------------------------------------------------------------

#[test]
fn a_post_carries_the_json_the_key_and_returns_the_reply() {
    let server = serve(|_| {
        Some(http(
            200,
            &[("Content-Type", "application/json")],
            br#"{"ok":true}"#,
        ))
    });
    let reply = post(
        &server.url("/webhook/abc"),
        &json!({"titel": "Überprüfung", "n": 3}),
        "RUN1:hook",
        &fast(),
    )
    .unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, r#"{"ok":true}"#);
    assert!(!reply.truncated);
    assert!(reply.content_type.contains("json"));
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    let head = seen[0].head.to_ascii_lowercase();
    assert!(head.starts_with("post /webhook/abc "), "{head}");
    assert!(head.contains("content-type: application/json"), "{head}");
    assert!(head.contains("idempotency-key: run1:hook"), "{head}");
    let sent: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(sent["titel"], "Überprüfung");
}

#[test]
fn a_huge_reply_is_cut_at_the_limit_and_still_counts_as_delivered() {
    let big = vec![b'x'; 5000];
    let server = serve(move |_| Some(http(200, &[], &big)));
    let opts = HttpOpts {
        max_response_bytes: 1000,
        ..fast()
    };
    let reply = post(&server.url("/h"), &json!({}), "k", &opts).unwrap();
    assert!(reply.truncated);
    assert_eq!(reply.bytes, 1000);
    assert_eq!(reply.body.len(), 1000);
}

#[test]
fn a_redirect_is_never_followed_and_is_a_rejection() {
    let target = serve(|_| Some(http(200, &[], b"da bin ich")));
    let location = format!("{}/geklaut", target.base);
    let origin = serve(move |_| Some(http(302, &[("Location", &location)], b"")));
    let err = post(&origin.url("/h"), &json!({"a": 1}), "k", &fast()).unwrap_err();
    assert_eq!(err, WebhookError::Redirect(302));
    assert_eq!(err.class(), Class::Rejected);
    assert_eq!(origin.seen().len(), 1);
    assert!(
        target.seen().is_empty(),
        "dem Ziel der Umleitung darf nie etwas gesendet werden"
    );
}

#[test]
fn status_codes_are_classified_by_what_they_say_about_delivery() {
    for (status, class) in [
        (404, Class::Rejected),
        (400, Class::Rejected),
        (401, Class::Rejected),
        (429, Class::NotSent),
        (408, Class::NotSent),
        (500, Class::Unknown),
        (503, Class::Unknown),
    ] {
        let server = serve(move |_| Some(http(status, &[], b"nein")));
        let err = post(&server.url("/h"), &json!({}), "k", &fast()).unwrap_err();
        assert_eq!(err, WebhookError::Status(status));
        assert_eq!(err.class(), class, "{status}");
    }
}

#[test]
fn a_server_that_never_answers_is_a_timeout_and_unknown() {
    let server = serve(|_| None);
    let err = post(&server.url("/h"), &json!({}), "k", &fast()).unwrap_err();
    assert_eq!(err, WebhookError::Timeout);
    assert_eq!(
        err.class(),
        Class::Unknown,
        "angekommen sein kann es trotzdem"
    );
}

#[test]
fn a_closed_port_is_not_sent() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let url = parse_url(&format!("http://127.0.0.1:{port}/webhook/GEHEIM-9")).unwrap();
    let err = post(&url, &json!({}), "k", &fast()).unwrap_err();
    assert!(matches!(err, WebhookError::Connect(_)), "{err:?}");
    assert_eq!(err.class(), Class::NotSent);
    let text = err.to_string();
    assert!(
        !text.contains("GEHEIM"),
        "der Pfad steht nie im Fehler: {text}"
    );
}

#[test]
fn a_request_over_the_limit_never_opens_a_connection() {
    let server = serve(|_| Some(http(200, &[], b"")));
    let big = json!({"text": "x".repeat(MAX_REQUEST_BYTES + 10)});
    let err = post(&server.url("/h"), &big, "k", &fast()).unwrap_err();
    assert!(matches!(err, WebhookError::TooLarge(_)), "{err:?}");
    assert_eq!(err.class(), Class::Rejected);
    assert!(server.seen().is_empty());
}

#[test]
fn error_texts_never_carry_the_secret_path() {
    let server = serve(|_| Some(http(500, &[], b"/webhook/GEHEIM-77?sig=zzz")));
    let url = server.url("/webhook/GEHEIM-77?sig=zzz");
    let err = post(&url, &json!({}), "k", &fast()).unwrap_err();
    let text = format!("{err} {err:?}");
    assert!(
        !text.contains("GEHEIM") && !text.contains("sig=zzz"),
        "{text}"
    );
}
