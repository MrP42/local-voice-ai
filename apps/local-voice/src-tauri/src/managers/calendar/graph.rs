//! Microsoft Graph als Kalenderquelle (M5, P5f; `entwurf/m5-m6-kalender-export.md`
//! §3 F15 „Quelle 2 Graph“, §7).
//!
//! Ablauf: OAuth-2.0-Autorisierungscode mit PKCE (S256) ueber den Systembrowser
//! (`opener`), Umleitung auf einen Loopback-Listener (`127.0.0.1`, freier Port),
//! Tokenaustausch, danach `GET /me` (eigene Adresse) und `GET /me/calendarView`
//! (Zeiten in UTC, Paging ueber `@odata.nextLink`). Es gibt KEINE eingebaute
//! Client-ID (Entscheidung E14): der Nutzer traegt die Anwendungs-ID seiner eigenen
//! Entra-App (oeffentlicher Client, Weiterleitungs-URI `http://localhost`) in den
//! Einstellungen ein. Handgeschrieben mit reqwest/sha2/base64/rand, kein oauth2-Crate.
//!
//! Die Weiterleitungs-URI der Anfrage ist `http://localhost:<Port>` (ohne Pfad):
//! Entra gleicht `localhost` unter Ignorieren des Ports ab; eine Anfrage mit
//! `127.0.0.1` passt nicht auf eine registrierte `http://localhost`. Der Listener
//! selbst bindet nur `127.0.0.1`; ein Browser, der `localhost` zuerst als `::1`
//! aufloest, faellt bei „Verbindung abgelehnt“ auf IPv4 zurueck.
//!
//! Fehlerfaelle (P5f) und ihre Absicherung:
//! - Zwei Anmeldungen gleichzeitig (Doppelklick): `GraphState::begin_sign_in`
//!   laesst nur EINE zu (Test `only_one_sign_in_at_a_time`). Anmeldung und Abruf:
//!   der `gate` des Dienstes wird nicht waehrend des Wartens auf den Browser
//!   gehalten (sonst stuende der ICS-Abruf 5 min), sondern nur beim Speichern.
//! - Abbruch mitten im Vorgang (Browser zu, Nutzer bricht ab, 5-min-Zeitlimit):
//!   der Listener endet, der Port wird frei (Tests `loopback_times_out_and_frees_the_port`,
//!   `loopback_cancel_...`); Quelle und Geheimnis entstehen erst NACH Token und
//!   Adresse, in dieser Reihenfolge, und werden bei einem Fehler wieder entfernt
//!   (`register_account_...`).
//! - Voller Datentraeger / Geheimnis nicht schreibbar: bei der Anmeldung entsteht
//!   keine Quelle (`register_account_without_a_writable_vault_creates_no_source`);
//!   beim Erneuern des Tokens bleibt das alte Token im Tresor stehen und der Lauf
//!   nutzt das neue Zugriffstoken (`a_failed_token_write_keeps_the_old_token_and_the_run_going`).
//! - Fehlendes Geraet: kein Browser (`opener` scheitert) → Listener sofort zu,
//!   Klartext (`sign_in_reports_a_missing_browser`); kein Netz → `Network`, der
//!   Cache bleibt (der Aufrufer schreibt nur bei Erfolg).
//! - Kindprozess: keiner. Der Browser wird nur ueber `opener` gestartet, ohne
//!   Handle; es gibt nichts zu beenden oder zu ueberwachen.
//! - Angriffe/Stoerungen am Loopback: falscher `state`, fremder Host-Header
//!   (DNS-Rebinding), falscher Pfad oder falsche Methode, haengende
//!   Vorab-Verbindungen des Browsers: je Verbindung eine eigene Aufgabe mit
//!   Lesezeitlimit; nur eine Anfrage mit richtigem `state` beendet das Warten
//!   (Tests `loopback_ignores_...`, `loopback_rejects_...`).
//! - Server: 401 → einmal Token erneuern und wiederholen, sonst „Anmeldung nötig“;
//!   429 → `Retry-After` wird gemerkt (`GraphState::throttle`); fremder
//!   `@odata.nextLink` wird NICHT verfolgt (sonst liefe das Token an einen
//!   fremden Host); Seiten- und Groessenlimit.
//!
//! Sicherheit der Geheimnisse: das Erneuerungs-Token steht nur DPAPI-verschluesselt
//! in `<appdata>/secrets/<quellen-id>.bin` (`calendar::secret`), zusammen mit
//! Client-ID und Verzeichnis der Anmeldung (ein spaeter geaenderter Einstellungswert
//! bricht bestehende Konten nicht). Nie in `settings_store.json`, nie in
//! `meetings.db`, nie im Log, nie in einer Fehlermeldung. Zugriffstoken leben nur im
//! Arbeitsspeicher (`TokenCache`); Klartext liegt in `Zeroizing`-Puffern.
//!
//! Was bei vollem RAM passiert: der Aufrufer (`service.rs`) prueft vor jedem Abruf
//! `process_guard::check_ram_for_start`; ist der Speicher knapp, wird dieser Lauf mit
//! Klartext uebersprungen. Eine Seite ist auf 8 MB, der Abruf auf 60 Seiten begrenzt.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::rngs::OsRng;
use rand::RngCore;
use reqwest::header::{ACCEPT, RETRY_AFTER};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Notify};
use zeroize::{Zeroize, Zeroizing};

use super::ics;
use super::model::{event_key, Attendee, CalEvent, CalendarKind, CalendarSource};
use super::secret;
use crate::managers::meetings::store::MeetingStore;

pub const AUTHORITY: &str = "https://login.microsoftonline.com";
pub const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
/// Nur lesend: Kalender lesen, Erneuerungs-Token, eigenes Profil.
pub const SCOPES: &str = "Calendars.Read offline_access User.Read";
pub const DEFAULT_TENANT: &str = "common";
/// So lange wartet die Anmeldung auf den Browser.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Name der Quelle in der Liste.
pub const SOURCE_LABEL: &str = "Outlook / Microsoft 365";

const PAGE_SIZE: u32 = 100;
const MAX_PAGES: usize = 60;
const MAX_PAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOKEN_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_HEAD_BYTES: usize = 8 * 1024;
const MAX_CODE_CHARS: usize = 4096;
const READ_HEAD_TIMEOUT: Duration = Duration::from_secs(5);
/// Ein Zugriffstoken gilt im Cache nur bis kurz vor seinem Ablauf.
const TOKEN_SAFETY_MARGIN: Duration = Duration::from_secs(120);
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);
const EVENT_SELECT: &str = "id,iCalUId,subject,start,end,isAllDay,isCancelled,attendees,organizer,onlineMeeting,location,bodyPreview";

