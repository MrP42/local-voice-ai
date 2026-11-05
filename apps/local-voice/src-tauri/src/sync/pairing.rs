//! Geräte-Kopplung per QR (Desktop-Seite, Paket P3 von local-voice-ai#289).
//! Der Desktop zeigt einen QR-Code, das iPhone scannt ihn und erhält nach
//! Vergleich eines sechsstelligen Codes den Sync-Schlüssel und ein eigenes,
//! widerrufbares Token – ohne Passwort.
//!
//! Spec: `docs/superpowers/specs/2026-11-05-geraete-kopplung-qr.md`,
//! Zustandsautomat Abschnitt 5.2, Kryptografie in [`super::pairing_proto`].
//!
//! Geheimnisse (`d_sk`, `q`, `s`, Hub-`secret`, Schlüssel K) liegen nur im
//! Speicher dieser Sitzung und werden beim Freigeben überschrieben. Nichts
//! davon geht ins Log.

use super::account;
use super::client::{HubClient, HubError};
use super::pairing_proto as proto;
use proto::LEN;
use rand::rngs::OsRng;
use rand::RngCore;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{AppHandle, State};
use x25519_dalek::StaticSecret;
use zeroize::{Zeroize, Zeroizing};

/// Nach `expires_at` wartet der Desktop so lange auf die Bestätigung des iPhones.
const CONFIRM_GRACE: Duration = Duration::from_secs(30);
/// Höchstfrist laut Spec (Abschnitt 5a, Regel 3).
const MAX_TTL: u64 = 120;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PairStage {
    /// QR anzeigen; mit `device_name` ist das iPhone erkannt und die Verbindung entsteht.
    Qr,
    /// Vergleichscode zeigen, Ja/Nein abfragen.
    Sas,
    /// Freigegeben, warte auf die Bestätigung des iPhones.
    WaitConfirm,
    Connected,
    /// Freigegeben, aber keine gültige Bestätigung: Gerät abmelden anbieten.
    Unconfirmed,
    Expired,
    Aborted,
    /// Hub vorübergehend nicht erreichbar, bevor das iPhone gescannt hat.
    Network,
}

impl PairStage {
    fn is_final(self) -> bool {
        matches!(
            self,
            PairStage::Connected | PairStage::Unconfirmed | PairStage::Expired | PairStage::Aborted
        )
    }
}

/// Ein Gerät des Kontos (`GET /api/v1/voice/devices`).
#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
pub struct PairedDevice {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub current: bool,
    #[serde(default)]
    pub via_pairing: bool,
    #[serde(default)]
    pub paired_by: Option<i64>,
    /// Geräte, die beim Abmelden dieses Geräts mit abgemeldet werden.
    #[serde(default)]
    pub revokes_with: Vec<i64>,
}

#[derive(Serialize, Clone, Debug, specta::Type)]
pub struct PairView {
    pub stage: PairStage,
    /// SVG des QR-Codes, schwarz auf weiß (nur Stufe `qr`).
    pub qr_svg: Option<String>,
    /// Restzeit bis zum Ablauf des Codes in Sekunden.
    pub remaining_s: u32,
    /// Vom iPhone angegebener Name (nicht geprüft, nur angezeigt).
    pub device_name: Option<String>,
    /// Sechsstelliger Vergleichscode.
    pub sas: Option<String>,
    /// Begründung für Abbruch, Netzfehler oder fehlende Bestätigung.
    pub reason: Option<String>,
    /// Bei `unconfirmed`: das neu verbundene Gerät, das abgemeldet werden sollte.
    pub suspect: Option<PairedDevice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    WaitScan,
    WaitReveal,
    ShowSas,
    WaitConfirm,
}

struct Session {
    url: String,
    token: Zeroizing<String>,
    user_id: i64,
    email: String,
    key: Zeroizing<[u8; LEN]>,
    id: String,
    d_sk: StaticSecret,
    d_pk: [u8; LEN],
    q: Zeroizing<[u8; LEN]>,
    qr_svg: String,
    /// Nur für den Ende-zu-Ende-Test: Inhalt des QR-Codes (das „gescannte“ Bild).
    #[cfg(test)]
    uri: String,
    deadline: Instant,
    phase: Phase,
    /// Einmalig beim `claimed` festgehalten (Commit/Reveal).
    commit: Option<[u8; LEN]>,
    name: Option<String>,
    /// Zufall des Desktops, erst nach Sicht auf C erzeugt.
    n_d: Option<[u8; LEN]>,
    p_pk: Option<[u8; LEN]>,
    sas: Option<String>,
    s: Option<Zeroizing<[u8; LEN]>>,
    /// `approve` ist abgeschickt (auch bei unklarem Ausgang): ab hier nie mehr
    /// still „abgebrochen“ melden, das iPhone hat womöglich schon ein Token.
    approve_sent: bool,
}

