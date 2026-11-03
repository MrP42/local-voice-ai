//! Tests fuer mehrere Audiodateien je Besprechung (`parts.rs`): Plan (Zeitachse,
//! Sprecher, Notizen), Audio-Verkettung und die Transaktion im Store.

use super::*;
use crate::managers::meetings::notes::model::NoteBlockKind;
use crate::managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};
use crate::managers::transcription::WordTime;

fn tmp_store() -> (tempfile::TempDir, MeetingStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    (dir, store)
}

fn seg(index: u32, channel: u8, start: u64, end: u64, speaker: Option<u32>) -> StoredSegment {
    StoredSegment {
        segment_index: index,
        text: format!("Satz {index}"),
        start_ms: start,
        end_ms: end,
        channel,
        speaker_index: speaker,
        words: Some(vec![WordTime {
            text: "Satz".into(),
            start_ms: start,
            end_ms: end,
        }]),
    }
}

/// Schreibt `ms` Millisekunden 16-kHz-Mono (Wert `value`) nach `path`.
fn write_wav(path: &Path, ms: u64, value: i16) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..(ms * 16) {
        w.write_sample(value).unwrap();
    }
    w.finalize().unwrap();
}

fn read_wav(path: &Path) -> Vec<i16> {
    hound::WavReader::open(path)
        .unwrap()
        .samples::<i16>()
        .map(|s| s.unwrap())
        .collect()
}

/// Eine fertige importierte Besprechung mit WAV (`ms` lang), Segmenten,
/// Sprecherhinweisen und optional einem Namen fuer Sprecher 1.
fn ready_import(
    store: &MeetingStore,
    dir: &Path,
    file: &str,
    ms: u64,
    value: i16,
    segments: Vec<StoredSegment>,
    name: Option<&str>,
) -> Meeting {
    let m = store
        .create_meeting("Komm.ONE", MeetingSource::Import, Some(1))
        .unwrap();
    store
        .set_source_path(&m.id, &format!("C:/Aufnahmen/{file}"))
        .unwrap();
    let wav = dir.join(format!("{}.wav", m.id));
    write_wav(&wav, ms, value);
    store
        .set_audio_paths(&m.id, wav.to_str(), None, Some(ms))
        .unwrap();
    let turns: Vec<Value> = segments
        .iter()
        .filter_map(|s| {
            s.speaker_index
                .filter(|n| *n > 0)
                .map(|n| serde_json::json!([s.start_ms, s.end_ms, n]))
        })
        .collect();
    store
        .append_delta(
            &m.id,
            &TranscriptDelta {
                new_segments: segments,
            },
        )
        .unwrap();
    store
        .get_connection()
        .unwrap()
        .execute(
            "UPDATE transcripts SET speaker_hints_json = ?1 WHERE meeting_id = ?2",
            params![
                serde_json::json!({"v": 1, "model": "test", "channels": {"2": turns}}).to_string(),
                m.id
            ],
        )
        .unwrap();
    if let Some(name) = name {
        store.set_speaker_name(&m.id, 2, 1, Some(name)).unwrap();
    }
    store.set_status(&m.id, MeetingStatus::Ready).unwrap();
    store.get_meeting(&m.id).unwrap().unwrap()
}

