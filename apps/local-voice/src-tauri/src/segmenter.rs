//! Sentence-by-sentence transcription and injection.
//!
//! Instead of waiting for the whole dictation to finish and pasting one block, this
//! emits each spoken sentence as soon as the speaker pauses. Text appears while you
//! are still talking.
//!
//! # Why pause detection is nearly free here
//!
//! The recorder's audio callback fires *after* the Silero VAD has been applied, so
//! it only ever receives speech frames — silence is already filtered out upstream.
//! That means we do not need our own voice detector: a gap between two callbacks
//! simply *is* a pause. If no frame has arrived for `pause_ms`, the speaker stopped.
//!
//! # Why sentence-wise rather than true token streaming
//!
//! A finished sentence is stable. Token-level streaming with a batch model requires
//! re-transcribing a growing buffer and then retro-actively correcting text that has
//! already been inserted into someone's document, which is where such implementations
//! turn ugly. Here each segment is transcribed once, inserted once, and never touched
//! again.
//!
//! # The trade-off, stated plainly
//!
//! Each segment gives the model less context, and both Whisper and Parakeet use
//! context for punctuation and capitalisation. Segment mode therefore produces
//! slightly weaker punctuation than one whole-recording pass. That is why it is a
//! setting rather than a replacement.
//!
//! # Failure behaviour (docs/DECISIONS.md D14)
//!
//! This mode inserts several times per run, so the batch contract of "one paste
//! attempt, then one notice" does not apply. Every segment instead goes through
//! the fail-closed checks of `paste_guard` via [`PasteSession`]: the window that
//! has the focus at the first insertion is the run's target, and the first
//! deviation (focus moved, elevated target, failed paste) blocks the run. From
//! then on segments are transcribed and kept for the history as usual, but
//! BUFFERED instead of inserted; [`SentenceSegmenter::finish`] hands the rest
//! over once, and the caller shows ONE notice with it.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use log::{debug, warn};
use tauri::AppHandle;

use crate::clipboard::GuardedPasteOutcome;
use crate::managers::transcription::TranscriptionManager;
use crate::paste_guard::PasteTarget;
use crate::paste_session::{Gate, HeldRemainder, PasteSession, Surroundings, SystemSurroundings};

/// 16 kHz mono is what the recorder hands us and what the engines expect.
const SAMPLE_RATE: usize = 16_000;

/// Don't emit a segment shorter than this. Guards against a stray cough or a
/// clipped syllable becoming its own "sentence".
const MIN_SEGMENT_MS: usize = 700;

/// How often the watchdog looks for a pause. Well below the pause threshold, so
/// the detection granularity is not what the user perceives.
const TICK: Duration = Duration::from_millis(100);

struct Shared {
    /// Speech frames accumulated since the last emitted segment.
    buffer: Mutex<Vec<f32>>,
    /// When the most recent speech frame arrived. `None` before the first frame.
    last_frame_at: Mutex<Option<Instant>>,
    /// Everything emitted so far this run, for the history entry.
    emitted_text: Mutex<Vec<String>>,
    /// Set while a dictation run is in progress.
    running: AtomicBool,
    /// Number of segments emitted this run; lets the caller tell whether segment
    /// mode actually produced anything.
    emitted_count: AtomicUsize,
    /// Serialises emission (take audio -> transcribe -> insert) and holds the
    /// run's insertion guard. The watchdog and `finish` both emit, and without
    /// this lock `finish` could return before a segment that is still being
    /// transcribed had been inserted or buffered, so its remainder would be
    /// incomplete and the segments could land out of order. Never taken on the
    /// audio path: `feed` only touches `buffer` and `last_frame_at`.
    delivery: Mutex<PasteSession>,
    /// Set by `begin`/`cancel`: the next holder of `delivery` starts from a
    /// fresh session. A flag instead of locking in those calls, because a
    /// cancelled run may still be transcribing a segment and the hotkey path
    /// must not wait for it.
    reset_session: AtomicBool,
    /// Bumped by `cancel`. A segment that was already being transcribed when
    /// the dictation was cancelled must be discarded instead of inserted (the
    /// transcription cannot be interrupted, so the check happens after it).
    epoch: AtomicU64,
}

/// What the run produced, for the history entry and the end-of-run notice.
pub(crate) struct FinishedSegments {
    /// Full text of every segment, inserted or buffered.
    pub(crate) text: String,
    /// Segments that were NOT inserted because the guard refused. The caller
    /// reports them once.
    pub(crate) remainder: Option<HeldRemainder>,
}

