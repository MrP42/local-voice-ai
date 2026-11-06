//! Die lesenden Werkzeuge der Agentenbruecke fuer den Orchestrator des Sprachgespraechs
//! (Stufe 2, Tauri-Befehle `voice_tools_list` und `voice_tool_call` in `commands::voice_tools`).
//!
//! Der Orchestrator im Frontend (`src/components/voice-call/orchestration.ts`) bietet dem
//! Sprachmodell Werkzeuge an. Quellen: die lesenden Werkzeuge des Katalogs (`catalog::CATALOG`;
//! ihre Faehigkeit hat `Access::Read`, das ist auch die Annotation `readOnlyHint` in `tools/list`)
//! und die Wissenswerkzeuge in [`knowledge`] (nur fuer das Sprachgespraech). Alles andere fehlt in
//! der Liste und wird beim Aufruf abgelehnt, auch wenn ein Modell den Namen errät.
//!
//! # Freigabe: Wissensbereiche statt Rechte der Agentenbruecke
//! Was das Sprachgespraech nachschlagen darf, bestimmen allein die Bereiche des Nutzers
//! (Einstellung `voice_call_knowledge`, Standard leer; `areas`). Das ist getrennt von den Rechten
//! externer Agenten und des lokalen Agenten an der Agentenbruecke: ein Recht „aus“ dort sperrt das
//! Gespraech nicht mehr, ein freigegebener Bereich oeffnet keinen Weg fuer externe Agenten. Fruehere
//! Fassung: die Liste folgte `Caller::AgentLocal` und war ohne ausdrueckliches Recht leer.
//!
//! | Bereich | Werkzeuge |
//! |---|---|
//! | Besprechungen | `meetings_list`, `meetings_search` (Katalog der Bruecke) |
//! | Transkripte | `meeting_transcript` (Katalog); Unterschalter `voice_call_read_transcripts`, nur mit Besprechungen |
//! | Diktate, fruehere Gespraeche, Termine, Vorlesen-Seiten, Tagesueberblick | `knowledge` (nur Sprachgespraech) |
//! | Vorlesen (Anbieter) | die lesenden `tts_*`-Werkzeuge des Katalogs, im Bereich „Vorlesen-Seiten“ |
//!
//! Lesende Katalogwerkzeuge ohne Bereich (`list_workflows`, `get_run`) bietet das Gespraech nicht an.
//!
//! # Weg durch das Tor
//! Jeder Aufruf schreibt ein Audit ueber das Tor des Registers (`integrations::gate::run_authorized`):
//! „laeuft“-Eintrag VOR der Aktion (fehlgeschlagen = nichts wird ausgefuehrt), dann das Ergebnis. Die
//! Rechtepruefung des Tors entfaellt, weil die Bereiche sie ersetzen. Aufrufer im Audit ist
//! `Caller::AgentLocal`, das Ziel nennt `<werkzeug> · voice_orchestrator`; Argumente und Inhalte
//! stehen nie darin. Die Faehigkeit im Audit ist die des Katalogs bzw. `knowledge.read`,
//! `calendar.read` und `tts.read` fuer die Wissenswerkzeuge.
//!
//! # Abweichung vom Entwurf: kein neuer `Caller`
//! Ein eigener Aufrufer `voice_orchestrator` braeuchte einen neuen Wert in `Caller` und eine
//! Migration der CHECK-Bedingungen (`integration_grants`, `provenance`), dazu Beschriftungen in der
//! Oberflaeche. Der vorhandene `AgentLocal` ist fuer lokale Agenten der App gedacht und laesst sich im
//! Protokoll weiter filtern; die Herkunft Sprachgespraech steht im Ziel des Audits.
//!
//! # Weiteres
//! - Antworten werden nie still gekuerzt: ist eine Antwort laenger als [`MAX_RESULT_CHARS`], kommt eine
//!   Meldung („kleinere Seite oder engere Suche“). Transkriptseiten sind fuer die Stimme auf
//!   [`VOICE_PAGE_CHARS`] Zeichen begrenzt, damit `next_cursor` stimmt.
//! - Hoechstens [`RateLimiter::MAX_CALLS`] Aufrufe je Minute (Schutz vor einer Schleife des Modells).
//! - Das laufende Gespraech (`conversation`) steht den Gespraechswerkzeugen nicht offen.
//!
//! # Fehlerfaelle und ihre Absicherung (Tests in `voice/tests.rs`, `voice/knowledge/tests.rs`)
//!
//! | # | Fehlerfall | Verhalten | Absicherung |
//! |---|---|---|---|
//! | 1 | Das Modell ruft ein schreibendes Werkzeug auf (`transcribe_file`, `start_recording`, `run_workflow`, ...) | abgelehnt, nichts ausgefuehrt, kein Audit-Eintrag eines Werkzeugs, Handler wird nie gerufen | `writing_tools_are_never_listed_or_run` |
//! | 2 | Unbekannter oder verschleierter Name (Gross-/Kleinschreibung, Pfad, Leerzeichen) | abgelehnt wie ein unbekanntes Werkzeug | `unknown_and_odd_names_are_refused` |
//! | 3 | Bereich nicht freigegeben | Werkzeug fehlt in der Liste; Aufruf scheitert mit Hinweis auf „Gesprächseinstellungen → Wissen“, nichts gelesen, kein Audit | `the_list_follows_the_areas`, `a_tool_outside_its_area_runs_nothing` |
//! | 4 | Audit nicht schreibbar (Platte voll, Datenbank gesperrt) | Tor schlaegt fehlgeschlossen: Werkzeug laeuft nicht | `gate` (A1), `a_failing_audit_blocks_the_call` |
//! | 5 | Werkzeug stuerzt ab (Panik) | `run_handler` faengt sie: Fehlertext, Audit `error` | `bridge::tests` |
//! | 6 | Argumente: kein Objekt, kein JSON, zu gross | Fehler vor dem Tor, nichts gelesen | `arguments_must_be_a_small_object` |
//! | 7 | Riesige Antwort | abgelehnt mit Meldung (nie still gekuerzt) | `long_results_are_refused_not_cut` |
//! | 8 | Audit zeigt Inhalte | nein: Ziel `<werkzeug> · voice_orchestrator`, Detail nur Phase/Ergebnis | `every_call_is_audited_without_content` |
//! | 9 | Rechte der Agentenbruecke auf „aus“ oder „fragen“ | beeinflusst das Gespraech nicht (nur die Bereiche) | `the_agent_rights_do_not_decide_for_the_call` |
//!
//! Audio-Echtzeitpfad, Kindprozesse und Arbeitsspeicher: nicht beruehrt (die Werkzeuge lesen die
//! Datenbank; der Aufruf laeuft auf einem Blockier-Thread, nie im Audio-Callback).

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::bridge::{run_handler, Bridge};
use super::catalog::{self, CallContext, CatalogEntry, ToolRegistry, STATUS_TOOL};
use super::clients;
use crate::managers::integrations::gate::{self, GateOutcome, Request};
use crate::managers::integrations::model::{Access, Caller, Capability, IntegrationError};