#[test]
fn the_plan_puts_every_part_on_one_continuous_timeline() {
    let (dir, store) = tmp_store();
    let a = ready_import(
        &store,
        dir.path(),
        "Aufnahme 112.m4a",
        10_000,
        1,
        vec![seg(0, 2, 0, 4_000, Some(1)), seg(1, 2, 5_000, 9_000, Some(2))],
        Some("Laura"),
    );
    let b = ready_import(
        &store,
        dir.path(),
        "Aufnahme 113.m4a",
        6_000,
        2,
        vec![seg(0, 2, 1_000, 3_000, Some(1)), seg(1, 2, 3_000, 5_000, None)],
        Some("David"),
    );
    let mut sources = load_sources(&store, &[a.id.clone(), b.id.clone()]).unwrap();
    sources[0].length_ms = 10_000;
    sources[1].length_ms = 6_000;
    let p = plan(&sources);

    assert_eq!(p.duration_ms, 16_000);
    let times: Vec<(u32, u64, u64)> = p
        .segments
        .iter()
        .map(|s| (s.segment_index, s.start_ms, s.end_ms))
        .collect();
    assert_eq!(
        times,
        vec![(0, 0, 4_000), (1, 5_000, 9_000), (2, 11_000, 13_000), (3, 13_000, 15_000)]
    );
    // Wortzeiten wandern mit.
    assert_eq!(p.segments[2].words.as_ref().unwrap()[0].start_ms, 11_000);
    // Sprecher: Teil 2 beginnt hinter der hoechsten Nummer von Teil 1.
    let speakers: Vec<Option<u32>> = p.segments.iter().map(|s| s.speaker_index).collect();
    assert_eq!(speakers, vec![Some(1), Some(2), Some(3), None]);
    let names: Vec<(u32, Option<String>)> = p
        .speakers
        .iter()
        .map(|r| (r.speaker_index, r.display_name.clone()))
        .collect();
    assert_eq!(
        names,
        vec![(1, Some("Laura".into())), (3, Some("David".into()))]
    );
    // Turns verschoben, damit die neue Sprechertrennung die Namen weitertraegt.
    let hints: Value = serde_json::from_str(p.hints.as_deref().unwrap()).unwrap();
    assert_eq!(
        hints["channels"]["2"],
        serde_json::json!([[0, 4000, 1], [5000, 9000, 2], [11000, 13000, 3]])
    );
    assert_eq!(hints["model"], "test");
    // Teilliste: Dateiname und Beginn.
    let parts: Vec<(u32, &str, u64, u64)> = p
        .parts
        .iter()
        .map(|x| (x.part_index, x.title.as_str(), x.offset_ms, x.duration_ms))
        .collect();
    assert_eq!(
        parts,
        vec![
            (0, "Aufnahme 112.m4a", 0, 10_000),
            (1, "Aufnahme 113.m4a", 10_000, 6_000)
        ]
    );
}

#[test]
fn speaker_zero_is_the_whole_channel_and_keeps_the_first_name() {
    let mut a = PartSource {
        meeting: Meeting {
            id: "a".into(),
            title: "A".into(),
            status: "ready".into(),
            source: "import".into(),
            started_at: None,
            ended_at: None,
            language: None,
            mic_audio_path: None,
            system_audio_path: None,
            duration_ms: None,
            consent_confirmed_at: None,
            audio_retention_until: None,
            source_path: None,
            description: None,
            created_at: 0,
            deleted_at: None,
        },
        segments: vec![seg(0, 2, 0, 1_000, Some(0))],
        revision: 1,
        hints: None,
        speakers: vec![SpeakerRow {
            channel: 2,
            speaker_index: 0,
            display_name: None,
            human_id: None,
            consent_state: None,
        }],
        notes: vec![],
        parts: vec![],
        length_ms: 2_000,
    };
    let mut b = a.clone();
    b.meeting.id = "b".into();
    b.speakers[0].display_name = Some("Saal".into());
    a.speakers[0].display_name = None;
    let p = plan(&[a, b]);
    assert_eq!(
        p.segments.iter().map(|s| s.speaker_index).collect::<Vec<_>>(),
        vec![Some(0), Some(0)]
    );
    assert_eq!(p.speakers.len(), 1);
    assert_eq!(p.speakers[0].display_name.as_deref(), Some("Saal"));
}

#[test]
fn notes_keep_their_position_on_the_new_timeline() {
    let block = |id: &str, at: Option<u64>| NoteBlock {
        id: id.into(),
        kind: NoteBlockKind::Paragraph,
        text: id.into(),
        at_ms: at,
        checked: false,
    };
    let (dir, store) = tmp_store();
    let a = ready_import(&store, dir.path(), "a.m4a", 4_000, 1, vec![seg(0, 2, 0, 1_000, None)], None);
    let b = ready_import(&store, dir.path(), "b.m4a", 4_000, 1, vec![seg(0, 2, 0, 1_000, None)], None);
    store.save_notes(&a.id, &[block("n1", Some(1_000))], 0).unwrap();
    store
        .save_notes(&b.id, &[block("n2", Some(500)), block("n3", None)], 0)
        .unwrap();
    let mut sources = load_sources(&store, &[a.id, b.id]).unwrap();
    sources[0].length_ms = 4_000;
    sources[1].length_ms = 4_000;
    let p = plan(&sources);
    let notes: Vec<(&str, Option<u64>)> = p.notes.iter().map(|n| (n.id.as_str(), n.at_ms)).collect();
    assert_eq!(notes, vec![("n1", Some(1_000)), ("n2", Some(4_500)), ("n3", None)]);
}

