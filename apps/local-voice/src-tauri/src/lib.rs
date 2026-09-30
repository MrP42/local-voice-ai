mod actions;
mod appdata_migration;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod apple_intelligence;
mod audio_feedback;
mod books;
pub mod audio_toolkit;
mod catalog;
pub mod cli;
mod clipboard;
mod commands;
mod sync;
#[cfg(windows)]
mod context_menu;
mod helpers;
mod input;
mod local_update;
mod llm_client;
mod managers;
mod media;
pub mod mcp; // M6-P6e
mod meeting_prompt; // M5-P5b
mod overlay;
mod paste_guard;
pub mod portable;
mod process_guard;
mod refinement;
pub mod segmenter;
pub mod selftest;
mod settings;
mod shortcut;
mod signal_handle;
mod summarizer;
mod tagging;
mod transcription_coordinator;
mod translator;
mod tray;
mod tray_i18n;
mod utils;

pub use cli::CliArgs;
#[cfg(debug_assertions)]
use specta_typescript::{BigIntExportBehavior, Typescript};
use tauri_specta::{collect_commands, collect_events, Builder};

use env_filter::Builder as EnvFilterBuilder;
use managers::audio::AudioRecordingManager;
use managers::history::HistoryManager;
use managers::model::ModelManager;
use managers::transcription::TranscriptionManager;
#[cfg(unix)]
use signal_hook::consts::{SIGUSR1, SIGUSR2};
#[cfg(unix)]
use signal_hook::iterator::Signals;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use tauri::image::Image;
pub use transcription_coordinator::TranscriptionCoordinator;

use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};

use crate::settings::get_settings;

// Global atomic to store the file log level filter
// We use u8 to store the log::LevelFilter as a number
pub static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(log::LevelFilter::Debug as u8);

/// When `true`, log records are also forwarded to the webview via the
/// `log://log` event for the debug panel's live log viewer. Gated on debug
/// mode — the live log viewer is its only consumer and only exists in debug
/// mode — so normal runs never broadcast log records (which can include file
/// paths or transcribed text) onto the frontend event bus. Synced at startup
/// and whenever debug mode is toggled (see `shortcut::change_debug_mode_setting`).
pub static WEBVIEW_LOG_STREAMING: AtomicBool = AtomicBool::new(false);

fn level_filter_from_u8(value: u8) -> log::LevelFilter {
    match value {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        5 => log::LevelFilter::Trace,
        _ => log::LevelFilter::Trace,
    }
}

fn build_console_filter() -> env_filter::Filter {
    let mut builder = EnvFilterBuilder::new();

    match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => {
            if let Err(err) = builder.try_parse(&spec) {
                log::warn!(
                    "Ignoring invalid RUST_LOG value '{}': {}. Falling back to info-level console logging",
                    spec,
                    err
                );
                builder.filter_level(log::LevelFilter::Info);
            }
        }
        _ => {
            builder.filter_level(log::LevelFilter::Info);
        }
    }

    builder.build()
}

/// Whether the main window has been placed on screen yet. The builder's
/// `.center()` does not survive to first paint — measured: the window came up
/// 1499 px off centre — so it is re-applied here, where the final size is
/// known. Only once: after that the position is the user's business.
static MAIN_WINDOW_PLACED: AtomicBool = AtomicBool::new(false);

/// Put the window in the middle of the primary monitor.
///
/// Computed rather than delegated: both `WebviewWindowBuilder::center()` and
/// `WebviewWindow::center()` left the window 1489 px off centre on this
/// two-monitor setup (primary 3840x2160 at 0,0 plus a portrait screen to its
/// right). Physical pixels throughout, so DPI scaling cannot skew it.
fn centre_on_primary_monitor(window: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = window.primary_monitor() else {
        log::warn!("No primary monitor reported; leaving the window where it is");
        return;
    };
    let Ok(size) = window.outer_size() else {
        log::warn!("Could not read the window size; leaving it where it is");
        return;
    };
    let screen = monitor.size();
    let origin = monitor.position();
    let x = origin.x + (screen.width as i32 - size.width as i32) / 2;
    let y = origin.y + (screen.height as i32 - size.height as i32) / 2;
    if let Err(e) = window.set_position(tauri::PhysicalPosition::new(x, y)) {
        log::warn!("Could not centre the main window: {}", e);
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(main_window) = app.get_webview_window("main") {
        if !MAIN_WINDOW_PLACED.swap(true, Ordering::AcqRel) {
            centre_on_primary_monitor(&main_window);
        }
        if let Err(e) = main_window.unminimize() {
            log::error!("Failed to unminimize webview window: {}", e);
        }
        if let Err(e) = main_window.show() {
            log::error!("Failed to show webview window: {}", e);
        }
        if let Err(e) = main_window.set_focus() {
            log::error!("Failed to focus webview window: {}", e);
        }
        #[cfg(target_os = "macos")]
        {
            if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
                log::error!("Failed to set activation policy to Regular: {}", e);
            }
        }
        return;
    }

    let webview_labels = app.webview_windows().keys().cloned().collect::<Vec<_>>();
    log::error!(
        "Main window not found. Webview labels: {:?}",
        webview_labels
    );
}

#[allow(unused_variables)]
fn should_force_show_permissions_window(app: &AppHandle) -> bool {
    #[cfg(target_os = "windows")]
    {
        let model_manager = app.state::<Arc<ModelManager>>();
        let has_downloaded_models = model_manager
            .get_available_models()
            .iter()
            .any(|model| model.is_downloaded);

        if !has_downloaded_models {
            return false;
        }

        let status = commands::audio::get_windows_microphone_permission_status();
        if status.supported && status.overall_access == commands::audio::PermissionAccess::Denied {
            log::info!(
                "Windows microphone permissions are denied; forcing main window visible for onboarding"
            );
            return true;
        }
    }

    false
}

fn initialize_core_logic(app_handle: &AppHandle) {
    // Note: Enigo (keyboard/mouse simulation) is NOT initialized here.
    // The frontend is responsible for calling the `initialize_enigo` command
    // after onboarding completes. This avoids triggering permission dialogs
    // on macOS before the user is ready.

    // Initialize the managers. The audio recorder receives the streaming router
    // explicitly, so always-on microphone startup can wire live-preview frames
    // even before Tauri state is populated.
    let model_manager =
        Arc::new(ModelManager::new(app_handle).expect("Failed to initialize model manager"));
    let transcription_manager = Arc::new(
        TranscriptionManager::new(app_handle, model_manager.clone())
            .expect("Failed to initialize transcription manager"),
    );
    let recording_manager = Arc::new(
        AudioRecordingManager::new(app_handle, transcription_manager.stream_router())
            .expect("Failed to initialize recording manager"),
    );
    let history_manager =
        Arc::new(HistoryManager::new(app_handle).expect("Failed to initialize history manager"));
    let tts_manager = managers::tts::TtsManager::new(app_handle);
    // Piper-Katalog: Laufzeit- und Stimmen-Downloads (Paket B-E3). Getrennt
    // vom eigentlichen TtsManager (parallele Pakete bauen die Piper-Engine
    // selbst) — dieser Manager kennt nur Katalog + Downloads.
    let tts_model_manager = Arc::new(
        managers::tts::models::TtsModelManager::new(app_handle)
            .expect("Failed to initialize Piper model manager"),
    );
    // Lokales Sprachmodell: Laufzeit-Downloads + Serverprozess. Als globale
    // Zugriffe hinterlegt, weil `llm_client` keinen AppHandle hat, den
    // Server aber vor der ersten Anfrage starten koennen muss.
    let llm_runtime = Arc::new(
        managers::llm::LlmRuntimeManager::new(app_handle)
            .expect("Failed to initialize LLM runtime manager"),
    );
    let llm_server = Arc::new(managers::llm::LocalLlmServer::new());
    managers::llm::install_globals(llm_runtime.clone(), llm_server.clone());
    // Speicherwächter: faellt der freie RAM unter die Notgrenze, stoppt die
    // App ihre eigenen Server (fish-speech, llama-server), bevor Windows in
    // die Auslagerung kippt und unbedienbar wird (Ausfall 15.09.2026).
    {
        let tts = tts_manager.clone();
        let llm = llm_server.clone();
        let notify = app_handle.clone();
        process_guard::spawn_memory_watchdog(move |free_mb| {
            tts.stop_server();
            llm.stop();
            // M4-P4b: auch der zweite llama-server (Embedding-Modus).
            managers::llm::stop_embedding();
            log::error!("memory watchdog stopped TTS and LLM servers at {free_mb} MB free");
            crate::utils::show_transient_notice(&notify, "guard.memoryLow");
        });
    }
    // Verbrauchs-Ledger: jeder Sprachmodell-Aufruf wird gebucht. Global aus
    // demselben Grund wie der Server: `llm_client` bucht ohne AppHandle.
    let usage_ledger = Arc::new(
        managers::usage::UsageLedger::new(app_handle).expect("Failed to initialize usage ledger"),
    );
    let settings_handle = app_handle.clone();
    managers::usage::install_globals(
        usage_ledger.clone(),
        Arc::new(move || settings::get_settings(&settings_handle)),
    );
    // M5-P5b: Zustand des Hinweisfensters (auch ohne Store verwaltet: die Befehle
    // des Fensters brauchen ihn).
    app_handle.manage(meeting_prompt::MeetingPromptState::default());
    // Meetings (M8): the store is shared by recorder and commands. A store
    // that fails to open must not take the whole app down — dictation and TTS
    // work without it, so meetings degrade to "unavailable" instead.
    let meeting_store = match managers::meetings::store::MeetingStore::new(app_handle) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            log::error!("Failed to initialize meetings store: {e}");
            None
        }
    };

    // Initialize the transcribe-cpp native backend (logging + backend module
    // registration) once, before any whisper model is loaded.
    managers::transcription::init_transcribe_backend();

    // Apply accelerator preferences before any model loads
    managers::transcription::apply_accelerator_settings(app_handle);

    // Add managers to Tauri's managed state
    app_handle.manage(recording_manager.clone());
    app_handle.manage(model_manager.clone());
    app_handle.manage(transcription_manager.clone());
    app_handle.manage(history_manager.clone());
    app_handle.manage(tts_manager);
    app_handle.manage(tts_model_manager);
    app_handle.manage(llm_runtime);
    app_handle.manage(llm_server);
    app_handle.manage(usage_ledger);
    app_handle.manage(commands::tts::AutoTagRun::default());
    app_handle.manage(commands::tts::BuilderRun::default());
    // Geraete-Sync: Status-Objekt fuer die Oberflaeche; die Schleife startet
    // nach dem Setup und laeuft nur, wenn sync.json existiert.
    app_handle.manage(sync::SyncEngine::default());
    sync::start(app_handle.clone());
    app_handle.manage(tray::CurrentTrayIconState::new());

    // Entwuerfe des Stimmen-Baukastens aelter als 30 Tage entfernen:
    // Kandidaten-WAVs sind gross, und ein vergessener Entwurf haelt
    // sonst dauerhaft Platz. Best effort — ein Fehler hier darf den
    // Start nicht aufhalten.
    {
        let fish_dir =
            std::path::PathBuf::from(crate::settings::get_settings(app_handle).tts_fish_dir);
        let removed = crate::managers::tts::builder::prune_drafts(&fish_dir, 30);
        if removed > 0 {
            log::info!("Baukasten: {removed} alte Entwuerfe entfernt");
        }
    }

    if let Some(store) = meeting_store {
        let recorder = Arc::new(managers::meetings::recorder::MeetingRecorderManager::new(
            app_handle,
            store.clone(),
            transcription_manager.clone(),
        ));
        // A meeting still marked 'recording' means the app died mid recording:
        // repair the WAV headers and hand the meeting back as 'ready'.
        recorder.recover_orphans();
        // Retention (Task 12): sweep for audio whose `audio_retention_until`
        // already passed while the app was closed (e.g. a `Days(n)` policy).
        // The `AfterMinutes` fast path also purges inline right after a
        // protocol is generated, so this mainly catches the elapsed-days case.
        if let Err(e) =
            managers::meetings::retention::purge_due_audio(&store, chrono::Utc::now().timestamp())
        {
            log::warn!("meetings: startup retention purge failed: {e}");
        }
        let index_store = store.clone(); // M4-P4b
        let calendar_store = store.clone(); // M5-P5b
        app_handle.manage(store);
        app_handle.manage(recorder);
        // M1-P1f: KI-Notizen starten nach `TranscriptFinal` (Einstellung
        // `meeting_auto_enhance`); das Backend entscheidet, kein Fenster noetig.
        commands::meeting_enhance::register_auto_enhance(app_handle);
        // M4-P4b: Such-Index. Lexikalisch sofort, Vektoren im Hintergrund
        // hinter Gates; Nachholen beim Start, Ausloeser ueber MeetingEvent.
        managers::meetings::search::indexer::start_for_app(app_handle, index_store);
        // M5-P5b: Kalender-Sync (15 min), Erinnerung 1 min vorher, Hinweisfenster.
        // Legt auch den Geheimnisordner fest (`secret::init_dir`).
        let calendar =
            managers::calendar::service::CalendarService::spawn(app_handle.clone(), calendar_store);
        app_handle.manage(calendar);
    }

    // M5-P5c: Ad-hoc-Erkennung laufender Besprechungen (nur Hinweis).
    managers::meeting_detect::start(app_handle);

    // Note: Shortcuts are NOT initialized here.
    // The frontend is responsible for calling the `initialize_shortcuts` command
    // after permissions are confirmed (on macOS) or after onboarding completes.
    // This matches the pattern used for Enigo initialization.

    #[cfg(unix)]
    let signals = Signals::new([SIGUSR1, SIGUSR2]).unwrap();
    // Set up signal handlers for toggling transcription
    #[cfg(unix)]
    signal_handle::setup_signal_handler(app_handle.clone(), signals);

    // Apply macOS Accessory policy if starting hidden and tray is available.
    // If the tray icon is disabled, keep the dock icon so the user can reopen.
    #[cfg(target_os = "macos")]
    {
        let settings = settings::get_settings(app_handle);
        if settings.start_hidden && settings.show_tray_icon {
            let _ = app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
        }
    }
    // Get the current theme to set the appropriate initial icon
    let initial_theme = tray::get_current_theme(app_handle);

    // Choose the appropriate initial icon based on theme
    let initial_icon_path = tray::get_icon_path(initial_theme, tray::TrayIconState::Idle);

    let mut tray_builder = TrayIconBuilder::new()
        .icon(
            Image::from_path(
                app_handle
                    .path()
                    .resolve(initial_icon_path, tauri::path::BaseDirectory::Resource)
                    .unwrap(),
            )
            .unwrap(),
        )
        .tooltip(tray::tray_tooltip())
        .icon_as_template(true);

    // Windows notification-area convention: left click opens the app, right click
    // shows the menu. Elsewhere (macOS menu bar, Linux) the menu stays on left click.
    #[cfg(target_os = "windows")]
    {
        tray_builder = tray_builder
            .show_menu_on_left_click(false)
            .on_tray_icon_event(|tray, event| {
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
                let opens_window = matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                );
                if opens_window {
                    show_main_window(tray.app_handle());
                }
            });
    }
    #[cfg(not(target_os = "windows"))]
    {
        tray_builder = tray_builder.show_menu_on_left_click(true);
    }

    let tray = tray_builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => {
                show_main_window(app);
            }
            "check_updates" => {
                // Auch ohne Online-Suche: der lokale Update-Ordner braucht
                // kein Netz, der Prüfer entscheidet selbst, was er abfragt.
                show_main_window(app);
                let _ = app.emit("check-for-updates", ());
            }
            "copy_last_transcript" => {
                tray::copy_last_transcript(app);
            }
            "unload_model" => {
                let transcription_manager = app.state::<Arc<TranscriptionManager>>();
                if !transcription_manager.is_model_loaded() {
                    log::warn!("No model is currently loaded.");
                    return;
                }
                match transcription_manager.unload_model() {
                    Ok(()) => log::info!("Model unloaded via tray."),
                    Err(e) => log::error!("Failed to unload model via tray: {}", e),
                }
            }
            "cancel" => {
                use crate::utils::cancel_current_operation;

                // Use centralized cancellation that handles all operations
                cancel_current_operation(app);
            }
            "quit" => {
                app.exit(0);
            }
            id if id.starts_with("model_select:") => {
                let model_id = id.strip_prefix("model_select:").unwrap().to_string();
                let current_model = settings::get_settings(app).selected_model;
                if model_id == current_model {
                    return;
                }
                let app_clone = app.clone();
                std::thread::spawn(move || {
                    match commands::models::switch_active_model(&app_clone, &model_id) {
                        Ok(()) => {
                            log::info!("Model switched to {} via tray.", model_id);
                        }
                        Err(e) => {
                            log::error!("Failed to switch model via tray: {}", e);
                        }
                    }
                    tray::update_tray_menu(&app_clone, None);
                });
            }
            _ => {}
        })
        .build(app_handle)
        .unwrap();
    app_handle.manage(tray);

    // Initialize tray menu with idle state
    utils::update_tray_menu(app_handle, None);

    // Apply show_tray_icon setting
    let settings = settings::get_settings(app_handle);
    if !settings.show_tray_icon {
        tray::set_tray_visibility(app_handle, false);
    }

    // Refresh tray menu when model state changes
    let app_handle_for_listener = app_handle.clone();
    app_handle.listen("model-state-changed", move |_| {
        tray::update_tray_menu(&app_handle_for_listener, None);
    });

    // Get the autostart manager and configure based on user setting
    let autostart_manager = app_handle.autolaunch();
    let settings = settings::get_settings(app_handle);

    if settings.autostart_enabled {
        // Enable autostart if user has opted in
        let _ = autostart_manager.enable();
    } else {
        // Disable autostart if user has opted out
        let _ = autostart_manager.disable();
    }

    // Create the recording overlay window (hidden by default)
    utils::create_recording_overlay(app_handle);
}

