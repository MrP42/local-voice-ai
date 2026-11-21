//! Updates aus einem lokalen Ordner.
//!
//! Der GitHub-Updater braucht eine signierte `latest.json`; ohne den privaten
//! Schluessel im CI kommt auf diesem Weg nichts an. Fuer Abnahmestaende, die
//! lokal gebaut werden (`dev.ps1 bundle`), schaut die App zusaetzlich in einen
//! eingestellten Ordner: liegt dort ein Installer mit hoeherer Version als die
//! laufende, wird er in der Fussleiste angeboten. Auf Klick werden sein Pfad
//! ausgegeben und sein Ordner geoeffnet; der Nutzer startet ihn selbst.
//! Kein Netz, keine Signatur — der Ordner ist Vertrauensgrenze, deshalb ist er
//! eine bewusste Einstellung und kein Standardpfad. Ohne Einstellung keine Suche.

use serde::Serialize;
use specta::Type;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct LocalUpdate {
    pub version: String,
    pub path: String,
    pub file_name: String,
    /// Release-Notes (Markdown) aus `<installer>.notes.md` neben dem Installer, fuer
    /// alle Versionen seit der installierten (neueste zuerst); `None` ohne Datei.
    pub notes: Option<String>,
}

/// Groesste Notizdatei, die gelesen wird.
const MAX_NOTES_BYTES: u64 = 32 * 1024;