/// Laufende Kopplung; höchstens eine je App.
#[derive(Default)]
pub struct PairingState(tokio::sync::Mutex<Option<Session>>);

impl PairingState {
    /// Nur für den Ende-zu-Ende-Test: den QR-Inhalt „scannen“.
    #[cfg(test)]
    pub async fn scanned_uri(&self) -> Option<String> {
        self.0.lock().await.as_ref().map(|s| s.uri.clone())
    }

    async fn lock(&self) -> tokio::sync::MutexGuard<'_, Option<Session>> {
        self.0.lock().await
    }
}

fn random32() -> Zeroizing<[u8; LEN]> {
    let mut b = Zeroizing::new([0u8; LEN]);
    OsRng.fill_bytes(b.as_mut());
    b
}

fn hub_message(j: &Value, fallback: &str) -> String {
    j["error"]["message"]
        .as_str()
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn qr_svg(uri: &str) -> Result<String, String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::with_error_correction_level(uri.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| format!("QR-Code: {e}"))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .quiet_zone(true)
        .build())
}

/// `localvoice://pair?v=2&h=<hub>&p=<id>&s=<secret>&q=<q>&k=<d_pk>`; `h` ist die
/// URL-kodierte Basisadresse des Hubs.
pub fn pair_uri(hub: &str, id: &str, secret: &str, q: &[u8; LEN], d_pk: &[u8; LEN]) -> String {
    let h: String = url::form_urlencoded::byte_serialize(hub.trim_end_matches('/').as_bytes()).collect();
    format!(
        "localvoice://pair?v=2&h={h}&p={id}&s={secret}&q={}&k={}",
        proto::b64u(q),
        proto::b64u(d_pk)
    )
}

impl Session {
    fn client(&self) -> HubClient {
        HubClient::new(&self.url, &self.token)
    }

    fn path(&self, tail: &str) -> String {
        format!("/api/v1/voice/pair/{}{tail}", self.id)
    }

    fn remaining(&self) -> u32 {
        // Aufrunden: die Anzeige erreicht 0 erst, wenn die Frist im Kern abgelaufen ist.
        let ms = self.deadline.saturating_duration_since(Instant::now()).as_millis();
        ms.div_ceil(1000) as u32
    }

    fn view(&self, stage: PairStage, reason: Option<String>) -> PairView {
        PairView {
            stage,
            qr_svg: (stage == PairStage::Qr || stage == PairStage::Network)
                .then(|| self.qr_svg.clone()),
            remaining_s: self.remaining(),
            device_name: self.name.clone(),
            sas: (stage == PairStage::Sas).then(|| self.sas.clone()).flatten(),
            reason,
            suspect: None,
        }
    }

    /// Kopplung am Hub verwerfen, Fehler egal. Nach einem Abbruch ab
    /// `challenge` entsteht kein neues `n_d` (Spec 5.2, letzte Zeile).
    async fn reject_quiet(&self) {
        let _ = self.client().call_json(Method::POST, &self.path("/reject"), Some(&json!({}))).await;
    }

    async fn abort(&self, reason: &str) -> PairView {
        if self.approve_sent {
            return self.settle(reason).await;
        }
        self.reject_quiet().await;
        self.view(PairStage::Aborted, Some(reason.to_string()))
    }

