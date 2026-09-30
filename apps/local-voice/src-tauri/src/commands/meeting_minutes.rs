//! Duenne Command-Huelle ueber die Protokoll-Erzeugung (P1k, B14; Muster
//! `commands/meeting_enhance.rs`). Die Logik liegt in
//! `managers::meetings::minutes`; hier stehen Argumente, das Ereignis
//! (Fortschritt, Ende, Fehler) und die Abfragen fuer die Oberflaeche.
//!
//! Der Lauf gehoert dem Backend, nicht dem Reiter: die Oberflaeche fragt beim
//! Einblenden `meetings_minutes_state` und hoert auf `MinutesEvent`, ein
//! Reiterwechsel startet oder beendet nichts.
//!
//! Datenschutz (D9): weder Transkript- noch Protokolltext gelangen ins Log;
//! geloggt werden nur Codes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::managers::meetings::minutes::{
    self, error_code, MinutesMeta, MinutesPhase, MinutesRunState, CODE_BUSY,
};
use crate::managers::meetings::notes::classify::{self, AutoTemplateInfo};
use crate::managers::meetings::store::{MeetingDocument, MeetingStore};

/// Ereignis eines Protokoll-Laufs. `code` von `Failed` ist einer der Codes aus
/// `minutes::ALL_CODES`; die Oberflaeche uebersetzt ihn. Ein abgewiesener zweiter
/// Start (`minutes_busy`) sendet KEIN Ereignis: der laufende Lauf gehoert dem
/// ersten Start, und dessen Anzeige darf nicht gestoert werden.
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MinutesEvent {
    Progress {
        meeting_id: String,
        phase: MinutesPhase,
        done: u32,
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

/// Protokoll erzeugen und dabei `MinutesEvent` senden. Gemeinsamer Weg fuer den
/// Knopf (und spaeter jeden Automatismus).
pub async fn generate_and_notify(
    app: &AppHandle,
    store: Arc<MeetingStore>,
    meeting_id: &str,
    template_id: Option<&str>,
) -> Result<MeetingDocument, String> {
    let progress_app = app.clone();
    let progress_id = meeting_id.to_string();
    let result = minutes::generate_minutes(app, store, meeting_id, template_id, &move |p| {
        let _ = MinutesEvent::Progress {
            meeting_id: progress_id.clone(),
            phase: p.phase,
            done: p.done,
            total: p.total,
        }
        .emit(&progress_app);
    })
    .await;
    match &result {
        Ok(document) => {
            let _ = MinutesEvent::Done {
                meeting_id: meeting_id.to_string(),
                document_id: document.id.clone(),
            }
            .emit(app);
        }
        Err(message) => {
            let code = error_code(message);
            log::warn!("Protokoll fehlgeschlagen: {code}");
            if code != CODE_BUSY {
                let _ = MinutesEvent::Failed {
                    meeting_id: meeting_id.to_string(),
                    code: code.to_string(),
                }
                .emit(app);
            }
        }
    }
    result
}

/// Laeuft fuer die Besprechung gerade ein Protokoll-Lauf? Beim Einblenden des
/// Reiters abfragen (B14); danach halten `MinutesEvent`s den Stand aktuell.
#[tauri::command]
#[specta::specta]
pub async fn meetings_minutes_state(meeting_id: String) -> Result<MinutesRunState, String> {
    Ok(minutes::run_state(&meeting_id))
}

/// Stopp anfordern: `true`, wenn ein Lauf besteht. Er endet vor dem naechsten
/// Modellaufruf mit `minutes_cancelled` und schreibt nichts.
#[tauri::command]
#[specta::specta]
pub async fn meetings_minutes_cancel(meeting_id: String) -> Result<bool, String> {
    Ok(minutes::request_cancel(&meeting_id))
}

/// Mit welcher Vorlage das jüngste Protokoll entstand und ob etwas fehlt.
#[tauri::command]
#[specta::specta]
pub async fn meetings_minutes_meta(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Option<MinutesMeta>, String> {
    Ok(minutes::latest_meta(&store, &meeting_id))
}

/// Die zuletzt automatisch gewählte Vorlage der Besprechung ("Automatisch:
/// Kundengespräch"); `None`, solange noch nie nach Inhalt gewählt wurde.
#[tauri::command]
#[specta::specta]
pub async fn meetings_get_auto_template(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Option<AutoTemplateInfo>, String> {
    Ok(classify::stored_choice(&store, &meeting_id))
}
