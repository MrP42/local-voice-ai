//! App-Schicht des globalen Abrufindex (Paket P2): Quellen einsammeln,
//! Freigaben anwenden, Index im Arbeitsspeicher halten, Abruf mit Audit.
//!
//! # Schalter und Freigaben (Plan M1, M2, M8)
//! - `voice_global_context_enabled` (Standard AUS): aus ⇒ Index leer, kein
//!   Aufbau, kein Vorwärmen, kein Abruf. Ausschalten leert sofort.
//! - Bereiche aus `voice_call_knowledge` (Standard leer): nur freigegebene
//!   Quellen werden eingesammelt; ein entzogener Bereich wird sofort aus dem
//!   Index entfernt (`revoke`, erhöht `epoch`).
//! - Termine, Profil, Einstellungen und Schlüssel werden nie indiziert.
//! - Gespräche der Datenklasse `restricted` oder mit besonderer Kategorie
//!   werden nie aufgenommen; eine Umklassifizierung entfernt sie.
//!
//! # Löschung
//! Vor jeder Ausgabe prüft [`still_present`] je Treffer, ob die Quelle noch
//! existiert; gelöschte Treffer fallen weg und lösen einen Abgleich aus. Ein
//! Abgleich läuft beim Gesprächsstart (`app_context_warm`), nach jeder
//! Änderung der Freigaben und höchstens alle [`RESYNC_AFTER`] im Hintergrund.
//!
//! # Grenzen
//! Inhalte stehen nie im Log und nie im Audit (Ziel `app_context ·
//! voice_orchestrator`, Argumente nur Bereiche und Trefferzahl).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tauri::{AppHandle, Manager};

use super::index::AppIndex;
use super::model::{ContextDoc, ContextHits, Jump, SourceKind};
use super::progress;
use super::retrieve::{self, Query};
use crate::agent_bridge::clients;
use crate::agent_bridge::voice::areas::{Area, Areas};
use crate::agent_bridge::voice::CALLER_TAG;
use crate::book_ai::source::SummarySource;
use crate::conversations::store::ConversationStore;
use crate::managers::history::HistoryManager;
use crate::managers::integrations::gate::{self, GateOutcome, Request};
use crate::managers::integrations::model::{Caller, Capability};
use crate::managers::meetings::store::MeetingStore;
use crate::settings::AppSettings;

/// Spätestens nach dieser Zeit gleicht ein Abruf im Hintergrund neu ab.
pub const RESYNC_AFTER: Duration = Duration::from_secs(300);
/// Höchstens so viele Diktate, Gespräche und Besprechungen je Quelle.
const MAX_DICTATIONS: usize = 1_000;
const MAX_CONVERSATIONS: usize = 300;
const MAX_MEETINGS: u32 = 500;
/// Datenklasse, die nie indiziert wird (ADR-0022).
const RESTRICTED: &str = "restricted";

static INDEX: Lazy<RwLock<AppIndex>> = Lazy::new(Default::default);
/// Nur ein Abgleich zur Zeit.
static SYNCING: AtomicBool = AtomicBool::new(false);
static LAST_SYNC: Lazy<Mutex<Option<Instant>>> = Lazy::new(Default::default);

/// Zustand für die Oberfläche (Infozeile, Einstellungen).
#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
pub struct AppContextStatus {
    pub enabled: bool,
    pub areas: Vec<SourceKind>,
    pub documents: u32,
    pub chunks: u32,
    pub epoch: u32,
    /// Dauer des letzten Abgleichs (ms); 0, wenn keiner lief.
    pub sync_ms: u32,
}

/// Anfrage an den Abruf.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct AppContextRequest {
    pub question: String,
    /// Letzte Nutzerfrage (nur bei Rückbezug verwendet).
    pub previous: Option<String>,
    /// Das laufende Gespräch: bleibt verborgen.
    pub conversation_id: Option<String>,
    /// Bücher (Seiten-Ids) mit ausdrücklicher Ganzes-Buch-Freigabe.
    #[serde(default)]
    pub whole_books: Vec<String>,
}

