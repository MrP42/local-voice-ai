//! D1 (#70, M7 = #69): duenne Command-Huelle ueber die Folienerkennung
//! (`managers::meetings::slides`). Die Logik steht im Manager; hier stehen
//! Argumente, die Vorpruefungen, der Start als Auftrag und das Ende-Ereignis.
//!
//! Der Lauf gehoert dem Backend, nicht dem Reiter: er ist ein Auftrag im
//! Verzeichnis der Verarbeitungen (`job.rs`, Phase Folien). Fortschritt (mit
//! Laufzeit und Restdauer), Pause und Stopp laufen wie bei jeder Verarbeitung ueber
//! `MeetingEvent::Progress`, `meetings_progress_list`, `meetings_job_pause`,
//! `meetings_job_resume` und `meetings_job_stop`. Das Ende kommt als
//! [`MeetingSlidesEvent`].
//!
//! Fehlercodes (die Oberflaeche uebersetzt sie): `meetings_unavailable`,
//! `meeting_not_found`, `meeting_not_finished` (Aufnahme oder Import laeuft oder
//! wartet in der Warteschlange: die Folien kommen erst danach), `recording_active`
//! (eine Live-Aufnahme hat Vorrang), `job_busy`, `slides_no_video` (die
//! Besprechung hat keine Videodatei), `slides_video_missing` (die Datei gibt es
//! nicht mehr), `slide_not_found`, `store_failed`. Im Ereignis zusaetzlich die
//! Codes aus `SlideError::code` (`slides_ffmpeg_missing`, `slides_disk_full`, ...).
//!
//! Datenschutz (D9): weder Bildinhalt noch Dateipfade gelangen ins Log; geloggt
//! werden nur Codes.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::commands::meetings::{require_managed, store_err};
use crate::managers::meetings::job::{self, JobGuard};
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::slides::ocr::{self, OcrBackend};
use crate::managers::meetings::slides::run::{self, SlideOutcome, SlideRun};
use crate::managers::meetings::slides::store::MeetingSlide;
use crate::managers::meetings::slides::{SlideDetectConfig, SlideError, SlideOptions};
use crate::managers::meetings::store::MeetingStore;

/// Ende eines Folienlaufs. `Done` und `Stopped` tragen die Zaehler des Laufs: die
/// Oberflaeche laedt danach `list_meeting_slides` neu. `Skipped` ist kein Fehler
/// (keine Videospur, ffmpeg fehlt): ein Hinweis. `Failed` traegt einen Code aus
/// `SlideError::code`. Der Fortschritt kommt als `MeetingEvent::Progress`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeetingSlidesEvent {
    Done {
        meeting_id: String,
        /// Verschiedene Folien, die dieser Lauf im Video gefunden hat.
        slides: u32,
        /// Davon neu angelegt (bei einer Wiederholung weniger).
        added: u32,
    },
    Stopped {
        meeting_id: String,
        slides: u32,
        added: u32,
    },
    Skipped {
        meeting_id: String,
        code: String,
    },
    Failed {
        meeting_id: String,
        code: String,
    },
}

/// Darf fuer eine Besprechung in diesem Zustand ein Folienlauf beginnen? Nur wenn
/// nichts anderes sie belegt: eine Aufnahme, eine laufende Verarbeitung und eine in
/// der Import-Warteschlange wartende Datei (`queued`) behalten ihren Auftrag, die
/// Folien kommen DANACH (der Import nimmt sie als Phase in seinen eigenen Auftrag).
pub fn check_startable(status: &str) -> Result<(), &'static str> {
    match status {
        "ready" | "failed" | "cancelled" => Ok(()),
        _ => Err("meeting_not_finished"),
    }
}

/// Die Videodatei: die ausdrueckliche Angabe, sonst die Quelle des Imports. Die
/// Datei muss es geben.
pub fn resolve_video(
    explicit: Option<&str>,
    source_path: Option<&str>,
) -> Result<PathBuf, &'static str> {
    let chosen = explicit
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .or_else(|| source_path.map(str::trim).filter(|p| !p.is_empty()))
        .ok_or("slides_no_video")?;
    let path = PathBuf::from(chosen);
    if path.is_file() {
        Ok(path)
    } else {
        Err("slides_video_missing")
    }
}

