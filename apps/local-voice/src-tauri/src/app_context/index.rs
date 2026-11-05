//! Inkrementeller invertierter Index über alle Quellen.
//!
//! Jedes Dokument wird in Stücke zerlegt (Bücher über
//! `book_ai::talk::index::build`, sonst [`super::chunk::pieces`]); jedes Stück
//! trägt seine Termhäufigkeiten. Dokumentfrequenzen und Gesamtlänge werden bei
//! jedem Einfügen und Entfernen fortgeschrieben, so dass ein geändertes
//! Dokument nur sich selbst neu zerlegt.

use std::collections::{HashMap, HashSet};

use crate::book_ai::talk::index::{build as build_book, Chunk as BookChunk};
use crate::book_ai::talk::lex::{fnv1a, tokenize};

use super::chunk;
use super::model::{ContextDoc, DocKey, SourceKind};

/// Ein indiziertes Stück.
#[derive(Clone, Debug)]
pub struct Stored {
    pub doc: DocKey,
    pub title: String,
    pub at_ms: i64,
    pub book_id: Option<String>,
    /// Kapitel (nur Bücher).
    pub chapter: Option<String>,
    pub page: Option<u32>,
    /// UTF-16 (Bücher) bzw. Zeichenoffset (sonst) im Original.
    pub start: usize,
    pub end: usize,
    pub text: String,
    /// Nur Bücher: das Originalstück für das Kürzen an der Lesegrenze.
    pub book_chunk: Option<BookChunk>,
    pub tf: HashMap<String, u32>,
    pub len: u32,
    pub book_upto: Option<usize>,
    /// Hash des Dokuments (Quellversion).
    pub version: u64,
}

#[derive(Clone, Debug)]
struct DocMeta {
    hash: u64,
    pieces: Vec<usize>,
}

/// Der Index.
#[derive(Default, Debug)]
pub struct AppIndex {
    docs: HashMap<DocKey, DocMeta>,
    slots: Vec<Option<Stored>>,
    free: Vec<usize>,
    /// Term → Stück → Häufigkeit.
    postings: HashMap<String, HashMap<usize, u32>>,
    total_len: u64,
    live: usize,
    /// Erhöht sich bei jeder Entfernung (Löschung, Widerruf, Umklassifizierung,
    /// Änderung): Ergebnisse älterer Stände sind ungültig.
    epoch: u32,
}

/// Termhäufigkeiten eines Textes.
pub fn term_freq(text: &str) -> (HashMap<String, u32>, u32) {
    let tokens = tokenize(text);
    let len = tokens.len() as u32;
    let mut tf = HashMap::new();
    for t in tokens {
        *tf.entry(t).or_insert(0) += 1;
    }
    (tf, len)
}

fn doc_hash(doc: &ContextDoc) -> u64 {
    fnv1a(&format!(
        "{}\u{1f}{}\u{1f}{:?}\u{1f}{:?}\u{1f}{}",
        doc.title, doc.at_ms, doc.book_id, doc.book_upto, doc.text
    ))
}

