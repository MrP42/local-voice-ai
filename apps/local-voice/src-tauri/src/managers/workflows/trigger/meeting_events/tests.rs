//! Besprechungsereignisse als Ausloeser (B2).

use serde_json::json;

use super::*;
use crate::managers::workflows::store::{self, RunFilter};
use crate::managers::workflows::test_support::{armed_workflow, def, engine, note, FakeClock, Fx};
use crate::managers::workflows::trigger::test_sink::FlakySink;

fn meeting(id: &str) -> MeetingInfo {
    MeetingInfo {
        id: id.to_string(),
        title: format!("Besprechung {id}"),
    }
}

fn on(stage: &str) -> serde_json::Value {
    json!({"type": "meeting.finished", "stage": stage})
}

struct World {
    fx: Fx,
    engine: crate::managers::workflows::engine::Engine,
}

fn world() -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    World { fx, engine }
}

fn workflow(w: &World, name: &str, trigger: serde_json::Value) -> String {
    let mut d = def(vec![note("a")]);
    d["name"] = json!(name);
    d["trigger"] = trigger;
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
fn each_stage_starts_only_the_workflows_that_wait_for_it() {
    let w = world();
    let rec = workflow(&w, "Aufnahme", on("recording"));
    let tra = workflow(&w, "Transkript", on("transcript"));
    let not = workflow(&w, "Notizen", on("notes"));
    let min = workflow(&w, "Protokoll", on("minutes"));
    for (stage, expected) in [
        (Stage::Recording, &rec),
        (Stage::Transcript, &tra),
        (Stage::Notes, &not),
        (Stage::Minutes, &min),
    ] {
        let r = on_event(&w.engine, stage, &meeting("m1"));
        assert_eq!(r.started.len(), 1, "{stage:?}: {r:?}");
        assert_eq!(&r.started[0].0, expected);
    }
    for wf in [&rec, &tra, &not, &min] {
        assert_eq!(runs(&w, wf).len(), 1);
    }
}

#[test]
fn the_same_event_twice_or_from_many_threads_makes_one_run_per_workflow() {
    let w = world();
    let wf = workflow(&w, "Protokoll", on("minutes"));
    let other = workflow(&w, "Mail", on("minutes"));
    assert_eq!(
        on_event(&w.engine, Stage::Minutes, &meeting("m1"))
            .started
            .len(),
        2
    );
    let again = on_event(&w.engine, Stage::Minutes, &meeting("m1"));
    assert!(
        again.started.is_empty() && again.duplicates == 2,
        "{again:?}"
    );
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                on_event(&w.engine, Stage::Minutes, &meeting("m1"));
            });
        }
    });
    assert_eq!(runs(&w, &wf).len(), 1);
    assert_eq!(runs(&w, &other).len(), 1);
    // Eine andere Besprechung ist ein anderer Ausloeser.
    assert_eq!(
        on_event(&w.engine, Stage::Minutes, &meeting("m2"))
            .started
            .len(),
        2
    );
    assert_eq!(runs(&w, &wf).len(), 2);
}

#[test]
fn regenerating_the_document_does_not_start_the_workflow_again() {
    // Ein Ablauf „Protokoll fertig“, dessen Schritte das Protokoll neu erzeugen, loest sich
    // nicht selbst wieder aus: Schluessel = Besprechung + Stufe.
    let w = world();
    let wf = workflow(&w, "Schleife", on("minutes"));
    for _ in 0..5 {
        on_event(&w.engine, Stage::Minutes, &meeting("m1"));
    }
    assert_eq!(runs(&w, &wf).len(), 1);
    assert_eq!(key_for(Stage::Minutes, "m1"), "meeting:m1:minutes");
}

#[test]
fn disabled_workflows_and_other_triggers_are_ignored() {
    let w = world();
    let off = workflow(&w, "Aus", on("transcript"));
    w.engine.set_enabled(&off, false).unwrap();
    let manual = {
        let mut d = def(vec![note("a")]);
        d["name"] = json!("Manuell");
        armed_workflow(&w.engine, &d)
    };
    let r = on_event(&w.engine, Stage::Transcript, &meeting("m1"));
    assert!(
        r.started.is_empty() && r.duplicates == 0 && r.errors.is_empty(),
        "{r:?}"
    );
    assert!(runs(&w, &off).is_empty() && runs(&w, &manual).is_empty());
}

#[test]
fn the_trigger_data_carry_the_meeting() {
    let w = world();
    let wf = workflow(&w, "Daten", on("notes"));
    on_event(&w.engine, Stage::Notes, &meeting("m7"));
    let ctx: serde_json::Value = serde_json::from_str(&runs(&w, &wf)[0].context_json).unwrap();
    let t = &ctx["trigger"];
    for field in crate::managers::workflows::catalog::trigger_spec(KIND)
        .unwrap()
        .provides
    {
        assert!(t.get(field).is_some(), "{field} fehlt");
    }
    assert_eq!(t["meeting_id"], "m7");
    assert_eq!(t["stage"], "notes");
    assert_eq!(t["title"], "Besprechung m7");
    assert_eq!(t["meeting"]["id"], "m7");
    assert_eq!(t["meeting"]["title"], "Besprechung m7");
}

#[test]
fn a_very_long_title_is_cut_and_still_starts_a_run() {
    let w = world();
    let wf = workflow(&w, "Lang", on("transcript"));
    let r = on_event(
        &w.engine,
        Stage::Transcript,
        &MeetingInfo {
            id: "m1".into(),
            title: "ä".repeat(20_000),
        },
    );
    assert_eq!(r.started.len(), 1, "{r:?}");
    let ctx: serde_json::Value = serde_json::from_str(&runs(&w, &wf)[0].context_json).unwrap();
    assert_eq!(
        ctx["trigger"]["title"].as_str().unwrap().chars().count(),
        500
    );
}

#[test]
fn a_failed_enqueue_is_reported_and_the_next_event_still_works() {
    let w = world();
    let wf = workflow(&w, "Voll", on("transcript"));
    let flaky = FlakySink::new(&w.engine, 1);
    let first = on_event(&flaky, Stage::Transcript, &meeting("m1"));
    assert_eq!(first.errors.len(), 1, "{first:?}");
    // Ein Ereignis kommt nur einmal; der Fehler ist im Bericht und im Log. Das naechste
    // Ereignis einer anderen Besprechung laeuft wieder.
    assert_eq!(
        on_event(&flaky, Stage::Transcript, &meeting("m2"))
            .started
            .len(),
        1
    );
    assert_eq!(runs(&w, &wf).len(), 1);
}

#[test]
fn the_stage_names_match_the_catalog_choices() {
    let spec = crate::managers::workflows::catalog::trigger_spec(KIND).unwrap();
    let stage = spec.fields.iter().find(|f| f.name == "stage").unwrap();
    let crate::managers::workflows::catalog::FieldKind::Choice(options) = stage.kind else {
        panic!("stage ist eine Auswahl");
    };
    for s in [
        Stage::Recording,
        Stage::Transcript,
        Stage::Notes,
        Stage::Minutes,
    ] {
        assert!(options.contains(&s.as_str()), "{}", s.as_str());
    }
    assert_eq!(options.len(), 4);
}
