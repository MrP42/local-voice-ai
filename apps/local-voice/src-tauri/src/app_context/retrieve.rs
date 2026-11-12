//! Abruf über den globalen Index: BM25 (gleiche Parameter wie
//! `book_ai::talk::retrieve`), Freigabe je Bereich, Lesegrenze je Buch,
//! höchstens zwei Auszüge je Dokument, Zeichenbudget, Aktualitätsbonus nur
//! bei Zeitwörtern.
//!
//! # Regeln (Codex-Abnahme P1)
//! - **Statistik nur über den sichtbaren, freigegebenen Bestand:** Stückzahl,
//!   Durchschnittslänge und Dokumentfrequenzen kommen je Anfrage nur aus
//!   Stücken, die freigegeben und vor der Lesegrenze sind (gekürzte Stücke mit
//!   ihrem gekürzten Text). Ungelesener Text beeinflusst die Rangfolge nicht.
//! - **Vorfrage nur bei Rückbezug:** Die letzte Frage erweitert die Suche nur,
//!   wenn die neue Frage einen Rückbezug enthält („er“, „dazu“, „und …?“).
//!   Die Abdeckung wird immer an den Termen der AKTUELLEN Frage gemessen.
//! - **Auszug um den Treffer:** Der ausgegebene Auszug ist ein Fenster um den
//!   Satz mit den meisten Suchtermen; seine eigene Abdeckung muss die
//!   Mindestabdeckung erreichen, sonst entfällt der Treffer.

use std::collections::{HashMap, HashSet};

use crate::book_ai::talk::lex::{folded_words, tokenize};

use super::index::{term_freq, AppIndex, Stored};
use super::model::{ContextHit, ContextHits, Jump, SourceKind};

pub const DEFAULT_K: usize = 6;
pub const DEFAULT_MAX_CHARS: usize = 3_000;
/// Treffer unter 40 % der besten Punktzahl fallen weg (wie im Buch-Gespräch).
pub const MIN_RELATIVE_SCORE: f64 = 0.4;
pub const MAX_PER_DOC: usize = 2;
/// Mindestabdeckung je Treffer: Anteil der verschiedenen Terme der aktuellen
/// Frage, die im ausgegebenen Auszug vorkommen. Unabhängig von der
/// Korpusgröße, anders als eine absolute BM25-Schwelle. Vorläufiger Wert, wird
/// mit dem Eval kalibriert (Codex Antwort 1).
pub const MIN_COVERAGE: f64 = 0.34;
/// Höchstens so viele Stücke werden für Auszüge geprüft (Zeitgrenze).
pub const MAX_EXAMINED: usize = 64;
/// Längster einzelner Auszug (Zeichen).
pub const MAX_EXCERPT_CHARS: usize = 700;

const K1: f64 = 1.2;
const B: f64 = 0.75;
const DAY_MS: f64 = 86_400_000.0;

/// Wörter, die nach Aktualität fragen (gefaltet wie `folded_words`).
const TIME_WORDS: &[&str] = &[
    "gestern",
    "heute",
    "vorhin",
    "neulich",
    "zuletzt",
    "letzte",
    "letzten",
    "letzter",
    "letztes",
    "kurzlich",
    "eben",
    "morgen",
    "woche",
    "yesterday",
    "today",
    "recent",
    "recently",
    "last",
    "latest",
];

/// Rückbezüge (gefaltet): nur dann zählt die Vorfrage mit.
const BACK_REFS: &[&str] = &[
    "er", "sie", "es", "ihm", "ihn", "ihr", "ihnen", "dazu", "davon", "dabei", "darin", "daruber",
    "darauf", "dort", "dem", "der", "die", "das", "dies", "diese", "dieser", "dieses", "derselbe",
    "he", "she", "it", "they", "him", "them", "that", "this", "there", "those",
];

