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

/// Liest die Einstellung `cli_auto_update`. Von aussen gesetzt (wie bei
/// `managers::usage`): ein Aufruf von `settings::get_settings(&app)` hier
/// liesse die Test-EXE eine Windows-Bibliothek importieren, die sie nicht
/// laden kann (STATUS_ENTRYPOINT_NOT_FOUND).
type AutoUpdateSource = std::sync::Arc<dyn Fn() -> bool + Send + Sync>;
static AUTO_UPDATE: OnceLock<AutoUpdateSource> = OnceLock::new();

pub fn init(app: AppHandle, auto_update: AutoUpdateSource) {
    let _ = APP.set(app);
    let _ = AUTO_UPDATE.set(auto_update);
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

/// Hat npm diese CLI installiert? Windows: `…\npm\codex.cmd` mit dem Paket in
/// `…\npm\node_modules\@openai\codex`. macOS/Linux: `codex` ist ein Link auf
/// `…/lib/node_modules/@openai/codex/bin/codex.js` (Homebrew-Node, nvm, npm-Praefix).
/// Nur dann ist `npm i -g` der richtige Weg, sie zu ersetzen.
pub fn is_npm_managed(binary: &Path) -> bool {
    let real = std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    let in_package = |p: &Path| {
        let parts: Vec<String> = p
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
            .collect();
        parts
            .windows(2)
            .any(|w| w[0] == "node_modules" && w[1] == "@openai")
    };
    in_package(&real)
        || binary
            .parent()
            .is_some_and(|dir| dir.join("node_modules").join("@openai").join("codex").is_dir())
}

/// `npm-cli.js` zu einer Node-Installation: Windows `<ordner>/node_modules/npm/bin`,
/// macOS/Linux `<praefix>/lib/node_modules/npm/bin` (Node liegt in `<praefix>/bin`).
pub fn npm_cli_js(node: &Path) -> Option<PathBuf> {
    let mut nodes = vec![node.to_path_buf()];
    if let Ok(real) = std::fs::canonicalize(node) {
        nodes.push(real);
    }
    for n in nodes {
        let Some(dir) = n.parent() else { continue };
        let tail = ["node_modules", "npm", "bin", "npm-cli.js"];
        let windows = tail.iter().fold(dir.to_path_buf(), |p, c| p.join(c));
        let unix = tail
            .iter()
            .fold(dir.join("..").join("lib"), |p, c| p.join(c));
        for js in [windows, unix] {
            if js.is_file() {
                return Some(js);
            }
        }
    }
    None
}

/// Darf die App die CLI von sich aus aktualisieren? (Einstellung `cli_auto_update`)
fn auto_update_enabled() -> bool {
    AUTO_UPDATE.get().map(|read| read()).unwrap_or(true)
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

/// Ergebnis des letzten Updates (fuer Aufrufe, die auf ein laufendes Update warten).
static LAST_OK: AtomicBool = AtomicBool::new(false);

/// Hat dieser Fehler mit einer zu alten Codex-CLI zu tun?
pub fn on_cli_error(which: Cli, err: &str) -> bool {
    which == Cli::Codex && is_cli_too_old(err)
}

/// Aktualisiert die Codex-CLI und kehrt erst danach zurueck (blockiert, bis zu
/// fuenf Minuten -- in einem Blocking-Thread rufen). `true`: die CLI ist jetzt
/// neu, der Aufruf lohnt einen zweiten Versuch. Laeuft schon ein Update, wartet
/// der Aufruf darauf; ein Versuch je sechs Stunden, sonst `false`.
pub fn ensure_codex_updated() -> bool {
    if !auto_update_enabled() {
        emit(
            "manual",
            None,
            Some("Das automatische Aktualisieren ist in den Einstellungen ausgeschaltet.".to_string()),
        );
        return false;
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        let started = std::time::Instant::now();
        while RUNNING.load(Ordering::SeqCst) && started.elapsed() < NPM_TIMEOUT {
            std::thread::sleep(Duration::from_millis(500));
        }
        return LAST_OK.load(Ordering::SeqCst);
    }
    let t = now();
    if !may_try(t, LAST_TRY.load(Ordering::SeqCst)) {
        RUNNING.store(false, Ordering::SeqCst);
        return false;
    }
    LAST_TRY.store(t, Ordering::SeqCst);
    emit("started", None, None);
    let ok = match update_codex() {
        Ok(version) => {
            emit("done", Some(version), None);
            true
        }
        Err(Manual(why)) => {
            emit("manual", None, Some(why));
            false
        }
        Err(Failed(why)) => {
            emit("failed", None, Some(why));
            false
        }
    };
    LAST_OK.store(ok, Ordering::SeqCst);
    RUNNING.store(false, Ordering::SeqCst);
    ok
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
        // macOS/Linux: der Link zeigt in das npm-Paket (hier ohne echte Datei: Pfad zaehlt).
        assert!(is_npm_managed(Path::new("/usr/local/lib/node_modules/@openai/codex/bin/codex.js")));
        assert!(!is_npm_managed(Path::new("/opt/homebrew/Caskroom/codex/codex")));
        assert!(!is_npm_managed(Path::new(r"C:\Tools\codex\codex.exe")));
        // Windows: codex.cmd neben dem Paket im npm-Ordner.
        let dir = std::env::temp_dir().join(format!("lv-npm-managed-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("node_modules").join("@openai").join("codex")).unwrap();
        assert!(is_npm_managed(&dir.join("codex.cmd")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn npm_cli_is_found_in_the_windows_and_the_unix_layout() {
        let base = std::env::temp_dir().join(format!("lv-npm-test-{}", std::process::id()));
        // Windows: node.exe neben node_modules\npm
        let win = base.join("win");
        let bin = win.join("node_modules").join("npm").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        assert_eq!(npm_cli_js(&win.join("node.exe")), None, "ohne npm-cli.js");
        std::fs::write(bin.join("npm-cli.js"), "").unwrap();
        assert!(npm_cli_js(&win.join("node.exe")).is_some());
        // macOS/Linux: <praefix>/bin/node, <praefix>/lib/node_modules/npm
        let unix = base.join("unix");
        std::fs::create_dir_all(unix.join("bin")).unwrap();
        let ubin = unix.join("lib").join("node_modules").join("npm").join("bin");
        std::fs::create_dir_all(&ubin).unwrap();
        std::fs::write(ubin.join("npm-cli.js"), "").unwrap();
        assert!(npm_cli_js(&unix.join("bin").join("node")).is_some());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn the_error_note_names_the_cause_without_the_code_prefix() {
        remember_error("cli_model_not_in_plan: Codex-CLI zu alt");
        assert_eq!(recent_error_note(), " (Ursache: Codex-CLI zu alt)");
    }
}
