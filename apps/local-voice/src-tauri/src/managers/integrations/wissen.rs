//! WAI-Wissensbasis (A6, Goal „Integrationen“): `wissen_suchen` ueber den
//! MCP-Endpunkt des AI-OS (Streamable HTTP, `POST <endpunkt>`, Bearer-Schluessel mit
//! Scope `wissen:read`).
//!
//! Der Client spricht JSON-RPC 2.0 mit `initialize` -> `notifications/initialized`
//! -> `tools/call` (Antwort als JSON oder als Ereignisstrom `text/event-stream`).
//! Der AI-OS-Server arbeitet ohne Sitzung (`stateless_http`); liefert ein anderer
//! Server eine `Mcp-Session-Id`, wird sie mitgefuehrt und am Ende freigegeben.
//!
//! Sicherheitsannahmen:
//! - **Der Schluessel steht nur im Geheimnisspeicher** (Fach `token`), geht nur als
//!   `Authorization: Bearer` an den konfigurierten Endpunkt und nie in Fehlertexte,
//!   Protokoll oder Treffer. Weiterleitungen werden NICHT befolgt: ein Server kann den
//!   Schluessel nicht an einen anderen Host weiterreichen lassen.
//! - Nur `https`; `http` nur gegen Loopback (lokaler Test). Adressen mit Benutzer und
//!   Passwort (`https://u:p@host`) werden abgelehnt.
//! - **Treffer sind fremde Daten**: Titel, Pfad und Textstellen werden von
//!   Steuerzeichen befreit und gekuerzt; sie sind nie eine Anweisung (Prompt-Injection
//!   ist Sache der Aufrufer, die Treffer als Zitat, nicht als Befehl behandeln).
//! - Grenzen: Anfrage 2000 Zeichen, hoechstens 20 Treffer, Antwort hoechstens
//!   `MAX_RESPONSE_BYTES`, Zeitlimit je Anfrage. Kein Kindprozess, kein Modell; bei
//!   vollem Arbeitsspeicher aendert sich nichts (die Antwort ist auf 2 MiB begrenzt).
//!
//! Fehlermeldungen unterscheiden, woran es liegt: Adresse falsch / Dienst aus
//! (`wissen_unreachable`, `wissen_endpoint_not_found`), Schluessel ungueltig
//! (`wissen_unauthorized`, 401), Schluessel ohne Wissens-Scope (`wissen_scope_missing`,
//! 403), zu viele Anfragen (429).

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::audit::sanitize_audit_text;

pub const MAX_QUERY_CHARS: usize = 2000;
pub const MAX_LIMIT: u32 = 20;
pub const DEFAULT_LIMIT: u32 = 10;
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_TOOL: &str = "wissen_suchen";
const MAX_SNIPPET_CHARS: usize = 600;
const MAX_FIELD_CHARS: usize = 300;
const MAX_HITS: usize = 50;
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Einstellungen der Wissensbasis (`config_json`; der Schluessel steht NICHT hier).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WissenConfig {
    /// Adresse des MCP-Endpunkts, z. B. `https://os.example.de/mcp`.
    pub endpoint: String,
    /// Name des Suchwerkzeugs (Vorgabe `wissen_suchen`).
    #[serde(default = "default_tool")]
    pub search_tool: String,
    /// Vorgabe-Bereich fuer die Suche (leer: alle erlaubten).
    #[serde(default)]
    pub area: String,
}

fn default_tool() -> String {
    DEFAULT_TOOL.to_string()
}

