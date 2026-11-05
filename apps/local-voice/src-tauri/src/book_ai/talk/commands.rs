//! Sprachbefehle ohne Modell (Plan D7), dieselbe Liste wie auf iOS
//! (Swift-Kern ist die Referenz, `BookTalkIntent.swift`).
//!
//! Erkennung: die ganze Äußerung, normalisiert (klein, ä→ae, ohne
//! Satzzeichen), Füllwörter ignoriert, höchstens acht Wörter. Was zusätzlich
//! eine Frage enthält, ist kein Befehl.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::lex::norm_words;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BookTalkCommandKind {
    Jump,
    ReadCitation,
    /// „Ja“: nach der Rückfrage „Soll ich dir die Stelle vorlesen?“ gleich
    /// `ReadCitation`, sonst eine Frage (siehe `resolve_confirm`).
    Confirm,
    Resume,
    ScopeBook,
    ScopePosition,
    Stop,
    End,
}

/// Ein Sprachbefehl, Form wie `Command` im Vertrag: `{ "kind": "jump" }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BookTalkCommand {
    pub kind: BookTalkCommandKind,
}

const FILLERS: &[&str] = &[
    "bitte", "mal", "doch", "jetzt", "mir", "uns", "hey", "please", "just", "now", "kurz",
];

use BookTalkCommandKind as K;

/// Formen sind bereits normalisiert; Vergleich über die ganze Äußerung.
const TABLE: &[(K, &[&str])] = &[
    (
        K::Jump,
        &[
            "spring dahin",
            "spring hin",
            "spring zur stelle",
            "zeig die stelle",
            "zeig sie",
            "geh zu der stelle",
            "geh zur stelle",
            "geh dahin",
            "gehe dahin",
            "jump there",
            "jump to it",
            "go there",
            "show the passage",
            "take me there",
            "show me the passage",
        ],
    ),
    (
        K::ReadCitation,
        &[
            "lies die stelle vor",
            "lies das vor",
            "lies es vor",
            "lies sie vor",
            "lies vor",
            "lies die stelle",
            "vorlesen",
            "read it to me",
            "read it to us",
            "read it",
            "read that",
            "read it out",
            "read it aloud",
            "read the passage",
            "read the passage to me",
            "read that to me",
        ],
    ),
    (
        K::Confirm,
        &[
            "ja",
            "jo",
            "jawohl",
            "gerne",
            "klar",
            "ja gerne",
            "ja klar",
            "yes",
            "yeah",
            "sure",
            "okay",
            "yes please",
        ],
    ),
    (
        K::Resume,
        &[
            "weiterlesen",
            "lies weiter",
            "weiter",
            "mach weiter",
            "mach weiter wo ich war",
            "keep reading",
            "continue",
            "continue reading",
            "resume",
            "read on",
            "carry on",
        ],
    ),
    (
        K::ScopeBook,
        &[
            "ganzes buch",
            "das ganze buch",
            "auch spaetere kapitel",
            "ganzes buch erlauben",
            "whole book",
            "the whole book",
            "entire book",
            "full book",
            "include later chapters",
            "also later chapters",
        ],
    ),
    (
        K::ScopePosition,
        &[
            "nur bis hier",
            "bis hier",
            "nur bis zur leseposition",
            "nur bis zur aktuellen stelle",
            "spoilerschutz an",
            "only up to here",
            "up to here",
            "only until here",
            "no spoilers",
        ],
    ),
    (
        K::Stop,
        &[
            "stopp",
            "stop",
            "halt",
            "halt stopp",
            "sei still",
            "ruhe",
            "hoer auf",
            "aufhoeren",
            "be quiet",
            "shut up",
        ],
    ),
    (
        K::End,
        &[
            "gespraech beenden",
            "beende das gespraech",
            "beende gespraech",
            "gespraech ende",
            "ende",
            "tschuess",
            "end conversation",
            "end the conversation",
            "goodbye",
            "bye",
        ],
    ),
];

/// Ganze Äußerung muss ein Befehl sein; alles mit zusätzlicher Frage ist keiner.
pub fn detect(text: &str) -> Option<BookTalkCommandKind> {
    let words: Vec<String> = norm_words(text)
        .into_iter()
        .filter(|w| !FILLERS.contains(&w.as_str()))
        .collect();
    if words.is_empty() || words.len() > 8 {
        return None;
    }
    let joined = words.join(" ");
    TABLE
        .iter()
        .rev()
        .find(|(_, forms)| forms.contains(&joined.as_str()))
        .map(|(kind, _)| *kind)
}

/// „Ja“ ist nur nach der Rückfrage „Soll ich dir die Stelle vorlesen?“ ein
/// Befehl (`ReadCitation`), sonst eine Frage.
pub fn resolve_confirm(
    kind: BookTalkCommandKind,
    offered_read: bool,
) -> Option<BookTalkCommandKind> {
    match (kind, offered_read) {
        (BookTalkCommandKind::Confirm, true) => Some(BookTalkCommandKind::ReadCitation),
        (BookTalkCommandKind::Confirm, false) => None,
        (other, _) => Some(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book_ai::talk::testkit::befehle;

    fn code(c: Option<BookTalkCommandKind>) -> Option<String> {
        c.map(|c| {
            serde_json::to_value(c)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
    }

    #[test]
    fn every_fixture_command_is_recognised() {
        for b in befehle() {
            assert_eq!(code(detect(&b.text)), b.command, "Äußerung: {}", b.text);
        }
    }

    #[test]
    fn a_yes_is_a_command_only_after_the_offer() {
        assert_eq!(
            resolve_confirm(BookTalkCommandKind::Confirm, true),
            Some(BookTalkCommandKind::ReadCitation)
        );
        assert_eq!(resolve_confirm(BookTalkCommandKind::Confirm, false), None);
        assert_eq!(
            resolve_confirm(BookTalkCommandKind::Stop, false),
            Some(BookTalkCommandKind::Stop)
        );
    }

    #[test]
    fn a_question_around_a_command_word_is_no_command() {
        assert_eq!(
            detect("Ja, aber wer ist Anselm und was hat er getan im Hof"),
            None
        );
        assert_eq!(detect("Stopp!"), Some(BookTalkCommandKind::Stop));
    }
}
