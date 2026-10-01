//! AK3: „Termin beginnt“ mit fester Uhr und Test-Kalender.

use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::managers::calendar::model::{event_key, Attendee, CalendarKind};
use crate::managers::calendar::reminder::{due_reminders, ReminderCtx, CATCH_UP_MS};
use crate::managers::meetings::store::SyncMeta;
use crate::managers::workflows::engine::Engine;
use crate::managers::workflows::store::{self, RunFilter, RunRow};
use crate::managers::workflows::test_support::{
    armed_workflow, def, engine, engine_with, note, roomy_gate, FakeClock, Fx, T0,
};
use crate::managers::workflows::trigger::test_sink::FlakySink;

const MIN: i64 = 60_000;
/// Beginn des Testtermins: 10 Minuten nach `T0`.
const START: i64 = T0 + 10 * MIN;

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
    state: State,
}

fn world() -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    add_source(&fx, "cal-1", true);
    World {
        fx,
        clock,
        engine,
        state: State::default(),
    }
}

fn add_source(fx: &Fx, id: &str, has_attendee_data: bool) {
    fx.store
        .calendar_source_add(id, CalendarKind::Ics, "Test", Some("example.org"), 1)
        .unwrap();
    // Die Quelle meldet, ob sie Teilnehmerdaten liefert, erst beim ersten Abruf.
    put(fx, id, &[], has_attendee_data);
}

fn put(fx: &Fx, source: &str, events: &[CalEvent], has_attendee_data: bool) {
    fx.store
        .calendar_replace_events(
            source,
            events,
            &SyncMeta {
                has_attendee_data,
                etag: None,
                last_modified: None,
                now_ms: 1,
            },
        )
        .unwrap();
}

fn att(email: &str, organizer: bool) -> Attendee {
    Attendee {
        email: Some(email.to_string()),
        name: None,
        organizer,
        is_self: false,
        partstat: None,
    }
}

fn two() -> Vec<Attendee> {
    vec![att("anna@firma.de", true), att("bernd@kunde.de", false)]
}

fn event(source: &str, uid: &str, start: i64, attendees: Vec<Attendee>) -> CalEvent {
    CalEvent {
        key: event_key(source, uid, start),
        source_id: source.to_string(),
        uid: uid.to_string(),
        title: format!("Termin {uid}"),
        starts_at: start,
        ends_at: start + 60 * MIN,
        all_day: false,
        cancelled: false,
        location: None,
        join_url: None,
        description: None,
        attendees,
    }
}

fn trigger(extra: Value) -> Value {
    let mut t = json!({"type": "calendar.event_starting", "integration": "cal-1"});
    for (k, v) in extra.as_object().unwrap() {
        t[k] = v.clone();
    }
    t
}

fn workflow(w: &World, name: &str, trigger: Value) -> String {
    let mut d = def(vec![note("a")]);
    d["name"] = json!(name);
    d["trigger"] = trigger;
    armed_workflow(&w.engine, &d)
}

fn tick_with(
    w: &World,
    sink: &dyn RunSink,
    now: i64,
    recording: bool,
    self_emails: &[String],
) -> TickReport {
    on_tick(
        sink,
        &w.fx.store,
        &w.state,
        &TickInput {
            now_ms: now,
            recording,
            self_emails,
        },
    )
}

fn tick(w: &World, now: i64) -> TickReport {
    tick_with(w, &w.engine, now, false, &[])
}

