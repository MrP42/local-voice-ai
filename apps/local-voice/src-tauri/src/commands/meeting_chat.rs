//! Duenne Command-Huelle ueber den Chat-Motor (M4, P4c; Muster
//! `commands/meeting_enhance.rs`). Die Logik liegt in
//! `managers::meetings::chat`; hier stehen nur Argumente, Einstellungen,
//! App-Zustaende (Aufnahme, lokales Backend), das Ereignis und der Abbruch.
//!
//! Datenschutz (M1 D9): weder Frage noch Antwort noch Auszuege gelangen ins
//! Log; geloggt werden nur Codes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::managers::llm::{LlmRuntimeManager, LocalLlmServer, DEFAULT_CONTEXT_TOKENS};
use crate::managers::meetings::chat::context::clip_chars;
use crate::managers::meetings::chat::controller::{
    ask_guarded, running_flag, CancelFlag, ChatEnv, ChatProgress, LexicalOnly,
};
use crate::managers::meetings::chat::live::LiveSnapshot;
use crate::managers::meetings::chat::recipes::{
    builtin_seed_items, list_recipes, load_recipe, validate_spec, RecipeItem, RecipeSpec,
};
use crate::managers::meetings::chat::{
    ChatAnswer, ChatMessage, ChatRequest, ChatScope, ChatStage, CODE_THREAD_NOT_FOUND, EVENT_CODES,
};
use crate::managers::meetings::llm_call::resolve_provider_coded;
use crate::managers::meetings::notes::enhance::EnhanceGuard;
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::search::embed::{Embedder, LlamaEmbedder};
use crate::managers::meetings::search::index::{ChatThread, ThreadScope};
use crate::managers::meetings::store::MeetingStore;

/// Ereignis eines Chat-Laufs. `delta`: sichtbarer Antworttext in Stuecken
/// (Zitat-Marker gefiltert; die Endantwort von `meeting_chat_ask` ersetzt
/// ihn). `stage`: Phase und Runde (Runde 2 = Wiederholung, der bisherige
/// Text wird verworfen). `failed.code` ist einer von
/// `chat::EVENT_CODES` (`no_provider`, `no_model`, `memory_low`,
/// `recording_active_cpu`, `chat_busy`, `empty_scope`, `llm_failed`,
/// `cancelled`, `recipe_invalid`, `invalid_request`, `meeting_not_found`,
/// `thread_not_found`, `store_failed`).
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeetingChatEvent {
    Delta {
        request_id: String,
        text: String,
    },
    Stage {
        request_id: String,
        stage: ChatStage,
        round: u8,
    },
    Failed {
        request_id: String,
        code: String,
    },
}

// ---------------------------------------------------------------------------
// Abbruch
// ---------------------------------------------------------------------------

fn cancels() -> &'static Mutex<HashMap<String, CancelFlag>> {
    static CANCELS: OnceLock<Mutex<HashMap<String, CancelFlag>>> = OnceLock::new();
    CANCELS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Traegt den Lauf fuer `meeting_chat_cancel` ein; faellt mit dem Lauf wieder heraus.
struct CancelRegistration(String);

impl CancelRegistration {
    fn new(request_id: &str, flag: CancelFlag) -> Self {
        if let Ok(mut map) = cancels().lock() {
            map.insert(request_id.to_string(), flag);
        }
        Self(request_id.to_string())
    }
}

impl Drop for CancelRegistration {
    fn drop(&mut self) {
        if let Ok(mut map) = cancels().lock() {
            map.remove(&self.0);
        }
    }
}

fn cancel_request(request_id: &str) -> bool {
    cancels()
        .lock()
        .ok()
        .and_then(|map| map.get(request_id).cloned())
        .map(|flag| flag.cancel())
        .is_some()
}

// ---------------------------------------------------------------------------
// App-Zustaende
// ---------------------------------------------------------------------------

/// Laeuft das lokale Modell auf der CPU? Erst der laufende Server, sonst die
/// Wahl des Selbsttests; unbekannt zaehlt vorsichtig als CPU (kleineres
/// Budget, Aufnahme geschuetzt).
async fn local_backend_is_cpu(app: &AppHandle) -> bool {
    if let Some(server) = app.try_state::<Arc<LocalLlmServer>>() {
        if let Some(backend) = server.status().backend {
            return backend == "cpu";
        }
    }
    let runtime = app
        .try_state::<Arc<LlmRuntimeManager>>()
        .map(|r| Arc::clone(r.inner()));
    if let Some(runtime) = runtime {
        if let Ok((_, backend, _)) = runtime.resolve_runtime().await {
            return backend == "cpu";
        }
    }
    true
}

