use std::time::{Duration, Instant};

use super::*;
use crate::managers::youtube::link::normalize_link;
use crate::managers::youtube::test_support::{json_ok, serve, status};

fn video() -> YoutubeRef {
    normalize_link("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=30&si=TRACK").unwrap()
}

fn opts() -> FetchOpts {
    FetchOpts {
        timeout: Duration::from_secs(5),
        max_bytes: 64 * 1024,
        // Ein gesetzter HTTP_PROXY darf den lokalen Testserver nicht umleiten.
        use_env_proxy: false,
    }
}

const GOOD: &str = r#"{
  "title": "Lastgang verstehen: Spitzen glätten",
  "author_name": "Wolff Applied AI",
  "author_url": "https://www.youtube.com/@wolffappliedai",
  "type": "video",
  "height": 113, "width": 200, "version": "1.0",
  "provider_name": "YouTube", "provider_url": "https://www.youtube.com/",
  "thumbnail_height": 360, "thumbnail_width": 480,
  "thumbnail_url": "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg",
  "html": "<iframe width=\"200\" height=\"113\" src=\"https://www.youtube.com/embed/dQw4w9WgXcQ\"></iframe>"
}"#;

// ---------------------------------------------------------------------------
// Abruf gegen den lokalen Testserver
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_public_video_yields_title_channel_and_thumbnail() {
    let (base, seen) = serve(vec![json_ok(GOOD)]).await;
    let meta = fetch(&base, &video(), &opts()).await.unwrap();
    assert_eq!(meta.title, "Lastgang verstehen: Spitzen glätten");
    assert_eq!(meta.channel, "Wolff Applied AI");
    assert_eq!(
        meta.channel_url.as_deref(),
        Some("https://www.youtube.com/@wolffappliedai")
    );
    assert_eq!(
        meta.thumbnail_url.as_deref(),
        Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg")
    );

    let requests = seen.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "genau ein Abruf");
    let req = &requests[0];
    let line = req.lines().next().unwrap();
    assert!(line.starts_with("GET /oembed?"), "{line}");
    assert!(line.contains("format=json"), "{line}");
    // Der Link im Abruf ist der bereinigte: ohne Zeit und Verfolgungsparameter.
    assert!(line.contains("v%3DdQw4w9WgXcQ"), "{line}");
    assert!(!line.contains("TRACK") && !line.contains("si%3D"), "{line}");
    let lower = req.to_ascii_lowercase();
    assert!(lower.contains("user-agent: localvoiceai/"), "{req}");
    assert!(!lower.contains("cookie:"), "kein Cookie: {req}");
    assert!(!lower.contains("authorization:"), "kein Schluessel: {req}");
}

#[tokio::test]
async fn an_unavailable_video_is_reported_as_such() {
    for (code, reason) in [
        (404, "Not Found"),
        (401, "Unauthorized"),
        (403, "Forbidden"),
    ] {
        let (base, _) = serve(vec![status(code, reason)]).await;
        let err = fetch(&base, &video(), &opts()).await.unwrap_err();
        assert_eq!(err, YoutubeError::Unavailable, "{code}");
        assert_eq!(err.code(), "youtube_unavailable");
    }
}

#[tokio::test]
async fn too_many_requests_and_server_errors_are_told_apart() {
    let (base, _) = serve(vec![status(429, "Too Many Requests")]).await;
    assert_eq!(
        fetch(&base, &video(), &opts()).await.unwrap_err(),
        YoutubeError::RateLimited
    );
    let (base, _) = serve(vec![status(500, "Internal Server Error")]).await;
    assert_eq!(
        fetch(&base, &video(), &opts()).await.unwrap_err(),
        YoutubeError::Http(500)
    );
}

#[tokio::test]
async fn a_redirect_is_not_followed() {
    let (target, target_seen) = serve(vec![json_ok(GOOD)]).await;
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    let (base, _) = serve(vec![redirect]).await;
    let err = fetch(&base, &video(), &opts()).await.unwrap_err();
    assert!(matches!(err, YoutubeError::BadResponse(_)), "{err:?}");
    assert!(
        target_seen.lock().unwrap().is_empty(),
        "das Umleitungsziel darf nie angefragt werden"
    );
}