fn runs(w: &World, wf: &str) -> Vec<RunRow> {
    store::list_runs(
        &w.fx.conn(),
        &RunFilter {
            workflow_id: Some(wf.to_string()),
            state: None,
        },
        100,
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// Genau einmal
// ---------------------------------------------------------------------------

#[test]
fn it_fires_exactly_once_per_event_and_workflow_across_ticks_restarts_and_threads() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    let wf = workflow(&w, "Kundentermin", trigger(json!({})));

    // Ein Minute vorher: der erste Takt startet den Lauf.
    let first = tick(&w, START - MIN);
    assert_eq!(first.started.len(), 1, "{first:?}");
    assert_eq!(first.started[0].0, wf);
    // Alle 15 s ein weiterer Takt bis 2 min nach dem Beginn: nie ein zweiter Lauf.
    let mut now = START - MIN;
    while now < START + CATCH_UP_MS {
        now += 15_000;
        let r = tick(&w, now);
        assert!(r.started.is_empty(), "{r:?} bei {now}");
    }
    assert_eq!(runs(&w, &wf).len(), 1);

    // Neustart: neue Engine UND neuer Merker-Zustand, derselbe Termin im Fenster.
    let restarted = engine_with(&w.fx, &w.clock, roomy_gate());
    let fresh = State::default();
    let r = on_tick(
        &restarted,
        &w.fx.store,
        &fresh,
        &TickInput {
            now_ms: START,
            recording: false,
            self_emails: &[],
        },
    );
    assert!(r.started.is_empty() && r.duplicates == 1, "{r:?}");
    assert_eq!(runs(&w, &wf).len(), 1);

    // Mehrere Threads im selben Takt: weiterhin ein Lauf.
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                let _ = tick(&w, START);
            });
        }
    });
    assert_eq!(runs(&w, &wf).len(), 1, "ein Lauf, egal wer zuerst tickt");
}

#[test]
fn a_new_event_of_a_series_fires_again_because_the_key_has_the_start_in_it() {
    let w = world();
    let day = 24 * 60 * MIN;
    put(
        &w.fx,
        "cal-1",
        &[
            event("cal-1", "serie", START, two()),
            event("cal-1", "serie", START + day, two()),
        ],
        true,
    );
    let wf = workflow(&w, "Serie", trigger(json!({})));
    assert_eq!(tick(&w, START).started.len(), 1);
    assert_eq!(tick(&w, START + day).started.len(), 1);
    assert_eq!(runs(&w, &wf).len(), 2);
}

#[test]
fn two_workflows_on_one_event_each_get_their_own_run() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    let a = workflow(&w, "A", trigger(json!({})));
    let b = workflow(&w, "B", trigger(json!({})));
    let r = tick(&w, START);
    assert_eq!(r.started.len(), 2);
    assert_eq!(runs(&w, &a).len(), 1);
    assert_eq!(runs(&w, &b).len(), 1);
}

// ---------------------------------------------------------------------------
// Zeitfenster: dieselbe Regel wie die Erinnerung
// ---------------------------------------------------------------------------

#[test]
fn the_window_opens_at_the_lead_and_closes_two_minutes_after_the_start() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    let wf = workflow(&w, "Fenster", trigger(json!({"lead_min": 5})));
    assert!(tick(&w, START - 5 * MIN - 1).started.is_empty(), "zu frueh");
    assert!(runs(&w, &wf).is_empty());
    assert_eq!(
        tick(&w, START - 5 * MIN).started.len(),
        1,
        "genau am Vorlauf"
    );

    // Anderer Termin, anderer Ablauf: nach dem Nachlauf nicht mehr.
    let w2 = world();
    put(&w2.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    let wf2 = workflow(&w2, "Spaet", trigger(json!({})));
    assert!(
        tick(&w2, START + CATCH_UP_MS).started.is_empty(),
        "wer aus dem Ruhezustand kommt, holt keinen Termin nach, der laengst begann"
    );
    assert!(runs(&w2, &wf2).is_empty());
    let w3 = world();
    put(&w3.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    workflow(&w3, "Knapp", trigger(json!({})));
    assert_eq!(tick(&w3, START + CATCH_UP_MS - 1).started.len(), 1);
}

#[test]
fn without_lead_min_the_default_is_one_minute_like_the_reminder() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    workflow(&w, "Vorgabe", trigger(json!({})));
    assert!(tick(&w, START - MIN - 1).started.is_empty());
    assert_eq!(tick(&w, START - MIN).started.len(), 1);
}

