//! Die Bausteine „Aufnahme starten“ und „Aufnahme beenden“ und der Einwilligungsweg (B2,
//! AK4).
//!
//! # Einwilligung: ohne Klick keine Aufnahme
//!
//! Eine Aufnahme darf nie von selbst beginnen (§ 201 StGB, Goal R1/E2). Vier Sperren, jede
//! fuer sich ausreichend:
//!
//! 1. **Das Recht.** `recording.start` ist eine Faehigkeit mit `never_allow`: `grants::explain`
//!    macht aus „erlaubt“ immer „fragen“. Der Ablauf bleibt bei jedem Lauf in
//!    `awaiting_approval`, bis der Nutzer die Freigabe entschieden hat; „aus“ verweigert.
//! 2. **Die Freigabe ist einmalig und gebunden** (`integrations::approvals`): sie gilt fuer
//!    genau diesen Schritt dieses Laufs, eine Stunde lang, und wird beim Ausfuehren
//!    eingeloest. Ein zweiter Lauf braucht einen zweiten Klick.
//! 3. **Der Baustein prueft selbst** (`RunCtx::approved`): er laeuft nur, wenn die Engine ihn
//!    ueber eine eingeloeste Freigabe gerufen hat, sonst `Denied`. Das waere die letzte
//!    Sperre, falls je jemand das Register umgeht.
//! 4. **Der Trockenlauf** (neue Ablaeufe) fuehrt nie etwas aus, also auch keine Aufnahme.
//!
//! Der Klick selbst ist das Hinweisfenster (`meeting_prompt`, Art `workflow_recording`): es
//! zeigt Ablauf und Termin, verlangt dasselbe Haekchen wie jede Aufnahme („Alle wissen
//! Bescheid“) und entscheidet erst dann die Freigabe (`consent::decide`). Wer sie lieber
//! auf der Seite Integrationen entscheidet (Freigabedialog, A4), kann das: beide Wege
//! entscheiden dieselbe Freigabe, die Engine beobachtet nur ihren Zustand.
//!
//! # Traeger des Rechts `recording.start` (Entscheidung B2)
//!
//! Jedes Recht haengt an einer Integration des Registers. `recording.start` bieten nur
//! Integrationen der Art `agent` an (`Kind::capabilities`); Kalender (ICS nur lesend, Graph
//! ohne die Faehigkeit) scheiden aus, und eine neue Art wuerde Schema, Migration und Seite
//! Integrationen aendern. **Entscheidung:** der Traeger ist eine feste, von der App angelegte
//! Integration der Art `agent` mit der Kennung [`CARRIER_ID`] (`app-automation`, „Automationen
//! (diese App)“, Richtung „schreiben“). Sie entsteht beim Start (`ensure_carrier`, idempotent)
//! und erscheint wie jede Integration auf der Seite Integrationen; dort steht je Aufrufer
//! (Ablauf, lokaler Agent, externer Agent) das Recht, Vorgabe fuer Abläufe „fragen“. Die
//! Vorlagen setzen `"via": "app-automation"`. Fehlt die Integration oder ist sie
//! ausgeschaltet, ist die Aufnahme abgelehnt (fail closed): nichts haengt davon ab, dass sie
//! existiert, ausser dass automatische Aufnahmen moeglich sind. Eine andere Integration der
//! Art `agent` als `via` zu nennen ist erlaubt (eigene Rechte), aendert aber nichts an der
//! Einwilligung.
//!
//! # Fehlerfaelle
//!
//! - Es laeuft schon eine Aufnahme / Mikrofon oder Systemton fehlen / Diktat aktiv: der
//!   Start scheitert `Permanent` (nichts wurde aufgenommen). Der Lauf endet `failed`; er fragt
//!   NICHT erneut nach einer Einwilligung (kein Dauerklingeln), der Nutzer startet ihn
//!   bewusst neu (`retry_run`).
//! - Abbruch nach dem Start, vor dem Journal: `confirm` belegt ueber den Recorder, dass die
//!   Aufnahme dieses Schritts laeuft (Beginn nach dem Schrittbeginn); sonst
//!   `effect_uncertain`, nie ein zweiter Start.
//! - Freigabe zu spaet (Termin laengst zu Ende): der Baustein startet nicht
//!   (`trigger.end` liegt in der Vergangenheit).
//! - Vergessene Aufnahme (voller Datentraeger): jede automatisch gestartete Aufnahme hat ein
//!   Ende: `event_end`, `duration`/`max_minutes` oder das Sicherheitsnetz
//!   [`DEFAULT_MAX_MINUTES`]. Das Ende prueft der gemeinsame Takt (`StopSchedule`,
//!   `run_due_stops`); ein Neustart mitten in der Aufnahme geht den vorhandenen Weg der
//!   Wiederherstellung (`recover_orphans`), das Ende geht dann nicht verloren, weil die
//!   Aufnahme ohnehin abgeschlossen wird.
//! - Speicher/CPU: der Start ist derselbe Aufruf wie ein Start von Hand
//!   (`MeetingRecorderManager::start_into`); er startet keinen eigenen Prozess und haelt
//!   kein Modell. Bei vollem Arbeitsspeicher aendert sich nichts am Aufnehmen selbst
//!   (Mitschnitt, WAV-Datei), das Live-Transkript und der Enddurchlauf haben ihre eigenen
//!   Tore (`final_pass`, RAM-Tor). Kein Code dieses Moduls laeuft im Audio-Callback.

