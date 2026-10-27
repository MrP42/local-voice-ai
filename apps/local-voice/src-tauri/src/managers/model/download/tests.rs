use super::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ── SHA256 verification tests ─────────────────────────────────────────────

/// Helper: write `data` to a temp file and return (TempDir, path).
/// TempDir must be kept alive for the duration of the test.
fn write_temp_file(data: &[u8]) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.partial");
    let mut f = File::create(&path).unwrap();
    f.write_all(data).unwrap();
    (dir, path)
}

#[test]
fn test_verify_sha256_skipped_when_none() {
    // Custom models have no expected hash — verification must be a no-op.
    let (_dir, path) = write_temp_file(b"anything");
    assert!(ModelManager::verify_sha256(&path, None, "custom").is_ok());
    assert!(
        path.exists(),
        "file must be untouched when verification is skipped"
    );
}

#[test]
fn test_verify_sha256_passes_on_correct_hash() {
    // Compute the real hash so the test is self-consistent.
    let (_dir, path) = write_temp_file(b"hello world");
    let actual = ModelManager::compute_sha256(&path).unwrap();
    assert!(
        ModelManager::verify_sha256(&path, Some(&actual), "test_model").is_ok(),
        "should pass when hash matches"
    );
    assert!(
        path.exists(),
        "file must be kept on successful verification"
    );
}

#[test]
fn test_verify_sha256_fails_and_deletes_partial_on_mismatch() {
    let (_dir, path) = write_temp_file(b"this is not the real model");
    let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";

    let result = ModelManager::verify_sha256(&path, Some(wrong_hash), "bad_model");

    assert!(result.is_err(), "mismatch must return an error");
    assert!(
        result.unwrap_err().to_string().contains("corrupt"),
        "error message should mention corruption"
    );
    assert!(
        !path.exists(),
        "partial file must be deleted after hash mismatch"
    );
}

#[test]
fn test_verify_sha256_fails_and_deletes_partial_when_file_missing() {
    // Simulate a partial file that was already removed (e.g. disk full mid-download).
    let dir = TempDir::new().unwrap();
    let missing_path = dir.path().join("gone.partial");
    // Don't create the file — it should not exist.

    let result =
        ModelManager::verify_sha256(&missing_path, Some("anyexpectedhash"), "missing_model");

    assert!(result.is_err(), "missing file must return an error");
}

// ── Resumable HTTP downloader tests ───────────────────────────────────────
//
// Each test drives `download_http_resumable_with_events` against a scripted
// single-connection server on a local socket — no Tauri, no real network.
// Success means verified bytes are left in the partial (finalizing is the
// caller's job), so "no Completed on a bad hash" is the rename gate too.

fn sha_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn http_response(status_line: &str, headers: &[String], body: &[u8]) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {}\r\nConnection: close\r\n", status_line);
    for h in headers {
        head.push_str(h);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

async fn read_request_head(sock: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if sock.read_exact(&mut byte).await.is_err() {
            break;
        }
        buf.push(byte[0]);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Serve exactly one connection: capture the request head, write
/// `response` verbatim, close. Returns the URL to fetch and a handle
/// yielding the captured request head (lowercased for header asserts).
async fn serve_once(response: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let head = read_request_head(&mut sock).await;
        // The client may hang up early (error tests); that's not a
        // server-side failure.
        let _ = sock.write_all(&response).await;
        let _ = sock.shutdown().await;
        head.to_lowercase()
    });
    (format!("http://{}/file", addr), handle)
}

async fn run_download(
    url: &str,
    partial: &Path,
    expected_size: Option<u64>,
    expected_sha256: Option<&str>,
    cancel: &CancellationToken,
) -> Result<HttpDownloadOutcome> {
    ModelManager::download_http_resumable_with_events(
        "test/model",
        url,
        partial,
        expected_size,
        expected_sha256,
        cancel,
        &|_| {},
    )
    .await
}

