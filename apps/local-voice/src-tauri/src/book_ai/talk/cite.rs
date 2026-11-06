//! Belege per Konstruktion, satzweise Prüfung und Sprechfassung (Plan §3.5).
//! Gleiche Regeln wie der Swift-Kern (Referenz, `BookTalkCitations.swift`).
//!
//! Das Modell nennt nur `[Q<k>]`. Seite, Kapitel, Offsets und Zitattext setzt
//! der Code aus dem Buchtext. Jeder Satz der Antwort wird einzeln geprüft:
//!
//! - **Buchsatz**: gültiges `[Qk]` und Stützung durch ein Belegsatzfenster
//!   (ein Satz ±1 Nachbarsatz im zitierten Stück; eines genügt, das Zitat ist
//!   der Mittelsatz): ≥ 60 % Wortdeckung, gleiche Negationsparität gegen den
//!   best passenden Teilsatz, Namen und Zahlen des Satzes stehen im Fenster;
//! - **Allgemeinsatz** („Allgemein: …“): nur bei `explain`, einer je Antwort,
//!   ≤ 30 Wörter, ohne Buchwörter außer aus Frage oder Begriff, ohne Namen;
//! - **Nicht-im-Buch**: nur feste Vorlagen;
//! - danach der Spätwörter-Filter; alles andere wird verworfen (`dropped`).
//!
//! Die Prüfung ist lexikalisch (siehe Plan §7a). Rein, kein I/O, kein Text im Log.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::index::{sentence_spans, BookIndex, Chunk};
use super::intent::{BookTalkIntent, Lang};
use super::lex::{
    contains, content_stems, content_words, folded_words, norm_words, tokenize, Entity, EntityKind,
};
use super::retrieve::Excerpt;

/// Version der Belegprüfung (Bindung der Eval-Freigabe).
pub const CHECK_VERSION: &str = "book_talk_check@1";
/// Längstes Zitat in UTF-16-Einheiten.
pub const QUOTE_MAX_CHARS: usize = 240;
const MIN_COVERAGE: f64 = 0.6;
const MAX_GENERAL_WORDS: usize = 30;
/// Längstes gesprochenes Zitat in Wörtern.
pub const SPOKEN_QUOTE_WORDS: usize = 25;

/// Ein Beleg: Stelle im Buch mit wörtlichem Zitat (Form wie `Citation` im Vertrag).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BookTalkCitation {
    /// Auszugsnummer `Q<k>` im Prompt.
    pub q: u32,
    /// Seite aus der letzten Seitenmarke vor `start`; fehlt, wenn keine kam.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    pub chapter_index: u32,
    pub chapter_title: String,
    /// UTF-16-Offsets im Originaltext (Ende exklusiv).
    pub start: u32,
    pub end: u32,
    pub quote: String,
}

// ---------------------------------------------------- Marker `[Q<k>]` ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub ids: Vec<usize>,
    /// UTF-16-Offset im bereinigten Text.
    pub offset: usize,
}

/// Entfernt `[Q<k>]` aus der Modellausgabe, auch gestreamt in beliebigen
/// Stücken, und faltet Leerraum. Die Summe der `feed`-Rückgaben plus
/// `finish()` ist der Endtext; `markers` hält die Stellen.
#[derive(Debug, Default)]
pub struct DeltaFilter {
    pending: Vec<char>,
    held_space: bool,
    started: bool,
    emitted: usize,
    pub markers: Vec<Marker>,
}

impl DeltaFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn extract(raw: &str) -> (String, Vec<Marker>) {
        let mut f = Self::new();
        let mut text = f.feed(raw);
        text.push_str(&f.finish());
        (text, f.markers)
    }

    pub fn feed(&mut self, s: &str) -> String {
        let mut out = String::new();
        for ch in s.chars() {
            out.push_str(&self.step(ch));
        }
        out
    }

    pub fn finish(&mut self) -> String {
        let rest: String = self.pending.drain(..).collect();
        self.held_space = false;
        self.emit(&rest)
    }

    fn step(&mut self, ch: char) -> String {
        if self.pending.is_empty() {
            if ch == '[' {
                self.pending.push('[');
                return String::new();
            }
            return self.emit(&ch.to_string());
        }
        if ch == ']' {
            let inner: String = self.pending[1..].iter().collect();
            self.pending.clear();
            if let Some(ids) = parse_ids(&inner) {
                self.markers.push(Marker {
                    ids,
                    offset: self.emitted,
                });
                return String::new();
            }
            return self.emit(&format!("[{inner}]"));
        }
        if self.pending.len() >= 24 || !is_marker_char(ch) {
            let text: String = self.pending.drain(..).collect();
            let mut out = self.emit(&text);
            out.push_str(&self.step(ch));
            return out;
        }
        self.pending.push(ch);
        String::new()
    }

    fn emit(&mut self, s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            if c.is_whitespace() {
                if self.started {
                    self.held_space = true;
                }
                continue;
            }
            if self.held_space && !".,;:!?".contains(c) {
                out.push(' ');
                self.emitted += 1;
            }
            self.held_space = false;
            out.push(c);
            self.emitted += c.len_utf16();
            self.started = true;
        }
        out
    }
}

