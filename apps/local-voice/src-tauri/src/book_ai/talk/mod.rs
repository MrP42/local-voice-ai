//! Buch-Gespräch, Kern (Epic lva#307, Paket G1/lva#308): Fragen an ein Buch
//! per Sprache oder Text, mit Belegen und Spoilerschutz.
//!
//! Fester Ablauf ohne Tool-Calling (D2): Absicht → Suche im gelesenen Teil →
//! Auszüge `[Q<k>]` → Antwort → Satz für Satz geprüfte Belege.
//!
//! Alle deterministischen Teile (Stücke, Säubern, Tokenisierung, BM25, Q-Nummern,
//! Erstnennung, Spätwörter, Belegprüfung, Sprechfassung, Befehle) folgen dem
//! Swift-Kern (`apps/apple/VoiceCore/.../BookTalk*.swift`) als Referenz; die
//! gemeinsamen Golden-Ereignisse unter `tooling/fixtures/buch-gespraech/
//! beispiele/ereignisse/` binden beide Plattformen.
//!
//! - `index`: Stücke, Seiten, Kapitel, UTF-16-Offsets (rein).
//! - `intent`: Absicht und Sprache der Frage (rein).
//! - `commands`: Sprachbefehle ohne Modell (rein).
//! - `retrieve`: BM25-Suche mit Spoilergrenze (rein).
//! - `prompt`: Prompt mit Begrenzern, hartes Budget, Budgetprofile (rein).
//! - `cite`: satzweise Belegprüfung, Sprechfassung (rein).
//! - `flow`: Sammeln des Modellstroms, Rückfälle, Vertragsregeln (rein).
//! - `lex`: Wortwerkzeuge (rein).
//! - `store`: Verlauf `ki/gespraech.json`.
//!
//! Ereignisvertrag (`book-talk-event`, Form wie `vertrag.schema.json`):
//! je geprüftem Satz `sentence`, am Ende `done` mit der endgültigen Antwort
//! (sie ersetzt die Sätze), oder `failed` (nur `cancelled`; alles andere ist
//! eine Antwort `found_only` mit Grund). Bereich und Grenze sind je Anfrage
//! unveränderlich (Schnappschuss im `BookTalkRequest`).
//!
//! Datenschutz: kein Frage-, Antwort- oder Buchtext im Log; nur Codes,
//! Längen, Zähler. Cloud-Anbieter nur mit Freigabe (`book_talk_cloud_consent`).

pub mod adapter;
pub mod cite;
pub mod commands;
pub mod count;
pub mod eval;
pub mod flow;
pub mod index;
pub mod intent;
pub mod lex;
pub mod prompt;
pub mod retrieve;
pub mod store;
#[cfg(test)]
pub(crate) mod testkit;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::commands::pages::page_path;
use crate::managers::llm::cli::Cli;
use crate::managers::llm::evidence::Evidence;
use crate::managers::llm::preview::{
    self, Blocked, PreviewBinding, PreviewCheck, PreviewState, PreviewSwitch,
};
use crate::managers::meetings::chat::citations::strip_think;
use crate::managers::meetings::chat::context::{budget_chars, context_budget_tokens};
use crate::managers::meetings::chat::controller::{map_llm_error, CancelFlag};
use crate::managers::meetings::llm_call::resolve_provider_coded;
use crate::managers::meetings::recorder::MeetingRecorderManager;
pub use cite::BookTalkCitation;
use cite::{Grounding, CHECK_VERSION};
pub use commands::{BookTalkCommand, BookTalkCommandKind};
use flow::{Collector, Ctx, Fallback};
use index::BookIndex;
pub use intent::BookTalkIntent;
use intent::{detect_lang, Lang};
use lex::{Entity, EntityKind};
use prompt::{Budget, BudgetProfile, Turn, PROMPT_VERSION};
pub use store::StoredTurn;

// ------------------------------------------------------------------ Codes --

/// Codes des Vertrags (`FailCode`), als `failed` oder als `reason` von `found_only`.
pub const CODE_NO_MODEL: &str = "no_model";
pub const CODE_BUDGET: &str = "budget";
pub const CODE_OFFLINE: &str = "offline";
pub const CODE_CONSENT_REQUIRED: &str = "consent_required";
pub const CODE_CANCELLED: &str = "cancelled";
pub const CODE_TIMEOUT: &str = "timeout";
pub const CODE_LLM_FAILED: &str = "llm_failed";
pub const CODE_NO_FRONTIER: &str = "no_frontier";
/// Zusätzlich zum Vertrag: `reason` ohne Treffer.
pub const REASON_NO_HITS: &str = "no_hits";
/// Zwischen Modell und Anbieter: Speicher knapp (wird zu `no_model`).
const CODE_MEMORY_LOW: &str = "memory_low";

/// Fehler vor jedem Lauf (nur als `Err`-Text des Befehls, ohne Ereignis).
pub const ERR_BUSY: &str = "chat_busy";
pub const ERR_INVALID_REQUEST: &str = "invalid_request";
pub const ERR_BOOK_NOT_FOUND: &str = "book_not_found";

pub const MAX_QUESTION_CHARS: usize = 2_000;
pub const DEFAULT_MAX_ANSWER_TOKENS: u32 = 600;
/// Zeitrahmen einer Antwort auf dem Desktop (Plan §3.6).
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);
/// Kontext eines Cloud-Anbieters für die Budgetprüfung.
const REMOTE_CONTEXT_TOKENS: u32 = 32_000;

/// Fehler eines Laufs: nur der Code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalkError {
    pub code: &'static str,
}

impl TalkError {
    pub fn new(code: &'static str) -> Self {
        Self { code }
    }
}

impl std::fmt::Display for TalkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code)
    }
}

// ------------------------------------------------------------------ Typen --

/// Bereich der Suche (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BookTalkScope {
    /// Nur der gelesene oder gehörte Teil (Standard).
    UntilPosition,
    /// Das ganze Buch.
    Book,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BookTalkInput {
    Voice,
    Text,
}

/// Anfrage (Form wie `BookTalkRequest` im Vertrag). Bereich und Grenze gelten
/// für genau diese Anfrage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct BookTalkRequest {
    pub book_id: String,
    pub request_id: String,
    pub question: String,
    /// Spoilergrenze: weitester gelesener oder gehörter Stand (UTF-16 im
    /// Originaltext). Fehlt sie, gilt Schutz mit 0 (`no_frontier`).
    #[serde(default)]
    pub frontier: Option<u32>,
    pub scope: BookTalkScope,
    pub input: BookTalkInput,
}

/// Ein geprüfter Satz (Form wie `Sentence` im Vertrag).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct BookTalkSentence {
    pub index: u32,
    /// Satz ohne `[Q<k>]`-Marker.
    pub text: String,
    /// Sprechfassung, der erste belegte Satz mit gesprochenem Zitat.
    pub speech: String,
    pub citations: Vec<BookTalkCitation>,
    /// Allgemeinwissen, ohne Beleg.
    pub general: bool,
}

