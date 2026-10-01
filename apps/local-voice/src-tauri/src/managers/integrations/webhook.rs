//! Webhook als Ziel (B5, Goal „Workflow-Automation“, n8n-Bruecke): ein Ablauf schickt Daten per
//! `POST` an die Adresse eines Webhooks, die der Nutzer im Register hinterlegt hat.
//!
//! Sicherheitsannahmen:
//! - **Die Adresse ist ein Geheimnis.** n8n und aehnliche Dienste tragen den Schluessel im Pfad
//!   (`/webhook/<uuid>`). Sie steht nur im Geheimnisspeicher (Fach `url`); in der Konfiguration
//!   steht nur der Server zur Anzeige. Fehlertexte, Audit und Laufprotokoll nennen hoechstens den
//!   Server, nie Pfad oder Abfrage.
//! - Nur `https`; `http` nur gegen Loopback (lokaler n8n). Adressen mit Benutzer und Passwort
//!   werden abgelehnt. Umleitungen werden NIE befolgt: ein Server kann die Daten nicht an einen
//!   anderen Host weiterreichen lassen.
//! - Grenzen: Anfrage hoechstens [`MAX_REQUEST_BYTES`], Antwort hoechstens
//!   [`MAX_RESPONSE_BYTES`] (mehr wird nicht gelesen, die Verbindung faellt), Zeitlimit fuer
//!   Verbinden und Gesamtdauer. Kein Kindprozess, kein Modell; bei vollem Arbeitsspeicher aendert
//!   sich nichts (hoechstens 1 MiB Antwort und 256 KiB Anfrage im Speicher).
//! - **Zustellung**: [`WebhookError::class`] unterscheidet „nichts angekommen“ (Verbindung kam
//!   nicht zustande, 429), „abgelehnt“ (4xx, Umleitung, Konfiguration) und „unklar“ (Zeitlimit
//!   oder Abbruch NACH dem Verbinden, 5xx): bei unklarem Ausgang wiederholt niemand von selbst.

use std::time::Duration;

use reqwest::header::{HeaderValue, ACCEPT, CONTENT_TYPE};
use serde_json::{json, Value};

use super::smtp::is_loopback_host;

pub const MAX_URL_CHARS: usize = 500;
pub const MAX_REQUEST_BYTES: usize = 256 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Grenzen und Zeiten eines Aufrufs (Tests verkuerzen sie).
#[derive(Clone, Debug)]
pub struct HttpOpts {
    pub connect_timeout: Duration,
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

impl Default for HttpOpts {
    fn default() -> Self {
        Self {
            connect_timeout: CONNECT_TIMEOUT,
            timeout: REQUEST_TIMEOUT,
            max_response_bytes: MAX_RESPONSE_BYTES,
        }
    }
}

/// Wie ein gescheiterter Aufruf einzuordnen ist (siehe Moduldoku).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Es ist nichts angekommen: ein neuer Versuch ist sicher.
    NotSent,
    /// Wird so nie gelingen (Konfiguration, 4xx, Umleitung).
    Rejected,
    /// Ob der Webhook die Daten bekam, ist unklar: nie von selbst wiederholen.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebhookError {
    Config(&'static str),
    UrlMissing,
    /// Die Anfrage waere groesser als [`MAX_REQUEST_BYTES`].
    TooLarge(usize),
    /// Verbindung nicht zustande gekommen (Server genannt, nie der Pfad).
    Connect(String),
    /// Zeitlimit NACH dem Verbinden.
    Timeout,
    /// Verbindung riss nach dem Verbinden.
    Network(String),
    /// Der Server antwortet mit einer Umleitung (wird nie befolgt).
    Redirect(u16),
    Status(u16),
}

impl WebhookError {
    pub fn code(&self) -> &'static str {
        match self {
            WebhookError::Config(_) => "webhook_config_invalid",
            WebhookError::UrlMissing => "webhook_url_missing",
            WebhookError::TooLarge(_) => "webhook_request_too_large",
            WebhookError::Connect(_) => "webhook_unreachable",
            WebhookError::Timeout => "webhook_timeout",
            WebhookError::Network(_) => "webhook_network",
            WebhookError::Redirect(_) => "webhook_redirect",
            WebhookError::Status(_) => "webhook_status",
        }
    }

