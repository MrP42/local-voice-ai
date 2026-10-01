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

// --- B8: Start mit Herkunft ---------------------------------------------------------

#[test]
fn an_agent_start_has_the_origin_agent_and_names_the_client_in_the_key() {
    let (fx, e) = world();
    let wf = armed_workflow(&e, &def(vec![note("a")]));
    let r = start_as(&e, &wf, Map::new(), true, Origin::Agent, Some("C-1"), None).unwrap();
    assert!(r.created && r.dry_run);
    let run = store::get_run(&fx.conn(), &r.run_id).unwrap().unwrap();
    assert_eq!(run.origin, Origin::Agent);
    assert!(run.trigger_key.starts_with("agent:C-1:"), "{}", run.trigger_key);
    // Ohne Anfragekennung ist jeder Start ein eigener Lauf.
    let b = start_as(&e, &wf, Map::new(), true, Origin::Agent, Some("C-1"), None).unwrap();
    assert_ne!(r.run_id, b.run_id);
    assert_eq!(run_count(&fx, &wf), 2);
}

#[test]
fn a_request_id_makes_a_repeated_agent_start_the_same_run() {
    let (fx, e) = world();
    let wf = armed_workflow(&e, &def(vec![note("a")]));
    let a = start_as(&e, &wf, Map::new(), true, Origin::Agent, Some("C-1"), Some("R1")).unwrap();
    let b = start_as(&e, &wf, Map::new(), true, Origin::Agent, Some("C-1"), Some("R1")).unwrap();
    assert!(a.created && !b.created);
    assert_eq!(a.run_id, b.run_id);
    // Eine andere Anfrage oder ein anderer Zugang ist ein anderer Lauf.
    let c = start_as(&e, &wf, Map::new(), true, Origin::Agent, Some("C-1"), Some("R2")).unwrap();
    let d = start_as(&e, &wf, Map::new(), true, Origin::Agent, Some("C-2"), Some("R1")).unwrap();
    assert!(c.created && d.created);
    assert_eq!(run_count(&fx, &wf), 3);
}

#[test]
fn a_trigger_origin_cannot_be_started_by_hand() {
    let (fx, e) = world();
    let wf = armed_workflow(&e, &def(vec![note("a")]));
    assert!(matches!(
        start_as(&e, &wf, Map::new(), true, Origin::Trigger, None, None),
        Err(WorkflowError::BadInput(_))
    ));
    assert_eq!(run_count(&fx, &wf), 0);
}

#[test]
fn an_agent_never_runs_more_live_than_the_workflow() {
    let (fx, e) = world();
    // Nicht scharf geschaltet: auch ein Start „live“ plant nur.
    let row = e.save_workflow(None, &def(vec![note("a")])).unwrap();
    e.set_enabled(&row.id, true).unwrap();
    let r = start_as(&e, &row.id, Map::new(), false, Origin::Agent, Some("C-1"), None).unwrap();
    assert!(r.dry_run, "ein nicht scharfer Ablauf plant nur");
    // Ausgeschaltet: ein echter Lauf wird abgewiesen, der Trockenlauf geht.
    let wf = armed_workflow(&e, &def(vec![note("b")]));
    e.set_enabled(&wf, false).unwrap();
    assert!(matches!(
        start_as(&e, &wf, Map::new(), false, Origin::Agent, Some("C-1"), None),
        Err(WorkflowError::Disabled(_))
    ));
    assert!(start_as(&e, &wf, Map::new(), true, Origin::Agent, Some("C-1"), None).is_ok());
    assert_eq!(run_count(&fx, &wf), 1);
}
