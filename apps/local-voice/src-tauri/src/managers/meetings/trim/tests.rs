//! Tests fuer Bereiche einer Aufnahme (`trim.rs`): Zeitfunktionen, WAV-Schnitt und
//! die Transaktion (Klemmen, Versatz, Woerter, kuerzere und leere Spur, Fehlerpfad
//! ohne Aenderung).

use super::*;
use crate::managers::meetings::notes::model::NoteBlockKind;
use crate::managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};
use crate::managers::transcription::WordTime;

fn range(start_ms: u64, end_ms: u64) -> TimeRange {
    TimeRange { start_ms, end_ms }
}

fn tmp_store() -> (tempfile::TempDir, MeetingStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    (dir, store)
}

fn word(text: &str, start_ms: u64, end_ms: u64) -> WordTime {
    WordTime {
        text: text.into(),
        start_ms,
        end_ms,
    }
}

fn seg(index: u32, start: u64, end: u64) -> StoredSegment {
    StoredSegment {
        segment_index: index,
        text: format!("Satz {index}"),
        start_ms: start,
        end_ms: end,
        channel: 0,
        speaker_index: Some(1),
        words: Some(vec![word("Satz", start, end)]),
    }
}

/// 16-kHz-Mono, Wert je Sample = die Millisekunde, in der es liegt (so sieht man
/// im Ergebnis, wo der Schnitt war).
fn write_wav(path: &Path, ms: u64) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..ms * 16 {
        w.write_sample((i / 16) as i16).unwrap();
    }
    w.finalize().unwrap();
}

fn read_i16(path: &Path) -> Vec<i16> {
    hound::WavReader::open(path)
        .unwrap()
        .samples::<i16>()
        .map(|s| s.unwrap())
        .collect()
}

// ---------------------------------------------------------------------------
// Bereich
// ---------------------------------------------------------------------------

#[test]
fn a_range_needs_a_start_before_its_end() {
    assert_eq!(range(1_000, 2_000).validate(), Ok(range(1_000, 2_000)));
    assert_eq!(range(2_000, 2_000).validate(), Err("invalid_range".into()));
    assert_eq!(range(3_000, 2_000).validate(), Err("invalid_range".into()));
    assert_eq!(range(0, 0).validate(), Err("invalid_range".into()));
}

#[test]
fn provenance_params_carry_the_range_only_when_there_is_one() {
    let base = serde_json::json!({ "audio_ms": 5 });
    assert_eq!(params_with_range(base.clone(), None), base);
    let with = params_with_range(base, Some(range(1_000, 4_000)));
    assert_eq!(with["range"]["start_ms"], 1_000);
    assert_eq!(with["range"]["end_ms"], 4_000);
    assert_eq!(with["audio_ms"], 5);
}

// ---------------------------------------------------------------------------
// Samples
// ---------------------------------------------------------------------------

#[test]
fn a_track_is_cut_to_the_range_with_16_samples_per_millisecond() {
    let mut samples: Vec<i16> = (0..10_000 * 16).map(|i| (i / 16) as i16).collect();
    cut_in_place(&mut samples, range(2_000, 5_000));
    assert_eq!(samples.len(), 3_000 * 16);
    assert_eq!(samples[0], 2_000);
    assert_eq!(*samples.last().unwrap(), 4_999);
}

#[test]
fn a_shorter_track_is_clamped_and_one_ending_before_the_start_is_empty() {
    let mut short: Vec<i16> = vec![1; 3_000 * 16];
    cut_in_place(&mut short, range(2_000, 9_000));
    assert_eq!(short.len(), 1_000 * 16);

    let mut before: Vec<i16> = vec![1; 1_000 * 16];
    cut_in_place(&mut before, range(2_000, 9_000));
    assert!(before.is_empty());
}

#[test]
fn reading_a_range_equals_reading_everything_and_cutting() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("a.wav");
    write_wav(&wav, 10_000);
    let part = read_track_range(&wav, range(2_000, 5_000)).unwrap();
    let mut all = read_i16(&wav);
    cut_in_place(&mut all, range(2_000, 5_000));
    assert_eq!(part, all);
    // Ende hinter der Datei: geklemmt; Anfang dahinter: leer.
    assert_eq!(
        read_track_range(&wav, range(9_000, 99_000)).unwrap().len(),
        1_000 * 16
    );
    assert!(read_track_range(&wav, range(20_000, 30_000))
        .unwrap()
        .is_empty());
}

#[test]
fn a_track_in_another_format_is_converted_before_it_is_cut() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("stereo48.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&wav, spec).unwrap();
    for _ in 0..4_000 * 48 {
        w.write_sample(1_000i16).unwrap();
        w.write_sample(1_000i16).unwrap();
    }
    w.finalize().unwrap();
    let part = read_track_range(&wav, range(1_000, 2_000)).unwrap();
    assert_eq!(part.len(), 1_000 * 16, "16 kHz mono, eine Sekunde");
}

// ---------------------------------------------------------------------------
// Segmente, Turns, Notizen, Teile
// ---------------------------------------------------------------------------