/// Der Ordner einer Besprechung unter `base`. Eine ID, die kein einzelner
/// Ordnername ist (`..`, Trennzeichen, leer), wird abgelehnt: die ID kommt zwar aus
/// der Datenbank, ein zusammengesetzter Pfad prueft sich trotzdem selbst.
pub fn meeting_dir_of(base: &Path, meeting_id: &str) -> Result<PathBuf, &'static str> {
    let mut parts = Path::new(meeting_id).components();
    let one_name = matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(_)), None)
    );
    if !one_name || meeting_id.contains(['/', '\\']) {
        return Err("meeting_dir_invalid_id");
    }
    Ok(base.join(meeting_id))
}

fn store_of(app: &AppHandle) -> Result<Arc<MeetingStore>, String> {
    require_managed(
        app.try_state::<Arc<MeetingStore>>()
            .map(|state| Arc::clone(&state)),
    )
}

/// Das Ende-Ereignis zu einem Ergebnis des Laufs.
pub fn event_for(
    meeting_id: &str,
    result: &Result<run::SlideSummary, SlideError>,
) -> MeetingSlidesEvent {
    let meeting_id = meeting_id.to_string();
    match result {
        Ok(summary) => match summary.outcome {
            SlideOutcome::Done => MeetingSlidesEvent::Done {
                meeting_id,
                slides: summary.groups,
                added: summary.added,
            },
            SlideOutcome::Stopped => MeetingSlidesEvent::Stopped {
                meeting_id,
                slides: summary.groups,
                added: summary.added,
            },
            SlideOutcome::Skipped(code) => MeetingSlidesEvent::Skipped {
                meeting_id,
                code: code.to_string(),
            },
        },
        Err(e) => MeetingSlidesEvent::Failed {
            meeting_id,
            code: e.code().to_string(),
        },
    }
}

/// Fuehrt den Lauf aus (blockierend), meldet das Ende und gibt den Auftrag frei.
/// Eine Panik wird zum Ereignis `Failed` (`slides_panic`): der Auftrag steht danach
/// nie als Geist im Verzeichnis (der `JobGuard` raeumt beim Drop).
fn execute(
    app: &AppHandle,
    store: &MeetingStore,
    job: JobGuard,
    meeting_id: &str,
    video: &Path,
    meeting_dir: &Path,
    cfg: &SlideDetectConfig,
) {
    let handle = Arc::clone(job.handle());
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // Texterkennung des Systems (D2); ohne sie laeuft die Erkennung ohne Folientext.
        let backend = ocr::default_backend(ocr::PREFERRED_LANGUAGES);
        let reader: Option<&dyn OcrBackend> = match &backend {
            Ok(b) => Some(b.as_ref()),
            Err(e) => {
                log::info!("slides: keine Texterkennung ({e}), Folien ohne Text");
                None
            }
        };
        run::run(
            &handle,
            &SlideRun {
                store,
                meeting_id,
                video,
                meeting_dir,
                cfg,
                ocr: reader,
                pid_out: None,
            },
        )
    }));
    let event = match outcome {
        Ok(result) => {
            match &result {
                Ok(summary) => log::info!("slides: Lauf beendet ({:?})", summary.outcome),
                Err(e) => log::warn!("slides: Lauf gescheitert ({})", e.code()),
            }
            event_for(meeting_id, &result)
        }
        Err(_) => {
            log::error!("slides: Lauf mit Panik beendet");
            MeetingSlidesEvent::Failed {
                meeting_id: meeting_id.to_string(),
                code: "slides_panic".to_string(),
            }
        }
    };
    let _ = event.emit(app);
    drop(job); // meldet `JobEnded`
    // D5: neue oder geaenderte Folientexte gehoeren in den Suchindex (Chat, Suche).
    reindex(app, meeting_id);
}

