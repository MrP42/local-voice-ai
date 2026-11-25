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

/// Atomar schreiben: erst in eine Nachbardatei (`<name>.tmp`, mit privaten
/// Rechten, soweit der Datentraeger sie unterstuetzt), dann umbenennen. Ein
/// Absturz mitten im Schreiben laesst die alte Datei heil; eine halbe Datei gibt es nie.
pub(super) fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let mut tmp_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    tmp_name.push(".tmp");
    #[cfg(windows)]
    tmp_name.push(format!("-{}", new_device_id()));
    let tmp = path.with_file_name(tmp_name);
    if let Err(e) = write_private_tmp(&tmp, bytes) {
        #[cfg(not(windows))]
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

/// Windows: bei ACL-Unterstuetzung geschuetzte DACL schon beim Anlegen,
/// ausschliesslich die Token-SID; vor dem ersten Schreiben am Handle pruefen.
/// CREATE_NEW uebernimmt keine bestehende Datei mit unbekannten Rechten.
#[cfg(windows)]
fn write_private_tmp(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    write_private_tmp_checked(path, bytes, check_private_file)
}

#[cfg(windows)]
fn write_private_tmp_checked(
    path: &std::path::Path,
    bytes: &[u8],
    check: impl FnOnce(&std::fs::File) -> Result<(), String>,
) -> Result<(), String> {
    use std::io::Write;
    use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, GENERIC_WRITE, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_READ_ATTRIBUTES, FILE_SHARE_MODE, READ_CONTROL,
    };
    let sid = user_sid()?;
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;;FA;;;{sid})\0").encode_utf16().collect();
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if wide[..wide.len() - 1].contains(&0) { return Err("Dateiname ungueltig".into()); }
    // SAFETY: gueltige nullterminierte Zeichenketten. Der Deskriptor wird nach
    // CreateFileW freigegeben; File uebernimmt danach das Datei-Handle.
    let mut file = unsafe {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), 1, &mut sd, None)
            .map_err(|e| format!("Datei DACL: {e}"))?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let handle = CreateFileW(PCWSTR(wide.as_ptr()), GENERIC_WRITE.0 | READ_CONTROL.0 | FILE_READ_ATTRIBUTES.0, FILE_SHARE_MODE(0),
            Some(&sa), CREATE_NEW, FILE_ATTRIBUTE_NORMAL, None);
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        let handle = handle.map_err(|e| format!("Datei anlegen: {e}"))?;
        std::fs::File::from_raw_handle(handle.0)
    };
    let result = check(&file)
        .and_then(|()| file.write_all(bytes).map_err(|e| format!("Datei schreiben: {e}")))
        .and_then(|()| file.sync_all().map_err(|e| format!("Datei sichern: {e}")));
    drop(file);
    if result.is_err() { let _ = std::fs::remove_file(path); }
    result
}

#[cfg(windows)]
fn check_private_file(file: &std::fs::File) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;
    let handle = HANDLE(file.as_raw_handle());
    let mut flags = 0;
    // SAFETY: gehaltenes Datei-Handle, gueltiger Ausgabepuffer.
    unsafe { GetVolumeInformationByHandleW(handle, None, None, None, Some(&mut flags), None) }
        .map_err(|e| format!("Datei Datentraegerrechte nicht pruefbar: {e}"))?;
    check_private_volume(flags, || actual_private_dacl(handle))
}

#[cfg(windows)]
fn check_private_volume(flags: u32, dacl: impl FnOnce() -> Result<bool, String>) -> Result<(), String> {
    // FILE_PERSISTENT_ACLS (Win32); kein zusaetzliches SystemServices-Feature.
    if flags & 0x0000_0008 == 0 {
        // Wie bisher bei fehlgeschlagenem icacls: Schreiben bleibt erlaubt,
        // aber ohne eine Zusicherung privater Rechte (z. B. FAT/exFAT).
        log::warn!("Dateirechte nicht geschuetzt: Datentraeger unterstuetzt keine persistenten ACLs");
        return Ok(());
    }
    if dacl()? { Ok(()) } else { Err("Datei DACL nicht privat; Schreiben abgebrochen".into()) }
}

#[cfg(windows)]
fn actual_private_dacl(handle: windows::Win32::Foundation::HANDLE) -> Result<bool, String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, GetSecurityInfo, SE_FILE_OBJECT};
    use windows::Win32::Security::{
        GetAce, GetSecurityDescriptorControl, ACCESS_ALLOWED_ACE, ACL,
        DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    };
    let sid = user_sid()?;
    // SAFETY: SecurityInfo liefert Deskriptor und darin liegende ACL/ACE.
    // Beide allokierten Puffer werden auch auf Fehlerwegen freigegeben.
    unsafe {
        let mut acl: *mut ACL = std::ptr::null_mut();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        GetSecurityInfo(handle, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION,
            None, None, Some(&mut acl), None, Some(&mut sd)).ok()
            .map_err(|e| format!("Datei DACL nicht lesbar: {e}"))?;
        let result = (|| {
            let mut control = 0;
            let mut revision = 0;
            GetSecurityDescriptorControl(sd, &mut control, &mut revision)
                .map_err(|e| format!("Datei DACL-Steuerung: {e}"))?;
            if acl.is_null() || (*acl).AceCount != 1 || control & SE_DACL_PROTECTED.0 == 0 {
                return Ok(false);
            }
            let mut ace = std::ptr::null_mut();
            GetAce(acl, 0, &mut ace).map_err(|e| format!("Datei ACE: {e}"))?;
            let header = &*ace.cast::<windows::Win32::Security::ACE_HEADER>();
            if header.AceType != 0 || header.AceFlags != 0 { return Ok(false); }
            let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            let mut text = PWSTR::null();
            ConvertSidToStringSidW(PSID((&ace.SidStart as *const u32).cast_mut().cast()), &mut text)
                .map_err(|e| format!("Datei ACE-SID: {e}"))?;
            let actual = text.to_string();
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            let actual = actual.map_err(|e| format!("Datei ACE-SID: {e}"))?;
            Ok(actual == sid && ace.Mask == 0x001f01ff)
        })();
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        result
    }
}

