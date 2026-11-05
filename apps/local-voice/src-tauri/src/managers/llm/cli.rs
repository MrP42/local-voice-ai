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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cli {
    Claude,
    Codex,
}

impl Cli {
    /// Vorlagenkennung der Verbindung (`claude_cli`, `codex_cli`).
    pub fn from_kind(kind: &str) -> Option<Self> {
        match kind {
            "claude_cli" => Some(Self::Claude),
            "codex_cli" => Some(Self::Codex),
            _ => None,
        }
    }

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
    /// Das Modell, das die CLI laut ihrer Ausgabe tatsaechlich benutzt hat
    /// (Claude: Schluessel von `modelUsage`; Codex: Feld `model` eines
    /// Ereignisses). `None`, wenn die Ausgabe es nicht nennt.
    pub model: Option<String>,
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
    // Zielordner des offiziellen Codex-Installers (`open_setup`).
    if let Some(l) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        dirs.push(l.join("Programs").join("OpenAI").join("Codex").join("bin"));
    }
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

/// Zustand einer CLI auf diesem Rechner, fuer die Verbindungsseite.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct CliStatus {
    pub installed: bool,
    /// `None`: die CLI hat keine lesbare Antwort gegeben.
    pub logged_in: Option<bool>,
}

/// Wie lange die Statusabfrage dauern darf.
const STATUS_TIMEOUT: Duration = Duration::from_secs(15);

fn status_args(cli: Cli) -> &'static [&'static str] {
    match cli {
        Cli::Claude => &["auth", "status", "--json"],
        Cli::Codex => &["login", "status"],
    }
}

fn login_args(cli: Cli) -> &'static [&'static str] {
    match cli {
        Cli::Claude => &["auth", "login"],
        Cli::Codex => &["login"],
    }
}

/// Liest nur "angemeldet ja/nein" aus der Antwort der CLI. Konto und E-Mail,
/// die `claude auth status` mitliefert, werden weder gespeichert noch gezeigt.
fn parse_status(cli: Cli, stdout: &str, stderr: &str) -> Option<bool> {
    match cli {
        Cli::Claude => serde_json::from_str::<serde_json::Value>(stdout)
            .ok()?
            .get("loggedIn")?
            .as_bool(),
        Cli::Codex => {
            let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
            if text.contains("not logged in") {
                Some(false)
            } else if text.contains("logged in") {
                Some(true)
            } else {
                None
            }
        }
    }
}

/// Ist die CLI da, und ist man angemeldet? Blockiert -- auf einem
/// Blocking-Thread ausfuehren.
pub fn status(cli: Cli) -> CliStatus {
    let Some(binary) = locate(cli) else {
        return CliStatus {
            installed: false,
            logged_in: None,
        };
    };
    let (program, mut argv) = launcher(&binary);
    argv.extend(status_args(cli).iter().map(|s| s.to_string()));
    let mut cmd = Command::new(program);
    cmd.args(&argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let logged_in = cmd
        .spawn()
        .ok()
        .and_then(|child| run_with_timeout(child, None, STATUS_TIMEOUT).ok())
        .and_then(|out| {
            parse_status(
                cli,
                &String::from_utf8_lossy(&out.stdout),
                &String::from_utf8_lossy(&out.stderr),
            )
        });
    CliStatus {
        installed: true,
        logged_in,
    }
}

/// Befehlszeile fuer `cmd`: scheitert die Anmeldung, bleibt das Fenster mit
/// der Meldung der CLI offen, statt sich sofort zu schliessen.
#[cfg_attr(not(windows), allow(dead_code))]
fn login_command_line(program: &Path, argv: &[String]) -> String {
    let quoted: Vec<String> = std::iter::once(program.to_string_lossy().into_owned())
        .chain(argv.iter().cloned())
        .map(|a| format!("\"{a}\""))
        .collect();
    format!("/S /C \"{} || pause\"", quoted.join(" "))
}

/// PowerShell-Skript fuer [`open_setup`]: der dokumentierte Installer des
/// Anbieters (keine Adminrechte), danach dessen eigene Anmeldung im Browser.
/// Die App liefert das Programm nicht mit und veraendert es nicht; so bleibt
/// es bei der Anmeldung am unveraenderten Programm, wie Anthropic sie erlaubt
/// (code.claude.com/docs/en/legal-and-compliance, geprueft 05.11.2026).
fn setup_script(cli: Cli) -> &'static str {
    match cli {
        Cli::Claude => {
            "$ErrorActionPreference = 'Stop'; \
             irm https://claude.ai/install.ps1 | iex; \
             & \"$env:USERPROFILE\\.local\\bin\\claude.exe\" auth login; \
             exit $LASTEXITCODE"
        }
        // Ohne die Variable fragt der Installer am Ende "Start Codex now?".
        Cli::Codex => {
            "$ErrorActionPreference = 'Stop'; \
             $env:CODEX_NON_INTERACTIVE = '1'; \
             irm https://chatgpt.com/codex/install.ps1 | iex; \
             & \"$env:LOCALAPPDATA\\Programs\\OpenAI\\Codex\\bin\\codex.exe\" login; \
             exit $LASTEXITCODE"
        }
    }
}

