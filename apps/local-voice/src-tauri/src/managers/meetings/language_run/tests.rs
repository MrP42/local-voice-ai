use rusqlite::params;

use super::*;
use crate::managers::meetings::language::{SOURCE_PROBE, SOURCE_SETTING, SOURCE_TEXT};
use crate::managers::meetings::store::{
    MeetingSource, MeetingStatus, StoredSegment, TranscriptDelta,
};

const EN: [&str; 3] = [
    "Today we are going to talk about the load profile of the municipal utility.",
    "Afterwards we discuss the storage system that we want to buy next year.",
    "The peaks on Monday morning should become flatter with it.",
];
const DE: [&str; 3] = [
    "Wir sprechen heute über den Lastgang der Stadtwerke und danach über den Speicher.",
    "Den Speicher wollen wir im nächsten Jahr beschaffen, damit die Spitzen flacher werden.",
    "Besonders am Montagmorgen entstehen derzeit sehr hohe Lastspitzen im Netz.",
];

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
}

fn fixture(texts: &[&str]) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
    let meeting = store
        .create_meeting("Test", MeetingSource::Import, None)
        .unwrap()
        .id;
    store.set_status(&meeting, MeetingStatus::Ready).unwrap();
    store
        .append_delta(
            &meeting,
            &TranscriptDelta {
                new_segments: texts.iter().enumerate().map(|(i, t)| seg(i as u32, t)).collect(),
            },
        )
        .unwrap();
    Fx {
        _dir: dir,
        store,
        meeting,
    }
}

fn column(f: &Fx, sql: &str) -> Option<String> {
    f.store
        .get_connection()
        .unwrap()
        .query_row(sql, params![f.meeting], |r| r.get::<_, Option<String>>(0))
        .unwrap()
}

fn meeting_language(f: &Fx) -> Option<String> {
    column(f, "SELECT language FROM meetings WHERE id = ?1")
}

fn transcript_language(f: &Fx) -> Option<String> {
    column(f, "SELECT language FROM transcripts WHERE meeting_id = ?1")
}

// ---------------------------------------------------------------------------
// Vorgabe je Thread
// ---------------------------------------------------------------------------

#[test]
fn the_override_is_restored_when_it_ends_and_nests() {
    assert_eq!(LanguageOverride::current(), None);
    {
        let _outer = LanguageOverride::set(Some("en".into()), Some("model-a".into()));
        assert_eq!(LanguageOverride::current().as_deref(), Some("en"));
        assert_eq!(LanguageOverride::current_model().as_deref(), Some("model-a"));
        {
            let _inner = LanguageOverride::set(Some("auto".into()), None);
            assert_eq!(LanguageOverride::current().as_deref(), Some("auto"));
            assert_eq!(LanguageOverride::current_model(), None);
        }
        assert_eq!(LanguageOverride::current().as_deref(), Some("en"), "aussen wieder da");
        assert_eq!(LanguageOverride::current_model().as_deref(), Some("model-a"));
    }
    assert_eq!(LanguageOverride::current(), None);
    assert_eq!(LanguageOverride::current_model(), None);
}

#[test]
fn two_threads_never_see_each_others_override() {
    // Zwei gleichzeitige Importe der Warteschlange: je Thread eine eigene Sprache.
    let _mine = LanguageOverride::set(Some("de".into()), None);
    let other = std::thread::spawn(|| {
        assert_eq!(LanguageOverride::current(), None, "ein neuer Thread startet leer");
        let _theirs = LanguageOverride::set(Some("fr".into()), None);
        LanguageOverride::current()
    })
    .join()
    .unwrap();
    assert_eq!(other.as_deref(), Some("fr"));
    assert_eq!(LanguageOverride::current().as_deref(), Some("de"));
}

#[test]
fn a_panic_inside_the_scope_still_restores_the_override() {
    let result = std::panic::catch_unwind(|| {
        let _guard = LanguageOverride::set(Some("ja".into()), None);
        panic!("Engine abgestuerzt");
    });
    assert!(result.is_err());
    assert_eq!(LanguageOverride::current(), None, "kein Rest fuer den naechsten Lauf");
}

