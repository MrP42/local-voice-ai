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
    write_private(&path, raw.as_bytes())?;
    Ok(())
}

/// Atomar und privat schreiben: erst in eine Nachbardatei (`<name>.tmp`, von
/// Anfang an nur fuer den Benutzer), dann umbenennen. Ein Absturz mitten im
/// Schreiben laesst die alte Datei heil; eine halbe Datei gibt es nie.
pub(super) fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let mut tmp_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    if let Err(e) = write_private_tmp(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Datei ersetzen: {e}")
    })
}

/// Unix: Datei von Anfang an mit 0600 anlegen.
#[cfg(unix)]
fn write_private_tmp(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("Datei anlegen: {e}"))?;
    f.write_all(bytes)
        .map_err(|e| format!("Datei schreiben: {e}"))?;
    f.sync_all().map_err(|e| format!("Datei sichern: {e}"))?;
    // Bestehende Datei: Rechte nachziehen (mode gilt nur beim Anlegen).
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Datei Rechte: {e}"))
}

/// Windows: nach dem Schreiben die Vererbung kappen und nur den aktuellen
/// Benutzer eintragen (icacls, ohne Konsolenfenster). Recht `(M)` statt
/// `(R,W)`: das atomare Ersetzen per rename braucht DELETE auf der Zieldatei,
/// sonst scheitert jedes zweite Schreiben mit „Zugriff verweigert“. Schlaegt icacls fehl,
/// bleibt die Datei bestehen (sie liegt im Profil des Benutzers); das ist nur
/// eine Warnung -- ein Fehlschlag darf nie Daten loeschen.
#[cfg(windows)]
fn write_private_tmp(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    std::fs::write(path, bytes).map_err(|e| format!("Datei schreiben: {e}"))?;
    let user = std::env::var("USERNAME").unwrap_or_default();
    if user.is_empty() {
        log::warn!("Dateirechte nicht gesetzt: Benutzername unbekannt");
        return Ok(());
    }
    let status = std::process::Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r", &format!("{user}:(M)")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(_) => log::warn!("Dateirechte nicht gesetzt (icacls meldet Fehler)"),
        Err(e) => log::warn!("Dateirechte nicht gesetzt (icacls: {e})"),
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn write_private_tmp(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("Datei schreiben: {e}"))
}

pub fn remove(app: &AppHandle) {
    if let Ok(path) = config_path(app) {
        let _ = std::fs::remove_file(path);
    }
}

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
    (8..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
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
    fn schreiben_ist_atomar_und_hinterlaesst_keine_tmp_datei() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        write_private(&path, b"eins").unwrap();
        write_private(&path, b"zwei").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"zwei");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["x.json".to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
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
        let bad = SyncConfig {
            enc_key_b64: "AAAA".into(),
            ..cfg
        };
        assert!(bad.key().is_err());
    }
}
