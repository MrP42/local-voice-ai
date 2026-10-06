//! Tests der Dienst-Bausteine. Kein Netz: der Ausfuehrer ist eine Attrappe, die jede Anfrage
//! mitschreibt und vorgegebene Antworten liefert. Die Anfragen selbst prueft `services::ops`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use super::*;
use crate::managers::integrations::approvals;
use crate::managers::integrations::model::{Caller, GrantMode, NewIntegration};
use crate::managers::integrations::store as integrations_store;
use crate::managers::meetings::store::{MeetingDocument, MeetingStore};
use crate::managers::workflows::app_actions::{
    GenRequest, ServiceError as AppError, UnavailableServices,
};
use crate::managers::workflows::engine::{Clock, Engine, EnqueueRequest, RunOutcome};
use crate::managers::workflows::model::{Origin, StepState};
use crate::managers::workflows::test_support::{armed_workflow, engine, step, FakeClock, Fx, T0};
use crate::summarizer::SummaryOptions;

// ---------------------------------------------------------------------------
// Attrappen
// ---------------------------------------------------------------------------

struct Svc {
    secrets: Mutex<HashMap<String, String>>,
}

impl AppServices for Svc {
    fn store(&self) -> Result<Arc<MeetingStore>, AppError> {
        UnavailableServices.store()
    }
    fn generate_notes(
        &self,
        r: &GenRequest,
        c: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, AppError> {
        UnavailableServices.generate_notes(r, c)
    }
    fn generate_minutes(
        &self,
        r: &GenRequest,
        c: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, AppError> {
        UnavailableServices.generate_minutes(r, c)
    }
    fn summarize(
        &self,
        t: &str,
        o: &SummaryOptions,
        c: &dyn Fn() -> bool,
    ) -> Result<String, AppError> {
        UnavailableServices.summarize(t, o, c)
    }
    fn audio_dir(&self, m: Option<&str>) -> Result<std::path::PathBuf, AppError> {
        UnavailableServices.audio_dir(m)
    }
    fn render_speech(
        &self,
        t: &str,
        o: &std::path::Path,
        c: &dyn Fn() -> bool,
    ) -> Result<std::path::PathBuf, AppError> {
        UnavailableServices.render_speech(t, o, c)
    }
    fn notify(&self, t: &str, b: &str) -> Result<(), AppError> {
        UnavailableServices.notify(t, b)
    }
    fn secret(&self, i: &Integration, slot: &str) -> Result<Option<Zeroizing<String>>, String> {
        Ok(self
            .secrets
            .lock()
            .unwrap()
            .get(&format!("{}/{slot}", i.id))
            .map(|s| Zeroizing::new(s.clone())))
    }
}

#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<ApiRequest>>,
    replies: Mutex<VecDeque<Result<ApiReply, ServiceError>>>,
}

impl Fake {
    fn reply(&self, r: Result<Value, ServiceError>) {
        self.replies.lock().unwrap().push_back(r.map(ApiReply::ok));
    }
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    fn body(&self, n: usize) -> Value {
        self.calls.lock().unwrap()[n]
            .body
            .clone()
            .unwrap_or(Value::Null)
    }
}

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
    svc: Arc<Svc>,
    fake: Arc<Fake>,
}

const SLACK_URL: &str = "https://hooks.slack.com/services/T000/B000/GEHEIM";

fn world() -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    let svc = Arc::new(Svc {
        secrets: Mutex::new(HashMap::new()),
    });
    let fake = Arc::new(Fake::default());
    let f = fake.clone();
    let exec: Exec = Arc::new(move |r: &ApiRequest| {
        f.calls.lock().unwrap().push(r.clone());
        f.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(ApiReply::ok(Value::Null)))
    });
    for a in actions(svc.clone(), exec) {
        engine.register_action(a);
    }
    let w = World {
        fx,
        clock,
        engine,
        svc,
        fake,
    };
    w.add(
        "slack-1",
        "Team-Kanal",
        json!({"service": "slack", "host": "hooks.slack.com"}),
        Some("hooks.slack.com"),
        SLACK_URL,
    );
    w.add(
        "asana-1",
        "Kundenprojekt",
        json!({"service": "asana", "project_id": "120"}),
        Some("app.asana.com"),
        "pat-asana",
    );
    w.add(
        "notion-1",
        "Protokolle",
        json!({"service": "notion", "parent_page_id": "P1"}),
        None,
        "ntn",
    );
    w.add(
        "hubspot-1",
        "CRM",
        json!({"service": "hubspot"}),
        None,
        "svc-key",
    );
    w.add(
        "airtable-1",
        "Meeting-Log",
        json!({"service": "airtable", "base_id": "app1", "table": "Log"}),
        None,
        "pat-at",
    );
    w
}

