//! Tests der Integrations-Bausteine (B5). Kein Netz, keine Fenster, kein echter Mailversand:
//! Microsoft Graph ist der lokale Test-Server aus A5, der Mailserver ein SMTP-Server auf Loopback,
//! der Webhook ein HTTP-Server auf Loopback. Alle Dateien liegen in Wegwerf-Ordnern.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;

use super::*;
use crate::managers::calendar::model::{event_key, Attendee, CalEvent};
use crate::managers::integrations::approvals;
use crate::managers::integrations::audit::AuditFilter;
use crate::managers::integrations::m365::account::{StoredAccount, Vault};
use crate::managers::integrations::m365::config::M365Config;
use crate::managers::integrations::m365::test_server::{serve, Req, Resp, Seen};
use crate::managers::integrations::m365::FilesMode;
use crate::managers::integrations::model::{AuditEntry, NewIntegration};
use crate::managers::meetings::store::{MeetingSource, MeetingStatus, MeetingStore};
use crate::managers::workflows::app_actions::{self, ServiceError, UnavailableServices};
use crate::managers::workflows::engine::{Clock, CrashPoint, EnqueueRequest, RunOutcome};
use crate::managers::workflows::model::{Origin, RunState, StepState};
use crate::managers::workflows::templates;
use crate::managers::workflows::test_support::{
    armed_workflow, engine, engine_with, roomy_gate, set_grant, step, FakeClock, Fx, T0,
};
use crate::managers::workflows::trigger::calendar::trigger_data;

const CLIENT: &str = "11111111-2222-3333-4444-555555555555";
const TITLE: &str = "Jour fixe Überprüfung";
/// 2026-10-05T08:00:00Z: der Beginn des Termins (so steht er auch im Graph-Test-Server).
const EVENT_START_MS: i64 = 1_791_187_200_000;

// ---------------------------------------------------------------------------
// Dienste der „App“
// ---------------------------------------------------------------------------

struct Svc {
    store: Arc<MeetingStore>,
    m365: Option<Arc<M365Service>>,
    self_emails: Vec<String>,
    secrets: Mutex<HashMap<String, String>>,
    unavailable: UnavailableServices,
}

impl Svc {
    fn put_secret(&self, integration: &str, slot: &str, value: &str) {
        self.secrets
            .lock()
            .unwrap()
            .insert(format!("{integration}/{slot}"), value.to_string());
    }
}

impl AppServices for Svc {
    fn store(&self) -> Result<Arc<MeetingStore>, ServiceError> {
        Ok(self.store.clone())
    }

    fn generate_notes(
        &self,
        req: &app_actions::GenRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<crate::managers::meetings::store::MeetingDocument, ServiceError> {
        self.unavailable.generate_notes(req, cancel)
    }

    fn generate_minutes(
        &self,
        req: &app_actions::GenRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<crate::managers::meetings::store::MeetingDocument, ServiceError> {
        self.unavailable.generate_minutes(req, cancel)
    }

    fn summarize(
        &self,
        text: &str,
        opts: &crate::summarizer::SummaryOptions,
        cancel: &dyn Fn() -> bool,
    ) -> Result<String, ServiceError> {
        self.unavailable.summarize(text, opts, cancel)
    }

    fn audio_dir(&self, meeting_id: Option<&str>) -> Result<PathBuf, ServiceError> {
        self.unavailable.audio_dir(meeting_id)
    }

    fn render_speech(
        &self,
        text: &str,
        out: &Path,
        cancel: &dyn Fn() -> bool,
    ) -> Result<PathBuf, ServiceError> {
        self.unavailable.render_speech(text, out, cancel)
    }

    fn notify(&self, title: &str, body: &str) -> Result<(), ServiceError> {
        self.unavailable.notify(title, body)
    }

    fn self_emails(&self) -> Vec<String> {
        self.self_emails.clone()
    }

    fn m365(&self) -> Option<Arc<M365Service>> {
        self.m365.clone()
    }

    fn secret(
        &self,
        integration: &Integration,
        slot: &str,
    ) -> Result<Option<Zeroizing<String>>, String> {
        Ok(self
            .secrets
            .lock()
            .unwrap()
            .get(&format!("{}/{slot}", integration.id))
            .map(|v| Zeroizing::new(v.clone())))
    }
}

// ---------------------------------------------------------------------------
// Die Gegenstellen: Graph, SMTP, Webhook
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GraphMode {
    Ok,
    Status500,
    Status429,
    Forbidden,
    Drop,
}

struct Graph {
    mode: Arc<Mutex<GraphMode>>,
    seen: Seen,
    /// Der Text des Termins (HTML), wie ihn der Server hat.
    event_body: Arc<Mutex<String>>,
    patches: Arc<Mutex<Vec<Req>>>,
}

impl Graph {
    fn requests(&self, method: &str, path: &str) -> Vec<Req> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == method && r.path() == path)
            .cloned()
            .collect()
    }

    fn mails(&self) -> Vec<Req> {
        self.requests("POST", "/v1.0/me/sendMail")
    }
}

fn start_graph() -> (String, Graph) {
    let mode = Arc::new(Mutex::new(GraphMode::Ok));
    let event_body = Arc::new(Mutex::new(
        "<html><body><p>Einladung</p></body></html>".to_string(),
    ));
    let patches: Arc<Mutex<Vec<Req>>> = Default::default();
    let (m2, b2, p2) = (mode.clone(), event_body.clone(), patches.clone());
    let handler = move |req: &Req| -> Resp {
        if req.method == "POST" && req.path().ends_with("/oauth2/v2.0/token") {
            return Resp::json(
                200,
                json!({"access_token": "AT-1", "expires_in": 3600, "token_type": "Bearer"}),
            );
        }
        let mode = *m2.lock().unwrap();
        match (req.method.as_str(), req.path()) {
            ("POST", "/v1.0/me/sendMail") => match mode {
                GraphMode::Ok => Resp::empty(202),
                GraphMode::Status500 => Resp::json(
                    500,
                    json!({"error": {"code": "InternalServerError", "message": "kaputt"}}),
                ),
                GraphMode::Status429 => Resp::empty(429).with("Retry-After", "30"),
                GraphMode::Forbidden => Resp::json(
                    403,
                    json!({"error": {"code": "ErrorAccessDenied", "message": "Zugriff verweigert"}}),
                ),
                GraphMode::Drop => Resp::Drop,
            },
            ("GET", "/v1.0/me/calendarView") => Resp::json(
                200,
                json!({"value": [
                    {"id": "ANDERER", "iCalUId": "evt-uid-2", "subject": "Anderer",
                     "start": {"dateTime": "2026-10-05T08:00:00.0000000", "timeZone": "UTC"},
                     "end": {"dateTime": "2026-10-05T09:00:00.0000000", "timeZone": "UTC"}},
                    {"id": "EVT=1", "iCalUId": "evt-uid-1", "subject": TITLE,
                     "start": {"dateTime": "2026-10-05T08:00:00.0000000", "timeZone": "UTC"},
                     "end": {"dateTime": "2026-10-05T09:00:00.0000000", "timeZone": "UTC"}}
                ]}),
            ),
            ("GET", "/v1.0/me/events/EVT%3D1") => Resp::json(
                200,
                json!({"id": "EVT=1", "@odata.etag": "W/\"abc\"",
                       "body": {"contentType": "html", "content": b2.lock().unwrap().clone()}}),
            ),
            ("PATCH", "/v1.0/me/events/EVT%3D1") => {
                p2.lock().unwrap().push(req.clone());
                match mode {
                    GraphMode::Forbidden => Resp::json(
                        403,
                        json!({"error": {"code": "ErrorAccessDenied", "message": "Nur der Organisator darf den Termin ändern."}}),
                    ),
                    GraphMode::Drop => Resp::Drop,
                    _ => {
                        let sent = req.json();
                        if let Some(c) = sent.pointer("/body/content").and_then(Value::as_str) {
                            *b2.lock().unwrap() = c.to_string();
                        }
                        Resp::json(200, json!({"id": "EVT=1"}))
                    }
                }
            }
            _ => Resp::empty(404),
        }
    };
    let (base, seen) = tauri::async_runtime::block_on(serve(handler));
    (
        base,
        Graph {
            mode,
            seen,
            event_body,
            patches,
        },
    )
}

#[derive(Clone, Debug)]
struct SmtpMail {
    from: String,
    rcpt: Vec<String>,
    data: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SmtpMode {
    Ok,
    /// Die Nachricht wird gelesen, dann reisst die Verbindung ab (ohne Antwort).
    DropAfterData,
    /// Der Server lehnt einen Empfaenger ab.
    RejectRcpt,
}

struct Smtp {
    port: u16,
    mode: Arc<Mutex<SmtpMode>>,
    mails: Arc<Mutex<Vec<SmtpMail>>>,
    connections: Arc<AtomicUsize>,
}

fn start_smtp() -> Smtp {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mode = Arc::new(Mutex::new(SmtpMode::Ok));
    let mails: Arc<Mutex<Vec<SmtpMail>>> = Default::default();
    let connections = Arc::new(AtomicUsize::new(0));
    let (m2, mails2, c2) = (mode.clone(), mails.clone(), connections.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            c2.fetch_add(1, Ordering::SeqCst);
            let mode = *m2.lock().unwrap();
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let say = |w: &mut std::net::TcpStream, t: &str| {
                let _ = w.write_all(t.as_bytes());
                let _ = w.flush();
            };
            say(&mut writer, "220 mock.test ESMTP\r\n");
            let mut mail = SmtpMail {
                from: String::new(),
                rcpt: Vec::new(),
                data: String::new(),
            };
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let up = line.trim_end().to_uppercase();
                if up.starts_with("EHLO") {
                    say(&mut writer, "250-mock.test\r\n250 8BITMIME\r\n");
                } else if up.starts_with("MAIL FROM") {
                    mail.from = line.trim_end().to_string();
                    say(&mut writer, "250 ok\r\n");
                } else if up.starts_with("RCPT TO") {
                    if mode == SmtpMode::RejectRcpt {
                        say(&mut writer, "550 5.1.1 Empfaenger unbekannt\r\n");
                    } else {
                        mail.rcpt.push(line.trim_end()[8..].trim().to_string());
                        say(&mut writer, "250 ok\r\n");
                    }
                } else if up == "DATA" {
                    say(&mut writer, "354 los\r\n");
                    let mut data = String::new();
                    loop {
                        let mut l = String::new();
                        match reader.read_line(&mut l) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {}
                        }
                        if l == ".\r\n" {
                            break;
                        }
                        data.push_str(&l);
                    }
                    mail.data = data;
                    mails2.lock().unwrap().push(mail.clone());
                    if mode == SmtpMode::DropAfterData {
                        break;
                    }
                    say(&mut writer, "250 2.0.0 angenommen\r\n");
                } else if up == "QUIT" {
                    say(&mut writer, "221 tschuess\r\n");
                    break;
                } else {
                    say(&mut writer, "250 ok\r\n");
                }
            }
        }
    });
    Smtp {
        port,
        mode,
        mails,
        connections,
    }
}

#[derive(Clone, Debug)]
struct HookSeen {
    head: String,
    body: String,
}

struct Hook {
    base: String,
    seen: Arc<Mutex<Vec<HookSeen>>>,
    /// Was der Webhook beantwortet (Rohbytes), je Verbindung die erste Zeile der Liste.
    reply: Arc<Mutex<Vec<u8>>>,
    silent: Arc<AtomicBool>,
}

