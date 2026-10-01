//! Lokaler Test-Server fuer die Tests des Microsoft-365-Kontos (nur `cfg(test)`).
//!
//! Spielt Anmelde-Server (`/<tenant>/oauth2/v2.0/token`) und Graph (`/v1.0/...`)
//! in einem: jede Verbindung ist eine Anfrage, der Handler antwortet. Kein Netz,
//! kein Browser, keine Fenster.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
pub struct Req {
    pub method: String,
    pub target: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// Der Pfad ohne Abfrage.
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    pub fn query(&self) -> &str {
        self.target.split_once('?').map(|(_, q)| q).unwrap_or("")
    }

    pub fn body_str(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn form(&self) -> HashMap<String, String> {
        url::form_urlencoded::parse(&self.body)
            .into_owned()
            .collect()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    pub fn bearer(&self) -> Option<&str> {
        self.header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
    }
}

pub enum Resp {
    Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// Verbindung nach dem Lesen der Anfrage kappen, ohne Antwort.
    Drop,
}

impl Resp {
    pub fn json(status: u16, v: Value) -> Self {
        Resp::Reply {
            status,
            headers: Vec::new(),
            body: v.to_string().into_bytes(),
        }
    }

    pub fn empty(status: u16) -> Self {
        Resp::Reply {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn with(mut self, k: &str, v: &str) -> Self {
        if let Resp::Reply { headers, .. } = &mut self {
            headers.push((k.to_string(), v.to_string()));
        }
        self
    }
}

pub type Seen = Arc<Mutex<Vec<Req>>>;

/// Startet den Server; liefert `http://127.0.0.1:<port>` und die Liste der
/// gesehenen Anfragen.
pub async fn serve(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> (String, Seen) {
    let handler = Arc::new(handler);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let handler = handler.clone();
            let seen = seen2.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = vec![0u8; 64 * 1024];
                let end = loop {
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break p;
                    }
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                };
                let head = String::from_utf8_lossy(&buf[..end]).to_string();
                let mut lines = head.split("\r\n");
                let mut first = lines.next().unwrap().split(' ');
                let method = first.next().unwrap().to_string();
                let target = first.next().unwrap().to_string();
                let mut headers = HashMap::new();
                for l in lines {
                    if let Some((k, v)) = l.split_once(':') {
                        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                let len: usize = headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let mut body = buf[end + 4..].to_vec();
                while body.len() < len {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => body.extend_from_slice(&tmp[..n]),
                    }
                }
                let req = Req {
                    method,
                    target,
                    headers,
                    body,
                };
                seen.lock().unwrap().push(req.clone());
                match handler(&req) {
                    Resp::Drop => {
                        // Ohne Antwort schliessen: der Aufrufer sieht eine gerissene Verbindung.
                        drop(sock);
                    }
                    Resp::Reply {
                        status,
                        headers,
                        body,
                    } => {
                        let mut out = format!(
                            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n",
                            body.len()
                        );
                        for (k, v) in &headers {
                            out.push_str(&format!("{k}: {v}\r\n"));
                        }
                        out.push_str("\r\n");
                        let mut bytes = out.into_bytes();
                        bytes.extend_from_slice(&body);
                        let _ = sock.write_all(&bytes).await;
                        let _ = sock.shutdown().await;
                    }
                }
            });
        }
    });
    (base, seen)
}
