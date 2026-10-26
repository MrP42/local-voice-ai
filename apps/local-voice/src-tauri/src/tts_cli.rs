//! Kommandozeile der Sprachausgabe-Anbieter (Goal tts-provider, Paket P3b):
//! `--tts-providers`, `--tts-voices`, `--tts-test --tts-provider <id>` und
//! `--tts-bench`. Alles läuft headless (kein Fenster); die Messschleife selbst
//! steht in `managers::tts::providers::bench`.
//!
//! ## Keine Freigaben über die Kommandozeile
//!
//! Die CLI hat KEINEN Schalter für Opt-in, Lizenz-Zustimmung oder Fernzugriff
//! und liest sie nur aus den Einstellungen der App. Jede Nutzung (Stimmenliste,
//! Probe, Benchmark) läuft durch [`Plan::authorize`] und damit durch
//! `check_use`; ein gesperrter Anbieter endet mit Exit 3 und der Meldung, welche
//! Freigabe in der App fehlt. Überschrieben werden dürfen nur Exe-Pfad, Modell,
//! Adresse und Stimme (`--tts-exe`, `--tts-model`, `--tts-base-url`,
//! `--tts-voice`; Vorrang Argument > Umgebung > Einstellungen) — und auch eine
//! überschriebene Adresse außerhalb dieses Rechners bleibt gesperrt, solange die
//! App den Fernzugriff nicht erlaubt hat (`check_use` sieht die WIRKSAME Adresse).
//!
//! ## Exit-Codes
//!
//! | Code | Bedeutung |
//! |---|---|
//! | 0 | fertig (Benchmark: Zeilen geschrieben, auch bei einzelnen Fehlschlägen) |
//! | 1 | Fehler (Anbieter nicht bereit, Datei-/Ordnerfehler, Synthese gescheitert) |
//! | 2 | falsche Eingabe (unbekannter Anbieter, fehlendes/unpassendes Argument, Korpus) |
//! | 3 | gesperrt (Opt-in, Lizenz oder Fernzugriff fehlt in der App) |
//! | 4 | Benchmark abgebrochen nach wiederholten Fehlschlägen (Zeilen bleiben) |
//! | 130 | Benchmark mit Strg+C beendet (Zeilen bleiben, eigener Server beendet) |
//!
//! ## Strg+C
//!
//! Unter Windows meldet ein Konsolen-Handler den Abbruch an die Messschleife:
//! sie lässt die laufende Synthese fallen (Subprozess-Adapter beenden ihr Kind),
//! `rows.jsonl` behält alle ganzen Zeilen, ein SELBST gestarteter Fish-Server
//! wird beendet. Räumt der Lauf nicht binnen 15 s auf, beendet sich der Prozess
//! trotzdem (Fish hängt im Job-Objekt und stirbt mit). Andere Plattformen
//! behalten das Standardverhalten des Betriebssystems.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::cli::CliArgs;
use crate::commands::tts_providers::{build_entries, TtsProviderEntry};
use crate::managers::tts::engine::{self, TtsEngineKind};
use crate::managers::tts::piper;
use crate::managers::tts::providers::bench::{
    self, BenchBackend, BenchError, BenchOptions, ColdReset, Outcome,
};
use crate::managers::tts::providers::contract::{
    CancelToken, Device, ProviderDescriptor, ProviderTier, StreamingMode, SynthesisOptions,
    TtsError, TtsErrorKind, VoiceDescriptor,
};
use crate::managers::tts::providers::host::ProviderSnapshot;
use crate::managers::tts::providers::phonon::PHONON_ID;
use crate::managers::tts::providers::policy::{
    self, ConfigSlot, LiveApprovals, ProviderOverride, ProviderOverrides, ResolvedConfigs,
    TtsProvidersSettings, FISH_ID, PIPER_ID,
};
use crate::managers::tts::providers::registry::ProviderRegistry;
use crate::managers::tts::providers::setup::{sync_registry_gated, ProviderPaths};
use crate::managers::tts::providers::SynthFn;
use crate::managers::tts::TtsManager;

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_BAD_INPUT: i32 = 2;
pub const EXIT_BLOCKED: i32 = 3;
pub const EXIT_INCOMPLETE: i32 = 4;
pub const EXIT_INTERRUPTED: i32 = 130;

/// Frist der Gesundheitsprüfung je Anbieter (wie `commands::tts_providers`).
const HEALTH_LIMIT: Duration = Duration::from_secs(4);
/// Frist der Stimmenliste.
const VOICES_LIMIT: Duration = Duration::from_secs(10);
/// Frist der Probesynthese.
const TEST_TIMEOUT: Duration = Duration::from_secs(120);
/// Nach Strg+C: so lange darf das Aufräumen dauern.
#[cfg(windows)]
const INTERRUPT_GRACE: Duration = Duration::from_secs(15);

const DEFAULT_TEST_TEXT: &str = "Dies ist der Selbsttest der lokalen Sprachausgabe.";

// ---- Fehler ---------------------------------------------------------------------------

/// Ein Fehler mit Exit-Code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub code: i32,
    pub message: String,
}

impl CliError {
    fn bad(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_BAD_INPUT,
            message: message.into(),
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_ERROR,
            message: message.into(),
        }
    }

    fn from_tts(e: &TtsError) -> Self {
        let message = match e.kind {
            TtsErrorKind::LicenseBlocked | TtsErrorKind::PrivacyBlocked => blocked_message(e),
            _ => e.message.clone(),
        };
        Self {
            code: exit_code_for(e.kind),
            message,
        }
    }
}

/// Exit-Code zu einer Fehlerklasse.
pub fn exit_code_for(kind: TtsErrorKind) -> i32 {
    match kind {
        TtsErrorKind::LicenseBlocked | TtsErrorKind::PrivacyBlocked => EXIT_BLOCKED,
        TtsErrorKind::BadRequest | TtsErrorKind::InvalidConfig => EXIT_BAD_INPUT,
        _ => EXIT_ERROR,
    }
}

/// Die Sperrmeldung samt Hinweis, wer freigeben darf.
pub fn blocked_message(e: &TtsError) -> String {
    format!(
        "Gesperrt ({}): {}\nDie Freigabe erteilt nur die App (Einstellungen > Sprachausgabe-Anbieter); \
         die Kommandozeile hat dafür bewusst keinen Schalter.",
        e.kind.as_str(),
        e.message
    )
}