#[tokio::test]
async fn a_silent_server_runs_into_the_time_limit() {
    let (base, _) = serve(vec![Vec::new()]).await;
    let short = FetchOpts {
        timeout: Duration::from_millis(300),
        ..opts()
    };
    let started = Instant::now();
    let err = fetch(&base, &video(), &short).await.unwrap_err();
    assert_eq!(err, YoutubeError::Timeout);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn no_server_at_all_is_a_network_error_without_the_address() {
    // Einen freien Port suchen und schliessen: dort lauscht niemand.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let base = format!("http://127.0.0.1:{port}/oembed");
    let err = fetch(&base, &video(), &opts()).await.unwrap_err();
    assert!(matches!(err, YoutubeError::Network(_)), "{err:?}");
    let text = err.to_string();
    assert!(
        !text.contains("127.0.0.1"),
        "keine Adresse in der Meldung: {text}"
    );
    assert!(
        !text.contains("dQw4w9WgXcQ"),
        "keine Video-ID in der Meldung: {text}"
    );
}

#[tokio::test]
async fn an_oversized_answer_is_refused_before_it_fills_memory() {
    // Mit Content-Length ...
    let big = "x".repeat(200 * 1024);
    let (base, _) = serve(vec![json_ok(&big)]).await;
    let err = fetch(&base, &video(), &opts()).await.unwrap_err();
    assert!(matches!(err, YoutubeError::BadResponse(_)), "{err:?}");
    // ... und ohne (Ende durch Schliessen der Verbindung).
    let mut chunked = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    chunked.extend_from_slice(big.as_bytes());
    let (base, _) = serve(vec![chunked]).await;
    let err = fetch(&base, &video(), &opts()).await.unwrap_err();
    assert!(matches!(err, YoutubeError::BadResponse(_)), "{err:?}");
}

#[tokio::test]
async fn garbage_instead_of_json_is_a_bad_response() {
    let (base, _) = serve(vec![json_ok("<html>Bitte anmelden</html>")]).await;
    let err = fetch(&base, &video(), &opts()).await.unwrap_err();
    assert!(matches!(err, YoutubeError::BadResponse(_)), "{err:?}");
}

#[tokio::test]
async fn the_request_url_carries_the_clean_link_percent_encoded() {
    let url = request_url("https://www.youtube.com/oembed", &video()).unwrap();
    let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(
        pairs.get("url").map(String::as_str),
        Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
    );
    assert_eq!(pairs.get("format").map(String::as_str), Some("json"));
    assert_eq!(pairs.len(), 2);
    assert!(request_url("kein url", &video()).is_err());
}

// ---------------------------------------------------------------------------
// Antwort auswerten (rein)
// ---------------------------------------------------------------------------

#[test]
fn the_title_is_cleaned_and_clipped() {
    let body = serde_json::json!({
        "title": format!("  Zeile eins\nZeile\tzwei\u{7}{}  ", "ä".repeat(1000)),
        "author_name": "Kanal\r\nName",
        "type": "video",
    })
    .to_string();
    let meta = parse_response(&body).unwrap();
    assert!(
        meta.title.starts_with("Zeile eins Zeile zwei"),
        "{}",
        meta.title
    );
    assert!(!meta.title.chars().any(char::is_control));
    assert!(meta.title.chars().count() <= MAX_TEXT_CHARS);
    assert_eq!(meta.channel, "Kanal Name");
}

#[test]
fn a_missing_channel_is_allowed_a_missing_title_is_not() {
    let meta = parse_response(r#"{"title":"Nur Titel","type":"video"}"#).unwrap();
    assert_eq!(meta.channel, "");
    assert_eq!(meta.channel_url, None);
    assert_eq!(meta.thumbnail_url, None);
    for bad in [
        r#"{"author_name":"x"}"#,
        r#"{"title":"","author_name":"x"}"#,
        r#"{"title":"   \n ","author_name":"x"}"#,
        r#"{"title":42}"#,
        r#"[]"#,
        r#""text""#,
        "",
        "{",
    ] {
        assert!(
            matches!(parse_response(bad), Err(YoutubeError::BadResponse(_))),
            "{bad}"
        );
    }
}

#[test]
fn only_video_answers_are_accepted() {
    assert!(parse_response(r#"{"title":"t","type":"video"}"#).is_ok());
    assert!(parse_response(r#"{"title":"t"}"#).is_ok());
    for kind in ["rich", "photo", "link"] {
        let body = format!(r#"{{"title":"t","type":"{kind}"}}"#);
        assert!(
            matches!(parse_response(&body), Err(YoutubeError::BadResponse(_))),
            "{kind}"
        );
    }
}

#[test]
fn addresses_in_the_answer_must_point_to_youtube() {
    let with = |channel: &str, thumb: &str| {
        serde_json::json!({
            "title": "t", "type": "video",
            "author_url": channel, "thumbnail_url": thumb,
        })
        .to_string()
    };
    // Gute Adressen bleiben.
    let m = parse_response(&with(
        "https://www.youtube.com/channel/UCabc",
        "https://i.ytimg.com/vi/x/hq.jpg",
    ))
    .unwrap();
    assert!(m.channel_url.is_some() && m.thumbnail_url.is_some());
    // Fremde, unsichere oder verschleierte Adressen fallen weg.
    for bad in [
        "javascript:alert(1)",
        "data:text/html,<b>x</b>",
        "http://www.youtube.com/@x",
        "https://evil.example/@x",
        "https://www.youtube.com.evil.example/@x",
        "https://user:pw@www.youtube.com/@x",
        "https://i.ytimg.com.evil.example/vi/x/hq.jpg",
        "//evil.example/x.jpg",
        "",
    ] {
        let m = parse_response(&with(bad, bad)).unwrap();
        assert_eq!(m.channel_url, None, "{bad}");
        assert_eq!(m.thumbnail_url, None, "{bad}");
    }
}

// ---------------------------------------------------------------------------
// Prueffall: nur in der Sandbox darf die Adresse umgebogen werden
// ---------------------------------------------------------------------------

#[test]
fn the_endpoint_can_only_be_redirected_inside_a_sandbox() {
    let local = "http://127.0.0.1:9/oembed";
    // Ohne Sandbox gilt immer die echte Adresse.
    assert_eq!(oembed_base_from(Some(local), None), OEMBED_URL);
    assert_eq!(oembed_base_from(Some(local), Some("")), OEMBED_URL);
    assert_eq!(oembed_base_from(Some(local), Some("   ")), OEMBED_URL);
    assert_eq!(oembed_base_from(None, Some("C:/sandbox")), OEMBED_URL);
    // In der Sandbox gilt die Vorgabe, wenn sie eine http(s)-Adresse ist.
    assert_eq!(oembed_base_from(Some(local), Some("C:/sandbox")), local);
    assert_eq!(
        oembed_base_from(Some("https://stub.example/oembed"), Some("C:/sandbox")),
        "https://stub.example/oembed"
    );
    for bad in ["file:///x", "ftp://x/y", "javascript:1", "kein url", ""] {
        assert_eq!(
            oembed_base_from(Some(bad), Some("C:/sandbox")),
            OEMBED_URL,
            "{bad}"
        );
    }
}
