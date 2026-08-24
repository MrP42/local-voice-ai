use crate::input::{self, EnigoState};
use crate::paste_guard::{self, PasteFallback, PasteTarget};
#[cfg(target_os = "linux")]
use crate::settings::TypingTool;
use crate::settings::{get_settings, AutoSubmitKey, ClipboardHandling, PasteMethod};
use enigo::{Direction, Enigo, Key, Keyboard};
use log::{info, warn};
use std::process::Command;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[cfg(target_os = "linux")]
use crate::utils::{is_kde_wayland, is_wayland};

/// Pastes text using the clipboard: saves current content, writes text, sends paste keystroke, restores clipboard.
fn paste_via_clipboard(
    enigo: &mut Enigo,
    text: &str,
    app_handle: &AppHandle,
    paste_method: &PasteMethod,
    paste_delay_ms: u64,
    paste_delay_after_ms: u64,
) -> Result<(), String> {
    let clipboard = app_handle.clipboard();
    let saved_text = clipboard.read_text().ok().filter(|t| !t.is_empty());
    // Only probe for an image when there is no text to restore. Text is by far the
    // common case, and reading an image decodes the full bitmap, so this keeps the
    // text path exactly as cheap as it was before.
    let saved_image = if saved_text.is_none() {
        clipboard.read_image().ok().map(|image| image.to_owned())
    } else {
        None
    };

    // Write text to clipboard first
    // On Wayland, prefer wl-copy for better compatibility (especially with umlauts)
    #[cfg(target_os = "linux")]
    let write_result = if is_wayland() && is_wl_copy_available() {
        info!("Using wl-copy for clipboard write on Wayland");
        write_clipboard_via_wl_copy(text)
    } else {
        clipboard
            .write_text(text)
            .map_err(|e| format!("Failed to write to clipboard: {}", e))
    };

    #[cfg(not(target_os = "linux"))]
    let write_result = clipboard
        .write_text(text)
        .map_err(|e| format!("Failed to write to clipboard: {}", e));

    write_result?;

    std::thread::sleep(Duration::from_millis(paste_delay_ms));

    // Send paste key combo
    #[cfg(target_os = "linux")]
    let key_combo_sent = try_send_key_combo_linux(paste_method)?;

    #[cfg(not(target_os = "linux"))]
    let key_combo_sent = false;

    // Fall back to enigo if no native tool handled it
    if !key_combo_sent {
        match paste_method {
            PasteMethod::CtrlV => input::send_paste_ctrl_v(enigo)?,
            PasteMethod::CtrlShiftV => input::send_paste_ctrl_shift_v(enigo)?,
            PasteMethod::ShiftInsert => input::send_paste_shift_insert(enigo)?,
            _ => return Err("Invalid paste method for clipboard paste".into()),
        }
    }

    std::thread::sleep(Duration::from_millis(paste_delay_after_ms));

    // Restore original clipboard content.
    // Text takes priority so this path stays identical to the previous behavior;
    // an image is only restored when the clipboard held no text at all, which is
    // the case that used to silently wipe screenshots.
    if let Some(clipboard_content) = saved_text {
        // On Wayland, prefer wl-copy for better compatibility
        #[cfg(target_os = "linux")]
        if is_wayland() && is_wl_copy_available() {
            let _ = write_clipboard_via_wl_copy(&clipboard_content);
        } else {
            let _ = clipboard.write_text(&clipboard_content);
        }

        #[cfg(not(target_os = "linux"))]
        let _ = clipboard.write_text(&clipboard_content);
    } else if let Some(image) = saved_image {
        info!("Restoring image to clipboard");
        let _ = clipboard.write_image(&image);
    } else {
        // Nothing was there to begin with — don't leave the transcription behind.
        let _ = clipboard.clear();
    }

    Ok(())
}

