//! Fail-closed insertion for dictation modes that insert text SEVERAL times per
//! run: sentence mode (`segmenter.rs`) and live injection (`stream_injection`).
//!
//! `paste_guard` protects exactly one paste attempt after the stop hotkey. These
//! modes cannot reuse that contract verbatim — "exactly one attempt" is false by
//! construction, and one fallback overlay per sentence would be unusable. What
//! carries over is the principle (docs/DECISIONS.md D7): whenever the outcome of
//! an insertion would be uncertain, do not guess.
//!
//! The error model, per run:
//!
//! 1. The window that is in the foreground when the FIRST fragment is about to
//!    be inserted becomes the run's target (batch captures at stop; live text
//!    appears while speaking, so the first insertion is the equivalent moment).
//! 2. Every fragment is checked against that target with the same
//!    [`paste_guard::preflight`] the batch path uses: same window, target not
//!    elevated while we are not, elevation unknown counts as elevated.
//! 3. The first deviation BLOCKS the session for the rest of the run. That
//!    fragment and every later one is buffered instead of inserted, so order is
//!    kept and nothing is typed into a foreign window. Blocking is sticky on
//!    purpose: resuming after the user came back would insert text in the
//!    middle of whatever they did in between.
//! 4. At the end of the run the caller takes the buffered remainder ONCE and
//!    tells the user (one notice, remainder in the clipboard) — see
//!    [`HeldRemainder`].
//!
//! Everything here is a pure state machine over a [`Surroundings`] probe, so the
//! decisions are testable without a desktop.

use crate::paste_guard::{self, PasteFallback, PasteTarget};

/// What the session may observe about the desktop. Production uses
/// [`SystemSurroundings`]; tests script focus changes and elevation.
pub(crate) trait Surroundings {
    /// Whether foreground and elevation can be observed at all. Only Windows
    /// can; elsewhere the guard has nothing to check and (like the batch path)
    /// leaves the insertion to the platform code unchanged.
    fn observable(&self) -> bool;
    /// The foreground window right now.
    fn foreground(&self) -> Option<PasteTarget>;
    /// Whether the process behind `pid` is elevated; `None` when unknown.
    fn target_elevated(&self, pid: u32) -> Option<bool>;
    /// Whether this app itself runs elevated.
    fn self_elevated(&self) -> bool;
}

/// The real desktop.
pub(crate) struct SystemSurroundings;

impl Surroundings for SystemSurroundings {
    fn observable(&self) -> bool {
        cfg!(target_os = "windows")
    }

    fn foreground(&self) -> Option<PasteTarget> {
        paste_guard::capture_paste_target()
    }

    fn target_elevated(&self, pid: u32) -> Option<bool> {
        paste_guard::process_elevated(pid)
    }

    fn self_elevated(&self) -> bool {
        paste_guard::self_elevated()
    }
}

/// Verdict for one fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gate {
    /// Insert it now. `target` is the window to hand to the guarded paste
    /// (`None` where nothing can be observed).
    Deliver { target: Option<PasteTarget> },
    /// Do not insert. The fragment is buffered in the session.
    Held(PasteFallback),
}

/// Text a run could not (verifiably) insert, handed to the user once at the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeldRemainder {
    /// The buffered fragments in their original order, byte-exact.
    pub(crate) text: String,
    /// Why the session blocked (the first deviation of the run).
    pub(crate) reason: PasteFallback,
    /// The run's target window, for the one guarded retry at the end.
    pub(crate) target: Option<PasteTarget>,
}

impl HeldRemainder {
    /// True when NO insertion keystroke was attempted for the buffered text
    /// (the gate refused before anything was sent). Only then may the caller
    /// try the guarded paste once more at the end — for example because the
    /// user is back in the target window. After an attempt whose outcome is
    /// unknown (failed keystroke, focus moved mid-paste) a second attempt
    /// could insert the text twice, which D7 rules out: park it instead.
    pub(crate) fn untouched(&self) -> bool {
        matches!(
            self.reason,
            PasteFallback::NoTarget | PasteFallback::FocusChanged | PasteFallback::TargetElevated
        )
    }
}

/// What the end of a run has to report about buffered text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HeldOutcome {
    /// Everything that was handed in was inserted.
    Nothing,
    /// A remainder was buffered and must be reported.
    Held(HeldRemainder),
    /// The component holding the buffer could not be asked (worker gone or
    /// stuck). The text is then only in the history; the caller must still
    /// show a visible notice instead of staying silent.
    Lost,
}

