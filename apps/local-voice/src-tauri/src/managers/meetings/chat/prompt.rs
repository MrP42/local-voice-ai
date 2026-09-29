//! Prompts des Chats (M4 §6). Rein, keine I/O.
//!
//! Reihenfolge im Nutzerprompt: `Heute` · `Auszüge` · `Überblick` · `Verlauf`
//! · `Frage`. Die Auszuege stehen vorn, damit eine Folgefrage im selben
//! Verlauf (gleiche Auszuege zuerst) den Prompt-Cache von llama-server trifft
//! (M6: Folgefragen 4-13 s statt 111 s auf CPU); was sich je Frage aendert
//! (Karten, Verlauf, Frage), steht dahinter. Freitext statt JSON-Schema,
//! damit gestreamt werden kann.

use super::context::{clip_chars, Excerpt, MeetingCard};
use super::ChunkSource;
use crate::managers::meetings::search::chunking::clock;
use crate::managers::meetings::search::index::ChatMessageRow;

/// System-Prompt (Deutsch; die Antwortsprache folgt der Frage). Der Beispiel-
/// satz mit `[Q3]` hat sich im Smoke M6 bewaehrt (8/8 korrekt zitiert).
pub const SYSTEM_PROMPT: &str = "Du beantwortest Fragen zu Besprechungen des Nutzers. Nutze NUR die Auszüge unten.
Belege jede Aussage direkt dahinter mit der Auszugs-ID in eckigen Klammern, z. B. [Q3]. Mehrere: [Q3][Q7].
Erfinde keine Namen, Zahlen, Termine. Wenn die Auszüge die Frage nicht beantworten, schreibe genau: KEIN_BELEG
Antworte knapp, in der Sprache der Frage, ohne Überschriften. Datum der Besprechung angeben, wenn mehrere beteiligt sind.";

/// Laengster Verlaufsbeitrag im Prompt (eine lange Antwort soll den Verlauf
/// nicht allein fuellen).
const HISTORY_ITEM_CHARS: usize = 600;

/// Datum `TT.MM.JJJJ` in Ortszeit; leer bei ungueltigem Zeitstempel.
pub fn date_de(secs: Option<i64>) -> String {
    secs.and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%d.%m.%Y")
                .to_string()
        })
        .unwrap_or_default()
}

/// Kopfzeile eines Auszugs: `[Q5] B2 · Transkript 03:15–04:40`.
pub fn excerpt_heading(ex: &Excerpt) -> String {
    let what = match ex.source {
        ChunkSource::Transcript => match (ex.start_ms, ex.end_ms) {
            (Some(s), Some(e)) => format!("Transkript {}–{}", clock(s), clock(e)),
            (Some(s), None) => format!("Transkript ab {}", clock(s)),
            _ => "Transkript".to_string(),
        },
        ChunkSource::UserNotes => "Meine Notizen".to_string(),
        ChunkSource::AiNotes => match &ex.section {
            Some(section) => format!("KI-Notizen, Abschnitt {section}"),
            None => "KI-Notizen".to_string(),
        },
        ChunkSource::Title => "Titel".to_string(),
    };
    format!("[Q{}] {} · {}", ex.qid, ex.meeting_label, what)
}

/// Karte: `B2 "Titel" 12.09.2026 · Ordner Vertrieb · <Kurzfassung>`.
pub fn card_line(card: &MeetingCard) -> String {
    let mut line = format!("{} \"{}\"", card.label, card.meeting.title.trim());
    let date = date_de(card.meeting.started_at);
    if !date.is_empty() {
        line.push(' ');
        line.push_str(&date);
    }
    if !card.folders.is_empty() {
        line.push_str(" · Ordner ");
        line.push_str(&card.folders.join(", "));
    }
    if !card.summary.is_empty() {
        line.push_str(" · ");
        line.push_str(&card.summary);
    }
    line
}

/// Verlauf fuer den Prompt: die juengsten Beitraege, die in `max_chars`
/// passen, chronologisch. Anzeige-Nummern `[n]` fallen weg (das Modell
/// kennt nur `[Q<k>]` der aktuellen Auszuege).
pub fn render_history(messages: &[ChatMessageRow], max_chars: usize) -> String {
    let marks = regex::Regex::new(r"\s*\[\d+\]").expect("Regex");
    let mut picked: Vec<String> = Vec::new();
    let mut used = 0usize;
    for msg in messages.iter().rev() {
        let who = if msg.role == "user" {
            "Nutzer"
        } else {
            "Assistent"
        };
        let text = marks.replace_all(&msg.content, "");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let text = if text.is_empty() && msg.role != "user" {
            "(kein Beleg gefunden)".to_string()
        } else {
            clip_chars(&text, HISTORY_ITEM_CHARS)
        };
        let line = format!("{who}: {text}");
        let cost = line.chars().count() + 1;
        if used + cost > max_chars {
            break;
        }
        used += cost;
        picked.push(line);
    }
    picked.reverse();
    picked.join("\n")
}