// ---------------------------------------------------------------------------
// Fehler
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// Token fehlt/abgelaufen/widerrufen (401, `invalid_grant`): neu anmelden.
    NeedsSignIn,
    /// Nutzer oder Mandant lehnt ab (Einwilligung verweigert, App gesperrt, 403).
    Denied(String),
    Throttled {
        retry_after: Duration,
    },
    Http(u16),
    Network(String),
    Timeout,
    Parse(String),
    SignInTimeout,
    Cancelled,
    Browser(String),
    Listener(String),
    Config(String),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::NeedsSignIn => {
                write!(f, "Anmeldung nötig: Bitte erneut mit Microsoft anmelden.")
            }
            GraphError::Denied(msg) => write!(f, "Microsoft hat den Zugriff abgelehnt: {msg}"),
            GraphError::Throttled { retry_after } => {
                let minutes = retry_after.as_secs().div_ceil(60).max(1);
                write!(
                    f,
                    "Microsoft drosselt die Abfragen (HTTP 429). Nächster Versuch in {minutes} min."
                )
            }
            GraphError::Http(code) => write!(f, "Microsoft Graph antwortet mit HTTP {code}."),
            GraphError::Network(msg) => write!(f, "Keine Verbindung: {msg}"),
            GraphError::Timeout => write!(f, "Zeitüberschreitung bei Microsoft (30 s)."),
            GraphError::Parse(msg) => {
                write!(f, "Die Antwort von Microsoft ist nicht lesbar: {msg}")
            }
            GraphError::SignInTimeout => write!(
                f,
                "Die Anmeldung wurde nicht innerhalb von 5 Minuten abgeschlossen."
            ),
            GraphError::Cancelled => write!(f, "Die Anmeldung wurde abgebrochen."),
            GraphError::Browser(msg) => {
                write!(f, "Der Browser konnte nicht geöffnet werden: {msg}")
            }
            GraphError::Listener(msg) => write!(
                f,
                "Die lokale Anmelde-Schnittstelle konnte nicht gestartet werden: {msg}"
            ),
            GraphError::Config(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for GraphError {}

/// Fehler von `reqwest` ohne die Adresse (sie kann `state`/`skiptoken` tragen).
pub(crate) fn map_reqwest(e: reqwest::Error) -> GraphError {
    let e = e.without_url();
    if e.is_timeout() {
        return GraphError::Timeout;
    }
    let hint = if e.is_connect() {
        " (Der Windows-Systemproxy wird nicht verwendet, nur HTTPS_PROXY und HTTP_PROXY.)"
    } else {
        ""
    };
    GraphError::Network(format!("{e}{hint}"))
}

/// Erste Zeile einer Fehlerbeschreibung, gekuerzt (Microsoft haengt Trace-ID und
/// Zeitstempel in weiteren Zeilen an).
pub(crate) fn short_description(desc: &str) -> String {
    let first = desc.split(['\r', '\n']).next().unwrap_or("").trim();
    first.chars().take(240).collect()
}

/// `Retry-After` als Sekunden oder HTTP-Datum; fehlend/unlesbar = 60 s, hoechstens 1 h.
pub fn parse_retry_after(value: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> Duration {
    let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return DEFAULT_RETRY_AFTER;
    };
    let secs = if let Ok(n) = v.parse::<u64>() {
        n
    } else if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(v) {
        (dt.timestamp() - now.timestamp()).max(0) as u64
    } else {
        return DEFAULT_RETRY_AFTER;
    };
    Duration::from_secs(secs).clamp(Duration::from_secs(1), MAX_RETRY_AFTER)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Vergleich ohne Abbruch beim ersten Unterschied (fuer `state`).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------------------
// Eingaben pruefen
// ---------------------------------------------------------------------------

/// Anwendungs-(Client-)ID: eine GUID (8-4-4-4-12 Hexzeichen), klein geschrieben.
/// Die ID steht spaeter in einer Adresse; nur dieses Format kommt hinein.
pub fn validate_client_id(raw: &str) -> Result<String, String> {
    let t = raw.trim();
    let ok = t.len() == 36
        && t.char_indices().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
    if ok {
        Ok(t.to_ascii_lowercase())
    } else {
        Err("Das ist keine gültige Client-ID. Erwartet wird die Anwendungs-(Client-)ID der Entra-App im Format 8-4-4-4-12 (Hexzeichen).".to_string())
    }
}

/// Verzeichnis (Tenant): leer = `common`; sonst `common`, `organizations`,
/// `consumers`, eine Verzeichnis-ID oder eine Domain. Steht in einer Adresse, daher
/// nur `a-z 0-9 . -`.
pub fn validate_tenant(raw: &str) -> Result<String, String> {
    let t = raw.trim().to_ascii_lowercase();
    if t.is_empty() {
        return Ok(DEFAULT_TENANT.to_string());
    }
    let ok = t.len() <= 128
        && t.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && !t.starts_with(['.', '-'])
        && !t.ends_with(['.', '-'])
        && !t.contains("..");
    if ok {
        Ok(t)
    } else {
        Err("Ungültiges Verzeichnis (Tenant). Erlaubt sind common, organizations, consumers, eine Verzeichnis-ID oder eine Domain.".to_string())
    }
}

/// Einstellungswert der Client-ID: leer = `None`, sonst eine gueltige GUID.
pub fn normalize_client_id_setting(raw: Option<&str>) -> Result<Option<String>, String> {
    match raw.map(str::trim).filter(|r| !r.is_empty()) {
        None => Ok(None),
        Some(id) => validate_client_id(id).map(Some),
    }
}

/// Einstellungswert des Verzeichnisses: leer oder `common` = `None` (Standard).
pub fn normalize_tenant_setting(raw: Option<&str>) -> Result<Option<String>, String> {
    match raw.map(str::trim).filter(|r| !r.is_empty()) {
        None => Ok(None),
        Some(t) => {
            let t = validate_tenant(t)?;
            Ok((t != DEFAULT_TENANT).then_some(t))
        }
    }
}

// ---------------------------------------------------------------------------
// PKCE
// ---------------------------------------------------------------------------

/// 32 Zufallsbytes aus dem Betriebssystem, base64url ohne Fuellzeichen (43 Zeichen).
pub fn new_verifier() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(&mut *bytes);
    Zeroizing::new(URL_SAFE_NO_PAD.encode(*bytes))
}

/// `BASE64URL(SHA256(verifier))` (RFC 7636, Methode `S256`).
pub fn challenge_s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Zufaelliger, einmaliger `state` (CSRF-Schutz der Umleitung).
pub fn new_state() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

// ---------------------------------------------------------------------------
// Endpunkte
// ---------------------------------------------------------------------------

/// Basis-Adressen; im Betrieb Microsoft, in Tests ein lokaler Server.
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub authority: String,
    pub graph: String,
    /// Proxy-Umgebungsvariablen beachten (in Tests aus).
    pub use_env_proxy: bool,
}

