//! Baustein des lokalen Agenten (Goal Lokaler Agent, Issue #68, Paket C3): `agent.route`.
//!
//! Das lokale Sprachmodell waehlt aus der Werkzeugliste des Schritts EIN Werkzeug und fuellt
//! dessen Inhalt (Betreff, Text, Zeitangabe); die Politik (`agent::policy`) prueft die Wahl.
//! Der Baustein **fuehrt nichts aus** und braucht deshalb kein Recht (`needs` = keines): er
//! ist `Pure` (ein Modellaufruf und ein Provenienz-Eintrag). Die Wirkung entsteht in den
//! folgenden Schritten des Ablaufs, die der Autor selbst festlegt und die jeweils ihr eigenes
//! Recht am Tor haben (`Caller::Workflow`: Recht, Freigabe mit Vorschau, Audit):
//!
//! ```json
//! { "id": "wahl", "action": "agent.route", "params": {
//!     "task": "Prüfe, ob eine Mitteilung zur Frist nötig ist.",
//!     "context": "{{steps.fristen.deadlines}}",
//!     "tools": ["notify_local", "send_mail"], "recipients": "participants" } },
//! { "id": "hinweis", "action": "notify.local",
//!   "when": "steps.wahl.tool == 'notify_local'",
//!   "params": { "title": "{{steps.wahl.arguments.title}}", "body": "{{steps.wahl.arguments.body}}" } },
//! { "id": "mail", "action": "mail.send", "when": "steps.wahl.tool == 'send_mail'",
//!   "params": { "via": "m365", "to": "participants", "subject": "{{steps.wahl.arguments.subject}}" } }
//! ```
//!
//! So ist die Whitelist doppelt gesichert: das Modell kann nur waehlen, was `tools` nennt und
//! die Politik durchlaesst, und ausgefuehrt wird nur, wofuer der Ablauf einen Schritt mit
//! passender Bedingung enthaelt. Ziel, Empfaenger, Konto, Pfad und Recht stehen in diesen festen
//! Schritten, nie in der Ausgabe des Modells (B5: `to` und `via` sind feste Felder).
//!
//! # Ausgabe (`steps.<id>.*`)
//!
//! | Feld | Inhalt |
//! |------|--------|
//! | `agent_route` | immer `true` (die Obergrenze zaehlt daran) |
//! | `outcome` | `tool` oder `no_action` |
//! | `tool`, `action` | das gewaehlte Werkzeug und der Baustein, den es meint (`no_action`/leer sonst) |
//! | `arguments` | die geprueften Felder des Werkzeugs (fehlende optionale als `""`), bei Zeitangaben `<name>_date` vom Code |
//! | `recipients` | die Empfaengermenge des Codes (nur bei `send_mail`) |
//! | `effect` | `true`, wenn ein Werkzeug gewaehlt wurde (zaehlt zur Obergrenze) |
//! | `reason`, `reason_text` | warum `no_action`: `model_no_action`, `tool_not_allowed`, `recipient_not_allowed`, `limit_reached`, `injection_suspected`, `schema_invalid`, ... |
//! | `actions_used`, `max_actions` | Wahlen frueherer Schritte dieses Laufs und die wirksame Obergrenze |
//! | `signals`, `notes` | erkannte Muster einer Aufforderung an die KI (Namen), Vermerke (verworfene Felder, gekuerzter Kontext) |
//! | `provenance` | Modell, Token, Dauer, Versuche, Konfidenz (im Code berechnet) |
//!
//! # Fehlerklassen des Schritts
//!
//! | Lage | Ergebnis |
//! |------|----------|
//! | RAM knapp, Server belegt, Neustart gesperrt | `Defer` (Rueckstau, kein Versuch verbraucht) |
//! | schwerer Platz belegt (anderer Agent-/Modellschritt) | der Lauf wartet am `HeavyGate` (`heavy_slot`), das Modell wird nicht gefragt |
//! | Server nicht erreichbar / bricht ab, Zeit (60 s) | `Transient` (nichts entschieden, nichts geschrieben) |
//! | Modell nicht geladen, Anfrage abgelehnt, ungueltige Parameter | `Permanent` |
//! | Antwort auch im 2. Versuch kein gueltiges JSON, Politik verwirft die Wahl, Obergrenze, Kontext faellt auf | **Erfolg** mit `outcome: no_action` und Grund |
//! | Nutzer bricht ab | `Transient` (Anfrage faellt) |
//!
//! # Fehlerfaelle (C3) und ihre Absicherung
//!
//! | # | Fehlerfall | Verhalten | Beleg |
//! |---|------------|-----------|-------|
//! | 1 | **Nebenlaeufigkeit**: zwei Laeufe, zwei Schritte gleichzeitig; belegter Slot | das `HeavyGate` haelt schwere Schritte seriell, der zweite wartet; der Server serialisiert (`--parallel 1`); belegt er (503), `Defer`; die Obergrenze zaehlt aus dem Journal des EIGENEN Laufs | `a_busy_heavy_slot_*`, `a_busy_server_*`, `two_runs_*` |
//! | 2 | **Abbruch mitten im Vorgang**: App stirbt nach der Provenienz, vor dem Journal | `Pure`: die Engine wiederholt; die Provenienz entsteht hoechstens einmal je Schritt; eine Wahl ohne Journal hat nichts ausgeloest (der Baustein wirkt nie) | `a_second_attempt_*` |
//! | 3 | **Voller Datentraeger / gesperrte Datenbank** | Provenienz-Fehler scheitern nie den Schritt; ein Fehler am Tor trifft den folgenden Wirkschritt, nicht diesen | `a_broken_provenance_*` |
//! | 4 | **Fehlendes Geraet / Modell** | Modell nicht geladen -> `Permanent` mit Klartext, der Server startet nicht; kein Audio beteiligt | `the_router_model_*`, `server_trouble_*` |
//! | 5 | **Absturz des Kindprozesses (llama-server)** | Verbindungsabbruch -> `Transient`; `server_crashed:` -> `Defer` (Neustart eine Minute gesperrt) | Laufzeit-Tests, `server_trouble_*` |
//! | 6 | **Voller Arbeitsspeicher** | `HeavyGate` prueft vorher (RAM-Start-Tor, `process_guard`); `memory_low` des Verwalters -> `Defer`; der Rechner bleibt bedienbar | `a_busy_heavy_slot_*`, Engine-Tests B1 |
//! | 7 | **Feindliche Eingabe** (Einschleusen im Transkript/Mail/Video) | Kontext mit Aufforderung an die KI -> `no_action` ohne Modellaufruf; sonst Whitelist, Empfaengermenge, Obergrenze und Feldpruefung; Wirkung nur ueber den Folgeschritt mit Freigabe | `injection_*`, `every_eval_injection_*` |
//! | 8 | **Obergrenze** | je Lauf hoechstens `max_actions` (1..=10) Wahlen, auch ueber mehrere `agent.route`-Schritte | `the_limit_*` |
//! | 9 | **Trockenlauf** | der Engine-Trockenlauf plant den Baustein ohne Modellaufruf; [`preview`] zeigt die Modellentscheidung OHNE Wirkung (kein Audit, keine Freigabe, keine Provenienz, kein Folgeschritt) und nimmt den Platz am `HeavyGate` | `the_preview_*`, `the_engine_dry_run_*` |

