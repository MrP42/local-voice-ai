//! D7-Rest: welche Anbieter für eine Cloud-Ausnahme (Spec §5.1) in Frage kommen.
//!
//! Die Liste kommt nie aus der Oberfläche: sie folgt den konfigurierten Verbindungen, dem Regelwerk
//! (Compliance-Profil) und der höchsten Datenklasse, die ein Ziel annimmt – dieselbe Regel wie beim Abruf
//! (`projekte::tor_ziel::datenklasse_max`), damit Dialog und Versandtor nie auseinanderlaufen. Eine
//! Ausnahme gilt je Anbieterart (Vorlagenkennung der Verbindung, wie in `freigabe.json`).
//!
//! Nie Schlüssel, Adressen oder Namen von Ordnern in Log oder Fehlertext.

use crate::commands::compliance::provider_for_connection;
use crate::managers::compliance::{self, ComplianceProfile, Verdict};
use crate::projekte::tor_ziel::datenklasse_max;
use crate::settings::AppSettings;
use serde::Serialize;
use std::collections::BTreeMap;

use super::zusammenfuehren::Klasse;

/// Grund, warum ein Anbieter nicht wählbar ist: das Regelwerk sperrt ihn.
pub const GRUND_REGELWERK: &str = "regelwerk";
/// Grund, warum ein Anbieter nicht wählbar ist: er nimmt höchstens „Intern“ an.
pub const GRUND_KLASSE: &str = "klasse";

/// Eine Anbieterart für Cloud-Ausnahmen (D7-Rest): konfigurierte Verbindung, gefiltert nach Regelwerk und
/// Datenklasse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageAnbieter {
    /// Kennung wie in `freigabe.json` (Vorlagenkennung der Verbindung).
    pub id: String,
    /// Bezeichnung der Verbindung (Anzeige).
    pub name: String,
    /// Für „Vertraulich“ wählbar: Regelwerk erlaubt den Anbieter und er nimmt `confidential` an.
    pub waehlbar: bool,
    /// Warum nicht wählbar: `regelwerk` (der Anbieter erfüllt das Regelwerk nicht) oder `klasse` (er nimmt
    /// höchstens „Intern“ an); sonst `null`.
    pub grund: Option<String>,
}

/// Anbieterliste für die Cloud-Ausnahme eines Ordners.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageAnbieterListe {
    /// Aktives Regelwerk: `eu`, `nur_lokal` oder `keines`.
    pub regelwerk: String,
    pub anbieter: Vec<AblageAnbieter>,
}

fn regelwerk(profil: ComplianceProfile) -> &'static str {
    match profil {
        ComplianceProfile::Eu => "eu",
        ComplianceProfile::LocalOnly => "nur_lokal",
        ComplianceProfile::None => "keines",
    }
}