/// Momentaufnahme, wenn genau diese Besprechung gerade aufgenommen wird.
fn live_snapshot(
    store: &MeetingStore,
    recorder: &MeetingRecorderManager,
    scope: &ChatScope,
) -> Option<LiveSnapshot> {
    let ChatScope::Meeting { meeting_id } = scope else {
        return None;
    };
    if !recorder.is_recording() {
        return None;
    }
    let position = match recorder.position_ms() {
        Some((recording_id, pos)) if &recording_id == meeting_id => Some(pos),
        Some(_) => return None,
        // Ohne Position (WAV-Grenze erreicht): nur wenn die Besprechung noch aufnimmt.
        None => {
            let recording = store
                .get_meeting(meeting_id)
                .ok()
                .flatten()
                .is_some_and(|m| m.status == "recording");
            if !recording {
                return None;
            }
            None
        }
    };
    LiveSnapshot::from_store(store, meeting_id, position)
        .ok()
        .flatten()
}

/// Semantische Suche im Chat (Befund B5): nur mit Einstellung
/// `meeting_semantic_search` und vorhandenem BGE-M3. Waehrend einer Aufnahme
/// startet der Chat den Embedding-Server nicht selbst (Speicher und GPU
/// gehoeren dann der Transkription); laeuft er schon, darf er fragen.
fn semantic_search_usable(
    enabled: bool,
    model_ready: bool,
    recording_active: bool,
    server_running: bool,
) -> bool {
    enabled && model_ready && (!recording_active || server_running)
}

/// Der Embedder fuer die Suche des Chats: der echte (Embedding-Server mit
/// RAM-Gate und Job-Objekt, `llm::ensure_embedding`) oder rein lexikalisch.
/// Scheitert der echte zur Laufzeit (Server startet nicht, RAM knapp), faellt
/// die Suche selbst auf Worte zurueck (`hybrid_search`, `lexical_only`).
fn chat_embedder(semantic: bool) -> Arc<dyn Embedder> {
    if semantic {
        Arc::new(LlamaEmbedder::new(crate::managers::llm::EMBED_MODEL_ID))
    } else {
        Arc::new(LexicalOnly)
    }
}

/// Laeuft ein KI-Notizen-Lauf? Kurzer Griff nach dessen Guard (der hat
/// Vorrang): belegt -> ja; frei -> sofort wieder freigeben.
fn enhance_running() -> bool {
    EnhanceGuard::acquire().is_err()
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Stellt eine Frage an eine Besprechung (auch waehrend der Aufnahme) oder an
/// viele. Antworttext kommt vorab als `MeetingChatEvent::Delta`; das Ergebnis
/// ist die fertige Antwort mit Zitaten und Abdeckung (gespeichert im
/// Verlauf). Fehler: `<code>` oder `<code>: <art>` und `MeetingChatEvent::Failed`.
#[tauri::command]
#[specta::specta]
pub async fn meeting_chat_ask(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    req: ChatRequest,
) -> Result<ChatAnswer, String> {
    let store = Arc::clone(&store);
    let recorder = Arc::clone(&recorder);
    let settings = crate::settings::get_settings(&app);
    let request_id = req.request_id.clone();
    let local = resolve_provider_coded(&settings)
        .map(|(provider, _, _)| crate::managers::llm::is_local(&provider))
        .unwrap_or(false);
    let backend_cpu = local && local_backend_is_cpu(&app).await;
    let live = live_snapshot(&store, &recorder, &req.scope);
    let semantic = semantic_search_usable(
        settings.meeting_semantic_search,
        crate::managers::llm::embedding_model_ready(crate::managers::llm::EMBED_MODEL_ID),
        recorder.is_recording(),
        crate::managers::llm::embedding_running(),
    );
    let cancel = CancelFlag::default();
    let _registration = CancelRegistration::new(&request_id, cancel.clone());
    let env = ChatEnv {
        ctx_tokens: DEFAULT_CONTEXT_TOKENS,
        backend_cpu,
        recording_active: recorder.is_recording(),
        now: chrono::Utc::now().timestamp(),
        cancel,
        enhance_running: Arc::new(enhance_running),
    };
    let progress_app = app.clone();
    let progress_id = request_id.clone();
    let on_progress = move |progress: ChatProgress<'_>| {
        let event = match progress {
            ChatProgress::Delta(text) => MeetingChatEvent::Delta {
                request_id: progress_id.clone(),
                text: text.to_string(),
            },
            ChatProgress::Stage(stage, round) => MeetingChatEvent::Stage {
                request_id: progress_id.clone(),
                stage,
                round,
            },
        };
        let _ = event.emit(&progress_app);
    };
    let result = ask_guarded(
        running_flag(),
        &settings,
        store,
        chat_embedder(semantic),
        live,
        req,
        &env,
        &on_progress,
    )
    .await;
    result.map_err(|e| {
        debug_assert!(EVENT_CODES.contains(&e.code), "unbekannter Code {}", e.code);
        log::warn!("Chat fehlgeschlagen: {}", e.code);
        let _ = MeetingChatEvent::Failed {
            request_id,
            code: e.code.to_string(),
        }
        .emit(&app);
        e.to_string()
    })
}

