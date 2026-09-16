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

/// Version aus einem Tauri-NSIS-Dateinamen `<Name>_<X.Y.Z>_<arch>-setup.exe`,
/// aber NUR fuer diese App: der Name vor der Version muss `app_name` sein.
/// Am 15.09.2026 startete die App sonst `Anarlog_1.4.10_x64-setup.exe` aus
/// dem Downloads-Ordner — ein fremdes Programm, weil nur das Muster geprueft
/// wurde. Andere Dateien (DMG, Portable-ZIP, fremde EXE) ergeben `None`.
pub(crate) fn version_from_file_name(name: &str, app_name: &str) -> Option<String> {
    if !name.to_ascii_lowercase().ends_with("-setup.exe") {
        return None;
    }
    let mut parts = name.rsplitn(3, '_');
    let _arch_and_suffix = parts.next()?;
    let version = parts.next()?;
    let file_app_name = parts.next()?;
    if !file_app_name.eq_ignore_ascii_case(app_name) {
        return None;
    }
    parse_version(version).map(|_| version.to_string())
}

/// Der neueste Installer im Ordner, der neuer ist als `current`.
pub(crate) fn find_local_update(dir: &Path, current: &str, app_name: &str) -> Option<LocalUpdate> {
    let current = parse_version(current)?;
    let mut best: Option<((u64, u64, u64), LocalUpdate)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let Some(version) = version_from_file_name(&file_name, app_name) else {
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

/// Ordner fuer lokale Updates: die Einstellung, sonst Fallbacks, die auf
/// einem Entwicklungsrechner ohnehin existieren — der Bundle-Ordner des
/// Repos und der Download-Ordner des Nutzers. So funktioniert der Knopf
/// auch, bevor jemand die Einstellung entdeckt hat.
fn candidate_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = crate::settings::get_settings(app)
        .local_update_dir
        .filter(|d| !d.trim().is_empty())
    {
        dirs.push(PathBuf::from(d));
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(home);
        dirs.push(
            home.join("local-voice-project")
                .join("apps/local-voice/src-tauri/target/release/bundle/nsis"),
        );
    }
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

#[tauri::command]
#[specta::specta]
pub fn local_update_check(app: AppHandle) -> Result<Option<LocalUpdate>, String> {
    let current = app.package_info().version.to_string();
    let app_name = app.package_info().name.clone();
    let dirs = candidate_dirs(&app);
    log::info!(
        "local update: checking {} folder(s) for a version above {current}",
        dirs.len()
    );
    // Der neueste Fund ueber alle Ordner gewinnt.
    let found = dirs
        .iter()
        .filter_map(|dir| find_local_update(dir, &current, &app_name))
        .max_by_key(|u| parse_version(&u.version));
    match &found {
        Some(u) => log::info!("local update: {} at {}", u.version, u.path),
        None => log::info!("local update: none found"),
    }
    Ok(found)
}

/// Startet den Installer aus dem eingestellten Ordner und beendet die App,
/// damit der Installer die Dateien ersetzen kann. Nur Dateien, die direkt im
/// eingestellten Ordner liegen und wie ein Installer heissen, werden gestartet.
#[tauri::command]
#[specta::specta]
pub fn local_update_install(app: AppHandle, path: String) -> Result<(), String> {
    let candidate = PathBuf::from(&path);
    let file_name = candidate
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or("Ungueltiger Pfad")?;
    let app_name = app.package_info().name.clone();
    if version_from_file_name(&file_name, &app_name).is_none() {
        return Err(format!("{file_name} ist kein Installer dieser App"));
    }
    // Nur Dateien direkt in einem der erlaubten Ordner werden gestartet.
    let canon_file = std::fs::canonicalize(&candidate).map_err(|e| format!("Datei: {e}"))?;
    let allowed = candidate_dirs(&app)
        .iter()
        .filter_map(|d| std::fs::canonicalize(d).ok())
        .any(|d| canon_file.parent() == Some(d.as_path()));
    if !allowed {
        return Err("Der Installer liegt in keinem erlaubten Update-Ordner".into());
    }

    #[cfg(target_os = "windows")]
    {
        // Reihenfolge ist entscheidend: erst die App beenden, DANN den
        // Installer starten. Andersherum (16.09.2026) kam der Installer bei
        // "Extract: local-voice-ai.exe" an, waehrend die EXE noch lief ->
        // "error writing to file". Ein abgekoppelter cmd-Helfer wartet, bis
        // unser Prozess weg ist, startet dann den Installer passiv (/P) und
        // danach die neue App wieder — wie es der GitHub-Updater auch tut.
        use std::os::windows::process::CommandExt;
        let pid = std::process::id();
        let exe = std::env::current_exe().map_err(|e| format!("eigener Pfad: {e}"))?;
        let script = format!(
            "@echo off
:wait
tasklist /FI \"PID eq {pid}\" 2>NUL | find \"{pid}\" >NUL && (timeout /t 1 /nobreak >NUL & goto wait)
start \"\" /wait \"{installer}\" /P
start \"\" \"{exe}\"
del \"%~f0\"
",
            installer = candidate.display(),
            exe = exe.display()
        );
        let helper = std::env::temp_dir().join(format!("local-voice-update-{pid}.cmd"));
        std::fs::write(&helper, script).map_err(|e| format!("Update-Helfer: {e}"))?;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        std::process::Command::new("cmd")
            .args(["/C", &helper.to_string_lossy()])
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn()
            .map_err(|e| format!("Update-Helfer konnte nicht gestartet werden: {e}"))?;
        log::info!("local update: helper armed for {file_name}; exiting so the installer can replace the exe");
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
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
        let app = "Local Voice AI";
        assert_eq!(
            version_from_file_name("Local Voice AI_0.18.3_x64-setup.exe", app).as_deref(),
            Some("0.18.3")
        );
        assert_eq!(version_from_file_name("Local Voice AI_0.18.3_aarch64.dmg", app), None);
        assert_eq!(version_from_file_name("setup.exe", app), None);
        assert_eq!(version_from_file_name("Other_App_x64-setup.exe", app), None);
        // Der Vorfall vom 15.09.: fremdes Programm, passendes Muster.
        assert_eq!(version_from_file_name("Anarlog_1.4.10_x64-setup.exe", app), None);
    }

    #[test]
    fn picks_the_newest_installer_above_the_running_version() {
        let dir = tempfile::TempDir::new().unwrap();
        for name in [
            "Local Voice AI_0.18.1_x64-setup.exe",
            "Local Voice AI_0.18.3_x64-setup.exe",
            "Local Voice AI_0.18.10_x64-setup.exe",
            "Local Voice AI_0.18.10_aarch64.dmg",
            "Anarlog_1.4.10_x64-setup.exe",
            "notes.txt",
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let app = "Local Voice AI";
        let found = find_local_update(dir.path(), "0.18.3", app).expect("neuer Installer");
        assert_eq!(found.version, "0.18.10", "10 > 3 numerisch, nicht lexikalisch; Anarlog 1.4.10 zaehlt nicht");
        assert!(found.path.ends_with("Local Voice AI_0.18.10_x64-setup.exe"));
        assert!(find_local_update(dir.path(), "0.18.10", app).is_none());
        assert!(find_local_update(dir.path(), "1.0.0", app).is_none());
    }

    #[test]
    fn missing_folder_is_not_an_error() {
        assert!(find_local_update(Path::new("Z:/gibt/es/nicht"), "0.1.0", "Local Voice AI").is_none());
    }
}
