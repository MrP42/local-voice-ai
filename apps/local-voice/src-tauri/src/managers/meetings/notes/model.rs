//! Datenmodell fuer Notizblock, KI-Notizen, Vorlagen und Aufgaben (M1, P1a).
//!
//! Reine Typen ohne Logik. Sie leben hier, damit Store (`store.rs`), Vorlagen
//! (`templates.rs`) und spaeter Motor/Commands (P1b/P1c) dieselbe Definition
//! benutzen. Alle Typen tragen `specta::Type`, damit sie ohne Handarbeit in
//! `bindings.ts` landen, sobald ein Command sie verwendet.

use serde::{Deserialize, Serialize};
use specta::Type;

// ---------------------------------------------------------------------------
// Nutzernotizen (Notizblock)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum NoteBlockKind {
    Paragraph,
    Bullet,
    Heading,
    Todo,
}

/// Ein Block des Notizblocks. Die ID erzeugt das Frontend (ULID); `at_ms` ist
/// die Audioposition beim Anlegen (`None` = importiert oder nach dem Stopp
/// geschrieben) auf derselben Zeitachse wie `StoredSegment.start_ms`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct NoteBlock {
    pub id: String,
    pub kind: NoteBlockKind,
    pub text: String,
    pub at_ms: Option<u64>,
    pub checked: bool,
}

/// Der gesamte Notizblock einer Besprechung. `revision` ist der Zaehler der
/// optimistischen Sperre (`save_notes`); `updated_at` in Sekunden wie alle
/// Zeilen der Tabelle `meeting_notes` (0 = noch nie gespeichert).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MeetingNotes {
    pub meeting_id: String,
    pub blocks: Vec<NoteBlock>,
    pub revision: u64,
    pub updated_at: i64,
}

// ---------------------------------------------------------------------------
// Vorlagen
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    Text,
    Tasks,
}

/// Ein Abschnitt einer Vorlage. `id` ist ein stabiler Schluessel
/// (`[a-z0-9_]{1,32}`), unter dem das Modell Eintraege ablegt; `title` sieht
/// der Nutzer; `instruction` geht an das Modell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TemplateSection {
    pub id: String,
    pub title: String,
    pub instruction: String,
    pub kind: SectionKind,
}

/// Inhalt einer Vorlage; wird als JSON in `meeting_templates.sections_json`
/// abgelegt. `version` ist heute immer 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TemplateSpec {
    pub version: u32,
    pub context: String,
    pub sections: Vec<TemplateSection>,
}

/// Vorlage samt Metadaten, wie sie an die UI geht. `builtin` = mitgeliefert
/// (ID `builtin:<key>`, schreibgeschuetzt). `updated_at` in Sekunden.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TemplateInfo {
    pub id: String,
    pub title: String,
    pub builtin: bool,
    pub spec: TemplateSpec,
    pub updated_at: i64,
}