    /// Nach `approve`: Freigabe zurückziehen und am Hub klären, ob das iPhone
    /// schon abgeholt hat. Nur wenn der Hub `done` meldet, wird ein Gerät zum
    /// Abmelden vorgeschlagen; bei unklarem Stand nur auf die Geräteliste verwiesen.
    async fn settle(&self, reason: &str) -> PairView {
        let res = self
            .client()
            .call_json(Method::POST, &self.path("/reject"), Some(&json!({})))
            .await;
        match res {
            Ok((200, _)) => self.view(
                PairStage::Aborted,
                Some(format!("{reason} Die Freigabe wurde zurückgezogen, das iPhone hat nichts erhalten.")),
            ),
            Ok((409, j)) if j["status"] == "done" => {
                // Abgeholt: hat das iPhone inzwischen gültig bestätigt?
                if let Ok((200, j)) = self.client().call_json(Method::GET, &self.path(""), None).await {
                    if self.confirm_ok(&j) {
                        return self.view(PairStage::Connected, None);
                    }
                }
                self.unconfirmed("Das iPhone hat den Zugang erhalten, aber nicht gültig bestätigt.")
                    .await
            }
            Ok((409, _)) | Ok((404, _)) | Ok((410, _)) => self.view(
                PairStage::Aborted,
                Some("Das iPhone hat die Freigabe nicht abgeholt.".into()),
            ),
            _ => self.view(
                PairStage::Unconfirmed,
                Some("Der Stand am Hub ist unklar. Bitte die Liste „Verbundene Geräte“ prüfen und ein unbekanntes Gerät abmelden.".into()),
            ),
        }
    }

    fn confirm_ok(&self, j: &Value) -> bool {
        match (j["phone_confirm"].as_str().map(proto::b64u32), self.s.as_ref()) {
            (Some(Ok(c)), Some(s)) => proto::verify_confirm(s, &self.id, &c),
            _ => false,
        }
    }

    /// Nur aufrufen, wenn der Hub `done` gemeldet hat.
    async fn unconfirmed(&self, reason: &str) -> PairView {
        let mut v = self.view(PairStage::Unconfirmed, Some(reason.to_string()));
        v.suspect = newest_child(&self.client()).await;
        v
    }

    async fn step(&mut self) -> PairView {
        let now = Instant::now();
        match self.phase {
            Phase::WaitConfirm if now >= self.deadline + CONFIRM_GRACE => {
                return self.settle("Das iPhone hat die Verbindung nicht bestätigt.").await;
            }
            Phase::WaitConfirm => {}
            _ if now >= self.deadline => {
                self.reject_quiet().await;
                return self.view(PairStage::Expired, None);
            }
            Phase::ShowSas => return self.view(PairStage::Sas, None),
            _ => {}
        }

        let (status, j) = match self.client().call_json(Method::GET, &self.path(""), None).await {
            Ok(r) => r,
            Err(HubError::Unauthorized) => {
                return self.view(
                    PairStage::Aborted,
                    Some("Die Anmeldung am Hub ist abgelaufen. Bitte neu anmelden.".into()),
                )
            }
            Err(e) => {
                return match self.phase {
                    Phase::WaitScan => self.view(PairStage::Network, Some(e.to_string())),
                    Phase::WaitConfirm => self.view(PairStage::WaitConfirm, None),
                    _ => self.abort("Verbindung zum Hub unterbrochen. Kopplung abgebrochen.").await,
                };
            }
        };
        if status == 404 {
            return self.view(
                PairStage::Aborted,
                Some("Die Kopplung ist nicht mehr vorhanden.".into()),
            );
        }
        if status != 200 {
            return self.abort(&hub_message(&j, "Der Hub hat die Kopplung abgelehnt.")).await;
        }
        let hub_status = j["status"].as_str().unwrap_or("");

        match (self.phase, hub_status) {
            (Phase::WaitScan, "pending") => self.view(PairStage::Qr, None),
            (Phase::WaitScan, "claimed") => self.on_claimed(&j).await,
            (Phase::WaitReveal, "claimed" | "challenged") => {
                // Das festgehaltene C darf sich nie ändern.
                if !self.commit_unchanged(&j) {
                    return self.abort("Sicherheitsprüfung fehlgeschlagen. Kopplung abgebrochen.").await;
                }
                self.view(PairStage::Qr, None)
            }
            (Phase::WaitReveal, "revealed") => self.on_revealed(&j).await,
            (Phase::WaitConfirm, "approved") => self.view(PairStage::WaitConfirm, None),
            (Phase::WaitConfirm, "done") => match j["phone_confirm"].as_str() {
                None | Some("") => self.view(PairStage::WaitConfirm, None),
                Some(_) => {
                    if self.confirm_ok(&j) {
                        self.view(PairStage::Connected, None)
                    } else {
                        self.unconfirmed("Die Bestätigung des iPhones ist ungültig.").await
                    }
                }
            },
            (Phase::WaitConfirm, "expired") => self.view(
                PairStage::Aborted,
                Some("Das iPhone hat die Freigabe nicht rechtzeitig abgeholt.".into()),
            ),
            (_, "expired") => self.view(PairStage::Expired, None),
            (_, "rejected") => self.view(PairStage::Aborted, Some("Die Kopplung wurde abgebrochen.".into())),
            _ => self.abort("Unerwarteter Zustand. Kopplung abgebrochen.").await,
        }
    }