fn http_reply(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
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

fn start_hook() -> Hook {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<HookSeen>>> = Default::default();
    let reply = Arc::new(Mutex::new(http_reply(
        200,
        &[("Content-Type", "application/json")],
        br#"{"ok":true,"ticket":"T-42"}"#,
    )));
    let silent = Arc::new(AtomicBool::new(false));
    let (s2, r2, q2) = (seen.clone(), reply.clone(), silent.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
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
            s2.lock().unwrap().push(HookSeen {
                head,
                body: String::from_utf8_lossy(&body).to_string(),
            });
            if q2.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(20));
            } else {
                let bytes = r2.lock().unwrap().clone();
                let _ = s.write_all(&bytes);
                let _ = s.flush();
            }
        }
    });
    Hook {
        base,
        seen,
        reply,
        silent,
    }
}

// ---------------------------------------------------------------------------
// Die Welt
// ---------------------------------------------------------------------------

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
    svc: Arc<Svc>,
    graph: Graph,
    smtp: Smtp,
    hook: Hook,
    docs: PathBuf,
    meeting_id: String,
    secret_url: String,
}

fn temp_dir() -> PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().to_path_buf();
    std::mem::forget(dir);
    p
}

/// Ordner fuer den Tresor des Kontos und Vorlagen fuer die Besprechung.
fn make_meeting(store: &MeetingStore) -> String {
    let m = store
        .create_meeting(TITLE, MeetingSource::Import, None)
        .unwrap();
    store.set_status(&m.id, MeetingStatus::Ready).unwrap();
    store
        .insert_document(
            &m.id,
            "minutes",
            "markdown",
            "# Protokoll\n\n## Zusammenfassung\n\nDer Go-Live wurde auf Ende Oktober gelegt; die Abnahme prüft Frau Müller.\n\n## Entscheidungen\n\n- Go-Live am 30. Oktober\n",
            None,
            None,
        )
        .unwrap();
    m.id
}

fn world() -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);

    let (graph_base, graph) = start_graph();
    let smtp = start_smtp();
    let hook = start_hook();

    // Microsoft-365-Konto: angemeldet (Erneuerungs-Token im Test-Tresor), Mail und Termin-Notiz an.
    let vault_dir = temp_dir();
    let vault = Vault::in_dir(vault_dir);
    let cfg = M365Config::new(
        CLIENT,
        "common",
        &[Capability::MailSend, Capability::CalendarWrite],
        FilesMode::Full,
        "Local Voice AI",
    )
    .unwrap();
    let svc365 = M365Service::new(
        crate::managers::calendar::graph::Endpoints {
            authority: graph_base.clone(),
            graph: format!("{graph_base}/v1.0"),
            use_env_proxy: false,
        },
        vault.clone(),
    );
    let docs = temp_dir();
    {
        let conn = fx.conn();
        let mut n = NewIntegration::new(Kind::M365, "Mein Konto");
        n.id = Some("m365-1".to_string());
        n.config = cfg.to_json();
        let integ = integrations_store::create(&conn, &n, T0).unwrap();
        vault
            .save(
                &integ,
                &StoredAccount::new(
                    cfg.client_id.clone(),
                    cfg.tenant.clone(),
                    cfg.scope_string(),
                    "RT-geheim-0001".to_string(),
                    Some("Ich@Wolff.de".to_string()),
                    Some("Ich".to_string()),
                ),
            )
            .unwrap();

        // SMTP-Postfach auf Loopback, ohne Anmeldung.
        let smtp_cfg = SmtpConfig {
            host: "127.0.0.1".to_string(),
            port: smtp.port,
            security: smtp::Security::Plain,
            username: String::new(),
            from_address: "ich@wolff.de".to_string(),
            from_name: String::new(),
        };
        let mut n = NewIntegration::new(Kind::Smtp, "Mein Postfach");
        n.id = Some("smtp-1".to_string());
        n.config = smtp_cfg.to_json();
        integrations_store::create(&conn, &n, T0).unwrap();

        // Webhook: Konfiguration nur mit Server; die Adresse steht im Test-Geheimnisspeicher.
        let url = parse_hook(&hook.base, "/webhook/GEHEIM-pfad-4711?sig=zzz");
        let mut n = NewIntegration::new(Kind::Webhook, "n8n Protokolle");
        n.id = Some("hook-1".to_string());
        n.config = webhook::config_for(&url);
        integrations_store::create(&conn, &n, T0).unwrap();

        // Ordner fuer die Protokolle (lesen und schreiben erlaubt).
        let mut n = NewIntegration::new(Kind::Folder, "Protokolle");
        n.id = Some("folder-protokolle".to_string());
        n.config = json!({ "path": docs.to_string_lossy() });
        integrations_store::create(&conn, &n, T0).unwrap();
        set_grant(
            &conn,
            "folder-protokolle",
            Capability::FilesWrite,
            GrantMode::Allow,
        );
        set_grant(
            &conn,
            "folder-protokolle",
            Capability::FilesRead,
            GrantMode::Allow,
        );

        // Zwei Kalender des Nutzers.
        for id in ["cal-a", "cal-b"] {
            crate::managers::workflows::test_support::register(&conn, Kind::Ics, id);
        }
    }
    let secret_url = format!("{}/webhook/GEHEIM-pfad-4711?sig=zzz", hook.base);
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let meeting_id = make_meeting(&store);
    let svc = Arc::new(Svc {
        store,
        m365: Some(Arc::new(svc365)),
        self_emails: vec!["ich@wolff.de".to_string()],
        secrets: Mutex::new(HashMap::new()),
        unavailable: UnavailableServices,
    });
    svc.put_secret("hook-1", "url", &secret_url);
    app_actions::install(&engine, svc.clone());
    install(&engine, svc.clone());
    World {
        fx,
        clock,
        engine,
        svc,
        graph,
        smtp,
        hook,
        docs,
        meeting_id,
        secret_url,
    }
}

fn parse_hook(base: &str, path: &str) -> url::Url {
    webhook::parse_url(&format!("{base}{path}")).unwrap()
}

impl World {
    fn conn(&self) -> rusqlite::Connection {
        self.fx.conn()
    }

    fn grant(&self, id: &str, cap: Capability, mode: GrantMode) {
        set_grant(&self.conn(), id, cap, mode);
    }

    fn audit(&self) -> Vec<AuditEntry> {
        crate::managers::integrations::audit::list(&self.conn(), &AuditFilter::default(), 300)
            .unwrap()
    }

    fn detail(&self, run: &str) -> crate::managers::workflows::engine::RunDetail {
        self.engine.run_detail(run).unwrap()
    }

    fn step_state(&self, run: &str, id: &str) -> StepState {
        self.detail(run)
            .steps
            .iter()
            .rfind(|s| s.step_id == id)
            .map(|s| s.state)
            .unwrap_or_else(|| panic!("Schritt {id} fehlt"))
    }

    fn step_output(&self, run: &str, id: &str) -> Value {
        let d = self.detail(run);
        let s = d.steps.iter().rfind(|s| s.step_id == id).unwrap();
        serde_json::from_str(s.output_json.as_deref().unwrap_or("{}")).unwrap()
    }

    fn pending(&self) -> Vec<crate::managers::integrations::model::Approval> {
        approvals::list_pending(&self.conn(), T0).unwrap()
    }

    /// Der Ausloeser „Termin beginnt“ mit Teilnehmenden, wie B2 ihn bildet.
    fn trigger(&self, cal: &str) -> Value {
        let attendee = |email: &str, name: &str, is_self: bool| Attendee {
            email: Some(email.to_string()),
            name: Some(name.to_string()),
            organizer: is_self,
            is_self,
            partstat: None,
        };
        let e = CalEvent {
            key: event_key(cal, "evt-uid-1", EVENT_START_MS),
            source_id: cal.to_string(),
            uid: "evt-uid-1".to_string(),
            title: TITLE.to_string(),
            starts_at: EVENT_START_MS,
            ends_at: EVENT_START_MS + 3_600_000,
            all_day: false,
            cancelled: false,
            location: None,
            join_url: Some("https://teams.example/join".to_string()),
            description: None,
            attendees: vec![
                attendee("ich@wolff.de", "Ich", true),
                attendee("anna@kunde.de", "Anna Kunde", false),
                attendee("bert@wolff.de", "Bert Intern", false),
            ],
        };
        trigger_data(&e, &self.svc.self_emails)
    }

    fn fire(&self, wf: &str, cal: &str) -> String {
        let trigger = self.trigger(cal);
        let key = format!("calendar_start:{}", trigger["event_id"].as_str().unwrap());
        let queued = self
            .engine
            .enqueue(&EnqueueRequest {
                workflow_id: wf.to_string(),
                trigger_key: key,
                origin: Origin::Trigger,
                trigger,
                vars: serde_json::Map::new(),
                force_dry_run: false,
            })
            .unwrap();
        assert!(!queued.dry_run);
        queued.run_id
    }

    fn tick(&self) -> Vec<(String, RunOutcome)> {
        self.engine.tick().unwrap().outcomes
    }

    fn docx_files(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&self.docs)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        v.sort();
        v
    }
}

/// Stand-in fuer `recording.start` (B2): legt die fertige Besprechung in den Lauf, ohne Aufnahme.
struct FakeRecording {
    meeting_id: String,
}

impl Action for FakeRecording {
    fn id(&self) -> &str {
        "recording.start"
    }
    fn effect(&self) -> EffectKind {
        EffectKind::External
    }
    fn needs(&self, _params: &Value) -> Result<Option<Needs>, NeedsError> {
        Ok(None)
    }
    fn describe(&self, _params: &Value) -> String {
        "Aufnahme (Test)".to_string()
    }
    fn run(&self, _ctx: &RunCtx<'_>, _params: &Value) -> Result<StepOutput, StepError> {
        Ok(StepOutput::with_data(json!({
            "meeting": {"id": self.meeting_id, "title": TITLE},
            "meeting_id": self.meeting_id
        })))
    }
}

/// Stand-in fuer `meeting.minutes` (B4, braucht ein Sprachmodell): das Protokoll liegt schon.
struct FakeMinutes;

impl Action for FakeMinutes {
    fn id(&self) -> &str {
        "meeting.minutes"
    }
    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }
    fn needs(&self, _params: &Value) -> Result<Option<Needs>, NeedsError> {
        Ok(None)
    }
    fn describe(&self, _params: &Value) -> String {
        "Protokoll (Test)".to_string()
    }
    fn run(&self, _ctx: &RunCtx<'_>, _params: &Value) -> Result<StepOutput, StepError> {
        Ok(StepOutput::with_data(json!({"made": true})))
    }
}

impl World {
    /// Die Vorlage „Termin -> Protokoll -> Mail“, angepasst: Kalender, Kanal und Empfaengerregel.
    /// Aufnahme und Protokollmodell sind Stand-ins; Ablegen und Mail sind die echten Bausteine.
    fn template(&self, cal: &str, via: &str, empfaenger: &str) -> Value {
        self.engine.register_action(Arc::new(FakeRecording {
            meeting_id: self.meeting_id.clone(),
        }));
        self.engine.register_action(Arc::new(FakeMinutes));
        let mut def: Value = serde_json::from_str(templates::TERMIN_MAIL).unwrap();
        def["trigger"]["integration"] = json!(cal);
        def["variables"]["empfaenger"]["default"] = json!(empfaenger);
        for s in def["steps"].as_array_mut().unwrap() {
            if s["action"] == "mail.send" {
                s["params"]["via"] = json!(via);
            }
        }
        def
    }

    /// Ein Ablauf mit nur einem Mail-Schritt (die Anhaenge liegen schon im Ordner).
    fn mail_only(&self, params: Value) -> Value {
        json!({
            "schema": "lva-workflow@1",
            "name": "Mail",
            "trigger": {"type": "calendar.event_starting", "integration": "cal-a"},
            "steps": [step("m", "mail.send", params)]
        })
    }
}

