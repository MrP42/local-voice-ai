use crate::settings::SoundTheme;
use crate::settings::{self, AppSettings};
use cpal::traits::{DeviceTrait, HostTrait};
use log::{debug, error, warn};
use rodio::OutputStreamBuilder;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::thread;
use tauri::{AppHandle, Manager};

pub enum SoundType {
    Start,
    Stop,
}

fn resolve_sound_path(
    app: &AppHandle,
    settings: &AppSettings,
    sound_type: SoundType,
) -> Option<PathBuf> {
    let sound_file = get_sound_path(settings, sound_type);
    let base_dir = get_sound_base_dir(settings);
    match base_dir {
        tauri::path::BaseDirectory::AppData => {
            crate::portable::resolve_app_data(app, &sound_file).ok()
        }
        _ => app.path().resolve(&sound_file, base_dir).ok(),
    }
}

fn get_sound_path(settings: &AppSettings, sound_type: SoundType) -> String {
    match (settings.sound_theme, sound_type) {
        (SoundTheme::Custom, SoundType::Start) => "custom_start.wav".to_string(),
        (SoundTheme::Custom, SoundType::Stop) => "custom_stop.wav".to_string(),
        (_, SoundType::Start) => settings.sound_theme.to_start_path(),
        (_, SoundType::Stop) => settings.sound_theme.to_stop_path(),
    }
}

fn get_sound_base_dir(settings: &AppSettings) -> tauri::path::BaseDirectory {
    match settings.sound_theme {
        SoundTheme::Custom => tauri::path::BaseDirectory::AppData,
        _ => tauri::path::BaseDirectory::Resource,
    }
}

pub fn play_feedback_sound(app: &AppHandle, sound_type: SoundType) {
    let settings = settings::get_settings(app);
    if !settings.audio_feedback {
        return;
    }
    if let Some(path) = resolve_sound_path(app, &settings, sound_type) {
        play_sound_async(app, path);
    }
}

pub fn play_feedback_sound_blocking(app: &AppHandle, sound_type: SoundType) {
    let settings = settings::get_settings(app);
    if !settings.audio_feedback {
        return;
    }
    if let Some(path) = resolve_sound_path(app, &settings, sound_type) {
        play_sound_blocking(app, &path);
    }
}

pub fn play_test_sound(app: &AppHandle, sound_type: SoundType) {
    let settings = settings::get_settings(app);
    if let Some(path) = resolve_sound_path(app, &settings, sound_type) {
        play_sound_blocking(app, &path);
    }
}

fn play_sound_async(app: &AppHandle, path: PathBuf) {
    let app_handle = app.clone();
    thread::spawn(move || {
        if let Err(e) = play_sound_at_path(&app_handle, path.as_path()) {
            error!("Failed to play sound '{}': {}", path.display(), e);
        }
    });
}

fn play_sound_blocking(app: &AppHandle, path: &Path) {
    if let Err(e) = play_sound_at_path(app, path) {
        error!("Failed to play sound '{}': {}", path.display(), e);
    }
}

fn play_sound_at_path(app: &AppHandle, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let settings = settings::get_settings(app);
    let volume = settings.audio_feedback_volume;
    let selected_device = settings.selected_output_device.clone();
    play_audio_file(path, selected_device, volume)
}

/// Ein Auftrag an den Feedback-Thread: Datei, Geraet, Lautstaerke -- und
/// ein Kanal, ueber den er "fertig" meldet.
struct PlayRequest {
    path: PathBuf,
    device: Option<String>,
    volume: f32,
    done: std::sync::mpsc::SyncSender<Result<(), String>>,
}

/// Der Ausgabestream fuer die Feedback-Toene lebt in EINEM Thread und bleibt
/// zwischen zwei Toenen offen. Vorher wurde je Ton ein neuer WASAPI-Stream
/// geoeffnet, und dessen Anlaufzeit (100-300 ms Stille) schluckte den Anfang
/// des 0,5-s-Starttons -- mal ganz, mal halb (beobachtet 20.09.2026: "Ton
/// kommt nicht immer"). Der Thread baut den Stream nur neu, wenn sich das
/// Geraet aendert oder die Wiedergabe scheitert (Geraet abgezogen).
fn feedback_thread() -> &'static std::sync::Mutex<std::sync::mpsc::Sender<PlayRequest>> {
    static SENDER: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<PlayRequest>>> =
        std::sync::OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<PlayRequest>();
        thread::Builder::new()
            .name("audio-feedback".to_string())
            .spawn(move || {
                let mut current: Option<(Option<String>, rodio::OutputStream)> = None;
                while let Ok(req) = rx.recv() {
                    let needs_new = match &current {
                        Some((device, _)) => *device != req.device,
                        None => true,
                    };
                    if needs_new {
                        current = None;
                        match open_output(&req.device) {
                            Ok(stream) => current = Some((req.device.clone(), stream)),
                            Err(e) => {
                                let _ = req.done.send(Err(e));
                                continue;
                            }
                        }
                    }
                    let result = match &current {
                        Some((_, stream)) => play_on(stream, &req.path, req.volume),
                        None => Err("no output stream".to_string()),
                    };
                    if result.is_err() {
                        // Einmal frisch versuchen: Geraet weg, Stream tot.
                        current = None;
                        let retry = open_output(&req.device)
                            .and_then(|stream| {
                                let r = play_on(&stream, &req.path, req.volume);
                                current = Some((req.device.clone(), stream));
                                r
                            });
                        let _ = req.done.send(retry);
                    } else {
                        let _ = req.done.send(result);
                    }
                }
            })
            .expect("audio feedback thread");
        std::sync::Mutex::new(tx)
    })
}

fn open_output(selected_device: &Option<String>) -> Result<rodio::OutputStream, String> {
    let builder = match selected_device {
        Some(device_name) if device_name != "Default" => {
            let host = crate::audio_toolkit::get_cpal_host();
            let found = host
                .output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map(|n| n == *device_name).unwrap_or(false));
            match found {
                Some(device) => OutputStreamBuilder::from_device(device).map_err(|e| e.to_string())?,
                None => {
                    warn!("Device '{}' not found, using default device", device_name);
                    OutputStreamBuilder::from_default_device().map_err(|e| e.to_string())?
                }
            }
        }
        _ => {
            debug!("Using default device");
            OutputStreamBuilder::from_default_device().map_err(|e| e.to_string())?
        }
    };
    builder.open_stream().map_err(|e| e.to_string())
}

fn play_on(stream: &rodio::OutputStream, path: &Path, volume: f32) -> Result<(), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let sink = rodio::play(stream.mixer(), BufReader::new(file)).map_err(|e| e.to_string())?;
    sink.set_volume(volume);
    sink.sleep_until_end();
    Ok(())
}

fn play_audio_file(
    path: &std::path::Path,
    selected_device: Option<String>,
    volume: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
    feedback_thread()
        .lock()
        .map_err(|e| e.to_string())?
        .send(PlayRequest {
            path: path.to_path_buf(),
            device: selected_device,
            volume,
            done: done_tx,
        })
        .map_err(|e| e.to_string())?;
    // Laenger als jeder Feedback-Ton; ein haengender Treiber darf den
    // Aufrufer (Diktat-Start!) nicht festhalten.
    match done_rx.recv_timeout(std::time::Duration::from_secs(3)) {
        Ok(result) => result.map_err(|e| e.into()),
        Err(_) => Err("audio feedback timed out".into()),
    }
}
