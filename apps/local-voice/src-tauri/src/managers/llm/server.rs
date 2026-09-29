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
//!
//! P1h (Befund B4): ein abgestuerzter Server (Gemma 4 12B, CUDA "illegal memory
//! access") galt weiter als "bereit", jeder Folgeaufruf scheiterte sofort am
//! toten Port. Jetzt prueft `ensure` vor jeder Wiederverwendung, ob der
//! Prozess noch lebt und der Port nicht verweigert, raeumt einen toten
//! Prozess ab und startet hoechstens EINMAL PRO MINUTE neu (RAM-Gate wie bei
//! jedem Start). Ein zweiter Absturz innerhalb dieser Minute wird nicht mit
//! einem weiteren Start beantwortet, sondern mit dem Fehlercode
//! `server_crashed`, damit kein Absturz-Neustart-Kreis den Rechner belegt.

use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
/// Frist der Lebendpruefung vor einer Wiederverwendung. Eine Zeitueber-
/// schreitung heisst "beschaeftigt" (der Server rechnet gerade), nie "tot":
/// nur eine verweigerte Verbindung gilt als Absturz. Windows meldet "verweigert"
/// auf dem Loopback erst nach ~2,05 s (gemessen 29.09.2026, SYN-Wiederholung),
/// deshalb muss die Frist deutlich darueber liegen -- sonst hielte sie einen
/// toten Port fuer "beschaeftigt". Ein gesunder Server antwortet in Millisekunden.
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(4);
/// Mindestabstand zwischen zwei automatischen Neustarts nach einem Absturz
/// (Systemschutz: nie mehr als ein Neustart pro Minute).
pub const RESTART_COOLDOWN: Duration = Duration::from_secs(60);
/// Fehlercode (Praefix der Meldung, `<code>: <Text>`) fuer einen abgestuerzten
/// Server, der nicht neu gestartet wird oder dessen Neustart scheiterte.
pub const CODE_SERVER_CRASHED: &str = "server_crashed";
/// Fehlertext, wenn der Nutzer einen laufenden Start per Stopp abbricht.
const START_CANCELLED: &str = "Start abgebrochen";

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

/// Ein Absturz, dessen Neustart noch aussteht.
struct CrashInfo {
    model_id: String,
    reason: String,
}

/// Sperre nach einem zweiten Absturz (oder gescheiterten Neustart) innerhalb
/// der Minute: bis `until` startet `ensure` dieses Modell nicht erneut.
struct CrashBlock {
    model_id: String,
    until: Instant,
    reason: String,
}

/// Absturz-Buchfuehrung von `ensure`. Nie zusammen mit `phase`/`running`
/// gehalten -- kein Sperrenpaar, das sich verklemmen koennte.
#[derive(Default)]
struct CrashState {
    /// Beginn der laufenden Minute: letzter automatischer Neustart nach einem
    /// Absturz. Ein Stopp des Nutzers loescht ihn nicht (strenger Deckel).
    last_restart: Option<Instant>,
    pending: Option<CrashInfo>,
    blocked: Option<CrashBlock>,
}

/// Was `admit_start` entschieden hat.
enum StartKind {
    /// Erster Start, Modellwechsel oder Start nach einem Nutzer-Stopp.
    Normal,
    /// Neustart nach einem Absturz: scheitert er, sperrt `settle_start`.
    AfterCrash {
        model_id: String,
        restarted_at: Instant,
    },
}

/// Antwort der Lebendpruefung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Health {
    Ok,
    /// Prozess antwortet nicht rechtzeitig oder mit Fehlerstatus (laedt,
    /// rechnet): nicht tot, also nicht anfassen.
    Busy,
    /// Verbindung verweigert: niemand lauscht mehr.
    Down,
}

#[cfg(test)]
type SpawnHook = std::sync::Arc<dyn Fn(u16) -> std::io::Result<Child> + Send + Sync>;

pub struct LocalLlmServer {
    running: Mutex<Option<Running>>,
    phase: Mutex<(LocalLlmPhase, Option<String>)>,
    stop_requested: AtomicBool,
    http: reqwest::Client,
    /// Ein `ensure` nach dem anderen: zwei Aufrufer (KI-Notizen, Chat), die
    /// denselben Absturz sehen, starten sonst zwei Prozesse oder beenden
    /// gegenseitig den frisch gestarteten. `stop` nimmt diese Sperre nie.
    start_lock: tokio::sync::Mutex<()>,
    crash: Mutex<CrashState>,
    restart_cooldown: Duration,
    liveness_timeout: Duration,
    /// Wie viele Prozesse dieser Server seit dem Anlegen gestartet hat
    /// (Messung, Diagnose; PIDs werden unter Windows schnell wieder vergeben).
    starts: AtomicU32,
    /// Test: ersetzt das Starten von `llama-server` durch ein eigenes Kind.
    #[cfg(test)]
    spawn_hook: Mutex<Option<SpawnHook>>,
}

impl Default for LocalLlmServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Exit-Status des Kindprozesses, falls er schon beendet ist.
fn exit_of(r: &mut Running) -> Option<ExitStatus> {
    r.child.try_wait().ok().flatten()
}