/// D5: reiht die Besprechung beim Indexer ein (Folientext findbar). Ohne Indexer: nichts.
fn reindex(app: &AppHandle, meeting_id: &str) {
    use crate::managers::meetings::search::indexer::{self, IndexJob};
    indexer::submit(app, IndexJob::Meeting(meeting_id.to_string()));
}

/// Startet die Folienerkennung fuer eine fertige Besprechung mit Videodatei. Kehrt
/// sofort zurueck; Fortschritt und Ende kommen als Ereignisse (siehe Modulkopf).
/// Wiederholbar: ein zweiter Lauf fuegt nichts doppelt ein.
#[tauri::command]
#[specta::specta]
pub async fn detect_meeting_slides(
    app: AppHandle,
    meeting_id: String,
    options: SlideOptions,
) -> Result<(), String> {
    let store = store_of(&app)?;
    let meeting = store
        .get_meeting(&meeting_id)
        .map_err(store_err)?
        .ok_or_else(|| "meeting_not_found".to_string())?;
    check_startable(&meeting.status).map_err(str::to_string)?;
    if app
        .try_state::<Arc<MeetingRecorderManager>>()
        .is_some_and(|recorder| recorder.is_recording())
    {
        return Err("recording_active".to_string());
    }
    let video = resolve_video(
        options.video_path.as_deref(),
        meeting.source_path.as_deref(),
    )
    .map_err(str::to_string)?;
    let base = crate::managers::meetings::meetings_data_dir(&app)
        .map_err(|e| format!("app_data_dir_failed: {e}"))?;
    let meeting_dir = meeting_dir_of(&base, &meeting_id).map_err(str::to_string)?;
    let cfg = options.to_config();

    let job = job::global()
        .try_start(&meeting_id, job::app_emit(&app))
        .map_err(|e| e.to_string())?;
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        execute(&app, &store, job, &meeting_id, &video, &meeting_dir, &cfg);
    });
    Ok(())
}

/// Die Folien einer Besprechung, nach Nummer (auch ausgeblendete; die Oberflaeche
/// filtert). Bildpfade sind relativ zum Ordner aus `meeting_slides_dir`.
#[tauri::command]
#[specta::specta]
pub async fn list_meeting_slides(
    app: AppHandle,
    meeting_id: String,
) -> Result<Vec<MeetingSlide>, String> {
    let store = store_of(&app)?;
    store.slides_list(&meeting_id).map_err(store_err)
}

/// Blendet eine Folie aus oder wieder ein. `slide_not_found`, wenn es sie nicht gibt.
#[tauri::command]
#[specta::specta]
pub async fn set_meeting_slide_hidden(
    app: AppHandle,
    slide_id: String,
    hidden: bool,
) -> Result<(), String> {
    let store = store_of(&app)?;
    if store
        .slide_set_hidden(&slide_id, hidden)
        .map_err(store_err)?
    {
        // D5: eine ausgeblendete Folie verlaesst den Suchindex (und kommt wieder hinein).
        if let Ok(Some(slide)) = store.slide_get(&slide_id) {
            reindex(&app, &slide.meeting_id);
        }
        Ok(())
    } else {
        Err("slide_not_found".to_string())
    }
}

