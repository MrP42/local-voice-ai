//! Kommandos der Transkript-Fassungen (A3): auflisten, waehlen, Segmente fuer den
//! Vergleich, KI-Zusammenfuehren. Die Logik steht in `meetings::variants` und
//! `meetings::merge`; hier nur Argumente, Tor fuer laufende Auftraege und Fehler.
//!
//! Fehler gehen als Code an die Oberflaeche (`variant_*`, `merge_*`, `no_provider`).

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::managers::meetings::job::{self, Gate, JobPhase};
use crate::managers::meetings::merge;
use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::meetings::store::{MeetingStore, StoredSegment};
use crate::managers::meetings::translate::{self, Control, TranslationReport};
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

/// G5: „Übersetzen nach …“: das lokale Modell übersetzt die Fassung `source_variant_id`
/// Satz für Satz nach `target_language` und legt eine NEUE Fassung `translation` an (nicht
/// aktiv). Die Quelle bleibt unverändert und jederzeit wählbar. Läuft als Auftrag der
/// Besprechung (Phase Übersetzung): Fortschritt in Blöcken, Pause, Stopp; bei Stopp, Fehler
/// oder Absturz entsteht keine Fassung. Fehler: `translate_*`, `variant_*`, `no_provider`,
/// `no_model`, `job_busy`.
#[tauri::command]
#[specta::specta]
pub async fn transcript_variant_translate(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    source_variant_id: String,
    target_language: String,
) -> Result<TranscriptVariant, String> {
    let guard = job::global()
        .try_start(&meeting_id, job::app_emit(&app))
        .map_err(|e| e.to_string())?;
    let handle = Arc::clone(guard.handle());
    handle.begin_phase_ex(JobPhase::Translation, 0, false);
    let settings = crate::settings::get_settings(&app);
    let store: Arc<MeetingStore> = Arc::clone(&store);
    let progress_job = Arc::clone(&handle);
    let run = job::scope(
        Arc::clone(&handle),
        translate::translate_variant(
            &settings,
            &store,
            &meeting_id,
            &source_variant_id,
            &target_language,
            || async {
                match job::checkpoint().await {
                    Gate::Go { .. } => Control::Go,
                    Gate::Stopped | Gate::Cancelled => Control::Stop,
                }
            },
            move |done, total| {
                if done == 0 {
                    // Mehr als ein Block: dazwischen laesst sich anhalten.
                    progress_job.begin_phase_ex(JobPhase::Translation, total as u64, total >= 2);
                } else {
                    progress_job.advance(done as u64);
                }
            },
        ),
    );
    // Ein Stopp beendet die Anfrage an das Modell sofort; nichts wird gespeichert.
    let result = tokio::select! {
        result = run => result,
        _ = handle.stopped() => Err("translate_cancelled".to_string()),
    };
    drop(guard);
    if let Err(code) = &result {
        log::warn!("Übersetzung beendet ohne Fassung: {}", code.split(':').next().unwrap_or(code));
    }
    result
}

/// G5: der Prüfbericht einer Übersetzung (markierte Sätze mit Grund, Quelle, Sprachen).
/// `None` bei jeder anderen Fassung.
#[tauri::command]
#[specta::specta]
pub async fn transcript_variant_report(
    store: State<'_, Arc<MeetingStore>>,
    variant_id: String,
) -> Result<Option<TranslationReport>, String> {
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    let meta = variants::get_meta(&conn, &variant_id).map_err(variant_error)?;
    Ok(meta.and_then(|json| serde_json::from_str(&json).ok()))
}
