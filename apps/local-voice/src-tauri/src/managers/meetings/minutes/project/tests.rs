//! G3 (#70): Laeufe des Projekt-Protokolls gegen einen Mock-LLM und eine echte
//! (temporaere) Datenbank. Die reine Logik (Bloecke, Belege, Markdown) pruefen die
//! Tests in `corpus.rs` und `result.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use super::*;
use crate::managers::meetings::llm_call::test_support::{
    chat_body, settings_with_mock_provider, spawn_llm_mock_with, MockReply,
};
use crate::managers::meetings::search::index::tests::{ready_meeting, tmp_store};
use crate::managers::meetings::store::{MeetingStatus, StoredSegment, TranscriptDelta};

// -- Fixture -----------------------------------------------------------------------------------

struct Fx {
    store: Arc<MeetingStore>,
    _dir: tempfile::TempDir,
    folder_id: String,
    /// Die IDs in der Reihenfolge, in der die Aufnahmen angelegt wurden.
    ids: Vec<String>,
}

fn segment(index: u32, text: &str) -> StoredSegment {
    StoredSegment {
        segment_index: index,
        text: text.into(),
        start_ms: u64::from(index) * 60_000,
        end_ms: u64::from(index) * 60_000 + 50_000,
        channel: (index % 2) as u8,
        speaker_index: None,
        words: None,
    }
}

/// Ein Projekt mit Aufnahmen `(Titel, Start, Zahl der Segmente, Segmenttext)`.
fn fixture(recordings: &[(&str, i64, u32, &str)]) -> Fx {
    let (dir, store) = tmp_store();
    let folder_id = store
        .folder_save(None, "Kunde Stadtwerke", None)
        .unwrap()
        .id;
    let mut ids = Vec::new();
    for (title, started, count, text) in recordings {
        let meeting = ready_meeting(&store, title, *started);
        if *count > 0 {
            let segments: Vec<StoredSegment> = (0..*count)
                .map(|i| segment(i, &format!("{text} Nummer {i}.")))
                .collect();
            store
                .append_delta(
                    &meeting.id,
                    &TranscriptDelta {
                        new_segments: segments,
                    },
                )
                .unwrap();
        }
        store
            .set_meeting_folders(&meeting.id, &[folder_id.clone()])
            .unwrap();
        ids.push(meeting.id);
    }
    Fx {
        store: Arc::new(store),
        _dir: dir,
        folder_id,
        ids,
    }
}

fn three() -> Fx {
    fixture(&[
        ("Spaet", 1_790_200_000, 3, "Das Angebot wird besprochen."),
        ("Frueh", 1_790_000_000, 3, "Der Auftrag wird vorbereitet."),
        ("Mitte", 1_790_100_000, 3, "Der Termin steht fest."),
    ])
}

fn no_ram() -> u64 {
    0
}

fn low_ram() -> u64 {
    512
}

fn limits(budget_chars: usize) -> Limits {
    Limits {
        budget_chars: Some(budget_chars),
        timeout: Duration::from_secs(60),
        classify_timeout: Duration::from_secs(30),
        free_mb: no_ram,
    }
}

async fn run_with(
    fx: &Fx,
    settings: &AppSettings,
    ids: &[String],
    template: Option<&str>,
    kind: ProjectKind,
    limits: Limits,
) -> Result<ProjectMinutes, MinutesError> {
    generate_guarded(
        settings,
        fx.store.clone(),
        &Request {
            folder_id: &fx.folder_id,
            meeting_ids: ids,
            template_id: template,
            kind,
        },
        limits,
        &|_| {},
    )
    .await
}

async fn run(fx: &Fx, settings: &AppSettings) -> Result<ProjectMinutes, MinutesError> {
    run_with(
        fx,
        settings,
        &fx.ids,
        None,
        ProjectKind::Minutes,
        limits(1_000_000),
    )
    .await
}

fn is_classify(body: &str) -> bool {
    body.contains("Templates (choose one id)")
}

fn is_map(body: &str) -> bool {
    body.contains("This is part ")
}

fn is_reduce(body: &str) -> bool {
    body.contains("# Partial results")
}

/// Die erste Quellen-ID (`R<n>S<m>`) im Text.
fn first_id(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'R' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'S' {
                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                if k > j + 1 {
                    return Some(text[i..k].to_string());
                }
            }
        }
        i += 1;
    }
    None
}

fn answer(sources: &[&str]) -> String {
    chat_body(
        &json!({
            "zusammenfassung": [{"text": "Auftrag, Angebot und Termin stehen.", "sources": sources}],
            "besprochene_punkte": [],
            "entscheidungen": [],
            "aufgaben": [{"text": "Angebot schicken", "assignee": "Anna", "due": null, "sources": sources}],
            "offene_fragen": []
        })
        .to_string(),
    )
}

fn project_rows(fx: &Fx) -> i64 {
    fx.store
        .get_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM project_minutes", [], |r| r.get(0))
        .unwrap()
}

