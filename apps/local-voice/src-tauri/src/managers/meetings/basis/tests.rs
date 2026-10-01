use super::*;
use crate::managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};
use crate::managers::meetings::variants::{NewVariant, TranslationExtra};

fn seg(i: u32, text: &str) -> StoredSegment {
    StoredSegment {
        segment_index: i,
        text: text.to_string(),
        start_ms: u64::from(i) * 4000,
        end_ms: u64::from(i) * 4000 + 3500,
        channel: 2,
        speaker_index: None,
        words: None,
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    store: MeetingStore,
    meeting: String,
    original: String,
    translation: String,
}

/// Englisches Original (Fassung 1, aktiv) und eine deutsche Uebersetzung (Fassung 2).
fn fixture() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
    let meeting = store
        .create_meeting("Interview", MeetingSource::Import, None)
        .unwrap()
        .id;
    store.set_status(&meeting, MeetingStatus::Ready).unwrap();
    store
        .append_delta(
            &meeting,
            &TranscriptDelta {
                new_segments: vec![seg(0, "We pay 120 euros."), seg(1, "Anna agrees.")],
            },
        )
        .unwrap();
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
            segments: vec![seg(0, "Wir zahlen 120 Euro."), seg(1, "Anna ist einverstanden.")],
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
        _dir: dir,
        store,
        meeting,
        original,
        translation,
    }
}

