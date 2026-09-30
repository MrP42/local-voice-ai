//! Gemeinsame Bausteine der Tests (nur `cfg(test)`): ein kleiner HTTP-Server,
//! der vorbereitete Antworten ausliefert und die Anfragen mitschreibt.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Eine vorbereitete Antwort, optional nach einer Wartezeit.
#[derive(Clone)]
pub struct Reply {
    pub delay: Duration,
    pub bytes: Vec<u8>,
}

impl From<Vec<u8>> for Reply {
    fn from(bytes: Vec<u8>) -> Self {
        Reply {
            delay: Duration::ZERO,
            bytes,
        }
    }
}

pub fn delayed(ms: u64, bytes: Vec<u8>) -> Reply {
    Reply {
        delay: Duration::from_millis(ms),
        bytes,
    }
}

pub fn json_ok(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

pub fn status(code: u16, reason: &str) -> Vec<u8> {
    format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .into_bytes()
}

pub type Seen = Arc<Mutex<Vec<String>>>;

/// Beantwortet jede Verbindung mit der naechsten vorbereiteten Antwort (die
/// letzte wiederholt sich) und merkt sich die Anfragen. Eine leere Antwort laesst
/// den Client in seinen Zeitlimit laufen. Rueckgabe: Adresse des oEmbed-Pfads.
pub async fn serve<R: Into<Reply>>(responses: Vec<R>) -> (String, Seen) {
    let responses: Vec<Reply> = responses.into_iter().map(Into::into).collect();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    tokio::spawn(async move {
        let mut n = 0usize;
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let reply = responses[n.min(responses.len() - 1)].clone();
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
                if reply.bytes.is_empty() {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    return;
                }
                if !reply.delay.is_zero() {
                    tokio::time::sleep(reply.delay).await;
                }
                let _ = sock.write_all(&reply.bytes).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (format!("http://{addr}/oembed"), seen)
}