#[test]
fn segments_overlapping_the_range_stay_clamped_and_shifted() {
    let segments = vec![
        seg(0, 0, 1_000),     // davor
        seg(1, 1_500, 2_500), // ragt hinein
        seg(2, 3_000, 4_000), // mittendrin
        seg(3, 4_500, 6_000), // ragt hinaus
        seg(4, 7_000, 8_000), // danach
        seg(5, 1_000, 9_000), // umschliesst den Bereich
        seg(6, 5_000, 5_000), // leer, genau am Ende: draussen
        seg(7, 2_000, 2_000), // leer, genau am Anfang: drin
    ];
    let (out, map) = trim_segments(segments, range(2_000, 5_000));
    let times: Vec<(u32, u64, u64)> = out
        .iter()
        .map(|s| (s.segment_index, s.start_ms, s.end_ms))
        .collect();
    assert_eq!(
        times,
        vec![
            (0, 0, 500), // war 1500..2500
            (1, 1_000, 2_000),
            (2, 2_500, 3_000),
            (3, 0, 3_000), // war 1000..9000
            (4, 0, 0),     // war 2000..2000
        ]
    );
    // Alte Indizes zeigen auf die neuen; die verworfenen fehlen.
    assert_eq!(map.get(&1), Some(&0));
    assert_eq!(map.get(&2), Some(&1));
    assert_eq!(map.get(&3), Some(&2));
    assert_eq!(map.get(&5), Some(&3));
    assert_eq!(map.get(&7), Some(&4));
    assert!(!map.contains_key(&0) && !map.contains_key(&4) && !map.contains_key(&6));
}

#[test]
fn words_outside_the_range_are_dropped_and_the_rest_moves_with_the_segment() {
    let mut s = seg(0, 1_000, 6_000);
    s.words = Some(vec![
        word("a", 1_000, 1_800),
        word("b", 1_900, 2_600),
        word("c", 3_000, 3_500),
        word("d", 4_800, 5_500),
        word("e", 5_600, 6_000),
    ]);
    let (out, _) = trim_segments(vec![s], range(2_000, 5_000));
    let words = out[0].words.as_ref().unwrap();
    let got: Vec<(&str, u64, u64)> = words
        .iter()
        .map(|w| (w.text.as_str(), w.start_ms, w.end_ms))
        .collect();
    assert_eq!(
        got,
        vec![("b", 0, 600), ("c", 1_000, 1_500), ("d", 2_800, 3_000)]
    );
    assert_eq!((out[0].start_ms, out[0].end_ms), (0, 3_000));
}

#[test]
fn a_segment_without_a_word_in_the_range_keeps_its_text_but_no_word_times() {
    let mut s = seg(0, 1_000, 3_000);
    s.words = Some(vec![word("a", 1_000, 1_900)]);
    let (out, _) = trim_segments(vec![s], range(2_000, 5_000));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].text, "Satz 0");
    assert!(out[0].words.is_none());
}

#[test]
fn turns_are_filtered_clamped_and_shifted_and_everything_else_stays() {
    let hints = serde_json::json!({
        "v": 1, "model": "m",
        "channels": {
            "0": [[0, 1_000, 1], [1_500, 3_000, 2], [4_000, 8_000, 1], [6_000, 7_000, 3]],
            "1": [[2_000, 3_000, 1]]
        }
    })
    .to_string();
    let out: Value = serde_json::from_str(&trim_hints(&hints, range(2_000, 5_000))).unwrap();
    assert_eq!(out["v"], 1);
    assert_eq!(out["model"], "m");
    assert_eq!(
        out["channels"]["0"],
        serde_json::json!([[0, 1_000, 2], [2_000, 3_000, 1]])
    );
    assert_eq!(out["channels"]["1"], serde_json::json!([[0, 1_000, 1]]));
    // Unlesbar: bleibt, wie es ist.
    assert_eq!(trim_hints("kein json", range(1, 2)), "kein json");
}

fn note(id: &str, at_ms: Option<u64>) -> NoteBlock {
    NoteBlock {
        id: id.into(),
        kind: NoteBlockKind::Bullet,
        text: id.into(),
        at_ms,
        checked: false,
    }
}

#[test]
fn notes_keep_their_text_and_lose_a_position_outside_the_range() {
    let out = trim_notes(
        vec![
            note("davor", Some(1_000)),
            note("drin", Some(3_500)),
            note("anfang", Some(2_000)),
            note("ende", Some(5_000)),
            note("ohne", None),
        ],
        range(2_000, 5_000),
    );
    let got: Vec<(&str, Option<u64>)> = out.iter().map(|b| (b.id.as_str(), b.at_ms)).collect();
    assert_eq!(
        got,
        vec![
            ("davor", None),
            ("drin", Some(1_500)),
            ("anfang", Some(0)),
            ("ende", None),
            ("ohne", None)
        ]
    );
}

