//! Konto-Ablage des Geräte-Syncs: `<appdata>/sync.json` mit Hub-URL, Token,
//! Geräte-Kennung und dem lokal abgeleiteten Schlüssel. Fehlt die Datei, läuft
//! die App rein lokal. Kein Passwort auf Platte.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::AppHandle;

pub const DEFAULT_HUB_URL: &str = "https://portal.wolffappliedai.de";
const FILE_NAME: &str = "sync.json";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncConfig {
    pub url: String,
    pub token: String,
    pub device_id: String,
    pub device_name: String,
    pub user_id: i64,
    pub user_email: String,
    /// Der Objekt-Schlüssel, base64. Liegt hier, weil die App keinen
    /// Schlüsselbund anbindet (Spec Abschnitt 8, Punkt 6).
    pub enc_key_b64: String,
}

impl SyncConfig {
    pub fn key(&self) -> Result<[u8; super::crypto::KEY_LEN], String> {
        let raw = B64
            .decode(&self.enc_key_b64)
            .map_err(|_| "sync.json: Schlüssel unlesbar".to_string())?;
        raw.try_into()
            .map_err(|_| "sync.json: Schlüssel hat die falsche Länge".to_string())
    }
}

pub fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    crate::portable::app_data_dir(app)
        .map(|d| d.join(FILE_NAME))
        .map_err(|e| format!("Kein Datenverzeichnis: {e}"))
}

pub fn load(app: &AppHandle) -> Option<SyncConfig> {
    let path = config_path(app).ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<SyncConfig>(&raw) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            log::warn!("sync.json unlesbar ({e}) — Sync bleibt aus");
            None
        }
    }
}

pub fn save(app: &AppHandle, cfg: &SyncConfig) -> Result<(), String> {
    let path = config_path(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Datenverzeichnis: {e}"))?;
    }
    let raw = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, raw).map_err(|e| format!("sync.json schreiben: {e}"))?;
    restrict_permissions(&path);
    Ok(())
}

pub fn remove(app: &AppHandle) {
    if let Ok(path) = config_path(app) {
        let _ = std::fs::remove_file(path);
    }
}

/// Unix: nur der Besitzer liest die Datei. Windows: das Datenverzeichnis liegt
/// ohnehin im Benutzerprofil (%LOCALAPPDATA%), eine eigene ACL setzt die App
/// nicht (Spec Abschnitt 3).
#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &std::path::Path) {}

pub fn new_device_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Vorschlag für den Gerätenamen: der Hostname, sonst „Dieses Gerät".
pub fn default_device_name() -> String {
    sysinfo::System::host_name()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Dieses Gerät".to_string())
}

/// Der Hub akzeptiert `[A-Za-z0-9_-]{8,64}`; eine Geräte-Kennung aus
/// `new_device_id` erfüllt das, eine fremde nicht unbedingt.
pub fn valid_device_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geraete_kennung_passt_zum_hub_muster() {
        let id = new_device_id();
        assert_eq!(id.len(), 32);
        assert!(valid_device_id(&id));
        assert!(!valid_device_id("kurz"));
        assert!(!valid_device_id("hat leerzeichen!"));
    }

    #[test]
    fn schluessel_roundtrip_ueber_base64() {
        let key = [9u8; super::super::crypto::KEY_LEN];
        let cfg = SyncConfig {
            url: DEFAULT_HUB_URL.into(),
            token: "t".into(),
            device_id: new_device_id(),
            device_name: "PC".into(),
            user_id: 1,
            user_email: "a@b.c".into(),
            enc_key_b64: B64.encode(key),
        };
        assert_eq!(cfg.key().unwrap(), key);
        let bad = SyncConfig { enc_key_b64: "AAAA".into(), ..cfg };
        assert!(bad.key().is_err());
    }
}
