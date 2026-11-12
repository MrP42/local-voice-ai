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
        let settled = settle(&f.store, &f.id, false).unwrap();
        assert_eq!(
            settled,
            Settled::RolledBack(Some(MeetingStatus::Ready)),
            "Schritt {upto}"
        );
        assert_eq!(texts(&f), vec!["alt eins", "alt zwei"], "Schritt {upto}");
        assert_eq!(read(&f.store, &f.id).unwrap(), None, "Schritt {upto}");
        assert_eq!(variant_count(&f), 1, "Schritt {upto}");
        assert_eq!(prepare_resume(&f.store, &f.id).unwrap(), None);
    }
}

#[test]
fn a_stop_or_crash_after_new_sections_keeps_them_and_the_old_version() {
    let f = fixture();
    run_until(&f, 5);
    let Settled::Kept(Some(r)) = settle(&f.store, &f.id, false).unwrap() else {
        panic!("behalten erwartet");
    };
    assert_eq!(r.model, "neu-modell");
    assert_eq!(texts(&f), vec!["neu eins"]);
    assert_eq!(variant_count(&f), 2);
    // Doppelter Abschluss (doppelter Stopp, danach Wiederherstellung): unveraendert.
    assert_eq!(
        settle(&f.store, &f.id, false).unwrap(),
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
        settle(&f.store, &f.id, false).unwrap(),
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
    let mut m = read(&f.store, &f.id).unwrap().unwrap();
    m.phase = PHASE_ROLLBACK.to_string();
    write_marker(&f.store, &f.id, &m).unwrap(); // Absturz direkt danach
    assert_eq!(
        settle(&f.store, &f.id, false).unwrap(),
        Settled::RolledBack(Some(MeetingStatus::Ready))
    );
    assert_eq!(texts(&f), vec!["alt eins", "alt zwei"]);
    assert_eq!(variant_count(&f), 1);

    let g = fixture();
    run_until(&g, 5);
    assert_eq!(
        settle(&g.store, &g.id, true).unwrap(),
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
    let Settled::Kept(Some(r)) = settle(&f.store, &f.id, false).unwrap() else {
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
        prepare_resume(&f.store, &f.id).unwrap().as_deref(),
        Some("neu-modell")
    );
    assert_eq!(texts(&f), vec!["neu eins"]);
}

#[test]
fn a_new_run_over_a_kept_result_restores_it_when_it_leaves_nothing() {
    let f = fixture();
    run_until(&f, 5);
    settle(&f.store, &f.id, false).unwrap();
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
        settle(&f.store, &f.id, false).unwrap(),
        Settled::RolledBack(Some(MeetingStatus::Cancelled))
    );
    assert_eq!(texts(&f), vec!["neu eins"], "das erste Teilergebnis bleibt");
    assert_eq!(
        prepare_resume(&f.store, &f.id).unwrap().as_deref(),
        Some("neu-modell"),
        "und bleibt mit seinem Modell fortsetzbar"
    );
}

#[test]
fn a_finished_run_whose_marker_was_not_cleared_is_just_finished() {
    let f = fixture();
    run_until(&f, 5);
    {
        let mut c = f.store.get_connection().unwrap();
        variants::finish_rerun(&mut c, &f.id, variants::KIND_RETRANSCRIBED, None).unwrap();
    }
    assert_eq!(settle(&f.store, &f.id, false).unwrap(), Settled::Finished);
    assert_eq!(read(&f.store, &f.id).unwrap(), None);
    assert_eq!(texts(&f), vec!["neu eins"]);
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
        settle(&f.store, &f.id, false).unwrap(),
        Settled::RolledBack(None)
    );
    assert_eq!(texts(&f), vec!["alt eins", "alt zwei"]);
}
