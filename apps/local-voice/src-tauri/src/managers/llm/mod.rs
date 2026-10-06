//! Lokales Sprachmodell: Laufzeit-Download, Modelle und der Serverprozess.
//!
//! Die App spricht mit jedem Anbieter OpenAI-kompatibel; der gebuendelte
//! `llama-server` ist einfach ein weiterer -- nur dass die App ihn selbst
//! holt, startet und beendet. Muster wie bei der Piper-Laufzeit
//! (`managers::tts::models`): Katalogeintrag je Plattform, Download mit
//! Pruefsumme, entpacken, aufloesen.

pub mod app_usage;
pub mod cli;
pub mod context;
pub mod estimate;
pub mod external;
pub mod resources;
pub mod runtime;
pub mod server;
pub mod vision;

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

pub use estimate::FitReport;
pub use resources::SystemMemory;
pub use runtime::{ExternalInfo, LlmDownloadInfo, LlmDownloadKind, LlmRuntimeManager};
pub use server::{LocalLlmServer, LocalLlmStatus, StartOptions, CODE_SERVER_CRASHED};

use crate::settings::PostProcessProvider;

/// Kennung der Anbieter-Vorlage, hinter der der lokale Server steht.
pub const LOCAL_PROVIDER_ID: &str = "local";

/// Platzhalter-Adresse der Vorlage: Port 0 heisst "den waehlt der Server
/// beim Start". Daran erkennt `llm_client` den lokalen Anbieter -- auch bei
/// einer Verbindung mit eigener Kennung.
pub const LOCAL_PLACEHOLDER_URL: &str = "http://127.0.0.1:0/v1";

/// Standard-Kontext fuer lokale Modelle. Gross genug fuer ein Protokoll,
/// klein genug, dass der KV-Cache nicht den Speicher frisst. Boden der
/// automatischen Wahl (`context::choose_context_tokens`, P1g).
pub const DEFAULT_CONTEXT_TOKENS: u32 = 8192;

/// Fehlercode: das gewaehlte Modell aus einem Modellordner laedt mit dieser
/// Laufzeit nicht (`external::Compat::Incompatible`).
pub const CODE_EXTERNAL_INCOMPATIBLE: &str = "external_incompatible";

static RUNTIME: OnceLock<Arc<LlmRuntimeManager>> = OnceLock::new();
static SERVER: OnceLock<Arc<LocalLlmServer>> = OnceLock::new();
/// Modell und Kontext, mit denen der lokale Server zuletzt gestartet wurde.
/// Die Notizen-Budgets richten sich danach (P1g).
static ACTIVE_CONTEXT: std::sync::Mutex<Option<(String, u32)>> = std::sync::Mutex::new(None);

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
                // Fremde Modelle, die diese Laufzeit nicht laden kann, sind
                // nicht waehlbar.
                .filter(|d| {
                    !d.external
                        .as_ref()
                        .is_some_and(|x| matches!(x.compat, external::Compat::Incompatible(_)))
                })
                .map(|d| d.id)
                .collect()
        })
        .unwrap_or_default()
}

/// Kam der Fehler von einem abgestuerzten lokalen Server (Code
/// `server_crashed`, P1h)? Aufrufer koennen dann auf einen Wiederholversuch
/// verzichten: der naechste Start ist fuer eine Minute gesperrt.
pub fn is_server_crashed(err: &str) -> bool {
    err.strip_prefix(CODE_SERVER_CRASHED)
        .is_some_and(|rest| rest.starts_with(':'))
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
    ensure_local_with(model_id, false).await
}

/// D3: wie [`ensure_local`], aber der Server laeuft MIT Bild-Projektor
/// (`--mmproj`, `-b/-ub 2048`). Ein laufender Server ohne Projektor wird dafuer
/// neu gestartet; ein Server mit Projektor bedient danach jede Anfrage, bis
/// [`release_vision`] ihn beendet. Fehlt der Projektor, kommt `no_projector: ...`.
pub async fn ensure_local_vision(model_id: &str) -> Result<String, String> {
    ensure_local_with(model_id, true).await
}

