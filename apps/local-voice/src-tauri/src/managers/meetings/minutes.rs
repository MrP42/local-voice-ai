//! M8 meetings: Protokoll-Erzeugung.
//!
//! Das Protokoll entsteht in drei getrennten Schritten, damit jeder für sich
//! testbar bleibt: (1) deterministischer Kopf aus Store-Fakten (Titel, Datum,
//! Dauer, Redeanteile), (2) ein LLM-Aufruf mit striktem JSON-Schema, der
//! ausschließlich die inhaltlichen Sektionen füllt, (3) reines Rendering nach
//! Markdown. Zahlen aus Schritt 1 werden dem Modell als Fakten mitgegeben und
//! nie von ihm neu berechnet.
//!
//! Datenschutz (D9): weder Transkript noch Protokolltext werden geloggt —
//! Logzeilen nennen nur Längen, Blockzahlen und Fehlerursachen.

use std::sync::Arc;

use log::info;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::stats::{label_for_channel, speaking_shares, SpeakerShare};
use super::store::{MeetingDocument, MeetingStore, StoredSegment};
use crate::settings::AppSettings;

/// Ab dieser Transkriptlänge läuft die Erzeugung zweistufig (map-reduce).
/// Gleicher Wert wie im Summarizer: auch ein lokales 8k-Modell verkraftet
/// einen Block samt Prompt.
const MAP_REDUCE_CHARS: usize = 16_000;
/// Versuche je Block der map-Stufe (eigenes Budget, unabhaengig vom
/// Struktur-Retry innerhalb eines Versuchs).
const CHUNK_ATTEMPTS: usize = 2;

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct DecisionItem {
    pub text: String,
    pub context: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct TaskItem {
    pub text: String,
    pub assignee: Option<String>,
    pub due: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct OwnedItem {
    pub text: String,
    pub owner: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ReasonedItem {
    pub text: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct TextItem {
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MinutesJson {
    pub summary: String,
    pub scope: String,
    pub decisions: Vec<DecisionItem>,
    pub tasks: Vec<TaskItem>,
    pub next_steps: Vec<OwnedItem>,
    pub follow_ups: Vec<ReasonedItem>,
    pub open_questions: Vec<TextItem>,
}

/// Die deterministisch berechneten Kopfdaten eines Protokolls.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MinutesHead {
    pub title: String,
    pub date_iso: String,
    pub duration_ms: u64,
    pub shares: Vec<SpeakerShare>,
    /// Nur ein Kanal im Transkript: Redeanteile sind dann keine Information,
    /// sondern Rauschen — Tabelle und Validator lassen sie weg.
    pub single_speaker: bool,
    /// Der einzige Kanal ist eine Mischaufnahme (MixedCapture, Kanal 2): ein
    /// Import kann vier Personen enthalten, die alle auf denselben Kanal
    /// laufen. „Ein Kanal" heißt hier ausdrücklich NICHT „ein Sprecher" —
    /// die Unterscheidung steuert die Prompt-Formulierung.
    pub mixed_channel: bool,
}

// -- Formatierung ---------------------------------------------------------

fn mm_ss(ms: u64) -> String {
    let total_seconds = ms / 1_000;
    format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
}

fn duration_label(ms: u64) -> String {
    let total_seconds = ms / 1_000;
    let hours = total_seconds / 3_600;
    if hours > 0 {
        format!(
            "{}:{:02}:{:02}",
            hours,
            (total_seconds % 3_600) / 60,
            total_seconds % 60
        )
    } else {
        mm_ss(ms)
    }
}

/// Prozent in deutscher Schreibweise ("60,0 %") für das Markdown.
fn percent_de(percent: f64) -> String {
    format!("{percent:.1}").replace('.', ",") + " %"
}

// -- Prompt-Bausteine -----------------------------------------------------

/// Transkript für den Prompt: eine Zeile je Segment, mit Kanal-Label und
/// Startzeit. Reihenfolge übernimmt der Aufrufer (siehe `sorted_segments`).
pub fn render_transcript_for_prompt(segments: &[StoredSegment]) -> String {
    segments
        .iter()
        .map(|segment| {
            format!(
                "{} [{}]: {}",
                label_for_channel(segment.channel),
                mm_ss(segment.start_ms),
                segment.text.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Segmente nach Startzeit sortieren. `get_segments` liefert sie in
/// `segment_index`-Reihenfolge, die bei Live-Meetings Mikrofon- und
/// System-Blöcke verschränkt; für das Prompt-Rendering ist die gemeinsame
/// Zeitachse (beide Kanäle starten beim Meeting-Start) die bessere Ordnung.
fn sorted_segments(segments: &[StoredSegment]) -> Vec<StoredSegment> {
    let mut sorted = segments.to_vec();
    sorted.sort_by_key(|segment| segment.start_ms);
    sorted
}

/// Striktes JSON-Schema: alle sieben Sektionen sind Pflicht, Extra-Felder
/// sind verboten, optionale Strings sind explizit nullable (nur so akzeptiert
/// der strict-Modus ein weglassbares Feld).
pub fn minutes_schema() -> serde_json::Value {
    let nullable_string = serde_json::json!({ "type": ["string", "null"] });
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary", "scope", "decisions", "tasks", "next_steps", "follow_ups", "open_questions"],
        "properties": {
            "summary": { "type": "string" },
            "scope": { "type": "string" },
            "decisions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["text", "context"],
                    "properties": { "text": { "type": "string" }, "context": { "type": "string" } }
                }
            },
            "tasks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["text", "assignee", "due"],
                    "properties": {
                        "text": { "type": "string" },
                        "assignee": nullable_string,
                        "due": nullable_string
                    }
                }
            },
            "next_steps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["text", "owner"],
                    "properties": { "text": { "type": "string" }, "owner": nullable_string }
                }
            },
            "follow_ups": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["text", "reason"],
                    "properties": { "text": { "type": "string" }, "reason": { "type": "string" } }
                }
            },
            "open_questions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["text"],
                    "properties": { "text": { "type": "string" } }
                }
            }
        }
    })
}

