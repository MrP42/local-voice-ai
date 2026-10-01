//! Hinweisfenster `meeting_prompt` (M5, P5b; `entwurf/m5-m6-kalender-export.md`
//! §3 „Hinweisfenster“): ein kleines Fenster unten rechts, immer im Vordergrund,
//! ohne Fokusraub, das eine Besprechung ankuendigt („Jour fixe beginnt in 1 min“)
//! und die Aufnahme anbietet. Muster `overlay.rs`.
//!
//! Der Zustand liegt im Backend (`MeetingPromptState`), nicht im Ereignis: ein
//! frisch angelegtes Fenster verpasst Ereignisse, die vor seinem Start gesendet
//! wurden. Ablauf: `show` legt den Zustand ab und erzeugt bzw. weckt das Fenster
//! -> die Oberflaeche holt `meeting_prompt_current` -> meldet mit
//! `meeting_prompt_ready(hoehe)` ihre Hoehe -> erst dann wird das Fenster
//! positioniert und gezeigt (kein leeres Aufblitzen).
//!
//! Das Fenster wird einmal angelegt und danach nur versteckt und wieder gezeigt:
//! ein Neuanlegen unter demselben Label kollidiert unter Windows mit dem noch
//! nicht abgeschlossenen Schliessen des alten.
//!
//! Fehlerfaelle: Fenster laesst sich nicht anlegen -> Log, Zustand wird geleert
//! (kein Dauerzustand „offen“, der spaetere Erinnerungen blockiert); nach 3 min
//! ohne Aktion schliesst es sich (`AUTO_CLOSE`), waehrend einer Aufnahme kommt
//! nie ein Erinnerungshinweis (Dienst), ein bereits offener Hinweis wird nicht
//! ueberschrieben.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_specta::Event;

use crate::managers::calendar::model::CalEvent;
use crate::managers::calendar::reminder::distinct_attendees;
use crate::managers::meetings::store::MeetingStore;

/// Label des Fensters (auch in `capabilities/default.json`).
pub const LABEL: &str = "meeting_prompt";
const PAGE: &str = "src/meeting-prompt/index.html";
/// Logische Breite; die Hoehe meldet die Oberflaeche (`meeting_prompt_ready`).
const WIDTH: f64 = 380.0;
const START_HEIGHT: f64 = 170.0;
const MAX_HEIGHT: f64 = 560.0;
/// Abstand zum Rand des Arbeitsbereichs.
const MARGIN: f64 = 16.0;
/// Nach dieser Zeit ohne Aktion schliesst sich das Fenster.
pub const AUTO_CLOSE: Duration = Duration::from_secs(3 * 60);

static PROMPT_SEQ: AtomicU64 = AtomicU64::new(0);

/// Inhalt eines Hinweises.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct MeetingPromptPayload {
    pub prompt_id: String,
    /// `reminder` (Termin steht an); `detected` folgt mit P5c.
    pub kind: String,
    pub event: Option<CalEvent>,
    /// Verschiedene Teilnehmende des Termins (0 ohne Termin).
    pub attendee_count: u32,
    /// Name der erkannten Anwendung (P5c), sonst `None`.
    pub app_label: Option<String>,
    /// B2: nur bei `kind == "workflow_recording"`: ein Ablauf bittet um die Einwilligung
    /// zur Aufnahme. Entschieden wird ueber `meeting_prompt_workflow_decide`, nie ueber
    /// `meetings_start*`: die Aufnahme startet erst, wenn der Ablauf nach der Freigabe
    /// weiterlaeuft.
    #[serde(default)]
    pub workflow: Option<PromptWorkflow>,
}

/// Der Ablauf hinter einer Bitte um Einwilligung (B2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct PromptWorkflow {
    /// Name des Ablaufs („Kundentermin protokollieren“); bei einem Agenten sein Name.
    pub name: String,
    /// Titel des Termins bzw. der Besprechung, wenn der Ausloeser einen hat.
    pub title: Option<String>,
    /// A8: die Bitte kommt von einem externen Agenten (Claude Code, Codex, ein Skript) statt
    /// von einem Ablauf; `name` ist der Name seines Zugangs.
    #[serde(default)]
    pub agent: bool,
}