/// Where a guard-approved segment goes. Production inserts through the
/// fail-closed `paste_transcript_guarded`; tests record the calls.
pub(crate) trait SegmentSink {
    fn paste(&self, text: String, target: Option<PasteTarget>) -> GuardedPasteOutcome;
}

/// The real insertion: the same guarded paste the batch path uses, aimed at the
/// run's target window.
struct GuardedSink {
    app: AppHandle,
}

impl SegmentSink for GuardedSink {
    fn paste(&self, text: String, target: Option<PasteTarget>) -> GuardedPasteOutcome {
        // The last segment is emitted at the stop press, while the hotkey
        // chord is usually still down; typed into a Chromium-based target it
        // would be swallowed as a shortcut. Only methods that send keystrokes
        // need the wait (as in the legacy path this replaces).
        use crate::settings::{get_settings, PasteMethod};
        let method = get_settings(&self.app).paste_method;
        if !matches!(method, PasteMethod::None | PasteMethod::ExternalScript) {
            let waited =
                crate::input::wait_for_modifiers_released(crate::input::MODIFIER_RELEASE_TIMEOUT);
            if waited > Duration::from_millis(20) {
                debug!("segmenter: waited {waited:?} for modifier keys to be released");
            }
        }
        crate::clipboard::paste_transcript_guarded(text, self.app.clone(), target)
    }
}

/// Emits transcribed sentences as the speaker pauses.
#[derive(Clone)]
pub struct SentenceSegmenter {
    shared: Arc<Shared>,
    pause: Duration,
}

impl SentenceSegmenter {
    pub fn new(pause_ms: u64) -> Self {
        Self {
            shared: Arc::new(Shared {
                buffer: Mutex::new(Vec::new()),
                last_frame_at: Mutex::new(None),
                emitted_text: Mutex::new(Vec::new()),
                running: AtomicBool::new(false),
                emitted_count: AtomicUsize::new(0),
                delivery: Mutex::new(PasteSession::new()),
                reset_session: AtomicBool::new(false),
                epoch: AtomicU64::new(0),
            }),
            pause: Duration::from_millis(pause_ms),
        }
    }

