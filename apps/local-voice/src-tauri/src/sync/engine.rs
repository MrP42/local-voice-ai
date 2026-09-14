//! Der Sync-Zyklus: dirty Objekte versiegeln und pushen, dann seit dem Cursor
//! pullen und anwenden. Ein Lauf zur Zeit; Auslöser sind der Takt (60 s), ein
//! Anstoß aus der Oberfläche (entprellt) und der Start.

use super::client::{HubClient, HubError, PushObject, RemoteObject};
use super::collect::{self, LocalObject};
use super::crypto;
use super::ledger::{self, Entry, Ledger};
use super::account::{self, SyncConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

const PUSH_BATCH: usize = 100;
/// Der Hub nimmt 512 KiB je Anfrage; darunter bleiben, sonst blockieren zwei
/// grosse Seiten den ganzen Abgleich.
const PUSH_BATCH_BYTES: usize = 400 * 1024;
/// Obergrenze fuer Pull-Seiten je Lauf: ein Server, der bei `more=true` den
/// Cursor nicht bewegt, darf keinen Endloslauf ausloesen.
const PULL_MAX_PAGES: usize = 500;
const PULL_LIMIT: u32 = 100;
const TICK: Duration = Duration::from_secs(60);
const DEBOUNCE: Duration = Duration::from_secs(2);

/// Ereignis an die Oberfläche nach jedem Lauf, der etwas geändert hat.
pub const EVENT_CHANGED: &str = "sync-changed";
/// Ereignis nach jedem Lauf mit dem neuen Status.
pub const EVENT_STATUS: &str = "sync-status";

#[derive(Serialize, Deserialize, Clone, Debug, Default, specta::Type)]
pub struct SyncStatus {
    pub connected: bool,
    pub user_email: Option<String>,
    pub device_name: Option<String>,
    pub hub_url: Option<String>,
    pub running: bool,
    pub last_success_ms: Option<f64>,
    pub last_error: Option<String>,
    /// Objekte, die lokal geändert und noch nicht bestätigt sind.
    pub pending: u32,
    pub dead_letters: u32,
    /// Der Hub trägt Objekte mit einem anderen Schlüssel (Passwort geändert?).
    pub key_mismatch: bool,
    /// Anzahl synchronisierter Seiten laut Ledger.
    pub pages: u32,
}

pub struct SyncEngine {
    status: Mutex<SyncStatus>,
    run_lock: tokio::sync::Mutex<()>,
    notify: Notify,
}

impl Default for SyncEngine {
    fn default() -> Self {
        Self { status: Mutex::new(SyncStatus::default()), run_lock: tokio::sync::Mutex::new(()), notify: Notify::new() }
    }
}

fn now_ms() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64
}

impl SyncEngine {
    pub fn status(&self) -> SyncStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_status(&self, f: impl FnOnce(&mut SyncStatus)) {
        if let Ok(mut s) = self.status.lock() {
            f(&mut s);
        }
    }