fn part(index: u32, offset: u64, duration: u64) -> MeetingPart {
    MeetingPart {
        part_index: index,
        title: format!("Teil {index}"),
        source_path: None,
        offset_ms: offset,
        duration_ms: duration,
    }
}

#[test]
fn parts_move_clamp_and_drop_and_a_single_remaining_part_needs_no_list() {
    let parts = vec![
        part(0, 0, 4_000),
        part(1, 4_000, 4_000),
        part(2, 8_000, 4_000),
    ];
    let out = trim_parts(parts.clone(), range(2_000, 9_000));
    let got: Vec<(u32, u64, u64)> = out
        .iter()
        .map(|p| (p.part_index, p.offset_ms, p.duration_ms))
        .collect();
    assert_eq!(
        got,
        vec![(0, 0, 2_000), (1, 2_000, 4_000), (2, 6_000, 1_000)]
    );
    // Nur ein Teil uebrig: keine Teilliste mehr.
    assert!(trim_parts(parts.clone(), range(4_500, 7_000)).is_empty());
    // Teil 0 faellt weg, die anderen ruecken nach vorn (Index neu).
    let out = trim_parts(parts, range(4_000, 12_000));
    assert_eq!(out[0].part_index, 0);
    assert_eq!(out[0].offset_ms, 0);
    assert_eq!(out[1].offset_ms, 4_000);
}

#[test]
fn slide_occurrences_follow_the_range_and_flagged_sentences_follow_their_index() {
    let kept = trim_occurrences(
        vec![
            SlideOccurrence {
                start_ms: 0,
                end_ms: 1_000,
            },
            SlideOccurrence {
                start_ms: 1_500,
                end_ms: 3_000,
            },
        ],
        range(2_000, 5_000),
    );
    assert_eq!(
        kept,
        vec![SlideOccurrence {
            start_ms: 0,
            end_ms: 1_000
        }]
    );

    let map: HashMap<u32, u32> = [(3, 0), (5, 1)].into_iter().collect();
    let meta = serde_json::json!({
        "checked": 9,
        "flagged": [
            { "segment_index": 3, "reasons": ["numbers"] },
            { "segment_index": 4, "reasons": ["empty"] },
            { "segment_index": 5, "reasons": ["length"] }
        ]
    })
    .to_string();
    let out: Value = serde_json::from_str(&remap_flagged(&meta, &map)).unwrap();
    assert_eq!(out["checked"], 9);
    assert_eq!(
        out["flagged"],
        serde_json::json!([
            { "segment_index": 0, "reasons": ["numbers"] },
            { "segment_index": 1, "reasons": ["length"] }
        ])
    );
}

// ---------------------------------------------------------------------------
// WAV
// ---------------------------------------------------------------------------

#[test]
fn a_wav_is_cut_in_the_format_and_the_sample_rate_of_the_file() {
    let dir = tempfile::tempdir().unwrap();
    // 48 kHz, Stereo, 16 Bit: der Abschnitt wird in 48-kHz-Frames gerechnet.
    let src = dir.path().join("in.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&src, spec).unwrap();
    for frame in 0..4_000u64 * 48 {
        let ms = (frame / 48) as i16;
        w.write_sample(ms).unwrap();
        w.write_sample(-ms).unwrap();
    }
    w.finalize().unwrap();

    let dst = dir.path().join("out.wav");
    let frames = cut_wav(&src, &dst, range(1_000, 3_000)).unwrap();
    assert_eq!(frames, 2_000 * 48);
    let reader = hound::WavReader::open(&dst).unwrap();
    assert_eq!(reader.spec(), spec);
    assert_eq!(reader.duration() as u64, 2_000 * 48);
    let samples = read_i16(&dst);
    assert_eq!(&samples[..2], &[1_000, -1_000]);
    assert_eq!(&samples[samples.len() - 2..], &[2_999, -2_999]);
}

#[test]
fn a_float_wav_stays_float() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("f.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(&src, spec).unwrap();
    for i in 0..3_000 * 16 {
        w.write_sample((i / 16) as f32 / 4_000.0).unwrap();
    }
    w.finalize().unwrap();
    let dst = dir.path().join("o.wav");
    assert_eq!(
        cut_wav(&src, &dst, range(1_000, 2_000)).unwrap(),
        1_000 * 16
    );
    let mut reader = hound::WavReader::open(&dst).unwrap();
    assert_eq!(reader.spec(), spec);
    let first: f32 = reader.samples::<f32>().next().unwrap().unwrap();
    assert!((first - 0.25).abs() < 1e-6);
}

#[test]
fn a_range_behind_the_end_gives_a_valid_wav_without_samples() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("in.wav");
    write_wav(&src, 1_000);
    let dst = dir.path().join("out.wav");
    assert_eq!(cut_wav(&src, &dst, range(5_000, 9_000)).unwrap(), 0);
    let reader = hound::WavReader::open(&dst).unwrap();
    assert_eq!(reader.duration(), 0);
    assert_eq!(reader.spec().sample_rate, 16_000);
}

