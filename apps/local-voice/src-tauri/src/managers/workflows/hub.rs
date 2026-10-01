//! Anwendungsseite der Workflow-Engine (B2): haengt Engine, Ausloeser und Einwilligungsweg
//! an die laufende App.
//!
//! Was hier passiert, und was nicht:
//! - **Start** (`WorkflowHub::start`, aus `initialize_core_logic`, nur im normalen Start, nie
//!   in den Headless-Modi): Engine auf der Datenbank der Besprechungen, Traeger der Aufnahme
//!   anlegen (`recording::ensure_carrier`), Bausteine `recording.start`/`recording.stop`
//!   einsetzen, Beobachter fuer wartende Laeufe, Arbeiter- und Herzschlag-Thread
//!   (`Engine::spawn`), Hoerer an den Besprechungsereignissen.
//! - **Takt** (`on_tick`): wird von `CalendarService::remind_tick` gerufen, also vom
//!   Takt der Erinnerung alle 15 s. Es gibt hier keinen eigenen Zeitgeber (AK3). Pro Takt:
//!   Kalender-Ausloeser, Zeitplan, faellige Enden von Aufnahmen, offene Bitten um
//!   Einwilligung zeigen.
//! - **Ordner und Kanaele** (B3): `trigger::folder` und `trigger::youtube_channel` haengen am
//!   selben Takt (`on_tick`), laufen aber NICHT auf dessen Thread: Hashen grosser Dateien und ein
//!   Netzabruf duerfen die Erinnerungen nicht aufhalten. Je Art ein kurzlebiger Thread, hoechstens
//!   einer zugleich (`State::try_begin`), und nur, wenn ein eingeschalteter Ablauf mit dem
//!   Ausloeser existiert (sonst liest nichts den Datentraeger und nichts geht ins Netz).
//! - **Import-Bausteine und Tor** (B3): `import_app` setzt `meeting.import` und
//!   `youtube.add_source` ein und gibt der Engine ein Tor fuer schwere Schritte, das Aufnahme und
//!   Import-Warteschlange kennt.
//! - **Ende** (`shutdown`, bei `RunEvent::Exit`): setzt das Stoppzeichen und wartet hoechstens
//!   3 s. Steckt der Arbeiter in einem langen Schritt, bleibt er zurueck und stirbt mit dem
//!   Prozess; der Lauf wird beim naechsten Start ueber den abgelaufenen Mietvertrag
//!   fortgesetzt (B1: `recover_run`), ohne doppelte Aussenwirkung. Laeufe werden beim
//!   Beenden NICHT abgebrochen.
//! - **Einwilligung**: wartet ein Lauf auf die Freigabe der Aufnahme, zeigt das
//!   Hinweisfenster die Bitte (`show_pending_consents`): sofort, wenn die Engine den Lauf
//!   parkt (Beobachter), und als Sicherheitsnetz bei jedem Takt. Jede Bitte erscheint
//!   hoechstens einmal (gemerkt) und nur, solange sie juenger als 10 Minuten ist; was
//!   liegenbleibt, steht weiter in der Freigabeliste und verfaellt nach einer Stunde.
//!
//! Die Entscheidungen selbst stehen in reinen, getesteten Modulen (`trigger::*`,
//! `recording`, `consent`); diese Datei ist der Kleber und braucht den `AppHandle`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::commands::meeting_enhance::MeetingNotesEvent;
use crate::commands::meeting_minutes::MinutesEvent;
use crate::managers::calendar::reminder::distinct_attendees;
use crate::managers::meetings::recorder::{MeetingEvent, MeetingRecorderManager};
use crate::managers::meetings::store::MeetingStore;

use super::app_actions;
use super::app_services::AppServicesImpl;
use super::consent;
use super::engine::{Clock, Engine, EngineConfig, EngineHandle, EngineObserver, SystemClock};
use super::import_app;
use super::integration_actions;
use super::recording::{
    self, CurrentRecording, RecordingControl, StartRequest, StartedRecording, StopSchedule,
};
use super::trigger::calendar as calendar_trigger;
use super::trigger::enabled_with;
use super::trigger::folder as folder_trigger;
use super::trigger::meeting_events::{self, MeetingInfo, Stage};
use super::trigger::schedule as schedule_trigger;
use super::trigger::youtube_channel as channel_trigger;

