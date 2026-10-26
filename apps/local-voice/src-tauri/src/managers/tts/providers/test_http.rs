//! Fake-HTTP-Server für die Adapter-Tests (nur Tests, Paket P2).
//!
//! Ein winziger HTTP/1.1-Server auf `127.0.0.1:0` (Muster: `spawn_mock` in
//! `managers/tts/mod.rs`, hier mit Aufzeichnung der Anfragen und
//! gestückelten Antworten). Er kann alles, was die Adapter-Tests brauchen:
//!
//! * JSON-/Byte-Antworten mit frei wählbarem Status und Kopfzeilen,
//! * gestückelte Antworten (`Transfer-Encoding: chunked`), jedes Stück in
//!   EINEM Schreibvorgang, mit Pause dazwischen und wählbarem Ende
//!   (sauber, abgerissen ohne Abschluss, hängend),
//! * einen Server, der nie antwortet (`Hang`) oder die Verbindung ohne
//!   Antwort schließt (`Close`).
//!
//! `client_gone` zählt gestückelte Antworten, bei denen der Schreibvorgang
//! scheiterte (der Client hat die Verbindung geschlossen) — der Beweis, dass
//! ein Abbruch die Verbindung wirklich beendet.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// Eine aufgezeichnete Anfrage (Namen der Kopfzeilen klein geschrieben).
#[derive(Debug, Clone)]
pub(crate) struct Req {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

/// Wie eine gestückelte Antwort endet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamEnd {
    /// Sauber mit dem Abschluss-Stück `0`.
    Finish,
    /// Verbindung ohne Abschluss schließen (abgebrochener Strom).
    Abort,
    /// Nichts mehr senden, Verbindung offen lassen.
    Stall,
}

pub(crate) enum Reply {
    Bytes {
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
        headers: Vec<(&'static str, String)>,
    },
    Stream {
        pieces: Vec<Vec<u8>>,
        gap: Duration,
        content_type: &'static str,
        end: StreamEnd,
    },
    /// Nie antworten.
    Hang,
    /// Verbindung ohne Antwort schließen.
    Close,
    /// Erst nach `delay` mit `then` antworten (z. B. ein Modell, das erst lädt).
    Delayed { delay: Duration, then: Box<Reply> },
}

impl Reply {
    pub fn json(status: u16, value: serde_json::Value) -> Self {
        Self::Bytes {
            status,
            content_type: "application/json",
            body: value.to_string().into_bytes(),
            headers: Vec::new(),
        }
    }

    pub fn text(status: u16, body: &str) -> Self {
        Self::Bytes {
            status,
            content_type: "text/plain",
            body: body.as_bytes().to_vec(),
            headers: Vec::new(),
        }
    }

    pub fn audio(body: Vec<u8>) -> Self {
        Self::Bytes {
            status: 200,
            content_type: "audio/pcm",
            body,
            headers: Vec::new(),
        }
    }

    pub fn stream(pieces: Vec<Vec<u8>>, gap: Duration, end: StreamEnd) -> Self {
        Self::Stream {
            pieces,
            gap,
            content_type: "audio/pcm",
            end,
        }
    }

    /// Erst nach `delay` antworten (vor dem ersten Byte).
    pub fn delayed(delay: Duration, then: Reply) -> Self {
        Self::Delayed {
            delay,
            then: Box::new(then),
        }
    }

    pub fn redirect(to: &str) -> Self {
        Self::Bytes {
            status: 302,
            content_type: "text/plain",
            body: Vec::new(),
            headers: vec![("Location", to.to_string())],
        }
    }
}

type Handler = Arc<dyn Fn(&Req) -> Reply + Send + Sync>;

pub(crate) struct FakeServer {
    /// `http://127.0.0.1:<port>`
    pub root: String,
    pub port: u16,
    requests: Arc<Mutex<Vec<Req>>>,
    /// Gestückelte Antworten, bei denen das Schreiben scheiterte.
    pub client_gone: Arc<AtomicUsize>,
    /// Gestückelte Antworten, die sauber zu Ende gingen.
    pub finished: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeServer {
    pub async fn start(handler: impl Fn(&Req) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handler: Handler = Arc::new(handler);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let client_gone = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicUsize::new(0));
        let task = {
            let (requests, client_gone, finished) =
                (requests.clone(), client_gone.clone(), finished.clone());
            tokio::spawn(async move {
                loop {
                    let Ok((sock, _)) = listener.accept().await else {
                        break;
                    };
                    tokio::spawn(handle(
                        sock,
                        handler.clone(),
                        requests.clone(),
                        client_gone.clone(),
                        finished.clone(),
                    ));
                }
            })
        };
        Self {
            root: format!("http://127.0.0.1:{port}"),
            port,
            requests,
            client_gone,
            finished,
            task,
        }
    }

    pub fn requests(&self) -> Vec<Req> {
        self.requests.lock().unwrap().clone()
    }

