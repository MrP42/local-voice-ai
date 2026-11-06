//! Abo-CLIs (Claude Code, Codex) im Buch-Gespraech: nur als gekennzeichnete
//! VORSCHAU (lva#334, Entscheidung des Pruefers).
//!
//! Grundsatz: was nicht belegt ist, SPERRT (fail-closed); „unbekannt“ ergibt
//! nie Erfolg. Fuer ein Abo gibt es keinen Vorab-Beweis des Tokenverbrauchs:
//! die CLI schickt eigene, versteckte Anweisungen und kompaktiert still. Es
//! gibt deshalb KEINE eingebauten Zahlen und Faehigkeiten; zulaessig ist nur,
//! was auf diesem Geraet fuer genau diese CLI-Version belegt ist
//! ([`super::evidence`]: Faehigkeitstabelle `IsolationCaps` und Messreihe).
//!
//! * Freigabe: eigener Schalter `book_talk_subscription_preview`, ein
//!   gebundenes Tupel ([`PreviewBinding`]: Anbieter, Modell, CLI-Version,
//!   SHA-256 des TATSAECHLICH gestarteten Programms, [`payload_path`]), alle
//!   Isolationsfaehigkeiten dieser Version belegt UND eine passende Messung
//!   ([`gate`], [`proof_for`]). Fehlt eines, ist die Vorschau nicht freigegeben.
//! * Vorab ([`precheck`]): `UTF-8-Bytes(System + Prompt) + Overhead * 1,2 +
//!   max. Ausgabe <= Kompaktierungsschwelle * Kontext`, mit den GEMESSENEN
//!   Werten.
//! * Nachtraeglich ([`verify`]): Nutzungsangabe vorhanden und plausibel,
//!   gemeldetes Modell EXAKT das gemessene, Kompaktierung und Abschneiden
//!   je Tri-State ([`Tri`]) „nein“ (und der Ereignisvertrag der Version
//!   belegt), Eingabe + Ausgabe <= Kontext. Sonst wird der Text nie an
//!   Oberflaeche oder Sprache gegeben (Aufrufer: `cli::call_with_options`).
//! * Bindung: das Programm wird vor UND nach dem Aufruf neu gehasht
//!   ([`rebind`]); eine Abweichung (Auto-Update, Austausch) verwirft die Antwort.
//! * Schalter: [`PreviewSwitch`] wird vor, waehrend und nach dem Aufruf
//!   geprueft; bei „aus“ wird der Prozessbaum beendet und das Ergebnis verworfen.
//!
//! Kein Frage-, Antwort- oder Buchtext im Log: [`audit_line`] kennt nur Tupel,
//! Modellnamen, Methode und Status.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;

use super::cli::{launcher, split_spec, Cli, CliReply, CLAUDE_MODELS};
use super::evidence::{Evidence, IsolationCaps, Measurement};

/// Budgetmethode in Bericht und Audit.
pub const BUDGET_METHOD: &str = "empirical";

/// Kontextfenster der Claude-Modelle in Token (Vorgabe des Pruefers).
pub const CLAUDE_CONTEXT_TOKENS: u32 = 200_000;

/// Sicherheitszuschlag auf den GEMESSENEN Overhead, in Prozent (1,2-fach).
/// Kein Messwert und keine Freigabebasis, nur der Aufschlag auf einen solchen.
pub const OVERHEAD_FACTOR_PERCENT: u64 = 120;
/// Eine Nutzungsangabe mit weniger Eingabetoken als `Bytes / MIN_BYTES_PER_TOKEN`
/// ist unplausibel (kein Tokenizer packt mehr als ein paar Bytes in ein Token).
const MIN_BYTES_PER_TOKEN: usize = 16;

/// Drei Zustaende statt ja/nein: „unbekannt“ ist nie „nein“.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tri {
    No,
    Yes,
    /// Nicht erkennbar (z. B. Ausgabe ohne `stop_reason`, JSON statt Ereignisstrom).
    #[default]
    Unknown,
}

