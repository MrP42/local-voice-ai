//! G5: Fassungsart `translation`: Migration (Index 9, hinter G3) und Ablage. Die Uebersetzung selbst
//! (Modellaufruf, Treuepruefung) pruefen die Tests in `translate/tests.rs`.

use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};

use super::*;
use crate::managers::meetings::store::{
    MeetingSource, MeetingStatus, MeetingStore, TranscriptDelta, MIGRATIONS,
};

fn seg(i: u32, text: &str) -> StoredSegment {
    StoredSegment {
        segment_index: i,
        text: text.to_string(),
        start_ms: u64::from(i) * 1000,
        end_ms: u64::from(i) * 1000 + 900,
        channel: 2,
        speaker_index: Some(i % 2),
        words: None,
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    store: MeetingStore,
    meeting: String,
}

/// Eine Besprechung mit englischem Original als Fassung 1 (aktiv).
fn fixture() -> Fx {
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
                new_segments: vec![seg(0, "We pay 120 euros."), seg(1, "Anna agrees.")],
            },
        )
        .unwrap();
    {
        let c = store.get_connection().unwrap();
        c.execute(
            "UPDATE transcripts SET language = 'en' WHERE meeting_id = ?1",
            params![meeting],
        )
        .unwrap();
        c.execute("UPDATE meetings SET language = 'en' WHERE id = ?1", params![meeting])
            .unwrap();
    }
    Fx {
        _dir: dir,
        store,
        meeting,
    }
}

fn conn(f: &Fx) -> Connection {
    f.store.get_connection().unwrap()
}

fn german(texts: &[&str]) -> Vec<StoredSegment> {
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| seg(i as u32, t))
        .collect()
}

fn extra(source: &str) -> TranslationExtra {
    TranslationExtra {
        source_variant_id: source.to_string(),
        source_language: Some("en".into()),
        meta_json: r#"{"target_language":"de","flagged":[{"segment_index":0,"reasons":["numbers"]}]}"#
            .to_string(),
    }
}

fn translation(f: &Fx, activate: bool) -> NewVariant {
    NewVariant {
        meeting_id: f.meeting.clone(),
        kind: KIND_TRANSLATION,
        language: Some("de".into()),
        model: Some("llm-gemma4-e4b-q4".into()),
        segments: german(&["Wir zahlen 120 Euro.", "Anna ist einverstanden."]),
        activate,
    }
}

// ---------------------------------------------------------------------------
// Migration (Index 9)
// ---------------------------------------------------------------------------

/// Eine Datenbank vor G5 (Index 0 bis 8, also mit G3) mit Fassungen in allen Zustaenden: aktiv,
/// inaktiv, mit Neu-Lauf-Marke und geloescht.
fn db_before_g5(path: &std::path::Path) -> Connection {
    let mut c = Connection::open(path).unwrap();
    Migrations::new(MIGRATIONS[..9].to_vec())
        .to_latest(&mut c)
        .unwrap();
    c.execute(
        "INSERT INTO meetings (id, title, status, source, language, created_at, updated_at)
         VALUES ('M1', 't', 'ready', 'import', 'de', 100, 100)",
        [],
    )
    .unwrap();
    let json = r#"[{"segment_index":0,"text":"hallo","start_ms":0,"end_ms":900,"channel":2,"speaker_index":null}]"#;
    for (id, kind, number, active, marker, deleted) in [
        ("V1", "stt", 1, 0, None, None),
        ("V2", "retranscribed", 2, 1, Some(150_i64), None),
        ("V3", "merged", 3, 0, None, Some(160_i64)),
        ("V4", "subtitles_auto", 4, 0, None, None),
    ] {
        c.execute(
            "INSERT INTO transcript_variants (id, meeting_id, kind, language, model, segments_json,
                 speaker_hints_json, number, created_at, active, rerun_started_at, deleted_at)
             VALUES (?1, 'M1', ?2, 'de', 'whisper', ?3, '{\"turns\":[]}', ?4, 100, ?5, ?6, ?7)",
            params![id, kind, json, number, active, marker, deleted],
        )
        .unwrap();
    }
    c
}

