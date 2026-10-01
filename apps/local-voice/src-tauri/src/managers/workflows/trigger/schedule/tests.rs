//! Zeitplan mit fester Uhr (B2).

use std::sync::Arc;

use chrono::{FixedOffset, TimeZone, Utc};
use serde_json::{json, Map, Value};

use super::*;
use crate::managers::workflows::engine::Engine;
use crate::managers::workflows::store::{self, RunFilter};
use crate::managers::workflows::test_support::{
    armed_workflow, def, engine, engine_with, note, roomy_gate, FakeClock, Fx,
};
use crate::managers::workflows::trigger::test_sink::FlakySink;

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;

/// Berlin im Winter: UTC+1 (die Zeitzone ist ein Parameter, die Tests haengen nicht an der
/// Zone des Rechners).
fn berlin() -> FixedOffset {
    FixedOffset::east_opt(3600).unwrap()
}

/// Ortszeit (UTC+1) als ms UTC.
fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    berlin()
        .with_ymd_and_hms(y, mo, d, h, mi, 0)
        .unwrap()
        .timestamp_millis()
}

fn plan(every: &str, at: &str, weekdays: Option<Value>) -> Plan {
    let mut m = Map::new();
    m.insert("every".into(), json!(every));
    m.insert("at".into(), json!(at));
    if let Some(w) = weekdays {
        m.insert("weekdays".into(), w);
    }
    plan_from(&m).unwrap_or_else(|| panic!("Plan ungueltig: {m:?}"))
}