async fn ensure_local_with(model_id: &str, vision: bool) -> Result<String, String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "Lokales Sprachmodell nicht initialisiert".to_string())?;
    let server = SERVER
        .get()
        .ok_or_else(|| "Lokales Sprachmodell nicht initialisiert".to_string())?;
    // P1h: nur ein lebender Prozess UND antwortender Port zaehlt als bereit.
    // Sonst weiter zu `ensure`, das einen Absturz abraeumt, hoechstens einmal
    // pro Minute neu startet und sonst `server_crashed: ...` meldet.
    touch_local();
    if let Some(port) = server.live_port_for(model_id, vision).await {
        return Ok(format!("http://127.0.0.1:{port}/v1"));
    }
    // Fremdes Modell, das diese Laufzeit nicht laden kann (Ollama-Sonderformat):
    // gleich sagen, statt einen Server zu starten, der nach 0,4 s abstuerzt und
    // die Neustart-Sperre ausloest.
    if let Some(reason) = runtime.external_incompatibility(model_id) {
        return Err(format!("{CODE_EXTERNAL_INCOMPATIBLE}: {reason}"));
    }
    let model_path = runtime
        .model_path(model_id)
        .filter(|p| p.is_file())
        .ok_or_else(|| format!("Modell nicht geladen: {model_id}"))?;
    let mmproj = if vision {
        let path = runtime
            .projector_path(vision::VISION_PROJECTOR_ID)
            .filter(|p| p.is_file())
            .ok_or_else(|| format!("{}: Bild-Projektor nicht geladen", vision::CODE_NO_PROJECTOR))?;
        Some(path)
    } else {
        None
    };
    let (_, backend, binary) = runtime.resolve_runtime().await?;
    let gpu_layers = if backend == "cpu" { 0 } else { 99 };
    let context_tokens = plan_context(&model_path, &backend).await;
    let starts_before = server.start_count();
    let port = server
        .ensure(
            &binary,
            StartOptions {
                model_id: model_id.to_string(),
                model_path,
                backend,
                context_tokens,
                gpu_layers,
                embedding: None,
                mmproj,
            },
            Some(runtime.log_path()),
        )
        .await?;
    // Nur ein Start durch diesen Aufruf setzt den Kontext neu; ein Server, der
    // schon lief, hat ihn beim Start bekommen.
    if server.start_count() != starts_before {
        set_active_context(model_id, context_tokens);
    }
    Ok(format!("http://127.0.0.1:{port}/v1"))
}

// ---------------------------------------------------------------------------
// Leerlauf-Stopp des Chat-Servers: das Modell gibt RAM und VRAM wieder frei
// ---------------------------------------------------------------------------

/// Zeitpunkt der letzten Anfrage an den Chat-Server (ms seit Unix-Epoche).
static LOCAL_LAST_USE_MS: AtomicU64 = AtomicU64::new(0);

fn touch_local() {
    LOCAL_LAST_USE_MS.store(now_ms(), Ordering::Release);
}

/// Mindestens so lange bleibt der Chat-Server nach der letzten Anfrage
/// geladen: zwischen zwei Anfragen einer Sitzung (Notizen in Teilen,
/// Nachfragen) soll er nicht jedes Mal neu starten.
pub const LOCAL_MIN_IDLE: std::time::Duration = std::time::Duration::from_secs(120);

/// Leerlauf-Grenze aus der Einstellung "Modelle entladen nach" (Sekunden;
/// `None` = nie). Nie unter [`LOCAL_MIN_IDLE`]: "sofort" und die
/// 15-Sekunden-Stufe des Debug-Modus sind fuer das Diktatmodell gedacht, ein
/// Sprachmodell-Start dauert zu lange dafuer.
pub fn local_idle_limit(unload_secs: Option<u64>) -> Option<std::time::Duration> {
    unload_secs.map(|s| std::time::Duration::from_secs(s).max(LOCAL_MIN_IDLE))
}

/// Darf der Chat-Server jetzt wegen Leerlauf beendet werden? Nur wenn eine
/// Grenze gilt, die letzte Anfrage lange genug her ist und der Server nicht
/// gerade rechnet.
pub fn local_idle_expired(
    idle: std::time::Duration,
    limit: Option<std::time::Duration>,
    busy: bool,
) -> bool {
    !busy && limit.is_some_and(|l| idle >= l)
}

/// Rechnet der Server gerade? `/slots` des llama-servers: ein Slot mit
/// `is_processing` heisst "ja". Kein Endpunkt oder unlesbare Antwort zaehlt
/// als "nein" -- dann entscheidet allein die Zeit seit der letzten Anfrage.
pub fn slots_busy(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .is_some_and(|slots| {
            slots.iter().any(|s| {
                s.get("is_processing").and_then(|b| b.as_bool()).unwrap_or(false)
                    || s.get("state").and_then(|b| b.as_u64()).is_some_and(|n| n != 0)
            })
        })
}

