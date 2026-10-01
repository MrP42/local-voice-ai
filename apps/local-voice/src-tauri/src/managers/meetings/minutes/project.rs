//! G3 (#70, U9 aus #64): Projekt-Protokoll — mehrere Aufnahmen eines Projekts
//! gemeinsam protokollieren oder zusammenfassen.
//!
//! Untermodul von `minutes`, damit es die Mechanik des Einzelprotokolls (Anbieter,
//! JSON-Abfrage mit Retry, Bloecke, Halbieren, Zusammenfuehren, Laufsperre,
//! Fortschritt, Stopp, Fehlercodes) UNVERAENDERT mitbenutzt, ohne dass `minutes.rs`
//! dafuer umgebaut wird (G5 aendert dort gleichzeitig die Erzeugung): in
//! `minutes.rs` steht nur die Zeile `pub mod project;`.
//!
//! Ablauf eines Laufs:
//! (1) Auswahl pruefen und ALLE Aufnahmen laden, bevor ein Modell gefragt wird:
//!     eine Aufnahme ohne Transkript, ausserhalb des Projekts oder noch in
//!     Verarbeitung weist den ganzen Lauf ab (`no_transcript`,
//!     `not_in_project`, `meeting_not_finished`), nichts wird geschrieben;
//! (2) das gemeinsame Transkript (`corpus`): chronologisch, Bloecke je Aufnahme,
//!     jedes Segment mit Quellen-ID (`R2S14`);
//! (3) die Vorlage (Nutzerwahl, "Automatisch" ueber `classify::classify`, sonst die
//!     Standardvorlage); "Protokoll" oder "Zusammenfassung" aendert nur die Regeln im
//!     Prompt und den Titel, nicht die Mechanik;
//! (4) ein Aufruf, wenn alles samt Antwortreserve in den Kontext passt, sonst
//!     Bloecke mit der vorhandenen Halbier-Schleife und ein Zusammenfuehren;
//! (5) Pruefung (`result::assemble`): Belege gegen das Transkript, Markdown,
//!     EIN `INSERT` ins Projekt (`project_minutes_store`), Provenienz.
//!
//! Fehlerfaelle (Auftrag G3) und ihre Absicherung:
//! - Nebenlaeufigkeit: ein Lauf je Projekt (`MinutesRunGuard` unter
//!   `project-minutes:<id>`), der zweite Start bekommt `minutes_busy`
//!   (`a_second_start_for_the_same_project_is_refused_and_the_lock_is_freed`).
//!   Das Projekt oder eine Aufnahme waehrend des Laufs zu loeschen schadet nicht:
//!   die Aufnahmen sind zu Beginn gelesen, der `INSERT` prueft den Ordner
//!   (`a_project_deleted_during_the_run_stores_nothing`).
//! - Abbruch mitten im Vorgang (Stopp, Zeitlimit, verworfenes Future): geschrieben
//!   wird erst am Ende, die Sperre haengt am Guard und wird immer frei
//!   (`a_stop_ends_the_run_without_writing`, `a_hanging_server_ends_in_the_time_limit`).
//! - Speicher voll: RAM-Fehler und knapper RAM enden als `memory_low` ohne weiteren
//!   Versuch (`low_memory_ends_the_run_as_memory_low`); die Start-Sperre fuer das
//!   Modell bleibt beim vorhandenen Weg (`llm_client`, `process_guard`).
//! - Datentraeger voll: der `INSERT` scheitert als Ganzes (`store_failed`).
//! - Absturz des Modellservers: `llm_failed` ohne Halbierungsschleife
//!   (`is_splittable_error` schliesst Abstuerze aus), nichts geschrieben
//!   (`a_failing_server_ends_the_run_and_stores_nothing`).
//! - Erfundene Belege: werden verworfen und gezaehlt, ein Eintrag ohne gueltigen
//!   Beleg bleibt, ist aber markiert (`result::tests`).
//! - Audio-Echtzeitpfad, Geraete, Kindprozesse: nicht beteiligt (reine
//!   Textverarbeitung nach der Aufnahme); der Modellserver startet ueber den
//!   vorhandenen Weg mit RAM-Tor und Job-Objekt.
//!
//! Datenschutz (D9): weder Transkript noch Protokolltext werden geloggt.

pub mod corpus;
pub mod result;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use log::info;
use serde_json::{json, Map, Value};

use super::{
    ask_options, classify_llm_error, entry_text, no_retry, nullable_string, object_schema,
    template_block, Limits, MinutesError, MinutesPhase, MinutesProgress, MinutesRunGuard,
    ProgressFn, Reporter, base_rules, CODE_CANCELLED,
};
use crate::managers::meetings::llm_call::{
    ask_json, is_splittable_error, is_truncation_error, resolve_provider_coded, retry_chunk,
    should_retry, SemanticRetry,
};
use crate::managers::meetings::notes::blocks::{run_blocks, Step, Work};
use crate::managers::meetings::notes::budget;
use crate::managers::meetings::notes::classify::{self, AutoTemplateInfo, Decision};
use crate::managers::meetings::notes::enhance::{single_pass_budget_chars, TokenPlan};
use crate::managers::meetings::notes::model::{SectionKind, TemplateInfo, TemplateSpec};
use crate::managers::meetings::notes::templates::{
    builtin_id, builtin_templates, is_auto_id, DEFAULT_TEMPLATE_ID,
};
use crate::managers::meetings::project_minutes_store::{
    NewProjectMinutes, ProjectKind, ProjectMinutes, ProjectMinutesMeta,
};
use crate::managers::meetings::speakers::SpeakerDirectory;
use crate::managers::meetings::store::MeetingStore;
use crate::managers::usage::Purpose;
use crate::settings::AppSettings;