/// Bricht einen laufenden Chat ab (nichts wird gespeichert). `false`, wenn
/// zu dieser Anfrage kein Lauf (mehr) existiert.
#[tauri::command]
#[specta::specta]
pub fn meeting_chat_cancel(request_id: String) -> bool {
    cancel_request(&request_id)
}

/// Verlaeufe eines Scopes (Besprechung oder global), zuletzt benutzte zuerst.
#[tauri::command]
#[specta::specta]
pub async fn meeting_chat_threads(
    store: State<'_, Arc<MeetingStore>>,
    scope: ChatScope,
) -> Result<Vec<ChatThread>, String> {
    let scope = match scope {
        ChatScope::Meeting { meeting_id } => ThreadScope::Meeting(meeting_id),
        ChatScope::Global { .. } => ThreadScope::Global,
    };
    store.thread_list(&scope).map_err(|e| e.to_string())
}

/// Nachrichten eines Verlaufs mit Zitaten und Abdeckung.
#[tauri::command]
#[specta::specta]
pub async fn meeting_chat_thread(
    store: State<'_, Arc<MeetingStore>>,
    thread_id: String,
) -> Result<Vec<ChatMessage>, String> {
    let (_, rows) = store
        .thread_get(&thread_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| CODE_THREAD_NOT_FOUND.to_string())?;
    Ok(rows.iter().map(ChatMessage::from_row).collect())
}

/// Loescht einen Verlauf (Nachrichten hart).
#[tauri::command]
#[specta::specta]
pub async fn meeting_chat_thread_delete(
    store: State<'_, Arc<MeetingStore>>,
    thread_id: String,
) -> Result<(), String> {
    store.thread_delete(&thread_id).map_err(|e| e.to_string())
}

static RECIPES_SEEDED: AtomicBool = AtomicBool::new(false);

/// Mitgelieferte Recipes einmal je Programmlauf in die Tabelle bringen
/// (idempotent; ein Fehler kostet nichts, die Liste kommt aus dem Code).
fn seed_recipes_once(store: &MeetingStore) {
    if RECIPES_SEEDED.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Err(e) = store.recipes_seed_builtin(&builtin_seed_items()) {
        log::warn!("Recipes: Mitgelieferte nicht gespeichert ({e})");
        RECIPES_SEEDED.store(false, Ordering::Release);
    }
}

/// Mitgelieferte Recipes zuerst, dann die eigenen.
#[tauri::command]
#[specta::specta]
pub async fn chat_recipes_list(
    store: State<'_, Arc<MeetingStore>>,
) -> Result<Vec<RecipeItem>, String> {
    seed_recipes_once(&store);
    list_recipes(&store).map_err(|e| e.to_string())
}

/// Legt ein eigenes Recipe an (`id = None`) oder aendert eins. Fehler:
/// `recipe_invalid:<grund>`, `recipe_readonly`, `recipe_not_found`.
#[tauri::command]
#[specta::specta]
pub async fn chat_recipes_save(
    store: State<'_, Arc<MeetingStore>>,
    id: Option<String>,
    title: String,
    spec: RecipeSpec,
) -> Result<RecipeItem, String> {
    save_recipe(&store, id.as_deref(), &title, spec)
}

