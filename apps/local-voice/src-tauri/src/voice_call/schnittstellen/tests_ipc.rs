use std::sync::{Arc, Barrier};
use std::thread;

use super::ipc::InterfaceError;
use super::zustand::InterfaceState;

#[test]
fn ungueltige_kennungen_werden_an_jeder_position_abgewiesen() {
    let zustand = InterfaceState::default();
    zustand.begin("c", "a", "m").unwrap();
    for id in [String::new(), "x".repeat(65), "x\ny".into(), "ä".into()] {
        for position in 0..3 {
            let mut ids = ["c", "a", "m"];
            ids[position] = &id;
            assert_eq!(
                zustand.begin(ids[0], ids[1], ids[2]),
                Err(InterfaceError::KennungUngueltig)
            );
        }
        for position in 0..4 {
            let mut ids = ["c", "a", "m", "call"];
            ids[position] = &id;
            assert_eq!(
                zustand.call(ids[0], ids[1], ids[2], ids[3]),
                Err(InterfaceError::KennungUngueltig)
            );
        }
        assert_eq!(zustand.end(&id), Err(InterfaceError::KennungUngueltig));
    }
    let grenze = "x".repeat(64);
    assert_eq!(zustand.begin(&grenze, "A._:-09", "M._:-09"), Ok(()));
}

#[test]
fn bindungen_sind_idempotent_und_in_beiden_richtungen_eindeutig() {
    let zustand = InterfaceState::default();
    assert_eq!(zustand.begin("c", "a", "m"), Ok(()));
    assert_eq!(zustand.begin("c", "a", "m"), Ok(()));
    for (c, a, m) in [("c", "a", "fremd"), ("c", "b", "m"), ("d", "a", "m")] {
        assert_eq!(zustand.begin(c, a, m), Err(InterfaceError::Widerspruch));
    }
    assert!(zustand.ist_gebunden("c", "a", "m"));
    assert!(!zustand.ist_gebunden("c", "b", "m"));
}

#[test]
fn call_prueft_die_gesamte_bindung_und_verweigert_auch_gueltige_aufrufe() {
    let zustand = InterfaceState::default();
    zustand.begin("c", "a", "m").unwrap();
    for (c, a, m) in [("d", "a", "m"), ("c", "b", "m"), ("c", "a", "fremd")] {
        assert_eq!(
            zustand.call(c, a, m, "call"),
            Err(InterfaceError::Widerspruch)
        );
    }
    assert_eq!(
        zustand.call("c", "a", "m", "call"),
        Err(InterfaceError::Verweigert)
    );
    zustand.end("c").unwrap();
    assert_eq!(
        zustand.call("c", "a", "m", "call"),
        Err(InterfaceError::Beendet)
    );
}

#[test]
fn bindungsgrenze_und_freigabe_durch_end() {
    let zustand = InterfaceState::default();
    for c in 0..32 {
        for a in 0..8 {
            zustand
                .begin(&format!("c{c}"), &format!("a{a}"), &format!("m{c}-{a}"))
                .unwrap();
        }
    }
    assert_eq!(zustand.begin("c0", "a0", "m0-0"), Ok(()));
    assert_eq!(
        zustand.begin("neu", "a", "m-neu"),
        Err(InterfaceError::ZuViele)
    );
    zustand.end("c0").unwrap();
    assert!(!zustand.ist_gebunden("c0", "a0", "m0-0"));
    assert_eq!(zustand.begin("neu", "a", "m-neu"), Ok(()));
    assert_eq!(
        zustand.begin("c0", "a0", "m0-0"),
        Err(InterfaceError::Beendet)
    );
}

#[test]
fn end_ignoriert_ein_noch_nicht_begonnenes_gespraech() {
    let zustand = InterfaceState::default();
    assert_eq!(zustand.end("c"), Ok(()));
    assert_eq!(zustand.end("c"), Ok(()));
    assert_eq!(zustand.begin("c", "a", "m"), Ok(()));
}

#[test]
fn end_gegen_laufenden_begin_laesst_keine_wiederbelebung_zu() {
    for _ in 0..32 {
        let zustand = Arc::new(InterfaceState::default());
        zustand.begin("c", "a", "m").unwrap();
        let barriere = Arc::new(Barrier::new(2));
        let anderer_zustand = zustand.clone();
        let andere_barriere = barriere.clone();
        let begin = thread::spawn(move || {
            andere_barriere.wait();
            for _ in 0..32 {
                let ergebnis = anderer_zustand.begin("c", "a", "m");
                assert!(matches!(ergebnis, Ok(()) | Err(InterfaceError::Beendet)));
            }
        });
        barriere.wait();
        zustand.end("c").unwrap();
        begin.join().unwrap();
        assert!(!zustand.ist_gebunden("c", "a", "m"));
        assert_eq!(zustand.begin("c", "a", "m"), Err(InterfaceError::Beendet));
    }
}