    /// Anzahl der Anfragen mit diesem Pfad.
    pub fn count(&self, path: &str) -> usize {
        self.requests().iter().filter(|r| r.path == path).count()
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

async fn handle(
    mut sock: TcpStream,
    handler: Handler,
    log: Arc<Mutex<Vec<Req>>>,
    client_gone: Arc<AtomicUsize>,
    finished: Arc<AtomicUsize>,
) {
    let _ = sock.set_nodelay(true);
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = sock.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = find(&buf, b"\r\n\r\n") {
            break p + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = HashMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < header_end + content_length {
        let n = sock.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let req = Req {
        method,
        path,
        headers,
        body: buf[header_end..header_end + content_length].to_vec(),
    };
    log.lock().unwrap().push(req.clone());

    let mut reply = handler(&req);
    while let Reply::Delayed { delay, then } = reply {
        tokio::time::sleep(delay).await;
        reply = *then;
    }
    match reply {
        Reply::Delayed { .. } => unreachable!("oben aufgelöst"),
        Reply::Hang => std::future::pending::<()>().await,
        Reply::Close => {}
        Reply::Bytes {
            status,
            content_type,
            body,
            headers,
        } => {
            let mut head = format!(
                "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
                reason(status),
                body.len()
            );
            for (k, v) in headers {
                head.push_str(&format!("{k}: {v}\r\n"));
            }
            head.push_str("\r\n");
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(&body).await;
            let _ = sock.shutdown().await;
        }
        Reply::Stream {
            pieces,
            gap,
            content_type,
            end,
        } => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            );
            if sock.write_all(head.as_bytes()).await.is_err() {
                client_gone.fetch_add(1, Ordering::SeqCst);
                return;
            }
            for piece in pieces {
                let mut frame = format!("{:x}\r\n", piece.len()).into_bytes();
                frame.extend_from_slice(&piece);
                frame.extend_from_slice(b"\r\n");
                if sock.write_all(&frame).await.is_err() || sock.flush().await.is_err() {
                    client_gone.fetch_add(1, Ordering::SeqCst);
                    return;
                }
                tokio::time::sleep(gap).await;
            }
            match end {
                StreamEnd::Finish => {
                    if sock.write_all(b"0\r\n\r\n").await.is_err() {
                        client_gone.fetch_add(1, Ordering::SeqCst);
                        return;
                    }
                    let _ = sock.shutdown().await;
                    finished.fetch_add(1, Ordering::SeqCst);
                }
                StreamEnd::Abort => {
                    let _ = sock.shutdown().await;
                }
                StreamEnd::Stall => {
                    // Der Server sendet nichts mehr; ein Schreibversuch nach dem
                    // Schließen durch den Client deckt das Wegbrechen auf.
                    loop {
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        if sock.write_all(b"").await.is_err() {
                            client_gone.fetch_add(1, Ordering::SeqCst);
                            return;
                        }
                        let mut probe = [0u8; 1];
                        match tokio::time::timeout(Duration::from_millis(1), sock.read(&mut probe))
                            .await
                        {
                            Ok(Ok(0)) | Ok(Err(_)) => {
                                client_gone.fetch_add(1, Ordering::SeqCst);
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

// ---- Audio-Bausteine für die Antworten ------------------------------------

/// Sinus (16 Bit mono) mit `frames` Samples.
pub(crate) fn tone_pcm(frames: usize) -> Vec<u8> {
    (0..frames)
        .flat_map(|i| {
            let v = ((i as f32 * 0.05).sin() * 8000.0) as i16;
            v.to_le_bytes()
        })
        .collect()
}

/// WAV-Kopf mit Streaming-Längen (`0xFFFFFFFF`, so schreiben Ollama/Phonon)
/// oder exakten Längen, gefolgt von `pcm`.
pub(crate) fn wav_bytes(pcm: &[u8], rate: u32, streaming_header: bool) -> Vec<u8> {
    let (riff, data) = if streaming_header {
        (u32::MAX, u32::MAX)
    } else {
        (36 + pcm.len() as u32, pcm.len() as u32)
    };
    let mut v = b"RIFF".to_vec();
    v.extend(riff.to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes()); // PCM
    v.extend(1u16.to_le_bytes()); // mono
    v.extend(rate.to_le_bytes());
    v.extend((rate * 2).to_le_bytes());
    v.extend(2u16.to_le_bytes());
    v.extend(16u16.to_le_bytes());
    v.extend(b"data");
    v.extend(data.to_le_bytes());
    v.extend_from_slice(pcm);
    v
}

/// `data` in Stücke zerlegen; `sizes` wird zyklisch benutzt (ungerade Größen
/// legen die Stückgrenzen mitten in Samples und Kopffelder).
pub(crate) fn split_by(data: &[u8], sizes: &[usize]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut i = 0;
    while pos < data.len() {
        let n = sizes[i % sizes.len()].max(1).min(data.len() - pos);
        out.push(data[pos..pos + n].to_vec());
        pos += n;
        i += 1;
    }
    out
}

/// Antwort eines Sprachservers auf `POST …/audio/speech`: wählt Rohdaten
/// oder WAV nach `response_format` und stückelt ungerade. Länge des Audios
/// folgt dem Text (40 ms je Zeichen bei 24 kHz).
pub(crate) fn speech_reply(req: &Req, gap: Duration) -> Reply {
    let body = req.json();
    let input = body["input"].as_str().unwrap_or("");
    let frames = 24 * 40 * input.chars().count().max(1);
    let pcm = tone_pcm(frames);
    let data = match body["response_format"].as_str() {
        Some("wav") => wav_bytes(&pcm, 24_000, true),
        _ => pcm,
    };
    Reply::stream(
        split_by(&data, &[10, 33, 4001, 7, 2999, 1]),
        gap,
        StreamEnd::Finish,
    )
}
