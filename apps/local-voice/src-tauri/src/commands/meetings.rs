//! Thin command shell over `MeetingRecorderManager` and `MeetingStore`
//! (pattern: `commands/history.rs`). No logic beyond argument shuffling and
//! error mapping lives here.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;

use crate::managers::meetings::import::import_media_file;
use crate::managers::meetings::minutes::{generate_minutes, latest_minutes_file};
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::retention::delete_audio_files;
use crate::managers::meetings::retranscribe::retranscribe_meeting;
use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::meetings::store::{Meeting, MeetingDocument, MeetingStore, StoredSegment};
use crate::managers::transcription::TranscriptionManager;

/// Starting touches audio hardware and can block for seconds (loopback
/// start-up), hence `spawn_blocking` rather than running on the command task.
#[tauri::command]
#[specta::specta]
pub async fn meetings_start(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    title: String,
    consent_confirmed: bool,
    capture_system: bool,
) -> Result<Meeting, String> {
    let recorder = Arc::clone(&recorder);
    tauri::async_runtime::spawn_blocking(move || {
        recorder.start(title, consent_confirmed, capture_system)
    })
    .await
    .map_err(|e| format!("meetings_start panicked: {e}"))?
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_pause(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
) -> Result<(), String> {
    recorder.pause()
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_resume(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
) -> Result<(), String> {
    recorder.resume()
}

/// Stopping waits for the tail chunks to finish transcribing, so it must not
/// occupy the async runtime's worker.
#[tauri::command]
#[specta::specta]
pub async fn meetings_stop(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
) -> Result<String, String> {
    let recorder = Arc::clone(&recorder);
    tauri::async_runtime::spawn_blocking(move || recorder.stop())
        .await
        .map_err(|e| format!("meetings_stop panicked: {e}"))?
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_is_recording(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
) -> Result<bool, String> {
    Ok(recorder.is_recording())
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_list(
    store: State<'_, Arc<MeetingStore>>,
    offset: u32,
    limit: u32,
) -> Result<Vec<Meeting>, String> {
    store
        .list_meetings(offset, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_get_segments(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<StoredSegment>, String> {
    store.get_segments(&meeting_id).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_update_segment(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    segment_index: u32,
    text: String,
) -> Result<(), String> {
    store
        .update_segment_text(&meeting_id, segment_index, &text)
        .map_err(|e| e.to_string())?;
    indexer::submit(&app, IndexJob::Meeting(meeting_id)); // M4-P4b
    Ok(())
}

/// Renames a meeting. The title is free text and deliberately independent of
/// the file an imported meeting came from (that is kept in `source_path`).
#[tauri::command]
#[specta::specta]
pub async fn meetings_rename(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    title: String,
) -> Result<(), String> {
    store
        .set_title(&meeting_id, &title)
        .map_err(|e| e.to_string())?;
    indexer::submit(&app, IndexJob::Meeting(meeting_id)); // M4-P4b
    Ok(())
}

/// Re-runs the transcription of a finished meeting from its stored audio,
/// optionally with a different model. Discards the old segments — see
/// `retranscribe_meeting`.
#[tauri::command]
#[specta::specta]
pub async fn meetings_retranscribe(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    transcription: State<'_, Arc<TranscriptionManager>>,
    meeting_id: String,
    model_id: Option<String>,
) -> Result<(), String> {
    let store = Arc::clone(&store);
    let transcription = Arc::clone(&transcription);
    retranscribe_meeting(&app, store, transcription, meeting_id, model_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_get_documents(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<MeetingDocument>, String> {
    store.get_documents(&meeting_id).map_err(|e| e.to_string())
}

/// Soft-deletes the meeting, then hard-deletes its audio files from disk
/// (Spec A2 — a tombstoned meeting must never leave an orphaned WAV behind).
#[tauri::command]
#[specta::specta]
pub async fn meetings_delete(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<(), String> {
    let paths = store
        .soft_delete_meeting(&meeting_id)
        .map_err(|e| e.to_string())?;
    indexer::submit(&app, IndexJob::Deleted(meeting_id)); // M4-P4b
    delete_audio_files(&paths);
    Ok(())
}

/// Generates the standardized minutes for a finished meeting and stores them
/// as a new document version. The meeting status stays untouched — a failed
/// generation leaves a 'ready' meeting 'ready' and only returns the error.
#[tauri::command]
#[specta::specta]
pub async fn meetings_generate_minutes(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<MeetingDocument, String> {
    let store = Arc::clone(&store);
    generate_minutes(&app, store, &meeting_id).await
}

/// Where this meeting's minutes were filed as Markdown, if the file is there.
/// The database holds the authoritative copy; this is the convenience copy the
/// generator drops next to the recording so it can be opened without the app.
#[tauri::command]
#[specta::specta]
pub async fn meetings_minutes_file(
    app: tauri::AppHandle,
    meeting_id: String,
) -> Result<Option<String>, String> {
    let path = latest_minutes_file(&app, &meeting_id).map_err(|e| e.to_string())?;
    Ok(path.map(|p| p.to_string_lossy().into_owned()))
}

/// Writes a document the user assembled in the app to a path they picked in
/// the system save dialog — as Markdown, plain text or Word, chosen by the
/// file extension.
///
/// This deliberately does NOT go through the fs plugin from the frontend:
/// its capability scope is limited to `$APPDATA`, so saving into Documents —
/// what the save dialog offers — failed with "not allowed by ACL". The path
/// comes from the user's own choice in a system dialog; re-checking it
/// against an allowlist protects nobody. Writing here also makes Word export
/// possible at all, since a .docx is a ZIP archive rather than text.
#[tauri::command]
#[specta::specta]
pub async fn meetings_export_document(path: String, body: String) -> Result<(), String> {
    let target = std::path::PathBuf::from(&path);
    crate::managers::meetings::export::write_document(&target, &body)
}

/// Imports a local audio/video file or a VTT/SRT subtitle file as a new
/// meeting. Audio/video decoding and transcription can take a while, hence
/// this stays `async` end to end rather than blocking the command task
/// (`import_media_file` itself moves the heavy work to `spawn_blocking`).
#[tauri::command]
#[specta::specta]
pub async fn meetings_import_file(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    transcription: State<'_, Arc<TranscriptionManager>>,
    path: String,
    consent_confirmed: bool,
) -> Result<String, String> {
    let store = Arc::clone(&store);
    let transcription = Arc::clone(&transcription);
    import_media_file(
        &app,
        store,
        transcription,
        PathBuf::from(path),
        consent_confirmed,
    )
    .await
}

// M6-P6a: Export einer ganzen Besprechung.

/// Schreibt die Besprechung in eine vom Nutzer gewählte Datei; das Format
/// ergibt sich aus der Endung (`md`, `txt`, `docx`, `html`, `pdf`, `srt`,
/// `vtt`, `json`). `parts` wählt die Teile (SRT/VTT enthalten immer nur das
/// Transkript). Audio wird nie exportiert.
///
/// `pdf` läuft über ein verstecktes WebView2-Fenster (`meetings::pdf`, höchstens
/// 20 s). Ein Fehler beginnt mit `pdf_unavailable`, `pdf_timeout`,
/// `pdf_low_memory` oder `pdf_failed`; daran erkennt die Oberfläche, dass sie
/// „Drucken…" als Rückfall anbieten kann.
#[tauri::command]
#[specta::specta]
pub async fn meetings_export(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    path: String,
    parts: crate::managers::meetings::export::ExportParts,
) -> Result<(), String> {
    use crate::managers::meetings::export::{build_bundle, write_export, ExportFormat};
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || {
        let bundle = build_bundle(&store, &meeting_id)?;
        let target = PathBuf::from(&path);
        write_export(&target, ExportFormat::from_path(&target), &bundle, &parts)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Legt die Besprechung formatiert (HTML + Klartext) in die Zwischenablage.
#[tauri::command]
#[specta::specta]
pub async fn meetings_copy_formatted(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    parts: crate::managers::meetings::export::ExportParts,
) -> Result<(), String> {
    use crate::managers::meetings::export::{build_bundle, clipboard_payload};
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let store = Arc::clone(&store);
    let (html, text) = tauri::async_runtime::spawn_blocking(move || {
        build_bundle(&store, &meeting_id).map(|b| clipboard_payload(&b, &parts))
    })
    .await
    .map_err(|e| e.to_string())??;
    app.clipboard()
        .write_html(html, Some(text))
        .map_err(|e| e.to_string())
}

// M6-P6c: Follow-up-Mail.

/// Ausgang eines Follow-up-Entwurfs für `meeting_followup_open`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FollowupMode {
    /// HTML + Text in die Zwischenablage.
    Copy,
    /// Mailprogramm per `mailto:` öffnen.
    Mailto,
    /// Als .eml-Datei speichern (`path`).
    Eml,
}

/// Erzeugt den Follow-up-Entwurf: das Recipe "Follow-up-E-Mail an ..." läuft
/// im Scope der Besprechung (gleicher Motor, gleiche Sperren und Fehlercodes
/// wie `meeting_chat_ask`), danach wird die Antwort zum Entwurf. Empfänger
/// sind die Teilnehmenden ohne die eigene Person (leer, wenn keine bekannt).
/// Zusätzlicher Fehlercode: `followup_empty` (das Modell lieferte keinen Text).
#[tauri::command]
#[specta::specta]
pub async fn meeting_followup_draft(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    meeting_id: String,
) -> Result<crate::managers::meetings::mail::MailDraft, String> {
    use crate::managers::meetings::chat::{ChatRequest, ChatScope, RecipeCall};
    use crate::managers::meetings::mail::{draft_from_answer, participant_recipients};
    let settings = crate::settings::get_settings(&app);
    let (title, recipients) = {
        let store = Arc::clone(&store);
        let id = meeting_id.clone();
        let own = settings.meeting_self_emails.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let meeting = store
                .get_meeting(&id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "meeting_not_found".to_string())?;
            let recipients =
                participant_recipients(&store, &id, &own).map_err(|e| e.to_string())?;
            Ok::<_, String>((meeting.title, recipients))
        })
        .await
        .map_err(|e| e.to_string())??
    };
    let addressee = if recipients.names.is_empty() {
        "die Teilnehmenden".to_string()
    } else {
        recipients.names.join(", ")
    };
    let mut values = std::collections::HashMap::new();
    values.insert("empfaenger".to_string(), addressee);
    let req = ChatRequest {
        request_id: format!("followup-{}", ulid::Ulid::new()),
        thread_id: None,
        scope: ChatScope::Meeting {
            meeting_id: meeting_id.clone(),
        },
        question: String::new(),
        recipe: Some(RecipeCall {
            recipe_id: format!(
                "{}follow-up-mail",
                crate::managers::meetings::search::index::BUILTIN_PREFIX
            ),
            values,
        }),
    };
    let answer = crate::commands::meeting_chat::meeting_chat_ask(app, store, recorder, req).await?;
    if answer.not_found || answer.text.trim().is_empty() {
        return Err("followup_empty".to_string());
    }
    Ok(draft_from_answer(&answer.text, &title, recipients.emails))
}

/// Gibt einen (im Dialog bearbeiteten) Entwurf aus. `Copy`: HTML + Text in die
/// Zwischenablage. `Mailto`: Mailprogramm öffnen; ist die Adresse zu lang
/// (> 1 800 Zeichen), gehen nur Empfänger und Betreff mit, der Text kommt in
/// die Zwischenablage und das Ergebnis ist `true` (UI: Hinweis "einfügen").
/// `Eml`: Datei nach `path` schreiben (Endung `.eml` wird ergänzt).
/// Fehler: `clipboard_failed`, `mailto_failed` (Kopieren bleibt möglich),
/// `path_missing`, `write_failed`.
#[tauri::command]
#[specta::specta]
pub async fn meeting_followup_open(
    app: tauri::AppHandle,
    draft: crate::managers::meetings::mail::MailDraft,
    mode: FollowupMode,
    path: Option<String>,
) -> Result<bool, String> {
    use crate::managers::meetings::mail::{
        finalize, mailto_url, write_eml, MailDraft, MAILTO_MAX_LEN,
    };
    use tauri_plugin_clipboard_manager::ClipboardExt;
    use tauri_plugin_opener::OpenerExt;
    let d = finalize(&draft);
    let copy = |d: &MailDraft| {
        app.clipboard()
            .write_html(d.body_html.clone(), Some(d.body_text.clone()))
            .map_err(|e| {
                log::warn!("Follow-up: Zwischenablage fehlgeschlagen ({e})");
                "clipboard_failed".to_string()
            })
    };
    match mode {
        FollowupMode::Copy => copy(&d).map(|()| false),
        FollowupMode::Mailto => {
            let (url, clipped) = mailto_url(&d, MAILTO_MAX_LEN);
            if clipped {
                copy(&d)?;
            }
            app.opener().open_url(url, None::<String>).map_err(|e| {
                log::warn!("Follow-up: mailto nicht geöffnet ({e})");
                "mailto_failed".to_string()
            })?;
            Ok(clipped)
        }
        FollowupMode::Eml => {
            let raw = path.unwrap_or_default();
            if raw.trim().is_empty() {
                return Err("path_missing".to_string());
            }
            let mut target = PathBuf::from(raw.trim());
            if target.extension().is_none() {
                target.set_extension("eml");
            }
            let now = chrono::Utc::now();
            let seed = now.timestamp_nanos_opt().unwrap_or_default() as u64;
            let bytes = write_eml(&d, now, seed);
            tauri::async_runtime::spawn_blocking(move || std::fs::write(&target, bytes))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| {
                    log::warn!("Follow-up: .eml nicht geschrieben ({e})");
                    "write_failed".to_string()
                })?;
            Ok(false)
        }
    }
}

/// Einstellung `meeting_self_emails` ("Meine E-Mail-Adressen"): gespeichert
/// wird die bereinigte Liste (klein geschrieben, nur brauchbare, ohne Dubletten).
#[tauri::command]
#[specta::specta]
pub fn change_meeting_self_emails_setting(
    app: tauri::AppHandle,
    emails: Vec<String>,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.meeting_self_emails = crate::managers::meetings::mail::normalize_self_emails(&emails);
    crate::settings::write_settings(&app, settings);
    Ok(())
}

// M5-P5b: Aufnahme aus einem Kalendertermin.

/// Startet eine Aufnahme mit dem Bezug zu einem Termin: Titel = was der Nutzer
/// eingegeben hat, sonst der Termintitel; Vorlage = die der letzten Besprechung
/// derselben Serie (UID), sonst die Standardvorlage; danach Verknuepfung und
/// Teilnehmenden-Schnappschuss. `meetings_start` bleibt unveraendert.
///
/// `link_mode`: `prompt` (Hinweisfenster, Terminkarte; Standard) oder `auto`
/// (Titelvorschlag der Aufnahmekarte). `app_key` gehoert der Erkennung (P5c) und
/// wird bis dahin nicht gelesen. Ohne bestaetigte Einwilligung startet nichts
/// (`consent_required` vom Recorder). Fehler NACH dem Start (Verknuepfung,
/// Vorlage) kippen die laufende Aufnahme nicht; sie stehen im Log.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn meetings_start_from_event(
    app: tauri::AppHandle,
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    store: State<'_, Arc<MeetingStore>>,
    event_key: Option<String>,
    app_key: Option<String>,
    consent_confirmed: bool,
    capture_system: bool,
    title: Option<String>,
    link_mode: Option<String>,
) -> Result<Meeting, String> {
    use crate::managers::calendar::service::{finish_start, plan_start};
    let _ = app_key; // P5c
    let linked_by = match link_mode.as_deref() {
        Some("auto") => "auto",
        _ => "prompt",
    };
    let recorder = Arc::clone(&recorder);
    let store = Arc::clone(&store);
    let meeting = tauri::async_runtime::spawn_blocking(move || {
        let plan = plan_start(&store, event_key.as_deref(), title.as_deref())?;
        let meeting = recorder.start(plan.title.clone(), consent_confirmed, capture_system)?;
        let now = chrono::Utc::now().timestamp_millis();
        for problem in finish_start(&store, &meeting.id, &plan, linked_by, now) {
            log::warn!("meetings_start_from_event: {problem}");
        }
        Ok::<Meeting, String>(meeting)
    })
    .await
    .map_err(|e| format!("meetings_start_from_event panicked: {e}"))??;
    crate::meeting_prompt::close(&app);
    Ok(meeting)
}