fn params(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

// ---------------------------------------------------------------------------
// Pruefung
// ---------------------------------------------------------------------------

#[test]
fn valid_plans_pass_and_become_a_plan() {
    for p in [
        json!({"every": "daily", "at": "08:00"}),
        json!({"every": "daily", "at": "8:05"}),
        json!({"every": "daily", "at": "23:59"}),
        json!({"every": "weekly", "at": "17:30", "weekdays": ["mo", "Mi", "FR"]}),
        json!({"every": "weekly", "at": "07:00", "weekdays": ["mon", "sun", 3]}),
    ] {
        assert!(check(&params(p.clone())).is_empty(), "{p}");
        assert!(plan_from(&params(p.clone())).is_some(), "{p}");
    }
    let p = plan("weekly", "17:30", Some(json!(["mo", "mi", 5, "sun"])));
    assert_eq!((p.hour, p.minute), (17, 30));
    assert_eq!(
        p.weekdays.iter().copied().collect::<Vec<_>>(),
        vec![1, 3, 5, 7]
    );
}

#[test]
fn invalid_times_and_weekdays_are_named_with_a_sentence() {
    let bad_at = [
        "", "8", "24:00", "12:60", "12:5", "ab:cd", "08:00:00", "-1:00", "123:00",
    ];
    for at in bad_at {
        let found = check(&params(json!({"every": "daily", "at": at})));
        assert_eq!(found.len(), 1, "{at:?} -> {found:?}");
        assert_eq!(found[0].0, "at");
        assert!(found[0].1.contains("HH:MM"));
    }
    // `at` ist kein Text.
    assert_eq!(check(&params(json!({"every": "daily", "at": 8}))).len(), 1);
    // Wochentage.
    let weekly = |w: Value| {
        check(&params(
            json!({"every": "weekly", "at": "08:00", "weekdays": w}),
        ))
    };
    assert!(!weekly(json!([])).is_empty(), "leer");
    assert!(!weekly(json!(["xx"])).is_empty(), "unbekannt");
    assert!(!weekly(json!([8])).is_empty(), "Zahl ausserhalb");
    assert!(!weekly(json!("mo")).is_empty(), "kein Array");
    assert!(
        !check(&params(json!({"every": "weekly", "at": "08:00"}))).is_empty(),
        "weekly ohne Tage"
    );
    assert!(
        !check(&params(
            json!({"every": "daily", "at": "08:00", "weekdays": ["mo"]})
        ))
        .is_empty(),
        "daily mit Tagen"
    );
    assert!(plan_from(&params(json!({"every": "weekly", "at": "08:00"}))).is_none());
}

#[test]
fn saving_validates_the_schedule_and_variables_of_automatic_triggers() {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let e = engine(&fx, &clock);
    let mut d = def(vec![note("a")]);
    d["trigger"] = json!({"type": "schedule", "every": "daily", "at": "25:00"});
    let err = e.save_workflow(None, &d).unwrap_err();
    let store::WorkflowError::Invalid(issues) = err else {
        panic!("erwartet Invalid");
    };
    assert!(issues.iter().any(|i| i.path == "/trigger/at"), "{issues:?}");
    d["trigger"] = json!({"type": "schedule", "every": "daily", "at": "08:00"});
    e.save_workflow(None, &d).unwrap();

    // Eine Variable ohne Vorgabewert: bei einem Zeitplan kann niemand sie angeben.
    d["variables"] = json!({"wer": {"type": "string"}});
    let err = e.save_workflow(None, &d).unwrap_err();
    let store::WorkflowError::Invalid(issues) = err else {
        panic!("erwartet Invalid");
    };
    assert!(
        issues
            .iter()
            .any(|i| i.path == "/variables/wer" && i.message.contains("Vorgabewert")),
        "{issues:?}"
    );
    // Beim manuellen Start ist sie in Ordnung (der Start fragt sie ab).
    d["trigger"] = json!({"type": "manual"});
    e.save_workflow(None, &d).unwrap();
}

// ---------------------------------------------------------------------------
// Zeitpunkte
// ---------------------------------------------------------------------------

#[test]
fn the_slot_is_the_latest_one_within_the_grace_period() {
    let p = plan("daily", "08:00", None);
    let tz = berlin();
    let at = local(2026, 10, 2, 8, 0);
    assert_eq!(
        latest_slot(&p, at - 1, &tz),
        None,
        "kurz vor 8:00: der von gestern ist zu alt"
    );
    assert_eq!(latest_slot(&p, at, &tz), Some(at), "auf die Sekunde");
    assert_eq!(latest_slot(&p, at + 14 * MIN, &tz), Some(at));
    assert_eq!(
        latest_slot(&p, at + GRACE_MS, &tz),
        Some(at),
        "Karenz inklusive"
    );
    assert_eq!(
        latest_slot(&p, at + GRACE_MS + 1, &tz),
        None,
        "danach verfaellt der Lauf"
    );
    assert_eq!(latest_slot(&p, at + 5 * HOUR, &tz), None);
}

#[test]
fn midnight_is_crossed_correctly() {
    let p = plan("daily", "23:55", None);
    let tz = berlin();
    let slot = local(2026, 10, 1, 23, 55);
    // 00:05 am Folgetag: der Zeitpunkt von gestern gilt noch.
    assert_eq!(latest_slot(&p, local(2026, 10, 2, 0, 5), &tz), Some(slot));
    assert_eq!(latest_slot(&p, local(2026, 10, 2, 0, 11), &tz), None);
}

#[test]
fn weekly_plans_fire_only_on_their_days() {
    // 1.10.2026 ist ein Donnerstag.
    let p = plan("weekly", "09:00", Some(json!(["mo", "do"])));
    let tz = berlin();
    assert_eq!(
        latest_slot(&p, local(2026, 10, 1, 9, 1), &tz),
        Some(local(2026, 10, 1, 9, 0)),
        "Donnerstag"
    );
    assert_eq!(
        latest_slot(&p, local(2026, 10, 2, 9, 1), &tz),
        None,
        "Freitag"
    );
    assert_eq!(
        latest_slot(&p, local(2026, 10, 5, 9, 1), &tz),
        Some(local(2026, 10, 5, 9, 0)),
        "Montag"
    );
    // Sonntag-Zahl 7 und Montag-Zahl 1.
    let p = plan("weekly", "09:00", Some(json!([7])));
    assert_eq!(
        latest_slot(&p, local(2026, 10, 4, 9, 0), &tz),
        Some(local(2026, 10, 4, 9, 0)),
        "4.10.2026 ist ein Sonntag"
    );
}

#[test]
fn the_time_zone_decides_the_local_clock() {
    // 08:00 Ortszeit in UTC+1 ist 07:00 UTC; in UTC selbst wuerde derselbe Plan eine Stunde
    // spaeter feuern.
    let p = plan("daily", "08:00", None);
    let utc_slot = Utc
        .with_ymd_and_hms(2026, 10, 2, 8, 0, 0)
        .unwrap()
        .timestamp_millis();
    assert_eq!(latest_slot(&p, utc_slot, &Utc), Some(utc_slot));
    assert_eq!(
        latest_slot(&p, utc_slot, &berlin()),
        None,
        "9:00 Berlin: zu spaet"
    );
    assert_eq!(
        latest_slot(&p, local(2026, 10, 2, 8, 0), &berlin()),
        Some(local(2026, 10, 2, 8, 0))
    );
}

#[test]
fn the_key_is_the_instant_in_utc() {
    assert_eq!(
        key_for(local(2026, 10, 2, 8, 0)),
        "schedule:2026-10-02T07:00:00Z"
    );
}

// ---------------------------------------------------------------------------
// Takt
// ---------------------------------------------------------------------------

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
}