#[tokio::test]
async fn http_fresh_download_completes_and_verifies() {
    let body = b"hello world";
    let (url, server) = serve_once(http_response(
        "200 OK",
        &[format!("Content-Length: {}", body.len())],
        body,
    ))
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");

    let out = run_download(
        &url,
        &partial,
        Some(body.len() as u64),
        Some(&sha_hex(body)),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    assert!(matches!(out, HttpDownloadOutcome::Completed));
    assert_eq!(fs::read(&partial).unwrap(), body);
    let head = server.await.unwrap();
    assert!(
        !head.contains("range:"),
        "fresh download must not send a Range header"
    );
}

#[tokio::test]
async fn http_resume_appends_with_valid_content_range() {
    let full = b"helloworld";
    let (url, server) = serve_once(http_response(
        "206 Partial Content",
        &[
            "Content-Range: bytes 5-9/10".to_string(),
            "Content-Length: 5".to_string(),
        ],
        &full[5..],
    ))
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    fs::write(&partial, &full[..5]).unwrap();

    let out = run_download(
        &url,
        &partial,
        Some(10),
        Some(&sha_hex(full)),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    assert!(matches!(out, HttpDownloadOutcome::Completed));
    assert_eq!(fs::read(&partial).unwrap(), full);
    let head = server.await.unwrap();
    assert!(head.contains("range: bytes=5-"), "got request: {head}");
}

#[tokio::test]
async fn http_wrong_content_range_discards_partial() {
    // Server answers 206 but restarts the range at 0: appending would
    // corrupt, so the partial must be dropped and the call must fail.
    let full = b"helloworld";
    let (url, _server) = serve_once(http_response(
        "206 Partial Content",
        &[
            "Content-Range: bytes 0-9/10".to_string(),
            "Content-Length: 10".to_string(),
        ],
        full,
    ))
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    fs::write(&partial, &full[..5]).unwrap();

    let err = run_download(
        &url,
        &partial,
        Some(10),
        Some(&sha_hex(full)),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("Content-Range"), "{err}");
    assert!(!partial.exists(), "corrupt-prone partial must be deleted");
}

#[tokio::test]
async fn http_range_ignored_200_restarts_from_zero() {
    let body = b"helloworld!";
    let (url, _server) = serve_once(http_response(
        "200 OK",
        &[format!("Content-Length: {}", body.len())],
        body,
    ))
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    fs::write(&partial, b"XXXXX").unwrap(); // stale bytes that must not survive

    let out = run_download(
        &url,
        &partial,
        Some(body.len() as u64),
        Some(&sha_hex(body)),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    assert!(matches!(out, HttpDownloadOutcome::Completed));
    assert_eq!(fs::read(&partial).unwrap(), body);
}

#[tokio::test]
async fn http_416_with_hash_finalizes_complete_partial() {
    // URL-model shape: no catalog size, but a hash — the one case where a
    // 416 may bless the partial, because verification proves it.
    let body = b"the whole file";
    let (url, _server) = serve_once(http_response("416 Range Not Satisfiable", &[], b"")).await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    fs::write(&partial, body).unwrap();

    let out = run_download(
        &url,
        &partial,
        None,
        Some(&sha_hex(body)),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    assert!(matches!(out, HttpDownloadOutcome::Completed));
    assert_eq!(fs::read(&partial).unwrap(), body);
}

#[tokio::test]
async fn http_416_with_wrong_hash_clears_partial() {
    let (url, _server) = serve_once(http_response("416 Range Not Satisfiable", &[], b"")).await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    fs::write(&partial, b"corrupted bytes").unwrap();

    let err = run_download(
        &url,
        &partial,
        None,
        Some(&sha_hex(b"the real file")),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("corrupt"), "{err}");
    assert!(
        !partial.exists(),
        "failed verification must delete the partial"
    );
}

#[tokio::test]
async fn http_416_short_of_expected_size_clears_partial() {
    // Mirror shape: catalog size known, partial shorter, yet the server
    // says our offset is past EOF — its object is smaller than the catalog
    // expects. Never blessed, even with a hash on hand.
    let (url, _server) = serve_once(http_response("416 Range Not Satisfiable", &[], b"")).await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    fs::write(&partial, b"12345").unwrap();

    let err = run_download(
        &url,
        &partial,
        Some(10),
        Some(&sha_hex(b"1234567890")),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("416"), "{err}");
    assert!(!partial.exists());
}

#[tokio::test]
async fn http_416_without_trust_signals_clears_partial() {
    // No size, no hash: nothing can prove the partial complete, so a 416
    // must restart clean instead of accepting unverified bytes.
    let (url, _server) = serve_once(http_response("416 Range Not Satisfiable", &[], b"")).await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    fs::write(&partial, b"whatever").unwrap();

    let err = run_download(&url, &partial, None, None, &CancellationToken::new())
        .await
        .unwrap_err();

    assert!(err.to_string().contains("416"), "{err}");
    assert!(!partial.exists());
}

#[tokio::test]
async fn http_full_size_partial_verified_without_network() {
    // Crash-between-completion-and-finalize recovery: the partial already
    // has every byte, so no request is issued at all (the URL points at a
    // dead port to prove it).
    let body = b"complete content";
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    fs::write(&partial, body).unwrap();

    let out = run_download(
        "http://127.0.0.1:9/unreachable",
        &partial,
        Some(body.len() as u64),
        Some(&sha_hex(body)),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    assert!(matches!(out, HttpDownloadOutcome::Completed));
    assert_eq!(fs::read(&partial).unwrap(), body);
}

#[tokio::test]
async fn http_oversized_partial_restarts_clean() {
    let body = b"12345";
    let (url, server) = serve_once(http_response(
        "200 OK",
        &[format!("Content-Length: {}", body.len())],
        body,
    ))
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    fs::write(&partial, b"garbage-beyond-expected").unwrap();

    let out = run_download(
        &url,
        &partial,
        Some(body.len() as u64),
        Some(&sha_hex(body)),
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    assert!(matches!(out, HttpDownloadOutcome::Completed));
    assert_eq!(fs::read(&partial).unwrap(), body);
    let head = server.await.unwrap();
    assert!(
        !head.contains("range:"),
        "oversized partial must restart, not resume: {head}"
    );
}

#[tokio::test]
async fn http_conflicting_content_length_rejected_before_writing() {
    let (url, _server) = serve_once(http_response(
        "200 OK",
        &["Content-Length: 10".to_string()],
        b"0123456789",
    ))
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");

    let err = run_download(
        &url,
        &partial,
        Some(5),
        Some(&sha_hex(b"01234")),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("advertises"), "{err}");
    assert!(
        !partial.exists(),
        "nothing may be written on an up-front size conflict"
    );
}

#[tokio::test]
async fn http_body_exceeding_expected_size_aborts_and_clears() {
    // Chunked response (no Content-Length to reject up-front) that keeps
    // sending past the expected size: the disk-fill guard must cut the
    // transfer at the first excess byte and clear the tainted partial.
    let (url, _server) = serve_once(http_response(
        "200 OK",
        &["Transfer-Encoding: chunked".to_string()],
        b"3\r\nabc\r\n5\r\ndefgh\r\n0\r\n\r\n",
    ))
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");

    let err = run_download(
        &url,
        &partial,
        Some(5),
        Some(&sha_hex(b"abcde")),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("more than"), "{err}");
    assert!(!partial.exists(), "tainted partial must be deleted");
}

#[tokio::test]
async fn http_wrong_hash_after_download_clears_partial() {
    let body = b"tampered bytes";
    let (url, _server) = serve_once(http_response(
        "200 OK",
        &[format!("Content-Length: {}", body.len())],
        body,
    ))
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");

    let err = run_download(
        &url,
        &partial,
        Some(body.len() as u64),
        Some(&sha_hex(b"the expected bytes")),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("corrupt"), "{err}");
    assert!(!partial.exists());
}

#[tokio::test]
async fn http_cancel_while_awaiting_headers() {
    // Server accepts and goes silent. Cancellation must win immediately —
    // not after the stall timeout, and certainly not never.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let _ = read_request_head(&mut sock).await;
        tokio::time::sleep(Duration::from_secs(600)).await;
    });
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");

    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });

    let started = Instant::now();
    let out = run_download(
        &format!("http://{}/file", addr),
        &partial,
        Some(10),
        Some("00"),
        &cancel,
    )
    .await
    .unwrap();

    assert!(matches!(out, HttpDownloadOutcome::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "cancel must not wait out the stall timeout"
    );
}

#[tokio::test]
async fn http_cancel_during_stalled_body_keeps_partial() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let _ = read_request_head(&mut sock).await;
        let _ = sock
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nabc")
            .await;
        tokio::time::sleep(Duration::from_secs(600)).await;
    });
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");

    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.cancel();
    });

    let out = run_download(
        &format!("http://{}/file", addr),
        &partial,
        Some(100),
        Some("00"),
        &cancel,
    )
    .await
    .unwrap();

    assert!(matches!(out, HttpDownloadOutcome::Cancelled));
    assert_eq!(
        fs::read(&partial).unwrap(),
        b"abc",
        "bytes received before the cancel must be kept for resume"
    );
}

