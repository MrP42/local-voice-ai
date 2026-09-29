//! Duenne Command-Huelle ueber den KI-Notizen-Motor (M1, P1b; Muster
//! `commands/meetings.rs`). Die Logik liegt in
//! `managers::meetings::notes::enhance`; hier stehen nur Argumente,
//! Einstellungen, das Fortschrittsereignis und das Abbild der Fehler auf
//! Ereignis-Codes.
//!
//! Datenschutz (D9): weder Notiz- noch Transkript- noch Anweisungstext
//! gelangen ins Log; geloggt werden nur Codes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::managers::meetings::llm_call::resolve_provider_coded;
use crate::managers::meetings::notes::enhance::{
    self, check_recording_conflict, event_code, EnhanceError,
};
use crate::managers::meetings::notes::model::EnhancedNotes;
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::store::{MeetingDocument, MeetingStore};

/// Ereignis des KI-Notizen-Laufs. `code` ist einer von `no_provider`,
/// `no_model`, `memory_low`, `recording_active`, `enhance_busy`,
/// `no_transcript`, `llm_failed`, `meeting_not_finished`; die Oberflaeche
/// uebersetzt ihn (Muster `MeetingEvent::Error`).
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeetingNotesEvent {
    Progress {
        meeting_id: String,
        step: u32,
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

fn emit_failed(app: &AppHandle, meeting_id: &str, err: &str) {
    let code = event_code(err);
    log::warn!("KI-Notizen fehlgeschlagen: {code}");
    let _ = MeetingNotesEvent::Failed {
        meeting_id: meeting_id.to_string(),
        code: code.to_string(),
    }
    .emit(app);
}

fn emit_done(app: &AppHandle, document: &MeetingDocument) {
    let _ = MeetingNotesEvent::Done {
        meeting_id: document.meeting_id.clone(),
        document_id: document.id.clone(),
    }
    .emit(app);
}

/// Laeuft eine Aufnahme, waehrend der aktive Anbieter lokal ist? Ein
/// unvollstaendiger Anbieter meldet der Motor selbst (`no_provider`).
fn recording_conflict(
    settings: &crate::settings::AppSettings,
    recording_active: bool,
) -> Result<(), EnhanceError> {
    match resolve_provider_coded(settings) {
        Ok((provider, _, _)) => {
            check_recording_conflict(crate::managers::llm::is_local(&provider), recording_active)
        }
        Err(_) => Ok(()),
    }
}

/// KI-Notizen erzeugen und dabei `MeetingNotesEvent` senden (Fortschritt,
/// Ende, Fehler). Gemeinsamer Weg fuer den Knopf und den Auto-Lauf nach dem
/// Stopp (P1f): dort ist `recording_active = false`.
pub async fn enhance_and_notify(
    app: &AppHandle,
    store: Arc<MeetingStore>,
    recording_active: bool,
    meeting_id: &str,
    template_id: Option<&str>,
) -> Result<MeetingDocument, String> {
    let settings = crate::settings::get_settings(app);
    if let Err(e) = recording_conflict(&settings, recording_active) {
        let message = e.to_string();
        emit_failed(app, meeting_id, &message);
        return Err(message);
    }
    let progress_app = app.clone();
    let progress_id = meeting_id.to_string();
    let result = enhance::enhance_meeting(
        &settings,
        store,
        meeting_id,
        template_id,
        move |step, total| {
            let _ = MeetingNotesEvent::Progress {
                meeting_id: progress_id.clone(),
                step,
                total,
            }
            .emit(&progress_app);
        },
    )
    .await;
    match &result {
        Ok(document) => emit_done(app, document),
        Err(message) => emit_failed(app, meeting_id, message),
    }
    result
}

// M1-P1f: Auto-Lauf nach dem Live-Transkript ---------------------------------

/// Warum der Auto-Lauf nicht startet. Kein Fehler: der Nutzer kann die
/// KI-Notizen jederzeit von Hand erzeugen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoEnhanceSkip {
    /// Einstellung `meeting_auto_enhance` ist aus.
    Disabled,
    /// Import und Untertitel loesen nie aus (M1, Entscheidung E2).
    NotLive,
    /// Kein (vollstaendiger) Anbieter; der Code ist `no_provider` | `no_model`.
    NoProvider(&'static str),
    /// Eine neue Aufnahme laeuft schon und der Anbieter ist lokal.
    RecordingActive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoEnhanceDecision {
    Run,
    Skip(AutoEnhanceSkip),
}

/// Reine Entscheidung, ob nach `TranscriptFinal` KI-Notizen entstehen.
/// `source` ist `Meeting::source` (`live` | `import` | `subtitle`).
/// Reihenfolge: Einstellung, Herkunft, Anbieter, Aufnahme.
pub fn auto_enhance_decision(
    settings: &crate::settings::AppSettings,
    source: &str,
    recording_active: bool,
) -> AutoEnhanceDecision {
    use AutoEnhanceDecision::{Run, Skip};
    if !settings.meeting_auto_enhance {
        return Skip(AutoEnhanceSkip::Disabled);
    }
    if source != "live" {
        return Skip(AutoEnhanceSkip::NotLive);
    }
    match resolve_provider_coded(settings) {
        Err(e) => Skip(AutoEnhanceSkip::NoProvider(e.code)),
        Ok((provider, _, _)) => {
            if check_recording_conflict(crate::managers::llm::is_local(&provider), recording_active)
                .is_err()
            {
                Skip(AutoEnhanceSkip::RecordingActive)
            } else {
                Run
            }
        }
    }
}

/// Der Hinweis "kein Anbieter" erscheint hoechstens einmal je Programmlauf:
/// wer keinen Anbieter eingerichtet hat, soll nicht nach jeder Besprechung
/// wieder dieselbe Meldung sehen.
static NO_PROVIDER_HINT_SHOWN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn first_no_provider_hint(flag: &std::sync::atomic::AtomicBool) -> bool {
    !flag.swap(true, std::sync::atomic::Ordering::AcqRel)
}

/// Reagiert auf `TranscriptFinal`: startet die KI-Notizen im Hintergrund.
/// Laeuft im Backend, damit es auch ohne offenes Fenster klappt. Fehler
/// erscheinen nur als `MeetingNotesEvent::Failed`; das Transkript bleibt
/// unberuehrt. Kein Notiz-, Transkript- oder Ausgabetext im Log (D9).
fn on_transcript_final(app: &AppHandle, meeting_id: String) {
    use tauri::Manager;
    let Some(store) = app.try_state::<Arc<MeetingStore>>() else {
        return;
    };
    let store = Arc::clone(&store);
    let recording_active = app
        .try_state::<Arc<MeetingRecorderManager>>()
        .map(|r| r.is_recording())
        .unwrap_or(false);
    let source = match store.get_meeting(&meeting_id) {
        Ok(Some(meeting)) => meeting.source,
        _ => return,
    };
    let settings = crate::settings::get_settings(app);
    match auto_enhance_decision(&settings, &source, recording_active) {
        AutoEnhanceDecision::Run => {}
        AutoEnhanceDecision::Skip(AutoEnhanceSkip::NoProvider(code)) => {
            log::info!("KI-Notizen: Auto-Lauf uebersprungen ({code})");
            if first_no_provider_hint(&NO_PROVIDER_HINT_SHOWN) {
                let _ = MeetingNotesEvent::Failed {
                    meeting_id,
                    code: code.to_string(),
                }
                .emit(app);
            }
            return;
        }
        AutoEnhanceDecision::Skip(AutoEnhanceSkip::RecordingActive) => {
            emit_failed(app, &meeting_id, "recording_active");
            return;
        }
        AutoEnhanceDecision::Skip(reason) => {
            log::debug!("KI-Notizen: Auto-Lauf uebersprungen ({reason:?})");
            return;
        }
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Vorlage der Besprechung (aus `meetings_set_template`); `None` faellt
        // im Motor auf die Standardvorlage zurueck.
        let template = store.meeting_template_id(&meeting_id).ok().flatten();
        // Fehler kommen bereits als Ereignis an; hier nur der Code im Log.
        if let Err(message) = enhance_and_notify(
            &app,
            store,
            recording_active,
            &meeting_id,
            template.as_deref(),
        )
        .await
        {
            log::info!("KI-Notizen: Auto-Lauf endete mit {}", event_code(&message));
        }
    });
}

/// Haengt den Auto-Lauf an `MeetingEvent::TranscriptFinal` (einmal beim
/// Start, nachdem Store und Recorder verwaltet werden).
pub fn register_auto_enhance(app: &AppHandle) {
    use crate::managers::meetings::recorder::MeetingEvent;
    let handle = app.clone();
    MeetingEvent::listen_any(app, move |event| {
        if let MeetingEvent::TranscriptFinal { meeting_id, .. } = event.payload {
            on_transcript_final(&handle, meeting_id);
        }
    });
}

/// Erzeugt KI-Notizen fuer eine fertige Besprechung aus Notizblock,
/// Transkript und Vorlage (`None` = Vorlage der Besprechung, sonst die
/// Standardvorlage) und legt sie als neue Version ab. Fehler tragen einen
/// Code als Praefix (`no_provider`, `enhance_busy`, ...).
#[tauri::command]
#[specta::specta]
pub async fn meeting_notes_enhance(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    meeting_id: String,
    template_id: Option<String>,
) -> Result<MeetingDocument, String> {
    let store = Arc::clone(&store);
    enhance_and_notify(
        &app,
        store,
        recorder.is_recording(),
        &meeting_id,
        template_id.as_deref(),
    )
    .await
}

/// Wendet eine Freitext-Anweisung auf eine Version der KI-Notizen an. Die
/// eigenen Eintraege des Nutzers bleiben unveraendert; das Ergebnis ist eine
/// neue Version.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notes_apply_instruction(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    document_id: String,
    instruction: String,
) -> Result<MeetingDocument, String> {
    let store = Arc::clone(&store);
    let settings = crate::settings::get_settings(&app);
    let meeting_id = store
        .get_document(&document_id)
        .ok()
        .flatten()
        .map(|d| d.meeting_id)
        .unwrap_or_default();
    if let Err(e) = recording_conflict(&settings, recorder.is_recording()) {
        let message = e.to_string();
        emit_failed(&app, &meeting_id, &message);
        return Err(message);
    }
    let result = enhance::apply_instruction(&settings, store, &document_id, &instruction).await;
    match &result {
        Ok(document) => emit_done(&app, document),
        Err(message) => emit_failed(&app, &meeting_id, message),
    }
    result
}

/// Speichert eine handbearbeitete Fassung in die bestehende Version.
/// `expected_updated_at` ist der Stempel, den die Oberflaeche geladen hat;
/// weicht er ab (zweites Fenster, neuer Lauf), scheitert der Aufruf mit
/// `stale_document`. Liefert den neuen Stempel.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notes_update_enhanced(
    store: State<'_, Arc<MeetingStore>>,
    document_id: String,
    notes: EnhancedNotes,
    expected_updated_at: i64,
) -> Result<i64, String> {
    enhance::update_enhanced(&store, &document_id, notes, expected_updated_at)
}

