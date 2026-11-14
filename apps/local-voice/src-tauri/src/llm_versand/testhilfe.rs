//! Hilfen für Tests mit lokalem HTTP-Testserver (lva#604, Teilpaket 3). Nur unter `cfg(test)`.
//!
//! - [`mock`]: ein Server auf `127.0.0.1` (freier Port), der pro Anfrage einen Handler ruft und
//!   jede Anfrage protokolliert. Antworten tragen `Connection: close`, jede Anfrage ist also eine
//!   Verbindung und wird einmal gezählt.
//! - [`anbieter`]: ein Anbieter mit frei wählbarer Kennung und Adresse.
//! - [`kind_ausfuehren`]: startet die eigene Test-Binärdatei nur mit einem Test und bereinigter
//!   Umgebung. Wer Umgebungsvariablen (Proxy) braucht, setzt sie dort im **Kindprozess**, in dem
//!   nur dieser Test läuft; im Elternprozess liefe sonst jeder andere Test mit denselben
//!   Variablen.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::settings::PostProcessProvider;

/// Ein Anbieter mit dieser Kennung und Basis-Adresse.
pub(super) fn anbieter(id: &str, basis_url: &str) -> PostProcessProvider {
    PostProcessProvider {
        id: id.into(),
        label: "Test".into(),
        base_url: basis_url.into(),
        allow_base_url_edit: true,
        models_endpoint: None,
        supports_structured_output: false,
    }
}

/// Wird vor dem Schreiben des Stücks mit der Nummer (ab 0) gerufen.
pub(super) type VorStueck = Arc<dyn Fn(usize) + Send + Sync>;

/// Die Antwort des Testservers: Status, Content-Type, zusätzliche Kopfzeilen und Stücke, die mit
/// einer kleinen Pause einzeln geschrieben werden (der Body endet mit dem Schließen).
pub(super) struct RohAntwort {
    pub(super) status: u16,
    pub(super) content_type: &'static str,
    pub(super) kopfzeilen: Vec<String>,
    pub(super) stuecke: Vec<Vec<u8>>,
    /// Beobachtungspunkt für Tests: wie weit der Server mit dem Senden ist.
    pub(super) vor_stueck: Option<VorStueck>,
    /// Pause nach jedem Stück (Standard 5 ms).
    pub(super) pause: Duration,
}

impl RohAntwort {
    pub(super) fn neu(status: u16, content_type: &'static str, stuecke: Vec<Vec<u8>>) -> Self {
        Self {
            status,
            content_type,
            kopfzeilen: Vec::new(),
            stuecke,
            vor_stueck: None,
            pause: Duration::from_millis(5),
        }
    }

    pub(super) fn text(status: u16, text: &str) -> Self {
        Self::neu(status, "text/plain", vec![text.as_bytes().to_vec()])
    }

    /// Eine ganze Chat-Antwort (kein Strom) mit Inhalt, Abschlussgrund und Token.
    pub(super) fn json(inhalt: &str, finish: &str, prompt: u64, completion: u64) -> Self {
        let body = serde_json::json!({
            "choices": [{ "message": { "content": inhalt }, "finish_reason": finish }],
            "usage": { "prompt_tokens": prompt, "completion_tokens": completion },
        });
        Self::neu(200, "application/json", vec![body.to_string().into_bytes()])
    }

    /// Ein Strom aus Textstücken, mit `[DONE]` am Ende.
    pub(super) fn strom(teile: &[&str]) -> Self {
        let mut stuecke = sse_text(teile);
        stuecke.push(b"data: [DONE]\n\n".to_vec());
        Self::neu(200, "text/event-stream", stuecke)
    }

    pub(super) fn mit_stuecken(mut self, mehr: Vec<Vec<u8>>) -> Self {
        self.stuecke.extend(mehr);
        self
    }