#[tauri::command]
#[specta::specta]
fn trigger_update_check(app: AppHandle) -> Result<(), String> {
    app.emit("check-for-updates", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn show_main_window_command(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

/// Convert an unexpected panic on the headless worker into a normal CLI
/// failure. Without this guard the Tauri event loop remains alive after the
/// worker exits, leaving `--transcribe-file` hung indefinitely.
fn run_headless_guarded<F>(operation: F) -> i32
where
    F: FnOnce() -> i32,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(code) => code,
        Err(payload) => {
            let message = if let Some(message) = payload.downcast_ref::<&str>() {
                (*message).to_string()
            } else if let Some(message) = payload.downcast_ref::<String>() {
                message.clone()
            } else {
                "unknown panic".to_string()
            };
            eprintln!("error: headless transcription panicked: {message}");
            1
        }
    }
}

#[cfg(test)]
mod headless_guard_tests {
    use super::run_headless_guarded;

    #[test]
    fn preserves_normal_exit_codes() {
        assert_eq!(run_headless_guarded(|| 2), 2);
    }

    #[test]
    fn converts_worker_panics_to_runtime_failures() {
        assert_eq!(run_headless_guarded(|| panic!("simulated failure")), 1);
    }
}

#[cfg(test)]
mod make_orphan_gate_tests {
    use super::make_orphan_allowed;

    /// The case that matters: a release build is what users run, and
    /// `--make-orphan` fabricates rows in whatever meetings.db it finds.
    /// Hiding the flag from `--help` is not a control, so without the
    /// explicit opt-in it must refuse.
    #[test]
    fn release_without_the_env_var_is_refused() {
        assert!(!make_orphan_allowed(false, false));
    }

    #[test]
    fn release_with_the_env_var_is_allowed_for_the_harness() {
        assert!(make_orphan_allowed(false, true));
    }

    #[test]
    fn debug_builds_stay_unrestricted() {
        assert!(make_orphan_allowed(true, false));
        assert!(make_orphan_allowed(true, true));
    }
}

/// Headless one-shot transcription for the `--transcribe-file` / `--list-devices`
/// path. Drives the same `TranscriptionManager::transcribe` the app uses; no
/// mic, no VAD, no download. Returns a process exit code (0 ok, 1 runtime
/// failure, 2 bad input/usage).
/// Emit a result payload to stdout and, when asked, to a file.
///
/// The file is what an automated caller actually reads: this binary targets
/// the Windows GUI subsystem, so its stdout reaches a terminal but not a
/// calling script's pipe.
fn emit_headless_payload(payload: &serde_json::Value, out: Option<&std::path::Path>) {
    println!("{}", payload);
    if let Some(path) = out {
        match std::fs::write(
            path,
            serde_json::to_string_pretty(payload).unwrap_or_default(),
        ) {
            Ok(()) => eprintln!("wrote {}", path.display()),
            Err(e) => eprintln!("error: could not write {}: {}", path.display(), e),
        }
    }
}

/// Drive the live streaming path headlessly and report when text appeared.
///
/// Audio is pushed straight into the stream router in 100 ms frames, paced in
/// real time, so the model sees the same arrival pattern it would from a
/// microphone — a stream fed as fast as the disk allows would report latencies
/// that nobody can ever experience.
///
/// Nothing is injected anywhere: `stream_injection` governs typing into the
/// focused window, and this path deliberately never touches it. The only
/// output is the measurement.
fn run_headless_stream(
    app: &AppHandle,
    samples: &[f32],
    audio_secs: f64,
    model_id: &str,
    load_ms: u64,
    args: &CliArgs,
) -> i32 {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tauri::Listener;

    let tm = app.state::<Arc<TranscriptionManager>>();

    // Timestamp every committed growth. The event carries the whole committed
    // prefix, so a growth is "longer than last time"; the model also rewrites
    // the tentative tail constantly, which is not what we are timing.
    let observed: Arc<Mutex<(Vec<u64>, String)>> =
        Arc::new(Mutex::new((Vec::new(), String::new())));
    let start = Instant::now();
    let sink = Arc::clone(&observed);
    let listener = app.listen("stream-text-event", move |event| {
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(event.payload()) else {
            return;
        };
        let committed = payload
            .get("committed")
            .and_then(|c| c.as_str())
            .unwrap_or_default();
        let mut guard = sink.lock().unwrap_or_else(|e| e.into_inner());
        if committed.len() > guard.1.len() {
            guard.0.push(start.elapsed().as_millis() as u64);
            guard.1 = committed.to_string();
        }
    });

    tm.start_stream();

    // 100 ms of 16 kHz mono audio per frame, paced to wall clock.
    const FRAME: usize = 1_600;
    let mut next_due = start;
    for chunk in samples.chunks(FRAME) {
        tm.stream_router().feed(chunk);
        next_due += Duration::from_millis(100);
        let now = Instant::now();
        if next_due > now {
            std::thread::sleep(next_due - now);
        }
    }

    let text = match tm.finalize_stream() {
        Ok(Some(text)) => text,
        Ok(None) => {
            eprintln!(
                "error: the stream produced nothing — is '{}' a streaming-capable model?",
                model_id
            );
            app.unlisten(listener);
            return 1;
        }
        Err(e) => {
            eprintln!("error: finalize failed: {}", e);
            app.unlisten(listener);
            return 1;
        }
    };
    let total_ms = start.elapsed().as_millis() as u64;
    app.unlisten(listener);

    let commit_times = observed.lock().unwrap_or_else(|e| e.into_inner()).0.clone();
    let scored = crate::selftest::SelfTestResult::build(
        args.reference.as_deref().unwrap_or_default(),
        &text,
        commit_times,
        total_ms,
        audio_secs,
    );

    if args.json {
        let mut payload = serde_json::json!({
            "model": model_id,
            "mode": "stream",
            "load_ms": load_ms,
            "audio_secs": audio_secs,
        });
        payload["score"] = serde_json::to_value(&scored).unwrap_or_default();
        emit_headless_payload(&payload, args.out.as_deref());
    } else {
        if let Some(path) = args.out.as_deref() {
            let mut payload = serde_json::json!({ "model": model_id, "mode": "stream" });
            payload["score"] = serde_json::to_value(&scored).unwrap_or_default();
            emit_headless_payload(&payload, Some(path));
        }
        println!(
            "model={} mode=stream audio={:.2}s load={}ms total={}ms",
            model_id, audio_secs, load_ms, total_ms
        );
        println!(
            "updates={} first_text={} median_gap={}",
            scored.commit_times_ms.len(),
            scored
                .first_text_ms
                .map(|v| format!("{v}ms"))
                .unwrap_or_else(|| "never".into()),
            scored
                .median_gap_ms
                .map(|v| format!("{v}ms"))
                .unwrap_or_else(|| "-".into()),
        );
        if args.reference.is_some() {
            println!(
                "accuracy={:.1}% ({} of {} words; {} wrong, {} missing, {} extra)",
                scored.accuracy * 100.0,
                scored.correct,
                scored.reference_words,
                scored.substitutions,
                scored.deletions,
                scored.insertions,
            );
        }
        println!("text: {}", text);
    }
    0
}

/// Headless meetings path (`--import-meeting` / `--dump-meeting` /
/// `--make-orphan`), the M8 counterpart to `run_headless_transcription`.
///
/// Every run first performs the *same* startup housekeeping
/// `initialize_core_logic` does for the real app — `recover_orphans()` then
/// `retention::purge_due_audio()` — because that is precisely what the
/// acceptance harness needs to observe: "restart the app and see that the
/// due audio is gone / the orphan was repaired" becomes "run the CLI a
/// second time". Skipping it here would make the headless path a different
/// program from the one being accepted.
///
/// stdout carries only machine-readable lines (`MEETING_ID=`, `DB=`, or one
/// JSON object); logs already go to stderr in headless mode.
fn run_headless_meetings(app: &AppHandle, args: &CliArgs) -> i32 {
    // Same first move as every other headless entry point: this process must
    // never paste into a foreign window.
    crate::selftest::begin_headless_run();

    use managers::meetings::{retention, store::MeetingStore};

    // M2-P2c2: the simulation writes a whole fabricated meeting; it must never
    // land in a user's real meetings.db, so it only runs in a sandbox.
    if args.simulate_meeting
        && std::env::var(managers::meetings::MEETINGS_DIR_ENV)
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
    {
        eprintln!(
            "error: --simulate-meeting only runs in a sandbox: set {}=<empty temp dir>",
            managers::meetings::MEETINGS_DIR_ENV
        );
        return 2;
    }

    let store = match MeetingStore::new(app) {
        Ok(store) => Arc::new(store),
        Err(e) => {
            eprintln!("error: meetings store unavailable: {e}");
            return 1;
        }
    };
    println!("DB={}", store.db_path().display());

    let tm = app.state::<Arc<TranscriptionManager>>().inner().clone();

    // --- startup housekeeping, mirroring initialize_core_logic -------------
    let recorder = Arc::new(managers::meetings::recorder::MeetingRecorderManager::new(
        app,
        Arc::clone(&store),
        Arc::clone(&tm),
    ));
    recorder.recover_orphans();
    // M2-P2d: Nachholen/Enddurchlauf laufen im Hintergrund; dieser Prozess
    // endet sonst, bevor die Besprechung `ready` ist.
    recorder.wait_final_jobs();
    match retention::purge_due_audio(&store, chrono::Utc::now().timestamp()) {
        Ok(deleted) => eprintln!("meetings: startup retention purge deleted {deleted} file(s)"),
        Err(e) => eprintln!("warning: startup retention purge failed: {e}"),
    }

    if let Some(source) = args.make_orphan.clone() {
        // `--make-orphan` writes fabricated data into whatever meetings.db it
        // finds — normally a real user's. `hide = true` keeps it out of
        // `--help`, but hiding is not a control: the harness runs against the
        // RELEASE binary, so anyone who learns the flag name could corrupt
        // production data with it. Hence an explicit opt-in in release
        // builds. The check lives here rather than in clap so the refusal can
        // say what to do about it.
        if !make_orphan_allowed(
            cfg!(debug_assertions),
            std::env::var(HARNESS_DESTRUCTIVE_ENV).as_deref() == Ok("1"),
        ) {
            eprintln!(
                "error: --make-orphan writes fabricated test data into the meetings database \
                 and is disabled in release builds. Set {HARNESS_DESTRUCTIVE_ENV}=1 to allow it \
                 (scripts/m8-verify.ps1 does this for the orphan-recovery scenario only)."
            );
            return 3;
        }
        return make_orphan_meeting(&store, &source, args.out.as_deref());
    }

    if args.simulate_meeting {
        return run_simulate_meeting(app, &store, &tm, args);
    }

    if let Some(path) = args.import_meeting.clone() {
        if !path.exists() {
            eprintln!("error: no such file: {}", path.display());
            return 2;
        }
        // Audio/video imports go through `tm.transcribe_segments`, which —
        // unlike `transcribe()` — does NOT load a model on demand. The app
        // never hits this because the UI path has one loaded already; a cold
        // headless process does, so load it here (same call and the same
        // `--model` / `--device-index` semantics as `--transcribe-file`).
        // Subtitle imports never touch the model, so they must not require one.
        let is_subtitle = matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_lowercase()
                .as_str(),
            "vtt" | "srt"
        );
        if !is_subtitle {
            let model_id = args
                .model
                .clone()
                .unwrap_or_else(|| get_settings(app).selected_model);
            if model_id.is_empty() {
                eprintln!("error: no model selected (pass --model or pick one in the app)");
                return 2;
            }
            let load_start = std::time::Instant::now();
            if let Err(e) = tm.load_model_with_device(&model_id, args.device_index) {
                eprintln!("error: load_model('{model_id}') failed: {e}");
                return 1;
            }
            println!("MODEL={model_id}");
            println!("LOAD_MS={}", load_start.elapsed().as_millis());
        }

        let started = std::time::Instant::now();
        // Consent is confirmed by the caller: a headless import is an
        // explicit, deliberate act by whoever typed the flag (the UI gate
        // itself is covered by the consent-gate scenario, not by this path).
        let result = tauri::async_runtime::block_on(managers::meetings::import::import_media_file(
            app,
            Arc::clone(&store),
            Arc::clone(&tm),
            path.clone(),
            true,
        ));
        match result {
            Ok(meeting_id) => {
                println!("MEETING_ID={meeting_id}");
                println!("IMPORT_MS={}", started.elapsed().as_millis());
                // The release binary targets the Windows GUI subsystem, where
                // a calling script cannot reliably capture stdout (see
                // scripts/selftest-matrix.ps1) — so --out is the dependable
                // channel back to the harness. `--dump-meeting` in the same
                // run owns the file instead; the harness never combines them.
                if args.dump_meeting.is_none() {
                    // The full post-import state, not just the id: with a
                    // short retention policy the audio can already be gone by
                    // the time a *second* process could look, because every
                    // meetings run purges at startup. Observing "the audio
                    // was there and had an expiry" is therefore only possible
                    // from inside the run that created it.
                    let mut payload = meeting_payload(&store, &meeting_id)
                        .unwrap_or_else(|| serde_json::json!({}));
                    payload["meeting_id"] = serde_json::json!(meeting_id);
                    payload["db"] = serde_json::json!(store.db_path().display().to_string());
                    payload["import_ms"] = serde_json::json!(started.elapsed().as_millis() as u64);
                    payload["source_file"] = serde_json::json!(path.display().to_string());
                    payload["audio_file_exists"] = serde_json::json!(payload["mic_audio_path"]
                        .as_str()
                        .map(|p| std::path::Path::new(p).exists())
                        .unwrap_or(false));
                    emit_headless_payload(&payload, args.out.as_deref());
                }
            }
            Err(e) => {
                eprintln!("error: import failed: {e}");
                return 1;
            }
        }
    }

    if let Some(id) = args.dump_meeting.clone() {
        return dump_meeting(&store, &id, args.out.as_deref());
    }

    0
}

