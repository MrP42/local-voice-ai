//! Abruf über den globalen Index: BM25 (gleiche Parameter wie
//! `book_ai::talk::retrieve`), Freigabe je Bereich, Lesegrenze je Buch,
//! höchstens zwei Auszüge je Dokument, Zeichenbudget, Aktualitätsbonus nur
//! bei Zeitwörtern.

use std::collections::{HashMap, HashSet};

use crate::book_ai::talk::lex::{folded_words, tokenize};

use super::index::{term_freq, AppIndex, Stored};
use super::model::{ContextHit, ContextHits, Jump, SourceKind};

pub const DEFAULT_K: usize = 6;
pub const DEFAULT_MAX_CHARS: usize = 3_000;
/// Treffer unter 40 % der besten Punktzahl fallen weg (wie im Buch-Gespräch).
pub const MIN_RELATIVE_SCORE: f64 = 0.4;
pub const MAX_PER_DOC: usize = 2;
/// Mindestabdeckung je Treffer: Anteil der verschiedenen Suchterme, die im
/// sichtbaren Text vorkommen. Darunter gilt der Treffer als zufällig (relative
/// 40 % allein liefern auch bei unpassenden Fragen etwas). Unabhängig von der
/// Korpusgröße, anders als eine absolute BM25-Schwelle. Vorläufiger Wert, wird
/// mit dem Eval kalibriert (Codex Antwort 1).
pub const MIN_COVERAGE: f64 = 0.34;
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
    /// Mindestabdeckung der Suchterme je Treffer (Standard [`MIN_COVERAGE`]).
    pub min_coverage: f64,
}

/// Sichtbarer Text eines Stücks unter der Lesegrenze; `None` = unsichtbar.
/// Zweites Feld: `true`, wenn gekürzt wurde (dann eigene Bewertung).
/// Innen `Some(text)`, wenn an der Lesegrenze gekürzt wurde.
fn visible(s: &Stored, q: &Query) -> Option<Option<String>> {
    // Abgeleitete Inhalte mit Buchbezug (Notizen, KI-Dokumente, Gespräche):
    // nur, wenn ihre belegte Spanne vor der Lesegrenze liegt; unklare Spanne
    // nur mit Ganzes-Buch-Freigabe (Codex Must-fix 3). Gilt auch für den Titel,
    // weil der Treffer sonst gar nicht erscheint.
    if let Some(book) = s.book_id.as_ref().filter(|_| s.book_chunk.is_none()) {
        if q.whole_books.contains(book) {
            return Some(None);
        }
        let frontier = q.frontiers.get(book).copied()?;
        return match s.book_upto {
            Some(upto) if upto <= frontier => Some(None),
            _ => None,
        };
    }
    let Some(chunk) = s.book_chunk.as_ref() else {
        return Some(None);
    };
    if q.whole_books.contains(&s.doc.id) {
        return Some(None);
    }
    let frontier = q.frontiers.get(&s.doc.id).copied().unwrap_or(0);
    if s.end <= frontier {
        return Some(None);
    }
    let t = chunk.truncated(frontier)?;
    Some(Some(t.text))
}

fn bm25(idx: &AppIndex, query: &[String], tf: &HashMap<String, u32>, len: u32) -> f64 {
    let n = idx.len() as f64;
    let avg = idx.avg_len().max(1.0);
    let mut total = 0.0;
    for term in query {
        let (Some(f), d) = (tf.get(term), idx.df(term)) else {
            continue;
        };
        if d == 0 {
            continue;
        }
        let d = f64::from(d);
        let idf = (1.0 + (n - d + 0.5) / (d + 0.5)).ln();
        let ff = f64::from(*f);
        total += idf * (ff * (K1 + 1.0)) / (ff + K1 * (1.0 - B + B * f64::from(len) / avg));
    }
    total
}

fn asks_for_recency(question: &str) -> bool {
    folded_words(question)
        .iter()
        .any(|w| TIME_WORDS.contains(&w.as_str()))
}

/// Auszug auf höchstens `max` Zeichen, an einer Satz- oder Wortgrenze.
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
    let mut terms = tokenize(q.question);
    if let Some(p) = q.previous {
        terms.extend(tokenize(p));
    }
    let terms: Vec<String> = {
        let mut seen = HashSet::new();
        terms
            .into_iter()
            .filter(|t| seen.insert(t.clone()))
            .collect()
    };
    if terms.is_empty() || q.allowed.is_empty() || q.k == 0 {
        return ContextHits {
            searched,
            epoch: idx.epoch(),
            ..Default::default()
        };
    }

    // Kandidaten über die Postings (nur Stücke, die einen Suchterm enthalten).
    let mut candidates: HashSet<usize> = HashSet::new();
    for t in &terms {
        if let Some(p) = idx.postings(t) {
            candidates.extend(p.keys().copied());
        }
    }
    let recency = asks_for_recency(q.question);
    let mut scored: Vec<(usize, f64, Option<String>)> = Vec::new();
    for slot in candidates {
        let Some(s) = idx.get(slot) else { continue };
        if !q.allowed.contains(&s.doc.kind) {
            continue;
        }
        let Some(cut_text) = visible(s, q) else {
            continue;
        };
        let cut_tf;
        let (tf, len) = if let Some(text) = &cut_text {
            cut_tf = term_freq(text);
            (&cut_tf.0, cut_tf.1)
        } else {
            (&s.tf, s.len)
        };
        let covered = terms.iter().filter(|t| tf.contains_key(*t)).count();
        if (covered as f64) < q.min_coverage * terms.len() as f64 {
            continue;
        }
        let mut score = bm25(idx, &terms, tf, len);
        if score <= 0.0 {
            continue;
        }
        if recency && s.at_ms > 0 {
            let age_days = ((q.now_ms - s.at_ms) as f64 / DAY_MS).max(0.0);
            score *= 1.0 + 0.5 * (-age_days / 7.0).exp();
        }
        scored.push((slot, score, cut_text));
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
    let best = scored.first().map_or(0.0, |s| s.1);

    let mut hits = Vec::new();
    let mut per_doc: HashMap<_, usize> = HashMap::new();
    let mut used = 0usize;
    for (slot, score, cut_text) in scored {
        if hits.len() >= q.k || score < best * MIN_RELATIVE_SCORE {
            break;
        }
        let s = idx.get(slot).unwrap();
        let n = per_doc.entry(s.doc.clone()).or_insert(0);
        if *n >= MAX_PER_DOC {
            continue;
        }
        let ex = excerpt(cut_text.as_deref().unwrap_or(&s.text), MAX_EXCERPT_CHARS);
        let chars = ex.chars().count();
        if used + chars > q.max_chars {
            continue;
        }
        *n += 1;
        used += chars;
        let book_id = match s.doc.kind {
            SourceKind::Book => Some(s.doc.id.clone()),
            _ => s.book_id.clone(),
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
                offset: s.start as u32,
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
    }
}