#[test]
fn lead_zero_fires_at_the_start_not_before() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    workflow(&w, "Null", trigger(json!({"lead_min": 0})));
    assert!(tick(&w, START - 1).started.is_empty());
    assert_eq!(tick(&w, START).started.len(), 1);
}

/// Die Zeitregel ist KEINE Kopie: fuer jedes Zeitverhaeltnis und jede Termin-Art gibt der
/// Ausloeser dieselbe Antwort wie `due_reminders` (gleicher Vorlauf, keine „alle Termine“).
#[test]
fn it_answers_exactly_like_the_reminder_for_every_kind_of_event() {
    let mut all_day = event("cal-1", "tag", START, two());
    all_day.all_day = true;
    let mut cancelled = event("cal-1", "weg", START, two());
    cancelled.cancelled = true;
    let mut join = event("cal-1", "join", START, vec![]);
    join.join_url = Some("https://teams.example/x".to_string());
    let events = vec![
        event("cal-1", "zwei", START, two()),
        event("cal-1", "solo", START, vec![att("a@x.de", true)]),
        event("cal-1", "leer", START, vec![]),
        all_day,
        cancelled,
        join,
        event("cal-1", "spaeter", START + 30 * MIN, two()),
    ];
    let filter = Filter {
        integration: "cal-1".into(),
        lead_ms: MIN,
        only_meetings: true,
        title_contains: None,
        min_attendees: 0,
    };
    for offset in [
        -10 * MIN,
        -MIN - 1,
        -MIN,
        -1,
        0,
        MIN,
        CATCH_UP_MS - 1,
        CATCH_UP_MS,
    ] {
        let now = START + offset;
        let with_data = |_: &str| true;
        let handled = |_: &str| false;
        let ctx = ReminderCtx {
            now_ms: now,
            lead_ms: MIN,
            recording: false,
            all_events: false,
            attendee_data: &with_data,
            handled: &handled,
        };
        let mut expected: Vec<&str> = due_reminders(&events, &ctx)
            .iter()
            .map(|e| e.key.as_str())
            .collect();
        let mut got: Vec<&str> = due(&events, &filter, true, now, &with_data)
            .iter()
            .map(|e| e.key.as_str())
            .collect();
        expected.sort();
        got.sort();
        assert_eq!(got, expected, "bei Offset {offset}");
    }
}

// ---------------------------------------------------------------------------
// Ausschluesse
// ---------------------------------------------------------------------------

#[test]
fn all_day_and_cancelled_events_never_fire() {
    let w = world();
    let mut all_day = event("cal-1", "tag", START, two());
    all_day.all_day = true;
    let mut cancelled = event("cal-1", "weg", START, two());
    cancelled.cancelled = true;
    put(&w.fx, "cal-1", &[all_day, cancelled], true);
    // Auch mit allen Filtern aus.
    let wf = workflow(
        &w,
        "Alles",
        trigger(json!({"only_meetings": false, "lead_min": 120})),
    );
    for offset in [-120 * MIN, -MIN, 0, MIN] {
        let r = tick(&w, START + offset);
        assert!(r.started.is_empty() && r.duplicates == 0, "{r:?}");
    }
    assert!(runs(&w, &wf).is_empty());
}