impl World {
    fn conn(&self) -> rusqlite::Connection {
        self.fx.conn()
    }

    fn add(&self, id: &str, label: &str, config: Value, hint: Option<&str>, secret: &str) {
        let mut n = NewIntegration::new(Kind::Service, label);
        n.id = Some(id.to_string());
        n.config = config;
        n.account_hint = hint.map(str::to_string);
        integrations_store::create(&self.conn(), &n, T0).unwrap();
        self.svc
            .secrets
            .lock()
            .unwrap()
            .insert(format!("{id}/token"), secret.to_string());
    }

    fn action(&self, id: &str) -> Arc<dyn Action> {
        self.engine.registry().get(id).unwrap().clone()
    }

    fn run(
        &self,
        action: &str,
        run_id: &str,
        context: &Value,
        params: &Value,
    ) -> Result<StepOutput, StepError> {
        let cancel = AtomicBool::new(false);
        let clock: &dyn Clock = &*self.clock;
        let ctx = RunCtx {
            workflow_id: "wf-test",
            run_id,
            step_id: "s",
            attempt: 1,
            idempotency_key: format!("{run_id}:s"),
            context,
            step_started_at: T0,
            approved: false,
            gate_args: None,
            cancel: &cancel,
            clock,
            db_path: &self.fx.db_path,
        };
        self.action(action).run(&ctx, params)
    }

    fn gate(&self, action: &str, context: &Value, params: &Value, planning: bool) -> GateView {
        let conn = self.conn();
        let env = GateEnv {
            conn: &conn,
            context,
            planning,
        };
        self.action(action)
            .gate_view(&env, params)
            .unwrap()
            .unwrap()
    }
}

fn extraction() -> Value {
    json!({"steps": {"extract": {
        "outcome": "extracted", "meeting_id": "m1", "meeting_date": "2026-10-01",
        "todos": [{"text": "Frau Berg schickt das Angebot.", "assignee": "Frau Berg",
                   "segments": [2], "quote": "Ich schicke das Angebot.", "confidence": 0.8}],
        "deadlines": [{"text": "Das Angebot geht an die Stadtwerke.", "due": "2026-10-09",
                       "due_source": "angabe:freitag", "segments": [3],
                       "quote": "bis Freitag an die Stadtwerke", "confidence": 0.9}],
        "decisions": [{"text": "Die Abnahme ist am 15. Oktober.", "segments": [5],
                       "quote": "der 15. steht", "confidence": 0.95}],
        "provenance": {"model": "llm-test", "locality": "local"}
    }}})
}

// ---------------------------------------------------------------------------
// Ablaeufe
// ---------------------------------------------------------------------------

#[test]
fn chat_post_sends_once_and_a_repeated_step_does_not_post_again() {
    let w = world();
    let params = json!({"via": "slack-1", "text": "## Protokoll\n- Release verschoben"});
    let out = w.run("chat.post", "R1", &json!({}), &params).unwrap();
    assert_eq!(out.data["messages"], 1);
    assert_eq!(out.data["service"], "slack");
    assert_eq!(w.fake.count(), 1);
    assert_eq!(w.fake.body(0)["text"], "*Protokoll*\n- Release verschoben");
    let again = w.run("chat.post", "R1", &json!({}), &params).unwrap();
    assert_eq!(again.data["reused"], true);
    assert_eq!(w.fake.count(), 1, "kein zweiter Post");
    // Ein anderer Lauf postet neu.
    w.run("chat.post", "R2", &json!({}), &params).unwrap();
    assert_eq!(w.fake.count(), 2);
}

