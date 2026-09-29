//! Thin command shell over `MeetingRecorderManager` and `MeetingStore`
//! (pattern: `commands/history.rs`). No logic beyond argument shuffling and
//! error mapping lives here.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;

use crate::managers::meetings::import::import_media_file;
use crate::managers::meetings::minutes::{generate_minutes, latest_minutes_file};
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::retention::delete_audio_files;
use crate::managers::meetings::retranscribe::retranscribe_meeting;
use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::meetings::store::{Meeting, MeetingDocument, MeetingStore, StoredSegment};
use crate::managers::transcription::TranscriptionManager;

/// Starting touches audio hardware and can block for seconds (loopback
/// start-up), hence `spawn_blocking` rather than running on the command task.
#[tauri::command]
#[specta::specta]
pub async fn meetings_start(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    title: String,
    consent_confirmed: bool,
    capture_system: bool,
) -> Result<Meeting, String> {
    let recorder = Arc::clone(&recorder);
    tauri::async_runtime::spawn_blocking(move || {
        recorder.start(title, consent_confirmed, capture_system)
    })
    .await
    .map_err(|e| format!("meetings_start panicked: {e}"))?
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_pause(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
) -> Result<(), String> {
    recorder.pause()
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_resume(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
) -> Result<(), String> {
    recorder.resume()
}

/// Stopping waits for the tail chunks to finish transcribing, so it must not
/// occupy the async runtime's worker.
#[tauri::command]
#[specta::specta]
pub async fn meetings_stop(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
) -> Result<String, String> {
    let recorder = Arc::clone(&recorder);
    tauri::async_runtime::spawn_blocking(move || recorder.stop())
        .await
        .map_err(|e| format!("meetings_stop panicked: {e}"))?
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_is_recording(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
) -> Result<bool, String> {
    Ok(recorder.is_recording())
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_list(
    store: State<'_, Arc<MeetingStore>>,
    offset: u32,
    limit: u32,
) -> Result<Vec<Meeting>, String> {
    store
        .list_meetings(offset, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_get_segments(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<StoredSegment>, String> {
    store.get_segments(&meeting_id).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_update_segment(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    segment_index: u32,
    text: String,
) -> Result<(), String> {
    store
        .update_segment_text(&meeting_id, segment_index, &text)
        .map_err(|e| e.to_string())?;
    indexer::submit(&app, IndexJob::Meeting(meeting_id)); // M4-P4b
    Ok(())
}

/// Renames a meeting. The title is free text and deliberately independent of
/// the file an imported meeting came from (that is kept in `source_path`).
#[tauri::command]
#[specta::specta]
pub async fn meetings_rename(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    title: String,
) -> Result<(), String> {
    store
        .set_title(&meeting_id, &title)
        .map_err(|e| e.to_string())?;
    indexer::submit(&app, IndexJob::Meeting(meeting_id)); // M4-P4b
    Ok(())
}

/// Re-runs the transcription of a finished meeting from its stored audio,
/// optionally with a different model. Discards the old segments — see
/// `retranscribe_meeting`.
#[tauri::command]
#[specta::specta]
pub async fn meetings_retranscribe(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    transcription: State<'_, Arc<TranscriptionManager>>,
    meeting_id: String,
    model_id: Option<String>,
) -> Result<(), String> {
    let store = Arc::clone(&store);
    let transcription = Arc::clone(&transcription);
    retranscribe_meeting(&app, store, transcription, meeting_id, model_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_get_documents(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<MeetingDocument>, String> {
    store.get_documents(&meeting_id).map_err(|e| e.to_string())
}

/// Soft-deletes the meeting, then hard-deletes its audio files from disk
/// (Spec A2 — a tombstoned meeting must never leave an orphaned WAV behind).
#[tauri::command]
#[specta::specta]
pub async fn meetings_delete(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<(), String> {
    let paths = store
        .soft_delete_meeting(&meeting_id)
        .map_err(|e| e.to_string())?;
    indexer::submit(&app, IndexJob::Deleted(meeting_id)); // M4-P4b
    delete_audio_files(&paths);
    Ok(())
}

/// Generates the standardized minutes for a finished meeting and stores them
/// as a new document version. The meeting status stays untouched — a failed
/// generation leaves a 'ready' meeting 'ready' and only returns the error.
#[tauri::command]
#[specta::specta]
pub async fn meetings_generate_minutes(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<MeetingDocument, String> {
    let store = Arc::clone(&store);
    generate_minutes(&app, store, &meeting_id).await
}

/// Where this meeting's minutes were filed as Markdown, if the file is there.
/// The database holds the authoritative copy; this is the convenience copy the
/// generator drops next to the recording so it can be opened without the app.
#[tauri::command]
#[specta::specta]
pub async fn meetings_minutes_file(
    app: tauri::AppHandle,
    meeting_id: String,
) -> Result<Option<String>, String> {
    let path = latest_minutes_file(&app, &meeting_id).map_err(|e| e.to_string())?;
    Ok(path.map(|p| p.to_string_lossy().into_owned()))
}

/// Writes a document the user assembled in the app to a path they picked in
/// the system save dialog — as Markdown, plain text or Word, chosen by the
/// file extension.
///
/// This deliberately does NOT go through the fs plugin from the frontend:
/// its capability scope is limited to `$APPDATA`, so saving into Documents —
/// what the save dialog offers — failed with "not allowed by ACL". The path
/// comes from the user's own choice in a system dialog; re-checking it
/// against an allowlist protects nobody. Writing here also makes Word export
/// possible at all, since a .docx is a ZIP archive rather than text.
#[tauri::command]
#[specta::specta]
pub async fn meetings_export_document(path: String, body: String) -> Result<(), String> {
    let target = std::path::PathBuf::from(&path);
    crate::managers::meetings::export::write_document(&target, &body)
}

/// Imports a local audio/video file or a VTT/SRT subtitle file as a new
/// meeting. Audio/video decoding and transcription can take a while, hence
/// this stays `async` end to end rather than blocking the command task
/// (`import_media_file` itself moves the heavy work to `spawn_blocking`).
#[tauri::command]
#[specta::specta]
pub async fn meetings_import_file(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    transcription: State<'_, Arc<TranscriptionManager>>,
    path: String,
    consent_confirmed: bool,
) -> Result<String, String> {
    let store = Arc::clone(&store);
    let transcription = Arc::clone(&transcription);
    import_media_file(
        &app,
        store,
        transcription,
        PathBuf::from(path),
        consent_confirmed,
    )
    .await
}
