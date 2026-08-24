//! Guards the single paste attempt of a finished dictation.
//!
//! Windows cannot confirm that a synthetic Ctrl+V reached the target
//! application: `SendInput` only reports that events were queued, and UIPI
//! drops input aimed at an elevated window without raising any error
//! (docs/KNOWN-LIMITATIONS.md). This module therefore checks everything that
//! IS observable around the one attempt and fails closed — whenever the
//! outcome would be uncertain, the caller keeps the transcript in the
//! clipboard and shows a visible notice instead of guessing.

/// Foreground window captured at the stop hotkey — the window the dictation
/// belongs to. Captured at stop (not start) so the user can still move focus
/// to the intended target while speaking; what the guard protects against is
/// focus drifting away *between stop and paste*, while transcription runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PasteTarget {
    pub(crate) hwnd: isize,
    pub(crate) pid: u32,
}

/// Why the guarded paste kept (or had to abandon) the transcript instead of
/// pasting. Every variant maps to a visible user notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasteFallback {
    /// No foreground window could be captured when recording stopped.
    NoTarget,
    /// The foreground window changed between stop and paste.
    FocusChanged,
    /// The target runs elevated while we do not; UIPI would silently swallow
    /// the keystroke. Unknown elevation is treated the same way.
    TargetElevated,
    /// The paste keystroke itself reported an error.
    InjectionFailed,
    /// macOS refuses synthetic keystrokes without the Accessibility
    /// permission — the event is dropped silently, so we do not even try.
    AccessibilityDenied,
    /// The transcript could not be verifiably placed in the clipboard. The
    /// text then only survives in the history window.
    ClipboardUnverified,
    /// The foreground window changed while the paste was being delivered.
    FocusChangedDuringPaste,
}

impl PasteFallback {
    /// Whether the transcript is verifiably in the clipboard after this
    /// fallback (drives which notice the user sees).
    pub(crate) fn transcript_in_clipboard(self) -> bool {
        !matches!(self, PasteFallback::ClipboardUnverified)
    }
}

/// Pure decision for whether the single paste attempt may proceed.
///
/// `target_elevated` is consulted lazily and only when we ourselves are not
/// elevated: an elevated Local Voice AI may paste anywhere, UIPI never blocks
/// downward. `None` (elevation unknown) fails closed — a silent UIPI drop is
/// invisible, an unnecessary clipboard fallback is visible and recoverable.
pub(crate) fn preflight(
    target: Option<PasteTarget>,
    current_foreground: Option<isize>,
    target_elevated: impl FnOnce(u32) -> Option<bool>,
    self_elevated: bool,
) -> Result<PasteTarget, PasteFallback> {
    let target = target.ok_or(PasteFallback::NoTarget)?;
    let current = current_foreground.ok_or(PasteFallback::FocusChanged)?;
    if current != target.hwnd {
        return Err(PasteFallback::FocusChanged);
    }
    if !self_elevated {
        match target_elevated(target.pid) {
            Some(false) => {}
            Some(true) | None => return Err(PasteFallback::TargetElevated),
        }
    }
    Ok(target)
}

#[cfg(target_os = "windows")]
pub(crate) fn capture_paste_target() -> Option<PasteTarget> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    Some(PasteTarget {
        hwnd: hwnd.0 as isize,
        pid,
    })
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn capture_paste_target() -> Option<PasteTarget> {
    None
}

#[cfg(target_os = "windows")]
pub(crate) fn current_foreground() -> Option<isize> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.0.is_null()).then_some(hwnd.0 as isize)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn current_foreground() -> Option<isize> {
    None
}

/// Whether the process behind `pid` runs with an elevated token. `None` when
/// the process cannot be queried — callers treat that as elevated.
#[cfg(target_os = "windows")]
pub(crate) fn process_elevated(pid: u32) -> Option<bool> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let elevated = token_elevated(process);
    let _ = unsafe { CloseHandle(process) };
    elevated
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn process_elevated(_pid: u32) -> Option<bool> {
    Some(false)
}

#[cfg(target_os = "windows")]
pub(crate) fn self_elevated() -> bool {
    use windows::Win32::System::Threading::GetCurrentProcess;

    // GetCurrentProcess returns a pseudo handle that must not be closed.
    token_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn self_elevated() -> bool {
    false
}

#[cfg(target_os = "windows")]
fn token_elevated(process: windows::Win32::Foundation::HANDLE) -> Option<bool> {
    use std::mem::size_of;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::OpenProcessToken;

    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.ok()?;
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    let _ = unsafe { CloseHandle(token) };
    result.ok()?;
    Some(elevation.TokenIsElevated != 0)
}

#[cfg(test)]
mod tests {
    use super::{preflight, PasteFallback, PasteTarget};

    fn target() -> PasteTarget {
        PasteTarget { hwnd: 11, pid: 42 }
    }

    #[test]
    fn missing_capture_or_foreground_fails_closed() {
        assert_eq!(
            preflight(None, Some(11), |_| Some(false), false),
            Err(PasteFallback::NoTarget)
        );
        assert_eq!(
            preflight(Some(target()), None, |_| Some(false), false),
            Err(PasteFallback::FocusChanged)
        );
    }

    #[test]
    fn changed_foreground_window_prevents_the_paste() {
        assert_eq!(
            preflight(Some(target()), Some(12), |_| Some(false), false),
            Err(PasteFallback::FocusChanged)
        );
    }

    #[test]
    fn elevated_or_unqueryable_target_prevents_the_paste() {
        assert_eq!(
            preflight(Some(target()), Some(11), |_| Some(true), false),
            Err(PasteFallback::TargetElevated)
        );
        assert_eq!(
            preflight(Some(target()), Some(11), |_| None, false),
            Err(PasteFallback::TargetElevated)
        );
    }

    #[test]
    fn matching_unelevated_target_may_be_pasted_into() {
        assert_eq!(
            preflight(Some(target()), Some(11), |_| Some(false), false),
            Ok(target())
        );
    }

    #[test]
    fn elevated_self_skips_the_target_elevation_check() {
        assert_eq!(
            preflight(Some(target()), Some(11), |_| unreachable!(), true),
            Ok(target())
        );
    }

    #[test]
    fn clipboard_presence_is_reported_per_fallback_reason() {
        assert!(PasteFallback::FocusChanged.transcript_in_clipboard());
        assert!(PasteFallback::TargetElevated.transcript_in_clipboard());
        assert!(PasteFallback::InjectionFailed.transcript_in_clipboard());
        assert!(PasteFallback::AccessibilityDenied.transcript_in_clipboard());
        assert!(PasteFallback::FocusChangedDuringPaste.transcript_in_clipboard());
        assert!(!PasteFallback::ClipboardUnverified.transcript_in_clipboard());
    }
}
