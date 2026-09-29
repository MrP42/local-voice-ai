//! Kontext des Chats (M4 §6): Budget, Auszuege, Shortlist und eine kleine
//! BM25-Suche im Speicher. Rein, keine I/O.
//!
//! Ein Auszug (`Excerpt`) ist die Einheit, die das Modell als `[Q<k>]` sieht
//! und zitiert. Er traegt seine Zeilen einzeln (Transkript: je Segment mit
//! Index und Startzeit; Notizen: je Block/Eintrag mit Schluessel), damit ein
//! Zitat auf das beste Segment verfeinert werden kann (`citations`).

use std::collections::{HashMap, HashSet};

use super::ChunkSource;
use crate::managers::meetings::notes::model::{EnhancedNotes, NoteBlock, NoteBlockKind};
use crate::managers::meetings::search::chunking::clock;
use crate::managers::meetings::search::index::ChunkRow;
use crate::managers::meetings::stats::label_for_channel;
use crate::managers::meetings::store::StoredSegment;

// ---------------------------------------------------------------------------
// Budget (§6: ctx 8192 - Antwort 1024 - System+Recipe 900 - Verlauf 1000)
// ---------------------------------------------------------------------------

pub const ANSWER_RESERVE_TOKENS: u32 = 1_024;
pub const SYSTEM_RESERVE_TOKENS: u32 = 900;
pub const HISTORY_RESERVE_TOKENS: u32 = 1_000;
/// Lokales Modell auf CPU: Prefill ~53 Tok/s (M6) -> 2 000 Token ~ 40 s.
pub const CPU_BUDGET_TOKENS: u32 = 2_000;
/// Entfernte Anbieter haben grosse Kontexte; bewusst begrenzt (Kosten, Datenmenge).
pub const REMOTE_BUDGET_TOKENS: u32 = 12_000;
/// Untergrenze, damit ein kleiner Kontext nicht zu einem leeren Prompt fuehrt.
pub const MIN_BUDGET_TOKENS: u32 = 512;
/// Zeichen je Token Deutsch (schlechtester Messwert, Qwen3: 3,4), mal zehn.
const CHARS_PER_TOKEN_X10: usize = 34;
/// Kopfzeile eines Auszugs im Prompt (`[Q12] B3 · Transkript 1:03:15–1:04:40`).
const HEADING_ALLOWANCE: usize = 60;

/// Token fuer Auszuege. Lokal: Kontext minus Reserven (8 192 -> 5 268), auf
/// CPU hoechstens `CPU_BUDGET_TOKENS`; Anbieter: `REMOTE_BUDGET_TOKENS`.
pub fn context_budget_tokens(ctx_tokens: u32, backend_cpu: bool, local: bool) -> u32 {
    if !local {
        return REMOTE_BUDGET_TOKENS;
    }
    let budget = ctx_tokens
        .saturating_sub(ANSWER_RESERVE_TOKENS + SYSTEM_RESERVE_TOKENS + HISTORY_RESERVE_TOKENS)
        .max(MIN_BUDGET_TOKENS);
    if backend_cpu {
        budget.min(CPU_BUDGET_TOKENS)
    } else {
        budget
    }
}

pub fn budget_chars(tokens: u32) -> usize {
    tokens as usize * CHARS_PER_TOKEN_X10 / 10
}

pub fn history_budget_chars() -> usize {
    budget_chars(HISTORY_RESERVE_TOKENS)
}

// ---------------------------------------------------------------------------
// Auszuege
// ---------------------------------------------------------------------------

/// Eine Zeile eines Auszugs. `text` geht in den Prompt, `content` ist der
/// reine Inhalt (ohne `S12 03:15 Ich: `) fuer Zitat und Verfeinerung.
#[derive(Clone, Debug, PartialEq)]
pub struct ExcerptLine {
    pub segment_index: Option<u32>,
    pub start_ms: Option<u64>,
    pub ref_key: Option<String>,
    pub text: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Excerpt {
    /// `k` in `[Q<k>]`; vergibt `number_excerpts`.
    pub qid: u32,
    /// Chunk im Index; `None` bei Auszuegen, die aus Store-Daten gebaut sind
    /// (ganze Besprechung, Live).
    pub chunk_id: Option<i64>,
    pub meeting_id: String,
    pub meeting_title: String,
    pub started_at: Option<i64>,
    /// `B<n>`; vergibt `number_excerpts`.
    pub meeting_label: String,
    pub source: ChunkSource,
    pub epoch: u32,
    /// KI-Notizen: Titel des Abschnitts.
    pub section: Option<String>,
    /// Schluessel des ganzen Auszugs (Rueckfall, wenn eine Zeile keinen hat).
    pub ref_keys: Vec<String>,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub lines: Vec<ExcerptLine>,
    /// Rang aus der Suche (groesser = besser); 0 ohne Suche.
    pub score: f64,
}

/// Kopfdaten einer Besprechung, wie der Chat sie braucht.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeetingRef {
    pub id: String,
    pub title: String,
    pub started_at: Option<i64>,
}

