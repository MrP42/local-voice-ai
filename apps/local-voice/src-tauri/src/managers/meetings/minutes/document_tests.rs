//! Dokumenttypen im Protokoll-Motor (Entwurf 2026-10-13, Etappe 1). Mock-Server statt
//! Modell: geprueft wird, was das Modell zu sehen bekommt (Regeln des Typs, Eintragsgrenze,
//! keine Vorlagenwahl) und was gespeichert wird (`kind`, Version, Metadaten), dazu Stopp
//! und die geteilte Sperre je Besprechung.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::managers::meetings::doc_types::{by_key, DocType};
use crate::managers::meetings::llm_call::test_support::{
    chat_body, settings_with_mock_provider, spawn_llm_mock_with, MockReply,
};
use crate::managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};

struct Fx {
    store: Arc<MeetingStore>,
    meeting: String,
}

fn fixture(n: u32) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
    std::mem::forget(dir);
    let store = Arc::new(store);
    let meeting = store
        .create_meeting("Lenkungskreis", MeetingSource::Import, Some(1_755_600_000))
        .unwrap()
        .id;
    let segments: Vec<StoredSegment> = (0..n)
        .map(|i| StoredSegment {
            segment_index: i,
            text: format!(
                "Zeile {i:02}: Go-Live am 1. September beschlossen, Budget bleibt. {}",
                "Weitere Einzelheiten zu Risiken und Terminen. ".repeat(3)
            ),
            start_ms: u64::from(i) * 60_000,
            end_ms: u64::from(i) * 60_000 + 50_000,
            channel: (i % 2) as u8,
            speaker_index: None,
            words: None,
        })
        .collect();
    store
        .append_delta(
            &meeting,
            &TranscriptDelta {
                new_segments: segments,
            },
        )
        .unwrap();
    store.set_status(&meeting, MeetingStatus::Ready).unwrap();
    Fx { store, meeting }
}

fn limits(budget_chars: usize) -> Limits {
    Limits {
        budget_chars: Some(budget_chars),
        timeout: Duration::from_secs(120),
        classify_timeout: Duration::from_secs(30),
        free_mb: || 0,
    }
}

fn summary() -> DocType {
    by_key("management_summary").unwrap()
}

fn summary_answer() -> String {
    json!({
        "anlass_ziel": ["Entscheidung über den Go-Live."],
        "kernergebnisse": ["Go-Live am 1. September", "Budget bleibt unverändert"],
        "entscheidungen": ["Go-Live am 1. September"],
        "risiken_handlungsbedarf": [],
        "naechste_schritte": [{ "text": "Release-Notes schreiben", "assignee": "Anna", "due": "Freitag" }]
    })
    .to_string()
}

async fn run(
    fx: &Fx,
    settings: &AppSettings,
    budget: usize,
) -> Result<MeetingDocument, MinutesError> {
    generate_document_guarded(
        settings,
        fx.store.clone(),
        &fx.meeting,
        summary(),
        &DocBasis::default(),
        limits(budget),
        &|_| {},
    )
    .await
}

#[tokio::test]
async fn a_management_summary_is_one_call_with_its_rules_and_is_stored_under_its_own_kind() {
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = bodies.clone();
    let port = spawn_llm_mock_with(move |body| {
        seen.lock().unwrap().push(body.to_string());
        MockReply::Body(chat_body(&summary_answer()))
    })
    .await;
    let settings = settings_with_mock_provider(port);
    let fx = fixture(3);

    let doc = run(&fx, &settings, 1_000_000).await.unwrap();

    // Ein Aufruf: keine Vorlagenwahl, kein Block, kein Zusammenfuehren.
    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1, "genau eine Anfrage");
    assert!(bodies[0].contains("MANAGEMENT SUMMARY"));
    assert!(bodies[0].contains("Write at most 18 entries in total"));
    assert!(bodies[0].contains("kernergebnisse"));
    assert!(!bodies[0].contains("Templates (choose one id)"));

    assert_eq!(doc.kind, "doc:management_summary");
    assert_eq!(
        doc.template_id.as_deref(),
        Some("doctype:management_summary")
    );
    assert_eq!(doc.version, 1);
    assert!(
        doc.body.starts_with("# Management Summary: Lenkungskreis"),
        "{}",
        doc.body
    );
    assert!(doc
        .body
        .contains("- Release-Notes schreiben _(Wer: Anna, Bis: Freitag)_"));
    // Leerer Abschnitt faellt weg.
    assert!(!doc.body.contains("Risiken und Handlungsbedarf"));

    let meta: Value = serde_json::from_str(
        &fx.store
            .document_generation_metadata(&doc.id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(meta["doc_type"], json!("management_summary"));
    assert_eq!(meta["over_length"], json!(false));
    assert!(meta["words"].as_u64().unwrap() > 5);
    assert_eq!(meta["single_pass"], json!(true));

    // Zweiter Lauf: neue Version desselben Typs; das Protokoll bleibt unberuehrt.
    let again = run(&fx, &settings, 1_000_000).await.unwrap();
    assert_eq!(again.version, 2);
    let docs = fx.store.get_documents(&fx.meeting).unwrap();
    assert!(docs.iter().all(|d| d.kind == "doc:management_summary"));
    assert!(latest_meta(&fx.store, &fx.meeting).is_none());
    let meta = latest_meta_of_kind(&fx.store, &fx.meeting, "doc:management_summary").unwrap();
    assert_eq!(meta.document_id, again.id);
    assert_eq!(meta.template_title.as_deref(), Some("Management Summary"));
}

/// Zahl hinter "at most " in einem Prompt (map oder Zusammenfuehren).
fn caps_in(body: &str) -> Vec<usize> {
    body.match_indices("at most ")
        .filter_map(|(i, _)| {
            body[i + 8..]
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|n| n.parse().ok())
        })
        .collect()
}

