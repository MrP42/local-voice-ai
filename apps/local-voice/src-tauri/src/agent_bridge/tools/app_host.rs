//! Der echte Zugriff der Werkzeuge auf die laufende App (A8). Alles hier loest ueber den
//! `AppHandle` zur Laufzeit auf (`try_state`): die Werkzeuge werden vor Warteschlange, Recorder
//! und Sprach-Manager angemeldet und finden sie, sobald sie da sind. Fehlt etwas, ist das eine
//! Fehlermeldung des Werkzeugs, nie eine Panik.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use super::{Host, PageRef, QueuedImport, Rendered};
use crate::managers::meetings::queue::ImportQueue;
use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::tts::TtsManager;
use crate::managers::workflows::hub::{AppRecording, WorkflowHub};
use crate::managers::workflows::recording::RecordingControl;
use crate::managers::youtube::source::AddOptions;

pub struct AppHost {
    app: AppHandle,
}

impl AppHost {
    pub fn new(app: &AppHandle) -> Self {
        Self { app: app.clone() }
    }
}

/// Ein Dateiname im Ordner der Seite, den es noch nicht gibt (`name.wav`, dann `name (2).wav` ...).
/// Ein Agent ueberschreibt nie eine vorhandene Datei. `final_path` rechnet die Endung ein, die
/// die Einstellung der App wirklich bestimmt (WAV oder MP3).
fn free_target(
    dir: &Path,
    stem: &str,
    final_path: &dyn Fn(&str) -> String,
) -> Result<PathBuf, String> {
    for n in 1..=200u32 {
        let name = if n == 1 {
            format!("{stem}.wav")
        } else {
            format!("{stem} ({n}).wav")
        };
        let candidate = dir.join(&name);
        let landing = final_path(&candidate.to_string_lossy());
        if !Path::new(&landing).exists() && !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("Zu viele Dateien mit diesem Namen.".to_string())
}

impl Host for AppHost {
    fn enqueue_import(&self, title: &str, source_path: &str) -> Result<QueuedImport, String> {
        let queue = self
            .app
            .try_state::<Arc<ImportQueue>>()
            .ok_or_else(|| "Die Import-Warteschlange ist noch nicht bereit.".to_string())?;
        // Die Einwilligung fuer eine Datei bestaetigt der Nutzer in der App; ein Agent
        // bestaetigt sie nie fuer ihn (wie bei jedem Import ohne Haekchen: kein Zeitstempel).
        let meeting = queue.enqueue(title, source_path, None)?;
        Ok(QueuedImport {
            meeting_id: meeting.id,
            title: meeting.title,
            status: meeting.status,
        })
    }

    fn create_page(&self, title: &str, text: &str) -> Result<PageRef, String> {
        let page = crate::commands::pages::pages_create(self.app.clone(), title.to_string())?;
        let state =
            json!({ "text": text, "summary": "", "sourceUrl": "", "tab": "original" }).to_string();
        if let Err(e) =
            crate::commands::pages::page_state_save(self.app.clone(), page.id.clone(), state)
        {
            // Keine leere Seite zurueckbehalten.
            let _ = crate::commands::pages::pages_delete(self.app.clone(), page.id);
            return Err(e);
        }
        let _ = self.app.emit(crate::sync::engine::EVENT_CHANGED, ());
        Ok(PageRef {
            id: page.id,
            title: page.title,
        })
    }

    fn page_text(&self, page_id: &str) -> Result<String, String> {
        let raw = crate::commands::pages::page_state_load(self.app.clone(), page_id.to_string())?;
        if raw.trim().is_empty() {
            return Ok(String::new());
        }
        Ok(serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|v| v.get("text").and_then(Value::as_str).map(str::to_string))
            // Aeltere Staende der Oberflaeche: der Rohtext ist der Text.
            .unwrap_or(raw))
    }

    fn render_audio(&self, page_id: &str, text: &str, file_name: &str) -> Result<Rendered, String> {
        let tts = self
            .app
            .try_state::<Arc<TtsManager>>()
            .ok_or_else(|| "Die Sprachausgabe ist noch nicht bereit.".to_string())?;
        let dir = PathBuf::from(crate::commands::pages::page_dir(
            self.app.clone(),
            page_id.to_string(),
        )?);
        let target = free_target(&dir, file_name, &|p| tts.export_target_path(p))?;
        let target_text = target.to_string_lossy().into_owned();
        // Das Format bestimmt die Einstellung der App; der Lauf kehrt mit dem echten Pfad zurueck.
        let (_, landed) = tauri::async_runtime::block_on(tts.speak_to_file(text, &target_text))?;
        let meta = std::fs::metadata(&landed).map_err(|e| format!("Datei nicht gefunden: {e}"))?;
        let path = PathBuf::from(&landed);
        let format = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("wav")
            .to_ascii_lowercase();
        let file = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(file_name)
            .to_string();
        let _ = self.app.emit(crate::sync::engine::EVENT_CHANGED, ());
        Ok(Rendered {
            file_name: file,
            path: landed,
            bytes: meta.len(),
            format,
        })
    }

    fn recording(&self) -> Option<Arc<dyn RecordingControl>> {
        // Ohne Recorder (Besprechungen nicht verfuegbar) gibt es keine Aufnahme.
        self.app
            .try_state::<Arc<crate::managers::meetings::recorder::MeetingRecorderManager>>()?;
        Some(Arc::new(AppRecording::new(self.app.clone())))
    }

    fn schedule_stop(&self, meeting_id: &str, at_ms: i64) {
        if let Some(hub) = self.app.try_state::<Arc<WorkflowHub>>() {
            hub.schedule_stop(meeting_id, at_ms);
        } else {
            log::warn!("agent_bridge: kein Ende fuer die Aufnahme {meeting_id} vorgemerkt (kein Ablauf-Hub)");
        }
    }

    fn meeting_created(&self, meeting_id: &str) {
        indexer::submit(&self.app, IndexJob::Meeting(meeting_id.to_string()));
    }

    fn youtube_options(&self) -> AddOptions {
        AddOptions::production()
    }
}
