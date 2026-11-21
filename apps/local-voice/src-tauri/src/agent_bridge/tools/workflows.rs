//! Die Werkzeuge der Automationen (B8): ein externer Agent liest die Abläufe, startet einen
//! Ablauf und liest sein Laufprotokoll. Rechte, Freigaben und Audit erledigt die Bruecke (A7)
//! VOR dem Aufruf; die Handler hier setzen um und pruefen ihre Eingaben selbst.
//!
//! | Werkzeug | Recht | Wirkung |
//! |---|---|---|
//! | `list_workflows` | `workflow.read` | Abläufe mit Auslöser, Schaltzustand, Variablen, letztem Lauf (ohne Definition) |
//! | `run_workflow` | `workflow.run` | reiht einen Lauf mit Herkunft `agent` ein; Standard Trockenlauf |
//! | `get_run` | `workflow.read` | Laufprotokoll: Zustand, Schritte, Fehler; ohne Geheimnisse, Auslöserdaten, Definition und Eingaben |
//!
//! # Wann ein Lauf scharf wird
//! `run_workflow` plant standardmaessig nur (Trockenlauf: nichts wird geschrieben, versendet oder
//! aufgenommen). `live: true` startet einen echten Lauf, aber nur wenn ALLES gilt:
//! 1. der Ablauf ist vom Nutzer eingeschaltet UND scharf geschaltet (sonst Fehler, es entsteht
//!    KEIN Lauf: ein Agent soll nie glauben, etwas sei gelaufen),
//! 2. sein Ausloeser ist „Von Hand starten“ oder „Durch einen Agenten“ (ein Ablauf, der durch ein Ereignis
//!    startet, braucht dessen Daten: ein Agent plant ihn hoechstens im Trockenlauf), und
//! 3. das Werkzeugrecht des Zugangs erlaubt den Aufruf (`workflow.run` steht auf „fragen“ und der
//!    Nutzer hat freigegeben, oder auf „erlaubt“). Das prueft das Tor der Bruecke vor dem Handler.
//!
//! Die Schritte eines scharfen Laufs gehen wie bei jedem Lauf durch das Tor als Aufrufer
//! `workflow`: Rechte des Ablaufs, Freigaben und Einwilligung zur Aufnahme gelten weiter. Ein
//! Agent kann dadurch nie mehr ausloesen, als der Nutzer dem Ablauf ohnehin erlaubt hat.
//!
//! # Fehlerfaelle und ihre Absicherung (Tests in `workflows/tests.rs`)
//!
//! | # | Fehlerfall | Verhalten | Absicherung |
//! |---|---|---|---|
//! | 1 | Zwei Aufrufe starten dasselbe zugleich oder ein Agent wiederholt nach einem Abbruch | ohne `request_id` ist jeder Aufruf ein eigener Lauf (je Minute begrenzt, Bruecke); MIT `request_id` ergibt dieselbe Angabe denselben Lauf (`created: false`), auch ueber Neustart (Schluessel + `UNIQUE`) | `the_same_request_id_is_the_same_run`, `without_a_request_id_every_call_is_a_run` |
//! | 2 | Abbruch mitten im Vorgang (App endet nach dem Einreihen) | der Lauf ist eine Zeile in der Warteschlange und wird von der Engine wieder aufgenommen (B1); `get_run` zeigt `queued`/`running` | Engine-Tests (B1), `a_dry_run_is_planned_and_readable_through_get_run` |
//! | 3 | Platte voll / Datenbank gesperrt | das Einreihen scheitert mit Klartext, es entsteht kein Lauf; `store_unavailable`-Texte der Bruecke bleiben | `a_store_failure_is_a_plain_message` |
//! | 4 | Fehlendes Geraet, Absturz eines Kindprozesses | nicht beteiligt: der Handler startet nichts, die Schritte laufen in der Engine (Tore, `process_guard`, Einwilligung) | Engine-Tests (B1/B2) |
//! | 5 | Voller Arbeitsspeicher | der Handler braucht keinen; schwere Schritte warten am RAM-Tor der Engine; je Ablauf hoechstens 500 wartende Laeufe (`queue_full`, Klartext) | `engine_errors_become_plain_german_messages` |
//! | 6 | Agent will scharf starten, der Ablauf ist nicht scharf, ausgeschaltet oder startet durch ein Ereignis | Fehler mit Erklaerung, KEIN Lauf; auch ohne `live` bleibt es beim Trockenlauf | `live_needs_an_armed_and_enabled_workflow`, `live_is_refused_for_event_triggered_workflows`, `a_disabled_workflow_allows_only_the_dry_run` |
//! | 7 | Agent ohne Recht / Recht „aus“ | das Werkzeug fehlt in der Liste, ein Aufruf ist `tool_off` (ctl Exit 3); „fragen“ -> Freigabe in der App, einmalig, an die Argumente (`live`, `vars`) gebunden | `run_workflow_without_the_right_is_refused_and_no_run_exists`, `ask_binds_the_approval_to_live` |
//! | 8 | Unvertraute Argumente (falscher Typ, unbekanntes Feld, riesige `vars`, Steuerzeichen in Kennungen) | strenge Pruefung, klare Meldung, nichts eingereiht | `arguments_are_checked_strictly` |
//! | 9 | Geheimnisse im Protokoll | `get_run` liefert keine Definition, keine Auslöserdaten, keine Eingaben; Ausgaben und Fehler laufen durch die Schwaerzung des Audits (Schluessel wie `token`, Adressen auf den Host gekuerzt, Texte und Groesse begrenzt), der Plan eines Trockenlaufs verliert die eingesetzten Parameter | `get_run_hides_secrets_and_trigger_data`, `step_outputs_and_errors_are_scrubbed_and_bounded` |
//! | 10 | Die Engine gibt es nicht (headless ohne Hub, App startet noch) | Klartext, keine Panik | `without_an_engine_the_tools_say_so` |
//! | 11 | Der Lauf hat eine Besprechung gelesen (Zusammenfassung, Protokoll, auch abgeleitete Ausgaben spaeterer Schritte), die fuer KI-Clients nicht freigegeben ist (unbestaetigt, widerrufen, „nur lokal“) | Ausgaben, Fehler- und Wartetexte des GANZEN Laufs stehen als `besprechung_nicht_freigegeben` da, `withheld: true`; Bezug unbestimmt = gesperrt (G9a-2, Codex #583) | `get_run_hides_the_content_of_a_meeting_that_is_not_released` |
//!
//! Audio-Echtzeitpfad: nicht beruehrt. Kindprozesse: keine.