/// Beendet den Chat-Server, wenn er `limit` lang nicht benutzt wurde und
/// nicht rechnet -- RAM und VRAM des Modells werden frei, auch wenn die App
/// nach "Schliessen" im Infobereich weiterlaeuft. Der naechste Aufruf startet
/// ihn neu (`ensure_local`). Liefert, ob gestoppt wurde.
pub async fn stop_local_if_idle(limit: Option<std::time::Duration>) -> bool {
    let Some(server) = SERVER.get() else { return false };
    if limit.is_none() || !server.has_process() {
        return false;
    }
    let last = LOCAL_LAST_USE_MS.load(Ordering::Acquire);
    let idle = std::time::Duration::from_millis(now_ms().saturating_sub(last));
    // Schnellweg ohne Netz: noch nicht lange genug her.
    if !local_idle_expired(idle, limit, false) {
        return false;
    }
    let busy = server_busy(server).await;
    if busy {
        touch_local();
        return false;
    }
    log::info!(
        "Chat-Server: {} s ohne Anfrage, wird beendet (RAM/VRAM frei)",
        idle.as_secs()
    );
    let server = server.clone();
    let _ = tokio::task::spawn_blocking(move || server.stop()).await;
    true
}

/// Rechnet der Chat-Server gerade (`/slots`)? Unklar zaehlt als "nein".
async fn server_busy(server: &LocalLlmServer) -> bool {
    let Some(port) = server.status().port else {
        return false;
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    else {
        return false;
    };
    match client.get(format!("http://127.0.0.1:{port}/slots")).send().await {
        Ok(resp) if resp.status().is_success() => slots_busy(&resp.text().await.unwrap_or_default()),
        _ => false,
    }
}

/// D3: Ende eines Folienauftrags. Laeuft der Chat-Server mit Bild-Projektor,
/// wird er beendet, damit der naechste Chat wieder OHNE die ~0,8 GB Grafikspeicher
/// startet (Neustart bei der naechsten Anfrage). Rechnet er gerade fuer jemand
/// anderen (eine Anfrage hat sich an den Projektor-Server gehaengt), bleibt er: der
/// Leerlauf-Stopp (`stop_local_if_idle`) raeumt ihn spaeter ab. Liefert, ob gestoppt wurde.
pub async fn release_vision() -> bool {
    let Some(server) = SERVER.get() else { return false };
    if !server.has_vision() || server_busy(server).await {
        return false;
    }
    log::info!("Chat-Server: Folienauftrag beendet, Bild-Projektor wird entladen");
    let server = server.clone();
    tokio::task::spawn_blocking(move || {
        // Zwischen Pruefung und Stopp kann ein anderer Start gelaufen sein: nur einen
        // Server MIT Projektor beenden.
        if server.has_vision() {
            server.stop();
        }
    })
    .await
    .is_ok()
}

/// Kontext fuer einen Start, je freiem Grafikspeicher (`context`). Fehler beim
/// Messen fuehren zum Standard, nie zu einem Abbruch.
async fn plan_context(model_path: &std::path::Path, backend: &str) -> u32 {
    let path = model_path.to_path_buf();
    let backend = backend.to_string();
    tokio::task::spawn_blocking(move || context::plan_for_file(&path, &backend))
        .await
        .unwrap_or(DEFAULT_CONTEXT_TOKENS)
}

fn set_active_context(model_id: &str, context_tokens: u32) {
    *ACTIVE_CONTEXT.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some((model_id.to_string(), context_tokens));
}

/// Kontext, mit dem der laufende lokale Server `model_id` bedient. `None`, wenn
/// er nicht laeuft oder ein anderes Modell haelt.
pub fn active_context_tokens(model_id: &str) -> Option<u32> {
    if !SERVER.get().is_some_and(|s| s.has_process()) {
        return None;
    }
    ACTIVE_CONTEXT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|(id, _)| id == model_id)
        .map(|(_, ctx)| *ctx)
}

/// Kontext, den ein Aufruf an `model_id` haben wird: der des laufenden Servers,
/// sonst die Wahl, die ein Start jetzt treffen wuerde. Damit kennt der Aufrufer
/// sein Prompt-Budget schon vor dem (lazy) Serverstart. Ohne Laufzeit oder
/// Modelldatei (Test, unvollstaendige Installation): der Standard.
pub async fn context_for_model(model_id: &str) -> u32 {
    if let Some(ctx) = active_context_tokens(model_id) {
        return ctx;
    }
    let Some(runtime) = RUNTIME.get() else {
        return DEFAULT_CONTEXT_TOKENS;
    };
    let Some(path) = runtime.model_path(model_id).filter(|p| p.is_file()) else {
        return DEFAULT_CONTEXT_TOKENS;
    };
    match runtime.resolve_runtime().await {
        Ok((_, backend, _)) => plan_context(&path, &backend).await,
        Err(_) => DEFAULT_CONTEXT_TOKENS,
    }
}

