//! G5: der ganze Ablauf `translate_variant` mit einem Mock-Server statt des Modells:
//! Fassung, Herkunft, Pruefbericht, und was bei Fehler, Stopp und fehlendem Modell NICHT
//! entsteht. Die Bausteine (Treuepruefung, Bloecke) stehen in `tests.rs`.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::managers::integrations::test_support::Fx;
use crate::managers::meetings::llm_call::test_support::{
    settings_with_mock_provider, spawn_llm_mock_with, MockReply,
};
use crate::managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};
use crate::managers::provenance;
use crate::managers::usage::UsageLedger;
use crate::settings::get_default_settings;

fn ensure_ledger() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        if usage::ledger().is_none() {
            let ledger = UsageLedger::open(Path::new(":memory:")).unwrap();
            usage::install_globals(Arc::new(ledger), Arc::new(get_default_settings));
        }
    });
}

fn body(content: &str, prompt: u64, completion: u64) -> String {
    json!({
        "choices": [{ "message": { "role": "assistant", "content": content } }],
        "usage": { "prompt_tokens": prompt, "completion_tokens": completion }
    })
    .to_string()
}

fn seg(i: u32, text: &str) -> StoredSegment {
    StoredSegment {
        segment_index: i,
        text: text.to_string(),
        start_ms: u64::from(i) * 4000 + 1000,
        end_ms: u64::from(i) * 4000 + 4500,
        channel: 2,
        speaker_index: Some(i % 2),
        words: None,
    }
}

/// Besprechung mit englischem Original als Fassung 1 (aktiv, Sprache `en`).
fn english_meeting(fx: &Fx) -> (Arc<MeetingStore>, String, String) {
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let meeting = store
        .create_meeting("Interview", MeetingSource::Import, None)
        .unwrap()
        .id;
    store.set_status(&meeting, MeetingStatus::Ready).unwrap();
    store
        .append_delta(
            &meeting,
            &TranscriptDelta {
                new_segments: vec![
                    seg(0, "Good morning, this is Anna from Siemens."),
                    seg(1, "We pay 1,200.50 euros every month."),
                ],
            },
        )
        .unwrap();
    let mut conn = store.get_connection().unwrap();
    variants::set_language(&mut conn, &meeting, "en").unwrap();
    let original = variants::list(&mut conn, &meeting).unwrap()[0].id.clone();
    (store, meeting, original)
}

const GOOD: &str = r#"{"lines":[{"n":1,"text":"Guten Morgen, hier ist Anna von Siemens."},{"n":2,"text":"Wir zahlen jeden Monat 1.200,50 Euro."}]}"#;
const WRONG_NUMBER: &str = r#"{"lines":[{"n":1,"text":"Guten Morgen, hier ist Anna von Siemens."},{"n":2,"text":"Wir zahlen jeden Monat 120,50 Euro."}]}"#;

fn go() -> std::future::Ready<Control> {
    std::future::ready(Control::Go)
}

#[tokio::test]
async fn a_translation_becomes_a_new_inactive_variant_with_source_model_and_tokens() {
    ensure_ledger();
    let port = spawn_llm_mock_with(|_| MockReply::Body(body(GOOD, 150, 60))).await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let (store, meeting, original) = english_meeting(&fx);
    let before = {
        let conn = store.get_connection().unwrap();
        variants::get_segments(&conn, &original).unwrap().1
    };

    let mut seen = Vec::new();
    let translated = translate_variant(&settings, &store, &meeting, &original, "de", go, |d, t| {
        seen.push((d, t))
    })
    .await
    .unwrap();

    assert_eq!(translated.kind, variants::KIND_TRANSLATION);
    assert_eq!(translated.language.as_deref(), Some("de"));
    assert_eq!(translated.source_language.as_deref(), Some("en"));
    assert_eq!(translated.source_variant_id.as_deref(), Some(original.as_str()));
    assert!(!translated.active, "das Original bleibt das aktive Transkript");
    assert_eq!(translated.flagged, 0);
    assert_eq!(seen.last(), Some(&(1, 1)));

    let mut conn = store.get_connection().unwrap();
    let list = variants::list(&mut conn, &meeting).unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().find(|v| v.id == original).unwrap().active);
    // Das Original ist unveraendert, Segment fuer Segment.
    let (_, after) = variants::get_segments(&conn, &original).unwrap();
    assert_eq!(after, before);
    // 1:1 mit Zeit und Sprecher.
    let (_, translation) = variants::get_segments(&conn, &translated.id).unwrap();
    assert_eq!(translation.len(), before.len());
    assert_eq!(translation[1].text, "Wir zahlen jeden Monat 1.200,50 Euro.");
    for (t, o) in translation.iter().zip(&before) {
        assert_eq!((t.start_ms, t.end_ms, t.speaker_index), (o.start_ms, o.end_ms, o.speaker_index));
    }

    // Herkunft: Quelle = Ausgangsfassung, Modell, Token, Ledger.
    let entries = provenance::list(&conn, SubjectKind::TranscriptVariant, &translated.id).unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.operation, "translation");
    assert_eq!(
        e.sources.iter().map(|s| s.reference.as_str()).collect::<Vec<_>>(),
        vec![original.as_str()]
    );
    assert!(e.model_id.is_some());
    assert_eq!(e.prompt_tokens, Some(150));
    assert_eq!(e.completion_tokens, Some(60));
    assert!(e.usage_event_id.is_some());
    let params: serde_json::Value = serde_json::from_str(e.params_json.as_deref().unwrap()).unwrap();
    assert_eq!(params["target_language"], "de");
    assert_eq!(params["source_language"], "en");
    assert_eq!(params["flagged"], 0);
}

