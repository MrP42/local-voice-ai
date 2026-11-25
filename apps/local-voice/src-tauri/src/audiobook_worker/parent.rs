//! Parent-Kleber: Freigaben, Buchsteuerung und Cursor bleiben in der App.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tauri::Manager;

use super::{process::ProcessWorker, protocol::{LocalModel, ModelKind, Reply, Request}, InferenceSession, InferenceWorker, InferResult};
#[cfg(test)]
use super::protocol::WorkerError;
use crate::managers::meetings::{audiobook, job::JobHandle, store::MeetingStore};
use crate::managers::model::{EngineType, ModelManager};
use crate::managers::transcription::{ExtraEngineBinding, LanguageOverride, TranscriptionManager};

const MAX_RESTARTS: usize = 2;
type Profile = (audiobook::AudiobookTempo, usize);
type Signal = Arc<Mutex<Option<Instant>>>;
type Registry = Mutex<HashMap<String, Weak<Mutex<Option<Instant>>>>>;
static WORKERS: OnceLock<Registry> = OnceLock::new();
static FOREGROUND: Mutex<Option<Instant>> = Mutex::new(None);

fn profile(app: &tauri::AppHandle) -> Profile {
    let (parallel, total) = audiobook::app_budget(app);
    (audiobook::app_tempo(app), (total / parallel).max(1))
}

fn refresh_profile<W: InferenceWorker>(session: &mut Option<InferenceSession<W>>, old: &mut Profile, new: Profile) {
    if *old != new {
        session.take();
        *old = new;
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<Context>> = const { RefCell::new(None) };
}

fn set_deadline(signal: &Signal, deadline: Instant) {
    let mut at = signal.lock().unwrap_or_else(|e| e.into_inner());
    *at = Some(at.map_or(deadline, |old| old.min(deadline)));
}

pub(crate) fn register(id: &str) -> Signal {
    let signal = Arc::new(Mutex::new(None));
    WORKERS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), Arc::downgrade(&signal));
    signal
}

pub(crate) fn poll_worker_block<W: InferenceWorker>(session: &mut InferenceSession<W>,
    store: &MeetingStore, id: &str, generation: u64,
    deadline: impl FnMut() -> Result<Option<Instant>, String>) -> Result<InferResult, String> {
    let result = poll_block(session, deadline);
    if matches!(&result, Err(code) if code == audiobook::YIELDED) {
        store.audiobook_invalidate_worker(id, generation)
            .map_err(|_| "audiobook_control_unreadable")?;
    }
    result
}

/// Auch ein haengendes natives Modell blockiert hier nur sein Kind, nie das Polling.
fn poll_block<W: InferenceWorker>(session: &mut InferenceSession<W>,
    mut deadline: impl FnMut() -> Result<Option<Instant>, String>) -> Result<InferResult, String> {
    loop {
        if let Some(at) = deadline()? {
            session.request_stop(at).map_err(|_| "audiobook_worker_stop_failed")?;
        }
        match session.tick(Instant::now()) {
            Ok(Some(result)) => return Ok(result),
            Ok(None) if session.is_stopped() => return Err(audiobook::YIELDED.into()),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return Err("audiobook_worker_crashed".into()),
        }
    }
}

fn retry_block<W: InferenceWorker>(session: &mut Option<InferenceSession<W>>,
    mut load: impl FnMut() -> Result<InferenceSession<W>, String>,
    mut infer: impl FnMut(&mut InferenceSession<W>) -> Result<InferResult, String>,
    mut authorize: impl FnMut() -> Result<(), String>) -> Result<InferResult, String> {
    for restart in 0..=MAX_RESTARTS {
        let result = (|| {
            if session.is_none() { *session = Some(load()?); }
            if restart > 0 { authorize()?; }
            infer(session.as_mut().unwrap())
        })();
        match result {
            Ok(result) => return Ok(result),
            Err(code) if code == audiobook::YIELDED || !code.starts_with("audiobook_worker_") => return Err(code),
            Err(_) => { session.take(); }
        }
    }
    Err("audiobook_worker_crashed".into())
}

enum Backend<W, F> { Worker(W), Fallback(F) }

fn start_or_fallback<W, F>(load: impl FnOnce() -> Result<W, String>,
    fallback: impl FnOnce(String) -> Result<F, String>) -> Result<Backend<W, F>, String> {
    match load() {
        Ok(worker) => Ok(Backend::Worker(worker)),
        Err(code) if code == audiobook::YIELDED || code == "audiobook_memory" => Err(code),
        Err(code) => fallback(code).map(Backend::Fallback),
    }
}

/// Eine Nutzeraktion verteilt EINE absolute Frist an alle betroffenen Teile.
pub fn stop_books(ids: &[String], deadline: Instant) {
    let registry = WORKERS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        if let Some(signal) = registry.get(id).and_then(Weak::upgrade) {
            set_deadline(&signal, deadline);
        }
    }
}

