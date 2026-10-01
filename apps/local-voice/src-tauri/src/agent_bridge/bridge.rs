//! Die Entscheidungen der Agentenbruecke: anmelden, Werkzeugliste, Aufruf, Stand einer
//! Freigabe. Alles synchron und ohne Wissen ueber Pipe oder Zeilen (die liefern `server`
//! und `pipe`); damit sind Rechte, Audit und Freigaben ohne Netzwerk testbar.
//!
//! Jede Aktion eines Zugangs geht durch das Tor aus A1 (`integrations::gate`): Recht
//! pruefen, Audit schreiben (fehlgeschlagen = Aktion unterbleibt), Freigabe anlegen,
//! ausfuehren, Ergebnis festhalten. Hier kommt dazu:
//!
//! - **Aufrufer** ist immer `agent_external`; das Recht des Zugangs fuer das Werkzeug
//!   (`agent_tool_grants`, fehlend = aus) geht als `tool_mode` ein, das Strengere gewinnt.
//! - **Ziel** der Aktion im Audit und in der Freigabe: `<werkzeug> · <Zugangsname> (<Kennung>)`.
//!   Es steckt im Hash der Freigabe: eine Genehmigung fuer Zugang A taugt nicht fuer Zugang B.
//! - **Argumente** werden kanonisiert (Schluessel sortiert), damit dieselbe Anfrage denselben
//!   Hash ergibt, egal wie der Agent sie serialisiert.
//! - **Jede Anfrage** laedt den Zugang neu (`refresh`): zurueckgezogen oder entfernt gilt sofort.
//! - **Grenzen** (`limits`): Aufrufe je Minute je Zugang; die erste Abweisung je Fenster steht im
//!   Audit, die folgenden nur im Zaehler.
//! - **Werkzeuge** laufen hinter `catch_unwind`; eine Panik wird zu `failed` und ein Audit-Fehler.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde_json::{json, Value};

use super::catalog::{self, CallContext, CatalogEntry, ToolRegistry, STATUS_TOOL};
use super::clients::{self, AgentClient, AuthError};
use super::limits::{Limit, RateLimiter};
use super::protocol::code;
use super::{Config, APP_NAME, MAX_RESULT_BYTES, PROTOCOL_VERSION};
use crate::managers::integrations::approvals;
use crate::managers::integrations::gate::{self, Decision, GateOutcome, Request};
use crate::managers::integrations::grants::{explain, OffReason};
use crate::managers::integrations::model::{
    ApprovalState, AuditOutcome, Caller, GrantMode, IntegrationError, NewAudit,
};
use crate::managers::integrations::{audit, store};
use crate::managers::meetings::store::MeetingStore;

/// Wer eine Verbindung benutzt (nach der Anmeldung).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientCtx {
    pub id: String,
    pub label: String,
    pub integration_id: String,
}

impl From<&AgentClient> for ClientCtx {
    fn from(c: &AgentClient) -> Self {
        Self {
            id: c.id.clone(),
            label: c.label.clone(),
            integration_id: c.integration_id.clone(),
        }
    }
}

/// Fehler mit Code (siehe `protocol::code`), Klartext und optionalen Angaben.
#[derive(Clone, Debug, PartialEq)]
pub struct BridgeError {
    pub code: &'static str,
    pub message: String,
    pub data: Option<Value>,
}