#[test]
fn a_running_recording_blocks_the_event_and_it_does_not_fire_afterwards_either() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    let wf = workflow(&w, "Aufnahme", trigger(json!({})));
    // Der Nutzer nimmt von Hand auf: Takt waehrend der Aufnahme.
    let r = tick_with(&w, &w.engine, START - 30_000, true, &[]);
    assert!(r.started.is_empty());
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert!(r.skipped[0].contains("Aufnahme"));
    assert_eq!(w.state.skipped_len(), 1);
    // Die Aufnahme endet innerhalb des Nachlaufs: der Ablauf bittet NICHT um eine zweite.
    let later = tick_with(&w, &w.engine, START + MIN, false, &[]);
    assert!(
        later.started.is_empty() && later.duplicates == 0,
        "{later:?}"
    );
    assert!(runs(&w, &wf).is_empty());
    // Ein anderer Termin danach feuert wieder.
    put(
        &w.fx,
        "cal-1",
        &[
            event("cal-1", "u1", START, two()),
            event("cal-1", "u2", START + 20 * MIN, two()),
        ],
        true,
    );
    assert_eq!(
        tick_with(&w, &w.engine, START + 20 * MIN, false, &[])
            .started
            .len(),
        1
    );
}

#[test]
fn the_skip_memory_is_bounded() {
    let s = State::default();
    for i in 0..(MAX_SKIPPED + 40) {
        s.skip("wf", &format!("k{i}"));
    }
    assert_eq!(s.skipped_len(), MAX_SKIPPED);
    assert!(!s.is_skipped("wf", "k0"), "das Aelteste wurde verdraengt");
    assert!(s.is_skipped("wf", &format!("k{}", MAX_SKIPPED + 39)));
}

#[test]
fn a_disabled_workflow_does_not_fire_and_an_unarmed_one_only_plans() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    let off = workflow(&w, "Aus", trigger(json!({})));
    w.engine.set_enabled(&off, false).unwrap();
    // Neu angelegt, eingeschaltet, aber nicht scharf.
    let mut d = def(vec![note("a")]);
    d["name"] = json!("Trocken");
    d["trigger"] = trigger(json!({}));
    let dry = w.engine.save_workflow(None, &d).unwrap();
    w.engine.set_enabled(&dry.id, true).unwrap();
    let r = tick(&w, START);
    assert_eq!(r.started.len(), 1);
    assert!(runs(&w, &off).is_empty());
    let dry_runs = runs(&w, &dry.id);
    assert_eq!(dry_runs.len(), 1);
    assert!(dry_runs[0].dry_run, "ein neuer Ablauf plant nur");
    assert_eq!(dry_runs[0].origin.as_str(), "trigger");
}

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

#[test]
fn only_events_of_the_chosen_calendar_fire() {
    let w = world();
    add_source(&w.fx, "cal-2", true);
    put(&w.fx, "cal-2", &[event("cal-2", "u9", START, two())], true);
    let wf = workflow(&w, "Nur 1", trigger(json!({})));
    assert!(tick(&w, START).started.is_empty());
    assert!(runs(&w, &wf).is_empty());
}

#[test]
fn title_and_attendee_filters_apply() {
    let w = world();
    let mut a = event("cal-1", "a", START, two());
    a.title = "Jour fixe Vertrieb".to_string();
    let mut b = event("cal-1", "b", START, two());
    b.title = "Mittagessen".to_string();
    let c = event(
        "cal-1",
        "c",
        START,
        vec![
            att("a@x.de", true),
            att("b@x.de", false),
            att("c@x.de", false),
        ],
    );
    put(&w.fx, "cal-1", &[a, b, c], true);
    let by_title = workflow(
        &w,
        "Titel",
        trigger(json!({"title_contains": "  JOUR fixe "})),
    );
    let by_count = workflow(&w, "Anzahl", trigger(json!({"min_attendees": 3})));
    tick(&w, START);
    let titles: Vec<String> = runs(&w, &by_title)
        .iter()
        .map(|r| r.trigger_key.clone())
        .collect();
    assert_eq!(titles.len(), 1);
    assert!(titles[0].contains(":cal-1:a:"), "{titles:?}");
    let counts = runs(&w, &by_count);
    assert_eq!(counts.len(), 1);
    assert!(counts[0].trigger_key.contains(":cal-1:c:"));
}

