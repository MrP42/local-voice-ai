//! Lokaler HTTPS-Weiterleitungsproxy mit injizierbarer Windows-Anmeldung (lva#641).
//! Zugangstoken bleiben im Prozess; jede Tunnelverbindung hat ihren eigenen Auth-Zustand.

use std::fmt;
use std::io;
use std::net::{Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use rand::RngCore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const KOPF_MAX: usize = 16 * 1024;
const KOPF_FRIST: Duration = Duration::from_secs(10);
const VERBINDUNGS_FRIST: Duration = Duration::from_secs(15);
const HANDSHAKE_FRIST: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthFehler {
    NichtVerfuegbar,
    Abgelehnt,
}

impl fmt::Display for AuthFehler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NichtVerfuegbar => "Proxy-Anmeldung nicht verfuegbar",
            Self::Abgelehnt => "Proxy-Anmeldung abgelehnt",
        })
    }
}

pub trait ProxyAuthentifizierer: Send {
    fn schritt(
        &mut self,
        proxy_host: &str,
        schema: &str,
        server_token: Option<&[u8]>,
    ) -> Result<Vec<u8>, AuthFehler>;
}

pub type AuthFabrik = Arc<dyn Fn() -> Box<dyn ProxyAuthentifizierer> + Send + Sync>;

pub struct Upstream {
    pub host: String,
    pub port: u16,
}

pub struct WeiterleitungsProxy {
    adresse: SocketAddr,
    token: String,
    aufgabe: JoinHandle<()>,
}

impl fmt::Debug for WeiterleitungsProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeiterleitungsProxy")
            .field("adresse", &self.adresse)
            .field("token", &"[maskiert]")
            .finish_non_exhaustive()
    }
}

// Beim Abbruch der Accept-Aufgabe werden auch laufende Tunnel abgebrochen.
#[derive(Default)]
struct Tunnel(Vec<JoinHandle<()>>);

impl Drop for Tunnel {
    fn drop(&mut self) {
        for aufgabe in &self.0 {
            aufgabe.abort();
        }
    }
}

impl WeiterleitungsProxy {
    pub async fn starten(upstream: Upstream, fabrik: AuthFabrik) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let adresse = listener.local_addr()?;
        let mut zufall = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut zufall);
        let token: String = zufall.iter().map(|b| format!("{b:02x}")).collect();
        let anmeldung = Arc::new(base64(format!("lv:{token}").as_bytes()));
        let upstream = Arc::new(upstream);
        let aufgabe = tokio::spawn(async move {
            let mut tunnel = Tunnel::default();
            let plaetze = Arc::new(Semaphore::new(64));
            while let Ok((client, _)) = listener.accept().await {
                tunnel.0.retain(|t| !t.is_finished());
                let Ok(platz) = plaetze.clone().try_acquire_owned() else {
                    drop(client);
                    continue;
                };
                let upstream = upstream.clone();
                let fabrik = fabrik.clone();
                let anmeldung = anmeldung.clone();
                tunnel.0.push(tokio::spawn(async move {
                    let _platz = platz;
                    let _ = bediene(client, &upstream, &fabrik, &anmeldung).await;
                }));
            }
        });
        Ok(Self {
            adresse,
            token,
            aufgabe,
        })
    }

    pub fn adresse(&self) -> SocketAddr {
        self.adresse
    }

    pub fn proxy_url(&self) -> String {
        format!("http://lv:{}@{}", self.token, self.adresse)
    }
}

impl Drop for WeiterleitungsProxy {
    fn drop(&mut self) {
        self.aufgabe.abort();
    }
}

fn ungueltig() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Ungueltiger Proxy-Kopf")
}

// Kein Vorlesen: Bytes hinter dem Kopf gehören bereits zum Tunnel oder Rumpf.
async fn lies_kopf(sock: &mut TcpStream) -> io::Result<String> {
    timeout(KOPF_FRIST, async {
        let mut kopf = Vec::new();
        loop {
            if kopf.len() == KOPF_MAX {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Kopf zu gross"));
            }
            kopf.push(sock.read_u8().await?);
            if kopf.ends_with(b"\r\n\r\n") {
                return String::from_utf8(kopf).map_err(|_| ungueltig());
            }
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Kopffrist abgelaufen"))?
}

fn felder(kopf: &str) -> io::Result<Vec<(&str, &str)>> {
    kopf.split("\r\n")
        .skip(1)
        .take_while(|s| !s.is_empty())
        .map(|zeile| {
            let (name, wert) = zeile.split_once(':').ok_or_else(ungueltig)?;
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                return Err(ungueltig());
            }
            Ok((name, wert.trim()))
        })
        .collect()
}

fn konstant_gleich(a: &[u8], b: &[u8]) -> bool {
    let mut unterschied = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        unterschied |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    unterschied == 0
}

fn angemeldet(felder: &[(&str, &str)], erwartet: &str) -> bool {
    let mut werte = felder
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("Proxy-Authorization"));
    let Some((_, wert)) = werte.next() else {
        return false;
    };
    let Some((schema, token)) = wert.split_once(' ') else {
        return false;
    };
    werte.next().is_none()
        && schema.eq_ignore_ascii_case("Basic")
        && konstant_gleich(token.as_bytes(), erwartet.as_bytes())
}

fn ziel(authority: &str) -> Option<(&str, u16)> {
    let (host, port) = authority.rsplit_once(':')?;
    if host.is_empty() || host.len() > 253 {
        return None;
    }
    if host.starts_with('[') {
        host.strip_prefix('[')?
            .strip_suffix(']')?
            .parse::<Ipv6Addr>()
            .ok()?;
    } else if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return None;
    }
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port = port.parse::<u16>().ok().filter(|p| *p != 0)?;
    Some((host, port))
}