/// Die Antwort in zwei Formen (D5; Form wie `BookTalkAnswer` im Vertrag).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct BookTalkAnswer {
    pub intent: BookTalkIntent,
    pub sentences: Vec<BookTalkSentence>,
    pub text: String,
    /// Sprechfassung: Satz-Sprechfassungen, dann Rückfrage bzw. „bis Seite N“.
    pub speech: String,
    pub citations: Vec<BookTalkCitation>,
    /// Ohne belegte Modellantwort: nur Fundstellen oder ein Hinweis.
    pub found_only: bool,
    /// Grund für `found_only` (Codes des Vertrags oder `no_hits`); fehlt, wenn
    /// das Modell Sätze lieferte, aber keiner belegbar war.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Verworfene (unbelegte) Sätze der Modellantwort.
    pub dropped: u32,
    pub scope: BookTalkScope,
    pub frontier: u32,
    /// Die Antwort stammt aus der Abo-Vorschau (lva#334: Claude Code oder
    /// Codex ueber das Abo, Budget nur empirisch, nicht bewiesen). Die
    /// Oberflaeche kennzeichnet sie als „Vorschau“.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preview: bool,
}

/// Ereignis `book-talk-event` (Form wie `BookTalkEvent` im Vertrag).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BookTalkEvent {
    Sentence {
        request_id: String,
        sentence: BookTalkSentence,
    },
    Done {
        request_id: String,
        answer: BookTalkAnswer,
    },
    Failed {
        request_id: String,
        code: String,
    },
}

// ----------------------------------------------------------------- Modell --

pub type ModelFuture<'a> = Pin<Box<dyn Future<Output = Result<String, TalkError>> + Send + 'a>>;

/// Das Sprachmodell hinter dem Gespräch (in Tests ein Fake).
pub trait TalkModel: Send + Sync {
    fn complete<'a>(
        &'a self,
        system: &'a str,
        user: String,
        on_delta: &'a (dyn Fn(&str) + Send + Sync),
    ) -> ModelFuture<'a>;

    /// Exakte Tokenzahl von (System, Nutzer) samt Chat-Vorlage mit dem
    /// Tokenizer des Modells, falls verfügbar (darf den Server fragen). Ohne
    /// sie braucht es ein geprüftes Budgetprofil, sonst gibt es keinen Aufruf.
    fn token_count<'a>(&'a self, _system: &'a str, _user: &'a str) -> prompt::CountFuture<'a> {
        Box::pin(std::future::ready(None))
    }

    /// Kontextfenster, das das Modell tatsächlich hat (der Server meldet es),
    /// falls verfügbar. Das Budget nimmt das Kleinere.
    fn context_tokens(&self) -> prompt::CountFuture<'_> {
        Box::pin(std::future::ready(None))
    }
}

/// Was für den Modellaufruf zur Verfügung steht.
pub enum Model<'a> {
    Ready(&'a dyn TalkModel),
    /// Kein Modell, nicht freigegeben oder Speicher knapp: Antwort
    /// `found_only` mit diesem Grund (ein Code des Vertrags).
    Missing(&'static str),
}

pub type Emit<'a> = &'a (dyn Fn(BookTalkEvent) + Send + Sync);

/// Alles, was ein Lauf neben Anfrage und Index braucht.
pub struct AskParams<'a> {
    pub request: &'a BookTalkRequest,
    pub history: &'a [StoredTurn],
    /// Figuren, Orte, Ereignisse und Begriffe (nur Namen und Varianten).
    pub entities: &'a [Entity],
    pub budget: Budget,
    /// Budgetprofil der Konfiguration (`prompt::BUDGET_PROFILES`).
    pub profile: Option<&'a BudgetProfile>,
    pub timeout: Duration,
    /// Abo-Vorschau (lva#334): jede Antwort dieses Laufs traegt `preview`.
    pub preview: bool,
}

// ----------------------------------------------------------------- Ablauf --

/// Grund einer Lücke als `reason` der Antwort (Speicherknappheit heißt
/// `no_model`).
fn reason_of(code: &str) -> &str {
    if code == CODE_MEMORY_LOW {
        CODE_NO_MODEL
    } else {
        code
    }
}

async fn wait_cancel(cancel: &CancelFlag) {
    while !cancel.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Modellstrom: entfernt `<think>`-Blöcke und gibt nur Neues an den Sammler.
struct Stream<'a> {
    collector: Collector<'a>,
    raw: String,
    fed: String,
}

impl Stream<'_> {
    fn clean(&self, last: bool) -> String {
        let mut clean = strip_think(&self.raw);
        if !last {
            for k in (1.."<think>".len()).rev() {
                if clean.ends_with(&"<think>"[..k]) {
                    clean.truncate(clean.len() - k);
                    break;
                }
            }
        }
        clean
    }

    fn advance(&mut self, last: bool) -> Vec<BookTalkEvent> {
        let clean = self.clean(last);
        let Some(rest) = clean.strip_prefix(self.fed.as_str()) else {
            return Vec::new();
        };
        let rest = rest.to_string();
        let mut events = Vec::new();
        if !rest.is_empty() {
            events = self.collector.feed(&rest);
            self.fed = clean;
        }
        events
    }
}

/// Zähler eines Laufs: erst der exakte des Modells, dann das Profil.
pub struct ModelCounter<'m> {
    pub model: &'m dyn TalkModel,
    pub profile: Option<&'m BudgetProfile>,
}

impl prompt::Counter for ModelCounter<'_> {
    fn count<'a>(&'a self, system: &'a str, user: &'a str) -> prompt::CountFuture<'a> {
        Box::pin(async move {
            let exact = self.model.token_count(system, user).await;
            prompt::count_tokens(exact, self.profile, system, user)
        })
    }
}

