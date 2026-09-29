//! Lokales Sprachmodell: Laufzeit-Download, Modelle und der Serverprozess.
//!
//! Die App spricht mit jedem Anbieter OpenAI-kompatibel; der gebuendelte
//! `llama-server` ist einfach ein weiterer -- nur dass die App ihn selbst
//! holt, startet und beendet. Muster wie bei der Piper-Laufzeit
//! (`managers::tts::models`): Katalogeintrag je Plattform, Download mit
//! Pruefsumme, entpacken, aufloesen.

pub mod estimate;
pub mod resources;
pub mod runtime;
pub mod server;

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

pub use estimate::FitReport;
pub use resources::SystemMemory;
pub use runtime::{LlmDownloadInfo, LlmDownloadKind, LlmRuntimeManager};
pub use server::{LocalLlmServer, LocalLlmStatus, StartOptions};

use crate::settings::PostProcessProvider;

/// Kennung der Anbieter-Vorlage, hinter der der lokale Server steht.
pub const LOCAL_PROVIDER_ID: &str = "local";

/// Platzhalter-Adresse der Vorlage: Port 0 heisst "den waehlt der Server
/// beim Start". Daran erkennt `llm_client` den lokalen Anbieter -- auch bei
/// einer Verbindung mit eigener Kennung.
pub const LOCAL_PLACEHOLDER_URL: &str = "http://127.0.0.1:0/v1";

/// Standard-Kontext fuer lokale Modelle. Gross genug fuer ein Protokoll,
/// klein genug, dass der KV-Cache nicht den Speicher frisst.
pub const DEFAULT_CONTEXT_TOKENS: u32 = 8192;

static RUNTIME: OnceLock<Arc<LlmRuntimeManager>> = OnceLock::new();
static SERVER: OnceLock<Arc<LocalLlmServer>> = OnceLock::new();

/// Einmal beim App-Start gesetzt. `llm_client` hat keinen `AppHandle`, muss
/// den lokalen Server aber starten koennen, bevor es ihn anspricht.
pub fn install_globals(runtime: Arc<LlmRuntimeManager>, server: Arc<LocalLlmServer>) {
    let _ = RUNTIME.set(runtime);
    let _ = SERVER.set(server);
}

/// Ist dieser Anbieter der lokale Server? Erkannt an Vorlage oder
/// Platzhalter-Adresse, nicht am Namen der Verbindung.
pub fn is_local(provider: &PostProcessProvider) -> bool {
    provider.id == LOCAL_PROVIDER_ID || provider.base_url.trim_end_matches('/') == LOCAL_PLACEHOLDER_URL.trim_end_matches('/')
}

/// Geladene lokale Modelle -- das, was ein "Modelle laden" fuer den lokalen
/// Anbieter liefert. Kein Server noetig.
pub fn downloaded_model_ids() -> Vec<String> {
    RUNTIME
        .get()
        .map(|r| {
            r.list_downloads()
                .into_iter()
                .filter(|d| d.kind == LlmDownloadKind::Model && d.is_downloaded)
                .map(|d| d.id)
                .collect()
        })
        .unwrap_or_default()
}

/// Die Adresse, an die eine Anfrage fuer `model` geht: fuer jeden Anbieter
/// seine eigene -- fuer den lokalen die des laufenden Servers, der dafuer
/// bei Bedarf gestartet wird.
pub async fn resolve_base_url(provider: &PostProcessProvider, model: &str) -> Result<String, String> {
    if is_local(provider) {
        ensure_local(model).await
    } else {
        Ok(provider.base_url.clone())
    }
}

