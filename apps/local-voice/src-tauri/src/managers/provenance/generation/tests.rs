//! Erfassung der Modellaufrufe, Bau der Eintraege und die Verdrahtung an den
//! Erzeugungsstellen (Protokoll, KI-Notizen, Follow-up, STT).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::*;
use crate::managers::integrations::test_support::Fx;
use crate::managers::meetings::llm_call::test_support::{
    settings_with_mock_provider, spawn_llm_mock_with, MockReply,
};
use crate::managers::meetings::store::{
    MeetingSource, MeetingStatus, StoredSegment, TranscriptDelta,
};
use crate::managers::provenance::{self, ProvenanceOrigin};
use crate::managers::usage::{self, CapturedCall, Purpose, TokenUsage, UsageLedger};
use crate::settings::{get_default_settings, PostProcessProvider};

// -- Bausteine -----------------------------------------------------------------

/// Ein Ledger fuer diesen Testprozess (das globale Ledger ist ein `OnceLock`).
fn ensure_ledger() -> Arc<UsageLedger> {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        if usage::ledger().is_none() {
            let ledger = UsageLedger::open(Path::new(":memory:")).unwrap();
            usage::install_globals(Arc::new(ledger), Arc::new(get_default_settings));
        }
    });
    usage::ledger().expect("Ledger installiert")
}

fn provider(id: &str, base_url: &str) -> PostProcessProvider {
    PostProcessProvider {
        id: id.into(),
        label: id.to_uppercase(),
        base_url: base_url.into(),
        allow_base_url_edit: false,
        models_endpoint: None,
        supports_structured_output: false,
    }
}

fn call(
    model: &str,
    prompt: u64,
    completion: u64,
    ms: u32,
    ok: bool,
    id: Option<i64>,
) -> CapturedCall {
    CapturedCall {
        purpose: "minutes".into(),
        provider_id: "openai".into(),
        local: false,
        connection_label: "OpenAI privat".into(),
        model_id: model.into(),
        model_label: model.to_uppercase(),
        prompt_tokens: prompt,
        completion_tokens: completion,
        duration_ms: ms,
        ok,
        usage_event_id: id,
    }
}

fn generation<'a>(subject: &'a str, started: Instant) -> Generation<'a> {
    Generation {
        subject_kind: SubjectKind::Document,
        subject_id: subject,
        subject_revision: Some(2),
        operation: "minutes",
        actor_kind: ActorKind::User,
        actor_ref: None,
        started,
        sources: vec![SourceRef::new("transcript", "m-1", None)],
        params: json!({ "template_id": "builtin:allgemein" }),
        fallback: None,
    }
}

fn params_of(e: &NewProvenance) -> serde_json::Value {
    e.params.clone().expect("params")
}

// -- build -----------------------------------------------------------------------

#[test]
fn build_sums_tokens_and_takes_the_model_of_the_last_successful_call() {
    let calls = vec![
        call("gpt-4.1-mini", 500, 50, 800, true, Some(10)),
        call("gpt-4.1", 4_000, 600, 9_000, true, Some(11)),
        call("gpt-4.1", 0, 0, 100, false, Some(12)),
    ];
    let e = build(&generation("d", Instant::now()), &calls, 0);
    assert_eq!(
        e.model_id.as_deref(),
        Some("gpt-4.1"),
        "letzter ERFOLGREICHER Aufruf"
    );
    assert_eq!(e.model_label.as_deref(), Some("GPT-4.1"));
    assert_eq!(e.provider.as_deref(), Some("openai"));
    assert_eq!(e.locality, Some(Locality::Remote));
    assert_eq!(
        (e.prompt_tokens, e.completion_tokens),
        (Some(4_500), Some(650))
    );
    assert_eq!(e.subject_revision, Some(2));
    assert_eq!(e.actor_kind, ActorKind::User);
    let p = params_of(&e);
    assert_eq!(p["calls"], 3);
    assert_eq!(p["failed_calls"], 1);
    assert_eq!(p["llm_ms"], 9_900);
    assert_eq!(p["connection"], "OpenAI privat");
    assert_eq!(
        p["models"],
        json!(["gpt-4.1", "gpt-4.1-mini"]),
        "mehrere Modelle stehen in params"
    );
    assert_eq!(
        p["template_id"], "builtin:allgemein",
        "uebergebene Angaben bleiben"
    );
}

