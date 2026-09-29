//! Chat ueber Besprechungen (M4, Paket P4c). Siehe
//! `koordination/granola-besprechungen/entwurf/m4-chat-suche.md` §4 bis §7.
//!
//! Kein Tool-Calling durch das Modell, sondern ein fester Ablauf in Rust
//! (D4): Suche -> Shortlist (Breite) -> Lesen (Tiefe) -> hoechstens eine
//! Wiederholung -> Antwort -> Abdeckung. Zitate entstehen per Konstruktion
//! (D5): das Modell nennt nur Auszugs-IDs `[Q<k>]`, Rust bildet sie auf
//! Besprechung, Epoche und Segment ab und nummeriert fuer die Anzeige um.
//!
//! - `context`: Budget, Auszuege, Shortlist, In-Memory-BM25 (rein).
//! - `prompt`: System- und Nutzerprompt (rein).
//! - `citations`: Nachbearbeitung der Antwort, Segment-Verfeinerung,
//!   Filter fuer die gestreamten Stuecke (rein).
//! - `live`: Auszuege waehrend einer laufenden Aufnahme (ohne Index).
//! - `recipes`: Recipes mit Variablen, mitgelieferte Recipes.
//! - `controller`: der Ablauf mit Store, Embedder und LLM-Stream.
//! - `eval`: Eval AK8 (`--eval-chat`, P4f) auf synthetischen Fixtures.
//!
//! Datenschutz (wie M1 D9): kein Frage-, Antwort- oder Auszugstext im Log und
//! in Fehlermeldungen; nur Codes, Laengen, Zaehler. Kein Modul hier ruft
//! `settings::get_settings(&AppHandle)`: Einstellungen kommen als Parameter.

pub mod citations;
pub mod context;
pub mod controller;
pub mod eval;
pub mod live;
pub mod prompt;
pub mod recipes;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use specta::Type;

pub use super::search::chunking::ChunkSource;
pub use super::search::index::ScopeFilter;

// ---------------------------------------------------------------------------
// Fehlercodes (MeetingChatEvent::Failed)
// ---------------------------------------------------------------------------

pub const CODE_NO_PROVIDER: &str = "no_provider";
pub const CODE_NO_MODEL: &str = "no_model";
pub const CODE_MEMORY_LOW: &str = "memory_low";
pub const CODE_RECORDING_ACTIVE_CPU: &str = "recording_active_cpu";
pub const CODE_CHAT_BUSY: &str = "chat_busy";
pub const CODE_EMPTY_SCOPE: &str = "empty_scope";
pub const CODE_LLM_FAILED: &str = "llm_failed";
pub const CODE_CANCELLED: &str = "cancelled";
/// Zusaetzlich zu §5: Fehler vor jedem LLM-Aufruf, die die UI unterscheiden will.
pub const CODE_RECIPE_INVALID: &str = "recipe_invalid";
pub const CODE_INVALID_REQUEST: &str = "invalid_request";
pub const CODE_MEETING_NOT_FOUND: &str = "meeting_not_found";
pub const CODE_THREAD_NOT_FOUND: &str = "thread_not_found";
pub const CODE_STORE_FAILED: &str = "store_failed";

/// Alle Codes, die `MeetingChatEvent::Failed` tragen kann.
pub const EVENT_CODES: [&str; 13] = [
    CODE_NO_PROVIDER,
    CODE_NO_MODEL,
    CODE_MEMORY_LOW,
    CODE_RECORDING_ACTIVE_CPU,
    CODE_CHAT_BUSY,
    CODE_EMPTY_SCOPE,
    CODE_LLM_FAILED,
    CODE_CANCELLED,
    CODE_RECIPE_INVALID,
    CODE_INVALID_REQUEST,
    CODE_MEETING_NOT_FOUND,
    CODE_THREAD_NOT_FOUND,
    CODE_STORE_FAILED,
];

/// Fehler eines Chat-Laufs: `code` fuer UI und Log, `detail` ohne Nutzertext.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatError {
    pub code: &'static str,
    pub detail: String,
}

impl ChatError {
    pub fn code_only(code: &'static str) -> Self {
        Self {
            code,
            detail: String::new(),
        }
    }

    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.detail.is_empty() {
            write!(f, "{}", self.code)
        } else {
            write!(f, "{}: {}", self.code, self.detail)
        }
    }
}

