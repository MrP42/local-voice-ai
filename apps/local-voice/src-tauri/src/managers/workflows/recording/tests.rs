//! Tests der Bausteine `recording.start`/`recording.stop`, des Traegers und des
//! Einwilligungswegs (B2, AK4): ohne Klick keine Aufnahme, nach dem Klick geht der Lauf
//! weiter, jede Sperre einzeln.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::*;
use crate::managers::integrations::approvals;
use crate::managers::integrations::audit::{self, AuditFilter};
use crate::managers::integrations::model::{ApprovalState, Caller, Capability, GrantMode, Kind};
use crate::managers::integrations::store as istore;
use crate::managers::workflows::engine::{
    Clock, CrashPoint, Engine, EnqueueRequest, RunOutcome, TickReport,
};
use crate::managers::workflows::model::{Origin, RunState, StepState};
use crate::managers::workflows::store::{self, RunRow, StepRow, WorkflowError};
use crate::managers::workflows::test_support::{
    armed_workflow, engine, engine_with, roomy_gate, set_grant, step, FakeClock, Fx, Scripted, T0,
};

const MIN: i64 = 60_000;

// ---------------------------------------------------------------------------
// Attrappe des Recorders
// ---------------------------------------------------------------------------

struct FakeRecorder {
    clock: Arc<FakeClock>,
    state: Mutex<FakeState>,
}

#[derive(Default)]
struct FakeState {
    current: Option<(String, i64)>,
    starts: Vec<StartRequest>,
    stops: usize,
    fail_start: Option<String>,
    counter: usize,
}

impl FakeRecorder {
    fn new(clock: &Arc<FakeClock>) -> Arc<Self> {
        Arc::new(Self {
            clock: clock.clone(),
            state: Mutex::new(FakeState::default()),
        })
    }

    fn start_calls(&self) -> usize {
        self.state.lock().unwrap().starts.len()
    }

    fn starts(&self) -> Vec<StartRequest> {
        self.state.lock().unwrap().starts.clone()
    }

    fn stop_calls(&self) -> usize {
        self.state.lock().unwrap().stops
    }

    fn fail_next_start(&self, code: &str) {
        self.state.lock().unwrap().fail_start = Some(code.to_string());
    }

    /// Eine Aufnahme laeuft schon (vom Nutzer gestartet).
    fn already_recording(&self, meeting_id: &str) {
        self.state.lock().unwrap().current = Some((meeting_id.to_string(), self.clock.now_ms()));
    }

    /// Die Aufnahme wurde beendet (vom Nutzer), ohne dass der Baustein es wusste.
    fn user_stops(&self) {
        self.state.lock().unwrap().current = None;
    }
}

impl RecordingControl for FakeRecorder {
    fn current(&self) -> Option<CurrentRecording> {
        self.state
            .lock()
            .unwrap()
            .current
            .clone()
            .map(|(id, at)| CurrentRecording {
                meeting_id: Some(id),
                started_at_ms: Some(at),
            })
    }

    fn start(&self, req: &StartRequest) -> Result<StartedRecording, String> {
        let mut s = self.state.lock().unwrap();
        if let Some(code) = s.fail_start.take() {
            return Err(code);
        }
        if s.current.is_some() {
            return Err("already_recording".to_string());
        }
        s.starts.push(req.clone());
        s.counter += 1;
        let id = format!("m{}", s.counter);
        s.current = Some((id.clone(), self.clock.now_ms()));
        Ok(StartedRecording {
            meeting_id: id,
            title: req.title.clone(),
        })
    }

