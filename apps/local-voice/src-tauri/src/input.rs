use enigo::{Enigo, Key, Keyboard, Mouse, Settings};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplacementContext {
    pub(crate) foreground: isize,
    pub(crate) focus: isize,
    pub(crate) physical_generation: u64,
}

#[cfg(test)]
pub(crate) fn replacement_context_matches(
    captured: ReplacementContext,
    current: ReplacementContext,
) -> bool {
    captured == current
}

/// Wrapper for Enigo to store in Tauri's managed state.
/// Enigo is wrapped in a Mutex since it requires mutable access.
pub struct EnigoState(pub Mutex<Enigo>);

impl EnigoState {
    pub fn new() -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to initialize Enigo: {}", e))?;
        Ok(Self(Mutex::new(enigo)))
    }
}

/// Get the current mouse cursor position using the managed Enigo instance.
/// Returns None if the state is not available or if getting the location fails.
pub fn get_cursor_position(app_handle: &AppHandle) -> Option<(i32, i32)> {
    let enigo_state = app_handle.try_state::<EnigoState>()?;
    let enigo = enigo_state.0.lock().ok()?;
    enigo.location().ok()
}

/// How long the paste modifier stays held after the V keystroke.
///
/// Windows delivers injected events in order, but applications that read the
/// modifier state asynchronously (Chromium among them) still need it down
/// when they get around to the V. That is the only reason to hold it at all,
/// so the window is kept as short as that requires: while the modifier is
/// down it is down for the whole desktop, and the user's own scrolling then
/// reads as Ctrl+scroll (= zoom) everywhere. Stream injection pastes a
/// fragment every few hundred milliseconds while the user keeps working, so
/// a generous hold made the machine behave as if Ctrl were stuck for most of
/// a dictation.
const PASTE_MODIFIER_HOLD: std::time::Duration = std::time::Duration::from_millis(15);

/// The rest of the settle time the paste used to spend with the modifier
/// held down. The target still gets it - only now with the keyboard free.
const PASTE_SETTLE_AFTER_RELEASE: std::time::Duration = std::time::Duration::from_millis(85);

/// Keyboard sink behind the paste sequences, so the ordering below can be
/// tested without injecting real keystrokes into the developer's desktop.
pub(crate) trait KeySink {
    fn send_key(&mut self, key: Key, direction: enigo::Direction) -> Result<(), String>;
    fn hold(&mut self, duration: std::time::Duration);
}

impl KeySink for Enigo {
    fn send_key(&mut self, key: Key, direction: enigo::Direction) -> Result<(), String> {
        Keyboard::key(self, key, direction).map_err(|e| e.to_string())
    }

    fn hold(&mut self, duration: std::time::Duration) {
        std::thread::sleep(duration);
    }
}

/// Press modifier, tap `key`, release the modifier - and release it even when
/// the tap failed. An early return between press and release would leave the
/// modifier down for every application on the machine with nothing to undo
/// it (the stuck-Ctrl report of 08.09.2026).
pub(crate) fn send_modified_key<K: KeySink>(
    sink: &mut K,
    modifiers: &[Key],
    key: Key,
) -> Result<(), String> {
    let mut pressed: Vec<Key> = Vec::with_capacity(modifiers.len());
    let mut result = Ok(());

    for modifier in modifiers {
        match sink.send_key(*modifier, enigo::Direction::Press) {
            Ok(()) => pressed.push(*modifier),
            Err(e) => {
                result = Err(format!("Failed to press modifier key: {}", e));
                break;
            }
        }
    }

    if result.is_ok() {
        if let Err(e) = sink.send_key(key, enigo::Direction::Click) {
            result = Err(format!("Failed to click key: {}", e));
        }
        sink.hold(PASTE_MODIFIER_HOLD);
    }

    // Release in reverse order, and report a release failure only when the
    // sequence was otherwise fine - the first error is the interesting one.
    for modifier in pressed.iter().rev() {
        if let Err(e) = sink.send_key(*modifier, enigo::Direction::Release) {
            let message = format!("Failed to release modifier key: {}", e);
            if result.is_ok() {
                result = Err(message);
            }
        }
    }

    if result.is_ok() {
        sink.hold(PASTE_SETTLE_AFTER_RELEASE);
    }
    result
}

/// Sends a Ctrl+V or Cmd+V paste command using platform-specific virtual key codes.
/// This ensures the paste works regardless of keyboard layout (e.g., Russian, AZERTY, DVORAK).
/// Note: On Wayland, this may not work - callers should check for Wayland and use alternative methods.
pub fn send_paste_ctrl_v(enigo: &mut Enigo) -> Result<(), String> {
    // Platform-specific key definitions
    #[cfg(target_os = "macos")]
    let (modifier_key, v_key_code) = (Key::Meta, Key::Other(9));
    #[cfg(target_os = "windows")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Other(0x56)); // VK_V
    #[cfg(target_os = "linux")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Unicode('v'));

    send_modified_key(enigo, &[modifier_key], v_key_code)
}

