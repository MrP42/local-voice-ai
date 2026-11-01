//! Quelltext für die Buch-KI (lva#182): Auswahl (ganzes Buch oder Bereich),
//! Säubern und Gliedern in Kapitel.
//!
//! Der Seitentext ist die Quelle. Er enthält Formatzeichen des Readers:
//! - Seitenmarken `{{S. 12}}` (lva#169),
//! - Sprecher am Zeilenanfang `<Anna>`,
//! - Sprechstil-Marken `[calm]`.
//!
//! Diese Zeichen gehen nicht an das Sprachmodell. Die Seitenmarken bleiben
//! aber als Seitenbereich je Kapitel erhalten, damit Ergebnisse auf Seiten
//! verweisen können.
//!
//! Bereiche kommen aus der Oberfläche als UTF-16-Offsets (JavaScript-Strings)
//! und werden hier umgerechnet.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Was zusammengefasst wird.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SummarySource {
    /// Der ganze Seitentext.
    Book,
    /// Ein Ausschnitt, zum Beispiel ein Kapitel oder eine Markierung
    /// (UTF-16-Offsets, Ende exklusiv).
    Range { start: u32, end: u32 },
}

/// Ein Kapitel des gesäuberten Texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Überschrift ohne `#`; leer für Text vor der ersten Überschrift.
    pub title: String,
    pub text: String,
    /// Erste und letzte Seitenmarke im Kapitel, falls vorhanden.
    pub first_page: Option<u32>,
    pub last_page: Option<u32>,
}

impl Section {
    /// Bezeichnung für Prompts: „Kapitel 3: Titel (S. 40–52)“.
    pub fn label(&self, index: usize) -> String {
        let mut out = if self.title.is_empty() {
            format!("Teil {}", index + 1)
        } else {
            self.title.clone()
        };
        match (self.first_page, self.last_page) {
            (Some(a), Some(b)) if a != b => out.push_str(&format!(" (S. {a}–{b})")),
            (Some(a), _) => out.push_str(&format!(" (S. {a})")),
            _ => {}
        }
        out
    }
}

/// Byte-Index zu einem UTF-16-Offset (auf die nächste Zeichengrenze davor).
fn byte_index_for_utf16(text: &str, utf16: u32) -> usize {
    let mut units = 0u32;
    for (byte, c) in text.char_indices() {
        if units >= utf16 {
            return byte;
        }
        units += c.len_utf16() as u32;
    }
    text.len()
}

/// Den gewählten Ausschnitt des Seitentexts.
pub fn select(text: &str, source: &SummarySource) -> Result<String, String> {
    let out = match source {
        SummarySource::Book => text.to_string(),
        SummarySource::Range { start, end } => {
            if end <= start {
                return Err("book_ai:empty_range".into());
            }
            let a = byte_index_for_utf16(text, *start);
            let b = byte_index_for_utf16(text, *end);
            text[a..b].to_string()
        }
    };
    if out.trim().is_empty() {
        return Err("book_ai:empty_source".into());
    }
    Ok(out)
}

/// Seitenmarke `{{S. n}}` am Anfang von `s`: (Seite, Länge in Bytes).
fn page_mark_at(s: &str) -> Option<(u32, usize)> {
    let rest = s.strip_prefix("{{S. ")?;
    let end = rest.find("}}")?;
    let page = rest[..end].trim().parse().ok()?;
    Some((page, "{{S. ".len() + end + 2))
}

/// Sprechstil-Marke `[calm]`: nur kurze Wörter aus Kleinbuchstaben. Fußnoten
/// wie `[1]` bleiben stehen; ein Einschub wie `[sic]` fällt mit weg, was für
/// eine Zusammenfassung unschädlich ist.
fn style_mark_len(s: &str) -> Option<usize> {
    let rest = s.strip_prefix('[')?;
    let end = rest.find(']')?;
    let word = &rest[..end];
    let ok = (1..=24).contains(&word.len())
        && word
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '_' || c == '-' || c == ' ');
    ok.then_some(end + 2)
}

/// Sprecher am Zeilenanfang `<Name>` (ohne Zeilenumbruch, höchstens 40 Zeichen).
fn speaker_len(line: &str) -> Option<usize> {
    let rest = line.strip_prefix('<')?;
    let end = rest.find('>')?;
    let name = &rest[..end];
    let ok = (1..=40).contains(&name.chars().count()) && !name.contains(['<', '\n']);
    ok.then_some(end + 2)
}