// -- Einzeldurchlauf -----------------------------------------------------------------------------

#[tokio::test]
async fn a_run_combines_the_recordings_chronologically_with_sources_and_stores_it_in_the_project() {
    let fx = three();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = seen.clone();
    let port = spawn_llm_mock_with(move |req| {
        log.lock().unwrap().push(req.to_string());
        // Ein gueltiger Beleg je Aufnahme und ein erfundener.
        MockReply::Body(answer(&["R1S0", "R3S1", "R9S9"]))
    })
    .await;
    let settings = settings_with_mock_provider(port);

    let doc = run(&fx, &settings).await.unwrap();

    // Chronologisch: Frueh (R1), Mitte (R2), Spaet (R3) -- unabhaengig von der Auswahlreihenfolge.
    let titles: Vec<_> = doc.recordings.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, vec!["Frueh", "Mitte", "Spaet"]);
    assert_eq!(doc.recordings[0].meeting_id, fx.ids[1]);
    assert_eq!(doc.recordings[2].meeting_id, fx.ids[0]);
    assert_eq!(doc.recordings[0].segments, 3);

    // Belege: R1S0 und R3S1 gehen auf Aufnahme und Zeit zurueck, R9S9 faellt weg.
    let summary = &doc.sections[0].entries[0];
    assert_eq!(summary.sources.len(), 2);
    assert_eq!(
        (
            summary.sources[0].meeting_id.as_str(),
            summary.sources[0].segment_index,
            summary.sources[0].start_ms
        ),
        (fx.ids[1].as_str(), 0, 0)
    );
    assert_eq!(
        (
            summary.sources[1].meeting_id.as_str(),
            summary.sources[1].segment_index,
            summary.sources[1].start_ms
        ),
        (fx.ids[0].as_str(), 1, 60_000)
    );
    assert!(!summary.unsupported);
    assert_eq!(doc.meta.dropped_sources, 2, "R9S9 in zwei Eintraegen");

    // Herkunft wie bei Einzelprotokollen.
    assert_eq!(doc.meta.model, "test-model");
    assert_eq!(doc.meta.provider, "custom");
    assert_eq!(doc.meta.template_id, "builtin:allgemein");
    assert!(doc.meta.single_pass && !doc.meta.incomplete);
    assert!(doc.created_at > 0);
    assert_eq!(doc.title, "Projekt-Protokoll: Kunde Stadtwerke");
    assert!(doc
        .body
        .starts_with("# Projekt-Protokoll: Kunde Stadtwerke"));
    assert!(doc.body.contains("[A1 00:00, A3 01:00]"), "{}", doc.body);

    // Im Projekt gespeichert, erneut oeffnbar.
    assert_eq!(
        fx.store.project_minutes_list(&fx.folder_id).unwrap().len(),
        1
    );
    assert_eq!(fx.store.project_minutes_get(&doc.id).unwrap().unwrap(), doc);

    // Provenienz: ein Eintrag, der die drei Quellaufnahmen nennt.
    let conn = fx.store.get_connection().unwrap();
    let (sources_json, operation): (String, String) = conn
        .query_row(
            "SELECT sources_json, operation FROM provenance WHERE subject_id = ?1",
            [&doc.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(operation, "project_minutes");
    for id in &fx.ids {
        assert!(sources_json.contains(id.as_str()), "{sources_json}");
    }

    // Der Prompt: Projekt, alle drei Aufnahmen in dieser Reihenfolge, IDs.
    let requests = seen.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "ein Aufruf fuer alles");
    let prompt = &requests[0];
    assert!(prompt.contains("Project: Kunde Stadtwerke"));
    let at = |needle: &str| {
        prompt
            .find(needle)
            .unwrap_or_else(|| panic!("fehlt: {needle}"))
    };
    assert!(
        at("=== R1 · Frueh") < at("=== R2 · Mitte") && at("=== R2 · Mitte") < at("=== R3 · Spaet")
    );
    assert!(prompt.contains("R2S2 "));
    assert!(prompt.contains("Never invent an id"));
}

#[tokio::test]
async fn an_entry_without_a_valid_source_stays_but_is_marked_unsupported() {
    let fx = three();
    let port = spawn_llm_mock_with(|_| MockReply::Body(answer(&["R7S7"]))).await;
    let doc = run(&fx, &settings_with_mock_provider(port)).await.unwrap();
    let entry = &doc.sections[0].entries[0];
    assert!(entry.unsupported && entry.sources.is_empty());
    assert_eq!(doc.meta.unsupported_entries, 2);
    assert!(doc.body.contains("_(ohne Beleg)_"));
}