fn kind_of(area: Area) -> Option<SourceKind> {
    match area {
        Area::Books => Some(SourceKind::Book),
        Area::BookNotes => Some(SourceKind::BookNote),
        Area::KiDocs => Some(SourceKind::KiDoc),
        Area::Meetings => Some(SourceKind::Meeting),
        Area::Dictations => Some(SourceKind::Dictation),
        Area::Conversations => Some(SourceKind::Conversation),
        Area::Tts => Some(SourceKind::TtsPage),
        // Transkripte erweitern die Besprechungen; Termine bleiben Werkzeug.
        Area::Transcripts | Area::Calendar => None,
    }
}

/// Die freigegebenen Quellen (leer, wenn der Schalter aus ist).
pub fn allowed_kinds(settings: &AppSettings) -> HashSet<SourceKind> {
    if !settings.voice_global_context_enabled {
        return HashSet::new();
    }
    Areas::from_settings(
        &settings.voice_call_knowledge,
        settings.voice_call_read_transcripts,
    )
    .granted()
    .into_iter()
    .filter_map(kind_of)
    .collect()
}

fn read_transcripts(settings: &AppSettings) -> bool {
    Areas::from_settings(
        &settings.voice_call_knowledge,
        settings.voice_call_read_transcripts,
    )
    .has(Area::Transcripts)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn ts_ms(text: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

fn state<T: Send + Sync + 'static>(app: &AppHandle) -> Option<Arc<T>> {
    app.try_state::<Arc<T>>().map(|s| s.inner().clone())
}

// ---------------------------------------------------------------------------
// Quellen
// ---------------------------------------------------------------------------

/// Bücher der Bibliothek mit Text: (Seiten-Id, Titel).
fn library_texts(app: &AppHandle) -> Vec<(String, String)> {
    crate::library::library_list(app.clone())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|b| b.text.map(|t| (t.page_id, b.title)))
        .collect()
}

fn page_text(app: &AppHandle, page_id: &str) -> Option<String> {
    let dir = crate::commands::pages::page_path(app, page_id).ok()?;
    crate::book_ai::page_text(&dir).ok()
}

fn collect_books(app: &AppHandle, books: &[(String, String)]) -> Vec<ContextDoc> {
    books
        .iter()
        .filter_map(|(page, title)| {
            Some(ContextDoc {
                kind: SourceKind::Book,
                id: page.clone(),
                title: title.clone(),
                at_ms: 0,
                text: page_text(app, page)?,
                book_id: None,
                book_upto: None,
                restricted: false,
            })
        })
        .collect()
}

fn collect_ki_docs(app: &AppHandle, books: &[(String, String)]) -> Vec<ContextDoc> {
    let mut out = Vec::new();
    for (page, title) in books {
        let Ok(dir) = crate::commands::pages::page_path(app, page) else {
            continue;
        };
        let Ok(list) = crate::book_ai::store::list(&dir) else {
            continue;
        };
        // Länge des Buchs (UTF-16) nur, wenn ein Dokument das ganze Buch abdeckt.
        let mut whole_len: Option<usize> = None;
        for d in list {
            let Ok(text) = crate::book_ai::store::read_text(&dir, &d.id) else {
                continue;
            };
            let upto = match d.source {
                SummarySource::Range { end, .. } => Some(end as usize),
                SummarySource::Book => {
                    if whole_len.is_none() {
                        whole_len = page_text(app, page).map(|t| t.encode_utf16().count());
                    }
                    whole_len
                }
            };
            out.push(ContextDoc {
                kind: SourceKind::KiDoc,
                id: format!("{page}/{}", d.id),
                title: format!("{} – {}", d.title, title),
                at_ms: d.modified_ms as i64,
                text,
                book_id: Some(page.clone()),
                book_upto: upto,
                restricted: false,
            });
        }
    }
    out
}