fn graph_mail_to(req: &Req) -> Vec<String> {
    req.json()["message"]["toRecipients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["emailAddress"]["address"].as_str().unwrap().to_string())
        .collect()
}

fn graph_attachment(req: &Req) -> (String, Vec<u8>) {
    let v = req.json();
    let a = &v["message"]["attachments"][0];
    assert_eq!(a["@odata.type"], "#microsoft.graph.fileAttachment");
    (
        a["name"].as_str().unwrap().to_string(),
        base64::engine::general_purpose::STANDARD
            .decode(a["contentBytes"].as_str().unwrap())
            .unwrap(),
    )
}

/// Der erste base64-Anhang einer MIME-Nachricht, dekodiert.
fn smtp_attachment(data: &str) -> Vec<u8> {
    let start = data
        .find("Content-Transfer-Encoding: base64")
        .expect("ein base64-Teil");
    let rest = &data[start..];
    let body_start = rest.find("\r\n\r\n").expect("Kopf endet") + 4;
    let body = &rest[body_start..];
    let end = body.find("\r\n--").unwrap_or(body.len());
    let b64: String = body[..end].chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("gueltiges base64")
}

fn docx_xml(bytes: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("gültiges ZIP");
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .expect("word/document.xml")
        .read_to_string(&mut xml)
        .unwrap();
    xml
}

// ---------------------------------------------------------------------------
// AK7: Vorlage „Termin -> Protokoll -> Mail“
// ---------------------------------------------------------------------------

#[test]
fn ak7_calendar_a_sends_the_protocol_only_to_me_over_graph() {
    let w = world();
    // Mail an mich darf von selbst gehen: das Recht steht auf „erlaubt“.
    w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
    let wf = armed_workflow(&w.engine, &w.template("cal-a", "m365-1", "ich"));
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert_eq!(
        outcomes,
        vec![(run.clone(), RunOutcome::Done)],
        "{outcomes:?}"
    );

    // Genau eine Mail, genau an mich (die Adresse des Kalenders, nicht die des Kontos).
    let mails = w.graph.mails();
    assert_eq!(mails.len(), 1);
    assert_eq!(graph_mail_to(&mails[0]), vec!["ich@wolff.de".to_string()]);
    assert_eq!(
        mails[0].json()["message"]["subject"],
        format!("Protokoll: {TITLE}")
    );
    assert!(mails[0].json()["message"]["body"]["content"]
        .as_str()
        .unwrap()
        .contains(TITLE));
    // Der Anhang ist das echte Word-Protokoll aus dem Ordner.
    let (name, bytes) = graph_attachment(&mails[0]);
    assert_eq!(name, format!("{TITLE} – Protokoll.docx"));
    let xml = docx_xml(&bytes);
    assert!(
        xml.contains("Jour fixe Überprüfung") && xml.contains("Frau Müller"),
        "{xml}"
    );
    let on_disk = std::fs::read(&w.docx_files()[0]).unwrap();
    assert_eq!(on_disk, bytes, "der Anhang ist die abgelegte Datei");

    // Laufprotokoll: „Mail an alle“ wurde uebersprungen, „Mail an mich“ lief.
    assert_eq!(w.step_state(&run, "mail_ich"), StepState::Done);
    assert_eq!(w.step_state(&run, "mail_alle"), StepState::Skipped);
    let out = w.step_output(&run, "mail_ich");
    assert_eq!(out["sent"], true);
    assert_eq!(out["recipients"], 1);
    assert_eq!(out["rule"], "me");
    assert_eq!(out["attachments"][0], format!("{TITLE} – Protokoll.docx"));
    // Das Tor hat die Mail gebucht (ok), mit dem Empfaenger als Ziel, nie mit dem Text.
    let audit = w.audit();
    let row = audit
        .iter()
        .find(|a| a.capability.as_deref() == Some("mail.send") && a.outcome == "ok")
        .expect("Audit: Mail ok");
    assert_eq!(row.caller, "workflow");
    assert_eq!(row.integration_id.as_deref(), Some("m365-1"));
    assert_eq!(row.target.as_deref(), Some("ich@wolff.de"));
    // Provenienz: der Beleg fuer die Wiederaufnahme steht im Register.
    let conn = w.conn();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM provenance WHERE operation = 'mail' AND actor_ref LIKE ?1",
            [format!("%/{run}/mail_ich")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn ak7_calendar_b_sends_to_all_participants_but_only_after_the_approval() {
    let w = world();
    // Der Nutzer laesst die Vorgabe „fragen“ stehen.
    let wf = armed_workflow(&w.engine, &w.template("cal-b", "smtp-1", "alle"));
    let run = w.fire(&wf, "cal-b");
    let outcomes = w.tick();
    assert_eq!(
        outcomes,
        vec![(run.clone(), RunOutcome::AwaitingApproval)],
        "{outcomes:?}"
    );
    assert!(
        w.smtp.mails.lock().unwrap().is_empty() && w.smtp.connections.load(Ordering::SeqCst) == 0,
        "vor der Freigabe geht nichts hinaus"
    );
    assert_eq!(w.step_state(&run, "mail_ich"), StepState::Skipped);

    // Die Freigabe zeigt Empfaenger, Betreff und Anhang vollstaendig.
    let pending = w.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tool_or_capability, "mail.send");
    assert_eq!(pending[0].integration_id.as_deref(), Some("smtp-1"));
    let preview = pending[0].args_preview.clone().unwrap();
    assert!(preview.contains("Ziel: ich@wolff.de (+2)"), "{preview}");
    for who in ["ich@wolff.de", "anna@kunde.de", "bert@wolff.de"] {
        assert!(
            preview.contains(who),
            "{who} fehlt in der Vorschau: {preview}"
        );
    }
    assert!(
        preview.contains(&format!("Protokoll: {TITLE}")),
        "{preview}"
    );
    assert!(
        preview.contains("Protokoll.docx"),
        "Anhang fehlt: {preview}"
    );
    assert!(preview.contains("sha256"), "{preview}");
    assert!(preview.contains("rule: all"), "{preview}");

    // Nach „Freigeben“ geht die Mail an genau diese drei, mit dem Protokoll.
    approvals::decide(&w.conn(), &pending[0].id, true, T0).unwrap();
    let outcomes = w.tick();
    assert_eq!(
        outcomes,
        vec![(run.clone(), RunOutcome::Done)],
        "{outcomes:?}"
    );
    let mails = w.smtp.mails.lock().unwrap().clone();
    assert_eq!(mails.len(), 1);
    let rcpt: Vec<String> = mails[0]
        .rcpt
        .iter()
        .map(|r| r.trim_matches(['<', '>']).to_string())
        .collect();
    assert_eq!(rcpt, vec!["ich@wolff.de", "anna@kunde.de", "bert@wolff.de"]);
    assert!(mails[0].from.contains("ich@wolff.de"));
    assert!(mails[0].data.contains("Subject:"), "{}", mails[0].data);
    // Der Anhang ist das echte Word-Protokoll (Name nach RFC 2231 kodiert, Inhalt base64).
    assert!(
        mails[0].data.contains("Content-Disposition: attachment"),
        "{}",
        mails[0].data
    );
    assert!(mails[0].data.contains("koll.docx"), "{}", mails[0].data);
    let attached = smtp_attachment(&mails[0].data);
    assert_eq!(attached, std::fs::read(&w.docx_files()[0]).unwrap());
    assert!(docx_xml(&attached).contains("Frau Müller"));
    let out = w.step_output(&run, "mail_alle");
    assert_eq!(out["recipients"], 3);
    assert_eq!(out["rule"], "all");
    // Die Message-ID ist aus der Kennung des Schritts abgeleitet (fuer Server und Postfaecher
    // erkennbar dieselbe Nachricht bei einem Wiederholungsversuch).
    assert!(out["message_id"].as_str().unwrap().starts_with("lva-"));
    // Genau EINE Freigabe, nicht zwei (kein zweiter Torgang im Baustein).
    assert!(w.pending().is_empty());
}

#[test]
fn ak7_the_same_template_serves_both_calendars_with_one_variable() {
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
    let a = armed_workflow(&w.engine, &w.template("cal-a", "m365-1", "ich"));
    let mut def_b = w.template("cal-b", "m365-1", "alle");
    def_b["name"] = json!("Kalender B");
    let b = armed_workflow(&w.engine, &def_b);
    let run_a = w.fire(&a, "cal-a");
    let run_b = w.fire(&b, "cal-b");
    let outcomes = w.tick();
    // A: nur ich, von selbst. B: alle Teilnehmenden, wartet auf die Freigabe (E3).
    let of = |run: &str| outcomes.iter().find(|(r, _)| r == run).unwrap().1.clone();
    assert_eq!(of(&run_a), RunOutcome::Done);
    assert_eq!(of(&run_b), RunOutcome::AwaitingApproval);
    assert_eq!(w.graph.mails().len(), 1, "nur A hat gesendet");
    assert_eq!(graph_mail_to(&w.graph.mails()[0]), vec!["ich@wolff.de"]);
    assert_eq!(w.step_state(&run_a, "mail_alle"), StepState::Skipped);
    assert_eq!(w.step_state(&run_b, "mail_ich"), StepState::Skipped);
}

// ---------------------------------------------------------------------------
// AK8: Rechte
// ---------------------------------------------------------------------------

#[test]
fn ak8_integration_off_denies_the_step_ends_the_run_cleanly_and_audits_it() {
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Off);
    let wf = armed_workflow(&w.engine, &w.template("cal-a", "m365-1", "ich"));
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert!(
        matches!(&outcomes[0].1, RunOutcome::Failed { code } if code == "denied"),
        "{outcomes:?}"
    );
    assert_eq!(w.step_state(&run, "mail_ich"), StepState::Denied);
    let d = w.detail(&run);
    assert_eq!(d.run.state, RunState::Failed);
    assert!(
        d.run.lease_owner.is_none(),
        "der Lauf endet sauber, nichts bleibt gemietet"
    );
    assert!(w.graph.mails().is_empty(), "nichts darf hinausgehen");
    let denied: Vec<_> = w
        .audit()
        .into_iter()
        .filter(|a| a.outcome == "denied" && a.capability.as_deref() == Some("mail.send"))
        .collect();
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].caller, "workflow");
    assert_eq!(denied[0].integration_id.as_deref(), Some("m365-1"));
    assert!(denied[0]
        .detail_json
        .as_deref()
        .unwrap()
        .contains("grant_off"));
    // Das Protokoll liegt trotzdem im Ordner: der Lauf hat bis dahin gearbeitet.
    assert_eq!(w.docx_files().len(), 1);
}

#[test]
fn ak8_integration_switched_off_in_the_register_is_denied_too() {
    let w = world();
    integrations_store::update(
        &w.conn(),
        "smtp-1",
        &crate::managers::integrations::model::IntegrationPatch {
            enabled: Some(false),
            ..Default::default()
        },
        T0,
    )
    .unwrap();
    let wf = armed_workflow(&w.engine, &w.template("cal-a", "smtp-1", "ich"));
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert!(
        matches!(outcomes[0].1, RunOutcome::Failed { .. }),
        "{outcomes:?}"
    );
    assert_eq!(w.step_state(&run, "mail_ich"), StepState::Denied);
    assert!(w.smtp.mails.lock().unwrap().is_empty());
    assert!(w.audit().iter().any(|a| a.outcome == "denied"
        && a.detail_json
            .as_deref()
            .unwrap_or("")
            .contains("integration_disabled")));
}

