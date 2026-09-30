//! Erkennung eines SELBST installierten `yt-dlp` (A2, E1 „privat“).
//!
//! Die App buendelt yt-dlp nicht und laedt es nicht herunter (R1, R2). Hier wird
//! nur gefunden und die Version gezeigt: konfigurierter Pfad (Datei oder Ordner)
//! oder Suche im `PATH`. A3 darf das Programm spaeter nur benutzen, wenn der
//! Schalter „privat“ an ist UND das Tor `media.fetch` erlaubt.
//!
//! Sicherheit:
//! - Nie ueber den Namen starten: gesucht wird selbst, und zwar nur in absoluten
//!   `PATH`-Eintraegen. Windows wuerde einen blossen Namen zuerst im Arbeitsordner
//!   suchen (DLL-/EXE-Unterschiebung); der Start bekommt immer den absoluten Pfad.
//! - Nur ein Programm, dessen Dateiname mit `yt-dlp` beginnt (Windows: `.exe`,
//!   keine Skripte): ein falsch eingetragener Pfad startet nichts anderes.
//! - Feste Argumente (`--ignore-config --version`), nichts vom Nutzer; `--version`
//!   beendet yt-dlp vor jedem Netzzugriff und vor jedem Laden einer Konfigdatei.
//! - Kindprozess im Job-Objekt (`process_guard`: RAM-Deckel 1 GB, CPU-Deckel,
//!   niedrige Prioritaet, KILL_ON_JOB_CLOSE), Zeitlimit, Ausgabe begrenzt.
//! - Die Ausgabe muss genau eine Versionszeile sein (`2026.09.01`); alles andere
//!   heisst „kein yt-dlp“.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;

#[cfg(test)]
mod tests;

/// Wie lange `--version` hoechstens dauern darf. PyInstaller-Programme entpacken
/// beim ersten Start in einen Temp-Ordner; Virenscanner bremsen zusaetzlich.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
/// Mehr Ausgabe wird gelesen und verworfen.
pub const MAX_OUTPUT_BYTES: usize = 4096;
#[cfg(windows)]
const MEMORY_CAP_MB: u64 = 1024;
#[cfg(windows)]
const CPU_CAP_PERCENT: u32 = 50;
/// Wie lange nach dem Ende des Prozesses auf den Rest der Ausgabe gewartet wird.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Was bei der Erkennung schiefgehen kann; `code()` ist der Text fuer die Oberflaeche.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolError {
    NotFound,
    NotAbsolute,
    BadName,
    Timeout,
    /// Beendet mit einem Fehlercode.
    Failed(i32),
    StartFailed(String),
    /// Lief, antwortete aber nicht mit einer Versionszeile.
    NotYtdlp,
}

impl ToolError {
    pub fn code(&self) -> &'static str {
        match self {
            ToolError::NotFound => "not_found",
            ToolError::NotAbsolute => "not_absolute",
            ToolError::BadName => "bad_name",
            ToolError::Timeout => "timeout",
            ToolError::Failed(_) => "failed",
            ToolError::StartFailed(_) => "start_failed",
            ToolError::NotYtdlp => "not_ytdlp",
        }
    }
}

/// Ergebnis der Erkennung fuer die Oberflaeche.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Type)]
pub struct ToolStatus {
    pub found: bool,
    /// Der geprueft Pfad (auch wenn es misslang, zur Anzeige).
    pub path: Option<String>,
    pub version: Option<String>,
    /// `configured` oder `path`.
    pub source: Option<String>,
    /// Code eines Fehlers (siehe `ToolError::code`), sonst `None`.
    pub error: Option<String>,
}

impl ToolStatus {
    fn failed(path: Option<&Path>, source: Option<&str>, error: &ToolError) -> Self {
        Self {
            found: false,
            path: path.map(|p| p.display().to_string()),
            version: None,
            source: source.map(str::to_string),
            error: Some(error.code().to_string()),
        }
    }
}

/// `2026.09.01` oder `2026.09.01.232345` (Nightly), genau eine Zeile.
pub fn parse_version(output: &str) -> Option<String> {
    let text = output.trim_start_matches('\u{feff}').trim();
    if text.is_empty() || text.lines().count() != 1 {
        return None;
    }
    let ok = {
        let mut parts = text.split('.');
        let date_ok = matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some(y), Some(m), Some(d))
                if y.len() == 4 && m.len() == 2 && d.len() == 2
                    && [y, m, d].iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
        );
        let build_ok = match parts.next() {
            None => true,
            Some(b) => !b.is_empty() && b.bytes().all(|c| c.is_ascii_digit()),
        };
        date_ok && build_ok && parts.next().is_none()
    };
    ok.then(|| text.to_string())
}

/// Sieht der Dateiname nach yt-dlp aus (siehe Moduldoku)?
pub fn plausible_name(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(OsStr::to_str) else {
        return false;
    };
    if !stem.to_ascii_lowercase().starts_with("yt-dlp") {
        return false;
    }
    if cfg!(windows) {
        path.extension()
            .and_then(OsStr::to_str)
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    } else {
        true
    }
}

