//! Bausteine (Aktionen) der Engine: das `Action`-Trait, sein Kontext und das
//! Register der bekannten Bausteine (B1).
//!
//! **Vertrag fuer jeden Baustein** (B2 bis B6 und spaeter Goal C `agent.*`):
//!
//! - `effect`: `Pure` (liest nur, beliebig wiederholbar), `Idempotent` (wirkt, aber
//!   eine Wiederholung ergibt dasselbe: festes Ziel, Ersetzen statt Anhaengen) oder
//!   `External` (Mail, Termin-Notiz, Aufnahme, Webhook: eine zweite Ausfuehrung
//!   waere eine zweite Wirkung). Nach einem Absturz mitten im Schritt wiederholt die
//!   Engine nur `Pure` und `Idempotent` selbst; `External` meldet sie als
//!   `effect_uncertain`, es sei denn `confirm` kann belegen, dass die Wirkung schon
//!   eingetreten ist.
//! - Fehlerklassen: `Transient` = es ist NICHTS passiert, ein neuer Versuch ist sicher
//!   (Netz weg, 429/5xx, Datei gesperrt). `Permanent` = wird nie gelingen.
//!   `Unknown` = unklar, ob die Wirkung eintrat (Zeitueberschreitung NACH dem Senden):
//!   bei `External` nie von selbst wiederholt. Wer nicht sicher ist, dass nichts
//!   passiert ist, meldet `Unknown`, nicht `Transient`.
//! - `idempotency_key` (`<lauf>:<schritt>`) gehoert in jede Anfrage nach aussen, die
//!   einen Schluessel kennt (Graph `Prefer`/Client-ID, `Message-ID`), und in feste
//!   Dateinamen: so wird auch ein Wiederholungsfall ohne `confirm` nie doppelt.
//! - Rechte: `needs` nennt Integration und Faehigkeit; die Engine fragt das Tor
//!   (`integrations::gate`) VOR `run`. Ein Baustein prueft keine Rechte selbst und
//!   ruft nie ein Register-Schreibwerk am Tor vorbei.
//! - Schwere Bausteine (`heavy`): STT, LLM, TTS. Die Engine holt vorher einen Platz am
//!   `HeavyGate` (seriell, mit RAM-Tor); ohne Platz wartet der Lauf, er scheitert nicht.
//! - Kindprozesse und Server (Modell, Whisper, Fish Speech) gehen NUR ueber
//!   `process_guard` (Job-Objekt mit RAM-/CPU-Deckel, Start-Tor); `StepError`-Klasse
//!   bei Absturz des Kindes: `Transient`, wenn nichts geschrieben wurde.
//! - `run` ist blockierend und lauft auf dem Arbeiter-Thread der Engine (wie die
//!   Import-Warteschlange). Es darf lange dauern, soll aber `ctx.cancelled()` zwischen
//!   Bloecken beachten und gibt Ausgaben bis [`MAX_OUTPUT_BYTES`] zurueck; Grosses
//!   (Dokumenttext) steht in Dokumenten, im Ergebnis nur der Verweis.
//! - Ausgaben stehen unter `steps.<id>.*` im Laufkontext und im Laufprotokoll: keine
//!   Geheimnisse und keine Inhalte, die der Nutzer nicht im Protokoll sehen soll.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::managers::integrations::model::Capability;
use crate::managers::provenance::{self, ActorKind, NewProvenance, ProvenanceError, SourceRef};

use super::engine::Clock;

/// Groesste Ausgabe eines Schritts (JSON-Text, Bytes). Mehr ist ein Fehler des Bausteins.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Wirkungsart eines Bausteins (siehe Moduldoku).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Pure,
    Idempotent,
    External,
}

impl EffectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EffectKind::Pure => "pure",
            EffectKind::Idempotent => "idempotent",
            EffectKind::External => "external",
        }
    }
}

/// Bedarf eines schweren Schritts. `ram_mb` ist ein Schaetzwert fuer das RAM-Tor
/// (Bedarf plus Systemreserve, siehe `process_guard::check_ram_for_start`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeavyNeed {
    pub ram_mb: u64,
    /// Kurzname fuer Anzeige und Protokoll ("Transkription", "Sprachmodell").
    pub label: &'static str,
}

