use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use super::*;

fn need() -> HeavyNeed {
    HeavyNeed {
        ram_mb: 3_000,
        label: "Transkription",
    }
}

#[test]
fn there_is_one_slot_and_a_released_permit_frees_it() {
    let gate = LocalHeavyGate::with_probe(Box::new(|_| Ok(())));
    let first = gate.try_enter(&need()).unwrap();
    assert!(gate.is_busy());
    let second = gate.try_enter(&need()).err().expect("der Platz ist belegt");
    assert_eq!(second.reason, HeavyWaitReason::Slot);
    assert_eq!(second.retry_after_ms, SLOT_RETRY_MS);
    assert!(second.message.contains("Transkription"));
    drop(first);
    assert!(!gate.is_busy());
    let again = gate.try_enter(&need()).unwrap();
    drop(again);
}

#[test]
fn too_little_memory_waits_and_does_not_keep_the_slot() {
    let gate = LocalHeavyGate::with_probe(Box::new(|mb| Err(format!("{mb} MB sind nicht frei"))));
    let wait = gate.try_enter(&need()).err().expect("zu wenig Speicher");
    assert_eq!(wait.reason, HeavyWaitReason::Memory);
    assert_eq!(wait.retry_after_ms, MEMORY_RETRY_MS);
    assert!(wait.message.starts_with("Wartet auf Arbeitsspeicher"));
    assert!(
        wait.message.contains("3000"),
        "der Bedarf wird dem Tor genannt"
    );
    assert!(
        !gate.is_busy(),
        "ein abgewiesener Schritt haelt keinen Platz"
    );
    assert_eq!(HeavyWaitReason::Memory.code(), "heavy_memory");
    assert_eq!(HeavyWaitReason::Slot.code(), "heavy_slot");
}

#[test]
fn the_slot_is_released_when_the_holder_panics() {
    let gate = Arc::new(LocalHeavyGate::with_probe(Box::new(|_| Ok(()))));
    let g = gate.clone();
    let result = std::thread::spawn(move || {
        let _permit = g.try_enter(&need()).unwrap();
        panic!("Baustein abgestuerzt");
    })
    .join();
    assert!(result.is_err());
    assert!(!gate.is_busy(), "kein verwaister Platz nach einer Panik");
    assert!(gate.try_enter(&need()).is_ok());
}

#[test]
fn the_shared_gate_is_one_for_the_whole_process() {
    assert!(Arc::ptr_eq(
        &LocalHeavyGate::shared(),
        &LocalHeavyGate::shared()
    ));
}

#[test]
fn threads_never_hold_the_slot_together() {
    let gate = Arc::new(LocalHeavyGate::with_probe(Box::new(|_| Ok(()))));
    let inside = Arc::new(AtomicUsize::new(0));
    let broke = Arc::new(AtomicBool::new(false));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let (gate, inside, broke) = (gate.clone(), inside.clone(), broke.clone());
            std::thread::spawn(move || {
                let mut got = 0;
                for _ in 0..2_000 {
                    if let Ok(permit) = gate.try_enter(&need()) {
                        if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                            broke.store(true, Ordering::SeqCst);
                        }
                        got += 1;
                        inside.fetch_sub(1, Ordering::SeqCst);
                        drop(permit);
                    }
                }
                got
            })
        })
        .collect();
    let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
    assert!(total > 0);
    assert!(!broke.load(Ordering::SeqCst), "zwei Halter zugleich");
    assert!(!gate.is_busy());
}
