use super::*;

fn seg(i: u32, start_s: u64, text: &str) -> StoredSegment {
    StoredSegment {
        segment_index: i,
        text: text.to_string(),
        start_ms: start_s * 1000,
        end_ms: start_s * 1000 + 900,
        channel: 2,
        speaker_index: None,
        words: None,
    }
}

fn reply(lines: &[(u32, &str)]) -> MergeReply {
    MergeReply {
        lines: lines
            .iter()
            .map(|(n, t)| ReplyLine {
                n: *n,
                text: t.to_string(),
            })
            .collect(),
    }
}

fn a_small() -> Vec<StoredSegment> {
    vec![
        seg(0, 1, "willkommen beim Wolf Applied AI Kanal"),
        seg(1, 4, "heute geht es um den Lastgang von 400 Kilowatt"),
    ]
}

fn b_small() -> Vec<StoredSegment> {
    vec![
        seg(0, 1, "Willkommen beim Wolff Applied AI Kanal."),
        seg(
            1,
            4,
            "Heute geht es um den Lastgang von 400 Kilowattstunden.",
        ),
    ]
}

// ---------------------------------------------------------------------------
// Pruefung einer Antwort
// ---------------------------------------------------------------------------

#[test]
fn a_good_answer_takes_the_spelling_from_the_other_version() {
    let a = a_small();
    let texts = validate_block(
        &a,
        1,
        "Willkommen beim Wolff Applied AI Kanal.\nHeute geht es um den Lastgang von 400 Kilowattstunden.",
        &reply(&[
            (1, "Willkommen beim Wolff Applied AI Kanal."),
            (2, "Heute geht es um den Lastgang von 400 Kilowattstunden."),
        ]),
    )
    .unwrap();
    assert_eq!(texts[0], "Willkommen beim Wolff Applied AI Kanal.");
    assert_eq!(texts.len(), 2);
}

#[test]
fn an_answer_that_breaks_the_schema_is_rejected() {
    let a = a_small();
    let b = "x";
    for (why, r) in [
        (
            "zu wenige Zeilen",
            reply(&[(1, "willkommen beim Wolf Applied AI Kanal")]),
        ),
        (
            "doppelte Nummer",
            reply(&[
                (1, "willkommen beim Wolf Applied AI Kanal"),
                (1, "heute geht es um den Lastgang von 400 Kilowatt"),
            ]),
        ),
        (
            "fremde Nummer",
            reply(&[
                (1, "willkommen beim Wolf Applied AI Kanal"),
                (7, "heute geht es um den Lastgang von 400 Kilowatt"),
            ]),
        ),
        (
            "Nummer 0",
            reply(&[
                (0, "willkommen beim Wolf Applied AI Kanal"),
                (2, "heute geht es um den Lastgang von 400 Kilowatt"),
            ]),
        ),
        (
            "leerer Text",
            reply(&[(1, "willkommen beim Wolf Applied AI Kanal"), (2, "   ")]),
        ),
        ("zu viele Zeilen", reply(&[(1, "a"), (2, "b"), (3, "c")])),
    ] {
        assert_eq!(validate_block(&a, 1, b, &r), Err(Reject::Schema), "{why}");
    }
}

#[test]
fn an_answer_that_invents_more_than_a_fifth_of_its_text_is_rejected() {
    let a = a_small();
    // Die zweite Zeile wird durch erfundenen Text gleicher Laenge ersetzt (mehr als 20 % neue Woerter).
    let invented = reply(&[
        (1, "willkommen beim Wolf Applied AI Kanal"),
        (
            2,
            "morgen gewinnen wir ploetzlich saemtliche Wahlen mit Raketen jetzt",
        ),
    ]);
    assert_eq!(
        validate_block(&a, 1, "Willkommen beim Wolff Applied AI Kanal.", &invented),
        Err(Reject::Invented)
    );
    // Eine einzelne erfundene Kleinigkeit unter der Grenze ist noch erlaubt (1 von 13 Woertern).
    let small = reply(&[
        (1, "willkommen beim Wolf Applied AI Kanal"),
        (2, "heute geht es um den Lastgang von 400 Kilowatt genau"),
    ]);
    assert!(validate_block(&a, 1, "", &small).is_ok());
}

