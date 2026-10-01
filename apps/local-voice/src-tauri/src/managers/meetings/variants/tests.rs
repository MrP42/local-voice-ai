use rusqlite::{params, Connection};
use rusqlite_migration::Migrations;

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
        speaker_index: None,
        words: None,
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    store: MeetingStore,
    meeting: String,
}

fn fixture(with_transcript: bool) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
    let meeting = store
        .create_meeting("Test", MeetingSource::Import, None)
        .unwrap()
        .id;
    store.set_status(&meeting, MeetingStatus::Ready).unwrap();
    if with_transcript {
        store
            .append_delta(
                &meeting,
                &TranscriptDelta {
                    new_segments: vec![seg(0, "alt eins"), seg(1, "alt zwei")],
                },
            )
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

fn live_text(f: &Fx) -> Vec<String> {
    f.store
        .get_segments(&f.meeting)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .collect()
}

fn new_variant(f: &Fx, kind: &'static str, texts: &[&str], activate: bool) -> NewVariant {
    NewVariant {
        meeting_id: f.meeting.clone(),
        kind,
        language: Some("de".into()),
        model: None,
        segments: texts
            .iter()
            .enumerate()
            .map(|(i, t)| seg(i as u32, t))
            .collect(),
        activate,
    }
}

// ---------------------------------------------------------------------------
// Migration (Index 6)
// ---------------------------------------------------------------------------

#[test]
fn migration_6_is_the_next_index_after_the_register() {
    // A1 endet bei Index 5, A3 ist Index 6, U7 (Warteschlange) folgt als Index 7.
    assert!(
        MIGRATIONS.len() >= 8,
        "A1 = 5, A3 = 6, U7 = 7; weitere Schritte (B1 = 8) werden hinten angehaengt"
    );
    let mut c = Connection::open_in_memory().unwrap();
    Migrations::new(MIGRATIONS[..7].to_vec())
        .to_latest(&mut c)
        .unwrap();
    let has = |table: &str| -> bool {
        c.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            == 1
    };
    assert!(has("transcript_variants"), "Index 6 legt die Fassungen an");
    assert!(!has("import_queue"), "die Warteschlange folgt erst mit Index 7");
}

fn old_db(path: &std::path::Path) -> Connection {
    let mut c = Connection::open(path).unwrap();
    Migrations::new(MIGRATIONS[..6].to_vec())
        .to_latest(&mut c)
        .unwrap();
    for (id, source) in [("L1", "live"), ("S1", "subtitle"), ("E1", "import")] {
        c.execute(
            "INSERT INTO meetings (id, title, status, source, language, created_at, updated_at)
             VALUES (?1, 't', 'ready', ?2, 'de', 100, 100)",
            params![id, source],
        )
        .unwrap();
    }
    let seg_json = r#"[{"segment_index":0,"text":"hallo","start_ms":0,"end_ms":900,"channel":2,"speaker_index":null}]"#;
    for (tid, mid, json) in [
        ("T1", "L1", seg_json),
        ("T2", "S1", seg_json),
        ("T3", "E1", "[]"),
    ] {
        c.execute(
            "INSERT INTO transcripts (id, meeting_id, model, segments_json, created_at, updated_at)
             VALUES (?1, ?2, 'whisper', ?3, 100, 100)",
            params![tid, mid, json],
        )
        .unwrap();
    }
    c
}

fn variant_rows(c: &Connection) -> Vec<(String, String, u32, bool)> {
    let mut stmt = c
        .prepare(
            "SELECT meeting_id, kind, number, active FROM transcript_variants ORDER BY meeting_id",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn existing_transcripts_become_version_1_and_a_second_start_adds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.db");
    drop(old_db(&path));
    // Erster Start mit A3: Migration.
    let store = MeetingStore::open_at(&path).unwrap();
    let c = store.get_connection().unwrap();
    assert_eq!(
        variant_rows(&c),
        vec![
            ("L1".to_string(), "stt".to_string(), 1, true),
            ("S1".to_string(), "subtitles_manual".to_string(), 1, true),
        ],
        "das leere Transkript (E1) bekommt keine Fassung"
    );
    drop(c);
    drop(store);
    // Zweiter Start: nichts doppelt.
    let store = MeetingStore::open_at(&path).unwrap();
    let c = store.get_connection().unwrap();
    assert_eq!(variant_rows(&c).len(), 2);
    // Auch die Rueckfuellung allein ist idempotent (Abbruch und Wiederholung).
    let (_, backfill) = VARIANTS_MIGRATION.split_once("INSERT INTO").unwrap();
    let backfill = format!("INSERT INTO{backfill}");
    c.execute_batch(&backfill).unwrap();
    c.execute_batch(&backfill).unwrap();
    assert_eq!(variant_rows(&c).len(), 2);
    // Die Altdaten sind unberuehrt.
    let text: String = c
        .query_row(
            "SELECT segments_json FROM transcripts WHERE id = 'T1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(text.contains("hallo"));
}

// ---------------------------------------------------------------------------
// Fassungen
// ---------------------------------------------------------------------------

#[test]
fn a_transcript_without_a_variant_becomes_version_1_once() {
    let f = fixture(true);
    let mut c = conn(&f);
    let first = list(&mut c, &f.meeting).unwrap();
    let again = list(&mut c, &f.meeting).unwrap();
    assert_eq!(first, again, "zweimal gelistet: keine Dublette");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].number, 1);
    assert!(first[0].active);
    assert_eq!(first[0].kind, KIND_OWN);
    assert_eq!(first[0].segment_count, 2);
}

#[test]
fn a_meeting_without_transcript_has_no_variants_and_the_first_added_becomes_active() {
    let f = fixture(false);
    let mut c = conn(&f);
    assert!(list(&mut c, &f.meeting).unwrap().is_empty());
    let id = add(
        &mut c,
        new_variant(&f, KIND_SUBTITLES_AUTO, &["neu eins"], false),
    )
    .unwrap();
    let variants = list(&mut c, &f.meeting).unwrap();
    assert_eq!(variants.len(), 1);
    assert!(variants[0].active, "die erste Fassung wird aktiv");
    assert_eq!(variants[0].id, id);
    assert_eq!(live_text(&f), vec!["neu eins"]);
}

#[test]
fn choosing_a_variant_swaps_the_active_transcript_and_keeps_manual_edits() {
    let f = fixture(true);
    let mut c = conn(&f);
    list(&mut c, &f.meeting).unwrap();
    let subs = add(
        &mut c,
        new_variant(
            &f,
            KIND_SUBTITLES_MANUAL,
            &["sub eins", "sub zwei", "sub drei"],
            false,
        ),
    )
    .unwrap();
    assert_eq!(
        live_text(&f),
        vec!["alt eins", "alt zwei"],
        "Anlegen aendert nichts"
    );
    let epoch_before = f.store.segment_epoch(&f.meeting).unwrap();
    // Korrektur von Hand an der aktiven Fassung.
    c.execute(
        "UPDATE transcripts SET segments_json = replace(segments_json, 'alt zwei', 'alt ZWEI korrigiert')
         WHERE meeting_id = ?1",
        params![f.meeting],
    )
    .unwrap();
    let chosen = activate(&mut c, &subs).unwrap();
    assert!(chosen.active);
    assert_eq!(live_text(&f), vec!["sub eins", "sub zwei", "sub drei"]);
    assert!(
        f.store.segment_epoch(&f.meeting).unwrap() > epoch_before,
        "Belege erkennen den Wechsel"
    );
    // Zurueck zur ersten: die Handkorrektur ist erhalten.
    let first = list(&mut c, &f.meeting).unwrap()[0].id.clone();
    activate(&mut c, &first).unwrap();
    assert_eq!(live_text(&f), vec!["alt eins", "alt ZWEI korrigiert"]);
    let (_v, segments) = get_segments(&c, &subs).unwrap();
    assert_eq!(segments.len(), 3);
    let actives = list(&mut c, &f.meeting)
        .unwrap()
        .into_iter()
        .filter(|v| v.active)
        .count();
    assert_eq!(actives, 1, "genau eine Fassung ist aktiv");
}

#[test]
fn the_database_allows_only_one_active_variant_per_meeting() {
    let f = fixture(true);
    let mut c = conn(&f);
    list(&mut c, &f.meeting).unwrap();
    let err = c.execute(
        "INSERT INTO transcript_variants (id, meeting_id, kind, segments_json, number, created_at, active)
         VALUES ('x', ?1, 'stt', '[]', 9, 1, 1)",
        params![f.meeting],
    );
    assert!(err.is_err());
}

#[test]
fn activation_is_all_or_nothing() {
    let f = fixture(true);
    let mut c = conn(&f);
    list(&mut c, &f.meeting).unwrap();
    let other = add(&mut c, new_variant(&f, KIND_OWN, &["x"], false)).unwrap();
    // Ein Schreibfehler mitten im Wechsel (hier: die Tabelle der Deltas fehlt).
    c.execute_batch("DROP TABLE transcript_deltas").unwrap();
    assert!(activate(&mut c, &other).is_err());
    let variants = list(&mut c, &f.meeting).unwrap();
    assert_eq!(
        variants.iter().find(|v| v.active).unwrap().number,
        1,
        "die alte bleibt aktiv"
    );
    assert_eq!(live_text(&f), vec!["alt eins", "alt zwei"]);
}

#[test]
fn no_switch_while_the_meeting_is_being_processed() {
    let f = fixture(true);
    let mut c = conn(&f);
    list(&mut c, &f.meeting).unwrap();
    let other = add(&mut c, new_variant(&f, KIND_OWN, &["x"], false)).unwrap();
    f.store
        .set_status(&f.meeting, MeetingStatus::Processing)
        .unwrap();
    assert_eq!(activate(&mut c, &other).unwrap_err(), VariantError::Busy);
    assert_eq!(live_text(&f), vec!["alt eins", "alt zwei"]);
}

#[test]
fn an_empty_variant_is_refused_and_an_unknown_one_is_not_found() {
    let f = fixture(true);
    let mut c = conn(&f);
    let mut empty = new_variant(&f, KIND_OWN, &[], false);
    empty.segments.clear();
    assert_eq!(add(&mut c, empty).unwrap_err(), VariantError::Empty);
    assert_eq!(
        activate(&mut c, "nope").unwrap_err(),
        VariantError::NotFound
    );
    assert_eq!(list(&mut c, "nope").unwrap_err(), VariantError::NotFound);
}

// ---------------------------------------------------------------------------
// B17: Neu-Transkription
// ---------------------------------------------------------------------------

/// Was die Pipeline tut: Segmente des neuen Laufs live in `transcripts` schreiben.
fn run_writes(f: &Fx, texts: &[&str]) {
    f.store.clear_segments(&f.meeting).unwrap();
    f.store
        .append_delta(
            &f.meeting,
            &TranscriptDelta {
                new_segments: texts
                    .iter()
                    .enumerate()
                    .map(|(i, t)| seg(i as u32, t))
                    .collect(),
            },
        )
        .unwrap();
}

#[test]
fn a_finished_rerun_adds_a_new_active_variant_and_keeps_the_old_one() {
    let f = fixture(true);
    let mut c = conn(&f);
    begin_rerun(&mut c, &f.meeting).unwrap();
    run_writes(&f, &["neu eins", "neu zwei", "neu drei"]);
    let id = finish_rerun(&mut c, &f.meeting, KIND_RETRANSCRIBED, Some("de")).unwrap();
    let variants = list(&mut c, &f.meeting).unwrap();
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].number, 1);
    assert!(!variants[0].active);
    assert_eq!(variants[1].id, id);
    assert_eq!(variants[1].kind, KIND_RETRANSCRIBED);
    assert!(variants[1].active);
    assert_eq!(live_text(&f), vec!["neu eins", "neu zwei", "neu drei"]);
    let (_v, old) = get_segments(&c, &variants[0].id).unwrap();
    assert_eq!(
        old.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
        vec!["alt eins", "alt zwei"]
    );
}

