//! G3 (#70, U9): duenne Command-Huelle ueber das Projekt-Protokoll (Muster
//! `commands/meeting_minutes.rs`). Die Logik liegt in
//! `managers::meetings::minutes::project`, die Ablage in
//! `managers::meetings::project_minutes_store`; hier stehen Argumente, das
//! Ereignis (Ende, Fehler) und die Abfragen fuer die Oberflaeche.
//!
//! Der Lauf gehoert dem Backend, nicht dem Reiter: er ist zugleich ein Auftrag im
//! Verzeichnis der Verarbeitungen (`job.rs`, Phase Protokoll, Schluessel
//! `project-minutes:<projekt>`). Darueber laufen Fortschritt mit Laufzeit und
//! Restdauer (`MeetingEvent::Progress`, `meetings_progress_list`) und Stopp
//! (`meetings_job_stop`) genau wie bei der Einzelverarbeitung; die Oberflaeche
//! fragt beim Einblenden `project_minutes_state`, ein Reiterwechsel startet oder
//! beendet nichts. Pausieren gibt es wie beim Einzelprotokoll nicht: geschrieben
//! wird erst am Ende.
//!
//! Datenschutz (D9): weder Transkript- noch Protokolltext gelangen ins Log;
//! geloggt werden nur Codes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::managers::meetings::job::{self, JobPhase};
use crate::managers::meetings::minutes::project::{self, ProjectCandidate, Request};
use crate::managers::meetings::minutes::{MinutesPhase, MinutesRunState, CODE_BUSY};
use crate::managers::meetings::project_minutes_store::{
    ProjectKind, ProjectMinutes, ProjectMinutesSummary,
};
use crate::managers::meetings::store::MeetingStore;

/// Ende eines Projekt-Protokoll-Laufs. `code` von `Failed` ist einer der Codes aus
/// `minutes::ALL_CODES` oder `project::EXTRA_CODES`; die Oberflaeche uebersetzt ihn.
/// Ein abgewiesener zweiter Start (`minutes_busy`) sendet KEIN Ereignis: der laufende
/// Lauf gehoert dem ersten Start. Der Fortschritt kommt als `MeetingEvent::Progress`
/// unter dem Schluessel `project-minutes:<projekt>`.
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectMinutesEvent {
    Done {
        folder_id: String,
        minutes_id: String,
    },
    Failed {
        folder_id: String,
        code: String,
        /// Kurzer Grund (z. B. die ID der abgewiesenen Aufnahme); nie Inhalt.
        detail: String,
    },
}

/// Welche Aufnahmen eines Projekts in ein Projekt-Protokoll eingehen koennen, mit
/// dem Grund bei den anderen (`meeting_not_finished`, `no_transcript`,
/// `empty_entry`). Chronologisch.
#[tauri::command]
#[specta::specta]
pub async fn project_minutes_candidates(
    store: State<'_, Arc<MeetingStore>>,
    folder_id: String,
) -> Result<Vec<ProjectCandidate>, String> {
    project::candidates(&store, &folder_id).map_err(String::from)
}

/// Die Projekt-Protokolle eines Projekts, das juengste zuerst.
#[tauri::command]
#[specta::specta]
pub async fn project_minutes_list(
    store: State<'_, Arc<MeetingStore>>,
    folder_id: String,
) -> Result<Vec<ProjectMinutesSummary>, String> {
    store
        .project_minutes_list(&folder_id)
        .map_err(|e| e.to_string())
}

