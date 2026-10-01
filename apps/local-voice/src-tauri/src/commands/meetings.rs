//! Thin command shell over `MeetingRecorderManager` and `MeetingStore`
//! (pattern: `commands/history.rs`). No logic beyond argument shuffling and
//! error mapping lives here.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;

use crate::managers::meetings::import::{import_media_file, import_subtitle_file_into};
use crate::managers::meetings::job;
use crate::managers::meetings::queue::{self, ImportQueue};
use crate::managers::meetings::minutes::latest_minutes_file;
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::retention::delete_audio_files;
use crate::managers::meetings::retranscribe::retranscribe_meeting;
use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::meetings::store::{Meeting, MeetingDocument, MeetingStore, StoredSegment};
use crate::managers::transcription::TranscriptionManager;

/// Starting touches audio hardware and can block for seconds (loopback
/// start-up), hence `spawn_blocking` rather than running on the command task.
///
/// G1 (#70): mit `target_meeting_id` nimmt die Aufnahme in einem vorhandenen
/// LEEREN Eintrag auf (Notizen, Projekte und Id bleiben); ist das Ziel nicht
/// (mehr) leer, kommt `target_not_empty`. Die Einwilligung gilt in jedem Fall.
#[tauri::command]
#[specta::specta]
pub async fn meetings_start(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    title: String,
    consent_confirmed: bool,
    capture_system: bool,
    target_meeting_id: Option<String>,
) -> Result<Meeting, String> {
    let recorder = Arc::clone(&recorder);
    tauri::async_runtime::spawn_blocking(move || {
        recorder.start_into(
            title,
            consent_confirmed,
            capture_system,
            target_meeting_id.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("meetings_start panicked: {e}"))?
}

/// G1 (#70): legt einen leeren Eintrag an, ein Notizblock ohne Audio und ohne
/// Quelle, auf Wunsch gleich im Projekt `folder_id`. `title` ist der
/// vorgeschlagene Titel (die Oberflaeche liefert den lokalisierten). Aufnahme,
/// Datei und YouTube-Link fuellen den Eintrag spaeter (`target_meeting_id` der
/// jeweiligen Befehle). Fehler: `title_empty`, `title_too_long`, `folder_not_found`.
#[tauri::command]
#[specta::specta]
pub async fn meetings_create_empty(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    title: String,
    folder_id: Option<String>,
) -> Result<Meeting, String> {
    let store = Arc::clone(&store);
    let meeting = tauri::async_runtime::spawn_blocking(move || {
        store.create_empty_meeting(&title, folder_id.as_deref())
    })
    .await
    .map_err(|e| format!("meetings_create_empty panicked: {e}"))?
    .map_err(|e| e.to_string())?;
    indexer::submit(&app, IndexJob::Meeting(meeting.id.clone())); // M4-P4b
    Ok(meeting)
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
    queue: State<'_, Arc<ImportQueue>>,
    meeting_id: String,
) -> Result<(), String> {
    // P8a: eine laufende Verarbeitung (Import, Enddurchlauf, Notizen ...) endet,
    // bevor ihre Audiodateien verschwinden. Ohne Auftrag ist das ein leerer Aufruf.
    let _ = job::global().stop(&meeting_id);
    // U7: aus der Warteschlange nehmen (ein laufender Import endet, ein wartender
    // wird nie mehr gestartet).
    queue.forget(&meeting_id);
    let paths = store
        .soft_delete_meeting(&meeting_id)
        .map_err(|e| e.to_string())?;
    indexer::submit(&app, IndexJob::Deleted(meeting_id)); // M4-P4b
    delete_audio_files(&paths);
    Ok(())
}

/// Generates the minutes for a finished meeting, following a template, and
/// stores them as a new document version. `template_id`: a template id, `"auto"`
/// (chosen by content) or `None` (the meeting's own choice, else the standard
/// template). The meeting status stays untouched — a failed generation leaves a
/// 'ready' meeting 'ready' and only returns the error. One run per meeting: a
/// second start is refused with `minutes_busy` (P1k, B14).
#[tauri::command]
#[specta::specta]
pub async fn meetings_generate_minutes(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    template_id: Option<String>,
) -> Result<MeetingDocument, String> {
    let store = Arc::clone(&store);
    super::meeting_minutes::generate_and_notify(&app, store, &meeting_id, template_id.as_deref())
        .await
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
/// meeting. G1 (#70): mit `target_meeting_id` fuellt die Datei einen vorhandenen
/// LEEREN Eintrag (Titel, Projekte und Notizen bleiben; der Titel wird nur
/// ersetzt, solange er der vorgeschlagene ist) und kehrt mit dessen Id zurueck;
/// ist das Ziel nicht (mehr) leer, kommt `target_not_empty`.
///
/// U7: Audio und Video werden in die Import-Warteschlange gestellt und der
/// Befehl kehrt SOFORT mit der ID der neuen Besprechung (Status `queued`)
/// zurueck, statt bis zum Ende der Transkription zu laufen: weitere Dateien
/// lassen sich jederzeit hinzufuegen, sie laufen in der Reihenfolge des
/// Hinzufuegens. Untertitel (VTT/SRT) brauchen keine Transkription und sind
/// sofort fertig. Fortschritt, Position und Ende kommen ueber
/// `MeetingEvent::Progress`/`State` und `ImportQueueEvent`.
#[tauri::command]
#[specta::specta]
pub async fn meetings_import_file(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    transcription: State<'_, Arc<TranscriptionManager>>,
    queue: State<'_, Arc<ImportQueue>>,
    path: String,
    consent_confirmed: bool,
    target_meeting_id: Option<String>,
) -> Result<String, String> {
    let path = PathBuf::from(path);
    let consent_at = consent_confirmed.then(|| chrono::Utc::now().timestamp());
    if queue::is_subtitle(&path) {
        let store = Arc::clone(&store);
        if target_meeting_id.is_some() {
            let title = queue::title_from_path(&path);
            return tauri::async_runtime::spawn_blocking(move || {
                import_subtitle_file_into(
                    &store,
                    &title,
                    &path,
                    consent_at,
                    target_meeting_id.as_deref(),
                )
            })
            .await
            .map_err(|e| format!("meetings_import_file panicked: {e}"))?;
        }
        let transcription = Arc::clone(&transcription);
        return import_media_file(&app, store, transcription, path, consent_confirmed).await;
    }
    let source = path
        .to_str()
        .ok_or_else(|| "import_path_invalid".to_string())?
        .to_string();
    let title = queue::title_from_path(&path);
    match target_meeting_id {
        Some(target) => queue.enqueue_into(&target, &title, &source, consent_at),
        None => queue.enqueue(&title, &source, consent_at),
    }
    .map(|meeting| meeting.id)
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

/// Erzeugt den Follow-up-Entwurf (B13): eigener Prompt über KI-Notizen, eigene
/// Notizen und Protokoll (soweit vorhanden), sonst das Transkript; das Recipe
/// "Follow-up-E-Mail an ..." ist die Vorlage. Bewusst NICHT über den Chat: der
/// hat eine strenge Belegpflicht und antwortet bei Aufnahmen ohne Beschluss
/// "nicht gefunden". Gleiche Sperren wie der Chat (ein Lauf gleichzeitig, kein
/// lokales CPU-Modell während einer Aufnahme, KI-Notizen haben Vorrang).
/// Empfänger sind die Teilnehmenden ohne die eigene Person (leer, wenn keine
/// bekannt). Fehlercodes: die des Chats (`no_provider`, `no_model`,
/// `memory_low`, `recording_active_cpu`, `chat_busy`, `llm_failed`,
/// `meeting_not_found`, `store_failed`) sowie `followup_no_content` (keine
/// Grundlage, ein neuer Versuch hilft nicht) und `followup_empty` (das Modell
/// lieferte auch im zweiten Versuch keinen Text).
#[tauri::command]
#[specta::specta]
pub async fn meeting_followup_draft(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    meeting_id: String,
) -> Result<crate::managers::meetings::mail::MailDraft, String> {
    use crate::managers::meetings::chat::controller::{running_flag, CancelFlag, ChatEnv};
    use crate::managers::meetings::followup::{draft_guarded, RunLimits};
    use crate::managers::meetings::mail::participant_recipients;
    let settings = crate::settings::get_settings(&app);
    let (recipients, addressee) = {
        let store = Arc::clone(&store);
        let id = meeting_id.clone();
        let own = settings.meeting_self_emails.clone();
        tauri::async_runtime::spawn_blocking(move || {
            store
                .get_meeting(&id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "meeting_not_found".to_string())?;
            let recipients =
                participant_recipients(&store, &id, &own).map_err(|e| e.to_string())?;
            let addressee = if recipients.names.is_empty() {
                "die Teilnehmenden".to_string()
            } else {
                recipients.names.join(", ")
            };
            Ok::<_, String>((recipients, addressee))
        })
        .await
        .map_err(|e| e.to_string())??
    };
    let local = crate::managers::meetings::llm_call::resolve_provider_coded(&settings)
        .map(|(provider, _, _)| crate::managers::llm::is_local(&provider))
        .unwrap_or(false);
    let env = ChatEnv {
        ctx_tokens: crate::managers::llm::DEFAULT_CONTEXT_TOKENS,
        backend_cpu: local && crate::commands::meeting_chat::local_backend_is_cpu(&app).await,
        recording_active: recorder.is_recording(),
        now: chrono::Utc::now().timestamp(),
        cancel: CancelFlag::default(),
        enhance_running: Arc::new(crate::commands::meeting_chat::enhance_running),
    };
    draft_guarded(
        running_flag(),
        &settings,
        Arc::clone(&store),
        &meeting_id,
        &addressee,
        recipients.emails,
        &env,
        RunLimits::default(),
    )
    .await
    .map_err(|e| {
        log::warn!("Follow-up fehlgeschlagen: {}", e.code);
        e.to_string()
    })
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
/// Vorlage) kippen die laufende Aufnahme nicht; sie stehen im Log. G1 (#70):
/// `target_meeting_id` wie bei `meetings_start` (Aufnahme in einen leeren Eintrag).
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
    target_meeting_id: Option<String>,
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
        let meeting = recorder.start_into(
            plan.title.clone(),
            consent_confirmed,
            capture_system,
            target_meeting_id.as_deref(),
        )?;
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