use std::sync::Arc;

use chrono::{Local, NaiveDate, TimeZone};
use serde_json::{json, Map, Value};

use crate::agent::dates;
use crate::agent::policy::{self, Verdict, Whitelist};
use crate::agent::route::{self, Decision, RouteRequest, MAX_CONTEXT_CHARS, MAX_TASK_CHARS};
use crate::agent::runtime::{AgentError, AgentRuntime};
use crate::managers::provenance::{ActorKind, Locality, NewProvenance, SourceRef, SubjectKind};

use super::action::{
    Action, EffectKind, HeavyNeed, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::app_actions::{
    meeting_id_of, previous_result, run_cancellable, spec_of, svc, AppServices,
};
use super::catalog;
use super::engine::Engine;
use super::heavy::HeavyGate;
use super::integration_actions::recipients::{self, attendees_of, Rule, Sources};

const OPERATION: &str = "agent_route";
/// Hoechstzahl Vermerke im Ergebnis.
const MAX_NOTES: usize = 10;

/// Bedarf des Schritts am `HeavyGate`: das Router-Modell laeuft immer lokal (unabhaengig vom
/// Anbieter der Nachbearbeitung).
pub fn heavy_need() -> HeavyNeed {
    HeavyNeed {
        ram_mb: 6_144,
        label: "Sprachmodell (Agent)",
    }
}

// -- Parameter ------------------------------------------------------------------------------------

struct RouteParams {
    task: String,
    context: Option<String>,
    whitelist: Whitelist,
    rule: Option<Rule>,
    list: Vec<String>,
    max_actions: u32,
    model: Option<String>,
    reference: Option<NaiveDate>,
}

fn text_list(params: &Value, key: &str) -> Result<Vec<String>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("{key}: Liste aus Texten erwartet"))
            })
            .collect(),
        Some(_) => Err(format!("{key}: Liste aus Texten erwartet")),
    }
}

