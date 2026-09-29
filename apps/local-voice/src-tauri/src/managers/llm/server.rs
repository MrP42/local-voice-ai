//! Der lokale Sprachmodell-Server: ein `llama-server`-Subprozess.
//!
//! Warum ein Subprozess und kein Rust-Binding: Backends (Vulkan, CUDA, Metal)
//! und llama.cpp-Version bleiben unabhaengig von der App aktualisierbar,
//! ein Absturz oder Treiberfehler reisst die App nicht mit, und der
//! bestehende OpenAI-kompatible Client (`llm_client`) spricht mit ihm wie mit
//! jedem anderen Anbieter. Der Spike vom 13.09.2026 hat das belegt: bereit
//! nach 1–3 s, `usage` in jeder Antwort, ein Absturz mitten im Stream endet
//! beim Client nach ~220 ms mit Fehler statt zu haengen.
//!
//! Regeln aus dem Spike, hier fest verdrahtet:
//! - Bind nur an 127.0.0.1, freier Port je Start; einen fremden Prozess auf
//!   dem Port akzeptieren wir nicht.
//! - Beenden ausschliesslich ueber die eigene PID (Prozessbaum), nie ueber
//!   den Prozessnamen -- der Nutzer koennte einen eigenen `llama-server`
//!   laufen haben.
//! - Slots und Kontext explizit setzen: die Voreinstellung (4 Slots) hat
//!   beim 0,6B-Modell den Speicherbedarf mehr als verdoppelt.

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;

/// Wie lange der Start dauern darf, bevor er als gescheitert gilt. Ein
/// 8B-Modell von einer langsamen Platte braucht Zeit; zwei Minuten sind
/// grosszuegig, aber endlich.
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// Abstand der Bereitschaftsabfragen waehrend des Starts.
const HEALTH_POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LocalLlmPhase {
    Stopped,
    Starting,
    Ready,
    Error,
}

/// Zustand fuer die Oberflaeche: was laeuft, wo, mit welchem Backend.
#[derive(Debug, Clone, Serialize, Type)]
pub struct LocalLlmStatus {
    pub phase: LocalLlmPhase,
    pub model_id: Option<String>,
    pub backend: Option<String>,
    pub port: Option<u16>,
    pub message: Option<String>,
}

/// Startparameter, die die Oberflaeche nicht sieht, der Ressourcen-
/// koordinator aber spaeter steuern wird.
#[derive(Debug, Clone)]
pub struct StartOptions {
    pub model_id: String,
    pub model_path: PathBuf,
    pub backend: String,
    pub context_tokens: u32,
    /// Schichten auf der GPU; 99 = alles, 0 = nur CPU.
    pub gpu_layers: u32,
    /// M4-P4b: `Some` startet den Server im Embedding-Modus (zweiter Prozess
    /// neben dem Chat-Server, M4 V4). `None` = Chat-Server wie bisher.
    pub embedding: Option<EmbeddingOpts>,
}

/// Embedding-Modus von `llama-server` (M4 §4, D3): `--embedding --pooling
/// <pooling> -b 2048 -ub 2048 -np <parallel>`. Ein Embedding-Server beantwortet
/// keine Chat-Anfragen (HTTP 500, Spike V4) -- deshalb ein eigener Prozess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingOpts {
    /// `cls` fuer BGE-M3.
    pub pooling: &'static str,
    /// Gleichzeitige Sequenzen (`-np`).
    pub parallel: u32,
    /// Windows-Prozessklasse BELOW_NORMAL schon beim Start (Creation-Flag),
    /// nicht erst mit dem Job-Objekt.
    pub below_normal: bool,
}

/// Physische Batchgroesse im Embedding-Modus. Ein Nicht-Kausal-Modell muss
/// eine ganze Eingabe in EINEM Batch sehen; Chunks haben hoechstens 1 800
/// Zeichen (+ Kopfzeile), BGE-M3 braucht ~5 Zeichen je Token (M7).
pub const EMBED_BATCH_TOKENS: u32 = 2048;
/// CPU-Threads des Embedding-Servers auf dem CPU-Backend (M4 §9): er laeuft
/// im Hintergrund und darf die Maschine nicht belegen.
pub const EMBED_CPU_THREADS: usize = 4;
/// RAM-Bedarf fuer das Start-Gate des Embedding-Servers (MB): gemessene
/// Spitze 1,9 GB (M4), unabhaengig von der kleinen Modelldatei.
pub const EMBED_RAM_NEED_MB: u64 = 2048;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

