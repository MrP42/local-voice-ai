//! Datentypen der Besprechungsplanung (Konzept `docs/plans/2026-11-12_besprechungsplanung.md`, 3.2).
//!
//! Alle Zeiten sind Millisekunden UTC (`i64`), wie im Kalender der App. Die Typen
//! sind reine Daten; Pruefung, Rendering, Abgleich und Speicher liegen nebenan.

use serde::{Deserialize, Serialize};

/// Art des Ziels einer Besprechung. Bestimmt den Satzanfang des Zielsatzes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum GoalType {
    Klaeren,
    Feedback,
    Entscheiden,
    Information,
}

impl GoalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            GoalType::Klaeren => "klaeren",
            GoalType::Feedback => "feedback",
            GoalType::Entscheiden => "entscheiden",
            GoalType::Information => "information",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "klaeren" => GoalType::Klaeren,
            "feedback" => GoalType::Feedback,
            "entscheiden" => GoalType::Entscheiden,
            "information" => GoalType::Information,
            _ => return None,
        })
    }

    /// Sichtbare Bezeichnung.
    pub fn label(&self) -> &'static str {
        match self {
            GoalType::Klaeren => "Klären",
            GoalType::Feedback => "Feedback einholen",
            GoalType::Entscheiden => "Entscheiden",
            GoalType::Information => "Informieren",
        }
    }
}

/// Lebenslauf einer Planung. Die Reihenfolge ist die des Ablaufs; `Verschoben`
/// und `Abgesagt` setzt nur der Abgleich mit dem Kalender.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Entwurf,
    Uebergeben,
    Verknuepft,
    Verschoben,
    Abgesagt,
    Durchgefuehrt,
    Protokolliert,
    Versendet,
}

impl PlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanStatus::Entwurf => "entwurf",
            PlanStatus::Uebergeben => "uebergeben",
            PlanStatus::Verknuepft => "verknuepft",
            PlanStatus::Verschoben => "verschoben",
            PlanStatus::Abgesagt => "abgesagt",
            PlanStatus::Durchgefuehrt => "durchgefuehrt",
            PlanStatus::Protokolliert => "protokolliert",
            PlanStatus::Versendet => "versendet",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "entwurf" => PlanStatus::Entwurf,
            "uebergeben" => PlanStatus::Uebergeben,
            "verknuepft" => PlanStatus::Verknuepft,
            "verschoben" => PlanStatus::Verschoben,
            "abgesagt" => PlanStatus::Abgesagt,
            "durchgefuehrt" => PlanStatus::Durchgefuehrt,
            "protokolliert" => PlanStatus::Protokolliert,
            "versendet" => PlanStatus::Versendet,
            _ => return None,
        })
    }

    /// Hat die Besprechung schon stattgefunden? Dann aendert der Kalender den
    /// Status nicht mehr (eine spaete Absage ueberschreibt kein Protokoll).
    pub fn ist_abgeschlossen(&self) -> bool {
        matches!(
            self,
            PlanStatus::Durchgefuehrt | PlanStatus::Protokolliert | PlanStatus::Versendet
        )
    }
}

/// Ein Agendapunkt mit Zeitbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AgendaItem {
    pub minutes: u32,
    pub topic: String,
    /// Verantwortliche Person (Anzeigename), frei.
    #[serde(default)]
    pub owner: Option<String>,
}

/// Rollen in der Besprechung (frei, Anzeigenamen).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Roles {
    #[serde(default)]
    pub moderation: Option<String>,
    #[serde(default)]
    pub ergebnissicherung: Option<String>,
    #[serde(default)]
    pub entscheidung: Option<String>,
}

/// Eine Unterlage, als Link (Datenminimierung: Links statt Anhaenge).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Material {
    pub label: String,
    #[serde(default)]
    pub url: Option<String>,
    /// Bis wann lesen, `YYYY-MM-DD` (frei, nur angezeigt).
    #[serde(default)]
    pub due_date: Option<String>,
}

/// Hinweise im Fuss der Einladung.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Hints {
    /// Darf die Einladung weitergeleitet werden?
    #[serde(default)]
    pub weiterleiten_erlaubt: Option<bool>,
    #[serde(default)]
    pub barrierefreiheit: Option<String>,
    /// Bis wann das Ergebnis vorliegt, frei.
    #[serde(default)]
    pub ergebnisfrist: Option<String>,
    #[serde(default)]
    pub ablageort: Option<String>,
}

/// Eine Planung, wie Oberflaeche und Befehle sie sehen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Plan {
    /// ULID/UUID der App; zugleich die Marke im Termin (`lva-plan:<id>`).
    pub id: String,
    /// Die Besprechung (leerer Eintrag), sobald angelegt.
    pub meeting_id: Option<String>,
    pub title: String,
    pub goal_type: GoalType,
    pub goal_text: String,
    pub expected_result: Option<String>,
    pub duration_min: u32,
    pub classification: String,
    pub agenda: Vec<AgendaItem>,
    pub roles: Roles,
    pub materials: Vec<Material>,
    pub hints: Hints,
    pub transcription_planned: bool,
    pub minutes_template_id: Option<String>,
    pub status: PlanStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Was die Oberflaeche zum Anlegen oder Aendern schickt (ohne Status und Zeiten).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PlanInput {
    pub title: String,
    pub goal_type: GoalType,
    pub goal_text: String,
    #[serde(default)]
    pub expected_result: Option<String>,
    pub duration_min: u32,
    #[serde(default = "default_classification")]
    pub classification: String,
    #[serde(default)]
    pub agenda: Vec<AgendaItem>,
    #[serde(default)]
    pub roles: Roles,
    #[serde(default)]
    pub materials: Vec<Material>,
    #[serde(default)]
    pub hints: Hints,
    #[serde(default)]
    pub transcription_planned: bool,
    #[serde(default)]
    pub minutes_template_id: Option<String>,
}

