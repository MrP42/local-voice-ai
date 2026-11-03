//! Mehrere Audiodateien (Teile) je Besprechung: duenne Befehlshuelle ueber
//! `managers::meetings::parts`. Die Logik steht dort.
//!
//! Fehlercodes (die Oberflaeche uebersetzt sie, `meetingErrors.ts`):
//! `parts_too_few`, `parts_too_many`, `parts_not_ready`, `parts_no_audio`,
//! `parts_audio_missing`, `parts_mixed_sources`, `parts_has_variants`,
//! `parts_changed`, `parts_busy`, `parts_failed`, `parts_subtitle`,
//! `parts_target_invalid`, `meeting_not_found`, `import_path_invalid`.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::managers::meetings::parts::{self, PartsInfo};
use crate::managers::meetings::queue::{self, ImportQueue};
use crate::managers::meetings::store::MeetingStore;

fn store_of(app: &AppHandle) -> Result<Arc<MeetingStore>, String> {
    app.try_state::<Arc<MeetingStore>>()
        .map(|s| Arc::clone(&s))
        .ok_or_else(|| "meetings_unavailable".to_string())
}

fn queue_of(app: &AppHandle) -> Result<Arc<ImportQueue>, String> {
    app.try_state::<Arc<ImportQueue>>()
        .map(|s| Arc::clone(&s))
        .ok_or_else(|| "meetings_unavailable".to_string())
}

/// Teile einer Besprechung: zusammengefuehrte Teile, noch laufende Teile, die
/// angehaengt werden, und (fuer einen solchen Teil) sein Ziel.
#[tauri::command]
#[specta::specta]
pub async fn meetings_parts_info(app: AppHandle, meeting_id: String) -> Result<PartsInfo, String> {
    let store = store_of(&app)?;
    parts::parts_info(&store, &meeting_id).map_err(|e| format!("parts_failed: {e}"))
}

/// Fuehrt fertige Besprechungen zu einer zusammen. `keep_order = false`: nach
/// Aufnahmebeginn (bei Gleichstand nach Dateiname) geordnet, die erste bleibt
/// erhalten und nimmt die anderen auf (Titel, Projekte, Notizen bleiben; die
/// anderen gehen in den Papierkorb). Gibt die ID der Besprechung zurueck. Die
/// Sprechertrennung ueber das Ganze laeuft danach im Hintergrund.
#[tauri::command]
#[specta::specta]
pub async fn meetings_merge(
    app: AppHandle,
    meeting_ids: Vec<String>,
    keep_order: bool,
) -> Result<String, String> {
    let store = store_of(&app)?;
    let ids = if keep_order {
        meeting_ids
    } else {
        let mut meetings = Vec::with_capacity(meeting_ids.len());
        for id in &meeting_ids {
            match store.get_meeting(id).map_err(|e| format!("parts_failed: {e}"))? {
                Some(m) if m.deleted_at.is_none() => meetings.push(m),
                _ => return Err("meeting_not_found".into()),
            }
        }
        parts::chronological(&mut meetings);
        meetings.into_iter().map(|m| m.id).collect()
    };
    tauri::async_runtime::spawn_blocking(move || parts::merge_now(&app, &store, &ids))
        .await
        .map_err(|e| format!("parts_failed: {e}"))?
        .map_err(|e| e.to_string())
}

/// Haengt Audiodateien an eine Besprechung an. Jede Datei kommt als eigener
/// Auftrag in die Import-Warteschlange (Fortschritt wie jeder Import); sobald
/// alle Teile fertig sind, werden sie in dieser Reihenfolge hinten angefuegt.
/// Das Ziel darf selbst noch in der Warteschlange stehen (Import mehrerer
/// Dateien "als eine Besprechung"). Gibt die IDs der Teile zurueck.
#[tauri::command]
#[specta::specta]
pub async fn meetings_append_files(
    app: AppHandle,
    target_meeting_id: String,
    paths: Vec<String>,
    consent_confirmed: bool,
) -> Result<Vec<String>, String> {
    let store = store_of(&app)?;
    let queue = queue_of(&app)?;
    let target = match store
        .get_meeting(&target_meeting_id)
        .map_err(|e| format!("parts_failed: {e}"))?
    {
        Some(m) if m.deleted_at.is_none() => m,
        _ => return Err("meeting_not_found".into()),
    };
    if !matches!(target.source.as_str(), "import" | "live") {
        return Err("parts_target_invalid".into());
    }
    // Ein wartender Teil nimmt selbst keine Teile an: an sein Ziel anhaengen.
    let target_id = store
        .append_target_of(&target.id)
        .map_err(|e| format!("parts_failed: {e}"))?
        .unwrap_or(target.id);
    if paths.is_empty() {
        return Err("parts_too_few".into());
    }
    if paths.iter().any(|p| queue::is_subtitle(&PathBuf::from(p))) {
        return Err("parts_subtitle".into());
    }
    let consent_at = consent_confirmed.then(|| chrono::Utc::now().timestamp());
    let mut ids = Vec::with_capacity(paths.len());
    for path in paths {
        let path = PathBuf::from(path);
        let source = path
            .to_str()
            .ok_or_else(|| "import_path_invalid".to_string())?
            .to_string();
        let title = queue::title_from_path(&path);
        let meeting = queue.enqueue(&title, &source, consent_at)?;
        if let Err(e) = store.append_pending_add(&meeting.id, &target_id) {
            // Ohne Vormerkung bliebe der Teil eine eigene Besprechung: sichtbar, kein Verlust.
            log::warn!("meetings: part {} not marked for appending: {e}", meeting.id);
            return Err(format!("parts_failed: {e}"));
        }
        ids.push(meeting.id);
    }
    Ok(ids)
}

/// Ein vorgemerkter Teil wird doch eine eigene Besprechung (z. B. weil er
/// gescheitert ist und die anderen nicht warten sollen). Sind danach alle
/// uebrigen Teile fertig, wird sofort zusammengefuehrt.
#[tauri::command]
#[specta::specta]
pub async fn meetings_append_detach(app: AppHandle, meeting_id: String) -> Result<(), String> {
    let store = store_of(&app)?;
    let target = store
        .append_target_of(&meeting_id)
        .map_err(|e| format!("parts_failed: {e}"))?;
    store
        .append_pending_remove(&meeting_id)
        .map_err(|e| format!("parts_failed: {e}"))?;
    if let Some(target) = target {
        tauri::async_runtime::spawn_blocking(move || parts::after_import(&app, &store, &target))
            .await
            .map_err(|e| format!("parts_failed: {e}"))?;
    }
    Ok(())
}
