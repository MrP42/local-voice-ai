use super::injection_state::{ContextKey, PreparedSnapshot, ReplacementPlan, RunState};
use crate::input::{self, EnigoState, ReplacementContext};
use crate::paste_session::{HeldOutcome, HeldRemainder, Surroundings, SystemSurroundings};
use crate::settings::{get_settings, ClipboardHandling, PasteMethod};
use enigo::{Direction, Key, Keyboard};
use log::{debug, warn};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

const TARGET_PASTE_SETTLE_DELAY: Duration = Duration::from_millis(120);
const QUEUE_REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the end of a run waits for the worker to drain its queue and hand
/// over the buffered remainder. The queue is FIFO, so this includes pasting
/// every fragment queued before the request; far longer than that normally
/// takes, and a timeout is reported to the user, never swallowed.
const HELD_REPLY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(crate) struct InjectionHandle {
    tx: Arc<mpsc::Sender<InjectionCommand>>,
}

enum InjectionCommand {
    Begin {
        run_id: u64,
        refinement_enabled: bool,
    },
    Append {
        run_id: u64,
        fragment: String,
    },
    RegisterSentence {
        run_id: u64,
        sentence_id: u64,
        original: String,
    },
    ReplaceSentence {
        run_id: u64,
        sentence_id: u64,
        original: String,
        candidate: String,
    },
    TakeHeld {
        run_id: u64,
        reply: mpsc::SyncSender<Option<HeldRemainder>>,
    },
    PrepareFinal {
        run_id: u64,
        reply: mpsc::SyncSender<Option<PreparedSnapshot>>,
    },
    ReplaceFinal {
        run_id: u64,
        snapshot: PreparedSnapshot,
        candidate: String,
        reply: mpsc::SyncSender<bool>,
    },
    Cancel {
        run_id: u64,
    },
}

impl InjectionHandle {
    pub(crate) fn new(app: &AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();
        let app = app.clone();
        std::thread::spawn(move || run_worker(app, rx));
        Self { tx: Arc::new(tx) }
    }

    pub(crate) fn begin(&self, run_id: u64, refinement_enabled: bool) {
        let _ = self.tx.send(InjectionCommand::Begin {
            run_id,
            refinement_enabled,
        });
    }

    pub(crate) fn append(&self, run_id: u64, fragment: String) {
        let _ = self.tx.send(InjectionCommand::Append { run_id, fragment });
    }

    pub(crate) fn register_sentence(&self, run_id: u64, sentence_id: u64, original: String) {
        let _ = self.tx.send(InjectionCommand::RegisterSentence {
            run_id,
            sentence_id,
            original,
        });
    }

    pub(crate) fn replace_sentence(
        &self,
        run_id: u64,
        sentence_id: u64,
        original: String,
        candidate: String,
    ) {
        let _ = self.tx.send(InjectionCommand::ReplaceSentence {
            run_id,
            sentence_id,
            original,
            candidate,
        });
    }

    /// The text the guard kept out of the target window during this run.
    ///
    /// Goes through the same FIFO as the fragments, so every fragment queued
    /// before this call has been inserted or buffered when the answer comes.
    pub(crate) fn take_held(&self, run_id: u64) -> HeldOutcome {
        self.take_held_within(run_id, HELD_REPLY_TIMEOUT)
    }

    fn take_held_within(&self, run_id: u64, timeout: Duration) -> HeldOutcome {
        let (reply, result) = mpsc::sync_channel(1);
        if self
            .tx
            .send(InjectionCommand::TakeHeld { run_id, reply })
            .is_err()
        {
            return HeldOutcome::Lost;
        }
        match result.recv_timeout(timeout) {
            Ok(Some(remainder)) => HeldOutcome::Held(remainder),
            Ok(None) => HeldOutcome::Nothing,
            Err(_) => HeldOutcome::Lost,
        }
    }

    pub(crate) fn prepare_final(&self, run_id: u64) -> Option<PreparedSnapshot> {
        let (reply, result) = mpsc::sync_channel(1);
        self.tx
            .send(InjectionCommand::PrepareFinal { run_id, reply })
            .ok()?;
        result.recv_timeout(QUEUE_REPLY_TIMEOUT).ok().flatten()
    }