/// Ereignis an das Fenster: `show` = es liegt ein neuer Hinweis vor (die
/// Oberflaeche holt ihn ueber `meeting_prompt_current`), `close` = zu.
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind")]
pub enum MeetingPromptEvent {
    #[serde(rename = "show")]
    Show { prompt_id: String },
    #[serde(rename = "close")]
    Close,
}

#[derive(Default)]
struct Inner {
    current: Option<MeetingPromptPayload>,
    /// B2: Freigabe, die dieser Hinweis entscheidet (nur bei `workflow_recording`). Sie
    /// steht im Backend und nicht in der Nutzlast: das Fenster kann keine andere
    /// Freigabe entscheiden.
    consent_approval: Option<String>,
}

/// Verwalteter Zustand des Fensters.
#[derive(Default)]
pub struct MeetingPromptState {
    inner: Mutex<Inner>,
}

impl MeetingPromptState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Zeigt gerade ein Hinweis?
pub fn is_open(app: &AppHandle) -> bool {
    app.try_state::<MeetingPromptState>()
        .is_some_and(|s| s.lock().current.is_some())
}

/// Der Hinweis zu einem anstehenden Termin.
pub fn payload_for_event(event: CalEvent) -> MeetingPromptPayload {
    MeetingPromptPayload {
        prompt_id: format!("p{}", PROMPT_SEQ.fetch_add(1, Ordering::Relaxed) + 1),
        kind: "reminder".to_string(),
        attendee_count: distinct_attendees(&event) as u32,
        event: Some(event),
        app_label: None,
        workflow: None,
    }
}

/// B2: Der Hinweis „Ein Ablauf moechte aufnehmen“.
pub fn payload_for_consent(
    event: Option<CalEvent>,
    attendee_count: u32,
    workflow_name: String,
    title: Option<String>,
    agent: bool,
) -> MeetingPromptPayload {
    MeetingPromptPayload {
        prompt_id: format!("p{}", PROMPT_SEQ.fetch_add(1, Ordering::Relaxed) + 1),
        kind: "workflow_recording".to_string(),
        attendee_count,
        event,
        app_label: None,
        workflow: Some(PromptWorkflow {
            name: workflow_name,
            title,
            agent,
        }),
    }
}

pub fn show_event(app: &AppHandle, event: CalEvent) {
    show(app, payload_for_event(event));
}

/// Die Freigabe, die der gerade offene Hinweis entscheidet (B2); `None`, wenn keiner offen
/// ist oder ein anderer Hinweis offen ist.
pub fn consent_approval(app: &AppHandle) -> Option<String> {
    app.try_state::<MeetingPromptState>()
        .and_then(|s| s.lock().consent_approval.clone())
}

/// B2: Zeigt die Bitte um Einwilligung zur Aufnahme. Sie ersetzt eine Erinnerung (derselbe
/// Termin, genauer), aber keine andere Bitte: dann wartet sie (der Aufrufer fragt spaeter
/// wieder).
pub fn show_consent(
    app: &AppHandle,
    event: Option<CalEvent>,
    attendee_count: u32,
    workflow_name: String,
    title: Option<String>,
    agent: bool,
    approval_id: String,
) {
    if consent_approval(app).is_some() {
        return;
    }
    let payload = payload_for_consent(event, attendee_count, workflow_name, title, agent);
    show_with(app, payload, Some(approval_id));
}

/// Legt den Hinweis ab und erzeugt bzw. weckt das Fenster.
pub fn show(app: &AppHandle, payload: MeetingPromptPayload) {
    show_with(app, payload, None);
}

