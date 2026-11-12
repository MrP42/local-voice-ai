//! lva#523: Zustandsuebergaenge der Neu-Transkription, je Absturzpunkt.
//! Ein „Absturz“ ist hier: der Ablauf endet nach Schritt N, danach laeuft nur
//! `settle` (wie beim App-Start, beim Fortsetzen oder beim Oeffnen der Fassungen).

use super::*;
use crate::managers::meetings::store::{MeetingSource, StoredSegment, TranscriptDelta};

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
    id: String,
}

fn fixture() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
    let id = store
        .create_meeting("T", MeetingSource::Import, None)
        .unwrap()
        .id;
    store.set_status(&id, MeetingStatus::Ready).unwrap();
    append(&store, &id, &[(0, "alt eins"), (1, "alt zwei")]);
    Fx {
        _dir: dir,
        store,
        id,
    }
}

fn append(store: &MeetingStore, id: &str, segs: &[(u32, &str)]) {
    store
        .append_delta(
            id,
            &TranscriptDelta {
                new_segments: segs.iter().map(|(i, t)| seg(*i, t)).collect(),
            },
        )
        .unwrap();
}

fn texts(f: &Fx) -> Vec<String> {
    f.store
        .get_segments(&f.id)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .collect()
}

fn variant_count(f: &Fx) -> usize {
    let mut c = f.store.get_connection().unwrap();
    variants::list(&mut c, &f.id).unwrap().len()
}

/// Die Schritte eines Laufs bis einschliesslich `upto` (1..=5), genau in der
/// Reihenfolge von `retranscribe.rs`.
fn run_until(f: &Fx, upto: u8) {
    mark_start(&f.store, &f.id, "ready").unwrap();
    if upto < 2 {
        return;
    }
    let mut c = f.store.get_connection().unwrap();
    variants::begin_rerun(&mut c, &f.id).unwrap();
    if upto < 3 {
        return;
    }
    f.store.clear_segments(&f.id).unwrap();
    if upto < 4 {
        return;
    }
    mark_running(&f.store, &f.id, "neu-modell", variants::KIND_RETRANSCRIBED).unwrap();
    if upto < 5 {
        return;
    }
    append(&f.store, &f.id, &[(0, "neu eins")]);
}

#[test]
fn a_crash_before_or_right_after_clearing_never_keeps_old_segments_as_new() {
    // Blocker 1: Absturz nach Marke `start`, nach begin_rerun, nach dem Leeren und
    // nach `running` ohne neuen Abschnitt: immer die alte Fassung, nie fortsetzbar.
    for upto in 1..=4 {
        let f = fixture();
        run_until(&f, upto);
        let settled = settle(&f.store, &f.id, false, Ends::QUIET).unwrap();
        assert_eq!(
            settled,
            Settled::RolledBack(Some(MeetingStatus::Ready)),
            "Schritt {upto}"
        );
        assert_eq!(texts(&f), vec!["alt eins", "alt zwei"], "Schritt {upto}");
        assert_eq!(read(&f.store, &f.id).unwrap(), None, "Schritt {upto}");
        assert_eq!(variant_count(&f), 1, "Schritt {upto}");
        assert_eq!(prepare_resume(&f.store, &f.id).unwrap(), Resume::Normal);
    }
}

#[test]
fn a_stop_or_crash_after_new_sections_keeps_them_and_the_old_version() {
    let f = fixture();
    run_until(&f, 5);
    let Settled::Kept(Some(r)) = settle(&f.store, &f.id, false, Ends::QUIET).unwrap() else {
        panic!("behalten erwartet");
    };
    assert_eq!(r.model, "neu-modell");
    assert_eq!(texts(&f), vec!["neu eins"]);
    assert_eq!(variant_count(&f), 2);
    // Doppelter Abschluss (doppelter Stopp, danach Wiederherstellung): unveraendert.
    assert_eq!(
        settle(&f.store, &f.id, false, Ends::QUIET).unwrap(),
        Settled::Kept(Some(r.clone()))
    );
    assert_eq!(texts(&f), vec!["neu eins"]);
    assert_eq!(variant_count(&f), 2);
}