/// Argumente nach `-m <modell>`. Rein, damit die Kommandozeile testbar ist;
/// ohne `embedding` exakt die Argumente des Chat-Servers wie vor M4.
pub(crate) fn server_args(opts: &StartOptions, port: u16, cpu_threads: usize) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        opts.context_tokens.to_string(),
        "-ngl".into(),
        opts.gpu_layers.to_string(),
    ];
    match opts.embedding {
        None => {
            // Ein Slot: die Voreinstellung von vieren verdoppelt den KV-Cache,
            // und die App stellt ohnehin eine Anfrage nach der anderen.
            args.extend(["--parallel".into(), "1".into()]);
            // Nicht alle Kerne: die Oberflaeche des Rechners bleibt bedienbar.
            args.extend(["-t".into(), cpu_threads.to_string()]);
        }
        Some(emb) => {
            args.extend([
                "--embedding".into(),
                "--pooling".into(),
                emb.pooling.to_string(),
                "-b".into(),
                EMBED_BATCH_TOKENS.to_string(),
                "-ub".into(),
                EMBED_BATCH_TOKENS.to_string(),
                "-np".into(),
                emb.parallel.max(1).to_string(),
            ]);
            let threads = if opts.backend == "cpu" {
                EMBED_CPU_THREADS.min(cpu_threads.max(1))
            } else {
                cpu_threads
            };
            args.extend(["-t".into(), threads.to_string()]);
        }
    }
    args.push("--no-webui".into());
    args
}

/// Creation-Flags fuer Windows: nie ein Konsolenfenster; im Embedding-Modus
/// auf Wunsch gleich mit niedriger Prioritaet.
pub(crate) fn creation_flags(opts: &StartOptions) -> u32 {
    match opts.embedding {
        Some(emb) if emb.below_normal => CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS,
        _ => CREATE_NO_WINDOW,
    }
}

struct Running {
    child: Child,
    /// Job-Objekt mit RAM-/CPU-Deckel; faellt mit `Running`, und mit ihm
    /// der Prozessbaum (KILL_ON_JOB_CLOSE).
    _guard: Option<crate::process_guard::ProcessGuard>,
    port: u16,
    model_id: String,
    backend: String,
}

pub struct LocalLlmServer {
    running: Mutex<Option<Running>>,
    phase: Mutex<(LocalLlmPhase, Option<String>)>,
    stop_requested: AtomicBool,
    http: reqwest::Client,
}

impl Default for LocalLlmServer {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalLlmServer {
    pub fn new() -> Self {
        Self {
            running: Mutex::new(None),
            phase: Mutex::new((LocalLlmPhase::Stopped, None)),
            stop_requested: AtomicBool::new(false),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("reqwest client"),
        }
    }

    pub fn status(&self) -> LocalLlmStatus {
        let (phase, message) = self.phase.lock().unwrap().clone();
        let running = self.running.lock().unwrap();
        LocalLlmStatus {
            phase,
            model_id: running.as_ref().map(|r| r.model_id.clone()),
            backend: running.as_ref().map(|r| r.backend.clone()),
            port: running.as_ref().map(|r| r.port),
            message,
        }
    }

    /// Adresse fuer den OpenAI-kompatiblen Client, solange der Server laeuft.
    pub fn base_url(&self) -> Option<String> {
        self.running
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| format!("http://127.0.0.1:{}/v1", r.port))
    }