/// Ein Lauf: Absicht, Suche, Antwort. Meldet Sätze und `done` über `emit`.
/// `Err` nur bei ungültiger Anfrage und bei Abbruch (`cancelled`); den
/// `failed`-Event dazu meldet der Aufrufer. Eine abgebrochene Antwort liefert
/// nie ein `done` und wird nie gespeichert.
pub async fn run(
    index: &BookIndex,
    p: &AskParams<'_>,
    model: Model<'_>,
    cancel: &CancelFlag,
    emit: Emit<'_>,
) -> Result<BookTalkAnswer, TalkError> {
    let req = p.request;
    let question = req.question.trim();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        return Err(TalkError::new(ERR_INVALID_REQUEST));
    }
    let done = |mut a: BookTalkAnswer| {
        a.preview = p.preview;
        emit(BookTalkEvent::Done {
            request_id: req.request_id.clone(),
            answer: a.clone(),
        });
        a
    };
    let finish = |f: Fallback| {
        for e in f.events {
            emit(e);
        }
        done(f.answer)
    };

    let lang = detect_lang(question);
    let intent = intent::detect(question);
    let frontier_u = req.frontier.unwrap_or(0);
    let book = req.scope == BookTalkScope::Book;

    let turns: Vec<Turn> = p.history.iter().map(StoredTurn::prompt_turn).collect();
    let eligible = prompt::eligible_history(&turns, req.scope, frontier_u);
    let retrieval = retrieve::retrieve(
        index,
        question,
        intent,
        req.frontier.map(|f| f as usize),
        book,
        eligible.last().map(|t| t.question.as_str()),
        p.entities,
        retrieve::DEFAULT_K,
        p.budget.excerpt_chars,
    );
    let reading_page = req
        .frontier
        .and_then(|f| index.page_at((f as usize).saturating_sub(1)))
        .or_else(|| retrieval.excerpts.last().and_then(|e| e.chunk.page));
    let ctx = Ctx {
        request: req,
        intent,
        lang,
        reading_page,
        index,
    };

    if req.scope == BookTalkScope::UntilPosition && req.frontier.is_none() {
        return Ok(finish(Fallback::no_frontier(&ctx)));
    }
    if retrieval.excerpts.is_empty() {
        return Ok(finish(Fallback::no_hits(&ctx)));
    }
    let model: &dyn TalkModel = match model {
        Model::Ready(m) => m,
        Model::Missing(code) => {
            return Ok(finish(Fallback::found_only(
                &ctx,
                &retrieval,
                Some(reason_of(code)),
                0,
            )));
        }
    };

    // Budget hart und vor dem Aufruf: exakter Zähler, sonst geprüftes Profil,
    // sonst kein Aufruf (dieselbe Logik wie im Eval).
    let counter = ModelCounter {
        model,
        profile: p.profile,
    };
    let mut budget = p.budget.clone();
    if let Some(reported) = model.context_tokens().await {
        budget.context_tokens = budget.context_tokens.min(reported);
    }
    let Some(fit) = prompt::fit(
        &index.title,
        question,
        &retrieval,
        &turns,
        lang,
        req.scope,
        frontier_u,
        &budget,
        &counter,
    )
    .await
    else {
        return Ok(finish(Fallback::found_only(
            &ctx,
            &retrieval,
            Some(CODE_BUDGET),
            0,
        )));
    };

    let late = if book {
        std::collections::HashSet::new()
    } else {
        cite::late_stems(index, frontier_u as usize)
    };
    let grounding = || Grounding {
        index,
        excerpts: &fit.retrieval.excerpts,
        entities: p.entities,
        intent,
        question,
        late_stems: &late,
        book_stems: cite::book_stems(index),
    };
    let stream = Mutex::new(Stream {
        collector: Collector::new(grounding(), lang, &req.request_id),
        raw: String::new(),
        fed: String::new(),
    });
    let on_delta = |delta: &str| {
        let events = match stream.lock() {
            Ok(mut st) => {
                st.raw.push_str(delta);
                st.advance(false)
            }
            Err(_) => Vec::new(),
        };
        for e in events {
            emit(e);
        }
    };
    let call = model.complete(&fit.parts.instructions, fit.parts.prompt.clone(), &on_delta);
    let reply = tokio::select! {
        biased;
        _ = wait_cancel(cancel) => return Err(TalkError::new(CODE_CANCELLED)),
        r = tokio::time::timeout(p.timeout, call) => {
            r.unwrap_or_else(|_| Err(TalkError::new(CODE_TIMEOUT)))
        }
    };
    if cancel.is_cancelled() {
        return Err(TalkError::new(CODE_CANCELLED));
    }
    let mut st = stream
        .into_inner()
        .map_err(|_| TalkError::new(CODE_LLM_FAILED))?;
    let (mut collector, failure) = match reply {
        Ok(full) => {
            st.raw = full;
            for e in st.advance(true) {
                emit(e);
            }
            (st.collector, None)
        }
        Err(e) if e.code == CODE_CANCELLED => return Err(e),
        // Zeitüberschreitung, Stromabbruch, Fehler: halbe Antwort verwerfen
        // (wie im Swift-Kern); `done` ersetzt die schon gemeldeten Sätze.
        Err(e) => (
            Collector::new(grounding(), lang, &req.request_id),
            Some(e.code),
        ),
    };
    for e in collector.finish() {
        emit(e);
    }

    if !collector.has_anchor {
        return Ok(finish(Fallback::found_only(
            &ctx,
            &retrieval,
            failure.map(reason_of),
            collector.dropped,
        )));
    }
    let answer = collector.answer(&ctx);
    let bad = flow::violations(&answer, index);
    if !bad.is_empty() {
        log::warn!("book_talk: Antwort verworfen ({})", bad.join(", "));
        return Ok(finish(Fallback::found_only(
            &ctx,
            &retrieval,
            Some(CODE_LLM_FAILED),
            collector.dropped,
        )));
    }
    log::info!(
        "book_talk: {} Sätze, {} Belege, {} verworfen",
        answer.sentences.len(),
        answer.citations.len(),
        answer.dropped
    );
    Ok(done(answer))
}

// ------------------------------------------------- Eval-Freigabe (Tor) --

/// Datei mit den bestandenen Eval-Läufen (Plan §3.9).
pub const EVAL_PASS_FILE: &str = "book_talk_eval_pass.json";

/// Ein Abnahmeergebnis für ein Tupel aus Anbieter, Modell, Prompt- und
/// Prüfversion und Budgetprofil. `status`: fehlend oder `passed` = bestanden,
/// `failed` = durchgefallen (Spec §11.5).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EvalPass {
    pub provider: String,
    pub model_id: String,
    pub prompt_version: String,
    pub check_version: String,
    #[serde(default)]
    pub budget_profile: Option<String>,
    pub report_hash: String,
    #[serde(default)]
    pub status: Option<String>,
}

/// Eval-Status eines Tupels (Spec §11.5, dreiwertig).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalStatus {
    Passed,
    Failed,
    Unchecked,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EvalPassFile {
    List(Vec<EvalPass>),
    Object { passes: Vec<EvalPass> },
}

/// Das aufgelöste Tupel, für das ein Eintrag nötig ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTuple {
    pub provider: String,
    pub model_id: String,
    pub budget_profile: Option<String>,
}

pub fn parse_eval_passes(raw: &str) -> Vec<EvalPass> {
    match serde_json::from_str::<EvalPassFile>(raw) {
        Ok(EvalPassFile::List(l)) | Ok(EvalPassFile::Object { passes: l }) => l,
        Err(_) => Vec::new(),
    }
}

/// Eval-Status des Tupels. Nur Einträge mit aktuellen Versionen von Prompt und
/// Prüfung zählen; `passed` zusätzlich nur mit nicht leerem `report_hash`.
/// Ein `failed`-Eintrag gewinnt. Kein passender Eintrag = `Unchecked`.
pub fn eval_status(passes: &[EvalPass], t: &ModelTuple) -> EvalStatus {
    let mut status = EvalStatus::Unchecked;
    for p in passes.iter().filter(|p| {
        p.provider == t.provider
            && p.model_id == t.model_id
            && p.budget_profile == t.budget_profile
            && p.prompt_version == PROMPT_VERSION
            && p.check_version == CHECK_VERSION
    }) {
        match p.status.as_deref().map(str::trim) {
            Some("failed") => return EvalStatus::Failed,
            None | Some("passed") if !p.report_hash.trim().is_empty() => {
                status = EvalStatus::Passed;
            }
            _ => {}
        }
    }
    status
}

/// Hat das Tupel bestanden (aktuelle Versionen)?
pub fn pass_covers(passes: &[EvalPass], t: &ModelTuple) -> bool {
    eval_status(passes, t) == EvalStatus::Passed
}

/// Darf das Tupel generierte Antworten liefern? Mit eingeschalteter Vorschau
/// (`book_talk_enabled`) auch ungeprüfte, durchgefallene nie.
pub fn answer_allowed(enabled: bool, status: EvalStatus) -> bool {
    enabled && status != EvalStatus::Failed
}

// ---------------------------------------------------- Anbieter-Anbindung --