    /// True while a run is active. Cheap enough to call from the audio callback.
    #[inline]
    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire)
    }

    pub fn segments_emitted(&self) -> usize {
        self.shared.emitted_count.load(Ordering::Acquire)
    }

    /// Feed one post-VAD speech frame. Called on the recorder's consumer thread, so
    /// it does nothing but append and stamp the clock.
    pub fn feed(&self, frame: &[f32]) {
        if !self.is_running() {
            return;
        }
        self.shared.buffer.lock().unwrap().extend_from_slice(frame);
        *self.shared.last_frame_at.lock().unwrap() = Some(Instant::now());
    }

    /// Begin a run and spawn the pause watchdog.
    pub fn start(&self, app: AppHandle, tm: Arc<TranscriptionManager>) {
        if !self.begin() {
            return;
        }

        let shared = Arc::clone(&self.shared);
        let pause = self.pause;
        let me = self.clone();

        std::thread::spawn(move || {
            debug!("segmenter: watchdog started (pause {:?})", pause);
            let sink = GuardedSink { app };
            let transcribe = |audio: Vec<f32>| tm.transcribe(audio).map_err(|e| e.to_string());
            while shared.running.load(Ordering::Acquire) {
                std::thread::sleep(TICK);
                if !shared.running.load(Ordering::Acquire) {
                    break;
                }
                me.tick(&transcribe, &sink, &SystemSurroundings);
            }
            debug!("segmenter: watchdog stopped");
        });
    }

    /// Reset the run state and mark the run active. `false` when a run is
    /// already active (the caller must not start a second watchdog).
    fn begin(&self) -> bool {
        if self.shared.running.swap(true, Ordering::AcqRel) {
            warn!("segmenter: start called while already running");
            return false;
        }
        self.shared.buffer.lock().unwrap().clear();
        self.shared.emitted_text.lock().unwrap().clear();
        self.shared.emitted_count.store(0, Ordering::Release);
        *self.shared.last_frame_at.lock().unwrap() = None;
        self.shared.reset_session.store(true, Ordering::Release);
        true
    }

    /// The emission lock, with a fresh session if a new run began (or the
    /// previous one was cancelled) since the last holder. A poisoned lock is
    /// taken over: the session only holds text to report, and refusing to
    /// continue would turn one panic into lost dictation.
    fn lock_delivery(&self) -> MutexGuard<'_, PasteSession> {
        let mut session = self
            .shared
            .delivery
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.shared.reset_session.swap(false, Ordering::AcqRel) {
            *session = PasteSession::new();
        }
        session
    }

    /// One watchdog step: emit the buffered segment if the speaker paused.
    fn tick<T, S, E>(&self, transcribe: &T, sink: &S, env: &E)
    where
        T: Fn(Vec<f32>) -> Result<String, String>,
        S: SegmentSink,
        E: Surroundings,
    {
        let mut session = self.lock_delivery();
        // `finish` may have ended the run while this step waited for the lock;
        // its tail is then already handled and nothing may be emitted after it.
        if !self.is_running() {
            return;
        }
        if let Some(segment) = self.take_if_paused() {
            let epoch = self.shared.epoch.load(Ordering::Acquire);
            self.process(
                &mut session,
                epoch,
                segment,
                "segment",
                transcribe,
                sink,
                env,
            );
        }
    }

    /// Take the buffered audio if the speaker has paused long enough and the
    /// segment is worth transcribing.
    fn take_if_paused(&self) -> Option<Vec<f32>> {
        let last = (*self.shared.last_frame_at.lock().unwrap())?;
        if last.elapsed() < self.pause {
            return None;
        }
        let mut buf = self.shared.buffer.lock().unwrap();
        if buf.len() < MIN_SEGMENT_MS * SAMPLE_RATE / 1000 {
            return None;
        }
        Some(std::mem::take(&mut *buf))
    }

    #[allow(clippy::too_many_arguments)]
    fn process<T, S, E>(
        &self,
        session: &mut PasteSession,
        epoch: u64,
        segment: Vec<f32>,
        kind: &str,
        transcribe: &T,
        sink: &S,
        env: &E,
    ) where
        T: Fn(Vec<f32>) -> Result<String, String>,
        S: SegmentSink,
        E: Surroundings,
    {
        let seconds = segment.len() as f32 / SAMPLE_RATE as f32;
        let started = Instant::now();

        let text = match transcribe(segment) {
            Ok(t) => t,
            Err(e) => {
                // A failed segment must not abort the dictation: the speaker is
                // very likely still talking, and the remaining audio is unaffected.
                warn!("segmenter: {kind} transcription failed: {e}");
                return;
            }
        };

        // Cancelled while this segment was being transcribed: the dictation is
        // gone, so its text must not appear anywhere.
        if self.shared.epoch.load(Ordering::Acquire) != epoch {
            debug!("segmenter: {kind} discarded, the dictation was cancelled meanwhile");
            return;
        }

        let text = text.trim().to_string();
        if text.is_empty() {
            debug!("segmenter: {kind} produced no text ({seconds:.2}s)");
            return;
        }

        // Segment text is spoken content; production builds log only its length.
        #[cfg(debug_assertions)]
        debug!(
            "segmenter: {kind} {seconds:.2}s -> {:?} in {:?}",
            text,
            started.elapsed()
        );
        #[cfg(not(debug_assertions))]
        debug!(
            "segmenter: {kind} {seconds:.2}s -> {} chars in {:?}",
            text.chars().count(),
            started.elapsed()
        );

        // Separate sentences with a space so the target field reads naturally when
        // several segments land one after another.
        let to_paste = if self.shared.emitted_count.load(Ordering::Acquire) > 0 {
            format!(" {text}")
        } else {
            text.clone()
        };

        self.deliver(session, to_paste, kind, sink, env);

        // Keep the text in the run transcript whether it was inserted or
        // buffered, so the history entry stays complete and nothing the user
        // said is lost.
        self.shared.emitted_text.lock().unwrap().push(text);
        self.shared.emitted_count.fetch_add(1, Ordering::AcqRel);
    }

    /// Insert one segment, or buffer it when the run's guard refuses.
    ///
    /// The guard runs twice on purpose: [`PasteSession::gate`] is the pure
    /// decision (and keeps the run-wide target), `paste_transcript_guarded`
    /// re-checks right before the keystroke and after it, which also covers
    /// the few milliseconds in between.
    fn deliver<S, E>(
        &self,
        session: &mut PasteSession,
        to_paste: String,
        kind: &str,
        sink: &S,
        env: &E,
    ) where
        S: SegmentSink,
        E: Surroundings,
    {
        let target = match session.gate(&to_paste, env) {
            Gate::Deliver { target } => target,
            Gate::Held(reason) => {
                // Reasons are enum names, never transcript content.
                warn!("segmenter: {kind} buffered instead of inserted ({reason:?})");
                return;
            }
        };
        match sink.paste(to_paste.clone(), target) {
            GuardedPasteOutcome::Pasted | GuardedPasteOutcome::NothingToDo => {}
            GuardedPasteOutcome::Fallback(reason) => {
                // Not verifiably inserted: keep it, and stop inserting behind
                // the gap. The caller reports the buffered rest once.
                warn!("segmenter: {kind} not inserted ({reason:?}); buffering the rest");
                session.not_delivered(&to_paste, reason);
            }
        }
    }

    /// End the run, flush whatever is still buffered, and return the full text of
    /// everything emitted (for the history entry) plus the segments the guard
    /// kept back.
    pub(crate) fn finish(
        &self,
        app: &AppHandle,
        tm: &Arc<TranscriptionManager>,
    ) -> FinishedSegments {
        let sink = GuardedSink { app: app.clone() };
        let transcribe = |audio: Vec<f32>| tm.transcribe(audio).map_err(|e| e.to_string());
        self.finish_with(&transcribe, &sink, &SystemSurroundings)
    }

    fn finish_with<T, S, E>(&self, transcribe: &T, sink: &S, env: &E) -> FinishedSegments
    where
        T: Fn(Vec<f32>) -> Result<String, String>,
        S: SegmentSink,
        E: Surroundings,
    {
        if !self.shared.running.swap(false, Ordering::AcqRel) {
            return FinishedSegments {
                text: String::new(),
                remainder: None,
            };
        }
        // Wait for a segment the watchdog is still transcribing or inserting:
        // it must land (or be buffered) BEFORE the tail, and before the
        // remainder is read.
        let mut session = self.lock_delivery();
        let tail = std::mem::take(&mut *self.shared.buffer.lock().unwrap());
        if tail.len() >= MIN_SEGMENT_MS * SAMPLE_RATE / 1000 {
            let epoch = self.shared.epoch.load(Ordering::Acquire);
            self.process(&mut session, epoch, tail, "tail", transcribe, sink, env);
        } else if !tail.is_empty() {
            debug!(
                "segmenter: dropping {} trailing samples below minimum",
                tail.len()
            );
        }
        FinishedSegments {
            text: self.shared.emitted_text.lock().unwrap().join(" "),
            remainder: session.take_remainder(),
        }
    }

    /// Abort the run and discard everything. Nothing is transcribed or pasted.
    pub fn cancel(&self) {
        self.shared.running.store(false, Ordering::Release);
        self.shared.buffer.lock().unwrap().clear();
        self.shared.emitted_text.lock().unwrap().clear();
        self.shared.emitted_count.store(0, Ordering::Release);
        // Buffered-but-uninserted segments belong to the cancelled dictation
        // and go with it, just as the segments that were already inserted
        // are not taken back.
        self.shared.reset_session.store(true, Ordering::Release);
        self.shared.epoch.fetch_add(1, Ordering::AcqRel);
        debug!("segmenter: cancelled, buffer discarded");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ms: usize) -> Vec<f32> {
        vec![0.0; ms * SAMPLE_RATE / 1000]
    }

    #[test]
    fn ignores_frames_when_not_running() {
        let s = SentenceSegmenter::new(800);
        s.feed(&frame(1000));
        assert!(s.shared.buffer.lock().unwrap().is_empty());
    }

    #[test]
    fn does_not_emit_before_the_pause_elapses() {
        let s = SentenceSegmenter::new(800);
        s.shared.running.store(true, Ordering::Release);
        s.feed(&frame(1000));
        // The pause has not elapsed, so nothing may be taken yet.
        assert!(s.take_if_paused().is_none());
    }

    #[test]
    fn emits_after_the_pause() {
        let s = SentenceSegmenter::new(80); // short pause keeps the test fast
        s.shared.running.store(true, Ordering::Release);
        s.feed(&frame(1000));
        std::thread::sleep(Duration::from_millis(140));
        let seg = s.take_if_paused().expect("segment should be available");
        assert_eq!(seg.len(), SAMPLE_RATE); // one second of audio
                                            // Buffer is drained, so the same audio cannot be emitted twice.
        assert!(s.shared.buffer.lock().unwrap().is_empty());
        assert!(s.take_if_paused().is_none());
    }

    #[test]
    fn drops_segments_below_the_minimum() {
        let s = SentenceSegmenter::new(80);
        s.shared.running.store(true, Ordering::Release);
        s.feed(&frame(200)); // well under MIN_SEGMENT_MS
        std::thread::sleep(Duration::from_millis(140));
        assert!(
            s.take_if_paused().is_none(),
            "a cough must not become a sentence"
        );
    }

    #[test]
    fn cancel_discards_everything() {
        let s = SentenceSegmenter::new(80);
        s.shared.running.store(true, Ordering::Release);
        s.feed(&frame(1000));
        s.cancel();
        assert!(!s.is_running());
        assert!(s.shared.buffer.lock().unwrap().is_empty());
        assert_eq!(s.segments_emitted(), 0);
    }
}

