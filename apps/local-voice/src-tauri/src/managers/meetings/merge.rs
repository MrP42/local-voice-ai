//! KI-Zusammenfuehrung zweier Transkript-Fassungen (A3, AK5, Risiko R4).
//!
//! Fassung A (Basis) gibt die Zeitachse vor: das Ergebnis hat genau die Segmente
//! von A (Zeiten, Kanaele, Nummern), nur der TEXT jeder Zeile wird mit Hilfe von B
//! verbessert (Namen, Zahlen, verhoerte Woerter). So bleiben Quellspruenge, Notizen
//! und der Player-Zeitsprung gueltig, und das Modell kann keine Zeilen erfinden
//! oder umordnen.
//!
//! Der Ablauf ist deterministisch (`temperature` 0, fester `seed` fuer
//! `Purpose::TranscriptMerge`, wie bei KI-Notizen und Protokoll) und blockweise:
//! ganze Zeilen werden mit `notes::budget::pack_ranges` in Bloecke gepackt, je Block
//! sieht das Modell die A-Zeilen und die zeitlich passenden B-Zeilen.
//!
//! Schutz gegen erfundenen Text (R4). Jede Antwort wird geprueft, bevor sie etwas
//! aendert:
//! 1. **Schema**: genau ein Eintrag je A-Zeile des Blocks (`n` je einmal, keine
//!    fremde Nummer), kein leerer Text fuer eine nicht leere Zeile.
//! 2. **Laenge**: der Text des Blocks liegt zwischen 50 und 150 % des A-Textes.
//! 3. **Ueberdeckung**: hoechstens 20 % der Woerter der Antwort duerfen in keiner der
//!    beiden Quellen des Blocks vorkommen.
//!
//! Ein durchgefallener Block behaelt den Text von A (`rejected`); fallen mehr als
//! die Haelfte der Bloecke durch, wird die ganze Zusammenfuehrung verworfen
//! (`MergeError::Rejected`), es entsteht keine Fassung.

use std::collections::HashSet;
use std::future::Future;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use serde::Deserialize;
use serde_json::{json, Value};

use super::llm_call::{ask_json, resolve_provider_coded, AskOptions, SemanticRetry};
use super::notes::budget::{balanced_block_limit, pack_ranges};
use super::store::{MeetingStore, StoredSegment};
use super::variants::{self, NewVariant, TranscriptVariant};
use crate::managers::provenance::generation::{record_generation, Generation};
use crate::managers::provenance::{ActorKind, SourceRef, SubjectKind};
use crate::managers::usage::{self, Purpose};
use crate::settings::AppSettings;

#[cfg(test)]
mod tests;

/// Zeichen A-Text je Block (Zeilen werden nie zerschnitten). Konservativ: mit der
/// B-Sicht und der Antwort bleibt der Block unter etwa 5000 Token, auch bei einem
/// lokalen Modell mit 8k Kontext.
pub const BLOCK_CHARS: usize = 4_000;
/// Hoechstens so viele Zeichen B-Text je Block.
pub const CONTEXT_CHARS: usize = 6_000;
/// B-Zeilen, die bis so viel vor oder nach dem Block liegen, zaehlen mit.
pub const CONTEXT_MARGIN_MS: u64 = 3_000;
pub const MAX_UNCOVERED_PERCENT: usize = 20;
pub const MIN_LENGTH_PERCENT: usize = 50;
pub const MAX_LENGTH_PERCENT: usize = 150;

#[derive(Clone, Debug, Deserialize)]
pub struct MergeReply {
    pub lines: Vec<ReplyLine>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReplyLine {
    pub n: u32,
    pub text: String,
}

/// Warum eine Antwort nicht angenommen wurde.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    Schema,
    Length,
    Invented,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeError {
    /// Eine der Fassungen ist leer.
    Empty,
    /// Das Modell scheiterte (Text des Fehlers, ohne Inhalt).
    Llm(String),
    /// Mehr als die Haelfte der Bloecke fiel durch die Pruefung.
    Rejected { rejected: usize, blocks: usize },
}

impl MergeError {
    pub fn code(&self) -> &'static str {
        match self {
            MergeError::Empty => "merge_empty",
            MergeError::Llm(_) => "merge_llm_failed",
            MergeError::Rejected { .. } => "merge_rejected",
        }
    }
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeError::Llm(m) => write!(f, "{}: {m}", self.code()),
            MergeError::Rejected { rejected, blocks } => {
                write!(f, "{}: {rejected}/{blocks}", self.code())
            }
            other => write!(f, "{}", other.code()),
        }
    }
}