#[test]
fn ak8_ask_opens_an_approval_with_recipient_subject_and_attachment() {
    let w = world();
    let wf = armed_workflow(&w.engine, &w.template("cal-a", "m365-1", "ich"));
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert_eq!(outcomes[0].1, RunOutcome::AwaitingApproval);
    assert_eq!(w.step_state(&run, "mail_ich"), StepState::AwaitingApproval);
    let pending = w.pending();
    assert_eq!(pending.len(), 1);
    let preview = pending[0].args_preview.clone().unwrap();
    assert!(preview.contains("Ziel: ich@wolff.de"), "{preview}");
    assert!(
        preview.contains("to[0]: ich@wolff.de"),
        "Empfaenger: {preview}"
    );
    assert!(
        preview.contains(&format!("subject: Protokoll: {TITLE}")),
        "Betreff: {preview}"
    );
    assert!(
        preview.contains("attachments[0].file:"),
        "Anhang: {preview}"
    );
    assert!(preview.contains("Protokoll.docx"), "{preview}");
    assert!(w.graph.mails().is_empty());
    // „Verweigern“: der Schritt endet abgelehnt, es geht nichts hinaus.
    approvals::decide(&w.conn(), &pending[0].id, false, T0).unwrap();
    let outcomes = w.tick();
    assert!(
        matches!(outcomes[0].1, RunOutcome::Failed { .. }),
        "{outcomes:?}"
    );
    assert_eq!(w.step_state(&run, "mail_ich"), StepState::Denied);
    assert!(w.graph.mails().is_empty());
}

// ---------------------------------------------------------------------------
// E3: an Dritte nie ohne Freigabe
// ---------------------------------------------------------------------------

#[test]
fn third_parties_always_need_approval_even_when_allowed_and_me_does_not() {
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
    let params = |to: &str| json!({"via": "m365-1", "to": to, "subject": "Hallo", "body": "Text"});
    // Regel „ich“: geht von selbst.
    let me = armed_workflow(&w.engine, &w.mail_only(params("me")));
    let r1 = w.fire(&me, "cal-a");
    assert_eq!(w.tick(), vec![(r1, RunOutcome::Done)]);
    assert_eq!(w.graph.mails().len(), 1);

    // Jede andere Regel: wartet trotz „erlaubt“ auf den Klick.
    for (n, rule) in ["participants", "all", "internal"].into_iter().enumerate() {
        let mut def = w.mail_only(params(rule));
        def["name"] = json!(format!("Regel {rule}"));
        let wf = armed_workflow(&w.engine, &def);
        let run = w.fire(&wf, "cal-a");
        let outcomes = w.tick();
        assert_eq!(
            outcomes,
            vec![(run, RunOutcome::AwaitingApproval)],
            "Regel {rule}: {outcomes:?}"
        );
        assert_eq!(
            w.graph.mails().len(),
            1,
            "Regel {rule} ging ohne Freigabe hinaus"
        );
        assert_eq!(w.pending().len(), n + 1);
    }
}

#[test]
fn the_step_can_opt_out_of_the_cap_for_its_own_workflow() {
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({
            "via": "m365-1", "to": "participants", "subject": "Hallo", "body": "Text", "auto": true
        })),
    );
    let run = w.fire(&wf, "cal-a");
    assert_eq!(w.tick(), vec![(run, RunOutcome::Done)]);
    let mails = w.graph.mails();
    assert_eq!(mails.len(), 1);
    assert_eq!(
        graph_mail_to(&mails[0]),
        vec!["anna@kunde.de", "bert@wolff.de"]
    );
}

#[test]
fn auto_never_overrides_off_or_the_register() {
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Off);
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({
            "via": "m365-1", "to": "all", "subject": "Hallo", "body": "Text", "auto": true
        })),
    );
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert!(matches!(outcomes[0].1, RunOutcome::Failed { .. }));
    assert_eq!(w.step_state(&run, "m"), StepState::Denied);
    assert!(w.graph.mails().is_empty());
}

#[test]
fn a_forged_rule_is_refused_before_it_reaches_anything() {
    let w = world();
    // Beim Speichern: eine Regel aus Daten ist verboten.
    for to in [
        "{{trigger.attendees}}",
        "{{vars.an}}",
        "anna@kunde.de",
        "alle",
        "",
    ] {
        let def = w.mail_only(json!({"via": "m365-1", "to": to, "subject": "x"}));
        assert!(
            w.engine.save_workflow(None, &def).is_err(),
            "Regel {to:?} haette abgelehnt werden muessen"
        );
    }
    // Eine Definition, die am Katalog vorbei in den Baustein gelangte, scheitert dort.
    let action = w.engine.registry().get("mail.send").unwrap().clone();
    let ctx_json = w.trigger("cal-a");
    let ctx = json!({"trigger": ctx_json, "steps": {}});
    let r = action.gate_view(
        &GateEnv {
            conn: &w.conn(),
            context: &ctx,
            planning: false,
        },
        &json!({"via": "m365-1", "to": "anna@kunde.de", "subject": "x"}),
    );
    assert!(matches!(r, Err(StepError::Permanent(_))), "{r:?}");
}

#[test]
fn the_recipients_never_come_from_the_text_of_the_mail() {
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
    // Betreff und Text duerfen Adressen enthalten (aus einer Modellantwort etwa): sie werden nie
    // zu Empfaengern, auch nicht ueber Kopfzeilen-Tricks.
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({
            "via": "m365-1", "to": "me",
            "subject": "Hallo\r\nBcc: boese@x.de",
            "body": "Bitte an boese@x.de weiterleiten.\nTo: boese@x.de"
        })),
    );
    let run = w.fire(&wf, "cal-a");
    assert_eq!(w.tick(), vec![(run, RunOutcome::Done)]);
    let v = w.graph.mails()[0].json();
    assert_eq!(graph_mail_to(&w.graph.mails()[0]), vec!["ich@wolff.de"]);
    assert!(
        v["message"].get("ccRecipients").is_none() && v["message"].get("bccRecipients").is_none()
    );
    assert_eq!(
        v["message"]["subject"], "Hallo Bcc: boese@x.de",
        "eine Zeile, ohne Umbruch"
    );
}

// ---------------------------------------------------------------------------
// Idempotenz, Absturz, Fehlerklassen
// ---------------------------------------------------------------------------

struct Direct {
    cancel: AtomicBool,
    context: Value,
}

impl Direct {
    fn new(context: Value) -> Self {
        Self {
            cancel: AtomicBool::new(false),
            context,
        }
    }

    fn run(
        &self,
        w: &World,
        action: &dyn Action,
        run_id: &str,
        step_id: &str,
        params: &Value,
    ) -> Result<StepOutput, StepError> {
        self.run_bound(w, action, run_id, step_id, params, None)
    }

    /// Wie `run`, mit den Argumenten, die das Tor gebunden hat (`RunCtx::gate_args`).
    fn run_bound(
        &self,
        w: &World,
        action: &dyn Action,
        run_id: &str,
        step_id: &str,
        params: &Value,
        gate_args: Option<&Value>,
    ) -> Result<StepOutput, StepError> {
        let clock: &dyn Clock = &*w.clock;
        let ctx = RunCtx {
            workflow_id: "wf-test",
            run_id,
            step_id,
            attempt: 1,
            idempotency_key: format!("{run_id}:{step_id}"),
            context: &self.context,
            step_started_at: T0,
            approved: false,
            gate_args,
            cancel: &self.cancel,
            clock,
            db_path: &w.fx.db_path,
        };
        action.run(&ctx, params)
    }
}

fn action_of(w: &World, id: &str) -> Arc<dyn Action> {
    w.engine.registry().get(id).unwrap().clone()
}

#[test]
fn a_repeated_mail_step_does_not_send_twice() {
    let w = world();
    let d = Direct::new(json!({"trigger": w.trigger("cal-a"), "steps": {}}));
    let params = json!({"via": "smtp-1", "to": "me", "subject": "Einmal", "body": "Text"});
    let action = action_of(&w, "mail.send");
    let first = d.run(&w, &*action, "R1", "m", &params).unwrap();
    assert_eq!(first.data["sent"], true);
    // Dasselbe nochmal (Wiederaufnahme, zweiter Takt): nichts geht hinaus, das Ergebnis ist da.
    let again = d.run(&w, &*action, "R1", "m", &params).unwrap();
    assert_eq!(again.data["sent"], true);
    assert_eq!(w.smtp.mails.lock().unwrap().len(), 1);
    assert!(again.summary.unwrap().contains("schon gesendet"));
    // Ein anderer Schritt oder Lauf ist eine neue Mail.
    d.run(&w, &*action, "R1", "m2", &params).unwrap();
    d.run(&w, &*action, "R2", "m", &params).unwrap();
    assert_eq!(w.smtp.mails.lock().unwrap().len(), 3);
}

#[test]
fn two_runs_with_the_same_mail_ask_twice() {
    let w = world();
    let params = json!({"via": "m365-1", "to": "me", "subject": "Gleich", "body": "Text"});
    let wf = armed_workflow(&w.engine, &w.mail_only(params));
    let mut run_ids = Vec::new();
    for cal_event in ["a", "b"] {
        let mut trigger = w.trigger("cal-a");
        trigger["event_id"] = json!(format!("cal-a:evt-{cal_event}:{EVENT_START_MS}"));
        let q = w
            .engine
            .enqueue(&EnqueueRequest {
                workflow_id: wf.clone(),
                trigger_key: format!("calendar_start:{cal_event}"),
                origin: Origin::Trigger,
                trigger,
                vars: serde_json::Map::new(),
                force_dry_run: false,
            })
            .unwrap();
        run_ids.push(q.run_id);
    }
    w.tick();
    let pending = w.pending();
    assert_eq!(pending.len(), 2, "je Lauf eine Freigabe, nie eine geteilte");
    // Eine Freigabe loest genau EINE Mail aus.
    approvals::decide(&w.conn(), &pending[0].id, true, T0).unwrap();
    w.tick();
    assert_eq!(w.graph.mails().len(), 1);
}

#[test]
fn a_crash_after_the_mail_went_out_is_confirmed_and_not_sent_again() {
    let w = world();
    w.grant("smtp-1", Capability::MailSend, GrantMode::Allow);
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({"via": "smtp-1", "to": "me", "subject": "Beleg", "body": "Text"})),
    );
    let run = w.fire(&wf, "cal-a");
    w.engine.crash_at("m", CrashPoint::AfterAction);
    let r = w.engine.run_next();
    assert!(
        r.is_err(),
        "der Absturz haette ausgeloest werden muessen: {r:?}"
    );
    assert_eq!(
        w.smtp.mails.lock().unwrap().len(),
        1,
        "die Mail ging hinaus"
    );
    assert_eq!(
        w.step_state(&run, "m"),
        StepState::Running,
        "das Journal kennt `done` nicht"
    );

    // Ein zweiter Prozess uebernimmt: der Beleg aus der Provenienz bestaetigt die Wirkung.
    w.clock.advance(61_000);
    let other = engine_with(&w.fx, &w.clock, roomy_gate());
    app_actions::install(&other, w.svc.clone());
    install(&other, w.svc.clone());
    let report = other.tick().unwrap();
    assert_eq!(report.recovered, 1);
    assert_eq!(
        report.outcomes,
        vec![(run.clone(), RunOutcome::Done)],
        "{report:?}"
    );
    assert_eq!(w.smtp.mails.lock().unwrap().len(), 1, "keine zweite Mail");
    assert_eq!(w.step_state(&run, "m"), StepState::Done);
}

