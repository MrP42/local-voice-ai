//! Abo-CLIs (Claude Code, Codex) im Buch-Gespräch: nur als gekennzeichnete
//! VORSCHAU (lva#334, Entscheidung des Prüfers).
//!
//! Für ein Abo gibt es keinen Vorab-Beweis des Tokenverbrauchs: die CLI schickt
//! eigene, versteckte Anweisungen und kompaktiert still, wenn der Kontext
//! knapp wird. Zulässig ist deshalb nur ein empirisches Budget mit Sicherheits-
//! zuschlag und eine Prüfung NACH dem Aufruf, die im Zweifel verwirft:
//!
//! * Freigabe: eigener Schalter `book_talk_subscription_preview` und ein
//!   gebundenes Tupel ([`PreviewBinding`]: Anbieter, Modell, CLI-Version,
//!   SHA-256 des Programms). Weicht das Tupel ab, gilt die Freigabe nicht mehr.
//! * Vorab ([`fits`]): `UTF-8-Bytes(System + Prompt) + Overhead(CLI) * 1,2 +
//!   max. Ausgabe <= min(Kontext, Kompaktierungsschwelle)`.
//! * Nachträglich ([`verify`]): Nutzungsangabe vorhanden und plausibel,
//!   tatsächliches Modell passt, Eingabe + Ausgabe <= Kontext, kein Hinweis auf
//!   Kompaktierung oder Abschneiden. Sonst wird der Text nie an Oberfläche oder
//!   Sprache gegeben (Aufrufer: `cli::call_with_options`).
//!
//! Kein Frage-, Antwort- oder Buchtext im Log: [`audit_line`] kennt nur Tupel,
//! Modellnamen, Methode und Status.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;

use super::cli::{launcher, split_spec, Cli, CliReply, CLAUDE_MODELS};

/// Budgetmethode in Bericht und Audit.
pub const BUDGET_METHOD: &str = "empirical";

/// Kontextfenster der Claude-Modelle in Token (Vorgabe des Prüfers).
pub const CLAUDE_CONTEXT_TOKENS: u32 = 200_000;

/// Versteckte Eingabe der CLI (eigener Systemprompt, Werkzeugbeschreibungen,
/// Rahmentext) in Token. Empirisch, Messreihe nötig: bewusst hoch gewählt.
pub const CLAUDE_OVERHEAD_TOKENS: u64 = 30_000;
/// Wie [`CLAUDE_OVERHEAD_TOKENS`], für Codex. Empirisch, Messreihe nötig.
pub const CODEX_OVERHEAD_TOKENS: u64 = 20_000;
/// Sicherheitszuschlag auf den Overhead, in Prozent (1,2-fach).
pub const OVERHEAD_FACTOR_PERCENT: u64 = 120;
/// Ab diesem Anteil des Kontexts kompaktieren die CLIs; darüber wird nicht
/// geplant (0,8 × Kontext). Empirisch, Messreihe nötig.
pub const COMPACTION_PERCENT: u64 = 80;
/// Reserve für Denk-Token (Reasoning) über der sichtbaren Antwort. Empirisch,
/// Messreihe nötig.
pub const REASONING_RESERVE_TOKENS: u32 = 4_000;
/// Eine Nutzungsangabe mit weniger Eingabetoken als `Bytes / MIN_BYTES_PER_TOKEN`
/// ist unplausibel (kein Tokenizer packt mehr als ein paar Bytes in ein Token).
const MIN_BYTES_PER_TOKEN: usize = 16;

pub fn provider_of(cli: Cli) -> &'static str {
    match cli {
        Cli::Claude => "claude_cli",
        Cli::Codex => "codex_cli",
    }
}

/// Overhead der CLI in Token (ohne Zuschlag).
pub fn cli_overhead(cli: Cli) -> u64 {
    match cli {
        Cli::Claude => CLAUDE_OVERHEAD_TOKENS,
        Cli::Codex => CODEX_OVERHEAD_TOKENS,
    }
}

