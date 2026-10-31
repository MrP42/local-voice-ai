//! Die lesenden Werkzeuge der Agentenbruecke fuer den Orchestrator des Sprachgespraechs
//! (Stufe 2, Tauri-Befehle `voice_tools_list` und `voice_tool_call` in `commands::voice_tools`).
//!
//! Der Orchestrator im Frontend (`src/components/voice-call/orchestration.ts`) bietet dem
//! Sprachmodell Werkzeuge an. Quelle sind genau die Werkzeuge des Katalogs (`catalog::CATALOG`),
//! die NUR LESEN: ihre Faehigkeit hat `Access::Read` (das ist auch die Annotation `readOnlyHint`
//! in `tools/list`). Alles andere fehlt in der Liste und wird beim Aufruf abgelehnt, auch wenn
//! ein Modell den Namen errät.
//!
//! # Weg durch das Tor
//! Jeder Aufruf geht durch das Tor des Registers (`integrations::gate`), wie bei externen Agenten:
//! Recht pruefen, Audit schreiben (fehlgeschlagen = nichts wird ausgefuehrt), ausfuehren,
//! Ergebnis festhalten. Aufrufer ist `Caller::AgentLocal` (der lokale Agent des Registers, Vorgabe:
//! Lesendes erlaubt, Schreibendes fragen) an der Agent-Integration. Der Audit-Eintrag nennt das Ziel
//! `<werkzeug> · voice_orchestrator`; Argumente und Inhalte stehen nie darin.
//!
//! # Abweichung vom Entwurf: kein neuer `Caller`
//! Ein eigener Aufrufer `voice_orchestrator` braeuchte einen neuen Wert in `Caller` und eine
//! Migration der CHECK-Bedingungen (`integration_grants`, `provenance`). Der vorhandene `AgentLocal`
//! ist genau fuer lokale Agenten der App gedacht; die Herkunft steht im Ziel des Audits.
//!
//! # Rechte
//! - Die Liste enthaelt nur Werkzeuge, deren wirksames Recht fuer `AgentLocal` „erlaubt“ ist. Ein
//!   Recht „fragen“ oder „aus“ (Integration abgeschaltet, Fähigkeit nicht eingeschaltet, Richtung)
//!   nimmt das Werkzeug aus der Liste. Im Gespraech laesst sich nicht zurueckfragen.
//! - Der Aufruf prueft das Recht VOR dem Tor (`explain`): nur bei „erlaubt“ geht er ins Tor, sonst
//!   wird sofort abgelehnt, ohne eine Freigabe anzulegen und ohne etwas auszufuehren.
//! - Antworten werden nie still gekuerzt: ist eine Antwort laenger als [`MAX_RESULT_CHARS`], kommt eine
//!   Meldung („kleinere Seite oder engere Suche“). Transkriptseiten sind fuer die Stimme auf
//!   [`VOICE_PAGE_CHARS`] Zeichen begrenzt, damit `next_cursor` stimmt.
//! - Hoechstens [`RateLimiter::MAX_CALLS`] Aufrufe je Minute (Schutz vor einer Schleife des Modells).
//!
//! # Transkripte
//! Das Transkript liefert fuer das Sprachgespraech NUR die eigene Einstellung „Transkripte im
//! Sprachgespraech lesen“ (`voice_call_read_transcripts`, Standard aus, ADR-0021). `meeting_transcript`
//! fehlt ohne sie in der Liste und wird abgelehnt; die Suche zeigt keine Treffer aus dem Transkript.
//! Fuer externe Agenten ueber die Bruecke gilt davon getrennt `meeting_mcp_enabled &&
//! meeting_mcp_include_transcript`; `meeting.read` steht ihnen nur mit erteiltem Recht offen
//! (Vorgabe: „aus“).
//!
//! # Fehlerfaelle und ihre Absicherung (Tests in `voice/tests.rs`)
//!
//! | # | Fehlerfall | Verhalten | Absicherung |
//! |---|---|---|---|
//! | 1 | Das Modell ruft ein schreibendes Werkzeug auf (`transcribe_file`, `start_recording`, `run_workflow`, ...) | abgelehnt, nichts ausgefuehrt, kein Audit-Eintrag eines Werkzeugs, Handler wird nie gerufen | `writing_tools_are_never_listed_or_run` |
//! | 2 | Unbekannter oder verschleierter Name (Gross-/Kleinschreibung, Pfad, Leerzeichen) | abgelehnt wie ein unbekanntes Werkzeug | `unknown_and_odd_names_are_refused` |
//! | 3 | Recht auf „aus“/„fragen“ | Werkzeug fehlt in der Liste; Aufruf scheitert; bei „fragen“ liegt eine Freigabe vor, ausgefuehrt wird nichts | `the_list_follows_the_rights`, `a_right_of_ask_runs_nothing` |
//! | 4 | Audit nicht schreibbar (Platte voll, Datenbank gesperrt) | Tor schlaegt fehlgeschlossen: Werkzeug laeuft nicht | `gate` (A1) |
//! | 5 | Werkzeug stuerzt ab (Panik) | `run_handler` faengt sie: Fehlertext, Audit `error` | `bridge::tests` |
//! | 6 | Argumente: kein Objekt, kein JSON, zu gross | Fehler vor dem Tor, nichts gelesen | `arguments_must_be_a_small_object` |
//! | 7 | Riesige Antwort | auf 6000 Zeichen gekuerzt, an einer Zeichengrenze | `long_results_are_cut_at_a_character_boundary` |
//! | 8 | Audit zeigt Inhalte | nein: Ziel `<werkzeug> · voice_orchestrator`, Detail nur Phase/Ergebnis | `every_call_is_audited_without_content` |
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
use crate::managers::integrations::grants::explain;
use crate::managers::integrations::model::{Access, Caller, GrantMode, IntegrationError};
use crate::managers::integrations::store;

