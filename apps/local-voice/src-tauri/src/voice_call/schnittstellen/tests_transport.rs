use std::ffi::OsString;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::anbindung::{
    protokolltext, versende, Abbruch, Antwortrunde, Ausgang, Lauf, Namensaufloesung, Schnappschuss,
    Verbindung, Verbindungsaufbau, Ziel,
};
use super::bestaetigung::{Freigabe, Uhr};
use super::kodierung::{lies_json, Wert};
use super::transport::{
    hintergrund_anzahl, hintergrund_test_sperre, schreibe_gezaehlt, HttpVerbindung, SystemDns,
    TcpTlsAufbau,
};

enum Verhalten {
    Antwort(Vec<u8>),
    Schweigen,
    SpaeterAntwort(Duration, Vec<u8>),
    Strom { kopf: Vec<u8>, bytes: usize },
    StromDannSchweigen { kopf: Vec<u8>, bytes: usize },
    Abriss,
}

struct Server {
    port: u16,
    verbindungen: Arc<AtomicUsize>,
    anfragen: Arc<Mutex<Vec<Vec<u8>>>>,
    stopp: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn warten(stopp: &AtomicBool, dauer: Duration) {
    let ende = Instant::now() + dauer;
    while !stopp.load(Ordering::SeqCst) && Instant::now() < ende {
        thread::sleep(Duration::from_millis(5));
    }
}

fn bediene(
    mut strom: TcpStream,
    verhalten: Verhalten,
    stopp: &AtomicBool,
    anfragen: &Mutex<Vec<Vec<u8>>>,
) {
    if strom
        .set_read_timeout(Some(Duration::from_millis(20)))
        .is_err()
        || strom
            .set_write_timeout(Some(Duration::from_millis(100)))
            .is_err()
    {
        return;
    }
    let mut anfrage = Vec::new();
    let mut puffer = [0; 4096];
    loop {
        if stopp.load(Ordering::SeqCst) {
            return;
        }
        match strom.read(&mut puffer) {
            Ok(0) => return,
            Ok(n) => anfrage.extend_from_slice(&puffer[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => return,
        }
        // Ein TLS-ClientHello ist keine HTTP-Anfrage. Klartext erzwingt den TLS-Fehler.
        if anfrage.first() == Some(&0x16) {
            let _ = strom.write_all(b"muell");
            return;
        }
        if let Some(ende) = anfrage.windows(4).position(|w| w == b"\r\n\r\n") {
            let kopf = String::from_utf8_lossy(&anfrage[..ende]);
            let laenge = kopf
                .lines()
                .filter_map(|zeile| zeile.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, wert)| wert.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if anfrage.len() >= ende + 4 + laenge {
                anfragen.lock().unwrap().push(anfrage);
                break;
            }
        }
        if anfrage.len() > 2 * 1024 * 1024 {
            return;
        }
    }
    let danach_schweigen = matches!(&verhalten, Verhalten::StromDannSchweigen { .. });
    match verhalten {
        Verhalten::Antwort(bytes) => {
            let _ = strom.write_all(&bytes);
        }
        Verhalten::Schweigen => {
            while !stopp.load(Ordering::SeqCst) {
                warten(stopp, Duration::from_millis(20));
            }
        }
        Verhalten::SpaeterAntwort(dauer, bytes) => {
            warten(stopp, dauer);
            if !stopp.load(Ordering::SeqCst) {
                let _ = strom.write_all(&bytes);
            }
        }
        Verhalten::Strom { kopf, bytes } | Verhalten::StromDannSchweigen { kopf, bytes } => {
            if strom.write_all(&kopf).is_err() {
                return;
            }
            let stueck = [b'x'; 65536];
            let mut rest = bytes;
            while rest > 0 && !stopp.load(Ordering::SeqCst) {
                let n = rest.min(stueck.len());
                if strom.write_all(&stueck[..n]).is_err() {
                    break;
                }
                rest -= n;
            }
            if danach_schweigen {
                while !stopp.load(Ordering::SeqCst) {
                    warten(stopp, Duration::from_millis(20));
                }
            }
        }
        Verhalten::Abriss => {}
    }
}

impl Server {
    fn neu(skript: impl Fn(usize) -> Verhalten + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let verbindungen = Arc::new(AtomicUsize::new(0));
        let anfragen = Arc::new(Mutex::new(Vec::new()));
        let stopp = Arc::new(AtomicBool::new(false));
        let v = verbindungen.clone();
        let a = anfragen.clone();
        let halt = stopp.clone();
        let skript = Arc::new(skript);
        let thread = thread::spawn(move || {
            let mut kinder = Vec::new();
            for verbindung in listener.incoming() {
                if halt.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(strom) = verbindung else {
                    break;
                };
                let nr = v.fetch_add(1, Ordering::SeqCst);
                let verhalten = skript(nr);
                let a = a.clone();
                let halt = halt.clone();
                kinder.push(thread::spawn(move || bediene(strom, verhalten, &halt, &a)));
            }
            for kind in kinder {
                let _ = kind.join();
            }
        });
        Self {
            port,
            verbindungen,
            anfragen,
            stopp,
            thread: Some(thread),
        }
    }

    fn anzahl(&self) -> usize {
        self.anfragen.lock().unwrap().len()
    }

    fn verbindungszahl(&self) -> usize {
        self.verbindungen.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopp.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(
            &([127, 0, 0, 1], self.port).into(),
            Duration::from_millis(200),
        );
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn antwort() -> Vec<u8> {
    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec()
}

fn eintrag(port: u16, schreibend: bool, host: &str) -> Wert {
    let mut e = lies_json(
        r#"{
        "id":"probe", "name":"Probe", "beschreibung":"", "art":"http",
        "zugriff":"lesend", "abfluss":"lokal", "zeitlimit_ms":2000,
        "http":{"methode":"GET", "adresse":"", "netz":"geraet",
        "antwort_max_bytes":20480, "parameter":[], "kopfzeilen":[], "geheimnis":null}
    }"#,
    )
    .unwrap();
    let mut h = e["http"].clone();
    h.setzen("adresse", format!("http://{host}:{port}/pfad").as_str());
    if schreibend {
        e.setzen("zugriff", "schreibend");
        h.setzen("methode", "POST");
        h.setzen(
            "parameter",
            lies_json(
                r#"[{"name":"text", "beschreibung":"", "typ":"text", "ort":"koerper",
                "pflicht":true, "max_zeichen":500}]"#,
            )
            .unwrap(),
        );
    }
    e.setzen("http", h);
    pruefe_eintrag(&e);
    e
}

fn pruefe_eintrag(e: &Wert) {
    assert!(super::verzeichnis::pruefe_verzeichnis(&super::objekt([
        ("schema", "lva.schnittstellen/1".into()),
        ("eintraege", Wert::Liste(vec![e.clone()])),
    ]))
    .is_ok());
}

fn schnappschuss(port: u16, schreibend: bool, host: &str) -> Schnappschuss {
    let argumente = if schreibend {
        r#"{"text":"einmal"}"#
    } else {
        "{}"
    };
    Schnappschuss::neu(
        &eintrag(port, schreibend, host),
        &lies_json(argumente).unwrap(),
        None,
    )
    .unwrap()
}

fn lesend(port: u16) -> Schnappschuss {
    schnappschuss(port, false, "127.0.0.1")
}

fn lauf(ms: u64) -> Lauf {
    Lauf::fuer_test(Duration::from_millis(ms), Abbruch::neu())
}

fn ausfuehren(s: &Schnappschuss, frage: Option<(&Freigabe, u64)>, lauf: &Lauf) -> Ausgang {
    versende(s, frage, &mut SystemDns, &mut TcpTlsAufbau, lauf)
}

fn bestaetigt(f: &Freigabe, s: &Schnappschuss) -> u64 {
    let generation = f.frage(s.bindung());
    assert_eq!(f.aeusserung(f.aeusserungsbeginn(), true), "bestaetigt");
    generation
}

fn schreibend(port: u16, lauf: &Lauf) -> Ausgang {
    let s = schnappschuss(port, true, "127.0.0.1");
    let f = Freigabe::default();
    let generation = bestaetigt(&f, &s);
    ausfuehren(&s, Some((&f, generation)), lauf)
}

struct DnsAttrappe {
    listen: Vec<Vec<String>>,
    anzahl: usize,
}

impl DnsAttrappe {
    fn neu(adressen: &[&str]) -> Self {
        Self {
            listen: vec![
                adressen.iter().map(|ip| (*ip).into()).collect(),
                vec!["169.254.169.254".into()],
            ],
            anzahl: 0,
        }
    }
}

impl Namensaufloesung for DnsAttrappe {
    fn aufloesen(&mut self, _host: &str, _lauf: &Lauf) -> Result<Vec<String>, &'static str> {
        let liste = self.listen[self.anzahl.min(self.listen.len() - 1)].clone();
        self.anzahl += 1;
        Ok(liste)
    }
}

#[derive(Default)]
struct AufbauWrapper<'a> {
    anzahl: usize,
    aktion: Option<Box<dyn FnOnce() + 'a>>,
}