    fn commit_unchanged(&self, j: &Value) -> bool {
        match (self.commit, j["phone_commit"].as_str().map(proto::b64u32)) {
            (Some(stored), Some(Ok(c))) => proto::ct_eq(&stored, &c),
            _ => false,
        }
    }

    async fn on_claimed(&mut self, j: &Value) -> PairView {
        let parsed = (|| -> Result<([u8; LEN], [u8; LEN], String), ()> {
            let c = proto::b64u32(j["phone_commit"].as_str().ok_or(())?).map_err(|_| ())?;
            let mac = proto::b64u32(j["phone_mac"].as_str().ok_or(())?).map_err(|_| ())?;
            let name = j["device_name"].as_str().ok_or(())?.to_string();
            Ok((c, mac, name))
        })();
        let Ok((c, mac, name)) = parsed else {
            return self.abort("Antwort des Hubs unvollständig. Kopplung abgebrochen.").await;
        };
        if !proto::verify_claim_mac(&self.q, &self.id, &c, &mac) {
            return self
                .abort("Ein anderes Gerät hat den Code verwendet. Kopplung abgebrochen.")
                .await;
        }
        if !proto::valid_name(&name) {
            return self.abort("Der Gerätename ist ungültig. Kopplung abgebrochen.").await;
        }
        // C und Name einmalig festhalten; nur dieser Name wird angezeigt und geprüft.
        self.commit = Some(c);
        self.name = Some(name);

        let n_d = random32();
        let body = json!({ "desktop_nonce": proto::b64u(n_d.as_ref()) });
        match self.client().call_json(Method::POST, &self.path("/challenge"), Some(&body)).await {
            Ok((200, _)) => {
                self.phase = Phase::WaitReveal;
                // Der Desktop rechnet mit seinem eigenen n_d, nie mit einem Wert vom Hub.
                self.n_d = Some(*n_d);
                self.view(PairStage::Qr, None)
            }
            Ok((_, j)) => self.abort(&hub_message(&j, "Der Hub hat die Kopplung abgebrochen.")).await,
            Err(_) => self.abort("Verbindung zum Hub unterbrochen. Kopplung abgebrochen.").await,
        }
    }

    async fn on_revealed(&mut self, j: &Value) -> PairView {
        match self.verify_reveal(j) {
            Ok(()) => {
                self.phase = Phase::ShowSas;
                self.view(PairStage::Sas, None)
            }
            Err(()) => {
                self.abort("Sicherheitsprüfung fehlgeschlagen. Kopplung abgebrochen.")
                    .await
            }
        }
    }

    /// Spec 4.3 Schritt 4: C gegen das festgehaltene C, `p_pk ≠ d_pk`, mac erneut,
    /// Nullprüfung, dann SAS und `s` genau einmal.
    fn verify_reveal(&mut self, j: &Value) -> Result<(), ()> {
        if !self.commit_unchanged(j) {
            return Err(());
        }
        let c = self.commit.ok_or(())?;
        let name = self.name.clone().ok_or(())?;
        let n_d = self.n_d.ok_or(())?;
        let p_pk = proto::b64u32(j["phone_pub"].as_str().ok_or(())?).map_err(|_| ())?;
        let n_p = proto::b64u32(j["phone_nonce"].as_str().ok_or(())?).map_err(|_| ())?;
        let mac = proto::b64u32(j["phone_mac"].as_str().ok_or(())?).map_err(|_| ())?;
        if !proto::ct_eq(&proto::commitment(&self.id, &p_pk, &n_p, &name), &c) {
            return Err(());
        }
        if proto::ct_eq(&p_pk, &self.d_pk) {
            return Err(());
        }
        if !proto::verify_claim_mac(&self.q, &self.id, &c, &mac) {
            return Err(());
        }
        let z = proto::shared(&self.d_sk, &p_pk).map_err(|_| ())?;
        self.sas = Some(proto::sas(&self.id, &self.d_pk, &p_pk, &n_p, &n_d));
        self.s = Some(proto::session_key(&z, &self.q, &self.d_pk, &p_pk, &self.id));
        self.p_pk = Some(p_pk);
        Ok(())
    }
}