/// Eine Anfrage.
#[derive(Clone, Debug)]
pub struct Query<'a> {
    pub question: &'a str,
    /// Letzte Nutzerfrage (Rückbezug „und was sagt er dazu?“).
    pub previous: Option<&'a str>,
    pub allowed: &'a HashSet<SourceKind>,
    /// Lesegrenze je Buch-Id (UTF-16). Fehlt sie, ist nichts vom Buch sichtbar.
    pub frontiers: &'a HashMap<String, usize>,
    /// Bücher, für die der Nutzer ausdrücklich „ganzes Buch“ bestätigt hat.
    pub whole_books: &'a HashSet<String>,
    pub k: usize,
    pub max_chars: usize,
    pub now_ms: i64,
    /// Mindestabdeckung der Terme der aktuellen Frage je Auszug
    /// (Standard [`MIN_COVERAGE`]).
    pub min_coverage: f64,
}

/// Sichtbarkeit eines Stücks für diese Anfrage.
enum Vis {
    Full,
    /// An der Lesegrenze gekürzt: eigener Text und eigene Statistik.
    Cut {
        text: String,
        tf: HashMap<String, u32>,
        len: u32,
    },
}

fn visible(s: &Stored, q: &Query) -> Option<Vis> {
    if !q.allowed.contains(&s.doc.kind) {
        return None;
    }
    // Abgeleitete Inhalte mit Buchbezug (Notizen, KI-Dokumente, Gespräche):
    // nur, wenn ihre belegte Spanne vor der Lesegrenze liegt; unklare Spanne
    // nur mit Ganzes-Buch-Freigabe (Codex Must-fix 3). Gilt auch für den Titel,
    // weil der Treffer sonst gar nicht erscheint.
    if let Some(book) = s.book_id.as_ref().filter(|_| s.book_chunk.is_none()) {
        if q.whole_books.contains(book) {
            return Some(Vis::Full);
        }
        let frontier = q.frontiers.get(book).copied()?;
        return match s.book_upto {
            Some(upto) if upto <= frontier => Some(Vis::Full),
            _ => None,
        };
    }
    let Some(chunk) = s.book_chunk.as_ref() else {
        return Some(Vis::Full);
    };
    if q.whole_books.contains(&s.doc.id) {
        return Some(Vis::Full);
    }
    let frontier = q.frontiers.get(&s.doc.id).copied().unwrap_or(0);
    if s.end <= frontier {
        return Some(Vis::Full);
    }
    let t = chunk.truncated(frontier)?;
    let (tf, len) = term_freq(&t.text);
    Some(Vis::Cut {
        text: t.text,
        tf,
        len,
    })
}

fn distinct(terms: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    terms
        .into_iter()
        .filter(|t| seen.insert(t.clone()))
        .collect()
}

fn has_back_reference(question: &str) -> bool {
    let words = folded_words(question);
    words.first().is_some_and(|w| w == "und" || w == "and")
        || words.iter().any(|w| BACK_REFS.contains(&w.as_str()))
}

fn asks_for_recency(question: &str) -> bool {
    folded_words(question)
        .iter()
        .any(|w| TIME_WORDS.contains(&w.as_str()))
}

/// Statistik des sichtbaren Bestands.
struct Stats {
    n: f64,
    avg: f64,
    df: HashMap<String, u32>,
}

impl Stats {
    fn idf(&self, term: &str) -> f64 {
        let d = f64::from(self.df.get(term).copied().unwrap_or(0));
        if d == 0.0 {
            return 0.0;
        }
        (1.0 + (self.n - d + 0.5) / (d + 0.5)).ln()
    }

    fn bm25(&self, terms: &[String], tf: &HashMap<String, u32>, len: u32) -> f64 {
        terms
            .iter()
            .filter_map(|t| tf.get(t).map(|f| (t, f64::from(*f))))
            .map(|(t, ff)| {
                self.idf(t) * (ff * (K1 + 1.0))
                    / (ff + K1 * (1.0 - B + B * f64::from(len) / self.avg))
            })
            .sum()
    }
}

