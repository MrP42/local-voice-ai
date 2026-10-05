//! Anbieterangaben fuer das Regelwerk: wo verarbeitet der Anbieter, ist er
//! nach dem EU-US Data Privacy Framework zertifiziert, trainiert er mit den
//! Daten, gibt es einen Auftragsverarbeitungsvertrag?
//!
//! Jede Zeile traegt ihre Quellen und das Pruefdatum. Was nicht belegt ist,
//! steht als `Unknown` -- und `Unknown` zaehlt nie als erfuellt. API und Abo
//! (Verbraucherplan) sind getrennte Zeilen: es gelten andere Bedingungen.

/// Ja / nein / unbekannt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tri {
    Yes,
    No,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderFacts {
    /// Vorlagenkennung (`openai`, `anthropic`, `mistral`, `claude_cli`, …).
    pub provider: &'static str,
    /// Nur bei passender Adresse (z. B. EU-Endpunkt); `None` = Standard.
    pub host: Option<&'static str>,
    /// Serverstandorte, ISO-3166-Kuerzel; "EU" fuer EU-weit.
    pub countries: &'static [&'static str],
    /// Verarbeitung (und Speicherung) in EU/EWR zugesagt.
    pub eu_processing: Tri,
    /// Auf der Liste des EU-US Data Privacy Framework.
    pub dpf_certified: Tri,
    /// Trainiert der Anbieter standardmaessig mit den Eingaben?
    pub trains_on_data: Tri,
    /// Auftragsverarbeitungsvertrag (DPA/AVV) fuer diesen Zugang erhaeltlich.
    pub dpa: Tri,
    /// Laesst sich das Training im Konto abschalten? Dann zaehlt die
    /// Bestaetigung des Nutzers an der Verbindung (`training_opt_out`).
    pub training_opt_out_possible: bool,
    pub sources: &'static [&'static str],
    /// Pruefdatum `YYYY-MM-DD`.
    pub checked: &'static str,
}

const CHECKED: &str = "2026-10-06";