#[test]
fn build_points_at_the_first_ledger_event_and_lists_all_of_them() {
    let calls = vec![
        call("m", 10, 1, 5, true, Some(42)),
        call("m", 10, 1, 5, true, None),
        call("m", 10, 1, 5, true, Some(40)),
        call("m", 10, 1, 5, true, Some(41)),
    ];
    let e = build(&generation("d", Instant::now()), &calls, 0);
    assert_eq!(e.usage_event_id, Some(40), "die kleinste Nummer des Laufs");
    assert_eq!(params_of(&e)["usage_event_ids"], json!([42, 40, 41]));
    assert!(
        params_of(&e).get("models").is_none(),
        "ein Modell: keine Liste"
    );
}

#[test]
fn build_leaves_tokens_unknown_when_the_provider_reports_none() {
    let e = build(
        &generation("d", Instant::now()),
        &[call("m", 0, 0, 5, true, Some(1))],
        0,
    );
    assert_eq!((e.prompt_tokens, e.completion_tokens), (None, None));
    assert_eq!(e.usage_event_id, Some(1));
}

#[test]
fn build_without_calls_falls_back_to_the_provider_and_keeps_the_rest_empty() {
    let local = provider("local", "http://127.0.0.1:0/v1");
    let mut g = generation("d", Instant::now());
    g.fallback = Some(Fallback {
        provider: &local,
        model: "llm-gemma4-e4b-q4",
    });
    let e = build(&g, &[], 0);
    assert_eq!(e.provider.as_deref(), Some("local"));
    assert_eq!(e.locality, Some(Locality::Local));
    assert_eq!(e.model_id.as_deref(), Some("llm-gemma4-e4b-q4"));
    assert_eq!(
        (e.prompt_tokens, e.completion_tokens, e.usage_event_id),
        (None, None, None)
    );
    assert!(params_of(&e).get("calls").is_none());

    // Ganz ohne Angaben: kein Modell, aber ein gueltiger Eintrag.
    let e = build(&generation("d", Instant::now()), &[], 0);
    assert_eq!(
        (e.model_id.clone(), e.provider.clone(), e.locality),
        (None, None, None)
    );
    assert!(provenance::validate(&e).is_ok());
}

#[test]
fn build_measures_the_wall_clock_of_the_whole_run_and_counts_dropped_calls() {
    let started = Instant::now() - Duration::from_millis(1_500);
    let e = build(
        &generation("d", started),
        &[call("m", 1, 1, 10, true, None)],
        7,
    );
    assert!(
        e.duration_ms.unwrap() >= 1_500,
        "Wanduhrzeit, nicht die Summe der Modellzeiten"
    );
    let p = params_of(&e);
    assert_eq!(p["dropped_calls"], 7);
    assert_eq!(p["calls"], 8, "erfasste plus verworfene");
    assert_eq!(p["llm_ms"], 10);
}

#[test]
fn build_ignores_params_that_are_not_an_object() {
    let mut g = generation("d", Instant::now());
    g.params = json!("kein objekt");
    let e = build(&g, &[call("m", 1, 1, 1, true, None)], 0);
    assert!(params_of(&e).is_object());
    g.params = serde_json::Value::Null;
    assert!(params_of(&build(&g, &[], 0)).is_object());
}

