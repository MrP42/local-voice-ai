//! Datenmodell; Feldnamen und Wertebereiche folgen `contracts/core/project*.schema.json`.

use serde::{Deserialize, Serialize};
use wai_security::DataClass;

pub const SCHEMA_VERSION: &str = "0.1.5";

/// Datenklasse in der Schreibweise des Vertrags (aufsteigend nach Schutzbedarf).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Klasse {
    Public,
    Internal,
    Personal,
    Special,
    Restricted,
}

impl Klasse {
    /// Rangzahl (aufsteigend nach Schutzbedarf), z. B. zum Speichern in einer Datenbankspalte.
    pub fn zahl(self) -> i64 {
        self as i64
    }
    /// Umkehrung von [`Klasse::zahl`]; Unbekanntes gilt als hoechste Klasse (fail-closed).
    pub fn aus_zahl(n: i64) -> Klasse {
        match n {
            0 => Klasse::Public,
            1 => Klasse::Internal,
            2 => Klasse::Personal,
            3 => Klasse::Special,
            _ => Klasse::Restricted, // unbekannt = hoechste Klasse (fail-closed)
        }
    }
}

impl From<DataClass> for Klasse {
    fn from(c: DataClass) -> Self {
        match c {
            DataClass::Public => Klasse::Public,
            DataClass::Internal => Klasse::Internal,
            DataClass::Personal => Klasse::Personal,
            DataClass::Special => Klasse::Special,
            DataClass::Restricted => Klasse::Restricted,
        }
    }
}

impl From<Klasse> for DataClass {
    fn from(c: Klasse) -> Self {
        match c {
            Klasse::Public => DataClass::Public,
            Klasse::Internal => DataClass::Internal,
            Klasse::Personal => DataClass::Personal,
            Klasse::Special => DataClass::Special,
            Klasse::Restricted => DataClass::Restricted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Souveraenitaet {
    Local,
    EuCloud,
}

impl Souveraenitaet {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Souveraenitaet::Local => "local",
            Souveraenitaet::EuCloud => "eu_cloud",
        }
    }
    pub(crate) fn aus_text(s: &str) -> Souveraenitaet {
        if s == "eu_cloud" {
            Souveraenitaet::EuCloud
        } else {
            Souveraenitaet::Local
        }
    }
}

/// Eintragsarten (`kind` im Vertrag).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Art {
    Meeting,
    Transcript,
    Conversation,
    Book,
    BookDocument,
    Note,
    Research,
    Document,
    Plan,
    Talk,
}

