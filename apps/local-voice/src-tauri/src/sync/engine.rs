//! Der Sync-Zyklus: dirty Objekte versiegeln und pushen, dann seit dem Cursor
//! pullen und anwenden. Ein Lauf zur Zeit; Auslöser sind der Takt (60 s), ein
//! Anstoß aus der Oberfläche (entprellt) und der Start.

use super::account::{self, SyncConfig};
use super::client::{HubClient, HubError, PushObject, RemoteObject};
use super::collect::{self, LocalObject};
use super::ledger::{self, Entry, Ledger};
use super::{blobs, books, crypto};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

/// Objekte je Push. Klein gehalten, weil der Hub je `superseded` das volle
/// Serverobjekt (bis `collect::HUB_MAX_PAYLOAD`) zurückgibt; die Antwort ist
/// damit nach oben begrenzt (siehe `curl::MAX_OUT`).
pub const PUSH_BATCH: usize = 16;
/// Der Hub nimmt 512 KiB je Anfrage; darunter bleiben, sonst blockieren zwei
/// grosse Seiten den ganzen Abgleich.
const PUSH_BATCH_BYTES: usize = 400 * 1024;
/// Obergrenze fuer Pull-Seiten je Lauf: ein Server, der bei `more=true` den
/// Cursor nicht bewegt, darf keinen Endloslauf ausloesen.
const PULL_MAX_PAGES: usize = 500;
const PULL_LIMIT: u32 = 100;
const TICK: Duration = Duration::from_secs(60);
const DEBOUNCE: Duration = Duration::from_secs(2);
/// Bytes, die ein Lauf für Prüfsummen neuer Bibliotheksdateien lesen darf.
const HASH_BUDGET: u64 = 4 * 1024 * 1024 * 1024;
/// Klartext-Bytes, die ein Lauf hochlädt; der Rest folgt im nächsten Takt.
const UPLOAD_BUDGET: u64 = 256 * 1024 * 1024;
/// Bytes, die ein Lauf herunterlädt (mindestens eine Datei).
const DOWNLOAD_BUDGET: u64 = 256 * 1024 * 1024;
/// Ab so vielen gleichzeitig verschwundenen Objekten einer Sammlung, von der
/// lokal nichts übrig ist, greift die Löschbremse.
const MASS_DELETE_MIN: usize = 3;
/// Ereignis mit den Fortschritts-Schlüsseln, die ein anderes Gerät gelöscht hat.
pub const EVENT_PROGRESS_REMOVED: &str = "sync-progress-removed";

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
    /// Wartezeiten fehlgeschlagener Dateien beim nächsten Lauf verwerfen: beim
    /// App-Start und bei „Jetzt abgleichen“. Sonst wartet eine Datei nach einem
    /// behobenen Hub-Fehler bis zu 6 Stunden.
    retry_files: std::sync::atomic::AtomicBool,
}