impl Verbindungsaufbau for AufbauWrapper<'_> {
    type Verbunden = HttpVerbindung;

    fn verbinden(
        &mut self,
        ziel: &Ziel,
        ip: IpAddr,
        lauf: &Lauf,
    ) -> Result<HttpVerbindung, &'static str> {
        self.anzahl += 1;
        if let Some(aktion) = self.aktion.take() {
            aktion();
        }
        TcpTlsAufbau.verbinden(ziel, ip, lauf)
    }
}

#[test]
fn teilschreibungen_werden_sofort_gezaehlt() {
    // Gegenprobe: der alte Zaehler erst nach Erfolg verliert angenommene Teilstuecke.
    struct Teilziel {
        schritte: std::vec::IntoIter<io::Result<usize>>,
    }
    impl Write for Teilziel {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let n = self.schritte.next().unwrap();
            if let Ok(n) = n {
                assert!(n <= bytes.len());
            }
            n
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    for (schritte, erwartet, anzahl) in [
        (
            vec![Ok(3), Err(ErrorKind::TimedOut.into())],
            Some(ErrorKind::TimedOut),
            3,
        ),
        (vec![Ok(2), Ok(2), Ok(2), Ok(1)], None, 7),
        (vec![Ok(2), Ok(0)], Some(ErrorKind::WriteZero), 2),
        (vec![Err(ErrorKind::Interrupted.into()), Ok(7)], None, 7),
        (
            vec![Ok(3), Err(ErrorKind::WouldBlock.into())],
            Some(ErrorKind::WouldBlock),
            3,
        ),
    ] {
        let zaehler = AtomicUsize::new(0);
        let mut ziel = Teilziel {
            schritte: schritte.into_iter(),
        };
        let ergebnis = schreibe_gezaehlt(&mut ziel, b"1234567", &zaehler);
        assert_eq!(ergebnis.err().map(|e| e.kind()), erwartet);
        assert_eq!(zaehler.load(Ordering::SeqCst), anzahl);
        assert!(ziel.schritte.next().is_none());
    }
}

#[test]
fn versandthread_verwendet_den_laufenden_zaehler() {
    // Gegenprobe: write_all mit Zaehler erst nach Erfolg verliert den Teilversand.
    fn laufend(code: &str) -> bool {
        code.contains("schreibe_gezaehlt(&mut strom") && !code.contains("write_all(&bytes)")
    }
    let code = include_str!("transport.rs");
    assert!(laufend(code));
    let alt = code.replace("schreibe_gezaehlt(&mut strom", "strom.write_all(&bytes)");
    assert!(!laufend(&alt));
}

#[test]
fn teilempfang_vor_zeitlimit_bleibt_im_ausgang() {
    let _hintergrund = hintergrund_test_sperre();
    // Gegenprobe: empfangen_bytes = 0 im Fehlerfall verliert die zehn Koerperbytes.
    let a = Server::neu(|_| Verhalten::StromDannSchweigen {
        kopf: b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n".to_vec(),
        bytes: 10,
    });
    let erg = ausfuehren(&lesend(a.port), None, &lauf(400));
    assert_eq!(erg.ergebnis, "zeitlimit");
    assert_eq!(erg.empfangen_bytes, 10);
    assert!(erg.koerper.is_empty());
}

#[test]
fn teilempfang_vor_abriss_bleibt_im_ausgang() {
    let _hintergrund = hintergrund_test_sperre();
    // Gegenprobe: Zaehlen erst bei vollstaendiger Antwort verliert alle 70000 Bytes.
    let a = Server::neu(|_| Verhalten::Strom {
        kopf: b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\n".to_vec(),
        bytes: 70000,
    });
    let erg = ausfuehren(&lesend(a.port), None, &lauf(3000));
    assert_eq!(erg.ergebnis, "netzfehler");
    assert_eq!(erg.empfangen_bytes, 70000);
    assert!(erg.koerper.is_empty());
}

#[test]
fn schreibend_bleiben_beide_zaehler_bei_zeitlimit_erhalten() {
    let _hintergrund = hintergrund_test_sperre();
    // Gegenprobe: fehlende Zaehleruebernahme nach der Antwort verliert die Bytezahlen.
    let a = Server::neu(|_| Verhalten::StromDannSchweigen {
        kopf: b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n".to_vec(),
        bytes: 10,
    });
    let erg = schreibend(a.port, &lauf(400));
    assert_eq!(erg.ergebnis, "unklar");
    assert_eq!(erg.empfangen_bytes, 10);
    assert_eq!(a.anzahl(), 1);
    assert_eq!(erg.gesendet_bytes, a.anfragen.lock().unwrap()[0].len());
    assert!(erg.koerper.is_empty());
}

#[test]
fn versandthread_wartet_schon_vor_dem_senden() {
    let _hintergrund = hintergrund_test_sperre();
    // Schuetzt den Threadstart im Aufbau und genau einen Versand je Verbindung.
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let s = lesend(a.port);
    let lauf = lauf(3000);
    let mut v = TcpTlsAufbau
        .verbinden(s.ziel(), "127.0.0.1".parse().unwrap(), &lauf)
        .unwrap();
    assert_eq!(v.gesendet_bytes(), 0);
    thread::sleep(Duration::from_millis(150));
    assert_eq!(hintergrund_anzahl(), 1);
    assert!(a.anfragen.lock().unwrap().is_empty());
    v.senden(s.anfrage()).unwrap();
    assert_eq!(v.senden(s.anfrage()), Err("netzfehler"));
    let ende = Instant::now() + Duration::from_secs(1);
    while a.anzahl() == 0 && Instant::now() < ende {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(a.anzahl(), 1);
    assert_eq!(v.antwort(&lauf, 20480).unwrap().status, 200);
    // Schuetzt die Zahl erfolgreich uebergebener Bytes statt bloss geplanter Bytes.
    assert_eq!(v.gesendet_bytes(), a.anfragen.lock().unwrap()[0].len());
}

#[test]
fn ohne_schreibversuch_bleibt_der_zaehler_null() {
    let _hintergrund = hintergrund_test_sperre();
    // Ohne Fristpruefung im wartenden Thread wuerde senden den Zaehler bereits erhoehen.
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let s = lesend(a.port);
    let frist = lauf(300);
    let mut v = TcpTlsAufbau
        .verbinden(s.ziel(), "127.0.0.1".parse().unwrap(), &frist)
        .unwrap();
    assert_eq!(v.gesendet_bytes(), 0);
    thread::sleep(Duration::from_millis(400));
    let _ = v.senden(s.anfrage());
    assert_eq!(v.gesendet_bytes(), 0);
    assert_eq!(v.senden(s.anfrage()), Err("netzfehler"));
    assert_eq!(a.anzahl(), 0);
}

#[test]
fn nach_halt_wird_nichts_mehr_eingereiht() {
    let _hintergrund = hintergrund_test_sperre();
    // Ohne Haltpruefung koennte senden nach dem Abbruch noch Ok liefern.
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let s = lesend(a.port);
    let mut v = TcpTlsAufbau
        .verbinden(s.ziel(), "127.0.0.1".parse().unwrap(), &lauf(3000))
        .unwrap();
    assert!(matches!(v.antwort(&lauf(0), 20480), Err("zeitlimit")));
    assert_eq!(v.senden(s.anfrage()), Err("netzfehler"));
    assert_eq!(v.gesendet_bytes(), 0);
    assert_eq!(a.anzahl(), 0);
}

#[test]
fn notbremse_beendet_handshake_und_leser_auf_dem_server() {
    let _hintergrund = hintergrund_test_sperre();
    // Die lange Frist bei Abbruch macht fehlendes shutdown trotz Socket-Timeout sichtbar.
    for tls in [true, false] {
        for abbrechen in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let (angenommen_tx, angenommen_rx) = std::sync::mpsc::channel();
            let (ende_tx, ende_rx) = std::sync::mpsc::channel();
            let server = thread::spawn(move || {
                let (mut strom, _) = listener.accept().unwrap();
                strom
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .unwrap();
                angenommen_tx.send(()).unwrap();
                let ende = Instant::now() + Duration::from_secs(3);
                let mut puffer = [0; 4096];
                let mut eof = false;
                while Instant::now() < ende {
                    match strom.read(&mut puffer) {
                        Ok(0) => {
                            eof = true;
                            break;
                        }
                        Ok(_) => {}
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(_) => break,
                    }
                }
                let _ = ende_tx.send(eof);
            });
            let abbruch = Abbruch::neu();
            let signal = abbruch.clone();
            let ausloeser = thread::spawn(move || {
                angenommen_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                if abbrechen {
                    thread::sleep(Duration::from_millis(300));
                    signal.ausloesen();
                }
            });
            let lauf = Lauf::fuer_test(
                Duration::from_millis(if abbrechen { 5000 } else { 300 }),
                abbruch,
            );
            let ziel = Ziel {
                host: "localhost".into(),
                port,
                tls,
            };
            let ergebnis = TcpTlsAufbau.verbinden(&ziel, "127.0.0.1".parse().unwrap(), &lauf);
            let erwartet = if abbrechen {
                "abgebrochen"
            } else {
                "zeitlimit"
            };
            // Bei HTTP bleibt v bis nach der EOF-Pruefung leben: Drop darf nicht helfen.
            let mut verbindung = None;
            if tls {
                assert!(matches!(ergebnis, Err(e) if e == erwartet));
            } else {
                let mut v = ergebnis.unwrap();
                v.senden(lesend(port).anfrage()).unwrap();
                assert!(matches!(v.antwort(&lauf, 20480), Err(e) if e == erwartet));
                verbindung = Some(v);
            }
            assert_eq!(ende_rx.recv_timeout(Duration::from_secs(2)), Ok(true));
            drop(verbindung);
            ausloeser.join().unwrap();
            server.join().unwrap();
        }
    }
}

#[test]
fn umleitung_wird_abgewiesen() {
    let _hintergrund = hintergrund_test_sperre();
    // Ohne Umleitungssperre wuerde B erreicht oder ein erfolgreicher Status erscheinen.
    let b = Server::neu(|_| Verhalten::Antwort(antwort()));
    let port = b.port;
    let a = Server::neu(move |_| {
        Verhalten::Antwort(
            format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/ziel\r\n\
                 Content-Length: 0\r\n\r\n"
            )
            .into_bytes(),
        )
    });
    let erg = ausfuehren(&lesend(a.port), None, &lauf(2000));
    assert_eq!(erg.ergebnis, "umleitung");
    assert_eq!(erg.status, Some(302));
    assert!(erg.koerper.is_empty());
    assert_eq!(a.anzahl(), 1);
    thread::sleep(Duration::from_millis(200));
    assert_eq!(b.verbindungszahl(), 0);
}