#[test]
fn audio_is_concatenated_and_a_missing_system_track_is_filled_with_silence() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    let b_sys = dir.path().join("b_sys.wav");
    write_wav(&a, 100, 7);
    write_wav(&b, 50, 9);
    write_wav(&b_sys, 80, 3);
    let out = concat_audio(
        &[(a.clone(), None), (b.clone(), Some(b_sys.clone()))],
        &dir.path().join("ziel"),
    )
    .unwrap();
    assert_eq!(out.lengths_ms, vec![100, 80]);
    let mic = read_wav(&out.mic);
    assert_eq!(mic.len(), 180 * 16);
    assert!(mic[..1600].iter().all(|s| *s == 7));
    assert!(mic[1600..2400].iter().all(|s| *s == 9));
    // Teil 2 ist auf der Mikrofonspur kuerzer als auf der Systemspur: Stille.
    assert!(mic[2400..].iter().all(|s| *s == 0));
    let sys = read_wav(out.system.as_ref().unwrap());
    assert_eq!(sys.len(), 180 * 16);
    assert!(sys[..1600].iter().all(|s| *s == 0));
    assert!(sys[1600..].iter().all(|s| *s == 3));
}

#[test]
fn other_sample_rates_are_converted_to_16_khz() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a48.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&a, spec).unwrap();
    for _ in 0..48_000 {
        w.write_sample(100i16).unwrap();
        w.write_sample(100i16).unwrap();
    }
    w.finalize().unwrap();
    let b = dir.path().join("b.wav");
    write_wav(&b, 500, 5);
    let out = concat_audio(&[(a, None), (b, None)], dir.path()).unwrap();
    assert_eq!(out.lengths_ms, vec![1_000, 500]);
    assert_eq!(read_wav(&out.mic).len(), 1_500 * 16);
}

#[test]
fn merging_writes_one_meeting_and_removes_the_others_in_one_go() {
    let (dir, store) = tmp_store();
    let a = ready_import(
        &store,
        dir.path(),
        "Aufnahme 112.m4a",
        2_000,
        1,
        vec![seg(0, 2, 0, 1_000, Some(1))],
        Some("Laura"),
    );
    let b = ready_import(
        &store,
        dir.path(),
        "Aufnahme 113.m4a",
        1_000,
        2,
        vec![seg(0, 2, 0, 500, Some(1))],
        None,
    );
    let folder = store.folder_save(None, "Komm.ONE", None).unwrap();
    store.set_meeting_folders(&b.id, &[folder.id.clone()]).unwrap();
    let old_a_wav = a.mic_audio_path.clone().unwrap();
    let old_b_wav = b.mic_audio_path.clone().unwrap();

    let target = merge_into_first(&store, &[a.id.clone(), b.id.clone()], &dir.path().join(&a.id)).unwrap();
    assert_eq!(target, a.id);

    let merged = store.get_meeting(&a.id).unwrap().unwrap();
    assert_eq!(merged.duration_ms, Some(3_000));
    assert_eq!(merged.status, "ready");
    let wav = merged.mic_audio_path.clone().unwrap();
    assert_ne!(wav, old_a_wav);
    assert_eq!(read_wav(Path::new(&wav)).len(), 3_000 * 16);
    // Alte Dateien sind weg, die angehaengte Besprechung ist geloescht.
    assert!(!Path::new(&old_a_wav).exists());
    assert!(!Path::new(&old_b_wav).exists());
    assert!(store.get_meeting(&b.id).unwrap().is_none());

    let segs = store.get_segments(&a.id).unwrap();
    assert_eq!(
        segs.iter().map(|s| (s.start_ms, s.speaker_index)).collect::<Vec<_>>(),
        vec![(0, Some(1)), (2_000, Some(2))]
    );
    let rows = store.speaker_rows(&a.id).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].display_name.as_deref(), Some("Laura"));
    // Projekt des zweiten Teils geht auf die Besprechung ueber.
    assert_eq!(store.meeting_folder_ids(&a.id).unwrap(), vec![folder.id]);
    let parts = store.meeting_parts(&a.id).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[1].offset_ms, 2_000);
    assert_eq!(parts[1].title, "Aufnahme 113.m4a");
    // Epoche gestiegen: KI-Notizen erkennen veraltete Belege.
    assert!(store.segment_epoch(&a.id).unwrap() >= 1);
}

