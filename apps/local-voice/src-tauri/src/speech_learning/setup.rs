//! Vorlesetext für die Einrichtung der Erkennung.
//!
//! Fester deutscher Grundtext (frei erfunden, mit typischen Verwaltungsbegriffen) plus
//! ein Satz mit dem eigenen Wortschatz (`custom_words`). Der Text enthält keine echten Daten.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::diff::{has_forbidden_chars, MAX_TEXT_CHARS};
use super::store::MAX_WORD_CHARS;

/// Höchstens so viele eigene Begriffe kommen in den Vorlesetext.
pub const MAX_WORDS_IN_TEXT: usize = 12;

const BASE: [&str; 6] = [
    "Der Bürgermeister bat die Verwaltung, den Bauantrag für die neue Kindertagesstätte bis Freitag zu prüfen.",
    "Im Haushaltsplan sind Mittel für die Sanierung der Grundschule und die Erneuerung der Straßenbeleuchtung vorgesehen.",
    "Das Ordnungsamt versendet den Gebührenbescheid zur Grundsteuer zusammen mit der Widerspruchsbelehrung.",
    "Vor der Sitzung des Gemeinderats wurde die Tagesordnung um einen Antrag zur Wohnraumförderung ergänzt.",
    "Die Datenschutzbeauftragte weist darauf hin, dass personenbezogene Daten nur mit einer Einwilligung verarbeitet werden dürfen.",
    "Zum Abschluss fasst die Sachbearbeiterin das Protokoll zusammen und verteilt es an alle Fachbereiche.",
];

/// Der Vorlesetext, einmal als Sätze (zum Anzeigen) und einmal am Stück (zum Vergleichen).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SetupText {
    pub sentences: Vec<String>,
    pub text: String,
}

/// Baut den Vorlesetext. Eigene Begriffe: getrimmt, ohne Doppelte (Groß-/Kleinschreibung
/// egal), ohne Zeichen, die in Texten Ärger machen, höchstens `MAX_WORDS_IN_TEXT`.
pub fn setup_text(custom_words: &[String]) -> SetupText {
    let mut sentences: Vec<String> = BASE.iter().map(|s| s.to_string()).collect();
    let mut words: Vec<String> = Vec::new();
    for raw in custom_words {
        let w = raw.trim();
        let skip = w.is_empty()
            || w.chars().count() > MAX_WORD_CHARS
            || has_forbidden_chars(w)
            || w.chars().any(char::is_control)
            || words.iter().any(|x| x.to_lowercase() == w.to_lowercase());
        if !skip {
            words.push(w.to_string());
        }
        if words.len() == MAX_WORDS_IN_TEXT {
            break;
        }
    }
    if !words.is_empty() {
        sentences.push(format!(
            "Außerdem noch einige Begriffe: {}.",
            words.join(", ")
        ));
    }
    let mut text = sentences.join(" ");
    while text.chars().count() > MAX_TEXT_CHARS && sentences.len() > BASE.len() {
        sentences.pop();
        text = sentences.join(" ");
    }
    SetupText { sentences, text }
}