#[test]
fn a_failing_cut_leaves_no_half_written_file() {
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("out.wav");
    let err = cut_wav(&dir.path().join("fehlt.wav"), &dst, range(0, 1_000)).unwrap_err();
    assert!(err.starts_with("wav_unreadable"), "{err}");
    assert!(!dst.exists());
}

// ---------------------------------------------------------------------------
// Store und Ablauf
// ---------------------------------------------------------------------------

struct Fixture {
    _dir: tempfile::TempDir,
    store: MeetingStore,
    id: String,
    dir: PathBuf,
    mic: PathBuf,
    system: PathBuf,
}

fn variant_row(
    store: &MeetingStore,
    id: &str,
    kind: &str,
    active: i64,
    segs: &[StoredSegment],
    meta: Option<&str>,
) {
    store
        .get_connection()
        .unwrap()
        .execute(
            "INSERT INTO transcript_variants (id, meeting_id, kind, segments_json, speaker_hints_json,
                 number, created_at, active, meta_json)
             SELECT ?1, meeting_id, ?2, ?3, speaker_hints_json,
                 (SELECT COUNT(*) FROM transcript_variants) + 1, 1, ?4, ?5
             FROM transcripts LIMIT 1",
            params![id, kind, serde_json::to_string(segs).unwrap(), active, meta],
        )
        .unwrap();
}

/// Eine fertige Aufnahme: Mikrofon 10 s, Systemton 6 s (kuerzer), Transkript mit
/// vier Saetzen, Turns, Notizen, zwei Teilen, einer Folie und einer Uebersetzung.
fn fixture() -> Fixture {
    let (tmp, store) = tmp_store();
    let m = store
        .create_meeting("Aufnahme", MeetingSource::Live, Some(1))
        .unwrap();
    let dir = tmp.path().join(&m.id);
    std::fs::create_dir_all(&dir).unwrap();
    let mic = dir.join("mic.wav");
    let system = dir.join("system.wav");
    write_wav(&mic, 10_000);
    write_wav(&system, 6_000);
    // mic_aec.wav gehoert zur mic.wav und geht mit ihr.
    write_wav(&dir.join("mic_aec.wav"), 10_000);
    store
        .set_audio_paths(&m.id, mic.to_str(), system.to_str(), Some(10_000))
        .unwrap();
    let segments = vec![
        seg(0, 0, 1_800),
        seg(1, 2_200, 3_500),
        seg(2, 4_000, 5_500),
        seg(3, 8_000, 9_500),
    ];
    store
        .append_delta(
            &m.id,
            &TranscriptDelta {
                new_segments: segments.clone(),
            },
        )
        .unwrap();
    let hints = serde_json::json!({"v": 1, "channels": {"0": [[0, 1_800, 1], [4_000, 5_500, 2]]}});
    let conn = store.get_connection().unwrap();
    conn.execute(
        "UPDATE transcripts SET speaker_hints_json = ?1 WHERE meeting_id = ?2",
        params![hints.to_string(), m.id],
    )
    .unwrap();
    conn.execute(
        "UPDATE meetings SET metadata_json = ?1 WHERE id = ?2",
        params![
            r#"{"aec":{"file":"mic_aec.wav","wav_kept":true},"timeline":{"basis":"qpc"}}"#,
            m.id
        ],
    )
    .unwrap();
    for (i, (title, offset, duration)) in [("A", 0u64, 6_000u64), ("B", 6_000, 4_000)]
        .into_iter()
        .enumerate()
    {
        conn.execute(
            "INSERT INTO meeting_parts (meeting_id, part_index, title, source_path, offset_ms,
                 duration_ms, created_at) VALUES (?1, ?2, ?3, NULL, ?4, ?5, 1)",
            params![m.id, i as i64, title, offset as i64, duration as i64],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO meeting_chunks (meeting_id, source, text, created_at) VALUES (?1, 'transcript', 'x', 1)",
        params![m.id],
    )
    .unwrap();
    drop(conn);
    variant_row(&store, "v-stt", "stt", 1, &segments, None);
    let translated: Vec<StoredSegment> = segments
        .iter()
        .map(|s| StoredSegment {
            text: format!("Sentence {}", s.segment_index),
            ..s.clone()
        })
        .collect();
    variant_row(
        &store,
        "v-en",
        "translation",
        0,
        &translated,
        Some(
            r#"{"checked":4,"flagged":[{"segment_index":0,"reasons":["numbers"]},{"segment_index":2,"reasons":["empty"]}]}"#,
        ),
    );
    store
        .save_notes(
            &m.id,
            &[
                note("vorher", Some(500)),
                note("mitte", Some(4_200)),
                note("danach", None),
            ],
            0,
        )
        .unwrap();
    store
        .slide_insert(&crate::managers::meetings::slides::store::NewSlide {
            meeting_id: m.id.clone(),
            number: None,
            origin: "video",
            image_path: "slides/0001.jpg".into(),
            thumb_path: None,
            dhash: 1,
            occurrences: vec![SlideOccurrence {
                start_ms: 0,
                end_ms: 1_500,
            }],
        })
        .unwrap();
    store
        .slide_insert(&crate::managers::meetings::slides::store::NewSlide {
            meeting_id: m.id.clone(),
            number: None,
            origin: "video",
            image_path: "slides/0002.jpg".into(),
            thumb_path: None,
            dhash: 2,
            occurrences: vec![SlideOccurrence {
                start_ms: 3_000,
                end_ms: 7_000,
            }],
        })
        .unwrap();
    store.set_status(&m.id, MeetingStatus::Ready).unwrap();
    Fixture {
        _dir: tmp,
        store,
        id: m.id,
        dir,
        mic,
        system,
    }
}