use corpus::{
    build_corpus, clean_selection, eligibility, order_chronologically, Corpus, Eligibility,
    RecordingInput, CODE_NOT_IN_PROJECT,
};
use result::{
    assemble, merge_deterministic, partial_line, project_markdown, raw_is_empty, validate_sections,
    MapOutput, RawProject, MAX_SOURCES_PER_ENTRY,
};

/// Alle Codes, die dieses Untermodul zusaetzlich zu `minutes::ALL_CODES` erzeugt;
/// die Oberflaeche uebersetzt sie.
pub const EXTRA_CODES: [&str; 5] = [
    "folder_not_found",
    corpus::CODE_NO_SELECTION,
    corpus::CODE_TOO_MANY,
    CODE_NOT_IN_PROJECT,
    "kind_invalid",
];

/// Der Code einer Fehlermeldung des Projekt-Protokolls (eigene oder die des
/// Einzelprotokolls); Unbekanntes gilt als `llm_failed`.
pub fn error_code(err: &str) -> &'static str {
    EXTRA_CODES
        .iter()
        .find(|code| err == **code || err.strip_prefix(**code).is_some_and(|r| r.starts_with(':')))
        .copied()
        .unwrap_or_else(|| super::error_code(err))
}

/// Unter diesem Schluessel steht der Lauf eines Projekts in der Laufsperre
/// (`MinutesRunGuard`) und im Auftragsverzeichnis (`job`). Praefix, damit er nie
/// mit der ID einer Besprechung (ULID) zusammenfaellt.
pub fn run_key(folder_id: &str) -> String {
    format!("project-minutes:{folder_id}")
}

/// Was ein Lauf wissen muss.
pub struct Request<'a> {
    pub folder_id: &'a str,
    pub meeting_ids: &'a [String],
    /// Vorlage, `"auto"` oder `None` (Standardvorlage).
    pub template_id: Option<&'a str>,
    pub kind: ProjectKind,
}

// ---------------------------------------------------------------------------
// Prompts
// ---------------------------------------------------------------------------

const PROJECT_RULES: &str = "\
- The transcript holds several recordings R1, R2, ... in chronological order (R1 is the \
oldest). Each recording starts with a header line \"=== R2 · title · date · duration ===\" \
(not citable). Every other line starts with its source id, e.g. R2S14.\n\
- Every entry carries \"sources\": the ids of the 1 to 4 transcript lines that support it, \
copied exactly as they appear (e.g. \"R2S14\"). Never invent an id. An entry that combines \
statements from several recordings cites at least one line from each.\n\
- When a later recording changes, corrects or settles something from an earlier one, state \
the current status and mention the change instead of listing both as if both still held.\n\
- The project facts (titles, dates, durations) are computed: restate them, never recompute.\n";

const SUMMARY_RULES: &str = "\
- This is a SUMMARY, not full minutes: merge related points across the recordings into few, \
general statements (at most 5 entries per section), keep decisions and action items, and \
leave out detail that does not change the picture.\n";

fn kind_rules(kind: ProjectKind) -> &'static str {
    match kind {
        ProjectKind::Minutes => "",
        ProjectKind::Summary => SUMMARY_RULES,
    }
}

/// System-Prompt des Einzeldurchlaufs und des Zusammenfuehrens.
pub fn system_prompt(kind: ProjectKind) -> String {
    let what = match kind {
        ProjectKind::Minutes => "minutes",
        ProjectKind::Summary => "summary",
    };
    let rules = base_rules();
    format!(
        "You are a meeting-minutes writer for a PROJECT. You combine the transcripts of several \
recordings of one project into ONE set of {what} that follows a template.\n\
- Use exactly the section ids of the template as JSON keys and follow each section's \
instruction.\n\
- A section is an array of entries {{\"text\",\"sources\"}}; an action-items section's entries \
are {{\"text\",\"assignee\",\"due\",\"sources\"}}.\n{PROJECT_RULES}{}{rules}",
        kind_rules(kind)
    )
}

/// System-Prompt der map-Stufe (ein Teil des langen Transkripts).
fn map_system_prompt(kind: ProjectKind) -> String {
    let rules = base_rules();
    format!(
        "You extract minutes entries from ONE PART of the long transcript of several recordings \
of one project, for a template with sections.\n\
- Reply with {{\"entries\":[...]}}; every entry names its section id and carries \"text\", \
\"sources\", \"assignee\" and \"due\" (null for anything that is not an action item or not \
named).\n\
- Follow each section's instruction, but only for what THIS part contains.\n{PROJECT_RULES}{}\
{rules}",
        kind_rules(kind)
    )
}

/// Nutzertext des Einzeldurchlaufs.
pub fn user_prompt(facts: &str, spec: &TemplateSpec, transcript: &str) -> String {
    format!(
        "{facts}\nThe speaker labels below are channel labels, not names. Do not invent \
         participants, numbers, dates or decisions that the transcript does not contain.\n\n\
         {}\n# Transcript\n{transcript}",
        template_block(spec),
    )
}

/// Eintragsgrenze eines Teils: lokal immer, sonst erst nach einem Abschneiden
/// (`minutes::Ctx::entry_cap`). Eine Zusammenfassung braucht weniger Eintraege.
fn entry_cap_for(kind: ProjectKind, chars: usize) -> usize {
    let cap = budget::minutes_entry_cap(chars);
    match kind {
        ProjectKind::Minutes => cap,
        ProjectKind::Summary => (cap / 3).max(4),
    }
}

