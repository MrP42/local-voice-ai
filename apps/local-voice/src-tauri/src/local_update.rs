//! Updates aus einem lokalen Ordner.
//!
//! Der GitHub-Updater braucht eine signierte `latest.json`; ohne den privaten
//! Schluessel im CI kommt auf diesem Weg nichts an. Fuer Abnahmestaende, die
//! lokal gebaut werden (`dev.ps1 bundle`), schaut die App zusaetzlich in einen
//! eingestellten Ordner: liegt dort ein Installer mit hoeherer Version als die
//! laufende, wird er in der Fussleiste angeboten und auf Klick gestartet.
//! Kein Netz, keine Signatur — der Ordner ist Vertrauensgrenze, deshalb ist er
//! eine bewusste Einstellung und kein Standardpfad.

use serde::Serialize;
use specta::Type;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct LocalUpdate {
    pub version: String,
    pub path: String,
    pub file_name: String,
}

/// `X.Y.Z` (optional mit fuehrendem `v`), sonst `None`.
pub(crate) fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.trim().trim_start_matches('v').split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Version aus einem Tauri-NSIS-Dateinamen `<Name>_<X.Y.Z>_<arch>-setup.exe`.
/// Andere Dateien (DMG, Portable-ZIP, fremde EXE) ergeben `None`.
pub(crate) fn version_from_file_name(name: &str) -> Option<String> {
    if !name.to_ascii_lowercase().ends_with("-setup.exe") {
        return None;
    }
    let mut parts = name.rsplitn(3, '_');
    let _arch_and_suffix = parts.next()?;
    let version = parts.next()?;
    let _app_name = parts.next()?;
    parse_version(version).map(|_| version.to_string())
}

/// Der neueste Installer im Ordner, der neuer ist als `current`.
pub(crate) fn find_local_update(dir: &Path, current: &str) -> Option<LocalUpdate> {
    let current = parse_version(current)?;
    let mut best: Option<((u64, u64, u64), LocalUpdate)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let Some(version) = version_from_file_name(&file_name) else {
            continue;
        };
        let Some(parsed) = parse_version(&version) else {
            continue;
        };
        if parsed <= current {
            continue;
        }
        if best.as_ref().is_none_or(|(b, _)| parsed > *b) {
            best = Some((
                parsed,
                LocalUpdate {
                    version,
                    path: entry.path().to_string_lossy().to_string(),
                    file_name,
                },
            ));
        }
    }
    best.map(|(_, update)| update)
}

fn configured_dir(app: &AppHandle) -> Option<PathBuf> {
    crate::settings::get_settings(app)
        .local_update_dir
        .filter(|d| !d.trim().is_empty())
        .map(PathBuf::from)
}

#[tauri::command]
#[specta::specta]
pub fn local_update_check(app: AppHandle) -> Result<Option<LocalUpdate>, String> {
    let Some(dir) = configured_dir(&app) else {
        return Ok(None);
    };
    let current = app.package_info().version.to_string();
    let found = find_local_update(&dir, &current);
    if let Some(u) = &found {
        log::info!(
            "local update: {} in {} (running {})",
            u.version,
            dir.display(),
            current
        );
    }
    Ok(found)
}

/// Startet den Installer aus dem eingestellten Ordner und beendet die App,
/// damit der Installer die Dateien ersetzen kann. Nur Dateien, die direkt im
/// eingestellten Ordner liegen und wie ein Installer heissen, werden gestartet.
#[tauri::command]
#[specta::specta]
pub fn local_update_install(app: AppHandle, path: String) -> Result<(), String> {
    let dir = configured_dir(&app).ok_or("Kein Ordner fuer lokale Updates eingestellt")?;
    let candidate = PathBuf::from(&path);
    let file_name = candidate
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or("Ungueltiger Pfad")?;
    if version_from_file_name(&file_name).is_none() {
        return Err(format!("{file_name} ist kein Installer dieser App"));
    }
    let canon_dir = std::fs::canonicalize(&dir).map_err(|e| format!("Ordner: {e}"))?;
    let canon_file = std::fs::canonicalize(&candidate).map_err(|e| format!("Datei: {e}"))?;
    if canon_file.parent() != Some(canon_dir.as_path()) {
        return Err("Der Installer liegt nicht im eingestellten Ordner".into());
    }

    #[cfg(target_os = "windows")]
    {
        // /P = passiv: Fortschritt sichtbar, keine Rueckfragen. Der Tauri-NSIS-
        // Installer ersetzt die laufende Installation an Ort und Stelle.
        std::process::Command::new(&candidate)
            .arg("/P")
            .spawn()
            .map_err(|e| format!("Installer konnte nicht gestartet werden: {e}"))?;
        log::info!("local update: installer started ({file_name}); exiting");
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            app.exit(0);
        });
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Err("Lokale Installer-Updates gibt es bisher nur unter Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_v_prefixed_versions() {
        assert_eq!(parse_version("0.18.3"), Some((0, 18, 3)));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("abc"), None);
    }

    #[test]
    fn reads_version_from_nsis_file_name() {
        assert_eq!(
            version_from_file_name("Local Voice AI_0.18.3_x64-setup.exe").as_deref(),
            Some("0.18.3")
        );
        assert_eq!(version_from_file_name("Local Voice AI_0.18.3_aarch64.dmg"), None);
        assert_eq!(version_from_file_name("setup.exe"), None);
        assert_eq!(version_from_file_name("Other_App_x64-setup.exe"), None);
    }

    #[test]
    fn picks_the_newest_installer_above_the_running_version() {
        let dir = tempfile::TempDir::new().unwrap();
        for name in [
            "Local Voice AI_0.18.1_x64-setup.exe",
            "Local Voice AI_0.18.3_x64-setup.exe",
            "Local Voice AI_0.18.10_x64-setup.exe",
            "Local Voice AI_0.18.10_aarch64.dmg",
            "notes.txt",
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let found = find_local_update(dir.path(), "0.18.3").expect("neuer Installer");
        assert_eq!(found.version, "0.18.10", "10 > 3 numerisch, nicht lexikalisch");
        assert!(found.path.ends_with("Local Voice AI_0.18.10_x64-setup.exe"));
        assert!(find_local_update(dir.path(), "0.18.10").is_none());
        assert!(find_local_update(dir.path(), "1.0.0").is_none());
    }

    #[test]
    fn missing_folder_is_not_an_error() {
        assert!(find_local_update(Path::new("Z:/gibt/es/nicht"), "0.1.0").is_none());
    }
}