pub fn minutes_system_prompt() -> String {
    "You are a meeting-minutes writer. You turn a raw meeting transcript into \
     the structured sections of a formal set of minutes.\n\
     Write every field in the SAME language as the transcript.\n\
     Rules:\n\
     - Do not invent participants, numbers, dates or decisions that are not in \
     the transcript. Unclear items belong in open_questions.\n\
     - Only record a decision if the transcript shows it was actually decided; \
     an intention or a proposal is not a decision.\n\
     - Leave assignee, owner or due empty (null) unless the transcript names \
     them explicitly. Never guess a name from the speaker labels.\n\
     - A section with nothing to report stays an empty array. Do not pad it.\n\
     - No meta commentary, no markdown, no headings inside the fields.\n\
     - Reply with ONLY a JSON object that matches the given schema."
        .to_string()
}

fn head_facts_block(head: &MinutesHead) -> String {
    let mut block = format!(
        "# Meeting facts (computed, treat as given — restate them, never recompute)\n\
         Title: {}\nDate: {}\nDuration: {}\n",
        head.title,
        head.date_iso,
        duration_label(head.duration_ms),
    );
    if head.mixed_channel {
        // Eine Mischaufnahme kann beliebig viele Personen enthalten. Dem
        // Modell hier „ein Sprecher" als Fakt zu geben, würde ein Meeting mit
        // vier Personen zum Monolog machen — genau die Halluzination, die der
        // System-Prompt verbietet.
        block.push_str(
            "Speakers: the transcript is a single mixed recording channel; the \
             number of speakers is unknown and speaking shares are not \
             available. Attribute statements only where the transcript itself \
             makes the speaker clear.\n",
        );
    } else if head.single_speaker || head.shares.is_empty() {
        block.push_str("Speakers: a single recorded speaker (no speaking shares).\n");
    } else {
        block.push_str("Speaking shares:\n");
        for share in &head.shares {
            block.push_str(&format!(
                "- {} (channel {}): {:.1} % of the speech time\n",
                share.label, share.channel, share.percent
            ));
        }
    }
    block
}

pub fn minutes_user_prompt(head: &MinutesHead, transcript: &str) -> String {
    format!(
        "{}\nThe speaker labels below are channel labels, not names. Do not \
         invent participants, numbers, dates or decisions that the transcript \
         does not contain; put anything unclear into open_questions.\n\n\
         # Transcript\n{transcript}",
        head_facts_block(head),
    )
}

/// Prompt für einen Teilblock des Transkripts (map-Stufe).
fn chunk_prompt(head: &MinutesHead, index: usize, total: usize, chunk: &str) -> String {
    format!(
        "{}\nThis is part {} of {} of one long transcript. Extract only what \
         THIS part contains; do not summarize the whole meeting yet and do not \
         invent anything that is not in this part.\n\n# Transcript (part {})\n{chunk}",
        head_facts_block(head),
        index + 1,
        total,
        index + 1,
    )
}

/// Prompt der Reduce-Stufe: Zwischenergebnisse zu einem Protokoll verdichten.
/// `missing` nennt die Bloecke (1-basiert), die nicht ausgewertet werden
/// konnten - das Modell soll die Luecke kennen, statt sie zu ueberspielen.
fn merge_prompt(head: &MinutesHead, partials: &[String], missing: &[usize]) -> String {
    let gap_note = if missing.is_empty() {
        String::new()
    } else {
        format!(
            "\nNote: part(s) {} of the transcript could not be processed and are \
             missing below. Merge only what is present and do not pretend the \
             meeting had no other content; mention the gap in open_questions.\n",
            missing
                .iter()
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        )
    };
    format!(
        "{}\nThe following JSON objects are partial minutes of consecutive \
         parts of ONE meeting, in order. Merge them into a single set of \
         minutes with the same structure: one coherent summary and scope, \
         deduplicated lists, later information winning over earlier when they \
         contradict. Add nothing that is not in the parts.\n{}\n{}",
        head_facts_block(head),
        gap_note,
        partials.join("\n\n---\n\n"),
    )
}

/// Hinweiszeile fuer ein Protokoll, dem Transkriptbloecke fehlen.
fn incomplete_note(total: usize, failed: &[usize]) -> String {
    let list = failed
        .iter()
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "\n> **Hinweis:** {} von {} Transkriptblöcken konnten nicht ausgewertet \
         werden (Block {}). Das Protokoll deckt den übrigen Teil der Besprechung \
         ab; das vollständige Transkript bleibt erhalten.\n",
        failed.len(),
        total,
        list,
    )
}

// -- Validierung ----------------------------------------------------------

