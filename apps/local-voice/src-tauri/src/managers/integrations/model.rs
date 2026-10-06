//! Datentypen des Integrations-Registers (A1, Goal „Integrationen“).
//!
//! Zeitstempel sind Millisekunden UTC. Die Konfiguration einer Integration
//! (`config_json`) enthaelt NIE ein Geheimnis: Passwoerter, Tokens und Adressen
//! mit Zugriffsschluessel liegen im Geheimnisspeicher (`secrets`).

use serde::{Deserialize, Serialize};
use specta::Type;

/// Art einer Integration. `Ics` und `Graph` sind die Kalenderquellen, die das
/// Register aus `calendar_sources` uebernimmt (gleiche ID).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Youtube,
    Ics,
    Graph,
    M365,
    Smtp,
    Folder,
    Obsidian,
    Wissen,
    Agent,
    /// Webhook als Ziel (B5): Adresse eines n8n-Ablaufs o. ae., die Adresse liegt im
    /// Geheimnisspeicher.
    Webhook,
    /// Ein Dienst aus dem Register `services::registry` (Slack, Notion, Jira, ...): welcher,
    /// steht in `config_json.service`; Schluessel oder Webhook-Adresse im Fach `token`.
    Service,
}

impl Kind {
    pub const ALL: [Kind; 11] = [
        Kind::Youtube,
        Kind::Ics,
        Kind::Graph,
        Kind::M365,
        Kind::Smtp,
        Kind::Folder,
        Kind::Obsidian,
        Kind::Wissen,
        Kind::Agent,
        Kind::Webhook,
        Kind::Service,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Youtube => "youtube",
            Kind::Ics => "ics",
            Kind::Graph => "graph",
            Kind::M365 => "m365",
            Kind::Smtp => "smtp",
            Kind::Folder => "folder",
            Kind::Obsidian => "obsidian",
            Kind::Wissen => "wissen",
            Kind::Agent => "agent",
            Kind::Webhook => "webhook",
            Kind::Service => "service",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Wird diese Art aus `calendar_sources` gespiegelt? Dann bleibt der
    /// Kalender fuehrend (Name, Schalter, Entfernen); das Register haelt nur
    /// Richtung und Rechte.
    pub fn is_calendar_managed(self) -> bool {
        matches!(self, Kind::Ics | Kind::Graph)
    }

    /// Welche Richtungen diese Art kennt: eine ICS-Adresse nur lesend, ein
    /// SMTP-Postfach nur schreibend.
    pub fn allowed_directions(self) -> &'static [Direction] {
        match self {
            Kind::Youtube | Kind::Ics | Kind::Wissen => &[Direction::Read],
            Kind::Smtp | Kind::Webhook | Kind::Service => &[Direction::Write],
            Kind::Graph | Kind::M365 | Kind::Folder | Kind::Obsidian | Kind::Agent => {
                &[Direction::Read, Direction::Write, Direction::Both]
            }
        }
    }

    /// Vorgabe fuer die Richtung beim Anlegen.
    pub fn default_direction(self) -> Direction {
        match self {
            Kind::Youtube | Kind::Ics | Kind::Wissen | Kind::Graph => Direction::Read,
            Kind::Smtp | Kind::Webhook | Kind::Service => Direction::Write,
            Kind::M365 | Kind::Folder | Kind::Obsidian | Kind::Agent => Direction::Both,
        }
    }

    /// Faehigkeiten, die diese Art ueberhaupt anbietet. Bei `Service` die aller Dienste; was
    /// eine einzelne Integration kann, sagt `Integration::capabilities`.
    pub fn capabilities(self) -> &'static [Capability] {
        use Capability::*;
        match self {
            Kind::Youtube => &[MediaFetch, YoutubeAdd],
            Kind::Ics => &[CalendarRead],
            Kind::Graph => &[CalendarRead, CalendarWrite],
            Kind::M365 => &[
                CalendarRead,
                CalendarWrite,
                MailSend,
                MailDraft,
                FilesRead,
                FilesWrite,
            ],
            Kind::Smtp => &[MailSend],
            Kind::Folder => &[FilesRead, FilesWrite],
            Kind::Obsidian => &[VaultWrite, FilesRead],
            Kind::Wissen => &[KnowledgeSearch, KnowledgeRead],
            Kind::Agent => &[
                MeetingCreate,
                RecordingStart,
                TranscribeFile,
                TtsRender,
                YoutubeAdd,
                WorkflowRead,
                WorkflowRun,
            ],
            Kind::Webhook => &[WebhookPost],
            Kind::Service => &[
                ChatPost,
                TaskCreate,
                PageWrite,
                CrmWrite,
                RecordWrite,
                CalendarWrite,
            ],
        }
    }
}