#[test]
fn task_create_from_makes_one_task_per_todo_and_deadline_and_never_twice() {
    let w = world();
    w.fake.reply(Ok(
        json!({"data": {"gid": "1", "permalink_url": "https://app.asana.com/0/120/1"}}),
    ));
    w.fake.reply(Ok(json!({"data": {"gid": "2"}})));
    let params = json!({"via": "asana-1", "from": "extract"});
    let out = w
        .run("task.create_from", "R1", &extraction(), &params)
        .unwrap();
    assert_eq!(out.data["count"], 2, "To-do und Frist, keine Entscheidung");
    assert_eq!(w.fake.count(), 2);
    let first = w.fake.body(0);
    assert_eq!(first["data"]["name"], "Frau Berg schickt das Angebot.");
    assert!(first["data"]["notes"]
        .as_str()
        .unwrap()
        .contains("Zuständig: Frau Berg"));
    assert_eq!(w.fake.body(1)["data"]["due_on"], "2026-10-09");
    // Wiederholung desselben Schritts: nichts Neues.
    let again = w
        .run("task.create_from", "R1", &extraction(), &params)
        .unwrap();
    assert_eq!(again.data["count"], 0);
    assert_eq!(again.data["reused_count"], 2);
    assert_eq!(w.fake.count(), 2);
    // Ohne Fristen: nur das To-do.
    let only = json!({"via": "asana-1", "deadlines": false});
    w.fake.reply(Ok(json!({"data": {"gid": "3"}})));
    assert_eq!(
        w.run("task.create_from", "R2", &extraction(), &only)
            .unwrap()
            .data["count"],
        1
    );
}

#[test]
fn after_a_failure_midway_only_the_missing_tasks_are_created() {
    let w = world();
    w.fake.reply(Ok(json!({"data": {"gid": "1"}})));
    w.fake
        .reply(Err(ServiceError::Connect("app.asana.com".into())));
    let params = json!({"via": "asana-1"});
    let err = w
        .run("task.create_from", "R1", &extraction(), &params)
        .unwrap_err();
    match &err {
        StepError::Transient(t) => assert!(t.contains("1 von 2"), "{t}"),
        other => panic!("{other:?}"),
    }
    w.fake.reply(Ok(json!({"data": {"gid": "2"}})));
    let out = w
        .run("task.create_from", "R1", &extraction(), &params)
        .unwrap();
    assert_eq!(out.data["count"], 1);
    assert_eq!(out.data["reused_count"], 1);
    assert_eq!(w.fake.count(), 3, "erste Aufgabe nicht noch einmal");
    assert_eq!(
        w.fake.body(2)["data"]["name"],
        "Das Angebot geht an die Stadtwerke."
    );
}

#[test]
fn page_note_and_record_reach_their_services() {
    let w = world();
    w.fake
        .reply(Ok(json!({"id": "PG", "url": "https://www.notion.so/PG"})));
    let out = w
        .run(
            "page.create",
            "R1",
            &json!({}),
            &json!({"via": "notion-1", "title": "Protokoll 06.10.", "content": "# A\n- b"}),
        )
        .unwrap();
    assert_eq!(out.data["url"], "https://www.notion.so/PG");

    w.fake.reply(Ok(json!({"results": [{"id": "501"}]})));
    w.fake.reply(Ok(json!({"id": "n1"})));
    let out = w
        .run("crm.note", "R2", &json!({}), &json!({"via": "hubspot-1", "text": "Angebot folgt.", "contact_email": "Kunde@Firma.de"}))
        .unwrap();
    assert_eq!(out.data["linked"], true);
    assert_eq!(
        w.fake.body(1)["filterGroups"][0]["filters"][0]["value"],
        "kunde@firma.de"
    );

    w.fake.reply(Ok(json!({"records": [{"id": "rec1"}]})));
    let out = w
        .run("record.append", "R3", &json!({}), &json!({"via": "airtable-1", "fields": {"Titel": "Jour fixe", "Dauer": 45, "Tags": ["a", "b"]}}))
        .unwrap();
    assert_eq!(out.data["id"], "rec1");
}

