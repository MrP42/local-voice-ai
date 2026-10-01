//! G3 (#70, U9): Antwortformen, Pruefung und Markdown eines Projekt-Protokolls.
//!
//! Das Modell nennt je Eintrag Belege als Quellen-IDs (`R2S14`). Hier werden sie
//! deterministisch gegen das gemeinsame Transkript geprueft (`Corpus::resolve`):
//! was es nicht gibt, faellt weg und wird gezaehlt; ein Eintrag ohne gueltigen
//! Beleg bleibt stehen, ist aber `unsupported` markiert (sonst waere die
//! Belegquote geschoent, dieselbe Regel wie bei den KI-Notizen).

use std::collections::BTreeMap;

use serde::Deserialize;

use super::corpus::{source_id, Corpus};
use crate::managers::meetings::llm_call::{duration_label, mm_ss};
use crate::managers::meetings::minutes::{clean, clean_opt, one_line};
use crate::managers::meetings::notes::assemble::RawEntry;
use crate::managers::meetings::notes::classify::AutoOutcome;
use crate::managers::meetings::notes::model::{SectionKind, TemplateSpec};
use crate::managers::meetings::project_minutes_store::{
    EntrySource, ProjectEntry, ProjectKind, ProjectSection, SourceRecording,
};

/// Hoechstzahl Belege je Eintrag (mehr waere eine Wand aus Zeitmarken).
pub const MAX_SOURCES_PER_ENTRY: usize = 8;

// ---------------------------------------------------------------------------
// Antwortformen
// ---------------------------------------------------------------------------

/// Ein Eintrag der Antwort: ein Objekt, oder (tolerant, wenn ein Modell das
/// Schema nicht trifft) ein blosser Text ohne Belege.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum RawItem {
    Detailed(RawEntry),
    Plain(String),
}

impl RawItem {
    fn into_entry(self) -> RawEntry {
        match self {
            RawItem::Detailed(entry) => entry,
            RawItem::Plain(text) => RawEntry {
                text,
                ..RawEntry::default()
            },
        }
    }
}

/// Antwort des Einzeldurchlaufs und des Zusammenfuehrens: Abschnitts-ID -> Eintraege.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RawProject(pub BTreeMap<String, Vec<RawItem>>);

/// Antwort der map-Stufe: eine flache Liste, die Abschnitts-ID steht je Eintrag.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MapOutput {
    #[serde(default)]
    pub entries: Vec<RawEntry>,
}

pub fn raw_is_empty(raw: &RawProject) -> bool {
    raw.0
        .values()
        .all(|items| items.iter().all(|item| item_text(item).trim().is_empty()))
}

fn item_text(item: &RawItem) -> &str {
    match item {
        RawItem::Detailed(entry) => &entry.text,
        RawItem::Plain(text) => text,
    }
}

// ---------------------------------------------------------------------------
// Pruefung und Zusammenbau
// ---------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AssembleStats {
    /// Eintraege unter einer Abschnitts-ID, die die Vorlage nicht kennt.
    pub dropped_unknown: usize,
    /// Leere Eintraege.
    pub dropped_empty: usize,
    /// Wortgleiche Doppelte in einem Abschnitt (ihre Belege sind uebernommen).
    pub duplicates: usize,
    /// Quellen-IDs, die es im Transkript nicht gibt.
    pub dropped_sources: usize,
    /// Eintraege ohne gueltigen Beleg.
    pub unsupported: usize,
}

/// Gueltige Belege, chronologisch und ohne Doppelte; `dropped` zaehlt die
/// erfundenen. Eine Stelle, die zweimal genannt wird, ist kein Verwerfen.
fn resolve_sources(raw: &[String], corpus: &Corpus, dropped: &mut usize) -> Vec<EntrySource> {
    let mut kept: Vec<EntrySource> = Vec::new();
    for id in raw {
        match corpus.resolve(id) {
            Some(source) => {
                let known = kept.iter().any(|k| {
                    k.recording == source.recording && k.segment_index == source.segment_index
                });
                if !known {
                    kept.push(source);
                }
            }
            None => *dropped += 1,
        }
    }
    kept.sort_by_key(|s| (s.recording, s.start_ms, s.segment_index));
    kept.truncate(MAX_SOURCES_PER_ENTRY);
    kept
}

