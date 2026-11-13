use super::*;
use crate::managers::meetings::diarize::Turn;
use crate::managers::meetings::speakers::{hints_with_turns, HINTS_MODEL_NAME};
use crate::managers::meetings::store::{
    MeetingSource, MeetingStatus, SpeakerWrite, StoredSegment, TranscriptDelta,
};
use std::cell::Cell;
use std::rc::Rc;

/// Der Zwischenspeicher ist prozessweit: Tests, die ihn nutzen, laufen nacheinander.
static SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    _dir: tempfile::TempDir,
    store: MeetingStore,
    id: String,
}

/// Gegenseite mit drei Sprechern zu je 12 s. Sprecher 1 und 3 haben dieselbe
/// „Stimme“ (Abtastwert +0,1), Sprecher 2 eine andere (-0,2).
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    let m = store
        .create_meeting("t", MeetingSource::Live, Some(0))
        .unwrap();
    let wav = dir.path().join("system.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&wav, spec).unwrap();
    for (value, secs) in [(3_277i16, 12), (-6_554, 12), (3_277, 12)] {
        for _ in 0..16_000 * secs {
            w.write_sample(value).unwrap();
        }
    }
    w.finalize().unwrap();
    store
        .set_audio_paths(&m.id, None, Some(wav.to_str().unwrap()), Some(36_000))
        .unwrap();
    store.set_status(&m.id, MeetingStatus::Processing).unwrap();
    let segments: Vec<StoredSegment> = (0..3)
        .map(|i| StoredSegment {
            segment_index: i,
            text: format!("Text {i}"),
            start_ms: u64::from(i) * 12_000,
            end_ms: u64::from(i + 1) * 12_000,
            channel: 1,
            speaker_index: Some(i + 1),
            words: None,
        })
        .collect();
    store
        .append_delta(
            &m.id,
            &TranscriptDelta {
                new_segments: segments.clone(),
            },
        )
        .unwrap();
    let turns: Vec<Turn> = (0..3)
        .map(|i| Turn {
            start_ms: u64::from(i) * 12_000,
            end_ms: u64::from(i + 1) * 12_000,
            speaker: i + 1,
        })
        .collect();
    let write = SpeakerWrite {
        hints_json: hints_with_turns(
            None,
            HINTS_MODEL_NAME,
            1_000,
            100,
            &BTreeMap::from([(1u8, turns)]),
        ),
        present: vec![(1, 1), (1, 2), (1, 3)],
        fresh: BTreeMap::new(),
    };
    let snapshot = store.transcript_snapshot(&m.id).unwrap();
    store
        .update_segments_with_speakers(&m.id, &segments, snapshot.revision, false, &write)
        .unwrap();
    Fixture {
        _dir: dir,
        store,
        id: m.id,
    }
}

/// Attrappe des Modells: zählt Ladevorgänge und Abdrücke.
fn run(f: &Fixture, loads: &Rc<Cell<u32>>, embeds: &Rc<Cell<u32>>) -> VoiceMatches {
    let (loads, embeds) = (Rc::clone(loads), Rc::clone(embeds));
    let mut load = move || -> Result<EmbedFn, String> {
        loads.set(loads.get() + 1);
        let embeds = Rc::clone(&embeds);
        Ok(Box::new(
            move |frei: &crate::managers::planning::Freigabe, pcm: &[f32]| {
                frei.einloesen()?;
                embeds.set(embeds.get() + 1);
                Ok(if pcm[0] > 0.0 {
                    vec![1.0, 0.1]
                } else {
                    vec![0.1, 1.0]
                })
            },
        ))
    };
    voice_matches(&f.store, &f.id, &mut load).unwrap()
}

#[test]
fn similar_speakers_of_a_channel_are_suggested_once_and_cached() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    forget_all();
    let f = fixture();
    let (loads, embeds) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let r = run(&f, &loads, &embeds);
    assert_eq!(r.state, "ready");
    assert_eq!(
        r.matches,
        vec![VoiceMatch {
            channel: 1,
            a: 1,
            b: 3,
            score: 100
        }]
    );
    assert_eq!(loads.get(), 1);
    let first = embeds.get();
    assert!(first >= 3, "{first}");
    // Zweiter Aufruf: alles aus dem Zwischenspeicher, kein Modell.
    let again = run(&f, &loads, &embeds);
    assert_eq!(again, r);
    assert_eq!((loads.get(), embeds.get()), (1, first));
    // Ausschalten vergisst die Abdrücke.
    forget_all();
    run(&f, &loads, &embeds);
    assert_eq!(loads.get(), 2);
}

