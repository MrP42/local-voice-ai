use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;

fn touch(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, b"x").unwrap();
    p
}

fn ok_runner(version: &'static str) -> impl Fn(&Path) -> Result<String, ToolError> {
    move |_| Ok(format!("{version}\r\n"))
}

// ---------------------------------------------------------------------------
// Version lesen
// ---------------------------------------------------------------------------

#[test]
fn a_version_line_is_recognised_strictly() {
    for (out, want) in [
        ("2026.09.01\n", Some("2026.09.01")),
        ("2026.09.01\r\n", Some("2026.09.01")),
        ("  2026.09.01  ", Some("2026.09.01")),
        ("2026.09.01.232345\n", Some("2026.09.01.232345")),
        ("\u{feff}2026.09.01\n", Some("2026.09.01")),
        ("", None),
        ("   \n", None),
        ("yt-dlp 2026.09.01\n", None),
        ("2026.9.1\n", None),
        ("Microsoft Windows [Version 10.0.26200]\n", None),
        ("2026.09.01\nzweite Zeile\n", None),
        ("2026.09.01; calc.exe\n", None),
        ("<html>", None),
    ] {
        assert_eq!(parse_version(out).as_deref(), want, "{out:?}");
    }
}

#[test]
fn the_file_name_must_look_like_ytdlp() {
    for ok in [
        "yt-dlp.exe",
        "YT-DLP.EXE",
        "yt-dlp_x86.exe",
        "yt-dlp-2026.09.01.exe",
    ] {
        assert!(plausible_name(Path::new(ok)), "{ok}");
    }
    for bad in [
        "cmd.exe",
        "powershell.exe",
        "not-yt-dlp.exe",
        "ytdlp-but-not.exe",
        "",
    ] {
        assert!(!plausible_name(Path::new(bad)), "{bad}");
    }
    if cfg!(windows) {
        // Keine Skripte: nur ein Programm wird gestartet.
        for bad in [
            "yt-dlp.bat",
            "yt-dlp.cmd",
            "yt-dlp.ps1",
            "yt-dlp.exe.txt",
            "yt-dlp",
            "yt-dlp.dll",
        ] {
            assert!(!plausible_name(Path::new(bad)), "{bad}");
        }
    }
}

// ---------------------------------------------------------------------------
// Suche im PATH (nie im aktuellen Ordner)
// ---------------------------------------------------------------------------

fn name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

#[test]
fn the_tool_is_found_in_the_first_path_entry_that_has_it() {
    let empty = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let in_a = touch(a.path(), name());
    touch(b.path(), name());
    let path = std::env::join_paths([empty.path(), a.path(), b.path()]).unwrap();
    assert_eq!(find_in_path(&path), Some(in_a));
}

#[test]
fn a_directory_with_the_tool_name_or_a_relative_entry_is_ignored() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join(name())).unwrap();
    // Relative Eintraege (auch ".") zaehlen nicht: Windows wuerde dort im
    // Arbeitsordner suchen, und genau das soll nie einen Start ausloesen.
    let path = std::env::join_paths([
        d.path().to_path_buf(),
        PathBuf::from("."),
        PathBuf::from("relativ"),
    ])
    .unwrap();
    assert_eq!(find_in_path(&path), None);
    assert_eq!(find_in_path(&OsString::new()), None);
}

// ---------------------------------------------------------------------------
// Erkennung (mit untergeschobenem Start)
// ---------------------------------------------------------------------------

#[test]
fn a_configured_tool_is_started_and_its_version_shown() {
    let d = tempfile::tempdir().unwrap();
    let exe = touch(d.path(), name());
    let status = detect_with(exe.to_str(), None, ok_runner("2026.09.01"));
    assert!(status.found, "{status:?}");
    assert_eq!(status.version.as_deref(), Some("2026.09.01"));
    assert_eq!(status.source.as_deref(), Some("configured"));
    assert_eq!(status.path.as_deref(), exe.to_str());
    assert_eq!(status.error, None);
}

