//! Der Baustein `knowledge.reconcile` (B6): Aussagen mit der Wissensbasis abgleichen.
//!
//! Ablauf des Schritts:
//!
//! 1. **Aussagen ziehen.** Das lokale Modell zerlegt den Text (meist die Zusammenfassung des Videos) in
//!    hoechstens `max_claims` pruefbare Einzelaussagen (Schema-Modus, siehe `ask`).
//! 2. **Nachschlagen.** Je Aussage sucht der Code in der Wissensbasis (`wissen_suchen` ueber MCP) und, wenn
//!    `vault` gesetzt ist, im Vault (Wortsuche, siehe `sources`). Notizen zu DIESEM Video zaehlen nicht.
//! 3. **Einordnen.** Ohne Treffer ist die Aussage `neu` (kein Modellaufruf). Mit Treffern ordnet das Modell
//!    sie ein: `vorhanden`, `ergaenzt`, `widerspricht` oder `neu` (die Treffer behandeln das Thema nicht),
//!    mit den Nummern der Belege, auf die es sich stuetzt. Ohne belegte Einordnung (auch nach dem
//!    Wiederholversuch) bleibt `unklar`: ein Widerspruch ohne Beleg wird nie behauptet (R8).
//!
//! Der Baustein **schreibt nichts**. Er liefert Daten (`claims`, `counts`) und fertiges Markdown
//! (`markdown`: der Abschnitt „Abgleich mit der Wissensbasis“ mit Verweisen und markierten Widerspruechen);
//! die Notiz schreibt `obsidian.note` mit eigenem Recht (und fragt, wenn es eine vorhandene aendert).
//! Belege sind immer vom Code gebildet (Treffer aus Suche und Vault), nie vom Modell erfunden: das Modell
//! nennt nur Nummern.
//!
//! # Rechte
//!
//! `knowledge.search` an der Wissensbasis `via` (Katalog; Vorgabe fuer Ablaeufe: erlaubt). Liest der Schritt
//! zusaetzlich den Vault (`vault`), prueft `gate_view` das Recht `files.read` an diesem Vault: „aus“ ->
//! abgelehnt mit Audit, „fragen“ -> der Schritt verlangt die Freigabe, „erlaubt“ -> frei.
//!
//! # Fehlerklassen
//!
//! | Lage | Ergebnis des Schritts |
//! |------|-----------------------|
//! | Wissensbasis nicht erreichbar, Zeit, 429/5xx, Vault nicht eingehaengt | `Transient`: nichts geschrieben, **nie** „alles neu“ daraus abgeleitet |
//! | Schluessel/Scope/Adresse/Werkzeug falsch, Integration fehlt oder falsche Art | `Permanent` mit dem Satz des Dienstes |
//! | Modellserver nicht erreichbar, Zeit | `Transient`; Speicher knapp, Server belegt, Neustart gesperrt | `Defer` |
//! | Modell nicht eingerichtet, Anbieter nicht lokal, Anfrage abgelehnt | `Permanent` |
//! | Antwort zu Aussagen auch im 2. Versuch unbrauchbar | **Erfolg** mit `outcome: no_action` |
//! | Einordnung einer Aussage unbrauchbar oder ohne Beleg | die Aussage ist `unklar`, der Schritt laeuft weiter |
//! | Nutzer bricht ab; Zeitgrenze des Schritts (15 min) | `Transient` |

use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::agent::extract::clean_text;
use crate::agent::runtime::{AgentError, AgentRuntime};
use crate::managers::integrations::audit;
use crate::managers::integrations::grants::explain;
use crate::managers::integrations::model::{
    AuditOutcome, Caller, Capability, GrantMode, Kind as IntegrationKind, NewAudit,
};
use crate::managers::integrations::store as integrations_store;
use crate::managers::integrations::wissen::HttpOpts;
use crate::managers::provenance::SourceRef;