/// Das Modell der Funktion `book_talk` über den LLM-Client.
struct ProviderModel {
    provider: crate::settings::PostProcessProvider,
    api_key: String,
    model: String,
    /// Kontext, mit dem das Budget rechnet (Ollama bekommt ihn als `num_ctx`).
    context_tokens: u32,
    /// Exakter Zähler: nur der eingebaute lokale Server hat einen.
    counter: tokio::sync::OnceCell<Option<count::LlamaCounter>>,
    /// Abo-Vorschau (lva#334): gemessenes Budget statt Zähler, gehärtete
    /// Isolation, Prüfung der Antwort vor jeder Ausgabe.
    preview: Option<PreviewCheck>,
    /// Der Vorschau-Schalter live: vor, während und nach dem Aufruf geprüft.
    switch: Option<PreviewSwitch>,
}

impl ProviderModel {
    fn new(
        provider: crate::settings::PostProcessProvider,
        api_key: String,
        model: String,
        context_tokens: u32,
        preview: Option<PreviewCheck>,
        switch: Option<PreviewSwitch>,
    ) -> Self {
        Self {
            provider,
            api_key,
            model,
            context_tokens,
            counter: tokio::sync::OnceCell::new(),
            preview,
            switch,
        }
    }

    /// Ist die Vorschau dieses Laufs (noch) eingeschaltet? Ohne Vorschau immer.
    fn switch_on(&self) -> bool {
        self.preview.is_none() || self.switch.as_ref().is_some_and(PreviewSwitch::is_on)
    }

    /// Zähler des lokalen Servers (startet ihn bei Bedarf wie der Aufruf selbst).
    async fn llama(&self) -> Option<&count::LlamaCounter> {
        if !crate::managers::llm::is_local(&self.provider) {
            return None;
        }
        self.counter
            .get_or_init(|| async {
                let base = crate::managers::llm::resolve_base_url(&self.provider, &self.model)
                    .await
                    .ok()?;
                let counter = count::LlamaCounter::new(&base)?;
                counter.context().await?;
                Some(counter)
            })
            .await
            .as_ref()
    }

    fn limits(&self) -> crate::llm_client::StreamLimits {
        crate::llm_client::StreamLimits {
            max_tokens: Some(DEFAULT_MAX_ANSWER_TOKENS),
            num_ctx: Some(self.context_tokens),
            strict: true,
        }
    }
}

impl TalkModel for ProviderModel {
    fn complete<'a>(
        &'a self,
        system: &'a str,
        user: String,
        on_delta: &'a (dyn Fn(&str) + Send + Sync),
    ) -> ModelFuture<'a> {
        Box::pin(async move {
            let messages = vec![
                crate::llm_client::StreamMessage::new("system", system),
                crate::llm_client::StreamMessage::new("user", user),
            ];
            let limits = self.limits();
            let call = crate::llm_client::send_chat_completion_stream_limited(
                crate::managers::usage::Purpose::Chat,
                &self.provider,
                self.api_key.clone(),
                &self.model,
                messages,
                on_delta,
                &limits,
            );
            // Der CLI-Pfad gibt den Text erst nach der Prüfung an `on_delta`.
            if !self.switch_on() {
                return Err(TalkError::new(CODE_CANCELLED));
            }
            let result = match (&self.preview, &self.switch) {
                (Some(check), Some(switch)) => {
                    crate::llm_client::CLI_PREVIEW_SWITCH
                        .scope(
                            switch.clone(),
                            crate::llm_client::CLI_PREVIEW.scope(check.clone(), call),
                        )
                        .await
                }
                // Vorschau ohne Schalter gibt es nicht: gesperrt.
                (Some(_), None) => return Err(TalkError::new(CODE_CANCELLED)),
                (None, _) => call.await,
            };
            // Nach dem Aufruf, vor Anzeige und Speichern: ausgeschaltet heißt verwerfen.
            if !self.switch_on() {
                return Err(TalkError::new(CODE_CANCELLED));
            }
            result.map_err(|e| {
                let code = map_error_code(&e);
                log::warn!("book_talk: LLM-Aufruf fehlgeschlagen ({code})");
                TalkError::new(code)
            })
        })
    }

    fn token_count<'a>(&'a self, system: &'a str, user: &'a str) -> prompt::CountFuture<'a> {
        Box::pin(async move {
            if let Some(check) = &self.preview {
                // Empirisch: Bytes (so, wie die CLI sie bekommt) plus gemessener Overhead.
                let bytes = system.len() + crate::llm_client::dialog_line("user", user).len();
                return usize::try_from(check.input_estimate(bytes)).ok();
            }
            self.llama().await?.count(system, user).await
        })
    }

    fn context_tokens(&self) -> prompt::CountFuture<'_> {
        Box::pin(async move {
            if let Some(check) = &self.preview {
                // Planungsgrenze: gemessene Kompaktierungsschwelle des Kontexts.
                return usize::try_from(check.limit()).ok();
            }
            self.llama().await?.context().await
        })
    }
}

/// Modellfamilie für die Budgetprofile (aus der Modell-ID).
fn family_of(model_id: &str) -> &'static str {
    if model_id.to_lowercase().contains("gemma") {
        "gemma"
    } else {
        "unknown"
    }
}

/// Fehlertext des LLM-Clients auf einen Code; der Text selbst (könnte Teile
/// der Anfrage enthalten) geht nirgends hin.
pub fn map_error_code(err: &str) -> &'static str {
    let lower = err.to_lowercase();
    // Abo-Vorschau: Budget (Kompaktierung, Überschreitung, Abschneiden) oder
    // `llm_failed` (Nutzungsangabe, Modell).
    match preview::rejection_code(err) {
        Some("budget") => return CODE_BUDGET,
        Some("cancelled") => return CODE_CANCELLED,
        Some(_) => return CODE_LLM_FAILED,
        None => {}
    }
    if err.contains("Monatsbudget") {
        return CODE_BUDGET;
    }
    if lower.contains("context length")
        || lower.contains("context window")
        || lower.contains("exceeds the context")
        || lower.contains("too many tokens")
    {
        return CODE_BUDGET;
    }
    let mapped = map_llm_error(err);
    match mapped.code {
        "memory_low" => CODE_MEMORY_LOW,
        "no_model" => CODE_NO_MODEL,
        _ if mapped.detail == "connection" => CODE_OFFLINE,
        _ if mapped.detail == "timeout" => CODE_TIMEOUT,
        _ => CODE_LLM_FAILED,
    }
}

// ------------------------------------------------------- Tauri-Befehle --

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Wie lange eine neue Frage auf eine bereits abgebrochene Vorgängerin wartet
/// (der Lauf der Vorgängerin endet gleich, der Wächter wird frei).
pub const BUSY_RETRY: Duration = Duration::from_millis(300);
const BUSY_POLL: Duration = Duration::from_millis(15);

fn current_run() -> &'static Mutex<Option<CancelFlag>> {
    static CURRENT: OnceLock<Mutex<Option<CancelFlag>>> = OnceLock::new();
    CURRENT.get_or_init(|| Mutex::new(None))
}

struct RunGuard;

