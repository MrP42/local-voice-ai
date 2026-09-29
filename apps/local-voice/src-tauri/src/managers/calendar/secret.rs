//! Geheimnisse der Kalenderquellen (ICS-Adresse, spaeter Graph-Token) unter
//! Windows mit DPAPI verschluesselt in `<appdata>/secrets/<id>.bin`
//! (M5, `entwurf/m5-m6-kalender-export.md` §3 „Geheimnisse“).
//!
//! - Nicht in `settings_store.json`, nicht in `meetings.db`, nicht im Geraete-Sync.
//! - Benutzerbereich (`CryptProtectData` ohne `CRYPTPROTECT_LOCAL_MACHINE`): nur
//!   dasselbe Windows-Konto entschluesselt. Ein anderer Benutzer oder ein
//!   kopierter Portable-Ordner bekommt einen Fehler, keine Daten.
//! - Zusatzgeheimnis (Entropy) = fester Text + Name: eine `.bin`, die unter einem
//!   anderen Namen abgelegt wird, laesst sich nicht entschluesseln.
//! - Schreiben: erst in eine eindeutig benannte Temp-Datei im selben Ordner,
//!   `sync_all`, dann atomar umbenennen. Ein Abbruch laesst den alten Stand oder
//!   nichts zurueck, nie eine halbe Datei. Ein Rest `*.tmp` nach hartem Abbruch
//!   enthaelt nur Geheimtext.
//! - Klartext liegt nur in `Zeroizing`-Puffern; der vom System gelieferte Puffer
//!   wird vor `LocalFree` ueberschrieben.
//! - Keine Fehlermeldung enthaelt Klartext oder Adresse.
//! - Nicht unter Windows: `secret_put` meldet Fehler (macOS-Keychain folgt bei
//!   Bedarf); es wird nie unverschluesselt gespeichert.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"LVS1";
const ENTROPY_PREFIX: &str = "local-voice-ai/calendar-secret@1:";
const MAX_NAME_LEN: usize = 64;
/// Ein Geheimnis ist eine Adresse oder ein Token; mehr ist ein Fehler.
const MAX_SECRET_BYTES: usize = 64 * 1024;

static SECRETS_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Ordner der Geheimnisse einer App-Instanz: `<meetings-Sandbox>/secrets`, wenn
/// `LVA_MEETINGS_DIR` gesetzt ist (Harness/Tests beruehren nie echte Geheimnisse),
/// sonst `<appdata>/secrets`.
pub fn secrets_dir_for(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    if let Some(sandbox) = std::env::var(super::super::meetings::MEETINGS_DIR_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Ok(PathBuf::from(sandbox).join("secrets"));
    }
    Ok(crate::portable::app_data_dir(app)?.join("secrets"))
}

/// Legt den Ordner fuer `secret_put/get/delete` fest (beim Start des
/// Kalenderdienstes).
pub fn init_dir(dir: PathBuf) {
    *SECRETS_DIR.write().unwrap_or_else(|e| e.into_inner()) = Some(dir);
}

fn current_dir() -> Result<PathBuf, String> {
    SECRETS_DIR
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| "Der Geheimnisspeicher ist nicht initialisiert.".to_string())
}

pub fn secret_put(name: &str, data: &[u8]) -> Result<(), String> {
    put_in(&current_dir()?, name, data)
}

/// `Ok(None)`, wenn es das Geheimnis nicht gibt. `Err`, wenn die Datei da ist,
/// sich aber nicht entschluesseln laesst (anderer Benutzer, beschaedigt).
pub fn secret_get(name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    get_in(&current_dir()?, name)
}

/// Loescht das Geheimnis; ein fehlendes ist kein Fehler.
pub fn secret_delete(name: &str) {
    if let Ok(dir) = current_dir() {
        delete_in(&dir, name);
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn secret_path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err("Ungültiger Name für ein Geheimnis.".to_string());
    }
    Ok(dir.join(format!("{name}.bin")))
}

fn entropy_for(name: &str) -> Vec<u8> {
    format!("{ENTROPY_PREFIX}{name}").into_bytes()
}

