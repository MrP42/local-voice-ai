//! Abo-Modelle ueber die offiziellen CLIs: Claude Code (`claude -p`, Claude
//! Pro/Max) und Codex (`codex exec`, ChatGPT Plus/Pro).
//!
//! Die App ruft nur das unveraenderte Programm des Anbieters auf; Anmeldung
//! und Zugangsdaten bleiben ganz bei der CLI -- die App liest, speichert oder
//! vermittelt nichts davon (Bedingung aus den Anthropic-Regeln, Recherche
//! 06.10.2026).
//!
//! Jeder Aufruf ist isoliert (Spike 06.10.2026): ohne die persoenlichen
//! Einstellungen, Hooks, Skills und MCP-Server des Nutzers, ohne Werkzeuge, in
//! einem leeren Arbeitsordner. Sonst laedt `claude -p` rund 89.000 Token
//! Kontext und fuehrt die Hooks des Nutzers aus.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Adresse der Vorlagen -- daran erkennt `llm_client` den CLI-Weg.
pub const CLAUDE_CLI_URL: &str = "cli://claude";
pub const CODEX_CLI_URL: &str = "cli://codex";

/// Wie lange ein Aufruf hoechstens dauern darf (lange Protokolle).
const CALL_TIMEOUT: Duration = Duration::from_secs(600);

/// Modelle, die die CLI mit einem Abo annimmt. Claude: Aliase der CLI.
/// Codex: mit ChatGPT-Konto lehnt der Server u. a. `gpt-6.1-sol` und `gpt-6`
/// ab, `gpt-6-astra` geht (Spike 06.10.2026).
pub const CLAUDE_MODELS: &[&str] = &["sonnet", "opus", "haiku"];
pub const CODEX_MODELS: &[&str] = &["gpt-6-astra"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cli {
    Claude,
    Codex,
}

impl Cli {
    pub fn from_base_url(url: &str) -> Option<Self> {
        match url.trim_end_matches('/') {
            CLAUDE_CLI_URL => Some(Self::Claude),
            CODEX_CLI_URL => Some(Self::Codex),
            _ => None,
        }
    }

    pub fn models(self) -> &'static [&'static str] {
        match self {
            Self::Claude => CLAUDE_MODELS,
            Self::Codex => CODEX_MODELS,
        }
    }
}

