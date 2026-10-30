//! Zerlegt einen Token-Strom in sprechbare Häppchen.
//!
//! Das Gespräch lebt von der Zeit bis zum ersten Laut. Deshalb wartet die
//! Sprachausgabe nicht auf die ganze Antwort und auch nicht auf einen
//! Satzpunkt, wenn schon ein langer Halbsatz vorliegt: das ERSTE Häppchen
//! darf an einem Komma enden (ab `FIRST_CLAUSE_MIN` Zeichen), alle weiteren
//! enden an Satzzeichen. Kürzel („z. B.“, „Dr.“) und Zahlen („3.5“, „12.“)
//! trennen nicht.

/// Kürzestes erstes Häppchen, das an einem Komma/Semikolon enden darf.
pub const FIRST_CLAUSE_MIN: usize = 24;
/// Ein Häppchen ohne Satzzeichen wird spätestens hier am letzten Leerzeichen
/// getrennt (sonst wartet die Stimme bei Aufzählungen ohne Punkt zu lange).
pub const HARD_LIMIT: usize = 220;

const ABBREVIATIONS: &[&str] = &[
    "z", "b", "d", "h", "u", "a", "o", "v", "bzw", "ca", "evtl", "ggf", "inkl", "max", "min", "nr",
    "usw", "vgl", "dr", "prof", "str", "tel", "ca", "etc", "sog", "St", "Hr", "Fr",
];

#[derive(Debug, Default)]
pub struct SentenceSplitter {
    buf: String,
    emitted: usize,
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Nimmt neuen Text entgegen und gibt alle jetzt vollständigen Häppchen zurück.
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.buf.push_str(delta);
        let mut out = Vec::new();
        while let Some(cut) = self.find_cut() {
            let piece: String = self.buf.drain(..cut).collect();
            let piece = piece.trim().to_string();
            if !piece.is_empty() {
                self.emitted += 1;
                out.push(piece);
            }
        }
        out
    }

    /// Rest am Ende des Stroms.
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        let rest = rest.trim();
        if rest.is_empty() {
            None
        } else {
            self.emitted += 1;
            Some(rest.to_string())
        }
    }

    fn find_cut(&self) -> Option<usize> {
        let chars: Vec<(usize, char)> = self.buf.char_indices().collect();
        for (i, &(byte, c)) in chars.iter().enumerate() {
            let end = byte + c.len_utf8();
            let next = chars.get(i + 1).map(|&(_, n)| n);
            match c {
                '\n' => return Some(end),
                '.' | '!' | '?' | '…' | ':' => {
                    // Ohne folgendes Zeichen wissen wir nicht, ob „3.“ oder „3.5“ kommt.
                    let Some(n) = next else { continue };
                    if !n.is_whitespace() && !matches!(n, '"' | '“' | '”' | '»' | '«' | ')') {
                        continue;
                    }
                    if c == '.' && self.is_abbreviation_or_number(&chars, i) {
                        continue;
                    }
                    if c == ':' && !self.buf[end..].starts_with(char::is_whitespace) {
                        continue;
                    }
                    return Some(end);
                }
                ',' | ';' if self.emitted == 0 => {
                    let Some(n) = next else { continue };
                    if n.is_whitespace() && self.buf[..end].chars().count() >= FIRST_CLAUSE_MIN {
                        return Some(end);
                    }
                }
                _ => {}
            }
        }
        if self.buf.chars().count() >= HARD_LIMIT {
            // Am letzten Leerzeichen vor der Grenze trennen.
            let limit_byte = chars
                .get(HARD_LIMIT)
                .map(|&(b, _)| b)
                .unwrap_or(self.buf.len());
            if let Some(pos) = self.buf[..limit_byte].rfind(' ') {
                if pos > 0 {
                    return Some(pos + 1);
                }
            }
        }
        None
    }

    fn is_abbreviation_or_number(&self, chars: &[(usize, char)], dot: usize) -> bool {
        // Wort direkt vor dem Punkt.
        let mut start = dot;
        while start > 0 && chars[start - 1].1.is_alphanumeric() {
            start -= 1;
        }
        let word: String = chars[start..dot].iter().map(|&(_, c)| c).collect();
        if word.is_empty() {
            return false;
        }
        // Ordnungszahl oder Datum: „12. Mai“, „3.“ am Satzanfang.
        if word.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
        let lower = word.to_lowercase();
        // Einzelbuchstaben („z. B.“, „u. a.“) und bekannte Kürzel.
        (word.chars().count() == 1 && word.chars().all(char::is_alphabetic))
            || ABBREVIATIONS.iter().any(|a| a.to_lowercase() == lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(parts: &[&str]) -> Vec<String> {
        let mut s = SentenceSplitter::new();
        let mut out = Vec::new();
        for p in parts {
            out.extend(s.push(p));
        }
        out.extend(s.finish());
        out
    }

    #[test]
    fn splits_at_sentence_end_across_deltas() {
        assert_eq!(
            feed(&["Hallo", " zusammen.", " Wie geht", " es dir?"]),
            vec!["Hallo zusammen.", "Wie geht es dir?"]
        );
    }

    #[test]
    fn abbreviations_and_numbers_do_not_split() {
        assert_eq!(
            feed(&["Das kostet z. B. 3.5 Euro und ca. 12. Mai fällig. Danke."]),
            vec![
                "Das kostet z. B. 3.5 Euro und ca. 12. Mai fällig.",
                "Danke."
            ]
        );
    }

    #[test]
    fn first_piece_may_end_at_comma_later_ones_may_not() {
        let out = feed(&[
            "Also ich würde vorschlagen, dass wir morgen früh anfangen, wenn das passt. Gut",
        ]);
        assert_eq!(out[0], "Also ich würde vorschlagen,");
        assert_eq!(out[1], "dass wir morgen früh anfangen, wenn das passt.");
        assert_eq!(out[2], "Gut");
    }

    #[test]
    fn short_first_clause_waits_for_more() {
        assert_eq!(feed(&["Ja, genau. Okay."]), vec!["Ja, genau.", "Okay."]);
    }

    #[test]
    fn trailing_dot_without_follow_up_waits_then_finishes() {
        let mut s = SentenceSplitter::new();
        assert!(s.push("Fertig.").is_empty());
        assert_eq!(s.finish().as_deref(), Some("Fertig."));
    }

    #[test]
    fn newline_splits() {
        assert_eq!(feed(&["Eins\nZwei"]), vec!["Eins", "Zwei"]);
    }

    #[test]
    fn hard_limit_splits_at_space() {
        let long = "wort ".repeat(60);
        let out = feed(&[&long]);
        assert!(out.len() >= 2);
        assert!(out.iter().all(|p| p.chars().count() <= HARD_LIMIT));
    }

    #[test]
    fn umlauts_are_safe() {
        assert_eq!(
            feed(&["Schöne Grüße. Über uns."]),
            vec!["Schöne Grüße.", "Über uns."]
        );
    }
}