/// Befehlszeile fuer `cmd`. Das Skript geht als `-EncodedCommand`, damit es
/// keine zwei Quotierungsebenen durchlaufen muss.
#[cfg_attr(not(windows), allow(dead_code))]
fn setup_command_line(cli: Cli) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = setup_script(cli)
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    format!(
        "/S /C \"powershell.exe -NoProfile -ExecutionPolicy Bypass -EncodedCommand {encoded} || pause\""
    )
}

/// Laedt die CLI ueber den offiziellen Installer und startet ihre Anmeldung,
/// beides in einem eigenen Konsolenfenster.
#[cfg(windows)]
pub fn open_setup(cli: Cli) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    Command::new("cmd")
        .raw_arg(setup_command_line(cli))
        .creation_flags(0x0000_0010) // CREATE_NEW_CONSOLE
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cli_failed: {e}"))
}

#[cfg(not(windows))]
pub fn open_setup(cli: Cli) -> Result<(), String> {
    Err(format!(
        "cli_failed: Bitte im Terminal installieren: {}",
        match cli {
            Cli::Claude => "curl -fsSL https://claude.ai/install.sh | bash",
            Cli::Codex => "curl -fsSL https://chatgpt.com/codex/install.sh | sh",
        }
    ))
}

/// Oeffnet die Anmeldung der CLI in einem eigenen Konsolenfenster. Sie laeuft
/// ganz in der CLI und im Browser; die App sieht davon nichts.
#[cfg(windows)]
pub fn open_login(cli: Cli) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let binary = locate(cli)
        .ok_or_else(|| "cli_missing: Die CLI des Anbieters ist nicht installiert".to_string())?;
    let (program, mut argv) = launcher(&binary);
    argv.extend(login_args(cli).iter().map(|s| s.to_string()));
    Command::new("cmd")
        .raw_arg(login_command_line(&program, &argv))
        .creation_flags(0x0000_0010) // CREATE_NEW_CONSOLE
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cli_failed: {e}"))
}