pub mod areas;
pub mod knowledge;

use areas::{Area, Areas};
use knowledge::{KnowledgeSource, Need};

/// Aufrufer im Ziel des Audit-Eintrags.
pub const CALLER_TAG: &str = "voice_orchestrator";
/// Name des Zugangs im `CallContext` (nur Anzeige; es gibt keinen echten Zugang).
const CALLER_LABEL: &str = "Sprachgespräch";
/// Laengste Antwort an das Sprachmodell (Zeichen). Der Orchestrator kuerzt selbst bei 8000: darunter bleiben.
pub const MAX_RESULT_CHARS: usize = 8_000;
/// Zeichen je Transkriptseite fuer die Stimme (eine Seite samt JSON bleibt unter [`MAX_RESULT_CHARS`]).
pub const VOICE_PAGE_CHARS: usize = 4_000;

/// Einstellungen des Nutzers, die die Liste und den Aufruf beeinflussen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoiceOptions {
    /// Die freigegebenen Wissensbereiche (Standard: keiner).
    pub areas: Areas,
}

/// Begrenzt die Aufrufe je Minute (gleitendes Fenster); rein, mit uebergebener Uhr.
#[derive(Debug, Default)]
pub struct RateLimiter {
    calls: std::collections::VecDeque<i64>,
}

impl RateLimiter {
    pub const MAX_CALLS: usize = 8;
    const WINDOW_MS: i64 = 60_000;

