//! Gemeinsame Bausteine der Tests des Workflow-Moduls (nur `cfg(test)`).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde_json::{json, Value};

use super::action::{
    Action, EffectKind, HeavyNeed, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::catalog;
use super::engine::{Clock, Engine, EngineConfig};
use super::heavy::{HeavyGate, LocalHeavyGate};
use crate::managers::meetings::store::MeetingStore;

pub const T0: i64 = 1_800_000_000_000;

/// Eine frische Datenbank in einem eigenen Ordner (bleibt bis Prozessende stehen).
pub struct Fx {
    pub store: MeetingStore,
    pub db_path: PathBuf,
}

impl Fx {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("meetings.db");
        let store = MeetingStore::open_at(&db_path).unwrap();
        std::mem::forget(dir);
        Self { store, db_path }
    }

    pub fn conn(&self) -> Connection {
        self.store.get_connection().unwrap()
    }
}

/// Feste, von Hand gestellte Uhr.
pub struct FakeClock(AtomicI64);

impl FakeClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(T0)))
    }

    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn set(&self, ms: i64) {
        self.0.store(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Ein Tor mit einem Platz und RAM-Tor, das immer genug RAM meldet.
pub fn roomy_gate() -> Arc<LocalHeavyGate> {
    Arc::new(LocalHeavyGate::with_probe(Box::new(|_| Ok(()))))
}

pub fn engine_with(fx: &Fx, clock: &Arc<FakeClock>, gate: Arc<dyn HeavyGate>) -> Engine {
    Engine::new(
        fx.db_path.clone(),
        gate,
        clock.clone(),
        EngineConfig::default(),
    )
}

pub fn engine(fx: &Fx, clock: &Arc<FakeClock>) -> Engine {
    engine_with(fx, clock, roomy_gate())
}

// ---------------------------------------------------------------------------
// Definitionen
// ---------------------------------------------------------------------------

pub fn manual_trigger() -> Value {
    json!({"type": "manual"})
}

pub fn def(steps: Vec<Value>) -> Value {
    json!({
        "schema": "lva-workflow@1",
        "name": "Testablauf",
        "trigger": manual_trigger(),
        "steps": steps
    })
}

pub fn step(id: &str, action: &str, params: Value) -> Value {
    json!({"id": id, "action": action, "params": params})
}

/// Ein einfacher Schritt ohne Rechte: `notify.local`.
pub fn note(id: &str) -> Value {
    step(
        id,
        "notify.local",
        json!({"title": format!("Schritt {id}")}),
    )
}

/// Legt einen scharf geschalteten, eingeschalteten Ablauf an und gibt seine Kennung.
pub fn armed_workflow(engine: &Engine, definition: &Value) -> String {
    let row = engine.save_workflow(None, definition).unwrap();
    engine.set_enabled(&row.id, true).unwrap();
    engine.set_armed(&row.id, true).unwrap();
    row.id
}

// ---------------------------------------------------------------------------
// Bausteine mit Drehbuch
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Call {
    pub key: String,
    pub attempt: u32,
    pub params: Value,
}

pub type Hook = Box<dyn Fn(&RunCtx<'_>) + Send + Sync>;
type SharedHook = Arc<dyn Fn(&RunCtx<'_>) + Send + Sync>;

/// Ersetzt einen Katalogbaustein: nimmt Ergebnisse aus einem Drehbuch, merkt sich jeden
/// Aufruf. Recht, Wirkungstext und Parameter kommen aus dem Katalog.
pub struct Scripted {
    id: &'static str,
    effect: EffectKind,
    heavy: Option<HeavyNeed>,
    script: Mutex<VecDeque<Result<StepOutput, StepError>>>,
    calls: Mutex<Vec<Call>>,
    hook: Mutex<Option<SharedHook>>,
    confirm: Mutex<Option<StepOutput>>,
    panic_next: AtomicUsize,
}

impl Scripted {
    pub fn new(id: &'static str, effect: EffectKind) -> Arc<Self> {
        Arc::new(Self {
            id,
            effect,
            heavy: None,
            script: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            hook: Mutex::new(None),
            confirm: Mutex::new(None),
            panic_next: AtomicUsize::new(0),
        })
    }

    pub fn heavy(id: &'static str, effect: EffectKind, need: HeavyNeed) -> Arc<Self> {
        Arc::new(Self {
            id,
            effect,
            heavy: Some(need),
            script: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            hook: Mutex::new(None),
            confirm: Mutex::new(None),
            panic_next: AtomicUsize::new(0),
        })
    }

    pub fn push(&self, r: Result<StepOutput, StepError>) {
        self.script.lock().unwrap().push_back(r);
    }

    pub fn push_ok(&self, data: Value) {
        self.push(Ok(StepOutput::with_data(data)));
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn set_hook(&self, hook: Hook) {
        *self.hook.lock().unwrap() = Some(Arc::from(hook));
    }

    pub fn set_confirm(&self, out: Option<StepOutput>) {
        *self.confirm.lock().unwrap() = out;
    }

    pub fn panic_once(&self) {
        self.panic_next.store(1, Ordering::SeqCst);
    }
}

impl Action for Scripted {
    fn id(&self) -> &str {
        self.id
    }

    fn effect(&self) -> EffectKind {
        self.effect
    }

    fn heavy(&self, _params: &Value) -> Option<HeavyNeed> {
        self.heavy
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(catalog::action_spec(self.id).unwrap(), params)
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(catalog::action_spec(self.id).unwrap(), params)
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        self.calls.lock().unwrap().push(Call {
            key: ctx.idempotency_key.clone(),
            attempt: ctx.attempt,
            params: params.clone(),
        });
        // Das Schloss nicht waehrend des Hooks halten: ein Hook darf selbst weitere
        // Laeufe anstossen (zweite Engine), die denselben Baustein aufrufen.
        let hook = self.hook.lock().unwrap().clone();
        if let Some(h) = hook {
            h(ctx);
        }
        if self.panic_next.swap(0, Ordering::SeqCst) == 1 {
            panic!("absichtlicher Test-Absturz");
        }
        let next = self.script.lock().unwrap().pop_front();
        match next {
            Some(r) => r,
            None => Ok(StepOutput::with_data(json!({"n": self.call_count()}))),
        }
    }

    fn confirm(&self, _ctx: &RunCtx<'_>, _params: &Value) -> Option<StepOutput> {
        self.confirm.lock().unwrap().clone()
    }
}

/// Legt eine Integration der Art `kind` an (Kennung wie von der Oberflaeche vergeben).
pub fn register(conn: &Connection, kind: crate::managers::integrations::model::Kind, id: &str) {
    use crate::managers::integrations::model::NewIntegration;
    if kind.is_calendar_managed() {
        // Kalenderquellen entstehen im Kalender; das Register spiegelt sie.
        crate::managers::integrations::test_support::add_calendar_source(
            conn,
            id,
            kind.as_str(),
            "Testkalender",
            T0 / 1000,
        );
        return;
    }
    let mut n = NewIntegration::new(kind, "Testintegration");
    n.id = Some(id.to_string());
    crate::managers::integrations::store::create(conn, &n, T0).unwrap();
}

pub fn set_grant(
    conn: &Connection,
    integration: &str,
    cap: crate::managers::integrations::model::Capability,
    mode: crate::managers::integrations::model::GrantMode,
) {
    crate::managers::integrations::store::set_grant(
        conn,
        integration,
        cap,
        crate::managers::integrations::model::Caller::Workflow,
        mode,
    )
    .unwrap();
}