fn map_prompt(
    facts: &str,
    spec: &TemplateSpec,
    label: &str,
    total: usize,
    max_entries: Option<usize>,
    chunk: &str,
) -> String {
    let cap = max_entries
        .map(|n| {
            format!(
                " Write at most {n} entries in total for this part (all sections together): \
merge related points into one entry and keep the most important."
            )
        })
        .unwrap_or_default();
    format!(
        "{facts}\n{}\nThis is part {label} of {total} of one long project transcript. Extract \
entries only from THIS part; do not summarize everything and do not invent anything that is not \
in this part. Cite only ids that appear in this part.{cap}\n\n# Transcript (part {label})\n{chunk}",
        template_block(spec),
    )
}

fn reduce_prompt(
    facts: &str,
    spec: &TemplateSpec,
    lines: &[String],
    gaps: &[String],
    max_entries: Option<usize>,
) -> String {
    let cap = max_entries
        .map(|n| {
            format!(
                " The final minutes hold at most {n} entries in total: merge related lines \
and drop the least important."
            )
        })
        .unwrap_or_default();
    let gap = if gaps.is_empty() {
        String::new()
    } else {
        format!(
            "\nNote: the transcript part(s) at {} could not be processed and are missing \
below. Merge only what is present and do not pretend the recordings had no other content; \
mention the gap in an open-questions section if the template has one.\n",
            gaps.join("; ")
        )
    };
    format!(
        "{facts}\n{}\n# Partial results\nThe transcripts were too long for one pass, so \
consecutive parts were analysed separately. Each line reads `section | text | sources: ids` \
(action items: `section | text | assignee | due | sources: ids`). Merge the lines into the \
final minutes with the same sections: deduplicate, keep the best wording, later information \
wins over earlier when they contradict. Keep the source ids of every line you merge (give \
the union) and add nothing that is not in the lines.{cap}\n{gap}\n{}",
        template_block(spec),
        lines.join("\n"),
    )
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// Belege eines Eintrags. Nur lokal (llama-server-Grammatik): Muster und
/// Obergrenze; Cloud-strict (OpenAI) lehnt sie ab (wie bei den KI-Notizen).
fn sources_schema(local: bool) -> Value {
    let mut items = json!({ "type": "string" });
    let mut array = json!({ "type": "array" });
    if local {
        items["pattern"] = json!("^R[0-9]+S[0-9]+$");
        array["maxItems"] = json!(MAX_SOURCES_PER_ENTRY);
    }
    array["items"] = items;
    array
}

fn entry_schema(kind: SectionKind, local: bool) -> Value {
    let mut props = Map::new();
    props.insert("text".into(), entry_text(local));
    if kind == SectionKind::Tasks {
        props.insert("assignee".into(), nullable_string());
        props.insert("due".into(), nullable_string());
    }
    props.insert("sources".into(), sources_schema(local));
    object_schema(props)
}

/// Endschema: ein Objekt, dessen Schluessel die Abschnitts-IDs der Vorlage sind;
/// jeder Abschnitt eine Liste von Eintraegen mit Belegen. Strict: alle Felder
/// Pflicht, keine Zusatzfelder.
pub fn project_schema(spec: &TemplateSpec, local: bool) -> Value {
    let mut sections = Map::new();
    for section in &spec.sections {
        sections.insert(
            section.id.clone(),
            json!({ "type": "array", "items": entry_schema(section.kind, local) }),
        );
    }
    object_schema(sections)
}

/// Schema der map-Stufe: eine flache Liste mit Abschnitts-ID je Eintrag.
fn map_schema(spec: &TemplateSpec, local: bool) -> Value {
    let ids: Vec<Value> = spec.sections.iter().map(|s| json!(s.id)).collect();
    let mut props = Map::new();
    props.insert("section".into(), json!({ "type": "string", "enum": ids }));
    props.insert("text".into(), entry_text(local));
    props.insert("assignee".into(), nullable_string());
    props.insert("due".into(), nullable_string());
    props.insert("sources".into(), sources_schema(local));
    let mut top = Map::new();
    top.insert(
        "entries".into(),
        json!({ "type": "array", "items": object_schema(props) }),
    );
    object_schema(top)
}

/// Gueltiges JSON ohne einen einzigen Eintrag: einmal mit Hinweis nachfragen.
fn empty_answer_retry(raw: &RawProject) -> Option<SemanticRetry> {
    raw_is_empty(raw).then(|| SemanticRetry {
        reason: "Projekt-Protokoll ohne Eintraege".to_string(),
        hint: "Your previous reply contained no entries. Write the minutes now: fill the \
               sections of the template with what the transcripts contain, in the same \
               language as the transcripts, and cite source ids. Reply with ONLY the JSON \
               object."
            .to_string(),
    })
}

// ---------------------------------------------------------------------------
// Lauf
// ---------------------------------------------------------------------------

struct PCtx<'a> {
    settings: &'a AppSettings,
    kind: ProjectKind,
    /// Projekt und Aufnahmen als Fakten fuer jeden Prompt.
    facts: String,
    spec: TemplateSpec,
    corpus: Corpus,
    limits: Limits,
    tokens: Option<TokenPlan>,
    entry_cap: bool,
    cancel: Arc<AtomicBool>,
    report: Reporter<'a>,
}

impl PCtx<'_> {
    /// Stopp des Nutzers: VOR jedem Modellaufruf zu pruefen.
    fn check_cancel(&self) -> Result<(), MinutesError> {
        if self.cancel.load(Ordering::Acquire) {
            Err(MinutesError::code_only(CODE_CANCELLED))
        } else {
            Ok(())
        }
    }

    fn free_mb(&self) -> u64 {
        (self.limits.free_mb)()
    }
}

