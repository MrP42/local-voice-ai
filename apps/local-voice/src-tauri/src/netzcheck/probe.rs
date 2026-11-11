//! Die echte Probe: Namensauflösung, TLS-Aufbau, HTTP-Status, Dauer (lva#496).
//!
//! Läuft nur, wenn der Nutzer auf „Verbindungen prüfen“ drückt.
//!
//! Ablauf je Gegenstelle, alles innerhalb eines gemeinsamen Zeitbudgets:
//!
//! 1. Namensauflösung (nur ohne Proxy; mit Proxy löst der Proxy auf).
//! 2. TCP und TLS-Handshake über `native-tls` (dieselbe Bibliothek wie der
//!    HTTP-Client der App, also dieselben Stammzertifikate des Systems); nur
//!    ohne Proxy und nur bei `https`. Liefert Dauer, das Zertifikat der
//!    Gegenstelle (Aussteller) und die Art eines Fehlers.
//!    Mit einem HTTP-Proxy tunnelt die Probe selbst (`CONNECT`) und liest so
//!    dasselbe Zertifikat, bestmöglich (ohne Zugangsdaten; PAC und SOCKS: nein).
//! 3. HTTP `HEAD /` mit dem Standard-Client von `reqwest` (nutzt Proxy und
//!    Zertifikatsspeicher wie die App). Es werden KEINE Zugangsdaten gesendet
//!    und nie ein anderer Pfad als `/`: weder Schlüssel noch Adressen der
//!    Nutzer verlassen den Rechner. Jeder HTTP-Status, auch 401/404, zeigt,
//!    dass die Gegenstelle antwortet.
//!
//! Der Aussteller des Zertifikats wird aus der DER-Kodierung gelesen
//! ([`issuer_from_der`], minimal, ohne neue Abhängigkeit), geschwärzt und
//! gekürzt; er dient nur dem Hinweis auf einen TLS-aufbrechenden Proxy.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::proxy::ProxyInfo;
use super::redact::{scrub_text, Endpoint};
use super::{ms, Failure, Probe, ProbeOutcome, Stage, PROBE_TIMEOUT};

/// Zeit, die dem äußeren Zeitlimit als Puffer bleibt, damit die Probe ihren
/// Schritt noch selbst benennen kann.
const MARGIN: Duration = Duration::from_millis(150);

pub struct SystemProbe {
    budget: Duration,
    /// Es ist ein Proxy eingerichtet: Auflösung und TLS-Aufbau macht dann der
    /// Proxy, die App misst nur den Weg über ihn.
    proxied: bool,
    /// Ein einfacher HTTP-Proxy (`host`, `port`) für den Tunnel zum Auslesen
    /// des Zertifikats; `None` bei PAC, SOCKS oder ohne Proxy.
    tunnel: Option<(String, u16)>,
}

impl SystemProbe {
    pub fn new(proxy: &ProxyInfo) -> Self {
        let tunnel = proxy
            .entries
            .iter()
            .find(|e| matches!(e.scope.as_str(), "https" | "all"))
            .and_then(|e| e.endpoint.rsplit_once(':'))
            .and_then(|(h, p)| Some((h.trim_matches(['[', ']']).to_string(), p.parse().ok()?)));
        Self {
            budget: PROBE_TIMEOUT.saturating_sub(MARGIN),
            proxied: proxy.configured(),
            tunnel,
        }
    }
}

// ---- Fehlerarten -----------------------------------------------------------------

/// Deutet der Text eines TLS-Fehlers auf ein nicht vertrautes Zertifikat?
/// OpenSSL, Schannel (Windows) und Security.framework (macOS) formulieren
/// verschieden; wir suchen die Wörter, nicht den Wortlaut.
pub fn is_certificate_error(message: &str) -> bool {
    let m = message.to_lowercase();
    [
        "certificate",
        "zertifikat",
        "cert_",
        "untrusted",
        "not trusted",
        "self signed",
        "self-signed",
        "unable to get local issuer",
        "verify failed",
        "sec_e_untrusted_root",
        "sec_e_cert",
        "crypt_e_",
    ]
    .iter()
    .any(|k| m.contains(k))
}