/// Attempts to send a key combination using Linux-native tools.
/// Returns `Ok(true)` if a native tool handled it, `Ok(false)` to fall back to enigo.
#[cfg(target_os = "linux")]
fn try_send_key_combo_linux(paste_method: &PasteMethod) -> Result<bool, String> {
    if is_wayland() {
        // Wayland: prefer wtype (but not on KDE), then dotool, then ydotool
        // Note: wtype doesn't work on KDE (no zwp_virtual_keyboard_manager_v1 support)
        if !is_kde_wayland() && is_wtype_available() {
            info!("Using wtype for key combo");
            send_key_combo_via_wtype(paste_method)?;
            return Ok(true);
        }
        if is_dotool_available() {
            info!("Using dotool for key combo");
            send_key_combo_via_dotool(paste_method)?;
            return Ok(true);
        }
        if is_ydotool_available() {
            info!("Using ydotool for key combo");
            send_key_combo_via_ydotool(paste_method)?;
            return Ok(true);
        }
    } else {
        // X11: prefer xdotool, then ydotool
        if is_xdotool_available() {
            info!("Using xdotool for key combo");
            send_key_combo_via_xdotool(paste_method)?;
            return Ok(true);
        }
        if is_ydotool_available() {
            info!("Using ydotool for key combo");
            send_key_combo_via_ydotool(paste_method)?;
            return Ok(true);
        }
    }

    Ok(false)
}

/// Attempts to type text directly using Linux-native tools.
/// Returns `Ok(true)` if a native tool handled it, `Ok(false)` to fall back to enigo.
#[cfg(target_os = "linux")]
fn try_direct_typing_linux(text: &str, preferred_tool: TypingTool) -> Result<bool, String> {
    // If user specified a tool, try only that one
    if preferred_tool != TypingTool::Auto {
        return match preferred_tool {
            TypingTool::Wtype if is_wtype_available() => {
                info!("Using user-specified wtype");
                type_text_via_wtype(text)?;
                Ok(true)
            }
            TypingTool::Kwtype if is_kwtype_available() => {
                info!("Using user-specified kwtype");
                type_text_via_kwtype(text)?;
                Ok(true)
            }
            TypingTool::Dotool if is_dotool_available() => {
                info!("Using user-specified dotool");
                type_text_via_dotool(text)?;
                Ok(true)
            }
            TypingTool::Ydotool if is_ydotool_available() => {
                info!("Using user-specified ydotool");
                type_text_via_ydotool(text)?;
                Ok(true)
            }
            TypingTool::Xdotool if is_xdotool_available() => {
                info!("Using user-specified xdotool");
                type_text_via_xdotool(text)?;
                Ok(true)
            }
            _ => Err(format!(
                "Typing tool {:?} is not available on this system",
                preferred_tool
            )),
        };
    }

    // Auto mode - existing fallback chain
    if is_wayland() {
        // KDE Wayland: prefer kwtype (uses KDE Fake Input protocol, supports umlauts)
        if is_kde_wayland() && is_kwtype_available() {
            info!("Using kwtype for direct text input on KDE Wayland");
            type_text_via_kwtype(text)?;
            return Ok(true);
        }
        // Wayland: prefer wtype, then dotool, then ydotool
        // Note: wtype doesn't work on KDE (no zwp_virtual_keyboard_manager_v1 support)
        if !is_kde_wayland() && is_wtype_available() {
            info!("Using wtype for direct text input");
            type_text_via_wtype(text)?;
            return Ok(true);
        }
        if is_dotool_available() {
            info!("Using dotool for direct text input");
            type_text_via_dotool(text)?;
            return Ok(true);
        }
        if is_ydotool_available() {
            info!("Using ydotool for direct text input");
            type_text_via_ydotool(text)?;
            return Ok(true);
        }
    } else {
        // X11: prefer xdotool, then ydotool
        if is_xdotool_available() {
            info!("Using xdotool for direct text input");
            type_text_via_xdotool(text)?;
            return Ok(true);
        }
        if is_ydotool_available() {
            info!("Using ydotool for direct text input");
            type_text_via_ydotool(text)?;
            return Ok(true);
        }
    }

    Ok(false)
}

