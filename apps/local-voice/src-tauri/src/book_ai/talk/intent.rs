//! Absicht einer Frage (Plan §3.2, Regeln auf Deutsch und Englisch) und
//! Sprache der Frage. Rein, ohne LLM. Gleiche Listen und Reihenfolge wie der
//! Swift-Kern (Referenz, `BookTalkIntent.swift`).

use serde::{Deserialize, Serialize};
use specta::Type;

use super::lex::norm_words;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BookTalkIntent {
    Recap,
    Locate,
    Explain,
    Question,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
}

const RECAP: &[&str] = &[
    "bisher",
    "was ist passiert",
    "was passiert ist",
    "was geschah",
    "verpasst",
    "zusammenfass",
    "zusammengefasst",
    "so far",
    "recap",
    "what happened",
    "summari",
    "i missed",
    "have i missed",
    "catch me up",
];
const LOCATE_WORDS: &[&str] = &["wo", "where"];
const LOCATE: &[&str] = &[
    "an welcher stelle",
    "welche seite",
    "welcher seite",
    "auf welcher seite",
    "zum ersten mal",
    "erstmals",
    "wann wird",
    "which page",
    "what page",
    "first time",
    "first mention",
    "which chapter",
    "welches kapitel",
];
const EXPLAIN: &[&str] = &[
    "erklaer",
    "was bedeutet",
    "was heisst",
    "was versteht man",
    "gemeint",
    "bedeutung",
    "was ist ein ",
    "was ist eine ",
    "explain",
    "what does",
    "what is meant",
    "what do you mean",
    "meaning of",
    "define ",
];

/// Reihenfolge: recap, locate, explain, question. „Wo wird das erklärt?“ ist locate.
pub fn detect(text: &str) -> BookTalkIntent {
    let words = norm_words(text);
    let flat = format!(" {} ", words.join(" "));
    if RECAP.iter().any(|p| flat.contains(p)) {
        return BookTalkIntent::Recap;
    }
    if words.iter().any(|w| LOCATE_WORDS.contains(&w.as_str()))
        || LOCATE.iter().any(|p| flat.contains(p))
    {
        return BookTalkIntent::Locate;
    }
    if EXPLAIN.iter().any(|p| flat.contains(p)) {
        return BookTalkIntent::Explain;
    }
    BookTalkIntent::Question
}

const GERMAN_HINTS: &[&str] = &[
    "der", "die", "das", "und", "ist", "nicht", "was", "wer", "wo", "wie", "warum", "ich", "mir",
    "mein", "eine", "einen", "erklaer", "bitte", "von", "mit", "wird", "sind", "hat", "kapitel",
    "buch", "bisher", "gibt", "es", "zu", "im", "dem",
];
const ENGLISH_HINTS: &[&str] = &[
    "the", "and", "is", "not", "what", "who", "where", "how", "why", "me", "my", "explain",
    "please", "of", "with", "are", "has", "chapter", "book", "far", "there", "does", "did", "in",
    "to", "it",
];

/// Sprache der Frage (für System-Prompt und Sprechfassung); Standard Deutsch.
pub fn detect_lang(text: &str) -> Lang {
    if text.chars().any(|c| "äöüÄÖÜß".contains(c)) {
        return Lang::De;
    }
    let words = norm_words(text);
    let de = words
        .iter()
        .filter(|w| GERMAN_HINTS.contains(&w.as_str()))
        .count();
    let en = words
        .iter()
        .filter(|w| ENGLISH_HINTS.contains(&w.as_str()))
        .count();
    if en > de {
        Lang::En
    } else {
        Lang::De
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book_ai::talk::testkit::fragen;

    fn name(i: BookTalkIntent) -> &'static str {
        match i {
            BookTalkIntent::Recap => "recap",
            BookTalkIntent::Locate => "locate",
            BookTalkIntent::Explain => "explain",
            BookTalkIntent::Question => "question",
        }
    }

    #[test]
    fn every_fixture_question_gets_its_intent() {
        for f in fragen() {
            assert_eq!(name(detect(&f.question)), f.intent, "Frage {}", f.id);
        }
    }

    #[test]
    fn english_and_odd_cases() {
        assert_eq!(detect("What happened so far?"), BookTalkIntent::Recap);
        assert_eq!(
            detect("Where is the workshop described?"),
            BookTalkIntent::Locate
        );
        assert_eq!(
            detect("What does liability mean here?"),
            BookTalkIntent::Explain
        );
        assert_eq!(detect("Wer ist Konrad Ilg?"), BookTalkIntent::Question);
        assert_eq!(
            detect("Wann zum ersten Mal ist vom Haus die Rede?"),
            BookTalkIntent::Locate
        );
        // „wo“ nur als ganzes Wort.
        assert_eq!(detect("Der Wolf wohnt im Wald."), BookTalkIntent::Question);
    }

    #[test]
    fn the_language_is_detected() {
        assert_eq!(
            detect_lang("What happened to the old house in the story?"),
            Lang::En
        );
        assert_eq!(
            detect_lang("Was ist in diesem Kapitel bisher passiert?"),
            Lang::De
        );
        assert_eq!(detect_lang("Wähle das Haus"), Lang::De);
    }
}
