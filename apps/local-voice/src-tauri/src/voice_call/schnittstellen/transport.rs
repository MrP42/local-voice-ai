//! Direkte DNS-, TCP/TLS- und HTTP/1.1-Adapter ohne Proxy oder Verbindungszustand
//! zwischen Aufrufen. TLS prueft Zertifikat und urspruenglichen Hostnamen.
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError},
    Arc,
};
use std::time::{Duration, Instant};

use super::anbindung::{
    Antwort, Lauf, Namensaufloesung, Verbindung, Verbindungsaufbau, Ziel, MAX_ANTWORT_BYTES,
};
use super::http1::{baue_anfragebytes, lies_antwort, Rohantwort};
use super::kodierung::Wert;

/// Wartet abbrechbar auf blockierende Systemaufrufe. Bei Abbruch oder Fristende
/// bleibt der Thread bis zum Ende seines Systemaufrufs zurueck; sein Ergebnis
/// wird verworfen. Insbesondere laesst sich getaddrinfo nicht unterbrechen.
fn im_hintergrund<T: Send + 'static>(
    lauf: &Lauf,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, &'static str> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("lva-schnittstelle".into())
        .spawn(move || {
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
        im_hintergrund(lauf, move || {
            let rest = restzeit(ende)?;
            let stream = TcpStream::connect_timeout(&SocketAddr::new(ip, ziel.port), rest)
                .map_err(|_| netzfehler(ende))?;
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
            Ok(HttpVerbindung::neu(strom, ziel, ende))
        })?
    }
}

/// Eine Anfrage je Verbindung; Schreiben und Lesen laufen im Hintergrund.
pub struct HttpVerbindung {
    strom: Option<Strom>,
    ziel: Ziel,
    ende: Instant,
    halt: Arc<AtomicBool>,
    gesendet: usize,
    ergebnis: Option<Receiver<Result<Rohantwort, &'static str>>>,
}

impl HttpVerbindung {
    fn neu(strom: Strom, ziel: Ziel, ende: Instant) -> Self {
        Self {
            strom: Some(strom),
            ziel,
            ende,
            halt: Arc::new(AtomicBool::new(false)),
            gesendet: 0,
            ergebnis: None,
        }
    }
}

impl Drop for HttpVerbindung {
    fn drop(&mut self) {
        // Ein wartender Leser bemerkt den Halt spaetestens nach einem Lesefenster.
        self.halt.store(true, Ordering::SeqCst);
    }
}

impl Verbindung for HttpVerbindung {
    fn senden(&mut self, anfrage: &Wert) -> Result<(), &'static str> {
        let bytes = baue_anfragebytes(&self.ziel, anfrage).map_err(|_| "netzfehler")?;
        let mut strom = self.strom.take().ok_or("netzfehler")?;
        let anzahl = bytes.len();
        let ende = self.ende;
        let halt = Arc::clone(&self.halt);
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("lva-schnittstelle".into())
            .spawn(move || {
                let ergebnis = (|| {
                    let rest = restzeit(ende)?.max(Duration::from_millis(1));
                    strom
                        .tcp()
                        .set_write_timeout(Some(rest))
                        .map_err(|_| "netzfehler")?;
                    strom.write_all(&bytes).map_err(|_| "netzfehler")?;
                    let rest = restzeit(ende)?;
                    strom
                        .tcp()
                        .set_read_timeout(Some(rest.min(Duration::from_millis(100))))
                        .map_err(|_| "netzfehler")?;
                    let mut warten = || {
                        if halt.load(Ordering::SeqCst) {
                            Err("abgebrochen")
                        } else if Instant::now() >= ende {
                            Err("zeitlimit")
                        } else {
                            Ok(())
                        }
                    };
                    warten()?;
                    lies_antwort(&mut strom, MAX_ANTWORT_BYTES, &mut warten)
                })();
                let _ = tx.send(ergebnis);
            })
            .map_err(|_| "netzfehler")?;
        self.ergebnis = Some(rx);
        // Die Schnittstelle zaehlt die fuer den Hintergrundversand bestimmten Bytes.
        self.gesendet = anzahl;
        Ok(())
    }

    fn antwort(&mut self, lauf: &Lauf, max_bytes: usize) -> Result<Antwort, &'static str> {
        let rx = self.ergebnis.as_ref().ok_or("netzfehler")?;
        loop {
            if let Err(e) = lauf.pruefen() {
                self.halt.store(true, Ordering::SeqCst);
                return Err(e);
            }
            let Some(rest) = lauf.rest() else {
                self.halt.store(true, Ordering::SeqCst);
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
        self.gesendet
    }
}

#[cfg(test)]
mod tests {
    fn produktiver_code(text: &str) -> String {
        let text = text.replace("\r\n", "\n");
        text.split("#[cfg(test)]")
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