fn collect_pages(app: &AppHandle, books: &[(String, String)]) -> Vec<ContextDoc> {
    let book_pages: HashSet<&String> = books.iter().map(|(p, _)| p).collect();
    crate::commands::pages::pages_snapshot(app)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !book_pages.contains(&p.id))
        .filter_map(|p| {
            let raw = crate::commands::pages::page_state_load(app.clone(), p.id.clone()).ok()?;
            let text = serde_json::from_str::<serde_json::Value>(&raw)
                .ok()?
                .get("text")?
                .as_str()?
                .to_string();
            (!text.trim().is_empty()).then(|| ContextDoc {
                kind: SourceKind::TtsPage,
                id: p.id,
                title: p.title,
                at_ms: p.modified_ms as i64,
                text,
                book_id: None,
                book_upto: None,
                restricted: false,
            })
        })
        .collect()
}

fn collect_dictations(app: &AppHandle) -> Vec<ContextDoc> {
    let Some(history) = state::<HistoryManager>(app) else {
        return Vec::new();
    };
    history
        .dictations_between(None, None, MAX_DICTATIONS)
        .unwrap_or_default()
        .into_iter()
        .map(|e| ContextDoc {
            kind: SourceKind::Dictation,
            id: e.id.to_string(),
            title: if e.title.trim().is_empty() {
                "Diktat".to_string()
            } else {
                e.title.clone()
            },
            at_ms: e.timestamp * 1000,
            text: e
                .post_processed_text
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(e.transcription_text),
            book_id: None,
            book_upto: None,
            restricted: false,
        })
        .collect()
}

fn collect_conversations(app: &AppHandle) -> Vec<ContextDoc> {
    let Some(store) = state::<ConversationStore>(app) else {
        return Vec::new();
    };
    let mut list = store.list(false).unwrap_or_default();
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    list.into_iter()
        .take(MAX_CONVERSATIONS)
        .filter_map(|c| {
            let detail = store.get(&c.id).ok()?;
            let conv = detail.conversation;
            if conv.archived {
                return None;
            }
            let restricted = conv.scope.data_class == RESTRICTED || conv.scope.special_category;
            let text = detail
                .entries
                .iter()
                .filter(|e| !e.text.trim().is_empty())
                .map(|e| format!("{}: {}", e.role, e.text))
                .collect::<Vec<_>>()
                .join("\n");
            Some(ContextDoc {
                kind: SourceKind::Conversation,
                id: conv.id,
                title: conv.title,
                at_ms: ts_ms(&conv.updated_at),
                text,
                book_id: None,
                book_upto: None,
                restricted,
            })
        })
        .collect()
}

fn collect_meetings(app: &AppHandle, transcripts: bool) -> Vec<ContextDoc> {
    let Some(store) = state::<MeetingStore>(app) else {
        return Vec::new();
    };
    store
        .list_meetings(0, MAX_MEETINGS)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.deleted_at.is_none())
        .filter_map(|m| {
            let mut text = String::new();
            if let Some(doc) = store
                .get_documents(&m.id)
                .unwrap_or_default()
                .into_iter()
                .filter(|d| d.kind == "enhanced_notes")
                .next_back()
            {
                if let Ok(md) =
                    crate::managers::meetings::notes::enhance::markdown_for(&store, &doc.id)
                {
                    text.push_str(&md);
                }
            }
            if transcripts {
                let segs = store.get_segments(&m.id).unwrap_or_default();
                if !segs.is_empty() {
                    text.push_str("\n\n");
                    for s in segs {
                        text.push_str(s.text.trim());
                        text.push('\n');
                    }
                }
            }
            (!text.trim().is_empty()).then(|| ContextDoc {
                kind: SourceKind::Meeting,
                id: m.id,
                title: m.title,
                at_ms: m.started_at.unwrap_or(m.created_at),
                text,
                book_id: None,
                book_upto: None,
                restricted: false,
            })
        })
        .collect()
}