#[test]
fn gesendet_bytes_im_ausgang_sind_die_geschriebenen_bytes() {
    let _hintergrund = hintergrund_test_sperre();
    // Der Versand laeuft im Hintergrund: ohne erneutes Lesen nach der Antwort bliebe der Wert 0.
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let erg = ausfuehren(&lesend(a.port), None, &lauf(2000));
    assert_eq!(erg.ergebnis, "ok");
    let angekommen = a.anfragen.lock().unwrap()[0].len();
    assert!(erg.gesendet_bytes > 0);
    assert_eq!(erg.gesendet_bytes, angekommen);
}

#[test]
fn alt_svc_und_upgrade_aendern_das_ziel_nicht() {
    let _hintergrund = hintergrund_test_sperre();
    // README-Probe alternatives_ziel: Zielwechsel und automatische Aushandlung fallen auf.
    for wiederverwenden in [false, true] {
        let c = Server::neu(|_| Verhalten::Antwort(antwort()));
        let port = c.port;
        let a = Server::neu(move |_| {
            Verhalten::Antwort(
                format!(
                    "HTTP/1.1 200 OK\r\nAlt-Svc: h3=\":{port}\"; ma=86400\r\n\
                     Upgrade: h2c\r\nConnection: Upgrade\r\nContent-Length: 2\r\n\r\n{{}}"
                )
                .into_bytes(),
            )
        });
        let mut aufbau = TcpTlsAufbau;
        for _ in 0..2 {
            let s = lesend(a.port);
            let erg = if wiederverwenden {
                versende(&s, None, &mut SystemDns, &mut aufbau, &lauf(2000))
            } else {
                ausfuehren(&s, None, &lauf(2000))
            };
            assert_eq!(erg.ergebnis, "ok");
        }
        let anfragen = a.anfragen.lock().unwrap();
        assert_eq!(anfragen.len(), 2);
        for anfrage in anfragen.iter() {
            let text = String::from_utf8_lossy(anfrage).to_ascii_lowercase();
            assert!(text.contains(&format!("\r\nhost: 127.0.0.1:{}\r\n", a.port)));
            assert!(!text.contains("upgrade"));
            assert!(!text.contains("accept-encoding"));
        }
        thread::sleep(Duration::from_millis(200));
        assert_eq!(c.verbindungszahl(), 0);
    }
}

