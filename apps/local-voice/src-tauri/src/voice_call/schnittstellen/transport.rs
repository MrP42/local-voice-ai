//! Direkte DNS-, TCP/TLS- und HTTP/1.1-Adapter ohne Proxy oder Verbindungszustand
//! zwischen Aufrufen. TLS prueft Zertifikat und urspruenglichen Hostnamen.
use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, Sender},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use super::anbindung::{
    Antwort, Lauf, Namensaufloesung, Verbindung, Verbindungsaufbau, Ziel, MAX_ANTWORT_BYTES,
};
use super::http1::{baue_anfragebytes, lies_antwort_gezaehlt, Rohantwort};
use super::kodierung::Wert;

/// Zaehlt jedes vom Strom angenommene Teilstueck unmittelbar.
pub(crate) fn schreibe_gezaehlt(
    ziel: &mut impl Write,
    mut bytes: &[u8],
    zaehler: &AtomicUsize,
) -> io::Result<()> {
    while !bytes.is_empty() {
        match ziel.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                zaehler.fetch_add(n, Ordering::SeqCst);
                bytes = &bytes[n..];
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

static HINTERGRUND_ANZAHL: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(super) static HINTERGRUND_TEST_MUTEX: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(super) fn hintergrund_anzahl() -> usize {
    HINTERGRUND_ANZAHL.load(Ordering::SeqCst)
}

#[cfg(test)]
pub(super) fn hintergrund_test_sperre() -> std::sync::MutexGuard<'static, ()> {
    // Ein fehlgeschlagener Test darf die uebrigen nicht ueber eine vergiftete Sperre mitreissen.
    let sperre = HINTERGRUND_TEST_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let ende = Instant::now() + Duration::from_secs(2);
    while hintergrund_anzahl() != 0 && Instant::now() < ende {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(hintergrund_anzahl(), 0);
    sperre
}

struct Hintergrundwache;

impl Hintergrundwache {
    fn neu() -> Result<Self, &'static str> {
        HINTERGRUND_ANZAHL
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |anzahl| {
                (anzahl < 16).then_some(anzahl + 1)
            })
            .map(|_| Self)
            .map_err(|_| "netzfehler")
    }
}

impl Drop for Hintergrundwache {
    fn drop(&mut self) {
        HINTERGRUND_ANZAHL.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Notbremse {
    halt: AtomicBool,
    socket: Mutex<Option<TcpStream>>,
}

impl Notbremse {
    fn stoppen(&self) {
        self.halt.store(true, Ordering::SeqCst);
        if let Some(socket) = self.socket.lock().unwrap().as_ref() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}

/// Wartet abbrechbar auf blockierende Systemaufrufe. Bei Abbruch oder Fristende
/// bleibt der Thread bis zum Ende seines Systemaufrufs zurueck; sein Ergebnis
/// wird verworfen. Insbesondere laesst sich getaddrinfo nicht unterbrechen.
fn im_hintergrund<T: Send + 'static>(
    lauf: &Lauf,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, &'static str> {
    lauf.pruefen()?;
    let wache = Hintergrundwache::neu()?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("lva-schnittstelle".into())
        .spawn(move || {
            let _wache = wache;
            let _ = tx.send(f());
        })
        .map_err(|_| "netzfehler")?;
    loop {
        lauf.pruefen()?;
        let rest = lauf.rest().ok_or("zeitlimit")?;
        match rx.recv_timeout(rest.min(Duration::from_millis(20))) {
            Ok(v) => return Ok(v),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("netzfehler"),
        }
    }
}

fn restzeit(ende: Instant) -> Result<Duration, &'static str> {
    ende.checked_duration_since(Instant::now())
        .filter(|rest| !rest.is_zero())
        .ok_or("zeitlimit")
}

fn netzfehler(ende: Instant) -> &'static str {
    if restzeit(ende).is_err() {
        "zeitlimit"
    } else {
        "netzfehler"
    }
}

/// Liefert alle Adressen aus genau einem lokalen Aufloesungsvorgang.
#[derive(Default, Clone, Copy)]
pub struct SystemDns;

impl Namensaufloesung for SystemDns {
    fn aufloesen(&mut self, host: &str, lauf: &Lauf) -> Result<Vec<String>, &'static str> {
        let host = host.to_owned();
        im_hintergrund(lauf, move || {
            let adressen = (host.as_str(), 0u16)
                .to_socket_addrs()
                .map_err(|_| "netzfehler")?;
            let mut ips = Vec::new();
            for adresse in adressen {
                let ip = adresse.ip().to_string();
                if !ips.contains(&ip) {
                    ips.push(ip);
                }
            }
            Ok(ips)
        })?
    }
}