impl Endpoints {
    pub fn production() -> Self {
        Self {
            authority: AUTHORITY.to_string(),
            graph: GRAPH_BASE.to_string(),
            use_env_proxy: true,
        }
    }

    fn authorize_endpoint(&self, tenant: &str) -> String {
        format!(
            "{}/{tenant}/oauth2/v2.0/authorize",
            self.authority.trim_end_matches('/')
        )
    }

    fn token_endpoint(&self, tenant: &str) -> String {
        format!(
            "{}/{tenant}/oauth2/v2.0/token",
            self.authority.trim_end_matches('/')
        )
    }

    fn graph_base(&self) -> &str {
        self.graph.trim_end_matches('/')
    }
}

pub(crate) fn http_client(ep: &Endpoints) -> Result<reqwest::Client, GraphError> {
    let mut builder = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        // Keine Umleitung verfolgen: das Zugriffstoken darf nirgends anders hin.
        .redirect(Policy::none())
        .user_agent(concat!(
            "LocalVoiceAI/",
            env!("CARGO_PKG_VERSION"),
            " (calendar)"
        ));
    if !ep.use_env_proxy {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|e| GraphError::Network(e.without_url().to_string()))
}

/// Die Anmelde-Adresse fuer den Systembrowser (Kalender: Lese-Scopes `SCOPES`).
pub fn authorize_url(
    ep: &Endpoints,
    tenant: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> Result<String, GraphError> {
    authorize_url_scoped(
        ep,
        tenant,
        client_id,
        redirect_uri,
        state,
        challenge,
        SCOPES,
    )
}

/// Wie `authorize_url`, mit frei gewaehlten Scopes (Microsoft-365-Konto, A5: nur die
/// Scopes der eingeschalteten Faehigkeiten).
pub fn authorize_url_scoped(
    ep: &Endpoints,
    tenant: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
    scope: &str,
) -> Result<String, GraphError> {
    let mut url = url::Url::parse(&ep.authorize_endpoint(tenant))
        .map_err(|_| GraphError::Config("Ungültige Anmelde-Adresse.".to_string()))?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_mode", "query")
        .append_pair("scope", scope)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", "select_account");
    Ok(url.into())
}

// ---------------------------------------------------------------------------
// Loopback-Listener
// ---------------------------------------------------------------------------

/// Was die Umleitung geliefert hat.
enum Callback {
    Code(Zeroizing<String>),
    Error { code: String, description: String },
}

impl std::fmt::Debug for Callback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Callback::Code(_) => write!(f, "Callback::Code(<verborgen>)"),
            Callback::Error { code, .. } => write!(f, "Callback::Error({code})"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Code(String),
    ProviderError {
        code: String,
        description: String,
    },
    /// Anfrage abgewiesen; das Warten laeuft weiter.
    Reject(u16),
}

/// Prueft eine Anfrage an den Listener. Nur `GET /` mit erwartetem Host und
/// richtigem `state` zaehlt; alles andere wird abgewiesen, ohne etwas zu bewirken.
fn parse_request(head: &str, port: u16, expected_state: &str) -> Parsed {
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split(' ');
    let (method, target) = (parts.next(), parts.next());
    if method != Some("GET") {
        return Parsed::Reject(405);
    }
    let Some(target) = target.filter(|t| t.starts_with('/')) else {
        return Parsed::Reject(400);
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" {
        return Parsed::Reject(404);
    }
    // DNS-Rebinding: eine fremde Seite kann einen Namen auf 127.0.0.1 zeigen lassen.
    let host = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim().to_ascii_lowercase());
    let allowed = [format!("localhost:{port}"), format!("127.0.0.1:{port}")];
    if !host.is_some_and(|h| allowed.contains(&h)) {
        return Parsed::Reject(400);
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut description = String::new();
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            "error_description" => description = v.into_owned(),
            _ => {}
        }
    }
    if !state.is_some_and(|s| ct_eq(s.as_bytes(), expected_state.as_bytes())) {
        return Parsed::Reject(400);
    }
    if let Some(code) = error {
        return Parsed::ProviderError {
            code: code
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(60)
                .collect(),
            description,
        };
    }
    match code {
        Some(c) if !c.is_empty() && c.chars().count() <= MAX_CODE_CHARS => Parsed::Code(c),
        _ => Parsed::Reject(400),
    }
}

const PAGE_OK: &str = "<!doctype html><html lang=\"de\"><head><meta charset=\"utf-8\"><title>Local Voice AI</title></head><body style=\"font-family:sans-serif;max-width:32em;margin:4em auto;padding:0 1em\"><h1>Anmeldung abgeschlossen</h1><p>Du kannst dieses Fenster schließen und zu Local Voice AI zurückkehren.</p></body></html>";
const PAGE_DENIED: &str = "<!doctype html><html lang=\"de\"><head><meta charset=\"utf-8\"><title>Local Voice AI</title></head><body style=\"font-family:sans-serif;max-width:32em;margin:4em auto;padding:0 1em\"><h1>Anmeldung nicht abgeschlossen</h1><p>Bitte kehre zu Local Voice AI zurück; dort steht der Grund.</p></body></html>";
const PAGE_INVALID: &str = "<!doctype html><html lang=\"de\"><head><meta charset=\"utf-8\"><title>Local Voice AI</title></head><body style=\"font-family:sans-serif;max-width:32em;margin:4em auto;padding:0 1em\"><h1>Ungültige Anfrage</h1></body></html>";

fn http_response(status: u16, body: &str) -> Vec<u8> {
    let text = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Bad Request",
    };
    format!(
        "HTTP/1.1 {status} {text}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Liest den Kopf einer Anfrage (bis zur Leerzeile, hoechstens 8 KB).
async fn read_head(sock: &mut TcpStream) -> Option<String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            buf.truncate(pos);
            return Some(String::from_utf8_lossy(&buf).into_owned());
        }
        if buf.len() > MAX_HEAD_BYTES {
            return None;
        }
        match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
}