    fn stop(&self) -> Result<String, String> {
        let mut s = self.state.lock().unwrap();
        match s.current.take() {
            Some((id, _)) => {
                s.stops += 1;
                Ok(id)
            }
            None => Err("not_recording".to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Aufbau
// ---------------------------------------------------------------------------

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
    rec: Arc<FakeRecorder>,
    stops: Arc<StopSchedule>,
    /// Ersetzt den (noch nicht gebauten) Baustein `notify.local` als Folgeschritt.
    notify: Arc<Scripted>,
}

fn world() -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    ensure_carrier(&fx.conn(), T0).unwrap();
    let rec = FakeRecorder::new(&clock);
    let stops = Arc::new(StopSchedule::default());
    install(&engine, rec.clone(), stops.clone());
    let notify = Scripted::new("notify.local", EffectKind::Idempotent);
    engine.register_action(notify.clone());
    World {
        fx,
        clock,
        engine,
        rec,
        stops,
        notify,
    }
}

impl World {
    fn conn(&self) -> rusqlite::Connection {
        self.fx.conn()
    }

    /// Ein zweiter Prozess auf derselben Datenbank, mit demselben Recorder.
    fn restart(&self) -> Engine {
        let e = engine_with(&self.fx, &self.clock, roomy_gate());
        install(&e, self.rec.clone(), self.stops.clone());
        e.register_action(self.notify.clone());
        e
    }

    fn run(&self, id: &str) -> RunRow {
        store::get_run(&self.conn(), id).unwrap().unwrap()
    }

    fn steps(&self, id: &str) -> Vec<StepRow> {
        store::steps_for(&self.conn(), id).unwrap()
    }

    fn audit(&self) -> Vec<crate::managers::integrations::model::AuditEntry> {
        audit::list(&self.conn(), &AuditFilter::default(), 100).unwrap()
    }

    fn pending(&self) -> Vec<crate::managers::integrations::model::Approval> {
        approvals::list_pending(&self.conn(), self.clock.now_ms()).unwrap()
    }

    /// Der Lauf eines Termins, der um `T0 + 1 min` beginnt und `end_min` Minuten nach `T0`
    /// endet, fuer den scharfen Ablauf `wf`.
    fn trigger_event(&self, wf: &str, key: &str, end_min: i64) -> String {
        let data = json!({
            "calendar": "cal-1",
            "event_id": format!("cal-1:uid:{key}"),
            "title": "Jour fixe Vertrieb",
            "start": iso(T0 + MIN),
            "end": iso(T0 + end_min * MIN),
            "attendees": [],
            "external_attendees": 0,
            "online": false,
            "meeting": {"title": "Jour fixe Vertrieb"}
        });
        self.engine
            .enqueue(&EnqueueRequest {
                workflow_id: wf.to_string(),
                trigger_key: format!("calendar_start:{key}"),
                origin: Origin::Trigger,
                trigger: data,
                vars: Default::default(),
                force_dry_run: false,
            })
            .unwrap()
            .run_id
    }

    fn tick(&self) -> TickReport {
        self.engine.tick().unwrap()
    }
}

fn rec_def(extra: Vec<Value>, stop: &str) -> Value {
    let mut steps = vec![step(
        "rec",
        "recording.start",
        // Der Titel steht in der Vorschau der Freigabe (`Ziel: ...`).
        json!({"via": CARRIER_ID, "stop": stop, "title": "{{trigger.title}}"}),
    )];
    steps.extend(extra);
    json!({
        "schema": "lva-workflow@1",
        "name": "Aufnahme-Test",
        "trigger": {"type": "calendar.event_starting", "integration": "cal-1", "lead_min": 1},
        "steps": steps
    })
}

fn outcome_of(report: &TickReport, run: &str) -> RunOutcome {
    report
        .outcomes
        .iter()
        .find(|(id, _)| id == run)
        .unwrap_or_else(|| panic!("kein Ergebnis fuer {run}: {report:?}"))
        .1
        .clone()
}

// ---------------------------------------------------------------------------
// AK4: ohne Bestaetigung keine Aufnahme, nach dem Klick geht der Lauf weiter
// ---------------------------------------------------------------------------

#[test]
fn without_a_click_no_recording_starts_and_the_run_waits_for_the_approval() {
    let w = world();
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 61);

    let report = w.tick();
    assert_eq!(outcome_of(&report, &run), RunOutcome::AwaitingApproval);
    assert_eq!(w.run(&run).state, RunState::AwaitingApproval);
    assert_eq!(w.rec.start_calls(), 0, "ohne Bestaetigung keine Aufnahme");

    // Eine halbe Stunde und viele Takte spaeter: dasselbe.
    for _ in 0..120 {
        w.clock.advance(15_000);
        w.tick();
    }
    assert_eq!(w.rec.start_calls(), 0);
    assert_eq!(w.run(&run).state, RunState::AwaitingApproval);
    assert_eq!(
        w.pending().len(),
        1,
        "eine einzige offene Bitte, keine Flut"
    );
    let p = &w.pending()[0];
    assert_eq!(p.caller, "workflow");
    assert_eq!(p.tool_or_capability, "recording.start");
    assert_eq!(p.integration_id.as_deref(), Some(CARRIER_ID));
    let preview = p.args_preview.as_deref().unwrap();
    assert!(
        preview.contains("Jour fixe Vertrieb"),
        "die Vorschau nennt, was aufgenommen wird: {preview}"
    );
}

#[test]
fn after_the_click_the_run_continues_and_the_recording_starts_exactly_once() {
    let w = world();
    let wf = armed_workflow(
        &w.engine,
        &rec_def(
            vec![step(
                "danach",
                "notify.local",
                json!({"title": "Aufnahme {{steps.rec.meeting_id}} laeuft"}),
            )],
            "event_end",
        ),
    );
    let run = w.trigger_event(&wf, "t1", 61);
    w.tick();
    assert_eq!(w.rec.start_calls(), 0);

    // Der Klick im Hinweisfenster.
    let approval = w.pending()[0].id.clone();
    crate::managers::workflows::consent::decide(&w.conn(), &approval, true, true, w.clock.now_ms())
        .unwrap();
    assert_eq!(
        w.rec.start_calls(),
        0,
        "die Entscheidung allein startet nichts"
    );

    let report = w.tick();
    assert_eq!(report.approvals_released, 1);
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Done,
        "{report:?} / {:?}",
        w.steps(&run)
    );
    assert_eq!(w.rec.start_calls(), 1, "genau eine Aufnahme");
    let start = &w.rec.starts()[0];
    assert_eq!(start.title, "Jour fixe Vertrieb", "Titel aus dem Termin");
    assert_eq!(start.event_key.as_deref(), Some("cal-1:uid:t1"));
    let rows = w.steps(&run);
    assert_eq!(rows[0].state, StepState::Done);
    let out: Value = serde_json::from_str(rows[0].output_json.as_deref().unwrap()).unwrap();
    assert_eq!(out["meeting_id"], "m1");
    assert_eq!(out["meeting"]["title"], "Jour fixe Vertrieb");
    assert_eq!(rows[1].state, StepState::Done, "der Folgeschritt lief");
    assert_eq!(
        w.notify.calls()[0].params["title"],
        "Aufnahme m1 laeuft",
        "die Kennung der Besprechung steht im Ergebnis des Schritts"
    );
    assert_eq!(
        approvals::get(&w.conn(), &approval).unwrap().unwrap().state,
        ApprovalState::Used,
        "die Freigabe ist eingeloest"
    );
    // Weitere Takte aendern nichts.
    for _ in 0..3 {
        w.tick();
    }
    assert_eq!(w.rec.start_calls(), 1);
}

