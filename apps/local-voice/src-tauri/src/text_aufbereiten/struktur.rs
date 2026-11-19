//! Struktur von Buchtext erkennen: Zeilen zu Absaetzen, Lauftitel und Seitenzahlen,
//! Silbentrennung, Seitenmarken, Kapitel (lva#658 A1, aus `ebook.rs` herausgeloest).
//!
//! Reine Funktionen ohne Tauri. Verhalten unveraendert: `ebook.rs` ruft diesen Baustein.

use std::sync::LazyLock;

/// Einmal kompilierte Muster; ein grosses Buch ruft sie zehntausendfach auf.
macro_rules! re {
    ($name:ident, $pat:expr) => {
        pub(crate) static $name: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new($pat).expect("static regex"));
    };
}
re!(LIST_LINE_RE, r"^(?:[•\-–*]\s|\d{1,3}[.)]\s)");
// Zahl mit Punkt, danach ein kleingeschriebenes Wort: Datum im Satz, keine Liste.
re!(NUMBER_THEN_LOWER_RE, r"^\d{1,4}\.\s+\p{Ll}");
re!(
    SPEAKER_LINE_RE,
    r"^(?:<[^<>]{1,40}>|[A-ZÄÖÜ][\w .'-]{0,30}:\s)"
);
// Kapitelanfang im PDF: „Kapitel 3“, „Chapter IV“, „Teil 2“, „Part II“ allein auf der Zeile …
re!(
    CHAPTER_HEAD_RE,
    r"(?i)^(?:(?:kapitel|chapter|teil|part|buch|book)\s+(?:\d{1,3}|(?-i:[IVXLCDM]{1,8})|(?:one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty)(?:[- ](?:one|two|three|four|five|six|seven|eight|nine))?)|\p{L}{3,}(?:tes|ter|te)\s+(?:kapitel|buch|teil|abschnitt|gesang))\.?$"
);
// … oder mit Titel in derselben Zeile: „Kapitel 3: Die Klippe“.
re!(
    CHAPTER_HEAD_TITLE_RE,
    r"(?i)^((?:kapitel|chapter|teil|part)\s+(?:\d{1,3}|(?-i:[IVXLCDM]{1,8})))\s*[:\u{2013}\u{2014}\u{B7}-]\s*(\S.{0,50})$"
);
// … oder ein bekannter Abschnittsname.
re!(
    SPECIAL_HEAD_RE,
    r"(?i)^(?:prolog|epilog|vorwort|nachwort|einleitung|prologue|epilogue|preface|foreword|afterword|introduction)$"
);
// Nummer eines Verzeichniseintrags: „KHM 12.“, „12.“, „Kapitel 3:“, „IV.“
re!(
    TITLE_ID_RE,
    r"^(?:\p{L}{1,8}\s+)?(?:\d{1,4}[a-z]?|[IVXLC]{1,6})[.:)]\s"
);

/// PDF-Text: Druckzeilen zu Absätzen zusammenführen.
///
/// - Leerzeilen trennen Absätze.
/// - Eine Zeile, die mit Satzzeichen endet und deutlich kürzer ist als die
///   üblichen Zeilen, beendet den Absatz; ebenso eine sehr kurze Zeile ohne
///   Satzzeichen vor einer groß beginnenden Zeile (Überschrift).
/// - Aufzählungen, `Name:`- und `<Name>`-Zeilen beginnen einen neuen Absatz.
/// - Trennstriche am Zeilenende vor Kleinbuchstaben werden aufgehoben, außer
///   vor „und/oder“ („Vor- und Nachteile“); vor Großbuchstaben bleibt der
///   Bindestrich ohne Leerzeichen („Rheinland-Pfalz“).
/// - Zeilen nur aus einer kurzen Zahl an einer Seitengrenze (Seitenzahlen)
///   entfallen.
///
/// Anders als beim EPUB bleiben `<…>` und `[…]` stehen: PDFs sind oft
/// eigene Skripte mit Sprechern und Tags.
pub fn reflow_plain_text(text: &str) -> String {
    let lines: Vec<(String, Option<u32>)> = text
        .replace('\u{0C}', "\n\n")
        .lines()
        .map(|l| (l.to_string(), None))
        .collect();
    reflow_lines(lines)
}

/// Seitenmarke im Seitentext: `{{S. 12}}`. Die Buchansicht zeigt sie als
/// Seitenzahl des Originals, die Synthese liest sie nicht vor.
pub fn page_mark(page: u32) -> String {
    format!("{{{{S. {page}}}}}")
}