impl RunGuard {
    fn try_acquire(flag: &CancelFlag) -> Result<Self, TalkError> {
        RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| {
                if let Ok(mut cur) = current_run().lock() {
                    *cur = Some(flag.clone());
                }
                Self
            })
            .map_err(|_| TalkError::new(ERR_BUSY))
    }

    /// Wächter holen. Läuft noch eine Frage, die schon abgebrochen ist, wird
    /// bis zu [`BUSY_RETRY`] gewartet statt sofort `chat_busy` zu melden.
    async fn acquire(flag: &CancelFlag) -> Result<Self, TalkError> {
        let deadline = tokio::time::Instant::now() + BUSY_RETRY;
        loop {
            match Self::try_acquire(flag) {
                Ok(guard) => return Ok(guard),
                Err(e) => {
                    let predecessor_cancelled = current_run()
                        .lock()
                        .ok()
                        .and_then(|cur| cur.as_ref().map(CancelFlag::is_cancelled))
                        .unwrap_or(false);
                    if !predecessor_cancelled || tokio::time::Instant::now() >= deadline {
                        return Err(e);
                    }
                    tokio::time::sleep(BUSY_POLL).await;
                }
            }
        }
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if let Ok(mut cur) = current_run().lock() {
            *cur = None;
        }
        RUNNING.store(false, Ordering::Release);
    }
}

/// Abbruch-Register: Flags laufender Anfragen und Grabsteine für Abbrüche, die
/// vor der Registrierung eintrafen (Abbruch und Frage überholen sich).
#[derive(Default)]
struct CancelBook {
    flags: HashMap<String, CancelFlag>,
    tombstones: std::collections::VecDeque<String>,
}

const MAX_TOMBSTONES: usize = 64;

fn cancels() -> &'static Mutex<CancelBook> {
    static CANCELS: OnceLock<Mutex<CancelBook>> = OnceLock::new();
    CANCELS.get_or_init(|| Mutex::new(CancelBook::default()))
}

/// Meldet die Anfrage an und liefert ihr Flag. Kam der Abbruch schon vorher,
/// ist das Flag bereits gesetzt (der Grabstein wird verbraucht).
struct CancelRegistration {
    id: String,
    flag: CancelFlag,
}

impl CancelRegistration {
    fn new(request_id: &str) -> Self {
        let flag = CancelFlag::default();
        if let Ok(mut book) = cancels().lock() {
            if let Some(pos) = book.tombstones.iter().position(|t| t == request_id) {
                book.tombstones.remove(pos);
                flag.cancel();
            }
            book.flags.insert(request_id.to_string(), flag.clone());
        }
        Self {
            id: request_id.to_string(),
            flag,
        }
    }
}

impl Drop for CancelRegistration {
    fn drop(&mut self) {
        if let Ok(mut book) = cancels().lock() {
            book.flags.remove(&self.id);
        }
    }
}

/// Bricht die Anfrage ab; ist sie noch nicht angemeldet, wird der Abbruch
/// vorgemerkt. `true`, wenn ein laufender Lauf getroffen wurde.
fn cancel_request(request_id: &str) -> bool {
    let Ok(mut book) = cancels().lock() else {
        return false;
    };
    if let Some(flag) = book.flags.get(request_id) {
        flag.cancel();
        return true;
    }
    if book.tombstones.len() >= MAX_TOMBSTONES {
        book.tombstones.pop_front();
    }
    book.tombstones.push_back(request_id.to_string());
    false
}

/// Antwort speichern, aber nur, wenn sie nicht abgebrochen wurde: ein Abbruch
/// nach dem Ende der Generierung verwirft die Antwort samt Verlauf
/// (Plan §3.7: Abgebrochenes gelangt nie in den gespeicherten Verlauf).
fn finish_turn(
    dir: &std::path::Path,
    req: &BookTalkRequest,
    answer: &BookTalkAnswer,
    cancel: &CancelFlag,
) -> Result<(), TalkError> {
    if cancel.is_cancelled() {
        return Err(TalkError::new(CODE_CANCELLED));
    }
    let turn = StoredTurn {
        question: req.question.trim().to_string(),
        answer: answer.clone(),
        scope: req.scope,
        frontier: req.frontier.unwrap_or(0),
        input: req.input,
        at: now_rfc3339(),
    };
    if store::append(dir, turn).is_err() {
        log::warn!("book_talk: Verlauf nicht gespeichert");
    }
    Ok(())
}

/// Index je Buch im Speicher; Neuaufbau, wenn sich der Text ändert (Hash).
fn cached_index(book_id: &str, title: &str, text: &str) -> Arc<BookIndex> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<BookIndex>>>> = OnceLock::new();
    const KEEP: usize = 4;
    let hash = index::text_hash(text);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(map) = cache.lock() {
        if let Some(hit) = map
            .get(book_id)
            .filter(|i| i.hash == hash && i.title == title)
        {
            return Arc::clone(hit);
        }
    }
    let built = Arc::new(index::build(title, text));
    if let Ok(mut map) = cache.lock() {
        if map.len() >= KEEP {
            map.clear();
        }
        map.insert(book_id.to_string(), Arc::clone(&built));
    }
    built
}

fn book_title(app: &AppHandle, book_id: &str) -> String {
    crate::commands::pages::load_index(app)
        .ok()
        .and_then(|i| i.pages.into_iter().find(|p| p.id == book_id))
        .map(|p| p.title)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Buch".to_string())
}

/// Figuren, Orte, Ereignisse und Begriffe aus `begriffe.json` (nur Namen und
/// Varianten, keine Beschreibungen) für Anfrageerweiterung und Belegprüfung.
fn entities(dir: &std::path::Path) -> Vec<Entity> {
    use crate::book_ai::terms::TermKind;
    let Ok(file) = crate::book_ai::terms::load(dir) else {
        return Vec::new();
    };
    file.terms
        .into_iter()
        .filter(|t| !t.hidden)
        .map(|t| Entity {
            name: t.term,
            variants: t.variants,
            kind: match t.kind {
                TermKind::Person => EntityKind::Person,
                TermKind::Ort => EntityKind::Place,
                TermKind::Ereignis => EntityKind::Event,
                _ => EntityKind::Term,
            },
        })
        .collect()
}

