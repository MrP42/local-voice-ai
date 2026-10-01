//! Die App-Seite der Import-Bausteine (B3): die echte Import-Warteschlange, der Speicher der
//! Besprechungen, der YouTube-Quellweg und die Auskunft ueber Aufnahme und Warteschlange fuer das
//! Tor der schweren Schritte. Duenner Kleber; die Entscheidungen stehen in `import`,
//! `queue_gate` und `trigger::*` und sind dort getestet. Diese Datei braucht den `AppHandle`.
//!
//! Zustaende der App werden erst beim Aufruf geholt (`try_state`): Engine und Hub starten
//! frueher als die Besprechungen; fehlt noch etwas, antwortet der Kleber mit
//! `meetings_unavailable` (der Schritt wiederholt sich, nichts ist entstanden).

use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use tauri::{AppHandle, Manager};

use crate::managers::meetings::import::import_subtitle_file_into;
use crate::managers::meetings::metadata::MetadataEdit;
use crate::managers::meetings::queue::{self, ImportQueue};
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::meetings::store::MeetingStore;
use crate::managers::youtube::source::{self, AddOptions};
use crate::managers::youtube::{YoutubeError, METADATA_KEY, SOURCE_KIND};

use super::engine::Engine;
use super::heavy::{HeavyGate, LocalHeavyGate};
use super::import::{
    self, ImportControl, ImportRequest, ImportState, Imported, MeetingRef, YoutubeControl,
};
use super::queue_gate::{QueueAwareGate, QueueProbe};

fn store_of(app: &AppHandle) -> Option<Arc<MeetingStore>> {
    app.try_state::<Arc<MeetingStore>>().map(|s| Arc::clone(&s))
}

pub struct AppImport {
    app: AppHandle,
}

impl AppImport {
    pub fn new(app: &AppHandle) -> Self {
        Self { app: app.clone() }
    }
}

fn project_exists(store: &MeetingStore, project_id: &str) -> Result<bool, String> {
    let conn = store
        .get_connection()
        .map_err(|e| format!("queue_enqueue_failed: {e}"))?;
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM meeting_folders WHERE id = ?1 AND deleted_at IS NULL)",
        params![project_id],
        |r| r.get(0),
    )
    .map_err(|e| format!("queue_enqueue_failed: {e}"))
}

impl ImportControl for AppImport {
    fn enqueue(&self, req: &ImportRequest) -> Result<Imported, String> {
        let store = store_of(&self.app).ok_or_else(|| "meetings_unavailable".to_string())?;
        // Das Projekt zuerst: fehlt es, entsteht nichts.
        if let Some(project) = &req.project {
            if !project_exists(&store, project)? {
                return Err("folder_not_found".to_string());
            }
        }
        let source = req
            .path
            .to_str()
            .ok_or_else(|| "import_path_invalid".to_string())?
            .to_string();
        // Keine Einwilligung hinterlegt: sie wurde nicht erfragt (siehe `import`).
        let imported = if queue::is_subtitle(&req.path) {
            let id = import_subtitle_file_into(&store, &req.title, &req.path, None, None)?;
            indexer::submit(&self.app, IndexJob::Meeting(id.clone()));
            Imported {
                meeting_id: id,
                title: req.title.clone(),
                state: ImportState::Ready,
            }
        } else {
            let queue = self
                .app
                .try_state::<Arc<ImportQueue>>()
                .map(|q| Arc::clone(&q))
                .ok_or_else(|| "meetings_unavailable".to_string())?;
            let meeting = queue.enqueue(&req.title, &source, None)?;
            Imported {
                meeting_id: meeting.id,
                title: meeting.title,
                state: ImportState::Queued,
            }
        };
        if let Some(project) = &req.project {
            let edit = MetadataEdit {
                folder_ids: Some(vec![project.clone()]),
                ..MetadataEdit::default()
            };
            if let Err(e) = store.update_metadata(&imported.meeting_id, &edit) {
                // Die Besprechung steht; nur die Zuordnung fehlt (das Projekt verschwand eben).
                log::warn!(
                    "workflows: Besprechung {} nicht dem Projekt zugeordnet: {e}",
                    imported.meeting_id
                );
            }
        }
        Ok(imported)
    }

