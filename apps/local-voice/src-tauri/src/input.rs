use enigo::{Enigo, Key, Keyboard, Mouse, Settings};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

#[cfg(any(target_os = "windows", test))]
mod own_input;

/// One injection, with a separate ownership check around every enigo call.
pub(crate) struct InjectionFrame {
    #[cfg(target_os = "windows")]
    own: own_input::OwnInputFrame,
}

impl InjectionFrame {
    pub(crate) fn new(marker: Option<u32>) -> Self {
        #[cfg(not(target_os = "windows"))]
        let _ = marker;
        Self {
            #[cfg(target_os = "windows")]
            own: own_input::OwnInputFrame::new(marker),
        }
    }

    pub(crate) fn stroke<T>(&mut self, send: impl FnOnce() -> T) -> T {
        self.stroke_checked(|_| send())
    }

    pub(crate) fn stroke_checked<T>(&mut self, send: impl FnOnce(bool) -> T) -> T {
        #[cfg(target_os = "windows")]
        { self.own.stroke(&mut windows_input_monitor::Clock, send) }
        #[cfg(not(target_os = "windows"))]
        { send(true) }
    }

    pub(crate) fn unchanged(&mut self, tick: Option<u32>) -> bool {
        #[cfg(target_os = "windows")]
        { self.own.observe(tick) }
        #[cfg(not(target_os = "windows"))]
        { let _ = tick; true }
    }
}

struct TrackedEnigo<'a> {
    enigo: &'a mut Enigo,
    frame: &'a mut InjectionFrame,
    replacement: bool,
}

impl KeySink for TrackedEnigo<'_> {
    fn send_key(&mut self, key: Key, direction: enigo::Direction) -> Result<(), String> {
        self.frame.stroke_checked(|safe| {
            // Replacement stops before the next stroke. Always release a
            // modifier already pressed; dictation appends must still finish.
            if self.replacement && !safe && direction != enigo::Direction::Release {
                return Err("Replacement stopped after foreign input".into());
            }
            self.enigo.send_key(key, direction)
        })
    }

    fn hold(&mut self, duration: std::time::Duration) {
        std::thread::sleep(duration);
    }
}

pub(crate) fn send_tracked_paste_ctrl_v(
    enigo: &mut Enigo,
    frame: &mut InjectionFrame,
    replacement: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let keys = (Key::Meta, Key::Other(9));
    #[cfg(target_os = "windows")]
    let keys = (Key::Control, Key::Other(0x56));
    #[cfg(target_os = "linux")]
    let keys = (Key::Control, Key::Unicode('v'));
    send_modified_key(&mut TrackedEnigo { enigo, frame, replacement }, &[keys.0], keys.1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplacementContext {
    pub(crate) foreground: isize,
    pub(crate) focus: isize,
    pub(crate) physical_generation: u32,
}

#[cfg(test)]
pub(crate) fn replacement_context_matches(
    captured: ReplacementContext,
    current: ReplacementContext,
) -> bool {
    captured == current
}

/// How long an injection waits, at most, for the user to let go of the
/// hotkey before it types. Longer than any normal key release; short enough
/// that a genuinely held key (a game, a stuck switch) does not stall dictation.
pub(crate) const MODIFIER_RELEASE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(1500);

/// Whether any modifier key is physically held right now (Ctrl, Shift, Alt,
/// Win). Only Windows can ask cheaply; elsewhere this reports `false`.
#[cfg(target_os = "windows")]
pub(crate) fn modifiers_physically_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
        .into_iter()
        // High bit set = key is down at the time of the call.
        .any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } as u16 & 0x8000 != 0)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn modifiers_physically_held() -> bool {
    false
}

/// Block until no modifier key is physically held, or `max` has passed.
/// Returns how long it waited.
///
/// Why: the dictation hotkey is a modifier chord (Ctrl+Win by default). The
/// text that finishes a dictation is typed within ~100 ms of the stop press,
/// which is before the user has let go of the keys. Chromium-based targets
/// (VS Code, browsers, Electron apps) treat a character arriving with Ctrl
/// held as a shortcut, not as text, and drop it — the last words of a
/// dictation vanished exactly this way (log 15.09.2026: tail typed, key
/// release logged only afterwards). Waiting for the release costs at most
/// the time the user needs to lift a finger.
pub(crate) fn wait_for_modifiers_released(max: std::time::Duration) -> std::time::Duration {
    wait_for_modifiers_released_with(
        &mut modifiers_physically_held,
        max,
        std::time::Duration::from_millis(10),
    )
}