/// Die Daten fuer die Wahl als Text: ein Text bleibt, alles andere (Liste/Objekt aus einem
/// Vorschritt) wird zu JSON-Text. Gedeckelt (die Laufzeit kuerzt ohnehin auf das Budget).
fn context_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    let cut: String = text.chars().take(MAX_CONTEXT_CHARS * 4).collect();
    (!cut.trim().is_empty()).then_some(cut)
}

/// `templated`: Werte mit `{{...}}` stehen noch nicht fest (Pruefung beim Speichern).
fn parse_params(params: &Value, templated: bool) -> Result<RouteParams, String> {
    let task = params
        .get("task")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "task: die Aufgabe fehlt".to_string())?;
    if task.chars().count() > MAX_TASK_CHARS {
        return Err(format!("task: höchstens {MAX_TASK_CHARS} Zeichen"));
    }
    let tools = text_list(params, "tools")?;
    let whitelist = Whitelist::parse(&tools).map_err(|e| format!("tools: {e}"))?;

    let rule = match params.get("recipients") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(
            Rule::parse(s).ok_or_else(|| "recipients: unbekannte Empfängerregel".to_string())?,
        ),
        Some(_) => return Err("recipients: Text erwartet".to_string()),
    };
    let list = text_list(params, "list")?;
    if whitelist.has_mail() && rule.is_none() {
        return Err(
            "recipients: mit dem Werkzeug send_mail ist die Empfängerregel Pflicht (die Empfänger bildet das Programm, nie das Modell)"
                .to_string(),
        );
    }
    if !whitelist.has_mail() && (rule.is_some() || !list.is_empty()) {
        return Err("recipients/list: gelten nur zusammen mit dem Werkzeug send_mail".to_string());
    }
    match rule {
        Some(Rule::List) if list.is_empty() => {
            return Err("list: die feste Empfängerliste fehlt".to_string())
        }
        Some(r) if r != Rule::List && !list.is_empty() => {
            return Err("list: gilt nur mit der Regel „list“".to_string())
        }
        _ => {}
    }

    let max_actions = match params.get("max_actions") {
        None | Some(Value::Null) => policy::DEFAULT_MAX_ACTIONS,
        Some(v) => match v.as_u64() {
            Some(n) if (1..=u64::from(policy::HARD_MAX_ACTIONS)).contains(&n) => n as u32,
            _ => {
                return Err(format!(
                    "max_actions: Zahl zwischen 1 und {}",
                    policy::HARD_MAX_ACTIONS
                ))
            }
        },
    };
    let model = match params.get("model") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) if route::valid_model_id(s.trim()) => Some(s.trim().to_string()),
        Some(_) => return Err("model: keine gültige Modellkennung".to_string()),
    };
    let reference = match params.get("reference_date") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) if templated && s.contains("{{") => None,
        Some(Value::String(s)) => {
            let head: String = s.trim().chars().take(10).collect();
            Some(
                dates::parse_iso(&head)
                    .map_err(|_| "reference_date: kein ISO-Datum (JJJJ-MM-TT)".to_string())?,
            )
        }
        Some(_) => return Err("reference_date: Text erwartet".to_string()),
    };
    Ok(RouteParams {
        task: task.to_string(),
        context: params.get("context").and_then(context_text),
        whitelist,
        rule,
        list,
        max_actions,
        model,
        reference,
    })
}