#[test]
fn a_refused_request_never_starts_a_recording() {
    let w = world();
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 61);
    w.tick();
    let approval = w.pending()[0].id.clone();
    crate::managers::workflows::consent::decide(&w.conn(), &approval, false, true, w.clock.now_ms())
        .unwrap();
    let report = w.tick();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "denied".into()
        }
    );
    assert_eq!(w.rec.start_calls(), 0);
    assert_eq!(w.steps(&run)[0].state, StepState::Denied);
    assert!(w.pending().is_empty());
}

#[test]
fn an_unanswered_request_expires_after_an_hour_and_never_records() {
    let w = world();
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 300);
    w.tick();
    w.clock.advance(approvals::TTL_MS + 1_000);
    let report = w.tick();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "approval_expired".into()
        }
    );
    assert_eq!(w.rec.start_calls(), 0);
}

#[test]
fn a_new_workflow_is_a_dry_run_and_neither_asks_nor_records() {
    let w = world();
    let row = w
        .engine
        .save_workflow(None, &rec_def(vec![], "event_end"))
        .unwrap();
    w.engine.set_enabled(&row.id, true).unwrap(); // eingeschaltet, aber nicht scharf
    let run = w.trigger_event(&row.id, "t1", 61);
    let report = w.tick();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    assert!(w.run(&run).dry_run);
    assert_eq!(w.steps(&run)[0].state, StepState::Planned);
    assert!(w.pending().is_empty(), "ein Trockenlauf bittet um nichts");
    assert_eq!(w.rec.start_calls(), 0);
}

