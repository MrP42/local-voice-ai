//! G5: Protokoll mit gewaehlter Grundlage und Ausgabesprache. Mock-Server statt Modell: geprueft
//! wird, WAS das Modell zu sehen bekommt (Segmente der Fassung, Sprachforderung) und was im
//! Dokument und in den Metadaten steht (Kopfzeile, Grundlage, Ausgabesprache).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::managers::meetings::llm_call::test_support::{
    chat_body, settings_with_mock_provider, spawn_llm_mock_with, MockReply,
};
use crate::managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};
use crate::managers::meetings::variants::{self, NewVariant, TranslationExtra, KIND_TRANSLATION};

fn seg(i: u32, text: &str) -> StoredSegment {
    StoredSegment {
        segment_index: i,
        text: text.to_string(),
        start_ms: u64::from(i) * 60_000,
        end_ms: u64::from(i) * 60_000 + 50_000,
        channel: 1,
        speaker_index: None,
        words: None,
    }
}

struct Fx {
    store: Arc<MeetingStore>,
    meeting: String,
    original: String,
    translation: String,
}

const ORIGINAL: [&str; 2] = [
    "We agreed to go live on the first of September after the test phase.",
    "Anna will write the release notes by Friday.",
];
const TRANSLATED: [&str; 2] = [
    "Wir haben uns auf den Go-live am ersten September nach der Testphase geeinigt.",
    "Anna schreibt die Release Notes bis Freitag.",
];

/// Englisches Original (aktiv) und deutsche Uebersetzung.
fn fixture() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
    std::mem::forget(dir);
    let store = Arc::new(store);
    let meeting = store
        .create_meeting("Review", MeetingSource::Import, Some(1_755_600_000))
        .unwrap()
        .id;
    store
        .append_delta(
            &meeting,
            &TranscriptDelta {
                new_segments: ORIGINAL.iter().enumerate().map(|(i, t)| seg(i as u32, t)).collect(),
            },
        )
        .unwrap();
    store.set_status(&meeting, MeetingStatus::Ready).unwrap();
    let mut conn = store.get_connection().unwrap();
    variants::set_language(&mut conn, &meeting, "en").unwrap();
    let original = variants::list(&mut conn, &meeting).unwrap()[0].id.clone();
    let translation = variants::add_translation(
        &mut conn,
        NewVariant {
            meeting_id: meeting.clone(),
            kind: KIND_TRANSLATION,
            language: Some("de".into()),
            model: None,
            segments: TRANSLATED.iter().enumerate().map(|(i, t)| seg(i as u32, t)).collect(),
            activate: false,
        },
        TranslationExtra {
            source_variant_id: original.clone(),
            source_language: Some("en".into()),
            meta_json: "{}".into(),
        },
    )
    .unwrap();
    drop(conn);
    Fx {
        store,
        meeting,
        original,
        translation,
    }
}

fn answer() -> String {
    json!({
        "zusammenfassung": ["Der Go-Live wurde auf den 1. September gelegt."],
        "besprochene_punkte": ["Testphase ist abgeschlossen"],
        "entscheidungen": ["Go-Live am 1. September"],
        "aufgaben": [{ "text": "Release Notes schreiben", "assignee": "Anna", "due": "Freitag" }],
        "offene_fragen": []
    })
    .to_string()
}

fn limits() -> Limits {
    Limits {
        budget_chars: Some(1_000_000),
        timeout: Duration::from_secs(60),
        classify_timeout: Duration::from_secs(30),
        free_mb: || 0,
    }
}

/// Laeuft das Protokoll mit der Wahl; gibt das Dokument und alle Anfragen des Modells zurueck.
async fn run_with(fx: &Fx, doc_basis: &DocBasis) -> (Result<MeetingDocument, MinutesError>, Vec<String>) {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&seen);
    let port = spawn_llm_mock_with(move |body| {
        log.lock().unwrap().push(body.to_string());
        MockReply::Body(chat_body(&answer()))
    })
    .await;
    let settings = settings_with_mock_provider(port);
    let result = generate_guarded_with(
        &settings,
        fx.store.clone(),
        &fx.meeting,
        Some("builtin:allgemein"),
        doc_basis,
        limits(),
        &|_| {},
    )
    .await;
    let bodies = seen.lock().unwrap().clone();
    (result, bodies)
}

fn metadata_of(fx: &Fx, doc: &MeetingDocument) -> Value {
    serde_json::from_str(&fx.store.document_generation_metadata(&doc.id).unwrap().unwrap()).unwrap()
}

#[tokio::test]
async fn the_translation_as_basis_is_what_the_model_reads_and_the_header_says_so() {
    let fx = fixture();
    let (doc, bodies) = run_with(
        &fx,
        &DocBasis {
            variant_id: Some(fx.translation.clone()),
            output_language: Some("de".into()),
        },
    )
    .await;
    let doc = doc.unwrap();
    assert_eq!(bodies.len(), 1);
    let request = &bodies[0];
    assert!(request.contains("Testphase geeinigt"), "das Modell liest die Uebersetzung");
    assert!(!request.contains("after the test phase"), "und nicht das Original");
    assert!(
        request.contains("Write ALL texts of the reply in German"),
        "die Ausgabesprache wird ausdruecklich gefordert"
    );
    assert!(!request.contains("Same language as the transcript."));
    assert!(
        doc.body.contains("**Grundlage:** Übersetzung (Deutsch) · Protokoll auf Deutsch"),
        "Kopfzeile fehlt: {}",
        doc.body
    );
    let meta = metadata_of(&fx, &doc);
    assert_eq!(meta["basis"]["variant_id"], json!(fx.translation));
    assert_eq!(meta["basis"]["variant_kind"], "translation");
    assert_eq!(meta["basis"]["language"], "de");
    assert_eq!(meta["output_language"], Value::Null, "die Ausgabesprache steht in der Grundlage-Angabe");
}

