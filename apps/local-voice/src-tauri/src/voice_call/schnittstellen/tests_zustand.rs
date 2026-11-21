use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use super::super::abbruch::Zugriffsart::{Lesend, Schreibend};
use super::{gemeinsam, InterfaceError, InterfaceState, Uhr, BEENDET_TTL_MS, MAX_BEENDET};

struct Testuhr(AtomicI64);

impl Uhr for Testuhr {
    fn jetzt_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[test]
fn gemeinsam_liefert_auch_im_anderen_thread_denselben_zustand() {
    let zustand = gemeinsam();
    let anderer = std::thread::spawn(gemeinsam).join().unwrap();
    assert!(Arc::ptr_eq(&zustand, &anderer));
    assert!(std::ptr::eq(&zustand.abbrueche, &anderer.abbrueche));
}

#[test]
fn cancel_meldet_nach_den_bestehenden_zweigen_an_den_gemeinsamen_zustand() {
    let quelle = include_str!("../mod.rs").replace("\r\n", "\n");
    let (_, cancel) = quelle.split_once("pub fn voice_call_cancel(").unwrap();
    let (cancel, hook) = cancel
        .split_once("fn melde_abbruch_an_schnittstellen(request_id: &str) {")
        .unwrap();
    assert!(cancel
        .contains("};\n    melde_abbruch_an_schnittstellen(&request_id);\n    abgebrochen\n}"));
    assert_eq!(
        cancel
            .matches("melde_abbruch_an_schnittstellen(&request_id)")
            .count(),
        1
    );
    let (hook, _) = hook.split_once("\n}").unwrap();
    assert!(hook.contains(
        "schnittstellen::zustand::gemeinsam()\n        .abbrueche\n        \
         .melde_sprachbeginn(request_id)"
    ));
}

#[test]
fn tauri_verwaltet_den_gemeinsamen_zustand() {
    let quelle = include_str!("../../lib.rs");
    assert!(quelle.contains("app_handle.manage(voice_call::schnittstellen::zustand::gemeinsam());"));
    assert!(!quelle.contains(
        "app_handle.manage(Arc::new(voice_call::schnittstellen::zustand::InterfaceState::default()))"
    ));
}

#[test]
fn hook_trifft_lesende_schreibende_und_vorgezogene_aufrufe() {
    let zustand = gemeinsam();
    let lesend = zustand
        .abbrueche
        .registriere("zustand-hook-lesend", Lesend)
        .unwrap();
    let schreibend = zustand
        .abbrueche
        .registriere("zustand-hook-schreibend", Schreibend)
        .unwrap();
    std::thread::spawn(|| {
        for id in [
            "zustand-hook-lesend",
            "zustand-hook-schreibend",
            "zustand-hook-frueh",
        ] {
            super::super::super::melde_abbruch_an_schnittstellen(id);
        }
    })
    .join()
    .unwrap();
    assert!(lesend.abbruch().ausgeloest());
    assert!(!schreibend.abbruch().ausgeloest());
    let frueh = zustand
        .abbrueche
        .registriere("zustand-hook-frueh", Lesend)
        .unwrap();
    assert!(frueh.abbruch().ausgeloest());
}

#[test]
fn unbekanntes_end_verbraucht_keinen_speicher() {
    let zustand = InterfaceState::default();
    for i in 0..=MAX_BEENDET {
        zustand.end(&format!("unbekannt-{i}")).unwrap();
    }
    assert!(zustand.beendet.lock().unwrap().is_empty());
    assert_eq!(zustand.begin("unbekannt-0", "a", "m"), Ok(()));
}

#[test]
fn sperre_verfaellt_erst_nach_24_stunden_und_end_verlaengert_sie_nicht() {
    let uhr = Arc::new(Testuhr(AtomicI64::new(0)));
    let zustand = InterfaceState::mit_uhr(uhr.clone());
    zustand.begin("c", "a", "m").unwrap();
    zustand.end("c").unwrap();
    uhr.0.store(BEENDET_TTL_MS - 1, Ordering::SeqCst);
    zustand.end("c").unwrap();
    assert_eq!(zustand.begin("c", "a", "m"), Err(InterfaceError::Beendet));
    assert_eq!(zustand.beendet.lock().unwrap().len(), 1);
    uhr.0.store(BEENDET_TTL_MS, Ordering::SeqCst);
    assert_eq!(zustand.begin("c", "a", "m"), Ok(()));
    assert!(zustand.beendet.lock().unwrap().is_empty());
}

#[test]
fn volle_menge_behaelt_sperren_und_reserviert_laufende_abschluesse() {
    let uhr = Arc::new(Testuhr(AtomicI64::new(0)));
    let zustand = InterfaceState::mit_uhr(uhr.clone());
    for i in 0..MAX_BEENDET - 1 {
        let c = format!("c{i}");
        zustand.begin(&c, "a", "m").unwrap();
        zustand.end(&c).unwrap();
    }
    zustand.begin("letztes", "a", "m").unwrap();
    assert_eq!(zustand.begin("letztes", "a", "m"), Ok(()));
    assert_eq!(zustand.begin("letztes", "b", "m-b"), Ok(()));
    assert_eq!(
        zustand.begin("neu", "a", "m-neu"),
        Err(InterfaceError::ZuViele)
    );
    zustand.end("letztes").unwrap();
    assert_eq!(zustand.beendet.lock().unwrap().len(), MAX_BEENDET);
    assert_eq!(
        zustand.begin("neu", "a", "m-neu"),
        Err(InterfaceError::ZuViele)
    );
    assert_eq!(zustand.begin("c0", "a", "m"), Err(InterfaceError::Beendet));
    uhr.0.store(BEENDET_TTL_MS - 1, Ordering::SeqCst);
    assert_eq!(
        zustand.begin("neu", "a", "m-neu"),
        Err(InterfaceError::ZuViele)
    );
    uhr.0.store(BEENDET_TTL_MS, Ordering::SeqCst);
    assert_eq!(zustand.begin("neu", "a", "m-neu"), Ok(()));
    assert!(zustand.beendet.lock().unwrap().is_empty());
}