#[tokio::test]
async fn long_transcripts_run_in_blocks_and_every_part_carries_the_entry_cap_of_the_type() {
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = bodies.clone();
    let port = spawn_llm_mock_with(move |body| {
        seen.lock().unwrap().push(body.to_string());
        if body.contains("This is part ") {
            MockReply::Body(chat_body(
                &json!({ "entries": [{
                    "section": "kernergebnisse", "text": "Go-Live am 1. September",
                    "assignee": null, "due": null
                }] })
                .to_string(),
            ))
        } else {
            MockReply::Body(chat_body(&summary_answer()))
        }
    })
    .await;
    let settings = settings_with_mock_provider(port);
    let fx = fixture(12);

    let doc = run(&fx, &settings, 2_500).await.unwrap();

    let bodies = bodies.lock().unwrap().clone();
    let maps: Vec<&String> = bodies
        .iter()
        .filter(|b| b.contains("This is part "))
        .collect();
    assert!(maps.len() >= 2, "Bloecke erwartet, waren {}", maps.len());
    for body in &maps {
        let caps = caps_in(body);
        assert!(!caps.is_empty(), "map ohne Eintragsgrenze");
        assert!(caps.iter().all(|n| *n <= 18), "{caps:?}");
        assert!(body.contains("MANAGEMENT SUMMARY"));
    }
    assert_eq!(doc.kind, "doc:management_summary");
}

#[tokio::test]
async fn a_stop_during_the_call_writes_nothing() {
    let fx = fixture(3);
    let meeting = fx.meeting.clone();
    let port = spawn_llm_mock_with(move |_| {
        assert!(request_cancel(&meeting));
        MockReply::Body(chat_body(&summary_answer()))
    })
    .await;
    let settings = settings_with_mock_provider(port);

    let err = run(&fx, &settings, 1_000_000).await.unwrap_err();

    assert_eq!(err.code, CODE_CANCELLED);
    assert!(fx.store.get_documents(&fx.meeting).unwrap().is_empty());
    assert!(!run_state(&fx.meeting).running, "Sperre frei");
}

#[tokio::test]
async fn a_document_and_the_minutes_share_one_lock_per_meeting() {
    let fx = fixture(2);
    let settings = settings_with_mock_provider(1);
    let _held = MinutesRunGuard::acquire(&fx.meeting).unwrap();

    let err = run(&fx, &settings, 1_000_000).await.unwrap_err();

    assert_eq!(err.code, CODE_BUSY);
    assert!(fx.store.get_documents(&fx.meeting).unwrap().is_empty());
}