impl Excerpt {
    fn empty(meeting: &MeetingRef, source: ChunkSource, epoch: u32) -> Self {
        Self {
            qid: 0,
            chunk_id: None,
            meeting_id: meeting.id.clone(),
            meeting_title: meeting.title.clone(),
            started_at: meeting.started_at,
            meeting_label: String::new(),
            source,
            epoch,
            section: None,
            ref_keys: Vec::new(),
            start_ms: None,
            end_ms: None,
            lines: Vec::new(),
            score: 0.0,
        }
    }

    pub fn body(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Zeichen, die der Auszug im Prompt kostet (Kopfzeile pauschal).
    pub fn cost(&self) -> usize {
        HEADING_ALLOWANCE
            + self
                .lines
                .iter()
                .map(|l| l.text.chars().count() + 1)
                .sum::<usize>()
    }

    /// Auszug aus einem Index-Chunk. Transkriptzeilen (`S12 03:15 Ich: ...`)
    /// werden zerlegt; bei Notizen gehoeren die Zeilen 1:1 zu `ref_keys`, wenn
    /// die Anzahl passt (sonst bleibt der Schluessel am ganzen Auszug).
    pub fn from_chunk(row: &ChunkRow, meeting: &MeetingRef) -> Self {
        let mut ex = Excerpt::empty(meeting, row.source, row.epoch);
        ex.chunk_id = Some(row.id);
        ex.ref_keys = row.ref_keys.clone();
        ex.start_ms = row.start_ms;
        ex.end_ms = row.end_ms;
        let raw: Vec<&str> = row.text.lines().filter(|l| !l.trim().is_empty()).collect();
        match row.source {
            ChunkSource::Transcript => {
                ex.lines = raw
                    .iter()
                    .map(|line| match parse_transcript_line(line) {
                        Some((seg, start_ms, content)) => ExcerptLine {
                            segment_index: Some(seg),
                            start_ms: Some(start_ms),
                            ref_key: None,
                            text: line.to_string(),
                            content: content.to_string(),
                        },
                        None => plain_line(line, None),
                    })
                    .collect();
            }
            ChunkSource::AiNotes => {
                let mut body: Vec<&str> = raw.clone();
                if let Some(first) = body.first() {
                    if let Some(title) = first.strip_prefix("## ") {
                        ex.section = Some(title.trim().to_string());
                        body.remove(0);
                    }
                }
                let one_to_one = body.len() == row.ref_keys.len();
                ex.lines = body
                    .iter()
                    .enumerate()
                    .map(|(i, line)| {
                        let key = one_to_one.then(|| row.ref_keys[i].clone());
                        let mut l = plain_line(line, key);
                        l.content = line.trim_start_matches("- ").to_string();
                        l
                    })
                    .collect();
            }
            ChunkSource::UserNotes | ChunkSource::Title => {
                let one_to_one = raw.len() == row.ref_keys.len();
                ex.lines = raw
                    .iter()
                    .enumerate()
                    .map(|(i, line)| plain_line(line, one_to_one.then(|| row.ref_keys[i].clone())))
                    .collect();
            }
        }
        ex
    }
}

fn plain_line(line: &str, ref_key: Option<String>) -> ExcerptLine {
    ExcerptLine {
        segment_index: None,
        start_ms: None,
        ref_key,
        text: line.to_string(),
        content: line.to_string(),
    }
}

/// `mm:ss` oder `h:mm:ss` in Millisekunden.
fn parse_clock(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split(':').collect();
    let nums: Option<Vec<u64>> = parts.iter().map(|p| p.parse::<u64>().ok()).collect();
    let nums = nums?;
    let secs = match nums.as_slice() {
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3_600 + m * 60 + s,
        _ => return None,
    };
    Some(secs * 1_000)
}

/// `S12 03:15 Ich: Text` -> (12, 195 000, "Text").
fn parse_transcript_line(line: &str) -> Option<(u32, u64, &str)> {
    let rest = line.strip_prefix('S')?;
    let (idx, rest) = rest.split_once(' ')?;
    let seg: u32 = idx.parse().ok()?;
    let (clock_str, rest) = rest.split_once(' ')?;
    let start_ms = parse_clock(clock_str)?;
    let (_label, content) = rest.split_once(": ")?;
    Some((seg, start_ms, content))
}

/// Zeile eines Segments, gleiches Format wie der Index (`chunking`).
pub fn segment_line(seg: &StoredSegment) -> ExcerptLine {
    let content = seg.text.split_whitespace().collect::<Vec<_>>().join(" ");
    ExcerptLine {
        segment_index: Some(seg.segment_index),
        start_ms: Some(seg.start_ms),
        ref_key: None,
        text: format!(
            "S{} {} {}: {}",
            seg.segment_index,
            clock(seg.start_ms),
            label_for_channel(seg.channel),
            content
        ),
        content,
    }
}

/// Transkript in aufeinanderfolgende Bloecke von hoechstens `max_chars`
/// Zeichen (ohne Ueberlappung; ein einzelnes laengeres Segment bleibt ein
/// Block). Leere Segmente entfallen; Reihenfolge nach Startzeit.
pub fn transcript_blocks(
    meeting: &MeetingRef,
    segs: &[StoredSegment],
    epoch: u32,
    max_chars: usize,
) -> Vec<Excerpt> {
    let mut ordered: Vec<&StoredSegment> =
        segs.iter().filter(|s| !s.text.trim().is_empty()).collect();
    ordered.sort_by_key(|s| (s.start_ms, s.segment_index));
    let mut out = Vec::new();
    let mut cur = Excerpt::empty(meeting, ChunkSource::Transcript, epoch);
    let mut cur_chars = 0usize;
    for seg in ordered {
        let line = segment_line(seg);
        let chars = line.text.chars().count() + 1;
        if !cur.lines.is_empty() && cur_chars + chars > max_chars {
            out.push(std::mem::replace(
                &mut cur,
                Excerpt::empty(meeting, ChunkSource::Transcript, epoch),
            ));
            cur_chars = 0;
        }
        cur.start_ms = Some(cur.start_ms.map_or(seg.start_ms, |s| s.min(seg.start_ms)));
        cur.end_ms = Some(cur.end_ms.map_or(seg.end_ms, |e| e.max(seg.end_ms)));
        cur_chars += chars;
        cur.lines.push(line);
    }
    if !cur.lines.is_empty() {
        out.push(cur);
    }
    out
}

/// Notizblock als Auszuege (je hoechstens `max_chars`), eine Zeile je Block
/// mit der Block-ID als Schluessel.
pub fn notes_excerpts(
    meeting: &MeetingRef,
    blocks: &[NoteBlock],
    max_chars: usize,
) -> Vec<Excerpt> {
    let mut out = Vec::new();
    let mut cur = Excerpt::empty(meeting, ChunkSource::UserNotes, 0);
    let mut cur_chars = 0usize;
    for block in blocks {
        let text = block.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let prefix = match block.kind {
            NoteBlockKind::Heading => "# ",
            NoteBlockKind::Bullet => "- ",
            NoteBlockKind::Todo if block.checked => "[x] ",
            NoteBlockKind::Todo => "[ ] ",
            NoteBlockKind::Paragraph => "",
        };
        let line = ExcerptLine {
            segment_index: None,
            start_ms: block.at_ms,
            ref_key: Some(block.id.clone()),
            text: format!("{prefix}{text}"),
            content: text,
        };
        let chars = line.text.chars().count() + 1;
        if !cur.lines.is_empty() && cur_chars + chars > max_chars {
            out.push(std::mem::replace(
                &mut cur,
                Excerpt::empty(meeting, ChunkSource::UserNotes, 0),
            ));
            cur_chars = 0;
        }
        cur.ref_keys.push(block.id.clone());
        cur_chars += chars;
        cur.lines.push(line);
    }
    if !cur.lines.is_empty() {
        out.push(cur);
    }
    out
}

/// KI-Notizen als Auszuege, einer je Abschnitt; Zeilen = Eintraege mit
/// ihrer ID ("E7") als Schluessel.
pub fn enhanced_excerpts(meeting: &MeetingRef, notes: &EnhancedNotes) -> Vec<Excerpt> {
    let mut out = Vec::new();
    for section in &notes.sections {
        let mut ex = Excerpt::empty(meeting, ChunkSource::AiNotes, notes.segment_epoch);
        let title = section
            .title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        ex.section = Some(if title.is_empty() {
            section.id.clone()
        } else {
            title.chars().take(120).collect()
        });
        for entry in &section.entries {
            let mut text = entry.text.split_whitespace().collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                continue;
            }
            let mut extra: Vec<String> = Vec::new();
            if let Some(a) = entry
                .assignee
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
            {
                extra.push(a.to_string());
            }
            if let Some(d) = entry
                .due
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
            {
                extra.push(format!("bis {d}"));
            }
            if !extra.is_empty() {
                text.push_str(&format!(" ({})", extra.join(", ")));
            }
            ex.ref_keys.push(entry.id.clone());
            ex.lines.push(ExcerptLine {
                segment_index: None,
                start_ms: None,
                ref_key: Some(entry.id.clone()),
                text: format!("- {text}"),
                content: text,
            });
        }
        if !ex.lines.is_empty() {
            out.push(ex);
        }
    }
    out
}