/// Per-run state of the multi-insert guard.
#[derive(Debug, Default)]
pub(crate) struct PasteSession {
    target: Option<PasteTarget>,
    blocked: Option<PasteFallback>,
    held: Vec<String>,
}

impl PasteSession {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Decide whether `fragment` may be inserted now. A refused fragment is
    /// buffered here; the caller must not insert it anywhere.
    pub(crate) fn gate(&mut self, fragment: &str, env: &dyn Surroundings) -> Gate {
        if let Some(reason) = self.blocked {
            self.held.push(fragment.to_string());
            return Gate::Held(reason);
        }
        if !env.observable() {
            return Gate::Deliver { target: None };
        }

        let current = env.foreground();
        if self.target.is_none() {
            // The first fragment defines the target, even if it is refused
            // right away: the end-of-run retry needs to know the window.
            self.target = current;
        }
        match paste_guard::preflight(
            self.target,
            current.map(|window| window.hwnd),
            |pid| env.target_elevated(pid),
            env.self_elevated(),
        ) {
            Ok(target) => Gate::Deliver {
                target: Some(target),
            },
            Err(reason) => {
                self.block(reason, fragment);
                Gate::Held(reason)
            }
        }
    }

    /// Record that a fragment that passed the gate did NOT verifiably land (the
    /// keystroke failed, or the guarded paste reported a fallback). It is
    /// buffered, and the session blocks so later fragments cannot be inserted
    /// behind a gap.
    pub(crate) fn not_delivered(&mut self, fragment: &str, reason: PasteFallback) {
        self.block(reason, fragment);
    }

    /// Check the foreground again after a fragment was inserted. If the window
    /// moved while the paste was in flight, whether the target received it is
    /// unknowable: the fragment is treated as not verifiably delivered.
    pub(crate) fn confirm_delivery(&mut self, fragment: &str, env: &dyn Surroundings) {
        if !env.observable() {
            return;
        }
        let Some(target) = self.target else {
            return;
        };
        if env.foreground().map(|window| window.hwnd) != Some(target.hwnd) {
            self.block(PasteFallback::FocusChangedDuringPaste, fragment);
        }
    }

    /// Whether the session already refuses further insertions.
    pub(crate) fn is_blocked(&self) -> bool {
        self.blocked.is_some()
    }

    /// Hand out the buffered text, once. `None` when nothing was held.
    pub(crate) fn take_remainder(&mut self) -> Option<HeldRemainder> {
        if self.held.is_empty() {
            return None;
        }
        let text = std::mem::take(&mut self.held).concat();
        Some(HeldRemainder {
            text,
            // A non-empty buffer always comes with a reason; the fallback only
            // keeps the function total.
            reason: self.blocked.unwrap_or(PasteFallback::InjectionFailed),
            target: self.target,
        })
    }

    fn block(&mut self, reason: PasteFallback, fragment: &str) {
        // The first deviation is the cause worth telling the user about.
        self.blocked.get_or_insert(reason);
        self.held.push(fragment.to_string());
    }
}

/// Scripted desktop for tests of every module that uses the session.
#[cfg(test)]
pub(crate) mod testing {
    use super::Surroundings;
    use crate::paste_guard::PasteTarget;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    pub(crate) struct FakeDesktop {
        pub(crate) observable: bool,
        pub(crate) foreground: RefCell<Option<PasteTarget>>,
        /// Elevation per pid; a missing pid is "unknown".
        pub(crate) elevated: RefCell<HashMap<u32, bool>>,
        pub(crate) self_elevated: bool,
        /// How often the (comparatively expensive) elevation query ran.
        pub(crate) elevation_queries: Cell<usize>,
    }

    pub(crate) fn window(hwnd: isize, pid: u32) -> PasteTarget {
        PasteTarget { hwnd, pid }
    }

    impl FakeDesktop {
        /// A desktop whose foreground is `window` and whose processes are all
        /// known to be unelevated.
        pub(crate) fn focused(window: PasteTarget) -> Self {
            let mut elevated = HashMap::new();
            elevated.insert(window.pid, false);
            Self {
                observable: true,
                foreground: RefCell::new(Some(window)),
                elevated: RefCell::new(elevated),
                self_elevated: false,
                elevation_queries: Cell::new(0),
            }
        }

        pub(crate) fn focus(&self, window: Option<PasteTarget>) {
            if let Some(window) = window {
                self.elevated
                    .borrow_mut()
                    .entry(window.pid)
                    .or_insert(false);
            }
            *self.foreground.borrow_mut() = window;
        }

        pub(crate) fn set_elevated(&self, pid: u32, elevated: bool) {
            self.elevated.borrow_mut().insert(pid, elevated);
        }
    }

