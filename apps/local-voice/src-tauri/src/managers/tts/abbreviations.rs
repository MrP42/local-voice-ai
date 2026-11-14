//! Abkuerzungen fuer die SYNTHESE ausschreiben (deutsche Texte).
//!
//! Warum: Die Satztrennung schneidet seit PR #99 nicht mehr nach „ca.“, „bzw.“
//! oder „Dr.“ -- trotzdem macht ein Sprachmodell an jedem Punkt eine
//! Sprechpause, auch mitten im Satz (Patrick hoerte sie bei Phonon nach
//! „ca.“). Wer „ca.“ als „circa“ schreibt, hat keinen Punkt mehr, an dem das
//! Modell absetzen koennte, und die Engine liest ein echtes Wort statt
//! Buchstaben zu buchstabieren.
//!
//! Das gilt NUR fuer die Synthese: Anzeige, Satzgrenzen und Satzzaehlung
//! bleiben beim Originaltext (der Aufrufer ruft diese Funktion erst direkt
//! vor dem Cache-Schluessel und der Engine, siehe `TtsCore::spoken_text`).
//!
//! Zwei Regeln, in dieser Reihenfolge:
//!
//! 1. **Tabelle**: bekannte Abkuerzungen (auch mit Leerzeichen dazwischen:
//!    „z. B.“, „z.B.“, „Z.\u{a0}B.“) werden zum Wort. Die Gross-/Kleinschreibung
//!    folgt dem Original („Ca. 20“ -> „Circa 20“); Hauptwoerter („Nummer“,
//!    „Doktor“) bleiben gross. Manche Eintraege gelten nur vor einer Zahl oder
//!    einem Grossbuchstaben („Art. 5“, „Fr. Müller“).
//! 2. **Allgemein**: ein Punkt direkt nach einem Wort entfaellt, wenn danach ein
//!    klein geschriebenes Wort folgt (so beginnt kein deutscher Satz), oder wenn
//!    eine bekannte Abkuerzung ohne Tabelleneintrag vor einer Zahl steht
//!    („min. 5“). Ein Punkt nach einer Ziffer bleibt IMMER: „am 24. Dezember“ ist
//!    eine Ordnungszahl, ohne Punkt las die Engine „vierundzwanzig“.
//!
//! Nie angefasst werden Stil-Tags `[…]` und Sprechermarker `<…>`.
//!
//! Endet ein Satz mit einer Abkuerzung („…, Birnen usw.“), bleibt ein Punkt
//! stehen, damit das Modell den Satz abschliesst.
//!
//! Eingesetzte Aussprachen eines Assistenten (`aussprache`, [`Schutz`]) bleiben aus der GANZEN
//! Aufbereitung heraus: aus Datum, Lexikon, Tabelle und Punktregeln. Der Text neben einem
//! geschuetzten Stueck wird wie jeder andere behandelt und sieht das Stueck dabei (ein Punkt vor
//! einem geschuetzten Wort ist kein Satzende am Textende, ein Datum danach ist ein Datum); keine
//! Regel greift aber in ein geschuetztes Zeichen oder ueber eines hinweg.

use std::sync::OnceLock;

use super::aussprache::Schutz;
use super::{lang, protocol};

/// Was nach einer Abkuerzung folgen muss, damit der Eintrag gilt.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Next {
    /// Egal (die meisten Abkuerzungen).
    Any,
    /// Eine Ziffer („Art. 5“, „Abb. 3“, „geb. 1950“).
    Digit,
    /// Ein Grossbuchstabe („Fr. Müller“, „geb. Müller“) -- sonst ist „Fr.“
    /// vermutlich der Freitag.
    Capital,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    /// Kleinbuchstaben und Punkte, ohne Leerzeichen („z.b.“). Zwischen den
    /// Teilen darf im Text ein Leerzeichen stehen.
    pattern: &'static str,
    expansion: &'static str,
    /// Form nach der Zahl 1 („1 Mio.“ -> „1 Million“).
    singular: Option<&'static str>,
    next: Next,
    /// Beschliesst diese Abkuerzung fast immer einen Satz („… usw.“)? Dann bleibt
    /// vor einem Grossbuchstaben oder einer Zahl ein Punkt stehen. Nur fuer
    /// Abkuerzungen, nach denen KEIN Hauptwort folgt: „u. a. Kuchen“ ist ein
    /// Satz mitten drin, und ein Grossbuchstabe verraet im Deutschen keinen
    /// Satzanfang.
    may_end: bool,
}

const fn any(pattern: &'static str, expansion: &'static str) -> Entry {
    Entry {
        pattern,
        expansion,
        singular: None,
        next: Next::Any,
        may_end: false,
    }
}

const fn ends(pattern: &'static str, expansion: &'static str) -> Entry {
    Entry {
        may_end: true,
        ..any(pattern, expansion)
    }
}

const fn before(next: Next, pattern: &'static str, expansion: &'static str) -> Entry {
    Entry {
        next,
        ..any(pattern, expansion)
    }
}

const fn counted(pattern: &'static str, plural: &'static str, singular: &'static str) -> Entry {
    Entry {
        singular: Some(singular),
        ..any(pattern, plural)
    }
}