#[test]
fn a_stopped_rerun_leaves_the_old_transcript_active_b17() {
    let f = fixture(true);
    let mut c = conn(&f);
    begin_rerun(&mut c, &f.meeting).unwrap();
    run_writes(&f, &["halb"]);
    assert_eq!(
        live_text(&f),
        vec!["halb"],
        "die Arbeitskopie ist waehrend des Laufs sichtbar"
    );
    assert!(abort_rerun(&mut c, &f.meeting).unwrap());
    assert_eq!(live_text(&f), vec!["alt eins", "alt zwei"]);
    let variants = list(&mut c, &f.meeting).unwrap();
    assert_eq!(variants.len(), 1, "keine halbe Fassung bleibt");
    assert!(variants[0].active);
    assert!(
        !abort_rerun(&mut c, &f.meeting).unwrap(),
        "zweiter Abbruch: nichts zu tun"
    );
}

#[test]
fn a_crash_during_a_rerun_is_repaired_by_recover() {
    let f = fixture(true);
    let mut c = conn(&f);
    begin_rerun(&mut c, &f.meeting).unwrap();
    run_writes(&f, &["halb"]);
    // Absturz: weder finish noch abort. Neue Verbindung, wie nach einem Neustart.
    drop(c);
    let mut c = conn(&f);
    assert!(recover_interrupted(&mut c, &f.meeting).unwrap());
    assert_eq!(live_text(&f), vec!["alt eins", "alt zwei"]);
    assert_eq!(list(&mut c, &f.meeting).unwrap().len(), 1);
    // Ohne Marke tut recover nichts: ein gesundes Transkript bleibt, wie es ist.
    assert!(!recover_interrupted(&mut c, &f.meeting).unwrap());
    assert_eq!(live_text(&f), vec!["alt eins", "alt zwei"]);
}