/// Wie [`reflow_plain_text`], aber je Zeile optional die Seite des Originals,
/// die mit dieser Zeile beginnt; sie landet als [`page_mark`] im Absatz.
pub(crate) fn reflow_lines(raw: Vec<(String, Option<u32>)>) -> String {
    let (lines, starts): (Vec<String>, Vec<Option<u32>>) = raw
        .into_iter()
        .map(|(l, m)| (l.split_whitespace().collect::<Vec<_>>().join(" "), m))
        .unzip();
    let mut lengths: Vec<usize> = lines
        .iter()
        .map(|l| l.chars().count())
        .filter(|&n| n > 20)
        .collect();
    lengths.sort_unstable();
    let typical = lengths.get(lengths.len() * 3 / 4).copied().unwrap_or(0);

    let starts_block = |line: &str| LIST_LINE_RE.is_match(line) || SPEAKER_LINE_RE.is_match(line);
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    // Seitenanfänge im laufenden Absatz: (Byte-Position, Seite).
    let mut marks: Vec<(usize, u32)> = Vec::new();
    let mut pending: Option<u32> = None;
    let end_paragraph =
        |current: &mut String, marks: &mut Vec<(usize, u32)>, paragraphs: &mut Vec<String>| {
            let text = insert_page_marks(current, marks);
            if !text.trim().is_empty() {
                paragraphs.push(text.trim().to_string());
            }
            current.clear();
            marks.clear();
        };
    for (i, line) in lines.iter().enumerate() {
        if let Some(page) = starts[i] {
            pending = Some(page);
        }
        if line.is_empty() {
            end_paragraph(&mut current, &mut marks, &mut paragraphs);
            continue;
        }
        let at_gap = |j: Option<usize>| j.and_then(|j| lines.get(j)).is_none_or(String::is_empty);
        if line.len() <= 3
            && line.chars().all(|c| c.is_ascii_digit())
            && (at_gap(i.checked_sub(1)) || at_gap(Some(i + 1)))
        {
            continue;
        }
        // „… und erst am / 23. kommt.“: eine Zahl mit Punkt am Zeilenanfang
        // beginnt nur dann einen Listenpunkt, wenn der Satz davor zu Ende ist
        // oder schon eine Liste läuft.
        // „… und erst am ⏎ 23. kommt.“: Zahl mit Punkt, danach klein weiter.
        let mid_sentence = NUMBER_THEN_LOWER_RE.is_match(line)
            && !current.is_empty()
            && !LIST_LINE_RE.is_match(&current)
            && !current
                .trim_end_matches(['"', '“', '”', '«', '»', '’', '\'', ')'])
                .ends_with(['.', '!', '?', ':', '…']);
        if starts_block(line) && !mid_sentence {
            end_paragraph(&mut current, &mut marks, &mut paragraphs);
        }
        let first_word = line.split(' ').next().unwrap_or_default();
        if current.is_empty() {
            if let Some(page) = pending.take() {
                // Hinter `- `, `1. ` oder `Name:`, damit Liste und Sprecher
                // in der Buchansicht erkannt bleiben.
                let prefix = LIST_LINE_RE
                    .find(line)
                    .or_else(|| SPEAKER_LINE_RE.find(line))
                    .map_or(0, |m| m.end());
                marks.push((prefix, page));
            }
            current.push_str(line);
        } else if current.ends_with('-')
            && !current.ends_with(" -")
            && current[..current.len() - 1]
                .chars()
                .next_back()
                .is_some_and(char::is_alphabetic)
        {
            let next_lower = line.chars().next().is_some_and(char::is_lowercase);
            if next_lower
                && !matches!(
                    first_word,
                    "und" | "oder" | "bzw." | "sowie" | "bis" | "als"
                )
            {
                current.pop();
            } else if next_lower {
                current.push(' ');
            }
            if let Some(page) = pending.take() {
                marks.push((current.len(), page));
            }
            current.push_str(line);
        } else {
            current.push(' ');
            if let Some(page) = pending.take() {
                marks.push((current.len(), page));
            }
            current.push_str(line);
        }
        let n = line.chars().count();
        let ends_sentence = line
            .trim_end_matches(['"', '“', '”', '«', '»', '’', '\'', ')'])
            .ends_with(['.', '!', '?', ':', '…']);
        let next_upper = lines
            .get(i + 1)
            .and_then(|l| l.chars().next())
            .is_some_and(char::is_uppercase);
        let short_heading =
            typical > 0 && n * 2 < typical && !line.ends_with([',', '-', ';']) && next_upper;
        if typical > 0 && ((ends_sentence && n * 10 < typical * 8) || short_heading) {
            end_paragraph(&mut current, &mut marks, &mut paragraphs);
        }
    }
    end_paragraph(&mut current, &mut marks, &mut paragraphs);
    paragraphs.join("\n\n")
}

/// Seitenmarken in einen Absatz setzen: vor das erste Wort der Seite, oder —
/// wenn die Seite mitten im Wort beginnt (getrenntes Wort über die
/// Seitengrenze) — hinter dieses Wort.
pub(crate) fn insert_page_marks(text: &str, marks: &[(usize, u32)]) -> String {
    let mut out = String::with_capacity(text.len() + marks.len() * 12);
    let mut from = 0;
    for &(at, page) in marks {
        let at = at.min(text.len()).max(from);
        // Mitten im Wort auch nach einem stehen gebliebenen Bindestrich
        // („Süd-“ / „Ost“ auf der nächsten Seite).
        let mid_word = text[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '-')
            && text[at..].chars().next().is_some_and(char::is_alphanumeric);
        if mid_word {
            let end = text[at..]
                .find(char::is_whitespace)
                .map_or(text.len(), |e| at + e);
            out.push_str(&text[from..end]);
            out.push(' ');
            out.push_str(&page_mark(page));
            from = end;
        } else {
            out.push_str(&text[from..at]);
            out.push_str(&page_mark(page));
            out.push(' ');
            from = at;
        }
    }
    out.push_str(&text[from..]);
    out
}

/// Wie viele Zeilen oben und unten einer PDF-Seite höchstens als Kopf-/Fußzeile
/// in Frage kommen. Auf kurzen Seiten weniger (ein Drittel der Zeilen), damit
/// der Fließtext einer kurzen Seite nie ganz als „Rand“ gilt; entfernt wird
/// dort ohnehin nur, was sich buchweit wiederholt.
const RUNNING_LINES: usize = 3;

/// Schlüssel einer Kopf-/Fußzeile für den buchweiten Abgleich: Ziffern zählen
/// nicht („Seite 41 von 300“ und „Seite 42 von 300“ sind dieselbe Fußzeile),
/// Leerraum und Groß-/Kleinschreibung auch nicht.
pub(crate) fn running_key(line: &str) -> String {
    line.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect()
}

/// Nur eine Seitenzahl: „12“, „- 12 -“, „Seite 12“, „S. 12“, „12 / 300“.
/// Kein Punkt am Ende („1914.“ ist ein Satzende, „1.“ ein Listenpunkt).
pub(crate) fn is_page_number(line: &str) -> bool {
    let t = line.trim();
    let lower = t.to_lowercase();
    let t = ["seite", "s.", "page", "p."]
        .iter()
        .find(|p| lower.starts_with(*p))
        .map_or(t, |p| &t[p.len()..])
        .trim();
    // „12 von 300“ / „12 of 300“ wie „12 / 300“.
    let t = &t.replace(" von ", " / ").replace(" of ", " / ");
    let digits = t.chars().filter(char::is_ascii_digit).count();
    (1..=8).contains(&digits)
        && t.chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, '-' | '–'))
        && !t.ends_with('.')
        && t.chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || matches!(c, '-' | '–' | '/' | '|'))
        && t.split(['/', '|'])
            .all(|part| part.chars().filter(char::is_ascii_digit).count() <= 4)
}

/// Seitenzahl mit Wort davor („Seite 12“, „S. 12“, „Page 12“).
pub(crate) fn has_page_word(line: &str) -> bool {
    let l = line.trim_start().to_lowercase();
    ["seite ", "s. ", "page ", "p. "]
        .iter()
        .any(|w| l.starts_with(w))
}

/// Rand einer Seite: `Some(true)` oben, `Some(false)` unten, `None` Fließtext.
pub(crate) fn page_zone(len: usize, i: usize) -> Option<bool> {
    // Seiten mit kaum mehr als Kopf- und Fußzeile (Bildtafeln): alle Zeilen
    // sind Rand.
    // Bildseiten mit wenigen Zeilen: ein Drittel der Zeilen.
    let k = if len <= 6 {
        len.div_ceil(2)
    } else {
        RUNNING_LINES.min(len.div_ceil(3))
    };
    if i < k {
        Some(true)
    } else if i + k >= len {
        Some(false)
    } else {
        None
    }
}

