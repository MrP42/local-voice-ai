//! JSON-Aufrufe an Dienste (Slack, Notion, Jira, …) nach denselben Regeln wie der Webhook
//! (`integrations::webhook`): nur HTTPS (http nur gegen Loopback, fuer Attrappen), Umleitungen
//! werden nie befolgt, Anfrage und Antwort sind begrenzt, Zeitlimits fuer Verbinden und Dauer.
//!
//! Zusaetzlich: der Host muss zur Regel des Dienstes passen (Register), damit ein Schluessel
//! nie an einen fremden Server geht. Geheimnisse stehen nur in Kopfzeilen oder im Pfad einer
//! Webhook-Adresse; Fehlertexte nennen hoechstens den Server.

use reqwest::header::{HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use serde_json::Value;

use crate::managers::integrations::smtp::is_loopback_host;
use crate::managers::integrations::webhook::{host_of, HttpOpts, MAX_REQUEST_BYTES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Patch,
    Put,
}

/// Eine Anfrage an einen Dienst. `headers` kann Geheimnisse tragen (Authorization) und wird
/// deshalb nie geloggt oder angezeigt.
#[derive(Clone, PartialEq)]
pub struct ApiRequest {
    pub method: Method,
    pub url: url::Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
}

impl std::fmt::Debug for ApiRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Ohne Kopfzeilen (Schluessel) und ohne Abfrage (Trello o. ae.).
        f.debug_struct("ApiRequest")
            .field("method", &self.method)
            .field("host", &host_of(&self.url))
            .field("path", &self.url.path())
            .finish()
    }
}