async fn single_pass(ctx: &PCtx<'_>) -> Result<RawProject, String> {
    let prompt = user_prompt(&ctx.facts, &ctx.spec, &ctx.corpus.transcript());
    ask_json::<RawProject>(
        ctx.settings,
        &ask_options(),
        &system_prompt(ctx.kind),
        &|local| project_schema(&ctx.spec, local),
        &prompt,
        &empty_answer_retry,
    )
    .await
}

/// Passt alles in einen Aufruf? Lokal mit exakter Messung der fertigen Prompts,
/// sonst die Zeichen gegen das Budget (wie `minutes::fits_single_pass`).
async fn fits_single_pass(ctx: &PCtx<'_>, payload_chars: usize, budget_chars: usize) -> bool {
    match ctx.tokens.as_ref().filter(|plan| plan.is_exact()) {
        Some(plan) => {
            let prompt = user_prompt(&ctx.facts, &ctx.spec, &ctx.corpus.transcript());
            let tokens = plan.prompt_tokens(&system_prompt(ctx.kind), &prompt).await;
            let fits = plan.budget.fits(tokens);
            log::info!(
                "Projekt-Protokoll: Einzeldurchlauf-Prompt {tokens} Token (gemessen), Kontext {}, Antwortreserve {} -> {}",
                plan.budget.context,
                plan.budget.reserve,
                if fits { "Einzeldurchlauf" } else { "Bloecke" }
            );
            fits
        }
        None => payload_chars <= budget_chars,
    }
}

/// Zeichen je Block der Planung (wie `minutes::plan_block_chars`, mit den
/// Prompts des Projekt-Protokolls). `force_split`: mindestens zwei Bloecke.
async fn plan_block_chars(
    ctx: &PCtx<'_>,
    budget_chars: usize,
    force_split: bool,
    line_chars: &[usize],
) -> usize {
    let total: usize = line_chars.iter().sum();
    let longest = line_chars.iter().copied().max().unwrap_or(0);
    let template_chars = template_block(&ctx.spec).chars().count();
    let facts_chars = ctx.facts.chars().count();
    let by_chars = budget_chars
        .saturating_sub(template_chars + facts_chars)
        .max(200);
    let mut max_block = match ctx.tokens.as_ref() {
        Some(plan) if !plan.is_exact() => {
            let fixed =
                plan.budget.overhead + plan.budget.estimate_tokens(template_chars + facts_chars);
            plan.budget.block_room_tokens(fixed) * plan.budget.cpt_x100 / 100
        }
        Some(plan) => {
            let fixed = plan
                .prompt_tokens(
                    &map_system_prompt(ctx.kind),
                    &map_prompt(
                        &ctx.facts,
                        &ctx.spec,
                        "1",
                        1,
                        Some(budget::MINUTES_MAX_ENTRIES),
                        "",
                    ),
                )
                .await;
            let room = plan.budget.block_room_tokens(fixed);
            let transcript = ctx.corpus.transcript();
            let transcript_chars = transcript.chars().count();
            let tokens = plan.text_tokens(&transcript).await;
            let chars = budget::block_chars_from_measure(room, transcript_chars, tokens);
            log::info!(
                "Projekt-Protokoll: Blockgroesse aus Messung: Prompt-Rahmen {fixed} Token, Platz {room} Token, Transkript {transcript_chars} Zeichen = {tokens} Token -> {chars} Zeichen je Block"
            );
            chars
        }
        None => by_chars,
    };
    if force_split {
        max_block = max_block.min(total.div_ceil(2) + longest);
    }
    budget::balanced_block_limit(total, max_block.max(200), longest)
}

/// Ein map-Aufruf fuer `work` (wie `minutes::map_step`): zu gross oder ungueltig
/// heisst halbieren, nie wiederholen und nie verwerfen; RAM-Fehler brechen ab.
async fn map_step(
    ctx: &PCtx<'_>,
    work: Work,
    total_blocks: usize,
) -> Step<Vec<crate::managers::meetings::notes::assemble::RawEntry>, MinutesError> {
    let label = work.label();
    let chunk = ctx.corpus.chunk(&work.range);
    let prompt = map_prompt(
        &ctx.facts,
        &ctx.spec,
        &label,
        total_blocks,
        (ctx.entry_cap || !work.path.is_empty())
            .then(|| entry_cap_for(ctx.kind, chunk.chars().count())),
        &chunk,
    );
    let system = map_system_prompt(ctx.kind);
    if work.can_split() {
        if let Some(plan) = ctx.tokens.as_ref().filter(|plan| plan.is_exact()) {
            let tokens = plan.prompt_tokens(&system, &prompt).await;
            if !plan.budget.fits(tokens) {
                log::warn!(
                    "Projekt-Protokoll: Block {label}/{total_blocks}: Prompt {tokens} Token laesst nicht genug Platz fuer die Antwort -- wird halbiert"
                );
                return Step::Split;
            }
        }
    }
    let (settings, spec, prompt_ref, system_ref) = (ctx.settings, &ctx.spec, &prompt, &system);
    let result = retry_chunk(
        "Projekt-Protokoll",
        work.block_index,
        total_blocks,
        |e| !is_splittable_error(e) && should_retry(e, ctx.free_mb()),
        move || async move {
            ask_json::<MapOutput>(
                settings,
                &ask_options(),
                system_ref,
                &|local| map_schema(spec, local),
                prompt_ref,
                &no_retry::<MapOutput>,
            )
            .await
        },
    )
    .await;
    match result {
        Ok(output) => Step::Done(output.entries),
        Err(e) => {
            if !should_retry(&e, ctx.free_mb()) {
                return Step::Abort(MinutesError::new("memory_low", e));
            }
            if work.can_split() && is_splittable_error(&e) {
                log::warn!(
                    "Projekt-Protokoll: Block {label}/{total_blocks} zu gross -- wird halbiert"
                );
                Step::Split
            } else {
                log::warn!(
                    "Projekt-Protokoll: Block {label}/{total_blocks} endgueltig nicht ausgewertet ({e}) -- das Protokoll wird unvollstaendig"
                );
                Step::Failed(e)
            }
        }
    }
}