async fn antwort(client: &mut TcpStream, status: &str, text: &str) -> io::Result<()> {
    let auth = if status.starts_with("407 ") {
        "Proxy-Authenticate: Basic realm=\"lokal\"\r\n"
    } else {
        ""
    };
    client
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\n{auth}Content-Length: {}\r\n\
                 Connection: close\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}",
                text.len()
            )
            .as_bytes(),
        )
        .await
}

async fn bediene(
    mut client: TcpStream,
    upstream: &Upstream,
    fabrik: &AuthFabrik,
    anmeldung: &str,
) -> io::Result<()> {
    let kopf = match lies_kopf(&mut client).await {
        Ok(kopf) => kopf,
        Err(e) => {
            let status = if e.kind() == io::ErrorKind::InvalidData {
                "431 Request Header Fields Too Large"
            } else {
                "400 Bad Request"
            };
            return antwort(&mut client, status, "Ungueltiger Anfragekopf").await;
        }
    };
    let felder = match felder(&kopf) {
        Ok(felder) => felder,
        Err(_) => return antwort(&mut client, "400 Bad Request", "Ungueltiger Anfragekopf").await,
    };
    if !angemeldet(&felder, anmeldung) {
        return antwort(
            &mut client,
            "407 Proxy Authentication Required",
            "Anmeldung erforderlich",
        )
        .await;
    }
    let teile: Vec<_> = kopf
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    if teile.first().copied() != Some("CONNECT") {
        return antwort(&mut client, "501 Not Implemented", "nur HTTPS-Tunnel").await;
    }
    if teile.len() != 3 || !matches!(teile[2], "HTTP/1.0" | "HTTP/1.1") {
        return antwort(&mut client, "400 Bad Request", "Ungueltiges Ziel").await;
    }
    let Some((host, port)) = ziel(teile[1]) else {
        return antwort(&mut client, "400 Bad Request", "Ungueltiges Ziel").await;
    };
    if crate::netz::proxy::ist_lokal(&format!("http://{host}:{port}")) {
        return antwort(&mut client, "403 Forbidden", "Lokales Ziel").await;
    }
    let verbindung = match timeout(HANDSHAKE_FRIST, handshake(upstream, fabrik, teile[1])).await {
        Ok(Ok(sock)) => sock,
        Ok(Err(HandshakeFehler::Verbindung)) => {
            return antwort(&mut client, "502 Bad Gateway", "Proxy nicht erreichbar").await;
        }
        ergebnis => {
            let text = if matches!(ergebnis, Ok(Err(HandshakeFehler::NichtUnterstuetzt))) {
                "Proxy-Anmeldung nicht unterstuetzt"
            } else {
                "Proxy-Anmeldung fehlgeschlagen"
            };
            return antwort(&mut client, "407 Proxy Authentication Required", text).await;
        }
    };
    let mut verbindung = verbindung;
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    tokio::io::copy_bidirectional(&mut client, &mut verbindung).await?;
    Ok(())
}

enum HandshakeFehler {
    Verbindung,
    Anmeldung,
    NichtUnterstuetzt,
}

impl From<io::Error> for HandshakeFehler {
    fn from(_: io::Error) -> Self {
        Self::Anmeldung
    }
}

async fn verbinde(upstream: &Upstream) -> Result<TcpStream, HandshakeFehler> {
    timeout(
        VERBINDUNGS_FRIST,
        TcpStream::connect((upstream.host.as_str(), upstream.port)),
    )
    .await
    .map_err(|_| HandshakeFehler::Verbindung)?
    .map_err(|_| HandshakeFehler::Verbindung)
}

async fn sende_connect(sock: &mut TcpStream, ziel: &str, auth: &str) -> io::Result<()> {
    sock.write_all(
        format!(
            "CONNECT {ziel} HTTP/1.1\r\nHost: {ziel}\r\n\
             Proxy-Connection: Keep-Alive\r\n{auth}\r\n"
        )
        .as_bytes(),
    )
    .await
}

fn status(kopf: &str) -> io::Result<u16> {
    let mut teile = kopf.lines().next().unwrap_or("").split_whitespace();
    if !matches!(teile.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
        return Err(ungueltig());
    }
    teile
        .next()
        .ok_or_else(ungueltig)?
        .parse()
        .map_err(|_| ungueltig())
}

fn challenge(kopf: &str, schema: &str) -> io::Result<Option<Option<Vec<u8>>>> {
    for (name, wert) in felder(kopf)? {
        if !name.eq_ignore_ascii_case("Proxy-Authenticate") {
            continue;
        }
        for angebot in wert.split(',') {
            let mut teile = angebot.split_whitespace();
            if teile.next().is_some_and(|s| s.eq_ignore_ascii_case(schema)) {
                let token = teile.next().map(debase64).transpose()?;
                if teile.next().is_some() {
                    return Err(ungueltig());
                }
                return Ok(Some(token));
            }
        }
    }
    Ok(None)
}

// Nur bekannte, vollständig verworfene Rümpfe erlauben dieselbe Verbindung.
async fn verwerfe_rumpf(sock: &mut TcpStream, kopf: &str) -> io::Result<bool> {
    let mut laenge = None;
    let mut schliessen = kopf.starts_with("HTTP/1.0 ");
    let mut transfer = false;
    for (name, wert) in felder(kopf)? {
        if name.eq_ignore_ascii_case("Content-Length") {
            let n = wert.parse::<u64>().map_err(|_| ungueltig())?;
            if laenge.is_some_and(|alt| alt != n) {
                return Err(ungueltig());
            }
            laenge = Some(n);
        }
        if name.eq_ignore_ascii_case("Transfer-Encoding") {
            transfer = true;
        }
        if (name.eq_ignore_ascii_case("Connection")
            || name.eq_ignore_ascii_case("Proxy-Connection"))
            && wert
                .split(',')
                .any(|w| w.trim().eq_ignore_ascii_case("close"))
        {
            schliessen = true;
        }
    }
    if transfer {
        return Ok(false);
    }
    if let Some(n) = laenge {
        let gelesen = tokio::io::copy(&mut sock.take(n), &mut tokio::io::sink()).await?;
        if gelesen != n {
            return Err(ungueltig());
        }
        Ok(!schliessen)
    } else {
        Ok(false)
    }
}

