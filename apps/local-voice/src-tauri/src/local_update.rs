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

/// Ordner fuer lokale Updates: die Einstellung, sonst Fallbacks, die auf
/// einem Entwicklungsrechner ohnehin existieren — der Bundle-Ordner des
/// Repos und der Download-Ordner des Nutzers. So funktioniert der Knopf
/// auch, bevor jemand die Einstellung entdeckt hat.
/// Ob ein Pfad unveraendert in ein cmd-Skript darf: keine Anfuehrungszeichen,
/// keine Verkettungs-, Umleitungs- oder Variablenzeichen, keine Zeilenumbrueche.
pub(crate) fn cmd_safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.chars().any(|c| {
            matches!(
                c,
                '"' | '&' | '|' | '<' | '>' | '^' | '%' | '!' | '\n' | '\r' | '\0'
            )
        })
}

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

/// Der neueste Installer ueber alle Update-Ordner, der neuer ist als die
/// laufende Version.
fn find_newest(app: &AppHandle) -> Option<LocalUpdate> {
    let current = app.package_info().version.to_string();
    let app_name = app.package_info().name.clone();
    candidate_dirs(app)
        .iter()
        .filter_map(|dir| find_local_update(dir, &current, &app_name))
        .max_by_key(|u| parse_version(&u.version))
}

#[tauri::command]
#[specta::specta]
pub fn local_update_check(app: AppHandle) -> Result<Option<LocalUpdate>, String> {
    log::info!(
        "local update: checking for a version above {}",
        app.package_info().version
    );
    // Der neueste Fund ueber alle Ordner gewinnt.
    let found = find_newest(&app);
    match &found {
        Some(u) => log::info!("local update: {} at {}", u.version, u.path),
        None => log::info!("local update: none found"),
    }
    Ok(found)
}

/// Ereignis an die Oberflaeche: eine neue Testversion liegt bereit.
pub const LOCAL_UPDATE_EVENT: &str = "lokales-update-bereit";

/// Abstand zwischen zwei Blicken in die Update-Ordner.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Groesse und Aenderungszeit: erst wenn beides ueber zwei Blicke gleich
/// bleibt, ist der Installer fertig kopiert.
fn fingerprint(update: &LocalUpdate) -> Option<(String, u64, Option<std::time::SystemTime>)> {
    let meta = std::fs::metadata(&update.path).ok()?;
    Some((update.path.clone(), meta.len(), meta.modified().ok()))
}

/// Was der Waechter mit einem Fund tut: melden, sobald er stabil ist, und
/// jede Version nur einmal je Programmlauf. Als gemeldet gilt eine Version
/// erst, wenn das Ereignis wirklich hinausging (`announced`); eine schon
/// gemeldete Version kommt auch nach einem Wechsel A -> B -> A nicht wieder.
#[derive(Default)]
pub(crate) struct Watch {
    last: Option<(String, u64, Option<std::time::SystemTime>)>,
    announced: std::collections::HashSet<String>,
}

impl Watch {
    pub(crate) fn should_announce(
        &mut self,
        version: &str,
        fp: Option<(String, u64, Option<std::time::SystemTime>)>,
    ) -> bool {
        let stable = fp.is_some() && fp == self.last;
        self.last = fp;
        stable && !self.announced.contains(version)
    }

    pub(crate) fn announced(&mut self, version: &str) {
        self.announced.insert(version.to_string());
    }
}

type DictationState = std::sync::Arc<crate::managers::audio::AudioRecordingManager>;
type MeetingState = std::sync::Arc<crate::managers::meetings::recorder::MeetingRecorderManager>;
type TtsState = crate::managers::tts::TtsManager;

/// Belegt beim Start des Waechters, dass die Sperr-Zustaende unter genau
/// diesen Typen registriert sind; sonst fiele der Schutz still aus.
fn log_busy_sources(app: &AppHandle) {
    use tauri::Manager;
    let (d, m, t) = (
        app.try_state::<DictationState>().is_some(),
        app.try_state::<MeetingState>().is_some(),
        app.try_state::<TtsState>().is_some(),
    );
    if d && m && t {
        log::info!("local update: Sperren aktiv (Diktat, Besprechung, Vorlesen)");
    } else {
        log::warn!("local update: Sperre fehlt: Diktat {d}, Besprechung {m}, Vorlesen {t}");
    }
}