impl WissenConfig {
    pub fn from_config_json(json: &str) -> Result<Self, WissenError> {
        let cfg: WissenConfig = serde_json::from_str(json).map_err(|_| {
            WissenError::Config("Die Einstellungen der Wissensbasis sind unvollständig.")
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn to_json(&self) -> Value {
        json!({ "endpoint": self.endpoint, "search_tool": self.search_tool, "area": self.area })
    }

    pub fn validate(&self) -> Result<(), WissenError> {
        parse_endpoint(&self.endpoint)?;
        let tool = self.search_tool.trim();
        if tool.is_empty()
            || tool.len() > 64
            || !tool
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        {
            return Err(WissenError::Config("Der Werkzeugname ist ungültig."));
        }
        if self.area.len() > 40
            || !self
                .area
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(WissenError::Config("Der Bereich ist ungültig."));
        }
        Ok(())
    }
}

/// Prueft die Adresse und liefert sie bereinigt (ohne Fragment).
pub fn parse_endpoint(raw: &str) -> Result<url::Url, WissenError> {
    let bad = |m: &'static str| Err(WissenError::Config(m));
    let text = raw.trim();
    if text.is_empty() {
        return bad("Die Adresse des Endpunkts fehlt.");
    }
    if text.len() > 500 || text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return bad("Die Adresse des Endpunkts ist ungültig.");
    }
    let Ok(mut url) = url::Url::parse(text) else {
        return bad("Die Adresse des Endpunkts ist ungültig (zum Beispiel https://host/mcp).");
    };
    if !url.username().is_empty() || url.password().is_some() {
        return bad("Die Adresse darf keinen Benutzer und kein Passwort enthalten; der Schlüssel gehört in das Feld „Schlüssel“.");
    }
    let Some(host) = url.host_str().map(str::to_string) else {
        return bad("Die Adresse des Endpunkts hat keinen Server.");
    };
    match url.scheme() {
        "https" => {}
        "http" if super::smtp::is_loopback_host(&host) => {}
        "http" => return bad("Der Endpunkt muss https verwenden (http nur auf diesem Rechner)."),
        _ => return bad("Der Endpunkt muss mit https:// beginnen."),
    }
    url.set_fragment(None);
    Ok(url)
}

/// Ein Treffer der Suche (alles fremder Text, bereinigt).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct WissenHit {
    pub title: String,
    pub path: String,
    pub area: String,
    pub snippet: String,
    pub score: f64,
    /// `vault` oder `buch`.
    pub source: String,
    pub page: Option<u32>,
    pub document_id: Option<String>,
}

/// Fehler der Wissensbasis. `code()` ist der Schluessel der Oberflaeche.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WissenError {
    Config(&'static str),
    TokenMissing,
    Unreachable(String),
    Tls(String),
    Timeout,
    /// 401: Schluessel unbekannt, widerrufen oder abgelaufen.
    Unauthorized,
    /// 403: Schluessel ohne Wissens-Scope (Text des Servers, falls vorhanden).
    ScopeMissing(String),
    RateLimited,
    EndpointNotFound,
    Server(u16),
    Protocol(String),
    ToolMissing(String),
    /// Das Werkzeug meldete selbst einen Fehler (Text des Servers).
    Tool(String),
}

impl WissenError {
    pub fn code(&self) -> &'static str {
        match self {
            WissenError::Config(_) => "wissen_config_invalid",
            WissenError::TokenMissing => "wissen_token_missing",
            WissenError::Unreachable(_) => "wissen_unreachable",
            WissenError::Tls(_) => "wissen_tls_failed",
            WissenError::Timeout => "wissen_timeout",
            WissenError::Unauthorized => "wissen_unauthorized",
            WissenError::ScopeMissing(_) => "wissen_scope_missing",
            WissenError::RateLimited => "wissen_rate_limited",
            WissenError::EndpointNotFound => "wissen_endpoint_not_found",
            WissenError::Server(_) => "wissen_server_error",
            WissenError::Protocol(_) => "wissen_protocol",
            WissenError::ToolMissing(_) => "wissen_tool_missing",
            WissenError::Tool(_) => "wissen_tool_error",
        }
    }
}

impl std::fmt::Display for WissenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WissenError::Config(m) => write!(f, "{m}"),
            WissenError::TokenMissing => write!(
                f,
                "Der Zugangsschlüssel fehlt. Bitte in der Integration neu eintragen."
            ),
            WissenError::Unreachable(host) => write!(
                f,
                "Die Wissensbasis ist nicht erreichbar ({host}). Läuft der Dienst, und stimmt die Adresse?"
            ),
            WissenError::Tls(m) => write!(
                f,
                "Die verschlüsselte Verbindung zur Wissensbasis kam nicht zustande (Zertifikat oder Protokoll): {m}"
            ),
            WissenError::Timeout => write!(f, "Die Wissensbasis antwortet nicht (Zeitüberschreitung)."),
            WissenError::Unauthorized => write!(
                f,
                "Der Zugangsschlüssel wurde abgelehnt: ungültig, widerrufen oder abgelaufen. Bitte in der Wissensbasis einen neuen Schlüssel anlegen und hier eintragen."
            ),
            WissenError::ScopeMissing(server) => {
                write!(
                    f,
                    "Der Zugangsschlüssel ist gültig, darf aber nicht in der Wissensbasis suchen: Es fehlt der Scope „wissen:read“ (oder „wissen:read:<bereich>“). Bitte den Schlüssel in der Wissensbasis um diesen Scope erweitern."
                )?;
                if !server.is_empty() {
                    write!(f, " Meldung des Servers: {server}")?;
                }
                Ok(())
            }
            WissenError::RateLimited => write!(
                f,
                "Zu viele Anfragen an die Wissensbasis. Bitte kurz warten und noch einmal versuchen."
            ),
            WissenError::EndpointNotFound => write!(
                f,
                "Unter dieser Adresse gibt es keinen MCP-Endpunkt (404/405). Der Pfad lautet meist /mcp."
            ),
            WissenError::Server(code) => write!(
                f,
                "Die Wissensbasis meldet einen Serverfehler (HTTP {code}). Bitte später noch einmal versuchen."
            ),
            WissenError::Protocol(m) => write!(f, "Unerwartete Antwort der Wissensbasis: {m}"),
            WissenError::ToolMissing(t) => write!(
                f,
                "Die Wissensbasis bietet das Werkzeug „{t}“ nicht an. Bitte den Werkzeugnamen in der Integration prüfen."
            ),
            WissenError::Tool(m) => write!(f, "Die Wissensbasis meldet: {m}"),
        }
    }
}