// ── Zugang (Token) fuer gated Downloads ───────────────────────────────────
//
// Das Token darf nur an EINEN Ursprung gehen und bei Weiterleitungen nie
// mitgenommen werden (Hugging Face leitet auf ein CDN eines anderen Hosts um).
// Fake-Server auf Loopback: der "Ursprung" ist dort 127.0.0.1 samt Port, ein
// anderer Port ist also ein anderer Ursprung.

use super::auth::{DownloadAuth, HttpStatusError, Origin, Secret};

const TOKEN: &str = "hf_GeheimesTestToken_0123456789abcdef";

fn auth_for_port(port: u16) -> DownloadAuth {
    DownloadAuth::new(
        Secret::new(TOKEN).unwrap(),
        Origin::plain_http("127.0.0.1", port),
    )
}

fn port_of(url: &str) -> u16 {
    url.split("127.0.0.1:")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .and_then(|p| p.parse().ok())
        .expect("Port in der Adresse")
}

/// Mehrere Verbindungen nacheinander bedienen (eine Antwort je Verbindung).
/// Liefert die Adresse (`http://127.0.0.1:PORT`) und die mitgeschriebenen Anfragekoepfe.
async fn serve_script(responses: Vec<Vec<u8>>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let mut heads = Vec::new();
        for response in responses {
            let (mut sock, _) = listener.accept().await.unwrap();
            heads.push(read_request_head(&mut sock).await.to_lowercase());
            let _ = sock.write_all(&response).await;
            let _ = sock.shutdown().await;
        }
        heads
    });
    (base, handle)
}