/// Nimmt Auszuege der Reihe nach, solange sie ins Budget passen und die
/// Besprechung ihr Limit (`per_meeting`) noch nicht hat. Liefert
/// (genommen, uebrig) in der Eingabereihenfolge.
pub fn pack(
    candidates: Vec<Excerpt>,
    budget: usize,
    per_meeting: Option<usize>,
) -> (Vec<Excerpt>, Vec<Excerpt>) {
    let mut used = 0usize;
    let mut per: HashMap<String, usize> = HashMap::new();
    let mut taken = Vec::new();
    let mut left = Vec::new();
    for ex in candidates {
        let count = per.get(&ex.meeting_id).copied().unwrap_or(0);
        let cost = ex.cost();
        if used + cost <= budget && per_meeting.is_none_or(|cap| count < cap) {
            used += cost;
            per.insert(ex.meeting_id.clone(), count + 1);
            taken.push(ex);
        } else {
            left.push(ex);
        }
    }
    (taken, left)
}

pub fn total_cost(excerpts: &[Excerpt]) -> usize {
    excerpts.iter().map(Excerpt::cost).sum()
}

/// Ergebnis der Such-/Lesestufe: was in Runde 1 gelesen wird (`primary`),
/// die Kandidaten der einen Wiederholung (`secondary`) und die Zahlen fuer
/// die Abdeckung.
#[derive(Clone, Debug, Default)]
pub struct ExcerptPlan {
    pub primary: Vec<Excerpt>,
    pub secondary: Vec<Excerpt>,
    pub cards: Vec<MeetingCard>,
    /// Mehr passende Stellen als Budget.
    pub truncated: bool,
    pub lexical_only: bool,
    pub meetings_in_scope: u32,
    pub meetings_with_hits: u32,
}

