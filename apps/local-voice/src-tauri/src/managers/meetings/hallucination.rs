//! M2 / P2a: Textfilter gegen Stille-Halluzinationen (Spec m2-audio-stt.md,
//! 3.4, Stufe 3). Whisper erfindet auf Stille und Rauschen Saetze ("Thank
//! you.", "Untertitel der Amara.org-Community"); Parakeet tut das nicht.
//!
//! Stufe 1 (nur VAD-Segmente gehen an die Engine, digitale Nullen erzeugen
//! keinen Aufruf) steht in `segmenter.rs`, Stufe 2 (Engine-Parameter) im
//! Befund des Pakets. Dies hier ist das letzte Netz: rein, ohne I/O, gibt nur
//! den Grund zurueck. Der Aufrufer zaehlt; der Text selbst wird nie geloggt.
//!
//! Bekannte Grenze: eine echte, allein stehende Aeusserung "Thank you." wird
//! verworfen. Die Audiodatei bleibt roh erhalten, der Enddurchlauf (P2d) sieht
//! sie wieder.

/// Mehr als so viele Zeichen je Sekunde Sprache sind kein Sprechtempo.
pub const MAX_CHARS_PER_SECOND: f32 = 25.0;
/// Anteil wiederholter Woerter, ab dem ein Text als Schleife gilt.
pub const MAX_REPETITION_RATIO: f32 = 0.5;
/// Text bei weniger VAD-Sprachanteil ist erfunden.
pub const MIN_SPEECH_SHARE: f32 = 0.20;
/// Unter dieser Wortzahl ist "Wiederholung" nicht aussagekraeftig ("ja ja ja").
const MIN_TOKENS_FOR_REPETITION: usize = 6;
/// Kurze Segmente rechnen mit mindestens einer Sekunde (sonst ist "Ja, genau."
/// in 0,4 s schon zu dicht).
const DENSITY_FLOOR_MS: u64 = 1_000;

/// Ganze Texte (normalisiert), die nie eine echte Aeusserung sind.
const KNOWN_PHRASES: &[&str] = &[
    "thank you",
    "thank you very much",
    "thanks for watching",
    "thank you for watching",
    "thanks for watching and see you next time",
    "you",
    "blank audio",
    "vielen dank fürs zuschauen",
    "danke fürs zuschauen",
    "bis zum nächsten mal",
];

/// Anfaenge (normalisiert, an Wortgrenze), die Untertitel-Abspann sind.
const KNOWN_PREFIXES: &[&str] = &[
    "untertitel",
    "untertitelung",
    "subtitles by",
    "subtitled by",
    "amara org",
    "sous titres",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    KnownPhrase,
    Repetition,
    TooDense,
    LowSpeechShare,
}

/// Was der Segmentierer ueber den Block weiss. `None` = unbekannt (Rueckfall-
/// Chunker ohne VAD): dann greifen nur die Regeln, die ohne VAD gehen.
#[derive(Clone, Copy, Debug)]
pub struct BlockFacts {
    pub audio_ms: u64,
    pub speech_ms: Option<u64>,
    pub span_ms: Option<u64>,
}

/// Kleinschreibung, Satzzeichen und Apostrophe weg ("fuer's" -> "fuers"),
/// Leerraum zusammengezogen.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for ch in text.chars().flat_map(char::to_lowercase) {
        if matches!(ch, '\'' | '’' | '`' | '´') {
            continue;
        }
        if ch.is_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Anteil der Woerter, die nur Wiederholung sind: 1 - verschiedene/alle.
/// 0.0 bei weniger als 6 Woertern.
pub fn repetition_ratio(text: &str) -> f32 {
    let norm = normalize(text);
    let tokens: Vec<&str> = norm.split(' ').filter(|t| !t.is_empty()).collect();
    if tokens.len() < MIN_TOKENS_FOR_REPETITION {
        return 0.0;
    }
    let mut distinct = tokens.clone();
    distinct.sort_unstable();
    distinct.dedup();
    1.0 - distinct.len() as f32 / tokens.len() as f32
}

/// Regeln, die nur den Text brauchen: Floskeln und Wiederholungsschleifen.
pub fn check_text(text: &str) -> Option<Reason> {
    let norm = normalize(text);
    if norm.is_empty() {
        return None;
    }
    if KNOWN_PHRASES.contains(&norm.as_str()) {
        return Some(Reason::KnownPhrase);
    }
    let prefixed = KNOWN_PREFIXES
        .iter()
        .any(|p| norm == *p || (norm.starts_with(p) && norm[p.len()..].starts_with(' ')));
    if prefixed {
        return Some(Reason::KnownPhrase);
    }
    if repetition_ratio(text) > MAX_REPETITION_RATIO {
        return Some(Reason::Repetition);
    }
    None
}