#[test]
fn a_grant_set_to_off_denies_without_any_prompt() {
    let w = world();
    set_grant(
        &w.conn(),
        CARRIER_ID,
        Capability::RecordingStart,
        GrantMode::Off,
    );
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 61);
    let report = w.tick();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "denied".into()
        }
    );
    assert!(w.pending().is_empty(), "bei „aus“ gibt es nichts zu fragen");
    assert_eq!(w.rec.start_calls(), 0);
    assert!(
        w.audit()
            .iter()
            .any(|a| a.outcome == "denied" && a.capability.as_deref() == Some("recording.start")),
        "die Ablehnung steht im Audit"
    );
}

#[test]
fn the_register_refuses_to_store_allow_for_a_recording() {
    let w = world();
    let err = istore::set_grant(
        &w.conn(),
        CARRIER_ID,
        Capability::RecordingStart,
        Caller::Workflow,
        GrantMode::Allow,
    )
    .unwrap_err();
    assert!(err.to_string().contains("nie dauerhaft erlaubt"), "{err}");
}

#[test]
fn even_a_damaged_grant_row_saying_allow_still_asks_for_a_recording() {
    let w = world();
    // Die Schnittstelle verweigert „erlaubt“; hier steht es trotzdem in der Tabelle
    // (beschaedigte oder von Hand geaenderte Daten): die Regel macht daraus „fragen“.
    w.conn()
        .execute(
            "INSERT INTO integration_grants (integration_id, capability, caller, mode)
             VALUES (?1, 'recording.start', 'workflow', 'allow')",
            rusqlite::params![CARRIER_ID],
        )
        .unwrap();
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 61);
    let report = w.tick();
    assert_eq!(outcome_of(&report, &run), RunOutcome::AwaitingApproval);
    assert_eq!(w.rec.start_calls(), 0);
}

#[test]
fn the_action_refuses_to_run_without_a_redeemed_approval_even_when_called_directly() {
    let w = world();
    let action = RecordingStart::new(w.rec.clone(), w.stops.clone());
    let cancel = AtomicBool::new(false);
    let ctx_value = json!({"trigger": {"title": "X"}});
    let mk = |approved: bool| RunCtx {
        workflow_id: "wf",
        run_id: "run",
        step_id: "rec",
        attempt: 1,
        idempotency_key: "run:rec".into(),
        context: &ctx_value,
        step_started_at: T0,
        approved,
        cancel: &cancel,
        clock: &*w.clock,
        db_path: &w.fx.db_path,
    };
    let params = json!({"via": CARRIER_ID});
    let err = action.run(&mk(false), &params).unwrap_err();
    assert!(matches!(err, StepError::Denied(_)), "{err:?}");
    assert_eq!(w.rec.start_calls(), 0);
    // Mit der eingeloesten Freigabe laeuft derselbe Aufruf.
    action.run(&mk(true), &params).unwrap();
    assert_eq!(w.rec.start_calls(), 1);
}