fn variant_dump(c: &Connection) -> Vec<String> {
    let mut stmt = c
        .prepare(
            "SELECT id || '|' || meeting_id || '|' || kind || '|' || COALESCE(language,'-') || '|'
                 || COALESCE(model,'-') || '|' || segments_json || '|' || COALESCE(speaker_hints_json,'-')
                 || '|' || number || '|' || created_at || '|' || active || '|'
                 || COALESCE(rerun_started_at,'-') || '|' || COALESCE(deleted_at,'-')
             FROM transcript_variants ORDER BY id",
        )
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn migration_9_is_a_single_step_after_the_project_minutes() {
    assert!(
        MIGRATIONS.len() >= 10,
        "G5 ist Index 9: nach A1 = 5, A3 = 6, U7 = 7, G3 = 8"
    );
}

#[test]
fn migration_9_widens_the_kind_check_and_keeps_every_variant_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.db");
    let before = {
        let c = db_before_g5(&path);
        // Vorher: 'translation' ist verboten.
        assert!(c
            .execute(
                "INSERT INTO transcript_variants (id, meeting_id, kind, segments_json, number, created_at)
                 VALUES ('X', 'M1', 'translation', '[]', 9, 1)",
                [],
            )
            .is_err());
        variant_dump(&c)
    };
    assert_eq!(before.len(), 4);

    let store = MeetingStore::open_at(&path).unwrap();
    let c = store.get_connection().unwrap();
    assert_eq!(variant_dump(&c), before, "alle Zeilen wie zuvor, auch Marke und geloeschte");
    // Die neuen Spalten sind leer, nichts wurde erfunden.
    let extras: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM transcript_variants
             WHERE source_variant_id IS NOT NULL OR source_language IS NOT NULL OR meta_json IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(extras, 0);
    // Jetzt erlaubt, und nur diese Art mehr.
    c.execute(
        "INSERT INTO transcript_variants (id, meeting_id, kind, segments_json, number, created_at,
             source_variant_id, source_language, meta_json)
         VALUES ('T', 'M1', 'translation', '[]', 5, 1, 'V1', 'en', '{}')",
        [],
    )
    .unwrap();
    assert!(c
        .execute(
            "INSERT INTO transcript_variants (id, meeting_id, kind, segments_json, number, created_at)
             VALUES ('U', 'M1', 'unbekannt', '[]', 6, 1)",
            [],
        )
        .is_err());
    // Die Indizes sind wieder da: nur EINE aktive Fassung je Besprechung.
    assert!(c
        .execute(
            "INSERT INTO transcript_variants (id, meeting_id, kind, segments_json, number, created_at, active)
             VALUES ('A', 'M1', 'stt', '[]', 7, 1, 1)",
            [],
        )
        .is_err());
    let indexes: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index'
             AND name IN ('idx_variants_meeting', 'idx_variants_one_active')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexes, 2);
}

#[test]
fn migration_9_runs_once_and_a_second_start_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.db");
    drop(db_before_g5(&path));
    let first = {
        let store = MeetingStore::open_at(&path).unwrap();
        variant_dump(&store.get_connection().unwrap())
    };
    let store = MeetingStore::open_at(&path).unwrap();
    let c = store.get_connection().unwrap();
    assert_eq!(variant_dump(&c), first);
    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(version, MIGRATIONS.len() as i64);
}

#[test]
fn an_abort_inside_migration_9_leaves_the_old_table_complete() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.db");
    let before = variant_dump(&db_before_g5(&path));

    // Der Umbau laeuft bis zum Ende durch und scheitert erst am letzten Befehl:
    // die Transaktion muss ALLES zuruecknehmen (alte Tabelle da, keine halbe neue).
    let broken: &'static str = Box::leak(
        format!("{} SELECT no_such_function();", VARIANTS_TRANSLATION_MIGRATION).into_boxed_str(),
    );
    let mut steps = MIGRATIONS[..9].to_vec();
    steps.push(M::up(broken));
    let mut c = Connection::open(&path).unwrap();
    assert!(Migrations::new(steps).to_latest(&mut c).is_err());

    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(version, 9, "Version unveraendert");
    assert_eq!(variant_dump(&c), before, "keine Fassung verloren");
    let leftovers: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'transcript_variants_g5%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leftovers, 0, "keine halb angelegte Ersatztabelle");
    drop(c);
    // Danach startet die App mit dem richtigen Schritt ganz normal.
    let store = MeetingStore::open_at(&path).unwrap();
    assert_eq!(variant_dump(&store.get_connection().unwrap()), before);
}

// ---------------------------------------------------------------------------
// Ablage
// ---------------------------------------------------------------------------