struct BlocksResult {
    raw: RawProject,
    chunks_total: u32,
    chunks_failed: Vec<u32>,
    gaps: Vec<String>,
    chunks_split: u32,
}

/// Bloecke, Halbieren, Zusammenfuehren. Nichts wird verworfen: was nicht geht,
/// steht als Luecke (`gaps`) im Ergebnis.
async fn write_blocks(
    ctx: &PCtx<'_>,
    budget_chars: usize,
    force_split: bool,
) -> Result<BlocksResult, MinutesError> {
    let line_chars = ctx.corpus.line_chars();
    let block_chars = plan_block_chars(ctx, budget_chars, force_split, &line_chars).await;
    let ranges = ctx.corpus.pack(block_chars);
    let total_blocks = ranges.len();
    let total_steps = (total_blocks + 1) as u32;
    log::info!(
        "Projekt-Protokoll: {total_blocks} Bloecke (bis {block_chars} Zeichen je Block) aus {} Aufnahmen",
        ctx.corpus.recordings.len()
    );
    (ctx.report)(MinutesPhase::Write, 0, total_steps);

    let outcome = run_blocks(
        &ranges,
        &line_chars,
        || ctx.check_cancel(),
        |done| (ctx.report)(MinutesPhase::Write, done as u32, total_steps),
        |work| map_step(ctx, work, total_blocks),
    )
    .await?;

    let failed = outcome.failed_numbers();
    let leaves_total = outcome.leaves.len();
    if failed.len() == leaves_total {
        return Err(MinutesError::new(
            "llm_failed",
            format!("kein einziger der {leaves_total} Transkriptbloecke konnte ausgewertet werden"),
        ));
    }
    let gaps: Vec<String> = outcome
        .gap_ranges()
        .iter()
        .flat_map(|range| ctx.corpus.gap_labels(range))
        .collect();
    let chunks_split = outcome.splits;
    let collected: Vec<_> = outcome
        .leaves
        .into_iter()
        .filter_map(|leaf| leaf.value)
        .flatten()
        .collect();

    ctx.check_cancel()?;
    (ctx.report)(MinutesPhase::Merge, total_blocks as u32, total_steps);
    let lines: Vec<String> = collected
        .iter()
        .filter_map(|entry| partial_line(entry, &ctx.spec, &ctx.corpus))
        .collect();
    let transcript_chars: usize = line_chars.iter().sum();
    let reduce_prompt_text = if lines.is_empty() {
        None
    } else {
        let cap = match ctx.kind {
            ProjectKind::Minutes => ctx
                .entry_cap
                .then(|| budget::minutes_entry_cap(transcript_chars)),
            // Eine Zusammenfassung ist immer knapp, auch bei entfernten Anbietern.
            ProjectKind::Summary => Some(entry_cap_for(ctx.kind, transcript_chars)),
        };
        let prompt = reduce_prompt(&ctx.facts, &ctx.spec, &lines, &gaps, cap);
        let fits = match ctx.tokens.as_ref().filter(|plan| plan.is_exact()) {
            Some(plan) => {
                let tokens = plan.prompt_tokens(&system_prompt(ctx.kind), &prompt).await;
                plan.budget.fits(tokens)
            }
            None => prompt.chars().count() <= budget_chars,
        };
        fits.then_some(prompt)
    };
    let raw = match reduce_prompt_text {
        None => {
            log::info!(
                "Projekt-Protokoll: Zusammenfuehren uebersprungen ({} Zeilen passen nicht ins Budget oder fehlen) -- Eintraege bleiben in Blockreihenfolge",
                lines.len()
            );
            merge_deterministic(&collected, &ctx.spec)
        }
        Some(prompt) => {
            let (settings, spec, prompt_ref, kind) = (ctx.settings, &ctx.spec, &prompt, ctx.kind);
            let result = retry_chunk(
                "Projekt-Protokoll (Zusammenfuehren)",
                total_blocks,
                total_blocks + 1,
                |e| !is_truncation_error(e) && should_retry(e, ctx.free_mb()),
                move || async move {
                    ask_json::<RawProject>(
                        settings,
                        &ask_options(),
                        &system_prompt(kind),
                        &|local| project_schema(spec, local),
                        prompt_ref,
                        &empty_answer_retry,
                    )
                    .await
                },
            )
            .await;
            match result {
                Ok(raw) if !raw_is_empty(&raw) => raw,
                Ok(_) => {
                    log::warn!(
                        "Projekt-Protokoll: Zusammenfuehren ohne Eintraege -- Eintraege der Bloecke"
                    );
                    merge_deterministic(&collected, &ctx.spec)
                }
                Err(e) => {
                    if !should_retry(&e, ctx.free_mb()) {
                        return Err(MinutesError::new("memory_low", e));
                    }
                    log::warn!(
                        "Projekt-Protokoll: Zusammenfuehren fehlgeschlagen -- Eintraege der Bloecke"
                    );
                    merge_deterministic(&collected, &ctx.spec)
                }
            }
        }
    };
    (ctx.report)(MinutesPhase::Merge, total_steps, total_steps);
    Ok(BlocksResult {
        raw,
        chunks_total: leaves_total as u32,
        chunks_failed: failed,
        gaps,
        chunks_split,
    })
}