    /// Zaehlt einen Aufruf; `false`, wenn die Grenze der letzten Minute erreicht ist.
    pub fn allow(&mut self, now_ms: i64) -> bool {
        while self
            .calls
            .front()
            .is_some_and(|t| now_ms - *t >= Self::WINDOW_MS || *t > now_ms)
        {
            self.calls.pop_front();
        }
        if self.calls.len() >= Self::MAX_CALLS {
            return false;
        }
        self.calls.push_back(now_ms);
        true
    }
}
/// Groesste Argumentzeichenkette (Bytes).
pub const MAX_ARGS_BYTES: usize = 16 * 1024;

/// Ein Werkzeug, wie das Frontend es dem Sprachmodell anbietet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct VoiceToolInfo {
    pub name: String,
    pub title: String,
    pub description: String,
    /// Schema der Argumente (JSON-Text; specta kennt kein freies JSON).
    pub parameters_json: String,
    /// Fähigkeit des Katalogs, z. B. `meeting.read`.
    pub capability: String,
}

/// Ob ein Katalogeintrag nur liest (Annotation `readOnlyHint`, folgt aus der Faehigkeit).
pub fn is_read_only(entry: &CatalogEntry) -> bool {
    entry.capability.access() == Access::Read
}

fn store_error(e: &IntegrationError) -> String {
    log::warn!("voice_tools: Speicherfehler: {e}");
    "Die Datenbank ist gerade nicht verfügbar. Es wurde nichts ausgeführt.".to_string()
}

/// Das lesende Werkzeug zum Namen: nur der Katalog entscheidet, nie der Name des Modells.
fn read_only_entry(name: &str) -> Result<&'static CatalogEntry, String> {
    if name == STATUS_TOOL || !catalog::valid_tool_name(name) {
        return Err(format!("Das Werkzeug „{}“ gibt es nicht.", shown(name)));
    }
    let entry = catalog::find(name)
        .ok_or_else(|| format!("Das Werkzeug „{}“ gibt es nicht.", shown(name)))?;
    if !is_read_only(entry) {
        return Err(
            "Dieses Werkzeug ändert Daten und steht im Sprachgespräch nicht zur Verfügung (nur lesende Werkzeuge)."
                .to_string(),
        );
    }
    Ok(entry)
}

/// Name fuer Meldungen: begrenzt, ohne Steuerzeichen.
fn shown(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>()
}

/// Der Bereich eines lesenden Katalogwerkzeugs; `None`: das Gespraech bietet es nicht an.
fn area_of_catalog(entry: &CatalogEntry) -> Option<Area> {
    match entry.capability {
        Capability::MeetingRead if entry.name == "meeting_transcript" => Some(Area::Transcripts),
        Capability::MeetingRead => Some(Area::Meetings),
        Capability::TtsRead => Some(Area::Tts),
        _ => None,
    }
}

/// Faehigkeit im Audit fuer ein Wissenswerkzeug.
fn knowledge_capability(name: &str) -> Capability {
    match name {
        "calendar_events" => Capability::CalendarRead,
        "tts_pages_list" | "tts_page_text" => Capability::TtsRead,
        _ => Capability::KnowledgeRead,
    }
}

/// Die lesenden Werkzeuge, die das Sprachgespraech jetzt benutzen darf: genau die der
/// freigegebenen Bereiche.
pub fn list(registry: &ToolRegistry, opts: VoiceOptions) -> Vec<VoiceToolInfo> {
    let mut out = Vec::new();
    for name in registry.names() {
        let (Some(entry), Some(spec)) = (catalog::find(&name), registry.spec(&name)) else {
            continue;
        };
        let allowed =
            is_read_only(entry) && area_of_catalog(entry).is_some_and(|area| opts.areas.has(area));
        if !allowed {
            continue;
        }
        out.push(VoiceToolInfo {
            name: spec.name.clone(),
            title: spec.title.clone(),
            description: spec.description.clone(),
            parameters_json: spec.input_schema.to_string(),
            capability: entry.capability.as_str().to_string(),
        });
    }
    for spec in knowledge::specs() {
        if !spec.need.met(opts.areas) {
            continue;
        }
        out.push(VoiceToolInfo {
            name: spec.name.to_string(),
            title: spec.title.to_string(),
            description: spec.description.to_string(),
            parameters_json: spec.parameters.to_string(),
            capability: knowledge_capability(spec.name).as_str().to_string(),
        });
    }
    out
}

/// Argumente aus dem Text des Frontends: ein kleines JSON-Objekt (leer gilt als `{}`).
fn parse_args(raw: &str) -> Result<Value, String> {
    if raw.len() > MAX_ARGS_BYTES {
        return Err("Die Argumente sind zu groß.".to_string());
    }
    if raw.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| "Die Argumente sind kein gültiges JSON.".to_string())?;
    Bridge::normalize_args(&value).map_err(|e| e.message)
}