/// Wie [`wait_for_modifiers_released`], mit austauschbarer Tastaturabfrage und
/// Abfragetakt: `held` sagt, ob gerade eine Umschalttaste gehalten wird. Fragt
/// sofort, schlaeft nur zwischen zwei Abfragen und gibt nach `max` auf (eine
/// dauerhaft gehaltene Taste, ein Spiel, ein klemmender Schalter, darf das Diktat
/// nicht aufhalten). Gibt zurueck, wie lange gewartet wurde.
pub(crate) fn wait_for_modifiers_released_with(
    held: &mut dyn FnMut() -> bool,
    max: std::time::Duration,
    poll: std::time::Duration,
) -> std::time::Duration {
    let started = std::time::Instant::now();
    while held() && started.elapsed() < max {
        std::thread::sleep(poll);
    }
    started.elapsed()
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

pub(crate) fn send_select_left(
    enigo: &mut Enigo,
    count: usize,
    frame: &mut InjectionFrame,
) -> Result<(), String> {
    if count == 0 {
        return Err("Cannot select an empty replacement range".to_string());
    }

    let mut sink = TrackedEnigo { enigo, frame, replacement: true };
    sink
        .send_key(Key::Shift, enigo::Direction::Press)
        .map_err(|error| format!("Failed to press Shift: {error}"))?;

    let mut selection_error = None;
    for _ in 0..count {
        if let Err(error) = sink.send_key(Key::LeftArrow, enigo::Direction::Click) {
            selection_error = Some(format!("Failed to extend replacement selection: {error}"));
            break;
        }
    }
    let release_result = sink
        .send_key(Key::Shift, enigo::Direction::Release)
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
    use super::own_input::InputClock;
    use std::mem::size_of;
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GUITHREADINFO,
    };

    pub(super) struct Clock;

    impl InputClock for Clock {
        fn last_input(&mut self) -> Option<u32> { last_input_tick() }
        fn now(&mut self) -> u32 { unsafe { GetTickCount() } }
    }

    fn last_input_tick() -> Option<u32> {
        let mut last = LASTINPUTINFO {
            cbSize: size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if !unsafe { GetLastInputInfo(&mut last) }.as_bool() {
            return None;
        }
        Some(last.dwTime)
    }

    pub(super) fn capture() -> Option<ReplacementContext> {
        let tick = last_input_tick()?;

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
            // dwTime ist u32 und laeuft ueber. Nur Gleichheit ist ein sicherer
            // Unveraendert-Nachweis; auch rueckwaertige Ticks gelten als Eingabe.
            physical_generation: tick,
        })
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
                self.events
                    .push(format!("{:?} {:?} FAILED", direction, key));
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

    #[test]
    fn last_input_tick_wrap_and_backwards_ticks_are_changes() {
        let captured = ReplacementContext {
            foreground: 11,
            focus: 22,
            physical_generation: u32::MAX,
        };
        assert!(replacement_context_matches(captured, captured));
        for tick in [0, 1, u32::MAX - 1] {
            assert!(!replacement_context_matches(captured, ReplacementContext {
                physical_generation: tick,
                ..captured
            }));
        }
    }
}

#[cfg(test)]
mod modifier_release_tests {
    use super::*;

    const STEP: std::time::Duration = std::time::Duration::from_millis(2);

    /// Simulierter Tastaturzustand: gehalten fuer die ersten `polls_held` Abfragen.
    fn keyboard(polls_held: u32) -> (impl FnMut() -> bool, std::rc::Rc<std::cell::Cell<u32>>) {
        let polls = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let seen = std::rc::Rc::clone(&polls);
        (
            move || {
                seen.set(seen.get() + 1);
                seen.get() <= polls_held
            },
            polls,
        )
    }

    #[test]
    fn the_wait_ends_as_soon_as_the_chord_is_let_go() {
        let (mut held, polls) = keyboard(3); // Strg+Win noch 3 Abfragen unten
        let waited =
            wait_for_modifiers_released_with(&mut held, std::time::Duration::from_secs(5), STEP);
        assert_eq!(polls.get(), 4, "3x gehalten, dann losgelassen");
        assert!(waited >= STEP * 3, "es wurde wirklich gewartet: {waited:?}");
        assert!(
            waited < std::time::Duration::from_secs(1),
            "und nicht bis zum Limit: {waited:?}"
        );
    }

    #[test]
    fn a_key_held_forever_ends_at_the_time_limit() {
        let mut held = || true;
        let limit = std::time::Duration::from_millis(40);
        let waited = wait_for_modifiers_released_with(&mut held, limit, STEP);
        assert!(waited >= limit, "{waited:?}");
        assert!(waited < std::time::Duration::from_secs(2), "{waited:?}");
    }

    #[test]
    fn nothing_held_costs_one_query_and_no_sleep() {
        let (mut held, polls) = keyboard(0);
        let waited = wait_for_modifiers_released_with(
            &mut held,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(5),
        );
        assert_eq!(polls.get(), 1);
        assert!(
            waited < std::time::Duration::from_millis(500),
            "kein Takt-Schlaf: {waited:?}"
        );
    }

    /// Ohne gedrueckte Taste darf das Warten nichts kosten — sonst wuerde
    /// jede Injektion um die Wartezeit spaeter landen.
    #[test]
    fn returns_immediately_when_no_modifier_is_held() {
        // Auf einem Rechner, an dem gerade jemand Strg haelt, ist der Test
        // nicht aussagekraeftig; dann ueberspringen statt falsch rot werden.
        if modifiers_physically_held() {
            return;
        }
        let waited = wait_for_modifiers_released(MODIFIER_RELEASE_TIMEOUT);
        assert!(
            waited < std::time::Duration::from_millis(50),
            "waited {waited:?}"
        );
    }
}