enum Strom {
    Klar(TcpStream),
    Tls(Box<native_tls::TlsStream<TcpStream>>),
}

impl Strom {
    fn tcp(&self) -> &TcpStream {
        match self {
            Self::Klar(strom) => strom,
            Self::Tls(strom) => strom.get_ref(),
        }
    }
}

impl Read for Strom {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Klar(strom) => strom.read(bytes),
            Self::Tls(strom) => strom.read(bytes),
        }
    }
}

impl Write for Strom {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Klar(strom) => strom.write(bytes),
            Self::Tls(strom) => strom.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Klar(strom) => strom.flush(),
            Self::Tls(strom) => strom.flush(),
        }
    }
}

/// Verbindet ausschliesslich zur geprueften IP, TLS verwendet den Zielhost.
#[derive(Default, Clone, Copy)]
pub struct TcpTlsAufbau;

impl Verbindungsaufbau for TcpTlsAufbau {
    type Verbunden = HttpVerbindung;

    fn verbinden(
        &mut self,
        ziel: &Ziel,
        ip: IpAddr,
        lauf: &Lauf,
    ) -> Result<Self::Verbunden, &'static str> {
        let ziel = ziel.clone();
        let ende = lauf.ende();
        let bremse = Arc::new(Notbremse::default());
        let hintergrund = Arc::clone(&bremse);
        let ergebnis = im_hintergrund(lauf, move || {
            let rest = restzeit(ende)?;
            let stream = TcpStream::connect_timeout(&SocketAddr::new(ip, ziel.port), rest)
                .map_err(|_| netzfehler(ende))?;
            *hintergrund.socket.lock().unwrap() =
                Some(stream.try_clone().map_err(|_| "netzfehler")?);
            // Der Vordergrund kann schon vor dem Eintragen des Klons aufgegeben haben.
            if hintergrund.halt.load(Ordering::SeqCst) {
                hintergrund.stoppen();
                return Err("abgebrochen");
            }
            stream.set_nodelay(true).map_err(|_| netzfehler(ende))?;
            let rest = restzeit(ende)?.max(Duration::from_millis(1));
            stream
                .set_read_timeout(Some(rest))
                .map_err(|_| netzfehler(ende))?;
            stream
                .set_write_timeout(Some(rest))
                .map_err(|_| netzfehler(ende))?;
            let strom = if ziel.tls {
                let connector = native_tls::TlsConnector::builder()
                    .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
                    .build()
                    .map_err(|_| netzfehler(ende))?;
                let tls = connector
                    .connect(&ziel.host, stream)
                    .map_err(|_| netzfehler(ende))?;
                Strom::Tls(Box::new(tls))
            } else {
                Strom::Klar(stream)
            };
            restzeit(ende)?;
            HttpVerbindung::neu(strom, ziel, ende, hintergrund)
        })
        .and_then(|ergebnis| ergebnis);
        if ergebnis.is_err() {
            bremse.stoppen();
        }
        ergebnis
    }
}

/// Eine Anfrage je Verbindung; Schreiben und Lesen laufen im Hintergrund.
pub struct HttpVerbindung {
    anfrage: Option<Sender<Vec<u8>>>,
    ziel: Ziel,
    bremse: Arc<Notbremse>,
    gesendet: Arc<AtomicUsize>,
    empfangen: Arc<AtomicUsize>,
    ergebnis: Receiver<Result<Rohantwort, &'static str>>,
}