// ---- Einstellungen und Argumente ------------------------------------------------------

/// Die Einstellungen, die die CLI braucht (aus `AppSettings` gezogen, damit der Rest rein ist).
#[derive(Debug, Clone, Default)]
pub struct CliSettings {
    pub providers: TtsProvidersSettings,
    pub fish_dir: String,
    pub fish_voice: Option<String>,
    pub piper_voice: Option<String>,
    pub piper_auto_language: bool,
    pub tts_engine: String,
    pub app_data: Option<PathBuf>,
}

impl CliSettings {
    pub fn from_app(app: &AppHandle) -> Self {
        let s = crate::settings::get_settings(app);
        Self {
            providers: s.tts_providers.clone(),
            fish_dir: s.tts_fish_dir.clone(),
            fish_voice: s.tts_voice.clone(),
            piper_voice: s.tts_piper_voice.clone(),
            piper_auto_language: s.tts_piper_auto_language,
            tts_engine: s.tts_engine.clone(),
            app_data: crate::portable::data_dir()
                .cloned()
                .or_else(|| app.path().app_local_data_dir().ok()),
        }
    }
}

/// Die Argumente, die einen Anbieter betreffen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderArgs {
    pub provider: String,
    pub voice: Option<String>,
    pub exe: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

fn trimmed(v: &Option<String>) -> Option<String> {
    v.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

impl ProviderArgs {
    /// `None`, wenn `--tts-provider` fehlt.
    pub fn from_cli(a: &CliArgs) -> Option<Self> {
        let provider = trimmed(&a.tts_provider)?;
        Some(Self {
            provider,
            voice: trimmed(&a.tts_voice),
            exe: a
                .tts_exe
                .as_ref()
                .map(|p| p.to_string_lossy().trim().to_string())
                .filter(|s| !s.is_empty()),
            model: trimmed(&a.tts_model),
            base_url: trimmed(&a.tts_base_url),
        })
    }
}

/// Überschreibungen nur dort, wo der Anbieter das Feld kennt; sonst eine klare Meldung.
fn overrides_for(
    providers: &TtsProvidersSettings,
    args: &ProviderArgs,
) -> Result<ProviderOverrides, CliError> {
    let one = ProviderOverride {
        exe_path: args.exe.clone(),
        model: args.model.clone(),
        base_url: args.base_url.clone(),
        voice: args.voice.clone(),
    };
    let mut ov = ProviderOverrides::default();
    let id = &args.provider;
    match policy::config_slot(providers, id) {
        Some(ConfigSlot::Phonon) => {
            if args.base_url.is_some() {
                return Err(CliError::bad(
                    "--tts-base-url gilt nicht für phonon (ein Programm, keine Adresse)",
                ));
            }
            ov.phonon = one;
        }
        Some(ConfigSlot::OpenAiSpeech) => {
            if args.exe.is_some() {
                return Err(CliError::bad(format!(
                    "--tts-exe gilt nicht für {id} (ein Server, kein Programm)"
                )));
            }
            ov.openai_speech = one;
        }
        Some(ConfigSlot::OllamaSpeech) => {
            if args.exe.is_some() {
                return Err(CliError::bad(format!(
                    "--tts-exe gilt nicht für {id} (ein Server, kein Programm)"
                )));
            }
            ov.ollama_speech = one;
        }
        None => {
            if args.exe.is_some() || args.model.is_some() || args.base_url.is_some() {
                return Err(CliError::bad(format!(
                    "--tts-exe, --tts-model und --tts-base-url gelten nur für phonon, \
                     openai-speech und ollama-speech, nicht für '{id}'"
                )));
            }
        }
    }
    Ok(ov)
}

/// Gewählter Anbieter aus `tts_engine` (wie `TtsManager::provider_snapshot`).
fn selected_provider(tts_engine: &str) -> String {
    match engine::provider_id_from_setting(tts_engine) {
        Some(id) => id,
        None => match TtsEngineKind::from_setting(tts_engine) {
            TtsEngineKind::Piper => PIPER_ID.to_string(),
            _ => FISH_ID.to_string(),
        },
    }
}

fn paths_for(cs: &CliSettings, piper_voice: Option<String>, auto_language: bool) -> ProviderPaths {
    ProviderPaths {
        fish_dir: PathBuf::from(&cs.fish_dir),
        app_data: cs.app_data.clone(),
        piper_voice,
        piper_auto_language: auto_language,
    }
}

// ---- Plan -----------------------------------------------------------------------------

/// Alles, was ein Lauf über EINEN Anbieter wissen muss, rein berechnet aus
/// Einstellungen, Umgebung und Argumenten.
pub struct Plan {
    pub id: String,
    pub providers: Arc<TtsProvidersSettings>,
    pub resolved: Arc<ResolvedConfigs>,
    pub paths: ProviderPaths,
    /// Wirksame Stimme (Argument vor Umgebung vor Einstellungen).
    pub voice: Option<String>,
    selected: String,
    fish_voice: Option<String>,
    piper_voice: Option<String>,
}

/// Plan für `args.provider`. `for_bench`: Piper erkennt dann die Sprache NICHT je Satz
/// (gemessen wird die verlangte Stimme).
pub fn plan(
    cs: &CliSettings,
    env: &dyn Fn(&str) -> Option<String>,
    args: &ProviderArgs,
    for_bench: bool,
) -> Result<Plan, CliError> {
    let overrides = overrides_for(&cs.providers, args)?;
    let resolved = Arc::new(policy::resolve_provider_configs(
        &cs.providers,
        env,
        &overrides,
    ));
    let id = args.provider.clone();
    let voice = match id.as_str() {
        FISH_ID => args.voice.clone().or_else(|| cs.fish_voice.clone()),
        PIPER_ID => args.voice.clone().or_else(|| cs.piper_voice.clone()),
        other => resolved.voice_for(other),
    };
    let piper_voice = args
        .voice
        .clone()
        .filter(|_| id == PIPER_ID)
        .or_else(|| cs.piper_voice.clone());
    let auto = cs.piper_auto_language && args.voice.is_none() && !for_bench;
    Ok(Plan {
        paths: paths_for(cs, piper_voice.clone(), auto),
        selected: selected_provider(&cs.tts_engine),
        fish_voice: cs.fish_voice.clone(),
        piper_voice,
        id,
        providers: Arc::new(cs.providers.clone()),
        resolved,
        voice,
    })
}

impl Plan {
    /// Registry mit den fünf Anbietern, OHNE Wiederholung (Messung, Probe).
    pub fn registry(&self, synth: Option<&SynthFn>) -> Result<Arc<ProviderRegistry>, TtsError> {
        let registry = bench::bench_registry();
        // Die Prüffunktion der Guardrails gegen die Einstellungen der Datei: der
        // Ollama-Anbieter macht seine Probe nur, wenn die App sie freigegeben hat.
        let gate = LiveApprovals::new((*self.providers).clone()).gate();
        sync_registry_gated(&registry, &self.resolved, &self.paths, synth, Some(&gate))?;
        Ok(Arc::new(registry))
    }

    pub fn snapshot(&self, registry: Arc<ProviderRegistry>) -> ProviderSnapshot {
        ProviderSnapshot {
            settings: self.providers.clone(),
            resolved: self.resolved.clone(),
            registry,
            selected: self.selected.clone(),
            fish_voice: self.fish_voice.clone(),
            piper_voice: self.piper_voice.clone(),
        }
    }

    /// DIE Prüfung vor jeder Nutzung: Anbieter muss bekannt sein und `check_use`
    /// bestehen. Gesperrt = Exit 3 mit der Meldung, was in der App fehlt.
    pub fn authorize(&self) -> Result<ProviderDescriptor, CliError> {
        let registry = self.registry(None).map_err(|e| CliError::from_tts(&e))?;
        let known: Vec<String> = registry.descriptors().into_iter().map(|d| d.id).collect();
        if !known.contains(&self.id) {
            return Err(CliError::bad(format!(
                "Unbekannter Anbieter '{}'. Bekannt: {}",
                self.id,
                known.join(", ")
            )));
        }
        self.snapshot(registry)
            .checked(&self.id)
            .map_err(|e| CliError::from_tts(&e))
    }

    /// Modellname bzw. -datei für `env.json` (nur, was sich belegen lässt).
    pub fn model_label(&self) -> Option<String> {
        match self.id.as_str() {
            PIPER_ID => {
                let voice = self.voice.clone()?;
                let file = self.paths.app_data.as_deref().and_then(|dir| {
                    piper::installed_voices(dir)
                        .into_iter()
                        .find(|(id, _, _)| *id == voice)
                        .and_then(|(_, _, p)| {
                            p.model.file_name().map(|n| n.to_string_lossy().to_string())
                        })
                });
                Some(file.unwrap_or(voice))
            }
            FISH_ID => self
                .paths
                .fish_dir
                .join("checkpoints")
                .join("s2-pro")
                .is_dir()
                .then(|| "s2-pro".to_string()),
            other => Some(self.resolved.subject(other).model).filter(|m| !m.is_empty()),
        }
    }
}

/// Hinweise für `env.json`, die für den Anbieter immer gelten.
fn static_notes(id: &str) -> Vec<String> {
    match id {
        FISH_ID => Vec::new(),
        PIPER_ID | PHONON_ID => vec![
            "Der Anbieter startet je Satz einen eigenen Prozess: Kalt- und Warmläufe \
             unterscheiden sich nur durch den Dateicache, process_start_ms ist nicht messbar."
                .to_string(),
        ],
        _ => vec![
            "Externer Server: ein Kaltlauf baut den Anbieter und die Verbindung neu auf, \
             startet den Server aber nicht neu; process_start_ms ist nicht messbar."
                .to_string(),
        ],
    }
}

/// Snapshot ohne Overrides für die Anbieterliste (Fish ohne Synthese: Gesundheit ist nur Dateisystem).
pub fn listing_snapshot(
    cs: &CliSettings,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<ProviderSnapshot, TtsError> {
    let resolved = Arc::new(policy::resolve_provider_configs(
        &cs.providers,
        env,
        &ProviderOverrides::default(),
    ));
    let paths = paths_for(cs, cs.piper_voice.clone(), cs.piper_auto_language);
    let registry = Arc::new(bench::bench_registry());
    let gate = LiveApprovals::new(cs.providers.clone()).gate();
    sync_registry_gated(&registry, &resolved, &paths, None, Some(&gate))?;
    Ok(ProviderSnapshot {
        settings: Arc::new(cs.providers.clone()),
        resolved,
        registry,
        selected: selected_provider(&cs.tts_engine),
        fish_voice: cs.fish_voice.clone(),
        piper_voice: cs.piper_voice.clone(),
    })
}

// ---- Ausgabe (rein) -----------------------------------------------------------------------

fn tier_label(t: ProviderTier) -> &'static str {
    match t {
        ProviderTier::Supported => "Standard",
        ProviderTier::Experimental => "Experimentell",
        ProviderTier::BenchmarkOnly => "Nur Benchmark",
        ProviderTier::Unavailable => "Nicht verfügbar",
    }
}

fn streaming_label(s: StreamingMode) -> &'static str {
    match s {
        StreamingMode::None => "keins",
        StreamingMode::Sentence => "je Satz",
        StreamingMode::Chunk => "Teilstücke",
    }
}

