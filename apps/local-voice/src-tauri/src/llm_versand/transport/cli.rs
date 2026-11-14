//! CLI-Rohtransport: Prozessstart, Vorschauprüfung, Beobachter (lva#604, Teilpaket 3).
//!
//! Diese Einstiege standen bis Teilpaket 3 in `managers::llm::cli` (`call`, `call_with_timeout`,
//! `call_with_options`, dazu `call_inner` mit der Vorschauprüfung `preview_precheck` und
//! `verify_preview`). Sie sind hierher **verschoben**: außerhalb von `llm_versand` startet
//! nichts mehr das Modellprogramm der CLIs. Argumentaufbau, Parser, Fehlerklassifikation,
//! `CallOptions` und die Prozesshilfen bleiben in `managers::llm::cli` (kein Netzzugriff, nur
//! `pub(crate)` für diesen Transport).
//!
//! Das Verhalten ist unverändert. Roh-Fehlertexte der CLI (`cli_failed: <letzte stderr-Zeile>`)
//! verlassen dieses Modul nur über [`super::cli_chat`], das sie klassifiziert und verwirft.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::managers::llm::cli::{
    ancestors_clean, args_with, check_usage, classify_error, codex_isolation_args, launcher,
    locate_native_claude, max_tokens_args, parse_claude, parse_claude_stream, parse_codex,
    run_cancellable_in, run_mit_abgriff, split_fast, split_spec, valid_effort, valid_model_name,
    CallOptions, Cli, CliReply, CALL_TIMEOUT, CLAUDE_PREVIEW_ARGS, CLAUDE_PREVIEW_ENV,
};
use crate::managers::llm::preview;

/// Ein Aufruf. Blockiert -- auf einem Blocking-Thread ausfuehren.
#[cfg(test)]
pub(super) fn call(
    cli: Cli,
    binary: &Path,
    model: &str,
    system: Option<&str>,
    user: &str,
) -> Result<CliReply, String> {
    call_with_timeout(cli, binary, model, system, user, CALL_TIMEOUT)
}

/// Wie [`call`] mit eigener Zeitgrenze (der Probelauf fuer neue Modelle
/// wartet hoechstens Sekunden, nicht Minuten).
pub(super) fn call_with_timeout(
    cli: Cli,
    binary: &Path,
    model: &str,
    system: Option<&str>,
    user: &str,
    timeout: Duration,
) -> Result<CliReply, String> {
    call_with_options(
        cli,
        binary,
        model,
        system,
        user,
        timeout,
        &CallOptions::default(),
    )
}

/// Wie [`call_with_timeout`], mit [`CallOptions`].
pub(super) fn call_with_options(
    cli: Cli,
    binary: &Path,
    model: &str,
    system: Option<&str>,
    user: &str,
    timeout: Duration,
    opts: &CallOptions,
) -> Result<CliReply, String> {
    let result = call_inner(cli, binary, model, system, user, timeout, opts);
    if let (Some(check), Err(e)) = (&opts.preview, &result) {
        // Ablehnungen sind schon im Audit; hier die uebrigen Fehler (nur der Code).
        if !e.starts_with(preview::REJECTED_PREFIX) {
            let code = e.split(':').next().unwrap_or("error");
            log::info!(
                "{}",
                preview::audit_line(&check.binding, model, None, &format!("error:{code}"))
            );
        }
    }
    result
}

/// Audit-Zeile (ohne Inhalt) einer Ablehnung der Vorschau.
fn audit_rejection(
    check: &preview::PreviewCheck,
    requested: &str,
    actual: Option<&str>,
    reason: &str,
) {
    log::info!(
        "{}",
        preview::audit_line(
            &check.binding,
            requested,
            actual,
            &format!("rejected:{reason}")
        )
    );
}

/// Gemeinsame Pruefung der Vorschau: Antwort verwenden oder verwerfen, mit
/// Audit-Zeile (ohne Inhalt) in beiden Faellen.
fn verify_preview(
    check: &preview::PreviewCheck,
    requested: &str,
    reply: CliReply,
    input_bytes: usize,
) -> Result<CliReply, String> {
    let verdict = preview::verify(check, &reply, input_bytes);
    let status = match &verdict {
        Ok(()) => "ok".to_string(),
        Err(r) => format!("rejected:{}", r.reason),
    };
    log::info!(
        "{}",
        preview::audit_line(&check.binding, requested, reply.model.as_deref(), &status)
    );
    verdict.map(|()| reply).map_err(|r| r.error())
}

/// Setzt `--output-format stream-json` in die Claude-Argumente.
fn claude_stream_args(argv: &mut [String]) {
    if let Some(i) = argv.iter().position(|a| a == "--output-format") {
        if let Some(v) = argv.get_mut(i + 1) {
            *v = "stream-json".to_string();
        }
    }
}