static ENV_MUTEX: Mutex<()> = Mutex::new(());
const PROXY_VARIABLEN: [&str; 8] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

struct EnvSchutz(Vec<(&'static str, Option<OsString>)>);

// Unter Windows sind Gross- und Kleinschreibung dieselbe Umgebungsvariable.
#[allow(unused_unsafe)]
fn setze_env(name: &str, wert: Option<&std::ffi::OsStr>) {
    unsafe {
        if let Some(wert) = wert {
            std::env::set_var(name, wert);
        } else {
            std::env::remove_var(name);
        }
    }
}

impl EnvSchutz {
    fn neu(port: u16) -> Self {
        let schutz = Self(
            PROXY_VARIABLEN
                .iter()
                .map(|name| (*name, std::env::var_os(name)))
                .collect(),
        );
        let proxy = OsString::from(format!("http://127.0.0.1:{port}"));
        for name in PROXY_VARIABLEN {
            setze_env(
                name,
                (!name.eq_ignore_ascii_case("no_proxy")).then_some(proxy.as_os_str()),
            );
        }
        schutz
    }
}

impl Drop for EnvSchutz {
    fn drop(&mut self) {
        for (name, wert) in &self.0 {
            setze_env(name, wert.as_deref());
        }
    }
}

#[test]
fn proxy_variablen_werden_ignoriert() {
    let _hintergrund = hintergrund_test_sperre();
    // Eine Auswertung von Proxy-Variablen erreicht P statt des festgelegten Ziels.
    let _sperre = ENV_MUTEX.lock().unwrap();
    let p = Server::neu(|_| Verhalten::Antwort(antwort()));
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let _schutz = EnvSchutz::neu(p.port);
    assert_eq!(
        ausfuehren(&lesend(a.port), None, &lauf(2000)).ergebnis,
        "ok"
    );
    assert_eq!(a.anzahl(), 1);
    thread::sleep(Duration::from_millis(200));
    assert_eq!(p.verbindungszahl(), 0);
}

#[test]
fn zeitlimit_lesend_und_schreibend() {
    let _hintergrund = hintergrund_test_sperre();
    // Ohne Frist endet Lesen nicht; ohne Schreib-Zuordnung erscheint zeitlimit statt unklar.
    let a = Server::neu(|_| Verhalten::Schweigen);
    let erg = ausfuehren(&lesend(a.port), None, &lauf(300));
    assert_eq!(erg.ergebnis, "zeitlimit");
    assert!((250..=3000).contains(&erg.dauer_ms));
    assert!(erg.koerper.is_empty());
    assert_eq!(erg.status, None);
    assert_eq!(a.anzahl(), 1);
    a.anfragen.lock().unwrap().clear();
    let erg = schreibend(a.port, &lauf(300));
    assert_eq!(erg.ergebnis, "unklar");
    assert_eq!(a.anzahl(), 1);
    assert!(a.anfragen.lock().unwrap()[0].starts_with(b"POST "));
}

#[test]
fn uebergroesse() {
    let _hintergrund = hintergrund_test_sperre();
    // Ohne Grenze werden 1048577 Bytes akzeptiert; die exakte Grenze muss nutzbar bleiben.
    for (mit_laenge, bytes, erwartet) in [
        (true, 1048577, "zu_gross"),
        (false, 1048577, "zu_gross"),
        (false, 1048576, "ok"),
    ] {
        let a = Server::neu(move |_| Verhalten::Strom {
            kopf: if mit_laenge {
                format!("HTTP/1.1 200 OK\r\nContent-Length: {bytes}\r\n\r\n").into_bytes()
            } else {
                b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec()
            },
            bytes,
        });
        let erg = ausfuehren(&lesend(a.port), None, &lauf(3000));
        assert_eq!(erg.ergebnis, erwartet);
        if erwartet == "ok" {
            assert_eq!(erg.empfangen_bytes, bytes);
        } else if mit_laenge {
            assert_eq!(erg.empfangen_bytes, 0);
        } else {
            // Ohne Laenge zaehlt auch die gelesene Probe jenseits der Grenze.
            assert!(erg.empfangen_bytes > 0 && erg.empfangen_bytes <= 1048577 + 8192);
        }
    }
    let a = Server::neu(|_| {
        let mut bytes = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n100000\r\n".to_vec();
        bytes.extend(vec![b'x'; 1048576]);
        bytes.extend_from_slice(b"\r\n1\r\nx\r\n0\r\n\r\n");
        Verhalten::Antwort(bytes)
    });
    let erg = ausfuehren(&lesend(a.port), None, &lauf(3000));
    assert_eq!(erg.ergebnis, "zu_gross");
    // Der erste Chunk wurde vollstaendig gelesen, bevor der zweite die Grenze verletzt.
    assert_eq!(erg.empfangen_bytes, 1048576);
    let a = Server::neu(|_| Verhalten::Strom {
        kopf: b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n".to_vec(),
        bytes: 1048577,
    });
    assert_eq!(schreibend(a.port, &lauf(3000)).ergebnis, "zu_gross");
}

#[test]
fn abbruch() {
    let _hintergrund = hintergrund_test_sperre();
    // Lesen bleibt abbrechbar; abgeschicktes Schreiben wartet, vorab gesperrtes sendet nichts.
    let a = Server::neu(|_| Verhalten::Schweigen);
    let abbruch = Abbruch::neu();
    let signal = abbruch.clone();
    let ausloeser = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        signal.ausloesen();
    });
    let erg = ausfuehren(
        &lesend(a.port),
        None,
        &Lauf::fuer_test(Duration::from_secs(5), abbruch),
    );
    ausloeser.join().unwrap();
    assert_eq!(erg.ergebnis, "abgebrochen");
    assert!(erg.dauer_ms < 2000);
    assert_eq!(a.anzahl(), 1);
    let a = Server::neu(|_| Verhalten::SpaeterAntwort(Duration::from_millis(500), antwort()));
    let abbruch = Abbruch::neu();
    let signal = abbruch.clone();
    let gelesen = a.anfragen.clone();
    let ausloeser = thread::spawn(move || {
        let ende = Instant::now() + Duration::from_secs(2);
        while gelesen.lock().unwrap().is_empty() && Instant::now() < ende {
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(100));
        signal.ausloesen();
    });
    let erg = schreibend(
        a.port,
        &Lauf::fuer_test(Duration::from_secs(5), abbruch.clone()),
    );
    let waehrend_antwort_ausgeloest = abbruch.ausgeloest();
    ausloeser.join().unwrap();
    assert!(waehrend_antwort_ausgeloest);
    assert_eq!(erg.ergebnis, "ok");
    assert_eq!(a.anzahl(), 1);
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let abbruch = Abbruch::neu();
    abbruch.ausloesen();
    assert_eq!(
        schreibend(a.port, &Lauf::fuer_test(Duration::from_secs(5), abbruch)).ergebnis,
        "abgebrochen"
    );
    assert_eq!(a.verbindungszahl(), 0);
}