impl std::error::Error for WissenError {}

#[derive(Clone, Debug)]
pub struct HttpOpts {
    pub timeout: Duration,
}

impl Default for HttpOpts {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(25),
        }
    }
}

fn clean(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    sanitize_audit_text(&flat, max)
}

fn host_of(url: &url::Url) -> String {
    url.host_str().unwrap_or("?").to_string()
}

fn map_reqwest(url: &url::Url, e: reqwest::Error) -> WissenError {
    if e.is_timeout() {
        return WissenError::Timeout;
    }
    let chain = format!("{e:?}").to_lowercase();
    if chain.contains("certificate")
        || chain.contains("tls")
        || chain.contains("ssl")
        || chain.contains("schannel")
        || chain.contains("handshake")
    {
        return WissenError::Tls(clean(&e.without_url().to_string(), 160));
    }
    WissenError::Unreachable(host_of(url))
}

struct Reply {
    status: u16,
    headers: HeaderMap,
    body: String,
}

async fn read_body(mut resp: reqwest::Response, url: &url::Url) -> Result<Reply, WissenError> {
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if buf.len() + chunk.len() > MAX_RESPONSE_BYTES {
                    return Err(WissenError::Protocol(
                        "Die Antwort ist zu groß.".to_string(),
                    ));
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => return Err(map_reqwest(url, e)),
        }
    }
    Ok(Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&buf).into_owned(),
    })
}

/// Meldung des Servers aus einem Fehlerrumpf (`{code, message}`), bereinigt.
fn server_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
        .map(|m| clean(&m, 200))
        .unwrap_or_default()
}

/// Scope aus `WWW-Authenticate: Bearer error="insufficient_scope", scope="..."`.
fn challenge_scope(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("www-authenticate")?.to_str().ok()?;
    let start = value.find("scope=\"")? + 7;
    let rest = &value[start..];
    let end = rest.find('"')?;
    Some(clean(&rest[..end], 80))
}

fn status_error(reply: &Reply) -> Option<WissenError> {
    match reply.status {
        200..=299 => None,
        401 => Some(WissenError::Unauthorized),
        403 => {
            let mut message = server_message(&reply.body);
            if let Some(scope) = challenge_scope(&reply.headers) {
                if !message.contains(&scope) {
                    message = format!("{message} (benötigt: {scope})").trim().to_string();
                }
            }
            Some(WissenError::ScopeMissing(message))
        }
        404 | 405 => Some(WissenError::EndpointNotFound),
        429 => Some(WissenError::RateLimited),
        s if (500..600).contains(&s) => Some(WissenError::Server(s)),
        s if (300..400).contains(&s) => Some(WissenError::Protocol(format!(
            "Der Server leitet weiter (HTTP {s}); Weiterleitungen werden aus Sicherheitsgründen nicht befolgt. Bitte die endgültige Adresse eintragen."
        ))),
        s => Some(WissenError::Protocol(format!("HTTP {s}"))),
    }
}