    impl Surroundings for FakeDesktop {
        fn observable(&self) -> bool {
            self.observable
        }

        fn foreground(&self) -> Option<PasteTarget> {
            *self.foreground.borrow()
        }

        fn target_elevated(&self, pid: u32) -> Option<bool> {
            self.elevation_queries.set(self.elevation_queries.get() + 1);
            self.elevated.borrow().get(&pid).copied()
        }

        fn self_elevated(&self) -> bool {
            self.self_elevated
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{window, FakeDesktop};
    use super::{Gate, HeldRemainder, PasteSession};
    use crate::paste_guard::PasteFallback;

    #[test]
    fn normal_case_delivers_every_fragment_into_the_first_window() {
        let desk = FakeDesktop::focused(window(11, 42));
        let mut session = PasteSession::new();

        for fragment in ["Erster Satz.", " Zweiter Satz.", " Dritter."] {
            assert_eq!(
                session.gate(fragment, &desk),
                Gate::Deliver {
                    target: Some(window(11, 42))
                }
            );
            session.confirm_delivery(fragment, &desk);
        }

        assert!(!session.is_blocked());
        assert_eq!(session.take_remainder(), None);
    }

    #[test]
    fn focus_change_buffers_the_fragment_and_everything_after_it() {
        let desk = FakeDesktop::focused(window(11, 42));
        let mut session = PasteSession::new();
        assert!(matches!(session.gate("Eins.", &desk), Gate::Deliver { .. }));

        desk.focus(Some(window(12, 43)));
        assert_eq!(
            session.gate(" Zwei.", &desk),
            Gate::Held(PasteFallback::FocusChanged)
        );
        assert_eq!(
            session.gate(" Drei.", &desk),
            Gate::Held(PasteFallback::FocusChanged)
        );

        let rest = session.take_remainder().expect("remainder");
        assert_eq!(rest.text, " Zwei. Drei.");
        assert_eq!(rest.reason, PasteFallback::FocusChanged);
        assert_eq!(rest.target, Some(window(11, 42)));
    }

    #[test]
    fn blocking_is_sticky_even_when_focus_returns() {
        let desk = FakeDesktop::focused(window(11, 42));
        let mut session = PasteSession::new();
        assert!(matches!(session.gate("A", &desk), Gate::Deliver { .. }));

        desk.focus(Some(window(12, 43)));
        assert!(matches!(session.gate("B", &desk), Gate::Held(_)));

        // Back in the original window: the order would be broken by resuming,
        // and the user did something in between. The rest stays buffered.
        desk.focus(Some(window(11, 42)));
        assert_eq!(
            session.gate("C", &desk),
            Gate::Held(PasteFallback::FocusChanged)
        );
        assert_eq!(session.take_remainder().expect("remainder").text, "BC");
    }

    #[test]
    fn lost_foreground_window_fails_closed() {
        let desk = FakeDesktop::focused(window(11, 42));
        let mut session = PasteSession::new();
        assert!(matches!(session.gate("A", &desk), Gate::Deliver { .. }));

        desk.focus(None);
        assert_eq!(
            session.gate("B", &desk),
            Gate::Held(PasteFallback::FocusChanged)
        );
    }

    #[test]
    fn elevated_target_is_refused_from_the_first_fragment() {
        let desk = FakeDesktop::focused(window(11, 42));
        desk.set_elevated(42, true);
        let mut session = PasteSession::new();

        assert_eq!(
            session.gate("A", &desk),
            Gate::Held(PasteFallback::TargetElevated)
        );
        let rest = session.take_remainder().expect("remainder");
        assert_eq!(rest.text, "A");
        // The target is still remembered for the end-of-run retry.
        assert_eq!(rest.target, Some(window(11, 42)));
    }

    #[test]
    fn unknown_elevation_counts_as_elevated() {
        let desk = FakeDesktop::focused(window(11, 42));
        desk.elevated.borrow_mut().clear();
        let mut session = PasteSession::new();

        assert_eq!(
            session.gate("A", &desk),
            Gate::Held(PasteFallback::TargetElevated)
        );
    }

    #[test]
    fn elevated_self_skips_the_target_elevation_query() {
        let mut desk = FakeDesktop::focused(window(11, 42));
        desk.self_elevated = true;
        desk.set_elevated(42, true);
        let mut session = PasteSession::new();

        assert!(matches!(session.gate("A", &desk), Gate::Deliver { .. }));
        assert_eq!(desk.elevation_queries.get(), 0);
    }

    #[test]
    fn no_foreground_window_at_the_first_fragment_means_no_target() {
        let desk = FakeDesktop::focused(window(11, 42));
        desk.focus(None);
        let mut session = PasteSession::new();

        assert_eq!(
            session.gate("A", &desk),
            Gate::Held(PasteFallback::NoTarget)
        );
        let rest = session.take_remainder().expect("remainder");
        assert_eq!(rest.target, None);
    }

    #[test]
    fn unobservable_platform_inserts_unchanged_and_never_holds() {
        let mut desk = FakeDesktop::focused(window(11, 42));
        desk.observable = false;
        desk.focus(None);
        let mut session = PasteSession::new();

        assert_eq!(session.gate("A", &desk), Gate::Deliver { target: None });
        session.confirm_delivery("A", &desk);
        assert_eq!(session.gate("B", &desk), Gate::Deliver { target: None });
        assert_eq!(session.take_remainder(), None);
    }

    #[test]
    fn later_fragments_do_not_query_elevation_once_blocked() {
        let desk = FakeDesktop::focused(window(11, 42));
        let mut session = PasteSession::new();
        assert!(matches!(session.gate("A", &desk), Gate::Deliver { .. }));
        desk.focus(Some(window(12, 43)));
        assert!(matches!(session.gate("B", &desk), Gate::Held(_)));
        let queries = desk.elevation_queries.get();

        assert!(matches!(session.gate("C", &desk), Gate::Held(_)));
        assert_eq!(desk.elevation_queries.get(), queries);
    }

    #[test]
    fn failed_delivery_is_buffered_and_blocks_what_follows() {
        let desk = FakeDesktop::focused(window(11, 42));
        let mut session = PasteSession::new();
        assert!(matches!(session.gate("A", &desk), Gate::Deliver { .. }));
        assert!(matches!(session.gate(" B", &desk), Gate::Deliver { .. }));
        session.not_delivered(" B", PasteFallback::InjectionFailed);

        assert_eq!(
            session.gate(" C", &desk),
            Gate::Held(PasteFallback::InjectionFailed)
        );
        let rest = session.take_remainder().expect("remainder");
        assert_eq!(rest.text, " B C");
        assert_eq!(rest.reason, PasteFallback::InjectionFailed);
    }

    #[test]
    fn focus_moving_during_the_paste_marks_the_fragment_uncertain() {
        let desk = FakeDesktop::focused(window(11, 42));
        let mut session = PasteSession::new();
        assert!(matches!(session.gate("A", &desk), Gate::Deliver { .. }));

        desk.focus(Some(window(12, 43)));
        session.confirm_delivery("A", &desk);

        assert!(session.is_blocked());
        let rest = session.take_remainder().expect("remainder");
        assert_eq!(rest.text, "A");
        assert_eq!(rest.reason, PasteFallback::FocusChangedDuringPaste);
    }

    #[test]
    fn first_reason_wins() {
        let desk = FakeDesktop::focused(window(11, 42));
        let mut session = PasteSession::new();
        assert!(matches!(session.gate("A", &desk), Gate::Deliver { .. }));
        desk.focus(Some(window(12, 43)));
        assert!(matches!(session.gate("B", &desk), Gate::Held(_)));
        session.not_delivered("C", PasteFallback::InjectionFailed);

        assert_eq!(
            session.take_remainder().expect("remainder").reason,
            PasteFallback::FocusChanged
        );
    }

    #[test]
    fn remainder_is_handed_out_exactly_once() {
        let desk = FakeDesktop::focused(window(11, 42));
        let mut session = PasteSession::new();
        desk.set_elevated(42, true);
        assert!(matches!(session.gate("A", &desk), Gate::Held(_)));

        assert!(session.take_remainder().is_some());
        assert_eq!(session.take_remainder(), None);
    }

    #[test]
    fn retry_at_the_end_is_only_allowed_when_nothing_was_attempted() {
        let remainder = |reason| HeldRemainder {
            text: "x".into(),
            reason,
            target: None,
        };
        assert!(remainder(PasteFallback::NoTarget).untouched());
        assert!(remainder(PasteFallback::FocusChanged).untouched());
        assert!(remainder(PasteFallback::TargetElevated).untouched());
        // An attempt happened and its outcome is unknown: a second one could
        // insert the text twice (D7).
        assert!(!remainder(PasteFallback::InjectionFailed).untouched());
        assert!(!remainder(PasteFallback::FocusChangedDuringPaste).untouched());
        assert!(!remainder(PasteFallback::ClipboardUnverified).untouched());
    }
}
