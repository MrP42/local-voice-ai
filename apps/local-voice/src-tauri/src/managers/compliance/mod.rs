//! Regelwerk fuer Sprachmodelle: darf dieses Modell unter dem gewaehlten
//! Profil (EU, nur lokal, keine Einschraenkung) benutzt werden -- und warum?
//!
//! Grundlage sind dokumentierte Anbieterangaben (`facts`), nicht Vermutungen:
//! was unbekannt ist, zaehlt nie als erfuellt. Die Bewertung ersetzt keine
//! Datenschutzpruefung; sie macht sichtbar, was die Anbieter selbst zusagen.
//!
//! Durchgesetzt wird an drei Stellen: beim Aktivieren eines Modells, in der
//! Auswahl (ausgegraut mit Grund) und vor jedem Aufruf in `llm_client` --
//! wie das harte Budget.

pub mod facts;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::settings::PostProcessProvider;

pub use facts::{facts_for, ProviderFacts, Tri};

/// Fehlercode, mit dem ein gesperrter Aufruf abgewiesen wird.
pub const CODE_COMPLIANCE_BLOCKED: &str = "compliance_blocked";

/// Nach so vielen Tagen gelten Anbieterangaben als veraltet: das Modell
/// bleibt erlaubt, das Schild wird gelb.
pub const FACTS_MAX_AGE_DAYS: i64 = 180;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceProfile {
    /// EU-KI-VO und DSGVO: Verarbeitung in EU/EWR, oder in einem Drittland mit
    /// Angemessenheitsbeschluss (EU-US Data Privacy Framework) -- jeweils mit
    /// Auftragsverarbeitungsvertrag und ohne Training mit den Daten.
    #[default]
    Eu,
    /// Nichts verlaesst den Rechner.
    LocalOnly,
    /// Keine Einschraenkung; Cloud-Modelle werden nur gekennzeichnet.
    None,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allowed,
    /// Erlaubt, aber mit einer Bedingung oder Unsicherheit (Schild gelb).
    Conditional,
    Blocked,
}

/// Bewertung eines Modells unter einem Profil.
#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct Assessment {
    pub verdict: Verdict,
    /// Laeuft das Modell bei einem Anbieter (nicht auf diesem Rechner)?
    pub cloud: bool,
    /// Serverstandorte als ISO-3166-Kuerzel ("US", "FR"), "EU" fuer EU-weit.
    pub countries: Vec<String>,
    /// Gruende als Kennungen (uebersetzt in der Oberflaeche), wichtigster zuerst.
    pub reasons: Vec<String>,
    /// Woher die Angaben stammen (URL) und wann sie geprueft wurden.
    pub sources: Vec<String>,
    pub checked: Option<String>,
}

impl Assessment {
    fn local() -> Self {
        Self {
            verdict: Verdict::Allowed,
            cloud: false,
            countries: Vec::new(),
            reasons: vec!["local".into()],
            sources: Vec::new(),
            checked: None,
        }
    }
}

/// Laeuft der Anbieter auf diesem Rechner? Dieselbe Regel wie im
/// Verbrauchsbuch (eingebauter Server oder Loopback-Adresse).
pub fn is_local(provider: &PostProcessProvider) -> bool {
    crate::managers::usage::provider_is_local(provider)
}