#[test]
fn a_meeting_merged_before_keeps_its_parts_when_more_are_appended() {
    let (dir, store) = tmp_store();
    let a = ready_import(&store, dir.path(), "1.m4a", 1_000, 1, vec![seg(0, 2, 0, 500, None)], None);
    let b = ready_import(&store, dir.path(), "2.m4a", 1_000, 1, vec![seg(0, 2, 0, 500, None)], None);
    let c = ready_import(&store, dir.path(), "3.m4a", 1_000, 1, vec![seg(0, 2, 0, 500, None)], None);
    merge_into_first(&store, &[a.id.clone(), b.id.clone()], &dir.path().join(&a.id)).unwrap();
    merge_into_first(&store, &[a.id.clone(), c.id.clone()], &dir.path().join(&a.id)).unwrap();
    let parts = store.meeting_parts(&a.id).unwrap();
    assert_eq!(
        parts
            .iter()
            .map(|p| (p.part_index, p.title.as_str(), p.offset_ms))
            .collect::<Vec<_>>(),
        vec![(0, "1.m4a", 0), (1, "2.m4a", 1_000), (2, "3.m4a", 2_000)]
    );
    assert_eq!(
        store
            .get_segments(&a.id)
            .unwrap()
            .iter()
            .map(|s| s.start_ms)
            .collect::<Vec<_>>(),
        vec![0, 1_000, 2_000]
    );
}

#[test]
fn a_part_that_is_not_ready_blocks_the_merge_and_changes_nothing() {
    let (dir, store) = tmp_store();
    let a = ready_import(&store, dir.path(), "a.m4a", 1_000, 1, vec![seg(0, 2, 0, 500, None)], None);
    let b = ready_import(&store, dir.path(), "b.m4a", 1_000, 1, vec![seg(0, 2, 0, 500, None)], None);
    store.set_status(&b.id, MeetingStatus::Processing).unwrap();
    let err = merge_into_first(&store, &[a.id.clone(), b.id.clone()], &dir.path().join(&a.id)).unwrap_err();
    assert_eq!(err, PartsError::NotReady(b.id.clone()));
    assert_eq!(store.get_meeting(&a.id).unwrap().unwrap().mic_audio_path, a.mic_audio_path);
    assert!(store.meeting_parts(&a.id).unwrap().is_empty());
    // Keine neue Datei im Zielordner.
    assert!(!dir.path().join(&a.id).exists());
}

#[test]
fn a_change_to_a_transcript_during_the_merge_rolls_everything_back() {
    let (dir, store) = tmp_store();
    let a = ready_import(&store, dir.path(), "a.m4a", 1_000, 1, vec![seg(0, 2, 0, 500, None)], None);
    let b = ready_import(&store, dir.path(), "b.m4a", 1_000, 1, vec![seg(0, 2, 0, 500, None)], None);
    let mut sources = load_sources(&store, &[a.id.clone(), b.id.clone()]).unwrap();
    for s in &mut sources {
        s.length_ms = 1_000;
    }
    let p = plan(&sources);
    // Eine Korrektur von Hand nach dem Lesen.
    store.update_segment_text(&b.id, 0, "korrigiert").unwrap();
    let revisions: Vec<(String, i64)> = sources.iter().map(|s| (s.meeting.id.clone(), s.revision)).collect();
    let err = store
        .apply_merge(&MergeWrite {
            target: &a.id,
            members: &[b.id.clone()],
            revisions: &revisions,
            plan: &p,
            mic: "x.wav",
            system: None,
        })
        .unwrap_err();
    assert_eq!(err, PartsError::Changed);
    assert!(store.get_meeting(&b.id).unwrap().unwrap().deleted_at.is_none());
    assert_eq!(store.get_meeting(&a.id).unwrap().unwrap().mic_audio_path, a.mic_audio_path);
}