use super::super::action::{
    Action, EffectKind, GateEnv, GateView, HeavyNeed, Needs, NeedsError, RunCtx, StepError,
    StepOutput,
};
use super::super::app_actions::{
    has_template, llm_need, meeting_source, spec_of, svc, text_param, AppServices,
};
use super::super::catalog::{self, ActionSpec};
use super::ask::{self, Class};
use super::note::yq;
use super::sources::{
    Evidence, Exclude, SearchError, SourceSet, VaultIndex, VaultLimits, WissenSource, MAX_LIMIT,
};
use super::vault_note::open_vault;
use super::{
    agent_step_error, block_on, db_err, material, md_inline, record_llm, video_source, LlmMeta,
    MAX_TEXT_CHARS,
};

pub const DEFAULT_MAX_CLAIMS: i64 = 8;
pub const DEFAULT_LIMIT: i64 = 5;
/// So lange darf ein Schritt hoechstens suchen und einordnen (Wanduhr).
pub const RUN_BUDGET: Duration = Duration::from_secs(15 * 60);
/// Groesste Ausgabe des Schritts, bevor sie gekuerzt wird (die Engine erlaubt 64 KiB).
const OUTPUT_CAP_BYTES: usize = 56 * 1024;

fn int_param(
    params: &Value,
    key: &str,
    default: i64,
    min: i64,
    max: i64,
) -> Result<i64, StepError> {
    let n = match params.get(key) {
        None | Some(Value::Null) => return Ok(default),
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) if s.trim().is_empty() => return Ok(default),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    };
    match n {
        Some(n) if (min..=max).contains(&n) => Ok(n),
        _ => Err(StepError::Permanent(format!(
            "{key}: erwartet wird eine Zahl von {min} bis {max}."
        ))),
    }
}

fn search_error(e: SearchError) -> StepError {
    match e {
        SearchError::Transient(m) => StepError::Transient(m),
        SearchError::Permanent(m) => StepError::Permanent(m),
    }
}

fn audit_refusal(conn: &Connection, integration: &str, target: &str, reason: &str) {
    let entry = NewAudit {
        caller: Caller::Workflow.as_str().to_string(),
        integration_id: Some(integration.to_string()),
        capability: Some(Capability::FilesRead.as_str().to_string()),
        target: Some(target.to_string()),
        outcome: AuditOutcome::Denied,
        detail: Some(json!({ "reason": reason })),
    };
    if let Err(e) = audit::record(conn, &entry) {
        log::warn!("workflows: Audit der Ablehnung nicht geschrieben: {e}");
    }
}

// ---------------------------------------------------------------------------
// Ergebnis je Aussage und Markdown
// ---------------------------------------------------------------------------

/// Eine eingeordnete Aussage.
#[derive(Clone, Debug)]
pub struct ClaimOut {
    pub id: String,
    pub text: String,
    pub class: Class,
    pub reason: String,
    /// Nullbasierte Indizes der Belege, auf die sich die Einordnung stuetzt.
    pub cited: Vec<usize>,
    pub evidence: Vec<Evidence>,
}

