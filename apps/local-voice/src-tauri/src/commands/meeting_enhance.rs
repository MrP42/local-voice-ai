//! Duenne Command-Huelle ueber den KI-Notizen-Motor (M1, P1b; Muster
//! `commands/meetings.rs`). Die Logik liegt in
//! `managers::meetings::notes::enhance`; hier stehen nur Argumente,
//! Einstellungen, das Fortschrittsereignis und das Abbild der Fehler auf
//! Ereignis-Codes.
//!
//! Datenschutz (D9): weder Notiz- noch Transkript- noch Anweisungstext
//! gelangen ins Log; geloggt werden nur Codes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::managers::meetings::llm_call::resolve_provider_coded;
use crate::managers::meetings::notes::enhance::{
    self, check_recording_conflict, event_code, EnhanceError,
};
use crate::managers::meetings::notes::model::EnhancedNotes;
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::store::{MeetingDocument, MeetingStore};

/// Ereignis des KI-Notizen-Laufs. `code` ist einer von `no_provider`,
/// `no_model`, `memory_low`, `recording_active`, `enhance_busy`,
/// `no_transcript`, `llm_failed`, `meeting_not_finished`; die Oberflaeche
/// uebersetzt ihn (Muster `MeetingEvent::Error`).
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeetingNotesEvent {
    Progress {
        meeting_id: String,
        step: u32,
        total: u32,
    },
    Done {
        meeting_id: String,
        document_id: String,
    },
    Failed {
        meeting_id: String,
        code: String,
    },
}

fn emit_failed(app: &AppHandle, meeting_id: &str, err: &str) {
    let code = event_code(err);
    log::warn!("KI-Notizen fehlgeschlagen: {code}");
    let _ = MeetingNotesEvent::Failed {
        meeting_id: meeting_id.to_string(),
        code: code.to_string(),
    }
    .emit(app);
}

fn emit_done(app: &AppHandle, document: &MeetingDocument) {
    let _ = MeetingNotesEvent::Done {
        meeting_id: document.meeting_id.clone(),
        document_id: document.id.clone(),
    }
    .emit(app);
}

/// Laeuft eine Aufnahme, waehrend der aktive Anbieter lokal ist? Ein
/// unvollstaendiger Anbieter meldet der Motor selbst (`no_provider`).
fn recording_conflict(
    settings: &crate::settings::AppSettings,
    recording_active: bool,
) -> Result<(), EnhanceError> {
    match resolve_provider_coded(settings) {
        Ok((provider, _, _)) => {
            check_recording_conflict(crate::managers::llm::is_local(&provider), recording_active)
        }
        Err(_) => Ok(()),
    }
}

/// KI-Notizen erzeugen und dabei `MeetingNotesEvent` senden (Fortschritt,
/// Ende, Fehler). Gemeinsamer Weg fuer den Knopf und den Auto-Lauf nach dem
/// Stopp (P1f): dort ist `recording_active = false`.
pub async fn enhance_and_notify(
    app: &AppHandle,
    store: Arc<MeetingStore>,
    recording_active: bool,
    meeting_id: &str,
    template_id: Option<&str>,
) -> Result<MeetingDocument, String> {
    let settings = crate::settings::get_settings(app);
    if let Err(e) = recording_conflict(&settings, recording_active) {
        let message = e.to_string();
        emit_failed(app, meeting_id, &message);
        return Err(message);
    }
    let progress_app = app.clone();
    let progress_id = meeting_id.to_string();
    let result = enhance::enhance_meeting(
        &settings,
        store,
        meeting_id,
        template_id,
        move |step, total| {
            let _ = MeetingNotesEvent::Progress {
                meeting_id: progress_id.clone(),
                step,
                total,
            }
            .emit(&progress_app);
        },
    )
    .await;
    match &result {
        Ok(document) => emit_done(app, document),
        Err(message) => emit_failed(app, meeting_id, message),
    }
    result
}