/// Die KI-Notizen als Markdown, zum Speichern oder Kopieren (die Ausgabe
/// selbst schreibt `meetings_export_document`).
#[tauri::command]
#[specta::specta]
pub async fn meeting_notes_markdown(
    store: State<'_, Arc<MeetingStore>>,
    document_id: String,
) -> Result<String, String> {
    enhance::markdown_for(&store, &document_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;

    #[test]
    fn the_event_serializes_with_a_kind_tag() {
        let progress = serde_json::to_value(MeetingNotesEvent::Progress {
            meeting_id: "m".into(),
            step: 2,
            total: 5,
        })
        .unwrap();
        assert_eq!(
            progress,
            serde_json::json!({"kind": "progress", "meeting_id": "m", "step": 2, "total": 5})
        );
        let done = serde_json::to_value(MeetingNotesEvent::Done {
            meeting_id: "m".into(),
            document_id: "d".into(),
        })
        .unwrap();
        assert_eq!(done["kind"], "done");
        let failed = serde_json::to_value(MeetingNotesEvent::Failed {
            meeting_id: "m".into(),
            code: "memory_low".into(),
        })
        .unwrap();
        assert_eq!(
            failed,
            serde_json::json!({"kind": "failed", "meeting_id": "m", "code": "memory_low"})
        );
    }

    #[test]
    fn a_recording_blocks_only_the_local_provider() {
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "local".into();
        settings
            .post_process_models
            .insert("local".into(), "irgendein-modell".into());
        assert_eq!(
            recording_conflict(&settings, true).unwrap_err().code,
            "recording_active"
        );
        assert!(recording_conflict(&settings, false).is_ok());

        settings.post_process_provider_id = "custom".into();
        settings
            .post_process_models
            .insert("custom".into(), "modell".into());
        assert!(
            recording_conflict(&settings, true).is_ok(),
            "entfernter Anbieter: erlaubt"
        );

        // Ohne Anbieter meldet der Motor den Fehler selbst.
        settings.post_process_provider_id = "gibt-es-nicht".into();
        assert!(recording_conflict(&settings, true).is_ok());
    }

    fn provider_settings(local: bool) -> crate::settings::AppSettings {
        let mut settings = get_default_settings();
        let id = if local { "local" } else { "custom" };
        settings.post_process_provider_id = id.into();
        settings
            .post_process_models
            .insert(id.into(), "modell".into());
        settings
    }

    #[test]
    fn auto_enhance_runs_for_a_live_meeting_with_a_provider() {
        let settings = provider_settings(true);
        assert_eq!(
            auto_enhance_decision(&settings, "live", false),
            AutoEnhanceDecision::Run
        );
    }

    #[test]
    fn auto_enhance_respects_the_setting() {
        let mut settings = provider_settings(true);
        settings.meeting_auto_enhance = false;
        assert_eq!(
            auto_enhance_decision(&settings, "live", false),
            AutoEnhanceDecision::Skip(AutoEnhanceSkip::Disabled)
        );
    }

    #[test]
    fn auto_enhance_never_runs_for_imports() {
        let settings = provider_settings(true);
        for source in ["import", "subtitle"] {
            assert_eq!(
                auto_enhance_decision(&settings, source, false),
                AutoEnhanceDecision::Skip(AutoEnhanceSkip::NotLive),
                "{source}"
            );
        }
    }

    #[test]
    fn auto_enhance_needs_a_complete_provider() {
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "gibt-es-nicht".into();
        assert_eq!(
            auto_enhance_decision(&settings, "live", false),
            AutoEnhanceDecision::Skip(AutoEnhanceSkip::NoProvider("no_provider"))
        );
        // Anbieter ohne Modell: ebenfalls kein Lauf, eigener Code.
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "custom".into();
        settings.post_process_models.remove("custom");
        assert_eq!(
            auto_enhance_decision(&settings, "live", false),
            AutoEnhanceDecision::Skip(AutoEnhanceSkip::NoProvider("no_model"))
        );
    }

    #[test]
    fn auto_enhance_waits_for_a_running_recording_only_with_a_local_provider() {
        assert_eq!(
            auto_enhance_decision(&provider_settings(true), "live", true),
            AutoEnhanceDecision::Skip(AutoEnhanceSkip::RecordingActive)
        );
        assert_eq!(
            auto_enhance_decision(&provider_settings(false), "live", true),
            AutoEnhanceDecision::Run,
            "entfernter Anbieter konkurriert nicht um die Maschine"
        );
    }

    #[test]
    fn the_no_provider_hint_shows_once() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        assert!(first_no_provider_hint(&flag));
        assert!(!first_no_provider_hint(&flag));
        assert!(!first_no_provider_hint(&flag));
    }
}