/// Fachliche Mindestanforderungen an ein erzeugtes Protokoll. Leere Listen
/// sind ausdrücklich zulässig — ein Gespräch ohne Entscheidungen ist normal,
/// ein Protokoll ohne Zusammenfassung nicht.
pub fn validate_minutes(minutes: &MinutesJson, single_speaker: bool) -> Result<(), String> {
    if minutes.summary.trim().is_empty() {
        return Err("Protokoll ohne Zusammenfassung".into());
    }
    // Ein-Sprecher-Aufnahmen (Diktat/Import) haben oft keinen Besprechungs-
    // rahmen; für ein Meeting mit mehreren Kanälen ist der Scope Pflicht.
    if !single_speaker && minutes.scope.trim().is_empty() {
        return Err("Protokoll ohne Scope".into());
    }
    let empty_item = minutes.decisions.iter().any(|i| i.text.trim().is_empty())
        || minutes.tasks.iter().any(|i| i.text.trim().is_empty())
        || minutes.next_steps.iter().any(|i| i.text.trim().is_empty())
        || minutes.follow_ups.iter().any(|i| i.text.trim().is_empty())
        || minutes
            .open_questions
            .iter()
            .any(|i| i.text.trim().is_empty());
    if empty_item {
        return Err("Protokoll enthält einen leeren Listeneintrag".into());
    }
    Ok(())
}

// -- Rendering ------------------------------------------------------------

fn section(markdown: &mut String, heading: &str, lines: Vec<String>) {
    markdown.push_str(&format!("\n## {heading}\n\n"));
    if lines.is_empty() {
        markdown.push_str("_keine_\n");
    } else {
        for line in lines {
            markdown.push_str(&format!("- {line}\n"));
        }
    }
}

pub fn minutes_to_markdown(head: &MinutesHead, minutes: &MinutesJson) -> String {
    let mut markdown = format!("# Protokoll: {}\n\n", head.title);
    markdown.push_str(&format!(
        "**Datum:** {} · **Dauer:** {}\n",
        head.date_iso,
        duration_label(head.duration_ms)
    ));

    markdown.push_str("\n## Zusammenfassung\n\n");
    markdown.push_str(&format!("{}\n", minutes.summary.trim()));

    markdown.push_str("\n## Scope\n\n");
    markdown.push_str(&format!("{}\n", minutes.scope.trim()));

    if !head.single_speaker && !head.shares.is_empty() {
        markdown.push_str("\n## Sprecher & Redeanteile\n\n");
        markdown.push_str("| Sprecher | Redezeit | Anteil |\n|---|---|---|\n");
        for share in &head.shares {
            markdown.push_str(&format!(
                "| {} | {} | {} |\n",
                share.label,
                duration_label(share.speech_ms),
                percent_de(share.percent)
            ));
        }
    }

    section(
        &mut markdown,
        "Entscheidungen",
        minutes
            .decisions
            .iter()
            .map(|d| {
                if d.context.trim().is_empty() {
                    d.text.trim().to_string()
                } else {
                    format!("{} _({})_", d.text.trim(), d.context.trim())
                }
            })
            .collect(),
    );

    section(
        &mut markdown,
        "Aufgaben",
        minutes
            .tasks
            .iter()
            .map(|t| {
                let mut line = t.text.trim().to_string();
                let mut extras = Vec::new();
                if let Some(assignee) = t
                    .assignee
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    extras.push(format!("Wer: {assignee}"));
                }
                if let Some(due) = t.due.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    extras.push(format!("Bis: {due}"));
                }
                if !extras.is_empty() {
                    line.push_str(&format!(" _({})_", extras.join(", ")));
                }
                line
            })
            .collect(),
    );

    section(
        &mut markdown,
        "Next Steps",
        minutes
            .next_steps
            .iter()
            .map(
                |s| match s.owner.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
                    Some(owner) => format!("{} _(Wer: {owner})_", s.text.trim()),
                    None => s.text.trim().to_string(),
                },
            )
            .collect(),
    );

    section(
        &mut markdown,
        "Follow-Ups",
        minutes
            .follow_ups
            .iter()
            .map(|f| {
                if f.reason.trim().is_empty() {
                    f.text.trim().to_string()
                } else {
                    format!("{} _({})_", f.text.trim(), f.reason.trim())
                }
            })
            .collect(),
    );

    section(
        &mut markdown,
        "Offene Fragen",
        minutes
            .open_questions
            .iter()
            .map(|q| q.text.trim().to_string())
            .collect(),
    );

    markdown
}

// -- Erzeugung ------------------------------------------------------------

/// Modelle liefern das JSON gelegentlich in einem Codefence; das kostet einen
/// Retry, den ein Dreizeiler spart.
fn strip_code_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let body = match rest.find('\n') {
        Some(newline) => &rest[newline + 1..],
        None => rest,
    };
    body.trim_end().trim_end_matches("```").trim()
}

fn resolve_provider(
    settings: &AppSettings,
) -> Result<(crate::settings::PostProcessProvider, String, String), String> {
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or_else(|| crate::llm_client::MODEL_SETUP_REQUIRED.to_string())?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return Err(crate::llm_client::MODEL_SETUP_REQUIRED.to_string());
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    Ok((provider, model, api_key))
}