/// Fluss der Daten: was die Integration lesen darf, was sie schreiben darf.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Read,
    Write,
    Both,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Read => "read",
            Direction::Write => "write",
            Direction::Both => "both",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Direction::Read),
            "write" => Some(Direction::Write),
            "both" => Some(Direction::Both),
            _ => None,
        }
    }

    /// Erlaubt diese Richtung eine Faehigkeit der Zugriffsart `access`?
    pub fn permits(self, access: Access) -> bool {
        matches!(
            (self, access),
            (Direction::Both, _)
                | (Direction::Read, Access::Read)
                | (Direction::Write, Access::Write)
        )
    }
}

/// Lesend oder schreibend (veraendernd)?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    Read,
    Write,
}

/// Eine Faehigkeit, fuer die es je Integration und Aufrufer ein Recht gibt.
/// Die Schreibweise mit Punkt ist die der Oberflaeche und des Audit-Logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum Capability {
    #[serde(rename = "calendar.read")]
    CalendarRead,
    #[serde(rename = "calendar.write")]
    CalendarWrite,
    #[serde(rename = "mail.send")]
    MailSend,
    #[serde(rename = "files.read")]
    FilesRead,
    #[serde(rename = "files.write")]
    FilesWrite,
    #[serde(rename = "knowledge.search")]
    KnowledgeSearch,
    #[serde(rename = "knowledge.read")]
    KnowledgeRead,
    #[serde(rename = "vault.write")]
    VaultWrite,
    /// YouTube-Datei holen (externes Werkzeug, Schalter „privat“, E1).
    #[serde(rename = "media.fetch")]
    MediaFetch,
    #[serde(rename = "youtube.add")]
    YoutubeAdd,
    #[serde(rename = "meeting.create")]
    MeetingCreate,
    /// Aufnahme starten: nie `allow` (Einwilligungsdialog, § 201 StGB).
    #[serde(rename = "recording.start")]
    RecordingStart,
    #[serde(rename = "transcribe.file")]
    TranscribeFile,
    #[serde(rename = "tts.render")]
    TtsRender,
    /// Daten an einen Webhook senden (B5, n8n-Bruecke).
    #[serde(rename = "webhook.post")]
    WebhookPost,
    /// Ablaeufe auflisten und Laufprotokolle lesen (B8, Agentenbruecke).
    #[serde(rename = "workflow.read")]
    WorkflowRead,
    /// Einen Ablauf starten (B8): Trockenlauf oder, bei scharfem Ablauf, ein echter Lauf.
    #[serde(rename = "workflow.run")]
    WorkflowRun,
    /// In einen Kanal posten (Slack, Teams, Discord).
    #[serde(rename = "chat.post")]
    ChatPost,
    /// Aufgabe/Ticket anlegen (Asana, Jira, Todoist, ...).
    #[serde(rename = "task.create")]
    TaskCreate,
    /// Seite anlegen oder ergaenzen (Notion, Confluence).
    #[serde(rename = "page.write")]
    PageWrite,
    /// Notiz im CRM (HubSpot, Pipedrive).
    #[serde(rename = "crm.write")]
    CrmWrite,
    /// Datensatz anhaengen (Airtable).
    #[serde(rename = "record.write")]
    RecordWrite,
    /// Mail als Entwurf im Postfach ablegen (Outlook, Scope `Mail.ReadWrite`); geht an niemanden.
    #[serde(rename = "mail.draft")]
    MailDraft,
}

impl Capability {
    pub const ALL: [Capability; 23] = [
        Capability::CalendarRead,
        Capability::CalendarWrite,
        Capability::MailSend,
        Capability::FilesRead,
        Capability::FilesWrite,
        Capability::KnowledgeSearch,
        Capability::KnowledgeRead,
        Capability::VaultWrite,
        Capability::MediaFetch,
        Capability::YoutubeAdd,
        Capability::MeetingCreate,
        Capability::RecordingStart,
        Capability::TranscribeFile,
        Capability::TtsRender,
        Capability::WebhookPost,
        Capability::WorkflowRead,
        Capability::WorkflowRun,
        Capability::ChatPost,
        Capability::TaskCreate,
        Capability::PageWrite,
        Capability::CrmWrite,
        Capability::RecordWrite,
        Capability::MailDraft,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Capability::CalendarRead => "calendar.read",
            Capability::CalendarWrite => "calendar.write",
            Capability::MailSend => "mail.send",
            Capability::FilesRead => "files.read",
            Capability::FilesWrite => "files.write",
            Capability::KnowledgeSearch => "knowledge.search",
            Capability::KnowledgeRead => "knowledge.read",
            Capability::VaultWrite => "vault.write",
            Capability::MediaFetch => "media.fetch",
            Capability::YoutubeAdd => "youtube.add",
            Capability::MeetingCreate => "meeting.create",
            Capability::RecordingStart => "recording.start",
            Capability::TranscribeFile => "transcribe.file",
            Capability::TtsRender => "tts.render",
            Capability::WebhookPost => "webhook.post",
            Capability::WorkflowRead => "workflow.read",
            Capability::WorkflowRun => "workflow.run",
            Capability::ChatPost => "chat.post",
            Capability::TaskCreate => "task.create",
            Capability::PageWrite => "page.write",
            Capability::CrmWrite => "crm.write",
            Capability::RecordWrite => "record.write",
            Capability::MailDraft => "mail.draft",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }

    /// Liest die Faehigkeit nur, oder veraendert sie etwas (Mail senden, Datei
    /// schreiben, Aufnahme starten)? Schreibendes bekommt strengere Vorgaben.
    pub fn access(self) -> Access {
        match self {
            Capability::CalendarRead
            | Capability::FilesRead
            | Capability::KnowledgeSearch
            | Capability::KnowledgeRead
            | Capability::MediaFetch
            | Capability::WorkflowRead => Access::Read,
            _ => Access::Write,
        }
    }

    /// Nie dauerhaft erlaubt: hoechstens „fragen“. Die App zeigt vor jeder
    /// Aufnahme den Einwilligungsdialog, egal was hier steht (§ 201 StGB).
    pub fn never_allow(self) -> bool {
        matches!(self, Capability::RecordingStart)
    }
}

/// Wer eine Faehigkeit benutzt. `User` ist der Nutzer in der Oberflaeche; er
/// braucht keine Freigabe und hat keine Zeile in `integration_grants`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Caller {
    User,
    Workflow,
    AgentExternal,
    AgentLocal,
}

impl Caller {
    /// Aufrufer, fuer die Rechte gespeichert werden.
    pub const GRANTABLE: [Caller; 3] =
        [Caller::Workflow, Caller::AgentExternal, Caller::AgentLocal];

    pub fn as_str(self) -> &'static str {
        match self {
            Caller::User => "user",
            Caller::Workflow => "workflow",
            Caller::AgentExternal => "agent_external",
            Caller::AgentLocal => "agent_local",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Caller::User,
            Caller::Workflow,
            Caller::AgentExternal,
            Caller::AgentLocal,
        ]
        .into_iter()
        .find(|c| c.as_str() == s)
    }
}

/// Recht: aus, nachfragen oder erlaubt. Die Ordnung `Off < Ask < Allow` macht
/// „das Strengste gewinnt“ zu `min`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum GrantMode {
    Off,
    Ask,
    Allow,
}

impl GrantMode {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantMode::Off => "off",
            GrantMode::Ask => "ask",
            GrantMode::Allow => "allow",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(GrantMode::Off),
            "ask" => Some(GrantMode::Ask),
            "allow" => Some(GrantMode::Allow),
            _ => None,
        }
    }
}

/// Eine Integration, wie Oberflaeche und Dump sie sehen. Enthaelt kein Geheimnis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Integration {
    pub id: String,
    pub kind: Kind,
    pub label: String,
    pub enabled: bool,
    pub direction: Direction,
    /// Art-spezifische Einstellungen als JSON-Objekt, ohne Geheimnisse.
    pub config_json: String,
    /// Konto/Host zur Anzeige (z. B. `outlook.office365.com`), nie die Adresse.
    pub account_hint: Option<String>,
    pub data_class: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_ok_at: Option<i64>,
    pub last_error: Option<String>,
}

impl Integration {
    /// Faehigkeiten DIESER Integration: bei einem Dienst die aus dem Register (Slack kann nur
    /// posten, Jira nur Aufgaben), sonst die der Art. Ein Dienst mit unbekannter oder
    /// fehlender Angabe kann nichts.
    pub fn capabilities(&self) -> &'static [Capability] {
        match self.kind {
            Kind::Service => self
                .service()
                .map(|s| super::services::registry::def(s).capabilities)
                .unwrap_or(&[]),
            k => k.capabilities(),
        }
    }

    /// Der Dienst einer `Service`-Integration (aus `config_json.service`).
    pub fn service(&self) -> Option<super::services::registry::ServiceId> {
        if self.kind != Kind::Service {
            return None;
        }
        let cfg: serde_json::Value = serde_json::from_str(&self.config_json).ok()?;
        super::services::registry::ServiceId::parse(cfg.get("service")?.as_str()?)
    }
}

