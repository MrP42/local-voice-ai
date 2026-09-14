//! Kleine Spracherkennung für die Piper-Stimmenwahl: Stoppwörter zählen,
//! Umlaute als Hinweis. Kein Modell, keine Abhängigkeit — für „Deutsch oder
//! Englisch?" je Satz reicht das, und genau das entscheidet, welche
//! Piper-Stimme spricht. Unentschieden heißt `None`, dann bleibt die
//! eingestellte Stimme.

const DE: &[&str] = &[
    "der", "die", "das", "und", "ist", "nicht", "ein", "eine", "ich", "sie", "er", "es", "wir",
    "mit", "auf", "für", "von", "zu", "den", "dem", "des", "im", "in", "auch", "sich", "wie",
    "aber", "noch", "nur", "war", "wird", "sind", "haben", "hat", "bei", "nach", "aus", "über",
    "als", "wenn", "dann", "oder", "sehr", "schon", "kann", "mich", "dich", "ihr", "ihm",
];
const EN: &[&str] = &[
    "the", "and", "is", "not", "a", "an", "of", "to", "in", "it", "that", "was", "for", "on",
    "are", "with", "as", "his", "her", "they", "at", "be", "this", "have", "from", "or", "one",
    "had", "by", "but", "what", "were", "we", "when", "your", "can", "there", "you", "he",
    "she", "will", "would", "about", "which", "their", "just", "into",
];
const FR: &[&str] = &[
    "le", "la", "les", "et", "est", "un", "une", "des", "du", "que", "qui", "dans", "pour",
    "pas", "sur", "avec", "ce", "il", "elle", "nous", "vous", "sont", "mais", "plus", "au",
];
const ES: &[&str] = &[
    "el", "la", "los", "las", "y", "es", "un", "una", "que", "de", "del", "en", "por", "para",
    "con", "no", "se", "su", "al", "como", "más", "pero", "son", "está", "muy",
];
const IT: &[&str] = &[
    "il", "lo", "la", "gli", "le", "e", "è", "un", "una", "che", "di", "del", "della", "per",
    "con", "non", "si", "sono", "come", "più", "ma", "anche", "nel", "alla", "sono",
];

const LANGS: &[(&str, &[&str])] = &[("de", DE), ("en", EN), ("fr", FR), ("es", ES), ("it", IT)];

/// Sprache eines Texts als ISO-639-1-Kürzel, oder `None`, wenn zu wenig
/// Anhaltspunkte da sind (Zahlen, ein einzelnes Wort, Fachjargon).
pub fn detect_language(text: &str) -> Option<&'static str> {
    let mut scores = [0i32; 5];
    let mut words = 0;
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '\'' && c != 'ß') {
        if raw.is_empty() {
            continue;
        }
        let w = raw.to_lowercase();
        words += 1;
        for (i, (_, list)) in LANGS.iter().enumerate() {
            if list.contains(&w.as_str()) {
                scores[i] += 2;
            }
        }
    }
    // Umlaute und ß sind ein starkes Deutsch-Signal, Akzente ein Romania-Signal.
    for c in text.chars() {
        match c {
            'ä' | 'ö' | 'ü' | 'Ä' | 'Ö' | 'Ü' | 'ß' => scores[0] += 3,
            'é' | 'è' | 'ê' | 'ç' | 'à' | 'ù' => scores[2] += 1,
            'ñ' | '¿' | '¡' => scores[3] += 3,
            _ => {}
        }
    }
    if words == 0 {
        return None;
    }
    let (best, best_score) = scores
        .iter()
        .enumerate()
        .max_by_key(|(_, s)| **s)
        .map(|(i, s)| (i, *s))?;
    let second = scores.iter().enumerate().filter(|(i, _)| *i != best).map(|(_, s)| *s).max().unwrap_or(0);
    // Mindestens zwei Treffer und ein klarer Vorsprung — sonst raten wir nicht.
    if best_score < 4 || best_score < second + 2 {
        return None;
    }
    Some(LANGS[best].0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deutsch_und_englisch_werden_je_satz_erkannt() {
        assert_eq!(detect_language("Die drei Feen und die Wolke voller Wünsche."), Some("de"));
        assert_eq!(detect_language("It was a quiet evening in Luminara."), Some("en"));
        assert_eq!(detect_language("The sky was clear, the flowers had closed their blossoms."), Some("en"));
        assert_eq!(detect_language("Es war ein ruhiger Abend, und die Blumen hatten sich geschlossen."), Some("de"));
    }

    #[test]
    fn andere_sprachen_und_unklares() {
        assert_eq!(detect_language("Le ciel est clair et les fleurs sont fermées."), Some("fr"));
        assert_eq!(detect_language("El cielo está claro y las flores son azules."), Some("es"));
        assert_eq!(detect_language("Luminara"), None);
        assert_eq!(detect_language("42 17 3"), None);
        assert_eq!(detect_language(""), None);
    }
}
