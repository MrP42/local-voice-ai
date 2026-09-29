//! Der Chat-Ablauf (M4 §6, D4): Suche -> Shortlist -> Lesen -> hoechstens
//! eine Wiederholung -> Antwort -> Abdeckung. Fester Ablauf in Rust statt
//! Tool-Calling: 8 192 Kontext, 4-9B-Modelle, auf CPU kostet jede Runde
//! ein bis zwei Minuten Prefill.
//!
//! Schutz:
//! - Ein Chat-Lauf gleichzeitig (`ChatGuard`, `chat_busy`); ein laufender
//!   KI-Notizen-Lauf hat Vorrang (`enhance_running` -> `chat_busy`).
//! - Waehrend einer Aufnahme lokales LLM nur mit GPU (`recording_active_cpu`).
//! - Abbruch (`CancelFlag`) und Zeitlimit lassen das Future fallen: der
//!   Stream schliesst, NICHTS wird gespeichert (Speichern ist der letzte,
//!   synchrone Schritt ohne Await dazwischen).
//! - Kein Frage-, Antwort- oder Auszugstext im Log.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;

use super::citations::{postprocess, DeltaFilter};
use super::context::{
    bm25_rank, budget_chars, card_summary, context_budget_tokens, enhanced_excerpts,
    history_budget_chars, notes_excerpts, number_excerpts, order_for_reading, pack, rank_meetings,
    total_cost, transcript_blocks, Excerpt, ExcerptPlan, MeetingCard, MeetingRef, MAX_PER_MEETING,
    SEARCH_TOP, SECOND_ROUND_LAST_RANK, SHORTLIST,
};
use super::live::{live_plan, LiveSnapshot};
use super::prompt::{
    build_user_prompt, card_line, date_de, render_history, PromptInput, SYSTEM_PROMPT,
};
use super::recipes::{apply_filter, check_use, load_recipe, render_recipe, StoreLookup};
use super::*;
use crate::llm_client::{send_chat_completion_stream, StreamMessage};
use crate::managers::meetings::llm_call::{
    is_memory_error, resolve_provider_coded, CODE_NO_MODEL as LLM_NO_MODEL,
};
use crate::managers::meetings::notes::enhance::{parse_enhanced, DOC_KIND};
use crate::managers::meetings::notes::model::EnhancedNotes;
use crate::managers::meetings::search::chunking::TARGET_CHARS;
use crate::managers::meetings::search::hybrid::{hybrid_search, EmbedError, EmbedKind, Embedder};
use crate::managers::meetings::search::index::{ChatMessageRow, STATUS_LEXICAL, STATUS_READY};
use crate::managers::meetings::store::MeetingStore;
use crate::managers::usage::Purpose;
use crate::settings::AppSettings;

/// Obergrenze fuer einen ganzen Lauf (zwei Runden auf CPU plus Modellstart);
/// schuetzt davor, dass ein haengender Server den `ChatGuard` belegt.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
pub const MAX_QUESTION_CHARS: usize = 4_000;
/// Anteil des Budgets fuer den Ueberblick (Notizen, KI-Notizen) einer
/// Besprechung, die nicht ganz ins Budget passt.
const OVERVIEW_SHARE_PERCENT: usize = 40;
/// Anteil des Budgets, den die Auszuege der vorigen Antwort (Folgefrage)
/// hoechstens belegen.
const PREVIOUS_SHARE_PERCENT: usize = 50;

// ---------------------------------------------------------------------------
// Guard, Abbruch, Umgebung
// ---------------------------------------------------------------------------

static CHAT_RUNNING: AtomicBool = AtomicBool::new(false);

/// Hoechstens ein Chat-Lauf gleichzeitig (zwei Laeufe konkurrierten um
/// dasselbe lokale Modell). Freigabe beim Drop, auch bei Fehler, Abbruch,
/// Zeitlimit und verworfenem Future.
pub struct ChatGuard<'a> {
    flag: &'a AtomicBool,
}

/// Das Flag des einen Chat-Laufs der App (fuer `ask_guarded`).
pub fn running_flag() -> &'static AtomicBool {
    &CHAT_RUNNING
}

impl<'a> ChatGuard<'a> {
    pub fn try_acquire(flag: &'a AtomicBool) -> Result<Self, ChatError> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self { flag })
            .map_err(|_| ChatError::code_only(CODE_CHAT_BUSY))
    }
}

impl Drop for ChatGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