/// Die Anbieterarten der aktivierten, nicht lokalen Verbindungen mit ihrem Stand. Eine Art mit mehreren
/// Verbindungen ist wählbar, sobald eine davon es ist (der Abruf prüft die Verbindung des Ziels).
/// Wählbare zuerst, dann nach Bezeichnung.
pub fn anbieterliste(s: &AppSettings, heute: &str) -> AblageAnbieterListe {
    let mut je_art: BTreeMap<String, AblageAnbieter> = BTreeMap::new();
    for c in s.llm_connections.iter().filter(|c| c.enabled) {
        let provider = provider_for_connection(c);
        if crate::managers::llm::is_local(&provider) {
            continue;
        }
        let bewertung =
            compliance::assess(s.compliance_profile, &provider, c.training_opt_out, heute);
        let klasse = datenklasse_max(s.compliance_profile, false, &bewertung);
        let waehlbar = bewertung.verdict != Verdict::Blocked && klasse >= Klasse::Confidential;
        let grund = if waehlbar {
            None
        } else if bewertung.verdict == Verdict::Blocked {
            Some(GRUND_REGELWERK.to_string())
        } else {
            Some(GRUND_KLASSE.to_string())
        };
        let neu = AblageAnbieter {
            id: c.kind.clone(),
            name: c.label.clone(),
            waehlbar,
            grund,
        };
        match je_art.get(&c.kind) {
            Some(alt) if alt.waehlbar || !neu.waehlbar => {}
            _ => {
                je_art.insert(c.kind.clone(), neu);
            }
        }
    }
    let mut anbieter: Vec<AblageAnbieter> = je_art.into_values().collect();
    anbieter.sort_by(|a, b| {
        b.waehlbar
            .cmp(&a.waehlbar)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    AblageAnbieterListe {
        regelwerk: regelwerk(s.compliance_profile).to_string(),
        anbieter,
    }
}

/// Darf für diese Anbieterart jetzt eine Ausnahme erteilt werden? Nur für wählbare Anbieter der Liste;
/// entziehen geht immer.
pub fn zulaessig(s: &AppSettings, heute: &str, id: &str) -> bool {
    anbieterliste(s, heute)
        .anbieter
        .iter()
        .any(|a| a.waehlbar && a.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{get_default_settings, LlmConnection};

    fn verbindung(id: &str, kind: &str, label: &str, url: &str, opt_out: bool) -> LlmConnection {
        LlmConnection {
            id: id.into(),
            kind: kind.into(),
            label: label.into(),
            base_url: url.into(),
            enabled: true,
            monthly_budget_usd: None,
            budget_enforced: false,
            training_opt_out: opt_out,
        }
    }

    fn url(kind: &str) -> String {
        get_default_settings()
            .post_process_provider(kind)
            .unwrap()
            .base_url
            .clone()
    }

    /// Einstellungen mit Verbindungen unter dem Profil EU und ein Datum, an dem die Angaben aktuell sind.
    fn eu(verbindungen: Vec<LlmConnection>) -> (AppSettings, String) {
        let mut s = get_default_settings();
        s.compliance_profile = ComplianceProfile::Eu;
        s.llm_connections = verbindungen;
        let heute = compliance::facts_for("openai_eu", &url("openai_eu"))
            .unwrap()
            .checked
            .to_string();
        (s, heute)
    }

    fn ids(l: &AblageAnbieterListe) -> Vec<(&str, bool, Option<&str>)> {
        l.anbieter
            .iter()
            .map(|a| (a.id.as_str(), a.waehlbar, a.grund.as_deref()))
            .collect()
    }

    #[test]
    fn eu_listet_cloud_verbindungen_mit_grund_und_laesst_lokales_und_abgeschaltetes_weg() {
        let mut aus = verbindung("v4", "openai_eu", "Abgeschaltet", &url("openai_eu"), false);
        aus.enabled = false;
        let (s, heute) = eu(vec![
            verbindung("v1", "anthropic", "Claude API", &url("anthropic"), false),
            verbindung("v2", "openai_eu", "OpenAI (EU)", &url("openai_eu"), false),
            verbindung("v3", "ollama", "Ollama", &url("ollama"), false),
            verbindung(
                "v5",
                "local",
                "Eingebaut",
                crate::managers::llm::LOCAL_PLACEHOLDER_URL,
                false,
            ),
            aus,
        ]);
        let l = anbieterliste(&s, &heute);
        assert_eq!(l.regelwerk, "eu");
        // Wählbare zuerst, dann nach Bezeichnung; Loopback gilt nicht als lokal, nimmt aber kein
        // „Vertraulich“ an; die eingebaute lokale Verbindung braucht keine Ausnahme.
        assert_eq!(
            ids(&l),
            vec![
                ("openai_eu", true, None),
                ("anthropic", false, Some(GRUND_REGELWERK)),
                ("ollama", false, Some(GRUND_KLASSE)),
            ]
        );
        assert_eq!(l.anbieter[0].name, "OpenAI (EU)");
    }

    #[test]
    fn veraltete_angaben_machen_den_anbieter_nicht_waehlbar() {
        let (s, _) = eu(vec![verbindung(
            "v1",
            "openai_eu",
            "OpenAI (EU)",
            &url("openai_eu"),
            false,
        )]);
        let l = anbieterliste(&s, "2099-01-01");
        assert_eq!(ids(&l), vec![("openai_eu", false, Some(GRUND_KLASSE))]);
    }

    #[test]
    fn nur_lokal_sperrt_alle_und_keines_nimmt_nie_vertraulich() {
        let (mut s, heute) = eu(vec![verbindung(
            "v1",
            "openai_eu",
            "OpenAI (EU)",
            &url("openai_eu"),
            false,
        )]);
        s.compliance_profile = ComplianceProfile::LocalOnly;
        let l = anbieterliste(&s, &heute);
        assert_eq!(l.regelwerk, "nur_lokal");
        assert_eq!(ids(&l), vec![("openai_eu", false, Some(GRUND_REGELWERK))]);
        s.compliance_profile = ComplianceProfile::None;
        let l = anbieterliste(&s, &heute);
        assert_eq!(l.regelwerk, "keines");
        assert_eq!(ids(&l), vec![("openai_eu", false, Some(GRUND_KLASSE))]);
    }

    #[test]
    fn eine_art_mit_mehreren_verbindungen_ist_waehlbar_sobald_eine_es_ist() {
        let heute = compliance::facts_for("mistral", &url("mistral"))
            .unwrap()
            .checked
            .to_string();
        let mut s = get_default_settings();
        s.compliance_profile = ComplianceProfile::Eu;
        s.llm_connections = vec![
            verbindung("v1", "mistral", "Mistral A", &url("mistral"), false),
            verbindung("v2", "mistral", "Mistral B", &url("mistral"), true),
        ];
        let l = anbieterliste(&s, &heute);
        assert_eq!(ids(&l), vec![("mistral", true, None)]);
        assert_eq!(
            l.anbieter[0].name, "Mistral B",
            "die wählbare Verbindung benennt die Art"
        );
        // Reihenfolge der Verbindungen spielt keine Rolle.
        s.llm_connections.reverse();
        assert_eq!(anbieterliste(&s, &heute), l);
        // Ohne Bestätigung „Training abgeschaltet“ ist keine wählbar; Grund: das Regelwerk.
        s.llm_connections[0].training_opt_out = false;
        s.llm_connections[1].training_opt_out = false;
        let l = anbieterliste(&s, &heute);
        assert_eq!(ids(&l), vec![("mistral", false, Some(GRUND_REGELWERK))]);
    }

    #[test]
    fn erteilen_nur_fuer_waehlbare_anbieter() {
        let (s, heute) = eu(vec![
            verbindung("v1", "openai_eu", "OpenAI (EU)", &url("openai_eu"), false),
            verbindung("v2", "anthropic", "Claude API", &url("anthropic"), false),
        ]);
        assert!(zulaessig(&s, &heute, "openai_eu"));
        assert!(!zulaessig(&s, &heute, "anthropic"), "Regelwerk sperrt");
        assert!(!zulaessig(&s, &heute, "unbekannt"), "nicht konfiguriert");
        assert!(!zulaessig(&s, &heute, ""));
        assert!(
            !zulaessig(&s, "2099-01-01", "openai_eu"),
            "Angaben veraltet"
        );
    }

    #[test]
    fn eine_leere_konfiguration_liefert_eine_leere_liste_mit_regelwerk() {
        let (s, heute) = eu(vec![]);
        let l = anbieterliste(&s, &heute);
        assert_eq!(l.regelwerk, "eu");
        assert!(l.anbieter.is_empty());
    }
}
