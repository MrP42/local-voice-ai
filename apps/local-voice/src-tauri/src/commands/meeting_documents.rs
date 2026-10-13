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

// ---------------------------------------------------------------------------
// Gestalteter Export und Briefkoepfe (Etappe 2)
// ---------------------------------------------------------------------------

use crate::managers::letterhead::{self, Letterhead};
use crate::managers::meetings::doc_export;

fn letterhead_store(app: &AppHandle) -> Result<letterhead::Store, String> {
    let base = crate::portable::app_data_dir(app).map_err(|e| format!("letterhead_io: {e}"))?;
    Ok(letterhead::Store::new(base.join(letterhead::DIR_NAME)))
}

/// Schreibt `markdown` gestaltet (Word, PDF) oder als Text/Markdown nach `path`, mit dem
/// Briefkopf `letterhead_id` (ohne: der eingestellte). Laeuft auf einem Arbeitsthread.
fn export_with_letterhead(
    app: &AppHandle,
    path: String,
    markdown: String,
    letterhead_id: Option<String>,
) -> Result<(), String> {
    let id =
        letterhead_id.unwrap_or_else(|| crate::settings::get_settings(app).document_letterhead_id);
    let store = letterhead_store(app)?;
    let lh = store.get(&id)?;
    let logo = store.logo(&lh)?;
    doc_export::write(
        std::path::Path::new(&path),
        &markdown,
        &doc_export::Design {
            letterhead: &lh,
            logo: logo.as_ref(),
        },
    )
}

/// Ein gespeichertes Dokument (Protokoll oder Dokumenttyp) exportieren: Format nach der
/// Endung (`md`, `txt`, `docx`, `pdf`), Word und PDF mit dem eingestellten Briefkopf.
/// Quelle ist der gespeicherte Text samt Korrekturen von Hand.
#[tauri::command]
#[specta::specta]
pub async fn meetings_export_document_as(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    document_id: String,
    path: String,
) -> Result<(), String> {
    let document = store
        .get_document(&document_id)
        .map_err(|e| format!("store_failed: {e}"))?
        .ok_or_else(|| "document_not_found".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        export_with_letterhead(&app, path, document.body, None)
    })
    .await
    .map_err(|e| format!("export_failed: {e}"))?
}

/// Alle Briefkoepfe: mitgelieferte zuerst, dann eigene.
#[tauri::command]
#[specta::specta]
pub async fn letterheads_list(app: AppHandle) -> Result<Vec<Letterhead>, String> {
    Ok(letterhead_store(&app)?.list())
}

/// Eigenen Briefkopf anlegen oder aendern (mitgelieferte sind schreibgeschuetzt).
#[tauri::command]
#[specta::specta]
pub async fn letterhead_save(app: AppHandle, letterhead: Letterhead) -> Result<(), String> {
    letterhead_store(&app)?.save(&letterhead)
}

/// Eigenen Briefkopf entfernen. War er eingestellt, gilt wieder die Vorgabe.
#[tauri::command]
#[specta::specta]
pub async fn letterhead_delete(app: AppHandle, id: String) -> Result<(), String> {
    letterhead_store(&app)?.delete(&id)?;
    let mut settings = crate::settings::get_settings(&app);
    if settings.document_letterhead_id == id {
        settings.document_letterhead_id = letterhead::DEFAULT_ID.to_string();
        crate::settings::write_settings(&app, settings);
    }
    Ok(())
}

/// Logo eines eigenen Briefkopfs setzen (PNG/JPEG, hoechstens 2 MB und 4000 px) oder
/// entfernen (`path = null`).
#[tauri::command]
#[specta::specta]
pub async fn letterhead_set_logo(
    app: AppHandle,
    id: String,
    path: Option<String>,
) -> Result<Letterhead, String> {
    letterhead_store(&app)?.set_logo(&id, path.as_deref().map(std::path::Path::new))
}

/// Einstellung "Briefkopf fuer Word und PDF". Unbekannte Ids werden abgelehnt.
#[tauri::command]
#[specta::specta]
pub fn change_document_letterhead_setting(app: AppHandle, id: String) -> Result<(), String> {
    letterhead_store(&app)?.get(&id)?;
    let mut settings = crate::settings::get_settings(&app);
    settings.document_letterhead_id = id;
    crate::settings::write_settings(&app, settings);
    Ok(())
}

/// Beispiel mit einem Briefkopf speichern (Vorschau in der Einstellung).
#[tauri::command]
#[specta::specta]
pub async fn letterhead_export_sample(
    app: AppHandle,
    id: String,
    path: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        export_with_letterhead(&app, path, SAMPLE_MARKDOWN.to_string(), Some(id))
    })
    .await
    .map_err(|e| format!("export_failed: {e}"))?
}

/// Inhalt des Beispiels: alle Bausteine einmal (Titel, Kopfzeile, Liste, Aufgaben,
/// Tabelle, Hinweis).
const SAMPLE_MARKDOWN: &str = "# Management Summary: Beispielbesprechung\n\n\
**Datum:** 2026-10-13 · **Dauer:** 45:00 · **Für:** Entscheider, nicht anwesend\n\n\
## Kernergebnisse\n\n\
- Der Go-Live ist für den 1. September beschlossen.\n\
- Das Budget bleibt unverändert; Risiken liegen bei der Beschaffung.\n\n\
## Empfehlung und nächste Schritte\n\n\
- Release-Notes schreiben _(Wer: Anna, Bis: Freitag)_\n\
- Abnahmetermin mit dem Kunden vereinbaren _(Wer: Ben)_\n\n\
## Sprecher & Redeanteile\n\n\
| Sprecher | Redezeit | Anteil |\n|---|---|---|\n| Anna | 20:15 | 45,0 % |\n| Ben | 24:45 | 55,0 % |\n\n\
> **Hinweis:** Dies ist ein Beispiel mit dem gewählten Briefkopf.\n";
