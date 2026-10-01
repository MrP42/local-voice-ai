//! Tests von `agent.extract` als Workflow-Baustein: durch die Engine, mit einer
//! llama-server-Attrappe als Modell.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use chrono::TimeZone;
use serde_json::{json, Value};

use super::*;
use crate::agent::runtime::Target;
use crate::agent::test_support::{closed_port, mock, mock_with, ok, Mock, R};
use crate::managers::meetings::store::{
    MeetingSource, MeetingStatus, MeetingStore, StoredSegment, TranscriptDelta,
};
use crate::managers::provenance::{self, ProvenanceEntry};
use crate::managers::workflows::app_actions::{self, GenRequest, ServiceError};
use crate::managers::workflows::cli;
use crate::managers::workflows::engine::{Clock, Engine, EnqueueRequest, RunOutcome};
use crate::managers::workflows::model::{Origin, RunState, StepState};
use crate::managers::workflows::test_support::{
    armed_workflow, def, engine, step, FakeClock, Fx, T0,
};

fn block<F: std::future::Future>(f: F) -> F::Output {
    tauri::async_runtime::block_on(f)
}

const FIXTURE: &str = include_str!("../../../agent/fixtures/jourfixe-nordlicht.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

fn segments() -> Vec<StoredSegment> {
    fixture()["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| StoredSegment {
            segment_index: s["i"].as_u64().unwrap() as u32,
            text: s["text"].as_str().unwrap().to_string(),
            start_ms: s["start_ms"].as_u64().unwrap(),
            end_ms: s["start_ms"].as_u64().unwrap() + 5_000,
            channel: 1,
            speaker_index: None,
            words: None,
        })
        .collect()
}

/// Dienste mit dem Modell hinter einer Attrappe; alles andere gibt es hier nicht.
struct TestServices {
    store: Arc<MeetingStore>,
    target: Mutex<Result<Target, ServiceError>>,
    notified: Mutex<Vec<(String, String)>>,
}

fn nope<T>() -> Result<T, ServiceError> {
    Err(ServiceError::NotAvailable("nicht Teil dieses Tests".into()))
}

impl app_actions::AppServices for TestServices {
    fn store(&self) -> Result<Arc<MeetingStore>, ServiceError> {
        Ok(self.store.clone())
    }
    fn agent_target(&self) -> Result<Target, ServiceError> {
        self.target.lock().unwrap().clone()
    }
    fn generate_notes(
        &self,
        _: &GenRequest,
        _: &dyn Fn() -> bool,
    ) -> Result<crate::managers::meetings::store::MeetingDocument, ServiceError> {
        nope()
    }
    fn generate_minutes(
        &self,
        _: &GenRequest,
        _: &dyn Fn() -> bool,
    ) -> Result<crate::managers::meetings::store::MeetingDocument, ServiceError> {
        nope()
    }
    fn summarize(
        &self,
        _: &str,
        _: &crate::summarizer::SummaryOptions,
        _: &dyn Fn() -> bool,
    ) -> Result<String, ServiceError> {
        nope()
    }
    fn audio_dir(&self, _: Option<&str>) -> Result<PathBuf, ServiceError> {
        nope()
    }
    fn render_speech(&self, _: &str, _: &Path, _: &dyn Fn() -> bool) -> Result<PathBuf, ServiceError> {
        nope()
    }
    fn notify(&self, title: &str, body: &str) -> Result<(), ServiceError> {
        self.notified
            .lock()
            .unwrap()
            .push((title.to_string(), body.to_string()));
        Ok(())
    }
}

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
    services: Arc<TestServices>,
    meeting_id: String,
}

fn target_of(m: &Mock) -> Target {
    Target::Endpoint {
        base_url: m.base_url.clone(),
        model: "llm-test".into(),
        context_tokens: 8192,
    }
}

