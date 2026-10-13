//! Duenne Command-Huelle ueber die Dokumenttypen (Entwurf 2026-10-13, Etappe 1; Muster
//! `commands/meeting_minutes.rs`). Die Logik liegt in `managers::meetings::doc_types`
//! (Katalog) und im Protokoll-Motor `managers::meetings::minutes`.
//!
//! Eigenes Ereignis `DocumentEvent` statt `MinutesEvent`: auf `MinutesEvent::Done` haengen
//! Workflows ("nach dem Protokoll", `workflows/hub.rs`), die ein Management Summary nicht
//! ausloesen darf. Zustand und Stopp teilen sich Dokument und Protokoll
//! (`meetings_minutes_state`, `meetings_minutes_cancel`): eine Sperre je Besprechung.
//!
//! Datenschutz (D9): weder Transkript- noch Dokumenttext gelangen ins Log.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use super::big_stack;
use crate::managers::meetings::basis::DocBasis;
use crate::managers::meetings::doc_types::{self, DocEngine, DocTypeInfo};
use crate::managers::meetings::job::{self, JobPhase};
use crate::managers::meetings::minutes::{self, error_code, MinutesMeta, MinutesPhase, CODE_BUSY};
use crate::managers::meetings::store::{MeetingDocument, MeetingStore};

/// Code, wenn der Schluessel keinen erzeugbaren Dokumenttyp nennt (unbekannt, das
/// Protokoll oder der Follow-up-Dialog).
pub const CODE_UNKNOWN_TYPE: &str = "doc_type_unknown";

/// Ereignis eines Dokumentlaufs; `doc_type` ist der Schluessel des Katalogs.
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentEvent {
    Progress {
        meeting_id: String,
        doc_type: String,
        phase: MinutesPhase,
        done: u32,
        total: u32,
    },
    Done {
        meeting_id: String,
        doc_type: String,
        document_id: String,
    },
    Failed {
        meeting_id: String,
        doc_type: String,
        code: String,
    },
}

/// Der Katalog fuer die Oberflaeche (Reihenfolge = Anzeige, Protokoll zuerst).
#[tauri::command]
#[specta::specta]
pub async fn meetings_document_types() -> Result<Vec<DocTypeInfo>, String> {
    Ok(doc_types::infos())
}

/// Anzeigedaten des juengsten Dokuments eines Typs (Unvollstaendigkeit, Luecken).
#[tauri::command]
#[specta::specta]
pub async fn meetings_document_meta(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    doc_type: String,
) -> Result<Option<MinutesMeta>, String> {
    Ok(minutes::latest_meta_of_kind(
        &store,
        &meeting_id,
        &doc_types::doc_kind(&doc_type),
    ))
}

/// Dokumenttyp erzeugen (neue Version), mit `DocumentEvent`. Fehler sind Codes wie beim
/// Protokoll (`minutes_busy`, `no_provider`, ...) oder `doc_type_unknown`.
#[tauri::command]
#[specta::specta]
pub async fn meetings_generate_document(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    doc_type: String,
    basis: Option<DocBasis>,
) -> Result<MeetingDocument, String> {
    let store = Arc::clone(&store);
    let basis = basis.unwrap_or_default();
    let (job_app, job_meeting, job_type) = (app.clone(), meeting_id.clone(), doc_type.clone());
    let result = big_stack::run_result("meeting-document", move || async move {
        generate_and_notify(&job_app, store, &job_meeting, &job_type, &basis).await
    })
    .await;
    if matches!(&result, Err(code) if code == big_stack::CODE_THREAD) {
        let _ = DocumentEvent::Failed {
            meeting_id,
            doc_type,
            code: error_code(big_stack::CODE_THREAD).to_string(),
        }
        .emit(&app);
    }
    result
}

/// Der Lauf: Auftrag im Verzeichnis der Verarbeitungen (Fortschritt und Stopp im
/// Statusbereich der Besprechung, wie das Protokoll), dann der Motor.
async fn generate_and_notify(
    app: &AppHandle,
    store: Arc<MeetingStore>,
    meeting_id: &str,
    key: &str,
    basis: &DocBasis,
) -> Result<MeetingDocument, String> {
    let Some(doc_type) = doc_types::by_key(key).filter(|t| t.engine == DocEngine::Document) else {
        return Err(CODE_UNKNOWN_TYPE.to_string());
    };
    let job = match job::global().try_start(meeting_id, job::app_emit(app)) {
        Ok(job) => job,
        Err(e) => {
            let code = if job::global().phase_of(meeting_id) == Some(JobPhase::Minutes) {
                CODE_BUSY.to_string()
            } else {
                e.to_string()
            };
            return Err(code);
        }
    };
    let handle = Arc::clone(job.handle());
    handle.begin_phase_ex(JobPhase::Minutes, 0, false);
    let cancel_id = meeting_id.to_string();
    handle.set_on_stop(Box::new(move || {
        minutes::request_cancel(&cancel_id);
    }));
    let settings = crate::settings::get_settings(app);
    let (progress_app, progress_id, progress_key) =
        (app.clone(), meeting_id.to_string(), key.to_string());
    let bridge = Arc::clone(&handle);
    let last_phase = Mutex::new(None::<(MinutesPhase, u32)>);
    let result = minutes::generate_document_with_settings(
        &settings,
        store,
        meeting_id,
        doc_type,
        basis,
        &move |p| {
            let _ = DocumentEvent::Progress {
                meeting_id: progress_id.clone(),
                doc_type: progress_key.clone(),
                phase: p.phase,
                done: p.done,
                total: p.total,
            }
            .emit(&progress_app);
            let mut last = last_phase.lock().unwrap_or_else(|e| e.into_inner());
            if *last != Some((p.phase, p.total)) {
                bridge.begin_phase_ex(JobPhase::Minutes, u64::from(p.total), false);
                *last = Some((p.phase, p.total));
            }
            bridge.advance(u64::from(p.done));
        },
    )
    .await;
    match &result {
        Ok(document) => {
            let _ = DocumentEvent::Done {
                meeting_id: meeting_id.to_string(),
                doc_type: key.to_string(),
                document_id: document.id.clone(),
            }
            .emit(app);
        }
        Err(message) => {
            let code = error_code(message);
            log::warn!("Dokument {key} fehlgeschlagen: {code}");
            if code != CODE_BUSY {
                let _ = DocumentEvent::Failed {
                    meeting_id: meeting_id.to_string(),
                    doc_type: key.to_string(),
                    code: code.to_string(),
                }
                .emit(app);
            }
        }
    }
    result
}