#[test]
fn a_dismissed_pair_stays_hidden_in_this_epoch_only() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    forget_all();
    let f = fixture();
    let (loads, embeds) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    dismiss(&f.store, &f.id, 1, 3, 1).unwrap();
    dismiss(&f.store, &f.id, 1, 1, 3).unwrap(); // doppelt: kein zweiter Eintrag
    let list = f.store.metadata_json(&f.id).unwrap().unwrap()[DISMISSED_KEY].clone();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!(run(&f, &loads, &embeds).matches.is_empty());
    // Eintrag einer anderen Epoche gilt nicht.
    let epoch = f.store.segment_epoch(&f.id).unwrap();
    f.store
        .set_metadata_key(
            &f.id,
            DISMISSED_KEY,
            json!([{ "channel": 1, "a": 1, "b": 3, "epoch": epoch + 1 }]),
        )
        .unwrap();
    assert_eq!(run(&f, &loads, &embeds).matches.len(), 1);
    assert_eq!(
        dismiss(&f.store, &f.id, 1, 0, 3),
        Err("speaker_invalid".into())
    );
    assert_eq!(
        dismiss(&f.store, &f.id, 1, 2, 2),
        Err("speaker_invalid".into())
    );
}

#[test]
fn two_named_speakers_are_not_suggested_one_named_is() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    forget_all();
    let f = fixture();
    let (loads, embeds) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    f.store
        .set_speaker_name(&f.id, 1, 1, Some("Anna Beispiel"))
        .unwrap();
    assert_eq!(run(&f, &loads, &embeds).matches.len(), 1, "einer benannt");
    f.store
        .set_speaker_name(&f.id, 1, 3, Some("Ben Beispiel"))
        .unwrap();
    assert!(run(&f, &loads, &embeds).matches.is_empty(), "beide benannt");
}

#[test]
fn without_turns_or_audio_there_is_nothing_to_compare() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    forget_all();
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    let m = store
        .create_meeting("t", MeetingSource::Live, Some(0))
        .unwrap();
    let mut load = || -> Result<EmbedFn, String> { Err("darf nicht laden".into()) };
    assert_eq!(
        voice_matches(&store, &m.id, &mut load).unwrap().state,
        "no_audio"
    );
    let f = fixture();
    std::fs::remove_file(f._dir.path().join("system.wav")).unwrap();
    assert_eq!(
        voice_matches(&f.store, &f.id, &mut load).unwrap().state,
        "no_audio"
    );
    assert_eq!(
        voice_matches(&f.store, "fehlt", &mut load),
        Err("meeting_not_found".into())
    );
}

// ---- lva#544, Fixrunde 5: der Stimmvergleich steht hinter dem Einwilligungs-Tor ---------------

/// Ohne Zustimmung: kein Modell, keine Vorschlaege (Zustand `no_consent`).
#[test]
fn without_consent_no_voiceprint_model_is_loaded() {
    use crate::managers::planning::testhilfe as th;
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    forget_all();
    let f = fixture();
    let _ = th::plan_fuer_besprechung(&f.store, &f.id, "u-v1", false);
    let (loads, embeds) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let r = run(&f, &loads, &embeds);
    assert_eq!(r.state, "no_consent");
    assert!(r.matches.is_empty());
    assert_eq!((loads.get(), embeds.get()), (0, 0));
}

/// Widerruf WAEHREND des ersten Abdrucks: der Abdruck wird verworfen, es folgt kein weiterer,
/// nichts landet im Zwischenspeicher (nach erneuter Zustimmung wird alles neu gerechnet).
#[test]
fn a_revocation_during_the_first_voiceprint_stops_the_comparison() {
    use crate::managers::planning::testhilfe as th;
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    forget_all();
    let f = fixture();
    let (p, _) = th::plan_fuer_besprechung(&f.store, &f.id, "u-v2", true);
    let embeds = Rc::new(Cell::new(0u32));
    let zaehler = Rc::clone(&embeds);
    let db = f._dir.path().join("meetings.db");
    let plan_id = p.id.clone();
    let mut load = move || -> Result<EmbedFn, String> {
        let (zaehler, db, plan_id) = (Rc::clone(&zaehler), db.clone(), plan_id.clone());
        Ok(Box::new(
            move |frei: &crate::managers::planning::Freigabe, _pcm: &[f32]| {
                frei.einloesen()?;
                zaehler.set(zaehler.get() + 1);
                let andere = MeetingStore::open_at(&db).unwrap();
                crate::managers::planning::einwilligung_setzen(
                    &andere,
                    &th::widerruf(&plan_id, th::BERT),
                    th::NOW + 5,
                )
                .unwrap();
                Ok(vec![1.0, 0.1])
            },
        ))
    };
    let r = voice_matches(&f.store, &f.id, &mut load).unwrap();
    assert_eq!(r.state, "no_consent");
    assert!(r.matches.is_empty());
    assert_eq!(embeds.get(), 1, "kein zweiter Abdruck");

    // Wieder zugestimmt: nichts stand im Zwischenspeicher, das Modell rechnet alles neu.
    crate::managers::planning::einwilligung_setzen(
        &f.store,
        &th::zustimmung(&p.id, th::BERT),
        th::NOW + 6,
    )
    .unwrap();
    let (loads, neu) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let r = run(&f, &loads, &neu);
    assert_eq!(r.state, "ready");
    assert_eq!(loads.get(), 1);
    assert!(neu.get() >= 3, "{}", neu.get());
}