pub(crate) fn put_in(dir: &Path, name: &str, data: &[u8]) -> Result<(), String> {
    let target = secret_path(dir, name)?;
    if data.is_empty() || data.len() > MAX_SECRET_BYTES {
        return Err("Das Geheimnis ist leer oder zu groß.".to_string());
    }
    let blob = dpapi::protect(data, &entropy_for(name))?;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Ordner für Geheimnisse nicht anlegbar: {e}"))?;

    let tmp = dir.join(format!(
        "{name}.{}.{}.tmp",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(MAGIC)?;
        f.write_all(&blob)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &target)
    };
    match write() {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(format!("Geheimnis nicht speicherbar: {e}"))
        }
    }
}

pub(crate) fn get_in(dir: &Path, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    let path = secret_path(dir, name)?;
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Geheimnis nicht lesbar: {e}")),
    };
    let Some(blob) = bytes.strip_prefix(MAGIC.as_slice()) else {
        return Err("Das Geheimnis ist beschädigt: Adresse neu eingeben.".to_string());
    };
    dpapi::unprotect(blob, &entropy_for(name)).map(Some)
}

pub(crate) fn delete_in(dir: &Path, name: &str) {
    if let Ok(path) = secret_path(dir, name) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(windows)]
mod dpapi {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    use zeroize::Zeroizing;

    fn blob_of(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    pub fn protect(data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob_of(data);
        let ent = blob_of(entropy);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: `input` und `ent` zeigen auf Puffer, die diesen Aufruf
        // ueberleben; `out` wird vom System mit LocalAlloc gefuellt und unten frei gegeben.
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                Some(&ent),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .map_err(|e| format!("Verschlüsselung (DPAPI) fehlgeschlagen: {e}"))?;
        // SAFETY: bei Erfolg zeigt `out.pbData` auf `out.cbData` gueltige Bytes.
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: der Puffer stammt aus LocalAlloc.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
        }
        Ok(v)
    }

    pub fn unprotect(blob: &[u8], entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        let input = blob_of(blob);
        let ent = blob_of(entropy);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: wie bei `protect`.
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&ent),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .map_err(|_| {
            "Das Geheimnis lässt sich nicht entschlüsseln (anderer Benutzer oder beschädigt): Adresse neu eingeben."
                .to_string()
        })?;
        // SAFETY: bei Erfolg zeigt `out.pbData` auf `out.cbData` gueltige Bytes.
        let plain = Zeroizing::new(
            unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec(),
        );
        // SAFETY: Klartext im Systempuffer vor der Freigabe ueberschreiben.
        unsafe {
            std::ptr::write_bytes(out.pbData, 0, out.cbData as usize);
            let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
        }
        Ok(plain)
    }
}

#[cfg(not(windows))]
mod dpapi {
    use zeroize::Zeroizing;

    const UNSUPPORTED: &str =
        "Geheimnisse werden nur unter Windows (DPAPI) verschlüsselt gespeichert.";