    pub(crate) fn replace_final(
        &self,
        run_id: u64,
        snapshot: PreparedSnapshot,
        candidate: String,
    ) -> bool {
        let (reply, result) = mpsc::sync_channel(1);
        if self
            .tx
            .send(InjectionCommand::ReplaceFinal {
                run_id,
                snapshot,
                candidate,
                reply,
            })
            .is_err()
        {
            return false;
        }
        result.recv_timeout(QUEUE_REPLY_TIMEOUT).unwrap_or(false)
    }

    pub(crate) fn cancel(&self, run_id: u64) {
        let _ = self.tx.send(InjectionCommand::Cancel { run_id });
    }
}

fn run_worker(app: AppHandle, rx: mpsc::Receiver<InjectionCommand>) {
    let mut state: Option<RunState> = None;
    while let Ok(command) = rx.recv() {
        match command {
            InjectionCommand::Begin {
                run_id,
                refinement_enabled,
            } => state = Some(RunState::new(run_id, refinement_enabled)),
            InjectionCommand::Append { run_id, fragment } => {
                let Some(run) = state.as_mut().filter(|run| run.is_run(run_id)) else {
                    continue;
                };
                // The dictation chord's press/release predates the first marker.
                // Held PTT repeats after that marker still count as foreign input.
                input::wait_for_modifiers_released(input::MODIFIER_RELEASE_TIMEOUT);
                let outcome = append_step(
                    run,
                    &fragment,
                    &SystemSurroundings,
                    capture_key,
                    |fragment, frame| paste_fragment(&app, fragment, frame),
                );
                if outcome == AppendOutcome::Buffered {
                    // Length only: the fragment is spoken content. The reason is
                    // logged once, when the run's remainder is reported.
                    debug!(
                        "stream injection: {} bytes buffered instead of inserted",
                        fragment.len()
                    );
                }
            }
            InjectionCommand::TakeHeld { run_id, reply } => {
                let held = state.as_mut().and_then(|run| run.take_held(run_id));
                let _ = reply.send(held);
            }
            InjectionCommand::RegisterSentence {
                run_id,
                sentence_id,
                original,
            } => {
                if let Some(run) = state.as_mut() {
                    run.register_sentence(run_id, sentence_id, &original);
                }
            }
            InjectionCommand::ReplaceSentence {
                run_id,
                sentence_id,
                original,
                candidate,
            } => {
                let Some(run) = state.as_mut().filter(|run| run.is_run(run_id)) else {
                    continue;
                };
                let Some(current) = capture_key() else {
                    run.invalidate();
                    continue;
                };
                let Some(plan) =
                    run.plan_sentence(run_id, sentence_id, &original, &candidate, current)
                else {
                    continue;
                };
                if execute_replacement(&app, run, plan, current) {
                    debug!("Sentence refinement applied");
                }
            }
            InjectionCommand::PrepareFinal { run_id, reply } => {
                let snapshot = state
                    .as_mut()
                    .and_then(|run| run.prepare_final(run_id, capture_key()));
                let _ = reply.send(snapshot);
            }
            InjectionCommand::ReplaceFinal {
                run_id,
                snapshot,
                candidate,
                reply,
            } => {
                let applied = if let Some(run) = state.as_mut().filter(|run| run.is_run(run_id)) {
                    if let Some(current) = capture_key() {
                        if let Some(plan) = run.plan_final(run_id, &snapshot, &candidate, current) {
                            execute_replacement(&app, run, plan, current)
                        } else {
                            false
                        }
                    } else {
                        run.invalidate();
                        false
                    }
                } else {
                    false
                };
                let _ = reply.send(applied);
            }
            InjectionCommand::Cancel { run_id } => {
                if let Some(run) = state.as_mut() {
                    run.cancel(run_id);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppendOutcome {
    /// The fragment went out through the paste path.
    Inserted,
    /// The guard refused it; it sits in the run's buffer.
    Buffered,
}

/// One `Append` of the worker. The guard decides FIRST and independently of
/// refinement (issue #9: it used to hang on `RunState::wants_context()`, so a
/// run without refinement pasted into whatever window had the focus). The
/// effects are injected so the order "guard, then paste" is testable without
/// a desktop.
fn append_step(
    run: &mut RunState,
    fragment: &str,
    env: &dyn Surroundings,
    capture: impl Fn() -> Option<ContextKey>,
    paste: impl FnOnce(&str, &mut input::InjectionFrame) -> bool,
) -> AppendOutcome {
    if !run.admit_fragment(fragment, env) {
        return AppendOutcome::Buffered;
    }
    let track_context = run.wants_context();
    let before = track_context.then(&capture).flatten();
    let mut frame = input::InjectionFrame::new(before.map(|key| key.physical_generation));
    let pasted = paste(fragment, &mut frame);
    let after = track_context.then(&capture).flatten();
    let unchanged = frame.unchanged(after.map(|key| key.physical_generation))
        && (cfg!(target_os = "windows") || before == after);
    run.record_tracked_append(fragment, before, after, pasted, unchanged);
    run.finish_delivery(fragment, pasted, env);
    AppendOutcome::Inserted
}

/// Gibt die Zwischenablage nach einer Injektion wieder her, solange die
/// Einstellung sie unberuehrt lassen soll.
///
/// Beide Injektionswege legen den einzufuegenden Text in der Zwischenablage
/// ab. Ohne diesen Waechter blieb er dort liegen -- bei laufendem Streaming
/// nach jedem Fragment aufs Neue, und damit auch dann, wenn der Nutzer
/// ausdruecklich eingestellt hat, dass die Ablage nicht veraendert werden
/// soll. Wiederhergestellt wird beim Verlassen der Funktion, also nach der
/// Wartezeit, in der die Zielanwendung den Einfuegevorgang verarbeitet, und
/// auch dann, wenn die Injektion unterwegs abgebrochen ist.
struct ClipboardGuard<'a> {
    app: &'a AppHandle,
    text: Option<String>,
    image: Option<tauri::image::Image<'static>>,
    restore: bool,
}

impl<'a> ClipboardGuard<'a> {
    /// Sichert den aktuellen Inhalt. Wird vor jedem Fragment neu aufgerufen:
    /// kopiert der Nutzer waehrend des Diktats etwas, gewinnt sein Inhalt.
    fn capture(app: &'a AppHandle, restore: bool) -> Self {
        if !restore {
            return Self {
                app,
                text: None,
                image: None,
                restore,
            };
        }
        let clipboard = app.clipboard();
        let text = clipboard.read_text().ok().filter(|t| !t.is_empty());
        // Ein Bild zu lesen dekodiert die volle Bitmap, deshalb nur dann,
        // wenn nichts anderes zu retten ist -- wie im Einfuegepfad auch.
        let image = if text.is_none() {
            clipboard.read_image().ok().map(|image| image.to_owned())
        } else {
            None
        };
        Self {
            app,
            text,
            image,
            restore,
        }
    }
}

impl Drop for ClipboardGuard<'_> {
    fn drop(&mut self) {
        if !self.restore {
            return;
        }
        let clipboard = self.app.clipboard();
        if let Some(text) = self.text.take() {
            let _ = clipboard.write_text(&text);
        } else if let Some(image) = self.image.take() {
            let _ = clipboard.write_image(&image);
        } else {
            let _ = clipboard.clear();
        }
    }
}

/// Ob die Zwischenablage nach der Injektion zurueckgesetzt wird. Bei
/// "in die Zwischenablage kopieren" ist das Ueberschreiben gewollt.
fn restores_clipboard(handling: ClipboardHandling) -> bool {
    handling != ClipboardHandling::CopyToClipboard
}

fn paste_fragment(app: &AppHandle, fragment: &str, frame: &mut input::InjectionFrame) -> bool {
    // Stream injection fires every few hundred milliseconds while the user
    // keeps using the machine, so it must honour the configured paste method
    // instead of forcing Ctrl+V on everyone: a held Ctrl is held for the whole
    // desktop, and the user's scrolling then reads as zoom.
    let settings = get_settings(app);
    let direct = settings.paste_method == PasteMethod::Direct;

    // Beim direkten Tippen wird die Ablage gar nicht erst angefasst; sonst
    // haelt der Waechter den vorherigen Inhalt fest.
    let _clipboard = (!direct)
        .then(|| ClipboardGuard::capture(app, restores_clipboard(settings.clipboard_handling)));

    if !direct {
        if let Err(error) = app.clipboard().write_text(fragment) {
            warn!("stream injection: clipboard write failed: {error}");
            return false;
        }
    }
    let Some(state) = app.try_state::<EnigoState>() else {
        warn!("stream injection: Enigo not initialised");
        return false;
    };
    let mut enigo = match state.0.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn!("stream injection: Enigo mutex poisoned, recovering");
            poisoned.into_inner()
        }
    };
    let injected = if direct {
        frame.stroke(|| input::paste_text_direct(&mut enigo, fragment))
    } else {
        input::send_tracked_paste_ctrl_v(&mut enigo, frame, false)
    };
    if let Err(error) = injected {
        warn!("stream injection failed: {error}");
        return false;
    }
    drop(enigo);
    std::thread::sleep(TARGET_PASTE_SETTLE_DELAY);
    true
}

fn execute_replacement(
    app: &AppHandle,
    run: &mut RunState,
    plan: ReplacementPlan,
    expected_context: ContextKey,
) -> bool {
    let _clipboard = ClipboardGuard::capture(
        app,
        restores_clipboard(get_settings(app).clipboard_handling),
    );
    if let Err(error) = app.clipboard().write_text(&plan.replacement) {
        warn!("text refinement: clipboard write failed: {error}");
        run.invalidate();
        return false;
    }
    let Some(state) = app.try_state::<EnigoState>() else {
        warn!("text refinement: Enigo not initialised");
        run.invalidate();
        return false;
    };
    let mut enigo = match state.0.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn!("text refinement: Enigo mutex poisoned, recovering");
            poisoned.into_inner()
        }
    };

    if capture_key() != Some(expected_context) {
        run.invalidate();
        return false;
    }
    let mut frame = input::InjectionFrame::new(Some(expected_context.physical_generation));
    if let Err(error) = input::send_select_left(&mut enigo, plan.select_chars, &mut frame) {
        warn!("text refinement: selection failed: {error}");
        let _ = frame.stroke_checked(|safe| {
            safe.then(|| enigo.key(Key::RightArrow, Direction::Click))
        });
        run.invalidate();
        return false;
    }
    if let Err(error) = input::send_tracked_paste_ctrl_v(&mut enigo, &mut frame, true) {
        warn!("text refinement: replacement paste failed: {error}");
        let _ = frame.stroke_checked(|safe| {
            safe.then(|| enigo.key(Key::RightArrow, Direction::Click))
        });
        run.invalidate();
        return false;
    }
    drop(enigo);
    std::thread::sleep(TARGET_PASTE_SETTLE_DELAY);

    let after = capture_key();
    let unchanged = frame.unchanged(after.map(|key| key.physical_generation));
    finish_replacement(run, plan, expected_context, after, unchanged)
}