#[test]
fn a_service_without_the_capability_is_refused_before_any_call() {
    let w = world();
    let e = w
        .run(
            "chat.post",
            "R1",
            &json!({}),
            &json!({"via": "asana-1", "text": "x"}),
        )
        .unwrap_err();
    assert!(
        matches!(&e, StepError::Permanent(t) if t.contains("Asana kann keine")),
        "{e:?}"
    );
    let e = w
        .run(
            "task.create",
            "R1",
            &json!({}),
            &json!({"via": "slack-1", "title": "x"}),
        )
        .unwrap_err();
    assert!(matches!(e, StepError::Permanent(_)));
    assert_eq!(w.fake.count(), 0);
}

#[test]
fn bad_inputs_fail_with_plain_text_and_send_nothing() {
    let w = world();
    let cases = [
        (
            "task.create",
            json!({"via": "asana-1", "title": "x", "due": "morgen"}),
            "kein Datum",
        ),
        (
            "task.create",
            json!({"via": "asana-1", "title": "  "}),
            "keinen Titel",
        ),
        ("chat.post", json!({"via": "slack-1", "text": ""}), "leer"),
        (
            "crm.note",
            json!({"via": "hubspot-1", "text": "x", "contact_email": "kein-at"}),
            "keine gültige",
        ),
        (
            "record.append",
            json!({"via": "airtable-1", "fields": {"a": {"b": 1}}}),
            "keinen einfachen Wert",
        ),
        (
            "record.append",
            json!({"via": "airtable-1", "fields": {}}),
            "1 bis 50",
        ),
    ];
    for (action, params, want) in cases {
        match w.run(action, "R1", &json!({}), &params) {
            Err(StepError::Permanent(t)) => assert!(t.contains(want), "{action}: {t}"),
            other => panic!("{action}: {other:?}"),
        }
    }
    assert_eq!(w.fake.count(), 0);
}