fn collect(app: &AppHandle, kind: SourceKind, settings: &AppSettings) -> Vec<ContextDoc> {
    match kind {
        SourceKind::Book => collect_books(app, &library_texts(app)),
        SourceKind::KiDoc => collect_ki_docs(app, &library_texts(app)),
        SourceKind::TtsPage => collect_pages(app, &library_texts(app)),
        // Buchnotizen haben noch keinen Speicher im Kern (Buch-KI B5 offen).
        SourceKind::BookNote => Vec::new(),
        SourceKind::Dictation => collect_dictations(app),
        SourceKind::Conversation => collect_conversations(app),
        SourceKind::Meeting => collect_meetings(app, read_transcripts(settings)),
    }
}

// ---------------------------------------------------------------------------
// Abgleich
// ---------------------------------------------------------------------------

fn index_write() -> std::sync::RwLockWriteGuard<'static, AppIndex> {
    INDEX.write().unwrap_or_else(|e| e.into_inner())
}

fn index_read() -> std::sync::RwLockReadGuard<'static, AppIndex> {
    INDEX.read().unwrap_or_else(|e| e.into_inner())
}

/// Entfernt sofort alles, was nicht (mehr) freigegeben ist. Billig, ohne
/// Quellen zu lesen; läuft bei jeder Einstellungsänderung und vor jedem Abruf.
pub fn apply_grants(settings: &AppSettings) {
    let allowed = allowed_kinds(settings);
    let mut idx = index_write();
    if allowed.is_empty() {
        if !idx.is_empty() || idx.doc_count() > 0 {
            idx.revoke(None);
        }
        return;
    }
    for kind in SourceKind::ALL {
        if !allowed.contains(&kind) && idx.has_kind(kind) {
            idx.revoke(Some(kind));
        }
    }
}

/// Gleicht alle freigegebenen Quellen ab (blockierend). Liefert die Dauer.
pub fn sync(app: &AppHandle) -> Option<Duration> {
    if SYNCING.swap(true, Ordering::SeqCst) {
        return None;
    }
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            SYNCING.store(false, Ordering::SeqCst);
        }
    }
    let _done = Done;
    let started = Instant::now();
    let settings = crate::settings::get_settings(app);
    apply_grants(&settings);
    for kind in allowed_kinds(&settings) {
        // Quellen außerhalb der Sperre lesen; nur das Einpflegen sperrt.
        let docs = collect(app, kind, &settings);
        // Freigabe könnte sich während des Lesens geändert haben.
        let now = crate::settings::get_settings(app);
        if !allowed_kinds(&now).contains(&kind) {
            continue;
        }
        index_write().sync_kind(kind, &docs);
    }
    apply_grants(&crate::settings::get_settings(app));
    *LAST_SYNC.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    Some(started.elapsed())
}

fn sync_in_background(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        sync(&app);
    });
}

fn stale() -> bool {
    LAST_SYNC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_none_or(|t| t.elapsed() > RESYNC_AFTER)
}

/// Nach einer Änderung der Freigaben (Einstellungen): sofort entziehen,
/// dann im Hintergrund neu einsammeln.
pub fn on_settings_changed(app: &AppHandle) {
    let settings = crate::settings::get_settings(app);
    apply_grants(&settings);
    if settings.voice_global_context_enabled && !allowed_kinds(&settings).is_empty() {
        sync_in_background(app);
    }
}

