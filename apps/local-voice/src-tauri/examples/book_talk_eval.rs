//! Eval-Tor des Buch-Gesprächs (lva#324, Plan §3.9): Fragensatz durch die
//! Pipeline, Bericht und `eval-pass`-Eintrag. Ohne Tauri, ohne App-Daten.
//!
//! ```text
//! cargo run --example book_talk_eval -- --provider fake --out out/eval
//! cargo run --example book_talk_eval -- --provider ollama --model gemma3:4b --out out/eval
//! cargo run --example book_talk_eval -- --provider openai --base-url https://api.openai.com/v1 \
//!     --model gpt-5-mini --api-key-env OPENAI_API_KEY --out out/eval
//! cargo run --example book_talk_eval -- --provider local_server --url http://127.0.0.1:8080 \
//!     --model <id> --out out/eval
//! cargo run --example book_talk_eval -- --provider claude_cli --model sonnet --out out/eval
//! ```
//!
//! Standard sind Buch und Fragen unter `tooling/fixtures/buch-gespraech/eval/`.
//! Anleitung und Ablage der Pass-Datei: dort `README.md`.

use std::path::PathBuf;
use std::time::Duration;

use local_voice_ai_lib::book_talk_eval::{
    build_model, eval_pass_entry, merge_pass, report_hash, report_json, run_eval, write_outputs,
    Backend, EvalConfig, EvalInput, DEFAULT_CONTEXT_TOKENS, PROVIDERS,
};

const USAGE: &str = "\
Aufruf: book_talk_eval --provider <fake|ollama|openai|local_server|claude_cli|codex_cli> [Optionen]

  --provider <p>        Anbieter (Pflicht)
  --model <id>          Modell-ID (Pflicht außer bei fake)
  --base-url <url>      OpenAI-kompatibler Server (ollama, openai)
  --url <url>           laufender llama-server, z. B. http://127.0.0.1:8080
                        (local_server; mit exaktem Zähler, Kennung `local`)
  --api-key-env <NAME>  Umgebungsvariable mit dem API-Schlüssel
  --provider-id <id>    Kennung im eval-pass-Eintrag (Standard: der Anbieter);
                        muss zur Verbindung in der App passen
  --book <datei>        Buchtext (Standard: eval/buch-eval.txt)
  --questions <datei>   Fragen (Standard: eval/fragen-eval.json)
  --out <ordner>        Ausgabe: bericht.md, bericht.json, eval-pass.json
  --context-tokens <n>  Kontextfenster (Standard: 8192)
  --timeout-secs <n>    Zeitrahmen je Antwort (Standard: 30, wie die App)
  --merge-into <datei>  Eintrag in diese book_talk_eval_pass.json einfügen
  --require-pass        Exit-Code 1, wenn der Lauf nicht besteht
";

#[derive(Default)]
struct Args {
    provider: String,
    model: Option<String>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    provider_id: Option<String>,
    book: Option<PathBuf>,
    questions: Option<PathBuf>,
    out: Option<PathBuf>,
    context_tokens: Option<u32>,
    timeout_secs: Option<u64>,
    merge_into: Option<PathBuf>,
    require_pass: bool,
}

fn parse(argv: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = argv.iter();
    while let Some(flag) = it.next() {
        if flag == "--require-pass" {
            a.require_pass = true;
            continue;
        }
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{flag}: Wert fehlt"))
        };
        match flag.as_str() {
            "--provider" => a.provider = value()?,
            "--model" => a.model = Some(value()?),
            "--base-url" | "--url" => a.base_url = Some(value()?),
            "--api-key-env" => a.api_key_env = Some(value()?),
            "--provider-id" => a.provider_id = Some(value()?),
            "--book" => a.book = Some(PathBuf::from(value()?)),
            "--questions" => a.questions = Some(PathBuf::from(value()?)),
            "--out" => a.out = Some(PathBuf::from(value()?)),
            "--context-tokens" => {
                a.context_tokens = Some(value()?.parse().map_err(|_| "--context-tokens: Zahl")?)
            }
            "--timeout-secs" => {
                a.timeout_secs = Some(value()?.parse().map_err(|_| "--timeout-secs: Zahl")?)
            }
            "--merge-into" => a.merge_into = Some(PathBuf::from(value()?)),
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unbekannte Option: {other}")),
        }
    }
    if !PROVIDERS.contains(&a.provider.as_str()) {
        return Err(format!(
            "--provider: eine von {} angeben",
            PROVIDERS.join(", ")
        ));
    }
    if a.provider != "fake" && a.model.as_deref().unwrap_or("").is_empty() {
        return Err("--model: Pflicht (außer bei fake)".to_string());
    }
    Ok(a)
}

/// Standardordner der Fixture: relativ zum Quellbaum, unabhängig vom
/// Arbeitsverzeichnis des Aufrufs.
fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tooling/fixtures/buch-gespraech/eval")
}

fn run(a: Args) -> Result<i32, String> {
    let dir = fixture_dir();
    let input = EvalInput::load(
        &a.book.clone().unwrap_or_else(|| dir.join("buch-eval.txt")),
        &a.questions
            .clone()
            .unwrap_or_else(|| dir.join("fragen-eval.json")),
    )?;
    let api_key = a
        .api_key_env
        .as_deref()
        .map(|name| std::env::var(name).map_err(|_| format!("Umgebungsvariable {name} fehlt")))
        .transpose()?;
    let model_id = a.model.clone().unwrap_or_else(|| "fake".to_string());
    let context_tokens = a.context_tokens.unwrap_or(DEFAULT_CONTEXT_TOKENS);
    let model = build_model(
        &a.provider,
        a.base_url.as_deref(),
        &model_id,
        api_key,
        context_tokens,
    )?;
    let default_id = if a.provider == "local_server" {
        "local"
    } else {
        &a.provider
    };
    let mut cfg = EvalConfig::new(a.provider_id.as_deref().unwrap_or(default_id), &model_id);
    cfg.context_tokens = context_tokens;
    if let Some(s) = a.timeout_secs {
        cfg.timeout = Duration::from_secs(s);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Laufzeit: {e}"))?;
    let report = runtime.block_on(run_eval(
        &input,
        match &model {
            Some(m) => Backend::Model(m.as_ref()),
            None => Backend::Fake,
        },
        &cfg,
    ));
    let hash = match &a.out {
        Some(out) => write_outputs(out, &report)?,
        None => report_hash(&report_json(&report)),
    };
    let s = &report.summary;
    println!(
        "Eval {}: {} ({} Fragen, Abdeckung {:.1} %, verworfen {:.1} %, Verstöße {})",
        report.provider,
        report.status,
        s.evaluated,
        s.coverage * 100.0,
        s.dropped_share * 100.0,
        s.spoiler_violations + s.injection_violations
    );
    for why in &report.failures {
        println!("  - {why}");
    }
    println!("report_hash {hash}");
    if let Some(path) = &a.merge_into {
        let existing = std::fs::read_to_string(path).ok();
        let merged = merge_pass(existing.as_deref(), &eval_pass_entry(&report, &hash))?;
        std::fs::write(path, merged).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("Eintrag in {} geschrieben", path.display());
    }
    Ok(if a.require_pass && report.status != "passed" {
        1
    } else {
        0
    })
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match parse(&argv).and_then(run) {
        Ok(code) => code,
        Err(e) if e.is_empty() => {
            print!("{USAGE}");
            0
        }
        Err(e) => {
            eprintln!("Fehler: {e}\n\n{USAGE}");
            2
        }
    };
    std::process::exit(code);
}