impl BridgeError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    /// Die Datenbank ist nicht benutzbar: es wurde NICHTS ausgefuehrt.
    pub fn store(e: &IntegrationError) -> Self {
        log::warn!("agent_bridge: Speicherfehler: {e}");
        Self::new(
            code::STORE_UNAVAILABLE,
            "Die Datenbank ist gerade nicht verfügbar oder der Vorgang ließ sich nicht protokollieren. Es wurde nichts ausgeführt.",
        )
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BridgeError {}

/// Naechster Schritt eines Aufrufs.
#[derive(Clone, Debug, PartialEq)]
pub enum CallStep {
    /// Ausgefuehrt (oder ein eingebautes Werkzeug beantwortet).
    Done(Value),
    /// Die App wartet auf den Nutzer: auf diese Freigabe warten.
    Wait(String),
}

/// Stand einer Freigabe aus Sicht des Zugangs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pending,
    Approved,
    Denied,
    Expired,
    Used,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Pending => "pending",
            Phase::Approved => "approved",
            Phase::Denied => "denied",
            Phase::Expired => "expired",
            Phase::Used => "used",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalInfo {
    pub approval_id: String,
    pub tool: String,
    pub phase: Phase,
    pub created_at: i64,
    pub decided_at: Option<i64>,
}

/// Wird gerufen, wenn fuer einen Aufruf eine Freigabe angelegt (oder eine offene wiederverwendet)
/// wurde, mit dem Namen des Werkzeugs. Die App zeigt damit ohne Wartezeit die Bitte um
/// Einwilligung zur Aufnahme (A8). Der Rueckruf entscheidet nie etwas: die Freigabe bleibt offen,
/// bis der Nutzer sie in der Oberflaeche entscheidet.
pub type ApprovalNotifier = Arc<dyn Fn(&str) + Send + Sync>;

pub struct Bridge {
    store: Arc<MeetingStore>,
    registry: ToolRegistry,
    cfg: Config,
    limiter: Mutex<RateLimiter>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    notifier: Option<ApprovalNotifier>,
}

/// Aufrufer-Kennzeichnung im Audit.
const CALLER: &str = "agent_external";

/// Das Recht eines Zugangs fuer ein Werkzeug: Obergrenze der Agent-Integration und Recht des
/// Zugangs, das Strengere gewinnt. Dieselbe Rechnung wie beim Aufruf (Liste und Oberflaeche
/// zeigen nie etwas anderes als das Tor entscheidet).
pub fn tool_rights(
    conn: &Connection,
    client: &AgentClient,
    entry: &CatalogEntry,
) -> Result<(GrantMode, Option<OffReason>), IntegrationError> {
    let Some(integration) = store::get(conn, &client.integration_id)? else {
        return Ok((GrantMode::Off, Some(OffReason::IntegrationDisabled)));
    };
    let grants = store::grants_for(conn, &integration.id)?;
    let tool_mode = clients::tool_mode(conn, &client.id, entry.name)?.unwrap_or(GrantMode::Off);
    Ok(explain(
        &integration,
        entry.capability,
        Caller::AgentExternal,
        &grants,
        Some(tool_mode),
    ))
}

/// Schluessel sortieren (rekursiv): derselbe Inhalt ergibt denselben Text.
fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), canonical(&map[k]))).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

impl Bridge {
    pub fn new(store: Arc<MeetingStore>, registry: ToolRegistry, cfg: Config) -> Self {
        Self {
            store,
            registry,
            cfg,
            limiter: Mutex::new(RateLimiter::new()),
            clock: Arc::new(|| chrono::Utc::now().timestamp_millis()),
            notifier: None,
        }
    }

    /// Meldet neue Freigaben an die App (siehe `ApprovalNotifier`).
    pub fn with_notifier(mut self, notifier: ApprovalNotifier) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// Ruft den Rueckruf; ein Absturz darin stoert den Aufruf nicht.
    fn notify_approval(&self, tool: &str) {
        if let Some(n) = &self.notifier {
            if catch_unwind(AssertUnwindSafe(|| n(tool))).is_err() {
                log::warn!("agent_bridge: Rueckruf fuer Freigaben ist abgestuerzt");
            }
        }
    }

    /// Eigene Uhr (Millisekunden UTC), vor allem fuer Tests.
    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    pub fn now_ms(&self) -> i64 {
        (self.clock)()
    }

    fn conn(&self) -> Result<Connection, BridgeError> {
        self.store
            .get_connection()
            .map_err(|e| BridgeError::store(&IntegrationError::Store(e.to_string())))
    }

