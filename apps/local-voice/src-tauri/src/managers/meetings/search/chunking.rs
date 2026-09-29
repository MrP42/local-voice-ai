//! Chunking und FTS5-Abfragebauer (rein, keine I/O).
//!
//! Zerlegt Transkript, Nutzernotizen und KI-Notizen einer Besprechung in
//! Chunks fuer den Such-Index (M4 §4). Ein Chunk ist die Einheit, die
//! gesucht, eingebettet und im Chat zitiert wird: Ziel ~1 200 Zeichen, nie
//! mehr als 1 800. Alle Laengen sind ZEICHEN (`chars()`), nicht Bytes.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::super::notes::model::{EnhancedNotes, NoteBlock, NoteBlockKind};
use super::super::speakers::SpeakerDirectory;
use super::super::store::StoredSegment;

/// Richtwert je Chunk; Umbrueche an Pause/Kanalwechsel greifen erst ab hier.
pub const TARGET_CHARS: usize = 1_200;
/// Harte Obergrenze: kein Chunk-Text ist laenger (auch nicht bei einem
/// einzelnen Riesensegment, das dann an Wortgrenzen geteilt wird).
pub const MAX_CHARS: usize = 1_800;
/// Stille zwischen zwei Segmenten, ab der (nach `TARGET_CHARS`) ein neuer Chunk beginnt.
pub const PAUSE_BREAK_MS: u64 = 2_000;

/// Mehr Suchterme als das gibt eine Abfrage nicht her (Schutz vor riesigen
/// Match-Ausdruecken durch eingefuegten Fliesstext).
const MAX_QUERY_TERMS: usize = 24;
const MAX_TERM_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChunkSource {
    Title,
    Transcript,
    UserNotes,
    AiNotes,
}

impl ChunkSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ChunkSource::Title => "title",
            ChunkSource::Transcript => "transcript",
            ChunkSource::UserNotes => "user_notes",
            ChunkSource::AiNotes => "ai_notes",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "title" => ChunkSource::Title,
            "transcript" => ChunkSource::Transcript,
            "user_notes" => ChunkSource::UserNotes,
            "ai_notes" => ChunkSource::AiNotes,
            _ => return None,
        })
    }
}

