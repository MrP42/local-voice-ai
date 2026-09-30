//! Datentypen der Provenienz (A1, Goal „Integrationen“, AK1/AK6).
//!
//! Zeitstempel sind Millisekunden UTC. `ProvenanceEntry` ist die Form, die die
//! Oberflaeche (Kontextmenue „Herkunft“, Paket A3) bekommt; `NewProvenance` ist,
//! was eine Erzeugungsstelle mitbringt.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Art des erzeugten Inhalts (Spalte `provenance.subject_kind`). Die Liste ist
/// fest und steht zusaetzlich als CHECK in der Migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    /// Transkript einer Besprechung; `subject_id` ist die Besprechungs-ID (je
    /// Besprechung gibt es genau ein aktives Transkript).
    Transcript,
    /// Fassung eines Transkripts (Untertitel, STT, Zusammenfuehrung; A3).
    TranscriptVariant,
    /// Dokument einer Besprechung: Protokoll, KI-Notizen (`meeting_documents.id`).
    Document,
    /// Zusammenfassung (Video, Buch, A3).
    Summary,
    /// Notiz im Wissensspeicher / Vault (A6).
    KnowledgeNote,
    /// Erzeugtes Audio (Vorlesen).
    TtsAudio,
    /// Ausgabe nach aussen: Follow-up-Entwurf, Export, Mail.
    Export,
    /// Ergebnis eines Workflow-Laufs (Goal B).
    RunOutput,
}

impl SubjectKind {
    pub const ALL: [SubjectKind; 8] = [
        SubjectKind::Transcript,
        SubjectKind::TranscriptVariant,
        SubjectKind::Document,
        SubjectKind::Summary,
        SubjectKind::KnowledgeNote,
        SubjectKind::TtsAudio,
        SubjectKind::Export,
        SubjectKind::RunOutput,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SubjectKind::Transcript => "transcript",
            SubjectKind::TranscriptVariant => "transcript_variant",
            SubjectKind::Document => "document",
            SubjectKind::Summary => "summary",
            SubjectKind::KnowledgeNote => "knowledge_note",
            SubjectKind::TtsAudio => "tts_audio",
            SubjectKind::Export => "export",
            SubjectKind::RunOutput => "run_output",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// Wer den Inhalt ausgeloest hat (Spalte `provenance.actor_kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    /// Der Nutzer per Knopfdruck.
    User,
    /// Die App von selbst (Enddurchlauf, Automatik nach der Aufnahme).
    Auto,
    Workflow,
    AgentExternal,
    AgentLocal,
}

impl ActorKind {
    pub const ALL: [ActorKind; 5] = [
        ActorKind::User,
        ActorKind::Auto,
        ActorKind::Workflow,
        ActorKind::AgentExternal,
        ActorKind::AgentLocal,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ActorKind::User => "user",
            ActorKind::Auto => "auto",
            ActorKind::Workflow => "workflow",
            ActorKind::AgentExternal => "agent_external",
            ActorKind::AgentLocal => "agent_local",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// Lief das Modell auf diesem Rechner oder bei einem entfernten Anbieter?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Locality {
    Local,
    Remote,
}

impl Locality {
    pub fn as_str(self) -> &'static str {
        match self {
            Locality::Local => "local",
            Locality::Remote => "remote",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "local" => Some(Locality::Local),
            "remote" => Some(Locality::Remote),
            _ => None,
        }
    }
}

/// Woher ein Eintrag stammt: `Recorded` wurde bei der Erzeugung geschrieben,
/// `Derived` ist aus aelteren Daten (`generation_metadata_json`, Kopfzeile des
/// Transkripts) rekonstruiert und traegt nur, was dort stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceOrigin {
    Recorded,
    Derived,
}

/// Eine Quelle des Inhalts. `kind`: `transcript`, `meeting`, `notes`, `audio`,
/// `subtitle`, `youtube`, `rag`, `vault`, `web` (frei erweiterbar, nur
/// Kleinbuchstaben, Ziffern und `_`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SourceRef {
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: Option<String>,
    pub url: Option<String>,
}

impl SourceRef {
    pub fn new(kind: &str, reference: &str, title: Option<&str>) -> Self {
        Self {
            kind: kind.to_string(),
            reference: reference.to_string(),
            title: title.map(str::to_string),
            url: None,
        }
    }
}