#[test]
fn a_late_approval_for_a_meeting_that_is_over_does_not_record() {
    let w = world();
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 30); // endet T0 + 30 min
    w.tick();
    let approval = w.pending()[0].id.clone();
    w.clock.advance(40 * MIN); // der Nutzer klickt erst nach dem Ende des Termins
    crate::managers::workflows::consent::decide(&w.conn(), &approval, true, true, w.clock.now_ms())
        .unwrap();
    let report = w.tick();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "permanent".into()
        }
    );
    assert_eq!(
        w.rec.start_calls(),
        0,
        "keine Aufnahme eines beendeten Termins"
    );
    assert!(w.run(&run).error.as_deref().unwrap().contains("zu Ende"));
}

#[test]
fn a_second_request_while_a_recording_runs_fails_cleanly_and_does_not_ask_again() {
    let w = world();
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 61);
    w.tick();
    let approval = w.pending()[0].id.clone();
    // Der Nutzer hat inzwischen selbst eine Aufnahme gestartet.
    w.rec.already_recording("manuell");
    crate::managers::workflows::consent::decide(&w.conn(), &approval, true, true, w.clock.now_ms())
        .unwrap();
    let report = w.tick();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "permanent".into()
        }
    );
    assert_eq!(w.rec.start_calls(), 0);
    for _ in 0..5 {
        w.clock.advance(MIN);
        w.tick();
    }
    assert!(
        w.pending().is_empty(),
        "keine neue Bitte nach dem Fehlschlag"
    );
}

#[test]
fn a_missing_microphone_fails_permanently_and_never_asks_a_second_time() {
    let w = world();
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 61);
    w.tick();
    w.rec.fail_next_start("mic_stream_error: kein Geraet");
    let approval = w.pending()[0].id.clone();
    crate::managers::workflows::consent::decide(&w.conn(), &approval, true, true, w.clock.now_ms())
        .unwrap();
    let report = w.tick();
    assert_eq!(
        outcome_of(&report, &run),
        RunOutcome::Failed {
            code: "permanent".into()
        }
    );
    let error = w.run(&run).error.unwrap();
    assert!(error.contains("Mikrofon"), "{error}");
    assert_eq!(w.steps(&run).len(), 1, "ein Versuch, keine Wiederholung");
    for _ in 0..4 {
        w.clock.advance(MIN);
        w.tick();
    }
    assert!(w.pending().is_empty(), "kein zweites Klingeln");
    assert_eq!(w.rec.start_calls(), 0);
}

#[test]
fn two_runs_for_the_same_event_ask_twice_but_record_once() {
    let w = world();
    let first = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let mut other = rec_def(vec![], "event_end");
    other["name"] = json!("Zweiter Ablauf");
    let second = armed_workflow(&w.engine, &other);
    let run_a = w.trigger_event(&first, "t1", 61);
    let run_b = w.trigger_event(&second, "t1", 61);
    w.tick();
    assert_eq!(w.pending().len(), 2);
    for a in w.pending() {
        crate::managers::workflows::consent::decide(&w.conn(), &a.id, true, true, w.clock.now_ms())
            .unwrap();
    }
    w.tick();
    assert_eq!(w.rec.start_calls(), 1, "eine Aufnahme, nicht zwei");
    let states = [w.run(&run_a).state, w.run(&run_b).state];
    assert!(states.contains(&RunState::Done));
    assert!(states.contains(&RunState::Failed));
}

// ---------------------------------------------------------------------------
// Abbruch mitten im Vorgang
// ---------------------------------------------------------------------------