async fn handshake(
    upstream: &Upstream,
    fabrik: &AuthFabrik,
    ziel: &str,
) -> Result<TcpStream, HandshakeFehler> {
    'neustart: for versuch in 0..=1 {
        let mut sock = verbinde(upstream).await?;
        sende_connect(&mut sock, ziel, "").await?;
        let mut kopf = lies_kopf(&mut sock).await?;
        match status(&kopf)? {
            200..=299 => return Ok(sock),
            407 => {}
            _ => return Err(HandshakeFehler::Anmeldung),
        }
        let (schema, mut token) = if let Some(token) = challenge(&kopf, "Negotiate")? {
            ("Negotiate", token)
        } else if let Some(token) = challenge(&kopf, "NTLM")? {
            ("NTLM", token)
        } else {
            return Err(HandshakeFehler::NichtUnterstuetzt);
        };
        let mut auth = fabrik();
        if !verwerfe_rumpf(&mut sock, &kopf).await? {
            // Nur vor dem ersten Token einmal mit frischem Auth-Zustand neu beginnen.
            if versuch == 0 {
                continue 'neustart;
            }
            return Err(HandshakeFehler::Anmeldung);
        }
        for n in 1..=4 {
            log::debug!("Weiterleitungsproxy: Handshake {n}. Runde");
            let antwort = auth
                .schritt(&upstream.host, schema, token.as_deref())
                .map_err(|_| HandshakeFehler::Anmeldung)?;
            let anmeldung = format!("Proxy-Authorization: {schema} {}\r\n", base64(&antwort));
            sende_connect(&mut sock, ziel, &anmeldung).await?;
            kopf = lies_kopf(&mut sock).await?;
            match status(&kopf)? {
                200..=299 => return Ok(sock),
                407 => {
                    token = Some(
                        challenge(&kopf, schema)?
                            .flatten()
                            .ok_or(HandshakeFehler::Anmeldung)?,
                    );
                    // Nach einem Token gehört der Auth-Zustand zu genau dieser Verbindung.
                    if !verwerfe_rumpf(&mut sock, &kopf).await? {
                        return Err(HandshakeFehler::Anmeldung);
                    }
                }
                _ => return Err(HandshakeFehler::Anmeldung),
            }
        }
        return Err(HandshakeFehler::Anmeldung);
    }
    Err(HandshakeFehler::Anmeldung)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut ausgabe = String::new();
    for block in bytes.chunks(3) {
        let a = block[0];
        let b = block.get(1).copied().unwrap_or(0);
        let c = block.get(2).copied().unwrap_or(0);
        ausgabe.push(ALPHABET[usize::from(a >> 2)] as char);
        ausgabe.push(ALPHABET[usize::from(((a & 3) << 4) | (b >> 4))] as char);
        ausgabe.push(if block.len() > 1 {
            ALPHABET[usize::from(((b & 15) << 2) | (c >> 6))] as char
        } else {
            '='
        });
        ausgabe.push(if block.len() > 2 {
            ALPHABET[usize::from(c & 63)] as char
        } else {
            '='
        });
    }
    ausgabe
}

fn debase64(text: &str) -> io::Result<Vec<u8>> {
    if text.is_empty() || text.len() % 4 != 0 {
        return Err(ungueltig());
    }
    let mut ausgabe = Vec::new();
    for block in text.as_bytes().chunks_exact(4) {
        let mut werte = [0u8; 4];
        for (i, &b) in block.iter().enumerate() {
            if b != b'=' {
                werte[i] = ALPHABET
                    .iter()
                    .position(|&a| a == b)
                    .ok_or_else(ungueltig)? as u8;
            }
        }
        ausgabe.push((werte[0] << 2) | (werte[1] >> 4));
        if block[2] != b'=' {
            ausgabe.push((werte[1] << 4) | (werte[2] >> 2));
        }
        if block[3] != b'=' {
            ausgabe.push((werte[2] << 6) | werte[3]);
        }
    }
    // Die Rückkodierung prüft Position und Anzahl der Füllzeichen sowie Restbits.
    if base64(&ausgabe) != text {
        return Err(ungueltig());
    }
    Ok(ausgabe)
}

/// Ersetzt die Adresse eines entfernten HTTP-Proxys durch den lokalen Weiterleitungsproxy, der
/// bei „407 Proxy-Anmeldung“ den Handshake mit dem Windows-Konto (Negotiate/NTLM, SSPI) uebernimmt
/// (lva#641). Ohne Anmeldung passiert nichts: der Weiterleitungsproxy fragt zuerst unangemeldet
/// und meldet sich nur bei 407 an. Unveraendert bleiben lokale Proxys, `https://`-Proxys (der
/// Weiterleitungsproxy spricht nur Klartext-HTTP zum Proxy), ungueltige Adressen, andere
/// Betriebssysteme und der Fall, dass der lokale Proxy nicht startet.
pub fn vermittelt(proxy: &str) -> String {
    if url::Url::parse(proxy)
        .is_ok_and(|url| !url.username().is_empty() || url.password().is_some())
    {
        return proxy.to_string();
    }
    #[cfg(windows)]
    {
        if let Some(lokal) = vermitteln_windows(proxy) {
            return lokal;
        }
    }
    proxy.to_string()
}