#[tokio::test]
async fn a_summary_changes_rules_and_title_but_not_the_mechanics() {
    let fx = three();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = seen.clone();
    let port = spawn_llm_mock_with(move |req| {
        log.lock().unwrap().push(req.to_string());
        MockReply::Body(answer(&["R1S0"]))
    })
    .await;
    let doc = run_with(
        &fx,
        &settings_with_mock_provider(port),
        &fx.ids,
        None,
        ProjectKind::Summary,
        limits(1_000_000),
    )
    .await
    .unwrap();
    assert_eq!(doc.kind, ProjectKind::Summary);
    assert_eq!(doc.title, "Projekt-Zusammenfassung: Kunde Stadtwerke");
    let prompt = seen.lock().unwrap()[0].clone();
    assert!(prompt.contains("This is a SUMMARY"));
    assert!(prompt.contains("summary that follows a template"));
    // Dieselbe Quelle, dieselbe Pruefung, dieselbe Ablage.
    assert_eq!(doc.sections[0].entries[0].sources[0].meeting_id, fx.ids[1]);
    assert_eq!(
        fx.store.project_minutes_list(&fx.folder_id).unwrap()[0].kind,
        ProjectKind::Summary
    );
}

#[tokio::test]
async fn the_template_is_the_users_choice_automatic_or_the_standard() {
    // Ausdruecklich: nur diese Vorlage, kein Klassifizieren.
    let fx = three();
    let classified = Arc::new(AtomicUsize::new(0));
    let counter = classified.clone();
    let port = spawn_llm_mock_with(move |req| {
        if is_classify(req) {
            counter.fetch_add(1, Ordering::SeqCst);
            return MockReply::Body(chat_body(r#"{"template_id":"builtin:vertrieb","reason":"Kundengespraech"}"#));
        }
        MockReply::Body(chat_body(
            &json!({
                "kunde_ausgangslage": [{"text": "Stadtwerke wollen glaetten.", "sources": ["R1S0"]}],
                "zusammenfassung": [{"text": "Es geht um das Glaetten.", "sources": ["R1S0"]}]
            })
            .to_string(),
        ))
    })
    .await;
    let settings = settings_with_mock_provider(port);

    let explicit = run_with(
        &fx,
        &settings,
        &fx.ids,
        Some("builtin:vertrieb"),
        ProjectKind::Minutes,
        limits(1_000_000),
    )
    .await
    .unwrap();
    assert_eq!(explicit.meta.template_id, "builtin:vertrieb");
    assert!(explicit.meta.auto.is_none());
    assert_eq!(classified.load(Ordering::SeqCst), 0);

    // Automatisch: ein Klassifizieren ueber den Inhalt aller Aufnahmen; die Wahl steht in der Herkunft.
    let auto = run_with(
        &fx,
        &settings,
        &fx.ids,
        Some("auto"),
        ProjectKind::Minutes,
        limits(1_000_000),
    )
    .await
    .unwrap();
    assert_eq!(classified.load(Ordering::SeqCst), 1);
    assert_eq!(auto.meta.template_id, "builtin:vertrieb");
    let info = auto
        .meta
        .auto
        .as_ref()
        .expect("automatische Wahl ausgewiesen");
    assert_eq!(info.template_id, "builtin:vertrieb");
    assert_eq!(info.reason, "Kundengespraech");
    assert!(auto.body.contains("(automatisch gewählt)"));
    assert_eq!(auto.sections[0].id, "kunde_ausgangslage");

    // Die automatische Wahl gehoert keiner Besprechung: keine Metadaten angefasst.
    for id in &fx.ids {
        let metadata = fx.store.metadata_json(id).unwrap();
        assert!(metadata
            .map(|m| m.get("template_auto").is_none())
            .unwrap_or(true));
    }

    // Standard und unbekannt.
    let standard = run(&fx, &settings).await.unwrap();
    assert_eq!(standard.meta.template_id, "builtin:allgemein");
    let err = run_with(
        &fx,
        &settings,
        &fx.ids,
        Some("builtin:gibt-es-nicht"),
        ProjectKind::Minutes,
        limits(1_000_000),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "template_not_found");
}

// -- Bloecke -------------------------------------------------------------------------------------

fn long_three() -> Fx {
    let long =
        "Das ist eine lange Aussage mit vielen Worten zu Angebot, Termin und Budget des Kunden.";
    fixture(&[
        ("Erste", 1_790_000_000, 4, long),
        ("Zweite", 1_790_100_000, 4, long),
        ("Dritte", 1_790_200_000, 4, long),
    ])
}

#[tokio::test]
async fn an_overlong_input_runs_in_blocks_per_recording_and_merges_with_sources() {
    let fx = long_three();
    let maps: Arc<Mutex<Vec<String>>> = Arc::default();
    let reduces = Arc::new(AtomicUsize::new(0));
    let (log, red) = (maps.clone(), reduces.clone());
    let port = spawn_llm_mock_with(move |req| {
        if is_map(req) {
            log.lock().unwrap().push(req.to_string());
            let chunk = req.split("# Transcript (part").nth(1).unwrap_or("");
            let id = first_id(chunk).unwrap_or_else(|| "R1S0".into());
            return MockReply::Body(chat_body(
                &json!({"entries": [{"section": "besprochene_punkte", "text": format!("Punkt {id}"), "assignee": null, "due": null, "sources": [id]}]})
                    .to_string(),
            ));
        }
        if is_reduce(req) {
            red.fetch_add(1, Ordering::SeqCst);
            return MockReply::Body(answer(&["R1S0", "R3S1"]));
        }
        panic!("Einzeldurchlauf trotz Ueberlaenge");
    })
    .await;
    // Passt nicht in einen Aufruf, aber jede Aufnahme einzeln.
    let doc = run_with(
        &fx,
        &settings_with_mock_provider(port),
        &fx.ids,
        None,
        ProjectKind::Minutes,
        limits(2_600),
    )
    .await
    .unwrap();

    assert!(!doc.meta.single_pass);
    assert!(
        doc.meta.chunks_total >= 3,
        "mindestens ein Block je Aufnahme: {}",
        doc.meta.chunks_total
    );
    assert_eq!(reduces.load(Ordering::SeqCst), 1, "ein Zusammenfuehren");
    let maps = maps.lock().unwrap().clone();
    assert_eq!(maps.len() as u32, doc.meta.chunks_total);
    // Blockgrenzen: kein Block mischt zwei Aufnahmen.
    for request in &maps {
        let chunk = request.split("# Transcript (part").nth(1).unwrap();
        let headers = chunk.matches("=== R").count();
        assert!(headers <= 1, "ein Block mit {headers} Kopfzeilen: {chunk}");
    }
    // Jeder Block nennt IDs seiner eigenen Aufnahme; alle drei Aufnahmen kommen dran.
    for n in 1..=3 {
        assert!(
            maps.iter().any(|m| m.contains(&format!("R{n}S0 "))),
            "Aufnahme {n} wurde nie gefragt"
        );
    }
    // Das Ergebnis ist geprueft und belegt.
    let summary = &doc.sections[0].entries[0];
    assert_eq!(summary.sources.len(), 2);
    assert!(!doc.body.contains("hinweis"));
    assert_eq!(
        fx.store.project_minutes_list(&fx.folder_id).unwrap().len(),
        1
    );
}

#[tokio::test]
async fn a_block_that_cannot_be_evaluated_becomes_a_named_gap_not_a_silent_loss() {
    let fx = long_three();
    let port = spawn_llm_mock_with(move |req| {
        if is_map(req) {
            let chunk = req.split("# Transcript (part").nth(1).unwrap_or("");
            // Alles aus Aufnahme 2 scheitert, der Rest klappt.
            if chunk.contains("R2S") {
                return MockReply::Status(500);
            }
            let id = first_id(chunk).unwrap_or_else(|| "R1S0".into());
            return MockReply::Body(chat_body(
                &json!({"entries": [{"section": "besprochene_punkte", "text": format!("Punkt {id}"), "assignee": null, "due": null, "sources": [id]}]})
                    .to_string(),
            ));
        }
        // Zusammenfuehren: scheitert auch, die Eintraege der Bloecke bleiben.
        MockReply::Status(500)
    })
    .await;
    let doc = run_with(
        &fx,
        &settings_with_mock_provider(port),
        &fx.ids,
        None,
        ProjectKind::Minutes,
        limits(2_600),
    )
    .await
    .unwrap();
    assert!(doc.meta.incomplete);
    assert!(
        doc.meta.gaps.iter().any(|g| g.starts_with("Aufnahme 2, ")),
        "{:?}",
        doc.meta.gaps
    );
    assert!(
        doc.body
            .contains("> **Hinweis:** Das Protokoll ist unvollständig."),
        "{}",
        doc.body
    );
    // Die ausgewerteten Teile sind da, belegt.
    let points = doc
        .sections
        .iter()
        .find(|s| s.id == "besprochene_punkte")
        .unwrap();
    assert!(points.entries.len() >= 2);
    assert!(points.entries.iter().all(|e| !e.sources.is_empty()));
}

#[tokio::test]
async fn when_no_block_works_the_run_fails_and_stores_nothing() {
    let fx = long_three();
    let port = spawn_llm_mock_with(|_| MockReply::Status(500)).await;
    let err = run_with(
        &fx,
        &settings_with_mock_provider(port),
        &fx.ids,
        None,
        ProjectKind::Minutes,
        limits(2_600),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "llm_failed");
    assert_eq!(project_rows(&fx), 0);
}

// -- Auswahl wird abgewiesen, bevor ein Modell gefragt wird --------------------------------------------

#[tokio::test]
async fn a_recording_without_a_transcript_rejects_the_whole_run_before_any_model_call() {
    let fx = fixture(&[
        ("Mit Text", 1_790_000_000, 3, "Inhalt."),
        ("Ohne Text", 1_790_100_000, 0, "x"),
    ]);
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let port = spawn_llm_mock_with(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        MockReply::Body(answer(&["R1S0"]))
    })
    .await;
    let err = run(&fx, &settings_with_mock_provider(port))
        .await
        .unwrap_err();
    assert_eq!(err.code, "no_transcript");
    assert_eq!(
        err.detail, fx.ids[1],
        "die abgelehnte Aufnahme wird genannt"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "kein einziger Modellaufruf"
    );
    assert_eq!(project_rows(&fx), 0);
    assert!(!run_state(&fx.folder_id).running, "keine Sperre uebrig");
    // Ohne die abgelehnte Aufnahme geht es.
    let ok = run_with(
        &fx,
        &settings_with_mock_provider(port),
        &fx.ids[..1],
        None,
        ProjectKind::Minutes,
        limits(1_000_000),
    )
    .await
    .unwrap();
    assert_eq!(ok.recordings.len(), 1);
}

#[tokio::test]
async fn invalid_selections_are_refused_with_their_own_codes() {
    let fx = three();
    let port = spawn_llm_mock_with(|_| MockReply::Body(answer(&["R1S0"]))).await;
    let settings = settings_with_mock_provider(port);
    let code = |ids: Vec<String>| {
        let (fx, settings) = (&fx, &settings);
        async move {
            run_with(
                fx,
                settings,
                &ids,
                None,
                ProjectKind::Minutes,
                limits(1_000_000),
            )
            .await
            .unwrap_err()
        }
    };

    assert_eq!(code(vec![]).await.code, "no_selection");
    assert_eq!(code(vec!["  ".into()]).await.code, "no_selection");
    assert_eq!(
        code(vec!["gibt-es-nicht".into()]).await.code,
        "meeting_not_found"
    );
    let many: Vec<String> = (0..=corpus::MAX_RECORDINGS)
        .map(|i| format!("m{i}"))
        .collect();
    assert_eq!(code(many).await.code, "too_many_recordings");

    // Eine Besprechung, die nicht in diesem Projekt liegt.
    let outside = ready_meeting(&fx.store, "Woanders", 1_790_300_000);
    fx.store
        .append_delta(
            &outside.id,
            &TranscriptDelta {
                new_segments: vec![segment(0, "Text.")],
            },
        )
        .unwrap();
    let err = code(vec![fx.ids[0].clone(), outside.id.clone()]).await;
    assert_eq!(
        (err.code, err.detail.as_str()),
        ("not_in_project", outside.id.as_str())
    );

    // Noch in Verarbeitung.
    fx.store
        .set_status(&fx.ids[2], MeetingStatus::Processing)
        .unwrap();
    assert_eq!(code(fx.ids.clone()).await.code, "meeting_not_finished");
    fx.store
        .set_status(&fx.ids[2], MeetingStatus::Ready)
        .unwrap();

    // Geloescht.
    fx.store.soft_delete_meeting(&fx.ids[1]).unwrap();
    assert_eq!(code(fx.ids.clone()).await.code, "meeting_not_found");
    assert_eq!(project_rows(&fx), 0, "keine der Abweisungen schrieb etwas");

    // Unbekanntes Projekt.
    let ghost = generate_guarded(
        &settings,
        fx.store.clone(),
        &Request {
            folder_id: "gibt-es-nicht",
            meeting_ids: &fx.ids[..1],
            template_id: None,
            kind: ProjectKind::Minutes,
        },
        limits(1_000_000),
        &|_| {},
    )
    .await
    .unwrap_err();
    assert_eq!(ghost.code, "folder_not_found");
}

#[test]
fn the_candidates_name_the_reason_a_recording_cannot_be_chosen() {
    let fx = fixture(&[
        ("Mit Text", 1_790_000_000, 3, "Inhalt."),
        ("Leer", 1_790_100_000, 0, "x"),
        ("Laeuft", 1_790_200_000, 2, "Inhalt."),
    ]);
    fx.store
        .set_status(&fx.ids[2], MeetingStatus::Processing)
        .unwrap();
    let empty = fx
        .store
        .create_empty_meeting("Neue Besprechung", Some(&fx.folder_id))
        .unwrap();

    let list = candidates(&fx.store, &fx.folder_id).unwrap();
    let by = |id: &str| list.iter().find(|c| c.meeting_id == id).unwrap().clone();
    assert!(by(&fx.ids[0]).eligible && by(&fx.ids[0]).reason.is_none());
    assert_eq!(by(&fx.ids[0]).segments, 3);
    assert_eq!(by(&fx.ids[1]).reason.as_deref(), Some("no_transcript"));
    assert_eq!(
        by(&fx.ids[2]).reason.as_deref(),
        Some("meeting_not_finished")
    );
    assert_eq!(by(&empty.id).reason.as_deref(), Some("empty_entry"));
    assert!(!by(&empty.id).eligible);
    assert_eq!(
        candidates(&fx.store, "gibt-es-nicht").unwrap_err().code,
        "folder_not_found"
    );
}

// -- Sperre, Stopp, Zeitlimit, Speicher, Absturz -----------------------------------------------------------

#[tokio::test]
async fn a_second_start_for_the_same_project_is_refused_and_the_lock_is_freed() {
    let fx = three();
    // Der erste Lauf haengt; der zweite startet, sobald die Anfrage des ersten
    // beim Server angekommen ist (kein Warten auf die Uhr: unter Last kaeme ein
    // fester Abstand zu spaet).
    let arrived = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = arrived.clone();
    let hang = spawn_llm_mock_with(move |_| {
        flag.store(true, Ordering::SeqCst);
        MockReply::Hang
    })
    .await;
    let settings = settings_with_mock_provider(hang);
    let refused = {
        let first = run_with(
            &fx,
            &settings,
            &fx.ids,
            None,
            ProjectKind::Minutes,
            limits(1_000_000),
        );
        tokio::pin!(first);
        let second = async {
            for _ in 0..2_000 {
                if arrived.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(arrived.load(Ordering::SeqCst), "die Anfrage kam nie an");
            assert!(
                run_state(&fx.folder_id).running,
                "der erste Lauf haelt die Sperre"
            );
            run_with(
                &fx,
                &settings,
                &fx.ids,
                None,
                ProjectKind::Minutes,
                limits(1_000_000),
            )
            .await
        };
        tokio::select! {
            ended = &mut first => panic!("der erste Lauf endete von selbst: {ended:?}"),
            refused = second => refused,
        }
        // Hier wird der haengende erste Lauf verworfen (Fenster zu, Task abgebrochen).
    };
    assert_eq!(refused.unwrap_err().code, "minutes_busy");
    // Danach ist die Sperre frei; ein anderes Projekt war nie betroffen.
    assert!(!run_state(&fx.folder_id).running);
    assert_eq!(project_rows(&fx), 0);
    assert!(MinutesRunGuard::acquire(&run_key("anderes-projekt")).is_ok());
    let ok_port = spawn_llm_mock_with(|_| MockReply::Body(answer(&["R1S0"]))).await;
    assert!(run(&fx, &settings_with_mock_provider(ok_port))
        .await
        .is_ok());
}

#[tokio::test]
async fn a_stop_ends_the_run_without_writing() {
    let fx = three();
    let folder = fx.folder_id.clone();
    let port = spawn_llm_mock_with(move |_| {
        // Der Nutzer stoppt, waehrend das Modell antwortet.
        assert!(request_cancel(&folder));
        MockReply::Body(answer(&["R1S0"]))
    })
    .await;
    let err = run(&fx, &settings_with_mock_provider(port))
        .await
        .unwrap_err();
    assert_eq!(err.code, CODE_CANCELLED);
    assert_eq!(project_rows(&fx), 0, "ein gestoppter Lauf schreibt nichts");
    assert!(!run_state(&fx.folder_id).running);
    // Und ohne Stopp danach laeuft es wieder.
    let ok = spawn_llm_mock_with(|_| MockReply::Body(answer(&["R1S0"]))).await;
    assert!(run(&fx, &settings_with_mock_provider(ok)).await.is_ok());
}

#[tokio::test]
async fn a_stop_before_the_first_call_never_asks_the_model() {
    let fx = three();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let port = spawn_llm_mock_with(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        MockReply::Body(answer(&["R1S0"]))
    })
    .await;
    let settings = settings_with_mock_provider(port);
    let request = Request {
        folder_id: &fx.folder_id,
        meeting_ids: &fx.ids,
        template_id: Some("auto"),
        kind: ProjectKind::Minutes,
    };
    // Stopp, sobald die erste Fortschrittsmeldung (Vorlagenwahl) kommt.
    let folder = fx.folder_id.clone();
    let err = generate_guarded(
        &settings,
        fx.store.clone(),
        &request,
        limits(1_000_000),
        &move |_| {
            request_cancel(&folder);
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, CODE_CANCELLED);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(project_rows(&fx), 0);
}

#[tokio::test]
async fn a_hanging_server_ends_in_the_time_limit_and_frees_the_lock() {
    let fx = three();
    let port = spawn_llm_mock_with(|_| MockReply::Hang).await;
    let mut short = limits(1_000_000);
    short.timeout = Duration::from_millis(300);
    let err = run_with(
        &fx,
        &settings_with_mock_provider(port),
        &fx.ids,
        None,
        ProjectKind::Minutes,
        short,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "llm_failed");
    assert!(err.detail.contains("Zeitlimit"));
    assert_eq!(project_rows(&fx), 0);
    assert!(!run_state(&fx.folder_id).running);
}

#[tokio::test]
async fn a_dropped_run_frees_the_lock() {
    let fx = three();
    let port = spawn_llm_mock_with(|_| MockReply::Hang).await;
    let settings = settings_with_mock_provider(port);
    {
        let fut = run(&fx, &settings);
        // Das Future wird nach kurzer Zeit verworfen (Fenster zu, Task abgebrochen).
        let _ = tokio::time::timeout(Duration::from_millis(150), fut).await;
    }
    assert!(!run_state(&fx.folder_id).running);
    assert_eq!(project_rows(&fx), 0);
}

#[tokio::test]
async fn a_project_deleted_during_the_run_stores_nothing() {
    let fx = three();
    let (store, folder) = (fx.store.clone(), fx.folder_id.clone());
    let port = spawn_llm_mock_with(move |_| {
        store.folder_delete(&folder).unwrap();
        MockReply::Body(answer(&["R1S0"]))
    })
    .await;
    let err = run(&fx, &settings_with_mock_provider(port))
        .await
        .unwrap_err();
    assert_eq!(err.code, "folder_not_found");
    assert_eq!(project_rows(&fx), 0, "weder Zeile noch Rest");
    assert!(!run_state(&fx.folder_id).running);
}

#[tokio::test]
async fn a_recording_deleted_during_the_run_does_not_change_the_result() {
    let fx = three();
    let (store, victim) = (fx.store.clone(), fx.ids[0].clone());
    let port = spawn_llm_mock_with(move |_| {
        store.soft_delete_meeting(&victim).unwrap();
        MockReply::Body(answer(&["R3S0"]))
    })
    .await;
    // Die Aufnahmen sind zu Beginn gelesen: das Protokoll nennt sie weiter.
    let doc = run(&fx, &settings_with_mock_provider(port)).await.unwrap();
    assert_eq!(doc.recordings.len(), 3);
    assert_eq!(doc.sections[0].entries[0].sources[0].meeting_id, fx.ids[0]);
}

#[tokio::test]
async fn a_failing_server_ends_the_run_and_stores_nothing() {
    let fx = three();
    let port = spawn_llm_mock_with(|_| MockReply::Status(500)).await;
    let err = run(&fx, &settings_with_mock_provider(port))
        .await
        .unwrap_err();
    assert_eq!(err.code, "llm_failed");
    assert_eq!(project_rows(&fx), 0);
    assert!(!run_state(&fx.folder_id).running);
}

#[tokio::test]
async fn low_memory_ends_the_run_as_memory_low_without_another_attempt() {
    let fx = three();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let port = spawn_llm_mock_with(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        MockReply::Status(500)
    })
    .await;
    let mut low = limits(1_000_000);
    low.free_mb = low_ram;
    let err = run_with(
        &fx,
        &settings_with_mock_provider(port),
        &fx.ids,
        None,
        ProjectKind::Minutes,
        low,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "memory_low");
    assert_eq!(project_rows(&fx), 0);

    // Im Blocklauf: abbrechen, nicht den naechsten Block starten.
    let fx = long_three();
    let before = calls.load(Ordering::SeqCst);
    let mut low = limits(2_600);
    low.free_mb = low_ram;
    let port = spawn_llm_mock_with({
        let counter = calls.clone();
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            MockReply::Status(500)
        }
    })
    .await;
    let err = run_with(
        &fx,
        &settings_with_mock_provider(port),
        &fx.ids,
        None,
        ProjectKind::Minutes,
        low,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "memory_low");
    assert_eq!(
        calls.load(Ordering::SeqCst) - before,
        1,
        "genau ein Aufruf, dann Schluss"
    );
}