/// Abbruch durch den Nutzer (`meeting_chat_cancel`).
#[derive(Clone, Default, Debug)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// Was der Lauf ueber seine Umgebung wissen muss (die Command-Schicht fuellt
/// es aus App-Zustaenden; Tests setzen es direkt).
#[derive(Clone)]
pub struct ChatEnv {
    /// Kontext des lokalen Servers (`llm::DEFAULT_CONTEXT_TOKENS`).
    pub ctx_tokens: u32,
    /// Das lokale Backend ist die CPU.
    pub backend_cpu: bool,
    pub recording_active: bool,
    /// Unix-Sekunden fuer "Heute".
    pub now: i64,
    pub cancel: CancelFlag,
    /// Laeuft gerade ein KI-Notizen-Lauf? (Der hat Vorrang.)
    pub enhance_running: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// Fortschritt fuer die UI.
pub enum ChatProgress<'a> {
    /// Sichtbarer Antworttext (Marker und Denk-Bloecke gefiltert).
    Delta(&'a str),
    Stage(ChatStage, u8),
}

pub type ProgressFn<'a> = &'a (dyn Fn(ChatProgress<'_>) + Send + Sync);

/// Embedder ohne Modell: die Suche laeuft rein lexikalisch. Bis P4b den
/// Embedding-Server liefert, nimmt die Command-Schicht diesen.
pub struct LexicalOnly;

impl Embedder for LexicalOnly {
    fn embed<'a>(
        &'a self,
        _texts: &'a [String],
        _kind: EmbedKind,
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>> {
        Box::pin(async { Err(EmbedError::NoModel) })
    }

    fn model_id(&self) -> &str {
        ""
    }
}

/// Pforten vor jedem LLM-Aufruf: Aufnahme + lokales Modell auf CPU, dann ein
/// laufender KI-Notizen-Lauf.
pub fn check_gates(local: bool, env: &ChatEnv) -> Result<(), ChatError> {
    if local && env.recording_active && env.backend_cpu {
        return Err(ChatError::code_only(CODE_RECORDING_ACTIVE_CPU));
    }
    if (env.enhance_running)() {
        return Err(ChatError::new(CODE_CHAT_BUSY, "enhance_running"));
    }
    Ok(())
}

fn store_err(e: anyhow::Error) -> ChatError {
    let msg = e.to_string();
    if msg == "thread_not_found" {
        ChatError::code_only(CODE_THREAD_NOT_FOUND)
    } else if msg.ends_with("not found") {
        ChatError::code_only(CODE_MEETING_NOT_FOUND)
    } else {
        ChatError::new(CODE_STORE_FAILED, msg)
    }
}

/// Fehler des LLM-Aufrufs auf Codes; der Detailtext nennt nur die Art (der
/// Fehlerkoerper eines Servers koennte Teile der Anfrage enthalten).
pub fn map_llm_error(err: &str) -> ChatError {
    if is_memory_error(err) {
        return ChatError::code_only(CODE_MEMORY_LOW);
    }
    if err.starts_with("Modell nicht geladen") {
        return ChatError::code_only(CODE_NO_MODEL);
    }
    let kind = if let Some(pos) = err.find("status ") {
        err[pos..]
            .split(':')
            .next()
            .unwrap_or("status")
            .chars()
            .take(40)
            .collect::<String>()
    } else if err.starts_with("HTTP request failed") {
        "connection".to_string()
    } else if err.contains("Zeitlimit") {
        "timeout".to_string()
    } else {
        "failed".to_string()
    };
    ChatError::new(CODE_LLM_FAILED, kind)
}

// ---------------------------------------------------------------------------
// Ablauf
// ---------------------------------------------------------------------------

/// Ein Chat-Lauf mit Guard, Zeitlimit und Abbruch. Die Command-Schicht ruft
/// das mit dem globalen Flag (`running_flag`); Tests mit eigenem Flag.
#[allow(clippy::too_many_arguments)]
pub async fn ask_guarded(
    flag: &AtomicBool,
    settings: &AppSettings,
    store: Arc<MeetingStore>,
    embed: Arc<dyn Embedder>,
    live: Option<LiveSnapshot>,
    req: ChatRequest,
    env: &ChatEnv,
    on_progress: ProgressFn<'_>,
) -> Result<ChatAnswer, ChatError> {
    let _guard = ChatGuard::try_acquire(flag)?;
    let run = tokio::time::timeout(
        RUN_TIMEOUT,
        ask(settings, store, embed, live, req, env, on_progress),
    );
    tokio::select! {
        biased;
        _ = env.cancel.cancelled() => Err(ChatError::code_only(CODE_CANCELLED)),
        result = run => result.unwrap_or_else(|_| Err(ChatError::new(CODE_LLM_FAILED, "timeout"))),
    }
}

/// Der Ablauf ohne Guard (siehe `ask_guarded`).
#[allow(clippy::too_many_arguments)]
pub async fn ask(
    settings: &AppSettings,
    store: Arc<MeetingStore>,
    embed: Arc<dyn Embedder>,
    live: Option<LiveSnapshot>,
    req: ChatRequest,
    env: &ChatEnv,
    on_progress: ProgressFn<'_>,
) -> Result<ChatAnswer, ChatError> {
    // 1. Anfrage und Anbieter.
    let user_q = req.question.trim();
    if user_q.chars().count() > MAX_QUESTION_CHARS {
        return Err(ChatError::new(CODE_INVALID_REQUEST, "question_too_long"));
    }
    if user_q.is_empty() && req.recipe.is_none() {
        return Err(ChatError::new(CODE_INVALID_REQUEST, "question_empty"));
    }
    let (provider, model, api_key) = resolve_provider_coded(settings).map_err(|e| {
        ChatError::code_only(if e.code == LLM_NO_MODEL {
            CODE_NO_MODEL
        } else {
            CODE_NO_PROVIDER
        })
    })?;
    let local = crate::managers::llm::is_local(&provider);
    check_gates(local, env)?;

    let meeting_scope = match &req.scope {
        ChatScope::Meeting { meeting_id } => Some(meeting_id.clone()),
        ChatScope::Global { .. } => None,
    };
    let live = live.filter(|s| meeting_scope.as_deref() == Some(s.meeting.id.as_str()));

    // 2. Recipe: Text und Filter, Fehler vor jedem LLM-Aufruf.
    let (question, scope) = match &req.recipe {
        None => (user_q.to_string(), req.scope.clone()),
        Some(call) => {
            let recipe = load_recipe(&store, &call.recipe_id)?;
            check_use(&recipe.spec, meeting_scope.is_some(), live.is_some())?;
            let rendered = render_recipe(&recipe.spec, &call.values, &StoreLookup(&store))?;
            let question = if user_q.is_empty() {
                rendered.prompt
            } else {
                format!("{}\n{}", rendered.prompt, user_q)
            };
            (question, apply_filter(req.scope.clone(), &rendered.filter))
        }
    };

    // 3. Verlauf.
    let (history_rows, previous_chunks) = match &req.thread_id {
        None => (Vec::new(), Vec::new()),
        Some(id) => {
            let (thread, messages) = store
                .thread_get(id)
                .map_err(store_err)?
                .ok_or_else(|| ChatError::code_only(CODE_THREAD_NOT_FOUND))?;
            if thread.meeting_id != meeting_scope {
                return Err(ChatError::new(CODE_INVALID_REQUEST, "thread_scope"));
            }
            let previous = last_excerpt_chunks(&messages);
            (messages, previous)
        }
    };

    // 4. Budget und Suche.
    let budget = budget_chars(context_budget_tokens(
        env.ctx_tokens,
        env.backend_cpu,
        local,
    ));
    on_progress(ChatProgress::Stage(ChatStage::Searching, 1));
    let plan = match (&scope, &live) {
        (_, Some(snapshot)) => {
            let cards = vec![MeetingCard {
                label: String::new(),
                meeting: snapshot.meeting.clone(),
                folders: Vec::new(),
                summary: String::new(),
            }];
            let mut plan = live_plan(
                snapshot,
                &question,
                budget.saturating_sub(cards_cost(&cards)),
            );
            plan.cards = cards;
            plan
        }
        (ChatScope::Meeting { meeting_id }, None) => {
            meeting_plan(
                &store,
                embed.as_ref(),
                meeting_id,
                &question,
                budget,
                &previous_chunks,
            )
            .await?
        }
        (ChatScope::Global { filter }, None) => {
            global_plan(
                &store,
                embed.as_ref(),
                filter,
                &question,
                budget,
                &previous_chunks,
            )
            .await?
        }
    };
    log::info!(
        "Chat: Scope {} Besprechungen, Treffer in {}, {} Auszuege, {} Kandidaten, Budget {} Zeichen",
        plan.meetings_in_scope,
        plan.meetings_with_hits,
        plan.primary.len(),
        plan.secondary.len(),
        budget
    );

    // 5. Runden.
    let history = render_history(&history_rows, history_budget_chars());
    let today = date_de(Some(env.now));
    let live_position = live.as_ref().map(LiveSnapshot::end_ms);
    let per_meeting = meeting_scope.is_none().then_some(MAX_PER_MEETING);
    let mut coverage = Coverage {
        meetings_in_scope: plan.meetings_in_scope,
        meetings_with_hits: plan.meetings_with_hits,
        lexical_only: plan.lexical_only,
        truncated: plan.truncated,
        live: live.is_some(),
        cpu_limited: local && env.backend_cpu,
        ..Coverage::default()
    };
    let mut read_meetings: HashSet<String> = HashSet::new();
    let mut read_chunks: Vec<i64> = Vec::new();
    let mut excerpts = plan.primary;
    let mut secondary = plan.secondary;
    let mut cards = plan.cards;
    if excerpts.is_empty() {
        excerpts = pack(std::mem::take(&mut secondary), budget, per_meeting).0;
    }
    let mut outcome: (String, Vec<Citation>, bool) = (String::new(), Vec::new(), true);
    for round in 1..=2u8 {
        if round == 2 {
            let (next, rest) = pack(std::mem::take(&mut secondary), budget, per_meeting);
            if next.is_empty() {
                break;
            }
            excerpts = next;
            secondary = rest;
            cards.clear();
        }
        if excerpts.is_empty() {
            break;
        }
        number_excerpts(&mut cards, &mut excerpts);
        on_progress(ChatProgress::Stage(ChatStage::Reading, round));
        let user_prompt = build_user_prompt(&PromptInput {
            today: &today,
            live_position_ms: live_position,
            cards: &cards,
            excerpts: &excerpts,
            history: &history,
            question: &question,
        });
        on_progress(ChatProgress::Stage(ChatStage::Answering, round));
        let raw = call_llm(&provider, api_key.clone(), &model, user_prompt, on_progress).await?;
        // Inzwischen geloeschte Besprechungen: ihre Zitate fallen weg.
        let alive = drop_deleted(&store, &excerpts);
        let (text, citations, dropped, not_found) = postprocess(&raw, &alive);
        log::info!(
            "Chat: Runde {round}, Antwort {} Zeichen, {} Zitate, {} verworfen, kein Beleg: {not_found}",
            raw.chars().count(),
            citations.len(),
            dropped
        );
        coverage.rounds = round;
        coverage.dropped_citations += dropped;
        coverage.excerpts_read += excerpts.len() as u32;
        read_meetings.extend(excerpts.iter().map(|e| e.meeting_id.clone()));
        read_chunks.extend(excerpts.iter().filter_map(|e| e.chunk_id));
        outcome = (text, citations, not_found);
        if !(not_found && round == 1 && !secondary.is_empty()) {
            break;
        }
    }
    coverage.meetings_read = read_meetings.len() as u32;
    if !secondary.is_empty() {
        coverage.truncated = true;
    }
    let (text, citations, not_found) = outcome;
    let uncited = !not_found && citations.is_empty();

    // 6. Speichern (synchron, ohne Await: ein Abbruch trifft davor oder gar nicht).
    if env.cancel.is_cancelled() {
        return Err(ChatError::code_only(CODE_CANCELLED));
    }
    let thread_id = match &req.thread_id {
        Some(id) => id.clone(),
        None => {
            let scope_json = serde_json::to_string(&req.scope)
                .map_err(|e| ChatError::new(CODE_STORE_FAILED, e.to_string()))?;
            store
                .thread_create(&scope_json, meeting_scope.as_deref(), None)
                .map_err(store_err)?
                .id
        }
    };
    store
        .thread_append(&thread_id, "user", &question, None, None)
        .map_err(store_err)?;
    let meta = MessageMeta {
        coverage: coverage.clone(),
        not_found,
        uncited,
        provider_local: local,
        excerpt_chunk_ids: read_chunks,
    };
    let citations_json = serde_json::to_string(&citations)
        .map_err(|e| ChatError::new(CODE_STORE_FAILED, e.to_string()))?;
    let meta_json = serde_json::to_string(&meta)
        .map_err(|e| ChatError::new(CODE_STORE_FAILED, e.to_string()))?;
    let message = store
        .thread_append(
            &thread_id,
            "assistant",
            &text,
            Some(&citations_json),
            Some(&meta_json),
        )
        .map_err(store_err)?;
    Ok(ChatAnswer {
        thread_id,
        message_id: message.id,
        text,
        citations,
        coverage,
        not_found,
        uncited,
        provider_local: local,
    })
}

/// Die Chunk-IDs, die die letzte Antwort gelesen hat (fuer den Prompt-Cache).
fn last_excerpt_chunks(messages: &[ChatMessageRow]) -> Vec<i64> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .and_then(|m| m.coverage_json.as_deref())
        .and_then(|j| serde_json::from_str::<MessageMeta>(j).ok())
        .map(|m| m.excerpt_chunk_ids)
        .unwrap_or_default()
}

async fn call_llm(
    provider: &crate::settings::PostProcessProvider,
    api_key: String,
    model: &str,
    user_prompt: String,
    on_progress: ProgressFn<'_>,
) -> Result<String, ChatError> {
    let filter = Mutex::new(DeltaFilter::default());
    let on_delta = |delta: &str| {
        let visible = filter.lock().map(|mut f| f.push(delta)).unwrap_or(None);
        if let Some(visible) = visible {
            on_progress(ChatProgress::Delta(&visible));
        }
    };
    let messages = vec![
        StreamMessage::new("system", SYSTEM_PROMPT),
        StreamMessage::new("user", user_prompt),
    ];
    send_chat_completion_stream(Purpose::Chat, provider, api_key, model, messages, &on_delta)
        .await
        .map_err(|e| {
            let err = map_llm_error(&e);
            log::warn!("Chat: LLM-Aufruf fehlgeschlagen ({err})");
            err
        })
}

/// Auszuege ohne die Besprechungen, die inzwischen geloescht wurden.
fn drop_deleted(store: &MeetingStore, excerpts: &[Excerpt]) -> Vec<Excerpt> {
    let mut alive: HashMap<String, bool> = HashMap::new();
    excerpts
        .iter()
        .filter(|e| {
            *alive.entry(e.meeting_id.clone()).or_insert_with(|| {
                matches!(store.get_meeting(&e.meeting_id), Ok(Some(m)) if m.deleted_at.is_none())
            })
        })
        .cloned()
        .collect()
}

fn meeting_ref(store: &MeetingStore, id: &str) -> Option<MeetingRef> {
    let m = store.get_meeting(id).ok().flatten()?;
    if m.deleted_at.is_some() {
        return None;
    }
    Some(MeetingRef {
        id: m.id,
        title: m.title,
        started_at: m.started_at.or(Some(m.created_at)),
    })
}

fn latest_enhanced(store: &MeetingStore, meeting_id: &str) -> Option<EnhancedNotes> {
    let doc = store
        .get_documents(meeting_id)
        .ok()?
        .into_iter()
        .filter(|d| d.kind == DOC_KIND)
        .max_by_key(|d| d.version)?;
    parse_enhanced(&doc).ok()
}

fn folder_names(
    store: &MeetingStore,
    meeting_id: &str,
    all: &HashMap<String, String>,
) -> Vec<String> {
    store
        .meeting_folder_ids(meeting_id)
        .unwrap_or_default()
        .iter()
        .filter_map(|id| all.get(id).cloned())
        .collect()
}

fn card_for(
    store: &MeetingStore,
    meeting: MeetingRef,
    folders: &HashMap<String, String>,
) -> MeetingCard {
    MeetingCard {
        label: String::new(),
        folders: folder_names(store, &meeting.id, folders),
        summary: latest_enhanced(store, &meeting.id)
            .map(|n| card_summary(&n))
            .unwrap_or_default(),
        meeting,
    }
}

fn cards_cost(cards: &[MeetingCard]) -> usize {
    cards
        .iter()
        .map(|c| card_line(c).chars().count() + 1)
        .sum::<usize>()
        + 16
}

// ---------------------------------------------------------------------------
// Eine Besprechung
// ---------------------------------------------------------------------------

/// Passt alles (Notizen, juengste KI-Notizen, Transkript) ins Budget, wird
/// alles gelesen. Sonst: Notizen und KI-Notizen als Ueberblick (hoechstens
/// 40 %), dazu die besten Transkriptstellen mit je einem Nachbarn.
async fn meeting_plan(
    store: &MeetingStore,
    embed: &dyn Embedder,
    meeting_id: &str,
    question: &str,
    budget: usize,
    previous: &[i64],
) -> Result<ExcerptPlan, ChatError> {
    let meeting = meeting_ref(store, meeting_id)
        .ok_or_else(|| ChatError::code_only(CODE_MEETING_NOT_FOUND))?;
    let notes = match store.get_notes(meeting_id) {
        Ok(n) => n.blocks,
        Err(e) => {
            log::warn!("Chat: Notizblock nicht lesbar ({e})");
            Vec::new()
        }
    };
    let segments = store.get_segments(meeting_id).map_err(store_err)?;
    let epoch = store.segment_epoch(meeting_id).map_err(store_err)?;
    let mut overview = latest_enhanced(store, meeting_id)
        .map(|n| enhanced_excerpts(&meeting, &n))
        .unwrap_or_default();
    overview.extend(notes_excerpts(&meeting, &notes, TARGET_CHARS));
    let blocks = transcript_blocks(&meeting, &segments, epoch, TARGET_CHARS);

    let folders: HashMap<String, String> = store
        .folders_list()
        .unwrap_or_default()
        .into_iter()
        .map(|f| (f.id, f.name))
        .collect();
    let mut plan = ExcerptPlan {
        meetings_in_scope: 1,
        cards: vec![card_for(store, meeting.clone(), &folders)],
        ..ExcerptPlan::default()
    };
    let budget = budget.saturating_sub(cards_cost(&plan.cards));
    if total_cost(&overview) + total_cost(&blocks) <= budget {
        plan.meetings_with_hits = u32::from(!overview.is_empty() || !blocks.is_empty());
        plan.primary = overview;
        plan.primary.extend(blocks);
        return Ok(plan);
    }

    let (overview, _) = pack(overview, budget * OVERVIEW_SHARE_PERCENT / 100, None);
    let left = budget.saturating_sub(total_cost(&overview));
    let (ranked, lexical_only) =
        rank_meeting_transcript(store, embed, &meeting, epoch, question, &blocks, previous).await;
    plan.lexical_only = lexical_only;
    plan.meetings_with_hits = u32::from(!ranked.is_empty() || !overview.is_empty());

    // Treffer mit Nachbarn, solange Platz ist; der Rest wartet auf Runde 2.
    let mut used = 0usize;
    let mut seen: HashSet<String> = HashSet::new();
    let key = |e: &Excerpt| match e.chunk_id {
        Some(id) => format!("c{id}"),
        None => format!("b{}", e.start_ms.unwrap_or(0)),
    };
    let mut selected: Vec<Excerpt> = Vec::new();
    for (hit, neighbours) in ranked {
        if seen.contains(&key(&hit)) {
            continue;
        }
        let mut group = vec![hit.clone()];
        group.extend(neighbours.into_iter().filter(|n| !seen.contains(&key(n))));
        let group_cost = total_cost(&group);
        if used + group_cost <= left {
            used += group_cost;
        } else if used + hit.cost() <= left {
            used += hit.cost();
            group.truncate(1);
        } else {
            plan.secondary.push(hit);
            continue;
        }
        for ex in group {
            seen.insert(key(&ex));
            selected.push(ex);
        }
    }
    plan.secondary.retain(|e| !seen.contains(&key(e)));
    plan.truncated = !plan.secondary.is_empty();
    selected.sort_by_key(|e| e.start_ms.unwrap_or(0));
    plan.primary = overview;
    plan.primary.extend(selected);
    Ok(plan)
}

type RankedWithNeighbours = Vec<(Excerpt, Vec<Excerpt>)>;

/// Transkriptstellen einer Besprechung nach Relevanz, je mit Nachbarn. Mit
/// Index (aktuelle Epoche): hybride Suche nur in dieser Besprechung, Nachbar-
/// Chunks per ID +-1; Stellen der vorigen Antwort zuerst. Ohne Index (P4b
/// noch nicht gelaufen, veraltete Epoche): BM25 ueber Bloecke im Speicher.
async fn rank_meeting_transcript(
    store: &MeetingStore,
    embed: &dyn Embedder,
    meeting: &MeetingRef,
    epoch: u32,
    question: &str,
    blocks: &[Excerpt],
    previous: &[i64],
) -> (RankedWithNeighbours, bool) {
    let indexed = store
        .index_state(&meeting.id)
        .ok()
        .flatten()
        .is_some_and(|s| s.status == STATUS_LEXICAL || s.status == STATUS_READY);
    if indexed {
        let scope = [meeting.id.clone()];
        match hybrid_search(store, embed, question, &scope, SEARCH_TOP).await {
            Ok(result) => {
                let mut ids: Vec<i64> = previous.to_vec();
                ids.extend(result.hits.iter().map(|(id, _)| *id));
                let scores: HashMap<i64, f64> = result.hits.iter().copied().collect();
                let usable = |r: &crate::managers::meetings::search::index::ChunkRow| {
                    r.meeting_id == meeting.id
                        && r.source == ChunkSource::Transcript
                        && r.epoch == epoch
                };
                let rows = store.get_chunks(&ids).unwrap_or_default();
                let mut wanted: Vec<i64> = Vec::new();
                for r in rows.iter().filter(|r| usable(r)) {
                    wanted.extend([r.id - 1, r.id + 1]);
                }
                let neighbours: HashMap<i64, Excerpt> = store
                    .get_chunks(&wanted)
                    .unwrap_or_default()
                    .iter()
                    .filter(|r| usable(r))
                    .map(|r| (r.id, Excerpt::from_chunk(r, meeting)))
                    .collect();
                let mut seen = HashSet::new();
                let ranked: RankedWithNeighbours = rows
                    .iter()
                    .filter(|r| usable(r) && seen.insert(r.id))
                    .map(|r| {
                        let mut ex = Excerpt::from_chunk(r, meeting);
                        ex.score = scores.get(&r.id).copied().unwrap_or(0.0);
                        let around = [r.id - 1, r.id + 1]
                            .iter()
                            .filter_map(|id| neighbours.get(id).cloned())
                            .collect();
                        (ex, around)
                    })
                    .collect();
                if !ranked.is_empty() {
                    return (ranked, result.lexical_only);
                }
            }
            Err(e) => log::warn!("Chat: Suche in der Besprechung fehlgeschlagen ({e})"),
        }
    }
    let docs: Vec<String> = blocks.iter().map(Excerpt::body).collect();
    let ranked = bm25_rank(question, &docs)
        .into_iter()
        .map(|(ix, score)| {
            let mut ex = blocks[ix].clone();
            ex.score = score;
            let mut around = Vec::new();
            if ix > 0 {
                around.push(blocks[ix - 1].clone());
            }
            if let Some(next) = blocks.get(ix + 1) {
                around.push(next.clone());
            }
            (ex, around)
        })
        .collect();
    (ranked, true)
}

// ---------------------------------------------------------------------------
// Viele Besprechungen
// ---------------------------------------------------------------------------

/// Global (§6): Scope aufloesen, hybride Suche (top 40), Besprechungen nach
/// Punkten, die besten 8 als Karten und zum Lesen (je hoechstens 3 Stellen),
/// Raenge 9-20 als Kandidaten der Wiederholung. Stellen der vorigen Antwort
/// stehen vorn (Prompt-Cache), hoechstens die Haelfte des Budgets.
async fn global_plan(
    store: &MeetingStore,
    embed: &dyn Embedder,
    filter: &ScopeFilter,
    question: &str,
    budget: usize,
    previous: &[i64],
) -> Result<ExcerptPlan, ChatError> {
    let ids = store.resolve_scope(filter).map_err(store_err)?;
    if ids.is_empty() {
        return Err(ChatError::code_only(CODE_EMPTY_SCOPE));
    }
    let in_scope: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let result = hybrid_search(store, embed, question, &ids, SEARCH_TOP)
        .await
        .map_err(store_err)?;
    let scores: HashMap<i64, f64> = result.hits.iter().copied().collect();
    let hit_ids: Vec<i64> = result.hits.iter().map(|(id, _)| *id).collect();
    let rows = store.get_chunks(&hit_ids).map_err(store_err)?;

    let mut refs: HashMap<String, Option<MeetingRef>> = HashMap::new();
    let mut epochs: HashMap<String, u32> = HashMap::new();
    let mut current =
        |store: &MeetingStore, row: &crate::managers::meetings::search::index::ChunkRow| {
            if row.source == ChunkSource::Transcript {
                let epoch = *epochs
                    .entry(row.meeting_id.clone())
                    .or_insert_with(|| store.segment_epoch(&row.meeting_id).unwrap_or(0));
                if epoch != row.epoch {
                    return None;
                }
            }
            refs.entry(row.meeting_id.clone())
                .or_insert_with(|| meeting_ref(store, &row.meeting_id))
                .clone()
        };

    let ranking = rank_meetings(
        &rows
            .iter()
            .map(|r| {
                (
                    r.meeting_id.clone(),
                    scores.get(&r.id).copied().unwrap_or(0.0),
                )
            })
            .collect::<Vec<_>>(),
    );
    let top: Vec<&str> = ranking
        .iter()
        .take(SHORTLIST)
        .map(|(m, _)| m.as_str())
        .collect();
    let second: Vec<&str> = ranking
        .iter()
        .skip(SHORTLIST)
        .take(SECOND_ROUND_LAST_RANK - SHORTLIST)
        .map(|(m, _)| m.as_str())
        .collect();

    let mut candidates: Vec<Excerpt> = Vec::new();
    for row in &rows {
        if row.source == ChunkSource::Title {
            continue;
        }
        if let Some(meeting) = current(store, row) {
            let mut ex = Excerpt::from_chunk(row, &meeting);
            ex.score = scores.get(&row.id).copied().unwrap_or(0.0);
            candidates.push(ex);
        }
    }

    // Folgefrage: die Stellen der vorigen Antwort zuerst.
    let mut prefix: Vec<Excerpt> = Vec::new();
    if !previous.is_empty() {
        let old = store.get_chunks(previous).map_err(store_err)?;
        let old: Vec<Excerpt> = old
            .iter()
            .filter(|r| r.source != ChunkSource::Title && in_scope.contains(r.meeting_id.as_str()))
            .filter_map(|r| current(store, r).map(|m| Excerpt::from_chunk(r, &m)))
            .collect();
        prefix = pack(old, budget * PREVIOUS_SHARE_PERCENT / 100, None).0;
    }
    let taken: HashSet<i64> = prefix.iter().filter_map(|e| e.chunk_id).collect();

    let folders: HashMap<String, String> = store
        .folders_list()
        .unwrap_or_default()
        .into_iter()
        .map(|f| (f.id, f.name))
        .collect();
    let cards: Vec<MeetingCard> = top
        .iter()
        .filter_map(|id| {
            refs.get(*id)
                .cloned()
                .flatten()
                .or_else(|| meeting_ref(store, id))
        })
        .map(|m| card_for(store, m, &folders))
        .collect();
    let left = budget.saturating_sub(total_cost(&prefix) + cards_cost(&cards));

    let (mut first, mut later): (Vec<Excerpt>, Vec<Excerpt>) = candidates
        .into_iter()
        .filter(|e| !e.chunk_id.is_some_and(|id| taken.contains(&id)))
        .filter(|e| top.contains(&e.meeting_id.as_str()) || second.contains(&e.meeting_id.as_str()))
        .partition(|e| top.contains(&e.meeting_id.as_str()));
    order_for_reading(&mut first);
    order_for_reading(&mut later);
    let (read, unread_top) = pack(first, left, Some(MAX_PER_MEETING));

    let mut primary = prefix;
    primary.extend(read);
    let with_hits = ranking.len() as u32;
    Ok(ExcerptPlan {
        truncated: !unread_top.is_empty()
            || !later.is_empty()
            || ranking.len() > SECOND_ROUND_LAST_RANK,
        primary,
        secondary: later,
        cards,
        lexical_only: result.lexical_only,
        meetings_in_scope: ids.len() as u32,
        meetings_with_hits: with_hits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::llm_call::test_support::{
        chat_body, settings_with_mock_provider, spawn_llm_mock_with, MockReply,
    };
    use crate::managers::meetings::notes::model::{NoteBlock, NoteBlockKind};
    use crate::managers::meetings::search::chunking::ChunkDraft;
    use crate::managers::meetings::search::index::tests::{ready_meeting, state, tmp_store};
    use crate::managers::meetings::store::{Meeting, StoredSegment, TranscriptDelta};
    use std::sync::atomic::AtomicUsize;

    // ---- Hilfen -----------------------------------------------------------

    fn env() -> ChatEnv {
        ChatEnv {
            ctx_tokens: 8_192,
            backend_cpu: false,
            recording_active: false,
            now: 1_790_000_000,
            cancel: CancelFlag::default(),
            enhance_running: Arc::new(|| false),
        }
    }

    fn request(scope: ChatScope, question: &str) -> ChatRequest {
        ChatRequest {
            request_id: "r1".into(),
            thread_id: None,
            scope,
            question: question.into(),
            recipe: None,
        }
    }

    fn global() -> ChatScope {
        ChatScope::Global {
            filter: ScopeFilter::default(),
        }
    }

    fn no_progress() -> impl Fn(ChatProgress<'_>) + Send + Sync {
        |_| {}
    }

    /// Nutzerprompt aus dem JSON-Body einer Anfrage.
    fn user_prompt(body: &str) -> String {
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        v["messages"][1]["content"].as_str().unwrap().to_string()
    }

    /// Mock, der die Prompts mitschreibt und der Reihe nach antwortet.
    async fn mock(replies: Vec<&'static str>) -> (u16, Arc<Mutex<Vec<String>>>) {
        let prompts = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = Arc::clone(&prompts);
        let port = spawn_llm_mock_with(move |body| {
            let mut p = seen.lock().unwrap();
            p.push(user_prompt(body));
            let reply = replies.get(p.len() - 1).copied().unwrap_or("KEIN_BELEG");
            MockReply::Body(chat_body(reply))
        })
        .await;
        (port, prompts)
    }

    /// Transkript-Chunk, der an Segment `seg` haengt.
    fn chunk(text: &str, seg: u32) -> ChunkDraft {
        ChunkDraft {
            source: ChunkSource::Transcript,
            epoch: 0,
            segment_ids: vec![seg],
            ref_keys: vec![],
            document_id: None,
            start_ms: Some(u64::from(seg) * 1_000),
            end_ms: Some(u64::from(seg) * 1_000 + 900),
            channel: Some(0),
            text: format!("S{seg} 00:{:02} Ich: {text}", seg % 60),
            embed_text: text.into(),
        }
    }

    fn index(store: &MeetingStore, m: &Meeting, texts: &[(&str, u32)]) -> Vec<i64> {
        let drafts: Vec<ChunkDraft> = texts.iter().map(|(t, s)| chunk(t, *s)).collect();
        store
            .replace_meeting_chunks(
                &m.id,
                &[ChunkSource::Transcript],
                &drafts,
                &state(STATUS_LEXICAL),
            )
            .unwrap()
    }

    fn segments(store: &MeetingStore, m: &Meeting, texts: &[&str]) {
        let segs: Vec<StoredSegment> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| StoredSegment {
                segment_index: i as u32,
                text: (*t).into(),
                start_ms: i as u64 * 10_000,
                end_ms: i as u64 * 10_000 + 8_000,
                channel: 0,
                speaker_index: None,
            })
            .collect();
        store
            .append_delta(&m.id, &TranscriptDelta { new_segments: segs })
            .unwrap();
    }

    async fn run(
        settings: &AppSettings,
        store: &Arc<MeetingStore>,
        req: ChatRequest,
    ) -> Result<ChatAnswer, ChatError> {
        let flag = AtomicBool::new(false);
        let progress = no_progress();
        ask_guarded(
            &flag,
            settings,
            Arc::clone(store),
            Arc::new(LexicalOnly),
            None,
            req,
            &env(),
            &progress,
        )
        .await
    }

    // ---- Pforten ------------------------------------------------------------

    #[test]
    fn only_one_chat_runs_and_enhance_has_priority() {
        let flag = AtomicBool::new(false);
        let first = ChatGuard::try_acquire(&flag).unwrap();
        assert_eq!(
            ChatGuard::try_acquire(&flag).err().unwrap().code,
            CODE_CHAT_BUSY
        );
        drop(first);
        assert!(ChatGuard::try_acquire(&flag).is_ok(), "Freigabe beim Drop");

        let mut e = env();
        assert!(check_gates(true, &e).is_ok());
        e.enhance_running = Arc::new(|| true);
        assert_eq!(check_gates(false, &e).unwrap_err().code, CODE_CHAT_BUSY);
    }

    #[test]
    fn a_recording_blocks_only_a_local_model_on_the_cpu() {
        let mut e = env();
        e.recording_active = true;
        e.backend_cpu = true;
        assert_eq!(
            check_gates(true, &e).unwrap_err().code,
            CODE_RECORDING_ACTIVE_CPU
        );
        assert!(check_gates(false, &e).is_ok(), "externer Anbieter erlaubt");
        e.backend_cpu = false;
        assert!(check_gates(true, &e).is_ok(), "GPU erlaubt");
    }

    #[test]
    fn llm_errors_map_to_codes_without_the_server_text() {
        assert_eq!(
            map_llm_error("Zu wenig freier Arbeitsspeicher: 1 GB frei").code,
            CODE_MEMORY_LOW
        );
        assert_eq!(
            map_llm_error("Modell nicht geladen: qwen").code,
            CODE_NO_MODEL
        );
        let e = map_llm_error("API request failed with status 500 Internal Server Error: GEHEIM");
        assert_eq!(e.code, CODE_LLM_FAILED);
        assert!(!e.detail.contains("GEHEIM"), "{}", e.detail);
        assert!(e.detail.starts_with("status 500"));
        assert_eq!(
            map_llm_error("HTTP request failed: refused").detail,
            "connection"
        );
    }

    #[tokio::test]
    async fn gates_fail_before_any_llm_request() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let (port, prompts) = mock(vec!["egal"]).await;

        // Kein Anbieter.
        let mut none = crate::settings::get_default_settings();
        none.post_process_provider_id = "gibt-es-nicht".into();
        assert_eq!(
            run(&none, &store, request(global(), "Budget?"))
                .await
                .unwrap_err()
                .code,
            CODE_NO_PROVIDER
        );

        // Leerer Scope (Ordner ohne Besprechungen).
        let settings = settings_with_mock_provider(port);
        let empty = ChatScope::Global {
            filter: ScopeFilter {
                folder_id: Some("leer".into()),
                ..ScopeFilter::default()
            },
        };
        assert_eq!(
            run(&settings, &store, request(empty, "Budget?"))
                .await
                .unwrap_err()
                .code,
            CODE_EMPTY_SCOPE
        );

        // Leere Frage, unbekannte Besprechung, unbekannter Verlauf.
        assert_eq!(
            run(&settings, &store, request(global(), "  "))
                .await
                .unwrap_err()
                .code,
            CODE_INVALID_REQUEST
        );
        let gone = ChatScope::Meeting {
            meeting_id: "gibt-es-nicht".into(),
        };
        assert_eq!(
            run(&settings, &store, request(gone, "x"))
                .await
                .unwrap_err()
                .code,
            CODE_MEETING_NOT_FOUND
        );
        let mut req = request(global(), "x");
        req.thread_id = Some("gibt-es-nicht".into());
        assert_eq!(
            run(&settings, &store, req).await.unwrap_err().code,
            CODE_THREAD_NOT_FOUND
        );

        // Aufnahme + lokales Modell auf CPU.
        let mut local = crate::settings::get_default_settings();
        local.post_process_provider_id = "local".into();
        local
            .post_process_models
            .insert("local".into(), "modell".into());
        let mut e = env();
        e.recording_active = true;
        e.backend_cpu = true;
        let flag = AtomicBool::new(false);
        let progress = no_progress();
        let err = ask_guarded(
            &flag,
            &local,
            Arc::clone(&store),
            Arc::new(LexicalOnly),
            None,
            request(global(), "x"),
            &e,
            &progress,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, CODE_RECORDING_ACTIVE_CPU);

        // Pflichtvariable des Recipes fehlt.
        let m = ready_meeting(&store, "Kickoff", 1_000);
        let mut req = request(
            ChatScope::Meeting {
                meeting_id: m.id.clone(),
            },
            "",
        );
        req.recipe = Some(RecipeCall {
            recipe_id: "builtin:follow-up-mail".into(),
            values: HashMap::new(),
        });
        let err = run(&settings, &store, req).await.unwrap_err();
        assert_eq!(err.code, CODE_RECIPE_INVALID);
        assert_eq!(err.detail, "missing_variable:empfaenger");

        assert!(
            prompts.lock().unwrap().is_empty(),
            "kein einziger LLM-Aufruf"
        );
    }

    // ---- Eine Besprechung ----------------------------------------------------

    #[tokio::test]
    async fn a_meeting_that_fits_is_read_completely_and_the_answer_is_saved() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let m = ready_meeting(&store, "Nordlicht", 1_789_214_400);
        segments(
            &store,
            &m,
            &[
                "Guten Morgen zusammen.",
                "Das Budget liegt bei 5 000 Euro.",
                "Danke.",
            ],
        );
        store
            .save_notes(
                &m.id,
                &[NoteBlock {
                    id: "n1".into(),
                    kind: NoteBlockKind::Bullet,
                    text: "Angebot schicken".into(),
                    at_ms: None,
                    checked: false,
                }],
                0,
            )
            .unwrap();
        let (port, prompts) = mock(vec![
            "Das Budget liegt bei 5 000 Euro [Q2]. Unbekannt [Q7].",
        ])
        .await;
        let settings = settings_with_mock_provider(port);
        let deltas = Arc::new(Mutex::new(String::new()));
        let stages = Arc::new(Mutex::new(Vec::new()));
        let (d, s) = (Arc::clone(&deltas), Arc::clone(&stages));
        let progress = move |p: ChatProgress<'_>| match p {
            ChatProgress::Delta(t) => d.lock().unwrap().push_str(t),
            ChatProgress::Stage(stage, round) => s.lock().unwrap().push((stage, round)),
        };
        let flag = AtomicBool::new(false);
        let answer = ask_guarded(
            &flag,
            &settings,
            Arc::clone(&store),
            Arc::new(LexicalOnly),
            None,
            request(
                ChatScope::Meeting {
                    meeting_id: m.id.clone(),
                },
                "Wie hoch ist das Budget?",
            ),
            &env(),
            &progress,
        )
        .await
        .unwrap();

        assert_eq!(
            answer.text,
            "Das Budget liegt bei 5 000 Euro [1]. Unbekannt."
        );
        assert_eq!(answer.citations.len(), 1);
        let c = &answer.citations[0];
        assert_eq!(
            (c.source, c.segment_index, c.start_ms),
            (ChunkSource::Transcript, Some(1), Some(10_000))
        );
        assert_eq!(c.meeting_title, "Nordlicht");
        assert_eq!(answer.coverage.dropped_citations, 1);
        assert_eq!(answer.coverage.rounds, 1);
        assert_eq!(answer.coverage.excerpts_read, 2);
        assert_eq!(answer.coverage.meetings_read, 1);
        assert!(
            !answer.coverage.truncated
                && !answer.coverage.live
                && !answer.not_found
                && !answer.uncited
        );
        assert!(!answer.provider_local);

        let prompt = prompts.lock().unwrap()[0].clone();
        assert!(
            prompt.contains("[Q1] B1 · Meine Notizen\n- Angebot schicken"),
            "{prompt}"
        );
        assert!(
            prompt.contains("[Q2] B1 · Transkript 00:00–00:28"),
            "{prompt}"
        );
        assert!(prompt.ends_with("Frage: Wie hoch ist das Budget?"));
        assert!(
            !deltas.lock().unwrap().contains("[Q"),
            "Marker nie sichtbar"
        );
        assert_eq!(
            *stages.lock().unwrap(),
            vec![
                (ChatStage::Searching, 1),
                (ChatStage::Reading, 1),
                (ChatStage::Answering, 1)
            ]
        );

        // Gespeichert: Frage und Antwort samt Zitaten und Abdeckung.
        let (thread, messages) = store.thread_get(&answer.thread_id).unwrap().unwrap();
        assert_eq!(thread.meeting_id.as_deref(), Some(m.id.as_str()));
        assert_eq!(messages.len(), 2);
        let shown = ChatMessage::from_row(&messages[1]);
        assert_eq!(shown.text, answer.text);
        assert_eq!(shown.citations, answer.citations);
        assert_eq!(shown.coverage.as_ref(), Some(&answer.coverage));
        assert_eq!(messages[0].content, "Wie hoch ist das Budget?");
    }