#[cfg(windows)]
fn vermitteln_windows(proxy: &str) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static REGISTER: OnceLock<Mutex<HashMap<String, WeiterleitungsProxy>>> = OnceLock::new();
    static LAUFZEIT: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();

    let url = url::Url::parse(proxy).ok()?;
    if url.scheme() != "http" || super::proxy::ist_lokal(proxy) {
        return None;
    }
    let host = url.host_str()?.to_string();
    let port = url.port_or_known_default()?;
    let schluessel = format!("{}:{port}", host.to_ascii_lowercase());
    let register = REGISTER.get_or_init(|| Mutex::new(HashMap::new()));
    let mut karte = register.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(vorhanden) = karte.get(&schluessel) {
        return Some(vorhanden.proxy_url());
    }
    let laufzeit = LAUFZEIT
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("netz-weiterleitung")
                .enable_all()
                .build()
                .ok()
        })
        .as_ref()?;
    // `block_on` darf nicht auf einem Tokio-Arbeitsthread laufen: eigener, kurzer Thread.
    let gestartet = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                laufzeit.block_on(WeiterleitungsProxy::starten(
                    Upstream {
                        host: host.clone(),
                        port,
                    },
                    super::sspi::fabrik(),
                ))
            })
            .join()
            .ok()
    })?;
    match gestartet {
        Ok(weiterleitung) => {
            let adresse = weiterleitung.proxy_url();
            karte.insert(schluessel, weiterleitung);
            Some(adresse)
        }
        Err(_) => {
            log::warn!("Weiterleitungsproxy nicht gestartet; Proxy ohne Windows-Anmeldung");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    const TEST_FRIST: Duration = Duration::from_secs(5);
    const ZIEL: &str = "extern.example.org:443";
    const ERFOLG: &str = "HTTP/1.1 200 Connection Established\r\n\r\n";

    type Aufruf = (String, String, Option<Vec<u8>>);
    type Aufrufe = Arc<Mutex<Vec<Aufruf>>>;

    struct FakeAuth {
        aufrufe: Aufrufe,
        token: VecDeque<Vec<u8>>,
        fehler: Option<AuthFehler>,
    }

    impl ProxyAuthentifizierer for FakeAuth {
        fn schritt(
            &mut self,
            host: &str,
            schema: &str,
            challenge: Option<&[u8]>,
        ) -> Result<Vec<u8>, AuthFehler> {
            self.aufrufe.lock().unwrap().push((
                host.into(),
                schema.into(),
                challenge.map(<[u8]>::to_vec),
            ));
            if let Some(fehler) = &self.fehler {
                return Err(fehler.clone());
            }
            Ok(self.token.pop_front().unwrap_or_else(|| b"weiter".to_vec()))
        }
    }

    fn fabrik(fehler: Option<AuthFehler>) -> (AuthFabrik, Aufrufe) {
        let aufrufe = Arc::new(Mutex::new(Vec::new()));
        let log = aufrufe.clone();
        let fabrik: AuthFabrik = Arc::new(move || {
            Box::new(FakeAuth {
                aufrufe: log.clone(),
                token: VecDeque::from([b"anfang".to_vec(), b"antwort".to_vec()]),
                fehler: fehler.clone(),
            })
        });
        (fabrik, aufrufe)
    }

    #[derive(Clone)]
    struct Schritt {
        auth: Option<String>,
        antwort: String,
    }

    fn schritt(auth: Option<&str>, antwort: &str) -> Schritt {
        Schritt {
            auth: auth.map(str::to_string),
            antwort: antwort.into(),
        }
    }

    fn herausforderung(schema: &str, token: Option<&[u8]>) -> String {
        let token = token.map(|t| format!(" {}", base64(t))).unwrap_or_default();
        format!(
            "HTTP/1.1 407 Proxy Authentication Required\r\n\
             Proxy-Authenticate: {schema}{token}\r\nContent-Length: 4\r\n\r\nrest"
        )
    }

    struct FakeUpstream {
        port: u16,
        verbindungen: Arc<AtomicUsize>,
        koepfe: Arc<Mutex<Vec<(usize, String)>>>,
        erwartungen: Arc<Mutex<Vec<bool>>>,
        aufgabe: JoinHandle<()>,
    }

    impl Drop for FakeUpstream {
        fn drop(&mut self) {
            self.aufgabe.abort();
        }
    }

    impl FakeUpstream {
        async fn starten(skripte: Vec<Vec<Schritt>>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let verbindungen = Arc::new(AtomicUsize::new(0));
            let zaehler = verbindungen.clone();
            let koepfe = Arc::new(Mutex::new(Vec::new()));
            let log = koepfe.clone();
            let erwartungen = Arc::new(Mutex::new(Vec::new()));
            let pruefungen = erwartungen.clone();
            let aufgabe = tokio::spawn(async move {
                let mut tunnel = Tunnel::default();
                while let Ok((mut sock, _)) = listener.accept().await {
                    let nummer = zaehler.fetch_add(1, Ordering::SeqCst);
                    let skript = skripte.get(nummer).cloned().unwrap_or_default();
                    let log = log.clone();
                    let pruefungen = pruefungen.clone();
                    tunnel.0.retain(|t| !t.is_finished());
                    tunnel.0.push(tokio::spawn(async move {
                        for schritt in skript {
                            let Ok(kopf) = lies_kopf(&mut sock).await else {
                                pruefungen.lock().unwrap().push(false);
                                return;
                            };
                            let auth =
                                felder(&kopf).unwrap().into_iter().find_map(|(name, wert)| {
                                    name.eq_ignore_ascii_case("Proxy-Authorization")
                                        .then_some(wert)
                                });
                            let passt = auth == schritt.auth.as_deref()
                                && kopf.starts_with(&format!("CONNECT {ZIEL} HTTP/1.1\r\n"))
                                && kopf.contains(&format!("Host: {ZIEL}\r\n"))
                                && kopf.contains("Proxy-Connection: Keep-Alive\r\n");
                            pruefungen.lock().unwrap().push(passt);
                            log.lock().unwrap().push((nummer, kopf));
                            if sock.write_all(schritt.antwort.as_bytes()).await.is_err() {
                                return;
                            }
                            if schritt.antwort.starts_with("HTTP/1.1 200 ") {
                                // Sofortige Nutzdaten dürfen beim Kopflesen nicht verloren gehen.
                                if sock.write_all(b"bereit").await.is_err() {
                                    return;
                                }
                                let mut puffer = [0u8; 64];
                                while let Ok(n) = sock.read(&mut puffer).await {
                                    if n == 0 || sock.write_all(&puffer[..n]).await.is_err() {
                                        break;
                                    }
                                }
                                return;
                            }
                        }
                    }));
                }
            });
            Self {
                port,
                verbindungen,
                koepfe,
                erwartungen,
                aufgabe,
            }
        }

        async fn proxy(&self, fabrik: AuthFabrik) -> WeiterleitungsProxy {
            WeiterleitungsProxy::starten(
                Upstream {
                    host: "127.0.0.1".into(),
                    port: self.port,
                },
                fabrik,
            )
            .await
            .unwrap()
        }

        fn pruefe(&self, anfragen: usize, verbindungen: usize) {
            assert_eq!(self.verbindungen.load(Ordering::SeqCst), verbindungen);
            assert_eq!(self.koepfe.lock().unwrap().len(), anfragen);
            let erwartungen = self.erwartungen.lock().unwrap();
            assert_eq!(erwartungen.len(), anfragen);
            assert!(erwartungen.iter().all(|&passt| passt));
        }
    }

    async fn client(proxy: &WeiterleitungsProxy, zeile: &str, auth: Option<&str>) -> TcpStream {
        let mut sock = TcpStream::connect(proxy.adresse()).await.unwrap();
        let auth = auth
            .map(|a| format!("Proxy-Authorization: {a}\r\n"))
            .unwrap_or_default();
        sock.write_all(format!("{zeile}\r\n{auth}\r\n").as_bytes())
            .await
            .unwrap();
        sock
    }

    fn anmeldung(proxy: &WeiterleitungsProxy) -> String {
        format!("Basic {}", base64(format!("lv:{}", proxy.token).as_bytes()))
    }

    async fn anfrage(proxy: &WeiterleitungsProxy, zeile: &str) -> String {
        let mut sock = client(proxy, zeile, Some(&anmeldung(proxy))).await;
        let mut antwort = String::new();
        timeout(TEST_FRIST, sock.read_to_string(&mut antwort))
            .await
            .unwrap()
            .unwrap();
        antwort
    }

    async fn echo(proxy: &WeiterleitungsProxy) {
        timeout(TEST_FRIST, async {
            let mut sock = client(
                proxy,
                &format!("CONNECT {ZIEL} HTTP/1.1"),
                Some(&anmeldung(proxy)),
            )
            .await;
            assert_eq!(lies_kopf(&mut sock).await.unwrap(), ERFOLG);
            let mut begruessung = [0u8; 6];
            sock.read_exact(&mut begruessung).await.unwrap();
            assert_eq!(&begruessung, b"bereit");
            sock.write_all(b"hin und zurueck").await.unwrap();
            let mut daten = [0u8; 15];
            sock.read_exact(&mut daten).await.unwrap();
            assert_eq!(&daten, b"hin und zurueck");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn sofortiger_tunnel_mit_echo_ohne_auth() {
        let fake = FakeUpstream::starten(vec![vec![schritt(None, ERFOLG)]]).await;
        let (fabrik, aufrufe) = fabrik(None);
        let proxy = fake.proxy(fabrik).await;
        echo(&proxy).await;
        fake.pruefe(1, 1);
        assert!(aufrufe.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn negotiate_eine_runde_bevorzugt_vor_ntlm() {
        let angebot = "HTTP/1.1 407 Proxy Authentication Required\r\n\
                       Proxy-Authenticate: NTLM\r\nProxy-Authenticate: Negotiate\r\n\
                       Content-Length: 0\r\n\r\n";
        let fake = FakeUpstream::starten(vec![vec![
            schritt(None, angebot),
            schritt(Some("Negotiate YW5mYW5n"), ERFOLG),
        ]])
        .await;
        let (fabrik, aufrufe) = fabrik(None);
        echo(&fake.proxy(fabrik).await).await;
        fake.pruefe(2, 1);
        assert_eq!(
            *aufrufe.lock().unwrap(),
            vec![("127.0.0.1".into(), "Negotiate".into(), None)]
        );
    }

    #[tokio::test]
    async fn negotiate_folge_challenge_und_rumpf() {
        let fake = FakeUpstream::starten(vec![vec![
            schritt(None, &herausforderung("Negotiate", None)),
            schritt(
                Some("Negotiate YW5mYW5n"),
                &herausforderung("Negotiate", Some(b"frage")),
            ),
            schritt(Some("Negotiate YW50d29ydA=="), ERFOLG),
        ]])
        .await;
        let (fabrik, aufrufe) = fabrik(None);
        echo(&fake.proxy(fabrik).await).await;
        fake.pruefe(3, 1);
        let aufrufe = aufrufe.lock().unwrap();
        assert_eq!(aufrufe.len(), 2);
        assert_eq!(aufrufe[0].2, None);
        assert_eq!(aufrufe[1].2.as_deref(), Some(b"frage".as_slice()));
    }

    #[tokio::test]
    async fn ntlm_bleibt_auf_einer_verbindung() {
        let fake = FakeUpstream::starten(vec![vec![
            schritt(None, &herausforderung("NTLM", None)),
            schritt(
                Some("NTLM YW5mYW5n"),
                &herausforderung("NTLM", Some(b"type2")),
            ),
            schritt(Some("NTLM YW50d29ydA=="), ERFOLG),
        ]])
        .await;
        let (fabrik, aufrufe) = fabrik(None);
        echo(&fake.proxy(fabrik).await).await;
        fake.pruefe(3, 1);
        let aufrufe = aufrufe.lock().unwrap();
        assert_eq!(aufrufe.len(), 2);
        assert_eq!(aufrufe[0].1, "NTLM");
        assert_eq!(aufrufe[0].2, None);
        assert_eq!(aufrufe[1].2.as_deref(), Some(b"type2".as_slice()));
    }

    #[tokio::test]
    async fn basic_digest_und_unbekannt_ohne_auth_versuch() {
        for schema in [
            "Basic realm=\"firma\"",
            "Digest realm=\"firma\"",
            "Unbekannt",
        ] {
            let fake =
                FakeUpstream::starten(vec![vec![schritt(None, &herausforderung(schema, None))]])
                    .await;
            let (fabrik, aufrufe) = fabrik(None);
            let proxy = fake.proxy(fabrik).await;
            let antwort = anfrage(&proxy, &format!("CONNECT {ZIEL} HTTP/1.1")).await;
            assert!(antwort.starts_with("HTTP/1.1 407 "));
            assert!(antwort.contains("Proxy-Authenticate: Basic realm=\"lokal\""));
            assert!(antwort.contains("nicht unterstuetzt"));
            assert!(aufrufe.lock().unwrap().is_empty());
            fake.pruefe(1, 1);
        }
    }

    #[tokio::test]
    async fn falsche_und_fehlende_client_anmeldung_ohne_upstream() {
        let fake = FakeUpstream::starten(vec![]).await;
        let (fabrik, aufrufe) = fabrik(None);
        let proxy = fake.proxy(fabrik).await;
        for auth in [None, Some("Basic bHY6ZmFsc2No")] {
            let mut sock = client(&proxy, &format!("CONNECT {ZIEL} HTTP/1.1"), auth).await;
            let mut antwort = String::new();
            timeout(TEST_FRIST, sock.read_to_string(&mut antwort))
                .await
                .unwrap()
                .unwrap();
            assert!(antwort.starts_with("HTTP/1.1 407 "));
            assert!(antwort.contains("Proxy-Authenticate: Basic realm=\"lokal\""));
        }
        fake.pruefe(0, 0);
        assert!(aufrufe.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn lokale_ziele_ohne_upstream() {
        let fake = FakeUpstream::starten(vec![]).await;
        let (fabrik, _) = fabrik(None);
        let proxy = fake.proxy(fabrik).await;
        for ziel in [
            "127.0.0.1:443",
            "localhost:443",
            "[::1]:443",
            "169.254.1.2:80",
        ] {
            let antwort = anfrage(&proxy, &format!("CONNECT {ziel} HTTP/1.1")).await;
            assert!(antwort.starts_with("HTTP/1.1 403 "));
        }
        fake.pruefe(0, 0);
    }

    #[tokio::test]
    async fn nur_connect() {
        let fake = FakeUpstream::starten(vec![]).await;
        let (fabrik, _) = fabrik(None);
        let proxy = fake.proxy(fabrik).await;
        let antwort = anfrage(&proxy, "GET http://extern.example.org/ HTTP/1.1").await;
        assert!(antwort.starts_with("HTTP/1.1 501 "));
        assert!(antwort.contains("nur HTTPS-Tunnel"));
        fake.pruefe(0, 0);
    }

    #[tokio::test]
    async fn ungueltige_ziele_ohne_upstream() {
        let fake = FakeUpstream::starten(vec![]).await;
        let (fabrik, _) = fabrik(None);
        let proxy = fake.proxy(fabrik).await;
        for ziel in [
            ":443",
            "extern.example.org:0",
            "extern.example.org:65536",
            "extern.example.org:+443",
            "extern.example.org:x",
            "extern.example.org/pfad:443",
            "nutzer@extern.example.org:443",
            "[kaputt]:443",
            "::1:443",
            &format!("{}:443", "a".repeat(254)),
        ] {
            let antwort = anfrage(&proxy, &format!("CONNECT {ziel} HTTP/1.1")).await;
            assert!(antwort.starts_with("HTTP/1.1 400 "));
        }
        fake.pruefe(0, 0);
    }

    #[tokio::test]
    async fn zu_grosser_kopf_ohne_upstream() {
        let fake = FakeUpstream::starten(vec![]).await;
        let (fabrik, _) = fabrik(None);
        let proxy = fake.proxy(fabrik).await;
        let mut sock = TcpStream::connect(proxy.adresse()).await.unwrap();
        // Genau bis zur Grenze senden: So bleibt kein ungelesener TCP-Rest zurück.
        sock.write_all(&vec![b'a'; KOPF_MAX]).await.unwrap();
        let kopf = timeout(TEST_FRIST, lies_kopf(&mut sock))
            .await
            .unwrap()
            .unwrap();
        assert!(kopf.starts_with("HTTP/1.1 431 "));
        fake.pruefe(0, 0);
    }

    #[tokio::test]
    async fn geschlossener_upstream_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let (fabrik, aufrufe) = fabrik(None);
        let proxy = WeiterleitungsProxy::starten(
            Upstream {
                host: "127.0.0.1".into(),
                port,
            },
            fabrik,
        )
        .await
        .unwrap();
        let antwort = anfrage(&proxy, &format!("CONNECT {ZIEL} HTTP/1.1")).await;
        assert!(antwort.starts_with("HTTP/1.1 502 "));
        assert!(aufrufe.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn auth_fehler_schliesst_tunnel() {
        for fehler in [AuthFehler::NichtVerfuegbar, AuthFehler::Abgelehnt] {
            let fake = FakeUpstream::starten(vec![vec![schritt(
                None,
                &herausforderung("Negotiate", None),
            )]])
            .await;
            let (fabrik, aufrufe) = fabrik(Some(fehler));
            let proxy = fake.proxy(fabrik).await;
            let antwort = anfrage(&proxy, &format!("CONNECT {ZIEL} HTTP/1.1")).await;
            assert!(antwort.starts_with("HTTP/1.1 407 "));
            assert!(antwort.contains("fehlgeschlagen"));
            assert_eq!(aufrufe.lock().unwrap().len(), 1);
            fake.pruefe(1, 1);
        }
    }

    #[tokio::test]
    async fn ntlm_close_oder_unbekannte_laenge_hoechstens_ein_neustart() {
        for kopf in ["Connection: close\r\nContent-Length: 0\r\n", ""] {
            let antwort = format!(
                "HTTP/1.1 407 Proxy Authentication Required\r\n\
                 Proxy-Authenticate: NTLM\r\n{kopf}\r\n"
            );
            let fake = FakeUpstream::starten(vec![
                vec![schritt(None, &antwort)],
                vec![schritt(None, &antwort)],
            ])
            .await;
            let (fabrik, aufrufe) = fabrik(None);
            let proxy = fake.proxy(fabrik).await;
            let antwort = anfrage(&proxy, &format!("CONNECT {ZIEL} HTTP/1.1")).await;
            assert!(antwort.starts_with("HTTP/1.1 407 "));
            assert!(antwort.contains("fehlgeschlagen"));
            assert!(aufrufe.lock().unwrap().is_empty());
            fake.pruefe(2, 2);
        }
    }

    #[tokio::test]
    async fn ntlm_beginnt_nach_erster_407_mit_close_frisch() {
        let angebot = "HTTP/1.1 407 Proxy Authentication Required\r\n\
                       Proxy-Authenticate: NTLM\r\nConnection: close\r\n\
                       Content-Length: 0\r\n\r\n";
        let fake = FakeUpstream::starten(vec![
            vec![schritt(None, angebot)],
            vec![
                schritt(None, &herausforderung("NTLM", None)),
                schritt(
                    Some("NTLM YW5mYW5n"),
                    &herausforderung("NTLM", Some(b"type2")),
                ),
                schritt(Some("NTLM YW50d29ydA=="), ERFOLG),
            ],
        ])
        .await;
        let (fabrik, aufrufe) = fabrik(None);
        let instanzen = Arc::new(AtomicUsize::new(0));
        let zaehler = instanzen.clone();
        let gezaehlt: AuthFabrik = Arc::new(move || {
            zaehler.fetch_add(1, Ordering::SeqCst);
            fabrik()
        });
        echo(&fake.proxy(gezaehlt).await).await;
        fake.pruefe(4, 2);
        assert_eq!(instanzen.load(Ordering::SeqCst), 2);
        let aufrufe = aufrufe.lock().unwrap();
        assert_eq!(aufrufe.len(), 2);
        assert_eq!(aufrufe[0].2, None);
        assert_eq!(aufrufe[1].2.as_deref(), Some(b"type2".as_slice()));
    }

    #[tokio::test]
    async fn close_nach_token_ohne_neue_verbindung() {
        for schema in ["Negotiate", "NTLM"] {
            // Sowohl angekuendigtes Schliessen als auch EOF mitten im Handshake.
            for mit_antwort in [true, false] {
                let antwort = if mit_antwort {
                    format!(
                        "HTTP/1.1 407 Proxy Authentication Required\r\n\
                         Proxy-Authenticate: {schema} {}\r\n\
                         Connection: close\r\nContent-Length: 0\r\n\r\n",
                        base64(b"frage")
                    )
                } else {
                    String::new()
                };
                let fake = FakeUpstream::starten(vec![vec![
                    schritt(None, &herausforderung(schema, None)),
                    schritt(Some(&format!("{schema} YW5mYW5n")), &antwort),
                ]])
                .await;
                let (fabrik, aufrufe) = fabrik(None);
                let proxy = fake.proxy(fabrik).await;
                let antwort = anfrage(&proxy, &format!("CONNECT {ZIEL} HTTP/1.1")).await;
                assert!(antwort.starts_with("HTTP/1.1 407 "));
                assert_eq!(aufrufe.lock().unwrap().len(), 1);
                fake.pruefe(2, 1);
            }
        }
    }

    #[tokio::test]
    async fn hoechstens_64_verbindungen_und_freigabe_nach_schliessen() {
        let fake = FakeUpstream::starten(vec![]).await;
        let (fabrik, _) = fabrik(None);
        let proxy = fake.proxy(fabrik).await;
        let mut offen = Vec::new();
        for _ in 0..64 {
            offen.push(TcpStream::connect(proxy.adresse()).await.unwrap());
        }
        let mut ueberzaehlig = TcpStream::connect(proxy.adresse()).await.unwrap();
        let mut byte = [0u8; 1];
        assert_eq!(
            timeout(TEST_FRIST, ueberzaehlig.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        for sock in &offen {
            assert_eq!(
                sock.try_read(&mut byte).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }
        let mut geschlossen = offen.pop().unwrap();
        geschlossen.shutdown().await.unwrap();
        let mut antwort = String::new();
        timeout(TEST_FRIST, geschlossen.read_to_string(&mut antwort))
            .await
            .unwrap()
            .unwrap();
        assert!(antwort.starts_with("HTTP/1.1 400 "));
        // Nach EOF ist der Verbindungstask beendet und sein Platz wieder frei.
        let mut neu = client(&proxy, &format!("CONNECT {ZIEL} HTTP/1.1"), None).await;
        let mut antwort = String::new();
        timeout(TEST_FRIST, neu.read_to_string(&mut antwort))
            .await
            .unwrap()
            .unwrap();
        assert!(antwort.starts_with("HTTP/1.1 407 "));
        fake.pruefe(0, 0);
    }

    #[tokio::test]
    async fn hoechstens_vier_auth_runden() {
        let mut skript = vec![schritt(None, &herausforderung("Negotiate", None))];
        for token in ["YW5mYW5n", "YW50d29ydA==", "d2VpdGVy", "d2VpdGVy"] {
            skript.push(schritt(
                Some(&format!("Negotiate {token}")),
                &herausforderung("Negotiate", Some(b"nochmal")),
            ));
        }
        let fake = FakeUpstream::starten(vec![skript]).await;
        let (fabrik, aufrufe) = fabrik(None);
        let proxy = fake.proxy(fabrik).await;
        let antwort = anfrage(&proxy, &format!("CONNECT {ZIEL} HTTP/1.1")).await;
        assert!(antwort.starts_with("HTTP/1.1 407 "));
        assert_eq!(aufrufe.lock().unwrap().len(), 4);
        fake.pruefe(5, 1);
    }

    #[tokio::test]
    async fn drop_schliesst_port_und_laufenden_tunnel() {
        let fake = FakeUpstream::starten(vec![vec![schritt(None, ERFOLG)]]).await;
        let (fabrik, _) = fabrik(None);
        let proxy = fake.proxy(fabrik).await;
        let adresse = proxy.adresse();
        let mut sock = client(
            &proxy,
            &format!("CONNECT {ZIEL} HTTP/1.1"),
            Some(&anmeldung(&proxy)),
        )
        .await;
        assert_eq!(lies_kopf(&mut sock).await.unwrap(), ERFOLG);
        let mut bereit = [0u8; 6];
        sock.read_exact(&mut bereit).await.unwrap();
        drop(proxy);
        timeout(TEST_FRIST, async {
            let mut byte = [0u8; 1];
            assert_eq!(sock.read(&mut byte).await.unwrap(), 0);
            loop {
                if TcpStream::connect(adresse).await.is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn debug_maskiert_zufaelligen_token() {
        let fake = FakeUpstream::starten(vec![]).await;
        let (fabrik, _) = fabrik(None);
        let proxy = fake.proxy(fabrik.clone()).await;
        let anderer = fake.proxy(fabrik).await;
        assert_eq!(proxy.token.len(), 32);
        assert!(proxy.token.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(proxy.token, anderer.token);
        assert!(!format!("{proxy:?}").contains(&proxy.token));
        assert!(proxy.proxy_url().starts_with("http://lv:"));
        assert!(proxy
            .proxy_url()
            .ends_with(&format!("@{}", proxy.adresse())));
    }

    #[tokio::test]
    async fn negotiate_verbindet_nach_close_neu_je_tunnel_frischer_zustand() {
        let angebot = "HTTP/1.1 407 Proxy Authentication Required\r\n\
                       Proxy-Authenticate: Negotiate\r\nConnection: close\r\n\r\n";
        let fake = FakeUpstream::starten(vec![
            vec![schritt(None, angebot)],
            vec![
                schritt(None, &herausforderung("Negotiate", None)),
                schritt(Some("Negotiate YW5mYW5n"), ERFOLG),
            ],
            vec![
                schritt(None, &herausforderung("Negotiate", None)),
                schritt(Some("Negotiate YW5mYW5n"), ERFOLG),
            ],
        ])
        .await;
        let (fabrik, aufrufe) = fabrik(None);
        let proxy = fake.proxy(fabrik).await;
        echo(&proxy).await;
        fake.pruefe(3, 2);
        // Der zweite Tunnel beginnt trotz desselben Proxys wieder mit dem Anfangstoken.
        echo(&proxy).await;
        fake.pruefe(5, 3);
        let aufrufe = aufrufe.lock().unwrap();
        assert_eq!(aufrufe.len(), 2);
        assert!(aufrufe
            .iter()
            .all(|(_, schema, token)| { schema == "Negotiate" && token.is_none() }));
    }

    #[tokio::test]
    async fn ungueltige_challenge_wird_nicht_an_auth_weitergegeben() {
        for token in ["!!!!", "Zg=", "Zh==", "YWJj=AAA"] {
            let angebot = format!(
                "HTTP/1.1 407 Proxy Authentication Required\r\n\
                 Proxy-Authenticate: Negotiate {token}\r\nContent-Length: 0\r\n\r\n"
            );
            let fake = FakeUpstream::starten(vec![vec![schritt(None, &angebot)]]).await;
            let (fabrik, aufrufe) = fabrik(None);
            let proxy = fake.proxy(fabrik).await;
            let antwort = anfrage(&proxy, &format!("CONNECT {ZIEL} HTTP/1.1")).await;
            assert!(antwort.starts_with("HTTP/1.1 407 "));
            assert!(aufrufe.lock().unwrap().is_empty());
            fake.pruefe(1, 1);
        }
    }

    #[test]
    fn vermittelt_laesst_ausdrueckliche_zugangsdaten_unveraendert() {
        for proxy in [
            "http://nutzer:pw@proxy.example.invalid:3128",
            "http://nutzer@proxy.example.invalid:3128",
            "http://:pw@proxy.example.invalid:3128",
        ] {
            assert_eq!(vermittelt(proxy), proxy);
        }
    }

    #[test]
    fn vermittelt_laesst_lokale_https_und_ungueltige_proxys_unveraendert() {
        for proxy in [
            "http://127.0.0.1:3128",
            "http://localhost:8080",
            "https://proxy.example.invalid:3128",
            "kein proxy",
            "socks5://proxy.example.invalid:1080",
        ] {
            assert_eq!(vermittelt(proxy), proxy, "{proxy}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn vermittelt_ersetzt_einen_entfernten_http_proxy_durch_den_lokalen() {
        let erste = vermittelt("http://proxy.example.invalid:3128");
        assert!(erste.starts_with("http://lv:"), "{erste}");
        assert!(erste.contains("@127.0.0.1:"), "{erste}");
        // Derselbe Upstream bekommt denselben lokalen Proxy (kein zweiter Listener).
        assert_eq!(vermittelt("http://proxy.example.invalid:3128"), erste);
        // Ein anderer Upstream bekommt einen eigenen.
        let andere = vermittelt("http://proxy2.example.invalid:3128");
        assert_ne!(andere, erste);
    }
}
