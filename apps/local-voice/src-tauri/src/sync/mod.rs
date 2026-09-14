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
    if !url.starts_with("https://") && !url.starts_with("http://localhost") && !url.starts_with("http://127.0.0.1") {
        return Err("Der Hub muss über https erreichbar sein.".into());
    }
    let device_name = {
        let d = device_name.trim();
        if d.is_empty() { account::default_device_name() } else { d.chars().take(60).collect() }
    };

    // Argon2 (64 MiB) gehört nicht auf den Async-Executor.
    let (pw, em) = (password.clone(), email.clone());
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
    account::save(&app, &cfg)?;
    ledger::remove(&app);
    engine.refresh_from_config(&app);
    Ok(engine.run_once(&app).await)
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
    account::remove(&app);
    ledger::remove(&app);
    engine.refresh_from_config(&app);
    Ok(engine.status())
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