fn eval_passes(app: &AppHandle) -> Vec<EvalPass> {
    crate::portable::app_data_dir(app)
        .ok()
        .and_then(|d| std::fs::read_to_string(d.join(EVAL_PASS_FILE)).ok())
        .map(|raw| parse_eval_passes(&raw))
        .unwrap_or_default()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Frage an ein Buch. Geprüfte Sätze kommen vorab als `sentence`-Ereignis,
/// das Ergebnis ist die endgültige Antwort (auch als `done`). Bereich und
/// Grenze gelten für genau diese Anfrage. Bei Eval-Status „durchgefallen“
/// für das aufgelöste Modell, ohne Modell, ohne Cloud-Freigabe, offline oder ohne
/// Budgetnachweis kommt eine Antwort `found_only` mit Grund. `Err` nur bei
/// ungültiger Anfrage, laufender Frage und Abbruch (`cancelled`, dann
/// `failed`-Ereignis, nichts wird gespeichert). Bei ausgeschaltetem Schalter
/// `book_talk_enabled` gilt der Fundstellen-Modus (`found_only`, `no_model`).
/// Der Schalter ist die Vorschau (Spec §11.5): ungeprüfte Tupel antworten,
/// bestandene ebenso, durchgefallene nie. Das `done`-Ereignis bleibt
/// unverändert (Vertrag ohne Zusatzfeld); die Oberfläche kennzeichnet jede
/// generierte Antwort weiter als „Vorschau“.
#[tauri::command]
#[specta::specta]
pub async fn book_talk_ask(
    app: AppHandle,
    recorder: State<'_, Arc<MeetingRecorderManager>>,
    req: BookTalkRequest,
) -> Result<BookTalkAnswer, String> {
    let fail = |code: &'static str| {
        log::warn!("book_talk fehlgeschlagen: {code}");
        if code == CODE_CANCELLED {
            let _ = BookTalkEvent::Failed {
                request_id: req.request_id.clone(),
                code: code.to_string(),
            }
            .emit(&app);
        }
        code.to_string()
    };
    // Abbruch-Anmeldung vor jedem await: ein früher `book_talk_cancel` trifft
    // die Anfrage sicher (laufend oder per Grabstein vorgemerkt).
    let registration = CancelRegistration::new(&req.request_id);
    let cancel = registration.flag.clone();
    let settings = crate::settings::get_settings(&app);
    // Schalter aus: Fundstellen-Modus statt Fehler (lva#309); die Oberfläche
    // zeigt die Fundstellen mit einem Hinweis.
    let enabled = settings.book_talk_enabled;
    let _guard = RunGuard::acquire(&cancel).await.map_err(|e| fail(e.code))?;
    if cancel.is_cancelled() {
        return Err(fail(CODE_CANCELLED));
    }
    let dir = page_path(&app, &req.book_id).map_err(|_| fail(ERR_BOOK_NOT_FOUND))?;
    let text = crate::book_ai::page_text(&dir).map_err(|_| fail(ERR_BOOK_NOT_FOUND))?;
    let index = cached_index(&req.book_id, &book_title(&app, &req.book_id), &text);
    let settings = settings.for_function(crate::settings::FN_BOOK_TALK);

    let history = store::load(&dir).unwrap_or_default();
    let entities = entities(&dir);

    // Modell, Freigaben und Pforten: jede Lücke ist eine Antwort `found_only`.
    let resolved = resolve_provider_coded(&settings);
    let (local, backend_cpu) = match &resolved {
        Ok((provider, _, _)) => {
            let local = crate::managers::llm::is_local(provider);
            let cpu = local && crate::commands::meeting_chat::local_backend_is_cpu(&app).await;
            (local, cpu)
        }
        Err(_) => (false, false),
    };
    // Abo-CLI nur als Vorschau: Schalter und gebundenes Tupel müssen gelten,
    // der Kontext muss belegt sein (sonst bleibt es beim Grund `budget`).
    let preview_check = match &resolved {
        Ok((provider, model, _)) => match Cli::from_base_url(&provider.base_url) {
            Some(cli) => preview_check_for(&app, &settings, cli, model).await,
            None => None,
        },
        Err(_) => None,
    };
    let ctx_tokens = match (&preview_check, local) {
        (Some(check), _) => check.context_tokens,
        (None, true) => crate::managers::llm::DEFAULT_CONTEXT_TOKENS,
        (None, false) => REMOTE_CONTEXT_TOKENS,
    };
    let answer_tokens = preview_check
        .as_ref()
        .map_or(DEFAULT_MAX_ANSWER_TOKENS, |c| c.max_output_tokens);
    // Der Schalter wird LIVE gelesen, nicht der Stand zu Beginn der Frage.
    let switch = preview_check.as_ref().map(|_| {
        let app = app.clone();
        PreviewSwitch::new(move || crate::settings::book_talk_preview_switch_on(&app))
    });
    let provider_model = resolved.as_ref().ok().map(|(provider, model, key)| {
        ProviderModel::new(
            provider.clone(),
            key.clone(),
            model.clone(),
            ctx_tokens,
            preview_check.clone(),
            switch.clone(),
        )
    });
    let profile = resolved
        .as_ref()
        .ok()
        .and_then(|(_, model, _)| prompt::profile_for(family_of(model)));
    let eval = resolved
        .as_ref()
        .ok()
        .map(|(provider, model, _)| {
            eval_status(
                &eval_passes(&app),
                &ModelTuple {
                    provider: provider.id.clone(),
                    model_id: model.clone(),
                    budget_profile: profile.map(|p| p.name.to_string()),
                },
            )
        })
        .unwrap_or(EvalStatus::Unchecked);
    let missing: Option<&'static str> = match &resolved {
        _ if !answer_allowed(enabled, eval) => Some(CODE_NO_MODEL),
        Err(_) => Some(CODE_NO_MODEL),
        Ok(_) if !local && !has_consent(&settings) => Some(CODE_CONSENT_REQUIRED),
        Ok(_) if local && recorder.is_recording() && backend_cpu => Some(CODE_NO_MODEL),
        Ok(_) if crate::commands::meeting_chat::enhance_running() => Some(CODE_LLM_FAILED),
        Ok(_) => None,
    };
    let model = match (&provider_model, missing) {
        (_, Some(code)) => Model::Missing(code),
        (Some(m), None) => Model::Ready(m),
        (None, None) => Model::Missing(CODE_NO_MODEL),
    };

    let params = AskParams {
        request: &req,
        history: &history,
        entities: &entities,
        budget: Budget::desktop(
            ctx_tokens as usize,
            answer_tokens as usize,
            budget_chars(context_budget_tokens(ctx_tokens, backend_cpu, local)),
        ),
        profile,
        timeout: ANSWER_TIMEOUT,
        preview: preview_check.is_some(),
    };
    let emit_app = app.clone();
    let emit = move |event: BookTalkEvent| {
        let _ = event.emit(&emit_app);
    };
    let answer = run(&index, &params, model, &cancel, &emit)
        .await
        .map_err(|e| fail(e.code))?;

    // Ausgeschaltet während der Frage: Ergebnis verwerfen, nichts speichern.
    if switch.as_ref().is_some_and(|s| !s.is_on()) {
        return Err(fail(CODE_CANCELLED));
    }
    finish_turn(&dir, &req, &answer, &cancel).map_err(|e| fail(e.code))?;
    Ok(answer)
}

/// Bricht eine laufende Frage ab; es kommt nur noch `failed { cancelled }`,
/// nichts wird gespeichert. Trifft der Abbruch vor der Anmeldung der Frage
/// ein, wird er vorgemerkt und gilt für sie. `false`, wenn zu dieser Anfrage
/// kein Lauf (mehr) existiert.
#[tauri::command]
#[specta::specta]
pub fn book_talk_cancel(request_id: String) -> bool {
    cancel_request(&request_id)
}

/// Verlauf des Gesprächs mit einem Buch (älteste zuerst, höchstens 200 Wechsel).
#[tauri::command]
#[specta::specta]
pub fn book_talk_history(app: AppHandle, book_id: String) -> Result<Vec<StoredTurn>, String> {
    let dir = page_path(&app, &book_id).map_err(|_| ERR_BOOK_NOT_FOUND.to_string())?;
    store::load(&dir)
}

/// Verlauf löschen.
#[tauri::command]
#[specta::specta]
pub fn book_talk_clear(app: AppHandle, book_id: String) -> Result<(), String> {
    let dir = page_path(&app, &book_id).map_err(|_| ERR_BOOK_NOT_FOUND.to_string())?;
    store::clear(&dir)
}

