//! Bausteine des lokalen Agenten (Goal Lokaler Agent, Issue #68, Paket C2): `agent.extract`.
//!
//! Der Baustein zieht To-dos, Fristen und Entscheidungen aus dem Transkript der Besprechung
//! des Laufs (`agent::extract`). Er hat **keine Aussenwirkung und braucht kein Recht**: er
//! liest die eigene Besprechung und liefert Daten (`steps.<id>.todos`, `.deadlines`,
//! `.decisions`, `.dropped`, `.notes`, `.provenance`). Was daraus entsteht (Vault-Notiz,
//! Mitteilung, Kalender), tun spaetere, deterministische Bausteine mit ihren eigenen Rechten
//! (C3 bis C5).
//!
//! | Baustein        | Wirkung | Schwer   | Recht (Tor) |
//! |-----------------|---------|----------|-------------|
//! | `agent.extract` | `Pure`  | Modell   | keines      |
//!
//! `Pure`, weil nur gelesen wird: nach einem Absturz mitten im Schritt wiederholt die Engine
//! ihn selbst (ein zweiter Modellaufruf, kein zweites Ergebnis). Die Provenienz des Schritts
//! (Modell, Token, Dauer, Quellen, Konfidenz) wird nur geschrieben, wenn es fuer genau diesen
//! Schritt noch keinen Eintrag gibt (`previous_result`).
//!
//! # Fehlerklassen
//!
//! | Lage                                              | Ergebnis des Schritts            |
//! |---------------------------------------------------|----------------------------------|
//! | Besprechung noch nicht fertig                     | `Defer` (15 s, wie B4)           |
//! | RAM knapp, Server belegt, Neustart gesperrt       | `Defer` (Rueckstau, kein Versuch)|
//! | Server nicht erreichbar / bricht ab, Zeit         | `Transient` (nichts geschrieben) |
//! | Modell nicht eingerichtet, Anbieter nicht lokal,  | `Permanent`                      |
//! | Transkript leer, Anfrage abgelehnt                |                                  |
//! | Antwort auch im 2. Versuch kein gueltiges JSON    | **Erfolg** mit `outcome: no_action` und Grund |
//! | Nutzer bricht ab                                  | `Transient` (Anfrage faellt)     |
//!
//! # Fehlerfaelle (C2) und ihre Absicherung
//!
//! - **Nebenlaeufigkeit**: das `HeavyGate` haelt schwere Schritte seriell; zwei Laeufe auf
//!   derselben Besprechung lesen nur (`agent_actions::tests::two_runs_*`). Der Server selbst
//!   (`--parallel 1`) serialisiert Anfragen; die Zeitgrenze je Anfrage verhindert, dass ein
//!   belegter Slot den schweren Platz unbegrenzt haelt.
//! - **Abbruch mitten im Vorgang**: Nutzerabbruch beendet die Anfrage (`run_cancellable`);
//!   ein Absturz der App hinterlaesst nichts ausser dem Journal (`running`), die Engine
//!   wiederholt den `Pure`-Schritt; die Provenienz entsteht hoechstens einmal je Schritt
//!   (`a_second_attempt_*`).
//! - **Voller Arbeitsspeicher**: der Server startet nur ueber den Modellverwalter (RAM-Start-
//!   Tor, `process_guard`-Job-Objekt); `memory_low` wird `Defer` (`server_trouble_*`). Das
//!   `HeavyGate` prueft vorher 6 GB frei.
//! - **Voller Datentraeger / gesperrte Datenbank**: die Provenienz-Schreibung darf den Schritt
//!   nie scheitern lassen (`a_broken_provenance_table_*`); die Ausgabe ist auf das Hoechstmass
//!   der Engine gekuerzt (`agent::extract::fit_json`).
//! - **Absturz des Kindprozesses (llama-server)**: Verbindungsabbruch -> `Transient`;
//!   `server_crashed:` vom Verwalter -> `Defer` (Neustart ist eine Minute gesperrt).
//! - **Fehlendes Geraet**: kein Audio beteiligt. Fehlt die GPU, entscheidet der Verwalter
//!   (CPU-Rueckfall oder `Unavailable`).

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::agent::extract::{
    self, fit_json, ExtractOptions, Kinds, NoActionReason, Outcome, Source,
};
use crate::agent::runtime::{AgentError, AgentRuntime};
use crate::managers::meetings::speakers::SpeakerDirectory;
use crate::managers::meetings::store::StoredSegment;
use crate::managers::provenance::{Locality, NewProvenance, SourceRef, SubjectKind};

