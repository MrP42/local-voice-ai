//! Werkzeug-Katalog: die EINE Tabelle, die jedem Werkzeug der Agentenbruecke eine
//! Faehigkeit (und damit ein Recht, A1) zuordnet, dazu `ToolHandler` und die Registry.
//!
//! Sicherheitsidee: Ein Werkzeug ohne Eintrag im Katalog gibt es nicht. Wer ein neues
//! Werkzeug anbietet (A8), traegt es hier mit seiner Faehigkeit ein und liefert in einem
//! `ToolHandler` Beschreibung und Ausfuehrung. Die Registry nimmt nur Werkzeuge an, deren
//! Name im Katalog steht; die Faehigkeit kommt IMMER aus dem Katalog, nie aus dem Handler.
//! So kann ein Handler sich nicht an den Rechten vorbeimogeln, und die Rechte-Matrix der
//! Oberflaeche (`Kind::Agent.capabilities()`) bleibt vollstaendig.
//!
//! `get_action_status` ist ein eingebautes Werkzeug ohne Recht: es verraet einem Zugang nur den
//! Stand SEINER Freigaben (`bridge::Bridge::action_status`).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::managers::integrations::model::{Access, Capability, Kind};

/// Eingebautes Werkzeug ohne Recht: Stand einer Freigabe.
pub const STATUS_TOOL: &str = "get_action_status";

/// Ein Eintrag des Katalogs.
#[derive(Clone, Copy, Debug)]
pub struct CatalogEntry {
    pub name: &'static str,
    /// Das Recht, das dieses Werkzeug braucht.
    pub capability: Capability,
    /// Kurzname fuer die Oberflaeche.
    pub title: &'static str,
    pub description: &'static str,
}

/// Alle Werkzeuge, die ein Zugang freischalten kann (schreibende Werkzeuge der Agentenbruecke).
/// `stop_recording` teilt sich das Recht mit `start_recording` („Aufnahme steuern“): ein Agent,
/// der Aufnahmen beenden darf, kann Besprechungen stoeren, auch wenn er keine starten darf.
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "add_youtube_source",
        capability: Capability::YoutubeAdd,
        title: "YouTube-Link als Quelle anlegen",
        description: "Legt zu einem YouTube-Link eine Besprechung mit der Quelle YouTube an.",
    },
    CatalogEntry {
        name: "start_recording",
        capability: Capability::RecordingStart,
        title: "Aufnahme starten",
        description: "Fragt die App nach einer Aufnahme; die App zeigt immer den Einwilligungsdialog.",
    },
    CatalogEntry {
        name: "stop_recording",
        capability: Capability::RecordingStart,
        title: "Aufnahme beenden",
        description: "Beendet die laufende Aufnahme (gleiches Recht wie Aufnahme starten).",
    },
    CatalogEntry {
        name: "transcribe_file",
        capability: Capability::TranscribeFile,
        title: "Datei transkribieren",
        description: "Transkribiert eine Audio- oder Videodatei und legt eine Besprechung an.",
    },
    CatalogEntry {
        name: "create_session",
        capability: Capability::MeetingCreate,
        title: "Session anlegen",
        description: "Legt eine Session (Ordner fuer Aufnahmen) an.",
    },
    CatalogEntry {
        name: "create_meeting",
        capability: Capability::MeetingCreate,
        title: "Besprechung anlegen",
        description: "Legt eine leere Besprechung an.",
    },
    CatalogEntry {
        name: "tts_page_create",
        capability: Capability::TtsRender,
        title: "Vorlesen-Seite anlegen",
        description: "Legt eine Seite in der Vorlesen-Bibliothek an.",
    },
    CatalogEntry {
        name: "tts_render_audio",
        capability: Capability::TtsRender,
        title: "Audio erzeugen",
        description: "Erzeugt aus einer Vorlesen-Seite eine Audiodatei.",
    },
    CatalogEntry {
        name: "list_workflows",
        capability: Capability::WorkflowRead,
        title: "Automationen auflisten",
        description: "Listet die Abläufe der Automationen mit Auslöser, Schaltzustand und letztem Lauf.",
    },
    CatalogEntry {
        name: "get_run",
        capability: Capability::WorkflowRead,
        title: "Laufprotokoll lesen",
        description: "Liest das Protokoll eines Laufs: Zustand, Schritte, Fehler (ohne Geheimnisse und ohne Auslöserdaten).",
    },
    CatalogEntry {
        name: "run_workflow",
        capability: Capability::WorkflowRun,
        title: "Ablauf starten",
        description: "Startet einen Ablauf als Trockenlauf; scharf nur, wenn der Ablauf scharf geschaltet ist und das Recht es erlaubt.",
    },
];