/// Ein Projekt-Protokoll mit Abschnitten, Belegen und Herkunft.
#[tauri::command]
#[specta::specta]
pub async fn project_minutes_get(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<Option<ProjectMinutes>, String> {
    store.project_minutes_get(&id).map_err(|e| e.to_string())
}

/// Loescht ein Projekt-Protokoll (weich). Die Quellaufnahmen bleiben.
#[tauri::command]
#[specta::specta]
pub async fn project_minutes_delete(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<bool, String> {
    store.project_minutes_delete(&id).map_err(|e| e.to_string())
}

/// Laeuft fuer das Projekt gerade ein Lauf? Beim Einblenden abfragen; danach
/// halten `MeetingEvent::Progress` und `ProjectMinutesEvent` den Stand aktuell.
#[tauri::command]
#[specta::specta]
pub async fn project_minutes_state(folder_id: String) -> Result<MinutesRunState, String> {
    Ok(project::run_state(&folder_id))
}

/// Stopp anfordern: `true`, wenn ein Lauf besteht. Er endet vor dem naechsten
/// Modellaufruf mit `minutes_cancelled` und schreibt nichts.
#[tauri::command]
#[specta::specta]
pub async fn project_minutes_cancel(folder_id: String) -> Result<bool, String> {
    Ok(project::request_cancel(&folder_id))
}

/// Projekt-Protokoll erzeugen: die gewaehlten Aufnahmen gemeinsam, mit Vorlage
/// (`"auto"`, eine ID oder `None` = Standard) und Art (`"minutes"` oder
/// `"summary"`). Kehrt erst am Ende zurueck (wie `meetings_generate_minutes`); die
/// Oberflaeche ruft ihn nicht blockierend und liest Fortschritt und Ende aus den
/// Ereignissen.
#[tauri::command]
#[specta::specta]
pub async fn project_minutes_generate(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    folder_id: String,
    meeting_ids: Vec<String>,
    template_id: Option<String>,
    kind: String,
) -> Result<ProjectMinutes, String> {
    let store = Arc::clone(&store);
    generate_and_notify(
        &app,
        store,
        &folder_id,
        &meeting_ids,
        template_id.as_deref(),
        &kind,
    )
    .await
}

/// Erzeugen und dabei Auftrag und Ereignisse bedienen.
pub async fn generate_and_notify(
    app: &AppHandle,
    store: Arc<MeetingStore>,
    folder_id: &str,
    meeting_ids: &[String],
    template_id: Option<&str>,
    kind: &str,
) -> Result<ProjectMinutes, String> {
    let kind = ProjectKind::parse(kind).ok_or_else(|| "kind_invalid".to_string())?;
    let key = project::run_key(folder_id);
    // Ein Lauf je Projekt: ein zweiter Start ist `minutes_busy`, der erste laeuft
    // weiter (kein Ereignis).
    let job = job::global()
        .try_start(&key, job::app_emit(app))
        .map_err(|_| CODE_BUSY.to_string())?;
    let handle = Arc::clone(job.handle());
    handle.begin_phase_ex(JobPhase::Minutes, 0, false);
    let cancel_id = folder_id.to_string();
    handle.set_on_stop(Box::new(move || {
        project::request_cancel(&cancel_id);
    }));

    // Jede Teilphase (Vorlage, Schreiben, Zusammenfuehren) zaehlt ihre eigenen
    // Schritte: bei einem Wechsel beginnt der Auftrag eine neue Phase.
    let bridge = Arc::clone(&handle);
    let last_phase = std::sync::Mutex::new(None::<(MinutesPhase, u32)>);
    let settings = crate::settings::get_settings(app);
    let request = Request {
        folder_id,
        meeting_ids,
        template_id,
        kind,
    };
    let result = project::generate_project_minutes(&settings, store, &request, &move |p| {
        let mut last = last_phase.lock().unwrap_or_else(|e| e.into_inner());
        if *last != Some((p.phase, p.total)) {
            bridge.begin_phase_ex(JobPhase::Minutes, u64::from(p.total), false);
            *last = Some((p.phase, p.total));
        }
        bridge.advance(u64::from(p.done));
    })
    .await;
    match &result {
        Ok(document) => {
            let _ = ProjectMinutesEvent::Done {
                folder_id: folder_id.to_string(),
                minutes_id: document.id.clone(),
            }
            .emit(app);
        }
        Err(message) => {
            let code = project::error_code(message);
            log::warn!("Projekt-Protokoll fehlgeschlagen: {code}");
            if code != CODE_BUSY {
                let detail = message
                    .strip_prefix(code)
                    .and_then(|rest| rest.strip_prefix(':'))
                    .map(|d| d.trim().to_string())
                    .unwrap_or_default();
                let _ = ProjectMinutesEvent::Failed {
                    folder_id: folder_id.to_string(),
                    code: code.to_string(),
                    // Nur kurze Gruende (IDs, Zahlen): kein Fehlertext des Modells
                    // gelangt in die Oberflaeche, der koennte Transkript zitieren.
                    detail: if code == "llm_failed" || code == "memory_low" {
                        String::new()
                    } else {
                        detail
                    },
                }
                .emit(app);
            }
        }
    }
    result
}