/// `--simulate-meeting` (M2-P2c2): two WAVs through the live DSP thread and
/// transcription worker, one JSON object out. Exit 0 ok, 1 runtime error,
/// 2 bad input.
fn run_simulate_meeting(
    app: &AppHandle,
    store: &Arc<managers::meetings::store::MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    args: &CliArgs,
) -> i32 {
    use managers::meetings::simulate::{read_pcm16_mono, SimulateOptions};

    // M2-P2b2: benchmark scenes (mic + system + reference.json) instead of
    // --mic/--system; the reference texts give live WER and far-word leak.
    let scene = if args.scene.is_empty() {
        None
    } else {
        if args.mic.is_some() || args.system.is_some() {
            eprintln!("error: --scene replaces --mic/--system, pass one or the other");
            return 2;
        }
        let mic_file = args.scene_mic.as_deref().unwrap_or("mic_echo.wav");
        match managers::meetings::simulate::load_scenes(&args.scene, mic_file) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
    };
    if let Some(scene) = scene {
        return run_simulate_opts(
            app,
            store,
            tm,
            args,
            SimulateOptions {
                title: "Simulation".to_string(),
                mic: scene.mic,
                system: Some(scene.system),
                aec: !args.no_aec,
                system_delay_ms: args.system_delay_ms.unwrap_or(0),
                far_text: Some(scene.far_text),
                near_text: Some(scene.near_text),
                realtime: args.realtime,
            },
            Some(scene.utterances),
        );
    }

    let Some(mic_path) = args.mic.as_deref() else {
        eprintln!("error: --simulate-meeting needs --mic <wav>");
        return 2;
    };
    let mic = match read_pcm16_mono(mic_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let system = match args.system.as_deref().map(read_pcm16_mono).transpose() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let read_text = |p: &Option<std::path::PathBuf>| -> Result<Option<String>, String> {
        p.as_deref()
            .map(|p| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display())))
            .transpose()
    };
    let (far_text, near_text) = match (read_text(&args.far_text), read_text(&args.near_text)) {
        (Ok(f), Ok(n)) => (f, n),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let opts = SimulateOptions {
        title: "Simulation".to_string(),
        mic,
        system,
        aec: !args.no_aec,
        system_delay_ms: args.system_delay_ms.unwrap_or(0),
        far_text,
        near_text,
        realtime: args.realtime, // M2-P2b2
    };
    run_simulate_opts(app, store, tm, args, opts, None)
}

/// Loads the live model and runs the simulation (shared by --mic and --scene).
fn run_simulate_opts(
    app: &AppHandle,
    store: &Arc<managers::meetings::store::MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    args: &CliArgs,
    opts: managers::meetings::simulate::SimulateOptions,
    utterances: Option<usize>,
) -> i32 {
    use managers::meetings::simulate::simulate_with_app;

    let model_id = args.model.clone().unwrap_or_else(|| {
        tm.meeting_model_target(&get_settings(app))
    });
    if model_id.is_empty() {
        eprintln!("error: no model selected (pass --model or pick one in the app)");
        return 2;
    }
    let load_start = std::time::Instant::now();
    if let Err(e) = tm.load_model_with_device(&model_id, args.device_index) {
        eprintln!("error: load_model('{model_id}') failed: {e}");
        return 1;
    }
    let load_ms = load_start.elapsed().as_millis() as u64;

    match simulate_with_app(app, Arc::clone(store), Arc::clone(tm), opts) {
        Ok(mut payload) => {
            payload["model"] = serde_json::json!(model_id);
            payload["load_ms"] = serde_json::json!(load_ms);
            payload["scenes"] = serde_json::json!(args.scene); // M2-P2b2
            payload["reference_utterances"] = serde_json::json!(utterances);
            // M2-P2d: Enddurchlauf auf derselben Besprechung.
            if let (Some(final_model), Some(id)) =
                (args.final_model.as_deref(), payload["meeting_id"].as_str())
            {
                let id = id.to_string();
                payload["final"] = managers::meetings::simulate::final_pass_with_app(
                    app,
                    Arc::clone(store),
                    Arc::clone(tm),
                    &id,
                    final_model,
                );
            }
            // M7-P7b: KI-Notizen wie der Auto-Lauf nach `TranscriptFinal`.
            if let (true, Some(id)) = (args.notes, payload["meeting_id"].as_str()) {
                // Das STT-Modell bleibt geladen wie in der App (Entladen erst
                // nach `model_unload_timeout`).
                let id = id.to_string();
                payload["notes"] = simulate_notes(app, store, &id, args.notes_model.as_deref());
            }
            emit_headless_payload(&payload, args.out.as_deref());
            0
        }
        Err(e) => {
            eprintln!("error: simulation failed: {e}");
            1
        }
    }
}

/// M7-P7b (QG3): KI-Notizen nach dem Enddurchlauf wie der Auto-Lauf nach
/// `TranscriptFinal` (`on_transcript_final`: Entscheidung, Vorlage,
/// `enhance_meeting`), mit Zeitmessung. Der lokale Server wird wie in
/// `--eval-notes` eingerichtet (samt Speicherwaechter) und am Ende gestoppt.
/// Kein Notiztext im Ergebnis (D9), nur Kennzahlen.
fn simulate_notes(
    app: &AppHandle,
    store: &Arc<managers::meetings::store::MeetingStore>,
    meeting_id: &str,
    model_override: Option<&str>,
) -> serde_json::Value {
    use commands::meeting_enhance::{auto_enhance_decision, AutoEnhanceDecision};
    use managers::meetings::notes::enhance;

    let mut settings = get_settings(app);
    if let Some(model) = model_override.map(str::trim).filter(|m| !m.is_empty()) {
        managers::meetings::notes::eval::apply_model_override(&mut settings, model);
    }
    let decision = auto_enhance_decision(&settings, "live", false);
    let provider = managers::meetings::llm_call::resolve_provider_coded(&settings)
        .ok()
        .map(|(p, model, _)| format!("{}:{model}", p.id));
    if decision != AutoEnhanceDecision::Run {
        return serde_json::json!({
            "ran": false,
            "decision": format!("{decision:?}"),
            "provider": provider,
        });
    }
    let llm_runtime = match managers::llm::LlmRuntimeManager::new(app) {
        Ok(runtime) => Arc::new(runtime),
        Err(e) => {
            return serde_json::json!({ "ran": false, "error": format!("llm runtime: {e}") })
        }
    };
    let llm_server = Arc::new(managers::llm::LocalLlmServer::new());
    managers::llm::install_globals(llm_runtime, llm_server.clone());
    {
        let llm = llm_server.clone();
        process_guard::spawn_memory_watchdog(move |free_mb| {
            llm.stop();
            eprintln!("error: memory watchdog stopped the LLM server at {free_mb} MB free");
        });
    }
    let template = settings
        .meeting_default_template_id
        .clone()
        .filter(|t| !t.trim().is_empty());
    let steps = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let steps_in = Arc::clone(&steps);
    let started = std::time::Instant::now();
    let result = tauri::async_runtime::block_on(enhance::enhance_meeting(
        &settings,
        Arc::clone(store),
        meeting_id,
        template.as_deref(),
        move |_, total| steps_in.store(total, std::sync::atomic::Ordering::Relaxed),
    ));
    let ms = started.elapsed().as_millis() as u64;
    llm_server.stop();
    // P1i: `ok` allein sagt nicht, ob die Notizen das ganze Transkript abdecken.
    // Teile (nach dem Halbieren), die nicht auswertbar waren, stehen hier;
    // `incomplete` ist genau das Kennzeichen, das die Oberflaeche als Warnung zeigt.
    let stats = result
        .as_ref()
        .ok()
        .and_then(|doc| enhance::parse_enhanced(doc).ok())
        .map(|notes| notes.stats);
    serde_json::json!({
        "ran": true,
        "ok": result.is_ok(),
        "ms": ms,
        "provider": provider,
        "template": template,
        "steps": steps.load(std::sync::atomic::Ordering::Relaxed),
        "error_code": result.as_ref().err().map(|e| enhance::event_code(e)),
        "document_id": result.as_ref().ok().map(|d| d.id.clone()),
        "single_pass": stats.as_ref().map(|s| s.single_pass),
        "chunks_total": stats.as_ref().map(|s| s.chunks_total),
        "chunks_failed": stats.as_ref().map(|s| s.chunks_failed.clone()),
        "incomplete": stats.as_ref().map(|s| !s.chunks_failed.is_empty()),
    })
}

/// `--dump-meeting`: one JSON object describing what the store actually
/// holds for `id`. Deliberately includes the derived numbers the harness
/// asserts on (segment count, first/last segment times, channel set) so the
/// assertions live in one place instead of being re-derived from raw rows.
fn dump_meeting(
    store: &Arc<managers::meetings::store::MeetingStore>,
    id: &str,
    out: Option<&std::path::Path>,
) -> i32 {
    match meeting_payload(store, id) {
        Some(payload) => {
            emit_headless_payload(&payload, out);
            0
        }
        None => {
            eprintln!("error: no meeting {id}");
            2
        }
    }
}

/// Shared by `--dump-meeting` and the `--import-meeting` result: everything
/// the harness asserts on, including the derived numbers (segment count,
/// first/last segment times, the set of channels used), so those derivations
/// live in one place instead of being re-done in PowerShell.
fn meeting_payload(
    store: &Arc<managers::meetings::store::MeetingStore>,
    id: &str,
) -> Option<serde_json::Value> {
    let meeting = match store.get_meeting(id) {
        Ok(Some(m)) => m,
        Ok(None) => return None,
        Err(e) => {
            eprintln!("error: meeting lookup failed: {e}");
            return None;
        }
    };
    let segments = match store.get_segments(id) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: segment lookup failed: {e}");
            return None;
        }
    };
    let documents = store.get_documents(id).unwrap_or_default();

    let mut channels: Vec<u8> = segments.iter().map(|s| s.channel).collect();
    channels.sort_unstable();
    channels.dedup();

    Some(serde_json::json!({
        "id": meeting.id,
        "title": meeting.title,
        "status": meeting.status,
        "source": meeting.source,
        "duration_ms": meeting.duration_ms,
        "ended_at": meeting.ended_at,
        "mic_audio_path": meeting.mic_audio_path,
        "system_audio_path": meeting.system_audio_path,
        "audio_retention_until": meeting.audio_retention_until,
        "consent_confirmed_at": meeting.consent_confirmed_at,
        "segment_count": segments.len(),
        "channels": channels,
        "first_start_ms": segments.first().map(|s| s.start_ms),
        "last_start_ms": segments.last().map(|s| s.start_ms),
        "last_end_ms": segments.last().map(|s| s.end_ms),
        "total_text_chars": segments.iter().map(|s| s.text.len()).sum::<usize>(),
        "document_kinds": documents.iter().map(|d| d.kind.clone()).collect::<Vec<_>>(),
        "segments": segments,
    }))
}

/// Opt-in switch for `--make-orphan` in release builds.
pub const HARNESS_DESTRUCTIVE_ENV: &str = "LVA_HARNESS_DESTRUCTIVE";