/// Eine Welt mit fertiger Besprechung vom Donnerstag, 1.10.2026, mittags (Ortszeit).
fn world_with(target: Result<Target, ServiceError>, status: MeetingStatus, with_text: bool) -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let meeting = store
        .create_meeting("Jour fixe Projekt Nordlicht", MeetingSource::Import, None)
        .unwrap();
    if with_text {
        store
            .append_delta(
                &meeting.id,
                &TranscriptDelta {
                    new_segments: segments(),
                },
            )
            .unwrap();
    }
    store.set_status(&meeting.id, status).unwrap();
    let noon = chrono::Local
        .with_ymd_and_hms(2026, 10, 1, 12, 0, 0)
        .unwrap()
        .timestamp();
    fx.conn()
        .execute(
            "UPDATE meetings SET started_at = ?1 WHERE id = ?2",
            rusqlite::params![noon, meeting.id],
        )
        .unwrap();
    let services = Arc::new(TestServices {
        store,
        target: Mutex::new(target),
        notified: Mutex::new(Vec::new()),
    });
    app_actions::install(&engine, services.clone());
    install(&engine, services.clone());
    World {
        fx,
        clock,
        engine,
        services,
        meeting_id: meeting.id,
    }
}

fn world(m: &Mock) -> World {
    world_with(Ok(target_of(m)), MeetingStatus::Ready, true)
}

impl World {
    fn enqueue(&self, workflow: &str) -> String {
        self.engine
            .enqueue(&EnqueueRequest {
                workflow_id: workflow.to_string(),
                trigger_key: format!("meeting:{}", self.meeting_id),
                origin: Origin::Trigger,
                trigger: json!({ "meeting_id": self.meeting_id, "stage": "transcript" }),
                vars: serde_json::Map::new(),
                force_dry_run: false,
            })
            .unwrap()
            .run_id
    }

    fn step_output(&self, run: &str, step_id: &str) -> Value {
        let detail = self.engine.run_detail(run).unwrap();
        let row = detail.steps.iter().rfind(|s| s.step_id == step_id).unwrap();
        serde_json::from_str(row.output_json.as_deref().unwrap_or("{}")).unwrap()
    }

    fn step_state(&self, run: &str, step_id: &str) -> StepState {
        self.engine
            .run_detail(run)
            .unwrap()
            .steps
            .iter()
            .rfind(|s| s.step_id == step_id)
            .map(|s| s.state)
            .unwrap()
    }

    fn provenance(&self, subject: &str) -> Vec<ProvenanceEntry> {
        provenance::list(&self.fx.conn(), SubjectKind::RunOutput, subject).unwrap()
    }
}

/// Ein RunCtx fuer den direkten Aufruf des Bausteins (ohne Engine).
struct Direct {
    cancel: AtomicBool,
    context: Value,
}

impl Direct {
    fn new(meeting_id: &str) -> Self {
        Self {
            cancel: AtomicBool::new(false),
            context: json!({ "trigger": { "meeting_id": meeting_id }, "steps": {} }),
        }
    }

    fn run(&self, w: &World, run_id: &str, params: &Value) -> Result<StepOutput, StepError> {
        let clock: &dyn Clock = &*w.clock;
        let ctx = RunCtx {
            workflow_id: "wf-test",
            run_id,
            step_id: "extract",
            attempt: 1,
            idempotency_key: format!("{run_id}:extract"),
            context: &self.context,
            step_started_at: T0,
            approved: false,
            gate_args: None,
            cancel: &self.cancel,
            clock,
            db_path: &w.fx.db_path,
        };
        AgentExtract::new(w.services.clone()).run(&ctx, params)
    }
}

fn extraction_reply() -> String {
    fixture()["responses"][0].to_string()
}

fn extract_then_notify() -> Value {
    def(vec![
        step("extract", "agent.extract", json!({})),
        step(
            "note",
            "notify.local",
            json!({
                "title": "{{steps.extract.counts.todos}} To-dos, {{steps.extract.counts.deadlines}} Fristen",
                "body": "Modell {{steps.extract.provenance.model}}"
            }),
        ),
    ])
}

// -- Durch die Engine ------------------------------------------------------------------------------