async fn handle_connection(
    mut sock: TcpStream,
    port: u16,
    state: Arc<str>,
    tx: mpsc::UnboundedSender<Callback>,
) {
    let head = match tokio::time::timeout(READ_HEAD_TIMEOUT, read_head(&mut sock)).await {
        Ok(Some(head)) => head,
        // Leere Vorab-Verbindung des Browsers oder zu langsam: wegwerfen.
        _ => return,
    };
    let (response, callback) = match parse_request(&head, port, &state) {
        Parsed::Reject(status) => (http_response(status, PAGE_INVALID), None),
        Parsed::Code(code) => (
            http_response(200, PAGE_OK),
            Some(Callback::Code(Zeroizing::new(code))),
        ),
        Parsed::ProviderError { code, description } => (
            http_response(200, PAGE_DENIED),
            Some(Callback::Error { code, description }),
        ),
    };
    let _ = sock.write_all(&response).await;
    let _ = sock.shutdown().await;
    if let Some(cb) = callback {
        let _ = tx.send(cb);
    }
}

fn callback_error(code: &str, description: &str) -> GraphError {
    let desc = short_description(description);
    if code == "access_denied" {
        return GraphError::Denied(if desc.is_empty() {
            "Die Anmeldung wurde im Browser abgelehnt oder abgebrochen.".to_string()
        } else {
            desc
        });
    }
    GraphError::Denied(if desc.is_empty() {
        format!("Fehler „{code}“ bei der Anmeldung.")
    } else {
        desc
    })
}

/// Der Listener auf `127.0.0.1` fuer die Umleitung nach der Anmeldung.
pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

impl Loopback {
    /// Bindet einen freien Port auf `127.0.0.1` (nie auf allen Schnittstellen).
    pub async fn bind() -> Result<Self, GraphError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| GraphError::Listener(e.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| GraphError::Listener(e.to_string()))?
            .port();
        Ok(Self { listener, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Die Weiterleitungs-URI der Anfrage: `localhost` ohne Pfad (siehe Kopf).
    pub fn redirect_uri(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    /// Wartet auf die Umleitung mit dem richtigen `state`. Gibt den Code zurueck.
    /// Endet mit `SignInTimeout` nach `timeout`, mit `Cancelled` auf `cancel`.
    /// Der Listener wird beim Zurueckkehren geschlossen (der Port ist frei).
    pub async fn wait_for_code(
        self,
        state: &str,
        timeout: Duration,
        cancel: &Notify,
    ) -> Result<Zeroizing<String>, GraphError> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Callback>();
        let state: Arc<str> = Arc::from(state);
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => return Err(GraphError::SignInTimeout),
                _ = cancel.notified() => return Err(GraphError::Cancelled),
                msg = rx.recv() => match msg {
                    Some(Callback::Code(code)) => return Ok(code),
                    Some(Callback::Error { code, description }) => {
                        return Err(callback_error(&code, &description));
                    }
                    None => {}
                },
                accepted = self.listener.accept() => match accepted {
                    Ok((sock, peer)) => {
                        if peer.ip().is_loopback() {
                            tokio::spawn(handle_connection(
                                sock,
                                self.port,
                                state.clone(),
                                tx.clone(),
                            ));
                        }
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                },
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

pub struct Tokens {
    pub access_token: Zeroizing<String>,
    pub refresh_token: Option<Zeroizing<String>>,
    pub expires_in: Duration,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tokens(<verborgen>, expires_in={:?})", self.expires_in)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Code,
    Refresh,
}

fn parse_tokens(body: &[u8]) -> Result<Tokens, GraphError> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|_| GraphError::Parse("Antwort des Token-Endpunkts".to_string()))?;
    let take = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| Zeroizing::new(s.to_string()))
    };
    let access_token = take("access_token")
        .ok_or_else(|| GraphError::Parse("Antwort ohne Zugriffstoken".to_string()))?;
    let expires = v
        .get("expires_in")
        .and_then(|e| {
            e.as_u64()
                .or_else(|| e.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(3600);
    Ok(Tokens {
        access_token,
        refresh_token: take("refresh_token"),
        expires_in: Duration::from_secs(expires),
    })
}

fn token_error(status: u16, body: &[u8], kind: TokenKind) -> GraphError {
    if status >= 500 {
        return GraphError::Http(status);
    }
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let code: String = v
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(60)
        .collect();
    let desc = short_description(
        v.get("error_description")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
    if kind == TokenKind::Refresh
        && matches!(
            code.as_str(),
            "invalid_grant" | "interaction_required" | "login_required" | "consent_required"
        )
    {
        return GraphError::NeedsSignIn;
    }
    GraphError::Denied(if desc.is_empty() {
        format!("Fehler „{code}“ (HTTP {status}).")
    } else {
        desc
    })
}

/// Liest den Koerper einer Antwort mit Obergrenze (auch ohne `Content-Length`).
pub(crate) async fn read_limited(
    mut resp: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, GraphError> {
    if resp.content_length().is_some_and(|l| l > max as u64) {
        return Err(GraphError::Parse("Antwort zu groß".to_string()));
    }
    let mut body: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > max {
                    body.zeroize();
                    return Err(GraphError::Parse("Antwort zu groß".to_string()));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(body),
            Err(e) => return Err(map_reqwest(e)),
        }
    }
}

async fn post_token(
    ep: &Endpoints,
    tenant: &str,
    form: &[(&str, &str)],
    kind: TokenKind,
) -> Result<Tokens, GraphError> {
    let client = http_client(ep)?;
    let resp = client
        .post(ep.token_endpoint(tenant))
        .header(ACCEPT, "application/json")
        .form(form)
        .send()
        .await
        .map_err(map_reqwest)?;
    let status = resp.status().as_u16();
    let retry = resp
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let mut body = read_limited(resp, MAX_TOKEN_RESPONSE_BYTES).await?;
    let result = if (200..300).contains(&status) {
        parse_tokens(&body)
    } else if status == 429 {
        Err(GraphError::Throttled {
            retry_after: parse_retry_after(retry.as_deref(), chrono::Utc::now()),
        })
    } else {
        Err(token_error(status, &body, kind))
    };
    body.zeroize();
    result
}

/// Tauscht den Code gegen Token (mit dem PKCE-Verifier, ohne Client-Geheimnis).
pub async fn exchange_code(
    ep: &Endpoints,
    client_id: &str,
    tenant: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Tokens, GraphError> {
    exchange_code_scoped(ep, client_id, tenant, code, verifier, redirect_uri, SCOPES).await
}

/// Wie `exchange_code`, mit frei gewaehlten Scopes (dieselben wie in der Anmelde-Adresse).
pub async fn exchange_code_scoped(
    ep: &Endpoints,
    client_id: &str,
    tenant: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    scope: &str,
) -> Result<Tokens, GraphError> {
    post_token(
        ep,
        tenant,
        &[
            ("client_id", client_id),
            ("scope", scope),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ],
        TokenKind::Code,
    )
    .await
}

/// Erneuert das Zugriffstoken. Microsoft liefert meist ein neues Erneuerungs-Token mit.
pub async fn refresh_tokens(
    ep: &Endpoints,
    client_id: &str,
    tenant: &str,
    refresh_token: &str,
) -> Result<Tokens, GraphError> {
    refresh_tokens_scoped(ep, client_id, tenant, refresh_token, SCOPES).await
}

/// Wie `refresh_tokens`, mit frei gewaehlten Scopes (hoechstens die der Anmeldung).
pub async fn refresh_tokens_scoped(
    ep: &Endpoints,
    client_id: &str,
    tenant: &str,
    refresh_token: &str,
    scope: &str,
) -> Result<Tokens, GraphError> {
    post_token(
        ep,
        tenant,
        &[
            ("client_id", client_id),
            ("scope", scope),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ],
        TokenKind::Refresh,
    )
    .await
}

// ---------------------------------------------------------------------------
// Geheimnis (Erneuerungs-Token) und Zugriffstoken-Cache
// ---------------------------------------------------------------------------

/// Was DPAPI-verschluesselt in `secrets/<quellen-id>.bin` steht. Client-ID und
/// Verzeichnis der Anmeldung bleiben beim Konto, auch wenn der Nutzer den
/// Einstellungswert spaeter aendert.
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredAccount {
    v: u32,
    pub client_id: String,
    pub tenant: String,
    refresh_token: String,
}

impl StoredAccount {
    pub fn new(client_id: String, tenant: String, refresh_token: String) -> Self {
        Self {
            v: 1,
            client_id,
            tenant,
            refresh_token,
        }
    }

    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }
}

impl Drop for StoredAccount {
    fn drop(&mut self) {
        self.refresh_token.zeroize();
    }
}

impl std::fmt::Debug for StoredAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StoredAccount({}, {})", self.client_id, self.tenant)
    }
}

/// Zugriff auf den Geheimnisspeicher; im Betrieb `calendar::secret` (DPAPI).
pub trait SecretVault: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String>;
    fn put(&self, name: &str, data: &[u8]) -> Result<(), String>;
    fn delete(&self, name: &str);
}

pub struct SystemVault;

impl SecretVault for SystemVault {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        secret::secret_get(name)
    }
    fn put(&self, name: &str, data: &[u8]) -> Result<(), String> {
        secret::secret_put(name, data)
    }
    fn delete(&self, name: &str) {
        secret::secret_delete(name)
    }
}

/// Fehlt das Geheimnis oder ist es unlesbar/kaputt, muss der Nutzer sich neu anmelden.
fn load_account(vault: &dyn SecretVault, name: &str) -> Result<StoredAccount, GraphError> {
    let bytes = match vault.get(name) {
        Ok(Some(b)) => b,
        Ok(None) | Err(_) => return Err(GraphError::NeedsSignIn),
    };
    let account: StoredAccount =
        serde_json::from_slice(&bytes).map_err(|_| GraphError::NeedsSignIn)?;
    if account.v != 1 || account.refresh_token.is_empty() {
        return Err(GraphError::NeedsSignIn);
    }
    Ok(account)
}

fn save_account(
    vault: &dyn SecretVault,
    name: &str,
    account: &StoredAccount,
) -> Result<(), String> {
    let bytes = Zeroizing::new(
        serde_json::to_vec(account).map_err(|_| "Das Konto ist nicht speicherbar.".to_string())?,
    );
    vault.put(name, &bytes)
}

/// Zugriffstoken im Arbeitsspeicher je Quelle (nie auf dem Datentraeger).
#[derive(Default)]
pub struct TokenCache(Mutex<HashMap<String, (Zeroizing<String>, Instant)>>);

impl TokenCache {
    pub fn get(&self, name: &str) -> Option<Zeroizing<String>> {
        let map = lock(&self.0);
        map.get(name)
            .filter(|(_, expires)| *expires > Instant::now())
            .map(|(token, _)| token.clone())
    }