impl Art {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Art::Meeting => "meeting",
            Art::Transcript => "transcript",
            Art::Conversation => "conversation",
            Art::Book => "book",
            Art::BookDocument => "book_document",
            Art::Note => "note",
            Art::Research => "research",
            Art::Document => "document",
            Art::Plan => "plan",
            Art::Talk => "talk",
        }
    }
    pub(crate) fn aus_text(s: &str) -> Option<Art> {
        Some(match s {
            "meeting" => Art::Meeting,
            "transcript" => Art::Transcript,
            "conversation" => Art::Conversation,
            "book" => Art::Book,
            "book_document" => Art::BookDocument,
            "note" => Art::Note,
            "research" => Art::Research,
            "document" => Art::Document,
            "plan" => Art::Plan,
            "talk" => Art::Talk,
            _ => return None,
        })
    }
    /// Herkunftsapp, die der Verweis tragen muss; `None` = jede App mit gueltigem Link.
    pub fn erwartete_app(self) -> Option<&'static str> {
        match self {
            Art::Talk => Some("presenter"),
            Art::Meeting | Art::Book => Some("lva"),
            Art::Research => Some("corporate-llm"),
            Art::Plan => Some("planner"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZugeordnetDurch {
    Person,
    Automation,
}

impl ZugeordnetDurch {
    pub(crate) fn text(self) -> &'static str {
        match self {
            ZugeordnetDurch::Person => "person",
            ZugeordnetDurch::Automation => "automation",
        }
    }
    pub(crate) fn aus_text(s: &str) -> Self {
        if s == "automation" {
            Self::Automation
        } else {
            Self::Person
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KopieZiel {
    Export,
    Presenter,
    ExchangeFolder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExterneKopie {
    pub target: KopieZiel,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
}

mod revision_text {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// `project.schema.json` ohne `server_context` (Mandant bleibt im Verzeichnis).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projekt {
    pub schema_version: String,
    pub id: String,
    pub ts: String,
    pub trace_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub goal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub instructions: Option<String>,
    pub data_class: Klasse,
    pub special_category: bool,
    pub sovereignty: Souveraenitaet,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub retention_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub web_research_enabled: Option<bool>,
    pub archived: bool,
    #[serde(with = "revision_text")]
    pub revision: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub plan_ref: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// `project-entry.schema.json` ohne `server_context`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projekteintrag {
    pub schema_version: String,
    pub id: String,
    pub ts: String,
    pub trace_id: String,
    pub project_id: String,
    pub kind: Art,
    pub title: String,
    pub origin_app: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub data_class: Klasse,
    pub assigned_by: ZugeordnetDurch,
    pub assigned_at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub preview: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub external_copies: Vec<ExterneKopie>,
}

/// Auflistung: lesbarer Eintrag oder Platzhalter ohne Titel und Vorschau.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Eintragsansicht {
    Sichtbar(Projekteintrag),
    NichtVerfuegbar {
        id: String,
        project_id: String,
        kind: Art,
    },
}

#[derive(Debug, Clone)]
pub struct NeuesProjekt {
    pub title: String,
    pub goal: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub data_class: Klasse,
    pub special_category: bool,
    pub sovereignty: Souveraenitaet,
    pub retention_days: Option<i64>,
    pub web_research_enabled: Option<bool>,
    pub plan_ref: Option<String>,
}

impl NeuesProjekt {
    pub fn neu(title: &str, data_class: Klasse) -> Self {
        NeuesProjekt {
            title: title.into(),
            goal: None,
            description: None,
            instructions: None,
            data_class,
            special_category: false,
            sovereignty: Souveraenitaet::Local,
            retention_days: None,
            web_research_enabled: None,
            plan_ref: None,
        }
    }
}

/// Aenderung mit `Option<Option<_>>`: aeussere `None` = unveraendert, innere `None` = leeren.
/// Plan und Archivstatus haben eigene Operationen.
#[derive(Debug, Clone, Default)]
pub struct Aenderung {
    pub title: Option<String>,
    pub goal: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub instructions: Option<Option<String>>,
    pub data_class: Option<Klasse>,
    pub special_category: Option<bool>,
    pub sovereignty: Option<Souveraenitaet>,
    pub retention_days: Option<Option<i64>>,
    pub web_research_enabled: Option<Option<bool>>,
}

#[derive(Debug, Clone)]
pub struct NeuerEintrag {
    pub kind: Art,
    pub title: String,
    pub origin_app: String,
    pub ref_: String,
    pub data_class: Klasse,
    pub assigned_by: ZugeordnetDurch,
    pub preview: Option<String>,
    pub external_copies: Vec<ExterneKopie>,
}

/// Ereignis nur mit Kennungen, nie mit Titeln oder Texten (ADR-0026 Abschnitt 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ereignis {
    pub name: &'static str,
    pub projekt_id: String,
    pub eintrag_id: Option<String>,
    pub art: Option<Art>,
}

/// Dauerhafte Loeschmeldung fuer Index und Herkunftsapps (E3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loeschmeldung {
    pub id: i64,
    pub projekt_id: String,
    /// `None` = das ganze Projekt wurde geloescht.
    pub eintrag_id: Option<String>,
    pub ref_: Option<String>,
    pub grund: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AbgleichBericht {
    pub geprueft: usize,
    pub waisen_entfernt: Vec<String>,
    /// Eintraege, deren Ursprung nicht beurteilt werden konnte (nicht geloescht).
    pub unbekannt: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoeschBericht {
    pub eintraege_entfernt: usize,
    /// (Eintragskennung, Kopie): Kopien ausserhalb des Projekts, die die Person bereinigen muss.
    pub externe_kopien: Vec<(String, ExterneKopie)>,
}