async fn ask_for_minutes_json(
    settings: &AppSettings,
    user_prompt: &str,
) -> Result<MinutesJson, String> {
    let (provider, model, api_key) = resolve_provider(settings)?;

    let mut prompt = user_prompt.to_string();
    let mut last_error = String::new();
    // Ein Retry: Struktur-Fehler sind meist einmalige Ausrutscher, ein zweiter
    // Versuch mit dem Fehlertext repariert sie — mehr wäre nur Wartezeit.
    for attempt in 0..2 {
        let response = crate::llm_client::send_chat_completion_with_schema(
            crate::managers::usage::Purpose::Minutes,
            &provider,
            api_key.clone(),
            &model,
            prompt.clone(),
            Some(minutes_system_prompt()),
            Some(minutes_schema()),
            None,
            None,
        )
        .await
        .map_err(|e| format!("Protokoll-Erzeugung fehlgeschlagen: {e}"))?
        .ok_or_else(|| "Protokoll-Antwort ohne Inhalt".to_string())?;

        match serde_json::from_str::<MinutesJson>(strip_code_fence(&response)) {
            Ok(minutes) => return Ok(minutes),
            Err(e) => {
                last_error = e.to_string();
                log::warn!(
                    "Protokoll-Antwort war kein gültiges JSON (Versuch {}): {}",
                    attempt + 1,
                    last_error
                );
                prompt = format!(
                    "{user_prompt}\n\nYour previous reply could not be parsed as \
                     the required JSON object (error: {last_error}). Reply with \
                     ONLY a JSON object matching the schema, nothing else."
                );
            }
        }
    }
    Err(format!(
        "Protokoll-Antwort war kein gültiges JSON: {last_error}"
    ))
}

/// Ein Block der map-Stufe mit eigenem Retry-Budget. `ask_for_minutes_json`
/// wiederholt nur Struktur-Fehler; ein Transportfehler (Ollama kurz weg,
/// Timeout) kommt sofort zurueck und wuerde ohne diesen zweiten Anlauf den
/// ganzen Lauf kosten.
async fn ask_for_chunk(
    settings: &AppSettings,
    prompt: &str,
    index: usize,
    total: usize,
) -> Result<MinutesJson, String> {
    let mut last_error = String::new();
    for attempt in 1..=CHUNK_ATTEMPTS {
        match ask_for_minutes_json(settings, prompt).await {
            Ok(minutes) => return Ok(minutes),
            Err(e) => {
                last_error = e;
                log::warn!(
                    "Protokoll: Block {}/{} fehlgeschlagen (Versuch {} von {}): {}",
                    index + 1,
                    total,
                    attempt,
                    CHUNK_ATTEMPTS,
                    last_error
                );
            }
        }
    }
    Err(last_error)
}

/// Kopfdaten aus den Store-Fakten. `date_iso` bevorzugt den Start der
/// Aufnahme und fällt auf das Anlagedatum zurück (Importe haben kein
/// `started_at`).
fn build_head(meeting: &super::store::Meeting, segments: &[StoredSegment]) -> MinutesHead {
    let shares = speaking_shares(segments);
    let duration_ms = meeting.duration_ms.unwrap_or_else(|| {
        segments
            .iter()
            .map(|segment| segment.end_ms)
            .max()
            .unwrap_or(0)
    });
    let timestamp = meeting.started_at.unwrap_or(meeting.created_at);
    let date_iso = chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default();

    // Importe landen vollständig auf Kanal 2 (MixedCapture) — dort steht ein
    // Kanal für unbekannt viele Sprecher, nicht für einen.
    let mixed_channel = segments.iter().any(|segment| segment.channel == 2);

    MinutesHead {
        title: meeting.title.clone(),
        date_iso,
        duration_ms,
        single_speaker: shares.len() <= 1,
        mixed_channel,
        shares,
    }
}

