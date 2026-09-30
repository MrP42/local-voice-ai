use rusqlite::params;
use serde_json::json;

use super::*;
use crate::managers::integrations::test_support::Fx;
use crate::managers::meetings::store::{MeetingSource, StoredSegment};

fn base(kind: SubjectKind, id: &str) -> NewProvenance {
    NewProvenance::new(kind, id, "minutes", ActorKind::User)
}

fn full() -> NewProvenance {
    let mut e = base(SubjectKind::Document, "doc-1");
    e.subject_revision = Some(3);
    e.operation = "minutes".into();
    e.actor_kind = ActorKind::Workflow;
    e.actor_ref = Some("run-42".into());
    e.provider = Some("openai".into());
    e.locality = Some(Locality::Remote);
    e.model_id = Some("gpt-4.1".into());
    e.model_label = Some("GPT-4.1".into());
    e.usage_event_id = Some(77);
    e.prompt_tokens = Some(12_345);
    e.completion_tokens = Some(678);
    e.duration_ms = Some(9_876);
    e.sources = vec![
        SourceRef::new("transcript", "m-1", Some("Jour fixe")),
        SourceRef {
            kind: "youtube".into(),
            reference: "dQw4w9WgXcQ".into(),
            title: Some("Video".into()),
            url: Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into()),
        },
    ];
    e.confidence = Some(0.73);
    e.params = Some(json!({ "template_id": "builtin:allgemein", "chunks": 4 }));
    e
}

fn meeting_with_transcript(fx: &Fx, model: &str) -> String {
    let m = fx
        .store
        .create_meeting("Jour fixe", MeetingSource::Live, Some(1_755_600_000))
        .unwrap();
    let segments = vec![StoredSegment {
        segment_index: 0,
        text: "Hallo zusammen.".into(),
        start_ms: 0,
        end_ms: 1_000,
        channel: 0,
        speaker_index: None,
        words: None,
    }];
    fx.store
        .replace_segments(&m.id, &segments, model, "segment@1", 0)
        .unwrap();
    m.id
}

#[test]
fn an_entry_round_trips_with_every_field() {
    let fx = Fx::new();
    let conn = fx.conn();
    let id = record_at(&conn, &full(), 1_790_000_000_123).unwrap();
    assert_eq!(id.len(), 26, "ULID");
    let got = list(&conn, SubjectKind::Document, "doc-1").unwrap();
    assert_eq!(got.len(), 1);
    let e = &got[0];
    assert_eq!(e.id, id);
    assert_eq!(e.created_at, 1_790_000_000_123);
    assert_eq!(e.subject_revision, Some(3));
    assert_eq!(e.operation, "minutes");
    assert_eq!(e.actor_kind, Some(ActorKind::Workflow));
    assert_eq!(e.actor_ref.as_deref(), Some("run-42"));
    assert_eq!(
        (e.provider.as_deref(), e.locality),
        (Some("openai"), Some(Locality::Remote))
    );
    assert_eq!(
        (e.model_id.as_deref(), e.model_label.as_deref()),
        (Some("gpt-4.1"), Some("GPT-4.1"))
    );
    assert_eq!(e.usage_event_id, Some(77));
    assert_eq!(
        (e.prompt_tokens, e.completion_tokens, e.duration_ms),
        (Some(12_345), Some(678), Some(9_876))
    );
    assert_eq!(e.sources.len(), 2);
    assert_eq!(
        e.sources[1].url.as_deref(),
        Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
    );
    assert_eq!(e.confidence, Some(0.73));
    let params: serde_json::Value =
        serde_json::from_str(e.params_json.as_deref().unwrap()).unwrap();
    assert_eq!(params["chunks"], 4);
    assert_eq!(e.origin, ProvenanceOrigin::Recorded);
}

#[test]
fn optional_fields_stay_empty_not_zero() {
    let fx = Fx::new();
    let conn = fx.conn();
    record_at(&conn, &base(SubjectKind::Summary, "s-1"), 1).unwrap();
    let e = list(&conn, SubjectKind::Summary, "s-1").unwrap().remove(0);
    assert_eq!(
        (e.prompt_tokens, e.completion_tokens, e.duration_ms),
        (None, None, None)
    );
    assert_eq!(
        (e.usage_event_id, e.confidence, e.locality),
        (None, None, None)
    );
    assert!(e.sources.is_empty() && e.params_json.is_none());
}