async fn run_authed(
    url: &str,
    partial: &Path,
    expected_size: Option<u64>,
    expected_sha256: Option<&str>,
    auth: Option<&DownloadAuth>,
) -> Result<HttpDownloadOutcome> {
    ModelManager::download_http_resumable_authed(
        "test/model",
        url,
        partial,
        expected_size,
        expected_sha256,
        &CancellationToken::new(),
        &|_| {},
        auth,
    )
    .await
}

fn has_token(head: &str) -> bool {
    head.contains(&TOKEN.to_lowercase())
}

#[tokio::test]
async fn das_token_geht_an_den_freigegebenen_ursprung() {
    let body = b"gesperrte Gewichte";
    let (base, server) = serve_script(vec![http_response(
        "200 OK",
        &[format!("Content-Length: {}", body.len())],
        body,
    )])
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    let auth = auth_for_port(port_of(&base));

    let out = run_authed(
        &format!("{base}/file"),
        &partial,
        Some(body.len() as u64),
        Some(&sha_hex(body)),
        Some(&auth),
    )
    .await
    .unwrap();

    assert!(matches!(out, HttpDownloadOutcome::Completed));
    assert_eq!(fs::read(&partial).unwrap(), body);
    let heads = server.await.unwrap();
    assert!(
        heads[0].contains("authorization: bearer "),
        "Bearer-Kopf fehlt: {}",
        heads[0]
    );
    assert!(has_token(&heads[0]));
}