#[test]
fn a_crash_after_the_start_is_confirmed_by_the_recorder_and_never_repeated() {
    let w = world();
    let wf = armed_workflow(
        &w.engine,
        &rec_def(
            vec![step("danach", "notify.local", json!({"title": "weiter"}))],
            "event_end",
        ),
    );
    let run = w.trigger_event(&wf, "t1", 61);
    w.tick();
    let approval = w.pending()[0].id.clone();
    crate::managers::workflows::consent::decide(&w.conn(), &approval, true, true, w.clock.now_ms())
        .unwrap();
    // Die App stirbt, nachdem die Aufnahme lief, aber bevor das Journal `done` kennt.
    w.engine.crash_at("rec", CrashPoint::AfterAction);
    let crashed = w.tick();
    assert!(
        crashed.errors.iter().any(|e| e.contains("after_action")),
        "{crashed:?}"
    );
    assert_eq!(w.rec.start_calls(), 1, "die Aufnahme lief");
    w.clock.advance(61_000);
    let other = w.restart();
    let report = other.tick().unwrap();
    assert_eq!(report.recovered, 1);
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done, "{report:?}");
    assert_eq!(
        w.rec.start_calls(),
        1,
        "bestaetigt, nicht ein zweites Mal gestartet"
    );
    let rows = w.steps(&run);
    assert_eq!(rows[0].state, StepState::Done);
    assert!(rows[0].output_json.as_deref().unwrap().contains("m1"));
}

#[test]
fn a_crash_after_the_start_without_proof_is_uncertain_and_never_repeated() {
    let w = world();
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 61);
    w.tick();
    let approval = w.pending()[0].id.clone();
    crate::managers::workflows::consent::decide(&w.conn(), &approval, true, true, w.clock.now_ms())
        .unwrap();
    w.engine.crash_at("rec", CrashPoint::AfterAction);
    w.tick();
    assert_eq!(w.rec.start_calls(), 1);
    // Die Aufnahme ist inzwischen vom Nutzer beendet worden: der Recorder kann nichts belegen.
    w.rec.user_stops();
    w.clock.advance(61_000);
    let other = w.restart();
    other.tick().unwrap();
    let r = w.run(&run);
    assert_eq!(r.state, RunState::Failed);
    assert_eq!(r.error_code.as_deref(), Some("effect_uncertain"));
    assert_eq!(w.steps(&run)[0].state, StepState::Uncertain);
    for _ in 0..3 {
        w.clock.advance(MIN);
        other.tick().unwrap();
    }
    assert_eq!(w.rec.start_calls(), 1, "nie ein zweiter Start von selbst");
    assert!(w.pending().is_empty());
}

#[test]
fn an_app_restart_between_the_click_and_the_start_still_records_exactly_once() {
    // Die Entscheidung steht in der Datenbank; ein neuer Prozess setzt den Lauf fort.
    let w = world();
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 61);
    w.tick();
    let approval = w.pending()[0].id.clone();
    crate::managers::workflows::consent::decide(&w.conn(), &approval, true, true, w.clock.now_ms())
        .unwrap();
    // Die App stirbt, bevor der Arbeiter die Entscheidung gesehen hat.
    let other = w.restart();
    let report = other.tick().unwrap();
    assert_eq!(outcome_of(&report, &run), RunOutcome::Done);
    assert_eq!(w.rec.start_calls(), 1);
    // Der alte Prozess (zweiter Arbeiter) tut nichts mehr.
    w.tick();
    assert_eq!(w.rec.start_calls(), 1);
}

// ---------------------------------------------------------------------------
// Ende der Aufnahme
// ---------------------------------------------------------------------------

fn stop_of(params: Value, end: Option<i64>) -> i64 {
    plan_stop(&params, end, T0)
}

#[test]
fn the_end_of_an_automatic_recording_is_planned_from_the_event_or_the_duration() {
    // Ende des Termins
    assert_eq!(
        stop_of(json!({"stop": "event_end"}), Some(T0 + 60 * MIN)),
        T0 + 60 * MIN
    );
    // Dauer
    assert_eq!(
        stop_of(json!({"stop": "duration", "max_minutes": 45}), None),
        T0 + 45 * MIN
    );
    // Beides: das Fruehere gewinnt.
    assert_eq!(
        stop_of(
            json!({"stop": "event_end", "max_minutes": 30}),
            Some(T0 + 60 * MIN)
        ),
        T0 + 30 * MIN
    );
    // „event_end“ ohne Terminende und „manual“ ohne Angabe: das Sicherheitsnetz, nie unbegrenzt.
    let net = T0 + DEFAULT_MAX_MINUTES * MIN;
    assert_eq!(stop_of(json!({"stop": "event_end"}), None), net);
    assert_eq!(stop_of(json!({"stop": "manual"}), None), net);
    assert_eq!(stop_of(json!({}), None), net);
    // Eine als Text angekommene Zahl zaehlt, Unsinn nicht.
    assert_eq!(stop_of(json!({"max_minutes": "20"}), None), T0 + 20 * MIN);
    assert_eq!(stop_of(json!({"max_minutes": "viel"}), None), net);
    assert_eq!(stop_of(json!({"max_minutes": 100000}), None), net);
}