/// Was zum Anlegen noetig ist.
#[derive(Clone, Debug)]
pub struct NewIntegration {
    /// `None`: eine neue ULID.
    pub id: Option<String>,
    pub kind: Kind,
    pub label: String,
    /// `None`: Vorgabe der Art.
    pub direction: Option<Direction>,
    pub config: serde_json::Value,
    pub account_hint: Option<String>,
    pub data_class: Option<String>,
}

impl NewIntegration {
    pub fn new(kind: Kind, label: &str) -> Self {
        Self {
            id: None,
            kind,
            label: label.to_string(),
            direction: None,
            config: serde_json::json!({}),
            account_hint: None,
            data_class: None,
        }
    }
}

/// Aenderungen an einer Integration (`None` = unveraendert).
#[derive(Clone, Debug, Default)]
pub struct IntegrationPatch {
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub direction: Option<Direction>,
    pub config: Option<serde_json::Value>,
    pub data_class: Option<Option<String>>,
}

/// Ein gespeichertes Recht.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct GrantRow {
    pub integration_id: String,
    pub capability: Capability,
    pub caller: Caller,
    pub mode: GrantMode,
}

/// Ein Eintrag des Audit-Logs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct AuditEntry {
    pub id: i64,
    pub ts: i64,
    pub caller: String,
    pub integration_id: Option<String>,
    pub capability: Option<String>,
    pub target: Option<String>,
    /// `ok`, `denied`, `error` oder `pending`.
    pub outcome: String,
    pub detail_json: Option<String>,
}

/// Was zum Schreiben eines Audit-Eintrags noetig ist.
#[derive(Clone, Debug)]
pub struct NewAudit {
    pub caller: String,
    pub integration_id: Option<String>,
    pub capability: Option<String>,
    pub target: Option<String>,
    pub outcome: AuditOutcome,
    pub detail: Option<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditOutcome {
    Ok,
    Denied,
    Error,
    Pending,
}

impl AuditOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditOutcome::Ok => "ok",
            AuditOutcome::Denied => "denied",
            AuditOutcome::Error => "error",
            AuditOutcome::Pending => "pending",
        }
    }
}

/// Zustand einer Freigabe-Anfrage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Approved,
    Denied,
    Expired,
    /// Eine genehmigte Anfrage wurde eingeloest (einmalig).
    Used,
}

impl ApprovalState {
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalState::Pending => "pending",
            ApprovalState::Approved => "approved",
            ApprovalState::Denied => "denied",
            ApprovalState::Expired => "expired",
            ApprovalState::Used => "used",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(ApprovalState::Pending),
            "approved" => Some(ApprovalState::Approved),
            "denied" => Some(ApprovalState::Denied),
            "expired" => Some(ApprovalState::Expired),
            "used" => Some(ApprovalState::Used),
            _ => None,
        }
    }
}

/// Eine Freigabe-Anfrage: ein Aufrufer will etwas tun, das „fragen“ verlangt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Approval {
    pub id: String,
    pub created_at: i64,
    pub caller: String,
    pub integration_id: Option<String>,
    /// Faehigkeit oder Werkzeugname.
    pub tool_or_capability: String,
    /// Fuer den Menschen: Ziel und Kurzfassung der Argumente, ohne Geheimnisse.
    pub args_preview: Option<String>,
    pub state: ApprovalState,
    pub decided_at: Option<i64>,
}

/// Fehler des Registers. `Display` ist Klartext fuer die Oberflaeche.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntegrationError {
    /// Eingabe nie gueltig (Text in Deutsch).
    Invalid(String),
    NotFound(String),
    /// Wird vom Kalender gefuehrt (Name, Schalter, Entfernen dort aendern).
    Managed(String),
    /// Datenbank (gesperrt, voll, defekt).
    Store(String),
}

impl std::fmt::Display for IntegrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntegrationError::Invalid(m) => write!(f, "{m}"),
            IntegrationError::NotFound(id) => write!(f, "Integration nicht gefunden: {id}"),
            IntegrationError::Managed(m) => write!(f, "{m}"),
            IntegrationError::Store(m) => write!(f, "Speicherfehler: {m}"),
        }
    }
}

impl std::error::Error for IntegrationError {}

impl From<rusqlite::Error> for IntegrationError {
    fn from(e: rusqlite::Error) -> Self {
        IntegrationError::Store(e.to_string())
    }
}