#[tokio::test]
async fn das_token_geht_nicht_an_einen_anderen_ursprung() {
    let body = b"x";
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    // Einmal ist ein ANDERER Port desselben Hosts freigegeben, einmal die echte
    // Freigabe (https://huggingface.co): beide passen nicht auf den Fake-Server.
    for real_origin in [false, true] {
        let (base, server) = serve_script(vec![http_response(
            "200 OK",
            &["Content-Length: 1".to_string()],
            body,
        )])
        .await;
        let auth = if real_origin {
            DownloadAuth::huggingface(Secret::new(TOKEN).unwrap())
        } else {
            auth_for_port(port_of(&base).wrapping_add(1))
        };
        assert!(!auth.applies_to(&reqwest::Url::parse(&base).unwrap()));
        let _ = fs::remove_file(&partial);
        run_authed(
            &format!("{base}/file"),
            &partial,
            Some(1),
            None,
            Some(&auth),
        )
        .await
        .unwrap();
        let heads = server.await.unwrap();
        assert!(!heads[0].contains("authorization"), "{}", heads[0]);
        assert!(!has_token(&heads[0]));
    }
}

#[tokio::test]
async fn eine_weiterleitung_zu_einem_anderen_host_nimmt_das_token_nicht_mit() {
    // Das "CDN": anderer Port = anderer Ursprung.
    let body = b"Gewichte vom CDN";
    let (cdn, cdn_server) = serve_script(vec![http_response(
        "200 OK",
        &[format!("Content-Length: {}", body.len())],
        body,
    )])
    .await;
    let (origin, origin_server) = serve_script(vec![http_response(
        "302 Found",
        &[
            format!("Location: {cdn}/signed/file?sig=abc"),
            "Content-Length: 0".to_string(),
        ],
        b"",
    )])
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    let auth = auth_for_port(port_of(&origin));

    let out = run_authed(
        &format!("{origin}/resolve/file"),
        &partial,
        Some(body.len() as u64),
        Some(&sha_hex(body)),
        Some(&auth),
    )
    .await
    .unwrap();

    assert!(matches!(out, HttpDownloadOutcome::Completed));
    assert_eq!(fs::read(&partial).unwrap(), body);
    let origin_heads = origin_server.await.unwrap();
    let cdn_heads = cdn_server.await.unwrap();
    assert!(
        has_token(&origin_heads[0]),
        "der Ursprung bekommt das Token"
    );
    assert!(
        !cdn_heads[0].contains("authorization") && !has_token(&cdn_heads[0]),
        "das CDN darf das Token nie sehen: {}",
        cdn_heads[0]
    );
}

