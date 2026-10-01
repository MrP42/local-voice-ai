//! Der Baustein `knowledge.rate` (B6): Relevanz nach dem Themenprofil.
//!
//! Das Profil ist ein Text des Nutzers (Themen und Ausschluesse, z. B. in der Variable `profil` des
//! Ablaufs) und damit bearbeitbar (R10); das Modell bewertet den Inhalt von 0 bis 10 und begruendet es.
//! `relevant` ist `score >= threshold` (Vorgabe 6): spaetere Schritte fragen es mit `when:
//! steps.<id>.relevant == true` ab. Ob die Bewertung stimmt, entscheidet der Mensch; „relevant/nicht
//! relevant“ fliesst von Hand ins Profil.
//!
//! `Pure`, kein Recht: der Baustein liest nur den Text, den der Ablauf ihm gibt, und liefert Daten. Der
//! Text ist fremd (Transkript, Zusammenfassung): er steht im Prompt als Daten (siehe `ask`), die Antwort ist
//! an das Schema gebunden, und die Begruendung wird bereinigt.
//!
//! Ergebnis: `steps.<id>.score`, `.relevant`, `.threshold`, `.reason`, `.topics`, `.title`, `.truncated`,
//! `.provenance` (Modell, Token, Dauer).

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::agent::runtime::{AgentError, AgentRuntime};
use crate::managers::provenance::SourceRef;

use super::super::action::{
    Action, EffectKind, HeavyNeed, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::super::app_actions::{llm_need, meeting_source, spec_of, svc, text_param, AppServices};
use super::super::catalog::{self, ActionSpec};
use super::ask;
use super::{
    agent_step_error, block_on, material, md_block, record_llm, video_source, LlmMeta,
    MAX_TEXT_CHARS,
};

pub const DEFAULT_THRESHOLD: i64 = 6;
const MAX_PROFILE_CHARS: usize = 4_000;

/// Die Schwelle aus dem Parameter: Zahl, Text mit Zahl oder die Vorgabe.
pub fn threshold_of(params: &Value) -> Result<i64, StepError> {
    let n = match params.get("threshold") {
        None | Some(Value::Null) => return Ok(DEFAULT_THRESHOLD),
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) if s.trim().is_empty() => return Ok(DEFAULT_THRESHOLD),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    };
    match n {
        Some(n) if (1..=10).contains(&n) => Ok(n),
        _ => Err(StepError::Permanent(
            "Die Schwelle (threshold) muss eine Zahl von 1 bis 10 sein.".to_string(),
        )),
    }
}

pub struct KnowledgeRate {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl KnowledgeRate {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("knowledge.rate"),
            services,
        }
    }
}

impl Action for KnowledgeRate {
    fn id(&self) -> &str {
        "knowledge.rate"
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
        match params.get("profile") {
            Some(Value::String(s)) if s.trim().is_empty() => {
                Err("profile: das Themenprofil darf nicht leer sein.".to_string())
            }
            _ => Ok(()),
        }
    }

    fn describe(&self, params: &Value) -> String {
        let base = catalog::describe_from_spec(self.spec, params);
        match text_param(params, "threshold") {
            Some(t) => format!("{base}; relevant ab {t}"),
            None => base,
        }
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let profile = text_param(params, "profile")
            .map(|p| md_block(p, MAX_PROFILE_CHARS))
            .filter(|p| !p.is_empty())
            .ok_or_else(|| {
                StepError::Permanent(
                    "Es ist kein Themenprofil angegeben (Parameter profile): Themen und Ausschlüsse als Text."
                        .to_string(),
                )
            })?;
        let threshold = threshold_of(params)?;
        let material = material(ctx, &*self.services, params, MAX_TEXT_CHARS)?;
        let rt = AgentRuntime::new(self.services.agent_target().map_err(svc)?);

        // Passt der Text nicht in den Kontext, wird er halbiert (hoechstens zweimal).
        let mut text: String = material.text.clone();
        let mut answered = None;
        for _ in 0..3 {
            match block_on(ctx, ask::rate(&rt, &profile, &material.title, &text))? {
                Ok(a) => {
                    answered = Some(a);
                    break;
                }
                Err(AgentError::ContextExceeded) => {
                    let keep = text.chars().count() / 2;
                    text = text.chars().take(keep).collect();
                }
                Err(e) => return Err(agent_step_error(&e)),
            }
        }
        let answer = answered.ok_or_else(|| {
            StepError::Permanent(
                "Der Text ist für den Kontext des Sprachmodells zu lang.".to_string(),
            )
        })?;

        let rating = answer.value;
        let mut meta = LlmMeta {
            model: rt.model().to_string(),
            local: rt.is_local(),
            ..LlmMeta::default()
        };
        meta.add(answer.usage, answer.duration_ms, answer.attempts);

        let mut sources: Vec<SourceRef> = Vec::new();
        if let Some(m) = &material.meeting {
            sources.push(meeting_source(m));
        }
        if let Some(v) = video_source(ctx) {
            sources.push(v);
        }
        record_llm(
            ctx,
            "knowledge_rate",
            &meta,
            &sources,
            json!({
                "score": rating.score,
                "threshold": threshold,
                "chars": material.chars,
                "truncated": material.truncated,
            }),
        );
        let relevant = i64::from(rating.score) >= threshold;
        let mut out = StepOutput::with_data(json!({
            "score": rating.score,
            "relevant": relevant,
            "threshold": threshold,
            "reason": rating.reason,
            "topics": rating.topics,
            "title": material.title,
            "truncated": material.truncated,
            "provenance": meta.to_json(),
        }))
        .summary(&format!(
            "Relevanz {} von 10 ({}): {}",
            rating.score,
            if relevant {
                "relevant"
            } else {
                "nicht relevant"
            },
            crate::agent::extract::clean_text(&rating.reason, 160)
        ));
        out.sources = sources;
        Ok(out)
    }
}
