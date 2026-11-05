//! Suche im Buch (Plan §3.3): BM25 über die Stücke, mit Spoilergrenze.
//! Gleiche Formel, Parameter, Top-k, Gleichstandsregel, Nummerierung `Q<k>`
//! und Erstnennung wie der Swift-Kern (Referenz, `BookTalkSearch.swift`).
//!
//! - Bereich „bis hier“: nur Stücke mit Anfang vor der Grenze; ein
//!   überspannendes Stück wird an der Satzgrenze davor gekürzt.
//! - Die Anfrage wird nur um Namen erweitert, die in der Frage vorkommen und
//!   im sichtbaren Text nachgewiesen sind.

use std::collections::{HashMap, HashSet};

use super::index::{BookIndex, Chunk};
use super::intent::BookTalkIntent;
use super::lex::{contains, folded_words, tokenize, Entity};

pub const DEFAULT_K: usize = 6;
/// Treffer unter 40 % der besten Punktzahl fallen weg.
pub const MIN_RELATIVE_SCORE: f64 = 0.4;
pub const RECAP_CHUNKS: usize = 5;
/// Pro Auszug Kopfzeile und Begrenzer im Prompt.
pub const HEADER_CHARS: usize = 90;

const K1: f64 = 1.2;
const B: f64 = 0.75;

/// Okapi BM25 über vortokenisierte Dokumente.
pub struct Bm25 {
    tf: Vec<HashMap<String, u32>>,
    lengths: Vec<f64>,
    avg_length: f64,
    df: HashMap<String, u32>,
}

impl Bm25 {
    pub fn new(documents: &[Vec<String>]) -> Self {
        let mut tf = Vec::new();
        let mut df: HashMap<String, u32> = HashMap::new();
        let mut lengths = Vec::new();
        for doc in documents {
            let mut m: HashMap<String, u32> = HashMap::new();
            for t in doc {
                *m.entry(t.clone()).or_default() += 1;
            }
            for t in m.keys() {
                *df.entry(t.clone()).or_default() += 1;
            }
            tf.push(m);
            lengths.push(doc.len() as f64);
        }
        let avg_length = if lengths.is_empty() {
            0.0
        } else {
            (lengths.iter().sum::<f64>() / lengths.len() as f64).max(1.0)
        };
        Self {
            tf,
            lengths,
            avg_length,
            df,
        }
    }

    pub fn score(&self, query: &[String], i: usize) -> f64 {
        if i >= self.tf.len() {
            return 0.0;
        }
        let n = self.tf.len() as f64;
        let mut terms: Vec<&String> = query.iter().collect::<HashSet<_>>().into_iter().collect();
        terms.sort();
        let mut total = 0.0;
        for term in terms {
            let (Some(f), Some(d)) = (self.tf[i].get(term), self.df.get(term)) else {
                continue;
            };
            let d = f64::from(*d);
            let idf = (1.0 + (n - d + 0.5) / (d + 0.5)).ln();
            let ff = f64::from(*f);
            total += idf * (ff * (K1 + 1.0))
                / (ff + K1 * (1.0 - B + B * self.lengths[i] / self.avg_length));
        }
        total
    }

    /// Dokumente mit Punktzahl > 0, beste zuerst (bei Gleichstand frühere zuerst).
    pub fn rank(&self, query: &[String], candidates: &[usize]) -> Vec<(usize, f64)> {
        let mut v: Vec<(usize, f64)> = candidates
            .iter()
            .map(|&i| (i, self.score(query, i)))
            .filter(|(_, s)| *s > 0.0)
            .collect();
        v.sort_by(|a, b| {
            if a.1 != b.1 {
                b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                a.0.cmp(&b.0)
            }
        });
        v
    }
}

#[derive(Debug, Clone)]
pub struct Excerpt {
    /// 1-basiert, in Buchreihenfolge: `[Q<id>]`.
    pub id: usize,
    /// Bei Spoilergrenze am `frontier` gekürzt.
    pub chunk: Chunk,
    pub score: f64,
    pub is_first_mention: bool,
    /// 0 = wichtigster Auszug (Erstnennung, dann Punktzahl; Rückblick: neuester).
    pub rank: usize,
}

#[derive(Debug, Clone)]
pub struct Retrieval {
    pub intent: BookTalkIntent,
    /// In Buchreihenfolge, nummeriert Q1…Qk.
    pub excerpts: Vec<Excerpt>,
    /// Im erlaubten Bereich gibt es keinen Text (Grenze vor dem ersten Stück).
    pub scope_empty: bool,
    /// Namen mit nachgewiesenem Vorkommen, mit denen die Anfrage erweitert
    /// wurde. Nur intern.
    pub expansion: Vec<String>,
}

impl Retrieval {
    pub fn excerpt_chars(&self) -> usize {
        self.excerpts.iter().map(|e| e.chunk.text_len()).sum()
    }

    /// Beste zuerst (für found_only).
    pub fn best_first(&self) -> Vec<&Excerpt> {
        let mut v: Vec<&Excerpt> = self.excerpts.iter().collect();
        v.sort_by(|a, b| {
            if a.score != b.score {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            } else {
                a.id.cmp(&b.id)
            }
        });
        v
    }

    /// Wirft den rangniedrigsten Auszug weg und nummeriert neu (hartes Budget).
    pub fn removing_lowest_rank(&self) -> Retrieval {
        let Some(worst) = self.excerpts.iter().max_by_key(|e| e.rank).map(|e| e.id) else {
            return self.clone();
        };
        let excerpts = self
            .excerpts
            .iter()
            .filter(|e| e.id != worst)
            .enumerate()
            .map(|(n, e)| Excerpt {
                id: n + 1,
                ..e.clone()
            })
            .collect();
        Retrieval {
            excerpts,
            ..self.clone()
        }
    }
}