fn device_label(d: Device) -> &'static str {
    match d {
        Device::Cpu => "cpu",
        Device::Cuda => "cuda",
        Device::Metal => "metal",
        Device::Remote => "remote",
    }
}

/// Kurze deutsche Liste der Anbieter.
pub fn render_providers_text(entries: &[TtsProviderEntry]) -> String {
    let selected = entries
        .iter()
        .find(|e| e.selected)
        .map(|e| e.descriptor.id.as_str())
        .unwrap_or("-");
    let mut out = format!(
        "Sprachausgabe-Anbieter ({}), gewählt: {selected}\n",
        entries.len()
    );
    for e in entries {
        let d = &e.descriptor;
        let health = if e.health.available {
            "bereit".to_string()
        } else {
            format!(
                "nicht bereit: {}",
                e.health.reason.as_deref().unwrap_or("unbekannter Grund")
            )
        };
        out.push_str(&format!(
            "\n{}  {}  [{}]  {}\n",
            d.id,
            d.label,
            tier_label(d.tier),
            health
        ));
        let devices: Vec<&str> = d.caps.devices.iter().map(|x| device_label(*x)).collect();
        out.push_str(&format!(
            "    Sprachen: {} | Streaming: {} | Geräte: {} | Klonen: {}\n",
            d.caps.languages.join(", "),
            streaming_label(d.caps.streaming),
            devices.join(", "),
            if d.caps.cloning { "ja" } else { "nein" }
        ));
        out.push_str(&format!(
            "    Lizenz: Code {}, Gewichte {}{}{}\n",
            d.license.code,
            d.license.weights,
            if d.license.non_commercial {
                ", nichtkommerziell"
            } else {
                ""
            },
            if d.license.verified {
                ", geprüft"
            } else {
                ", nicht geprüft"
            }
        ));
        let release = match &e.blocked {
            None => "frei".to_string(),
            Some(b) => format!("gesperrt ({}): {}", b.kind.as_str(), b.message),
        };
        out.push_str(&format!(
            "    Freigabe: {release} [aktiv: {}, Lizenz bestätigt: {}, Fernzugriff erlaubt: {}]\n",
            yes_no(e.enabled),
            if e.license_needs_ack {
                yes_no(e.license_acked)
            } else {
                "nicht nötig"
            },
            yes_no(e.allow_remote)
        ));
    }
    out
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "ja"
    } else {
        "nein"
    }
}

