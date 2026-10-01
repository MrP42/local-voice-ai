//! `local-voice-ai.exe ctl ...`: das Kommandozeilenwerkzeug der Agentenbruecke (A7).
//!
//! ```text
//! local-voice-ai.exe ctl status [--json]                 laeuft die App? (ohne Token moeglich)
//! local-voice-ai.exe ctl tools  [--json]                 welche Werkzeuge darf dieser Zugang benutzen?
//! local-voice-ai.exe ctl call <werkzeug> [--args JSON|@datei] [--approval ID] [--json]
//! local-voice-ai.exe ctl approval <ID> [--json]          Stand einer Freigabe
//! local-voice-ai.exe ctl workflow list [--json]          Abläufe der Automationen (Werkzeug list_workflows)
//! local-voice-ai.exe ctl workflow run <ID> [--live] [--vars JSON|@datei] [--request-id ID] [--json]
//!                                                        Ablauf starten, Standard Trockenlauf (run_workflow)
//! local-voice-ai.exe ctl workflow get <RUN-ID> [--json]  Laufprotokoll (get_run)
//! ```
//!
//! Das Token kommt aus der Umgebungsvariable `LVA_AGENT_TOKEN` oder aus `--token-file DATEI`
//! (nie aus einem Argument: Argumente stehen in der Prozessliste). Ausgabe: Text oder mit
//! `--json` ein JSON-Objekt `{"ok", "exit", "result" | "error"}`; mit `--out DATEI` zusaetzlich in
//! eine Datei (die Release-EXE ist eine Fensteranwendung: ihre Standardausgabe ist nur bei
//! Umleitung nutzbar, eine Datei geht immer).
//!
//! # Exit-Codes
//! | Code | Bedeutung |
//! |---|---|
//! | 0 | ausgefuehrt / App laeuft / Freigabe erteilt |
//! | 1 | Fehler (Eingabe, Werkzeug fehlgeschlagen, unbekanntes Werkzeug, Zeitueberschreitung) |
//! | 2 | App nicht erreichbar oder Verbindung abgebrochen (laeuft nicht, beendet sich, ausgelastet) |
//! | 3 | nicht erlaubt: Werkzeug „aus“ oder Freigabe abgelehnt |
//! | 4 | Anmeldung fehlgeschlagen: Token fehlt, ungueltig oder zurueckgezogen |
//! | 5 | wartet auf die Freigabe des Nutzers (`approval_id` steht in der Ausgabe) |

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use clap::{Args, Subcommand};
use serde_json::{json, Value};

use super::client::{Client, ClientError};
use super::pipe;
use super::protocol::code;

/// Umgebungsvariable mit dem Token des Zugangs.
pub const TOKEN_ENV: &str = "LVA_AGENT_TOKEN";

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_NOT_RUNNING: i32 = 2;
pub const EXIT_DENIED: i32 = 3;
pub const EXIT_AUTH: i32 = 4;
pub const EXIT_PENDING: i32 = 5;

/// Gesamtfrist eines Aufrufs: die Wartezeit der App auf eine Freigabe (30 s) plus Reserve.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Args, Debug, Clone)]
pub struct CtlArgs {
    /// Ausgabe als JSON-Objekt.
    #[arg(long, global = true)]
    pub json: bool,

    /// Ausgabe zusaetzlich in diese Datei schreiben.
    #[arg(long, global = true, value_name = "DATEI")]
    pub out: Option<PathBuf>,

    /// Datei mit dem Token des Zugangs (sonst Umgebungsvariable LVA_AGENT_TOKEN).
    #[arg(long, global = true, value_name = "DATEI")]
    pub token_file: Option<PathBuf>,