#[tokio::test]
async fn a_deviating_sentence_is_marked_in_the_report_and_kept_in_the_text() {
    ensure_ledger();
    let port = spawn_llm_mock_with(|_| MockReply::Body(body(WRONG_NUMBER, 10, 5))).await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let (store, meeting, original) = english_meeting(&fx);

    let translated = translate_variant(&settings, &store, &meeting, &original, "de", go, |_, _| {})
        .await
        .unwrap();
    assert_eq!(translated.flagged, 1, "der Satz mit der falschen Zahl ist markiert");
    let conn = store.get_connection().unwrap();
    let (_, translation) = variants::get_segments(&conn, &translated.id).unwrap();
    assert_eq!(translation[1].text, "Wir zahlen jeden Monat 120,50 Euro.", "nicht still verworfen");
    let report: TranslationReport =
        serde_json::from_str(&variants::get_meta(&conn, &translated.id).unwrap().unwrap()).unwrap();
    assert_eq!(report.flagged.len(), 1);
    assert_eq!(report.flagged[0].segment_index, 1);
    assert_eq!(report.flagged[0].reasons, vec![REASON_NUMBERS.to_string()]);
    assert_eq!(report.source_variant_id, original);
}

#[tokio::test]
async fn a_server_failure_leaves_no_variant_and_the_original_untouched() {
    // Der Server stirbt (500): kein Teilergebnis.
    ensure_ledger();
    let port = spawn_llm_mock_with(|_| MockReply::Status(500)).await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let (store, meeting, original) = english_meeting(&fx);
    let err = translate_variant(&settings, &store, &meeting, &original, "de", go, |_, _| {})
        .await
        .unwrap_err();
    assert!(err.starts_with("translate_llm_failed"), "{err}");
    let mut conn = store.get_connection().unwrap();
    assert_eq!(variants::list(&mut conn, &meeting).unwrap().len(), 1);
    assert_eq!(store.get_segments(&meeting).unwrap()[0].text, "Good morning, this is Anna from Siemens.");
}

#[tokio::test]
async fn a_stop_leaves_no_variant_and_asks_the_model_nothing() {
    ensure_ledger();
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&asked);
    let port = spawn_llm_mock_with(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        MockReply::Body(body(GOOD, 1, 1))
    })
    .await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let (store, meeting, original) = english_meeting(&fx);
    let err = translate_variant(&settings, &store, &meeting, &original, "de", || std::future::ready(Control::Stop), |_, _| {})
        .await
        .unwrap_err();
    assert_eq!(err, "translate_cancelled");
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    let mut conn = store.get_connection().unwrap();
    assert_eq!(variants::list(&mut conn, &meeting).unwrap().len(), 1);
}

#[tokio::test]
async fn argument_errors_come_before_any_model_call() {
    ensure_ledger();
    let fx = Fx::new();
    let (store, meeting, original) = english_meeting(&fx);
    // Ohne Modell: der Code der Einrichtung, kein Netzzugriff.
    let defaults = get_default_settings();
    let err = translate_variant(&defaults, &store, &meeting, &original, "de", go, |_, _| {})
        .await
        .unwrap_err();
    assert!(err == "no_provider" || err == "no_model", "{err}");

    let port = spawn_llm_mock_with(|_| MockReply::Body(body(GOOD, 1, 1))).await;
    let settings = settings_with_mock_provider(port);
    // Gleiche Sprache.
    assert_eq!(
        translate_variant(&settings, &store, &meeting, &original, "en-US", go, |_, _| {})
            .await
            .unwrap_err(),
        "translate_same_language"
    );
    // Keine gueltige Zielsprache.
    assert_eq!(
        translate_variant(&settings, &store, &meeting, &original, "auto", go, |_, _| {})
            .await
            .unwrap_err(),
        "translate_same_language"
    );
    // Unbekannte Fassung, Fassung einer anderen Besprechung.
    assert_eq!(
        translate_variant(&settings, &store, &meeting, "gibt-es-nicht", "de", go, |_, _| {})
            .await
            .unwrap_err(),
        "variant_not_found"
    );
    let other = store.create_meeting("x", MeetingSource::Import, None).unwrap().id;
    assert_eq!(
        translate_variant(&settings, &store, &other, &original, "de", go, |_, _| {})
            .await
            .unwrap_err(),
        "variant_not_found"
    );
    let mut conn = store.get_connection().unwrap();
    assert_eq!(variants::list(&mut conn, &meeting).unwrap().len(), 1);
}

#[tokio::test]
async fn the_translation_can_be_translated_again_into_another_language() {
    // Quelle ist nicht nur das Original: auch eine Uebersetzung laesst sich weitergeben.
    ensure_ledger();
    let port = spawn_llm_mock_with(|request| {
        if request.contains("into French") {
            MockReply::Body(body(
                r#"{"lines":[{"n":1,"text":"Bonjour, ici Anna de Siemens."},{"n":2,"text":"Nous payons 1 200,50 euros par mois."}]}"#,
                1,
                1,
            ))
        } else {
            MockReply::Body(body(GOOD, 1, 1))
        }
    })
    .await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let (store, meeting, original) = english_meeting(&fx);
    let german = translate_variant(&settings, &store, &meeting, &original, "de", go, |_, _| {})
        .await
        .unwrap();
    let french = translate_variant(&settings, &store, &meeting, &german.id, "fr", go, |_, _| {})
        .await
        .unwrap();
    assert_eq!(french.source_variant_id.as_deref(), Some(german.id.as_str()));
    assert_eq!(french.source_language.as_deref(), Some("de"));
    let mut conn = store.get_connection().unwrap();
    assert_eq!(variants::list(&mut conn, &meeting).unwrap().len(), 3);
}
