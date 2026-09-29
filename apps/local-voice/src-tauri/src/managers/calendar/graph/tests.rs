//! Tests fuer `calendar::graph`. Kein Netz: Token-Endpunkt und Graph sind ein lokaler
//! Test-HTTP-Server (`serve`), der Browser ist ein Roh-TCP-Aufruf an den Listener.

use super::*;
use crate::managers::calendar::model::CalendarKind;
use chrono::TimeZone;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Hilfen
// ---------------------------------------------------------------------------

const CLIENT_ID: &str = "11111111-2222-3333-4444-555555555555";

fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    chrono::Utc
        .with_ymd_and_hms(y, mo, d, h, mi, 0)
        .unwrap()
        .timestamp_millis()
}

fn eps(base: &str) -> Endpoints {
    Endpoints {
        authority: base.to_string(),
        graph: format!("{base}/v1.0"),
        use_env_proxy: false,
    }
}

#[derive(Clone, Debug)]
struct Req {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Req {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
    fn form(&self) -> HashMap<String, String> {
        url::form_urlencoded::parse(self.body.as_bytes())
            .into_owned()
            .collect()
    }
    fn origin(&self) -> String {
        format!("http://{}", self.header("host").unwrap())
    }
}

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn json(status: u16, v: Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: v.to_string(),
        }
    }
    fn with(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_string(), v.to_string()));
        self
    }
}

type Seen = Arc<Mutex<Vec<Req>>>;

/// Lokaler HTTP-Server: jede Verbindung ist eine Anfrage; `handler` antwortet.
async fn serve(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> (String, Seen) {
    let handler = Arc::new(handler);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let handler = handler.clone();
            let seen = seen2.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let end = loop {
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break p;
                    }
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                };
                let head = String::from_utf8_lossy(&buf[..end]).to_string();
                let mut lines = head.split("\r\n");
                let mut first = lines.next().unwrap().split(' ');
                let method = first.next().unwrap().to_string();
                let target = first.next().unwrap().to_string();
                let mut headers = HashMap::new();
                for l in lines {
                    if let Some((k, v)) = l.split_once(':') {
                        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                let len: usize = headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let mut body = buf[end + 4..].to_vec();
                while body.len() < len {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => body.extend_from_slice(&tmp[..n]),
                    }
                }
                let req = Req {
                    method,
                    target,
                    headers,
                    body: String::from_utf8_lossy(&body).to_string(),
                };
                seen.lock().unwrap().push(req.clone());
                let resp = handler(&req);
                let mut out = format!(
                    "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n",
                    resp.status,
                    resp.body.len()
                );
                for (k, v) in &resp.headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                out.push_str(&resp.body);
                let _ = sock.write_all(out.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (base, seen)
}

fn count(seen: &Seen, prefix: &str) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|r| r.target.starts_with(prefix))
        .count()
}

/// Ein Browser ohne Browser: ein Roh-GET an den Listener.
async fn raw_request(port: u16, method: &str, target: &str, host: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(
        format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out).await;
    out
}

async fn raw_get(port: u16, target: &str, host: &str) -> String {
    raw_request(port, "GET", target, host).await
}

type WaitHandle = tokio::task::JoinHandle<Result<Zeroizing<String>, GraphError>>;

/// Startet einen Listener und wartet im Hintergrund auf den Code.
async fn listen(state: &'static str, timeout: Duration) -> (u16, Arc<Notify>, WaitHandle) {
    let lb = Loopback::bind().await.unwrap();
    let port = lb.port();
    let cancel = Arc::new(Notify::new());
    let c2 = cancel.clone();
    let handle = tokio::spawn(async move { lb.wait_for_code(state, timeout, &c2).await });
    (port, cancel, handle)
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(120)).await;
}

#[derive(Default)]
struct MemVault {
    map: Mutex<HashMap<String, Vec<u8>>>,
    fail_put: AtomicBool,
    puts: AtomicUsize,
}

impl MemVault {
    fn with_account(name: &str, refresh: &str) -> Self {
        let v = Self::default();
        save_account(
            &v,
            name,
            &StoredAccount::new(CLIENT_ID.into(), "common".into(), refresh.into()),
        )
        .unwrap();
        v.puts.store(0, Ordering::SeqCst);
        v
    }
    fn refresh_token(&self, name: &str) -> Option<String> {
        load_account(self, name)
            .ok()
            .map(|a| a.refresh_token().to_string())
    }
}

impl SecretVault for MemVault {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(name)
            .map(|b| Zeroizing::new(b.clone())))
    }
    fn put(&self, name: &str, data: &[u8]) -> Result<(), String> {
        if self.fail_put.load(Ordering::SeqCst) {
            return Err("Geheimnis nicht speicherbar: Der Datenträger ist voll.".to_string());
        }
        self.puts.fetch_add(1, Ordering::SeqCst);
        self.map
            .lock()
            .unwrap()
            .insert(name.to_string(), data.to_vec());
        Ok(())
    }
    fn delete(&self, name: &str) {
        self.map.lock().unwrap().remove(name);
    }
}

fn tokens_json(access: &str, refresh: Option<&str>) -> Value {
    let mut v = json!({"token_type": "Bearer", "expires_in": 3599, "access_token": access});
    if let Some(r) = refresh {
        v["refresh_token"] = json!(r);
    }
    v
}

fn event_json(ical: &str, subject: &str, start: &str, end: &str) -> Value {
    json!({
        "id": format!("AAMk-{ical}"),
        "iCalUId": ical,
        "subject": subject,
        "start": {"dateTime": start, "timeZone": "UTC"},
        "end": {"dateTime": end, "timeZone": "UTC"},
        "isAllDay": false,
        "isCancelled": false,
        "attendees": [],
        "organizer": null,
    })
}

fn store() -> MeetingStore {
    let dir = tempfile::tempdir().unwrap();
    let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    std::mem::forget(dir);
    s
}

// ---------------------------------------------------------------------------
// PKCE, Adresse, Eingaben
// ---------------------------------------------------------------------------

