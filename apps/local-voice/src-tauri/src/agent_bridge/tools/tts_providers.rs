//! Die Werkzeuge der Sprachausgabe-Anbieter (Goal tts-provider, Paket P3b): ein externer
//! Agent liest Anbieter, Fähigkeiten, Stimmen und Gesundheit, wählt Anbieter und Stimme fürs
//! Vorlesen und liest die Ergebnisse der Benchmarks. Rechte, Freigaben und Audit erledigt die
//! Brücke (A7) VOR dem Aufruf; die Handler hier setzen um und prüfen ihre Eingaben selbst.
//!
//! | Werkzeug | Recht | Wirkung |
//! |---|---|---|
//! | `tts_providers_list` | `tts.read` | alle Anbieter mit Stufe, Fähigkeiten, Lizenz, Gesundheit (mit Frist), Freigabestand und Sperrgrund |
//! | `tts_provider_capabilities` | `tts.read` | Fähigkeiten, Lizenz und Freigabestand EINES Anbieters |
//! | `tts_provider_voices` | `tts.read` | Stimmen eines Anbieters (gilt als Nutzung: gesperrt = Sperrgrund) |
//! | `tts_provider_health` | `tts.read` | Gesundheit eines Anbieters mit Frist |
//! | `tts_provider_select` | `tts.render` | wählt Anbieter und Stimme fürs Vorlesen, nur wenn `check_use` es erlaubt |
//! | `tts_benchmark_results` | `tts.read` | listet die Zusammenfassungen oder liefert eine `results.json` |
//!
//! # Keine Freigaben über die Brücke
//!
//! Opt-in, Lizenz-Zustimmung und Fernzugriff eines Anbieters setzt NUR der Nutzer in der App
//! (`commands::tts_providers`, Moduldoku). Dieses Modul hat dafür weder ein Werkzeug noch ein
//! Argument, und der Zugriff auf die App ([`TtsProviderAccess`]) kennt keine solche Methode: er
//! kann lesen und Anbieter/Stimme wählen — mehr nicht. `tts_provider_select` prüft die
//! Guardrails (`check_use`) zweimal: vor der Stimmenliste (gegen die Momentaufnahme) und noch
//! einmal beim Schreiben der Einstellungen: `select_in` prüft unter der Schreibsperre gegen den
//! Stand dieses Moments und löst Modell und Adresse aus genau diesem Stand NEU auf — ein
//! zwischenzeitlicher Widerruf oder Modellwechsel lässt die Wahl scheitern (Test
//! `select_prueft_den_stand_im_moment_des_schreibens…` in `commands::tts_providers`). Auch
//! `tts_render_audio` hält die Sperren ein: bei `tts_engine = provider:<id>` läuft jeder Satz
//! durch `ProviderEngine::guard`, der den gemeinsamen, aktuellen Stand der Freigaben liest
//! (siehe `managers::tts::providers::pipeline`).
//!
//! # Fehlerfälle und ihre Absicherung (Tests in `tts_providers/tests.rs`)
//!
//! | # | Fehlerfall | Verhalten | Absicherung |
//! |---|---|---|---|
//! | 1 | Zwei Aufrufe wählen gleichzeitig (Agent und Oberfläche) | die Einstellungen werden unter derselben Sperre gelesen–geändert–geschrieben wie die Befehle der Oberfläche (`mutate_settings`); beide Wahlen sind ganz oder gar nicht da | `commands::tts_providers` (Sperre), `select_sets_engine_and_voice_for_a_released_provider` |
//! | 2 | Abbruch mitten im Vorgang (Agent geht, App endet) | die Wahl ist EIN atomares Schreiben der Einstellungen; Listen und Prüfungen schreiben nichts; ein hängender Anbieter hält die Frist (4 s Gesundheit, 10 s Stimmen) und wird „nicht verfügbar“ | `list_shows_every_provider_with_release_state_and_health`, `voices_need_a_released_provider_and_filter_by_language` |
//! | 3 | Platte oder Speicher voll | die Werkzeuge schreiben außer der Wahl nichts; Antworten sind begrenzt (200 Stimmen, 100 Läufe, 512 KiB je `results.json`, Texte gekürzt) | `benchmark_results_bound_the_size_and_check_the_content` |
//! | 4 | Gerät oder Programm fehlt | der Anbieter meldet `available=false` mit Grund; die Wahl eines nicht bereiten, aber freigegebenen Anbieters ist erlaubt (die Nutzung scheitert dann mit dem Grund, wie in der Oberfläche) | `list_shows_every_provider_with_release_state_and_health` |
//! | 5 | Anbieter hängt oder stürzt ab | `Registry::health` fängt Panik und Hänger; die Stimmenliste hat eine Frist | `commands::tts_providers::tests` (Frist), `voices_need_a_released_provider_and_filter_by_language` |
//! | 6 | Unvertraute Argumente (Pfade, `..`, falscher Typ, unbekanntes Feld, Freigabe-Schalter) | strenge Prüfung, klare Meldung, nichts getan | `no_tool_can_change_enabled_acks_or_remote`, `benchmark_results_refuse_path_traversal`, `select_keeps_the_voice_without_one_and_rejects_unlisted_or_odd_voices` |
//! | 7 | Agent will einen gesperrten Anbieter wählen oder hören | Fehler mit Grund, Einstellungen unverändert; Datenschutzsperre (Fernzugriff) bleibt auch bei Opt-in und Zustimmung | `select_refuses_a_blocked_provider_and_changes_nothing`, `select_cannot_lift_the_privacy_block` |
//! | 8 | Agent liest fremde Dateien über `tts_benchmark_results` | nur zwei feste Ordner, Kennung ohne Pfadanteile, aufgelöster Pfad muss im Ordner liegen (auch bei Verknüpfungen) | `benchmark_results_refuse_path_traversal` |
//! | 9 | Es gibt keine Anbieterschicht (headless Sandbox, App startet noch) | Klartext, keine Panik | `without_the_host_access_the_tools_say_so` |
//!
//! Audio-Echtzeitpfad: nicht berührt. Kindprozesse: keine (die Gesundheitsprüfung der Anbieter
//! startet höchstens ihre eigenen, begrenzten Prüfungen). Voller Arbeitsspeicher: die Werkzeuge
//! laden kein Modell.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::{clean_line, paths, spec, string_prop, AppTools, Args};
use crate::agent_bridge::catalog::ToolSpec;
use crate::commands::tts_providers::{build_entries, TtsProviderEntry};
use crate::managers::integrations::audit;
use crate::managers::tts::engine;
use crate::managers::tts::providers::contract::{
    ProviderCaps, ProviderHealth, TtsError, TtsErrorKind, VoiceDescriptor,
};
use crate::managers::tts::providers::host::ProviderSnapshot;