fn is_marker_char(c: char) -> bool {
    matches!(c, 'Q' | 'q' | ',' | ';' | ' ') || c.is_ascii_digit()
}

fn parse_ids(inner: &str) -> Option<Vec<usize>> {
    let parts: Vec<&str> = inner
        .split(|c| c == ',' || c == ';' || c == ' ')
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let mut ids = Vec::new();
    for p in parts {
        let n = p.chars().count();
        let mut cs = p.chars();
        let first = cs.next()?;
        if !(2..=4).contains(&n) || !(first == 'Q' || first == 'q') {
            return None;
        }
        let rest: String = cs.collect();
        if !rest.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        ids.push(rest.parse::<usize>().ok()?);
    }
    Some(ids)
}

// ----------------------------------------------------------- Prüfung ----

/// Art eines geprüften Satzes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentenceKind {
    /// Buchsatz mit gültigem Beleg und Stützung.
    Cited,
    /// „Allgemein:“-Satz (nur explain, höchstens einer).
    General,
    /// Feste Nicht-im-Buch-Vorlage.
    Absent,
    /// Ohne gültigen oder gestützten Beleg oder mit Spätwort: verworfen.
    Dropped,
}

/// Alles, was die Prüfung eines Antwortsatzes braucht.
pub struct Grounding<'a> {
    pub index: &'a BookIndex,
    pub excerpts: &'a [Excerpt],
    pub entities: &'a [Entity],
    pub intent: BookTalkIntent,
    pub question: &'a str,
    /// Stämme von Inhaltswörtern, die im Buch erst hinter der Grenze erstmals
    /// vorkommen (nur Bereich „bis hier“).
    pub late_stems: &'a HashSet<String>,
    /// Stämme aller Inhaltswörter des ganzen Buchs.
    pub book_stems: &'a HashSet<String>,
}

const NEGATIONS: &[&str] = &[
    "nicht", "nichts", "nie", "niemals", "ohne", "weder", "niemand", "nirgends", "not", "no",
    "never", "without", "neither", "nothing", "nobody", "none",
];

fn number_words() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        [
            ("null", "0"),
            ("zwei", "2"),
            ("drei", "3"),
            ("vier", "4"),
            ("funf", "5"),
            ("sechs", "6"),
            ("sieben", "7"),
            ("acht", "8"),
            ("neun", "9"),
            ("zehn", "10"),
            ("elf", "11"),
            ("zwolf", "12"),
            ("dreizehn", "13"),
            ("vierzehn", "14"),
            ("funfzehn", "15"),
            ("sechzehn", "16"),
            ("siebzehn", "17"),
            ("achtzehn", "18"),
            ("neunzehn", "19"),
            ("zwanzig", "20"),
            ("dreissig", "30"),
            ("vierzig", "40"),
            ("funfzig", "50"),
            ("sechzig", "60"),
            ("siebzig", "70"),
            ("achtzig", "80"),
            ("neunzig", "90"),
            ("hundert", "100"),
            ("tausend", "1000"),
            ("million", "1000000"),
            ("millionen", "1000000"),
            ("two", "2"),
            ("three", "3"),
            ("four", "4"),
            ("five", "5"),
            ("six", "6"),
            ("seven", "7"),
            ("eight", "8"),
            ("nine", "9"),
            ("ten", "10"),
            ("eleven", "11"),
            ("twelve", "12"),
            ("twenty", "20"),
            ("thirty", "30"),
            ("forty", "40"),
            ("fifty", "50"),
            ("hundred", "100"),
            ("thousand", "1000"),
        ]
        .into_iter()
        .collect()
    })
}