pub(super) fn today(now_ms: i64) -> NaiveDate {
    Local
        .timestamp_millis_opt(now_ms)
        .single()
        .map(|d| d.date_naive())
        .unwrap_or_else(|| chrono::Utc::now().date_naive())
}

/// Wie ein Fehler des Servers den Schritt trifft (siehe Moduldoku).
fn step_error(e: &AgentError) -> StepError {
    match e {
        AgentError::NotConfigured(m) => StepError::Permanent(m.clone()),
        AgentError::Rejected { .. } => StepError::Permanent(e.describe()),
        AgentError::Busy {
            retry_after_ms,
            reason,
        } => StepError::Defer {
            retry_after_ms: *retry_after_ms,
            reason: reason.clone(),
        },
        // Server weg, Zeit, unerwartet: nichts entschieden, ein neuer Versuch ist sicher.
        _ => StepError::Transient(e.describe()),
    }
}

// -- Entscheidung ---------------------------------------------------------------------------------

/// Was die Rechnung ergibt: Parameter, Urteil, Zaehler.
struct Computed {
    decision: Decision,
    used: u32,
    limit: u32,
}

/// Die Empfaengermenge des Codes (B5) fuer `send_mail`; scheitert die Regel (keine Teilnehmenden,
/// keine eigene Adresse), ist sie leer und der Grund steht in den Vermerken.
fn recipient_set(
    services: &dyn AppServices,
    context: &Value,
    p: &RouteParams,
) -> (Vec<String>, Option<String>) {
    let Some(rule) = p.rule.filter(|_| p.whitelist.has_mail()) else {
        return (Vec::new(), None);
    };
    let attendees = attendees_of(context);
    let self_emails = services.self_emails();
    let sources = Sources {
        attendees: &attendees,
        self_emails: &self_emails,
        own_address: None,
        list: &p.list,
    };
    match recipients::resolve(rule, &sources) {
        Ok(set) => (set, None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    }
}

fn compute(
    services: &dyn AppServices,
    context: &Value,
    params: &Value,
    now_ms: i64,
    sample_context: Option<&str>,
    templated: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<Computed, StepError> {
    let mut p = parse_params(params, templated).map_err(StepError::Permanent)?;
    if let Some(sample) = sample_context {
        p.context = context_text(&Value::String(sample.to_string()));
    }
    let reference = p.reference.unwrap_or_else(|| today(now_ms));
    let used = policy::actions_used(context);
    let limit = policy::effective_limit(p.max_actions);
    let (recipients, recipient_note) = recipient_set(services, context, &p);
    let req = RouteRequest {
        task: &p.task,
        context: p.context.as_deref(),
        whitelist: &p.whitelist,
        recipients: &recipients,
        reference,
        max_actions: p.max_actions,
        used_actions: used,
    };
    let mut decision = match route::precheck(&req) {
        // Ohne das Modell entschieden: kein Server, kein Modellname noetig.
        Some(d) => d,
        None => {
            let target = services
                .agent_route_target(p.model.as_deref())
                .map_err(svc)?;
            let runtime = AgentRuntime::new(target).with_config(route::route_config());
            match run_cancellable(cancel, route::decide(&runtime, &req)) {
                None => {
                    return Err(StepError::Transient(
                        "Der Lauf wurde abgebrochen.".to_string(),
                    ))
                }
                Some(Ok(d)) => d,
                Some(Err(e)) => return Err(step_error(&e)),
            }
        }
    };
    if let Some(note) = recipient_note {
        decision.notes.push(note);
    }
    decision.notes.truncate(MAX_NOTES);
    Ok(Computed {
        decision,
        used,
        limit,
    })
}

/// Das Ergebnis des Schritts (`steps.<id>.*`).
fn output_data(c: &Computed) -> Map<String, Value> {
    let d = &c.decision;
    let (reason, reason_text) = d.reason();
    let (outcome, tool, action, arguments, recipients) = match &d.verdict {
        Verdict::Run(a) => (
            "tool",
            a.tool,
            a.action,
            Value::Object(a.arguments.clone()),
            json!(a.recipients),
        ),
        _ => ("no_action", "no_action", "", json!({}), json!([])),
    };
    let mut out = Map::new();
    out.insert("agent_route".into(), json!(true));
    out.insert("outcome".into(), json!(outcome));
    out.insert("tool".into(), json!(tool));
    out.insert("action".into(), json!(action));
    out.insert("arguments".into(), arguments);
    out.insert("recipients".into(), recipients);
    out.insert("effect".into(), json!(outcome == "tool"));
    out.insert("reason".into(), json!(reason));
    out.insert("reason_text".into(), json!(reason_text));
    out.insert(
        "requested_tool".into(),
        json!(d.requested_tool().unwrap_or("")),
    );
    out.insert("actions_used".into(), json!(c.used));
    out.insert("max_actions".into(), json!(c.limit));
    out.insert("signals".into(), json!(d.signals));
    out.insert("notes".into(), json!(d.notes));
    out.insert("model_called".into(), json!(d.model_called));
    out.insert("context_truncated".into(), json!(d.context_truncated));
    out.insert(
        "provenance".into(),
        json!({
            "model": if d.model_called { d.model.as_str() } else { "" },
            "local": d.local,
            "attempts": d.attempts,
            "prompt_tokens": d.usage.prompt_tokens,
            "completion_tokens": d.usage.completion_tokens,
            "duration_ms": d.duration_ms,
            "confidence": d.confidence,
        }),
    );
    out
}

fn summary_of(c: &Computed) -> String {
    match &c.decision.verdict {
        Verdict::Run(a) => format!("Werkzeug gewählt: {} (Baustein {}).", a.tool, a.action),
        _ => {
            let (_, text) = c.decision.reason();
            if text.is_empty() {
                "Keine Aktion gewählt.".to_string()
            } else {
                format!("Keine Aktion: {text}")
            }
        }
    }
}

// -- Der Baustein ---------------------------------------------------------------------------------------

pub struct AgentRoute {
    spec: &'static catalog::ActionSpec,
    services: Arc<dyn AppServices>,
}

impl AgentRoute {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("agent.route"),
            services,
        }
    }

    /// Provenienz des Schritts: Modell, Token, Dauer, Quellen, Konfidenz. Nie ein Grund zu
    /// scheitern; hoechstens ein Eintrag je Schritt.
    fn record(&self, ctx: &RunCtx<'_>, c: &Computed, sources: &[SourceRef]) {
        if previous_result(ctx, SubjectKind::RunOutput, OPERATION).is_some() {
            return;
        }
        let d = &c.decision;
        let mut entry = NewProvenance::new(
            SubjectKind::RunOutput,
            &ctx.idempotency_key,
            OPERATION,
            ActorKind::Workflow,
        );
        if d.model_called {
            entry.provider = Some(if d.local { "local" } else { "remote" }.to_string());
            entry.locality = Some(if d.local {
                Locality::Local
            } else {
                Locality::Remote
            });
            entry.model_id = Some(d.model.clone());
            entry.prompt_tokens = Some(d.usage.prompt_tokens);
            entry.completion_tokens = Some(d.usage.completion_tokens);
            entry.duration_ms = Some(d.duration_ms);
        }
        entry.sources = sources.to_vec();
        entry.confidence = d.confidence;
        let (reason, _) = d.reason();
        entry.params = Some(json!({
            "outcome": if d.verdict.is_run() { "tool" } else { "no_action" },
            "tool": match &d.verdict { Verdict::Run(a) => a.tool, _ => "no_action" },
            "reason": reason,
            "model_called": d.model_called,
            "attempts": d.attempts,
            "actions_used": c.used,
            "max_actions": c.limit,
            "signals": d.signals,
        }));
        if let Err(e) = ctx.record_provenance(entry) {
            log::warn!(
                "workflows: Provenienz fuer {}/{} nicht geschrieben: {e}",
                ctx.run_id,
                ctx.step_id
            );
        }
    }
}