/// Probe gegen eine echte Besprechung, nur von Hand (`--ignored`): liest eine KOPIE der
/// Datenbank (`LVA_PROBE_DB`), die Einstellungen als Datei (`LVA_PROBE_SETTINGS`, Inhalt
/// von `settings_store.json`; nie `settings::get_settings`), erzeugt den Typ
/// `LVA_PROBE_TYPE` (Vorgabe `management_summary`) fuer `LVA_PROBE_MEETING` und schreibt
/// das Markdown nach `LVA_PROBE_OUT`. Optional `LVA_PROBE_PROVIDER` / `LVA_PROBE_MODEL`.
#[tokio::test]
#[ignore = "Probe gegen echte Daten und ein echtes Modell, nur von Hand"]
async fn probe_real_meeting() {
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} fehlt"));
    let raw: Value =
        serde_json::from_str(&std::fs::read_to_string(var("LVA_PROBE_SETTINGS")).unwrap()).unwrap();
    let mut settings: AppSettings =
        serde_json::from_value(raw.get("settings").cloned().unwrap_or(raw)).unwrap();
    if let Ok(provider) = std::env::var("LVA_PROBE_PROVIDER") {
        settings.post_process_provider_id = provider;
    }
    if let Ok(model) = std::env::var("LVA_PROBE_MODEL") {
        let id = settings.post_process_provider_id.clone();
        settings.post_process_models.insert(id, model);
    }
    let store =
        Arc::new(MeetingStore::open_at(std::path::Path::new(&var("LVA_PROBE_DB"))).unwrap());
    // `LVA_PROBE_TYPE=all`: jeder erzeugbare Typ des Katalogs.
    let key = std::env::var("LVA_PROBE_TYPE").unwrap_or_else(|_| "management_summary".into());
    let keys: Vec<String> = if key == "all" {
        crate::managers::meetings::doc_types::catalog()
            .into_iter()
            .filter(|t| t.engine == crate::managers::meetings::doc_types::DocEngine::Document)
            .map(|t| t.key.to_string())
            .collect()
    } else {
        vec![key]
    };
    // Ausgabe je Typ: `<dir>/<key>.md`, `.meta.json`, `.docx`, `.pdf` (Briefkopf
    // `LVA_PROBE_LETTERHEAD`, Vorgabe `wai`).
    let out_dir = std::path::PathBuf::from(var("LVA_PROBE_OUT_DIR"));
    std::fs::create_dir_all(&out_dir).unwrap();
    let lh_store = crate::managers::letterhead::Store::new(out_dir.join("briefkoepfe"));
    let lh = lh_store
        .get(&std::env::var("LVA_PROBE_LETTERHEAD").unwrap_or_else(|_| "wai".into()))
        .unwrap();
    let logo = lh_store.logo(&lh).unwrap();
    let design = crate::managers::meetings::doc_export::Design {
        letterhead: &lh,
        logo: logo.as_ref(),
    };
    for key in keys {
        let started = std::time::Instant::now();
        let doc = generate_document_guarded(
            &settings,
            store.clone(),
            &var("LVA_PROBE_MEETING"),
            by_key(&key).unwrap(),
            &DocBasis::default(),
            Limits::default(),
            &|p| eprintln!("{key}: {:?} {}/{}", p.phase, p.done, p.total),
        )
        .await
        .unwrap_or_else(|e| panic!("Probe {key} fehlgeschlagen: {e}"));
        let meta = store
            .document_generation_metadata(&doc.id)
            .unwrap()
            .unwrap_or_default();
        std::fs::write(out_dir.join(format!("{key}.md")), &doc.body).unwrap();
        std::fs::write(out_dir.join(format!("{key}.meta.json")), meta).unwrap();
        for ext in ["docx", "pdf"] {
            crate::managers::meetings::doc_export::write(
                &out_dir.join(format!("{key}.{ext}")),
                &doc.body,
                &design,
            )
            .unwrap();
        }
        eprintln!(
            "{key}: fertig in {:?}, {} Zeichen",
            started.elapsed(),
            doc.body.len()
        );
    }
}

/// Klaerung 3 (Etappe 2): das Protokoll rendert Eintraege mit derselben Funktion wie die
/// Dokumenttypen -- kein "Wer: null", keine zerrissene Liste.
#[test]
fn the_minutes_renderer_drops_placeholder_names_and_keeps_entries_on_one_line() {
    let head = MinutesHead {
        title: "Jour fixe".into(),
        description: String::new(),
        date_iso: "2026-10-13".into(),
        duration_ms: 600_000,
        shares: vec![],
        single_speaker: true,
        mixed_channel: false,
    };
    let entry = |text: &str, who: &str| MinutesEntry {
        text: text.into(),
        assignee: Some(who.into()),
        due: Some("null".into()),
    };
    let sections = vec![
        MinutesSection {
            id: "zusammenfassung".into(),
            title: "Zusammenfassung".into(),
            kind: SectionKind::Text,
            entries: vec![MinutesEntry {
                text: "Erste Zeile\nzweite Zeile".into(),
                assignee: None,
                due: None,
            }],
        },
        MinutesSection {
            id: "aufgaben".into(),
            title: "Aufgaben".into(),
            kind: SectionKind::Tasks,
            entries: vec![entry("Angebot\nschicken", "null"), entry("Notiz", "Anna")],
        },
    ];
    let md = minutes_to_markdown(&head, "Allgemein", None, &sections, &[]);
    assert!(
        md.contains("## Zusammenfassung\n\nErste Zeile zweite Zeile\n"),
        "{md}"
    );
    assert!(md.contains("- Angebot schicken\n"), "{md}");
    assert!(md.contains("- Notiz _(Wer: Anna)_\n"), "{md}");
    assert!(!md.contains("null"), "{md}");
}