/// Antwort eines CLI-Aufrufs samt Token (soweit die CLI sie meldet).
#[derive(Debug, Clone, PartialEq)]
pub struct CliReply {
    pub text: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// Wo liegt die CLI? PATH zuerst, dann die ueblichen Installationsorte.
pub fn locate(cli: Cli) -> Option<PathBuf> {
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let names: &[&str] = match (cli, cfg!(windows)) {
        // Nur das native Programm: ein `.cmd`-Skript liefe ueber cmd.exe, und
        // der Systemprompt im Argument koennte dort Befehle einschleusen.
        (Cli::Claude, true) => &["claude.exe"],
        (Cli::Claude, false) => &["claude"],
        (Cli::Codex, true) => &["codex.cmd", "codex.exe"],
        (Cli::Codex, false) => &["codex"],
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(h) = &home {
        dirs.push(h.join(".local").join("bin"));
        dirs.push(h.join(".claude").join("local"));
    }
    if let Some(a) = &appdata {
        dirs.push(a.join("npm"));
    }
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

/// Argumente des isolierten Aufrufs (Prompt kommt ueber stdin).
pub(crate) fn args(cli: Cli, model: &str, system: Option<&str>) -> Vec<String> {
    match cli {
        Cli::Claude => {
            let mut a: Vec<String> = [
                "-p",
                "--output-format",
                "json",
                "--model",
                model,
                "--tools",
                "",
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--strict-mcp-config",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
                "--disable-slash-commands",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            if let Some(sys) = system {
                a.push("--system-prompt".into());
                a.push(sys.to_string());
            }
            a
        }
        Cli::Codex => [
            "exec",
            "--json",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--ephemeral",
            "-m",
            model,
            "-c",
            "mcp_servers={}",
            "-",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
    }
}

/// Liest die Antwort von `claude -p --output-format json`.
pub(crate) fn parse_claude(stdout: &str) -> Result<CliReply, String> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("cli_bad_output: Claude-Antwort unlesbar ({e})"))?;
    if v.get("is_error").and_then(|b| b.as_bool()) == Some(true) {
        let msg = v
            .get("result")
            .and_then(|r| r.as_str())
            .unwrap_or("unbekannter Fehler");
        return Err(classify_error(msg));
    }
    let text = v
        .get("result")
        .and_then(|r| r.as_str())
        .ok_or_else(|| "cli_bad_output: Claude-Antwort ohne Ergebnis".to_string())?
        .to_string();
    let u = v.get("usage");
    let n = |k: &str| {
        u.and_then(|u| u.get(k))
            .and_then(|x| x.as_u64())
            .unwrap_or(0)
    };
    Ok(CliReply {
        text,
        prompt_tokens: n("input_tokens")
            + n("cache_creation_input_tokens")
            + n("cache_read_input_tokens"),
        completion_tokens: n("output_tokens"),
    })
}

/// Liest die JSONL-Ereignisse von `codex exec --json`: die Antwort ist die
/// letzte `agent_message`, Token stehen in `turn.completed`.
pub(crate) fn parse_codex(stdout: &str) -> Result<CliReply, String> {
    let mut text = None;
    let mut error = None;
    let (mut pin, mut pout) = (0, 0);
    for line in stdout.lines() {
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match ev.get("type").and_then(|t| t.as_str()) {
            Some("item.completed") => {
                let item = ev.get("item");
                if item.and_then(|i| i.get("type")).and_then(|t| t.as_str())
                    == Some("agent_message")
                {
                    text = item
                        .and_then(|i| i.get("text"))
                        .and_then(|t| t.as_str())
                        .map(str::to_string);
                }
            }
            Some("turn.completed") => {
                let u = ev.get("usage");
                let n = |k: &str| {
                    u.and_then(|u| u.get(k))
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0)
                };
                pin = n("input_tokens");
                pout = n("output_tokens");
            }
            Some("error") | Some("turn.failed") => {
                error = Some(ev.to_string());
            }
            _ => {}
        }
    }
    match (text, error) {
        (Some(text), _) => Ok(CliReply {
            text,
            prompt_tokens: pin,
            completion_tokens: pout,
        }),
        (None, Some(e)) => Err(classify_error(&e)),
        (None, None) => Err("cli_bad_output: Codex lieferte keine Antwort".to_string()),
    }
}

/// Fehler der CLI in Codes, die die Oberflaeche erklaeren kann.
pub(crate) fn classify_error(msg: &str) -> String {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("login")
        || lower.contains("not logged in")
        || lower.contains("authenticat")
        || lower.contains("401")
    {
        format!("cli_not_logged_in: {msg}")
    } else if lower.contains("not supported when using codex with a chatgpt account") {
        format!("cli_model_not_in_plan: {msg}")
    } else if lower.contains("usage limit") || lower.contains("rate limit") || lower.contains("429")
    {
        format!("cli_limit_reached: {msg}")
    } else {
        format!("cli_failed: {msg}")
    }
}

/// Ein Aufruf. Blockiert -- auf einem Blocking-Thread ausfuehren.
pub fn call(
    cli: Cli,
    binary: &Path,
    model: &str,
    system: Option<&str>,
    user: &str,
) -> Result<CliReply, String> {
    // Nur bekannte Modellnamen gelangen auf die Kommandozeile.
    if !cli.models().contains(&model) {
        return Err(format!("cli_model_not_in_plan: {model}"));
    }
    let work = std::env::temp_dir().join("local-voice-cli");
    std::fs::create_dir_all(&work).map_err(|e| format!("cli_failed: {e}"))?;
    // Codex erhaelt den Systemtext vor der Aufgabe -- `exec` kennt keinen
    // eigenen Systemprompt.
    let stdin_text = match (cli, system) {
        (Cli::Codex, Some(sys)) => format!("{sys}\n\n{user}"),
        _ => user.to_string(),
    };
    let (program, mut argv) = launcher(binary);
    argv.extend(args(cli, model, system));
    let mut cmd = Command::new(program);
    cmd.args(&argv)
        .current_dir(&work)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("cli_missing: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_text.as_bytes())
            .map_err(|e| format!("cli_failed: {e}"))?;
    }
    let output = wait_with_timeout(child, CALL_TIMEOUT)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let reply = match cli {
        Cli::Claude => parse_claude(&stdout),
        Cli::Codex => parse_codex(&stdout),
    };
    match reply {
        Ok(r) => Ok(r),
        Err(e) if e.starts_with("cli_bad_output") && !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(classify_error(stderr.lines().last().unwrap_or("")))
        }
        Err(e) => Err(e),
    }
}

/// Wie die CLI gestartet wird -- nie ueber `cmd /C`: cmd.exe wertet
/// Anfuehrungszeichen anders aus als Rust sie setzt, ein `"` mit `&` in einem
/// Argument koennte dort einen Befehl ausfuehren.
///
/// `codex` ist unter Windows ein npm-Skript (`codex.cmd`), das nur
/// `node …\node_modules\@openai\codex\bin\codex.js` aufruft. Das tun wir
/// direkt. Fehlt Node im PATH, startet Rusts `Command` das Skript selbst --
/// es maskiert die Argumente fuer Stapeldateien sicher oder lehnt ab
/// (Schutz seit Rust 1.77).
fn launcher(binary: &Path) -> (PathBuf, Vec<String>) {
    let is_script = binary
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if cfg!(windows) && is_script {
        let script = binary.parent().map(|d| {
            d.join("node_modules")
                .join("@openai")
                .join("codex")
                .join("bin")
                .join("codex.js")
        });
        if let (Some(script), Some(node)) = (script.filter(|s| s.is_file()), find_node()) {
            return (node, vec![script.to_string_lossy().into_owned()]);
        }
    }
    (binary.to_path_buf(), Vec::new())
}

fn find_node() -> Option<PathBuf> {
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    use std::io::Read;
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut o = Vec::new();
        let mut e = Vec::new();
        if let Some(s) = out.as_mut() {
            let _ = s.read_to_end(&mut o);
        }
        if let Some(s) = err.as_mut() {
            let _ = s.read_to_end(&mut e);
        }
        (o, e)
    });
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("cli_timeout: Die CLI hat nicht rechtzeitig geantwortet".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(format!("cli_failed: {e}")),
        }
    };
    let (stdout, stderr) = reader
        .join()
        .map_err(|_| "cli_failed: Ausgabe unlesbar".to_string())?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_json_reply_is_parsed_with_tokens() {
        let out = r#"{"type":"result","is_error":false,"result":"Release am Freitag.","usage":{"input_tokens":2,"cache_creation_input_tokens":780,"cache_read_input_tokens":0,"output_tokens":12}}"#;
        let r = parse_claude(out).unwrap();
        assert_eq!(r.text, "Release am Freitag.");
        assert_eq!((r.prompt_tokens, r.completion_tokens), (782, 12));
        let err = parse_claude(r#"{"is_error":true,"result":"Not logged in · Please run /login"}"#)
            .unwrap_err();
        assert!(err.starts_with("cli_not_logged_in"), "{err}");
    }

    /// Die Ereignisfolge aus dem Spike vom 06.10.2026.
    #[test]
    fn codex_events_yield_the_last_agent_message() {
        let out = concat!(
            r#"{"type":"thread.started","thread_id":"x"}"#,
            "\n",
            r#"{"type":"item.completed","item":{"type":"reasoning","text":"denke"}}"#,
            "\n",
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"Release am Freitag; Anna repariert die Tests."}}"#,
            "\n",
            r#"{"type":"turn.completed","usage":{"input_tokens":20744,"output_tokens":18}}"#,
            "\n",
        );
        let r = parse_codex(out).unwrap();
        assert_eq!(r.text, "Release am Freitag; Anna repariert die Tests.");
        assert_eq!((r.prompt_tokens, r.completion_tokens), (20744, 18));
        let refused = r#"{"type":"error","message":"The 'gpt-6' model is not supported when using Codex with a ChatGPT account."}"#;
        assert!(parse_codex(refused)
            .unwrap_err()
            .starts_with("cli_model_not_in_plan"));
    }

    #[test]
    fn calls_are_isolated_from_the_users_configuration() {
        let a = args(Cli::Claude, "sonnet", Some("System"));
        for flag in [
            "--setting-sources",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--tools",
            "--no-session-persistence",
        ] {
            assert!(a.iter().any(|x| x == flag), "{flag} fehlt");
        }
        // --bare liest den Abo-Login nicht.
        assert!(!a.iter().any(|x| x == "--bare"));
        let c = args(Cli::Codex, "gpt-6-astra", None);
        assert!(c
            .windows(2)
            .any(|w| w[0] == "-c" && w[1] == "mcp_servers={}"));
        assert!(c
            .windows(2)
            .any(|w| w[0] == "--sandbox" && w[1] == "read-only"));
    }

    /// Gegen die echten CLIs mit dem Login des Nutzers (nur von Hand):
    /// `cargo test --lib real_cli_calls -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_cli_calls() {
        for (cli, model) in [(Cli::Claude, "sonnet"), (Cli::Codex, "gpt-6-astra")] {
            let bin = locate(cli).expect("CLI installiert");
            let t = std::time::Instant::now();
            let r = call(
                cli,
                &bin,
                model,
                Some("Du schreibst knappe deutsche Protokolle. Antworte nur mit dem Ergebnis."),
                "Fasse in einem Satz zusammen: Das Release wird auf Freitag verschoben, Anna repariert die Tests.",
            );
            println!("{cli:?} {} in {:?}: {r:?}", bin.display(), t.elapsed());
            let r = r.expect("Antwort");
            assert!(r.text.contains("Freitag"), "{}", r.text);
            assert!(r.prompt_tokens > 0);
        }
    }

    /// Sicherheitsbefund 06.10.2026: kein Start ueber `cmd /C`, keine fremden
    /// Modellnamen auf der Kommandozeile.
    #[test]
    fn never_launches_through_cmd_and_rejects_unknown_models() {
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("codex.cmd");
        std::fs::write(&shim, b"@echo off").unwrap();
        let (program, args) = launcher(&shim);
        assert_ne!(program.file_name().and_then(|n| n.to_str()), Some("cmd"));
        assert!(!args.iter().any(|a| a.eq_ignore_ascii_case("/C")));

        // Mit codex.js daneben: node + Skript (sofern Node im PATH liegt).
        let js = dir.path().join("node_modules/@openai/codex/bin");
        std::fs::create_dir_all(&js).unwrap();
        std::fs::write(js.join("codex.js"), b"").unwrap();
        let (program, args) = launcher(&shim);
        if find_node().is_some() {
            assert!(
                program
                    .to_string_lossy()
                    .to_lowercase()
                    .ends_with("node.exe")
                    || program.ends_with("node")
            );
            assert!(args[0].ends_with("codex.js"));
        }

        let err = call(Cli::Codex, &shim, "gpt-6 & calc", None, "x").unwrap_err();
        assert!(err.starts_with("cli_model_not_in_plan"), "{err}");
    }

    #[test]
    fn templates_are_recognised_by_their_address() {
        assert_eq!(Cli::from_base_url("cli://claude"), Some(Cli::Claude));
        assert_eq!(Cli::from_base_url("cli://codex/"), Some(Cli::Codex));
        assert_eq!(Cli::from_base_url("https://api.openai.com/v1"), None);
    }
}
