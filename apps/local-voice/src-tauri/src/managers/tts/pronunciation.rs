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

use super::aussprache::{Aufbau, Schutz};

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
    "version",
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

/// Datumsangaben und Lexikon auf ein Stueck ohne Tags anwenden (ohne Schutzbereiche; die Aufbereitung
/// der Sprachausgabe geht über [`apply_geschuetzt`]).
#[cfg(test)]
pub fn apply(segment: &str) -> String {
    apply_geschuetzt(segment, &Schutz::leer()).0
}

/// Wie [`apply`], aber mit Schutzbereichen: eingesetzte Aussprachen eines Assistenten
/// ([`Schutz`]) bleiben, wie sie sind. Datum und Lexikon sehen den Text daneben unveraendert (ein
/// Datum nach einem geschuetzten Wort ist ein Datum), greifen aber nie in ein geschuetztes Stueck:
/// ein Treffer, der eines beruehrt, gilt nicht. Die Bereiche des Ergebnisses stehen auf dem neuen
/// Text.
pub fn apply_geschuetzt(segment: &str, schutz: &Schutz) -> (String, Schutz) {
    let (dated, schutz) = dates(segment, schutz);
    lexicon()
        .iter()
        .fold((dated, schutz), |(text, schutz), (re, replacement)| {
            let mut aus = Aufbau::neu(&text, &schutz);
            let mut last = 0usize;
            for caps in re.captures_iter(&text) {
                let whole = caps.get(0).expect("group 0");
                if schutz.beruehrt(&(whole.start()..whole.end())) {
                    continue;
                }
                aus.behalte(last..whole.start());
                let mut ersatz = String::new();
                caps.expand(replacement, &mut ersatz);
                aus.setze(&ersatz);
                last = whole.end();
            }
            aus.behalte(last..text.len());
            aus.fertig()
        })
}

fn dates(text: &str, schutz: &Schutz) -> (String, Schutz) {
    let mut out = Aufbau::neu(text, schutz);
    let mut last = 0usize;
    for caps in date_re().captures_iter(text) {
        let whole = caps.get(0).expect("group 0");
        if schutz.beruehrt(&(whole.start()..whole.end())) {
            continue;
        }
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
        out.behalte(last..whole.start());
        out.setze(&format!("{day}. {}", MONTH_NAMES[month - 1]));
        match year {
            Some(y) => {
                out.setze(" ");
                out.setze(y);
            }
            None => {
                // Der Schlusspunkt von „31.5.“ beendet nur am Zeilenende einen Satz.
                let rest = &text[whole.end()..];
                let next = rest.trim_start_matches(|c: char| c != '\n' && c.is_whitespace());
                if next.is_empty() || next.starts_with('\n') {
                    out.setze(".");
                }
            }
        }
        last = whole.end();
    }
    out.behalte(last..text.len());
    out.fertig()
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

fn spoken_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[0-9]+(?:\.[0-9]+)+[.)]?|[\p{L}\p{N}_]+[.)]?")
            .expect("spoken token regex")
    })
}

/// Nach Datum, Lexikon und Abkuerzungstabelle: nur Engine-Text, nie geschuetzte Zeichen.
pub(super) fn numbers_and_initialisms(text: &str, schutz: &Schutz) -> (String, Schutz) {
    let mut out = Aufbau::neu(text, schutz);
    let mut last = 0;
    for token in spoken_token_re().find_iter(text) {
        if token.start() < last {
            continue;
        }
        let before = &text[..token.start()];
        let after = &text[token.end()..];
        // Keine Teiltreffer in Bezeichnern, Dateinamen oder Punktketten.
        if before.ends_with(|c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | '/'))
            || after.starts_with(|c: char| c.is_alphanumeric() || c == '_')
        {
            continue;
        }
        let bare = token.as_str().trim_end_matches(['.', ')']);
        let replacement = if bare.bytes().all(|c| c.is_ascii_digit() || c == b'.') {
            spoken_number(token.as_str(), before, after)
        } else {
            spell_initialism(bare).map(|mut word| {
                word.push_str(&token.as_str()[bare.len()..]);
                (word, 0)
            })
        };
        let Some((word, extra)) = replacement else {
            continue;
        };
        let end = token.end() + extra;
        if schutz.beruehrt(&(token.start()..end)) {
            continue;
        }
        out.behalte(last..token.start());
        out.setze(&word);
        last = end;
    }
    out.behalte(last..text.len());
    out.fertig()
}