#[test]
fn abriss_und_kaputte_antwort() {
    let _hintergrund = hintergrund_test_sperre();
    // Transport- und Kodierungsfehler duerfen Erfolg und sicheren Schreibfehler nicht vortaeuschen.
    for fall in 0..3 {
        let a = Server::neu(move |_| match fall {
            0 => Verhalten::Abriss,
            1 => Verhalten::Antwort(b"muell".to_vec()),
            _ => Verhalten::Antwort(
                b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 2\r\n\r\n{}"
                    .to_vec(),
            ),
        });
        assert_eq!(
            ausfuehren(&lesend(a.port), None, &lauf(2000)).ergebnis,
            if fall == 2 {
                "antwort_ungueltig"
            } else {
                "netzfehler"
            }
        );
        assert_eq!(schreibend(a.port, &lauf(2000)).ergebnis, "unklar");
        assert_eq!(a.anzahl(), 2);
    }
}

#[test]
fn nur_die_geprueften_ip_und_der_port_des_eintrags() {
    let _hintergrund = hintergrund_test_sperre();
    // README-Probe bindung_an_geprueften_ip: zweite Aufloesung und Teilmengenpruefung fallen auf.
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let s = schnappschuss(a.port, false, "localhost");
    for _ in 0..2 {
        let mut dns = DnsAttrappe::neu(&["127.0.0.1"]);
        assert_eq!(
            versende(&s, None, &mut dns, &mut TcpTlsAufbau, &lauf(2000)).ergebnis,
            "ok"
        );
        assert_eq!(dns.anzahl, 1);
    }
    assert_eq!(a.anzahl(), 2);
    for bytes in a.anfragen.lock().unwrap().iter() {
        assert!(String::from_utf8_lossy(bytes)
            .to_ascii_lowercase()
            .contains(&format!("\r\nhost: localhost:{}\r\n", a.port)));
    }
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let s = schnappschuss(a.port, false, "localhost");
    for adressen in [["127.0.0.1", "8.8.8.8"], ["8.8.8.8", "127.0.0.1"]] {
        let mut dns = DnsAttrappe::neu(&adressen);
        let mut aufbau = AufbauWrapper::default();
        assert_eq!(
            versende(&s, None, &mut dns, &mut aufbau, &lauf(2000)).ergebnis,
            "adresse_abgewiesen"
        );
        assert_eq!(dns.anzahl, 1);
        assert_eq!(aufbau.anzahl, 0);
    }
    assert_eq!(a.verbindungszahl(), 0);
    let mut e = eintrag(a.port, false, "localhost");
    let mut h = e["http"].clone();
    h.setzen("adresse", "https://api.example.org/x");
    h.setzen("netz", "oeffentlich");
    e.setzen("abfluss", "netz");
    e.setzen("http", h);
    pruefe_eintrag(&e);
    let s = Schnappschuss::neu(&e, &lies_json("{}").unwrap(), None).unwrap();
    for (adressen, erwartet) in [
        (vec!["8.8.8.8", "127.0.0.1"], "adresse_abgewiesen"),
        (vec![], "netzfehler"),
    ] {
        let mut dns = DnsAttrappe::neu(&adressen);
        let mut aufbau = AufbauWrapper::default();
        assert_eq!(
            versende(&s, None, &mut dns, &mut aufbau, &lauf(2000)).ergebnis,
            erwartet
        );
        assert_eq!(dns.anzahl, 1);
        assert_eq!(aufbau.anzahl, 0);
    }
}