/// Bewertet einen Anbieter (Vorlage + Adresse) unter `profile`. `today` als
/// `YYYY-MM-DD` fuer das Alter der Angaben (Tests geben es vor).
/// `training_opt_out`: der Nutzer hat bestaetigt, dass das Training im Konto
/// abgeschaltet ist (zaehlt nur, wo der Anbieter das anbietet).
pub fn assess(
    profile: ComplianceProfile,
    provider: &PostProcessProvider,
    training_opt_out: bool,
    today: &str,
) -> Assessment {
    if is_local(provider) {
        return Assessment::local();
    }
    let facts = facts_for(&provider.id, &provider.base_url);
    let countries = facts
        .map(|f| f.countries.iter().map(|c| c.to_string()).collect())
        .unwrap_or_default();
    let sources = facts
        .map(|f| f.sources.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let checked = facts.map(|f| f.checked.to_string());
    let (verdict, reasons) = match profile {
        ComplianceProfile::None => (Verdict::Allowed, vec!["no_restriction".to_string()]),
        ComplianceProfile::LocalOnly => (Verdict::Blocked, vec!["cloud_not_allowed".to_string()]),
        ComplianceProfile::Eu => match facts {
            None => (Verdict::Blocked, vec!["facts_unknown".to_string()]),
            Some(f) => eu_verdict(f, training_opt_out, today),
        },
    };
    Assessment {
        verdict,
        cloud: true,
        countries,
        reasons,
        sources,
        checked,
    }
}

/// Die EU-Regel. Gesperrt, sobald eine Bedingung sicher verletzt oder eine
/// Pflichtangabe unbekannt ist; erlaubt mit Bedingung bei Drittland mit DPF
/// oder veralteten Angaben.
fn eu_verdict(f: &ProviderFacts, training_opt_out: bool, today: &str) -> (Verdict, Vec<String>) {
    let mut blocked = Vec::new();
    let mut conditions = Vec::new();
    let mut notes = Vec::new();
    match f.trains_on_data {
        Tri::Yes if f.training_opt_out_possible && training_opt_out => {
            notes.push("training_opt_out_attested")
        }
        Tri::Yes if f.training_opt_out_possible => blocked.push("training_opt_out_missing"),
        Tri::Yes => blocked.push("trains_on_data"),
        Tri::Unknown => blocked.push("training_unknown"),
        Tri::No => {}
    }
    match f.dpa {
        Tri::No => blocked.push("no_dpa"),
        Tri::Unknown => blocked.push("dpa_unknown"),
        Tri::Yes => {}
    }
    match (f.eu_processing, f.dpf_certified) {
        (Tri::Yes, _) => {}
        (_, Tri::Yes) => conditions.push("third_country_dpf"),
        _ => blocked.push("third_country_no_safeguard"),
    }
    if facts_age_days(f.checked, today).is_none_or(|d| d > FACTS_MAX_AGE_DAYS) {
        conditions.push("facts_stale");
    }
    if !blocked.is_empty() {
        (Verdict::Blocked, blocked.into_iter().map(String::from).collect())
    } else if !conditions.is_empty() {
        conditions.extend(notes);
        (Verdict::Conditional, conditions.into_iter().map(String::from).collect())
    } else {
        let mut reasons = vec!["eu_ok"];
        reasons.extend(notes);
        (Verdict::Allowed, reasons.into_iter().map(String::from).collect())
    }
}

/// Tage zwischen zwei Daten `YYYY-MM-DD`; `None` bei unlesbarem Datum.
fn facts_age_days(checked: &str, today: &str) -> Option<i64> {
    let parse = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
    Some((parse(today)? - parse(checked)?).num_days())
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Vor jedem Aufruf (`llm_client`): unter dem eingestellten Profil gesperrte
/// Anbieter werden abgewiesen, bevor etwas das Geraet verlaesst. Ohne
/// Einstellungen (Tests, frueher Start) gilt das Standardprofil.
pub fn check_call(provider: &PostProcessProvider) -> Result<(), String> {
    let settings = crate::managers::usage::settings_snapshot();
    let profile = settings.as_ref().map(|s| s.compliance_profile).unwrap_or_default();
    let opt_out = settings.as_ref().is_some_and(|s| training_opt_out_for(s, provider));
    let a = assess(profile, provider, opt_out, &today());
    if a.verdict == Verdict::Blocked {
        record_block(&provider.label);
        return Err(format!(
            "{CODE_COMPLIANCE_BLOCKED}: {} ({})",
            provider.label,
            a.reasons.join(", ")
        ));
    }
    Ok(())
}

/// Hat der Nutzer an einer passenden Verbindung (gleiche Vorlage und
/// Adresse) bestaetigt, dass das Training abgeschaltet ist?
pub fn training_opt_out_for(settings: &crate::settings::AppSettings, provider: &PostProcessProvider) -> bool {
    settings.llm_connections.iter().any(|c| {
        c.kind == provider.id
            && c.base_url.trim_end_matches('/') == provider.base_url.trim_end_matches('/')
            && c.training_opt_out
    })
}

// ---------------------------------------------------------------------------
// Gesperrte Versuche (fuer das Schild)
// ---------------------------------------------------------------------------

/// Gesperrte Aufrufe der letzten Zeit: (Unix-Sekunden, Anbieter). Sie stehen
/// nicht im Verbrauchsbuch -- es wurde ja nichts gesendet.
static BLOCKS: std::sync::Mutex<Vec<(i64, String)>> = std::sync::Mutex::new(Vec::new());

fn record_block(provider_label: &str) {
    let now = chrono::Utc::now().timestamp();
    let mut blocks = BLOCKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    blocks.retain(|(ts, _)| now - ts < 86_400);
    blocks.push((now, provider_label.to_string()));
}

/// Gesperrte Versuche der letzten 24 Stunden, neueste zuerst.
pub fn recent_blocks() -> Vec<(i64, String)> {
    let now = chrono::Utc::now().timestamp();
    let blocks = BLOCKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut out: Vec<_> = blocks.iter().filter(|(ts, _)| now - ts < 86_400).cloned().collect();
    out.reverse();
    out
}

// ---------------------------------------------------------------------------
// Schild
// ---------------------------------------------------------------------------

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Type)]
#[serde(rename_all = "snake_case")]
pub enum ShieldLevel {
    Green,
    Yellow,
    Red,
}