impl From<ChatError> for String {
    fn from(e: ChatError) -> String {
        e.to_string()
    }
}

// ---------------------------------------------------------------------------
// Anfrage und Antwort
// ---------------------------------------------------------------------------

/// Worauf sich ein Chat bezieht: eine Besprechung (auch waehrend der
/// Aufnahme) oder viele (alle / Ordner / Person / Zeitraum / Auswahl).
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatScope {
    Meeting { meeting_id: String },
    Global { filter: ScopeFilter },
}

/// Aufruf eines Recipes: ID (`builtin:<key>` oder eigene) und Werte der
/// Variablen nach Name. Werte fuer `folder` sind Ordner-IDs, fuer `meeting`
/// Besprechungs-IDs, fuer `date_*` `JJJJ-MM-TT`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
pub struct RecipeCall {
    pub recipe_id: String,
    #[serde(default)]
    pub values: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ChatRequest {
    /// Von der UI vergeben; Schluessel fuer Deltas und `meeting_chat_cancel`.
    pub request_id: String,
    /// `None` = neuer Verlauf.
    pub thread_id: Option<String>,
    pub scope: ChatScope,
    /// Freitext; darf leer sein, wenn ein Recipe gewaehlt ist (dann ergaenzt er es).
    pub question: String,
    pub recipe: Option<RecipeCall>,
}

/// Ein Beleg in der Antwort. `n` ist die Anzeige-Nummer (`[n]` im Text).
/// Transkript: `segment_index` + `start_ms` (Epoche `epoch`); Notizen:
/// `ref_key` (NoteBlock-ID bzw. KI-Notizen-Eintrag "E7").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Citation {
    pub n: u32,
    pub meeting_id: String,
    pub meeting_title: String,
    pub started_at: Option<i64>,
    pub source: ChunkSource,
    pub epoch: u32,
    pub segment_index: Option<u32>,
    pub start_ms: Option<u64>,
    pub ref_key: Option<String>,
    /// Hoechstens 200 Zeichen aus der belegten Stelle.
    pub quote: String,
}

/// Was der Chat gesehen hat (deterministisch; die UI formt daraus die graue
/// Abdeckungszeile).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct Coverage {
    pub meetings_in_scope: u32,
    pub meetings_with_hits: u32,
    pub meetings_read: u32,
    pub excerpts_read: u32,
    /// Ohne Vektoren gesucht (nur Stichwortsuche).
    pub lexical_only: bool,
    /// LLM-Runden (0 = ohne Treffer gar nicht gefragt, 2 = mit Wiederholung).
    pub rounds: u8,
    /// Es gab mehr passende Stellen, als ins Budget passten.
    pub truncated: bool,
    /// Waehrend der Aufnahme gefragt (Live-Auszuege ohne Index).
    pub live: bool,
    /// Zitate auf unbekannte Auszugs-IDs, verworfen.
    pub dropped_citations: u32,
    /// Lokales Modell auf CPU: kleineres Budget ("CPU: weniger Auszuege gelesen").
    pub cpu_limited: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct ChatAnswer {
    pub thread_id: String,
    pub message_id: String,
    /// Antworttext mit `[1]`, `[2]` ... (leer bei `not_found`).
    pub text: String,
    pub citations: Vec<Citation>,
    pub coverage: Coverage,
    /// Das Modell fand in den Auszuegen keinen Beleg (UI zeigt den i18n-Text).
    pub not_found: bool,
    /// Antwort ohne ein einziges gueltiges Zitat (UI-Hinweis "ohne Beleg").
    pub uncited: bool,
    /// Lokales Modell (sonst gingen Auszuege an einen externen Anbieter).
    pub provider_local: bool,
}

/// Phase eines Laufs fuer `MeetingChatEvent::Stage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChatStage {
    Searching,
    Reading,
    Answering,
}

/// Eine gespeicherte Nachricht, fuer die UI aufbereitet
/// (`meeting_chat_thread`). Unlesbares JSON einer Zeile kostet deren Zitate,
/// nicht den Verlauf.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct ChatMessage {
    pub id: String,
    /// `user` | `assistant`
    pub role: String,
    pub text: String,
    pub citations: Vec<Citation>,
    pub coverage: Option<Coverage>,
    pub not_found: bool,
    pub uncited: bool,
    pub created_at: i64,
}