    pub fn class(&self) -> Class {
        match self {
            WebhookError::Config(_)
            | WebhookError::UrlMissing
            | WebhookError::TooLarge(_)
            | WebhookError::Redirect(_) => Class::Rejected,
            WebhookError::Connect(_) => Class::NotSent,
            WebhookError::Timeout | WebhookError::Network(_) => Class::Unknown,
            // 429/408: der Server hat die Anfrage nicht bearbeitet. Andere 4xx sind endgueltig.
            WebhookError::Status(429 | 408) => Class::NotSent,
            WebhookError::Status(s) if (400..500).contains(s) => Class::Rejected,
            WebhookError::Status(_) => Class::Unknown,
        }
    }
}

impl std::fmt::Display for WebhookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebhookError::Config(m) => write!(f, "{m}"),
            WebhookError::UrlMissing => write!(
                f,
                "Die Adresse des Webhooks fehlt. Bitte in der Integration neu eintragen."
            ),
            WebhookError::TooLarge(bytes) => write!(
                f,
                "Die Daten sind zu groß für den Webhook ({} KiB, höchstens {} KiB).",
                bytes.div_ceil(1024),
                MAX_REQUEST_BYTES / 1024
            ),
            WebhookError::Connect(host) => {
                write!(f, "Der Webhook-Server ist nicht erreichbar ({host}).")
            }
            WebhookError::Timeout => write!(
                f,
                "Der Webhook antwortet nicht rechtzeitig; ob er die Daten bekam, ist unklar."
            ),
            WebhookError::Network(host) => write!(
                f,
                "Die Verbindung zum Webhook riss ab ({host}); ob er die Daten bekam, ist unklar."
            ),
            WebhookError::Redirect(status) => write!(
                f,
                "Der Webhook leitet um (HTTP {status}). Umleitungen werden nicht befolgt; bitte die endgültige Adresse eintragen."
            ),
            WebhookError::Status(status) => {
                write!(f, "Der Webhook antwortet mit HTTP {status}.")
            }
        }
    }
}

impl std::error::Error for WebhookError {}

/// Prueft die Adresse und liefert sie bereinigt (ohne Fragment).
pub fn parse_url(raw: &str) -> Result<url::Url, WebhookError> {
    let bad = |m: &'static str| Err(WebhookError::Config(m));
    let text = raw.trim();
    if text.is_empty() {
        return Err(WebhookError::UrlMissing);
    }
    if text.len() > MAX_URL_CHARS || text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return bad("Die Adresse des Webhooks ist ungültig.");
    }
    let Ok(mut url) = url::Url::parse(text) else {
        return bad(
            "Die Adresse des Webhooks ist ungültig (zum Beispiel https://host/webhook/abc).",
        );
    };
    if !url.username().is_empty() || url.password().is_some() {
        return bad("Die Adresse darf keinen Benutzer und kein Passwort enthalten.");
    }
    let Some(host) = url.host_str().map(str::to_string) else {
        return bad("Die Adresse des Webhooks hat keinen Server.");
    };
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_host(&host) => {}
        "http" => return bad("Der Webhook muss https verwenden (http nur auf diesem Rechner)."),
        _ => return bad("Der Webhook muss mit https:// beginnen."),
    }
    url.set_fragment(None);
    Ok(url)
}

/// Der Server einer Adresse zur Anzeige (nie Pfad oder Abfrage).
pub fn host_of(url: &url::Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => "?".to_string(),
    }
}

/// Die Konfiguration einer Webhook-Integration (`config_json`, ohne Geheimnis): nur der Server
/// zur Anzeige.
pub fn config_for(url: &url::Url) -> Value {
    json!({ "host": host_of(url) })
}