fn spoken_number(token: &str, before: &str, after: &str) -> Option<(String, usize)> {
    let bare = token.trim_end_matches(['.', ')']);
    let groups: Vec<_> = bare.split('.').collect();
    let next = after.trim_start_matches(|c: char| c.is_whitespace() && c != '\n' && c != '\r');
    let next_word: String = next.chars().take_while(|c| c.is_alphabetic()).collect();
    let list = super::protocol::is_list_start(before);
    if groups.len() > 1 {
        // Uhrzeit vor Gliederung und Tausendern; das ganze „Uhr“ gehoert zum Treffer.
        if groups.len() == 2
            && !token.ends_with(['.', ')'])
            && next_word == "Uhr"
            && groups[0].len() <= 2
            && groups[1].len() == 2
            && groups[0].parse::<u8>().is_ok_and(|n| n < 24)
            && groups[1].parse::<u8>().is_ok_and(|n| n < 60)
        {
            return Some((
                format!("{} Uhr {}", groups[0], groups[1]),
                after.len() - next.len() + 3,
            ));
        }
        let thousands = (1..=3).contains(&groups[0].len())
            && !groups[0].starts_with('0')
            && groups[1..].iter().all(|g| g.len() == 3);
        let mut word = if thousands {
            groups.concat()
        } else {
            groups.join(" Punkt ")
        };
        // Bei Tausendern gehoert ein nachfolgender Punkt weiter zum Satz.
        if thousands && token.ends_with('.') {
            word.push('.');
        }
        if token.ends_with(')') {
            word.push(')');
        }
        return Some((word, 0));
    }
    let n = bare.parse::<usize>().ok()?;
    // Auch ein Datum am Zeilenanfang ist kein Listenpunkt.
    if super::protocol::is_month(&next_word) {
        return None;
    }
    if list
        && (token.ends_with('.') || token.ends_with(')'))
        && !next_word.is_empty()
        && bare.len() <= 3
        && n > 0
    {
        const LIST: [&str; 10] = [
            "Erstens", "Zweitens", "Drittens", "Viertens", "Fünftens",
            "Sechstens", "Siebtens", "Achtens", "Neuntens", "Zehntens",
        ];
        let word = LIST.get(n - 1).map_or_else(|| format!("Punkt {n}"), |s| s.to_string());
        return Some((format!("{word}:"), 0));
    }
    // Datumsaufbereitung behaelt ihre bisherige Tages-/Monatsdarstellung.
    if !token.ends_with('.')
        || !(1..=99).contains(&n)
        || bare.len() > 2
        || !next_word.starts_with(char::is_uppercase)
    {
        return None;
    }
    // Das Substantiv allein bestimmt die Endung nicht („der 4. Platz“, „dem 4. Platz“).
    let suffix = super::protocol::ordinal_suffix(before)?;
    Some((format!("{}{suffix}", ordinal_stem(n)), 0))
}

fn ordinal_stem(n: usize) -> String {
    const SMALL: [&str; 20] = ["", "erst", "zweit", "dritt", "viert", "fünft", "sechst", "siebt",
        "acht", "neunt", "zehnt", "elft", "zwölft", "dreizehnt", "vierzehnt", "fünfzehnt",
        "sechzehnt", "siebzehnt", "achtzehnt", "neunzehnt"];
    if n < 20 {
        return SMALL[n].to_string();
    }
    const ONES: [&str; 10] = ["", "ein", "zwei", "drei", "vier", "fünf", "sechs", "sieben", "acht", "neun"];
    const TENS: [&str; 10] = ["", "", "zwanzig", "dreißig", "vierzig", "fünfzig", "sechzig", "siebzig", "achtzig", "neunzig"];
    let head = if n % 10 == 0 {
        String::new()
    } else {
        format!("{}und", ONES[n % 10])
    };
    format!("{head}{}st", TENS[n / 10])
}