    /// Konto-Datei und Ledger nur wechseln, wenn kein Lauf mehr schreibt:
    /// Login/Logout halten diese Sperre, solange sie Dateien ersetzen.
    pub async fn account_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.run_lock.lock().await
    }

    /// Anstoß aus der Oberfläche oder nach einem lokalen Schreiben.
    pub fn touch(&self) {
        self.notify.notify_one();
    }

    /// Status aus der Konto-Datei ableiten (beim Start, nach Login/Logout).
    pub fn refresh_from_config(&self, app: &AppHandle) {
        let cfg = account::load(app);
        let led = ledger::load(app);
        self.set_status(|s| {
            s.connected = cfg.is_some();
            s.user_email = cfg.as_ref().map(|c| c.user_email.clone());
            s.device_name = cfg.as_ref().map(|c| c.device_name.clone());
            s.hub_url = cfg.as_ref().map(|c| c.url.clone());
            s.dead_letters = led.dead.len() as u32;
            s.pages = led.objects.keys().filter(|k| k.starts_with("page/")).count() as u32;
            if cfg.is_none() {
                s.last_error = None;
                s.pending = 0;
                s.key_mismatch = false;
            }
        });
    }

    /// Ein vollständiger Lauf. Läuft nie doppelt; ein zweiter Aufruf wartet.
    pub async fn run_once(&self, app: &AppHandle) -> SyncStatus {
        let _guard = self.run_lock.lock().await;
        let Some(cfg) = account::load(app) else {
            self.refresh_from_config(app);
            return self.status();
        };
        self.set_status(|s| s.running = true);
        let _ = app.emit(EVENT_STATUS, self.status());
        let result = cycle(app, &cfg).await;
        self.set_status(|s| {
            s.running = false;
            s.connected = true;
            s.user_email = Some(cfg.user_email.clone());
            s.device_name = Some(cfg.device_name.clone());
            s.hub_url = Some(cfg.url.clone());
        });
        match result {
            Ok(outcome) => {
                self.set_status(|s| {
                    s.last_success_ms = Some(now_ms());
                    s.last_error = None;
                    s.pending = outcome.pending;
                    s.dead_letters = outcome.dead_letters;
                    s.key_mismatch = outcome.key_mismatch;
                    s.pages = outcome.pages;
                });
                if outcome.changed {
                    let _ = app.emit(EVENT_CHANGED, ());
                }
            }
            Err(HubError::Unauthorized) => {
                // Token widerrufen (Gerät im Portal abgemeldet): lokal abmelden,
                // die Daten bleiben.
                account::remove(app);
                self.refresh_from_config(app);
                self.set_status(|s| s.last_error = Some(HubError::Unauthorized.to_string()));
            }
            Err(e) => {
                self.set_status(|s| s.last_error = Some(e.to_string()));
            }
        }
        let status = self.status();
        let _ = app.emit(EVENT_STATUS, status.clone());
        status
    }
}

/// Hintergrundschleife: Takt oder Anstoß, dann entprellt ein Lauf.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<SyncEngine>();
        engine.refresh_from_config(&app);
        // Erster Lauf kurz nach dem Start, nicht blockierend.
        tokio::time::sleep(Duration::from_secs(5)).await;
        loop {
            engine.run_once(&app).await;
            tokio::select! {
                _ = tokio::time::sleep(TICK) => {}
                _ = engine.notify.notified() => {
                    tokio::time::sleep(DEBOUNCE).await;
                }
            }
        }
    });
}

struct Outcome {
    changed: bool,
    pending: u32,
    dead_letters: u32,
    key_mismatch: bool,
    pages: u32,
}