/// Baut aus der Antwort die Abschnitte der Vorlage, in ihrer Reihenfolge:
/// Texte bereinigt (eine Zeile), Leeres entfernt, Doppelte zusammengefuehrt
/// (Belege vereint), Belege geprueft, Verantwortliche und Termine nur in
/// Aufgaben-Abschnitten. Unbekannte Abschnitte fallen weg (gezaehlt, nicht still).
pub fn assemble(
    raw: RawProject,
    spec: &TemplateSpec,
    corpus: &Corpus,
) -> (Vec<ProjectSection>, AssembleStats) {
    let mut stats = AssembleStats::default();
    let mut raw = raw.0;
    let mut sections = Vec::with_capacity(spec.sections.len());
    for template in &spec.sections {
        let items = raw.remove(&template.id).unwrap_or_default();
        let mut entries: Vec<ProjectEntry> = Vec::new();
        for item in items {
            let item = item.into_entry();
            let text = clean(&item.text);
            if text.is_empty() {
                stats.dropped_empty += 1;
                continue;
            }
            let sources = resolve_sources(&item.sources, corpus, &mut stats.dropped_sources);
            let tasks = template.kind == SectionKind::Tasks;
            let duplicate = entries
                .iter_mut()
                .find(|known| known.text.to_lowercase() == text.to_lowercase());
            if let Some(known) = duplicate {
                stats.duplicates += 1;
                for source in sources {
                    let have = known.sources.iter().any(|k| {
                        k.recording == source.recording && k.segment_index == source.segment_index
                    });
                    if !have && known.sources.len() < MAX_SOURCES_PER_ENTRY {
                        known.sources.push(source);
                    }
                }
                known
                    .sources
                    .sort_by_key(|s| (s.recording, s.start_ms, s.segment_index));
                known.unsupported = known.sources.is_empty();
                continue;
            }
            entries.push(ProjectEntry {
                text,
                assignee: if tasks {
                    clean_opt(item.assignee.as_deref())
                } else {
                    None
                },
                due: if tasks {
                    clean_opt(item.due.as_deref())
                } else {
                    None
                },
                unsupported: sources.is_empty(),
                sources,
            });
        }
        stats.unsupported += entries.iter().filter(|e| e.unsupported).count();
        sections.push(ProjectSection {
            id: template.id.clone(),
            title: template.title.clone(),
            kind: template.kind,
            entries,
        });
    }
    stats.dropped_unknown = raw.values().map(Vec::len).sum();
    (sections, stats)
}