/// Frist der Gesundheitsprüfung je Anbieter (wie die Befehle der Oberfläche).
pub const HEALTH_LIMIT: Duration = Duration::from_secs(4);
/// Frist der Stimmenliste.
pub const VOICES_LIMIT: Duration = Duration::from_secs(10);
/// Höchstens so viele Stimmen in der Antwort.
pub const MAX_VOICES: usize = 200;
/// Höchstens so viele Läufe in der Liste der Benchmark-Ergebnisse.
pub const MAX_RUNS_LISTED: usize = 100;
/// Größte `results.json`, die geliefert wird.
pub const MAX_RESULTS_BYTES: u64 = 512 * 1024;
/// Längste Stimmenkennung, die ein Agent angeben darf.
const MAX_VOICE_CHARS: usize = 100;
/// Längste Sprachangabe.
const MAX_LANGUAGE_CHARS: usize = 12;
/// Längster Text aus einem Anbieter (Grund, Hinweis, Meldung), nach der Schwärzung.
const MAX_TEXT_CHARS: usize = 400;
/// Längste Stimmenkennung bzw. -bezeichnung in der Antwort.
const MAX_VOICE_TEXT_CHARS: usize = 120;

/// Der Zugriff der Werkzeuge auf die Anbieterschicht. Die App liefert den echten Zugriff
/// (`app_host`), die Tests eine Attrappe. Es gibt hier ABSICHTLICH keine Methode, die Opt-in,
/// Lizenz-Zustimmung oder Fernzugriff setzt (siehe Moduldoku).
pub trait TtsProviderAccess: Send + Sync {
    /// Momentaufnahme: Einstellungen (Freigaben), wirksame Konfiguration, Registry.
    fn snapshot(&self) -> Result<ProviderSnapshot, String>;
    /// Anbieter (und Stimme) fürs Vorlesen wählen. Schreibt die Einstellungen und prüft die
    /// Guardrails an der Stelle, die schreibt.
    fn select(&self, id: &str, voice: Option<String>) -> Result<(), TtsError>;
    /// Orte der Benchmark-Zusammenfassungen.
    fn benchmark_roots(&self) -> BenchRoots;
}