/// Welche Integration und Faehigkeit ein Schritt braucht (fuers Tor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Needs {
    pub integration_id: String,
    pub capability: Capability,
    /// Wohin/woran (Datei, Empfaengerregel, Termin): erscheint im Audit und in der
    /// Vorschau der Freigabe.
    pub target: Option<String>,
}

/// Warum sich fuer einen Schritt kein Recht bestimmen laesst.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NeedsError {
    /// Die Parameter nennen keine gueltige Integration (dauerhafter Fehler).
    Invalid(String),
    /// Das Register kennt fuer diese Wirkung noch kein Recht: fail closed, abgelehnt.
    Unmodeled(String),
}

/// Wie ein Schritt scheiterte (siehe Moduldoku).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepError {
    Transient(String),
    Permanent(String),
    /// Das Recht fehlt (der Baustein hat es selbst festgestellt).
    Denied(String),
    /// Unklar, ob die Wirkung eintrat.
    Unknown(String),
    /// Nicht jetzt: der Lauf wartet `retry_after_ms` und der Baustein laeuft danach
    /// erneut (kein Versuch verbraucht). Fuer "warten bis ...".
    Defer {
        retry_after_ms: u64,
        reason: String,
    },
    /// Der Baustein ist (noch) nicht eingebaut.
    NotAvailable(String),
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepError::Transient(m)
            | StepError::Permanent(m)
            | StepError::Denied(m)
            | StepError::Unknown(m)
            | StepError::NotAvailable(m) => write!(f, "{m}"),
            StepError::Defer { reason, .. } => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for StepError {}

/// Ergebnis eines Schritts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepOutput {
    /// Wird zu `steps.<id>.*`. Ein Objekt; `status`, `ok` und `error` setzt die Engine.
    pub data: Map<String, Value>,
    /// Ein Satz fuers Laufprotokoll.
    pub summary: Option<String>,
    /// Quellen fuer den Provenienz-Eintrag des Schritts (Verweise, keine Inhalte).
    pub sources: Vec<SourceRef>,
    /// 0..=1, nur wo der Baustein eine Sicherheit angibt.
    pub confidence: Option<f64>,
}

impl StepOutput {
    pub fn with_data(data: Value) -> Self {
        let map = match data {
            Value::Object(m) => m,
            other => {
                let mut m = Map::new();
                m.insert("value".to_string(), other);
                m
            }
        };
        Self {
            data: map,
            ..Self::default()
        }
    }

    pub fn summary(mut self, text: &str) -> Self {
        self.summary = Some(text.to_string());
        self
    }
}

/// Was ein Baustein ueber seinen Lauf weiss.
pub struct RunCtx<'a> {
    pub workflow_id: &'a str,
    pub run_id: &'a str,
    pub step_id: &'a str,
    /// Nummer des Versuchs (ab 1).
    pub attempt: u32,
    /// `<lauf>:<schritt>`: stabil ueber Wiederholungen und Wiederaufnahme.
    pub idempotency_key: String,
    /// Der volle Laufkontext (`trigger`, `vars`, `steps`, `meeting`, `run`, `workflow`),
    /// nur lesend.
    pub context: &'a Value,
    /// Wann der Schritt zum ersten Mal begann (bleibt ueber `Defer` gleich).
    pub step_started_at: i64,
    pub(crate) cancel: &'a AtomicBool,
    pub(crate) clock: &'a dyn Clock,
    pub(crate) db_path: &'a std::path::Path,
}

impl RunCtx<'_> {
    /// Hat der Nutzer den Lauf abgebrochen? Lange Bausteine fragen zwischen Bloecken.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// Verbindung zur Datenbank des Laufs (fuer Provenienz und eigene Abfragen).
    pub fn conn(&self) -> rusqlite::Result<Connection> {
        Connection::open(self.db_path)
    }

    /// Schreibt einen Provenienz-Eintrag mit Akteur `workflow` und der Kennung
    /// `<workflow>/<lauf>/<schritt>`. Ein Fehler wird gemeldet, soll aber die
    /// Erzeugung nie scheitern lassen (Aufrufer loggen und machen weiter).
    pub fn record_provenance(&self, mut entry: NewProvenance) -> Result<String, ProvenanceError> {
        entry.actor_kind = ActorKind::Workflow;
        entry.actor_ref = Some(actor_ref(self.workflow_id, self.run_id, self.step_id));
        let conn = self
            .conn()
            .map_err(|e| ProvenanceError::Store(e.to_string()))?;
        provenance::record_at(&conn, &entry, self.now_ms())
    }
}