fn io_failure(e: &io::Error) -> Failure {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Failure::Timeout(Stage::Connect),
        _ => Failure::Connect,
    }
}

// ---- Aussteller aus dem DER-Zertifikat --------------------------------------------

/// Liest ein DER-Element: (Tag, Inhalt, Rest). Nur Längen bis 4 Byte.
fn tlv(data: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = data.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
        (len, &rest[n..])
    };
    if rest.len() < len {
        return None;
    }
    Some((tag, &rest[..len], &rest[len..]))
}

const OID_CN: [u8; 3] = [0x55, 0x04, 0x03];
const OID_O: [u8; 3] = [0x55, 0x04, 0x0a];

fn attribute_text(value_tag: u8, value: &[u8]) -> Option<String> {
    match value_tag {
        // UTF8String, PrintableString, IA5String, TeletexString (hier wie Latin-1 gelesen)
        0x0c | 0x13 | 0x16 | 0x14 => std::str::from_utf8(value)
            .ok()
            .map(str::to_string)
            .or_else(|| Some(value.iter().map(|b| char::from(*b)).collect())),
        // BMPString (UTF-16BE)
        0x1e => {
            let units: Vec<u16> = value
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16(&units).ok()
        }
        _ => None,
    }
}

/// Aussteller (`CN`, sonst `O`; beide, wenn vorhanden) eines X.509-Zertifikats
/// in DER-Kodierung, z. B. `Zscaler Root CA (Zscaler Inc.)`.
pub fn issuer_from_der(der: &[u8]) -> Option<String> {
    let (tag, cert, _) = tlv(der)?;
    if tag != 0x30 {
        return None;
    }
    let (tag, tbs, _) = tlv(cert)?;
    if tag != 0x30 {
        return None;
    }
    let mut rest = tbs;
    // [0] Version (optional)
    let (tag, _, after) = tlv(rest)?;
    if tag == 0xa0 {
        rest = after;
    }
    // Seriennummer, Signaturalgorithmus
    let (_, _, rest) = tlv(rest)?;
    let (_, _, rest) = tlv(rest)?;
    // Aussteller
    let (tag, issuer, _) = tlv(rest)?;
    if tag != 0x30 {
        return None;
    }
    let (mut cn, mut org) = (None, None);
    let mut rdns = issuer;
    while !rdns.is_empty() {
        let (tag, set, next) = tlv(rdns)?;
        rdns = next;
        if tag != 0x31 {
            continue;
        }
        let mut attrs = set;
        while !attrs.is_empty() {
            let (tag, seq, next) = tlv(attrs)?;
            attrs = next;
            if tag != 0x30 {
                continue;
            }
            let (otag, oid, value) = tlv(seq)?;
            if otag != 0x06 {
                continue;
            }
            let (vtag, vbytes, _) = tlv(value)?;
            let text = attribute_text(vtag, vbytes);
            if oid == OID_CN {
                cn = cn.or(text);
            } else if oid == OID_O {
                org = org.or(text);
            }
        }
    }
    let joined = match (cn, org) {
        (Some(cn), Some(org)) if cn != org => format!("{cn} ({org})"),
        (Some(cn), _) => cn,
        (None, Some(org)) => org,
        (None, None) => return None,
    };
    Some(scrub_text(&joined))
}

// ---- TCP und TLS ------------------------------------------------------------------

struct TlsResult {
    issuer: Option<String>,
}