/// May `--make-orphan` run? Pure, so the rule is testable without a build
/// flag or a process environment.
///
/// A debug build is already a developer's own machine, so the flag stays as
/// convenient as it was. A release build is what the harness — and every
/// user — actually runs, so there it takes a deliberate
/// `LVA_HARNESS_DESTRUCTIVE=1` before it will fabricate rows in a real
/// meetings database.
fn make_orphan_allowed(is_debug: bool, env_set: bool) -> bool {
    is_debug || env_set
}

/// `--make-orphan`: fabricates exactly the on-disk situation a crash during
/// a live recording leaves behind — a meeting row still on `recording`,
/// already-appended segments, and a WAV whose RIFF/data size fields were
/// never patched because `finalize()` never ran. The recovery itself is NOT
/// simulated: the next run goes through the real `recover_orphans()` above.
fn make_orphan_meeting(
    store: &Arc<managers::meetings::store::MeetingStore>,
    source: &std::path::Path,
    out: Option<&std::path::Path>,
) -> i32 {
    use audio_toolkit::audio::wav_writer::StreamingWavWriter;
    use managers::meetings::store::{MeetingSource, StoredSegment, TranscriptDelta};

    // The live recorder writes i16 PCM; read_wav_samples hands back f32, so
    // scale back to what StreamingWavWriter actually appends.
    let samples: Vec<i16> = match crate::audio_toolkit::read_wav_samples(source) {
        Ok(s) => s
            .into_iter()
            .map(|v| (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect(),
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", source.display());
            return 2;
        }
    };

    let meeting = match store.create_meeting("Crash-Test", MeetingSource::Live, Some(0)) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: create_meeting failed: {e}");
            return 1;
        }
    };

    // Temp, not next to the source fixture: a harness that dies mid-run must
    // not leave debris in the repo.
    let dir = std::env::temp_dir().join(format!(
        "m8-orphan-{}",
        &meeting.id[meeting.id.len().saturating_sub(8)..]
    ));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("error: orphan dir failed: {e}");
        return 1;
    }
    let wav_path = dir.join("mic.wav");

    // Write and flush, then drop WITHOUT finalize — that is the crash.
    {
        let mut writer = match StreamingWavWriter::create(&wav_path, 16_000) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("error: wav writer failed: {e}");
                return 1;
            }
        };
        if let Err(e) = writer.append(&samples) {
            eprintln!("error: wav append failed: {e}");
            return 1;
        }
        // No flush_header() either: the sizes stay at 0, the worst case.
    }

    let segment = StoredSegment {
        segment_index: 0,
        text: "Vor dem Absturz aufgezeichnet.".to_string(),
        start_ms: 0,
        end_ms: 3_000,
        channel: 0,
        speaker_index: None,
        words: None,
    };
    if let Err(e) = store.append_delta(
        &meeting.id,
        &TranscriptDelta {
            new_segments: vec![segment],
        },
    ) {
        eprintln!("error: append_delta failed: {e}");
        return 1;
    }
    if let Err(e) = store.set_audio_paths(&meeting.id, wav_path.to_str(), None, None) {
        eprintln!("error: set_audio_paths failed: {e}");
        return 1;
    }

    println!("MEETING_ID={}", meeting.id);
    println!("ORPHAN_WAV={}", wav_path.display());
    emit_headless_payload(
        &serde_json::json!({
            "meeting_id": meeting.id,
            "orphan_wav": wav_path.display().to_string(),
            "samples": samples.len(),
        }),
        out,
    );
    0
}