#[test]
fn a_huge_event_list_is_capped_in_the_params() {
    let calls: Vec<CapturedCall> = (1..=300)
        .map(|i| call("m", 1, 1, 1, true, Some(i)))
        .collect();
    let e = build(&generation("d", Instant::now()), &calls, 0);
    assert_eq!(
        params_of(&e)["usage_event_ids"].as_array().unwrap().len(),
        100
    );
    assert_eq!(e.usage_event_id, Some(1));
    assert!(provenance::validate(&e).is_ok());
}

// -- Anbieter lokal oder entfernt ------------------------------------------------

#[test]
fn a_provider_counts_as_local_when_it_is_the_built_in_one_or_listens_on_loopback() {
    for (id, url, expected) in [
        ("local", "http://placeholder/v1", true),
        ("ollama", "http://localhost:11434/v1", true),
        ("vllm", "http://127.0.0.1:8000/v1", true),
        ("custom", "http://[::1]:8080/v1", true),
        ("custom", "http://LOCALHOST/v1", true),
        ("openai", "https://api.openai.com/v1", false),
        ("custom", "http://192.168.1.20:11434/v1", false),
        ("custom", "https://localhost.example.com/v1", false),
        ("custom", "kein url", false),
        ("custom", "", false),
    ] {
        assert_eq!(
            usage::provider_is_local(&provider(id, url)),
            expected,
            "{id} {url}"
        );
    }
}

// -- Erfassungsbereich --------------------------------------------------------------

#[tokio::test]
async fn a_capture_scope_collects_each_call_with_its_ledger_event() {
    let ledger = ensure_ledger();
    let p = provider("openai", "https://api.openai.com/v1");
    let (calls, dropped) = usage::with_capture(async {
        usage::record_call(
            Purpose::Minutes,
            &p,
            "gpt-4.1",
            TokenUsage {
                prompt_tokens: 1_234,
                completion_tokens: 56,
            },
            2_500,
            Ok(()),
        )
        .await;
        usage::record_call(
            Purpose::Minutes,
            &p,
            "gpt-4.1",
            TokenUsage::default(),
            90,
            Err("HTTP 500".into()),
        )
        .await;
        usage::captured_calls()
    })
    .await;
    assert_eq!(dropped, 0);
    assert_eq!(calls.len(), 2);
    assert!(calls[0].ok && !calls[1].ok);
    assert_eq!(
        (
            calls[0].prompt_tokens,
            calls[0].completion_tokens,
            calls[0].duration_ms
        ),
        (1_234, 56, 2_500)
    );
    assert_eq!(
        (calls[0].provider_id.as_str(), calls[0].model_id.as_str()),
        ("openai", "gpt-4.1")
    );
    assert!(!calls[0].local);
    // Die Nummer zeigt auf genau das Ereignis im Ledger.
    let id = calls[0].usage_event_id.expect("Ereignisnummer");
    let event = ledger
        .events(100_000, 0)
        .unwrap()
        .into_iter()
        .find(|e| e.id == id)
        .expect("Ereignis im Ledger");
    assert_eq!(
        (
            event.purpose.as_str(),
            event.prompt_tokens,
            event.completion_tokens,
            event.duration_ms
        ),
        ("minutes", 1_234, 56, 2_500)
    );
    assert!(
        calls[1].usage_event_id.is_some() && calls[1].usage_event_id != calls[0].usage_event_id
    );
}

#[tokio::test]
async fn calls_outside_a_scope_are_not_captured_and_do_not_break() {
    ensure_ledger();
    let p = provider("openai", "https://api.openai.com/v1");
    usage::record_call(Purpose::Chat, &p, "m", TokenUsage::default(), 1, Ok(())).await;
    assert_eq!(usage::captured_calls(), (Vec::new(), 0));
    // Ein Bereich sieht nur, was in ihm geschieht.
    let (before, _) = usage::with_capture(async { usage::captured_calls() }).await;
    assert!(before.is_empty());
}

