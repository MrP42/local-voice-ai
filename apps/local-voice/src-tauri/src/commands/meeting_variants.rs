//! Kommandos der Transkript-Fassungen (A3): auflisten, waehlen, Segmente fuer den
//! Vergleich, KI-Zusammenfuehren. Die Logik steht in `meetings::variants` und
//! `meetings::merge`; hier nur Argumente, Tor fuer laufende Auftraege und Fehler.
//!
//! Fehler gehen als Code an die Oberflaeche (`variant_*`, `merge_*`, `no_provider`).

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::managers::meetings::job;
use crate::managers::meetings::merge;
use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::meetings::store::{MeetingStore, StoredSegment};
use crate::managers::meetings::variants::{self, TranscriptVariant};

fn variant_error(e: variants::VariantError) -> String {
    e.to_string()
}

/// Die Fassungen einer Besprechung. Eine unterbrochene Neu-Transkription (Absturz)
/// wird vorher zurueckgenommen, sofern kein Auftrag mehr laeuft.
#[tauri::command]
#[specta::specta]
pub async fn transcript_variants(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<TranscriptVariant>, String> {
    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    if !job::global().is_running(&meeting_id) {
        // Nur wenn der Status nicht mehr `processing` ist: dann gehoert die Arbeitskopie
        // in `transcripts` keinem Lauf mehr.
        let _ = variants::recover_interrupted(&mut conn, &meeting_id);
    }
    variants::list(&mut conn, &meeting_id).map_err(variant_error)
}

/// „Fassung waehlen“: macht die Fassung zum aktiven Transkript.
#[tauri::command]
#[specta::specta]
pub async fn transcript_variant_activate(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    variant_id: String,
) -> Result<TranscriptVariant, String> {
    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    let (variant, _) = variants::get_segments(&conn, &variant_id).map_err(variant_error)?;
    if job::global().is_running(&variant.meeting_id) {
        return Err(variants::VariantError::Busy.to_string());
    }
    let chosen = variants::activate(&mut conn, &variant_id).map_err(variant_error)?;
    indexer::submit(&app, IndexJob::Meeting(chosen.meeting_id.clone()));
    Ok(chosen)
}

/// Die Segmente einer Fassung (fuer die Vergleichsansicht). Die aktive Fassung liefert
/// den Stand von `transcripts`, also auch Korrekturen von Hand.
#[tauri::command]
#[specta::specta]
pub async fn transcript_variant_segments(
    store: State<'_, Arc<MeetingStore>>,
    variant_id: String,
) -> Result<Vec<StoredSegment>, String> {
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    variants::get_segments(&conn, &variant_id)
        .map(|(_, segments)| segments)
        .map_err(variant_error)
}

/// „Zusammenfuehren“: das lokale Modell verbessert den Text von Fassung `base_id`
/// mit Hilfe von `other_id` und legt eine dritte Fassung an (nicht aktiv). Ausgaben,
/// die das Schema verletzen oder zu viel erfinden, werden verworfen (`merge`).
#[tauri::command]
#[specta::specta]
pub async fn transcript_variants_merge(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    base_id: String,
    other_id: String,
) -> Result<TranscriptVariant, String> {
    if job::global().is_running(&meeting_id) {
        return Err(variants::VariantError::Busy.to_string());
    }
    let store: Arc<MeetingStore> = Arc::clone(&store);
    let settings = crate::settings::get_settings(&app);
    merge::merge_variants(&settings, &store, &meeting_id, &base_id, &other_id).await
}