#[test]
fn a_crash_before_the_record_is_uncertain_and_waits_for_the_user() {
    let w = world();
    w.grant("smtp-1", Capability::MailSend, GrantMode::Allow);
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({"via": "smtp-1", "to": "me", "subject": "Unklar", "body": "Text"})),
    );
    let run = w.fire(&wf, "cal-a");
    // Der Absturz kommt NACH dem Beginn des Schritts, aber der Baustein hat noch nichts getan.
    w.engine.crash_at("m", CrashPoint::AfterBegin);
    assert!(w.engine.run_next().is_err());
    assert!(w.smtp.mails.lock().unwrap().is_empty());
    w.clock.advance(61_000);
    let other = engine_with(&w.fx, &w.clock, roomy_gate());
    app_actions::install(&other, w.svc.clone());
    install(&other, w.svc.clone());
    other.tick().unwrap();
    let r = w.engine.run_detail(&run).unwrap().run;
    assert_eq!(r.state, RunState::Failed);
    assert_eq!(r.error_code.as_deref(), Some("effect_uncertain"));
    // Egal wie oft: nie von selbst.
    for _ in 0..3 {
        w.clock.advance(3_600_000);
        other.tick().unwrap();
        w.engine.tick().unwrap();
    }
    assert!(w.smtp.mails.lock().unwrap().is_empty());
}

#[test]
fn smtp_server_down_is_transient_and_sends_nothing() {
    let w = world();
    w.grant("smtp-1", Capability::MailSend, GrantMode::Allow);
    // Den Port freigeben: dort lauscht niemand mehr.
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let conn = w.conn();
    let mut i = integrations_store::get(&conn, "smtp-1").unwrap().unwrap();
    let mut cfg = SmtpConfig::from_config_json(&i.config_json).unwrap();
    cfg.port = dead;
    i.config_json = cfg.to_json().to_string();
    conn.execute(
        "UPDATE integrations SET config_json = ?2 WHERE id = ?1",
        rusqlite::params!["smtp-1", i.config_json],
    )
    .unwrap();
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({"via": "smtp-1", "to": "me", "subject": "x", "body": "y"})),
    );
    let run = w.fire(&wf, "cal-a");
    w.tick();
    // „Nichts ist passiert“: Wiederholung mit Wartezeit, kein Endzustand.
    assert_eq!(w.step_state(&run, "m"), StepState::Retrying);
    assert_eq!(w.detail(&run).run.state, RunState::Queued);
    assert!(w.smtp.mails.lock().unwrap().is_empty());
}

#[test]
fn a_connection_lost_after_the_data_is_unknown_and_never_repeated_by_itself() {
    let w = world();
    w.grant("smtp-1", Capability::MailSend, GrantMode::Allow);
    *w.smtp.mode.lock().unwrap() = SmtpMode::DropAfterData;
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({"via": "smtp-1", "to": "me", "subject": "x", "body": "y"})),
    );
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert!(
        matches!(&outcomes[0].1, RunOutcome::Failed { code } if code == "effect_uncertain"),
        "{outcomes:?}"
    );
    assert_eq!(w.step_state(&run, "m"), StepState::Uncertain);
    assert_eq!(
        w.smtp.mails.lock().unwrap().len(),
        1,
        "der Server hatte die Daten"
    );
    for _ in 0..3 {
        w.clock.advance(3_600_000);
        w.tick();
    }
    assert_eq!(
        w.smtp.mails.lock().unwrap().len(),
        1,
        "nie von selbst ein zweites Mal"
    );
}

#[test]
fn an_smtp_server_that_refuses_a_recipient_is_a_permanent_failure() {
    let w = world();
    w.grant("smtp-1", Capability::MailSend, GrantMode::Allow);
    *w.smtp.mode.lock().unwrap() = SmtpMode::RejectRcpt;
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({"via": "smtp-1", "to": "me", "subject": "x", "body": "y"})),
    );
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert!(
        matches!(&outcomes[0].1, RunOutcome::Failed { code } if code == "permanent"),
        "{outcomes:?}"
    );
    assert_eq!(w.step_state(&run, "m"), StepState::Failed);
    assert!(w.smtp.mails.lock().unwrap().is_empty());
}

#[test]
fn graph_outcomes_are_classified_by_what_they_say_about_delivery() {
    // (Modus, erwarteter Schrittzustand, Lauf-Code)
    let cases = [
        (GraphMode::Status429, StepState::Retrying, None),
        (
            GraphMode::Status500,
            StepState::Uncertain,
            Some("effect_uncertain"),
        ),
        (
            GraphMode::Drop,
            StepState::Uncertain,
            Some("effect_uncertain"),
        ),
        (GraphMode::Forbidden, StepState::Failed, Some("permanent")),
    ];
    for (mode, state, code) in cases {
        let w = world();
        w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
        *w.graph.mode.lock().unwrap() = mode;
        let wf = armed_workflow(
            &w.engine,
            &w.mail_only(json!({"via": "m365-1", "to": "me", "subject": "x", "body": "y"})),
        );
        let run = w.fire(&wf, "cal-a");
        w.tick();
        assert_eq!(w.step_state(&run, "m"), state, "{mode:?}");
        let r = w.detail(&run).run;
        assert_eq!(r.error_code.as_deref(), code, "{mode:?}");
        // Wie oft der Server auch gerufen wurde: nach `Unknown` nie ein zweites Mal.
        if code == Some("effect_uncertain") {
            let n = w.graph.mails().len();
            for _ in 0..3 {
                w.clock.advance(3_600_000);
                w.tick();
            }
            assert_eq!(
                w.graph.mails().len(),
                n,
                "{mode:?}: nie von selbst wiederholt"
            );
        }
    }
}

#[test]
fn a_cancelled_run_never_starts_the_mail() {
    let w = world();
    let d = Direct::new(json!({"trigger": w.trigger("cal-a"), "steps": {}}));
    d.cancel.store(true, Ordering::SeqCst);
    let params = json!({"via": "smtp-1", "to": "me", "subject": "x", "body": "y"});
    let r = d.run(&w, &*action_of(&w, "mail.send"), "R9", "m", &params);
    assert!(matches!(r, Err(StepError::Transient(_))), "{r:?}");
    assert!(w.smtp.mails.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Anhaenge im Lauf
// ---------------------------------------------------------------------------

fn doc_in(w: &World, name: &str, bytes: &[u8]) -> String {
    let p = w.docs.join(name);
    std::fs::write(&p, bytes).unwrap();
    p.to_string_lossy().to_string()
}

#[test]
fn an_attachment_outside_the_folders_is_denied_and_audited_and_sends_nothing() {
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
    let elsewhere = temp_dir();
    let secret = elsewhere.join("passwoerter.txt");
    std::fs::write(&secret, b"geheim").unwrap();
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({
            "via": "m365-1", "to": "me", "subject": "x", "body": "y",
            "attach": [secret.to_string_lossy()]
        })),
    );
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert!(
        matches!(outcomes[0].1, RunOutcome::Failed { .. }),
        "{outcomes:?}"
    );
    assert_eq!(w.step_state(&run, "m"), StepState::Denied);
    assert!(w.graph.mails().is_empty());
    let audit = w.audit();
    let row = audit
        .iter()
        .find(|a| {
            a.outcome == "denied"
                && a.detail_json
                    .as_deref()
                    .unwrap_or("")
                    .contains("attachment_outside_allowed_folders")
        })
        .expect("Audit der Ablehnung");
    assert_eq!(row.caller, "workflow");
    assert_eq!(
        row.target.as_deref(),
        Some("passwoerter.txt"),
        "nur der Name, nie der Inhalt"
    );
}

#[test]
fn a_changed_attachment_breaks_the_approval() {
    let w = world();
    let path = doc_in(&w, "Protokoll.docx", b"erste Fassung");
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({
            "via": "m365-1", "to": "me", "subject": "x", "body": "y", "attach": [path.clone()]
        })),
    );
    let run = w.fire(&wf, "cal-a");
    assert_eq!(w.tick()[0].1, RunOutcome::AwaitingApproval);
    let pending = w.pending();
    // Der Nutzer sah „erste Fassung“; bis er klickt, wird die Datei ausgetauscht.
    std::fs::write(&path, b"ganz andere zweite Fassung").unwrap();
    approvals::decide(&w.conn(), &pending[0].id, true, T0).unwrap();
    let outcomes = w.tick();
    assert!(
        matches!(outcomes[0].1, RunOutcome::Failed { .. }),
        "{outcomes:?}"
    );
    assert_eq!(w.step_state(&run, "m"), StepState::Denied);
    assert!(
        w.graph.mails().is_empty(),
        "die genehmigte Fassung ist nicht die gesendete"
    );
}

/// Die Tor-Ansicht der Mail, wie die Engine sie bindet (ohne `lauf`).
fn bound_view(w: &World, action: &dyn Action, ctx: &Value, params: &Value) -> Value {
    action
        .gate_view(
            &GateEnv {
                conn: &w.conn(),
                context: ctx,
                planning: false,
            },
            params,
        )
        .unwrap()
        .unwrap()
        .args
}

#[test]
fn an_attachment_swapped_between_the_gate_and_the_send_is_refused_and_audited() {
    // B21 (QG5): Das Tor hat Fassung A gesehen und die Freigabe daran gebunden; bis der Baustein
    // die Datei fuer den Versand liest, wurde sie durch B ersetzt. Gesendet wird nichts.
    let w = world();
    let path = doc_in(&w, "Protokoll.docx", b"genehmigte Fassung");
    let action = action_of(&w, "mail.send");
    let ctx = json!({"trigger": w.trigger("cal-a"), "steps": {}});
    let params = json!({
        "via": "smtp-1", "to": "me", "subject": "x", "body": "y", "attach": [path.clone()]
    });
    let bound = bound_view(&w, &*action, &ctx, &params);
    assert_eq!(
        bound["attachments"][0]["sha256"].as_str().unwrap().len(),
        64,
        "die Freigabe bindet das vollstaendige SHA-256"
    );

    std::fs::write(&path, b"ausgetauschte Fassung").unwrap();
    let d = Direct::new(ctx);
    let r = d.run_bound(&w, &*action, "R1", "m", &params, Some(&bound));
    assert!(matches!(r, Err(StepError::Permanent(_))), "{r:?}");
    assert!(
        w.smtp.mails.lock().unwrap().is_empty(),
        "die genehmigte Fassung ist nicht die gesendete"
    );
    let audit = w.audit();
    let row = audit
        .iter()
        .find(|a| {
            a.outcome == "denied"
                && a.detail_json
                    .as_deref()
                    .unwrap_or("")
                    .contains("attachment_changed_after_approval")
        })
        .expect("Audit der Ablehnung");
    assert_eq!(row.caller, "workflow");
    assert_eq!(row.target.as_deref(), Some("Protokoll.docx"));
}

#[test]
fn the_bound_attachment_is_sent_byte_for_byte() {
    let w = world();
    let path = doc_in(&w, "Protokoll.docx", b"genehmigte Fassung");
    let action = action_of(&w, "mail.send");
    let ctx = json!({"trigger": w.trigger("cal-a"), "steps": {}});
    let params = json!({
        "via": "smtp-1", "to": "me", "subject": "x", "body": "y", "attach": [path]
    });
    let bound = bound_view(&w, &*action, &ctx, &params);
    let d = Direct::new(ctx);
    d.run_bound(&w, &*action, "R1", "m", &params, Some(&bound))
        .unwrap();
    let mails = w.smtp.mails.lock().unwrap();
    assert_eq!(mails.len(), 1);
    assert_eq!(smtp_attachment(&mails[0].data), b"genehmigte Fassung");
}

