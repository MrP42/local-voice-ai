//! G5 (Goal Issues-Abschluss #70): Grundlage und Ausgabesprache von Protokoll und
//! KI-Notizen.
//!
//! Beim Erzeugen laesst sich waehlen, aus welcher FASSUNG des Transkripts das Dokument
//! entsteht (Original oder Uebersetzung; Standard: die aktive) und in welcher SPRACHE es
//! geschrieben wird (Standard der Oberflaeche: Sprache der App bzw. letzte Wahl; ohne Angabe
//! wie bisher "wie das Transkript"). Beides steht im Dokument selbst (Protokoll-Kopf), in den
//! Metadaten der Dokumentversion (Info-Dialog) und im Prompt:
//! - der Prompt FORDERT die Ausgabesprache ausdruecklich (`language_rule`), statt es dem
//!   Modell zu ueberlassen;
//! - Quellspruenge und Belege verweisen auf die Segmente der Grundlage. Eine Uebersetzung hat
//!   dieselben Segmentnummern wie ihr Original, ein Beleg gilt also fuer beide Fassungen.
//!
//! Die Ausgabesprache erreicht die Prompt-Bausteine (`minutes.rs`, `notes/enhance.rs`) ueber
//! eine Task-Variable (`with_output_language`), nicht ueber jede Signatur: ohne Angabe bleibt
//! jeder Prompt Zeichen fuer Zeichen wie vor G5.

use std::future::Future;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::language::{detect_text, name_de, name_en, normalize_code};
use super::store::{MeetingStore, StoredSegment};
use super::variants::{self, TranscriptVariant, KIND_TRANSLATION};

/// Die Wahl beim Erzeugen.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DocBasis {
    /// Kennung der Fassung, aus der erzeugt wird; `None`: die aktive Fassung.
    pub variant_id: Option<String>,
    /// Sprache des Dokuments (Code); `None` oder `auto`: wie das Transkript.
    pub output_language: Option<String>,
}

/// Was die Wahl bedeutet: die Segmente der Grundlage und was darueber bekannt ist.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedBasis {
    pub segments: Vec<StoredSegment>,
    /// Die Fassung; `None`, wenn die Besprechung (noch) keine hat.
    pub variant: Option<TranscriptVariant>,
    /// Sprache der Grundlage (Fassung, sonst Besprechung, sonst erkannt).
    pub source_language: Option<String>,
    /// Gewuenschte Ausgabesprache (Basiscode); `None`: wie das Transkript.
    pub output_language: Option<String>,
}

/// Was in den Metadaten der Dokumentversion steht und der Oberflaeche gemeldet wird.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DocumentBasis {
    pub variant_id: Option<String>,
    pub variant_number: Option<u32>,
    /// `translation`, `stt`, `retranscribed`, ...
    pub variant_kind: Option<String>,
    /// Sprache der Grundlage.
    pub language: Option<String>,
    /// Ausgabesprache; `None`: wie die Grundlage.
    pub output_language: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BasisError {
    /// Die Fassung gibt es nicht oder sie gehoert zu einer anderen Besprechung.
    VariantNotFound,
    Store(String),
}

impl BasisError {
    pub fn code(&self) -> &'static str {
        match self {
            BasisError::VariantNotFound => "variant_not_found",
            BasisError::Store(_) => "store_failed",
        }
    }
}

impl std::fmt::Display for BasisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BasisError::Store(m) => write!(f, "{}: {m}", self.code()),
            other => write!(f, "{}", other.code()),
        }
    }
}

impl std::error::Error for BasisError {}

/// Die Segmente der gewaehlten Grundlage. Ohne `variant_id` das aktive Transkript (wie vor
/// G5); mit einer Kennung die Segmente dieser Fassung, auch wenn eine andere gerade aktiv
/// ist. Die aktive Fassung liefert den Stand von `transcripts` (Korrekturen von Hand).
pub fn resolve(
    store: &MeetingStore,
    meeting_id: &str,
    basis: &DocBasis,
) -> Result<ResolvedBasis, BasisError> {
    let output_language = basis.output_language.as_deref().and_then(normalize_code);
    let requested = basis.variant_id.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let (variant, segments) = match requested {
        Some(id) => {
            let conn = store.get_connection().map_err(|e| BasisError::Store(e.to_string()))?;
            let (variant, segments) = variants::get_segments(&conn, id).map_err(|e| match e {
                variants::VariantError::NotFound => BasisError::VariantNotFound,
                other => BasisError::Store(other.to_string()),
            })?;
            if variant.meeting_id != meeting_id {
                return Err(BasisError::VariantNotFound);
            }
            (Some(variant), segments)
        }
        None => {
            let segments = store
                .get_segments(meeting_id)
                .map_err(|e| BasisError::Store(e.to_string()))?;
            let variant = store.get_connection().ok().and_then(|mut conn| {
                variants::list(&mut conn, meeting_id)
                    .ok()
                    .and_then(|list| list.into_iter().find(|v| v.active))
            });
            (variant, segments)
        }
    };
    let source_language = variant
        .as_ref()
        .and_then(|v| v.language.as_deref())
        .and_then(normalize_code)
        .or_else(|| {
            store
                .get_meeting(meeting_id)
                .ok()
                .flatten()
                .and_then(|m| m.language)
                .and_then(|l| normalize_code(&l))
        })
        .or_else(|| {
            let sample: String = segments.iter().take(40).map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
            detect_text(&sample).map(|d| d.code)
        });
    Ok(ResolvedBasis {
        segments,
        variant,
        source_language,
        output_language,
    })
}

