//! Ad-hoc-Erkennung laufender Besprechungen (M5 F16, `entwurf/m5-m6-kalender-export.md`).
//!
//! Die App merkt, wenn ein Meeting-Programm (Teams, Zoom, Webex, Slack, Discord,
//! WhatsApp, Skype, Signal oder ein Browser) das Mikrofon benutzt, und meldet
//! das als `MeetingDetectEvent`. Sie hoert nie mit: gelesen wird nur das
//! Nutzungsprotokoll von Windows (`source`), nie ein Audiostrom. Es gibt einen
//! Hinweis, nie einen automatischen Start.
//!
//! Bausteine: `source` (Registry, austauschbar), `catalog` (welche App ist
//! das), `detector` (reine Regeln). Hier liegen der Watcher-Thread, die
//! Prozessliste fuer die Leichen-Pruefung und der Lauf fuer `--detect-mic`.
//!
//! # Schnittstelle fuer das Hinweisfenster (P5b)
//! - Event `MeetingDetectEvent::{Started{app_key,label,class}, Ended{app_key}}`
//!   (Frontend: `events.meetingDetectEvent`; Rust: `MeetingDetectEvent::listen`).
//! - `current_notice(app)`: der gerade offene Hinweis, falls das Fenster spaeter
//!   entsteht als das Event.
//! - Unterdrueckt ist der Hinweis bereits, wenn eine Aufnahme oder ein Diktat
//!   laeuft; ein Termin (+-15 min) wird vom Fenster daneben gelegt.
//!
//! # Fehlerfaelle
//! - Registry fehlt/gesperrt (Gruppenrichtlinie, kein Windows): der Watcher
//!   meldet es einmal im Log und liest weiter langsam; die Einstellung zeigt
//!   ueber `meeting_detect_available` "nicht verfuegbar".
//! - Absturz eines Programms (`Stop` bleibt 0): Prozesspruefung, kein Hinweis.
//! - Ruhezustand: der Start stammt aus der Registry, nicht aus einer eigenen
//!   Uhr; nach dem Aufwachen gilt der Stand der Registry.
//! - Speicher/CPU: ~2 ms je Durchlauf alle 2 s, eine Prozessliste nur, wenn ein
//!   fertiger Kandidat sie braucht. Bei vollem RAM aendert sich nichts: die
//!   Erkennung haelt keine Daten ausser der Sitzungsliste (einige Bytes).
//! - Kindprozess: keiner.

pub mod catalog;
pub mod detector;
pub mod source;

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

pub use detector::{DetectCtx, DetectEvent, DetectMode, Detector};
pub use source::{MicUsageSource, RegistrySource};

/// Abfrageabstand des Watchers.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Ein offener Hinweis (fuer das Fenster, das nach dem Event entsteht).
#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct DetectNotice {
    pub app_key: String,
    pub label: String,
    /// `meeting_app` | `browser` | `other`
    pub class: String,
}

/// Ereignis an Fenster und Rust-Verbraucher (Muster `MeetingEvent`).
#[derive(Clone, Debug, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeetingDetectEvent {
    Started {
        app_key: String,
        label: String,
        class: String,
    },
    Ended {
        app_key: String,
    },
}

/// Verwalteter Zustand: der offene Hinweis.
#[derive(Default)]
pub struct DetectState {
    open: Mutex<Option<DetectNotice>>,
}

impl DetectState {
    fn apply(&self, event: &DetectEvent) {
        let mut open = self.open.lock().unwrap();
        match event {
            DetectEvent::Started {
                app_key,
                label,
                class,
            } => {
                *open = Some(DetectNotice {
                    app_key: app_key.clone(),
                    label: label.clone(),
                    class: class.to_string(),
                });
            }
            DetectEvent::Ended { app_key } => {
                if open.as_ref().is_some_and(|n| &n.app_key == app_key) {
                    *open = None;
                }
            }
        }
    }

    #[allow(dead_code)] // Verbraucher: das Hinweisfenster (P5b)
    pub fn notice(&self) -> Option<DetectNotice> {
        self.open.lock().unwrap().clone()
    }
}