pub fn provider_of(cli: Cli) -> &'static str {
    match cli {
        Cli::Claude => "claude_cli",
        Cli::Codex => "codex_cli",
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

/// Planungsgrenze: Kompaktierungsschwelle (Prozent des Kontexts, gemessen).
pub fn limit_tokens(context: u32, threshold_percent: u64) -> u64 {
    u64::from(context) * threshold_percent.min(100) / 100
}

/// Eingabe in Token-Einheiten: UTF-8-Bytes (eine Obergrenze der Token) plus
/// gemessener Overhead mit Zuschlag.
pub fn input_estimate(overhead_tokens: u64, text_bytes: usize) -> u64 {
    text_bytes as u64 + overhead_tokens * OVERHEAD_FACTOR_PERCENT / 100
}

/// Maximale Ausgabe einer Antwort mit der gemessenen Reserve fuer Denk-Token.
pub fn max_output(answer_tokens: u32, reserve_tokens: u32) -> u32 {
    answer_tokens.saturating_add(reserve_tokens)
}

// ---------------------------------------------------------------- Bindung --

/// Das Tupel, an das die Freigabe gebunden ist. Aendert sich eines der Felder,
/// ist die Freigabe ungueltig und muss erneut bestaetigt werden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PreviewBinding {
    /// `claude_cli` oder `codex_cli`.
    pub provider: String,
    /// Modell, wie in der Konfiguration eingetragen (inkl. Effort-Endung).
    pub model_id: String,
    /// Konkrete Modell-ID statt Alias, soweit bekannt (nur Anzeige; die
    /// Pruefung nutzt das in der Messung gemeldete Modell).
    #[serde(default)]
    pub concrete_model: Option<String>,
    /// Ausgabe von `<cli> --version` (erste Zeile).
    pub cli_version: String,
    /// SHA-256 (hex) des tatsaechlich gestarteten Programms ([`payload_path`]).
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

/// Version und Pruefsumme der installierten CLI.
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
    /// Gespeichertes Tupel stimmt mit dem aktuellen ueberein.
    Valid,
    /// Fuer dieses Modell wurde nie bestaetigt.
    Missing,
    /// Version, Programm oder Modell weichen ab: erneut bestaetigen.
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

/// Belege, die zu einem Tupel gehoeren: Faehigkeiten (vollstaendig) und Messung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    pub caps: IsolationCaps,
    pub measurement: Measurement,
}

/// Warum die Vorschau (noch) nicht freigegeben ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Blocked {
    /// Keine CLI zu finden oder nicht lesbar.
    NoCli,
    /// Das tatsaechlich gestartete Programm ist nicht bestimmbar.
    PayloadUnknown,
    /// Fuer dieses Modell wurde nie bestaetigt.
    BindingMissing,
    /// Version, Programm oder Modell weichen vom bestaetigten Tupel ab.
    BindingChanged,
    /// Fuer diese CLI-Version sind nicht alle Isolationsfaehigkeiten belegt.
    IsolationUnverified,
    /// Keine Messung fuer (CLI, Version, Programm, Modell) auf diesem Geraet.
    MeasurementMissing,
    /// Der Kontext des Modells ist nicht belegt.
    ContextUnknown,
}

/// Belege fuer ein Tupel; die erste Luecke gibt den Grund.
pub fn proof_for(evidence: &Evidence, binding: &PreviewBinding) -> Result<Proof, Blocked> {
    let caps = evidence
        .caps_for(&binding.provider, &binding.cli_version)
        .filter(IsolationCaps::complete)
        .ok_or(Blocked::IsolationUnverified)?;
    let measurement = evidence
        .measurement_for(
            &binding.provider,
            &binding.cli_version,
            &binding.binary_sha256,
            &binding.model_id,
        )
        .ok_or(Blocked::MeasurementMissing)?;
    Ok(Proof { caps, measurement })
}

/// Wirkt die Vorschau fuer diesen Aufruf?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PreviewState {
    /// Schalter aus.
    Off,
    /// Schalter an, Tupel gueltig, Isolation und Messung belegt.
    Active,
    /// Schalter an, aber das Tupel fehlt oder weicht ab (oder die CLI ist nicht
    /// zu pruefen): der Schalter wirkt nicht, die Oberflaeche fragt erneut.
    Reconfirm,
    /// Schalter an, Tupel gueltig, aber Isolation oder Messung auf diesem
    /// Geraet fehlen: „Noch nicht freigegeben“.
    NotReleased,
}

/// Ergebnis von [`gate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub state: PreviewState,
    pub blocked: Option<Blocked>,
    pub proof: Option<Proof>,
}

impl Gate {
    fn state(state: PreviewState, blocked: Option<Blocked>) -> Self {
        Self {
            state,
            blocked,
            proof: None,
        }
    }
}

/// Alle Voraussetzungen: Schalter, gueltiges Tupel, vollstaendige Isolation,
/// gueltige Messung. Nur dann `Active` mit Belegen.
pub fn gate(
    enabled: bool,
    stored: &[PreviewBinding],
    current: Option<&PreviewBinding>,
    evidence: &Evidence,
) -> Gate {
    if !enabled {
        return Gate::state(PreviewState::Off, None);
    }
    let Some(current) = current else {
        return Gate::state(PreviewState::Reconfirm, Some(Blocked::NoCli));
    };
    match binding_state(stored, current) {
        BindingState::Missing => {
            Gate::state(PreviewState::Reconfirm, Some(Blocked::BindingMissing))
        }
        BindingState::Changed => {
            Gate::state(PreviewState::Reconfirm, Some(Blocked::BindingChanged))
        }
        BindingState::Valid => match proof_for(evidence, current) {
            Ok(proof) => Gate {
                state: PreviewState::Active,
                blocked: None,
                proof: Some(proof),
            },
            Err(b) => Gate::state(PreviewState::NotReleased, Some(b)),
        },
    }
}