fn show_with(app: &AppHandle, payload: MeetingPromptPayload, consent_approval: Option<String>) {
    let Some(state) = app.try_state::<MeetingPromptState>() else {
        log::warn!("meeting_prompt: state not managed");
        return;
    };
    let prompt_id = payload.prompt_id.clone();
    {
        let mut inner = state.lock();
        inner.current = Some(payload);
        inner.consent_approval = consent_approval;
    }

    match app.get_webview_window(LABEL) {
        Some(_) => {
            let _ = MeetingPromptEvent::Show {
                prompt_id: prompt_id.clone(),
            }
            .emit(app);
        }
        None => {
            if let Err(e) = create_window(app) {
                log::error!("meeting_prompt: window not created: {e}");
                let mut inner = state.lock();
                inner.current = None;
                inner.consent_approval = None;
                return;
            }
            // Die neue Oberflaeche holt den Hinweis selbst beim Start.
        }
    }

    // Automatisch schliessen, sofern in der Zwischenzeit kein anderer kam.
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(AUTO_CLOSE).await;
        let still_current = app.try_state::<MeetingPromptState>().is_some_and(|s| {
            s.lock().current.as_ref().map(|c| c.prompt_id.as_str()) == Some(prompt_id.as_str())
        });
        if still_current {
            close(&app);
        }
    });
}

/// Schliesst den Hinweis: Zustand leeren, Fenster verstecken.
pub fn close(app: &AppHandle) {
    if let Some(state) = app.try_state::<MeetingPromptState>() {
        let mut inner = state.lock();
        inner.current = None;
        inner.consent_approval = None;
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
        let _ = MeetingPromptEvent::Close.emit(app);
    }
}

fn create_window(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(PAGE.into()))
        .title("Local Voice AI")
        .inner_size(WIDTH, START_HEIGHT)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .shadow(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .accept_first_mouse(true)
        .focusable(false)
        .focused(false)
        .visible(false);
    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }
    builder.build().map(|_| ())
}

/// Unten rechts im Arbeitsbereich des Hauptbildschirms (ohne Taskleiste).
fn place(app: &AppHandle, window: &tauri::WebviewWindow, height: f64) {
    let monitor = app
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let w = (WIDTH * scale).round() as i32;
    let h = (height * scale).round() as i32;
    let margin = (MARGIN * scale).round() as i32;
    let x = area.position.x + area.size.width as i32 - w - margin;
    let y = area.position.y + area.size.height as i32 - h - margin;
    let _ = window.set_size(tauri::PhysicalSize::new(w as u32, h as u32));
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

// ---------------------------------------------------------------------------
// Befehle des Fensters
// ---------------------------------------------------------------------------

/// Der aktuelle Hinweis; `None`, wenn keiner offen ist.
#[tauri::command]
#[specta::specta]
pub fn meeting_prompt_current(
    state: State<'_, MeetingPromptState>,
) -> Option<MeetingPromptPayload> {
    state.lock().current.clone()
}

/// Die Oberflaeche hat gerendert und meldet ihre Hoehe (logisch): jetzt wird das
/// Fenster positioniert und ohne Fokus gezeigt.
#[tauri::command]
#[specta::specta]
pub fn meeting_prompt_ready(
    app: AppHandle,
    state: State<'_, MeetingPromptState>,
    height: f64,
) -> Result<(), String> {
    if state.lock().current.is_none() {
        return Ok(()); // inzwischen geschlossen
    }
    let window = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "meeting_prompt_window_missing".to_string())?;
    let height = if height.is_finite() {
        height
    } else {
        START_HEIGHT
    };
    let height = height.clamp(80.0, MAX_HEIGHT);
    place(&app, &window, height);
    let _ = window.show();
    // Nach dem Zeigen erneut setzen: der DPI-Wechsel beim Verschieben auf einen
    // anderen Bildschirm verzieht sonst die erste Platzierung (Muster overlay.rs).
    place(&app, &window, height);
    Ok(())
}