#[tokio::test]
async fn beim_fortsetzen_bleibt_der_range_kopf_ueber_die_weiterleitung_erhalten() {
    let full = b"helloworld";
    let (cdn, cdn_server) = serve_script(vec![http_response(
        "206 Partial Content",
        &[
            "Content-Range: bytes 5-9/10".to_string(),
            "Content-Length: 5".to_string(),
        ],
        &full[5..],
    )])
    .await;
    let (origin, origin_server) = serve_script(vec![http_response(
        "307 Temporary Redirect",
        &[
            format!("Location: {cdn}/f"),
            "Content-Length: 0".to_string(),
        ],
        b"",
    )])
    .await;
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    fs::write(&partial, &full[..5]).unwrap();
    let auth = auth_for_port(port_of(&origin));

    run_authed(
        &format!("{origin}/f"),
        &partial,
        Some(10),
        Some(&sha_hex(full)),
        Some(&auth),
    )
    .await
    .unwrap();

    assert_eq!(fs::read(&partial).unwrap(), full);
    assert!(origin_server.await.unwrap()[0].contains("range: bytes=5-"));
    let cdn_head = &cdn_server.await.unwrap()[0];
    assert!(cdn_head.contains("range: bytes=5-"), "{cdn_head}");
    assert!(!has_token(cdn_head));
}

#[tokio::test]
async fn eine_weiterleitung_auf_denselben_ursprung_behaelt_das_token() {
    let body = b"ok";
    // Zwei Antworten desselben Servers: erst 302 auf einen anderen Pfad, dann die Datei.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let location = format!("{base}/api/resolve-cache/file");
    let server = tokio::spawn(async move {
        let mut heads = Vec::new();
        for response in [
            http_response(
                "302 Found",
                &[format!("Location: {location}"), "Content-Length: 0".into()],
                b"",
            ),
            http_response("200 OK", &["Content-Length: 2".into()], b"ok"),
        ] {
            let (mut sock, _) = listener.accept().await.unwrap();
            heads.push(read_request_head(&mut sock).await.to_lowercase());
            let _ = sock.write_all(&response).await;
            let _ = sock.shutdown().await;
        }
        heads
    });
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    let auth = auth_for_port(port_of(&base));

    run_authed(
        &format!("{base}/resolve/file"),
        &partial,
        Some(2),
        Some(&sha_hex(body)),
        Some(&auth),
    )
    .await
    .unwrap();

    let heads = server.await.unwrap();
    assert!(has_token(&heads[0]) && has_token(&heads[1]), "{heads:?}");
}

#[tokio::test]
async fn endlose_weiterleitungen_brechen_ab() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let location = format!("{base}/again");
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let _ = read_request_head(&mut sock).await;
            let r = http_response(
                "302 Found",
                &[format!("Location: {location}"), "Content-Length: 0".into()],
                b"",
            );
            let _ = sock.write_all(&r).await;
            let _ = sock.shutdown().await;
        }
    });
    let dir = TempDir::new().unwrap();
    let partial = dir.path().join("m.partial");
    let auth = auth_for_port(port_of(&base));
    let err = run_authed(&format!("{base}/x"), &partial, Some(1), None, Some(&auth))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("too many redirects"), "{err}");
    assert!(!err.to_string().contains(TOKEN));
}

#[tokio::test]
async fn http_401_und_403_sind_typisierte_fehler_und_nennen_das_token_nie() {
    for status in [401u16, 403] {
        let (base, server) = serve_script(vec![http_response(
            &format!("{status} Forbidden"),
            &["Content-Length: 0".to_string()],
            b"",
        )])
        .await;
        let dir = TempDir::new().unwrap();
        let partial = dir.path().join("m.partial");
        let auth = auth_for_port(port_of(&base));
        let err = run_authed(
            &format!("{base}/file"),
            &partial,
            Some(1),
            None,
            Some(&auth),
        )
        .await
        .unwrap_err();
        let typed = err
            .downcast_ref::<HttpStatusError>()
            .expect("typisierter Statusfehler");
        assert_eq!(typed.0.as_u16(), status);
        // Wortlaut unveraendert (andere Aufrufer und Tests kennen ihn).
        assert!(
            err.to_string().starts_with("server returned HTTP "),
            "{err}"
        );
        for text in [err.to_string(), format!("{err:?}"), format!("{err:#}")] {
            assert!(!text.contains(TOKEN), "Token im Fehlertext: {text}");
        }
        let heads = server.await.unwrap();
        assert!(has_token(&heads[0]));
        assert!(!partial.exists() || fs::metadata(&partial).unwrap().len() == 0);
    }
}