impl std::error::Error for MergeError {}

/// Was ein Block an das Modell schickt.
pub struct BlockRequest {
    pub system: String,
    pub user: String,
    /// Die erwarteten Zeilennummern (1-basiert, Position in A).
    pub expected: Vec<u32>,
}

pub struct MergeOutcome {
    pub segments: Vec<StoredSegment>,
    pub blocks: usize,
    pub rejected: usize,
}

pub fn schema(_local: bool) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["lines"],
        "properties": {
            "lines": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["n", "text"],
                    "properties": { "n": { "type": "integer" }, "text": { "type": "string" } }
                }
            }
        }
    })
}

pub const SYSTEM_PROMPT: &str = "You merge two transcripts of the SAME recording. \
Transcript A is the base and fixes the order and number of lines. For EACH numbered line of A, \
return the best text for that line. Use transcript B only as a reference to correct mistakes \
in A (misspelled names, numbers, misheard words, missing punctuation). Never add facts, \
sentences or words that are in neither transcript, never merge or drop lines, never reorder. \
Keep the language of the transcripts. Return ONLY a JSON object {\"lines\":[{\"n\":<number>,\
\"text\":\"<text>\"}]} with exactly one entry per line of A and the same numbers.";

/// A nach Startzeit; Nummern im Prompt sind die Position in dieser Reihenfolge.
fn sorted(segments: &[StoredSegment]) -> Vec<StoredSegment> {
    let mut v = segments.to_vec();
    v.sort_by_key(|s| (s.start_ms, s.segment_index));
    v
}

/// Bloecke aus ganzen Zeilen (Logik aus `notes::budget`).
pub fn plan_blocks(a: &[StoredSegment]) -> Vec<Range<usize>> {
    let lens: Vec<usize> = a.iter().map(|s| s.text.chars().count() + 8).collect();
    let total: usize = lens.iter().sum();
    let longest = lens.iter().copied().max().unwrap_or(0);
    pack_ranges(&lens, balanced_block_limit(total, BLOCK_CHARS, longest))
}