pub fn find(name: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.name == name)
}

/// Gueltiger Werkzeugname: kleine Buchstaben, Ziffern und `_`, 1 bis 64 Zeichen.
pub fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Beschreibung eines Werkzeugs fuer `tools/list` (MCP-nah: Name, Titel, Beschreibung, Schema).
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
}

/// Wer ruft: Zugang und Zustand der Freigabe.
#[derive(Clone, Debug)]
pub struct CallContext {
    pub client_id: String,
    pub client_label: String,
    /// `true`: der Nutzer hat diese Ausfuehrung freigegeben (eine Genehmigung wurde eingeloest).
    pub approved: bool,
    pub now_ms: i64,
}

/// Fuehrt Werkzeuge aus (A8). Wird auf einem Arbeitsthread gerufen (nie im Audio-Pfad); ein
/// Aufruf soll zuegig zurueckkehren (Auftrag starten, nicht abwarten), weil die Verbindung
/// des Agenten bis dahin wartet. Ein Fehler ist ein Klartext (kein Geheimnis, keine Inhalte).
pub trait ToolHandler: Send + Sync {
    fn specs(&self) -> Vec<ToolSpec>;
    fn call(&self, ctx: &CallContext, tool: &str, args: &Value) -> Result<Value, String>;
}

/// Warum `register` ein Werkzeug ablehnt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// Name nicht im Katalog (oder keine gueltige Schreibweise).
    NotInCatalog(String),
    Duplicate(String),
    /// Der Katalog ordnet eine Faehigkeit zu, die die Art `agent` nicht anbietet.
    CapabilityNotOffered(String),
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::NotInCatalog(n) => write!(f, "Werkzeug „{n}“ steht nicht im Katalog."),
            RegisterError::Duplicate(n) => write!(f, "Werkzeug „{n}“ ist schon registriert."),
            RegisterError::CapabilityNotOffered(n) => {
                write!(f, "Die Fähigkeit von „{n}“ bietet die Agent-Integration nicht an.")
            }
        }
    }
}

impl std::error::Error for RegisterError {}

struct Registered {
    spec: ToolSpec,
    handler: Arc<dyn ToolHandler>,
}

/// Die Werkzeuge, die diese App-Version tatsaechlich ausfuehren kann.
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Registered>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Nimmt alle Werkzeuge eines Handlers auf; scheitert ein Werkzeug, wird KEINES
    /// aufgenommen.
    pub fn register(&mut self, handler: Arc<dyn ToolHandler>) -> Result<(), RegisterError> {
        let specs = handler.specs();
        let mut seen: Vec<&str> = Vec::new();
        for s in &specs {
            let entry = valid_tool_name(&s.name)
                .then(|| find(&s.name))
                .flatten()
                .ok_or_else(|| RegisterError::NotInCatalog(s.name.clone()))?;
            if self.tools.contains_key(&s.name) || seen.contains(&s.name.as_str()) {
                return Err(RegisterError::Duplicate(s.name.clone()));
            }
            if !Kind::Agent.capabilities().contains(&entry.capability) {
                return Err(RegisterError::CapabilityNotOffered(s.name.clone()));
            }
            seen.push(&s.name);
        }
        for spec in specs {
            self.tools.insert(
                spec.name.clone(),
                Registered {
                    spec,
                    handler: handler.clone(),
                },
            );
        }
        Ok(())
    }

    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn spec(&self, name: &str) -> Option<&ToolSpec> {
        self.tools.get(name).map(|r| &r.spec)
    }

    pub fn handler(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        self.tools.get(name).map(|r| r.handler.clone())
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }
}

/// Hinweise fuer `tools/list` (MCP `annotations`): schreibende Werkzeuge sind nicht
/// schreibgeschuetzt und nicht idempotent.
pub fn annotations(entry: &CatalogEntry) -> Value {
    let read_only = entry.capability.access() == Access::Read;
    serde_json::json!({
        "readOnlyHint": read_only,
        "destructiveHint": false,
        "idempotentHint": false,
        "openWorldHint": false
    })
}

#[cfg(test)]
mod tests;