/// Die Tabelle. Hauptwoerter stehen gross, alles andere klein.
const ENTRIES: &[Entry] = &[
    // Fuellwoerter und Verweise
    any("ca.", "circa"),
    any("bzw.", "beziehungsweise"),
    any("ggf.", "gegebenenfalls"),
    any("evtl.", "eventuell"),
    any("inkl.", "inklusive"),
    any("exkl.", "exklusive"),
    any("zzgl.", "zuzüglich"),
    any("abzgl.", "abzüglich"),
    any("vgl.", "vergleiche"),
    any("bspw.", "beispielsweise"),
    any("sog.", "sogenannt"),
    any("insb.", "insbesondere"),
    any("allg.", "allgemein"),
    any("gem.", "gemäß"),
    any("lt.", "laut"),
    any("od.", "oder"),
    any("jew.", "jeweils"),
    any("mind.", "mindestens"),
    any("max.", "maximal"),
    any("betr.", "betreffend"),
    any("ggü.", "gegenüber"),
    any("vs.", "versus"),
    // Mehrteilige Abkuerzungen
    any("z.b.", "zum Beispiel"),
    any("d.h.", "das heißt"),
    any("u.u.", "unter Umständen"),
    any("i.d.r.", "in der Regel"),
    any("v.a.", "vor allem"),
    any("z.t.", "zum Teil"),
    any("u.a.", "unter anderem"),
    any("o.ä.", "oder Ähnliches"),
    any("s.o.", "siehe oben"),
    any("s.u.", "siehe unten"),
    // Beschliessen Aufzaehlungen: danach beginnt fast immer ein neuer Satz.
    ends("u.ä.", "und Ähnliches"),
    ends("u.v.m.", "und vieles mehr"),
    ends("usw.", "und so weiter"),
    ends("etc.", "et cetera"),
    // Titel und Anreden
    any("dr.-ing.", "Doktor-Ingenieur"),
    any("dipl.-ing.", "Diplom-Ingenieur"),
    any("dr.", "Doktor"),
    any("prof.", "Professor"),
    before(Next::Capital, "hr.", "Herr"),
    before(Next::Capital, "fr.", "Frau"),
    // Verweise auf Stellen im Text
    any("nr.", "Nummer"),
    any("abs.", "Absatz"),
    before(Next::Digit, "art.", "Artikel"),
    before(Next::Digit, "kap.", "Kapitel"),
    before(Next::Digit, "abb.", "Abbildung"),
    before(Next::Digit, "tab.", "Tabelle"),
    before(Next::Digit, "bd.", "Band"),
    any("anm.", "Anmerkung"),
    any("hrsg.", "Herausgeber"),
    any("aufl.", "Auflage"),
    any("jg.", "Jahrgang"),
    // Angaben
    before(Next::Digit, "geb.", "geboren"),
    before(Next::Capital, "geb.", "geborene"),
    any("tel.", "Telefon"),
    any("str.", "Straße"),
    any("mwst.", "Mehrwertsteuer"),
    any("jh.", "Jahrhundert"),
    any("jhd.", "Jahrhundert"),
    any("tsd.", "Tausend"),
    counted("mio.", "Millionen", "Million"),
    counted("mrd.", "Milliarden", "Milliarde"),
    counted("std.", "Stunden", "Stunde"),
    counted("sek.", "Sekunden", "Sekunde"),
];

/// Die Muster als Zeichenlisten, laengste zuerst: bei gleichem Anfang gewinnt
/// die genauere Abkuerzung („dr.-ing.“ vor „dr.“).
fn table() -> &'static [(Vec<char>, Entry)] {
    static TABLE: OnceLock<Vec<(Vec<char>, Entry)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut all: Vec<(Vec<char>, Entry)> = ENTRIES
            .iter()
            .map(|entry| (entry.pattern.chars().collect(), *entry))
            .collect();
        all.sort_by_key(|(pattern, _)| std::cmp::Reverse(pattern.len()));
        all
    })
}

/// Der Text als deutsch gelesen? Unentschieden gilt als deutsch -- dieselbe
/// Voreinstellung wie bei der Anbieter-Pipeline (`unwrap_or("de")`), denn
/// kurze deutsche Saetze haben oft zu wenige Stoppwoerter fuer eine sichere
/// Erkennung.
pub fn is_german(text: &str) -> bool {
    // Stil-Tags sind englisch („calm and quiet“) und duerfen die Erkennung nicht kippen.
    let plain;
    let sample = if text.contains('[') {
        plain = protocol::strip_tag_spans(text);
        plain.as_str()
    } else {
        text
    };
    lang::detect_language(sample).unwrap_or("de") == "de"
}

/// Wie [`expand_for_synthesis`], aber nur bei deutschem Text (ohne Schutzbereiche; die Sprachausgabe
/// geht über [`expand_if_german_geschuetzt`]).
#[cfg(test)]
pub fn expand_if_german(text: &str) -> String {
    expand_if_german_geschuetzt(text, &Schutz::leer())
}

/// Wie [`expand_if_german`], mit Schutzbereichen. Die Sprache wird EINMAL am ganzen Text erkannt
/// (auch eingesetzte Aussprachen gehoeren zu dem, was die Engine liest).
pub fn expand_if_german_geschuetzt(text: &str, schutz: &Schutz) -> String {
    expand_if_german_mit_schutz(text, schutz).0
}

/// Wie [`expand_if_german_geschuetzt`], mit den Bereichen der geschuetzten Stuecke im Ergebnis
/// (fuer Pruefungen: sie stehen dort unveraendert). Stil-Tags und Sprechermarker zaehlen nicht dazu,
/// auch wenn ein eingesetztes Stueck in ihnen lag: die Marken haben Vorrang. Ist der Text nicht
/// deutsch, bleibt er, wie er ist, und mit ihm der Schutz.
pub fn expand_if_german_mit_schutz(text: &str, schutz: &Schutz) -> (String, Schutz) {
    if is_german(text) {
        expand_mit_schutz(text, schutz)
    } else {
        (text.to_string(), schutz.clone())
    }
}

/// Abkuerzungen ausschreiben (Sprache wird vorausgesetzt, siehe
/// [`expand_if_german`]). Tags `[…]` und Sprechermarker `<…>` bleiben
/// unberuehrt. Vorher laufen Datum und Lexikon (`pronunciation`); ohne diese und ohne
/// Punkt kommt der Text unveraendert zurueck.
#[cfg(test)]
pub fn expand_for_synthesis(text: &str) -> String {
    expand_mit_schutz(text, &Schutz::leer()).0
}