/// Kandidat für eine Titelzeile unter „Kapitel 3“: kurz, groß beginnend, kein
/// Satz und deutlich kürzer als die üblichen Textzeilen.
pub(crate) fn looks_like_chapter_title(line: &str, typical: usize) -> bool {
    let n = line.chars().count();
    (2..=60).contains(&n)
        && line.split_whitespace().count() <= 9
        && line.chars().next().is_some_and(char::is_uppercase)
        && line.chars().filter(|c| c.is_alphabetic()).count() >= 2
        && !line.ends_with(['.', ',', ';', ':', '-', '–'])
        && !is_page_number(line)
        && !CHAPTER_HEAD_RE.is_match(line)
        && !SPECIAL_HEAD_RE.is_match(line)
        && (typical == 0 || n * 10 < typical * 7)
}

/// Kapitelüberschriften ganz oben auf einer Seite (`Kapitel 3`, `Prolog` …,
/// ggf. mit Titel in der nächsten kurzen Zeile). Liefert die Überschriften
/// (ohne `## `) und wie viele Zeilen sie verbraucht haben. Nur vom
/// Seitenanfang aus: eine Fundstelle mitten im Text ist meist ein Verweis.
pub(crate) fn page_chapter_headings(kept: &[&str], typical: usize) -> (Vec<String>, usize) {
    let norm = |l: &str| l.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut headings = Vec::new();
    let mut at = 0;
    while at < kept.len() && headings.len() < 2 {
        let line = norm(kept[at]);
        if let Some(c) = CHAPTER_HEAD_TITLE_RE.captures(&line) {
            let title = c[2].trim();
            if looks_like_chapter_title(title, 0) {
                headings.push(format!("{}: {title}", c[1].trim_end_matches(['.', ' '])));
                at += 1;
                continue;
            }
        }
        let is_head = CHAPTER_HEAD_RE.is_match(&line) || SPECIAL_HEAD_RE.is_match(&line);
        if !is_head {
            break;
        }
        at += 1;
        let head = line.trim_end_matches('.').to_string();
        match kept.get(at).map(|l| norm(l)) {
            Some(title) if looks_like_chapter_title(&title, typical) => {
                headings.push(format!("{head}: {title}"));
                at += 1;
            }
            _ => headings.push(head),
        }
    }
    (headings, at)
}

/// Lauftitel für die Überschrift: Seitenzahlen und Trennzeichen am Rand weg.
pub(crate) fn clean_running_title(line: &str) -> String {
    let t = line.split_whitespace().collect::<Vec<_>>().join(" ");
    t.trim_matches(|c: char| c.is_ascii_digit() || c.is_whitespace() || "|·•–—-/".contains(c))
        .to_string()
}

/// PDF-Seiten (je Seite der Text) zu einem Text zusammenführen und dabei
/// Kopf- und Fußzeilen entfernen sowie Seitenzahlen am Seitenrand:
///
/// - buchweit: eine Randzeile, die (ohne Ziffern verglichen) auf mindestens
///   einem Drittel der Seiten an derselben Stelle (oben/unten) steht — Buchtitel,
///   „Seite 12 von 300“; erst ab zehn Seiten, sonst wären „Abschnitt 1“,
///   „Abschnitt 2“ … oben auf jeder Seite eines kurzen Texts Kopfzeilen;
/// - kapitelweit: eine wortgleiche Randzeile auf mindestens drei Seiten mit
///   höchstens einer Seite Abstand — „Kapitel 5 · Die Klippe“ über die Seiten des
///   Kapitels. Wortgleich, damit „Abschnitt 1“, „Abschnitt 2“ … bleiben;
/// - Seitenzahl: nur die erste oder letzte Zeile einer Seite.
///
/// Kapitel werden als `## Titel`-Zeilen ausgegeben, damit die Kapitelliste der
/// Buchansicht etwas findet: (a) `Kapitel 3`, `Teil 2`, `Prolog` … ganz oben auf
/// der Seite (mit Titelzeile darunter zu „Kapitel 3: Titel“); (b) wo sich der
/// entfernte Lauftitel ändert und (a) nichts fand, dieser Lauftitel. Lieber
/// keine Überschrift als eine falsche: ein Lauftitel, der auf der halben
/// Buchlänge steht (Buchtitel), wechselnde Titel im Seitentakt (links Buch,
/// rechts Kapitel) und einmalige Titel ergeben keine.
///
/// Danach [`reflow_plain_text`]. Die Seiten hängen mit einem einfachen Umbruch
/// aneinander — wie bisher bei `extract_text`, das auch keine Seitengrenze
/// markiert —, damit ein Absatz über eine Seitengrenze ein Absatz bleibt.
///
/// Der Import nimmt [`pdf_book_to_text`] (mit Schriftgrößen); diese Form ohne
/// Größen dient den Tests mit reinem Seitentext.
#[cfg(test)]
pub fn pdf_pages_to_text(pages: &[String]) -> String {
    let book = crate::pdf_text::PdfBook {
        pages: pages
            .iter()
            .map(|p| {
                p.lines()
                    .map(|l| crate::pdf_text::PdfLine {
                        text: l.to_string(),
                        size: 0.0,
                        gap: false,
                    })
                    .collect()
            })
            .collect(),
        outline: Vec::new(),
    };
    pdf_book_to_text(&book)
}

