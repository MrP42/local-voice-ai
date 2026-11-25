//! lva#806 P1a: Buchbefehle ueber vorhandene Teilauftraege und JobControl.
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

use crate::managers::meetings::audiobook::AudiobookTempo;
use crate::managers::meetings::{job, queue::ImportQueue, store::MeetingStore};

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct LibraryTranscriptionControl {
    pub book_id: String,
    /// `running`, `pausing`, `paused`, `stopping`, `cancelled`.
    pub state: String,
    pub generations: Vec<(String, u64)>,
    pub errors: Vec<String>,
    pub has_partial: bool,
}

fn job_ids(app: &AppHandle, book_id: &str) -> Result<Vec<String>, String> {
    let book = crate::library::library_list(app.clone())?
        .into_iter()
        .find(|b| b.id == book_id)
        .ok_or("book_not_found")?;
    let audio = book.audio.ok_or("audio_missing")?;
    let mut ids = Vec::new();
    for track in crate::library::tracks_of(&audio) {
        if track.transcript_jobs.is_empty() {
            ids.extend(track.transcript_job);
        } else {
            ids.extend(track.transcript_jobs.into_iter().map(|part| part.job));
        }
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_status(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    queue: State<'_, Arc<ImportQueue>>,
    book_id: String,
) -> Result<LibraryTranscriptionControl, String> {
    let ids = job_ids(&app, &book_id)?;
    let progress = job::global().snapshots();
    let mut paused = false;
    let mut cancelled = queue.audiobook_cancel_pending(&ids);
    let mut active = false;
    let mut moving = false;
    let mut generations = Vec::new();
    let mut errors = Vec::new();
    let mut has_partial = false;
    for id in ids {
        if store.get_meeting(&id).map_err(|e| e.to_string())?
            .is_some_and(|m| m.status == "failed") {
            if let Some(code) = store.metadata_json(&id).map_err(|e| e.to_string())?
                .and_then(|m| m.get("import_error").and_then(|v| v.as_str()).map(str::to_owned)) {
                errors.push(code);
            }
        }
        has_partial = has_partial || store.has_transcript_segments(&id).map_err(|e| e.to_string())?;
        if let Some(c) = store.audiobook_control(&id).map_err(|e| e.to_string())? {
            generations.push((id.clone(), c.generation));
            paused |= c.desired_state == "paused";
            cancelled |= c.desired_state == "cancelled";
        }
        if let Some(p) = progress.iter().find(|p| p.meeting_id == id) {
            active = true;
            moving |= p.state != job::JobRunState::Paused;
        }
    }
    let state = if cancelled {
        if active {
            "stopping"
        } else {
            "cancelled"
        }
    } else if paused {
        if moving {
            "pausing"
        } else {
            "paused"
        }
    } else {
        "running"
    };
    Ok(LibraryTranscriptionControl {
        book_id,
        state: state.into(),
        generations,
        errors,
        has_partial,
    })
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_finish(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    book_id: String,
    adoption: crate::library::transcription::TranscriptAdoption,
    token: String,
) -> Result<crate::library::LibraryBook, String> {
    let _book = crate::library::lock();
    let mut book = crate::library::library_list(app.clone())?
        .into_iter().find(|b| b.id == book_id).ok_or("book_not_found")?;
    let ids = job_ids(&app, &book_id)?;
    store
        .audiobook_check_claim(&book_id, &token, &ids)
        .map_err(|e| e.to_string())?;
    crate::library::transcription::apply_transcript(&mut book, adoption, &ids)?;
    let saved = crate::library::save_locked(app, book)?;
    // Das Buch ist bereits gespeichert: ein Freigabefehler darf keinen
    // Frontend-Rollback ausloesen. Finally versucht die Freigabe erneut.
    if store.audiobook_release_claim(&saved.id, &token).is_err() {
        log::warn!("library: adoption claim release failed");
    }
    Ok(saved)
}

/// Beim Start und vor dem naechsten Entfernen alle dauerhaften Absichten nachholen.
pub(crate) fn recover_removals(app: &AppHandle) -> Result<(), String> {
    let _book = crate::library::lock();
    let root = crate::library::transcription::root(app)?;
    crate::library::transcription::remove_jobs(&root, "", &[], |id| {
        super::meetings::delete_audiobook_helper(app, id)
    })
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_remove(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    book_id: String,
    job_ids: Vec<String>,
) -> Result<(), String> {
    recover_removals(&app)?;
    let _book = crate::library::lock();
    let linked = self::job_ids(&app, &book_id)?;
    // Unbekannte IDs duerfen keine unabhaengige Besprechung entfernen.
    // Ein wiederholter Aufruf nach erfolgreichem Unlink hat keine Arbeit mehr.
    let ids: Vec<_> = job_ids.into_iter().filter(|id| linked.contains(id)).collect();
    for id in &ids {
        if job::global().snapshots().iter().any(|p| p.meeting_id == *id) {
            return Err("job_stopping".into());
        }
        store.audiobook_validate_removal(id).map_err(|e| e.to_string())?;
    }
    let root = crate::library::transcription::root(&app)?;
    crate::library::transcription::remove_jobs(&root, &book_id, &ids, |id| {
        super::meetings::delete_audiobook_helper(&app, id)
    })
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_claim(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    queue: State<'_, Arc<ImportQueue>>,
    book_id: String,
    generations: Vec<(String, u64)>,
) -> Result<String, String> {
    let _book = crate::library::lock();
    let ids = job_ids(&app, &book_id)?;
    if queue.audiobook_cancel_pending(&ids) {
        return Err("stale_audiobook_completion".into());
    }
    let mut expected: Vec<_> = generations.iter().map(|(id, _)| id.clone()).collect();
    expected.sort();
    if ids != expected {
        return Err("stale_audiobook_completion".into());
    }
    store
        .audiobook_claim(&book_id, &generations)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_release(
    store: State<'_, Arc<MeetingStore>>,
    book_id: String,
    token: String,
) -> Result<(), String> {
    let _book = crate::library::lock();
    store
        .audiobook_release_claim(&book_id, &token)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_pause(
    app: AppHandle,
    queue: State<'_, Arc<ImportQueue>>,
    book_id: String,
) -> Result<(), String> {
    let _book = crate::library::lock();
    queue.control_audiobooks(&job_ids(&app, &book_id)?, "paused", true)
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_resume(
    app: AppHandle,
    queue: State<'_, Arc<ImportQueue>>,
    book_id: String,
) -> Result<(), String> {
    let _book = crate::library::lock();
    queue.control_audiobooks(&job_ids(&app, &book_id)?, "running", true)
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_cancel(
    app: AppHandle,
    queue: State<'_, Arc<ImportQueue>>,
    book_id: String,
    keep_partial: bool,
) -> Result<(), String> {
    let _book = crate::library::lock();
    queue.control_audiobooks(&job_ids(&app, &book_id)?, "cancelled", keep_partial)
}

#[tauri::command]
#[specta::specta]
pub fn change_audiobook_tempo_setting(app: AppHandle, value: AudiobookTempo) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.audiobook_tempo = value;
    crate::settings::write_settings(&app, settings);
    if let Some(queue) = app.try_state::<Arc<ImportQueue>>() {
        queue.notify();
    }
    Ok(())
}
