//! U7 (Issue #64): Duenne Befehlshuelle ueber die Import-Warteschlange
//! (`managers::meetings::queue`), das Bearbeiten der Metadaten einer
//! Besprechung (`managers::meetings::metadata`) und die Einstellung
//! "Gleichzeitige Transkriptionen". Die Logik steht in den Managern.
//!
//! Fehlercodes (die Oberflaeche uebersetzt sie): `not_in_queue`, `not_queued`
//! sowie die der Metadaten (`meeting_not_found`, `title_empty`,
//! `title_too_long`, `description_too_long`, `date_invalid`, `person_not_found`,
//! `folder_not_found`).

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::managers::meetings::metadata::MetadataEdit;
use crate::managers::meetings::queue::{ImportQueue, QueueSnapshot, RemoveOutcome};
use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::meetings::store::{Meeting, MeetingStore};

/// Der Stand der Warteschlange (Hydrierung der Oberflaeche; danach halten ihn
/// die `ImportQueueEvent` aktuell).
#[tauri::command]
#[specta::specta]
pub async fn meetings_queue_list(
    queue: State<'_, Arc<ImportQueue>>,
) -> Result<QueueSnapshot, String> {
    Ok(queue.snapshot())
}

/// Nimmt eine WARTENDE Datei aus der Warteschlange (Besprechung `cancelled`,
/// "Fortsetzen" reiht sie wieder ein) oder stoppt die LAUFENDE (wie der
/// Stopp-Knopf). `not_in_queue`, wenn sie weder wartet noch laeuft.
#[tauri::command]
#[specta::specta]
pub async fn meetings_queue_remove(
    queue: State<'_, Arc<ImportQueue>>,
    meeting_id: String,
) -> Result<(), String> {
    match queue.remove(&meeting_id)? {
        RemoveOutcome::Cancelled | RemoveOutcome::Stopping => Ok(()),
        RemoveOutcome::NotInQueue => Err("not_in_queue".to_string()),
    }
}

/// Zieht eine wartende Datei an die erste Stelle der wartenden (`not_queued`,
/// wenn sie nicht wartet).
#[tauri::command]
#[specta::specta]
pub async fn meetings_queue_to_front(
    queue: State<'_, Arc<ImportQueue>>,
    meeting_id: String,
) -> Result<(), String> {
    if queue.move_to_front(&meeting_id)? {
        Ok(())
    } else {
        Err("not_queued".to_string())
    }
}

/// Bearbeitet Titel, Beschreibung, Datum, Teilnehmende und Projekte in einem
/// Schritt (alles oder nichts) und liefert die aktuelle Besprechung. Der
/// Such-Index wird neu angestossen: die Beschreibung ist durchsuchbar.
#[tauri::command]
#[specta::specta]
pub async fn meetings_update_metadata(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    edit: MetadataEdit,
) -> Result<Meeting, String> {
    let store = Arc::clone(&store);
    let id = meeting_id.clone();
    let meeting = tauri::async_runtime::spawn_blocking(move || store.update_metadata(&id, &edit))
        .await
        .map_err(|e| format!("meetings_update_metadata panicked: {e}"))?
        .map_err(|e| e.to_string())?;
    indexer::submit(&app, IndexJob::Meeting(meeting_id)); // M4-P4b
    Ok(meeting)
}

/// Einstellung `meeting_import_parallel`: 1, 2 oder 3 gleichzeitige
/// Transkriptionen (Werte ausserhalb zaehlen als der naechste gueltige). Wirkt
/// ab der naechsten Datei; laufende Laeufe werden nie abgebrochen. Mehr als 1
/// nur, solange Arbeitsspeicher (und Grafikspeicher) fuer die weitere Engine
/// reichen, sonst wartet die Datei.
#[tauri::command]
#[specta::specta]
pub fn change_meeting_import_parallel_setting(app: AppHandle, value: u32) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.meeting_import_parallel = crate::managers::meetings::queue::clamp_limit(value) as u32;
    crate::settings::write_settings(&app, settings);
    // Die Einstellung gilt auch ohne Warteschlange (Besprechungen nicht verfuegbar).
    if let Some(queue) = app.try_state::<Arc<ImportQueue>>() {
        queue.notify();
    }
    Ok(())
}