#[test]
fn entries_are_ordered_by_time_and_isolated_per_subject() {
    let fx = Fx::new();
    let conn = fx.conn();
    let second = record_at(&conn, &base(SubjectKind::Document, "d"), 2_000).unwrap();
    let first = record_at(&conn, &base(SubjectKind::Document, "d"), 1_000).unwrap();
    record_at(&conn, &base(SubjectKind::Document, "other"), 1_500).unwrap();
    record_at(&conn, &base(SubjectKind::Summary, "d"), 1_600).unwrap();
    let ids: Vec<String> = list(&conn, SubjectKind::Document, "d")
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids, vec![first, second], "aelteste zuerst");
    assert!(list(&conn, SubjectKind::Document, "nope")
        .unwrap()
        .is_empty());
    // Dieselbe Kennung unter einer anderen Art ist ein anderer Inhalt.
    assert_eq!(list(&conn, SubjectKind::Summary, "d").unwrap().len(), 1);
}

#[test]
fn invalid_input_is_refused_not_stored() {
    let fx = Fx::new();
    let conn = fx.conn();
    let mut cases: Vec<(&str, NewProvenance)> = Vec::new();
    cases.push(("leere Kennung", base(SubjectKind::Document, "  ")));
    let mut e = full();
    e.operation = "Protokoll!".into();
    cases.push(("Operation mit Sonderzeichen", e));
    let mut e = full();
    e.operation = String::new();
    cases.push(("leere Operation", e));
    for bad in [f64::NAN, f64::INFINITY, -0.1, 1.0001] {
        let mut e = full();
        e.confidence = Some(bad);
        cases.push(("Konfidenz ausser 0..1", e));
    }
    let mut e = full();
    e.usage_event_id = Some(0);
    cases.push(("Ereignisnummer 0", e));
    let mut e = full();
    e.sources = vec![SourceRef::new("Web Seite", "x", None)];
    cases.push(("Quellenart mit Leerzeichen", e));
    let mut e = full();
    e.sources = vec![SourceRef::new("web", "  ", None)];
    cases.push(("leerer Quellenverweis", e));
    let mut e = full();
    e.sources = (0..=MAX_SOURCES)
        .map(|i| SourceRef::new("web", &format!("r{i}"), None))
        .collect();
    cases.push(("zu viele Quellen", e));
    let mut e = full();
    e.params = Some(json!(["kein", "objekt"]));
    cases.push(("params kein Objekt", e));
    let mut e = full();
    e.params = Some(json!({ "blob": "x".repeat(MAX_PARAMS_BYTES) }));
    cases.push(("params zu gross", e));
    for (what, entry) in cases {
        let err = record_at(&conn, &entry, 1).unwrap_err();
        assert!(matches!(err, ProvenanceError::Invalid(_)), "{what}: {err}");
    }
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM provenance", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "nichts gespeichert");
}

#[test]
fn boundary_values_are_accepted() {
    let fx = Fx::new();
    let conn = fx.conn();
    for c in [0.0, 1.0, 0.5] {
        let mut e = full();
        e.confidence = Some(c);
        record_at(&conn, &e, 1).unwrap();
    }
    let mut e = full();
    e.sources = (0..MAX_SOURCES)
        .map(|i| SourceRef::new("web", &format!("r{i}"), None))
        .collect();
    record_at(&conn, &e, 1).unwrap();
    // u64 jenseits von i64 wird begrenzt statt abgeschnitten.
    let mut e = full();
    e.prompt_tokens = Some(u64::MAX);
    record_at(&conn, &e, 1).unwrap();
}