#[test]
fn a_started_recording_is_scheduled_to_end_and_the_shared_tick_stops_it() {
    let w = world();
    let wf = armed_workflow(&w.engine, &rec_def(vec![], "event_end"));
    let run = w.trigger_event(&wf, "t1", 61);
    w.tick();
    let approval = w.pending()[0].id.clone();
    crate::managers::workflows::consent::decide(&w.conn(), &approval, true, true, w.clock.now_ms())
        .unwrap();
    w.tick();
    assert_eq!(w.run(&run).state, RunState::Done);
    assert_eq!(w.stops.entries(), vec![("m1".to_string(), T0 + 61 * MIN)]);
    // Vor dem Ende geschieht nichts.
    assert!(run_due_stops(&*w.rec, &w.stops, T0 + 60 * MIN).is_empty());
    assert_eq!(w.rec.stop_calls(), 0);
    // Zum Ende beendet der Takt die Aufnahme (einmal).
    assert_eq!(run_due_stops(&*w.rec, &w.stops, T0 + 61 * MIN), vec!["m1"]);
    assert_eq!(w.rec.stop_calls(), 1);
    assert!(run_due_stops(&*w.rec, &w.stops, T0 + 62 * MIN).is_empty());
    assert_eq!(w.rec.stop_calls(), 1);
}

#[test]
fn a_due_end_never_stops_another_recording_the_user_started_since() {
    let rec = FakeRecorder::new(&FakeClock::new());
    let stops = StopSchedule::default();
    stops.add("m-workflow", T0);
    rec.already_recording("m-nutzer");
    assert!(run_due_stops(&*rec, &stops, T0 + MIN).is_empty());
    assert_eq!(rec.stop_calls(), 0, "die Aufnahme des Nutzers bleibt");
    // Hat der Nutzer die Aufnahme schon selbst beendet, ist nichts zu tun.
    let stops = StopSchedule::default();
    stops.add("m-workflow", T0);
    rec.user_stops();
    assert!(run_due_stops(&*rec, &stops, T0 + MIN).is_empty());
    assert_eq!(rec.stop_calls(), 0);
}

#[test]
fn the_stop_schedule_is_bounded_and_replaces_a_meeting_instead_of_doubling_it() {
    let stops = StopSchedule::default();
    for i in 0..(MAX_STOPS + 5) {
        stops.add(&format!("m{i}"), T0 + i as i64);
    }
    assert_eq!(stops.entries().len(), MAX_STOPS);
    stops.add("m12", T0 + 999);
    assert_eq!(
        stops.entries().iter().filter(|(id, _)| id == "m12").count(),
        1
    );
}

#[test]
fn the_step_recording_stop_ends_the_running_recording_and_is_harmless_when_idle() {
    let w = world();
    let stop = RecordingStop::new(w.rec.clone());
    let cancel = AtomicBool::new(false);
    let ctx_value = json!({});
    let ctx = RunCtx {
        workflow_id: "wf",
        run_id: "run",
        step_id: "s",
        attempt: 1,
        idempotency_key: "run:s".into(),
        context: &ctx_value,
        step_started_at: T0,
        approved: false,
        cancel: &cancel,
        clock: &*w.clock,
        db_path: &w.fx.db_path,
    };
    // Nichts laeuft: in Ordnung.
    let out = stop.run(&ctx, &json!({})).unwrap();
    assert_eq!(out.data["stopped"], false);
    // Es laeuft etwas: beendet.
    w.rec.already_recording("m7");
    let out = stop.run(&ctx, &json!({})).unwrap();
    assert_eq!(out.data["stopped"], true);
    assert_eq!(out.data["meeting_id"], "m7");
    assert_eq!(w.rec.stop_calls(), 1);
    assert_eq!(stop.effect(), EffectKind::Idempotent);
    assert!(
        stop.needs(&json!({})).unwrap().is_none(),
        "kein Recht noetig"
    );
}