struct Context {
    app: tauri::AppHandle,
    store: Arc<MeetingStore>,
    job: Arc<JobHandle>,
    id: String,
    model_id: String,
    signal: Signal,
    generation: u64,
    profile: Profile,
    session: Option<InferenceSession<ProcessWorker>>,
}

impl Context {
    fn deadline(&self) -> Result<Option<Instant>, String> {
        if let Some(at) = *self.signal.lock().unwrap_or_else(|e| e.into_inner()) { return Ok(Some(at)); }
        let control = self.store.nonblocking().audiobook_control(&self.id)
            .map_err(|_| "audiobook_control_unreadable")?.ok_or("not_audiobook")?;
        if control.desired_state != "running" || self.job.is_stopped() || self.job.is_paused() {
            set_deadline(&self.signal, Instant::now() + Duration::from_secs(2));
        }
        let held = audiobook::foreground_hold(&self.app);
        let mut foreground = FOREGROUND.lock().unwrap_or_else(|e| e.into_inner());
        if held {
            let deadline = *foreground.get_or_insert_with(|| Instant::now() + Duration::from_secs(2));
            set_deadline(&self.signal, deadline);
        } else {
            *foreground = None;
        }
        Ok(*self.signal.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn invalidate(&self) -> Result<(), String> {
        self.store.audiobook_invalidate_worker(&self.id, self.generation)
            .map_err(|_| "audiobook_control_unreadable".into())
    }

    fn load(&self, tm: &TranscriptionManager) -> Result<InferenceSession<ProcessWorker>, String> {
        let models = self.app.state::<Arc<ModelManager>>();
        let info = models.get_model_info(&self.model_id).filter(|m| m.is_downloaded)
            .ok_or("audiobook_worker_load_failed")?;
        let path = models.get_model_path(&self.model_id).map_err(|_| "audiobook_worker_load_failed")?;
        let settings = crate::settings::get_settings(&self.app);
        let (size, gpu) = tm.meeting_model_footprint(&settings);
        let pending = crate::managers::engine_ledger::global().reserve(
            size, gpu,
            || crate::managers::meetings::final_pass::probe_hardware(&self.app),
            |hw| {
                if crate::managers::accurate_run::active() { return Err("audiobook_memory"); }
                crate::managers::meetings::queue::plan_extra_slot(size, gpu, hw).map_err(|_| "audiobook_memory")
            },
        ).map_err(str::to_string)?;
        let kind = match info.engine_type {
            EngineType::TranscribeCpp => ModelKind::TranscribeCpp,
            EngineType::Parakeet => ModelKind::Parakeet,
            EngineType::Moonshine => ModelKind::Moonshine,
            EngineType::MoonshineStreaming => ModelKind::MoonshineStreaming,
            EngineType::SenseVoice => ModelKind::SenseVoice,
            EngineType::GigaAM => ModelKind::GigaAm,
            EngineType::Canary => ModelKind::Canary,
            EngineType::Cohere => ModelKind::Cohere,
        };
        let mut worker = ProcessWorker::begin_load(Request::Load {
            model: LocalModel { path, kind, supported_languages: info.supported_languages,
                supports_language_detection: info.supports_language_detection },
            n_threads: self.profile.1 as i32,
            accelerator: settings.transcribe_accelerator,
            ort_accelerator: settings.ort_accelerator,
            device: settings.transcribe_gpu_device,
            language: LanguageOverride::current().unwrap_or_else(|| "auto".into()),
        }, self.profile.0).map_err(|_| "audiobook_worker_start_failed")?;
        let timeout = Instant::now() + Duration::from_secs(300);
        loop {
            if self.deadline()?.is_some_and(|at| Instant::now() >= at) {
                let _ = worker.terminate();
                self.invalidate()?;
                return Err(audiobook::YIELDED.into());
            }
            match worker.poll() {
                Ok(Some(Reply::Loaded)) => break,
                Ok(None) if Instant::now() < timeout => std::thread::sleep(Duration::from_millis(20)),
                _ => return Err("audiobook_worker_load_failed".into()),
            }
        }
        // Geladene Worker stecken im gemessenen Systemverbrauch, nicht zweimal abziehen.
        drop(pending);
        Ok(InferenceSession::new(worker))
    }

    fn block(&mut self, tm: &TranscriptionManager, samples: Vec<f32>) -> Result<InferResult, String> {
        if self.deadline()?.is_some() {
            self.session.take();
            self.invalidate()?;
            return Err(audiobook::YIELDED.into());
        }
        refresh_profile(&mut self.session, &mut self.profile, profile(&self.app));
        let mut session = self.session.take();
        let result = retry_block(&mut session,
            || {
                if self.deadline()?.is_some() { return Err(audiobook::YIELDED.into()); }
                self.load(tm)
            },
            |session| {
                let settings = crate::settings::get_settings(&self.app);
                let models = self.app.state::<Arc<ModelManager>>();
                let info = models.get_model_info(&self.model_id).ok_or("audiobook_worker_load_failed")?;
                let language = crate::managers::model::effective_language(
                    &LanguageOverride::current().unwrap_or_else(|| "auto".into()),
                    &info.supported_languages, info.supports_language_detection,
                );
                session.worker_mut().configure(language, settings.custom_words, self.model_id.clone());
                session.start(self.generation, samples.clone()).map_err(|_| "audiobook_worker_crashed")?;
                poll_worker_block(session, &self.store, &self.id, self.generation, || self.deadline())
            },
            || crate::managers::planning::modell_versuch(&self.store, &self.id, |frei| frei.einloesen())
                .map_err(|e| match e {
                    crate::managers::planning::LaufFehler::Tor(tor) => tor.code(),
                    crate::managers::planning::LaufFehler::Aufruf(code) => code,
                }),
        );
        if result.is_err() && self.deadline()?.is_some() {
            drop(session);
            self.invalidate()?;
            return Err(audiobook::YIELDED.into());
        }
        self.session = session;
        result
    }
}

#[cfg(test)]
mod tests;

/// Lebensdauer exakt ein Import; keine Engine der Diktat-/Besprechungsspur.
pub struct Binding {
    id: String,
    _fallback: Option<ExtraEngineBinding>,
}

impl Binding {
    pub fn enter(app: &tauri::AppHandle, store: &Arc<MeetingStore>, tm: &TranscriptionManager,
        id: &str, job: &Arc<JobHandle>) -> Result<Self, String> {
        if active() { return Err("audiobook_worker_already_bound".into()); }
        let run = store.audiobook_run_for(id).map_err(|_| "audiobook_context_unreadable")?
            .ok_or("audiobook_run_missing")?;
        for key in ["worker_failure_code", "worker_fallback_code"] {
            store.set_metadata_key(id, key, serde_json::Value::Null)
                .map_err(|_| "audiobook_worker_notice_failed")?;
        }
        let signal = register(id);
        let mut binding = Self { id: id.into(), _fallback: None };
        let mut context = Context { app: app.clone(), store: store.clone(), job: job.clone(),
            id: id.into(), model_id: run.effective_model_id.clone(), signal,
            generation: store.audiobook_cursor(id).map_err(|_| "audiobook_cursor_unreadable")?.0,
            profile: profile(app),
            session: None };
        let backend = start_or_fallback(|| context.load(tm), |code| {
                if context.deadline()?.is_some() {
                    context.invalidate()?;
                    return Err(audiobook::YIELDED.into());
                }
                store.set_metadata_key(id, "worker_fallback_code", serde_json::json!(code))
                    .map_err(|_| "audiobook_worker_notice_failed")?;
                let (parallel, total) = audiobook::app_budget(app);
                let settings = crate::settings::get_settings(app);
                let (size, gpu) = tm.meeting_model_footprint(&settings);
                let _pending = crate::managers::engine_ledger::global().reserve(size, gpu,
                    || crate::managers::meetings::final_pass::probe_hardware(app),
                    |hw| crate::managers::meetings::queue::plan_extra_slot(size, gpu, hw)
                        .map_err(|_| "audiobook_memory"),
                ).map_err(str::to_string)?;
                let engine = tm.load_extra_engine(&run.effective_model_id, (total / parallel).max(1) as i32)
                    .map_err(|_| "audiobook_fallback_load_failed")?;
                crate::managers::transcription::bind_extra_engine(engine)
                    .map_err(|_| "audiobook_fallback_bind_failed".into())
        })?;
        match backend {
            Backend::Worker(session) => {
                context.session = Some(session);
                ACTIVE.with(|active| *active.borrow_mut() = Some(context));
            }
            Backend::Fallback(fallback) => binding._fallback = Some(fallback),
        }
        Ok(binding)
    }
}

impl Drop for Binding {
    fn drop(&mut self) {
        ACTIVE.with(|active| { active.borrow_mut().take(); });
        WORKERS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

pub fn active() -> bool { ACTIVE.with(|active| active.borrow().is_some()) }

pub fn infer(tm: &TranscriptionManager, samples: Vec<f32>) -> Option<Result<(Vec<super::TimedSegment>, Option<String>), String>> {
    ACTIVE.with(|active| active.borrow_mut().as_mut().map(|context| {
        let result = context.block(tm, samples).map(|r| (r.segments, r.detected_language));
        if let Err(code) = &result {
            if code.starts_with("audiobook_worker_") {
                context.store.set_metadata_key(&context.id, "worker_failure_code", serde_json::json!(code))
                    .map_err(|_| "audiobook_worker_notice_failed".to_string())?;
            }
        }
        result
    }))
}
