//! G5: Kommandos der Sprache einer Besprechung: der Chip im Kopf (Sprache, Herkunft,
//! Modell, Vorschlag), die Korrektur von Hand und das passende Modell einer Sprache fuer
//! den Dialog der Neu-Transkription. Die Logik steht in `meetings::language` (rein) und
//! `meetings::language_run` (Anbindung); hier nur Argumente, Tor fuer laufende Auftraege
//! und Fehlercodes.
//!
//! Fehler gehen als Code an die Oberflaeche (`meeting_not_found`, `meeting_busy`,
//! `language_invalid`, `variant_*`).

use std::sync::Arc;

use tauri::State;

use crate::managers::meetings::basis::{self, DocumentBasis};
use crate::managers::meetings::job;
use crate::managers::meetings::language::{
    normalize_code, LanguageInfo, ModelSuggestion, StoredLanguage, SOURCE_USER,
};
use crate::managers::meetings::language_run;
use crate::managers::meetings::store::MeetingStore;
use crate::managers::model::ModelManager;

/// Der Chip im Kopf: die Sprache der Besprechung mit Herkunft, das Modell der
/// Transkription und, wenn es die Sprache nicht abdeckt, ein besseres.
#[tauri::command]
#[specta::specta]
pub async fn meetings_language_info(
    store: State<'_, Arc<MeetingStore>>,
    models: State<'_, Arc<ModelManager>>,
    meeting_id: String,
) -> Result<LanguageInfo, String> {
    language_run::info(&store, &language_run::candidates(&models), &meeting_id)
}

/// „Sprache korrigieren“: der Nutzer setzt die Sprache (Chip im Kopf). Gilt fuer die
/// Besprechung, das aktive Transkript und die aktive Fassung, mit Herkunft `user`; andere
/// Fassungen behalten ihre Sprache. Nicht waehrend einer Verarbeitung (`meeting_busy`).
/// Das Transkript selbst aendert sich nicht: eine Neu-Transkription mit dem passenden Modell
/// bietet die Oberflaeche danach an (`meetings_retranscribe` mit dieser Sprache).
#[tauri::command]
#[specta::specta]
pub async fn meetings_set_language(
    store: State<'_, Arc<MeetingStore>>,
    models: State<'_, Arc<ModelManager>>,
    meeting_id: String,
    language: String,
) -> Result<LanguageInfo, String> {
    let code = normalize_code(&language).ok_or_else(|| "language_invalid".to_string())?;
    if job::global().is_running(&meeting_id) {
        return Err("meeting_busy".to_string());
    }
    let previous = language_run::stored_language(&store, &meeting_id);
    let stored = StoredLanguage {
        code,
        source: SOURCE_USER.to_string(),
        confidence: Some(1.0),
        forced: None,
        mismatch: None,
        // Das Modell, das transkribiert hat, bleibt die Auskunft ueber den Text.
        model_id: previous.and_then(|p| p.model_id),
    };
    language_run::persist(&store, &meeting_id, &stored)?;
    language_run::info(&store, &language_run::candidates(&models), &meeting_id)
}

/// Das Modell, das die App fuer `language` waehlen wuerde (installiert und genau, sonst
/// ein Vorschlag aus dem Katalog), vom Modell `current_model` aus gesehen. Fuer die
/// Vorbelegung im Dialog der Neu-Transkription. `None`: keine Antwort.
#[tauri::command]
#[specta::specta]
pub async fn meetings_model_for_language(
    models: State<'_, Arc<ModelManager>>,
    current_model: Option<String>,
    language: String,
) -> Result<Option<ModelSuggestion>, String> {
    Ok(language_run::model_for_language(
        &language_run::candidates(&models),
        current_model.as_deref().unwrap_or(""),
        &language,
    ))
}

/// G5: Grundlage und Ausgabesprache einer Version von Protokoll (`minutes`) oder KI-Notizen
/// (`enhanced_notes`): aus welcher Fassung des Transkripts, in welcher Sprache. `document_id`
/// waehlt die Version, ohne ist es die juengste. `None` ohne Dokument und bei Versionen aus
/// der Zeit davor.
#[tauri::command]
#[specta::specta]
pub async fn meetings_document_basis(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    kind: String,
    document_id: Option<String>,
) -> Result<Option<DocumentBasis>, String> {
    Ok(match document_id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => basis::document_basis(&store, id),
        None => basis::latest_document_basis(&store, &meeting_id, &kind),
    })
}