fn texts(segments: &[StoredSegment]) -> Vec<(String, u64, u64)> {
    segments
        .iter()
        .map(|s| (s.text.clone(), s.start_ms, s.end_ms))
        .collect()
}

#[test]
fn trimming_moves_audio_transcript_versions_notes_parts_and_slides_in_one_go() {
    let f = fixture();
    let epoch_before = f.store.segment_epoch(&f.id).unwrap();
    let revision_before = f.store.transcript_snapshot(&f.id).unwrap().revision;

    let new_len = trim_meeting(&f.store, &f.id, range(2_000, 6_000))
        .unwrap()
        .duration_ms;
    // Mikrofon 4 s (2..6); der Systemton ist nur 6 s lang: auch 4 s.
    assert_eq!(new_len, 4_000);

    let m = f.store.get_meeting(&f.id).unwrap().unwrap();
    assert_eq!(m.duration_ms, Some(4_000));
    let (mic, system) = (
        PathBuf::from(m.mic_audio_path.unwrap()),
        PathBuf::from(m.system_audio_path.unwrap()),
    );
    // Neue Dateien in `trim-<ulid>/` mit den alten Namen, die alten samt alter
    // mic_aec.wav sind weg; die geschnittene mic_aec.wav liegt neben der neuen mic.wav.
    let new_dir = mic.parent().unwrap().to_path_buf();
    assert_eq!(new_dir.parent().unwrap(), f.dir);
    assert!(new_dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("trim-"));
    assert_eq!(mic.file_name().unwrap(), "mic.wav");
    assert_eq!(system, new_dir.join("system.wav"));
    assert!(!f.mic.exists() && !f.system.exists());
    assert!(!f.dir.join("mic_aec.wav").exists());
    let aec_samples = read_i16(&new_dir.join("mic_aec.wav"));
    assert_eq!(aec_samples.len(), 4_000 * 16);
    assert_eq!(aec_samples[0], 2_000);
    assert_eq!(
        crate::managers::meetings::derived_audio_paths(&mic.to_string_lossy()),
        vec![new_dir.join("mic_aec.wav").to_string_lossy().to_string()]
    );
    assert!(!f.dir.join(CLEANUP_FILE).exists());
    let mic_samples = read_i16(&mic);
    assert_eq!(mic_samples.len(), 4_000 * 16);
    assert_eq!(mic_samples[0], 2_000);
    assert_eq!(read_i16(&system).len(), 4_000 * 16);

    // Transkript: Satz 1, 2 (und der Rest von 0? 0..1800 liegt davor).
    let snapshot = f.store.transcript_snapshot(&f.id).unwrap();
    assert_eq!(
        texts(&snapshot.segments),
        vec![
            ("Satz 1".into(), 200, 1_500),
            ("Satz 2".into(), 2_000, 3_500)
        ]
    );
    assert_eq!(
        snapshot
            .segments
            .iter()
            .map(|s| s.segment_index)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(
        snapshot.segments[0].words.as_ref().unwrap()[0].start_ms,
        200
    );
    assert_eq!(f.store.segment_epoch(&f.id).unwrap(), epoch_before + 1);
    assert_eq!(snapshot.revision, revision_before + 1);
    // Die Deltas sind weg: nach dem Neuladen gilt der Stand aus `segments_json`.
    assert_eq!(f.store.get_segments(&f.id).unwrap().len(), 2);

    // Turns.
    let hints: Value =
        serde_json::from_str(&f.store.speaker_hints(&f.id).unwrap().unwrap()).unwrap();
    assert_eq!(
        hints["channels"]["0"],
        serde_json::json!([[2_000, 3_500, 2]])
    );

    // Fassungen werden mitgekuerzt, die Pruefmarken der Uebersetzung folgen.
    let conn = f.store.get_connection().unwrap();
    let (segs, meta): (String, Option<String>) = conn
        .query_row(
            "SELECT segments_json, meta_json FROM transcript_variants WHERE id = 'v-en'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let segs: Vec<StoredSegment> = serde_json::from_str(&segs).unwrap();
    assert_eq!(
        texts(&segs),
        vec![
            ("Sentence 1".into(), 200, 1_500),
            ("Sentence 2".into(), 2_000, 3_500)
        ]
    );
    let meta: Value = serde_json::from_str(&meta.unwrap()).unwrap();
    assert_eq!(
        meta["flagged"],
        serde_json::json!([{ "segment_index": 1, "reasons": ["empty"] }])
    );
    let active: String = conn
        .query_row(
            "SELECT segments_json FROM transcript_variants WHERE id = 'v-stt'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<StoredSegment>>(&active)
            .unwrap()
            .len(),
        2
    );

    // Metadaten: mic_aec.wav ist mitgeschnitten, der Verweis bleibt, die Zeitachse auch.
    let metadata = f.store.metadata_json(&f.id).unwrap().unwrap();
    assert_eq!(metadata["aec"]["file"], "mic_aec.wav");
    assert_eq!(metadata["aec"]["wav_kept"], true);
    assert_eq!(metadata["timeline"]["basis"], "qpc");

    // Notizen: Text bleibt, Positionen ausserhalb entfallen.
    let notes = f.store.get_notes(&f.id).unwrap();
    assert_eq!(notes.revision, 2);
    let got: Vec<(&str, Option<u64>)> = notes
        .blocks
        .iter()
        .map(|b| (b.text.as_str(), b.at_ms))
        .collect();
    assert_eq!(
        got,
        vec![("vorher", None), ("mitte", Some(2_200)), ("danach", None)]
    );

    // Teile: A (0..6 s) und B (6..10 s) im Bereich 2..6 s: nur A bleibt -> keine Liste.
    assert!(f.store.meeting_parts(&f.id).unwrap().is_empty());

    // Folien: die erste liegt ausserhalb (weich geloescht), die zweite ist verschoben.
    let slides = f.store.slides_list(&f.id).unwrap();
    assert_eq!(slides.len(), 1);
    assert_eq!(
        slides[0].occurrences,
        vec![SlideOccurrence {
            start_ms: 1_000,
            end_ms: 4_000
        }]
    );

    // Such-Index: neu aufbauen.
    let chunks: i64 = conn
        .query_row("SELECT COUNT(*) FROM meeting_chunks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(chunks, 0);
}

#[test]
fn a_track_that_ends_before_the_start_is_kept_as_an_empty_wav() {
    let f = fixture();
    // Systemton: 6 s. Bereich ab 7 s: nur das Mikrofon hat noch etwas.
    let new_len = trim_meeting(&f.store, &f.id, range(7_000, 10_000))
        .unwrap()
        .duration_ms;
    assert_eq!(new_len, 3_000);
    let m = f.store.get_meeting(&f.id).unwrap().unwrap();
    let system = PathBuf::from(m.system_audio_path.unwrap());
    assert_eq!(hound::WavReader::open(&system).unwrap().duration(), 0);
    assert_eq!(
        read_i16(&PathBuf::from(m.mic_audio_path.unwrap())).len(),
        3_000 * 16
    );
}

#[test]
fn an_end_behind_the_recording_is_clamped() {
    let f = fixture();
    let new_len = trim_meeting(&f.store, &f.id, range(4_000, 999_000))
        .unwrap()
        .duration_ms;
    assert_eq!(new_len, 6_000);
}

#[test]
fn a_failure_before_the_commit_changes_nothing_and_removes_the_new_files() {
    let f = fixture();
    // Ein unlesbares Transkript laesst die Transaktion scheitern, NACHDEM die WAVs geschnitten sind.
    f.store
        .get_connection()
        .unwrap()
        .execute(
            "UPDATE transcripts SET segments_json = 'kaputt' WHERE meeting_id = ?1",
            params![f.id],
        )
        .unwrap();
    let before = f.store.get_meeting(&f.id).unwrap().unwrap();
    let epoch = f.store.segment_epoch(&f.id).unwrap();

    let err = trim_meeting(&f.store, &f.id, range(2_000, 6_000)).unwrap_err();
    assert!(err.starts_with("store_failed"), "{err}");

    let after = f.store.get_meeting(&f.id).unwrap().unwrap();
    assert_eq!(after.mic_audio_path, before.mic_audio_path);
    assert_eq!(after.system_audio_path, before.system_audio_path);
    assert_eq!(after.duration_ms, before.duration_ms);
    assert_eq!(f.store.segment_epoch(&f.id).unwrap(), epoch);
    assert_eq!(f.store.get_notes(&f.id).unwrap().revision, 1);
    assert_eq!(f.store.slides_list(&f.id).unwrap().len(), 2);
    // Die alten Dateien sind noch da, die neuen verschwunden.
    assert!(f.mic.exists() && f.system.exists() && f.dir.join("mic_aec.wav").exists());
    let leftovers: Vec<String> = std::fs::read_dir(&f.dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("trim-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn trimming_is_refused_while_the_meeting_is_busy_or_without_usable_audio() {
    let f = fixture();
    for status in [MeetingStatus::Recording, MeetingStatus::Processing] {
        f.store.set_status(&f.id, status).unwrap();
        assert_eq!(
            trim_meeting(&f.store, &f.id, range(1_000, 2_000)),
            Err("meeting_busy".into())
        );
    }
    f.store.set_status(&f.id, MeetingStatus::Ready).unwrap();
    // Nichts zu schneiden / ungueltig.
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(0, 10_000)),
        Err("nothing_to_trim".into())
    );
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(0, 99_000)),
        Err("nothing_to_trim".into())
    );
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(3_000, 3_000)),
        Err("invalid_range".into())
    );
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(12_000, 15_000)),
        Err("invalid_range".into())
    );
    assert_eq!(
        trim_meeting(&f.store, "gibt-es-nicht", range(1_000, 2_000)),
        Err("meeting_not_found".into())
    );
    // Datei fehlt.
    std::fs::remove_file(&f.system).unwrap();
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(1_000, 2_000)),
        Err("audio_missing".into())
    );
    // Ohne Audio.
    f.store.set_audio_paths(&f.id, None, None, None).unwrap();
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(1_000, 2_000)),
        Err("no_audio".into())
    );
    // Nichts davon hat das Transkript angefasst.
    assert_eq!(f.store.segment_epoch(&f.id).unwrap(), 0);
}