#[cfg(not(windows))]
pub fn open_login(cli: Cli) -> Result<(), String> {
    Err(format!(
        "cli_failed: Bitte im Terminal anmelden: {} {}",
        if cli == Cli::Claude {
            "claude"
        } else {
            "codex"
        },
        login_args(cli).join(" ")
    ))
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
        // Haertung gegen eingeschleuste Anweisungen im Transkript: `codex exec`
        // kennt keinen Schalter "keine Werkzeuge", also schalten wir jedes
        // bekannte Werkzeug einzeln ab -- Shell (`shell_tool`, `unified_exec`),
        // Bildbetrachter (liest lokale Dateien), Websuche, Browser/Computer,
        // Plugins/Apps, MCP. Dazu Read-only-Sandbox und ein frischer, leerer
        // Arbeitsordner je Aufruf (`call_with_timeout`).
        // `mcp_servers={}` allein reicht NICHT: Codex fuehrt Tabellen rekursiv
        // zusammen, eine leere Tabelle loescht die Server der Nutzerkonfiguration
        // nicht. Deshalb setzt `call_with_timeout` zusaetzlich
        // [`codex_isolation_args`] ein (`--ignore-user-config` oder jeden
        // Server einzeln aus).
        // Was das NICHT leistet: ein Werkzeug, das eine kuenftige Codex-Version
        // neu einfuehrt, ist hier nicht abgeschaltet; die Sandbox verbietet
        // Schreiben, nicht Lesen; der leere Ordner haelt nur relative Pfade
        // fern. Geprueft gegen codex-cli 0.162.1 (`codex features list`).
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
            "-c",
            "features.shell_tool=false",
            "-c",
            "features.unified_exec=false",
            "-c",
            "features.view_image=false",
            "-c",
            "features.browser_use=false",
            "-c",
            "features.in_app_browser=false",
            "-c",
            "features.computer_use=false",
            "-c",
            "features.image_generation=false",
            "-c",
            "features.plugins=false",
            "-c",
            "features.apps=false",
            "-c",
            "web_search=\"disabled\"",
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
        model: claude_model_of(&v),
    })
}

/// Das tatsaechliche Modell aus `modelUsage` (genau ein Eintrag), sonst aus `model`.
pub(crate) fn claude_model_of(v: &serde_json::Value) -> Option<String> {
    if let Some(map) = v.get("modelUsage").and_then(|m| m.as_object()) {
        // Mehrere Modelle (z. B. ein Hilfsmodell): das mit den meisten Ausgabetoken.
        return map
            .iter()
            .max_by_key(|(_, u)| u.get("outputTokens").and_then(|t| t.as_u64()).unwrap_or(0))
            .map(|(k, _)| k.clone());
    }
    v.get("model").and_then(|m| m.as_str()).map(str::to_string)
}

/// Liest die JSONL-Ereignisse von `codex exec --json`: die Antwort ist die
/// letzte `agent_message`, Token stehen in `turn.completed`.
pub(crate) fn parse_codex(stdout: &str) -> Result<CliReply, String> {
    let mut text = None;
    let mut error = None;
    let mut failed = None;
    let (mut pin, mut pout) = (0, 0);
    let mut model: Option<String> = None;
    for line in stdout.lines() {
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if model.is_none() {
            model = ev
                .get("model")
                .or_else(|| ev.get("item").and_then(|i| i.get("model")))
                .and_then(|m| m.as_str())
                .filter(|m| !m.is_empty())
                .map(str::to_string);
        }
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
            Some("error") => {
                error = Some(ev.to_string());
            }
            // Ein gescheiterter Zug ist nie ein Erfolg, auch wenn vorher schon
            // eine (Teil-)Antwort kam.
            Some("turn.failed") => {
                failed = Some(ev.to_string());
            }
            _ => {}
        }
    }
    if let Some(e) = failed {
        return Err(classify_error(&e));
    }
    match (text, error) {
        (Some(text), _) => Ok(CliReply {
            text,
            prompt_tokens: pin,
            completion_tokens: pout,
            model,
        }),
        (None, Some(e)) => Err(classify_error(&e)),
        (None, None) => Err("cli_bad_output: Codex lieferte keine Antwort".to_string()),
    }
}

/// Schalter, ohne die der Aufruf nicht isoliert ist (Spec 11.7 R4). Fehlt einer
/// in der Hilfe der installierten CLI, wird das Modell nicht fuer das Relay angeboten.
pub fn required_flags(cli: Cli) -> &'static [&'static str] {
    match cli {
        Cli::Claude => &[
            "--output-format",
            "--tools",
            "--no-session-persistence",
            "--setting-sources",
            "--strict-mcp-config",
            "--mcp-config",
            "--disable-slash-commands",
            "--system-prompt",
        ],
        Cli::Codex => &["--json", "--sandbox", "--ephemeral", "--skip-git-repo-check"],
    }
}

/// Fehlende Pflichtschalter in einem Hilfetext.
pub fn missing_flags(cli: Cli, help: &str) -> Vec<&'static str> {
    required_flags(cli)
        .iter()
        .copied()
        .filter(|f| !help.contains(f))
        .collect()
}

