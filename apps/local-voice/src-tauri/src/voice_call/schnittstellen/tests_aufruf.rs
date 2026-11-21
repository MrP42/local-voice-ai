use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use super::{CallResult, InterfaceError, InterfaceState, Uhr, BEENDET_TTL_MS};

struct Testuhr(AtomicI64);

impl Uhr for Testuhr {
    fn jetzt_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn abgeschlossen() -> Result<CallResult, InterfaceError> {
    Ok(CallResult::Completed {
        outcome: "ok".into(),
        data: None,
        message: None,
    })
}

#[test]
fn gebundene_ausfuehrung_gibt_ihr_ergebnis_zurueck() {
    let s = InterfaceState::default();
    s.begin("c", "a", "m").unwrap();
    assert_eq!(
        s.fuehre_gebunden_aus("c", "a", "m", "call", abgeschlossen),
        abgeschlossen()
    );
    assert_eq!(
        s.fuehre_gebunden_aus("c", "a", "fremd", "call", || {
            panic!("Ausfuehrung ohne Bindung")
        }),
        Err(InterfaceError::Widerspruch)
    );
}

#[test]
fn end_waehrend_ausfuehrung_verwirft_ergebnis_auch_nach_neuem_begin() {
    for neu_beginnen in [false, true] {
        let uhr = Arc::new(Testuhr(AtomicI64::new(0)));
        let s = InterfaceState::mit_uhr(uhr.clone());
        s.begin("c", "a", "m").unwrap();
        let (gestartet, start) = mpsc::channel();
        let (fortsetzen, weiter) = mpsc::channel();
        std::thread::scope(|scope| {
            let zustand = &s;
            let lauf = scope.spawn(move || {
                zustand.fuehre_gebunden_aus("c", "a", "m", "call", || {
                    gestartet.send(()).unwrap();
                    weiter.recv_timeout(Duration::from_secs(5)).unwrap();
                    abgeschlossen()
                })
            });
            start.recv_timeout(Duration::from_secs(5)).unwrap();
            // end muss fertig werden, obwohl die Ausfuehrung noch wartet.
            s.end("c").unwrap();
            if neu_beginnen {
                uhr.0.store(BEENDET_TTL_MS, Ordering::SeqCst);
                s.begin("c", "a", "m").unwrap();
            }
            fortsetzen.send(()).unwrap();
            assert_eq!(lauf.join().unwrap(), Err(InterfaceError::Beendet));
        });
        assert_eq!(s.ist_gebunden("c", "a", "m"), neu_beginnen);
    }
}

#[test]
fn weiteres_begin_desselben_gespraechs_entwertet_keinen_aufruf() {
    let s = InterfaceState::default();
    s.begin("c", "a", "m").unwrap();
    assert_eq!(
        s.fuehre_gebunden_aus("c", "a", "m", "call", || {
            s.begin("c", "a", "m").unwrap();
            s.begin("c", "b", "n").unwrap();
            abgeschlossen()
        }),
        abgeschlossen()
    );
}

#[derive(Default)]
struct Testumgebung(std::sync::atomic::AtomicUsize);

impl super::Aufrufumgebung for Testumgebung {
    fn fuehre(
        &self,
        _: &[super::Wert],
        _: &[String],
        _: &str,
        _: &str,
        _: &mut super::super::anbindung::Antwortrunde,
        abbruch: &super::super::anbindung::Abbruch,
        _: Option<(&super::Freigabe, u64)>,
    ) -> super::super::aufrufkern::Aufrufergebnis {
        self.0.fetch_add(1, Ordering::SeqCst);
        super::super::aufrufkern::Aufrufergebnis {
            ergebnis: if abbruch.ausgeloest() {
                "abgebrochen"
            } else {
                "ok"
            },
            daten: Some(super::super::rahmung::rahme_zufaellig("test", b"antwort", 1024).unwrap()),
            meldung: None,
            protokollzeile: String::new(),
        }
    }
}

#[test]
fn aufruf_wird_nur_einmal_ausgefuehrt_und_argumentwechsel_verweigert() {
    let umgebung = Arc::new(Testumgebung::default());
    let s = InterfaceState::mit_umgebung(Arc::new(Testuhr(AtomicI64::new(0))), umgebung.clone());
    s.begin("c", "a", "m").unwrap();
    let ergebnis = s.call("c", "a", "m", "call", "werkzeug", "{}");
    assert!(matches!(
        &ergebnis,
        Ok(CallResult::Completed { data: Some(_), .. })
    ));
    assert_eq!(s.call("c", "a", "m", "call", "werkzeug", "{}"), ergebnis);
    assert_eq!(
        s.call("c", "a", "m", "call", "werkzeug", "{\"x\":1}"),
        Err(InterfaceError::Widerspruch)
    );
    assert_eq!(umgebung.0.load(Ordering::SeqCst), 1);
    s.end("c").unwrap();
    assert_eq!(
        s.call("c", "a", "m", "call", "werkzeug", "{}"),
        Err(InterfaceError::Beendet)
    );
}

#[test]
fn vorgemerkter_sprachbeginn_erreicht_den_aufruf() {
    let s = InterfaceState::mit_umgebung(
        Arc::new(Testuhr(AtomicI64::new(0))),
        Arc::new(Testumgebung::default()),
    );
    s.begin("c", "a", "m").unwrap();
    s.abbrueche.melde_sprachbeginn("m");
    assert!(matches!(
        s.call("c", "a", "m", "call", "werkzeug", "{}"),
        Ok(CallResult::Completed { outcome, .. }) if outcome == "abgebrochen"
    ));
}

#[test]
fn json_bildet_alle_varianten_und_zahl_fallback_ab() {
    use super::super::ipc::als_json;
    use super::Wert;
    let wert = Wert::Objekt(std::collections::BTreeMap::from([(
        "werte".into(),
        Wert::Liste(vec![
            Wert::Null,
            Wert::Bool(true),
            Wert::Zahl("12.5".into()),
            Wert::Zahl("1e9999".into()),
            Wert::Text("text".into()),
        ]),
    )]));
    assert_eq!(
        als_json(wert),
        serde_json::json!({"werte": [null, true, 12.5, "1e9999", "text"]})
    );
    assert_eq!(
        CallResult::from(super::super::aufrufkern::Aufrufergebnis {
            ergebnis: "unbekannt",
            daten: None,
            meldung: None,
            protokollzeile: String::new(),
        }),
        CallResult::Completed {
            outcome: "netzfehler".into(),
            data: None,
            message: None,
        }
    );
}