/// Protokoll erzeugen und als neue Dokumentversion ablegen. Ändert den Status
/// des Meetings nicht — ein fehlgeschlagener Lauf lässt ein 'ready' Meeting
/// 'ready'.
pub async fn generate_minutes_with_settings(
    settings: &AppSettings,
    store: Arc<MeetingStore>,
    meeting_id: &str,
) -> Result<MeetingDocument, String> {
    let meeting = store
        .get_meeting(meeting_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Meeting {meeting_id} nicht gefunden"))?;
    // Guard against a live recording: under the (default) `AfterMinutes`
    // retention policy, the caller purges audio right after this returns
    // (see `generate_minutes` below) — deleting a WAV the recorder still has
    // open, then nulling its path, is exactly how `recover_orphans` loses
    // audio for good after a crash. `failed` is allowed through so a
    // meeting stuck in that terminal state can still get minutes from
    // whatever transcript it captured before failing.
    if meeting.status != "ready" && meeting.status != "failed" {
        return Err(format!(
            "meeting_not_finished: cannot generate minutes while status is '{}' \
             (recording must finish first)",
            meeting.status
        ));
    }
    let segments = sorted_segments(&store.get_segments(meeting_id).map_err(|e| e.to_string())?);
    if segments.is_empty() {
        return Err("Kein Transkript vorhanden — Protokoll nicht möglich".into());
    }

    let head = build_head(&meeting, &segments);
    let transcript = render_transcript_for_prompt(&segments);

    // Blockbilanz der map-Stufe: bei einem einzelnen Ausreisser wird
    // degradiert statt abgebrochen - ein zwei Stunden langes Meeting darf
    // nicht daran scheitern, dass ein Block von zwoelf nicht durchkommt.
    let mut chunks_total: usize = 1;
    let mut chunks_failed: Vec<usize> = Vec::new();
    let minutes = if transcript.chars().count() > MAP_REDUCE_CHARS {
        let chunks = crate::summarizer::chunk_text(&transcript, MAP_REDUCE_CHARS);
        chunks_total = chunks.len();
        log::info!("Protokoll: {} Blöcke (map-reduce)", chunks.len());
        let mut partials = Vec::with_capacity(chunks.len());
        for (index, chunk) in chunks.iter().enumerate() {
            let prompt = chunk_prompt(&head, index, chunks.len(), chunk);
            match ask_for_chunk(settings, &prompt, index, chunks.len()).await {
                Ok(partial) => {
                    partials.push(serde_json::to_string(&partial).map_err(|e| e.to_string())?)
                }
                Err(e) => {
                    log::warn!(
                        "Protokoll: Block {}/{} endgültig nicht ausgewertet ({}) —                          das Protokoll entsteht aus den übrigen Blöcken",
                        index + 1,
                        chunks.len(),
                        e
                    );
                    chunks_failed.push(index + 1);
                }
            }
        }
        if partials.is_empty() {
            return Err(format!(
                "Protokoll-Erzeugung fehlgeschlagen: kein einziger der {chunks_total}                  Transkriptblöcke konnte ausgewertet werden"
            ));
        }
        ask_for_minutes_json(settings, &merge_prompt(&head, &partials, &chunks_failed)).await?
    } else {
        ask_for_minutes_json(settings, &minutes_user_prompt(&head, &transcript)).await?
    };

    validate_minutes(&minutes, head.single_speaker)?;

    let (provider, model, _) = resolve_provider(settings)?;
    let mut body = minutes_to_markdown(&head, &minutes);
    if !chunks_failed.is_empty() {
        // Der Hinweis steht im Dokument selbst: wer das Protokoll liest, muss
        // sehen, dass ein Teil des Gesprächs nicht darin steckt.
        body.push_str(&incomplete_note(chunks_total, &chunks_failed));
    }
    let metadata = serde_json::json!({
        "model": model,
        "provider": provider.id,
        "chunks_total": chunks_total,
        "chunks_failed": chunks_failed,
    })
    .to_string();
    let document_id = store
        .upsert_document(meeting_id, "minutes", "markdown@1", &body, Some(&metadata))
        .map_err(|e| e.to_string())?;

    store
        .get_documents(meeting_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|document| document.id == document_id)
        .ok_or_else(|| "Protokoll wurde gespeichert, ist aber nicht lesbar".to_string())
}

/// Präfix der automatisch abgelegten Protokolle im Ordner der Besprechung.
/// Der volle Name trägt den Erzeugungszeitpunkt: `protokoll_2026-08-21_14-30-05.md`.
const MINUTES_PREFIX: &str = "protokoll";

/// Das jüngste abgelegte Protokoll dieser Besprechung — oder keines.
///
/// Jede Erzeugung schreibt ihre eigene Datei (siehe `write_minutes_file`);
/// angezeigt und verlinkt wird immer die neueste. Das namenlose
/// `protokoll.md` aus der Zeit vor den Zeitstempeln zählt mit, sortiert sich
/// aber hinter jede gestempelte Fassung — lexikographisch liegt
/// `protokoll.md` vor `protokoll_…`, deshalb wird über den Zeitstempel im
/// Namen verglichen, nicht über Datei-Metadaten: die ändern sich beim
/// Kopieren, der Name nicht.
pub fn latest_minutes_file(
    app: &tauri::AppHandle,
    meeting_id: &str,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    let dir = super::meetings_data_dir(app)?.join(meeting_id);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(None);
    };
    let mut candidates: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|ext| ext == "md")
                && p.file_stem()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == MINUTES_PREFIX || n.starts_with("protokoll_"))
        })
        .collect();
    candidates.sort();
    Ok(candidates.pop())
}

