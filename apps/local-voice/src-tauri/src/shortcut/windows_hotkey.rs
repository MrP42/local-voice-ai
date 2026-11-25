//! lva#874: RegisterHotKey ohne den aufnahmeunabhaengigen Release-Poller
//! der Plugin-Abhaengigkeit. Registrierungen gehoeren ihrem Worker-Thread.

use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT,
    MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_REMOVE, WM_HOTKEY};

use super::handler::handle_shortcut_event;
use crate::managers::audio::AudioRecordingManager;
use crate::settings::{get_settings, KeyboardImplementation, ShortcutBinding};
use crate::TranscriptionCoordinator;

enum Command {
    Register(ShortcutBinding, mpsc::Sender<Result<(), String>>),
    Unregister(String, mpsc::Sender<Result<(), String>>),
    Watch(String, String),
    Shutdown,
}

struct Hotkeys(mpsc::Sender<Command>);

impl Drop for Hotkeys {
    fn drop(&mut self) {
        let _ = self.0.send(Command::Shutdown);
    }
}

struct Registered {
    id: i32,
    binding_id: String,
    shortcut: Shortcut,
    raw: String,
    registered: bool,
}

fn dispatch_hotkey(registered: &Registered, handle: impl FnOnce(&str, &str)) {
    // Actions also inspect the original spelling (notably the AltGr guard).
    handle(&registered.binding_id, &registered.raw);
}

impl Registered {
    fn unregister(&mut self) -> Result<(), String> {
        unsafe { UnregisterHotKey(None, self.id) }.map_err(|_| "UnregisterHotKey failed")?;
        self.registered = false;
        Ok(())
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        if self.registered && self.unregister().is_err() {
            log::warn!("RegisterHotKey registration could not be released");
        }
    }
}

// The dependency's Windows mapping is private. Keep the same supported key
// family (except the dependency's ambiguous NumpadEqual -> E mapping), and
// reject everything else before changing a user's saved backend.
fn virtual_key(code: Code) -> Option<u32> {
    let name = code.to_string();
    if let Some(letter) = name.strip_prefix("Key") {
        if letter.len() == 1 && letter.as_bytes()[0].is_ascii_uppercase() {
            return Some(u32::from(letter.as_bytes()[0]));
        }
    }
    if let Some(digit) = name.strip_prefix("Digit") {
        if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() {
            return Some(u32::from(digit.as_bytes()[0]));
        }
    }
    if let Some(number) = name.strip_prefix('F').and_then(|s| s.parse::<u32>().ok()) {
        if (1..=24).contains(&number) {
            return Some(0x70 + number - 1);
        }
    }
    if let Some(digit) = name.strip_prefix("Numpad") {
        if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() {
            return Some(0x60 + u32::from(digit.as_bytes()[0] - b'0'));
        }
    }
    Some(match code {
        Code::Space => 0x20,
        Code::Escape => 0x1b,
        Code::Enter | Code::NumpadEnter => 0x0d,
        Code::Tab => 0x09,
        Code::Backspace => 0x08,
        Code::CapsLock => 0x14,
        Code::PageUp => 0x21,
        Code::PageDown => 0x22,
        Code::End => 0x23,
        Code::Home => 0x24,
        Code::ArrowLeft => 0x25,
        Code::ArrowUp => 0x26,
        Code::ArrowRight => 0x27,
        Code::ArrowDown => 0x28,
        Code::PrintScreen => 0x2c,
        Code::Insert => 0x2d,
        Code::Delete => 0x2e,
        Code::Pause | Code::MediaPause => 0x13,
        Code::NumLock => 0x90,
        Code::ScrollLock => 0x91,
        Code::NumpadMultiply => 0x6a,
        Code::NumpadAdd => 0x6b,
        Code::NumpadSubtract => 0x6d,
        Code::NumpadDecimal => 0x6e,
        Code::NumpadDivide => 0x6f,
        Code::Equal => 0xbb,
        Code::Comma => 0xbc,
        Code::Minus => 0xbd,
        Code::Period => 0xbe,
        Code::Semicolon => 0xba,
        Code::Slash => 0xbf,
        Code::Backquote => 0xc0,
        Code::BracketLeft => 0xdb,
        Code::Backslash => 0xdc,
        Code::BracketRight => 0xdd,
        Code::Quote => 0xde,
        Code::AudioVolumeMute => 0xad,
        Code::AudioVolumeDown => 0xae,
        Code::AudioVolumeUp => 0xaf,
        Code::MediaTrackNext => 0xb0,
        Code::MediaTrackPrevious => 0xb1,
        Code::MediaStop => 0xb2,
        Code::MediaPlayPause => 0xb3,
        Code::MediaPlay => 0xfa,
        _ => return None,
    })
}