/// JSON-RPC-Antwort aus einem JSON- oder Ereignisstrom-Rumpf.
fn parse_rpc(body: &str, content_type: &str, id: i64) -> Result<Value, WissenError> {
    let proto = |m: &str| WissenError::Protocol(m.to_string());
    let value: Value = if content_type.contains("text/event-stream") {
        let mut found = None;
        let mut data = String::new();
        let flush = |data: &mut String, found: &mut Option<Value>| {
            if data.is_empty() {
                return;
            }
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if v.get("id").and_then(Value::as_i64) == Some(id)
                    && (v.get("result").is_some() || v.get("error").is_some())
                {
                    *found = Some(v);
                }
            }
            data.clear();
        };
        for line in body.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            } else if line.trim().is_empty() {
                flush(&mut data, &mut found);
            }
        }
        flush(&mut data, &mut found);
        found.ok_or_else(|| proto("Der Ereignisstrom enthält keine Antwort."))?
    } else {
        serde_json::from_str(body).map_err(|_| proto("Die Antwort ist kein JSON."))?
    };
    if let Some(err) = value.get("error") {
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .map(|m| clean(m, 200))
            .unwrap_or_else(|| "unbekannter Fehler".to_string());
        return Err(WissenError::Tool(message));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| proto("Die Antwort enthält kein Ergebnis."))
}

struct Client {
    http: reqwest::Client,
    url: url::Url,
    session: Option<String>,
    version: Option<String>,
}

impl Client {
    fn new(url: url::Url, token: &str, opts: &HttpOpts) -> Result<Self, WissenError> {
        let mut headers = HeaderMap::new();
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| WissenError::Config("Der Zugangsschlüssel enthält ungültige Zeichen."))?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(opts.timeout)
            .user_agent(concat!("local-voice-ai/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| WissenError::Tls(clean(&e.to_string(), 160)))?;
        Ok(Self {
            http,
            url,
            session: None,
            version: None,
        })
    }

    async fn post(&mut self, body: Value, id: Option<i64>) -> Result<Option<Value>, WissenError> {
        let mut req = self.http.post(self.url.clone()).json(&body);
        if let Some(s) = &self.session {
            req = req.header("Mcp-Session-Id", s);
        }
        if let Some(v) = &self.version {
            req = req.header("MCP-Protocol-Version", v);
        }
        let resp = req.send().await.map_err(|e| map_reqwest(&self.url, e))?;
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if let Some(s) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            if s.len() <= 200 && s.chars().all(|c| c.is_ascii_graphic()) {
                self.session = Some(s.to_string());
            }
        }
        let reply = read_body(resp, &self.url).await?;
        if let Some(e) = status_error(&reply) {
            return Err(e);
        }
        match id {
            Some(id) => parse_rpc(&reply.body, &content_type, id).map(Some),
            None => Ok(None),
        }
    }

    async fn initialize(&mut self) -> Result<(), WissenError> {
        let result = self
            .post(
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {
                        "protocolVersion": PROTOCOL_VERSION,
                        "capabilities": {},
                        "clientInfo": { "name": "local-voice-ai", "version": env!("CARGO_PKG_VERSION") }
                    }
                }),
                Some(1),
            )
            .await?;
        if let Some(v) = result
            .as_ref()
            .and_then(|r| r.get("protocolVersion"))
            .and_then(Value::as_str)
        {
            if v.len() <= 20 && v.chars().all(|c| c.is_ascii_graphic()) {
                self.version = Some(v.to_string());
            }
        }
        self.post(
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            None,
        )
        .await?;
        Ok(())
    }

    async fn close(&mut self) {
        if let Some(s) = self.session.take() {
            // Sitzung freigeben; ein Fehler dabei ist unwichtig.
            let _ = self
                .http
                .delete(self.url.clone())
                .header("Mcp-Session-Id", s)
                .send()
                .await;
        }
    }
}

fn str_field(v: &Value, key: &str, max: usize) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .map(|s| clean(s, max))
        .unwrap_or_default()
}

fn hit_from(v: &Value) -> Option<WissenHit> {
    if !v.is_object() {
        return None;
    }
    let title = str_field(v, "titel", MAX_FIELD_CHARS);
    let path = str_field(v, "pfad", MAX_FIELD_CHARS);
    let snippet = str_field(v, "snippet", MAX_SNIPPET_CHARS);
    if title.is_empty() && path.is_empty() && snippet.is_empty() {
        return None;
    }
    Some(WissenHit {
        title: if title.is_empty() {
            path.clone()
        } else {
            title
        },
        path,
        area: str_field(v, "bereich", 40),
        snippet,
        score: v.get("score").and_then(Value::as_f64).unwrap_or(0.0),
        source: str_field(v, "quelle", 20),
        page: v
            .get("seite")
            .and_then(Value::as_u64)
            .and_then(|p| u32::try_from(p).ok()),
        document_id: v
            .get("document_id")
            .and_then(Value::as_str)
            .map(|s| clean(s, 64))
            .filter(|s| !s.is_empty()),
    })
}

