//! Hybride Suche (M4 §4 und §6): Wort-FTS und Vektorsuche, verschmolzen per
//! Reciprocal Rank Fusion (k = 60). Funktioniert auch ohne Vektoren: fehlt das
//! Embedding-Modell, laeuft der Server nicht oder ist der Index leer, kommt die
//! reine Wortsuche zurueck und `lexical_only` ist gesetzt (der Chat sagt das in
//! der Abdeckungsnotiz).
//!
//! `Embedder` und seine Enums liegen seit P4b in `embed.rs`; hier bleiben sie
//! per `pub use` erreichbar (bestehende Aufrufer, P4c).

use anyhow::Result;
#[cfg(test)]
use futures_util::future::BoxFuture;
use std::collections::{HashMap, HashSet};

use super::super::store::MeetingStore;
use super::chunking::fts_query_words;
use super::vectors::{global_cache, VectorCache, COARSE_CANDIDATES};
// Alter Pfad fuer P4c (`hybrid::EmbedError` ...), auch wenn hier nicht alles gebraucht wird.
#[allow(unused_imports)]
pub use super::embed::{EmbedError, EmbedKind, Embedder};

/// RRF-Konstante (Cormack et al.): dampft den Vorsprung der obersten Raenge.
pub const RRF_K: f64 = 60.0;
/// Kandidaten je Liste vor der Fusion (M4 §6.2).
pub const WORD_LIMIT: u32 = 100;
pub const VECTOR_LIMIT: usize = 100;

// ---------------------------------------------------------------------------
// RRF
// ---------------------------------------------------------------------------

/// Reciprocal Rank Fusion ueber beliebig viele Ranglisten (beste zuerst; die
/// Zahl in jedem Paar wird ignoriert, nur die Reihenfolge zaehlt). Punkte:
/// Summe `1 / (k + Rang)` mit Rang ab 1; ein Eintrag, der in einer Liste
/// mehrfach steht, zaehlt dort einmal (mit seinem besten Rang).
///
/// Gleichstand ist deterministisch: bester Rang in irgendeiner Liste, dann die
/// frueheste Liste (Wort vor Vektor), dann die kleinere ID. Punkte werden auf
/// 1e-12 gerundet verglichen, damit Rundungsrauschen beim Aufsummieren keinen
/// Gleichstand kippt.
pub fn rrf(lists: &[Vec<(i64, f64)>], k: f64, top: usize) -> Vec<(i64, f64)> {
    struct Acc {
        score: f64,
        best_rank: usize,
        best_list: usize,
    }
    let k = k.max(0.0);
    let mut acc: HashMap<i64, Acc> = HashMap::new();
    for (list_ix, list) in lists.iter().enumerate() {
        let mut seen: HashSet<i64> = HashSet::new();
        let mut rank = 0usize;
        for (id, _) in list {
            if !seen.insert(*id) {
                continue;
            }
            rank += 1;
            let entry = acc.entry(*id).or_insert(Acc {
                score: 0.0,
                best_rank: rank,
                best_list: list_ix,
            });
            entry.score += 1.0 / (k + rank as f64);
            if (rank, list_ix) < (entry.best_rank, entry.best_list) {
                entry.best_rank = rank;
                entry.best_list = list_ix;
            }
        }
    }
    let mut out: Vec<(i64, Acc)> = acc.into_iter().collect();
    out.sort_by_key(|(id, a)| {
        (
            std::cmp::Reverse((a.score * 1e12).round() as i64),
            a.best_rank,
            a.best_list,
            *id,
        )
    });
    out.into_iter()
        .take(top)
        .map(|(id, a)| (id, a.score))
        .collect()
}

// ---------------------------------------------------------------------------
// Hybride Suche
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct HybridResult {
    /// `(Chunk-ID, RRF-Punkte)`, beste zuerst.
    pub hits: Vec<(i64, f64)>,
    /// Die Vektorstufe hat nichts beigetragen (kein Modell, Fehler, leerer Index).
    pub lexical_only: bool,
}

