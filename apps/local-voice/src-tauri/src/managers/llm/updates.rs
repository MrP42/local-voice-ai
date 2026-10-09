//! Neue Modelle der Anbieter erkennen und das aeltere derselben Familie durch das
//! neuere ersetzen.
//!
//! Die App prueft nach dem Start (und danach taeglich), ob eine Verbindung
//! Modelle anbietet, die sie noch nicht kennt: Codex ueber seinen Katalog,
//! API-Anbieter und Ollama ueber ihren Modell-Endpunkt, das Claude-Abo -- das
//! keinen Katalog hat -- durch Ausprobieren naheliegender Nachfolger der
//! bekannten Namen. Was neu ist, entscheidet `llm_model_history`: ein Modell,
//! das nicht freigegeben ist, ist nicht automatisch neu (es kann auch ersetzt
//! oder bewusst weggelassen sein).
//!
//! Dieses Modul haelt die reinen Teile (Namen, Vergleich, Erkennung, Ersetzen);
//! die Befehle in `commands::llm` verbinden sie mit Netz, CLI und Einstellungen.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;

use super::cli::{self, Cli};
use crate::settings::{AppSettings, LlmConnection, LlmModelConfig, LlmModelSeen};

/// Hoechstens so viele Claude-Kandidaten je Lauf (jeder ist ein CLI-Aufruf).
pub const PROBE_PER_RUN: usize = 3;
/// Zeitgrenze je Kandidat.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// Zeitgrenze fuer alle Kandidaten eines Laufs.
pub const PROBE_BUDGET: Duration = Duration::from_secs(60);
/// Ein Kandidat, den der Anbieter nicht kennt, wird fruehestens nach so vielen
/// Sekunden (7 Tage) erneut ausprobiert.
pub const ABSENT_RECHECK_SECS: i64 = 7 * 24 * 3600;

pub const MODE_ASK: &str = "ask";
pub const MODE_ON: &str = "on";
pub const MODE_OFF: &str = "off";

pub const STATUS_SEEN: &str = "seen";
pub const STATUS_NEW: &str = "new";
pub const STATUS_REPLACED: &str = "replaced";
pub const STATUS_ABSENT: &str = "absent";

pub fn valid_mode(mode: &str) -> bool {
    matches!(mode, MODE_ASK | MODE_ON | MODE_OFF)
}

/// Ein neues Modell, das zur Uebernahme ansteht.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Type)]
pub struct ModelUpdate {
    pub connection_id: String,
    pub connection_label: String,
    pub remote_id: String,
    /// Das freigegebene Modell (`connection_id:remote_id`), das es ersetzt.
    pub replaces: Option<String>,
    pub replaces_remote_id: Option<String>,
    /// Das zu ersetzende Modell ist gerade das aktive.
    pub replaces_active: bool,
}

/// Familie und Version aus einem Modellnamen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedModel {
    pub family: String,
    pub version: Vec<u32>,
}

fn all_digits(s: &str, max_len: usize) -> bool {
    !s.is_empty() && s.len() <= max_len && s.chars().all(|c| c.is_ascii_digit())
}

/// Erkennt `claude-<familie>-<major>[-<minor>][-<datum>]` und
/// `gpt-<version>-<name>`. Aliase (`haiku`, `sonnet` …) und alles andere sind
/// keine festen Versionen und werden nie ersetzt.
pub fn parse_model(remote_id: &str) -> Option<ParsedModel> {
    let tokens: Vec<&str> = remote_id.split('-').collect();
    match tokens.as_slice() {
        ["claude", fam, rest @ ..]
            if !fam.is_empty() && fam.chars().all(|c| c.is_ascii_alphabetic()) =>
        {
            let mut version = Vec::new();
            let mut iter = rest.iter().peekable();
            while let Some(t) = iter.peek() {
                if all_digits(t, 2) && version.len() < 2 {
                    version.push(t.parse().ok()?);
                    iter.next();
                } else {
                    break;
                }
            }
            // Ein Datum (achtstellig) darf folgen, sonst nichts.
            match iter.next() {
                None => {}
                Some(date) if all_digits(date, 8) && date.len() == 8 && iter.next().is_none() => {}
                _ => return None,
            }
            if version.is_empty() {
                return None;
            }
            Some(ParsedModel {
                family: format!("claude-{fam}"),
                version,
            })
        }
        ["gpt", ver, name]
            if ver.chars().next().is_some_and(|c| c.is_ascii_digit())
                && ver.chars().all(|c| c.is_ascii_digit() || c == '.')
                && !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphabetic()) =>
        {
            let version: Option<Vec<u32>> = ver.split('.').map(|p| p.parse().ok()).collect();
            Some(ParsedModel {
                family: format!("gpt-{name}"),
                version: version?,
            })
        }
        _ => None,
    }
}