use std::collections::BTreeSet;

use serde_json::{json, Map, Value};

use super::{clean_line, spec, string_prop, AppTools, Args};
use crate::agent_bridge::catalog::{CallContext, ToolSpec};
use crate::managers::integrations::audit;
use crate::managers::workflows::engine::Engine;
use crate::managers::workflows::model::Origin;
use crate::managers::workflows::store::WorkflowError;
use crate::managers::workflows::trigger::{is_automatic, manual};
use crate::managers::workflows::{ui, validate};

/// Hoechstens so viele Abläufe in der Liste.
pub const MAX_WORKFLOWS_LISTED: usize = 200;
/// Hoechstens so viele Schritt-Zeilen (Versuche eingerechnet) in einem Laufprotokoll.
pub const MAX_STEPS_LISTED: usize = 200;
/// Groesse der Werte fuer deklarierte Variablen (serialisiert).
pub const MAX_VARS_BYTES: usize = 16 * 1024;
/// Laengster Text (Fehler, Ausgabe) im Laufprotokoll.
const MAX_LOG_TEXT_CHARS: usize = 400;
/// Laengster Text in der Ausgabe eines Schritts (die Ausgabe ist eine Zusammenfassung, kein Inhalt).
const MAX_OUTPUT_STRING_CHARS: usize = 160;
/// Groesse der Ausgabe eines Schritts im Laufprotokoll (serialisiert, nach der Schwaerzung).
const MAX_STEP_OUTPUT_BYTES: usize = 2048;