#[test]
fn a_missing_key_is_a_clear_permanent_error() {
    let w = world();
    w.svc.secrets.lock().unwrap().remove("asana-1/token");
    match w.run(
        "task.create",
        "R1",
        &json!({}),
        &json!({"via": "asana-1", "title": "x"}),
    ) {
        Err(StepError::Permanent(t)) => assert!(t.contains("Schlüssel"), "{t}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.fake.count(), 0);
}

#[test]
fn service_errors_keep_their_meaning_for_retries() {
    let cases = [
        (ServiceError::Auth(401), "permanent"),
        (ServiceError::NotFound, "permanent"),
        (ServiceError::Connect("x".into()), "transient"),
        (ServiceError::RateLimited(Some(5)), "transient"),
        (ServiceError::Timeout, "unknown"),
        (ServiceError::Status(502, String::new()), "unknown"),
        (ServiceError::Partial("1 von 2".into()), "unknown"),
    ];
    for (e, want) in cases {
        let got = match step_err(e.clone()) {
            StepError::Permanent(_) => "permanent",
            StepError::Transient(_) => "transient",
            StepError::Unknown(_) => "unknown",
            _ => "other",
        };
        assert_eq!(got, want, "{e:?}");
    }
}

// ---------------------------------------------------------------------------
// Tor und Freigabe
// ---------------------------------------------------------------------------

#[test]
fn the_approval_shows_target_and_full_text_but_never_the_webhook_address() {
    let w = world();
    let text = "Zusammenfassung:\n- Release verschoben\n- Tests reparieren";
    let v = w.gate(
        "chat.post",
        &json!({}),
        &json!({"via": "slack-1", "text": text}),
        false,
    );
    assert_eq!(
        v.target.as_deref(),
        Some("Slack „Team-Kanal“ (hooks.slack.com)")
    );
    assert_eq!(v.args["text"], text);
    assert_eq!(v.args["dienst"], "Slack");
    assert!(!v.args.to_string().contains("GEHEIM"));

    let v = w.gate(
        "task.create_from",
        &extraction(),
        &json!({"via": "asana-1"}),
        false,
    );
    assert_eq!(v.args["anzahl"], 2);
    assert_eq!(v.args["aufgaben"][1]["faellig"], "2026-10-09");
    // Im Trockenlauf gibt es das Ergebnis der Extraktion noch nicht.
    let v = w.gate(
        "task.create_from",
        &json!({"steps": {}}),
        &json!({"via": "asana-1"}),
        true,
    );
    assert!(v.args["hinweis"].as_str().unwrap().contains("extract"));
}

#[test]
fn a_workflow_asks_first_and_posts_only_after_the_approval() {
    let w = world();
    let def = json!({
        "schema": "lva-workflow@1", "name": "Protokoll nach Slack",
        "trigger": {"type": "manual"},
        "steps": [step("post", "chat.post", json!({"via": "slack-1", "text": "Hallo Team"}))]
    });
    let wf = armed_workflow(&w.engine, &def);
    let queued = w
        .engine
        .enqueue(&EnqueueRequest {
            workflow_id: wf,
            trigger_key: "manual:1".to_string(),
            origin: Origin::Manual,
            trigger: json!({}),
            vars: Map::new(),
            force_dry_run: false,
        })
        .unwrap();
    w.engine.tick().unwrap();
    let state = |w: &World| {
        w.engine
            .run_detail(&queued.run_id)
            .unwrap()
            .steps
            .iter()
            .rfind(|s| s.step_id == "post")
            .map(|s| s.state)
            .unwrap()
    };
    assert_eq!(
        state(&w),
        StepState::AwaitingApproval,
        "Vorgabe für Workflows: fragen"
    );
    assert_eq!(w.fake.count(), 0, "vor der Freigabe geht nichts hinaus");
    let pending = approvals::list_pending(&w.conn(), T0).unwrap();
    assert_eq!(pending.len(), 1);
    let preview = pending[0].args_preview.clone().unwrap_or_default();
    assert!(preview.contains("Hallo Team"), "{preview}");
    assert!(!preview.contains("GEHEIM"), "{preview}");

    approvals::decide(&w.conn(), &pending[0].id, true, T0).unwrap();
    let outcomes = w.engine.tick().unwrap().outcomes;
    assert_eq!(
        outcomes,
        vec![(queued.run_id.clone(), RunOutcome::Done)],
        "{outcomes:?}"
    );
    assert_eq!(w.fake.count(), 1);
    assert_eq!(state(&w), StepState::Done);
}

#[test]
fn an_integration_switched_off_for_workflows_is_denied_without_a_call() {
    let w = world();
    integrations_store::set_grant(
        &w.conn(),
        "slack-1",
        Capability::ChatPost,
        Caller::Workflow,
        GrantMode::Off,
    )
    .unwrap();
    let def = json!({
        "schema": "lva-workflow@1", "name": "Aus",
        "trigger": {"type": "manual"},
        "steps": [step("post", "chat.post", json!({"via": "slack-1", "text": "x"}))]
    });
    let wf = armed_workflow(&w.engine, &def);
    let queued = w
        .engine
        .enqueue(&EnqueueRequest {
            workflow_id: wf,
            trigger_key: "manual:2".to_string(),
            origin: Origin::Manual,
            trigger: json!({}),
            vars: Map::new(),
            force_dry_run: false,
        })
        .unwrap();
    w.engine.tick().unwrap();
    let detail = w.engine.run_detail(&queued.run_id).unwrap();
    assert_eq!(detail.steps.last().unwrap().state, StepState::Denied);
    assert_eq!(w.fake.count(), 0);
}

#[test]
fn provenance_operations_are_valid_tokens() {
    let valid = |o: &str| {
        !o.is_empty()
            && o.len() <= 48
            && o.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    for op in Op::ALL {
        assert!(valid(op.record_op()), "{}", op.record_op());
    }
    assert!(valid(&item_op("0123456789abcdef")));
}