#[test]
fn pkce_challenge_matches_the_rfc_7636_vector() {
    // RFC 7636, Anhang B.
    assert_eq!(
        challenge_s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn verifier_and_state_are_random_url_safe_and_long_enough() {
    let (a, b) = (new_verifier(), new_verifier());
    assert_ne!(a.as_str(), b.as_str());
    for s in [a.as_str(), new_state().as_str(), new_state().as_str()] {
        assert!(s.len() >= 43 && s.len() <= 128, "{}", s.len());
        assert!(s
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
    }
    assert_ne!(new_state(), new_state());
}

#[test]
fn authorize_url_carries_pkce_state_and_the_read_only_scopes() {
    let ep = Endpoints::production();
    let url = authorize_url(
        &ep,
        "contoso.com",
        CLIENT_ID,
        "http://localhost:51234",
        "ST",
        "CH",
    )
    .unwrap();
    let u = url::Url::parse(&url).unwrap();
    assert_eq!(u.host_str(), Some("login.microsoftonline.com"));
    assert_eq!(u.path(), "/contoso.com/oauth2/v2.0/authorize");
    let q: HashMap<_, _> = u.query_pairs().into_owned().collect();
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["redirect_uri"], "http://localhost:51234");
    assert_eq!(q["state"], "ST");
    assert_eq!(q["code_challenge"], "CH");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["response_mode"], "query");
    assert_eq!(q["scope"], "Calendars.Read offline_access User.Read");
    assert!(!q.contains_key("client_secret"));
}

#[test]
fn validation_refuses_everything_that_could_change_the_address() {
    assert_eq!(
        validate_client_id(&format!("  {}  ", CLIENT_ID.to_uppercase())).unwrap(),
        CLIENT_ID
    );
    for bad in [
        "",
        "abc",
        "../x",
        "11111111-2222-3333-4444-55555555555",
        "11111111-2222-3333-4444-5555555555556",
        "11111111_2222_3333_4444_555555555555",
        "1111111g-2222-3333-4444-555555555555",
        "11111111-2222-3333-4444-555555555555/../x",
    ] {
        assert!(validate_client_id(bad).is_err(), "{bad:?}");
    }
    assert_eq!(validate_tenant("").unwrap(), "common");
    assert_eq!(validate_tenant(" Contoso.COM ").unwrap(), "contoso.com");
    assert_eq!(validate_tenant(CLIENT_ID).unwrap(), CLIENT_ID);
    for bad in [
        "../x",
        "a/b",
        "a b",
        "a?b=c",
        "a#b",
        "a@b",
        "a\\b",
        ".a",
        "a.",
        "-a",
        "a..b",
        &"x".repeat(129),
    ] {
        assert!(validate_tenant(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn settings_values_are_normalised() {
    assert_eq!(normalize_client_id_setting(None).unwrap(), None);
    assert_eq!(normalize_client_id_setting(Some("  ")).unwrap(), None);
    assert_eq!(
        normalize_client_id_setting(Some(&CLIENT_ID.to_uppercase())).unwrap(),
        Some(CLIENT_ID.to_string())
    );
    assert!(normalize_client_id_setting(Some("nicht-gueltig")).is_err());
    assert_eq!(normalize_tenant_setting(None).unwrap(), None);
    assert_eq!(normalize_tenant_setting(Some("common")).unwrap(), None);
    assert_eq!(normalize_tenant_setting(Some(" COMMON ")).unwrap(), None);
    assert_eq!(
        normalize_tenant_setting(Some("contoso.com")).unwrap(),
        Some("contoso.com".to_string())
    );
    assert!(normalize_tenant_setting(Some("a/b")).is_err());
}

#[test]
fn retry_after_forms_are_parsed_and_capped() {
    let now = chrono::Utc.with_ymd_and_hms(2026, 9, 29, 10, 0, 0).unwrap();
    assert_eq!(parse_retry_after(Some("7"), now), Duration::from_secs(7));
    assert_eq!(
        parse_retry_after(Some(" 120 "), now),
        Duration::from_secs(120)
    );
    assert_eq!(
        parse_retry_after(Some("Tue, 29 Sep 2026 10:00:30 GMT"), now),
        Duration::from_secs(30)
    );
    assert_eq!(parse_retry_after(None, now), DEFAULT_RETRY_AFTER);
    assert_eq!(parse_retry_after(Some("bald"), now), DEFAULT_RETRY_AFTER);
    assert_eq!(parse_retry_after(Some("0"), now), Duration::from_secs(1));
    assert_eq!(parse_retry_after(Some("999999"), now), MAX_RETRY_AFTER);
    // Ein Datum in der Vergangenheit heisst „sofort“, nicht „nie“.
    assert_eq!(
        parse_retry_after(Some("Tue, 29 Sep 2026 09:00:00 GMT"), now),
        Duration::from_secs(1)
    );
}

// ---------------------------------------------------------------------------
// Loopback
// ---------------------------------------------------------------------------

#[tokio::test]
async fn loopback_accepts_the_matching_state_and_returns_the_code() {
    for host in ["127.0.0.1", "localhost"] {
        let (port, _cancel, handle) = listen("STATE-1", Duration::from_secs(10)).await;
        let response = raw_get(
            port,
            "/?code=the-code&state=STATE-1",
            &format!("{host}:{port}"),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("Anmeldung abgeschlossen"));
        let code = handle.await.unwrap().unwrap();
        assert_eq!(code.as_str(), "the-code");
    }
}

#[tokio::test]
async fn loopback_binds_only_the_ipv4_loopback_and_uses_localhost_in_the_redirect() {
    let lb = Loopback::bind().await.unwrap();
    let addr = lb.listener.local_addr().unwrap();
    assert!(addr.ip().is_loopback() && addr.is_ipv4(), "{addr}");
    assert_eq!(lb.redirect_uri(), format!("http://localhost:{}", lb.port()));
    assert_ne!(lb.port(), 0);
}

#[tokio::test]
async fn loopback_ignores_a_wrong_or_missing_state_and_keeps_waiting() {
    let (port, _cancel, handle) = listen("RIGHT", Duration::from_secs(10)).await;
    let host = format!("127.0.0.1:{port}");
    for target in [
        "/?code=evil&state=WRONG",
        "/?code=evil",
        "/?code=evil&state=",
        "/?code=evil&state=RIGH",
        "/?code=evil&state=RIGHTT",
        "/?state=RIGHT",
        "/?state=RIGHT&code=",
    ] {
        let response = raw_get(port, target, &host).await;
        assert!(response.starts_with("HTTP/1.1 400"), "{target}: {response}");
    }
    settle().await;
    assert!(
        !handle.is_finished(),
        "ein falscher state beendet das Warten nicht"
    );
    let ok = raw_get(port, "/?code=real&state=RIGHT", &host).await;
    assert!(ok.starts_with("HTTP/1.1 200"));
    assert_eq!(handle.await.unwrap().unwrap().as_str(), "real");
}

#[tokio::test]
async fn loopback_rejects_a_foreign_host_a_wrong_path_and_a_wrong_method() {
    let (port, _cancel, handle) = listen("S", Duration::from_secs(10)).await;
    let good_host = format!("127.0.0.1:{port}");
    let query = "/?code=evil&state=S";
    // DNS-Rebinding: ein fremder Name zeigt auf 127.0.0.1.
    for host in [
        "evil.example".to_string(),
        format!("evil.example:{port}"),
        format!("127.0.0.1:{}", port.wrapping_add(1)),
        format!("localhost.evil.example:{port}"),
    ] {
        let r = raw_get(port, query, &host).await;
        assert!(r.starts_with("HTTP/1.1 400"), "{host}: {r}");
    }
    let r = raw_get(port, "/favicon.ico", &good_host).await;
    assert!(r.starts_with("HTTP/1.1 404"), "{r}");
    let r = raw_get(port, "/callback?code=evil&state=S", &good_host).await;
    assert!(r.starts_with("HTTP/1.1 404"), "{r}");
    let r = raw_request(port, "POST", query, &good_host).await;
    assert!(r.starts_with("HTTP/1.1 405"), "{r}");
    settle().await;
    assert!(!handle.is_finished());
    raw_get(port, "/?code=real&state=S", &good_host).await;
    assert_eq!(handle.await.unwrap().unwrap().as_str(), "real");
}

#[tokio::test]
async fn loopback_reports_a_provider_error_only_with_the_right_state() {
    // Ohne richtigen state kann niemand die Anmeldung abbrechen.
    let (port, _cancel, handle) = listen("S", Duration::from_secs(10)).await;
    let host = format!("127.0.0.1:{port}");
    raw_get(port, "/?error=access_denied&state=WRONG", &host).await;
    settle().await;
    assert!(!handle.is_finished());
    let r = raw_get(
        port,
        "/?error=access_denied&error_description=AADSTS65004%3A+Der+Benutzer+lehnte+ab.%0D%0ATrace+ID%3A+abc&state=S",
        &host,
    )
    .await;
    assert!(r.starts_with("HTTP/1.1 200"));
    let err = handle.await.unwrap().unwrap_err();
    match &err {
        GraphError::Denied(m) => {
            assert_eq!(m, "AADSTS65004: Der Benutzer lehnte ab.");
            assert!(!m.contains("Trace ID"));
        }
        other => panic!("{other:?}"),
    }
    // Ohne Beschreibung: fester Klartext.
    let (port, _c, handle) = listen("S", Duration::from_secs(10)).await;
    raw_get(
        port,
        "/?error=access_denied&state=S",
        &format!("127.0.0.1:{port}"),
    )
    .await;
    let err = handle.await.unwrap().unwrap_err();
    assert!(
        err.to_string().contains("abgelehnt oder abgebrochen"),
        "{err}"
    );
}

#[tokio::test]
async fn loopback_times_out_and_frees_the_port() {
    let (port, _cancel, handle) = listen("S", Duration::from_millis(150)).await;
    let err = handle.await.unwrap().unwrap_err();
    assert_eq!(err, GraphError::SignInTimeout);
    assert!(err.to_string().contains("5 Minuten"));
    // Der Port ist wieder frei.
    TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("der Listener ist geschlossen");
}

#[tokio::test]
async fn loopback_cancel_ends_the_wait_and_frees_the_port() {
    let (port, cancel, handle) = listen("S", Duration::from_secs(30)).await;
    settle().await;
    cancel.notify_one();
    let err = tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("Abbruch wirkt sofort")
        .unwrap()
        .unwrap_err();
    assert_eq!(err, GraphError::Cancelled);
    TcpListener::bind(("127.0.0.1", port)).await.unwrap();
}

#[tokio::test]
async fn loopback_survives_an_idle_preconnect_of_the_browser() {
    let (port, _cancel, handle) = listen("S", Duration::from_secs(10)).await;
    // Browser oeffnen gern Verbindungen im Voraus und schicken nichts.
    let _idle1 = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let _idle2 = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let started = Instant::now();
    raw_get(port, "/?code=c&state=S", &format!("127.0.0.1:{port}")).await;
    let code = tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("haengende Verbindungen blockieren den Listener nicht")
        .unwrap()
        .unwrap();
    assert_eq!(code.as_str(), "c");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn loopback_answers_never_reflect_input_and_forbid_caching() {
    let (port, _cancel, handle) = listen("S", Duration::from_secs(10)).await;
    let host = format!("127.0.0.1:{port}");
    let xss = "%3Cscript%3Ealert(1)%3C%2Fscript%3E";
    let r1 = raw_get(port, &format!("/?code={xss}&state={xss}"), &host).await;
    assert!(!r1.contains("<script") && !r1.contains("alert"), "{r1}");
    let r2 = raw_get(
        port,
        &format!("/?error=x&error_description={xss}&state=S"),
        &host,
    )
    .await;
    assert!(!r2.contains("<script") && !r2.contains("alert"), "{r2}");
    let lower = r2.to_ascii_lowercase();
    assert!(lower.contains("cache-control: no-store"));
    assert!(lower.contains("referrer-policy: no-referrer"));
    assert!(lower.contains("content-security-policy: default-src 'none'"));
    let _ = handle.await;
}

#[tokio::test]
async fn loopback_rejects_oversized_request_heads() {
    let (port, _cancel, handle) = listen("S", Duration::from_secs(10)).await;
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let junk = format!(
        "GET /?state=S HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX: {}\r\n\r\n",
        "a".repeat(20_000)
    );
    let _ = s.write_all(junk.as_bytes()).await;
    let mut out = String::new();
    let _ = s.read_to_string(&mut out).await;
    assert!(
        out.is_empty(),
        "kein Antwort auf einen Riesenkopf: {out:.60}"
    );
    settle().await;
    assert!(!handle.is_finished());
    raw_get(port, "/?code=c&state=S", &format!("127.0.0.1:{port}")).await;
    assert!(handle.await.unwrap().is_ok());
}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exchange_code_posts_the_pkce_form_and_parses_the_tokens() {
    let (base, seen) = serve(|_| Resp::json(200, tokens_json("AT", Some("RT")))).await;
    let t = exchange_code(
        &eps(&base),
        CLIENT_ID,
        "contoso.com",
        "the-code",
        "the-verifier",
        "http://localhost:5555",
    )
    .await
    .unwrap();
    assert_eq!(t.access_token.as_str(), "AT");
    assert_eq!(t.refresh_token.as_deref().map(String::as_str), Some("RT"));
    assert_eq!(t.expires_in, Duration::from_secs(3599));
    let reqs = seen.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.target, "/contoso.com/oauth2/v2.0/token");
    assert_eq!(
        r.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    let f = r.form();
    assert_eq!(f["grant_type"], "authorization_code");
    assert_eq!(f["client_id"], CLIENT_ID);
    assert_eq!(f["code"], "the-code");
    assert_eq!(f["code_verifier"], "the-verifier");
    assert_eq!(f["redirect_uri"], "http://localhost:5555");
    assert_eq!(f["scope"], "Calendars.Read offline_access User.Read");
    assert!(!f.contains_key("client_secret"), "oeffentlicher Client");
}

#[tokio::test]
async fn token_errors_are_short_and_never_echo_the_body() {
    let desc = "AADSTS70008: Der Code ist abgelaufen.\r\nTrace ID: 0000\r\nCorrelation ID: 1111\r\nTimestamp: 2026-09-29 10:00:00Z";
    let (base, _) = serve(move |_| {
        Resp::json(
            400,
            json!({"error": "invalid_grant", "error_description": desc, "access_token": "GEHEIM"}),
        )
    })
    .await;
    let err = exchange_code(
        &eps(&base),
        CLIENT_ID,
        "common",
        "c",
        "v",
        "http://localhost:1",
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        GraphError::Denied("AADSTS70008: Der Code ist abgelaufen.".to_string())
    );
    let shown = err.to_string();
    assert!(
        !shown.contains("Trace") && !shown.contains("GEHEIM"),
        "{shown}"
    );

    // 5xx und 429.
    let (base, _) = serve(|_| Resp::json(503, json!({}))).await;
    assert_eq!(
        exchange_code(&eps(&base), CLIENT_ID, "common", "c", "v", "u")
            .await
            .unwrap_err(),
        GraphError::Http(503)
    );
    let (base, _) = serve(|_| Resp::json(429, json!({})).with("Retry-After", "12")).await;
    assert_eq!(
        exchange_code(&eps(&base), CLIENT_ID, "common", "c", "v", "u")
            .await
            .unwrap_err(),
        GraphError::Throttled {
            retry_after: Duration::from_secs(12)
        }
    );
}

#[tokio::test]
async fn a_response_without_an_access_token_is_a_parse_error() {
    let (base, _) = serve(|_| Resp::json(200, json!({"token_type": "Bearer"}))).await;
    let err = refresh_tokens(&eps(&base), CLIENT_ID, "common", "rt")
        .await
        .unwrap_err();
    assert!(matches!(err, GraphError::Parse(_)), "{err:?}");
}

#[tokio::test]
async fn refresh_rotates_the_token_stores_it_and_the_cache_saves_the_next_call() {
    let (base, seen) = serve(|_| Resp::json(200, tokens_json("AT-NEW", Some("RT-NEW")))).await;
    let ep = eps(&base);
    let vault = MemVault::with_account("graph-1", "RT-OLD");
    let cache = TokenCache::default();

    let t = access_token(&ep, &vault, &cache, "graph-1", false)
        .await
        .unwrap();
    assert_eq!(t.as_str(), "AT-NEW");
    let form = seen.lock().unwrap()[0].form();
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["refresh_token"], "RT-OLD");
    assert_eq!(form["client_id"], CLIENT_ID);
    assert_eq!(vault.refresh_token("graph-1").as_deref(), Some("RT-NEW"));

    // Zweiter Aufruf: aus dem Cache, kein Netz.
    let again = access_token(&ep, &vault, &cache, "graph-1", false)
        .await
        .unwrap();
    assert_eq!(again.as_str(), "AT-NEW");
    assert_eq!(count(&seen, "/"), 1);

    // Erzwungene Erneuerung nimmt das NEUE Token.
    access_token(&ep, &vault, &cache, "graph-1", true)
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap()[1].form()["refresh_token"], "RT-NEW");
}

#[tokio::test]
async fn refresh_keeps_the_stored_token_when_microsoft_sends_none() {
    let (base, _) = serve(|_| Resp::json(200, tokens_json("AT", None))).await;
    let vault = MemVault::with_account("graph-1", "RT-OLD");
    access_token(
        &eps(&base),
        &vault,
        &TokenCache::default(),
        "graph-1",
        false,
    )
    .await
    .unwrap();
    assert_eq!(vault.refresh_token("graph-1").as_deref(), Some("RT-OLD"));
    assert_eq!(
        vault.puts.load(Ordering::SeqCst),
        0,
        "nichts neu geschrieben"
    );
}

#[tokio::test]
async fn refresh_uses_the_client_id_and_tenant_stored_with_the_account() {
    let (base, seen) = serve(|_| Resp::json(200, tokens_json("AT", None))).await;
    let vault = MemVault::default();
    save_account(
        &vault,
        "graph-1",
        &StoredAccount::new(CLIENT_ID.into(), "contoso.com".into(), "RT".into()),
    )
    .unwrap();
    access_token(
        &eps(&base),
        &vault,
        &TokenCache::default(),
        "graph-1",
        false,
    )
    .await
    .unwrap();
    let r = seen.lock().unwrap()[0].clone();
    assert_eq!(r.target, "/contoso.com/oauth2/v2.0/token");
    assert_eq!(r.form()["client_id"], CLIENT_ID);
}

#[tokio::test]
async fn refresh_with_invalid_grant_needs_a_new_sign_in() {
    for error in ["invalid_grant", "interaction_required", "consent_required"] {
        let (base, _) = serve(move |_| {
            Resp::json(
                400,
                json!({"error": error, "error_description": "AADSTS700082: abgelaufen"}),
            )
        })
        .await;
        let vault = MemVault::with_account("graph-1", "RT");
        let cache = TokenCache::default();
        cache.put(
            "graph-1",
            Zeroizing::new("ALT".into()),
            Duration::from_secs(3600),
        );
        let err = access_token(&eps(&base), &vault, &cache, "graph-1", true)
            .await
            .unwrap_err();
        assert_eq!(err, GraphError::NeedsSignIn, "{error}");
        assert!(cache.get("graph-1").is_none(), "der Cache wird geleert");
        assert!(err.to_string().starts_with("Anmeldung nötig"));
    }
}

#[tokio::test]
async fn a_missing_or_corrupt_secret_needs_sign_in_without_calling_the_server() {
    let (base, seen) = serve(|_| Resp::json(200, tokens_json("AT", None))).await;
    let ep = eps(&base);
    let empty = MemVault::default();
    assert_eq!(
        access_token(&ep, &empty, &TokenCache::default(), "graph-1", false)
            .await
            .unwrap_err(),
        GraphError::NeedsSignIn
    );
    for bad in [
        b"kein json".to_vec(),
        br#"{"v":2,"client_id":"a","tenant":"b","refresh_token":"c"}"#.to_vec(),
        br#"{"v":1,"client_id":"a","tenant":"b","refresh_token":""}"#.to_vec(),
        br#"{"v":1}"#.to_vec(),
    ] {
        let vault = MemVault::default();
        vault.put("graph-1", &bad).unwrap();
        assert_eq!(
            access_token(&ep, &vault, &TokenCache::default(), "graph-1", false)
                .await
                .unwrap_err(),
            GraphError::NeedsSignIn
        );
    }
    assert_eq!(count(&seen, "/"), 0);
}

#[tokio::test]
async fn a_failed_token_write_keeps_the_old_token_and_the_run_going() {
    let (base, _) = serve(|_| Resp::json(200, tokens_json("AT-NEW", Some("RT-NEW")))).await;
    let vault = MemVault::with_account("graph-1", "RT-OLD");
    vault.fail_put.store(true, Ordering::SeqCst);
    let t = access_token(
        &eps(&base),
        &vault,
        &TokenCache::default(),
        "graph-1",
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        t.as_str(),
        "AT-NEW",
        "der Lauf geht mit dem Zugriffstoken weiter"
    );
    assert_eq!(
        vault.refresh_token("graph-1").as_deref(),
        Some("RT-OLD"),
        "das alte Erneuerungs-Token bleibt unbeschaedigt"
    );
}

#[test]
fn the_token_cache_expires_and_forgets() {
    let cache = TokenCache::default();
    cache.put("a", Zeroizing::new("T".into()), Duration::from_secs(3600));
    assert_eq!(cache.get("a").as_deref().map(String::as_str), Some("T"));
    // Kurz vor Ablauf (innerhalb der Sicherheitsspanne) gilt es nicht mehr.
    cache.put("b", Zeroizing::new("T".into()), Duration::from_secs(60));
    assert!(cache.get("b").is_none());
    cache.clear("a");
    assert!(cache.get("a").is_none());
}

// ---------------------------------------------------------------------------
// Abruf: 401, 429, Paging, Zeiten
// ---------------------------------------------------------------------------

const FROM: i64 = 1_790_000_000_000;
const TO: i64 = FROM + 30 * 86_400_000;

#[tokio::test]
async fn calendar_view_401_needs_a_sign_in() {
    let (base, _) = serve(|_| {
        Resp::json(
            401,
            json!({"error": {"code": "InvalidAuthenticationToken"}}),
        )
    })
    .await;
    let err = fetch_events(&eps(&base), "AT", "graph-1", FROM, TO, None)
        .await
        .unwrap_err();
    assert_eq!(err, GraphError::NeedsSignIn);
    assert!(err.to_string().starts_with("Anmeldung nötig"));
}

#[tokio::test]
async fn a_401_refreshes_once_and_retries_with_the_new_token() {
    let (base, seen) = serve(|r| {
        if r.target.starts_with("/common/oauth2/v2.0/token") {
            return Resp::json(200, tokens_json("AT-2", Some("RT-2")));
        }
        match r.header("authorization") {
            Some("Bearer AT-2") => Resp::json(
                200,
                json!({"value": [event_json("U1", "Nach Erneuerung", "2026-09-29T09:00:00.0000000", "2026-09-29T10:00:00.0000000")]}),
            ),
            _ => Resp::json(401, json!({})),
        }
    })
    .await;
    let vault = MemVault::with_account("graph-1", "RT-1");
    let cache = TokenCache::default();
    cache.put(
        "graph-1",
        Zeroizing::new("AT-1".into()),
        Duration::from_secs(3600),
    );
    let got = sync_events(&eps(&base), &vault, &cache, "graph-1", FROM, TO, None)
        .await
        .unwrap();
    assert_eq!(got.events.len(), 1);
    assert_eq!(got.events[0].title, "Nach Erneuerung");
    assert_eq!(count(&seen, "/v1.0/me/calendarView"), 2);
    assert_eq!(count(&seen, "/common/oauth2/v2.0/token"), 1);
    assert_eq!(vault.refresh_token("graph-1").as_deref(), Some("RT-2"));
}

#[tokio::test]
async fn a_second_401_after_the_refresh_needs_a_sign_in() {
    let (base, seen) = serve(|r| {
        if r.target.starts_with("/common/oauth2/v2.0/token") {
            Resp::json(200, tokens_json("AT-2", None))
        } else {
            Resp::json(401, json!({}))
        }
    })
    .await;
    let vault = MemVault::with_account("graph-1", "RT-1");
    let cache = TokenCache::default();
    let err = sync_events(&eps(&base), &vault, &cache, "graph-1", FROM, TO, None)
        .await
        .unwrap_err();
    assert_eq!(err, GraphError::NeedsSignIn);
    // Erstes Token holen, 401, erneuern, 401 - dann Schluss (keine Schleife).
    assert_eq!(count(&seen, "/v1.0/me/calendarView"), 2);
    assert_eq!(count(&seen, "/common/oauth2/v2.0/token"), 2);
    assert!(cache.get("graph-1").is_none());
}

#[tokio::test]
async fn a_refresh_that_fails_during_a_401_retry_needs_a_sign_in() {
    let (base, _) = serve(|r| {
        if r.target.starts_with("/common/oauth2/v2.0/token") {
            let n = r.form().get("refresh_token").cloned().unwrap_or_default();
            if n == "RT-1" {
                return Resp::json(400, json!({"error": "invalid_grant"}));
            }
        }
        Resp::json(401, json!({}))
    })
    .await;
    let vault = MemVault::with_account("graph-1", "RT-1");
    let cache = TokenCache::default();
    cache.put(
        "graph-1",
        Zeroizing::new("AT-1".into()),
        Duration::from_secs(3600),
    );
    assert_eq!(
        sync_events(&eps(&base), &vault, &cache, "graph-1", FROM, TO, None)
            .await
            .unwrap_err(),
        GraphError::NeedsSignIn
    );
}

#[tokio::test]
async fn throttling_429_reports_retry_after_and_stops() {
    let (base, seen) = serve(|_| Resp::json(429, json!({})).with("Retry-After", "7")).await;
    let err = fetch_events(&eps(&base), "AT", "graph-1", FROM, TO, None)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        GraphError::Throttled {
            retry_after: Duration::from_secs(7)
        }
    );
    assert!(err.to_string().contains("HTTP 429"));
    assert!(err.to_string().contains("1 min"));
    assert_eq!(seen.lock().unwrap().len(), 1, "kein Wiederholen im Sturm");

    let (base, _) = serve(|_| Resp::json(429, json!({}))).await;
    assert_eq!(
        fetch_events(&eps(&base), "AT", "graph-1", FROM, TO, None)
            .await
            .unwrap_err(),
        GraphError::Throttled {
            retry_after: DEFAULT_RETRY_AFTER
        }
    );
}

#[test]
fn graph_state_remembers_a_throttled_source_until_the_time_is_up() {
    let state = GraphState::default();
    assert_eq!(state.throttle_left("graph-1"), None);
    state.throttle("graph-1", Duration::from_millis(80));
    let left = state.throttle_left("graph-1").expect("noch gedrosselt");
    assert!(left <= Duration::from_millis(80));
    assert_eq!(state.throttle_left("graph-2"), None, "je Quelle");
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(state.throttle_left("graph-1"), None);
    state.throttle("graph-1", Duration::from_secs(60));
    state.forget("graph-1");
    assert_eq!(state.throttle_left("graph-1"), None);
}

#[tokio::test]
async fn other_statuses_are_reported_with_a_clear_text() {
    for (status, expect) in [
        (403u16, "Calendars.Read"),
        (500, "HTTP 500"),
        (404, "HTTP 404"),
    ] {
        let (base, _) =
            serve(move |_| Resp::json(status, json!({"error": {"message": "GEHEIM"}}))).await;
        let err = fetch_events(&eps(&base), "AT", "graph-1", FROM, TO, None)
            .await
            .unwrap_err();
        let shown = err.to_string();
        assert!(shown.contains(expect), "{status}: {shown}");
        assert!(
            !shown.contains("GEHEIM"),
            "der Koerper wird nicht wiedergegeben"
        );
    }
}

#[tokio::test]
async fn a_redirect_is_not_followed_so_the_token_stays_here() {
    let (other, other_seen) = serve(|_| Resp::json(200, json!({"value": []}))).await;
    let target = format!("{other}/steal");
    let (base, _) = serve(move |_| Resp::json(302, json!({})).with("Location", &target)).await;
    let err = fetch_events(&eps(&base), "AT", "graph-1", FROM, TO, None)
        .await
        .unwrap_err();
    assert_eq!(err, GraphError::Http(302));
    assert!(
        other_seen.lock().unwrap().is_empty(),
        "das Token ging nicht weiter"
    );
}

#[tokio::test]
async fn next_link_paging_collects_every_page_with_the_same_headers() {
    let (base, seen) = serve(|r| {
        let origin = r.origin();
        let page = |uid: &str, next: Option<String>| {
            let mut v = json!({"value": [event_json(
                uid,
                &format!("Termin {uid}"),
                if uid == "U3" { "2026-09-30T09:00:00.0000000" } else if uid == "U2" { "2026-09-29T11:00:00.0000000" } else { "2026-09-29T09:00:00.0000000" },
                "2026-10-01T00:00:00.0000000",
            )]});
            if let Some(n) = next {
                v["@odata.nextLink"] = json!(n);
            }
            Resp::json(200, v)
        };
        if r.target.contains("skiptoken=P3") {
            page("U3", None)
        } else if r.target.contains("skiptoken=P2") {
            page("U2", Some(format!("{origin}/v1.0/me/calendarView?%24skiptoken=P3")))
        } else {
            page("U1", Some(format!("{origin}/v1.0/me/calendarView?%24skiptoken=P2")))
        }
    })
    .await;
    let got = fetch_events(
        &eps(&base),
        "AT-X",
        "graph-1",
        FROM,
        TO,
        Some("ich@firma.de"),
    )
    .await
    .unwrap();
    assert_eq!(
        got.events
            .iter()
            .map(|e| e.title.as_str())
            .collect::<Vec<_>>(),
        vec!["Termin U1", "Termin U2", "Termin U3"],
        "alle Seiten, nach Beginn sortiert"
    );
    let reqs = seen.lock().unwrap();
    assert_eq!(reqs.len(), 3);
    let first = &reqs[0].target;
    assert!(
        first.starts_with("/v1.0/me/calendarView?startDateTime="),
        "{first}"
    );
    assert!(first.contains("&endDateTime="));
    assert!(first.contains("$top=100"), "{first}");
    assert!(first.contains("$select=id%2CiCalUId%2Csubject"), "{first}");
    for r in reqs.iter() {
        assert_eq!(r.header("authorization"), Some("Bearer AT-X"));
        assert_eq!(r.header("prefer"), Some("outlook.timezone=\"UTC\""));
        assert_eq!(r.header("accept"), Some("application/json"));
    }
    // Fenster als ISO in UTC.
    assert!(first.contains(&format!(
        "startDateTime={}",
        iso_utc(FROM).replace(':', "%3A")
    )));
}

#[tokio::test]
async fn a_next_link_to_a_foreign_host_is_refused() {
    for evil in [
        "http://evil.example/steal".to_string(),
        "https://127.0.0.1/v1.0/me/calendarView".to_string(),
        "not a url".to_string(),
    ] {
        let evil2 = evil.clone();
        let (base, seen) =
            serve(move |_| Resp::json(200, json!({"value": [], "@odata.nextLink": evil2}))).await;
        let err = fetch_events(&eps(&base), "AT", "graph-1", FROM, TO, None)
            .await
            .unwrap_err();
        assert!(matches!(err, GraphError::Parse(_)), "{evil}: {err:?}");
        assert_eq!(seen.lock().unwrap().len(), 1, "{evil}: kein zweiter Abruf");
    }
    // Anderer Port desselben Hosts zaehlt als fremd.
    let (base, _) = serve(|r| {
        let host = r.header("host").unwrap().replace(':', "1:");
        Resp::json(
            200,
            json!({"value": [], "@odata.nextLink": format!("http://{host}/x")}),
        )
    })
    .await;
    assert!(fetch_events(&eps(&base), "AT", "g", FROM, TO, None)
        .await
        .is_err());
}

#[tokio::test]
async fn too_many_pages_is_an_error_not_a_partial_result() {
    let (base, seen) = serve(|r| {
        Resp::json(
            200,
            json!({"value": [], "@odata.nextLink": format!("{}/v1.0/me/calendarView?p=n", r.origin())}),
        )
    })
    .await;
    let err = fetch_events(&eps(&base), "AT", "graph-1", FROM, TO, None)
        .await
        .unwrap_err();
    assert!(matches!(err, GraphError::Parse(_)), "{err:?}");
    assert_eq!(seen.lock().unwrap().len(), MAX_PAGES);
}

#[tokio::test]
async fn an_oversized_page_is_refused() {
    let (base, _) = serve(|_| {
        Resp::json(
            200,
            json!({"value": [], "pad": "x".repeat(MAX_PAGE_BYTES + 10)}),
        )
    })
    .await;
    let err = fetch_events(&eps(&base), "AT", "graph-1", FROM, TO, None)
        .await
        .unwrap_err();
    assert!(matches!(err, GraphError::Parse(_)), "{err:?}");
}

#[tokio::test]
async fn fetch_me_prefers_mail_then_the_principal_name() {
    let (base, seen) = serve(|_| {
        Resp::json(
            200,
            json!({"displayName": "Anna Berg", "mail": "Anna.Berg@Firma.DE", "userPrincipalName": "abc@firma.onmicrosoft.com"}),
        )
    })
    .await;
    let me = fetch_me(&eps(&base), "AT").await.unwrap();
    assert_eq!(
        me,
        Me {
            address: "anna.berg@firma.de".into(),
            name: Some("Anna Berg".into())
        }
    );
    assert!(seen.lock().unwrap()[0]
        .target
        .starts_with("/v1.0/me?$select=displayName,mail,userPrincipalName"));

    let (base, _) = serve(|_| {
        Resp::json(200, json!({"displayName": null, "mail": null, "userPrincipalName": "abc@firma.onmicrosoft.com"}))
    })
    .await;
    let me = fetch_me(&eps(&base), "AT").await.unwrap();
    assert_eq!(me.address, "abc@firma.onmicrosoft.com");
    assert_eq!(me.name, None);

    let (base, _) = serve(|_| {
        Resp::json(
            200,
            json!({"mail": null, "userPrincipalName": "keine-adresse"}),
        )
    })
    .await;
    assert!(matches!(
        fetch_me(&eps(&base), "AT").await.unwrap_err(),
        GraphError::Parse(_)
    ));
}

// ---------------------------------------------------------------------------
// Termine lesen
// ---------------------------------------------------------------------------

#[test]
fn times_are_read_as_utc_with_and_without_fraction() {
    let e = parse_event(
        &event_json(
            "U1",
            "T",
            "2026-09-29T09:30:00.0000000",
            "2026-09-29T10:00:00",
        ),
        "graph-1",
        None,
    )
    .unwrap();
    assert_eq!(e.starts_at, ms(2026, 9, 29, 9, 30));
    assert_eq!(e.ends_at, ms(2026, 9, 29, 10, 0));
    assert_eq!(e.key, format!("graph-1:U1:{}", e.starts_at));
    let z = parse_event(
        &event_json("U1", "T", "2026-09-29T09:30:00Z", "2026-09-29T10:00:00.5Z"),
        "graph-1",
        None,
    )
    .unwrap();
    assert_eq!(z.starts_at, ms(2026, 9, 29, 9, 30));
    // Ende vor Beginn wird auf den Beginn gehoben.
    let odd = parse_event(
        &event_json("U1", "T", "2026-09-29T09:30:00", "2026-09-29T09:00:00"),
        "graph-1",
        None,
    )
    .unwrap();
    assert_eq!(odd.ends_at, odd.starts_at);
}

#[test]
fn a_non_utc_time_is_skipped_with_a_clear_warning_not_guessed() {
    let mut item = event_json(
        "U1",
        "Berliner Termin",
        "2026-09-29T09:30:00",
        "2026-09-29T10:00:00",
    );
    item["start"]["timeZone"] = json!("W. Europe Standard Time");
    let err = parse_event(&item, "graph-1", None).unwrap_err();
    assert!(
        err.contains("Berliner Termin") && err.contains("W. Europe Standard Time"),
        "{err}"
    );
    let mut bad = event_json("U1", "T", "gestern", "2026-09-29T10:00:00");
    assert!(parse_event(&bad, "graph-1", None).is_err());
    bad = event_json("U1", "T", "2026-09-29T09:30:00", "2026-09-29T10:00:00");
    bad.as_object_mut().unwrap().remove("end");
    assert!(parse_event(&bad, "graph-1", None).is_err());
}

#[tokio::test]
async fn skipped_events_are_counted_and_the_rest_still_arrives() {
    let mut odd = event_json(
        "U2",
        "Andere Zone",
        "2026-09-29T11:00:00",
        "2026-09-29T12:00:00",
    );
    odd["start"]["timeZone"] = json!("Pacific Standard Time");
    let (base, _) = serve(move |_| {
        Resp::json(
            200,
            json!({"value": [
                event_json("U1", "Gut", "2026-09-29T09:00:00", "2026-09-29T10:00:00"),
                odd.clone(),
                {"subject": "Ohne Kennung", "start": {"dateTime": "2026-09-29T09:00:00", "timeZone": "UTC"}, "end": {"dateTime": "2026-09-29T09:30:00", "timeZone": "UTC"}},
            ]}),
        )
    })
    .await;
    let got = fetch_events(&eps(&base), "AT", "graph-1", FROM, TO, None)
        .await
        .unwrap();
    assert_eq!(got.events.len(), 1);
    assert_eq!(got.warnings.len(), 2, "{:?}", got.warnings);
}

#[test]
fn is_cancelled_and_all_day_are_carried() {
    let mut item = event_json(
        "U1",
        "Abgesagt: Jour fixe",
        "2026-09-29T09:00:00",
        "2026-09-29T10:00:00",
    );
    item["isCancelled"] = json!(true);
    let e = parse_event(&item, "graph-1", None).unwrap();
    assert!(e.cancelled && !e.all_day);
    assert_eq!(e.title, "Abgesagt: Jour fixe");

    let mut day = event_json("U2", "Urlaub", "2026-09-28T22:00:00", "2026-09-29T22:00:00");
    day["isAllDay"] = json!(true);
    let e = parse_event(&day, "graph-1", None).unwrap();
    assert!(e.all_day && !e.cancelled);
    assert_eq!(e.ends_at - e.starts_at, 24 * 3_600_000);
}

#[test]
fn attendees_organizer_first_rooms_skipped_self_flagged_and_partstat_mapped() {
    let mut item = event_json(
        "U1",
        "Jour fixe",
        "2026-09-29T09:00:00",
        "2026-09-29T10:00:00",
    );
    item["organizer"] =
        json!({"emailAddress": {"name": "Anna Berg", "address": "Anna.Berg@Firma.de"}});
    item["attendees"] = json!([
        {"type": "required", "status": {"response": "organizer"}, "emailAddress": {"name": "Anna Berg", "address": "anna.berg@firma.de"}},
        {"type": "required", "status": {"response": "accepted"}, "emailAddress": {"name": "Ich Selbst", "address": "ICH@firma.de"}},
        {"type": "optional", "status": {"response": "tentativelyAccepted"}, "emailAddress": {"name": "Bernd Alt", "address": "bernd@extern.de"}},
        {"type": "required", "status": {"response": "declined"}, "emailAddress": {"name": "Clara", "address": "clara@extern.de"}},
        {"type": "required", "status": {"response": "notResponded"}, "emailAddress": {"name": "Dora", "address": "dora@extern.de"}},
        {"type": "resource", "status": {"response": "accepted"}, "emailAddress": {"name": "Raum 3.14", "address": "raum314@firma.de"}},
        {"type": "required", "status": {"response": "none"}, "emailAddress": {"name": "Nur Name", "address": "/O=EXCHANGELABS/OU=X/CN=RECIPIENTS/CN=abc"}},
        {"type": "required", "emailAddress": {"name": "", "address": ""}},
        {"type": "required", "emailAddress": {"name": "bernd@extern.de", "address": "bernd@extern.de"}},
    ]);
    let e = parse_event(&item, "graph-1", Some("ich@firma.de")).unwrap();
    type Row<'a> = (
        Option<&'a str>,
        Option<&'a str>,
        bool,
        bool,
        Option<&'a str>,
    );
    let rows: Vec<Row<'_>> = e
        .attendees
        .iter()
        .map(|a| {
            (
                a.email.as_deref(),
                a.name.as_deref(),
                a.organizer,
                a.is_self,
                a.partstat.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (Some("anna.berg@firma.de"), Some("Anna Berg"), true, false, None),
            (Some("ich@firma.de"), Some("Ich Selbst"), false, true, Some("ACCEPTED")),
            (Some("bernd@extern.de"), Some("Bernd Alt"), false, false, Some("TENTATIVE")),
            (Some("clara@extern.de"), Some("Clara"), false, false, Some("DECLINED")),
            (Some("dora@extern.de"), Some("Dora"), false, false, Some("NEEDS-ACTION")),
            (None, Some("Nur Name"), false, false, None),
        ],
        "Organisator zuerst und nur einmal, Raum und Leereintrag fehlen, Exchange-Adresse ist nur ein Name"
    );
}

#[test]
fn join_url_comes_from_the_online_meeting_then_from_location_or_preview() {
    let mut item = event_json("U1", "T", "2026-09-29T09:00:00", "2026-09-29T10:00:00");
    item["onlineMeeting"] = json!({"joinUrl": "https://teams.microsoft.com/l/meetup-join/abc"});
    item["location"] = json!({"displayName": "https://meet.google.com/xyz"});
    let e = parse_event(&item, "graph-1", None).unwrap();
    assert_eq!(
        e.join_url.as_deref(),
        Some("https://teams.microsoft.com/l/meetup-join/abc")
    );
    assert_eq!(e.location.as_deref(), Some("https://meet.google.com/xyz"));

    let mut item = event_json("U1", "T", "2026-09-29T09:00:00", "2026-09-29T10:00:00");
    item["onlineMeeting"] = Value::Null;
    item["bodyPreview"] = json!("Bitte beitreten: https://zoom.us/j/12345?pwd=x.");
    let e = parse_event(&item, "graph-1", None).unwrap();
    assert_eq!(e.join_url.as_deref(), Some("https://zoom.us/j/12345?pwd=x"));
    assert_eq!(
        e.description.as_deref(),
        Some("Bitte beitreten: https://zoom.us/j/12345?pwd=x.")
    );

    // Kein http(s): nicht uebernehmen.
    let mut item = event_json("U1", "T", "2026-09-29T09:00:00", "2026-09-29T10:00:00");
    item["onlineMeeting"] = json!({"joinUrl": "javascript:alert(1)"});
    assert_eq!(parse_event(&item, "graph-1", None).unwrap().join_url, None);
}

#[test]
fn title_and_fields_are_capped_and_a_missing_subject_is_defaulted() {
    let mut item = event_json("U1", "  ", "2026-09-29T09:00:00", "2026-09-29T10:00:00");
    item["subject"] = Value::Null;
    assert_eq!(
        parse_event(&item, "graph-1", None).unwrap().title,
        ics::UNTITLED
    );
    let mut long = event_json(
        "U1",
        &"T".repeat(1000),
        "2026-09-29T09:00:00",
        "2026-09-29T10:00:00",
    );
    long["bodyPreview"] = json!("d".repeat(9000));
    long["location"] = json!({"displayName": "o".repeat(2000)});
    let e = parse_event(&long, "graph-1", None).unwrap();
    assert_eq!(e.title.chars().count(), ics::MAX_TITLE_CHARS);
    assert_eq!(
        e.description.unwrap().chars().count(),
        ics::MAX_DESCRIPTION_CHARS
    );
    assert_eq!(e.location.unwrap().chars().count(), ics::MAX_LOCATION_CHARS);
}

#[test]
fn without_an_ical_uid_the_graph_id_identifies_the_event() {
    let mut item = event_json("U1", "T", "2026-09-29T09:00:00", "2026-09-29T10:00:00");
    item.as_object_mut().unwrap().remove("iCalUId");
    assert_eq!(parse_event(&item, "graph-1", None).unwrap().uid, "AAMk-U1");
}

// ---------------------------------------------------------------------------
// Dublette: Graph gewinnt
// ---------------------------------------------------------------------------

const OUTLOOK_UID: &str = "040000008200E00074C5B7101A82E00800000000D3D70B8A6A17D70100000000000000001000000074665914A06C3F49BB4B7D7EEE4304DA";

fn cal(source: &str, uid: &str, title: &str, start: i64) -> CalEvent {
    CalEvent {
        key: crate::managers::calendar::model::event_key(source, uid, start),
        source_id: source.into(),
        uid: uid.into(),
        title: title.into(),
        starts_at: start,
        ends_at: start + 3_600_000,
        all_day: false,
        cancelled: false,
        location: None,
        join_url: None,
        description: None,
        attendees: vec![],
    }
}

fn graph_set() -> HashSet<String> {
    ["graph-1".to_string()].into_iter().collect()
}

#[test]
fn clean_global_object_id_zeroes_only_the_instance_date() {
    let occurrence = format!("{}20260929{}", &OUTLOOK_UID[..32], &OUTLOOK_UID[40..]);
    assert_eq!(clean_global_object_id(&occurrence), OUTLOOK_UID);
    assert_eq!(
        clean_global_object_id(OUTLOOK_UID),
        OUTLOOK_UID,
        "schon sauber"
    );
    // Kleinschreibung der Kennung zaehlt; alles andere bleibt unangetastet.
    let lower = occurrence.to_ascii_lowercase();
    assert_eq!(
        clean_global_object_id(&lower),
        lower.replace(&lower[32..40], "00000000")
    );
    for other in [
        "",
        "kurz",
        "AAMk-abc",
        "someone@example.org",
        "040000008200E00074C5B7101A82E008ZZZZZZZZ",
    ] {
        assert_eq!(clean_global_object_id(other), other);
    }
    // Nicht-ASCII vor Zeichen 32/40 fuehrt nicht zu einer Panik.
    let umlauts = "äöü".repeat(20);
    assert_eq!(clean_global_object_id(&umlauts), umlauts);
}

#[test]
fn dedupe_graph_wins_by_uid_and_start() {
    let start = ms(2026, 9, 29, 9, 0);
    let events = vec![
        cal("ics-1", "UID-A", "Titel aus ICS", start),
        cal("graph-1", "UID-A", "Titel aus Graph", start),
        cal("ics-1", "UID-B", "Nur im ICS", start),
    ];
    let out = dedupe_graph_wins(events, &graph_set());
    let shown: Vec<(&str, &str)> = out
        .iter()
        .map(|e| (e.source_id.as_str(), e.title.as_str()))
        .collect();
    assert_eq!(
        shown,
        vec![("graph-1", "Titel aus Graph"), ("ics-1", "Nur im ICS")]
    );
}

#[test]
fn dedupe_matches_a_series_occurrence_whose_graph_uid_carries_the_date() {
    let start = ms(2026, 9, 29, 9, 0);
    let occurrence = format!("{}20260929{}", &OUTLOOK_UID[..32], &OUTLOOK_UID[40..]);
    let events = vec![
        cal("ics-1", OUTLOOK_UID, "Jour fixe (ICS)", start),
        cal(
            "graph-1",
            &occurrence,
            "Jour fixe (Graph, anderer Titel)",
            start,
        ),
    ];
    let out = dedupe_graph_wins(events, &graph_set());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].source_id, "graph-1");
}

#[test]
fn dedupe_falls_back_to_start_and_title_when_the_uids_differ() {
    let start = ms(2026, 9, 29, 9, 0);
    let events = vec![
        cal("ics-1", "GANZ-ANDERE-UID", "  Jour   FIXE Vertrieb ", start),
        cal("graph-1", "AAMk-xyz", "Jour fixe Vertrieb", start),
    ];
    let out = dedupe_graph_wins(events, &graph_set());
    assert_eq!(
        out.iter().map(|e| e.source_id.as_str()).collect::<Vec<_>>(),
        vec!["graph-1"]
    );
}

#[test]
fn dedupe_keeps_events_that_only_look_alike() {
    let start = ms(2026, 9, 29, 9, 0);
    let events = vec![
        // anderer Beginn, gleiche UID und gleicher Titel: eine andere Instanz
        cal("ics-1", "UID-A", "Jour fixe", start + 7 * 86_400_000),
        cal("graph-1", "UID-A", "Jour fixe", start),
        // gleicher Beginn, anderer Titel und andere UID
        cal("ics-1", "UID-C", "Zahnarzt", start),
        // ohne Titel: nie ueber den Titel gleichsetzen
        cal("ics-1", "UID-D", ics::UNTITLED, start),
        cal("graph-1", "UID-E", ics::UNTITLED, start),
    ];
    let out = dedupe_graph_wins(events.clone(), &graph_set());
    assert_eq!(out.len(), events.len(), "{out:?}");
}

#[test]
fn dedupe_never_touches_graph_events_or_a_calendar_without_graph() {
    let start = ms(2026, 9, 29, 9, 0);
    let two_graph = vec![
        cal("graph-1", "UID-A", "Gleich", start),
        cal("graph-2", "UID-A", "Gleich", start),
    ];
    let both: HashSet<String> = ["graph-1".to_string(), "graph-2".to_string()]
        .into_iter()
        .collect();
    assert_eq!(dedupe_graph_wins(two_graph.clone(), &both).len(), 2);
    let only_ics = vec![
        cal("ics-1", "UID-A", "Gleich", start),
        cal("ics-2", "UID-A", "Gleich", start),
    ];
    assert_eq!(
        dedupe_graph_wins(only_ics.clone(), &HashSet::new()),
        only_ics
    );
    // Graph-Quelle da, aber ohne Termine: nichts wird ausgeblendet.
    assert_eq!(dedupe_graph_wins(only_ics.clone(), &graph_set()), only_ics);
}

#[test]
fn the_store_shows_the_graph_event_and_brings_the_ics_one_back_when_graph_is_off() {
    let s = store();
    s.calendar_source_add("ics-1", CalendarKind::Ics, "Outlook ICS", None, 1)
        .unwrap();
    s.calendar_source_add(
        "graph-1",
        CalendarKind::Graph,
        SOURCE_LABEL,
        Some("a@b.de"),
        1,
    )
    .unwrap();
    let start = ms(2026, 9, 29, 9, 0);
    let meta = crate::managers::meetings::store::SyncMeta {
        has_attendee_data: true,
        etag: None,
        last_modified: None,
        now_ms: 1,
    };
    s.calendar_replace_events(
        "ics-1",
        &[
            cal("ics-1", "UID-A", "Jour fixe", start),
            cal("ics-1", "UID-B", "Nur ICS", start + 3_600_000),
        ],
        &meta,
    )
    .unwrap();
    s.calendar_replace_events(
        "graph-1",
        &[cal("graph-1", "UID-A", "Jour fixe", start)],
        &meta,
    )
    .unwrap();

    let list = |s: &MeetingStore| -> Vec<String> {
        s.calendar_events_between(start - 1, start + 86_400_000, false)
            .unwrap()
            .into_iter()
            .map(|e| format!("{}:{}", e.source_id, e.uid))
            .collect()
    };
    assert_eq!(list(&s), vec!["graph-1:UID-A", "ics-1:UID-B"]);
    // Graph aus: der ICS-Termin ist wieder da (nichts ging verloren).
    s.calendar_source_set_enabled("graph-1", false, 2).unwrap();
    assert_eq!(list(&s), vec!["ics-1:UID-A", "ics-1:UID-B"]);
    // Entfernte Graph-Quelle: ebenso.
    s.calendar_source_set_enabled("graph-1", true, 3).unwrap();
    s.calendar_source_remove("graph-1", 4).unwrap();
    assert_eq!(list(&s), vec!["ics-1:UID-A", "ics-1:UID-B"]);
}

// ---------------------------------------------------------------------------
// Anmeldung im Ganzen
// ---------------------------------------------------------------------------

/// Ein Token-Server, der den PKCE-Beweis prueft: `S256(verifier) == challenge`.
async fn pkce_checking_token_server(challenge: Arc<Mutex<Option<String>>>) -> (String, Seen) {
    serve(move |r| {
        let f = r.form();
        let expected = challenge.lock().unwrap().clone();
        let proof_ok = f
            .get("code_verifier")
            .is_some_and(|v| Some(challenge_s256(v)) == expected);
        if f.get("grant_type").map(String::as_str) == Some("authorization_code")
            && f.get("code").map(String::as_str) == Some("auth-code-1")
            && proof_ok
        {
            Resp::json(200, tokens_json("AT-1", Some("RT-1")))
        } else {
            Resp::json(400, json!({"error": "invalid_grant", "error_description": "AADSTS70008: PKCE-Prüfung fehlgeschlagen"}))
        }
    })
    .await
}

fn query_of(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

#[tokio::test]
async fn sign_in_runs_the_whole_flow_and_proves_pkce_to_the_token_endpoint() {
    let challenge = Arc::new(Mutex::new(None));
    let (base, seen) = pkce_checking_token_server(challenge.clone()).await;
    let opened: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let (opened2, challenge2) = (opened.clone(), challenge.clone());
    let cancel = Notify::new();
    let signed = sign_in(
        &eps(&base),
        CLIENT_ID,
        "contoso.com",
        move |url| {
            let q = query_of(&url);
            *challenge2.lock().unwrap() = Some(q["code_challenge"].clone());
            *opened2.lock().unwrap() = Some(url.clone());
            // Der „Browser“: ruft die Weiterleitung mit Code und state auf.
            let port: u16 = q["redirect_uri"]
                .rsplit(':')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let state = q["state"].clone();
            tokio::spawn(async move {
                raw_get(
                    port,
                    &format!("/?code=auth-code-1&state={state}"),
                    &format!("localhost:{port}"),
                )
                .await;
            });
            Ok(())
        },
        Duration::from_secs(10),
        &cancel,
    )
    .await
    .unwrap();

    assert_eq!(signed.access_token.as_str(), "AT-1");
    assert_eq!(signed.account.refresh_token(), "RT-1");
    assert_eq!(signed.account.client_id, CLIENT_ID);
    assert_eq!(signed.account.tenant, "contoso.com");
    let url = opened.lock().unwrap().clone().unwrap();
    assert!(
        url.starts_with(&format!("{base}/contoso.com/oauth2/v2.0/authorize?")),
        "{url}"
    );
    let q = query_of(&url);
    assert_eq!(q["code_challenge_method"], "S256");
    assert!(q["redirect_uri"].starts_with("http://localhost:"));
    let form = seen.lock().unwrap()[0].form();
    assert_eq!(
        form["redirect_uri"], q["redirect_uri"],
        "dieselbe URI wie in der Anfrage"
    );
    assert_eq!(challenge_s256(&form["code_verifier"]), q["code_challenge"]);
}

#[tokio::test]
async fn sign_in_reports_a_missing_browser_and_starts_no_exchange() {
    let (base, seen) = serve(|_| Resp::json(200, tokens_json("AT", Some("RT")))).await;
    let err = sign_in(
        &eps(&base),
        CLIENT_ID,
        "",
        |_| Err("Kein Standardbrowser gefunden".to_string()),
        Duration::from_secs(10),
        &Notify::new(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, GraphError::Browser(m) if m.contains("Standardbrowser")),
        "{err:?}"
    );
    assert!(err
        .to_string()
        .starts_with("Der Browser konnte nicht geöffnet werden"));
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sign_in_needs_a_refresh_token_from_microsoft() {
    let (base, _) = serve(|_| Resp::json(200, tokens_json("AT", None))).await;
    let err = sign_in(
        &eps(&base),
        CLIENT_ID,
        "",
        |url| {
            let q = query_of(&url);
            let port: u16 = q["redirect_uri"]
                .rsplit(':')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let state = q["state"].clone();
            tokio::spawn(async move {
                raw_get(
                    port,
                    &format!("/?code=c&state={state}"),
                    &format!("localhost:{port}"),
                )
                .await;
            });
            Ok(())
        },
        Duration::from_secs(10),
        &Notify::new(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, GraphError::Denied(m) if m.contains("offline_access")),
        "{err:?}"
    );
}

#[tokio::test]
async fn sign_in_times_out_is_cancellable_and_refuses_a_bad_client_id() {
    let (base, seen) = serve(|_| Resp::json(200, tokens_json("AT", Some("RT")))).await;
    let ep = eps(&base);
    let err = sign_in(
        &ep,
        CLIENT_ID,
        "",
        |_| Ok(()),
        Duration::from_millis(150),
        &Notify::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(err, GraphError::SignInTimeout);

    let cancel = Arc::new(Notify::new());
    let c2 = cancel.clone();
    let task = tokio::spawn({
        let ep = ep.clone();
        async move { sign_in(&ep, CLIENT_ID, "", |_| Ok(()), Duration::from_secs(30), &c2).await }
    });
    settle().await;
    cancel.notify_one();
    let err = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(err, GraphError::Cancelled);

    // Ein Abbruch VOR dem Warten geht nicht verloren (Notify merkt ihn).
    let early = Notify::new();
    early.notify_one();
    let err = sign_in(
        &ep,
        CLIENT_ID,
        "",
        |_| Ok(()),
        Duration::from_secs(30),
        &early,
    )
    .await
    .unwrap_err();
    assert_eq!(err, GraphError::Cancelled);

    let opened = AtomicBool::new(false);
    let err = sign_in(
        &ep,
        "kaputt",
        "",
        |_| {
            opened.store(true, Ordering::SeqCst);
            Ok(())
        },
        Duration::from_secs(1),
        &Notify::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, GraphError::Config(_)));
    assert!(
        !opened.load(Ordering::SeqCst),
        "der Browser oeffnet sich nicht mit falscher ID"
    );
    let err = sign_in(
        &ep,
        CLIENT_ID,
        "a/b",
        |_| Ok(()),
        Duration::from_secs(1),
        &Notify::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, GraphError::Config(_)));
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn only_one_sign_in_at_a_time() {
    let state = GraphState::default();
    assert!(!state.cancel_sign_in(), "nichts zu brechen");
    let (cancel, guard) = state.begin_sign_in().unwrap();
    let second = state.begin_sign_in().unwrap_err();
    assert!(second.to_string().contains("läuft bereits"), "{second}");
    assert!(state.cancel_sign_in());
    // Das Signal kam an.
    let waited = futures_util::FutureExt::now_or_never(cancel.notified());
    assert!(waited.is_some(), "der Abbruch ist zugestellt");
    drop(guard);
    assert!(
        state.begin_sign_in().is_ok(),
        "nach dem Ende ist die naechste erlaubt"
    ); // Klammer sofort verworfen
       // Jede Anmeldung bekommt ein eigenes Signal: ein altes Signal bricht die neue nicht.
    let (c1, g1) = state.begin_sign_in().unwrap();
    drop(g1);
    let (c2, _g2) = state.begin_sign_in().unwrap();
    assert!(!Arc::ptr_eq(&c1, &c2));
}

// ---------------------------------------------------------------------------
// Quelle anlegen
// ---------------------------------------------------------------------------

fn me() -> Me {
    Me {
        address: "anna.berg@firma.de".into(),
        name: Some("Anna Berg".into()),
    }
}

fn account(rt: &str) -> StoredAccount {
    StoredAccount::new(CLIENT_ID.into(), "common".into(), rt.into())
}

#[test]
fn register_account_creates_the_source_the_secret_and_the_self_person() {
    let s = store();
    let vault = MemVault::default();
    let source = register_account(&s, &vault, &account("RT-1"), &me(), 1_000).unwrap();
    assert_eq!(source.kind, CalendarKind::Graph);
    assert_eq!(source.label, SOURCE_LABEL);
    assert_eq!(source.account_hint.as_deref(), Some("anna.berg@firma.de"));
    assert!(source.id.starts_with("graph-") && source.id.len() == 22);
    assert!(source.enabled);
    // Das Geheimnis liegt unter der Quellen-ID und traegt Client-ID und Verzeichnis.
    let stored = load_account(&vault, &source.id).unwrap();
    assert_eq!(stored.refresh_token(), "RT-1");
    assert_eq!(stored.client_id, CLIENT_ID);
    // Der Klartext steht in keiner Zeile der Datenbank.
    let conn = s.get_connection().unwrap();
    let hit: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM calendar_sources WHERE id || label || IFNULL(account_hint,'') || IFNULL(last_error,'') LIKE '%RT-1%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hit, 0);
    // Die eigene Adresse ist „ich“.
    let person = s
        .find_person(Some("anna.berg@firma.de"), None)
        .unwrap()
        .unwrap();
    assert!(person.is_self);
    assert_eq!(person.name, "Anna Berg");
}

#[test]
fn register_account_reuses_the_source_of_the_same_address() {
    let s = store();
    let vault = MemVault::default();
    let first = register_account(&s, &vault, &account("RT-1"), &me(), 1_000).unwrap();
    s.calendar_source_mark_sync(
        &first.id,
        2_000,
        crate::managers::meetings::store::SyncMark::Failed("Anmeldung nötig"),
    )
    .unwrap();
    s.calendar_source_set_enabled(&first.id, false, 2_500)
        .unwrap();
    let again = register_account(
        &s,
        &vault,
        &account("RT-2"),
        &Me {
            address: "ANNA.BERG@firma.de".into(),
            name: None,
        },
        3_000,
    )
    .unwrap();
    assert_eq!(again.id, first.id, "keine zweite Quelle");
    assert!(
        again.enabled,
        "die erneute Anmeldung schaltet sie wieder ein"
    );
    assert_eq!(s.calendar_sources().unwrap().len(), 1);
    assert_eq!(vault.refresh_token(&first.id).as_deref(), Some("RT-2"));
    // Ein anderes Konto ist eine zweite Quelle.
    let other = register_account(
        &s,
        &vault,
        &account("RT-3"),
        &Me {
            address: "b@other.de".into(),
            name: None,
        },
        4_000,
    )
    .unwrap();
    assert_ne!(other.id, first.id);
    assert_eq!(s.calendar_sources().unwrap().len(), 2);
}

#[test]
fn register_account_without_a_writable_vault_creates_no_source() {
    let s = store();
    let vault = MemVault::default();
    vault.fail_put.store(true, Ordering::SeqCst);
    let err = register_account(&s, &vault, &account("RT-1"), &me(), 1_000).unwrap_err();
    assert!(err.contains("nicht speicherbar"), "{err}");
    assert!(
        s.calendar_sources().unwrap().is_empty(),
        "keine Quelle ohne Geheimnis"
    );
    assert!(vault.map.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Geheimnisse
// ---------------------------------------------------------------------------

#[test]
fn nothing_prints_a_token() {
    let acc = account("RT-SEHR-GEHEIM");
    let signed = SignedIn {
        access_token: Zeroizing::new("AT-SEHR-GEHEIM".into()),
        expires_in: Duration::from_secs(1),
        account: acc.clone(),
    };
    let tokens = Tokens {
        access_token: Zeroizing::new("AT-SEHR-GEHEIM".into()),
        refresh_token: Some(Zeroizing::new("RT-SEHR-GEHEIM".into())),
        expires_in: Duration::from_secs(1),
    };
    let shown = format!(
        "{acc:?} {signed:?} {tokens:?} {:?}",
        Callback::Code(Zeroizing::new("CODE-GEHEIM".into()))
    );
    for secret in ["SEHR-GEHEIM", "CODE-GEHEIM"] {
        assert!(!shown.contains(secret), "{shown}");
    }
}

#[test]
fn the_secret_blob_is_versioned_json_with_client_and_tenant() {
    let vault = MemVault::default();
    save_account(&vault, "graph-1", &account("RT")).unwrap();
    let raw = vault.get("graph-1").unwrap().unwrap();
    let v: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(v["v"], 1);
    assert_eq!(v["client_id"], CLIENT_ID);
    assert_eq!(v["tenant"], "common");
    assert_eq!(v["refresh_token"], "RT");
}

/// Mit dem echten DPAPI-Speicher: ein grosses Erneuerungs-Token (mehr als die 2 560
/// Byte des Anmeldeinformationsspeichers) geht hin und zurueck, und auf dem Datentraeger
/// steht kein Klartext.
#[cfg(windows)]
struct DirVault(std::path::PathBuf);

#[cfg(windows)]
impl SecretVault for DirVault {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        secret::get_in(&self.0, name)
    }
    fn put(&self, name: &str, data: &[u8]) -> Result<(), String> {
        secret::put_in(&self.0, name, data)
    }
    fn delete(&self, name: &str) {
        secret::delete_in(&self.0, name)
    }
}

#[cfg(windows)]
#[tokio::test]
async fn dpapi_roundtrip_of_a_large_refresh_token_through_the_real_vault() {
    let dir = tempfile::tempdir().unwrap();
    let vault = DirVault(dir.path().to_path_buf());
    let big = format!("0.AAAA{}", "xQ7-_".repeat(700)); // > 3 KB
    save_account(&vault, "graph-1", &account(&big)).unwrap();
    assert_eq!(
        load_account(&vault, "graph-1").unwrap().refresh_token(),
        big
    );
    let raw = std::fs::read(dir.path().join("graph-1.bin")).unwrap();
    assert!(
        !raw.windows(20).any(|w| w == &big.as_bytes()[..20]),
        "kein Klartext im Speicher"
    );
    assert!(!raw
        .windows(CLIENT_ID.len())
        .any(|w| w == CLIENT_ID.as_bytes()));

    // Und die Erneuerung schreibt das neue Token ueber DPAPI zurueck.
    let (base, _) = serve(|_| Resp::json(200, tokens_json("AT", Some("RT-ROTIERT")))).await;
    access_token(
        &eps(&base),
        &vault,
        &TokenCache::default(),
        "graph-1",
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        load_account(&vault, "graph-1").unwrap().refresh_token(),
        "RT-ROTIERT"
    );
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["graph-1.bin".to_string()], "kein .tmp-Rest");
}