fn run_headless_transcription(app: &AppHandle, args: &CliArgs) -> i32 {
    // Before anything else: this process must not paste into any window.
    crate::selftest::begin_headless_run();

    use std::time::Instant;

    // --list-devices: print registered compute devices (with indices) and exit.
    // Useful on multi-GPU machines to discover the index for --device-index.
    if args.list_devices {
        let devices = crate::managers::transcription::describe_compute_devices();
        if devices.is_empty() {
            println!("No transcribe-cpp compute devices registered.");
        } else {
            println!("transcribe-cpp compute devices:");
            for d in &devices {
                println!("  {}", d);
            }
        }
        if args.transcribe_file.is_none() {
            return 0;
        }
    }

    // --list-models: print the model registry (catalog + on-disk + custom) with
    // their ids — the same ids `--model` accepts — then exit. `--json` emits the
    // full ModelInfo array for scripting.
    if args.list_models {
        let model_manager = app.state::<Arc<ModelManager>>();
        let models = model_manager.get_available_models();
        if args.json {
            match serde_json::to_string_pretty(&models) {
                Ok(s) => println!("{}", s),
                Err(e) => {
                    eprintln!("error: failed to serialize models: {}", e);
                    return 1;
                }
            }
        } else if models.is_empty() {
            println!("No models available.");
        } else {
            println!("Available models (✓ = installed):");
            let width = models.iter().map(|m| m.id.len()).max().unwrap_or(0);
            for m in &models {
                let mark = if m.is_downloaded { "✓" } else { " " };
                let rec = if m.is_recommended {
                    "  [recommended]"
                } else {
                    ""
                };
                println!(
                    "  {}  {:<width$}  {}{}",
                    mark,
                    m.id,
                    m.name,
                    rec,
                    width = width
                );
            }
        }
        if args.transcribe_file.is_none() {
            return 0;
        }
    }

    let Some(wav) = args.transcribe_file.clone() else {
        return 0;
    };

    // read_wav_samples reads 16-bit int samples and does no validation; the app
    // only ever saves 16 kHz mono 16-bit PCM, so reject anything else rather than
    // transcribe garbage / mis-time / mis-decode.
    match hound::WavReader::open(&wav) {
        Ok(reader) => {
            let spec = reader.spec();
            if spec.sample_rate != 16_000
                || spec.channels != 1
                || spec.bits_per_sample != 16
                || spec.sample_format != hound::SampleFormat::Int
            {
                eprintln!(
                    "error: expected 16 kHz mono 16-bit PCM WAV, got {} Hz / {} ch / {}-bit {:?}",
                    spec.sample_rate, spec.channels, spec.bits_per_sample, spec.sample_format
                );
                return 2;
            }
        }
        Err(e) => {
            eprintln!("error: cannot open {}: {}", wav.display(), e);
            return 2;
        }
    }

    let samples = match crate::audio_toolkit::read_wav_samples(&wav) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to read {}: {}", wav.display(), e);
            return 2;
        }
    };
    let audio_secs = samples.len() as f64 / 16_000.0;

    let tm = app.state::<Arc<TranscriptionManager>>();

    let model_id = args
        .model
        .clone()
        .unwrap_or_else(|| get_settings(app).selected_model);
    if model_id.is_empty() {
        eprintln!("error: no model selected (pass --model or pick one in the app)");
        return 2;
    }

    // --device-index hard-selects a compute device by its --list-devices registry
    // index (transcribe-cpp / whisper-family models only; not persisted). Omit it
    // to use the persisted accelerator setting.
    let device_index = args.device_index;
    let requested_device = match device_index {
        Some(idx) => format!("index {}", idx),
        None => "settings".to_string(),
    };

    // Cold load (timed).
    let load_start = Instant::now();
    if let Err(e) = tm.load_model_with_device(&model_id, device_index) {
        eprintln!("error: load_model('{}') failed: {}", model_id, e);
        return 1;
    }
    let load_ms = load_start.elapsed().as_millis() as u64;
    let bound_backend = tm.current_backend();

    if args.stream {
        return run_headless_stream(app, &samples, audio_secs, &model_id, load_ms, &args);
    }

    let runs = args.repeat.unwrap_or(1).max(1);
    let mut times_ms: Vec<u64> = Vec::new();
    let mut text = String::new();
    for i in 0..runs {
        // If the model's unload-timeout is "Immediately", transcribe() unloads
        // the engine after each run; reload (untimed) so repeats keep working
        // and the inference timing below stays clean.
        if !tm.is_model_loaded() {
            if let Err(e) = tm.load_model_with_device(&model_id, device_index) {
                eprintln!("error: reload before run {} failed: {}", i + 1, e);
                return 1;
            }
        }
        let t = Instant::now();
        match tm.transcribe(samples.clone()) {
            Ok(out) => text = out,
            Err(e) => {
                eprintln!("error: transcribe failed: {}", e);
                return 1;
            }
        }
        times_ms.push(t.elapsed().as_millis() as u64);
    }
    let best_ms = times_ms.iter().copied().min().unwrap_or(0);
    let rtf = if best_ms > 0 {
        audio_secs / (best_ms as f64 / 1000.0)
    } else {
        0.0
    };

    if args.json {
        let mut payload = serde_json::json!({
            "model": model_id,
            "requested_device": requested_device,
            "bound_backend": bound_backend,
            "audio_secs": audio_secs,
            "load_ms": load_ms,
            "transcribe_ms": times_ms,
            "best_ms": best_ms,
            "rtf": rtf,
            "text": text,
        });
        if let Some(reference) = args.reference.as_deref() {
            let scored = crate::selftest::SelfTestResult::build(
                reference,
                &text,
                Vec::new(),
                best_ms,
                audio_secs,
            );
            payload["score"] = serde_json::to_value(&scored).unwrap_or_default();
        }
        emit_headless_payload(&payload, args.out.as_deref());
    } else if let Some(reference) = args.reference.as_deref() {
        let scored = crate::selftest::SelfTestResult::build(
            reference,
            &text,
            Vec::new(),
            best_ms,
            audio_secs,
        );
        println!(
            "model={} backend={} audio={:.2}s best={}ms rtf={:.2}x",
            model_id,
            bound_backend.as_deref().unwrap_or("?"),
            audio_secs,
            best_ms,
            rtf,
        );
        println!(
            "accuracy={:.1}% ({} of {} words; {} wrong, {} missing, {} extra)",
            scored.accuracy * 100.0,
            scored.correct,
            scored.reference_words,
            scored.substitutions,
            scored.deletions,
            scored.insertions,
        );
        println!("text: {}", text);
    } else {
        println!(
            "model={} device={} backend={} audio={:.2}s load={}ms best={}ms rtf={:.2}x",
            model_id,
            requested_device,
            bound_backend.as_deref().unwrap_or("?"),
            audio_secs,
            load_ms,
            best_ms,
            rtf,
        );
        println!("text: {}", text);
    }
    0
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(cli_args: CliArgs) {
    // Detect portable mode before anything else
    portable::init();

    // Rebranding-Umzug der App-Daten (Settings, Verlauf, Modelle) — muss vor
    // dem ersten Store-Zugriff laufen und ist idempotent.
    appdata_migration::migrate_legacy_app_data();

    // Parse console logging directives from RUST_LOG, falling back to info-level logging
    // when the variable is unset
    let console_filter = build_console_filter();

    let specta_builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            shortcut::change_binding,
            shortcut::reset_binding,
            shortcut::change_ptt_setting,
            shortcut::change_audio_feedback_setting,
            shortcut::change_audio_feedback_volume_setting,
            shortcut::change_sound_theme_setting,
            shortcut::change_theme_setting,
            shortcut::change_start_hidden_setting,
            shortcut::change_autostart_setting,
            shortcut::change_translate_to_english_setting,
            shortcut::change_selected_language_setting,
            shortcut::change_tts_fish_dir_setting,
            shortcut::change_tts_port_setting,
            shortcut::change_tts_seed_setting,
            shortcut::change_tts_idle_minutes_setting,
            shortcut::change_tts_max_chars_setting,
            shortcut::change_tts_voice_setting,
            shortcut::change_tts_compile_setting,
            shortcut::change_tts_translate_lang_setting,
            shortcut::change_tts_volume_setting,
            shortcut::change_tts_normalize_setting,
            shortcut::change_tts_prewarm_setting,
            shortcut::change_tts_enhance_setting,
            shortcut::change_tts_reference_auto_transcribe_setting,
            shortcut::change_tts_enhance_strength_setting,
            commands::llm::llm_upsert_connection,
            commands::llm::llm_remove_connection,
            commands::llm::llm_upsert_model,
            commands::llm::llm_remove_model,
            commands::llm::llm_set_active_model,
            commands::llm::llm_list_remote_models,
            commands::llm::llm_set_api_key,
            commands::llm::llm_local_list,
            commands::llm::llm_local_download,
            commands::llm::llm_local_cancel,
            commands::llm::llm_local_delete,
            commands::llm::llm_local_status,
            commands::llm::llm_local_start,
            commands::llm::llm_local_stop,
            commands::llm::llm_local_backend,
            commands::llm::llm_local_activate,
            commands::llm::system_memory,
            commands::llm::llm_local_fit,
            commands::usage::usage_summary,
            commands::usage::usage_events,
            commands::usage::usage_clear,
            commands::usage::usage_budget_states,
            sync::sync_login,
            sync::sync_logout,
            sync::sync_status,
            sync::sync_now,
            sync::sync_touch,
            sync::sync_default_device_name,
            sync::sync_hub_status,
            shortcut::change_tts_engine_setting,
            shortcut::change_tts_piper_voice_setting,
            shortcut::change_tts_piper_auto_language_setting,
            shortcut::change_tts_script_check_setting,
            shortcut::change_tts_speed_setting,
            shortcut::change_tts_export_format_setting,
            shortcut::change_tts_export_bitrate_setting,
            shortcut::change_tts_context_menu_setting,
            shortcut::change_tts_tag_favorites_setting,
            shortcut::change_tts_autotag_presets_setting,
            shortcut::change_tts_tag_language_setting,
            shortcut::change_tts_autotag_last_setting,
            shortcut::change_tts_tag_provider_setting,
            shortcut::change_tts_tag_device_setting,
            shortcut::change_tts_tag_model_setting,
            shortcut::change_overlay_position_setting,
            shortcut::change_overlay_style_setting,
            shortcut::change_debug_mode_setting,
            shortcut::change_word_correction_threshold_setting,
            shortcut::change_extra_recording_buffer_setting,
            shortcut::change_paste_delay_ms_setting,
            shortcut::change_paste_delay_after_ms_setting,
            shortcut::change_paste_method_setting,
            shortcut::get_available_typing_tools,
            shortcut::change_typing_tool_setting,
            shortcut::change_external_script_path_setting,
            shortcut::change_clipboard_handling_setting,
            shortcut::change_auto_submit_setting,
            shortcut::change_auto_submit_key_setting,
            shortcut::change_post_process_enabled_setting,
            shortcut::change_experimental_enabled_setting,
            shortcut::change_post_process_base_url_setting,
            shortcut::change_post_process_api_key_setting,
            shortcut::change_post_process_model_setting,
            shortcut::set_post_process_provider,
            shortcut::fetch_post_process_models,
            shortcut::add_post_process_prompt,
            shortcut::update_post_process_prompt,
            shortcut::delete_post_process_prompt,
            shortcut::set_post_process_selected_prompt,
            shortcut::update_custom_words,
            shortcut::suspend_binding,
            shortcut::resume_binding,
            shortcut::change_mute_while_recording_setting,
            shortcut::change_dictation_audio_setting,
            shortcut::change_dictation_audio_duck_percent_setting,
            shortcut::change_local_update_dir_setting,
            local_update::local_update_check,
            local_update::local_update_install,
            shortcut::change_append_trailing_space_setting,
            shortcut::change_lazy_stream_close_setting,
            shortcut::change_vad_enabled_setting,
            shortcut::change_app_language_setting,
            shortcut::change_update_checks_setting,
            shortcut::change_show_whats_new_on_update_setting,
            shortcut::change_whats_new_last_seen_version_setting,
            shortcut::change_keyboard_implementation_setting,
            shortcut::get_keyboard_implementation,
            shortcut::change_show_tray_icon_setting,
            shortcut::change_transcribe_accelerator_setting,
            shortcut::change_ort_accelerator_setting,
            shortcut::change_transcribe_gpu_device,
            shortcut::get_available_accelerators,
            shortcut::change_meeting_audio_retention_setting,
            shortcut::change_meeting_language_setting,
            shortcut::change_meeting_model_setting,
            // M1-P1f
            shortcut::change_meeting_capture_system_setting,
            shortcut::change_meeting_auto_enhance_setting,
            shortcut::change_meeting_default_template_setting,
            shortcut::change_meeting_echo_cancellation_setting,
            shortcut::change_meeting_final_model_setting,
            // M5-P5c
            shortcut::change_meeting_detect_mode_setting,
            shortcut::change_meeting_detect_ignored_apps_setting,
            shortcut::meeting_detect_available,
            shortcut::handy_keys::start_handy_keys_recording,
            shortcut::handy_keys::stop_handy_keys_recording,
            trigger_update_check,
            show_main_window_command,
            commands::cancel_operation,
            commands::is_portable,
            commands::set_level_monitoring,
            commands::get_app_dir_path,
            commands::get_app_settings,
            commands::get_default_settings,
            commands::get_log_dir_path,
            commands::set_log_level,
            commands::open_recordings_folder,
            commands::open_log_dir,
            commands::open_app_data_dir,
            commands::check_apple_intelligence_available,
            commands::initialize_enigo,
            commands::initialize_shortcuts,
            commands::models::get_available_models,
            commands::models::get_model_info,
            commands::models::download_model,
            commands::models::delete_model,
            commands::models::cancel_download,
            commands::models::set_active_model,
            commands::models::get_current_model,
            commands::models::get_transcription_model_status,
            commands::models::is_model_loading,
            commands::models::rescan_local_models,
            commands::audio::update_microphone_mode,
            commands::audio::get_microphone_mode,
            commands::audio::get_windows_microphone_permission_status,
            commands::audio::open_microphone_privacy_settings,
            commands::audio::get_available_microphones,
            commands::audio::set_selected_microphone,
            commands::audio::get_selected_microphone,
            commands::audio::get_available_output_devices,
            commands::audio::set_selected_output_device,
            commands::audio::get_selected_output_device,
            commands::audio::play_test_sound,
            commands::audio::check_custom_sounds,
            commands::audio::set_clamshell_microphone,
            commands::audio::get_clamshell_microphone,
            commands::audio::is_recording,
            commands::transcription::set_model_unload_timeout,
            commands::transcription::get_model_load_status,
            commands::transcription::unload_model_manually,
            commands::history::get_history_entries,
            commands::history::toggle_history_entry_saved,
            commands::history::get_audio_file_path,
            commands::history::delete_history_entry,
            commands::history::retry_history_entry_transcription,
            commands::history::update_history_limit,
            commands::history::update_recording_retention_period,
            commands::meetings::meetings_start,
            commands::meetings::meetings_pause,
            commands::meetings::meetings_resume,
            commands::meetings::meetings_stop,
            commands::meetings::meetings_is_recording,
            commands::meetings::meetings_list,
            commands::meetings::meetings_get_segments,
            commands::meetings::meetings_update_segment,
            commands::meetings::meetings_rename,
            commands::meetings::meetings_retranscribe,
            commands::meetings::meetings_get_documents,
            commands::meetings::meetings_delete,
            commands::meetings::meetings_import_file,
            commands::meetings::meetings_generate_minutes,
            commands::meetings::meetings_minutes_file,
            // P1k
            commands::meeting_minutes::meetings_minutes_state,
            commands::meeting_minutes::meetings_minutes_cancel,
            commands::meeting_minutes::meetings_minutes_meta,
            commands::meeting_minutes::meetings_get_auto_template,
            commands::meetings::meetings_export_document,
            // M1-P1c
            commands::meeting_notes::meeting_notes_get,
            commands::meeting_notes::meeting_notes_save,
            commands::meeting_notes::meetings_recording_position,
            commands::meeting_notes::meetings_set_template,
            commands::meeting_notes::meetings_get_template,
            commands::meeting_notes::meeting_templates_list,
            commands::meeting_notes::meeting_templates_save,
            commands::meeting_notes::meeting_templates_duplicate,
            commands::meeting_notes::meeting_templates_delete,
            commands::meeting_notes::meeting_templates_export,
            commands::meeting_notes::meeting_templates_import,
            commands::meeting_notes::action_items_list,
            commands::meeting_notes::action_items_set_status,
            // M1-P1b: KI-Notizen-Motor
            commands::meeting_enhance::meeting_notes_enhance,
            commands::meeting_enhance::meeting_notes_apply_instruction,
            commands::meeting_enhance::meeting_notes_update_enhanced,
            commands::meeting_enhance::meeting_notes_markdown,
            // M1-P1d
            commands::meeting_enhance::meetings_segment_epoch,
            // M4-P4d: Suche und Ordner in der Besprechungsliste
            commands::meeting_search::meetings_search,
            commands::meeting_search::meeting_folders_list,
            commands::meeting_search::meeting_folders_save,
            commands::meeting_search::meeting_folders_delete,
            commands::meeting_search::meetings_set_folders,
            commands::meeting_search::meetings_get_folders,
            // M4-P4b: Index-Status, Embedding-Modell, Einstellung
            commands::meeting_search::meeting_index_status,
            commands::meeting_search::meeting_embedding_model_download,
            commands::meeting_search::change_meeting_semantic_search_setting,
            // M4-P4c: Chat-Motor
            commands::meeting_chat::meeting_chat_ask,
            commands::meeting_chat::meeting_chat_cancel,
            commands::meeting_chat::meeting_chat_threads,
            commands::meeting_chat::meeting_chat_thread,
            commands::meeting_chat::meeting_chat_thread_delete,
            commands::meeting_chat::chat_recipes_list,
            commands::meeting_chat::chat_recipes_save,
            commands::meeting_chat::chat_recipes_delete,
            commands::meeting_chat::chat_recipes_duplicate,
            // M6-P6a: Export einer Besprechung
            commands::meetings::meetings_export,
            commands::meetings::meetings_copy_formatted,
            // M3-P3c: Sprecher benennen, zusammenfuehren, zuordnen
            commands::meeting_speakers::meeting_speakers_list,
            commands::meeting_speakers::meeting_speaker_rename,
            commands::meeting_speakers::meeting_speaker_merge,
            commands::meeting_speakers::meeting_segment_set_speaker,
            commands::meeting_speakers::meeting_speaker_notices,
            commands::meeting_speakers::meetings_set_diarize_mic,
            commands::meeting_speakers::change_meeting_diarization_setting,
            // M6-P6c: Follow-up-Mail
            commands::meetings::meeting_followup_draft,
            commands::meetings::meeting_followup_open,
            commands::meetings::change_meeting_self_emails_setting,
            // M5-P5b: Kalender, Erinnerung, Hinweisfenster, Start aus Termin
            commands::calendar::calendar_sources_list,
            commands::calendar::calendar_source_add_ics,
            commands::calendar::calendar_source_remove,
            commands::calendar::calendar_sync_now,
            commands::calendar::calendar_upcoming,
            commands::calendar::calendar_suggest_event,
            commands::calendar::calendar_open_join_url,
            commands::calendar::change_meeting_reminder_lead_setting,
            commands::calendar::change_meeting_reminder_all_events_setting,
            // M5-P5f: Microsoft Graph
            commands::calendar::calendar_graph_sign_in,
            commands::calendar::calendar_graph_cancel_sign_in,
            commands::calendar::calendar_graph_sign_out,
            commands::calendar::change_calendar_graph_client_id_setting,
            commands::calendar::change_calendar_graph_tenant_setting,
            // M6-P6e
            commands::meeting_mcp::change_meeting_mcp_enabled_setting,
            commands::meeting_mcp::change_meeting_mcp_include_transcript_setting,
            commands::meeting_mcp::meeting_mcp_info,
            commands::meetings::meetings_start_from_event,
            meeting_prompt::meeting_prompt_current,
            meeting_prompt::meeting_prompt_ready,
            meeting_prompt::meeting_prompt_dismiss,
            // M5-P5d/P5e
            commands::people::people_list,
            commands::people::people_get,
            commands::people::people_merge,
            commands::people::people_update,
            commands::people::meeting_participants,
            commands::people::people_brief_info,
            commands::people::people_brief_open,
            commands::people::people_brief_pending,
            commands::tts::tts_speak_text,
            commands::tts::tts_speak_clipboard,
            commands::tts::tts_cancel,
            commands::tts::tts_server_kill,
            commands::tts::tts_translate,
            commands::tts::tts_cached_translation,
            commands::tts::tts_save_seed_voice,
            commands::tts::llm_ps,
            commands::tts::llm_unload,
            commands::tts::llm_warm,
            commands::pages::pages_list,
            books::books_list,
            books::books_create,
            books::books_update,
            books::books_delete,
            books::books_add_page,
            books::books_memory_read,
            books::books_memory_write,
            books::books_templates,
            books::books_template_save,
            books::books_template_reset,
            books::books_generate,
            books::books_export_preview,
            books::books_export,
            books::books_inspect,
            books::books_import,
            commands::pages_package::pages_export_preview,
            commands::pages_package::pages_export,
            commands::pages_package::pages_package_inspect,
            commands::pages_package::pages_import,
            commands::pages::pages_create,
            commands::pages::pages_rename,
            commands::pages::pages_delete,
            commands::pages::pages_reorder,
            commands::pages::page_state_load,
            commands::pages::page_state_save,
            commands::pages::page_dir,
            commands::pages::page_files,
            commands::pages::page_audio_note,
            commands::pages::page_file_delete,
            commands::pages::page_file_rename,
            commands::pages::page_file_add,
            commands::pages::page_file_open,
            commands::tts::tts_dictate_start,
            commands::tts::tts_dictate_stop,
            commands::tts::tts_server_start,
            commands::tts::tts_server_stop,
            commands::tts::tts_speak_text_from,
            commands::tts::tts_prewarm,
            commands::tts::tts_prewarm_cancel,
            commands::tts::tts_server_status,
            commands::tts::tts_list_voices,
            commands::tts::tts_voice_demo,
            commands::tts::tts_voice_demo_cached,
            commands::tts::tts_record_reference_start,
            commands::tts::tts_record_reference_stop,
            commands::tts::tts_transcribe_reference,
            commands::tts::tts_save_voice,
            commands::tts::tts_import_voice,
            commands::tts::tts_delete_voice,
            commands::tts::tts_translate_speak,
            commands::tts::tts_record_translate_start,
            commands::tts::tts_record_translate_stop,
            commands::tts::tts_reading_open,
            commands::tts::tts_reading_play,
            commands::tts::tts_reading_pause,
            commands::tts::tts_reading_list,
            commands::tts::tts_reading_reset,
            commands::tts::tts_reading_remove,
            commands::tts::tts_reading_seek,
            commands::tts::tts_speak_resume,
            commands::tts::tts_export_format,
            commands::tts::tts_summarize_text,
            commands::tts::tts_tidy_text,
            commands::tts::tts_extract_document,
            commands::tts::tts_extract_url,
            commands::tts::tts_voicechange_record_start,
            commands::tts::tts_voicechange_record_stop,
            commands::tts::tts_voicechange_file,
            commands::tts::tts_speak_to_file,
            commands::tts::tts_export_cancel,
            commands::tts::tts_speak_seek,
            commands::tts::tts_synthesize_to_file,
            commands::tts::tts_list_downloads,
            commands::tts::tts_download_model,
            commands::tts::tts_cancel_download,
            commands::tts::tts_delete_model,
            commands::tts::tts_list_voice_infos,
            commands::tts::tts_get_voice_meta,
            commands::tts::tts_set_voice_meta,
            commands::tts::tts_set_voice_avatar,
            commands::tts::tts_clear_voice_avatar,
            commands::tts::tts_save_style_reference,
            commands::tts::tts_delete_style,
            commands::tts::tts_analyze_reference,
            commands::tts::tts_analyze_pending_reference,
            commands::tts::tts_seed_preview,
            commands::tts::tts_save_seed_voice_v2,
            commands::tts::tts_auto_tag,
            commands::tts::tts_auto_tag_cancel,
            commands::tts::tts_builder_create_draft,
            commands::tts::tts_builder_list_drafts,
            commands::tts::tts_builder_update_draft,
            commands::tts::tts_builder_delete_draft,
            commands::tts::tts_builder_generate,
            commands::tts::tts_builder_cancel,
            commands::tts::tts_builder_candidate_wav,
            commands::tts::tts_builder_add_wav,
            commands::tts::tts_builder_commit,
            commands::tts::tts_export_voice,
            commands::tts::tts_export_voices,
            commands::tts::tts_inspect_voice_archive,
            commands::tts::tts_import_voice_archive,
            commands::tts::tts_rename_voice_id,
            helpers::clamshell::is_laptop,
        ])
        .events(collect_events![
            managers::history::HistoryUpdatePayload,
            managers::meetings::recorder::MeetingEvent,
            // M1-P1b
            commands::meeting_enhance::MeetingNotesEvent,
            // P1k
            commands::meeting_minutes::MinutesEvent,
            // M4-P4b
            managers::meetings::search::indexer::MeetingIndexEvent,
            // M5-P5c
            managers::meeting_detect::MeetingDetectEvent,
            // M4-P4c
            commands::meeting_chat::MeetingChatEvent,
            // M3-P3c
            commands::meeting_speakers::SpeakersChanged,
            // M5-P5b
            managers::calendar::service::CalendarSyncEvent,
            meeting_prompt::MeetingPromptEvent,
            commands::people::BriefRequestEvent,
            managers::transcription::StreamTextEvent,
            managers::transcription::StreamPhaseEvent,
        ]);

    #[cfg(debug_assertions)] // <- Only export on non-release builds
    specta_builder
        .export(
            Typescript::default().bigint(BigIntExportBehavior::Number),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    let invoke_handler = specta_builder.invoke_handler();

    // The headless path must run as its own instance (see the single-instance
    // note below), not forward to an already-running app.
    let headless_mode = cli_args.transcribe_file.is_some()
        || cli_args.list_devices
        || cli_args.list_models
        || cli_args.tts_test
        || cli_args.import_meeting.is_some()
        || cli_args.dump_meeting.is_some()
        || cli_args.make_orphan.is_some()
        || cli_args.bench_search // M4-P4a
        || cli_args.simulate_meeting // M2-P2c2
        || cli_args.eval_notes.is_some() // M1-P1e
        || cli_args.eval_minutes.is_some() // M1-P1k
        || cli_args.reindex_meetings // M4-P4b
        || cli_args.eval_diarization.is_some() // M3-P3a
        || cli_args.eval_chat.is_some() // M4-P4f
        || cli_args.export_meeting.is_some() // M6-P6a
        || cli_args.calendar_dump.is_some() // M5-P5a
        || cli_args.detect_mic; // M5-P5c

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .device_event_filter(tauri::DeviceEventFilter::Always)
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            LogBuilder::new()
                .level(log::LevelFilter::Trace) // Set to most verbose level globally
                .max_file_size(500_000)
                .rotation_strategy(RotationStrategy::KeepOne)
                .clear_targets()
                .targets([
                    // Console output respects RUST_LOG environment variable. In
                    // headless mode (--transcribe-file/--list-devices/--list-models)
                    // stdout carries only the result (JSON or plain), so send console
                    // logs to stderr instead to keep stdout clean for CI parsing.
                    Target::new(if headless_mode {
                        TargetKind::Stderr
                    } else {
                        TargetKind::Stdout
                    })
                    .filter({
                        let console_filter = console_filter.clone();
                        move |metadata| console_filter.enabled(metadata)
                    }),
                    // File logs respect the user's settings (stored in FILE_LOG_LEVEL atomic)
                    Target::new(if let Some(data_dir) = portable::data_dir() {
                        TargetKind::Folder {
                            path: data_dir.join("logs"),
                            file_name: Some("handy".into()),
                        }
                    } else {
                        TargetKind::LogDir {
                            file_name: Some("handy".into()),
                        }
                    })
                    .filter(|metadata| {
                        let file_level = FILE_LOG_LEVEL.load(Ordering::Relaxed);
                        metadata.level() <= level_filter_from_u8(file_level)
                    }),
                    // Stream logs to the webview (via the `log://log` event) so the
                    // debug panel's live log viewer can show them in real time. Only
                    // active while debug mode is on (its sole consumer), and shares the
                    // file log level so the "Log Level" setting controls verbosity.
                    Target::new(TargetKind::Webview).filter(|metadata| {
                        WEBVIEW_LOG_STREAMING.load(Ordering::Relaxed)
                            && metadata.level()
                                <= level_filter_from_u8(FILE_LOG_LEVEL.load(Ordering::Relaxed))
                    }),
                ])
                .build(),
        );

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    // Single-instance forwards CLI args to an already-running Handy and exits.
    // That would make the headless path
    // (--transcribe-file/--list-devices/--list-models) a silent no-op whenever the
    // app is already open, so skip it in headless mode and run a standalone
    // instance instead.
    if !headless_mode {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--toggle-transcription") {
                signal_handle::send_transcription_input(app, "transcribe", "CLI");
            } else if args.iter().any(|a| a == "--toggle-post-process") {
                signal_handle::send_transcription_input(app, "transcribe_with_post_process", "CLI");
            } else if args.iter().any(|a| a == "--cancel") {
                crate::utils::cancel_current_operation(app);
            } else if let Some(pos) = args.iter().position(|a| a == "--read-file") {
                // Explorer-Kontextmenü bei bereits laufender App: Dokument
                // öffnen und sofort vorlesen.
                if let Some(path) = args.get(pos + 1).cloned() {
                    show_main_window(app);
                    let tts = app
                        .state::<Arc<managers::tts::TtsManager>>()
                        .inner()
                        .clone();
                    match tts.reading_open(&path) {
                        Ok(_) => {
                            if let Err(e) = tts.reading_play() {
                                log::error!("--read-file play failed: {e}");
                            }
                        }
                        Err(e) => log::error!("--read-file open failed: {e}"),
                    }
                }
            } else {
                show_main_window(app);
            }
        }));
    }

    // In-app updates and remembered window geometry. Both are desktop-only and
    // pointless in a headless run (no window, no user to prompt).
    if !headless_mode {
        builder = builder
            .plugin(tauri_plugin_updater::Builder::new().build())
            // The recording overlay places itself (overlay.rs) — restoring a
            // saved position would fight that logic, so it is excluded.
            .plugin(
                tauri_plugin_window_state::Builder::default()
                    .with_denylist(&["recording_overlay"])
                    .build(),
            );
    }

    builder
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_macos_permissions::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(cli_args.clone())
        .setup(move |app| {
            specta_builder.mount_events(app);

            // Headless one-shot path (`--transcribe-file` / `--list-devices` /
            // `--list-models`): initialize only what transcription needs — the
            // store/paths plugins, the model + transcription managers, and the
            // transcribe-cpp backend + accelerator settings — then run on a worker
            // thread and exit. Deliberately skips the window, tray, overlay, audio
            // recorder (so it never opens the mic, even with always_on_microphone),
            // signal handlers, and autostart that initialize_core_logic sets up.
            if headless_mode {
                // TTS-Selbsttest: braucht weder Modelle noch Mikrofon — nur den
                // TtsManager. Misst Serverstart und Synthese, validiert das WAV
                // und beendet den Prozess mit einem CI-tauglichen Exit-Code.
                if cli_args.tts_test {
                    let app_handle = app.handle().clone();
                    let args = cli_args.clone();
                    std::thread::spawn(move || {
                        let code = run_headless_guarded(|| {
                            crate::selftest::begin_headless_run();
                            let tts = managers::tts::TtsManager::new(&app_handle);
                            let text = args.tts_text.clone().unwrap_or_else(|| {
                                "Dies ist der Selbsttest der lokalen Sprachausgabe.".to_string()
                            });
                            let result = tauri::async_runtime::block_on(
                                tts.bench_fetch(&text, args.tts_voice.as_deref()),
                            );
                            let code = match result {
                                Ok((wav, start_ms, tts_ms)) => {
                                    let bytes = wav.len();
                                    if let Some(path) = args.tts_out_wav.as_deref() {
                                        match std::fs::write(path, &wav) {
                                            Ok(()) => eprintln!("wrote {}", path.display()),
                                            Err(e) => eprintln!(
                                                "error: could not write {}: {}",
                                                path.display(),
                                                e
                                            ),
                                        }
                                    }
                                    let payload = serde_json::json!({
                                        "mode": "tts",
                                        "wav_bytes": bytes,
                                        "server_start_ms": start_ms,
                                        "tts_ms": tts_ms,
                                    });
                                    if args.json {
                                        emit_headless_payload(&payload, args.out.as_deref());
                                    } else {
                                        if let Some(path) = args.out.as_deref() {
                                            emit_headless_payload(&payload, Some(path));
                                        }
                                        println!(
                                            "tts ok: {} bytes, server_start={}ms, tts={}ms",
                                            bytes, start_ms, tts_ms
                                        );
                                    }
                                    0
                                }
                                Err(e) => {
                                    eprintln!("error: tts self-test failed: {e}");
                                    1
                                }
                            };
                            // Nur selbst gestartete Server wieder stoppen.
                            tts.stop_server();
                            code
                        });
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(code);
                    });
                    return Ok(());
                }

                // M5-P5a: Kalender lesen (ICS-Datei oder Adresse), Serien
                // expandieren, Termine ausgeben. Keine Datenbank, keine Modelle.
                if cli_args.calendar_dump.is_some() {
                    let args = cli_args.clone();
                    std::thread::spawn(move || {
                        let code = run_headless_guarded(|| run_headless_calendar_dump(&args));
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(code);
                    });
                    return Ok(());
                }

                // M5-P5c: Mikrofonnutzung beobachten (Registry, nur lesend).
                // Keine Datenbank, keine Modelle, kein Mikrofonzugriff.
                if cli_args.detect_mic {
                    let args = cli_args.clone();
                    std::thread::spawn(move || {
                        let code = run_headless_guarded(|| run_headless_detect_mic(&args));
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(code);
                    });
                    return Ok(());
                }

                // M4-P4a: Performance des Such-Index auf einer synthetischen
                // Sandbox-DB im Temp-Verzeichnis (nie die produktive meetings.db).
                // Braucht weder Modelle noch Mikrofon; Exit 0 ok, 1 Fehler,
                // 3 wenn ein p95 die 500-ms-Grenze erreicht.
                if cli_args.bench_search {
                    let args = cli_args.clone();
                    std::thread::spawn(move || {
                        let code = run_headless_guarded(|| run_headless_bench_search(&args));
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(code);
                    });
                    return Ok(());
                }

                // M4-P4b: Such-Index neu aufbauen (lexikalisch + Vektoren) und
                // den Embedding-Server am Ende beenden. Braucht kein Mikrofon
                // und kein Sprachmodell fuer die Transkription.
                if cli_args.reindex_meetings {
                    let app_handle = app.handle().clone();
                    let args = cli_args.clone();
                    std::thread::spawn(move || {
                        let code = run_headless_guarded(|| {
                            run_headless_reindex_meetings(&app_handle, &args)
                        });
                        // Nie einen Kindprozess zuruecklassen, auch nach Panik.
                        managers::llm::stop_embedding();
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(code);
                    });
                    return Ok(());
                }

                // M1-P1e: KI-Notizen-Eval (AK3) auf synthetischen Fixtures in
                // einem Sandbox-Store; braucht nur das Sprachmodell.
                if let Some(dir) = cli_args.eval_notes.clone() {
                    let app_handle = app.handle().clone();
                    let args = cli_args.clone();
                    std::thread::spawn(move || {
                        let code = run_headless_guarded(|| {
                            run_headless_eval_notes(&app_handle, &args, &dir)
                        });
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(code);
                    });
                    return Ok(());
                }

                // M1-P1k: Vorlagenwahl und Protokoll auf synthetischen Fixtures in
                // einem Sandbox-Store; braucht nur das Sprachmodell.
                if let Some(dir) = cli_args.eval_minutes.clone() {
                    let app_handle = app.handle().clone();
                    let args = cli_args.clone();
                    std::thread::spawn(move || {
                        let code = run_headless_guarded(|| {
                            run_headless_eval_minutes(&app_handle, &args, &dir)
                        });
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(code);
                    });
                    return Ok(());
                }

                // M4-P4f: Chat-Eval (AK8) auf synthetischen Fixtures in einem
                // Sandbox-Store; Sprachmodell und Embedding-Server werden am
                // Ende beendet.
                if let Some(dir) = cli_args.eval_chat.clone() {
                    let app_handle = app.handle().clone();
                    let args = cli_args.clone();
                    std::thread::spawn(move || {
                        let code = run_headless_guarded(|| {
                            run_headless_eval_chat(&app_handle, &args, &dir)
                        });
                        // Nach Panik: Embedding-Server ausdruecklich; der
                        // LLM-Server stirbt mit dem Job-Objekt (KILL_ON_JOB_CLOSE).
                        managers::llm::stop_embedding();
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(code);
                    });
                    return Ok(());
                }

                // M3-P3a: Sprechertrennung (DER, AK7) auf Paaren
                // <name>.wav + <name>.rttm; braucht nur das Diarisierungsmodell.
                if let Some(dir) = cli_args.eval_diarization.clone() {
                    let app_handle = app.handle().clone();
                    let args = cli_args.clone();
                    std::thread::spawn(move || {
                        let code = run_headless_guarded(|| {
                            run_headless_eval_diarization(&app_handle, &args, &dir)
                        });
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(code);
                    });
                    return Ok(());
                }

                // M6-P6a: Export einer Besprechung. Nur lesend: kein Modell,
                // keine Aufnahme, keine Aufraeumarbeiten beim Start.
                if let Some(id) = cli_args.export_meeting.clone() {
                    let app_handle = app.handle().clone();
                    let args = cli_args.clone();
                    std::thread::spawn(move || {
                        let code = run_headless_guarded(|| {
                            run_headless_export_meeting(&app_handle, &args, &id)
                        });
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(code);
                    });
                    return Ok(());
                }

                let app_handle = app.handle().clone();
                let model_manager = Arc::new(
                    ModelManager::new(&app_handle).expect("Failed to initialize model manager"),
                );
                let transcription_manager = Arc::new(
                    TranscriptionManager::new(&app_handle, model_manager.clone())
                        .expect("Failed to initialize transcription manager"),
                );
                app_handle.manage(model_manager);
                app_handle.manage(transcription_manager);
                managers::transcription::init_transcribe_backend();
                managers::transcription::apply_accelerator_settings(&app_handle);

                let handle = app_handle.clone();
                let args = cli_args.clone();
                let meetings_mode = args.import_meeting.is_some()
                    || args.dump_meeting.is_some()
                    || args.make_orphan.is_some()
                    || args.simulate_meeting;
                std::thread::spawn(move || {
                    let code = if meetings_mode {
                        run_headless_guarded(|| run_headless_meetings(&handle, &args))
                    } else {
                        run_headless_guarded(|| run_headless_transcription(&handle, &args))
                    };
                    // Drop the loaded engine before teardown: ggml-metal's global
                    // device free asserts (SIGABRT) if a model's Metal resources
                    // are still alive at C++ static-destructor time.
                    if let Some(tm) = handle.try_state::<Arc<TranscriptionManager>>() {
                        let _ = tm.unload_model();
                    }
                    // process::exit (not app.exit, which exits 0 regardless) so the
                    // exit code propagates to the shell for CI gating. Flush first
                    // since process::exit runs no destructors / buffer flushes.
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    let _ = std::io::stderr().flush();
                    std::process::exit(code);
                });
                return Ok(());
            }

            // Create main window programmatically so we can set data_directory
            // for portable mode (redirects WebView2 cache to portable Data dir)
            let mut win_builder =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("/".into()))
                    .title("Local Voice AI")
                    // Start size; the persisted geometry (tauri-plugin-window-state)
                    // overrides this from the second launch on.
                    .inner_size(800.0, 600.0)
                    // Deliberately well below the start size: the layout is
                    // responsive down to a single narrow column (sidebar
                    // collapses to icons), so the window may be made small.
                    .min_inner_size(480.0, 420.0)
                    .resizable(true)
                    .maximizable(true)
                    // Centred rather than wherever Windows would cascade it.
                    // The window is built hidden and shown later, so this is
                    // the placement the user sees on first appearance.
                    .center()
                    .visible(false);

            // Set the taskbar icon explicitly. Windows does not fall back to
            // the executable's icon for a window whose icon was never set, so
            // without this the frame kept showing the icon of whatever was
            // baked in previously — and no amount of rebuilding changes that.
            // Embedded at compile time so it cannot go missing next to an
            // unbundled binary.
            match Image::from_bytes(include_bytes!("../icons/128x128.png")) {
                Ok(icon) => win_builder = win_builder.icon(icon).expect("window icon"),
                Err(error) => log::warn!("Could not load the window icon: {error}"),
            }

            if let Some(data_dir) = portable::data_dir() {
                win_builder = win_builder.data_directory(data_dir.join("webview"));
            }

            win_builder.build()?;

            let mut settings = get_settings(app.handle());

            // Apply the persisted appearance theme to the Windows title bar before
            // the window is shown, so it matches the in-app palette without a flash
            // of the wrong theme. On macOS/Linux, Tauri themes are app-wide and
            // would also affect windows that intentionally keep the system theme.
            #[cfg(target_os = "windows")]
            shortcut::apply_window_theme(app.handle(), settings.theme);

            // CLI --debug flag overrides debug_mode and log level (runtime-only, not persisted)
            if cli_args.debug {
                settings.debug_mode = true;
                settings.log_level = settings::LogLevel::Trace;
            }

            let tauri_log_level: tauri_plugin_log::LogLevel = settings.log_level.into();
            let file_log_level: log::Level = tauri_log_level.into();
            // Store the file log level in the atomic for the filter to use
            FILE_LOG_LEVEL.store(file_log_level.to_level_filter() as u8, Ordering::Relaxed);
            // Only forward logs to the webview while debug mode is on (the live log
            // viewer is the sole consumer and only exists in debug mode). This also
            // honors the runtime `--debug` override applied to `settings` above.
            WEBVIEW_LOG_STREAMING.store(settings.debug_mode, Ordering::Relaxed);
            let app_handle = app.handle().clone();
            app.manage(TranscriptionCoordinator::new(app_handle.clone()));

            initialize_core_logic(&app_handle);

            // Kontextmenü-Eintrag mit dem Setting synchronisieren (aktualisiert
            // auch den EXE-Pfad, wenn Builds wandern).
            #[cfg(windows)]
            if let Err(e) = context_menu::sync(settings.tts_context_menu) {
                log::warn!("context menu sync failed: {e}");
            }

            // Kontextmenü-/CLI-Start mit Dokument: öffnen und sofort vorlesen.
            if let Some(path) = cli_args.read_file.clone() {
                let handle = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    let tts = handle
                        .state::<Arc<managers::tts::TtsManager>>()
                        .inner()
                        .clone();
                    match tts.reading_open(&path.to_string_lossy()) {
                        Ok(_) => {
                            if let Err(e) = tts.reading_play() {
                                log::error!("--read-file play failed: {e}");
                            }
                        }
                        Err(e) => log::error!("--read-file open failed: {e}"),
                    }
                });
            }

            // Populate the overlay-enabled cache from initial settings so the
            // audio path (overlay::emit_levels, called ~24 Hz during recording)
            // can do a single atomic load instead of reading the Tauri store.
            // Kept in sync by shortcut::change_overlay_style_setting.
            overlay::update_overlay_enabled_cache(
                settings.overlay_style != settings::OverlayStyle::None,
            );

            // Pre-warm GPU/accelerator enumeration on a background thread. The first
            // get_available_accelerators call enumerates ORT execution providers and
            // transcribe-cpp compute devices, which can take a moment; without this
            // the cost is paid synchronously when the user first opens Advanced
            // settings, freezing the UI. Result is cached in a OnceLock.
            std::thread::spawn(|| {
                let _ = crate::managers::transcription::get_available_accelerators();
            });

            // Hide tray icon if --no-tray was passed
            if cli_args.no_tray {
                tray::set_tray_visibility(&app_handle, false);
            }

            // Show main window only if not starting hidden.
            // CLI --start-hidden flag overrides the setting.
            // But if permission onboarding is required, always show the window.
            let should_hide = settings.start_hidden || cli_args.start_hidden;
            let should_force_show = should_force_show_permissions_window(&app_handle);

            // If start_hidden but tray is disabled, we must show the window
            // anyway. Without a tray icon, the dock is the only way back in.
            let tray_available = settings.show_tray_icon && !cli_args.no_tray;
            if should_force_show || !should_hide || !tray_available {
                show_main_window(&app_handle);
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _res = window.hide();

                #[cfg(target_os = "macos")]
                {
                    let settings = get_settings(window.app_handle());
                    let tray_visible =
                        settings.show_tray_icon && !window.app_handle().state::<CliArgs>().no_tray;
                    if tray_visible {
                        // Tray is available: hide the dock icon, app lives in the tray
                        let res = window
                            .app_handle()
                            .set_activation_policy(tauri::ActivationPolicy::Accessory);
                        if let Err(e) = res {
                            log::error!("Failed to set activation policy: {}", e);
                        }
                    }
                    // No tray: keep the dock icon visible so the user can reopen
                }
            }
            tauri::WindowEvent::ThemeChanged(theme) => {
                log::info!("Theme changed to: {:?}", theme);
                // Re-apply the current tray state with the new theme's icon set
                utils::refresh_tray_icon(window.app_handle());
            }
            _ => {}
        })
        .invoke_handler(invoke_handler)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match &event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                show_main_window(app);
            }
            // Teardown transcribe.cpp before exit
            tauri::RunEvent::Exit => {
                if let Some(tm) = app.try_state::<Arc<TranscriptionManager>>() {
                    let _ = tm.unload_model();
                }
                // Der lokale Sprachmodell-Server ist unser Kind: er stirbt mit
                // der App, ueber seine PID, nie ueber den Prozessnamen.
                if let Some(llm) = app.try_state::<Arc<managers::llm::LocalLlmServer>>() {
                    llm.stop();
                }
                // M4-P4b: der Embedding-Server ebenso.
                managers::llm::stop_embedding();
                // Kein Serverprozess ueberlebt die Anwendung — auch keiner,
                // den wir nur adoptiert haben. Er haelt rund 17 GB VRAM, und
                // nach dem Ende der App gibt es niemanden mehr, der ihn
                // beenden koennte: der Nutzer muesste in den Taskmanager.
                // `stop_server` beendet den eigenen Prozessbaum UND alles,
                // was noch auf dem TTS-Port lauscht.
                if let Some(tts) = app.try_state::<Arc<managers::tts::TtsManager>>() {
                    tts.stop_server();
                }
            }
            _ => {}
        });
}

