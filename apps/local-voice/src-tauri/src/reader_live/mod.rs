//! Satzweise Übersetzung beim Lesen des Originals (lva#782 A1).
//! Nur Arbeitsspeicher; kein Modellstart, kein CLI- oder Cloud-Rückfall.

use std::collections::BTreeMap;
use std::future::Future;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;
use tokio::sync::watch;

use crate::llm_versand::{Anfrage, AppFreigabeQuelle, ChatInhalt, Inhalt, Quellen, Ziel};
use crate::managers::llm::{server::LocalLlmPhase, LocalLlmServer};
use crate::managers::tts::TtsManager;
use crate::settings::{AppSettings, PostProcessProvider};

const CACHE_LIMIT: usize = 2000;
const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const CANCELLED: &str = "reader_live:cancelled";

#[derive(Clone, Deserialize, Type)]
pub struct LiveRequest {
    /// Vom Reader vergebene Sitzungskennung; bei einem neuen Reader neu vergeben.
    pub session_id: String,
    pub page_id: String,
    /// Exakt der angezeigte Originaltext, ohne Normalisierung der Quellkoordinaten.
    pub text: String,
    pub target_lang: String,
    /// UTF-16 im Original. Ein Wechsel nach dem aktuellen Satz übergibt dessen Ende.
    pub source_offset: u32,
    /// Streng steigend je Sitzung, auch bei Vorlauf-Nachbestellungen und Abbruch.
    pub epoch: u32,
    pub rate: f64,
}

#[derive(Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveResult {
    /// Vorrang vor Live; keine Zuordnung zu einzelnen Originalsätzen behaupten.
    Existing { translation: String },
    Window { sentences: u32 },
}

/// Ereignis `reader-live-sentence`: genau ein fertiger Satz, keine Token-Deltas.
#[derive(Clone, Serialize, Deserialize, Type, Event)]
pub struct ReaderLiveSentence {
    pub session_id: String,
    pub page_id: String,
    pub epoch: u32,
    pub target_lang: String,
    pub source_start: u32,
    /// Ausschließlich; beide Offsets beziehen sich auf UTF-16 der Quellfassung.
    pub source_end: u32,
    pub translation: String,
}

struct Unit {
    bytes: Range<usize>,
    start: u32,
    end: u32,
}

fn units(text: &str) -> Result<Vec<Unit>, &'static str> {
    let mut out = Vec::new();
    let mut byte = 0;
    let mut offset = 0;
    for sentence in crate::managers::tts::protocol::split_live_sentences(text) {
        let relative = text[byte..]
            .find(&sentence)
            .ok_or("reader_live:source_invalid")?;
        offset += text[byte..byte + relative].encode_utf16().count() as u32;
        byte += relative;
        let end = offset + sentence.encode_utf16().count() as u32;
        out.push(Unit {
            bytes: byte..byte + sentence.len(),
            start: offset,
            end,
        });
        byte += sentence.len();
        offset = end;
    }
    Ok(out)
}

fn window(units: &[Unit], offset: u32, rate: f64) -> Result<Range<usize>, &'static str> {
    if !rate.is_finite() || !(0.1..=4.0).contains(&rate) {
        return Err("reader_live:invalid_rate");
    }
    let start = units.partition_point(|unit| unit.end <= offset);
    let mut end = start;
    let mut seconds = 0.0;
    while end < units.len() && end - start < 40 {
        seconds += f64::from(units[end].end - units[end].start) / (15.0 * rate);
        end += 1;
        if end - start >= 6 && seconds >= 30.0 {
            break;
        }
    }
    Ok(start..end)
}

struct Session {
    id: String,
    page: String,
    text: String,
    target: String,
    epoch: u32,
    cancelled: bool,
    route: String,
    cache: BTreeMap<u32, String>,
}

impl Session {
    fn bind_route(&mut self, route: String) {
        if self.route != route {
            self.cache.clear();
            self.route = route;
        }
    }

    fn put(&mut self, offset: u32, text: String) {
        self.cache.insert(offset, text);
        while self.cache.len() > CACHE_LIMIT {
            self.cache.pop_first();
        }
    }
}

#[derive(Default)]
struct Slot {
    generation: u64,
    active: Option<Session>,
}

impl Slot {
    fn enter(&mut self, request: &LiveRequest) -> Result<u64, &'static str> {
        if let Some(old) = &self.active {
            if old.id == request.session_id && request.epoch <= old.epoch {
                return Err("reader_live:stale_epoch");
            }
        }
        let same_source = self.active.as_ref().is_some_and(|old| {
            old.id == request.session_id
                && old.page == request.page_id
                && old.text == request.text
                && old.target == request.target_lang
        });
        if !same_source {
            self.active = Some(Session {
                id: request.session_id.clone(),
                page: request.page_id.clone(),
                text: request.text.clone(),
                target: request.target_lang.clone(),
                epoch: request.epoch,
                cancelled: false,
                route: String::new(),
                cache: BTreeMap::new(),
            });
        }
        let active = self.active.as_mut().ok_or(CANCELLED)?;
        active.epoch = request.epoch;
        active.cancelled = false;
        self.generation += 1;
        Ok(self.generation)
    }

    fn current(&self, generation: u64) -> bool {
        self.generation == generation && self.active.as_ref().is_some_and(|s| !s.cancelled)
    }

    fn cancel(&mut self, id: &str, epoch: u32) -> bool {
        let Some(active) = &mut self.active else {
            return false;
        };
        if active.id != id || epoch <= active.epoch {
            return false;
        }
        active.epoch = epoch;
        active.cancelled = true;
        active.cache.clear();
        active.text.clear();
        self.generation += 1;
        true
    }
}

