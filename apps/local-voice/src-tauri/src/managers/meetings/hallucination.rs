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
//!
//! Stufe 4 (G7, Issue #70, unten): Decoder-Schleifen der Transducer-Modelle
//! ("if if if if if if if if if if if the heat death") werden beim Erzeugen eines
//! Transkripts zusammengefasst. Anders als die Stufen 1 bis 3 verwirft sie nichts,
//! sondern kuerzt Woerter und fuehrt Wortzeiten mit.

use crate::managers::transcription::{TimedSegment, WordTime};

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

// ---------------------------------------------------------------------------
// Stufe 4 (G7, Issue #70): Wiederholungsschleifen des Decoders
// ---------------------------------------------------------------------------
//
// Ursache: Der Greedy-RNN-T-Decoder von transcribe-cpp (Nemotron, Parakeet)
// bleibt bei unsicherer Stelle auf EINEM Encoder-Frame haengen und gibt
// dasselbe Wort wieder und wieder aus, bis die feste Obergrenze von 10 Symbolen
// je Frame (weights.cpp: `tdt_max_symbols = 10`, nicht einstellbar, keine
// Wiederholungs- oder Blank-Strafe) das Frame weiterschaltet. Das Ergebnis sind
// Laeufe von 9 bis 13 gleichen Woertern: "if if if if if if if if if if if the
// heat death", "s s s s s s s s s sort of", "I I I I I I I I I I guess".
//
// Das Netz hier ist deterministisch und arbeitet nur auf dem Text des Blocks
// (und seinen Wortzeiten): kein Modell, keine Einstellung, kein I/O. Es greift
// beim ERZEUGEN eines Transkripts (neue Aufnahme, Neu-Transkription, Import,
// YouTube-Eigentranskription, Diktat). Gespeicherte Transkripte werden weder
// beim Anzeigen noch beim Export veraendert: wer eine Altlast loswerden will,
// transkribiert neu.

/// Ab dieser Laenge gilt ein Lauf gleicher Woerter als Schleife. Menschliches
/// Stottern und Nachdruck bleibt bei zwei bis drei ("no no no", "very very
/// very", "nein nein nein"); an echten Besprechungen kamen 4 und mehr
/// aufeinanderfolgende gleiche Woerter nur als Decoder-Schleife vor.
pub const MIN_LOOP_RUN: usize = 4;
/// Lachen und Liedtext ("ha ha ha ha", "la la la la") ist Inhalt, keine
/// Schleife: ein solcher Lauf wird auf diese Anzahl gekappt, nicht auf eins.
const INTERJECTION_KEEP: usize = 3;
/// Bruchstuecke ("cre cre cre created") sind hoechstens so lang.
const FRAGMENT_MAX_CHARS: usize = 3;

const INTERJECTIONS: &[&str] = &[
    "ha", "hah", "haha", "hahaha", "he", "hehe", "hi", "hihi", "ho", "la", "lala", "na", "nana",
    "bla", "blah",
];

/// Zahlwoerter (en, de, fr, es): "five five five five" ist eine Telefon- oder
/// Kontonummer, keine Schleife. Ziffern werden ueber `char::is_numeric` erkannt.
const NUMBER_WORDS: &[&str] = &[
    "zero", "oh", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "null", "eins", "ein", "zwei", "drei", "vier", "fünf", "fuenf", "sechs", "sieben", "acht",
    "neun", "zehn", "zéro", "un", "deux", "trois", "quatre", "cinq", "sept", "huit", "neuf",
    "dix", "cero", "uno", "dos", "tres", "cuatro", "cinco", "seis", "siete", "ocho", "nueve",
];

/// Was ein Aufruf zusammengefasst hat. Nur Zahlen, nie Text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopStats {
    /// Anzahl gefundener Laeufe.
    pub runs: usize,
    /// Anzahl entfernter Woerter.
    pub removed: usize,
}