/// Wie `pdf_pages_to_text`, mit Schriftgrößen und Lesezeichen
/// ([`crate::pdf_text::read_pdf`]). Zusätzliche Überschriften:
///
/// - **Lesezeichen** (Outline) des PDFs, wenn es welche hat — dann allein,
///   ohne die Heuristiken darunter;
/// - sonst Einträge eines **Inhaltsverzeichnisses** im Vorspann, die im Text
///   als eigene Zeile wiederkehren („KHM 12. RAPUNZEL“), in Reihenfolge;
/// - und Zeilen in **größerer Schrift** als der Fließtext.
pub fn pdf_book_to_text(book: &crate::pdf_text::PdfBook) -> String {
    use std::collections::{HashMap, HashSet};
    // Leerzeilen weg; gesperrt gesetzte Wörter („S E I T E 12“) nur am
    // Seitenrand zusammenziehen: im Fließtext wäre Sperrsatz („d e r K ö n i g“)
    // nicht von Wortabständen zu unterscheiden und klebte zusammen.
    let owned: Vec<Vec<(String, f32, bool)>> = book
        .pages
        .iter()
        .map(|p| {
            let lines: Vec<&crate::pdf_text::PdfLine> =
                p.iter().filter(|l| !l.text.trim().is_empty()).collect();
            let n = lines.len();
            lines
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let text = if page_zone(n, i).is_some() {
                        despace(&l.text)
                    } else {
                        l.text.clone()
                    };
                    (text, l.size, l.gap)
                })
                .filter(|(t, _, _)| !t.trim().is_empty())
                .collect()
        })
        .collect();
    let page_lines: Vec<Vec<&str>> = owned
        .iter()
        .map(|p| p.iter().map(|(t, _, _)| t.as_str()).collect())
        .collect();
    let exact_key = |line: &str| line.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut masked: HashMap<(bool, String), usize> = HashMap::new();
    // Je Randzeile mit Zahl: Zahl minus Seitenindex, um mitlaufende
    // Seitenzahlen in Kopfzeilen zu erkennen („… · Seite 3“).
    let mut tracking: HashMap<(bool, String), HashMap<i64, usize>> = HashMap::new();
    let mut exact: HashMap<(bool, String), Vec<usize>> = HashMap::new();
    for (p, lines) in page_lines.iter().enumerate() {
        let mut seen = HashSet::new();
        for (i, line) in lines.iter().enumerate() {
            let Some(top) = page_zone(lines.len(), i) else {
                continue;
            };
            let key = running_key(line);
            if seen.insert((top, key.clone())) {
                *masked.entry((top, key.clone())).or_default() += 1;
                if let Some(n) = last_number(line) {
                    *tracking
                        .entry((top, key))
                        .or_default()
                        .entry(n as i64 - p as i64)
                        .or_default() += 1;
                }
            }
            let pages_of = exact.entry((top, exact_key(line))).or_default();
            if pages_of.last() != Some(&p) {
                pages_of.push(p);
            }
        }
    }
    let with_text = page_lines.iter().filter(|l| !l.is_empty()).count();
    let threshold = with_text.div_ceil(3).max(3);
    let chapter_run = |pages_of: &[usize]| {
        let mut run = 1;
        pages_of.windows(2).any(|w| {
            run = if w[1] - w[0] <= 2 { run + 1 } else { 1 };
            run >= 3
        })
    };
    // Kurze PDFs (unter zehn Seiten): eine Randzeile mit Text und einer
    // Seitenzahl, die Seite für Seite mitzählt, auf mindestens der Hälfte der
    // Seiten (mindestens drei) ist eine Kopf- oder Fußzeile. „Abschnitt 1“,
    // „Abschnitt 2“ … bleiben: es braucht zwei Wörter neben der Zahl.
    let page_counter = |top: bool, line: &str| {
        let key = (top, running_key(line));
        let words = line
            .split_whitespace()
            .filter(|w| w.chars().any(char::is_alphabetic))
            .count();
        words >= 2
            && masked
                .get(&key)
                .is_some_and(|&n| n >= 3 && n * 2 >= with_text)
            && tracking.get(&key).is_some_and(|offsets| {
                let total: usize = offsets.values().sum();
                offsets.values().max().is_some_and(|&m| m * 5 >= total * 4)
            })
    };
    // Auf Seiten mit höchstens sechs Zeilen (Bildtafeln) zählt nur, was
    // buchweit Kopf- oder Fußzeile ist, nicht die Kapitelregel.
    // Buchweit zählt eine Randzeile oben wie unten: manche PDFs schreiben die
    // Fußzeile im Inhaltsstrom vor den Seitentext.
    let running = |top: bool, line: &str, short_page: bool| {
        (with_text >= 10
            && [top, !top].iter().any(|&t| {
                masked
                    .get(&(t, running_key(line)))
                    .is_some_and(|&n| n >= threshold)
            }))
            || (!short_page
                && exact
                    .get(&(top, exact_key(line)))
                    .is_some_and(|p| chapter_run(p)))
            || page_counter(top, line)
    };

    // Gedruckte Seitenzahl = Seitenindex + Versatz (Vorspann ohne Zahl,
    // Kapitelanfänge ohne Zahl): der häufigste Versatz der erkannten Zahlen.
    let mut offsets: HashMap<i64, usize> = HashMap::new();
    for (p, lines) in page_lines.iter().enumerate() {
        let edges = [lines.first(), lines.last()].into_iter().flatten();
        let worded = lines
            .iter()
            .enumerate()
            .filter(|(i, _)| page_zone(lines.len(), *i).is_some())
            .map(|(_, l)| l)
            .filter(|l| has_page_word(l));
        let printed = edges
            .chain(worded)
            .find(|l| is_page_number(l))
            .and_then(|l| first_number(l));
        if let Some(n) = printed {
            *offsets.entry(n as i64 - p as i64).or_default() += 1;
        }
    }
    let offset = offsets
        .into_iter()
        .max_by_key(|&(o, n)| (n, -o.abs()))
        .map_or(1, |(o, _)| o);

    // Pro Seite: die behaltenen Zeilen (mit Schriftgröße) und der entfernte
    // Lauftitel (oben).
    let mut kept_pages: Vec<Vec<(&str, f32)>> = Vec::with_capacity(page_lines.len());
    let mut kept_gaps: Vec<Vec<bool>> = Vec::with_capacity(page_lines.len());
    let mut run_titles: Vec<Vec<&str>> = Vec::with_capacity(page_lines.len());
    for (p, lines) in page_lines.iter().enumerate() {
        let mut kept = Vec::new();
        let mut gaps = Vec::new();
        let mut run_title: Vec<&str> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let at_edge = i == 0 || i + 1 == lines.len();
            let zone = page_zone(lines.len(), i);
            let run = zone.is_some_and(|top| running(top, line, lines.len() <= 6));
            let numbered =
                is_page_number(line) && (at_edge || (zone.is_some() && has_page_word(line)));
            if run || numbered {
                if run && zone == Some(true) && !is_page_number(line) {
                    run_title.push(*line);
                }
            } else {
                kept.push((*line, owned[p][i].1));
                gaps.push(owned[p][i].2);
            }
        }
        // Die erste Zeile einer Seite steht immer „nach einem Abstand“ (Seitenkopf,
        // entfernte Kopfzeile): kein Absatzwechsel, sonst risse jeder Seitenwechsel
        // den Satz auseinander.
        if let Some(first) = gaps.first_mut() {
            *first = false;
        }
        kept_pages.push(kept);
        kept_gaps.push(gaps);
        run_titles.push(run_title);
    }

    // Übliche Textzeilenlänge (wie in `reflow_lines`) für die Titelprobe.
    let mut lengths: Vec<usize> = kept_pages
        .iter()
        .flatten()
        .map(|(l, _)| exact_key(l).chars().count())
        .filter(|&n| n > 20)
        .collect();
    lengths.sort_unstable();
    let typical = lengths.get(lengths.len() * 3 / 4).copied().unwrap_or(0);

    // Überschriften mitten auf der Seite: je behaltener Zeile (Ebene, Gruppe).
    // Zeilen einer Gruppe werden zu einer Überschrift verbunden; der Text
    // kommt aus `titles` (Inhaltsverzeichnis), sonst aus den Zeilen selbst.
    let mut flags: Vec<Vec<Option<(u8, usize)>>> =
        kept_pages.iter().map(|k| vec![None; k.len()]).collect();
    let mut titles: HashMap<usize, String> = HashMap::new();
    let mut group = 0usize;
    // Eingefügte Überschriften (Lesezeichen ohne Fundstelle): Zeile, vor der
    // sie stehen, Ebene, Text.
    let mut inserted: Vec<Vec<(usize, u8, String)>> = vec![Vec::new(); kept_pages.len()];
    // Seiten-Lesezeichen von Scannern („Seite 3“) sind keine Kapitel.
    let outline: Vec<&crate::pdf_text::OutlineEntry> = book
        .outline
        .iter()
        .filter(|e| !is_page_number(&e.title))
        .collect();
    let mut has_outline = false;
    if outline.len() >= 2 {
        let mut found_count = 0usize;
        let mut missing = Vec::new();
        for entry in &outline {
            let level = (entry.level + 2).min(3) as u8;
            group += 1;
            let found = [entry.page, entry.page + 1].into_iter().find(|&p| {
                kept_pages.get(p).is_some_and(|kept| {
                    mark_title(kept, &mut flags[p], &entry.title, level, group, 0)
                })
            });
            match found {
                Some(_) => {
                    found_count += 1;
                    titles.insert(group, entry.title.clone());
                }
                None => missing.push((entry.page, level, entry.title.clone())),
            }
        }
        // Findet sich nicht wenigstens die Hälfte der Lesezeichen im Text,
        // taugen sie nicht als Gliederung: dann die anderen Wege.
        if found_count * 2 >= outline.len() {
            has_outline = true;
            for (page, level, title) in missing {
                let at = insert_point(&kept_pages, page);
                inserted[page].push((at, level, title));
            }
        } else {
            for f in flags.iter_mut().flatten() {
                *f = None;
            }
            titles.clear();
        }
    }
    if !has_outline {
        // Inhaltsverzeichnis im ersten Drittel: Einträge im Text wiederfinden.
        if let Some((entries, start)) = toc_entries(&kept_pages) {
            // Einmal normalisieren: sonst kostet der Abgleich (30 Einträge je
            // Zeile) bei einem dicken Buch über eine Sekunde.
            let norms: Vec<String> = entries.iter().map(|e| title_norm(e)).collect();
            let mut cursor = 0usize;
            let mut matched = 0usize;
            'pages: for p in start.0..kept_pages.len() {
                let from = if p == start.0 { start.1 } else { 0 };
                let mut i = from;
                while i < kept_pages[p].len() {
                    if cursor >= entries.len() {
                        break 'pages;
                    }
                    let line = kept_pages[p][i].0;
                    let line_norm = title_norm(line);
                    let hit = (cursor..(cursor + 30).min(entries.len())).find(|&e| {
                        title_match_norm(line, &line_norm, &entries[e], &norms[e]).is_some()
                    });
                    if let Some(e) = hit {
                        // Zählt nur, wenn genau diese Zeile (ggf. mit Folgezeilen)
                        // markiert wird, nicht eine spätere auf der Seite.
                        group += 1;
                        let before = flags[p].clone();
                        let marked =
                            mark_title(&kept_pages[p], &mut flags[p], &entries[e], 2, group, i)
                                && flags[p][i] == Some((2, group));
                        if marked {
                            titles.insert(group, entries[e].clone());
                            matched += 1;
                            cursor = e + 1;
                        } else {
                            flags[p] = before;
                        }
                    }
                    i += 1;
                }
            }
            // Ein einzelner Treffer ist eher Zufall als ein Verzeichnis.
            if matched < 2 {
                for f in flags.iter_mut().flatten() {
                    *f = None;
                }
                titles.clear();
            }
        }
        size_headings(&kept_pages, &mut flags, &mut group, typical);
    }

    // (a) Kapitelüberschriften am Seitenanfang (nur ohne Lesezeichen).
    let mut headings: Vec<Vec<(u8, String)>> = Vec::with_capacity(kept_pages.len());
    let mut skip: Vec<usize> = Vec::with_capacity(kept_pages.len());
    for (p, kept) in kept_pages.iter().enumerate() {
        if has_outline || flags[p].first().is_some_and(Option::is_some) {
            headings.push(Vec::new());
            skip.push(0);
            continue;
        }
        let lines: Vec<&str> = kept.iter().map(|(l, _)| *l).collect();
        let (h, used) = page_chapter_headings(&lines, typical);
        // Eine schon markierte Zeile gehört der Markierung.
        if flags[p][..used].iter().any(Option::is_some) {
            headings.push(Vec::new());
            skip.push(0);
            continue;
        }
        headings.push(h.into_iter().map(|h| (2, h)).collect());
        skip.push(used);
    }

    // (b) Wechselnde Lauftitel, wo sonst nichts gefunden wurde. Verglichen
    // wird der Titel ohne Seitenzahl am Rand („12 Die Klippe“ = „Die Klippe“).
    let any_flag = flags.iter().flatten().any(Option::is_some);
    let title_key = |t: &str| clean_running_title(t).to_lowercase();
    let mut title_pages: HashMap<String, usize> = HashMap::new();
    for t in run_titles.iter().flatten() {
        *title_pages.entry(title_key(t)).or_default() += 1;
    }
    // Pro Seite der erste Randtext, der kein Buchtitel ist (steht auf der
    // halben Buchlänge oder mehr) und Buchstaben enthält.
    let usable = |k: &str| {
        let twice = title_pages.get(k).copied().unwrap_or(0) * 2;
        k.chars().any(char::is_alphabetic)
            && (twice < with_text || (twice == with_text && with_text < 10))
    };
    let titled: Vec<(usize, String, &str)> = run_titles
        .iter()
        .enumerate()
        .filter_map(|(p, lines)| {
            lines
                .iter()
                .map(|t| (p, title_key(t), *t))
                .find(|(_, k, _)| usable(k))
        })
        .collect();
    for (i, (p, key, raw)) in titled.iter().enumerate() {
        let changed = i == 0 || titled[i - 1].1 != *key;
        // Wechsel im Seitentakt (A, B, A, B …) sind keine Kapitel.
        let flips_back = i >= 2 && titled[i - 2].1 == *key;
        // Der neue Titel muss sich halten (mindestens zwei Seiten).
        let persists = titled.get(i + 1).is_some_and(|n| n.1 == *key);
        if changed
            && !flips_back
            && persists
            && !has_outline
            && !any_flag
            && headings[*p].is_empty()
        {
            headings[*p].push((2, clean_running_title(raw)));
        }
    }

    // Absatzabstände (mehr als anderthalb Zeilen) trennen Absätze, wenn sie
    // die Ausnahme sind (bis zwei Fünftel der Zeilen); bei weitem
    // Zeilenabstand stünde sonst jede Zeile für sich.
    let gap_lines = kept_gaps.iter().flatten().filter(|g| **g).count();
    let all_lines = kept_gaps.iter().map(Vec::len).sum::<usize>();
    let use_gaps = gap_lines > 0 && gap_lines * 5 <= all_lines * 2;

    let mut out: Vec<(String, Option<u32>)> = Vec::new();
    let heading = |out: &mut Vec<(String, Option<u32>)>, level: u8, text: &str| {
        out.push((String::new(), None));
        out.push((format!("{} {text}", "#".repeat(level as usize)), None));
        out.push((String::new(), None));
    };
    for (p, kept) in kept_pages.iter().enumerate() {
        // Ohne zweite Seite keine Marken: ein Einseiter braucht keine Seitenzahl.
        let mut mark = (book.pages.len() > 1)
            .then(|| u32::try_from(p as i64 + offset).ok())
            .flatten()
            .filter(|&n| n > 0);
        // Überschriften als eigene Absätze; die Seitenmarke gehört an die
        // erste Textzeile danach, sonst stünde sie mitten in der Überschrift.
        for (level, h) in &headings[p] {
            heading(&mut out, *level, h);
        }
        let mut ins = 0usize;
        let mut i = skip[p];
        while i < kept.len() {
            while ins < inserted[p].len() && inserted[p][ins].0 <= i {
                heading(&mut out, inserted[p][ins].1, &inserted[p][ins].2);
                ins += 1;
            }
            let Some((level, g)) = flags[p][i] else {
                let starts_lower = kept[i].0.trim_start().starts_with(char::is_lowercase);
                if use_gaps && kept_gaps[p][i] && !starts_lower {
                    out.push((String::new(), None));
                }
                out.push((kept[i].0.to_string(), mark.take()));
                i += 1;
                continue;
            };
            let mut parts = Vec::new();
            while i < kept.len() && flags[p][i].is_some_and(|(_, h)| h == g) {
                parts.push(exact_key(kept[i].0));
                i += 1;
            }
            let text = titles
                .get(&g)
                .map(|t| heading_text(&parts.join(" "), t))
                .unwrap_or_else(|| clean_heading(&parts.join(" ")));
            heading(&mut out, level, &text);
        }
        for (_, level, h) in &inserted[p][ins..] {
            heading(&mut out, *level, h);
        }
    }
    reflow_lines(out)
}