#[test]
fn long_texts_are_clipped_and_blank_ones_dropped() {
    let fx = Fx::new();
    let conn = fx.conn();
    let mut e = full();
    e.model_id = Some("m".repeat(5_000));
    e.actor_ref = Some("   ".into());
    let long_title = "t".repeat(5_000);
    e.sources = vec![SourceRef::new(
        "web",
        &"r".repeat(5_000),
        Some(long_title.as_str()),
    )];
    record_at(&conn, &e, 1).unwrap();
    let got = list(&conn, SubjectKind::Document, "doc-1")
        .unwrap()
        .remove(0);
    assert!(got.model_id.unwrap().chars().count() <= MAX_FIELD_CHARS + 1);
    assert_eq!(got.actor_ref, None, "leere Angabe wird nicht gespeichert");
    assert!(got.sources[0].reference.chars().count() <= MAX_FIELD_CHARS + 1);
    assert!(got.sources[0].title.as_ref().unwrap().chars().count() <= MAX_FIELD_CHARS + 1);
}

#[test]
fn the_database_itself_refuses_bad_enums_and_confidence() {
    let fx = Fx::new();
    let conn = fx.conn();
    for sql in [
        "INSERT INTO provenance (id, subject_kind, subject_id, created_at, operation, actor_kind)
         VALUES ('a', 'bogus', 'x', 1, 'stt', 'user')",
        "INSERT INTO provenance (id, subject_kind, subject_id, created_at, operation, actor_kind)
         VALUES ('b', 'document', 'x', 1, 'stt', 'robot')",
        "INSERT INTO provenance (id, subject_kind, subject_id, created_at, operation, actor_kind, locality)
         VALUES ('c', 'document', 'x', 1, 'stt', 'user', 'mars')",
        "INSERT INTO provenance (id, subject_kind, subject_id, created_at, operation, actor_kind, confidence)
         VALUES ('d', 'document', 'x', 1, 'stt', 'user', 1.5)",
    ] {
        assert!(conn.execute(sql, []).is_err(), "{sql}");
    }
}

