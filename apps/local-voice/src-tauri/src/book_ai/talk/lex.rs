//! Wortwerkzeuge des Buch-Gesprächs: Normalisierung, Tokenisierung,
//! Stoppwörter, Stammkürzung. Gleiche Regeln wie der Swift-Kern (Referenz,
//! `BookTalkSearch.swift`/`BookTalkIntent.swift`), damit Suche und Belegprüfung
//! auf beiden Plattformen dieselben Ergebnisse liefern.

use std::collections::HashSet;
use std::sync::OnceLock;

const STOP: &[&str] = &[
    // de
    "der", "die", "das", "den", "dem", "des", "ein", "eine", "einen", "einem", "einer", "eines",
    "und", "oder", "aber", "ist", "sind", "war", "waren", "wird", "werden", "wurde", "hat",
    "haben", "hatte", "nicht", "kein", "keine", "ich", "du", "er", "sie", "es", "wir", "ihr",
    "mir", "mich", "dir", "dich", "mein", "dein", "sein", "von", "mit", "bei", "aus", "auf", "an",
    "in", "im", "zu", "zum", "zur", "fur", "uber", "um", "als", "wie", "was", "wer", "wo", "wann",
    "warum", "wieso", "welche", "welcher", "welches", "wird", "dass", "da", "dann", "auch", "noch",
    "nur", "so", "sich", "man", "gibt", "diesem", "dieser", "diese", "dieses", "bitte", "mal",
    "doch", "jetzt", "bisher", "erklar", "erklart", "gemeint", "steht", "stelle", "buch", "seine",
    "seinen", "seines", "seiner", "seinem", "ihre", "ihren", "ihrem", "ihrer", "ihres", "meine",
    "meinen", "meines", "meiner", "unser", "wenn", "weil", "denn", "sehr", "dazu", "davon",
    "dabei", "damit", "darauf", "dafur", "dies", "immer", "werden", "wurde", "wurden", "sind",
    "hatten", "kann", "konnte", "muss", "musste", "soll", "sollte", "will", "wollte", "gab",
    "mehr", "viel", "viele", "alle", "alles", "jeden", "jede", "jedes", "hier", "dort", "nach",
    "vor", "durch", "ohne", "sagt", "sagte", "wurde", "etwa", "also", "oder", "ganz", "eigene",
    "eigenen", "eigener", "andere", "anderen", "anderer", // en
    "the", "a", "an", "and", "or", "but", "is", "are", "was", "were", "be", "been", "has", "have",
    "had", "not", "no", "i", "you", "he", "she", "it", "we", "they", "me", "my", "your", "his",
    "her", "its", "of", "with", "at", "from", "on", "in", "to", "for", "by", "as", "how", "what",
    "who", "where", "when", "why", "which", "that", "this", "these", "those", "there", "then",
    "also", "only", "so", "does", "do", "did", "about", "explain", "please", "book",
];

fn stop_set() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| STOP.iter().copied().collect())
}

pub fn is_stop(word: &str) -> bool {
    stop_set().contains(word)
}

/// Wörter für die Absichts- und Befehlserkennung: klein, ä→ae, ö→oe, ü→ue,
/// ß→ss, Apostrophe weg, Satzzeichen trennen (`BookTalkNormalize.words`).
pub fn norm_words(s: &str) -> Vec<String> {
    let mut out = String::new();
    for ch in s.to_lowercase().chars() {
        match ch {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            '\'' | '’' | '‘' => {}
            c if c.is_alphabetic() || c.is_numeric() => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Gefaltete Wörter für Suche und Prüfung: klein, ä→a, ö→o, ü→u, ß→ss
/// (`BookTalkTokens.foldedWords`).
pub fn folded_words(s: &str) -> Vec<String> {
    let mut out = String::new();
    for ch in s.to_lowercase().chars() {
        match ch {
            'ä' => out.push('a'),
            'ö' => out.push('o'),
            'ü' => out.push('u'),
            'ß' => out.push_str("ss"),
            c if c.is_alphabetic() || c.is_numeric() => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn len(s: &str) -> usize {
    s.chars().count()
}

const SUFFIXES: &[&str] = &["ern", "en", "er", "es", "e", "n", "s", "ing", "ed"];

/// Zwei Durchgänge: „Lehrers“ → „Lehrer“ → „Lehr“.
pub fn stem(w: &str) -> String {
    let mut w = w.to_string();
    for _ in 0..2 {
        if len(&w) <= 5 {
            break;
        }
        let Some(suf) = SUFFIXES
            .iter()
            .find(|s| w.ends_with(**s) && len(&w) - len(s) >= 4)
        else {
            break;
        };
        w.truncate(w.len() - suf.len());
    }
    w
}

/// Einfache Tokenisierung: gefaltet, ohne Stoppwörter, leichte Endungskürzung.
pub fn tokenize(text: &str) -> Vec<String> {
    folded_words(text)
        .into_iter()
        .filter_map(|w| {
            if len(&w) < 2 || is_stop(&w) {
                return None;
            }
            let s = stem(&w);
            (len(&s) >= 2).then_some(s)
        })
        .collect()
}

/// Inhaltswörter der Belegprüfung: gefaltet, ab 4 Zeichen, keine Stoppwörter,
/// ohne Stammkürzung.
pub fn content_words(s: &str) -> Vec<String> {
    folded_words(s)
        .into_iter()
        .filter(|w| len(w) >= 4 && !is_stop(w))
        .collect()
}

pub fn content_stems(s: &str) -> HashSet<String> {
    content_words(s).iter().map(|w| stem(w)).collect()
}

/// Ganzes Wort bzw. Wortfolge (gefaltet) in einer gefalteten Wortfolge.
pub fn contains(words: &[String], form: &str) -> bool {
    let f = folded_words(form);
    if f.is_empty() || words.len() < f.len() {
        return false;
    }
    words.windows(f.len()).any(|w| w == f.as_slice())
}

pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Figur, Ort, Ereignis oder Begriff: nur Namen und Varianten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub name: String,
    pub variants: Vec<String>,
    pub kind: EntityKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Person,
    Place,
    Event,
    Term,
}

impl Entity {
    pub fn person(name: &str, variants: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            variants: variants.iter().map(|v| v.to_string()).collect(),
            kind: EntityKind::Person,
        }
    }

    pub fn forms(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.name).chain(self.variants.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_and_stemming() {
        assert_eq!(folded_words("Väter, Größe!"), ["vater", "grosse"]);
        assert_eq!(
            norm_words("Väter, Größe! Don't"),
            ["vaeter", "groesse", "dont"]
        );
        assert_eq!(stem("lehrers"), "lehr");
        assert_eq!(tokenize("Was sagt der Lehrer?"), ["lehr"]);
        assert!(contains(
            &folded_words("Der Meister Anselm sprach"),
            "meister anselm"
        ));
        assert!(!contains(&folded_words("Meister"), "Meister Anselm"));
    }

    #[test]
    fn fnv1a_matches_the_reference_vector() {
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
    }
}