/// Sends a Ctrl+Shift+V paste command.
/// This is commonly used in terminal applications on Linux to paste without formatting.
/// Note: On Wayland, this may not work - callers should check for Wayland and use alternative methods.
pub fn send_paste_ctrl_shift_v(enigo: &mut Enigo) -> Result<(), String> {
    // Platform-specific key definitions
    #[cfg(target_os = "macos")]
    let (modifier_key, v_key_code) = (Key::Meta, Key::Other(9)); // Cmd+Shift+V on macOS
    #[cfg(target_os = "windows")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Other(0x56)); // VK_V
    #[cfg(target_os = "linux")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Unicode('v'));

    send_modified_key(enigo, &[modifier_key, Key::Shift], v_key_code)
}

/// Sends a Shift+Insert paste command (Windows and Linux only).
/// This is more universal for terminal applications and legacy software.
/// Note: On Wayland, this may not work - callers should check for Wayland and use alternative methods.
pub fn send_paste_shift_insert(enigo: &mut Enigo) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let insert_key_code = Key::Other(0x2D); // VK_INSERT
    #[cfg(not(target_os = "windows"))]
    let insert_key_code = Key::Other(0x76); // XK_Insert (keycode 118 / 0x76, also used as fallback)

    send_modified_key(enigo, &[Key::Shift], insert_key_code)
}

/// Pastes text directly using the enigo text method.
/// This tries to use system input methods if possible, otherwise simulates keystrokes one by one.
pub fn paste_text_direct(enigo: &mut Enigo, text: &str) -> Result<(), String> {
    enigo
        .text(text)
        .map_err(|e| format!("Failed to send text directly: {}", e))?;

    Ok(())
}

pub(crate) fn send_select_left(enigo: &mut Enigo, count: usize) -> Result<(), String> {
    if count == 0 {
        return Err("Cannot select an empty replacement range".to_string());
    }

    enigo
        .key(Key::Shift, enigo::Direction::Press)
        .map_err(|error| format!("Failed to press Shift: {error}"))?;

    let mut selection_error = None;
    for _ in 0..count {
        if let Err(error) = enigo.key(Key::LeftArrow, enigo::Direction::Click) {
            selection_error = Some(format!("Failed to extend replacement selection: {error}"));
            break;
        }
    }
    let release_result = enigo
        .key(Key::Shift, enigo::Direction::Release)
        .map_err(|error| format!("Failed to release Shift: {error}"));

    if let Some(error) = selection_error {
        return Err(error);
    }
    release_result
}

