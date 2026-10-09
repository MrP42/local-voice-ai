//! Die Codex-CLI selbst aktualisieren, wenn sie ein Modell nicht annimmt.
//!
//! Der OpenAI-Server lehnt neue Modelle (gpt-6.1-sol, gpt-6-sol) von einer zu
//! alten Codex-CLI ab: Fehler `cli_model_not_in_plan`, Antwort nach drei
//! Sekunden ohne Token. Statt das Protokoll scheitern zu lassen und den Nutzer
//! zu einem Terminal zu schicken, stoesst die App das Update selbst an:
//! `npm i -g @openai/codex@latest`, nur wenn die CLI per npm installiert ist,
//! hoechstens einmal je sechs Stunden und ohne Shell (Node startet npm direkt).
//! Der Ausgang geht als Ereignis `cli-update` an die Oberflaeche.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::cli::{self, Cli};

/// Frueheste Wiederholung nach einem Versuch (Sekunden).
const RETRY_AFTER_SECS: i64 = 6 * 3600;
/// Zeitgrenze fuer `npm i -g`.
const NPM_TIMEOUT: Duration = Duration::from_secs(300);
const PACKAGE: &str = "@openai/codex@latest";

static APP: OnceLock<AppHandle> = OnceLock::new();
static RUNNING: AtomicBool = AtomicBool::new(false);
static LAST_TRY: AtomicI64 = AtomicI64::new(0);

/// Was die Oberflaeche erfaehrt (`state`: `started`, `done`, `failed`, `manual`).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CliUpdateEvent {
    pub cli: String,
    pub state: String,
    pub version: Option<String>,
    pub message: Option<String>,
}

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Nur dieser Fehler heisst "CLI zu alt": der Server nimmt das Modell von dieser
/// Version nicht an.
pub fn is_cli_too_old(err: &str) -> bool {
    err.starts_with("cli_model_not_in_plan")
}

/// Ist ein neuer Versuch erlaubt? (`last` = Zeitpunkt des letzten Versuchs.)
pub fn may_try(now: i64, last: i64) -> bool {
    last == 0 || now - last >= RETRY_AFTER_SECS
}

/// Liegt die CLI in einem npm-Ordner (`…\npm\codex.cmd`, `…/npm/bin/codex`)?
/// Nur dann ist `npm i -g` der richtige Weg, sie zu ersetzen.
pub fn is_npm_managed(binary: &Path) -> bool {
    binary
        .components()
        .any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("npm"))
}

/// `npm-cli.js` neben der Node-Installation (`<node-ordner>/node_modules/npm/bin`).
pub fn npm_cli_js(node: &Path) -> Option<PathBuf> {
    let js = node
        .parent()?
        .join("node_modules")
        .join("npm")
        .join("bin")
        .join("npm-cli.js");
    js.is_file().then_some(js)
}

fn emit(state: &str, version: Option<String>, message: Option<String>) {
    if let Some(app) = APP.get() {
        let _ = app.emit(
            "cli-update",
            CliUpdateEvent {
                cli: "codex".into(),
                state: state.into(),
                version,
                message,
            },
        );
    }
}

/// Wird mit dem Fehler eines CLI-Aufrufs gerufen; stoesst bei "CLI zu alt" das
/// Update an (im Hintergrund, nie auf dem Antwortpfad).
pub fn on_cli_error(which: Cli, err: &str) {
    if which == Cli::Codex && is_cli_too_old(err) {
        start_codex_update();
    }
}