/// Beobachtet waehrend des Laufs Schalter und Abbruch und setzt dann `kill`.
struct Watch {
    kill: std::sync::Arc<std::sync::atomic::AtomicBool>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    switched: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Watch {
    fn start(opts: &CallOptions) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        // Mit dem AKTUELLEN Wert des Originalsignals starten (Relay-/Desktop-Abbruch,
        // Vorschau-Schalter): ein schon gesetzter Abbruch gilt sofort und synchron,
        // nicht erst nach dem ersten Takt des Beobachter-Threads.
        let off = switch_is_off(opts);
        let cancelled = opts
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::SeqCst));
        let kill = Arc::new(AtomicBool::new(off || cancelled));
        let done = Arc::new(AtomicBool::new(false));
        let switched = Arc::new(AtomicBool::new(off));
        let handle = {
            let (kill, done, switched) = (kill.clone(), done.clone(), switched.clone());
            let cancel = opts.cancel.clone();
            let switch = opts.preview_switch.clone();
            std::thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    if switch.as_ref().is_some_and(|s| !s.is_on()) {
                        switched.store(true, Ordering::SeqCst);
                        kill.store(true, Ordering::SeqCst);
                        return;
                    }
                    if cancel.as_ref().is_some_and(|c| c.load(Ordering::SeqCst)) {
                        kill.store(true, Ordering::SeqCst);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
        };
        Self {
            kill,
            done,
            switched,
            handle: Some(handle),
        }
    }

    /// Beendet die Beobachtung; `true`, wenn der Schalter ausgeschaltet wurde.
    fn stop(mut self) -> bool {
        use std::sync::atomic::Ordering;
        self.done.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.switched.load(Ordering::SeqCst)
    }
}

fn switch_is_off(opts: &CallOptions) -> bool {
    opts.preview_switch.as_ref().is_some_and(|s| !s.is_on())
}

/// Vorab-Pruefungen der Vorschau: Codex (immer gesperrt), Schalter, Budget und
/// Bindung ans Programm. Gibt das gehaltene Original zurueck, das allein gestartet wird.
fn preview_precheck(
    cli: Cli,
    binary: &Path,
    check: &preview::PreviewCheck,
    input_bytes: usize,
    opts: &CallOptions,
) -> Result<preview::Pinned, preview::Rejection> {
    if cli != Cli::Claude {
        return Err(preview::CODEX_BLOCKED);
    }
    if switch_is_off(opts) {
        return Err(preview::SWITCH_OFF);
    }
    preview::precheck(check, input_bytes)?;
    // Dieselbe Wahl wie bei der Bindung: die native Claude-Datei.
    #[cfg(not(test))]
    let source = locate_native_claude().unwrap_or_else(|_| binary.to_path_buf());
    #[cfg(test)]
    let source = binary.to_path_buf();
    preview::Pinned::prepare(&source, check)
}

