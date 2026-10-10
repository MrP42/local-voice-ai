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

/// Claude-Modelle: Aliase (immer das neueste) und volle Namen (feste Version).
/// Alle mit `claude -p --model … --effort low` geprueft (06.10.2026).
pub const CLAUDE_MODELS: &[&str] = &[
    "fable",
    "opus",
    "sonnet",
    "haiku",
    "claude-fable-5-1",
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-haiku-4-5-20251001",
];
/// Claude-Modelle, die ein Probelauf als verfuegbar erkannt hat und die noch
/// nicht in [`CLAUDE_MODELS`] stehen (`managers::llm::updates`). Der Speicher
/// liegt hier, weil `Cli::models()` keinen Zugriff auf die Einstellungen hat.
static CLAUDE_EXTRA: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Setzt die zusaetzlich erkannten Claude-Modelle (ersetzt die bisherigen).
pub fn set_claude_extra_models(ids: Vec<String>) {
    if let Ok(mut extra) = CLAUDE_EXTRA.lock() {
        *extra = ids.into_iter().filter(|i| valid_model_name(i)).collect();
    }
}

/// Effort-Stufen von `claude --effort`.
pub const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Codex: die Liste kommt aus dem Modellkatalog, den Codex selbst laedt
/// (`~/.codex/models_cache.json`, nur `visibility: list`), dazu das Modell aus
/// `~/.codex/config.toml`. Ohne Katalog diese Rueckfallliste (Stand 06.10.2026).
/// Neue Modelle (z. B. `gpt-6.1-sol`) nimmt der Server nur von einer aktuellen
/// Codex-CLI an; eine alte meldet „not supported when using Codex with a
/// ChatGPT account“.
pub const CODEX_FALLBACK: &[&str] = &[
    "gpt-6.1-sol",
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
];
pub const CODEX_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Alle Effort-Stufen, die eine CLI je annimmt (Pruefung vor dem Aufruf).
const KNOWN_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"];

/// Ein Modell der CLI mit den Effort-Stufen, die es annimmt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliModel {
    pub id: String,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
    /// Bietet den Fast-Modus an (Codex-Katalog: `additional_speed_tiers`).
    pub fast: bool,
}

/// Endung der gespeicherten Angabe fuer den Fast-Modus: `gpt-6.1-sol@high~fast`.
pub const FAST_SUFFIX: &str = "~fast";

/// Modellname, wie er auf die Kommandozeile darf: Buchstaben, Ziffern, `.`, `-`,
/// `_`, hoechstens 64 Zeichen. Es gibt keine Shell dazwischen; die Regel haelt
/// trotzdem jedes Sonderzeichen fern.
pub fn valid_model_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

pub fn valid_effort(s: &str) -> bool {
    KNOWN_EFFORTS.contains(&s)
}

/// Modell und Effort aus der gespeicherten Angabe `modell@effort` (eine
/// Endung `~fast` faellt dabei weg, siehe [`split_fast`]).
pub fn split_spec(spec: &str) -> (&str, Option<&str>) {
    let spec = split_fast(spec).0;
    match spec.split_once('@') {
        Some((m, e)) if !e.is_empty() => (m, Some(e)),
        Some((m, _)) => (m, None),
        None => (spec, None),
    }
}

/// Angabe ohne Fast-Endung und ob sie gesetzt war.
pub fn split_fast(spec: &str) -> (&str, bool) {
    match spec.strip_suffix(FAST_SUFFIX) {
        Some(rest) => (rest, true),
        None => (spec, false),
    }
}

fn codex_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_HOME") {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(|h| PathBuf::from(h).join(".codex"))
}

/// Modelle aus dem Codex-Katalog (Datei ohne Zugangsdaten; der Kontoteil wird
/// nicht gelesen).
pub(crate) fn codex_catalog(home: &Path) -> Vec<CliModel> {
    let Ok(text) = std::fs::read_to_string(home.join("models_cache.json")) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    v.get("models")
        .and_then(|m| m.as_array())
        .into_iter()
        .flatten()
        .filter(|m| m.get("visibility").and_then(|x| x.as_str()) == Some("list"))
        .filter_map(|m| {
            let id = m.get("slug").and_then(|x| x.as_str())?;
            if !valid_model_name(id) {
                return None;
            }
            let efforts: Vec<String> = m
                .get("supported_reasoning_levels")
                .and_then(|x| x.as_array())
                .into_iter()
                .flatten()
                .filter_map(|l| l.get("effort").and_then(|e| e.as_str()))
                .filter(|e| valid_effort(e))
                .map(str::to_string)
                .collect();
            let fast = m
                .get("additional_speed_tiers")
                .and_then(|x| x.as_array())
                .is_some_and(|tiers| tiers.iter().any(|t| t.as_str() == Some("fast")));
            Some(CliModel {
                id: id.to_string(),
                efforts,
                fast,
                default_effort: m
                    .get("default_reasoning_level")
                    .and_then(|x| x.as_str())
                    .filter(|e| valid_effort(e))
                    .map(str::to_string),
            })
        })
        .collect()
}