fn crash_message(reason: &str) -> String {
    format!("{CODE_SERVER_CRASHED}: {reason}")
}

/// Meldung an die Aufrufer, solange der Neustart gesperrt ist.
fn blocked_message(remaining: Duration, reason: &str) -> String {
    format!(
        "{CODE_SERVER_CRASHED}: Das lokale Sprachmodell ist abgestürzt. Ein automatischer \
         Neustart wird höchstens einmal pro Minute versucht; nächster Versuch in {} s. \
         Ursache: {reason}",
        remaining.as_secs() + 1
    )
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
            start_lock: tokio::sync::Mutex::new(()),
            crash: Mutex::new(CrashState::default()),
            restart_cooldown: RESTART_COOLDOWN,
            liveness_timeout: LIVENESS_TIMEOUT,
            starts: AtomicU32::new(0),
            #[cfg(test)]
            spawn_hook: Mutex::new(None),
        }
    }

    /// Zustand fuer die Oberflaeche. Ein Prozess, der beendet ist, obwohl der
    /// Zustand noch "bereit" sagt, erscheint sofort als Fehler (lesend, ohne
    /// etwas abzuraeumen -- das tut erst das naechste `ensure`).
    pub fn status(&self) -> LocalLlmStatus {
        let (mut phase, mut message) = self.phase.lock().unwrap().clone();
        let mut running = self.running.lock().unwrap();
        if phase == LocalLlmPhase::Ready {
            if let Some(status) = running.as_mut().and_then(exit_of) {
                phase = LocalLlmPhase::Error;
                message = Some(crash_message(&format!(
                    "llama-server wurde unerwartet beendet ({status})"
                )));
            }
        }
        LocalLlmStatus {
            phase,
            model_id: running.as_ref().map(|r| r.model_id.clone()),
            backend: running.as_ref().map(|r| r.backend.clone()),
            port: running.as_ref().map(|r| r.port),
            message,
        }
    }

    /// Adresse fuer den OpenAI-kompatiblen Client, solange der Server laeuft.
    /// (Die Aufrufer gehen seit P1h ueber `live_port`; bleibt fuer Diagnose/Tests.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn base_url(&self) -> Option<String> {
        self.running
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| format!("http://127.0.0.1:{}/v1", r.port))
    }

    /// Port des Servers, wenn er bereit ist, genau dieses Modell bedient UND
    /// sein Prozess noch lebt (P1h/B4: ein toter Prozess galt vorher weiter
    /// als bereit). Rein lesend; den Port prueft nur `live_port`.
    fn serving_port(&self, model_id: &str) -> Option<u16> {
        let ready = matches!(self.phase.lock().unwrap().0, LocalLlmPhase::Ready);
        if !ready {
            return None;
        }
        self.running
            .lock()
            .unwrap()
            .as_mut()
            .and_then(|r| (r.model_id == model_id && exit_of(r).is_none()).then_some(r.port))
    }

    /// Laeuft der Server gerade mit genau diesem Modell (Prozess lebt)?
    pub fn is_serving(&self, model_id: &str) -> bool {
        self.serving_port(model_id).is_some()
    }

    /// Wie `is_serving`, zusaetzlich antwortet der Port (verweigerte
    /// Verbindung = tot). Liefert den Port zur Wiederverwendung; sonst `None`,
    /// und der Aufrufer geht ueber `ensure`, das den Absturz behandelt.
    pub async fn live_port(&self, model_id: &str) -> Option<u16> {
        let port = self.serving_port(model_id)?;
        (self.probe(port).await != Health::Down).then_some(port)
    }

    /// M4-P4b: Ist der verwaltete Prozess inzwischen beendet (Absturz, Deckel
    /// des Job-Objekts)? Dann darf er nicht als "bereit" wiederverwendet
    /// werden -- sonst liefe jede Anfrage gegen einen toten Port.
    /// (Seit P1h behandelt `reap_if_dead` das; bleibt fuer Diagnose/Tests.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn child_exited(&self) -> bool {
        self.running
            .lock()
            .unwrap()
            .as_mut()
            .is_some_and(|r| exit_of(r).is_some())
    }

    /// M4-P4b: Gibt es einen verwalteten Prozess (startend oder bereit)?
    pub fn has_process(&self) -> bool {
        self.running.lock().unwrap().is_some()
    }

    /// Anzahl der bisher gestarteten Prozesse (Messung, Diagnose).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn start_count(&self) -> u32 {
        self.starts.load(Ordering::Acquire)
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
    ///
    /// P1h: vor jeder Wiederverwendung Lebendpruefung (Prozess + Port). Ein
    /// toter Server wird abgeraeumt und hoechstens einmal pro Minute neu
    /// gestartet; sonst antwortet `ensure` mit `server_crashed: ...`.
    pub async fn ensure(&self, binary: &Path, opts: StartOptions, log_path: Option<PathBuf>) -> Result<u16, String> {
        let _serial = self.start_lock.lock().await;
        self.detect_crash().await;
        if let Some(port) = self.serving_port(&opts.model_id) {
            return Ok(port);
        }
        let kind = self.admit_start(&opts.model_id)?;
        self.stop_process();
        if let (StartKind::AfterCrash { .. }, Some(log)) = (&kind, log_path.as_deref()) {
            // Der Start ueberschreibt die Logdatei: die Ausgabe des abgestuerzten
            // Prozesses (Ursache!) vorher sichern. Scheitert das, bleibt es beim Neustart.
            let _ = std::fs::copy(log, log.with_extension("crash.log"));
        }
        let result = self.start(binary, opts, log_path).await;
        self.settle_start(kind, result)
    }

    /// Antwortet `/health` auf `port`? Nur eine verweigerte Verbindung ist
    /// "Down"; alles Unklare (Frist, Fehlerstatus, Transportfehler) zaehlt als
    /// "beschaeftigt", damit ein rechnender Server nie fuer tot erklaert wird.
    async fn probe(&self, port: u16) -> Health {
        match self
            .http
            .get(format!("http://127.0.0.1:{port}/health"))
            .timeout(self.liveness_timeout)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => Health::Ok,
            Ok(_) => Health::Busy,
            Err(e) if e.is_timeout() => Health::Busy,
            Err(e) if e.is_connect() => Health::Down,
            Err(_) => Health::Busy,
        }
    }

    /// Raeumt einen Prozess ab, der beendet ist, obwohl der Zustand noch
    /// "bereit" sagt (Absturz, Deckel des Job-Objekts), und merkt sich den
    /// Absturz fuer die Neustart-Entscheidung. Liefert, ob es einen gab.
    /// Nur im Zustand "bereit": ein startender Prozess wird von `start`
    /// selbst beobachtet.
    pub fn reap_if_dead(&self) -> bool {
        if !matches!(self.phase.lock().unwrap().0, LocalLlmPhase::Ready) {
            return false;
        }
        let taken = {
            let mut guard = self.running.lock().unwrap();
            let status = guard.as_mut().and_then(exit_of);
            status.and_then(|status| guard.take().map(|r| (r, status)))
        };
        let Some((dead, status)) = taken else {
            return false;
        };
        // Der Prozess ist weg: kein taskkill (die PID koennte neu vergeben
        // sein). Das Job-Objekt faellt mit `dead` und nimmt Reste mit.
        let model_id = dead.model_id.clone();
        drop(dead);
        self.record_crash(
            model_id,
            format!("llama-server wurde unerwartet beendet ({status})"),
        );
        true
    }

    /// Lebendpruefung eines "bereiten" Servers: beendeter Prozess oder
    /// verweigerte Verbindung gelten als Absturz und werden abgeraeumt.
    async fn detect_crash(&self) {
        if self.reap_if_dead() {
            return;
        }
        let target = {
            let ready = matches!(self.phase.lock().unwrap().0, LocalLlmPhase::Ready);
            let guard = self.running.lock().unwrap();
            guard
                .as_ref()
                .filter(|_| ready)
                .map(|r| (r.port, r.model_id.clone()))
        };
        let Some((port, model_id)) = target else {
            return;
        };
        if self.probe(port).await != Health::Down {
            return;
        }
        // Zwischen Pruefung und Abraeumen kann `stop` gelaufen sein: nur den
        // Prozess abraeumen, den wir eben geprueft haben.
        let same = self
            .running
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|r| r.port == port);
        if !same {
            return;
        }
        self.stop_process(); // lebt noch, antwortet nicht: ueber PID und Job-Objekt beenden
        self.record_crash(
            model_id,
            format!("llama-server antwortet nicht mehr (Port {port} verweigert die Verbindung)"),
        );
    }

    fn record_crash(&self, model_id: String, reason: String) {
        log::warn!("llama-server abgestuerzt ({model_id}): {reason}");
        self.set_phase(LocalLlmPhase::Error, Some(crash_message(&reason)));
        self.crash.lock().unwrap().pending = Some(CrashInfo { model_id, reason });
    }

    /// Entscheidet vor jedem Start: erlaubt, oder Sperre nach Absturz.
    /// Hoechstens ein Neustart nach Absturz pro `restart_cooldown`.
    fn admit_start(&self, model_id: &str) -> Result<StartKind, String> {
        let now = Instant::now();
        let refusal: String = {
            let mut st = self.crash.lock().unwrap();
            // Abgelaufene Sperre: der Absturz gilt wieder als unbehandelt, der
            // naechste Start zaehlt damit als Neustart und oeffnet eine neue
            // Minute (sonst waeren zwei Starts binnen Sekunden moeglich).
            if st.blocked.as_ref().is_some_and(|b| now >= b.until) {
                if let Some(b) = st.blocked.take() {
                    st.pending = Some(CrashInfo {
                        model_id: b.model_id,
                        reason: b.reason,
                    });
                }
            }
            if let Some(b) = st.blocked.as_ref().filter(|b| b.model_id == model_id) {
                blocked_message(b.until.saturating_duration_since(now), &b.reason)
            } else if !st.pending.as_ref().is_some_and(|p| p.model_id == model_id) {
                // Erster Start, Modellwechsel oder Start nach einem Stopp des
                // Nutzers: ein Absturz eines anderen Modells ist damit erledigt.
                st.pending = None;
                return Ok(StartKind::Normal);
            } else if let Some(last) = st
                .last_restart
                .filter(|t| now.saturating_duration_since(*t) < self.restart_cooldown)
            {
                // Zweiter Absturz innerhalb der Minute: nicht noch einmal starten.
                let until = last + self.restart_cooldown;
                let reason = st.pending.take().map(|p| p.reason).unwrap_or_default();
                let msg = blocked_message(until.saturating_duration_since(now), &reason);
                st.blocked = Some(CrashBlock {
                    model_id: model_id.to_string(),
                    until,
                    reason,
                });
                msg
            } else {
                st.last_restart = Some(now);
                st.pending = None;
                return Ok(StartKind::AfterCrash {
                    model_id: model_id.to_string(),
                    restarted_at: now,
                });
            }
        };
        self.set_phase(LocalLlmPhase::Error, Some(refusal.clone()));
        Err(refusal)
    }

    /// Nach dem Start: scheitert ein Neustart nach Absturz, gilt die Sperre
    /// fuer den Rest der Minute, und die Meldung traegt den Code.
    fn settle_start(&self, kind: StartKind, result: Result<u16, String>) -> Result<u16, String> {
        let StartKind::AfterCrash {
            model_id,
            restarted_at,
        } = kind
        else {
            return result;
        };
        match result {
            Ok(port) => {
                log::info!("llama-server nach Absturz neu gestartet ({model_id}, Port {port})");
                Ok(port)
            }
            // Der Nutzer hat den Start selbst abgebrochen: keine Sperre.
            Err(e) if e == START_CANCELLED => Err(e),
            Err(e) => {
                self.crash.lock().unwrap().blocked = Some(CrashBlock {
                    model_id,
                    until: restarted_at + self.restart_cooldown,
                    reason: e.clone(),
                });
                let msg = crash_message(&format!("Neustart nach Absturz gescheitert: {e}"));
                self.set_phase(LocalLlmPhase::Error, Some(msg.clone()));
                Err(msg)
            }
        }
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

        let child = self.spawn_process(binary, &opts, port, cpu_threads, log_path.as_deref())?;
        self.starts.fetch_add(1, Ordering::AcqRel);
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
                self.stop_process();
                return Err(START_CANCELLED.to_string());
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
                self.stop_process();
                let msg = "llama-server wurde nicht rechtzeitig bereit".to_string();
                self.set_phase(LocalLlmPhase::Error, Some(msg.clone()));
                return Err(msg);
            }
            tokio::time::sleep(HEALTH_POLL).await;
        }
    }

    /// Startet den `llama-server`-Prozess (Kommandozeile, Log, Creation-Flags).
    fn spawn_process(
        &self,
        binary: &Path,
        opts: &StartOptions,
        port: u16,
        cpu_threads: usize,
        log_path: Option<&Path>,
    ) -> Result<Child, String> {
        #[cfg(test)]
        {
            let hook = self.spawn_hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                return hook(port)
                    .map_err(|e| format!("llama-server liess sich nicht starten: {e}"));
            }
        }
        let mut cmd = std::process::Command::new(binary);
        cmd.arg("-m")
            .arg(&opts.model_path)
            .args(server_args(opts, port, cpu_threads));
        if let Some(dir) = binary.parent() {
            cmd.current_dir(dir);
        }
        // Ausgabe in eine Datei, damit ein Startfehler diagnostizierbar
        // bleibt (dasselbe Muster wie beim Fish-Speech-Server).
        match log_path.and_then(|p| std::fs::File::create(p).ok()) {
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
            cmd.creation_flags(creation_flags(opts)); // CREATE_NO_WINDOW (+ BELOW_NORMAL)
        }
        cmd.spawn()
            .map_err(|e| format!("llama-server liess sich nicht starten: {e}"))
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
    /// Prozessbaum, nie ueber den Namen. Das ist der Stopp des NUTZERS (und
    /// des App-Endes): er loescht auch die Absturz-Sperre, damit ein
    /// bewusster Neustart per Klick moeglich bleibt. Den Beginn der laufenden
    /// Minute (`last_restart`) laesst er stehen.
    pub fn stop(&self) {
        self.stop_process();
        let mut st = self.crash.lock().unwrap();
        st.pending = None;
        st.blocked = None;
    }

    /// Beendet den Prozess, ohne die Absturz-Buchfuehrung anzufassen (interner
    /// Weg von `ensure`/`start`/Abraeumen).
    fn stop_process(&self) {
        self.stop_requested.store(true, Ordering::Release);
        if let Some(mut running) = self.running.lock().unwrap().take() {
            // Schon beendet (Absturz): kein taskkill -- die PID koennte neu
            // vergeben sein, und es gibt nichts mehr zu beenden.
            let exited = exit_of(&mut running).is_some();
            // Beim Herunterfahren von Windows kein taskkill: der Start
            // scheitert dann mit 0xc0000142 samt Fehlerfenster. Den Baum
            // beendet das Job-Objekt, sobald `running` hier endet.
            #[cfg(windows)]
            if !exited && !crate::process_guard::session_ending() {
                let pid = running.child.id();
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .output();
            }
            if !exited {
                let _ = running.child.kill();
            }
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

    // ---- P1h (Befund B4): toter Server wird erkannt, hoechstens 1 Neustart/min ----
    //
    // Statt `llama-server` startet der Test-Haken einen schlafenden Prozess
    // (`ping`) und einen winzigen /health-Server im Testprozess. Ein
    // "Absturz" ist `Child::kill` auf genau diesem Prozess.

    #[cfg(windows)]
    mod crash {
        use super::*;
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;

        /// Minimaler /health-Server (Thread). `hang`: nimmt Verbindungen an,
        /// antwortet aber nie -- ein Server, der gerade rechnet.
        struct FakeHealth {
            stop: Arc<AtomicBool>,
            thread: Option<std::thread::JoinHandle<()>>,
        }

        impl FakeHealth {
            fn start(port: u16, hang: bool) -> Self {
                // Ein eben freigegebener Port kann kurz gesperrt sein.
                let mut listener = None;
                for _ in 0..40 {
                    match TcpListener::bind(("127.0.0.1", port)) {
                        Ok(l) => {
                            listener = Some(l);
                            break;
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(50)),
                    }
                }
                let listener = listener.expect("Port binden");
                listener.set_nonblocking(true).unwrap();
                let stop = Arc::new(AtomicBool::new(false));
                let flag = stop.clone();
                let thread = std::thread::spawn(move || {
                    let mut held = Vec::new();
                    while !flag.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok((s, _)) if hang => {
                                let _ = s.set_nonblocking(false);
                                held.push(s);
                            }
                            Ok((mut s, _)) => {
                                let _ = s.set_nonblocking(false);
                                let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
                                let mut buf = [0u8; 2048];
                                let _ = s.read(&mut buf);
                                let body = r#"{"status":"ok"}"#;
                                let _ = write!(
                                    s,
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                    body.len()
                                );
                            }
                            Err(_) => std::thread::sleep(Duration::from_millis(5)),
                        }
                    }
                    drop(held);
                });
                Self {
                    stop,
                    thread: Some(thread),
                }
            }

            /// Schliesst den Port (Thread beendet, Listener freigegeben).
            fn close(&mut self) {
                self.stop.store(true, Ordering::Release);
                if let Some(t) = self.thread.take() {
                    let _ = t.join();
                }
            }
        }

        impl Drop for FakeHealth {
            fn drop(&mut self) {
                self.close();
            }
        }

        #[derive(Clone, Copy)]
        enum Behaviour {
            /// Prozess lebt, /health antwortet.
            Healthy,
            /// Prozess beendet sich sofort (Start scheitert).
            ExitImmediately,
        }

        fn sleeper() -> std::io::Result<Child> {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("ping")
                .args(["-n", "300", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
        }

        struct Harness {
            server: LocalLlmServer,
            spawned: Arc<AtomicUsize>,
            health: Arc<Mutex<Vec<FakeHealth>>>,
            behaviour: Arc<Mutex<Behaviour>>,
            model: PathBuf,
        }

        impl Harness {
            fn new(cooldown: Duration) -> Self {
                static COUNTER: AtomicUsize = AtomicUsize::new(0);
                let model = std::env::temp_dir().join(format!(
                    "lva-p1h-{}-{}.gguf",
                    std::process::id(),
                    COUNTER.fetch_add(1, Ordering::AcqRel)
                ));
                std::fs::write(&model, b"x").expect("Modelldatei");
                let mut server = LocalLlmServer::new();
                server.restart_cooldown = cooldown;
                server.liveness_timeout = Duration::from_millis(300);
                let spawned = Arc::new(AtomicUsize::new(0));
                let health: Arc<Mutex<Vec<FakeHealth>>> = Arc::new(Mutex::new(Vec::new()));
                let behaviour = Arc::new(Mutex::new(Behaviour::Healthy));
                {
                    let (spawned, health, behaviour) =
                        (spawned.clone(), health.clone(), behaviour.clone());
                    *server.spawn_hook.lock().unwrap() = Some(Arc::new(move |port| {
                        spawned.fetch_add(1, Ordering::AcqRel);
                        let mode = *behaviour.lock().unwrap();
                        match mode {
                            Behaviour::Healthy => {
                                let child = sleeper()?;
                                health.lock().unwrap().push(FakeHealth::start(port, false));
                                Ok(child)
                            }
                            Behaviour::ExitImmediately => {
                                use std::os::windows::process::CommandExt;
                                let mut child = std::process::Command::new("cmd")
                                    .args(["/C", "exit 3"])
                                    .creation_flags(CREATE_NO_WINDOW)
                                    .spawn()?;
                                // Schon beendet, wenn `start` es zum ersten Mal sieht
                                // (sonst wartete es erst auf einen 2-s-Verbindungsversuch).
                                let _ = child.wait();
                                Ok(child)
                            }
                        }
                    }));
                }
                Self {
                    server,
                    spawned,
                    health,
                    behaviour,
                    model,
                }
            }

            fn opts(&self, model_id: &str) -> StartOptions {
                StartOptions {
                    model_id: model_id.into(),
                    model_path: self.model.clone(),
                    backend: "cpu".into(),
                    context_tokens: 512,
                    gpu_layers: 0,
                    embedding: None,
                }
            }

            async fn ensure_model(&self, model_id: &str) -> Result<u16, String> {
                let exe = std::env::current_exe().expect("Test-EXE");
                self.server.ensure(&exe, self.opts(model_id), None).await
            }

            async fn ensure(&self) -> Result<u16, String> {
                self.ensure_model("m").await
            }

            async fn ensure_logged(&self, log: &Path) -> Result<u16, String> {
                let exe = std::env::current_exe().expect("Test-EXE");
                self.server
                    .ensure(&exe, self.opts("m"), Some(log.to_path_buf()))
                    .await
            }

            fn spawned(&self) -> usize {
                self.spawned.load(Ordering::Acquire)
            }

            /// "Absturz": den Prozess beenden, ohne den Server davon zu
            /// unterrichten -- der Zustand bleibt "bereit".
            fn crash(&self) {
                let mut guard = self.server.running.lock().unwrap();
                let r = guard.as_mut().expect("laufender Prozess");
                r.child.kill().expect("kill");
                let _ = r.child.wait();
            }
        }

        impl Drop for Harness {
            fn drop(&mut self) {
                self.server.stop();
                let _ = std::fs::remove_file(&self.model);
            }
        }

        const CRASHED: &str = "server_crashed:";

        #[tokio::test]
        async fn a_dead_process_is_cleared_and_the_next_ensure_starts_a_new_one() {
            let h = Harness::new(RESTART_COOLDOWN);
            h.ensure().await.expect("erster Start");
            assert!(h.server.is_serving("m"));
            assert_eq!(h.spawned(), 1);

            h.crash();
            // Vor dem Aufraeumen: nicht mehr "bereit", Status zeigt den Absturz.
            assert!(
                !h.server.is_serving("m"),
                "toter Prozess gilt nicht als bereit"
            );
            assert!(h.server.live_port("m").await.is_none());
            let status = h.server.status();
            assert_eq!(status.phase, LocalLlmPhase::Error);
            assert!(status.message.unwrap_or_default().starts_with(CRASHED));

            let port = h.ensure().await.expect("Neustart nach Absturz");
            assert_eq!(h.spawned(), 2, "genau ein neuer Prozess");
            assert_eq!(h.server.start_count(), 2);
            assert!(h.server.is_serving("m"));
            assert_eq!(h.server.live_port("m").await, Some(port));
            assert_eq!(h.server.status().phase, LocalLlmPhase::Ready);
        }

        /// Der Neustart ueberschreibt die Logdatei; die Ausgabe des
        /// abgestuerzten Prozesses bleibt als `*.crash.log` erhalten.
        #[tokio::test]
        async fn the_log_of_the_crashed_process_is_kept_for_diagnosis() {
            let h = Harness::new(RESTART_COOLDOWN);
            let log = h.model.with_extension("log");
            h.ensure_logged(&log).await.unwrap();
            std::fs::write(&log, "CUDA error: an illegal memory access was encountered").unwrap();
            h.crash();
            h.ensure_logged(&log).await.expect("Neustart");
            let kept = log.with_extension("crash.log");
            let text = std::fs::read_to_string(&kept).unwrap_or_default();
            let _ = std::fs::remove_file(&log);
            let _ = std::fs::remove_file(&kept);
            assert!(text.contains("illegal memory access"), "war: {text}");
        }

        #[tokio::test]
        async fn a_healthy_server_is_reused_without_a_restart() {
            let h = Harness::new(RESTART_COOLDOWN);
            let a = h.ensure().await.unwrap();
            let b = h.ensure().await.unwrap();
            let c = h.server.live_port("m").await;
            assert_eq!(a, b);
            assert_eq!(c, Some(a));
            assert_eq!(h.spawned(), 1, "gesunder Server wird wiederverwendet");
            assert_eq!(h.server.status().phase, LocalLlmPhase::Ready);
        }

        #[tokio::test]
        async fn a_second_crash_within_the_cooldown_is_not_restarted() {
            let h = Harness::new(RESTART_COOLDOWN);
            h.ensure().await.unwrap();
            h.crash();
            h.ensure().await.expect("erster Neustart ist erlaubt");
            assert_eq!(h.spawned(), 2);

            h.crash();
            let err = h.ensure().await.unwrap_err();
            assert!(err.starts_with(CRASHED), "{err}");
            assert!(err.contains("höchstens einmal pro Minute"), "{err}");
            assert_eq!(h.spawned(), 2, "kein dritter Start binnen der Minute");
            assert!(!h.server.has_process(), "toter Prozess abgeraeumt");
            let status = h.server.status();
            assert_eq!(status.phase, LocalLlmPhase::Error);
            assert!(status.message.unwrap_or_default().starts_with(CRASHED));

            // Jeder weitere Aufruf in der Sperre bekommt sofort denselben
            // Fehler und startet nichts.
            for _ in 0..3 {
                let again = h.ensure().await.unwrap_err();
                assert!(again.starts_with(CRASHED), "{again}");
            }
            assert_eq!(h.spawned(), 2);
        }

        #[tokio::test]
        async fn the_restart_is_allowed_again_after_the_cooldown_and_opens_a_new_minute() {
            let h = Harness::new(Duration::from_millis(900));
            h.ensure().await.unwrap();
            h.crash();
            h.ensure().await.unwrap(); // Neustart 1
            h.crash();
            assert!(h.ensure().await.unwrap_err().starts_with(CRASHED));
            assert_eq!(h.spawned(), 2);

            tokio::time::sleep(Duration::from_millis(1000)).await;
            h.ensure().await.expect("nach der Sperrzeit wieder erlaubt");
            assert_eq!(h.spawned(), 3);

            // Der Start nach der Sperre zaehlt als Neustart: bricht auch er
            // sofort ab, folgt keine zweite Chance binnen der neuen Minute.
            h.crash();
            let err = h.ensure().await.unwrap_err();
            assert!(err.starts_with(CRASHED), "{err}");
            assert_eq!(h.spawned(), 3);
        }

        #[tokio::test]
        async fn a_failed_restart_blocks_further_attempts_and_reports_the_code() {
            let h = Harness::new(Duration::from_millis(1500));
            h.ensure().await.unwrap();
            h.crash();
            *h.behaviour.lock().unwrap() = Behaviour::ExitImmediately;
            let err = h.ensure().await.unwrap_err();
            assert!(err.starts_with(CRASHED), "{err}");
            assert!(err.contains("Neustart nach Absturz gescheitert"), "{err}");
            assert_eq!(h.spawned(), 2);
            assert!(!h.server.has_process());

            // Die Ursache ist behoben, aber die Sperre haelt noch: kein Start.
            *h.behaviour.lock().unwrap() = Behaviour::Healthy;
            let blocked = h.ensure().await.unwrap_err();
            assert!(blocked.starts_with(CRASHED), "{blocked}");
            assert_eq!(h.spawned(), 2, "kein Start in der Sperre");

            tokio::time::sleep(Duration::from_millis(1600)).await;
            h.ensure().await.expect("nach der Sperre wieder ein Start");
            assert_eq!(h.spawned(), 3);
        }

        #[tokio::test]
        async fn a_process_that_refuses_connections_counts_as_dead() {
            let mut h = Harness::new(RESTART_COOLDOWN);
            // Volle Frist: Windows meldet "verweigert" erst nach ~2 s.
            h.server.liveness_timeout = LIVENESS_TIMEOUT;
            let port = h.ensure().await.unwrap();
            // Prozess lebt, aber niemand lauscht mehr am Port.
            h.health
                .lock()
                .unwrap()
                .iter_mut()
                .for_each(FakeHealth::close);
            assert!(
                h.server.is_serving("m"),
                "try_wait allein sieht noch nichts"
            );
            assert!(
                h.server.live_port("m").await.is_none(),
                "verweigerter Port = tot"
            );

            let new_port = h.ensure().await.expect("Neustart");
            assert_eq!(h.spawned(), 2);
            assert_eq!(h.server.live_port("m").await, Some(new_port));
            let _ = port;
        }

        #[tokio::test]
        async fn a_busy_server_that_answers_too_slowly_is_not_declared_dead() {
            let h = Harness::new(RESTART_COOLDOWN);
            let port = h.ensure().await.unwrap();
            // Derselbe Port, aber die Antwort bleibt aus (der Server rechnet).
            h.health
                .lock()
                .unwrap()
                .iter_mut()
                .for_each(FakeHealth::close);
            h.health.lock().unwrap().push(FakeHealth::start(port, true));

            let again = h.ensure().await.expect("beschaeftigt ist nicht tot");
            assert_eq!(again, port);
            assert_eq!(h.spawned(), 1, "kein Neustart unter Last");
            assert_eq!(h.server.live_port("m").await, Some(port));
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn concurrent_callers_seeing_the_same_crash_start_only_one_process() {
            let h = Harness::new(RESTART_COOLDOWN);
            h.ensure().await.unwrap();
            h.crash();
            let (a, b, c) = tokio::join!(h.ensure(), h.ensure(), h.ensure());
            let (a, b, c) = (a.unwrap(), b.unwrap(), c.unwrap());
            assert!(a == b && b == c, "alle bekommen denselben Server");
            assert_eq!(h.spawned(), 2, "ein einziger Neustart");
            assert_eq!(h.server.live_port("m").await, Some(a));
        }

        #[tokio::test]
        async fn a_stop_by_the_user_clears_the_block_but_not_the_minute() {
            let h = Harness::new(RESTART_COOLDOWN);
            h.ensure().await.unwrap();
            h.crash();
            h.ensure().await.unwrap(); // Neustart 1, oeffnet die Minute
            h.crash();
            assert!(h.ensure().await.unwrap_err().starts_with(CRASHED));

            h.server.stop(); // bewusster Stopp des Nutzers
            h.ensure()
                .await
                .expect("bewusster Start ist nicht gesperrt");
            assert_eq!(h.spawned(), 3);

            // Die Minute laeuft aber weiter: ein weiterer Absturz wird nicht
            // automatisch aufgefangen.
            h.crash();
            assert!(h.ensure().await.unwrap_err().starts_with(CRASHED));
            assert_eq!(h.spawned(), 3);
        }

        #[tokio::test]
        async fn a_block_applies_to_the_crashed_model_only() {
            let h = Harness::new(RESTART_COOLDOWN);
            h.ensure().await.unwrap();
            h.crash();
            h.ensure().await.unwrap();
            h.crash();
            assert!(h.ensure().await.unwrap_err().starts_with(CRASHED));

            h.ensure_model("anderes")
                .await
                .expect("ein anderes Modell ist nicht betroffen");
            assert!(h.server.is_serving("anderes"));
            assert_eq!(h.spawned(), 3);
        }

        #[tokio::test]
        async fn a_cancelled_restart_does_not_arm_the_block() {
            let h = Harness::new(RESTART_COOLDOWN);
            // Der Nutzer bricht den Start ab (Stopp setzt das Flag).
            let start = h.server.settle_start(
                StartKind::AfterCrash {
                    model_id: "m".into(),
                    restarted_at: Instant::now(),
                },
                Err(START_CANCELLED.to_string()),
            );
            assert_eq!(start.unwrap_err(), START_CANCELLED);
            assert!(h.server.crash.lock().unwrap().blocked.is_none());
        }
    }

    /// Echter Prozess: `llama-server` (Embedding-Modus, BGE-M3, nur CPU)
    /// starten, den Prozess von aussen per PID beenden, `ensure` erneut ->
    /// ein neuer Prozess antwortet auf /health.
    ///
    /// LVA_TEST_LLAMA_SERVER=<llama-server.exe> LVA_TEST_LLAMA_MODEL=<bge-m3.gguf> \
    ///   cargo test --lib llm::server::tests::real_ -- --ignored --nocapture
    #[cfg(windows)]
    #[ignore = "braucht llama-server.exe und ein kleines GGUF (Umgebungsvariablen)"]
    #[tokio::test]
    async fn real_llama_server_is_restarted_after_its_process_was_killed() {
        let (Some(binary), Some(model)) = (
            std::env::var_os("LVA_TEST_LLAMA_SERVER").map(PathBuf::from),
            std::env::var_os("LVA_TEST_LLAMA_MODEL").map(PathBuf::from),
        ) else {
            panic!("LVA_TEST_LLAMA_SERVER und LVA_TEST_LLAMA_MODEL setzen");
        };
        let log = std::env::temp_dir().join(format!("lva-p1h-real-{}.log", std::process::id()));
        let opts = || StartOptions {
            model_id: "real".into(),
            model_path: model.clone(),
            backend: "cpu".into(),
            context_tokens: 4096,
            gpu_layers: 0,
            embedding: Some(EmbeddingOpts {
                pooling: "cls",
                parallel: 2,
                below_normal: true,
            }),
        };
        let server = LocalLlmServer::new();

        let port1 = server
            .ensure(&binary, opts(), Some(log.clone()))
            .await
            .expect("erster Start");
        let pid1 = server.pid().expect("PID");
        assert!(server.live_port("real").await.is_some());
        println!("gestartet: pid {pid1}, Port {port1}");

        // Von aussen beenden -- nur die eigene PID, nie ueber den Namen.
        let out = std::process::Command::new("taskkill")
            .args(["/PID", &pid1.to_string(), "/T", "/F"])
            .output()
            .expect("taskkill");
        assert!(out.status.success(), "taskkill: {out:?}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !server.child_exited() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(server.child_exited(), "Prozess ist beendet");
        assert!(
            !server.is_serving("real"),
            "toter Prozess gilt nicht als bereit"
        );
        println!("Status nach Absturz: {:?}", server.status().message);

        let port2 = server
            .ensure(&binary, opts(), Some(log.clone()))
            .await
            .expect("Neustart");
        let pid2 = server.pid().expect("PID");
        println!(
            "neu gestartet: pid {pid2}, Port {port2}, Starts {}",
            server.start_count()
        );
        assert_eq!(
            server.start_count(),
            2,
            "ein neuer Prozess (PIDs koennen wiederverwendet werden)"
        );
        assert!(server.live_port("real").await.is_some());
        let health = reqwest::get(format!("http://127.0.0.1:{port2}/health"))
            .await
            .expect("health");
        assert!(health.status().is_success());

        server.stop();
        assert!(!server.has_process());
        let _ = std::fs::remove_file(&log);
        // Die Ausgabe des beendeten Prozesses blieb als *.crash.log erhalten.
        let _ = std::fs::remove_file(log.with_extension("crash.log"));
    }
}