// M5-P5a: `--calendar-dump <datei|url> [--from D] [--to D]`. Liest einen
// Kalender wie der Sync-Dienst (Abruf, Parser, Serienexpansion) und gibt die
// Termine im Fenster aus; die Adresse erscheint nie in der Ausgabe. JSON auf
// stdout mit `--json`, sonst eine Tabelle; `--out` schreibt das JSON in eine
// Datei (das Release-Binary hat kein Konsolen-Subsystem). Exit 0 gelesen,
// 1 Lese-/Abruf-/Parse-Fehler, 2 falscher Aufruf.
fn run_headless_calendar_dump(args: &CliArgs) -> i32 {
    let target = args.calendar_dump.clone().unwrap_or_default();
    let (code, payload) = managers::calendar::dump::run_cli(
        &target,
        args.cal_from.as_deref(),
        args.cal_to.as_deref(),
        chrono::Utc::now().timestamp_millis(),
    );
    if let Some(error) = payload.get("error").and_then(|e| e.as_str()) {
        eprintln!("error: calendar-dump failed: {error}");
    }
    if args.json {
        emit_headless_payload(&payload, args.out.as_deref());
    } else {
        println!("{}", managers::calendar::dump::format_table(&payload));
        if let Some(path) = args.out.as_deref() {
            match std::fs::write(
                path,
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            ) {
                Ok(()) => eprintln!("wrote {}", path.display()),
                Err(e) => eprintln!("error: could not write {}: {}", path.display(), e),
            }
        }
    }
    code
}