/// Belegter Kontext des Modells. Claude: 200000 (nie mehr, auch wenn die
/// Konfiguration mehr nennt). Codex: nur ein in der Konfiguration eingetragener
/// Wert; ohne ihn ist der Kontext nicht belegt (`None`, kein Aufruf).
pub fn context_tokens(cli: Cli, configured: Option<u32>) -> Option<u32> {
    match cli {
        Cli::Claude => {
            Some(configured.map_or(CLAUDE_CONTEXT_TOKENS, |c| c.clamp(1, CLAUDE_CONTEXT_TOKENS)))
        }
        Cli::Codex => configured.filter(|c| *c > 0),
    }
}

/// Planungsgrenze: `min(Kontext, Kompaktierungsschwelle)`.
pub fn limit_tokens(context: u32) -> u64 {
    let threshold = u64::from(context) * COMPACTION_PERCENT / 100;
    threshold.min(u64::from(context))
}

/// Eingabe in Token-Einheiten: UTF-8-Bytes (eine Obergrenze der Token) plus
/// Overhead mit Zuschlag.
pub fn input_estimate(cli: Cli, text_bytes: usize) -> u64 {
    text_bytes as u64 + cli_overhead(cli) * OVERHEAD_FACTOR_PERCENT / 100
}

/// Maximale Ausgabe einer Antwort mit Reserve für Denk-Token.
pub fn max_output(answer_tokens: u32) -> u32 {
    answer_tokens.saturating_add(REASONING_RESERVE_TOKENS)
}

/// Die Vorab-Prüfung: passt der Aufruf in `min(Kontext, Schwelle)`?
pub fn fits(cli: Cli, context: u32, text_bytes: usize, max_output_tokens: u32) -> bool {
    input_estimate(cli, text_bytes) + u64::from(max_output_tokens) <= limit_tokens(context)
}

// ---------------------------------------------------------------- Bindung --

/// Das Tupel, an das die Freigabe gebunden ist. Ändert sich eines der Felder,
/// ist die Freigabe ungültig und muss erneut bestätigt werden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PreviewBinding {
    /// `claude_cli` oder `codex_cli`.
    pub provider: String,
    /// Modell, wie in der Konfiguration eingetragen (inkl. Effort-Endung).
    pub model_id: String,
    /// Konkrete Modell-ID statt Alias, soweit bekannt (Claude-Aliase aus der
    /// Liste der App; Codex-Modelle sind schon konkret).
    #[serde(default)]
    pub concrete_model: Option<String>,
    /// Ausgabe von `<cli> --version` (erste Zeile).
    pub cli_version: String,
    /// SHA-256 (hex) des Programms.
    pub binary_sha256: String,
}

/// Konkrete Modell-ID zu einem Claude-Alias (`sonnet` -> `claude-sonnet-5-5`),
/// nach der Liste der App ([`CLAUDE_MODELS`]: erst Aliase, dann volle Namen in
/// gleicher Reihenfolge). Sonst die Angabe selbst; `None`, wenn nicht bekannt.
pub fn concrete_model(cli: Cli, model_id: &str) -> Option<String> {
    let base = split_spec(model_id).0;
    match cli {
        Cli::Codex => Some(base.to_string()),
        Cli::Claude => {
            let half = CLAUDE_MODELS.len() / 2;
            match CLAUDE_MODELS.iter().position(|m| *m == base) {
                Some(i) if i < half => Some(CLAUDE_MODELS[i + half].to_string()),
                Some(_) => Some(base.to_string()),
                None => None,
            }
        }
    }
}

/// Version und Prüfsumme der installierten CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub version: String,
    pub sha256: String,
}