    #[command(subcommand)]
    pub verb: Verb,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Verb {
    /// Laeuft die App? Mit Token zusaetzlich: wer bin ich, offene Freigaben, sichtbare Werkzeuge.
    Status,
    /// Die Werkzeuge, die dieser Zugang jetzt benutzen darf.
    Tools,
    /// Ein Werkzeug aufrufen.
    Call {
        /// Name des Werkzeugs (siehe `tools`).
        tool: String,
        /// Argumente als JSON-Objekt, oder `@datei.json`.
        #[arg(long, value_name = "JSON")]
        args: Option<String>,
        /// Kennung einer erteilten Freigabe: fuehrt den frueher angefragten Aufruf aus.
        #[arg(long, value_name = "ID")]
        approval: Option<String>,
    },
    /// Stand einer Freigabe abfragen.
    Approval {
        /// Kennung aus der Antwort `pending`.
        id: String,
    },
    /// Automationen (B8): Abläufe auflisten, starten, Laufprotokoll lesen.
    Workflow {
        #[command(subcommand)]
        action: WorkflowVerb,
    },
}

/// `ctl workflow ...`: dieselben Rechte, Freigaben und Exit-Codes wie `ctl call` (die Werkzeuge
/// `list_workflows`, `run_workflow`, `get_run`).
#[derive(Subcommand, Debug, Clone)]
pub enum WorkflowVerb {
    /// Die Abläufe mit Auslöser, Schaltzustand und letztem Lauf.
    List,
    /// Einen Ablauf starten. Ohne `--live` ein Trockenlauf (nichts wird ausgeführt).
    Run {
        /// Kennung des Ablaufs (siehe `workflow list`).
        id: String,
        /// Echter Lauf: nur bei eingeschaltetem, scharf geschaltetem Ablauf und mit dem Recht
        /// `workflow.run`; sonst Fehler, es entsteht kein Lauf.
        #[arg(long)]
        live: bool,
        /// Werte für die deklarierten Variablen als JSON-Objekt, oder `@datei.json`.
        #[arg(long, value_name = "JSON")]
        vars: Option<String>,
        /// Eigene Kennung: dieselbe Kennung startet keinen zweiten Lauf.
        #[arg(long, value_name = "ID")]
        request_id: Option<String>,
        /// Kennung einer erteilten Freigabe (Recht „fragen“): führt den früher angefragten Aufruf aus.
        #[arg(long, value_name = "ID")]
        approval: Option<String>,
    },
    /// Das Protokoll eines Laufs.
    Get {
        /// Kennung des Laufs (aus `workflow run`).
        run_id: String,
    },
}

impl WorkflowVerb {
    /// Werkzeug, Argumente und Freigabe, auf die dieser Befehl abgebildet wird.
    fn as_call(&self) -> Result<(&'static str, Value, Option<String>), String> {
        Ok(match self {
            WorkflowVerb::List => ("list_workflows", json!({}), None),
            WorkflowVerb::Get { run_id } => ("get_run", json!({ "run_id": run_id }), None),
            WorkflowVerb::Run {
                id,
                live,
                vars,
                request_id,
                approval,
            } => {
                let mut args = json!({ "workflow_id": id });
                if *live {
                    args["live"] = json!(true);
                }
                if let Some(v) = vars {
                    args["vars"] = parse_args(v)?;
                }
                if let Some(r) = request_id {
                    args["request_id"] = json!(r);
                }
                ("run_workflow", args, approval.clone())
            }
        })
    }
}

/// Alles, was `execute` von aussen braucht (in Tests ersetzbar).
#[derive(Debug, Clone)]
pub struct CtlEnv {
    pub pipe_name: Result<String, String>,
    /// `Ok(None)`: kein Token angegeben. `Err`: Quelle angegeben, aber nicht lesbar.
    pub token: Result<Option<String>, String>,
    pub deadline: Duration,
}

impl CtlEnv {
    pub fn from_process(args: &CtlArgs) -> Self {
        Self {
            pipe_name: pipe::effective_pipe_name().map_err(|e| e.to_string()),
            token: read_token(args.token_file.as_deref(), std::env::var(TOKEN_ENV).ok()),
            deadline: DEFAULT_DEADLINE,
        }
    }
}

/// Token aus Datei (Vorrang) oder Umgebung; Leerraum und BOM werden entfernt.
pub fn read_token(file: Option<&Path>, env: Option<String>) -> Result<Option<String>, String> {
    let clean = |s: &str| {
        let t = s.trim_start_matches('\u{feff}').lines().next().unwrap_or("").trim().to_string();
        (!t.is_empty()).then_some(t)
    };
    if let Some(path) = file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Die Token-Datei lässt sich nicht lesen: {e}"))?;
        return clean(&text)
            .map(Some)
            .ok_or_else(|| "Die Token-Datei ist leer.".to_string());
    }
    Ok(env.as_deref().and_then(clean))
}

/// Ergebnis eines Aufrufs: Exit-Code, JSON-Form und Text.
#[derive(Debug, Clone)]
pub struct CtlResult {
    pub exit: i32,
    pub json: Value,
    pub text: String,
    /// Ausgabe fuer den Fehlerkanal (Fehlertexte).
    pub stderr: String,
}

impl CtlResult {
    fn ok(result: Value, text: String) -> Self {
        Self {
            exit: EXIT_OK,
            json: json!({ "ok": true, "exit": EXIT_OK, "result": result }),
            text,
            stderr: String::new(),
        }
    }