pub(super) fn parse_shortcut(raw: &str) -> Result<Shortcut, String> {
    let shortcut = raw.parse::<Shortcut>().map_err(|_| "Unsupported RegisterHotKey shortcut")?;
    virtual_key(shortcut.key).ok_or("Unsupported RegisterHotKey key")?;
    Ok(shortcut)
}

fn modifiers(shortcut: Shortcut) -> HOT_KEY_MODIFIERS {
    let mut flags = MOD_NOREPEAT;
    for (modifier, flag) in [
        (Modifiers::CONTROL, MOD_CONTROL),
        (Modifiers::ALT, MOD_ALT),
        (Modifiers::SHIFT, MOD_SHIFT),
        (Modifiers::SUPER, MOD_WIN),
    ] {
        if shortcut.mods.contains(modifier) {
            flags |= flag;
        }
    }
    flags
}

/// Only keys of this chord are queried, and only after the recording started.
/// Releasing either Win key ends the chord only when neither is held anymore.
fn chord_held(shortcut: Shortcut, mut down: impl FnMut(i32) -> bool) -> bool {
    let Some(key) = virtual_key(shortcut.key) else {
        return false;
    };
    if !down(key as i32) {
        return false;
    }
    for (modifier, vk) in [
        (Modifiers::CONTROL, 0x11),
        (Modifiers::SHIFT, 0x10),
        (Modifiers::ALT, 0x12),
    ] {
        if shortcut.mods.contains(modifier) && !down(vk) {
            return false;
        }
    }
    !shortcut.mods.contains(Modifiers::SUPER) || down(0x5b) || down(0x5c)
}

fn poll_release(
    shortcut: Shortcut,
    is_recording: impl FnOnce() -> bool,
    down: impl FnMut(i32) -> bool,
) -> bool {
    if !is_recording() {
        return false;
    }
    !chord_held(shortcut, down)
}

fn worker(app: AppHandle, rx: mpsc::Receiver<Command>) {
    let mut registered: HashMap<i32, Registered> = HashMap::new();
    let mut next_id = 1;
    let mut watch: Option<(String, String, Shortcut)> = None;
    let mut message = MSG::default();
    // Ensure the thread's message queue exists before the first registration.
    let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) };
    loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Command::Register(binding, reply)) => {
                let result = (|| {
                    let shortcut = parse_shortcut(&binding.current_binding)?;
                    if registered.values().any(|r| r.shortcut == shortcut || r.binding_id == binding.id) {
                        return Err("Shortcut is already registered".into());
                    }
                    if next_id > 0xbfff {
                        return Err("Hotkey ID range exhausted".into());
                    }
                    let id = next_id;
                    next_id += 1;
                    let key = virtual_key(shortcut.key).ok_or("Unsupported RegisterHotKey key")?;
                    unsafe { RegisterHotKey(None, id, modifiers(shortcut), key) }
                        .map_err(|_| "RegisterHotKey failed (reserved or already in use)")?;
                    registered.insert(id, Registered {
                        id,
                        binding_id: binding.id,
                        shortcut,
                        raw: binding.current_binding,
                        registered: true,
                    });
                    Ok(())
                })();
                let _ = reply.send(result);
            }
            Ok(Command::Unregister(binding_id, reply)) => {
                let id = registered.iter().find(|(_, r)| r.binding_id == binding_id).map(|(id, _)| *id);
                let result = if let Some(id) = id {
                    let result = registered.get_mut(&id).expect("existing registration").unregister();
                    if result.is_ok() {
                        registered.remove(&id);
                    }
                    result
                } else {
                    Err("Shortcut is not registered".into())
                };
                let _ = reply.send(result);
            }
            Ok(Command::Watch(binding, raw)) => {
                watch = parse_shortcut(&raw).ok().map(|sc| (binding, raw, sc));
            }
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        while unsafe { PeekMessageW(&mut message, None, WM_HOTKEY, WM_HOTKEY, PM_REMOVE) }.as_bool() {
            if let Some(r) = registered.get(&(message.wParam.0 as i32)) {
                dispatch_hotkey(r, |binding_id, raw| {
                    handle_shortcut_event(&app, binding_id, raw, true);
                });
            }
        }
        if let Some((binding, raw, shortcut)) = &watch {
            let recording = app.try_state::<Arc<AudioRecordingManager>>()
                .is_some_and(|audio| audio.is_recording());
            // A watch belongs to the recording that started it. A concurrent
            // backend change must not discard that recording's only release.
            if !recording {
                watch = None;
                continue;
            }
            if poll_release(*shortcut, || recording, |vk| unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0) {
                // The mode belongs to this recording, even when the user has
                // changed the setting since its start.
                if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
                    coordinator.send_input(binding, raw, false, true);
                }
                watch = None;
            }
        }
    }
    // Registered::drop unregisters on the same thread, including error exits.
}