/// TCP-Verbindung (erste erreichbare Adresse) und TLS-Handshake, blockierend,
/// mit Frist. Läuft in `spawn_blocking`.
fn connect_tls(host: &str, addrs: &[SocketAddr], budget: Duration) -> Result<TlsResult, Failure> {
    let deadline = Instant::now() + budget;
    let mut last = Failure::Connect;
    let mut stream = None;
    for addr in addrs.iter().take(4) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Failure::Timeout(Stage::Connect));
        }
        match TcpStream::connect_timeout(addr, left) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = io_failure(&e),
        }
    }
    let stream = stream.ok_or(last)?;
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(Failure::Timeout(Stage::Connect));
    }
    let _ = stream.set_read_timeout(Some(left));
    let _ = stream.set_write_timeout(Some(left));
    let connector = native_tls::TlsConnector::new().map_err(|_| Failure::Tls)?;
    match connector.connect(host, stream) {
        Ok(tls) => {
            let issuer = tls
                .peer_certificate()
                .ok()
                .flatten()
                .and_then(|c| c.to_der().ok())
                .and_then(|der| issuer_from_der(&der));
            Ok(TlsResult { issuer })
        }
        Err(native_tls::HandshakeError::Failure(e)) => {
            let text = format!("{e} {e:?}");
            if is_certificate_error(&text) {
                Err(Failure::TlsCertificate)
            } else if text.to_lowercase().contains("timed out")
                || text.to_lowercase().contains("wouldblock")
            {
                Err(Failure::Timeout(Stage::Connect))
            } else {
                Err(Failure::Tls)
            }
        }
        Err(native_tls::HandshakeError::WouldBlock(_)) => Err(Failure::Timeout(Stage::Connect)),
    }
}

/// Über einen HTTP-Proxy (`CONNECT`) zur Gegenstelle tunneln und das Zertifikat
/// lesen: Nur so sieht die Prüfung, ob der Proxy TLS aufbricht, obwohl sie
/// selbst nicht direkt verbindet. Bestmöglich: jeder Fehler (Anmeldung am
/// Proxy, abgelehnt, Zeitlimit) ergibt `None`, die HTTP-Anfrage danach meldet
/// den eigentlichen Befund. Es werden keine Zugangsdaten gesendet.
fn tunnel_issuer(proxy: &(String, u16), host: &str, port: u16, budget: Duration) -> Option<String> {
    let deadline = Instant::now() + budget;
    let left = || deadline.saturating_duration_since(Instant::now());
    let addr = (proxy.0.as_str(), proxy.1).to_socket_addrs().ok()?.next()?;
    let mut stream =
        TcpStream::connect_timeout(&addr, left().max(Duration::from_millis(1))).ok()?;
    stream
        .set_read_timeout(Some(left().max(Duration::from_millis(1))))
        .ok()?;
    stream
        .set_write_timeout(Some(left().max(Duration::from_millis(1))))
        .ok()?;
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    stream
        .write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .ok()?;
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8 * 1024 || left().is_zero() {
            return None;
        }
        stream.read_exact(&mut byte).ok()?;
        head.push(byte[0]);
    }
    let status_line = head.split(|b| *b == b'\n').next()?;
    let status_line = std::str::from_utf8(status_line).ok()?;
    if status_line.split_whitespace().nth(1) != Some("200") {
        return None;
    }
    let tls = native_tls::TlsConnector::new()
        .ok()?
        .connect(host, stream)
        .ok()?;
    let der = tls.peer_certificate().ok().flatten()?.to_der().ok()?;
    issuer_from_der(&der)
}

// ---- HTTP -------------------------------------------------------------------------

fn http_url(ep: &Endpoint) -> String {
    let scheme = if ep.tls { "https" } else { "http" };
    let default = if ep.tls { 443 } else { 80 };
    let host = if ep.host.contains(':') {
        format!("[{}]", ep.host)
    } else {
        ep.host.clone()
    };
    if ep.port == default {
        format!("{scheme}://{host}/")
    } else {
        format!("{scheme}://{host}:{}/", ep.port)
    }
}

fn classify_reqwest(e: &reqwest::Error, tls: bool) -> Failure {
    if e.is_timeout() {
        return Failure::Timeout(Stage::Http);
    }
    let chain = format!("{e:?}");
    if tls && is_certificate_error(&chain) {
        return Failure::TlsCertificate;
    }
    if e.is_connect() {
        return Failure::Connect;
    }
    Failure::Http
}