    pub fn put(&self, name: &str, token: Zeroizing<String>, expires_in: Duration) {
        let valid = expires_in.saturating_sub(TOKEN_SAFETY_MARGIN);
        lock(&self.0).insert(name.to_string(), (token, Instant::now() + valid));
    }

    pub fn clear(&self, name: &str) {
        lock(&self.0).remove(name);
    }
}

/// Ein gueltiges Zugriffstoken: aus dem Cache, sonst per Erneuerungs-Token. Ein
/// neues Erneuerungs-Token ersetzt das alte atomar; scheitert das Schreiben, laeuft
/// dieser Abruf trotzdem mit dem Zugriffstoken weiter (das alte Erneuerungs-Token
/// bleibt gueltig).
pub async fn access_token(
    ep: &Endpoints,
    vault: &dyn SecretVault,
    cache: &TokenCache,
    name: &str,
    force_refresh: bool,
) -> Result<Zeroizing<String>, GraphError> {
    if !force_refresh {
        if let Some(token) = cache.get(name) {
            return Ok(token);
        }
    }
    let account = load_account(vault, name)?;
    let tokens = match refresh_tokens(
        ep,
        &account.client_id,
        &account.tenant,
        account.refresh_token(),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            cache.clear(name);
            return Err(e);
        }
    };
    if let Some(new) = &tokens.refresh_token {
        if new.as_str() != account.refresh_token() {
            let updated = StoredAccount::new(
                account.client_id.clone(),
                account.tenant.clone(),
                new.to_string(),
            );
            if let Err(e) = save_account(vault, name, &updated) {
                log::warn!("calendar: graph refresh token not saved: {e}");
            }
        }
    }
    cache.put(name, tokens.access_token.clone(), tokens.expires_in);
    Ok(tokens.access_token)
}

// ---------------------------------------------------------------------------
// Anmeldung
// ---------------------------------------------------------------------------

pub struct SignedIn {
    pub access_token: Zeroizing<String>,
    pub expires_in: Duration,
    pub account: StoredAccount,
}

impl std::fmt::Debug for SignedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SignedIn(<verborgen>)")
    }
}

/// Interaktive Anmeldung: Listener binden, Adresse im Systembrowser oeffnen
/// (`open`), auf die Umleitung warten, Code gegen Token tauschen.
pub async fn sign_in(
    ep: &Endpoints,
    client_id: &str,
    tenant: &str,
    open: impl FnOnce(String) -> Result<(), String>,
    timeout: Duration,
    cancel: &Notify,
) -> Result<SignedIn, GraphError> {
    sign_in_scoped(ep, client_id, tenant, SCOPES, open, timeout, cancel).await
}