// ---------------------------------------------------------------------------
// Shortlist (§6 Global)
// ---------------------------------------------------------------------------

/// Besprechungen, die als Karte in den Ueberblick kommen und zuerst gelesen werden.
pub const SHORTLIST: usize = 8;
/// Raenge 9..=20: Kandidaten der einen Wiederholung.
pub const SECOND_ROUND_LAST_RANK: usize = 20;
/// Hoechstens so viele Auszuege je Besprechung (global).
pub const MAX_PER_MEETING: usize = 3;
/// Treffer der hybriden Suche, die der Chat betrachtet.
pub const SEARCH_TOP: usize = 40;

/// Punkte je Besprechung: bester Treffer + 0,3 x Summe der uebrigen.
/// Gleichstand: fruehestes Auftreten in der Trefferliste.
pub fn rank_meetings(hits: &[(String, f64)]) -> Vec<(String, f64)> {
    let mut order: Vec<String> = Vec::new();
    let mut scores: HashMap<String, Vec<f64>> = HashMap::new();
    for (meeting, score) in hits {
        if !scores.contains_key(meeting) {
            order.push(meeting.clone());
        }
        scores.entry(meeting.clone()).or_default().push(*score);
    }
    let mut ranked: Vec<(usize, String, f64)> = order
        .into_iter()
        .enumerate()
        .map(|(i, m)| {
            let list = &scores[&m];
            let max = list.iter().copied().fold(f64::MIN, f64::max);
            let sum: f64 = list.iter().sum();
            (i, m, max + 0.3 * (sum - max))
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    ranked.into_iter().map(|(_, m, s)| (m, s)).collect()
}

/// Lesereihenfolge: Rang absteigend; bei gleichem Rang KI-Notizen vor
/// Nutzernotizen vor Transkript (Notizen sind dichter).
pub fn source_priority(source: ChunkSource) -> u8 {
    match source {
        ChunkSource::AiNotes => 0,
        ChunkSource::UserNotes => 1,
        ChunkSource::Transcript => 2,
        ChunkSource::Title => 3,
    }
}

pub fn order_for_reading(excerpts: &mut [Excerpt]) {
    excerpts.sort_by(|a, b| {
        let (sa, sb) = ((a.score * 1e9).round(), (b.score * 1e9).round());
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(source_priority(a.source).cmp(&source_priority(b.source)))
    });
}

/// Karte im Ueberblick: `B2 "Titel" 12.09.2026 · Ordner Vertrieb · <Kurzfassung>`.
#[derive(Clone, Debug, PartialEq)]
pub struct MeetingCard {
    pub label: String,
    pub meeting: MeetingRef,
    pub folders: Vec<String>,
    pub summary: String,
}

/// Erster Abschnitt der KI-Notizen mit Inhalt, hoechstens 200 Zeichen.
pub fn card_summary(notes: &EnhancedNotes) -> String {
    for section in &notes.sections {
        let joined = section
            .entries
            .iter()
            .map(|e| e.text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        if !joined.is_empty() {
            return clip_chars(&joined, 200);
        }
    }
    String::new()
}

/// Vergibt `B<n>` (Karten zuerst, dann Besprechungen der Auszuege in
/// Reihenfolge) und `Q<k>` (1..n in Auszugsreihenfolge).
pub fn number_excerpts(cards: &mut [MeetingCard], excerpts: &mut [Excerpt]) {
    let mut labels: HashMap<String, String> = HashMap::new();
    for card in cards.iter_mut() {
        let next = format!("B{}", labels.len() + 1);
        let label = labels
            .entry(card.meeting.id.clone())
            .or_insert(next)
            .clone();
        card.label = label;
    }
    for (i, ex) in excerpts.iter_mut().enumerate() {
        let next = format!("B{}", labels.len() + 1);
        ex.meeting_label = labels.entry(ex.meeting_id.clone()).or_insert(next).clone();
        ex.qid = i as u32 + 1;
    }
}

pub fn clip_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// Inhaltswoerter und BM25 im Speicher (Live und Besprechung ohne Index)
// ---------------------------------------------------------------------------

/// Haeufige Woerter ohne Inhalt, bereits gefaltet (ae->a, ss). Eigene Liste:
/// die des Index ist privat und fuer FTS-Ausdruecke gedacht.
const STOPWORDS: &[&str] = &[
    "aber", "alle", "allem", "allen", "aller", "alles", "als", "also", "am", "an", "auch", "auf",
    "aus", "bei", "bin", "bis", "bitte", "da", "dabei", "damit", "dann", "das", "dass", "dem",
    "den", "denn", "der", "des", "die", "dies", "diese", "diesem", "diesen", "dieser", "dieses",
    "doch", "dort", "du", "durch", "ein", "eine", "einem", "einen", "einer", "eines", "er", "es",
    "etwas", "euch", "fur", "gab", "gibt", "habe", "haben", "hat", "hatte", "hatten", "hier",
    "ich", "ihr", "ihre", "im", "in", "ist", "ja", "jetzt", "kann", "kein", "keine", "man", "mal",
    "mein", "meine", "mir", "mit", "mich", "muss", "nach", "nein", "nicht", "noch", "nur", "ob",
    "oder", "schon", "sehr", "sein", "seine", "sich", "sie", "sind", "so", "soll", "sollte",
    "uber", "um", "und", "uns", "unser", "von", "vom", "vor", "wahrend", "wann", "war", "waren",
    "warum", "was", "weil", "welche", "welcher", "welches", "wenn", "wer", "werden", "wie", "wir",
    "wird", "wo", "wurde", "wurden", "zu", "zum", "zur", "the", "and", "or", "of", "to", "is",
    "are", "was", "were", "be", "by", "for", "from", "it", "this", "that", "with", "what", "who",
    "when", "where", "how", "why", "did", "does", "do",
];

fn fold(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push('a'),
            'ö' => out.push('o'),
            'ü' => out.push('u'),
            'ß' => out.push_str("ss"),
            other => out.push(other),
        }
    }
    out
}

/// Stammform: die ersten 6 Zeichen (grob, aber fuer Beugungen im Deutschen
/// ausreichend: "Entscheidungen" ~ "Entscheidung", "Budgets" ~ "Budget").
fn stem(word: &str) -> String {
    word.chars().take(6).collect()
}

/// Inhaltswoerter eines Textes: gefaltet, ohne Stoppwoerter und
/// Einzelbuchstaben, gestutzt (Reihenfolge wie im Text, mit Doppelten).
pub fn content_terms(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(fold)
        .filter(|w| {
            let short = w.chars().count() < 2 && !w.chars().all(|c| c.is_ascii_digit());
            !short && !STOPWORDS.contains(&w.as_str())
        })
        .map(|w| stem(&w))
        .collect()
}

/// BM25 (k1 = 1,2, b = 0,75) der Dokumente gegen die Anfrage. Nur Treffer
/// mit Punkten > 0, beste zuerst, Gleichstand: frueheres Dokument.
pub fn bm25_rank(query: &str, docs: &[String]) -> Vec<(usize, f64)> {
    const K1: f64 = 1.2;
    const B: f64 = 0.75;
    let q: Vec<String> = {
        let mut seen = HashSet::new();
        content_terms(query)
            .into_iter()
            .filter(|t| seen.insert(t.clone()))
            .collect()
    };
    if q.is_empty() || docs.is_empty() {
        return Vec::new();
    }
    let tokenized: Vec<Vec<String>> = docs.iter().map(|d| content_terms(d)).collect();
    let n = tokenized.len() as f64;
    let avgdl = (tokenized.iter().map(Vec::len).sum::<usize>() as f64 / n).max(1.0);
    let mut df: HashMap<&str, f64> = HashMap::new();
    for doc in &tokenized {
        let unique: HashSet<&str> = doc.iter().map(String::as_str).collect();
        for term in &q {
            if unique.contains(term.as_str()) {
                *df.entry(term.as_str()).or_default() += 1.0;
            }
        }
    }
    let mut ranked: Vec<(usize, f64)> = tokenized
        .iter()
        .enumerate()
        .filter_map(|(i, doc)| {
            let dl = doc.len() as f64;
            let score: f64 = q
                .iter()
                .map(|term| {
                    let tf = doc.iter().filter(|t| *t == term).count() as f64;
                    if tf == 0.0 {
                        return 0.0;
                    }
                    let d = df.get(term.as_str()).copied().unwrap_or(0.0);
                    let idf = (1.0 + (n - d + 0.5) / (d + 0.5)).ln();
                    idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * dl / avgdl))
                })
                .sum();
            (score > 0.0).then_some((i, score))
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    ranked
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::managers::meetings::notes::model::{
        EnhanceStats, EnhancedEntry, EnhancedSection, EntryFlags, Origin, SectionKind,
    };

    pub(crate) fn meeting(id: &str) -> MeetingRef {
        MeetingRef {
            id: id.into(),
            title: format!("Besprechung {id}"),
            started_at: Some(1_789_214_400),
        }
    }

    pub(crate) fn seg(i: u32, start_s: u64, text: &str) -> StoredSegment {
        StoredSegment {
            segment_index: i,
            text: text.into(),
            start_ms: start_s * 1_000,
            end_ms: start_s * 1_000 + 4_000,
            channel: (i % 2) as u8,
            speaker_index: None,
        }
    }

    pub(crate) fn entry(id: &str, text: &str) -> EnhancedEntry {
        EnhancedEntry {
            id: id.into(),
            origin: Origin::Ai,
            text: text.into(),
            note_id: None,
            source_segment_ids: vec![1],
            assignee: None,
            due: None,
            flags: EntryFlags::default(),
        }
    }

    pub(crate) fn enhanced(sections: Vec<(&str, Vec<EnhancedEntry>)>) -> EnhancedNotes {
        EnhancedNotes {
            format: "enhanced@1".into(),
            template_id: None,
            template_title: "Allgemein".into(),
            segment_epoch: 0,
            sections: sections
                .into_iter()
                .map(|(title, entries)| EnhancedSection {
                    id: title.to_lowercase(),
                    title: title.into(),
                    kind: SectionKind::Text,
                    entries,
                })
                .collect(),
            stats: EnhanceStats::default(),
        }
    }

    #[test]
    fn budget_table_matches_the_design() {
        // Lokal, GPU, 8 192 Kontext: 8192 - 1024 - 900 - 1000.
        assert_eq!(context_budget_tokens(8_192, false, true), 5_268);
        // Anbieter: fest, CPU-Flag egal.
        assert_eq!(
            context_budget_tokens(8_192, false, false),
            REMOTE_BUDGET_TOKENS
        );
        assert_eq!(
            context_budget_tokens(8_192, true, false),
            REMOTE_BUDGET_TOKENS
        );
        // Zu kleiner Kontext: Untergrenze statt null.
        assert_eq!(context_budget_tokens(2_048, false, true), MIN_BUDGET_TOKENS);
        // 5 268 Token * 3,4 = 17 911 Zeichen (~17 700 im Entwurf).
        assert_eq!(budget_chars(5_268), 17_911);
    }

    #[test]
    fn cpu_budget_smaller() {
        let gpu = context_budget_tokens(8_192, false, true);
        let cpu = context_budget_tokens(8_192, true, true);
        assert_eq!(cpu, CPU_BUDGET_TOKENS);
        assert!(cpu < gpu);
        // Auch bei riesigem Kontext bleibt CPU bei 2 000.
        assert_eq!(
            context_budget_tokens(131_072, true, true),
            CPU_BUDGET_TOKENS
        );
        assert!(budget_chars(cpu) < budget_chars(gpu));
    }

    #[test]
    fn a_transcript_chunk_is_split_into_segment_lines() {
        let row = ChunkRow {
            id: 9,
            meeting_id: "m".into(),
            source: ChunkSource::Transcript,
            epoch: 2,
            segment_ids: vec![12, 13],
            ref_keys: vec![],
            document_id: None,
            start_ms: Some(195_000),
            end_ms: Some(3_700_000),
            channel: None,
            text: "S12 03:15 Ich: Das Budget: 5 000 Euro.\nS13 1:01:02 Kanal 7: Einverstanden"
                .into(),
            started_at: None,
        };
        let ex = Excerpt::from_chunk(&row, &meeting("m"));
        assert_eq!(ex.chunk_id, Some(9));
        assert_eq!(ex.epoch, 2);
        assert_eq!(ex.lines.len(), 2);
        assert_eq!(ex.lines[0].segment_index, Some(12));
        assert_eq!(ex.lines[0].start_ms, Some(195_000));
        assert_eq!(ex.lines[0].content, "Das Budget: 5 000 Euro.");
        assert_eq!(ex.lines[1].segment_index, Some(13));
        assert_eq!(ex.lines[1].start_ms, Some(3_662_000));
        assert_eq!(ex.lines[1].content, "Einverstanden");
    }

    #[test]
    fn ai_notes_chunk_lines_map_to_entries_only_when_counts_match() {
        let mut row = ChunkRow {
            id: 1,
            meeting_id: "m".into(),
            source: ChunkSource::AiNotes,
            epoch: 0,
            segment_ids: vec![],
            ref_keys: vec!["E1".into(), "E2".into()],
            document_id: Some("d".into()),
            start_ms: None,
            end_ms: None,
            channel: None,
            text: "## Entscheidungen\n- Budget freigegeben\n- Termin am Freitag".into(),
            started_at: None,
        };
        let ex = Excerpt::from_chunk(&row, &meeting("m"));
        assert_eq!(ex.section.as_deref(), Some("Entscheidungen"));
        assert_eq!(ex.lines[1].ref_key.as_deref(), Some("E2"));
        assert_eq!(ex.lines[1].content, "Termin am Freitag");
        // Ein geteilter Eintrag: drei Zeilen, zwei Schluessel -> keine Zuordnung je Zeile.
        row.text.push_str("\n- Fortsetzung");
        let ex = Excerpt::from_chunk(&row, &meeting("m"));
        assert!(ex.lines.iter().all(|l| l.ref_key.is_none()));
        assert_eq!(ex.ref_keys, vec!["E1", "E2"]);
    }

    #[test]
    fn transcript_blocks_respect_the_limit_and_the_time_order() {
        let segs = vec![
            seg(2, 20, "zweites"),
            seg(0, 0, "erstes Segment mit etwas Text"),
            seg(1, 10, "   "),
            seg(3, 30, "drittes"),
        ];
        let blocks = transcript_blocks(&meeting("m"), &segs, 4, 60);
        let ids: Vec<Vec<u32>> = blocks
            .iter()
            .map(|b| b.lines.iter().filter_map(|l| l.segment_index).collect())
            .collect();
        assert_eq!(ids, vec![vec![0], vec![2, 3]], "leeres Segment faellt weg");
        assert!(blocks.iter().all(|b| b.epoch == 4));
        assert_eq!(blocks[1].start_ms, Some(20_000));
        assert_eq!(blocks[1].end_ms, Some(34_000));
    }

    #[test]
    fn pack_keeps_order_budget_and_the_per_meeting_cap() {
        let mk = |m: &str, len: usize| {
            let mut ex = Excerpt::empty(&meeting(m), ChunkSource::Transcript, 0);
            ex.lines.push(plain_line(&"x".repeat(len), None));
            ex
        };
        let cands = vec![
            mk("a", 100),
            mk("a", 100),
            mk("b", 1_000),
            mk("a", 100),
            mk("c", 10),
        ];
        let budget = 3 * (HEADING_ALLOWANCE + 101) + (HEADING_ALLOWANCE + 11);
        let (taken, left) = pack(cands.clone(), budget, Some(2));
        let got: Vec<&str> = taken.iter().map(|e| e.meeting_id.as_str()).collect();
        assert_eq!(
            got,
            vec!["a", "a", "c"],
            "b zu gross, drittes a ueber dem Limit"
        );
        assert_eq!(left.len(), 2);
        let (taken, _) = pack(cands, budget, None);
        assert_eq!(taken.len(), 4);
    }

    #[test]
    fn meetings_rank_by_best_hit_plus_a_share_of_the_rest() {
        let hits = vec![
            ("a".to_string(), 1.0),
            ("b".to_string(), 0.9),
            ("b".to_string(), 0.8),
            ("c".to_string(), 0.5),
            ("d".to_string(), 0.5),
        ];
        let ranked = rank_meetings(&hits);
        let order: Vec<&str> = ranked.iter().map(|(m, _)| m.as_str()).collect();
        // b: 0.9 + 0.3*0.8 = 1.14 > a: 1.0; c und d gleich -> Reihenfolge der Treffer.
        assert_eq!(order, vec!["b", "a", "c", "d"]);
        assert!((ranked[0].1 - 1.14).abs() < 1e-9);
    }

    #[test]
    fn ai_notes_are_read_before_the_transcript_at_equal_rank() {
        let mut a = Excerpt::empty(&meeting("m"), ChunkSource::Transcript, 0);
        a.score = 0.5;
        let mut b = Excerpt::empty(&meeting("m"), ChunkSource::AiNotes, 0);
        b.score = 0.5;
        let mut c = Excerpt::empty(&meeting("m"), ChunkSource::Transcript, 0);
        c.score = 0.9;
        let mut list = vec![a, b, c];
        order_for_reading(&mut list);
        let order: Vec<(ChunkSource, f64)> = list.iter().map(|e| (e.source, e.score)).collect();
        assert_eq!(
            order,
            vec![
                (ChunkSource::Transcript, 0.9),
                (ChunkSource::AiNotes, 0.5),
                (ChunkSource::Transcript, 0.5)
            ]
        );
    }

    #[test]
    fn labels_follow_cards_then_excerpts_and_ids_start_at_one() {
        let mut cards = vec![MeetingCard {
            label: String::new(),
            meeting: meeting("b"),
            folders: vec![],
            summary: String::new(),
        }];
        let mut ex = vec![
            Excerpt::empty(&meeting("a"), ChunkSource::Transcript, 0),
            Excerpt::empty(&meeting("b"), ChunkSource::Transcript, 0),
            Excerpt::empty(&meeting("a"), ChunkSource::UserNotes, 0),
        ];
        number_excerpts(&mut cards, &mut ex);
        assert_eq!(cards[0].label, "B1");
        let got: Vec<(u32, &str)> = ex
            .iter()
            .map(|e| (e.qid, e.meeting_label.as_str()))
            .collect();
        assert_eq!(got, vec![(1, "B2"), (2, "B1"), (3, "B2")]);
    }

    #[test]
    fn notes_and_ai_notes_become_keyed_lines() {
        let blocks = vec![
            NoteBlock {
                id: "n1".into(),
                kind: NoteBlockKind::Heading,
                text: "Budget".into(),
                at_ms: None,
                checked: false,
            },
            NoteBlock {
                id: "n2".into(),
                kind: NoteBlockKind::Todo,
                text: "Angebot   schicken".into(),
                at_ms: Some(5_000),
                checked: true,
            },
            NoteBlock {
                id: "n3".into(),
                kind: NoteBlockKind::Paragraph,
                text: " ".into(),
                at_ms: None,
                checked: false,
            },
        ];
        let ex = notes_excerpts(&meeting("m"), &blocks, 10_000);
        assert_eq!(ex.len(), 1);
        assert_eq!(ex[0].body(), "# Budget\n[x] Angebot schicken");
        assert_eq!(ex[0].lines[1].ref_key.as_deref(), Some("n2"));

        let mut e2 = entry("E2", "Anna schickt das Angebot");
        e2.assignee = Some("Anna".into());
        e2.due = Some("Freitag".into());
        let notes = enhanced(vec![
            ("Leer", vec![]),
            ("Aufgaben", vec![entry("E1", "Budget klaeren"), e2]),
        ]);
        let ai = enhanced_excerpts(&meeting("m"), &notes);
        assert_eq!(ai.len(), 1, "leere Abschnitte entfallen");
        assert_eq!(ai[0].section.as_deref(), Some("Aufgaben"));
        assert_eq!(
            ai[0].lines[1].text,
            "- Anna schickt das Angebot (Anna, bis Freitag)"
        );
        assert_eq!(ai[0].lines[1].ref_key.as_deref(), Some("E2"));
        assert_eq!(
            card_summary(&notes),
            "Budget klaeren; Anna schickt das Angebot"
        );
    }

    #[test]
    fn bm25_prefers_the_matching_document_and_folds_spelling() {
        let docs = vec![
            "Wir sprechen über das Wetter.".to_string(),
            "Die Straße wird im Budget berücksichtigt, Budgets steigen.".to_string(),
            "Nichts davon.".to_string(),
        ];
        let ranked = bm25_rank("Was ist mit dem Budget und der Strasse?", &docs);
        assert_eq!(ranked.first().map(|r| r.0), Some(1));
        assert!(
            ranked.iter().all(|(i, _)| *i == 1),
            "nur Treffer mit Punkten"
        );
        assert!(
            bm25_rank("und oder aber", &docs).is_empty(),
            "nur Stoppwoerter"
        );
        assert_eq!(
            content_terms("KI und 5 Entscheidungen"),
            vec!["ki", "5", "entsch"]
        );
    }
}