// ---------------------------------------------------------------------------
// Auswahl laden
// ---------------------------------------------------------------------------

fn store_err(err: impl std::fmt::Display) -> MinutesError {
    MinutesError::new("store_failed", err.to_string())
}

/// Eine Aufnahme der Auswahl in der Liste der Oberflaeche.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ProjectCandidate {
    pub meeting_id: String,
    /// Waehlbar? Sonst `reason`.
    pub eligible: bool,
    /// `meeting_not_finished` | `no_transcript` | `empty_entry`.
    pub reason: Option<String>,
    /// Zahl der Segmente mit Text.
    pub segments: u32,
}

/// Alle Aufnahmen eines Projekts mit der Auskunft, ob sie in ein Projekt-Protokoll
/// eingehen koennen (chronologisch). Gleiche Pruefung wie im Lauf.
pub fn candidates(
    store: &MeetingStore,
    folder_id: &str,
) -> Result<Vec<ProjectCandidate>, MinutesError> {
    let meetings = store.project_meetings(folder_id).map_err(|e| {
        if e.to_string() == "folder_not_found" {
            MinutesError::code_only("folder_not_found")
        } else {
            store_err(e)
        }
    })?;
    meetings
        .into_iter()
        .map(|meeting| {
            let segments = store.get_segments(&meeting.id).map_err(store_err)?;
            let verdict = eligibility(&meeting, &segments);
            Ok(ProjectCandidate {
                eligible: verdict == Eligibility::Ok,
                reason: verdict.code().map(str::to_string),
                segments: segments
                    .iter()
                    .filter(|s| !s.text.trim().is_empty())
                    .count() as u32,
                meeting_id: meeting.id,
            })
        })
        .collect()
}

/// Die Aufnahmen der Auswahl laden und pruefen, in chronologischer Reihenfolge.
/// Jede Abweisung gilt fuer den ganzen Lauf (nichts wird teilweise gemacht).
fn load_recordings(
    store: &MeetingStore,
    folder_id: &str,
    meeting_ids: &[String],
) -> Result<Vec<RecordingInput>, MinutesError> {
    let ids = clean_selection(meeting_ids)?;
    let members: std::collections::HashSet<String> = store
        .project_meetings(folder_id)
        .map_err(|e| {
            if e.to_string() == "folder_not_found" {
                MinutesError::code_only("folder_not_found")
            } else {
                store_err(e)
            }
        })?
        .into_iter()
        .map(|m| m.id)
        .collect();
    let mut inputs = Vec::with_capacity(ids.len());
    for id in &ids {
        let meeting = store
            .get_meeting(id)
            .map_err(store_err)?
            .ok_or_else(|| MinutesError::new("meeting_not_found", id.clone()))?;
        if !members.contains(id) {
            return Err(MinutesError::new(CODE_NOT_IN_PROJECT, id.clone()));
        }
        let segments = store.get_segments(id).map_err(store_err)?;
        match eligibility(&meeting, &segments) {
            Eligibility::Ok => {}
            Eligibility::NotFinished => {
                return Err(MinutesError::new("meeting_not_finished", id.clone()))
            }
            Eligibility::NoTranscript | Eligibility::EmptyEntry => {
                return Err(MinutesError::new("no_transcript", id.clone()))
            }
        }
        let labels = SpeakerDirectory::load(store, id);
        inputs.push(RecordingInput::new(meeting, &segments, labels));
    }
    order_chronologically(&mut inputs);
    Ok(inputs)
}

// ---------------------------------------------------------------------------
// Vorlage
// ---------------------------------------------------------------------------

fn default_template(store: &MeetingStore) -> Result<TemplateInfo, MinutesError> {
    if let Some(info) = store
        .get_template_info(DEFAULT_TEMPLATE_ID)
        .map_err(store_err)?
    {
        return Ok(info);
    }
    builtin_templates()
        .into_iter()
        .find(|(key, _, _)| builtin_id(key) == DEFAULT_TEMPLATE_ID)
        .map(|(_, title, spec)| TemplateInfo {
            id: DEFAULT_TEMPLATE_ID.to_string(),
            title: title.to_string(),
            builtin: true,
            spec,
            updated_at: 0,
        })
        .ok_or_else(|| MinutesError::code_only("template_not_found"))
}

/// Laengster Auszug fuer die automatische Vorlagenwahl (Zeichen).
const AUTO_EXCERPT_CHARS: usize = 4_000;