#[test]
fn a_swap_between_the_gate_check_and_the_send_in_a_real_run_is_never_sent() {
    use crate::managers::integrations::folder::race;
    // Derselbe Durchgang der Engine: Tor liest den Anhang (1. Lesen), der Baustein liest ihn fuer
    // den Versand (2. Lesen); dazwischen wird er gegen eine Fassung gleicher Laenge getauscht.
    let w = world();
    let path = doc_in(&w, "Protokoll.docx", b"genehmigte Fassung");
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({
            "via": "m365-1", "to": "me", "subject": "x", "body": "y", "attach": [path.clone()]
        })),
    );
    let run = w.fire(&wf, "cal-a");
    assert_eq!(w.tick()[0].1, RunOutcome::AwaitingApproval);
    let pending = w.pending();
    approvals::decide(&w.conn(), &pending[0].id, true, T0).unwrap();

    let reads = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let (seen, p2) = (reads.clone(), path.clone());
    race::install(move |stage| {
        if stage == "before_read" {
            seen.set(seen.get() + 1);
            if seen.get() == 2 {
                std::fs::write(&p2, b"GENEHMIGTE FASSUNG").unwrap();
            }
        }
    });
    let outcomes = w.tick();
    race::clear();
    assert_eq!(reads.get(), 2, "Tor und Baustein lesen den Anhang je einmal");
    assert!(
        matches!(outcomes[0].1, RunOutcome::Failed { .. }),
        "{outcomes:?}"
    );
    assert!(
        w.graph.mails().is_empty(),
        "die genehmigte Fassung ist nicht die gesendete"
    );
    assert!(w
        .detail(&run)
        .steps
        .iter()
        .any(|s| s.error.as_deref().unwrap_or("").contains("seit die Freigabe erteilt wurde")));
    assert!(w.audit().iter().any(|a| a.outcome == "denied"
        && a.detail_json
            .as_deref()
            .unwrap_or("")
            .contains("attachment_changed_after_approval")));
}

#[test]
fn an_attachment_from_a_folder_that_asks_makes_even_a_mail_to_me_ask() {
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
    w.grant("folder-protokolle", Capability::FilesRead, GrantMode::Ask);
    let path = doc_in(&w, "a.txt", b"inhalt");
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({
            "via": "m365-1", "to": "me", "subject": "x", "body": "y", "attach": path
        })),
    );
    let run = w.fire(&wf, "cal-a");
    assert_eq!(w.tick(), vec![(run, RunOutcome::AwaitingApproval)]);
    assert!(w.graph.mails().is_empty());
}

#[test]
fn a_missing_attachment_is_permanent_and_named() {
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({
            "via": "m365-1", "to": "me", "subject": "x", "body": "y",
            "attach": [w.docs.join("gibtsnicht.docx").to_string_lossy()]
        })),
    );
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert!(
        matches!(&outcomes[0].1, RunOutcome::Failed { code } if code == "permanent"),
        "{outcomes:?}"
    );
    assert!(w.detail(&run).steps.iter().any(|s| s
        .error
        .as_deref()
        .unwrap_or("")
        .contains("gibt es nicht")));
    assert!(w.graph.mails().is_empty());
}

#[cfg(windows)]
#[test]
fn a_locked_attachment_is_transient_and_sends_nothing() {
    use std::os::windows::fs::OpenOptionsExt;
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
    let path = doc_in(&w, "gesperrt.docx", b"inhalt");
    // Ein anderer Prozess haelt die Datei exklusiv (Word, Virenscanner): Lesen ist unmoeglich.
    let _lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({
            "via": "m365-1", "to": "me", "subject": "x", "body": "y", "attach": [path]
        })),
    );
    let run = w.fire(&wf, "cal-a");
    w.tick();
    assert_eq!(
        w.step_state(&run, "m"),
        StepState::Retrying,
        "nichts ist passiert: ein neuer Versuch ist sicher"
    );
    assert!(w.graph.mails().is_empty());
}

#[test]
fn attachments_must_be_a_path_or_a_list_of_paths() {
    let w = world();
    for bad in [json!(5), json!({"a": 1}), json!([1, 2]), json!([["x"]])] {
        let def = w.mail_only(json!({"via": "m365-1", "to": "me", "subject": "x", "attach": bad}));
        assert!(w.engine.save_workflow(None, &def).is_err(), "{bad}");
    }
    let ok = w.mail_only(
        json!({"via": "m365-1", "to": "me", "subject": "x", "attach": ["{{steps.a.path}}"]}),
    );
    // (steps.a gibt es hier nicht: der Katalog prueft Verweise, nicht den Baustein)
    assert!(w.engine.save_workflow(None, &ok).is_err());
}

// ---------------------------------------------------------------------------
// Speichern: Entwurf, Liste
// ---------------------------------------------------------------------------

#[test]
fn drafts_and_inconsistent_lists_are_refused_when_saving() {
    let w = world();
    let draft = w.mail_only(json!({"via": "m365-1", "to": "me", "subject": "x", "draft": true}));
    let e = w
        .engine
        .save_workflow(None, &draft)
        .unwrap_err()
        .to_string();
    assert!(e.contains("Entwürfe"), "{e}");
    let ok = w.mail_only(json!({"via": "m365-1", "to": "me", "subject": "x", "draft": false}));
    assert!(w.engine.save_workflow(None, &ok).is_ok());

    let list_without_rule =
        w.mail_only(json!({"via": "m365-1", "to": "me", "subject": "x", "list": ["a@x.de"]}));
    assert!(w.engine.save_workflow(None, &list_without_rule).is_err());
    let rule_without_list = w.mail_only(json!({"via": "m365-1", "to": "list", "subject": "x"}));
    assert!(w.engine.save_workflow(None, &rule_without_list).is_err());
    let bad_address = w.mail_only(
        json!({"via": "m365-1", "to": "list", "subject": "x", "list": ["a@x.de", "kein-mensch"]}),
    );
    let e = w
        .engine
        .save_workflow(None, &bad_address)
        .unwrap_err()
        .to_string();
    assert!(e.contains("kein-mensch"), "{e}");
    let good = w.mail_only(
        json!({"via": "m365-1", "to": "list", "subject": "x", "list": ["a@x.de", "b@y.de"]}),
    );
    assert!(w.engine.save_workflow(None, &good).is_ok());
}

#[test]
fn a_fixed_list_goes_to_exactly_that_list() {
    let w = world();
    w.grant("smtp-1", Capability::MailSend, GrantMode::Allow);
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({
            "via": "smtp-1", "to": "list", "list": ["kunde@x.de", "Chef@Y.de"],
            "subject": "Liste", "body": "Text", "auto": true
        })),
    );
    let run = w.fire(&wf, "cal-a");
    assert_eq!(w.tick(), vec![(run, RunOutcome::Done)]);
    let rcpt: Vec<String> = w.smtp.mails.lock().unwrap()[0]
        .rcpt
        .iter()
        .map(|r| r.trim_matches(['<', '>']).to_string())
        .collect();
    assert_eq!(rcpt, vec!["kunde@x.de", "Chef@Y.de"]);
}

#[test]
fn without_a_known_recipient_nothing_is_asked_and_nothing_is_sent() {
    let w = world();
    // Ohne Teilnehmende (Ausloeser ohne Termin) kann „participants“ niemanden bilden.
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({"via": "smtp-1", "to": "participants", "subject": "x", "body": "y"})),
    );
    let q = w
        .engine
        .enqueue(&EnqueueRequest {
            workflow_id: wf,
            trigger_key: "manuell-1".to_string(),
            origin: Origin::Trigger,
            trigger: json!({}),
            vars: serde_json::Map::new(),
            force_dry_run: false,
        })
        .unwrap();
    let outcomes = w.tick();
    assert!(
        matches!(&outcomes[0].1, RunOutcome::Failed { code } if code == "permanent"),
        "{outcomes:?}"
    );
    assert!(
        w.pending().is_empty(),
        "keine Freigabe fuer etwas, das es nicht gibt"
    );
    assert!(w.smtp.mails.lock().unwrap().is_empty());
    let d = w.detail(&q.run_id);
    assert!(d.steps[0]
        .error
        .as_deref()
        .unwrap()
        .contains("keine Teilnehmenden"));
}

#[test]
fn me_falls_back_to_the_senders_address_and_never_to_nothing() {
    let w = world();
    // Keine Einstellung, kein Kalender-Kennzeichen: bei SMTP ist der Absender „ich“.
    // (Die Bausteine der Engine halten einen Verweis auf die Dienste, daher neu einhaengen.)
    let svc = Arc::new(Svc {
        store: w.svc.store.clone(),
        m365: w.svc.m365.clone(),
        self_emails: vec![],
        secrets: Mutex::new(HashMap::new()),
        unavailable: UnavailableServices,
    });
    install(&w.engine, svc);
    w.grant("smtp-1", Capability::MailSend, GrantMode::Allow);
    let wf = armed_workflow(
        &w.engine,
        &w.mail_only(json!({"via": "smtp-1", "to": "me", "subject": "x", "body": "y"})),
    );
    let mut trigger = w.trigger("cal-a");
    trigger["attendees"] = json!([{"email": "anna@kunde.de", "is_self": false}]);
    let q = w
        .engine
        .enqueue(&EnqueueRequest {
            workflow_id: wf,
            trigger_key: "k".to_string(),
            origin: Origin::Trigger,
            trigger,
            vars: serde_json::Map::new(),
            force_dry_run: false,
        })
        .unwrap();
    assert_eq!(w.tick(), vec![(q.run_id, RunOutcome::Done)]);
    let rcpt = w.smtp.mails.lock().unwrap()[0].rcpt.clone();
    assert_eq!(rcpt.len(), 1);
    assert!(rcpt[0].contains("ich@wolff.de"), "{rcpt:?}");
}

// ---------------------------------------------------------------------------
// Trockenlauf
// ---------------------------------------------------------------------------

#[test]
fn the_dry_run_shows_the_real_recipients_and_the_cap_without_sending() {
    let w = world();
    w.grant("m365-1", Capability::MailSend, GrantMode::Allow);
    let def = w.template("cal-b", "m365-1", "alle");
    let parsed = crate::managers::workflows::validate::parse_definition(&def).unwrap();
    let plan = crate::managers::workflows::plan::plan_definition(
        &w.conn(),
        &w.engine.registry(),
        &parsed,
        None,
    );
    let steps = plan["steps"].as_array().unwrap();
    let ich = steps.iter().find(|s| s["id"] == "mail_ich").unwrap();
    assert_eq!(
        ich["status"], "skipped",
        "empfaenger=alle: „Mail an mich“ entfaellt"
    );
    let alle = steps.iter().find(|s| s["id"] == "mail_alle").unwrap();
    assert_eq!(alle["effect_kind"], "external");
    assert_eq!(alle["permission"]["mode"], "ask", "{alle}");
    assert_eq!(
        alle["permission"]["capped_to"], "ask",
        "erlaubt, aber an Dritte: Freigabe"
    );
    assert_eq!(alle["permission"]["result"], "needs_approval");
    let preview = alle["permission"]["preview"].as_str().unwrap();
    assert!(
        preview.contains("ich@beispiel.invalid") && preview.contains("kunde@beispiel.invalid"),
        "{preview}"
    );
    assert!(
        preview.contains("Pfad wird beim Lauf eingesetzt"),
        "{preview}"
    );
    assert!(w.graph.mails().is_empty() && w.smtp.mails.lock().unwrap().is_empty());
    assert!(
        w.pending().is_empty(),
        "der Trockenlauf legt keine Freigabe an"
    );
    assert!(w.audit().is_empty(), "und schreibt nichts ins Audit");
}