#[test]
fn a_translation_is_stored_beside_the_original_and_the_original_stays_unchanged() {
    let f = fixture();
    let mut c = conn(&f);
    let original = list(&mut c, &f.meeting).unwrap();
    assert_eq!(original.len(), 1);
    let source = original[0].id.clone();
    let source_json: String = c
        .query_row(
            "SELECT segments_json FROM transcripts WHERE meeting_id = ?1",
            params![f.meeting],
            |r| r.get(0),
        )
        .unwrap();

    let id = add_translation(&mut c, translation(&f, false), extra(&source)).unwrap();

    let all = list(&mut c, &f.meeting).unwrap();
    assert_eq!(all.len(), 2);
    let new = all.iter().find(|v| v.id == id).unwrap();
    assert_eq!(new.kind, KIND_TRANSLATION);
    assert_eq!(new.language.as_deref(), Some("de"));
    assert_eq!(new.source_language.as_deref(), Some("en"));
    assert_eq!(new.source_variant_id.as_deref(), Some(source.as_str()));
    assert_eq!(new.flagged, 1, "ein markierter Satz aus dem Bericht");
    assert!(!new.active, "die Uebersetzung wird nicht von selbst aktiv");
    assert!(all.iter().find(|v| v.id == source).unwrap().active);
    // Das Original (Transkript und Fassung) ist Byte fuer Byte wie zuvor.
    let after_json: String = c
        .query_row(
            "SELECT segments_json FROM transcripts WHERE meeting_id = ?1",
            params![f.meeting],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after_json, source_json);
    let (_, original_segments) = get_segments(&c, &source).unwrap();
    assert_eq!(original_segments[0].text, "We pay 120 euros.");
}

#[test]
fn a_translation_keeps_times_channels_and_speakers_of_the_source() {
    let f = fixture();
    let mut c = conn(&f);
    let source = list(&mut c, &f.meeting).unwrap()[0].id.clone();
    let id = add_translation(&mut c, translation(&f, false), extra(&source)).unwrap();
    let (_, translated) = get_segments(&c, &id).unwrap();
    let (_, original) = get_segments(&c, &source).unwrap();
    assert_eq!(translated.len(), original.len());
    for (t, o) in translated.iter().zip(&original) {
        assert_eq!(
            (t.start_ms, t.end_ms, t.channel, t.speaker_index, t.segment_index),
            (o.start_ms, o.end_ms, o.channel, o.speaker_index, o.segment_index)
        );
    }
}