impl Action for AgentRoute {
    fn id(&self) -> &str {
        "agent.route"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Pure
    }

    fn heavy(&self, _params: &Value) -> Option<HeavyNeed> {
        Some(heavy_need())
    }

    fn needs(&self, _params: &Value) -> Result<Option<Needs>, NeedsError> {
        Ok(None)
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<(), String> {
        parse_params(&Value::Object(params.clone()), true).map(|_| ())
    }

    fn describe(&self, params: &Value) -> String {
        match parse_params(params, true) {
            Ok(p) => format!(
                "Das lokale Sprachmodell wählt aus {} freigegebenen Werkzeugen ({}) höchstens eines; es führt nichts selbst aus. Die folgenden Schritte wirken nur nach ihrer Bedingung und mit ihren Rechten (höchstens {} Aktionen je Lauf).",
                p.whitelist.len(),
                p.whitelist.names().join(", "),
                policy::effective_limit(p.max_actions)
            ),
            Err(_) => catalog::describe_from_spec(self.spec, params),
        }
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let cancel = || ctx.cancelled();
        let computed = compute(
            &*self.services,
            ctx.context,
            params,
            ctx.now_ms(),
            None,
            false,
            &cancel,
        )?;
        let sources: Vec<SourceRef> = meeting_id_of(ctx)
            .map(|id| vec![SourceRef::new("meeting", &id, None)])
            .unwrap_or_default();
        self.record(ctx, &computed, &sources);
        let mut out = StepOutput::with_data(Value::Object(output_data(&computed)))
            .summary(&summary_of(&computed));
        out.sources = sources;
        out.confidence = computed.decision.confidence;
        Ok(out)
    }
}

/// Trockenlauf mit Modellentscheidung: was WUERDE der Schritt waehlen? Dieselbe Rechnung wie
/// `run`, aber ohne jede Wirkung: kein Journal, keine Provenienz, kein Audit, keine Freigabe,
/// kein Folgeschritt (das Ergebnis nennt nur, welcher Baustein mit welchen Argumenten dran
/// waere). Der Aufruf nimmt den Platz am `HeavyGate` selbst (AK9): ist er belegt, `Defer`, und
/// das Modell wird nicht gefragt. `context` ist der Laufkontext (im Trockenlauf der Plan mit den
/// Beispieldaten des Ausloesers), `sample_context` ein Beispieltext fuer das Feld `context`, wenn
/// es von einem Vorschritt abhaengt.
pub fn preview(
    services: &dyn AppServices,
    gate: &dyn HeavyGate,
    context: &Value,
    params: &Value,
    now_ms: i64,
    sample_context: Option<&str>,
    cancel: &dyn Fn() -> bool,
) -> Result<Value, StepError> {
    // Haengt der Kontext von einem Vorschritt ab, der im Trockenlauf kein Ergebnis hat, wuerde das
    // Modell die Vorlage selbst als "Daten" sehen: ohne Beispieltext keine Entscheidung.
    let probe = parse_params(params, true).map_err(StepError::Permanent)?;
    if sample_context.is_none() && probe.context.as_deref().is_some_and(|c| c.contains("{{")) {
        return Ok(json!({
            "dry_run": true,
            "writes": "nothing",
            "skipped": true,
            "reason": "context_unresolved",
            "reason_text": "Der Kontext hängt vom Ergebnis eines Vorschritts ab: für die Modellentscheidung im Trockenlauf einen Beispieltext angeben.",
        }));
    }
    let _permit = gate
        .try_enter(&heavy_need())
        .map_err(|w| StepError::Defer {
            retry_after_ms: w.retry_after_ms,
            reason: w.message,
        })?;
    let computed = compute(
        services,
        context,
        params,
        now_ms,
        sample_context,
        true,
        cancel,
    )?;
    let mut data = output_data(&computed);
    data.insert("dry_run".into(), json!(true));
    data.insert("writes".into(), json!("nothing"));
    if let Verdict::Run(a) = &computed.decision.verdict {
        data.insert(
            "would_run".into(),
            json!({ "action": a.action, "tool": a.tool, "arguments": a.arguments, "recipients": a.recipients }),
        );
    }
    Ok(Value::Object(data))
}

/// Haengt den Baustein in die Engine (ersetzt den Katalogbaustein).
pub fn install(engine: &Engine, services: Arc<dyn AppServices>) {
    engine.register_action(Arc::new(AgentRoute::new(services)));
}

#[cfg(test)]
mod tests;