fn absent_templates() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        [
            "Dazu steht bis hier nichts im Buch.",
            "Dazu steht bis hierhin nichts im Buch.",
            "Davon steht nichts im Buch.",
            "Dazu steht nichts im Buch.",
            "Dazu finde ich bis hier nichts.",
            "Dazu finde ich im Buch nichts.",
            "Bis zu deiner Leseposition steht dazu noch nichts im Buch.",
            "The book says nothing about that so far.",
            "The book says nothing about that.",
            "There is nothing about that in the book.",
            "I found nothing about that in the book.",
        ]
        .iter()
        .map(|t| norm_words(t).join(" "))
        .collect()
    })
}

pub fn is_absent_template(sentence: &str) -> bool {
    absent_templates().contains(&norm_words(sentence).join(" "))
}

fn has_general_prefix(sentence: &str) -> bool {
    let l = sentence.to_lowercase();
    l.starts_with("allgemein:") || l.starts_with("general:")
}

pub fn negation_count(words: &[String]) -> usize {
    words
        .iter()
        .filter(|w| {
            NEGATIONS.contains(&w.as_str()) || (w.starts_with("kein") && w.chars().count() <= 7)
        })
        .count()
}

pub fn numbers(words: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for w in words {
        if !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()) {
            let t = w.trim_start_matches('0');
            out.insert(if t.is_empty() {
                "0".to_string()
            } else {
                t.to_string()
            });
        } else if let Some(n) = number_words().get(w.as_str()) {
            out.insert((*n).to_string());
        }
    }
    out
}

fn shared_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// Gleiches Wort oder gemeinsamer Anfang ab 5 Zeichen („Freund“ ~ „Freundes“).
fn covered(word: &str, window: &[String]) -> bool {
    window
        .iter()
        .any(|w| w == word || shared_prefix(w, word) >= 5)
}

/// Allgemeinsatz: nur bei explain, genau ein Satz je Antwort, ≤ 30 Wörter,
/// keine Buchwörter außer aus der Frage, keine Namen.
fn is_allowed_general(sentence: &str, ctx: &Grounding<'_>) -> bool {
    if ctx.intent != BookTalkIntent::Explain || !has_general_prefix(sentence) {
        return false;
    }
    let body = sentence.split_once(':').map(|(_, b)| b).unwrap_or_default();
    if sentence.split_whitespace().count() > MAX_GENERAL_WORDS {
        return false;
    }
    let seq = folded_words(body);
    if ctx
        .entities
        .iter()
        .any(|e| e.forms().any(|f| contains(&seq, f)))
    {
        return false;
    }
    let asked = content_stems(ctx.question);
    !content_stems(body)
        .iter()
        .any(|s| ctx.book_stems.contains(s) && !asked.contains(s))
}

/// Prüft einen Satz (Belegtreue). `ids` sind die `[Q<k>]` im Satz,
/// `general_free`: der Allgemeinsatz der Antwort ist noch frei.
pub fn evaluate(
    sentence: &str,
    ids: &[usize],
    ctx: &Grounding<'_>,
    general_free: bool,
) -> (SentenceKind, Vec<BookTalkCitation>) {
    let result = if is_absent_template(sentence) {
        (SentenceKind::Absent, Vec::new())
    } else if general_free && is_allowed_general(sentence, ctx) {
        (SentenceKind::General, Vec::new())
    } else {
        let cites = ground(sentence, ids, ctx);
        if cites.is_empty() {
            (SentenceKind::Dropped, Vec::new())
        } else {
            (SentenceKind::Cited, cites)
        }
    };
    if result.0 != SentenceKind::Dropped
        && !ctx.late_stems.is_empty()
        && !content_stems(sentence).is_disjoint(ctx.late_stems)
    {
        return (SentenceKind::Dropped, Vec::new());
    }
    result
}

/// Buchsatz: Stützung durch ein Belegsatzfenster; es genügt ein Fenster, das
/// Zitat ist dessen Mittelsatz.
fn ground(sentence: &str, ids: &[usize], ctx: &Grounding<'_>) -> Vec<BookTalkCitation> {
    let mut out = Vec::new();
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    for &id in ids {
        if id < 1 || id > ctx.excerpts.len() {
            continue;
        }
        let chunk = &ctx.excerpts[id - 1].chunk;
        let Some(q) = supported_quote(sentence, chunk, ctx) else {
            continue;
        };
        if !seen.insert((q.start, q.end)) {
            continue;
        }
        out.push(BookTalkCitation {
            q: id as u32,
            page: ctx.index.page_at(q.start),
            chapter_index: chunk.chapter_index as u32,
            chapter_title: chunk.chapter_title.clone(),
            start: q.start as u32,
            end: q.end as u32,
            quote: q.text,
        });
    }
    out
}