#[cfg(target_os = "windows")]
pub(crate) fn capture_replacement_context() -> Option<ReplacementContext> {
    windows_input_monitor::capture()
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn capture_replacement_context() -> Option<ReplacementContext> {
    None
}

#[cfg(target_os = "windows")]
mod windows_input_monitor {
    use super::ReplacementContext;
    use std::mem::size_of;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, OnceLock};
    use std::time::Duration;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetGUIThreadInfo, GetMessageW,
        SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, GUITHREADINFO, HC_ACTION,
        KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL,
        WH_MOUSE_LL,
    };

    static PHYSICAL_INPUT_GENERATION: AtomicU64 = AtomicU64::new(0);
    static MONITOR_AVAILABLE: OnceLock<bool> = OnceLock::new();

    pub(super) fn capture() -> Option<ReplacementContext> {
        if !monitor_available() {
            return None;
        }

        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0.is_null() {
            return None;
        }

        let mut gui = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetGUIThreadInfo(0, &mut gui) }.is_err() || gui.hwndFocus.0.is_null() {
            return None;
        }

        Some(ReplacementContext {
            foreground: foreground.0 as isize,
            focus: gui.hwndFocus.0 as isize,
            physical_generation: PHYSICAL_INPUT_GENERATION.load(Ordering::Acquire),
        })
    }

    fn monitor_available() -> bool {
        *MONITOR_AVAILABLE.get_or_init(|| {
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            std::thread::spawn(move || run_monitor(ready_tx));
            ready_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap_or(false)
        })
    }

    fn run_monitor(ready: mpsc::SyncSender<bool>) {
        let module = match unsafe { GetModuleHandleW(PCWSTR::null()) } {
            Ok(module) => HINSTANCE(module.0),
            Err(_) => {
                let _ = ready.send(false);
                return;
            }
        };
        let keyboard = match unsafe {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(module), 0)
        } {
            Ok(hook) => hook,
            Err(_) => {
                let _ = ready.send(false);
                return;
            }
        };
        let mouse =
            match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(module), 0) } {
                Ok(hook) => hook,
                Err(_) => {
                    let _ = unsafe { UnhookWindowsHookEx(keyboard) };
                    let _ = ready.send(false);
                    return;
                }
            };

        let _ = ready.send(true);
        let mut message = MSG::default();
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
            let _ = unsafe { TranslateMessage(&message) };
            unsafe { DispatchMessageW(&message) };
        }
        let _ = unsafe { UnhookWindowsHookEx(mouse) };
        let _ = unsafe { UnhookWindowsHookEx(keyboard) };
    }

    unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            if !event.flags.contains(LLKHF_INJECTED) {
                PHYSICAL_INPUT_GENERATION.fetch_add(1, Ordering::AcqRel);
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            if event.flags & LLMHF_INJECTED == 0 {
                PHYSICAL_INPUT_GENERATION.fetch_add(1, Ordering::AcqRel);
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        replacement_context_matches, send_modified_key, KeySink, ReplacementContext,
        PASTE_MODIFIER_HOLD, PASTE_SETTLE_AFTER_RELEASE,
    };
    use enigo::{Direction, Key};
    use std::time::Duration;

    /// Records the keystroke sequence instead of injecting it, and can be
    /// told to fail on one specific step.
    struct FakeSink {
        events: Vec<String>,
        fail_on: Option<(Key, Direction)>,
    }

    impl FakeSink {
        fn new() -> Self {
            Self {
                events: Vec::new(),
                fail_on: None,
            }
        }

        fn failing_on(key: Key, direction: Direction) -> Self {
            Self {
                events: Vec::new(),
                fail_on: Some((key, direction)),
            }
        }
    }

    impl KeySink for FakeSink {
        fn send_key(&mut self, key: Key, direction: Direction) -> Result<(), String> {
            if self.fail_on == Some((key, direction)) {
                self.events.push(format!("{:?} {:?} FAILED", direction, key));
                return Err("injection refused".to_string());
            }
            self.events.push(format!("{:?} {:?}", direction, key));
            Ok(())
        }

        fn hold(&mut self, duration: Duration) {
            self.events.push(format!("hold {}ms", duration.as_millis()));
        }
    }

    #[test]
    fn paste_releases_the_modifier_before_the_settle_wait() {
        // The whole point: the desktop must not sit with Ctrl held down while
        // we wait for the target to catch up, or the user's scrolling zooms.
        let mut sink = FakeSink::new();
        send_modified_key(&mut sink, &[Key::Control], Key::Other(0x56)).unwrap();

        assert_eq!(
            sink.events,
            vec![
                "Press Control".to_string(),
                "Click Other(86)".to_string(),
                format!("hold {}ms", PASTE_MODIFIER_HOLD.as_millis()),
                "Release Control".to_string(),
                format!("hold {}ms", PASTE_SETTLE_AFTER_RELEASE.as_millis()),
            ]
        );
        assert!(
            PASTE_MODIFIER_HOLD <= Duration::from_millis(20),
            "the modifier is held desktop-wide; keep the window short"
        );
    }

    #[test]
    fn paste_releases_the_modifier_even_when_the_keystroke_fails() {
        let mut sink = FakeSink::failing_on(Key::Other(0x56), Direction::Click);
        let result = send_modified_key(&mut sink, &[Key::Control], Key::Other(0x56));

        assert!(result.is_err());
        assert!(
            sink.events.contains(&"Release Control".to_string()),
            "a failed paste must not leave Ctrl down for the whole machine: {:?}",
            sink.events
        );
    }

    #[test]
    fn paste_releases_modifiers_in_reverse_order() {
        let mut sink = FakeSink::new();
        send_modified_key(&mut sink, &[Key::Control, Key::Shift], Key::Other(0x56)).unwrap();

        let releases: Vec<&String> = sink
            .events
            .iter()
            .filter(|e| e.starts_with("Release"))
            .collect();
        assert_eq!(releases, vec!["Release Shift", "Release Control"]);
    }


    #[test]
    fn replacement_context_requires_same_window_focus_and_input_generation() {
        let captured = ReplacementContext {
            foreground: 11,
            focus: 22,
            physical_generation: 7,
        };

        assert!(replacement_context_matches(captured, captured));
        assert!(!replacement_context_matches(
            captured,
            ReplacementContext {
                foreground: 12,
                ..captured
            }
        ));
        assert!(!replacement_context_matches(
            captured,
            ReplacementContext {
                focus: 23,
                ..captured
            }
        ));
        assert!(!replacement_context_matches(
            captured,
            ReplacementContext {
                physical_generation: 8,
                ..captured
            }
        ));
    }
}
