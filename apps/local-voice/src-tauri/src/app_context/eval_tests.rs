//! Eval-Tor des globalen Abrufindex (Plan M4) gegen die synthetischen
//! Fixtures aus `tooling/fixtures/app-context/generate.py`.
//!
//! Harte Grenzen: 0 Spoiler, 0 Widerrufslecks, 0 befolgbare Injektionen
//! (Block bleibt geschlossen), 0 Treffer bei unpassenden Fragen. Abruf:
//! erwartete Quelle unter den ersten `DEFAULT_K` Treffern bei mindestens 95 %.
//! Antworttreue und Belegdeckung mit echtem Modell laufen getrennt auf dem PC.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::index::AppIndex;
use super::model::{ContextDoc, SourceKind};
use super::prompt::context_message;
use super::retrieve::{retrieve, Query, DEFAULT_K, DEFAULT_MAX_CHARS, MIN_COVERAGE};

const CORPUS: &str = include_str!("../../../../../tooling/fixtures/app-context/corpus.json");
const QUESTIONS: &str = include_str!("../../../../../tooling/fixtures/app-context/questions.json");
const NOW: i64 = 1_792_000_000_000;

fn kind(v: &Value) -> SourceKind {
    serde_json::from_value(v.clone()).expect("Quellart")
}

fn load() -> (AppIndex, HashMap<String, usize>, Vec<Value>) {
    let corpus: Value = serde_json::from_str(CORPUS).expect("corpus.json");
    let mut idx = AppIndex::new();
    for d in corpus["documents"].as_array().unwrap() {
        let doc = ContextDoc {
            kind: kind(&d["kind"]),
            id: d["id"].as_str().unwrap().into(),
            title: d["title"].as_str().unwrap().into(),
            at_ms: d["at_ms"].as_i64().unwrap(),
            text: d["text"].as_str().unwrap().into(),
            book_id: d["book_id"].as_str().map(Into::into),
            book_upto: d["book_upto"].as_u64().map(|n| n as usize),
            restricted: false,
        };
        assert!(idx.upsert(&doc), "Dokument {} nicht aufgenommen", doc.id);
    }
    let frontiers = corpus["frontiers"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_u64().unwrap() as usize))
        .collect();
    let questions: Value = serde_json::from_str(QUESTIONS).expect("questions.json");
    (
        idx,
        frontiers,
        questions["questions"].as_array().unwrap().clone(),
    )
}

fn query<'a>(
    question: &'a str,
    allowed: &'a HashSet<SourceKind>,
    frontiers: &'a HashMap<String, usize>,
    whole: &'a HashSet<String>,
) -> Query<'a> {
    Query {
        question,
        previous: None,
        allowed,
        frontiers,
        whole_books: whole,
        k: DEFAULT_K,
        max_chars: DEFAULT_MAX_CHARS,
        now_ms: NOW,
        min_coverage: MIN_COVERAGE,
    }
}

fn of(questions: &[Value], cat: &str) -> Vec<Value> {
    questions
        .iter()
        .filter(|q| q["category"] == cat)
        .cloned()
        .collect()
}

#[test]
fn eval_retrieval_trifft_erwartete_quelle() {
    let (idx, frontiers, questions) = load();
    let all: HashSet<SourceKind> = SourceKind::ALL.into_iter().collect();
    let whole = HashSet::new();
    let cases = of(&questions, "retrieval");
    assert!(cases.len() >= 30);
    let mut misses = Vec::new();
    for q in &cases {
        let text = q["question"].as_str().unwrap();
        let want_kind = kind(&q["expect"]["kind"]);
        let want_id = q["expect"]["id"].as_str().unwrap();
        let hits = retrieve(&idx, &query(text, &all, &frontiers, &whole));
        if !hits
            .hits
            .iter()
            .any(|h| h.jump.kind == want_kind && h.jump.id == want_id)
        {
            misses.push(text.to_string());
        }
    }
    let rate = 1.0 - misses.len() as f64 / cases.len() as f64;
    assert!(
        rate >= 0.95,
        "Treffer@{DEFAULT_K} {rate:.2}, verfehlt: {misses:?}"
    );
}