/// Gibt es die Quelle eines Treffers noch? (Löschung vor der Ausgabe prüfen.)
fn still_present(app: &AppHandle, jump: &Jump) -> bool {
    match jump.kind {
        SourceKind::Book | SourceKind::TtsPage => crate::commands::pages::page_path(app, &jump.id)
            .map(|p| p.join("state.json").exists())
            .unwrap_or(false),
        SourceKind::KiDoc => {
            let Some((page, doc)) = jump.id.split_once('/') else {
                return false;
            };
            crate::commands::pages::page_path(app, page)
                .ok()
                .and_then(|dir| crate::book_ai::store::read_text(&dir, doc).ok())
                .is_some()
        }
        SourceKind::Conversation => state::<ConversationStore>(app)
            .and_then(|s| s.get(&jump.id).ok())
            .is_some_and(|d| {
                !d.conversation.archived
                    && d.conversation.scope.data_class != RESTRICTED
                    && !d.conversation.scope.special_category
            }),
        SourceKind::Meeting => state::<MeetingStore>(app)
            .and_then(|s| s.get_meeting(&jump.id).ok().flatten())
            .is_some_and(|m| m.deleted_at.is_none()),
        // Diktate: kein Einzelabruf im Verlauf; Löschungen greifen mit dem
        // nächsten Abgleich (höchstens RESYNC_AFTER).
        SourceKind::Dictation | SourceKind::BookNote => true,
    }
}

fn status(settings: &AppSettings, sync_ms: u32) -> AppContextStatus {
    let idx = index_read();
    let mut areas: Vec<SourceKind> = allowed_kinds(settings).into_iter().collect();
    areas.sort();
    AppContextStatus {
        enabled: settings.voice_global_context_enabled,
        areas,
        documents: idx.doc_count() as u32,
        chunks: idx.len() as u32,
        epoch: idx.epoch(),
        sync_ms,
    }
}

// ---------------------------------------------------------------------------
// Befehle
// ---------------------------------------------------------------------------

/// Vorwärmen beim Gesprächsstart: Index im Hintergrund aufbauen bzw. abgleichen.
/// Schalter aus oder keine Bereiche: nichts geschieht (Index wird geleert).
#[tauri::command]
#[specta::specta]
pub async fn app_context_warm(app: AppHandle) -> Result<AppContextStatus, String> {
    let settings = crate::settings::get_settings(&app);
    apply_grants(&settings);
    if allowed_kinds(&settings).is_empty() {
        return Ok(status(&settings, 0));
    }
    let a = app.clone();
    let took = tauri::async_runtime::spawn_blocking(move || sync(&a))
        .await
        .map_err(|e| e.to_string())?;
    Ok(status(
        &crate::settings::get_settings(&app),
        took.map_or(0, |d| d.as_millis() as u32),
    ))
}

/// Zustand des Index (Einstellungen, Infozeile).
#[tauri::command]
#[specta::specta]
pub fn app_context_status(app: AppHandle) -> AppContextStatus {
    status(&crate::settings::get_settings(&app), 0)
}

/// Schalter „Globales Wissen im Sprachgespräch“ (Standard aus).
#[tauri::command]
#[specta::specta]
pub fn change_voice_global_context_enabled_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.voice_global_context_enabled = enabled;
    crate::settings::write_settings(&app, settings);
    on_settings_changed(&app);
    Ok(())
}

/// Abruf für eine Frage. Lokal, ohne Modell. Jeder Abruf läuft durch das Tor
/// mit Audit (ohne Inhalte). Schalter aus oder keine Bereiche: leeres Ergebnis
/// ohne Audit (es wurde nichts gelesen).
#[tauri::command]
#[specta::specta]
pub async fn app_context_retrieve(
    app: AppHandle,
    req: AppContextRequest,
) -> Result<ContextHits, String> {
    tauri::async_runtime::spawn_blocking(move || retrieve_blocking(&app, &req))
        .await
        .map_err(|e| e.to_string())?
}