#[test]
fn only_meetings_defaults_to_true_and_can_be_switched_off() {
    let w = world();
    // Eine Quelle MIT Teilnehmerdaten und ein Einzeltermin: keine Besprechung.
    put(
        &w.fx,
        "cal-1",
        &[event(
            "cal-1",
            "solo",
            START,
            vec![att("ich@firma.de", true)],
        )],
        true,
    );
    let strict = workflow(&w, "Streng", trigger(json!({})));
    let lax = workflow(&w, "Locker", trigger(json!({"only_meetings": false})));
    tick(&w, START);
    assert!(
        runs(&w, &strict).is_empty(),
        "ein Einzeltermin ist keine Besprechung"
    );
    assert_eq!(runs(&w, &lax).len(), 1);
}

// ---------------------------------------------------------------------------
// Termin endet
// ---------------------------------------------------------------------------

#[test]
fn the_end_trigger_fires_at_the_end_once_even_while_a_recording_runs() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    let mut d = def(vec![note("a")]);
    d["trigger"] = json!({"type": "calendar.event_ended", "integration": "cal-1"});
    let wf = armed_workflow(&w.engine, &d);
    let end = START + 60 * MIN;
    assert!(tick(&w, end - 1).started.is_empty(), "noch nicht zu Ende");
    // Am Ende laeuft die Aufnahme noch: das haelt „Termin endet“ nicht auf.
    let r = tick_with(&w, &w.engine, end, true, &[]);
    assert_eq!(r.started.len(), 1, "{r:?}");
    assert!(tick(&w, end + MIN).started.is_empty());
    let all = runs(&w, &wf);
    assert_eq!(all.len(), 1);
    assert!(all[0].trigger_key.starts_with("calendar_end:"));
    assert!(
        tick(&w, end + CATCH_UP_MS).started.is_empty(),
        "nach dem Nachlauf nicht mehr"
    );
}

// ---------------------------------------------------------------------------
// Fehler beim Einreihen
// ---------------------------------------------------------------------------

#[test]
fn a_failed_enqueue_is_reported_and_retried_by_the_next_tick_within_the_window() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    let wf = workflow(&w, "Voll", trigger(json!({})));
    let flaky = FlakySink::new(&w.engine, 2);
    let first = tick_with(&w, &flaky, START - MIN, false, &[]);
    assert!(first.started.is_empty());
    assert_eq!(first.errors.len(), 1, "{first:?}");
    assert!(first.errors[0].contains("disk is full"));
    let second = tick_with(&w, &flaky, START - MIN + 15_000, false, &[]);
    assert_eq!(second.errors.len(), 1);
    // Die Platte hat wieder Platz: der Termin ist noch im Fenster, der Lauf entsteht, einmal.
    let third = tick_with(&w, &flaky, START - MIN + 30_000, false, &[]);
    assert_eq!(third.started.len(), 1, "{third:?}");
    assert!(tick_with(&w, &flaky, START - MIN + 45_000, false, &[])
        .started
        .is_empty());
    assert_eq!(runs(&w, &wf).len(), 1);
}

#[test]
fn a_full_queue_is_an_error_of_the_tick_not_a_crash() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    workflow(&w, "Voll", trigger(json!({})));
    // Ein Sink, dessen Warteschlange immer voll ist.
    struct Full<'a>(&'a Engine);
    impl RunSink for Full<'_> {
        fn workflows(&self) -> Result<Vec<store::WorkflowRow>, store::WorkflowError> {
            self.0.workflows()
        }
        fn enqueue(
            &self,
            _: &crate::managers::workflows::engine::EnqueueRequest,
        ) -> Result<crate::managers::workflows::engine::Enqueued, store::WorkflowError> {
            Err(store::WorkflowError::QueueFull("voll".into()))
        }
    }
    let r = tick_with(&w, &Full(&w.engine), START, false, &[]);
    assert_eq!(r.errors.len(), 1);
    assert!(r.started.is_empty());
}