// ---------------------------------------------------------------------------
// Traeger
// ---------------------------------------------------------------------------

#[test]
fn the_carrier_is_an_agent_integration_created_once_with_a_write_direction() {
    let fx = Fx::new();
    let conn = fx.conn();
    assert!(ensure_carrier(&conn, T0).unwrap(), "neu angelegt");
    assert!(
        !ensure_carrier(&conn, T0 + 1).unwrap(),
        "ein zweites Mal nichts"
    );
    let all: Vec<_> = istore::list(&conn)
        .unwrap()
        .into_iter()
        .filter(|i| i.id == CARRIER_ID)
        .collect();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].kind, Kind::Agent);
    assert_eq!(all[0].direction, Direction::Write);
    assert!(all[0].enabled);
    assert!(Kind::Agent
        .capabilities()
        .contains(&Capability::RecordingStart));
}

#[test]
fn for_workflows_the_carrier_asks_by_default_and_calendars_cannot_carry_the_right() {
    use crate::managers::integrations::grants::effective_mode;
    let fx = Fx::new();
    let conn = fx.conn();
    ensure_carrier(&conn, T0).unwrap();
    let carrier = istore::get(&conn, CARRIER_ID).unwrap().unwrap();
    let grants = istore::grants_for(&conn, CARRIER_ID).unwrap();
    assert_eq!(
        effective_mode(
            &carrier,
            Capability::RecordingStart,
            Caller::Workflow,
            &grants,
            None
        ),
        GrantMode::Ask
    );
    // Die Gegenprobe der Entscheidung: kein Kalender traegt das Recht.
    assert!(!Kind::Ics
        .capabilities()
        .contains(&Capability::RecordingStart));
    assert!(!Kind::Graph
        .capabilities()
        .contains(&Capability::RecordingStart));
}

#[test]
fn ensure_carrier_survives_a_concurrent_second_creator() {
    let fx = Fx::new();
    let a = fx.conn();
    let b = fx.conn();
    let results: Vec<bool> = std::thread::scope(|s| {
        let h1 = s.spawn(move || ensure_carrier(&a, T0).unwrap());
        let h2 = s.spawn(move || ensure_carrier(&b, T0).unwrap());
        vec![h1.join().unwrap(), h2.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|c| **c).count(), 1);
}

#[test]
fn saving_a_duration_without_max_minutes_is_rejected_with_a_sentence() {
    let w = world();
    let err = w
        .engine
        .save_workflow(None, &rec_def(vec![], "duration"))
        .unwrap_err();
    let WorkflowError::Invalid(issues) = err else {
        panic!("erwartet Invalid");
    };
    assert!(
        issues.iter().any(|i| i.message.contains("max_minutes")),
        "{issues:?}"
    );
    // Mit der Angabe geht es.
    let mut d = rec_def(vec![], "duration");
    d["steps"][0]["params"]["max_minutes"] = json!(30);
    w.engine.save_workflow(None, &d).unwrap();
}

#[test]
fn the_start_failure_texts_are_german_sentences_that_keep_the_code() {
    assert!(start_failure("already_recording").contains("schon eine Aufnahme"));
    assert!(start_failure("loopback_start_failed").contains("Systemton"));
    assert!(start_failure("mic_stream_error: kein Geraet").contains("Mikrofon"));
    assert!(start_failure("etwas_neues").contains("etwas_neues"));
}
