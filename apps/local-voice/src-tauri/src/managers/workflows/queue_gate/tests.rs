//! Das Tor mit Blick auf Aufnahme und Import-Warteschlange (B3, QG5).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::managers::workflows::action::EffectKind;
use crate::managers::workflows::catalog;
use crate::managers::workflows::engine::{EnqueueRequest, RunOutcome};
use crate::managers::workflows::heavy::LocalHeavyGate;
use crate::managers::workflows::test_support::{
    armed_workflow, def, engine_with, step, FakeClock, Fx, Scripted,
};

#[derive(Default)]
struct Probe {
    recording: AtomicBool,
    busy: AtomicBool,
}

impl QueueProbe for Probe {
    fn recording(&self) -> bool {
        self.recording.load(Ordering::SeqCst)
    }
    fn import_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }
}

const STT: HeavyNeed = HeavyNeed {
    ram_mb: 3_072,
    label: "Transkription",
};

fn gate_with(probe: &Arc<Probe>) -> (QueueAwareGate, Arc<LocalHeavyGate>) {
    let inner = Arc::new(LocalHeavyGate::with_probe(Box::new(|_| Ok(()))));
    (QueueAwareGate::new(inner.clone(), probe.clone()), inner)
}

#[test]
fn an_idle_system_lets_a_heavy_step_in_and_the_permit_frees_the_slot() {
    let probe = Arc::new(Probe::default());
    let (gate, inner) = gate_with(&probe);
    let permit = gate.try_enter(&STT).expect("frei");
    assert!(inner.is_busy(), "der Platz des inneren Tors ist belegt");
    drop(permit);
    assert!(!inner.is_busy());
}

#[test]
fn a_running_recording_has_priority_and_the_step_waits_instead_of_failing() {
    let probe = Arc::new(Probe::default());
    let (gate, inner) = gate_with(&probe);
    probe.recording.store(true, Ordering::SeqCst);
    let wait = gate.try_enter(&STT).err().expect("wartet");
    assert_eq!(wait.reason, HeavyWaitReason::Other);
    assert_eq!(wait.reason.code(), "heavy_other");
    assert_eq!(wait.retry_after_ms, RECORDING_RETRY_MS);
    assert!(wait.message.contains("Aufnahme"), "{}", wait.message);
    assert!(!inner.is_busy(), "kein Platz wurde belegt");
    probe.recording.store(false, Ordering::SeqCst);
    assert!(gate.try_enter(&STT).is_ok(), "danach geht es");
}

#[test]
fn a_busy_import_queue_makes_heavy_steps_wait_so_files_run_one_after_another() {
    let probe = Arc::new(Probe::default());
    let (gate, _) = gate_with(&probe);
    probe.busy.store(true, Ordering::SeqCst);
    for need in [
        STT,
        HeavyNeed {
            ram_mb: 6_144,
            label: "Sprachmodell",
        },
    ] {
        let wait = gate.try_enter(&need).err().expect("wartet");
        assert_eq!(wait.retry_after_ms, QUEUE_RETRY_MS);
        assert!(wait.message.contains("Import-Warteschlange") && wait.message.contains(need.label));
    }
    probe.busy.store(false, Ordering::SeqCst);
    assert!(gate.try_enter(&STT).is_ok());
}

#[test]
fn the_inner_gate_still_decides_about_its_slot_and_about_memory() {
    let probe = Arc::new(Probe::default());
    let inner = Arc::new(LocalHeavyGate::with_probe(Box::new(|need| {
        if need > 4_000 {
            Err("zu wenig frei".to_string())
        } else {
            Ok(())
        }
    })));
    let gate = QueueAwareGate::new(inner, probe);
    let first = gate.try_enter(&STT).expect("erster Platz");
    let second = gate.try_enter(&STT).err().expect("zweiter wartet");
    assert_eq!(second.reason, HeavyWaitReason::Slot);
    drop(first);
    let big = HeavyNeed {
        ram_mb: 6_144,
        label: "Sprachmodell",
    };
    let wait = gate.try_enter(&big).err().expect("zu wenig RAM");
    assert_eq!(wait.reason, HeavyWaitReason::Memory);
}

/// Mit der Engine: ein schwerer Schritt, der wartet, verbraucht keinen Versuch und laeuft, sobald
/// die Warteschlange leer ist.
#[test]
fn the_engine_parks_a_heavy_run_while_the_queue_is_busy_and_finishes_it_afterwards() {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let probe = Arc::new(Probe::default());
    probe.busy.store(true, Ordering::SeqCst);
    let (gate, _) = gate_with(&probe);
    let engine = engine_with(&fx, &clock, Arc::new(gate));
    let notes = Scripted::heavy("meeting.notes", EffectKind::Idempotent, {
        let spec = catalog::action_spec("meeting.notes").unwrap();
        spec.heavy.unwrap()
    });
    engine.register_action(notes.clone());
    let wf = armed_workflow(
        &engine,
        &def(vec![step("n", "meeting.notes", json!({}))]),
    );
    let run = engine.enqueue(&EnqueueRequest::manual(&wf)).unwrap();

    let report = engine.tick().unwrap();
    assert!(
        matches!(
            report.outcomes.iter().find(|(id, _)| *id == run.run_id).map(|o| &o.1),
            Some(RunOutcome::Parked { .. })
        ),
        "{report:?}"
    );
    assert_eq!(notes.call_count(), 0, "der Baustein lief nicht");

    probe.busy.store(false, Ordering::SeqCst);
    clock.advance(QUEUE_RETRY_MS as i64 + 1);
    let report = engine.tick().unwrap();
    assert!(
        report
            .outcomes
            .iter()
            .any(|(id, o)| *id == run.run_id && *o == RunOutcome::Done),
        "{report:?}"
    );
    assert_eq!(notes.call_count(), 1);
}