pub fn specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "list_workflows",
            "Listet die Abläufe der Automationen von Local Voice AI: Kennung, Name, Auslöser, ob der Ablauf \
             eingeschaltet und scharf geschaltet ist (armed), die deklarierten Variablen und den letzten Lauf. \
             Nur ein scharf geschalteter Ablauf kann mit run_workflow und live=true echt laufen. Die Definition \
             der Schritte wird nicht geliefert.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        spec(
            "run_workflow",
            "Startet einen Ablauf der Automationen. Standard ist ein TROCKENLAUF: die App plant jeden Schritt und \
             zeigt, was er täte und ob er dürfte; nichts wird geschrieben, gesendet oder aufgenommen. Mit \
             live=true startet ein echter Lauf, aber nur, wenn der Nutzer den Ablauf eingeschaltet und scharf \
             geschaltet hat und das Recht dieses Zugangs es erlaubt; sonst kommt ein Fehler und es entsteht kein \
             Lauf. Rechte, Freigaben und die Einwilligung zur Aufnahme der einzelnen Schritte gelten wie immer. \
             Liefert run_id; das Protokoll liest get_run.",
            json!({
                "type": "object",
                "properties": {
                    "workflow_id": string_prop("Kennung des Ablaufs aus list_workflows.", 64),
                    "live": { "type": "boolean", "default": false,
                              "description": "true: echter Lauf (nur bei scharfem Ablauf). Standard false: Trockenlauf." },
                    "vars": { "type": "object", "description": "Werte für die deklarierten Variablen des Ablaufs (Name -> Wert)." },
                    "request_id": string_prop("Eigene Kennung dieser Anfrage: dieselbe Kennung startet keinen zweiten Lauf (Wiederholung nach einem Abbruch).", 64)
                },
                "required": ["workflow_id"],
                "additionalProperties": false
            }),
        ),
        spec(
            "get_run",
            "Liest das Protokoll eines Laufs: Zustand (queued, running, awaiting_approval, done, failed, cancelled), \
             ob es ein Trockenlauf war, Herkunft, Zeiten, Fehler und je Schritt Zustand und gekürzte Ausgabe. \
             Ohne Auslöserdaten, Definition, Eingaben und Geheimnisse. Ein wartender Lauf braucht die Freigabe \
             des Nutzers in der App.",
            json!({
                "type": "object",
                "properties": { "run_id": string_prop("Kennung des Laufs aus run_workflow oder list_workflows.", 64) },
                "required": ["run_id"],
                "additionalProperties": false
            }),
        ),
    ]
}

impl Args {
    /// `true`/`false`; `null` und fehlend zaehlen als nicht angegeben.
    fn boolean(&mut self, key: &str) -> Result<Option<bool>, String> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(b)),
            Some(_) => Err(format!("„{key}“ muss true oder false sein.")),
        }
    }

    /// Ein JSON-Objekt, serialisiert hoechstens `max_bytes` gross.
    fn object(
        &mut self,
        key: &str,
        max_bytes: usize,
    ) -> Result<Option<Map<String, Value>>, String> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::Object(m)) => {
                let size = Value::Object(m.clone()).to_string().len();
                if size > max_bytes {
                    return Err(format!(
                        "„{key}“ ist zu groß (höchstens {} KiB).",
                        max_bytes / 1024
                    ));
                }
                Ok(Some(m))
            }
            Some(_) => Err(format!("„{key}“ muss ein Objekt sein.")),
        }
    }
}

/// Klartext fuer einen Fehler der Engine. Datenbankdetails bleiben im Log.
fn wf_error(e: &WorkflowError) -> String {
    match e {
        WorkflowError::Store(m) => {
            log::warn!("agent_bridge: Automationen: Datenbankfehler: {m}");
            "Die Datenbank der Automationen ist gerade nicht verfügbar. Es wurde nichts gestartet."
                .to_string()
        }
        WorkflowError::NotFound(_) => "Den Ablauf oder Lauf gibt es nicht.".to_string(),
        WorkflowError::Invalid(_) => {
            "Die Definition des Ablaufs ist ungültig; der Nutzer muss sie in der App korrigieren."
                .to_string()
        }
        other => audit::sanitize_audit_text(&other.to_string(), MAX_LOG_TEXT_CHARS),
    }
}

