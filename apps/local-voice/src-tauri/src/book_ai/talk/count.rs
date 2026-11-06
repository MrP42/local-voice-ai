//! Exakter Zähler für llama-server (lva#324): die Eingabetoken eines Aufrufs
//! mit dem Tokenizer und der Chat-Vorlage des geladenen Modells.
//!
//! Zwei Aufrufe am laufenden Server (llama.cpp b10938, beide dokumentiert in
//! `tools/server/README.md`):
//! 1. `POST /apply-template` mit den echten Nachrichten liefert `prompt`, den
//!    Text, den der Server dem Modell gibt (Vorlage, Rollen, BOS-Text).
//! 2. `POST /tokenize` mit `{"content": prompt, "add_special": true}` liefert
//!    die Token; ihre Zahl ist die Eingabelänge. `add_special` zählt ein
//!    zusätzliches BOS mit, falls die Vorlage keines enthält (eher eins zu
//!    viel als zu wenig).
//!
//! Das Kontextfenster meldet `GET /props` (`default_generation_settings.n_ctx`).
//! Fehlt ein Endpunkt, antwortet der Server mit einem Fehler oder ist die
//! Antwort unlesbar, gibt es keinen Zähler (`None`): kein Modellaufruf, Antwort
//! `found_only` mit Grund `budget` (fail-closed). Kein Text im Log.

use std::time::Duration;

use serde_json::{json, Value};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

pub struct LlamaCounter {
    client: reqwest::Client,
    /// Adresse ohne `/v1`.
    base: String,
}

fn root(base_url: &str) -> String {
    let t = base_url.trim_end_matches('/');
    t.strip_suffix("/v1").unwrap_or(t).to_string()
}

impl LlamaCounter {
    pub fn new(base_url: &str) -> Option<Self> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .ok()?;
        Some(Self {
            client,
            base: root(base_url),
        })
    }

    async fn post(&self, path: &str, body: &Value) -> Option<Value> {
        let response = self
            .client
            .post(format!("{}{path}", self.base))
            .json(body)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            log::warn!("book_talk: {path} abgelehnt ({})", response.status());
            return None;
        }
        response.json().await.ok()
    }

    /// Kontextfenster (je Slot), wie der Server es meldet.
    pub async fn context(&self) -> Option<usize> {
        let response = self
            .client
            .get(format!("{}/props", self.base))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: Value = response.json().await.ok()?;
        let n = body
            .pointer("/default_generation_settings/n_ctx")?
            .as_u64()?;
        usize::try_from(n).ok().filter(|n| *n > 0)
    }

    /// Exakte Eingabetoken von (System, Nutzer) samt Vorlage.
    pub async fn count(&self, system: &str, user: &str) -> Option<usize> {
        let rendered = self
            .post(
                "/apply-template",
                &json!({
                    "messages": [
                        {"role": "system", "content": system},
                        {"role": "user", "content": user},
                    ],
                    "chat_template_kwargs": {"enable_thinking": false},
                }),
            )
            .await?;
        let prompt = rendered.get("prompt")?.as_str()?;
        let tokens = self
            .post(
                "/tokenize",
                &json!({"content": prompt, "add_special": true}),
            )
            .await?;
        let n = tokens.get("tokens")?.as_array()?.len();
        (n > 0).then_some(n)
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! Kleiner HTTP-Server, der llama-server nachahmt (keine Modelle).

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Clone, Default)]
    pub struct Behaviour {
        pub no_template: bool,
        pub no_tokenize: bool,
        pub no_props: bool,
        pub n_ctx: usize,
    }

    pub struct Server {
        pub url: String,
        pub requests: Arc<AtomicUsize>,
    }

    /// `prompt` = `<s>[system|user]`, ein Token je Zeichen (sichtbar zählbar).
    pub async fn start(b: Behaviour) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (b, seen) = (b.clone(), seen.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 8192];
                    let body_at = loop {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break p + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..body_at]).to_string();
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    while buf.len() < body_at + len {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    seen.fetch_add(1, Ordering::SeqCst);
                    let body: serde_json::Value =
                        serde_json::from_slice(&buf[body_at..]).unwrap_or_default();
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    let (status, reply) = match path.as_str() {
                        "/apply-template" if !b.no_template => {
                            let text: String = body["messages"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|m| {
                                    format!(
                                        "[{}|{}]",
                                        m["role"].as_str().unwrap_or(""),
                                        m["content"].as_str().unwrap_or("")
                                    )
                                })
                                .collect();
                            (200, serde_json::json!({"prompt": format!("<s>{text}")}))
                        }
                        "/tokenize" if !b.no_tokenize => {
                            let n = body["content"].as_str().unwrap_or("").chars().count();
                            (200, serde_json::json!({"tokens": vec![1; n]}))
                        }
                        "/props" if !b.no_props => (
                            200,
                            serde_json::json!({"default_generation_settings": {"n_ctx": b.n_ctx}}),
                        ),
                        _ => (404, serde_json::json!({"error": "nicht da"})),
                    };
                    let payload = reply.to_string();
                    let text = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                        payload.len()
                    );
                    let _ = sock.write_all(text.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Server { url, requests }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{start, Behaviour};
    use super::*;

    fn ok() -> Behaviour {
        Behaviour {
            n_ctx: 4096,
            ..Behaviour::default()
        }
    }

    #[test]
    fn the_v1_suffix_is_cut() {
        assert_eq!(root("http://127.0.0.1:8080/v1"), "http://127.0.0.1:8080");
        assert_eq!(root("http://127.0.0.1:8080/v1/"), "http://127.0.0.1:8080");
        assert_eq!(root("http://127.0.0.1:8080"), "http://127.0.0.1:8080");
    }

    #[tokio::test]
    async fn counts_the_templated_prompt_and_reads_the_context() {
        let s = start(ok()).await;
        let c = LlamaCounter::new(&s.url).unwrap();
        // "<s>[system|ab][user|cde]"
        assert_eq!(c.count("ab", "cde").await, Some(24));
        assert_eq!(c.context().await, Some(4096));
        assert_eq!(s.requests.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_missing_endpoint_means_no_counter() {
        for b in [
            Behaviour {
                no_template: true,
                ..ok()
            },
            Behaviour {
                no_tokenize: true,
                ..ok()
            },
        ] {
            let s = start(b).await;
            let c = LlamaCounter::new(&s.url).unwrap();
            assert_eq!(c.count("a", "b").await, None);
        }
        let s = start(Behaviour {
            no_props: true,
            ..ok()
        })
        .await;
        assert_eq!(LlamaCounter::new(&s.url).unwrap().context().await, None);
    }

    #[tokio::test]
    async fn an_unreachable_server_means_no_counter() {
        let c = LlamaCounter::new("http://127.0.0.1:1/v1").unwrap();
        assert_eq!(c.count("a", "b").await, None);
        assert_eq!(c.context().await, None);
    }
}