/// So lange wartet `shutdown` auf die Arbeiter.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);
/// Aelter als das zeigt das Fenster eine Bitte nicht mehr von selbst.
const CONSENT_SHOW_MAX_AGE_MS: i64 = 10 * 60_000;
const MAX_SHOWN: usize = 256;

pub struct WorkflowHub {
    app: AppHandle,
    engine: Engine,
    db_path: PathBuf,
    handle: Mutex<Option<EngineHandle>>,
    calendar: calendar_trigger::State,
    stops: Arc<StopSchedule>,
    recording: Arc<dyn RecordingControl>,
    shown: Mutex<HashSet<String>>,
    /// B3: Zustand der Ordner-Ausloeser (Stabilitaet) und der Kanal-Ausloeser (Intervall, Ausfall).
    folders: folder_trigger::State,
    channels: channel_trigger::State,
    feed: Arc<dyn channel_trigger::FeedFetcher>,
}

/// Der echte Recorder hinter `RecordingControl`. Auch die Werkzeuge der Agentenbruecke (A8)
/// starten darueber, nach derselben Einwilligung.
pub(crate) struct AppRecording {
    app: AppHandle,
}

impl AppRecording {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn recorder(&self) -> Option<Arc<MeetingRecorderManager>> {
        self.app
            .try_state::<Arc<MeetingRecorderManager>>()
            .map(|r| Arc::clone(&r))
    }

    fn store(&self) -> Option<Arc<MeetingStore>> {
        self.app
            .try_state::<Arc<MeetingStore>>()
            .map(|s| Arc::clone(&s))
    }
}

impl RecordingControl for AppRecording {
    fn is_recording(&self) -> bool {
        self.recorder().is_some_and(|r| r.is_recording())
    }

    fn current(&self) -> Option<CurrentRecording> {
        let recorder = self.recorder()?;
        if !recorder.is_recording() {
            return None;
        }
        let meeting_id = recorder.position_ms().map(|(id, _)| id);
        let started_at_ms = match (&meeting_id, self.store()) {
            (Some(id), Some(store)) => store
                .get_meeting(id)
                .ok()
                .flatten()
                .and_then(|m| m.started_at)
                .map(|s| s.saturating_mul(1_000)),
            _ => None,
        };
        Some(CurrentRecording {
            meeting_id,
            started_at_ms,
        })
    }

    fn start(&self, req: &StartRequest) -> Result<StartedRecording, String> {
        use crate::managers::calendar::service::{finish_start, plan_start};
        let store = self
            .store()
            .ok_or_else(|| "meetings_unavailable".to_string())?;
        let recorder = self
            .recorder()
            .ok_or_else(|| "meetings_unavailable".to_string())?;
        let plan = match plan_start(&store, req.event_key.as_deref(), Some(&req.title)) {
            Ok(p) => p,
            Err(e) => {
                // Der Termin ist aus dem Cache verschwunden: die Aufnahme laeuft trotzdem,
                // nur ohne Terminbezug.
                log::warn!("workflows: Termin fuer die Aufnahme nicht gefunden ({e})");
                plan_start(&store, None, Some(&req.title))?
            }
        };
        let capture_system = crate::settings::get_settings(&self.app).meeting_capture_system;
        // Die Einwilligung hat der Nutzer im Hinweisfenster gegeben: der Baustein ruft
        // dies nur ueber eine eingeloeste Freigabe (`RunCtx::approved`).
        let meeting = recorder.start_into(plan.title.clone(), true, capture_system, None)?;
        let now = chrono::Utc::now().timestamp_millis();
        for problem in finish_start(&store, &meeting.id, &plan, "auto", now) {
            log::warn!("workflows: Aufnahme gestartet, aber: {problem}");
        }
        crate::meeting_prompt::close(&self.app);
        Ok(StartedRecording {
            meeting_id: meeting.id,
            title: meeting.title,
        })
    }