/// Endet die Zeile einen Satz (auch hinter Anführungszeichen)?
pub(crate) fn ends_sentence(line: &str) -> bool {
    line.trim_end()
        .trim_end_matches(['"', '“', '”', '«', '»', '’', '\'', ')'])
        .ends_with(['.', '!', '?', '…', ':'])
}

/// Wohin eine Überschrift ohne Fundstelle auf ihrer Seite kommt: vor die
/// erste Zeile, die nach einem Satzende beginnt. Sonst risse sie den letzten
/// Absatz der Vorseite auseinander.
pub(crate) fn insert_point(kept_pages: &[Vec<(&str, f32)>], page: usize) -> usize {
    let Some(kept) = kept_pages.get(page) else {
        return 0;
    };
    let before_page = kept_pages[..page]
        .iter()
        .rev()
        .find_map(|k| k.last().map(|(l, _)| *l));
    (0..kept.len())
        .find(|&i| {
            let prev = if i == 0 {
                before_page
            } else {
                Some(kept[i - 1].0)
            };
            prev.is_none_or(ends_sentence)
        })
        .unwrap_or(0)
}

/// Gesperrt gesetzte Wörter zusammenziehen: drei und mehr einzelne
/// Buchstaben mit Leerraum dazwischen („S E I T E 12“ → „SEITE 12“).
pub(crate) fn despace(line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.iter().filter(|w| w.chars().count() == 1).count() < 3 {
        return line.to_string();
    }
    let single = |w: &str| w.chars().count() == 1 && w.chars().all(char::is_alphabetic);
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let run = words[i..].iter().take_while(|w| single(w)).count();
        if run >= 3 {
            out.push(words[i..i + run].concat());
            i += run;
        } else {
            out.push(words[i].to_string());
            i += 1;
        }
    }
    out.join(" ")
}

