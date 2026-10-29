//! Aussprache fuer die SYNTHESE (deutsche Texte): Datumsangaben und ein kleines Lexikon.
//!
//! Wie [`super::abbreviations`] gilt das nur fuer den Text, den die Engine bekommt;
//! Anzeige, Satzgrenzen und Satzzaehlung bleiben beim Original.
//!
//! * **Datum**: „31.5.“ oder „31.05.2026“ liest ein Sprachmodell mit einer
//!   Sprechpause nach dem ersten Punkt, mitten in der Angabe. „31. Mai“ hat nur noch
//!   den Ordnungszahl-Punkt, den die Engine als „einunddreissigsten“ liest.
//!   Nummerierungen wie „3.2.“ (Kapitel, Punkt, TOP) bleiben unberuehrt.
//! * **Lexikon**: Woerter, die die Engine falsch betont oder ausspricht. „Beurlaubung“
//!   liest sie mit „eu“ wie in „Beute“; ein Bindestrich nach „Be“ trennt die Silben.

use std::sync::OnceLock;

use regex::Regex;

const MONTH_NAMES: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];

/// Woerter vor einer Nummer, die „3.2.“ als Gliederungspunkt statt als Datum ausweisen.
const NUMBERING_WORDS: &[&str] = &[
    "punkt",
    "top",
    "kapitel",
    "abschnitt",
    "ziffer",
    "nr",
    "nummer",
    "siehe",
    "vgl",
    "tz",
    "randnummer",
    "anlage",
    "teil",
    "gemäß",
    "gem",
];

/// (Muster, Ersatz): gilt ueberall im Wort, die Schreibweise bleibt erhalten.
const LEXICON: &[(&str, &str)] = &[("(?i)(be)(urlaub)", "$1-$2")];

fn date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\d{1,2})\.(\d{1,2})\.(\d{4}|\d{2})?").expect("date regex"))
}

fn lexicon() -> &'static [(Regex, &'static str)] {
    static TABLE: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        LEXICON
            .iter()
            .map(|(pattern, replacement)| {
                (Regex::new(pattern).expect("lexicon regex"), *replacement)
            })
            .collect()
    })
}

/// Datumsangaben und Lexikon auf ein Stueck ohne Tags anwenden.
pub fn apply(segment: &str) -> String {
    let dated = dates(segment);
    lexicon().iter().fold(dated, |text, (re, replacement)| {
        re.replace_all(&text, *replacement).into_owned()
    })
}

fn dates(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut last = 0usize;
    for caps in date_re().captures_iter(text) {
        let whole = caps.get(0).expect("group 0");
        let before = text[..whole.start()].chars().next_back();
        let after = text[whole.end()..].chars().next();
        let dotted_chain = text[whole.end()..]
            .strip_prefix('.')
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
        if before.is_some_and(|c| c.is_ascii_digit() || c == '.')
            || after.is_some_and(|c| c.is_ascii_digit())
            || dotted_chain
        {
            continue;
        }
        let (Ok(day), Ok(month)) = (caps[1].parse::<usize>(), caps[2].parse::<usize>()) else {
            continue;
        };
        if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
            continue;
        }
        let year = caps.get(3).map(|m| m.as_str());
        if year.is_none() && is_numbering(text, whole.start()) {
            continue;
        }
        out.push_str(&text[last..whole.start()]);
        out.push_str(&format!("{day}. {}", MONTH_NAMES[month - 1]));
        match year {
            Some(y) => {
                out.push(' ');
                out.push_str(y);
            }
            None => {
                // Der Schlusspunkt von „31.5.“ beendet nur am Zeilenende einen Satz.
                let rest = &text[whole.end()..];
                let next = rest.trim_start_matches(|c: char| c != '\n' && c.is_whitespace());
                if next.is_empty() || next.starts_with('\n') {
                    out.push('.');
                }
            }
        }
        last = whole.end();
    }
    out.push_str(&text[last..]);
    out
}

/// Steht die Nummer am Zeilenanfang (Ueberschrift, Listenpunkt) oder nach einem
/// Gliederungswort („Punkt 3.2.“)?
fn is_numbering(text: &str, start: usize) -> bool {
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let head = &text[line_start..start];
    if head
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '#' | '-' | '*' | '>'))
        .is_empty()
    {
        return true;
    }
    let word: String = head
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_alphabetic() || *c == '§')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let lower = word.to_lowercase();
    lower == "§" || NUMBERING_WORDS.contains(&lower.as_str())
}

#[cfg(test)]
mod tests {
    use super::apply;

    #[test]
    fn datum_ohne_jahr_wird_zu_tag_und_monat() {
        assert_eq!(
            apply("Bis zum 31.5. ist Zeit."),
            "Bis zum 31. Mai ist Zeit."
        );
        assert_eq!(
            apply("Am 1.12., spaetestens."),
            "Am 1. Dezember, spaetestens."
        );
    }

    #[test]
    fn datum_mit_jahr() {
        assert_eq!(apply("Stand 31.05.2026 gilt."), "Stand 31. Mai 2026 gilt.");
        assert_eq!(apply("Stand 3.7.26."), "Stand 3. Juli 26.");
    }

    #[test]
    fn schlusspunkt_nur_am_zeilenende() {
        assert_eq!(apply("Frist ist der 31.5."), "Frist ist der 31. Mai.");
        assert_eq!(apply("Frist 31.5.\nWeiter"), "Frist 31. Mai.\nWeiter");
    }

    #[test]
    fn nummerierungen_und_versionen_bleiben() {
        for text in [
            "3.2. Ergebnisse",
            "## 3.2. Ergebnisse",
            "Siehe Punkt 3.2. dazu.",
            "TOP 4.1. Haushalt",
            "Version 1.2.3 ist da.",
            "Adresse 10.10.10.10 offen.",
            "Wert 31.13. unklar.",
            "Tag 32.5. unklar.",
        ] {
            assert_eq!(apply(text), text, "{text}");
        }
    }

    #[test]
    fn beurlaubung_wird_silbenweise_getrennt() {
        assert_eq!(apply("Die Beurlaubung läuft."), "Die Be-urlaubung läuft.");
        assert_eq!(apply("beurlaubt"), "be-urlaubt");
        assert_eq!(apply("Sonderbeurlaubungen"), "Sonderbe-urlaubungen");
        assert_eq!(apply("Urlaub und Erlaubnis"), "Urlaub und Erlaubnis");
    }
}