    #[tokio::test]
    async fn an_uncited_answer_is_flagged_and_a_large_meeting_is_searched() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let m = ready_meeting(&store, "Lang", 1_000);
        // 400 Segmente ~ 30 000 Zeichen: passt nicht in 2 000 Token (CPU).
        let texts: Vec<String> = (0..400)
            .map(|i| {
                if i == 200 {
                    "Der Liefertermin fuer die Turbine ist der 14. Oktober.".to_string()
                } else {
                    format!("Allgemeines Gespraech ueber Punkt {i} ohne besonderen Inhalt hier.")
                }
            })
            .collect();
        segments(
            &store,
            &m,
            &texts.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let (port, prompts) = mock(vec!["Ohne jeden Beleg."]).await;
        let settings = settings_with_mock_provider(port);
        let e = env();
        let flag = AtomicBool::new(false);
        let progress = no_progress();
        let answer = ask_guarded(
            &flag,
            &settings,
            Arc::clone(&store),
            Arc::new(LexicalOnly),
            None,
            request(
                ChatScope::Meeting {
                    meeting_id: m.id.clone(),
                },
                "Wann ist der Liefertermin der Turbine?",
            ),
            &e,
            &progress,
        )
        .await
        .unwrap();
        assert!(answer.uncited && !answer.not_found);
        assert!(answer.citations.is_empty());
        let prompt = prompts.lock().unwrap()[0].clone();
        assert!(
            prompt.contains("Liefertermin fuer die Turbine"),
            "Treffer gelesen"
        );

        // Kleines Budget (etwa lokal auf CPU), ohne Index: BM25-Stellen, der
        // Rest bleibt Kandidat fuer Runde 2.
        let meeting = meeting_ref(&store, &m.id).unwrap();
        let plan = meeting_plan(
            &store,
            &LexicalOnly,
            &m.id,
            "Liefertermin Turbine",
            3_000,
            &[],
        )
        .await
        .unwrap();
        assert!(plan.lexical_only);
        let seg_ids: Vec<u32> = plan
            .primary
            .iter()
            .flat_map(|e| e.lines.iter().filter_map(|l| l.segment_index))
            .collect();
        assert!(seg_ids.contains(&200), "Treffer: {seg_ids:?}");
        assert!(total_cost(&plan.primary) <= 3_000);
        // Nur eine Stelle passt zur Frage; die Nachbarn passten nicht mehr ins Budget.
        assert!(plan.secondary.is_empty() && !plan.truncated);
        assert_eq!(plan.cards[0].meeting.title, meeting.title);
    }