#[tokio::test]
async fn a_spawned_task_is_not_part_of_the_scope() {
    ensure_ledger();
    let p = provider("openai", "https://api.openai.com/v1");
    let (calls, _) = usage::with_capture(async {
        let p2 = p.clone();
        tokio::spawn(async move {
            usage::record_call(Purpose::Chat, &p2, "m", TokenUsage::default(), 1, Ok(())).await;
        })
        .await
        .unwrap();
        usage::captured_calls()
    })
    .await;
    assert!(calls.is_empty(), "abgespaltene Tasks gehoeren nicht dazu");
}

#[tokio::test]
async fn nested_scopes_keep_their_calls_apart() {
    ensure_ledger();
    let p = provider("openai", "https://api.openai.com/v1");
    let (outer_calls, inner_calls) = usage::with_capture(async {
        usage::record_call(Purpose::Chat, &p, "outer", TokenUsage::default(), 1, Ok(())).await;
        let inner = usage::with_capture(async {
            usage::record_call(Purpose::Chat, &p, "inner", TokenUsage::default(), 1, Ok(())).await;
            usage::captured_calls().0
        })
        .await;
        (usage::captured_calls().0, inner)
    })
    .await;
    assert_eq!(
        outer_calls
            .iter()
            .map(|c| c.model_id.as_str())
            .collect::<Vec<_>>(),
        vec!["outer"]
    );
    assert_eq!(
        inner_calls
            .iter()
            .map(|c| c.model_id.as_str())
            .collect::<Vec<_>>(),
        vec!["inner"]
    );
}

#[tokio::test]
async fn the_number_of_captured_calls_is_bounded() {
    ensure_ledger();
    let p = provider("openai", "https://api.openai.com/v1");
    let (calls, dropped) = usage::with_capture(async {
        for _ in 0..usage::MAX_CAPTURED_CALLS + 3 {
            usage::record_call(Purpose::Chat, &p, "m", TokenUsage::default(), 1, Ok(())).await;
        }
        usage::captured_calls()
    })
    .await;
    assert_eq!(calls.len(), usage::MAX_CAPTURED_CALLS);
    assert_eq!(dropped, 3);
}

// -- Schreiben ---------------------------------------------------------------------

#[tokio::test]
async fn record_generation_writes_the_entry_from_the_current_scope() {
    ensure_ledger();
    let fx = Fx::new();
    let p = provider("openai", "https://api.openai.com/v1");
    let id = usage::with_capture(async {
        usage::record_call(
            Purpose::Summary,
            &p,
            "gpt-4.1",
            TokenUsage {
                prompt_tokens: 900,
                completion_tokens: 100,
            },
            1_000,
            Ok(()),
        )
        .await;
        record_generation(&fx.store, generation("doc-7", Instant::now()))
    })
    .await
    .expect("Eintrag geschrieben");
    let got = provenance::get(&fx.conn(), SubjectKind::Document, "doc-7").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, id);
    assert_eq!(got[0].origin, ProvenanceOrigin::Recorded);
    assert_eq!(
        (got[0].prompt_tokens, got[0].completion_tokens),
        (Some(900), Some(100))
    );
    assert!(got[0].usage_event_id.is_some());
}

#[test]
fn a_broken_provenance_table_never_fails_the_generation() {
    let fx = Fx::new();
    fx.conn().execute_batch("DROP TABLE provenance").unwrap();
    // Kein Panik, kein Fehler nach aussen: nur `None` (und eine Warnung im Log).
    assert_eq!(
        record_generation(&fx.store, generation("doc-1", Instant::now())),
        None
    );
    assert_eq!(
        record_stt(
            &fx.store,
            SttRun {
                meeting_id: "m",
                operation: "stt",
                actor_kind: ActorKind::Auto,
                model_id: "whisper",
                revision: None,
                duration_ms: 1,
                sources: vec![],
                params: json!({}),
            }
        ),
        None
    );
}

#[test]
fn an_entry_that_fails_validation_is_dropped_with_none() {
    let fx = Fx::new();
    let mut g = generation("doc-1", Instant::now());
    g.operation = "Nicht gueltig!";
    assert_eq!(record_generation(&fx.store, g), None);
    assert!(list_all(&fx).is_empty());
}