#[test]
fn a_tool_in_the_path_is_found_when_nothing_is_configured() {
    let d = tempfile::tempdir().unwrap();
    let exe = touch(d.path(), name());
    let path = std::env::join_paths([d.path()]).unwrap();
    for configured in [None, Some(""), Some("   ")] {
        let status = detect_with(configured, Some(&path), ok_runner("2026.08.30"));
        assert!(status.found, "{configured:?} {status:?}");
        assert_eq!(status.source.as_deref(), Some("path"));
        assert_eq!(status.path.as_deref(), exe.to_str());
        assert_eq!(status.version.as_deref(), Some("2026.08.30"));
    }
}

#[test]
fn a_configured_path_wins_over_the_path_variable() {
    let cfg_dir = tempfile::tempdir().unwrap();
    let path_dir = tempfile::tempdir().unwrap();
    let configured = touch(cfg_dir.path(), name());
    touch(path_dir.path(), name());
    let path = std::env::join_paths([path_dir.path()]).unwrap();
    let started = std::sync::Mutex::new(Vec::new());
    let status = detect_with(configured.to_str(), Some(&path), |p| {
        started.lock().unwrap().push(p.to_path_buf());
        Ok("2026.09.01\n".to_string())
    });
    assert_eq!(status.source.as_deref(), Some("configured"));
    assert_eq!(started.lock().unwrap().as_slice(), [configured]);
}

#[test]
fn nothing_found_is_a_plain_not_found_without_starting_anything() {
    let d = tempfile::tempdir().unwrap();
    let path = std::env::join_paths([d.path()]).unwrap();
    let status = detect_with(None, Some(&path), |_| panic!("darf nichts starten"));
    assert!(!status.found);
    assert_eq!(status.error.as_deref(), Some("not_found"));
    assert_eq!(status.version, None);
    assert_eq!(status.path, None);
    // Kein PATH gesetzt: dasselbe.
    let status = detect_with(None, None, |_| panic!("darf nichts starten"));
    assert_eq!(status.error.as_deref(), Some("not_found"));
}

#[test]
fn a_bad_configured_path_is_explained_not_started() {
    let d = tempfile::tempdir().unwrap();
    let missing = d.path().join(name());
    let other = touch(d.path(), "notepad-alike.exe");
    let rel = PathBuf::from("yt-dlp.exe");
    let panic_runner = |_: &Path| -> Result<String, ToolError> { panic!("darf nichts starten") };
    for (value, code) in [
        (missing.to_str().unwrap(), "not_found"),
        (other.to_str().unwrap(), "bad_name"),
        (rel.to_str().unwrap(), "not_absolute"),
        ("yt-dlp", "not_absolute"),
    ] {
        let status = detect_with(Some(value), None, panic_runner);
        assert!(!status.found, "{value}");
        assert_eq!(status.error.as_deref(), Some(code), "{value}");
        assert_eq!(status.source.as_deref(), Some("configured"), "{value}");
    }
}

#[test]
fn a_configured_folder_is_searched_for_the_tool() {
    let d = tempfile::tempdir().unwrap();
    let exe = touch(d.path(), name());
    let status = detect_with(d.path().to_str(), None, ok_runner("2026.09.01"));
    assert!(status.found, "{status:?}");
    assert_eq!(status.path.as_deref(), exe.to_str());
    // Ein Ordner ohne das Programm: nicht gefunden.
    let empty = tempfile::tempdir().unwrap();
    let status = detect_with(empty.path().to_str(), None, |_| panic!("kein Start"));
    assert!(!status.found);
    assert_eq!(status.error.as_deref(), Some("not_found"));
}

#[test]
fn output_that_is_no_version_means_this_is_not_ytdlp() {
    let d = tempfile::tempdir().unwrap();
    let exe = touch(d.path(), name());
    let status = detect_with(exe.to_str(), None, |_| Ok("Hallo Welt\n".to_string()));
    assert!(!status.found);
    assert_eq!(status.error.as_deref(), Some("not_ytdlp"));
    assert_eq!(status.version, None);
    assert_eq!(
        status.path.as_deref(),
        exe.to_str(),
        "der Pfad bleibt zur Anzeige"
    );
}

#[test]
fn start_failures_carry_their_code() {
    let d = tempfile::tempdir().unwrap();
    let exe = touch(d.path(), name());
    for (err, code) in [
        (ToolError::Timeout, "timeout"),
        (ToolError::Failed(3), "failed"),
        (
            ToolError::StartFailed("Zugriff verweigert".into()),
            "start_failed",
        ),
        (ToolError::NotFound, "not_found"),
    ] {
        let status = detect_with(exe.to_str(), None, move |_| Err(err.clone()));
        assert!(!status.found);
        assert_eq!(status.error.as_deref(), Some(code));
    }
}