/// Die Vorlage des Laufs: Nutzerwahl vor Automatik. "Automatisch" waehlt nach dem
/// Inhalt ALLER Aufnahmen (Auszug je Aufnahme); die Wahl wird nicht gespeichert
/// (ein Projekt-Protokoll gehoert keiner einzelnen Besprechung).
async fn resolve_template(
    settings: &AppSettings,
    store: &MeetingStore,
    explicit: Option<&str>,
    project: &str,
    inputs: &[RecordingInput],
    limits: &Limits,
) -> Result<(TemplateInfo, Option<Decision>), MinutesError> {
    let explicit = explicit.map(str::trim).filter(|id| !id.is_empty());
    let Some(id) = explicit else {
        return Ok((default_template(store)?, None));
    };
    if !is_auto_id(id) {
        let info = store
            .get_template_info(id)
            .map_err(store_err)?
            .ok_or_else(|| MinutesError::code_only("template_not_found"))?;
        return Ok((info, None));
    }
    let templates = store.list_template_infos().map_err(store_err)?;
    let candidates = classify::candidates_from(&templates);
    let mut excerpt = String::new();
    for input in inputs {
        let lines = classify::excerpt_lines(&input.segments, &input.labels);
        let part = classify::build_excerpt(&lines);
        if !part.is_empty() {
            if !excerpt.is_empty() {
                excerpt.push_str("\n---\n");
            }
            excerpt.push_str(&part);
        }
        if excerpt.chars().count() >= AUTO_EXCERPT_CHARS {
            break;
        }
    }
    let excerpt: String = excerpt.chars().take(AUTO_EXCERPT_CHARS).collect();
    let decision = classify::classify(
        settings,
        Purpose::Minutes,
        project,
        &candidates,
        &excerpt,
        limits.classify_timeout,
    )
    .await;
    let info = match templates.into_iter().find(|t| t.id == decision.template_id) {
        Some(info) => info,
        None => default_template(store)?,
    };
    Ok((info, Some(decision)))
}

// ---------------------------------------------------------------------------
// Der Lauf
// ---------------------------------------------------------------------------

async fn run_project_minutes(
    settings: &AppSettings,
    store: Arc<MeetingStore>,
    request: &Request<'_>,
    limits: Limits,
    guard: &MinutesRunGuard,
    on_progress: ProgressFn<'_>,
) -> Result<ProjectMinutes, MinutesError> {
    let started = std::time::Instant::now();
    let project = store
        .folders_list()
        .map_err(store_err)?
        .into_iter()
        .find(|f| f.id == request.folder_id)
        .ok_or_else(|| MinutesError::code_only("folder_not_found"))?;
    let inputs = load_recordings(&store, request.folder_id, request.meeting_ids)?;
    let (provider, model, _) =
        resolve_provider_coded(settings).map_err(|e| MinutesError::new(e.code, e.message))?;
    let local = crate::managers::llm::is_local(&provider);

    let report = |phase: MinutesPhase, done: u32, total: u32| {
        let progress = MinutesProgress { phase, done, total };
        guard.set_progress(progress.clone());
        on_progress(&progress);
    };
    let cancel = guard.cancel_token();
    let stopped = || {
        if cancel.load(Ordering::Acquire) {
            Err(MinutesError::code_only(CODE_CANCELLED))
        } else {
            Ok(())
        }
    };

    stopped()?;
    report(MinutesPhase::Template, 0, 0);
    // Ein Stopp, der mit der Meldung eintrifft, gilt vor dem Klassifizieren (ein
    // Modellaufruf): VOR jedem Modellaufruf pruefen.
    stopped()?;
    let (info, auto) = resolve_template(
        settings,
        &store,
        request.template_id,
        &project.name,
        &inputs,
        &limits,
    )
    .await?;
    stopped()?;

    let tokens = if local && limits.budget_chars.is_none() {
        Some(TokenPlan::for_local(&model).await)
    } else {
        None
    };
    let corpus = build_corpus(&inputs);
    let descriptions: Vec<String> = inputs
        .iter()
        .map(|i| i.meeting.description.clone().unwrap_or_default())
        .collect();
    let ctx = PCtx {
        settings,
        kind: request.kind,
        facts: corpus.facts_block(&project.name, &descriptions),
        spec: info.spec.clone(),
        corpus,
        limits,
        tokens,
        entry_cap: local,
        cancel: guard.cancel_token(),
        report: &report,
    };
    let budget_chars = match (limits.budget_chars, &ctx.tokens) {
        (Some(chars), _) => chars,
        (None, Some(plan)) => plan.budget.payload_chars(),
        (None, None) => single_pass_budget_chars(&model, local).await,
    };
    let payload = ctx.corpus.transcript().chars().count()
        + template_block(&ctx.spec).chars().count()
        + ctx.facts.chars().count();
    log::info!(
        "Projekt-Protokoll: {} Aufnahmen, {} Zeilen, {payload} Zeichen (Einzeldurchlauf bis {budget_chars}), Vorlage {}",
        ctx.corpus.recordings.len(),
        ctx.corpus.lines.len(),
        info.id
    );

    let single_fits = fits_single_pass(&ctx, payload, budget_chars).await;
    let mut alone: Option<RawProject> = None;
    if single_fits {
        stopped()?;
        report(MinutesPhase::Write, 0, 1);
        match single_pass(&ctx).await {
            Ok(raw) => {
                report(MinutesPhase::Write, 1, 1);
                alone = Some(raw);
            }
            // Abgeschnitten: die Antwort war fuer den Kontext zu gross -> Bloecke.
            Err(e) if is_truncation_error(&e) => {
                log::warn!("Projekt-Protokoll: Einzeldurchlauf nicht auswertbar (Antwort zu lang) -- weiter in Bloecken");
            }
            Err(e) => return Err(classify_llm_error(e, ctx.free_mb())),
        }
    }
    let (raw, blocks) = match alone {
        Some(raw) => (raw, None),
        None => {
            let mut result = write_blocks(&ctx, budget_chars, single_fits).await?;
            (std::mem::take(&mut result.raw), Some(result))
        }
    };

    let (sections, stats) = assemble(raw, &ctx.spec, &ctx.corpus);
    validate_sections(&sections).map_err(|e| MinutesError::new("llm_failed", e))?;
    if stats.dropped_unknown > 0 || stats.dropped_sources > 0 {
        log::warn!(
            "Projekt-Protokoll: {} Eintraege unter unbekannten Abschnitten und {} erfundene Belege verworfen",
            stats.dropped_unknown,
            stats.dropped_sources
        );
    }
    let gaps: Vec<String> = blocks.as_ref().map(|b| b.gaps.clone()).unwrap_or_default();
    let gap_count = gaps.len();
    let auto_info = auto.as_ref().map(|d| AutoTemplateInfo {
        template_id: d.template_id.clone(),
        title: info.title.clone(),
        reason: d.reason.clone(),
        outcome: d.outcome,
    });
    let body = project_markdown(
        request.kind,
        &project.name,
        &info.title,
        auto.as_ref().map(|d| d.outcome),
        &ctx.corpus.recordings,
        &sections,
        &gaps,
    );
    let meta = ProjectMinutesMeta {
        model: model.clone(),
        provider: provider.id.clone(),
        template_id: info.id.clone(),
        template_title: info.title.clone(),
        auto: auto_info,
        single_pass: blocks.is_none(),
        chunks_total: blocks.as_ref().map(|b| b.chunks_total).unwrap_or(1),
        chunks_split: blocks.as_ref().map(|b| b.chunks_split).unwrap_or(0),
        incomplete: !gaps.is_empty(),
        gaps,
        dropped_sources: stats.dropped_sources as u32,
        unsupported_entries: stats.unsupported as u32,
    };
    // Letzte Gelegenheit zum Stoppen; danach schreibt der Lauf mit EINEM INSERT.
    stopped()?;
    let id = ulid::Ulid::new().to_string();
    let recordings = ctx.corpus.recordings.clone();
    store
        .project_minutes_insert(&NewProjectMinutes {
            id: id.clone(),
            folder_id: project.id.clone(),
            kind: request.kind,
            title: result::document_title(request.kind, &project.name),
            body,
            sections: sections.clone(),
            recordings: recordings.clone(),
            meta,
        })
        .map_err(|e| {
            if e.to_string() == "folder_not_found" {
                MinutesError::code_only("folder_not_found")
            } else {
                store_err(e)
            }
        })?;
    // Herkunft (Modell, Token, Dauer, Quellaufnahmen). Scheitert das Schreiben,
    // bleibt das Protokoll gueltig; die Herkunft steht auch in `meta`.
    crate::managers::provenance::generation::record_generation(
        &store,
        crate::managers::provenance::generation::Generation {
            subject_kind: crate::managers::provenance::SubjectKind::Document,
            subject_id: &id,
            subject_revision: None,
            operation: "project_minutes",
            actor_kind: crate::managers::provenance::ActorKind::User,
            actor_ref: None,
            started,
            sources: recordings
                .iter()
                .map(|r| {
                    crate::managers::provenance::SourceRef::new(
                        "transcript",
                        &r.meeting_id,
                        Some(&r.title),
                    )
                })
                .collect(),
            params: json!({
                "kind": request.kind.as_str(),
                "folder_id": project.id,
                "template_id": info.id,
                "auto_template": auto.is_some(),
                "recordings": recordings.len(),
                "single_pass": blocks.is_none(),
                "chunks_total": blocks.as_ref().map(|b| b.chunks_total).unwrap_or(1),
                "chunks_failed": blocks.as_ref().map(|b| b.chunks_failed.len()).unwrap_or(0),
            }),
            fallback: Some(crate::managers::provenance::generation::Fallback {
                provider: &provider,
                model: &model,
            }),
        },
    );
    info!(
        "Projekt-Protokoll: {} Abschnitte, {} Eintraege, {} Luecken, {} ohne Beleg",
        sections.len(),
        sections.iter().map(|s| s.entries.len()).sum::<usize>(),
        gap_count,
        stats.unsupported
    );
    store
        .project_minutes_get(&id)
        .map_err(store_err)?
        .ok_or_else(|| {
            MinutesError::new(
                "store_failed",
                "Projekt-Protokoll gespeichert, aber nicht lesbar",
            )
        })
}