#[tokio::test]
async fn the_original_can_be_the_basis_for_a_german_report() {
    // Original Englisch, Protokoll auf Deutsch: die Grundlage ist das Original, das Modell
    // soll auf Deutsch schreiben.
    let fx = fixture();
    let (doc, bodies) = run_with(
        &fx,
        &DocBasis {
            variant_id: Some(fx.original.clone()),
            output_language: Some("de".into()),
        },
    )
    .await;
    let doc = doc.unwrap();
    assert!(bodies[0].contains("after the test phase"));
    assert!(bodies[0].contains("Write ALL texts of the reply in German"));
    assert!(
        doc.body.contains("**Grundlage:** Original (Englisch) · Protokoll auf Deutsch"),
        "{}",
        doc.body
    );
}

#[tokio::test]
async fn without_a_choice_the_prompt_and_the_active_transcript_are_as_before() {
    let fx = fixture();
    let (doc, bodies) = run_with(&fx, &DocBasis::default()).await;
    let doc = doc.unwrap();
    assert!(bodies[0].contains("Same language as the transcript."), "die alte Regel");
    assert!(!bodies[0].contains("Write ALL texts"));
    assert!(bodies[0].contains("after the test phase"), "aktive Fassung = Original");
    // Auch ohne Wahl steht die Grundlage im Kopf.
    assert!(
        doc.body.contains("**Grundlage:** Original (Englisch) · Protokoll auf Englisch"),
        "{}",
        doc.body
    );
}

#[tokio::test]
async fn the_active_transcript_and_the_variants_are_not_touched() {
    let fx = fixture();
    let before: Vec<String> = fx.store.get_segments(&fx.meeting).unwrap().into_iter().map(|s| s.text).collect();
    let (doc, _) = run_with(
        &fx,
        &DocBasis {
            variant_id: Some(fx.translation.clone()),
            output_language: Some("en".into()),
        },
    )
    .await;
    doc.unwrap();
    let after: Vec<String> = fx.store.get_segments(&fx.meeting).unwrap().into_iter().map(|s| s.text).collect();
    assert_eq!(after, before);
    let mut conn = fx.store.get_connection().unwrap();
    let list = variants::list(&mut conn, &fx.meeting).unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().find(|v| v.id == fx.original).unwrap().active);
}

#[tokio::test]
async fn an_unknown_variant_fails_before_any_model_call_and_writes_nothing() {
    let fx = fixture();
    let (result, bodies) = run_with(
        &fx,
        &DocBasis {
            variant_id: Some("gibt-es-nicht".into()),
            output_language: Some("de".into()),
        },
    )
    .await;
    let err = result.unwrap_err();
    assert_eq!(err.code, "store_failed");
    assert!(err.detail.contains("variant_not_found"));
    assert!(bodies.is_empty(), "kein Modellaufruf");
    assert!(fx.store.get_documents(&fx.meeting).unwrap().is_empty());
}

#[tokio::test]
async fn the_output_language_does_not_leak_into_the_next_run() {
    let fx = fixture();
    let (_, with_language) = run_with(
        &fx,
        &DocBasis {
            variant_id: None,
            output_language: Some("fr".into()),
        },
    )
    .await;
    assert!(with_language[0].contains("Write ALL texts of the reply in French"));
    let (_, plain) = run_with(&fx, &DocBasis::default()).await;
    assert!(plain[0].contains("Same language as the transcript."), "der Bereich endet mit dem Lauf");
}

#[test]
fn the_markdown_head_gains_the_basis_line_only_when_given() {
    let head = MinutesHead {
        title: "Review".into(),
        description: String::new(),
        date_iso: "2026-10-01".into(),
        duration_ms: 120_000,
        shares: vec![],
        single_speaker: true,
        mixed_channel: false,
    };
    let sections = vec![MinutesSection {
        id: "zusammenfassung".into(),
        title: "Zusammenfassung".into(),
        kind: SectionKind::Text,
        entries: vec![MinutesEntry { text: "Alles gut.".into(), assignee: None, due: None }],
    }];
    let plain = minutes_to_markdown(&head, "Allgemein", None, &sections, &[]);
    let with = minutes_to_markdown_with_basis(
        &head,
        "Allgemein",
        None,
        &sections,
        &[],
        Some("**Grundlage:** Original (Englisch) · Protokoll auf Deutsch"),
    );
    assert!(!plain.contains("Grundlage"), "ohne Angabe wie vor G5");
    let lines: Vec<&str> = with.lines().collect();
    let at = lines.iter().position(|l| l.starts_with("**Datum:**")).unwrap();
    assert_eq!(lines[at + 1], "**Grundlage:** Original (Englisch) · Protokoll auf Deutsch", "direkt unter der Datumszeile");
    assert_eq!(with.replace("**Grundlage:** Original (Englisch) · Protokoll auf Deutsch\n", ""), plain);
}