#[test]
fn switching_between_original_and_translation_loses_nothing() {
    let f = fixture();
    let mut c = conn(&f);
    let source = list(&mut c, &f.meeting).unwrap()[0].id.clone();
    let id = add_translation(&mut c, translation(&f, false), extra(&source)).unwrap();

    let chosen = activate(&mut c, &id).unwrap();
    assert!(chosen.active);
    let live: Vec<String> = f
        .store
        .get_segments(&f.meeting)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(live, vec!["Wir zahlen 120 Euro.", "Anna ist einverstanden."]);
    let language: Option<String> = c
        .query_row(
            "SELECT language FROM transcripts WHERE meeting_id = ?1",
            params![f.meeting],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(language.as_deref(), Some("de"), "die Sprache folgt der aktiven Fassung");

    activate(&mut c, &source).unwrap();
    let back: Vec<String> = f
        .store
        .get_segments(&f.meeting)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(back, vec!["We pay 120 euros.", "Anna agrees."]);
    let language: Option<String> = c
        .query_row(
            "SELECT language FROM transcripts WHERE meeting_id = ?1",
            params![f.meeting],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(language.as_deref(), Some("en"));
    assert_eq!(list(&mut c, &f.meeting).unwrap().len(), 2, "beide Fassungen bleiben");
}

#[test]
fn a_translation_of_an_unknown_or_foreign_source_is_refused() {
    let f = fixture();
    let mut c = conn(&f);
    list(&mut c, &f.meeting).unwrap();
    assert_eq!(
        add_translation(&mut c, translation(&f, false), extra("nicht-da")).unwrap_err(),
        VariantError::NotFound
    );
    // Eine Fassung einer ANDEREN Besprechung ist keine Quelle.
    let other = f
        .store
        .create_meeting("Andere", MeetingSource::Import, None)
        .unwrap()
        .id;
    f.store.set_status(&other, MeetingStatus::Ready).unwrap();
    f.store
        .append_delta(
            &other,
            &TranscriptDelta {
                new_segments: vec![seg(0, "other")],
            },
        )
        .unwrap();
    let foreign = list(&mut c, &other).unwrap()[0].id.clone();
    assert_eq!(
        add_translation(&mut c, translation(&f, false), extra(&foreign)).unwrap_err(),
        VariantError::NotFound
    );
    assert_eq!(list(&mut c, &f.meeting).unwrap().len(), 1, "nichts angelegt");
}

#[test]
fn only_the_translation_kind_goes_through_add_translation() {
    let f = fixture();
    let mut c = conn(&f);
    let source = list(&mut c, &f.meeting).unwrap()[0].id.clone();
    let mut wrong = translation(&f, false);
    wrong.kind = KIND_MERGED;
    assert!(matches!(
        add_translation(&mut c, wrong, extra(&source)).unwrap_err(),
        VariantError::Store(_)
    ));
}

#[test]
fn the_report_of_a_translation_can_be_read_back() {
    let f = fixture();
    let mut c = conn(&f);
    let source = list(&mut c, &f.meeting).unwrap()[0].id.clone();
    let id = add_translation(&mut c, translation(&f, false), extra(&source)).unwrap();
    let meta = get_meta(&c, &id).unwrap().unwrap();
    assert!(meta.contains("target_language"));
    // Eine Fassung ohne Bericht hat keinen.
    assert_eq!(get_meta(&c, &source).unwrap(), None);
    assert_eq!(get_meta(&c, "gibt-es-nicht").unwrap_err(), VariantError::NotFound);
}

#[test]
fn setting_the_language_updates_transcript_meeting_and_active_variant() {
    let f = fixture();
    let mut c = conn(&f);
    let source = list(&mut c, &f.meeting).unwrap()[0].id.clone();
    set_language(&mut c, &f.meeting, "fr").unwrap();
    let transcript: Option<String> = c
        .query_row(
            "SELECT language FROM transcripts WHERE meeting_id = ?1",
            params![f.meeting],
            |r| r.get(0),
        )
        .unwrap();
    let meeting: Option<String> = c
        .query_row("SELECT language FROM meetings WHERE id = ?1", params![f.meeting], |r| r.get(0))
        .unwrap();
    assert_eq!(transcript.as_deref(), Some("fr"));
    assert_eq!(meeting.as_deref(), Some("fr"));
    let variants = list(&mut c, &f.meeting).unwrap();
    assert_eq!(variants.iter().find(|v| v.id == source).unwrap().language.as_deref(), Some("fr"));
    assert_eq!(set_language(&mut c, "gibt-es-nicht", "fr").unwrap_err(), VariantError::NotFound);
}

#[test]
fn a_language_correction_does_not_touch_other_variants() {
    let f = fixture();
    let mut c = conn(&f);
    let source = list(&mut c, &f.meeting).unwrap()[0].id.clone();
    let id = add_translation(&mut c, translation(&f, false), extra(&source)).unwrap();
    set_language(&mut c, &f.meeting, "nl").unwrap();
    let variants = list(&mut c, &f.meeting).unwrap();
    assert_eq!(variants.iter().find(|v| v.id == id).unwrap().language.as_deref(), Some("de"));
}

#[test]
fn no_language_correction_while_a_rerun_is_marked() {
    let f = fixture();
    let mut c = conn(&f);
    list(&mut c, &f.meeting).unwrap();
    begin_rerun(&mut c, &f.meeting).unwrap();
    assert_eq!(set_language(&mut c, &f.meeting, "fr").unwrap_err(), VariantError::Busy);
}

#[test]
fn a_write_failure_leaves_no_half_variant_and_the_original_untouched() {
    // Volle oder schreibgeschuetzte Platte: der Schreibversuch scheitert, es entsteht nichts.
    let f = fixture();
    let mut c = conn(&f);
    let source = list(&mut c, &f.meeting).unwrap()[0].id.clone();
    drop(c);
    let path = f._dir.path().join("m.db");
    let mut read_only = Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let err = add_translation(&mut read_only, translation(&f, false), extra(&source)).unwrap_err();
    assert!(matches!(err, VariantError::Store(_)), "{err:?}");
    drop(read_only);

    let mut c = conn(&f);
    let all = list(&mut c, &f.meeting).unwrap();
    assert_eq!(all.len(), 1, "keine halbe Fassung");
    assert!(all[0].active);
    assert_eq!(
        f.store.get_segments(&f.meeting).unwrap()[0].text,
        "We pay 120 euros."
    );
}
