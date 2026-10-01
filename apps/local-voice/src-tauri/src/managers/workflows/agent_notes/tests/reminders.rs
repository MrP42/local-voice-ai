//! `deadline.remind`: Mitteilung zum eingestellten Zeitpunkt vor der Frist, mit fester Uhr (AK7).

use super::*;
use crate::managers::workflows::engine::RunOutcome::{Done, Parked};

const HOUR: i64 = 3_600_000;

fn remind_workflow(w: &World, params: Value) -> String {
    w.workflow(vec![
        step("extract", "agent.extract", json!({})),
        step("remind", "deadline.remind", params),
    ])
}

fn parked() -> RunOutcome {
    Parked {
        reason: "defer".to_string(),
    }
}

fn ledger_rows(w: &World) -> i64 {
    w.conn()
        .query_row(
            "SELECT COUNT(*) FROM provenance WHERE operation = 'deadline_reminder_shown'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

// -- AK7 ----------------------------------------------------------------------------------------------

#[test]
fn ak7_a_deadline_in_two_days_is_notified_at_the_configured_time_with_a_fixed_clock() {
    let w = World::new(); // 1.10.2026, 14:00; die Frist liegt am 3.10. (in zwei Tagen)
    let wf = remind_workflow(&w, json!({"days_before": 1, "at": "09:00"}));
    let run = w.start(&wf, standard_extraction(&w.meeting_id));

    // Jetzt: nichts zu zeigen, der Lauf wartet (hoechstens 6 Stunden am Stueck).
    assert_eq!(w.tick(), vec![(run.clone(), parked())]);
    assert!(w.toasts().is_empty());
    assert_eq!(w.state(&run, "remind"), StepState::Waiting);
    assert_eq!(w.next_run_at(&run), Some(at(2026, 10, 1, 14, 0) + 6 * HOUR));
    let reason = w.row(&run, "remind").error.unwrap();
    assert!(
        reason.contains("02.10.2026 09:00")
            && reason.contains("Frist")
            && reason.contains("Angebot"),
        "{reason}"
    );

    // Eine Minute vor dem Zeitpunkt: der Takt weckt den Lauf, es kommt noch nichts, er schlaeft bis zur Minute.
    let fire = at(2026, 10, 2, 9, 0);
    w.set_now(fire - 60_000);
    assert_eq!(w.tick(), vec![(run.clone(), parked())]);
    assert!(w.toasts().is_empty(), "08:59 ist zu frueh");
    assert_eq!(w.next_run_at(&run), Some(fire));

    // Eine Sekunde davor ruehrt sich nichts, auch bei weiteren Takten.
    w.set_now(fire - 1_000);
    assert!(w.tick().is_empty(), "der Lauf ist noch nicht faellig");
    assert!(w.toasts().is_empty());

    // Zum Zeitpunkt: genau eine Mitteilung, der Lauf ist fertig.
    w.set_now(fire);
    assert_eq!(w.tick(), vec![(run.clone(), Done)]);
    assert_eq!(
        w.toasts(),
        vec![(
            "Frist morgen: Das Angebot geht an die Stadtwerke.".to_string(),
            format!("Samstag, 03.10.2026. Besprechung „{TITLE}“.")
        )]
    );
    let out = w.output(&run, "remind");
    assert_eq!(
        (out["reminders"].as_u64(), out["sent"].as_u64()),
        (Some(1), Some(1))
    );
    assert_eq!(ledger_rows(&w), 1);

    // Weitere Takte und Stunden aendern nichts.
    w.set_now(fire + 30 * HOUR);
    assert!(w.tick().is_empty());
    assert_eq!(w.toasts().len(), 1);
}

#[test]
fn the_same_deadline_of_the_same_meeting_is_never_reminded_twice_not_even_by_a_second_run() {
    let w = World::new();
    let wf = remind_workflow(&w, json!({"days_before": 1, "at": "09:00"}));
    let first = w.start(&wf, standard_extraction(&w.meeting_id));
    w.tick();
    w.set_now(at(2026, 10, 2, 9, 0));
    assert_eq!(w.tick(), vec![(first, Done)]);
    assert_eq!(w.toasts().len(), 1);

    // Ein zweiter Lauf auf derselben Besprechung (anderer Ausloeser, Neustart, von Hand).
    let second = w.start(&wf, standard_extraction(&w.meeting_id));
    assert_eq!(w.tick(), vec![(second.clone(), Done)]);
    assert_eq!(w.toasts().len(), 1, "keine zweite Mitteilung");
    let out = w.output(&second, "remind");
    assert_eq!(
        (out["sent"].as_u64(), out["already_sent"].as_u64()),
        (Some(0), Some(1))
    );
}

#[test]
fn two_runs_waiting_for_the_same_deadline_show_it_once() {
    let w = World::new();
    let wf = remind_workflow(&w, json!({"days_before": 1, "at": "09:00"}));
    let a = w.start(&wf, standard_extraction(&w.meeting_id));
    let b = w.start(&wf, standard_extraction(&w.meeting_id));
    w.tick();
    w.set_now(at(2026, 10, 2, 9, 0));
    let outcomes = w.tick();
    assert_eq!(outcomes.len(), 2, "{outcomes:?}");
    assert!(outcomes.iter().all(|(_, o)| *o == Done));
    assert_eq!(w.toasts().len(), 1, "{:?}", w.toasts());
    let _ = (a, b);
}

#[test]
fn each_deadline_waits_for_its_own_time_and_the_run_ends_with_the_last() {
    let w = World::new();
    let wf = remind_workflow(&w, json!({"days_before": 1, "at": "09:00"}));
    let mut ex = standard_extraction(&w.meeting_id);
    ex["deadlines"].as_array_mut().unwrap().push(deadline(
        "Die Septemberrechnung muss raus.",
        "2026-10-10",
        "angabe:ende_des_monats",
        &[8],
        "Die Rechnung für September muss bis Ende des Monats raus.",
    ));
    let run = w.start(&wf, ex);
    assert_eq!(w.tick(), vec![(run.clone(), parked())]);

    w.set_now(at(2026, 10, 2, 9, 0));
    assert_eq!(
        w.tick(),
        vec![(run.clone(), parked())],
        "die zweite Frist steht noch aus"
    );
    assert_eq!(w.toasts().len(), 1);
    assert!(w.toasts()[0].0.starts_with("Frist morgen: Das Angebot"));
    assert!(w
        .row(&run, "remind")
        .error
        .unwrap()
        .contains("Septemberrechnung"));

    w.set_now(at(2026, 10, 9, 9, 0));
    assert_eq!(w.tick(), vec![(run.clone(), Done)]);
    assert_eq!(w.toasts().len(), 2);
    assert!(w.toasts()[1]
        .0
        .starts_with("Frist morgen: Die Septemberrechnung"));
    assert_eq!(w.output(&run, "remind")["sent"], 2);
}

#[test]
fn a_far_deadline_sleeps_at_most_six_hours_at_a_time() {
    let w = World::new();
    let wf = remind_workflow(&w, json!({"days_before": 3}));
    let mut ex = standard_extraction(&w.meeting_id);
    ex["deadlines"][0]["due"] = json!("2027-06-01");
    let run = w.start(&wf, ex);
    w.tick();
    assert_eq!(w.next_run_at(&run), Some(at(2026, 10, 1, 14, 0) + 6 * HOUR));
    assert!(w.toasts().is_empty());
}

// -- Verpasst und vorbei ------------------------------------------------------------------------------

#[test]
fn a_missed_reminder_comes_at_once_while_the_deadline_is_ahead_and_never_after_it() {
    // App war aus: der Zeitpunkt (2.10. 09:00) ist vorbei, die Frist (3.10.) steht noch aus.
    let w = World::new();
    w.set_now(at(2026, 10, 2, 18, 0));
    let wf = remind_workflow(&w, json!({"days_before": 1, "at": "09:00"}));
    let run = w.start(&wf, standard_extraction(&w.meeting_id));
    assert_eq!(w.tick(), vec![(run, Done)]);
    assert_eq!(w.toasts().len(), 1);
    assert!(w.toasts()[0].0.starts_with("Frist morgen:"));

    // Am Tag der Frist selbst: „heute“.
    let w = World::new();
    w.set_now(at(2026, 10, 3, 8, 0));
    let wf = remind_workflow(&w, json!({"days_before": 1}));
    w.start(&wf, standard_extraction(&w.meeting_id));
    w.tick();
    assert!(
        w.toasts()[0].0.starts_with("Frist heute:"),
        "{:?}",
        w.toasts()
    );

    // Nach der Frist: keine Mitteilung, der Eintrag steht mit Grund im Ergebnis.
    let w = World::new();
    w.set_now(at(2026, 10, 4, 9, 0));
    let wf = remind_workflow(&w, json!({"days_before": 1}));
    let run = w.start(&wf, standard_extraction(&w.meeting_id));
    assert_eq!(w.tick(), vec![(run.clone(), Done)]);
    assert!(w.toasts().is_empty());
    let out = w.output(&run, "remind");
    assert_eq!(out["skipped"][0]["reason"], "Die Frist ist schon vorbei.");
    assert_eq!(out["skipped"][0]["due"], "2026-10-03");
    assert_eq!(ledger_rows(&w), 0);
}

#[test]
fn days_before_zero_notifies_on_the_day_and_the_title_counts_the_days() {
    let w = World::new();
    let wf = remind_workflow(&w, json!({"days_before": 0, "at": "7:30"}));
    let run = w.start(&wf, standard_extraction(&w.meeting_id));
    w.tick();
    w.set_now(at(2026, 10, 3, 7, 29));
    assert!(w.tick().len() <= 1 && w.toasts().is_empty());
    w.set_now(at(2026, 10, 3, 7, 30));
    assert_eq!(w.tick(), vec![(run, Done)]);
    assert!(w.toasts()[0].0.starts_with("Frist heute:"));

    let w = World::new();
    let wf = remind_workflow(&w, json!({"days_before": 2}));
    w.start(&wf, standard_extraction(&w.meeting_id));
    w.set_now(at(2026, 10, 1, 9, 0));
    w.tick();
    assert!(
        w.toasts()[0].0.starts_with("Frist in 2 Tagen:"),
        "{:?}",
        w.toasts()
    );
}

// -- To-dos und geschaetzte Daten --------------------------------------------------------------------

fn with_dated_todo(w: &World, source: &str) -> Value {
    let mut ex = standard_extraction(&w.meeting_id);
    ex["todos"] = json!([todo(
        "Herr Vogt ruft den Betriebsrat an.",
        "Herr Vogt",
        Some("2026-10-05"),
        Some(source)
    )]);
    ex
}

#[test]
fn to_dos_with_a_date_are_reminded_only_when_asked_for() {
    let w = World::new();
    w.set_now(at(2026, 10, 4, 9, 0));
    let wf = remind_workflow(&w, json!({"days_before": 1}));
    let run = w.start(&wf, with_dated_todo(&w, "angabe:freitag"));
    w.tick();
    assert!(
        w.toasts().is_empty(),
        "die Frist (3.10.) ist vorbei, das To-do nicht erinnert: {:?}",
        w.toasts()
    );
    assert_eq!(w.output(&run, "remind")["reminders"], 1);

    let w = World::new();
    w.set_now(at(2026, 10, 4, 9, 0));
    let wf = remind_workflow(&w, json!({"days_before": 1, "todos": true}));
    w.start(&wf, with_dated_todo(&w, "angabe:freitag"));
    w.tick();
    assert_eq!(w.toasts().len(), 1, "{:?}", w.toasts());
    let (title, body) = &w.toasts()[0];
    assert_eq!(
        title,
        "To-do fällig morgen: Herr Vogt ruft den Betriebsrat an."
    );
    assert!(
        body.contains("Montag, 05.10.2026") && body.contains("Zuständig: Herr Vogt."),
        "{body}"
    );
}

#[test]
fn dates_the_model_only_guessed_are_marked_in_the_notification_or_left_out_on_request() {
    let w = World::new();
    w.set_now(at(2026, 10, 4, 9, 0));
    let wf = remind_workflow(&w, json!({"days_before": 1, "todos": true}));
    w.start(&wf, with_dated_todo(&w, "modell"));
    w.tick();
    let (title, body) = &w.toasts()[0];
    assert!(title.starts_with("To-do fällig morgen:"), "{title}");
    assert!(
        body.contains("Datum vom Sprachmodell geschätzt, bitte prüfen."),
        "{body}"
    );

    let w = World::new();
    w.set_now(at(2026, 10, 4, 9, 0));
    let wf = remind_workflow(
        &w,
        json!({"days_before": 1, "todos": true, "skip_model_dates": true}),
    );
    let mut only_todo = with_dated_todo(&w, "modell");
    only_todo["deadlines"] = json!([]);
    let run = w.start(&wf, only_todo);
    assert_eq!(w.tick(), vec![(run.clone(), Done)]);
    assert!(w.toasts().is_empty());
    assert_eq!(w.output(&run, "remind")["reminders"], 0);

    // Eine gesicherte Frist daneben wird trotzdem erinnert, ohne Kennzeichnung.
    let w = World::new();
    w.set_now(at(2026, 10, 2, 9, 0));
    let wf = remind_workflow(&w, json!({"days_before": 1, "skip_model_dates": true}));
    w.start(&wf, standard_extraction(&w.meeting_id));
    w.tick();
    assert!(!w.toasts()[0].1.contains("geschätzt"));
}

// -- Fehlerfaelle ---------------------------------------------------------------------------------------

fn direct_remind(
    w: &World,
    run_id: &str,
    params: &Value,
    cancel: &AtomicBool,
) -> Result<StepOutput, StepError> {
    let context = json!({"trigger": {}, "steps": {"extract": standard_extraction(&w.meeting_id)}});
    let clock: &dyn Clock = &*w.clock;
    let ctx = RunCtx {
        workflow_id: "wf-test",
        run_id,
        step_id: "remind",
        attempt: 1,
        idempotency_key: format!("{run_id}:remind"),
        context: &context,
        step_started_at: T0,
        approved: false,
        cancel,
        clock,
        db_path: &w.fx.db_path,
    };
    DeadlineRemind::new(w.svc.clone()).run(&ctx, params)
}

#[test]
fn a_failing_notification_service_is_transient_leaves_no_entry_and_the_retry_shows_it_once() {
    let w = World::new();
    w.set_now(at(2026, 10, 2, 9, 0));
    let cancel = AtomicBool::new(false);
    let params = json!({"days_before": 1});
    w.svc.notify_fails.store(true, Ordering::SeqCst);
    let r = direct_remind(&w, "R1", &params, &cancel);
    assert!(
        matches!(&r, Err(StepError::Transient(m)) if m.contains("Mitteilungsdienst")),
        "{r:?}"
    );
    assert_eq!(
        ledger_rows(&w),
        0,
        "nichts vermerkt, ein neuer Versuch ist sicher"
    );
    w.svc.notify_fails.store(false, Ordering::SeqCst);
    direct_remind(&w, "R1", &params, &cancel).unwrap();
    direct_remind(&w, "R1", &params, &cancel).unwrap();
    assert_eq!(
        w.toasts().len(),
        1,
        "auch eine Wiederholung nach einem Absturz zeigt nichts doppelt"
    );
}

#[test]
fn a_full_disk_after_the_toast_is_transient_with_a_clear_text_and_the_ledger_is_checked_first() {
    let w = World::new();
    w.set_now(at(2026, 10, 2, 9, 0));
    let cancel = AtomicBool::new(false);
    // Lesen geht, Schreiben nicht (Platte voll / gesperrt).
    w.conn()
        .execute_batch(
            "CREATE TRIGGER c4_block BEFORE INSERT ON provenance BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
        )
        .unwrap();
    let r = direct_remind(&w, "R2", &json!({"days_before": 1}), &cancel);
    match &r {
        Err(StepError::Transient(m)) => assert!(
            m.contains("Herkunftsregister") && m.contains("sicher"),
            "{m}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.toasts().len(), 1, "die Mitteilung wurde gezeigt");
    // Eine kaputte Tabelle (Lesen scheitert) zeigt gar nichts.
    let w = World::new();
    w.set_now(at(2026, 10, 2, 9, 0));
    w.conn().execute("DROP TABLE provenance", []).unwrap();
    let r = direct_remind(&w, "R3", &json!({"days_before": 1}), &cancel);
    assert!(matches!(&r, Err(StepError::Transient(_))), "{r:?}");
    assert!(w.toasts().is_empty());
}

#[test]
fn a_cancelled_run_shows_nothing() {
    let w = World::new();
    w.set_now(at(2026, 10, 2, 9, 0));
    let cancel = AtomicBool::new(true);
    let r = direct_remind(&w, "R4", &json!({"days_before": 1}), &cancel);
    assert!(
        matches!(&r, Err(StepError::Transient(m)) if m.contains("abgebrochen")),
        "{r:?}"
    );
    assert!(w.toasts().is_empty());
}

#[test]
fn an_extraction_without_result_is_a_quiet_success_and_a_missing_step_a_permanent_error() {
    let w = World::new();
    let wf = remind_workflow(&w, json!({}));
    let mut none = extraction(&w.meeting_id, vec![], vec![], vec![]);
    none["outcome"] = json!("no_action");
    let run = w.start(&wf, none);
    assert_eq!(w.tick(), vec![(run.clone(), Done)]);
    assert_eq!(w.output(&run, "remind")["reminders"], 0);
    assert!(w.toasts().is_empty());

    let wf = w.workflow(vec![step(
        "remind",
        "deadline.remind",
        json!({"from": "nirgends"}),
    )]);
    let run = w.enqueue(&wf);
    w.tick();
    assert_eq!(w.state(&run, "remind"), StepState::Failed);
}

// -- Parameter, Zeitrechnung --------------------------------------------------------------------------

#[test]
fn the_time_of_a_reminder_is_days_before_at_local_time() {
    let due = chrono::NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
    let lead = |d, h, m| deadlines::Lead {
        days_before: d,
        hour: h,
        minute: m,
    };
    assert_eq!(
        deadlines::fire_at_ms(due, &lead(1, 9, 0)),
        Some(at(2026, 10, 2, 9, 0))
    );
    assert_eq!(
        deadlines::fire_at_ms(due, &lead(0, 7, 30)),
        Some(at(2026, 10, 3, 7, 30))
    );
    assert_eq!(
        deadlines::fire_at_ms(due, &lead(30, 23, 59)),
        Some(at(2026, 9, 3, 23, 59))
    );
    assert_eq!(
        deadlines::local_day(at(2026, 10, 1, 0, 0)),
        chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
    );
    assert_eq!(
        deadlines::local_day(at(2026, 10, 1, 23, 59)),
        chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
    );
    assert_eq!(deadlines::parse_at("09:00"), Some((9, 0)));
    assert_eq!(deadlines::parse_at(" 9:05 "), Some((9, 5)));
    for bad in [
        "24:00", "09:60", "9", "0900", "09:5", "ab:cd", "", "-1:00", "009:00",
    ] {
        assert_eq!(deadlines::parse_at(bad), None, "{bad}");
    }
}

#[test]
fn parameters_are_checked_when_saving_and_described_with_their_defaults() {
    let w = World::new();
    let save = |p: Value| {
        w.engine
            .save_workflow(None, &def(vec![step("r", "deadline.remind", p)]))
    };
    assert!(save(json!({})).is_ok());
    assert!(save(
        json!({"days_before": 2, "at": "17:45", "todos": true, "skip_model_dates": true})
    )
    .is_ok());
    for bad in [
        json!({"at": "25:00"}),
        json!({"at": "9"}),
        json!({"days_before": 31}),
        json!({"days_before": -1}),
        json!({"from": "A B"}),
        json!({"from": "{{trigger.x}}"}),
    ] {
        assert!(save(bad.clone()).is_err(), "{bad}");
    }
    let a = w.engine.registry().get("deadline.remind").unwrap().clone();
    assert_eq!(
        a.describe(&json!({})),
        "Zu den Fristen aus Schritt „extract“ einen Tag vor der Frist um 09:00 Uhr eine Windows-Mitteilung anzeigen; der Lauf wartet bis dahin"
    );
    let text = a.describe(
        &json!({"days_before": 3, "at": "08:15", "todos": true, "skip_model_dates": true}),
    );
    assert!(
        text.contains("Fristen und To-dos mit Datum")
            && text.contains("3 Tage vor der Frist um 08:15 Uhr")
            && text.contains("ohne vom Sprachmodell geschätzte Daten"),
        "{text}"
    );
    assert!(a.needs(&json!({})).unwrap().is_none(), "kein Recht noetig");
    assert_eq!(a.effect(), EffectKind::Idempotent);
}