use super::action::{
    Action, EffectKind, HeavyNeed, Needs, NeedsError, RunCtx, StepError, StepOutput,
    MAX_OUTPUT_BYTES,
};
use super::app_actions::{
    llm_need, meeting_id_of, meeting_source, no_meeting, previous_result, ready_meeting,
    run_cancellable, spec_of, svc, AppServices,
};
use super::catalog;
use super::heavy::HeavyGate;

/// Reserve unter dem Hoechstmass der Engine fuer `status`, `ok` und `error`.
const OUTPUT_MARGIN_BYTES: usize = 4 * 1024;
/// Hoechstzahl Segment-Quellen im Provenienz-Eintrag (die Engine fuegt einen Ausloeser hinzu).
const MAX_SEGMENT_SOURCES: usize = 150;
const OPERATION: &str = "agent_extract";

fn kinds_of(params: &Value) -> Result<Kinds, String> {
    match params.get("kinds") {
        None | Some(Value::Null) => Ok(Kinds::ALL),
        Some(Value::Array(items)) => {
            let names: Vec<String> = items
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "kinds: Liste aus Texten erwartet".to_string())
                })
                .collect::<Result<_, _>>()?;
            Kinds::parse(&names)
        }
        Some(_) => Err("kinds: Liste aus Texten erwartet".to_string()),
    }
}

/// Was kein Ergebnis ist, aber auch kein Schritt-Erfolg: Server, Speicher, Einrichtung.
/// `None`: die Extraktion zaehlt (auch ein `no_action` mit unbrauchbarer Antwort).
fn blocking_error(outcome: &Outcome) -> Option<StepError> {
    let Outcome::NoAction(reason) = outcome else {
        return None;
    };
    match reason {
        NoActionReason::EmptyTranscript => Some(StepError::Permanent(
            "Die Besprechung hat kein Transkript, aus dem sich etwas ziehen ließe.".to_string(),
        )),
        NoActionReason::Failed(error) => match error {
            AgentError::NotConfigured(m) => Some(StepError::Permanent(m.clone())),
            AgentError::Rejected { .. } => Some(StepError::Permanent(error.describe())),
            AgentError::Busy {
                retry_after_ms,
                reason,
            } => Some(StepError::Defer {
                retry_after_ms: *retry_after_ms,
                reason: reason.clone(),
            }),
            // Server weg oder Zeit: nichts geschrieben, ein neuer Versuch ist sicher.
            _ if reason.is_retryable() => Some(StepError::Transient(error.describe())),
            // Eine unbrauchbare Antwort ist ein gueltiges `no_action`.
            _ => None,
        },
    }
}

/// Laengster Beispieltext der Vorschau (Zeichen) und Zahl der Zeilen (je Zeile ein Segment).
pub const MAX_SAMPLE_CHARS: usize = 20_000;
const MAX_SAMPLE_SEGMENTS: usize = 400;