async fn cycle(app: &AppHandle, cfg: &SyncConfig) -> Result<Outcome, HubError> {
    let key = cfg.key().map_err(HubError::Other)?;
    let my_key_id = crypto::key_id(&key);
    let client = HubClient::new(&cfg.url, &cfg.token);
    let mut led = ledger::load(app);
    let mut changed = false;

    // ---- Push -----------------------------------------------------------
    let locals = collect::collect(app).map_err(HubError::Other)?;
    let by_key: HashMap<String, &LocalObject> =
        locals.iter().map(|o| (ledger::key(o.collection, &o.id), o)).collect();

    let mut to_push: Vec<(PushObject, String, Option<&LocalObject>)> = Vec::new(); // (obj, hash, local)
    for o in &locals {
        let h = collect::hash(&o.value);
        if !led.is_dirty(o.collection, &o.id, &h) || led.is_dead(o.collection, &o.id, &h) {
            continue;
        }
        let plain = collect::canonical(&o.value);
        if plain.len() > collect::MAX_PLAINTEXT {
            led.mark_dead(o.collection, &o.id, &h, "too_large");
            continue;
        }
        let base = led.get(o.collection, &o.id).map(|e| e.revision).unwrap_or(0);
        let sealed = crypto::seal(&key, &crypto::aad(cfg.user_id, o.collection, &o.id), plain.as_bytes())
            .map_err(HubError::Other)?;
        to_push.push((
            PushObject {
                collection: o.collection.to_string(),
                object_id: o.id.clone(),
                base_revision: base,
                payload: sealed,
                key_id: my_key_id.clone(),
                deleted: false,
            },
            h,
            Some(o),
        ));
    }
    // Lokal verschwundene Seiten → Tombstone.
    let gone: Vec<(String, Entry)> = led
        .objects
        .iter()
        .filter(|(k, e)| k.starts_with("page/") && !e.deleted && !by_key.contains_key(*k))
        .map(|(k, e)| (k.clone(), e.clone()))
        .collect();
    for (k, e) in gone {
        let id = k.trim_start_matches("page/").to_string();
        to_push.push((
            PushObject {
                collection: collect::COLLECTION_PAGE.to_string(),
                object_id: id,
                base_revision: e.revision,
                payload: String::new(),
                key_id: my_key_id.clone(),
                deleted: true,
            },
            String::new(),
            None,
        ));
    }

    let mut pending: u32 = 0;
    let mut chunks: Vec<&[(PushObject, String, Option<&LocalObject>)]> = Vec::new();
    let mut start = 0;
    let mut bytes = 0;
    for (i, (o, _, _)) in to_push.iter().enumerate() {
        let size = o.payload.len() + 200;
        if i > start && (i - start >= PUSH_BATCH || bytes + size > PUSH_BATCH_BYTES) {
            chunks.push(&to_push[start..i]);
            start = i;
            bytes = 0;
        }
        bytes += size;
    }
    if start < to_push.len() {
        chunks.push(&to_push[start..]);
    }
    for chunk in chunks {
        let objs: Vec<PushObject> = chunk.iter().map(|(o, _, _)| o.clone()).collect();
        let results = client.push(&cfg.device_id, &objs).await?;
        for r in results {
            let Some((sent, h, local)) =
                chunk.iter().find(|(o, _, _)| o.collection == r.collection && o.object_id == r.object_id)
            else {
                continue;
            };
            match r.outcome.as_str() {
                "accepted" => {
                    led.set(&sent.collection, &sent.object_id, Entry { revision: r.revision, hash: h.clone(), deleted: sent.deleted });
                }
                "superseded" => {
                    match r.current {
                        Some(remote) => {
                            if remote.key_id != my_key_id {
                                return Ok(finish(app, led, changed, pending + 1, true));
                            }
                            let synced = led.get(&remote.collection, &remote.object_id).map(|e| e.hash.clone());
                            let _ = local;
                            let applied = apply_remote(app, cfg, &key, &remote, synced.as_deref())
                                .map_err(HubError::Other)?;
                            if applied {
                                changed = true;
                            }
                            led.set(&remote.collection, &remote.object_id, Entry { revision: remote.revision, hash: remote_hash(&key, cfg, &remote), deleted: remote.deleted });
                        }
                        None => {
                            // Server kennt das Objekt nicht (mehr): nächster Lauf mit Basis 0.
                            led.set(&sent.collection, &sent.object_id, Entry { revision: 0, hash: String::new(), deleted: false });
                            pending += 1;
                        }
                    }
                }
                "rejected" => {
                    led.mark_dead(&sent.collection, &sent.object_id, h, r.reason.as_deref().unwrap_or("rejected"));
                }
                _ => {
                    // retry / unbekannt: beim nächsten Lauf erneut.
                    pending += 1;
                }
            }
        }
        ledger::save(app, &led).map_err(HubError::Other)?;
    }

    // ---- Pull -----------------------------------------------------------
    let mut since = led.cursor;
    let mut key_mismatch = false;
    let mut reset_done = false;
    let mut pages_seen = 0usize;
    'pull: loop {
        pages_seen += 1;
        if pages_seen > PULL_MAX_PAGES {
            return Err(HubError::Other("Pull ohne Ende: zu viele Seiten in einem Lauf".into()));
        }
        let page = match client.pull(since, PULL_LIMIT).await {
            Ok(p) => p,
            Err(HubError::CursorExpired) if since > 0 && !reset_done => {
                reset_done = true;
                since = 0;
                continue;
            }
            Err(e) => return Err(e),
        };
        if page.more && page.cursor <= since {
            return Err(HubError::Other("Pull ohne Fortschritt: Cursor bewegt sich nicht".into()));
        }
        for remote in &page.objects {
            if remote.device_id == cfg.device_id && led.get(&remote.collection, &remote.object_id).is_some_and(|e| e.revision == remote.revision) {
                continue; // eigene, schon bestätigte Änderung
            }
            if !remote.deleted && remote.key_id != my_key_id {
                key_mismatch = true;
                break 'pull; // Cursor bleibt stehen, bis der Schlüssel passt
            }
            // Lokal ungesichert geaendert? Das entscheidet apply_page am
            // frischen Stand unter der Seiten-Sperre und sichert vorher die
            // Konfliktkopie -- ein Schnappschuss vom Zyklusanfang reicht
            // dafuer nicht (Review-Befund 14.09.).
            let synced = led.get(&remote.collection, &remote.object_id).map(|e| e.hash.clone());
            if apply_remote(app, cfg, &key, remote, synced.as_deref()).map_err(HubError::Other)? {
                changed = true;
            }
            led.set(&remote.collection, &remote.object_id, Entry { revision: remote.revision, hash: remote_hash(&key, cfg, remote), deleted: remote.deleted });
        }
        led.cursor = page.cursor.max(since);
        since = led.cursor;
        ledger::save(app, &led).map_err(HubError::Other)?;
        if !page.more {
            break;
        }
    }
    Ok(finish(app, led, changed, pending, key_mismatch))
}

