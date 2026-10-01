//! G3 (#70, U9): das gemeinsame Transkript eines Projekt-Protokolls.
//!
//! Aus den gewaehlten Aufnahmen eines Projekts wird EIN Transkript in Bloecken je
//! Aufnahme: chronologisch, jede Aufnahme mit einer Kopfzeile (Nummer, Titel,
//! Datum, Dauer), jedes Segment mit seiner Quellen-ID (`R2S14` = Aufnahme 2,
//! Segment 14). Die IDs sind das, was das Modell als Beleg zitiert; `resolve`
//! macht daraus deterministisch Aufnahme, Segment und Zeitstempel zurueck. Was
//! das Modell nicht zitieren kann (Kopfzeilen), hat keine ID.
//!
//! Rein und ohne I/O: die Tests pruefen Reihenfolge, Blockgrenzen, Ueberlaenge
//! und die Ablehnung einer Aufnahme ohne Transkript ohne Modell und Datenbank.
//! Die Bloecke selbst (Halbieren, Luecken) macht die vorhandene Schleife
//! (`notes::blocks`), die Groessen der Bloecke die vorhandene Rechnung
//! (`notes::budget`).

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::managers::meetings::llm_call::{
    duration_label, mm_ss, prompt_description, sorted_segments,
};
use crate::managers::meetings::minutes::{one_line, MinutesError};
use crate::managers::meetings::notes::budget;
use crate::managers::meetings::project_minutes_store::{EntrySource, SourceRecording};
use crate::managers::meetings::speakers::SpeakerDirectory;
use crate::managers::meetings::store::{Meeting, StoredSegment};

/// Hoechstzahl Aufnahmen je Lauf: was darueber liegt, waere kein Protokoll mehr,
/// sondern ein Stapel; der Lauf wird abgewiesen statt stundenlang zu rechnen.
pub const MAX_RECORDINGS: usize = 30;

/// Laengster Titel einer Aufnahme im Prompt und in der Kopfzeile (Zeichen).
const MAX_TITLE_CHARS: usize = 120;

pub const CODE_NO_SELECTION: &str = "no_selection";
pub const CODE_TOO_MANY: &str = "too_many_recordings";
pub const CODE_NOT_IN_PROJECT: &str = "not_in_project";

// ---------------------------------------------------------------------------
// Auswahl
// ---------------------------------------------------------------------------

/// Kann diese Aufnahme in ein Projekt-Protokoll eingehen? Dieselbe Pruefung
/// fuer die Auswahl in der Oberflaeche (ausgegraut mit Grund) und fuer den Lauf
/// (Ablehnung).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eligibility {
    Ok,
    /// Noch in Aufnahme oder Verarbeitung (oder wartet in der Warteschlange).
    NotFinished,
    /// Kein Transkript (oder nur leere Segmente).
    NoTranscript,
    /// Leerer Eintrag (G1): Notizblock ohne Audio.
    EmptyEntry,
}

impl Eligibility {
    /// Der Code fuer Oberflaeche und Fehlermeldung; `None` = wählbar.
    pub fn code(self) -> Option<&'static str> {
        match self {
            Eligibility::Ok => None,
            Eligibility::NotFinished => Some("meeting_not_finished"),
            Eligibility::NoTranscript => Some("no_transcript"),
            Eligibility::EmptyEntry => Some("empty_entry"),
        }
    }
}

/// Wie `minutes::run_minutes`: `failed` und `cancelled` zaehlen mit, wenn sie ein
/// Transkript haben (das bis dahin Erfasste ist da). Ein laufender oder wartender
/// Eintrag nicht: sein Transkript waechst noch.
pub fn eligibility(meeting: &Meeting, segments: &[StoredSegment]) -> Eligibility {
    if !matches!(meeting.status.as_str(), "ready" | "failed" | "cancelled") {
        return Eligibility::NotFinished;
    }
    if segments.iter().all(|s| s.text.trim().is_empty()) {
        return if meeting.source == "empty" {
            Eligibility::EmptyEntry
        } else {
            Eligibility::NoTranscript
        };
    }
    Eligibility::Ok
}