#[async_trait]
impl Probe for SystemProbe {
    async fn probe(&self, ep: &Endpoint) -> ProbeOutcome {
        let start = Instant::now();
        let deadline = start + self.budget;
        let left = move || deadline.saturating_duration_since(Instant::now());
        let mut out = ProbeOutcome::default();
        let finish = |mut out: ProbeOutcome, failure: Option<Failure>| {
            out.failure = failure;
            out.total_ms = ms(start.elapsed());
            out
        };

        // 1. Namensauflösung
        let mut addrs: Vec<SocketAddr> = Vec::new();
        if !self.proxied {
            let t = Instant::now();
            let lookup =
                tokio::time::timeout(left(), tokio::net::lookup_host((ep.host.as_str(), ep.port)))
                    .await;
            match lookup {
                Err(_) => return finish(out, Some(Failure::Timeout(Stage::Dns))),
                Ok(Err(_)) => return finish(out, Some(Failure::Dns)),
                Ok(Ok(found)) => addrs = found.collect(),
            }
            if addrs.is_empty() {
                return finish(out, Some(Failure::Dns));
            }
            out.dns_ms = Some(ms(t.elapsed()));
        }

        // 2. TCP und TLS
        if ep.tls && !self.proxied {
            let t = Instant::now();
            let host = ep.host.clone();
            let budget = left();
            let joined = tokio::time::timeout(
                budget,
                tokio::task::spawn_blocking(move || connect_tls(&host, &addrs, budget)),
            )
            .await;
            match joined {
                Err(_) => return finish(out, Some(Failure::Timeout(Stage::Connect))),
                Ok(Err(_)) => return finish(out, Some(Failure::Tls)),
                Ok(Ok(Err(failure))) => return finish(out, Some(failure)),
                Ok(Ok(Ok(tls))) => {
                    out.tls_ms = Some(ms(t.elapsed()));
                    out.issuer = tls.issuer;
                }
            }
        }

        // 2b. Mit Proxy: Zertifikat über einen Tunnel lesen (best effort).
        if ep.tls && self.proxied {
            if let Some(tunnel) = self.tunnel.clone() {
                let t = Instant::now();
                let host = ep.host.clone();
                let port = ep.port;
                let budget = left() / 2;
                let joined = tokio::time::timeout(
                    budget,
                    tokio::task::spawn_blocking(move || {
                        tunnel_issuer(&tunnel, &host, port, budget)
                    }),
                )
                .await;
                if let Ok(Ok(Some(issuer))) = joined {
                    out.tls_ms = Some(ms(t.elapsed()));
                    out.issuer = Some(issuer);
                }
            }
        }

        // 3. HTTP
        if left().is_zero() {
            return finish(out, Some(Failure::Timeout(Stage::Http)));
        }
        let client = match reqwest::Client::builder()
            .timeout(left())
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!(
                "local-voice-ai/",
                env!("CARGO_PKG_VERSION"),
                " netzcheck"
            ))
            .tls_info(true)
            .build()
        {
            Ok(c) => c,
            Err(_) => return finish(out, Some(Failure::Http)),
        };
        let t = Instant::now();
        match client.head(http_url(ep)).send().await {
            Ok(resp) => {
                out.http_status = Some(resp.status().as_u16());
                out.http_ms = Some(ms(t.elapsed()));
                if out.issuer.is_none() {
                    out.issuer = resp
                        .extensions()
                        .get::<reqwest::tls::TlsInfo>()
                        .and_then(|i| i.peer_certificate())
                        .and_then(issuer_from_der);
                }
                finish(out, None)
            }
            Err(e) => finish(out, Some(classify_reqwest(&e, ep.tls))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Selbstsigniertes Testzertifikat: CN=Zscaler Root CA, O=Zscaler Inc. (DER, hex).
    const CERT_HEX: &str = include_str!("testdata/zscaler_root.der.hex");
    /// Selbstsigniertes Testzertifikat nur mit CN=nur-cn.example.
    const CERT_CN_ONLY_HEX: &str = include_str!("testdata/nur_cn.der.hex");

    fn unhex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn aussteller_aus_echtem_der_zertifikat() {
        let der = unhex(CERT_HEX);
        assert_eq!(
            issuer_from_der(&der).as_deref(),
            Some("Zscaler Root CA (Zscaler Inc.)")
        );
        let der = unhex(CERT_CN_ONLY_HEX);
        assert_eq!(issuer_from_der(&der).as_deref(), Some("nur-cn.example"));
    }

    #[test]
    fn kaputtes_der_ergibt_nichts_und_stuerzt_nicht() {
        let der = unhex(CERT_HEX);
        assert_eq!(issuer_from_der(&[]), None);
        assert_eq!(issuer_from_der(&[0x30]), None);
        assert_eq!(issuer_from_der(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]), None);
        for cut in [1, 5, 20, 60, der.len() / 2] {
            // abgeschnitten: kein Absturz, kein Unsinn
            let _ = issuer_from_der(&der[..cut]);
        }
        let mut flipped = der.clone();
        for i in (0..40).step_by(3) {
            flipped[i] ^= 0xff;
            let _ = issuer_from_der(&flipped);
        }
    }

    #[test]
    fn zertifikatsfehler_in_allen_mundarten() {
        for m in [
            "error:0A000086:SSL routines::certificate verify failed",
            "The certificate chain was issued by an authority that is not trusted. (SEC_E_UNTRUSTED_ROOT)",
            "Das Zertifikat ist nicht vertrauenswürdig",
            "self-signed certificate in certificate chain",
            "CERT_E_UNTRUSTEDROOT",
        ] {
            assert!(is_certificate_error(m), "{m}");
        }
        for m in [
            "connection reset by peer",
            "Connection refused",
            "handshake failure",
        ] {
            assert!(!is_certificate_error(m), "{m}");
        }
    }

    #[test]
    fn http_adresse_hat_nur_wurzelpfad() {
        let ep = |host: &str, port, tls| Endpoint {
            host: host.into(),
            port,
            tls,
        };
        assert_eq!(http_url(&ep("a.example", 443, true)), "https://a.example/");
        assert_eq!(
            http_url(&ep("a.example", 8443, true)),
            "https://a.example:8443/"
        );
        assert_eq!(http_url(&ep("a.example", 80, false)), "http://a.example/");
        assert_eq!(
            http_url(&ep("fd00::1", 11434, false)),
            "http://[fd00::1]:11434/"
        );
    }

    /// Kleiner lokaler HTTP-Server (Loopback, kein echtes Netz).
    fn serve_once(status_line: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let n = s.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                // Es geht nur ein HEAD auf "/" ohne Zugangsdaten hinaus.
                assert!(request.starts_with("HEAD / HTTP/1.1"), "{request}");
                assert!(!request.to_lowercase().contains("authorization"));
                let _ = s.write_all(
                    format!("{status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                );
            }
        });
        port
    }

