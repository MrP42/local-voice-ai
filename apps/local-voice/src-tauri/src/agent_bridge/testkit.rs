//! Gemeinsame Bausteine der Tests der Agentenbruecke (nur `cfg(test)`).

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{json, Value};

use super::bridge::{Bridge, ClientCtx};
use super::catalog::{self, CallContext, ToolHandler, ToolRegistry, ToolSpec};
use super::clients::{self, AgentClient};
use super::Config;
use crate::managers::integrations::model::{Caller, GrantMode};
use crate::managers::integrations::store;
use crate::managers::integrations::test_support::Fx;
use crate::managers::meetings::store::MeetingStore;

pub const NOW: i64 = 1_790_000_000_000;

/// Was ein Test-Werkzeug beim Aufruf tut.
#[derive(Clone, Debug)]
pub enum Behavior {
    Ok,
    Err(String),
    Panic,
    /// Antwort groesser als erlaubt.
    Huge,
}

/// Aufgezeichnete Aufrufe: (Werkzeug, Argumente, freigegeben).
#[derive(Clone, Default)]
pub struct Calls(pub Arc<Mutex<Vec<(String, Value, bool)>>>);

impl Calls {
    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    pub fn all(&self) -> Vec<(String, Value, bool)> {
        self.0.lock().unwrap().clone()
    }
}

pub struct TestHandler {
    pub calls: Calls,
    pub behavior: Arc<Mutex<Behavior>>,
    pub tools: Vec<&'static str>,
}

impl ToolHandler for TestHandler {
    fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .map(|n| {
                let e = catalog::find(n).expect("Katalogwerkzeug");
                ToolSpec {
                    name: n.to_string(),
                    title: e.title.to_string(),
                    description: e.description.to_string(),
                    input_schema: json!({"type": "object"}),
                }
            })
            .collect()
    }

    fn call(&self, ctx: &CallContext, tool: &str, args: &Value) -> Result<Value, String> {
        self.calls
            .0
            .lock()
            .unwrap()
            .push((tool.to_string(), args.clone(), ctx.approved));
        // Die Sperre vor einem Absturz loslassen, sonst vergiftet der Absturz sie.
        let behavior = self.behavior.lock().unwrap().clone();
        match behavior {
            Behavior::Ok => Ok(json!({"ran": tool, "client": ctx.client_label})),
            Behavior::Err(m) => Err(m),
            Behavior::Panic => panic!("absichtlicher Absturz im Test-Werkzeug"),
            Behavior::Huge => Ok(json!({"blob": "x".repeat(super::MAX_RESULT_BYTES + 10)})),
        }
    }
}

/// Alle Katalogwerkzeuge.
pub const ALL_TOOLS: [&str; 11] = [
    "add_youtube_source",
    "start_recording",
    "stop_recording",
    "transcribe_file",
    "create_session",
    "create_meeting",
    "tts_page_create",
    "tts_render_audio",
    "list_workflows",
    "run_workflow",
    "get_run",
];

pub struct Fixture {
    pub fx: Fx,
    pub bridge: Arc<Bridge>,
    pub calls: Calls,
    pub behavior: Arc<Mutex<Behavior>>,
    pub clock: Arc<AtomicI64>,
    pub client: AgentClient,
    pub token: String,
}

/// Kurze Zeiten fuer Tests (die Vorgaben dauern Sekunden bis Minuten).
pub fn fast_config() -> Config {
    Config {
        approval_wait: Duration::from_millis(400),
        poll_interval: Duration::from_millis(20),
        // Fristen sind lang genug, dass ein ausgelasteter Rechner sie nicht reisst; die Tests der
        // Fristen selbst setzen ihre eigenen kurzen Werte.
        handshake_timeout: Duration::from_secs(5),
        anonymous_idle_timeout: Duration::from_secs(5),
        idle_timeout: Duration::from_secs(5),
        ..Config::default()
    }
}

impl Fixture {
    pub fn new() -> Self {
        Self::with(fast_config(), &ALL_TOOLS)
    }

    pub fn with(cfg: Config, tools: &[&'static str]) -> Self {
        let fx = Fx::new();
        let calls = Calls::default();
        let behavior = Arc::new(Mutex::new(Behavior::Ok));
        let mut registry = ToolRegistry::new();
        registry
            .register(Arc::new(TestHandler {
                calls: calls.clone(),
                behavior: behavior.clone(),
                tools: tools.to_vec(),
            }))
            .unwrap();
        let clock = Arc::new(AtomicI64::new(NOW));
        let c2 = clock.clone();
        let bridge = Arc::new(
            Bridge::new(
                Arc::new(MeetingStore::open_at(&fx.db_path).unwrap()),
                registry,
                cfg,
            )
                .with_clock(Arc::new(move || c2.load(Ordering::SeqCst))),
        );
        let conn = fx.conn();
        let (client, token) = clients::create(&conn, "Claude Code", None, NOW).unwrap();
        Self {
            fx,
            bridge,
            calls,
            behavior,
            clock,
            client,
            token,
        }
    }

    pub fn conn(&self) -> Connection {
        self.fx.conn()
    }

    pub fn ctx(&self) -> ClientCtx {
        ClientCtx::from(&self.client)
    }

    pub fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    /// Setzt Obergrenze (Agent-Integration, Aufrufer `agent_external`) UND Recht des
    /// Zugangs fuer ein Werkzeug. Die Obergrenze ist `allow` (bei Aufnahme `ask`: nie erlaubt).
    pub fn grant(&self, tool: &str, mode: GrantMode) {
        self.grant_for(&self.client.id, tool, mode);
    }

    pub fn grant_for(&self, client_id: &str, tool: &str, mode: GrantMode) {
        let conn = self.conn();
        let entry = catalog::find(tool).unwrap();
        let ceiling = if entry.capability.never_allow() {
            GrantMode::Ask
        } else {
            GrantMode::Allow
        };
        let client = clients::get(&conn, client_id).unwrap().unwrap();
        store::set_grant(&conn, &client.integration_id, entry.capability, Caller::AgentExternal, ceiling)
            .unwrap();
        clients::set_tool_mode(&conn, client_id, tool, mode).unwrap();
    }

    /// Nur die Obergrenze der Integration.
    pub fn ceiling(&self, tool: &str, mode: GrantMode) {
        let conn = self.conn();
        let entry = catalog::find(tool).unwrap();
        store::set_grant(&conn, &self.client.integration_id, entry.capability, Caller::AgentExternal, mode)
            .unwrap();
    }

    pub fn second_client(&self, label: &str) -> (ClientCtx, String) {
        let conn = self.conn();
        let (c, t) = clients::create(&conn, label, None, self.now()).unwrap();
        (ClientCtx::from(&c), t)
    }

    /// Der Nutzer entscheidet in der Oberflaeche.
    pub fn user_decides(&self, approval_id: &str, approve: bool) {
        let conn = self.conn();
        crate::managers::integrations::approvals::decide(&conn, approval_id, approve, self.now())
            .unwrap();
    }

    pub fn audit(&self) -> Vec<crate::managers::integrations::model::AuditEntry> {
        let conn = self.conn();
        let mut rows = crate::managers::integrations::audit::list(
            &conn,
            &crate::managers::integrations::audit::AuditFilter::default(),
            500,
        )
        .unwrap();
        rows.reverse();
        rows
    }

    pub fn count(&self, sql: &str) -> i64 {
        self.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}
