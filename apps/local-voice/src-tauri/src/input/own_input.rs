/// Injectable desktop tick source and clock; no keyboard or window dependency.
pub(super) trait InputClock {
    fn last_input(&mut self) -> Option<u32>;
    fn now(&mut self) -> u32;
}

pub(super) struct OwnInputFrame {
    marker: Option<u32>,
    safe: bool,
}

impl OwnInputFrame {
    pub(super) fn new(marker: Option<u32>) -> Self {
        Self { marker, safe: marker.is_some() }
    }

    pub(super) fn observe(&mut self, tick: Option<u32>) -> bool {
        self.safe &= tick.is_some() && tick == self.marker;
        self.safe
    }

    pub(super) fn stroke<T>(
        &mut self,
        clock: &mut impl InputClock,
        send: impl FnOnce(bool) -> T,
    ) -> T {
        self.observe(clock.last_input());
        let start = clock.now();
        // Foreign input disables replacement, never an in-flight dictation paste.
        let result = send(self.safe);
        let end = clock.now();
        match clock.last_input() {
            Some(tick) if Some(tick) == self.marker => {}
            // No tolerance beyond the stroke itself, even across u32 wrap.
            Some(tick) if tick.wrapping_sub(start) <= end.wrapping_sub(start) => {
                self.marker = Some(tick);
            }
            _ => self.safe = false,
        }
        // GetLastInputInfo has no origin: foreign input in the same tick as an
        // own stroke is indistinguishable. PTT key repeats count as foreign,
        // just as with the previous hook-based guard.
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Clock {
        ticks: VecDeque<Option<u32>>,
        times: VecDeque<u32>,
    }

    fn stroke(frame: &mut OwnInputFrame, before: Option<u32>, start: u32, end: u32, after: Option<u32>) {
        let mut clock = Clock { ticks: [before, after].into(), times: [start, end].into() };
        let mut sent = false;
        frame.stroke(&mut clock, |_| sent = true);
        assert!(sent, "foreign input must not interrupt dictation delivery");
        assert!(clock.ticks.is_empty() && clock.times.is_empty());
    }

    impl InputClock for Clock {
        fn last_input(&mut self) -> Option<u32> { self.ticks.pop_front().unwrap() }
        fn now(&mut self) -> u32 { self.times.pop_front().unwrap() }
    }

    #[test]
    fn consecutive_own_appends_keep_replacement_available() {
        use crate::refinement::injection_state::{ContextKey, RunState};
        // The user's dictation chord predates the initial marker.
        let mut frame = OwnInputFrame::new(Some(90));
        let mut run = RunState::new(1, true);
        let mut context = ContextKey { foreground: 1, focus: 2, physical_generation: 90 };
        for (start, fragment) in [(100, "Text."), (200, " Mehr."), (300, " Ende.")] {
            let before = frame.marker;
            stroke(&mut frame, before, start, start + 2, Some(start + 1));
            let after = ContextKey { physical_generation: start + 1, ..context };
            let own = frame.observe(Some(after.physical_generation));
            assert!(own);
            run.record_tracked_append(fragment, Some(context), Some(after), true, own);
            context = after;
        }
        run.register_sentence(1, 1, "Text.");
        assert!(run.plan_sentence(1, 1, "Text.", "Text!", context).is_some());
        let changed = ContextKey { physical_generation: 400, ..context };
        assert!(!frame.observe(Some(changed.physical_generation)));
        assert!(run.plan_sentence(1, 1, "Text.", "Text!", changed).is_none());
    }

    #[test]
    fn foreign_input_before_first_or_between_strokes_is_sticky() {
        for between in [false, true] {
            let mut frame = OwnInputFrame::new(Some(90));
            if between {
                stroke(&mut frame, Some(90), 100, 102, Some(101));
            }
            stroke(&mut frame, Some(150), 200, 202, Some(201));
            stroke(&mut frame, Some(201), 300, 302, Some(301));
            assert!(!frame.observe(Some(301)));
        }
    }

    #[test]
    fn foreign_input_after_last_stroke_or_after_its_end_is_rejected() {
        let mut frame = OwnInputFrame::new(Some(90));
        stroke(&mut frame, Some(90), 100, 102, Some(101));
        assert!(!frame.observe(Some(103)));
        let mut frame = OwnInputFrame::new(Some(90));
        stroke(&mut frame, Some(90), 100, 102, Some(103));
        assert!(!frame.observe(Some(103)));
    }

    #[test]
    fn wrap_is_allowed_only_inside_the_own_stroke() {
        let mut frame = OwnInputFrame::new(Some(u32::MAX - 10));
        stroke(&mut frame, Some(u32::MAX - 10), u32::MAX - 1, 2, Some(0));
        assert!(frame.observe(Some(0)));
        stroke(&mut frame, Some(0), 3, 4, Some(u32::MAX));
        assert!(!frame.observe(Some(u32::MAX)));
    }

    #[test]
    fn same_clock_tick_and_both_stroke_boundaries_are_accepted() {
        let mut frame = OwnInputFrame::new(Some(90));
        stroke(&mut frame, Some(90), 100, 100, Some(100));
        assert!(frame.observe(Some(100)));
        stroke(&mut frame, Some(100), 200, 202, Some(200));
        assert!(frame.observe(Some(200)));
        stroke(&mut frame, Some(200), 300, 302, Some(302));
        assert!(frame.observe(Some(302)));
    }

    #[test]
    fn last_input_failure_at_any_boundary_blocks_replacement() {
        let mut frame = OwnInputFrame::new(None);
        stroke(&mut frame, Some(90), 100, 102, Some(101));
        assert!(!frame.observe(Some(101)));
        for (before, after) in [(None, Some(101)), (Some(90), None)] {
            let mut frame = OwnInputFrame::new(Some(90));
            stroke(&mut frame, before, 100, 102, after);
            assert!(!frame.observe(Some(101)));
        }
        let mut frame = OwnInputFrame::new(Some(90));
        assert!(!frame.observe(None));
    }

    #[test]
    fn replacement_checks_ownership_before_the_effect() {
        let mut frame = OwnInputFrame::new(Some(90));
        let mut clock = Clock { ticks: [Some(91), Some(91)].into(), times: [100, 102].into() };
        let mut replaced = false;
        frame.stroke(&mut clock, |safe| {
            if safe { replaced = true; }
        });
        assert!(!replaced);
        assert!(!frame.observe(Some(91)));
    }
}