pub struct LiveState {
    inner: Mutex<Slot>,
    changed: watch::Sender<u64>,
    serial: tokio::sync::Mutex<()>,
}

impl Default for LiveState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Slot::default()),
            changed: watch::channel(0).0,
            serial: tokio::sync::Mutex::new(()),
        }
    }
}

impl LiveState {
    fn begin(&self, request: &LiveRequest) -> Result<(u64, watch::Receiver<u64>), &'static str> {
        let mut slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let generation = slot.enter(request)?;
        self.changed.send_replace(generation);
        Ok((generation, self.changed.subscribe()))
    }

    fn cancel(&self, id: &str, epoch: u32) -> bool {
        let mut slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let cancelled = slot.cancel(id, epoch);
        if cancelled {
            self.changed.send_replace(slot.generation);
        }
        cancelled
    }
}

fn usable_reply(text: Option<String>, truncated: bool) -> Result<String, &'static str> {
    let text = text.unwrap_or_default();
    if truncated || text.trim().is_empty() {
        Err("reader_live:unusable_reply")
    } else {
        Ok(text.trim().to_string())
    }
}

// Keine Auflösung über ensure_local/CLI: dieser Weg verwendet nur laufende Server.
fn local_target(
    provider: &PostProcessProvider,
    model: &str,
    running: Option<(&str, u16)>,
) -> Option<Ziel> {
    let configured = Ziel::aus_provider(provider, model);
    if !configured.ist_lokal() || configured.basis_url().is_none() {
        return None;
    }
    if crate::managers::llm::is_local(provider) {
        let (running_model, port) = running?;
        if running_model != model || port == 0 {
            return None;
        }
        Some(Ziel::http(
            &provider.id,
            &format!("http://127.0.0.1:{port}/v1"),
            model,
        ))
    } else {
        Some(configured)
    }
}

fn route(
    settings: &AppSettings,
    running: Option<(&str, u16)>,
) -> Result<(PostProcessProvider, Ziel, String), &'static str> {
    // Aktives lokales Modell zuerst; ein Cloud-Standard führt zur Suche nach einem lokalen.
    let mut models: Vec<_> = settings.llm_models.iter().filter(|m| m.enabled).collect();
    models.sort_by_key(|m| settings.llm_active_model_id.as_deref() != Some(m.id.as_str()));
    for model in models {
        let Some(connection) = settings
            .llm_connections
            .iter()
            .find(|c| c.id == model.connection_id && c.enabled)
        else {
            continue;
        };
        // Vor active_llm_model/sync_legacy filtern: CLI-Nutzbarkeit startet hier keinen Probeprozess.
        if !Ziel::http(&connection.kind, &connection.base_url, &model.remote_id).ist_lokal() {
            continue;
        }
        let Some(mut provider) = settings
            .post_process_providers
            .iter()
            .find(|p| p.id == connection.kind)
            .cloned()
        else {
            continue;
        };
        provider.base_url = connection.base_url.clone();
        let Some(target) = local_target(&provider, &model.remote_id, running) else {
            continue;
        };
        let key = settings
            .post_process_api_keys
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();
        return Ok((provider, target, key));
    }
    Err("reader_live:no_local_model")
}

async fn run_window<F, Fut, E>(
    state: &LiveState,
    generation: u64,
    mut changed: watch::Receiver<u64>,
    request: &LiveRequest,
    units: &[Unit],
    range: Range<usize>,
    mut send: F,
    emit: E,
) -> Result<u32, &'static str>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<String, &'static str>>,
    E: Fn(ReaderLiveSentence) -> Result<(), ()>,
{
    let _serial = tokio::select! {
        biased;
        _ = changed.changed() => return Err(CANCELLED),
        guard = state.serial.lock() => guard,
    };
    let mut count = 0;
    for unit in &units[range] {
        let cached = {
            let slot = state.inner.lock().unwrap_or_else(|e| e.into_inner());
            if !slot.current(generation) {
                return Err(CANCELLED);
            }
            slot.active.as_ref().and_then(|s| s.cache.get(&unit.start).cloned())
        };
        let answer = match cached {
            Some(text) => Ok(text),
            None => tokio::select! {
                biased;
                _ = changed.changed() => return Err(CANCELLED),
                result = send(request.text[unit.bytes.clone()].to_string()) => result,
            },
        };
        // Prüfung, Speicher und Ausgabe unter derselben Sperre wie Epoche/Abbruch.
        let mut slot = state.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !slot.current(generation) {
            return Err(CANCELLED);
        }
        let translation = answer?;
        emit(ReaderLiveSentence {
            session_id: request.session_id.clone(),
            page_id: request.page_id.clone(),
            epoch: request.epoch,
            target_lang: request.target_lang.clone(),
            source_start: unit.start,
            source_end: unit.end,
            translation: translation.clone(),
        })
        .map_err(|_| "reader_live:event_failed")?;
        if let Some(session) = &mut slot.active {
            session.put(unit.start, translation);
        }
        count += 1;
    }
    if !state.inner.lock().unwrap_or_else(|e| e.into_inner()).current(generation) {
        return Err(CANCELLED);
    }
    Ok(count)
}