fn world_at(now: i64) -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    clock.set(now);
    let engine = engine_with(&fx, &clock, roomy_gate());
    World { fx, clock, engine }
}

fn scheduled(w: &World, name: &str, plan: Value) -> String {
    let mut d = def(vec![note("a")]);
    d["name"] = json!(name);
    let mut t = json!({"type": "schedule"});
    for (k, v) in plan.as_object().unwrap() {
        t[k] = v.clone();
    }
    d["trigger"] = t;
    armed_workflow(&w.engine, &d)
}

fn runs(w: &World, wf: &str) -> Vec<store::RunRow> {
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

#[test]
fn a_daily_plan_fires_exactly_once_per_slot_across_ticks_and_restarts() {
    let slot = local(2026, 10, 2, 8, 0);
    // Der Ablauf wurde gestern angelegt.
    let w = world_at(slot - 20 * HOUR);
    let wf = scheduled(&w, "Tagesbericht", json!({"every": "daily", "at": "08:00"}));
    w.clock.set(slot);
    assert!(
        on_tick(&w.engine, slot - 1, &berlin()).started.is_empty(),
        "vor der Zeit"
    );
    let first = on_tick(&w.engine, slot, &berlin());
    assert_eq!(first.started.len(), 1, "{first:?}");
    // Alle 15 s bis zum Ende der Karenzzeit: kein zweiter Lauf.
    let mut now = slot;
    while now < slot + GRACE_MS + MIN {
        now += 15_000;
        let r = on_tick(&w.engine, now, &berlin());
        assert!(r.started.is_empty(), "{r:?} bei +{}s", (now - slot) / 1000);
    }
    assert_eq!(runs(&w, &wf).len(), 1);
    // Neustart innerhalb der Karenzzeit: kein zweiter Lauf.
    let restarted = engine_with(&w.fx, &w.clock, roomy_gate());
    let r = on_tick(&restarted, slot + 3 * MIN, &berlin());
    assert!(r.started.is_empty() && r.duplicates == 1, "{r:?}");
    // Am naechsten Tag feuert der naechste Zeitpunkt.
    let next = local(2026, 10, 3, 8, 0);
    assert_eq!(on_tick(&w.engine, next, &berlin()).started.len(), 1);
    assert_eq!(runs(&w, &wf).len(), 2);
    let second = runs(&w, &wf)
        .into_iter()
        .find(|r| r.trigger_key == key_for(next))
        .expect("Lauf des naechsten Tages");
    let ctx: Value = serde_json::from_str(&second.context_json).unwrap();
    assert_eq!(ctx["trigger"]["scheduled_for"], "2026-10-03T07:00:00Z");
}

#[test]
fn a_run_missed_while_the_app_was_closed_is_caught_up_only_within_the_grace_period() {
    let slot = local(2026, 10, 2, 8, 0);
    let w = world_at(slot - 12 * HOUR);
    let wf = scheduled(&w, "Nachholen", json!({"every": "daily", "at": "08:00"}));
    // Start der App um 08:10: nachholen.
    let r = on_tick(&w.engine, slot + 10 * MIN, &berlin());
    assert_eq!(r.started.len(), 1);
    // Ein anderer Ablauf, App erst um 11:00 gestartet: verfaellt.
    let w2 = world_at(slot - 12 * HOUR);
    let wf2 = scheduled(&w2, "Zu spaet", json!({"every": "daily", "at": "08:00"}));
    assert!(on_tick(&w2.engine, slot + 3 * HOUR, &berlin())
        .started
        .is_empty());
    assert!(runs(&w2, &wf2).is_empty());
    assert_eq!(runs(&w, &wf).len(), 1);
}

#[test]
fn a_new_or_changed_workflow_never_fires_for_a_slot_that_lies_before_it() {
    let slot = local(2026, 10, 2, 8, 0);
    // Angelegt, eingeschaltet und scharf geschaltet um 08:05: der Zeitpunkt 08:00 liegt davor.
    let w = world_at(slot + 5 * MIN);
    let wf = scheduled(&w, "Neu", json!({"every": "daily", "at": "08:00"}));
    assert!(on_tick(&w.engine, slot + 6 * MIN, &berlin())
        .started
        .is_empty());
    assert!(runs(&w, &wf).is_empty());
    // Geaendert auf 08:10, noch bevor dieser Zeitpunkt kam: er gilt.
    let mut d = def(vec![note("a")]);
    d["name"] = json!("Neu");
    d["trigger"] = json!({"type": "schedule", "every": "daily", "at": "08:10"});
    w.engine.save_workflow(Some(&wf), &d).unwrap();
    w.engine.set_enabled(&wf, true).unwrap();
    w.engine.set_armed(&wf, true).unwrap();
    assert_eq!(
        on_tick(&w.engine, slot + 10 * MIN, &berlin()).started.len(),
        1,
        "der neue Zeitpunkt liegt NACH dem Aendern"
    );
}

#[test]
fn a_disabled_schedule_does_not_fire_and_other_triggers_are_ignored() {
    let slot = local(2026, 10, 2, 8, 0);
    let w = world_at(slot - 5 * HOUR);
    let wf = scheduled(&w, "Aus", json!({"every": "daily", "at": "08:00"}));
    w.engine.set_enabled(&wf, false).unwrap();
    let mut d = def(vec![note("a")]);
    d["name"] = json!("Manuell");
    let manual = armed_workflow(&w.engine, &d);
    assert!(on_tick(&w.engine, slot, &berlin()).started.is_empty());
    assert!(runs(&w, &wf).is_empty() && runs(&w, &manual).is_empty());
}

#[test]
fn two_schedules_and_two_threads_still_make_one_run_each() {
    let slot = local(2026, 10, 2, 8, 0);
    let w = world_at(slot - 5 * HOUR);
    let a = scheduled(&w, "A", json!({"every": "daily", "at": "08:00"}));
    let b = scheduled(
        &w,
        "B",
        json!({"every": "weekly", "at": "08:00", "weekdays": ["fr"]}),
    );
    std::thread::scope(|s| {
        for _ in 0..6 {
            s.spawn(|| {
                on_tick(&w.engine, slot + MIN, &berlin());
            });
        }
    });
    assert_eq!(runs(&w, &a).len(), 1);
    assert_eq!(runs(&w, &b).len(), 1, "2.10.2026 ist ein Freitag");
}

#[test]
fn a_failed_enqueue_is_retried_by_the_next_tick_within_the_grace_period() {
    let slot = local(2026, 10, 2, 8, 0);
    let w = world_at(slot - 5 * HOUR);
    let wf = scheduled(&w, "Voll", json!({"every": "daily", "at": "08:00"}));
    let flaky = FlakySink::new(&w.engine, 1);
    let first = on_tick(&flaky, slot, &berlin());
    assert_eq!(first.errors.len(), 1);
    assert!(runs(&w, &wf).is_empty());
    let second = on_tick(&flaky, slot + 15_000, &berlin());
    assert_eq!(second.started.len(), 1, "{second:?}");
    assert_eq!(runs(&w, &wf).len(), 1);
}

#[test]
fn a_clock_that_jumps_back_does_not_repeat_a_run() {
    let slot = local(2026, 10, 2, 8, 0);
    let w = world_at(slot - 5 * HOUR);
    let wf = scheduled(&w, "Uhr", json!({"every": "daily", "at": "08:00"}));
    assert_eq!(
        on_tick(&w.engine, slot + 2 * MIN, &berlin()).started.len(),
        1
    );
    // Die Uhr springt (Zeitsynchronisation) eine Minute zurueck und wieder vor.
    assert!(on_tick(&w.engine, slot + MIN, &berlin()).started.is_empty());
    assert!(on_tick(&w.engine, slot + 3 * MIN, &berlin())
        .started
        .is_empty());
    assert_eq!(runs(&w, &wf).len(), 1);
}