impl AppIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Zahl der lebenden Stücke.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    pub fn doc_count(&self) -> usize {
        self.docs.len()
    }

    pub fn avg_len(&self) -> f64 {
        if self.live == 0 {
            0.0
        } else {
            self.total_len as f64 / self.live as f64
        }
    }

    pub fn df(&self, term: &str) -> u32 {
        self.postings.get(term).map_or(0, |p| p.len() as u32)
    }

    /// Alle lebenden Stücke.
    pub fn slots(&self) -> impl Iterator<Item = (usize, &Stored)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i, s)))
    }

    pub fn get(&self, slot: usize) -> Option<&Stored> {
        self.slots.get(slot).and_then(Option::as_ref)
    }

    pub fn postings(&self, term: &str) -> Option<&HashMap<usize, u32>> {
        self.postings.get(term)
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Hash eines Dokuments im Index (Quellversion), `None` wenn es fehlt.
    pub fn version(&self, key: &DocKey) -> Option<u64> {
        self.docs.get(key).map(|m| m.hash)
    }

    /// Entfernt alle Dokumente einer Art (Bereich widerrufen) und leert den
    /// Index vollständig, wenn `kind` `None` ist (Funktion ausgeschaltet).
    pub fn revoke(&mut self, kind: Option<SourceKind>) {
        let keys: Vec<DocKey> = self
            .docs
            .keys()
            .filter(|k| kind.is_none_or(|kind| k.kind == kind))
            .cloned()
            .collect();
        for k in keys {
            self.remove(&k);
        }
        if kind.is_none() {
            *self = AppIndex {
                epoch: self.epoch.wrapping_add(1),
                ..AppIndex::default()
            };
        } else {
            self.epoch = self.epoch.wrapping_add(1);
        }
    }

    /// Gibt es Dokumente dieser Art im Index?
    pub fn has_kind(&self, kind: SourceKind) -> bool {
        self.docs.keys().any(|k| k.kind == kind)
    }

    pub fn contains(&self, key: &DocKey) -> bool {
        self.docs.contains_key(key)
    }

    /// Fügt ein Dokument ein oder ersetzt es. `false`, wenn es unverändert war.
    /// Ein `restricted`-Dokument wird nie aufgenommen; war es vorher drin
    /// (Umklassifizierung), wird es entfernt.
    pub fn upsert(&mut self, doc: &ContextDoc) -> bool {
        let key = doc.key();
        // `restricted` nie; zwingend buchbezogene Quellen ohne Buchbezug auch
        // nicht (Spoilerschutz ließe sich sonst umgehen, Codex P1 Must-fix 1).
        if doc.restricted || (doc.kind.requires_book() && doc.book_id.is_none()) {
            let had = self.docs.contains_key(&key);
            self.remove(&key);
            return had;
        }
        let hash = doc_hash(doc);
        if self.docs.get(&key).is_some_and(|m| m.hash == hash) {
            return false;
        }
        self.remove(&key);
        let mut slots = Vec::new();
        for s in stored_pieces(doc, hash) {
            slots.push(self.insert(s));
        }
        self.docs.insert(
            key,
            DocMeta {
                hash,
                pieces: slots,
            },
        );
        true
    }

    /// Entfernt ein Dokument (keine Wirkung, wenn es fehlt).
    pub fn remove(&mut self, key: &DocKey) {
        let Some(meta) = self.docs.remove(key) else {
            return;
        };
        self.epoch = self.epoch.wrapping_add(1);
        for slot in meta.pieces {
            let Some(s) = self.slots[slot].take() else {
                continue;
            };
            for term in s.tf.keys() {
                if let Some(p) = self.postings.get_mut(term) {
                    p.remove(&slot);
                    if p.is_empty() {
                        self.postings.remove(term);
                    }
                }
            }
            self.total_len -= u64::from(s.len);
            self.live -= 1;
            self.free.push(slot);
        }
    }

    /// Gleicht eine ganze Quelle ab: fügt ein/ersetzt `docs` und entfernt
    /// Dokumente dieser Art, die nicht mehr vorkommen. Liefert die Zahl der
    /// neu zerlegten Dokumente.
    pub fn sync_kind(&mut self, kind: SourceKind, docs: &[ContextDoc]) -> usize {
        let keep: HashSet<DocKey> = docs.iter().map(ContextDoc::key).collect();
        let stale: Vec<DocKey> = self
            .docs
            .keys()
            .filter(|k| k.kind == kind && !keep.contains(*k))
            .cloned()
            .collect();
        for k in stale {
            self.remove(&k);
        }
        docs.iter()
            .filter(|d| d.kind == kind)
            .filter(|d| self.upsert(d))
            .count()
    }

    fn insert(&mut self, s: Stored) -> usize {
        let slot = match self.free.pop() {
            Some(i) => i,
            None => {
                self.slots.push(None);
                self.slots.len() - 1
            }
        };
        for (term, f) in &s.tf {
            self.postings
                .entry(term.clone())
                .or_default()
                .insert(slot, *f);
        }
        self.total_len += u64::from(s.len);
        self.live += 1;
        self.slots[slot] = Some(s);
        slot
    }
}

fn stored_pieces(doc: &ContextDoc, version: u64) -> Vec<Stored> {
    let base = |text: String,
                start: usize,
                end: usize,
                chapter: Option<String>,
                page: Option<u32>,
                book_chunk: Option<BookChunk>| {
        let (tf, len) = term_freq(&text);
        Stored {
            doc: doc.key(),
            title: doc.title.clone(),
            at_ms: doc.at_ms,
            book_id: doc.book_id.clone(),
            chapter,
            page,
            start,
            end,
            text,
            book_chunk,
            tf,
            len,
            book_upto: doc.book_upto,
            version,
        }
    };
    if doc.kind == SourceKind::Book {
        return build_book(&doc.title, &doc.text)
            .chunks
            .into_iter()
            .map(|c| {
                let chapter = (!c.chapter_title.is_empty()).then(|| c.chapter_title.clone());
                base(c.text.clone(), c.start, c.end, chapter, c.page, Some(c))
            })
            .collect();
    }
    chunk::pieces(&doc.text)
        .into_iter()
        .map(|p| {
            let end = p.start + p.text.chars().count();
            base(p.text, p.start, end, None, None, None)
        })
        .collect()
}