/// Bestellt nur die aktuelle Stelle plus Vorlauf. Vor invoke auf das Satzereignis hören.
/// Der Future endet nach dem Fenster; eine neuere Epoche bricht ihn sofort ab.
#[tauri::command]
#[specta::specta]
pub async fn reader_live_translate(
    app: AppHandle,
    state: State<'_, Arc<LiveState>>,
    request: LiveRequest,
) -> Result<LiveResult, String> {
    translate(&app, state.inner(), &request)
        .await
        .map_err(str::to_string)
}

async fn translate(
    app: &AppHandle,
    state: &LiveState,
    request: &LiveRequest,
) -> Result<LiveResult, &'static str> {
    if request.session_id.is_empty()
        || request.session_id.len() > 200
        || request.page_id.is_empty()
        || request.page_id.len() > 200
        || request.text.trim().is_empty()
        || request.text.len() > MAX_SOURCE_BYTES
        || request.target_lang.is_empty()
        || request.target_lang.len() > 35
        || !request.target_lang.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-' || b == b'_')
        || request.source_offset as usize > request.text.encode_utf16().count()
    {
        return Err("reader_live:invalid_request");
    }
    let list = units(&request.text)?;
    let range = window(&list, request.source_offset, request.rate)?;
    let (generation, rx) = state.begin(request)?;
    let existing = app.state::<Arc<TtsManager>>()
        .cached_translation(request.text.trim(), &request.target_lang);
    let result = prefer_existing(existing, || {
        translate_local(app, state, request, &list, range, generation, rx)
    })
    .await;
    if !state.inner.lock().unwrap_or_else(|e| e.into_inner()).current(generation) {
        return Err(CANCELLED);
    }
    result
}

async fn prefer_existing<F, Fut>(existing: Option<String>, live: F) -> Result<LiveResult, &'static str>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<LiveResult, &'static str>>,
{
    match existing {
        Some(translation) => Ok(LiveResult::Existing { translation }),
        None => live().await,
    }
}

async fn translate_local(
    app: &AppHandle,
    state: &LiveState,
    request: &LiveRequest,
    list: &[Unit],
    range: Range<usize>,
    generation: u64,
    rx: watch::Receiver<u64>,
) -> Result<LiveResult, &'static str> {
    if range.is_empty() {
        return Ok(LiveResult::Window { sentences: 0 });
    }
    let settings = crate::settings::get_settings(app);
    let server = app.state::<Arc<LocalLlmServer>>();
    let status = server.status();
    let running = if status.phase == LocalLlmPhase::Ready {
        status.model_id.as_deref().zip(status.port)
    } else {
        None
    };
    let (provider, target, api_key) = route(&settings, running)?;
    let source = AppFreigabeQuelle::aus_app(app).map_err(|_| "reader_live:permission_unavailable")?;
    {
        let mut slot = state.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !slot.current(generation) {
            return Err(CANCELLED);
        }
        let route_key = format!("{}|{}|{}", provider.id, target.modell(), target.basis_url().unwrap_or_default());
        if let Some(session) = &mut slot.active {
            session.bind_route(route_key);
        }
    }
    let count = run_window(state, generation, rx, request, list, range, |sentence| {
        let provider = provider.clone();
        let target = target.clone();
        let api_key = api_key.clone();
        let source = &source;
        async move {
            let answer = crate::llm_versand::versenden(source, &target, &Quellen::keine(), Anfrage {
                zweck: crate::managers::usage::Purpose::Translation,
                anbieter: &provider, api_key,
                inhalt: Inhalt::Chat(ChatInhalt {
                    nutzer: crate::translator::translation_prompt(&request.target_lang, &sentence),
                    system: None, schema: None, reasoning_effort: Some("none".into()), reasoning: None,
                }),
            }).await.map_err(|_| "reader_live:translation_failed")?;
            usable_reply(answer.text, answer.abgeschnitten)
        }
    }, |event| event.emit(app).map_err(|_| ())).await?;
    Ok(LiveResult::Window { sentences: count })
}

#[tauri::command]
#[specta::specta]
pub fn reader_live_cancel(
    state: State<'_, Arc<LiveState>>,
    session_id: String,
    epoch: u32,
) -> bool {
    state.cancel(&session_id, epoch)
}

#[cfg(test)]
mod tests;