fn retrieve_blocking(app: &AppHandle, req: &AppContextRequest) -> Result<ContextHits, String> {
    let started = Instant::now();
    let settings = crate::settings::get_settings(app);
    apply_grants(&settings);
    let allowed = allowed_kinds(&settings);
    if allowed.is_empty() {
        return Ok(ContextHits {
            epoch: index_read().epoch(),
            ..Default::default()
        });
    }
    if stale() {
        sync_in_background(app);
    }
    let pages: Vec<String> = library_texts(app).into_iter().map(|(p, _)| p).collect();
    let progress_map = crate::library::library_progress_load(app.clone()).unwrap_or_default();
    let frontiers: HashMap<String, usize> = progress::frontiers(&progress_map, &pages);
    let whole: HashSet<String> = req.whole_books.iter().cloned().collect();

    let store = state::<MeetingStore>(app)
        .ok_or_else(|| "Die Datenbank ist noch nicht bereit.".to_string())?;
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    let now = now_ms();
    let integration = clients::ensure_agent_integration(&conn, now).map_err(|e| e.to_string())?;
    let mut keys: Vec<&str> = allowed.iter().map(|k| k.key()).collect();
    keys.sort_unstable();
    let audit_args = json!({ "areas": keys });
    let audit_target = format!("app_context · {CALLER_TAG}");
    let request = Request {
        caller: Caller::AgentLocal,
        integration_id: &integration.id,
        capability: Capability::KnowledgeSearch,
        target: Some(&audit_target),
        args: Some(&audit_args),
        tool_mode: None,
    };
    let outcome = gate::run_authorized(&conn, &request, now, || {
        let idx = index_read();
        let q = Query {
            question: &req.question,
            previous: req.previous.as_deref(),
            allowed: &allowed,
            frontiers: &frontiers,
            whole_books: &whole,
            k: retrieve::DEFAULT_K,
            max_chars: retrieve::DEFAULT_MAX_CHARS,
            now_ms: now,
            min_coverage: retrieve::MIN_COVERAGE,
        };
        Ok(retrieve::retrieve(&idx, &q))
    })
    .map_err(|e| e.to_string())?;
    let mut hits = match outcome {
        GateOutcome::Done(h) => h,
        GateOutcome::Failed(m) => return Err(m),
        GateOutcome::Denied { reason, .. } => return Err(reason),
        GateOutcome::Pending { .. } => return Err("Der Abruf wurde nicht ausgeführt.".into()),
    };
    // Vor der Ausgabe: laufendes Gespräch ausblenden, Gelöschtes entfernen.
    let before = hits.hits.len();
    hits.hits.retain(|h| {
        !(h.kind == SourceKind::Conversation
            && req.conversation_id.as_deref() == Some(h.jump.id.as_str()))
            && still_present(app, &h.jump)
    });
    if hits.hits.len() != before {
        sync_in_background(app);
    }
    for (i, h) in hits.hits.iter_mut().enumerate() {
        h.k = i as u32 + 1;
    }
    hits.retrieve_ms = started.elapsed().as_millis() as u32;
    Ok(hits)
}

/// Kontextnachricht für den Sprecher (gleiche Regeln wie im Kern, damit die
/// Oberfläche keinen eigenen Prompt baut). `None` ohne Treffer.
#[tauri::command]
#[specta::specta]
pub fn app_context_message(hits: ContextHits, german: bool) -> Option<String> {
    super::prompt::context_message(&hits.hits, german)
}

/// Prüft die Belege einer Antwort gegen den aktuellen Stand: Marken, die es
/// in diesem Abruf gab, deren Quelle unverändert (`version`) und deren Stand
/// (`epoch`) aktuell ist. Liefert die gültigen Belegnummern.
#[tauri::command]
#[specta::specta]
pub fn app_context_verify(answer: String, hits: ContextHits) -> Vec<u32> {
    let idx = index_read();
    if idx.epoch() != hits.epoch {
        return Vec::new();
    }
    super::prompt::cited(&answer, &hits.hits)
        .into_iter()
        .filter(|k| {
            hits.hits.iter().find(|h| h.k == *k).is_some_and(|h| {
                idx.version(&super::model::DocKey {
                    kind: h.kind,
                    id: h.jump.id.clone(),
                })
                .is_some_and(|v| format!("{v:016x}") == h.version)
            })
        })
        .collect()
}