/// Eine Zeile säubern; Seitenmarken werden an `pages` gemeldet.
fn clean_line(line: &str, pages: &mut Vec<u32>) -> String {
    let mut body = line;
    if let Some(n) = speaker_len(body.trim_start()) {
        body = body.trim_start()[n..].trim_start();
    }
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        let rest = &body[i..];
        if let Some((page, len)) = page_mark_at(rest) {
            pages.push(page);
            i += len;
            continue;
        }
        if let Some(len) = style_mark_len(rest) {
            i += len;
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    // Doppelte Leerzeichen, die das Entfernen hinterlässt, zusammenziehen.
    let mut squeezed = String::with_capacity(out.len());
    let mut prev_space = false;
    for c in out.chars() {
        let space = c == ' ';
        if !(space && prev_space) {
            squeezed.push(c);
        }
        prev_space = space;
    }
    squeezed.trim_end().to_string()
}

/// Überschrift `#`, `##` oder `###`: (Ebene, Titel).
fn heading(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let level = t.chars().take_while(|c| *c == '#').count();
    if !(1..=3).contains(&level) {
        return None;
    }
    let title = t[level..].strip_prefix(' ')?.trim();
    (!title.is_empty()).then_some((level, title))
}

/// Säubern und in Kapitel gliedern. Kapitelgrenze ist die oberste
/// Überschriftenebene, die mehr als einmal vorkommt. So wird die Titelei
/// (`# Buchtitel`) nicht zum einzigen Kapitel, wenn darunter `##`-Kapitel
/// folgen. Gibt es keine Überschriften, entsteht ein einziges Kapitel.
pub fn sections(text: &str) -> Vec<Section> {
    let mut counts = [0usize; 4];
    for line in text.lines() {
        if let Some((level, _)) = heading(line) {
            counts[level] += 1;
        }
    }
    let split_level = (1..=3)
        .find(|l| counts[*l] > 1)
        .or_else(|| (1..=3).find(|l| counts[*l] == 1));

    let mut out: Vec<Section> = Vec::new();
    let mut current = Section {
        title: String::new(),
        text: String::new(),
        first_page: None,
        last_page: None,
    };
    let mut pages = Vec::new();
    let flush = |current: &mut Section, out: &mut Vec<Section>| {
        let text = current.text.trim().to_string();
        if !text.is_empty() || !current.title.is_empty() {
            out.push(Section {
                title: current.title.clone(),
                text,
                first_page: current.first_page,
                last_page: current.last_page,
            });
        }
    };
    for line in text.lines() {
        pages.clear();
        if let Some((level, title)) = heading(line) {
            let title = clean_line(title, &mut pages);
            if Some(level) == split_level {
                flush(&mut current, &mut out);
                current = Section {
                    title,
                    text: String::new(),
                    first_page: None,
                    last_page: None,
                };
            } else if split_level.is_some_and(|s| level < s)
                && current.title.is_empty()
                && current.text.trim().is_empty()
            {
                // Titelei (`# Buchtitel` über `##`-Kapiteln): benennt den Vorspann.
                current.title = title;
            } else {
                // Unterüberschrift: bleibt als Zeile im Kapitel.
                current.text.push_str(&title);
                current.text.push('\n');
            }
        } else {
            let cleaned = clean_line(line, &mut pages);
            current.text.push_str(&cleaned);
            current.text.push('\n');
        }
        for p in &pages {
            current.first_page.get_or_insert(*p);
            current.last_page = Some(*p);
        }
    }
    flush(&mut current, &mut out);
    // Kapitel ohne Text (z. B. die Titelei) tragen nichts bei.
    out.retain(|s| !s.text.trim().is_empty());
    out
}

/// Anzahl der Wörter (für Zielumfang und Anzeige).
pub fn word_count(text: &str) -> u32 {
    text.split_whitespace().count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_tags_and_speakers_never_reach_the_model() {
        let text = "<Anna> [calm] Der Wind {{S. 3}} wehte.\nNormaler Satz [1] bleibt.";
        let s = sections(text);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].text, "Der Wind wehte.\nNormaler Satz [1] bleibt.");
        assert_eq!(s[0].first_page, Some(3));
    }

    #[test]
    fn chapters_follow_the_repeated_heading_level_not_the_title_page() {
        let text = "# Das Buch\nvon A. Autor\n\n## Eins\n{{S. 1}}Anfang.\n### Unter\nMehr {{S. 2}}.\n## Zwei\n{{S. 3}}Ende {{S. 5}}.";
        let s = sections(text);
        let titles: Vec<_> = s.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Das Buch", "Eins", "Zwei"]);
        assert!(s[1].text.contains("Unter\nMehr ."));
        assert_eq!((s[1].first_page, s[1].last_page), (Some(1), Some(2)));
        assert_eq!(s[2].label(2), "Zwei (S. 3–5)");
        assert_eq!(s[0].label(0), "Das Buch");
    }

    #[test]
    fn text_without_headings_is_one_part() {
        let s = sections("Nur Text.\n\nNoch ein Absatz.");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].label(0), "Teil 1");
    }

    #[test]
    fn ranges_use_utf16_offsets() {
        // „😀“ ist ein Zeichen, aber zwei UTF-16-Einheiten.
        let text = "😀 Äpfel und Birnen";
        let r = select(text, &SummarySource::Range { start: 3, end: 8 }).unwrap();
        assert_eq!(r, "Äpfel");
        assert_eq!(
            select(text, &SummarySource::Range { start: 5, end: 5 }).unwrap_err(),
            "book_ai:empty_range"
        );
        assert_eq!(
            select("   ", &SummarySource::Book).unwrap_err(),
            "book_ai:empty_source"
        );
        // Über das Ende hinaus: bis zum Ende.
        assert_eq!(
            select("abc", &SummarySource::Range { start: 1, end: 99 }).unwrap(),
            "bc"
        );
    }

    #[test]
    fn words_are_counted() {
        assert_eq!(word_count(" eins  zwei\ndrei "), 3);
    }
}