/// Vorschau (C5): was wuerde `agent.extract` aus diesem Beispieltext ziehen? Dieselbe
/// Extraktion wie im Lauf, aber ohne Besprechung und ohne jede Wirkung: kein Journal, keine
/// Provenienz, kein Vault, keine Mitteilung. Je nicht leerer Zeile ein Segment (`S0`, `S1`, ...),
/// Bezugsdatum ist heute. Der Aufruf nimmt den Platz am `HeavyGate` (belegt: `Defer`, das Modell
/// wird nicht gefragt). Ohne Beispieltext keine Entscheidung (der Schritt liest das Transkript
/// der Besprechung des Laufs, die es im Editor nicht gibt).
pub fn preview_extract(
    services: &dyn AppServices,
    gate: &dyn HeavyGate,
    params: &Value,
    sample_text: Option<&str>,
    now_ms: i64,
    cancel: &dyn Fn() -> bool,
) -> Result<Value, StepError> {
    let kinds = kinds_of(params).map_err(StepError::Permanent)?;
    let sample = sample_text.map(str::trim).filter(|s| !s.is_empty());
    let Some(sample) = sample else {
        return Ok(json!({
            "kind": "extract",
            "dry_run": true,
            "writes": "nothing",
            "skipped": true,
            "reason": "sample_required",
            "reason_text": "Der Schritt liest das Transkript der Besprechung des Laufs: für die Vorschau einen Beispieltext angeben.",
        }));
    };
    let _permit = gate
        .try_enter(&llm_need(services))
        .map_err(|w| StepError::Defer {
            retry_after_ms: w.retry_after_ms,
            reason: w.message,
        })?;
    let text: String = sample.chars().take(MAX_SAMPLE_CHARS).collect();
    let segments: Vec<StoredSegment> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(MAX_SAMPLE_SEGMENTS)
        .enumerate()
        .map(|(i, line)| StoredSegment {
            segment_index: i as u32,
            text: line.to_string(),
            start_ms: i as u64 * 5_000,
            end_ms: i as u64 * 5_000 + 5_000,
            channel: 1,
            speaker_index: None,
            words: None,
        })
        .collect();
    let source = Source {
        meeting_id: "sample".to_string(),
        title: "Beispieltext".to_string(),
        date: super::agent_route::today(now_ms),
        labels: SpeakerDirectory::from_segments(&segments),
        segments,
    };
    let runtime = AgentRuntime::new(services.agent_target().map_err(svc)?);
    let opts = ExtractOptions {
        kinds,
        ..ExtractOptions::default()
    };
    let extraction = run_cancellable(cancel, extract::extract(&runtime, &source, &opts))
        .ok_or_else(|| StepError::Transient("Der Lauf wurde abgebrochen.".to_string()))?;
    if let Some(e) = blocking_error(&extraction.outcome) {
        return Err(e);
    }
    let mut data = fit_json(
        extraction.to_json("sample"),
        MAX_OUTPUT_BYTES - OUTPUT_MARGIN_BYTES,
    );
    data["kind"] = json!("extract");
    data["dry_run"] = json!(true);
    data["writes"] = json!("nothing");
    data["summary"] = json!(extraction.summary());
    Ok(data)
}

pub struct AgentExtract {
    spec: &'static catalog::ActionSpec,
    services: Arc<dyn AppServices>,
}