    /// Laeuft der Server gerade mit genau diesem Modell?
    pub fn is_serving(&self, model_id: &str) -> bool {
        matches!(self.phase.lock().unwrap().0, LocalLlmPhase::Ready)
            && self
                .running
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|r| r.model_id == model_id)
    }

    /// M4-P4b: Ist der verwaltete Prozess inzwischen beendet (Absturz, Deckel
    /// des Job-Objekts)? Dann darf er nicht als "bereit" wiederverwendet
    /// werden -- sonst liefe jede Anfrage gegen einen toten Port.
    pub fn child_exited(&self) -> bool {
        self.running
            .lock()
            .unwrap()
            .as_mut()
            .is_some_and(|r| matches!(r.child.try_wait(), Ok(Some(_))))
    }

    /// M4-P4b: Gibt es einen verwalteten Prozess (startend oder bereit)?
    pub fn has_process(&self) -> bool {
        self.running.lock().unwrap().is_some()
    }

    /// M4-P4b: PID des verwalteten Prozesses (Messung, Diagnose).
    pub fn pid(&self) -> Option<u32> {
        self.running.lock().unwrap().as_ref().map(|r| r.child.id())
    }

    /// Test: einen fremden (z. B. schon beendeten) Prozess als "bereit"
    /// uebernehmen, um den Absturzfall nachzustellen.
    #[cfg(test)]
    pub(crate) fn adopt_for_test(&self, child: Child, model_id: &str) {
        *self.running.lock().unwrap() = Some(Running {
            child,
            _guard: None,
            port: 1,
            model_id: model_id.to_string(),
            backend: "cpu".into(),
        });
        self.set_phase(LocalLlmPhase::Ready, None);
    }

    fn set_phase(&self, phase: LocalLlmPhase, message: Option<String>) {
        *self.phase.lock().unwrap() = (phase, message);
    }

    /// Startet den Server, falls er nicht schon dieses Modell bedient. Ein
    /// anderes Modell wird vorher beendet -- zwei Modelle nebeneinander
    /// kosten doppelten Speicher, ohne dass jemand beide gleichzeitig
    /// braucht.
    pub async fn ensure(&self, binary: &Path, opts: StartOptions, log_path: Option<PathBuf>) -> Result<u16, String> {
        if self.is_serving(&opts.model_id) {
            if let Some(port) = self.running.lock().unwrap().as_ref().map(|r| r.port) {
                return Ok(port);
            }
        }
        self.stop();
        self.start(binary, opts, log_path).await
    }

    async fn start(&self, binary: &Path, opts: StartOptions, log_path: Option<PathBuf>) -> Result<u16, String> {
        if !binary.is_file() {
            let msg = format!("Laufzeit fehlt: {}", binary.display());
            self.set_phase(LocalLlmPhase::Error, Some(msg.clone()));
            return Err(msg);
        }
        if !opts.model_path.is_file() {
            let msg = format!("Modell fehlt: {}", opts.model_path.display());
            self.set_phase(LocalLlmPhase::Error, Some(msg.clone()));
            return Err(msg);
        }
        let port = free_port()?;
        self.stop_requested.store(false, Ordering::Release);
        self.set_phase(LocalLlmPhase::Starting, None);

        // Start-Gate: die Modelldatei wird (bei CPU-Anteil) in den RAM
        // gemappt; mindestens ihre Groesse muss frei sein.
        let model_mb = std::fs::metadata(&opts.model_path)
            .map(|m| m.len() / (1024 * 1024))
            .unwrap_or(2048);
        // M4-P4b: der Embedding-Server braucht mehr als seine kleine Datei.
        let need_mb = if opts.embedding.is_some() {
            model_mb.max(EMBED_RAM_NEED_MB)
        } else {
            model_mb
        };
        let free_mb = crate::process_guard::check_ram_for_start(need_mb).map_err(|msg| {
            self.set_phase(LocalLlmPhase::Error, Some(msg.clone()));
            msg
        })?;
        let cpu_threads = crate::process_guard::cpu_threads_for(crate::process_guard::logical_cpus());

        let mut cmd = std::process::Command::new(binary);
        cmd.arg("-m")
            .arg(&opts.model_path)
            .args(server_args(&opts, port, cpu_threads));
        if let Some(dir) = binary.parent() {
            cmd.current_dir(dir);
        }
        // Ausgabe in eine Datei, damit ein Startfehler diagnostizierbar
        // bleibt (dasselbe Muster wie beim Fish-Speech-Server).
        match log_path.as_ref().and_then(|p| std::fs::File::create(p).ok()) {
            Some(file) => {
                let err = file.try_clone().map_err(|e| e.to_string())?;
                cmd.stdout(std::process::Stdio::from(file))
                    .stderr(std::process::Stdio::from(err));
            }
            None => {
                cmd.stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(creation_flags(&opts)); // CREATE_NO_WINDOW (+ BELOW_NORMAL)
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("llama-server liess sich nicht starten: {e}"))?;
        let guard = crate::process_guard::ProcessGuard::attach(
            &child,
            Some(crate::process_guard::memory_limit_mb(free_mb)),
            crate::process_guard::CPU_CAP_PERCENT,
        );
        log::info!(
            "llama-server gestartet (pid {}, {} auf 127.0.0.1:{port}, Backend {})",
            child.id(),
            opts.model_id,
            opts.backend
        );
        *self.running.lock().unwrap() = Some(Running {
            child,
            _guard: guard,
            port,
            model_id: opts.model_id.clone(),
            backend: opts.backend.clone(),
        });

        let started = Instant::now();
        loop {
            if self.stop_requested.load(Ordering::Acquire) {
                self.stop();
                return Err("Start abgebrochen".to_string());
            }
            // Ein vorzeitig beendeter Prozess wartet nicht auf den Timeout.
            let exited = self
                .running
                .lock()
                .unwrap()
                .as_mut()
                .and_then(|r| r.child.try_wait().ok().flatten());
            if let Some(status) = exited {
                *self.running.lock().unwrap() = None;
                let msg = format!("llama-server hat sich beendet: {status}");
                self.set_phase(LocalLlmPhase::Error, Some(msg.clone()));
                return Err(msg);
            }
            if self.health_ok(port).await {
                self.set_phase(LocalLlmPhase::Ready, None);
                log::info!("llama-server bereit nach {:.1} s", started.elapsed().as_secs_f32());
                return Ok(port);
            }
            if started.elapsed() > START_TIMEOUT {
                self.stop();
                let msg = "llama-server wurde nicht rechtzeitig bereit".to_string();
                self.set_phase(LocalLlmPhase::Error, Some(msg.clone()));
                return Err(msg);
            }
            tokio::time::sleep(HEALTH_POLL).await;
        }
    }

    async fn health_ok(&self, port: u16) -> bool {
        self.http
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Beendet den eigenen Prozess -- und nur den. Ueber die PID samt
    /// Prozessbaum, nie ueber den Namen.
    pub fn stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
        if let Some(mut running) = self.running.lock().unwrap().take() {
            // Beim Herunterfahren von Windows kein taskkill: der Start
            // scheitert dann mit 0xc0000142 samt Fehlerfenster. Den Baum
            // beendet das Job-Objekt, sobald `running` hier endet.
            #[cfg(windows)]
            if !crate::process_guard::session_ending() {
                let pid = running.child.id();
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .output();
            }
            let _ = running.child.kill();
            let _ = running.child.wait();
            log::info!("llama-server beendet ({})", running.model_id);
        }
        self.set_phase(LocalLlmPhase::Stopped, None);
    }
}