#[test]
fn secret_und_zugang_zeigen_den_wert_in_keiner_ausgabe() {
    let auth = DownloadAuth::huggingface(Secret::new(TOKEN).unwrap());
    for text in [format!("{auth:?}"), format!("{auth:#?}")] {
        assert!(!text.contains(TOKEN), "{text}");
        assert!(!text.contains("GeheimesTest"), "{text}");
    }
    assert!(format!("{:?}", Secret::new(TOKEN).unwrap()).contains("***"));
}

#[test]
fn ein_ungueltiges_token_wird_abgelehnt() {
    assert!(Secret::new("").is_none());
    assert!(Secret::new("   \r\n").is_none());
    assert!(Secret::new("hf_abc def").is_none(), "Leerzeichen im Token");
    assert!(
        Secret::new("hf_abc\r\nX-Evil: 1").is_none(),
        "Zeilenumbruch im Token"
    );
    assert!(Secret::new("hf_\u{fc}nicode").is_none());
    assert!(Secret::new(&"a".repeat(513)).is_none());
    assert!(
        Secret::new("  hf_gueltig_123\n").is_some(),
        "Umgebung darf Rand-Leerraum haben"
    );
}

#[test]
fn die_freigabe_gilt_nur_fuer_https_huggingface_co() {
    let auth = DownloadAuth::huggingface(Secret::new(TOKEN).unwrap());
    let ok = |u: &str| auth.applies_to(&reqwest::Url::parse(u).unwrap());
    assert!(ok(
        "https://huggingface.co/gradium/phonon-40m-de/resolve/abc/config.json"
    ));
    assert!(
        ok("https://HuggingFace.co/x"),
        "Gross-/Kleinschreibung des Hosts"
    );
    assert!(!ok("http://huggingface.co/x"), "nie ueber http");
    assert!(
        !ok("https://cdn-lfs.huggingface.co/x"),
        "Unterdomain ist ein anderer Host"
    );
    assert!(!ok("https://cas-bridge.xethub.hf.co/x"));
    assert!(!ok("https://huggingface.co.evil.example/x"));
    assert!(!ok("https://evil.example/huggingface.co"));
    assert!(!ok("https://huggingface.co:8443/x"));
    assert!(
        !ok("https://huggingface.co@evil.example/x"),
        "Host ist evil.example"
    );
}

#[test]
fn weiterleitungsziele_werden_geprueft() {
    use super::auth::redirect_target;
    let from = reqwest::Url::parse("https://huggingface.co/a/b").unwrap();
    assert_eq!(
        redirect_target(&from, "/api/resolve-cache/x?y=1")
            .unwrap()
            .as_str(),
        "https://huggingface.co/api/resolve-cache/x?y=1"
    );
    assert_eq!(
        redirect_target(&from, "https://cas-bridge.xethub.hf.co/file?sig=1")
            .unwrap()
            .host_str(),
        Some("cas-bridge.xethub.hf.co")
    );
    assert!(
        redirect_target(&from, "http://example.org/x").is_err(),
        "kein Rueckschritt auf http"
    );
    assert!(redirect_target(&from, "file:///C:/Windows/win.ini").is_err());
    assert!(redirect_target(&from, "ftp://example.org/x").is_err());
    assert!(
        redirect_target(&from, "https://user:pw@example.org/x").is_err(),
        "keine Zugangsdaten in der Adresse"
    );
    let plain = reqwest::Url::parse("http://127.0.0.1:1/a").unwrap();
    assert!(
        redirect_target(&plain, "http://127.0.0.1:2/b").is_ok(),
        "http bleibt http"
    );
}