/// Die B-Zeilen, die zeitlich zu den A-Zeilen des Blocks passen.
pub fn b_context(a_block: &[StoredSegment], b: &[StoredSegment]) -> String {
    let (Some(first), Some(last)) = (a_block.first(), a_block.last()) else {
        return String::new();
    };
    let from = first.start_ms.saturating_sub(CONTEXT_MARGIN_MS);
    let to = last.end_ms.saturating_add(CONTEXT_MARGIN_MS);
    let mut out = String::new();
    for s in b.iter().filter(|s| s.end_ms >= from && s.start_ms <= to) {
        let line = s.text.trim();
        if line.is_empty() {
            continue;
        }
        if out.chars().count() + line.chars().count() + 1 > CONTEXT_CHARS {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

pub fn user_prompt(a_block: &[StoredSegment], first_number: u32, b_text: &str) -> String {
    let mut a_text = String::new();
    for (i, s) in a_block.iter().enumerate() {
        a_text.push_str(&format!(
            "[{}] {}\n",
            first_number + i as u32,
            s.text.trim()
        ));
    }
    format!("# Transcript A (numbered lines)\n{a_text}\n# Transcript B (reference)\n{b_text}")
}

/// Woerter fuer die Ueberdeckung: kleingeschrieben, nur Buchstaben und Ziffern.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Pruefung einer Antwort gegen ihren Block (Schema, Laenge, Ueberdeckung). Gibt
/// die Texte in der Reihenfolge von A zurueck.
pub fn validate_block(
    a_block: &[StoredSegment],
    first_number: u32,
    b_text: &str,
    reply: &MergeReply,
) -> Result<Vec<String>, Reject> {
    if reply.lines.len() != a_block.len() {
        return Err(Reject::Schema);
    }
    let mut texts: Vec<Option<String>> = vec![None; a_block.len()];
    for line in &reply.lines {
        let at = line
            .n
            .checked_sub(first_number)
            .map(|d| d as usize)
            .filter(|d| *d < a_block.len())
            .ok_or(Reject::Schema)?;
        if texts[at].is_some() {
            return Err(Reject::Schema);
        }
        let text = line.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() && !a_block[at].text.trim().is_empty() {
            return Err(Reject::Schema);
        }
        texts[at] = Some(text);
    }
    let texts: Vec<String> = texts.into_iter().map(|t| t.unwrap_or_default()).collect();

    let a_len: usize = a_block.iter().map(|s| s.text.trim().chars().count()).sum();
    let out_len: usize = texts.iter().map(|t| t.chars().count()).sum();
    if a_len > 0
        && (out_len * 100 < a_len * MIN_LENGTH_PERCENT
            || out_len * 100 > a_len * MAX_LENGTH_PERCENT)
    {
        return Err(Reject::Length);
    }

    let known: HashSet<String> = a_block
        .iter()
        .flat_map(|s| words(&s.text))
        .chain(words(b_text))
        .collect();
    let out_words: Vec<String> = texts.iter().flat_map(|t| words(t)).collect();
    let uncovered = out_words.iter().filter(|w| !known.contains(*w)).count();
    if uncovered * 100 > out_words.len() * MAX_UNCOVERED_PERCENT {
        return Err(Reject::Invented);
    }
    Ok(texts)
}

/// Hinweis fuer den zweiten Versuch, wenn die Form nicht stimmt (`ask_json`).
pub fn shape_problem(reply: &MergeReply, expected: &[u32]) -> Option<String> {
    let mut got: Vec<u32> = reply.lines.iter().map(|l| l.n).collect();
    got.sort_unstable();
    (got != expected).then(|| {
        format!(
            "The previous reply did not contain exactly one entry for each of the line numbers \
             {}..{}. Return exactly {} entries with those numbers.",
            expected.first().copied().unwrap_or(0),
            expected.last().copied().unwrap_or(0),
            expected.len()
        )
    })
}

/// Fuehrt A und B zusammen. `ask` fragt das Modell je Block (Transportfehler sind
/// `Err`, dann bricht der Lauf ab; eine fachlich schlechte Antwort ist `Ok` und wird
/// hier geprueft).
pub async fn run<F, Fut>(
    a: &[StoredSegment],
    b: &[StoredSegment],
    mut ask: F,
) -> Result<MergeOutcome, MergeError>
where
    F: FnMut(BlockRequest) -> Fut,
    Fut: Future<Output = Result<MergeReply, String>>,
{
    if a.iter().all(|s| s.text.trim().is_empty()) || b.iter().all(|s| s.text.trim().is_empty()) {
        return Err(MergeError::Empty);
    }
    let a = sorted(a);
    let b = sorted(b);
    let ranges = plan_blocks(&a);
    let blocks = ranges.len();
    let mut out = a.clone();
    let mut rejected = 0usize;
    for range in ranges {
        let block = &a[range.clone()];
        let first_number = range.start as u32 + 1;
        let b_text = b_context(block, &b);
        let request = BlockRequest {
            system: SYSTEM_PROMPT.to_string(),
            user: user_prompt(block, first_number, &b_text),
            expected: (first_number..first_number + block.len() as u32).collect(),
        };
        let reply = ask(request).await.map_err(MergeError::Llm)?;
        match validate_block(block, first_number, &b_text, &reply) {
            Ok(texts) => {
                for (segment, text) in out[range].iter_mut().zip(texts) {
                    segment.text = text;
                }
            }
            Err(reason) => {
                log::warn!("Zusammenführung: Block {first_number}.. verworfen ({reason:?})");
                rejected += 1;
            }
        }
    }
    if rejected * 2 > blocks {
        return Err(MergeError::Rejected { rejected, blocks });
    }
    for (i, s) in out.iter_mut().enumerate() {
        s.segment_index = u32::try_from(i).unwrap_or(u32::MAX);
    }
    Ok(MergeOutcome {
        segments: out,
        blocks,
        rejected,
    })
}

// ---------------------------------------------------------------------------
// Der ganze Ablauf (Command und Tests)
// ---------------------------------------------------------------------------

fn label(v: &TranscriptVariant) -> String {
    format!(
        "v{} {}{}",
        v.number,
        v.kind,
        v.language
            .as_deref()
            .map(|l| format!(" ({l})"))
            .unwrap_or_default()
    )
}

/// Fuehrt die Fassungen `base_id` und `other_id` zusammen, legt die dritte Fassung an
/// (nicht aktiv) und schreibt ihre Herkunft: Quellen = beide Fassungen, Modell, Token
/// und Verweise auf das Ledger aus den Modellaufrufen dieses Laufs. Fehler sind Codes
/// (`variant_*`, `merge_*`, `no_provider`, `no_model`).
pub async fn merge_variants(
    settings: &AppSettings,
    store: &Arc<MeetingStore>,
    meeting_id: &str,
    base_id: &str,
    other_id: &str,
) -> Result<TranscriptVariant, String> {
    if base_id == other_id {
        return Err("merge_same_variant".to_string());
    }
    let (base, base_segments, other, other_segments) = {
        let conn = store.get_connection().map_err(|e| e.to_string())?;
        let (b, bs) = variants::get_segments(&conn, base_id).map_err(|e| e.to_string())?;
        let (o, os) = variants::get_segments(&conn, other_id).map_err(|e| e.to_string())?;
        (b, bs, o, os)
    };
    if base.meeting_id != meeting_id || other.meeting_id != meeting_id {
        return Err(variants::VariantError::NotFound.to_string());
    }
    // Kein Modell eingerichtet: der Code vor dem Lauf, nicht nach dem ersten Block.
    resolve_provider_coded(settings).map_err(|e| e.code.to_string())?;

    let started = Instant::now();
    let work = usage::with_capture(async {
        let outcome = run(&base_segments, &other_segments, |request| {
            let settings = settings.clone();
            async move {
                let expected = request.expected.clone();
                ask_json::<MergeReply>(
                    &settings,
                    &AskOptions {
                        purpose: Purpose::TranscriptMerge,
                        noun: "Zusammenführung",
                        redact_parse_errors: true,
                    },
                    &request.system,
                    &schema,
                    &request.user,
                    &move |reply: &MergeReply| {
                        shape_problem(reply, &expected).map(|hint| SemanticRetry {
                            reason: "Zeilennummern passen nicht".to_string(),
                            hint,
                        })
                    },
                )
                .await
            }
        })
        .await?;
        let mut conn = store
            .get_connection()
            .map_err(|e| MergeError::Llm(e.to_string()))?;
        let id = variants::add(
            &mut conn,
            NewVariant {
                meeting_id: meeting_id.to_string(),
                kind: variants::KIND_MERGED,
                language: base.language.clone().or_else(|| other.language.clone()),
                model: None,
                segments: outcome.segments,
                activate: false,
            },
        )
        .map_err(|e| MergeError::Llm(e.to_string()))?;
        // Noch im Erfassungsbereich: die Modellaufrufe stehen mit Modell und Token im
        // Eintrag. Die Herkunft laesst das Anlegen nie scheitern (`record_generation`).
        record_generation(
            store,
            Generation {
                subject_kind: SubjectKind::TranscriptVariant,
                subject_id: &id,
                subject_revision: None,
                operation: "merge",
                actor_kind: ActorKind::User,
                actor_ref: None,
                started,
                sources: vec![
                    SourceRef::new("transcript", &base.id, Some(&label(&base))),
                    SourceRef::new("transcript", &other.id, Some(&label(&other))),
                ],
                params: json!({
                    "base": base.id,
                    "other": other.id,
                    "blocks": outcome.blocks,
                    "rejected_blocks": outcome.rejected,
                }),
                fallback: None,
            },
        );
        Ok::<String, MergeError>(id)
    });
    let id = tokio::time::timeout(MERGE_TIMEOUT, work)
        .await
        .map_err(|_| "merge_timeout".to_string())?
        .map_err(|e| e.to_string())?;
    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    variants::list(&mut conn, meeting_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| variants::VariantError::NotFound.to_string())
}

/// Hoechstdauer einer Zusammenfuehrung (alle Bloecke zusammen).
pub const MERGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45 * 60);