/// Die zwei Orte, an denen Zusammenfassungen liegen können.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BenchRoots {
    /// `%LOCALAPPDATA%\lva-bench\tts` (Rohdaten und Berichte des Harness):
    /// `<Lauf>/report/results.json`.
    pub bench: Option<PathBuf>,
    /// `docs/tts/benchmarks` (veröffentlichte Zusammenfassungen): `<Datum>/results.json`.
    pub docs: Option<PathBuf>,
}

/// Orte aus Umgebung und Programmpfad: `LVA_BENCH_DIR` (sonst `%LOCALAPPDATA%\lva-bench`) +
/// `tts`; die veröffentlichten Zusammenfassungen unter `LVA_TTS_BENCH_DOCS`, sonst
/// `docs/tts/benchmarks` oberhalb des Programms (nur im Entwicklungsbau, siehe [`bench_roots_with`]).
pub fn bench_roots_from(env: &dyn Fn(&str) -> Option<String>, exe: Option<&Path>) -> BenchRoots {
    bench_roots_with(env, exe, cfg!(debug_assertions))
}

/// Wie [`bench_roots_from`]; `search_exe_ancestors` schaltet die Suche nach
/// `docs/tts/benchmarks` oberhalb des Programms ein. Sie gilt NUR im
/// Entwicklungsbau ([`bench_roots_from`] übergibt `cfg!(debug_assertions)`): eine
/// installierte App würde sonst bis zu acht Ebenen über sich nach einem Ordner
/// `docs/tts/benchmarks` suchen (auch `C:\docs\…`) und daraus lesen. Dort bleibt
/// `LVA_TTS_BENCH_DOCS` der ausdrückliche Weg.
pub fn bench_roots_with(
    env: &dyn Fn(&str) -> Option<String>,
    exe: Option<&Path>,
    search_exe_ancestors: bool,
) -> BenchRoots {
    let non_empty = |name: &str| env(name).filter(|v| !v.trim().is_empty());
    let bench = non_empty("LVA_BENCH_DIR")
        .map(PathBuf::from)
        .or_else(|| non_empty("LOCALAPPDATA").map(|p| PathBuf::from(p).join("lva-bench")))
        .map(|p| p.join("tts"));
    let docs = non_empty("LVA_TTS_BENCH_DOCS")
        .map(PathBuf::from)
        .or_else(|| {
            if !search_exe_ancestors {
                return None;
            }
            exe.and_then(|e| {
                e.ancestors()
                    .skip(1)
                    .take(8)
                    .map(|d| d.join("docs").join("tts").join("benchmarks"))
                    .find(|p| p.is_dir())
            })
        });
    BenchRoots { bench, docs }
}

// ---------------------------------------------------------------------------
// Beschreibungen
// ---------------------------------------------------------------------------