pub fn start_codex_update() {
    let t = now();
    if !may_try(t, LAST_TRY.load(Ordering::SeqCst)) || RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    LAST_TRY.store(t, Ordering::SeqCst);
    std::thread::spawn(|| {
        emit("started", None, None);
        match update_codex() {
            Ok(version) => emit("done", Some(version), None),
            Err(Manual(why)) => emit("manual", None, Some(why)),
            Err(Failed(why)) => emit("failed", None, Some(why)),
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

enum Outcome {
    Manual(String),
    Failed(String),
}
use Outcome::{Failed, Manual};

fn update_codex() -> Result<String, Outcome> {
    let binary = cli::locate(Cli::Codex)
        .ok_or_else(|| Manual("Die Codex-CLI wurde nicht gefunden.".to_string()))?;
    if !is_npm_managed(&binary) {
        return Err(Manual(format!(
            "Die Codex-CLI liegt nicht in einem npm-Ordner ({}).",
            binary.display()
        )));
    }
    let node = cli::find_node()
        .ok_or_else(|| Manual("Node.js wurde nicht gefunden.".to_string()))?;
    let npm = npm_cli_js(&node)
        .ok_or_else(|| Manual("npm wurde neben Node.js nicht gefunden.".to_string()))?;

    let mut cmd = Command::new(&node);
    cmd.arg(&npm)
        .args(["i", "-g", PACKAGE])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_window(&mut cmd);
    let child = cmd.spawn().map_err(|e| Failed(format!("npm: {e}")))?;
    let output = cli::wait_with_timeout(child, NPM_TIMEOUT).map_err(Failed)?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
        return Err(Failed(format!("npm i -g {PACKAGE}: {last}")));
    }
    let (program, mut argv) = cli::launcher(&binary);
    argv.push("--version".into());
    let mut v = Command::new(program);
    v.args(argv).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    hide_window(&mut v);
    let version = v
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "neueste Version".to_string());
    Ok(version)
}

fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Die Ursache des letzten gescheiterten CLI-Aufrufs als Zusatz fuer eine
/// Fehlermeldung ("" wenn keiner juengst gescheitert ist).
static LAST_ERROR: std::sync::Mutex<Option<(i64, String)>> = std::sync::Mutex::new(None);

pub fn remember_error(err: &str) {
    if let Ok(mut last) = LAST_ERROR.lock() {
        *last = Some((now(), err.to_string()));
    }
}

/// ` (Ursache: …)`, wenn in den letzten zwei Minuten ein CLI-Aufruf gescheitert ist.
pub fn recent_error_note() -> String {
    LAST_ERROR
        .lock()
        .ok()
        .and_then(|l| l.clone())
        .filter(|(at, _)| now() - at <= 120)
        .map(|(_, e)| format!(" (Ursache: {})", e.split_once(": ").map_or(e.as_str(), |x| x.1)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_model_not_accepted_error_means_too_old() {
        assert!(is_cli_too_old("cli_model_not_in_plan: Dieses Modell nimmt der Server …"));
        for e in ["cli_failed: x", "cli_limit_reached: x", "cli_not_logged_in: x", "cli_missing: x"] {
            assert!(!is_cli_too_old(e), "{e}");
        }
    }

    #[test]
    fn an_attempt_is_allowed_once_per_six_hours() {
        assert!(may_try(1000, 0));
        assert!(!may_try(1000 + RETRY_AFTER_SECS - 1, 1000));
        assert!(may_try(1000 + RETRY_AFTER_SECS, 1000));
    }

    #[test]
    fn only_npm_installations_are_updated_by_npm() {
        assert!(is_npm_managed(Path::new(r"C:\Users\x\AppData\Roaming\npm\codex.cmd")));
        assert!(is_npm_managed(Path::new("/usr/lib/npm/bin/codex")));
        assert!(!is_npm_managed(Path::new(r"C:\Tools\codex\codex.exe")));
        assert!(!is_npm_managed(Path::new(r"C:\Users\x\.local\bin\codex")));
    }

    #[test]
    fn npm_cli_is_looked_up_next_to_node() {
        let dir = std::env::temp_dir().join(format!("lv-npm-test-{}", std::process::id()));
        let bin = dir.join("node_modules").join("npm").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let node = dir.join("node.exe");
        assert_eq!(npm_cli_js(&node), None, "ohne npm-cli.js");
        std::fs::write(bin.join("npm-cli.js"), "").unwrap();
        assert_eq!(npm_cli_js(&node), Some(bin.join("npm-cli.js")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_error_note_names_the_cause_without_the_code_prefix() {
        remember_error("cli_model_not_in_plan: Codex-CLI zu alt");
        assert_eq!(recent_error_note(), " (Ursache: Codex-CLI zu alt)");
    }
}