/// Letzte Zahl einer Zeile („… · Seite 12“ → 12).
pub(crate) fn last_number(line: &str) -> Option<u32> {
    line.split(|c: char| !c.is_ascii_digit())
        .rfind(|s| !s.is_empty() && s.len() <= 5)
        .and_then(|s| s.parse().ok())
}

/// Vergleichsform eines Titels: klein, nur Buchstaben und Ziffern, ß = ss.
pub(crate) fn title_norm(s: &str) -> String {
    s.to_lowercase()
        .replace('ß', "ss")
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Passt die Zeile zu einem Titel? `Some(true)`: ganz (höchstens ein paar
/// Zeichen mehr, etwa ein Fußnotenstern); `Some(false)`: sie ist der Anfang
/// eines mehrzeiligen Titels (mindestens die Hälfte davon).
pub(crate) fn title_match(line: &str, title: &str) -> Option<bool> {
    title_match_norm(line, &title_norm(line), title, &title_norm(title))
}

/// `title_match` mit schon normalisierter Zeile und Titel.
pub(crate) fn title_match_norm(line: &str, l: &str, title: &str, t: &str) -> Option<bool> {
    if t.chars().filter(|c| c.is_alphabetic()).count() < 2 || l.is_empty() {
        return None;
    }
    if l == t || (l.starts_with(t) && l.len() <= t.len() + 3) {
        return Some(true);
    }
    if t.starts_with(l) && t[l.len()..].starts_with(' ') && l.len() * 2 >= t.len() {
        return Some(false);
    }
    // Nummerierte Einträge („KHM 4. Märchen von …“): gleiche Nummer am
    // Zeilenanfang und danach ein Titel in Großbuchstaben genügt, auch wenn
    // der Wortlaut abweicht („VOM EINEM“) oder der Titel umbricht.
    let (Some(ti), Some(li)) = (TITLE_ID_RE.find(title), TITLE_ID_RE.find(line.trim())) else {
        return None;
    };
    // Nur der Titel bis zu einer Klammer oder einem Zitat danach zählt.
    let rest = line.trim()[li.end()..]
        .split(['(', '"', '„', '“', '['])
        .next()
        .unwrap_or_default();
    let letters: Vec<char> = rest
        .chars()
        .filter(|c| c.is_alphabetic())
        .take(30)
        .collect();
    let upper = letters.iter().filter(|c| c.is_uppercase()).count();
    (title_norm(ti.as_str()) == title_norm(li.as_str())
        && letters.len() >= 3
        && upper * 10 >= letters.len() * 8)
        .then_some(true)
}

/// Den Titel ab Zeile `from` der Seite suchen und markieren (bei
/// mehrzeiligen Titeln die Folgezeilen mit). `true`, wenn gefunden.
pub(crate) fn mark_title(
    kept: &[(&str, f32)],
    flags: &mut [Option<(u8, usize)>],
    title: &str,
    level: u8,
    group: usize,
    from: usize,
) -> bool {
    for i in from..kept.len() {
        if flags[i].is_some() {
            continue;
        }
        match title_match(kept[i].0, title) {
            Some(true) => {
                flags[i] = Some((level, group));
                return true;
            }
            Some(false) => {
                // Folgezeilen, solange der Titel weitergeht.
                let mut acc = kept[i].0.to_string();
                let mut end = i + 1;
                while end < kept.len() && end < i + 3 {
                    acc = format!("{acc} {}", kept[end].0);
                    match title_match(&acc, title) {
                        Some(true) => {
                            for f in &mut flags[i..=end] {
                                *f = Some((level, group));
                            }
                            return true;
                        }
                        Some(false) => end += 1,
                        None => break,
                    }
                }
            }
            None => {}
        }
    }
    false
}

/// Überschrift eines Inhaltsverzeichnis-Eintrags: der Text aus dem
/// Verzeichnis, wenn die Zeile im Text in Großbuchstaben steht („KHM 12.
/// RAPUNZEL*“ → „KHM 12. Rapunzel“), sonst die Zeile selbst.
pub(crate) fn heading_text(line: &str, title: &str) -> String {
    let letters: Vec<char> = line.chars().filter(|c| c.is_alphabetic()).collect();
    let upper = letters.iter().filter(|c| c.is_uppercase()).count();
    if letters.len() >= 4 && upper * 10 >= letters.len() * 8 {
        clean_heading(title)
    } else {
        clean_heading(line)
    }
}

/// Fußnotenzeichen und Füllpunkte am Ende einer Überschrift weg.
pub(crate) fn clean_heading(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(['*', '†', ' ', '.', '…', '·', '_'])
        .to_string()
}

/// Überschrift eines Inhaltsverzeichnisses („Inhalt“, „Contents“ …).
pub(crate) fn is_toc_heading(line: &str) -> bool {
    matches!(
        title_norm(line).as_str(),
        "inhalt"
            | "inhaltsverzeichnis"
            | "inhaltsübersicht"
            | "verzeichnis"
            | "contents"
            | "table of contents"
            | "sommaire"
            | "table des matières"
            | "indice"
    )
}

/// Eintrag eines Inhaltsverzeichnisses: Aufzählungszeichen vorn sowie
/// Füllpunkte und Seitenzahl hinten weg. `None` für Zeilen ohne Titel.
pub(crate) fn toc_entry(line: &str) -> Option<String> {
    let t = line
        .trim()
        .trim_start_matches(['•', '·', '-', '–', '—', '*', '▪', '◦', ' ']);
    // Seitenzahl hinten nur weg, wenn davor Füllpunkte stehen oder noch
    // mindestens zwei Wörter bleiben („Teil 1“ behält seine Nummer).
    let stripped = t.trim_end_matches(|c: char| c.is_ascii_digit() || c.is_whitespace());
    let leaders = stripped.ends_with(['.', '·', '…', '_'])
        && stripped.trim_end_matches(['.', '·', '…', '_', ' ']).len() + 2 < stripped.len();
    let t = if stripped.len() < t.len() && (leaders || stripped.split_whitespace().count() >= 2) {
        stripped.trim_end_matches(['.', '·', '…', '_', ' ', '-', '–'])
    } else {
        t
    };
    let t = t.trim();
    let letters = t.chars().filter(|c| c.is_alphabetic()).count();
    let page_word = matches!(title_norm(t).as_str(), "seite" | "page" | "s" | "p");
    (letters >= 2 && t.chars().count() <= 120 && !is_toc_heading(t) && !page_word)
        .then(|| t.to_string())
}

/// Sieht die Zeile wie ein Verzeichniseintrag aus (Aufzählung vorn oder
/// Seitenzahl hinten, nicht zu lang)?
pub(crate) fn toc_like(line: &str) -> bool {
    let t = line.trim();
    t.chars().count() <= 100
        && (t.starts_with(['•', '·', '▪', '◦'])
            || t.chars().next_back().is_some_and(|c| c.is_ascii_digit())
                && t.chars().any(char::is_alphabetic))
}

/// Inhaltsverzeichnis im ersten Drittel der Seiten: die Einträge und die
/// Stelle (Seite, Zeile) direkt dahinter, ab der der Text beginnt. Das
/// Verzeichnis reicht von der Seite mit der Überschrift (steht sie dort fast
/// allein, auch über die nächste Seite) über alle weiteren, auf denen
/// mindestens fünf Zeilen oder zwei Fünftel der Zeilen wie Einträge aussehen. Gibt es genug solche Zeilen,
/// zählen nur sie (Erläuterungen im Verzeichnis sind keine Einträge); eine
/// kurze Folgezeile ohne Zahl und Aufzählungszeichen setzt den Eintrag davor
/// fort („KHM 4. Märchen von einem, der auszog das“ / „Fürchten zu lernen“).
pub(crate) fn toc_entries(
    kept_pages: &[Vec<(&str, f32)>],
) -> Option<(Vec<String>, (usize, usize))> {
    let limit = (kept_pages.len() / 3).max(3).min(kept_pages.len());
    let (p0, i0) = (0..limit).find_map(|p| {
        kept_pages[p]
            .iter()
            .position(|(l, _)| is_toc_heading(l))
            .map(|i| (p, i))
    })?;
    let mut region: Vec<&str> = Vec::new();
    let mut end = (p0 + 1, 0);
    for (p, page) in kept_pages.iter().enumerate().skip(p0) {
        let from = if p == p0 { i0 + 1 } else { 0 };
        let lines = &page[from.min(page.len())..];
        let like = lines.iter().filter(|(l, _)| toc_like(l)).count();
        // Die Seite nach der Überschrift gehört dazu, wenn die Überschrift
        // fast allein auf ihrer Seite steht („INHALT“ als Zwischentitel).
        let heading_alone = kept_pages[p0].len() <= i0 + 3;
        let follows = p == p0 + 1 && heading_alone;
        if p > p0 && !follows && (lines.is_empty() || (like < 5 && like * 5 < lines.len() * 2)) {
            break;
        }
        region.extend(lines.iter().map(|(l, _)| *l));
        end = (p + 1, 0);
    }
    if end.0 >= kept_pages.len() {
        return None;
    }
    let only_like = region.iter().filter(|l| toc_like(l)).count() >= 5;
    let mut entries: Vec<String> = Vec::new();
    let mut prev_like = false;
    for (k, line) in region.iter().enumerate() {
        let like = toc_like(line);
        if only_like && !like {
            let next_like = region.get(k + 1).is_none_or(|n| toc_like(n));
            let continues = prev_like
                && next_like
                && line.trim().chars().count() <= 60
                && line.chars().any(char::is_alphabetic);
            if continues {
                if let Some(last) = entries.last_mut() {
                    *last = format!("{} {}", last, line.trim());
                }
            }
            prev_like = false;
            continue;
        }
        prev_like = like;
        if let Some(e) = toc_entry(line) {
            entries.push(e);
        }
    }
    // Doppelte Einträge (Verzeichnis über zwei Spalten, Kopf wiederholt) weg.
    let mut seen = std::collections::HashSet::new();
    entries.retain(|e| seen.insert(title_norm(e)));
    (entries.len() >= 3).then_some((entries, end))
}

/// Überschriften nach Schriftgröße: Zeilen deutlich größer als der
/// Fließtext (häufigste Größe, nach Zeichen gewichtet), kurz und mit
/// Buchstaben. Aufeinanderfolgende gleich große Zeilen sind eine Überschrift.
/// Die größte Stufe wird `#`, wenn sie höchstens zweimal und nur auf den
/// ersten Seiten vorkommt (Buchtitel); sonst beginnen die Stufen bei `##`. Eine Stufe,
/// die mehr als ein Fünftel aller Zeilen ausmacht, ist keine Überschrift.
pub(crate) fn size_headings(
    kept_pages: &[Vec<(&str, f32)>],
    flags: &mut [Vec<Option<(u8, usize)>>],
    group: &mut usize,
    typical: usize,
) {
    use std::collections::HashMap;
    let bucket = |s: f32| (s * 2.0).round() as i32;
    let mut weight: HashMap<i32, usize> = HashMap::new();
    let mut total_lines = 0usize;
    for (l, s) in kept_pages.iter().flatten() {
        if *s > 0.0 {
            *weight.entry(bucket(*s)).or_default() += l.chars().count();
            total_lines += 1;
        }
    }
    let Some((&body, _)) = weight.iter().max_by_key(|(b, w)| (**w, -**b)) else {
        return;
    };
    let body = body as f32 / 2.0;
    let candidate = |line: &str, size: f32| {
        let n = line.chars().count();
        size >= body * 1.18
            && size - body >= 1.0
            && n <= 90
            && line.split_whitespace().count() <= 14
            && line.chars().filter(|c| c.is_alphabetic()).count() >= 3
            && !line.trim_end().ends_with([',', ';'])
            && !is_page_number(line)
    };
    // Gruppen: (Seite, erste Zeile, Zeilenzahl, Stufe). Mehr als drei
    // gleich große Zeilen am Stück sind ein Textabschnitt, keine Überschrift.
    let mut groups: Vec<(usize, usize, usize, i32)> = Vec::new();
    for (p, kept) in kept_pages.iter().enumerate() {
        let mut i = 0;
        while i < kept.len() {
            if flags[p][i].is_some() || !candidate(kept[i].0, kept[i].1) {
                i += 1;
                continue;
            }
            let tier = bucket(kept[i].1);
            let mut n = 1;
            while i + n < kept.len()
                && flags[p][i + n].is_none()
                && bucket(kept[i + n].1) == tier
                && candidate(kept[i + n].0, kept[i + n].1)
            {
                n += 1;
            }
            if n <= 3 {
                groups.push((p, i, n, tier));
            }
            i += n;
        }
    }
    let mut tiers: Vec<i32> = groups.iter().map(|g| g.3).collect();
    tiers.sort_unstable_by(|a, b| b.cmp(a));
    tiers.dedup();
    let lines_of = |t: i32| -> usize { groups.iter().filter(|g| g.3 == t).map(|g| g.2).sum() };
    // Eine Stufe mit langen Zeilen (im Mittel über drei Viertel der üblichen
    // Zeilenlänge) ist hervorgehobener Text, keine Überschrift.
    let chars_of = |t: i32| -> usize {
        groups
            .iter()
            .filter(|g| g.3 == t)
            .flat_map(|g| &kept_pages[g.0][g.1..g.1 + g.2])
            .map(|(l, _)| {
                l.split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .count()
            })
            .sum()
    };
    tiers.retain(|&t| {
        lines_of(t) * 5 <= total_lines
            && (typical == 0 || chars_of(t) * 4 <= lines_of(t) * typical * 3)
    });
    if tiers.is_empty() {
        return;
    }
    let count_of = |t: i32| groups.iter().filter(|g| g.3 == t).count();
    // Die größte Stufe ist der Buchtitel, wenn sie höchstens zweimal vorkommt
    // und es darunter Kapitel gibt oder sonst gar keine Überschrift.
    // … und nur auf den ersten drei Seiten mit Text steht (Titelseite);
    // weiter hinten wäre ein `#` ein Abschnitt, der alles Folgende umfasst.
    let text_pages: Vec<usize> = (0..kept_pages.len())
        .filter(|&p| !kept_pages[p].is_empty())
        .take(3)
        .collect();
    let on_title_pages = groups
        .iter()
        .filter(|g| g.3 == tiers[0])
        .all(|g| text_pages.contains(&g.0));
    let first =
        if count_of(tiers[0]) <= 2 && on_title_pages && (tiers.len() > 1 || groups.len() <= 2) {
            1
        } else {
            2
        };
    for (p, i, n, tier) in groups {
        let Some(rank) = tiers.iter().position(|&t| t == tier) else {
            continue;
        };
        let level = (first + rank).min(3) as u8;
        *group += 1;
        for f in &mut flags[p][i..i + n] {
            *f = Some((level, *group));
        }
    }
}

/// Erste Zahl einer Seitenzahl-Zeile („Seite 12 von 300“ → 12).
pub(crate) fn first_number(line: &str) -> Option<u32> {
    line.split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
}
