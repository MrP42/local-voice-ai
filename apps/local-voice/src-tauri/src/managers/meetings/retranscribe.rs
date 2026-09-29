//! M9: re-running the transcription of a meeting that already has audio.
//!
//! The point is model choice: a meeting first transcribed with the streaming
//! dictation model can be redone with a batch model (or the other way round)
//! without re-importing the file. The audio the app kept is the input — so
//! this only works while the recording still exists on disk (retention
//! policy, see `retention.rs`); once purged, the transcript is all there is.
//!
//! Everything except the audio source is the import pipeline verbatim
//! (`import::transcribe_and_store`), so a re-transcribed meeting is
//! byte-for-byte the same shape as a freshly imported one.

use std::path::Path;
use std::sync::Arc;

use log::{error, info};
use tauri_specta::Event;

use super::import::{read_wav_i16_mono_16k, run_speaker_step, transcribe_and_store};
use super::recorder::MeetingEvent;
use super::store::{MeetingStatus, MeetingStore};
use crate::managers::transcription::TranscriptionManager;

/// `StoredSegment::channel` values, mirroring `store.rs`.
const CHANNEL_MIC: u8 = 0;
const CHANNEL_SYSTEM: u8 = 1;
const CHANNEL_MIXED: u8 = 2;

/// Re-transcribes `meeting_id` from its stored audio.
///
/// `model_id` overrides the model for this run only — `None` falls back to
/// the configured meeting model (which itself falls back to the dictation
/// model). The dictation model is restored afterwards either way, exactly as
/// the import and live-recording paths do.
pub async fn retranscribe_meeting(
    app: &tauri::AppHandle,
    store: Arc<MeetingStore>,
    tm: Arc<TranscriptionManager>,
    meeting_id: String,
    model_id: Option<String>,
) -> Result<(), String> {
    let meeting = store
        .get_meeting(&meeting_id)
        .map_err(|e| format!("meeting_lookup_failed: {e}"))?
        .ok_or_else(|| "meeting_not_found".to_string())?;

    // Subtitle imports carry no audio at all — there is nothing to redo, and
    // silently clearing their segments would destroy the only copy.
    let audio: Vec<(String, u8)> = match (&meeting.mic_audio_path, &meeting.system_audio_path) {
        (Some(mic), Some(system)) => {
            vec![(mic.clone(), CHANNEL_MIC), (system.clone(), CHANNEL_SYSTEM)]
        }
        // A single track: the import path stores it as `mic_audio_path` and
        // labels its segments "mixed", so keep that labelling for imports.
        (Some(single), None) => {
            let channel = if meeting.source == "import" {
                CHANNEL_MIXED
            } else {
                CHANNEL_MIC
            };
            vec![(single.clone(), channel)]
        }
        (None, Some(system)) => vec![(system.clone(), CHANNEL_SYSTEM)],
        (None, None) => return Err("no_audio".to_string()),
    };

    for (path, _) in &audio {
        if !Path::new(path).exists() {
            return Err("audio_missing".to_string());
        }
    }

    let target = match model_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => TranscriptionManager::meeting_model_target(&crate::settings::get_settings(app)),
    };

    store
        .set_status(&meeting_id, MeetingStatus::Processing)
        .map_err(|e| format!("status_processing_failed: {e}"))?;
    emit_state(app, &meeting_id, "processing");

    let app_owned = app.clone();
    let blocking_store = Arc::clone(&store);
    let blocking_id = meeting_id.clone();
    let join = tauri::async_runtime::spawn_blocking(move || {
        run_retranscribe(
            &app_owned,
            &blocking_store,
            &tm,
            &blocking_id,
            &audio,
            &target,
        )
    })
    .await;

    let outcome = match join {
        Ok(result) => result,
        Err(join_err) => Err(format!("meetings_retranscribe panicked: {join_err}")),
    };

    match outcome {
        Ok(()) => {
            store
                .set_status(&meeting_id, MeetingStatus::Ready)
                .map_err(|e| format!("status_ready_failed: {e}"))?;
            emit_state(app, &meeting_id, "ready");
            info!("meetings: retranscribe ready ({meeting_id})");
            Ok(())
        }
        Err(e) => {
            error!("meetings: retranscribe failed ({meeting_id}): {e}");
            let _ = store.set_status(&meeting_id, MeetingStatus::Failed);
            emit_state(app, &meeting_id, "failed");
            emit_error(app, &meeting_id, "retranscribe_failed");
            Err(e)
        }
    }
}

/// The blocking body. Clears the old segments only once the first audio file
/// has actually been read: a meeting whose WAV turns out to be unreadable
/// keeps the transcript it had.
fn run_retranscribe(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    meeting_id: &str,
    audio: &[(String, u8)],
    target: &str,
) -> Result<(), String> {
    tm.initiate_model_load_target(target);

    let mut tracks: Vec<(Vec<i16>, u8)> = Vec::with_capacity(audio.len());
    for (path, channel) in audio {
        tracks.push((read_wav_i16_mono_16k(Path::new(path))?, *channel));
    }

    let result = (move || -> Result<(), String> {
        store
            .clear_segments(meeting_id)
            .map_err(|e| format!("clear_segments_failed: {e}"))?;
        let _ = (MeetingEvent::Reset {
            meeting_id: meeting_id.to_string(),
        })
        .emit(app);

        let mut next_index = 0u32;
        for (samples, channel) in &tracks {
            next_index =
                transcribe_and_store(app, store, tm, meeting_id, samples, *channel, next_index)?;
        }
        // M3-P3b: Sprecher. Die Turns stehen in `speaker_hints_json` und ueber-
        // stehen `clear_segments` (samt Namen in `speakers`): die neuen Segmente
        // bekommen dieselben Sprecher, nur fehlende Turns werden neu berechnet.
        drop(tracks);
        run_speaker_step(app, store, meeting_id);
        Ok(())
    })();

    // Restore the dictation model, win or lose (mirrors import.rs).
    let dictation_model = crate::settings::get_settings(app).selected_model;
    tm.initiate_model_load_target(&dictation_model);

    result
}

fn emit_state(app: &tauri::AppHandle, meeting_id: &str, status: &str) {
    let _ = (MeetingEvent::State {
        meeting_id: meeting_id.to_string(),
        status: status.to_string(),
        paused: false,
    })
    .emit(app);
}

fn emit_error(app: &tauri::AppHandle, meeting_id: &str, message: &str) {
    let _ = (MeetingEvent::Error {
        meeting_id: meeting_id.to_string(),
        message: message.to_string(),
    })
    .emit(app);
}