#[test]
fn system_dns() {
    let _hintergrund = hintergrund_test_sperre();
    // Der echte Resolver muss Loopback liefern und Abbruch, Frist und Fehler unterscheiden.
    let ips = SystemDns.aufloesen("localhost", &lauf(5000)).unwrap();
    assert!(!ips.is_empty());
    assert!(ips
        .iter()
        .all(|ip| ip.parse::<IpAddr>().unwrap().is_loopback()));
    assert!(ips.iter().any(|ip| ip == "127.0.0.1" || ip == "::1"));
    let abbruch = Abbruch::neu();
    abbruch.ausloesen();
    assert_eq!(
        SystemDns.aufloesen(
            "localhost",
            &Lauf::fuer_test(Duration::from_secs(5), abbruch)
        ),
        Err("abgebrochen")
    );
    assert_eq!(SystemDns.aufloesen("localhost", &lauf(0)), Err("zeitlimit"));
    assert_eq!(
        SystemDns.aufloesen("pruefe-nie-vorhanden.invalid", &lauf(5000)),
        Err("netzfehler")
    );
}

#[test]
fn tls_pfad_ohne_gueltiges_zertifikat() {
    let _hintergrund = hintergrund_test_sperre();
    // Ohne TLS-Handshake wuerde verbinden gegen Klartext bereits Ok liefern.
    let a = Server::neu(|_| Verhalten::Abriss);
    let erg = TcpTlsAufbau.verbinden(
        &Ziel {
            host: "127.0.0.1".into(),
            port: a.port,
            tls: true,
        },
        "127.0.0.1".parse().unwrap(),
        &lauf(2000),
    );
    assert!(matches!(erg, Err("netzfehler" | "zeitlimit")));
    assert_eq!(a.verbindungszahl(), 1);
    assert_eq!(a.anzahl(), 0);
}