/// Notizen zu einem Installer: `Local Voice AI_0.21.9_x64-setup.exe` ->
/// `Local Voice AI_0.21.9_x64-setup.notes.md`.
fn notes_for(installer: &Path) -> Option<String> {
    let stem = installer.file_stem()?.to_string_lossy().to_string();
    let path = installer.with_file_name(format!("{stem}.notes.md"));
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_NOTES_BYTES {
        return None;
    }
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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
    // Notizen aller neueren Installer im Ordner, um sie gesammelt zu zeigen.
    let mut all_notes: Vec<((u64, u64, u64), String, String)> = Vec::new();
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
        if let Some(n) = notes_for(&entry.path()) {
            all_notes.push((parsed, version.clone(), n));
        }
        if best.as_ref().is_none_or(|(b, _)| parsed > *b) {
            best = Some((
                parsed,
                LocalUpdate {
                    version,
                    path: entry.path().to_string_lossy().to_string(),
                    file_name,
                    notes: None,
                },
            ));
        }
    }
    all_notes.sort_by(|a, b| b.0.cmp(&a.0));
    best.map(|(_, mut update)| {
        if !all_notes.is_empty() {
            update.notes = Some(
                all_notes
                    .into_iter()
                    .map(|(_, v, n)| format!("### {v}\n\n{n}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            );
        }
        update
    })
}

/// Nur der ausdruecklich eingestellte, vorhandene Ordner ist erlaubt.
fn candidate_dirs_from(configured: Option<String>) -> Vec<PathBuf> {
    configured
        .filter(|d| !d.trim().is_empty())
        .map(PathBuf::from)
        .filter(|d| d.is_dir())
        .into_iter()
        .collect()
}

/// Ordner fuer lokale Updates: eine bewusste Einstellung, ohne Standardpfad.
fn candidate_dirs(app: &AppHandle) -> Vec<PathBuf> {
    candidate_dirs_from(crate::settings::get_settings(app).local_update_dir)
}

#[tauri::command]
#[specta::specta]
pub fn local_update_check(app: AppHandle) -> Result<Option<LocalUpdate>, String> {
    let current = app.package_info().version.to_string();
    let app_name = app.package_info().name.clone();
    let dirs = candidate_dirs(&app);
    if dirs.is_empty() {
        log::info!("local update: no folder configured");
        return Ok(None);
    }
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

/// Prueft App-Zugehoerigkeit und direkte Lage im erlaubten Ordner.
fn pruefe_installer(path: &str, app_name: &str, erlaubt: &[PathBuf]) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(path);
    let file_name = candidate
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or("Ungueltiger Pfad")?;
    if version_from_file_name(&file_name, app_name).is_none() {
        return Err(format!("{file_name} ist kein Installer dieser App"));
    }
    // Nur Dateien direkt in einem der erlaubten Ordner werden akzeptiert.
    let canon_file = std::fs::canonicalize(&candidate).map_err(|e| format!("Datei: {e}"))?;
    let allowed = erlaubt
        .iter()
        .filter_map(|d| std::fs::canonicalize(d).ok())
        .any(|d| canon_file.parent() == Some(d.as_path()));
    if !allowed {
        return Err("Der Installer liegt in keinem erlaubten Update-Ordner".into());
    }

    Ok(canon_file)
}

/// Entfernt das Windows-Verbatim-Praefix fuer die Anzeige.
fn anzeigepfad(p: &Path) -> String {
    let path = p.to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(&path).to_string()
}

/// Startet NICHTS (lva#746: signierter Update-Helfer folgt): gibt den geprueften
/// Installerpfad zurueck und oeffnet dessen Ordner. Der Nutzer beendet die App
/// und startet den Installer selbst sichtbar.
#[tauri::command]
#[specta::specta]
pub fn local_update_install(app: AppHandle, path: String) -> Result<String, String> {
    let canon_file = pruefe_installer(&path, &app.package_info().name, &candidate_dirs(&app))?;
    use tauri_plugin_opener::OpenerExt;
    if app.opener().reveal_item_in_dir(&canon_file).is_err() {
        log::warn!("local update: installer folder could not be opened");
    }
    let file_name = canon_file.file_name().unwrap_or_default().to_string_lossy();
    log::info!("local update: installer path handed to the user: {file_name}");
    Ok(anzeigepfad(&canon_file))
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
        assert_eq!(
            version_from_file_name("Local Voice AI_0.18.3_aarch64.dmg", app),
            None
        );
        assert_eq!(version_from_file_name("setup.exe", app), None);
        assert_eq!(version_from_file_name("Other_App_x64-setup.exe", app), None);
        // Der Vorfall vom 15.09.: fremdes Programm, passendes Muster.
        assert_eq!(
            version_from_file_name("Anarlog_1.4.10_x64-setup.exe", app),
            None
        );
    }

    #[test]
    fn notes_of_all_newer_installers_come_along_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        for v in ["0.21.7", "0.21.8", "0.21.9"] {
            std::fs::write(
                dir.path().join(format!("Local Voice AI_{v}_x64-setup.exe")),
                b"x",
            )
            .unwrap();
        }
        std::fs::write(
            dir.path().join("Local Voice AI_0.21.9_x64-setup.notes.md"),
            "- Neu: Fusszeile",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Local Voice AI_0.21.8_x64-setup.notes.md"),
            "- Neu: Effort",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Local Voice AI_0.21.7_x64-setup.notes.md"),
            "- alt",
        )
        .unwrap();
        let u = find_local_update(dir.path(), "0.21.7", "Local Voice AI").unwrap();
        assert_eq!(u.version, "0.21.9");
        let notes = u.notes.unwrap();
        assert!(notes.starts_with("### 0.21.9"), "{notes}");
        assert!(
            notes.contains(
                "### 0.21.8

- Neu: Effort"
            ),
            "{notes}"
        );
        assert!(
            !notes.contains("alt"),
            "installierte Version gehoert nicht dazu"
        );
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
        assert_eq!(
            found.version, "0.18.10",
            "10 > 3 numerisch, nicht lexikalisch; Anarlog 1.4.10 zaehlt nicht"
        );
        assert!(found.path.ends_with("Local Voice AI_0.18.10_x64-setup.exe"));
        assert!(find_local_update(dir.path(), "0.18.10", app).is_none());
        assert!(find_local_update(dir.path(), "1.0.0", app).is_none());
    }

    #[test]
    fn ohne_eingestellten_ordner_wird_nicht_gesucht() {
        assert!(candidate_dirs_from(None).is_empty());
        assert!(candidate_dirs_from(Some("".into())).is_empty());
        assert!(candidate_dirs_from(Some("   ".into())).is_empty());
        let dir = tempfile::tempdir().unwrap();
        let fehlt = dir.path().join("nicht-vorhanden");
        assert!(candidate_dirs_from(Some(fehlt.to_string_lossy().into_owned())).is_empty());
    }

    #[test]
    fn eingestellter_ordner_bleibt() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            candidate_dirs_from(Some(dir.path().to_string_lossy().into_owned())),
            vec![dir.path().to_path_buf()]
        );
        // Auch bei gesetztem USERPROFILE bleibt ohne Einstellung die Liste leer.
        assert!(candidate_dirs_from(None).is_empty());
        // Die Gegenprobe verbietet Profil-Fallbacks ohne globale Umgebungsaenderung.
        let src = include_str!("local_update.rs");
        let produktion = src.split("#[cfg(test)]").next().unwrap();
        assert!(!produktion.contains("std::env"));
    }

    #[test]
    fn produktionscode_startet_keine_shell_oder_skripte() {
        let src = include_str!("local_update.rs");
        let produktion = src.split("#[cfg(test)]").next().unwrap();
        for fragment in [
            "Command",
            "spawn",
            "cmd.exe",
            ".cmd",
            "tasklist",
            "ping",
            "temp_dir",
            "creation_flags",
            "CREATE_NO_WINDOW",
            "powershell",
            "app.exit",
            "std::process",
        ] {
            assert!(
                !produktion.contains(fragment),
                "Verbotenes Fragment: {fragment}"
            );
        }
    }

    #[test]
    fn anzeigepfad_entfernt_nur_das_verbatim_praefix() {
        assert_eq!(anzeigepfad(Path::new(r"\\?\C:\x\a.exe")), r"C:\x\a.exe");
        assert_eq!(
            anzeigepfad(Path::new(r"\\?\UNC\server\share\x.exe")),
            r"\\server\share\x.exe"
        );
        assert_eq!(anzeigepfad(Path::new(r"C:\x\a.exe")), r"C:\x\a.exe");
    }

    #[test]
    fn installer_befehl_liefert_pfad_statt_zu_starten() {
        let dir = tempfile::tempdir().unwrap();
        let anderer = tempfile::tempdir().unwrap();
        let app = "Local Voice AI";
        let name = "Local Voice AI_0.18.10_x64-setup.exe";
        let installer = dir.path().join(name);
        let fremd = dir.path().join("Anarlog_1.4.10_x64-setup.exe");
        let ausserhalb = anderer.path().join(name);
        for path in [&installer, &fremd, &ausserhalb] {
            std::fs::write(path, b"x").unwrap();
        }
        let erlaubt = vec![dir.path().to_path_buf()];
        assert_eq!(
            pruefe_installer(&installer.to_string_lossy(), app, &erlaubt).unwrap(),
            std::fs::canonicalize(&installer).unwrap()
        );
        assert!(pruefe_installer(&fremd.to_string_lossy(), app, &erlaubt).is_err());
        assert!(pruefe_installer(&ausserhalb.to_string_lossy(), app, &erlaubt).is_err());
        assert!(pruefe_installer(&installer.to_string_lossy(), app, &[]).is_err());
    }

    #[test]
    fn missing_folder_is_not_an_error() {
        assert!(
            find_local_update(Path::new("Z:/gibt/es/nicht"), "0.1.0", "Local Voice AI").is_none()
        );
    }
}
