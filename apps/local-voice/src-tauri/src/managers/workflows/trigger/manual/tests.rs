use serde_json::{json, Map};

use super::*;
use crate::managers::workflows::store::{self, RunFilter};
use crate::managers::workflows::test_support::{armed_workflow, def, engine, note, FakeClock, Fx};

fn world() -> (Fx, crate::managers::workflows::engine::Engine) {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let e = engine(&fx, &clock);
    (fx, e)
}

fn run_count(fx: &Fx, wf: &str) -> usize {
    store::list_runs(
        &fx.conn(),
        &RunFilter {
            workflow_id: Some(wf.to_string()),
            state: None,
        },
        100,
    )
    .unwrap()
    .len()
}

#[test]
fn every_manual_start_is_its_own_run() {
    let (fx, e) = world();
    let wf = armed_workflow(&e, &def(vec![note("a")]));
    let a = start(&e, &wf, Map::new(), false).unwrap();
    let b = start(&e, &wf, Map::new(), false).unwrap();
    assert!(a.created && b.created);
    assert_ne!(a.run_id, b.run_id);
    assert!(!a.dry_run, "ein scharfer Ablauf laeuft scharf");
    assert_eq!(run_count(&fx, &wf), 2);
    let run = store::get_run(&fx.conn(), &a.run_id).unwrap().unwrap();
    assert_eq!(run.origin.as_str(), "manual");
}

#[test]
fn a_dry_run_can_be_forced_even_for_an_armed_workflow() {
    let (_fx, e) = world();
    let wf = armed_workflow(&e, &def(vec![note("a")]));
    let r = start(&e, &wf, Map::new(), true).unwrap();
    assert!(r.dry_run);
}

#[test]
fn a_disabled_workflow_allows_only_the_dry_run() {
    let (_fx, e) = world();
    let wf = armed_workflow(&e, &def(vec![note("a")]));
    e.set_enabled(&wf, false).unwrap();
    assert!(start(&e, &wf, Map::new(), true).is_ok(), "zum Ausprobieren");
    assert!(matches!(
        start(&e, &wf, Map::new(), false),
        Err(WorkflowError::Disabled(_))
    ));
}

#[test]
fn values_for_declared_variables_are_passed_on_and_checked() {
    let (fx, e) = world();
    let mut d = def(vec![note("a")]);
    d["variables"] = json!({"thema": {"type": "string"}});
    let wf = armed_workflow(&e, &d);
    // Ohne Wert: abgelehnt, nichts eingereiht.
    assert!(matches!(
        start(&e, &wf, Map::new(), false),
        Err(WorkflowError::BadInput(_))
    ));
    assert_eq!(run_count(&fx, &wf), 0);
    // Falsche Art.
    let mut bad = Map::new();
    bad.insert("thema".into(), json!(5));
    assert!(matches!(
        start(&e, &wf, bad, false),
        Err(WorkflowError::BadInput(_))
    ));
    // Unbekannte Variable.
    let mut unknown = Map::new();
    unknown.insert("thema".into(), json!("x"));
    unknown.insert("anderes".into(), json!("y"));
    assert!(matches!(
        start(&e, &wf, unknown, false),
        Err(WorkflowError::BadInput(_))
    ));
    let mut ok = Map::new();
    ok.insert("thema".into(), json!("Budget"));
    let r = start(&e, &wf, ok, false).unwrap();
    let run = store::get_run(&fx.conn(), &r.run_id).unwrap().unwrap();
    let ctx: serde_json::Value = serde_json::from_str(&run.context_json).unwrap();
    assert_eq!(ctx["vars"]["thema"], "Budget");
}

#[test]
fn an_unknown_workflow_is_not_found() {
    let (_fx, e) = world();
    assert!(matches!(
        start(&e, "gibt-es-nicht", Map::new(), false),
        Err(WorkflowError::NotFound(_))
    ));
}