fn finish(app: &AppHandle, led: Ledger, changed: bool, pending: u32, key_mismatch: bool) -> Outcome {
    let _ = ledger::save(app, &led);
    Outcome {
        changed,
        pending,
        dead_letters: led.dead.len() as u32,
        key_mismatch,
        pages: led.objects.keys().filter(|k| k.starts_with("page/")).count() as u32,
    }
}

fn remote_hash(key: &[u8; crypto::KEY_LEN], cfg: &SyncConfig, remote: &RemoteObject) -> String {
    if remote.deleted {
        return String::new();
    }
    decrypt(key, cfg, remote).map(|v| collect::hash(&v)).unwrap_or_default()
}

fn decrypt(key: &[u8; crypto::KEY_LEN], cfg: &SyncConfig, remote: &RemoteObject) -> Result<serde_json::Value, String> {
    let plain = crypto::open(key, &crypto::aad(cfg.user_id, &remote.collection, &remote.object_id), &remote.payload)?;
    serde_json::from_slice(&plain).map_err(|e| format!("Objekt unlesbar: {e}"))
}

/// Ein Server-Objekt lokal anwenden. `synced_hash` ist der zuletzt
/// bestätigte Hash des Objekts; `apply_page` vergleicht damit den FRISCHEN
/// lokalen Stand und sichert ihn bei Abweichung als Konfliktkopie. Liefert,
/// ob sich lokal etwas geändert hat.
fn apply_remote(
    app: &AppHandle,
    cfg: &SyncConfig,
    key: &[u8; crypto::KEY_LEN],
    remote: &RemoteObject,
    synced_hash: Option<&str>,
) -> Result<bool, String> {
    match remote.collection.as_str() {
        collect::COLLECTION_PAGE => {
            if remote.deleted {
                collect::tombstone_page(app, &remote.object_id)?;
                return Ok(true);
            }
            let value = decrypt(key, cfg, remote)?;
            collect::apply_page(app, &remote.object_id, &value, synced_hash, &cfg.device_name)?;
            Ok(true)
        }
        collect::COLLECTION_SETTINGS => {
            if remote.deleted {
                return Ok(false);
            }
            let value = decrypt(key, cfg, remote)?;
            collect::apply_settings(app, &value)?;
            Ok(true)
        }
        other => {
            log::warn!("Sync: unbekannte Sammlung {other} übersprungen");
            Ok(false)
        }
    }
}