fn finish_replacement(
    run: &mut RunState,
    plan: ReplacementPlan,
    expected_context: ContextKey,
    after: Option<ContextKey>,
    own_input_unchanged: bool,
) -> bool {
    #[cfg(target_os = "windows")]
    {
        let Some(after) = after.filter(|after| {
            after.foreground == expected_context.foreground && after.focus == expected_context.focus
        }) else {
            run.invalidate();
            return false;
        };
        // Die Ersetzung wurde bereits ausgefuehrt. Nur nachgewiesene eigene
        // Ticks erlauben weitere Ersetzungen mit dem neuen Merkpunkt.
        run.commit(plan);
        if own_input_unchanged {
            run.advance_context(after);
        } else {
            run.invalidate();
        }
        true
    }
    #[cfg(not(target_os = "windows"))]
    {
        if !own_input_unchanged || after != Some(expected_context) {
            run.invalidate();
            return false;
        }
        run.commit(plan);
        true
    }
}

#[cfg(all(test, target_os = "windows"))]
mod replacement_tick_tests {
    use super::*;

    #[test]
    fn completed_own_injection_is_not_reported_as_failed_but_ambiguity_stops_further_edits() {
        let before = ContextKey { foreground: 1, focus: 2, physical_generation: u32::MAX };
        let mut run = RunState::new(1, true);
        run.record_append("Text.", Some(before), Some(before), true);
        run.register_sentence(1, 1, "Text.");
        let plan = run.plan_sentence(1, 1, "Text.", "Text!", before).unwrap();
        let after = ContextKey { physical_generation: 0, ..before };
        assert!(finish_replacement(&mut run, plan, before, Some(after), false));
        assert_eq!(run.rendered_text(), "Text!");
        assert!(run.prepare_final(1, Some(after)).is_none());
    }