pub(super) fn init(app: &AppHandle) {
    if app.try_state::<Hotkeys>().is_none() {
        let (tx, rx) = mpsc::channel();
        if app.manage(Hotkeys(tx)) {
            let app = app.clone();
            std::thread::spawn(move || worker(app, rx));
        }
    }
}

pub(super) fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    init(app);
    let (tx, rx) = mpsc::channel();
    app.state::<Hotkeys>().0.send(Command::Register(binding, tx)).map_err(|_| "Hotkey worker stopped")?;
    rx.recv().map_err(|_| "Hotkey worker stopped")?
}

pub(super) fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    let state = app.try_state::<Hotkeys>().ok_or("Hotkey worker not initialized")?;
    let (tx, rx) = mpsc::channel();
    state.0.send(Command::Unregister(binding.id, tx)).map_err(|_| "Hotkey worker stopped")?;
    rx.recv().map_err(|_| "Hotkey worker stopped")?
}

pub(super) fn recording_started(app: &AppHandle, binding_id: &str, raw: &str) {
    let settings = get_settings(app);
    if !settings.push_to_talk || settings.keyboard_implementation != KeyboardImplementation::Tauri {
        return;
    }
    if let Some(state) = app.try_state::<Hotkeys>() {
        if state.0.send(Command::Watch(binding_id.into(), raw.into())).is_err() {
            log::warn!("Push-to-talk release monitor unavailable");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_preserves_ctrl_spelling_used_by_the_existing_altgr_guard() {
        let registered = Registered {
            id: 1,
            binding_id: "voice_call".into(),
            shortcut: parse_shortcut("ctrl+alt+g").unwrap(),
            raw: "ctrl+alt+g".into(),
            registered: false, // Synthetic registration: no desktop API in this test.
        };
        assert_ne!(registered.shortcut.into_string(), registered.raw);
        let mut seen = None;
        dispatch_hotkey(&registered, |id, raw| seen = Some((id.to_owned(), raw.to_owned())));
        assert_eq!(seen, Some(("voice_call".into(), "ctrl+alt+g".into())));
    }

    #[test]
    fn only_the_registered_chord_is_queried_while_recording() {
        let shortcut = parse_shortcut("ctrl+space").unwrap();
        let mut queried = Vec::new();
        assert!(!poll_release(shortcut, || false, |vk| { queried.push(vk); true }));
        assert!(queried.is_empty());
        assert!(!poll_release(shortcut, || true, |vk| { queried.push(vk); true }));
        assert_eq!(queried, [0x20, 0x11]);
        for released in [0x20, 0x11] {
            assert!(poll_release(shortcut, || true, |vk| vk != released));
        }
    }

    #[test]
    fn parser_rejects_hook_only_shortcuts_without_losing_supported_keys() {
        for raw in ["ctrl", "ctrl+shift", "mouse4", "ctrl+mouseleft", "fn+space"] {
            assert!(parse_shortcut(raw).is_err(), "{raw}");
        }
        for raw in ["ctrl+space", "ctrl+shift+space", "escape", "alt+f24", "super+g"] {
            assert!(parse_shortcut(raw).is_ok(), "{raw}");
        }
    }
}