/// Grund aus dem Fehlertext von [`current_binding`] bzw. [`identity`].
pub fn blocked_of_error(err: &str) -> Blocked {
    if err.starts_with("payload_unknown") {
        Blocked::PayloadUnknown
    } else {
        Blocked::NoCli
    }
}

// ------------------------------------------------- Programm-Identitaet (IO) --

/// Version je Inhaltshash: der Start von `--version` passiert nicht je Frage.
/// Der Hash selbst wird nie gemerkt, sondern bei jeder Pruefung neu gebildet.
static VERSION_CACHE: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// SHA-256 (hex) einer Datei, gelesen in Bloecken.
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

/// Erste nichtleere Zeile einer `--version`-Ausgabe, auf 120 Zeichen gekuerzt.
pub fn parse_version(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(120).collect())
}

fn is_shim(binary: &Path) -> bool {
    binary
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
}

/// Wurzel des npm-Pakets `@openai/codex`, wenn `binary` nur dessen Startskript
/// ist: ein `.cmd`-Skript neben `node_modules/@openai/codex` (Windows) oder ein
/// Verweis (`bin/codex` -> `…/@openai/codex/bin/codex.js`, Linux/macOS).
fn npm_package_root(binary: &Path, real: &Path) -> Option<PathBuf> {
    if is_shim(binary) {
        let pkg = binary
            .parent()?
            .join("node_modules")
            .join("@openai")
            .join("codex");
        return pkg.is_dir().then_some(pkg);
    }
    real.ancestors()
        .find(|a| a.ends_with(Path::new("@openai").join("codex")))
        .map(Path::to_path_buf)
}

/// Das native Programm eines npm-Pakets: `…/codex-<plattform>/vendor/<triple>/
/// codex/codex[.exe]`, im Paket selbst, darunter oder als Nachbar unter
/// `@openai/`. Genau ein Treffer, sonst ist es nicht bestimmbar.
fn native_in_package(root: &Path) -> Result<PathBuf, String> {
    let mut vendors: Vec<PathBuf> = vec![root.join("vendor")];
    let scopes = [
        Some(root.join("node_modules").join("@openai")),
        root.parent().map(Path::to_path_buf),
    ];
    for scope in scopes.into_iter().flatten() {
        let Ok(entries) = std::fs::read_dir(&scope) else {
            continue;
        };
        vendors.extend(
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("codex-"))
                .map(|e| e.path().join("vendor")),
        );
    }
    let mut found = BTreeSet::new();
    for vendor in vendors {
        let Ok(triples) = std::fs::read_dir(&vendor) else {
            continue;
        };
        for triple in triples.flatten() {
            for name in ["codex", "codex.exe"] {
                let exe = triple.path().join("codex").join(name);
                if exe.is_file() {
                    if let Ok(real) = std::fs::canonicalize(&exe) {
                        found.insert(real);
                    }
                }
            }
        }
    }
    match found.len() {
        1 => Ok(found.into_iter().next().unwrap_or_default()),
        0 => Err("payload_unknown: kein natives Programm im npm-Paket gefunden".to_string()),
        _ => Err("payload_unknown: mehrere native Programme im npm-Paket".to_string()),
    }
}

/// Das Programm, das wirklich laeuft und gehasht wird: bei Claude die
/// aufgeloeste Datei (Verweise aufgeloest), bei einem npm-Codex die NATIVE
/// Binaerdatei statt des Startskripts. Nicht bestimmbar: `payload_unknown`.
pub fn payload_path(binary: &Path) -> Result<PathBuf, String> {
    let real = std::fs::canonicalize(binary).map_err(|e| format!("cli_missing: {e}"))?;
    match npm_package_root(binary, &real) {
        None => Ok(real),
        Some(root) => native_in_package(&root),
    }
}

/// Version und Pruefsumme der CLI. Hasht bei JEDEM Aufruf neu (kein Cache ueber
/// Pfad/Zeit/Groesse); nur die Version wird je Inhaltshash gemerkt. Blockiert
/// (liest die Datei, startet ggf. die CLI): auf einem Blocking-Thread ausfuehren.
pub fn identity(binary: &Path) -> Result<Identity, String> {
    let payload = payload_path(binary)?;
    let sha256 = sha256_file(&payload).map_err(|e| format!("cli_failed: {e}"))?;
    let cached = VERSION_CACHE.lock().ok().and_then(|c| {
        c.iter()
            .find(|(hash, _)| *hash == sha256)
            .map(|(_, v)| v.clone())
    });
    if let Some(version) = cached {
        return Ok(Identity { version, sha256 });
    }
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
    if let Ok(mut c) = VERSION_CACHE.lock() {
        c.retain(|(hash, _)| *hash != sha256);
        c.push((sha256.clone(), version.clone()));
    }
    Ok(Identity { version, sha256 })
}

