//! Lesegrenze aus dem gespeicherten Fortschritt (rein). Gleiche Regel wie
//! `src/lib/library/frontier.ts` (`frontierOf`): das Größte aus gespeicherter
//! Grenze und dem Ende der gehörten und gelesenen Bereiche. Die aktuelle
//! Stelle (`offset`) zählt nicht.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

/// Schlüssel des Textfortschritts einer Seite (`textProgressKey`).
pub fn text_key(page_id: &str) -> String {
    format!("text:{page_id}:original")
}

fn finite(v: &Value) -> Option<usize> {
    v.as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .map(|n| n as usize)
}

/// Grenze eines Datensatzes (JSON-Text); 0, wenn nichts lesbar ist.
pub fn frontier_of(raw: &str) -> usize {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return 0;
    };
    let mut best = v.get("frontier").and_then(finite).unwrap_or(0);
    for field in ["heard", "read"] {
        for range in v.get(field).and_then(Value::as_array).into_iter().flatten() {
            if let Some(end) = range.get(1).and_then(finite) {
                best = best.max(end);
            }
        }
    }
    best
}

/// Grenzen je Seite für die gegebenen Seiten. Fehlt ein Fortschritt oder ist
/// er 0, fehlt die Seite in der Karte (dann ist vom Buch nichts sichtbar).
pub fn frontiers(progress: &BTreeMap<String, String>, pages: &[String]) -> HashMap<String, usize> {
    pages
        .iter()
        .filter_map(|p| {
            let f = progress.get(&text_key(p)).map(|r| frontier_of(r))?;
            (f > 0).then(|| (p.clone(), f))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontier_is_the_furthest_heard_or_read_end() {
        assert_eq!(
            frontier_of(r#"{"offset":900,"heard":[[0,120]],"read":[[100,300]]}"#),
            300
        );
        assert_eq!(frontier_of(r#"{"frontier":500,"heard":[[0,120]]}"#), 500);
        assert_eq!(
            frontier_of(r#"{"offset":900}"#),
            0,
            "die Stelle allein zählt nicht"
        );
        assert_eq!(frontier_of("kaputt"), 0);
        assert_eq!(frontier_of(r#"{"heard":[[0,-5]],"frontier":"x"}"#), 0);
    }

    #[test]
    fn pages_without_progress_have_no_frontier() {
        let mut m = BTreeMap::new();
        m.insert(text_key("a"), r#"{"read":[[0,40]]}"#.to_string());
        m.insert(text_key("b"), r#"{"offset":10}"#.to_string());
        let f = frontiers(&m, &["a".into(), "b".into(), "c".into()]);
        assert_eq!(f.get("a"), Some(&40));
        assert!(!f.contains_key("b") && !f.contains_key("c"));
    }
}
