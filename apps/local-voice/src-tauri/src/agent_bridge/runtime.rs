//! Anbindung an die laufende App: die Bruecke startet mit der App, ihr Zustand ist fuer die
//! Oberflaeche abfragbar, und fuer Tests gibt es eine headless Sandbox-Instanz (A7).
//!
//! Die Bruecke laeuft auf der async-Laufzeit der App (`tauri::async_runtime`), nie im Audio-Pfad:
//! sie teilt weder Sperren noch Puffer mit der Aufnahme. Ein Fehler beim Start (Pipe-Name
//! belegt, keine SID) laesst die App normal weiterlaufen; der Zustand zeigt ihn an.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::oneshot;

use super::bridge::Bridge;
use super::catalog::{ToolHandler, ToolRegistry};
use super::server::Server;
use super::{pipe, Config};
use crate::managers::meetings::store::MeetingStore;

/// Zustand der Bruecke fuer die Oberflaeche.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BridgeStatus {
    pub running: bool,
    /// Name der Pipe (nie ein Geheimnis; der Nutzer traegt ihn bei Bedarf in Skripte ein).
    pub pipe_name: Option<String>,
    /// Warum sie nicht laeuft.
    pub error: Option<String>,
}

/// Zustand und Zugriff fuer die Commands. Als Tauri-State verwaltet.
pub struct AppBridge {
    status: Mutex<BridgeStatus>,
    server: Arc<Server>,
}

impl AppBridge {
    pub fn status(&self) -> BridgeStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set(&self, status: BridgeStatus) {
        if let Ok(mut s) = self.status.lock() {
            *s = status;
        }
    }

    /// Werkzeuge, die diese App-Version ausfuehren kann (fuer die Ansicht der Zugaenge).
    pub fn available_tools(&self) -> HashSet<String> {
        self.server.bridge().registry().names().into_iter().collect()
    }

    pub fn server(&self) -> &Arc<Server> {
        &self.server
    }
}

/// Startet die Bruecke mit der App. `handlers`: die Werkzeuge dieser App-Version (A8 haengt
/// hier die schreibenden MCP-Werkzeuge an); ohne Handler laeuft die Bruecke trotzdem
/// (Anmeldung, Liste, `status`), es gibt nur noch nichts auszufuehren.
pub fn start_for_app(
    store: Arc<MeetingStore>,
    handlers: Vec<Arc<dyn ToolHandler>>,
) -> Arc<AppBridge> {
    let mut registry = ToolRegistry::new();
    for h in handlers {
        if let Err(e) = registry.register(h) {
            log::error!("agent_bridge: Werkzeug nicht registriert: {e}");
        }
    }
    let bridge = Arc::new(Bridge::new(store, registry, Config::default()));
    let server = Server::new(bridge);
    let app = Arc::new(AppBridge {
        status: Mutex::new(BridgeStatus::default()),
        server: server.clone(),
    });
    let name = match pipe::effective_pipe_name() {
        Ok(n) => n,
        Err(e) => {
            log::warn!("agent_bridge: Pipe-Name unbekannt: {e}");
            app.set(BridgeStatus {
                running: false,
                pipe_name: None,
                error: Some(e.to_string()),
            });
            return app;
        }
    };
    let state = app.clone();
    tauri::async_runtime::spawn(async move {
        let (tx, rx) = oneshot::channel();
        let (s2, n2) = (server.clone(), name.clone());
        let task = tauri::async_runtime::spawn(async move { pipe::serve(s2, &n2, Some(tx)).await });
        match rx.await {
            Ok(Ok(())) => {
                log::info!("agent_bridge: Pipe bereit ({name})");
                state.set(BridgeStatus {
                    running: true,
                    pipe_name: Some(name.clone()),
                    error: None,
                });
            }
            Ok(Err(e)) => {
                log::warn!("agent_bridge: Pipe nicht angelegt: {e}");
                state.set(BridgeStatus {
                    running: false,
                    pipe_name: Some(name.clone()),
                    error: Some(e.to_string()),
                });
                return;
            }
            Err(_) => return,
        }
        if let Ok(Err(e)) = task.await {
            log::warn!("agent_bridge: Pipe beendet: {e}");
            state.set(BridgeStatus {
                running: false,
                pipe_name: Some(name),
                error: Some(e.to_string()),
            });
        }
    });
    app
}

/// Headless Sandbox-Instanz (`--agent-bridge-serve`): bedient die Pipe, bis der Prozess beendet
/// wird oder `seconds` ablaufen. Gibt `AGENT_BRIDGE_READY pipe=<name>` auf der Standardausgabe aus,
/// sobald die Pipe steht. `LVA_AGENT_APPROVAL_WAIT_MS` verkuerzt die Wartezeit auf Freigaben
/// (nur dieser Sandbox-Weg). Rueckgabe: Exit-Code (0 ok, 1 Fehler).
pub fn run_headless(
    store: Arc<MeetingStore>,
    handlers: Vec<Arc<dyn ToolHandler>>,
    seconds: Option<u64>,
) -> i32 {
    let mut registry = ToolRegistry::new();
    for h in handlers {
        if let Err(e) = registry.register(h) {
            eprintln!("error: Werkzeug nicht registriert: {e}");
            return 1;
        }
    }
    let name = match pipe::effective_pipe_name() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let mut cfg = Config::default();
    // Nur fuer Tests gegen die echte EXE: die Wartezeit auf Freigaben verkuerzen (50 ms bis 30 s).
    if let Some(ms) = std::env::var("LVA_AGENT_APPROVAL_WAIT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
    {
        cfg.approval_wait = Duration::from_millis(ms.clamp(50, 30_000));
    }
    let server = Server::new(Arc::new(Bridge::new(store, registry, cfg)));
    tauri::async_runtime::block_on(async move {
        let (tx, rx) = oneshot::channel();
        let (s2, n2) = (server.clone(), name.clone());
        let task = tauri::async_runtime::spawn(async move { pipe::serve(s2, &n2, Some(tx)).await });
        match rx.await {
            Ok(Ok(())) => {
                println!("AGENT_BRIDGE_READY pipe={name}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            Ok(Err(e)) => {
                eprintln!("error: Pipe nicht angelegt: {e}");
                return 1;
            }
            Err(_) => return 1,
        }
        match seconds {
            Some(s) => {
                tokio::time::sleep(Duration::from_secs(s)).await;
                server.shutdown();
                let _ = task.await;
                0
            }
            None => match task.await {
                Ok(Ok(())) => 0,
                _ => 1,
            },
        }
    })
}