/// Zitat im Original.
pub struct Quote {
    pub text: String,
    pub start: usize,
    pub end: usize,
    /// Belegsatz plus je ein Nachbarsatz (nur bei `best_quote`).
    pub window: String,
}

fn similarity(want: &HashSet<String>, got: &HashSet<String>) -> f64 {
    if want.is_empty() || got.is_empty() {
        return 0.0;
    }
    want.intersection(got).count() as f64 / (want.len() as f64 * got.len() as f64).sqrt()
}

fn supported_quote(statement: &str, chunk: &Chunk, ctx: &Grounding<'_>) -> Option<Quote> {
    let units = &chunk.units;
    let sentences = sentence_spans(units);
    if sentences.is_empty() {
        return None;
    }
    let want: HashSet<String> = tokenize(statement).into_iter().collect();
    let mut best: Option<(f64, usize)> = None;
    for i in 0..sentences.len() {
        let w0 = sentences[i.saturating_sub(1)].0;
        let w1 = sentences[(i + 1).min(sentences.len() - 1)].1;
        let window = String::from_utf16_lossy(&units[w0..w1]);
        if !supported(statement, &window, ctx) {
            continue;
        }
        let got: HashSet<String> = tokenize(&String::from_utf16_lossy(
            &units[sentences[i].0..sentences[i].1],
        ))
        .into_iter()
        .collect();
        let sc = similarity(&want, &got);
        if best.map_or(true, |b| sc > b.0) {
            best = Some((sc, i));
        }
    }
    let (_, i) = best?;
    quote(chunk, sentences[i].0, sentences[i].1)
}

/// Verneinung: Parität der Negationen im Antwortsatz gegen die Negationen der
/// best passenden Teilsatz-Einheit des Fensters (Komma, Semikolon, Doppelpunkt,
/// Satz- und Anführungszeichen trennen).
fn clause_negations(window: &str, content: &[String]) -> usize {
    let mut best: (i64, usize) = (-1, 0);
    for clause in window
        .split(|c| ",;:.!?„“”\"".contains(c))
        .filter(|c| !c.is_empty())
    {
        let w = folded_words(clause);
        let hits = content.iter().filter(|c| covered(c, &w)).count() as i64;
        if hits > best.0 {
            best = (hits, negation_count(&w));
        }
    }
    best.1
}

fn supported(sentence: &str, window: &str, ctx: &Grounding<'_>) -> bool {
    let answer_words = folded_words(sentence);
    let window_words = folded_words(window);
    let content = content_words(sentence);
    if content.is_empty() {
        return false;
    }
    let hits = content.iter().filter(|w| covered(w, &window_words)).count();
    if (hits as f64) / (content.len() as f64) < MIN_COVERAGE {
        return false;
    }
    if negation_count(&answer_words) % 2 != clause_negations(window, &content) % 2 {
        return false;
    }
    for e in ctx.entities.iter().filter(|e| e.kind != EntityKind::Term) {
        let in_answer = e.forms().any(|f| contains(&answer_words, f));
        if in_answer && !e.forms().any(|f| contains(&window_words, f)) {
            return false;
        }
    }
    numbers(&answer_words).is_subset(&numbers(&window_words))
}

fn quote(chunk: &Chunk, from: usize, to: usize) -> Option<Quote> {
    let units = &chunk.units;
    let mut to = to;
    if to - from > QUOTE_MAX_CHARS {
        let mut e = from + QUOTE_MAX_CHARS;
        while e > from + QUOTE_MAX_CHARS / 2 && units[e - 1] != 32 {
            e -= 1;
        }
        while e > from && (units[e - 1] == 32 || units[e - 1] == 44) {
            e -= 1;
        }
        to = e;
    }
    if to <= from {
        return None;
    }
    Some(Quote {
        text: String::from_utf16_lossy(&units[from..to]),
        start: chunk.offset_map[from],
        end: chunk.offset_map[to - 1] + 1,
        window: String::new(),
    })
}