#[test]
fn a_change_of_the_audio_paths_between_cutting_and_committing_is_refused() {
    let f = fixture();
    let err = f
        .store
        .apply_trim(&TrimWrite {
            meeting_id: &f.id,
            old_mic: Some("ein/anderer/Pfad.wav"),
            old_system: f.system.to_str(),
            new_mic: None,
            new_system: None,
            duration_ms: 1,
            range: range(1_000, 2_000),
            aec_kept: false,
        })
        .unwrap_err();
    assert_eq!(err, "trim_changed");
    assert_eq!(f.store.segment_epoch(&f.id).unwrap(), 0);
}

fn set_metadata(f: &Fixture, json: &str) {
    f.store
        .get_connection()
        .unwrap()
        .execute(
            "UPDATE meetings SET metadata_json = ?1 WHERE id = ?2",
            params![json, f.id],
        )
        .unwrap();
}

fn current_mic(f: &Fixture) -> PathBuf {
    PathBuf::from(
        f.store
            .get_meeting(&f.id)
            .unwrap()
            .unwrap()
            .mic_audio_path
            .unwrap(),
    )
}

fn trim_dirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir() && is_trim_dir(p))
        .collect()
}

#[test]
fn trimming_twice_keeps_one_flat_folder_and_removes_the_previous_one() {
    let f = fixture();
    trim_meeting(&f.store, &f.id, range(1_000, 9_000)).unwrap();
    let first = current_mic(&f).parent().unwrap().to_path_buf();
    assert_eq!(trim_dirs(&f.dir), vec![first.clone()]);

    let result = trim_meeting(&f.store, &f.id, range(1_000, 5_000)).unwrap();
    assert_eq!(result.duration_ms, 4_000);
    assert_eq!(result.leftover_files, 0);
    let second = current_mic(&f).parent().unwrap().to_path_buf();
    assert_ne!(first, second);
    assert_eq!(second.parent().unwrap(), f.dir, "nicht ineinander");
    assert_eq!(trim_dirs(&f.dir), vec![second.clone()]);
    assert!(!first.exists());
    // Die entechote Kopie ist wieder neben der mic.wav und gleich lang.
    assert_eq!(read_i16(&second.join("mic_aec.wav")).len(), 4_000 * 16);
    assert_eq!(read_i16(&second.join("mic.wav"))[0], 2_000);
}