pub fn binding_for(cli: Cli, model_id: &str, identity: &Identity) -> PreviewBinding {
    PreviewBinding {
        provider: provider_of(cli).to_string(),
        model_id: model_id.to_string(),
        concrete_model: concrete_model(cli, model_id),
        cli_version: identity.version.clone(),
        binary_sha256: identity.sha256.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingState {
    /// Gespeichertes Tupel stimmt mit dem aktuellen überein.
    Valid,
    /// Für dieses Modell wurde nie bestätigt.
    Missing,
    /// Version, Programm oder Modell weichen ab: erneut bestätigen.
    Changed,
}

pub fn binding_state(stored: &[PreviewBinding], current: &PreviewBinding) -> BindingState {
    match stored
        .iter()
        .find(|b| b.provider == current.provider && b.model_id == current.model_id)
    {
        None => BindingState::Missing,
        Some(b) if b == current => BindingState::Valid,
        Some(_) => BindingState::Changed,
    }
}

/// Wirkt die Vorschau für diesen Aufruf?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PreviewState {
    /// Schalter aus.
    Off,
    /// Schalter an, Tupel gültig.
    Active,
    /// Schalter an, aber das Tupel fehlt oder weicht ab (oder die CLI ist nicht
    /// zu prüfen): der Schalter wirkt nicht, die Oberfläche fragt erneut.
    Reconfirm,
}

pub fn gate(
    enabled: bool,
    stored: &[PreviewBinding],
    current: Option<&PreviewBinding>,
) -> PreviewState {
    if !enabled {
        return PreviewState::Off;
    }
    match current.map(|c| binding_state(stored, c)) {
        Some(BindingState::Valid) => PreviewState::Active,
        _ => PreviewState::Reconfirm,
    }
}

// ------------------------------------------------- Programm-Identität (IO) --

/// Hash und Version, gemerkt je (Pfad, Änderungszeit, Größe): das Hashen einer
/// großen Programmdatei und der Start von `--version` passieren nicht je Frage.
/// Grenze: ein Austausch mit gleicher Größe und Zeit fiele nicht auf.
static IDENTITY_CACHE: Mutex<Vec<((PathBuf, Option<SystemTime>, u64), Identity)>> =
    Mutex::new(Vec::new());

/// SHA-256 (hex) einer Datei, gelesen in Blöcken.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Erste nichtleere Zeile einer `--version`-Ausgabe, auf 120 Zeichen gekürzt.
pub fn parse_version(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(120).collect())
}

/// Die Datei, die gehasht wird: das Programm selbst, bei einem npm-Skript
/// (Windows, `codex.cmd`) das JavaScript, das es startet.
fn hashed_file(binary: &Path) -> PathBuf {
    let (program, argv) = launcher(binary);
    match argv.first() {
        Some(script) => PathBuf::from(script),
        None => program,
    }
}

/// Version und Prüfsumme der CLI. Blockiert (startet die CLI, liest die
/// Datei): auf einem Blocking-Thread ausführen.
pub fn identity(binary: &Path) -> Result<Identity, String> {
    let file = hashed_file(binary);
    let meta = std::fs::metadata(&file).map_err(|e| format!("cli_missing: {e}"))?;
    let key = (file.clone(), meta.modified().ok(), meta.len());
    if let Some(hit) = IDENTITY_CACHE
        .lock()
        .ok()
        .and_then(|c| c.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()))
    {
        return Ok(hit);
    }
    let sha256 = sha256_file(&file).map_err(|e| format!("cli_failed: {e}"))?;
    let (program, mut argv) = launcher(binary);
    argv.push("--version".into());
    let mut cmd = std::process::Command::new(program);
    cmd.args(&argv)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let child = cmd.spawn().map_err(|e| format!("cli_missing: {e}"))?;
    let out = super::cli::wait_with_timeout(child, std::time::Duration::from_secs(15))?;
    let version = parse_version(&String::from_utf8_lossy(&out.stdout))
        .or_else(|| parse_version(&String::from_utf8_lossy(&out.stderr)))
        .filter(|_| out.status.success())
        .ok_or_else(|| "cli_failed: Version der CLI nicht lesbar".to_string())?;
    let id = Identity { version, sha256 };
    if let Ok(mut c) = IDENTITY_CACHE.lock() {
        c.retain(|(k, _)| k.0 != key.0);
        c.push((key, id.clone()));
    }
    Ok(id)
}