/// Einstellung `book_talk_enabled` (Schalter für das Gespräch mit dem Buch).
#[tauri::command]
#[specta::specta]
pub fn change_book_talk_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.book_talk_enabled = enabled;
    crate::settings::write_settings(&app, settings);
    Ok(())
}

// ------------------------------------------------------ Abo-Vorschau (#334) --

/// Ergebnis der Prüfung der Abo-Vorschau für ein Modell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewVerdict {
    pub state: PreviewState,
    pub blocked: Option<Blocked>,
    /// Gesetzt nur bei `Active`.
    pub check: Option<PreviewCheck>,
}

/// Prüfauftrag der Abo-Vorschau für dieses Modell, falls sie wirkt: Schalter
/// an, gebundenes Tupel gültig, Isolation dieser CLI-Version vollständig
/// belegt, Messung für (CLI, Version, Programm, Modell) vorhanden, Kontext
/// belegt. Jede Lücke sperrt und nennt den Grund. Rein; `current` ist das
/// aktuelle Tupel (`None`, wenn die CLI nicht zu prüfen ist).
pub fn preview_verdict(
    settings: &crate::settings::AppSettings,
    cli: Cli,
    current: Option<PreviewBinding>,
    answer_tokens: u32,
    evidence: &Evidence,
) -> PreviewVerdict {
    let gate = preview::gate(
        settings.book_talk_subscription_preview,
        &settings.book_talk_preview_bindings,
        current.as_ref(),
        evidence,
    );
    let blocked = |state, blocked| PreviewVerdict {
        state,
        blocked,
        check: None,
    };
    let (PreviewState::Active, Some(proof), Some(binding)) = (gate.state, gate.proof, current)
    else {
        return blocked(gate.state, gate.blocked);
    };
    let configured = settings
        .active_llm_model()
        .and_then(|(_, model)| model.context_limit);
    let Some(context_tokens) = preview::context_tokens(cli, configured) else {
        return blocked(PreviewState::NotReleased, Some(Blocked::ContextUnknown));
    };
    PreviewVerdict {
        state: PreviewState::Active,
        blocked: None,
        check: Some(PreviewCheck::new(
            binding,
            context_tokens,
            answer_tokens,
            proof,
        )),
    }
}

/// Wie [`preview_verdict`], nur der Prüfauftrag (`None`: gesperrt).
pub fn preview_check(
    settings: &crate::settings::AppSettings,
    cli: Cli,
    current: Option<PreviewBinding>,
    answer_tokens: u32,
    evidence: &Evidence,
) -> Option<PreviewCheck> {
    preview_verdict(settings, cli, current, answer_tokens, evidence).check
}

/// Belege dieses Geräts (Fähigkeitstabelle und Messdatei im App-Datenordner);
/// ohne Ordner oder Dateien leer, also alles gesperrt.
pub fn load_evidence(app: &AppHandle) -> Evidence {
    crate::portable::app_data_dir(app)
        .map(|dir| Evidence::load(&dir))
        .unwrap_or_default()
}

async fn preview_check_for(
    app: &AppHandle,
    settings: &crate::settings::AppSettings,
    cli: Cli,
    model: &str,
) -> Option<PreviewCheck> {
    if !settings.book_talk_subscription_preview {
        return None;
    }
    let id = model.to_string();
    let current = tokio::task::spawn_blocking(move || preview::current_binding(cli, &id))
        .await
        .ok()
        .and_then(Result::ok);
    preview_check(
        settings,
        cli,
        current,
        DEFAULT_MAX_ANSWER_TOKENS,
        &load_evidence(app),
    )
}

/// Tupel für alle eingeschalteten Abo-Modelle eingeschalteter Verbindungen
/// (Desktop und Relay), mit der Identität, die `identity_of` je CLI liefert.
/// CLIs ohne Identität (nicht installiert) bleiben ohne Tupel.
pub fn bindings_for(
    settings: &crate::settings::AppSettings,
    identity_of: impl Fn(Cli) -> Option<preview::Identity>,
) -> Vec<PreviewBinding> {
    let mut out: Vec<PreviewBinding> = Vec::new();
    for model in settings.llm_models.iter().filter(|m| m.enabled) {
        let Some(connection) = settings
            .llm_connections
            .iter()
            .find(|c| c.id == model.connection_id && c.enabled)
        else {
            continue;
        };
        let Some(cli) = Cli::from_base_url(&connection.base_url) else {
            continue;
        };
        // Phase 1: nur Claude. Codex bekommt in der Vorschau nie ein Tupel.
        if cli != Cli::Claude {
            continue;
        }
        let Some(identity) = identity_of(cli) else {
            continue;
        };
        let binding = preview::binding_for(cli, &model.remote_id, &identity);
        if !out
            .iter()
            .any(|b| b.provider == binding.provider && b.model_id == binding.model_id)
        {
            out.push(binding);
        }
    }
    out
}

/// Fehlercode, wenn die Vorschau ohne Risikohinweis eingeschaltet wird.
pub const ERR_RISK_NOT_ACKNOWLEDGED: &str = "risk_not_acknowledged";
/// Fehlercode, wenn keine CLI zu binden ist.
pub const ERR_NO_CLI: &str = "no_cli";
/// Fehlercode, wenn die Claude-CLI keine native Binaerdatei ist (npm-, Node-
/// oder Skript-Launcher).
pub const ERR_CLAUDE_NOT_NATIVE: &str = "claude_not_native";
/// Fehlercode, wenn dieses System das geprüfte Programm nicht ohne
/// Austauschfenster starten kann (macOS, andere Unix, Linux ohne `/proc`).
pub const ERR_PLATFORM_UNSUPPORTED: &str = "platform_unsupported";