#[test]
fn an_old_marker_with_another_epoch_never_claims_a_working_copy() {
    // Blocker 1: Marke `running` aus einem frueheren Lauf, Arbeitskopie einer
    // anderen Epoche (etwa ein Lauf ohne Marke): zuruecknehmen.
    let f = fixture();
    run_until(&f, 5);
    f.store.clear_segments(&f.id).unwrap();
    append(&f.store, &f.id, &[(0, "fremd")]);
    assert_eq!(
        settle(&f.store, &f.id, false, Ends::QUIET).unwrap(),
        Settled::RolledBack(Some(MeetingStatus::Ready))
    );
    assert_eq!(texts(&f), vec!["alt eins", "alt zwei"]);
}

#[test]
fn a_failure_rolls_back_and_a_crash_mid_rollback_still_rolls_back() {
    // Blocker 4: Absicht `rollback` steht dauerhaft; ein spaeterer Abschluss
    // (auch ohne `rollback`-Wunsch) behaelt dann nichts.
    let f = fixture();
    run_until(&f, 5);
    mark_rollback(&f.store, &f.id).unwrap(); // Absturz direkt danach
    assert_eq!(
        settle(&f.store, &f.id, false, Ends::QUIET).unwrap(),
        Settled::RolledBack(Some(MeetingStatus::Ready))
    );
    assert_eq!(texts(&f), vec!["alt eins", "alt zwei"]);
    assert_eq!(variant_count(&f), 1);

    let g = fixture();
    run_until(&g, 5);
    assert_eq!(
        settle(&g.store, &g.id, true, Ends::QUIET).unwrap(),
        Settled::RolledBack(Some(MeetingStatus::Ready))
    );
    assert_eq!(texts(&g), vec!["alt eins", "alt zwei"]);
}

#[test]
fn resuming_reactivates_the_stopped_version_after_a_switch() {
    // Blocker 2: der Nutzer schaltet nach dem Stopp auf die alte Fassung;
    // Fortsetzen ergaenzt trotzdem die gestoppte.
    let f = fixture();
    run_until(&f, 5);
    let Settled::Kept(Some(r)) = settle(&f.store, &f.id, false, Ends::QUIET).unwrap() else {
        panic!("behalten erwartet");
    };
    let old = {
        let mut c = f.store.get_connection().unwrap();
        let list = variants::list(&mut c, &f.id).unwrap();
        list.into_iter().find(|v| v.id != r.variant_id).unwrap().id
    };
    {
        let mut c = f.store.get_connection().unwrap();
        variants::activate(&mut c, &old).unwrap();
    }
    assert_eq!(texts(&f), vec!["alt eins", "alt zwei"]);
    assert_eq!(
        prepare_resume(&f.store, &f.id).unwrap(),
        Resume::Partial("neu-modell".into())
    );
    assert_eq!(texts(&f), vec!["neu eins"]);
}

#[test]
fn a_new_run_over_a_kept_result_restores_it_when_it_leaves_nothing() {
    let f = fixture();
    run_until(&f, 5);
    settle(&f.store, &f.id, false, Ends::QUIET).unwrap();
    f.store.set_status(&f.id, MeetingStatus::Cancelled).unwrap();
    // Zweiter Lauf, gestoppt vor dem ersten Abschnitt.
    mark_start(&f.store, &f.id, "cancelled").unwrap();
    {
        let mut c = f.store.get_connection().unwrap();
        variants::begin_rerun(&mut c, &f.id).unwrap();
    }
    f.store.clear_segments(&f.id).unwrap();
    mark_running(&f.store, &f.id, "zweit", variants::KIND_RETRANSCRIBED).unwrap();
    assert_eq!(
        settle(&f.store, &f.id, false, Ends::QUIET).unwrap(),
        Settled::RolledBack(Some(MeetingStatus::Cancelled))
    );
    assert_eq!(texts(&f), vec!["neu eins"], "das erste Teilergebnis bleibt");
    assert_eq!(
        prepare_resume(&f.store, &f.id).unwrap(),
        Resume::Partial("neu-modell".into()),
        "und bleibt mit seinem Modell fortsetzbar"
    );
}

