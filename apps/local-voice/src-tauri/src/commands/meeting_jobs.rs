//! P8a (Issue #59): Duenne Command-Huelle ueber das Auftragsverzeichnis
//! (`managers::meetings::job`): Fortschritt abfragen, pausieren, fortsetzen,
//! stoppen; dazu "Fortsetzen" einer gestoppten Verarbeitung. Die Logik steht
//! im Manager, hier nur Argumente und Fehlercodes.
//!
//! Fehlercodes (die Oberflaeche uebersetzt sie): `no_job` (fuer die
//! Besprechung laeuft nichts), `not_pausable` (die Phase laesst sich nicht
//! anhalten), `job_stopping`, `job_busy`, `not_cancelled`, `audio_missing`.

use std::sync::Arc;

use tauri::State;

use crate::managers::meetings::job::{self, JobProgress};
use crate::managers::meetings::queue::ImportQueue;
use crate::managers::meetings::recorder::MeetingRecorderManager;

/// Stand aller laufenden Verarbeitungen. Die Oberflaeche fragt beim Oeffnen
/// (Liste, Detail, Notizen, Protokoll) und haelt sich danach an
/// `MeetingEvent::Progress` / `JobEnded`; so geht der Laufzustand beim
/// Reiterwechsel nicht verloren.
#[tauri::command]
#[specta::specta]
pub async fn meetings_progress_list() -> Result<Vec<JobProgress>, String> {
    Ok(job::global().snapshots())
}

/// Haelt die Verarbeitung am naechsten Block an (der laufende wird fertig).
#[tauri::command]
#[specta::specta]
pub async fn meetings_job_pause(meeting_id: String) -> Result<(), String> {
    job::global().pause(&meeting_id).map_err(|e| e.to_string())
}

/// Setzt eine pausierte (oder eine noch nicht wirksame) Pause fort.
#[tauri::command]
#[specta::specta]
pub async fn meetings_job_resume(meeting_id: String) -> Result<(), String> {
    job::global().resume(&meeting_id).map_err(|e| e.to_string())
}

/// Stoppt die Verarbeitung: am naechsten Block, auch aus der Pause. Bereits
/// transkribierte Segmente bleiben; mehrfaches Stoppen ist harmlos.
#[tauri::command]
#[specta::specta]
pub async fn meetings_job_stop(meeting_id: String) -> Result<(), String> {
    job::global().stop(&meeting_id).map_err(|e| e.to_string())
}

/// "Fortsetzen" einer gestoppten Verarbeitung (Status `cancelled`): holt den
/// Rest nach (wie die Wiederherstellung nach einem Absturz). U7: eine aus der
/// Warteschlange genommene Datei, die noch kein Audio hat, wird dagegen wieder
/// hinten eingereiht.
#[tauri::command]
#[specta::specta]
pub async fn meetings_continue(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    queue: State<'_, Arc<ImportQueue>>,
    meeting_id: String,
) -> Result<(), String> {
    if queue.requeue(&meeting_id)? {
        return Ok(());
    }
    let recorder = Arc::clone(&recorder);
    tauri::async_runtime::spawn_blocking(move || recorder.continue_processing(&meeting_id))
        .await
        .map_err(|e| format!("meetings_continue panicked: {e}"))?
}