/// Sätze (Zeichenbereiche) eines Textes, getrennt an `.`, `!`, `?` mit Leerraum danach.
fn sentences(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        let end_mark = matches!(chars[i], '.' | '!' | '?')
            && chars.get(i + 1).is_none_or(|c| c.is_whitespace());
        if end_mark {
            out.push((start, i + 1));
            start = i + 1;
        }
    }
    if start < chars.len() {
        out.push((start, chars.len()));
    }
    out.retain(|(a, b)| chars[*a..*b].iter().any(|c| !c.is_whitespace()));
    out
}

fn coverage(text: &str, terms: &[String]) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let tokens: HashSet<String> = tokenize(text).into_iter().collect();
    terms.iter().filter(|t| tokens.contains(*t)).count() as f64 / terms.len() as f64
}

/// Auszugsfenster (≤ `max` Zeichen) um den Satz mit den meisten Suchtermen,
/// abwechselnd nach vorn und hinten erweitert; Kürzungen mit „…“ markiert.
/// Liefert auch den Zeichenoffset des Fensters im Stück.
pub fn window(text: &str, terms: &[String], max: usize) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return (text.to_string(), 0);
    }
    let sents = sentences(&chars);
    if sents.is_empty() {
        return (String::new(), 0);
    }
    let want: HashSet<&String> = terms.iter().collect();
    let hits = |(a, b): (usize, usize)| -> usize {
        let s: String = chars[a..b].iter().collect();
        tokenize(&s)
            .iter()
            .collect::<HashSet<_>>()
            .intersection(&want)
            .count()
    };
    let best = (0..sents.len())
        .max_by(|&i, &j| hits(sents[i]).cmp(&hits(sents[j])).then(j.cmp(&i)))
        .unwrap_or(0);
    let (mut lo, mut hi) = (best, best);
    // Ein einzelner Satz über `max`: hart um seinen Anfang schneiden.
    let span = |lo: usize, hi: usize| sents[hi].1 - sents[lo].0;
    loop {
        let mut grew = false;
        if hi + 1 < sents.len() && span(lo, hi + 1) <= max {
            hi += 1;
            grew = true;
        }
        if lo > 0 && span(lo - 1, hi) <= max {
            lo -= 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }
    let (a, mut b) = (sents[lo].0, sents[hi].1);
    b = b.min(a + max);
    let mut out: String = chars[a..b].iter().collect::<String>().trim().to_string();
    if a > 0 {
        out = format!("… {out}");
    }
    if b < chars.len() {
        out.push_str(" …");
    }
    (out, a)
}

/// Kürzt auf höchstens `max` Zeichen an einer Satz- oder Wortgrenze.
pub fn excerpt(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let at = cut
        .rfind(['.', '!', '?'])
        .filter(|&i| i > max / 2)
        .map(|i| i + 1)
        .or_else(|| cut.rfind(' '))
        .unwrap_or(cut.len());
    format!("{} …", cut[..at].trim_end())
}