/// Der Besprechungsordner (absolut), in dem `slides/` liegt: davor setzt die
/// Oberflaeche den relativen `image_path` einer Folie zusammen.
#[tauri::command]
#[specta::specta]
pub async fn meeting_slides_dir(app: AppHandle, meeting_id: String) -> Result<String, String> {
    let base = crate::managers::meetings::meetings_data_dir(&app)
        .map_err(|e| format!("app_data_dir_failed: {e}"))?;
    let dir = meeting_dir_of(&base, &meeting_id).map_err(str::to_string)?;
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::slides::run::SlideSummary;

    #[test]
    fn only_finished_meetings_may_start_a_slides_run() {
        for ok in ["ready", "failed", "cancelled"] {
            assert_eq!(check_startable(ok), Ok(()), "{ok}");
        }
        // Aufnahme, Verarbeitung und die Import-Warteschlange behalten ihren Auftrag.
        for busy in ["recording", "processing", "queued", ""] {
            assert_eq!(check_startable(busy), Err("meeting_not_finished"), "{busy}");
        }
    }

    #[test]
    fn the_video_is_the_explicit_path_or_the_import_source_and_must_exist() {
        let dir = tempfile::tempdir().unwrap();
        let explicit = dir.path().join("a.mp4");
        let source = dir.path().join("b.mp4");
        std::fs::write(&explicit, b"x").unwrap();
        std::fs::write(&source, b"x").unwrap();
        let (e, s) = (explicit.to_str().unwrap(), source.to_str().unwrap());
        assert_eq!(
            resolve_video(Some(e), Some(s)),
            Ok(explicit.clone()),
            "ausdruecklich gewinnt"
        );
        assert_eq!(resolve_video(None, Some(s)), Ok(source.clone()));
        assert_eq!(
            resolve_video(Some("  "), Some(s)),
            Ok(source.clone()),
            "leer zaehlt nicht"
        );
        assert_eq!(resolve_video(None, None), Err("slides_no_video"));
        assert_eq!(resolve_video(Some(""), Some("")), Err("slides_no_video"));
        assert_eq!(
            resolve_video(Some(dir.path().join("weg.mp4").to_str().unwrap()), None),
            Err("slides_video_missing")
        );
        assert_eq!(
            resolve_video(None, Some(dir.path().to_str().unwrap())),
            Err("slides_video_missing"),
            "ein Ordner ist keine Datei"
        );
    }

    #[test]
    fn only_a_single_folder_name_is_a_meeting_dir() {
        let base = Path::new("C:/data/meetings");
        assert_eq!(
            meeting_dir_of(base, "01JABCDEFGHJKMNPQRSTVWXYZ0"),
            Ok(base.join("01JABCDEFGHJKMNPQRSTVWXYZ0"))
        );
        for bad in ["", "..", ".", "a/b", "a\\b", "../x", "C:\\x", "/abs"] {
            assert_eq!(
                meeting_dir_of(base, bad),
                Err("meeting_dir_invalid_id"),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn every_result_maps_to_exactly_one_end_event() {
        let summary = |outcome| SlideSummary {
            outcome,
            segments: 5,
            groups: 4,
            added: 3,
            updated: 1,
            duration_ms: Some(1),
            ocr_engine: None,
            text_slides: 0,
            ohne_text: 0,
            ocr_failed: 0,
        };
        assert_eq!(
            event_for("m", &Ok(summary(SlideOutcome::Done))),
            MeetingSlidesEvent::Done {
                meeting_id: "m".into(),
                slides: 4,
                added: 3
            }
        );
        assert_eq!(
            event_for("m", &Ok(summary(SlideOutcome::Stopped))),
            MeetingSlidesEvent::Stopped {
                meeting_id: "m".into(),
                slides: 4,
                added: 3
            }
        );
        assert_eq!(
            event_for("m", &Ok(summary(SlideOutcome::Skipped("slides_no_video")))),
            MeetingSlidesEvent::Skipped {
                meeting_id: "m".into(),
                code: "slides_no_video".into()
            }
        );
        assert_eq!(
            event_for("m", &Err(SlideError::DiskFull)),
            MeetingSlidesEvent::Failed {
                meeting_id: "m".into(),
                code: "slides_disk_full".into()
            }
        );
        // Ein Fehlertext mit Pfad oder Detail geht nie ins Ereignis, nur der Code.
        let leaky = SlideError::Ffmpeg("C:\\Users\\x\\geheim.mp4: kaputt".into());
        match event_for("m", &Err(leaky)) {
            MeetingSlidesEvent::Failed { code, .. } => assert_eq!(code, "slides_ffmpeg_failed"),
            other => panic!("{other:?}"),
        }
    }
}
