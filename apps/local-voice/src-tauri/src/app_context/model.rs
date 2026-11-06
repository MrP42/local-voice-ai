//! Datenmodell des globalen Abrufindex.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Art einer Quelle. Der Schlüssel (`key`) ist zugleich der Bereich in der
/// Freigabe des Sprachgesprächs.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Book,
    BookNote,
    KiDoc,
    Meeting,
    Dictation,
    Conversation,
    TtsPage,
}

impl SourceKind {
    pub const ALL: [SourceKind; 7] = [
        SourceKind::Book,
        SourceKind::BookNote,
        SourceKind::KiDoc,
        SourceKind::Meeting,
        SourceKind::Dictation,
        SourceKind::Conversation,
        SourceKind::TtsPage,
    ];

    pub fn key(self) -> &'static str {
        match self {
            SourceKind::Book => "books",
            SourceKind::BookNote => "book_notes",
            SourceKind::KiDoc => "ki_docs",
            SourceKind::Meeting => "meetings",
            SourceKind::Dictation => "dictations",
            SourceKind::Conversation => "conversations",
            SourceKind::TtsPage => "tts",
        }
    }

    pub fn parse(key: &str) -> Option<SourceKind> {
        SourceKind::ALL.into_iter().find(|k| k.key() == key)
    }

    /// Quellen, die immer zu einem Buch gehören: ohne `book_id` werden sie
    /// nicht aufgenommen.
    pub fn requires_book(self) -> bool {
        matches!(self, SourceKind::BookNote | SourceKind::KiDoc)
    }

    /// Bezeichnung im Prompt (und damit, was das Modell sagen darf).
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Book => "Buch",
            SourceKind::BookNote => "Buchnotiz",
            SourceKind::KiDoc => "KI-Dokument",
            SourceKind::Meeting => "Besprechung",
            SourceKind::Dictation => "Diktat",
            SourceKind::Conversation => "Gespräch",
            SourceKind::TtsPage => "Vorlesen-Seite",
        }
    }
}

/// Eindeutiger Schlüssel eines Dokuments im Index.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocKey {
    pub kind: SourceKind,
    pub id: String,
}

/// Ein Dokument, wie die App-Schicht es liefert.
#[derive(Clone, Debug)]
pub struct ContextDoc {
    pub kind: SourceKind,
    /// Kennung in der Quelle (Buch-Id, Besprechungs-Id, Diktat-Id …).
    pub id: String,
    pub title: String,
    /// Zeitpunkt (ms seit Epoche) für Aktualitätsfragen; 0 = unbekannt.
    pub at_ms: i64,
    pub text: String,
    /// Buchbezug abgeleiteter Inhalte (Notiz, KI-Dokument, Gespräch über ein
    /// Buch): Buch-Id. Bei Büchern selbst `None` (die Id ist `id`).
    pub book_id: Option<String>,
    /// Bis zu welcher Stelle (UTF-16 im Buch) ein abgeleiteter Inhalt das Buch
    /// belegt. `None` bei gesetztem `book_id` = unklar → nur mit
    /// Ganzes-Buch-Freigabe sichtbar (Spoilerschutz, Codex Must-fix 3).
    pub book_upto: Option<usize>,
    /// Datenklasse `restricted` oder besondere Kategorie, auch abgeleitet:
    /// wird nie indiziert (Codex Must-fix 1).
    pub restricted: bool,
}

impl ContextDoc {
    pub fn key(&self) -> DocKey {
        DocKey {
            kind: self.kind,
            id: self.id.clone(),
        }
    }
}

/// Sprungziel eines Treffers in der App.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Jump {
    pub kind: SourceKind,
    pub id: String,
    /// UTF-16-Offset im Original (Bücher) bzw. Zeichenoffset (sonst).
    pub offset: u32,
    pub page: Option<u32>,
    pub book_id: Option<String>,
}

/// Ein Treffer für Prompt und Verlauf.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct ContextHit {
    /// Belegnummer `Q<k>`, ab 1 in Rangfolge.
    pub k: u32,
    pub kind: SourceKind,
    pub title: String,
    /// Kapitel (nur Bücher).
    pub chapter: Option<String>,
    pub at_ms: i64,
    pub excerpt: String,
    pub jump: Jump,
    pub score: f64,
    /// Quellversion (Hash des Dokuments beim Abruf), für die Belegprüfung.
    pub version: String,
}

/// Ergebnis eines Abrufs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct ContextHits {
    pub hits: Vec<ContextHit>,
    /// Bereiche, die freigegeben waren (für die Anzeige „gesucht in …“).
    pub searched: Vec<SourceKind>,
    /// Dauer des Abrufs (ms), für die Latenzmessung.
    pub retrieve_ms: u32,
    /// Stand des Index beim Abruf. Jede Löschung, jeder Widerruf und jede
    /// Umklassifizierung erhöht ihn; ein Ergebnis mit älterem Stand ist
    /// ungültig und wird verworfen (Codex Must-fix 2).
    pub epoch: u32,
}

/// Kontext für einen Sprecheraufruf (`voice_call_chat`). Das Backend baut die
/// Kontextnachricht selbst und nur, wenn der Empfänger lokal ist und Stand,
/// Freigaben und Quellen noch stimmen (Codex M5/M6, Auflagen 4–6, 11).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct SpeakerContext {
    pub hits: ContextHits,
    pub german: bool,
}