/// Wie [`expand_for_synthesis`], mit den Bereichen der geschützten Stücke im Ergebnis. Die Segmente
/// zwischen den Marken laufen einzeln durch Datum, Lexikon, Abkürzungstabelle und Punktregeln; die
/// geschützten Stücke eines Segments sind dabei Löcher im echten Kontext, keine Segmentgrenzen.
fn expand_mit_schutz(text: &str, schutz: &Schutz) -> (String, Schutz) {
    let mut out = String::with_capacity(text.len() + 16);
    let mut geschuetzt: Vec<std::ops::Range<usize>> = Vec::new();
    let mut last = 0usize;
    let mut segment = |out: &mut String, von: usize, bis: usize| {
        let (neu, bleibt) = expand_segment(&text[von..bis], &schutz.ausschnitt(von, bis));
        geschuetzt.extend(
            bleibt
                .bereiche()
                .iter()
                .map(|b| out.len() + b.start..out.len() + b.end),
        );
        out.push_str(&neu);
    };
    for span in protected_spans(text) {
        segment(&mut out, last, span.start);
        out.push_str(&text[span.clone()]);
        last = span.end;
    }
    segment(&mut out, last, text.len());
    (out, Schutz::aus_bereichen(geschuetzt))
}

/// Byte-Bereiche, die nie veraendert werden: Stil-Tags und Sprechermarker,
/// aufsteigend und ohne Ueberlappung.
fn protected_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = protocol::tag_spans(text);
    for candidate in protocol::scan_marker_candidates(text) {
        // „a < b und b > c“ ist ein Vergleich, kein Marker (wie in `strip_speaker_markers`).
        let inner = &text[candidate.start + 1..candidate.end - 1];
        if inner.starts_with(char::is_whitespace) || inner.ends_with(char::is_whitespace) {
            continue;
        }
        spans.push(candidate.start..candidate.end);
    }
    spans.sort_by_key(|span| span.start);
    // Ueberlappendes zusammenfassen (ein Marker innerhalb eines Tags).
    let mut merged: Vec<std::ops::Range<usize>> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(prev) if span.start < prev.end => prev.end = prev.end.max(span.end),
            _ => merged.push(span),
        }
    }
    merged
}

