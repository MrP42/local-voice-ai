//! lva#396 N4a / lva#430 Review: Notizbuch-Text geht nur an lokale Modelle, auch aus
//! gespeicherten KI-Notizen (Anweisung). Synthetisches Notizbuch (Migration wie in
//! `notizbuch::verdrahtung_tests`), Mock-Server statt Modell: geprueft wird, WAS das Modell sieht.
//! Nur synthetische Daten (`example.*`).

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::*;
use crate::managers::meetings::llm_call::test_support::{
    chat_body, settings_with_mock_provider, spawn_llm_mock_with, MockReply,
};
use crate::managers::meetings::notizbuch::links::LinkArt;
use crate::managers::meetings::notizbuch::quelle::{fuer_besprechung_mit, NotizbuchNotizen};
use crate::managers::meetings::notizbuch::schutz;
use crate::managers::meetings::notizbuch::verdrahtung_tests::{
    ablage_mit_seiten, besprechung, nur_arbeit, tmp_store, Ablage,
};

/// Texte aus dem synthetischen Notizbuch (Seite s1), die nie an ein entferntes Modell duerfen.
const NOTIZBUCH_TEXTE: [&str; 4] = [
    "Budget Migration Frankfurt",
    "Rollout Server Dezember abschalten",
    "kontakt@example.org",
    "Angebot prüfen",
];

struct Fx {
    _ablage: Ablage,
    _dir: tempfile::TempDir,
    store: Arc<MeetingStore>,
    meeting: String,
    pfad: &'static std::path::Path,
}

/// Fertige Besprechung mit Transkript, Notizblock-Notiz und bestaetigt zugeordneter Seite s1.
fn fixture() -> Fx {
    let ablage = ablage_mit_seiten();
    let (dir, store) = tmp_store();
    let meeting = besprechung(&store);
    store
        .save_notes(
            &meeting,
            &[NoteBlock {
                id: "N-eigen".into(),
                kind: NoteBlockKind::Bullet,
                text: "Eigene Notiz zur Besprechung".into(),
                at_ms: None,
                checked: false,
            }],
            0,
        )
        .unwrap();
    store
        .note_link_setzen(&meeting, "s1", LinkArt::Bestaetigt, "")
        .unwrap();
    let pfad: &'static std::path::Path = Box::leak(ablage.pfad.clone().into_boxed_path());
    Fx {
        _ablage: ablage,
        _dir: dir,
        store: Arc::new(store),
        meeting,
        pfad,
    }
}

/// Mock-Anbieter (nicht lokal) mit Notizbuch-Freigabe „Arbeit“.
fn cloud_settings(port: u16) -> AppSettings {
    let mut s = settings_with_mock_provider(port);
    s.voice_call_knowledge = vec!["notebook".into()];
    s.voice_call_notebook_sections = vec!["notizbuch:nb1".into()];
    s
}

fn limits(pfad: &'static std::path::Path) -> RunLimits {
    RunLimits {
        budget_chars: None,
        timeout: Duration::from_secs(30),
        free_mb: || 0,
        notizbuch_ablage: Some(pfad),
    }
}

fn reply() -> Value {
    json!({
        "zusammenfassung": [{"ref": null, "text": "Budget und Rollout besprochen.", "sources": ["S0"]}],
        "besprochene_punkte": [{"ref": "N1", "text": "", "sources": ["S1"]}],
        "entscheidungen": [],
        "aufgaben": [],
        "offene_fragen": []
    })
}

async fn capturing_mock() -> (u16, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let port = spawn_llm_mock_with(move |body| {
        log.lock().unwrap().push(body.to_string());
        MockReply::Body(chat_body(&reply().to_string()))
    })
    .await;
    (port, seen)
}

fn assert_ohne_notizbuch(requests: &[String]) {
    assert!(!requests.is_empty(), "das Modell wurde gefragt");
    for r in requests {
        for t in NOTIZBUCH_TEXTE {
            assert!(!r.contains(t), "Notizbuch-Text {t:?} im Prompt");
        }
        assert!(!r.contains("Jour fixe Migration"), "Seitentitel im Prompt");
    }
}

fn notes_of(doc: &MeetingDocument) -> EnhancedNotes {
    serde_json::from_str(&doc.body).unwrap()
}

fn nb_texte(notes: &EnhancedNotes) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = notes
        .sections
        .iter()
        .flat_map(|s| s.entries.iter())
        .filter_map(|e| {
            e.note_id
                .as_deref()
                .filter(|id| id.starts_with("nb:"))
                .map(|id| (id.to_string(), e.text.clone()))
        })
        .collect();
    v.sort();
    v
}

fn nb_aufgabe_erledigt(store: &MeetingStore, meeting: &str) -> bool {
    store
        .list_action_items(meeting)
        .unwrap()
        .iter()
        .any(|i| i.text == "Angebot prüfen" && i.status == STATUS_DONE)
}

async fn enhance(fx: &Fx, settings: &AppSettings) -> MeetingDocument {
    let flag = AtomicBool::new(false);
    enhance_guarded(
        &flag,
        settings,
        fx.store.clone(),
        &fx.meeting,
        None,
        limits(fx.pfad),
        &|_, _| {},
    )
    .await
    .unwrap()
}