/// Aktuelles Tupel fuer ein Modell: sucht die CLI und liest ihre Identitaet.
/// Blockiert.
pub fn current_binding(cli: Cli, model_id: &str) -> Result<PreviewBinding, String> {
    let binary = super::cli::locate(cli)
        .ok_or_else(|| "cli_missing: Die CLI des Anbieters ist nicht installiert".to_string())?;
    Ok(binding_for(cli, model_id, &identity(&binary)?))
}

/// Ist das Programm noch genau das gebundene? Vor dem Start UND nach dem Ende
/// des Aufrufs; nach Auto-Update, Wiederholung oder Austausch weicht es ab und
/// die Freigabe gilt nicht mehr.
pub fn rebind(binary: &Path, expected: &PreviewBinding) -> Result<(), Rejection> {
    let Ok(now) = identity(binary) else {
        return Err(Rejection {
            code: LLM_FAILED,
            reason: "payload_unknown",
        });
    };
    if now.version == expected.cli_version
        && now.sha256.eq_ignore_ascii_case(&expected.binary_sha256)
    {
        return Ok(());
    }
    Err(Rejection {
        code: LLM_FAILED,
        reason: "binding_changed",
    })
}

// ------------------------------------------------------------ Schalter --

/// Liest den Vorschau-Schalter LIVE (nicht den Stand zu Beginn der Frage).
#[derive(Clone)]
pub struct PreviewSwitch(std::sync::Arc<dyn Fn() -> bool + Send + Sync>);

impl PreviewSwitch {
    pub fn new(read: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(read))
    }

    pub fn is_on(&self) -> bool {
        (self.0)()
    }
}

impl std::fmt::Debug for PreviewSwitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewSwitch")
    }
}

// ------------------------------------------------------------- Aufrufpruefung --

/// Alles, was ein Aufruf in der Vorschau einhalten muss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewCheck {
    pub binding: PreviewBinding,
    /// Belegter Kontext des Modells.
    pub context_tokens: u32,
    /// Hoechste Ausgabe inklusive gemessener Reserve ([`max_output`]).
    pub max_output_tokens: u32,
    /// Faehigkeiten und Messung dieser Version.
    pub proof: Proof,
}

impl PreviewCheck {
    pub fn new(
        binding: PreviewBinding,
        context_tokens: u32,
        answer_tokens: u32,
        proof: Proof,
    ) -> Self {
        let max_output_tokens =
            max_output(answer_tokens, proof.measurement.reasoning_reserve_tokens);
        Self {
            binding,
            context_tokens,
            max_output_tokens,
            proof,
        }
    }

    /// Eingabe in Token-Einheiten (gemessener Overhead mit Zuschlag).
    pub fn input_estimate(&self, text_bytes: usize) -> u64 {
        input_estimate(self.proof.measurement.max_overhead_tokens, text_bytes)
    }

    /// Planungsgrenze: gemessene Kompaktierungsschwelle des Kontexts.
    pub fn limit(&self) -> u64 {
        limit_tokens(
            self.context_tokens,
            self.proof.measurement.compaction_threshold_percent,
        )
    }

    /// Die Vorab-Pruefung: passt der Aufruf unter die Schwelle?
    pub fn fits(&self, text_bytes: usize) -> bool {
        self.input_estimate(text_bytes) + u64::from(self.max_output_tokens) <= self.limit()
    }
}

/// Ablehnung: `code` ist der Code des Buch-Gespraechs (`budget`, `llm_failed`
/// oder `cancelled`), `reason` ein fester Text fuer das Audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejection {
    pub code: &'static str,
    pub reason: &'static str,
}

/// Praefix des Fehlertexts einer Ablehnung: `cli_preview_rejected:<code>:<grund>`.
pub const REJECTED_PREFIX: &str = "cli_preview_rejected";

impl Rejection {
    pub fn error(&self) -> String {
        format!("{REJECTED_PREFIX}:{}:{}", self.code, self.reason)
    }
}

/// Code aus einem Fehlertext einer Ablehnung (`budget`, `cancelled` oder
/// `llm_failed`).
pub fn rejection_code(err: &str) -> Option<&'static str> {
    let rest = err.strip_prefix(REJECTED_PREFIX)?.strip_prefix(':')?;
    if rest.starts_with("budget") {
        Some("budget")
    } else if rest.starts_with("cancelled") {
        Some("cancelled")
    } else {
        Some("llm_failed")
    }
}

const BUDGET: &str = "budget";
const LLM_FAILED: &str = "llm_failed";
const CANCELLED: &str = "cancelled";

