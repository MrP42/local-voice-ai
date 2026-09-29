//! Erinnerung vor Terminen und Zuordnung „laeuft gerade“ (M5, P5b;
//! `entwurf/m5-m6-kalender-export.md` §3 „Erinnerung 1 min vorher“, „Start aus
//! Termin“). Rein: keine Uhr, keine Datenbank, kein Fenster. Der Dienst
//! (`service.rs`) fuettert die Funktionen mit dem Cache und der aktuellen Zeit,
//! die Tests mit festen Uhrzeiten.
//!
//! Regeln fuer `due_reminders`:
//! - faellig, wenn `start - lead <= jetzt < start + 2 min` (`CATCH_UP_MS`): wer
//!   den Rechner aus dem Ruhezustand holt, bekommt keine Erinnerung an einen
//!   Termin, der laengst begonnen hat;
//! - nie ganztaegig, nie abgesagt, nie schon erinnert oder verworfen, nie waehrend
//!   einer laufenden Aufnahme, nie mit `lead == 0` (Erinnerung aus);
//! - nur bei >= 2 verschiedenen Teilnehmenden (Organisator eingerechnet) ODER
//!   Beitritts-Adresse ODER Quelle ohne Teilnehmerdaten ODER Einstellung „alle
//!   Termine“ (Granola-Regel, E11): ein Einzeltermin ohne Gegenueber ist keine
//!   Besprechung.

use std::collections::HashSet;

use super::model::CalEvent;

/// Nach dem Beginn noch so lange faellig; danach gilt der Termin als verpasst.
pub const CATCH_UP_MS: i64 = 2 * 60_000;
/// Fenster um den Beginn, in dem ein manuell gestartete Aufnahme einem Termin
/// zugeordnet wird (`event_for_start`).
pub const START_WINDOW_MS: i64 = 15 * 60_000;

/// Alles, was `due_reminders` ausser den Terminen braucht.
pub struct ReminderCtx<'a> {
    pub now_ms: i64,
    /// Vorlauf in ms; 0 schaltet die Erinnerung aus.
    pub lead_ms: i64,
    /// Es laeuft schon eine Aufnahme.
    pub recording: bool,
    /// Einstellung „auch ohne Teilnehmende“.
    pub all_events: bool,
    /// Liefert die Quelle (Argument: `source_id`) Teilnehmerdaten?
    pub attendee_data: &'a dyn Fn(&str) -> bool,
    /// Wurde zu diesem Termin (Argument: `key`) schon erinnert oder verworfen?
    pub handled: &'a dyn Fn(&str) -> bool,
}

/// Verschiedene Teilnehmende eines Termins, Organisator eingerechnet. Gleich
/// sind zwei Eintraege mit derselben E-Mail (ohne Gross-/Kleinschreibung); ohne
/// E-Mail zaehlt der Name.
pub fn distinct_attendees(event: &CalEvent) -> usize {
    let mut seen: HashSet<String> = HashSet::new();
    for a in &event.attendees {
        let email = a.email.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let name = a.name.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(key) = email.or(name) {
            seen.insert(key.to_lowercase());
        }
    }
    seen.len()
}

fn is_meeting_like(event: &CalEvent, ctx: &ReminderCtx<'_>) -> bool {
    ctx.all_events
        || event
            .join_url
            .as_deref()
            .is_some_and(|u| !u.trim().is_empty())
        || !(ctx.attendee_data)(&event.source_id)
        || distinct_attendees(event) >= 2
}

/// Die Termine, zu denen jetzt erinnert werden soll, nach Beginn sortiert.
pub fn due_reminders<'e>(events: &'e [CalEvent], ctx: &ReminderCtx<'_>) -> Vec<&'e CalEvent> {
    if ctx.lead_ms <= 0 || ctx.recording {
        return Vec::new();
    }
    let mut due: Vec<&CalEvent> = events
        .iter()
        .filter(|e| {
            !e.all_day
                && !e.cancelled
                && e.starts_at - ctx.lead_ms <= ctx.now_ms
                && ctx.now_ms < e.starts_at + CATCH_UP_MS
                && !(ctx.handled)(&e.key)
                && is_meeting_like(e, ctx)
        })
        .collect();
    due.sort_by(|a, b| {
        a.starts_at
            .cmp(&b.starts_at)
            .then_with(|| a.key.cmp(&b.key))
    });
    due
}