fn list_all(fx: &Fx) -> Vec<String> {
    let conn = fx.conn();
    let mut stmt = conn.prepare("SELECT id FROM provenance").unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn a_stt_run_records_model_duration_and_source() {
    let fx = Fx::new();
    let id = record_stt(
        &fx.store,
        SttRun {
            meeting_id: "m-1",
            operation: "stt",
            actor_kind: ActorKind::Auto,
            model_id: "whisper-large-v3",
            revision: Some(4),
            duration_ms: 61_000,
            sources: vec![SourceRef::new("audio", "m-1", Some("Jour fixe"))],
            params: json!({ "audio_ms": 1_800_000, "blocks": 30 }),
        },
    )
    .unwrap();
    let e = provenance::get(&fx.conn(), SubjectKind::Transcript, "m-1")
        .unwrap()
        .remove(0);
    assert_eq!(e.id, id);
    assert_eq!(e.operation, "stt");
    assert_eq!(e.actor_kind, Some(ActorKind::Auto));
    assert_eq!(e.model_id.as_deref(), Some("whisper-large-v3"));
    assert_eq!(e.locality, Some(Locality::Local));
    assert_eq!((e.duration_ms, e.subject_revision), (Some(61_000), Some(4)));
    assert_eq!(
        (e.prompt_tokens, e.completion_tokens, e.usage_event_id),
        (None, None, None)
    );
    assert_eq!(e.sources[0].kind, "audio");
    assert!(e.params_json.unwrap().contains("audio_ms"));
}

#[test]
fn a_second_transcription_adds_an_entry_and_the_latest_comes_last() {
    let fx = Fx::new();
    let run = |op: &'static str| SttRun {
        meeting_id: "m-1",
        operation: op,
        actor_kind: ActorKind::User,
        model_id: "whisper",
        revision: None,
        duration_ms: 1,
        sources: vec![],
        params: json!({}),
    };
    record_stt(&fx.store, run("import")).unwrap();
    std::thread::sleep(Duration::from_millis(3));
    record_stt(&fx.store, run("retranscribe")).unwrap();
    let ops: Vec<String> = provenance::get(&fx.conn(), SubjectKind::Transcript, "m-1")
        .unwrap()
        .into_iter()
        .map(|e| e.operation)
        .collect();
    assert_eq!(ops, vec!["import", "retranscribe"]);
}

#[test]
fn a_subtitle_import_records_the_file_as_source_and_no_model() {
    let fx = Fx::new();
    record_subtitle_import(&fx.store, "m-2", "interview.vtt", 12).unwrap();
    let e = provenance::get(&fx.conn(), SubjectKind::Transcript, "m-2")
        .unwrap()
        .remove(0);
    assert_eq!(e.operation, "subtitles_import");
    assert_eq!(e.actor_kind, Some(ActorKind::User));
    assert_eq!((e.model_id, e.provider, e.locality), (None, None, None));
    assert_eq!(
        e.sources,
        vec![SourceRef::new(
            "subtitle",
            "interview.vtt",
            Some("interview.vtt")
        )]
    );
    assert!(e.params_json.unwrap().contains("12"));
}

// -- Verdrahtung an den Erzeugungsstellen ------------------------------------------------

fn body_with_usage(content: &str, prompt: u64, completion: u64) -> String {
    json!({
        "choices": [{ "message": { "role": "assistant", "content": content } }],
        "usage": { "prompt_tokens": prompt, "completion_tokens": completion }
    })
    .to_string()
}