    #[test]
    fn missing_context_or_changed_focus_is_still_a_failure() {
        let before = ContextKey { foreground: 1, focus: 2, physical_generation: 10 };
        for after in [None, Some(ContextKey { focus: 3, ..before })] {
            let mut run = RunState::new(1, true);
            run.record_append("Text.", Some(before), Some(before), true);
            run.register_sentence(1, 1, "Text.");
            let plan = run.plan_sentence(1, 1, "Text.", "Text!", before).unwrap();
            assert!(!finish_replacement(&mut run, plan, before, after, true));
            assert!(run.plan_sentence(1, 1, "Text.", "Text!", before).is_none());
        }
    }

    #[test]
    fn proven_own_replacement_advances_context_for_the_next_edit() {
        let before = ContextKey { foreground: 1, focus: 2, physical_generation: u32::MAX };
        let mut run = RunState::new(1, true);
        run.record_append("Text.", Some(before), Some(before), true);
        run.register_sentence(1, 1, "Text.");
        let plan = run.plan_sentence(1, 1, "Text.", "Text!", before).unwrap();
        let after = ContextKey { physical_generation: 0, ..before };
        assert!(finish_replacement(&mut run, plan, before, Some(after), true));
        let snapshot = run.prepare_final(1, Some(after)).unwrap();
        assert!(run.plan_final(1, &snapshot, "Mehr!", after).is_some());
    }
}