/// Was neben den Zitaten zu einer Antwort gespeichert wird (Spalte
/// `coverage_json`). `excerpt_chunk_ids` haelt die gelesenen Chunks fest:
/// eine Folgefrage im selben Verlauf stellt sie wieder nach vorn (Prompt-Cache).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageMeta {
    pub coverage: Coverage,
    pub not_found: bool,
    pub uncited: bool,
    pub provider_local: bool,
    pub excerpt_chunk_ids: Vec<i64>,
}

impl ChatMessage {
    pub fn from_row(row: &super::search::index::ChatMessageRow) -> Self {
        let citations: Vec<Citation> = row
            .citations_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        let meta: Option<MessageMeta> = row
            .coverage_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok());
        Self {
            id: row.id.clone(),
            role: row.role.clone(),
            text: row.content.clone(),
            citations,
            not_found: meta.as_ref().is_some_and(|m| m.not_found),
            uncited: meta.as_ref().is_some_and(|m| m.uncited),
            coverage: meta.map(|m| m.coverage),
            created_at: row.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::search::index::ChatMessageRow;

    #[test]
    fn scope_and_event_types_serialize_as_the_ui_expects() {
        let scope = serde_json::to_value(ChatScope::Meeting {
            meeting_id: "m1".into(),
        })
        .unwrap();
        assert_eq!(
            scope,
            serde_json::json!({"kind": "meeting", "meeting_id": "m1"})
        );
        let global: ChatScope = serde_json::from_value(
            serde_json::json!({"kind": "global", "filter": {"person": "Anna"}}),
        )
        .unwrap();
        match global {
            ChatScope::Global { filter } => assert_eq!(filter.person.as_deref(), Some("Anna")),
            _ => panic!("global erwartet"),
        }
        assert_eq!(
            serde_json::to_value(ChatStage::Answering).unwrap(),
            "answering"
        );
        assert_eq!(
            ChatError::new(CODE_LLM_FAILED, "Status 500").to_string(),
            "llm_failed: Status 500"
        );
        assert_eq!(
            ChatError::code_only(CODE_CHAT_BUSY).to_string(),
            "chat_busy"
        );
    }

    #[test]
    fn a_stored_message_with_broken_json_keeps_its_text() {
        let row = ChatMessageRow {
            id: "x".into(),
            thread_id: "t".into(),
            role: "assistant".into(),
            content: "Antwort [1]".into(),
            citations_json: Some("{kaputt".into()),
            coverage_json: Some(
                serde_json::to_string(&MessageMeta {
                    not_found: false,
                    uncited: true,
                    coverage: Coverage {
                        rounds: 2,
                        ..Coverage::default()
                    },
                    ..MessageMeta::default()
                })
                .unwrap(),
            ),
            created_at: 7,
        };
        let msg = ChatMessage::from_row(&row);
        assert_eq!(msg.text, "Antwort [1]");
        assert!(msg.citations.is_empty());
        assert!(msg.uncited);
        assert_eq!(msg.coverage.unwrap().rounds, 2);
    }

    #[test]
    fn public_types_export_to_typescript() {
        let mut types = specta::TypeCollection::default();
        types
            .register::<ChatRequest>()
            .register::<ChatAnswer>()
            .register::<ChatMessage>()
            .register::<ChatStage>()
            .register::<recipes::RecipeItem>();
        let ts = specta_typescript::Typescript::default()
            .bigint(specta_typescript::BigIntExportBehavior::Number)
            .export(&types)
            .expect("Typen muessen exportierbar sein");
        if let Ok(path) = std::env::var("LVA_DUMP_CHAT_TS") {
            std::fs::write(path, &ts).unwrap();
        }
        for expected in [
            "export type ChatScope = { kind: \"meeting\"; meeting_id: string } | { kind: \"global\"; filter: ScopeFilter }",
            "export type ChatStage = \"searching\" | \"reading\" | \"answering\"",
            "export type Citation",
            "export type Coverage",
            "export type RecipeSpec",
        ] {
            assert!(ts.contains(expected), "{expected} fehlt in:\n{ts}");
        }
    }
}