/// Eine fertige Besprechung mit `n` Segmenten (Index 0 bis n-1).
fn ready_meeting(fx: &Fx, n: u32) -> String {
    let meeting = fx
        .store
        .create_meeting("Kundengespräch", MeetingSource::Live, Some(1_755_600_000))
        .unwrap();
    let segments: Vec<StoredSegment> = (0..n)
        .map(|i| StoredSegment {
            segment_index: i,
            text: format!(
                "Aussage Nummer {i} zum Projekt und zum Zeitplan. {}",
                "Weitere Einzelheiten zu den offenen Punkten. ".repeat(3)
            ),
            start_ms: u64::from(i) * 5_000,
            end_ms: u64::from(i) * 5_000 + 4_000,
            channel: (i % 2) as u8,
            speaker_index: None,
            words: None,
        })
        .collect();
    fx.store
        .append_delta(
            &meeting.id,
            &TranscriptDelta {
                new_segments: segments,
            },
        )
        .unwrap();
    fx.store
        .set_status(&meeting.id, MeetingStatus::Ready)
        .unwrap();
    meeting.id
}

fn event_of(ledger: &UsageLedger, id: i64) -> crate::managers::usage::UsageEvent {
    ledger
        .events(100_000, 0)
        .unwrap()
        .into_iter()
        .find(|e| e.id == id)
        .expect("Ereignis im Ledger")
}

fn minutes_answer() -> String {
    json!({
        "zusammenfassung": ["Der Go-Live wurde auf den 1. September gelegt."],
        "besprochene_punkte": ["Testphase ist abgeschlossen"],
        "entscheidungen": ["Go-Live am 1. September"],
        "aufgaben": [{ "text": "Release-Notes schreiben", "assignee": null, "due": null }],
        "offene_fragen": []
    })
    .to_string()
}

#[tokio::test]
async fn creating_minutes_records_model_tokens_duration_and_the_ledger_event() {
    let ledger = ensure_ledger();
    let port =
        spawn_llm_mock_with(|_| MockReply::Body(body_with_usage(&minutes_answer(), 1_000, 200)))
            .await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let meeting_id = ready_meeting(&fx, 3);

    let doc = crate::managers::meetings::minutes::generate_minutes_with_settings(
        &settings,
        Arc::new(crate::managers::meetings::store::MeetingStore::open_at(&fx.db_path).unwrap()),
        &meeting_id,
        None,
        &|_| {},
    )
    .await
    .unwrap();

    let entries = provenance::get(&fx.conn(), SubjectKind::Document, &doc.id).unwrap();
    assert_eq!(entries.len(), 1, "genau ein Eintrag je Protokoll");
    let e = &entries[0];
    assert_eq!(e.origin, ProvenanceOrigin::Recorded);
    assert_eq!(e.operation, "minutes");
    assert_eq!(e.actor_kind, Some(ActorKind::User));
    assert_eq!(e.model_id.as_deref(), Some("test-model"), "Modell");
    assert_eq!(e.provider.as_deref(), Some("custom"));
    assert_eq!(
        e.locality,
        Some(Locality::Local),
        "der Mock lauscht auf Loopback"
    );
    assert_eq!(
        (e.prompt_tokens, e.completion_tokens),
        (Some(1_000), Some(200)),
        "Token"
    );
    assert!(e.duration_ms.is_some(), "Dauer");
    assert_eq!(e.subject_revision, None);
    assert_eq!(e.sources.len(), 1);
    assert_eq!(
        (e.sources[0].kind.as_str(), e.sources[0].reference.as_str()),
        ("transcript", meeting_id.as_str())
    );
    let params: serde_json::Value =
        serde_json::from_str(e.params_json.as_deref().unwrap()).unwrap();
    assert_eq!(params["single_pass"], true);
    assert_eq!(params["calls"], 1);

    // Der Verweis zeigt auf das echte Ereignis im Ledger.
    let event_id = e.usage_event_id.expect("usage_event_id");
    let event = event_of(&ledger, event_id);
    assert_eq!(event.purpose, "minutes");
    assert_eq!((event.prompt_tokens, event.completion_tokens), (1_000, 200));
    assert!(event.ok);
}