/// `a` ist eine hoehere Version als `b` (fehlende Stellen zaehlen als 0).
pub fn is_newer(a: &[u32], b: &[u32]) -> bool {
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// Naheliegende Nachfolger der bekannten festen Claude-Namen: je Familie die
/// hoechste bekannte Version, dazu Minor+1, Major+1.0 und Major+1.5 (Haiku 4.5
/// wurde zu Haiku 5.5). Ohne Datumssuffix, ohne bereits bekannte Namen.
pub fn claude_candidates(known: &[String]) -> Vec<String> {
    let mut best: Vec<(String, Vec<u32>)> = Vec::new();
    for id in known {
        let Some(p) = parse_model(id) else { continue };
        let Some(fam) = p.family.strip_prefix("claude-") else {
            continue;
        };
        match best.iter_mut().find(|(f, _)| f == fam) {
            Some((_, v)) if !is_newer(&p.version, v) => {}
            Some((_, v)) => *v = p.version,
            None => best.push((fam.to_string(), p.version)),
        }
    }
    let mut out = Vec::new();
    for (fam, v) in best {
        let (major, minor) = (v[0], v.get(1).copied().unwrap_or(0));
        for (a, b) in [(major, minor + 1), (major + 1, 0), (major + 1, 5)] {
            let name = format!("claude-{fam}-{a}-{b}");
            if !known.contains(&name) && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

fn entry<'a>(s: &'a AppSettings, key: &str) -> Option<&'a LlmModelSeen> {
    s.llm_model_history.iter().find(|h| h.key == key)
}

pub fn set_history(
    history: &mut Vec<LlmModelSeen>,
    key: &str,
    status: &str,
    now: i64,
    replaced_by: Option<String>,
) {
    match history.iter_mut().find(|h| h.key == key) {
        Some(h) => {
            h.status = status.to_string();
            h.at = now;
            h.replaced_by = replaced_by;
        }
        None => history.push(LlmModelSeen {
            key: key.to_string(),
            status: status.to_string(),
            at: now,
            replaced_by,
        }),
    }
}

/// Das freigegebene Modell derselben Familie, das ein `remote` ersetzen wuerde:
/// das neueste aeltere. Gibt es schon eine gleiche oder neuere Version, ersetzt
/// nichts (das Neue ist dann nicht das Neueste).
fn find_replaced<'a>(
    s: &'a AppSettings,
    connection_id: &str,
    remote: &str,
) -> Option<&'a LlmModelConfig> {
    let new = parse_model(remote)?;
    let same: Vec<(&LlmModelConfig, ParsedModel)> = s
        .llm_models
        .iter()
        .filter(|m| m.connection_id == connection_id && m.remote_id != remote)
        .filter_map(|m| parse_model(&m.remote_id).map(|p| (m, p)))
        .filter(|(_, p)| p.family == new.family)
        .collect();
    if same.iter().any(|(_, p)| !is_newer(&new.version, &p.version)) {
        return None;
    }
    same.into_iter()
        .max_by(|a, b| {
            if is_newer(&a.1.version, &b.1.version) {
                std::cmp::Ordering::Greater
            } else if is_newer(&b.1.version, &a.1.version) {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .map(|(m, _)| m)
}

/// Was ein Pruefdurchgang fuer eine Verbindung ergibt: neue Eintraege des
/// Verlaufs (nur Aenderungen, nach Schluessel).
///
/// `track_all`: auch Namen ohne erkennbare Version als neu vermerken (Ollama).
/// Bei API-Anbietern bleibt es aus: sie listen Dutzende Hilfsmodelle (Bilder,
/// Sprache, Einbettungen), die niemand "uebernehmen" will.
pub fn record_offered(
    s: &AppSettings,
    connection: &LlmConnection,
    offered: &[String],
    track_all: bool,
    now: i64,
) -> Vec<LlmModelSeen> {
    let prefix = format!("{}:", connection.id);
    // Erster Durchgang: alles, was der Anbieter heute anbietet, gilt als
    // bekannt -- sonst waere jedes Modell einer alten Installation "neu".
    let baseline = !s
        .llm_model_history
        .iter()
        .any(|h| h.key.starts_with(&prefix) && h.status != STATUS_ABSENT);
    let mut out = Vec::new();
    for remote in offered {
        let key = LlmModelConfig::make_id(&connection.id, remote);
        let released = s.llm_models.iter().any(|m| m.id == key);
        // Bekannt, ersetzt, abgelehnt oder schon als neu vermerkt: nichts zu tun.
        // Nur ein Kandidat, der einmal "abwesend" war, darf erneut auftauchen.
        if entry(s, &key).is_some_and(|h| h.status != STATUS_ABSENT) {
            continue;
        }
        let status = if baseline || released || !(track_all || parse_model(remote).is_some()) {
            STATUS_SEEN
        } else {
            STATUS_NEW
        };
        out.push(LlmModelSeen {
            key,
            status: status.to_string(),
            at: now,
            replaced_by: None,
        });
    }
    out
}

/// Die Modelle, die auf eine Uebernahme warten: im Verlauf als `new`, noch
/// nicht freigegeben, Verbindung vorhanden und eingeschaltet.
pub fn pending_updates(s: &AppSettings) -> Vec<ModelUpdate> {
    let mut out = Vec::new();
    for h in s.llm_model_history.iter().filter(|h| h.status == STATUS_NEW) {
        if s.llm_models.iter().any(|m| m.id == h.key) {
            continue;
        }
        let Some((c, remote)) = s
            .llm_connections
            .iter()
            .filter(|c| c.enabled)
            .find_map(|c| h.key.strip_prefix(&format!("{}:", c.id)).map(|r| (c, r)))
        else {
            continue;
        };
        let replaced = find_replaced(s, &c.id, remote);
        out.push(ModelUpdate {
            connection_id: c.id.clone(),
            connection_label: c.label.clone(),
            remote_id: remote.to_string(),
            replaces: replaced.map(|m| m.id.clone()),
            replaces_remote_id: replaced.map(|m| m.remote_id.clone()),
            replaces_active: replaced
                .is_some_and(|m| s.llm_active_model_id.as_deref() == Some(m.id.as_str())),
        });
    }
    out
}

/// Uebernimmt ein neues Modell: gibt es frei und ersetzt das aeltere derselben
/// Familie (aktiv, wenn das aeltere es war). Effort und Fast wandern mit; Preise
/// erbt das neue als Obergrenze, damit ein hartes Budget nicht ins Leere greift.
/// Eine gesperrte Verbindung nimmt nichts auf. Gibt die Id des neuen Modells.
pub fn apply(s: &mut AppSettings, update: &ModelUpdate, now: i64) -> Result<String, String> {
    let connection = s
        .llm_connections
        .iter()
        .find(|c| c.id == update.connection_id && c.enabled)
        .cloned()
        .ok_or_else(|| format!("Unbekannte Verbindung: {}", update.connection_id))?;
    if crate::commands::compliance::assess_connection(s, &connection).verdict
        == crate::managers::compliance::Verdict::Blocked
    {
        return Err(format!(
            "{}: {}",
            crate::managers::compliance::CODE_COMPLIANCE_BLOCKED,
            connection.label
        ));
    }
    let new_id = LlmModelConfig::make_id(&connection.id, &update.remote_id);
    // Das zu ersetzende Modell neu bestimmen: zwischen Pruefung und Antwort kann
    // der Nutzer etwas geaendert haben.
    let old = find_replaced(s, &connection.id, &update.remote_id).cloned();
    if !s.llm_models.iter().any(|m| m.id == new_id) {
        let mut model = LlmModelConfig {
            id: new_id.clone(),
            connection_id: connection.id.clone(),
            remote_id: update.remote_id.clone(),
            label: update.remote_id.clone(),
            enabled: true,
            context_limit: None,
            max_input_tokens: None,
            max_output_tokens: None,
            price_input_per_mtok: None,
            price_output_per_mtok: None,
            tags: Vec::new(),
            effort: None,
            fast: false,
        };
        if let Some(old) = &old {
            model.context_limit = old.context_limit;
            model.max_input_tokens = old.max_input_tokens;
            model.max_output_tokens = old.max_output_tokens;
            model.price_input_per_mtok = old.price_input_per_mtok;
            model.price_output_per_mtok = old.price_output_per_mtok;
            model.tags = old.tags.clone();
            model.effort = old.effort.clone();
            model.fast = old.fast;
        }
        s.llm_models.push(model);
    } else if let Some(m) = s.llm_models.iter_mut().find(|m| m.id == new_id) {
        m.enabled = true;
    }
    set_history(&mut s.llm_model_history, &new_id, STATUS_SEEN, now, None);
    if let Some(old) = old {
        s.llm_models.retain(|m| m.id != old.id);
        set_history(
            &mut s.llm_model_history,
            &old.id,
            STATUS_REPLACED,
            now,
            Some(new_id.clone()),
        );
        if s.llm_active_model_id.as_deref() == Some(old.id.as_str()) {
            s.llm_active_model_id = Some(new_id.clone());
        }
    }
    s.sync_legacy_from_llm();
    Ok(new_id)
}

/// Wie ein Probeaufruf ausging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// Die CLI hat geantwortet: das Modell gibt es.
    Available,
    /// Die CLI kennt das Modell nicht (oder das Konto hat keinen Zugriff).
    Absent,
    /// Limit, Anmeldung, Zeitgrenze, fehlende CLI: nichts schliessen, aufhoeren.
    Abort,
}

pub fn classify_probe(result: &Result<(), String>) -> Probe {
    match result {
        Ok(()) => Probe::Available,
        Err(e) if e.starts_with("cli_model_not_in_plan") => Probe::Absent,
        // Der Fehlertext der CLI nennt das Modell, wenn es das ist.
        Err(e) if e.starts_with("cli_failed") && e.to_ascii_lowercase().contains("model") => {
            Probe::Absent
        }
        Err(_) => Probe::Abort,
    }
}

/// Ergebnis der Kandidatenpruefung.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ProbeRun {
    pub available: Vec<String>,
    pub absent: Vec<String>,
}

/// Die Kandidaten, die in diesem Lauf drankommen: nie Gepruefte zuerst, dann
/// die laenger als sieben Tage zurueckliegenden; hoechstens [`PROBE_PER_RUN`].
pub fn due_candidates(s: &AppSettings, connection_id: &str, known: &[String], now: i64) -> Vec<String> {
    let mut fresh = Vec::new();
    let mut stale = Vec::new();
    for name in claude_candidates(known) {
        let key = LlmModelConfig::make_id(connection_id, &name);
        match entry(s, &key) {
            None => fresh.push(name),
            Some(h) if h.status == STATUS_ABSENT && now - h.at >= ABSENT_RECHECK_SECS => {
                stale.push(name)
            }
            Some(_) => {}
        }
    }
    fresh.extend(stale);
    fresh.truncate(PROBE_PER_RUN);
    fresh
}

/// Probiert die Kandidaten der Reihe nach. `call` fuehrt den Aufruf aus (in
/// Tests eine Attrappe); beim ersten Abbruchgrund oder nach [`PROBE_BUDGET`]
/// ist Schluss.
pub fn run_probes(
    candidates: &[String],
    budget: Duration,
    mut call: impl FnMut(&str) -> Result<(), String>,
) -> ProbeRun {
    let started = Instant::now();
    let mut run = ProbeRun::default();
    for name in candidates {
        if started.elapsed() >= budget {
            break;
        }
        match classify_probe(&call(name)) {
            Probe::Available => run.available.push(name.clone()),
            Probe::Absent => run.absent.push(name.clone()),
            Probe::Abort => break,
        }
    }
    run
}

/// Der echte Probeaufruf: ein winziger Auftrag an die CLI, isoliert wie jeder
/// andere Aufruf der App (kein Nutzerkontext, keine Werkzeuge, kein MCP).
pub fn probe_claude(binary: &Path, model: &str) -> Result<(), String> {
    cli::call_with_timeout(
        Cli::Claude,
        binary,
        &format!("{model}@low"),
        Some("Antworte nur mit: ok"),
        "ok?",
        PROBE_TIMEOUT,
    )
    .map(|_| ())
}

/// Setzt die in der Vergangenheit gefundenen Claude-Modelle neu (beim Start und
/// nach jedem Lauf): alles, was zu einer Claude-Abo-Verbindung im Verlauf steht,
/// nicht `absent` ist und nicht zur festen Liste gehoert.
pub fn restore_claude_extras(s: &AppSettings) {
    let mut extra = Vec::new();
    for c in s
        .llm_connections
        .iter()
        .filter(|c| Cli::from_base_url(&c.base_url) == Some(Cli::Claude))
    {
        let prefix = format!("{}:", c.id);
        for h in s.llm_model_history.iter().filter(|h| h.status != STATUS_ABSENT) {
            if let Some(remote) = h.key.strip_prefix(&prefix) {
                if !cli::CLAUDE_MODELS.contains(&remote) && parse_model(remote).is_some() {
                    extra.push(remote.to_string());
                }
            }
        }
    }
    cli::set_claude_extra_models(extra);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;

    fn conn(id: &str, url: &str) -> LlmConnection {
        LlmConnection {
            id: id.into(),
            kind: id.into(),
            label: id.into(),
            base_url: url.into(),
            enabled: true,
            monthly_budget_usd: None,
            budget_enforced: false,
            training_opt_out: false,
        }
    }

    fn model(c: &str, remote: &str) -> LlmModelConfig {
        LlmModelConfig {
            id: LlmModelConfig::make_id(c, remote),
            connection_id: c.into(),
            remote_id: remote.into(),
            label: remote.into(),
            enabled: true,
            context_limit: None,
            max_input_tokens: None,
            max_output_tokens: None,
            price_input_per_mtok: None,
            price_output_per_mtok: None,
            tags: Vec::new(),
            effort: None,
            fast: false,
        }
    }

    fn settings_with(models: &[(&str, &str)]) -> AppSettings {
        let mut s = get_default_settings();
        s.compliance_profile = crate::managers::compliance::ComplianceProfile::None;
        s.llm_connections = vec![conn("claude", "cli://claude"), conn("codex", "cli://codex")];
        s.llm_models = models.iter().map(|(c, r)| model(c, r)).collect();
        s
    }

    #[test]
    fn parse_claude_and_gpt_names() {
        let p = parse_model("claude-haiku-4-5-20251001").unwrap();
        assert_eq!((p.family.as_str(), p.version), ("claude-haiku", vec![4, 5]));
        let p = parse_model("claude-sonnet-5-5").unwrap();
        assert_eq!((p.family.as_str(), p.version), ("claude-sonnet", vec![5, 5]));
        let p = parse_model("claude-opus-4").unwrap();
        assert_eq!(p.version, vec![4]);
        let p = parse_model("gpt-6.1-sol").unwrap();
        assert_eq!((p.family.as_str(), p.version), ("gpt-sol", vec![6, 1]));
        let p = parse_model("gpt-6-astra").unwrap();
        assert_eq!((p.family.as_str(), p.version), ("gpt-astra", vec![6]));
    }

    #[test]
    fn aliases_and_foreign_names_have_no_version() {
        for id in ["haiku", "sonnet", "fable", "qwen3:4b", "gpt-4.1", "claude-haiku", "claude-x-1-2-3-4"] {
            assert_eq!(parse_model(id), None, "{id}");
        }
    }

    #[test]
    fn versions_compare_with_missing_places_as_zero() {
        assert!(is_newer(&[6, 1], &[6]));
        assert!(!is_newer(&[6], &[6, 0]));
        assert!(is_newer(&[5, 6], &[5, 5]));
        assert!(!is_newer(&[5, 5], &[5, 5]));
        assert!(is_newer(&[6], &[5, 6]));
    }

    #[test]
    fn candidates_follow_the_highest_known_version_per_family() {
        let known: Vec<String> = ["haiku", "claude-haiku-4-5-20251001", "claude-sonnet-5-5", "claude-fable-5-1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let c = claude_candidates(&known);
        assert!(c.contains(&"claude-haiku-5-5".to_string()), "{c:?}");
        assert!(c.contains(&"claude-haiku-4-6".to_string()));
        assert!(c.contains(&"claude-sonnet-5-6".to_string()));
        assert!(c.contains(&"claude-fable-5-2".to_string()));
        assert!(!c.contains(&"claude-sonnet-5-5".to_string()), "bekannte nie");
        // Mit Haiku 5.5 bekannt: der Nachfolger richtet sich danach.
        let mut known2 = known.clone();
        known2.push("claude-haiku-5-5".into());
        let c = claude_candidates(&known2);
        assert!(c.contains(&"claude-haiku-5-6".to_string()));
        assert!(!c.contains(&"claude-haiku-4-6".to_string()));
    }

    #[test]
    fn first_run_is_a_baseline_not_a_flood_of_new_models() {
        let s = settings_with(&[]);
        let c = s.llm_connections[1].clone();
        let seen = record_offered(&s, &c, &["gpt-6.1-sol".into(), "gpt-6-astra".into()], false, 100);
        assert!(seen.iter().all(|h| h.status == STATUS_SEEN), "{seen:?}");
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn later_runs_flag_only_never_seen_models_as_new() {
        let mut s = settings_with(&[("codex", "gpt-6.1-sol")]);
        s.llm_model_history = vec![
            LlmModelSeen { key: "codex:gpt-6.1-sol".into(), status: STATUS_SEEN.into(), at: 1, replaced_by: None },
            LlmModelSeen { key: "codex:gpt-6-astra".into(), status: "dismissed".into(), at: 1, replaced_by: None },
        ];
        let c = s.llm_connections[1].clone();
        let seen = record_offered(
            &s,
            &c,
            &["gpt-6.1-sol".into(), "gpt-6-astra".into(), "gpt-7-sol".into()],
            false,
            100,
        );
        assert_eq!(seen.len(), 1, "bekannte und abgelehnte nicht erneut: {seen:?}");
        assert_eq!((seen[0].key.as_str(), seen[0].status.as_str()), ("codex:gpt-7-sol", STATUS_NEW));
    }

    #[test]
    fn helper_models_of_api_providers_are_not_flagged_but_ollama_names_are() {
        let mut s = settings_with(&[]);
        s.llm_model_history = vec![LlmModelSeen { key: "codex:gpt-6-astra".into(), status: STATUS_SEEN.into(), at: 1, replaced_by: None }];
        let c = s.llm_connections[1].clone();
        let offered = ["dall-e-4".to_string(), "qwen3:8b".to_string()];
        let api = record_offered(&s, &c, &offered, false, 5);
        assert!(api.iter().all(|h| h.status == STATUS_SEEN), "{api:?}");
        let local = record_offered(&s, &c, &offered, true, 5);
        assert!(local.iter().all(|h| h.status == STATUS_NEW), "{local:?}");
    }

    #[test]
    fn a_replaced_model_that_the_provider_still_lists_is_not_new_again() {
        let mut s = settings_with(&[("codex", "gpt-7-sol")]);
        s.llm_model_history = vec![
            LlmModelSeen { key: "codex:gpt-7-sol".into(), status: STATUS_SEEN.into(), at: 1, replaced_by: None },
            LlmModelSeen {
                key: "codex:gpt-6.1-sol".into(),
                status: STATUS_REPLACED.into(),
                at: 1,
                replaced_by: Some("codex:gpt-7-sol".into()),
            },
        ];
        let c = s.llm_connections[1].clone();
        assert!(record_offered(&s, &c, &["gpt-7-sol".into(), "gpt-6.1-sol".into()], false, 5).is_empty());
    }

    #[test]
    fn pending_pairs_a_new_model_with_the_older_one_of_its_family() {
        let mut s = settings_with(&[("claude", "claude-haiku-4-5-20251001"), ("claude", "claude-sonnet-5-5")]);
        s.llm_active_model_id = Some("claude:claude-haiku-4-5-20251001".into());
        s.llm_model_history = vec![LlmModelSeen {
            key: "claude:claude-haiku-5-5".into(),
            status: STATUS_NEW.into(),
            at: 1,
            replaced_by: None,
        }];
        let p = pending_updates(&s);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].replaces.as_deref(), Some("claude:claude-haiku-4-5-20251001"));
        assert!(p[0].replaces_active);
    }

    #[test]
    fn a_model_older_than_a_released_one_replaces_nothing() {
        let mut s = settings_with(&[("claude", "claude-haiku-5-5")]);
        s.llm_model_history = vec![LlmModelSeen {
            key: "claude:claude-haiku-4-5".into(),
            status: STATUS_NEW.into(),
            at: 1,
            replaced_by: None,
        }];
        let p = pending_updates(&s);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].replaces, None);
    }

    #[test]
    fn apply_replaces_moves_the_active_model_and_keeps_effort_and_prices() {
        let mut s = settings_with(&[("claude", "claude-haiku-4-5-20251001")]);
        s.llm_models[0].effort = Some("high".into());
        s.llm_models[0].price_input_per_mtok = Some(1.0);
        s.llm_models[0].price_output_per_mtok = Some(5.0);
        s.llm_active_model_id = Some("claude:claude-haiku-4-5-20251001".into());
        s.llm_model_history = vec![LlmModelSeen {
            key: "claude:claude-haiku-5-5".into(),
            status: STATUS_NEW.into(),
            at: 1,
            replaced_by: None,
        }];
        let update = pending_updates(&s).remove(0);
        let id = apply(&mut s, &update, 42).unwrap();
        assert_eq!(id, "claude:claude-haiku-5-5");
        assert_eq!(s.llm_models.len(), 1, "das aeltere verschwindet");
        assert_eq!(s.llm_models[0].remote_id, "claude-haiku-5-5");
        assert_eq!(s.llm_models[0].effort.as_deref(), Some("high"));
        assert_eq!(s.llm_models[0].price_input_per_mtok, Some(1.0));
        assert_eq!(s.llm_active_model_id.as_deref(), Some("claude:claude-haiku-5-5"));
        // Der Spiegel in die aelteren Felder zieht mit.
        assert_eq!(
            s.post_process_models.get("claude").map(String::as_str),
            Some("claude-haiku-5-5@high")
        );
        let old = entry(&s, "claude:claude-haiku-4-5-20251001").unwrap();
        assert_eq!(old.status, STATUS_REPLACED);
        assert_eq!(old.replaced_by.as_deref(), Some("claude:claude-haiku-5-5"));
        assert_eq!(entry(&s, "claude:claude-haiku-5-5").unwrap().status, STATUS_SEEN);
        assert!(pending_updates(&s).is_empty());
    }

    #[test]
    fn apply_leaves_the_active_model_alone_when_another_one_is_replaced() {
        let mut s = settings_with(&[("claude", "claude-haiku-4-5-20251001"), ("claude", "claude-sonnet-5-5")]);
        s.llm_active_model_id = Some("claude:claude-sonnet-5-5".into());
        s.llm_model_history = vec![LlmModelSeen {
            key: "claude:claude-haiku-5-5".into(),
            status: STATUS_NEW.into(),
            at: 1,
            replaced_by: None,
        }];
        let update = pending_updates(&s).remove(0);
        apply(&mut s, &update, 1).unwrap();
        assert_eq!(s.llm_active_model_id.as_deref(), Some("claude:claude-sonnet-5-5"));
    }

    #[test]
    fn apply_without_family_partner_just_releases_the_model() {
        let mut s = settings_with(&[("codex", "gpt-6-astra")]);
        s.llm_model_history = vec![LlmModelSeen {
            key: "codex:gpt-9-nova".into(),
            status: STATUS_NEW.into(),
            at: 1,
            replaced_by: None,
        }];
        let update = pending_updates(&s).remove(0);
        assert_eq!(update.replaces, None);
        apply(&mut s, &update, 1).unwrap();
        assert_eq!(s.llm_models.len(), 2);
        assert_eq!(s.llm_active_model_id, None, "kein Wechsel ohne Vorgaenger");
    }

    #[test]
    fn apply_refuses_a_blocked_connection() {
        let mut s = settings_with(&[("claude", "claude-haiku-4-5-20251001")]);
        s.compliance_profile = crate::managers::compliance::ComplianceProfile::LocalOnly;
        s.llm_model_history = vec![LlmModelSeen {
            key: "claude:claude-haiku-5-5".into(),
            status: STATUS_NEW.into(),
            at: 1,
            replaced_by: None,
        }];
        let update = pending_updates(&s).remove(0);
        let err = apply(&mut s, &update, 1).unwrap_err();
        assert!(err.contains(crate::managers::compliance::CODE_COMPLIANCE_BLOCKED), "{err}");
        assert_eq!(s.llm_models.len(), 1, "nichts verändert");
    }

    #[test]
    fn probe_outcomes_are_classified_conservatively() {
        assert_eq!(classify_probe(&Ok(())), Probe::Available);
        assert_eq!(
            classify_probe(&Err("cli_failed: There's an issue with the selected model (x). It may not exist".into())),
            Probe::Absent
        );
        assert_eq!(classify_probe(&Err("cli_model_not_in_plan: x".into())), Probe::Absent);
        for e in ["cli_limit_reached: x", "cli_not_logged_in: x", "cli_timeout: x", "cli_missing: x", "cli_failed: Netzwerk"] {
            assert_eq!(classify_probe(&Err(e.into())), Probe::Abort, "{e}");
        }
    }

    /// Echte Antwort von `claude -p --model claude-haiku-9-9` (09.10.2026), durch
    /// dieselbe Fehlerklassifizierung wie jeder CLI-Aufruf der App.
    #[test]
    fn the_real_cli_message_for_an_unknown_model_counts_as_absent() {
        let message = "There's an issue with the selected model (claude-haiku-9-9). It may not exist or you may not have access to it.";
        let classified = cli::classify_error(message);
        assert_eq!(classify_probe(&Err(classified)), Probe::Absent);
    }

    #[test]
    fn probes_stop_at_the_first_abort_reason() {
        let cands: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let mut calls = Vec::new();
        let run = run_probes(&cands, Duration::from_secs(60), |m| {
            calls.push(m.to_string());
            match m {
                "a" => Err("cli_failed: model not found".into()),
                "b" => Err("cli_limit_reached: x".into()),
                _ => Ok(()),
            }
        });
        assert_eq!(calls, ["a", "b"], "nach dem Limit kein weiterer Aufruf");
        assert_eq!(run.absent, ["a"]);
        assert!(run.available.is_empty());
    }

    #[test]
    fn probes_respect_the_time_budget() {
        let cands: Vec<String> = (0..5).map(|i| format!("m{i}")).collect();
        let run = run_probes(&cands, Duration::ZERO, |_| Ok(()));
        assert!(run.available.is_empty(), "ohne Zeit kein Aufruf");
    }

    #[test]
    fn due_candidates_are_capped_and_remember_absent_ones_for_a_week() {
        let mut s = settings_with(&[]);
        let known: Vec<String> = ["claude-haiku-4-5-20251001", "claude-sonnet-5-5", "claude-fable-5-1", "claude-opus-5-5"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let now = 1_000_000;
        assert_eq!(due_candidates(&s, "claude", &known, now).len(), PROBE_PER_RUN);
        // Alle Kandidaten gerade als abwesend gemerkt -> nichts zu tun.
        for name in claude_candidates(&known) {
            set_history(&mut s.llm_model_history, &LlmModelConfig::make_id("claude", &name), STATUS_ABSENT, now - 10, None);
        }
        assert!(due_candidates(&s, "claude", &known, now).is_empty());
        // Nach einer Woche wieder faellig.
        let later = now + ABSENT_RECHECK_SECS;
        assert_eq!(due_candidates(&s, "claude", &known, later).len(), PROBE_PER_RUN);
    }
}