fn tool_file_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

/// Erstes `yt-dlp` in einem ABSOLUTEN Eintrag von `path_var`. Relative Eintraege
/// (auch `.`) zaehlen nie.
pub fn find_in_path(path_var: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(tool_file_name()))
        .find(|candidate| candidate.is_file())
}

/// Erkennung mit untergeschobenem Start (`run` liefert die Ausgabe von
/// `--version`). Reihenfolge: konfigurierter Pfad (gewinnt), sonst `PATH`.
pub fn detect_with(
    configured: Option<&str>,
    path_var: Option<&OsStr>,
    run: impl Fn(&Path) -> Result<String, ToolError>,
) -> ToolStatus {
    let probe = |exe: &Path, source: &str| match run(exe) {
        Ok(out) => match parse_version(&out) {
            Some(version) => ToolStatus {
                found: true,
                path: Some(exe.display().to_string()),
                version: Some(version),
                source: Some(source.to_string()),
                error: None,
            },
            None => ToolStatus::failed(Some(exe), Some(source), &ToolError::NotYtdlp),
        },
        Err(e) => ToolStatus::failed(Some(exe), Some(source), &e),
    };

    let configured = configured.map(str::trim).filter(|c| !c.is_empty());
    if let Some(value) = configured {
        let given = PathBuf::from(value);
        let fail = |e: ToolError| ToolStatus::failed(Some(&given), Some("configured"), &e);
        if !given.is_absolute() {
            return fail(ToolError::NotAbsolute);
        }
        if !given.exists() {
            return fail(ToolError::NotFound);
        }
        let exe = if given.is_dir() {
            let inside = given.join(tool_file_name());
            if !inside.is_file() {
                return fail(ToolError::NotFound);
            }
            inside
        } else {
            given.clone()
        };
        if !plausible_name(&exe) {
            return fail(ToolError::BadName);
        }
        return probe(&exe, "configured");
    }
    match path_var.and_then(find_in_path) {
        Some(exe) => probe(&exe, "path"),
        None => ToolStatus::failed(None, None, &ToolError::NotFound),
    }
}

/// Erkennung mit dem echten `PATH` und dem echten Start.
pub fn detect(configured: Option<&str>) -> ToolStatus {
    detect_with(configured, std::env::var_os("PATH").as_deref(), |exe| {
        run_version(exe, VERSION_TIMEOUT)
    })
}

/// Liest bis zum Ende, behaelt aber nur die ersten `MAX_OUTPUT_BYTES`: so
/// blockiert ein gespraechiges Programm nie an einer vollen Leitung.
fn read_limited(mut pipe: impl Read) -> Vec<u8> {
    let mut kept: Vec<u8> = Vec::new();
    let mut buf = [0u8; 1024];
    while let Ok(n) = pipe.read(&mut buf) {
        if n == 0 {
            break;
        }
        let room = MAX_OUTPUT_BYTES.saturating_sub(kept.len());
        kept.extend_from_slice(&buf[..n.min(room)]);
    }
    kept
}

/// Startet `<exe> --ignore-config --version` (siehe Moduldoku) und liefert die
/// Standardausgabe.
pub fn run_version(exe: &Path, timeout: Duration) -> Result<String, ToolError> {
    if !exe.is_file() {
        return Err(ToolError::NotFound);
    }
    let mut cmd = Command::new(exe);
    cmd.args(["--ignore-config", "--version"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| ToolError::StartFailed(e.kind().to_string()))?;
    #[cfg(windows)]
    let job =
        crate::process_guard::ProcessGuard::attach(&child, Some(MEMORY_CAP_MB), CPU_CAP_PERCENT);
    #[cfg(not(windows))]
    let job: Option<crate::process_guard::ProcessGuard> = None;

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let _ = tx.send(read_limited(stdout));
        });
    } else {
        drop(tx);
    }

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                // Beenden ueber dieses Prozess-Handle, danach das Job-Objekt
                // schliessen: auch Kinder des Programms (Skript, Entpacker) gehen mit.
                let _ = child.kill();
                let _ = child.wait();
                drop(job);
                return Err(ToolError::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(job);
                return Err(ToolError::StartFailed(e.kind().to_string()));
            }
        }
    };
    // Die Ausgabe ist mit dem Ende des Prozesses da; haelt ein Kind die Leitung
    // offen, wird es nach kurzer Frist mit dem Job-Objekt beendet.
    let output = match rx.recv_timeout(DRAIN_GRACE) {
        Ok(bytes) => bytes,
        Err(_) => {
            drop(job);
            rx.recv_timeout(DRAIN_GRACE).unwrap_or_default()
        }
    };
    if !status.success() {
        return Err(ToolError::Failed(status.code().unwrap_or(-1)));
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}