pub fn specs() -> Vec<ToolSpec> {
    let id_prop = || {
        string_prop(
            "Kennung des Anbieters aus tts_providers_list, z. B. piper.",
            64,
        )
    };
    vec![
        spec(
            "tts_providers_list",
            "Listet die Sprachausgabe-Anbieter von Local Voice AI (Fish Speech, Piper, Phonon, OpenAI-kompatibler \
             Server, Ollama-Speech): Kennung, Stufe, Fähigkeiten (Sprachen, Streaming, Geräte, Klonen), Lizenz der \
             Gewichte, Gesundheit (mit Frist), ob der Anbieter freigegeben ist (enabled, license_acked, allow_remote) \
             und warum er gesperrt ist (blocked). Freigaben setzt nur der Nutzer in der App; kein Werkzeug kann sie \
             ändern.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        spec(
            "tts_provider_capabilities",
            "Liefert die Fähigkeiten eines Sprachausgabe-Anbieters (Sprachen, Streaming, Formate, Abtastraten, \
             Klonen, Geräte, Plattformen), seine Lizenz und seinen Freigabestand.",
            json!({
                "type": "object",
                "properties": { "id": id_prop() },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        spec(
            "tts_provider_voices",
            "Listet die Stimmen eines Sprachausgabe-Anbieters, auf Wunsch nur einer Sprache. Das gilt als Nutzung des \
             Anbieters: ist er in der App nicht freigegeben, kommt der Sperrgrund statt einer Liste.",
            json!({
                "type": "object",
                "properties": {
                    "id": id_prop(),
                    "language": string_prop("Sprachkürzel zum Eingrenzen, z. B. de oder en.", MAX_LANGUAGE_CHARS)
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        spec(
            "tts_provider_health",
            "Prüft mit Frist, ob ein Sprachausgabe-Anbieter bereit ist; sonst nennt die Antwort den Grund und einen Hinweis.",
            json!({
                "type": "object",
                "properties": { "id": id_prop() },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        spec(
            "tts_provider_select",
            "Wählt Anbieter und Stimme fürs Vorlesen (wirkt wie die Auswahl in der App). Nur ein Anbieter, den der \
             Nutzer in der App freigegeben hat, lässt sich wählen; sonst kommt der Sperrgrund und nichts ändert sich. \
             Dieses Werkzeug kann keine Freigabe (Opt-in, Lizenz, Fernzugriff) setzen. Ohne voice bleibt die bisherige \
             Stimme des Anbieters; eine angegebene Stimme muss in tts_provider_voices stehen.",
            json!({
                "type": "object",
                "properties": {
                    "id": id_prop(),
                    "voice": string_prop("Stimmenkennung aus tts_provider_voices (Standard: die bisherige Stimme).", MAX_VOICE_CHARS)
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        spec(
            "tts_benchmark_results",
            "Ohne Argumente: listet die vorhandenen Zusammenfassungen der Sprachausgabe-Benchmarks (source docs = \
             veröffentlicht, bench = lokaler Messordner; je Lauf Kennung, Größe, Änderungszeit). Mit run_dir: liefert die \
             results.json dieses Laufs (höchstens 512 KiB).",
            json!({
                "type": "object",
                "properties": {
                    "run_dir": string_prop("Kennung des Laufs aus der Liste, z. B. 2026-10-27. Kein Pfad.", 64),
                    "source": { "type": "string", "enum": ["docs", "bench"],
                                "description": "Wo gesucht wird (Standard: erst docs, dann bench)." }
                },
                "additionalProperties": false
            }),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Hilfen
// ---------------------------------------------------------------------------

/// Text eines Anbieters für die Antwort: geschwärzt (Adressen auf den Host, Geheimnisse) und gekürzt.
fn clip(text: &str) -> String {
    audit::sanitize_audit_text(text, MAX_TEXT_CHARS)
}

fn short(text: &str) -> String {
    clean_line(text)
        .chars()
        .take(MAX_VOICE_TEXT_CHARS)
        .collect()
}

/// Fehlertext für den Agenten; Sperren sagen, wer freigeben darf.
fn failure_text(e: &TtsError) -> String {
    match e.kind {
        TtsErrorKind::LicenseBlocked | TtsErrorKind::PrivacyBlocked => format!(
            "Gesperrt ({}): {} Freigaben erteilt nur der Nutzer in der App (Einstellungen > Sprachausgabe-Anbieter); \
             kein Werkzeug kann sie setzen.",
            e.kind.as_str(),
            clip(&e.message)
        ),
        _ => clip(&e.message),
    }
}

fn health_json(h: &ProviderHealth) -> Value {
    json!({
        "available": h.available,
        "reason": h.reason.as_deref().map(clip),
        "hint": h.hint.as_deref().map(clip),
        "device": serde_json::to_value(h.device).unwrap_or(Value::Null),
        "version": h.version.as_deref().map(clip),
        "checked_ms": h.checked_ms,
    })
}

fn caps_json(caps: &ProviderCaps) -> Value {
    serde_json::to_value(caps).unwrap_or(Value::Null)
}

fn blocked_json(e: &Option<TtsError>) -> Value {
    match e {
        None => Value::Null,
        Some(e) => json!({ "kind": e.kind.as_str(), "message": clip(&e.message) }),
    }
}

fn entry_json(e: &TtsProviderEntry) -> Value {
    let d = &e.descriptor;
    json!({
        "id": d.id,
        "label": d.label,
        "tier": serde_json::to_value(d.tier).unwrap_or(Value::Null),
        "opt_in": d.opt_in,
        "capabilities": caps_json(&d.caps),
        "license": serde_json::to_value(&d.license).unwrap_or(Value::Null),
        "health": health_json(&e.health),
        "enabled": e.enabled,
        "license_needs_ack": e.license_needs_ack,
        "license_acked": e.license_acked,
        "allow_remote": e.allow_remote,
        "remote": e.remote,
        "blocked": blocked_json(&e.blocked),
        "usable": e.blocked.is_none() && e.health.available,
        "selected": e.selected,
        "voice": e.voice.as_deref().map(short),
    })
}

/// Sprachkürzel: Buchstaben, `-`, `_`.
fn valid_language(s: &str) -> bool {
    (2..=MAX_LANGUAGE_CHARS).contains(&s.chars().count())
        && s.chars()
            .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '_')
}

/// Form einer Stimmenkennung: kein Pfad, kein Schalter, keine Steuerzeichen. Ob es die Stimme
/// gibt, entscheidet erst die Liste des Anbieters.
fn valid_voice(v: &str) -> bool {
    !v.is_empty()
        && v.chars().count() <= MAX_VOICE_CHARS
        && !v.starts_with('-')
        && !v.contains("..")
        && v.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.' | '(' | ')' | '+'))
}

/// Kennung eines Laufs: wird zu einem Ordnernamen, also nie ein Pfad.
pub fn valid_run_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && paths::reserved_device_name(id).is_none()
}

// ---------------------------------------------------------------------------
// Benchmark-Ergebnisse (rein, ohne App)
// ---------------------------------------------------------------------------

/// (Quelle, Pfad der Zusammenfassung unterhalb von `<Ordner>/<Kennung>/`).
const SOURCES: [(&str, &str); 2] = [("docs", "results.json"), ("bench", "report/results.json")];

fn root_of<'a>(roots: &'a BenchRoots, source: &str) -> Option<&'a Path> {
    match source {
        "docs" => roots.docs.as_deref(),
        "bench" => roots.bench.as_deref(),
        _ => None,
    }
}

fn modified_text(meta: &std::fs::Metadata) -> Value {
    meta.modified()
        .ok()
        .map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .map(Value::String)
        .unwrap_or(Value::Null)
}

/// Die vorhandenen Zusammenfassungen (veröffentlichte zuerst, je Quelle die neueste Kennung zuerst).
/// Verknüpfungen werden übersprungen.
fn list_runs(roots: &BenchRoots) -> Vec<Value> {
    let mut found: Vec<(usize, String, Value)> = Vec::new();
    for (rank, (source, suffix)) in SOURCES.into_iter().enumerate() {
        let Some(root) = root_of(roots, source) else {
            continue;
        };
        let Ok(dir) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in dir.flatten().take(2000) {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !valid_run_id(&name) {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(root.join(&name).join(suffix)) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            found.push((
                rank,
                name.clone(),
                json!({
                    "source": source,
                    "run_dir": name,
                    "bytes": meta.len(),
                    "modified": modified_text(&meta),
                    "readable": meta.len() <= MAX_RESULTS_BYTES,
                }),
            ));
        }
    }
    // Veröffentlichte zuerst, je Quelle die neueste Kennung zuerst (Datum bzw. Zeitstempel vorn).
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    found
        .into_iter()
        .take(MAX_RUNS_LISTED)
        .map(|(_, _, v)| v)
        .collect()
}

/// Liest `<root>/<id>/<suffix>`: `Ok(None)`, wenn es sie nicht gibt; `Err` bei einem Pfad
/// außerhalb des Ordners, zu großer oder ungültiger Datei.
fn load_one(root: &Path, suffix: &str, id: &str) -> Result<Option<(u64, Value)>, String> {
    let Ok(root_real) = std::fs::canonicalize(root) else {
        return Ok(None);
    };
    let Ok(real) = std::fs::canonicalize(root.join(id).join(suffix)) else {
        return Ok(None);
    };
    // Auch eine Verknüpfung nach draußen endet hier: verglichen wird der AUFGELÖSTE Pfad.
    if !real.starts_with(&root_real) {
        return Err("Der Pfad liegt außerhalb des Ergebnisordners.".to_string());
    }
    let meta =
        std::fs::metadata(&real).map_err(|_| "Die results.json ist nicht lesbar.".to_string())?;
    if !meta.is_file() {
        return Ok(None);
    }
    let too_large = |bytes: u64| {
        format!(
            "Die results.json ist zu groß ({} KiB, höchstens {} KiB).",
            bytes.div_ceil(1024),
            MAX_RESULTS_BYTES / 1024
        )
    };
    if meta.len() > MAX_RESULTS_BYTES {
        return Err(too_large(meta.len()));
    }
    let mut buf = Vec::new();
    std::fs::File::open(&real)
        .and_then(|f| f.take(MAX_RESULTS_BYTES + 1).read_to_end(&mut buf))
        .map_err(|_| "Die results.json ist nicht lesbar.".to_string())?;
    if buf.len() as u64 > MAX_RESULTS_BYTES {
        return Err(too_large(buf.len() as u64));
    }
    let value: Value = serde_json::from_slice(&buf)
        .map_err(|_| "Die results.json ist kein gültiges JSON.".to_string())?;
    Ok(Some((buf.len() as u64, value)))
}

/// Antwort von `tts_benchmark_results` (rein, testbar).
pub fn benchmark_results(
    roots: &BenchRoots,
    run_dir: Option<&str>,
    source: Option<&str>,
) -> Result<Value, String> {
    let Some(id) = run_dir else {
        let runs = list_runs(roots);
        return Ok(json!({
            "runs": runs,
            "count": runs.len(),
            "roots": {
                "docs": roots.docs.as_deref().is_some_and(Path::is_dir),
                "bench": roots.bench.as_deref().is_some_and(Path::is_dir),
            },
            "hint": "Mit run_dir (und optional source) liefert dieses Werkzeug die results.json eines Laufs.",
        }));
    };
    if !valid_run_id(id) {
        return Err(
            "„run_dir“ ist keine gültige Kennung (nur Buchstaben, Ziffern, Punkt, Unterstrich und Bindestrich, \
             kein Pfad)."
                .to_string(),
        );
    }
    let order: Vec<&str> = match source {
        None => vec!["docs", "bench"],
        Some("docs") => vec!["docs"],
        Some("bench") => vec!["bench"],
        Some(_) => return Err("„source“ muss „docs“ oder „bench“ sein.".to_string()),
    };
    for src in order {
        let Some(root) = root_of(roots, src) else {
            continue;
        };
        let suffix = SOURCES
            .iter()
            .find(|(s, _)| *s == src)
            .map(|(_, p)| *p)
            .unwrap_or("results.json");
        if let Some((bytes, results)) = load_one(root, suffix, id)? {
            return Ok(json!({
                "source": src,
                "run_dir": id,
                "bytes": bytes,
                "results": results,
            }));
        }
    }
    Err("Zu dieser Kennung gibt es keine results.json. Ohne Argumente listet das Werkzeug die vorhandenen Läufe."
        .to_string())
}

// ---------------------------------------------------------------------------
// Ausführung
// ---------------------------------------------------------------------------

impl AppTools {
    fn tts_access(&self) -> Result<Arc<dyn TtsProviderAccess>, String> {
        self.host.tts_providers().ok_or_else(|| {
            "Die Sprachausgabe-Anbieter sind in dieser Umgebung nicht verfügbar.".to_string()
        })
    }

    pub(super) fn tts_providers_list(&self, a: Args) -> Result<Value, String> {
        a.done()?;
        let snapshot = self.tts_access()?.snapshot()?;
        let health = tauri::async_runtime::block_on(snapshot.registry.health_all(HEALTH_LIMIT));
        let entries = build_entries(&snapshot, health);
        let providers: Vec<Value> = entries.iter().map(entry_json).collect();
        Ok(json!({
            "providers": providers,
            "count": providers.len(),
            "selected": entries.iter().find(|e| e.selected).map(|e| e.descriptor.id.clone()),
            "hint": "Gesperrte Anbieter schaltet nur der Nutzer in der App frei. tts_provider_select wählt einen \
                     freigegebenen Anbieter, tts_provider_voices listet seine Stimmen.",
        }))
    }

    pub(super) fn tts_provider_capabilities(&self, mut a: Args) -> Result<Value, String> {
        let id = a.id("id")?.ok_or_else(|| "„id“ fehlt.".to_string())?;
        a.done()?;
        let snapshot = self.tts_access()?.snapshot()?;
        let d = snapshot.descriptor(&id).map_err(|e| failure_text(&e))?;
        let state = snapshot.use_state(&d);
        Ok(json!({
            "id": d.id,
            "label": d.label,
            "tier": serde_json::to_value(d.tier).unwrap_or(Value::Null),
            "opt_in": d.opt_in,
            "capabilities": caps_json(&d.caps),
            "license": serde_json::to_value(&d.license).unwrap_or(Value::Null),
            "enabled": state.enabled,
            "license_needs_ack": state.license_needs_ack,
            "license_acked": state.license_acked,
            "allow_remote": state.allow_remote,
            "remote": state.remote,
            "blocked": blocked_json(&state.blocked),
            "selected": snapshot.selected == d.id,
            "voice": snapshot.voice_of(&d.id).as_deref().map(short),
        }))
    }

    pub(super) fn tts_provider_health(&self, mut a: Args) -> Result<Value, String> {
        let id = a.id("id")?.ok_or_else(|| "„id“ fehlt.".to_string())?;
        a.done()?;
        let snapshot = self.tts_access()?.snapshot()?;
        snapshot.descriptor(&id).map_err(|e| failure_text(&e))?;
        let health = tauri::async_runtime::block_on(snapshot.registry.health(&id, HEALTH_LIMIT))
            .map_err(|e| failure_text(&e))?;
        Ok(json!({ "id": id, "health": health_json(&health) }))
    }

    /// Stimmen eines Anbieters mit Frist (die Prüfung der Guardrails macht der Aufrufer).
    fn voices_of(
        snapshot: &ProviderSnapshot,
        id: &str,
        language: Option<&str>,
    ) -> Result<Vec<VoiceDescriptor>, String> {
        let provider = snapshot
            .registry
            .get(id)
            .ok_or_else(|| format!("Unbekannter Anbieter '{id}'"))?;
        let listed = tauri::async_runtime::block_on(async {
            tokio::time::timeout(VOICES_LIMIT, provider.list_voices(language)).await
        });
        match listed {
            Ok(Ok(voices)) => Ok(voices),
            Ok(Err(e)) => Err(failure_text(&e)),
            Err(_) => Err(format!(
                "Die Stimmenliste kam nicht innerhalb von {} s.",
                VOICES_LIMIT.as_secs()
            )),
        }
    }

    pub(super) fn tts_provider_voices(&self, mut a: Args) -> Result<Value, String> {
        let id = a.id("id")?.ok_or_else(|| "„id“ fehlt.".to_string())?;
        let language = a.text("language", MAX_LANGUAGE_CHARS, false)?;
        a.done()?;
        if let Some(l) = &language {
            if !valid_language(l) {
                return Err("„language“ ist kein Sprachkürzel (z. B. de oder en).".to_string());
            }
        }
        let snapshot = self.tts_access()?.snapshot()?;
        // Die Stimmenliste gilt als Nutzung des Anbieters: gleiche Sperren wie bei der Oberfläche.
        snapshot.checked(&id).map_err(|e| failure_text(&e))?;
        let voices = Self::voices_of(&snapshot, &id, language.as_deref())?;
        let total = voices.len();
        let listed: Vec<Value> = voices
            .iter()
            .take(MAX_VOICES)
            .map(|v| {
                json!({
                    "id": short(&v.id),
                    "label": short(&v.label),
                    "language": short(&v.language),
                    "gender": v.gender.as_deref().map(short),
                    "license": v.license.as_ref().map(|l| json!({
                        "weights": short(&l.weights),
                        "non_commercial": l.non_commercial,
                        "verified": l.verified,
                    })),
                })
            })
            .collect();
        Ok(json!({
            "provider": id,
            "voices": listed,
            "count": listed.len(),
            "total": total,
        }))
    }

    pub(super) fn tts_provider_select(&self, mut a: Args) -> Result<Value, String> {
        let id = a.id("id")?.ok_or_else(|| "„id“ fehlt.".to_string())?;
        let voice = a.text("voice", MAX_VOICE_CHARS, false)?;
        a.done()?;
        if let Some(v) = &voice {
            if !valid_voice(v) {
                return Err(
                    "„voice“ ist keine gültige Stimmenkennung (kein Pfad, kein Schalter, keine Steuerzeichen). \
                     Die Kennungen listet tts_provider_voices."
                        .to_string(),
                );
            }
        }
        let access = self.tts_access()?;
        let snapshot = access.snapshot()?;
        // Erste Prüfung der Guardrails: ein gesperrter Anbieter wird nicht einmal nach Stimmen gefragt.
        snapshot.checked(&id).map_err(|e| failure_text(&e))?;
        let voice = match voice {
            Some(v) => {
                let known = Self::voices_of(&snapshot, &id, None)?;
                if !known.iter().any(|k| k.id == v) {
                    return Err(format!(
                        "Die Stimme „{v}“ gibt es bei diesem Anbieter nicht. tts_provider_voices listet die vorhandenen."
                    ));
                }
                Some(v)
            }
            // Ohne Angabe bleibt die bisherige Stimme (die Auswahl in der App würde sie zurücksetzen).
            None => snapshot.voice_of(&id),
        };
        // Zweite Prüfung beim Schreiben: `select` rechnet mit dem Stand dieses Moments.
        access
            .select(&id, voice.clone())
            .map_err(|e| failure_text(&e))?;
        Ok(json!({
            "selected": id,
            "voice": voice.as_deref().map(short),
            "engine": engine::engine_setting_for(&id),
            "hint": "Das Vorlesen nutzt ab jetzt diesen Anbieter. Freigaben bleiben unverändert; sie setzt nur der Nutzer in der App.",
        }))
    }

    pub(super) fn tts_benchmark_results(&self, mut a: Args) -> Result<Value, String> {
        let run_dir = a.text("run_dir", 64, false)?;
        let source = a.text("source", 8, false)?;
        a.done()?;
        let roots = self.tts_access()?.benchmark_roots();
        benchmark_results(&roots, run_dir.as_deref(), source.as_deref())
    }
}

#[cfg(test)]
mod tests;