// ---------------------------------------------------------------------------
// Prompt-Text ohne Modellaufruf
// ---------------------------------------------------------------------------

fn gelesen(fx: &Fx) -> NotizbuchNotizen {
    let z = fx._ablage.zugang();
    let nb = fuer_besprechung_mit(&fx.store, Some((&z, &nur_arbeit())), &fx.meeting);
    assert_eq!(nb.seiten.len(), 1, "Seite s1 zugeordnet und lesbar");
    nb
}

#[test]
fn notizbuch_text_steht_nur_im_prompt_eines_lokalen_modells() {
    let fx = fixture();
    let nb = gelesen(&fx);
    let eigen = fx.store.get_notes(&fx.meeting).unwrap().blocks;

    let lokal = notizquellen(&eigen, &nb, true);
    let prompt = render_notes_for_prompt(&lokal.prompt);
    assert!(lokal.notizbuch_im_prompt);
    for t in NOTIZBUCH_TEXTE {
        assert!(prompt.contains(t), "lokal: {t:?} fehlt");
    }
    assert!(prompt.contains("Eigene Notiz zur Besprechung"));

    let cloud = notizquellen(&eigen, &nb, false);
    let prompt = render_notes_for_prompt(&cloud.prompt);
    assert!(!cloud.notizbuch_im_prompt);
    for t in NOTIZBUCH_TEXTE {
        assert!(!prompt.contains(t), "Cloud: {t:?} im Prompt");
    }
    assert!(prompt.contains("Eigene Notiz zur Besprechung"));
    // Beide behalten alle Bloecke fuer Verlustpruefung und Aufgaben.
    assert_eq!(lokal.alle, cloud.alle);
    assert!(cloud.alle.iter().any(|b| b.id == "nb:s1#t1" && b.checked));
}

#[test]
fn ohne_sicht_gibt_es_keine_zweite_quelle() {
    let fx = fixture();
    assert!(fuer_besprechung_mit(&fx.store, None, &fx.meeting).ist_leer());
}

// ---------------------------------------------------------------------------
// Lauf mit entferntem Anbieter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ein_cloud_lauf_sieht_kein_notizbuch_und_verliert_keinen_block() {
    let fx = fixture();
    let (port, seen) = capturing_mock().await;
    let settings = cloud_settings(port);
    let doc = enhance(&fx, &settings).await;
    assert_ohne_notizbuch(&seen.lock().unwrap());

    // Lokale Anzeige: alle Notizbuch-Bloecke byte-genau im Dokument.
    let notes = notes_of(&doc);
    let ids: Vec<String> = nb_texte(&notes).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, vec!["nb:s1#b1", "nb:s1#b2", "nb:s1#b3", "nb:s1#t1"]);
    assert!(nb_aufgabe_erledigt(&fx.store, &fx.meeting));
    let meta: String = fx
        .store
        .get_connection()
        .unwrap()
        .query_row(
            "SELECT generation_metadata_json FROM meeting_documents WHERE id = ?1",
            rusqlite::params![doc.id],
            |row| row.get(0),
        )
        .unwrap();
    let meta: Value = serde_json::from_str(&meta).unwrap();
    assert_eq!(meta["notizbuch"]["im_prompt"], json!(false));
}

#[tokio::test]
async fn eine_anweisung_mit_cloud_anbieter_filtert_das_notizbuch() {
    let fx = fixture();
    let (port, _) = capturing_mock().await;
    let first = enhance(&fx, &cloud_settings(port)).await;
    let vorher = nb_texte(&notes_of(&first));

    // Der reine Eintragsblock: lokal mit, entfernt ohne Notizbuch.
    let stored = notes_of(&first);
    let lokal = instruction_entries(&stored, true);
    let cloud = instruction_entries(&stored, false);
    for t in NOTIZBUCH_TEXTE {
        assert!(lokal.contains(t), "lokal: {t:?} fehlt");
        assert!(!cloud.contains(t), "Cloud: {t:?} im Eintragsblock");
    }

    // Die Anweisung selbst: Prompt ohne Notizbuch, Notizbuch-Eintraege bleiben unveraendert.
    let (port, seen) = capturing_mock().await;
    let flag = AtomicBool::new(false);
    let second = apply_guarded(
        &flag,
        &cloud_settings(port),
        fx.store.clone(),
        &first.id,
        "kürzer fassen",
        limits(fx.pfad),
    )
    .await
    .unwrap();
    assert_ohne_notizbuch(&seen.lock().unwrap());
    assert!(seen.lock().unwrap()[0].contains("Eigene Notiz zur Besprechung"));
    assert_eq!(nb_texte(&notes_of(&second)), vorher, "nichts verloren");
    assert!(
        nb_aufgabe_erledigt(&fx.store, &fx.meeting),
        "Erledigt bleibt"
    );

    // Notizbuch nicht lesbar: der Erledigt-Stand kommt aus der Vorversion.
    let (port, seen) = capturing_mock().await;
    let fehlt: &'static std::path::Path =
        Box::leak(fx.pfad.with_file_name("fehlt.db").into_boxed_path());
    let third = apply_guarded(
        &flag,
        &cloud_settings(port),
        fx.store.clone(),
        &second.id,
        "noch kürzer",
        limits(fehlt),
    )
    .await
    .unwrap();
    assert_ohne_notizbuch(&seen.lock().unwrap());
    assert_eq!(nb_texte(&notes_of(&third)), vorher);
    assert!(
        nb_aufgabe_erledigt(&fx.store, &fx.meeting),
        "Erledigt aus Vorversion"
    );
}