/// Sorgt dafuer, dass der lokale Server das Modell `model_id` bedient, und
/// liefert seine Adresse. Startet ihn bei Bedarf -- die Laufzeit wird per
/// Selbsttest gewaehlt.
pub async fn ensure_local(model_id: &str) -> Result<String, String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "Lokales Sprachmodell nicht initialisiert".to_string())?;
    let server = SERVER
        .get()
        .ok_or_else(|| "Lokales Sprachmodell nicht initialisiert".to_string())?;
    if server.is_serving(model_id) {
        if let Some(url) = server.base_url() {
            return Ok(url);
        }
    }
    let model_path = runtime
        .model_path(model_id)
        .filter(|p| p.is_file())
        .ok_or_else(|| format!("Modell nicht geladen: {model_id}"))?;
    let (_, backend, binary) = runtime.resolve_runtime().await?;
    let gpu_layers = if backend == "cpu" { 0 } else { 99 };
    let port = server
        .ensure(
            &binary,
            StartOptions {
                model_id: model_id.to_string(),
                model_path,
                backend,
                context_tokens: DEFAULT_CONTEXT_TOKENS,
                gpu_layers,
                embedding: None,
            },
            Some(runtime.log_path()),
        )
        .await?;
    Ok(format!("http://127.0.0.1:{port}/v1"))
}

// ---------------------------------------------------------------------------
// M4-P4b: zweiter Server im Embedding-Modus (BGE-M3, M4 D3)
// ---------------------------------------------------------------------------
//
// Ein Chat-Server kann keine Embeddings liefern und umgekehrt (Spike V4),
// deshalb ein eigener Prozess mit eigenem Global. Regeln wie beim Chat-Server:
// Start nur ueber das RAM-Gate, Job-Objekt-Deckel, Beenden ueber die PID; dazu
// der Speicherwaechter (lib.rs) und der Leerlauf-Stopp nach 5 min.
//
// Fehler von `ensure_embedding` tragen einen Code als Praefix
// (`no_model: ...`, `no_runtime: ...`, `memory_low: ...`, `failed: ...`), den
// `search::embed` auf `EmbedError` abbildet. Kein Text aus Besprechungen.

/// Katalog-Kennung des einzigen Embedding-Modells (BGE-M3 Q8_0, MIT).
pub const EMBED_MODEL_ID: &str = "emb-bge-m3-q8";
/// Kontext des Embedding-Servers: zwei Slots zu je 2 048 Token.
pub const EMBED_CONTEXT_TOKENS: u32 = 4096;

static EMBED_SERVER: OnceLock<Arc<LocalLlmServer>> = OnceLock::new();
static EMBED_START: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
/// Zeitpunkt der letzten Nutzung (ms seit Unix-Epoche), fuer den Leerlauf-Stopp.
static EMBED_LAST_USE_MS: AtomicU64 = AtomicU64::new(0);
/// Laufende Anfragen; der Leerlauf-Stopp wartet, bis keine mehr laeuft.
static EMBED_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

fn embed_server() -> &'static Arc<LocalLlmServer> {
    EMBED_SERVER.get_or_init(|| Arc::new(LocalLlmServer::new()))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn touch_embedding() {
    EMBED_LAST_USE_MS.store(now_ms(), Ordering::Release);
}

/// Haelt eine Embedding-Anfrage als "laufend" (RAII): der Leerlauf-Stopp
/// beendet den Server nicht unter einer laufenden Anfrage weg.
pub struct EmbeddingUse(());

impl EmbeddingUse {
    pub fn begin() -> Self {
        EMBED_IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        touch_embedding();
        Self(())
    }
}