    fn with_exit(exit: i32, result: Value, text: String) -> Self {
        Self {
            exit,
            json: json!({ "ok": exit == EXIT_OK, "exit": exit, "result": result }),
            text,
            stderr: String::new(),
        }
    }

    fn failure(exit: i32, error_code: &str, message: &str) -> Self {
        Self {
            exit,
            json: json!({ "ok": false, "exit": exit, "error": { "code": error_code, "message": message } }),
            text: String::new(),
            stderr: format!("Fehler ({error_code}): {message}"),
        }
    }
}

/// Exit-Code zu einem Fehlercode des Servers.
pub fn exit_for_remote(error_code: &str) -> i32 {
    match error_code {
        code::TOKEN_INVALID | code::TOKEN_REVOKED | code::UNAUTHENTICATED => EXIT_AUTH,
        code::TOOL_OFF | code::DENIED | code::APPROVAL_DENIED => EXIT_DENIED,
        code::SHUTTING_DOWN | code::TOO_MANY_CONNECTIONS => EXIT_NOT_RUNNING,
        _ => EXIT_ERROR,
    }
}

fn from_error(e: &ClientError) -> CtlResult {
    match e {
        ClientError::NotRunning(m) | ClientError::UntrustedServer(m) | ClientError::Transport(m) => {
            CtlResult::failure(EXIT_NOT_RUNNING, "not_running", m)
        }
        ClientError::Protocol(m) => CtlResult::failure(EXIT_ERROR, "protocol", m),
        ClientError::Remote { code, message, .. } => {
            CtlResult::failure(exit_for_remote(code), code, message)
        }
    }
}

const NO_TOKEN: &str = "Kein Token: Umgebungsvariable LVA_AGENT_TOKEN setzen oder --token-file angeben.";

/// Fuehrt einen Aufruf aus, mit Gesamtfrist.
pub fn execute(args: &CtlArgs, env: &CtlEnv) -> CtlResult {
    let (args, env2) = (args.clone(), env.clone());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(|| run_verb(&args, &env2));
        let _ = tx.send(r);
    });
    match rx.recv_timeout(env.deadline) {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => CtlResult::failure(EXIT_ERROR, "internal", "Interner Fehler in ctl."),
        Err(_) => CtlResult::failure(
            EXIT_ERROR,
            "timeout",
            "Zeitüberschreitung: Local Voice AI hat nicht rechtzeitig geantwortet.",
        ),
    }
}

fn run_verb(args: &CtlArgs, env: &CtlEnv) -> CtlResult {
    let token = match &env.token {
        Ok(t) => t.clone(),
        Err(m) => return CtlResult::failure(EXIT_ERROR, "token_file", m),
    };
    let name = match &env.pipe_name {
        Ok(n) => n.clone(),
        Err(m) => return CtlResult::failure(EXIT_ERROR, "pipe_name", m),
    };
    // Eingaben pruefen, bevor verbunden wird.
    let call_args = match &args.verb {
        Verb::Call { args: Some(a), .. } => match parse_args(a) {
            Ok(v) => Some(v),
            Err(m) => return CtlResult::failure(EXIT_ERROR, "bad_args", &m),
        },
        _ => None,
    };
    let workflow_call = match &args.verb {
        Verb::Workflow { action } => match action.as_call() {
            Ok(c) => Some(c),
            Err(m) => return CtlResult::failure(EXIT_ERROR, "bad_args", &m),
        },
        _ => None,
    };
    if !matches!(args.verb, Verb::Status) && token.is_none() {
        return CtlResult::failure(EXIT_AUTH, "no_token", NO_TOKEN);
    }

    let mut client = match Client::connect(&name) {
        Ok(c) => c,
        Err(e) => return from_error(&e),
    };
    if let Some(t) = token.as_deref() {
        if let Err(e) = client.hello(Some(t)) {
            return from_error(&e);
        }
    }
    match &args.verb {
        Verb::Status => match client.request("status", json!({})) {
            Ok(v) => {
                let text = status_text(&v);
                CtlResult::ok(v, text)
            }
            Err(e) => from_error(&e),
        },
        Verb::Tools => match client.request("tools/list", json!({})) {
            Ok(v) => {
                let text = tools_text(&v);
                CtlResult::ok(v, text)
            }
            Err(e) => from_error(&e),
        },
        Verb::Call { tool, approval, .. } => call_tool(
            &mut client,
            tool,
            call_args.unwrap_or_else(|| json!({})),
            approval.as_deref(),
        ),
        Verb::Workflow { .. } => {
            let (tool, wf_args, approval) = workflow_call.expect("vorab gebildet");
            call_tool(&mut client, tool, wf_args, approval.as_deref())
        }
        Verb::Approval { id } => match client.request("approval/status", json!({ "approval_id": id })) {
            Ok(v) => {
                let exit = match v["state"].as_str() {
                    Some("approved") => EXIT_OK,
                    Some("pending") => EXIT_PENDING,
                    Some("denied") => EXIT_DENIED,
                    _ => EXIT_ERROR,
                };
                let text = format!(
                    "{}: {}",
                    v["state"].as_str().unwrap_or("?"),
                    v["hint"].as_str().unwrap_or("")
                );
                CtlResult::with_exit(exit, v, text)
            }
            Err(e) => from_error(&e),
        },
    }
}