/// Das jüngste Gerät, das über dieses Gerät gekoppelt wurde.
async fn newest_child(client: &HubClient) -> Option<PairedDevice> {
    let devices = fetch_devices(client).await.ok()?;
    let me = devices.iter().find(|d| d.current)?.id;
    devices
        .into_iter()
        .filter(|d| d.paired_by == Some(me))
        .max_by_key(|d| d.id)
}

async fn fetch_devices(client: &HubClient) -> Result<Vec<PairedDevice>, String> {
    #[derive(Deserialize)]
    struct Body {
        devices: Vec<PairedDevice>,
    }
    let (status, j) = client
        .call_json(Method::GET, "/api/v1/voice/devices", None)
        .await
        .map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(hub_message(&j, "Geräteliste nicht verfügbar."));
    }
    serde_json::from_value::<Body>(j)
        .map(|b| b.devices)
        .map_err(|_| "Geräteliste unlesbar.".to_string())
}

fn ended(stage: PairStage, reason: &str) -> PairView {
    PairView {
        stage,
        qr_svg: None,
        remaining_s: 0,
        device_name: None,
        sas: None,
        reason: Some(reason.to_string()),
        suspect: None,
    }
}

/// „Handy verbinden“: Kopplung am Hub starten, Schlüsselpaar und q erzeugen, QR liefern.
/// Eine laufende Kopplung wird ersetzt (der Hub verwirft sie beim neuen Start).
#[tauri::command]
#[specta::specta]
pub async fn pair_start(app: AppHandle, state: State<'_, PairingState>) -> Result<PairView, String> {
    let cfg = account::load(&app).ok_or("Zuerst hier anmelden.")?;
    let key = Zeroizing::new(cfg.key()?);
    let mut guard = state.lock().await;
    *guard = None;

    let client = HubClient::new(&cfg.url, &cfg.token);
    let (status, j) = client
        .call_json(Method::POST, "/api/v1/voice/pair/start", Some(&json!({})))
        .await
        .map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(hub_message(&j, "Die Kopplung konnte nicht gestartet werden."));
    }
    let id = j["pair_id"].as_str().unwrap_or_default().to_string();
    let secret = Zeroizing::new(j["secret"].as_str().unwrap_or_default().to_string());
    if !proto::valid_pair_id(&id) || proto::b64u32(&secret).is_err() {
        return Err("Antwort des Hubs unlesbar.".into());
    }
    let ttl = j["ttl"].as_u64().unwrap_or(MAX_TTL).clamp(10, MAX_TTL);

    let mut sk_bytes = random32();
    let d_sk = StaticSecret::from(*sk_bytes);
    sk_bytes.zeroize();
    let d_pk = proto::public_key(&d_sk);
    let q = random32();
    let uri = Zeroizing::new(pair_uri(&cfg.url, &id, &secret, &q, &d_pk));
    let svg = qr_svg(&uri)?;

    let session = Session {
        url: cfg.url.clone(),
        token: Zeroizing::new(cfg.token.clone()),
        user_id: cfg.user_id,
        email: cfg.user_email.clone(),
        key,
        id,
        d_sk,
        d_pk,
        q,
        qr_svg: svg,
        #[cfg(test)]
        uri: uri.to_string(),
        // Etwas Puffer gegen Laufzeit der Anfrage: lieber früher „abgelaufen“.
        deadline: Instant::now() + Duration::from_secs(ttl.saturating_sub(2)),
        phase: Phase::WaitScan,
        commit: None,
        name: None,
        n_d: None,
        p_pk: None,
        sas: None,
        s: None,
        approve_sent: false,
    };
    let view = session.view(PairStage::Qr, None);
    *guard = Some(session);
    Ok(view)
}

/// Zustand abfragen und den Automaten einen Schritt weiterführen (Takt ≈ 1,5 s).
#[tauri::command]
#[specta::specta]
pub async fn pair_poll(state: State<'_, PairingState>) -> Result<PairView, String> {
    let mut guard = state.lock().await;
    let Some(session) = guard.as_mut() else {
        return Ok(ended(PairStage::Aborted, "Keine laufende Kopplung."));
    };
    let view = session.step().await;
    if view.stage.is_final() {
        *guard = None;
    }
    Ok(view)
}