#[test]
fn an_answer_with_the_wrong_length_is_rejected() {
    let a = a_small();
    // Fast alles gestrichen.
    let short = reply(&[(1, "willkommen"), (2, "heute")]);
    assert_eq!(validate_block(&a, 1, "", &short), Err(Reject::Length));
    // Doppelt so lang durch Wiederholung (alle Woerter gedeckt, aber zu lang).
    let long = reply(&[
        (1, "willkommen beim Wolf Applied AI Kanal willkommen beim Wolf Applied AI Kanal"),
        (
            2,
            "heute geht es um den Lastgang von 400 Kilowatt heute geht es um den Lastgang von 400 Kilowatt",
        ),
    ]);
    assert_eq!(validate_block(&a, 1, "", &long), Err(Reject::Length));
}

#[test]
fn the_shape_hint_names_the_expected_numbers() {
    let r = reply(&[(1, "x")]);
    assert!(shape_problem(&r, &[1, 2]).unwrap().contains("1..2"));
    assert!(shape_problem(&reply(&[(2, "x"), (1, "y")]), &[1, 2]).is_none());
}

// ---------------------------------------------------------------------------
// Bloecke und Kontext
// ---------------------------------------------------------------------------

fn long_lines(count: u32) -> Vec<StoredSegment> {
    (0..count)
        .map(|i| {
            let word = format!("wort{i}");
            seg(i, u64::from(i) * 60, &format!("{word} ").repeat(200))
        })
        .collect()
}

#[test]
fn blocks_hold_whole_lines_and_cover_every_line_once() {
    let a = long_lines(9);
    let ranges = plan_blocks(&a);
    assert!(ranges.len() > 1, "{ranges:?}");
    let mut next = 0;
    for r in &ranges {
        assert_eq!(r.start, next, "keine Luecke, keine Doppelung");
        assert!(!r.is_empty());
        next = r.end;
    }
    assert_eq!(next, a.len());
    let chars: Vec<usize> = ranges
        .iter()
        .map(|r| a[r.clone()].iter().map(|s| s.text.chars().count()).sum())
        .collect();
    assert!(chars.iter().all(|c| *c <= BLOCK_CHARS + 1300), "{chars:?}");
}

#[test]
fn the_other_version_is_shown_by_time_window_and_size() {
    let a = vec![seg(0, 100, "a"), seg(1, 110, "b")];
    let b = vec![
        seg(0, 10, "viel zu frueh"),
        seg(1, 98, "knapp davor"),
        seg(2, 105, "mittendrin"),
        seg(3, 113, "knapp danach"),
        seg(4, 200, "viel zu spaet"),
    ];
    let text = b_context(&a, &b);
    assert_eq!(text, "knapp davor\nmittendrin\nknapp danach\n");
    let many: Vec<StoredSegment> = (0..400).map(|i| seg(i, 100, &"x".repeat(100))).collect();
    assert!(b_context(&a, &many).chars().count() <= CONTEXT_CHARS);
    assert_eq!(b_context(&[], &b), "");
}

// ---------------------------------------------------------------------------
// Der ganze Lauf
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_merge_keeps_timing_and_channels_and_takes_the_corrected_text() {
    let a = a_small();
    let outcome = run(&a, &b_small(), |req| async move {
        assert!(req.user.contains("[1] willkommen"));
        assert!(req.user.contains("Wolff"), "B steht im Prompt");
        assert_eq!(req.expected, vec![1, 2]);
        Ok(reply(&[
            (1, "Willkommen beim Wolff Applied AI Kanal."),
            (2, "Heute geht es um den Lastgang von 400 Kilowattstunden."),
        ]))
    })
    .await
    .unwrap();
    assert_eq!((outcome.blocks, outcome.rejected), (1, 0));
    assert_eq!(outcome.segments.len(), a.len());
    for (merged, base) in outcome.segments.iter().zip(&a) {
        assert_eq!(
            (merged.start_ms, merged.end_ms, merged.channel),
            (base.start_ms, base.end_ms, base.channel)
        );
    }
    assert_eq!(
        outcome.segments[0].text,
        "Willkommen beim Wolff Applied AI Kanal."
    );
}

#[tokio::test]
async fn a_single_rejected_block_discards_the_whole_merge() {
    let a = a_small();
    let err = run(&a, &b_small(), |_| async {
        Ok(reply(&[
            (1, "voellig erfundener Text ueber etwas ganz anderes"),
            (2, "noch mehr erfundene Saetze ohne jeden Bezug"),
        ]))
    })
    .await
    .err()
    .unwrap();
    assert_eq!(
        err,
        MergeError::Rejected {
            rejected: 1,
            blocks: 1
        }
    );
    assert_eq!(err.code(), "merge_rejected");
}