/// Der Nutzer hat entschieden. `later` = nicht jetzt (der Termin wird als
/// verworfen gemerkt und erinnert nicht erneut), `close` = nur schliessen (nach
/// dem Start der Aufnahme). Ein veralteter `prompt_id` wird ignoriert.
#[tauri::command]
#[specta::specta]
pub fn meeting_prompt_dismiss(
    app: AppHandle,
    state: State<'_, MeetingPromptState>,
    store: State<'_, Arc<MeetingStore>>,
    prompt_id: String,
    action: String,
) -> Result<(), String> {
    let current = state.lock().current.clone();
    let Some(current) = current else {
        return Ok(());
    };
    if current.prompt_id != prompt_id {
        return Ok(());
    }
    if action == "later" {
        if let Some(event) = &current.event {
            let now = chrono::Utc::now().timestamp_millis();
            if let Err(e) = store.calendar_mark_dismissed(&event.key, now) {
                log::warn!("meeting_prompt: dismiss not stored: {e}");
            }
        }
    }
    close(&app);
    Ok(())
}

/// B2: Der Nutzer entscheidet die Bitte eines Ablaufs um Einwilligung zur Aufnahme:
/// `approve = true` ist die Einwilligung (das Fenster hat das Haekchen verlangt), `false`
/// das Nein. Entschieden wird die Freigabe, die dieser Hinweis im Backend haelt; die
/// Aufnahme startet erst, wenn der Ablauf danach weiterlaeuft. Fehler: `consent_not_pending`
/// (schon entschieden, verfallen oder der Lauf wurde abgebrochen), `consent_invalid`.
#[tauri::command]
#[specta::specta]
pub fn meeting_prompt_workflow_decide(
    app: AppHandle,
    state: State<'_, MeetingPromptState>,
    store: State<'_, Arc<MeetingStore>>,
    prompt_id: String,
    approve: bool,
) -> Result<(), String> {
    let (current_id, approval) = {
        let inner = state.lock();
        (
            inner.current.as_ref().map(|c| c.prompt_id.clone()),
            inner.consent_approval.clone(),
        )
    };
    if current_id.as_deref() != Some(prompt_id.as_str()) {
        return Err("consent_not_pending".to_string());
    }
    let Some(approval_id) = approval else {
        return Err("consent_invalid".to_string());
    };
    let conn = store
        .get_connection()
        .map_err(|e| format!("store_failed: {e}"))?;
    let now = chrono::Utc::now().timestamp_millis();
    let result = crate::managers::workflows::consent::decide(&conn, &approval_id, approve, now);
    // Entschieden oder nicht mehr offen: der Hinweis ist erledigt.
    close(&app);
    match result {
        Ok(()) => {
            if let Some(hub) = app.try_state::<Arc<crate::managers::workflows::hub::WorkflowHub>>()
            {
                hub.after_decision();
            }
            Ok(())
        }
        Err(e) => Err(e.code().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::calendar::model::Attendee;

    fn event(attendees: Vec<Attendee>) -> CalEvent {
        CalEvent {
            key: "s:u:1".into(),
            source_id: "s".into(),
            uid: "u".into(),
            title: "Jour fixe".into(),
            starts_at: 1,
            ends_at: 2,
            all_day: false,
            cancelled: false,
            location: None,
            join_url: None,
            description: None,
            attendees,
        }
    }

    fn person(email: &str) -> Attendee {
        Attendee {
            email: Some(email.into()),
            name: None,
            organizer: false,
            is_self: false,
            partstat: None,
        }
    }

    #[test]
    fn payload_counts_distinct_attendees_and_ids_are_unique() {
        let a = payload_for_event(event(vec![
            person("a@x.de"),
            person("A@x.de"),
            person("b@x.de"),
        ]));
        let b = payload_for_event(event(vec![]));
        assert_eq!(a.attendee_count, 2, "eine Adresse zaehlt einmal");
        assert_eq!(b.attendee_count, 0);
        assert_ne!(a.prompt_id, b.prompt_id);
        assert_eq!(a.kind, "reminder");
        assert!(a.event.is_some() && a.app_label.is_none());
    }

    #[test]
    fn events_serialize_with_a_kind_tag() {
        let show = serde_json::to_value(MeetingPromptEvent::Show {
            prompt_id: "p1".into(),
        })
        .unwrap();
        assert_eq!(show["kind"], "show");
        assert_eq!(show["prompt_id"], "p1");
        assert_eq!(
            serde_json::to_value(MeetingPromptEvent::Close).unwrap()["kind"],
            "close"
        );
    }
}
