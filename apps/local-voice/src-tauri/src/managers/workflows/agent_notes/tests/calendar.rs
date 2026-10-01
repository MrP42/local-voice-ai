//! `deadline.calendar`: ganztaegiger Termin je Frist, nur nach Freigabe (AK7).

use super::*;
use crate::managers::integrations::m365::event_create::{request_body, NewAllDayEvent};
use crate::managers::integrations::m365::M365Error;
use crate::managers::workflows::engine::RunOutcome::Done;

fn calendar_workflow(w: &World, params: Value) -> String {
    w.workflow(vec![
        step("extract", "agent.extract", json!({})),
        step("cal", "deadline.calendar", params),
    ])
}

fn graph(w: &World) -> &Graph {
    w.graph.as_ref().unwrap()
}

/// Bis zur Freigabe laufen lassen und sie erteilen; gibt den Vorschautext zurueck.
fn approve_next(w: &World, run: &str) -> String {
    assert_eq!(
        w.tick(),
        vec![(run.to_string(), RunOutcome::AwaitingApproval)]
    );
    let p = w.pending();
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!(p[0].tool_or_capability, "calendar.write");
    let preview = p[0].args_preview.clone().unwrap();
    w.approve(&p[0].id);
    preview
}

#[test]
fn ak7_a_calendar_entry_is_made_only_after_approval_even_when_the_right_is_set_to_allowed() {
    let w = World::new().with_graph();
    // „erlaubt“ eingestellt: der Baustein verlangt trotzdem die Freigabe (`max_mode`).
    w.grant("m365-1", Capability::CalendarWrite, GrantMode::Allow);
    let wf = calendar_workflow(&w, json!({"via": "m365-1"}));
    let run = w.start(&wf, standard_extraction(&w.meeting_id));

    assert_eq!(w.tick(), vec![(run.clone(), RunOutcome::AwaitingApproval)]);
    assert!(
        graph(&w).created().is_empty(),
        "vor der Freigabe geht nichts an den Kalender"
    );
    let p = w.pending();
    assert_eq!(p.len(), 1);
    let preview = p[0].args_preview.clone().unwrap();
    assert!(
        preview.contains("Ziel: Kalender „Mein Kalender“: 1 ganztägige(r) Eintrag/Einträge"),
        "{preview}"
    );
    assert!(
        preview.contains("03.10.2026: Frist: Das Angebot geht an die Stadtwerke."),
        "{preview}"
    );

    w.approve(&p[0].id);
    assert_eq!(w.tick(), vec![(run.clone(), Done)]);
    let sent = graph(&w).created();
    assert_eq!(sent.len(), 1);
    let body = sent[0].json();
    assert_eq!(
        body["subject"],
        "Frist: Das Angebot geht an die Stadtwerke."
    );
    assert_eq!(body["isAllDay"], true);
    assert_eq!(
        body["start"],
        json!({"dateTime": "2026-10-03T00:00:00", "timeZone": "UTC"})
    );
    assert_eq!(
        body["end"],
        json!({"dateTime": "2026-10-04T00:00:00", "timeZone": "UTC"})
    );
    assert_eq!(body["showAs"], "free");
    assert!(
        body.get("attendees").is_none(),
        "es geht keine Einladung an Dritte"
    );
    assert_eq!(body["body"]["contentType"], "text");
    let text = body["body"]["content"].as_str().unwrap();
    assert!(
        text.contains(&format!(
            "Frist aus der Besprechung „{TITLE}“ vom 01.10.2026."
        )),
        "{text}"
    );
    assert!(
        text.contains("Datum aus der Angabe „bis übermorgen“."),
        "{text}"
    );
    assert!(
        text.contains("Beleg: „Ich schicke es bis übermorgen an den Kunden.“"),
        "{text}"
    );
    let tx = body["transactionId"].as_str().unwrap();
    assert_eq!(tx.len(), 32);
    assert!(tx.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(sent[0].bearer(), Some("AT-1"));
    let out = w.output(&run, "cal");
    assert_eq!(out["created"], 1);
    assert_eq!(out["events"][0]["due"], "2026-10-03");
}

#[test]
fn a_second_run_for_the_same_meeting_makes_no_second_entry() {
    let w = World::new().with_graph();
    let wf = calendar_workflow(&w, json!({"via": "m365-1"}));
    let first = w.start(&wf, standard_extraction(&w.meeting_id));
    approve_next(&w, &first);
    assert_eq!(w.tick(), vec![(first, Done)]);
    assert_eq!(graph(&w).created().len(), 1);

    let second = w.start(&wf, standard_extraction(&w.meeting_id));
    let preview = approve_next(&w, &second);
    assert!(
        preview.contains("0 ganztägige(r) Eintrag/Einträge"),
        "{preview}"
    );
    assert!(preview.contains("schon_eingetragen: 1"), "{preview}");
    assert_eq!(w.tick(), vec![(second.clone(), Done)]);
    assert_eq!(graph(&w).created().len(), 1, "kein zweiter Termin");
    assert_eq!(w.output(&second, "cal")["already_there"], 1);
}

#[test]
fn a_switched_off_calendar_denies_the_step_and_sends_nothing() {
    let w = World::new().with_graph();
    w.grant("m365-1", Capability::CalendarWrite, GrantMode::Off);
    let wf = calendar_workflow(&w, json!({"via": "m365-1"}));
    let run = w.start(&wf, standard_extraction(&w.meeting_id));
    w.tick();
    assert_eq!(w.state(&run, "cal"), StepState::Denied);
    assert!(graph(&w).created().is_empty());
}

#[test]
fn guessed_dates_are_marked_in_the_approval_and_the_event_or_left_out_on_request() {
    let w = World::new().with_graph();
    let wf = calendar_workflow(&w, json!({"via": "m365-1"}));
    let mut ex = standard_extraction(&w.meeting_id);
    ex["deadlines"][0]["due_source"] = json!("modell");
    let run = w.start(&wf, ex.clone());
    let preview = approve_next(&w, &run);
    assert!(
        preview.contains("Frist (Datum geschätzt): Das Angebot"),
        "{preview}"
    );
    w.tick();
    let sent = graph(&w).created();
    assert_eq!(
        sent[0].json()["subject"],
        "Frist (Datum geschätzt): Das Angebot geht an die Stadtwerke."
    );
    assert!(sent[0].json()["body"]["content"]
        .as_str()
        .unwrap()
        .contains("Das Datum hat das Sprachmodell geschätzt. Bitte prüfen."));

    let w = World::new().with_graph();
    let wf = calendar_workflow(&w, json!({"via": "m365-1", "skip_model_dates": true}));
    let run = w.start(&wf, ex);
    let preview = approve_next(&w, &run);
    assert!(preview.contains("0 ganztägige(r)"), "{preview}");
    w.tick();
    assert!(graph(&w).created().is_empty());
}

#[test]
fn deadlines_in_the_past_get_no_entry_and_at_most_ten_are_proposed_in_one_step() {
    let w = World::new().with_graph();
    w.set_now(at(2026, 10, 4, 9, 0));
    let wf = calendar_workflow(&w, json!({"via": "m365-1"}));
    let run = w.start(&wf, standard_extraction(&w.meeting_id));
    let preview = approve_next(&w, &run);
    assert!(preview.contains("0 ganztägige(r)"), "{preview}");
    w.tick();
    assert!(graph(&w).created().is_empty());

    let w = World::new().with_graph();
    let wf = calendar_workflow(&w, json!({"via": "m365-1"}));
    let many: Vec<Value> = (0..12)
        .map(|i| {
            deadline(
                &format!("Frist Nummer {i:02}"),
                &format!("2026-11-{:02}", i + 1),
                "angabe:x",
                &[i],
                "q",
            )
        })
        .collect();
    let run = w.start(&wf, extraction(&w.meeting_id, vec![], many, vec![]));
    let preview = approve_next(&w, &run);
    assert!(preview.contains("10 ganztägige(r)"), "{preview}");
    assert!(
        preview.contains("nicht_eingetragen_wegen_Obergrenze: 2"),
        "{preview}"
    );
    w.tick();
    assert_eq!(graph(&w).created().len(), 10);
    assert_eq!(w.output(&run, "cal")["not_entered_over_limit"], 2);
}

#[test]
fn to_dos_with_a_date_are_entered_only_when_asked_for() {
    let w = World::new().with_graph();
    let wf = calendar_workflow(&w, json!({"via": "m365-1", "todos": true}));
    let mut ex = standard_extraction(&w.meeting_id);
    ex["todos"] = json!([todo(
        "Herr Vogt ruft den Betriebsrat an.",
        "Herr Vogt",
        Some("2026-10-05"),
        Some("angabe:freitag")
    )]);
    let run = w.start(&wf, ex);
    let preview = approve_next(&w, &run);
    assert!(preview.contains("2 ganztägige(r)"), "{preview}");
    assert!(
        preview.contains("05.10.2026: To-do fällig: Herr Vogt ruft den Betriebsrat an."),
        "{preview}"
    );
    w.tick();
    let subjects: Vec<String> = graph(&w)
        .created()
        .iter()
        .map(|r| r.json()["subject"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(subjects.len(), 2, "{subjects:?}");
}

// -- Fehlerfaelle -----------------------------------------------------------------------------------------

fn ledger_rows(w: &World) -> i64 {
    w.conn()
        .query_row(
            "SELECT COUNT(*) FROM provenance WHERE operation = 'deadline_event_created'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn graph_refusal_is_permanent_and_a_server_error_is_uncertain_and_never_repeated_by_itself() {
    // 403: der Mandant lehnt ab -> Schritt gescheitert, nichts vermerkt.
    let w = World::new().with_graph();
    *graph(&w).mode.lock().unwrap() = GraphMode::Forbidden;
    let wf = calendar_workflow(&w, json!({"via": "m365-1"}));
    let run = w.start(&wf, standard_extraction(&w.meeting_id));
    approve_next(&w, &run);
    w.tick();
    assert_eq!(w.state(&run, "cal"), StepState::Failed);
    assert!(w
        .row(&run, "cal")
        .error
        .unwrap()
        .contains("Zugriff verweigert"));
    assert_eq!(ledger_rows(&w), 0);

    // 500 und abgerissene Verbindung: es ist unklar, ob der Termin entstand.
    for mode in [GraphMode::Status500, GraphMode::Drop] {
        let w = World::new().with_graph();
        *graph(&w).mode.lock().unwrap() = mode;
        let wf = calendar_workflow(&w, json!({"via": "m365-1"}));
        let run = w.start(&wf, standard_extraction(&w.meeting_id));
        approve_next(&w, &run);
        w.tick();
        assert_eq!(w.state(&run, "cal"), StepState::Uncertain, "{mode:?}");
        assert_eq!(graph(&w).created().len(), 1, "genau ein Versuch: {mode:?}");
        // Weitere Takte versuchen es nicht erneut.
        w.set_now(w.clock.now_ms() + 3_600_000);
        w.tick();
        assert_eq!(
            graph(&w).created().len(),
            1,
            "nie von selbst wiederholt: {mode:?}"
        );
    }
}

fn direct_calendar(w: &World, run_id: &str, params: &Value) -> Result<StepOutput, StepError> {
    let context = json!({"trigger": {}, "steps": {"extract": standard_extraction(&w.meeting_id)}});
    let cancel = AtomicBool::new(false);
    let clock: &dyn Clock = &*w.clock;
    let ctx = RunCtx {
        workflow_id: "wf-test",
        run_id,
        step_id: "cal",
        attempt: 1,
        idempotency_key: format!("{run_id}:cal"),
        context: &context,
        step_started_at: T0,
        approved: true,
        gate_args: None,
        cancel: &cancel,
        clock,
        db_path: &w.fx.db_path,
    };
    let action = DeadlineCalendar::new(w.svc.clone());
    let first = action.run(&ctx, params);
    // `confirm` dasselbe Mal: nach einem Erfolg belegt der Vermerk die Wirkung.
    if first.is_ok() {
        assert!(
            action.confirm(&ctx, params).is_some(),
            "confirm belegt aus dem Vermerk"
        );
    } else {
        assert!(
            action.confirm(&ctx, params).is_none(),
            "ohne Vermerk nichts zu belegen"
        );
    }
    first
}

#[test]
fn after_a_crash_confirm_proves_the_event_from_the_ledger_and_the_repeat_adds_nothing() {
    let w = World::new().with_graph();
    let params = json!({"via": "m365-1"});
    let first = direct_calendar(&w, "R1", &params).unwrap();
    assert_eq!(first.data["created"], 1);
    // Wiederholung (anderer Lauf, derselbe Vermerk): kein zweiter Termin.
    let again = direct_calendar(&w, "R2", &params).unwrap();
    assert_eq!(again.data["created"], 0);
    assert_eq!(again.data["already_there"], 1);
    assert_eq!(graph(&w).created().len(), 1);
}

#[test]
fn a_ledger_that_cannot_be_written_after_the_event_is_transient_and_the_retry_uses_the_same_transaction(
) {
    let w = World::new().with_graph();
    w.conn()
        .execute_batch("CREATE TRIGGER c4_block2 BEFORE INSERT ON provenance BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;")
        .unwrap();
    let r = direct_calendar(&w, "R3", &json!({"via": "m365-1"}));
    assert!(
        matches!(&r, Err(StepError::Transient(m)) if m.contains("Herkunftsregister")),
        "{r:?}"
    );
    // Wiederholung desselben Schritts: dieselbe transactionId, Graph erkennt den Termin wieder.
    let _ = direct_calendar(&w, "R3", &json!({"via": "m365-1"}));
    let ids: Vec<String> = graph(&w)
        .created()
        .iter()
        .map(|r| r.json()["transactionId"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1], "stabile Kennung je Lauf, Schritt und Frist");
}

#[test]
fn a_wrong_integration_or_a_missing_account_service_fails_with_a_clear_text() {
    let w = World::new().with_graph();
    let r = direct_calendar(&w, "R4", &json!({"via": "vault-1"}));
    assert!(
        matches!(&r, Err(StepError::Permanent(m)) if m.contains("kein Microsoft-365-Konto")),
        "{r:?}"
    );
    *w.svc.m365.lock().unwrap() = None;
    let r = direct_calendar(&w, "R5", &json!({"via": "m365-1"}));
    assert!(matches!(&r, Err(StepError::NotAvailable(_))), "{r:?}");
    assert!(graph(&w).created().is_empty());
}

#[test]
fn the_plan_of_the_block_asks_always_and_says_the_list_is_known_only_at_run_time() {
    let w = World::new().with_graph();
    w.grant("m365-1", Capability::CalendarWrite, GrantMode::Allow);
    let def = crate::managers::workflows::validate::parse_definition(&def(vec![
        step("extract", "agent.extract", json!({})),
        step("cal", "deadline.calendar", json!({"via": "m365-1"})),
    ]))
    .unwrap();
    let plan = crate::managers::workflows::plan::plan_definition(
        &w.conn(),
        &w.engine.registry(),
        &def,
        None,
    );
    let perm = &plan["steps"][1]["permission"];
    assert_eq!(perm["result"], "needs_approval", "{perm}");
    assert_eq!(perm["capped_to"], "ask", "{perm}");
    assert!(perm["preview"].as_str().unwrap().contains("erst beim Lauf"));
    assert!(graph(&w).created().is_empty());
}

// -- Anfrage an Graph (ohne Netz) ------------------------------------------------------------------------

#[test]
fn the_request_body_is_all_day_plain_text_without_attendees_and_validated() {
    let d = |y, m, dd| chrono::NaiveDate::from_ymd_opt(y, m, dd).unwrap();
    let ev = NewAllDayEvent {
        subject: "Frist: A\nB",
        date: d(2026, 10, 31),
        body: "x",
        transaction_id: "abc-DEF_123",
    };
    let b = request_body(&ev).unwrap();
    assert_eq!(b["subject"], "Frist: A B");
    assert_eq!(b["start"]["dateTime"], "2026-10-31T00:00:00");
    assert_eq!(b["end"]["dateTime"], "2026-11-01T00:00:00", "Monatsende");
    let ev = NewAllDayEvent {
        date: d(2026, 12, 31),
        ..ev.clone()
    };
    assert_eq!(
        request_body(&ev).unwrap()["end"]["dateTime"],
        "2027-01-01T00:00:00",
        "Jahresende"
    );
    for bad_tx in ["", "a b", "a/b", &"x".repeat(121)] {
        let ev = NewAllDayEvent {
            transaction_id: bad_tx,
            ..ev.clone()
        };
        assert!(
            matches!(request_body(&ev), Err(M365Error::Invalid(_))),
            "{bad_tx}"
        );
    }
    let ev2 = NewAllDayEvent {
        subject: "  \n ",
        ..ev.clone()
    };
    assert!(matches!(request_body(&ev2), Err(M365Error::Invalid(_))));
    let long = "y".repeat(20_001);
    let ev3 = NewAllDayEvent {
        body: &long,
        ..ev.clone()
    };
    assert!(matches!(request_body(&ev3), Err(M365Error::Invalid(_))));
    let long_subject = "z".repeat(400);
    let ev4 = NewAllDayEvent {
        subject: &long_subject,
        ..ev
    };
    assert_eq!(
        request_body(&ev4).unwrap()["subject"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        255
    );
}