#[test]
fn subtitles_youtube_and_mixed_sources_are_refused() {
    let (dir, store) = tmp_store();
    let a = ready_import(&store, dir.path(), "a.m4a", 1_000, 1, vec![seg(0, 2, 0, 500, None)], None);
    let sub = store
        .create_meeting("Untertitel", MeetingSource::Subtitle, None)
        .unwrap();
    store.set_status(&sub.id, MeetingStatus::Ready).unwrap();
    assert_eq!(
        load_sources(&store, &[a.id.clone(), sub.id.clone()]).unwrap_err(),
        PartsError::NoAudio(sub.id.clone())
    );
    let live = store.create_meeting("Live", MeetingSource::Live, Some(1)).unwrap();
    let wav = dir.path().join("live.wav");
    write_wav(&wav, 500, 1);
    store.set_audio_paths(&live.id, wav.to_str(), None, Some(500)).unwrap();
    store.set_status(&live.id, MeetingStatus::Ready).unwrap();
    assert_eq!(
        load_sources(&store, &[a.id.clone(), live.id.clone()]).unwrap_err(),
        PartsError::MixedSources
    );
    assert_eq!(load_sources(&store, &[a.id.clone()]).unwrap_err(), PartsError::TooFew);
    assert_eq!(
        load_sources(&store, &[a.id.clone(), a.id.clone()]).unwrap_err(),
        PartsError::TooFew
    );
}

#[test]
fn a_missing_audio_file_is_refused() {
    let (dir, store) = tmp_store();
    let a = ready_import(&store, dir.path(), "a.m4a", 1_000, 1, vec![], None);
    let b = ready_import(&store, dir.path(), "b.m4a", 1_000, 1, vec![], None);
    std::fs::remove_file(b.mic_audio_path.as_ref().unwrap()).unwrap();
    assert_eq!(
        load_sources(&store, &[a.id, b.id.clone()]).unwrap_err(),
        PartsError::AudioMissing(b.id)
    );
}

#[test]
fn pending_parts_are_listed_in_order_and_go_with_a_deleted_target() {
    let (dir, store) = tmp_store();
    let a = ready_import(&store, dir.path(), "a.m4a", 1_000, 1, vec![], None);
    let b = store.queue_enqueue("b", "C:/b.m4a", Some(1)).unwrap();
    let c = store.queue_enqueue("c", "C:/c.m4a", Some(1)).unwrap();
    assert_eq!(store.append_pending_add(&b.id, &a.id).unwrap(), 1);
    assert_eq!(store.append_pending_add(&c.id, &a.id).unwrap(), 2);
    let pending = store.append_pending_of(&a.id).unwrap();
    assert_eq!(
        pending.iter().map(|p| (p.meeting_id.as_str(), p.status.as_str())).collect::<Vec<_>>(),
        vec![(b.id.as_str(), "queued"), (c.id.as_str(), "queued")]
    );
    assert_eq!(store.append_target_of(&b.id).unwrap().as_deref(), Some(a.id.as_str()));
    let info = parts_info(&store, &b.id).unwrap();
    assert_eq!(info.append_to, Some((a.id.clone(), "Komm.ONE".to_string())));

    assert!(store.append_pending_remove(&c.id).unwrap());
    assert_eq!(store.append_pending_of(&a.id).unwrap().len(), 1);

    store.soft_delete_meeting(&a.id).unwrap();
    assert_eq!(store.append_target_of(&b.id).unwrap(), None);
}

#[test]
fn the_manual_order_follows_the_start_time_then_the_file_number() {
    let m = |id: &str, started: i64, file: &str| Meeting {
        id: id.into(),
        title: "Komm.ONE".into(),
        status: "ready".into(),
        source: "import".into(),
        started_at: Some(started),
        ended_at: None,
        language: None,
        mic_audio_path: None,
        system_audio_path: None,
        duration_ms: None,
        consent_confirmed_at: None,
        audio_retention_until: None,
        source_path: Some(format!("C:/x/{file}")),
        description: None,
        created_at: started,
        deleted_at: None,
    };
    let mut list = vec![
        m("d", 200, "Aufnahme 115.m4a"),
        m("b", 100, "Aufnahme 10.m4a"),
        m("a", 100, "Aufnahme 9.m4a"),
        m("c", 150, "Aufnahme 1.m4a"),
    ];
    chronological(&mut list);
    assert_eq!(
        list.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b", "c", "d"]
    );
}