impl HttpVerbindung {
    fn neu(
        mut strom: Strom,
        ziel: Ziel,
        ende: Instant,
        bremse: Arc<Notbremse>,
    ) -> Result<Self, &'static str> {
        let wache = Hintergrundwache::neu()?;
        let gesendet = Arc::new(AtomicUsize::new(0));
        let zaehler = Arc::clone(&gesendet);
        let empfangen = Arc::new(AtomicUsize::new(0));
        let empfangen_zaehler = Arc::clone(&empfangen);
        let hintergrund = Arc::clone(&bremse);
        let (anfrage, bytes_rx) = mpsc::channel::<Vec<u8>>();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("lva-schnittstelle".into())
            .spawn(move || {
                let _wache = wache;
                let ergebnis = (|| {
                    let mut warten = || {
                        if hintergrund.halt.load(Ordering::SeqCst) {
                            Err("abgebrochen")
                        } else {
                            restzeit(ende).map(|_| ())
                        }
                    };
                    let bytes = loop {
                        warten()?;
                        let rest = restzeit(ende)?;
                        match bytes_rx.recv_timeout(rest.min(Duration::from_millis(20))) {
                            Ok(bytes) => break bytes,
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => return Err("netzfehler"),
                        }
                    };
                    warten()?;
                    let rest = restzeit(ende)?.max(Duration::from_millis(1));
                    strom
                        .tcp()
                        .set_write_timeout(Some(rest))
                        .map_err(|_| "netzfehler")?;
                    schreibe_gezaehlt(&mut strom, &bytes, &zaehler).map_err(|_| "netzfehler")?;
                    let rest = restzeit(ende)?;
                    strom
                        .tcp()
                        .set_read_timeout(Some(rest.min(Duration::from_millis(100))))
                        .map_err(|_| "netzfehler")?;
                    warten()?;
                    lies_antwort_gezaehlt(
                        &mut strom,
                        MAX_ANTWORT_BYTES,
                        &mut warten,
                        &empfangen_zaehler,
                    )
                })();
                if ergebnis.is_err() {
                    hintergrund.stoppen();
                }
                let _ = tx.send(ergebnis);
            })
            .map_err(|_| "netzfehler")?;
        Ok(Self {
            anfrage: Some(anfrage),
            ziel,
            bremse,
            gesendet,
            empfangen,
            ergebnis: rx,
        })
    }
}

impl Drop for HttpVerbindung {
    fn drop(&mut self) {
        self.bremse.stoppen();
    }
}