impl ClaimOut {
    fn cited_evidence(&self) -> impl Iterator<Item = &Evidence> {
        self.cited.iter().filter_map(|i| self.evidence.get(*i))
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "text": self.text,
            "class": self.class.as_str(),
            "label": self.class.label(),
            "reason": self.reason,
            "cited": self.cited.iter().map(|i| i + 1).collect::<Vec<_>>(),
            "evidence": self.evidence.iter().enumerate().map(|(n, e)| json!({
                "n": n + 1,
                "source": e.source,
                "title": e.title,
                "path": e.path,
                "snippet": e.snippet,
                "score": (e.score * 100.0).round() / 100.0,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Ein Wikilink nur fuer Pfade, die darin nichts kaputt machen.
fn wikilink_target(path: &str) -> Option<String> {
    let p = path.trim().replace('\\', "/");
    let stem = p.strip_suffix(".md").or_else(|| p.strip_suffix(".MD"))?;
    let ok = !stem.is_empty()
        && stem.chars().count() <= 160
        && !stem.starts_with('/')
        && !stem.contains("..")
        && stem
            .chars()
            .all(|c| !c.is_control() && !"[]|#^\\<>:*?\"".contains(c));
    ok.then(|| stem.to_string())
}

/// Der Beleg als Markdown: Wikilink auf eine Vault-Notiz, sonst Titel und Pfad als Text.
pub fn evidence_ref(e: &Evidence) -> String {
    match wikilink_target(&e.path) {
        Some(target) => format!("[[{target}]]"),
        None => {
            let title = md_inline(&e.title, 80);
            let path = md_inline(&e.path, 80);
            match (title.is_empty(), path.is_empty()) {
                (false, false) => format!("„{title}“ ({path})"),
                (false, true) => format!("„{title}“"),
                (true, false) => path,
                (true, true) => "(ohne Titel)".to_string(),
            }
        }
    }
}

fn refs_of(claim: &ClaimOut) -> String {
    claim
        .cited_evidence()
        .map(evidence_ref)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn counts_of(claims: &[ClaimOut]) -> Value {
    let mut map = Map::new();
    for class in Class::ALL {
        let n = claims.iter().filter(|c| c.class == class).count();
        map.insert(class.as_str().to_string(), json!(n));
    }
    map.insert("gesamt".to_string(), json!(claims.len()));
    Value::Object(map)
}

/// Der Abschnitt „Abgleich mit der Wissensbasis“ fuer die Notiz (deterministisch, ohne Modell).
pub fn render_markdown(
    claims: &[ClaimOut],
    searched: &[&str],
    vault_incomplete: bool,
    note: Option<&str>,
) -> String {
    let mut out = String::from("## Abgleich mit der Wissensbasis\n\n");
    if let Some(n) = note {
        out.push_str(&format!("*{n}*\n"));
        return out;
    }
    let count = |c: Class| claims.iter().filter(|x| x.class == c).count();
    let places: Vec<&str> = searched
        .iter()
        .map(|s| {
            if *s == "wissen" {
                "Wissensbasis"
            } else {
                "Vault"
            }
        })
        .collect();
    let mut summary = format!(
        "{} Aussagen geprüft: {} neu, {} bereits vorhanden, {} ergänzt, {} widersprechen",
        claims.len(),
        count(Class::Neu),
        count(Class::Vorhanden),
        count(Class::Ergaenzt),
        count(Class::Widerspricht),
    );
    if count(Class::Unklar) > 0 {
        summary.push_str(&format!(", {} unklar", count(Class::Unklar)));
    }
    out.push_str(&format!(
        "*{summary}. Durchsucht: {}.*\n",
        places.join(", ")
    ));
    if vault_incomplete {
        out.push_str("*Hinweis: Der Vault ist sehr groß und wurde nur teilweise durchsucht.*\n");
    }
    out.push('\n');
    for c in claims {
        let refs = refs_of(c);
        match c.class {
            Class::Widerspricht => {
                out.push_str("> [!warning] Widerspruch\n");
                out.push_str(&format!("> {}\n", c.text));
                if !refs.is_empty() {
                    out.push_str(&format!("> Widerspricht: {refs}\n"));
                }
                if !c.reason.is_empty() {
                    out.push_str(&format!("> Begründung: {}\n", c.reason));
                }
                out.push('\n');
            }
            Class::Neu => out.push_str(&format!("- **neu** — {}\n", c.text)),
            Class::Vorhanden => {
                out.push_str(&format!("- **bereits vorhanden** — {}", c.text));
                if !refs.is_empty() {
                    out.push_str(&format!(" · Beleg: {refs}"));
                }
                out.push('\n');
            }
            Class::Ergaenzt => {
                out.push_str(&format!("- **ergänzt** — {}", c.text));
                if !refs.is_empty() {
                    out.push_str(&format!(" · Ergänzt: {refs}"));
                }
                if !c.reason.is_empty() {
                    out.push_str(&format!(" — {}", c.reason));
                }
                out.push('\n');
            }
            Class::Unklar => {
                out.push_str(&format!("- **unklar (bitte prüfen)** — {}\n", c.text));
            }
        }
    }
    out
}

/// Kuerzt die Ausgabe schrittweise, bis sie unter dem Hoechstmass liegt: erst die Ausschnitte, dann
/// die Belege (ausser den genannten), dann alle Belege. `markdown` und die Aussagen bleiben.
pub(super) fn fit_output(data: &mut Value) {
    let size = |d: &Value| d.to_string().len();
    if size(data) <= OUTPUT_CAP_BYTES {
        return;
    }
    let each_claim = |d: &mut Value, f: &mut dyn FnMut(&mut Map<String, Value>)| {
        if let Some(Value::Array(claims)) = d.get_mut("claims") {
            for c in claims {
                if let Some(o) = c.as_object_mut() {
                    f(o);
                }
            }
        }
    };
    each_claim(data, &mut |o| {
        if let Some(Value::Array(ev)) = o.get_mut("evidence") {
            for e in ev {
                if let Some(s) = e.get("snippet").and_then(Value::as_str) {
                    let cut = clean_text(s, 100);
                    e["snippet"] = json!(cut);
                }
            }
        }
    });
    if size(data) <= OUTPUT_CAP_BYTES {
        data["truncated_evidence"] = json!(true);
        return;
    }
    each_claim(data, &mut |o| {
        let cited: Vec<i64> = o
            .get("cited")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        if let Some(Value::Array(ev)) = o.get_mut("evidence") {
            ev.retain(|e| {
                e.get("n")
                    .and_then(Value::as_i64)
                    .is_some_and(|n| cited.contains(&n))
            });
        }
    });
    if size(data) > OUTPUT_CAP_BYTES {
        each_claim(data, &mut |o| {
            o.insert("evidence".to_string(), json!([]));
        });
    }
    data["truncated_evidence"] = json!(true);
}

// ---------------------------------------------------------------------------
// Der Baustein
// ---------------------------------------------------------------------------

pub struct KnowledgeReconcile {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
    vault_limits: VaultLimits,
    budget: Duration,
    http: HttpOpts,
}

impl KnowledgeReconcile {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("knowledge.reconcile"),
            services,
            vault_limits: VaultLimits::default(),
            budget: RUN_BUDGET,
            http: HttpOpts::default(),
        }
    }

    /// Fuer Tests: kleine Grenzen.
    #[allow(dead_code)]
    pub fn with_limits(mut self, vault: VaultLimits, budget: Duration, http: HttpOpts) -> Self {
        self.vault_limits = vault;
        self.budget = budget;
        self.http = http;
        self
    }

    /// Ordnet eine Aussage ein: ohne Treffer `neu`, sonst das Modell.
    fn classify_one(
        &self,
        ctx: &RunCtx<'_>,
        rt: &AgentRuntime,
        meta: &mut LlmMeta,
        claim: &str,
        evidence: &[Evidence],
    ) -> Result<(Class, Vec<usize>, String), StepError> {
        if evidence.is_empty() {
            return Ok((
                Class::Neu,
                Vec::new(),
                "Keine Treffer in der Wissensbasis und im Vault.".to_string(),
            ));
        }
        // Passt es nicht in den Kontext: kuerzere Ausschnitte, dann weniger Belege.
        let mut shown: Vec<Evidence> = evidence.to_vec();
        for round in 0..3 {
            match block_on(ctx, ask::classify(rt, claim, &shown))? {
                Ok(answer) => {
                    meta.add(answer.usage, answer.duration_ms, answer.attempts);
                    let v = answer.value;
                    return Ok((v.class, v.evidence, v.reason));
                }
                Err(AgentError::ContextExceeded) if round < 2 => {
                    if round == 0 {
                        for e in &mut shown {
                            e.snippet = clean_text(&e.snippet, 120);
                        }
                    } else {
                        shown.truncate(2);
                    }
                }
                Err(AgentError::SchemaInvalid {
                    usage, duration_ms, ..
                }) => {
                    meta.add(usage, duration_ms, ask_attempts());
                    return Ok((
                        Class::Unklar,
                        Vec::new(),
                        "Das Sprachmodell lieferte keine belegte Einordnung; bitte selbst prüfen."
                            .to_string(),
                    ));
                }
                Err(AgentError::ContextExceeded) => {
                    return Ok((
                        Class::Unklar,
                        Vec::new(),
                        "Aussage und Belege passen nicht in den Kontext des Sprachmodells; bitte selbst prüfen."
                            .to_string(),
                    ));
                }
                Err(e) => return Err(agent_step_error(&e)),
            }
        }
        Ok((Class::Unklar, Vec::new(), String::new()))
    }
}

fn ask_attempts() -> u32 {
    crate::agent::runtime::MAX_ATTEMPTS
}

impl Action for KnowledgeReconcile {
    fn id(&self) -> &str {
        "knowledge.reconcile"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Pure
    }

    fn heavy(&self, _params: &Value) -> Option<HeavyNeed> {
        Some(llm_need(&*self.services))
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn describe(&self, params: &Value) -> String {
        let base = catalog::describe_from_spec(self.spec, params);
        match text_param(params, "vault") {
            Some(v) => format!("{base} (auch im Vault {v})"),
            None => base,
        }
    }

    fn gate_view(&self, env: &GateEnv<'_>, params: &Value) -> Result<Option<GateView>, StepError> {
        let video = text_param(params, "video_id")
            .map(str::to_string)
            .or_else(|| {
                env.context
                    .pointer("/trigger/video_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        let target = video
            .clone()
            .or_else(|| text_param(params, "title").map(|t| md_inline(t, 80)))
            .unwrap_or_else(|| "Abgleich".to_string());
        let mut args = json!({
            "via": text_param(params, "via"),
            "vault": text_param(params, "vault"),
            "video_id": video,
            "max_claims": params.get("max_claims"),
            "limit": params.get("limit"),
        });
        let mut max_mode = None;
        if let Some(vault_id) = text_param(params, "vault") {
            if !(env.planning && has_template(vault_id)) {
                let integration = integrations_store::get(env.conn, vault_id)
                    .map_err(db_err)?
                    .ok_or_else(|| {
                        StepError::Permanent(format!("Der Vault „{vault_id}“ gibt es nicht."))
                    })?;
                if integration.kind != IntegrationKind::Obsidian {
                    return Err(StepError::Permanent(format!(
                        "Die Integration „{vault_id}“ ist kein Obsidian-Vault."
                    )));
                }
                let grants = integrations_store::grants_for(env.conn, vault_id).map_err(db_err)?;
                let (mode, reason) = explain(
                    &integration,
                    Capability::FilesRead,
                    Caller::Workflow,
                    &grants,
                    None,
                );
                match mode {
                    GrantMode::Off => {
                        let why = reason.map(|r| r.as_str()).unwrap_or("grant_off");
                        if !env.planning {
                            audit_refusal(env.conn, vault_id, &target, why);
                        }
                        return Err(StepError::Denied(format!(
                            "Das Lesen des Vaults „{}“ ist für Abläufe ausgeschaltet.",
                            integration.label
                        )));
                    }
                    GrantMode::Ask => {
                        max_mode = Some(GrantMode::Ask);
                        args["liest_vault"] = json!(integration.label);
                    }
                    GrantMode::Allow => {}
                }
            }
        }
        Ok(Some(GateView {
            target: Some(target),
            args,
            max_mode,
        }))
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let via = text_param(params, "via").ok_or_else(|| {
            StepError::Permanent("Es ist keine Wissensbasis angegeben (Parameter via).".to_string())
        })?;
        let max_claims = int_param(
            params,
            "max_claims",
            DEFAULT_MAX_CLAIMS,
            1,
            super::ask::MAX_CLAIMS as i64,
        )? as usize;
        let limit = int_param(params, "limit", DEFAULT_LIMIT, 1, MAX_LIMIT as i64)? as usize;
        let material = material(ctx, &*self.services, params, MAX_TEXT_CHARS)?;
        let video_id = text_param(params, "video_id")
            .map(str::to_string)
            .or_else(|| {
                ctx.context
                    .pointer("/trigger/video_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });

        // Wissensbasis und Vault zuerst oeffnen: ein falscher Schluessel oder ein fehlender Vault kostet
        // so keinen Modellaufruf.
        let conn = ctx.conn().map_err(db_err)?;
        let wissen_integration = integrations_store::get(&conn, via)
            .map_err(db_err)?
            .ok_or_else(|| {
                StepError::Permanent(format!("Die Wissensbasis „{via}“ gibt es nicht."))
            })?;
        if wissen_integration.kind != IntegrationKind::Wissen {
            return Err(StepError::Permanent(format!(
                "Die Integration „{via}“ ist keine Wissensbasis."
            )));
        }
        let token = self
            .services
            .secret(&wissen_integration, "token")
            .map_err(|e| {
                StepError::Permanent(format!(
                    "Der Schlüssel der Wissensbasis ließ sich nicht lesen ({e})."
                ))
            })?;
        let wissen = WissenSource::open(&wissen_integration, token, self.http.clone())
            .map_err(search_error)?;
        let vault = match text_param(params, "vault") {
            Some(id) => Some(open_vault(&conn, id)?),
            None => None,
        };
        let rt = AgentRuntime::new(self.services.agent_target().map_err(svc)?);
        let mut meta = LlmMeta {
            model: rt.model().to_string(),
            local: rt.is_local(),
            ..LlmMeta::default()
        };

        // 1. Aussagen
        let mut text = material.text.clone();
        let mut claims_answer = None;
        let mut schema_failure: Option<AgentError> = None;
        for _ in 0..3 {
            match block_on(
                ctx,
                ask::extract_claims(&rt, &material.title, &text, max_claims),
            )? {
                Ok(a) => {
                    claims_answer = Some(a);
                    break;
                }
                Err(AgentError::ContextExceeded) => {
                    let keep = text.chars().count() / 2;
                    text = text.chars().take(keep).collect();
                }
                Err(e @ AgentError::SchemaInvalid { .. }) => {
                    schema_failure = Some(e);
                    break;
                }
                Err(e) => return Err(agent_step_error(&e)),
            }
        }
        let mut sources: Vec<SourceRef> = Vec::new();
        if let Some(m) = &material.meeting {
            sources.push(meeting_source(m));
        }
        if let Some(v) = video_source(ctx) {
            sources.push(v);
        }
        let finish = |meta: &LlmMeta,
                      outcome: &str,
                      reason: Option<&str>,
                      claims: &[ClaimOut],
                      markdown: String,
                      searched: Vec<&'static str>,
                      vault_files: usize,
                      vault_incomplete: bool,
                      sources: Vec<SourceRef>|
         -> StepOutput {
            let mut data = json!({
                "outcome": outcome,
                "reason": reason,
                "claims": claims.iter().map(ClaimOut::to_json).collect::<Vec<_>>(),
                "counts": counts_of(claims),
                "markdown": markdown,
                "sources_searched": searched,
                "vault_files": vault_files,
                "vault_incomplete": vault_incomplete,
                "title": material.title,
                "truncated": material.truncated,
                "provenance": meta.to_json(),
            });
            fit_output(&mut data);
            let counts = counts_of(claims);
            let summary = match outcome {
                "reconciled" => format!(
                    "{} Aussagen abgeglichen: {} neu, {} vorhanden, {} ergänzt, {} widersprechen.",
                    claims.len(),
                    counts["neu"],
                    counts["vorhanden"],
                    counts["ergaenzt"],
                    counts["widerspricht"]
                ),
                "no_claims" => "Keine prüfbaren Aussagen im Text.".to_string(),
                _ => "Aussagen ließen sich nicht ziehen (Antwort des Modells unbrauchbar)."
                    .to_string(),
            };
            let mut out = StepOutput::with_data(data).summary(&summary);
            out.sources = sources;
            out
        };

        let Some(answer) = claims_answer else {
            // Auch im zweiten Versuch kein gueltiges JSON: nichts erfinden, das ist ein Ergebnis.
            let (usage, duration_ms) = match &schema_failure {
                Some(AgentError::SchemaInvalid {
                    usage, duration_ms, ..
                }) => (*usage, *duration_ms),
                _ => Default::default(),
            };
            meta.add(usage, duration_ms, ask_attempts());
            record_llm(
                ctx,
                "knowledge_reconcile",
                &meta,
                &sources,
                json!({"outcome": "no_action"}),
            );
            let code = schema_failure
                .as_ref()
                .map(AgentError::code)
                .unwrap_or("schema_invalid");
            let md = render_markdown(
                &[],
                &[],
                false,
                Some("Aus dem Text ließen sich keine Aussagen ziehen (die Antwort des Sprachmodells war unbrauchbar)."),
            );
            return Ok(finish(
                &meta,
                "no_action",
                Some(code),
                &[],
                md,
                Vec::new(),
                0,
                false,
                sources,
            ));
        };
        meta.add(answer.usage, answer.duration_ms, answer.attempts);
        let claims = answer.value;
        if claims.is_empty() {
            record_llm(
                ctx,
                "knowledge_reconcile",
                &meta,
                &sources,
                json!({"outcome": "no_claims"}),
            );
            let md = render_markdown(
                &[],
                &[],
                false,
                Some("Im Text standen keine prüfbaren Aussagen."),
            );
            return Ok(finish(
                &meta,
                "no_claims",
                None,
                &[],
                md,
                Vec::new(),
                0,
                false,
                sources,
            ));
        }

        // 2. Quellen (ein Durchlauf durch den Vault)
        let cancel = || ctx.cancelled();
        let (index, exclude_rels) = match &vault {
            Some(v) => {
                let mut lines: Vec<String> = Vec::new();
                if let Some(id) = &video_id {
                    lines.push(format!("lva_id: {}", yq(&format!("video-{id}"))));
                    lines.push(format!("video_id: {}", yq(id)));
                }
                let idx = VaultIndex::build(&v.sandbox, &lines, &self.vault_limits, &cancel);
                let rels = idx.excluded.clone();
                (Some(idx), rels)
            }
            None => (None, Vec::new()),
        };
        if ctx.cancelled() {
            return Err(StepError::Transient(
                "Der Lauf wurde abgebrochen.".to_string(),
            ));
        }
        let set = SourceSet {
            wissen: Some(wissen),
            vault: index,
            exclude: Exclude {
                video_id: video_id.clone(),
                rels: exclude_rels,
            },
        };

        // 3. Je Aussage nachschlagen und einordnen
        let started = Instant::now();
        let mut out_claims: Vec<ClaimOut> = Vec::with_capacity(claims.len());
        for (i, claim) in claims.iter().enumerate() {
            if ctx.cancelled() {
                return Err(StepError::Transient(
                    "Der Lauf wurde abgebrochen.".to_string(),
                ));
            }
            if started.elapsed() > self.budget {
                return Err(StepError::Transient(
                    "Der Abgleich dauert zu lange (Zeitgrenze des Schritts); ein neuer Versuch beginnt von vorn."
                        .to_string(),
                ));
            }
            let evidence = set.search(claim, limit).map_err(search_error)?;
            let (class, cited, reason) =
                self.classify_one(ctx, &rt, &mut meta, claim, &evidence)?;
            out_claims.push(ClaimOut {
                id: format!("A{}", i + 1),
                text: claim.clone(),
                class,
                reason,
                cited,
                evidence,
            });
        }

        let searched = set.names();
        let vault_incomplete = set.vault_incomplete();
        let vault_files = set.vault.as_ref().map(|v| v.files_scanned).unwrap_or(0);
        // Quellen der Herkunft: die genannten Belege.
        for c in &out_claims {
            for e in c.cited_evidence().take(3) {
                if sources.len() >= 40 {
                    break;
                }
                let kind = if e.source == "vault" { "vault" } else { "rag" };
                if !sources
                    .iter()
                    .any(|s| s.kind == kind && s.reference == e.path)
                {
                    sources.push(SourceRef::new(kind, &e.path, Some(&e.title)));
                }
            }
        }
        record_llm(
            ctx,
            "knowledge_reconcile",
            &meta,
            &sources,
            json!({
                "outcome": "reconciled",
                "claims": out_claims.len(),
                "counts": counts_of(&out_claims),
                "vault_files": vault_files,
                "vault_incomplete": vault_incomplete,
            }),
        );
        let markdown = render_markdown(&out_claims, &searched, vault_incomplete, None);
        Ok(finish(
            &meta,
            "reconciled",
            None,
            &out_claims,
            markdown,
            searched,
            vault_files,
            vault_incomplete,
            sources,
        ))
    }
}