/// Kopfdaten der Besprechung, die in jeden Einbettungstext einfliessen.
#[derive(Clone, Debug, Default)]
pub struct ChunkHead {
    pub title: String,
    pub started_at: Option<i64>,
    pub folder_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkDraft {
    pub source: ChunkSource,
    /// `segment_epoch` der Segmente, auf die sich `segment_ids` beziehen
    /// (Transkript: die des Transkripts; KI-Notizen: die ihrer Quellverweise).
    pub epoch: u32,
    pub segment_ids: Vec<u32>,
    /// NoteBlock-IDs (Nutzernotizen) bzw. Eintrags-IDs "E7" (KI-Notizen).
    pub ref_keys: Vec<String>,
    /// KI-Notizen: die Dokumentversion, aus der der Chunk stammt.
    pub document_id: Option<String>,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub channel: Option<u8>,
    /// Such- und Lesetext (Transkript: Zeilen `S12 03:15 Ich: ...`).
    pub text: String,
    /// Text fuer das Embedding: `text` mit Kopfzeile "Besprechung: <Titel>, <Datum>".
    pub embed_text: String,
}

// ---------------------------------------------------------------------------
// Hilfen
// ---------------------------------------------------------------------------

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `mm:ss`, ab einer Stunde `h:mm:ss`.
pub fn clock(ms: u64) -> String {
    let total = ms / 1_000;
    let (h, m, s) = (total / 3_600, (total % 3_600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Datum in deutscher Schreibweise (`12.09.2026`, Ortszeit) oder leer.
fn date_de(started_at: Option<i64>) -> String {
    started_at
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%d.%m.%Y")
                .to_string()
        })
        .unwrap_or_default()
}

/// Einbettungstext eines Chunks. Eine Stelle fuer Chunker UND Store
/// (`chunks_without_vectors` baut ihn aus der aktuellen Besprechung neu auf,
/// denn der Einbettungstext wird nicht gespeichert).
pub fn embed_text_for(
    source: ChunkSource,
    title: &str,
    started_at: Option<i64>,
    folders: &[String],
    text: &str,
) -> String {
    let date = date_de(started_at);
    let mut header = format!("Besprechung: {}", title.trim());
    if !date.is_empty() {
        header.push_str(", ");
        header.push_str(&date);
    }
    if source == ChunkSource::Title {
        if !folders.is_empty() {
            header.push_str("\nOrdner: ");
            header.push_str(&folders.join(", "));
        }
        return header;
    }
    format!("{header}\n{text}")
}

/// Teilt `body` in Stuecke, sodass `prefix + stueck` hoechstens `max` Zeichen
/// hat. Bevorzugt Wortgrenzen; ein einzelnes Wort ueber der Grenze wird hart
/// nach Zeichen geteilt. `prefix` allein muss kuerzer als `max` sein.
fn split_to_fit(prefix: &str, body: &str, max: usize) -> Vec<String> {
    let budget = max.saturating_sub(char_len(prefix)).max(1);
    if char_len(body) <= budget {
        return vec![format!("{prefix}{body}")];
    }
    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;
    let flush = |current: &mut String, current_chars: &mut usize, pieces: &mut Vec<String>| {
        if !current.is_empty() {
            pieces.push(format!("{prefix}{current}"));
            current.clear();
            *current_chars = 0;
        }
    };
    for word in body.split_whitespace() {
        let mut word_chars = char_len(word);
        let mut word = word;
        // Ein Wort, das allein nicht passt, wird hart nach Zeichen zerlegt.
        while word_chars > budget {
            flush(&mut current, &mut current_chars, &mut pieces);
            let cut = word
                .char_indices()
                .nth(budget)
                .map(|(i, _)| i)
                .unwrap_or(word.len());
            pieces.push(format!("{prefix}{}", &word[..cut]));
            word = &word[cut..];
            word_chars = char_len(word);
        }
        if word.is_empty() {
            continue;
        }
        let needed = if current.is_empty() {
            word_chars
        } else {
            current_chars + 1 + word_chars
        };
        if needed > budget {
            flush(&mut current, &mut current_chars, &mut pieces);
        }
        if !current.is_empty() {
            current.push(' ');
            current_chars += 1;
        }
        current.push_str(word);
        current_chars += word_chars;
    }
    flush(&mut current, &mut current_chars, &mut pieces);
    pieces
}

// ---------------------------------------------------------------------------
// Transkript
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Line {
    seg_id: u32,
    start_ms: u64,
    end_ms: u64,
    channel: u8,
    text: String,
    chars: usize,
}

fn segment_lines(seg: &StoredSegment, speakers: &SpeakerDirectory) -> Vec<Line> {
    let prefix = format!(
        "S{} {} {}: ",
        seg.segment_index,
        clock(seg.start_ms),
        speakers.label(seg)
    );
    split_to_fit(&prefix, &normalize_ws(&seg.text), MAX_CHARS)
        .into_iter()
        .map(|text| Line {
            seg_id: seg.segment_index,
            start_ms: seg.start_ms,
            end_ms: seg.end_ms,
            channel: seg.channel,
            chars: char_len(&text),
            text,
        })
        .collect()
}

fn transcript_draft(lines: &[Line], epoch: u32, head: &ChunkHead) -> ChunkDraft {
    let text = lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut segment_ids: Vec<u32> = Vec::new();
    for line in lines {
        if !segment_ids.contains(&line.seg_id) {
            segment_ids.push(line.seg_id);
        }
    }
    let channel = lines
        .iter()
        .all(|l| l.channel == lines[0].channel)
        .then_some(lines[0].channel);
    ChunkDraft {
        source: ChunkSource::Transcript,
        epoch,
        segment_ids,
        ref_keys: Vec::new(),
        document_id: None,
        start_ms: lines.first().map(|l| l.start_ms),
        end_ms: lines.iter().map(|l| l.end_ms).max(),
        channel,
        embed_text: embed_text_for(
            ChunkSource::Transcript,
            &head.title,
            head.started_at,
            &head.folder_names,
            &text,
        ),
        text,
    }
}

/// Zerlegt das Transkript nach `start_ms` in Chunks.
///
/// - Bruch an einer Pause (>= `PAUSE_BREAK_MS`) oder einem Kanalwechsel, sobald
///   der Chunk `TARGET_CHARS` erreicht hat; davor wird weiter gesammelt.
/// - Hart bei `MAX_CHARS`, auch mitten im Gespraech.
/// - Ein Segment Ueberlappung: der naechste Chunk beginnt mit dem letzten
///   Segment des vorigen (wenn beide zusammen in `TARGET_CHARS` passen).
/// - Leere Segmente entfallen; ein Segment ueber `MAX_CHARS` wird an
///   Wortgrenzen geteilt (jedes Stueck behaelt seine `S<n> mm:ss Sprecher:`-Zeile).
// Die App ruft `chunk_transcript_with` (mit den Sprechernamen der Besprechung);
// diese Fassung ohne Namen bleibt fuer die Tests.
#[allow(dead_code)]
pub fn chunk_transcript(segs: &[StoredSegment], epoch: u32, head: &ChunkHead) -> Vec<ChunkDraft> {
    chunk_transcript_with(segs, epoch, head, &SpeakerDirectory::from_segments(segs))
}

/// Wie [`chunk_transcript`], mit den Sprechernamen (M3-P3c): der Chunk-Text
/// traegt "Anna Berg" statt "Gegenseite 2", damit Suche und Chat den Namen
/// finden. Ein Umbenennen hebt `content_revision` und baut das Transkript neu.
pub fn chunk_transcript_with(
    segs: &[StoredSegment],
    epoch: u32,
    head: &ChunkHead,
    speakers: &SpeakerDirectory,
) -> Vec<ChunkDraft> {
    let mut ordered: Vec<&StoredSegment> =
        segs.iter().filter(|s| !s.text.trim().is_empty()).collect();
    ordered.sort_by_key(|s| (s.start_ms, s.segment_index));

    let mut out = Vec::new();
    let mut cur: Vec<Line> = Vec::new();
    let mut cur_chars = 0usize;
    for line in ordered
        .into_iter()
        .flat_map(|seg| segment_lines(seg, speakers))
    {
        if let Some(prev) = cur.last() {
            let hard = cur_chars + 1 + line.chars > MAX_CHARS;
            let gap = line.start_ms.saturating_sub(prev.end_ms);
            let soft = cur_chars >= TARGET_CHARS
                && (gap >= PAUSE_BREAK_MS || line.channel != prev.channel);
            if hard || soft {
                let overlap = prev.clone();
                out.push(transcript_draft(&cur, epoch, head));
                cur.clear();
                cur_chars = 0;
                if overlap.seg_id != line.seg_id && overlap.chars + 1 + line.chars <= TARGET_CHARS {
                    cur_chars = overlap.chars;
                    cur.push(overlap);
                }
            }
        }
        cur_chars += line.chars + usize::from(!cur.is_empty());
        cur.push(line);
    }
    if !cur.is_empty() {
        out.push(transcript_draft(&cur, epoch, head));
    }
    out
}

// ---------------------------------------------------------------------------
// Titel, Nutzernotizen, KI-Notizen
// ---------------------------------------------------------------------------

/// Der Titel als eigener kleiner Chunk, damit Titelsuche und Titeltreffer im
/// Chat ohne Sonderweg funktionieren.
pub fn chunk_title(head: &ChunkHead) -> Option<ChunkDraft> {
    let title = normalize_ws(&head.title);
    if title.is_empty() {
        return None;
    }
    Some(ChunkDraft {
        source: ChunkSource::Title,
        epoch: 0,
        segment_ids: Vec::new(),
        ref_keys: Vec::new(),
        document_id: None,
        start_ms: None,
        end_ms: None,
        channel: None,
        embed_text: embed_text_for(
            ChunkSource::Title,
            &title,
            head.started_at,
            &head.folder_names,
            &title,
        ),
        text: title,
    })
}

#[derive(Default)]
struct NoteAcc {
    lines: Vec<String>,
    ref_keys: Vec<String>,
    segment_ids: Vec<u32>,
    min_ms: Option<u64>,
    max_ms: Option<u64>,
    chars: usize,
    has_body: bool,
}

impl NoteAcc {
    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    fn push_line(&mut self, line: String) {
        self.chars += char_len(&line) + usize::from(!self.lines.is_empty());
        self.lines.push(line);
    }

    fn add_ref(&mut self, key: &str) {
        if !self.ref_keys.iter().any(|k| k == key) {
            self.ref_keys.push(key.to_string());
        }
    }

    fn touch_time(&mut self, at_ms: Option<u64>) {
        if let Some(ms) = at_ms {
            self.min_ms = Some(self.min_ms.map_or(ms, |m| m.min(ms)));
            self.max_ms = Some(self.max_ms.map_or(ms, |m| m.max(ms)));
        }
    }
}

fn note_draft(
    acc: &mut NoteAcc,
    source: ChunkSource,
    epoch: u32,
    document_id: Option<&str>,
    head: &ChunkHead,
    out: &mut Vec<ChunkDraft>,
) {
    if acc.is_empty() {
        return;
    }
    let acc = std::mem::take(acc);
    let text = acc.lines.join("\n");
    let mut segment_ids = acc.segment_ids;
    segment_ids.sort_unstable();
    segment_ids.dedup();
    out.push(ChunkDraft {
        source,
        epoch,
        segment_ids,
        ref_keys: acc.ref_keys,
        document_id: document_id.map(str::to_string),
        start_ms: acc.min_ms,
        end_ms: acc.max_ms,
        channel: None,
        embed_text: embed_text_for(
            source,
            &head.title,
            head.started_at,
            &head.folder_names,
            &text,
        ),
        text,
    });
}

/// Nutzernotizen: ein Chunk pro Ueberschriftsabschnitt, hoechstens
/// `TARGET_CHARS` (ein einzelner laengerer Block bis `MAX_CHARS`, darueber
/// geteilt). Eine Ueberschrift ohne eigenen Inhalt bleibt nicht allein
/// stehen, sondern geht mit der naechsten. `ref_keys` = Block-IDs.
pub fn chunk_user_notes(blocks: &[NoteBlock], head: &ChunkHead) -> Vec<ChunkDraft> {
    let mut out = Vec::new();
    let mut acc = NoteAcc::default();
    for block in blocks {
        let text = normalize_ws(&block.text);
        if text.is_empty() {
            continue;
        }
        let (prefix, is_heading) = match block.kind {
            NoteBlockKind::Heading => ("# ", true),
            NoteBlockKind::Bullet => ("- ", false),
            NoteBlockKind::Todo if block.checked => ("[x] ", false),
            NoteBlockKind::Todo => ("[ ] ", false),
            NoteBlockKind::Paragraph => ("", false),
        };
        for piece in split_to_fit(prefix, &text, MAX_CHARS) {
            let piece_chars = char_len(&piece);
            let flush = if is_heading {
                acc.has_body
            } else {
                !acc.is_empty() && acc.chars + 1 + piece_chars > TARGET_CHARS
            };
            if flush {
                note_draft(&mut acc, ChunkSource::UserNotes, 0, None, head, &mut out);
            }
            acc.push_line(piece);
            acc.add_ref(&block.id);
            acc.touch_time(block.at_ms);
            if !is_heading {
                acc.has_body = true;
            }
        }
    }
    note_draft(&mut acc, ChunkSource::UserNotes, 0, None, head, &mut out);
    out
}

/// KI-Notizen: ein Chunk je Abschnitt (grosse Abschnitte nach Eintraegen
/// geteilt, die Ueberschrift steht in jedem Teil). Die Quellsegmente der
/// Eintraege werden vereinigt, `ref_keys` sind die Eintrags-IDs ("E7").
pub fn chunk_enhanced(
    document_id: &str,
    notes: &EnhancedNotes,
    head: &ChunkHead,
) -> Vec<ChunkDraft> {
    let mut out = Vec::new();
    for section in &notes.sections {
        // Die Ueberschrift steht in jedem Teil des Abschnitts; ein absurd langer
        // Titel darf den Platz fuer die Eintraege nicht auffressen.
        let title: String = normalize_ws(&section.title).chars().take(120).collect();
        let heading = format!(
            "## {}",
            if title.is_empty() {
                &section.id
            } else {
                &title
            }
        );
        let mut acc = NoteAcc::default();
        for entry in &section.entries {
            let text = normalize_ws(&entry.text);
            if text.is_empty() {
                continue;
            }
            let mut body = text;
            let mut extra: Vec<String> = Vec::new();
            if let Some(a) = entry.assignee.as_deref().map(normalize_ws) {
                if !a.is_empty() {
                    extra.push(a);
                }
            }
            if let Some(d) = entry.due.as_deref().map(normalize_ws) {
                if !d.is_empty() {
                    extra.push(format!("bis {d}"));
                }
            }
            if !extra.is_empty() {
                body.push_str(&format!(" ({})", extra.join(", ")));
            }
            // Platz fuer die Ueberschrift lassen, die jeder Teil wiederholt.
            for piece in split_to_fit(
                "- ",
                &body,
                MAX_CHARS.saturating_sub(char_len(&heading) + 1),
            ) {
                let piece_chars = char_len(&piece);
                if acc.has_body && acc.chars + 1 + piece_chars > TARGET_CHARS {
                    note_draft(
                        &mut acc,
                        ChunkSource::AiNotes,
                        notes.segment_epoch,
                        Some(document_id),
                        head,
                        &mut out,
                    );
                }
                if acc.is_empty() {
                    acc.push_line(heading.clone());
                }
                acc.push_line(piece);
                acc.add_ref(&entry.id);
                acc.has_body = true;
                acc.segment_ids
                    .extend(entry.source_segment_ids.iter().copied());
            }
        }
        note_draft(
            &mut acc,
            ChunkSource::AiNotes,
            notes.segment_epoch,
            Some(document_id),
            head,
            &mut out,
        );
    }
    out
}

// ---------------------------------------------------------------------------
// FTS5-Abfragen
// ---------------------------------------------------------------------------

const STOPWORDS: &[&str] = &[
    // Deutsch
    "aber", "alle", "allem", "allen", "aller", "alles", "als", "also", "am", "an", "auch", "auf",
    "aus", "bei", "bin", "bis", "bitte", "da", "dabei", "dann", "das", "dass", "dem", "den", "der",
    "des", "die", "dies", "diese", "diesem", "diesen", "dieser", "dieses", "doch", "dort", "du",
    "durch", "ein", "eine", "einem", "einen", "einer", "eines", "er", "es", "euch", "euer", "fuer",
    "für", "gegen", "hat", "hatte", "hatten", "haben", "hier", "ich", "ihr", "ihre", "im", "in",
    "ist", "ja", "kann", "kein", "keine", "man", "mal", "mein", "meine", "mit", "muss", "nach",
    "nein", "nicht", "noch", "nur", "ob", "oder", "sehr", "sein", "seine", "sich", "sie", "sind",
    "so", "um", "und", "uns", "unser", "von", "vom", "vor", "waehrend", "wann", "war", "waren",
    "warum", "was", "welche", "welcher", "welches", "wenn", "wer", "werden", "wie", "wir", "wird",
    "wo", "wurde", "wurden", "zu", "zum", "zur",
    // Englisch (Besprechungen sind oft gemischt)
    "the", "and", "or", "of", "to", "is", "are", "was", "were", "be", "by", "for", "from", "it",
    "this", "that", "with", "what", "who", "when", "where", "how", "why", "did", "does", "do",
];

fn is_stopword(term: &str) -> bool {
    STOPWORDS.contains(&term)
}

/// Kleingeschriebene Terme (nur Buchstaben und Ziffern), ohne Doppelte, in
/// Eingabereihenfolge, hoechstens `MAX_QUERY_TERMS`. Alles andere ist
/// Trennzeichen: so kann kein FTS5-Operator und kein Anfuehrungszeichen aus
/// der Nutzereingabe in den Match-Ausdruck gelangen.
pub fn query_terms(q: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for raw in q.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        let term: String = raw.to_lowercase().chars().take(MAX_TERM_CHARS).collect();
        if !terms.contains(&term) {
            terms.push(term);
        }
        if terms.len() >= MAX_QUERY_TERMS {
            break;
        }
    }
    terms
}

/// Schreibvarianten eines Terms. Weder unicode61 noch trigram falten ß und ss
/// ineinander, deshalb sucht Rust beide Formen (ODER).
fn spelling_variants(term: &str) -> Vec<String> {
    let mut variants = vec![term.to_string()];
    if term.contains('ß') {
        variants.push(term.replace('ß', "ss"));
    }
    if term.contains("ss") {
        variants.push(term.replace("ss", "ß"));
    }
    variants
}

fn quoted(term: &str) -> String {
    // Terme enthalten nur Buchstaben/Ziffern; das Verdoppeln bleibt als Gurt.
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// `"a" AND ("b" OR "c")`: jeder Term muss vorkommen, ss/ß-Varianten sind Alternativen.
pub(super) fn and_of_terms(terms: &[String]) -> Option<String> {
    let parts: Vec<String> = terms
        .iter()
        .map(|term| {
            let variants = spelling_variants(term);
            if variants.len() == 1 {
                quoted(&variants[0])
            } else {
                format!(
                    "({})",
                    variants
                        .iter()
                        .map(|v| quoted(v))
                        .collect::<Vec<_>>()
                        .join(" OR ")
                )
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" AND "))
}

/// Kleinster Teil eines zerlegten Kompositums (Zeichen).
const MIN_COMPOUND_PART: usize = 4;
/// Erst ab dieser Laenge kann ein Term aus zwei Teilen mit je
/// `MIN_COMPOUND_PART` Zeichen bestehen.
const MIN_COMPOUND_CHARS: usize = 2 * MIN_COMPOUND_PART;
/// Teile je Term und je Anfrage: haelt den Match-Ausdruck klein, auch wenn
/// jemand 24 lange Woerter tippt.
const MAX_PARTS_PER_TERM: usize = 12;
const MAX_PARTS_PER_QUERY: usize = 48;
/// Ab dieser Laenge bekommt ein Teil zusaetzlich einen Praefix (`"teil"*`);
/// kuerzere nur exakt, damit zufaellige Wortstuecke ("serve") nicht "server"
/// mitziehen.
const PART_PREFIX_MIN_CHARS: usize = 6;

/// Einfache Zerlegung eines Kompositums ohne Woerterbuch: jeder Schnitt mit
/// zwei Teilen ab `MIN_COMPOUND_PART` Zeichen, ausgewogene Schnitte zuerst
/// ("budgetreserve" -> budget, reserve, ...). Beim Vorderteil zusaetzlich
/// ohne Fugenlaut (s, es, n: "arbeitsplan" -> arbeits, arbeit, plan). Teile,
/// die es im Index nicht gibt, kosten nichts (exakte Terme ohne Treffer);
/// Stoppwoerter fallen weg. Nur reine Buchstaben-Terme; Ziffern bleiben ganz.
pub(crate) fn compound_parts(term: &str) -> Vec<String> {
    let chars: Vec<char> = term.chars().collect();
    let n = chars.len();
    if n < MIN_COMPOUND_CHARS || !chars.iter().all(|c| c.is_alphabetic()) {
        return Vec::new();
    }
    let mut cuts: Vec<usize> = (MIN_COMPOUND_PART..=n - MIN_COMPOUND_PART).collect();
    cuts.sort_by_key(|&i| i.abs_diff(n - i));
    let mut parts: Vec<String> = Vec::new();
    let add = |candidate: String, parts: &mut Vec<String>| {
        if char_len(&candidate) >= MIN_COMPOUND_PART
            && candidate != term
            && !is_stopword(&candidate)
            && !parts.contains(&candidate)
        {
            parts.push(candidate);
        }
    };
    for cut in cuts {
        let left: String = chars[..cut].iter().collect();
        let right: String = chars[cut..].iter().collect();
        for linking in ["es", "s", "n"] {
            if let Some(stem) = left.strip_suffix(linking) {
                add(stem.to_string(), &mut parts);
            }
        }
        add(left, &mut parts);
        add(right, &mut parts);
        if parts.len() >= MAX_PARTS_PER_TERM {
            break;
        }
    }
    parts.truncate(MAX_PARTS_PER_TERM);
    parts
}

/// Match-Ausdruck fuer die Wort-FTS (unicode61) im Chat: Stoppwoerter raus,
/// Terme gequotet, ab 5 Zeichen zusaetzlich als Praefix (`"term"*`), ss/ß-
/// Varianten, alles mit ODER (BM25 gewichtet). Lange Terme werden zusaetzlich
/// in ihre Teile zerlegt (`compound_parts`): "Budgetreserve" findet "Die
/// Reserve betraegt ...". `None`, wenn nichts Suchbares uebrig bleibt.
pub fn fts_query_words(q: &str) -> Option<String> {
    let terms: Vec<String> = query_terms(q)
        .into_iter()
        .filter(|t| !is_stopword(t))
        .collect();
    let head = or_of_terms(&terms, true)?;
    let mut parts: Vec<String> = Vec::new();
    for term in &terms {
        for part in compound_parts(term) {
            if parts.len() >= MAX_PARTS_PER_QUERY {
                break;
            }
            if !terms.contains(&part) && !parts.contains(&part) {
                parts.push(part);
            }
        }
    }
    let mut pieces: Vec<String> = Vec::new();
    for part in &parts {
        for variant in spelling_variants(part) {
            let q = quoted(&variant);
            if char_len(&variant) >= PART_PREFIX_MIN_CHARS {
                pieces.push(format!("{q}*"));
            }
            pieces.push(q);
        }
    }
    if pieces.is_empty() {
        Some(head)
    } else {
        Some(format!("{head} OR {}", pieces.join(" OR ")))
    }
}

/// Wie `fts_query_words`, aber ohne Stoppwortfilter und ohne Praefixe; fuer die
/// Listensuche, die genau das findet, was der Nutzer getippt hat.
pub(super) fn or_of_terms(terms: &[String], prefix: bool) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for term in terms {
        for variant in spelling_variants(term) {
            let q = quoted(&variant);
            if prefix && char_len(&variant) >= 5 {
                parts.push(format!("{q}*"));
            }
            parts.push(q);
        }
    }
    parts.dedup();
    (!parts.is_empty()).then(|| parts.join(" OR "))
}

/// Match-Ausdruck fuer die Trigram-FTS (Listensuche, Teilwoerter/Komposita):
/// nur Terme ab 3 Zeichen (kuerzere kennt Trigram nicht), gequotet, alle
/// muessen vorkommen (UND). `None` -> der Aufrufer nimmt die Wortsuche.
pub fn fts_query_trigram(q: &str) -> Option<String> {
    let terms: Vec<String> = query_terms(q)
        .into_iter()
        .filter(|t| char_len(t) >= 3)
        .collect();
    and_of_terms(&terms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::notes::model::{
        EnhanceStats, EnhancedEntry, EnhancedSection, EntryFlags, Origin, SectionKind,
    };

    fn head() -> ChunkHead {
        ChunkHead {
            title: "Kundengespräch Nordlicht".into(),
            // 12.09.2026 12:00 UTC: in jeder Zeitzone derselbe Kalendertag.
            started_at: Some(1_789_214_400),
            folder_names: vec!["Vertrieb".into()],
        }
    }

    fn seg(idx: u32, start_ms: u64, end_ms: u64, channel: u8, text: &str) -> StoredSegment {
        StoredSegment {
            segment_index: idx,
            text: text.into(),
            start_ms,
            end_ms,
            channel,
            speaker_index: None,
            words: None,
        }
    }

    /// `n` Segmente mit je `len` Zeichen, lueckenlos aneinander (keine Pause).
    fn dense(n: u32, len: usize, channel: u8) -> Vec<StoredSegment> {
        (0..n)
            .map(|i| {
                let t = u64::from(i) * 5_000;
                seg(i, t, t + 4_900, channel, &"a".repeat(len))
            })
            .collect()
    }

    fn max_chars(chunks: &[ChunkDraft]) -> usize {
        chunks.iter().map(|c| char_len(&c.text)).max().unwrap_or(0)
    }

    #[test]
    fn a_short_transcript_is_one_chunk_with_labelled_lines() {
        let segs = vec![
            seg(0, 0, 2_000, 0, "Guten Tag."),
            seg(1, 2_100, 4_000, 1, "Danke, gern."),
            seg(12, 195_000, 197_000, 2, "Aufnahme."),
        ];
        let chunks = chunk_transcript(&segs, 3, &head());
        assert_eq!(chunks.len(), 1);
        let c = &chunks[0];
        assert_eq!(
            c.text,
            "S0 00:00 Ich: Guten Tag.\nS1 00:02 Gegenseite: Danke, gern.\nS12 03:15 Aufnahme: Aufnahme."
        );
        assert_eq!(c.source, ChunkSource::Transcript);
        assert_eq!(c.epoch, 3);
        assert_eq!(c.segment_ids, vec![0, 1, 12]);
        assert_eq!((c.start_ms, c.end_ms), (Some(0), Some(197_000)));
        assert_eq!(c.channel, None, "gemischte Kanaele");
        assert!(c
            .embed_text
            .starts_with("Besprechung: Kundengespräch Nordlicht, 12.09.2026\n"));
        assert!(c.embed_text.ends_with(&c.text));
    }

    #[test]
    fn no_transcript_no_chunks_and_blank_segments_are_skipped() {
        assert!(chunk_transcript(&[], 0, &head()).is_empty());
        let segs = vec![
            seg(0, 0, 100, 0, "   "),
            seg(1, 200, 300, 0, "\n\t"),
            seg(2, 400, 500, 0, "Echt."),
        ];
        let chunks = chunk_transcript(&segs, 0, &head());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].segment_ids, vec![2]);
    }

    #[test]
    fn segments_are_ordered_by_start_time_not_by_input_order() {
        let segs = vec![
            seg(5, 9_000, 9_500, 0, "spaet"),
            seg(9, 1_000, 1_500, 0, "frueh"),
        ];
        let chunks = chunk_transcript(&segs, 0, &head());
        assert_eq!(chunks[0].segment_ids, vec![9, 5]);
        assert!(chunks[0].text.starts_with("S9 00:01"));
    }

    #[test]
    fn a_pause_below_the_target_size_does_not_break_the_chunk() {
        let mut segs = dense(2, 100, 0);
        segs.push(seg(2, 60_000, 61_000, 0, "nach langer Pause"));
        assert_eq!(chunk_transcript(&segs, 0, &head()).len(), 1);
    }

    #[test]
    fn a_pause_breaks_the_chunk_once_the_target_size_is_reached() {
        // 7 Segmente * ~200 Zeichen > TARGET_CHARS, dann 2,5 s Stille.
        let mut segs = dense(7, 200, 0);
        segs.push(seg(7, 37_500, 38_000, 0, "neues Thema"));
        segs.push(seg(8, 41_000, 42_000, 0, "weiter"));
        let chunks = chunk_transcript(&segs, 0, &head());
        assert_eq!(chunks.len(), 2, "genau ein Bruch an der Pause");
        assert!(!chunks[0].segment_ids.contains(&7));
        assert_eq!(
            chunks[1].segment_ids.first(),
            Some(&6),
            "Ueberlappung: letztes Segment des vorigen Chunks steht vorn"
        );
        assert!(chunks[1].segment_ids.contains(&7));
    }

    #[test]
    fn a_channel_change_breaks_the_chunk_once_the_target_size_is_reached() {
        let mut segs = dense(7, 200, 0);
        segs.push(seg(7, 35_000, 36_000, 1, "Antwort der Gegenseite"));
        let chunks = chunk_transcript(&segs, 0, &head());
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].channel, Some(0));
        assert!(chunks[1].segment_ids.contains(&7));
        // Unter der Zielgroesse bleibt ein Kanalwechsel im selben Chunk.
        let mut small = dense(2, 100, 0);
        small.push(seg(2, 10_000, 11_000, 1, "kurz"));
        assert_eq!(chunk_transcript(&small, 0, &head()).len(), 1);
    }

    #[test]
    fn max_chars_is_a_hard_limit_even_without_any_pause() {
        // Lueckenlos, gleicher Kanal, 400 Zeichen je Segment: nur MAX kann brechen.
        let chunks = chunk_transcript(&dense(20, 400, 0), 0, &head());
        assert!(chunks.len() >= 5, "8000 Zeichen brauchen mehrere Chunks");
        assert!(
            max_chars(&chunks) <= MAX_CHARS,
            "laengster Chunk {} > MAX",
            max_chars(&chunks)
        );
    }

    #[test]
    fn a_giant_segment_is_split_and_never_exceeds_max_counted_in_chars() {
        // 'ä' ist 2 Bytes: 5 000 Zeichen = 10 000 Bytes. Gezaehlt wird in Zeichen.
        let word = "Übergabeprotokoll ";
        let text = word.repeat(5_000 / word.chars().count() + 1);
        let segs = vec![seg(3, 0, 60_000, 0, &text)];
        let chunks = chunk_transcript(&segs, 0, &head());
        assert!(chunks.len() >= 3);
        assert!(max_chars(&chunks) <= MAX_CHARS);
        for c in &chunks {
            assert_eq!(c.segment_ids, vec![3], "alle Stuecke gehoeren zu Segment 3");
            assert!(c.text.lines().all(|l| l.starts_with("S3 00:00 Ich: ")));
        }
        // Kein Wort geht verloren.
        let words: usize = chunks
            .iter()
            .flat_map(|c| c.text.lines())
            .map(|l| {
                l.trim_start_matches("S3 00:00 Ich: ")
                    .split_whitespace()
                    .count()
            })
            .sum();
        assert_eq!(words, text.split_whitespace().count());
    }

    #[test]
    fn one_endless_word_is_cut_by_characters() {
        let text = "ß".repeat(4_000);
        let chunks = chunk_transcript(&[seg(0, 0, 1, 0, &text)], 0, &head());
        assert!(max_chars(&chunks) <= MAX_CHARS);
        let total: usize = chunks.iter().map(|c| c.text.matches('ß').count()).sum();
        assert_eq!(total, 4_000);
    }

    #[test]
    fn umlauts_and_emoji_survive_chunking_unchanged() {
        let segs = vec![seg(0, 0, 1_000, 0, "Größe, Übergang, Straße 🚀 – Ärger?")];
        let chunks = chunk_transcript(&segs, 0, &head());
        assert!(chunks[0]
            .text
            .contains("Größe, Übergang, Straße 🚀 – Ärger?"));
    }

    #[test]
    fn the_last_segment_of_a_chunk_reappears_at_the_start_of_the_next() {
        let chunks = chunk_transcript(&dense(20, 400, 0), 0, &head());
        for pair in chunks.windows(2) {
            let last = *pair[0].segment_ids.last().unwrap();
            assert_eq!(pair[1].segment_ids.first(), Some(&last));
        }
    }

    #[test]
    fn title_chunk_carries_folders_only_in_the_embedding_text() {
        let c = chunk_title(&head()).unwrap();
        assert_eq!(c.text, "Kundengespräch Nordlicht");
        assert_eq!(c.source, ChunkSource::Title);
        assert_eq!(
            c.embed_text,
            "Besprechung: Kundengespräch Nordlicht, 12.09.2026\nOrdner: Vertrieb"
        );
        assert!(chunk_title(&ChunkHead::default()).is_none());
    }

    fn nb(id: &str, kind: NoteBlockKind, text: &str, at_ms: Option<u64>) -> NoteBlock {
        NoteBlock {
            id: id.into(),
            kind,
            text: text.into(),
            at_ms,
            checked: false,
        }
    }

    #[test]
    fn user_notes_break_at_headings_and_keep_block_ids_and_times() {
        let blocks = vec![
            nb("b1", NoteBlockKind::Heading, "Preise", Some(1_000)),
            nb("b2", NoteBlockKind::Bullet, "Rabatt 10 %", Some(5_000)),
            nb("b3", NoteBlockKind::Heading, "Termine", Some(9_000)),
            nb("b4", NoteBlockKind::Todo, "Angebot schicken", None),
            NoteBlock {
                checked: true,
                ..nb("b5", NoteBlockKind::Todo, "Vertrag prüfen", Some(12_000))
            },
        ];
        let chunks = chunk_user_notes(&blocks, &head());
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "# Preise\n- Rabatt 10 %");
        assert_eq!(chunks[0].ref_keys, vec!["b1", "b2"]);
        assert_eq!(
            (chunks[0].start_ms, chunks[0].end_ms),
            (Some(1_000), Some(5_000))
        );
        assert_eq!(
            chunks[1].text,
            "# Termine\n[ ] Angebot schicken\n[x] Vertrag prüfen"
        );
        assert_eq!(chunks[1].ref_keys, vec!["b3", "b4", "b5"]);
        assert!(chunks.iter().all(|c| c.source == ChunkSource::UserNotes));
    }

    #[test]
    fn a_heading_without_content_joins_the_next_section() {
        let blocks = vec![
            nb("h1", NoteBlockKind::Heading, "Leer", None),
            nb("h2", NoteBlockKind::Heading, "Voll", None),
            nb("p1", NoteBlockKind::Paragraph, "Inhalt", None),
        ];
        let chunks = chunk_user_notes(&blocks, &head());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "# Leer\n# Voll\nInhalt");
    }

    #[test]
    fn long_user_notes_split_at_the_target_size() {
        let blocks: Vec<NoteBlock> = (0..10)
            .map(|i| {
                nb(
                    &format!("b{i}"),
                    NoteBlockKind::Paragraph,
                    &"x".repeat(400),
                    None,
                )
            })
            .collect();
        let chunks = chunk_user_notes(&blocks, &head());
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|c| char_len(&c.text) <= TARGET_CHARS));
        assert!(chunk_user_notes(&[], &head()).is_empty());
        assert!(
            chunk_user_notes(&[nb("b", NoteBlockKind::Paragraph, "  ", None)], &head()).is_empty()
        );
    }

    fn entry(id: &str, text: &str, sources: &[u32], assignee: Option<&str>) -> EnhancedEntry {
        EnhancedEntry {
            id: id.into(),
            origin: Origin::Ai,
            text: text.into(),
            note_id: None,
            source_segment_ids: sources.to_vec(),
            assignee: assignee.map(str::to_string),
            due: None,
            flags: EntryFlags::default(),
        }
    }

    fn enhanced(sections: Vec<EnhancedSection>) -> EnhancedNotes {
        EnhancedNotes {
            format: "enhanced@1".into(),
            template_id: None,
            template_title: "Allgemein".into(),
            segment_epoch: 4,
            sections,
            stats: EnhanceStats::default(),
        }
    }

    #[test]
    fn enhanced_notes_make_one_chunk_per_section_with_united_sources() {
        let notes = enhanced(vec![
            EnhancedSection {
                id: "decisions".into(),
                title: "Entscheidungen".into(),
                kind: SectionKind::Text,
                entries: vec![
                    entry("E1", "Start im Oktober", &[4, 2], None),
                    entry("E2", "Budget 20 000 Euro", &[2, 9], None),
                ],
            },
            EnhancedSection {
                id: "tasks".into(),
                title: "Aufgaben".into(),
                kind: SectionKind::Tasks,
                entries: vec![entry("E3", "Angebot senden", &[11], Some("Frau Müller"))],
            },
            EnhancedSection {
                id: "leer".into(),
                title: "Ohne Inhalt".into(),
                kind: SectionKind::Text,
                entries: vec![entry("E4", "  ", &[1], None)],
            },
        ]);
        let chunks = chunk_enhanced("DOC1", &notes, &head());
        assert_eq!(chunks.len(), 2, "leerer Abschnitt entfaellt");
        assert_eq!(
            chunks[0].text,
            "## Entscheidungen\n- Start im Oktober\n- Budget 20 000 Euro"
        );
        assert_eq!(
            chunks[0].segment_ids,
            vec![2, 4, 9],
            "vereinigt, sortiert, ohne Doppelte"
        );
        assert_eq!(chunks[0].ref_keys, vec!["E1", "E2"]);
        assert_eq!(chunks[0].document_id.as_deref(), Some("DOC1"));
        assert_eq!(chunks[0].epoch, 4, "Epoche der Quellverweise");
        assert_eq!(chunks[0].source, ChunkSource::AiNotes);
        assert_eq!(
            chunks[1].text,
            "## Aufgaben\n- Angebot senden (Frau Müller)"
        );
    }

    #[test]
    fn a_big_enhanced_section_splits_and_repeats_its_heading() {
        let entries: Vec<EnhancedEntry> = (0..12)
            .map(|i| entry(&format!("E{i}"), &"z".repeat(300), &[i], None))
            .collect();
        let notes = enhanced(vec![EnhancedSection {
            id: "s".into(),
            title: "Gross".into(),
            kind: SectionKind::Text,
            entries,
        }]);
        let chunks = chunk_enhanced("D", &notes, &head());
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|c| c.text.starts_with("## Gross\n")));
        assert!(chunks.iter().all(|c| char_len(&c.text) <= MAX_CHARS));
        let all_refs: usize = chunks.iter().map(|c| c.ref_keys.len()).sum();
        assert_eq!(all_refs, 12, "jeder Eintrag in genau einem Chunk");
    }

    #[test]
    fn words_query_drops_stopwords_quotes_terms_and_adds_prefixes() {
        assert_eq!(
            fts_query_words("Wie war der Termin?").as_deref(),
            Some("\"termin\"* OR \"termin\"")
        );
        assert_eq!(fts_query_words("KI").as_deref(), Some("\"ki\""));
        assert_eq!(
            fts_query_words("Q3-Planung").as_deref(),
            Some("\"q3\" OR \"planung\"* OR \"planung\"")
        );
        // Nur Stoppwoerter oder Satzzeichen: nichts Suchbares.
        assert_eq!(fts_query_words("und der die"), None);
        assert_eq!(fts_query_words("?!  --"), None);
        assert_eq!(fts_query_words(""), None);
    }

    #[test]
    fn words_query_offers_both_spellings_of_ss_and_sharp_s() {
        let q = fts_query_words("Straße").unwrap();
        assert!(q.contains("\"straße\"") && q.contains("\"strasse\""), "{q}");
        let q = fts_query_words("Strasse").unwrap();
        assert!(q.contains("\"strasse\"") && q.contains("\"straße\""), "{q}");
    }

    /// Die Terme eines Match-Ausdrucks (ohne `*`, ohne Anfuehrungszeichen).
    fn words_in(query: &str) -> Vec<String> {
        query
            .split(" OR ")
            .map(|p| p.trim_matches(|c| c == '"' || c == '*').to_string())
            .collect()
    }

    #[test]
    fn a_compound_is_also_searched_by_its_parts() {
        let q = fts_query_words("Wie hoch ist die Budgetreserve im ERP-Projekt?").unwrap();
        let words = words_in(&q);
        for wanted in [
            "budgetreserve",
            "budget",
            "reserve",
            "erp",
            "projekt",
            "hoch",
        ] {
            assert!(words.contains(&wanted.to_string()), "{wanted} fehlt in {q}");
        }
        // Der ganze Term bleibt mit Praefix, die Teile stehen dahinter.
        assert!(
            q.starts_with("\"hoch\" OR \"budgetreserve\"* OR \"budgetreserve\""),
            "{q}"
        );
        assert!(q.contains("\"budget\"* OR \"budget\""), "{q}");
        assert!(q.contains("\"reserve\"* OR \"reserve\""), "{q}");
        // Fugen-s: "Arbeitsplan" -> arbeits, arbeit, plan.
        let words = words_in(&fts_query_words("Arbeitsplan").unwrap());
        for wanted in ["arbeitsplan", "arbeits", "arbeit", "plan"] {
            assert!(words.contains(&wanted.to_string()), "{wanted} in {words:?}");
        }
        // Kunden|portal: das n faellt fuer die Grundform ("kunde").
        let words = words_in(&fts_query_words("Kundenportal").unwrap());
        assert!(words.contains(&"kunden".to_string()) && words.contains(&"kunde".to_string()));
        assert!(words.contains(&"portal".to_string()));
        // Umlaute und ss/ß: die Teile bekommen beide Schreibweisen.
        let q = fts_query_words("Straßenbaustelle").unwrap();
        assert!(
            q.contains("\"straßen\"") && q.contains("\"strassen\""),
            "{q}"
        );
        assert!(q.contains("\"baustelle\""), "{q}");
    }

    #[test]
    fn short_words_digits_and_stopword_parts_are_not_split() {
        // Unter acht Zeichen: unveraendert (bestehende Ausdruecke).
        assert_eq!(
            fts_query_words("Wie war der Termin?").as_deref(),
            Some("\"termin\"* OR \"termin\"")
        );
        assert_eq!(
            fts_query_words("Planung").as_deref(),
            Some("\"planung\"* OR \"planung\"")
        );
        // Ziffern gehoeren zu keinem Wortteil.
        assert_eq!(compound_parts("projekt2026"), Vec::<String>::new());
        assert_eq!(compound_parts("q3planung"), Vec::<String>::new());
        // Teile unter vier Zeichen und Stoppwoerter fallen weg.
        let parts = compound_parts("zwischendurch");
        assert!(parts.contains(&"zwischen".to_string()), "{parts:?}");
        assert!(
            !parts.contains(&"durch".to_string()),
            "Stoppwort: {parts:?}"
        );
        assert!(parts.iter().all(|p| char_len(p) >= MIN_COMPOUND_PART));
        // Genau zwei Mindestteile sind der kleinste Fall.
        assert_eq!(compound_parts("abcdefgh"), vec!["abcd", "efgh"]);
        assert!(compound_parts("abcdefg").is_empty());
        // Kein Schnitt an Nicht-Buchstaben; der Term selbst ist nie sein eigener Teil.
        assert!(compound_parts("ab-cdefgh").is_empty());
        assert!(!compound_parts("budgetreserve").contains(&"budgetreserve".to_string()));
    }

    #[test]
    fn the_parts_of_a_query_stay_bounded_and_the_expression_valid() {
        let long: Vec<String> = (0..24)
            .map(|i| {
                format!(
                    "verarbeitungsverzeichnis{}",
                    &"abcdefghijklmnopqrstuvwx"[..i]
                )
            })
            .collect();
        let q = fts_query_words(&long.join(" ")).unwrap();
        let mut extra = words_in(&q);
        extra.retain(|w| !long.contains(w));
        extra.sort();
        extra.dedup();
        assert!(
            extra.len() <= MAX_PARTS_PER_QUERY * 3,
            "{} Teile (mit ss-Varianten und Grundform)",
            extra.len()
        );
        assert!(
            q.len() < 8_000,
            "Ausdruck bleibt klein: {} Zeichen",
            q.len()
        );
        // Je Term hoechstens MAX_PARTS_PER_TERM Teile.
        assert!(compound_parts(&long[23]).len() <= MAX_PARTS_PER_TERM);
        // Anfuehrungszeichen paarig, nur Buchstaben und Ziffern darin.
        let parts: Vec<&str> = q.split('"').collect();
        assert_eq!(parts.len() % 2, 1);
        for (i, part) in parts.iter().enumerate() {
            if i % 2 == 1 {
                assert!(!part.is_empty() && part.chars().all(char::is_alphanumeric));
            }
        }
    }

    #[test]
    fn query_builders_cannot_be_broken_by_operators_or_quotes() {
        for evil in [
            "\" OR 1=1 --",
            "a AND NOT b NEAR(c d)",
            "col:foo * ^bar",
            "\"\"\"",
            "(((",
        ] {
            for q in [fts_query_words(evil), fts_query_trigram(evil)]
                .into_iter()
                .flatten()
            {
                let parts: Vec<&str> = q.split('"').collect();
                assert_eq!(parts.len() % 2, 1, "Anfuehrungszeichen paarig in {q}");
                for (i, part) in parts.iter().enumerate() {
                    if i % 2 == 1 {
                        assert!(
                            !part.is_empty() && part.chars().all(char::is_alphanumeric),
                            "nur Buchstaben/Ziffern in Anfuehrungszeichen: {q}"
                        );
                    } else {
                        let rest = part.replace(['(', ')', '*'], " ");
                        assert!(
                            rest.split_whitespace().all(|t| t == "OR" || t == "AND"),
                            "unerwarteter Operator ausserhalb der Terme in {q}"
                        );
                    }
                }
            }
        }
        // Zu viele Terme werden begrenzt.
        let long = (0..200)
            .map(|i| format!("wort{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(query_terms(&long).len(), MAX_QUERY_TERMS);
    }

    #[test]
    fn trigram_query_needs_three_characters_and_requires_all_terms() {
        assert_eq!(
            fts_query_trigram("Budget Planung").as_deref(),
            Some("\"budget\" AND \"planung\"")
        );
        assert_eq!(
            fts_query_trigram("KI Budget").as_deref(),
            Some("\"budget\"")
        );
        assert_eq!(fts_query_trigram("KI"), None, "unter 3 Zeichen: Wortsuche");
        assert_eq!(fts_query_trigram(""), None);
        let q = fts_query_trigram("Straße").unwrap();
        assert!(q.starts_with('(') && q.contains("\"strasse\""), "{q}");
    }
}