#[test]
fn parallel_writers_keep_every_entry() {
    let fx = Fx::new();
    let threads: Vec<_> = (0..4)
        .map(|t| {
            let path = fx.db_path.clone();
            std::thread::spawn(move || {
                let conn = rusqlite::Connection::open(&path).unwrap();
                for i in 0..25 {
                    record_at(&conn, &base(SubjectKind::Document, "shared"), t * 1000 + i).unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let got = list(&fx.conn(), SubjectKind::Document, "shared").unwrap();
    assert_eq!(got.len(), 100);
    let unique: std::collections::HashSet<_> = got.iter().map(|e| e.id.clone()).collect();
    assert_eq!(unique.len(), 100);
}

#[test]
fn names_round_trip_and_serialize_in_snake_case() {
    for k in SubjectKind::ALL {
        assert_eq!(SubjectKind::parse(k.as_str()), Some(k));
        assert_eq!(serde_json::to_value(k).unwrap(), json!(k.as_str()));
    }
    for a in ActorKind::ALL {
        assert_eq!(ActorKind::parse(a.as_str()), Some(a));
        assert_eq!(serde_json::to_value(a).unwrap(), json!(a.as_str()));
    }
    for l in [Locality::Local, Locality::Remote] {
        assert_eq!(Locality::parse(l.as_str()), Some(l));
    }
    assert_eq!(SubjectKind::parse("Document"), None);
    // Die Quelle heisst im JSON `ref`, wie in der Tabelle.
    let s = serde_json::to_value(SourceRef::new("web", "x", None)).unwrap();
    assert_eq!(s["ref"], "x");
}

// -- Rueckfall auf Altdaten ---------------------------------------------------

const MINUTES_METADATA: &str = r#"{"model":"gemma-4-e4b","provider":"local","template_id":"builtin:allgemein","template_title":"Allgemein","single_pass":true,"chunks_total":1,"chunks_failed":[],"incomplete":false}"#;

#[test]
fn an_old_document_yields_its_origin_from_generation_metadata() {
    let fx = Fx::new();
    let m = fx
        .store
        .create_meeting("Jour fixe", MeetingSource::Live, None)
        .unwrap();
    let doc_id = fx
        .store
        .insert_document(
            &m.id,
            "minutes",
            "markdown@1",
            "# Protokoll",
            Some("builtin:allgemein"),
            Some(MINUTES_METADATA),
        )
        .unwrap();
    let conn = fx.conn();
    assert!(
        list(&conn, SubjectKind::Document, &doc_id)
            .unwrap()
            .is_empty(),
        "kein gespeicherter Eintrag"
    );

    let got = get(&conn, SubjectKind::Document, &doc_id).unwrap();
    assert_eq!(got.len(), 1);
    let e = &got[0];
    assert_eq!(e.origin, ProvenanceOrigin::Derived);
    assert_eq!(e.model_id.as_deref(), Some("gemma-4-e4b"));
    assert_eq!(e.provider.as_deref(), Some("local"));
    assert_eq!(
        e.locality,
        Some(Locality::Local),
        "der eingebaute Anbieter ist lokal"
    );
    assert_eq!(e.operation, "minutes");
    assert_eq!(e.subject_revision, Some(1));
    assert_eq!(
        e.actor_kind, None,
        "der Ausloeser ist aus Altdaten nicht ablesbar"
    );
    assert_eq!(
        (
            e.prompt_tokens,
            e.completion_tokens,
            e.duration_ms,
            e.usage_event_id
        ),
        (None, None, None, None)
    );
    assert_eq!(e.sources, vec![SourceRef::new("transcript", &m.id, None)]);
    // Die Sekunden der Tabelle werden zu Millisekunden.
    let created_secs: i64 = conn
        .query_row(
            "SELECT created_at FROM meeting_documents WHERE id = ?1",
            params![doc_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(e.created_at, created_secs * 1000);
    // Die Metadaten bleiben als params erhalten (Vorlage, Bloecke).
    let params: serde_json::Value =
        serde_json::from_str(e.params_json.as_deref().unwrap()).unwrap();
    assert_eq!(params["template_id"], "builtin:allgemein");
    assert_eq!(params["single_pass"], true);
}

#[test]
fn an_old_enhanced_notes_document_is_called_notes_and_a_remote_provider_stays_unknown() {
    let fx = Fx::new();
    let m = fx
        .store
        .create_meeting("Kunde", MeetingSource::Live, None)
        .unwrap();
    let doc_id = fx
        .store
        .insert_document(
            &m.id,
            "enhanced_notes",
            "enhanced@1",
            "{}",
            None,
            Some(r#"{"mode":"enhance","model":"gpt-4.1","provider":"openai"}"#),
        )
        .unwrap();
    let e = get(&fx.conn(), SubjectKind::Document, &doc_id)
        .unwrap()
        .remove(0);
    assert_eq!(e.operation, "notes");
    assert_eq!(e.provider.as_deref(), Some("openai"));
    assert_eq!(e.locality, None, "aus der Kennung allein nicht erkennbar");
}

#[test]
fn a_document_without_usable_metadata_has_no_derived_origin() {
    let fx = Fx::new();
    let m = fx
        .store
        .create_meeting("x", MeetingSource::Live, None)
        .unwrap();
    let conn = fx.conn();
    let none = fx
        .store
        .insert_document(&m.id, "minutes", "markdown@1", "a", None, None)
        .unwrap();
    let blank = fx
        .store
        .insert_document(&m.id, "minutes", "markdown@1", "b", None, Some("  "))
        .unwrap();
    let broken = fx
        .store
        .insert_document(
            &m.id,
            "minutes",
            "markdown@1",
            "c",
            None,
            Some("{kein json"),
        )
        .unwrap();
    let not_object = fx
        .store
        .insert_document(&m.id, "minutes", "markdown@1", "d", None, Some("[1,2]"))
        .unwrap();
    for id in [none, blank, broken, not_object] {
        assert!(
            get(&conn, SubjectKind::Document, &id).unwrap().is_empty(),
            "{id}"
        );
    }
    // Unbekanntes Dokument und geloeschte Besprechung.
    assert!(get(&conn, SubjectKind::Document, "nope")
        .unwrap()
        .is_empty());
}

#[test]
fn a_recorded_entry_wins_over_the_legacy_metadata() {
    let fx = Fx::new();
    let m = fx
        .store
        .create_meeting("x", MeetingSource::Live, None)
        .unwrap();
    let doc_id = fx
        .store
        .insert_document(
            &m.id,
            "minutes",
            "markdown@1",
            "a",
            None,
            Some(MINUTES_METADATA),
        )
        .unwrap();
    let conn = fx.conn();
    let mut e = base(SubjectKind::Document, &doc_id);
    e.model_id = Some("gpt-4.1".into());
    record_at(&conn, &e, 5).unwrap();
    let got = get(&conn, SubjectKind::Document, &doc_id).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].origin, ProvenanceOrigin::Recorded);
    assert_eq!(got[0].model_id.as_deref(), Some("gpt-4.1"));
}

#[test]
fn an_old_transcript_yields_model_and_source_from_its_row() {
    let fx = Fx::new();
    let meeting_id = meeting_with_transcript(&fx, "whisper-large-v3");
    let conn = fx.conn();
    let got = get(&conn, SubjectKind::Transcript, &meeting_id).unwrap();
    assert_eq!(got.len(), 1);
    let e = &got[0];
    assert_eq!(e.origin, ProvenanceOrigin::Derived);
    assert_eq!(e.operation, "stt");
    assert_eq!(e.model_id.as_deref(), Some("whisper-large-v3"));
    assert_eq!(
        e.locality,
        Some(Locality::Local),
        "die Spracherkennung laeuft lokal"
    );
    assert_eq!(e.sources, vec![SourceRef::new("audio", &meeting_id, None)]);
    assert_eq!(e.subject_revision, Some(1));
    assert!(
        e.created_at > 0 && e.created_at % 1000 == 0,
        "Sekunden als Millisekunden"
    );
}

#[test]
fn a_transcript_without_a_model_or_a_row_has_no_derived_origin() {
    let fx = Fx::new();
    let m = fx
        .store
        .create_meeting("Live", MeetingSource::Live, None)
        .unwrap();
    let conn = fx.conn();
    // Noch kein Transkript.
    assert!(get(&conn, SubjectKind::Transcript, &m.id)
        .unwrap()
        .is_empty());
    // Zeile ohne Modell (Live-Segmente ohne Enddurchlauf).
    fx.store
        .append_delta(
            &m.id,
            &crate::managers::meetings::store::TranscriptDelta {
                new_segments: vec![StoredSegment {
                    segment_index: 0,
                    text: "Hallo".into(),
                    start_ms: 0,
                    end_ms: 500,
                    channel: 0,
                    speaker_index: None,
                    words: None,
                }],
            },
        )
        .unwrap();
    assert!(get(&conn, SubjectKind::Transcript, &m.id)
        .unwrap()
        .is_empty());
}

#[test]
fn kinds_without_a_legacy_source_have_nothing_to_derive() {
    let fx = Fx::new();
    let conn = fx.conn();
    for kind in [
        SubjectKind::Summary,
        SubjectKind::TranscriptVariant,
        SubjectKind::KnowledgeNote,
        SubjectKind::TtsAudio,
        SubjectKind::Export,
        SubjectKind::RunOutput,
    ] {
        assert!(get(&conn, kind, "x").unwrap().is_empty(), "{kind:?}");
    }
}

#[test]
fn a_document_without_an_entry_falls_back_to_its_metadata_when_the_table_is_gone() {
    let fx = Fx::new();
    let m = fx
        .store
        .create_meeting("x", MeetingSource::Live, None)
        .unwrap();
    let doc_id = fx
        .store
        .insert_document(
            &m.id,
            "minutes",
            "markdown@1",
            "a",
            None,
            Some(MINUTES_METADATA),
        )
        .unwrap();
    let conn = fx.conn();
    // Der Lesepfad sagt es ehrlich, wenn die Tabelle nicht lesbar ist (Fehler statt Raten) ...
    conn.execute_batch("DROP TABLE provenance").unwrap();
    assert!(get(&conn, SubjectKind::Document, &doc_id).is_err());
    // ... und die Legacy-Ableitung selbst braucht die Tabelle nicht.
    let e = derive_legacy(&conn, SubjectKind::Document, &doc_id)
        .unwrap()
        .unwrap();
    assert_eq!(e.model_id.as_deref(), Some("gemma-4-e4b"));
}