struct DnsWrapper<'a> {
    dns: DnsAttrappe,
    aktion: Option<Box<dyn FnOnce() + 'a>>,
}

impl Namensaufloesung for DnsWrapper<'_> {
    fn aufloesen(&mut self, host: &str, lauf: &Lauf) -> Result<Vec<String>, &'static str> {
        if let Some(aktion) = self.aktion.take() {
            aktion();
        }
        self.dns.aufloesen(host, lauf)
    }
}

#[derive(Default)]
struct TestUhr(AtomicI64);

impl Uhr for TestUhr {
    fn jetzt_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[test]
fn probe_widerruf_nach_bestaetigung() {
    let _hintergrund = hintergrund_test_sperre();
    // Ohne erneute Freigabepruefung nach DNS/TCP wuerde das bestaetigte POST gesendet.
    for bei_dns in [true, false] {
        let a = Server::neu(|_| Verhalten::Antwort(antwort()));
        let s = schnappschuss(a.port, true, "localhost");
        let f = Freigabe::default();
        let generation = bestaetigt(&f, &s);
        let widerruf = || {
            assert_eq!(f.aeusserung(f.aeusserungsbeginn(), false), "abgelehnt");
        };
        let mut dns = DnsWrapper {
            dns: DnsAttrappe::neu(&["127.0.0.1"]),
            aktion: None,
        };
        let mut aufbau = AufbauWrapper::default();
        if bei_dns {
            dns.aktion = Some(Box::new(widerruf));
        } else {
            aufbau.aktion = Some(Box::new(widerruf));
        }
        assert_eq!(
            versende(
                &s,
                Some((&f, generation)),
                &mut dns,
                &mut aufbau,
                &lauf(2000)
            )
            .ergebnis,
            "bestaetigung_abgelehnt"
        );
        assert_eq!(dns.dns.anzahl, 1);
        assert_eq!(aufbau.anzahl, 1);
        assert_eq!(a.anzahl(), 0);
    }
}

#[test]
fn probe_aeusserung_vor_der_frage() {
    let _hintergrund = hintergrund_test_sperre();
    // Eine vor der Frage begonnene Aeusserung darf keinen echten Versand freigeben.
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let s = schnappschuss(a.port, true, "127.0.0.1");
    let uhr = Arc::new(TestUhr::default());
    let f = Freigabe::mit_uhr(uhr.clone());
    let beginn = f.aeusserungsbeginn();
    uhr.0.store(1, Ordering::SeqCst);
    let generation = f.frage(s.bindung());
    uhr.0.store(2, Ordering::SeqCst);
    assert_eq!(f.aeusserung(beginn, true), "keine_frage");
    assert_eq!(
        ausfuehren(&s, Some((&f, generation)), &lauf(2000)).ergebnis,
        "bestaetigung_abgelaufen"
    );
    assert_eq!(a.anzahl(), 0);
    assert_eq!(a.verbindungszahl(), 0);
}

#[test]
fn probe_aeusserung_zu_frueherer_frage() {
    let _hintergrund = hintergrund_test_sperre();
    // Die Antwort auf eine ersetzte Frage darf die neue Frage weder bestaetigen noch ablehnen.
    for ja in [true, false] {
        let a = Server::neu(|_| Verhalten::Antwort(antwort()));
        let s = schnappschuss(a.port, true, "127.0.0.1");
        let uhr = Arc::new(TestUhr::default());
        let f = Freigabe::mit_uhr(uhr.clone());
        f.frage("fruehere-bindung");
        let beginn = f.aeusserungsbeginn();
        uhr.0.store(1, Ordering::SeqCst);
        let generation = f.frage(s.bindung());
        uhr.0.store(2, Ordering::SeqCst);
        assert_eq!(f.aeusserung(beginn, ja), "keine_frage");
        assert_eq!(
            ausfuehren(&s, Some((&f, generation)), &lauf(2000)).ergebnis,
            "bestaetigung_abgelaufen"
        );
        assert_eq!(a.anzahl(), 0);
        assert_eq!(a.verbindungszahl(), 0);
        uhr.0.store(3, Ordering::SeqCst);
        assert_eq!(f.aeusserung(f.aeusserungsbeginn(), true), "bestaetigt");
        assert_eq!(
            ausfuehren(&s, Some((&f, generation)), &lauf(2000)).ergebnis,
            "ok"
        );
        assert_eq!(a.anzahl(), 1);
    }
}

#[test]
fn probe_unteilbares_einloesen() {
    let _hintergrund = hintergrund_test_sperre();
    // Ohne Fristpruefung beim Einloesen wird nach dem echten Aufbau trotzdem gesendet.
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let s = schnappschuss(a.port, true, "127.0.0.1");
    let uhr = Arc::new(TestUhr::default());
    let f = Freigabe::mit_uhr(uhr.clone());
    let generation = bestaetigt(&f, &s);
    let mut aufbau = AufbauWrapper {
        anzahl: 0,
        aktion: Some(Box::new(|| uhr.0.store(30_000, Ordering::SeqCst))),
    };
    assert_eq!(
        versende(
            &s,
            Some((&f, generation)),
            &mut SystemDns,
            &mut aufbau,
            &lauf(2000)
        )
        .ergebnis,
        "bestaetigung_abgelaufen"
    );
    assert_eq!(aufbau.anzahl, 1);
    assert_eq!(a.anzahl(), 0);
}

#[test]
fn aufruflimit_mit_echten_adaptern() {
    let _hintergrund = hintergrund_test_sperre();
    // Ohne Aufrufgrenze wuerde der sechste Lauf erlaubt und eine sechste Anfrage moeglich.
    let a = Server::neu(|_| Verhalten::Antwort(antwort()));
    let s = lesend(a.port);
    let abbruch = Abbruch::neu();
    let mut runde = Antwortrunde::neu();
    for _ in 0..5 {
        let lauf = runde.naechster_aufruf(&s, &abbruch).unwrap();
        assert_eq!(ausfuehren(&s, None, &lauf).ergebnis, "ok");
    }
    assert!(matches!(
        runde.naechster_aufruf(&s, &abbruch),
        Err("abgelehnt")
    ));
    assert_eq!(a.anzahl(), 5);
}

#[test]
fn protokoll_ohne_inhalte() {
    let _hintergrund = hintergrund_test_sperre();
    // Jeder protokollierte Anfrage-/Antwortinhalt oder jede Zielbindung bricht die Gegenprobe.
    let a = Server::neu(|_| {
        let koerper = r#"{"antwort":"INHALT-98765"}"#;
        Verhalten::Antwort(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{koerper}",
                koerper.len()
            )
            .into_bytes(),
        )
    });
    let mut e = eintrag(a.port, false, "127.0.0.1");
    let mut h = e["http"].clone();
    h.setzen(
        "adresse",
        format!("http://127.0.0.1:{}/{{kennung}}", a.port).as_str(),
    );
    h.setzen(
        "parameter",
        lies_json(
            r#"[{"name":"kennung", "beschreibung":"", "typ":"text", "ort":"pfad",
            "pflicht":true, "max_zeichen":500}]"#,
        )
        .unwrap(),
    );
    h.setzen(
        "geheimnis",
        lies_json(
            r#"{"kopf":"authorization", "praefix":"Bearer ", "schluessel":"probe-schluessel"}"#,
        )
        .unwrap(),
    );
    e.setzen("http", h);
    pruefe_eintrag(&e);
    let s = Schnappschuss::neu(
        &e,
        &lies_json(r#"{"kennung":"abc123"}"#).unwrap(),
        Some(&Wert::from("GEHEIM-12345")),
    )
    .unwrap();
    let erg = ausfuehren(&s, None, &lauf(2000));
    assert_eq!(erg.ergebnis, "ok");
    assert!(
        erg.koerper.as_slice() == br#"{"antwort":"INHALT-98765"}"#,
        "Antwortkoerper stimmt nicht mit der Testantwort ueberein"
    );
    let anfragen = a.anfragen.lock().unwrap();
    assert_eq!(anfragen.len(), 1);
    let anfrage = String::from_utf8_lossy(&anfragen[0]);
    assert!(
        anfrage.starts_with("GET /abc123 HTTP/1.1\r\n"),
        "Pfadparameter wurde nicht gesendet"
    );
    assert!(
        anfrage.contains("Bearer GEHEIM-12345"),
        "Testgeheimnis wurde nicht gesendet"
    );
    let zeile = protokolltext(&s, &erg, "probe-assistent", "2026-11-21T12:00:00Z", false);
    for feld in [
        "zeit",
        "assistent",
        "schnittstelle",
        "art",
        "zugriff",
        "abfluss",
        "ergebnis",
        "dauer_ms",
        "status",
        "gesendet_bytes",
        "empfangen_bytes",
        "server",
        "bestaetigt",
    ] {
        assert!(zeile.contains(&format!("\"{feld}\":")));
    }
    for inhalt in [
        "abc123",
        "/pfad",
        "GEHEIM-12345",
        "INHALT-98765",
        "Bearer",
        s.bindung(),
        &a.port.to_string(),
    ] {
        assert!(!zeile.contains(inhalt));
    }
}
