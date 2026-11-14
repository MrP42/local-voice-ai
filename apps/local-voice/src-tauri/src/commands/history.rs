use crate::actions::process_transcription_output_for_model;
use crate::managers::{
    accurate_run::LiveState,
    history::{HistoryManager, PaginatedHistory},
    transcription::TranscriptionManager,
};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
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

/// Fehlercode: fuer diesen Eintrag laeuft schon eine Wiederholung.
const ERR_RETRY_BUSY: &str = "retry_busy";
/// Fehlercode: die Nachbearbeitung des genauen Laufs war verlangt und eingerichtet,
/// schlug aber fehl; der Eintrag blieb vollstaendig unveraendert.
const ERR_POST_PROCESS: &str = "accurate_post_process_failed";

static RETRYING: Mutex<BTreeSet<i64>> = Mutex::new(BTreeSet::new());

/// Je Verlaufseintrag hoechstens eine Wiederholung zur Zeit, gleich mit welchem
/// Modell. Faellt mit dem Wert (auch bei Fehler oder Abbruch des Kommandos).
#[derive(Debug)]
struct RetryClaim(i64);

impl RetryClaim {
    fn take(id: i64) -> Result<Self, String> {
        let mut running = RETRYING.lock().unwrap_or_else(|e| e.into_inner());
        if !running.insert(id) {
            return Err(ERR_RETRY_BUSY.to_string());
        }
        Ok(Self(id))
    }
}

impl Drop for RetryClaim {
    fn drop(&mut self) {
        RETRYING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

/// Was ausserhalb des `TranscriptionManager` gerade live arbeitet (Aufnahmen,
/// Besprechungsverarbeitung); Laden und Live-Transkription ergaenzt dieser selbst.
fn live_state(app: &AppHandle) -> LiveState {
    use crate::managers::audio::AudioRecordingManager;
    use crate::managers::meetings::recorder::MeetingRecorderManager;
    use tauri::Manager;
    LiveState {
        dictation_recording: app
            .try_state::<Arc<AudioRecordingManager>>()
            .is_some_and(|a| a.is_recording()),
        meeting_recording: app
            .try_state::<Arc<MeetingRecorderManager>>()
            .is_some_and(|r| r.is_recording()),
        meeting_work: !crate::managers::meetings::job::global()
            .snapshots()
            .is_empty(),
        ..LiveState::default()
    }
}

async fn retry_entry(
    app: AppHandle,
    history_manager: &Arc<HistoryManager>,
    transcription_manager: &Arc<TranscriptionManager>,
    id: i64,
    model: RetryModel,
) -> Result<(), String> {
    let _claim = RetryClaim::take(id)?;
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

    let accurate = match &model {
        RetryModel::Dictation => {
            transcription_manager.initiate_model_load();
            None
        }
        RetryModel::Explicit(model_id) => Some(model_id.clone()),
    };

    let tm = Arc::clone(transcription_manager);
    let run = accurate.clone();
    let live_app = app.clone();
    let transcription = tauri::async_runtime::spawn_blocking(move || match run {
        None => tm.transcribe(samples),
        Some(model_id) => tm.transcribe_accurate(&model_id, samples, &|| live_state(&live_app)),
    })
    .await
    .map_err(|e| format!("Transcription task panicked: {}", e))?
    .map_err(|e| e.to_string())?;

    if transcription.is_empty() {
        return Err("Recording contains no speech".to_string());
    }

    let processed = process_transcription_output_for_model(
        &app,
        &transcription,
        entry.post_process_requested,
        accurate.as_deref(),
    )
    .await;
    // Nur der genaue Lauf: der bisherige Weg (Diktatmodell) bleibt, wie er war.
    // Scheitert die Nachbearbeitung, wird nichts geschrieben, gleich ob der
    // Eintrag schon einen nachbearbeiteten Text hatte.
    if accurate.is_some() && processed.post_process_failed {
        return Err(ERR_POST_PROCESS.to_string());
    }
    history_manager
        .update_transcription_if_unchanged(
            &entry,
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
/// (`accurate_model`). Der Lauf rechnet mit einer eigenen Engine; das Diktatmodell
/// bleibt, wie es ist. Fehlercodes: `accurate_model_unset`, die Codes aus
/// `managers::accurate_run` (`accurate_model_live`, `_busy`, `_missing`, `_memory`,
/// `_load_failed`), `accurate_post_process_failed`, `retry_busy`, `entry_changed`.
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
    use super::{accurate_model_of, RetryClaim, ERR_RETRY_BUSY};
    use crate::settings::get_default_settings;

    #[test]
    fn one_entry_is_retried_by_one_run_at_a_time() {
        let id = -4711; // kein echter Eintrag, kein anderer Test nutzt ihn
        let first = RetryClaim::take(id).expect("frei");
        assert_eq!(RetryClaim::take(id).unwrap_err(), ERR_RETRY_BUSY);
        // Ein anderer Eintrag ist unabhaengig.
        drop(RetryClaim::take(id - 1).expect("anderer Eintrag"));
        drop(first);
        // Fehlerweg gibt frei.
        let failing = || -> Result<(), String> {
            let _claim = RetryClaim::take(id)?;
            Err("x".into())
        };
        assert!(failing().is_err());
        drop(RetryClaim::take(id).expect("wieder frei"));
    }

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