#[test]
fn agent_extract_runs_in_a_workflow_feeds_later_steps_and_writes_step_provenance() {
    let m = block(mock(vec![ok(&extraction_reply())]));
    let w = world(&m);
    let wf = armed_workflow(&w.engine, &extract_then_notify());
    let run = w.enqueue(&wf);
    let report = w.engine.tick().unwrap();
    assert_eq!(report.outcomes, vec![(run.clone(), RunOutcome::Done)], "{report:?}");
    assert_eq!(w.engine.run_detail(&run).unwrap().run.state, RunState::Done);
    assert_eq!(m.count(), 1);

    // Die Ausgabe: Listen mit aufgeloesten Daten und Belegen, Herkunft des Modells.
    let out = w.step_output(&run, "extract");
    assert_eq!(out["outcome"], "extracted");
    assert_eq!(out["meeting_date"], "2026-10-01");
    assert_eq!(out["counts"]["todos"], 4);
    assert_eq!(out["todos"][0]["due"], "2026-10-03");
    assert_eq!(out["todos"][1]["due"], "2026-10-09");
    assert_eq!(out["deadlines"][0]["segments"], json!([5, 6]));
    assert_eq!(out["provenance"]["model"], "llm-test");
    assert_eq!(out["provenance"]["prompt_tokens"], 100);

    // Ein spaeterer Schritt nutzt das Ergebnis.
    let notified = w.services.notified.lock().unwrap().clone();
    assert_eq!(notified.len(), 1);
    assert_eq!(notified[0].0, "4 To-dos, 2 Fristen");
    assert_eq!(notified[0].1, "Modell llm-test");

    // Provenienz des Schritts: Modell, Token, Dauer, Quellen (Besprechung und Segmente), Konfidenz.
    let own = w.provenance(&format!("{run}:extract"));
    assert_eq!(own.len(), 1, "{own:?}");
    let entry = &own[0];
    assert_eq!(entry.operation, "agent_extract");
    assert_eq!(entry.model_id.as_deref(), Some("llm-test"));
    assert_eq!(entry.locality, Some(crate::managers::provenance::Locality::Local));
    assert_eq!(entry.prompt_tokens, Some(100));
    assert_eq!(entry.completion_tokens, Some(20));
    assert!(entry.duration_ms.is_some());
    assert_eq!(entry.confidence, Some(1.0));
    assert!(entry.sources.iter().any(|s| s.kind == "meeting" && s.reference == w.meeting_id));
    assert!(entry
        .sources
        .iter()
        .any(|s| s.kind == "segment" && s.reference == format!("{}:S5", w.meeting_id)));
    // Dazu der allgemeine Eintrag der Engine mit Konfidenz und Quellen.
    let generic = w.engine.run_detail(&run).unwrap().provenance;
    assert!(generic
        .iter()
        .any(|p| p.operation == "agent_extract" && p.confidence == Some(1.0)));
}

#[test]
fn garbage_from_the_model_is_a_successful_no_action_step() {
    let m = block(mock(vec![ok("{kaputt"), ok("noch kaputt")]));
    let w = world(&m);
    let wf = armed_workflow(&w.engine, &extract_then_notify());
    let run = w.enqueue(&wf);
    w.engine.tick().unwrap();
    assert_eq!(m.count(), 2, "genau ein Wiederholversuch");
    assert_eq!(w.step_state(&run, "extract"), StepState::Done);
    let out = w.step_output(&run, "extract");
    assert_eq!(out["outcome"], "no_action");
    assert_eq!(out["reason"], "schema_invalid");
    assert_eq!(out["counts"]["todos"], 0);
    // Der folgende Schritt laeuft und sieht null Eintraege: nichts wird erfunden.
    assert_eq!(w.services.notified.lock().unwrap()[0].0, "0 To-dos, 0 Fristen");
}

#[test]
fn two_runs_on_the_same_meeting_both_read_and_share_nothing() {
    let m = block(mock(vec![ok(&extraction_reply())]));
    let w = world(&m);
    let wf = armed_workflow(&w.engine, &extract_then_notify());
    let first = w.enqueue(&wf);
    // Ein zweiter Lauf desselben Ablaufs fuer dieselbe Besprechung (anderer Ausloeser).
    let second = w
        .engine
        .enqueue(&EnqueueRequest {
            workflow_id: wf.clone(),
            trigger_key: "manual:zweiter-lauf".into(),
            origin: Origin::Trigger,
            trigger: json!({ "meeting_id": w.meeting_id }),
            vars: serde_json::Map::new(),
            force_dry_run: false,
        })
        .unwrap()
        .run_id;
    w.engine.tick().unwrap();
    w.engine.tick().unwrap();
    for run in [&first, &second] {
        assert_eq!(w.step_state(run, "extract"), StepState::Done, "{run}");
        assert_eq!(w.step_output(run, "extract")["counts"]["todos"], 4);
        assert_eq!(w.provenance(&format!("{run}:extract")).len(), 1);
    }
    assert_eq!(m.count(), 2, "ein Modellaufruf je Lauf");
}