#[tokio::test]
async fn two_minutes_runs_give_two_documents_with_their_own_entries() {
    ensure_ledger();
    let port =
        spawn_llm_mock_with(|_| MockReply::Body(body_with_usage(&minutes_answer(), 10, 5))).await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let meeting_id = ready_meeting(&fx, 3);
    let store =
        Arc::new(crate::managers::meetings::store::MeetingStore::open_at(&fx.db_path).unwrap());
    let mut ids = Vec::new();
    for _ in 0..2 {
        let doc = crate::managers::meetings::minutes::generate_minutes_with_settings(
            &settings,
            store.clone(),
            &meeting_id,
            None,
            &|_| {},
        )
        .await
        .unwrap();
        ids.push(doc.id);
    }
    assert_ne!(ids[0], ids[1]);
    let conn = fx.conn();
    let first = provenance::get(&conn, SubjectKind::Document, &ids[0]).unwrap();
    let second = provenance::get(&conn, SubjectKind::Document, &ids[1]).unwrap();
    assert_eq!((first.len(), second.len()), (1, 1));
    assert_ne!(
        first[0].usage_event_id, second[0].usage_event_id,
        "je Lauf ein eigenes Ereignis"
    );
}

#[tokio::test]
async fn a_failed_minutes_run_leaves_no_provenance_behind() {
    ensure_ledger();
    let port = spawn_llm_mock_with(|_| MockReply::Status(500)).await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let meeting_id = ready_meeting(&fx, 2);
    let store =
        Arc::new(crate::managers::meetings::store::MeetingStore::open_at(&fx.db_path).unwrap());
    let result = crate::managers::meetings::minutes::generate_minutes_with_settings(
        &settings,
        store,
        &meeting_id,
        None,
        &|_| {},
    )
    .await;
    assert!(result.is_err());
    assert!(list_all(&fx).is_empty(), "ohne Dokument keine Herkunft");
}

#[tokio::test]
async fn creating_enhanced_notes_records_the_origin_with_sources() {
    let ledger = ensure_ledger();
    let notes_answer = json!({
        "zusammenfassung": [{"ref": null, "text": "Der Kunde will im Herbst starten.", "sources": ["S1"]}],
        "besprochene_punkte": [{"ref": null, "text": "Budget ist offen.", "sources": ["S3"]}],
        "entscheidungen": [],
        "aufgaben": [{"ref": null, "text": "Angebot senden", "sources": ["S4"], "assignee": "Frau Meyer", "due": null}],
        "offene_fragen": []
    })
    .to_string();
    let port =
        spawn_llm_mock_with(move |_| MockReply::Body(body_with_usage(&notes_answer, 2_000, 300)))
            .await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let meeting_id = ready_meeting(&fx, 6);
    let store =
        Arc::new(crate::managers::meetings::store::MeetingStore::open_at(&fx.db_path).unwrap());

    let doc = crate::managers::meetings::notes::enhance::enhance_meeting(
        &settings,
        store,
        &meeting_id,
        None,
        |_, _| {},
    )
    .await
    .unwrap();

    let e = provenance::get(&fx.conn(), SubjectKind::Document, &doc.id)
        .unwrap()
        .remove(0);
    assert_eq!(e.origin, ProvenanceOrigin::Recorded);
    assert_eq!(e.operation, "notes");
    assert_eq!(e.model_id.as_deref(), Some("test-model"));
    assert_eq!(
        (e.prompt_tokens, e.completion_tokens),
        (Some(2_000), Some(300))
    );
    assert_eq!(e.subject_revision, Some(i64::from(doc.version)));
    assert_eq!(e.sources[0].kind, "transcript");
    assert!(e.duration_ms.is_some());
    assert_eq!(
        event_of(&ledger, e.usage_event_id.unwrap()).purpose,
        "enhanced_notes"
    );
}