// ---------------------------------------------------------------------------
// Daten des Ausloesers
// ---------------------------------------------------------------------------

#[test]
fn the_trigger_data_carry_what_the_catalog_promises_and_no_join_link() {
    let mut e = event("cal-1", "u1", START, two());
    e.title = "Jour fixe".to_string();
    e.join_url = Some("https://teams.example/join/GEHEIM".to_string());
    e.description = Some("Zugang: 123456".to_string());
    let data = trigger_data(&e, &["anna@firma.de".to_string()]);
    for field in crate::managers::workflows::catalog::trigger_spec(KIND_START)
        .unwrap()
        .provides
    {
        assert!(data.get(field).is_some(), "{field} fehlt");
    }
    assert_eq!(data["calendar"], "cal-1");
    assert_eq!(data["event_id"], e.key);
    assert_eq!(data["title"], "Jour fixe");
    assert_eq!(data["meeting"]["title"], "Jour fixe");
    assert_eq!(data["start"], iso(START));
    assert_eq!(data["end"], iso(START + 60 * MIN));
    assert_eq!(data["online"], true);
    let text = data.to_string();
    assert!(
        !text.contains("GEHEIM") && !text.contains("123456"),
        "{text}"
    );
    // Selbst und extern.
    assert_eq!(data["attendees"][0]["is_self"], true);
    assert_eq!(data["attendees"][1]["is_self"], false);
    assert_eq!(data["external_attendees"], 1, "bernd@kunde.de ist extern");
}

#[test]
fn external_means_another_domain_than_the_own_addresses() {
    let e = event(
        "cal-1",
        "u1",
        START,
        vec![
            att("ich@firma.de", true),
            att("Kollegin@FIRMA.de", false),
            att("kunde@kunde.de", false),
            att("kunde@kunde.de", false), // doppelt
            Attendee {
                email: None,
                name: Some("Ohne Adresse".into()),
                organizer: false,
                is_self: false,
                partstat: None,
            },
        ],
    );
    let own = vec!["ich@firma.de".to_string()];
    assert_eq!(trigger_data(&e, &own)["external_attendees"], 1);
    // Ohne eigene Adressen ist jede andere Person extern.
    assert_eq!(trigger_data(&e, &[])["external_attendees"], 3);
}

#[test]
fn an_event_with_hundreds_of_attendees_still_starts_a_run() {
    let w = world();
    let many: Vec<Attendee> = (0..500)
        .map(|i| att(&format!("p{i}@kunde.de"), i == 0))
        .collect();
    let mut e = event("cal-1", "gross", START, many);
    e.title = "x".repeat(3_000);
    put(&w.fx, "cal-1", &[e], true);
    let wf = workflow(&w, "Gross", trigger(json!({})));
    let r = tick(&w, START);
    assert_eq!(r.started.len(), 1, "{r:?}");
    let run = &runs(&w, &wf)[0];
    let ctx: Value = serde_json::from_str(&run.context_json).unwrap();
    assert_eq!(
        ctx["trigger"]["attendees"].as_array().unwrap().len(),
        MAX_ATTENDEES
    );
    assert_eq!(
        ctx["trigger"]["external_attendees"], 500,
        "gezaehlt wird alles"
    );
    assert_eq!(
        ctx["trigger"]["title"].as_str().unwrap().chars().count(),
        500
    );
}

// ---------------------------------------------------------------------------
// Erinnerung und Ausloeser stoeren sich nicht
// ---------------------------------------------------------------------------