/// Die Kennung, die in Audit und Provenienz fuer einen Schritt steht.
pub fn actor_ref(workflow_id: &str, run_id: &str, step_id: &str) -> String {
    format!("{workflow_id}/{run_id}/{step_id}")
}

pub trait Action: Send + Sync {
    /// Kennung des Bausteins (`mail.send`).
    fn id(&self) -> &str;
    fn effect(&self) -> EffectKind;

    /// Schwer? Dann geht der Schritt durch das `HeavyGate`.
    fn heavy(&self, _params: &Value) -> Option<HeavyNeed> {
        None
    }

    /// Integration und Faehigkeit fuer das Tor; `Ok(None)`: kein Recht noetig
    /// (reines Lokales). `Err`: die Parameter ergeben kein Ziel oder fuer den
    /// Baustein ist im Register noch kein Recht vorgesehen (dann wird abgelehnt,
    /// nie ohne Recht ausgefuehrt).
    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError>;

    /// Zusaetzliche Pruefung der (noch nicht eingesetzten) Parameter beim Speichern.
    fn validate(&self, _params: &Map<String, Value>) -> Result<(), String> {
        Ok(())
    }

    /// Geplante Wirkung in einem deutschen Satz, OHNE etwas zu tun (kein I/O).
    fn describe(&self, params: &Value) -> String;

    /// Fuehrt den Schritt aus. Die Parameter sind eingesetzt.
    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError>;

    /// Nur fuer `External`: war die Wirkung schon eingetreten (nach einem Absturz)?
    /// `Some(ausgabe)` = ja, der Schritt gilt als erledigt; `None` = nicht belegbar.
    fn confirm(&self, _ctx: &RunCtx<'_>, _params: &Value) -> Option<StepOutput> {
        None
    }
}

/// Register der Bausteine. `with_catalog` legt fuer jeden Katalogeintrag einen
/// Baustein an, der planen und pruefen kann, aber noch nicht laeuft; die Pakete
/// B2 bis B6 ersetzen sie durch `register` mit derselben Kennung.
#[derive(Clone, Default)]
pub struct ActionRegistry {
    map: std::collections::HashMap<String, Arc<dyn Action>>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Alle Katalogbausteine (noch ohne Ausfuehrung) plus die eingebauten.
    pub fn with_catalog() -> Self {
        let mut r = Self::new();
        for spec in super::catalog::actions() {
            r.register(Arc::new(super::catalog::SpecAction::new(spec)));
        }
        r.register(Arc::new(super::builtin::WaitAction));
        r
    }

    /// Ersetzt einen vorhandenen Baustein gleicher Kennung.
    pub fn register(&mut self, action: Arc<dyn Action>) {
        self.map.insert(action.id().to_string(), action);
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn Action>> {
        self.map.get(id)
    }

    pub fn ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.map.keys().cloned().collect();
        v.sort();
        v
    }
}

/// Pruefung der Ausgabegroesse eines Bausteins (siehe `MAX_OUTPUT_BYTES`).
pub fn output_json(out: &StepOutput) -> Result<String, String> {
    let text = Value::Object(out.data.clone()).to_string();
    if text.len() > MAX_OUTPUT_BYTES {
        return Err(format!(
            "Die Ausgabe des Bausteins ist zu groß ({} KiB, höchstens {} KiB): Inhalte gehören in Dokumente, im Ergebnis steht nur der Verweis.",
            text.len() / 1024,
            MAX_OUTPUT_BYTES / 1024
        ));
    }
    Ok(text)
}

/// Kleiner Helfer fuer Tests und Bausteine: `{"key": value}` als Ausgabe.
pub fn output_of(key: &str, value: Value) -> StepOutput {
    StepOutput::with_data(json!({ key: value }))
}