pub struct PromptInput<'a> {
    /// `TT.MM.JJJJ`
    pub today: &'a str,
    /// Aufnahme laeuft: Position des Transkript-Endes.
    pub live_position_ms: Option<u64>,
    pub cards: &'a [MeetingCard],
    pub excerpts: &'a [Excerpt],
    /// Bereits gerendert (`render_history`), leer = neuer Verlauf.
    pub history: &'a str,
    pub question: &'a str,
}

pub fn build_user_prompt(p: &PromptInput<'_>) -> String {
    let mut out = format!("Heute: {}\n", p.today);
    if let Some(pos) = p.live_position_ms {
        out.push_str(&format!(
            "Die Besprechung läuft noch; das Transkript reicht bis {}.\n",
            clock(pos)
        ));
    }
    out.push_str("\nAuszüge:\n");
    for ex in p.excerpts {
        out.push_str(&excerpt_heading(ex));
        out.push('\n');
        out.push_str(&ex.body());
        out.push_str("\n\n");
    }
    if !p.cards.is_empty() {
        out.push_str("Überblick:\n");
        for card in p.cards {
            out.push_str(&card_line(card));
            out.push('\n');
        }
        out.push('\n');
    }
    if !p.history.trim().is_empty() {
        out.push_str("Verlauf:\n");
        out.push_str(p.history.trim());
        out.push_str("\n\n");
    }
    out.push_str("Frage: ");
    out.push_str(p.question.trim());
    out
}

#[cfg(test)]
mod tests {
    use super::super::context::tests::{meeting, seg};
    use super::super::context::{number_excerpts, transcript_blocks};
    use super::*;

    fn row(role: &str, content: &str) -> ChatMessageRow {
        ChatMessageRow {
            id: "x".into(),
            thread_id: "t".into(),
            role: role.into(),
            content: content.into(),
            citations_json: None,
            coverage_json: None,
            created_at: 0,
        }
    }

    #[test]
    fn the_system_prompt_demands_ids_and_the_not_found_token() {
        assert!(SYSTEM_PROMPT.contains("[Q3]"));
        assert!(SYSTEM_PROMPT.contains("KEIN_BELEG"));
        assert!(SYSTEM_PROMPT.contains("NUR die Auszüge"));
    }

    #[test]
    fn prompt_lists_excerpts_before_overview_history_and_question() {
        let mut excerpts = transcript_blocks(
            &meeting("a"),
            &[seg(12, 195, "Das Budget steht."), seg(13, 280, "Gut.")],
            0,
            10_000,
        );
        let mut notes = excerpts[0].clone();
        notes.source = ChunkSource::AiNotes;
        notes.section = Some("Entscheidungen".into());
        excerpts.push(notes);
        let mut cards = vec![MeetingCard {
            label: String::new(),
            meeting: meeting("a"),
            folders: vec!["Vertrieb".into()],
            summary: "Budget freigegeben".into(),
        }];
        number_excerpts(&mut cards, &mut excerpts);
        let prompt = build_user_prompt(&PromptInput {
            today: "29.09.2026",
            live_position_ms: Some(290_000),
            cards: &cards,
            excerpts: &excerpts,
            history: "Nutzer: Vorher?",
            question: "Wie hoch ist das Budget?",
        });
        assert!(prompt.starts_with("Heute: 29.09.2026\n"));
        assert!(prompt.contains("[Q1] B1 · Transkript 03:15–04:44\nS12 03:15 Ich: Das Budget steht.\nS13 04:40 Gegenseite: Gut."));
        assert!(prompt.contains("[Q2] B1 · KI-Notizen, Abschnitt Entscheidungen"));
        assert!(prompt
            .contains("B1 \"Besprechung a\" 12.09.2026 · Ordner Vertrieb · Budget freigegeben"));
        assert!(prompt.contains("das Transkript reicht bis 04:50"));
        let pos = |s: &str| prompt.find(s).unwrap();
        assert!(pos("Auszüge:") < pos("Überblick:"));
        assert!(pos("Überblick:") < pos("Verlauf:"));
        assert!(pos("Verlauf:") < pos("Frage: Wie hoch ist das Budget?"));
        assert!(prompt.ends_with("Frage: Wie hoch ist das Budget?"));
    }

    #[test]
    fn history_keeps_the_newest_turns_within_budget_without_display_numbers() {
        let msgs = vec![
            row("user", "Erste Frage ganz alt"),
            row("assistant", "Alte Antwort [1]"),
            row("user", "Wie hoch ist das Budget?"),
            row("assistant", "Es sind 5 000 Euro [1][2]."),
            row("assistant", ""),
        ];
        let all = render_history(&msgs, 10_000);
        assert_eq!(
            all,
            "Nutzer: Erste Frage ganz alt\nAssistent: Alte Antwort\nNutzer: Wie hoch ist das Budget?\nAssistent: Es sind 5 000 Euro.\nAssistent: (kein Beleg gefunden)"
        );
        let newest = render_history(&msgs, 100);
        assert!(newest.starts_with("Nutzer: Wie hoch"), "war: {newest}");
        assert!(!newest.contains("ganz alt"));
        assert_eq!(render_history(&[], 100), "");
    }
}