/// Ein Wort in seine drei Teile zerlegt: fuehrende Zeichen ohne Buchstabe oder
/// Ziffer (Anfuehrungszeichen, Klammern), der Wortkern und die Satzzeichen
/// dahinter.
struct Parts<'a> {
    lead: &'a str,
    core: &'a str,
    trail: &'a str,
}

fn split_parts(tok: &str) -> Parts<'_> {
    let first = tok.char_indices().find(|&(_, c)| c.is_alphanumeric());
    let last = tok.char_indices().rev().find(|&(_, c)| c.is_alphanumeric());
    match (first, last) {
        (Some((s, _)), Some((e, ec))) => {
            let end = e + ec.len_utf8();
            Parts {
                lead: &tok[..s],
                core: &tok[s..end],
                trail: &tok[end..],
            }
        }
        _ => Parts {
            lead: tok,
            core: "",
            trail: "",
        },
    }
}

/// Vergleichsschluessel: klein geschrieben, ohne Apostrophe, ohne Satzzeichen
/// an den Raendern. Leer = kein Wort (reine Satzzeichen), daraus entsteht nie
/// ein Lauf.
fn loop_key(tok: &str) -> String {
    split_parts(tok)
        .core
        .chars()
        .filter(|c| !matches!(c, '\'' | '’' | '`' | '´'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Endet das Wort einen Satz? Dann beginnt dahinter eine neue Aeusserung, und
/// ein Lauf reisst ab ("Go. Go. Go. Go." bleibt stehen).
fn ends_sentence(tok: &str) -> bool {
    split_parts(tok)
        .trail
        .chars()
        .any(|c| matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？'))
}

fn is_latin_letters(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_alphabetic() && (c as u32) < 0x250)
}

/// Was mit einem Lauf geschieht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    /// Der Lauf ist ein Wortbruchstueck vor seinem Wort: ganz entfernen.
    DropAll,
    /// Die ersten n Woerter behalten, der Rest entfaellt.
    Keep(usize),
}

#[derive(Clone, Copy, Debug)]
struct Run {
    start: usize,
    len: usize,
    cut: Cut,
}

/// Regeln (n = Laenge des Laufs, `completed` = das naechste Wort beginnt mit
/// dem Lauf und ist laenger):
/// - Ziffern und Zahlwoerter: nie.
/// - Einzelbuchstabe ausser "a"/"i", n >= 2, vom folgenden Wort fortgesetzt
///   ("y y you", "s s s sort"): ganz entfernen. So sieht kein Mensch ein Wort an.
/// - n < 4: unveraendert (Stottern, Nachdruck, "B B C").
/// - Lachen/Liedtext: auf drei kappen.
/// - Bruchstueck bis drei Zeichen vor seinem Wort ("cre cre ... created"):
///   ganz entfernen.
/// - sonst: auf ein Wort zusammenfassen.
fn decide(key: &str, n: usize, completed: bool) -> Option<Cut> {
    if key.chars().any(char::is_numeric) || NUMBER_WORDS.contains(&key) {
        return None;
    }
    let chars = key.chars().count();
    let real_single = matches!(key, "a" | "i");
    let fragment_ok = is_latin_letters(key) && completed && !real_single;
    if chars == 1 && fragment_ok {
        return Some(Cut::DropAll);
    }
    if n < MIN_LOOP_RUN {
        return None;
    }
    if INTERJECTIONS.contains(&key) {
        return (n > INTERJECTION_KEEP).then_some(Cut::Keep(INTERJECTION_KEEP));
    }
    if chars <= FRAGMENT_MAX_CHARS && fragment_ok {
        return Some(Cut::DropAll);
    }
    Some(Cut::Keep(1))
}

/// Findet die Laeufe in einer Wortfolge. Linear, ohne Rekursion.
fn plan_runs(toks: &[&str]) -> Vec<Run> {
    let keys: Vec<String> = toks.iter().map(|t| loop_key(t)).collect();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if keys[i].is_empty() {
            i += 1;
            continue;
        }
        let mut j = i;
        while j + 1 < toks.len() && keys[j + 1] == keys[i] && !ends_sentence(toks[j]) {
            j += 1;
        }
        let n = j - i + 1;
        if n >= 2 {
            let completed = !ends_sentence(toks[j])
                && keys.get(j + 1).is_some_and(|next| {
                    next.len() > keys[i].len() && next.starts_with(keys[i].as_str())
                });
            if let Some(cut) = decide(&keys[i], n, completed) {
                runs.push(Run {
                    start: i,
                    len: n,
                    cut,
                });
            }
        }
        i = j + 1;
    }
    runs
}

/// Ein Ausgabewort. `first..=last` sind die Eingabewoerter, die es ersetzt;
/// `changed` = der Text weicht vom Eingabewort ab.
struct Out {
    first: usize,
    last: usize,
    text: String,
    changed: bool,
}

/// Grossbuchstabe und Anfuehrungszeichen eines entfernten Bruchstuecks gehen an
/// das naechste Wort ("S s s sort of" -> "Sort of").
struct Carry {
    lead: String,
    capitalize: bool,
}

fn with_carry(carry: &mut Option<Carry>, text: &str) -> (String, bool) {
    let Some(c) = carry.take() else {
        return (text.to_string(), false);
    };
    let mut out = String::with_capacity(c.lead.len() + text.len());
    out.push_str(&c.lead);
    if c.capitalize {
        let mut done = false;
        for ch in text.chars() {
            if !done && ch.is_alphanumeric() {
                out.extend(ch.to_uppercase());
                done = true;
            } else {
                out.push(ch);
            }
        }
    } else {
        out.push_str(text);
    }
    (out, true)
}

fn rewrite(toks: &[&str], runs: &[Run]) -> Vec<Out> {
    let mut out: Vec<Out> = Vec::with_capacity(toks.len());
    let mut carry: Option<Carry> = None;
    let mut i = 0;
    let mut r = 0;
    while i < toks.len() {
        if let Some(run) = runs.get(r).filter(|run| run.start == i) {
            let last = run.start + run.len - 1;
            match run.cut {
                Cut::DropAll => {
                    let p = split_parts(toks[i]);
                    carry = Some(Carry {
                        lead: p.lead.to_string(),
                        capitalize: p.core.chars().next().is_some_and(char::is_uppercase),
                    });
                }
                Cut::Keep(k) => {
                    for t in i..i + k - 1 {
                        let (text, changed) = with_carry(&mut carry, toks[t]);
                        out.push(Out {
                            first: t,
                            last: t,
                            text,
                            changed,
                        });
                    }
                    let kth = i + k - 1;
                    let p = split_parts(toks[kth]);
                    let end = split_parts(toks[last]);
                    // Das Satzzeichen hinter dem LETZTEN Wort des Laufs gilt.
                    let trail = if end.trail.is_empty() {
                        p.trail
                    } else {
                        end.trail
                    };
                    let merged = format!("{}{}{}", p.lead, p.core, trail);
                    let (text, _) = with_carry(&mut carry, &merged);
                    out.push(Out {
                        first: kth,
                        last,
                        text,
                        changed: true,
                    });
                }
            }
            i += run.len;
            r += 1;
            continue;
        }
        let (text, changed) = with_carry(&mut carry, toks[i]);
        out.push(Out {
            first: i,
            last: i,
            text,
            changed,
        });
        i += 1;
    }
    out
}

fn word_keys<'a>(toks: impl Iterator<Item = &'a str>) -> Vec<String> {
    toks.map(loop_key).filter(|k| !k.is_empty()).collect()
}

