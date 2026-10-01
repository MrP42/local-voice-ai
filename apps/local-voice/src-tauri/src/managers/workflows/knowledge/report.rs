//! Der Baustein `channel.report` (B6): die Kanal-Management-Summary zu einem Video.
//!
//! Aus der Zusammenfassung des Videos, der Einordnung seiner Aussagen (`knowledge.reconcile`) und der
//! Relevanz (`knowledge.rate`) entsteht ein Eintrag fuer die Kanalnotiz: **Neuigkeiten, Erkenntnisse,
//! Handlungsempfehlungen und Quellen**. Die drei Listen schreibt das lokale Modell (ein Aufruf, Schema-
//! Modus, siehe `ask`); alles andere baut der Code: Ueberschrift mit Link auf das Video, Kanal, Datum,
//! Relevanz, die Zaehlung der Einordnung, die markierten Widersprueche und die Quellen (das Video und die
//! Belege, auf die sich die Einordnung stuetzt, mit Wikilinks).
//!
//! `Pure`, kein Recht: der Baustein erzeugt Text. Geschrieben wird er von `obsidian.note` im Modus
//! „Sammelnotiz“ (`entry` = Video-ID): derselbe Eintrag wird ersetzt, ein neues Video kommt oben dazu.
//! Warum nicht in einem Schritt: ein Schritt mit Modell UND Schreiben koennte die Freigabe nicht an den
//! endgueltigen Text binden.
//!
//! Alle Eingaben sind fremd (Zusammenfassung, Aussagen, Belege): sie stehen im Prompt als Daten, und alles,
//! was in den Eintrag geht, ist bereinigt (`md_inline`). Fehlerklassen wie bei `knowledge.rate`; eine auch im
//! zweiten Versuch unbrauchbare Antwort ist hier `Transient` (ohne Management-Summary gibt es nichts zu
//! schreiben, ein neuer Versuch kostet nichts ausser Zeit).
//!
//! Ergebnis: `steps.<id>.markdown` (der Eintrag), `.neuigkeiten`, `.erkenntnisse`,
//! `.handlungsempfehlungen`, `.sources`, `.title`, `.entry` (die Video-ID), `.provenance`.

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::agent::runtime::{AgentError, AgentRuntime};
use crate::managers::provenance::SourceRef;

