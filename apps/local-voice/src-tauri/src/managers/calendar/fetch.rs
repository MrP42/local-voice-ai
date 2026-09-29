//! ICS-Abruf ueber HTTP(S) (M5, `entwurf/m5-m6-kalender-export.md` §3 F15, §7).
//!
//! Grenzen: 30 s Gesamtzeit, hoechstens 20 MB (schon beim Empfang abgebrochen,
//! nicht erst nach dem Download), hoechstens 5 Umleitungen, nur `http(s)` und
//! `webcal(s)`, Antwort muss mit `BEGIN:VCALENDAR` beginnen. `If-None-Match` und
//! `If-Modified-Since` sparen den Abruf, wenn sich nichts geaendert hat.
//!
//! Die Adresse ist ein Geheimnis (Lesezugriff auf den Kalender): sie steht in
//! keiner Fehlermeldung und in keinem Log. `reqwest` haengt sie an seine Fehler,
//! deshalb wird sie dort entfernt (`without_url`).
//!
//! Proxy: `reqwest` nutzt die Umgebungsvariablen (`HTTPS_PROXY`, `HTTP_PROXY`,
//! `NO_PROXY`), nicht den Windows-Systemproxy; die Fehlermeldung sagt es.

use std::time::Duration;

use reqwest::header::{ACCEPT, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::redirect::Policy;
use reqwest::StatusCode;

use super::ics::looks_like_calendar;
pub use super::model::CalendarError;

/// Hoechstgroesse einer Kalenderdatei.
pub const MAX_BODY_BYTES: usize = 20 * 1024 * 1024;
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchOutcome {
    NotModified,
    Body {
        text: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

/// Grenzen eines Abrufs; die Vorgabe sind die Werte oben. Tests setzen kleine.
#[derive(Clone, Debug)]
pub struct FetchOpts {
    pub timeout: Duration,
    pub max_bytes: usize,
    pub max_redirects: usize,
    /// Proxy-Umgebungsvariablen beachten (in Tests aus, damit ein gesetzter
    /// `HTTP_PROXY` den lokalen Testserver nicht umleitet).
    pub use_env_proxy: bool,
}

impl Default for FetchOpts {
    fn default() -> Self {
        Self {
            timeout: FETCH_TIMEOUT,
            max_bytes: MAX_BODY_BYTES,
            max_redirects: MAX_REDIRECTS,
            use_env_proxy: true,
        }
    }
}

/// `webcal://` und `webcals://` werden zu `https://`; `http(s)` bleibt; alles
/// andere (`file:`, `ftp:`, ...) ist kein Kalenderabruf.
pub fn normalize_url(url: &str) -> Result<String, CalendarError> {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    let rewritten = if lower.starts_with("webcals://") {
        format!("https://{}", &u["webcals://".len()..])
    } else if lower.starts_with("webcal://") {
        format!("https://{}", &u["webcal://".len()..])
    } else if lower.starts_with("https://") || lower.starts_with("http://") {
        u.to_string()
    } else {
        return Err(CalendarError::Network(
            "Nur Adressen mit https://, http:// oder webcal:// werden unterstützt.".to_string(),
        ));
    };
    match url::Url::parse(&rewritten) {
        Ok(parsed) if parsed.host_str().is_some() => Ok(rewritten),
        _ => Err(CalendarError::Network(
            "Die Adresse ist keine gültige Internetadresse.".to_string(),
        )),
    }
}

/// Der Host der Adresse fuer die Anzeige (`account_hint`); nie der Pfad, nie
/// Benutzerangaben, nie die Abfrage.
pub fn host_hint(url: &str) -> Option<String> {
    let n = normalize_url(url).ok()?;
    url::Url::parse(&n)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
}

fn map_reqwest_error(e: reqwest::Error, max_redirects: usize) -> CalendarError {
    let e = e.without_url();
    if e.is_timeout() {
        return CalendarError::Timeout;
    }
    if e.is_redirect() {
        return CalendarError::Network(format!("Zu viele Umleitungen (mehr als {max_redirects})."));
    }
    let hint = if e.is_connect() {
        " (Der Windows-Systemproxy wird nicht verwendet, nur HTTPS_PROXY und HTTP_PROXY.)"
    } else {
        ""
    };
    CalendarError::Network(format!("{e}{hint}"))
}

/// Ruft die Kalenderdatei mit den Standardgrenzen ab.
pub async fn fetch_ics(
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<FetchOutcome, CalendarError> {
    fetch_ics_with(url, etag, last_modified, &FetchOpts::default()).await
}

pub async fn fetch_ics_with(
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    opts: &FetchOpts,
) -> Result<FetchOutcome, CalendarError> {
    let url = normalize_url(url)?;
    let mut builder = reqwest::Client::builder()
        .timeout(opts.timeout)
        .redirect(Policy::limited(opts.max_redirects))
        .user_agent(concat!(
            "LocalVoiceAI/",
            env!("CARGO_PKG_VERSION"),
            " (calendar)"
        ));
    if !opts.use_env_proxy {
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|e| CalendarError::Network(e.without_url().to_string()))?;

    let mut req = client
        .get(&url)
        .header(ACCEPT, "text/calendar, text/plain;q=0.8, */*;q=0.5");
    if let Some(etag) = etag.filter(|e| !e.is_empty()) {
        req = req.header(IF_NONE_MATCH, etag);
    }
    if let Some(lm) = last_modified.filter(|l| !l.is_empty()) {
        req = req.header(IF_MODIFIED_SINCE, lm);
    }
    let mut resp = req
        .send()
        .await
        .map_err(|e| map_reqwest_error(e, opts.max_redirects))?;

    let status = resp.status();
    if status == StatusCode::NOT_MODIFIED {
        return Ok(FetchOutcome::NotModified);
    }
    if !status.is_success() {
        return Err(CalendarError::Http(status.as_u16()));
    }
    if resp
        .content_length()
        .is_some_and(|len| len > opts.max_bytes as u64)
    {
        return Err(CalendarError::TooLarge);
    }
    let header_string = |name| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let new_etag = header_string(ETAG);
    let new_last_modified = header_string(LAST_MODIFIED);

    // Stueckweise lesen und beim Ueberschreiten der Grenze abbrechen: ein Server
    // ohne Content-Length (chunked) darf den Speicher nicht fuellen.
    let mut body: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > opts.max_bytes {
                    return Err(CalendarError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => return Err(map_reqwest_error(e, opts.max_redirects)),
        }
    }
    let text = match String::from_utf8(body) {
        Ok(t) => t,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    if !looks_like_calendar(&text) {
        return Err(CalendarError::NotCalendar);
    }
    Ok(FetchOutcome::Body {
        text,
        etag: new_etag,
        last_modified: new_last_modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n";

    fn opts() -> FetchOpts {
        FetchOpts {
            use_env_proxy: false,
            ..Default::default()
        }
    }

    /// Kleiner HTTP-Server: beantwortet jede Verbindung mit der naechsten
    /// vorbereiteten Antwort (die letzte wiederholt sich) und merkt sich die
    /// Anfragen.
    async fn serve(responses: Vec<Vec<u8>>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            let mut n = 0usize;
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let resp = responses[n.min(responses.len() - 1)].clone();
                n += 1;
                let seen = seen2.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 1024];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => break,
                            Ok(k) => buf.extend_from_slice(&tmp[..k]),
                        }
                    }
                    seen.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf).into_owned());
                    if resp.is_empty() {
                        // Kein Antwortbyte: der Client muss in seinen Timeout laufen.
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        return;
                    }
                    let _ = sock.write_all(&resp).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (
            format!("http://{addr}/private/secret-token-42/cal.ics"),
            seen,
        )
    }

    fn ok_response(body: &str, extra: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn status_response(line: &str) -> Vec<u8> {
        format!("HTTP/1.1 {line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
    }

    #[test]
    fn webcal_becomes_https_and_other_schemes_are_refused() {
        assert_eq!(
            normalize_url("webcal://cal.example.org/a.ics").unwrap(),
            "https://cal.example.org/a.ics"
        );
        assert_eq!(
            normalize_url("  WEBCALS://cal.example.org/a.ics ").unwrap(),
            "https://cal.example.org/a.ics"
        );
        assert_eq!(
            normalize_url("http://nas.local/cal.ics").unwrap(),
            "http://nas.local/cal.ics"
        );
        for bad in [
            "file:///C:/x.ics",
            "ftp://h/x.ics",
            "javascript:alert(1)",
            "cal.ics",
            "https://",
        ] {
            assert!(
                matches!(normalize_url(bad), Err(CalendarError::Network(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn host_hint_never_carries_path_or_query() {
        assert_eq!(
            host_hint("https://outlook.office365.com/owa/calendar/abc123/reachcalendar.ics?x=1")
                .as_deref(),
            Some("outlook.office365.com")
        );
        assert_eq!(
            host_hint("webcal://p01-caldav.icloud.com/published/2/TOKEN").as_deref(),
            Some("p01-caldav.icloud.com")
        );
        assert_eq!(host_hint("kein url"), None);
    }

    #[tokio::test]
    async fn ok_response_returns_body_and_validators() {
        let (url, _) = serve(vec![ok_response(
            ICS,
            "ETag: \"abc\"\r\nLast-Modified: Tue, 29 Sep 2026 10:00:00 GMT\r\n",
        )])
        .await;
        match fetch_ics_with(&url, None, None, &opts()).await.unwrap() {
            FetchOutcome::Body {
                text,
                etag,
                last_modified,
            } => {
                assert_eq!(text, ICS);
                assert_eq!(etag.as_deref(), Some("\"abc\""));
                assert_eq!(
                    last_modified.as_deref(),
                    Some("Tue, 29 Sep 2026 10:00:00 GMT")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn validators_are_sent_and_304_means_not_modified() {
        let (url, seen) = serve(vec![status_response("304 Not Modified")]).await;
        let out = fetch_ics_with(
            &url,
            Some("\"abc\""),
            Some("Tue, 29 Sep 2026 10:00:00 GMT"),
            &opts(),
        )
        .await
        .unwrap();
        assert_eq!(out, FetchOutcome::NotModified);
        let req = seen.lock().unwrap()[0].to_ascii_lowercase();
        assert!(req.contains("if-none-match: \"abc\""), "{req}");
        assert!(
            req.contains("if-modified-since: tue, 29 sep 2026 10:00:00 gmt"),
            "{req}"
        );
    }

    #[tokio::test]
    async fn http_errors_carry_the_status_code() {
        for (line, code) in [
            ("401 Unauthorized", 401u16),
            ("403 Forbidden", 403),
            ("404 Not Found", 404),
            ("410 Gone", 410),
            ("500 Internal Server Error", 500),
        ] {
            let (url, _) = serve(vec![status_response(line)]).await;
            assert_eq!(
                fetch_ics_with(&url, None, None, &opts()).await,
                Err(CalendarError::Http(code)),
                "{line}"
            );
        }
        // Der Klartext nennt die Ursache, nicht nur die Zahl.
        assert!(CalendarError::Http(410)
            .to_string()
            .contains("nicht mehr veröffentlicht"));
        assert!(CalendarError::Http(403).to_string().contains("gesperrt"));
    }

    #[tokio::test]
    async fn html_body_with_status_200_is_not_a_calendar() {
        let (url, _) = serve(vec![ok_response(
            "<!DOCTYPE html><html><body>Bitte anmelden</body></html>",
            "",
        )])
        .await;
        assert_eq!(
            fetch_ics_with(&url, None, None, &opts()).await,
            Err(CalendarError::NotCalendar)
        );
    }

    #[tokio::test]
    async fn body_over_the_limit_is_refused_by_content_length() {
        let (url, _) = serve(vec![ok_response(
            &format!("BEGIN:VCALENDAR\r\n{}", "X".repeat(4000)),
            "",
        )])
        .await;
        let small = FetchOpts {
            max_bytes: 1024,
            ..opts()
        };
        assert_eq!(
            fetch_ics_with(&url, None, None, &small).await,
            Err(CalendarError::TooLarge)
        );
    }

    #[tokio::test]
    async fn chunked_body_over_the_limit_is_cut_off_while_streaming() {
        // Keine Content-Length: nur das stueckweise Zaehlen schuetzt hier.
        let mut resp =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        let chunk = format!("BEGIN:VCALENDAR\r\n{}\r\n", "X".repeat(900));
        for _ in 0..8 {
            resp.extend_from_slice(format!("{:x}\r\n{chunk}\r\n", chunk.len()).as_bytes());
        }
        resp.extend_from_slice(b"0\r\n\r\n");
        let (url, _) = serve(vec![resp]).await;
        let small = FetchOpts {
            max_bytes: 2048,
            ..opts()
        };
        assert_eq!(
            fetch_ics_with(&url, None, None, &small).await,
            Err(CalendarError::TooLarge)
        );
    }

    #[tokio::test]
    async fn a_server_that_never_answers_ends_in_timeout() {
        let (url, _) = serve(vec![Vec::new()]).await;
        let quick = FetchOpts {
            timeout: Duration::from_millis(300),
            ..opts()
        };
        assert_eq!(
            fetch_ics_with(&url, None, None, &quick).await,
            Err(CalendarError::Timeout)
        );
    }

    #[tokio::test]
    async fn redirect_loops_stop_at_the_limit() {
        // Der Server verweist jede Anfrage auf sich selbst.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut tmp = [0u8; 2048];
                    let _ = sock.read(&mut tmp).await;
                    let resp = format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://{addr}/again\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        let out = fetch_ics_with(&format!("http://{addr}/start"), None, None, &opts()).await;
        match out {
            Err(CalendarError::Network(msg)) => assert!(msg.contains("Umleitungen"), "{msg}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_redirect_is_followed_to_the_calendar() {
        let (target, _) = serve(vec![ok_response(ICS, "")]).await;
        let (url, _) = serve(vec![format!(
            "HTTP/1.1 301 Moved Permanently\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()])
        .await;
        assert!(matches!(
            fetch_ics_with(&url, None, None, &opts()).await,
            Ok(FetchOutcome::Body { .. })
        ));
    }

    #[tokio::test]
    async fn errors_never_contain_the_secret_url() {
        // Nichts lauscht auf Port 1: Verbindungsfehler, den reqwest sonst mit der URL meldet.
        let url = "https://127.0.0.1:1/private/secret-token-42/cal.ics?key=SECRET";
        let err = fetch_ics_with(url, None, None, &opts()).await.unwrap_err();
        let text = format!("{err} | {err:?}");
        assert!(!text.contains("secret-token-42"), "{text}");
        assert!(!text.contains("SECRET"), "{text}");
        assert!(
            matches!(err, CalendarError::Network(_) | CalendarError::Timeout),
            "{err:?}"
        );
    }
}