// M5-P5c: `--detect-mic --seconds N [--all-apps] [--json] [--out F]`. Beobachtet
// N Sekunden lang, welche Programme das Mikrofon oeffnen, mit denselben Regeln
// wie der Watcher (5 s Entprellung, Eigenfilter, Leichen, webview2), nur mit
// 1 s statt 2 s Abfrageabstand, damit eine Messung nach dem Oeffnen schneller
// ein Ergebnis hat. `--all-apps` meldet auch Programme ausserhalb des
// Katalogs. Nur lesend. Exit 0 beobachtet, 1 Registry nicht lesbar,
// 2 falscher Aufruf.
fn run_headless_detect_mic(args: &CliArgs) -> i32 {
    let (code, payload) = managers::meeting_detect::run_cli(
        args.seconds.unwrap_or(15),
        args.all_apps,
        std::time::Duration::from_secs(1),
    );
    if let Some(error) = payload.get("error").and_then(|e| e.as_str()) {
        eprintln!("error: detect-mic failed: {error}");
    }
    if args.json {
        emit_headless_payload(&payload, args.out.as_deref());
    } else {
        println!("{}", managers::meeting_detect::format_table(&payload));
        if let Some(path) = args.out.as_deref() {
            if let Err(e) = std::fs::write(
                path,
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            ) {
                eprintln!("error: could not write {}: {}", path.display(), e);
            }
        }
    }
    code
}

// M4-P4a: `--bench-search`. Misst Listensuche, Wortsuche und hybride Suche auf
// einer synthetischen Sandbox-Datenbank (Temp-Ordner, wird danach geloescht).
// Ausgabe wie die anderen Headless-Laeufe: JSON auf stdout und mit `--out` in
// eine Datei (das Release-Binary hat kein Konsolen-Subsystem). Exit 0 bestanden,
// 1 Fehler, 3 wenn `ui_search_p95_ms` oder `hybrid_p95_ms` 500 ms erreichen.
fn run_headless_bench_search(args: &CliArgs) -> i32 {
    let (code, payload) = managers::meetings::search::bench::run_cli(
        args.meetings,
        args.chunks,
        args.repeat,
        args.bench_dir.clone(),
    );
    if let Some(error) = payload.get("error").and_then(|e| e.as_str()) {
        eprintln!("error: bench-search failed: {error}");
    }
    emit_headless_payload(&payload, args.out.as_deref());
    code
}