impl ApiRequest {
    pub fn new(method: Method, url: url::Url) -> Self {
        Self {
            method,
            url,
            headers: Vec::new(),
            body: None,
        }
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    pub fn json(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    /// Kopfzeile nach Namen (fuer Tests).
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Antwort eines Dienstes: Status und Koerper (JSON, falls lesbar).
#[derive(Clone, Debug, PartialEq)]
pub struct ApiReply {
    pub status: u16,
    pub json: Value,
}

impl ApiReply {
    pub fn ok(json: Value) -> Self {
        Self { status: 200, json }
    }
}

/// Wie ein gescheiterter Aufruf einzuordnen ist (wie beim Webhook).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Nichts angekommen: ein neuer Versuch ist sicher.
    NotSent,
    /// Wird so nie gelingen (Schluessel, Ziel, Eingabe).
    Rejected,
    /// Unklar, ob es ankam: nie von selbst wiederholen.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// Konfiguration oder Eingabe unbrauchbar (Text ohne Geheimnis).
    Config(String),
    /// Der Dienst lehnt den Schluessel ab (401/403).
    Auth(u16),
    /// Ziel (Projekt, Liste, Seite) nicht gefunden (404).
    NotFound,
    /// Zu viele Anfragen (429); Sekunden laut `Retry-After`, falls genannt.
    RateLimited(Option<u64>),
    /// Andere Ablehnung mit kurzer Begruendung des Dienstes (ohne Geheimnis).
    Status(u16, String),
    Connect(String),
    Timeout,
    Network(String),
    Redirect(u16),
    TooLarge(usize),
    /// Die Antwort passt nicht zur erwarteten Form.
    BadResponse(&'static str),
    /// Ein Teil kam an, der Rest scheiterte (mehrteilige Nachricht, Seite ohne alle Bloecke):
    /// nie von selbst wiederholen, sonst kaeme der erste Teil doppelt.
    Partial(String),
}

impl ServiceError {
    pub fn code(&self) -> &'static str {
        match self {
            ServiceError::Config(_) => "service_config_invalid",
            ServiceError::Auth(_) => "service_auth",
            ServiceError::NotFound => "service_target_not_found",
            ServiceError::RateLimited(_) => "service_rate_limited",
            ServiceError::Status(..) => "service_status",
            ServiceError::Connect(_) => "service_unreachable",
            ServiceError::Timeout => "service_timeout",
            ServiceError::Network(_) => "service_network",
            ServiceError::Redirect(_) => "service_redirect",
            ServiceError::TooLarge(_) => "service_request_too_large",
            ServiceError::BadResponse(_) => "service_bad_response",
            ServiceError::Partial(_) => "service_partial",
        }
    }

    pub fn class(&self) -> Class {
        match self {
            ServiceError::Connect(_) | ServiceError::RateLimited(_) => Class::NotSent,
            ServiceError::Timeout
            | ServiceError::Network(_)
            | ServiceError::BadResponse(_)
            | ServiceError::Partial(_) => Class::Unknown,
            ServiceError::Status(s, _) if *s >= 500 => Class::Unknown,
            _ => Class::Rejected,
        }
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Config(m) => write!(f, "{m}"),
            ServiceError::Auth(s) => write!(
                f,
                "Der Dienst lehnt den Schlüssel ab (HTTP {s}). Bitte Schlüssel und Rechte in der Integration prüfen."
            ),
            ServiceError::NotFound => write!(
                f,
                "Das Ziel wurde nicht gefunden (HTTP 404). Bitte Projekt, Liste oder Seite in der Integration prüfen."
            ),
            ServiceError::RateLimited(Some(s)) => {
                write!(f, "Der Dienst bremst (zu viele Anfragen); in {s} s erneut versuchen.")
            }
            ServiceError::RateLimited(None) => {
                write!(f, "Der Dienst bremst (zu viele Anfragen); später erneut versuchen.")
            }
            ServiceError::Status(s, why) if why.is_empty() => {
                write!(f, "Der Dienst antwortet mit HTTP {s}.")
            }
            ServiceError::Status(s, why) => write!(f, "Der Dienst antwortet mit HTTP {s}: {why}"),
            ServiceError::Connect(h) => write!(f, "Der Dienst ist nicht erreichbar ({h})."),
            ServiceError::Timeout => write!(
                f,
                "Der Dienst antwortet nicht rechtzeitig; ob der Eintrag ankam, ist unklar."
            ),
            ServiceError::Network(h) => write!(
                f,
                "Die Verbindung riss ab ({h}); ob der Eintrag ankam, ist unklar."
            ),
            ServiceError::Redirect(s) => write!(
                f,
                "Der Dienst leitet um (HTTP {s}); Umleitungen werden nicht befolgt."
            ),
            ServiceError::TooLarge(b) => write!(
                f,
                "Der Inhalt ist zu groß ({} KiB, höchstens {} KiB).",
                b.div_ceil(1024),
                MAX_REQUEST_BYTES / 1024
            ),
            ServiceError::BadResponse(m) => write!(f, "Unerwartete Antwort des Dienstes: {m}"),
            ServiceError::Partial(m) => write!(f, "Nur teilweise übertragen: {m}"),
        }
    }
}

impl std::error::Error for ServiceError {}

/// Regel fuer erlaubte Hosts eines Dienstes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostRule {
    /// Genau dieser Host.
    Exact(&'static str),
    /// Dieser Host oder eine Unterdomain (`.atlassian.net`).
    Suffix(&'static str),
}

impl HostRule {
    pub fn matches(self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        match self {
            HostRule::Exact(h) => host == h,
            HostRule::Suffix(s) => {
                let s = s.trim_start_matches('.');
                host == s || host.ends_with(&format!(".{s}"))
            }
        }
    }
}

/// Prueft Schema und Host einer Adresse gegen die Regeln des Dienstes. Loopback-`http` ist
/// nur fuer Attrappen in Tests erlaubt (`allow_loopback`).
pub fn check_url(
    url: &url::Url,
    rules: &[HostRule],
    allow_loopback: bool,
) -> Result<(), ServiceError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ServiceError::Config(
            "Die Adresse darf keinen Benutzer und kein Passwort enthalten.".into(),
        ));
    }
    let host = url.host_str().unwrap_or("");
    if allow_loopback && is_loopback_host(host) {
        return Ok(());
    }
    if url.scheme() != "https" {
        return Err(ServiceError::Config(
            "Der Dienst muss https verwenden.".into(),
        ));
    }
    if !rules.iter().any(|r| r.matches(host)) {
        return Err(ServiceError::Config(format!(
            "Die Adresse gehört nicht zu diesem Dienst ({}).",
            host_of(url)
        )));
    }
    Ok(())
}