#[test]
fn completing_a_run_ends_variant_marker_and_status_together() {
    let f = fixture();
    run_until(&f, 5);
    f.store
        .set_status(&f.id, MeetingStatus::Processing)
        .unwrap();
    complete(&f.store, &f.id, variants::KIND_RETRANSCRIBED).unwrap();
    assert_eq!(read(&f.store, &f.id).unwrap(), None);
    assert_eq!(status(&f), "ready");
    assert_eq!(texts(&f), vec!["neu eins"]);
    assert_eq!(
        settle(&f.store, &f.id, false, Ends::QUIET).unwrap(),
        Settled::Nothing
    );
}

#[test]
fn running_without_an_open_run_is_an_error_never_finished() {
    // F1: frueher als „fertig“ gedeutet (Marke weg, `ready`); jetzt ein Fehler ohne
    // jede Aenderung.
    let f = fixture();
    run_until(&f, 5);
    {
        let mut c = f.store.get_connection().unwrap();
        variants::finish_rerun(&mut c, &f.id, variants::KIND_RETRANSCRIBED, None).unwrap();
    }
    f.store
        .set_status(&f.id, MeetingStatus::Processing)
        .unwrap();
    assert!(settle(&f.store, &f.id, false, Ends::QUIET).is_err());
    assert_eq!(read(&f.store, &f.id).unwrap().unwrap().phase, PHASE_RUNNING);
    assert_eq!(status(&f), "processing");
}

fn status(f: &Fx) -> String {
    f.store.get_meeting(&f.id).unwrap().unwrap().status
}

fn raw_metadata(f: &Fx) -> Option<String> {
    f.store
        .get_connection()
        .unwrap()
        .query_row(
            "SELECT metadata_json FROM meetings WHERE id = ?1",
            rusqlite::params![f.id],
            |r| r.get(0),
        )
        .unwrap()
}

fn set_raw_metadata(f: &Fx, value: Option<&str>) {
    f.store
        .get_connection()
        .unwrap()
        .execute(
            "UPDATE meetings SET metadata_json = ?1 WHERE id = ?2",
            rusqlite::params![value, f.id],
        )
        .unwrap();
}

#[test]
fn a_failing_marker_write_undoes_the_kept_version_in_the_same_transaction() {
    // F1/F3 (Fenster nach dem Fassungs-Commit): das Schreiben der Marke scheitert
    // NACH dem Sichern der Fassung in derselben Transaktion -> nichts ist geschehen,
    // ein spaeterer Abschluss behaelt alles.
    let f = fixture();
    run_until(&f, 5);
    f.store
        .set_status(&f.id, MeetingStatus::Processing)
        .unwrap();
    let good = raw_metadata(&f);
    set_raw_metadata(&f, Some("[kaputt"));
    let ends = Ends {
        kept: Some(MeetingStatus::Cancelled),
        resumable: true,
        fallback: Some(MeetingStatus::Ready),
    };
    assert!(settle(&f.store, &f.id, false, ends).is_err());
    {
        let c = f.store.get_connection().unwrap();
        assert!(
            variants::rerun_pending(&c, &f.id).unwrap(),
            "Lauf weiter offen"
        );
    }
    assert_eq!(texts(&f), vec!["neu eins"], "Arbeitskopie unveraendert");
    assert_eq!(variant_count(&f), 1, "keine halbe Fassung");
    assert_eq!(status(&f), "processing");
    set_raw_metadata(&f, good.as_deref());
    assert!(matches!(
        settle(&f.store, &f.id, false, ends).unwrap(),
        Settled::Kept(Some(_))
    ));
    assert_eq!(status(&f), "cancelled", "Status im selben Schritt");
    assert_eq!(texts(&f), vec!["neu eins"]);
    assert_eq!(variant_count(&f), 2);
}