/// Returns the list of available typing tools on this system.
/// Always includes "auto" as the first entry.
#[cfg(target_os = "linux")]
pub fn get_available_typing_tools() -> Vec<String> {
    let mut tools = vec!["auto".to_string()];
    if is_wtype_available() {
        tools.push("wtype".to_string());
    }
    if is_kwtype_available() {
        tools.push("kwtype".to_string());
    }
    if is_dotool_available() {
        tools.push("dotool".to_string());
    }
    if is_ydotool_available() {
        tools.push("ydotool".to_string());
    }
    if is_xdotool_available() {
        tools.push("xdotool".to_string());
    }
    tools
}

/// Check if wtype is available (Wayland text input tool)
#[cfg(target_os = "linux")]
fn is_wtype_available() -> bool {
    Command::new("which")
        .arg("wtype")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if dotool is available (another Wayland text input tool)
#[cfg(target_os = "linux")]
fn is_dotool_available() -> bool {
    Command::new("which")
        .arg("dotool")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if ydotool is available (uinput-based, works on both Wayland and X11)
#[cfg(target_os = "linux")]
fn is_ydotool_available() -> bool {
    Command::new("which")
        .arg("ydotool")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn is_xdotool_available() -> bool {
    Command::new("which")
        .arg("xdotool")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if kwtype is available (KDE Wayland virtual keyboard input tool)
#[cfg(target_os = "linux")]
fn is_kwtype_available() -> bool {
    Command::new("which")
        .arg("kwtype")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if wl-copy is available (Wayland clipboard tool)
#[cfg(target_os = "linux")]
fn is_wl_copy_available() -> bool {
    Command::new("which")
        .arg("wl-copy")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Type text directly via wtype on Wayland.
#[cfg(target_os = "linux")]
fn type_text_via_wtype(text: &str) -> Result<(), String> {
    let output = Command::new("wtype")
        .arg("--") // Protect against text starting with -
        .arg(text)
        .output()
        .map_err(|e| format!("Failed to execute wtype: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("wtype failed: {}", stderr));
    }

    Ok(())
}

/// Type text directly via xdotool on X11.
#[cfg(target_os = "linux")]
fn type_text_via_xdotool(text: &str) -> Result<(), String> {
    let output = Command::new("xdotool")
        .arg("type")
        .arg("--clearmodifiers")
        .arg("--")
        .arg(text)
        .output()
        .map_err(|e| format!("Failed to execute xdotool: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("xdotool failed: {}", stderr));
    }

    Ok(())
}

/// Type text directly via dotool (works on both Wayland and X11 via uinput).
#[cfg(target_os = "linux")]
fn type_text_via_dotool(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("dotool")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn dotool: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        // dotool uses "type <text>" command
        writeln!(stdin, "type {}", text)
            .map_err(|e| format!("Failed to write to dotool stdin: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to wait for dotool: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("dotool failed: {}", stderr));
    }

    Ok(())
}

/// Type text directly via ydotool (uinput-based, requires ydotoold daemon).
#[cfg(target_os = "linux")]
fn type_text_via_ydotool(text: &str) -> Result<(), String> {
    let output = Command::new("ydotool")
        .arg("type")
        .arg("--")
        .arg(text)
        .output()
        .map_err(|e| format!("Failed to execute ydotool: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ydotool failed: {}", stderr));
    }

    Ok(())
}

/// Type text directly via kwtype (KDE Wayland virtual keyboard, uses KDE Fake Input protocol).
#[cfg(target_os = "linux")]
fn type_text_via_kwtype(text: &str) -> Result<(), String> {
    let output = Command::new("kwtype")
        .arg("--")
        .arg(text)
        .output()
        .map_err(|e| format!("Failed to execute kwtype: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("kwtype failed: {}", stderr));
    }

    Ok(())
}

/// Write text to clipboard via wl-copy (Wayland clipboard tool).
/// Uses Stdio::null() to avoid blocking on repeated calls — wl-copy forks a
/// daemon that inherits piped fds, causing read_to_end to hang indefinitely.
#[cfg(target_os = "linux")]
fn write_clipboard_via_wl_copy(text: &str) -> Result<(), String> {
    use std::process::Stdio;
    let status = Command::new("wl-copy")
        .arg("--")
        .arg(text)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to execute wl-copy: {}", e))?;

    if !status.success() {
        return Err("wl-copy failed".into());
    }

    Ok(())
}

/// Send a key combination (e.g., Ctrl+V) via wtype on Wayland.
#[cfg(target_os = "linux")]
fn send_key_combo_via_wtype(paste_method: &PasteMethod) -> Result<(), String> {
    let args: Vec<&str> = match paste_method {
        PasteMethod::CtrlV => vec!["-M", "ctrl", "-k", "v"],
        PasteMethod::ShiftInsert => vec!["-M", "shift", "-k", "Insert"],
        PasteMethod::CtrlShiftV => vec!["-M", "ctrl", "-M", "shift", "-k", "v"],
        _ => return Err("Unsupported paste method".into()),
    };

    let output = Command::new("wtype")
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to execute wtype: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("wtype failed: {}", stderr));
    }

    Ok(())
}

/// Send a key combination (e.g., Ctrl+V) via dotool.
#[cfg(target_os = "linux")]
fn send_key_combo_via_dotool(paste_method: &PasteMethod) -> Result<(), String> {
    let command;
    match paste_method {
        PasteMethod::CtrlV => command = "echo key ctrl+v | dotool",
        PasteMethod::ShiftInsert => command = "echo key shift+insert | dotool",
        PasteMethod::CtrlShiftV => command = "echo key ctrl+shift+v | dotool",
        _ => return Err("Unsupported paste method".into()),
    }
    use std::process::Stdio;
    let status = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to execute dotool: {}", e))?;
    if !status.success() {
        return Err("dotool failed".into());
    }

    Ok(())
}

/// Send a key combination (e.g., Ctrl+V) via ydotool (requires ydotoold daemon).
#[cfg(target_os = "linux")]
fn send_key_combo_via_ydotool(paste_method: &PasteMethod) -> Result<(), String> {
    // ydotool uses Linux input event keycodes with format <keycode>:<pressed>
    // where pressed is 1 for down, 0 for up. Keycodes: ctrl=29, shift=42, v=47, insert=110
    let args: Vec<&str> = match paste_method {
        PasteMethod::CtrlV => vec!["key", "29:1", "47:1", "47:0", "29:0"],
        PasteMethod::ShiftInsert => vec!["key", "42:1", "110:1", "110:0", "42:0"],
        PasteMethod::CtrlShiftV => vec!["key", "29:1", "42:1", "47:1", "47:0", "42:0", "29:0"],
        _ => return Err("Unsupported paste method".into()),
    };

    let output = Command::new("ydotool")
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to execute ydotool: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ydotool failed: {}", stderr));
    }

    Ok(())
}

/// Send a key combination (e.g., Ctrl+V) via xdotool on X11.
#[cfg(target_os = "linux")]
fn send_key_combo_via_xdotool(paste_method: &PasteMethod) -> Result<(), String> {
    let key_combo = match paste_method {
        PasteMethod::CtrlV => "ctrl+v",
        PasteMethod::CtrlShiftV => "ctrl+shift+v",
        PasteMethod::ShiftInsert => "shift+Insert",
        _ => return Err("Unsupported paste method".into()),
    };

    let output = Command::new("xdotool")
        .arg("key")
        .arg("--clearmodifiers")
        .arg(key_combo)
        .output()
        .map_err(|e| format!("Failed to execute xdotool: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("xdotool failed: {}", stderr));
    }

    Ok(())
}

/// Pastes text by invoking an external script.
/// The script receives the text to paste as a single argument.
fn paste_via_external_script(text: &str, script_path: &str) -> Result<(), String> {
    info!("Pasting via external script: {}", script_path);

    let output = Command::new(script_path)
        .arg(text)
        .output()
        .map_err(|e| format!("Failed to execute external script '{}': {}", script_path, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "External script '{}' failed with exit code {:?}. stderr: {}, stdout: {}",
            script_path,
            output.status.code(),
            stderr.trim(),
            stdout.trim()
        ));
    }

    Ok(())
}

/// Types text directly by simulating individual key presses.
fn paste_direct(
    enigo: &mut Enigo,
    text: &str,
    #[cfg(target_os = "linux")] typing_tool: TypingTool,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        if try_direct_typing_linux(text, typing_tool)? {
            return Ok(());
        }
        info!("Falling back to enigo for direct text input");
    }

    input::paste_text_direct(enigo, text)
}

fn send_return_key(enigo: &mut Enigo, key_type: AutoSubmitKey) -> Result<(), String> {
    match key_type {
        AutoSubmitKey::Enter => {
            enigo
                .key(Key::Return, Direction::Press)
                .map_err(|e| format!("Failed to press Return key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Release)
                .map_err(|e| format!("Failed to release Return key: {}", e))?;
        }
        AutoSubmitKey::CtrlEnter => {
            enigo
                .key(Key::Control, Direction::Press)
                .map_err(|e| format!("Failed to press Control key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Press)
                .map_err(|e| format!("Failed to press Return key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Release)
                .map_err(|e| format!("Failed to release Return key: {}", e))?;
            enigo
                .key(Key::Control, Direction::Release)
                .map_err(|e| format!("Failed to release Control key: {}", e))?;
        }
        AutoSubmitKey::CmdEnter => {
            enigo
                .key(Key::Meta, Direction::Press)
                .map_err(|e| format!("Failed to press Meta/Cmd key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Press)
                .map_err(|e| format!("Failed to press Return key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Release)
                .map_err(|e| format!("Failed to release Return key: {}", e))?;
            enigo
                .key(Key::Meta, Direction::Release)
                .map_err(|e| format!("Failed to release Meta/Cmd key: {}", e))?;
        }
    }

    Ok(())
}

fn should_send_auto_submit(auto_submit: bool, paste_method: PasteMethod) -> bool {
    auto_submit && paste_method != PasteMethod::None
}

/// Outcome of the guarded dictation paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardedPasteOutcome {
    /// Every observable check passed and exactly one paste was delivered.
    Pasted,
    /// The user configured `PasteMethod::None`; nothing was attempted.
    NothingToDo,
    /// The paste was not (or not verifiably) delivered. Unless the reason says
    /// otherwise, the full transcript was left in the clipboard on purpose and
    /// the caller must show a visible notice.
    Fallback(PasteFallback),
}

/// `AXIsProcessTrusted` — whether macOS lets this process post synthetic
/// input events. Querying it does NOT show the permission prompt.
#[cfg(target_os = "macos")]
fn macos_accessibility_trusted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Write `text` to the clipboard and verify it landed there by reading it
/// back. One silent retry, because another process can hold the clipboard
/// open for a moment.
fn write_clipboard_verified(app_handle: &AppHandle, text: &str) -> bool {
    let clipboard = app_handle.clipboard();
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(60));
        }
        if clipboard.write_text(text).is_err() {
            continue;
        }
        if clipboard.read_text().is_ok_and(|readback| readback == text) {
            return true;
        }
    }
    false
}

/// Paste a finished dictation with fail-closed guards (Windows).
///
/// The contract behind it: a successfully finished dictation either lands in
/// the window the user stopped in, or the full transcript stays in the
/// clipboard and the caller shows a visible notice. Silent loss is the one
/// outcome this function must never produce; the invisible failure modes that
/// remain (a target that ignores Ctrl+V) are documented, and the transcript
/// additionally survives in the history window.
///
/// On non-Windows targets the guards cannot observe anything, so this
/// delegates to the legacy [`paste`] unchanged.
pub fn paste_transcript_guarded(
    text: String,
    app_handle: AppHandle,
    target: Option<PasteTarget>,
) -> GuardedPasteOutcome {
    if !cfg!(target_os = "windows") {
        let settings = get_settings(&app_handle);
        // What a manual paste should insert: same trailing-space transform
        // that paste() applies internally, so parked clipboard content and a
        // delivered paste are byte-identical.
        let parked_text = if settings.append_trailing_space {
            format!("{} ", text)
        } else {
            text.clone()
        };

        if settings.paste_method == PasteMethod::None {
            // Same shape as the Windows branch below: honour the opt-in
            // copy-to-clipboard side effect (which paste() would also do,
            // but the guarded outcome must say NothingToDo, not Pasted).
            if settings.clipboard_handling == ClipboardHandling::CopyToClipboard {
                let _ = app_handle.clipboard().write_text(&parked_text);
            }
            return GuardedPasteOutcome::NothingToDo;
        }

        // Without the Accessibility permission macOS drops synthetic
        // keystrokes WITHOUT an error: paste() would report success, and its
        // clipboard-restore step would then remove the transcript again —
        // the text would survive only in history. Park it instead and tell
        // the user what is missing. (Ad-hoc-signed builds lose the granted
        // permission on every update, so this is a common state, not an
        // edge case.) ExternalScript stays exempt: a user script needs no
        // Accessibility permission and may be exactly the workaround for it.
        #[cfg(target_os = "macos")]
        if settings.paste_method != PasteMethod::ExternalScript
            && !macos_accessibility_trusted()
        {
            warn!("paste: macOS Accessibility permission missing — parking transcript in clipboard");
            if write_clipboard_verified(&app_handle, &parked_text) {
                return GuardedPasteOutcome::Fallback(PasteFallback::AccessibilityDenied);
            }
            return GuardedPasteOutcome::Fallback(PasteFallback::ClipboardUnverified);
        }

        return match paste(text.clone(), app_handle.clone()) {
            Ok(()) => GuardedPasteOutcome::Pasted,
            Err(_) => {
                // paste() only errors BEFORE delivering the keystroke (post-
                // paste steps log instead of erroring, see its tail), and its
                // clipboard restore has then already run — re-park so the
                // fallback notice's "the text is in the clipboard" is true.
                if write_clipboard_verified(&app_handle, &parked_text) {
                    GuardedPasteOutcome::Fallback(PasteFallback::InjectionFailed)
                } else {
                    GuardedPasteOutcome::Fallback(PasteFallback::ClipboardUnverified)
                }
            }
        };
    }

    let settings = get_settings(&app_handle);
    let paste_method = settings.paste_method;
    if paste_method == PasteMethod::None {
        // The user opted out of auto-paste entirely; keep the legacy
        // copy-to-clipboard side effect and do nothing else.
        if settings.clipboard_handling == ClipboardHandling::CopyToClipboard {
            let _ = app_handle.clipboard().write_text(&text);
        }
        return GuardedPasteOutcome::NothingToDo;
    }

    let text = if settings.append_trailing_space {
        format!("{} ", text)
    } else {
        text
    };

    // Log the inputs of the decision, never the transcript. Without this a
    // guard that silently decides "fine, paste it" is indistinguishable from
    // one that never ran.
    let foreground = paste_guard::current_foreground();
    let self_is_elevated = paste_guard::self_elevated();
    let target_is_elevated = target.and_then(|t| paste_guard::process_elevated(t.pid));
    info!(
        "paste guard: target={:?} foreground={:?} self_elevated={} target_elevated={:?}",
        target, foreground, self_is_elevated, target_is_elevated
    );

    let target = match paste_guard::preflight(
        target,
        foreground,
        |_| target_is_elevated,
        self_is_elevated,
    ) {
        Ok(target) => target,
        Err(reason) => {
            // No paste attempt. Park the transcript in the clipboard so the
            // user can insert it manually — and only claim that in the notice
            // if the clipboard verifiably holds it.
            if write_clipboard_verified(&app_handle, &text) {
                return GuardedPasteOutcome::Fallback(reason);
            }
            return GuardedPasteOutcome::Fallback(PasteFallback::ClipboardUnverified);
        }
    };

    // Save the previous clipboard for the success path. Text has priority;
    // an image is only probed when there is no text (same policy as the
    // legacy paste path).
    let clipboard = app_handle.clipboard();
    let saved_text = clipboard.read_text().ok().filter(|t| !t.is_empty());
    let saved_image = if saved_text.is_none() {
        clipboard.read_image().ok().map(|image| image.to_owned())
    } else {
        None
    };

    let uses_clipboard = matches!(
        paste_method,
        PasteMethod::CtrlV | PasteMethod::CtrlShiftV | PasteMethod::ShiftInsert
    );
    if uses_clipboard && !write_clipboard_verified(&app_handle, &text) {
        return GuardedPasteOutcome::Fallback(PasteFallback::ClipboardUnverified);
    }

    let enigo_state = match app_handle.try_state::<EnigoState>() {
        Some(state) => state,
        None => {
            warn!("guarded paste: Enigo state not initialized");
            return GuardedPasteOutcome::Fallback(PasteFallback::InjectionFailed);
        }
    };
    let mut enigo = match enigo_state.0.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn!("guarded paste: Enigo mutex poisoned, recovering");
            poisoned.into_inner()
        }
    };

    std::thread::sleep(Duration::from_millis(settings.paste_delay_ms));

    // Exactly one attempt, no matter what.
    let injection = match paste_method {
        PasteMethod::CtrlV => input::send_paste_ctrl_v(&mut enigo),
        PasteMethod::CtrlShiftV => input::send_paste_ctrl_shift_v(&mut enigo),
        PasteMethod::ShiftInsert => input::send_paste_shift_insert(&mut enigo),
        PasteMethod::Direct => paste_direct(
            &mut enigo,
            &text,
            #[cfg(target_os = "linux")]
            settings.typing_tool,
        ),
        PasteMethod::ExternalScript => {
            let script = settings
                .external_script_path
                .as_deref()
                .filter(|path| !path.is_empty());
            match script {
                Some(path) => paste_via_external_script(&text, path),
                None => Err("External script path is not configured".to_string()),
            }
        }
        PasteMethod::None => unreachable!("handled above"),
    };
    if let Err(error) = injection {
        warn!("guarded paste: injection failed: {error}");
        drop(enigo);
        // For non-clipboard methods the transcript is not in the clipboard
        // yet; park it there so the notice can point somewhere real.
        if uses_clipboard || write_clipboard_verified(&app_handle, &text) {
            return GuardedPasteOutcome::Fallback(PasteFallback::InjectionFailed);
        }
        return GuardedPasteOutcome::Fallback(PasteFallback::ClipboardUnverified);
    }

    // Give the target time to service the paste before the clipboard changes
    // again. The floor matters: pasting is asynchronous, and restoring the old
    // clipboard too early makes the target read the OLD content.
    std::thread::sleep(Duration::from_millis(
        settings.paste_delay_after_ms.max(150),
    ));

    if paste_guard::current_foreground() != Some(target.hwnd) {
        // Focus moved while the paste was in flight; whether the right window
        // received it is unknowable. Keep the transcript in the clipboard and
        // say so instead of restoring.
        return GuardedPasteOutcome::Fallback(PasteFallback::FocusChangedDuringPaste);
    }

    if should_send_auto_submit(settings.auto_submit, paste_method) {
        std::thread::sleep(Duration::from_millis(50));
        if let Err(error) = send_return_key(&mut enigo, settings.auto_submit_key) {
            warn!("guarded paste: auto-submit failed: {error}");
        }
    }
    drop(enigo);

    // Success path: restore the previous clipboard only now, per setting.
    if uses_clipboard {
        match settings.clipboard_handling {
            ClipboardHandling::CopyToClipboard => {
                // Transcript intentionally stays in the clipboard.
            }
            _ => {
                if let Some(previous) = saved_text {
                    let _ = clipboard.write_text(&previous);
                } else if let Some(image) = saved_image {
                    let _ = clipboard.write_image(&image);
                } else {
                    let _ = clipboard.clear();
                }
            }
        }
    } else if settings.clipboard_handling == ClipboardHandling::CopyToClipboard {
        let _ = clipboard.write_text(&text);
    }

    GuardedPasteOutcome::Pasted
}

pub fn paste(text: String, app_handle: AppHandle) -> Result<(), String> {
    let settings = get_settings(&app_handle);
    let paste_method = settings.paste_method;
    let paste_delay_ms = settings.paste_delay_ms;
    let paste_delay_after_ms = settings.paste_delay_after_ms;

    // Append trailing space if setting is enabled
    let text = if settings.append_trailing_space {
        format!("{} ", text)
    } else {
        text
    };

    info!(
        "Using paste method: {:?}, delay before: {}ms, delay after: {}ms",
        paste_method, paste_delay_ms, paste_delay_after_ms
    );

    // Get the managed Enigo instance
    let enigo_state = app_handle
        .try_state::<EnigoState>()
        .ok_or("Enigo state not initialized")?;
    let mut enigo = enigo_state
        .0
        .lock()
        .map_err(|e| format!("Failed to lock Enigo: {}", e))?;

    // Streaming/segment callers reach paste() directly (segmenter.rs), so the
    // Accessibility probe must also live here: without it macOS swallows the
    // keystroke silently, this function reports Ok, and the clipboard restore
    // wipes the text. Fail loudly instead. ExternalScript needs no permission.
    #[cfg(target_os = "macos")]
    if !matches!(paste_method, PasteMethod::None | PasteMethod::ExternalScript)
        && !macos_accessibility_trusted()
    {
        return Err(
            "macOS Accessibility permission missing; synthetic keystrokes would be dropped"
                .to_string(),
        );
    }

    // Perform the paste operation
    match paste_method {
        PasteMethod::None => {
            info!("PasteMethod::None selected - skipping paste action");
        }
        PasteMethod::Direct => {
            paste_direct(
                &mut enigo,
                &text,
                #[cfg(target_os = "linux")]
                settings.typing_tool,
            )?;
        }
        PasteMethod::CtrlV | PasteMethod::CtrlShiftV | PasteMethod::ShiftInsert => {
            paste_via_clipboard(
                &mut enigo,
                &text,
                &app_handle,
                &paste_method,
                paste_delay_ms,
                paste_delay_after_ms,
            )?
        }
        PasteMethod::ExternalScript => {
            let script_path = settings
                .external_script_path
                .as_ref()
                .filter(|p| !p.is_empty())
                .ok_or("External script path is not configured")?;
            paste_via_external_script(&text, script_path)?;
        }
    }

    // Everything below runs AFTER the text was delivered. These steps must
    // not turn into an Err: callers treat Err as "nothing was inserted"
    // (paste_transcript_guarded re-parks the clipboard and tells the user to
    // paste manually — which would duplicate the already-inserted text).
    if should_send_auto_submit(settings.auto_submit, paste_method) {
        std::thread::sleep(Duration::from_millis(50));
        if let Err(e) = send_return_key(&mut enigo, settings.auto_submit_key) {
            warn!("paste: auto-submit keystroke failed after successful paste: {e}");
        }
    }

    // After pasting, optionally copy to clipboard based on settings
    if settings.clipboard_handling == ClipboardHandling::CopyToClipboard {
        if let Err(e) = app_handle.clipboard().write_text(&text) {
            warn!("paste: copy-to-clipboard after successful paste failed: {e}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_submit_requires_setting_enabled() {
        assert!(!should_send_auto_submit(false, PasteMethod::CtrlV));
        assert!(!should_send_auto_submit(false, PasteMethod::Direct));
    }

    #[test]
    fn auto_submit_skips_none_paste_method() {
        assert!(!should_send_auto_submit(true, PasteMethod::None));
    }

    #[test]
    fn auto_submit_runs_for_active_paste_methods() {
        assert!(should_send_auto_submit(true, PasteMethod::CtrlV));
        assert!(should_send_auto_submit(true, PasteMethod::Direct));
        assert!(should_send_auto_submit(true, PasteMethod::CtrlShiftV));
        assert!(should_send_auto_submit(true, PasteMethod::ShiftInsert));
    }
}