    fn stop(&self) -> Result<String, String> {
        self.recorder()
            .ok_or_else(|| "meetings_unavailable".to_string())?
            .stop()
    }
}

/// Meldungen der Engine -> Hinweisfenster. Haelt nur den `AppHandle` (keinen Zyklus mit dem
/// Hub, der die Engine haelt).
struct PromptObserver {
    app: AppHandle,
}

impl EngineObserver for PromptObserver {
    fn run_awaiting_approval(&self, _run_id: &str) {
        if let Some(hub) = self.app.try_state::<Arc<WorkflowHub>>() {
            hub.show_pending_consents(chrono::Utc::now().timestamp_millis());
        }
    }
}

impl WorkflowHub {
    /// Startet die Engine in der laufenden App. Der Aufrufer verwaltet das Ergebnis
    /// (`app.manage`).
    pub fn start(app: &AppHandle, store: &MeetingStore) -> Arc<Self> {
        let db_path = store.db_path().to_path_buf();
        // B3: das Tor kennt Aufnahme und Import-Warteschlange (QG5).
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let engine = Engine::new(
            db_path.clone(),
            import_app::heavy_gate(app),
            clock,
            EngineConfig::default(),
        );
        let now = chrono::Utc::now().timestamp_millis();
        match store.get_connection() {
            Ok(conn) => match recording::ensure_carrier(&conn, now) {
                Ok(true) => log::info!("workflows: Integration „Automationen“ angelegt"),
                Ok(false) => {}
                Err(e) => log::warn!("workflows: Integration „Automationen“ nicht angelegt: {e}"),
            },
            Err(e) => log::warn!("workflows: Register nicht geoeffnet: {e}"),
        }
        let stops = Arc::new(StopSchedule::default());
        let recording: Arc<dyn RecordingControl> = Arc::new(AppRecording { app: app.clone() });
        recording::install(&engine, recording.clone(), stops.clone());
        import_app::install(&engine, app);
        let services = Arc::new(AppServicesImpl::new(app));
        app_actions::install(&engine, services.clone());
        integration_actions::install(&engine, services);
        engine.set_observer(Arc::new(PromptObserver { app: app.clone() }));
        let handle = engine.spawn();
        register_meeting_listeners(app);
        log::info!("workflows: Engine gestartet");
        Arc::new(Self {
            app: app.clone(),
            engine,
            db_path,
            handle: Mutex::new(Some(handle)),
            calendar: calendar_trigger::State::default(),
            stops,
            recording,
            shown: Mutex::new(HashSet::new()),
            folders: folder_trigger::State::default(),
            channels: channel_trigger::State::default(),
            feed: Arc::new(channel_trigger::HttpFeedFetcher::production()),
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// B7: Dateien, die nur in der Cloud liegen (OneDrive-Platzhalter) und deshalb nicht
    /// verarbeitet werden.
    pub fn cloud_only_files(&self) -> Vec<folder_trigger::CloudFile> {
        self.folders.cloud_only()
    }

    /// B7: Stand der beobachteten YouTube-Kanaele (letzter Abruf, Ausfall).
    pub fn channel_status(&self) -> Vec<channel_trigger::ChannelStatus> {
        self.channels.status()
    }

    /// Merkt das Ende einer Aufnahme vor (A8: auch die Aufnahme eines Agenten hat ein Ende,
    /// damit eine vergessene nicht die Platte fuellt). Der gemeinsame Takt beendet sie.
    pub fn schedule_stop(&self, meeting_id: &str, at_ms: i64) {
        self.stops.add(meeting_id, at_ms);
    }

    /// Beendet die Arbeiter (siehe Moduldoku). Mehrfaches Rufen ist harmlos.
    pub fn shutdown(&self) {
        let handle = self.handle.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(handle) = handle {
            if handle.stop_within(SHUTDOWN_WAIT) {
                log::info!("workflows: Engine beendet");
            } else {
                log::warn!(
                    "workflows: Engine steckt in einem Schritt; sie endet mit der App, der Lauf wird beim naechsten Start fortgesetzt"
                );
            }
        }
    }

    /// Ein Takt des Kalenders (siehe Moduldoku). Gerufen von `CalendarService::remind_tick`.
    pub fn on_tick(&self, store: &MeetingStore, now_ms: i64) {
        let settings = crate::settings::get_settings(&self.app);
        let recording = self.recording.is_recording();
        let mut report = calendar_trigger::on_tick(
            &self.engine,
            store,
            &self.calendar,
            &calendar_trigger::TickInput {
                now_ms,
                recording,
                self_emails: &settings.meeting_self_emails,
            },
        );
        report.absorb(schedule_trigger::on_tick(
            &self.engine,
            now_ms,
            &chrono::Local,
        ));
        if !report.is_empty() {
            log::info!(
                "workflows: Ausloeser: {} gestartet, {} bekannt, {} uebersprungen, {} Fehler",
                report.started.len(),
                report.duplicates,
                report.skipped.len(),
                report.errors.len()
            );
        }
        recording::run_due_stops(&*self.recording, &self.stops, now_ms);
        self.spawn_scans(now_ms);
        self.close_decided_consent_prompt(now_ms);
        self.show_pending_consents(now_ms);
    }

    /// B3: Ordner und Kanaele auf eigenen, kurzlebigen Threads (siehe Moduldoku). Auf dem Takt
    /// selbst geschieht nur die billige Frage, ob es ueberhaupt einen Ablauf dafuer gibt.
    fn spawn_scans(&self, now_ms: i64) {
        let Some(hub) = self
            .app
            .try_state::<Arc<WorkflowHub>>()
            .map(|h| Arc::clone(&h))
        else {
            return;
        };
        let has = |kind: &str| {
            enabled_with(&self.engine, &[kind])
                .map(|a| !a.is_empty())
                .unwrap_or(false)
        };
        if has(folder_trigger::KIND) && !self.folders.is_busy() {
            let hub = Arc::clone(&hub);
            spawn_scan("workflow-folder-scan", move || {
                let report = folder_trigger::on_tick(
                    hub.engine(),
                    &hub.folders,
                    &hub.db_path,
                    &folder_trigger::SystemProbe,
                    now_ms,
                );
                log_report("Ordner", &report);
            });
        }
        if has(channel_trigger::KIND) && !self.channels.is_busy() {
            spawn_scan("workflow-channel-poll", move || {
                let report = channel_trigger::on_tick(
                    hub.engine(),
                    &hub.channels,
                    &*hub.feed,
                    &hub.db_path,
                    now_ms,
                );
                log_report("Kanäle", &report);
            });
        }
    }

    fn conn(&self) -> Option<Connection> {
        crate::managers::meetings::store::open_connection(&self.db_path).ok()
    }

    /// Zeigt die aelteste noch nicht gezeigte, offene Bitte um Einwilligung im
    /// Hinweisfenster. Eine Bitte nach der anderen: laeuft schon eine, wartet die naechste.
    pub fn show_pending_consents(&self, now_ms: i64) {
        if crate::meeting_prompt::consent_approval(&self.app).is_some() {
            return;
        }
        let Some(conn) = self.conn() else { return };
        let pending = match consent::pending(&conn, now_ms) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("workflows: offene Bitten nicht gelesen: {e}");
                return;
            }
        };
        let store = self.app.try_state::<Arc<MeetingStore>>();
        let next = {
            let mut shown = self.shown.lock().unwrap_or_else(|e| e.into_inner());
            if shown.len() > MAX_SHOWN {
                shown.clear();
            }
            let found = pending.into_iter().find(|c| {
                !shown.contains(&c.approval_id) && now_ms - c.created_at <= CONSENT_SHOW_MAX_AGE_MS
            });
            if let Some(c) = &found {
                shown.insert(c.approval_id.clone());
            }
            found
        };
        let Some(c) = next else { return };
        let event = c.event_key.as_deref().and_then(|key| {
            store
                .as_ref()
                .and_then(|s| s.calendar_event(key).ok().flatten())
        });
        crate::meeting_prompt::show_consent(
            &self.app,
            event.clone(),
            event.as_ref().map(distinct_attendees).unwrap_or(0) as u32,
            c.workflow_name.clone(),
            c.title.clone(),
            c.agent,
            c.approval_id,
        );
    }

    /// Wurde die Freigabe anderswo entschieden (Seite Integrationen), verfaellt sie oder
    /// wurde der Lauf abgebrochen, schliesst sich das Hinweisfenster der Bitte.
    fn close_decided_consent_prompt(&self, now_ms: i64) {
        let Some(approval_id) = crate::meeting_prompt::consent_approval(&self.app) else {
            return;
        };
        if let Some(conn) = self.conn() {
            if !consent::is_pending(&conn, &approval_id, now_ms) {
                crate::meeting_prompt::close(&self.app);
            }
        }
    }

    /// Nach einer Entscheidung des Nutzers: Arbeiter wecken, naechste Bitte zeigen.
    pub fn after_decision(&self) {
        self.engine.wake();
        self.show_pending_consents(chrono::Utc::now().timestamp_millis());
    }
}

/// Startet `work` auf einem benannten Thread; ein Fehler beim Start ist nur ein Logeintrag (der
/// naechste Takt versucht es wieder).
fn spawn_scan(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.to_string()).spawn(work) {
        log::warn!("workflows: Thread {name} nicht gestartet: {e}");
    }
}