/// Umgebungsvariablen, die die CLI auf kostenpflichtige API-Abrechnung stellen.
pub fn api_key_vars(cli: Cli) -> &'static [&'static str] {
    match cli {
        Cli::Claude => &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"],
        Cli::Codex => &["OPENAI_API_KEY", "CODEX_API_KEY"],
    }
}

/// Ist fuer die CLI ein API-Schluessel in der Umgebung gesetzt? (Spec 11.7 R6)
pub fn api_key_in_env(cli: Cli) -> bool {
    api_key_vars(cli)
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
}

/// Ist die CLI installiert, per Abo angemeldet, ohne API-Schluessel in der
/// Umgebung und kennt sie alle Isolationsschalter? Blockiert (startet die CLI
/// mehrfach) -- auf einem Blocking-Thread ausfuehren und das Ergebnis merken.
pub fn relay_ready(cli: Cli) -> bool {
    if api_key_in_env(cli) {
        return false;
    }
    let st = status(cli);
    if !st.installed || st.logged_in != Some(true) {
        return false;
    }
    let Some(binary) = locate(cli) else {
        return false;
    };
    let (program, mut argv) = launcher(&binary);
    match cli {
        Cli::Claude => argv.push("--help".into()),
        Cli::Codex => argv.extend(["exec".to_string(), "--help".to_string()]),
    }
    let mut cmd = Command::new(program);
    cmd.args(&argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Some(out) = cmd
        .spawn()
        .ok()
        .and_then(|child| run_with_timeout(child, None, STATUS_TIMEOUT).ok())
    else {
        return false;
    };
    let help = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    missing_flags(cli, &help).is_empty()
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
            "cli_model_not_in_plan: Dieses Modell nimmt der Server von der installierten              Codex-CLI nicht an. Meist hilft ein Update: npm i -g @openai/codex@latest, oder den offiziellen Installer erneut ausfuehren ({msg})"
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
    // Frischer, leerer Arbeitsordner je Aufruf; wird beim Verlassen geloescht.
    let work = tempfile::Builder::new()
        .prefix("local-voice-cli-")
        .tempdir()
        .map_err(|e| format!("cli_failed: {e}"))?;
    // Codex erhaelt den Systemtext vor der Aufgabe -- `exec` kennt keinen
    // eigenen Systemprompt.
    let stdin_text = match (cli, system) {
        (Cli::Codex, Some(sys)) => format!("{sys}\n\n{user}"),
        _ => user.to_string(),
    };
    let (program, mut argv) = launcher(binary);
    let prefix = argv.clone();
    argv.extend(args_with(cli, model, effort, fast, system));
    if cli == Cli::Codex {
        // Vor dem abschliessenden "-" (Prompt aus stdin).
        let at = argv.len() - 1;
        argv.splice(at..at, codex_isolation_args(&program, &prefix)?);
    }
    let mut cmd = Command::new(program);
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
    let child = cmd.spawn().map_err(|e| format!("cli_missing: {e}"))?;
    let output = run_with_timeout(child, Some(stdin_text.into_bytes()), timeout)?;
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
    child: std::process::Child,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    run_with_timeout(child, None, timeout)
}

/// Wartet auf `child`, schreibt dabei `input` nach stdin und liest stdout und
/// stderr -- alles in eigenen Threads und alles unter EINER Zeitgrenze ab
/// Start. Laeuft sie ab (auch weil ein Enkelprozess eine Pipe offen haelt),
/// wird der Prozessbaum beendet: unter Windows ueber ein Job-Objekt mit
/// KILL_ON_JOB_CLOSE (faengt auch Enkel, die nach dem Start entstehen;
/// scheitert das Anhaengen, `taskkill /T` auf die eigene Kind-PID). Ausserhalb
/// von Windows nur das Kind selbst. Ein unvollstaendig geschriebener Prompt
/// ist ein Fehler, nie ein Erfolg.
pub(crate) fn run_with_timeout(
    mut child: std::process::Child,
    input: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    use std::io::Read;
    use std::sync::mpsc;
    use std::time::Instant;

    let deadline = Instant::now() + timeout;
    // Grenze: das Job-Objekt wird erst nach dem Start zugeordnet -- was das
    // Kind davor schon startet, faengt es nicht. Ausserhalb von Windows wird
    // bei Zeitueberschreitung nur das Kind beendet, nicht seine Kinder.
    let job = crate::process_guard::ProcessGuard::attach(&child, None, 100);
    let kill_tree = |child: &mut std::process::Child,
                     job: Option<crate::process_guard::ProcessGuard>| {
        #[cfg(windows)]
        if job.is_none() && matches!(child.try_wait(), Ok(None)) {
            let _ = Command::new("taskkill")
                .args(["/PID", &child.id().to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .output();
        }
        // Drop des Job-Objekts beendet den ganzen Baum.
        drop(job);
        let _ = child.kill();
        let _ = child.wait();
    };

    fn spawn_into<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> mpsc::Receiver<T> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx
    }
    fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
        spawn_into(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    }
    // Eigener Thread: liest die CLI stdin nicht, blockierte sonst das
    // Schreiben vor der Zeitueberwachung.
    let writer = match (input, child.stdin.take()) {
        (Some(data), Some(mut stdin)) => Some(spawn_into(move || stdin.write_all(&data))),
        _ => None,
    };
    let out_rx = drain(child.stdout.take());
    let err_rx = drain(child.stderr.take());

    let timed_out = || -> Result<std::process::Output, String> {
        Err("cli_timeout: Die CLI hat nicht rechtzeitig geantwortet".to_string())
    };
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                kill_tree(&mut child, job);
                return timed_out();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                kill_tree(&mut child, job);
                return Err(format!("cli_failed: {e}"));
            }
        }
    };
    let left = || deadline.saturating_duration_since(Instant::now());
    let (Ok(stdout), Ok(stderr)) = (out_rx.recv_timeout(left()), err_rx.recv_timeout(left()))
    else {
        // Das Kind ist fertig, aber jemand haelt die Pipe offen (Enkelprozess).
        kill_tree(&mut child, job);
        return timed_out();
    };
    if let Some(w) = writer {
        match w.recv_timeout(left()) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(format!(
                    "cli_failed: Die Eingabe kam nicht vollstaendig bei der CLI an ({e})"
                ))
            }
            Err(_) => {
                kill_tree(&mut child, job);
                return timed_out();
            }
        }
    }
    drop(job);
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Haelt `codex exec` von der Konfiguration des Nutzers fern (MCP-Server,
/// Plugins, Hooks, `notify`). Bevorzugt `--ignore-user-config`: laut
/// `codex exec --help` (0.162.1) wird dann `$CODEX_HOME/config.toml` nicht
/// geladen, die Anmeldung kommt weiter aus `CODEX_HOME` -- die App liest oder
/// beruehrt `auth.json` nie. Kennt die installierte CLI den Schalter nicht,
/// wird jeder Server aus `codex mcp list --json` einzeln abgeschaltet
/// (`mcp_servers.<name>.enabled=false`). Grenze dieses Rueckfalls: er wirkt
/// nur auf MCP-Server, nicht auf Plugins/Hooks der Nutzerkonfiguration.
/// Scheitert beides, wird nicht aufgerufen (Fehler statt ungeschuetzt).
fn codex_isolation_args(program: &Path, prefix: &[String]) -> Result<Vec<String>, String> {
    static SUPPORTS: std::sync::Mutex<Vec<(PathBuf, bool)>> = std::sync::Mutex::new(Vec::new());
    let probe = |args: &[&str]| -> Result<String, String> {
        let mut cmd = Command::new(program);
        cmd.args(prefix)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = cmd.spawn().map_err(|e| format!("cli_missing: {e}"))?;
        let out = wait_with_timeout(child, Duration::from_secs(30))?;
        if !out.status.success() {
            return Err(format!("cli_failed: codex {} scheiterte", args.join(" ")));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let cached = SUPPORTS
        .lock()
        .ok()
        .and_then(|c| c.iter().find(|(p, _)| p == program).map(|(_, s)| *s));
    let supports = match cached {
        Some(s) => s,
        None => {
            let s = supports_ignore_user_config(&probe(&["exec", "--help"])?);
            if let Ok(mut c) = SUPPORTS.lock() {
                c.push((program.to_path_buf(), s));
            }
            s
        }
    };
    if supports {
        return Ok(vec!["--ignore-user-config".to_string()]);
    }
    mcp_disable_args(&probe(&["mcp", "list", "--json"])?)
}

/// Kennt `codex exec` den Schalter `--ignore-user-config`?
pub(crate) fn supports_ignore_user_config(exec_help: &str) -> bool {
    exec_help.contains("--ignore-user-config")
}

/// `-c mcp_servers.<name>.enabled=false` fuer jeden Server aus
/// `codex mcp list --json`. Nur Namen aus `[A-Za-z0-9_-]+`: ein Punkt wuerde
/// als Konfigurationspfad gelesen und traefe einen anderen Server. Jeder andere
/// Name bricht ab (kein Quoting) -- lieber kein Aufruf als ein Server, der an bleibt.
pub(crate) fn mcp_disable_args(list_json: &str) -> Result<Vec<String>, String> {
    let v: serde_json::Value = serde_json::from_str(list_json.trim())
        .map_err(|e| format!("cli_failed: MCP-Liste von Codex unlesbar ({e})"))?;
    let servers = v
        .as_array()
        .ok_or_else(|| "cli_failed: MCP-Liste von Codex unlesbar".to_string())?;
    let mut out = Vec::new();
    for s in servers {
        let name = s.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let plain = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !plain {
            return Err(format!(
                "cli_failed: MCP-Server mit unerwartetem Namen, Aufruf abgebrochen ({name:?})"
            ));
        }
        out.push("-c".to_string());
        out.push(format!("mcp_servers.{name}.enabled=false"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_login_state_is_read_from_both_clis() {
        let logged_in = r#"{"loggedIn":true,"email":"x@example.org"}"#;
        assert_eq!(parse_status(Cli::Claude, logged_in, ""), Some(true));
        assert_eq!(
            parse_status(Cli::Claude, r#"{"loggedIn":false}"#, ""),
            Some(false)
        );
        assert_eq!(parse_status(Cli::Claude, "kaputt", ""), None);
        // Codex schreibt je nach Version nach stdout oder stderr.
        assert_eq!(
            parse_status(Cli::Codex, "", "Logged in using ChatGPT"),
            Some(true)
        );
        assert_eq!(parse_status(Cli::Codex, "Not logged in", ""), Some(false));
        assert_eq!(parse_status(Cli::Codex, "", ""), None);
    }

    #[test]
    fn the_login_window_stays_open_on_failure() {
        let line = login_command_line(
            Path::new(r"C:\Program Files\claude.exe"),
            &["auth".to_string(), "login".to_string()],
        );
        assert_eq!(
            line,
            r#"/S /C ""C:\Program Files\claude.exe" "auth" "login" || pause""#
        );
    }

    #[test]
    fn setup_runs_the_official_installer_and_then_the_login() {
        // Ohne `Stop` endete ein gescheiterter Download mit Exit 0, und das
        // Fenster schloss sich ohne Meldung.
        for cli in [Cli::Claude, Cli::Codex] {
            assert!(setup_script(cli).starts_with("$ErrorActionPreference = 'Stop';"));
        }
        assert!(setup_script(Cli::Claude).contains("irm https://claude.ai/install.ps1 | iex;"));
        assert!(setup_script(Cli::Claude).contains("claude.exe\" auth login;"));
        assert!(
            setup_script(Cli::Codex).contains("irm https://chatgpt.com/codex/install.ps1 | iex;")
        );
        assert!(setup_script(Cli::Codex).contains("CODEX_NON_INTERACTIVE"));
        assert!(setup_script(Cli::Codex).contains("codex.exe\" login;"));

        use base64::Engine;
        let line = setup_command_line(Cli::Codex);
        let encoded = line
            .split("-EncodedCommand ")
            .nth(1)
            .and_then(|r| r.split(' ').next())
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(
            String::from_utf16(&units).unwrap(),
            setup_script(Cli::Codex)
        );
        assert!(line.ends_with(" || pause\""));
    }

    #[test]
    fn only_the_two_cli_kinds_are_accepted() {
        assert_eq!(Cli::from_kind("claude_cli"), Some(Cli::Claude));
        assert_eq!(Cli::from_kind("codex_cli"), Some(Cli::Codex));
        assert_eq!(Cli::from_kind("anthropic"), None);
    }

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
        let failed = concat!(
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"halb fertig"}}"#,
            "\n",
            r#"{"type":"turn.failed","error":{"message":"stream disconnected"}}"#,
            "\n",
        );
        assert!(parse_codex(failed).unwrap_err().starts_with("cli_failed"));
        let refused = r#"{"type":"error","message":"The 'gpt-6' model is not supported when using Codex with a ChatGPT account."}"#;
        assert!(parse_codex(refused)
            .unwrap_err()
            .starts_with("cli_model_not_in_plan"));
    }

    #[test]
    fn codex_isolation_prefers_ignore_user_config_else_disables_each_server() {
        assert!(supports_ignore_user_config(
            "      --ignore-user-config\n          Do not load `$CODEX_HOME/config.toml`"
        ));
        assert!(!supports_ignore_user_config("      --ephemeral\n"));
        let a = mcp_disable_args(
            r#"[{"name":"node_repl","enabled":true},{"name":"code-review","enabled":false}]"#,
        )
        .unwrap();
        assert_eq!(
            a,
            [
                "-c",
                "mcp_servers.node_repl.enabled=false",
                "-c",
                "mcp_servers.code-review.enabled=false"
            ]
        );
        assert!(mcp_disable_args("[]").unwrap().is_empty());
        assert!(mcp_disable_args(r#"[{"name":"a b=true"}]"#).is_err());
        assert!(mcp_disable_args(r#"[{"name":"a.b"}]"#).is_err());
        assert!(mcp_disable_args(r#"[{"name":""}]"#).is_err());
        assert!(mcp_disable_args("kein json").is_err());
    }

    /// Ein Enkelprozess haelt stdout offen, das Kind ist laengst fertig: die
    /// Zeitgrenze greift trotzdem, und der Baum wird beendet.
    #[cfg(windows)]
    #[test]
    fn a_grandchild_holding_the_pipe_hits_the_timeout() {
        let child = Command::new("cmd")
            .args(["/C", "start /B ping -n 60 127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let t = std::time::Instant::now();
        let err = wait_with_timeout(child, Duration::from_secs(3)).unwrap_err();
        assert!(err.starts_with("cli_timeout"), "{err}");
        assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
    }

    /// Liest die CLI den Prompt nicht vollstaendig, ist das ein Fehler.
    #[cfg(windows)]
    #[test]
    fn an_incomplete_prompt_is_an_error() {
        let child = Command::new("cmd")
            .args(["/C", "exit 0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let err = run_with_timeout(child, Some(vec![b'x'; 8 << 20]), Duration::from_secs(30))
            .unwrap_err();
        assert!(err.starts_with("cli_failed"), "{err}");
    }

    /// Viel stderr bei leerem stdout: nacheinander gelesen haengt das bis zur
    /// Zeitgrenze (volle stderr-Pipe), parallel gelesen nicht.
    #[cfg(windows)]
    #[test]
    fn full_stderr_does_not_block_the_reader() {
        let child = Command::new("cmd")
            .args([
                "/C",
                "for /L %i in (1,1,6000) do @echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx 1>&2",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let out = wait_with_timeout(child, Duration::from_secs(60)).unwrap();
        assert!(out.stdout.is_empty());
        assert!(out.stderr.len() > 200_000, "{}", out.stderr.len());
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
        for off in [
            "features.shell_tool=false",
            "features.unified_exec=false",
            "features.view_image=false",
            "web_search=\"disabled\"",
        ] {
            assert!(
                c.windows(2).any(|w| w[0] == "-c" && w[1] == off),
                "{off} fehlt"
            );
        }
        assert!(c.iter().any(|x| x == "--skip-git-repo-check"));
        assert_eq!(c.last().map(String::as_str), Some("-"));
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