/// Das Standardmodell aus `config.toml` (Zeile `model = "…"`).
pub(crate) fn codex_configured_model(home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join("config.toml")).ok()?;
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("model")?.trim_start();
        let value = rest.strip_prefix('=')?.trim().trim_matches('"');
        valid_model_name(value).then(|| value.to_string())
    })
}

fn standard(ids: &[&str], efforts: &[&str]) -> Vec<CliModel> {
    ids.iter()
        .map(|id| CliModel {
            id: id.to_string(),
            efforts: efforts.iter().map(|e| e.to_string()).collect(),
            default_effort: None,
            fast: false,
        })
        .collect()
}

/// Codex-Liste: Katalog (sonst Rueckfallliste) plus das eingestellte Modell.
pub(crate) fn codex_models_in(home: Option<&Path>) -> Vec<CliModel> {
    let mut models = home.map(codex_catalog).unwrap_or_default();
    if models.is_empty() {
        models = standard(CODEX_FALLBACK, CODEX_EFFORTS);
    }
    if let Some(configured) = home.and_then(codex_configured_model) {
        if !models.iter().any(|m| m.id == configured) {
            models.insert(0, standard(&[configured.as_str()], CODEX_EFFORTS).remove(0));
        }
    }
    models
}

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

    /// Die Modelle, die diese CLI anbietet (Codex: aus ihrem Katalog).
    pub fn models(self) -> Vec<CliModel> {
        match self {
            Self::Claude => {
                let mut models = standard(CLAUDE_MODELS, CLAUDE_EFFORTS);
                let extra = CLAUDE_EXTRA.lock().map(|e| e.clone()).unwrap_or_default();
                for id in extra {
                    if !models.iter().any(|m| m.id == id) {
                        models.extend(standard(&[id.as_str()], CLAUDE_EFFORTS));
                    }
                }
                models
            }
            Self::Codex => codex_models_in(codex_home().as_deref()),
        }
    }

    /// Effort-Stufen eines Modells; leer, wenn das Modell unbekannt ist.
    pub fn efforts(self, model: &str) -> Vec<String> {
        self.models()
            .into_iter()
            .find(|m| m.id == model)
            .map(|m| m.efforts)
            .unwrap_or_default()
    }

    /// Bietet das Modell den Fast-Modus an? (nur Codex laut Katalog)
    pub fn offers_fast(self, model: &str) -> bool {
        self.models().into_iter().any(|m| m.id == model && m.fast)
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
pub(crate) fn args(
    cli: Cli,
    model: &str,
    effort: Option<&str>,
    system: Option<&str>,
) -> Vec<String> {
    args_with(cli, model, effort, false, system)
}

/// Wie [`args`], dazu der Fast-Modus (nur Codex: `service_tier = "fast"`).
pub(crate) fn args_with(
    cli: Cli,
    model: &str,
    effort: Option<&str>,
    fast: bool,
    system: Option<&str>,
) -> Vec<String> {
    let mut a = args_without_effort(cli, model, system);
    if fast && cli == Cli::Codex {
        let at = a.len() - 1;
        a.insert(at, "service_tier=\"fast\"".into());
        a.insert(at, "-c".into());
    }
    if let Some(e) = effort {
        match cli {
            Cli::Claude => {
                a.push("--effort".into());
                a.push(e.to_string());
            }
            // Vor dem abschliessenden "-" (Prompt aus stdin).
            Cli::Codex => {
                let at = a.len() - 1;
                a.insert(at, format!("model_reasoning_effort=\"{e}\""));
                a.insert(at, "-c".into());
            }
        }
    }
    a
}

fn args_without_effort(cli: Cli, model: &str, system: Option<&str>) -> Vec<String> {
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
        // Meist ist die Codex-CLI zu alt fuer das Modell (06.10.2026: 0.153.3
        // lehnt gpt-6.1-sol ab, 0.160.1 nimmt es an).
        format!(
            "cli_model_not_in_plan: Dieses Modell nimmt der Server von der installierten              Codex-CLI nicht an. Meist hilft ein Update: npm i -g @openai/codex@latest ({msg})"
        )
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
    call_with_timeout(cli, binary, model, system, user, CALL_TIMEOUT)
}

/// Wie [`call`] mit eigener Zeitgrenze (der Probelauf fuer neue Modelle
/// wartet hoechstens Sekunden, nicht Minuten).
pub fn call_with_timeout(
    cli: Cli,
    binary: &Path,
    model: &str,
    system: Option<&str>,
    user: &str,
    timeout: Duration,
) -> Result<CliReply, String> {
    // `modell@effort~fast`; auf die Kommandozeile kommt nur, was die Regeln erlaubt.
    let fast = split_fast(model).1;
    let (model, effort) = split_spec(model);
    if !valid_model_name(model) {
        return Err(format!("cli_model_not_in_plan: {model}"));
    }
    if let Some(e) = effort.filter(|e| !valid_effort(e)) {
        return Err(format!("cli_failed: unbekannte Effort-Stufe {e}"));
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
    argv.extend(args_with(cli, model, effort, fast, system));
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
    let output = wait_with_timeout(child, timeout)?;
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
pub(crate) fn launcher(binary: &Path) -> (PathBuf, Vec<String>) {
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

pub(crate) fn find_node() -> Option<PathBuf> {
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

pub(crate) fn wait_with_timeout(
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
        let a = args(Cli::Claude, "sonnet", None, Some("System"));
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
        let c = args(Cli::Codex, "gpt-6-astra", None, None);
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
        for (cli, model) in [(Cli::Claude, "sonnet@low"), (Cli::Codex, "gpt-6-astra@low")] {
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
    fn effort_goes_to_the_right_flag_and_the_prompt_stays_last() {
        let a = args(Cli::Claude, "opus", Some("high"), None);
        assert!(a.windows(2).any(|w| w[0] == "--effort" && w[1] == "high"));
        let c = args(Cli::Codex, "gpt-6.1-sol", Some("xhigh"), None);
        assert!(c
            .windows(2)
            .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=\"xhigh\""));
        assert_eq!(c.last().map(String::as_str), Some("-"));
        assert_eq!(
            split_spec("gpt-6.1-sol@high"),
            ("gpt-6.1-sol", Some("high"))
        );
        assert_eq!(split_spec("sonnet"), ("sonnet", None));
        assert_eq!(
            split_spec("gpt-6.1-sol@high~fast"),
            ("gpt-6.1-sol", Some("high"))
        );
        assert_eq!(split_spec("gpt-6-luna~fast"), ("gpt-6-luna", None));
        assert_eq!(split_fast("gpt-6-luna@low~fast"), ("gpt-6-luna@low", true));
        assert_eq!(split_fast("gpt-6-luna@low"), ("gpt-6-luna@low", false));
    }

    #[test]
    fn names_and_efforts_are_checked_before_the_command_line() {
        for ok in ["gpt-6.1-sol", "claude-haiku-4-5-20251001", "fable"] {
            assert!(valid_model_name(ok), "{ok}");
        }
        for bad in ["", "gpt-6 & calc", "a\"b", "x;y", "../x", &"a".repeat(65)] {
            assert!(!valid_model_name(bad), "{bad}");
        }
        assert!(valid_effort("ultra") && !valid_effort("high; rm"));
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("claude.exe");
        std::fs::write(&shim, b"").unwrap();
        let err = call(Cli::Claude, &shim, "opus@turbo", None, "x").unwrap_err();
        assert!(err.contains("Effort"), "{err}");
    }

    #[test]
    fn the_codex_list_comes_from_its_catalog_plus_the_configured_model() {
        let dir = tempfile::tempdir().unwrap();
        // Ohne Katalog: Rueckfallliste.
        let fallback = codex_models_in(Some(dir.path()));
        assert!(fallback.iter().any(|m| m.id == "gpt-6.1-sol"));
        std::fs::write(
            dir.path().join("models_cache.json"),
            r#"{"identity":{"geheim":"nie gelesen"},"models":[
              {"slug":"gpt-6-astra","visibility":"list","default_reasoning_level":"medium",
               "additional_speed_tiers":["fast"],
               "supported_reasoning_levels":[{"effort":"low"},{"effort":"ultra"},{"effort":"x;y"}]},
              {"slug":"gpt-reserve","visibility":"hide","supported_reasoning_levels":[]},
              {"slug":"bad name","visibility":"list","supported_reasoning_levels":[]}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "model = \"gpt-6.1-sol\"\nmodel_reasoning_effort = \"medium\"\n",
        )
        .unwrap();
        let models = codex_models_in(Some(dir.path()));
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["gpt-6.1-sol", "gpt-6-astra"]);
        assert_eq!(models[1].efforts, ["low", "ultra"]);
        assert_eq!(models[1].default_effort.as_deref(), Some("medium"));
        assert!(models[1].fast, "Katalog bietet Fast an");
        assert!(
            !models[0].fast,
            "eingestelltes Modell ohne Katalogeintrag: kein Fast"
        );
    }

    #[test]
    fn fast_mode_sets_the_service_tier_only_for_codex() {
        let c = args_with(Cli::Codex, "gpt-6.1-sol", Some("high"), true, None);
        assert!(c
            .windows(2)
            .any(|w| w[0] == "-c" && w[1] == "service_tier=\"fast\""));
        assert_eq!(c.last().map(String::as_str), Some("-"));
        let slow = args_with(Cli::Codex, "gpt-6.1-sol", None, false, None);
        assert!(!slow.iter().any(|a| a.contains("service_tier")));
        let claude = args_with(Cli::Claude, "sonnet", None, true, None);
        assert!(!claude.iter().any(|a| a.contains("service_tier")));
    }

    #[test]
    fn templates_are_recognised_by_their_address() {
        assert_eq!(Cli::from_base_url("cli://claude"), Some(Cli::Claude));
        assert_eq!(Cli::from_base_url("cli://codex/"), Some(Cli::Codex));
        assert_eq!(Cli::from_base_url("https://api.openai.com/v1"), None);
    }
}
