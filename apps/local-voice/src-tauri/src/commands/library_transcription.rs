//! lva#806 P1a: Buchbefehle ueber vorhandene Teilauftraege und JobControl.
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::managers::meetings::{job, queue::ImportQueue, store::MeetingStore};
use crate::managers::meetings::audiobook::AudiobookTempo;

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct LibraryTranscriptionControl {
    pub book_id: String,
    /// `running`, `pausing`, `paused`, `stopping`, `cancelled`.
    pub state: String,
}

fn job_ids(app: &AppHandle, book_id: &str) -> Result<Vec<String>, String> {
    let book = crate::library::library_list(app.clone())?.into_iter()
        .find(|b| b.id == book_id).ok_or("book_not_found")?;
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
pub fn library_transcription_status(app: AppHandle, store: State<'_, Arc<MeetingStore>>, book_id: String) -> Result<LibraryTranscriptionControl, String> {
    let ids = job_ids(&app, &book_id)?;
    let progress = job::global().snapshots();
    let mut paused = false;
    let mut cancelled = false;
    let mut active = false;
    let mut moving = false;
    for id in ids {
        let ready = store.get_meeting(&id).map_err(|e| e.to_string())?
            .is_some_and(|m| m.status == "ready");
        if ready { continue; }
        if let Some(c) = store.audiobook_control(&id).map_err(|e| e.to_string())? {
            paused |= c.desired_state == "paused";
            cancelled |= c.desired_state == "cancelled";
        }
        if let Some(p) = progress.iter().find(|p| p.meeting_id == id) {
            active = true;
            moving |= p.state != job::JobRunState::Paused;
        }
    }
    let state = if cancelled { if active { "stopping" } else { "cancelled" } }
        else if paused { if moving { "pausing" } else { "paused" } }
        else { "running" };
    Ok(LibraryTranscriptionControl { book_id, state: state.into() })
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_pause(app: AppHandle, queue: State<'_, Arc<ImportQueue>>, book_id: String) -> Result<(), String> {
    queue.control_audiobooks(&job_ids(&app, &book_id)?, "paused", true)
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_resume(app: AppHandle, queue: State<'_, Arc<ImportQueue>>, book_id: String) -> Result<(), String> {
    queue.control_audiobooks(&job_ids(&app, &book_id)?, "running", true)
}

#[tauri::command]
#[specta::specta]
pub fn library_transcription_cancel(app: AppHandle, queue: State<'_, Arc<ImportQueue>>, book_id: String, keep_partial: bool) -> Result<(), String> {
    queue.control_audiobooks(&job_ids(&app, &book_id)?, "cancelled", keep_partial)
}

#[tauri::command]
#[specta::specta]
pub fn change_audiobook_tempo_setting(app: AppHandle, value: AudiobookTempo) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.audiobook_tempo = value;
    crate::settings::write_settings(&app, settings);
    if let Some(queue) = app.try_state::<Arc<ImportQueue>>() { queue.notify(); }
    Ok(())
}