/// Issue #3: segment mode called the unguarded `clipboard::paste` for every
/// sentence.
#[cfg(test)]
mod guard_tests {
    use super::*;
    use crate::paste_guard::PasteFallback;
    use crate::paste_session::testing::{window, FakeDesktop};
    use std::collections::VecDeque;
    use std::sync::mpsc;

    /// Records every insertion; answers from a script (default: `Pasted`).
    struct FakeSink {
        calls: Mutex<Vec<(String, Option<PasteTarget>)>>,
        script: Mutex<VecDeque<GuardedPasteOutcome>>,
    }

    impl FakeSink {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                script: Mutex::new(VecDeque::new()),
            }
        }

        fn answering(outcomes: Vec<GuardedPasteOutcome>) -> Self {
            let sink = Self::new();
            *sink.script.lock().unwrap() = outcomes.into();
            sink
        }

        fn texts(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(text, _)| text.clone())
                .collect()
        }
    }

    impl SegmentSink for FakeSink {
        fn paste(&self, text: String, target: Option<PasteTarget>) -> GuardedPasteOutcome {
            self.calls.lock().unwrap().push((text, target));
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(GuardedPasteOutcome::Pasted)
        }
    }

    fn audio() -> Vec<f32> {
        vec![0.0; SAMPLE_RATE]
    }

    fn says(text: &'static str) -> impl Fn(Vec<f32>) -> Result<String, String> {
        move |_| Ok(text.to_string())
    }

    /// One segment through the real emission path (no pause timing involved).
    fn emit(seg: &SentenceSegmenter, text: &'static str, sink: &FakeSink, desk: &FakeDesktop) {
        let mut session = seg.lock_delivery();
        let epoch = seg.shared.epoch.load(Ordering::Acquire);
        seg.process(
            &mut session,
            epoch,
            audio(),
            "segment",
            &says(text),
            sink,
            desk,
        );
    }

    fn running() -> SentenceSegmenter {
        let seg = SentenceSegmenter::new(50);
        assert!(seg.begin());
        seg
    }

    #[test]
    fn segments_are_inserted_through_the_guarded_sink_into_the_run_target() {
        let seg = running();
        let desk = FakeDesktop::focused(window(11, 42));
        let sink = FakeSink::new();

        emit(&seg, "Eins.", &sink, &desk);

        let calls = sink.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "Eins.");
        assert_eq!(calls[0].1, Some(window(11, 42)), "guard gets the target");
    }

    #[test]
    fn the_legacy_unguarded_paste_is_not_reachable_from_this_file() {
        // Tripwire for the regression of issue #3. The needle is assembled so
        // that this test does not match itself.
        let legacy_call = concat!("clipboard::", "paste(");
        let source = include_str!("segmenter.rs");
        assert!(
            !source.contains(legacy_call),
            "segment mode must insert through paste_transcript_guarded"
        );
    }

    #[test]
    fn normal_case_inserts_every_segment_and_reports_nothing() {
        let seg = running();
        let desk = FakeDesktop::focused(window(11, 42));
        let sink = FakeSink::new();

        emit(&seg, "Eins.", &sink, &desk);
        emit(&seg, "Zwei.", &sink, &desk);

        assert_eq!(sink.texts(), vec!["Eins.", " Zwei."]);
        let done = seg.finish_with(&says("unused"), &sink, &desk);
        assert_eq!(done.text, "Eins. Zwei.");
        assert!(done.remainder.is_none());
    }

    #[test]
    fn focus_change_buffers_the_rest_and_finish_reports_it_once() {
        let seg = running();
        let desk = FakeDesktop::focused(window(11, 42));
        let sink = FakeSink::new();

        emit(&seg, "Eins.", &sink, &desk);
        desk.focus(Some(window(12, 43)));
        emit(&seg, "Zwei.", &sink, &desk);
        emit(&seg, "Drei.", &sink, &desk);

        assert_eq!(
            sink.texts(),
            vec!["Eins."],
            "nothing may be typed into the foreign window"
        );
        let done = seg.finish_with(&says("unused"), &sink, &desk);
        assert_eq!(done.text, "Eins. Zwei. Drei.", "history stays complete");
        let rest = done.remainder.expect("remainder");
        assert_eq!(rest.text, " Zwei. Drei.");
        assert_eq!(rest.reason, PasteFallback::FocusChanged);
        assert_eq!(rest.target, Some(window(11, 42)));
    }

    #[test]
    fn the_tail_segment_is_buffered_too_when_focus_moved() {
        let seg = running();
        let desk = FakeDesktop::focused(window(11, 42));
        let sink = FakeSink::new();
        emit(&seg, "Eins.", &sink, &desk);

        desk.focus(Some(window(12, 43)));
        seg.feed(&audio());
        let done = seg.finish_with(&says("Schluss."), &sink, &desk);

        assert_eq!(sink.texts(), vec!["Eins."]);
        assert_eq!(done.remainder.expect("remainder").text, " Schluss.");
    }

    #[test]
    fn elevated_target_receives_no_segment() {
        let seg = running();
        let desk = FakeDesktop::focused(window(11, 42));
        desk.set_elevated(42, true);
        let sink = FakeSink::new();

        emit(&seg, "Eins.", &sink, &desk);
        emit(&seg, "Zwei.", &sink, &desk);

        assert!(
            sink.texts().is_empty(),
            "fail-closed from the first segment"
        );
        let rest = seg
            .finish_with(&says("unused"), &sink, &desk)
            .remainder
            .expect("remainder");
        assert_eq!(rest.text, "Eins. Zwei.");
        assert_eq!(rest.reason, PasteFallback::TargetElevated);
        assert!(rest.untouched());
    }

    #[test]
    fn a_guard_fallback_inside_the_paste_blocks_the_following_segments() {
        let seg = running();
        let desk = FakeDesktop::focused(window(11, 42));
        let sink = FakeSink::answering(vec![
            GuardedPasteOutcome::Pasted,
            GuardedPasteOutcome::Fallback(PasteFallback::FocusChangedDuringPaste),
        ]);

        emit(&seg, "Eins.", &sink, &desk);
        emit(&seg, "Zwei.", &sink, &desk);
        emit(&seg, "Drei.", &sink, &desk);

        assert_eq!(sink.texts(), vec!["Eins.", " Zwei."], "no third attempt");
        let rest = seg
            .finish_with(&says("unused"), &sink, &desk)
            .remainder
            .expect("remainder");
        assert_eq!(rest.text, " Zwei. Drei.");
        assert_eq!(rest.reason, PasteFallback::FocusChangedDuringPaste);
        assert!(!rest.untouched());
    }

    #[test]
    fn where_nothing_can_be_observed_segments_are_inserted_as_before() {
        let seg = running();
        let mut desk = FakeDesktop::focused(window(11, 42));
        desk.observable = false;
        desk.focus(None);
        let sink = FakeSink::new();

        emit(&seg, "Eins.", &sink, &desk);
        emit(&seg, "Zwei.", &sink, &desk);

        let calls = sink.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|(_, target)| target.is_none()));
        drop(calls);
        assert!(seg
            .finish_with(&says("x"), &sink, &desk)
            .remainder
            .is_none());
    }

    #[test]
    fn a_new_run_starts_with_a_clean_guard_after_cancel() {
        let seg = running();
        let desk = FakeDesktop::focused(window(11, 42));
        let sink = FakeSink::new();
        emit(&seg, "Eins.", &sink, &desk);
        desk.focus(Some(window(12, 43)));
        emit(&seg, "Zwei.", &sink, &desk);
        seg.cancel();
        assert_eq!(seg.segments_emitted(), 0);

        // Next dictation, now in the other window: it is a fresh target, and
        // the cancelled run's buffered text must not resurface.
        assert!(seg.begin());
        let fresh = FakeSink::new();
        emit(&seg, "Neu.", &fresh, &desk);
        assert_eq!(fresh.texts(), vec!["Neu."]);
        assert_eq!(fresh.calls.lock().unwrap()[0].1, Some(window(12, 43)));
        assert!(seg
            .finish_with(&says("x"), &fresh, &desk)
            .remainder
            .is_none());
    }

    #[test]
    fn a_segment_being_transcribed_when_the_dictation_is_cancelled_is_discarded() {
        let seg = running();
        let desk = FakeDesktop::focused(window(11, 42));
        let sink = FakeSink::new();

        {
            let mut session = seg.lock_delivery();
            let epoch = seg.shared.epoch.load(Ordering::Acquire);
            // Esc arrives while the engine is still working on this segment.
            let cancelling = |_audio: Vec<f32>| -> Result<String, String> {
                seg.cancel();
                Ok("zu spaet".to_string())
            };
            seg.process(
                &mut session,
                epoch,
                audio(),
                "segment",
                &cancelling,
                &sink,
                &desk,
            );
        }

        assert!(
            sink.texts().is_empty(),
            "no text from a cancelled dictation"
        );
        assert_eq!(seg.segments_emitted(), 0);
        assert!(seg.shared.emitted_text.lock().unwrap().is_empty());
    }

    #[test]
    fn finish_waits_for_the_segment_in_flight_and_keeps_the_order() {
        // The watchdog is inside the insertion of "eins" when the stop press
        // arrives. finish() must neither return early (incomplete remainder)
        // nor overtake it with the tail.
        struct BlockingSink {
            entered: Mutex<mpsc::Sender<()>>,
            release: Mutex<mpsc::Receiver<()>>,
            texts: Mutex<Vec<String>>,
        }
        impl SegmentSink for BlockingSink {
            fn paste(&self, text: String, _target: Option<PasteTarget>) -> GuardedPasteOutcome {
                let first = self.texts.lock().unwrap().is_empty();
                self.texts.lock().unwrap().push(text);
                if first {
                    self.entered.lock().unwrap().send(()).unwrap();
                    self.release.lock().unwrap().recv().unwrap();
                }
                GuardedPasteOutcome::Pasted
            }
        }

        let seg = SentenceSegmenter::new(20);
        assert!(seg.begin());
        seg.feed(&audio());
        std::thread::sleep(Duration::from_millis(60));

        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let sink = Arc::new(BlockingSink {
            entered: Mutex::new(entered_tx),
            release: Mutex::new(release_rx),
            texts: Mutex::new(Vec::new()),
        });
        let spoken = Arc::new(Mutex::new(VecDeque::from(["eins", "zwei"])));
        let transcribe = {
            let spoken = Arc::clone(&spoken);
            move |_audio: Vec<f32>| -> Result<String, String> {
                Ok(spoken.lock().unwrap().pop_front().unwrap().to_string())
            }
        };
        let transcribe = Arc::new(transcribe);

        let watchdog = {
            let (seg, sink, transcribe) = (seg.clone(), Arc::clone(&sink), Arc::clone(&transcribe));
            std::thread::spawn(move || {
                let desk = FakeDesktop::focused(window(11, 42));
                seg.tick(&*transcribe, &*sink, &desk);
            })
        };
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("watchdog is inserting");

        seg.feed(&audio()); // the tail the user is still speaking
        let (done_tx, done_rx) = mpsc::channel();
        let stopper = {
            let (seg, sink, transcribe) = (seg.clone(), Arc::clone(&sink), Arc::clone(&transcribe));
            std::thread::spawn(move || {
                let desk = FakeDesktop::focused(window(11, 42));
                done_tx
                    .send(seg.finish_with(&*transcribe, &*sink, &desk))
                    .unwrap();
            })
        };

        assert!(
            done_rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "finish must wait for the segment in flight"
        );
        release_tx.send(()).unwrap();
        let done = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("finish returns");
        watchdog.join().unwrap();
        stopper.join().unwrap();

        assert_eq!(done.text, "eins zwei");
        assert_eq!(*sink.texts.lock().unwrap(), vec!["eins", " zwei"]);
        assert!(done.remainder.is_none());
    }
}