    #[tokio::test]
    async fn an_indexed_meeting_is_searched_in_the_index_with_neighbour_chunks() {
        let (_dir, store) = tmp_store();
        let m = ready_meeting(&store, "Indexiert", 1_000);
        let filler = "Allgemeines ohne Bezug ".repeat(40);
        let texts: Vec<(String, u32)> = (0..30u32)
            .map(|i| {
                if i == 15 {
                    ("Die Turbine kommt am 14. Oktober".to_string(), i)
                } else {
                    (format!("{filler} Nummer {i}"), i)
                }
            })
            .collect();
        let ids = index(
            &store,
            &m,
            &texts
                .iter()
                .map(|(t, s)| (t.as_str(), *s))
                .collect::<Vec<_>>(),
        );
        // Transkript im Store lang genug, damit nicht alles ins Budget passt.
        segments(
            &store,
            &m,
            &texts.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(),
        );
        let plan = meeting_plan(&store, &LexicalOnly, &m.id, "Turbine", 4_000, &[])
            .await
            .unwrap();
        let chunk_ids: Vec<i64> = plan.primary.iter().filter_map(|e| e.chunk_id).collect();
        assert!(chunk_ids.contains(&ids[15]), "Treffer aus dem Index");
        assert!(
            chunk_ids.contains(&ids[14]) || chunk_ids.contains(&ids[16]),
            "Nachbar dabei"
        );
        let starts: Vec<u64> = plan.primary.iter().filter_map(|e| e.start_ms).collect();
        assert!(starts.windows(2).all(|w| w[0] <= w[1]), "chronologisch");
    }