struct Pick {
    i: usize,
    score: f64,
    first: bool,
}

/// - `frontier`: weitester gelesener oder gehörter Stand (UTF-16 im Original).
///   Bei „bis hier“ ohne Grenze bleibt der Schutz an (0, kein Treffer).
/// - `previous`: letzte Nutzerfrage (Pronomen-Auflösung ohne Modell).
/// - `entities`: nur Namen und Varianten, die in der Frage vorkommen und im
///   sichtbaren Text (vor der Grenze) nachgewiesen sind, erweitern die Anfrage.
#[allow(clippy::too_many_arguments)]
pub fn retrieve(
    index: &BookIndex,
    question: &str,
    intent: BookTalkIntent,
    frontier: Option<usize>,
    book_scope: bool,
    previous: Option<&str>,
    entities: &[Entity],
    k: usize,
    max_chars: usize,
) -> Retrieval {
    let visible: Vec<Chunk> = if book_scope {
        index.chunks.clone()
    } else {
        let f = frontier.unwrap_or(0);
        index
            .chunks
            .iter()
            .filter(|c| c.start < f)
            .filter_map(|c| c.truncated(f))
            .collect()
    };
    if visible.is_empty() {
        return Retrieval {
            intent,
            excerpts: Vec::new(),
            scope_empty: true,
            expansion: Vec::new(),
        };
    }

    let mut query = tokenize(question);
    if let Some(p) = previous {
        query.extend(tokenize(p));
    }
    let asked = folded_words(&format!("{} {}", question, previous.unwrap_or("")));
    let visible_words: Vec<Vec<String>> = visible.iter().map(|c| folded_words(&c.text)).collect();
    let mut expansion = Vec::new();
    for e in entities
        .iter()
        .filter(|e| e.forms().any(|f| contains(&asked, f)))
    {
        for f in e
            .forms()
            .filter(|f| visible_words.iter().any(|w| contains(w, f)))
        {
            expansion.push(f.clone());
            query.extend(tokenize(f));
        }
    }
    let docs: Vec<Vec<String>> = visible.iter().map(|c| tokenize(&c.text)).collect();
    let bm25 = Bm25::new(&docs);
    let all: Vec<usize> = (0..visible.len()).collect();
    let ranked = bm25.rank(&query, &all);

    let mut picked: Vec<Pick>;
    if intent == BookTalkIntent::Recap {
        let last_chapter = visible[visible.len() - 1].chapter_index;
        let in_last: Vec<usize> = all
            .iter()
            .copied()
            .filter(|&i| visible[i].chapter_index == last_chapter)
            .collect();
        let mut take: Vec<usize> = in_last[in_last.len().saturating_sub(RECAP_CHUNKS)..].to_vec();
        if take.len() < 2 {
            if let Some(before) = all
                .iter()
                .copied()
                .rev()
                .find(|&i| visible[i].chapter_index != last_chapter)
            {
                take.insert(0, before);
            }
        }
        picked = take
            .into_iter()
            .map(|i| Pick {
                i,
                score: 0.0,
                first: false,
            })
            .collect();
    } else {
        let top = ranked.first().map_or(0.0, |r| r.1);
        picked = ranked
            .iter()
            .filter(|r| r.1 >= top * MIN_RELATIVE_SCORE)
            .take(k)
            .map(|r| Pick {
                i: r.0,
                score: r.1,
                first: false,
            })
            .collect();
        if intent == BookTalkIntent::Locate {
            if let Some(earliest) = ranked.iter().map(|r| r.0).min() {
                if let Some(p) = picked.iter_mut().find(|p| p.i == earliest) {
                    p.first = true;
                } else {
                    let s = ranked.iter().find(|r| r.0 == earliest).map_or(0.0, |r| r.1);
                    if picked.len() >= k {
                        picked.pop();
                    }
                    picked.push(Pick {
                        i: earliest,
                        score: s,
                        first: true,
                    });
                }
            }
        }
    }

    let priority: Vec<Pick> = if intent == BookTalkIntent::Recap {
        picked.into_iter().rev().collect()
    } else {
        picked.sort_by(|a, b| {
            if a.first != b.first {
                return b.first.cmp(&a.first);
            }
            if a.score != b.score {
                return b
                    .score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal);
            }
            a.i.cmp(&b.i)
        });
        picked
    };
    let mut kept: Vec<(usize, f64, bool, usize)> = Vec::new();
    let mut used = 0;
    for (rank, p) in priority.iter().enumerate() {
        let cost = visible[p.i].text_len() + HEADER_CHARS;
        if used + cost > max_chars {
            continue;
        }
        used += cost;
        kept.push((p.i, p.score, p.first, rank));
    }
    kept.sort_by_key(|p| p.0);
    let excerpts = kept
        .into_iter()
        .enumerate()
        .map(|(n, (i, score, first, rank))| Excerpt {
            id: n + 1,
            chunk: visible[i].clone(),
            score,
            is_first_mention: first,
            rank,
        })
        .collect();
    Retrieval {
        intent,
        excerpts,
        scope_empty: false,
        expansion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bm25_orders_by_relevance_and_breaks_ties_by_position() {
        let docs: Vec<Vec<String>> = ["apfel birne", "apfel apfel apfel", "kirsche", "apfel birne"]
            .iter()
            .map(|d| d.split(' ').map(str::to_string).collect())
            .collect();
        let bm = Bm25::new(&docs);
        let r = bm.rank(&["apfel".to_string()], &[0, 1, 2, 3]);
        assert_eq!(r.iter().map(|x| x.0).collect::<Vec<_>>(), [1, 0, 3]);
        assert!(r.iter().all(|x| x.1 > 0.0));
    }
}
