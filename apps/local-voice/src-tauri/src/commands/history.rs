use crate::actions::process_transcription_output;
use crate::managers::{
    history::{HistoryManager, PaginatedHistory},
    transcription::TranscriptionManager,
};
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
#[specta::specta]
pub async fn get_history_entries(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<PaginatedHistory, String> {
    history_manager
        .get_history_entries(cursor, limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_history_entry_saved(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .toggle_saved_status(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_audio_file_path(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    file_name: String,
) -> Result<String, String> {
    let path = history_manager.get_audio_file_path(&file_name);
    path.to_str()
        .ok_or_else(|| "Invalid file path".to_string())
        .map(|s| s.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_history_entry(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .delete_entry(id)
        .await
        .map_err(|e| e.to_string())
}

/// Welches Modell eine Wiederholung aus dem Verlauf nimmt.
enum RetryModel {
    /// Das Diktatmodell (wird bei Bedarf geladen).
    Dictation,
    /// Dieses Modell, ohne das Diktatmodell zu entladen.
    Explicit(String),
}

/// Das eingerichtete genaue Modell, oder der Fehlercode `accurate_model_unset`.
fn accurate_model_of(settings: &crate::settings::AppSettings) -> Result<String, String> {
    settings
        .accurate_model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "accurate_model_unset".to_string())
}

async fn retry_entry(
    app: AppHandle,
    history_manager: &Arc<HistoryManager>,
    transcription_manager: &Arc<TranscriptionManager>,
    id: i64,
    model: RetryModel,
) -> Result<(), String> {
    let entry = history_manager
        .get_entry_by_id(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("History entry {} not found", id))?;

    let audio_path = history_manager.get_audio_file_path(&entry.file_name);
    let samples = crate::audio_toolkit::read_wav_samples(&audio_path)
        .map_err(|e| format!("Failed to load audio: {}", e))?;

    if samples.is_empty() {
        return Err("Recording has no audio samples".to_string());
    }

    if matches!(model, RetryModel::Dictation) {
        transcription_manager.initiate_model_load();
    }

    let tm = Arc::clone(transcription_manager);
    let transcription = tauri::async_runtime::spawn_blocking(move || match model {
        RetryModel::Dictation => tm.transcribe(samples),
        RetryModel::Explicit(id) => tm.transcribe_with_model(&id, samples),
    })
    .await
    .map_err(|e| format!("Transcription task panicked: {}", e))?
    .map_err(|e| e.to_string())?;

    if transcription.is_empty() {
        return Err("Recording contains no speech".to_string());
    }

    let processed =
        process_transcription_output(&app, &transcription, entry.post_process_requested).await;
    history_manager
        .update_transcription(
            id,
            transcription,
            processed.post_processed_text,
            processed.post_process_prompt,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn retry_history_entry_transcription(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    id: i64,
) -> Result<(), String> {
    retry_entry(
        app,
        &history_manager,
        &transcription_manager,
        id,
        RetryModel::Dictation,
    )
    .await
}

/// Wie [`retry_history_entry_transcription`], aber mit dem genauen Modell
/// (`accurate_model`). Das Diktatmodell bleibt geladen. Fehlercodes:
/// `accurate_model_unset`, `accurate_model_missing`, `accurate_model_memory`.
#[tauri::command]
#[specta::specta]
pub async fn retry_history_entry_with_accurate_model(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    id: i64,
) -> Result<(), String> {
    let model = accurate_model_of(&crate::settings::get_settings(&app))?;
    retry_entry(
        app,
        &history_manager,
        &transcription_manager,
        id,
        RetryModel::Explicit(model),
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn update_history_limit(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    limit: usize,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.history_limit = limit;
    crate::settings::write_settings(&app, settings);

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_recording_retention_period(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    period: String,
) -> Result<(), String> {
    use crate::settings::RecordingRetentionPeriod;

    let retention_period = match period.as_str() {
        "never" => RecordingRetentionPeriod::Never,
        "preserve_limit" => RecordingRetentionPeriod::PreserveLimit,
        "days3" => RecordingRetentionPeriod::Days3,
        "weeks2" => RecordingRetentionPeriod::Weeks2,
        "months3" => RecordingRetentionPeriod::Months3,
        _ => return Err(format!("Invalid retention period: {}", period)),
    };

    let mut settings = crate::settings::get_settings(&app);
    settings.recording_retention_period = retention_period;
    crate::settings::write_settings(&app, settings);

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::accurate_model_of;
    use crate::settings::get_default_settings;

    #[test]
    fn without_an_accurate_model_the_retry_reports_that_it_is_unset() {
        let mut settings = get_default_settings();
        assert_eq!(
            accurate_model_of(&settings).unwrap_err(),
            "accurate_model_unset"
        );
        settings.accurate_model = Some("  ".into());
        assert_eq!(
            accurate_model_of(&settings).unwrap_err(),
            "accurate_model_unset"
        );
        settings.accurate_model = Some(" whisper-large-v3 ".into());
        assert_eq!(accurate_model_of(&settings).unwrap(), "whisper-large-v3");
    }
}