/// Aufrufer im Ziel des Audit-Eintrags.
pub const CALLER_TAG: &str = "voice_orchestrator";
/// Name des Zugangs im `CallContext` (nur Anzeige; es gibt keinen echten Zugang).
const CALLER_LABEL: &str = "Sprachgespräch";
/// Laengste Antwort an das Sprachmodell (Zeichen). Der Orchestrator kuerzt selbst bei 8000: darunter bleiben.
pub const MAX_RESULT_CHARS: usize = 8_000;
/// Zeichen je Transkriptseite fuer die Stimme (eine Seite samt JSON bleibt unter [`MAX_RESULT_CHARS`]).
pub const VOICE_PAGE_CHARS: usize = 4_000;
/// Werkzeuge, die den Inhalt von Transkripten lesen (brauchen die Freigabe im Sprachgespraech).
const NEEDS_TRANSCRIPT: [&str; 1] = ["meeting_transcript"];

/// Einstellungen des Nutzers, die die Liste und den Aufruf beeinflussen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoiceOptions {
    /// „Transkripte im Sprachgespraech lesen“ (Standard aus).
    pub transcripts: bool,
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

/// Die lesenden Werkzeuge, die das Sprachgespraech jetzt benutzen darf (Recht „erlaubt“).
pub fn list(
    conn: &Connection,
    registry: &ToolRegistry,
    opts: VoiceOptions,
    now_ms: i64,
) -> Result<Vec<VoiceToolInfo>, String> {
    let integration =
        clients::ensure_agent_integration(conn, now_ms).map_err(|e| store_error(&e))?;
    let grants = store::grants_for(conn, &integration.id).map_err(|e| store_error(&e))?;
    let mut out = Vec::new();
    for name in registry.names() {
        let (Some(entry), Some(spec)) = (catalog::find(&name), registry.spec(&name)) else {
            continue;
        };
        if !is_read_only(entry) || (!opts.transcripts && NEEDS_TRANSCRIPT.contains(&entry.name)) {
            continue;
        }
        let (mode, _) = explain(
            &integration,
            entry.capability,
            Caller::AgentLocal,
            &grants,
            None,
        );
        if mode != GrantMode::Allow {
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
    Ok(out)
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

/// Fuehrt ein lesendes Werkzeug durch das Tor aus; die Antwort ist der JSON-Text des Ergebnisses.
pub fn call(
    conn: &Connection,
    registry: &ToolRegistry,
    opts: VoiceOptions,
    name: &str,
    args_json: &str,
    now_ms: i64,
) -> Result<String, String> {
    let entry = read_only_entry(name)?;
    if !opts.transcripts && NEEDS_TRANSCRIPT.contains(&entry.name) {
        return Err("Transkripte darf die Sprach-KI nur lesen, wenn „Transkripte im Sprachgespräch lesen“ eingeschaltet ist.".to_string());
    }
    let args = parse_args(args_json)?;
    let handler = registry.handler(entry.name).ok_or_else(|| {
        format!(
            "Das Werkzeug „{}“ ist in dieser Version nicht verfügbar.",
            entry.name
        )
    })?;
    let integration =
        clients::ensure_agent_integration(conn, now_ms).map_err(|e| store_error(&e))?;
    let grants = store::grants_for(conn, &integration.id).map_err(|e| store_error(&e))?;
    let (mode, _) = explain(
        &integration,
        entry.capability,
        Caller::AgentLocal,
        &grants,
        None,
    );
    match mode {
        GrantMode::Allow => {}
        GrantMode::Ask => {
            return Err("Dieses Werkzeug braucht eine Freigabe in der App (Integrationen). Im Sprachgespräch ist keine Rückfrage möglich, es wurde nichts ausgeführt.".to_string())
        }
        GrantMode::Off => {
            return Err("Dieses Werkzeug ist für das Sprachgespräch ausgeschaltet (Integrationen). Es wurde nichts ausgeführt.".to_string())
        }
    }
    let target = format!("{} · {CALLER_TAG}", entry.name);
    let request = Request {
        caller: Caller::AgentLocal,
        integration_id: &integration.id,
        capability: entry.capability,
        target: Some(&target),
        args: Some(&args),
        tool_mode: None,
    };
    let ctx = CallContext {
        client_id: CALLER_TAG.to_string(),
        client_label: CALLER_LABEL.to_string(),
        approved: false,
        now_ms,
    };
    let outcome = gate::run(conn, &request, now_ms, || {
        run_handler(handler.as_ref(), &ctx, entry.name, &args)
    })
    .map_err(|e| store_error(&e))?;
    match outcome {
        GateOutcome::Done(value) => clamp_result(value.to_string(), MAX_RESULT_CHARS),
        GateOutcome::Failed(message) => Err(message),
        GateOutcome::Denied { reason, .. } => Err(reason),
        GateOutcome::Pending { .. } => Err(
            "Dieses Werkzeug braucht eine Freigabe in der App (Integrationen). Im Sprachgespräch ist keine Rückfrage möglich, es wurde nichts ausgeführt."
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests;