#[tokio::test]
async fn a_follow_up_draft_records_its_origin_under_the_export_subject() {
    let ledger = ensure_ledger();
    let port = spawn_llm_mock_with(|_| {
        MockReply::Body(body_with_usage(
            "Betreff: Follow-up zum Gespräch\n\nHallo Anna,\n\nvielen Dank für das Gespräch. Wir senden das Angebot bis Freitag.\n\nViele Grüße",
            3_000,
            120,
        ))
    })
    .await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let meeting_id = ready_meeting(&fx, 4);
    let store =
        Arc::new(crate::managers::meetings::store::MeetingStore::open_at(&fx.db_path).unwrap());

    let draft = crate::managers::meetings::followup::draft(
        &settings,
        store,
        &meeting_id,
        "Anna",
        vec![],
        crate::managers::meetings::followup::RunLimits::default(),
    )
    .await
    .unwrap();
    assert!(draft.body_text.contains("Angebot"));

    let subject = crate::managers::meetings::followup::provenance_subject(&meeting_id);
    let e = provenance::get(&fx.conn(), SubjectKind::Export, &subject)
        .unwrap()
        .remove(0);
    assert_eq!(e.operation, "followup");
    assert_eq!(e.model_id.as_deref(), Some("test-model"));
    assert_eq!(
        (e.prompt_tokens, e.completion_tokens),
        (Some(3_000), Some(120))
    );
    assert_eq!(e.sources[0].kind, "meeting");
    assert_eq!(
        event_of(&ledger, e.usage_event_id.unwrap()).purpose,
        "followup"
    );
}

#[tokio::test]
async fn an_unreadable_provenance_table_does_not_stop_the_minutes_from_being_written() {
    ensure_ledger();
    let port =
        spawn_llm_mock_with(|_| MockReply::Body(body_with_usage(&minutes_answer(), 10, 5))).await;
    let settings = settings_with_mock_provider(port);
    let fx = Fx::new();
    let meeting_id = ready_meeting(&fx, 3);
    fx.conn().execute_batch("DROP TABLE provenance").unwrap();
    let store =
        Arc::new(crate::managers::meetings::store::MeetingStore::open_at(&fx.db_path).unwrap());
    // `open_at` aendert an der gedroppten Tabelle nichts (Migration schon durch).
    let doc = crate::managers::meetings::minutes::generate_minutes_with_settings(
        &settings,
        store,
        &meeting_id,
        None,
        &|_| {},
    )
    .await
    .expect("das Protokoll entsteht trotzdem");
    // Die Herkunft kommt aus den Altdaten.
    let derived = provenance::derive_legacy(&fx.conn(), SubjectKind::Document, &doc.id)
        .unwrap()
        .expect("aus generation_metadata_json");
    assert_eq!(derived.model_id.as_deref(), Some("test-model"));
    assert_eq!(derived.origin, ProvenanceOrigin::Derived);
}

#[test]
fn every_purpose_has_its_own_stable_snake_case_name() {
    let all = [
        (Purpose::PostProcess, "post_process"),
        (Purpose::Minutes, "minutes"),
        (Purpose::EnhancedNotes, "enhanced_notes"),
        (Purpose::Summary, "summary"),
        (Purpose::Tagging, "tagging"),
        (Purpose::Translation, "translation"),
        (Purpose::Chat, "chat"),
        (Purpose::Followup, "followup"),
        // A1: die Zwecke der naechsten Pakete stehen schon in der festen Liste.
        (Purpose::TranscriptMerge, "transcript_merge"),
        (Purpose::Relevance, "relevance"),
        (Purpose::Reconcile, "reconcile"),
        (Purpose::FactCheck, "fact_check"),
        (Purpose::AgentRoute, "agent_route"),
        (Purpose::Extract, "extract"),
    ];
    let mut seen = std::collections::HashSet::new();
    for (purpose, name) in all {
        assert_eq!(purpose.as_str(), name);
        assert!(seen.insert(name), "{name} doppelt");
        // Dieselbe Schreibweise wie in der Datenbank und im JSON.
        assert_eq!(serde_json::to_value(purpose).unwrap(), json!(name));
    }
}