/// Wie `sign_in`, mit frei gewaehlten Scopes (Microsoft-365-Konto, A5).
pub async fn sign_in_scoped(
    ep: &Endpoints,
    client_id: &str,
    tenant: &str,
    scope: &str,
    open: impl FnOnce(String) -> Result<(), String>,
    timeout: Duration,
    cancel: &Notify,
) -> Result<SignedIn, GraphError> {
    let client_id = validate_client_id(client_id).map_err(GraphError::Config)?;
    let tenant = validate_tenant(tenant).map_err(GraphError::Config)?;
    let listener = Loopback::bind().await?;
    let redirect_uri = listener.redirect_uri();
    let verifier = new_verifier();
    let state = new_state();
    let url = authorize_url_scoped(
        ep,
        &tenant,
        &client_id,
        &redirect_uri,
        &state,
        &challenge_s256(&verifier),
        scope,
    )?;
    // Scheitert das Oeffnen, wird der Listener beim Verlassen geschlossen.
    open(url).map_err(GraphError::Browser)?;
    let code = listener.wait_for_code(&state, timeout, cancel).await?;
    let tokens = exchange_code_scoped(
        ep,
        &client_id,
        &tenant,
        &code,
        &verifier,
        &redirect_uri,
        scope,
    )
    .await?;
    let Some(refresh) = tokens.refresh_token.as_ref() else {
        return Err(GraphError::Denied(
            "Microsoft hat kein Erneuerungs-Token geliefert. Prüfe, ob die App die Berechtigung offline_access nutzen darf."
                .to_string(),
        ));
    };
    Ok(SignedIn {
        access_token: tokens.access_token.clone(),
        expires_in: tokens.expires_in,
        account: StoredAccount::new(client_id, tenant, refresh.to_string()),
    })
}

/// Zustand des Dienstes: Zugriffstoken, Drosselung, laufende Anmeldung.
#[derive(Default)]
pub struct GraphState {
    pub tokens: TokenCache,
    throttle: Mutex<HashMap<String, Instant>>,
    active: Mutex<Option<Arc<Notify>>>,
}

/// Haelt die laufende Anmeldung fest; beim Verwerfen ist sie beendet.
pub struct ActiveSignIn<'a> {
    state: &'a GraphState,
}

impl std::fmt::Debug for ActiveSignIn<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ActiveSignIn")
    }
}

impl Drop for ActiveSignIn<'_> {
    fn drop(&mut self) {
        *lock(&self.state.active) = None;
    }
}

impl GraphState {
    /// Beginnt eine Anmeldung; eine zweite gleichzeitige wird abgewiesen.
    pub fn begin_sign_in(&self) -> Result<(Arc<Notify>, ActiveSignIn<'_>), GraphError> {
        let mut active = lock(&self.active);
        if active.is_some() {
            return Err(GraphError::Config(
                "Es läuft bereits eine Anmeldung. Schließe sie ab oder brich sie ab.".to_string(),
            ));
        }
        let cancel = Arc::new(Notify::new());
        *active = Some(cancel.clone());
        Ok((cancel, ActiveSignIn { state: self }))
    }

    /// Laeuft gerade eine Anmeldung?
    pub fn is_signing_in(&self) -> bool {
        lock(&self.active).is_some()
    }

    /// Bricht die laufende Anmeldung ab; `false`, wenn keine laeuft.
    pub fn cancel_sign_in(&self) -> bool {
        match lock(&self.active).as_ref() {
            Some(cancel) => {
                cancel.notify_one();
                true
            }
            None => false,
        }
    }

    /// Merkt `Retry-After` einer Quelle.
    pub fn throttle(&self, source_id: &str, wait: Duration) {
        lock(&self.throttle).insert(source_id.to_string(), Instant::now() + wait);
    }

    /// Verbleibende Wartezeit nach einem 429, sonst `None`.
    pub fn throttle_left(&self, source_id: &str) -> Option<Duration> {
        let mut map = lock(&self.throttle);
        match map.get(source_id).copied() {
            Some(until) if until > Instant::now() => Some(until - Instant::now()),
            Some(_) => {
                map.remove(source_id);
                None
            }
            None => None,
        }
    }

    /// Vergisst alles zu einer Quelle (Abmelden).
    pub fn forget(&self, source_id: &str) {
        self.tokens.clear(source_id);
        lock(&self.throttle).remove(source_id);
    }
}

// ---------------------------------------------------------------------------
// Graph-Abruf
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Me {
    /// Klein geschrieben.
    pub address: String,
    pub name: Option<String>,
}

async fn graph_get(client: &reqwest::Client, url: &str, token: &str) -> Result<Value, GraphError> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .header(ACCEPT, "application/json")
        // Zeiten in UTC anfordern (auch fuer Folgeseiten).
        .header("Prefer", "outlook.timezone=\"UTC\"")
        .send()
        .await
        .map_err(map_reqwest)?;
    let status = resp.status().as_u16();
    match status {
        200..=299 => {}
        401 => return Err(GraphError::NeedsSignIn),
        403 => {
            return Err(GraphError::Denied(
                "Zugriff auf den Kalender verweigert (HTTP 403). Fehlt der App die Berechtigung Calendars.Read, oder sperrt der Mandant sie?"
                    .to_string(),
            ))
        }
        429 => {
            let retry = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            return Err(GraphError::Throttled {
                retry_after: parse_retry_after(retry.as_deref(), chrono::Utc::now()),
            });
        }
        other => return Err(GraphError::Http(other)),
    }
    let body = read_limited(resp, MAX_PAGE_BYTES).await?;
    serde_json::from_slice(&body)
        .map_err(|_| GraphError::Parse("keine gültige JSON-Antwort".to_string()))
}