// ---------------------------------------------------------------------------
// KI-Notizen (Dokumentversion `enhanced_notes`, Body = JSON dieser Struktur)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    User,
    Ai,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EntryFlags {
    /// KI-Eintrag ohne gueltige Quelle.
    pub unsupported: bool,
    pub dropped_sources: u32,
    pub placed_by_fallback: bool,
    pub edited: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EnhancedEntry {
    /// "E1", "E2", ... in Ausgabereihenfolge.
    pub id: String,
    pub origin: Origin,
    pub text: String,
    /// Bei `origin = User`: der Block, aus dem der Text stammt.
    pub note_id: Option<String>,
    pub source_segment_ids: Vec<u32>,
    /// D5: Nummern der Folien (`meeting_slides.number`), auf die sich der Eintrag
    /// belegt (`F7`). Fehlt in aelteren Dokumenten: dann leer.
    #[serde(default)]
    pub source_slide_ids: Vec<u32>,
    pub assignee: Option<String>,
    pub due: Option<String>,
    pub flags: EntryFlags,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EnhancedSection {
    pub id: String,
    pub title: String,
    pub kind: SectionKind,
    pub entries: Vec<EnhancedEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EnhanceStats {
    pub user_notes_total: u32,
    pub user_notes_by_model: u32,
    pub user_notes_by_fallback: u32,
    pub ai_entries: u32,
    pub ai_entries_sourced: u32,
    pub dropped_source_ids: u32,
    pub chunks_total: u32,
    pub chunks_failed: Vec<u32>,
    pub single_pass: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EnhancedNotes {
    /// Immer "enhanced@1".
    pub format: String,
    pub template_id: Option<String>,
    pub template_title: String,
    /// `transcripts.segment_epoch` zum Zeitpunkt der Erzeugung.
    pub segment_epoch: u32,
    pub sections: Vec<EnhancedSection>,
    pub stats: EnhanceStats,
}

// ---------------------------------------------------------------------------
// Aufgaben
// ---------------------------------------------------------------------------

/// Status-Werte von `ActionItem.status`.
pub const STATUS_TODO: &str = "todo";
pub const STATUS_DONE: &str = "done";

/// Herkunfts-Werte von `ActionItem.source`.
pub const SOURCE_AI: &str = "ai";
pub const SOURCE_USER: &str = "user";
pub const SOURCE_MANUAL: &str = "manual";

/// Eine Aufgabe (Zeile in `action_items`). `assignee_label` ist Freitext, die
/// Verknuepfung mit der `humans`-Tabelle folgt in M9.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ActionItem {
    pub id: String,
    pub meeting_id: String,
    pub text: String,
    /// `todo` | `done`
    pub status: String,
    pub assignee_label: Option<String>,
    /// Erzeugende KI-Notizen-Version (`None` bei `manual`).
    pub document_id: Option<String>,
    /// Eintrag darin ("E7").
    pub entry_id: Option<String>,
    pub source_segment_ids: Vec<u32>,
    /// `ai` | `user` | `manual`
    pub source: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&NoteBlockKind::Todo).unwrap(),
            "\"todo\""
        );
        assert_eq!(
            serde_json::to_string(&SectionKind::Tasks).unwrap(),
            "\"tasks\""
        );
        assert_eq!(serde_json::to_string(&Origin::Ai).unwrap(), "\"ai\"");
    }

    /// Die Typen muessen sich so nach TypeScript exportieren lassen, wie
    /// `lib.rs` es tut (u64 als number), sonst bricht erst der Debug-Lauf
    /// von P1b/P1c.
    #[test]
    fn all_types_export_to_typescript() {
        let mut types = specta::TypeCollection::default();
        types
            .register::<NoteBlock>()
            .register::<MeetingNotes>()
            .register::<TemplateInfo>()
            .register::<EnhancedNotes>()
            .register::<ActionItem>();
        let ts = specta_typescript::Typescript::default()
            .bigint(specta_typescript::BigIntExportBehavior::Number)
            .export(&types)
            .expect("Typen muessen exportierbar sein");
        for expected in [
            "export type NoteBlock",
            "export type MeetingNotes",
            "export type TemplateSpec",
            "export type EnhancedNotes",
            "export type ActionItem",
            "export type NoteBlockKind = \"paragraph\" | \"bullet\" | \"heading\" | \"todo\"",
            "export type Origin = \"user\" | \"ai\"",
            "at_ms: number | null",
            "source_slide_ids?: number[]",
        ] {
            assert!(ts.contains(expected), "{expected} fehlt in:\n{ts}");
        }
    }

    #[test]
    fn a_note_block_round_trips_with_and_without_a_timestamp() {
        for at_ms in [Some(195_000u64), None] {
            let b = NoteBlock {
                id: "01J0".into(),
                kind: NoteBlockKind::Bullet,
                text: "Größe & Übergang 🚀".into(),
                at_ms,
                checked: false,
            };
            let json = serde_json::to_string(&b).unwrap();
            assert_eq!(serde_json::from_str::<NoteBlock>(&json).unwrap(), b);
        }
    }
}
