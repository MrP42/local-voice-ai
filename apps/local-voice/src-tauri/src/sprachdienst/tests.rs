//! Tests des Sprachdienstes. Die meisten laufen über einen In-Memory-Strom (`duplex`) und damit
//! auf jedem System; die mit echten Sockets (Rechte, Peer-Prüfung, Interop mit dem Client des SDK)
//! nur unter Unix. Der Pipe-Teil unter Windows (DACL, Prüfung der Gegenstelle) ist nur dort prüfbar.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::watch;
use tokio::time::timeout;
use wai_voice_dienst::beweis;
use wai_voice_dienst::frame::{Art, Decoder, Rahmen, Rolle};
use wai_voice_dienst::grenzen;
use wai_voice_dienst::msg::{
    code, AnApp, AnDienst, Engines, Operation, Recht, Souveraenitaet, Wortpaar,
};
use wai_voice_dienst::PROTOKOLL;

use super::engine::{EngineFehler, Sprache};
use super::kontext::{Konfig, Kontext, Meldung};
use super::verbindung::verbindung;
use super::wav;
use super::zugang::{Zugang, ZugangStore};

const CLIENT: &str = "corporate-llm";
const SCHLUESSEL: &str = "lvat_test_schluessel";
const BEREICH: &str = "projekt-a";

fn jetzt() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn fp_hex() -> String {
    hex(&beweis::fingerabdruck(SCHLUESSEL))
}

// --- Attrappe der Engines -----------------------------------------------------------------------------

struct Attrappe {
    lokal: AtomicBool,
    bereit: AtomicBool,
    verzoegerung: Mutex<Duration>,
    /// Länge des PCM-Audios, das `sprechen` liefert.
    sprech_bytes: usize,
    erkannt: Mutex<Vec<usize>>,
    /// Statt der Byte-Zahl diesen Text liefern (Test des Logs).
    antwort: Mutex<Option<String>>,
}

impl Attrappe {
    fn neu() -> Arc<Self> {
        Arc::new(Self {
            lokal: AtomicBool::new(true),
            bereit: AtomicBool::new(true),
            verzoegerung: Mutex::new(Duration::ZERO),
            sprech_bytes: 70_000,
            erkannt: Mutex::new(Vec::new()),
            antwort: Mutex::new(None),
        })
    }
}

impl Sprache for Attrappe {
    fn engines(&self) -> Engines {
        Engines {
            erkennung: "attrappe".into(),
            stimme: "attrappe".into(),
            lokal: self.lokal.load(Ordering::SeqCst),
        }
    }

    fn bereit(&self) -> bool {
        self.bereit.load(Ordering::SeqCst)
    }

    /// Der Text ist die Zahl der empfangenen Bytes: Das Audio kam vollständig an.
    fn erkennen(&self, pcm: &[u8], _bereich: &str) -> Result<String, EngineFehler> {
        let v = *self.verzoegerung.lock().unwrap();
        std::thread::sleep(v);
        self.erkannt.lock().unwrap().push(pcm.len());
        if let Some(a) = self.antwort.lock().unwrap().clone() {
            return Ok(a);
        }
        Ok(pcm.len().to_string())
    }

    fn sprechen(&self, _text: &str, _stimme: Option<&str>) -> Result<Vec<Vec<u8>>, EngineFehler> {
        Ok(vec![wav::wav_aus_pcm(
            &vec![3u8; self.sprech_bytes],
            1,
            16_000,
        )])
    }
}

// --- Prüfstand ------------------------------------------------------------------------------------------

struct Stand {
    ctx: Arc<Kontext>,
    engine: Arc<Attrappe>,
    meldungen: Arc<Mutex<Vec<Meldung>>>,
    stopp: watch::Sender<bool>,
}

fn stand_mit(anpassen: impl FnOnce(&mut Konfig)) -> Stand {
    let mut konfig = Konfig::standard("0.0.0-test");
    anpassen(&mut konfig);
    let engine = Attrappe::neu();
    let meldungen = Arc::new(Mutex::new(Vec::new()));
    let m2 = meldungen.clone();
    let ctx = Kontext::new(
        konfig,
        Arc::new(ZugangStore::im_speicher()),
        engine.clone(),
        jetzt,
        Box::new(move |m| m2.lock().unwrap().push(m)),
    );
    let (stopp, _) = watch::channel(false);
    Stand {
        ctx,
        engine,
        meldungen,
        stopp,
    }
}

fn stand() -> Stand {
    stand_mit(|_| {})
}

impl Stand {
    fn verbinden(&self) -> Kunde {
        let (a, b) = duplex(1 << 20);
        let (ctx, stopp) = (self.ctx.clone(), self.stopp.subscribe());
        tokio::spawn(async move { verbindung(b, ctx, stopp).await });
        Kunde::neu(a)
    }

    /// Eine gekoppelte App mit Rechten, so als hätte der Nutzer sie freigegeben.
    fn gekoppelt(&self, rechte: &[Recht]) -> Zugang {
        let z = self
            .ctx
            .zugaenge
            .hinzufuegen(CLIENT, "Corporate LLM", &fp_hex(), jetzt())
            .unwrap();
        for r in rechte {
            self.ctx.zugaenge.recht_setzen(&z.zugang, *r, true).unwrap();
        }
        self.ctx.zugaenge.get(&z.zugang).unwrap()
    }