/// Der angezeigte Server aus einem `config_json`; leer, wenn er fehlt.
pub fn host_from_config(config_json: &str) -> String {
    serde_json::from_str::<Value>(config_json)
        .ok()
        .and_then(|v| v.get("host").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// Die Antwort des Webhooks (fremder Text, nur begrenzt gelesen).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookReply {
    pub status: u16,
    pub content_type: String,
    pub body: String,
    /// Die Antwort war groesser als erlaubt; der Rest wurde nicht gelesen.
    pub truncated: bool,
    pub bytes: usize,
}

/// Sendet `payload` als JSON per `POST`. Blockiert; laeuft auf einem eigenen Thread mit eigener
/// Laufzeit (gilt fuer Arbeiter-Threads wie fuer Tests und verschachtelt nie in einer Laufzeit).
/// Erfolg ist jede 2xx-Antwort.
pub fn post(
    url: &url::Url,
    payload: &Value,
    idempotency_key: &str,
    opts: &HttpOpts,
) -> Result<WebhookReply, WebhookError> {
    let body = serde_json::to_vec(payload)
        .map_err(|_| WebhookError::Config("Die Daten ließen sich nicht als JSON schreiben."))?;
    if body.len() > MAX_REQUEST_BYTES {
        return Err(WebhookError::TooLarge(body.len()));
    }
    std::thread::scope(|s| {
        s.spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| WebhookError::Config("Die Netzwerk-Laufzeit startete nicht."))?;
            rt.block_on(post_async(url, body, idempotency_key, opts))
        })
        .join()
        .unwrap_or(Err(WebhookError::Config("Interner Fehler beim Senden.")))
    })
}

async fn post_async(
    url: &url::Url,
    body: Vec<u8>,
    idempotency_key: &str,
    opts: &HttpOpts,
) -> Result<WebhookReply, WebhookError> {
    let host = host_of(url);
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(opts.connect_timeout)
        .timeout(opts.timeout)
        .user_agent(concat!("local-voice-ai/", env!("CARGO_PKG_VERSION")));
    if url.host_str().is_some_and(is_loopback_host) {
        // Ein lokaler n8n geht nie ueber einen Proxy.
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|_| WebhookError::Config("Der Netzwerk-Client startete nicht."))?;
    let key = HeaderValue::from_str(idempotency_key)
        .unwrap_or_else(|_| HeaderValue::from_static("unbekannt"));
    let mut resp = client
        .post(url.clone())
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json, text/plain, */*")
        .header("Idempotency-Key", key)
        .body(body)
        .send()
        .await
        .map_err(|e| map_send(&host, &e))?;
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect::<String>();
    // Die Antwort wird nur bis zur Grenze gelesen; Mehr bricht ab, ohne die Zustellung zu
    // beruehren (die Daten sind dann laengst angekommen).
    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let room = opts.max_response_bytes.saturating_sub(buf.len());
                if chunk.len() > room {
                    buf.extend_from_slice(&chunk[..room]);
                    truncated = true;
                    break;
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) if (200..300).contains(&status) => break,
            Err(e) => return Err(map_send(&host, &e)),
        }
    }
    if (300..400).contains(&status) {
        return Err(WebhookError::Redirect(status));
    }
    if !(200..300).contains(&status) {
        return Err(WebhookError::Status(status));
    }
    Ok(WebhookReply {
        status,
        content_type,
        bytes: buf.len(),
        body: String::from_utf8_lossy(&buf).into_owned(),
        truncated,
    })
}

fn map_send(host: &str, e: &reqwest::Error) -> WebhookError {
    if e.is_connect() {
        return WebhookError::Connect(host.to_string());
    }
    if e.is_timeout() {
        return WebhookError::Timeout;
    }
    if e.is_builder() {
        return WebhookError::Config("Die Anfrage ließ sich nicht aufbauen.");
    }
    WebhookError::Network(host.to_string())
}

#[cfg(test)]
mod tests;