/// Schreibt das Protokoll als Markdown neben die Aufzeichnung — mit dem
/// Erzeugungszeitpunkt im Namen, damit KEINE Fassung eine frühere
/// überschreibt. Wer dreimal neu erzeugt, hat drei Dateien und kann
/// vergleichen; vorher gewann stillschweigend die letzte.
fn write_minutes_file(
    app: &tauri::AppHandle,
    meeting_id: &str,
    body: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let path = super::meetings_data_dir(app)?
        .join(meeting_id)
        .join(format!("{MINUTES_PREFIX}_{stamp}.md"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, body)?;
    info!("meetings: Protokoll abgelegt unter {}", path.display());
    Ok(path)
}

pub async fn generate_minutes(
    app: &tauri::AppHandle,
    store: Arc<MeetingStore>,
    meeting_id: &str,
) -> Result<MeetingDocument, String> {
    let settings = crate::settings::get_settings(app);
    let document = generate_minutes_with_settings(&settings, store.clone(), meeting_id).await?;

    // Eine Datei neben der Aufzeichnung, ohne dass jemand einen Dialog
    // bestaetigen muss. Die Datenbank bleibt die Quelle der Wahrheit — schlaegt
    // das Schreiben fehl, ist das Protokoll trotzdem erzeugt, also wird hier
    // nur gewarnt statt die Erzeugung zu verwerfen.
    if let Err(e) = write_minutes_file(app, meeting_id, &document.body) {
        log::warn!("meetings: Protokolldatei nicht geschrieben ({meeting_id}): {e}");
    }

    // A minutes document now exists — recompute the audio's retention.
    // Anchored to the meeting's actual `ended_at` (falling back to
    // `created_at` for the vanishingly unlikely case it's still unset), not
    // to "now" — minutes are often generated well after the meeting ended,
    // and a `Days(n)` policy must not silently extend from that later time.
    // Under the (default) `AfterMinutes` policy this is due right now
    // regardless of `ended_at`, and waiting for the next startup sweep would
    // delay the deletion the spec wants to happen immediately, so purge this
    // meeting's audio inline.
    let now = chrono::Utc::now().timestamp();
    let policy = settings.meeting_audio_retention;
    let meeting = store.get_meeting(meeting_id).map_err(|e| e.to_string())?;
    let ended_at = meeting.as_ref().map(|m| m.ended_at.unwrap_or(m.created_at));
    let until = ended_at
        .and_then(|ended_at| super::retention::retention_until(&policy, now, ended_at, true));
    if let Err(e) = store.set_retention_until(meeting_id, until) {
        log::warn!("meetings: retention_until not stored after minutes: {e}");
    }
    if until.is_some_and(|due| due <= now) {
        if let Some(meeting) = meeting {
            // `purge_meeting_audio` only clears a path (and the retention
            // marker) once its file is actually gone — a locked/undeletable
            // WAV keeps its path so the meeting isn't left pointing at
            // audio that a later `recover_orphans` could never find again.
            super::retention::purge_meeting_audio(&store, &meeting);
        }
    }

    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_minutes_model_requests_setup_before_inference() {
        let mut settings = crate::settings::get_default_settings();
        settings.post_process_models.clear();
        assert_eq!(
            resolve_provider(&settings).unwrap_err(),
            crate::llm_client::MODEL_SETUP_REQUIRED
        );
        settings.post_process_provider_id = "removed".into();
        assert_eq!(
            resolve_provider(&settings).unwrap_err(),
            crate::llm_client::MODEL_SETUP_REQUIRED
        );
    }

    fn head(single: bool) -> MinutesHead {
        MinutesHead {
            title: "Jour fixe".into(),
            date_iso: "2026-08-19".into(),
            duration_ms: 1_800_000,
            shares: vec![
                SpeakerShare {
                    label: "Ich".into(),
                    channel: 0,
                    speech_ms: 900_000,
                    percent: 60.0,
                },
                SpeakerShare {
                    label: "Gegenseite".into(),
                    channel: 1,
                    speech_ms: 600_000,
                    percent: 40.0,
                },
            ],
            single_speaker: single,
            mixed_channel: false,
        }
    }

    /// Kopf eines Imports: alles auf Kanal 2, ein Kanal — aber unbekannt
    /// viele Sprecher.
    fn mixed_import_head() -> MinutesHead {
        MinutesHead {
            title: "Aufzeichnung Kundencall".into(),
            date_iso: "2026-08-19".into(),
            duration_ms: 1_800_000,
            shares: vec![SpeakerShare {
                label: "Aufnahme".into(),
                channel: 2,
                speech_ms: 1_500_000,
                percent: 100.0,
            }],
            single_speaker: true,
            mixed_channel: true,
        }
    }

    fn minimal_minutes() -> MinutesJson {
        MinutesJson {
            summary: "Es wurde der Projektstand besprochen und der Go-Live bestätigt.".into(),
            scope: "Wöchentlicher Projekt-Jour-fixe.".into(),
            decisions: vec![],
            tasks: vec![],
            next_steps: vec![],
            follow_ups: vec![],
            open_questions: vec![],
        }
    }

    #[test]
    fn the_user_prompt_carries_head_data_and_transcript_but_no_invented_speakers() {
        let p = minutes_user_prompt(&head(false), "Ich [00:00]: Hallo.");
        assert!(p.contains("Jour fixe"));
        assert!(p.contains("60")); // Redeanteil steht als Fakt im Prompt
        assert!(p.contains("Ich [00:00]: Hallo."));
        assert!(p.contains("Do not invent")); // Anti-Halluzination-Regel
    }

    #[test]
    fn a_mixed_import_prompt_calls_the_speaker_count_unknown_instead_of_one() {
        let p = minutes_user_prompt(&mixed_import_head(), "Aufnahme [00:00]: Guten Tag.");
        assert!(
            p.contains("single mixed recording channel"),
            "Mischaufnahme wird als solche benannt"
        );
        assert!(
            p.contains("number of speakers is unknown"),
            "Sprecherzahl bleibt ausdrücklich offen"
        );
        assert!(
            !p.contains("a single recorded speaker"),
            "ein Import mit vier Personen darf nicht als Monolog behauptet werden"
        );
        assert!(
            !p.contains("Speaking shares:"),
            "ohne Kanaltrennung gibt es keine Redeanteile"
        );
    }

    #[test]
    fn a_mic_only_recording_still_says_a_single_recorded_speaker() {
        let mut mic_only = head(true);
        mic_only.shares = vec![SpeakerShare {
            label: "Ich".into(),
            channel: 0,
            speech_ms: 1_500_000,
            percent: 100.0,
        }];
        let p = minutes_user_prompt(&mic_only, "Ich [00:00]: Notiz an mich selbst.");
        assert!(p.contains("a single recorded speaker"));
        assert!(!p.contains("number of speakers is unknown"));
    }

    #[test]
    fn build_head_marks_channel_two_as_mixed_and_channel_zero_as_not_mixed() {
        let meeting = super::super::store::Meeting {
            id: "m".into(),
            title: "T".into(),
            status: "ready".into(),
            source: "import".into(),
            started_at: None,
            ended_at: None,
            language: None,
            mic_audio_path: None,
            system_audio_path: None,
            duration_ms: Some(10_000),
            consent_confirmed_at: None,
            audio_retention_until: None,
            source_path: None,
            created_at: 1_755_600_000,
            deleted_at: None,
        };
        let segment = |channel: u8| StoredSegment {
            segment_index: 0,
            text: "x".into(),
            start_ms: 0,
            end_ms: 1_000,
            channel,
            speaker_index: None,
        };

        let imported = build_head(&meeting, &[segment(2)]);
        assert!(imported.mixed_channel);
        assert!(
            imported.single_speaker,
            "ein Kanal → keine Redeanteil-Tabelle"
        );

        let mic_only = build_head(&meeting, &[segment(0)]);
        assert!(!mic_only.mixed_channel);
        assert!(mic_only.single_speaker);
    }

    #[test]
    fn the_merge_prompt_names_the_blocks_that_are_missing() {
        let complete = merge_prompt(&head(false), &["{}".to_string()], &[]);
        assert!(!complete.contains("could not be processed"));
        let with_gap = merge_prompt(&head(false), &["{}".to_string()], &[2, 5]);
        assert!(with_gap.contains("part(s) 2, 5"));
        assert!(with_gap.contains("open_questions"));
    }

    #[test]
    fn the_incomplete_note_states_how_many_blocks_are_missing() {
        let note = incomplete_note(12, &[3]);
        assert!(note.contains("1 von 12"));
        assert!(note.contains("Block 3"));
    }

    #[test]
    fn the_schema_forbids_extra_properties_and_requires_all_sections() {
        let s = minutes_schema();
        assert_eq!(s["additionalProperties"], serde_json::json!(false));
        let req = s["required"].as_array().unwrap();
        for k in [
            "summary",
            "scope",
            "decisions",
            "tasks",
            "next_steps",
            "follow_ups",
            "open_questions",
        ] {
            assert!(req.iter().any(|v| v == k), "{k} fehlt in required");
        }
    }

    #[test]
    fn validation_rejects_empty_summary_but_allows_empty_lists() {
        let mut m = minimal_minutes();
        assert!(
            validate_minutes(&m, false).is_ok(),
            "leere Listen sind zulässig"
        );
        m.summary = "  ".into();
        assert!(validate_minutes(&m, false).is_err());
    }

    #[test]
    fn a_recording_without_channel_separation_may_have_no_scope() {
        let mut m = minimal_minutes();
        m.scope = "".into();
        assert!(
            validate_minutes(&m, true).is_ok(),
            "Diktat wie Mischaufnahme: unbekannter Rahmen ist kein Fehler"
        );
        assert!(validate_minutes(&m, false).is_err());
    }

    #[test]
    fn markdown_contains_all_sections_and_shares_table() {
        let md = minutes_to_markdown(&head(false), &minimal_minutes());
        for h in [
            "# Protokoll: Jour fixe",
            "## Zusammenfassung",
            "## Scope",
            "## Sprecher & Redeanteile",
            "## Entscheidungen",
            "## Aufgaben",
            "## Next Steps",
            "## Follow-Ups",
            "## Offene Fragen",
        ] {
            assert!(md.contains(h), "{h} fehlt");
        }
        assert!(md.contains("60,0 %") || md.contains("60.0 %"));
        assert!(
            md.contains("_keine_"),
            "leere Sektionen sagen das explizit statt zu fehlen"
        );
    }

    #[test]
    fn single_speaker_markdown_omits_the_shares_table() {
        let md = minutes_to_markdown(&head(true), &minimal_minutes());
        assert!(
            !md.contains("## Sprecher & Redeanteile"),
            "Spec: Validator-Ausnahme Ein-Sprecher-Import"
        );
    }

    #[test]
    fn transcript_rendering_prefixes_channel_and_time() {
        let segs = vec![StoredSegment {
            segment_index: 0,
            text: "Hallo.".into(),
            start_ms: 65_000,
            end_ms: 66_000,
            channel: 1,
            speaker_index: None,
        }];
        assert_eq!(
            render_transcript_for_prompt(&segs),
            "Gegenseite [01:05]: Hallo."
        );
    }

    // -- Integration gegen einen Mock-LLM ---------------------------------

    use crate::managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};
    use crate::settings::get_default_settings;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Mock eines OpenAI-kompatiblen /chat/completions-Endpunkts (Muster
    /// `translator.rs`), der den kompletten Antwort-Body vorgibt und den
    /// Request — inklusive `response_format` — schlicht verwirft.
    async fn spawn_llm_mock(body: String) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1_048_576];
                    let mut read = 0usize;
                    loop {
                        let n = sock.read(&mut buf[read..]).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        read += n;
                        let text = String::from_utf8_lossy(&buf[..read]).to_lowercase();
                        if let Some(header_end) = text.find("\r\n\r\n") {
                            let content_length = text
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length: "))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            if read >= header_end + 4 + content_length {
                                let head = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                                    body.len()
                                );
                                let _ = sock.write_all(head.as_bytes()).await;
                                let _ = sock.write_all(body.as_bytes()).await;
                                let _ = sock.shutdown().await;
                                break;
                            }
                        }
                    }
                });
            }
        });
        port
    }

    fn settings_with_mock_provider(port: u16) -> AppSettings {
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "custom".into();
        if let Some(custom) = settings.post_process_provider_mut("custom") {
            custom.base_url = format!("http://127.0.0.1:{port}/v1");
        }
        settings
            .post_process_models
            .insert("custom".into(), "test-model".into());
        settings
    }

    fn temp_store() -> (Arc<MeetingStore>, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let store = MeetingStore::open_at(&path).unwrap();
        std::mem::forget(dir); // Tempdir bis Prozessende behalten
        (Arc::new(store), path)
    }

    fn mock_response_body() -> String {
        let minutes = MinutesJson {
            summary: "Der Go-Live wurde auf den 1. September gelegt.".into(),
            scope: "Wöchentlicher Projekt-Jour-fixe.".into(),
            decisions: vec![DecisionItem {
                text: "Go-Live am 1. September".into(),
                context: "Testphase ist abgeschlossen".into(),
            }],
            tasks: vec![TaskItem {
                text: "Release-Notes schreiben".into(),
                assignee: None,
                due: None,
            }],
            next_steps: vec![],
            follow_ups: vec![],
            open_questions: vec![],
        };
        serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": serde_json::to_string(&minutes).unwrap()
                }
            }]
        })
        .to_string()
    }

    #[tokio::test]
    async fn generate_minutes_persists_a_versioned_markdown_document() {
        let port = spawn_llm_mock(mock_response_body()).await;
        let settings = settings_with_mock_provider(port);

        let (store, db_path) = temp_store();
        let meeting = store
            .create_meeting("Jour fixe", MeetingSource::Live, Some(1_755_600_000))
            .unwrap();
        store
            .append_delta(
                &meeting.id,
                &TranscriptDelta {
                    new_segments: vec![
                        StoredSegment {
                            segment_index: 0,
                            text: "Sind wir bereit für den Go-Live?".into(),
                            start_ms: 0,
                            end_ms: 3_000,
                            channel: 0,
                            speaker_index: None,
                        },
                        StoredSegment {
                            segment_index: 1,
                            text: "Ja, wir gehen am 1. September live.".into(),
                            start_ms: 3_200,
                            end_ms: 7_000,
                            channel: 1,
                            speaker_index: None,
                        },
                    ],
                },
            )
            .unwrap();
        // Mirrors the real flow: the recorder moves a meeting to `ready`
        // once it stops. The status guard (review finding #1) now refuses
        // minutes for a meeting still `recording`.
        store.set_status(&meeting.id, MeetingStatus::Ready).unwrap();

        let document = generate_minutes_with_settings(&settings, Arc::clone(&store), &meeting.id)
            .await
            .unwrap();

        assert_eq!(document.kind, "minutes");
        assert_eq!(document.body_format, "markdown@1");
        assert_eq!(document.version, 1);
        assert!(
            document.body.starts_with("# Protokoll:"),
            "war: {}",
            &document.body[..document.body.len().min(40)]
        );
        assert!(document.body.contains("Go-Live am 1. September"));
        assert!(
            document.body.contains("## Sprecher & Redeanteile"),
            "zwei Kanäle → Redeanteile im Protokoll"
        );

        let stored = store.get_documents(&meeting.id).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, document.id);

        // Metadaten führen das verwendete Modell — nicht über get_documents
        // exponiert, daher direkt aus der Datenbank gelesen.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let metadata: String = conn
            .query_row(
                "SELECT generation_metadata_json FROM meeting_documents WHERE id = ?1",
                rusqlite::params![document.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(metadata.contains("test-model"), "war: {metadata}");
        assert!(metadata.contains("custom"), "war: {metadata}");
    }

    /// Review finding #1: generating minutes for a meeting that is still
    /// `recording` must be rejected outright — under the default
    /// `AfterMinutes` policy, letting it through would purge audio the
    /// recorder still has the file handle open on.
    #[tokio::test]
    async fn generate_minutes_refuses_a_meeting_that_is_still_recording() {
        // No LLM mock is spun up: a real call would prove the guard didn't
        // fire before doing any (expensive, network-touching) work.
        let settings = get_default_settings();
        let (store, _db_path) = temp_store();
        let meeting = store
            .create_meeting("Live jour fixe", MeetingSource::Live, Some(1_755_600_000))
            .unwrap();
        assert_eq!(
            meeting.status, "recording",
            "MeetingSource::Live starts recording"
        );

        let result =
            generate_minutes_with_settings(&settings, Arc::clone(&store), &meeting.id).await;

        let err = result.expect_err("must not generate minutes for a live recording");
        assert!(err.starts_with("meeting_not_finished"), "war: {err}");

        assert!(
            store.get_documents(&meeting.id).unwrap().is_empty(),
            "no minutes document may have been created"
        );
        let stored = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(
            stored.audio_retention_until, None,
            "no purge may have run — retention marker must be untouched"
        );
    }
}