impl From<&DetectEvent> for MeetingDetectEvent {
    fn from(event: &DetectEvent) -> Self {
        match event {
            DetectEvent::Started {
                app_key,
                label,
                class,
            } => Self::Started {
                app_key: app_key.clone(),
                label: label.clone(),
                class: class.to_string(),
            },
            DetectEvent::Ended { app_key } => Self::Ended {
                app_key: app_key.clone(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Prozessliste (Leichen-Pruefung)
// ---------------------------------------------------------------------------

fn norm_path(p: &Path) -> String {
    p.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_ascii_lowercase()
}

/// Laeuft zu einem Pfad ein Prozess? Die Liste wird nur gelesen, wenn jemand
/// fragt, und hoechstens einmal je Sekunde.
pub struct LiveProcesses {
    sys: RefCell<sysinfo::System>,
    refreshed: Cell<Option<Instant>>,
}

impl Default for LiveProcesses {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveProcesses {
    pub fn new() -> Self {
        Self {
            sys: RefCell::new(sysinfo::System::new()),
            refreshed: Cell::new(None),
        }
    }

    pub fn alive(&self, path: &Path) -> bool {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
        if self
            .refreshed
            .get()
            .is_none_or(|at| at.elapsed() > Duration::from_secs(1))
        {
            self.sys.borrow_mut().refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
            );
            self.refreshed.set(Some(Instant::now()));
        }
        let want = norm_path(path);
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        self.sys
            .borrow()
            .processes()
            .values()
            .any(|p| match p.exe() {
                Some(exe) => norm_path(exe) == want,
                // Geschuetzter Prozess ohne lesbaren Pfad: der Name muss genuegen.
                None => p.name().to_string_lossy().to_ascii_lowercase() == file,
            })
    }
}

/// Schluessel der eigenen EXE in Registry-Form (`np:C:#...#local-voice-ai.exe`).
pub fn own_keys() -> Vec<String> {
    std::env::current_exe()
        .map(|p| vec![catalog::key_of_path(&p)])
        .unwrap_or_default()
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Watcher
// ---------------------------------------------------------------------------

/// Stimmt die Registry-Quelle auf diesem System (fuer die Einstellungszeile)?
pub fn available() -> bool {
    RegistrySource::new().snapshot().is_ok()
}

/// Der offene Hinweis, falls einer offen ist.
pub fn current_notice(app: &tauri::AppHandle) -> Option<DetectNotice> {
    use tauri::Manager;
    app.try_state::<DetectState>().and_then(|s| s.notice())
}

/// Laeuft eine Aufnahme (Besprechung) oder ein Diktat?
fn recording_now(app: &tauri::AppHandle) -> bool {
    use tauri::Manager;
    let meeting = app
        .try_state::<Arc<crate::managers::meetings::recorder::MeetingRecorderManager>>()
        .is_some_and(|r| r.is_recording());
    let dictation = app
        .try_state::<Arc<crate::managers::audio::AudioRecordingManager>>()
        .is_some_and(|a| a.is_recording());
    meeting || dictation
}

/// Startet den Watcher-Thread (einmal, beim Start der App). Unter anderen
/// Systemen als Windows startet er nicht.
pub fn start(app: &tauri::AppHandle) {
    use tauri::Manager;
    use tauri_specta::Event;

    if !cfg!(windows) {
        return;
    }
    app.manage(DetectState::default());
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("meeting-detect".into())
        .spawn(move || {
            let mut source = RegistrySource::new();
            let mut detector = Detector::new();
            let procs = LiveProcesses::new();
            let selfs = own_keys();
            let mut last_error = String::new();
            loop {
                std::thread::sleep(POLL_INTERVAL);
                let settings = crate::settings::get_settings(&app);
                let mode = settings.meeting_detect_mode;
                // Aus: nichts lesen, nur einen offenen Hinweis schliessen.
                let snap = if mode == DetectMode::Off {
                    if detector.open_app().is_none() {
                        continue;
                    }
                    Vec::new()
                } else {
                    match source.snapshot() {
                        Ok(v) => {
                            last_error.clear();
                            v
                        }
                        Err(e) => {
                            if e != last_error {
                                log::warn!("meeting-detect: {e}");
                                last_error = e;
                            }
                            continue;
                        }
                    }
                };
                let alive = |p: &Path| procs.alive(p);
                let ctx = DetectCtx {
                    mode,
                    recording: recording_now(&app),
                    ignored: &settings.meeting_detect_ignored_apps,
                    self_keys: &selfs,
                    process_alive: &alive,
                };
                for event in detector.step(now_unix_ms(), &snap, &ctx) {
                    log::info!("meeting-detect: {event:?}");
                    if let Some(state) = app.try_state::<DetectState>() {
                        state.apply(&event);
                    }
                    let _ = MeetingDetectEvent::from(&event).emit(&app);
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("meeting-detect: Thread nicht gestartet: {e}");
    }
}

// ---------------------------------------------------------------------------
// --detect-mic
// ---------------------------------------------------------------------------

fn class_of(app_key: &str) -> &'static str {
    catalog::kind_name(catalog::classify(app_key))
}

/// Beobachtet `duration` lang die Mikrofonnutzung wie der Watcher und
/// sammelt die Ereignisse. Ohne Einstellungen und ohne Ignorierliste; nur die
/// eigene EXE ist ausgenommen.
pub fn run_loop(
    source: &mut dyn MicUsageSource,
    alive: &dyn Fn(&Path) -> bool,
    mode: DetectMode,
    duration: Duration,
    poll: Duration,
) -> (i32, Value) {
    let started_unix = now_unix_ms();
    let started = Instant::now();
    let mut detector = Detector::new();
    let selfs = own_keys();
    let mut events = Vec::new();
    let mut seen: std::collections::BTreeMap<String, Value> = Default::default();
    let mut polls = 0u32;
    let mut failures = 0u32;
    let mut last_error: Option<String> = None;
    loop {
        match source.snapshot() {
            Ok(snap) => {
                polls += 1;
                let ctx = DetectCtx {
                    mode,
                    recording: false,
                    ignored: &[],
                    self_keys: &selfs,
                    process_alive: alive,
                };
                for u in snap.iter().filter(|u| u.active) {
                    seen.entry(u.app_key.clone()).or_insert_with(|| {
                        json!({
                            "app_key": u.app_key,
                            "class": class_of(&u.app_key),
                            "excluded": Detector::exclusion(u, &ctx),
                            "last_start_unix_ms": source::filetime_to_unix_ms(u.last_start),
                        })
                    });
                }
                let at = now_unix_ms();
                for event in detector.step(at, &snap, &ctx) {
                    let t_s = started.elapsed().as_secs_f64();
                    events.push(match event {
                        DetectEvent::Started {
                            app_key,
                            label,
                            class,
                        } => json!({"kind": "started", "app_key": app_key, "label": label,
                            "class": class, "at_unix_ms": at, "t_s": (t_s * 10.0).round() / 10.0}),
                        DetectEvent::Ended { app_key } => json!({"kind": "ended",
                            "app_key": app_key, "at_unix_ms": at,
                            "t_s": (t_s * 10.0).round() / 10.0}),
                    });
                }
            }
            Err(e) => {
                failures += 1;
                last_error = Some(e);
            }
        }
        if started.elapsed() >= duration {
            break;
        }
        std::thread::sleep(poll.min(duration.saturating_sub(started.elapsed())));
    }
    let failed = polls == 0;
    let payload = json!({
        "mode": "detect_mic",
        "detect_mode": match mode {
            DetectMode::Off => "off",
            DetectMode::MeetingApps => "meeting_apps",
            DetectMode::AllApps => "all_apps",
        },
        "seconds": duration.as_secs_f64(),
        "poll_ms": poll.as_millis() as u64,
        "started_unix_ms": started_unix,
        "polls": polls,
        "failed_polls": failures,
        "events": events,
        "active_seen": seen.into_values().collect::<Vec<_>>(),
        "error": if failed { last_error } else { None },
    });
    (if failed { 1 } else { 0 }, payload)
}

/// Text fuer den Lauf ohne `--json`.
pub fn format_table(payload: &Value) -> String {
    let mut out = String::new();
    for e in payload["events"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{:>6.1} s  {:<8} {}\n",
            e["t_s"].as_f64().unwrap_or(0.0),
            e["kind"].as_str().unwrap_or(""),
            e["label"]
                .as_str()
                .or_else(|| e["app_key"].as_str())
                .unwrap_or("")
        ));
    }
    if out.is_empty() {
        out.push_str("keine Ereignisse\n");
    }
    out
}

/// Einstiegspunkt fuer `--detect-mic --seconds N [--all-apps]`.
/// Exit 0 beobachtet, 1 Registry nicht lesbar, 2 falscher Aufruf.
pub fn run_cli(seconds: u64, all_apps: bool, poll: Duration) -> (i32, Value) {
    if seconds == 0 || seconds > 3600 {
        return (
            2,
            json!({"mode": "detect_mic", "error": "--seconds muss zwischen 1 und 3600 liegen"}),
        );
    }
    let mode = if all_apps {
        DetectMode::AllApps
    } else {
        DetectMode::MeetingApps
    };
    let procs = LiveProcesses::new();
    let alive = |p: &Path| procs.alive(p);
    run_loop(
        &mut RegistrySource::new(),
        &alive,
        mode,
        Duration::from_secs(seconds),
        poll,
    )
}

#[cfg(test)]
mod tests {
    use super::source::{unix_ms_to_filetime, FakeSource, MicUsage};
    use super::*;

    fn teams(age_ms: u64) -> MicUsage {
        MicUsage {
            app_key: "pkg:MSTeams_8wekyb3d8bbwe".into(),
            exe_path: None,
            last_start: unix_ms_to_filetime(now_unix_ms() - age_ms),
            active: true,
        }
    }

    #[test]
    fn run_loop_reports_a_started_event_with_class_and_time() {
        let mut src = FakeSource {
            steps: vec![Ok(vec![teams(10_000)])],
            next: 0,
        };
        let (code, payload) = run_loop(
            &mut src,
            &|_| true,
            DetectMode::MeetingApps,
            Duration::from_millis(30),
            Duration::from_millis(10),
        );
        assert_eq!(code, 0);
        let events = payload["events"].as_array().unwrap();
        assert_eq!(events.len(), 1, "{payload}");
        assert_eq!(events[0]["kind"], "started");
        assert_eq!(events[0]["label"], "Microsoft Teams");
        assert_eq!(events[0]["class"], "meeting_app");
        assert!(
            events[0]["at_unix_ms"].as_u64().unwrap()
                >= payload["started_unix_ms"].as_u64().unwrap()
        );
    }

    #[test]
    fn run_loop_lists_excluded_entries_with_the_reason() {
        let own = MicUsage {
            app_key: "np:C:#x#local-voice-ai.exe".into(),
            exe_path: Some("C:\\x\\local-voice-ai.exe".into()),
            last_start: unix_ms_to_filetime(now_unix_ms() - 60_000),
            active: true,
        };
        let mut src = FakeSource {
            steps: vec![Ok(vec![own])],
            next: 0,
        };
        let (_, payload) = run_loop(
            &mut src,
            &|_| true,
            DetectMode::AllApps,
            Duration::from_millis(10),
            Duration::from_millis(5),
        );
        assert!(payload["events"].as_array().unwrap().is_empty());
        assert_eq!(payload["active_seen"][0]["excluded"], "own_app");
    }

    #[test]
    fn run_loop_fails_with_exit_1_when_the_source_never_answers() {
        let mut src = FakeSource {
            steps: vec![Err("gesperrt".into())],
            next: 0,
        };
        let (code, payload) = run_loop(
            &mut src,
            &|_| true,
            DetectMode::MeetingApps,
            Duration::from_millis(10),
            Duration::from_millis(5),
        );
        assert_eq!(code, 1);
        assert_eq!(payload["error"], "gesperrt");
    }

    #[test]
    fn run_cli_rejects_a_missing_or_absurd_duration() {
        assert_eq!(run_cli(0, false, Duration::from_millis(10)).0, 2);
        assert_eq!(run_cli(99_999, false, Duration::from_millis(10)).0, 2);
    }

    #[test]
    fn state_tracks_the_open_notice() {
        let state = DetectState::default();
        state.apply(&DetectEvent::Started {
            app_key: "pkg:x".into(),
            label: "X".into(),
            class: "other",
        });
        assert_eq!(state.notice().unwrap().label, "X");
        state.apply(&DetectEvent::Ended {
            app_key: "pkg:other".into(),
        });
        assert!(state.notice().is_some(), "fremdes Ende schliesst nichts");
        state.apply(&DetectEvent::Ended {
            app_key: "pkg:x".into(),
        });
        assert!(state.notice().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn live_processes_finds_the_test_executable_and_not_a_ghost() {
        let procs = LiveProcesses::new();
        let me = std::env::current_exe().unwrap();
        assert!(procs.alive(&me), "{}", me.display());
        assert!(!procs.alive(Path::new("C:\\gibt\\es\\nicht\\geist-1234.exe")));
    }
}