// ---------------------------------------------------------------------------
// Der echte Kindprozess (Windows: Batch-Dateien als Attrappen)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod process {
    use super::*;

    /// Frist fuer Laeufe, die gelingen sollen: nur die Grenze, ab der ein Tool als
    /// haengend gilt. Das Job-Objekt startet Kinder mit BELOW_NORMAL-Prioritaet; auf
    /// einem ausgelasteten Rechner (parallele Builds) verhungert selbst ein
    /// `cmd /c echo` Sekunden lang (gemessen 11 bis 21 s statt 0,1 s). Ein gesundes
    /// Tool braucht Millisekunden, die Frist kostet also nichts.
    const PATIENCE: Duration = Duration::from_secs(180);
    /// Ein haengendes Skript lebt laenger als jede Frist unten (200 s): nur das
    /// Beenden durch den Job kann den Aufruf vorher zurueckbringen.
    const HANG: &str = "ping -n 200 127.0.0.1 >nul";

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("@echo off\r\n{body}\r\n")).unwrap();
        p
    }

    #[test]
    fn a_tool_that_prints_its_version_is_read() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), "fake.cmd", "echo 2026.09.01");
        let out = run_version(&exe, PATIENCE).unwrap();
        assert_eq!(parse_version(&out).as_deref(), Some("2026.09.01"));
    }

    #[test]
    fn the_arguments_are_fixed_and_config_files_are_ignored() {
        let d = tempfile::tempdir().unwrap();
        // Die Attrappe gibt ihre Argumente aus: genau diese zwei, nichts vom Nutzer.
        let exe = script(d.path(), "args.cmd", "echo %*");
        let out = run_version(&exe, PATIENCE).unwrap();
        assert_eq!(out.trim(), "--ignore-config --version");
    }

    #[test]
    fn a_failing_tool_reports_its_exit_code() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), "fail.cmd", "exit /b 3");
        assert_eq!(
            run_version(&exe, PATIENCE).unwrap_err(),
            ToolError::Failed(3)
        );
    }

    #[test]
    fn a_hanging_tool_is_killed_at_the_time_limit() {
        let d = tempfile::tempdir().unwrap();
        // ping erzeugt ein Kind des Skripts: auch das muss mit dem Job-Objekt gehen.
        let exe = script(d.path(), "hang.cmd", HANG);
        let started = Instant::now();
        let err = run_version(&exe, Duration::from_millis(1500)).unwrap_err();
        assert_eq!(err, ToolError::Timeout);
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "kein Haenger: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_flood_of_output_is_bounded_and_does_not_block_the_tool() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(
            d.path(),
            "flood.cmd",
            "for /L %%i in (1,1,6000) do echo 2026.09.01 xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        );
        let out = run_version(&exe, PATIENCE).unwrap();
        assert!(out.len() <= MAX_OUTPUT_BYTES, "{}", out.len());
        assert!(
            parse_version(&out).is_none(),
            "mehr als eine Zeile ist keine Version"
        );
    }

    #[test]
    fn a_missing_file_is_not_started() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            run_version(&d.path().join("yt-dlp.exe"), Duration::from_secs(5)).unwrap_err(),
            ToolError::NotFound
        );
    }

    #[test]
    fn a_real_program_with_the_wrong_output_is_not_taken_for_ytdlp() {
        // Ein echtes Windows-Programm unter dem Namen yt-dlp.exe: es antwortet
        // auf `--ignore-config --version` nicht mit einer Versionszeile.
        let system = std::env::var_os("SystemRoot").map(PathBuf::from);
        let Some(whoami) = system
            .map(|s| s.join("System32").join("whoami.exe"))
            .filter(|p| p.exists())
        else {
            return;
        };
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("yt-dlp.exe");
        std::fs::copy(whoami, &exe).unwrap();
        let status = detect_with(exe.to_str(), None, |p| run_version(p, PATIENCE));
        assert!(!status.found, "{status:?}");
        assert!(
            matches!(status.error.as_deref(), Some("not_ytdlp") | Some("failed")),
            "{status:?}"
        );
    }
}