/// Sperre nehmen, Zeitlimit setzen, laufen lassen. Die Sperre haengt am Lauf
/// (nicht am Future-Inneren): auch bei Zeitlimit und verworfenem Future frei.
pub(crate) async fn generate_guarded(
    settings: &AppSettings,
    store: Arc<MeetingStore>,
    request: &Request<'_>,
    limits: Limits,
    on_progress: ProgressFn<'_>,
) -> Result<ProjectMinutes, MinutesError> {
    let guard = MinutesRunGuard::acquire(&run_key(request.folder_id))?;
    match tokio::time::timeout(
        limits.timeout,
        crate::managers::usage::with_capture(run_project_minutes(
            settings,
            store,
            request,
            limits,
            &guard,
            on_progress,
        )),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(MinutesError::new("llm_failed", "Zeitlimit überschritten")),
    }
}

/// Projekt-Protokoll erzeugen und im Projekt ablegen. Ein Lauf je Projekt; ein
/// zweiter Start bekommt `minutes_busy`.
pub async fn generate_project_minutes(
    settings: &AppSettings,
    store: Arc<MeetingStore>,
    request: &Request<'_>,
    on_progress: ProgressFn<'_>,
) -> Result<ProjectMinutes, String> {
    generate_guarded(settings, store, request, Limits::default(), on_progress)
        .await
        .map_err(String::from)
}

/// Laeuft fuer das Projekt gerade ein Lauf?
pub fn run_state(folder_id: &str) -> super::MinutesRunState {
    super::run_state(&run_key(folder_id))
}

/// Stopp anfordern: `true`, wenn ein Lauf besteht.
pub fn request_cancel(folder_id: &str) -> bool {
    super::request_cancel(&run_key(folder_id))
}

#[cfg(test)]
mod tests;