/// Ein Stueck ohne Tags: erst Datum und Lexikon, dann die Tabelle, dann die allgemeine
/// Punktregel. Geschuetzte Zeichen (`schutz`, bezogen auf das Stueck) bleiben unberuehrt; das
/// Ergebnis meldet, wo sie danach stehen.
fn expand_segment(segment: &str, schutz: &Schutz) -> (String, Schutz) {
    let (segment, schutz) = super::pronunciation::apply_geschuetzt(segment, schutz);
    let segment = segment.as_str();
    if !segment.contains('.') {
        return (segment.to_string(), schutz);
    }
    let chars: Vec<char> = segment.chars().collect();
    let fest = schutz.maske(segment);
    let mut out = String::with_capacity(segment.len() + 16);
    let mut out_fest: Vec<bool> = Vec::with_capacity(chars.len() + 16);
    let mut i = 0usize;
    while i < chars.len() {
        if !fest[i] && at_word_start(&chars, i) {
            if let Some((end, replacement)) = expand_at(&chars, &fest, i) {
                out.push_str(&replacement);
                out_fest.extend(replacement.chars().map(|_| false));
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        out_fest.push(fest[i]);
        i += 1;
    }
    drop_continuation_periods(&out, &out_fest)
}

fn at_word_start(chars: &[char], i: usize) -> bool {
    chars[i].is_alphabetic() && (i == 0 || !chars[i - 1].is_alphanumeric())
}

/// Passt an Stelle `i` ein Tabelleneintrag? Rueckgabe: Ende im Text und der
/// Ersatz (samt Schlusspunkt, wenn der Satz hier endet).
fn expand_at(chars: &[char], fest: &[bool], i: usize) -> Option<(usize, String)> {
    for (pattern, entry) in table() {
        let Some(end) = match_pattern(chars, i, pattern) else {
            continue;
        };
        // Ein geschuetztes Zeichen gehoert nie zu einer Abkuerzung (ein kuerzerer Eintrag kann
        // trotzdem noch davor passen).
        if fest[i..end].iter().any(|&f| f) {
            continue;
        }
        // Ende am Wortrand: „ca.20“ oder „Dr.Müller“ sind keine Abkuerzungen,
        // sondern Tippfehler -- lieber stehen lassen als raten.
        if chars.get(end).is_some_and(|c| c.is_alphanumeric()) {
            continue;
        }
        let following = chars[end..].iter().copied().find(|c| !c.is_whitespace());
        let fits = match entry.next {
            Next::Any => true,
            Next::Digit => following.is_some_and(|c| c.is_ascii_digit()),
            Next::Capital => following.is_some_and(char::is_uppercase),
        };
        if !fits {
            continue;
        }
        let mut word = if entry.singular.is_some() && follows_one(chars, i) {
            entry.singular.unwrap_or(entry.expansion).to_string()
        } else {
            entry.expansion.to_string()
        };
        if chars[i].is_uppercase() {
            word = capitalize_first(&word);
        }
        // Der Satz endet hier: am Textende immer, sonst nur bei Abkuerzungen, die
        // einen Satz beschliessen koennen, und nur vor Gross-/Zahl/Anfuehrung.
        let ends_sentence = match following {
            None => true,
            Some(c) => {
                entry.may_end
                    && (c.is_uppercase()
                        || c.is_ascii_digit()
                        || matches!(c, '"' | '„' | '“' | '»' | '(' | '\''))
            }
        };
        if ends_sentence {
            word.push('.');
        }
        return Some((end, word));
    }
    None
}

/// Das Muster ab `start` abgleichen: Buchstaben ohne Beachtung der Schreibweise,
/// nach jedem Punkt VOR einem weiteren Teil darf ein Leerzeichen stehen (auch ein
/// geschuetztes oder schmales, wie es der deutsche Satz bei „z. B.“ setzt).
fn match_pattern(chars: &[char], start: usize, pattern: &[char]) -> Option<usize> {
    let mut at = start;
    for (index, &expected) in pattern.iter().enumerate() {
        let c = *chars.get(at)?;
        if !c.to_lowercase().eq(expected.to_lowercase()) {
            return None;
        }
        at += 1;
        if expected == '.'
            && index + 1 < pattern.len()
            && chars
                .get(at)
                .is_some_and(|c| c.is_whitespace() && *c != '\n' && *c != '\r')
        {
            at += 1;
        }
    }
    Some(at)
}

/// Steht vor der Abkuerzung die Zahl 1 („1 Mio.“)?
fn follows_one(chars: &[char], i: usize) -> bool {
    let mut at = i;
    while at > 0 && chars[at - 1].is_whitespace() {
        at -= 1;
    }
    let end = at;
    while at > 0 && !chars[at - 1].is_whitespace() {
        at -= 1;
    }
    let token: String = chars[at..end].iter().collect();
    matches!(token.as_str(), "1" | "eine" | "einer" | "einem")
}

fn capitalize_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Allgemeine Regel: ein Punkt nach einem Wort entfaellt, wenn ein klein
/// geschriebenes Wort folgt, oder wenn eine bekannte Abkuerzung vor einer Zahl
/// steht. Nie nach Ziffern (Ordnungszahlen), nie in Auslassungspunkten, nie ueber
/// einen Zeilenumbruch hinweg.
fn drop_continuation_periods(text: &str, fest: &[bool]) -> (String, Schutz) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut out_fest: Vec<bool> = Vec::with_capacity(chars.len());
    for (k, &c) in chars.iter().enumerate() {
        // Ein geschuetzter Punkt faellt nie weg.
        if c == '.' && !fest[k] && is_continuation_period(&chars, k) {
            continue;
        }
        out.push(c);
        out_fest.push(fest[k]);
    }
    let schutz = Schutz::aus_maske(&out, &out_fest);
    (out, schutz)
}

fn is_horizontal_space(c: char) -> bool {
    c.is_whitespace() && c != '\n' && c != '\r'
}

fn is_continuation_period(chars: &[char], k: usize) -> bool {
    let Some(&prev) = k.checked_sub(1).and_then(|p| chars.get(p)) else {
        return false;
    };
    if !prev.is_alphabetic() {
        return false;
    }
    if !chars.get(k + 1).is_some_and(|c| is_horizontal_space(*c)) {
        return false;
    }
    let mut at = k + 1;
    while chars.get(at).is_some_and(|c| is_horizontal_space(*c)) {
        at += 1;
    }
    let Some(&next) = chars.get(at) else {
        return false;
    };
    if next.is_lowercase() {
        return true;
    }
    if next.is_ascii_digit() {
        let word: String = chars[..k]
            .iter()
            .rev()
            .take_while(|c| c.is_alphabetic())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return protocol::is_known_abbreviation(&word);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(text: &str) -> String {
        expand_for_synthesis(text)
    }

    /// Die Fälle aus dem Auftrag: jede genannte Abkürzung wird zum Wort.
    #[test]
    fn die_gaengigen_abkuerzungen_werden_ausgeschrieben() {
        let table = [
            ("Das kostet ca. 20 Euro.", "Das kostet circa 20 Euro."),
            ("Er kommt, z. B. morgen.", "Er kommt, zum Beispiel morgen."),
            ("Rot bzw. blau.", "Rot beziehungsweise blau."),
            ("Äpfel usw. und Birnen.", "Äpfel und so weiter und Birnen."),
            (
                "Das heißt, d. h. das Ende.",
                "Das heißt, das heißt das Ende.",
            ),
            ("Es gibt u. a. Kuchen.", "Es gibt unter anderem Kuchen."),
            ("Ggf. mit Sahne.", "Gegebenenfalls mit Sahne."),
            ("Es gibt evtl. Regen.", "Es gibt eventuell Regen."),
            ("Preis inkl. Versand.", "Preis inklusive Versand."),
            ("Preis zzgl. Versand.", "Preis zuzüglich Versand."),
            ("Siehe vgl. Kapitel.", "Siehe vergleiche Kapitel."),
            (
                "Bestellung Nr. 4711 ist da.",
                "Bestellung Nummer 4711 ist da.",
            ),
            ("Dr. Müller kommt.", "Doktor Müller kommt."),
            ("Prof. Meier kommt.", "Professor Meier kommt."),
            ("Siehe Abs. 3.", "Siehe Absatz 3."),
            (
                "Das ist bspw. ein Test.",
                "Das ist beispielsweise ein Test.",
            ),
            ("Die sog. Experten irren.", "Die sogenannt Experten irren."),
            ("Mehr s. o. und s. u.", "Mehr siehe oben und siehe unten."),
            ("Das geht i. d. R. gut.", "Das geht in der Regel gut."),
            ("Es klappt u. U. nicht.", "Es klappt unter Umständen nicht."),
            ("Obst u. Ä. liegt da.", "Obst und Ähnliches liegt da."),
            (
                "Es gibt Kaffee o. ä. hier.",
                "Es gibt Kaffee oder Ähnliches hier.",
            ),
            (
                "Hr. Müller und Fr. Schmidt.",
                "Herr Müller und Frau Schmidt.",
            ),
            ("Tel. 0151 123456.", "Telefon 0151 123456."),
            ("Hauptstadt Str. 5.", "Hauptstadt Straße 5."),
            ("Im 19. Jh. war das so.", "Im 19. Jahrhundert war das so."),
            ("Dipl.-Ing. Wolff.", "Diplom-Ingenieur Wolff."),
            ("Dr.-Ing. Wolff.", "Doktor-Ingenieur Wolff."),
        ];
        for (input, expected) in table {
            assert_eq!(expand(input), expected, "Eingabe: {input}");
        }
    }

    /// „Ca. 20“ am Satzanfang bleibt groß; Hauptwörter bleiben auch klein
    /// geschrieben groß.
    #[test]
    fn die_grossschreibung_folgt_dem_original() {
        assert_eq!(expand("Ca. 20 Leute kamen."), "Circa 20 Leute kamen.");
        assert_eq!(expand("Z. B. Äpfel."), "Zum Beispiel Äpfel.");
        assert_eq!(expand("D. h. nein."), "Das heißt nein.");
        assert_eq!(expand("Bzw. beides."), "Beziehungsweise beides.");
        assert_eq!(expand("Siehe nr. 5."), "Siehe Nummer 5.");
        assert_eq!(expand("Der dr. Müller."), "Der Doktor Müller.");
    }

    /// Schreibvarianten: ohne Leerzeichen, mit geschütztem und schmalem
    /// Leerzeichen, Großbuchstaben.
    #[test]
    fn schreibvarianten_von_mehrteiligen_abkuerzungen() {
        for variant in [
            "z.B.",
            "z. B.",
            "z.\u{a0}B.",
            "z.\u{202f}B.",
            "Z.B.",
            "z. b.",
        ] {
            let text = format!("Es gibt {variant} Kuchen.");
            let out = expand(&text);
            assert!(
                out == "Es gibt zum Beispiel Kuchen." || out == "Es gibt Zum Beispiel Kuchen.",
                "{variant} -> {out}"
            );
            assert!(!out.contains('.') || out.ends_with("Kuchen."), "{out}");
        }
        assert_eq!(expand("i.d.R. gut"), "in der Regel gut");
    }

    /// Ordnungszahlen behalten ihren Punkt -- ohne ihn läse die Engine
    /// „vierundzwanzig“ statt „vierundzwanzigsten“.
    #[test]
    fn ordnungszahlen_behalten_ihren_punkt() {
        for text in [
            "Am 24. Dezember kommt er.",
            "Am 24. dezember kommt er.",
            "Der 3. oder 4. Platz.",
            "Im 3. Quartal 2026.",
            "Platz 1. und Platz 2.",
        ] {
            assert_eq!(expand(text), text);
        }
    }

    /// Ohne Eintrag, aber vor einem kleingeschriebenen Wort: der Punkt entfällt.
    #[test]
    fn ein_punkt_vor_einem_kleinen_wort_faellt_weg() {
        assert_eq!(
            expand("Er kam am Mo. und ging am Di. wieder."),
            "Er kam am Mo und ging am Di wieder."
        );
        assert_eq!(
            expand("Das gilt allen. aber nicht uns."),
            "Das gilt allen aber nicht uns."
        );
        // Eine bekannte Abkürzung vor einer Zahl.
        assert_eq!(expand("Dauer min. 5 Minuten."), "Dauer min 5 Minuten.");
    }

    /// Echte Satzenden bleiben: Großbuchstabe oder Ende folgt.
    #[test]
    fn echte_satzenden_bleiben_stehen() {
        for text in [
            "Das war gut. Dann ging er.",
            "Das war gut.",
            "Wirklich? Ja. Nein.",
            "Er sagte nichts. 5 Leute lachten.",
            "Wartet … und dann. Schluss.",
        ] {
            assert_eq!(expand(text), text, "{text}");
        }
    }

    /// Endet der Satz mit einer Abkürzung, bleibt ein Punkt, damit das Modell
    /// ihn abschließt.
    #[test]
    fn endet_ein_satz_mit_einer_abkuerzung_bleibt_ein_punkt() {
        assert_eq!(
            expand("Wir kaufen Äpfel, Birnen usw."),
            "Wir kaufen Äpfel, Birnen und so weiter."
        );
        assert_eq!(
            expand("Wir kaufen Äpfel usw. Dann gehen wir."),
            "Wir kaufen Äpfel und so weiter. Dann gehen wir."
        );
        assert_eq!(expand("Das wird z. B."), "Das wird zum Beispiel.");
        // Eine Abkürzung ohne Satzende-Möglichkeit verliert den Punkt vor Großbuchstaben.
        assert_eq!(expand("Das sagte Dr. Müller."), "Das sagte Doktor Müller.");
        assert_eq!(expand("Das ist ca. 20 Meter."), "Das ist circa 20 Meter.");
    }

    /// Nach der Zahl 1 steht die Einzahl.
    #[test]
    fn einzahl_nach_der_zahl_eins() {
        assert_eq!(
            expand("Das kostet 1 Mio. Euro."),
            "Das kostet 1 Million Euro."
        );
        assert_eq!(
            expand("Das kostet 5 Mio. Euro."),
            "Das kostet 5 Millionen Euro."
        );
        assert_eq!(
            expand("Es sind 3,5 Mrd. Euro."),
            "Es sind 3,5 Milliarden Euro."
        );
        assert_eq!(expand("Es dauert 1 Std. lang."), "Es dauert 1 Stunde lang.");
        assert_eq!(
            expand("Es dauert 2 Std. lang."),
            "Es dauert 2 Stunden lang."
        );
    }

    /// Manche Einträge gelten nur im passenden Zusammenhang.
    #[test]
    fn eintraege_mit_bedingung_gelten_nur_im_passenden_zusammenhang() {
        assert_eq!(expand("Siehe Art. 5 GG."), "Siehe Artikel 5 GG.");
        assert_eq!(
            expand("Art. und Weise gehören dazu."),
            "Art und Weise gehören dazu."
        );
        assert_eq!(expand("Frau Fr. Müller."), "Frau Frau Müller.");
        // Freitag, nicht Frau: kein Großbuchstabe danach.
        assert_eq!(expand("Mo. bis Fr. ab 10 Uhr."), "Mo bis Fr ab 10 Uhr.");
        assert_eq!(expand("Maria, geb. Müller."), "Maria, geborene Müller.");
        assert_eq!(expand("Max, geb. 1950."), "Max, geboren 1950.");
        assert_eq!(expand("Siehe Abb. 3."), "Siehe Abbildung 3.");
    }

    /// Stil-Tags und Sprechermarker bleiben, wie sie sind.
    #[test]
    fn tags_und_sprechermarker_bleiben_unveraendert() {
        assert_eq!(
            expand("[laughing] Das kostet ca. 5 Euro."),
            "[laughing] Das kostet circa 5 Euro."
        );
        assert_eq!(
            expand("[dead tired, ca. 5 sec. pause.] Er kam."),
            "[dead tired, ca. 5 sec. pause.] Er kam."
        );
        assert_eq!(
            expand("<Dr. Anna:ruhig> Hallo ca. zehn."),
            "<Dr. Anna:ruhig> Hallo circa zehn."
        );
        assert_eq!(expand("a < ca. b und b > c"), "a < circa b und b > c");
        // Ein Tag mitten im Satz teilt den Text, nicht die Abkürzung.
        assert_eq!(
            expand("Es gibt [pause] usw. mehr."),
            "Es gibt [pause] und so weiter mehr."
        );
    }

    /// Wörter, die nur zufällig auf eine Abkürzung enden, und Tippfehler
    /// bleiben wie sie sind.
    #[test]
    fn nur_ganze_abkuerzungen_werden_ersetzt() {
        for text in [
            "Erica. Hallo.",
            "Das Formular.pdf liegt da.",
            "Besuch www.example.com heute.",
            "Er sagte ca.20 Leute.",
            "Mitarbeiter.Wolff meldete sich.",
            "Version 1.2.3 ist da.",
            "Das kostet 3.5 Euro.",
        ] {
            assert_eq!(expand(text), text, "{text}");
        }
    }

    /// Zeilenumbrüche bleiben und trennen: ein Punkt vor einer klein
    /// geschriebenen Zeile fällt nicht weg.
    #[test]
    fn zeilenumbrueche_bleiben_erhalten() {
        assert_eq!(
            expand("Erster Punkt.\nzweiter Punkt ca. 5."),
            "Erster Punkt.\nzweiter Punkt circa 5."
        );
    }

    #[test]
    fn leer_und_ohne_punkt_bleibt_unveraendert() {
        assert_eq!(expand(""), "");
        assert_eq!(expand("   "), "   ");
        assert_eq!(expand("Kein Punkt hier"), "Kein Punkt hier");
        assert_eq!(expand("."), ".");
        assert_eq!(expand("ca."), "circa.");
    }

    /// Die Sprachprüfung: nur deutscher (oder unentschiedener) Text wird
    /// angefasst, englischer nicht -- „Dr.“ hieße dort „Doctor“.
    #[test]
    fn nur_deutscher_text_wird_ausgeschrieben() {
        assert_eq!(
            expand_if_german("Dr. Müller kommt ca. um zehn Uhr und ist nicht da."),
            "Doktor Müller kommt circa um zehn Uhr und ist nicht da."
        );
        let english = "Dr. Smith said that the report is due, and it was a quiet evening.";
        assert!(!is_german(english));
        assert_eq!(expand_if_german(english), english);
        // Unentschieden (kein Stoppwort) zählt als deutsch.
        assert_eq!(
            expand_if_german("Ca. 20 Mitarbeiter."),
            "Circa 20 Mitarbeiter."
        );
        // Englische Stil-Tags kippen die Erkennung nicht.
        assert_eq!(
            expand_if_german("[with a calm and quiet voice, and the] Das ist ca. fünf."),
            "[with a calm and quiet voice, and the] Das ist circa fünf."
        );
    }

    #[test]
    fn jeder_tabelleneintrag_ist_wohlgeformt() {
        for entry in ENTRIES {
            assert!(entry.pattern.ends_with('.'), "{}", entry.pattern);
            assert_eq!(
                entry.pattern,
                entry.pattern.to_lowercase(),
                "{}",
                entry.pattern
            );
            assert!(!entry.pattern.contains(' '), "{}", entry.pattern);
            assert!(!entry.expansion.is_empty() && !entry.expansion.ends_with('.'));
        }
        // Gleiches Muster nur mit verschiedener Bedingung.
        let mut seen = std::collections::HashSet::new();
        for entry in ENTRIES {
            assert!(
                seen.insert((entry.pattern, entry.next)),
                "doppelt: {} {:?}",
                entry.pattern,
                entry.next
            );
        }
    }

    // ------------------------------------------------------------------
    // Eingesetzte Aussprachen (lva#556): bleiben aus der GANZEN Aufbereitung heraus
    // ------------------------------------------------------------------

    use super::super::aussprache::{aus_paaren, stuecke};

    /// Ausschreiben mit Schutz, unabhängig von der Sprache (die Erkennung hat eigene Tests).
    fn mit(teile: &[(&str, bool)]) -> (String, Schutz) {
        let (text, schutz) = aus_paaren(teile);
        expand_mit_schutz(&text, &schutz)
    }

    #[test]
    fn beurlaubung_ungeschuetzt_wird_getrennt_geschuetzt_nicht() {
        // Ohne Regel liest das Lexikon „Beurlaubung“ silbenweise getrennt.
        assert_eq!(expand("Die Beurlaubung läuft."), "Die Be-urlaubung läuft.");
        // Als eingesetzte Aussprache bleibt das Wort, wie die Regel es schreibt.
        let (aus, schutz) = mit(&[("Die ", false), ("Beurlaubung", true), (" läuft.", false)]);
        assert_eq!(aus, "Die Beurlaubung läuft.");
        assert_eq!(stuecke(&aus, &schutz), vec!["Beurlaubung"]);
        // Daneben wirkt das Lexikon weiter, und die Bereiche wandern mit der Verlängerung des Textes.
        let (aus, schutz) = mit(&[("Beurlaubung und ", false), ("Beurlaubung", true)]);
        assert_eq!(aus, "Be-urlaubung und Beurlaubung");
        assert_eq!(stuecke(&aus, &schutz), vec!["Beurlaubung"]);
        // Ein Treffer, der ein geschütztes Stück nur berührt, gilt nicht.
        let (aus, schutz) = mit(&[("Be", false), ("urlaub", true), ("ung", false)]);
        assert_eq!(aus, "Beurlaubung");
        assert_eq!(stuecke(&aus, &schutz), vec!["urlaub"]);
    }

    #[test]
    fn unersetzte_nachbarstuecke_bekommen_keine_kuenstliche_lexikongrenze() {
        // Zwei Stücke ohne Ersetzung, die an einem Wort grenzen, sind ein Text: das Lexikon sieht „Beurlaubung“.
        let (aus, schutz) = mit(&[("Beur", false), ("laubung", false)]);
        assert_eq!(aus, "Be-urlaubung");
        assert!(schutz.ist_leer());
        // Ein geschütztes Stück ist dagegen eine Grenze des Wortes: dahinter beginnt kein neues „Be“.
        let (aus, _) = mit(&[("Be", false), ("X", true), ("urlaub", false)]);
        assert_eq!(aus, "BeXurlaub");
    }

    #[test]
    fn abkuerzungen_ungeschuetzt_ausgeschrieben_geschuetzt_nicht() {
        assert_eq!(
            expand("Es gibt z. B. Kuchen, ca. 5."),
            "Es gibt zum Beispiel Kuchen, circa 5."
        );
        let (aus, schutz) = mit(&[
            ("Es gibt ", false),
            ("z. B.", true),
            (" Kuchen, ca. 5.", false),
        ]);
        assert_eq!(aus, "Es gibt z. B. Kuchen, circa 5.");
        assert_eq!(stuecke(&aus, &schutz), vec!["z. B."]);
        // Nur ein Teil der Abkürzung ist eingesetzt: sie gilt nicht, die Punkte bleiben.
        let (aus, _) = mit(&[("Es gibt z. ", false), ("B.", true), (" Kuchen", false)]);
        assert_eq!(aus, "Es gibt z. B. Kuchen");
        // Ein kürzerer Eintrag davor gilt trotzdem: „Dr.“ vor einem geschützten „-Ing.“.
        let (aus, schutz) = mit(&[("Dr.", false), ("-Ing.", true), (" Wolff", false)]);
        assert_eq!(aus, "Doktor-Ing. Wolff");
        assert_eq!(stuecke(&aus, &schutz), vec!["-Ing."]);
        // Zeilenwechsel, Mehrfachtreffer: jede ungeschützte Abkürzung wird ausgeschrieben.
        let (aus, schutz) = mit(&[
            ("ca. 5 und ", false),
            ("ca. 6", true),
            (" und bzw. 7", false),
        ]);
        assert_eq!(aus, "circa 5 und ca. 6 und beziehungsweise 7");
        assert_eq!(stuecke(&aus, &schutz), vec!["ca. 6"]);
    }

    #[test]
    fn datum_ungeschuetzt_ausgeschrieben_geschuetzt_nicht() {
        assert_eq!(expand("Bis 31.5. ist Zeit."), "Bis 31. Mai ist Zeit.");
        let (aus, schutz) = mit(&[("Bis ", false), ("31.5.", true), (" ist Zeit.", false)]);
        assert_eq!(aus, "Bis 31.5. ist Zeit.");
        assert_eq!(stuecke(&aus, &schutz), vec!["31.5."]);
        // Nur ein Teil des Datums ist eingesetzt: kein Datum.
        let (aus, _) = mit(&[("Bis 31.", false), ("5.", true), (" ist Zeit.", false)]);
        assert_eq!(aus, "Bis 31.5. ist Zeit.");
        // Ein Datum vor einem geschützten Stück, mit verschobenen Bereichen.
        let (aus, schutz) = mit(&[("Am 1.12. und ", false), ("SQL", true)]);
        assert_eq!(aus, "Am 1. Dezember und SQL");
        assert_eq!(stuecke(&aus, &schutz), vec!["SQL"]);
    }

    #[test]
    fn das_datum_sieht_die_umgebung_ein_geschuetztes_wort_davor_ist_kein_zeilenanfang() {
        // Nach einem geschützten Wort ist „31.5.“ ein Datum (ein Zeilenanfang wäre eine Nummerierung).
        let (aus, schutz) = mit(&[("Frist", true), (" 31.5. endet", false)]);
        assert_eq!(aus, "Frist 31. Mai endet");
        assert_eq!(stuecke(&aus, &schutz), vec!["Frist"]);
        // Die Umgebung ist, was die Engine hört: nach einem geschützten „Punkt“ ist es eine Nummerierung.
        let (aus, _) = mit(&[("Punkt", true), (" 3.2. dazu", false)]);
        assert_eq!(aus, "Punkt 3.2. dazu");
    }

    #[test]
    fn punktregeln_lassen_geschuetzte_punkte_stehen_und_machen_keine_satzgrenze_am_stueck() {
        // Ohne Schutz fällt der Punkt vor dem kleinen Wort weg, ein geschützter Punkt bleibt.
        assert_eq!(
            expand("Das gilt allen. aber nicht uns."),
            "Das gilt allen aber nicht uns."
        );
        let (aus, schutz) = mit(&[
            ("Das gilt allen", false),
            (".", true),
            (" aber nicht uns.", false),
        ]);
        assert_eq!(aus, "Das gilt allen. aber nicht uns.");
        assert_eq!(stuecke(&aus, &schutz), vec!["."]);
        // Keine künstliche Satzgrenze: „ca.“ vor einem geschützten Wort endet keinen Satz.
        let (aus, _) = mit(&[("Das ist ca. ", false), ("Zehn", true), (" Meter.", false)]);
        assert_eq!(aus, "Das ist circa Zehn Meter.");
        let (aus, _) = mit(&[("Das ist ca.", false), (" 5", true), (" Meter.", false)]);
        assert_eq!(aus, "Das ist circa 5 Meter.");
        // Nur am wirklichen Ende bleibt der Punkt.
        let (aus, _) = mit(&[("Das ist ca.", false)]);
        assert_eq!(aus, "Das ist circa.");
        // Wie ohne Schutz: eine Abkürzung, die Sätze beendet, vor einem geschützten Großbuchstaben.
        let (aus, _) = mit(&[
            ("Äpfel usw. ", false),
            ("Birnen", true),
            (" liegen da.", false),
        ]);
        assert_eq!(aus, expand("Äpfel usw. Birnen liegen da."));
        // Der Punkt nach einem geschützten Wort vor einem kleinen Wort fällt weg (die Umgebung zählt).
        let (aus, _) = mit(&[("Das gilt Alle", true), (". aber nicht uns.", false)]);
        assert_eq!(aus, "Das gilt Alle aber nicht uns.");
    }

    #[test]
    fn eine_abkuerzung_endet_nicht_an_einem_geschuetzten_wortteil() {
        // „ca.“ direkt vor einem geschützten Zeichen ohne Lücke ist keine Abkürzung (Wortrand wie sonst).
        let (aus, _) = mit(&[("ca.", false), ("20", true), (" Leute", false)]);
        assert_eq!(aus, "ca.20 Leute");
        // Ein geschützter Buchstabe vor „ca.“ nimmt ihm den Wortanfang.
        let (aus, _) = mit(&[("X", true), ("ca. 5", false)]);
        assert_eq!(aus, "Xca. 5");
    }

    #[test]
    fn marken_werden_am_zusammengesetzten_text_erkannt_und_haben_vorrang() {
        // `[laXhen]` mit X → c: ein Tag, auch wenn ein Stück davon eingesetzt wurde. Es bleibt, wie es ist.
        let (aus, schutz) = mit(&[("Hallo [la", false), ("c", true), ("hen] ca. 5", false)]);
        assert_eq!(aus, "Hallo [lachen] circa 5");
        assert!(schutz.ist_leer(), "das Stück im Tag gehört dem Tag");
        // Ein Tag, das nur aus eingesetzten Stücken und einem Rest besteht, hat denselben Vorrang.
        let (aus, _) = mit(&[
            ("[ca", true),
            (". ", false),
            ("5]", true),
            (" ca. 5", false),
        ]);
        assert_eq!(aus, "[ca. 5] circa 5");
        // Ein Sprechermarker, dessen Anfang eingesetzt wurde, bleibt ganz.
        let (aus, _) = mit(&[
            ("<D", false),
            ("r.", true),
            (" Anna:ruhig> Hallo ca. zehn.", false),
        ]);
        assert_eq!(aus, "<Dr. Anna:ruhig> Hallo circa zehn.");
        // Ein Vergleich ist kein Marker: die Abkürzung darin wird ausgeschrieben.
        let (aus, _) = mit(&[("a < ca. b und ", false), ("b", true), (" > c", false)]);
        assert_eq!(aus, "a < circa b und b > c");
    }

    #[test]
    fn die_sprache_wird_einmal_am_ganzen_text_erkannt() {
        let (text, schutz) = aus_paaren(&[
            ("Dr. Müller kommt ca. 5 Uhr", false),
            (
                " and the of it is not that was for on are with the and",
                true,
            ),
        ]);
        // Das Englisch des eingesetzten Stücks bestimmt die Sprache des ganzen Textes: nichts wird ausgeschrieben.
        assert!(!is_german(&text));
        let (aus, bleibt) = expand_if_german_mit_schutz(&text, &schutz);
        assert_eq!(aus, text);
        assert_eq!(bleibt, schutz);
        // Dasselbe Stück ohne Schutz erkennt dieselbe Sprache.
        assert_eq!(expand_if_german(&text), text);
        // Deutsch: wie sonst, nur am geschützten Stück vorbei.
        let (text, schutz) = aus_paaren(&[
            ("Dr. Müller und ", false),
            ("ca. 5", true),
            (" und ca. 6 sind da.", false),
        ]);
        assert_eq!(
            expand_if_german_geschuetzt(&text, &schutz),
            "Doktor Müller und ca. 5 und circa 6 sind da."
        );
    }

    /// Die Bausteine für den Zufallsnachweis: Abkürzungen, Daten, Lexikon, Punkte, Tags, Marker.
    const STUECKE: [&str; 16] = [
        "ca. ",
        "Dr. ",
        "z. B. ",
        "31.5. ",
        "Beurlaubung",
        "usw. ",
        "Wolf",
        "x ",
        ". ",
        "1 Mio. ",
        "[tag] ",
        "<A:b> ",
        "Mo. ",
        "é",
        "bzw. ",
        "min. 5 ",
    ];

    struct Strom(u64);
    impl Strom {
        fn bis(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    fn zufall(strom: &mut Strom) -> Vec<(&'static str, bool)> {
        (0..1 + strom.bis(7))
            .map(|_| (STUECKE[strom.bis(STUECKE.len())], strom.bis(3) == 0))
            .collect()
    }

    /// Der Nachweis, dass kein Eingriff ein geschütztes Zeichen berührt: Was der Schutz bezeichnet, ist
    /// danach genau das, was vorher geschützt war (ohne die Stücke in Tags und Markern, die den Marken
    /// gehören), und ganz geschützter Text bleibt Byte für Byte, wie er ist.
    #[test]
    fn geschuetzte_zeichen_bleiben_in_jeder_mischung_unveraendert_und_in_reihenfolge() {
        let mut strom = Strom(0xC0FF_EE15_BADC_0DE5);
        for lauf in 0..20_000 {
            let teile = zufall(&mut strom);
            let (text, schutz) = aus_paaren(&teile);
            let (aus, bleibt) = expand_mit_schutz(&text, &schutz);
            let marken = protected_spans(&text);
            let soll: String = schutz
                .bereiche()
                .iter()
                .flat_map(|b| {
                    text[b.clone()]
                        .char_indices()
                        .map(move |(i, c)| (b.start + i, c))
                })
                .filter(|(i, _)| !marken.iter().any(|m| m.contains(i)))
                .map(|(_, c)| c)
                .collect();
            assert_eq!(
                stuecke(&aus, &bleibt).concat(),
                soll,
                "Lauf {lauf}: {teile:?}"
            );
            for b in bleibt.bereiche() {
                assert!(
                    aus.is_char_boundary(b.start) && aus.is_char_boundary(b.end),
                    "Lauf {lauf}"
                );
            }
            // Alles geschützt: nichts ändert sich.
            let alles = Schutz::aus_bereichen(vec![0..text.len()]);
            assert_eq!(
                expand_mit_schutz(&text, &alles).0,
                text,
                "Lauf {lauf}: {text:?}"
            );
        }
    }
}