#[test]
fn the_shipped_template_is_valid_and_names_every_block_it_uses() {
    let w = world();
    let def = w.template("cal-a", "m365-1", "ich");
    let row = w.engine.save_workflow(None, &def).unwrap();
    assert!(!row.id.is_empty());
    let parsed: Value = serde_json::from_str(templates::TERMIN_MAIL).unwrap();
    let actions: Vec<&str> = parsed["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        vec![
            "recording.start",
            "meeting.minutes",
            "export.document",
            "mail.send",
            "mail.send"
        ]
    );
}

// ---------------------------------------------------------------------------
// calendar.note
// ---------------------------------------------------------------------------

fn note_workflow(w: &World, text: &str) -> String {
    armed_workflow(
        &w.engine,
        &json!({
            "schema": "lva-workflow@1",
            "name": "Notiz",
            "trigger": {"type": "calendar.event_ended", "integration": "cal-a"},
            "steps": [step("n", "calendar.note", json!({"via": "m365-1", "text": text}))]
        }),
    )
}

#[test]
fn a_note_is_written_into_the_event_of_the_trigger_after_the_approval() {
    let w = world();
    let wf = note_workflow(&w, "Beschluss: Go-Live am 30. Oktober.");
    let run = w.fire(&wf, "cal-a");
    assert_eq!(w.tick()[0].1, RunOutcome::AwaitingApproval);
    let p = w.pending();
    assert_eq!(p[0].tool_or_capability, "calendar.write");
    let preview = p[0].args_preview.clone().unwrap();
    assert!(
        preview.contains(&format!("Ziel: Termin „{TITLE}“, 05.10.2026 08:00 UTC")),
        "{preview}"
    );
    assert!(preview.contains("Go-Live am 30. Oktober"), "{preview}");
    assert!(
        w.graph.patches.lock().unwrap().is_empty(),
        "vor der Freigabe wird nichts geschrieben"
    );
    approvals::decide(&w.conn(), &p[0].id, true, T0).unwrap();
    assert_eq!(w.tick(), vec![(run.clone(), RunOutcome::Done)]);
    let patches = w.graph.patches.lock().unwrap().clone();
    assert_eq!(patches.len(), 1);
    let sent = patches[0].json();
    let content = sent["body"]["content"].as_str().unwrap();
    assert!(
        content.starts_with("<html><body><p>Einladung</p>"),
        "{content}"
    );
    assert!(content.contains("Go-Live am 30. Oktober"));
    assert!(content.contains("<!--lva-note:"));
    assert_eq!(patches[0].header("if-match"), Some("W/\"abc\""));
    assert_eq!(w.step_output(&run, "n")["added"], true);
}

#[test]
fn a_note_that_is_already_there_is_not_added_twice_and_a_repeat_step_does_not_write_again() {
    let w = world();
    w.grant("m365-1", Capability::CalendarWrite, GrantMode::Allow);
    let d = Direct::new(json!({"trigger": w.trigger("cal-a"), "steps": {}}));
    let params = json!({"via": "m365-1", "text": "Einmal."});
    let action = action_of(&w, "calendar.note");
    let first = d.run(&w, &*action, "R1", "n", &params).unwrap();
    assert_eq!(first.data["added"], true);
    // Dieselbe Notiz unter anderer Kennung: der Marker im Termin verhindert die Dublette.
    let second = d.run(&w, &*action, "R1", "n2", &params).unwrap();
    assert_eq!(second.data["already_there"], true);
    assert_eq!(w.graph.patches.lock().unwrap().len(), 1);
    // Wiederholung desselben Schritts: nicht einmal eine Anfrage.
    let calls_before = w.graph.seen.lock().unwrap().len();
    let again = d.run(&w, &*action, "R1", "n", &params).unwrap();
    assert!(again.summary.unwrap().contains("schon im Termin"));
    assert_eq!(
        w.graph.seen.lock().unwrap().len(),
        calls_before,
        "keine neue Anfrage"
    );
}

#[test]
fn a_note_without_an_event_or_with_a_forged_event_id_fails_clearly() {
    let w = world();
    let action = action_of(&w, "calendar.note");
    let d = Direct::new(json!({"trigger": {}, "steps": {}}));
    let r = d.run(
        &w,
        &*action,
        "R1",
        "n",
        &json!({"via": "m365-1", "text": "x"}),
    );
    assert!(
        matches!(&r, Err(StepError::Permanent(m)) if m.contains("Der Termin fehlt")),
        "{r:?}"
    );
    for bad in ["kein-schluessel", "a:b", "a:b:abc", "a::123"] {
        let r = d.run(
            &w,
            &*action,
            "R1",
            "n",
            &json!({"via": "m365-1", "text": "x", "event": bad}),
        );
        assert!(matches!(&r, Err(StepError::Permanent(_))), "{bad}: {r:?}");
    }
    let d = Direct::new(json!({"trigger": w.trigger("cal-a"), "steps": {}}));
    let r = d.run(
        &w,
        &*action,
        "R1",
        "n",
        &json!({"via": "m365-1", "text": "   "}),
    );
    assert!(
        matches!(&r, Err(StepError::Permanent(m)) if m.contains("leer")),
        "{r:?}"
    );
    let r = d.run(
        &w,
        &*action,
        "R1",
        "n",
        &json!({"via": "smtp-1", "text": "x"}),
    );
    assert!(
        matches!(&r, Err(StepError::Permanent(m)) if m.contains("kein Microsoft-365-Konto")),
        "{r:?}"
    );
    assert!(w.graph.patches.lock().unwrap().is_empty());
}

#[test]
fn a_note_that_the_organizer_forbids_is_permanent_and_one_lost_after_sending_is_unknown() {
    let w = world();
    w.grant("m365-1", Capability::CalendarWrite, GrantMode::Allow);
    let wf = note_workflow(&w, "Notiz.");
    *w.graph.mode.lock().unwrap() = GraphMode::Forbidden;
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert!(
        matches!(&outcomes[0].1, RunOutcome::Failed { code } if code == "permanent"),
        "{outcomes:?}"
    );
    assert!(w.detail(&run).steps[0]
        .error
        .as_deref()
        .unwrap()
        .contains("Organisator"));

    let w = world();
    w.grant("m365-1", Capability::CalendarWrite, GrantMode::Allow);
    let wf = note_workflow(&w, "Notiz.");
    *w.graph.mode.lock().unwrap() = GraphMode::Drop;
    let run = w.fire(&wf, "cal-a");
    let outcomes = w.tick();
    assert!(
        matches!(&outcomes[0].1, RunOutcome::Failed { code } if code == "effect_uncertain"),
        "{outcomes:?}"
    );
    assert_eq!(w.step_state(&run, "n"), StepState::Uncertain);
}