#[tokio::test]
async fn a_rejected_block_keeps_the_text_of_the_base_when_most_blocks_pass() {
    let a = long_lines(9);
    let blocks = plan_blocks(&a).len();
    assert!(blocks >= 3, "{blocks}");
    let b: Vec<StoredSegment> = a
        .iter()
        .map(|s| seg(s.segment_index, s.start_ms / 1000, &s.text))
        .collect();
    let mut call = 0usize;
    let outcome = run(&a, &b, |req| {
        call += 1;
        let this = call;
        async move {
            // Der erste Block wird erfunden, alle anderen kommen unveraendert zurueck.
            let lines: Vec<(u32, String)> = req
                .expected
                .iter()
                .map(|n| {
                    let text = if this == 1 {
                        "voellig anderer erfundener Text ohne Bezug".to_string()
                    } else {
                        req.user
                            .lines()
                            .find(|l| l.starts_with(&format!("[{n}] ")))
                            .unwrap()
                            .split_once("] ")
                            .unwrap()
                            .1
                            .to_string()
                    };
                    (*n, text)
                })
                .collect();
            Ok(MergeReply {
                lines: lines
                    .into_iter()
                    .map(|(n, text)| ReplyLine { n, text })
                    .collect(),
            })
        }
    })
    .await
    .unwrap();
    assert_eq!(outcome.rejected, 1);
    assert_eq!(outcome.blocks, blocks);
    // Der durchgefallene Block hat den A-Text, nicht den erfundenen.
    assert_eq!(outcome.segments[0].text, a[0].text);
    assert!(outcome
        .segments
        .iter()
        .all(|s| !s.text.contains("erfundener")));
}

#[tokio::test]
async fn a_model_failure_aborts_without_a_result_and_empty_inputs_are_refused() {
    let err = run(&a_small(), &b_small(), |_| async {
        Err("Verbindung verloren".to_string())
    })
    .await
    .err()
    .unwrap();
    assert_eq!(err, MergeError::Llm("Verbindung verloren".to_string()));
    let empty = vec![seg(0, 1, "  ")];
    for (a, b) in [
        (Vec::new(), b_small()),
        (a_small(), Vec::new()),
        (empty.clone(), b_small()),
        (a_small(), empty),
    ] {
        let err = run(&a, &b, |_| async {
            unreachable!("kein Aufruf bei leerer Eingabe")
        })
        .await
        .err()
        .unwrap();
        assert_eq!(err, MergeError::Empty);
    }
}

// ---------------------------------------------------------------------------
// Mit Modell (Mock-Server), Fassung und Herkunft
// ---------------------------------------------------------------------------

mod with_model {
    use std::path::Path;
    use std::sync::Arc;

    use serde_json::json;

    use super::super::*;
    use super::{a_small, b_small};
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

    /// Besprechung mit Fassung A (aktiv) und B (Untertitel).
    fn two_variants(fx: &Fx) -> (Arc<MeetingStore>, String, String, String) {
        let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
        let meeting = store
            .create_meeting("Video", MeetingSource::Youtube, None)
            .unwrap()
            .id;
        store.set_status(&meeting, MeetingStatus::Ready).unwrap();
        store
            .append_delta(
                &meeting,
                &TranscriptDelta {
                    new_segments: a_small(),
                },
            )
            .unwrap();
        let mut conn = store.get_connection().unwrap();
        let a = variants::list(&mut conn, &meeting).unwrap()[0].id.clone();
        let b = variants::add(
            &mut conn,
            NewVariant {
                meeting_id: meeting.clone(),
                kind: variants::KIND_SUBTITLES_MANUAL,
                language: Some("de".into()),
                model: None,
                segments: b_small(),
                activate: false,
            },
        )
        .unwrap();
        (store, meeting, a, b)
    }

    const GOOD: &str = r#"{"lines":[{"n":1,"text":"Willkommen beim Wolff Applied AI Kanal."},{"n":2,"text":"Heute geht es um den Lastgang von 400 Kilowattstunden."}]}"#;