    fn find_since(&self, source_path: &str, since_ms: i64) -> Option<Imported> {
        let store = store_of(&self.app)?;
        let conn = store.get_connection().ok()?;
        conn.query_row(
            "SELECT id, title, status FROM meetings
             WHERE source_path = ?1 AND created_at >= ?2 AND deleted_at IS NULL
             ORDER BY created_at DESC LIMIT 1",
            params![source_path, since_ms.div_euclid(1_000)],
            |r| {
                let status: String = r.get(2)?;
                Ok(Imported {
                    meeting_id: r.get(0)?,
                    title: r.get(1)?,
                    state: if status == "ready" {
                        ImportState::Ready
                    } else {
                        ImportState::Queued
                    },
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }
}

pub struct AppYoutube {
    app: AppHandle,
}

impl AppYoutube {
    pub fn new(app: &AppHandle) -> Self {
        Self { app: app.clone() }
    }
}

impl YoutubeControl for AppYoutube {
    fn find_video(&self, video_id: &str) -> Option<MeetingRef> {
        let store = store_of(&self.app)?;
        let conn = store.get_connection().ok()?;
        conn.query_row(
            "SELECT id, title, created_at FROM meetings
             WHERE source = ?1 AND deleted_at IS NULL
               AND json_extract(metadata_json, '$.' || ?2 || '.video_id') = ?3
             ORDER BY created_at ASC LIMIT 1",
            params![SOURCE_KIND, METADATA_KEY, video_id],
            |r| {
                let secs: i64 = r.get(2)?;
                Ok(MeetingRef {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    created_at_ms: secs.saturating_mul(1_000),
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    fn add(&self, url: &str, project: Option<&str>) -> Result<MeetingRef, YoutubeError> {
        let store = store_of(&self.app)
            .ok_or_else(|| YoutubeError::Store("Besprechungen nicht verfügbar".to_string()))?;
        // Der Arbeiter der Engine ist ein eigener Thread ohne Laufzeit: blockieren ist erlaubt.
        let added = tauri::async_runtime::block_on(source::add_youtube_source(
            &store,
            url,
            project,
            &AddOptions::production(),
        ))?;
        indexer::submit(&self.app, IndexJob::Meeting(added.meeting.id.clone()));
        Ok(MeetingRef {
            id: added.meeting.id,
            title: added.meeting.title,
            created_at_ms: added.meeting.created_at.saturating_mul(1_000),
        })
    }
}

/// Auskunft fuer das Tor der schweren Schritte.
pub struct AppQueueProbe {
    app: AppHandle,
}

impl AppQueueProbe {
    pub fn new(app: &AppHandle) -> Self {
        Self { app: app.clone() }
    }
}

impl QueueProbe for AppQueueProbe {
    fn recording(&self) -> bool {
        self.app
            .try_state::<Arc<MeetingRecorderManager>>()
            .is_some_and(|r| r.is_recording())
    }

    fn import_busy(&self) -> bool {
        self.app
            .try_state::<Arc<ImportQueue>>()
            .is_some_and(|q| {
                let s = q.snapshot();
                !s.waiting.is_empty() || !s.running.is_empty()
            })
    }
}

/// Das Tor der Engine in der App: ein Platz und das RAM-Tor (`LocalHeavyGate`), davor Aufnahme und
/// Import-Warteschlange.
pub fn heavy_gate(app: &AppHandle) -> Arc<dyn HeavyGate> {
    Arc::new(QueueAwareGate::new(
        LocalHeavyGate::shared(),
        Arc::new(AppQueueProbe::new(app)),
    ))
}

/// Haengt die Import-Bausteine in die Engine.
pub fn install(engine: &Engine, app: &AppHandle) {
    import::install(
        engine,
        Arc::new(AppImport::new(app)),
        Arc::new(AppYoutube::new(app)),
    );
}