#[test]
fn without_an_aec_file_next_to_the_mic_the_reference_in_the_metadata_goes() {
    let f = fixture();
    std::fs::remove_file(f.dir.join("mic_aec.wav")).unwrap();
    trim_meeting(&f.store, &f.id, range(2_000, 6_000)).unwrap();
    let new_dir = current_mic(&f).parent().unwrap().to_path_buf();
    assert!(!new_dir.join("mic_aec.wav").exists());
    let metadata = f.store.metadata_json(&f.id).unwrap().unwrap();
    assert!(metadata["aec"].get("file").is_none());
    assert_eq!(metadata["aec"]["wav_kept"], false);
    assert_eq!(metadata["timeline"]["basis"], "qpc");
}

#[test]
fn speaker_merges_follow_their_segments_and_turns_to_the_new_epoch() {
    use crate::managers::meetings::speaker_merges::{MergeRecord, MERGES_KEY};
    let f = fixture();
    let rec = |id: &str, epoch: u32, segments: Vec<u32>, turns: Vec<usize>| MergeRecord {
        id: id.into(),
        channel: 0,
        from: 2,
        into: 1,
        epoch,
        segments,
        turns,
        ..Default::default()
    };
    // Segmente 0..3 liegen bei 0, 2200, 4000, 8000 ms; Turns bei 0 und 4000 ms.
    let records = vec![
        rec("teils", 0, vec![0, 2, 3], vec![0, 1]),
        rec("ausserhalb", 0, vec![0, 3], vec![0]),
        rec("fremd", 5, vec![3], vec![0]),
    ];
    set_metadata(
        &f,
        &serde_json::json!({ MERGES_KEY: records, "timeline": {"basis": "qpc"} }).to_string(),
    );
    trim_meeting(&f.store, &f.id, range(2_000, 6_000)).unwrap();

    let epoch = f.store.segment_epoch(&f.id).unwrap();
    assert_eq!(epoch, 1);
    let metadata = f.store.metadata_json(&f.id).unwrap().unwrap();
    let list: Vec<MergeRecord> = serde_json::from_value(metadata[MERGES_KEY].clone()).unwrap();
    let ids: Vec<&str> = list.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["teils", "fremd"], "ohne Rest entfaellt der Eintrag");
    // Segment 2 (4000 ms) ist jetzt Nummer 1, Turn 1 (4000 ms) jetzt Position 0.
    assert_eq!(list[0].segments, vec![1]);
    assert_eq!(list[0].turns, vec![0]);
    assert_eq!(list[0].epoch, epoch);
    // Ein Eintrag einer anderen Epoche bleibt, wie er war.
    assert_eq!(list[1].epoch, 5);
    assert_eq!(list[1].segments, vec![3]);
    assert_eq!(metadata["timeline"]["basis"], "qpc");
    // Er gilt auf der neuen Epoche noch.
    assert_eq!(
        crate::managers::meetings::speaker_merges::live(list, epoch).len(),
        1
    );
}