#[test]
fn the_trigger_neither_consumes_nor_needs_the_reminder_marks() {
    let w = world();
    let e = event("cal-1", "u1", START, two());
    put(&w.fx, "cal-1", &[e.clone()], true);
    workflow(&w, "Ablauf", trigger(json!({})));
    tick(&w, START - MIN);
    assert_eq!(
        w.fx.store.calendar_reminder_state(&e.key).unwrap(),
        Some((None, None)),
        "der Ausloeser verbraucht die Erinnerung nicht"
    );
    // Eine Erinnerung (und selbst „Spaeter“) verbraucht den Ausloeser nicht.
    let w2 = world();
    put(&w2.fx, "cal-1", &[e.clone()], true);
    w2.fx
        .store
        .calendar_mark_reminded(&e.key, START - MIN)
        .unwrap();
    w2.fx
        .store
        .calendar_mark_dismissed(&e.key, START - MIN)
        .unwrap();
    workflow(&w2, "Ablauf", trigger(json!({})));
    assert_eq!(tick(&w2, START - MIN).started.len(), 1);
}

#[test]
fn a_disabled_calendar_source_fires_nothing() {
    let w = world();
    put(&w.fx, "cal-1", &[event("cal-1", "u1", START, two())], true);
    workflow(&w, "Ablauf", trigger(json!({})));
    w.fx.store
        .calendar_source_set_enabled("cal-1", false, 5)
        .unwrap();
    assert!(tick(&w, START).started.is_empty());
}

// ---------------------------------------------------------------------------
// Ein Takt, kein zweiter Poller
// ---------------------------------------------------------------------------

/// Der Ausloeser hat keinen eigenen Zeitgeber: er wird aus `remind_tick` gerufen. Dieser
/// Test liest die Quellen (ein Strukturtest, wie ihn `commands/meetings.rs` kennt).
#[test]
fn the_calendar_trigger_rides_the_reminder_tick_and_starts_no_poller_of_its_own() {
    let service = include_str!("../../../calendar/service.rs");
    let production = service.split("#[cfg(test)]").next().unwrap();
    // Genau EIN Takt-Schleifenkopf, und `remind_tick` ruft den Workflow-Takt vor der Erinnerung.
    assert_eq!(
        production
            .matches("tokio::time::sleep(TICK_INTERVAL)")
            .count(),
        1,
        "es gibt genau einen 15-s-Takt"
    );
    let remind = production
        .split("pub fn remind_tick")
        .nth(1)
        .expect("remind_tick");
    let before_reminders = remind.split("let settings").next().unwrap();
    assert!(
        before_reminders.contains("self.workflow_tick(now_ms)"),
        "remind_tick ruft den Workflow-Takt zuerst"
    );
    assert!(production.contains("hub.on_tick(&self.store, now_ms)"));

    // Die Ausloeser-Module starten weder Threads noch Zeitgeber noch Async-Aufgaben.
    for (name, src) in [
        ("calendar", include_str!("../calendar.rs")),
        ("schedule", include_str!("../schedule.rs")),
        ("meeting_events", include_str!("../meeting_events.rs")),
        ("manual", include_str!("../manual.rs")),
        ("trigger", include_str!("../../trigger.rs")),
    ] {
        let code = src.split("#[cfg(test)]").next().unwrap();
        for needle in [
            "thread::spawn",
            "thread::sleep",
            "thread::Builder",
            "tokio::time",
            "async_runtime::spawn",
            "std::time::Instant",
            "interval(",
        ] {
            assert!(
                !code.contains(needle),
                "trigger::{name} enthaelt {needle}: Ausloeser haengen am Takt, sie bauen keinen eigenen"
            );
        }
    }
}

#[test]
fn the_pure_selection_is_sorted_by_start() {
    let events = vec![
        event("cal-1", "spaet", START + 20_000, two()),
        event("cal-1", "frueh", START, two()),
    ];
    let f = Filter {
        integration: "cal-1".into(),
        lead_ms: MIN,
        only_meetings: false,
        title_contains: None,
        min_attendees: 0,
    };
    let keys: Vec<&str> = due(&events, &f, true, START + 30_000, &|_| true)
        .iter()
        .map(|e| e.uid.as_str())
        .collect();
    assert_eq!(keys, vec!["frueh", "spaet"]);
}