/// Fuehrt eine Anfrage aus. Blockiert; eigener Thread mit eigener Laufzeit (wie der Webhook).
pub fn execute(req: &ApiRequest, opts: &HttpOpts) -> Result<ApiReply, ServiceError> {
    let body = match &req.body {
        Some(v) => Some(serde_json::to_vec(v).map_err(|_| {
            ServiceError::Config("Die Daten ließen sich nicht als JSON schreiben.".into())
        })?),
        None => None,
    };
    if let Some(b) = &body {
        if b.len() > MAX_REQUEST_BYTES {
            return Err(ServiceError::TooLarge(b.len()));
        }
    }
    std::thread::scope(|s| {
        s.spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| {
                    ServiceError::Config("Die Netzwerk-Laufzeit startete nicht.".into())
                })?;
            rt.block_on(execute_async(req, body, opts))
        })
        .join()
        .unwrap_or(Err(ServiceError::Config(
            "Interner Fehler beim Senden.".into(),
        )))
    })
}

async fn execute_async(
    req: &ApiRequest,
    body: Option<Vec<u8>>,
    opts: &HttpOpts,
) -> Result<ApiReply, ServiceError> {
    let host = host_of(&req.url);
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(opts.connect_timeout)
        .timeout(opts.timeout)
        .user_agent(concat!("local-voice-ai/", env!("CARGO_PKG_VERSION")));
    if req.url.host_str().is_some_and(is_loopback_host) {
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|_| ServiceError::Config("Der Netzwerk-Client startete nicht.".into()))?;
    let method = match req.method {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
        Method::Patch => reqwest::Method::PATCH,
        Method::Put => reqwest::Method::PUT,
    };
    let mut rb = client
        .request(method, req.url.clone())
        .header(ACCEPT, "application/json");
    for (name, value) in &req.headers {
        let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            return Err(ServiceError::Config(
                "Der Schlüssel enthält unzulässige Zeichen.".into(),
            ));
        };
        rb = rb.header(n, v);
    }
    if let Some(b) = body {
        rb = rb.header(CONTENT_TYPE, "application/json").body(b);
    }
    let mut resp = rb.send().await.map_err(|e| map_send(&host, &e))?;
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let room = opts.max_response_bytes.saturating_sub(buf.len());
                if chunk.len() > room {
                    buf.extend_from_slice(&chunk[..room]);
                    break;
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) if (200..300).contains(&status) => break,
            Err(e) => return Err(map_send(&host, &e)),
        }
    }
    let json: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
    match status {
        200..=299 => Ok(ApiReply { status, json }),
        300..=399 => Err(ServiceError::Redirect(status)),
        401 | 403 => Err(ServiceError::Auth(status)),
        404 => Err(ServiceError::NotFound),
        429 => Err(ServiceError::RateLimited(retry_after)),
        _ => Err(ServiceError::Status(status, reason(&json))),
    }
}

/// Kurze Begruendung aus einer Fehlerantwort (gaengige Felder), gekuerzt, ohne Steuerzeichen.
pub fn reason(json: &Value) -> String {
    let pick = |v: &Value| -> Option<String> {
        for key in ["message", "error", "errorMessage", "detail", "title"] {
            match v.get(key) {
                Some(Value::String(s)) => return Some(s.clone()),
                Some(Value::Object(_)) => {
                    if let Some(Value::String(s)) = v.get(key).and_then(|o| o.get("message")) {
                        return Some(s.clone());
                    }
                }
                _ => {}
            }
        }
        if let Some(Value::Array(a)) = v.get("errors").or_else(|| v.get("errorMessages")) {
            return a.first().and_then(|e| {
                e.as_str()
                    .map(str::to_string)
                    .or_else(|| e.get("message").and_then(Value::as_str).map(str::to_string))
            });
        }
        None
    };
    pick(json)
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect()
}

fn map_send(host: &str, e: &reqwest::Error) -> ServiceError {
    if e.is_connect() {
        return ServiceError::Connect(host.to_string());
    }
    if e.is_timeout() {
        return ServiceError::Timeout;
    }
    if e.is_builder() {
        return ServiceError::Config("Die Anfrage ließ sich nicht aufbauen.".into());
    }
    ServiceError::Network(host.to_string())
}

/// Kurze Zeitgrenzen fuer Tests.
#[cfg(test)]
pub fn test_opts() -> HttpOpts {
    HttpOpts {
        connect_timeout: std::time::Duration::from_secs(2),
        timeout: std::time::Duration::from_secs(5),
        max_response_bytes: 64 * 1024,
    }
}