/// Schalter „Abo-Vorschau“ (`book_talk_subscription_preview`, Standard aus).
/// Einschalten (und erneutes Bestätigen) geht nur nach dem Risikohinweis
/// („Budget empirisch, nicht bewiesen; Anfragen gehen an Anthropic/OpenAI über
/// dein Abo“): `acknowledged` kommt vom Dialog. Dabei werden Anbieter, Modell,
/// CLI-Version und SHA-256 des Programms je Abo-Modell gebunden; weicht eines
/// später ab, wirkt der Schalter nicht, bis erneut bestätigt wird. Ausschalten
/// löscht die Bindung.
#[tauri::command]
#[specta::specta]
pub async fn change_book_talk_subscription_preview_setting(
    app: AppHandle,
    enabled: bool,
    acknowledged: bool,
) -> Result<(), String> {
    let bindings = if enabled {
        if !acknowledged {
            return Err(ERR_RISK_NOT_ACKNOWLEDGED.to_string());
        }
        if !preview::platform_supported() {
            return Err(ERR_PLATFORM_UNSUPPORTED.to_string());
        }
        let current = crate::settings::get_settings(&app);
        // Bindung = die installierte native Claude-Datei (keine Kopie), gehalten
        // und gehasht ueber `preview::Pinned`.
        let (bound, not_native) = tokio::task::spawn_blocking(move || {
            let not_native = std::sync::atomic::AtomicBool::new(false);
            let bound = bindings_for(&current, |cli| {
                let binary = match crate::managers::llm::cli::locate_native_claude() {
                    Ok(binary) => binary,
                    Err(e) => {
                        if e.starts_with("claude_not_native") {
                            not_native.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        return None;
                    }
                };
                match preview::identity(&binary) {
                    Ok(identity) => Some(identity),
                    Err(e) => {
                        if e.starts_with("claude_not_native") {
                            not_native.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        None
                    }
                }
            });
            (bound, not_native.load(std::sync::atomic::Ordering::SeqCst))
        })
        .await
        .map_err(|e| e.to_string())?;
        if bound.is_empty() {
            return Err(if not_native {
                ERR_CLAUDE_NOT_NATIVE
            } else {
                ERR_NO_CLI
            }
            .to_string());
        }
        bound
    } else {
        Vec::new()
    };
    {
        let _guard = crate::managers::tts::providers::host::SETTINGS_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut settings = crate::settings::get_settings(&app);
        settings.book_talk_subscription_preview = enabled;
        settings.book_talk_preview_bindings = bindings;
        crate::settings::write_settings(&app, settings);
    }
    crate::sync::booktalk_relay::wake();
    Ok(())
}

/// Zustand der Abo-Vorschau für die Oberfläche.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct BookTalkPreviewStatus {
    /// Schalter `book_talk_subscription_preview`.
    pub enabled: bool,
    /// Das Gesprächsmodell ist ein Abo-Modell (CLI).
    pub applies: bool,
    /// `reconfirm`: der Schalter ist an, wirkt aber nicht, bis erneut bestätigt
    /// wird; `not_released`: Isolation oder Messung auf diesem Gerät fehlen.
    pub state: PreviewState,
    /// Warum die Vorschau (noch) nicht wirkt (`measurement_missing`,
    /// `isolation_unverified`, `binding_changed` …); sonst `None`.
    pub blocked: Option<Blocked>,
}

/// Status der Vorschau für ein Codex-Modell: immer gesperrt (Phase 2).
pub fn codex_status(enabled: bool) -> BookTalkPreviewStatus {
    BookTalkPreviewStatus {
        enabled,
        applies: true,
        state: PreviewState::NotReleased,
        blocked: Some(Blocked::CodexPreviewNotAvailable),
    }
}

/// Zustand der Abo-Vorschau für das aktuelle Gesprächsmodell.
#[tauri::command]
#[specta::specta]
pub async fn book_talk_preview_status(app: AppHandle) -> Result<BookTalkPreviewStatus, String> {
    let settings = crate::settings::get_settings(&app).for_function(crate::settings::FN_BOOK_TALK);
    let enabled = settings.book_talk_subscription_preview;
    let resolved = resolve_provider_coded(&settings).ok();
    let cli = resolved
        .as_ref()
        .and_then(|(provider, _, _)| Cli::from_base_url(&provider.base_url));
    let (Some(cli), Some((_, model, _))) = (cli, resolved) else {
        return Ok(BookTalkPreviewStatus {
            enabled,
            applies: false,
            state: if enabled {
                PreviewState::Active
            } else {
                PreviewState::Off
            },
            blocked: None,
        });
    };
    // Codex gibt es in der Vorschau nicht: gesperrt, auch bei ausgeschaltetem Schalter, ohne die CLI zu suchen.
    if cli != Cli::Claude {
        return Ok(codex_status(enabled));
    }
    if !enabled {
        return Ok(BookTalkPreviewStatus {
            enabled,
            applies: true,
            state: PreviewState::Off,
            blocked: None,
        });
    }
    let evidence = load_evidence(&app);
    let current = tokio::task::spawn_blocking(move || preview::current_binding(cli, &model))
        .await
        .map_err(|e| e.to_string())?;
    let (current, error_blocked) = match current {
        Ok(binding) => (Some(binding), None),
        Err(e) => (None, Some(preview::blocked_of_error(&e))),
    };
    let verdict = preview_verdict(
        &settings,
        cli,
        current,
        DEFAULT_MAX_ANSWER_TOKENS,
        &evidence,
    );
    Ok(BookTalkPreviewStatus {
        enabled,
        applies: true,
        state: verdict.state,
        blocked: error_blocked.or(verdict.blocked),
    })
}

/// Schlüssel einer Cloud-Freigabe: Verbindung und Modell (Plan §3.8, §11.4:
/// „je Modell“). Gleiche Form wie `consentKeyOf` in der Oberfläche.
pub fn consent_key(connection_id: &str, model_remote_id: &str) -> String {
    format!("{connection_id}::{model_remote_id}")
}

/// Gilt für das aufgelöste Modell (Funktion `book_talk`) eine Freigabe?
fn has_consent(settings: &crate::settings::AppSettings) -> bool {
    settings
        .active_llm_model()
        .is_some_and(|(connection, model)| {
            settings
                .book_talk_cloud_consent
                .contains(&consent_key(&connection.id, &model.remote_id))
        })
}

/// Freigabeliste bereinigen: ohne Doppelte, nur Schlüssel bekannter
/// Verbindungs-Modell-Paare, Reihenfolge der Eingabe.
fn sanitize_consent(
    requested: Vec<String>,
    settings: &crate::settings::AppSettings,
) -> Vec<String> {
    let known: std::collections::HashSet<String> = settings
        .llm_models
        .iter()
        .filter(|m| {
            settings
                .llm_connections
                .iter()
                .any(|c| c.id == m.connection_id)
        })
        .map(|m| consent_key(&m.connection_id, &m.remote_id))
        .collect();
    let mut seen = std::collections::HashSet::new();
    requested
        .into_iter()
        .filter(|k| known.contains(k) && seen.insert(k.clone()))
        .collect()
}

/// Einstellung `book_talk_cloud_consent` (Schlüssel `Verbindung::Modell` mit
/// Freigabe). Doppelte und unbekannte Schlüssel fallen weg; Lesen und
/// Schreiben laufen unter der gemeinsamen Einstellungssperre, damit kein
/// veralteter Stand einen frischen Widerruf überschreibt.
#[tauri::command]
#[specta::specta]
pub fn change_book_talk_cloud_consent_setting(
    app: AppHandle,
    providers: Vec<String>,
) -> Result<(), String> {
    let _guard = crate::managers::tts::providers::host::SETTINGS_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut settings = crate::settings::get_settings(&app);
    settings.book_talk_cloud_consent = sanitize_consent(providers, &settings);
    crate::settings::write_settings(&app, settings);
    Ok(())
}

/// Sprachbefehl erkennen (ohne Modell). „Ja“ ist nur dann ein Befehl
/// (`read_citation`), wenn die letzte Antwort die Stelle zum Vorlesen anbot.
#[tauri::command]
#[specta::specta]
pub fn book_talk_command(
    app: AppHandle,
    book_id: String,
    text: String,
) -> Result<Option<BookTalkCommand>, String> {
    let dir = page_path(&app, &book_id).map_err(|_| ERR_BOOK_NOT_FOUND.to_string())?;
    let offered_read = store::load(&dir)
        .unwrap_or_default()
        .last()
        .is_some_and(|t| {
            t.answer.intent == BookTalkIntent::Locate && !t.answer.citations.is_empty()
        });
    Ok(commands::detect(&text)
        .and_then(|c| commands::resolve_confirm(c, offered_read))
        .map(|kind| BookTalkCommand { kind }))
}

#[cfg(test)]
mod evaltest;
#[cfg(test)]
mod tests;