fn call_inner(
    cli: Cli,
    binary: &Path,
    model: &str,
    system: Option<&str>,
    user: &str,
    timeout: Duration,
    opts: &CallOptions,
) -> Result<CliReply, String> {
    let requested = model.to_string();
    let input_bytes = system.map_or(0, str::len) + user.len();
    let mut launch: Option<preview::Pinned> = None;
    if let Some(check) = &opts.preview {
        match preview_precheck(cli, binary, check, input_bytes, opts) {
            Ok(copy) => launch = Some(copy),
            Err(r) => {
                audit_rejection(check, &requested, None, r.reason);
                return Err(r.error());
            }
        }
    }
    // Die Vorschau startet AUSSCHLIESSLICH das gehaltene, verifizierte Original.
    // Unter Linux ueber `/proc/self/fd/<fd>` des gehaltenen, gehashten Handles
    // (kein Austauschfenster zwischen Pruefung und Start).
    let exec_path = launch.as_ref().map(preview::Pinned::exec_path);
    let binary: &Path = exec_path.as_deref().unwrap_or(binary);
    // `modell@effort~fast`; auf die Kommandozeile kommt nur, was die Regeln erlaubt.
    let fast = split_fast(model).1;
    let (model, effort) = split_spec(model);
    if !valid_model_name(model) {
        return Err(format!("cli_model_not_in_plan: {model}"));
    }
    if let Some(e) = effort.filter(|e| !valid_effort(e)) {
        return Err(format!("cli_failed: unbekannte Effort-Stufe {e}"));
    }
    // Frischer, leerer Arbeitsordner je Aufruf; wird beim Verlassen geloescht.
    let work = tempfile::Builder::new()
        .prefix("local-voice-cli-")
        .tempdir()
        .map_err(|e| format!("cli_failed: {e}"))?;
    // Vorschau: kein Repo und keine Anweisungsdatei ueber dem Arbeitsordner.
    if opts.preview.is_some() {
        ancestors_clean(work.path())?;
    }
    // Codex erhaelt den Systemtext vor der Aufgabe -- `exec` kennt keinen
    // eigenen Systemprompt.
    let stdin_text = match (cli, system) {
        (Cli::Codex, Some(sys)) => format!("{sys}\n\n{user}"),
        _ => user.to_string(),
    };
    let (program, mut argv) = launcher(binary);
    let prefix = argv.clone();
    argv.extend(args_with(cli, model, effort, fast, system));
    if cli == Cli::Claude && opts.preview.is_some() {
        claude_stream_args(&mut argv);
        argv.extend(CLAUDE_PREVIEW_ARGS.iter().map(|a| a.to_string()));
    }
    if cli == Cli::Codex {
        // Vor dem abschliessenden "-" (Prompt aus stdin).
        let at = argv.len() - 1;
        let isolation = codex_isolation_args(&program, &prefix)?;
        argv.splice(at..at, isolation);
        if let Some(n) = opts.max_tokens {
            let at = argv.len() - 1;
            argv.splice(at..at, max_tokens_args(n));
        }
    }
    let mut cmd = Command::new(program);
    if cli == Cli::Claude {
        if let Some(n) = opts.max_tokens {
            cmd.env("CLAUDE_CODE_MAX_OUTPUT_TOKENS", n.to_string());
        }
        if opts.preview.is_some() {
            cmd.envs(CLAUDE_PREVIEW_ENV.iter().copied());
        }
    }
    cmd.args(&argv)
        .current_dir(work.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    // Vorschau, Unix: eigene Prozessgruppe, damit Widerruf, Abbruch und
    // Zeitablauf den ganzen Baum treffen (`kill(-pgid, ...)`). Windows: Job-Objekt.
    #[cfg(unix)]
    if opts.preview.is_some() {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    // Pruefung (1): Endpfade der gehaltenen Handles direkt vor dem Start (Windows:
    // Umbenennen wird erkannt, nicht verhindert; siehe `Pinned::check_paths`).
    if let (Some(pinned), Some(check)) = (launch.as_ref(), opts.preview.as_ref()) {
        if let Err(r) = pinned.check_paths() {
            audit_rejection(check, &requested, None, r.reason);
            return Err(r.error());
        }
    }
    let child = cmd.spawn().map_err(|e| format!("cli_missing: {e}"))?;
    let Some(check) = &opts.preview else {
        let output = run_mit_abgriff(
            child,
            Some(stdin_text.into_bytes()),
            timeout,
            opts.cancel.as_deref(),
            false,
            opts.stdout_zeile.as_ref(),
        )?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let reply = match cli {
            Cli::Claude => parse_claude(&stdout),
            Cli::Codex => parse_codex(&stdout),
        };
        return match reply {
            Ok(r) if opts.require_usage => check_usage(r),
            Ok(r) => Ok(r),
            Err(e) if e.starts_with("cli_bad_output") && !output.status.success() => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(classify_error(stderr.lines().last().unwrap_or("")))
            }
            Err(e) => Err(e),
        };
    };

    // ---- Vorschau: laufen lassen, beobachten, danach alles pruefen. ----
    let watch = Watch::start(opts);
    // Pruefung (2): direkt nach dem Spawn; Abweichung beendet den Prozessbaum.
    let moved = launch
        .as_ref()
        .and_then(|p| p.check_paths().err())
        .inspect(|_| watch.kill.store(true, std::sync::atomic::Ordering::SeqCst));
    let ran = run_cancellable_in(
        child,
        Some(stdin_text.into_bytes()),
        timeout,
        Some(watch.kill.as_ref()),
        true,
    );
    let switched = watch.stop();
    let reject = |r: preview::Rejection, actual: Option<&str>| {
        audit_rejection(check, &requested, actual, r.reason);
        r.error()
    };
    if let Some(r) = moved {
        return Err(reject(r, None));
    }
    let output = match ran {
        Err(_) if switched => return Err(reject(preview::SWITCH_OFF, None)),
        Err(e) => return Err(e),
        Ok(o) => o,
    };
    // Nach dem Aufruf, vor allem anderen: Schalter und Programm.
    if switched || switch_is_off(opts) {
        return Err(reject(preview::SWITCH_OFF, None));
    }
    if let Some(Err(r)) = launch.as_ref().map(preview::Pinned::verify) {
        return Err(reject(r, None));
    }
    // Exit-Code <> 0 ist nie ein Erfolg, auch wenn Text vorliegt.
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(
            match stderr.lines().last().filter(|l| !l.trim().is_empty()) {
                Some(line) => classify_error(line),
                None => "cli_failed: Die CLI endete mit einem Fehlercode".to_string(),
            },
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let reply = parse_claude_stream(&stdout).map_err(|e| {
        // Ablehnungen des Parsers (Ereignisvertrag, Modellwechsel, Kompaktierung)
        // gehoeren ins Audit wie die der Pruefung.
        if let Some(reason) = e
            .strip_prefix("cli_preview_rejected:")
            .and_then(|r| r.split(':').nth(1))
        {
            audit_rejection(check, &requested, None, reason);
        }
        e
    })?;
    let reply = verify_preview(check, &requested, reply, input_bytes)?;
    // Vor der Rueckgabe (Anzeige, Speichern, Relay) ein letztes Mal.
    if switch_is_off(opts) {
        return Err(reject(preview::SWITCH_OFF, None));
    }
    Ok(reply)
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod cli_tests;