// ---------------------------------------------------------------------------
// Ablage
// ---------------------------------------------------------------------------

#[test]
fn the_language_is_stored_everywhere_it_is_read() {
    let f = fixture(&EN);
    let mut conn = f.store.get_connection().unwrap();
    let variant = variants::list(&mut conn, &f.meeting).unwrap()[0].id.clone();
    drop(conn);
    let stored = StoredLanguage {
        code: "en".into(),
        source: SOURCE_PROBE.into(),
        confidence: Some(0.9),
        forced: None,
        mismatch: None,
        model_id: Some("m".into()),
    };
    persist(&f.store, &f.meeting, &stored).unwrap();
    assert_eq!(meeting_language(&f).as_deref(), Some("en"));
    assert_eq!(transcript_language(&f).as_deref(), Some("en"));
    let mut conn = f.store.get_connection().unwrap();
    let list = variants::list(&mut conn, &f.meeting).unwrap();
    assert_eq!(list.iter().find(|v| v.id == variant).unwrap().language.as_deref(), Some("en"));
    assert_eq!(stored_language(&f.store, &f.meeting), Some(stored));
}

#[test]
fn storing_the_language_keeps_the_other_metadata_keys() {
    let f = fixture(&EN);
    f.store
        .set_metadata_key(&f.meeting, "timeline", serde_json::json!({"a": 1}))
        .unwrap();
    persist(
        &f.store,
        &f.meeting,
        &StoredLanguage {
            code: "en".into(),
            source: SOURCE_TEXT.into(),
            confidence: None,
            forced: None,
            mismatch: None,
            model_id: None,
        },
    )
    .unwrap();
    let meta = f.store.metadata_json(&f.meeting).unwrap().unwrap();
    assert_eq!(meta["timeline"]["a"], 1);
    assert_eq!(meta["language"]["code"], "en");
}

#[test]
fn a_transcript_without_stored_language_reads_as_unknown() {
    let f = fixture(&EN);
    assert_eq!(stored_language(&f.store, &f.meeting), None);
}

// ---------------------------------------------------------------------------
// Nach dem Lauf
// ---------------------------------------------------------------------------

#[test]
fn without_a_plan_the_language_comes_from_the_transcript_text() {
    let f = fixture(&EN);
    finalize_from_text(&f.store, &f.meeting, Some("whisper".into()));
    let stored = stored_language(&f.store, &f.meeting).unwrap();
    assert_eq!((stored.code.as_str(), stored.source.as_str()), ("en", SOURCE_TEXT));
    assert_eq!(stored.model_id.as_deref(), Some("whisper"));
    assert_eq!(meeting_language(&f).as_deref(), Some("en"));
}

#[test]
fn a_language_the_user_chose_is_never_replaced_by_a_detection() {
    let f = fixture(&EN);
    persist(
        &f.store,
        &f.meeting,
        &StoredLanguage {
            code: "de".into(),
            source: SOURCE_USER.into(),
            confidence: Some(1.0),
            forced: None,
            mismatch: None,
            model_id: None,
        },
    )
    .unwrap();
    finalize_from_text(&f.store, &f.meeting, None);
    assert_eq!(stored_language(&f.store, &f.meeting).unwrap().code, "de");
    assert_eq!(meeting_language(&f).as_deref(), Some("de"));
}

#[test]
fn a_transcript_too_short_to_judge_stores_nothing() {
    let f = fixture(&["Okay.", "Ja."]);
    finalize_from_text(&f.store, &f.meeting, None);
    assert_eq!(stored_language(&f.store, &f.meeting), None);
    assert_eq!(meeting_language(&f).as_deref(), None);
}

#[test]
fn gap_placeholders_do_not_count_as_text() {
    // Eine Luecke traegt deutsche Worte ("Nicht transkribiert ... bitte anhoeren"): sie darf
    // das Urteil ueber den englischen Rest nicht kippen.
    let mut texts: Vec<String> = EN.iter().map(|s| (*s).to_string()).collect();
    for _ in 0..4 {
        texts.push(crate::managers::meetings::import::gap_placeholder(1000, 5000));
    }
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let f = fixture(&refs);
    finalize_from_text(&f.store, &f.meeting, None);
    assert_eq!(stored_language(&f.store, &f.meeting).unwrap().code, "en");
}