/// Erzeugt KI-Notizen fuer eine fertige Besprechung aus Notizblock,
/// Transkript und Vorlage (`None` = Vorlage der Besprechung, sonst die
/// Standardvorlage) und legt sie als neue Version ab. Fehler tragen einen
/// Code als Praefix (`no_provider`, `enhance_busy`, ...).
#[tauri::command]
#[specta::specta]
pub async fn meeting_notes_enhance(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    meeting_id: String,
    template_id: Option<String>,
) -> Result<MeetingDocument, String> {
    let store = Arc::clone(&store);
    enhance_and_notify(
        &app,
        store,
        recorder.is_recording(),
        &meeting_id,
        template_id.as_deref(),
    )
    .await
}

/// Wendet eine Freitext-Anweisung auf eine Version der KI-Notizen an. Die
/// eigenen Eintraege des Nutzers bleiben unveraendert; das Ergebnis ist eine
/// neue Version.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notes_apply_instruction(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    document_id: String,
    instruction: String,
) -> Result<MeetingDocument, String> {
    let store = Arc::clone(&store);
    let settings = crate::settings::get_settings(&app);
    let meeting_id = store
        .get_document(&document_id)
        .ok()
        .flatten()
        .map(|d| d.meeting_id)
        .unwrap_or_default();
    if let Err(e) = recording_conflict(&settings, recorder.is_recording()) {
        let message = e.to_string();
        emit_failed(&app, &meeting_id, &message);
        return Err(message);
    }
    let result = enhance::apply_instruction(&settings, store, &document_id, &instruction).await;
    match &result {
        Ok(document) => emit_done(&app, document),
        Err(message) => emit_failed(&app, &meeting_id, message),
    }
    result
}

/// Speichert eine handbearbeitete Fassung in die bestehende Version.
/// `expected_updated_at` ist der Stempel, den die Oberflaeche geladen hat;
/// weicht er ab (zweites Fenster, neuer Lauf), scheitert der Aufruf mit
/// `stale_document`. Liefert den neuen Stempel.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notes_update_enhanced(
    store: State<'_, Arc<MeetingStore>>,
    document_id: String,
    notes: EnhancedNotes,
    expected_updated_at: i64,
) -> Result<i64, String> {
    enhance::update_enhanced(&store, &document_id, notes, expected_updated_at)
}

/// Die KI-Notizen als Markdown, zum Speichern oder Kopieren (die Ausgabe
/// selbst schreibt `meetings_export_document`).
#[tauri::command]
#[specta::specta]
pub async fn meeting_notes_markdown(
    store: State<'_, Arc<MeetingStore>>,
    document_id: String,
) -> Result<String, String> {
    enhance::markdown_for(&store, &document_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;

    #[test]
    fn the_event_serializes_with_a_kind_tag() {
        let progress = serde_json::to_value(MeetingNotesEvent::Progress {
            meeting_id: "m".into(),
            step: 2,
            total: 5,
        })
        .unwrap();
        assert_eq!(
            progress,
            serde_json::json!({"kind": "progress", "meeting_id": "m", "step": 2, "total": 5})
        );
        let done = serde_json::to_value(MeetingNotesEvent::Done {
            meeting_id: "m".into(),
            document_id: "d".into(),
        })
        .unwrap();
        assert_eq!(done["kind"], "done");
        let failed = serde_json::to_value(MeetingNotesEvent::Failed {
            meeting_id: "m".into(),
            code: "memory_low".into(),
        })
        .unwrap();
        assert_eq!(
            failed,
            serde_json::json!({"kind": "failed", "meeting_id": "m", "code": "memory_low"})
        );
    }

    #[test]
    fn a_recording_blocks_only_the_local_provider() {
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "local".into();
        settings
            .post_process_models
            .insert("local".into(), "irgendein-modell".into());
        assert_eq!(
            recording_conflict(&settings, true).unwrap_err().code,
            "recording_active"
        );
        assert!(recording_conflict(&settings, false).is_ok());

        settings.post_process_provider_id = "custom".into();
        settings
            .post_process_models
            .insert("custom".into(), "modell".into());
        assert!(
            recording_conflict(&settings, true).is_ok(),
            "entfernter Anbieter: erlaubt"
        );

        // Ohne Anbieter meldet der Motor den Fehler selbst.
        settings.post_process_provider_id = "gibt-es-nicht".into();
        assert!(recording_conflict(&settings, true).is_ok());
    }
}