/// Fachliche Mindestanforderung: ein Protokoll ohne einen einzigen Eintrag ist
/// keines. Leere einzelne Abschnitte sind zulaessig.
pub fn validate_sections(sections: &[ProjectSection]) -> Result<(), String> {
    if sections.iter().all(|s| s.entries.is_empty()) {
        return Err("Protokoll ohne Inhalt".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Zusammenfuehren ohne Modellaufruf
// ---------------------------------------------------------------------------

/// Ohne Zusammenfuehren-Aufruf vereinen: die flachen Eintraege der map-Stufe
/// wandern in Blockreihenfolge in ihre Abschnitte (Belege bleiben dran).
pub fn merge_deterministic(collected: &[RawEntry], spec: &TemplateSpec) -> RawProject {
    let known: std::collections::HashSet<&str> =
        spec.sections.iter().map(|s| s.id.as_str()).collect();
    let mut merged: BTreeMap<String, Vec<RawItem>> = BTreeMap::new();
    for entry in collected {
        let Some(section) = entry.section.as_deref().filter(|s| known.contains(s)) else {
            continue;
        };
        merged
            .entry(section.to_string())
            .or_default()
            .push(RawItem::Detailed(RawEntry {
                section: None,
                ..entry.clone()
            }));
    }
    RawProject(merged)
}

/// Eine Zeile der Zusammenfuehren-Eingabe:
/// `section | text [| assignee | due] | sources: R1S3 R2S5`. Nur Belege, die es
/// gibt, und in ihrer geprueften Schreibweise.
pub fn partial_line(entry: &RawEntry, spec: &TemplateSpec, corpus: &Corpus) -> Option<String> {
    let section = spec
        .sections
        .iter()
        .find(|s| Some(s.id.as_str()) == entry.section.as_deref())?;
    let text = one_line(&entry.text);
    if text.is_empty() {
        return None;
    }
    let mut dropped = 0;
    let sources: Vec<String> = resolve_sources(&entry.sources, corpus, &mut dropped)
        .iter()
        .map(|s| source_id((s.recording - 1) as usize, s.segment_index))
        .collect();
    let mut line = format!("{} | {text}", section.id);
    if section.kind == SectionKind::Tasks {
        let assignee = clean_opt(entry.assignee.as_deref()).unwrap_or_default();
        let due = clean_opt(entry.due.as_deref()).unwrap_or_default();
        if !assignee.is_empty() || !due.is_empty() {
            line.push_str(&format!(" | {assignee} | {due}"));
        }
    }
    line.push_str(&format!(" | sources: {}", sources.join(" ")));
    Some(line)
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

fn date_label(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// `A1 03:15, A2 10:02` (Aufnahme-Nummer und Startzeit der Stelle).
fn source_marks(sources: &[EntrySource]) -> String {
    sources
        .iter()
        .map(|s| format!("A{} {}", s.recording, mm_ss(s.start_ms)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn entry_line(kind: SectionKind, entry: &ProjectEntry) -> String {
    let mut line = entry.text.clone();
    if kind == SectionKind::Tasks {
        let mut extras = Vec::new();
        if let Some(assignee) = &entry.assignee {
            extras.push(format!("Wer: {assignee}"));
        }
        if let Some(due) = &entry.due {
            extras.push(format!("Bis: {due}"));
        }
        if !extras.is_empty() {
            line.push_str(&format!(" _({})_", extras.join(", ")));
        }
    }
    if entry.sources.is_empty() {
        line.push_str(" _(ohne Beleg)_");
    } else {
        line.push_str(&format!(" [{}]", source_marks(&entry.sources)));
    }
    line
}

/// Titel eines Projekt-Protokolls: Art und Projektname.
pub fn document_title(kind: ProjectKind, project: &str) -> String {
    match kind {
        ProjectKind::Minutes => format!("Projekt-Protokoll: {}", one_line(project)),
        ProjectKind::Summary => format!("Projekt-Zusammenfassung: {}", one_line(project)),
    }
}

/// Das Projekt-Protokoll als Markdown: Kopf (Zeitraum, Aufnahmen, Vorlage), die
/// Abschnitte der Vorlage mit Belegen je Eintrag, die Quellaufnahmen als Tabelle,
/// bei Luecken der Hinweis.
pub fn project_markdown(
    kind: ProjectKind,
    project: &str,
    template_title: &str,
    auto: Option<AutoOutcome>,
    recordings: &[SourceRecording],
    sections: &[ProjectSection],
    gaps: &[String],
) -> String {
    let mut markdown = format!("# {}\n\n", document_title(kind, project));
    let suffix = match auto {
        None => "",
        Some(AutoOutcome::Model) => " (automatisch gewählt)",
        Some(AutoOutcome::Uncertain | AutoOutcome::Failed) => {
            " (Standardvorlage, Automatik nicht eindeutig)"
        }
    };
    let first = recordings.iter().map(|r| r.started_at).min().unwrap_or(0);
    let last = recordings.iter().map(|r| r.started_at).max().unwrap_or(0);
    let period = if date_label(first) == date_label(last) {
        format!("**Datum:** {}", date_label(first))
    } else {
        format!(
            "**Zeitraum:** {} bis {}",
            date_label(first),
            date_label(last)
        )
    };
    markdown.push_str(&format!(
        "{period} · **Aufnahmen:** {} · **Vorlage:** {template_title}{suffix}\n",
        recordings.len()
    ));

    for section in sections {
        markdown.push_str(&format!("\n## {}\n\n", section.title));
        match section.entries.as_slice() {
            [] => markdown.push_str("_keine_\n"),
            [only] if section.kind == SectionKind::Text => {
                markdown.push_str(&format!("{}\n", entry_line(section.kind, only)));
            }
            entries => {
                for entry in entries {
                    markdown.push_str(&format!("- {}\n", entry_line(section.kind, entry)));
                }
            }
        }
    }

    markdown.push_str("\n## Aufnahmen\n\n| Nr. | Aufnahme | Datum | Dauer |\n|---|---|---|---|\n");
    for recording in recordings {
        markdown.push_str(&format!(
            "| A{} | {} | {} | {} |\n",
            recording.index,
            recording.title.replace('|', "/"),
            date_label(recording.started_at),
            recording
                .duration_ms
                .map(duration_label)
                .unwrap_or_else(|| "–".to_string())
        ));
    }
    markdown.push_str(
        "\nDie Angaben in eckigen Klammern nennen die Aufnahme und die Zeit der Stelle im Gespräch.\n",
    );
    if !gaps.is_empty() {
        // Der Hinweis steht im Dokument selbst: wer es liest, muss sehen, dass
        // ein Teil der Gespräche nicht darin steckt.
        markdown.push_str(&format!(
            "\n> **Hinweis:** Das Protokoll ist unvollständig. Folgende Teile konnten nicht \
             ausgewertet werden und fehlen hier: {}. Die vollständigen Transkripte bleiben \
             erhalten.\n",
            gaps.join("; ")
        ));
    }
    markdown
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::corpus::{build_corpus, RecordingInput};
    use super::*;
    use crate::managers::meetings::notes::templates::builtin_templates;
    use crate::managers::meetings::speakers::SpeakerDirectory;
    use crate::managers::meetings::store::{Meeting, StoredSegment};

    fn spec_of(key: &str) -> TemplateSpec {
        builtin_templates()
            .into_iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, _, spec)| spec)
            .unwrap()
    }

    fn corpus() -> Corpus {
        let mk = |id: &str, title: &str, at: i64| {
            let segs: Vec<StoredSegment> = (0..4)
                .map(|i| StoredSegment {
                    segment_index: i,
                    text: format!("Aussage {i}"),
                    start_ms: u64::from(i) * 60_000,
                    end_ms: u64::from(i) * 60_000 + 5_000,
                    channel: 0,
                    speaker_index: None,
                    words: None,
                })
                .collect();
            let meeting = Meeting {
                id: id.into(),
                title: title.into(),
                status: "ready".into(),
                source: "live".into(),
                started_at: Some(at),
                ended_at: None,
                language: None,
                mic_audio_path: None,
                system_audio_path: None,
                duration_ms: Some(300_000),
                consent_confirmed_at: None,
                audio_retention_until: None,
                source_path: None,
                description: None,
                created_at: at,
                deleted_at: None,
            };
            let labels = SpeakerDirectory::from_segments(&segs);
            RecordingInput::new(meeting, &segs, labels)
        };
        build_corpus(&[
            mk("m-a", "Kick-off", 1_790_000_000),
            mk("m-b", "Review | Teil 2", 1_790_086_400),
        ])
    }

    fn raw_of(value: serde_json::Value) -> RawProject {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn valid_sources_become_chronological_references_with_audio_times() {
        let spec = spec_of("allgemein");
        let (sections, stats) = assemble(
            raw_of(json!({
                "zusammenfassung": [
                    {"text": "Budget und Termin stehen.", "sources": ["R2S1", "r1-s2", "R1S2"]}
                ]
            })),
            &spec,
            &corpus(),
        );
        let entry = &sections[0].entries[0];
        assert_eq!(entry.text, "Budget und Termin stehen.");
        assert!(!entry.unsupported);
        assert_eq!(
            entry.sources,
            vec![
                EntrySource {
                    recording: 1,
                    meeting_id: "m-a".into(),
                    segment_index: 2,
                    start_ms: 120_000
                },
                EntrySource {
                    recording: 2,
                    meeting_id: "m-b".into(),
                    segment_index: 1,
                    start_ms: 60_000
                },
            ],
            "chronologisch (Aufnahme, Zeit), die doppelte Nennung zaehlt einmal"
        );
        assert_eq!(stats.dropped_sources, 0);
        assert_eq!(stats.unsupported, 0);
    }

    #[test]
    fn invented_sources_are_dropped_and_counted_and_an_unsupported_entry_stays_marked() {
        let spec = spec_of("allgemein");
        let (sections, stats) = assemble(
            raw_of(json!({
                "zusammenfassung": [
                    {"text": "Mit einem guten und einem erfundenen Beleg.", "sources": ["R1S0", "R9S9", "S3"]},
                    {"text": "Nur erfundene Belege.", "sources": ["R7S1"]},
                    {"text": "Ganz ohne Beleg.", "sources": []}
                ]
            })),
            &spec,
            &corpus(),
        );
        let entries = &sections[0].entries;
        assert_eq!(
            entries.len(),
            3,
            "kein Eintrag wird wegen fehlender Belege geloescht"
        );
        assert_eq!(entries[0].sources.len(), 1);
        assert!(!entries[0].unsupported);
        assert!(entries[1].unsupported && entries[1].sources.is_empty());
        assert!(entries[2].unsupported);
        assert_eq!(stats.dropped_sources, 3);
        assert_eq!(stats.unsupported, 2);
    }

    #[test]
    fn duplicate_statements_merge_their_sources_instead_of_losing_them() {
        let spec = spec_of("allgemein");
        let (sections, stats) = assemble(
            raw_of(json!({
                "entscheidungen": [
                    {"text": "Das Angebot wird angenommen.", "sources": ["R1S1"]},
                    {"text": "  das angebot wird ANGENOMMEN. ", "sources": ["R2S0", "R1S1"]},
                    {"text": "Etwas anderes.", "sources": ["R1S3"]}
                ]
            })),
            &spec,
            &corpus(),
        );
        let decisions = sections.iter().find(|s| s.id == "entscheidungen").unwrap();
        assert_eq!(decisions.entries.len(), 2);
        assert_eq!(
            decisions.entries[0]
                .sources
                .iter()
                .map(|s| (s.recording, s.segment_index))
                .collect::<Vec<_>>(),
            vec![(1, 1), (2, 0)],
            "beide Belege, ohne Doppelte, chronologisch"
        );
        assert_eq!(stats.duplicates, 1);
    }

    #[test]
    fn unknown_sections_and_empty_entries_are_counted_and_assignees_stay_in_task_sections() {
        let spec = spec_of("allgemein");
        let (sections, stats) = assemble(
            raw_of(json!({
                "gibt_es_nicht": [{"text": "x", "sources": ["R1S0"]}],
                "besprochene_punkte": [
                    {"text": "   ", "sources": ["R1S0"]},
                    {"text": "Punkt", "assignee": "Anna", "due": "Freitag", "sources": ["R1S0"]}
                ],
                "aufgaben": [
                    {"text": "Angebot schicken", "assignee": " Anna ", "due": "", "sources": ["R1S1"]}
                ]
            })),
            &spec,
            &corpus(),
        );
        assert_eq!(stats.dropped_unknown, 1);
        assert_eq!(stats.dropped_empty, 1);
        let points = sections
            .iter()
            .find(|s| s.id == "besprochene_punkte")
            .unwrap();
        assert_eq!(
            points.entries[0].assignee, None,
            "nur Aufgaben tragen Verantwortliche"
        );
        assert_eq!(points.entries[0].due, None);
        let tasks = sections.iter().find(|s| s.id == "aufgaben").unwrap();
        assert_eq!(tasks.entries[0].assignee.as_deref(), Some("Anna"));
        assert_eq!(tasks.entries[0].due, None);
        // Reihenfolge und Titel der Vorlage.
        let ids: Vec<_> = sections.iter().map(|s| s.id.as_str()).collect();
        let want: Vec<_> = spec.sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, want);
    }

    #[test]
    fn the_answer_is_read_leniently() {
        // Ein blosser Text statt eines Objekts, Belege als Text oder null, Zahl.
        let raw = raw_of(json!({
            "zusammenfassung": [
                "Nur ein Text",
                {"text": "Mit einem Beleg als Text", "sources": "R1S0"},
                {"text": "Mit null", "sources": null, "assignee": null},
                {"text": "Mit Zahl", "sources": [3]}
            ]
        }));
        let (sections, stats) = assemble(raw, &spec_of("allgemein"), &corpus());
        let e = &sections[0].entries;
        assert_eq!(e.len(), 4);
        assert!(e[0].unsupported);
        assert_eq!(e[1].sources.len(), 1);
        assert!(e[2].unsupported);
        assert!(
            e[3].unsupported,
            "die Zahl 3 ist keine Quellen-ID dieses Formats"
        );
        assert_eq!(stats.dropped_sources, 1);
    }

    #[test]
    fn a_minutes_document_without_any_entry_is_not_valid() {
        let (sections, _) = assemble(raw_of(json!({})), &spec_of("allgemein"), &corpus());
        assert!(validate_sections(&sections).is_err());
        let (sections, _) = assemble(
            raw_of(json!({"zusammenfassung": [{"text": "x", "sources": []}]})),
            &spec_of("allgemein"),
            &corpus(),
        );
        assert!(validate_sections(&sections).is_ok());
        assert!(raw_is_empty(&raw_of(
            json!({"a": [{"text": " "}], "b": []})
        )));
        assert!(!raw_is_empty(&raw_of(json!({"a": ["x"]}))));
    }

    // -- Zusammenfuehren -------------------------------------------------------------------

    fn entry(section: &str, text: &str, sources: &[&str]) -> RawEntry {
        RawEntry {
            section: Some(section.into()),
            text: text.into(),
            sources: sources.iter().map(|s| s.to_string()).collect(),
            ..RawEntry::default()
        }
    }

    #[test]
    fn the_merge_lines_carry_only_valid_sources_in_their_checked_spelling() {
        let spec = spec_of("allgemein");
        let c = corpus();
        let mut task = entry("aufgaben", "Angebot schicken", &["r2-s1", "R9S9"]);
        task.assignee = Some("Anna".into());
        assert_eq!(
            partial_line(
                &entry(
                    "zusammenfassung",
                    "Es geht um das Budget.",
                    &["R1S0", "R2S3"]
                ),
                &spec,
                &c
            )
            .unwrap(),
            "zusammenfassung | Es geht um das Budget. | sources: R1S0 R2S3"
        );
        assert_eq!(
            partial_line(&task, &spec, &c).unwrap(),
            "aufgaben | Angebot schicken | Anna |  | sources: R2S1"
        );
        assert!(partial_line(&entry("gibt_es_nicht", "x", &["R1S0"]), &spec, &c).is_none());
        assert!(partial_line(&entry("zusammenfassung", "  ", &["R1S0"]), &spec, &c).is_none());
        assert_eq!(
            partial_line(&entry("zusammenfassung", "Ohne Beleg", &[]), &spec, &c).unwrap(),
            "zusammenfassung | Ohne Beleg | sources: "
        );
    }

    #[test]
    fn the_deterministic_merge_keeps_block_order_and_the_sources() {
        let spec = spec_of("allgemein");
        let merged = merge_deterministic(
            &[
                entry("besprochene_punkte", "A", &["R1S0"]),
                entry("gibt_es_nicht", "weg", &["R1S0"]),
                entry("besprochene_punkte", "B", &["R2S1"]),
            ],
            &spec,
        );
        let (sections, stats) = assemble(merged, &spec, &corpus());
        let points = sections
            .iter()
            .find(|s| s.id == "besprochene_punkte")
            .unwrap();
        assert_eq!(
            points
                .entries
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        assert_eq!(points.entries[1].sources[0].meeting_id, "m-b");
        assert_eq!(
            stats.dropped_unknown, 0,
            "Unbekanntes gelangt gar nicht erst hinein"
        );
    }

    // -- Markdown ----------------------------------------------------------------------------

    #[test]
    fn the_markdown_names_recordings_sources_and_what_has_no_proof() {
        let spec = spec_of("allgemein");
        let c = corpus();
        let (sections, _) = assemble(
            raw_of(json!({
                "zusammenfassung": [{"text": "Das Budget steht.", "sources": ["R1S1", "R2S2"]}],
                "besprochene_punkte": [
                    {"text": "Punkt A", "sources": ["R1S0"]},
                    {"text": "Punkt B ohne Beleg", "sources": []}
                ],
                "aufgaben": [{"text": "Angebot schicken", "assignee": "Anna", "due": "Freitag", "sources": ["R2S0"]}]
            })),
            &spec,
            &c,
        );
        let md = project_markdown(
            ProjectKind::Minutes,
            "Kunde Stadtwerke",
            "Allgemein",
            Some(AutoOutcome::Model),
            &c.recordings,
            &sections,
            &[],
        );
        assert!(
            md.starts_with("# Projekt-Protokoll: Kunde Stadtwerke\n"),
            "{md}"
        );
        assert!(md.contains("**Zeitraum:** 2026-"), "{md}");
        assert!(md.contains("**Aufnahmen:** 2"));
        assert!(md.contains("**Vorlage:** Allgemein (automatisch gewählt)"));
        // Ein Text-Abschnitt mit genau einem Eintrag steht als Absatz.
        assert!(
            md.contains("## Zusammenfassung\n\nDas Budget steht. [A1 01:00, A2 02:00]\n"),
            "{md}"
        );
        assert!(md.contains("- Punkt A [A1 00:00]\n"));
        assert!(md.contains("- Punkt B ohne Beleg _(ohne Beleg)_\n"));
        assert!(md.contains("- Angebot schicken _(Wer: Anna, Bis: Freitag)_ [A2 00:00]\n"));
        assert!(md.contains("## Entscheidungen\n\n_keine_\n"));
        // Quellaufnahmen als Tabelle; das Pipe-Zeichen im Titel bricht sie nicht.
        assert!(md.contains("| A1 | Kick-off |"), "{md}");
        assert!(md.contains("| A2 | Review / Teil 2 |"), "{md}");
        assert!(!md.contains("Hinweis"), "ohne Luecken kein Hinweis");
    }

    #[test]
    fn a_gap_is_named_in_the_document_and_a_summary_has_its_own_title() {
        let spec = spec_of("allgemein");
        let c = corpus();
        let (sections, _) = assemble(
            raw_of(json!({"zusammenfassung": [{"text": "x", "sources": ["R1S0"]}]})),
            &spec,
            &c,
        );
        let md = project_markdown(
            ProjectKind::Summary,
            "P",
            "Allgemein",
            None,
            &c.recordings[..1],
            &sections,
            &["Aufnahme 2, 03:15-07:40".to_string()],
        );
        assert!(md.starts_with("# Projekt-Zusammenfassung: P\n"));
        assert!(md.contains("**Datum:** 2026-"), "eine Aufnahme: ein Datum");
        assert!(md.contains("> **Hinweis:** Das Protokoll ist unvollständig."));
        assert!(md.contains("Aufnahme 2, 03:15-07:40"));
        assert_eq!(
            document_title(ProjectKind::Minutes, "A\nB"),
            "Projekt-Protokoll: A B"
        );
    }
}