/// Aktuelles Tupel für ein Modell: sucht die CLI und liest ihre Identität.
/// Blockiert.
pub fn current_binding(cli: Cli, model_id: &str) -> Result<PreviewBinding, String> {
    let binary = super::cli::locate(cli)
        .ok_or_else(|| "cli_missing: Die CLI des Anbieters ist nicht installiert".to_string())?;
    Ok(binding_for(cli, model_id, &identity(&binary)?))
}

// ------------------------------------------------------------- Aufrufprüfung --

/// Alles, was ein Aufruf in der Vorschau einhalten muss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewCheck {
    pub binding: PreviewBinding,
    /// Belegter Kontext des Modells.
    pub context_tokens: u32,
    /// Höchste Ausgabe inklusive Reserve ([`max_output`]).
    pub max_output_tokens: u32,
}

/// Ablehnung: `code` ist der Code des Buch-Gesprächs (`budget` oder
/// `llm_failed`), `reason` ein fester Text für das Audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejection {
    pub code: &'static str,
    pub reason: &'static str,
}

/// Präfix des Fehlertexts einer Ablehnung: `cli_preview_rejected:<code>:<grund>`.
pub const REJECTED_PREFIX: &str = "cli_preview_rejected";

impl Rejection {
    pub fn error(&self) -> String {
        format!("{REJECTED_PREFIX}:{}:{}", self.code, self.reason)
    }
}

/// Code aus einem Fehlertext einer Ablehnung (`budget` oder `llm_failed`).
pub fn rejection_code(err: &str) -> Option<&'static str> {
    let rest = err.strip_prefix(REJECTED_PREFIX)?.strip_prefix(':')?;
    if rest.starts_with("budget") {
        Some("budget")
    } else {
        Some("llm_failed")
    }
}

const BUDGET: &str = "budget";
const LLM_FAILED: &str = "llm_failed";

/// Vorab: Passt der Aufruf in das empirische Budget?
pub fn precheck(cli: Cli, check: &PreviewCheck, text_bytes: usize) -> Result<(), Rejection> {
    if fits(
        cli,
        check.context_tokens,
        text_bytes,
        check.max_output_tokens,
    ) {
        return Ok(());
    }
    Err(Rejection {
        code: BUDGET,
        reason: "budget_precheck",
    })
}

/// Passt das tatsächliche Modell zum angefragten? `claude-sonnet-5-5` erfüllt
/// `sonnet` (Alias) und sich selbst; Effort und `~fast` zählen nicht.
pub fn model_matches(requested: &str, concrete: Option<&str>, actual: &str) -> bool {
    let actual = actual.trim().to_lowercase();
    let base = split_spec(requested).0.trim().to_lowercase();
    if actual.is_empty() || base.is_empty() {
        return false;
    }
    let concrete = concrete.map(str::to_lowercase);
    actual == base || concrete.as_deref() == Some(actual.as_str()) || actual.contains(&base)
}

/// Nachträglich: darf die Antwort verwendet werden? Fail-closed.
pub fn verify(
    check: &PreviewCheck,
    requested_model: &str,
    reply: &CliReply,
    input_bytes: usize,
) -> Result<(), Rejection> {
    let reject = |code, reason| Err(Rejection { code, reason });
    if reply.prompt_tokens == 0 || reply.completion_tokens == 0 {
        return reject(LLM_FAILED, "usage_missing");
    }
    let plausible_min = (input_bytes / MIN_BYTES_PER_TOKEN) as u64;
    if reply.prompt_tokens < plausible_min {
        return reject(LLM_FAILED, "usage_implausible");
    }
    if let Some(actual) = reply.model.as_deref() {
        if !model_matches(
            requested_model,
            check.binding.concrete_model.as_deref(),
            actual,
        ) {
            return reject(LLM_FAILED, "model_mismatch");
        }
    }
    if reply.compacted {
        return reject(BUDGET, "compaction");
    }
    if reply.truncated {
        return reject(BUDGET, "truncated");
    }
    if reply.prompt_tokens + reply.completion_tokens > u64::from(check.context_tokens) {
        return reject(BUDGET, "context_exceeded");
    }
    Ok(())
}