fn log_report(what: &str, report: &super::trigger::TickReport) {
    if report.is_empty() {
        return;
    }
    log::info!(
        "workflows: {what}: {} gestartet, {} bekannt, {} Hinweise, {} Fehler",
        report.started.len(),
        report.duplicates,
        report.skipped.len(),
        report.errors.len()
    );
    for line in report.skipped.iter().chain(report.errors.iter()) {
        log::info!("workflows: {what}: {line}");
    }
}

// ---------------------------------------------------------------------------
// Besprechungsereignisse
// ---------------------------------------------------------------------------

fn register_meeting_listeners(app: &AppHandle) {
    let handle = app.clone();
    MeetingEvent::listen_any(app, move |event| match event.payload {
        MeetingEvent::TranscriptFinal { meeting_id, .. } => {
            on_meeting(&handle, Stage::Transcript, meeting_id)
        }
        // Die Aufnahme ist zu: `stop()` meldet `processing` (auch die Wiederherstellung).
        MeetingEvent::State {
            meeting_id, status, ..
        } if status == "processing" => on_meeting(&handle, Stage::Recording, meeting_id),
        _ => {}
    });
    let handle = app.clone();
    MeetingNotesEvent::listen_any(app, move |event| {
        if let MeetingNotesEvent::Done { meeting_id, .. } = event.payload {
            on_meeting(&handle, Stage::Notes, meeting_id);
        }
    });
    let handle = app.clone();
    MinutesEvent::listen_any(app, move |event| {
        if let MinutesEvent::Done { meeting_id, .. } = event.payload {
            on_meeting(&handle, Stage::Minutes, meeting_id);
        }
    });
}

/// Ein Besprechungsereignis: nicht auf dem Ereignis-Thread (SQLite), und ohne den Absender
/// zu stoeren.
fn on_meeting(app: &AppHandle, stage: Stage, meeting_id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (Some(hub), Some(store)) = (
            app.try_state::<Arc<WorkflowHub>>(),
            app.try_state::<Arc<MeetingStore>>(),
        ) else {
            return;
        };
        let meeting = match store.get_meeting(&meeting_id) {
            Ok(Some(m)) => m,
            _ => return,
        };
        if stage == Stage::Recording && meeting.source != "live" {
            return;
        }
        let report = meeting_events::on_event(
            hub.engine(),
            stage,
            &MeetingInfo {
                id: meeting.id,
                title: meeting.title,
            },
        );
        if !report.is_empty() {
            log::info!(
                "workflows: Besprechung {} ({}): {} Laeufe gestartet, {} bekannt, {} Fehler",
                meeting_id,
                stage.as_str(),
                report.started.len(),
                report.duplicates,
                report.errors.len()
            );
        }
    });
}
