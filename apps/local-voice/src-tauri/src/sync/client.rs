//! HTTP-Client zum Hub (wai-portal, `/api/v1/voice/*`). Nur Chiffrate gehen
//! über die Leitung; dieser Baustein kennt keinen Schlüssel.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug)]
pub enum HubError {
    /// Token widerrufen oder abgelaufen — Konto lokal als abgemeldet behandeln.
    Unauthorized,
    /// Pull-Cursor liegt hinter der Log-Retention — mit 0 neu laden.
    CursorExpired,
    /// Alles andere: Netz, 5xx, unerwartete Antwort. Text für den Status.
    Other(String),
}

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HubError::Unauthorized => write!(f, "Anmeldung abgelaufen — bitte neu anmelden"),
            HubError::CursorExpired => write!(f, "Cursor abgelaufen"),
            HubError::Other(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct PushObject {
    pub collection: String,
    pub object_id: String,
    pub base_revision: i64,
    pub payload: String,
    pub key_id: String,
    pub deleted: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RemoteObject {
    pub collection: String,
    pub object_id: String,
    pub revision: i64,
    #[serde(default)]
    pub payload: String,
    #[serde(default)]
    pub key_id: String,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub server_seq: i64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PushResult {
    #[serde(default)]
    pub collection: String,
    #[serde(default)]
    pub object_id: String,
    pub outcome: String,
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub current: Option<RemoteObject>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct PushResponse {
    results: Vec<PushResult>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PullResponse {
    pub objects: Vec<RemoteObject>,
    pub cursor: i64,
    pub more: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, specta::Type)]
pub struct HubDevice {
    pub device: String,
    #[serde(default)]
    pub last_push: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, specta::Type)]
pub struct HubStatus {
    #[serde(default)]
    pub hub_seq: i64,
    #[serde(default)]
    pub objects: std::collections::HashMap<String, i64>,
    #[serde(default)]
    pub devices: Vec<HubDevice>,
}

/// Stand eines Blobs in der Dateiablage (`GET /api/v1/voice/blobs/{id}`).
#[derive(Deserialize, Debug, Clone, Default)]
pub struct BlobInfo {
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub chunks: Vec<BlobChunkInfo>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct BlobChunkInfo {
    pub idx: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChunkBody {
    pub data: String,
    pub sha256: String,
}

/// Ergebnis von `commit`: vollständig oder die fehlenden Stücke.
#[derive(Debug, Clone, PartialEq)]
pub enum CommitOutcome {
    Complete,
    Missing(Vec<i64>),
}

#[derive(Deserialize, Debug, Clone)]
pub struct LoginUser {
    pub id: i64,
    pub email: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct LoginResponse {
    pub token: String,
    pub user: LoginUser,
}

#[derive(Deserialize, Debug)]
struct ApiError {
    error: ApiErrorBody,
}

#[derive(Deserialize, Debug)]
struct ApiErrorBody {
    code: String,
    #[serde(default)]
    message: String,
}

fn http() -> Result<reqwest::Client, HubError> {
    http_with(Duration::from_secs(30))
}

/// Stücke der Dateiablage tragen bis zu 3,65 MiB; langsame Leitungen brauchen Zeit.
fn http_with(timeout: Duration) -> Result<reqwest::Client, HubError> {
    let ua = std::env::var("LV_SYNC_UA")
        .unwrap_or_else(|_| concat!("local-voice-ai/", env!("CARGO_PKG_VERSION")).to_string());
    let mut b = reqwest::Client::builder().timeout(timeout).user_agent(ua);
    // Diagnose-Schalter (Hub-500 nur bei POSTs der App, 06.11.2026):
    // LV_SYNC_HTTP1=1 erzwingt HTTP/1.1, LV_SYNC_IPV4=1 nur IPv4,
    // LV_SYNC_TITLECASE=1 Kopfzeilennamen wie curl (Content-Type statt content-type),
    // LV_SYNC_UA=<text> ersetzt den User-Agent.
    if std::env::var_os("LV_SYNC_HTTP1").is_some() {
        b = b.http1_only();
    }
    if std::env::var_os("LV_SYNC_IPV4").is_some() {
        b = b.local_address(Some(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)));
    }
    // LV_SYNC_NOPROXY=1 umgeht System-Proxy (reqwest liest unter Windows die
    // Internetoptionen, curl nicht).
    if std::env::var_os("LV_SYNC_NOPROXY").is_some() {
        b = b.no_proxy();
    }
    if std::env::var_os("LV_SYNC_TITLECASE").is_some() {
        b = b.http1_title_case_headers();
    }
    b.build().map_err(|e| HubError::Other(e.to_string()))
}

/// POST zum Hub. Unter Windows über das System-curl (siehe `sync::curl`),
/// sonst über reqwest. Auf reqwest wird nur zurückgefallen, wenn curl fehlt.
async fn send_logged(
    rb: reqwest::RequestBuilder,
    timeout: Duration,
) -> Result<reqwest::Response, HubError> {
    let (client, req) = rb.build_split();
    let req = req.map_err(|e| HubError::Other(e.to_string()))?;
    let req = if super::curl::enabled() {
        let (r, back) = tauri::async_runtime::spawn_blocking(move || {
            let r = super::curl::send(&req, timeout);
            (r, req)
        })
        .await
        .map_err(|e| HubError::Other(e.to_string()))?;
        match r {
            Ok(resp) => return Ok(reqwest::Response::from(resp)),
            Err(super::curl::CurlError::Missing) => {
                log::warn!("Sync: curl fehlt, sende über reqwest");
                back
            }
            Err(super::curl::CurlError::Failed(e)) => {
                return Err(HubError::Other(format!("Hub nicht erreichbar: {e}")))
            }
        }
    } else {
        req
    };
    client
        .execute(req)
        .await
        .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))
}

fn base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

async fn error_from(res: reqwest::Response) -> HubError {
    let status = res.status();
    let kind = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .filter(|v| v.len() <= 64)
        .unwrap_or("ohne Typ")
        .to_string();
    let text = res.text().await.unwrap_or_default();
    let code = serde_json::from_str::<ApiError>(&text)
        .ok()
        .map(|e| (e.error.code, e.error.message));
    match (status.as_u16(), code) {
        (401, _) => HubError::Unauthorized,
        (410, _) => HubError::CursorExpired,
        (_, Some((code, msg))) if !msg.is_empty() => HubError::Other(format!("{code}: {msg}")),
        (_, Some((code, _))) => HubError::Other(code),
        (s, None) => {
            // Kein JSON der App (Hosting oder PHP-Abbruch). Ohne Antwortauszug
            // und ohne Pfad ins Log, nur Status und Inhaltstyp.
            log::warn!("Hub: HTTP {s} ohne App-Fehler ({kind})");
            HubError::Other(format!("HTTP {s}"))
        }
    }
}

pub async fn login(
    url: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> Result<LoginResponse, HubError> {
    let res = http()?
        .post(format!("{}/api/v1/voice/auth/login", base(url)))
        .json(&serde_json::json!({ "email": email, "password": password, "device_name": device_name }))
        .send()
        .await
        .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
    if !res.status().is_success() {
        return Err(match error_from(res).await {
            // 401 heißt hier: falsche Zugangsdaten, nicht „Token abgelaufen".
            HubError::Unauthorized => HubError::Other("E-Mail oder Passwort stimmen nicht".into()),
            e => e,
        });
    }
    res.json::<LoginResponse>()
        .await
        .map_err(|e| HubError::Other(format!("Antwort unlesbar: {e}")))
}

pub struct HubClient {
    url: String,
    token: String,
}

impl HubClient {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            url: base(url),
            token: token.to_string(),
        }
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.bearer_auth(&self.token)
    }

    pub async fn push(
        &self,
        device_id: &str,
        objects: &[PushObject],
    ) -> Result<Vec<PushResult>, HubError> {
        let res = self
            // Einzelne Objekte tragen bis ~4,2 MB (lange Transkripte).
            .auth(
                http_with(Duration::from_secs(120))?
                    .post(format!("{}/api/v1/voice/sync/push", self.url)),
            )
            .json(&serde_json::json!({ "device_id": device_id, "objects": objects }));
        let res = send_logged(res, Duration::from_secs(120)).await?;
        if !res.status().is_success() {
            return Err(error_from(res).await);
        }
        res.json::<PushResponse>()
            .await
            .map(|r| r.results)
            .map_err(|e| HubError::Other(format!("Antwort unlesbar: {e}")))
    }

    pub async fn pull(&self, since: i64, limit: u32) -> Result<PullResponse, HubError> {
        let res = self
            .auth(http()?.get(format!("{}/api/v1/voice/sync/pull", self.url)))
            .query(&[("since", since.to_string()), ("limit", limit.to_string())])
            .send()
            .await
            .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
        if !res.status().is_success() {
            return Err(error_from(res).await);
        }
        res.json::<PullResponse>()
            .await
            .map_err(|e| HubError::Other(format!("Antwort unlesbar: {e}")))
    }

    /// Wie [`pull`](Self::pull), aber ueber einen wiederverwendeten Client
    /// (Verbindung bleibt offen): fuer den schnellen Takt des Abo-Relays, der
    /// alle 1,5 s fragt und nicht jedes Mal neu verbinden soll.
    pub async fn pull_shared(&self, since: i64, limit: u32) -> Result<PullResponse, HubError> {
        static SHARED: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        let client = match SHARED.get() {
            Some(c) => c.clone(),
            None => {
                let c = http_with(Duration::from_secs(15))?;
                SHARED.get_or_init(|| c).clone()
            }
        };
        let res = self
            .auth(client.get(format!("{}/api/v1/voice/sync/pull", self.url)))
            .query(&[("since", since.to_string()), ("limit", limit.to_string())])
            .send()
            .await
            .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
        if !res.status().is_success() {
            return Err(error_from(res).await);
        }
        res.json::<PullResponse>()
            .await
            .map_err(|e| HubError::Other(format!("Antwort unlesbar: {e}")))
    }

    pub async fn status(&self) -> Result<HubStatus, HubError> {
        let res = self
            .auth(http()?.get(format!("{}/api/v1/voice/sync/status", self.url)))
            .send()
            .await
            .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
        if !res.status().is_success() {
            return Err(error_from(res).await);
        }
        res.json()
            .await
            .map_err(|e| HubError::Other(format!("Antwort unlesbar: {e}")))
    }

    fn blob_url(&self, blob_id: &str) -> Result<String, HubError> {
        // Nur 64 Hex: die Kennung landet im Pfad.
        if blob_id.len() != 64 || !blob_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(HubError::Other("Ungültige Blob-Kennung".into()));
        }
        Ok(format!("{}/api/v1/voice/blobs/{blob_id}", self.url))
    }

    /// Stand eines Blobs; `None`, wenn der Hub ihn nicht kennt.
    pub async fn blob_info(&self, blob_id: &str) -> Result<Option<BlobInfo>, HubError> {
        let res = self
            .auth(http()?.get(self.blob_url(blob_id)?))
            .send()
            .await
            .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
        if res.status().as_u16() == 404 {
            return Ok(None);
        }
        if !res.status().is_success() {
            return Err(error_from(res).await);
        }
        res.json::<BlobInfo>()
            .await
            .map(Some)
            .map_err(|e| HubError::Other(format!("Antwort unlesbar: {e}")))
    }

    pub async fn put_chunk(
        &self,
        blob_id: &str,
        index: usize,
        sealed: &[u8],
    ) -> Result<(), HubError> {
        let body = ChunkBody {
            data: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, sealed),
            sha256: super::blobs::sha256_hex(sealed),
        };
        let res = self
            .auth(
                http_with(Duration::from_secs(180))?
                    .post(format!("{}/chunks/{index}", self.blob_url(blob_id)?)),
            )
            .json(&body);
        let res = send_logged(res, Duration::from_secs(180)).await?;
        if !res.status().is_success() {
            log::warn!(
                "Hub: Stück {index} abgelehnt (HTTP {}), Chiffrat {} Bytes, Body ~{} Bytes",
                res.status().as_u16(),
                sealed.len(),
                body.data.len() + 90
            );
            return Err(error_from(res).await);
        }
        Ok(())
    }

    pub async fn commit_blob(
        &self,
        blob_id: &str,
        chunks: usize,
    ) -> Result<CommitOutcome, HubError> {
        #[derive(Deserialize)]
        struct Body {
            #[serde(default)]
            outcome: String,
            #[serde(default)]
            missing: Vec<i64>,
        }
        let res = self
            .auth(http()?.post(format!("{}/commit", self.blob_url(blob_id)?)))
            .json(&serde_json::json!({ "chunks": chunks }))
            .send()
            .await
            .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
        let status = res.status().as_u16();
        if res.status().is_success() || status == 409 {
            let text = res.text().await.unwrap_or_default();
            if let Ok(body) = serde_json::from_str::<Body>(&text) {
                match body.outcome.as_str() {
                    "complete" => return Ok(CommitOutcome::Complete),
                    "incomplete" => return Ok(CommitOutcome::Missing(body.missing)),
                    _ => {}
                }
            }
            return Err(HubError::Other(format!(
                "Blob-Abschluss abgelehnt (HTTP {status})"
            )));
        }
        Err(error_from(res).await)
    }

    /// Ein versiegeltes Stück; die Prüfsumme des Transports ist geprüft.
    pub async fn get_chunk(&self, blob_id: &str, index: usize) -> Result<Vec<u8>, HubError> {
        let res = self
            .auth(
                http_with(Duration::from_secs(180))?
                    .get(format!("{}/chunks/{index}", self.blob_url(blob_id)?)),
            )
            .send()
            .await
            .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
        if !res.status().is_success() {
            return Err(error_from(res).await);
        }
        let body = res
            .json::<ChunkBody>()
            .await
            .map_err(|e| HubError::Other(format!("Antwort unlesbar: {e}")))?;
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body.data.trim())
                .map_err(|_| HubError::Other("Stück ist kein Base64".into()))?;
        if super::blobs::sha256_hex(&bytes) != body.sha256 {
            return Err(HubError::Other(
                "Prüfsumme eines Stücks stimmt nicht".into(),
            ));
        }
        Ok(bytes)
    }

    /// Aufruf der Kopplungs- und Geräte-Routen (`/api/v1/voice/pair/*`,
    /// `/api/v1/voice/devices*`). Liefert HTTP-Status und JSON-Körper, weil die
    /// Kopplung 409/410 selbst auswertet; nur Netzfehler sind `Err`.
    pub async fn call_json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(u16, serde_json::Value), HubError> {
        let mut rb = self.auth(
            http_with(Duration::from_secs(15))?.request(method, format!("{}{path}", self.url)),
        );
        if let Some(b) = body {
            rb = rb.json(b);
        }
        let res = rb
            .send()
            .await
            .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
        let status = res.status().as_u16();
        if status == 401 {
            return Err(HubError::Unauthorized);
        }
        let text = res.text().await.unwrap_or_default();
        let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        Ok((status, json))
    }

    pub async fn logout(&self) -> Result<(), HubError> {
        let res = self
            .auth(http()?.post(format!("{}/api/v1/voice/auth/logout", self.url)))
            .send()
            .await
            .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
        if !res.status().is_success() {
            return Err(error_from(res).await);
        }
        Ok(())
    }
}