/// Hinweise in einem CLI-Ereignis (Claude: das Ergebnisobjekt, Codex: je
/// JSONL-Zeile) auf Kompaktierung oder abgeschnittene Ausgabe.
pub fn hints(ev: &serde_json::Value) -> Hints {
    let mut h = Hints::default();
    scan(ev, &mut h);
    h
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hints {
    pub compacted: bool,
    pub truncated: bool,
}

fn truthy(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
    }
}

fn scan(v: &serde_json::Value, h: &mut Hints) {
    const TRUNCATING: &[&str] = &[
        "max_tokens",
        "max_output_tokens",
        "length",
        "model_context_window_exceeded",
    ];
    match v {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let k = key.to_ascii_lowercase();
                // Der Antworttext selbst wird nie durchsucht.
                if k == "result" || k == "text" || k == "message" {
                    continue;
                }
                if k.contains("compact") && truthy(val) {
                    h.compacted = true;
                }
                if k == "incomplete_details" && truthy(val) {
                    h.truncated = true;
                }
                if let Some(s) = val.as_str() {
                    let s = s.to_ascii_lowercase();
                    if matches!(k.as_str(), "type" | "subtype" | "event" | "kind" | "status") {
                        if s.contains("compact") {
                            h.compacted = true;
                        }
                        if s.contains("incomplete") {
                            h.truncated = true;
                        }
                    }
                    if matches!(k.as_str(), "stop_reason" | "finish_reason" | "reason")
                        && TRUNCATING.contains(&s.as_str())
                    {
                        h.truncated = true;
                    }
                }
                scan(val, h);
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|i| scan(i, h)),
        _ => {}
    }
}

// ------------------------------------------------------------------ Audit --

/// Audit-Zeile ohne Inhalte: Tupel, Modell angefragt und tatsächlich,
/// Budgetmethode, Status. Gleiches Muster wie die übrigen `book_talk`-Zeilen.
pub fn audit_line(
    binding: &PreviewBinding,
    requested_model: &str,
    actual_model: Option<&str>,
    status: &str,
) -> String {
    format!(
        "book_talk_preview: provider={} model_requested={} model_actual={} cli_version={} cli_sha256={} method={} status={}",
        binding.provider,
        clean(requested_model),
        actual_model.map_or_else(|| "unknown".to_string(), clean),
        clean(&binding.cli_version),
        binding.binary_sha256,
        BUDGET_METHOD,
        status
    )
}