/// Ähnlichster Satz (oder zwei benachbarte, ≤ 240 Zeichen) zur Aussage;
/// Offsets im Original.
pub fn best_quote(statement: &str, chunk: &Chunk) -> Option<Quote> {
    let units = &chunk.units;
    let sentences = sentence_spans(units);
    if sentences.is_empty() {
        return None;
    }
    let want: HashSet<String> = tokenize(statement).into_iter().collect();
    let sets: Vec<HashSet<String>> = sentences
        .iter()
        .map(|s| {
            tokenize(&String::from_utf16_lossy(&units[s.0..s.1]))
                .into_iter()
                .collect()
        })
        .collect();
    let (mut best_idx, mut best_score) = (0usize, -1.0f64);
    for (i, set) in sets.iter().enumerate() {
        let sc = similarity(&want, set);
        if sc > best_score {
            best_score = sc;
            best_idx = i;
        }
    }
    let (mut lo, mut hi) = (best_idx, best_idx);
    for i in 0..sentences.len() - 1 {
        let (a, b) = (sentences[i], sentences[i + 1]);
        if b.1 - a.0 > QUOTE_MAX_CHARS {
            continue;
        }
        let union: HashSet<String> = sets[i].union(&sets[i + 1]).cloned().collect();
        let sc = similarity(&want, &union);
        if sc > best_score + 0.1 {
            best_score = sc;
            lo = i;
            hi = i + 1;
        }
    }
    let w0 = sentences[lo.saturating_sub(1)].0;
    let w1 = sentences[(hi + 1).min(sentences.len() - 1)].1;
    let q = quote(chunk, sentences[lo].0, sentences[hi].1)?;
    Some(Quote {
        window: String::from_utf16_lossy(&units[w0..w1]),
        ..q
    })
}

// --------------------------------------------------------- Spätwörter ----

/// Stämme von Inhaltswörtern, die erst hinter der Grenze erstmals vorkommen.
pub fn late_stems(index: &BookIndex, frontier: usize) -> HashSet<String> {
    let mut before = HashSet::new();
    let mut all = HashSet::new();
    for c in &index.chunks {
        all.extend(content_stems(&c.text));
        if c.start < frontier {
            if let Some(t) = c.truncated(frontier) {
                before.extend(content_stems(&t.text));
            }
        }
    }
    all.difference(&before).cloned().collect()
}

pub fn book_stems(index: &BookIndex) -> &HashSet<String> {
    index.stems.get_or_init(|| {
        let mut out = HashSet::new();
        for c in &index.chunks {
            out.extend(content_stems(&c.text));
        }
        out
    })
}

// ------------------------------------------------------ Sprechfassung ----

const ABBREVIATIONS_DE: &[(&str, &str)] = &[
    ("z. B.", "zum Beispiel"),
    ("z.B.", "zum Beispiel"),
    ("d. h.", "das heißt"),
    ("d.h.", "das heißt"),
    ("u. a.", "unter anderem"),
    ("u.a.", "unter anderem"),
    ("usw.", "und so weiter"),
    ("bzw.", "beziehungsweise"),
    ("ca.", "circa"),
    ("vgl.", "vergleiche"),
    ("ggf.", "gegebenenfalls"),
    ("evtl.", "eventuell"),
    ("Nr.", "Nummer"),
    ("S.", "Seite"),
    ("Dr.", "Doktor"),
    ("etc.", "und so weiter"),
];
const ABBREVIATIONS_EN: &[(&str, &str)] = &[
    ("e.g.", "for example"),
    ("i.e.", "that is"),
    ("etc.", "and so on"),
    ("vs.", "versus"),
    ("approx.", "approximately"),
    ("No.", "number"),
    ("Dr.", "Doctor"),
    ("p.", "page"),
];

/// Was nach den Satz-Sprechfassungen noch gesprochen wird: Rückfrage (locate,
/// mit Beleg) oder „bis Seite N“ (recap). Der eine gesprochene Beleg hängt
/// schon an der Sprechfassung des ersten belegten Satzes.
pub fn speech_tail(
    intent: BookTalkIntent,
    has_citation: bool,
    lang: Lang,
    reading_page: Option<u32>,
) -> String {
    let de = lang == Lang::De;
    match intent {
        BookTalkIntent::Recap => match reading_page {
            Some(p) if de => format!("Das war der Stand bis Seite {p}."),
            Some(p) => format!("That was the story up to page {p}."),
            None => String::new(),
        },
        BookTalkIntent::Locate if has_citation => {
            if de {
                "Soll ich dir die Stelle vorlesen?".to_string()
            } else {
                "Shall I read the passage to you?".to_string()
            }
        }
        _ => String::new(),
    }
}