use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::managers::integrations::model::{Direction, IntegrationError, Kind, NewIntegration};
use crate::managers::integrations::store as integrations_store;

use super::action::{Action, EffectKind, Needs, NeedsError, RunCtx, StepError, StepOutput};
use super::catalog::{self, ActionSpec};
use super::engine::Engine;
use super::trigger::iso;

/// Kennung der Integration, an der das Recht `recording.start` fuer Ablaeufe haengt.
pub const CARRIER_ID: &str = "app-automation";
pub const CARRIER_LABEL: &str = "Automationen (diese App)";
/// Sicherheitsnetz: laenger als so viele Minuten laeuft eine automatisch gestartete
/// Aufnahme nie, wenn der Ablauf nichts anderes vorgibt (`max_minutes` bis 720).
pub const DEFAULT_MAX_MINUTES: i64 = 480;
/// Hoechstzahl vorgemerkter Enden.
const MAX_STOPS: usize = 8;

/// Legt den Traeger an, falls es ihn noch nicht gibt. `true`: neu angelegt.
pub fn ensure_carrier(conn: &Connection, now_ms: i64) -> Result<bool, IntegrationError> {
    if integrations_store::get(conn, CARRIER_ID)?.is_some() {
        return Ok(false);
    }
    let mut n = NewIntegration::new(Kind::Agent, CARRIER_LABEL);
    n.id = Some(CARRIER_ID.to_string());
    // Alle Faehigkeiten der Art `agent` sind schreibend: „schreiben“ genuegt.
    n.direction = Some(Direction::Write);
    n.config = json!({"system": true});
    match integrations_store::create(conn, &n, now_ms) {
        Ok(_) => Ok(true),
        // Ein zweiter Prozess war schneller.
        Err(IntegrationError::Invalid(m)) if m.contains("vergeben") => Ok(false),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Schnittstelle zum Recorder
// ---------------------------------------------------------------------------

/// Was ueber die laufende Aufnahme bekannt ist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentRecording {
    /// `None`, wenn der Recorder die Kennung gerade nicht nennen kann.
    pub meeting_id: Option<String>,
    /// Beginn (ms UTC), wenn bekannt.
    pub started_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartRequest {
    pub title: String,
    /// Schluessel des Kalendertermins, falls der Ausloeser einer ist.
    pub event_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartedRecording {
    pub meeting_id: String,
    pub title: String,
}

/// Der Recorder, wie die Bausteine ihn sehen. Die App setzt den echten ein
/// (`hub::AppRecording`), Tests eine Attrappe. `start` darf NUR aufgerufen werden, wenn der
/// Nutzer eingewilligt hat; der Baustein stellt das sicher (`RunCtx::approved`).
pub trait RecordingControl: Send + Sync {
    /// Laeuft eine Aufnahme (auch eine, die gerade startet)? Der Takt fragt das fuer die
    /// Regel „nicht waehrend einer laufenden Aufnahme“.
    fn is_recording(&self) -> bool {
        self.current().is_some()
    }
    fn current(&self) -> Option<CurrentRecording>;
    /// Startet die Aufnahme. `Err` ist ein Code des Recorders (`already_recording`,
    /// `loopback_start_failed`, ...); es wurde dann nichts aufgenommen.
    fn start(&self, req: &StartRequest) -> Result<StartedRecording, String>;
    /// Beendet die laufende Aufnahme und liefert die Kennung der Besprechung.
    fn stop(&self) -> Result<String, String>;
}

pub(crate) fn start_failure(code: &str) -> String {
    let code = code.split(':').next().unwrap_or(code).trim();
    let text = match code {
        "already_recording" => "Es läuft schon eine Aufnahme.",
        "dictation_active" => "Ein Diktat läuft gerade; die Aufnahme konnte nicht starten.",
        "meetings_unavailable" => {
            "Die Besprechungen sind nicht verfügbar (Speicher nicht geöffnet)."
        }
        "loopback_start_failed" | "loopback_start_timeout" => {
            "Der Systemton ließ sich nicht aufnehmen."
        }
        "mic_stream_error" | "mic_start_failed" | "no_input_device" => {
            "Das Mikrofon ließ sich nicht öffnen."
        }
        _ => "Die Aufnahme konnte nicht starten.",
    };
    format!("{text} ({code})")
}

// ---------------------------------------------------------------------------
// Vorgemerkte Enden
// ---------------------------------------------------------------------------

/// Wann welche vom Ablauf gestartete Aufnahme enden soll. Im Speicher: eine Aufnahme ueber
/// einen Neustart hinweg gibt es nicht (Wiederherstellung schliesst sie ab).
#[derive(Default)]
pub struct StopSchedule {
    entries: Mutex<Vec<(String, i64)>>,
}

impl StopSchedule {
    pub fn add(&self, meeting_id: &str, at_ms: i64) {
        let mut e = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        e.retain(|(id, _)| id != meeting_id);
        if e.len() >= MAX_STOPS {
            e.remove(0);
        }
        e.push((meeting_id.to_string(), at_ms));
    }

    /// Nimmt die faelligen Eintraege heraus.
    pub fn take_due(&self, now_ms: i64) -> Vec<String> {
        let mut e = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut due = Vec::new();
        e.retain(|(id, at)| {
            if *at <= now_ms {
                due.push(id.clone());
                false
            } else {
                true
            }
        });
        due
    }

    pub fn entries(&self) -> Vec<(String, i64)> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Beendet die Aufnahmen, deren Ende faellig ist. Laeuft eine ANDERE Besprechung (der Nutzer
/// hat inzwischen eine neue gestartet), bleibt sie unberuehrt. Gibt die beendeten
/// Besprechungen zurueck. Haengt am gemeinsamen Takt (`hub::on_tick`).
pub fn run_due_stops(
    control: &dyn RecordingControl,
    stops: &StopSchedule,
    now_ms: i64,
) -> Vec<String> {
    let mut stopped = Vec::new();
    for meeting_id in stops.take_due(now_ms) {
        let Some(cur) = control.current() else {
            continue; // schon beendet (vom Nutzer)
        };
        if cur.meeting_id.as_deref().is_some_and(|id| id != meeting_id) {
            continue;
        }
        match control.stop() {
            Ok(id) => {
                log::info!("workflows: Aufnahme {id} zum geplanten Ende beendet");
                stopped.push(id);
            }
            Err(e) => log::warn!("workflows: Aufnahme {meeting_id} liess sich nicht beenden: {e}"),
        }
    }
    stopped
}

fn parse_iso(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Wann die Aufnahme enden soll (ms UTC); immer ein Zeitpunkt (Sicherheitsnetz).
pub fn plan_stop(params: &Value, trigger_end_ms: Option<i64>, now_ms: i64) -> i64 {
    let mode = text(params, "stop").unwrap_or("manual");
    // Nach dem Einsetzen kann die Zahl als Text ankommen (`"{{vars.minuten}}"`).
    let max_minutes = params
        .get("max_minutes")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
        .filter(|m| (1..=720).contains(m));
    let mut at: Option<i64> = None;
    if mode == "event_end" {
        at = trigger_end_ms;
    }
    if let Some(m) = max_minutes {
        let cap = now_ms.saturating_add(m.saturating_mul(60_000));
        at = Some(at.map_or(cap, |a| a.min(cap)));
    }
    at.unwrap_or_else(|| now_ms.saturating_add(DEFAULT_MAX_MINUTES * 60_000))
}

// ---------------------------------------------------------------------------
// Aufnahme starten
// ---------------------------------------------------------------------------

pub struct RecordingStart {
    spec: &'static ActionSpec,
    control: Arc<dyn RecordingControl>,
    stops: Arc<StopSchedule>,
}

impl RecordingStart {
    pub fn new(control: Arc<dyn RecordingControl>, stops: Arc<StopSchedule>) -> Self {
        Self {
            spec: catalog::action_spec("recording.start").expect("Katalogeintrag recording.start"),
            control,
            stops,
        }
    }
}

impl Action for RecordingStart {
    fn id(&self) -> &str {
        "recording.start"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::External
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<(), String> {
        if params.get("stop").and_then(Value::as_str) == Some("duration")
            && !params.contains_key("max_minutes")
        {
            return Err("Bei „duration“ ist „max_minutes“ nötig.".to_string());
        }
        Ok(())
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        // Sperre 3: nie ohne die eingeloeste Freigabe des Nutzers.
        if !ctx.approved {
            return Err(StepError::Denied(
                "Ohne Ihre Bestätigung startet keine Aufnahme.".to_string(),
            ));
        }
        let now = ctx.now_ms();
        let trigger = &ctx.context["trigger"];
        let end_ms = text(trigger, "end").and_then(parse_iso);
        if end_ms.is_some_and(|end| end <= now) {
            return Err(StepError::Permanent(
                "Der Termin ist schon zu Ende; es wurde keine Aufnahme gestartet.".to_string(),
            ));
        }
        if self.control.current().is_some() {
            return Err(StepError::Permanent(start_failure("already_recording")));
        }
        let title = text(params, "title")
            .or_else(|| text(trigger, "title"))
            .or_else(|| text(&ctx.context["meeting"], "title"))
            .unwrap_or("Besprechung")
            .to_string();
        let event_key = text(trigger, "event_id").map(str::to_string);
        let started = self
            .control
            .start(&StartRequest {
                title: title.clone(),
                event_key,
            })
            .map_err(|code| StepError::Permanent(start_failure(&code)))?;
        let stop_at = plan_stop(params, end_ms, now);
        self.stops.add(&started.meeting_id, stop_at);
        let mut out = StepOutput::with_data(json!({
            "meeting_id": started.meeting_id,
            "meeting": {"id": started.meeting_id, "title": started.title},
            "started_at": iso(now),
            "auto_stop_at": iso(stop_at),
        }));
        out.summary = Some(format!("Aufnahme „{}“ gestartet.", started.title));
        Ok(out)
    }

    fn confirm(&self, ctx: &RunCtx<'_>, _params: &Value) -> Option<StepOutput> {
        // Nach einem Absturz: laeuft eine Aufnahme, die NACH dem Schrittbeginn startete,
        // gehoert sie diesem Schritt.
        let cur = self.control.current()?;
        let id = cur.meeting_id?;
        if cur.started_at_ms? < ctx.step_started_at.saturating_sub(2_000) {
            return None;
        }
        let mut out = StepOutput::with_data(json!({
            "meeting_id": id,
            "meeting": {"id": id},
        }));
        out.summary = Some("Aufnahme nach dem Neustart bestätigt.".to_string());
        Some(out)
    }
}

// ---------------------------------------------------------------------------
// Aufnahme beenden
// ---------------------------------------------------------------------------

pub struct RecordingStop {
    spec: &'static ActionSpec,
    control: Arc<dyn RecordingControl>,
}

impl RecordingStop {
    pub fn new(control: Arc<dyn RecordingControl>) -> Self {
        Self {
            spec: catalog::action_spec("recording.stop").expect("Katalogeintrag recording.stop"),
            control,
        }
    }
}

impl Action for RecordingStop {
    fn id(&self) -> &str {
        "recording.stop"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn run(&self, _ctx: &RunCtx<'_>, _params: &Value) -> Result<StepOutput, StepError> {
        if self.control.current().is_none() {
            let mut out = StepOutput::with_data(json!({"stopped": false}));
            out.summary = Some("Es lief keine Aufnahme.".to_string());
            return Ok(out);
        }
        match self.control.stop() {
            Ok(id) => {
                let mut out = StepOutput::with_data(json!({
                    "stopped": true,
                    "meeting_id": id,
                    "meeting": {"id": id},
                }));
                out.summary = Some("Aufnahme beendet.".to_string());
                Ok(out)
            }
            Err(e) if e.starts_with("not_recording") => {
                Ok(StepOutput::with_data(json!({"stopped": false})))
            }
            // Beenden zu wiederholen ist sicher.
            Err(e) => Err(StepError::Transient(format!(
                "Die Aufnahme ließ sich nicht beenden ({e})."
            ))),
        }
    }
}

/// Haengt beide Bausteine in die Engine (ersetzt die Katalogbausteine).
pub fn install(engine: &Engine, control: Arc<dyn RecordingControl>, stops: Arc<StopSchedule>) {
    engine.register_action(Arc::new(RecordingStart::new(control.clone(), stops)));
    engine.register_action(Arc::new(RecordingStop::new(control)));
}

#[cfg(test)]
mod tests;