/// Regeln, die Text und Audio zusammen brauchen: Zeichendichte und
/// Sprachanteil. `text` ist der ganze Block (alle Teilsegmente zusammen).
pub fn check_block(text: &str, facts: &BlockFacts) -> Option<Reason> {
    let chars = text.trim().chars().count();
    if chars == 0 {
        return None;
    }
    let basis_ms = facts
        .span_ms
        .unwrap_or(facts.audio_ms)
        .max(DENSITY_FLOOR_MS);
    let per_second = chars as f32 / (basis_ms as f32 / 1_000.0);
    if per_second > MAX_CHARS_PER_SECOND {
        return Some(Reason::TooDense);
    }
    if let Some(speech_ms) = facts.speech_ms {
        if facts.audio_ms > 0 && (speech_ms as f32 / facts.audio_ms as f32) < MIN_SPEECH_SHARE {
            return Some(Reason::LowSpeechShare);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_english_filler_is_dropped() {
        for text in [
            "Thank you.",
            "thank you",
            "Thanks for watching!",
            "you",
            "You.",
        ] {
            assert_eq!(check_text(text), Some(Reason::KnownPhrase), "{text}");
        }
    }

    #[test]
    fn known_german_subtitle_credits_are_dropped() {
        for text in [
            "Untertitel der Amara.org-Community",
            "Untertitel im Auftrag des ZDF, 2020",
            "Untertitelung des ZDF für funk, 2017",
            "Vielen Dank fürs Zuschauen.",
            "Vielen Dank für's Zuschauen!",
            "Bis zum nächsten Mal.",
        ] {
            assert_eq!(check_text(text), Some(Reason::KnownPhrase), "{text}");
        }
    }

    #[test]
    fn real_sentences_that_merely_contain_a_filler_word_survive() {
        for text in [
            "Thank you for the update on the budget.",
            "Danke, dass du das übernimmst.",
            "Ich schaue mir die Untertitel später an.",
            "Vielen Dank für Ihre Aufmerksamkeit.",
            "Wenn you das so sagst, passt es.",
        ] {
            assert_eq!(check_text(text), None, "{text}");
        }
    }

    #[test]
    fn a_looping_phrase_is_a_repetition() {
        let looped = "Ich habe gesagt, dass ich gehe. ".repeat(6);
        assert_eq!(check_text(&looped), Some(Reason::Repetition));
        assert!(repetition_ratio(&looped) > 0.5);
    }

    #[test]
    fn normal_speech_has_a_low_repetition_ratio() {
        let text =
            "Wir halten fest: das Budget bleibt unverändert, die Abnahme erfolgt im September.";
        assert!(repetition_ratio(text) < 0.2);
        assert_eq!(check_text(text), None);
    }

    #[test]
    fn short_repeated_answers_are_not_a_repetition() {
        // Unter 6 Woertern ist "ja ja ja" keine Schleife.
        assert_eq!(check_text("Ja, ja, ja."), None);
        assert_eq!(repetition_ratio("Ja ja ja"), 0.0);
    }

    #[test]
    fn text_too_dense_for_the_speech_time_is_dropped() {
        let text = "x".repeat(200);
        let facts = BlockFacts {
            audio_ms: 3_000,
            speech_ms: Some(2_000),
            span_ms: Some(2_000),
        };
        assert_eq!(check_block(&text, &facts), Some(Reason::TooDense));
    }

    #[test]
    fn a_fast_but_human_speaker_passes() {
        // 20 Zeichen je Sekunde ueber 10 s.
        let text = "a".repeat(200);
        let facts = BlockFacts {
            audio_ms: 11_000,
            speech_ms: Some(9_000),
            span_ms: Some(10_000),
        };
        assert_eq!(check_block(&text, &facts), None);
    }

    #[test]
    fn short_segments_count_at_least_one_second() {
        // "Ja, genau." (10 Zeichen) in 0,4 s Sprache ist nicht "zu dicht".
        let facts = BlockFacts {
            audio_ms: 1_000,
            speech_ms: Some(420),
            span_ms: Some(420),
        };
        assert_eq!(check_block("Ja, genau.", &facts), None);
    }

    #[test]
    fn text_over_almost_no_speech_is_dropped() {
        let facts = BlockFacts {
            audio_ms: 10_000,
            speech_ms: Some(1_500),
            span_ms: Some(9_000),
        };
        assert_eq!(
            check_block("Das ist ein erfundener Satz", &facts),
            Some(Reason::LowSpeechShare)
        );
    }

    #[test]
    fn without_vad_facts_only_density_can_fire() {
        // Rueckfall-Chunker: 20 s Audio, kein Sprachanteil bekannt.
        let facts = BlockFacts {
            audio_ms: 20_000,
            speech_ms: None,
            span_ms: None,
        };
        assert_eq!(check_block("Das ist ein normaler Satz.", &facts), None);
        assert_eq!(
            check_block(&"wort ".repeat(200), &facts),
            Some(Reason::TooDense)
        );
    }

    #[test]
    fn normalize_strips_punctuation_and_apostrophes() {
        assert_eq!(
            normalize("  Vielen Dank für's  Zuschauen!! "),
            "vielen dank fürs zuschauen"
        );
        assert_eq!(normalize("…"), "");
    }
}