impl Drop for EmbeddingUse {
    fn drop(&mut self) {
        touch_embedding();
        EMBED_IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Liegt die Modelldatei des Embedding-Modells vor?
pub fn embedding_model_ready(model_id: &str) -> bool {
    RUNTIME
        .get()
        .and_then(|r| r.embedding_model_path(model_id))
        .is_some_and(|p| p.is_file())
}

/// Wird das Embedding-Modell gerade heruntergeladen?
pub fn embedding_model_downloading(model_id: &str) -> bool {
    RUNTIME.get().is_some_and(|r| r.is_downloading_id(model_id))
}

/// Laeuft (oder startet) der Embedding-Server gerade?
pub fn embedding_running() -> bool {
    EMBED_SERVER.get().is_some_and(|s| s.has_process())
}

/// PID des Embedding-Servers (Messung im Headless-Lauf).
pub fn embedding_pid() -> Option<u32> {
    EMBED_SERVER.get().and_then(|s| s.pid())
}

/// Sorgt dafuer, dass der Embedding-Server `model_id` bedient, und liefert
/// seine Adresse (`http://127.0.0.1:<port>/v1`). Starts sind serialisiert:
/// Indexer und Chat-Abfrage duerfen nie zwei Prozesse gleichzeitig starten.
/// Laufzeit und Backend teilt er mit dem Chat-Server (Selbsttest).
pub async fn ensure_embedding(model_id: &str) -> Result<String, String> {
    let lock = EMBED_START.get_or_init(|| tokio::sync::Mutex::new(()));
    let _guard = lock.lock().await;
    touch_embedding();
    let server = embed_server();
    if server.child_exited() {
        // Abgestuerzt (oder vom Job-Deckel beendet): erst abraeumen, sonst
        // haelt `ensure` den toten Prozess fuer bereit und liefert seinen Port.
        log::warn!("Embedding-Server hat sich beendet, wird neu gestartet");
        server.stop();
    }
    if server.is_serving(model_id) {
        if let Some(url) = server.base_url() {
            return Ok(url);
        }
    }
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "no_runtime: Lokales Sprachmodell nicht initialisiert".to_string())?;
    let model_path = runtime
        .embedding_model_path(model_id)
        .filter(|p| p.is_file())
        .ok_or_else(|| format!("no_model: Embedding-Modell nicht geladen: {model_id}"))?;
    let (_, backend, binary) = runtime
        .resolve_runtime()
        .await
        .map_err(|e| format!("no_runtime: {e}"))?;
    // Eigenes Gate VOR dem Start, damit "zu wenig RAM" als Code ankommt
    // (der Server prueft beim Start noch einmal).
    crate::process_guard::check_ram_for_start(server::EMBED_RAM_NEED_MB)
        .map_err(|e| format!("memory_low: {e}"))?;
    let gpu_layers = if backend == "cpu" { 0 } else { 99 };
    let port = server
        .ensure(
            &binary,
            StartOptions {
                model_id: model_id.to_string(),
                model_path,
                backend,
                context_tokens: EMBED_CONTEXT_TOKENS,
                gpu_layers,
                embedding: Some(server::EmbeddingOpts {
                    pooling: "cls",
                    parallel: 2,
                    below_normal: true,
                }),
            },
            Some(runtime.embed_log_path()),
        )
        .await
        .map_err(|e| {
            if e.contains("Arbeitsspeicher") {
                format!("memory_low: {e}")
            } else if e.contains("Modell fehlt") {
                format!("no_model: {e}")
            } else {
                format!("failed: {e}")
            }
        })?;
    touch_embedding();
    Ok(format!("http://127.0.0.1:{port}/v1"))
}

/// Beendet den Embedding-Server (Speicherwaechter, Gates, App-Ende). Ohne
/// laufenden Prozess ein Nichts.
pub fn stop_embedding() {
    if let Some(server) = EMBED_SERVER.get() {
        if server.has_process() {
            server.stop();
        }
    }
}

/// Beendet den Embedding-Server, wenn er `idle` lang nicht benutzt wurde und
/// keine Anfrage laeuft. Liefert, ob gestoppt wurde.
pub fn stop_embedding_if_idle(idle: std::time::Duration) -> bool {
    if !embedding_running() || EMBED_IN_FLIGHT.load(Ordering::Acquire) > 0 {
        return false;
    }
    let last = EMBED_LAST_USE_MS.load(Ordering::Acquire);
    if now_ms().saturating_sub(last) < idle.as_millis() as u64 {
        return false;
    }
    log::info!("Embedding-Server: {} s ohne Nutzung, wird beendet", idle.as_secs());
    stop_embedding();
    true
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn provider(id: &str, base_url: &str) -> PostProcessProvider {
        PostProcessProvider {
            id: id.into(),
            label: id.into(),
            base_url: base_url.into(),
            allow_base_url_edit: false,
            models_endpoint: None,
            supports_structured_output: false,
        }
    }

    #[test]
    fn the_local_provider_is_recognised_by_template_or_placeholder() {
        assert!(is_local(&provider("local", "http://127.0.0.1:0/v1")));
        // Eine Verbindung mit eigener Kennung, aber Platzhalter-Adresse.
        assert!(is_local(&provider("local-abc", "http://127.0.0.1:0/v1/")));
        assert!(!is_local(&provider("ollama", "http://127.0.0.1:11434/v1")));
        assert!(!is_local(&provider("openai", "https://api.openai.com/v1")));
    }

    #[tokio::test]
    async fn a_cloud_provider_keeps_its_own_address() {
        let p = provider("openai", "https://api.openai.com/v1");
        assert_eq!(resolve_base_url(&p, "gpt-4.1").await.unwrap(), "https://api.openai.com/v1");
    }

    /// Ohne initialisierte Laufzeit kann kein lokaler Server starten -- der
    /// Fehler sagt das, statt in einen Verbindungsversuch zu laufen.
    #[tokio::test]
    async fn the_local_provider_fails_readably_when_uninitialised() {
        let p = provider("local", "http://127.0.0.1:0/v1");
        let err = resolve_base_url(&p, "llm-qwen3-4b-q4").await.unwrap_err();
        assert!(err.contains("nicht initialisiert") || err.contains("nicht geladen"), "{err}");
    }

    // ---- M4-P4b ----------------------------------------------------------

    /// Tests, die den globalen Embedding-Server anfassen, laufen nacheinander.
    pub(crate) fn global_embed_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Ohne Laufzeit startet kein Embedding-Server; der Fehler traegt einen
    /// Code, und es laeuft danach kein Prozess.
    #[tokio::test]
    async fn embedding_without_runtime_fails_with_a_code_and_starts_nothing() {
        let _g = global_embed_lock();
        let err = ensure_embedding(EMBED_MODEL_ID).await.unwrap_err();
        assert!(
            err.starts_with("no_runtime:") || err.starts_with("no_model:"),
            "{err}"
        );
        assert!(!embedding_running());
        assert!(!embedding_model_ready(EMBED_MODEL_ID));
        // Stoppen ohne Prozess ist harmlos (Waechter und App-Ende rufen blind).
        stop_embedding();
        stop_embedding();
        assert!(!stop_embedding_if_idle(std::time::Duration::ZERO));
    }

    /// Zwei gleichzeitige Aufrufe (Indexer + Chat-Abfrage) laufen nacheinander
    /// durch die Start-Sperre und enden beide sauber.
    #[tokio::test]
    async fn concurrent_embedding_starts_are_serialised() {
        let _g = global_embed_lock();
        let (a, b) = tokio::join!(ensure_embedding(EMBED_MODEL_ID), ensure_embedding(EMBED_MODEL_ID));
        assert!(a.is_err() && b.is_err());
        assert!(!embedding_running());
    }

    /// Abgestuerzter Embedding-Server (Prozess beendet, Zustand noch
    /// "bereit"): `ensure_embedding` raeumt ihn ab, statt den toten Port zu
    /// liefern.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_crashed_embedding_server_is_cleared_before_reuse() {
        use std::os::windows::process::CommandExt;
        let _g = global_embed_lock();
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "exit 3"])
            .creation_flags(0x0800_0000)
            .spawn()
            .expect("cmd.exe");
        let _ = child.wait();
        embed_server().adopt_for_test(child, EMBED_MODEL_ID);
        assert!(embedding_running());
        let result = ensure_embedding(EMBED_MODEL_ID).await;
        assert!(result.is_err(), "toter Server darf nicht als bereit gelten: {result:?}");
        assert!(!embedding_running(), "toter Prozess abgeraeumt");
    }

    #[test]
    fn an_embedding_use_counts_while_alive() {
        // Andere Tests laufen parallel: nur die eigenen zwei Anfragen zaehlen.
        let _a = EmbeddingUse::begin();
        let _b = EmbeddingUse::begin();
        assert!(EMBED_IN_FLIGHT.load(Ordering::Acquire) >= 2);
        assert!(EMBED_LAST_USE_MS.load(Ordering::Acquire) > 0);
        // Mit laufender Anfrage stoppt der Leerlauf-Stopp nie.
        assert!(!stop_embedding_if_idle(std::time::Duration::ZERO));
    }
}