#[test]
fn a_fixed_german_setting_is_corrected_by_an_english_transcript() {
    // Einstellung "de", das Modell hat trotzdem Englisch geliefert: gespeichert wird die
    // Sprache des Textes, die Einstellung steht als `forced` daneben.
    let f = fixture(&EN);
    let plan = LanguagePlan::planned_for_test(Some(("de".to_string(), SOURCE_SETTING, 1.0)), "m");
    finalize(&f.store, &f.meeting, &plan, Some("parakeet".into()));
    let stored = stored_language(&f.store, &f.meeting).unwrap();
    assert_eq!(stored.code, "en");
    assert_eq!(stored.source, SOURCE_TEXT);
    assert_eq!(stored.forced.as_deref(), Some("de"));
    assert_eq!(meeting_language(&f).as_deref(), Some("en"));
}

#[test]
fn a_matching_probe_result_is_stored_as_found() {
    let f = fixture(&DE);
    let plan = LanguagePlan::planned_for_test(Some(("de".to_string(), SOURCE_PROBE, 1.0)), "m");
    finalize(&f.store, &f.meeting, &plan, None);
    let stored = stored_language(&f.store, &f.meeting).unwrap();
    assert_eq!((stored.code.as_str(), stored.source.as_str()), ("de", SOURCE_PROBE));
    assert_eq!(stored.mismatch, None);
    assert_eq!(stored.model_id.as_deref(), Some("m"));
}

// ---------------------------------------------------------------------------
// Anzeige
// ---------------------------------------------------------------------------

fn catalog() -> Vec<ModelCandidate> {
    let wide: Vec<String> = ["en", "de", "fr", "ja", "es", "it", "pt", "nl", "pl", "ru", "tr", "ar"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    vec![
        ModelCandidate {
            id: "english-only".into(),
            name: "English Only".into(),
            languages: vec!["en".into()],
            downloaded: true,
            size_mb: 600,
            accuracy: 91,
        },
        ModelCandidate {
            id: "multi".into(),
            name: "Multi".into(),
            languages: wide,
            downloaded: true,
            size_mb: 1500,
            accuracy: 90,
        },
    ]
}

#[test]
fn the_info_reads_the_stored_language_and_the_transcript_model() {
    let f = fixture(&DE);
    f.store
        .get_connection()
        .unwrap()
        .execute(
            "UPDATE transcripts SET model = 'english-only' WHERE meeting_id = ?1",
            params![f.meeting],
        )
        .unwrap();
    persist(
        &f.store,
        &f.meeting,
        &StoredLanguage {
            code: "de".into(),
            source: SOURCE_TEXT.into(),
            confidence: Some(0.99),
            forced: None,
            mismatch: None,
            model_id: None,
        },
    )
    .unwrap();
    let info = info(&f.store, &catalog(), &f.meeting).unwrap();
    assert_eq!(info.code.as_deref(), Some("de"));
    assert_eq!(info.model_id.as_deref(), Some("english-only"));
    assert_eq!(info.model_covers, Some(false));
    assert_eq!(info.suggestion.unwrap().model_id, "multi");
}

#[test]
fn the_model_for_a_language_prefers_a_covering_installed_one() {
    let suggestion = model_for_language(&catalog(), "english-only", "ja").unwrap();
    assert_eq!(suggestion.model_id, "multi");
    assert!(suggestion.downloaded);
    // Das aktuelle Modell deckt die Sprache ab: es bleibt.
    assert_eq!(model_for_language(&catalog(), "english-only", "en").unwrap().model_id, "english-only");
    assert_eq!(model_for_language(&catalog(), "multi", "auto"), None);
}

#[test]
fn the_info_of_an_unknown_meeting_is_an_error_code() {
    let f = fixture(&EN);
    assert_eq!(info(&f.store, &catalog(), "gibt-es-nicht").unwrap_err(), "meeting_not_found");
}