/// „Das steht auf Seite 14: ‚…‘“
pub fn reference_sentence(c: &BookTalkCitation, lang: Lang) -> String {
    let de = lang == Lang::De;
    let q = spoken_quote(&c.quote);
    let place = match (c.page, de) {
        (Some(p), true) => format!("Das steht auf Seite {p}:"),
        (Some(p), false) => format!("That is on page {p}:"),
        (None, true) => "Das steht im Buch:".to_string(),
        (None, false) => "That is in the book:".to_string(),
    };
    format!("{place} ‚{q}‘")
}

/// Höchstens 25 Wörter, sonst mit „…“ gekürzt; Anführungszeichen im Zitat raus.
pub fn spoken_quote(quote: &str) -> String {
    let cleaned: String = quote.chars().filter(|c| !"„“”\"«»".contains(*c)).collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    if words.len() <= SPOKEN_QUOTE_WORDS {
        return words.join(" ");
    }
    let mut cut = words[..SPOKEN_QUOTE_WORDS].join(" ");
    while cut.ends_with([',', ';', ':']) {
        cut.pop();
    }
    cut.push('…');
    cut
}

/// Text ohne Markdown und mit ausgeschriebenen Abkürzungen.
pub fn plain(text: &str, lang: Lang) -> String {
    let mut s: String = text.chars().filter(|c| !"*_#`".contains(*c)).collect();
    let table = if lang == Lang::De {
        ABBREVIATIONS_DE
    } else {
        ABBREVIATIONS_EN
    };
    for (a, b) in table {
        s = s.replace(a, b);
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_filter_removes_markers_across_chunks() {
        let mut f = DeltaFilter::new();
        let mut text = String::new();
        for piece in [
            "Der Meister sagt das [Q",
            "1]. Und [Q2, Q",
            "3] mehr [x",
            "y] Ende",
        ] {
            text.push_str(&f.feed(piece));
        }
        text.push_str(&f.finish());
        assert_eq!(text, "Der Meister sagt das. Und mehr [xy] Ende");
        assert_eq!(
            f.markers.iter().map(|m| m.ids.clone()).collect::<Vec<_>>(),
            [vec![1], vec![2, 3]]
        );
        assert_eq!(
            f.markers[0].offset,
            "Der Meister sagt das".encode_utf16().count()
        );
    }

    #[test]
    fn markers_need_the_exact_form() {
        let (t, m) = DeltaFilter::extract("A [Q12] b [Q] c [Q1x] d [QQ1].");
        assert_eq!(m.len(), 1);
        assert_eq!(t, "A b [Q] c [Q1x] d [QQ1].");
    }

    #[test]
    fn numbers_negations_and_prefixes() {
        let w = folded_words("Es waren zwei Kinder, 007 Boote und keine Hilfe nicht");
        assert_eq!(negation_count(&w), 2);
        let n = numbers(&w);
        assert!(n.contains("2") && n.contains("7"));
        assert!(covered("freund", &folded_words("des Freundes Haus")));
        assert!(!covered("haus", &folded_words("Hausmeister")));
    }

    #[test]
    fn spoken_quote_is_cut_at_25_words() {
        let q = (1..=40)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let s = spoken_quote(&q);
        assert!(s.ends_with('…'));
        assert_eq!(s.split_whitespace().count(), 25);
        assert_eq!(spoken_quote("„Hallo“ Welt"), "Hallo Welt");
    }

    #[test]
    fn plain_expands_abbreviations_and_drops_markdown() {
        assert_eq!(
            plain("**Fett** z. B. so.", Lang::De),
            "Fett zum Beispiel so."
        );
    }

    #[test]
    fn templates_are_recognised_without_case_and_punctuation() {
        assert!(is_absent_template("dazu steht bis hier nichts im buch"));
        assert!(is_absent_template(
            "Bis zu deiner Leseposition steht dazu noch nichts im Buch."
        ));
        assert!(!is_absent_template("Dazu steht im Buch etwas."));
    }
}
