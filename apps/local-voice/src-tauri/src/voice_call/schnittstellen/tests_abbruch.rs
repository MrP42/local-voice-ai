use super::super::abbruch::{Abbrueche, Wirkung, Zugriffsart::*};
use super::super::aeusserung::Rueckfrage;
use super::*;
use std::cell::Cell;

struct Signal<'a, T> {
    register: &'a Abbrueche,
    phase: u8,
    senden: &'a Cell<usize>,
    innen: T,
}
impl Namensaufloesung for Signal<'_, Dns> {
    fn aufloesen(&mut self, host: &str, lauf: &Lauf) -> Result<Vec<String>, &'static str> {
        let adressen = self.innen.aufloesen(host, lauf)?;
        if self.phase == 1 {
            assert_eq!(self.register.melde_sprachbeginn("r"), Wirkung::Abgebrochen);
        }
        lauf.pruefen()?;
        Ok(adressen)
    }
}
impl<'a> Verbindungsaufbau for Signal<'a, Netz> {
    type Verbunden = Signal<'a, Verbunden>;
    fn verbinden(
        &mut self,
        ziel: &Ziel,
        ip: IpAddr,
        lauf: &Lauf,
    ) -> Result<Self::Verbunden, &'static str> {
        if self.phase == 2 {
            assert_eq!(self.register.melde_sprachbeginn("r"), Wirkung::Abgebrochen);
        }
        lauf.pruefen()?;
        Ok(Signal {
            register: self.register,
            phase: self.phase,
            senden: self.senden,
            innen: self.innen.verbinden(ziel, ip, lauf)?,
        })
    }
}
impl Verbindung for Signal<'_, Verbunden> {
    fn senden(&mut self, anfrage: &Wert) -> Result<(), &'static str> {
        self.senden.set(self.senden.get() + 1);
        if self.phase == 4 {
            assert_eq!(
                self.register.melde_sprachbeginn("r"),
                Wirkung::Weiterlaufend
            );
        }
        self.innen.senden(anfrage)
    }
    fn antwort(&mut self, lauf: &Lauf, max: usize) -> Result<Antwort, &'static str> {
        if self.phase == 3 {
            self.innen.empfangen = 17;
            assert_eq!(self.register.melde_sprachbeginn("r"), Wirkung::Abgebrochen);
        }
        lauf.pruefen()?;
        self.innen.antwort(lauf, max)
    }
    fn gesendet_bytes(&self) -> usize {
        self.innen.gesendet_bytes()
    }
    fn empfangen_bytes(&self) -> usize {
        self.innen.empfangen_bytes()
    }
}
#[test]
fn reinsprechen_beachtet_transportphase_und_zugriffsart() {
    for (phase, schreibend, sendefehler, frueh) in [
        (1, false, false, false),
        (2, false, false, false),
        (3, false, false, false),
        (0, false, false, true),
        (4, true, false, false),
        (4, true, true, false),
        (0, true, false, true),
    ] {
        let register = Abbrueche::default();
        if frueh {
            assert_eq!(register.melde_sprachbeginn("r"), Wirkung::Vorgemerkt);
        }
        let art = if schreibend { Schreibend } else { Lesend };
        let handle = register.registriere("r", art).unwrap();
        let mut e = eintraege();
        if schreibend {
            e[0].setzen("zugriff", Wert::from("schreibend"));
            let mut http = e[0]["http"].clone();
            http.setzen("methode", Wert::from("POST"));
            e[0].setzen("http", http);
        }
        let s = Schnappschuss::neu(&e[0], &lies_json("{}").unwrap(), None).unwrap();
        let freigabe = Freigabe::default();
        let generation = freigabe.frage(s.bindung());
        freigabe.aeusserung(freigabe.aeusserungsbeginn(), true);
        let senden = Cell::new(0);
        let mut netz = Signal {
            register: &register,
            phase,
            senden: &senden,
            innen: Netz {
                sendefehler,
                ..Netz::default()
            },
        };
        let mut dns = Signal {
            register: &register,
            phase,
            senden: &senden,
            innen: Dns::default(),
        };
        let a = fuehre_aus(
            &e,
            &["test.api".into()],
            "test_dapi",
            "{}",
            &mut Antwortrunde::neu(),
            handle.abbruch(),
            Some((&freigabe, generation)),
            &AllesErlaubt,
            &mut dns,
            &mut netz,
            "test",
            "de",
            "2026-11-21T12:00:00Z",
        );
        let erwartet = match (schreibend, sendefehler) {
            (false, _) => "abgebrochen",
            (true, true) => "unklar",
            (true, false) => "ok",
        };
        assert_eq!(a.ergebnis, erwartet);
        assert_eq!(handle.abbruch().ausgeloest(), !schreibend);
        assert_eq!(dns.innen.0, usize::from(!frueh || schreibend));
        assert_eq!(senden.get(), usize::from(phase >= 3 || schreibend));
        let p = protokoll(&a);
        assert_eq!(p["ergebnis"].text(), Some(erwartet));
        assert_eq!(p["bestaetigt"].bool(), Some(schreibend));
        let bytes = match (phase, schreibend, sendefehler) {
            (3, _, _) => (137, 17),
            (_, true, false) => (137, 91),
            _ => (0, 0),
        };
        assert_eq!(p["gesendet_bytes"].ganz(), Some(bytes.0));
        assert_eq!(p["empfangen_bytes"].ganz(), Some(bytes.1));
        drop(handle);
        assert!(!register
            .registriere("r", Lesend)
            .unwrap()
            .abbruch()
            .ausgeloest());
    }
}
#[test]
fn vormerkungen_verfallen_ohne_verlaengerung_und_verwerfen_die_aelteste() {
    let uhr = Arc::new(Testuhr(AtomicI64::new(0)));
    let register = Abbrueche::mit_uhr(uhr.clone());
    register.melde_sprachbeginn("alt");
    uhr.0.store(29_999, Ordering::SeqCst);
    register.melde_sprachbeginn("alt");
    register.melde_sprachbeginn("jung");
    uhr.0.store(30_000, Ordering::SeqCst);
    assert!(!register
        .registriere("alt", Lesend)
        .unwrap()
        .abbruch()
        .ausgeloest());
    assert!(register
        .registriere("jung", Lesend)
        .unwrap()
        .abbruch()
        .ausgeloest());
    for i in 0..33 {
        register.melde_sprachbeginn(&i.to_string());
    }
    for id in ["", "a/b", "a b", "ä", &"a".repeat(65)] {
        assert_eq!(register.melde_sprachbeginn(id), Wirkung::Ungueltig);
        assert_eq!(
            register.registriere(id, Lesend).err(),
            Some("kennung_ungueltig")
        );
    }
    for i in 0..33 {
        let h = register.registriere(&i.to_string(), Lesend).unwrap();
        assert_eq!(h.abbruch().ausgeloest(), i != 0);
    }
}
#[test]
fn laufende_aufrufe_sind_eindeutig_begrenzt_und_werden_freigegeben() {
    let register = Abbrueche::default();
    let handles: Vec<_> = (0..64)
        .map(|i| register.registriere(&i.to_string(), Lesend).unwrap())
        .collect();
    assert_eq!(
        register.registriere("0", Lesend).err(),
        Some("laeuft_schon")
    );
    assert_eq!(register.registriere("neu", Lesend).err(), Some("zu_viele"));
    for _ in 0..2 {
        assert_eq!(register.melde_sprachbeginn("0"), Wirkung::Abgebrochen);
    }
    assert!(handles[0].abbruch().ausgeloest());
    drop(handles);
    assert!(!register
        .registriere("0", Lesend)
        .unwrap()
        .abbruch()
        .ausgeloest());
}
#[test]
fn paralleler_sprachbeginn_geht_nicht_verloren() {
    let register = Abbrueche::default();
    for _ in 0..100 {
        let start = Barrier::new(2);
        std::thread::scope(|scope| {
            let signal = scope.spawn(|| {
                start.wait();
                register.melde_sprachbeginn("r")
            });
            start.wait();
            let h = register.registriere("r", Lesend).unwrap();
            assert!(matches!(
                signal.join().unwrap(),
                Wirkung::Vorgemerkt | Wirkung::Abgebrochen
            ));
            assert!(h.abbruch().ausgeloest());
        });
    }
}
#[test]
fn ausdrueckliches_nein_verhindert_den_versand() {
    let register = Abbrueche::default();
    let freigabe = Freigabe::default();
    let frage = Rueckfrage::neu(&freigabe, "bindung", "k", "q").unwrap();
    let generation = frage.ereignis().generation;
    freigabe.aeusserung(freigabe.aeusserungsbeginn(), true);
    register.lehne_ab(&frage, &freigabe);
    assert_eq!(
        freigabe.absenden(generation, "bindung", || ()),
        Err("bestaetigung_abgelehnt")
    );
}