/// Nur Zeichen, die in einer Modell- oder Versionsangabe vorkommen (kein
/// Zeilenumbruch, keine Leerzeichen): ein fremder Wert kann das Log nicht
/// umbrechen.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric()
                || matches!(c, '.' | '-' | '_' | '@' | '~' | '/' | '(' | ')')
            {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding() -> PreviewBinding {
        binding_for(
            Cli::Claude,
            "sonnet",
            &Identity {
                version: "2.1.318 (Claude Code)".into(),
                sha256: "ab".repeat(32),
            },
        )
    }

    fn check(context: u32) -> PreviewCheck {
        PreviewCheck {
            binding: binding(),
            context_tokens: context,
            max_output_tokens: max_output(600),
        }
    }

    fn reply(input: u64, output: u64, model: Option<&str>) -> CliReply {
        CliReply {
            text: "synthetisch".into(),
            prompt_tokens: input,
            completion_tokens: output,
            model: model.map(str::to_string),
            compacted: false,
            truncated: false,
        }
    }

    #[test]
    fn the_budget_formula_is_bytes_plus_scaled_overhead_plus_output() {
        // Claude: 30000 * 1,2 = 36000; Schwelle 0,8 * 200000 = 160000.
        assert_eq!(input_estimate(Cli::Claude, 1_000), 37_000);
        assert_eq!(limit_tokens(200_000), 160_000);
        let room = 160_000 - 36_000 - u64::from(max_output(600));
        assert!(fits(Cli::Claude, 200_000, room as usize, max_output(600)));
        assert!(!fits(
            Cli::Claude,
            200_000,
            room as usize + 1,
            max_output(600)
        ));
        // Codex rechnet mit 20000 * 1,2 = 24000.
        assert_eq!(input_estimate(Cli::Codex, 0), 24_000);
        assert_eq!(limit_tokens(32_000), 25_600);
        assert!(!fits(Cli::Codex, 32_000, 0, max_output(600)));
        assert!(fits(Cli::Codex, 64_000, 0, max_output(600)));
    }

    #[test]
    fn the_reasoning_reserve_is_part_of_the_output() {
        assert_eq!(max_output(600), 600 + REASONING_RESERVE_TOKENS);
        assert_eq!(max_output(u32::MAX), u32::MAX);
    }

    #[test]
    fn only_a_proven_context_counts() {
        assert_eq!(context_tokens(Cli::Claude, None), Some(200_000));
        assert_eq!(context_tokens(Cli::Claude, Some(1_000_000)), Some(200_000));
        assert_eq!(context_tokens(Cli::Claude, Some(100_000)), Some(100_000));
        assert_eq!(context_tokens(Cli::Codex, None), None);
        assert_eq!(context_tokens(Cli::Codex, Some(0)), None);
        assert_eq!(context_tokens(Cli::Codex, Some(272_000)), Some(272_000));
    }

    #[test]
    fn aliases_get_a_concrete_model_where_known() {
        assert_eq!(
            concrete_model(Cli::Claude, "sonnet@high").as_deref(),
            Some("claude-sonnet-5-5")
        );
        assert_eq!(
            concrete_model(Cli::Claude, "claude-opus-5-5").as_deref(),
            Some("claude-opus-5-5")
        );
        assert_eq!(concrete_model(Cli::Claude, "unbekannt"), None);
        assert_eq!(
            concrete_model(Cli::Codex, "gpt-6.1-sol~fast").as_deref(),
            Some("gpt-6.1-sol")
        );
    }

    #[test]
    fn a_changed_tuple_invalidates_the_approval() {
        let stored = vec![binding()];
        assert_eq!(binding_state(&stored, &binding()), BindingState::Valid);
        let mut newer = binding();
        newer.cli_version = "2.1.319 (Claude Code)".into();
        assert_eq!(binding_state(&stored, &newer), BindingState::Changed);
        let mut swapped = binding();
        swapped.binary_sha256 = "cd".repeat(32);
        assert_eq!(binding_state(&stored, &swapped), BindingState::Changed);
        let mut other_model = binding();
        other_model.model_id = "opus".into();
        assert_eq!(binding_state(&stored, &other_model), BindingState::Missing);
        assert_eq!(binding_state(&[], &binding()), BindingState::Missing);
    }

    #[test]
    fn the_switch_works_only_with_a_valid_tuple() {
        let stored = vec![binding()];
        assert_eq!(gate(false, &stored, Some(&binding())), PreviewState::Off);
        assert_eq!(gate(true, &stored, Some(&binding())), PreviewState::Active);
        assert_eq!(gate(true, &[], Some(&binding())), PreviewState::Reconfirm);
        assert_eq!(gate(true, &stored, None), PreviewState::Reconfirm);
        let mut newer = binding();
        newer.cli_version = "9".into();
        assert_eq!(gate(true, &stored, Some(&newer)), PreviewState::Reconfirm);
    }

    #[test]
    fn precheck_rejects_with_the_budget_code() {
        let c = check(50_000);
        let err = precheck(Cli::Claude, &c, 1).unwrap_err();
        assert_eq!(err.code, "budget");
        assert_eq!(rejection_code(&err.error()), Some("budget"));
        assert!(precheck(Cli::Claude, &check(200_000), 10_000).is_ok());
    }

    #[test]
    fn a_good_reply_passes() {
        let r = reply(40_000, 300, Some("claude-sonnet-5-5"));
        assert_eq!(verify(&check(200_000), "sonnet", &r, 10_000), Ok(()));
        // Das Modell nennt die Ausgabe nicht: erlaubt (nur "falls gemeldet").
        let r = reply(40_000, 300, None);
        assert_eq!(verify(&check(200_000), "sonnet", &r, 10_000), Ok(()));
    }

    #[test]
    fn missing_or_implausible_usage_fails_closed() {
        let c = check(200_000);
        for r in [reply(0, 300, None), reply(40_000, 0, None)] {
            let e = verify(&c, "sonnet", &r, 10_000).unwrap_err();
            assert_eq!((e.code, e.reason), ("llm_failed", "usage_missing"));
        }
        // 10 Eingabetoken bei 160000 Bytes Prompt kann nicht stimmen.
        let e = verify(&c, "sonnet", &reply(10, 300, None), 160_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("llm_failed", "usage_implausible"));
    }

    #[test]
    fn a_different_model_is_rejected() {
        let r = reply(40_000, 300, Some("claude-haiku-4-5-20251001"));
        let e = verify(&check(200_000), "sonnet", &r, 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("llm_failed", "model_mismatch"));
        assert!(model_matches("sonnet", None, "claude-sonnet-5-5"));
        assert!(model_matches(
            "claude-sonnet-5-5@high",
            None,
            "claude-sonnet-5-5"
        ));
        assert!(model_matches("gpt-6.1-sol~fast", None, "gpt-6.1-sol"));
        assert!(!model_matches("gpt-6.1-sol", None, "gpt-6-luna"));
        assert!(!model_matches("sonnet", None, ""));
    }

    #[test]
    fn exceeding_the_context_or_compacting_or_truncating_is_a_budget_failure() {
        let c = check(100_000);
        let over = reply(99_800, 300, Some("claude-sonnet-5-5"));
        let e = verify(&c, "sonnet", &over, 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("budget", "context_exceeded"));
        let mut compacted = reply(40_000, 300, None);
        compacted.compacted = true;
        let e = verify(&c, "sonnet", &compacted, 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("budget", "compaction"));
        let mut cut = reply(40_000, 300, None);
        cut.truncated = true;
        let e = verify(&c, "sonnet", &cut, 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("budget", "truncated"));
    }

    #[test]
    fn events_hint_at_compaction_and_truncation() {
        assert_eq!(
            hints(&json!({"type":"result","stop_reason":"end_turn","usage":{"input_tokens":5}})),
            Hints::default()
        );
        assert!(hints(&json!({"type":"result","stop_reason":"max_tokens"})).truncated);
        assert!(hints(&json!({"type":"system","subtype":"compact_boundary"})).compacted);
        assert!(hints(&json!({"type":"context_compacted"})).compacted);
        assert!(
            hints(&json!({"type":"item.completed","item":{"type":"context_compaction"}})).compacted
        );
        assert!(hints(&json!({"num_compactions": 1})).compacted);
        assert!(!hints(&json!({"num_compactions": 0})).compacted);
        assert!(hints(&json!({"type":"response.incomplete"})).truncated);
        assert!(
            hints(&json!({"response":{"incomplete_details":{"reason":"max_output_tokens"}}}))
                .truncated
        );
        // Der Antworttext zählt nie, auch wenn er das Wort enthält.
        assert_eq!(
            hints(&json!({"type":"result","result":"compact max_tokens incomplete"})),
            Hints::default()
        );
    }

    #[test]
    fn the_audit_line_has_no_content_and_cannot_break_the_log() {
        let line = audit_line(
            &binding(),
            "sonnet\nFrage: geheim",
            Some("claude-sonnet-5-5"),
            "ok",
        );
        assert!(!line.contains('\n'));
        assert!(!line.contains("Frage:"));
        for part in [
            "provider=claude_cli",
            "model_requested=sonnet_Frage__geheim",
            "model_actual=claude-sonnet-5-5",
            "cli_version=2.1.318_(Claude_Code)",
            "method=empirical",
            "status=ok",
        ] {
            assert!(line.contains(part), "{part} fehlt in {line}");
        }
        assert!(line.contains(&"ab".repeat(32)));
    }

    #[test]
    fn version_and_hash_are_read() {
        assert_eq!(
            parse_version("\n 2.1.318 (Claude Code)\nmehr").as_deref(),
            Some("2.1.318 (Claude Code)")
        );
        assert_eq!(parse_version("  \n"), None);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("cli");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            sha256_file(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