#[test]
fn a_note_off_is_denied_and_audited_like_every_other_right() {
    let w = world();
    w.grant("m365-1", Capability::CalendarWrite, GrantMode::Off);
    let wf = note_workflow(&w, "Notiz.");
    let run = w.fire(&wf, "cal-a");
    assert!(matches!(w.tick()[0].1, RunOutcome::Failed { .. }));
    assert_eq!(w.step_state(&run, "n"), StepState::Denied);
    assert!(w
        .audit()
        .iter()
        .any(|a| a.outcome == "denied" && a.capability.as_deref() == Some("calendar.write")));
    assert!(w.graph.patches.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// webhook.post
// ---------------------------------------------------------------------------

fn hook_workflow(w: &World, body: Value) -> String {
    armed_workflow(
        &w.engine,
        &json!({
            "schema": "lva-workflow@1",
            "name": "n8n",
            "trigger": {"type": "calendar.event_ended", "integration": "cal-a"},
            "steps": [step("hook", "webhook.post", json!({"via": "hook-1", "body": body}))]
        }),
    )
}

#[test]
fn a_webhook_gets_the_payload_after_the_approval_and_the_reply_comes_back() {
    let w = world();
    let wf = hook_workflow(&w, json!({"titel": "{{trigger.title}}", "anzahl": 3}));
    let run = w.fire(&wf, "cal-a");
    assert_eq!(w.tick()[0].1, RunOutcome::AwaitingApproval);
    assert!(
        w.hook.seen.lock().unwrap().is_empty(),
        "vor der Freigabe nichts senden"
    );
    let p = w.pending();
    assert_eq!(p[0].tool_or_capability, "webhook.post");
    let preview = p[0].args_preview.clone().unwrap();
    assert!(
        preview.contains("Webhook „n8n Protokolle“ (127.0.0.1:"),
        "{preview}"
    );
    assert!(preview.contains(TITLE), "{preview}");
    assert!(
        !preview.contains("GEHEIM") && !preview.contains("sig=zzz"),
        "die Adresse ist ein Geheimnis: {preview}"
    );
    approvals::decide(&w.conn(), &p[0].id, true, T0).unwrap();
    assert_eq!(w.tick(), vec![(run.clone(), RunOutcome::Done)]);

    let seen = w.hook.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    // Der Pfad der Adresse kommt am Server an (er braucht ihn), nirgends sonst.
    assert!(
        seen[0]
            .head
            .starts_with("POST /webhook/GEHEIM-pfad-4711?sig=zzz "),
        "{}",
        seen[0].head
    );
    assert!(seen[0]
        .head
        .to_ascii_lowercase()
        .contains(&format!("idempotency-key: {run}:hook").to_ascii_lowercase()));
    let payload: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(payload["schema"], "lva-webhook@1");
    assert_eq!(payload["run"], run);
    assert_eq!(payload["step"], "hook");
    assert_eq!(payload["workflow"]["name"], "n8n");
    assert_eq!(payload["body"]["titel"], TITLE);
    assert_eq!(payload["body"]["anzahl"], 3);

    // Die Antwort steht fuer spaetere Schritte im Ergebnis; sie ist begrenzt.
    let out = w.step_output(&run, "hook");
    assert_eq!(out["ok"], true);
    assert_eq!(out["status"], 200);
    assert_eq!(out["json"]["ticket"], "T-42");
    assert!(out["text"].as_str().unwrap().contains("T-42"));
    assert_eq!(out["truncated"], false);
}

#[test]
fn the_webhook_payload_carries_no_secrets_and_the_address_stays_secret() {
    let w = world();
    w.grant("hook-1", Capability::WebhookPost, GrantMode::Allow);
    let wf = hook_workflow(
        &w,
        json!({
            "token": "super-geheimer-token",
            "api_key": "abc123",
            "notiz": "Zugang: Authorization: Bearer abcdefghijklmnop und password=hunter22",
            "adresse": "https://user:pw@example.org/x",
            "text": "Alles andere bleibt."
        }),
    );
    let run = w.fire(&wf, "cal-a");
    assert_eq!(w.tick(), vec![(run.clone(), RunOutcome::Done)]);
    let raw = w.hook.seen.lock().unwrap()[0].body.clone();
    for secret in [
        "super-geheimer-token",
        "abc123",
        "abcdefghijklmnop",
        "hunter22",
        "pw@",
    ] {
        assert!(!raw.contains(secret), "{secret} steht im Payload: {raw}");
    }
    let payload: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(payload["body"]["token"], "***");
    assert_eq!(payload["body"]["text"], "Alles andere bleibt.");

    // Die Adresse (Pfad und Schluessel) steht nirgends im Register ausser im Geheimnisspeicher.
    let conn = w.conn();
    for table_sql in [
        "SELECT group_concat(COALESCE(config_json,''), ' ') FROM integrations",
        "SELECT group_concat(COALESCE(detail_json,'') || COALESCE(target,''), ' ') FROM audit_log",
        "SELECT group_concat(COALESCE(args_preview,''), ' ') FROM approvals",
        "SELECT group_concat(COALESCE(output_json,'') || COALESCE(input_json,'') || COALESCE(error,''), ' ') FROM workflow_run_steps",
        "SELECT group_concat(COALESCE(context_json,'') || COALESCE(error,''), ' ') FROM workflow_runs",
        "SELECT group_concat(COALESCE(params_json,''), ' ') FROM provenance",
    ] {
        let text: Option<String> = conn
            .query_row(table_sql, [], |r| r.get(0))
            .unwrap_or_else(|e| panic!("{table_sql}: {e}"));
        let text = text.unwrap_or_default();
        assert!(
            !text.contains("GEHEIM-pfad") && !text.contains("sig=zzz"),
            "Adresse in {table_sql}: {text}"
        );
    }
    assert!(
        w.secret_url.contains("GEHEIM"),
        "der Test prueft gegen die echte Adresse"
    );
}

#[test]
fn webhook_outcomes_are_classified_by_what_they_say_about_delivery() {
    let cases: [(u16, StepState, Option<&str>); 4] = [
        (404, StepState::Failed, Some("permanent")),
        (429, StepState::Retrying, None),
        (500, StepState::Uncertain, Some("effect_uncertain")),
        (302, StepState::Failed, Some("permanent")),
    ];
    for (status, state, code) in cases {
        let w = world();
        w.grant("hook-1", Capability::WebhookPost, GrantMode::Allow);
        *w.hook.reply.lock().unwrap() = if status == 302 {
            http_reply(302, &[("Location", "http://127.0.0.1:1/anderswo")], b"")
        } else {
            http_reply(status, &[], b"nein")
        };
        let wf = hook_workflow(&w, json!({"a": 1}));
        let run = w.fire(&wf, "cal-a");
        w.tick();
        assert_eq!(w.step_state(&run, "hook"), state, "HTTP {status}");
        assert_eq!(
            w.detail(&run).run.error_code.as_deref(),
            code,
            "HTTP {status}"
        );
        let calls = w.hook.seen.lock().unwrap().len();
        if code == Some("effect_uncertain") {
            for _ in 0..3 {
                w.clock.advance(3_600_000);
                w.tick();
            }
            assert_eq!(
                w.hook.seen.lock().unwrap().len(),
                calls,
                "nie von selbst ein zweites Mal"
            );
        }
    }
}

#[test]
fn an_unreachable_webhook_is_transient_and_a_repeated_step_does_not_post_again() {
    let w = world();
    // Erreichbar: ein Aufruf, dann Wiederholung ohne neuen Aufruf.
    let d = Direct::new(
        json!({"trigger": w.trigger("cal-a"), "steps": {}, "workflow": {"name": "n8n"}}),
    );
    let action = action_of(&w, "webhook.post");
    let params = json!({"via": "hook-1", "body": {"a": 1}});
    d.run(&w, &*action, "R1", "hook", &params).unwrap();
    let again = d.run(&w, &*action, "R1", "hook", &params).unwrap();
    assert_eq!(again.data["reused"], true);
    assert_eq!(w.hook.seen.lock().unwrap().len(), 1);

    // Nicht erreichbar: nichts angekommen -> Transient.
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    w.svc.put_secret(
        "hook-1",
        "url",
        &format!("http://127.0.0.1:{dead}/webhook/GEHEIM-2"),
    );
    let r = d.run(&w, &*action, "R2", "hook", &params);
    assert!(
        matches!(&r, Err(StepError::Transient(m)) if !m.contains("GEHEIM")),
        "{r:?}"
    );
}

#[test]
fn a_webhook_without_an_address_or_with_a_bad_one_fails_before_any_connection() {
    let w = world();
    let d = Direct::new(json!({"trigger": {}, "steps": {}}));
    let action = action_of(&w, "webhook.post");
    let params = json!({"via": "hook-1"});
    // Geheimnis fehlt.
    w.svc.secrets.lock().unwrap().clear();
    let r = d.run(&w, &*action, "R1", "h", &params);
    assert!(
        matches!(&r, Err(StepError::Permanent(m)) if m.contains("Adresse")),
        "{r:?}"
    );
    // http ausserhalb von Loopback, Zugangsdaten, falsches Schema.
    for bad in [
        "http://n8n.example.org/webhook/x",
        "https://u:p@n8n.example.org/x",
        "ftp://x/y",
        "kaputt",
    ] {
        w.svc.put_secret("hook-1", "url", bad);
        let r = d.run(&w, &*action, "R1", "h", &params);
        assert!(matches!(&r, Err(StepError::Permanent(_))), "{bad}: {r:?}");
    }
    assert!(w.hook.seen.lock().unwrap().is_empty());
    // Eine Integration, die kein Webhook ist, und eine unbekannte.
    let r = d.run(&w, &*action, "R1", "h", &json!({"via": "smtp-1"}));
    assert!(
        matches!(&r, Err(StepError::Permanent(m)) if m.contains("kein Webhook")),
        "{r:?}"
    );
    let r = d.run(&w, &*action, "R1", "h", &json!({"via": "gibtsnicht"}));
    assert!(
        matches!(&r, Err(StepError::Permanent(m)) if m.contains("gibt es nicht")),
        "{r:?}"
    );
}

#[test]
fn a_silent_webhook_ends_as_unknown_not_as_failed() {
    let w = world();
    w.grant("hook-1", Capability::WebhookPost, GrantMode::Allow);
    w.hook.silent.store(true, Ordering::SeqCst);
    // Kurzes Zeitlimit gibt es im Baustein nicht einstellbar; der Test ruft deshalb das Ziel
    // direkt mit kurzer Frist auf und prueft die Einordnung des Bausteins.
    let url = webhook::parse_url(&w.secret_url).unwrap();
    let err = webhook::post(
        &url,
        &json!({}),
        "k",
        &webhook::HttpOpts {
            connect_timeout: Duration::from_millis(500),
            timeout: Duration::from_millis(700),
            max_response_bytes: 1024,
        },
    )
    .unwrap_err();
    assert!(matches!(webhook_error(err), StepError::Unknown(_)));
}

#[test]
fn a_webhook_that_is_off_is_denied_and_audited_without_a_call() {
    let w = world();
    w.grant("hook-1", Capability::WebhookPost, GrantMode::Off);
    let wf = hook_workflow(&w, json!({"a": 1}));
    let run = w.fire(&wf, "cal-a");
    assert!(matches!(w.tick()[0].1, RunOutcome::Failed { .. }));
    assert_eq!(w.step_state(&run, "hook"), StepState::Denied);
    assert!(w.hook.seen.lock().unwrap().is_empty());
    assert!(w.audit().iter().any(|a| a.outcome == "denied"
        && a.capability.as_deref() == Some("webhook.post")
        && a.integration_id.as_deref() == Some("hook-1")));
}

#[test]
fn a_destination_cannot_be_chosen_by_data() {
    let w = world();
    for via in ["{{trigger.title}}", "{{vars.ziel}}"] {
        let def = json!({
            "schema": "lva-workflow@1", "name": "x",
            "trigger": {"type": "manual"},
            "steps": [step("h", "webhook.post", json!({"via": via}))]
        });
        assert!(w.engine.save_workflow(None, &def).is_err(), "{via}");
    }
    // Die alte Form mit einer Adresse im Ablauf gibt es nicht mehr.
    let def = json!({
        "schema": "lva-workflow@1", "name": "x",
        "trigger": {"type": "manual"},
        "steps": [step("h", "webhook.post", json!({"url": "https://evil.example/x"}))]
    });
    assert!(w.engine.save_workflow(None, &def).is_err());
}

#[test]
fn the_reply_of_a_webhook_is_clipped_and_stays_a_text() {
    let reply = webhook::WebhookReply {
        status: 200,
        content_type: "text/plain".to_string(),
        body: format!("Zeile\n{}\u{7}ende", "x".repeat(10_000)),
        truncated: false,
        bytes: 10_010,
    };
    let out = reply_output(&reply, false);
    let text = out.data["text"].as_str().unwrap();
    assert!(text.chars().count() <= MAX_REPLY_TEXT_CHARS);
    assert!(!text.contains('\n') && !text.contains('\u{7}'));
    assert!(out.data.get("json").is_none(), "kein JSON: kein Feld");
    let big_json = webhook::WebhookReply {
        body: format!("{{\"a\":\"{}\"}}", "y".repeat(MAX_REPLY_JSON_BYTES + 10)),
        ..reply.clone()
    };
    assert!(
        reply_output(&big_json, false).data.get("json").is_none(),
        "zu gross fuers Ergebnis"
    );
    let cut = webhook::WebhookReply {
        truncated: true,
        body: "{\"a\":1}".to_string(),
        ..reply
    };
    assert!(
        reply_output(&cut, false).data.get("json").is_none(),
        "abgeschnitten: nie als JSON"
    );
}

// ---------------------------------------------------------------------------
// Katalog und Einhaengen
// ---------------------------------------------------------------------------

#[test]
fn every_block_has_a_catalog_entry_the_right_effect_and_is_registered() {
    let w = world();
    for (id, effect) in [
        ("mail.send", EffectKind::External),
        ("calendar.note", EffectKind::External),
        ("webhook.post", EffectKind::External),
    ] {
        let a = action_of(&w, id);
        assert_eq!(a.effect(), effect, "{id}");
        assert!(a.heavy(&json!({})).is_none(), "{id}: nichts Schweres");
        assert_eq!(a.id(), id);
        // Das Recht kommt aus dem Katalog: ohne `via` gibt es keins (nie ohne Recht ausfuehren).
        assert!(
            matches!(a.needs(&json!({})), Err(NeedsError::Invalid(_))),
            "{id}"
        );
        let needs = a
            .needs(&json!({"via": "x-1", "to": "me"}))
            .unwrap()
            .unwrap();
        assert_eq!(needs.integration_id, "x-1");
    }
    assert_eq!(
        action_of(&w, "mail.send")
            .needs(&json!({"via": "m"}))
            .unwrap()
            .unwrap()
            .capability,
        Capability::MailSend
    );
    assert_eq!(
        action_of(&w, "calendar.note")
            .needs(&json!({"via": "m"}))
            .unwrap()
            .unwrap()
            .capability,
        Capability::CalendarWrite
    );
    assert_eq!(
        action_of(&w, "webhook.post")
            .needs(&json!({"via": "m"}))
            .unwrap()
            .unwrap()
            .capability,
        Capability::WebhookPost
    );
}

#[test]
fn without_the_app_the_blocks_report_that_they_are_not_built_in() {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    install(&engine, Arc::new(UnavailableServices));
    {
        let conn = fx.conn();
        let mut n = NewIntegration::new(Kind::M365, "Konto");
        n.id = Some("m365-1".to_string());
        let cfg = M365Config::new(
            CLIENT,
            "common",
            &[Capability::MailSend],
            FilesMode::Full,
            "x",
        )
        .unwrap();
        n.config = cfg.to_json();
        integrations_store::create(&conn, &n, T0).unwrap();
        set_grant(&conn, "m365-1", Capability::MailSend, GrantMode::Allow);
    }
    let wf = armed_workflow(
        &engine,
        &json!({
            "schema": "lva-workflow@1", "name": "x",
            "trigger": {"type": "manual"},
            "steps": [step("m", "mail.send", json!({"via": "m365-1", "to": "list", "list": ["a@x.de"], "subject": "x", "auto": true}))]
        }),
    );
    let q = engine.enqueue(&EnqueueRequest::manual(&wf)).unwrap();
    let report = engine.tick().unwrap();
    assert!(
        matches!(&report.outcomes[0].1, RunOutcome::Failed { code } if code == "not_available"),
        "{report:?} / {}",
        q.run_id
    );
}
