//! Multi-Device: Konto, Ende-zu-Ende-verschlüsselter Abgleich von Seiten und
//! Einstellungen über den Hub im wai-portal. Spec:
//! `docs/superpowers/specs/2026-09-14-multi-device-sync-design.md`.

pub mod account;
pub mod client;
pub mod collect;
pub mod crypto;
pub mod engine;
pub mod ledger;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
pub use engine::{start, SyncEngine, SyncStatus};
use tauri::{AppHandle, State};

/// Anmelden: Schlüssel ableiten (nur lokal), Token holen, Konto speichern,
/// Ledger leeren, ersten Lauf anstoßen.
#[tauri::command]
#[specta::specta]
pub async fn sync_login(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
    email: String,
    password: String,
    device_name: String,
    url: Option<String>,
) -> Result<SyncStatus, String> {
    let email = email.trim().to_lowercase();
    if email.is_empty() || password.is_empty() {
        return Err("E-Mail und Passwort eingeben.".into());
    }
    let url = url
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| account::DEFAULT_HUB_URL.to_string());
    check_hub_url(&url)?;
    let device_name = {
        let d = device_name.trim();
        if d.is_empty() { account::default_device_name() } else { d.chars().take(60).collect() }
    };

    // Passwort nur in Puffern, die beim Freigeben ueberschrieben werden.
    let password = zeroize::Zeroizing::new(password);
    // Argon2 (64 MiB) gehört nicht auf den Async-Executor.
    let (pw, em) = (zeroize::Zeroizing::new(password.to_string()), email.clone());
    let key = tauri::async_runtime::spawn_blocking(move || crypto::derive_key(&pw, &em))
        .await
        .map_err(|e| e.to_string())??;

    let login = client::login(&url, &email, &password, &device_name)
        .await
        .map_err(|e| e.to_string())?;
    drop(password);

    let cfg = account::SyncConfig {
        url,
        token: login.token,
        device_id: account::load(&app)
            .map(|c| c.device_id)
            .filter(|d| account::valid_device_id(d))
            .unwrap_or_else(account::new_device_id),
        device_name,
        user_id: login.user.id,
        user_email: login.user.email,
        enc_key_b64: B64.encode(key),
    };
    {
        // Kein laufender Zyklus darf das neue Konto oder das frische Ledger
        // mit seinem alten Stand ueberschreiben.
        let _guard = engine.account_guard().await;
        account::save(&app, &cfg)?;
        ledger::remove(&app);
        engine.refresh_from_config(&app);
    }
    Ok(engine.run_once(&app).await)
}

/// Erlaubt: https zu einem Host ohne Zugangsdaten in der URL, oder http nur
/// zu localhost/127.0.0.1. `http://localhost@evil.example` faellt durch.
fn check_hub_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| "Hub-Adresse ist keine gueltige URL.".to_string())?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Hub-Adresse darf keine Zugangsdaten enthalten.".into());
    }
    let host = parsed.host_str().unwrap_or("");
    match parsed.scheme() {
        "https" if !host.is_empty() => Ok(()),
        "http" if host == "localhost" || host == "127.0.0.1" => Ok(()),
        _ => Err("Der Hub muss über https erreichbar sein.".into()),
    }
}

/// Abmelden: Token im Hub widerrufen (best effort), Konto und Ledger löschen.
/// Die Seiten bleiben auf diesem Gerät.
#[tauri::command]
#[specta::specta]
pub async fn sync_logout(app: AppHandle, engine: State<'_, SyncEngine>) -> Result<SyncStatus, String> {
    if let Some(cfg) = account::load(&app) {
        let client = client::HubClient::new(&cfg.url, &cfg.token);
        if let Err(e) = client.logout().await {
            log::warn!("Sync-Logout am Hub fehlgeschlagen: {e} — lokal trotzdem abgemeldet");
        }
    }
    let _guard = engine.account_guard().await;
    account::remove(&app);
    ledger::remove(&app);
    engine.refresh_from_config(&app);
    Ok(engine.status())
}

#[cfg(test)]
mod tests {
    use super::check_hub_url;

    #[test]
    fn hub_url_nur_https_oder_loopback_ohne_zugangsdaten() {
        assert!(check_hub_url("https://portal.wolffappliedai.de").is_ok());
        assert!(check_hub_url("http://localhost:8000").is_ok());
        assert!(check_hub_url("http://127.0.0.1:8000/").is_ok());
        assert!(check_hub_url("http://localhost@evil.example").is_err());
        assert!(check_hub_url("http://evil.example").is_err());
        assert!(check_hub_url("https://user:pw@portal.wolffappliedai.de").is_err());
        assert!(check_hub_url("portal.wolffappliedai.de").is_err());
    }
}

#[tauri::command]
#[specta::specta]
pub fn sync_status(engine: State<'_, SyncEngine>) -> SyncStatus {
    engine.status()
}

/// Sofort abgleichen (Knopf „Jetzt abgleichen").
#[tauri::command]
#[specta::specta]
pub async fn sync_now(app: AppHandle, engine: State<'_, SyncEngine>) -> Result<SyncStatus, String> {
    Ok(engine.run_once(&app).await)
}

/// Anstoß nach einem lokalen Schreiben (entprellt im Engine-Takt).
#[tauri::command]
#[specta::specta]
pub fn sync_touch(engine: State<'_, SyncEngine>) {
    engine.touch();
}

#[tauri::command]
#[specta::specta]
pub fn sync_default_device_name() -> String {
    account::default_device_name()
}

/// Geräteliste und Zähler vom Hub (für die Konto-Gruppe).
#[tauri::command]
#[specta::specta]
pub async fn sync_hub_status(app: AppHandle) -> Result<client::HubStatus, String> {
    let cfg = account::load(&app).ok_or("Nicht angemeldet")?;
    client::HubClient::new(&cfg.url, &cfg.token)
        .status()
        .await
        .map_err(|e| e.to_string())
}
