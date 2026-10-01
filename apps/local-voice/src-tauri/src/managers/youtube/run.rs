//! Ein Lauf des selbst installierten `yt-dlp` mit Zeitlimit, Abbruch und
//! begrenzter Ausgabe (A3). Gegenstueck zu `tool::run_version`, das nur die
//! Version liest.
//!
//! Sicherheit (gleiche Regeln wie `tool`):
//! - Der Start bekommt immer den absoluten Pfad aus der Erkennung, nie einen Namen.
//! - Die Argumente baut allein dieses Modul aus validierten Teilen (Video-ID,
//!   Sprachcode); nichts Freies vom Nutzer, `--ignore-config` immer dabei.
//! - Der Kindprozess haengt unter Windows im Job-Objekt (`process_guard`: RAM-
//!   und CPU-Deckel, niedrige Prioritaet, KILL_ON_JOB_CLOSE): Zeitlimit, Stopp und
//!   App-Ende beenden auch Kinder des Programms.
//! - Arbeitsordner ist ein eigener Temp-Ordner; der Ausgabename ist relativ
//!   (`%(id)s.%(ext)s`), es entsteht nichts ausserhalb davon.
//! - Stdout und Stderr werden nebenher gelesen (kein Haengen an vollen Puffern), nur
//!   ein begrenzter Teil wird behalten.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[cfg(windows)]
const MEMORY_CAP_MB: u64 = 1024;
#[cfg(windows)]
const CPU_CAP_PERCENT: u32 = 50;
/// Wie lange nach dem Ende des Prozesses auf den Rest der Ausgabe gewartet wird.
const DRAIN_GRACE: Duration = Duration::from_secs(2);
/// Behaltene Fehlerausgabe (Ende), fuer die Einordnung des Fehlers.
const STDERR_KEEP: usize = 4096;

pub struct RunOpts<'a> {
    pub timeout: Duration,
    /// Mehr Standardausgabe wird gelesen und verworfen.
    pub max_stdout: usize,
    pub cancel: Option<&'a AtomicBool>,
    /// Arbeitsordner des Prozesses (der Temp-Ordner dieses Laufs).
    pub cwd: Option<&'a Path>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunEnd {
    Finished {
        /// `-1`, wenn das Betriebssystem keinen Code nennt.
        code: i32,
        stdout: Vec<u8>,
        stderr_tail: String,
    },
    TimedOut,
    Cancelled,
}

fn read_limited(mut pipe: impl Read, keep: usize, tail: bool) -> Vec<u8> {
    let mut kept: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = pipe.read(&mut buf) {
        if n == 0 {
            break;
        }
        kept.extend_from_slice(&buf[..n]);
        if kept.len() > keep {
            if tail {
                let cut = kept.len() - keep;
                kept.drain(..cut);
            } else {
                kept.truncate(keep);
            }
        }
    }
    kept
}

/// Startet `exe args...` (siehe Moduldoku). `Err` nur, wenn der Start selbst
/// scheitert (Art des Fehlers, ohne Pfad).
pub fn run(exe: &Path, args: &[String], opts: &RunOpts<'_>) -> Result<RunEnd, String> {
    if opts.cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
        return Ok(RunEnd::Cancelled);
    }
    if !exe.is_file() {
        return Err("not_found".to_string());
    }
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = opts.cwd {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| e.kind().to_string())?;
    #[cfg(windows)]
    let job =
        crate::process_guard::ProcessGuard::attach(&child, Some(MEMORY_CAP_MB), CPU_CAP_PERCENT);
    #[cfg(not(windows))]
    let job: Option<crate::process_guard::ProcessGuard> = None;

    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
    if let Some(stdout) = child.stdout.take() {
        let keep = opts.max_stdout;
        std::thread::spawn(move || {
            let _ = out_tx.send(read_limited(stdout, keep, false));
        });
    } else {
        drop(out_tx);
    }
    let (err_tx, err_rx) = mpsc::channel::<Vec<u8>>();
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let _ = err_tx.send(read_limited(stderr, STDERR_KEEP, true));
        });
    } else {
        drop(err_tx);
    }

    let deadline = Instant::now() + opts.timeout;
    let stop_child = |child: &mut std::process::Child, job: Option<_>| {
        // Beenden ueber dieses Handle, danach das Job-Objekt schliessen: auch
        // Kinder des Programms (Skript, Entpacker) gehen mit.
        let _ = child.kill();
        let _ = child.wait();
        drop(job);
    };
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if opts.cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
                    stop_child(&mut child, job);
                    return Ok(RunEnd::Cancelled);
                }
                if Instant::now() >= deadline {
                    stop_child(&mut child, job);
                    return Ok(RunEnd::TimedOut);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                stop_child(&mut child, job);
                return Err(e.kind().to_string());
            }
        }
    };
    // Haelt ein Kind die Leitung offen, wird es nach kurzer Frist mit dem
    // Job-Objekt beendet.
    let stdout = match out_rx.recv_timeout(DRAIN_GRACE) {
        Ok(bytes) => bytes,
        Err(_) => {
            drop(job);
            out_rx.recv_timeout(DRAIN_GRACE).unwrap_or_default()
        }
    };
    let stderr = err_rx.recv_timeout(DRAIN_GRACE).unwrap_or_default();
    Ok(RunEnd::Finished {
        code: status.code().unwrap_or(-1),
        stdout,
        stderr_tail: String::from_utf8_lossy(&stderr).into_owned(),
    })
}