    /// Ein Haken vor jedem Stück (Nummer ab 0): so sieht ein Test, wie weit der Server ist, wenn
    /// ein Rückruf läuft.
    pub(super) fn mit_vor_stueck(mut self, haken: impl Fn(usize) + Send + Sync + 'static) -> Self {
        self.vor_stueck = Some(Arc::new(haken));
        self
    }

    pub(super) fn mit_pause(mut self, pause: Duration) -> Self {
        self.pause = pause;
        self
    }
}

/// Eine eingegangene Anfrage.
#[derive(Clone, Debug)]
pub(super) struct MockAnfrage {
    /// Laufende Nummer ab 0.
    pub(super) nr: usize,
    /// Die Anfragezeile (`POST /v1/chat/completions HTTP/1.1`, beim Proxy mit voller URL).
    pub(super) zeile: String,
    pub(super) body: String,
}

/// Der Testserver.
pub(super) struct Mock {
    pub(super) port: u16,
    treffer: Arc<AtomicUsize>,
    anfragen: Arc<Mutex<Vec<MockAnfrage>>>,
}

impl Mock {
    /// `http://127.0.0.1:<port>`.
    pub(super) fn adresse(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Basis-URL eines OpenAI-kompatiblen Anbieters auf diesem Server.
    pub(super) fn basis(&self) -> String {
        format!("{}/v1", self.adresse())
    }

    /// Wie viele Anfragen sind vollständig eingegangen?
    pub(super) fn treffer(&self) -> usize {
        self.treffer.load(Ordering::SeqCst)
    }

    pub(super) fn anfragen(&self) -> Vec<MockAnfrage> {
        self.anfragen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Startet den Testserver; `handler` bekommt jede Anfrage und liefert die Antwort.
pub(super) async fn mock(
    handler: impl Fn(&MockAnfrage) -> RohAntwort + Send + Sync + 'static,
) -> Mock {
    let handler = Arc::new(handler);
    let treffer = Arc::new(AtomicUsize::new(0));
    let anfragen: Arc<Mutex<Vec<MockAnfrage>>> = Arc::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (zaehler, protokoll) = (Arc::clone(&treffer), Arc::clone(&anfragen));
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let (handler, zaehler, protokoll) = (
                Arc::clone(&handler),
                Arc::clone(&zaehler),
                Arc::clone(&protokoll),
            );
            tokio::spawn(async move {
                let mut puffer = vec![0u8; 1 << 20];
                let mut gelesen = 0usize;
                let (zeile, body) = loop {
                    let n = sock.read(&mut puffer[gelesen..]).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    gelesen += n;
                    let text = String::from_utf8_lossy(&puffer[..gelesen]).to_string();
                    let Some(ende) = text.find("\r\n\r\n") else {
                        continue;
                    };
                    let laenge = text[..ende]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|v| v.trim().to_string())
                        })
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if gelesen >= ende + 4 + laenge {
                        let zeile = text.lines().next().unwrap_or_default().to_string();
                        let body = String::from_utf8_lossy(&puffer[ende + 4..ende + 4 + laenge])
                            .to_string();
                        break (zeile, body);
                    }
                };
                let nr = zaehler.fetch_add(1, Ordering::SeqCst);
                let anfrage = MockAnfrage { nr, zeile, body };
                protokoll
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(anfrage.clone());
                let antwort = handler(&anfrage);
                let mut kopf = format!(
                    "HTTP/1.1 {} Mock\r\nContent-Type: {}\r\nConnection: close\r\n",
                    antwort.status, antwort.content_type
                );
                for zeile in &antwort.kopfzeilen {
                    kopf.push_str(zeile);
                    kopf.push_str("\r\n");
                }
                kopf.push_str("\r\n");
                let _ = sock.write_all(kopf.as_bytes()).await;
                for (nr, stueck) in antwort.stuecke.into_iter().enumerate() {
                    if let Some(haken) = &antwort.vor_stueck {
                        haken(nr);
                    }
                    let _ = sock.write_all(&stueck).await;
                    let _ = sock.flush().await;
                    tokio::time::sleep(antwort.pause).await;
                }
                let _ = sock.shutdown().await;
            });
        }
    });
    Mock {
        port,
        treffer,
        anfragen,
    }
}