/// Die Tabelle, geprueft am 06.10.2026 (Spike-Recherche, Primaerquellen
/// gegengelesen). Den DPF-Status konnten wir nicht selbst abfragen -- er steht
/// deshalb ueberall auf `Unknown`; fuer kein Ergebnis ist er entscheidend.
/// Eintraege mit `host` gehen vor dem Standard derselben Vorlage.
pub static FACTS: &[ProviderFacts] = &[
    // Anthropic API: Inferenz "global" oder "us", Speicherung nur "us"; kein
    // Training ohne Opt-in; DPA in den Commercial Terms.
    ProviderFacts {
        provider: "anthropic",
        host: None,
        countries: &["US"],
        eu_processing: Tri::No,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::No,
        dpa: Tri::Yes,
        training_opt_out_possible: false,
        sources: &[
            "https://platform.claude.com/docs/en/manage-claude/data-residency",
            "https://www.anthropic.com/legal/data-processing-addendum",
        ],
        checked: CHECKED,
    },
    // Claude Pro/Max ueber Claude Code: Verbraucherkonto, USA, Training je nach
    // Schalter (abschaltbar), keine DPA -- Anthropic ist Verantwortlicher.
    ProviderFacts {
        provider: "claude_cli",
        host: None,
        countries: &["US"],
        eu_processing: Tri::No,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::Yes,
        dpa: Tri::No,
        training_opt_out_possible: true,
        sources: &[
            "https://code.claude.com/docs/en/data-usage",
            "https://code.claude.com/docs/en/legal-and-compliance",
            "https://www.anthropic.com/legal/consumer-terms",
        ],
        checked: CHECKED,
    },
    // OpenAI API, Standard: USA; kein Training mit API-Daten; DPA.
    ProviderFacts {
        provider: "openai",
        host: None,
        countries: &["US"],
        eu_processing: Tri::No,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::No,
        dpa: Tri::Yes,
        training_opt_out_possible: false,
        sources: &[
            "https://developers.openai.com/api/docs/guides/your-data",
            "https://openai.com/policies/data-processing-addendum/",
        ],
        checked: CHECKED,
    },
    // OpenAI API mit EU-Datenresidenz: EU-Projekt + eu.api.openai.com
    // (Freigabe und Modified-Retention-Zusatz noetig -- ohne beides lehnt der
    // Endpunkt ab).
    ProviderFacts {
        provider: "openai_eu",
        host: None,
        countries: &["EU"],
        eu_processing: Tri::Yes,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::No,
        dpa: Tri::Yes,
        training_opt_out_possible: false,
        sources: &["https://developers.openai.com/api/docs/guides/your-data"],
        checked: CHECKED,
    },
    // ChatGPT Plus/Pro ueber Codex: Verbraucherkonto, USA, Training abschaltbar,
    // keine DPA.
    ProviderFacts {
        provider: "codex_cli",
        host: None,
        countries: &["US"],
        eu_processing: Tri::No,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::Yes,
        dpa: Tri::No,
        training_opt_out_possible: true,
        sources: &[
            "https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan",
            "https://openai.com/policies/eu-privacy-policy/",
            "https://openai.com/policies/eu-terms-of-use/",
        ],
        checked: CHECKED,
    },
    // Mistral API: standardmaessig in der EU gehostet (der US-Endpunkt nur auf
    // ausdruecklichen Wunsch); Pay-as-you-go trainiert, bis man widerspricht;
    // DPA mit SCC.
    ProviderFacts {
        provider: "mistral",
        host: None,
        countries: &["EU"],
        eu_processing: Tri::Yes,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::Yes,
        dpa: Tri::Yes,
        training_opt_out_possible: true,
        sources: &[
            "https://help.mistral.ai/en/articles/347629",
            "https://help.mistral.ai/en/articles/347617",
            "https://legal.mistral.ai/terms/data-processing-addendum",
        ],
        checked: CHECKED,
    },
    // AWS Bedrock (OpenAI-kompatibler Mantle-Endpunkt): die Region bestimmt den
    // Standort. Modellanbieter sehen weder Eingaben noch Antworten; Daten
    // verbessern die Basismodelle nicht; AWS-DPA (GDPR).
    ProviderFacts {
        provider: "bedrock_mantle",
        host: Some("bedrock-mantle.eu-central-1.api.aws"),
        countries: &["DE"],
        eu_processing: Tri::Yes,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::No,
        dpa: Tri::Yes,
        training_opt_out_possible: false,
        sources: BEDROCK_SOURCES,
        checked: CHECKED,
    },
    ProviderFacts {
        provider: "bedrock_mantle",
        host: Some("bedrock-mantle.eu-west-1.api.aws"),
        countries: &["IE"],
        eu_processing: Tri::Yes,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::No,
        dpa: Tri::Yes,
        training_opt_out_possible: false,
        sources: BEDROCK_SOURCES,
        checked: CHECKED,
    },
    ProviderFacts {
        provider: "bedrock_mantle",
        host: Some("bedrock-mantle.eu-south-1.api.aws"),
        countries: &["IT"],
        eu_processing: Tri::Yes,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::No,
        dpa: Tri::Yes,
        training_opt_out_possible: false,
        sources: BEDROCK_SOURCES,
        checked: CHECKED,
    },
    ProviderFacts {
        provider: "bedrock_mantle",
        host: Some("bedrock-mantle.eu-north-1.api.aws"),
        countries: &["SE"],
        eu_processing: Tri::Yes,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::No,
        dpa: Tri::Yes,
        training_opt_out_possible: false,
        sources: BEDROCK_SOURCES,
        checked: CHECKED,
    },
    // Alle anderen Bedrock-Regionen: ausserhalb von EU/EWR.
    ProviderFacts {
        provider: "bedrock_mantle",
        host: None,
        countries: &["US"],
        eu_processing: Tri::No,
        dpf_certified: Tri::Unknown,
        trains_on_data: Tri::No,
        dpa: Tri::Yes,
        training_opt_out_possible: false,
        sources: BEDROCK_SOURCES,
        checked: CHECKED,
    },
];

const BEDROCK_SOURCES: &[&str] = &[
    "https://docs.aws.amazon.com/bedrock/latest/userguide/data-protection.html",
    "https://aws.amazon.com/bedrock/security-compliance/",
    "https://docs.aws.amazon.com/bedrock/latest/userguide/endpoints-region-availability.html",
];

/// Bedrock-Regionen, die die Oberflaeche anbietet (Mantle-Endpunkt
/// `bedrock-mantle.<region>.api.aws`), EU zuerst. London fehlt bewusst: nicht
/// EU/EWR, und den Angemessenheitsbeschluss haben wir nicht geprueft.
pub const BEDROCK_REGIONS: &[&str] = &[
    "eu-central-1",
    "eu-west-1",
    "eu-south-1",
    "eu-north-1",
    "us-east-1",
    "us-east-2",
    "us-west-2",
];

/// Angaben zu Vorlage und Adresse: zuerst ein Eintrag mit passendem Host,
/// sonst der Standard der Vorlage, sonst `None` (unbekannt).
pub fn facts_for(provider: &str, base_url: &str) -> Option<&'static ProviderFacts> {
    let host = url::Url::parse(base_url.trim())
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase));
    FACTS
        .iter()
        .find(|f| f.provider == provider && f.host.is_some() && f.host.map(str::to_string) == host)
        .or_else(|| FACTS.iter().find(|f| f.provider == provider && f.host.is_none()))
}