/// Stimmenliste als Text.
pub fn render_voices_text(provider: &str, voices: &[VoiceDescriptor]) -> String {
    if voices.is_empty() {
        return format!("{provider}: keine Stimmen gefunden.\n");
    }
    let mut out = format!("{provider}: {} Stimme(n)\n", voices.len());
    let width = voices
        .iter()
        .map(|v| v.id.chars().count())
        .max()
        .unwrap_or(0);
    for v in voices {
        out.push_str(&format!(
            "  {:<width$}  {:<6}  {}{}\n",
            v.id,
            v.language,
            v.label,
            v.gender
                .as_deref()
                .map(|g| format!(" ({g})"))
                .unwrap_or_default(),
            width = width
        ));
    }
    out
}

// ---- Betriebsarten ---------------------------------------------------------------------

/// Welche Betriebsarten verlangt sind. `--tts-test` zählt nur mit `--tts-provider`
/// (ohne ihn bleibt das bisherige Fish-Verhalten unberührt).
pub fn requested_modes(a: &CliArgs) -> Vec<&'static str> {
    let mut modes = Vec::new();
    if a.tts_providers {
        modes.push("--tts-providers");
    }
    if a.tts_voices {
        modes.push("--tts-voices");
    }
    if a.tts_bench {
        modes.push("--tts-bench");
    }
    if a.tts_test && (!modes.is_empty() || a.tts_provider.is_some()) {
        modes.push("--tts-test");
    }
    modes
}

/// Führt die Anbieter-Betriebsarten aus. `None`: nichts davon verlangt — das
/// bisherige `--tts-test` (Fish) läuft unverändert weiter.
pub fn run_provider_modes(app: &AppHandle, args: &CliArgs) -> Option<i32> {
    let modes = requested_modes(args);
    let first = *modes.first()?;
    if modes.len() > 1 {
        eprintln!(
            "error: {} lassen sich nicht kombinieren",
            modes.join(" und ")
        );
        return Some(EXIT_BAD_INPUT);
    }
    let result = match first {
        "--tts-providers" => run_providers(app, args),
        "--tts-voices" => run_voices(app, args),
        "--tts-bench" => run_bench(app, args),
        _ => run_test(app, args),
    };
    Some(match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {}", e.message);
            e.code
        }
    })
}

/// Nur in die Datei (`--out` ohne `--json`).
fn write_json_file(payload: &Value, path: &Path) {
    match std::fs::write(
        path,
        serde_json::to_string_pretty(payload).unwrap_or_default(),
    ) {
        Ok(()) => eprintln!("wrote {}", path.display()),
        Err(e) => eprintln!("error: could not write {}: {e}", path.display()),
    }
}

fn run_providers(app: &AppHandle, args: &CliArgs) -> Result<i32, CliError> {
    let cs = CliSettings::from_app(app);
    let snapshot =
        listing_snapshot(&cs, &policy::process_env).map_err(|e| CliError::from_tts(&e))?;
    let health = tauri::async_runtime::block_on(snapshot.registry.health_all(HEALTH_LIMIT));
    let entries = build_entries(&snapshot, health);
    let payload = serde_json::to_value(&entries)
        .map_err(|e| CliError::failed(format!("Liste nicht darstellbar: {e}")))?;
    if args.json {
        crate::emit_headless_payload(&payload, args.out.as_deref());
    } else {
        print!("{}", render_providers_text(&entries));
        if let Some(path) = args.out.as_deref() {
            write_json_file(&payload, path);
        }
    }
    Ok(EXIT_OK)
}

fn require_provider(args: &CliArgs, mode: &str) -> Result<ProviderArgs, CliError> {
    ProviderArgs::from_cli(args)
        .ok_or_else(|| CliError::bad(format!("{mode} braucht --tts-provider <id>")))
}

fn run_voices(app: &AppHandle, args: &CliArgs) -> Result<i32, CliError> {
    let pargs = require_provider(args, "--tts-voices")?;
    let cs = CliSettings::from_app(app);
    let plan = plan(&cs, &policy::process_env, &pargs, false)?;
    plan.authorize()?;
    let registry = plan.registry(None).map_err(|e| CliError::from_tts(&e))?;
    let provider = registry
        .get(&plan.id)
        .ok_or_else(|| CliError::bad(format!("Unbekannter Anbieter '{}'", plan.id)))?;
    let language = trimmed(&args.tts_lang);
    let listed = tauri::async_runtime::block_on(async {
        tokio::time::timeout(VOICES_LIMIT, provider.list_voices(language.as_deref())).await
    });
    let voices = match listed {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(CliError::from_tts(&e)),
        Err(_) => {
            return Err(CliError::failed(format!(
                "Die Stimmenliste kam nicht innerhalb von {} s",
                VOICES_LIMIT.as_secs()
            )))
        }
    };
    if args.json {
        let payload = serde_json::to_value(&voices)
            .map_err(|e| CliError::failed(format!("Liste nicht darstellbar: {e}")))?;
        crate::emit_headless_payload(&payload, args.out.as_deref());
    } else {
        print!("{}", render_voices_text(&plan.id, &voices));
        if let Some(path) = args.out.as_deref() {
            if let Ok(payload) = serde_json::to_value(&voices) {
                write_json_file(&payload, path);
            }
        }
    }
    Ok(EXIT_OK)
}