/// Die SID kommt aus dem Token, nicht aus veraenderlichen Umgebungsvariablen.
#[cfg(windows)]
fn user_sid() -> Result<String, String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: Token-Puffer ausgerichtet und in der vom System erfragten Groesse;
    // alle Handles und allokierten Zeichenketten werden auch bei Fehlern frei.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| format!("Prozess-Token: {e}"))?;
        let token = crate::process_guard::OwnedHandle(token);
        let mut len = 0;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        let result = GetTokenInformation(token.0, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len);
        result.map_err(|e| format!("Benutzer-SID: {e}"))?;
        let user = &*buf.as_ptr().cast::<TOKEN_USER>();
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).map_err(|e| format!("SID: {e}"))?;
        let sid = text.to_string();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid.map_err(|e| format!("SID: {e}"))
    }
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

    #[cfg(windows)]
    #[test]
    fn private_tmp_checks_before_writing_and_refuses_a_bad_dacl() {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("geerbt.tmp");
        std::fs::write(&public, b"").unwrap();
        let public_file = std::fs::File::open(&public).unwrap();
        assert!(!actual_private_dacl(HANDLE(public_file.as_raw_handle())).unwrap());
        let path = dir.path().join("privat.tmp");
        let error = write_private_tmp_checked(&path, b"synthetisches-geheimnis", |file| {
            assert_eq!(file.metadata().unwrap().len(), 0, "Pruefung vor dem ersten Byte");
            assert!(actual_private_dacl(HANDLE(file.as_raw_handle()))?);
            check_private_file(&public_file)
        }).unwrap_err();
        assert!(error.contains("DACL nicht privat"), "{error}");
        assert!(!path.exists());
        // Gegenprobe: dieselbe Anlage mit wirksam privater DACL schreibt erfolgreich.
        write_private_tmp(&path, b"synthetisch").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"synthetisch");
    }

    #[cfg(windows)]
    #[test]
    fn a_volume_without_acls_keeps_the_old_write_behaviour() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ohne-acls.tmp");
        write_private_tmp_checked(&path, b"synthetisch", |file| {
            assert_eq!(file.metadata().unwrap().len(), 0);
            check_private_volume(0, || panic!("keine DACL-Zusicherung auf FAT/exFAT"))
        }).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"synthetisch");
        // Gegenprobe: ein ACL-faehiges Volume darf eine oeffentliche DACL nicht akzeptieren.
        assert!(check_private_volume(8, || Ok(false)).is_err());
        assert!(check_private_volume(8, || Err("Lesefehler".into())).is_err());
        assert!(check_private_volume(8, || Ok(true)).is_ok());
    }

    #[cfg(windows)]
    fn private_dacl(path: &std::path::Path) -> bool {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::{PCWSTR, PWSTR};
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::Security::Authorization::{
            ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
        };
        use windows::Win32::Security::{
            GetAce, GetSecurityDescriptorControl, ACCESS_ALLOWED_ACE, ACL,
            DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
        };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let mut acl: *mut ACL = std::ptr::null_mut();
            let mut sd = PSECURITY_DESCRIPTOR::default();
            GetNamedSecurityInfoW(PCWSTR(wide.as_ptr()), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION,
                None, None, Some(&mut acl), None, &mut sd).ok().unwrap();
            let mut control = 0;
            let mut revision = 0;
            GetSecurityDescriptorControl(sd, &mut control, &mut revision).unwrap();
            let result = if acl.is_null() || (*acl).AceCount != 1 {
                false
            } else {
                let mut ace = std::ptr::null_mut();
                GetAce(acl, 0, &mut ace).unwrap();
                let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                let mut text = PWSTR::null();
                ConvertSidToStringSidW(PSID((&ace.SidStart as *const u32).cast_mut().cast()), &mut text).unwrap();
                let sid = text.to_string().unwrap();
                let _ = LocalFree(Some(HLOCAL(text.0.cast())));
                control & SE_DACL_PROTECTED.0 != 0 && ace.Header.AceType == 0
                    && ace.Header.AceFlags == 0 && sid == user_sid().unwrap()
                    && ace.Mask & 0x001f01ff == 0x001f01ff
            };
            let _ = LocalFree(Some(HLOCAL(sd.0)));
            result
        }
    }

    #[cfg(windows)]
    #[test]
    fn private_tmp_has_exactly_the_token_sid_from_creation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("synthetisch.tmp");
        write_private_tmp(&path, b"synthetisch").unwrap();
        assert!(private_dacl(&path));
        // Gegenprobe: eine zweite Anlage darf weder Inhalt noch Rechte ersetzen.
        assert!(write_private_tmp(&path, b"ersetzt").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"synthetisch");
        assert!(private_dacl(&path));
        // Ein absichtlich ungeschuetzt angelegter Vergleichspfad muss abfallen.
        let public = dir.path().join("geerbt.tmp");
        std::fs::write(&public, b"synthetisch").unwrap();
        assert!(!private_dacl(&public));
    }

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