// ---------------------------------------------------------------------------
// P1i: Token exakt messen (`/tokenize` des llama-servers)
// ---------------------------------------------------------------------------

/// Wurzeladresse des lokalen Servers fuer `model_id` (ohne `/v1`), der dafuer
/// bei Bedarf gestartet wird. `None`, wenn er nicht bereit wird -- der Aufrufer
/// faellt dann auf eine Schaetzung zurueck; den Fehler selbst meldet der erste
/// echte Aufruf (RAM-Gate, Absturz) mit dem richtigen Code.
pub async fn local_server_root(model_id: &str) -> Option<String> {
    let base = ensure_local(model_id).await.ok()?;
    Some(base.trim_end_matches('/').trim_end_matches("/v1").to_string())
}

/// Wartezeit fuer `/tokenize`: ein Aufruf dauert Millisekunden. Haengt der
/// Server, faellt der Aufrufer auf die Schaetzung zurueck, statt zu warten.
const TOKENIZE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Anzahl Token von `text` laut `/tokenize` des Servers unter `root`. `None`
/// bei jedem Problem (Server ohne Endpunkt, Zeitueberschreitung, unbrauchbare
/// Antwort, nicht leerer Text mit null Token): eine falsche Messung ist
/// schlimmer als keine.
pub async fn tokenize_count(root: &str, text: &str) -> Option<usize> {
    if text.is_empty() {
        return Some(0);
    }
    let client = reqwest::Client::builder().timeout(TOKENIZE_TIMEOUT).build().ok()?;
    let response = client
        .post(format!("{}/tokenize", root.trim_end_matches('/')))
        .json(&serde_json::json!({ "content": text, "add_special": false }))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    let count = body.get("tokens")?.as_array()?.len();
    (count > 0).then_some(count)
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
    // Abgestuerzt (oder vom Job-Deckel beendet): sofort abraeumen und den
    // Absturz vormerken, damit der Neustart unter der Ein-Minuten-Sperre
    // steht (P1h) und nie ein toter Port als bereit gilt.
    if server.reap_if_dead() {
        log::warn!("Embedding-Server hat sich beendet, wird neu gestartet");
    }
    if let Some(port) = server.live_port(model_id).await {
        return Ok(format!("http://127.0.0.1:{port}/v1"));
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
                mmproj: None,
            },
            Some(runtime.embed_log_path()),
        )
        .await
        .map_err(|e| {
            if is_server_crashed(&e) {
                // Code bleibt vorn (`server_crashed: ...`); der Text darf den
                // Speicher-Marker des gescheiterten Neustarts enthalten.
                e
            } else if e.contains("Arbeitsspeicher") {
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


    // ---- P1i: /tokenize ---------------------------------------------------

    use crate::managers::meetings::llm_call::test_support::{spawn_llm_mock_with, MockReply};

    /// Ein Server, der wie llama-server auf `/tokenize` antwortet: ein Token je
    /// drei Zeichen.
    async fn tokenize_mock() -> String {
        let port = spawn_llm_mock_with(|body| {
            let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let text = v["content"].as_str().unwrap_or("");
            let tokens: Vec<u32> = (0..text.chars().count().div_ceil(3) as u32).collect();
            MockReply::Body(serde_json::json!({ "tokens": tokens }).to_string())
        })
        .await;
        format!("http://127.0.0.1:{port}")
    }

    #[tokio::test]
    async fn tokenize_count_reads_the_token_array_of_the_server() {
        let root = tokenize_mock().await;
        assert_eq!(tokenize_count(&root, "abcdefghi").await, Some(3));
        assert_eq!(tokenize_count(&format!("{root}/"), "abcd").await, Some(2), "Schraegstrich am Ende");
        assert_eq!(tokenize_count(&root, "").await, Some(0), "leerer Text braucht keinen Aufruf");
    }

    /// Kein Endpunkt, kaputte Antwort, toter Server, null Token fuer echten
    /// Text: immer `None`, nie eine erfundene Zahl.
    #[tokio::test]
    async fn tokenize_count_gives_up_instead_of_guessing() {
        let not_found = spawn_llm_mock_with(|_| MockReply::Status(404)).await;
        assert_eq!(tokenize_count(&format!("http://127.0.0.1:{not_found}"), "text").await, None);

        let garbage = spawn_llm_mock_with(|_| MockReply::Body("kein json".into())).await;
        assert_eq!(tokenize_count(&format!("http://127.0.0.1:{garbage}"), "text").await, None);

        let no_tokens = spawn_llm_mock_with(|_| MockReply::Body(r#"{"tokens":[]}"#.into())).await;
        assert_eq!(tokenize_count(&format!("http://127.0.0.1:{no_tokens}"), "text").await, None);

        let wrong_shape = spawn_llm_mock_with(|_| MockReply::Body(r#"{"tokens":"x"}"#.into())).await;
        assert_eq!(tokenize_count(&format!("http://127.0.0.1:{wrong_shape}"), "text").await, None);

        // Nichts lauscht auf dem Port (Server abgestuerzt).
        let dead = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        assert_eq!(tokenize_count(&format!("http://127.0.0.1:{dead}"), "text").await, None);
    }

    /// Ohne Laufzeit startet kein Server: die Wurzel gibt es nicht, der Aufrufer
    /// schaetzt.
    #[tokio::test]
    async fn the_server_root_is_missing_without_a_runtime() {
        assert_eq!(local_server_root("llm-qwen3-4b-q4").await, None);
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

    /// P1h: Der Fehlercode eines abgestuerzten Servers steht vorn und ist
    /// eindeutig erkennbar; Praefix allein oder ein Fremdtext zaehlt nicht.
    #[test]
    fn a_crashed_server_error_is_recognised_by_its_code() {
        assert!(is_server_crashed("server_crashed: Das lokale Sprachmodell ist abgestuerzt."));
        assert!(is_server_crashed(&format!("{CODE_SERVER_CRASHED}: x")));
        assert!(!is_server_crashed("server_crashed_at_all"));
        assert!(!is_server_crashed("failed: server_crashed: x"));
        assert!(!is_server_crashed("HTTP request failed: connection refused"));
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

#[cfg(test)]
mod idle_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn idle_limit_follows_the_unload_setting_but_never_undercuts_two_minutes() {
        assert_eq!(local_idle_limit(None), None, "Nie entladen = nie stoppen");
        assert_eq!(local_idle_limit(Some(0)), Some(LOCAL_MIN_IDLE), "\"sofort\" gilt nicht fuer das Sprachmodell");
        assert_eq!(local_idle_limit(Some(15)), Some(LOCAL_MIN_IDLE));
        assert_eq!(local_idle_limit(Some(300)), Some(Duration::from_secs(300)));
        assert_eq!(local_idle_limit(Some(3600)), Some(Duration::from_secs(3600)));
    }

    #[test]
    fn idle_stop_needs_a_limit_enough_idle_time_and_no_running_request() {
        let limit = Some(Duration::from_secs(300));
        assert!(local_idle_expired(Duration::from_secs(300), limit, false));
        assert!(local_idle_expired(Duration::from_secs(4000), limit, false));
        assert!(!local_idle_expired(Duration::from_secs(299), limit, false));
        // Rechnet der Server, wird er nie weggenommen, egal wie lange es her ist.
        assert!(!local_idle_expired(Duration::from_secs(4000), limit, true));
        // Ohne Grenze nie.
        assert!(!local_idle_expired(Duration::from_secs(99_999), None, false));
    }

    #[test]
    fn slots_report_is_read_defensively() {
        assert!(slots_busy(r#"[{"id":0,"is_processing":false},{"id":1,"is_processing":true}]"#));
        assert!(!slots_busy(r#"[{"id":0,"is_processing":false},{"id":1,"is_processing":false}]"#));
        assert!(slots_busy(r#"[{"id":0,"state":1}]"#));
        assert!(!slots_busy(r#"[{"id":0,"state":0}]"#));
        // Kein Endpunkt / Fehlertext / leer: nicht "beschaeftigt" (Zeit entscheidet).
        assert!(!slots_busy(""));
        assert!(!slots_busy("not json"));
        assert!(!slots_busy(r#"{"error":"slots disabled"}"#));
        assert!(!slots_busy("[]"));
    }

    #[test]
    fn touching_moves_the_last_use_forward() {
        LOCAL_LAST_USE_MS.store(0, Ordering::Release);
        touch_local();
        assert!(LOCAL_LAST_USE_MS.load(Ordering::Acquire) > 0);
    }
}