/// Der synchrone Kern: Abfragevektor liegt schon vor (`None` = keiner). Der
/// Bench misst genau das (ohne Embedding-Aufruf). Vektorfehler (Index zu
/// gross, Datenbankfehler beim Nachbewerten) machen die Suche NICHT kaputt,
/// sie fallen auf die Wortsuche zurueck.
pub fn hybrid_search_with_vector(
    store: &MeetingStore,
    cache: &VectorCache,
    model: &str,
    query: &str,
    qvec: Option<&[f32]>,
    scope_ids: &[String],
    top: usize,
) -> Result<HybridResult> {
    if scope_ids.is_empty() || top == 0 {
        return Ok(HybridResult {
            hits: Vec::new(),
            lexical_only: qvec.is_none(),
        });
    }
    let words = match fts_query_words(query) {
        Some(fts) => store.search_words(&fts, scope_ids, WORD_LIMIT)?,
        None => Vec::new(),
    };
    let mut lists = vec![words];
    let mut lexical_only = true;
    if let Some(q) = qvec {
        let vector_list =
            cache.with_index(store, model, |index| -> Result<Option<Vec<(i64, f64)>>> {
                if index.is_empty() || index.dim() != q.len() {
                    return Ok(None);
                }
                let mask = index.scope_mask(scope_ids);
                let coarse = index.topk(q, COARSE_CANDIDATES, Some(&mask));
                let exact = index.rescore(store, q, &coarse, VECTOR_LIMIT)?;
                Ok(Some(
                    exact
                        .into_iter()
                        .map(|(id, s)| (id, f64::from(s)))
                        .collect(),
                ))
            });
        match vector_list {
            // Eine leere Vektorliste (Scope ohne Vektoren, Nullvektor) traegt nichts bei:
            // die Antwort ist dann ehrlich "nur Stichwortsuche".
            Ok(Ok(Some(list))) if !list.is_empty() => {
                lists.push(list);
                lexical_only = false;
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) | Err(e) => log::warn!("Vektorsuche uebersprungen: {e}"),
        }
    }
    Ok(HybridResult {
        hits: rrf(&lists, RRF_K, top),
        lexical_only,
    })
}

/// Hybride Suche mit Embedding der Anfrage. Ein Fehler des Embedders
/// (`NoModel`, `MemoryLow`, `Busy`, `Failed`) ist kein Fehler der Suche: es
/// gibt Wortergebnisse und `lexical_only = true`.
pub async fn hybrid_search(
    store: &MeetingStore,
    embed: &dyn Embedder,
    query: &str,
    scope_ids: &[String],
    top: usize,
) -> Result<HybridResult> {
    let mut qvec: Option<Vec<f32>> = None;
    if !query.trim().is_empty() {
        match embed.embed(&[query.to_string()], EmbedKind::Query).await {
            Ok(mut vectors) if vectors.len() == 1 => qvec = vectors.pop(),
            Ok(_) => log::warn!("Embedding: unerwartete Anzahl Vektoren"),
            Err(e) => log::info!("Embedding nicht verfuegbar ({}), nur Wortsuche", e.code()),
        }
    }
    hybrid_search_with_vector(
        store,
        global_cache(),
        embed.model_id(),
        query,
        qvec.as_deref(),
        scope_ids,
        top,
    )
}

#[cfg(test)]
mod tests {
    use super::super::chunking::ChunkSource;
    use super::super::index::tests::{draft, ready_meeting, state, tmp_store};
    use super::super::index::STATUS_LEXICAL;
    use super::*;
    use crate::managers::meetings::store::Meeting;

    fn ids(hits: &[(i64, f64)]) -> Vec<i64> {
        hits.iter().map(|(id, _)| *id).collect()
    }

    fn list(ids: &[i64]) -> Vec<(i64, f64)> {
        ids.iter().map(|id| (*id, 0.0)).collect()
    }