/// Fasst die Schleifen der Segmente EINES Aufrufs zusammen. Laeufe duerfen an
/// einer Segmentgrenze liegen (ein Lauf zaehlt ueber die Grenze); das Ergebnis
/// steht im ersten beteiligten Segment, ein dabei leer gewordenes Segment
/// entfaellt. Segmente ohne Schleife bleiben byte-genau unveraendert.
///
/// Wortzeiten (`words`) folgen dem Text: ein zusammengefasstes Wort deckt die
/// Zeit des ganzen Laufs ab, entfernte Woerter verschwinden. Stimmen Text und
/// Wortliste danach nicht mehr ueberein, entfallen die Wortzeiten dieses
/// Segments (`None`): lieber keine Wortzeiten als falsche.
pub fn collapse_loops_in_segments(segments: &mut Vec<TimedSegment>) -> LoopStats {
    let mut toks: Vec<&str> = Vec::new();
    let mut seg_of: Vec<usize> = Vec::new();
    for (si, s) in segments.iter().enumerate() {
        for t in s.text.split_whitespace() {
            toks.push(t);
            seg_of.push(si);
        }
    }
    let runs = plan_runs(&toks);
    if runs.is_empty() {
        return LoopStats::default();
    }
    let outs = rewrite(&toks, &runs);
    let stats = LoopStats {
        runs: runs.len(),
        removed: toks.len() - outs.len(),
    };

    // Neuer Text je Segment; None = Segment blieb, wie es war.
    let mut new_text: Vec<Option<String>> = vec![None; segments.len()];
    let mut parts: Vec<Vec<&str>> = vec![Vec::new(); segments.len()];
    for o in &outs {
        parts[seg_of[o.first]].push(o.text.as_str());
    }
    for (si, s) in segments.iter().enumerate() {
        let before: Vec<&str> = s.text.split_whitespace().collect();
        if before != parts[si] {
            new_text[si] = Some(parts[si].join(" "));
        }
    }
    drop(parts);
    drop(toks);

    for (si, text) in new_text.into_iter().enumerate() {
        let Some(text) = text else { continue };
        let seg = &mut segments[si];
        seg.words = seg.words.take().and_then(|words| {
            let wt: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
            let wruns = plan_runs(&wt);
            let wouts = rewrite(&wt, &wruns);
            let new_words: Vec<WordTime> = wouts
                .into_iter()
                .map(|o| {
                    if o.changed {
                        WordTime {
                            text: o.text,
                            start_ms: words[o.first].start_ms,
                            end_ms: words[o.last].end_ms,
                        }
                    } else {
                        words[o.first].clone()
                    }
                })
                .collect();
            let consistent = word_keys(new_words.iter().map(|w| w.text.as_str()))
                == word_keys(text.split_whitespace());
            (consistent && !new_words.is_empty()).then_some(new_words)
        });
        seg.text = text;
    }
    segments.retain(|s| !s.text.trim().is_empty());
    stats
}