fn spell_initialism(token: &str) -> Option<String> {
    // Wort-Akronyme vor der allgemeinen Buchstabierung. Das Lexikon lief bereits vorher.
    const WORDS: &[&str] = &["NATO", "UNESCO", "DAX", "ADAC", "PIN", "TÜV", "BAföG"];
    let (base, plural) = token.strip_suffix('s').map_or((token, false), |s| (s, true));
    if WORDS.contains(&base)
        || !(2..=5).contains(&base.chars().count())
        || !base.chars().any(char::is_uppercase)
        || !base.chars().all(|c| c.is_uppercase() || c.is_ascii_digit())
    {
        return None;
    }
    let mut letters = Vec::new();
    for c in base.chars() {
        letters.push(match c {
            'A' => "A", 'B' => "Be", 'C' => "Ce", 'D' => "De", 'E' => "E", 'F' => "Ef",
            'G' => "Ge", 'H' => "Ha", 'I' => "I", 'J' => "Jot", 'K' => "Ka", 'L' => "El",
            'M' => "Em", 'N' => "En", 'O' => "O", 'P' => "Pe", 'Q' => "Ku", 'R' => "Er",
            'S' => "Es", 'T' => "Te", 'U' => "U", 'V' => "Fau", 'W' => "We", 'X' => "Ix",
            'Y' => "Ypsilon", 'Z' => "Zet", 'Ä' => "Ä", 'Ö' => "Ö", 'Ü' => "Ü",
            '0' => "0", '1' => "1", '2' => "2", '3' => "3", '4' => "4", '5' => "5",
            '6' => "6", '7' => "7", '8' => "8", '9' => "9",
            _ => return None,
        });
    }
    let mut word = letters.join("-");
    if plural {
        word.push('s');
    }
    Some(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordnungszahlen_ohne_erkannten_kontext_bleiben() {
        // Gegenprobe: der Substantiv-Fallback macht aus „4. Platz“ faelschlich „vierter Platz“.
        for text in [
            "Der 3. oder 4. Platz.",
            "Auf 3. Platz folgt der Rest.",
            "Mit 2. Versuch klappt es.",
            "In 4. Runde endet es.",
            "In 3. Klasse beginnt es.",
            "An 2. Tag regnet es.",
        ] {
            assert_eq!(numbers_and_initialisms(text, &Schutz::leer()).0, text, "{text}");
        }
        for (text, expected) in [
            ("Der 4. Platz.", "Der vierte Platz."),
            ("Auf dem 3. Platz.", "Auf dem dritten Platz."),
            ("Ein 2. Versuch.", "Ein zweiter Versuch."),
        ] {
            assert_eq!(numbers_and_initialisms(text, &Schutz::leer()).0, expected, "{text}");
        }
    }

    #[test]
    fn zahlen_regeln_942() {
        for (input, expected) in [
            ("Version 2.5. ist da.", "Version 2 Punkt 5 ist da."),
            ("(3.2)", "(3 Punkt 2)"),
            ("1.000. Danach", "1000. Danach"),
            ("Wir sehen 1.2 und 1.234.", "Wir sehen 1 Punkt 2 und 1234."),
            ("Wir kommen um 00.05 Uhr.", "Wir kommen um 00 Uhr 05."),
            ("Wir sehen 10.30 Werte.", "Wir sehen 10 Punkt 30 Werte."),
            ("Die GE, IT. EU! KI? A4 und 2FA.", "Die Ge-E, I-Te. E-U! Ka-I? A-4 und 2-Ef-A."),
            ("NATOs und PINs bleiben.", "NATOs und PINs bleiben."),
            ("Die abcGE und A12345 bleiben.", "Die abcGE und A12345 bleiben."),
            ("Datei GE.pdf und abc1.2 bleiben.", "Datei GE.pdf und abc1.2 bleiben."),
            ("Im Jahr 2024. Danach", "Im Jahr 2024. Danach"),
            ("am 3.\nDanach", "am 3.\nDanach"),
        ] {
            assert_eq!(numbers_and_initialisms(input, &Schutz::leer()).0, expected, "{input}");
        }
        // Gegenprobe fuer die Datumsvorrang-Regel: ohne „version“ wuerde daraus „2. Mai“.
        assert_eq!(apply("Version 2.5. ist da."), "Version 2.5. ist da.");
        assert_eq!(apply("Am 2.5. ist der Termin."), "Am 2. Mai ist der Termin.");
    }

    #[test]
    fn ordinalstaemme_und_endungen_942() {
        for (n, stem) in [
            (1, "erst"), (2, "zweit"), (3, "dritt"), (6, "sechst"),
            (7, "siebt"), (8, "acht"), (11, "elft"), (12, "zwölft"),
            (16, "sechzehnt"), (17, "siebzehnt"), (19, "neunzehnt"),
            (20, "zwanzigst"), (21, "einundzwanzigst"), (30, "dreißigst"),
            (60, "sechzigst"), (70, "siebzigst"), (99, "neunundneunzigst"),
        ] {
            assert_eq!(ordinal_stem(n), stem);
            for (before, suffix) in [("der ", "e"), ("am ", "en"), ("ein ", "er")] {
                let input = format!("{before}{n}. Versuch");
                assert_eq!(numbers_and_initialisms(&input, &Schutz::leer()).0,
                    format!("{before}{stem}{suffix} Versuch"));
            }
        }
    }

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