pub fn default_classification() -> String {
    "intern".to_string()
}

/// Ueber welchen Weg der Termin mit Outlook verbunden ist (Konzept 3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    /// A: Microsoft Graph.
    Graph,
    /// B: Outlook lokal ueber COM (Windows).
    OutlookCom,
    /// C: Text ueber die Zwischenablage, Zuordnung ueber die Marke im ICS-Feed.
    IcsMarker,
}

impl AdapterKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdapterKind::Graph => "graph",
            AdapterKind::OutlookCom => "outlook_com",
            AdapterKind::IcsMarker => "ics_marker",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "graph" => AdapterKind::Graph,
            "outlook_com" => AdapterKind::OutlookCom,
            "ics_marker" => AdapterKind::IcsMarker,
            _ => return None,
        })
    }
}

/// Zustand der Verknuepfung Planung <-> Termin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Ok,
    Verschoben,
    Abgesagt,
    /// Der App-Block wurde in Outlook geaendert; die App schreibt nicht mehr.
    Konflikt,
    /// Termin geloescht oder Marke entfernt.
    Verwaist,
}

impl SyncState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncState::Ok => "ok",
            SyncState::Verschoben => "verschoben",
            SyncState::Abgesagt => "abgesagt",
            SyncState::Konflikt => "konflikt",
            SyncState::Verwaist => "verwaist",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ok" => SyncState::Ok,
            "verschoben" => SyncState::Verschoben,
            "abgesagt" => SyncState::Abgesagt,
            "konflikt" => SyncState::Konflikt,
            "verwaist" => SyncState::Verwaist,
            _ => return None,
        })
    }
}

/// Die Verknuepfung einer Planung mit einem Kalendertermin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PlanLink {
    pub plan_id: String,
    pub adapter: AdapterKind,
    /// Kalenderquelle der App (`calendar_sources.id`), falls ueber den Feed gefunden.
    pub source_id: Option<String>,
    /// Graph ImmutableId bzw. COM GlobalAppointmentID.
    pub remote_id: Option<String>,
    pub ical_uid: Option<String>,
    pub change_key: Option<String>,
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub location: Option<String>,
    pub join_url: Option<String>,
    /// Pruefsumme des Blocks, den die App zuletzt geschrieben hat.
    pub body_hash: Option<String>,
    pub last_seen_at: Option<i64>,
    pub last_change_at: Option<i64>,
    /// Verdichteter Zustand fuer Liste und Status (Vorrang siehe `sync`).
    pub sync_state: SyncState,
    /// Eine Verschiebung ist noch nicht bestaetigt. Getrennt vom `sync_state`
    /// gefuehrt, damit sie unter Konflikt oder Absage nicht verloren geht.
    #[serde(default)]
    pub verschiebung_offen: bool,
    /// Der App-Block weicht von der zuletzt geschriebenen Fassung ab.
    #[serde(default)]
    pub block_konflikt: bool,
    /// Der Termin ist abgesagt (bleibt auch unter `verwaist` erhalten).
    #[serde(default)]
    pub abgesagt: bool,
    /// Serientermin: urspruenglicher Beginn der Instanz (UTC-ms), sonst `None`.
    #[serde(default)]
    pub instanz: Option<i64>,
    /// Zuletzt wurde ein anderer Termin bzw. eine andere Instanz beobachtet und
    /// verworfen. Merker, damit die Meldung nur einmal entsteht.
    #[serde(default)]
    pub fremd_beobachtet: bool,
}

/// Einwilligung einer Person zur Transkription, je Planung.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ConsentState {
    Offen,
    Zugestimmt,
    Abgelehnt,
}

impl ConsentState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConsentState::Offen => "offen",
            ConsentState::Zugestimmt => "zugestimmt",
            ConsentState::Abgelehnt => "abgelehnt",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "offen" => ConsentState::Offen,
            "zugestimmt" => ConsentState::Zugestimmt,
            "abgelehnt" => ConsentState::Abgelehnt,
            _ => return None,
        })
    }
}

/// Auf welchem Weg eine Einwilligung kam.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ConsentChannel {
    Einladung,
    MuendlichBeiStart,
    Mail,
}

impl ConsentChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConsentChannel::Einladung => "einladung",
            ConsentChannel::MuendlichBeiStart => "muendlich_bei_start",
            ConsentChannel::Mail => "mail",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "einladung" => ConsentChannel::Einladung,
            "muendlich_bei_start" => ConsentChannel::MuendlichBeiStart,
            "mail" => ConsentChannel::Mail,
            _ => return None,
        })
    }
}

/// Einwilligung einer Person. `person` ist die normalisierte E-Mail-Adresse
/// (klein, ohne `mailto:`) oder, ohne Adresse, die Personen-Id der App.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Consent {
    pub plan_id: String,
    pub person: String,
    pub state: ConsentState,
    pub channel: Option<ConsentChannel>,
    pub recorded_at: Option<i64>,
}