impl Default for SyncEngine {
    fn default() -> Self {
        Self {
            status: Mutex::new(SyncStatus::default()),
            run_lock: tokio::sync::Mutex::new(()),
            notify: Notify::new(),
            retry_files: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

fn now_ms() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64
}

impl SyncEngine {
    /// Beim nächsten Lauf alle Dateien sofort erneut versuchen.
    pub fn retry_files_now(&self) {
        self.retry_files
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn take_retry_files(&self) -> bool {
        self.retry_files
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    }

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
            s.pages = led
                .objects
                .keys()
                .filter(|k| k.starts_with("page/"))
                .count() as u32;
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
                    s.last_error = outcome.warning.clone();
                    s.pending = outcome.pending;
                    s.dead_letters = outcome.dead_letters;
                    s.key_mismatch = outcome.key_mismatch;
                    s.pages = outcome.pages;
                });
                if !outcome.removed.is_empty() {
                    let _ = app.emit(EVENT_PROGRESS_REMOVED, outcome.removed.clone());
                }
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
    // Abo-Relay fuer das Buch-Gespraech: eigener, schneller Takt (lva#320).
    super::booktalk_relay::start(app.clone());
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
    /// Lauf gelungen, aber eine Datei ließ sich nicht hoch- oder herunterladen.
    warning: Option<String>,
    /// Fortschritts-Schlüssel, die ein anderes Gerät gelöscht hat.
    removed: Vec<String>,
}

/// Sammlungen, deren lokale Löschung als Grabstein zum Hub geht.
const TOMBSTONED: [&str; 4] = [
    collect::COLLECTION_PAGE,
    books::COLLECTION_BOOK,
    books::COLLECTION_PROGRESS,
    books::COLLECTION_BOOKMARK,
];

fn is_library(collection: &str) -> bool {
    matches!(
        collection,
        books::COLLECTION_BOOK | books::COLLECTION_PROGRESS | books::COLLECTION_BOOKMARK
    )
}

async fn cycle(app: &AppHandle, cfg: &SyncConfig) -> Result<Outcome, HubError> {
    let key = cfg.key().map_err(HubError::Other)?;
    let my_key_id = crypto::key_id(&key);
    let client = HubClient::new(&cfg.url, &cfg.token);
    let mut led = ledger::load(app);
    if app.state::<SyncEngine>().take_retry_files()
        && (!led.failed.is_empty() || !led.dead.is_empty())
    {
        led.failed.clear();
        // Abgewiesene Objekte ebenfalls neu versuchen: Grenzen und zugelassene
        // Sammlungen des Hubs ändern sich mit Updates. Bleibt der Grund, wird
        // das Objekt im selben Lauf wieder als abgewiesen vermerkt.
        led.dead.clear();
        // Sofort sichern: bricht dieser Lauf früh ab, gilt der Neuversuch trotzdem.
        ledger::save(app, &led).map_err(HubError::Other)?;
    }
    let mut changed = false;
    let mut warning: Option<String> = None;
    let mut removed: Vec<String> = Vec::new();

    // ---- Sammeln ---------------------------------------------------------
    let (mut locals, held_pages) = collect::collect(app).map_err(HubError::Other)?;
    // Bibliothek: Prüfsummen großer Dateien gehören nicht auf den Executor.
    let lib = {
        let app2 = app.clone();
        let mut files = std::mem::take(&mut led.files);
        let (lib, files) = tauri::async_runtime::spawn_blocking(move || {
            let mut budget = HASH_BUDGET;
            let lib = books::collect_library(&app2, &key, &mut files, &mut budget);
            (lib, files)
        })
        .await
        .map_err(|e| HubError::Other(e.to_string()))?;
        led.files = files;
        lib.map_err(HubError::Other)?
    };
    if let Some(w) = lib.warnings.first() {
        warning = Some(w.clone());
    }

    // ---- Dateien hochladen ----------------------------------------------
    // Ein Buch geht erst zum Hub, wenn alle seine Dateien in der Ablage
    // abgeschlossen sind (Spec: „Reihenfolge beim Hochladen“).
    let mut blocked: HashSet<String> = lib.held.clone().into_iter().collect();
    let mut dirty_books: Vec<(&LocalObject, u64)> = lib
        .objects
        .iter()
        .filter(|o| o.collection == books::COLLECTION_BOOK)
        .filter(|o| {
            let h = collect::hash(&o.value);
            led.is_dirty(o.collection, &o.id, &h) && !led.is_dead(o.collection, &o.id, &h)
        })
        .map(|o| {
            let missing = lib
                .uploads
                .get(&o.id)
                .map(|ups| {
                    ups.iter()
                        .filter(|u| !led.uploaded.contains(&u.blob.blob))
                        .map(|u| u.blob.size)
                        .sum()
                })
                .unwrap_or(0);
            (o, missing)
        })
        .collect();
    // Kleine Bücher zuerst: sie erscheinen sofort auf dem anderen Gerät.
    dirty_books.sort_by_key(|(_, missing)| *missing);
    let mut upload_budget = UPLOAD_BUDGET;
    let mut upload_stopped = false;
    for (book, _) in dirty_books {
        let uploads = lib.uploads.get(&book.id).cloned().unwrap_or_default();
        for u in uploads {
            if led.uploaded.contains(&u.blob.blob) {
                continue;
            }
            if upload_stopped || led.backing_off(&u.blob.blob, now_ms()) {
                blocked.insert(book.id.clone());
                break;
            }
            match blobs::upload(
                &client,
                &key,
                cfg.user_id,
                &u.blob.blob,
                &u.source,
                u.blob.size,
                u.stamp.as_ref(),
                &mut upload_budget,
            )
            .await
            {
                Ok(blobs::Upload::Done) => {
                    led.uploaded.insert(u.blob.blob.clone());
                    led.failed.remove(&u.blob.blob);
                }
                Ok(blobs::Upload::Later) => {
                    blocked.insert(book.id.clone());
                    upload_stopped = upload_budget == 0;
                    break;
                }
                Err(HubError::Unauthorized) => return Err(HubError::Unauthorized),
                Err(e) => {
                    log::warn!("Sync: Datei von {} nicht hochgeladen: {e}", book.id);
                    warning = Some(format!("Datei nicht hochgeladen: {e}"));
                    blocked.insert(book.id.clone());
                    // Geänderte Datei: Prüfsumme im nächsten Lauf neu.
                    if let blobs::Source::File(p) = &u.source {
                        led.files.remove(&p.to_string_lossy().into_owned());
                    }
                    let text = e.to_string();
                    if text.contains("nicht erreichbar") || text.contains("quota_exceeded") {
                        // Netz weg oder Ablage voll: in diesem Lauf nichts mehr.
                        upload_stopped = true;
                    } else {
                        // Nur diese Datei ruht eine Weile, die anderen laufen.
                        led.note_failure(&u.blob.blob, now_ms());
                    }
                    break;
                }
            }
        }
        ledger::save(app, &led).map_err(HubError::Other)?;
    }
    let mut pending: u32 = blocked
        .iter()
        .filter(|id| {
            lib.objects.iter().any(|o| {
                o.collection == books::COLLECTION_BOOK
                    && &o.id == *id
                    && led.is_dirty(o.collection, &o.id, &collect::hash(&o.value))
            })
        })
        .count() as u32;
    // Zurückgehaltene Bücher und Seiten gelten nicht als gelöscht.
    let present: HashSet<String> = blocked
        .iter()
        .map(|id| ledger::key(books::COLLECTION_BOOK, id))
        .chain(
            held_pages
                .iter()
                .map(|id| ledger::key(collect::COLLECTION_PAGE, id)),
        )
        .collect();
    locals.extend(
        lib.objects
            .into_iter()
            .filter(|o| !(o.collection == books::COLLECTION_BOOK && blocked.contains(&o.id))),
    );

    // ---- Push -----------------------------------------------------------
    let by_key: HashMap<String, &LocalObject> = locals
        .iter()
        .map(|o| (ledger::key(o.collection, &o.id), o))
        .collect();

    let mut to_push: Vec<(PushObject, String)> = Vec::new(); // (obj, hash)
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
        let base = led
            .get(o.collection, &o.id)
            .map(|e| e.revision)
            .unwrap_or(0);
        let sealed = crypto::seal(
            &key,
            &crypto::aad(cfg.user_id, o.collection, &o.id),
            plain.as_bytes(),
        )
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
        ));
    }
    // Lokal verschwundene Objekte → Grabstein.
    let gone: Vec<(String, String, Entry)> = led
        .objects
        .iter()
        .filter(|(k, e)| !e.deleted && !by_key.contains_key(*k) && !present.contains(*k))
        .filter_map(|(k, e)| {
            let (c, id) = k.split_once('/')?;
            TOMBSTONED
                .contains(&c)
                .then(|| (c.to_string(), id.to_string(), e.clone()))
        })
        .collect();
    // Löschbremse: verschwindet eine ganze Bibliotheks-Sammlung auf einmal
    // (neues Datenverzeichnis, zurückgesetzter Fortschritt), ist das ein
    // Unfall und keine Absicht. Dann keine Grabsteine.
    let mut braked: HashSet<String> = HashSet::new();
    for c in [
        books::COLLECTION_BOOK,
        books::COLLECTION_PROGRESS,
        books::COLLECTION_BOOKMARK,
    ] {
        let gone_n = gone.iter().filter(|(gc, _, _)| gc == c).count();
        let left = locals.iter().filter(|o| o.collection == c).count()
            + present
                .iter()
                .filter(|k| k.starts_with(&format!("{c}/")))
                .count();
        if gone_n >= MASS_DELETE_MIN && left == 0 {
            log::warn!("Sync: {gone_n} Objekte in {c} lokal verschwunden, keine Grabsteine");
            warning = Some(format!(
                "{gone_n} Einträge ({c}) fehlen lokal; nichts wurde auf anderen Geräten gelöscht"
            ));
            braked.insert(c.to_string());
        }
    }
    for (c, id, e) in gone {
        if braked.contains(&c) {
            continue;
        }
        to_push.push((
            PushObject {
                collection: c,
                object_id: id,
                base_revision: e.revision,
                payload: String::new(),
                key_id: my_key_id.clone(),
                deleted: true,
            },
            String::new(),
        ));
    }

    let mut chunks: Vec<&[(PushObject, String)]> = Vec::new();
    let mut start = 0;
    let mut bytes = 0;
    for (i, (o, _)) in to_push.iter().enumerate() {
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
        let objs: Vec<PushObject> = chunk.iter().map(|(o, _)| o.clone()).collect();
        let results = client.push(&cfg.device_id, &objs).await?;
        for r in results {
            let Some((sent, h)) = chunk
                .iter()
                .find(|(o, _)| o.collection == r.collection && o.object_id == r.object_id)
            else {
                continue;
            };
            match r.outcome.as_str() {
                "accepted" => {
                    led.set(
                        &sent.collection,
                        &sent.object_id,
                        Entry {
                            revision: r.revision,
                            hash: h.clone(),
                            deleted: sent.deleted,
                        },
                    );
                }
                "superseded" => match r.current {
                    Some(remote) => {
                        if remote.key_id != my_key_id {
                            return Ok(finish(
                                app,
                                led,
                                changed,
                                pending + 1,
                                true,
                                warning,
                                removed,
                            ));
                        }
                        if settle(app, cfg, &key, &remote, &mut led, &mut removed)
                            .map_err(HubError::Other)?
                        {
                            changed = true;
                        }
                    }
                    None => {
                        // Server kennt das Objekt nicht (mehr): nächster Lauf mit Basis 0.
                        led.set(
                            &sent.collection,
                            &sent.object_id,
                            Entry {
                                revision: 0,
                                hash: String::new(),
                                deleted: false,
                            },
                        );
                        pending += 1;
                    }
                },
                "rejected" => {
                    led.mark_dead(
                        &sent.collection,
                        &sent.object_id,
                        h,
                        r.reason.as_deref().unwrap_or("rejected"),
                    );
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
    // Einmal alles neu einlesen: früher übersprungene Objekte kommen so an.
    // Bekannte Stände werden dabei nicht erneut angewendet (keine Konfliktkopien).
    let rescanning = led.rescan < ledger::RESCAN_LEVEL;
    if rescanning {
        led.cursor = 0;
        led.rescan = ledger::RESCAN_LEVEL;
        // iPhone-Bücher (`book_<ms>_<hex>`) wurden bisher übersprungen, aber als
        // gesehen verbucht: diese Einträge vergessen, damit sie jetzt ankommen.
        led.objects.retain(|k, _| {
            !k.strip_prefix("book/book_")
                .is_some_and(|rest| rest.contains('_'))
        });
    }
    let mut since = led.cursor;
    let mut key_mismatch = false;
    let mut reset_done = false;
    let mut pages_seen = 0usize;
    'pull: loop {
        pages_seen += 1;
        if pages_seen > PULL_MAX_PAGES {
            return Err(HubError::Other(
                "Pull ohne Ende: zu viele Seiten in einem Lauf".into(),
            ));
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
            return Err(HubError::Other(
                "Pull ohne Fortschritt: Cursor bewegt sich nicht".into(),
            ));
        }
        for remote in &page.objects {
            // Das Relay hat seinen eigenen Takt und Cursor (booktalk_relay.rs).
            if remote.collection == super::booktalk_relay::COLLECTION {
                continue;
            }
            if rescanning
                && led
                    .get(&remote.collection, &remote.object_id)
                    .is_some_and(|e| e.revision >= remote.revision)
            {
                continue; // beim Neueinlesen schon bekannt
            }
            if remote.device_id == cfg.device_id
                && led
                    .get(&remote.collection, &remote.object_id)
                    .is_some_and(|e| e.revision == remote.revision)
            {
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
            if settle(app, cfg, &key, remote, &mut led, &mut removed).map_err(HubError::Other)? {
                changed = true;
            }
        }
        led.cursor = page.cursor.max(since);
        since = led.cursor;
        ledger::save(app, &led).map_err(HubError::Other)?;
        if !page.more {
            break;
        }
    }

    // ---- Dateien herunterladen -------------------------------------------
    // Was andere Geräte gemeldet haben und hier fehlt: Text und Cover zuerst,
    // Hörbuchteile in Portionen über mehrere Läufe. Eine Datei, die nicht
    // ankommt, ruht eine Weile; die übrigen laufen weiter.
    let wanted = books::wanted(app).map_err(HubError::Other)?;
    let mut download_budget = DOWNLOAD_BUDGET;
    for w in wanted {
        if led.backing_off(&w.blob.blob, now_ms()) {
            continue;
        }
        if w.blob.size > download_budget && download_budget < DOWNLOAD_BUDGET {
            break; // mindestens eine Datei je Lauf, auch eine große
        }
        let target = books::download_target(&w);
        let sha = match blobs::download(
            &client,
            &key,
            cfg.user_id,
            &w.blob.blob,
            w.blob.size,
            &target,
        )
        .await
        {
            Ok(sha) => sha,
            Err(HubError::Unauthorized) => return Err(HubError::Unauthorized),
            Err(e) => {
                log::warn!("Sync: Datei für {} nicht geladen: {e}", w.book_id);
                warning = Some(format!("Datei nicht geladen: {e}"));
                led.note_failure(&w.blob.blob, now_ms());
                if e.to_string().contains("nicht erreichbar") {
                    break;
                }
                continue;
            }
        };
        download_budget = download_budget.saturating_sub(w.blob.size);
        let ledger_key = ledger::key(books::COLLECTION_BOOK, &w.book_id);
        let before = books::local_hash(
            app,
            &key,
            books::COLLECTION_BOOK,
            &w.book_id,
            &mut led.files,
        )
        .map_err(HubError::Other)?;
        match books::finish_download(app, &w, &cfg.device_name) {
            Ok(placed) => {
                // Die Prüfsumme ist bekannt: nicht noch einmal einlesen.
                if let Some(path) = placed {
                    if let Ok((size, mtime_ms)) = blobs::stat_of(&path) {
                        led.files.insert(
                            path.to_string_lossy().into_owned(),
                            blobs::FileStamp {
                                size,
                                mtime_ms,
                                sha256: sha,
                            },
                        );
                    }
                }
            }
            Err(e) => {
                log::warn!("Sync: Datei für {} nicht abgelegt: {e}", w.book_id);
                warning = Some(format!("Datei nicht abgelegt: {e}"));
                led.note_failure(&w.blob.blob, now_ms());
                continue;
            }
        }
        led.uploaded.insert(w.blob.blob.clone());
        led.failed.remove(&w.blob.blob);
        // Ein gesicherter Stand bleibt gesichert: die geladene Datei ist
        // keine lokale Änderung.
        let after = books::local_hash(
            app,
            &key,
            books::COLLECTION_BOOK,
            &w.book_id,
            &mut led.files,
        )
        .map_err(HubError::Other)?;
        if let Some(e) = led.objects.get_mut(&ledger_key) {
            if before.as_deref() == Some(e.hash.as_str()) {
                e.hash = after.unwrap_or_default();
            }
        }
        changed = true;
        ledger::save(app, &led).map_err(HubError::Other)?;
    }
    Ok(finish(
        app,
        led,
        changed,
        pending,
        key_mismatch,
        warning,
        removed,
    ))
}

fn finish(
    app: &AppHandle,
    led: Ledger,
    changed: bool,
    pending: u32,
    key_mismatch: bool,
    warning: Option<String>,
    removed: Vec<String>,
) -> Outcome {
    let _ = ledger::save(app, &led);
    Outcome {
        changed,
        pending,
        dead_letters: led.dead.len() as u32,
        key_mismatch,
        pages: led
            .objects
            .keys()
            .filter(|k| k.starts_with("page/"))
            .count() as u32,
        warning,
        removed,
    }
}

fn remote_hash(key: &[u8; crypto::KEY_LEN], cfg: &SyncConfig, remote: &RemoteObject) -> String {
    if remote.deleted {
        return String::new();
    }
    decrypt(key, cfg, remote)
        .map(|v| collect::hash(&v))
        .unwrap_or_default()
}

fn decrypt(
    key: &[u8; crypto::KEY_LEN],
    cfg: &SyncConfig,
    remote: &RemoteObject,
) -> Result<serde_json::Value, String> {
    let plain = crypto::open(
        key,
        &crypto::aad(cfg.user_id, &remote.collection, &remote.object_id),
        &remote.payload,
    )?;
    serde_json::from_slice(&plain).map_err(|e| format!("Objekt unlesbar: {e}"))
}

/// Ein Server-Objekt anwenden und das Ledger fortschreiben. Für die
/// Bibliothek gilt: war der lokale Stand gesichert, ist der Stand NACH dem
/// Anwenden der bestätigte (sonst schickte das Gerät das Zusammengeführte
/// gleich wieder zurück); war er ungesichert, bleibt er dirty und geht beim
/// nächsten Push hinaus.
fn settle(
    app: &AppHandle,
    cfg: &SyncConfig,
    key: &[u8; crypto::KEY_LEN],
    remote: &RemoteObject,
    led: &mut Ledger,
    removed: &mut Vec<String>,
) -> Result<bool, String> {
    let synced = led
        .get(&remote.collection, &remote.object_id)
        .map(|e| e.hash.clone());
    let library = is_library(&remote.collection);
    let before = if library {
        books::local_hash(
            app,
            key,
            &remote.collection,
            &remote.object_id,
            &mut led.files,
        )?
    } else {
        None
    };
    let applied = match apply_remote(app, cfg, key, remote, synced.as_deref(), &mut led.files) {
        Ok(a) => a,
        // Ein unbrauchbares Objekt (fremde Kennung, kaputter Verweis) darf
        // den Abgleich nicht anhalten: als gesehen verbuchen, überspringen.
        Err(books::ApplyError::Invalid(e)) => {
            log::warn!(
                "Sync: {}/{} übersprungen: {e}",
                remote.collection,
                remote.object_id
            );
            books::Applied::default()
        }
        // Ablage gerade nicht beschreibbar: Revision NICHT verbuchen, der
        // Lauf bricht ab und versucht es beim nächsten Takt erneut.
        Err(books::ApplyError::Retry(e)) => return Err(e),
    };
    led.uploaded.extend(applied.remote_blobs);
    removed.extend(applied.removed_keys);
    let rh = remote_hash(key, cfg, remote);
    let hash = if library && (before.is_none() || before == synced) {
        books::local_hash(
            app,
            key,
            &remote.collection,
            &remote.object_id,
            &mut led.files,
        )?
        .unwrap_or(rh)
    } else {
        rh
    };
    led.set(
        &remote.collection,
        &remote.object_id,
        Entry {
            revision: remote.revision,
            hash,
            deleted: remote.deleted,
        },
    );
    Ok(applied.changed)
}

/// Ein Server-Objekt lokal anwenden. `synced_hash` ist der zuletzt
/// bestätigte Hash des Objekts; `apply_page` vergleicht damit den FRISCHEN
/// lokalen Stand und sichert ihn bei Abweichung als Konfliktkopie.
fn apply_remote(
    app: &AppHandle,
    cfg: &SyncConfig,
    key: &[u8; crypto::KEY_LEN],
    remote: &RemoteObject,
    synced_hash: Option<&str>,
    files: &mut std::collections::BTreeMap<String, blobs::FileStamp>,
) -> Result<books::Applied, books::ApplyError> {
    let changed = |c: bool| books::Applied {
        changed: c,
        ..Default::default()
    };
    match remote.collection.as_str() {
        collect::COLLECTION_PAGE => {
            if remote.deleted {
                collect::tombstone_page(app, &remote.object_id)?;
                return Ok(changed(true));
            }
            let value = decrypt(key, cfg, remote)?;
            collect::apply_page(
                app,
                &remote.object_id,
                &value,
                synced_hash,
                &cfg.device_name,
            )?;
            Ok(changed(true))
        }
        collect::COLLECTION_SETTINGS => {
            if remote.deleted {
                return Ok(changed(false));
            }
            let value = decrypt(key, cfg, remote)?;
            collect::apply_settings(app, &value)?;
            Ok(changed(true))
        }
        books::COLLECTION_BOOK => {
            let value = (!remote.deleted)
                .then(|| decrypt(key, cfg, remote))
                .transpose()?;
            books::apply_book(
                app,
                key,
                &remote.object_id,
                value.as_ref(),
                synced_hash,
                files,
            )
        }
        books::COLLECTION_PROGRESS | books::COLLECTION_BOOKMARK => {
            let value = (!remote.deleted)
                .then(|| decrypt(key, cfg, remote))
                .transpose()?;
            books::apply_progress(
                app,
                &remote.collection,
                &remote.object_id,
                value.as_ref(),
                synced_hash,
            )
        }
        other => {
            log::warn!("Sync: unbekannte Sammlung {other} übersprungen");
            Ok(changed(false))
        }
    }
}