impl Verbindung for HttpVerbindung {
    fn senden(&mut self, anfrage: &Wert) -> Result<(), &'static str> {
        if self.bremse.halt.load(Ordering::SeqCst) {
            return Err("netzfehler");
        }
        let sender = self.anfrage.take().ok_or("netzfehler")?;
        let bytes = baue_anfragebytes(&self.ziel, anfrage).map_err(|_| "netzfehler")?;
        // Bis zum ersten Schreiben bleibt nur die Aufwachzeit des laufenden Threads.
        sender.send(bytes).map_err(|_| "netzfehler")
    }

    fn antwort(&mut self, lauf: &Lauf, max_bytes: usize) -> Result<Antwort, &'static str> {
        let rx = &self.ergebnis;
        loop {
            if let Err(e) = lauf.pruefen() {
                self.bremse.stoppen();
                return Err(e);
            }
            let Some(rest) = lauf.rest() else {
                self.bremse.stoppen();
                return Err("zeitlimit");
            };
            match rx.recv_timeout(rest.min(Duration::from_millis(20))) {
                Ok(Ok(r)) => {
                    if r.koerper.len() > max_bytes {
                        return Err("zu_gross");
                    }
                    return Ok(Antwort {
                        status: r.status,
                        empfangen_bytes: r.koerper.len(),
                        koerper: r.koerper,
                    });
                }
                Ok(Err(e)) => return Err(e),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err("netzfehler"),
            }
        }
    }

    fn gesendet_bytes(&self) -> usize {
        self.gesendet.load(Ordering::SeqCst)
    }

    fn empfangen_bytes(&self) -> usize {
        self.empfangen.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::super::anbindung::Abbruch;
    use super::*;
    use std::sync::Condvar;

    struct Freigabesignal(Arc<(Mutex<bool>, Condvar)>);

    impl Drop for Freigabesignal {
        fn drop(&mut self) {
            let (offen, signal) = &*self.0;
            *offen.lock().unwrap() = true;
            signal.notify_all();
        }
    }

    #[test]
    fn panik_gibt_den_hintergrundplatz_frei() {
        let _sperre = hintergrund_test_sperre();
        // Ohne Drop-Wache bliebe nach einer Panik ein Platz dauerhaft belegt.
        let lauf = Lauf::fuer_test(Duration::from_secs(1), Abbruch::neu());
        assert_eq!(
            im_hintergrund::<()>(&lauf, || panic!("Gegenprobe fuer die Drop-Wache")),
            Err("netzfehler")
        );
        let ende = Instant::now() + Duration::from_secs(2);
        while hintergrund_anzahl() != 0 && Instant::now() < ende {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(hintergrund_anzahl(), 0);
    }

    #[test]
    fn zurueckgelassene_arbeiten_bleiben_bis_zum_ende_gezaehlt() {
        // Schuetzt die Grenze auch nach Fristende; ohne Grenze startet Arbeit Nummer 17.
        let _sperre = hintergrund_test_sperre();
        let signal = Freigabesignal(Arc::new((Mutex::new(false), Condvar::new())));
        let gestartet = Arc::new(AtomicUsize::new(0));
        for _ in 0..16 {
            let signal = Arc::clone(&signal.0);
            let gestartet = Arc::clone(&gestartet);
            let lauf = Lauf::fuer_test(Duration::from_millis(100), Abbruch::neu());
            assert_eq!(
                im_hintergrund(&lauf, move || {
                    gestartet.fetch_add(1, Ordering::SeqCst);
                    let (offen, signal) = &*signal;
                    let mut offen = offen.lock().unwrap();
                    while !*offen {
                        offen = signal.wait(offen).unwrap();
                    }
                }),
                Err("zeitlimit")
            );
        }
        assert_eq!(gestartet.load(Ordering::SeqCst), 16);
        assert_eq!(hintergrund_anzahl(), 16);
        let lauf = Lauf::fuer_test(Duration::from_secs(1), Abbruch::neu());
        let start = Instant::now();
        let weiterer_start = Arc::clone(&gestartet);
        assert_eq!(
            im_hintergrund(&lauf, move || {
                weiterer_start.fetch_add(1, Ordering::SeqCst);
            }),
            Err("netzfehler")
        );
        assert!(start.elapsed() < Duration::from_millis(200));
        assert_eq!(gestartet.load(Ordering::SeqCst), 16);
        drop(signal);
        let ende = Instant::now() + Duration::from_secs(2);
        while hintergrund_anzahl() != 0 && Instant::now() < ende {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(hintergrund_anzahl(), 0);
        let lauf = Lauf::fuer_test(Duration::from_secs(1), Abbruch::neu());
        assert_eq!(im_hintergrund(&lauf, || 7), Ok(7));
        let ende = Instant::now() + Duration::from_secs(2);
        while hintergrund_anzahl() != 0 && Instant::now() < ende {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(hintergrund_anzahl(), 0);
    }

    fn produktiver_code(text: &str) -> String {
        let text = text.replace("\r\n", "\n");
        text.split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap()
            .lines()
            .filter(|zeile| !zeile.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn ohne_umwege(code: &str) -> bool {
        [
            "danger_accept",
            "add_root_certificate",
            "proxy",
            "Proxy",
            "env::var",
            "reqwest",
            "set_proxy",
            "HTTP_PROXY",
        ]
        .iter()
        .all(|wort| !code.contains(wort))
    }

    fn mit_tls_pruefung(code: &str) -> bool {
        code.contains("min_protocol_version") && code.contains("connect(&ziel.host")
    }

    fn einmal_gepruefte_ip(code: &str) -> bool {
        code.matches("TcpStream::connect_timeout(&SocketAddr::new(ip")
            .count()
            == 1
            && code.matches("to_socket_addrs").count() == 1
    }

    #[test]
    fn keine_umwege_oder_tls_ausnahmen() {
        // Schuetzt direkte Sockets ohne Umgebungszugriff und ohne TLS-Ausnahmen.
        let code = produktiver_code(include_str!("transport.rs"));
        assert!(ohne_umwege(&code));
        for verboten in [
            "danger_accept",
            "add_root_certificate",
            "proxy",
            "Proxy",
            "env::var",
            "reqwest",
            "set_proxy",
            "HTTP_PROXY",
        ] {
            assert!(!ohne_umwege(&format!("{code}\n{verboten}")));
        }
    }

    #[test]
    fn tls_bindet_den_host_und_die_mindestversion() {
        // Schuetzt TLS 1.2 als Untergrenze und den Hostnamen fuer SNI und Pruefung.
        let code = produktiver_code(include_str!("transport.rs"));
        assert!(mit_tls_pruefung(&code));
        assert!(!mit_tls_pruefung(
            &code.replace("min_protocol_version", "entfernt")
        ));
        assert!(!mit_tls_pruefung(
            &code.replace("connect(&ziel.host", "connect(&ip")
        ));
    }

    #[test]
    fn genau_eine_aufloesung_und_verbindung_zur_ip() {
        // Schuetzt die einmalige Aufloesung und Verbindung nur zur geprueften IP.
        let code = produktiver_code(include_str!("transport.rs"));
        assert!(einmal_gepruefte_ip(&code));
        for muster in [
            "TcpStream::connect_timeout(&SocketAddr::new(ip",
            "to_socket_addrs",
        ] {
            assert!(!einmal_gepruefte_ip(&code.replace(muster, "entfernt")));
            assert!(!einmal_gepruefte_ip(&format!("{code}\n{muster}")));
        }
    }
}