#[test]
fn eval_null_spoiler_auch_aus_abgeleiteten_inhalten() {
    let (idx, frontiers, questions) = load();
    let all: HashSet<SourceKind> = SourceKind::ALL.into_iter().collect();
    let whole = HashSet::new();
    let cases = of(&questions, "spoiler");
    assert_eq!(cases.len(), 3);
    for q in &cases {
        let text = q["question"].as_str().unwrap();
        let hits = retrieve(&idx, &query(text, &all, &frontiers, &whole));
        let msg = context_message(&hits.hits, true)
            .unwrap_or_default()
            .to_lowercase();
        for term in q["forbidden"].as_array().unwrap() {
            let term = term.as_str().unwrap().to_lowercase();
            assert!(!msg.contains(&term), "Spoiler „{term}“ bei „{text}“");
        }
        // Gegenprobe: mit Ganzes-Buch-Freigabe ist das Ende erreichbar.
        let book = q["book"].as_str().unwrap().to_string();
        let whole_one: HashSet<String> = [book].into_iter().collect();
        let open = retrieve(&idx, &query(text, &all, &frontiers, &whole_one));
        let msg = context_message(&open.hits, true)
            .unwrap_or_default()
            .to_lowercase();
        let first = q["forbidden"][0].as_str().unwrap().to_lowercase();
        assert!(
            msg.contains(&first),
            "Ganzes Buch frei, aber „{first}“ fehlt bei „{text}“"
        );
    }
}

#[test]
fn eval_null_widerrufslecks() {
    let (mut idx, frontiers, questions) = load();
    let all: HashSet<SourceKind> = SourceKind::ALL.into_iter().collect();
    let whole = HashSet::new();
    let cases = of(&questions, "revocation");
    assert_eq!(cases.len(), 10);
    // Vor dem Widerruf finden die Fragen Besprechungen.
    let before = cases
        .iter()
        .filter(|q| {
            let t = q["question"].as_str().unwrap();
            retrieve(&idx, &query(t, &all, &frontiers, &whole))
                .hits
                .iter()
                .any(|h| h.kind == SourceKind::Meeting)
        })
        .count();
    assert!(
        before >= 9,
        "nur {before}/10 Besprechungen vor dem Widerruf"
    );
    let epoch = idx.epoch();
    idx.revoke(Some(SourceKind::Meeting));
    assert!(idx.epoch() > epoch, "Widerruf muss den Stand erhöhen");
    assert!(!idx.has_kind(SourceKind::Meeting));
    // Nach dem Widerruf: weder aus dem Index noch über die Freigabeliste.
    let without: HashSet<SourceKind> = all
        .iter()
        .copied()
        .filter(|k| *k != SourceKind::Meeting)
        .collect();
    for q in &cases {
        let t = q["question"].as_str().unwrap();
        for allowed in [&all, &without] {
            let hits = retrieve(&idx, &query(t, allowed, &frontiers, &whole));
            assert!(
                hits.hits.iter().all(|h| h.kind != SourceKind::Meeting),
                "Widerrufsleck bei „{t}“"
            );
        }
    }
}

#[test]
fn eval_injektion_bleibt_daten() {
    let (idx, frontiers, questions) = load();
    let all: HashSet<SourceKind> = SourceKind::ALL.into_iter().collect();
    let whole = HashSet::new();
    let cases = of(&questions, "injection");
    assert_eq!(cases.len(), 2);
    for q in &cases {
        let t = q["question"].as_str().unwrap();
        let hits = retrieve(&idx, &query(t, &all, &frontiers, &whole));
        assert!(
            hits.hits.iter().any(|h| h.jump.id == "page_injection"),
            "Injektionsdokument nicht gefunden bei „{t}“"
        );
        let msg = context_message(&hits.hits, true).unwrap();
        let blocks = hits.hits.len();
        // Genau ein öffnender und ein schließender Begrenzer je Treffer.
        assert_eq!(
            msg.matches("<<<").count(),
            blocks + 1,
            "Einleitung nennt <<< einmal"
        );
        assert_eq!(
            msg.matches("\n>>>").count(),
            blocks,
            "Block durch Inhalt geschlossen"
        );
        // Keine gefälschte Belegmarke aus dem Inhalt.
        assert!(!msg.contains("[Q9]"), "Fremde Belegmarke durchgereicht");
        // Kopfzeilen außerhalb der Blöcke tragen keinen Fremdtext.
        for line in msg.lines().filter(|l| l.starts_with("[Q")) {
            assert!(!line.contains("SYSTEM"), "Fremdtext in Kopfzeile: {line}");
        }
    }
}

#[test]
fn eval_unpassende_fragen_ohne_treffer() {
    let (idx, frontiers, questions) = load();
    let all: HashSet<SourceKind> = SourceKind::ALL.into_iter().collect();
    let whole = HashSet::new();
    let cases = of(&questions, "no_hit");
    assert_eq!(cases.len(), 4);
    for q in &cases {
        let t = q["question"].as_str().unwrap();
        let hits = retrieve(&idx, &query(t, &all, &frontiers, &whole));
        assert!(
            hits.hits.is_empty(),
            "Treffer bei unpassender Frage „{t}“: {:?}",
            hits.hits.iter().map(|h| &h.title).collect::<Vec<_>>()
        );
        assert!(context_message(&hits.hits, true).is_none());
    }
}