// -- Direkt: Fehlerklassen ---------------------------------------------------------------------------

#[test]
fn a_meeting_that_is_still_processing_defers_without_a_model_call() {
    let m = block(mock(vec![ok(&extraction_reply())]));
    let w = world_with(Ok(target_of(&m)), MeetingStatus::Processing, true);
    let direct = Direct::new(&w.meeting_id);
    let err = direct.run(&w, "run-1", &json!({})).unwrap_err();
    assert!(matches!(err, StepError::Defer { retry_after_ms: 15_000, .. }), "{err:?}");
    assert_eq!(m.count(), 0);
}

#[test]
fn server_trouble_maps_to_transient_defer_or_permanent() {
    // Server weg: es ist nichts passiert, ein neuer Versuch ist sicher.
    let w = world_with(
        Ok(Target::Endpoint {
            base_url: format!("http://127.0.0.1:{}/v1", block(closed_port())),
            model: "llm-test".into(),
            context_tokens: 8192,
        }),
        MeetingStatus::Ready,
        true,
    );
    let err = Direct::new(&w.meeting_id).run(&w, "run-1", &json!({})).unwrap_err();
    assert!(matches!(err, StepError::Transient(_)), "{err:?}");

    // Server belegt: der Schritt wartet, er scheitert nicht.
    let busy = block(mock(vec![R::Status(503)]));
    let w = world(&busy);
    let err = Direct::new(&w.meeting_id).run(&w, "run-1", &json!({})).unwrap_err();
    assert!(matches!(err, StepError::Defer { .. }), "{err:?}");

    // Lokales Modell nicht eingerichtet (kein Verwalter): dauerhaft.
    let w = world_with(
        Ok(Target::Local {
            model: "llm-gemma4-e4b-q4".into(),
        }),
        MeetingStatus::Ready,
        true,
    );
    let err = Direct::new(&w.meeting_id).run(&w, "run-1", &json!({})).unwrap_err();
    assert!(matches!(err, StepError::Permanent(_)), "{err:?}");

    // Anbieter nicht lokal (Auswahl der App): dauerhaft, mit dem Satz des Dienstes.
    let w = world_with(
        Err(ServiceError::Permanent("Der lokale Agent läuft nur lokal.".into())),
        MeetingStatus::Ready,
        true,
    );
    let err = Direct::new(&w.meeting_id).run(&w, "run-1", &json!({})).unwrap_err();
    assert_eq!(err, StepError::Permanent("Der lokale Agent läuft nur lokal.".into()));

    // Ohne App (Trockenlauf-Kommandozeile): nicht eingebaut.
    let w = world_with(Err(ServiceError::NotAvailable("x".into())), MeetingStatus::Ready, true);
    let err = Direct::new(&w.meeting_id).run(&w, "run-1", &json!({})).unwrap_err();
    assert!(matches!(err, StepError::NotAvailable(_)), "{err:?}");
}

#[test]
fn an_empty_transcript_is_permanent_and_asks_the_model_nothing() {
    let m = block(mock(vec![ok(&extraction_reply())]));
    let w = world_with(Ok(target_of(&m)), MeetingStatus::Ready, false);
    let err = Direct::new(&w.meeting_id).run(&w, "run-1", &json!({})).unwrap_err();
    assert!(matches!(err, StepError::Permanent(_)), "{err:?}");
    assert_eq!(m.count(), 0);
}

#[test]
fn a_run_without_a_meeting_says_which_step_is_missing() {
    let m = block(mock(vec![ok(&extraction_reply())]));
    let w = world(&m);
    let direct = Direct {
        cancel: AtomicBool::new(false),
        context: json!({ "trigger": {}, "steps": {} }),
    };
    let err = direct.run(&w, "run-1", &json!({})).unwrap_err();
    assert!(matches!(&err, StepError::Permanent(m) if m.contains("Besprechung")), "{err:?}");
}

#[test]
fn a_second_attempt_of_the_same_step_writes_no_second_provenance_entry() {
    let m = block(mock(vec![ok(&extraction_reply())]));
    let w = world(&m);
    let direct = Direct::new(&w.meeting_id);
    let first = direct.run(&w, "run-7", &json!({})).unwrap();
    // Absturz nach der Provenienz, vor dem Journal: die Engine wiederholt den Pure-Schritt.
    let second = direct.run(&w, "run-7", &json!({})).unwrap();
    assert_eq!(first.data["counts"], second.data["counts"]);
    assert_eq!(m.count(), 2, "ein Pure-Schritt laeuft erneut ...");
    assert_eq!(w.provenance("run-7:extract").len(), 1, "... ohne zweite Provenienz");
}