    #[tokio::test]
    async fn a_merge_creates_a_third_variant_with_both_sources_model_and_tokens() {
        ensure_ledger();
        let port = spawn_llm_mock_with(|_| MockReply::Body(body(GOOD, 120, 45))).await;
        let settings = settings_with_mock_provider(port);
        let fx = Fx::new();
        let (store, meeting, a, b) = two_variants(&fx);

        let merged = merge_variants(&settings, &store, &meeting, &a, &b)
            .await
            .unwrap();
        assert_eq!(merged.kind, variants::KIND_MERGED);
        assert!(
            !merged.active,
            "die Zusammenfuehrung ersetzt nichts von selbst"
        );
        assert_eq!(merged.segment_count, 2);
        let mut conn = store.get_connection().unwrap();
        let list = variants::list(&mut conn, &meeting).unwrap();
        assert_eq!(list.len(), 3, "A, B und die dritte");
        assert_eq!(list.iter().filter(|v| v.active).count(), 1);
        let (_v, segments) = variants::get_segments(&conn, &merged.id).unwrap();
        assert_eq!(segments[0].text, "Willkommen beim Wolff Applied AI Kanal.");

        let entries = provenance::list(&conn, SubjectKind::TranscriptVariant, &merged.id).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.operation, "merge");
        let refs: Vec<&str> = e.sources.iter().map(|s| s.reference.as_str()).collect();
        assert_eq!(
            refs,
            vec![a.as_str(), b.as_str()],
            "Quellen = beide Fassungen"
        );
        assert!(e.model_id.is_some(), "Modell steht im Eintrag");
        assert_eq!(e.prompt_tokens, Some(120));
        assert_eq!(e.completion_tokens, Some(45));
        assert!(e.usage_event_id.is_some(), "Verweis auf das Ledger");
        let params: serde_json::Value =
            serde_json::from_str(e.params_json.as_deref().unwrap()).unwrap();
        assert_eq!(params["rejected_blocks"], 0);
        assert_eq!(params["blocks"], 1);
    }

    #[tokio::test]
    async fn an_invented_answer_is_discarded_and_leaves_no_variant() {
        ensure_ledger();
        let invented = r#"{"lines":[{"n":1,"text":"Voellig anderer erfundener Text ohne jeden Bezug"},{"n":2,"text":"Noch mehr frei erfundene Saetze ueber Raketen"}]}"#;
        let port = spawn_llm_mock_with(move |_| MockReply::Body(body(invented, 10, 5))).await;
        let settings = settings_with_mock_provider(port);
        let fx = Fx::new();
        let (store, meeting, a, b) = two_variants(&fx);
        let err = merge_variants(&settings, &store, &meeting, &a, &b)
            .await
            .unwrap_err();
        assert!(err.starts_with("merge_rejected"), "{err}");
        let mut conn = store.get_connection().unwrap();
        assert_eq!(
            variants::list(&mut conn, &meeting).unwrap().len(),
            2,
            "keine dritte Fassung"
        );
    }

    #[tokio::test]
    async fn a_reply_that_breaks_the_schema_is_discarded_too() {
        ensure_ledger();
        let wrong = r#"{"lines":[{"n":1,"text":"Willkommen beim Wolff Applied AI Kanal."}]}"#;
        let port = spawn_llm_mock_with(move |_| MockReply::Body(body(wrong, 10, 5))).await;
        let settings = settings_with_mock_provider(port);
        let fx = Fx::new();
        let (store, meeting, a, b) = two_variants(&fx);
        let err = merge_variants(&settings, &store, &meeting, &a, &b)
            .await
            .unwrap_err();
        assert!(err.starts_with("merge_rejected"), "{err}");
    }

    #[tokio::test]
    async fn argument_errors_come_before_any_model_call() {
        ensure_ledger();
        let fx = Fx::new();
        let (store, meeting, a, b) = two_variants(&fx);
        // Ohne Modell: der Code der Einrichtung, kein Netzzugriff.
        let defaults = get_default_settings();
        let err = merge_variants(&defaults, &store, &meeting, &a, &b)
            .await
            .unwrap_err();
        assert!(err == "no_provider" || err == "no_model", "{err}");
        assert_eq!(
            merge_variants(&defaults, &store, &meeting, &a, &a)
                .await
                .unwrap_err(),
            "merge_same_variant"
        );
        // Fassung einer anderen Besprechung.
        let other = store
            .create_meeting("x", MeetingSource::Import, None)
            .unwrap()
            .id;
        assert_eq!(
            merge_variants(&defaults, &store, &other, &a, &b)
                .await
                .unwrap_err(),
            "variant_not_found"
        );
    }
}