// ---- Anbindung an die App (Fish braucht den TtsManager) ----------------------------------

/// Anbindung der Messschleife an die App. Nur Fish braucht den `TtsManager`
/// (Server starten, HTTP-Weg des Kerns); alle anderen laufen ohne ihn.
enum Backend {
    Plain(Arc<Plan>),
    Fish {
        plan: Arc<Plan>,
        tts: Arc<TtsManager>,
    },
}

impl Backend {
    fn new(app: &AppHandle, plan: Arc<Plan>) -> Self {
        if plan.id == FISH_ID {
            Self::Fish {
                tts: TtsManager::new(app),
                plan,
            }
        } else {
            Self::Plain(plan)
        }
    }
}

impl BenchBackend for Backend {
    fn fresh_registry(&self) -> Result<Arc<ProviderRegistry>, TtsError> {
        match self {
            Self::Plain(plan) => plan.registry(None),
            // Ohne `refresh_from_settings`/`ensure_server` je Satz: beides macht
            // `prepare` bzw. `cold_reset` einmal VOR der gemessenen Zeit.
            Self::Fish { plan, tts } => plan.registry(Some(&tts.fish_bench_synth_fn())),
        }
    }

    fn cold_reset(&self) -> BoxFuture<'_, Result<ColdReset, TtsError>> {
        match self {
            Self::Plain(_) => Box::pin(async { Ok(ColdReset::rebuilt()) }),
            Self::Fish { tts, .. } => Box::pin(tts.bench_cold_reset()),
        }
    }

    fn prepare(&self) -> BoxFuture<'_, Result<(), TtsError>> {
        match self {
            Self::Plain(_) => Box::pin(async { Ok(()) }),
            Self::Fish { tts, .. } => Box::pin(tts.bench_prepare()),
        }
    }
}

impl Drop for Backend {
    /// Nur ein SELBST gestarteter Server wird beendet, nie ein fremder.
    fn drop(&mut self) {
        if let Self::Fish { tts, .. } = self {
            tts.stop_owned_server();
        }
    }
}

fn run_test(app: &AppHandle, args: &CliArgs) -> Result<i32, CliError> {
    let pargs = require_provider(args, "--tts-test")?;
    let cs = CliSettings::from_app(app);
    let plan = Arc::new(plan(&cs, &policy::process_env, &pargs, false)?);
    let descriptor = plan.authorize()?;
    let text = trimmed(&args.tts_text).unwrap_or_else(|| DEFAULT_TEST_TEXT.to_string());
    let language = trimmed(&args.tts_lang)
        .or_else(|| crate::managers::tts::lang::detect_language(&text).map(String::from))
        .unwrap_or_else(|| "de".to_string());
    let backend = Backend::new(app, plan.clone());
    // Bereit machen (Fish: den Server starten) — die Startzeit gehört nicht zur Synthese.
    let reset =
        tauri::async_runtime::block_on(backend.cold_reset()).map_err(|e| CliError::from_tts(&e))?;
    let registry = backend
        .fresh_registry()
        .map_err(|e| CliError::from_tts(&e))?;
    let health = tauri::async_runtime::block_on(registry.health(&plan.id, bench::HEALTH_TIMEOUT))
        .map_err(|e| CliError::from_tts(&e))?;
    if !health.available {
        return Err(CliError::failed(format!(
            "{} ist nicht bereit: {}",
            descriptor.label,
            health
                .reason
                .unwrap_or_else(|| "nicht verfügbar".to_string())
        )));
    }
    let opts = SynthesisOptions {
        language: language.clone(),
        voice: plan.voice.clone(),
        format: bench::pick_format(&descriptor.caps),
        speed: None,
        seed: None,
        timeout: TEST_TIMEOUT,
    };
    let measured = tauri::async_runtime::block_on(bench::measure_once(
        &registry,
        &plan.id,
        &text,
        &opts,
        true,
        bench::MAX_AUDIO_BYTES,
    ))
    .map_err(|e| CliError::from_tts(&e))?;
    let wav = measured
        .audio
        .as_ref()
        .ok_or_else(|| CliError::failed("Kein Audio zusammengesetzt"))?
        .to_wav()
        .map_err(|e| CliError::from_tts(&e))?;
    if let Some(path) = args.tts_out_wav.as_deref() {
        match std::fs::write(path, &wav) {
            Ok(()) => eprintln!("wrote {}", path.display()),
            Err(e) => eprintln!("error: could not write {}: {e}", path.display()),
        }
    }
    let server_start_ms = reset.process_start_ms.unwrap_or(0.0).round() as u64;
    let tts_ms = measured.total_ms.round() as u64;
    let payload = json!({
        "mode": "tts",
        "provider": plan.id,
        "voice": plan.voice,
        "language": language,
        "wav_bytes": wav.len(),
        "audio_ms": measured.audio_ms.round() as u64,
        "sample_rate": measured.sample_rate,
        "chunk_count": measured.chunk_count,
        "server_start_ms": server_start_ms,
        "ttfa_ms": measured.ttfa_ms.round() as u64,
        "tts_ms": tts_ms,
    });
    if args.json {
        crate::emit_headless_payload(&payload, args.out.as_deref());
    } else {
        if let Some(path) = args.out.as_deref() {
            write_json_file(&payload, path);
        }
        println!(
            "tts ok: {} bytes, provider={}, server_start={}ms, ttfa={}ms, tts={}ms",
            wav.len(),
            plan.id,
            server_start_ms,
            measured.ttfa_ms.round() as u64,
            tts_ms
        );
    }
    Ok(EXIT_OK)
}