#[test]
fn a_broken_provenance_table_never_fails_the_step() {
    let m = block(mock(vec![ok(&extraction_reply())]));
    let w = world(&m);
    w.fx.conn().execute("DROP TABLE provenance", []).unwrap();
    let out = Direct::new(&w.meeting_id)
        .run(&w, "run-8", &json!({}))
        .expect("die Provenienz darf den Schritt nicht scheitern lassen");
    assert_eq!(out.data["outcome"], "extracted");
    assert_eq!(out.confidence, Some(1.0));
    assert!(!out.sources.is_empty());
}

#[test]
fn a_cancelled_run_drops_the_request_and_writes_nothing() {
    let m = block(mock(vec![R::Hang]));
    let w = world(&m);
    let direct = Direct::new(&w.meeting_id);
    let started = std::time::Instant::now();
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(400));
            direct.cancel.store(true, std::sync::atomic::Ordering::Release);
        });
        direct.run(&w, "run-9", &json!({}))
    });
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "Abbruch greift nicht");
    assert!(matches!(result, Err(StepError::Transient(_))), "{result:?}");
    assert!(w.provenance("run-9:extract").is_empty());
}

#[test]
fn kinds_restrict_the_request_and_invalid_kinds_are_refused_when_saving() {
    let m = block(mock_with(|_| ok(r#"{"decisions":[]}"#)));
    let w = world(&m);
    let out = Direct::new(&w.meeting_id)
        .run(&w, "run-3", &json!({ "kinds": ["decisions"] }))
        .unwrap();
    assert_eq!(out.data["counts"]["todos"], 0);
    let sent = m.requests();
    let props = sent[0]["response_format"]["json_schema"]["schema"]["properties"]
        .as_object()
        .unwrap();
    assert_eq!(props.keys().collect::<Vec<_>>(), ["decisions"]);

    let bad = def(vec![step("x", "agent.extract", json!({ "kinds": ["fakten"] }))]);
    let err = w.engine.save_workflow(None, &bad).unwrap_err().to_string();
    assert!(err.contains("fakten"), "{err}");
    let good = def(vec![step("x", "agent.extract", json!({ "kinds": ["todos", "deadlines"] }))]);
    assert!(w.engine.save_workflow(None, &good).is_ok());
}

// -- Plan, Katalog und Vertrag --------------------------------------------------------------------------

#[test]
fn the_block_is_pure_heavy_needs_no_right_and_is_planned_without_a_model_call() {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let services = Arc::new(TestServices {
        store,
        target: Mutex::new(nope()),
        notified: Mutex::new(Vec::new()),
    });
    install(&engine, services.clone());
    let action = AgentExtract::new(services);
    assert_eq!(action.id(), "agent.extract");
    assert_eq!(action.effect(), EffectKind::Pure);
    assert_eq!(action.heavy(&json!({})).unwrap().label, "Sprachmodell");
    assert_eq!(action.needs(&json!({})), Ok(None));
    assert!(action.describe(&json!({})).contains("To-dos, Fristen und Entscheidungen"));
    let only = action.describe(&json!({ "kinds": ["deadlines", "todos"] }));
    assert!(only.ends_with("(nur To-dos, Fristen)"), "{only}");

    // Der Trockenlauf plant mit dem Katalogeintrag und ruft kein Modell.
    let definition = def(vec![step("x", "agent.extract", json!({}))]);
    let result = cli::dry_run(&fx.conn(), &definition.to_string());
    assert_eq!(result.payload["valid"], true, "{}", result.payload);
    let effect = result.payload["steps"][0]["effect"].as_str().unwrap();
    assert!(effect.contains("To-dos"), "{effect}");
    assert!(engine.registry().get("agent.extract").is_some());
}

#[test]
fn a_run_without_the_app_services_is_not_available_not_a_panic() {
    // Die Kommandozeile ohne App (`UnavailableServices`) liefert fuer den Agenten "nicht eingebaut".
    use app_actions::AppServices;
    assert!(matches!(
        app_actions::UnavailableServices.agent_target(),
        Err(ServiceError::NotAvailable(_))
    ));
}