#[test]
fn a_new_rerun_first_repairs_an_interrupted_one() {
    let f = fixture(true);
    let mut c = conn(&f);
    begin_rerun(&mut c, &f.meeting).unwrap();
    run_writes(&f, &["halb"]);
    begin_rerun(&mut c, &f.meeting).unwrap();
    // Die zweite Sicherung enthaelt die ECHTE alte Fassung, nicht die Arbeitskopie.
    run_writes(&f, &["zweiter lauf"]);
    finish_rerun(&mut c, &f.meeting, KIND_RETRANSCRIBED, None).unwrap();
    let variants = list(&mut c, &f.meeting).unwrap();
    let (_v, first) = get_segments(&c, &variants[0].id).unwrap();
    assert_eq!(first[0].text, "alt eins");
}

#[test]
fn a_rerun_on_a_meeting_without_transcript_clears_on_abort_and_numbers_from_1() {
    let f = fixture(false);
    let mut c = conn(&f);
    begin_rerun(&mut c, &f.meeting).unwrap();
    run_writes(&f, &["erste arbeit"]);
    assert!(abort_rerun(&mut c, &f.meeting).unwrap());
    assert!(live_text(&f).is_empty(), "die halbe Arbeitskopie ist weg");
    assert!(list(&mut c, &f.meeting).unwrap().is_empty());
    begin_rerun(&mut c, &f.meeting).unwrap();
    run_writes(&f, &["echt eins"]);
    finish_rerun(&mut c, &f.meeting, KIND_OWN, None).unwrap();
    let variants = list(&mut c, &f.meeting).unwrap();
    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].number, 1);
    assert!(variants[0].active);
}

#[test]
fn finishing_without_a_begun_rerun_or_with_nothing_written_is_refused() {
    let f = fixture(true);
    let mut c = conn(&f);
    assert_eq!(
        finish_rerun(&mut c, &f.meeting, KIND_OWN, None).unwrap_err(),
        VariantError::NoRerun
    );
    begin_rerun(&mut c, &f.meeting).unwrap();
    f.store.clear_segments(&f.meeting).unwrap();
    assert_eq!(
        finish_rerun(&mut c, &f.meeting, KIND_OWN, None).unwrap_err(),
        VariantError::Empty,
        "ein leeres Ergebnis wird nie zur aktiven Fassung"
    );
    // Die alte Fassung ist mit der Marke weiter wiederherstellbar.
    assert!(abort_rerun(&mut c, &f.meeting).unwrap());
    assert_eq!(live_text(&f), vec!["alt eins", "alt zwei"]);
}