/// Wie `collapse_loops_in_segments`, aber ein Absturz in der Zusammenfassung
/// darf ein erfolgreiches Transkript nie kosten: bei Panik kommt der
/// unveraenderte Text zurueck.
pub fn collapse_loops_fail_open(segments: Vec<TimedSegment>) -> (Vec<TimedSegment>, LoopStats) {
    let fallback = segments.clone();
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let mut work = segments;
        let stats = collapse_loops_in_segments(&mut work);
        (work, stats)
    }));
    match attempt {
        Ok(done) => done,
        Err(_) => (fallback, LoopStats::default()),
    }
}

/// Dasselbe fuer einen einzelnen Text (Diktat). Ohne Schleife kommt der Text
/// byte-genau zurueck.
pub fn collapse_loops(text: &str) -> (String, LoopStats) {
    let mut segs = vec![TimedSegment {
        start_ms: 0,
        end_ms: 0,
        text: text.to_string(),
        words: None,
    }];
    let stats = collapse_loops_in_segments(&mut segs);
    let out = segs.pop().map(|s| s.text).unwrap_or_default();
    (out, stats)
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

    // ----- Stufe 4 (G7, Issue #70): Wiederholungsschleifen -----

    fn text_of(input: &str) -> String {
        collapse_loops(input).0
    }

    fn seg(text: &str) -> TimedSegment {
        TimedSegment {
            start_ms: 0,
            end_ms: 10_000,
            text: text.to_string(),
            words: None,
        }
    }

    fn words_of(text: &str, start: u64, step: u64) -> Vec<WordTime> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| WordTime {
                text: w.to_string(),
                start_ms: start + i as u64 * step,
                end_ms: start + i as u64 * step + step - 10,
            })
            .collect()
    }

    // Die vier Faelle aus dem Installer 0.20.12 (Nemotron 3.5 ASR, YouTube).

    #[test]
    fn loop_of_a_function_word_becomes_one() {
        assert_eq!(
            text_of("So no if if if if if if if if if if if the heat death of the universe"),
            "So no if the heat death of the universe"
        );
    }

    #[test]
    fn loop_of_a_single_letter_fragment_vanishes_before_its_word() {
        assert_eq!(
            text_of("a long boring s s s s s s s s s sort of thin gray soup"),
            "a long boring sort of thin gray soup"
        );
    }

    #[test]
    fn loop_of_the_pronoun_i_keeps_one_i() {
        assert_eq!(
            text_of("I I I I I I I I I I guess the only thing we can do"),
            "I guess the only thing we can do"
        );
    }

    #[test]
    fn two_single_letter_fragments_before_their_word_vanish() {
        assert_eq!(text_of("there's y y you can't"), "there's you can't");
    }

    // Gegenbeispiele: das darf nie angefasst werden.

    #[test]
    fn human_emphasis_up_to_three_stays() {
        for text in [
            "no no no",
            "No, no, no, that is wrong",
            "very very very good",
            "nein nein nein, so nicht",
            "I I think so",
            "that that that is what he said",
        ] {
            assert_eq!(text_of(text), text, "{text}");
            assert_eq!(collapse_loops(text).1, LoopStats::default(), "{text}");
        }
    }

    #[test]
    fn a_longer_run_of_a_real_word_collapses_to_one() {
        assert_eq!(text_of("no no no no no no way"), "no way");
        assert_eq!(text_of("und und und und und das stimmt"), "und das stimmt");
        assert_eq!(text_of("it it it it it's fine"), "it's fine");
    }

    #[test]
    fn digits_and_number_words_are_never_collapsed() {
        for text in [
            "1 1 2 3 5",
            "0 0 0 0 0",
            "the code is 7 7 7 7 7 7",
            "five five five five five five",
            "null null null null null",
            "version 3 3 3 3",
        ] {
            assert_eq!(text_of(text), text, "{text}");
        }
    }

    #[test]
    fn spelled_letters_and_short_runs_of_letters_stay() {
        assert_eq!(text_of("it is spelled B B C"), "it is spelled B B C");
        assert_eq!(text_of("size A A A battery"), "size A A A battery");
        // Ab vier ohne folgendes Wort ist es eine Schleife.
        assert_eq!(text_of("so x x x x x then"), "so x then");
    }

    #[test]
    fn separate_sentences_are_not_a_run() {
        for text in ["Go. Go. Go. Go.", "No! No! No! No!", "Why? Why? Why? Why?"] {
            assert_eq!(text_of(text), text, "{text}");
        }
    }

    #[test]
    fn comma_separated_loop_collapses_and_keeps_the_punctuation() {
        assert_eq!(text_of("Well, well, well, well, okay"), "Well, okay");
        assert_eq!(text_of("Yes yes yes yes."), "Yes.");
        assert_eq!(text_of("it works, if if if if if, you know"), "it works, if, you know");
    }

    #[test]
    fn laughter_is_capped_at_three_not_one() {
        assert_eq!(text_of("ha ha ha ha ha ha ha"), "ha ha ha");
        assert_eq!(text_of("la la la la la la"), "la la la");
        assert_eq!(text_of("ha ha ha"), "ha ha ha");
    }

    #[test]
    fn longer_fragment_before_its_word_vanishes() {
        assert_eq!(
            text_of("you know create cre cre cre cre cre cre cre cre cre created open AI"),
            "you know create created open AI"
        );
        assert_eq!(
            text_of("sa sa sa sa sa sa sa safety and security"),
            "safety and security"
        );
        assert_eq!(
            text_of("W wh why wh wh wh wh wh wh wh wh wh why haven't you done that yet"),
            "W wh why why haven't you done that yet"
        );
    }

    #[test]
    fn a_dropped_fragment_hands_its_capital_to_the_next_word() {
        assert_eq!(text_of("S s s sort of"), "Sort of");
        assert_eq!(text_of("\"S s s sort of\""), "\"Sort of\"");
    }

    #[test]
    fn casing_is_ignored_when_comparing() {
        assert_eq!(text_of("I i I i I i guess"), "I guess");
        assert_eq!(text_of("Ä ä Ä ä Ä Ä Ärger"), "Ärger");
    }

    #[test]
    fn text_without_a_loop_is_returned_byte_for_byte() {
        let text = "  Doppelt  gespaced,\tmit Tab  und no no no am Ende.  ";
        let (out, stats) = collapse_loops(text);
        assert_eq!(out, text);
        assert_eq!(stats, LoopStats::default());
    }

    #[test]
    fn text_without_spaces_is_not_touched() {
        let text = "你好你好你好你好你好";
        assert_eq!(text_of(text), text);
        assert_eq!(text_of(""), "");
        assert_eq!(text_of("... — ..."), "... — ...");
    }

    #[test]
    fn stats_count_runs_and_removed_words() {
        let (out, stats) =
            collapse_loops("if if if if if the end, and and and and and then y y you go");
        assert_eq!(out, "if the end, and then you go");
        assert_eq!(
            stats,
            LoopStats {
                runs: 3,
                removed: 4 + 4 + 2
            }
        );
    }

    #[test]
    fn collapsing_twice_changes_nothing_more() {
        let first = text_of("So no if if if if if if if if if if if the heat death y y you can't");
        assert_eq!(text_of(&first), first);
    }

    // Segmentgrenzen, Wortzeiten.

    #[test]
    fn a_run_may_cross_a_segment_boundary() {
        let mut segs = vec![seg("and so on if if if if"), seg("if if if the heat death")];
        let stats = collapse_loops_in_segments(&mut segs);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "and so on if");
        assert_eq!(segs[1].text, "the heat death");
        assert_eq!(stats.removed, 6);
    }

    #[test]
    fn a_segment_emptied_by_a_dropped_fragment_disappears() {
        let mut segs = vec![seg("there is y y"), seg("you can't")];
        collapse_loops_in_segments(&mut segs);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "there is");
        assert_eq!(segs[1].text, "you can't");

        let mut only = vec![seg("s s s"), seg("sort of")];
        collapse_loops_in_segments(&mut only);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].text, "sort of");
    }

    #[test]
    fn untouched_segments_keep_their_text_words_and_times() {
        let words = words_of("hello there", 100, 400);
        let mut segs = vec![
            TimedSegment {
                start_ms: 100,
                end_ms: 900,
                text: "hello  there".to_string(),
                words: Some(words.clone()),
            },
            seg("i i i i i guess"),
        ];
        collapse_loops_in_segments(&mut segs);
        assert_eq!(segs[0].text, "hello  there");
        assert_eq!(segs[0].words, Some(words));
        assert_eq!((segs[0].start_ms, segs[0].end_ms), (100, 900));
        assert_eq!(segs[1].text, "i guess");
    }

    #[test]
    fn word_times_follow_the_text_and_cover_the_whole_run() {
        let text = "so if if if if if if the heat death";
        let words = words_of(text, 1_000, 100);
        let mut segs = vec![TimedSegment {
            start_ms: 1_000,
            end_ms: 1_000 + 10 * 100,
            text: text.to_string(),
            words: Some(words),
        }];
        collapse_loops_in_segments(&mut segs);
        assert_eq!(segs[0].text, "so if the heat death");
        let w = segs[0].words.as_ref().expect("words kept");
        let texts: Vec<&str> = w.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["so", "if", "the", "heat", "death"]);
        // "if" deckt die Zeit aller sechs "if" ab (1100 .. 1690).
        assert_eq!((w[1].start_ms, w[1].end_ms), (1_100, 1_690));
        // Reihenfolge und Grenzen bleiben stimmig.
        for pair in w.windows(2) {
            assert!(pair[0].start_ms <= pair[0].end_ms);
            assert!(pair[0].end_ms <= pair[1].start_ms);
        }
        // Die Segmentzeit bleibt, die Aeusserung dauerte so lange.
        assert_eq!((segs[0].start_ms, segs[0].end_ms), (1_000, 2_000));
    }

    #[test]
    fn word_times_of_a_dropped_fragment_vanish_with_it() {
        let text = "a long s s s s sort of";
        let mut segs = vec![TimedSegment {
            start_ms: 0,
            end_ms: 3_000,
            text: text.to_string(),
            words: Some(words_of(text, 0, 400)),
        }];
        collapse_loops_in_segments(&mut segs);
        assert_eq!(segs[0].text, "a long sort of");
        let w = segs[0].words.as_ref().expect("words kept");
        assert_eq!(w.len(), 4);
        assert_eq!(w[2].text, "sort");
        assert_eq!(w[2].start_ms, 2_400);
    }

    #[test]
    fn misaligned_word_list_is_dropped_instead_of_kept_wrong() {
        let mut segs = vec![TimedSegment {
            start_ms: 0,
            end_ms: 3_000,
            text: "I I I I I guess so".to_string(),
            // Die Engine lieferte eine andere Zerlegung als der Text.
            words: Some(words_of("I I I guess", 0, 500)),
        }];
        collapse_loops_in_segments(&mut segs);
        assert_eq!(segs[0].text, "I guess so");
        assert_eq!(segs[0].words, None);
    }

    #[test]
    fn a_word_list_without_the_loop_words_stays_valid() {
        // Manche Engines liefern die Wortzeiten schon ohne die Schleife.
        let mut segs = vec![TimedSegment {
            start_ms: 0,
            end_ms: 3_000,
            text: "I I I I I guess so".to_string(),
            words: Some(words_of("I guess so", 0, 500)),
        }];
        collapse_loops_in_segments(&mut segs);
        assert_eq!(segs[0].text, "I guess so");
        assert_eq!(segs[0].words, Some(words_of("I guess so", 0, 500)));
    }

    #[test]
    fn a_huge_loop_is_linear_and_does_not_panic() {
        let text = format!("start {} end", "x ".repeat(200_000));
        let started = std::time::Instant::now();
        let (out, stats) = collapse_loops(&text);
        assert_eq!(out, "start x end");
        assert_eq!(stats.removed, 199_999);
        assert!(started.elapsed().as_secs() < 5);
    }

    #[test]
    fn fail_open_returns_the_collapsed_segments() {
        let (segs, stats) = collapse_loops_fail_open(vec![seg("I I I I I guess")]);
        assert_eq!(segs[0].text, "I guess");
        assert_eq!(stats.runs, 1);
    }

    #[test]
    fn real_passage_from_the_failing_transcript() {
        let input = "ve I guess I've come to my my s s s s s s s s s sort of philosophical \
                     conclusion. So no if if if if if if if if if if if the heat death of the \
                     universe is um a long boring s s s s s s s s s sort of thin gray soup of uh \
                     I I I I I I I I I I guess the only thing we can t";
        let out = text_of(input);
        assert_eq!(
            out,
            "ve I guess I've come to my my sort of philosophical conclusion. So no if the heat \
             death of the universe is um a long boring sort of thin gray soup of uh I guess the \
             only thing we can t"
        );
    }

    #[test]
    fn the_loop_filter_rescues_a_chunk_the_repetition_check_would_drop() {
        // Vorher: Der Block gilt wegen der Schleife als Wiederholung und wird
        // als Halluzination verworfen, samt dem echten Inhalt.
        let raw = format!(
            "{}would recommend that we at least have a weekly call",
            "I ".repeat(14)
        );
        assert_eq!(check_text(&raw), Some(Reason::Repetition));
        let (clean, _) = collapse_loops(&raw);
        assert_eq!(clean, "I would recommend that we at least have a weekly call");
        assert_eq!(check_text(&clean), None);
    }
}