    pub fn protect(_data: &[u8], _entropy: &[u8]) -> Result<Vec<u8>, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub fn unprotect(_blob: &[u8], _entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        Err(UNSUPPORTED.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str =
        "https://outlook.office365.com/owa/calendar/aaaa-bbbb-cccc/SECRETKEY/reachcalendar.ics";

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip() {
        let dir = tmp();
        put_in(dir.path(), "src-01", URL.as_bytes()).unwrap();
        let got = get_in(dir.path(), "src-01").unwrap().unwrap();
        assert_eq!(got.as_slice(), URL.as_bytes());

        // Auf dem Datentraeger steht kein Klartext, nicht einmal Teile der Adresse.
        let raw = std::fs::read(dir.path().join("src-01.bin")).unwrap();
        assert!(raw.starts_with(MAGIC));
        for needle in ["SECRETKEY", "outlook", "reachcalendar"] {
            assert!(
                !raw.windows(needle.len()).any(|w| w == needle.as_bytes()),
                "{needle} im Klartext gefunden"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn overwrite_replaces_atomically_without_leftovers() {
        let dir = tmp();
        put_in(dir.path(), "src-01", b"https://a.example/1").unwrap();
        put_in(dir.path(), "src-01", b"https://a.example/2").unwrap();
        assert_eq!(
            get_in(dir.path(), "src-01").unwrap().unwrap().as_slice(),
            b"https://a.example/2"
        );
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["src-01.bin".to_string()], "kein .tmp-Rest");
    }

    #[test]
    fn a_missing_secret_is_none_not_an_error() {
        let dir = tmp();
        assert!(get_in(dir.path(), "nope").unwrap().is_none());
        // Auch im noch nicht angelegten Ordner.
        assert!(get_in(&dir.path().join("gibt-es-nicht"), "nope")
            .unwrap()
            .is_none());
    }

    #[cfg(windows)]
    #[test]
    fn a_damaged_blob_is_an_error_with_a_clear_message_and_no_panic() {
        let dir = tmp();
        put_in(dir.path(), "src-01", URL.as_bytes()).unwrap();
        let path = dir.path().join("src-01.bin");
        let mut raw = std::fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        raw[MAGIC.len() + 40] ^= 0xFF;
        std::fs::write(&path, &raw).unwrap();
        let err = get_in(dir.path(), "src-01").unwrap_err();
        assert!(err.contains("neu eingeben"), "{err}");
        assert!(!err.contains("SECRETKEY"));

        // Abgeschnitten und ohne Kennung.
        std::fs::write(&path, &raw[..8]).unwrap();
        assert!(get_in(dir.path(), "src-01").is_err());
        std::fs::write(&path, b"kein Geheimnis").unwrap();
        assert!(get_in(dir.path(), "src-01")
            .unwrap_err()
            .contains("beschädigt"));
    }

    #[cfg(windows)]
    #[test]
    fn a_blob_is_bound_to_its_name() {
        let dir = tmp();
        put_in(dir.path(), "src-01", URL.as_bytes()).unwrap();
        std::fs::copy(dir.path().join("src-01.bin"), dir.path().join("src-02.bin")).unwrap();
        assert!(get_in(dir.path(), "src-02").is_err());
        assert!(get_in(dir.path(), "src-01").unwrap().is_some());
    }

    #[test]
    fn delete_is_idempotent() {
        let dir = tmp();
        std::fs::write(dir.path().join("src-01.bin"), b"x").unwrap();
        delete_in(dir.path(), "src-01");
        assert!(!dir.path().join("src-01.bin").exists());
        delete_in(dir.path(), "src-01");
        delete_in(dir.path(), "../src-01");
    }

    #[test]
    fn names_that_could_leave_the_folder_are_refused() {
        let dir = tmp();
        for bad in ["", "../x", "a/b", "a\\b", "a b", "a.b", &"x".repeat(65)] {
            assert!(put_in(dir.path(), bad, b"x").is_err(), "{bad:?}");
            assert!(get_in(dir.path(), bad).is_err(), "{bad:?}");
        }
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn empty_and_oversized_secrets_are_refused_before_any_file_exists() {
        let dir = tmp();
        assert!(put_in(dir.path(), "src-01", b"").is_err());
        assert!(put_in(dir.path(), "src-01", &vec![b'x'; MAX_SECRET_BYTES + 1]).is_err());
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_write_leaves_no_temp_file_and_reports_an_error() {
        let dir = tmp();
        // Das Ziel ist ein nicht leerer Ordner: das Umbenennen muss scheitern.
        let target = dir.path().join("src-01.bin");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("x"), b"x").unwrap();
        let err = put_in(dir.path(), "src-01", URL.as_bytes()).unwrap_err();
        assert!(err.contains("nicht speicherbar"), "{err}");
        assert!(!err.contains("SECRETKEY"));
        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn the_global_store_needs_init() {
        // Ohne `init_dir` (in diesem Test-Prozess wird es nirgends gesetzt)
        // verweigert der Speicher, statt irgendwohin zu schreiben.
        assert!(secret_put("src-01", b"x").is_err());
        assert!(secret_get("src-01").is_err());
        secret_delete("src-01");
    }
}