#[test]
fn eine_handbearbeitung_behaelt_den_erledigt_stand_der_notizbuch_aufgabe() {
    let fx = fixture();
    let nb = gelesen(&fx);
    // Dokument wie nach einem Lauf: die Notizbuch-Aufgabe im Aufgaben-Abschnitt, erledigt.
    let entry = |id: &str, note: &str, text: &str| EnhancedEntry {
        id: id.into(),
        origin: Origin::User,
        text: text.into(),
        note_id: Some(note.into()),
        source_segment_ids: vec![],
        source_slide_ids: vec![],
        assignee: None,
        due: None,
        flags: Default::default(),
    };
    let notes = EnhancedNotes {
        format: DOC_FORMAT.into(),
        template_id: None,
        template_title: String::new(),
        segment_epoch: fx.store.segment_epoch(&fx.meeting).unwrap(),
        sections: vec![EnhancedSection {
            id: "aufgaben".into(),
            title: "Aufgaben".into(),
            kind: SectionKind::Tasks,
            entries: vec![entry("E1", "nb:s1#t1", "Angebot prüfen")],
        }],
        stats: Default::default(),
    };
    let doc = persist(&fx.store, &fx.meeting, &notes, &nb.note_blocks(), json!({})).unwrap();
    assert!(nb_aufgabe_erledigt(&fx.store, &fx.meeting));
    let mut edited = notes.clone();
    edited.sections[0].entries[0].assignee = Some("Frau Beispiel".into());
    update_enhanced(&fx.store, &doc.id, edited, doc.updated_at).unwrap();
    assert!(nb_aufgabe_erledigt(&fx.store, &fx.meeting));
}

// ---------------------------------------------------------------------------
// Zentrale Filterung
// ---------------------------------------------------------------------------

#[test]
fn der_filter_nimmt_notizbuch_eintraege_und_hinweise_und_laesst_ids_stehen() {
    let e = |id: &str, origin: Origin, note: Option<&str>, text: &str| EnhancedEntry {
        id: id.into(),
        origin,
        text: text.into(),
        note_id: note.map(str::to_string),
        source_segment_ids: vec![],
        source_slide_ids: vec![],
        assignee: None,
        due: None,
        flags: Default::default(),
    };
    let notes = EnhancedNotes {
        format: DOC_FORMAT.into(),
        template_id: None,
        template_title: String::new(),
        segment_epoch: 0,
        sections: vec![EnhancedSection {
            id: "weitere_notizen".into(),
            title: "Weitere Notizen".into(),
            kind: SectionKind::Text,
            entries: vec![
                e("E1", Origin::User, Some("N-eigen"), "Eigene Notiz"),
                e(
                    "E2",
                    Origin::User,
                    Some("nb:s1#b1"),
                    "Budget Migration Frankfurt",
                ),
                e(
                    "E3",
                    Origin::Ai,
                    None,
                    "Notizen unvollständig: Geheimer Titel",
                ),
                e("E4", Origin::Ai, None, "Ein KI-Satz"),
            ],
        }],
        stats: Default::default(),
    };
    let lokal = schutz::fuer_modell(&notes, true);
    assert_eq!(*lokal, notes);
    let cloud = schutz::fuer_modell(&notes, false);
    let ids: Vec<&str> = cloud.sections[0]
        .entries
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(ids, vec!["E1", "E4"]);
    assert_eq!(
        schutz::notizbuch_eintrag_ids(&notes),
        ["E2", "E3"].iter().map(|s| s.to_string()).collect()
    );
    // Markdown fuer ein Modell: ohne; Anzeige/Export unveraendert.
    let md = enhanced_to_markdown("T", &cloud);
    assert!(
        !md.contains("Budget") && !md.contains("Geheimer Titel"),
        "{md}"
    );
    let md = enhanced_to_markdown("T", &notes);
    assert!(md.contains("Budget") && md.contains("Geheimer Titel"));

    // Nach einer Anweisung kehrt ein fehlender Hinweis zurueck, IDs fortlaufend.
    let mut sections = cloud.sections.clone();
    assert!(schutz::hinweise_uebernehmen(&notes, &mut sections));
    let texte: Vec<(&str, &str)> = sections[0]
        .entries
        .iter()
        .map(|e| (e.id.as_str(), e.text.as_str()))
        .collect();
    assert_eq!(
        texte,
        vec![
            ("E1", "Eigene Notiz"),
            ("E2", "Ein KI-Satz"),
            ("E3", "Notizen unvollständig: Geheimer Titel"),
        ]
    );
    assert!(!schutz::hinweise_uebernehmen(&notes, &mut sections));
}