/// Laesst eine Antwort bis `max` Zeichen durch; eine laengere wird NIE still gekuerzt (das Modell
/// wuerde auf abgeschnittenen Daten antworten), sondern durch eine Meldung ersetzt.
pub fn clamp_result(text: String, max: usize) -> Result<String, String> {
    if text.chars().nth(max).is_none() {
        return Ok(text);
    }
    Err("Die Antwort ist zu lang für das Gespräch. Bitte eine kleinere Seite oder eine engere Suche wählen (weniger Treffer, kürzerer Zeitraum).".to_string())
}

/// Was ein Aufruf ausfuehrt: ein Katalogwerkzeug der Bruecke oder ein Wissenswerkzeug.
enum Target {
    Bridge(&'static CatalogEntry),
    Knowledge(knowledge::Spec),
}

fn resolve(name: &str) -> Result<Target, String> {
    if name == STATUS_TOOL || !catalog::valid_tool_name(name) {
        return Err(format!("Das Werkzeug „{}“ gibt es nicht.", shown(name)));
    }
    if let Some(spec) = knowledge::find(name) {
        return Ok(Target::Knowledge(spec));
    }
    Ok(Target::Bridge(read_only_entry(name)?))
}

/// Womit ein Aufruf arbeitet: Werkzeuge der Bruecke, Datenquelle der Wissenswerkzeuge und das
/// laufende Gespraech (wird den Gespraechswerkzeugen vorenthalten).
pub struct Session<'a> {
    pub registry: &'a ToolRegistry,
    pub source: &'a dyn KnowledgeSource,
    pub conversation: Option<&'a str>,
}

/// Fuehrt ein lesendes Werkzeug aus, wenn sein Bereich frei ist; jeder Aufruf steht im Audit.
pub fn call(
    conn: &Connection,
    session: &Session<'_>,
    opts: VoiceOptions,
    name: &str,
    args_json: &str,
    now_ms: i64,
) -> Result<String, String> {
    let target = resolve(name)?;
    let (tool, need, capability) = match &target {
        Target::Bridge(entry) => {
            let area = area_of_catalog(entry).ok_or_else(|| {
                "Dieses Werkzeug steht im Sprachgespräch nicht zur Verfügung.".to_string()
            })?;
            (entry.name, Need::One(area), entry.capability)
        }
        Target::Knowledge(spec) => (spec.name, spec.need, knowledge_capability(spec.name)),
    };
    if !need.met(opts.areas) {
        return Err(knowledge::not_shared(need));
    }
    let args = parse_args(args_json)?;
    let handler = match &target {
        Target::Bridge(entry) => Some(session.registry.handler(entry.name).ok_or_else(|| {
            format!(
                "Das Werkzeug „{}“ ist in dieser Version nicht verfügbar.",
                entry.name
            )
        })?),
        Target::Knowledge(_) => None,
    };
    let integration =
        clients::ensure_agent_integration(conn, now_ms).map_err(|e| store_error(&e))?;
    let audit_target = format!("{tool} · {CALLER_TAG}");
    let request = Request {
        caller: Caller::AgentLocal,
        integration_id: &integration.id,
        capability,
        target: Some(&audit_target),
        args: Some(&args),
        tool_mode: None,
    };
    let ctx = CallContext {
        client_id: CALLER_TAG.to_string(),
        client_label: CALLER_LABEL.to_string(),
        approved: false,
        now_ms,
    };
    let kctx = knowledge::Context {
        areas: opts.areas,
        current_conversation: session.conversation,
        now_ms,
    };
    let outcome = gate::run_authorized(conn, &request, now_ms, || match &handler {
        Some(handler) => run_handler(handler.as_ref(), &ctx, tool, &args),
        None => knowledge::run(session.source, &kctx, tool, &args),
    })
    .map_err(|e| store_error(&e))?;
    match outcome {
        GateOutcome::Done(value) => clamp_result(value.to_string(), MAX_RESULT_CHARS),
        GateOutcome::Failed(message) => Err(message),
        GateOutcome::Denied { reason, .. } => Err(reason),
        GateOutcome::Pending { .. } => Err("Der Zugriff wurde nicht ausgeführt.".to_string()),
    }
}

#[cfg(test)]
mod tests;