    async fn kopplung_abwarten(&self) -> super::kopplung::OffeneKopplung {
        for _ in 0..200 {
            if let Some(o) = self.ctx.kopplungen.offen(jetzt()).into_iter().next() {
                return o;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("keine Kopplungsanfrage");
    }
}

// --- Roher Client -----------------------------------------------------------------------------------------

struct Kunde {
    s: DuplexStream,
    dec: Decoder,
    rest: Vec<u8>,
}

impl Kunde {
    fn neu(s: DuplexStream) -> Self {
        Self {
            s,
            dec: Decoder::new(Rolle::App),
            rest: Vec::new(),
        }
    }

    async fn roh(&mut self, bytes: &[u8]) {
        let _ = self.s.write_all(bytes).await;
        let _ = self.s.flush().await;
    }

    async fn json(&mut self, strom: u32, json: &str) {
        let bytes = Rahmen::steuer(strom, json.as_bytes().to_vec())
            .kodieren()
            .unwrap();
        self.roh(&bytes).await;
    }

    async fn senden(&mut self, m: &AnDienst) {
        let json = serde_json::to_string(m).unwrap();
        self.json(m.strom(), &json).await;
    }

    fn audio_bytes(strom: u32, pcm: Vec<u8>) -> Vec<u8> {
        Rahmen {
            art: Art::AudioHinein,
            strom,
            nutzlast: pcm,
        }
        .kodieren()
        .unwrap()
    }

    async fn audio(&mut self, strom: u32, pcm: Vec<u8>) {
        self.roh(&Self::audio_bytes(strom, pcm)).await;
    }

    /// Nächster Rahmen; `None` bei Ende der Leitung. Nach 5 s ohne etwas bricht der Test ab.
    async fn rahmen(&mut self) -> Option<Rahmen> {
        timeout(Duration::from_secs(5), self.rahmen_ohne_frist())
            .await
            .expect("Dienst antwortet nicht")
    }

    async fn rahmen_ohne_frist(&mut self) -> Option<Rahmen> {
        loop {
            if !self.rest.is_empty() {
                let n = self
                    .dec
                    .einspeisen(&self.rest)
                    .expect("Rahmen des Dienstes ungültig");
                self.rest.drain(..n);
            }
            if let Some(r) = self.dec.naechster().expect("Rahmen des Dienstes ungültig") {
                return Some(r);
            }
            if !self.rest.is_empty() {
                continue;
            }
            let mut buf = [0u8; 8192];
            match self.s.read(&mut buf).await {
                Ok(0) | Err(_) => return None,
                Ok(n) => self.rest.extend_from_slice(&buf[..n]),
            }
        }
    }

    /// Nächster Rahmen innerhalb von `ms`, sonst `None`.
    async fn rahmen_binnen(&mut self, ms: u64) -> Option<Option<Rahmen>> {
        timeout(Duration::from_millis(ms), self.rahmen_ohne_frist())
            .await
            .ok()
    }

    async fn steuer(&mut self) -> AnApp {
        let r = self.rahmen().await.expect("Leitung zu");
        assert_eq!(r.art, Art::Steuer, "Steuerrahmen erwartet");
        let m: AnApp = serde_json::from_slice(&r.nutzlast).expect("Nachricht des Dienstes");
        assert_eq!(m.strom(), r.strom, "Strom in Kopf und Nachricht");
        m
    }

    async fn gruss(&mut self) -> String {
        match self.steuer().await {
            AnApp::Gruss { protokoll, zufall } => {
                assert_eq!(protokoll, PROTOKOLL);
                assert_eq!(zufall.len(), 64);
                zufall
            }
            m => panic!("Gruss erwartet, bekam {m:?}"),
        }
    }

    async fn hallo(&mut self, zugang: &str, client: &str, schluessel: &str) {
        let zufall = self.gruss().await;
        let fp = beweis::fingerabdruck(schluessel);
        self.senden(&AnDienst::Hallo {
            protokoll: PROTOKOLL,
            client: client.into(),
            zugang: zugang.into(),
            beweis: beweis::erzeugen(&fp, &zufall, client),
        })
        .await;
    }

    /// Handschlag mit dem richtigen Schlüssel; erwartet `willkommen`.
    async fn anmelden(&mut self, z: &Zugang) -> Vec<Recht> {
        self.hallo(&z.zugang, CLIENT, SCHLUESSEL).await;
        match self.steuer().await {
            AnApp::Willkommen { rechte, .. } => rechte,
            m => panic!("willkommen erwartet, bekam {m:?}"),
        }
    }

    async fn leitung_zu(&mut self) {
        assert!(self.rahmen().await.is_none(), "die Verbindung sollte enden");
    }

    async fn fehler(&mut self) -> (u32, String) {
        match self.steuer().await {
            AnApp::Fehler { strom, code, .. } => (strom, code),
            m => panic!("Fehler erwartet, bekam {m:?}"),
        }
    }

    async fn start(&mut self, strom: u32, op: Operation, souv: Souveraenitaet) {
        self.senden(&AnDienst::Start {
            strom,
            op,
            bereich: BEREICH.into(),
            souveraenitaet: souv,
        })
        .await;
    }

    async fn guthaben_erwarten(&mut self, strom: u32) -> u32 {
        match self.steuer().await {
            AnApp::Guthaben { strom: s, rahmen } => {
                assert_eq!(s, strom);
                rahmen
            }
            m => panic!("Guthaben erwartet, bekam {m:?}"),
        }
    }
}

fn koppeln_nachricht(client: &str, anzeige: &str, fp: &str, code_text: &str) -> AnDienst {
    AnDienst::Koppeln {
        protokoll: PROTOKOLL,
        client: client.into(),
        anzeige: anzeige.into(),
        fingerabdruck: fp.into(),
        code: code_text.into(),
    }
}

// --- Handschlag -----------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handschlag_mit_hmac_beweis_meldet_rechte_und_engines() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.hallo(&z.zugang, CLIENT, SCHLUESSEL).await;
    match k.steuer().await {
        AnApp::Willkommen {
            protokoll,
            dienst,
            version,
            rechte,
            engines,
        } => {
            assert_eq!(protokoll, PROTOKOLL);
            assert_eq!(dienst, "local-voice-ai");
            assert_eq!(version, "0.0.0-test");
            assert_eq!(rechte, vec![Recht::Hoeren]);
            assert!(engines.lokal);
        }
        m => panic!("{m:?}"),
    }
    assert!(
        st.ctx.zugaenge.get(&z.zugang).unwrap().zuletzt_ms.is_some(),
        "letzte Nutzung vermerkt"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn falscher_schluessel_unbekannter_zugang_und_fremder_client_sind_ungueltig() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);

    let mut k = st.verbinden();
    k.hallo(&z.zugang, CLIENT, "lvat_anderer_schluessel").await;
    assert_eq!(k.fehler().await, (0, code::SCHLUESSEL_UNGUELTIG.into()));
    k.leitung_zu().await;

    let mut k = st.verbinden();
    k.hallo("z-000000000000", CLIENT, SCHLUESSEL).await;
    assert_eq!(
        k.fehler().await,
        (0, code::SCHLUESSEL_UNGUELTIG.into()),
        "gleiche Antwort wie bei falschem Schlüssel"
    );
    k.leitung_zu().await;

    // Richtiger Schlüssel, aber unter einem anderen Client-Namen als bei der Kopplung.
    let mut k = st.verbinden();
    k.hallo(&z.zugang, "poststelle", SCHLUESSEL).await;
    assert_eq!(k.fehler().await, (0, code::SCHLUESSEL_UNGUELTIG.into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mitgeschnittener_beweis_gilt_nur_fuer_seine_verbindung() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut a = st.verbinden();
    let zufall_a = a.gruss().await;
    let beweis_a = beweis::erzeugen(&beweis::fingerabdruck(SCHLUESSEL), &zufall_a, CLIENT);

    let mut b = st.verbinden();
    let zufall_b = b.gruss().await;
    assert_ne!(zufall_a, zufall_b, "je Verbindung ein neuer Zufallswert");
    b.senden(&AnDienst::Hallo {
        protokoll: PROTOKOLL,
        client: CLIENT.into(),
        zugang: z.zugang.clone(),
        beweis: beweis_a,
    })
    .await;
    assert_eq!(b.fehler().await, (0, code::SCHLUESSEL_UNGUELTIG.into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn falsche_protokollversion_und_vertragsbrueche_beenden_die_verbindung() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);

    let mut k = st.verbinden();
    k.gruss().await;
    k.json(
        0,
        &format!(
            r#"{{"t":"hallo","protokoll":99,"client":"{CLIENT}","zugang":"{}","beweis":"{}"}}"#,
            z.zugang,
            "a".repeat(64)
        ),
    )
    .await;
    assert_eq!(k.fehler().await, (0, code::VERSION_INKOMPATIBEL.into()));
    k.leitung_zu().await;

    // Start vor dem Handschlag.
    let mut k = st.verbinden();
    k.gruss().await;
    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    assert_eq!(k.fehler().await, (0, code::NICHT_ANGEMELDET.into()));
    k.leitung_zu().await;

    // Unbekanntes Feld und kaputtes JSON.
    let mut k = st.verbinden();
    k.gruss().await;
    k.json(0, r#"{"t":"status","x":1}"#).await;
    assert_eq!(k.fehler().await, (0, code::UNGUELTIG.into()));
    k.leitung_zu().await;

    // Verletzter Rahmenkopf.
    let mut k = st.verbinden();
    k.gruss().await;
    k.roh(&[0u8; 12]).await;
    assert_eq!(k.fehler().await, (0, code::UNGUELTIG.into()));
    k.leitung_zu().await;

    // Audio für einen Strom, der nie geöffnet wurde.
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.audio(5, vec![0; 320]).await;
    assert_eq!(k.fehler().await, (0, code::STROM_UNBEKANNT.into()));
    k.leitung_zu().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fehler_auf_strom_0_ist_endgueltig_fehler_eines_stroms_nicht() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);

    // Fehler eines Stroms: die Verbindung lebt weiter.
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(
        1,
        Operation::Sprechen {
            text: "Hallo".into(),
            stimme: None,
        },
        Souveraenitaet::NurLokal,
    )
    .await;
    assert_eq!(k.fehler().await, (1, code::RECHT_FEHLT.into()));
    k.senden(&AnDienst::Status {}).await;
    assert!(matches!(
        k.steuer().await,
        AnApp::Status { bereit: true, .. }
    ));

    // Fehler auf Strom 0: danach kommt nichts mehr, auch kein Status.
    k.senden(&AnDienst::Ende { strom: 9 }).await;
    assert_eq!(k.fehler().await, (0, code::STROM_UNBEKANNT.into()));
    k.senden(&AnDienst::Status {}).await;
    k.leitung_zu().await;
}

// --- Widerruf ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn widerrufener_zugang_meldet_sich_nicht_mehr_an() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);
    st.ctx.zugaenge.widerrufen(&z.zugang, jetzt()).unwrap();
    let mut k = st.verbinden();
    k.hallo(&z.zugang, CLIENT, SCHLUESSEL).await;
    assert_eq!(k.fehler().await, (0, code::SCHLUESSEL_WIDERRUFEN.into()));
    k.leitung_zu().await;
    // Mit falschem Schlüssel erfährt niemand, dass der Zugang widerrufen ist.
    let mut k = st.verbinden();
    k.hallo(&z.zugang, CLIENT, "falsch").await;
    assert_eq!(k.fehler().await, (0, code::SCHLUESSEL_UNGUELTIG.into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn widerruf_beendet_laufende_verbindungen_sofort() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    st.ctx.zugaenge.widerrufen(&z.zugang, jetzt()).unwrap();
    st.ctx.verbindungen_beenden(&z.zugang);
    assert_eq!(k.fehler().await, (0, code::SCHLUESSEL_WIDERRUFEN.into()));
    k.leitung_zu().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn widerruf_wird_bei_jedem_start_geprueft_und_rechte_gelten_live() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;

    // Recht später freigegeben: gilt ab dem nächsten Start, ohne neuen Handschlag.
    k.start(
        1,
        Operation::Sprechen {
            text: "Hallo".into(),
            stimme: None,
        },
        Souveraenitaet::NurLokal,
    )
    .await;
    assert_eq!(k.fehler().await, (1, code::RECHT_FEHLT.into()));
    st.ctx
        .zugaenge
        .recht_setzen(&z.zugang, Recht::Sprechen, true)
        .unwrap();
    k.start(
        2,
        Operation::Sprechen {
            text: "Hallo".into(),
            stimme: None,
        },
        Souveraenitaet::NurLokal,
    )
    .await;
    k.senden(&AnDienst::Guthaben {
        strom: 2,
        rahmen: 64,
    })
    .await;
    loop {
        let r = k.rahmen().await.expect("Leitung zu");
        if r.art == Art::Steuer {
            assert!(matches!(
                serde_json::from_slice(&r.nutzlast),
                Ok(AnApp::Fertig { strom: 2 })
            ));
            break;
        }
    }

    // Widerruf ohne Meldung an die Verbindung: spätestens der nächste Start fällt auf.
    st.ctx.zugaenge.widerrufen(&z.zugang, jetzt()).unwrap();
    k.start(3, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    assert_eq!(k.fehler().await, (0, code::SCHLUESSEL_WIDERRUFEN.into()));
    k.leitung_zu().await;
}

// --- Kopplung ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kopplung_wird_in_der_oberflaeche_bestaetigt() {
    let st = stand();
    let mut k = st.verbinden();
    k.gruss().await;
    k.senden(&koppeln_nachricht(
        CLIENT,
        "Corporate LLM",
        &fp_hex(),
        "123456",
    ))
    .await;

    let offen = st.kopplung_abwarten().await;
    assert_eq!(
        (
            offen.client.as_str(),
            offen.anzeige.as_str(),
            offen.code.as_str()
        ),
        (CLIENT, "Corporate LLM", "123456")
    );
    assert!(offen.bis_ms > jetzt() + 100_000, "Frist von 120 s");
    assert!(st
        .meldungen
        .lock()
        .unwrap()
        .contains(&Meldung::KopplungOffen));
    assert!(
        st.ctx.zugaenge.liste().is_empty(),
        "vor der Bestätigung gibt es keinen Zugang"
    );

    assert!(st.ctx.kopplungen.entscheiden(&offen.id, true, jetzt()));
    let zugang = match k.steuer().await {
        AnApp::Gekoppelt { zugang } => zugang,
        m => panic!("{m:?}"),
    };
    k.leitung_zu().await;
    let z = st.ctx.zugaenge.get(&zugang).unwrap();
    assert_eq!(z.fingerabdruck, fp_hex());
    assert!(z.rechte.is_empty(), "Rechte vergibt der Nutzer getrennt");
    assert!(st.ctx.kopplungen.offen(jetzt()).is_empty());

    // Mit dem Schlüssel der App geht jetzt der Handschlag; ohne Recht kein Start.
    let mut k = st.verbinden();
    assert!(k.anmelden(&z).await.is_empty());
    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    assert_eq!(k.fehler().await, (1, code::RECHT_FEHLT.into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kopplung_ablehnen() {
    let st = stand();
    let mut k = st.verbinden();
    k.gruss().await;
    k.senden(&koppeln_nachricht(
        CLIENT,
        "Corporate LLM",
        &fp_hex(),
        "123456",
    ))
    .await;
    let offen = st.kopplung_abwarten().await;
    assert!(st.ctx.kopplungen.entscheiden(&offen.id, false, jetzt()));
    assert_eq!(k.fehler().await, (0, code::ABGELEHNT.into()));
    k.leitung_zu().await;
    assert!(st.ctx.zugaenge.liste().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kopplung_laeuft_nach_der_frist_ab() {
    let st = stand_mit(|k| k.bestaetigung = Duration::from_millis(150));
    let mut k = st.verbinden();
    k.gruss().await;
    k.senden(&koppeln_nachricht(
        CLIENT,
        "Corporate LLM",
        &fp_hex(),
        "123456",
    ))
    .await;
    let offen = st.kopplung_abwarten().await;
    assert_eq!(k.fehler().await, (0, code::ZEIT_UEBERSCHRITTEN.into()));
    k.leitung_zu().await;
    assert!(
        st.ctx.kopplungen.offen(jetzt()).is_empty(),
        "abgelaufene Anfrage verschwindet aus dem Dialog"
    );
    assert!(
        !st.ctx.kopplungen.entscheiden(&offen.id, true, jetzt()),
        "zu spät bestätigt gilt nicht"
    );
    assert!(st.ctx.zugaenge.liste().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn die_standardfrist_ist_120_sekunden() {
    assert_eq!(Konfig::standard("x").bestaetigung, Duration::from_secs(120));
    assert_eq!(grenzen::KOPPELN_BESTAETIGUNG_S, 120);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kopplung_ratenbremse_drei_je_minute_und_eine_offene() {
    let st = stand_mit(|k| k.bestaetigung = Duration::from_millis(30));
    for versuch in 1..=3 {
        let mut k = st.verbinden();
        k.gruss().await;
        k.senden(&koppeln_nachricht(
            CLIENT,
            "Corporate LLM",
            &fp_hex(),
            "123456",
        ))
        .await;
        assert_eq!(
            k.fehler().await,
            (0, code::ZEIT_UEBERSCHRITTEN.into()),
            "Versuch {versuch}"
        );
    }
    let mut k = st.verbinden();
    k.gruss().await;
    k.senden(&koppeln_nachricht(
        CLIENT,
        "Corporate LLM",
        &fp_hex(),
        "123456",
    ))
    .await;
    assert_eq!(
        k.fehler().await,
        (0, code::BESCHAEFTIGT.into()),
        "vierter Versuch binnen einer Minute"
    );
    k.leitung_zu().await;

    // Ein anderer Client ist von der Bremse dieses Clients nicht betroffen.
    let mut k = st.verbinden();
    k.gruss().await;
    k.senden(&koppeln_nachricht(
        "poststelle",
        "Poststelle",
        &fp_hex(),
        "123456",
    ))
    .await;
    assert_eq!(k.fehler().await, (0, code::ZEIT_UEBERSCHRITTEN.into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zweite_offene_kopplung_desselben_clients_ist_beschaeftigt() {
    let st = stand();
    let mut a = st.verbinden();
    a.gruss().await;
    a.senden(&koppeln_nachricht(
        CLIENT,
        "Corporate LLM",
        &fp_hex(),
        "123456",
    ))
    .await;
    let offen = st.kopplung_abwarten().await;

    let mut b = st.verbinden();
    b.gruss().await;
    b.senden(&koppeln_nachricht(
        CLIENT,
        "Corporate LLM",
        &"b".repeat(64),
        "654321",
    ))
    .await;
    assert_eq!(b.fehler().await, (0, code::BESCHAEFTIGT.into()));
    assert_eq!(
        st.ctx.kopplungen.offen(jetzt()).len(),
        1,
        "es bleibt bei einem Dialog"
    );

    assert!(st.ctx.kopplungen.entscheiden(&offen.id, false, jetzt()));
    assert_eq!(a.fehler().await, (0, code::ABGELEHNT.into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn schon_gekoppelter_fingerabdruck_bekommt_bereits_gekoppelt() {
    let st = stand();
    st.gekoppelt(&[]);
    let mut k = st.verbinden();
    k.gruss().await;
    k.senden(&koppeln_nachricht(
        CLIENT,
        "Corporate LLM",
        &fp_hex(),
        "123456",
    ))
    .await;
    assert_eq!(k.fehler().await, (0, code::BEREITS_GEKOPPELT.into()));
    k.leitung_zu().await;
    assert!(st.ctx.kopplungen.offen(jetzt()).is_empty(), "kein Dialog");
    assert_eq!(st.ctx.zugaenge.liste().len(), 1, "kein zweiter Zugang");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ungueltige_anzeige_wird_abgewiesen_bevor_ein_dialog_entsteht() {
    let st = stand();
    for anzeige in [
        "B\u{202E}ose",
        "Null\u{200B}breite",
        "<b>fett</b>",
        "Zeile\nzwei",
        " ",
        "",
        &"x".repeat(65),
    ] {
        let mut k = st.verbinden();
        k.gruss().await;
        // Roh gesendet, ohne die Prüfung des Clients: der Dienst prüft selbst.
        let json = serde_json::to_string(&koppeln_nachricht(CLIENT, anzeige, &fp_hex(), "123456"))
            .unwrap();
        k.json(0, &json).await;
        assert_eq!(k.fehler().await, (0, code::UNGUELTIG.into()), "{anzeige:?}");
        k.leitung_zu().await;
        assert!(st.ctx.kopplungen.offen(jetzt()).is_empty(), "{anzeige:?}");
    }
    // Erlaubt sind Buchstaben, Ziffern, Leerzeichen und -_.()&+/
    let mut k = st.verbinden();
    k.gruss().await;
    k.senden(&koppeln_nachricht(
        CLIENT,
        "Müller & Söhne (Test)-1.0_a+b/c",
        &fp_hex(),
        "123456",
    ))
    .await;
    let offen = st.kopplung_abwarten().await;
    assert_eq!(offen.anzeige, "Müller & Söhne (Test)-1.0_a+b/c");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn aufgelegte_kopplung_verschwindet_aus_dem_dialog() {
    let st = stand();
    let mut k = st.verbinden();
    k.gruss().await;
    k.senden(&koppeln_nachricht(
        CLIENT,
        "Corporate LLM",
        &fp_hex(),
        "123456",
    ))
    .await;
    st.kopplung_abwarten().await;
    drop(k);
    for _ in 0..200 {
        if st.ctx.kopplungen.offen(jetzt()).is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("Dialog hängt");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn waehrend_der_kopplung_sind_keine_weiteren_nachrichten_erlaubt() {
    let st = stand();
    let mut k = st.verbinden();
    k.gruss().await;
    k.senden(&koppeln_nachricht(
        CLIENT,
        "Corporate LLM",
        &fp_hex(),
        "123456",
    ))
    .await;
    st.kopplung_abwarten().await;
    k.senden(&AnDienst::Status {}).await;
    assert_eq!(k.fehler().await, (0, code::UNGUELTIG.into()));
    assert!(st.ctx.kopplungen.offen(jetzt()).is_empty());
}

// --- Erkennen, Guthaben, Fristen ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn startguthaben_kommt_ausdruecklich_und_audio_wird_erkannt() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    // Das Erste nach `start` ist das ausdrückliche Startguthaben.
    assert_eq!(k.guthaben_erwarten(1).await, grenzen::GUTHABEN_START);
    for _ in 0..3 {
        k.audio(1, vec![0; 3200]).await;
    }
    k.senden(&AnDienst::Ende { strom: 1 }).await;
    // Dazwischen darf Guthaben zurückkommen, das Ergebnis ist das Letzte.
    loop {
        match k.steuer().await {
            AnApp::Guthaben { strom: 1, .. } => continue,
            AnApp::Ergebnis { strom: 1, text } => {
                assert_eq!(text, "9600", "alles Audio kam an");
                break;
            }
            m => panic!("{m:?}"),
        }
    }
    assert_eq!(*st.engine.erkannt.lock().unwrap(), vec![9600]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audio_ueber_das_guthaben_hinaus_ist_kein_guthaben() {
    let st = stand_mit(|k| k.takt = Duration::from_secs(30));
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    assert_eq!(k.guthaben_erwarten(1).await, 16);
    let mut alles = Vec::new();
    for _ in 0..17 {
        alles.extend(Kunde::audio_bytes(1, vec![0; 320]));
    }
    k.roh(&alles).await;
    assert_eq!(
        k.fehler().await,
        (0, code::KEIN_GUTHABEN.into()),
        "der 17. Rahmen hat kein Guthaben"
    );
    k.leitung_zu().await;
    assert!(
        st.engine.erkannt.lock().unwrap().is_empty(),
        "nichts davon wurde erkannt"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn guthaben_kommt_im_takt_zurueck() {
    let st = stand_mit(|k| k.takt = Duration::from_millis(20));
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    let mut guthaben = k.guthaben_erwarten(1).await;
    let mut gesendet = 0;
    // 100 Rahmen sind nur mit zurückgegebenem Guthaben möglich.
    while gesendet < 100 {
        if guthaben == 0 {
            guthaben += k.guthaben_erwarten(1).await;
            continue;
        }
        k.audio(1, vec![0; 320]).await;
        guthaben -= 1;
        gesendet += 1;
    }
    k.senden(&AnDienst::Ende { strom: 1 }).await;
    loop {
        match k.steuer().await {
            AnApp::Guthaben { .. } => continue,
            AnApp::Ergebnis { text, .. } => {
                assert_eq!(text, "32000");
                break;
            }
            m => panic!("{m:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn teilergebnisse_beim_diktat() {
    let st = stand_mit(|k| {
        k.takt = Duration::from_millis(20);
        k.teil_abstand = 3200;
    });
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(1, Operation::ErkennenStrom, Souveraenitaet::NurLokal)
        .await;
    assert_eq!(k.guthaben_erwarten(1).await, 16);
    for _ in 0..4 {
        k.audio(1, vec![0; 3200]).await;
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    k.senden(&AnDienst::Ende { strom: 1 }).await;
    let mut teile = Vec::new();
    loop {
        match k.steuer().await {
            AnApp::Guthaben { .. } => {}
            AnApp::Teil { strom: 1, text } => teile.push(text.parse::<usize>().unwrap()),
            AnApp::Ergebnis { strom: 1, text } => {
                assert_eq!(text, "12800");
                break;
            }
            m => panic!("{m:?}"),
        }
    }
    assert!(!teile.is_empty(), "mindestens ein Teilergebnis");
    assert!(teile.windows(2).all(|w| w[0] < w[1]), "wachsend: {teile:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hoechstdauer_der_aeusserung_liefert_das_ergebnis_sofort() {
    let st = stand_mit(|k| {
        k.aeusserung_max_s = 1;
        k.takt = Duration::from_secs(30);
    });
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    k.guthaben_erwarten(1).await;
    k.audio(1, vec![0; grenzen::AUDIO_MAX]).await;
    assert!(matches!(k.steuer().await, AnApp::Ergebnis { strom: 1, text } if text == "32000"));
    // Rahmen, die noch unterwegs waren, sind kein Vertragsbruch.
    k.audio(1, vec![0; 3200]).await;
    k.senden(&AnDienst::Status {}).await;
    assert!(matches!(k.steuer().await, AnApp::Status { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leerlauf_beendet_den_strom_nicht_die_verbindung() {
    let st = stand_mit(|k| {
        k.leerlauf_strom = Duration::from_millis(100);
        k.takt = Duration::from_millis(20);
    });
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    k.guthaben_erwarten(1).await;
    assert_eq!(k.fehler().await, (1, code::ZEIT_UEBERSCHRITTEN.into()));
    k.senden(&AnDienst::Status {}).await;
    assert!(matches!(k.steuer().await, AnApp::Status { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ohne_handschlag_endet_die_verbindung_nach_der_frist() {
    let st = stand_mit(|k| {
        k.anfang = Duration::from_millis(100);
        k.takt = Duration::from_millis(20);
    });
    let mut k = st.verbinden();
    k.gruss().await;
    assert_eq!(k.fehler().await, (0, code::ZEIT_UEBERSCHRITTEN.into()));
    k.leitung_zu().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abbrechen_beendet_den_strom_sofort_und_unterdrueckt_das_ergebnis() {
    let st = stand();
    *st.engine.verzoegerung.lock().unwrap() = Duration::from_millis(300);
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    k.guthaben_erwarten(1).await;
    k.audio(1, vec![0; 3200]).await;
    k.senden(&AnDienst::Ende { strom: 1 }).await;
    k.senden(&AnDienst::Abbrechen { strom: 1 }).await;
    assert_eq!(k.fehler().await, (1, code::ABGEBROCHEN.into()));
    assert!(
        k.rahmen_binnen(700).await.is_none(),
        "kein Ergebnis nach dem Abbruch"
    );
    // Abbrechen eines unbekannten Stroms ist erlaubt und still.
    k.senden(&AnDienst::Abbrechen { strom: 7 }).await;
    k.senden(&AnDienst::Status {}).await;
    assert!(matches!(k.steuer().await, AnApp::Status { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn souveraenitaet_nur_lokal_verlangt_lokale_engines() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);
    st.engine.lokal.store(false, Ordering::SeqCst);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    assert_eq!(
        k.fehler().await,
        (1, code::NICHT_LOKAL.into()),
        "kein stilles Ausweichen"
    );
    k.start(2, Operation::Erkennen, Souveraenitaet::CloudErlaubt)
        .await;
    assert_eq!(k.guthaben_erwarten(2).await, 16);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ohne_engine_gibt_es_engine_fehlt() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);
    st.engine.bereit.store(false, Ordering::SeqCst);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    assert_eq!(k.fehler().await, (1, code::ENGINE_FEHLT.into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rueckmeldung_braucht_das_recht_lernen() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Hoeren]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.senden(&AnDienst::Rueckmeldung {
        bereich: BEREICH.into(),
        paare: vec![Wortpaar {
            gehoert: "Mayer".into(),
            gemeint: "Meier".into(),
        }],
    })
    .await;
    assert_eq!(k.fehler().await, (0, code::RECHT_FEHLT.into()));
    k.leitung_zu().await;
}

// --- Sprechen -------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sprechen_liefert_audio_nur_mit_guthaben_der_app_und_schliesst_mit_fertig() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Sprechen]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(
        1,
        Operation::Sprechen {
            text: "Guten Tag".into(),
            stimme: Some("provider:phonon:anton".into()),
        },
        Souveraenitaet::NurLokal,
    )
    .await;
    assert!(
        k.rahmen_binnen(300).await.is_none(),
        "ohne Guthaben der App kein Audio"
    );

    k.senden(&AnDienst::Guthaben {
        strom: 1,
        rahmen: 1,
    })
    .await;
    let r = k.rahmen().await.unwrap();
    assert_eq!((r.art, r.strom), (Art::AudioHeraus, 1));
    assert!(
        r.nutzlast.len() <= grenzen::AUDIO_MAX && r.nutzlast.starts_with(b"RIFF"),
        "vollständiges WAV-Häppchen"
    );
    assert!(
        k.rahmen_binnen(300).await.is_none(),
        "genau ein Rahmen je gewährtem Guthaben"
    );

    k.senden(&AnDienst::Guthaben {
        strom: 1,
        rahmen: 5,
    })
    .await;
    let mut audio = 1;
    loop {
        let r = k.rahmen().await.unwrap();
        match r.art {
            Art::AudioHeraus => audio += 1,
            Art::Steuer => {
                let m: AnApp = serde_json::from_slice(&r.nutzlast).unwrap();
                assert!(matches!(m, AnApp::Fertig { strom: 1 }), "{m:?}");
                break;
            }
            Art::AudioHinein => panic!("falsche Richtung"),
        }
    }
    assert_eq!(audio, 3, "70 000 Bytes in Häppchen unter 32 000 Bytes");
    // Der Strom ist zu Ende, die Verbindung lebt.
    k.senden(&AnDienst::Status {}).await;
    assert!(matches!(k.steuer().await, AnApp::Status { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abbrechen_beim_sprechen_verwirft_wartendes_audio() {
    let st = stand();
    let z = st.gekoppelt(&[Recht::Sprechen]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;
    k.start(
        1,
        Operation::Sprechen {
            text: "Hallo".into(),
            stimme: None,
        },
        Souveraenitaet::NurLokal,
    )
    .await;
    k.senden(&AnDienst::Guthaben {
        strom: 1,
        rahmen: 1,
    })
    .await;
    assert_eq!(k.rahmen().await.unwrap().art, Art::AudioHeraus);
    k.senden(&AnDienst::Abbrechen { strom: 1 }).await;
    assert_eq!(k.fehler().await, (1, code::ABGEBROCHEN.into()));
    assert!(
        k.rahmen_binnen(300).await.is_none(),
        "nichts mehr hinterher"
    );
}

/// Fängt alles ab, was über `log` läuft (einmal je Prozess installiert).
struct Fang;

static ZEILEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl log::Log for Fang {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        ZEILEN.lock().unwrap().push(format!("{}", r.args()));
    }
    fn flush(&self) {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inhalte_stehen_nicht_im_log() {
    static FANG: Fang = Fang;
    let _ = log::set_logger(&FANG);
    log::set_max_level(log::LevelFilter::Trace);

    let st = stand();
    *st.engine.antwort.lock().unwrap() = Some("GEHEIMER-ERKANNTER-SATZ-4711".into());
    let z = st.gekoppelt(&[Recht::Hoeren, Recht::Sprechen]);
    let mut k = st.verbinden();
    k.anmelden(&z).await;

    k.start(1, Operation::Erkennen, Souveraenitaet::NurLokal)
        .await;
    k.guthaben_erwarten(1).await;
    k.audio(1, vec![0; 3200]).await;
    k.senden(&AnDienst::Ende { strom: 1 }).await;
    loop {
        if let AnApp::Ergebnis { text, .. } = k.steuer().await {
            assert_eq!(text, "GEHEIMER-ERKANNTER-SATZ-4711");
            break;
        }
    }
    let sprechen = Operation::Sprechen {
        text: "GEHEIMER-GESPROCHENER-SATZ-4712".into(),
        stimme: None,
    };
    k.start(2, sprechen, Souveraenitaet::NurLokal).await;
    k.senden(&AnDienst::Guthaben {
        strom: 2,
        rahmen: 64,
    })
    .await;
    while let Some(r) = k.rahmen().await {
        if r.art == Art::Steuer {
            break;
        }
    }
    k.senden(&AnDienst::Ende { strom: 77 }).await;
    assert_eq!(k.fehler().await, (0, code::STROM_UNBEKANNT.into()));
    k.leitung_zu().await;

    let zeilen = ZEILEN.lock().unwrap().clone();
    assert!(
        zeilen.iter().any(|l| l.contains("erkannt strom=1")),
        "der Dienst protokolliert: {zeilen:?}"
    );
    assert!(
        zeilen
            .iter()
            .all(|l| !l.contains("GEHEIMER") && !l.contains(SCHLUESSEL) && !l.contains(&fp_hex())),
        "weder Text noch Schlüssel noch Fingerabdruck im Log: {zeilen:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verbindungsplaetze_sind_begrenzt() {
    let st = stand_mit(|k| k.max_verbindungen = 2);
    let a = st.ctx.platz().unwrap();
    let _b = st.ctx.platz().unwrap();
    assert!(st.ctx.platz().is_none());
    drop(a);
    assert!(st.ctx.platz().is_some());
}

// --- Echte Sockets (Unix) -----------------------------------------------------------------------------------------

#[cfg(unix)]
mod sockets {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::UnixStream as StdStream;
    use std::path::Path;

    use tokio::net::UnixStream;
    use wai_voice_dienst::client::{ClientFehler, Verbindung};
    use wai_voice_dienst::endpunkt::socket_pfad;

    use super::*;
    use crate::sprachdienst::dienst::Sprachdienst;
    use crate::sprachdienst::unix;

    fn modus(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    fn dienst(st: &Stand) -> Arc<Sprachdienst> {
        Sprachdienst::neu(st.ctx.clone())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn socket_0600_ordner_0700_nur_im_laufzeitordner() {
        let laufzeit = tempfile::tempdir().unwrap();
        std::fs::set_permissions(laufzeit.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let st = stand();
        let d = dienst(&st);
        let ep = d.starten_unix(laufzeit.path()).unwrap();

        let soll = socket_pfad(laufzeit.path());
        assert_eq!(Path::new(&ep), soll);
        assert!(std::fs::metadata(&soll).unwrap().file_type().is_socket());
        assert_eq!(modus(&soll), 0o600, "Socket 0600");
        assert_eq!(modus(soll.parent().unwrap()), 0o700, "Ordner 0700");
        assert_eq!(
            std::fs::metadata(&soll).unwrap().uid(),
            std::fs::metadata(laufzeit.path()).unwrap().uid()
        );
        assert!(d.status().laeuft && d.status().endpunkt.as_deref() == Some(ep.as_str()));

        d.stoppen();
        for _ in 0..200 {
            if !soll.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!soll.exists(), "der Socket verschwindet mit dem Dienst");
        assert!(!d.status().laeuft);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn ein_zu_offener_ordner_wird_nachgezogen_ein_offener_laufzeitordner_abgelehnt() {
        let laufzeit = tempfile::tempdir().unwrap();
        std::fs::set_permissions(laufzeit.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::create_dir(laufzeit.path().join("wai")).unwrap();
        std::fs::set_permissions(
            laufzeit.path().join("wai"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let st = stand();
        let d = dienst(&st);
        d.starten_unix(laufzeit.path()).unwrap();
        assert_eq!(
            modus(&laufzeit.path().join("wai")),
            0o700,
            "Ordner wird auf 0700 gesetzt"
        );
        d.stoppen();

        // Gemeinsames Verzeichnis wie /tmp: abgelehnt, es entsteht nichts darin.
        let offen = tempfile::tempdir().unwrap();
        std::fs::set_permissions(offen.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        let d = dienst(&st);
        let fehler = d.starten_unix(offen.path()).unwrap_err();
        assert!(fehler.contains("beschreibbar"), "{fehler}");
        assert!(!offen.path().join("wai").exists());
        assert!(!d.status().laeuft);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn fremde_datei_am_socketpfad_bleibt_unberuehrt_veralteter_socket_wird_ersetzt() {
        let laufzeit = tempfile::tempdir().unwrap();
        std::fs::set_permissions(laufzeit.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let pfad = socket_pfad(laufzeit.path());
        std::fs::create_dir(pfad.parent().unwrap()).unwrap();
        std::fs::write(&pfad, b"wichtig").unwrap();
        let st = stand();
        let d = dienst(&st);
        assert!(d.starten_unix(laufzeit.path()).is_err());
        assert_eq!(std::fs::read(&pfad).unwrap(), b"wichtig");
        std::fs::remove_file(&pfad).unwrap();

        // Ein Socket, auf dem niemand mehr hört (abgestürzter Vorgänger).
        drop(std::os::unix::net::UnixListener::bind(&pfad).unwrap());
        assert!(pfad.exists());
        d.starten_unix(laufzeit.path()).unwrap();
        assert!(StdStream::connect(&pfad).is_ok());
        d.stoppen();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn ein_laufender_dienst_wird_nicht_verdraengt() {
        let laufzeit = tempfile::tempdir().unwrap();
        std::fs::set_permissions(laufzeit.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let st = stand();
        let erster = dienst(&st);
        erster.starten_unix(laufzeit.path()).unwrap();
        let zweiter = dienst(&st);
        let fehler = zweiter.starten_unix(laufzeit.path()).unwrap_err();
        assert!(fehler.contains("belegt"), "{fehler}");
        assert!(zweiter.status().laeuft == false);
        assert!(
            StdStream::connect(socket_pfad(laufzeit.path())).is_ok(),
            "der erste läuft weiter"
        );
        erster.stoppen();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn gegenstelle_eines_anderen_benutzers_wird_ohne_ein_wort_geschlossen() {
        let laufzeit = tempfile::tempdir().unwrap();
        std::fs::set_permissions(laufzeit.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let st = stand();
        let g = unix::binden(laufzeit.path()).unwrap();
        let pfad = g.pfad.clone();
        let falsche_uid = g.uid.wrapping_add(1);
        let (_tx, rx) = watch::channel(false);
        tokio::spawn(unix::annehmen(g, falsche_uid, st.ctx.clone(), rx));

        let mut s = UnixStream::connect(&pfad).await.unwrap();
        let mut buf = [0u8; 64];
        let n = timeout(Duration::from_secs(5), s.read(&mut buf))
            .await
            .unwrap()
            .unwrap_or(0);
        assert_eq!(n, 0, "kein gruss für einen fremden Benutzer");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn client_des_sdk_koppelt_meldet_sich_an_erkennt_und_wird_widerrufen() {
        let laufzeit = tempfile::tempdir().unwrap();
        std::fs::set_permissions(laufzeit.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let st = stand();
        let d = dienst(&st);
        d.starten_unix(laufzeit.path()).unwrap();
        let pfad = socket_pfad(laufzeit.path());

        // Kopplung: Der Client wartet, der Nutzer bestätigt in der Oberfläche.
        let p = pfad.clone();
        let koppeln = tokio::task::spawn_blocking(move || {
            let mut v = Verbindung::neu(StdStream::connect(p).unwrap());
            v.koppeln(CLIENT, "Corporate LLM", SCHLUESSEL, "424242")
        });
        let offen = st.kopplung_abwarten().await;
        assert_eq!(offen.code, "424242");
        assert!(st.ctx.kopplungen.entscheiden(&offen.id, true, jetzt()));
        let zugang = koppeln.await.unwrap().expect("gekoppelt");

        // Ohne Recht: Start scheitert mit recht_fehlt.
        let p = pfad.clone();
        let z2 = zugang.clone();
        let ohne_recht = tokio::task::spawn_blocking(move || {
            let mut v = Verbindung::neu(StdStream::connect(p).unwrap());
            let w = v.anmelden(CLIENT, &z2, SCHLUESSEL).unwrap();
            assert!(w.rechte.is_empty());
            v.erkennen(1, BEREICH, Souveraenitaet::NurLokal, &[0; 640], |_| {})
        });
        let fehler = ohne_recht.await.unwrap().unwrap_err();
        assert_eq!(fehler.code(), code::RECHT_FEHLT);

        // Mit Recht: 40 s Audio gehen mit Guthaben durch (mehr als die 16 Startrahmen).
        st.ctx
            .zugaenge
            .recht_setzen(&zugang, Recht::Hoeren, true)
            .unwrap();
        let (p, z3) = (pfad.clone(), zugang.clone());
        let text = tokio::task::spawn_blocking(move || {
            let mut v = Verbindung::neu(StdStream::connect(p).unwrap());
            v.anmelden(CLIENT, &z3, SCHLUESSEL).unwrap();
            v.erkennen(
                1,
                BEREICH,
                Souveraenitaet::NurLokal,
                &vec![0u8; 40 * grenzen::AUDIO_MAX],
                |_| {},
            )
        })
        .await
        .unwrap()
        .expect("erkannt");
        assert_eq!(text, (40 * grenzen::AUDIO_MAX).to_string());

        // Widerruf: neue Anmeldungen scheitern.
        st.ctx.zugaenge.widerrufen(&zugang, jetzt()).unwrap();
        st.ctx.verbindungen_beenden(&zugang);
        let (p, z4) = (pfad.clone(), zugang.clone());
        let fehler = tokio::task::spawn_blocking(move || {
            let mut v = Verbindung::neu(StdStream::connect(p).unwrap());
            v.anmelden(CLIENT, &z4, SCHLUESSEL).unwrap_err()
        })
        .await
        .unwrap();
        assert!(
            matches!(&fehler, ClientFehler::Dienst { code, .. } if code == "schluessel_widerrufen"),
            "{fehler:?}"
        );
        d.stoppen();
    }
}