    #[tokio::test]
    async fn echte_probe_gegen_lokalen_server_status_404_ist_erreichbar() {
        let port = serve_once("HTTP/1.1 404 Not Found");
        let probe = SystemProbe::new(&ProxyInfo::default());
        let out = probe
            .probe(&Endpoint {
                host: "127.0.0.1".into(),
                port,
                tls: false,
            })
            .await;
        assert_eq!(out.failure, None, "{out:?}");
        assert_eq!(out.http_status, Some(404));
        assert!(out.dns_ms.is_some());
        assert_eq!(out.tls_ms, None);
        assert_eq!(
            super::super::judge::judge(&out).0,
            super::super::State::Reachable
        );
    }

    #[tokio::test]
    async fn echte_probe_geschlossener_port_ist_nicht_erreichbar() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let probe = SystemProbe::new(&ProxyInfo::default());
        let out = probe
            .probe(&Endpoint {
                host: "127.0.0.1".into(),
                port,
                tls: false,
            })
            .await;
        assert!(matches!(out.failure, Some(Failure::Connect)), "{out:?}");
        assert_eq!(out.http_status, None);
    }

    #[tokio::test]
    async fn echte_probe_unbekannter_name_ist_dns_fehler() {
        // `.invalid` ist nach RFC 2606 nie auflösbar.
        let probe = SystemProbe::new(&ProxyInfo::default());
        let out = probe
            .probe(&Endpoint {
                host: "gibt-es-nicht.invalid".into(),
                port: 443,
                tls: true,
            })
            .await;
        assert!(
            matches!(
                out.failure,
                Some(Failure::Dns) | Some(Failure::Timeout(Stage::Dns))
            ),
            "{out:?}"
        );
    }
}