    // --- Audit-Hilfen ------------------------------------------------------

    /// Verweigerung ohne Aktion (Anmeldung, Grenze, unbekanntes Werkzeug). Ein Fehler beim
    /// Schreiben ist hier nicht schlimm: es geschah ja nichts.
    fn audit_denied(
        &self,
        conn: &Connection,
        client: Option<&AgentClient>,
        reason: &str,
        target: Option<String>,
        extra: Value,
    ) {
        let mut detail = json!({ "reason": reason });
        if let (Some(d), Some(e)) = (detail.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                d.insert(k.clone(), v.clone());
            }
        }
        let entry = NewAudit {
            caller: CALLER.to_string(),
            integration_id: client.map(|c| c.integration_id.clone()),
            capability: None,
            target,
            outcome: AuditOutcome::Denied,
            detail: Some(detail),
        };
        if let Err(e) = audit::record_at(conn, &entry, self.now_ms()) {
            log::warn!("agent_bridge: Audit ({reason}) nicht geschrieben: {e}");
        }
    }

    /// Fuer die Pipe: eine abgewiesene Verbindung (falscher Benutzer, nicht pruefbar).
    pub fn audit_rejected_connection(&self, reason: &str, detail: Value) {
        if let Ok(conn) = self.conn() {
            self.audit_denied(&conn, None, reason, None, detail);
        }
    }

    fn client_target(client: &ClientCtx) -> String {
        format!("{} ({})", client.label, client.id)
    }

    // --- Anmelden ------------------------------------------------------------

    /// Stand ohne Anmeldung: nur das Noetigste (Lebenszeichen).
    pub fn anonymous_status(&self) -> Value {
        json!({
            "ok": true,
            "app": APP_NAME,
            "version": env!("CARGO_PKG_VERSION"),
            "protocol": PROTOCOL_VERSION,
            "authenticated": false,
        })
    }

    /// Meldet einen Zugang mit seinem Token an. Fehlversuche sind begrenzt und stehen im Audit.
    pub fn authenticate(&self, token: &str) -> Result<ClientCtx, BridgeError> {
        let now = self.now_ms();
        // Fehlversuche insgesamt begrenzen (die Datenbank wird bei Ueberschreitung nicht mehr
        // befragt): ein Raten oder Fluten kostet nichts.
        let conn = self.conn()?;
        match clients::authenticate(&conn, token) {
            Ok(client) => {
                if let Err(e) = clients::touch(&conn, &client.id, now) {
                    log::debug!("agent_bridge: last_used_at nicht geschrieben: {e}");
                }
                Ok(ClientCtx::from(&client))
            }
            Err(AuthError::Store(m)) => Err(BridgeError::store(&IntegrationError::Store(m))),
            Err(e) => {
                let limited = self.limiter.lock().map_or(Limit::Allowed, |mut l| {
                    l.hit("auth-failures", self.cfg.auth_failures_per_minute, now)
                });
                if let Limit::Limited { first } = limited {
                    if first {
                        self.audit_denied(&conn, None, "auth_rate_limited", None, json!({}));
                    }
                    return Err(BridgeError::new(
                        code::RATE_LIMITED,
                        "Zu viele fehlgeschlagene Anmeldungen. Bitte später erneut versuchen.",
                    ));
                }
                match e {
                    AuthError::Revoked(client) => {
                        self.audit_denied(
                            &conn,
                            Some(&client),
                            "token_revoked",
                            Some(format!("{} ({})", client.label, client.id)),
                            json!({}),
                        );
                        Err(BridgeError::new(
                            code::TOKEN_REVOKED,
                            "Dieser Zugang wurde zurückgezogen. In Local Voice AI unter Integrationen einen neuen Zugang anlegen.",
                        ))
                    }
                    _ => {
                        self.audit_denied(&conn, None, "token_invalid", None, json!({}));
                        Err(BridgeError::new(
                            code::TOKEN_INVALID,
                            "Das Token ist ungültig.",
                        ))
                    }
                }
            }
        }
    }

    /// Laedt den Zugang einer angemeldeten Verbindung neu: zurueckgezogen oder entfernt gilt
    /// sofort, auch mitten in einer Verbindung.
    pub fn refresh(&self, client_id: &str) -> Result<(ClientCtx, AgentClient), BridgeError> {
        let conn = self.conn()?;
        match clients::get(&conn, client_id) {
            Ok(Some(c)) if c.is_active() => Ok((ClientCtx::from(&c), c)),
            Ok(Some(c)) => {
                self.audit_denied(
                    &conn,
                    Some(&c),
                    "token_revoked",
                    Some(format!("{} ({})", c.label, c.id)),
                    json!({ "phase": "connected" }),
                );
                Err(BridgeError::new(
                    code::TOKEN_REVOKED,
                    "Dieser Zugang wurde zurückgezogen.",
                ))
            }
            Ok(None) => {
                self.audit_denied(&conn, None, "token_invalid", None, json!({ "phase": "connected" }));
                Err(BridgeError::new(
                    code::TOKEN_INVALID,
                    "Dieser Zugang gibt es nicht mehr.",
                ))
            }
            Err(e) => Err(BridgeError::store(&e)),
        }
    }

    /// Zaehlt eine Anfrage gegen die Grenzen des Zugangs.
    pub fn rate_check(&self, client: &ClientCtx, is_call: bool) -> Result<(), BridgeError> {
        let now = self.now_ms();
        let verdict = {
            let Ok(mut l) = self.limiter.lock() else {
                return Ok(());
            };
            let all = l.hit(
                &format!("req:{}", client.id),
                self.cfg.requests_per_minute,
                now,
            );
            if matches!(all, Limit::Allowed) && is_call {
                l.hit(&format!("call:{}", client.id), self.cfg.calls_per_minute, now)
            } else {
                all
            }
        };
        match verdict {
            Limit::Allowed => Ok(()),
            Limit::Limited { first } => {
                if first {
                    if let Ok(conn) = self.conn() {
                        let ac = clients::get(&conn, &client.id).ok().flatten();
                        self.audit_denied(
                            &conn,
                            ac.as_ref(),
                            "rate_limited",
                            Some(Self::client_target(client)),
                            json!({}),
                        );
                    }
                }
                Err(BridgeError::new(
                    code::RATE_LIMITED,
                    "Zu viele Anfragen in kurzer Zeit. Bitte kurz warten.",
                ))
            }
        }
    }

    /// Grenze fuer Anfragen ohne Anmeldung (nur `status`): gemeinsam fuer alle.
    pub fn rate_check_anonymous(&self) -> Result<(), BridgeError> {
        let now = self.now_ms();
        let verdict = self.limiter.lock().map_or(Limit::Allowed, |mut l| {
            l.hit("anonymous", self.cfg.requests_per_minute, now)
        });
        match verdict {
            Limit::Allowed => Ok(()),
            Limit::Limited { .. } => Err(BridgeError::new(
                code::RATE_LIMITED,
                "Zu viele Anfragen in kurzer Zeit. Bitte kurz warten.",
            )),
        }
    }

    // --- Status und Liste -----------------------------------------------------

    pub fn status(&self, client: &ClientCtx) -> Result<Value, BridgeError> {
        let conn = self.conn()?;
        let now = self.now_ms();
        let _ = approvals::expire_stale(&conn, now);
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_approvals l JOIN approvals a ON a.id = l.approval_id
                 WHERE l.client_id = ?1 AND a.state = 'pending' AND a.created_at > ?2",
                rusqlite::params![client.id, now - approvals::TTL_MS],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let tools = self.tools_list_with(&conn, client)?.len();
        let mut v = self.anonymous_status();
        v["authenticated"] = json!(true);
        v["client"] = json!({ "id": client.id, "label": client.label });
        v["pending_approvals"] = json!(pending);
        v["tools"] = json!(tools);
        Ok(v)
    }

    fn client_row(&self, conn: &Connection, client: &ClientCtx) -> Result<AgentClient, BridgeError> {
        match clients::get(conn, &client.id) {
            Ok(Some(c)) if c.is_active() => Ok(c),
            Ok(_) => Err(BridgeError::new(
                code::TOKEN_REVOKED,
                "Dieser Zugang ist nicht mehr gültig.",
            )),
            Err(e) => Err(BridgeError::store(&e)),
        }
    }

    /// Werkzeuge, die der Zugang jetzt benutzen darf: „aus“ und nicht ausfuehrbare Werkzeuge
    /// fehlen. Das eingebaute Statuswerkzeug ist immer da.
    pub fn tools_list(&self, client: &ClientCtx) -> Result<Vec<Value>, BridgeError> {
        let conn = self.conn()?;
        self.tools_list_with(&conn, client)
    }

    fn tools_list_with(&self, conn: &Connection, client: &ClientCtx) -> Result<Vec<Value>, BridgeError> {
        let row = self.client_row(conn, client)?;
        let mut out = vec![json!({
            "name": STATUS_TOOL,
            "title": "Stand einer Freigabe",
            "description": "Fragt den Stand einer Freigabe ab (pending, approved, denied, expired, used). Nach „approved“ das Werkzeug mit derselben approval_id und denselben Argumenten erneut aufrufen.",
            "inputSchema": {
                "type": "object",
                "properties": { "approval_id": { "type": "string" } },
                "required": ["approval_id"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
            "mode": "allow"
        })];
        for name in self.registry.names() {
            let (Some(entry), Some(spec)) = (catalog::find(&name), self.registry.spec(&name)) else {
                continue;
            };
            let (mode, _) = tool_rights(conn, &row, entry).map_err(|e| BridgeError::store(&e))?;
            if mode == GrantMode::Off {
                continue;
            }
            out.push(json!({
                "name": spec.name,
                "title": spec.title,
                "description": spec.description,
                "inputSchema": spec.input_schema,
                "annotations": catalog::annotations(entry),
                "mode": mode.as_str(),
            }));
        }
        Ok(out)
    }

    // --- Aufruf ------------------------------------------------------------------

    fn target_for(client: &ClientCtx, tool: &str) -> String {
        format!("{tool} · {}", Self::client_target(client))
    }

    /// Argumente als kanonisches Objekt (`null`/fehlend = `{}`).
    pub fn normalize_args(args: &Value) -> Result<Value, BridgeError> {
        match args {
            Value::Null => Ok(json!({})),
            Value::Object(_) => Ok(canonical(args)),
            _ => Err(BridgeError::new(
                code::BAD_REQUEST,
                "arguments muss ein Objekt sein.",
            )),
        }
    }

    /// Ruft ein Werkzeug (siehe Moduldoku). `approval_id`: der Agent kommt mit einer Freigabe
    /// zurueck, die er frueher erhalten hat.
    pub fn call(
        &self,
        client: &ClientCtx,
        name: &str,
        args: &Value,
        approval_id: Option<&str>,
    ) -> Result<CallStep, BridgeError> {
        if name == STATUS_TOOL {
            let id = args
                .get("approval_id")
                .and_then(Value::as_str)
                .ok_or_else(|| BridgeError::new(code::BAD_REQUEST, "approval_id fehlt."))?;
            return self.action_status(client, id).map(CallStep::Done);
        }
        let conn = self.conn()?;
        let row = self.client_row(&conn, client)?;
        let Some(entry) = catalog::find(name) else {
            self.audit_denied(
                &conn,
                Some(&row),
                "unknown_tool",
                Some(Self::target_for(client, &truncate_name(name))),
                json!({}),
            );
            return Err(BridgeError::new(
                code::UNKNOWN_TOOL,
                format!("Das Werkzeug „{}“ gibt es nicht.", truncate_name(name)),
            ));
        };
        let args = Self::normalize_args(args)?;

        if let Some(aid) = approval_id {
            return match self.approval_info(&conn, client, aid)? {
                info if info.tool != name => Err(BridgeError::new(
                    code::APPROVAL_NOT_FOUND,
                    "Diese Freigabe gehört nicht zu diesem Werkzeug.",
                )),
                info => match info.phase {
                    Phase::Pending => Ok(CallStep::Wait(info.approval_id)),
                    Phase::Approved => self
                        .run_approved(&conn, client, &row, entry, &args, aid)
                        .map(CallStep::Done),
                    other => Err(phase_error(other)),
                },
            };
        }

        let target = Self::target_for(client, name);
        let tool_mode = clients::tool_mode(&conn, &row.id, name)
            .map_err(|e| BridgeError::store(&e))?
            .unwrap_or(GrantMode::Off);
        let req = Request {
            caller: Caller::AgentExternal,
            integration_id: &row.integration_id,
            capability: entry.capability,
            target: Some(&target),
            args: Some(&args),
            tool_mode: Some(tool_mode),
        };
        let now = self.now_ms();
        match gate::check(&conn, &req, now).map_err(|e| BridgeError::store(&e))? {
            Decision::Denied { reason, code: c } => Err(denied_error(c, &reason)),
            Decision::NeedsApproval { approval_id } => {
                clients::link_approval(&conn, &approval_id, &row.id, name, now)
                    .map_err(|e| BridgeError::store(&e))?;
                self.notify_approval(name);
                Ok(CallStep::Wait(approval_id))
            }
            Decision::Allowed => {
                let Some(handler) = self.registry.handler(name) else {
                    self.audit_denied(
                        &conn,
                        Some(&row),
                        "tool_unavailable",
                        Some(target.clone()),
                        json!({}),
                    );
                    return Err(BridgeError::new(
                        code::TOOL_UNAVAILABLE,
                        format!("Das Werkzeug „{name}“ ist in dieser Version noch nicht verfügbar."),
                    ));
                };
                let ctx = CallContext {
                    client_id: row.id.clone(),
                    client_label: row.label.clone(),
                    approved: false,
                    now_ms: now,
                };
                match gate::run(&conn, &req, now, || {
                    run_handler(handler.as_ref(), &ctx, name, &args)
                })
                .map_err(|e| BridgeError::store(&e))?
                {
                    GateOutcome::Done(v) => Ok(CallStep::Done(v)),
                    GateOutcome::Failed(m) => Err(BridgeError::new(code::FAILED, m)),
                    GateOutcome::Denied { reason, code: c } => Err(denied_error(c, &reason)),
                    GateOutcome::Pending { approval_id } => {
                        clients::link_approval(&conn, &approval_id, &row.id, name, now)
                            .map_err(|e| BridgeError::store(&e))?;
                        self.notify_approval(name);
                        Ok(CallStep::Wait(approval_id))
                    }
                }
            }
        }
    }

    /// Fuehrt ein Werkzeug aus, nachdem der Nutzer die Freigabe erteilt hat (einmalig, an
    /// Zugang, Werkzeug und Argumente gebunden; das Recht wird erneut geprueft).
    pub fn call_approved(
        &self,
        client: &ClientCtx,
        name: &str,
        args: &Value,
        approval_id: &str,
    ) -> Result<Value, BridgeError> {
        let conn = self.conn()?;
        let row = self.client_row(&conn, client)?;
        let Some(entry) = catalog::find(name) else {
            return Err(BridgeError::new(code::UNKNOWN_TOOL, "Unbekanntes Werkzeug."));
        };
        let args = Self::normalize_args(args)?;
        self.run_approved(&conn, client, &row, entry, &args, approval_id)
    }

    fn run_approved(
        &self,
        conn: &Connection,
        client: &ClientCtx,
        row: &AgentClient,
        entry: &CatalogEntry,
        args: &Value,
        approval_id: &str,
    ) -> Result<Value, BridgeError> {
        let name = entry.name;
        // Gehoert die Freigabe diesem Zugang und diesem Werkzeug?
        let info = self.approval_info(conn, client, approval_id)?;
        if info.tool != name {
            return Err(BridgeError::new(
                code::APPROVAL_NOT_FOUND,
                "Diese Freigabe gehört nicht zu diesem Werkzeug.",
            ));
        }
        let Some(handler) = self.registry.handler(name) else {
            // Die Genehmigung bleibt unverbraucht.
            return Err(BridgeError::new(
                code::TOOL_UNAVAILABLE,
                format!("Das Werkzeug „{name}“ ist in dieser Version noch nicht verfügbar."),
            ));
        };
        let target = Self::target_for(client, name);
        let tool_mode = clients::tool_mode(conn, &row.id, name)
            .map_err(|e| BridgeError::store(&e))?
            .unwrap_or(GrantMode::Off);
        let req = Request {
            caller: Caller::AgentExternal,
            integration_id: &row.integration_id,
            capability: entry.capability,
            target: Some(&target),
            args: Some(args),
            tool_mode: Some(tool_mode),
        };
        let now = self.now_ms();
        let ctx = CallContext {
            client_id: row.id.clone(),
            client_label: row.label.clone(),
            approved: true,
            now_ms: now,
        };
        match gate::run_approved(conn, approval_id, &req, now, || {
            run_handler(handler.as_ref(), &ctx, name, args)
        })
        .map_err(|e| BridgeError::store(&e))?
        {
            GateOutcome::Done(v) => Ok(v),
            GateOutcome::Failed(m) => Err(BridgeError::new(code::FAILED, m)),
            GateOutcome::Denied { reason, code: c } => {
                // Ein Rennen (zwei Aufrufe, eine Genehmigung) erklaeren wir genauer.
                if c == "approval_not_granted" {
                    if let Ok(i) = self.approval_info(conn, client, approval_id) {
                        if i.phase != Phase::Approved {
                            return Err(phase_error(i.phase));
                        }
                    }
                }
                Err(denied_error(c, &reason))
            }
            GateOutcome::Pending { .. } => Err(BridgeError::new(
                code::APPROVAL_NOT_FOUND,
                "Die Freigabe ist nicht mehr gültig.",
            )),
        }
    }

    // --- Freigaben ------------------------------------------------------------------

    fn approval_info(
        &self,
        conn: &Connection,
        client: &ClientCtx,
        approval_id: &str,
    ) -> Result<ApprovalInfo, BridgeError> {
        let not_found = || {
            BridgeError::new(
                code::APPROVAL_NOT_FOUND,
                "Diese Freigabe gibt es nicht (oder sie gehört einem anderen Zugang).",
            )
        };
        let link = clients::approval_link(conn, approval_id).map_err(|e| BridgeError::store(&e))?;
        let Some((owner, tool)) = link else {
            return Err(not_found());
        };
        if owner != client.id {
            return Err(not_found());
        }
        let now = self.now_ms();
        let _ = approvals::expire_stale(conn, now);
        let Some(a) = approvals::get(conn, approval_id).map_err(|e| BridgeError::store(&e))? else {
            return Err(not_found());
        };
        let phase = match a.state {
            ApprovalState::Pending if a.created_at <= now - approvals::TTL_MS => Phase::Expired,
            ApprovalState::Pending => Phase::Pending,
            ApprovalState::Approved => Phase::Approved,
            ApprovalState::Denied => Phase::Denied,
            ApprovalState::Expired => Phase::Expired,
            ApprovalState::Used => Phase::Used,
        };
        Ok(ApprovalInfo {
            approval_id: a.id,
            tool,
            phase,
            created_at: a.created_at,
            decided_at: a.decided_at,
        })
    }

    /// Stand einer eigenen Freigabe (nur Lesen).
    pub fn approval_phase(
        &self,
        client: &ClientCtx,
        approval_id: &str,
    ) -> Result<ApprovalInfo, BridgeError> {
        let conn = self.conn()?;
        self.approval_info(&conn, client, approval_id)
    }

    /// Antwort fuer `approval/status` und das Werkzeug `get_action_status`.
    pub fn action_status(&self, client: &ClientCtx, approval_id: &str) -> Result<Value, BridgeError> {
        let info = self.approval_phase(client, approval_id)?;
        let hint = match info.phase {
            Phase::Pending => "Der Nutzer hat noch nicht entschieden. Später erneut abfragen.",
            Phase::Approved => "Freigegeben. Das Werkzeug mit derselben approval_id und denselben Argumenten erneut aufrufen, um es auszuführen.",
            Phase::Denied => "Der Nutzer hat abgelehnt.",
            Phase::Expired => "Die Freigabe ist abgelaufen. Das Werkzeug neu aufrufen.",
            Phase::Used => "Die Freigabe wurde bereits ausgeführt.",
        };
        Ok(json!({
            "approval_id": info.approval_id,
            "tool": info.tool,
            "state": info.phase.as_str(),
            "created_at": info.created_at,
            "decided_at": info.decided_at,
            "hint": hint,
        }))
    }
}

fn truncate_name(name: &str) -> String {
    name.chars().take(64).collect()
}

/// Antwort fuer eine Freigabe, die nicht (mehr) ausfuehrbar ist.
fn phase_error(phase: Phase) -> BridgeError {
    match phase {
        Phase::Denied => BridgeError::new(code::APPROVAL_DENIED, "Der Nutzer hat die Freigabe abgelehnt."),
        Phase::Expired => BridgeError::new(
            code::APPROVAL_EXPIRED,
            "Die Freigabe ist abgelaufen. Das Werkzeug bitte neu aufrufen.",
        ),
        Phase::Used => BridgeError::new(
            code::APPROVAL_USED,
            "Diese Freigabe wurde schon verwendet. Jede Freigabe gilt für genau eine Ausführung.",
        ),
        Phase::Pending | Phase::Approved => {
            BridgeError::new(code::APPROVAL_NOT_FOUND, "Die Freigabe ist nicht mehr gültig.")
        }
    }
}

/// Bildet die Ablehnung des Tors (A1) auf einen Fehlercode der Bruecke ab.
fn denied_error(gate_code: &str, reason: &str) -> BridgeError {
    let data = json!({ "reason": gate_code });
    match gate_code {
        "integration_disabled" | "capability_not_offered" | "direction_blocks" | "grant_off"
        | "tool_off" | "unknown_integration" => {
            BridgeError::new(code::TOOL_OFF, reason).with_data(data)
        }
        "approval_not_found" => BridgeError::new(code::APPROVAL_NOT_FOUND, reason).with_data(data),
        "approval_expired" => BridgeError::new(code::APPROVAL_EXPIRED, reason).with_data(data),
        "approval_mismatch" => BridgeError::new(
            code::APPROVAL_MISMATCH,
            "Die Freigabe gehört zu anderen Argumenten. Das Werkzeug mit den freigegebenen Argumenten aufrufen.",
        )
        .with_data(data),
        _ => BridgeError::new(code::DENIED, reason).with_data(data),
    }
}

/// Fuehrt den Handler aus: Panik wird zu einem Fehler, zu grosse Antworten werden ersetzt.
fn run_handler(
    handler: &dyn catalog::ToolHandler,
    ctx: &CallContext,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let outcome = catch_unwind(AssertUnwindSafe(|| handler.call(ctx, tool, args)));
    let value = match outcome {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            log::error!("agent_bridge: Werkzeug {tool} ist abgestürzt (Panik)");
            return Err("Interner Fehler im Werkzeug.".to_string());
        }
    };
    if value.to_string().len() > MAX_RESULT_BYTES {
        return Ok(json!({
            "truncated": true,
            "message": "Das Werkzeug wurde ausgeführt, aber die Antwort war zu groß und wird nicht übertragen."
        }));
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