/// Eigene Adresse und Name (`GET /me`).
pub async fn fetch_me(ep: &Endpoints, token: &str) -> Result<Me, GraphError> {
    let client = http_client(ep)?;
    let url = format!(
        "{}/me?$select=displayName,mail,userPrincipalName",
        ep.graph_base()
    );
    let v = graph_get(&client, &url, token).await?;
    let text = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let address = text("mail")
        .filter(|a| a.contains('@'))
        .or_else(|| text("userPrincipalName").filter(|a| a.contains('@')))
        .map(str::to_lowercase)
        .ok_or_else(|| GraphError::Parse("Konto ohne E-Mail-Adresse".to_string()))?;
    Ok(Me {
        address,
        name: text("displayName").map(str::to_string),
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphFetch {
    pub events: Vec<CalEvent>,
    /// Klartext-Hinweise (uebersprungene Termine).
    pub warnings: Vec<String>,
}

fn iso_utc(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn same_origin(a: &url::Url, b: &url::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Termine im Fenster `[from_ms, to_ms)` (`GET /me/calendarView`), mit Paging.
/// `me_email`: die eigene Adresse fuer `is_self`.
pub async fn fetch_events(
    ep: &Endpoints,
    token: &str,
    source_id: &str,
    from_ms: i64,
    to_ms: i64,
    me_email: Option<&str>,
) -> Result<GraphFetch, GraphError> {
    let client = http_client(ep)?;
    let base = url::Url::parse(ep.graph_base())
        .map_err(|_| GraphError::Config("Ungültige Graph-Adresse.".to_string()))?;
    let enc = |s: &str| -> String { url::form_urlencoded::byte_serialize(s.as_bytes()).collect() };
    let mut next = format!(
        "{}/me/calendarView?startDateTime={}&endDateTime={}&$select={}&$top={PAGE_SIZE}",
        ep.graph_base(),
        enc(&iso_utc(from_ms)),
        enc(&iso_utc(to_ms)),
        enc(EVENT_SELECT),
    );
    let me = me_email.map(str::to_lowercase);
    let mut out = GraphFetch::default();
    for _ in 0..MAX_PAGES {
        let page = graph_get(&client, &next, token).await?;
        if let Some(items) = page.get("value").and_then(Value::as_array) {
            for item in items {
                match parse_event(item, source_id, me.as_deref()) {
                    Ok(event) => out.events.push(event),
                    Err(why) => {
                        if out.warnings.len() < 100 {
                            out.warnings.push(why);
                        }
                    }
                }
            }
        }
        let Some(link) = page.get("@odata.nextLink").and_then(Value::as_str) else {
            out.events.sort_by(|a, b| {
                a.starts_at
                    .cmp(&b.starts_at)
                    .then_with(|| a.uid.cmp(&b.uid))
            });
            return Ok(out);
        };
        // Das Zugriffstoken geht nur an den Graph-Host, nie an einen Link aus der Antwort.
        let parsed = url::Url::parse(link)
            .map_err(|_| GraphError::Parse("ungültiger Folgelink".to_string()))?;
        if !same_origin(&parsed, &base) {
            return Err(GraphError::Parse(
                "Folgelink zeigt auf einen fremden Host".to_string(),
            ));
        }
        next = link.to_string();
    }
    Err(GraphError::Parse(format!(
        "Der Kalender hat mehr als {MAX_PAGES} Seiten."
    )))
}

/// Abruf mit einmaligem Erneuern bei 401: erst das Token erneuern und wiederholen;
/// scheitert es wieder, ist die Anmeldung noetig.
pub async fn sync_events(
    ep: &Endpoints,
    vault: &dyn SecretVault,
    cache: &TokenCache,
    name: &str,
    from_ms: i64,
    to_ms: i64,
    me_email: Option<&str>,
) -> Result<GraphFetch, GraphError> {
    let token = access_token(ep, vault, cache, name, false).await?;
    match fetch_events(ep, &token, name, from_ms, to_ms, me_email).await {
        Err(GraphError::NeedsSignIn) => {
            cache.clear(name);
            let token = access_token(ep, vault, cache, name, true).await?;
            match fetch_events(ep, &token, name, from_ms, to_ms, me_email).await {
                Err(GraphError::NeedsSignIn) => {
                    cache.clear(name);
                    Err(GraphError::NeedsSignIn)
                }
                other => other,
            }
        }
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Termine lesen
// ---------------------------------------------------------------------------

/// Nur bei Graph ist die Zeit garantiert UTC (`Prefer`-Kopf); alles andere wird
/// nicht geraten.
fn parse_graph_time(v: Option<&Value>) -> Result<i64, String> {
    let v = v.ok_or("Zeitangabe fehlt")?;
    let dt = v
        .get("dateTime")
        .and_then(Value::as_str)
        .ok_or("Zeitangabe fehlt")?;
    let tz = v.get("timeZone").and_then(Value::as_str).unwrap_or("UTC");
    if !(tz.eq_ignore_ascii_case("UTC") || tz.eq_ignore_ascii_case("Etc/UTC")) {
        return Err(format!("Zeitzone „{tz}“ statt UTC"));
    }
    let s = dt.trim().trim_end_matches(['Z', 'z']);
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .map_err(|_| format!("Zeit „{dt}“ nicht lesbar"))?;
    Ok(naive.and_utc().timestamp_millis())
}

fn text_of<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn partstat_of(response: Option<&str>) -> Option<String> {
    match response? {
        "accepted" => Some("ACCEPTED"),
        "declined" => Some("DECLINED"),
        "tentativelyAccepted" => Some("TENTATIVE"),
        "notResponded" => Some("NEEDS-ACTION"),
        _ => None,
    }
    .map(str::to_string)
}

fn build_attendees(item: &Value, me: Option<&str>) -> Vec<Attendee> {
    const MAX_ATTENDEES: usize = 500;
    let mut out: Vec<Attendee> = Vec::new();
    // Gleiche Adresse = gleiche Person; ohne Adresse zaehlt der Name.
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |entry: &Value, organizer: bool, response: Option<&str>| {
        if out.len() >= MAX_ATTENDEES {
            return;
        }
        let email = entry
            .get("address")
            .and_then(Value::as_str)
            .and_then(ics::normalize_email);
        // Exchange liefert bei internen Adressen teils `/O=...`: das ist keine Adresse
        // (`normalize_email` verwirft sie), der Name bleibt.
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .filter(|n| email.as_deref() != Some(n.to_lowercase().as_str()))
            .map(str::to_string);
        if email.is_none() && name.is_none() {
            return;
        }
        let identity = match (&email, &name) {
            (Some(e), _) => format!("e:{e}"),
            (None, Some(n)) => format!("n:{}", n.to_lowercase()),
            (None, None) => return,
        };
        if !seen.insert(identity) {
            return;
        }
        out.push(Attendee {
            is_self: me.is_some_and(|m| email.as_deref() == Some(m)),
            email,
            name,
            organizer,
            partstat: partstat_of(response),
        });
    };
    if let Some(o) = item.pointer("/organizer/emailAddress") {
        push(o, true, None);
    }
    if let Some(list) = item.get("attendees").and_then(Value::as_array) {
        for a in list {
            // Besprechungsraeume sind keine Teilnehmenden.
            if a.get("type").and_then(Value::as_str) == Some("resource") {
                continue;
            }
            let Some(entry) = a.get("emailAddress") else {
                continue;
            };
            push(
                entry,
                false,
                a.pointer("/status/response").and_then(Value::as_str),
            );
        }
    }
    out
}

/// Ein Termin aus der Graph-Antwort. `Err` = uebersprungen, mit Klartext.
pub fn parse_event(item: &Value, source_id: &str, me: Option<&str>) -> Result<CalEvent, String> {
    let subject = text_of(item, "/subject");
    let label = subject
        .unwrap_or(ics::UNTITLED)
        .chars()
        .take(60)
        .collect::<String>();
    let skip = |why: String| format!("Termin „{label}“ übersprungen: {why}");
    let uid = text_of(item, "/iCalUId")
        .map(clean_global_object_id)
        .or_else(|| text_of(item, "/id").map(str::to_string))
        .ok_or_else(|| skip("ohne Kennung".to_string()))?;
    let starts_at = parse_graph_time(item.get("start")).map_err(&skip)?;
    let ends_at = parse_graph_time(item.get("end"))
        .map_err(&skip)?
        .max(starts_at);
    let location =
        text_of(item, "/location/displayName").map(|l| ics::cap_chars(l, ics::MAX_LOCATION_CHARS));
    let preview = text_of(item, "/bodyPreview");
    let join_url = text_of(item, "/onlineMeeting/joinUrl")
        .filter(|u| ics::is_http_url(u) && u.chars().count() <= ics::MAX_JOIN_URL_CHARS)
        .map(str::to_string)
        .or_else(|| {
            let mut fields: Vec<&str> = Vec::new();
            fields.extend(location.as_deref());
            fields.extend(preview);
            ics::extract_join_url(&fields)
        });
    Ok(CalEvent {
        key: event_key(source_id, &uid, starts_at),
        source_id: source_id.to_string(),
        uid,
        title: ics::cap_chars(subject.unwrap_or(ics::UNTITLED), ics::MAX_TITLE_CHARS),
        starts_at,
        ends_at,
        all_day: item
            .get("isAllDay")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        cancelled: item
            .get("isCancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        location,
        join_url,
        description: preview.map(|d| ics::cap_chars(d, ics::MAX_DESCRIPTION_CHARS)),
        attendees: build_attendees(item, me),
    })
}

// ---------------------------------------------------------------------------
// Dublette ICS/Graph: Graph gewinnt
// ---------------------------------------------------------------------------

/// Outlook-`GlobalObjectId` als Hex: 16 Byte Kennung, dann 4 Byte Datum der
/// Instanz (0 bei Serie/Einzeltermin), Erstellzeit usw. Graphs `iCalUId` traegt bei
/// Serientermin-Instanzen das Datum, die ICS-`UID` derselben Serie nicht. Das Datum
/// wird genullt (`CleanGlobalObjectId`), damit beide gleich werden. Alles andere
/// bleibt unveraendert.
pub fn clean_global_object_id(uid: &str) -> String {
    const HEADER: &str = "040000008200E00074C5B7101A82E008";
    if let (Some(head), Some(date)) = (uid.get(..32), uid.get(32..40)) {
        if head.eq_ignore_ascii_case(HEADER) && date.bytes().all(|b| b.is_ascii_hexdigit()) {
            return format!("{head}00000000{}", &uid[40..]);
        }
    }
    uid.to_string()
}

fn normalized_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Derselbe Termin aus einer ICS-Quelle und aus Graph erscheint einmal, und zwar
/// der aus Graph (mit Teilnehmenden und Beitritts-Adresse). Gleich sind zwei
/// Termine bei gleicher (bereinigter) UID und gleichem Beginn, oder bei gleichem
/// Beginn und gleichem Titel (Graph kann bei Serientermin-Instanzen eine andere UID
/// als die ICS-Datei tragen). Nur Termine NICHT-Graph-Quellen werden ausgeblendet.
pub fn dedupe_graph_wins(events: Vec<CalEvent>, graph_sources: &HashSet<String>) -> Vec<CalEvent> {
    if graph_sources.is_empty() {
        return events;
    }
    let mut by_uid: HashSet<(String, i64)> = HashSet::new();
    let mut by_title: HashSet<(String, i64)> = HashSet::new();
    for e in events
        .iter()
        .filter(|e| graph_sources.contains(&e.source_id))
    {
        by_uid.insert((
            clean_global_object_id(&e.uid).to_ascii_lowercase(),
            e.starts_at,
        ));
        if e.title != ics::UNTITLED {
            by_title.insert((normalized_title(&e.title), e.starts_at));
        }
    }
    if by_uid.is_empty() {
        return events;
    }
    events
        .into_iter()
        .filter(|e| {
            if graph_sources.contains(&e.source_id) {
                return true;
            }
            let same_uid = by_uid.contains(&(
                clean_global_object_id(&e.uid).to_ascii_lowercase(),
                e.starts_at,
            ));
            let same_title = e.title != ics::UNTITLED
                && by_title.contains(&(normalized_title(&e.title), e.starts_at));
            !(same_uid || same_title)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Quelle anlegen
// ---------------------------------------------------------------------------

/// `graph-` plus 16 Hexziffern; zugleich Dateiname des Geheimnisses.
pub fn new_graph_source_id() -> String {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("graph-{hex}")
}

/// Legt Geheimnis und Quelle fuer ein angemeldetes Konto an. Dasselbe Konto (gleiche
/// Adresse) meldet sich erneut an: die vorhandene Quelle bekommt das neue Token,
/// es entsteht keine zweite. Reihenfolge: erst das Geheimnis, dann die Quelle; scheitert
/// eines, bleibt nichts Halbes zurueck. Die eigene Adresse wird als „Ich“ vermerkt
/// (fuer Empfaenger der Follow-up-Mail und den Brief).
pub fn register_account(
    store: &MeetingStore,
    vault: &dyn SecretVault,
    account: &StoredAccount,
    me: &Me,
    now_ms: i64,
) -> Result<CalendarSource, String> {
    let existing = store
        .calendar_sources()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| {
            s.kind == CalendarKind::Graph
                && s.account_hint
                    .as_deref()
                    .is_some_and(|h| h.eq_ignore_ascii_case(&me.address))
        });
    let id = existing
        .as_ref()
        .map_or_else(new_graph_source_id, |s| s.id.clone());
    save_account(vault, &id, account)?;
    if existing.is_some() {
        store
            .calendar_source_set_enabled(&id, true, now_ms)
            .map_err(|e| e.to_string())?;
    } else if let Err(e) = store.calendar_source_add(
        &id,
        CalendarKind::Graph,
        SOURCE_LABEL,
        Some(&me.address),
        now_ms,
    ) {
        vault.delete(&id);
        return Err(format!("Die Quelle konnte nicht gespeichert werden: {e}"));
    }
    match store.upsert_person(Some(&me.address), me.name.as_deref(), "calendar") {
        Ok(person) => {
            if let Err(e) = store.mark_person_self(&person) {
                log::warn!("calendar: could not mark own person: {e}");
            }
        }
        Err(e) => log::warn!("calendar: could not store own person: {e}"),
    }
    store
        .calendar_source(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Die Quelle ist nicht mehr da.".to_string())
}

#[cfg(test)]
mod tests;