/// Der Schalter wurde waehrend der Frage ausgeschaltet: Ergebnis verwerfen.
pub const SWITCH_OFF: Rejection = Rejection {
    code: CANCELLED,
    reason: "switch_off",
};

/// Vorab: Passt der Aufruf in das gemessene Budget?
pub fn precheck(check: &PreviewCheck, text_bytes: usize) -> Result<(), Rejection> {
    if check.fits(text_bytes) {
        return Ok(());
    }
    Err(Rejection {
        code: BUDGET,
        reason: "budget_precheck",
    })
}

/// Ist das gemeldete Modell GENAU das gemessene? Kein Teilstring, keine
/// Gross-/Kleinschreibung, kein Alias: nur Gleichheit nach Trimmen.
pub fn model_is_exact(expected: &str, actual: &str) -> bool {
    let (expected, actual) = (expected.trim(), actual.trim());
    !expected.is_empty() && expected == actual
}

/// Nachtraeglich: darf die Antwort verwendet werden? Fail-closed.
pub fn verify(check: &PreviewCheck, reply: &CliReply, input_bytes: usize) -> Result<(), Rejection> {
    let reject = |code, reason| Err(Rejection { code, reason });
    if !check.proof.caps.event_contract {
        return reject(LLM_FAILED, "event_contract");
    }
    if reply.prompt_tokens == 0 || reply.completion_tokens == 0 {
        return reject(LLM_FAILED, "usage_missing");
    }
    let plausible_min = (input_bytes / MIN_BYTES_PER_TOKEN) as u64;
    if reply.prompt_tokens < plausible_min {
        return reject(LLM_FAILED, "usage_implausible");
    }
    let Some(actual) = reply.model.as_deref() else {
        return reject(LLM_FAILED, "model_missing");
    };
    if !model_is_exact(&check.proof.measurement.reported_model, actual) {
        return reject(LLM_FAILED, "model_mismatch");
    }
    match reply.compacted {
        Tri::No => {}
        Tri::Yes => return reject(BUDGET, "compaction"),
        Tri::Unknown => return reject(BUDGET, "compaction_unknown"),
    }
    match reply.truncated {
        Tri::No => {}
        Tri::Yes => return reject(BUDGET, "truncated"),
        Tri::Unknown => return reject(BUDGET, "truncated_unknown"),
    }
    if reply.prompt_tokens + reply.completion_tokens > u64::from(check.context_tokens) {
        return reject(BUDGET, "context_exceeded");
    }
    Ok(())
}

// ------------------------------------------------------------------ Audit --

/// Audit-Zeile ohne Inhalte: Tupel, Modell angefragt und tatsaechlich,
/// Budgetmethode, Status. Gleiches Muster wie die uebrigen `book_talk`-Zeilen.
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