fn run_bench(app: &AppHandle, args: &CliArgs) -> Result<i32, CliError> {
    let pargs = require_provider(args, "--tts-bench")?;
    let corpus = args
        .bench_corpus
        .clone()
        .ok_or_else(|| CliError::bad("--tts-bench braucht --bench-corpus <jsonl>"))?;
    let out_dir = args
        .bench_out
        .clone()
        .ok_or_else(|| CliError::bad("--tts-bench braucht --bench-out <ordner>"))?;
    let smoke = match trimmed(&args.bench_subset).as_deref() {
        None | Some("full") => false,
        Some("smoke") => true,
        Some(other) => {
            return Err(CliError::bad(format!(
                "Unbekannte Teilmenge '{other}' (erlaubt: smoke, full)"
            )))
        }
    };
    let items = bench::load_corpus(&corpus, smoke).map_err(CliError::bad)?;
    let cs = CliSettings::from_app(app);
    let plan = Arc::new(plan(&cs, &policy::process_env, &pargs, true)?);
    // Die Sperren gelten auch hier — vor jedem Prozess- und Serverstart.
    plan.authorize()?;

    let mut opts = BenchOptions::new(
        &plan.id,
        &trimmed(&args.tts_lang).unwrap_or_else(|| "de".to_string()),
        out_dir,
    );
    opts.voice = plan.voice.clone();
    opts.model = plan.model_label();
    opts.warm = args.bench_warm.unwrap_or(opts.warm);
    opts.cold = args.bench_cold.unwrap_or(opts.cold);
    opts.notes = static_notes(&plan.id);

    let backend = Backend::new(app, plan.clone());
    let cancel = CancelToken::new("bench-cli");
    install_ctrl_c(cancel.clone());
    let result = tauri::async_runtime::block_on(bench::run_bench(
        &backend,
        &opts,
        &items,
        &cancel,
        &|line| eprintln!("{line}"),
    ));
    // Eigenen Fish-Server beenden, bevor der Prozess endet.
    drop(backend);

    let summary = match result {
        Ok(s) => s,
        Err(BenchError::BadInput(m)) => return Err(CliError::bad(m)),
        Err(BenchError::Unavailable(m)) | Err(BenchError::Io(m)) => {
            return Err(CliError::failed(m))
        }
    };
    let payload = summary.to_json();
    if args.json {
        crate::emit_headless_payload(&payload, args.out.as_deref());
    } else if let Some(path) = args.out.as_deref() {
        write_json_file(&payload, path);
    }
    eprintln!(
        "Benchmark {}: {} von {} Zeilen geschrieben ({} ok, {} fehlgeschlagen), {} WAV(s), Ordner {}",
        summary.provider,
        summary.rows_written,
        summary.rows_expected,
        summary.rows_ok,
        summary.rows_failed,
        summary.wavs_written,
        summary.out_dir.display()
    );
    if summary.sink_stalls > 0 {
        eprintln!(
            "Hinweis: der Stream-Kanal war {} Mal voll (Rückstau).",
            summary.sink_stalls
        );
    }
    Ok(match &summary.outcome {
        Outcome::Complete if summary.rows_written > 0 => EXIT_OK,
        Outcome::Complete => {
            eprintln!("error: es wurde keine Zeile geschrieben");
            EXIT_ERROR
        }
        Outcome::Interrupted => {
            eprintln!("Mit Strg+C abgebrochen; die geschriebenen Zeilen bleiben.");
            EXIT_INTERRUPTED
        }
        Outcome::Aborted(reason) => {
            eprintln!("Abgebrochen: {reason}; die geschriebenen Zeilen bleiben.");
            EXIT_INCOMPLETE
        }
    })
}

// ---- Strg+C --------------------------------------------------------------------------------

/// Meldet Strg+C/Strg+Untbr/Schließen der Konsole an `token` (siehe Moduldoku).
#[cfg(windows)]
fn install_ctrl_c(token: CancelToken) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;

    static TOKEN: OnceLock<CancelToken> = OnceLock::new();
    static FIRED: AtomicBool = AtomicBool::new(false);
    type HandlerRoutine = unsafe extern "system" fn(u32) -> i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<HandlerRoutine>, add: i32) -> i32;
    }

    /// CTRL_C_EVENT 0, CTRL_BREAK_EVENT 1, CTRL_CLOSE_EVENT 2.
    unsafe extern "system" fn handler(kind: u32) -> i32 {
        if kind > 2 {
            return 0;
        }
        if let Some(t) = TOKEN.get() {
            t.cancel();
        }
        if !FIRED.swap(true, Ordering::SeqCst) {
            // Notbremse: räumt der Lauf nicht auf, endet der Prozess trotzdem.
            std::thread::spawn(|| {
                std::thread::sleep(INTERRUPT_GRACE);
                std::process::exit(EXIT_INTERRUPTED);
            });
        }
        1
    }

    if TOKEN.set(token).is_ok() {
        // SAFETY: `handler` ist eine gültige HandlerRoutine und lebt für die Prozessdauer.
        unsafe {
            SetConsoleCtrlHandler(Some(handler), 1);
        }
    }
}