    // ---- Viele Besprechungen -------------------------------------------------

    /// Zehn Besprechungen mit je einem Budget-Treffer; je kleiner `i`, desto
    /// oefter steht "Budget" darin (Rangfolge 0..9 fest).
    fn ten_meetings(store: &MeetingStore) -> Vec<Meeting> {
        (0..10)
            .map(|i| {
                let m = ready_meeting(store, &format!("Runde {i}"), 1_000 + i as i64);
                let text = format!("{}der Runde {i}", "Budget ".repeat(10 - i));
                index(store, &m, &[(text.as_str(), 1)]);
                m
            })
            .collect()
    }

    #[tokio::test]
    async fn second_round_on_not_found_with_mock_llm() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let meetings = ten_meetings(&store);
        let (port, prompts) = mock(vec!["KEIN_BELEG", "Das Budget steht [Q1]."]).await;
        let settings = settings_with_mock_provider(port);
        let answer = run(
            &settings,
            &store,
            request(global(), "Wie steht das Budget?"),
        )
        .await
        .unwrap();

        let prompts = prompts.lock().unwrap().clone();
        assert_eq!(prompts.len(), 2, "genau eine Wiederholung");
        assert_eq!(answer.coverage.rounds, 2);
        assert!(!answer.not_found);
        assert_eq!(answer.citations.len(), 1);
        // Runde 1: die 8 besten Besprechungen, Runde 2: die Raenge 9 und 10.
        assert!(prompts[0].contains("Überblick:"));
        assert!(prompts[0].contains("Runde 0") && !prompts[0].contains("der Runde 9"));
        assert!(prompts[1].contains("der Runde 8") && prompts[1].contains("der Runde 9"));
        assert!(
            !prompts[1].contains("der Runde 0"),
            "Runde 2 liest nur Ungelesenes"
        );
        let cited = &answer.citations[0].meeting_id;
        assert!(cited == &meetings[8].id || cited == &meetings[9].id);
        assert_eq!(answer.coverage.meetings_in_scope, 10);
        assert_eq!(answer.coverage.meetings_with_hits, 10);
        assert_eq!(answer.coverage.meetings_read, 10);
        assert!(answer.coverage.lexical_only);
        assert_eq!(answer.coverage.excerpts_read, 10);