/// Die Auswahl bereinigt: ohne Leeres und Doppeltes, Reihenfolge der Nennung.
/// Keine Auswahl und zu viele sind Fehler (`no_selection`, `too_many_recordings`).
pub fn clean_selection(ids: &[String]) -> Result<Vec<String>, MinutesError> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let id = id.trim();
        if !id.is_empty() && seen.insert(id) {
            out.push(id.to_string());
        }
    }
    if out.is_empty() {
        return Err(MinutesError::code_only(CODE_NO_SELECTION));
    }
    if out.len() > MAX_RECORDINGS {
        return Err(MinutesError::new(
            CODE_TOO_MANY,
            format!("{} > {MAX_RECORDINGS}", out.len()),
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Eingabe und Reihenfolge
// ---------------------------------------------------------------------------

/// Eine Aufnahme mit allem, was das gemeinsame Transkript braucht.
pub struct RecordingInput {
    pub meeting: Meeting,
    /// Nach Startzeit sortiert, ohne leere Segmente (die haben keine Aussage und
    /// waeren kein sinnvoller Beleg).
    pub segments: Vec<StoredSegment>,
    pub labels: SpeakerDirectory,
}

impl RecordingInput {
    pub fn new(meeting: Meeting, segments: &[StoredSegment], labels: SpeakerDirectory) -> Self {
        let segments = sorted_segments(segments)
            .into_iter()
            .filter(|s| !s.text.trim().is_empty())
            .collect();
        Self {
            meeting,
            segments,
            labels,
        }
    }
}

/// Wann die Aufnahme stattfand: Start, sonst Anlage (Importe haben keinen Start).
fn happened_at(meeting: &Meeting) -> i64 {
    meeting.started_at.unwrap_or(meeting.created_at)
}

/// Chronologisch: Start, dann Anlage, dann die ID (stabil bei Gleichstand).
pub fn order_chronologically(inputs: &mut [RecordingInput]) {
    inputs.sort_by(|a, b| {
        (
            happened_at(&a.meeting),
            a.meeting.created_at,
            a.meeting.id.as_str(),
        )
            .cmp(&(
                happened_at(&b.meeting),
                b.meeting.created_at,
                b.meeting.id.as_str(),
            ))
    });
}

fn date_iso(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn clip_title(title: &str) -> String {
    let one = one_line(title);
    if one.chars().count() <= MAX_TITLE_CHARS {
        return one;
    }
    let clipped: String = one.chars().take(MAX_TITLE_CHARS).collect();
    format!("{}…", clipped.trim_end())
}

// ---------------------------------------------------------------------------
// Das Transkript
// ---------------------------------------------------------------------------

/// Wo eine Zeile des Transkripts herkommt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineRef {
    /// Nummer der Aufnahme ab 0.
    pub recording: usize,
    pub segment_index: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Das gemeinsame Transkript: eine Zeile je Kopf und Segment, dazu was jede
/// Zeile belegt.
pub struct Corpus {
    pub recordings: Vec<SourceRecording>,
    pub lines: Vec<String>,
    /// Parallel zu `lines`: `None` fuer Kopfzeilen.
    pub refs: Vec<Option<LineRef>>,
    /// Index der ersten Zeile (der Kopfzeile) je Aufnahme; am Ende die Zeilenzahl.
    pub starts: Vec<usize>,
    by_id: HashMap<(usize, u32), usize>,
}

/// Die ID, unter der das Modell ein Segment zitiert: `R<aufnahme>S<segment>`.
pub fn source_id(recording: usize, segment_index: u32) -> String {
    format!("R{}S{}", recording + 1, segment_index)
}

/// `R2S14` -> (1, 14): Aufnahme (ab 0) und Segment. Nachsichtig gegenueber
/// Gross-/Kleinschreibung und Trennern (`r2-s14`, `R2.S14`, `R2 S14`); alles
/// andere (`S14`, `R0S1`, `R2`) ist keine Quelle.
pub fn parse_source_id(raw: &str) -> Option<(usize, u32)> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '.' | ':' | '_'))
        .collect::<String>()
        .to_uppercase();
    let rest = cleaned.strip_prefix('R')?;
    let at = rest.find('S')?;
    let (recording, segment) = (&rest[..at], &rest[at + 1..]);
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(recording) || !digits(segment) {
        return None;
    }
    let recording: usize = recording.parse().ok()?;
    Some((recording.checked_sub(1)?, segment.parse().ok()?))
}

fn header_line(index: usize, recording: &SourceRecording) -> String {
    format!(
        "=== R{} · {} · {} · {} ===",
        index + 1,
        recording.title,
        date_iso(recording.started_at),
        recording
            .duration_ms
            .map(duration_label)
            .unwrap_or_else(|| "?".to_string()),
    )
}

/// Baut das gemeinsame Transkript. `inputs` stehen schon in der Reihenfolge der
/// Aufnahmen (`order_chronologically`).
pub fn build_corpus(inputs: &[RecordingInput]) -> Corpus {
    let mut recordings = Vec::with_capacity(inputs.len());
    let mut lines: Vec<String> = Vec::new();
    let mut refs: Vec<Option<LineRef>> = Vec::new();
    let mut starts = Vec::with_capacity(inputs.len() + 1);
    let mut by_id = HashMap::new();
    for (r, input) in inputs.iter().enumerate() {
        let meeting = &input.meeting;
        let recording = SourceRecording {
            index: (r + 1) as u32,
            meeting_id: meeting.id.clone(),
            title: clip_title(&meeting.title),
            started_at: happened_at(meeting),
            duration_ms: meeting
                .duration_ms
                .or_else(|| input.segments.iter().map(|s| s.end_ms).max()),
            segments: input.segments.len() as u32,
        };
        starts.push(lines.len());
        lines.push(header_line(r, &recording));
        refs.push(None);
        for segment in &input.segments {
            by_id
                .entry((r, segment.segment_index))
                .or_insert(lines.len());
            lines.push(format!(
                "{} {} [{}]: {}",
                source_id(r, segment.segment_index),
                input.labels.label(segment),
                mm_ss(segment.start_ms),
                one_line(&segment.text)
            ));
            refs.push(Some(LineRef {
                recording: r,
                segment_index: segment.segment_index,
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
            }));
        }
        recordings.push(recording);
    }
    starts.push(lines.len());
    Corpus {
        recordings,
        lines,
        refs,
        starts,
        by_id,
    }
}

impl Corpus {
    /// Das Transkript als Text (alle Zeilen).
    pub fn transcript(&self) -> String {
        self.lines.join("\n")
    }

    /// Ein Teil des Transkripts als Text.
    pub fn chunk(&self, range: &Range<usize>) -> String {
        self.lines[range.clone()].join("\n")
    }

    /// Laenge je Zeile samt Zeilenumbruch (Grundlage der Blockrechnung).
    pub fn line_chars(&self) -> Vec<usize> {
        self.lines.iter().map(|l| l.chars().count() + 1).collect()
    }

    /// Aus einer vom Modell genannten ID den Beleg machen; `None`, wenn es sie
    /// nicht gibt (erfunden, Kopfzeile, andere Schreibweise).
    pub fn resolve(&self, raw: &str) -> Option<EntrySource> {
        let (recording, segment_index) = parse_source_id(raw)?;
        let line = *self.by_id.get(&(recording, segment_index))?;
        let line_ref = self.refs.get(line)?.as_ref()?;
        Some(EntrySource {
            recording: (recording + 1) as u32,
            meeting_id: self.recordings.get(recording)?.meeting_id.clone(),
            segment_index,
            start_ms: line_ref.start_ms,
        })
    }

    /// Die Bloecke der Planung: je Aufnahme gepackt (`budget::pack_ranges`, ganze
    /// Zeilen, nie zerschnitten), danach benachbarte Aufnahmen zusammengelegt, wo
    /// beide GANZ in einen Block passen. So endet ein Block an einer Aufnahme-
    /// grenze, ausser kleine Aufnahmen teilen sich einen; eine lange Aufnahme
    /// wird in sich geteilt, nie quer zu ihrer Nachbarin.
    pub fn pack(&self, max_chars: usize) -> Vec<Range<usize>> {
        let line_chars = self.line_chars();
        pack_aligned(&line_chars, &self.starts, max_chars)
    }

    /// Die Stellen, die ein Bereich ohne Ergebnis abdeckt, je Aufnahme:
    /// `Aufnahme 2, 03:15-07:40`. Kopfzeilen allein ergeben nichts.
    pub fn gap_labels(&self, range: &Range<usize>) -> Vec<String> {
        let mut labels = Vec::new();
        for (r, bounds) in self.starts.windows(2).enumerate() {
            let from = range.start.max(bounds[0]);
            let to = range.end.min(bounds[1]);
            if from >= to {
                continue;
            }
            let covered: Vec<&LineRef> = self.refs[from..to].iter().flatten().collect();
            let (Some(first), Some(last)) = (covered.first(), covered.last()) else {
                continue;
            };
            labels.push(format!(
                "Aufnahme {}, {}-{}",
                r + 1,
                mm_ss(first.start_ms),
                mm_ss(last.end_ms.max(first.start_ms))
            ));
        }
        labels
    }

    /// Die berechneten Fakten fuer die Prompts: Projekt und Aufnahmen. Zahlen und
    /// Daten daraus werden dem Modell als gegeben mitgeteilt, nie von ihm
    /// berechnet.
    pub fn facts_block(&self, project: &str, descriptions: &[String]) -> String {
        let mut block = format!(
            "# Project facts (computed, treat as given — restate them, never recompute)\n\
             Project: {}\nRecordings ({}, chronological, R1 is the oldest):\n",
            one_line(project),
            self.recordings.len()
        );
        for (r, recording) in self.recordings.iter().enumerate() {
            block.push_str(&format!(
                "- R{}: {} ({}, {})\n",
                r + 1,
                recording.title,
                date_iso(recording.started_at),
                recording
                    .duration_ms
                    .map(duration_label)
                    .unwrap_or_else(|| "?".to_string())
            ));
            let description = descriptions
                .get(r)
                .map(|d| prompt_description(d))
                .unwrap_or_default();
            if !description.is_empty() {
                block.push_str(&format!(
                    "  Description (written by the user; background context only, not a \
                     source for quotes or decisions): {description}\n"
                ));
            }
        }
        block
    }
}

/// Packen je Aufnahme, dann Zusammenlegen an den Aufnahmegrenzen (siehe
/// [`Corpus::pack`]). `starts`: erste Zeile je Aufnahme, am Ende die Zeilenzahl.
pub fn pack_aligned(line_chars: &[usize], starts: &[usize], max_chars: usize) -> Vec<Range<usize>> {
    let mut packed: Vec<Range<usize>> = Vec::new();
    for bounds in starts.windows(2) {
        let (from, to) = (bounds[0], bounds[1]);
        for range in budget::pack_ranges(&line_chars[from..to], max_chars) {
            packed.push(from + range.start..from + range.end);
        }
    }
    let size = |range: &Range<usize>| line_chars[range.clone()].iter().sum::<usize>();
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(packed.len());
    for range in packed {
        let at_boundary = starts.contains(&range.start);
        if let Some(last) = merged.last_mut() {
            if at_boundary && size(last) + size(&range) <= max_chars {
                last.end = range.end;
                continue;
            }
        }
        merged.push(range);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(
        id: &str,
        title: &str,
        status: &str,
        source: &str,
        started_at: Option<i64>,
        created_at: i64,
    ) -> Meeting {
        Meeting {
            id: id.into(),
            title: title.into(),
            status: status.into(),
            source: source.into(),
            started_at,
            ended_at: None,
            language: Some("de".into()),
            mic_audio_path: None,
            system_audio_path: None,
            duration_ms: Some(1_800_000),
            consent_confirmed_at: None,
            audio_retention_until: None,
            source_path: None,
            description: None,
            created_at,
            deleted_at: None,
        }
    }

    fn seg(index: u32, start_ms: u64, text: &str) -> StoredSegment {
        StoredSegment {
            segment_index: index,
            text: text.into(),
            start_ms,
            end_ms: start_ms + 3_000,
            channel: (index % 2) as u8,
            speaker_index: None,
            words: None,
        }
    }

    fn input(
        id: &str,
        title: &str,
        started_at: i64,
        segments: Vec<StoredSegment>,
    ) -> RecordingInput {
        let m = meeting(id, title, "ready", "live", Some(started_at), started_at);
        let labels = SpeakerDirectory::from_segments(&segments);
        RecordingInput::new(m, &segments, labels)
    }

    fn segments(n: u32, text: &str) -> Vec<StoredSegment> {
        (0..n)
            .map(|i| seg(i, u64::from(i) * 10_000, text))
            .collect()
    }

    // -- Reihenfolge -------------------------------------------------------------

    #[test]
    fn recordings_are_ordered_by_start_then_creation_then_id() {
        let mut inputs = vec![
            input("c", "Spaet", 3_000, segments(1, "x")),
            input("b2", "Gleich B", 2_000, segments(1, "x")),
            input("a", "Frueh", 1_000, segments(1, "x")),
            input("b1", "Gleich A", 2_000, segments(1, "x")),
        ];
        order_chronologically(&mut inputs);
        let ids: Vec<_> = inputs.iter().map(|i| i.meeting.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b1", "b2", "c"]);
    }

    #[test]
    fn an_import_without_a_start_is_placed_by_its_creation_time() {
        let mut imported = meeting("imp", "Import", "ready", "import", None, 1_500);
        imported.started_at = None;
        let segs = segments(1, "x");
        let mut inputs = vec![
            input("late", "Spaet", 2_000, segs.clone()),
            RecordingInput::new(imported, &segs, SpeakerDirectory::from_segments(&segs)),
            input("early", "Frueh", 1_000, segs.clone()),
        ];
        order_chronologically(&mut inputs);
        let ids: Vec<_> = inputs.iter().map(|i| i.meeting.id.as_str()).collect();
        assert_eq!(ids, vec!["early", "imp", "late"]);
    }

    // -- Das Transkript: Bloecke je Aufnahme -------------------------------------

    #[test]
    fn the_corpus_has_one_header_per_recording_and_ids_for_every_segment() {
        let inputs = vec![
            input("a", "Kick-off", 1_790_000_000, segments(3, "Das Budget")),
            input("b", "Review", 1_790_086_400, segments(2, "Der Termin")),
        ];
        let corpus = build_corpus(&inputs);
        assert_eq!(corpus.recordings.len(), 2);
        assert_eq!(corpus.recordings[0].index, 1);
        assert_eq!(corpus.recordings[1].meeting_id, "b");
        assert_eq!(corpus.starts, vec![0, 4, 7]);
        assert_eq!(corpus.lines.len(), 7);
        assert!(
            corpus.lines[0].starts_with("=== R1 · Kick-off · 2026-"),
            "{}",
            corpus.lines[0]
        );
        assert!(corpus.lines[4].starts_with("=== R2 · Review ·"));
        assert!(corpus.lines[1].starts_with("R1S0 "), "{}", corpus.lines[1]);
        assert!(corpus.lines[1].contains("[00:00]") && corpus.lines[1].ends_with("Das Budget"));
        assert!(corpus.lines[6].starts_with("R2S1 ") && corpus.lines[6].contains("[00:10]"));
        // Kopfzeilen belegen nichts.
        assert!(corpus.refs[0].is_none() && corpus.refs[4].is_none());
        assert_eq!(corpus.refs.len(), corpus.lines.len());
        assert_eq!(corpus.recordings[0].segments, 3);
    }

    #[test]
    fn blank_segments_are_left_out_and_unsorted_ones_are_put_in_time_order() {
        let segs = vec![
            seg(2, 20_000, "Zweite Aussage"),
            seg(0, 0, "Erste Aussage"),
            seg(1, 10_000, "   \n  "),
        ];
        let corpus = build_corpus(&[input("a", "T", 1_000, segs)]);
        assert_eq!(corpus.lines.len(), 3, "Kopf + zwei Aussagen");
        assert!(corpus.lines[1].contains("Erste Aussage"));
        assert!(corpus.lines[2].contains("Zweite Aussage"));
        assert_eq!(corpus.recordings[0].segments, 2);
        assert!(
            corpus.resolve("R1S1").is_none(),
            "leeres Segment ist kein Beleg"
        );
    }

    #[test]
    fn a_multi_line_segment_stays_one_line() {
        let corpus = build_corpus(&[input(
            "a",
            "T",
            1,
            vec![seg(0, 0, "Zeile eins\nZeile zwei\r\n")],
        )]);
        assert_eq!(corpus.lines.len(), 2);
        assert!(corpus.lines[1].ends_with("Zeile eins Zeile zwei"));
    }

    #[test]
    fn a_title_cannot_break_the_header_line() {
        let long = format!("{}\nNeue Zeile", "T".repeat(300));
        let corpus = build_corpus(&[input("a", &long, 1, segments(1, "x"))]);
        assert!(!corpus.lines[0].contains('\n'));
        assert!(corpus.recordings[0].title.chars().count() <= MAX_TITLE_CHARS + 1);
        assert!(corpus.recordings[0].title.ends_with('…'));
    }

    // -- Quellen ---------------------------------------------------------------------

    #[test]
    fn source_ids_are_read_leniently_and_strictly_enough() {
        for ok in [
            "R2S14", "r2s14", " R2-S14 ", "R2.S14", "R2 S14", "R2:S14", "R2_S14",
        ] {
            assert_eq!(parse_source_id(ok), Some((1, 14)), "{ok}");
        }
        assert_eq!(parse_source_id("R1S0"), Some((0, 0)));
        for bad in [
            "S14", "R0S1", "R2", "R2S", "RS2", "R2S1x", "14", "", "R2S1S2",
        ] {
            assert_eq!(parse_source_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_cited_id_resolves_to_recording_segment_and_time() {
        let corpus = build_corpus(&[
            input("m-a", "Kick-off", 1_000, segments(3, "x")),
            input("m-b", "Review", 2_000, segments(4, "y")),
        ]);
        let s = corpus.resolve("R2S3").unwrap();
        assert_eq!(s.recording, 2);
        assert_eq!(s.meeting_id, "m-b");
        assert_eq!(s.segment_index, 3);
        assert_eq!(s.start_ms, 30_000);
        assert_eq!(corpus.resolve("r1-s2").unwrap().meeting_id, "m-a");
        // Erfunden: gibt es nicht.
        for invented in ["R3S0", "R1S9", "R9S9", "R2S4", "S1", "irgendwas"] {
            assert!(corpus.resolve(invented).is_none(), "{invented}");
        }
    }

    // -- Blockgrenzen und Ueberlaenge --------------------------------------------------

    fn assert_partition(ranges: &[Range<usize>], total: usize) {
        let mut next = 0;
        for r in ranges {
            assert_eq!(r.start, next, "luecken- und ueberlappungsfrei: {ranges:?}");
            assert!(r.end > r.start, "kein leerer Block: {ranges:?}");
            next = r.end;
        }
        assert_eq!(next, total, "alle Zeilen sind in genau einem Block");
    }

    #[test]
    fn everything_fits_one_block_when_the_budget_is_large() {
        let corpus = build_corpus(&[
            input("a", "A", 1, segments(5, "kurz")),
            input("b", "B", 2, segments(5, "kurz")),
            input("c", "C", 3, segments(5, "kurz")),
        ]);
        let ranges = corpus.pack(1_000_000);
        assert_eq!(ranges, vec![0..corpus.lines.len()]);
    }

    #[test]
    fn a_long_recording_is_split_inside_itself_and_never_across_its_neighbour() {
        // A ist lang, B ist lang: kein Block darf beide mischen.
        let corpus = build_corpus(&[
            input("a", "A", 1, segments(40, &"wort ".repeat(20))),
            input("b", "B", 2, segments(40, &"wort ".repeat(20))),
        ]);
        let max = 1_200;
        let ranges = corpus.pack(max);
        assert_partition(&ranges, corpus.lines.len());
        assert!(ranges.len() > 4, "ueberlang: mehrere Bloecke je Aufnahme");
        for r in &ranges {
            let first_rec = corpus.starts.iter().rposition(|&s| s <= r.start).unwrap();
            let last_rec = corpus.starts.iter().rposition(|&s| s < r.end).unwrap();
            assert_eq!(
                first_rec, last_rec,
                "Block {r:?} ueberschreitet eine Aufnahmegrenze"
            );
        }
        // Jeder Block haelt das Limit ein (die Zeilen sind kuerzer als das Limit).
        let chars = corpus.line_chars();
        for r in &ranges {
            assert!(chars[r.clone()].iter().sum::<usize>() <= max, "{r:?}");
        }
    }

    #[test]
    fn small_neighbours_share_a_block_only_when_both_fit_whole() {
        // Je Aufnahme: Kopf + 2 kurze Zeilen. Limit reicht fuer zwei Aufnahmen,
        // nicht fuer drei.
        let corpus = build_corpus(&[
            input("a", "A", 1, segments(2, "kurzer Text")),
            input("b", "B", 2, segments(2, "kurzer Text")),
            input("c", "C", 3, segments(2, "kurzer Text")),
        ]);
        let per_recording: usize = corpus.line_chars()[0..3].iter().sum();
        let ranges = corpus.pack(per_recording * 2 + 5);
        assert_partition(&ranges, corpus.lines.len());
        assert_eq!(
            ranges,
            vec![0..6, 6..9],
            "zwei teilen sich einen Block, die dritte steht allein"
        );
        // Genau ein Zeichen zu wenig fuer zwei: jede Aufnahme steht allein.
        let alone = corpus.pack(per_recording * 2 - 1);
        assert_eq!(alone, vec![0..3, 3..6, 6..9]);
    }

    #[test]
    fn the_blocks_follow_the_chronological_order_of_the_recordings() {
        let mut inputs = vec![
            input("late", "Spaet", 3_000, segments(3, "x")),
            input("early", "Frueh", 1_000, segments(3, "x")),
            input("mid", "Mitte", 2_000, segments(3, "x")),
        ];
        order_chronologically(&mut inputs);
        let corpus = build_corpus(&inputs);
        let ids: Vec<_> = corpus
            .recordings
            .iter()
            .map(|r| r.meeting_id.as_str())
            .collect();
        assert_eq!(ids, vec!["early", "mid", "late"]);
        let order: Vec<_> = corpus.refs.iter().flatten().map(|l| l.recording).collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(order, sorted, "R1 vor R2 vor R3 im ganzen Transkript");
    }

    #[test]
    fn a_single_oversized_line_gets_its_own_block_and_is_not_cut() {
        let mut segs = segments(3, "kurz");
        segs.insert(1, seg(99, 5_000, &"riesig ".repeat(500)));
        let corpus = build_corpus(&[input("a", "A", 1, segs)]);
        let ranges = corpus.pack(300);
        assert_partition(&ranges, corpus.lines.len());
        let big = corpus
            .lines
            .iter()
            .position(|l| l.starts_with("R1S99 "))
            .unwrap();
        assert!(
            ranges.iter().any(|r| *r == (big..big + 1)),
            "die lange Zeile steht allein: {ranges:?}"
        );
    }

    #[test]
    fn packing_is_a_partition_for_many_shapes() {
        for recordings in 1..=5usize {
            for lines_each in [1u32, 2, 7, 30] {
                for max in [50usize, 200, 1_000, 100_000] {
                    let inputs: Vec<_> = (0..recordings)
                        .map(|i| {
                            input(
                                &format!("m{i}"),
                                &format!("Aufnahme {i}"),
                                1_000 + i as i64,
                                segments(lines_each, "ein Satz mit etwas Inhalt"),
                            )
                        })
                        .collect();
                    let corpus = build_corpus(&inputs);
                    assert_partition(&corpus.pack(max), corpus.lines.len());
                }
            }
        }
    }

    // -- Luecken ------------------------------------------------------------------------

    #[test]
    fn a_gap_is_named_per_recording_with_its_time_span() {
        let corpus = build_corpus(&[
            input("a", "A", 1, segments(4, "x")),
            input("b", "B", 2, segments(4, "y")),
        ]);
        // Zeilen 3..8: R1 Segmente 2 und 3 (Zeilen 3, 4), R2-Kopf (5), R2 Segmente 0 und 1.
        let labels = corpus.gap_labels(&(3..8));
        assert_eq!(
            labels,
            vec![
                "Aufnahme 1, 00:20-00:33".to_string(),
                "Aufnahme 2, 00:00-00:13".to_string()
            ]
        );
        // Nur eine Kopfzeile: nichts zu melden.
        assert!(corpus.gap_labels(&(0..1)).is_empty());
    }

    // -- Auswahl --------------------------------------------------------------------------

    #[test]
    fn the_selection_is_cleaned_and_bounded() {
        let ids = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            clean_selection(&ids(&["b", " a ", "b", "", "c"])).unwrap(),
            ids(&["b", "a", "c"]),
            "ohne Doppelte, in Reihenfolge der Nennung"
        );
        assert_eq!(
            clean_selection(&ids(&["", "  "])).unwrap_err().code,
            CODE_NO_SELECTION
        );
        assert_eq!(clean_selection(&[]).unwrap_err().code, CODE_NO_SELECTION);
        let many: Vec<String> = (0..=MAX_RECORDINGS).map(|i| format!("m{i}")).collect();
        assert_eq!(clean_selection(&many).unwrap_err().code, CODE_TOO_MANY);
        assert_eq!(
            clean_selection(&many[..MAX_RECORDINGS]).unwrap().len(),
            MAX_RECORDINGS
        );
    }

    #[test]
    fn a_recording_without_a_transcript_is_refused_with_the_reason() {
        let with_text = segments(2, "Inhalt");
        let blank = vec![seg(0, 0, "  "), seg(1, 1_000, "")];
        let live = |status: &str| meeting("m", "T", status, "live", Some(1), 1);

        assert_eq!(eligibility(&live("ready"), &with_text), Eligibility::Ok);
        assert_eq!(eligibility(&live("failed"), &with_text), Eligibility::Ok);
        assert_eq!(eligibility(&live("cancelled"), &with_text), Eligibility::Ok);

        assert_eq!(eligibility(&live("ready"), &[]), Eligibility::NoTranscript);
        assert_eq!(
            eligibility(&live("ready"), &blank),
            Eligibility::NoTranscript
        );
        let empty = meeting("e", "Neue Besprechung", "ready", "empty", None, 1);
        assert_eq!(eligibility(&empty, &[]), Eligibility::EmptyEntry);

        for status in ["recording", "processing", "queued"] {
            assert_eq!(
                eligibility(&live(status), &with_text),
                Eligibility::NotFinished,
                "{status}: das Transkript waechst noch"
            );
        }
        assert_eq!(Eligibility::Ok.code(), None);
        assert_eq!(Eligibility::NoTranscript.code(), Some("no_transcript"));
        assert_eq!(
            Eligibility::NotFinished.code(),
            Some("meeting_not_finished")
        );
        assert_eq!(Eligibility::EmptyEntry.code(), Some("empty_entry"));
    }

    #[test]
    fn the_facts_block_lists_every_recording_in_order_and_the_user_description() {
        let corpus = build_corpus(&[
            input("a", "Kick-off", 1_790_000_000, segments(2, "x")),
            input("b", "Review", 1_790_086_400, segments(2, "y")),
        ]);
        let block = corpus.facts_block(
            "Kunde Stadtwerke",
            &["Thema: Angebot".to_string(), String::new()],
        );
        assert!(block.contains("Project: Kunde Stadtwerke"));
        let kick = block.find("- R1: Kick-off").unwrap();
        let review = block.find("- R2: Review").unwrap();
        assert!(kick < review);
        assert!(block.contains("Description (written by the user; background context only"));
        assert_eq!(
            block.matches("Description").count(),
            1,
            "nur wo es eine gibt"
        );
        assert!(block.contains("30:00"), "Dauer als Fakt: {block}");
    }
}