use super::super::action::{
    Action, EffectKind, HeavyNeed, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::super::app_actions::{llm_need, spec_of, svc, text_param, AppServices};
use super::super::catalog::{self, ActionSpec};
use super::ask::{self, Class, Management};
use super::reconcile::evidence_ref;
use super::sources::Evidence;
use super::{
    agent_step_error, block_on, md_inline, record_llm, video_source, LlmMeta, MAX_TEXT_CHARS,
};

/// Hoechstzahl der Quellen im Eintrag.
const MAX_SOURCES: usize = 12;

/// Ein Wert aus den Parametern oder den Daten des Ausloesers.
fn param_or_trigger(
    ctx: &RunCtx<'_>,
    params: &Value,
    key: &str,
    trigger_key: &str,
) -> Option<String> {
    text_param(params, key).map(str::to_string).or_else(|| {
        ctx.context
            .pointer(&format!("/trigger/{trigger_key}"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

/// Die Aussagen aus `steps.<id>.claims` (defensiv gelesen: es sind Daten eines frueheren Schritts).
#[derive(Clone, Debug)]
struct ClaimIn {
    class: Class,
    text: String,
    reason: String,
    cited: Vec<Evidence>,
}

fn claims_of(params: &Value) -> Vec<ClaimIn> {
    let Some(Value::Array(items)) = params.get("claims") else {
        return Vec::new();
    };
    items
        .iter()
        .take(ask::MAX_CLAIMS)
        .filter_map(|c| {
            let class = Class::parse(c.get("class")?.as_str()?)?;
            let text = md_inline(c.get("text")?.as_str()?, ask::CLAIM_CHARS);
            if text.is_empty() {
                return None;
            }
            let reason = c
                .get("reason")
                .and_then(Value::as_str)
                .map(|r| md_inline(r, 300))
                .unwrap_or_default();
            let numbers: Vec<i64> = c
                .get("cited")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            let cited = c
                .get("evidence")
                .and_then(Value::as_array)
                .map(|ev| {
                    ev.iter()
                        .filter(|e| {
                            e.get("n")
                                .and_then(Value::as_i64)
                                .is_some_and(|n| numbers.contains(&n))
                        })
                        .map(|e| Evidence {
                            source: if e.get("source").and_then(Value::as_str) == Some("vault") {
                                "vault"
                            } else {
                                "wissen"
                            },
                            title: md_inline(
                                e.get("title").and_then(Value::as_str).unwrap_or(""),
                                120,
                            ),
                            path: md_inline(
                                e.get("path").and_then(Value::as_str).unwrap_or(""),
                                200,
                            ),
                            snippet: String::new(),
                            score: 0.0,
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(ClaimIn {
                class,
                text,
                reason,
                cited,
            })
        })
        .collect()
}

fn bullets(out: &mut String, heading: &str, items: &[String]) {
    out.push_str(&format!("**{heading}**\n"));
    if items.is_empty() {
        out.push_str("- –\n");
    } else {
        for i in items {
            out.push_str(&format!("- {i}\n"));
        }
    }
    out.push('\n');
}

/// Der Eintrag fuer die Kanalnotiz (deterministisch aus den Daten; das Modell lieferte nur die Listen).
#[allow(clippy::too_many_arguments)]
fn render_entry(
    title: &str,
    url: Option<&str>,
    date: Option<&str>,
    channel: Option<&str>,
    score: Option<i64>,
    reason: Option<&str>,
    mgmt: &Management,
    claims: &[ClaimIn],
) -> (String, Vec<Value>) {
    let mut out = String::new();
    let heading_title = md_inline(title, 140).replace(['[', ']'], "");
    let head = match url {
        Some(u) => format!("[{heading_title}]({u})"),
        None => heading_title,
    };
    out.push_str(&format!(
        "### {}{head}\n",
        date.map(|d| format!("{d} · ")).unwrap_or_default()
    ));
    let mut facts: Vec<String> = Vec::new();
    if let Some(c) = channel {
        facts.push(format!("Kanal: {}", md_inline(c, 120)));
    }
    if let Some(s) = score {
        let why = reason
            .map(|r| format!(" ({})", md_inline(r, 200)))
            .unwrap_or_default();
        facts.push(format!("Relevanz: {s}/10{why}"));
    }
    if !facts.is_empty() {
        out.push_str(&format!("*{}*\n", facts.join(" · ")));
    }
    out.push('\n');
    bullets(&mut out, "Neuigkeiten", &mgmt.news);
    bullets(&mut out, "Erkenntnisse", &mgmt.insights);
    bullets(&mut out, "Handlungsempfehlungen", &mgmt.actions);

    if !claims.is_empty() {
        let n = |c: Class| claims.iter().filter(|x| x.class == c).count();
        out.push_str(&format!(
            "**Abgleich mit der Wissensbasis:** {} Aussagen – {} neu, {} bereits vorhanden, {} ergänzt, {} widersprechen\n\n",
            claims.len(),
            n(Class::Neu),
            n(Class::Vorhanden),
            n(Class::Ergaenzt),
            n(Class::Widerspricht),
        ));
        for c in claims.iter().filter(|c| c.class == Class::Widerspricht) {
            let refs: Vec<String> = c.cited.iter().map(evidence_ref).collect();
            out.push_str("> [!warning] Widerspruch\n");
            out.push_str(&format!("> {}\n", c.text));
            if !refs.is_empty() {
                out.push_str(&format!("> Widerspricht: {}\n", refs.join(", ")));
            }
            if !c.reason.is_empty() {
                out.push_str(&format!("> Begründung: {}\n", c.reason));
            }
            out.push('\n');
        }
    }

    // Quellen: das Video und die Belege, auf die sich die Einordnung stuetzt.
    let mut sources: Vec<Value> = Vec::new();
    out.push_str("**Quellen**\n");
    match url {
        Some(u) => {
            out.push_str(&format!("- Video: <{u}>\n"));
            sources.push(json!({"kind": "youtube", "url": u, "title": title}));
        }
        None => out.push_str("- Video: (ohne Link)\n"),
    }
    let mut seen: Vec<String> = Vec::new();
    for c in claims.iter().filter(|c| c.class != Class::Neu) {
        for e in &c.cited {
            if sources.len() >= MAX_SOURCES {
                break;
            }
            if seen.contains(&e.path) {
                continue;
            }
            seen.push(e.path.clone());
            out.push_str(&format!("- Wissen: {}\n", evidence_ref(e)));
            sources.push(json!({"kind": e.source, "path": e.path, "title": e.title}));
        }
    }
    (out, sources)
}

pub struct ChannelReport {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl ChannelReport {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("channel.report"),
            services,
        }
    }
}

fn score_of(params: &Value) -> Option<i64> {
    match params.get("score") {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n| (0..=10).contains(n))
}

impl Action for ChannelReport {
    fn id(&self) -> &str {
        "channel.report"
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
        match params.get("claims") {
            None | Some(Value::Null) | Some(Value::Array(_)) => Ok(()),
            Some(Value::String(s)) if s.contains("{{") => Ok(()),
            Some(_) => Err("claims: erwartet wird die Liste der Aussagen aus „Mit der Wissensbasis abgleichen“ ({{steps.<id>.claims}}).".to_string()),
        }
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let summary = text_param(params, "source").ok_or_else(|| {
            StepError::Permanent(
                "Es ist keine Zusammenfassung angegeben (Parameter source).".to_string(),
            )
        })?;
        let summary: String = super::md_block(summary, MAX_TEXT_CHARS);
        if summary.is_empty() {
            return Err(StepError::Permanent(
                "Die Zusammenfassung ist leer; daraus lässt sich keine Management-Summary schreiben.".to_string(),
            ));
        }
        let title =
            param_or_trigger(ctx, params, "title", "title").unwrap_or_else(|| "Video".to_string());
        let channel = param_or_trigger(ctx, params, "channel", "channel_title");
        let url = param_or_trigger(ctx, params, "url", "url").filter(|u| {
            (u.starts_with("https://") || u.starts_with("http://"))
                && !u.chars().any(|c| c.is_whitespace() || c.is_control())
        });
        let video_id = param_or_trigger(ctx, params, "video_id", "video_id");
        let published = param_or_trigger(ctx, params, "published", "published")
            .and_then(|p| p.get(..10).map(str::to_string))
            .filter(|d| d.is_ascii());
        let score = score_of(params);
        let reason = text_param(params, "reason").map(str::to_string);
        let claims = claims_of(params);

        let rt = AgentRuntime::new(self.services.agent_target().map_err(svc)?);
        let pairs: Vec<(Class, String)> =
            claims.iter().map(|c| (c.class, c.text.clone())).collect();
        let mut text = summary.clone();
        let mut answered = None;
        for _ in 0..3 {
            let user = ask::management_user(
                channel.as_deref().unwrap_or(""),
                &title,
                score.map(|s| (s, reason.as_deref().unwrap_or(""))),
                &text,
                &pairs,
            );
            match block_on(ctx, ask::management(&rt, &user))? {
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
        let mgmt = answer.value;
        let mut meta = LlmMeta {
            model: rt.model().to_string(),
            local: rt.is_local(),
            ..LlmMeta::default()
        };
        meta.add(answer.usage, answer.duration_ms, answer.attempts);

        let (markdown, sources_json) = render_entry(
            &title,
            url.as_deref(),
            published.as_deref(),
            channel.as_deref(),
            score,
            reason.as_deref(),
            &mgmt,
            &claims,
        );
        let mut sources: Vec<SourceRef> = Vec::new();
        if let Some(v) = video_source(ctx) {
            sources.push(v);
        }
        record_llm(
            ctx,
            "channel_report",
            &meta,
            &sources,
            json!({
                "news": mgmt.news.len(),
                "insights": mgmt.insights.len(),
                "actions": mgmt.actions.len(),
                "claims": claims.len(),
            }),
        );
        let mut out = StepOutput::with_data(json!({
            "markdown": markdown,
            "neuigkeiten": mgmt.news,
            "erkenntnisse": mgmt.insights,
            "handlungsempfehlungen": mgmt.actions,
            "sources": sources_json,
            "title": title,
            "entry": video_id,
            "provenance": meta.to_json(),
        }))
        .summary(&format!(
            "Management-Summary erzeugt: {} Neuigkeiten, {} Erkenntnisse, {} Handlungsempfehlungen.",
            mgmt.news.len(),
            mgmt.insights.len(),
            mgmt.actions.len()
        ));
        out.sources = sources;
        Ok(out)
    }
}