#[tokio::test]
async fn progress_reports_template_then_write_then_merge_and_survives_in_the_run_state() {
    let fx = long_three();
    let seen: Arc<Mutex<Vec<MinutesProgress>>> = Arc::default();
    let log = seen.clone();
    let folder = fx.folder_id.clone();
    let states: Arc<Mutex<Vec<bool>>> = Arc::default();
    let st = states.clone();
    let port = spawn_llm_mock_with(move |req| {
        // Waehrend des Laufs fragt die Oberflaeche den Zustand ab.
        st.lock().unwrap().push(run_state(&folder).running);
        if is_map(req) {
            let chunk = req.split("# Transcript (part").nth(1).unwrap_or("");
            let id = first_id(chunk).unwrap_or_else(|| "R1S0".into());
            return MockReply::Body(chat_body(
                &json!({"entries": [{"section": "besprochene_punkte", "text": format!("Punkt {id}"), "assignee": null, "due": null, "sources": [id]}]})
                    .to_string(),
            ));
        }
        MockReply::Body(answer(&["R1S0"]))
    })
    .await;
    let request = Request {
        folder_id: &fx.folder_id,
        meeting_ids: &fx.ids,
        template_id: None,
        kind: ProjectKind::Minutes,
    };
    generate_guarded(
        &settings_with_mock_provider(port),
        fx.store.clone(),
        &request,
        limits(2_600),
        &move |p| log.lock().unwrap().push(p.clone()),
    )
    .await
    .unwrap();
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.first().unwrap().phase, MinutesPhase::Template);
    assert_eq!(seen.first().unwrap().total, 0, "Vorlagenwahl: unbestimmt");
    let last = seen.last().unwrap();
    assert_eq!((last.phase, last.done), (MinutesPhase::Merge, last.total));
    let writes: Vec<u32> = seen
        .iter()
        .filter(|p| p.phase == MinutesPhase::Write)
        .map(|p| p.done)
        .collect();
    assert!(
        writes.windows(2).all(|w| w[0] <= w[1]),
        "nie rueckwaerts: {writes:?}"
    );
    assert!(
        states.lock().unwrap().iter().all(|running| *running),
        "der Zustand ist abfragbar"
    );
    assert!(!run_state(&fx.folder_id).running, "danach nicht mehr");
}