/// Was eine Erzeugungsstelle zum Schreiben mitbringt.
#[derive(Clone, Debug)]
pub struct NewProvenance {
    pub subject_kind: SubjectKind,
    pub subject_id: String,
    /// Revision/Version des Inhalts, falls er sich aendern kann.
    pub subject_revision: Option<i64>,
    /// `stt`, `import`, `subtitles_import`, `merge`, `summary`, `minutes`,
    /// `notes`, `followup`, ... (frei, Kleinbuchstaben/Ziffern/`_`).
    pub operation: String,
    pub actor_kind: ActorKind,
    /// Workflow-, Lauf- oder Client-Kennung.
    pub actor_ref: Option<String>,
    /// Anbieter-Kennung (`openai`, `local`, ...) bzw. bei STT die Engine.
    pub provider: Option<String>,
    pub locality: Option<Locality>,
    /// Modellkennung, wie der Anbieter sie kennt (`gpt-4.1`, `llm-gemma4-e4b-q4`).
    pub model_id: Option<String>,
    pub model_label: Option<String>,
    /// Verweis in `usage.db` (Kosten und Preise bleiben dort).
    pub usage_event_id: Option<i64>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub duration_ms: Option<u64>,
    pub sources: Vec<SourceRef>,
    /// 0..=1; nur wo der Erzeuger eine Sicherheit angibt.
    pub confidence: Option<f64>,
    /// Weitere Angaben als JSON-Objekt (Vorlage, Bloecke, alle Ereignisnummern).
    pub params: Option<serde_json::Value>,
}

impl NewProvenance {
    /// Nur die Pflichtfelder; der Rest ist leer.
    pub fn new(
        subject_kind: SubjectKind,
        subject_id: &str,
        operation: &str,
        actor_kind: ActorKind,
    ) -> Self {
        Self {
            subject_kind,
            subject_id: subject_id.to_string(),
            subject_revision: None,
            operation: operation.to_string(),
            actor_kind,
            actor_ref: None,
            provider: None,
            locality: None,
            model_id: None,
            model_label: None,
            usage_event_id: None,
            prompt_tokens: None,
            completion_tokens: None,
            duration_ms: None,
            sources: Vec::new(),
            confidence: None,
            params: None,
        }
    }
}

/// Ein Eintrag, wie die Oberflaeche ihn zeigt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct ProvenanceEntry {
    pub id: String,
    pub subject_kind: SubjectKind,
    pub subject_id: String,
    pub subject_revision: Option<i64>,
    /// Millisekunden UTC.
    pub created_at: i64,
    pub operation: String,
    /// `None` bei rekonstruierten Eintraegen: aus alten Daten ist der
    /// Ausloeser nicht ablesbar.
    pub actor_kind: Option<ActorKind>,
    pub actor_ref: Option<String>,
    pub provider: Option<String>,
    pub locality: Option<Locality>,
    pub model_id: Option<String>,
    pub model_label: Option<String>,
    pub usage_event_id: Option<i64>,
    /// Token ein (Prompt). `None`, wenn der Anbieter keine meldet.
    pub prompt_tokens: Option<u64>,
    /// Token aus (Antwort).
    pub completion_tokens: Option<u64>,
    pub duration_ms: Option<u64>,
    pub sources: Vec<SourceRef>,
    pub confidence: Option<f64>,
    /// Weitere Angaben als JSON-Text (Objekt) oder `None`.
    pub params_json: Option<String>,
    pub origin: ProvenanceOrigin,
}

/// Fehler beim Schreiben eines Eintrags: Eingaben, die nie gueltig sind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvenanceError {
    Invalid(String),
    Store(String),
}

impl std::fmt::Display for ProvenanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProvenanceError::Invalid(m) => write!(f, "Ungültige Provenienz: {m}"),
            ProvenanceError::Store(m) => write!(f, "Provenienz nicht gespeichert: {m}"),
        }
    }
}

impl std::error::Error for ProvenanceError {}

impl From<rusqlite::Error> for ProvenanceError {
    fn from(e: rusqlite::Error) -> Self {
        ProvenanceError::Store(e.to_string())
    }
}