// M6-P6a: `--export-meeting <id> --format <f> --out <datei>`. Liest die
// Besprechung (`LVA_MEETINGS_DIR` wird beachtet) und schreibt sie im Format;
// ohne Modell, ohne Aufraeumen beim Start. Exit 0 ok, 1 Fehler, 2 Eingabe
// (unbekannte Besprechung oder Format, kein --out).
fn run_headless_export_meeting(app: &AppHandle, args: &CliArgs, id: &str) -> i32 {
    use managers::meetings::export::{build_bundle, write_export, ExportFormat, ExportParts};
    use managers::meetings::store::MeetingStore;

    crate::selftest::begin_headless_run();
    let Some(out) = args.out.as_deref() else {
        eprintln!("error: --export-meeting needs --out <file>");
        return 2;
    };
    let format = match args.format.as_deref() {
        Some(name) => match ExportFormat::from_extension(name) {
            Some(format) => format,
            None => {
                eprintln!(
                    "error: unknown --format '{name}' (md, txt, docx, html, pdf, srt, vtt, json)"
                );
                return 2;
            }
        },
        None => ExportFormat::from_path(out),
    };
    let store = match MeetingStore::new(app) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("error: meetings store unavailable: {e}");
            return 1;
        }
    };
    let bundle = match build_bundle(&store, id) {
        Ok(bundle) => bundle,
        Err(e) if e.starts_with("meeting_not_found") => {
            eprintln!("error: no meeting {id}");
            return 2;
        }
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match write_export(out, format, &bundle, &ExportParts::all()) {
        Ok(()) => {
            eprintln!("wrote {}", out.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

// M4-P4b: `--reindex-meetings [--seed-meetings DIR]`. Baut den Such-Index der
// Besprechungen (`LVA_MEETINGS_DIR` wird beachtet) neu auf: Chunks und FTS
// sofort, dann Vektoren ueber den Embedding-Server, der am Ende beendet wird.
// Ausgabe wie die anderen Headless-Laeufe: JSON auf stdout und mit `--out` in
// eine Datei. Exit 0 fertig (ohne Modell nur lexikalisch), 1 Fehler, 2 falscher
// Aufruf, 3 Modell vorhanden, aber nicht alle Chunks eingebettet.
fn run_headless_reindex_meetings(app: &AppHandle, args: &CliArgs) -> i32 {
    crate::selftest::begin_headless_run();
    use managers::meetings::search::{embed::LlamaEmbedder, indexer, vectors::global_cache};
    use managers::meetings::store::MeetingStore;

    let sandbox = std::env::var(managers::meetings::MEETINGS_DIR_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty());
    if args.seed_meetings.is_some() && sandbox.is_none() {
        eprintln!(
            "error: --seed-meetings writes test meetings and requires {} (sandbox)",
            managers::meetings::MEETINGS_DIR_ENV
        );
        return 2;
    }
    let store = match MeetingStore::new(app) {
        Ok(store) => Arc::new(store),
        Err(e) => {
            eprintln!("error: meetings store unavailable: {e}");
            return 1;
        }
    };
    eprintln!("DB={}", store.db_path().display());
    let mut seeded = 0usize;
    if let Some(dir) = args.seed_meetings.as_deref() {
        match seed_meetings_from_fixtures(&store, dir) {
            Ok(n) => seeded = n,
            Err(e) => {
                eprintln!("error: --seed-meetings failed: {e}");
                return 1;
            }
        }
    }
    let runtime = match managers::llm::LlmRuntimeManager::new(app) {
        Ok(runtime) => Arc::new(runtime),
        Err(e) => {
            eprintln!("error: llm runtime unavailable: {e}");
            return 1;
        }
    };
    managers::llm::install_globals(runtime, Arc::new(managers::llm::LocalLlmServer::new()));
    // Auch headless gilt der Speicherwaechter: faellt der freie RAM unter die
    // Notgrenze, wird der Embedding-Server beendet (die Charge scheitert dann
    // mit einem Code, nichts Halbes wird gespeichert).
    process_guard::spawn_memory_watchdog(|free_mb| {
        managers::llm::stop_embedding();
        eprintln!("memory watchdog stopped the embedding server at {free_mb} MB free");
    });
    let embed = Arc::new(LlamaEmbedder::new(managers::llm::EMBED_MODEL_ID));
    let report = indexer::reindex_all(store.clone(), embed, global_cache(), true, &|| {
        managers::llm::embedding_pid()
    });
    managers::llm::stop_embedding();
    match report {
        Ok(report) => {
            let mut payload = serde_json::to_value(&report).unwrap_or_default();
            payload["mode"] = serde_json::json!("reindex_meetings");
            payload["seeded"] = serde_json::json!(seeded);
            payload["db"] = serde_json::json!(store.db_path().display().to_string());
            payload["embed_server_running"] =
                serde_json::json!(managers::llm::embedding_running());
            emit_headless_payload(&payload, args.out.as_deref());
            if report.model_ready && report.vectors != report.chunks {
                3
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("error: reindex failed: {e}");
            1
        }
    }
}

/// Eine Besprechungs-Fixture (Format der M1-Eval, `tests/fixtures/notes`).
#[derive(serde::Deserialize)]
struct MeetingFixture {
    title: String,
    segments: Vec<managers::meetings::store::StoredSegment>,
    #[serde(default)]
    notes: Vec<managers::meetings::notes::model::NoteBlock>,
}

/// Legt je `*.json` in `dir` eine fertige Besprechung (Import) an. Nur fuer
/// die Sandbox (`LVA_MEETINGS_DIR`), siehe `run_headless_reindex_meetings`.
fn seed_meetings_from_fixtures(
    store: &managers::meetings::store::MeetingStore,
    dir: &std::path::Path,
) -> anyhow::Result<usize> {
    use managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut count = 0usize;
    for file in files {
        let fixture: MeetingFixture = match serde_json::from_str(&std::fs::read_to_string(&file)?) {
            Ok(f) => f,
            Err(_) => {
                eprintln!("seed: {} ist keine Besprechungs-Fixture, uebersprungen", file.display());
                continue;
            }
        };
        let meeting = store.create_meeting(
            &fixture.title,
            MeetingSource::Import,
            Some(chrono::Utc::now().timestamp()),
        )?;
        store.append_delta(
            &meeting.id,
            &TranscriptDelta {
                new_segments: fixture.segments,
            },
        )?;
        if !fixture.notes.is_empty() {
            store.save_notes(&meeting.id, &fixture.notes, 0)?;
        }
        store.set_status(&meeting.id, MeetingStatus::Ready)?;
        count += 1;
    }
    Ok(count)
}

// M4-P4f: `--eval-chat <dir> [--lexical-only]`. Chat ueber Besprechungen
// gegen synthetische Fixtures (AK8) mit dem eingestellten (oder per `--model`
// gewaehlten lokalen) Sprachmodell. Sandbox-Store im Temp-Ordner, nie die
// produktive meetings.db. Globale Zugriffe des lokalen Servers wie in
// `run_headless_eval_notes`, Speicherwaechter fuer beide llama-server; beide
// werden am Ende gestoppt. Exit 0 Soll erfuellt, 3 verfehlt, 1 Fehler.
fn run_headless_eval_chat(app: &AppHandle, args: &CliArgs, dir: &std::path::Path) -> i32 {
    use managers::meetings::chat::eval;

    crate::selftest::begin_headless_run();
    let mut settings = get_settings(app);
    if let Some(model) = args.model.as_deref().filter(|m| !m.trim().is_empty()) {
        managers::meetings::notes::eval::apply_model_override(&mut settings, model.trim());
    }
    let llm_runtime = match managers::llm::LlmRuntimeManager::new(app) {
        Ok(runtime) => Arc::new(runtime),
        Err(e) => {
            eprintln!("error: LLM runtime unavailable: {e}");
            return 1;
        }
    };
    let llm_server = Arc::new(managers::llm::LocalLlmServer::new());
    managers::llm::install_globals(llm_runtime.clone(), llm_server.clone());
    {
        let llm = llm_server.clone();
        process_guard::spawn_memory_watchdog(move |free_mb| {
            llm.stop();
            managers::llm::stop_embedding();
            eprintln!("error: memory watchdog stopped the LLM servers at {free_mb} MB free");
        });
    }
    let backend_cpu = tauri::async_runtime::block_on(llm_runtime.resolve_runtime())
        .map(|(_, backend, _)| backend == "cpu")
        .unwrap_or(true);
    let sandbox = match tempfile::Builder::new().prefix("lva-eval-chat-").tempdir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("error: no sandbox directory: {e}");
            return 1;
        }
    };
    eprintln!("eval-chat: Sandbox {}", sandbox.path().display());
    let (code, payload) = eval::run_cli(
        settings,
        dir,
        sandbox.path(),
        eval::CliOptions {
            lexical_only: args.lexical_only,
            backend_cpu,
        },
    );
    managers::llm::stop_embedding();
    llm_server.stop();

    if args.json {
        emit_headless_payload(&payload, args.out.as_deref());
    } else {
        for line in eval::summary_lines(&payload) {
            println!("{line}");
        }
        if let Some(path) = args.out.as_deref() {
            match std::fs::write(
                path,
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            ) {
                Ok(()) => eprintln!("wrote {}", path.display()),
                Err(e) => eprintln!("error: could not write {}: {}", path.display(), e),
            }
        }
    }
    code
}

// M1-P1e: `--eval-notes <dir>`. KI-Notizen gegen die synthetischen Fixtures,
// mit dem eingestellten (oder per `--model` gewaehlten lokalen) Sprachmodell.
// Sandbox-Store im Temp-Ordner, nie die produktive meetings.db. Die globalen
// Zugriffe des lokalen Servers werden wie in `initialize_core_logic` gesetzt
// (ohne sie scheitert der lokale Anbieter headless mit "nicht initialisiert"),
// samt Speicherwaechter; der Server wird am Ende gestoppt. Exit 0 Soll
// erfuellt, 3 verfehlt, 1 Fehler.
fn run_headless_eval_notes(app: &AppHandle, args: &CliArgs, dir: &std::path::Path) -> i32 {
    use managers::meetings::notes::eval;

    crate::selftest::begin_headless_run();
    let mut settings = get_settings(app);
    if let Some(model) = args.model.as_deref().filter(|m| !m.trim().is_empty()) {
        eval::apply_model_override(&mut settings, model.trim());
    }

    let llm_runtime = match managers::llm::LlmRuntimeManager::new(app) {
        Ok(runtime) => Arc::new(runtime),
        Err(e) => {
            eprintln!("error: LLM runtime unavailable: {e}");
            return 1;
        }
    };
    let llm_server = Arc::new(managers::llm::LocalLlmServer::new());
    managers::llm::install_globals(llm_runtime, llm_server.clone());
    {
        let llm = llm_server.clone();
        process_guard::spawn_memory_watchdog(move |free_mb| {
            llm.stop();
            eprintln!("error: memory watchdog stopped the LLM server at {free_mb} MB free");
        });
    }

    let sandbox = match tempfile::Builder::new().prefix("lva-eval-notes-").tempdir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("error: no sandbox directory: {e}");
            return 1;
        }
    };
    eprintln!("eval-notes: Sandbox {}", sandbox.path().display());
    let (code, payload) =
        tauri::async_runtime::block_on(eval::run_cli(settings, dir, sandbox.path()));
    llm_server.stop();

    if args.json {
        emit_headless_payload(&payload, args.out.as_deref());
    } else {
        for line in eval::summary_lines(&payload) {
            println!("{line}");
        }
        if let Some(path) = args.out.as_deref() {
            match std::fs::write(
                path,
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            ) {
                Ok(()) => eprintln!("wrote {}", path.display()),
                Err(e) => eprintln!("error: could not write {}: {}", path.display(), e),
            }
        }
    }
    code
}

// M1-P1k: `--eval-minutes <dir> [--model id] [--minutes-template id|auto]`. Vorlagenwahl
// und Protokoll gegen synthetische Fixtures mit dem eingestellten (oder per
// `--model` gewaehlten lokalen) Sprachmodell. Sandbox-Store im Temp-Ordner, nie
// die produktive meetings.db; Server-Zugriffe und Speicherwaechter wie in
// `run_headless_eval_notes`. Exit 0 ok, 3 falsche Vorlage oder Luecke, 1 Fehler.
fn run_headless_eval_minutes(app: &AppHandle, args: &CliArgs, dir: &std::path::Path) -> i32 {
    use managers::meetings::notes::{eval, eval_minutes};

    crate::selftest::begin_headless_run();
    let mut settings = get_settings(app);
    if let Some(model) = args.model.as_deref().filter(|m| !m.trim().is_empty()) {
        eval::apply_model_override(&mut settings, model.trim());
    }
    let llm_runtime = match managers::llm::LlmRuntimeManager::new(app) {
        Ok(runtime) => Arc::new(runtime),
        Err(e) => {
            eprintln!("error: LLM runtime unavailable: {e}");
            return 1;
        }
    };
    let llm_server = Arc::new(managers::llm::LocalLlmServer::new());
    managers::llm::install_globals(llm_runtime, llm_server.clone());
    {
        let llm = llm_server.clone();
        process_guard::spawn_memory_watchdog(move |free_mb| {
            llm.stop();
            eprintln!("error: memory watchdog stopped the LLM server at {free_mb} MB free");
        });
    }
    let sandbox = match tempfile::Builder::new().prefix("lva-eval-minutes-").tempdir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("error: no sandbox directory: {e}");
            return 1;
        }
    };
    eprintln!("eval-minutes: Sandbox {}", sandbox.path().display());
    let template = args
        .minutes_template
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("auto");
    let (code, payload) = tauri::async_runtime::block_on(eval_minutes::run_cli(
        settings,
        dir,
        sandbox.path(),
        template,
    ));
    llm_server.stop();

    if args.json {
        emit_headless_payload(&payload, args.out.as_deref());
    } else {
        for line in eval_minutes::summary_lines(&payload) {
            println!("{line}");
        }
        if let Some(path) = args.out.as_deref() {
            match std::fs::write(
                path,
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            ) {
                Ok(()) => eprintln!("wrote {}", path.display()),
                Err(e) => eprintln!("error: could not write {}: {}", path.display(), e),
            }
        }
    }
    code
}

// M3-P3a: `--eval-diarization <dir> [--model id|pfad] [--collar s] [--rttm-out dir]`.
// DER je Datei und gewichtet je Gruppe (de / ami_dev / ami_test). Ein Modell
// zur Zeit (Diarisierer-Platz, RAM-Gate), das Modell wird einmal geladen und
// am Ende entladen. `--out` wird erst in eine Nachbardatei geschrieben und
// dann umbenannt. Exit 0 Ziele erfuellt, 3 verfehlt, 1 Fehler, 2 Eingabe/Modell.
fn run_headless_eval_diarization(app: &AppHandle, args: &CliArgs, dir: &std::path::Path) -> i32 {
    use managers::meetings::diarize::{self, der, DiarizeError};

    let models_root = match crate::portable::app_data_dir(app) {
        Ok(d) => d.join("models"),
        Err(e) => {
            eprintln!("error: no app data dir: {e}");
            return 1;
        }
    };
    let model_path = diarize::resolve_model_path(&models_root, args.model.as_deref());
    managers::transcription::init_transcribe_backend();
    let params = diarize::DiarizeParams::new(&model_path);
    eprintln!(
        "eval-diarization: Modell {} ({} Threads)",
        model_path.display(),
        params.threads
    );
    let (code, payload) = match diarize::Diarizer::load(&model_path, params.threads) {
        Ok(mut diarizer) => {
            let opts = der::EvalOptions {
                collar_s: args.collar.unwrap_or(der::DEFAULT_COLLAR_S),
                rttm_out: args.rttm_out.clone(),
                model: model_path.display().to_string(),
            };
            der::run_eval(&mut diarizer, dir, &params, &opts, |p| {
                managers::tts::voices::load_wav_mono_16k(p)
            })
        }
        Err(e) => {
            eprintln!("error: {e}");
            let code = if matches!(e, DiarizeError::ModelMissing(_) | DiarizeError::WrongModel(_)) {
                2
            } else {
                1
            };
            (
                code,
                serde_json::json!({"mode": "eval_diarization", "error": e.to_string()}),
            )
        }
    };

    if args.json {
        println!("{payload}");
    } else {
        for line in der::summary_lines(&payload) {
            println!("{line}");
        }
    }
    if let Some(path) = args.out.as_deref() {
        let text = serde_json::to_string_pretty(&payload).unwrap_or_default();
        match der::write_atomic(path, &text) {
            Ok(()) => eprintln!("wrote {}", path.display()),
            Err(e) => {
                eprintln!("error: could not write {e}");
                return 1;
            }
        }
    }
    code
}