fn save_recipe(
    store: &MeetingStore,
    id: Option<&str>,
    title: &str,
    spec: RecipeSpec,
) -> Result<RecipeItem, String> {
    validate_spec(&spec).map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&spec).map_err(|e| e.to_string())?;
    let info = store
        .recipe_save(id, title, &json)
        .map_err(|e| e.to_string())?;
    Ok(RecipeItem {
        id: info.id,
        title: info.title,
        builtin: false,
        spec,
        updated_at: info.updated_at,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn chat_recipes_delete(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<(), String> {
    store.recipe_delete(&id).map_err(|e| e.to_string())
}

/// Kopie eines (auch mitgelieferten) Recipes als eigenes, Titel mit "(Kopie)".
#[tauri::command]
#[specta::specta]
pub async fn chat_recipes_duplicate(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<RecipeItem, String> {
    duplicate_recipe(&store, &id)
}

fn duplicate_recipe(store: &MeetingStore, id: &str) -> Result<RecipeItem, String> {
    let original = load_recipe(store, id).map_err(String::from)?;
    let title = clip_chars(&format!("{} (Kopie)", original.title.trim()), 80);
    save_recipe(store, None, &title, original.spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::search::index::tests::tmp_store;

    #[test]
    fn the_event_serializes_with_a_kind_tag() {
        let delta = serde_json::to_value(MeetingChatEvent::Delta {
            request_id: "r".into(),
            text: "Teil".into(),
        })
        .unwrap();
        assert_eq!(
            delta,
            serde_json::json!({"kind": "delta", "request_id": "r", "text": "Teil"})
        );
        let stage = serde_json::to_value(MeetingChatEvent::Stage {
            request_id: "r".into(),
            stage: ChatStage::Reading,
            round: 2,
        })
        .unwrap();
        assert_eq!(
            stage,
            serde_json::json!({"kind": "stage", "request_id": "r", "stage": "reading", "round": 2})
        );
        let failed = serde_json::to_value(MeetingChatEvent::Failed {
            request_id: "r".into(),
            code: "chat_busy".into(),
        })
        .unwrap();
        assert_eq!(failed["kind"], "failed");
    }

    #[test]
    fn semantic_search_needs_setting_and_model_and_spares_a_recording() {
        assert!(semantic_search_usable(true, true, false, false));
        assert!(
            !semantic_search_usable(false, true, false, true),
            "Einstellung aus"
        );
        assert!(
            !semantic_search_usable(true, false, false, true),
            "Modell fehlt"
        );
        assert!(
            !semantic_search_usable(true, true, true, false),
            "Aufnahme: kein Serverstart"
        );
        assert!(
            semantic_search_usable(true, true, true, true),
            "Aufnahme, Server laeuft schon"
        );
    }

    #[test]
    fn the_chat_uses_the_real_embedder_or_falls_back_to_words() {
        assert_eq!(
            chat_embedder(true).model_id(),
            crate::managers::llm::EMBED_MODEL_ID
        );
        let lexical = chat_embedder(false);
        assert_eq!(lexical.model_id(), "");
    }

    #[test]
    fn cancel_reaches_only_a_registered_run_and_the_registration_ends_with_it() {
        let flag = CancelFlag::default();
        {
            let _reg = CancelRegistration::new("cmd-test-1", flag.clone());
            assert!(!cancel_request("cmd-test-andere"));
            assert!(cancel_request("cmd-test-1"));
            assert!(flag.is_cancelled());
        }
        assert!(
            !cancel_request("cmd-test-1"),
            "nach dem Lauf nicht mehr eingetragen"
        );
    }

    #[test]
    fn recipes_are_saved_validated_and_duplicated() {
        let (_dir, store) = tmp_store();
        let spec = RecipeSpec {
            version: 1,
            prompt: "Was sagt {{person}}?".into(),
            variables: vec![],
            scope: crate::managers::meetings::chat::recipes::RecipeScope::Any,
            live_ok: false,
        };
        let err = save_recipe(&store, None, "Kaputt", spec.clone()).unwrap_err();
        assert_eq!(err, "recipe_invalid:unknown_placeholder");

        let mut ok = spec;
        ok.prompt = "Was ist offen?".into();
        let saved = save_recipe(&store, None, "Offenes", ok.clone()).unwrap();
        assert!(!saved.builtin);
        assert_eq!(load_recipe(&store, &saved.id).unwrap().spec, ok);

        let copy = duplicate_recipe(&store, "builtin:einwaende").unwrap();
        assert_eq!(copy.title, "Einwände und Bedenken von Kunden (Kopie)");
        assert!(!copy.id.starts_with("builtin:"));
        assert!(save_recipe(&store, Some("builtin:einwaende"), "x", ok)
            .unwrap_err()
            .contains("recipe_readonly"));
    }
}