/// SSE-Zeilen mit Textstücken.
pub(super) fn sse_text(teile: &[&str]) -> Vec<Vec<u8>> {
    teile
        .iter()
        .map(|t| {
            format!(
                "data: {}\n\n",
                serde_json::json!({"choices": [{"delta": {"content": t}}]})
            )
            .into_bytes()
        })
        .collect()
}

/// SSE-Zeilen mit Denktext (`reasoning_content`).
pub(super) fn sse_denken(teile: &[&str]) -> Vec<Vec<u8>> {
    teile
        .iter()
        .map(|t| {
            format!(
                "data: {}\n\n",
                serde_json::json!({"choices": [{"delta": {"reasoning_content": t}}]})
            )
            .into_bytes()
        })
        .collect()
}

/// Ein Sammler für Textstücke: der Rückruf und das Gesammelte.
pub(super) fn sammler() -> (Arc<Mutex<Vec<String>>>, impl Fn(&str) + Send + Sync) {
    let gesehen = Arc::new(Mutex::new(Vec::new()));
    let ziel = Arc::clone(&gesehen);
    (gesehen, move |stueck: &str| {
        ziel.lock().unwrap().push(stueck.to_string())
    })
}

/// Umgebungsvariable: dieser Prozess ist das Kind eines [`kind_ausfuehren`].
const KIND_VAR: &str = "LVA_VERSAND_TEST_KIND";

/// Bin ich der Kindprozess eines [`kind_ausfuehren`]?
pub(super) fn ist_kind() -> bool {
    std::env::var_os(KIND_VAR).is_some()
}

/// Startet die eigene Test-Binärdatei nur mit `modul::name` (ohne Namen der Kiste) und wartet
/// höchstens eine Minute. Das Kind bekommt eine Umgebung ohne Proxyvariablen und ohne `NO_PROXY`;
/// es setzt, was es braucht, selbst. Schlägt fehl, wenn das Kind scheitert, hängt oder nicht
/// genau einen Test ausgeführt hat (falscher Name: `0 tests`).
pub(super) fn kind_ausfuehren(modulpfad: &str, name: &str) {
    let innen = modulpfad.split_once("::").map_or("", |(_, rest)| rest);
    let testname = if innen.is_empty() {
        name.to_string()
    } else {
        format!("{innen}::{name}")
    };
    let log = tempfile::NamedTempFile::new().expect("Ausgabedatei");
    let ausgabe = log.reopen().expect("Ausgabe");
    let fehler = ausgabe.try_clone().expect("Ausgabe");
    let mut kind = Command::new(std::env::current_exe().expect("eigene Binärdatei"));
    kind.args([
        testname.as_str(),
        "--exact",
        "--nocapture",
        "--test-threads=1",
    ])
    .env(KIND_VAR, "1")
    .stdin(Stdio::null())
    .stdout(ausgabe)
    .stderr(fehler);
    for var in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        kind.env_remove(var);
    }
    let mut kind = kind.spawn().expect("Kindprozess starten");
    let frist = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = kind.try_wait().expect("Kindprozess abfragen") {
            break Some(status);
        }
        if Instant::now() >= frist {
            let _ = kind.kill();
            let _ = kind.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let text = std::fs::read_to_string(log.path()).unwrap_or_default();
    let status = status.unwrap_or_else(|| panic!("Kindprozess hing (Frist 60 s).\n{text}"));
    assert!(status.success(), "Kindprozess gescheitert.\n{text}");
    assert!(
        text.contains("test result: ok. 1 passed"),
        "Kindprozess hat nicht genau einen Test ausgeführt.\n{text}"
    );
}