/// Ein Pruefpunkt der Schild-Erklaerung. `id` wird in der Oberflaeche
/// uebersetzt, `detail` traegt Namen und Zahlen.
#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct ShieldCheck {
    pub id: String,
    pub level: ShieldLevel,
    pub detail: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct ShieldStatus {
    pub level: ShieldLevel,
    pub profile: ComplianceProfile,
    pub active_model: Option<String>,
    pub active: Option<Assessment>,
    pub checks: Vec<ShieldCheck>,
}

/// Was das Schild zusaetzlich zum aktiven Modell wissen muss.
#[derive(Debug, Clone, Default)]
pub struct ShieldInputs {
    /// Anzahl gespeicherter API-Schluessel, die noch im Klartext liegen.
    pub plaintext_keys: usize,
    /// Gesperrte Versuche der letzten 24 h (Anbieter).
    pub blocks_24h: Vec<String>,
    /// Fehlgeschlagene Cloud-Aufrufe der letzten 24 h.
    pub cloud_errors_24h: usize,
}

/// Fasst alles zu einer Ampel zusammen; das Schlimmste bestimmt die Farbe.
pub fn shield(
    profile: ComplianceProfile,
    active: Option<(&str, &PostProcessProvider, bool)>,
    inputs: &ShieldInputs,
    today: &str,
) -> ShieldStatus {
    let mut checks = vec![ShieldCheck {
        id: format!("profile_{}", profile_key(profile)),
        level: ShieldLevel::Green,
        detail: None,
    }];
    let assessment = active.map(|(_, p, opt_out)| assess(profile, p, opt_out, today));
    match (&assessment, active) {
        (Some(a), Some((label, _, _))) => checks.push(ShieldCheck {
            id: match (a.cloud, a.verdict) {
                (false, _) => "model_local",
                (true, Verdict::Allowed) => "model_cloud_ok",
                (true, Verdict::Conditional) => "model_cloud_conditional",
                (true, Verdict::Blocked) => "model_blocked",
            }
            .to_string(),
            level: match a.verdict {
                Verdict::Allowed => ShieldLevel::Green,
                Verdict::Conditional => ShieldLevel::Yellow,
                Verdict::Blocked => ShieldLevel::Red,
            },
            detail: Some(label.to_string()),
        }),
        _ => checks.push(ShieldCheck {
            id: "model_none".into(),
            level: ShieldLevel::Green,
            detail: None,
        }),
    }
    checks.push(if inputs.plaintext_keys > 0 {
        ShieldCheck {
            id: "keys_plaintext".into(),
            level: ShieldLevel::Yellow,
            detail: Some(inputs.plaintext_keys.to_string()),
        }
    } else {
        ShieldCheck {
            id: "keys_protected".into(),
            level: ShieldLevel::Green,
            detail: None,
        }
    });
    if !inputs.blocks_24h.is_empty() {
        let mut names = inputs.blocks_24h.clone();
        names.sort();
        names.dedup();
        checks.push(ShieldCheck {
            id: "blocked_calls".into(),
            level: ShieldLevel::Red,
            detail: Some(format!("{} × {}", inputs.blocks_24h.len(), names.join(", "))),
        });
    }
    if inputs.cloud_errors_24h > 0 {
        checks.push(ShieldCheck {
            id: "cloud_errors".into(),
            level: ShieldLevel::Yellow,
            detail: Some(inputs.cloud_errors_24h.to_string()),
        });
    }
    let level = checks.iter().map(|c| c.level).max().unwrap_or(ShieldLevel::Green);
    ShieldStatus {
        level,
        profile,
        active_model: active.map(|(l, _, _)| l.to_string()),
        active: assessment,
        checks,
    }
}

fn profile_key(p: ComplianceProfile) -> &'static str {
    match p {
        ComplianceProfile::Eu => "eu",
        ComplianceProfile::LocalOnly => "local_only",
        ComplianceProfile::None => "none",
    }
}

#[cfg(test)]
mod tests;
