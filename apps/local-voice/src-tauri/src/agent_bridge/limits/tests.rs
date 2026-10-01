use super::*;

#[test]
fn calls_up_to_the_limit_pass_and_the_next_is_limited() {
    let mut l = RateLimiter::new();
    for i in 0..3 {
        assert_eq!(l.hit("c1", 3, 1_000 + i), Limit::Allowed);
    }
    assert_eq!(l.hit("c1", 3, 1_010), Limit::Limited { first: true });
}

#[test]
fn only_the_first_refusal_per_window_is_reported() {
    let mut l = RateLimiter::new();
    for _ in 0..2 {
        l.hit("c1", 2, 1_000);
    }
    assert_eq!(l.hit("c1", 2, 1_100), Limit::Limited { first: true });
    assert_eq!(l.hit("c1", 2, 1_200), Limit::Limited { first: false });
    assert_eq!(l.hit("c1", 2, 1_300), Limit::Limited { first: false });
}

#[test]
fn the_window_slides() {
    let mut l = RateLimiter::new();
    assert_eq!(l.hit("c1", 1, 0), Limit::Allowed);
    assert_eq!(l.hit("c1", 1, 30_000), Limit::Limited { first: true });
    // Nach einer Minute ist der erste Aufruf aus dem Fenster.
    assert_eq!(l.hit("c1", 1, WINDOW_MS), Limit::Allowed);
    // Eine Abweisung kurz danach gehoert noch zur gemeldeten (ab 30 s): still.
    assert_eq!(l.hit("c1", 1, WINDOW_MS + 10), Limit::Limited { first: false });
    // Eine Minute nach der letzten Meldung wird wieder gemeldet.
    assert_eq!(l.hit("c1", 1, 30_000 + WINDOW_MS + 10), Limit::Limited { first: true });
}

#[test]
fn clients_are_counted_separately() {
    let mut l = RateLimiter::new();
    assert_eq!(l.hit("a", 1, 0), Limit::Allowed);
    assert_eq!(l.hit("a", 1, 1), Limit::Limited { first: true });
    assert_eq!(l.hit("b", 1, 2), Limit::Allowed, "ein anderer Zugang ist nicht betroffen");
}

#[test]
fn a_limit_of_zero_refuses_everything() {
    let mut l = RateLimiter::new();
    assert_eq!(l.hit("c1", 0, 0), Limit::Limited { first: true });
}

#[test]
fn the_number_of_tracked_keys_is_bounded() {
    let mut l = RateLimiter::new();
    for i in 0..(MAX_KEYS * 3) {
        l.hit(&format!("client-{i}"), 5, i as i64);
    }
    assert!(l.keys() <= MAX_KEYS, "{}", l.keys());
}