/// Schwaerzt einen Wert fuer das Protokoll: Geheimnis-Schluessel `***`, Texte durch die Schwaerzung
/// des Audits (Adressen auf den Host), alles begrenzt.
fn scrub(v: &Value, depth: usize) -> Value {
    if depth >= 4 {
        return Value::String("…".to_string());
    }
    match v {
        Value::String(s) => Value::String(audit::sanitize_audit_text(s, MAX_OUTPUT_STRING_CHARS)),
        Value::Array(items) => {
            Value::Array(items.iter().take(10).map(|x| scrub(x, depth + 1)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .take(30)
                .map(|(k, val)| {
                    let shown = if audit::is_secret_key(k) {
                        Value::String("***".to_string())
                    } else {
                        scrub(val, depth + 1)
                    };
                    (k.chars().take(60).collect::<String>(), shown)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Ausgabe eines Schritts fuer das Protokoll: geschwaerzt und auf eine feste Groesse begrenzt.
fn step_output(raw: Option<&str>) -> Value {
    let Some(raw) = raw else {
        return Value::Null;
    };
    let mut value =
        serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
    // Der Plan eines Trockenlaufs nennt die EINGESETZTEN Parameter (Auslöserdaten, Empfänger,
    // Texte): die bekommt ein Agent nie, nur Wirkung, Bedingung und Rechte-Ergebnis.
    if let Value::Object(map) = &mut value {
        map.remove("params");
    }
    let shown = scrub(&value, 0);
    let size = shown.to_string().len();
    if size > MAX_STEP_OUTPUT_BYTES {
        return json!({ "truncated": true, "bytes": size });
    }
    shown
}

/// Worauf sich ein Lauf an Besprechungen bezieht (G9a-2, Codex #583 Fixrunde 3).
#[derive(Debug, PartialEq, Eq)]
enum Bezug {
    /// Kein Hinweis auf eine Besprechung.
    Keiner,
    /// Genau diese Besprechungen.
    Besprechungen(Vec<String>),
    /// Ein Hinweis auf eine Besprechung, aber nicht, auf welche: gilt als gesperrt.
    Unbestimmt,
}

/// Kennungen unter einem Besprechungsschluessel (`meeting_id`, `meeting`, `meetings`, `meeting_ids`):
/// Text, Objekt mit `id` oder eine Liste davon. Alles andere ist `Err` (unbestimmt).
fn kennungen(v: &Value, out: &mut BTreeSet<String>) -> Result<(), ()> {
    match v {
        Value::Null => Ok(()),
        Value::String(id) if !id.trim().is_empty() => {
            out.insert(id.trim().to_string());
            Ok(())
        }
        Value::Object(map) => match map.get("id") {
            Some(Value::String(id)) if !id.trim().is_empty() => {
                out.insert(id.trim().to_string());
                Ok(())
            }
            _ => Err(()),
        },
        Value::Array(items) => items.iter().try_for_each(|x| kennungen(x, out)),
        _ => Err(()),
    }
}

/// Sucht in einem JSON-Wert jeden Schluessel, der eine Besprechung nennt. Ein Schluessel mit
/// „meeting“ im Namen, aus dem sich keine Kennung lesen laesst (etwa `meeting_title`), macht den
/// ganzen Lauf unbestimmt: lieber zu viel zurueckhalten als Inhalt einer gesperrten Besprechung.
fn besprechungen_in(v: &Value, out: &mut BTreeSet<String>) -> Result<(), ()> {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                let key = k.to_ascii_lowercase();
                if key.contains("meeting") {
                    match key.as_str() {
                        "meeting_id" | "meeting" | "meetings" | "meeting_ids" => {
                            kennungen(val, out)?
                        }
                        _ if val.is_null() => {}
                        _ => return Err(()),
                    }
                }
                besprechungen_in(val, out)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(|x| besprechungen_in(x, out)),
        _ => Ok(()),
    }
}

/// Besprechungsbezuege eines Laufs: Kontext (Ausloeserdaten, Variablen, Ergebnisse der Schritte),
/// Ein- und Ausgaben jedes Schritts und die Quellen der Herkunft (jeder Baustein, der eine
/// Besprechung liest, nennt sie dort). Abgeleitete Ausgaben (ein spaeterer Schritt setzt den Text
/// einer Zusammenfassung ein) tragen keinen eigenen Bezug: darum gilt der Bezug fuer den GANZEN Lauf.
fn besprechungsbezug(d: &ui::WorkflowRunDetail) -> Bezug {
    let mut ids = BTreeSet::new();
    let mut texte: Vec<&str> = vec![&d.context_json];
    for s in &d.steps {
        texte.extend(s.input_json.as_deref());
        texte.extend(s.output_json.as_deref());
    }
    for text in texte {
        let ok = match serde_json::from_str::<Value>(text) {
            Ok(v) => besprechungen_in(&v, &mut ids).is_ok(),
            Err(_) => !text.to_ascii_lowercase().contains("meeting"),
        };
        if !ok {
            return Bezug::Unbestimmt;
        }
    }
    for e in &d.provenance {
        for q in e.sources.iter().filter(|q| q.kind == "meeting") {
            if q.reference.trim().is_empty() {
                return Bezug::Unbestimmt;
            }
            ids.insert(q.reference.trim().to_string());
        }
    }
    if ids.is_empty() {
        Bezug::Keiner
    } else {
        Bezug::Besprechungen(ids.into_iter().collect())
    }
}

/// Steht anstelle von Inhalten eines Laufs, der sich auf eine nicht freigegebene Besprechung bezieht.
const ZURUECKGEHALTEN: &str = "besprechung_nicht_freigegeben";

fn clip(text: &str) -> String {
    audit::sanitize_audit_text(text, MAX_LOG_TEXT_CHARS)
}

impl AppTools {
    fn engine(&self) -> Result<Engine, String> {
        self.host
            .workflow_engine()
            .ok_or_else(|| "Die Automationen sind in dieser Umgebung nicht verfügbar.".to_string())
    }

    pub(super) fn list_workflows(&self, a: Args) -> Result<Value, String> {
        a.done()?;
        let engine = self.engine()?;
        let items = ui::list(&engine).map_err(|e| wf_error(&e))?;
        let total = items.len();
        let workflows: Vec<Value> = items
            .iter()
            .take(MAX_WORKFLOWS_LISTED)
            .map(|w| {
                let (description, variables) =
                    match validate::parse_definition_str(&w.definition_json) {
                        Ok(def) => (
                            def.description
                                .as_deref()
                                .map(|d| audit::sanitize_audit_text(d, 200)),
                            def.variables
                                .iter()
                                .map(|(name, decl)| {
                                    json!({
                                        "name": name,
                                        "type": decl.ty.as_str(),
                                        "required": decl.default.is_none(),
                                    })
                                })
                                .collect::<Vec<_>>(),
                        ),
                        Err(_) => (None, Vec::new()),
                    };
                json!({
                    "id": w.id,
                    "name": clean_line(&w.name),
                    "description": description,
                    "enabled": w.enabled,
                    "armed": !w.dry_run,
                    "can_run_live": w.enabled && !w.dry_run && !is_automatic(&w.trigger_kind),
                    "trigger": w.trigger_kind,
                    "steps": w.step_count,
                    "variables": variables,
                    "open_runs": w.open_runs,
                    "last_run": w.last_run.as_ref().map(|r| json!({
                        "run_id": r.id,
                        "state": r.state,
                        "dry_run": r.dry_run,
                        "origin": r.origin,
                        "created_at": r.created_at,
                        "ended_at": r.ended_at,
                    })),
                })
            })
            .collect();
        Ok(json!({
            "workflows": workflows,
            "count": workflows.len(),
            "total": total,
            "hint": "run_workflow startet einen Ablauf (Standard: Trockenlauf). Echt laufen kann nur ein Ablauf mit enabled und armed.",
        }))
    }

    pub(super) fn run_workflow(&self, ctx: &CallContext, mut a: Args) -> Result<Value, String> {
        let id = a
            .id("workflow_id")?
            .ok_or_else(|| "„workflow_id“ fehlt.".to_string())?;
        let live = a.boolean("live")?.unwrap_or(false);
        let vars = a.object("vars", MAX_VARS_BYTES)?.unwrap_or_default();
        let request_id = a.id("request_id")?;
        a.done()?;
        let engine = self.engine()?;
        let item = ui::get(&engine, &id).map_err(|e| wf_error(&e))?;
        if live {
            // Sperre: nie schaerfer als der Ablauf. Die Engine rechnet es ebenfalls so
            // (`dry_run = wf.dry_run || force`), aber der Agent soll es ERFAHREN, nicht
            // stillschweigend einen Trockenlauf bekommen.
            if !item.enabled {
                return Err(format!(
                    "Der Ablauf „{}“ ist ausgeschaltet. Ein scharfer Lauf startet nicht. Ohne live=true läuft ein Trockenlauf.",
                    clean_line(&item.name)
                ));
            }
            if item.dry_run {
                return Err(format!(
                    "Der Ablauf „{}“ ist nicht scharf geschaltet; das kann nur der Nutzer in der App. Ohne live=true läuft ein Trockenlauf.",
                    clean_line(&item.name)
                ));
            }
            if is_automatic(&item.trigger_kind) {
                return Err(format!(
                    "Der Ablauf „{}“ startet durch ein Ereignis ({}) und braucht dessen Daten; ein Agent startet ihn nicht scharf. Der Nutzer kann den Auslöser auf „Durch einen Agenten starten“ stellen. Ohne live=true läuft ein Trockenlauf.",
                    clean_line(&item.name),
                    item.trigger_kind
                ));
            }
        }
        let started = manual::start_as(
            &engine,
            &id,
            vars,
            !live,
            Origin::Agent,
            Some(&ctx.client_id),
            request_id.as_deref(),
        )
        .map_err(|e| wf_error(&e))?;
        let hint = if started.dry_run {
            "Trockenlauf: es wird nur geplant, nichts ausgeführt. Das Protokoll liefert get_run."
        } else {
            "Der echte Lauf wurde eingereiht. Schritte mit Freigabe warten auf den Nutzer in der App. Das Protokoll liefert get_run."
        };
        Ok(json!({
            "run_id": started.run_id,
            "workflow_id": item.id,
            "workflow_name": clean_line(&item.name),
            "dry_run": started.dry_run,
            "created": started.created,
            "origin": "agent",
            "state": "queued",
            "hint": hint,
        }))
    }

    pub(super) fn get_run(&self, mut a: Args) -> Result<Value, String> {
        let run_id = a
            .id("run_id")?
            .ok_or_else(|| "„run_id“ fehlt.".to_string())?;
        a.done()?;
        let engine = self.engine()?;
        let d = ui::run_detail(&engine, &run_id).map_err(|e| wf_error(&e))?;
        // G9a-2 (Codex #583 Fixrunde 3): Ausgaben, Fehler- und Wartetexte eines Laufs mit Bezug zu
        // einer Besprechung gehen nur hinaus, wenn ALLE seine Besprechungen freigegeben sind
        // (gleiche Pruefung und Linearisierung wie die Besprechungs-Werkzeuge).
        let zeigen = match besprechungsbezug(&d) {
            Bezug::Keiner => true,
            Bezug::Besprechungen(ids) => self.besprechungen_freigegeben(&ids),
            Bezug::Unbestimmt => false,
        };
        let inhalt = |v: Value| if zeigen { v } else { json!(ZURUECKGEHALTEN) };
        let total_steps = d.steps.len();
        let steps: Vec<Value> = d
            .steps
            .iter()
            .take(MAX_STEPS_LISTED)
            .map(|s| {
                json!({
                    "step_id": s.step_id,
                    "attempt": s.attempt,
                    "action": s.action,
                    "title": s.action_title,
                    "state": s.state,
                    "error_class": s.error_class,
                    "error": s.error.as_deref().map(|e| inhalt(json!(clip(e)))),
                    "waits_for_approval": s.approval_id.is_some(),
                    "started_at": s.started_at,
                    "ended_at": s.ended_at,
                    "output": inhalt(step_output(s.output_json.as_deref())),
                })
            })
            .collect();
        let r = &d.run;
        Ok(json!({
            "run": {
                "id": r.id,
                "workflow_id": r.workflow_id,
                "workflow_name": clean_line(&r.workflow_name),
                "origin": r.origin,
                "state": r.state,
                "dry_run": r.dry_run,
                "created_at": r.created_at,
                "started_at": r.started_at,
                "ended_at": r.ended_at,
                "error": r.error.as_deref().map(|e| inhalt(json!(clip(e)))),
                "error_code": r.error_code,
                "wait_reason": r.wait_reason.as_deref().map(|w| inhalt(json!(clip(w)))),
                "cancel_requested": r.cancel_requested,
            },
            "steps": steps,
            "steps_total": total_steps,
            "withheld": !zeigen,
            "finished": matches!(r.state, crate::managers::workflows::model::RunState::Done
                | crate::managers::workflows::model::RunState::Failed
                | crate::managers::workflows::model::RunState::Cancelled),
        }))
    }
}

#[cfg(test)]
mod tests;