/// Der Termin, dem eine jetzt beginnende Aufnahme gehoert: sein Beginn liegt
/// hoechstens `window_ms` vor oder nach `now_ms`. Genau EIN Treffer, sonst
/// `None` (zwei Termine zur selben Zeit: nicht raten). Ganztaegige und
/// abgesagte zaehlen nicht; derselbe Termin aus zwei Quellen (gleiche UID und
/// gleicher Beginn) zaehlt einmal.
pub fn event_for_start<'e>(
    events: &'e [CalEvent],
    now_ms: i64,
    window_ms: i64,
) -> Option<&'e CalEvent> {
    let mut found: Option<&CalEvent> = None;
    let mut seen: HashSet<(&str, i64)> = HashSet::new();
    for e in events {
        if e.all_day || e.cancelled || (e.starts_at - now_ms).abs() > window_ms {
            continue;
        }
        if !seen.insert((e.uid.as_str(), e.starts_at)) {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(e);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::calendar::model::Attendee;

    const MIN: i64 = 60_000;
    /// 29.09.2026 10:00 UTC.
    const T0: i64 = 1_790_589_600_000;

    fn att(email: &str, organizer: bool) -> Attendee {
        Attendee {
            email: Some(email.to_string()),
            name: None,
            organizer,
            is_self: false,
            partstat: None,
        }
    }

    fn event(key: &str, start: i64, attendees: Vec<Attendee>) -> CalEvent {
        CalEvent {
            key: key.to_string(),
            source_id: "src".to_string(),
            uid: format!("uid-{key}"),
            title: format!("Termin {key}"),
            starts_at: start,
            ends_at: start + 30 * MIN,
            all_day: false,
            cancelled: false,
            location: None,
            join_url: None,
            description: None,
            attendees,
        }
    }

    fn two() -> Vec<Attendee> {
        vec![att("a@x.de", true), att("b@x.de", false)]
    }

    fn keys(due: &[&CalEvent]) -> Vec<String> {
        due.iter().map(|e| e.key.clone()).collect()
    }

    /// Kontext: Quelle mit Teilnehmerdaten, nichts erinnert, keine Aufnahme.
    fn run(events: &[CalEvent], now: i64, tweak: impl Fn(&mut Flags)) -> Vec<String> {
        let mut f = Flags::default();
        tweak(&mut f);
        let attendee_data = |_: &str| f.attendee_data;
        let handled = |k: &str| f.handled.contains(&k.to_string());
        let ctx = ReminderCtx {
            now_ms: now,
            lead_ms: f.lead_ms,
            recording: f.recording,
            all_events: f.all_events,
            attendee_data: &attendee_data,
            handled: &handled,
        };
        keys(&due_reminders(events, &ctx))
    }

    struct Flags {
        lead_ms: i64,
        recording: bool,
        all_events: bool,
        attendee_data: bool,
        handled: Vec<String>,
    }

    impl Default for Flags {
        fn default() -> Self {
            Flags {
                lead_ms: MIN,
                recording: false,
                all_events: false,
                attendee_data: true,
                handled: Vec::new(),
            }
        }
    }

    #[test]
    fn due_one_minute_before_start() {
        let evs = [event("a", T0, two())];
        assert_eq!(run(&evs, T0 - MIN, |_| {}), vec!["a"], "genau 1 min vorher");
        assert_eq!(run(&evs, T0 - 30_000, |_| {}), vec!["a"], "30 s vorher");
    }

    #[test]
    fn not_due_earlier_than_the_lead() {
        let evs = [event("a", T0, two())];
        assert!(run(&evs, T0 - MIN - 1, |_| {}).is_empty());
        assert!(run(&evs, T0 - 10 * MIN, |_| {}).is_empty());
    }

    #[test]
    fn still_due_until_two_minutes_after_start() {
        let evs = [event("a", T0, two())];
        assert_eq!(run(&evs, T0, |_| {}), vec!["a"]);
        assert_eq!(run(&evs, T0 + CATCH_UP_MS - 1, |_| {}), vec!["a"]);
        assert!(run(&evs, T0 + CATCH_UP_MS, |_| {}).is_empty());
    }

    #[test]
    fn two_attendees_are_required_when_the_source_has_attendee_data() {
        let solo = [event("a", T0, vec![att("a@x.de", true)])];
        assert!(run(&solo, T0 - MIN, |_| {}).is_empty(), "ein Teilnehmender");
        let none = [event("b", T0, vec![])];
        assert!(
            run(&none, T0 - MIN, |_| {}).is_empty(),
            "keine Teilnehmenden"
        );
        let pair = [event("c", T0, two())];
        assert_eq!(run(&pair, T0 - MIN, |_| {}), vec!["c"]);
    }

    #[test]
    fn organizer_counts_and_duplicates_count_once() {
        let organizer_plus_one = [event(
            "a",
            T0,
            vec![att("o@x.de", true), att("p@x.de", false)],
        )];
        assert_eq!(run(&organizer_plus_one, T0 - MIN, |_| {}), vec!["a"]);
        let dup = [event(
            "b",
            T0,
            vec![
                att("o@x.de", true),
                att("O@X.DE", false),
                att(" o@x.de ", false),
            ],
        )];
        assert!(
            run(&dup, T0 - MIN, |_| {}).is_empty(),
            "eine Person, dreimal gelistet"
        );
    }

    #[test]
    fn names_without_email_are_distinct_people() {
        let named = |n: &str| Attendee {
            email: None,
            name: Some(n.to_string()),
            organizer: false,
            is_self: false,
            partstat: None,
        };
        let evs = [event("a", T0, vec![named("Anna Berg"), named("Bernd Alt")])];
        assert_eq!(run(&evs, T0 - MIN, |_| {}), vec!["a"]);
        let same = [event("b", T0, vec![named("Anna Berg"), named("anna berg")])];
        assert!(run(&same, T0 - MIN, |_| {}).is_empty());
    }

    #[test]
    fn join_url_alone_is_enough() {
        let mut e = event("a", T0, vec![]);
        e.join_url = Some("https://teams.microsoft.com/l/meetup-join/x".to_string());
        assert_eq!(run(&[e], T0 - MIN, |_| {}), vec!["a"]);
        let mut blank = event("b", T0, vec![]);
        blank.join_url = Some("  ".to_string());
        assert!(
            run(&[blank], T0 - MIN, |_| {}).is_empty(),
            "leere Adresse zaehlt nicht"
        );
    }

    #[test]
    fn source_without_attendee_data_is_not_bound_to_two_attendees() {
        let evs = [event("a", T0, vec![])];
        assert_eq!(run(&evs, T0 - MIN, |f| f.attendee_data = false), vec!["a"]);
    }

    #[test]
    fn all_events_setting_lifts_the_attendee_rule() {
        let evs = [event("a", T0, vec![])];
        assert!(run(&evs, T0 - MIN, |_| {}).is_empty());
        assert_eq!(run(&evs, T0 - MIN, |f| f.all_events = true), vec!["a"]);
    }

    #[test]
    fn all_day_events_never_remind() {
        let mut e = event("a", T0, two());
        e.all_day = true;
        assert!(run(&[e.clone()], T0 - MIN, |_| {}).is_empty());
        assert!(
            run(&[e], T0 - MIN, |f| f.all_events = true).is_empty(),
            "auch nicht mit „alle Termine“"
        );
    }

    #[test]
    fn cancelled_events_never_remind() {
        let mut e = event("a", T0, two());
        e.cancelled = true;
        assert!(run(&[e], T0 - MIN, |f| f.all_events = true).is_empty());
    }

    #[test]
    fn already_reminded_or_dismissed_events_are_skipped() {
        let evs = [event("a", T0, two()), event("b", T0, two())];
        let got = run(&evs, T0 - MIN, |f| f.handled = vec!["a".to_string()]);
        assert_eq!(got, vec!["b"]);
    }

    #[test]
    fn a_running_recording_suppresses_every_reminder() {
        let evs = [event("a", T0, two())];
        assert!(run(&evs, T0 - MIN, |f| f.recording = true).is_empty());
    }

    #[test]
    fn lead_zero_switches_reminders_off() {
        let evs = [event("a", T0, two())];
        assert!(run(&evs, T0, |f| f.lead_ms = 0).is_empty());
    }

    #[test]
    fn a_longer_lead_fires_earlier() {
        let evs = [event("a", T0, two())];
        assert_eq!(run(&evs, T0 - 5 * MIN, |f| f.lead_ms = 5 * MIN), vec!["a"]);
        assert!(run(&evs, T0 - 5 * MIN - 1, |f| f.lead_ms = 5 * MIN).is_empty());
    }

    #[test]
    fn resume_after_sleep_does_not_catch_up_old_events() {
        // Rechner war im Ruhezustand: der erste Tick kommt 10 min nach dem Beginn.
        let evs = [event("a", T0, two())];
        assert!(run(&evs, T0 + 10 * MIN, |_| {}).is_empty());
    }

    #[test]
    fn due_events_are_sorted_by_start() {
        let evs = [
            event("spaet", T0 + 30_000, two()),
            event("frueh", T0, two()),
        ];
        assert_eq!(run(&evs, T0 - 10_000, |_| {}), vec!["frueh", "spaet"]);
    }

    // ---- event_for_start -----------------------------------------------------

    #[test]
    fn event_for_start_finds_the_single_event_in_the_window() {
        let evs = [
            event("a", T0, two()),
            event("weit", T0 + 3 * 60 * MIN, two()),
        ];
        assert_eq!(
            event_for_start(&evs, T0 - 10 * MIN, START_WINDOW_MS).map(|e| e.key.as_str()),
            Some("a")
        );
        assert_eq!(
            event_for_start(&evs, T0 + 14 * MIN, START_WINDOW_MS).map(|e| e.key.as_str()),
            Some("a")
        );
    }

    #[test]
    fn event_for_start_is_none_outside_the_window() {
        let evs = [event("a", T0, two())];
        assert!(event_for_start(&evs, T0 - 16 * MIN, START_WINDOW_MS).is_none());
        assert!(event_for_start(&evs, T0 + 16 * MIN, START_WINDOW_MS).is_none());
    }

    #[test]
    fn event_for_start_is_none_when_ambiguous() {
        let evs = [event("a", T0, two()), event("b", T0 + 10 * MIN, two())];
        assert!(event_for_start(&evs, T0 + 5 * MIN, START_WINDOW_MS).is_none());
    }

    #[test]
    fn event_for_start_ignores_all_day_and_cancelled_events() {
        let mut all_day = event("tag", T0, two());
        all_day.all_day = true;
        let mut cancelled = event("weg", T0, two());
        cancelled.cancelled = true;
        let real = event("echt", T0 + MIN, two());
        let evs = [all_day, cancelled, real];
        assert_eq!(
            event_for_start(&evs, T0, START_WINDOW_MS).map(|e| e.key.as_str()),
            Some("echt")
        );
    }

    #[test]
    fn event_for_start_counts_the_same_event_from_two_sources_once() {
        let mut a = event("a", T0, two());
        a.uid = "gleich".to_string();
        let mut b = event("b", T0, two());
        b.uid = "gleich".to_string();
        b.source_id = "andere".to_string();
        let evs = [a, b];
        assert!(event_for_start(&evs, T0, START_WINDOW_MS).is_some());
    }
}