        // Bleibt es bei KEIN_BELEG, ist die Antwort "nicht gefunden".
        let (port, prompts) = mock(vec!["KEIN_BELEG", "KEIN_BELEG"]).await;
        let settings = settings_with_mock_provider(port);
        let answer = run(
            &settings,
            &store,
            request(global(), "Wie steht das Budget?"),
        )
        .await
        .unwrap();
        assert!(answer.not_found && answer.text.is_empty() && answer.citations.is_empty());
        assert_eq!(prompts.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn no_second_round_without_unread_candidates() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let m = ready_meeting(&store, "Einzeln", 1_000);
        index(&store, &m, &[("Budget freigegeben", 1)]);
        let (port, prompts) = mock(vec!["KEIN_BELEG"]).await;
        let settings = settings_with_mock_provider(port);
        let answer = run(&settings, &store, request(global(), "Budget?"))
            .await
            .unwrap();
        assert!(answer.not_found);
        assert_eq!(answer.coverage.rounds, 1);
        assert_eq!(prompts.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn without_hits_the_answer_is_not_found_without_asking_the_model() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let m = ready_meeting(&store, "Anderes", 1_000);
        index(&store, &m, &[("Wetter und Urlaub", 1)]);
        let (port, prompts) = mock(vec!["egal"]).await;
        let settings = settings_with_mock_provider(port);
        let answer = run(&settings, &store, request(global(), "Budget?"))
            .await
            .unwrap();
        assert!(answer.not_found);
        assert_eq!(answer.coverage.rounds, 0);
        assert_eq!(answer.coverage.meetings_in_scope, 1);
        assert_eq!(answer.coverage.meetings_with_hits, 0);
        assert!(prompts.lock().unwrap().is_empty());
        // Auch das wird gespeichert (der Verlauf zeigt die Frage).
        assert_eq!(
            store
                .thread_get(&answer.thread_id)
                .unwrap()
                .unwrap()
                .1
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn a_follow_up_keeps_the_previous_excerpts_first_and_sees_the_history() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let a = ready_meeting(&store, "A", 1_000);
        let b = ready_meeting(&store, "B", 2_000);
        index(&store, &a, &[("Budget freigegeben fuer Nordlicht", 1)]);
        index(&store, &b, &[("Termin am Freitag mit Nordlicht", 1)]);
        let (port, prompts) = mock(vec!["Budget frei [Q1].", "Freitag [Q2]."]).await;
        let settings = settings_with_mock_provider(port);
        let first = run(&settings, &store, request(global(), "Budget?"))
            .await
            .unwrap();
        let mut req = request(global(), "Und der Termin?");
        req.thread_id = Some(first.thread_id.clone());
        let second = run(&settings, &store, req).await.unwrap();
        assert_eq!(second.thread_id, first.thread_id);

        let prompts = prompts.lock().unwrap().clone();
        let p2 = &prompts[1];
        let budget_at = p2.find("Budget freigegeben").expect("vorige Stelle");
        let termin_at = p2.find("Termin am Freitag").expect("neue Stelle");
        assert!(budget_at < termin_at, "vorige Auszuege zuerst");
        assert!(
            p2.contains("Verlauf:\nNutzer: Budget?\nAssistent: Budget frei."),
            "{p2}"
        );
        assert_eq!(second.citations[0].meeting_id, b.id);
        assert_eq!(
            store.thread_get(&first.thread_id).unwrap().unwrap().1.len(),
            4
        );
        // Ein Verlauf einer Besprechung passt nicht zu einem globalen Chat.
        let m_thread = store.thread_create("{}", Some(&a.id), None).unwrap();
        let mut req = request(global(), "x");
        req.thread_id = Some(m_thread.id);
        assert_eq!(
            run(&settings, &store, req).await.unwrap_err().code,
            CODE_INVALID_REQUEST
        );
    }

    #[tokio::test]
    async fn citations_of_a_meeting_deleted_during_the_chat_are_dropped() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let a = ready_meeting(&store, "A", 1_000);
        let b = ready_meeting(&store, "B", 2_000);
        index(&store, &a, &[("Budget Budget Budget A", 1)]);
        index(&store, &b, &[("Budget B", 1)]);
        let doomed = a.id.clone();
        let deleter = Arc::clone(&store);
        let port = spawn_llm_mock_with(move |body| {
            // Die Antwort zitiert beide Auszuege; waehrenddessen wird A geloescht.
            // Keine Asserts im Mock: ein Panik hier liesse die Anfrage haengen.
            let _ = deleter.soft_delete_meeting(&doomed);
            let _ = body;
            MockReply::Body(chat_body("Beide [Q1][Q2]."))
        })
        .await;
        let settings = settings_with_mock_provider(port);
        let answer = run(&settings, &store, request(global(), "Budget?"))
            .await
            .unwrap();
        assert_eq!(answer.citations.len(), 1);
        assert_eq!(answer.citations[0].meeting_id, b.id);
        assert_eq!(answer.coverage.dropped_citations, 1);
    }

    #[tokio::test]
    async fn a_cancelled_chat_saves_nothing() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let m = ready_meeting(&store, "A", 1_000);
        index(&store, &m, &[("Budget", 1)]);
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let port = spawn_llm_mock_with(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            MockReply::Hang
        })
        .await;
        let settings = settings_with_mock_provider(port);
        let e = env();
        let cancel = e.cancel.clone();
        let waiter = Arc::clone(&requests);
        tokio::spawn(async move {
            while waiter.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            cancel.cancel();
        });
        let flag = AtomicBool::new(false);
        let progress = no_progress();
        let err = ask_guarded(
            &flag,
            &settings,
            Arc::clone(&store),
            Arc::new(LexicalOnly),
            None,
            request(global(), "Budget?"),
            &e,
            &progress,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, CODE_CANCELLED);
        assert!(store
            .thread_list(&crate::managers::meetings::search::index::ThreadScope::Global)
            .unwrap()
            .is_empty());
        assert!(!flag.load(Ordering::SeqCst), "Guard freigegeben");
    }

    #[tokio::test]
    async fn a_live_question_uses_the_snapshot_and_a_recipe() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let m = ready_meeting(&store, "Laufend", 1_000);
        segments(
            &store,
            &m,
            &["Wir starten.", "Der Preis ist 12 Euro pro Stueck."],
        );
        let snapshot = LiveSnapshot::from_store(&store, &m.id, Some(30_000))
            .unwrap()
            .unwrap();
        let (port, prompts) = mock(vec!["Preis 12 Euro [Q1]."]).await;
        let settings = settings_with_mock_provider(port);
        let mut req = request(
            ChatScope::Meeting {
                meeting_id: m.id.clone(),
            },
            "",
        );
        req.recipe = Some(RecipeCall {
            recipe_id: "builtin:was-verpasst".into(),
            values: HashMap::new(),
        });
        let flag = AtomicBool::new(false);
        let progress = no_progress();
        let answer = ask_guarded(
            &flag,
            &settings,
            Arc::clone(&store),
            Arc::new(LexicalOnly),
            Some(snapshot.clone()),
            req,
            &env(),
            &progress,
        )
        .await
        .unwrap();
        assert!(answer.coverage.live);
        assert_eq!(answer.citations[0].segment_index, Some(1));
        let prompt = prompts.lock().unwrap()[0].clone();
        assert!(prompt.contains("Die Besprechung läuft noch; das Transkript reicht bis 00:30."));
        assert!(
            prompt.contains("Frage: Was wurde in den letzten Minuten"),
            "{prompt}"
        );

        // Ein Recipe, das waehrend der Aufnahme nicht passt, scheitert vorher.
        let mut req = request(
            ChatScope::Meeting {
                meeting_id: m.id.clone(),
            },
            "",
        );
        req.recipe = Some(RecipeCall {
            recipe_id: "builtin:follow-up-mail".into(),
            values: [("empfaenger".to_string(), "Frau Weber".to_string())].into(),
        });
        let err = ask_guarded(
            &flag,
            &settings,
            Arc::clone(&store),
            Arc::new(LexicalOnly),
            Some(snapshot),
            req,
            &env(),
            &progress,
        )
        .await
        .unwrap_err();
        assert_eq!(
            (err.code, err.detail.as_str()),
            (CODE_RECIPE_INVALID, "not_live")
        );
    }
}