#[test]
fn a_locked_old_file_goes_on_the_cleanup_list_and_is_reported() {
    let f = fixture();
    // Ein Eintrag aus einem frueheren Zuschnitt, der noch offen ist (ein Ordner
    // laesst sich mit remove_file nicht loeschen, wie eine gesperrte Datei).
    let stuck = f.dir.join("alt-gesperrt.wav");
    std::fs::create_dir_all(&stuck).unwrap();
    write_cleanup(&f.dir, &[stuck.to_string_lossy().to_string()]);

    let result = trim_meeting(&f.store, &f.id, range(2_000, 6_000)).unwrap();
    assert_eq!(result.leftover_files, 1);
    assert_eq!(read_cleanup(&f.dir).len(), 1);

    // Der Ordner ist weg (Sperre geloest): der naechste Versuch raeumt die Liste ab.
    std::fs::remove_dir_all(&stuck).unwrap();
    assert_eq!(retry_cleanup(&f.dir), 0);
    assert!(!f.dir.join(CLEANUP_FILE).exists());
}

#[test]
fn remove_files_reports_what_stays_and_treats_a_missing_file_as_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.wav");
    write_wav(&file, 10);
    let dir = tmp.path().join("gesperrt.wav");
    std::fs::create_dir(&dir).unwrap();
    let missing = tmp.path().join("weg.wav");
    let paths: Vec<String> = [&file, &dir, &missing]
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    assert_eq!(remove_files(&paths), vec![paths[1].clone()]);
    assert!(!file.exists());
}

#[test]
fn the_start_sweep_retries_the_list_and_removes_only_orphaned_trim_folders() {
    let f = fixture();
    let meetings_dir = f._dir.path().to_path_buf();
    trim_meeting(&f.store, &f.id, range(2_000, 6_000)).unwrap();
    let used = current_mic(&f).parent().unwrap().to_path_buf();

    // Verwaist: ein Ordner, den keine Besprechung nutzt (Abbruch vor dem Commit).
    let orphan = f.dir.join("trim-verwaist");
    std::fs::create_dir_all(&orphan).unwrap();
    write_wav(&orphan.join("mic.wav"), 100);
    // Kein trim-Ordner: bleibt.
    let other = f.dir.join("export");
    std::fs::create_dir_all(&other).unwrap();
    // Aufraeumliste: eine Datei im Ordner (wird geloescht), eine ausserhalb (bleibt).
    let old = f.dir.join("alt.wav");
    write_wav(&old, 100);
    let outside = meetings_dir.join("fremd.wav");
    write_wav(&outside, 100);
    write_cleanup(
        &f.dir,
        &[
            old.to_string_lossy().to_string(),
            outside.to_string_lossy().to_string(),
        ],
    );

    let report = sweep_leftovers(&f.store, &meetings_dir);
    assert_eq!(report.orphan_dirs_removed, 1);
    assert!(!orphan.exists() && !old.exists());
    assert!(used.join("mic.wav").exists(), "benutzter Ordner bleibt");
    assert!(other.exists());
    assert!(
        outside.exists(),
        "ausserhalb des Ordners wird nichts geloescht"
    );
    assert!(!f.dir.join(CLEANUP_FILE).exists() || read_cleanup(&f.dir).is_empty());
}

#[test]
fn reading_a_range_with_an_absurd_end_does_not_overflow() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.wav");
    write_wav(&path, 1_000);
    let all = read_track_range(
        &path,
        TimeRange {
            start_ms: 0,
            end_ms: u64::MAX,
        },
    )
    .unwrap();
    assert_eq!(all.len(), 16_000);
    let none = read_track_range(
        &path,
        TimeRange {
            start_ms: u64::MAX,
            end_ms: u64::MAX,
        },
    )
    .unwrap();
    assert!(none.is_empty());
}