/// Ruft ein Werkzeug und bildet die Antwort auf Exit-Code und Text ab (`call` und `workflow`).
fn call_tool<R: BufRead, W: Write>(
    client: &mut Client<R, W>,
    tool: &str,
    args: Value,
    approval: Option<&str>,
) -> CtlResult {
    let mut params = json!({ "name": tool, "arguments": args });
    if let Some(a) = approval {
        params["approval_id"] = json!(a);
    }
    match client.request("tools/call", params) {
        Ok(v) => match v["status"].as_str() {
            Some("done") => {
                let text = serde_json::to_string_pretty(&v["result"]).unwrap_or_default();
                CtlResult::ok(v, text)
            }
            Some("pending") => {
                let text = format!(
                    "Freigabe ausstehend: {}\nDie App wartet auf die Entscheidung des Nutzers. Stand: ctl approval {}; nach „approved“: denselben Aufruf mit --approval {} wiederholen.",
                    v["approval_id"].as_str().unwrap_or("?"),
                    v["approval_id"].as_str().unwrap_or("?"),
                    v["approval_id"].as_str().unwrap_or("?"),
                );
                CtlResult::with_exit(EXIT_PENDING, v, text)
            }
            _ => CtlResult::failure(EXIT_ERROR, "protocol", "Unerwartete Antwort."),
        },
        Err(e) => from_error(&e),
    }
}

fn parse_args(raw: &str) -> Result<Value, String> {
    let text = match raw.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| format!("Die Argumentdatei lässt sich nicht lesen: {e}"))?,
        None => raw.to_string(),
    };
    match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
        Ok(v @ Value::Object(_)) => Ok(v),
        Ok(_) => Err("Die Argumente müssen ein JSON-Objekt sein.".to_string()),
        Err(e) => Err(format!("Die Argumente sind kein gültiges JSON: {e}")),
    }
}

fn status_text(v: &Value) -> String {
    let mut s = format!(
        "Local Voice AI läuft (Version {}, Protokoll {}).",
        v["version"].as_str().unwrap_or("?"),
        v["protocol"]
    );
    if v["authenticated"] == json!(true) {
        s.push_str(&format!(
            "\nAngemeldet als „{}“; offene Freigaben: {}; Werkzeuge: {}.",
            v["client"]["label"].as_str().unwrap_or("?"),
            v["pending_approvals"],
            v["tools"]
        ));
    } else {
        s.push_str("\nNicht angemeldet (kein Token).");
    }
    s
}

fn tools_text(v: &Value) -> String {
    let Some(tools) = v["tools"].as_array() else {
        return String::new();
    };
    tools
        .iter()
        .map(|t| {
            format!(
                "{:<22} {:<6} {}",
                t["name"].as_str().unwrap_or("?"),
                t["mode"].as_str().unwrap_or("?"),
                t["title"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Einstieg aus `main.rs`: fuehrt aus, gibt aus, liefert den Exit-Code.
pub fn run(args: CtlArgs) -> i32 {
    let env = CtlEnv::from_process(&args);
    let result = execute(&args, &env);
    finish(&args, &result)
}

/// Gibt das Ergebnis aus (Standardausgabe, Fehlerkanal, `--out`) und liefert den Exit-Code.
pub fn finish(args: &CtlArgs, result: &CtlResult) -> i32 {
    let stdout = if args.json {
        result.json.to_string()
    } else {
        result.text.clone()
    };
    if !stdout.is_empty() {
        println!("{stdout}");
    }
    if !result.stderr.is_empty() {
        eprintln!("{}", result.stderr);
    }
    if let Some(path) = &args.out {
        let body = if args.json {
            result.json.to_string()
        } else if result.stderr.is_empty() {
            result.text.clone()
        } else {
            result.stderr.clone()
        };
        if let Err(e) = std::fs::write(path, body) {
            eprintln!("Die Ausgabedatei lässt sich nicht schreiben: {e}");
        }
    }
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    result.exit
}

#[cfg(test)]
mod tests;