/// „Ja, verbinden“: Schlüssel K für das iPhone versiegeln und freigeben.
#[tauri::command]
#[specta::specta]
pub async fn pair_approve(state: State<'_, PairingState>) -> Result<PairView, String> {
    let mut guard = state.lock().await;
    let session = guard.as_mut().ok_or("Keine laufende Kopplung.")?;
    if session.phase != Phase::ShowSas {
        return Err("Es wartet keine Bestätigung.".into());
    }
    if Instant::now() >= session.deadline {
        session.reject_quiet().await;
        let v = session.view(PairStage::Expired, None);
        *guard = None;
        return Ok(v);
    }
    let sealed = {
        let s = session.s.as_ref().ok_or("Schlüssel fehlt.")?;
        let p_pk = session.p_pk.ok_or("Schlüssel fehlt.")?;
        let payload = proto::payload_json(&session.key, &session.email, session.user_id);
        let mut nonce = [0u8; proto::NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        proto::seal(s, &session.id, &session.d_pk, &p_pk, payload.as_bytes(), &nonce)?
    };
    session.approve_sent = true;
    let res = session
        .client()
        .call_json(Method::POST, &session.path("/approve"), Some(&json!({ "sealed": sealed })))
        .await;
    let view = match res {
        Ok((200, _)) => {
            session.phase = Phase::WaitConfirm;
            session.view(PairStage::WaitConfirm, None)
        }
        Ok((_, j)) => session.abort(&hub_message(&j, "Die Freigabe wurde abgelehnt.")).await,
        Err(e) => session.abort(&e.to_string()).await,
    };
    if view.stage.is_final() {
        *guard = None;
    }
    Ok(view)
}

/// „Nein, abbrechen“ oder Dialog schließen. Nach der Freigabe wird am Hub geklärt,
/// ob das iPhone schon abgeholt hat (dann Gerät zum Abmelden anbieten).
#[tauri::command]
#[specta::specta]
pub async fn pair_cancel(state: State<'_, PairingState>) -> Result<PairView, String> {
    let mut guard = state.lock().await;
    let Some(session) = guard.take() else {
        return Ok(ended(PairStage::Aborted, "Abgebrochen."));
    };
    if session.approve_sent {
        return Ok(session.settle("Abgebrochen.").await);
    }
    session.reject_quiet().await;
    Ok(session.view(PairStage::Aborted, Some("Abgebrochen.".into())))
}

/// Liste „Verbundene Geräte“.
#[tauri::command]
#[specta::specta]
pub async fn sync_devices(app: AppHandle) -> Result<Vec<PairedDevice>, String> {
    let cfg = account::load(&app).ok_or("Nicht angemeldet")?;
    fetch_devices(&HubClient::new(&cfg.url, &cfg.token)).await
}

/// Gerät abmelden; der Hub meldet alle über dieses Gerät gekoppelten Geräte mit ab.
#[tauri::command]
#[specta::specta]
pub async fn sync_revoke_device(app: AppHandle, id: i64) -> Result<(), String> {
    let cfg = account::load(&app).ok_or("Nicht angemeldet")?;
    let (status, j) = HubClient::new(&cfg.url, &cfg.token)
        .call_json(Method::POST, &format!("/api/v1/voice/devices/{id}/revoke"), Some(&json!({})))
        .await
        .map_err(|e| e.to_string())?;
    if status == 200 {
        Ok(())
    } else {
        Err(hub_message(&j, "Das Gerät konnte nicht abgemeldet werden."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_kodiert_hub_und_traegt_alle_werte() {
        let q = [1u8; LEN];
        let k = [2u8; LEN];
        let uri = pair_uri(
            "https://portal.wolffappliedai.de/",
            "AAECAwQFBgcICQoLDA0ODw",
            "oaKjpKWmp6ipqqusra6vsLGys7S1tre4ubq7vL2-v8A",
            &q,
            &k,
        );
        assert!(uri.starts_with(
            "localvoice://pair?v=2&h=https%3A%2F%2Fportal.wolffappliedai.de&p=AAECAwQFBgcICQoLDA0ODw&s="
        ));
        assert!(uri.contains(&format!("&q={}", proto::b64u(&q))));
        assert!(uri.ends_with(&format!("&k={}", proto::b64u(&k))));
    }

    #[test]
    fn qr_ist_svg_schwarz_auf_weiss() {
        let svg = qr_svg("localvoice://pair?v=2").unwrap();
        assert!(svg.contains("<svg") && svg.contains("#000000") && svg.contains("#ffffff"));
    }
}