/// Treffer aus dem Ergebnis von `tools/call`.
fn hits_from_result(result: &Value) -> Result<Vec<WissenHit>, WissenError> {
    let text_of = |c: &Value| c.get("text").and_then(Value::as_str).map(str::to_string);
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(text_of)
            .unwrap_or_else(|| "unbekannter Fehler".to_string());
        return Err(WissenError::Tool(clean(&text, 200)));
    }
    let mut items: Vec<Value> = Vec::new();
    if let Some(arr) = result
        .get("structuredContent")
        .and_then(|s| s.get("result").or(Some(s)))
        .and_then(Value::as_array)
    {
        items.extend(arr.iter().cloned());
    } else if let Some(content) = result.get("content").and_then(Value::as_array) {
        for c in content {
            let Some(text) = text_of(c) else { continue };
            match serde_json::from_str::<Value>(&text) {
                Ok(Value::Array(a)) => items.extend(a),
                Ok(v @ Value::Object(_)) => items.push(v),
                _ => {}
            }
        }
    } else {
        return Err(WissenError::Protocol(
            "Die Antwort enthält keine Treffer-Liste.".to_string(),
        ));
    }
    Ok(items.iter().take(MAX_HITS).filter_map(hit_from).collect())
}

fn run_blocking<T: Send>(
    f: impl FnOnce() -> Result<T, WissenError> + Send,
) -> Result<T, WissenError> {
    // Eigener Thread mit eigener Laufzeit: gilt fuer Tauri-Arbeitsthreads wie fuer Tests
    // und kann nie in einer laufenden Laufzeit verschachteln.
    std::thread::scope(|s| {
        s.spawn(f)
            .join()
            .unwrap_or_else(|_| Err(WissenError::Protocol("interner Fehler".to_string())))
    })
}

fn runtime() -> Result<tokio::runtime::Runtime, WissenError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| WissenError::Protocol(clean(&e.to_string(), 100)))
}

/// Sucht in der Wissensbasis (`wissen_suchen`).
pub fn search(
    cfg: &WissenConfig,
    token: &str,
    query: &str,
    limit: Option<u32>,
    area: Option<&str>,
    opts: &HttpOpts,
) -> Result<Vec<WissenHit>, WissenError> {
    cfg.validate()?;
    let q = query.trim();
    if q.is_empty() {
        return Err(WissenError::Config("Die Suchanfrage ist leer."));
    }
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(WissenError::Config(
            "Die Suchanfrage ist zu lang (höchstens 2000 Zeichen).",
        ));
    }
    if token.is_empty() {
        return Err(WissenError::TokenMissing);
    }
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let area = area
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .unwrap_or(cfg.area.trim());
    if area.len() > 40 || !area.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(WissenError::Config("Der Bereich ist ungültig."));
    }
    let url = parse_endpoint(&cfg.endpoint)?;
    let mut arguments = json!({ "q": q, "limit": limit });
    if !area.is_empty() {
        arguments["bereich"] = json!(area);
    }
    let tool = cfg.search_tool.trim().to_string();
    run_blocking(|| {
        runtime()?.block_on(async {
            let mut client = Client::new(url, token, opts)?;
            client.initialize().await?;
            let outcome = client
                .post(
                    json!({
                        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                        "params": { "name": tool, "arguments": arguments }
                    }),
                    Some(3),
                )
                .await;
            client.close().await;
            let result = outcome?.unwrap_or(Value::Null);
            hits_from_result(&result)
        })
    })
}

/// Was `test` herausfindet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestInfo {
    pub tools: Vec<String>,
}

/// Probiert Adresse, Schluessel und Werkzeug aus (`initialize` und `tools/list`),
/// ohne zu suchen.
pub fn test(cfg: &WissenConfig, token: &str, opts: &HttpOpts) -> Result<TestInfo, WissenError> {
    cfg.validate()?;
    if token.is_empty() {
        return Err(WissenError::TokenMissing);
    }
    let url = parse_endpoint(&cfg.endpoint)?;
    let tool = cfg.search_tool.trim().to_string();
    run_blocking(|| {
        runtime()?.block_on(async {
            let mut client = Client::new(url, token, opts)?;
            client.initialize().await?;
            let outcome = client
                .post(
                    json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
                    Some(2),
                )
                .await;
            client.close().await;
            let result = outcome?.unwrap_or(Value::Null);
            let tools: Vec<String> = result
                .get("tools")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.get("name").and_then(Value::as_str))
                        .map(|n| clean(n, 64))
                        .collect()
                })
                .unwrap_or_default();
            if !tools.contains(&tool) {
                return Err(WissenError::ToolMissing(tool));
            }
            Ok(TestInfo { tools })
        })
    })
}

#[cfg(test)]
mod tests;