impl AgentExtract {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("agent.extract"),
            services,
        }
    }

    /// Provenienz des Schritts: Modell, Token, Dauer, Quellen (Besprechung und Segmente),
    /// Konfidenz. Nie ein Grund zu scheitern; hoechstens ein Eintrag je Schritt.
    fn record(&self, ctx: &RunCtx<'_>, extraction: &extract::Extraction, sources: &[SourceRef]) {
        if previous_result(ctx, SubjectKind::RunOutput, OPERATION).is_some() {
            return;
        }
        let mut entry = NewProvenance::new(
            SubjectKind::RunOutput,
            &ctx.idempotency_key,
            OPERATION,
            crate::managers::provenance::ActorKind::Workflow,
        );
        entry.provider = Some(if extraction.local { "local" } else { "remote" }.to_string());
        entry.locality = Some(if extraction.local {
            Locality::Local
        } else {
            Locality::Remote
        });
        entry.model_id = Some(extraction.model.clone());
        entry.prompt_tokens = Some(extraction.usage.prompt_tokens);
        entry.completion_tokens = Some(extraction.usage.completion_tokens);
        entry.duration_ms = Some(extraction.duration_ms);
        entry.sources = sources.to_vec();
        entry.confidence = extraction.confidence;
        entry.params = Some(json!({
            "meeting_date": extraction.meeting_date.format("%Y-%m-%d").to_string(),
            "todos": extraction.todos.len(),
            "deadlines": extraction.deadlines.len(),
            "decisions": extraction.decisions.len(),
            "dropped": extraction.dropped.len(),
            "requests": extraction.requests,
            "chunks": extraction.chunks_ok,
            "chunks_total": extraction.chunks_total,
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

impl Action for AgentExtract {
    fn id(&self) -> &str {
        "agent.extract"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Pure
    }

    fn heavy(&self, _params: &Value) -> Option<HeavyNeed> {
        Some(llm_need(&*self.services))
    }

    fn needs(&self, _params: &Value) -> Result<Option<Needs>, NeedsError> {
        Ok(None)
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<(), String> {
        if let Some(Value::Array(_)) = params.get("kinds") {
            kinds_of(&Value::Object(params.clone())).map_err(|m| format!("kinds: {m}"))?;
        }
        Ok(())
    }

    fn describe(&self, params: &Value) -> String {
        let base = catalog::describe_from_spec(self.spec, params);
        match kinds_of(params) {
            Ok(k) if k != Kinds::ALL => {
                let mut names = Vec::new();
                if k.todos {
                    names.push("To-dos");
                }
                if k.deadlines {
                    names.push("Fristen");
                }
                if k.decisions {
                    names.push("Entscheidungen");
                }
                format!("{base} (nur {})", names.join(", "))
            }
            _ => base,
        }
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let kinds = kinds_of(params).map_err(StepError::Permanent)?;
        let meeting_id = meeting_id_of(ctx).ok_or_else(no_meeting)?;
        let (store, meeting) = ready_meeting(ctx, &*self.services, &meeting_id)?;
        let segments = store.get_segments(&meeting_id).map_err(|e| {
            StepError::Transient(format!("Das Transkript ließ sich nicht lesen ({e})."))
        })?;
        let labels = SpeakerDirectory::load(&store, &meeting_id);
        let source = Source::from_meeting(&meeting, segments, labels);

        let runtime = AgentRuntime::new(self.services.agent_target().map_err(svc)?);
        let opts = ExtractOptions {
            kinds,
            ..ExtractOptions::default()
        };
        let cancel = || ctx.cancelled();
        let extraction = run_cancellable(&cancel, extract::extract(&runtime, &source, &opts))
            .ok_or_else(|| StepError::Transient("Der Lauf wurde abgebrochen.".to_string()))?;

        if let Some(e) = blocking_error(&extraction.outcome) {
            return Err(e);
        }

        let mut sources = vec![meeting_source(&meeting)];
        sources.extend(
            extraction
                .cited_segments()
                .into_iter()
                .take(MAX_SEGMENT_SOURCES)
                .map(|n| SourceRef::new("segment", &format!("{}:S{n}", meeting.id), None)),
        );
        self.record(ctx, &extraction, &sources);

        let data = fit_json(
            extraction.to_json(&meeting.id),
            MAX_OUTPUT_BYTES - OUTPUT_MARGIN_BYTES,
        );
        let mut out = StepOutput::with_data(data).summary(&extraction.summary());
        out.sources = sources;
        out.confidence = extraction.confidence;
        Ok(out)
    }
}

/// Haengt die Agent-Bausteine in die Engine (ersetzt die Katalogbausteine).
pub fn install(engine: &super::engine::Engine, services: Arc<dyn AppServices>) {
    engine.register_action(Arc::new(AgentExtract::new(services.clone())));
    super::agent_route::install(engine, services); // C3
}

#[cfg(test)]
mod tests;