// -- Bausteine -------------------------------------------------------------------------------------------

#[test]
fn error_codes_of_both_modules_survive_the_round_trip_through_text() {
    for code in EXTRA_CODES {
        assert_eq!(error_code(code), code);
        assert_eq!(error_code(&format!("{code}: detail")), code);
    }
    assert_eq!(error_code("minutes_busy"), "minutes_busy");
    assert_eq!(error_code("no_transcript: 01ABC"), "no_transcript");
    assert_eq!(error_code("irgendwas Neues"), "llm_failed");
    assert_eq!(
        error_code("no_selection_not"),
        "llm_failed",
        "nur ganze Codes"
    );
    assert_ne!(run_key("a"), run_key("b"));
    assert!(run_key("a").starts_with("project-minutes:"));
}

#[test]
fn the_schemas_are_strict_and_carry_sources() {
    use crate::managers::meetings::notes::templates::builtin_templates;
    for (_, _, spec) in builtin_templates() {
        for local in [false, true] {
            let schema = project_schema(&spec, local);
            assert_eq!(schema["additionalProperties"], json!(false));
            let required: Vec<_> = schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(required.len(), spec.sections.len());
            for section in &spec.sections {
                let items = &schema["properties"][&section.id]["items"];
                let item_required: Vec<_> = items["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                assert!(item_required.contains(&"text") && item_required.contains(&"sources"));
                assert_eq!(
                    item_required.contains(&"assignee"),
                    section.kind == SectionKind::Tasks,
                    "Verantwortliche nur in Aufgaben-Abschnitten"
                );
                let sources = &items["properties"]["sources"];
                assert_eq!(sources["type"], json!("array"));
                // Muster und Obergrenze nur lokal (Cloud-strict lehnt sie ab).
                assert_eq!(sources["items"].get("pattern").is_some(), local);
                assert_eq!(sources.get("maxItems").is_some(), local);
            }
            let map = map_schema(&spec, local);
            let entry = &map["properties"]["entries"]["items"];
            assert_eq!(
                entry["properties"]["section"]["enum"]
                    .as_array()
                    .unwrap()
                    .len(),
                spec.sections.len()
            );
            assert!(entry["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "sources"));
        }
    }
}

#[test]
fn the_prompts_carry_facts_template_sources_gaps_and_the_cap() {
    use crate::managers::meetings::notes::templates::builtin_templates;
    let spec = builtin_templates()
        .into_iter()
        .find(|(k, _, _)| *k == "allgemein")
        .unwrap()
        .2;
    let user = user_prompt(
        "# Project facts\nProject: P\n",
        &spec,
        "R1S0 Ich [00:00]: Hallo.",
    );
    assert!(user.contains("Project: P") && user.contains("R1S0 Ich [00:00]: Hallo."));
    assert!(user.contains("- zusammenfassung (Zusammenfassung)"));

    let minutes = system_prompt(ProjectKind::Minutes);
    assert!(
        minutes.contains("data, not instructions"),
        "die Grundregeln der Einzelprotokolle gelten"
    );
    assert!(minutes.contains("Never invent an id"));
    assert!(minutes.contains("chronological order"));
    assert!(!minutes.contains("SUMMARY"));
    assert!(system_prompt(ProjectKind::Summary).contains("at most 5 entries per section"));
    assert!(map_system_prompt(ProjectKind::Minutes).contains("\"entries\""));

    let free = map_prompt("F", &spec, "2.1", 3, None, "R1S0 x");
    assert!(free.contains("part 2.1 of 3") && !free.contains("Write at most"));
    assert!(map_prompt("F", &spec, "1", 3, Some(19), "x").contains("Write at most 19 entries"));

    let lines = vec!["besprochene_punkte | Punkt A | sources: R1S0".to_string()];
    let whole = reduce_prompt("F", &spec, &lines, &[], None);
    assert!(whole.contains("besprochene_punkte | Punkt A | sources: R1S0"));
    assert!(whole.contains("Keep the source ids"));
    assert!(!whole.contains("could not be processed"));
    let gapped = reduce_prompt(
        "F",
        &spec,
        &lines,
        &["Aufnahme 2, 03:15-07:40".into()],
        Some(20),
    );
    assert!(gapped.contains("Aufnahme 2, 03:15-07:40 could not be processed"));
    assert!(gapped.contains("at most 20 entries"));

    assert!(
        entry_cap_for(ProjectKind::Summary, 30_000) < entry_cap_for(ProjectKind::Minutes, 30_000)
    );
    assert!(entry_cap_for(ProjectKind::Summary, 10) >= 4);
}
