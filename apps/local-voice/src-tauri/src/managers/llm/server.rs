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
        let free_mb = crate::process_guard::check_ram_for_start(model_mb).map_err(|msg| {
            self.set_phase(LocalLlmPhase::Error, Some(msg.clone()));
            msg
        })?;
        let cpu_threads = crate::process_guard::cpu_threads_for(crate::process_guard::logical_cpus());

        let mut cmd = std::process::Command::new(binary);
        cmd.arg("-m")
            .arg(&opts.model_path)
            .args(["--host", "127.0.0.1"])
            .args(["--port", &port.to_string()])
            .args(["-c", &opts.context_tokens.to_string()])
            .args(["-ngl", &opts.gpu_layers.to_string()])
            // Ein Slot: die Voreinstellung von vieren verdoppelt den KV-Cache,
            // und die App stellt ohnehin eine Anfrage nach der anderen.
            .args(["--parallel", "1"])
            // Nicht alle Kerne: die Oberflaeche des Rechners bleibt bedienbar.
            .args(["-t", &cpu_threads.to_string()])
            .arg("--no-webui");
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
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("llama-server liess sich nicht starten: {e}"))?;
        let guard = crate::process_guard::ProcessGuard::attach(
            &child,
            crate::process_guard::memory_limit_mb(free_mb),
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
            #[cfg(windows)]
            {
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
}