/// Bausteine fuer Tests anderer Module: vollstaendige Faehigkeiten und eine
/// passende Messung zu einem Tupel.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn caps() -> IsolationCaps {
        IsolationCaps {
            instructions_blocked: true,
            project_instructions_blocked: true,
            hooks_blocked: true,
            memory_blocked: true,
            tools_blocked: true,
            event_contract: true,
        }
    }

    pub fn measurement(binding: &PreviewBinding, reported: &str) -> Measurement {
        Measurement {
            cli: binding.provider.clone(),
            cli_version: binding.cli_version.clone(),
            binary_sha256: binding.binary_sha256.clone(),
            model: binding.model_id.clone(),
            reported_model: reported.to_string(),
            runs: 10,
            max_overhead_tokens: 30_000,
            reasoning_reserve_tokens: 4_000,
            compaction_threshold_percent: 80,
            measured_at: "2026-11-07T10:00:00Z".into(),
            kind: "empirisch".into(),
        }
    }

    pub fn proof(binding: &PreviewBinding, reported: &str) -> Proof {
        Proof {
            caps: caps(),
            measurement: measurement(binding, reported),
        }
    }

    /// Belege fuer alle Tupel (Modell `reported` je Tupel).
    pub fn evidence(bindings: &[(&PreviewBinding, &str)]) -> Evidence {
        let caps_entries: Vec<(&str, &str, IsolationCaps)> = bindings
            .iter()
            .map(|(b, _)| (b.provider.as_str(), b.cli_version.as_str(), caps()))
            .collect();
        Evidence::from_parts(
            &caps_entries,
            bindings.iter().map(|(b, r)| measurement(b, r)).collect(),
        )
    }

    pub fn check(binding: PreviewBinding, reported: &str, context: u32) -> PreviewCheck {
        let proof = proof(&binding, reported);
        PreviewCheck::new(binding, context, 600, proof)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures;
    use super::*;

    const V: &str = "2.1.318 (Claude Code)";

    fn ident(version: &str, tag: &str) -> Identity {
        Identity {
            version: version.into(),
            sha256: tag.repeat(32),
        }
    }

    fn binding() -> PreviewBinding {
        binding_for(Cli::Claude, "sonnet", &ident(V, "ab"))
    }

    fn check(context: u32) -> PreviewCheck {
        fixtures::check(binding(), "claude-sonnet-5-5", context)
    }

    fn reply(input: u64, output: u64, model: Option<&str>) -> CliReply {
        CliReply {
            text: "synthetisch".into(),
            prompt_tokens: input,
            completion_tokens: output,
            model: model.map(str::to_string),
            compacted: Tri::No,
            truncated: Tri::No,
        }
    }

    #[test]
    fn the_budget_uses_only_measured_values() {
        // Messung: Overhead 30000 * 1,2 = 36000, Schwelle 80 % von 200000, Reserve 4000.
        let c = check(200_000);
        assert_eq!(c.input_estimate(1_000), 37_000);
        assert_eq!(c.limit(), 160_000);
        assert_eq!(c.max_output_tokens, 600 + 4_000);
        let room = 160_000 - 36_000 - u64::from(c.max_output_tokens);
        assert!(c.fits(room as usize));
        assert!(!c.fits(room as usize + 1));
        // Eine andere Messung ergibt eine andere Grenze: nichts ist eingebaut.
        let mut other = check(200_000);
        other.proof.measurement.compaction_threshold_percent = 50;
        other.proof.measurement.max_overhead_tokens = 10_000;
        assert_eq!(other.limit(), 100_000);
        assert_eq!(other.input_estimate(0), 12_000);
        let mut small = check(32_000);
        assert!(!small.fits(0));
        small.proof.measurement.max_overhead_tokens = 1_000;
        assert!(small.fits(0));
    }

    #[test]
    fn the_reasoning_reserve_is_part_of_the_output() {
        assert_eq!(max_output(600, 3_100), 3_700);
        assert_eq!(max_output(u32::MAX, 5), u32::MAX);
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
    fn ready_needs_switch_binding_isolation_and_measurement() {
        let b = binding();
        let stored = vec![b.clone()];
        let full = fixtures::evidence(&[(&b, "claude-sonnet-5-5")]);
        let blocked = |g: Gate| (g.state, g.blocked);
        // Alles belegt.
        let g = gate(true, &stored, Some(&b), &full);
        assert_eq!(blocked(g.clone()), (PreviewState::Active, None));
        assert!(g.proof.is_some());
        // Schalter aus.
        assert_eq!(
            blocked(gate(false, &stored, Some(&b), &full)),
            (PreviewState::Off, None)
        );
        // Kein Tupel / CLI nicht pruefbar.
        assert_eq!(
            blocked(gate(true, &stored, None, &full)),
            (PreviewState::Reconfirm, Some(Blocked::NoCli))
        );
        assert_eq!(
            blocked(gate(true, &[], Some(&b), &full)),
            (PreviewState::Reconfirm, Some(Blocked::BindingMissing))
        );
        let mut newer = b.clone();
        newer.cli_version = "9".into();
        assert_eq!(
            blocked(gate(true, &stored, Some(&newer), &full)),
            (PreviewState::Reconfirm, Some(Blocked::BindingChanged))
        );
        // Ohne Belege: gesperrt, mit Grund.
        assert_eq!(
            blocked(gate(true, &stored, Some(&b), &Evidence::default())),
            (
                PreviewState::NotReleased,
                Some(Blocked::IsolationUnverified)
            )
        );
        // Faehigkeiten nur teilweise: gesperrt.
        let mut partial = fixtures::caps();
        partial.hooks_blocked = false;
        let ev = Evidence::from_parts(
            &[(b.provider.as_str(), b.cli_version.as_str(), partial)],
            vec![fixtures::measurement(&b, "claude-sonnet-5-5")],
        );
        assert_eq!(
            blocked(gate(true, &stored, Some(&b), &ev)),
            (
                PreviewState::NotReleased,
                Some(Blocked::IsolationUnverified)
            )
        );
        // Faehigkeiten da, Messung fehlt.
        let ev = Evidence::from_parts(
            &[(
                b.provider.as_str(),
                b.cli_version.as_str(),
                fixtures::caps(),
            )],
            vec![],
        );
        assert_eq!(
            blocked(gate(true, &stored, Some(&b), &ev)),
            (PreviewState::NotReleased, Some(Blocked::MeasurementMissing))
        );
        // Messung fuer ein anderes Modell oder anderes Programm zaehlt nicht.
        let mut other = b.clone();
        other.model_id = "opus".into();
        let ev = Evidence::from_parts(
            &[(
                b.provider.as_str(),
                b.cli_version.as_str(),
                fixtures::caps(),
            )],
            vec![fixtures::measurement(&other, "claude-opus-5-5")],
        );
        assert_eq!(
            blocked(gate(true, &stored, Some(&b), &ev)),
            (PreviewState::NotReleased, Some(Blocked::MeasurementMissing))
        );
        let mut other_hash = b.clone();
        other_hash.binary_sha256 = "cd".repeat(32);
        let ev = fixtures::evidence(&[(&other_hash, "claude-sonnet-5-5")]);
        assert_eq!(
            blocked(gate(true, &stored, Some(&b), &ev)),
            (PreviewState::NotReleased, Some(Blocked::MeasurementMissing))
        );
    }

    #[test]
    fn precheck_rejects_with_the_budget_code() {
        let c = check(50_000);
        let err = precheck(&c, 1).unwrap_err();
        assert_eq!(err.code, "budget");
        assert_eq!(rejection_code(&err.error()), Some("budget"));
        assert!(precheck(&check(200_000), 10_000).is_ok());
    }

    #[test]
    fn a_good_reply_passes() {
        let r = reply(40_000, 300, Some("claude-sonnet-5-5"));
        assert_eq!(verify(&check(200_000), &r, 10_000), Ok(()));
    }

    #[test]
    fn missing_or_implausible_usage_fails_closed() {
        let c = check(200_000);
        for r in [
            reply(0, 300, Some("claude-sonnet-5-5")),
            reply(40_000, 0, Some("claude-sonnet-5-5")),
        ] {
            let e = verify(&c, &r, 10_000).unwrap_err();
            assert_eq!((e.code, e.reason), ("llm_failed", "usage_missing"));
        }
        // 10 Eingabetoken bei 160000 Bytes Prompt kann nicht stimmen.
        let r = reply(10, 300, Some("claude-sonnet-5-5"));
        let e = verify(&c, &r, 160_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("llm_failed", "usage_implausible"));
    }

    #[test]
    fn a_missing_or_inexact_model_is_rejected() {
        let c = check(200_000);
        // Ohne gemeldetes Modell: abgelehnt (frueher erlaubt).
        let e = verify(&c, &reply(40_000, 300, None), 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("llm_failed", "model_missing"));
        let e = verify(&c, &reply(40_000, 300, Some("  ")), 10_000).unwrap_err();
        assert_eq!(e.reason, "model_mismatch");
        for wrong in [
            "claude-haiku-4-5-20251001",
            // Teilstring-Treffer zaehlen nicht mehr.
            "claude-sonnet-5-5-preview",
            "xclaude-sonnet-5-5",
            "sonnet",
            "CLAUDE-SONNET-5-5",
        ] {
            let e = verify(&c, &reply(40_000, 300, Some(wrong)), 10_000).unwrap_err();
            assert_eq!(
                (e.code, e.reason),
                ("llm_failed", "model_mismatch"),
                "{wrong}"
            );
        }
        assert!(model_is_exact("claude-sonnet-5-5", " claude-sonnet-5-5\n"));
        assert!(!model_is_exact("", ""));
        assert!(!model_is_exact("gpt-6.1-sol", "gpt-6.1-sol-mini"));
    }

    #[test]
    fn unknown_is_never_success() {
        let c = check(200_000);
        let mut r = reply(40_000, 300, Some("claude-sonnet-5-5"));
        r.compacted = Tri::Unknown;
        assert_eq!(
            verify(&c, &r, 10_000).unwrap_err().reason,
            "compaction_unknown"
        );
        r.compacted = Tri::Yes;
        let e = verify(&c, &r, 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("budget", "compaction"));
        r.compacted = Tri::No;
        r.truncated = Tri::Unknown;
        assert_eq!(
            verify(&c, &r, 10_000).unwrap_err().reason,
            "truncated_unknown"
        );
        r.truncated = Tri::Yes;
        let e = verify(&c, &r, 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("budget", "truncated"));
        // Ohne belegten Ereignisvertrag der Version ist „nein“ nichts wert.
        let mut c2 = check(200_000);
        c2.proof.caps.event_contract = false;
        let ok = reply(40_000, 300, Some("claude-sonnet-5-5"));
        assert_eq!(
            verify(&c2, &ok, 10_000).unwrap_err().reason,
            "event_contract"
        );
        assert_eq!(Tri::default(), Tri::Unknown);
    }

    #[test]
    fn exceeding_the_context_is_a_budget_failure() {
        let c = check(100_000);
        let over = reply(99_800, 300, Some("claude-sonnet-5-5"));
        let e = verify(&c, &over, 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("budget", "context_exceeded"));
    }

    #[test]
    fn the_switch_rejection_maps_to_cancelled() {
        assert_eq!(
            SWITCH_OFF.error(),
            "cli_preview_rejected:cancelled:switch_off"
        );
        assert_eq!(rejection_code(&SWITCH_OFF.error()), Some("cancelled"));
        assert_eq!(rejection_code("cli_failed: x"), None);
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let f = flag.clone();
        let switch = PreviewSwitch::new(move || f.load(std::sync::atomic::Ordering::SeqCst));
        assert!(switch.is_on());
        flag.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(!switch.is_on());
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

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_plain_program_is_hashed_itself_through_links() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("versions").join("claude");
        write(&real, b"native");
        let got = payload_path(&real).unwrap();
        assert_eq!(got, std::fs::canonicalize(&real).unwrap());
        #[cfg(unix)]
        {
            let link = dir.path().join("bin").join("claude");
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert_eq!(payload_path(&link).unwrap(), got);
        }
        assert!(payload_path(&dir.path().join("fehlt")).is_err());
    }

    #[test]
    fn an_npm_codex_is_hashed_by_its_native_binary_not_the_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let npm = dir.path().join("npm");
        let pkg = npm.join("node_modules").join("@openai").join("codex");
        write(&npm.join("codex.cmd"), b"@echo off");
        write(&pkg.join("bin").join("codex.js"), b"// launcher");
        // Windows-Paket: Plattformpaket unter dem Paket selbst.
        let native = pkg
            .join("node_modules")
            .join("@openai")
            .join("codex-win32-x64")
            .join("vendor")
            .join("x86_64-pc-windows-msvc")
            .join("codex")
            .join("codex.exe");
        assert!(
            payload_path(&npm.join("codex.cmd")).is_err(),
            "ohne natives Programm: gesperrt"
        );
        write(&native, b"NATIVE-PAYLOAD");
        let got = payload_path(&npm.join("codex.cmd")).unwrap();
        assert_eq!(got, std::fs::canonicalize(&native).unwrap());
        assert_ne!(
            sha256_file(&got).unwrap(),
            sha256_file(&npm.join("codex.cmd")).unwrap()
        );
        // Ein zweites natives Programm: nicht eindeutig, gesperrt.
        let second = pkg
            .join("vendor")
            .join("aarch64-apple-darwin")
            .join("codex")
            .join("codex");
        write(&second, b"ANDERES");
        let err = payload_path(&npm.join("codex.cmd")).unwrap_err();
        assert!(err.starts_with("payload_unknown"), "{err}");
        assert_eq!(blocked_of_error(&err), Blocked::PayloadUnknown);
        assert_eq!(blocked_of_error("cli_missing: x"), Blocked::NoCli);
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_npm_codex_resolves_to_the_native_sibling_package() {
        let dir = tempfile::tempdir().unwrap();
        let scope = dir.path().join("lib").join("node_modules").join("@openai");
        write(
            &scope.join("codex").join("bin").join("codex.js"),
            b"// launcher",
        );
        let native = scope
            .join("codex-linux-x64")
            .join("vendor")
            .join("x86_64-unknown-linux-musl")
            .join("codex")
            .join("codex");
        write(&native, b"NATIVE");
        let bin = dir.path().join("bin").join("codex");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(scope.join("codex").join("bin").join("codex.js"), &bin).unwrap();
        assert_eq!(
            payload_path(&bin).unwrap(),
            std::fs::canonicalize(&native).unwrap()
        );
    }

    #[cfg(unix)]
    fn fake_version_cli(dir: &Path, version: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!("#!/bin/sh\n# {body}\n[ \"$1\" = \"--version\" ] && echo \"{version}\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[cfg(unix)]
    #[test]
    fn the_hash_is_recomputed_and_a_swapped_program_breaks_the_binding() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_version_cli(dir.path(), "7.7.7 (Claude Code)", "eins");
        let first = identity(&bin).unwrap();
        assert_eq!(first.version, "7.7.7 (Claude Code)");
        let bound = binding_for(Cli::Claude, "sonnet", &first);
        assert_eq!(rebind(&bin, &bound), Ok(()));
        // Anderer Inhalt, gleiche Groesse, gleicher Pfad: der Hash aendert sich
        // (kein Cache ueber Pfad/Zeit/Groesse).
        fake_version_cli(dir.path(), "7.7.7 (Claude Code)", "zwei");
        let second = identity(&bin).unwrap();
        assert_ne!(first.sha256, second.sha256);
        let err = rebind(&bin, &bound).unwrap_err();
        assert_eq!((err.code, err.reason), ("llm_failed", "binding_changed"));
        // Neue Version bei gleichem Pfad: ebenfalls abgewichen.
        fake_version_cli(dir.path(), "7.7.8 (Claude Code)", "eins");
        assert_eq!(rebind(&bin, &bound).unwrap_err().reason, "binding_changed");
        // Programm weg: nicht bestimmbar.
        std::fs::remove_file(&bin).unwrap();
        assert_eq!(rebind(&bin, &bound).unwrap_err().reason, "payload_unknown");
    }
}