fn texts(basis: &ResolvedBasis) -> Vec<&str> {
    basis.segments.iter().map(|s| s.text.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Grundlage
// ---------------------------------------------------------------------------

#[test]
fn without_a_choice_the_active_variant_is_the_basis() {
    let f = fixture();
    let basis = resolve(&f.store, &f.meeting, &DocBasis::default()).unwrap();
    assert_eq!(texts(&basis), vec!["We pay 120 euros.", "Anna agrees."]);
    assert_eq!(basis.variant.as_ref().unwrap().id, f.original);
    assert_eq!(basis.source_language.as_deref(), Some("en"));
    assert_eq!(basis.output_language, None);
}

#[test]
fn a_chosen_translation_is_the_basis_even_while_the_original_is_active() {
    let f = fixture();
    let basis = resolve(
        &f.store,
        &f.meeting,
        &DocBasis {
            variant_id: Some(f.translation.clone()),
            output_language: Some("de".into()),
        },
    )
    .unwrap();
    assert_eq!(texts(&basis), vec!["Wir zahlen 120 Euro.", "Anna ist einverstanden."]);
    assert_eq!(basis.variant.as_ref().unwrap().kind, KIND_TRANSLATION);
    assert_eq!(basis.source_language.as_deref(), Some("de"));
    // Das aktive Transkript wurde dabei nicht angefasst.
    assert_eq!(store_texts(&f), vec!["We pay 120 euros.", "Anna agrees."]);
}

fn store_texts(f: &Fx) -> Vec<String> {
    f.store.get_segments(&f.meeting).unwrap().into_iter().map(|s| s.text).collect()
}

#[test]
fn the_original_can_be_the_basis_while_the_translation_is_active() {
    let f = fixture();
    let mut conn = f.store.get_connection().unwrap();
    variants::activate(&mut conn, &f.translation).unwrap();
    drop(conn);
    let basis = resolve(
        &f.store,
        &f.meeting,
        &DocBasis {
            variant_id: Some(f.original.clone()),
            output_language: None,
        },
    )
    .unwrap();
    assert_eq!(texts(&basis), vec!["We pay 120 euros.", "Anna agrees."]);
    // Ohne Wahl gilt jetzt die aktive Uebersetzung.
    let default = resolve(&f.store, &f.meeting, &DocBasis::default()).unwrap();
    assert_eq!(default.variant.unwrap().kind, KIND_TRANSLATION);
}

#[test]
fn an_unknown_or_foreign_variant_is_refused() {
    let f = fixture();
    let err = resolve(
        &f.store,
        &f.meeting,
        &DocBasis {
            variant_id: Some("gibt-es-nicht".into()),
            output_language: None,
        },
    )
    .unwrap_err();
    assert_eq!(err, BasisError::VariantNotFound);
    let other = f
        .store
        .create_meeting("Andere", MeetingSource::Import, None)
        .unwrap()
        .id;
    let err = resolve(
        &f.store,
        &other,
        &DocBasis {
            variant_id: Some(f.original.clone()),
            output_language: None,
        },
    )
    .unwrap_err();
    assert_eq!(err, BasisError::VariantNotFound, "die Fassung einer anderen Besprechung zaehlt nicht");
}

#[test]
fn a_meeting_without_transcript_has_an_empty_basis() {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
    let id = store.create_meeting("leer", MeetingSource::Import, None).unwrap().id;
    let basis = resolve(&store, &id, &DocBasis::default()).unwrap();
    assert!(basis.segments.is_empty() && basis.variant.is_none());
}

#[test]
fn the_output_language_is_reduced_to_its_base_code() {
    let f = fixture();
    for (given, expected) in [(Some("de-DE"), Some("de")), (Some("auto"), None), (Some(""), None), (None, None)] {
        let basis = resolve(
            &f.store,
            &f.meeting,
            &DocBasis {
                variant_id: None,
                output_language: given.map(str::to_string),
            },
        )
        .unwrap();
        assert_eq!(basis.output_language.as_deref(), expected, "{given:?}");
    }
}

#[test]
fn an_unknown_variant_language_falls_back_to_the_text() {
    let f = fixture();
    f.store
        .get_connection()
        .unwrap()
        .execute("UPDATE transcript_variants SET language = NULL", [])
        .unwrap();
    f.store
        .get_connection()
        .unwrap()
        .execute("UPDATE meetings SET language = NULL", [])
        .unwrap();
    // Zu kurz fuer eine Erkennung: unbekannt, nicht geraten.
    let basis = resolve(&f.store, &f.meeting, &DocBasis::default()).unwrap();
    assert_eq!(basis.source_language, None);
}

// ---------------------------------------------------------------------------
// Kopf und Metadaten
// ---------------------------------------------------------------------------

fn doc(kind: &str, language: Option<&str>, output: Option<&str>) -> DocumentBasis {
    DocumentBasis {
        variant_id: Some("v".into()),
        variant_number: Some(1),
        variant_kind: Some(kind.into()),
        language: language.map(str::to_string),
        output_language: output.map(str::to_string),
    }
}

#[test]
fn the_header_names_basis_and_output_language_in_german() {
    assert_eq!(
        basis_line(&doc("stt", Some("en"), Some("de"))),
        "**Grundlage:** Original (Englisch) · Protokoll auf Deutsch"
    );
    assert_eq!(
        basis_line(&doc(KIND_TRANSLATION, Some("de"), Some("en"))),
        "**Grundlage:** Übersetzung (Deutsch) · Protokoll auf Englisch"
    );
}

#[test]
fn without_a_chosen_output_language_the_basis_language_is_named() {
    assert_eq!(
        basis_line(&doc("stt", Some("fr"), None)),
        "**Grundlage:** Original (Französisch) · Protokoll auf Französisch"
    );
    assert_eq!(
        basis_line(&doc("stt", None, None)),
        "**Grundlage:** Original · Protokoll in der Sprache der Grundlage"
    );
}

#[test]
fn the_metadata_round_trips_and_an_old_version_has_none() {
    let f = fixture();
    let basis = resolve(
        &f.store,
        &f.meeting,
        &DocBasis {
            variant_id: Some(f.translation.clone()),
            output_language: Some("en".into()),
        },
    )
    .unwrap();
    let meta = basis.metadata();
    let back = from_metadata(&meta).unwrap();
    assert_eq!(back.variant_id.as_deref(), Some(f.translation.as_str()));
    assert_eq!(back.variant_kind.as_deref(), Some(KIND_TRANSLATION));
    assert_eq!(back.language.as_deref(), Some("de"));
    assert_eq!(back, basis.to_document_basis());
    assert_eq!(from_metadata(&json!({"model": "x"})), None, "Version vor G5: keine Angabe");
}

// ---------------------------------------------------------------------------
// Prompt
// ---------------------------------------------------------------------------

#[test]
fn without_an_output_language_the_rule_is_the_old_sentence() {
    assert_eq!(language_rule(), "Same language as the transcript.");
}

#[tokio::test]
async fn an_output_language_is_demanded_explicitly_inside_the_scope_only() {
    let inside = with_output_language(Some("de".into()), async { language_rule() }).await;
    assert!(inside.contains("German"), "{inside}");
    assert!(inside.contains("ALL"), "die Forderung ist ausdruecklich: {inside}");
    assert!(inside.contains("Names, numbers"), "{inside}");
    assert_eq!(language_rule(), "Same language as the transcript.", "danach wieder der alte Satz");
    let none = with_output_language(None, async { language_rule() }).await;
    assert_eq!(none, "Same language as the transcript.");
}

#[tokio::test]
async fn the_scope_survives_awaits_inside() {
    let rule = with_output_language(Some("en".into()), async {
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        language_rule()
    })
    .await;
    assert!(rule.contains("English"));
}

#[test]
fn the_basis_of_the_latest_document_is_read_from_its_metadata() {
    let f = fixture();
    let basis = resolve(
        &f.store,
        &f.meeting,
        &DocBasis {
            variant_id: Some(f.translation.clone()),
            output_language: Some("en".into()),
        },
    )
    .unwrap();
    // Eine aeltere Version ohne Angabe (vor G5), dann eine neue mit Grundlage.
    f.store
        .insert_document(&f.meeting, "minutes", "markdown@1", "alt", None, Some(r#"{"model":"x"}"#))
        .unwrap();
    assert_eq!(latest_document_basis(&f.store, &f.meeting, "minutes"), None);
    f.store
        .insert_document(&f.meeting, "minutes", "markdown@1", "neu", None, Some(&basis.metadata().to_string()))
        .unwrap();
    let read = latest_document_basis(&f.store, &f.meeting, "minutes").unwrap();
    assert_eq!(read.variant_id.as_deref(), Some(f.translation.as_str()));
    assert_eq!(read.output_language.as_deref(), Some("en"));
    assert_eq!(latest_document_basis(&f.store, &f.meeting, "enhanced_notes"), None, "andere Art: keine Angabe");
}
