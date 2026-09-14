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
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("local-voice-ai/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| HubError::Other(e.to_string()))
}

fn base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

async fn error_from(res: reqwest::Response) -> HubError {
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    let code = serde_json::from_str::<ApiError>(&text)
        .ok()
        .map(|e| (e.error.code, e.error.message));
    match (status.as_u16(), code) {
        (401, _) => HubError::Unauthorized,
        (410, _) => HubError::CursorExpired,
        (_, Some((code, msg))) if !msg.is_empty() => HubError::Other(format!("{code}: {msg}")),
        (_, Some((code, _))) => HubError::Other(code),
        (s, None) => HubError::Other(format!("HTTP {s}")),
    }
}

pub async fn login(url: &str, email: &str, password: &str, device_name: &str) -> Result<LoginResponse, HubError> {
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
        Self { url: base(url), token: token.to_string() }
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.bearer_auth(&self.token)
    }

    pub async fn push(&self, device_id: &str, objects: &[PushObject]) -> Result<Vec<PushResult>, HubError> {
        let res = self
            .auth(http()?.post(format!("{}/api/v1/voice/sync/push", self.url)))
            .json(&serde_json::json!({ "device_id": device_id, "objects": objects }))
            .send()
            .await
            .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
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

    pub async fn status(&self) -> Result<HubStatus, HubError> {
        let res = self
            .auth(http()?.get(format!("{}/api/v1/voice/sync/status", self.url)))
            .send()
            .await
            .map_err(|e| HubError::Other(format!("Hub nicht erreichbar: {e}")))?;
        if !res.status().is_success() {
            return Err(error_from(res).await);
        }
        res.json().await.map_err(|e| HubError::Other(format!("Antwort unlesbar: {e}")))
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