impl ResolvedBasis {
    /// Fuer `generation_metadata_json` der Dokumentversion.
    pub fn to_document_basis(&self) -> DocumentBasis {
        DocumentBasis {
            variant_id: self.variant.as_ref().map(|v| v.id.clone()),
            variant_number: self.variant.as_ref().map(|v| v.number),
            variant_kind: self.variant.as_ref().map(|v| v.kind.clone()),
            language: self.source_language.clone(),
            output_language: self.output_language.clone(),
        }
    }

    /// Die Eintraege, die in die Metadaten der Dokumentversion kommen.
    pub fn metadata(&self) -> Value {
        json!({ "basis": self.to_document_basis() })
    }

    /// Die Zeile im Kopf des Protokolls (Markdown), z. B.
    /// `**Grundlage:** Original (Englisch) · Protokoll auf Deutsch`.
    pub fn header_line(&self) -> String {
        basis_line(&self.to_document_basis())
    }
}

/// Aus den Metadaten einer Dokumentversion (`generation_metadata_json`) zurueck; `None` bei
/// einer Version vor G5 (keine Angabe).
pub fn from_metadata(metadata: &Value) -> Option<DocumentBasis> {
    serde_json::from_value(metadata.get("basis")?.clone()).ok()
}

/// Die Grundlage EINER Dokumentversion aus ihren Metadaten; `None` bei einer Version vor G5.
pub fn document_basis(store: &MeetingStore, document_id: &str) -> Option<DocumentBasis> {
    let metadata: Value =
        serde_json::from_str(&store.document_generation_metadata(document_id).ok().flatten()?).ok()?;
    from_metadata(&metadata)
}

/// Die Grundlage der juengsten Dokumentversion einer Art (`minutes`, `enhanced_notes`);
/// `None` ohne Dokument oder bei einer Version vor G5.
pub fn latest_document_basis(store: &MeetingStore, meeting_id: &str, kind: &str) -> Option<DocumentBasis> {
    let latest = store
        .get_documents(meeting_id)
        .ok()?
        .into_iter()
        .filter(|d| d.kind == kind)
        .max_by_key(|d| d.version)?;
    document_basis(store, &latest.id)
}

fn variant_word(kind: Option<&str>) -> &'static str {
    if kind == Some(KIND_TRANSLATION) {
        "Übersetzung"
    } else {
        "Original"
    }
}

/// Die Kopfzeile aus der Grundlage: Fassung (Original oder Übersetzung) mit ihrer Sprache und
/// die Sprache des Dokuments.
pub fn basis_line(basis: &DocumentBasis) -> String {
    let word = variant_word(basis.variant_kind.as_deref());
    let language = basis
        .language
        .as_deref()
        .map(|l| format!(" ({})", name_de(l)))
        .unwrap_or_default();
    let output = match basis.output_language.as_deref().or(basis.language.as_deref()) {
        Some(code) => format!("auf {}", name_de(code)),
        None => "in der Sprache der Grundlage".to_string(),
    };
    format!("**Grundlage:** {word}{language} · Protokoll {output}")
}

// ---------------------------------------------------------------------------
// Ausgabesprache im Prompt
// ---------------------------------------------------------------------------

tokio::task_local! {
    static OUTPUT_LANGUAGE: Option<String>;
}

/// Fuehrt `fut` mit der Ausgabesprache aus: alle Prompt-Bausteine darin fordern sie.
pub async fn with_output_language<F: Future>(language: Option<String>, fut: F) -> F::Output {
    OUTPUT_LANGUAGE.scope(language, fut).await
}

/// Die Regel "Sprache" der Prompts. Ohne Ausgabesprache der Satz wie vor G5; mit ihr eine
/// ausdrueckliche Forderung, die auch greift, wenn das Transkript in einer anderen Sprache
/// steht. Namen, Zahlen, Daten und woertliche Zitate bleiben, wie sie sind.
pub fn language_rule() -> String {
    match OUTPUT_LANGUAGE.try_with(|l| l.clone()).ok().flatten() {
        Some(code) => format!(
            "Write ALL texts of the reply in {}, even if the transcript or the notes are in \
another language. Names, numbers, dates and quoted words stay exactly as they are in the \
transcript.",
            name_en(&code)
        ),
        None => "Same language as the transcript.".to_string(),
    }
}

#[cfg(test)]
mod tests;