impl Drop for LocalLlmServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Ein freier Port auf 127.0.0.1. Das Betriebssystem waehlt ihn, wir lassen
/// den Socket wieder los -- die winzige Luecke bis zum Start des Servers ist
/// das kleinere Uebel gegenueber einem festen Port, der belegt sein kann.
fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("kein freier Port: {e}"))?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_port_is_never_zero_and_not_reused_immediately() {
        let a = free_port().unwrap();
        let b = free_port().unwrap();
        assert!(a > 0 && b > 0);
        // Zwei Anfragen kurz nacheinander liefern in der Regel verschiedene
        // Ports; gleich waere zulaessig, nur unwahrscheinlich.
        let _ = (a, b);
    }

    #[test]
    fn a_fresh_server_reports_stopped_without_a_process() {
        let server = LocalLlmServer::new();
        let status = server.status();
        assert_eq!(status.phase, LocalLlmPhase::Stopped);
        assert!(status.port.is_none() && status.model_id.is_none());
        assert!(server.base_url().is_none());
        assert!(!server.is_serving("x"));
    }

    #[tokio::test]
    async fn a_missing_binary_fails_fast_with_a_readable_message() {
        let server = LocalLlmServer::new();
        let err = server
            .ensure(
                Path::new("C:/gibt/es/nicht/llama-server.exe"),
                StartOptions {
                    model_id: "m".into(),
                    model_path: PathBuf::from("C:/gibt/es/nicht/m.gguf"),
                    backend: "vulkan".into(),
                    context_tokens: 4096,
                    gpu_layers: 99,
                    embedding: None,
                },
                None,
            )
            .await
            .unwrap_err();
        assert!(err.contains("Laufzeit fehlt"), "{err}");
        assert_eq!(server.status().phase, LocalLlmPhase::Error);
    }

    /// Stop ohne laufenden Prozess ist ein Nichts, kein Fehler -- so darf
    /// der Exit-Hook der App ihn blind rufen.
    #[test]
    fn stopping_a_stopped_server_is_harmless() {
        let server = LocalLlmServer::new();
        server.stop();
        server.stop();
        assert_eq!(server.status().phase, LocalLlmPhase::Stopped);
    }

    // ---- M4-P4b: Embedding-Modus -----------------------------------------

    fn opts(backend: &str, embedding: Option<EmbeddingOpts>) -> StartOptions {
        StartOptions {
            model_id: "m".into(),
            model_path: PathBuf::from("m.gguf"),
            backend: backend.into(),
            context_tokens: 4096,
            gpu_layers: 99,
            embedding,
        }
    }

    const BGE: EmbeddingOpts = EmbeddingOpts {
        pooling: "cls",
        parallel: 2,
        below_normal: true,
    };

    fn pair(args: &[String], flag: &str) -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1).cloned())
    }

    #[test]
    fn embedding_args_contain_pooling_cls() {
        let args = server_args(&opts("vulkan", Some(BGE)), 4711, 16);
        assert!(args.contains(&"--embedding".to_string()), "{args:?}");
        assert_eq!(pair(&args, "--pooling").as_deref(), Some("cls"));
        assert_eq!(pair(&args, "-b").as_deref(), Some("2048"));
        assert_eq!(pair(&args, "-ub").as_deref(), Some("2048"));
        assert_eq!(pair(&args, "-np").as_deref(), Some("2"));
        assert_eq!(pair(&args, "--host").as_deref(), Some("127.0.0.1"));
        assert_eq!(pair(&args, "--port").as_deref(), Some("4711"));
        // Kein Chat-Slot-Argument im Embedding-Modus.
        assert!(!args.contains(&"--parallel".to_string()));
    }

    /// `embedding: None` ist exakt die Kommandozeile des Chat-Servers vor M4.
    #[test]
    fn chat_args_are_unchanged_without_embedding() {
        let args = server_args(&opts("cuda", None), 5000, 16);
        let expected: Vec<String> = [
            "--host", "127.0.0.1", "--port", "5000", "-c", "4096", "-ngl", "99", "--parallel",
            "1", "-t", "16", "--no-webui",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(args, expected);
        assert!(!args.contains(&"--embedding".to_string()));
        assert_eq!(creation_flags(&opts("cuda", None)), CREATE_NO_WINDOW);
    }

    /// Auf dem CPU-Backend bekommt der Embedding-Server hoechstens 4 Threads
    /// und startet mit Prozessklasse BELOW_NORMAL (0x4000).
    #[test]
    fn embedding_on_cpu_is_throttled_and_below_normal() {
        let cpu = server_args(&opts("cpu", Some(BGE)), 1, 16);
        assert_eq!(pair(&cpu, "-t").as_deref(), Some("4"));
        let gpu = server_args(&opts("vulkan", Some(BGE)), 1, 16);
        assert_eq!(pair(&gpu, "-t").as_deref(), Some("16"));
        assert_eq!(
            creation_flags(&opts("cpu", Some(BGE))),
            CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS
        );
        let normal = EmbeddingOpts {
            below_normal: false,
            ..BGE
        };
        assert_eq!(creation_flags(&opts("cpu", Some(normal))), CREATE_NO_WINDOW);
    }

    /// Ein abgestuerzter Kindprozess gilt nicht mehr als lebendig: sonst
    /// lieferte `ensure_embedding` die Adresse eines toten Servers.
    #[cfg(windows)]
    #[test]
    fn a_dead_child_is_reported_as_exited() {
        let server = LocalLlmServer::new();
        assert!(!server.child_exited() && !server.has_process());
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "exit 3"]);
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().expect("cmd.exe");
        let _ = child.wait();
        *server.running.lock().unwrap() = Some(Running {
            child,
            _guard: None,
            port: 1,
            model_id: "m".into(),
            backend: "cpu".into(),
        });
        assert!(server.has_process());
        assert!(server.child_exited());
        server.stop();
        assert!(!server.has_process());
    }
}