/// Laeuft gerade etwas, das ein Neustart zerstoeren wuerde (Diktat,
/// Besprechungsaufnahme oder deren Abschluss, Vorlesen)? Dann wartet das
/// Angebot bis zum naechsten Blick.
fn busy(app: &AppHandle) -> bool {
    use tauri::Manager;
    let dictation = app
        .try_state::<DictationState>()
        .is_some_and(|m| m.is_recording());
    let meeting = app
        .try_state::<MeetingState>()
        .is_some_and(|m| m.is_recording() || m.final_jobs_active());
    let speaking = app
        .try_state::<TtsState>()
        .is_some_and(|m| m.status().phase == crate::managers::tts::state::TtsPhase::Speaking);
    dictation || meeting || speaking
}

/// Testversionen sofort anbieten: ein Hintergrund-Thread schaut alle 30 s in
/// die Update-Ordner. Liegt dort eine neuere, fertig kopierte Version, geht
/// sie als Ereignis an die Oberflaeche, die das Update-Fenster oeffnet. Ein
/// Klick auf „Installieren“ beendet die App, installiert und startet neu —
/// wie beim Online-Update. Ohne Update-Ordner passiert nichts.
///
/// Grenzen (bewusst, Entwickler-PC): Der Waechter laeuft in jedem Build, wird
/// aber nur aktiv, wenn ein Update-Ordner existiert. Der Installationsweg ist
/// derselbe wie beim manuellen „Lokales Update“ und prueft keine Signatur;
/// installiert wird nur nach Klick und nur aus den Update-Ordnern. Die
/// Stabilitaetspruefung (Groesse + Zeit) ist eine Heuristik; das Build-Skript
/// veroeffentlicht deshalb atomar per Umbenennen.
pub fn start_watch(app: AppHandle) {
    use tauri::Emitter;
    let spawned = std::thread::Builder::new()
        .name("local-update-watch".into())
        .spawn(move || {
            let mut watch = Watch::default();
            log_busy_sources(&app);
            loop {
                std::thread::sleep(WATCH_INTERVAL);
                let Some(update) = find_newest(&app) else {
                    continue;
                };
                if !watch.should_announce(&update.version, fingerprint(&update)) || busy(&app) {
                    continue;
                }
                match app.emit(LOCAL_UPDATE_EVENT, &update) {
                    Ok(()) => {
                        log::info!("local update: {} bereit, angeboten", update.version);
                        watch.announced(&update.version);
                    }
                    Err(e) => log::warn!("local update: Angebot nicht gesendet: {e}"),
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("local update: Waechter nicht gestartet: {e}");
    }
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
        // Alle Befehle mit vollem System32-Pfad: auf Rechnern mit Git im PATH
        // ist `find` GNU find, die Warteschleife bricht sofort ab und der
        // Installer startet nie (16.09.2026, zwei liegengebliebene Helfer in
        // %TEMP%). `ping` statt `timeout` als Pause, weil `timeout` ohne
        // Konsoleneingabe abbricht.
        // Reihenfolge ist entscheidend: erst die App beenden, DANN den
        // Installer starten. Andersherum (16.09.2026) kam der Installer bei
        // "Extract: local-voice-ai.exe" an, waehrend die EXE noch lief ->
        // "error writing to file". Ein abgekoppelter cmd-Helfer wartet, bis
        // unser Prozess weg ist, startet dann den Installer passiv (/P) und
        // danach die neue App wieder — wie es der GitHub-Updater auch tut.
        use std::os::windows::process::CommandExt;
        let pid = std::process::id();
        let exe = std::env::current_exe().map_err(|e| format!("eigener Pfad: {e}"))?;
        // Beide Pfade werden woertlich in ein cmd-Skript geschrieben. Ein
        // Ordnername aus der Einstellung mit Shell-Metazeichen koennte das
        // Skript aufbrechen — deshalb nur Pfade ohne solche Zeichen.
        for path in [&candidate, &exe] {
            if !cmd_safe_path(&path.to_string_lossy()) {
                return Err(format!(
                    "Pfad enthaelt Zeichen, die im Update-Helfer nicht erlaubt sind: {}",
                    path.display()
                ));
            }
        }
        let script = format!(
            "@echo off
:wait
\"%SystemRoot%\\System32\\tasklist.exe\" /FI \"PID eq {pid}\" /NH 2>NUL | \"%SystemRoot%\\System32\\find.exe\" \"{pid}\" >NUL && (\"%SystemRoot%\\System32\\ping.exe\" -n 2 127.0.0.1 >NUL & goto wait)
start \"\" /wait \"{installer}\" /P
start \"\" \"{exe}\"
del \"%~f0\"
",
            installer = candidate.display(),
            exe = exe.display()
        );
        let helper = std::env::temp_dir().join(format!("local-voice-update-{pid}.cmd"));
        std::fs::write(&helper, script).map_err(|e| format!("Update-Helfer: {e}"))?;
        // Nur CREATE_NO_WINDOW, kein DETACHED_PROCESS: der Helfer braucht eine
        // (unsichtbare) Konsole, sonst laufen Pipe und `start` nicht zuverlaessig.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let cmd = std::env::var_os("SystemRoot")
            .map(|r| PathBuf::from(r).join("System32").join("cmd.exe"))
            .unwrap_or_else(|| PathBuf::from("cmd.exe"));
        std::process::Command::new(cmd)
            .args(["/C", &helper.to_string_lossy()])
            .creation_flags(CREATE_NO_WINDOW)
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
    fn watch_announces_each_stable_version_once() {
        let fp = |len| Some(("a.exe".to_string(), len, None));
        let mut w = Watch::default();
        assert!(
            !w.should_announce("0.23.5", fp(10)),
            "erster Blick: nicht stabil"
        );
        assert!(!w.should_announce("0.23.5", fp(20)), "waechst noch");
        assert!(w.should_announce("0.23.5", fp(20)), "stabil: melden");
        assert!(
            w.should_announce("0.23.5", fp(20)),
            "Senden scheiterte: erneut"
        );
        w.announced("0.23.5");
        assert!(!w.should_announce("0.23.5", fp(20)), "nur einmal");
        assert!(!w.should_announce("0.23.6", fp(30)));
        assert!(w.should_announce("0.23.6", fp(30)), "neue Version: melden");
        w.announced("0.23.6");
        assert!(!w.should_announce("0.23.5", fp(20)));
        assert!(
            !w.should_announce("0.23.5", fp(20)),
            "A -> B -> A: nicht wieder"
        );
        assert!(!w.should_announce("0.23.7", None), "ohne Datei nie");
    }

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
    fn helper_refuses_shell_metacharacters_in_paths() {
        assert!(cmd_safe_path(
            r"C:\Users\wolff\local-voice-project\Local Voice AI_0.18.10_x64-setup.exe"
        ));
        assert!(cmd_safe_path(
            r"C:\Users\Jörg Müller\Local Voice AI\local-voice-ai.exe"
        ));
        for bad in [
            r#"C:\a" & calc & "\Local Voice AI_1.0.0_x64-setup.exe"#,
            r"C:\a&calc\x.exe",
            r"C:\a|calc\x.exe",
            r"C:\%TEMP%\x.exe",
            "C:\\a\nstart calc\\x.exe",
            "",
        ] {
            assert!(!cmd_safe_path(bad), "{bad:?} muss abgelehnt werden");
        }
    }

    #[test]
    fn missing_folder_is_not_an_error() {
        assert!(
            find_local_update(Path::new("Z:/gibt/es/nicht"), "0.1.0", "Local Voice AI").is_none()
        );
    }
}