#[test]
fn a_rollback_writes_the_previous_status_in_the_same_transaction() {
    // F4: Vorstatus `failed`, keine Vormarke. Zuruecknehmen, Marke und Status sind
    // ein Schritt; scheitert er, bleibt die Wiederherstellungsinformation stehen.
    let f = fixture();
    f.store.set_status(&f.id, MeetingStatus::Failed).unwrap();
    mark_start(&f.store, &f.id, "failed").unwrap();
    {
        let mut c = f.store.get_connection().unwrap();
        variants::begin_rerun(&mut c, &f.id).unwrap();
    }
    f.store
        .set_status(&f.id, MeetingStatus::Processing)
        .unwrap();
    f.store.clear_segments(&f.id).unwrap();
    mark_running(&f.store, &f.id, "m", variants::KIND_RETRANSCRIBED).unwrap();
    append(&f.store, &f.id, &[(0, "neu")]);
    let good = raw_metadata(&f);
    set_raw_metadata(&f, Some("[kaputt"));
    assert!(settle(&f.store, &f.id, true, Ends::QUIET).is_err());
    assert_eq!(status(&f), "processing");
    set_raw_metadata(&f, good.as_deref());
    assert_eq!(
        settle(&f.store, &f.id, true, Ends::QUIET).unwrap(),
        Settled::RolledBack(Some(MeetingStatus::Failed))
    );
    assert_eq!(status(&f), "failed");
    assert_eq!(texts(&f), vec!["alt eins", "alt zwei"]);
    assert_eq!(read(&f.store, &f.id).unwrap(), None);
}

#[test]
fn a_failed_meeting_is_never_reinterpreted_as_a_kept_result() {
    // F3: Absicht `rollback` liess sich nicht speichern, nur `failed`: der naechste
    // Abschluss nimmt zurueck.
    let f = fixture();
    run_until(&f, 5);
    f.store.set_status(&f.id, MeetingStatus::Failed).unwrap();
    assert_eq!(
        settle(&f.store, &f.id, false, Ends::QUIET).unwrap(),
        Settled::RolledBack(Some(MeetingStatus::Ready))
    );
    assert_eq!(texts(&f), vec!["alt eins", "alt zwei"]);
}

#[test]
fn a_kept_marker_without_a_version_is_rejected_not_resumed() {
    // F5: Marke aus 02c1f87 (`{model, kind}`, ohne Phase und Fassung).
    let f = fixture();
    f.store.set_status(&f.id, MeetingStatus::Cancelled).unwrap();
    write(
        &f.store,
        &f.id,
        serde_json::json!({ "model": "alt", "kind": "retranscribed" }),
    )
    .unwrap();
    assert_eq!(
        prepare_resume(&f.store, &f.id).unwrap_err(),
        "partial_rerun_unassigned"
    );
    assert_eq!(texts(&f), vec!["alt eins", "alt zwei"], "nichts ergaenzt");
}

#[test]
fn a_stop_of_a_run_without_stored_audio_keeps_the_version_but_no_resume_marker() {
    let f = fixture();
    run_until(&f, 5);
    let ends = Ends {
        kept: Some(MeetingStatus::Ready),
        resumable: false,
        fallback: Some(MeetingStatus::Ready),
    };
    assert_eq!(
        settle(&f.store, &f.id, false, ends).unwrap(),
        Settled::Kept(None)
    );
    assert_eq!(read(&f.store, &f.id).unwrap(), None);
    assert_eq!(status(&f), "ready");
    assert_eq!(texts(&f), vec!["neu eins"]);
    assert_eq!(variant_count(&f), 2);
}

#[test]
fn a_pending_run_without_any_marker_is_rolled_back() {
    let f = fixture();
    {
        let mut c = f.store.get_connection().unwrap();
        variants::begin_rerun(&mut c, &f.id).unwrap();
    }
    f.store.clear_segments(&f.id).unwrap();
    append(&f.store, &f.id, &[(0, "ohne marke")]);
    assert_eq!(
        settle(&f.store, &f.id, false, Ends::QUIET).unwrap(),
        Settled::RolledBack(None)
    );
    assert_eq!(texts(&f), vec!["alt eins", "alt zwei"]);
}