#[cfg(not(windows))]
fn install_ctrl_c(_token: CancelToken) {}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn cs() -> CliSettings {
        CliSettings {
            fish_dir: "C:/gibt/es/nicht".to_string(),
            fish_voice: Some("patrick".to_string()),
            piper_voice: Some("de_DE-thorsten-high".to_string()),
            tts_engine: "fish".to_string(),
            ..Default::default()
        }
    }

    fn args(provider: &str) -> ProviderArgs {
        ProviderArgs {
            provider: provider.to_string(),
            ..Default::default()
        }
    }

    // ---- Sperren -----------------------------------------------------------------------------

    #[test]
    fn ein_gesperrter_anbieter_endet_mit_exit_3_und_nennt_die_fehlende_freigabe() {
        let plan = plan(&cs(), &no_env, &args("phonon"), true).unwrap();
        let err = plan.authorize().unwrap_err();
        assert_eq!(err.code, EXIT_BLOCKED);
        assert!(err.message.contains("license_blocked"), "{}", err.message);
        assert!(
            err.message.contains("nicht freigeschaltet"),
            "{}",
            err.message
        );
        assert!(err.message.contains("nur die App"), "{}", err.message);
    }

    #[test]
    fn ohne_fernzugriff_bleibt_auch_ein_ueberschriebener_server_gesperrt() {
        let mut s = cs();
        s.providers.set_enabled("openai-speech", true);
        // Lizenz zunächst offen, dann (unten) bestätigt: der Fernzugriff fehlt weiter.
        let mut a = args("openai-speech");
        a.base_url = Some("https://tts.example.org/v1".to_string());
        a.model = Some("kokoro".to_string());
        let p = plan(&s, &no_env, &a, true).unwrap();
        assert_eq!(p.authorize().unwrap_err().code, EXIT_BLOCKED);
        let desc = p
            .snapshot(p.registry(None).unwrap())
            .descriptor("openai-speech")
            .unwrap();
        s.providers.record_ack(
            "openai-speech",
            &policy::license_fingerprint(&desc, &p.resolved.subject("openai-speech")),
            "t",
        );
        let p = plan(&s, &no_env, &a, true).unwrap();
        let err = p.authorize().unwrap_err();
        assert_eq!(err.code, EXIT_BLOCKED);
        assert!(err.message.contains("privacy_blocked"), "{}", err.message);
        // Mit der Erlaubnis der App läuft es.
        s.providers.set_remote_allowed("openai-speech", true);
        assert!(plan(&s, &no_env, &a, true).unwrap().authorize().is_ok());
    }

    #[test]
    fn eingebaute_anbieter_sind_nie_gesperrt() {
        for id in ["fish", "piper"] {
            let p = plan(&cs(), &no_env, &args(id), true).unwrap();
            assert_eq!(p.authorize().unwrap().id, id);
        }
    }

    #[test]
    fn ein_unbekannter_anbieter_ist_eine_falsche_eingabe_und_nennt_die_bekannten() {
        let err = plan(&cs(), &no_env, &args("gibt-es-nicht"), true)
            .unwrap()
            .authorize()
            .unwrap_err();
        assert_eq!(err.code, EXIT_BAD_INPUT);
        for id in ["fish", "piper", "phonon", "openai-speech", "ollama-speech"] {
            assert!(err.message.contains(id), "{}", err.message);
        }
    }

    #[test]
    fn der_plan_aendert_die_einstellungen_nie() {
        let mut s = cs();
        s.providers.set_enabled("phonon", true);
        let before = s.providers.clone();
        let mut a = args("phonon");
        a.exe = Some("C:/x/ptts.exe".to_string());
        a.model = Some("m".to_string());
        let p = plan(&s, &no_env, &a, true).unwrap();
        assert_eq!(
            *p.providers, before,
            "Freigaben und Konfiguration bleiben unberührt"
        );
        let _ = p.authorize();
        assert_eq!(*p.providers, before);
    }

    #[test]
    fn die_cli_hat_keinen_schalter_fuer_freigaben() {
        let cmd = CliArgs::command();
        let tts: Vec<String> = cmd
            .get_arguments()
            .map(|a| a.get_id().to_string())
            .filter(|id| id.starts_with("tts_") || id.starts_with("bench_"))
            .collect();
        let allowed = [
            "tts_test",
            "tts_text",
            "tts_voice",
            "tts_out_wav",
            "tts_providers",
            "tts_voices",
            "tts_provider",
            "tts_lang",
            "tts_exe",
            "tts_model",
            "tts_base_url",
            "tts_bench",
            "bench_corpus",
            "bench_subset",
            "bench_warm",
            "bench_cold",
            "bench_out",
            // Vorbestehend (M4, Suchindex-Messung), kein TTS-Schalter.
            "bench_search",
            "bench_dir",
        ];
        for id in &tts {
            assert!(
                allowed.contains(&id.as_str()),
                "neuer TTS-Schalter '{id}': bitte prüfen, dass er keine Freigabe setzt, und hier eintragen"
            );
        }
        for id in &tts {
            for bad in [
                "enable", "allow", "license", "ack", "remote", "opt_in", "optin",
            ] {
                assert!(!id.contains(bad), "Schalter '{id}' sähe nach Freigabe aus");
            }
        }
        assert!(tts.contains(&"tts_provider".to_string()));
    }

    // ---- Vorrang und Überschreibungen -------------------------------------------------------

    #[test]
    fn argument_schlaegt_umgebung_schlaegt_einstellungen() {
        let mut s = cs();
        s.providers.phonon.exe_path = "C:/einstellungen/ptts.exe".to_string();
        let env =
            |name: &str| (name == "LVA_TTS_PHONON_EXE_PATH").then(|| "C:/env/ptts.exe".to_string());
        // Umgebung vor Einstellungen
        let p = plan(&s, &env, &args("phonon"), true).unwrap();
        assert_eq!(p.resolved.phonon.exe_path, "C:/env/ptts.exe");
        assert_eq!(
            p.resolved.source("phonon", "exe_path"),
            policy::ConfigSource::Env
        );
        // Argument vor Umgebung
        let mut a = args("phonon");
        a.exe = Some("C:/argument/ptts.exe".to_string());
        let p = plan(&s, &env, &a, true).unwrap();
        assert_eq!(p.resolved.phonon.exe_path, "C:/argument/ptts.exe");
        assert_eq!(
            p.resolved.source("phonon", "exe_path"),
            policy::ConfigSource::Override
        );
        // Einstellungen allein
        let p = plan(&s, &no_env, &args("phonon"), true).unwrap();
        assert_eq!(p.resolved.phonon.exe_path, "C:/einstellungen/ptts.exe");
    }

    #[test]
    fn modell_adresse_und_stimme_werden_je_anbieter_ueberschrieben() {
        let mut a = args("ollama-speech");
        a.model = Some("phonon:de".to_string());
        a.base_url = Some("http://127.0.0.1:11435".to_string());
        a.voice = Some("Ava".to_string());
        let p = plan(&cs(), &no_env, &a, true).unwrap();
        assert_eq!(p.resolved.ollama_speech.model, "phonon:de");
        assert_eq!(p.resolved.ollama_speech.base_url, "http://127.0.0.1:11435");
        assert_eq!(p.voice.as_deref(), Some("Ava"));
        assert_eq!(p.resolved.subject("ollama-speech").model, "phonon:de");
    }

    #[test]
    fn felder_die_ein_anbieter_nicht_kennt_werden_abgelehnt() {
        let mut a = args("phonon");
        a.base_url = Some("http://x".to_string());
        assert_eq!(
            plan(&cs(), &no_env, &a, true).err().unwrap().code,
            EXIT_BAD_INPUT
        );
        let mut a = args("openai-speech");
        a.exe = Some("C:/x.exe".to_string());
        assert_eq!(
            plan(&cs(), &no_env, &a, true).err().unwrap().code,
            EXIT_BAD_INPUT
        );
        for id in ["piper", "fish", "gibt-es-nicht"] {
            let mut a = args(id);
            a.model = Some("m".to_string());
            let err = plan(&cs(), &no_env, &a, true).err().unwrap();
            assert_eq!(err.code, EXIT_BAD_INPUT, "{id}");
            assert!(err.message.contains("--tts-model"), "{}", err.message);
        }
    }

    #[test]
    fn die_stimme_folgt_argument_vor_einstellung() {
        let p = plan(&cs(), &no_env, &args("fish"), true).unwrap();
        assert_eq!(p.voice.as_deref(), Some("patrick"));
        let mut a = args("fish");
        a.voice = Some("anna".to_string());
        assert_eq!(
            plan(&cs(), &no_env, &a, true).unwrap().voice.as_deref(),
            Some("anna")
        );
        let p = plan(&cs(), &no_env, &args("piper"), true).unwrap();
        assert_eq!(p.voice.as_deref(), Some("de_DE-thorsten-high"));
        let mut a = args("piper");
        a.voice = Some("de_DE-kerstin-low".to_string());
        let p = plan(&cs(), &no_env, &a, true).unwrap();
        assert_eq!(p.voice.as_deref(), Some("de_DE-kerstin-low"));
        assert_eq!(p.paths.piper_voice.as_deref(), Some("de_DE-kerstin-low"));
    }

    #[test]
    fn piper_erkennt_die_sprache_im_benchmark_nie_je_satz() {
        let mut s = cs();
        s.piper_auto_language = true;
        assert!(
            !plan(&s, &no_env, &args("piper"), true)
                .unwrap()
                .paths
                .piper_auto_language
        );
        assert!(
            plan(&s, &no_env, &args("piper"), false)
                .unwrap()
                .paths
                .piper_auto_language
        );
        let mut a = args("piper");
        a.voice = Some("de_DE-kerstin-low".to_string());
        assert!(
            !plan(&s, &no_env, &a, false)
                .unwrap()
                .paths
                .piper_auto_language
        );
    }

    // ---- Liste ---------------------------------------------------------------------------------

    #[test]
    fn die_liste_nennt_mindestens_fuenf_anbieter_als_json_und_als_text() {
        let snapshot = listing_snapshot(&cs(), &no_env).unwrap();
        let entries = build_entries(&snapshot, Vec::new());
        let json = serde_json::to_value(&entries).unwrap();
        let list = json.as_array().unwrap();
        assert!(list.len() >= 5, "{}", list.len());
        for e in list {
            for key in [
                "descriptor",
                "health",
                "enabled",
                "license_needs_ack",
                "license_acked",
                "blocked",
            ] {
                assert!(e.get(key).is_some(), "Feld {key} fehlt");
            }
            assert!(e["descriptor"]["tier"].is_string());
            assert!(e["descriptor"]["caps"]["languages"].is_array());
            assert!(e["descriptor"]["license"]["code"].is_string());
        }
        let text = render_providers_text(&entries);
        for id in ["fish", "piper", "phonon", "openai-speech", "ollama-speech"] {
            assert!(text.contains(id), "{id} fehlt im Text");
        }
        assert!(text.contains("gewählt: fish"), "{text}");
        assert!(text.contains("gesperrt (license_blocked)"), "{text}");
        assert!(text.contains("Experimentell") && text.contains("Standard"));
        assert!(text.contains("nicht bereit"), "{text}");
    }

    #[test]
    fn die_stimmenliste_als_text() {
        assert!(render_voices_text("piper", &[]).contains("keine Stimmen"));
        let voices = vec![VoiceDescriptor {
            id: "de_DE-thorsten-high".into(),
            label: "Thorsten".into(),
            language: "de".into(),
            gender: Some("m".into()),
            license: None,
        }];
        let t = render_voices_text("piper", &voices);
        assert!(
            t.contains("de_DE-thorsten-high") && t.contains("Thorsten (m)"),
            "{t}"
        );
    }

    // ---- Betriebsarten und Exit-Codes ----------------------------------------------------------

    #[test]
    fn das_bisherige_tts_test_bleibt_ohne_anbieter_unberuehrt() {
        let a = CliArgs {
            tts_test: true,
            ..Default::default()
        };
        assert!(
            requested_modes(&a).is_empty(),
            "kein --tts-provider: Fish wie bisher"
        );
        let a = CliArgs {
            tts_test: true,
            tts_provider: Some("piper".into()),
            ..Default::default()
        };
        assert_eq!(requested_modes(&a), ["--tts-test"]);
    }

    #[test]
    fn betriebsarten_lassen_sich_nicht_mischen() {
        let a = CliArgs {
            tts_providers: true,
            tts_bench: true,
            ..Default::default()
        };
        assert_eq!(requested_modes(&a), ["--tts-providers", "--tts-bench"]);
        let a = CliArgs {
            tts_test: true,
            tts_voices: true,
            ..Default::default()
        };
        assert_eq!(requested_modes(&a), ["--tts-voices", "--tts-test"]);
    }

    #[test]
    fn exit_codes_folgen_der_fehlerklasse() {
        assert_eq!(exit_code_for(TtsErrorKind::LicenseBlocked), EXIT_BLOCKED);
        assert_eq!(exit_code_for(TtsErrorKind::PrivacyBlocked), EXIT_BLOCKED);
        assert_eq!(exit_code_for(TtsErrorKind::BadRequest), EXIT_BAD_INPUT);
        assert_eq!(exit_code_for(TtsErrorKind::InvalidConfig), EXIT_BAD_INPUT);
        for k in [
            TtsErrorKind::Unavailable,
            TtsErrorKind::Timeout,
            TtsErrorKind::Backend,
            TtsErrorKind::Network,
            TtsErrorKind::Cancelled,
        ] {
            assert_eq!(exit_code_for(k), EXIT_ERROR, "{k:?}");
        }
    }

    #[test]
    fn die_argumente_werden_gesaeubert() {
        let a = CliArgs {
            tts_provider: Some("  piper ".into()),
            tts_voice: Some("  ".into()),
            tts_model: Some(" m ".into()),
            ..Default::default()
        };
        let p = ProviderArgs::from_cli(&a).unwrap();
        assert_eq!(p.provider, "piper");
        assert_eq!(p.voice, None);
        assert_eq!(p.model.as_deref(), Some("m"));
        assert!(ProviderArgs::from_cli(&CliArgs::default()).is_none());
    }

    #[test]
    fn statische_hinweise_erklaeren_was_kalt_bedeutet() {
        assert!(static_notes("fish").is_empty());
        assert!(static_notes("piper")[0].contains("Dateicache"));
        assert!(static_notes("openai-speech")[0].contains("Externer Server"));
    }
}