fn capture_key() -> Option<ContextKey> {
    input::capture_replacement_context().map(context_key)
}

fn context_key(context: ReplacementContext) -> ContextKey {
    ContextKey {
        foreground: context.foreground,
        focus: context.focus,
        physical_generation: context.physical_generation,
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::restores_clipboard;
    use crate::settings::ClipboardHandling;

    #[test]
    fn clipboard_is_restored_unless_the_user_asked_for_a_copy() {
        // Der Streaming-Pfad legte jedes Fragment in der Ablage ab und liess
        // es dort liegen -- auch bei "nicht veraendern".
        assert!(restores_clipboard(ClipboardHandling::DontModify));
        assert!(!restores_clipboard(ClipboardHandling::CopyToClipboard));
    }

    #[test]
    fn dont_modify_is_the_default() {
        assert!(restores_clipboard(ClipboardHandling::default()));
    }
}

#[cfg(test)]
mod guard_tests {
    use super::{
        append_step, AppendOutcome, ContextKey, HeldOutcome, HeldRemainder, InjectionCommand,
        InjectionHandle,
    };
    use crate::paste_guard::PasteFallback;
    use crate::paste_session::testing::{window, FakeDesktop};
    use crate::refinement::injection_state::RunState;
    use std::cell::RefCell;
    use std::sync::mpsc;

    fn no_context() -> Option<ContextKey> {
        None
    }

    /// Pastes recorded by the fake paste effect.
    struct Pastes(RefCell<Vec<String>>);

    impl Pastes {
        fn new() -> Self {
            Self(RefCell::new(Vec::new()))
        }
        fn record(&self, fragment: &str) -> bool {
            self.0.borrow_mut().push(fragment.to_string());
            true
        }
        fn all(&self) -> Vec<String> {
            self.0.borrow().clone()
        }
    }

    fn handle_with_receiver() -> (InjectionHandle, mpsc::Receiver<InjectionCommand>) {
        let (tx, rx) = mpsc::channel();
        (
            InjectionHandle {
                tx: std::sync::Arc::new(tx),
            },
            rx,
        )
    }

    #[test]
    fn a_vanished_worker_is_reported_as_lost_not_as_nothing_held() {
        let (handle, rx) = handle_with_receiver();
        drop(rx); // the worker thread is gone

        assert_eq!(handle.take_held(1), HeldOutcome::Lost);
    }

    #[test]
    fn a_stuck_worker_is_reported_as_lost_after_the_timeout() {
        let (handle, _rx) = handle_with_receiver(); // never answers

        assert_eq!(
            handle.take_held_within(1, std::time::Duration::from_millis(30)),
            HeldOutcome::Lost
        );
    }

    #[test]
    fn the_remainder_the_worker_hands_over_is_passed_on() {
        let (handle, rx) = handle_with_receiver();
        let worker = std::thread::spawn(move || match rx.recv().unwrap() {
            InjectionCommand::TakeHeld { run_id, reply } => {
                assert_eq!(run_id, 4);
                let _ = reply.send(Some(HeldRemainder {
                    text: " Rest.".into(),
                    reason: PasteFallback::FocusChanged,
                    target: None,
                }));
            }
            _ => panic!("expected TakeHeld"),
        });

        match handle.take_held(4) {
            HeldOutcome::Held(rest) => assert_eq!(rest.text, " Rest."),
            other => panic!("unexpected {other:?}"),
        }
        worker.join().unwrap();
    }

    #[test]
    fn nothing_held_is_reported_as_nothing() {
        let (handle, rx) = handle_with_receiver();
        let worker = std::thread::spawn(move || {
            if let InjectionCommand::TakeHeld { reply, .. } = rx.recv().unwrap() {
                let _ = reply.send(None);
            }
        });

        assert_eq!(handle.take_held(4), HeldOutcome::Nothing);
        worker.join().unwrap();
    }

    #[test]
    fn the_streaming_path_pastes_only_what_the_guard_admits() {
        let desk = FakeDesktop::focused(window(11, 42));
        let pastes = Pastes::new();
        // Refinement OFF: the configuration that had no check at all.
        let mut run = RunState::new(1, false);

        let first = append_step(&mut run, "Eins.", &desk, no_context, |f, _| pastes.record(f));
        assert_eq!(first, AppendOutcome::Inserted);

        desk.focus(Some(window(12, 43)));
        let second = append_step(&mut run, " Zwei.", &desk, no_context, |f, _| pastes.record(f));
        let third = append_step(&mut run, " Drei.", &desk, no_context, |f, _| pastes.record(f));

        assert_eq!(second, AppendOutcome::Buffered);
        assert_eq!(third, AppendOutcome::Buffered);
        assert_eq!(pastes.all(), vec!["Eins."], "nothing into the new window");
        let rest = run.take_held(1).expect("remainder");
        assert_eq!(rest.text, " Zwei. Drei.");
        assert_eq!(rest.reason, PasteFallback::FocusChanged);
    }

    #[test]
    fn an_elevated_target_gets_no_keystroke_at_all() {
        let desk = FakeDesktop::focused(window(11, 42));
        desk.set_elevated(42, true);
        let pastes = Pastes::new();
        let mut run = RunState::new(1, false);

        let outcome = append_step(&mut run, "Eins.", &desk, no_context, |f, _| pastes.record(f));

        assert_eq!(outcome, AppendOutcome::Buffered);
        assert!(pastes.all().is_empty());
        assert_eq!(
            run.take_held(1).expect("remainder").reason,
            PasteFallback::TargetElevated
        );
    }

    #[test]
    fn normal_case_pastes_every_fragment_in_order() {
        let desk = FakeDesktop::focused(window(11, 42));
        let pastes = Pastes::new();
        let mut run = RunState::new(1, false);

        for fragment in ["Eins.", " Zwei.", " Drei."] {
            let outcome = append_step(&mut run, fragment, &desk, no_context, |f, _| pastes.record(f));
            assert_eq!(outcome, AppendOutcome::Inserted);
        }

        assert_eq!(pastes.all(), vec!["Eins.", " Zwei.", " Drei."]);
        assert_eq!(run.take_held(1), None);
    }

    #[test]
    fn a_failed_paste_is_kept_and_nothing_is_typed_behind_the_gap() {
        let desk = FakeDesktop::focused(window(11, 42));
        let pastes = Pastes::new();
        let mut run = RunState::new(1, false);

        let failed = append_step(&mut run, "Eins.", &desk, no_context, |_, _| false);
        let later = append_step(&mut run, " Zwei.", &desk, no_context, |f, _| pastes.record(f));

        assert_eq!(failed, AppendOutcome::Inserted, "the attempt was made");
        assert_eq!(later, AppendOutcome::Buffered);
        assert!(pastes.all().is_empty());
        let rest = run.take_held(1).expect("remainder");
        assert_eq!(rest.text, "Eins. Zwei.");
        assert_eq!(rest.reason, PasteFallback::InjectionFailed);
    }

    #[test]
    fn focus_moving_while_a_fragment_is_in_flight_stops_the_following_ones() {
        let desk = FakeDesktop::focused(window(11, 42));
        let pastes = Pastes::new();
        let mut run = RunState::new(1, false);

        // The user switches window during the paste of the first fragment.
        let first = append_step(&mut run, "Eins.", &desk, no_context, |f, _| {
            desk.focus(Some(window(12, 43)));
            pastes.record(f)
        });
        let second = append_step(&mut run, " Zwei.", &desk, no_context, |f, _| pastes.record(f));

        assert_eq!(first, AppendOutcome::Inserted);
        assert_eq!(second, AppendOutcome::Buffered);
        assert_eq!(pastes.all(), vec!["Eins."]);
        let rest = run.take_held(1).expect("remainder");
        assert_eq!(rest.reason, PasteFallback::FocusChangedDuringPaste);
        assert!(!rest.untouched(), "never pasted a second time");
    }
}