/// Führt den Abruf aus.
pub fn retrieve(idx: &AppIndex, q: &Query) -> ContextHits {
    let mut searched: Vec<SourceKind> = q.allowed.iter().copied().collect();
    searched.sort();
    let empty = |searched| ContextHits {
        searched,
        epoch: idx.epoch(),
        ..Default::default()
    };
    let current = distinct(tokenize(q.question));
    if current.is_empty() || q.allowed.is_empty() || q.k == 0 {
        return empty(searched);
    }
    let mut terms = current.clone();
    if let Some(p) = q.previous.filter(|_| has_back_reference(q.question)) {
        terms.extend(tokenize(p));
    }
    let terms = distinct(terms);

    // Sichtbarer Bestand und seine Statistik (nur freigegeben, nur gelesen).
    let mut vis: HashMap<usize, Vis> = HashMap::new();
    let (mut n, mut total) = (0usize, 0u64);
    let mut df: HashMap<String, u32> = HashMap::new();
    for (slot, s) in idx.slots() {
        let Some(v) = visible(s, q) else { continue };
        n += 1;
        let tf = match &v {
            Vis::Full => {
                total += u64::from(s.len);
                &s.tf
            }
            Vis::Cut { tf, len, .. } => {
                total += u64::from(*len);
                tf
            }
        };
        let mut matched = false;
        for t in terms.iter().filter(|t| tf.contains_key(*t)) {
            *df.entry(t.clone()).or_insert(0) += 1;
            matched = true;
        }
        if matched {
            vis.insert(slot, v);
        }
    }
    if vis.is_empty() {
        return empty(searched);
    }
    let stats = Stats {
        n: n as f64,
        avg: (total as f64 / n as f64).max(1.0),
        df,
    };

    let recency = asks_for_recency(q.question);
    let mut scored: Vec<(usize, f64)> = Vec::new();
    for (slot, v) in &vis {
        let s = idx.get(*slot).expect("sichtbares Stück lebt");
        let (tf, len) = match v {
            Vis::Full => (&s.tf, s.len),
            Vis::Cut { tf, len, .. } => (tf, *len),
        };
        let mut score = stats.bm25(&terms, tf, len);
        if score <= 0.0 {
            continue;
        }
        if recency && s.at_ms > 0 {
            let age_days = ((q.now_ms - s.at_ms) as f64 / DAY_MS).max(0.0);
            score *= 1.0 + 0.5 * (-age_days / 7.0).exp();
        }
        scored.push((*slot, score));
    }
    // Beste zuerst; Gleichstand: Dokumentschlüssel, dann Offset (stabil).
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let (sa, sb) = (idx.get(a.0).unwrap(), idx.get(b.0).unwrap());
                sa.doc.cmp(&sb.doc).then(sa.start.cmp(&sb.start))
            })
    });

    let mut hits = Vec::new();
    let mut per_doc: HashMap<_, usize> = HashMap::new();
    let mut used = 0usize;
    let mut best: Option<f64> = None;
    let mut examined = 0usize;
    for (slot, score) in scored {
        if hits.len() >= q.k || best.is_some_and(|b| score < b * MIN_RELATIVE_SCORE) {
            break;
        }
        let s = idx.get(slot).unwrap();
        if per_doc.get(&s.doc).copied().unwrap_or(0) >= MAX_PER_DOC {
            continue;
        }
        examined += 1;
        if examined > MAX_EXAMINED {
            break;
        }
        let text = match &vis[&slot] {
            Vis::Full => s.text.as_str(),
            Vis::Cut { text, .. } => text.as_str(),
        };
        let (ex, offset) = window(text, &terms, MAX_EXCERPT_CHARS);
        // Der ausgegebene Auszug selbst muss die aktuelle Frage abdecken.
        if coverage(&ex, &current) < q.min_coverage {
            continue;
        }
        let chars = ex.chars().count();
        if used + chars > q.max_chars {
            continue;
        }
        best.get_or_insert(score);
        *per_doc.entry(s.doc.clone()).or_insert(0) += 1;
        used += chars;
        let book_id = match s.doc.kind {
            SourceKind::Book => Some(s.doc.id.clone()),
            _ => s.book_id.clone(),
        };
        // Sprungziel: Bücher springen an den Anfang des Stücks (UTF-16 im
        // Original), sonst an den Zeichenoffset des Fensters.
        let jump_offset = match s.doc.kind {
            SourceKind::Book => s.start,
            _ => s.start + offset,
        };
        hits.push(ContextHit {
            k: hits.len() as u32 + 1,
            kind: s.doc.kind,
            title: s.title.clone(),
            chapter: s.chapter.clone(),
            at_ms: s.at_ms,
            excerpt: ex,
            jump: Jump {
                kind: s.doc.kind,
                id: s.doc.id.clone(),
                offset: jump_offset as u32,
                page: s.page,
                book_id,
            },
            score,
            version: format!("{:016x}", s.version),
        });
    }
    ContextHits {
        hits,
        searched,
        retrieve_ms: 0,
        epoch: idx.epoch(),
        question_hash: String::new(),
        ..Default::default()
    }
}