    #[test]
    fn a_single_list_keeps_its_order_and_scores_by_rank() {
        let fused = rrf(&[list(&[7, 3, 9])], RRF_K, 10);
        assert_eq!(ids(&fused), vec![7, 3, 9]);
        assert!((fused[0].1 - 1.0 / 61.0).abs() < 1e-12);
        assert!((fused[2].1 - 1.0 / 63.0).abs() < 1e-12);
        assert!(rrf(&[], RRF_K, 10).is_empty());
        assert!(rrf(&[vec![]], RRF_K, 10).is_empty());
        assert!(rrf(&[list(&[1])], RRF_K, 0).is_empty());
    }

    #[test]
    fn an_entry_in_both_lists_beats_two_solo_entries_of_the_same_rank() {
        // 5 ist in beiden Listen Zweiter; 1 und 2 sind je in einer Erster.
        let fused = rrf(&[list(&[1, 5]), list(&[2, 5])], RRF_K, 10);
        assert_eq!(fused[0].0, 5, "2 * 1/62 > 1/61");
        assert_eq!(ids(&fused), vec![5, 1, 2]);
    }

    #[test]
    fn a_tie_is_broken_by_best_rank_then_list_then_id() {
        // Gleiche Punkte: 10 (Liste 0, Rang 1) und 20 (Liste 1, Rang 1).
        let fused = rrf(&[list(&[10]), list(&[20])], RRF_K, 10);
        assert_eq!(ids(&fused), vec![10, 20], "fruehere Liste zuerst");
        // Dieselbe ID-Reihenfolge unabhaengig von der Einfuegung.
        let fused = rrf(&[list(&[20]), list(&[10])], RRF_K, 10);
        assert_eq!(ids(&fused), vec![20, 10]);
        // Ganz gleich (gleicher Rang, gleiche Liste geht nicht): Rang 1 in Liste 0 vs Rang 2+Rang 2.
        let fused = rrf(&[list(&[1, 9]), list(&[2, 9]), list(&[3, 9])], RRF_K, 10);
        assert_eq!(fused[0].0, 9);
        assert_eq!(ids(&fused[1..]), vec![1, 2, 3]);
        // Reihenfolge ist stabil ueber Wiederholungen (HashMap-Reihenfolge spielt keine Rolle).
        let first = rrf(&[list(&[4, 8, 15]), list(&[16, 23, 42])], RRF_K, 10);
        for _ in 0..20 {
            assert_eq!(
                rrf(&[list(&[4, 8, 15]), list(&[16, 23, 42])], RRF_K, 10),
                first
            );
        }
    }

    #[test]
    fn duplicates_inside_a_list_count_once_and_top_truncates() {
        let fused = rrf(&[list(&[1, 1, 1, 2])], RRF_K, 10);
        assert_eq!(ids(&fused), vec![1, 2]);
        assert!(
            (fused[1].1 - 1.0 / 62.0).abs() < 1e-12,
            "2 ist Rang 2, nicht 4"
        );
        let fused = rrf(&[list(&[1, 2, 3, 4, 5])], RRF_K, 3);
        assert_eq!(ids(&fused), vec![1, 2, 3]);
    }

    // ---- hybride Suche mit Fake-Embedder ---------------------------------

    const MODEL: &str = "fake-emb";

    struct FakeEmbedder {
        model: String,
        outcome: Result<Vec<f32>, EmbedError>,
    }

    impl Embedder for FakeEmbedder {
        fn embed<'a>(
            &'a self,
            texts: &'a [String],
            kind: EmbedKind,
        ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>> {
            Box::pin(async move {
                assert_eq!(kind, EmbedKind::Query);
                self.outcome
                    .clone()
                    .map(|v| texts.iter().map(|_| v.clone()).collect())
            })
        }

        fn model_id(&self) -> &str {
            &self.model
        }
    }

    fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(fut)
    }

    struct World {
        _dir: tempfile::TempDir,
        store: MeetingStore,
        a: Meeting,
        b: Meeting,
        c: Meeting,
        /// a0 lexikalisch + semantisch, b0 nur semantisch, c0 nur lexikalisch.
        a0: i64,
        b0: i64,
        c0: i64,
    }

    fn world() -> World {
        let (dir, store) = tmp_store();
        let a = ready_meeting(&store, "A", 1_000);
        let b = ready_meeting(&store, "B", 2_000);
        let c = ready_meeting(&store, "C", 3_000);
        let put = |m: &Meeting, rows: &[(&str, [f32; 4])]| -> Vec<i64> {
            let drafts: Vec<_> = rows
                .iter()
                .map(|(t, _)| draft(ChunkSource::Transcript, t))
                .collect();
            let ids = store
                .replace_meeting_chunks(
                    &m.id,
                    &[ChunkSource::Transcript],
                    &drafts,
                    &state(STATUS_LEXICAL),
                )
                .unwrap();
            let vectors: Vec<(i64, Vec<f32>)> = ids
                .iter()
                .zip(rows)
                .map(|(id, (_, v))| (*id, v.to_vec()))
                .collect();
            store.put_vectors(MODEL, &vectors).unwrap();
            ids
        };
        let ia = put(
            &a,
            &[
                ("Das Budget wurde genehmigt", [1.0, 0.0, 0.0, 0.0]),
                ("Urlaub im Sommer", [0.0, 1.0, 0.0, 0.0]),
            ],
        );
        let ib = put(
            &b,
            &[
                ("Kosten und Finanzen im Blick", [0.9, 0.1, 0.0, 0.0]),
                ("Wetter morgen", [0.0, 0.0, 1.0, 0.0]),
            ],
        );
        let ic = put(&c, &[("Budget Budget Planung", [0.0, 0.0, 0.0, 1.0])]);
        World {
            _dir: dir,
            store,
            a,
            b,
            c,
            a0: ia[0],
            b0: ib[0],
            c0: ic[0],
        }
    }

    fn all_scope(w: &World) -> Vec<String> {
        vec![w.a.id.clone(), w.b.id.clone(), w.c.id.clone()]
    }

    fn embedder(vec: Result<Vec<f32>, EmbedError>) -> FakeEmbedder {
        FakeEmbedder {
            model: MODEL.into(),
            outcome: vec,
        }
    }

    #[test]
    fn hybrid_merges_word_and_vector_hits() {
        let w = world();
        let emb = embedder(Ok(vec![1.0, 0.0, 0.0, 0.0]));
        let res = block_on(hybrid_search(&w.store, &emb, "Budget", &all_scope(&w), 10)).unwrap();
        assert!(!res.lexical_only);
        let got = ids(&res.hits);
        assert_eq!(got[0], w.a0, "in beiden Listen ganz oben");
        assert!(
            got.contains(&w.b0),
            "nur semantisch gefunden (Kosten/Finanzen)"
        );
        assert!(got.contains(&w.c0), "nur lexikalisch gefunden");
        assert!(res.hits.windows(2).all(|p| p[0].1 >= p[1].1));
    }

    #[test]
    fn hybrid_respects_the_scope_in_both_lists() {
        let w = world();
        let emb = embedder(Ok(vec![1.0, 0.0, 0.0, 0.0]));
        let only_b = vec![w.b.id.clone()];
        let res = block_on(hybrid_search(&w.store, &emb, "Budget", &only_b, 10)).unwrap();
        assert!(!res.lexical_only);
        assert_eq!(res.hits[0].0, w.b0);
        let rows = w.store.get_chunks(&ids(&res.hits)).unwrap();
        assert!(rows.iter().all(|r| r.meeting_id == w.b.id));

        let none = block_on(hybrid_search(&w.store, &emb, "Budget", &[], 10)).unwrap();
        assert!(none.hits.is_empty());
        let top0 = block_on(hybrid_search(&w.store, &emb, "Budget", &all_scope(&w), 0)).unwrap();
        assert!(top0.hits.is_empty());
    }

    #[test]
    fn a_failing_embedder_degrades_to_word_search_instead_of_failing() {
        let w = world();
        for err in [
            EmbedError::NoModel,
            EmbedError::MemoryLow,
            EmbedError::Busy,
            EmbedError::Failed("Server abgestuerzt".into()),
        ] {
            let emb = embedder(Err(err));
            let res =
                block_on(hybrid_search(&w.store, &emb, "Budget", &all_scope(&w), 10)).unwrap();
            assert!(res.lexical_only);
            let got = ids(&res.hits);
            assert!(got.contains(&w.a0) && got.contains(&w.c0));
            assert!(
                !got.contains(&w.b0),
                "ohne Vektoren kein semantischer Treffer"
            );
        }
        assert_eq!(EmbedError::Failed("x".into()).code(), "failed");
    }

    #[test]
    fn a_model_without_vectors_or_with_another_dimension_is_lexical_only() {
        let w = world();
        // Anderes Modell: der Index dafuer ist leer.
        let other = FakeEmbedder {
            model: "anderes-modell".into(),
            outcome: Ok(vec![1.0, 0.0, 0.0, 0.0]),
        };
        let res = block_on(hybrid_search(
            &w.store,
            &other,
            "Budget",
            &all_scope(&w),
            10,
        ))
        .unwrap();
        assert!(res.lexical_only);
        assert!(!res.hits.is_empty());
        // Abfragevektor mit falscher Laenge.
        let wrong = embedder(Ok(vec![1.0, 0.0]));
        let res = block_on(hybrid_search(
            &w.store,
            &wrong,
            "Budget",
            &all_scope(&w),
            10,
        ))
        .unwrap();
        assert!(res.lexical_only);
        // Vektor-Nullvektor: keine Vektortreffer, aber kein Fehler.
        let zero = embedder(Ok(vec![0.0; 4]));
        let res = block_on(hybrid_search(&w.store, &zero, "Budget", &all_scope(&w), 10)).unwrap();
        assert!(!res.hits.is_empty());
        assert!(
            res.lexical_only,
            "leere Vektorliste = ehrlich nur Stichwortsuche"
        );
    }

    #[test]
    fn a_query_of_only_stopwords_still_searches_by_vector() {
        let w = world();
        let emb = embedder(Ok(vec![0.0, 0.0, 1.0, 0.0]));
        let res = block_on(hybrid_search(
            &w.store,
            &emb,
            "und der die",
            &all_scope(&w),
            3,
        ))
        .unwrap();
        assert!(!res.lexical_only);
        let rows = w.store.get_chunks(&ids(&res.hits)).unwrap();
        assert_eq!(rows[0].text, "Wetter morgen", "nur die Vektorliste traegt");
        // Leere Anfrage: kein Embedding-Aufruf, keine Treffer.
        let res = block_on(hybrid_search(&w.store, &emb, "   ", &all_scope(&w), 3)).unwrap();
        assert!(res.hits.is_empty());
        assert!(res.lexical_only);
    }

    #[test]
    fn a_stale_vector_index_never_returns_deleted_meetings() {
        let w = world();
        let cache = VectorCache::new();
        let q = [1.0, 0.0, 0.0, 0.0];
        let scope = all_scope(&w);
        let before =
            hybrid_search_with_vector(&w.store, &cache, MODEL, "Budget", Some(&q), &scope, 10)
                .unwrap();
        assert!(ids(&before.hits).contains(&w.a0));
        // A wird geloescht; der Cache erfaehrt es nicht (kein `update`).
        w.store.soft_delete_meeting(&w.a.id).unwrap();
        let after =
            hybrid_search_with_vector(&w.store, &cache, MODEL, "Budget", Some(&q), &scope, 10)
                .unwrap();
        assert!(!ids(&after.hits).contains(&w.a0));
        assert!(ids(&after.hits).contains(&w.b0));
    }

    #[test]
    fn a_broken_vector_table_falls_back_to_words_instead_of_failing() {
        let w = world();
        let cache = VectorCache::new();
        w.store
            .get_connection()
            .unwrap()
            .execute_batch("DROP TABLE meeting_chunk_vectors;")
            .unwrap();
        let res = hybrid_search_with_vector(
            &w.store,
            &cache,
            MODEL,
            "Budget",
            Some(&[1.0, 0.0, 0.0, 0.0]),
            &all_scope(&w),
            10,
        )
        .unwrap();
        assert!(res.lexical_only);
        assert!(!res.hits.is_empty(), "Wortsuche laeuft weiter");
    }
}
