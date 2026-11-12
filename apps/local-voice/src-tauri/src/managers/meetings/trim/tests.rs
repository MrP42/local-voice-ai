//! Tests fuer Bereiche einer Aufnahme (`trim.rs`): Zeitfunktionen, WAV-Schnitt und
//! die Transaktion (Klemmen, Versatz, Woerter, kuerzere und leere Spur, Fehlerpfad
//! ohne Aenderung).

use super::folder::{
    open_folder, push_superseded, Channel, NewDir, Superseded, TrimId, Unusable, SUPERSEDED_KEY,
    SUPERSEDED_MAX,
};
use super::*;
use crate::managers::meetings::notes::model::NoteBlockKind;
use crate::managers::meetings::rerun_state;
use crate::managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};
use crate::managers::transcription::WordTime;
use cap_std::fs::Dir;

fn range(start_ms: u64, end_ms: u64) -> TimeRange {
    TimeRange { start_ms, end_ms }
}

/// Der Auftrag (Sperre) der Besprechung, wie ihn `trim_now` haelt.
fn lock(meeting_id: &str) -> JobGuard {
    crate::managers::meetings::job::global()
        .try_start(meeting_id, Arc::new(|_| {}))
        .expect("Besprechung ist frei")
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

/// Ein frischer Ordner `neu` unter `base`, als Handle festgehalten (wie ihn der Zuschnitt
/// anlegt). Gibt das Handle von `base`, den neuen Ordner und dessen Pfad zurueck.
fn fresh_dir(base: &Path) -> (Dir, NewDir, PathBuf) {
    let home = Dir::open_ambient_dir(base, cap_std::ambient_authority()).unwrap();
    let out = NewDir::create(&home, "neu").unwrap();
    (home, out, base.join("neu"))
}

/// Eine WAV, deren Kopf mehr Daten verspricht, als die Datei hat: das Lesen scheitert mitten
/// im Schnitt (`wav_unreadable`), NACHDEM die Zieldatei angelegt ist.
fn write_truncated_wav(path: &Path) {
    write_wav(path, 500);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(60)
        .unwrap();
}

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

    let (_home, mut out, folder) = fresh_dir(dir.path());
    let cut = cut_wav(&src, &mut out, "out.wav", range(1_000, 3_000)).unwrap();
    assert_eq!(cut.frames, 2_000 * 48);
    assert_eq!(cut.sample_rate, 48_000);
    assert_eq!(cut.len_ms(), 2_000);
    let dst = folder.join("out.wav");
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
    let (_home, mut out, folder) = fresh_dir(dir.path());
    assert_eq!(
        cut_wav(&src, &mut out, "o.wav", range(1_000, 2_000))
            .unwrap()
            .frames,
        1_000 * 16
    );
    let mut reader = hound::WavReader::open(folder.join("o.wav")).unwrap();
    assert_eq!(reader.spec(), spec);
    let first: f32 = reader.samples::<f32>().next().unwrap().unwrap();
    assert!((first - 0.25).abs() < 1e-6);
}

#[test]
fn a_range_behind_the_end_gives_a_valid_wav_without_samples() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("in.wav");
    write_wav(&src, 1_000);
    let (_home, mut out, folder) = fresh_dir(dir.path());
    assert_eq!(
        cut_wav(&src, &mut out, "out.wav", range(5_000, 9_000))
            .unwrap()
            .frames,
        0
    );
    let reader = hound::WavReader::open(folder.join("out.wav")).unwrap();
    assert_eq!(reader.duration(), 0);
    assert_eq!(reader.spec().sample_rate, 16_000);
}

#[test]
fn an_unreadable_source_creates_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let (_home, mut out, folder) = fresh_dir(dir.path());
    let err = cut_wav(
        &dir.path().join("fehlt.wav"),
        &mut out,
        "out.wav",
        range(0, 1_000),
    )
    .unwrap_err();
    assert!(err.starts_with("wav_unreadable"), "{err}");
    assert!(!folder.join("out.wav").exists());
}

/// Ein Fehler MITTEN im Schnitt: die halbe Datei entfernt die Bereinigung ueber das Handle,
/// danach auch den leeren Ordner.
#[test]
fn a_failing_cut_leaves_no_half_written_file_after_the_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("kurz.wav");
    write_truncated_wav(&src);
    let (home, mut out, folder) = fresh_dir(dir.path());
    let err = cut_wav(&src, &mut out, "out.wav", range(0, 400)).unwrap_err();
    assert!(err.starts_with("wav_unreadable"), "{err}");
    assert!(
        folder.join("out.wav").exists(),
        "die halbe Datei liegt noch"
    );
    out.discard(&home);
    assert!(!folder.exists(), "Datei und Ordner sind weg");
    assert!(src.exists(), "die Quelle bleibt");
}

#[test]
fn a_cut_never_overwrites_an_existing_file_and_the_cleanup_leaves_it() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("in.wav");
    write_wav(&src, 2_000);
    let (home, mut out, folder) = fresh_dir(dir.path());
    let dst = folder.join("out.wav");
    std::fs::write(&dst, b"vorhanden").unwrap();
    let err = cut_wav(&src, &mut out, "out.wav", range(0, 1_000)).unwrap_err();
    assert!(err.starts_with("wav_write_failed"), "{err}");
    assert_eq!(std::fs::read(&dst).unwrap(), b"vorhanden");
    // Die Bereinigung entfernt nur, was der Zuschnitt selbst angelegt hat.
    out.discard(&home);
    assert_eq!(
        std::fs::read(&dst).unwrap(),
        b"vorhanden",
        "die vorhandene Datei bleibt, wie sie war (auch nicht geloescht)"
    );
}

/// Voller Datentraeger: der Schreibfehler wird als `wav_write_failed` gemeldet, nie
/// verschluckt.
#[test]
fn a_full_disk_is_reported_as_a_write_error() {
    struct FullDisk {
        buf: std::io::Cursor<Vec<u8>>,
        limit: usize,
    }
    impl std::io::Write for FullDisk {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            if self.buf.position() as usize + data.len() > self.limit {
                return Err(std::io::Error::other("no space left on device"));
            }
            self.buf.write(data)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl std::io::Seek for FullDisk {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            self.buf.seek(pos)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("in.wav");
    write_wav(&src, 2_000);
    let mut reader = hound::WavReader::open(&src).unwrap();
    let full = FullDisk {
        buf: std::io::Cursor::new(Vec::new()),
        limit: 200,
    };
    let err = cut_wav_into(&mut reader, full, range(0, 1_000)).unwrap_err();
    assert!(err.starts_with("wav_write_failed"), "{err}");
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

    let new_len = trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id))
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
    // Neue Dateien in `trim-<ulid>/` mit den festen Namen; die alten samt alter
    // mic_aec.wav bleiben liegen (siehe `the_old_files_stay_untouched...`); die
    // geschnittene mic_aec.wav liegt neben der neuen mic.wav.
    let new_dir = mic.parent().unwrap().to_path_buf();
    assert_eq!(new_dir.parent().unwrap(), f.dir);
    assert!(new_dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("trim-"));
    assert_eq!(mic.file_name().unwrap(), "mic.wav");
    assert_eq!(system, new_dir.join("system.wav"));
    assert!(f.mic.exists() && f.system.exists());
    assert!(f.dir.join("mic_aec.wav").exists());
    let aec_samples = read_i16(&new_dir.join("mic_aec.wav"));
    assert_eq!(aec_samples.len(), 4_000 * 16);
    assert_eq!(aec_samples[0], 2_000);
    assert_eq!(
        crate::managers::meetings::derived_audio_paths(&mic.to_string_lossy()),
        vec![new_dir.join("mic_aec.wav").to_string_lossy().to_string()]
    );
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
    let new_len = trim_meeting(&f.store, &f.id, range(7_000, 10_000), &lock(&f.id))
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
    let new_len = trim_meeting(&f.store, &f.id, range(4_000, 999_000), &lock(&f.id))
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

    let err = trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap_err();
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
    // Und kein Vermerk ueber ersetzte Dateien: derselbe Commit, der scheiterte.
    assert!(superseded(&f).is_empty());
}

#[test]
fn trimming_is_refused_while_the_meeting_is_busy_or_without_usable_audio() {
    let f = fixture();
    for status in [MeetingStatus::Recording, MeetingStatus::Processing] {
        f.store.set_status(&f.id, status).unwrap();
        assert_eq!(
            trim_meeting(&f.store, &f.id, range(1_000, 2_000), &lock(&f.id)),
            Err("meeting_busy".into())
        );
    }
    f.store.set_status(&f.id, MeetingStatus::Ready).unwrap();
    // Nichts zu schneiden / ungueltig.
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(0, 10_000), &lock(&f.id)),
        Err("nothing_to_trim".into())
    );
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(0, 99_000), &lock(&f.id)),
        Err("nothing_to_trim".into())
    );
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(3_000, 3_000), &lock(&f.id)),
        Err("invalid_range".into())
    );
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(12_000, 15_000), &lock(&f.id)),
        Err("invalid_range".into())
    );
    assert_eq!(
        trim_meeting(
            &f.store,
            "gibt-es-nicht",
            range(1_000, 2_000),
            &lock("gibt-es-nicht")
        ),
        Err("meeting_not_found".into())
    );
    // Datei fehlt.
    std::fs::remove_file(&f.system).unwrap();
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(1_000, 2_000), &lock(&f.id)),
        Err("audio_missing".into())
    );
    // Ohne Audio.
    f.store.set_audio_paths(&f.id, None, None, None).unwrap();
    assert_eq!(
        trim_meeting(&f.store, &f.id, range(1_000, 2_000), &lock(&f.id)),
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
            trim_id: &ulid_of('b'),
            old_mic: Some("ein/anderer/Pfad.wav"),
            old_system: f.system.to_str(),
            old_aec: None,
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

/// Streng `trim-<ULID>`.
fn is_trim_dir(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("trim-"))
        .is_some_and(|ulid| TrimId::parse(ulid).is_some())
}

fn trim_dirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir() && is_trim_dir(p))
        .collect()
}

#[test]
fn without_an_aec_file_next_to_the_mic_the_reference_in_the_metadata_goes() {
    let f = fixture();
    std::fs::remove_file(f.dir.join("mic_aec.wav")).unwrap();
    trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();
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
    trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();

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

// ---------------------------------------------------------------------------
// Alte Dateien bleiben liegen (trim_superseded) und der Zuschnittsordner (folder.rs)
// ---------------------------------------------------------------------------

/// Gueltige ULID (Crockford-Base32, 26 Zeichen) fuer feste Testordner.
fn ulid_of(fill: char) -> String {
    format!("01{}", fill.to_string().repeat(24))
}

fn ulid_dir(fill: char) -> String {
    format!("trim-{}", ulid_of(fill))
}

fn path_text(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

fn metadata_raw(f: &Fixture) -> String {
    f.store
        .get_connection()
        .unwrap()
        .query_row(
            "SELECT metadata_json FROM meetings WHERE id = ?1",
            params![f.id],
            |r| r.get(0),
        )
        .unwrap()
}

/// Die Vermerke `trim_superseded` der Besprechung.
fn superseded(f: &Fixture) -> Vec<Value> {
    f.store
        .metadata_json(&f.id)
        .unwrap()
        .unwrap()
        .get(SUPERSEDED_KEY)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Der Vermerk zu `path`/`channel`; stellt sicher, dass es genau einen gibt.
fn note_for<'a>(list: &'a [Value], path: &Path, channel: &str) -> &'a Value {
    let found: Vec<&Value> = list
        .iter()
        .filter(|v| v["path"] == path_text(path).as_str() && v["channel"] == channel)
        .collect();
    assert_eq!(found.len(), 1, "{path:?} / {channel} in {list:?}");
    found[0]
}

/// Die ULID eines Ordners `trim-<ULID>`.
fn trim_id_of(dir: &Path) -> String {
    dir.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("trim-"))
        .unwrap()
        .to_string()
}

/// Namen (sortiert) direkt in einem Ordner.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// Die alten Dateien bleiben Byte fuer Byte liegen, alles andere im Besprechungsordner
/// auch, und der Commit vermerkt sie dauerhaft.
#[test]
fn the_old_files_stay_untouched_and_are_recorded_as_superseded() {
    let f = fixture();
    let aec = f.dir.join("mic_aec.wav");
    // Fremdes im Besprechungsordner, auch mit einem Namen, der wie ein app-eigener aussieht.
    let notes = f.dir.join("notizen.wav");
    write_wav(&notes, 300);
    std::fs::write(f.dir.join("fremd.txt"), b"fremd").unwrap();
    let before: Vec<(PathBuf, Vec<u8>)> = [&f.mic, &f.system, &aec, &notes]
        .into_iter()
        .map(|p| (p.clone(), std::fs::read(p).unwrap()))
        .collect();

    trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();

    for (path, bytes) in &before {
        assert_eq!(&std::fs::read(path).unwrap(), bytes, "{path:?}");
    }
    assert_eq!(std::fs::read(f.dir.join("fremd.txt")).unwrap(), b"fremd");
    let dirs = trim_dirs(&f.dir);
    assert_eq!(dirs.len(), 1);
    let mut expected = vec![
        "fremd.txt".to_string(),
        "mic.wav".to_string(),
        "mic_aec.wav".to_string(),
        "notizen.wav".to_string(),
        "system.wav".to_string(),
        dirs[0].file_name().unwrap().to_string_lossy().to_string(),
    ];
    expected.sort();
    assert_eq!(
        names_in(&f.dir),
        expected,
        "nichts kam hinzu ausser dem neuen Ordner, nichts fiel weg"
    );

    let list = superseded(&f);
    assert_eq!(list.len(), 3, "{list:?}");
    let id = trim_id_of(&dirs[0]);
    for (path, channel) in [(&f.mic, "mic"), (&f.system, "system"), (&aec, "mic_aec")] {
        let note = note_for(&list, path, channel);
        assert_eq!(note["trim_id"], id.as_str());
        assert!(note["at"].as_i64().unwrap() > 0);
    }
    // Die Fremddatei steht nirgends.
    assert!(list.iter().all(|v| v["path"] != path_text(&notes).as_str()));
}

#[test]
fn trimming_twice_keeps_both_generations_and_records_the_first_folder() {
    let f = fixture();
    let aec = f.dir.join("mic_aec.wav");
    trim_meeting(&f.store, &f.id, range(1_000, 9_000), &lock(&f.id)).unwrap();
    let first = current_mic(&f).parent().unwrap().to_path_buf();
    assert_eq!(trim_dirs(&f.dir), vec![first.clone()]);
    let first_bytes = (
        std::fs::read(first.join("mic.wav")).unwrap(),
        std::fs::read(first.join("system.wav")).unwrap(),
        std::fs::read(first.join("mic_aec.wav")).unwrap(),
    );

    let result = trim_meeting(&f.store, &f.id, range(1_000, 5_000), &lock(&f.id)).unwrap();
    assert_eq!(result.duration_ms, 4_000);
    let second = current_mic(&f).parent().unwrap().to_path_buf();
    assert_ne!(first, second);
    assert_eq!(second.parent().unwrap(), f.dir, "nicht ineinander");
    let mut dirs = trim_dirs(&f.dir);
    dirs.sort();
    let mut both = vec![first.clone(), second.clone()];
    both.sort();
    assert_eq!(dirs, both, "beide Ordner liegen da");
    // Der erste Ordner ist unangetastet.
    assert_eq!(
        (
            std::fs::read(first.join("mic.wav")).unwrap(),
            std::fs::read(first.join("system.wav")).unwrap(),
            std::fs::read(first.join("mic_aec.wav")).unwrap(),
        ),
        first_bytes
    );
    // Die entechote Kopie ist wieder neben der mic.wav und gleich lang.
    assert_eq!(read_i16(&second.join("mic_aec.wav")).len(), 4_000 * 16);
    assert_eq!(read_i16(&second.join("mic.wav"))[0], 2_000);

    // Sechs Vermerke: die Originale (erster Zuschnitt) und die Dateien des ersten Ordners
    // (zweiter Zuschnitt), je mit dem Zuschnitt, der sie ersetzt hat.
    let list = superseded(&f);
    assert_eq!(list.len(), 6, "{list:?}");
    let (first_id, second_id) = (trim_id_of(&first), trim_id_of(&second));
    for (path, channel) in [(&f.mic, "mic"), (&f.system, "system"), (&aec, "mic_aec")] {
        assert_eq!(note_for(&list, path, channel)["trim_id"], first_id.as_str());
    }
    for (name, channel) in [
        ("mic.wav", "mic"),
        ("system.wav", "system"),
        ("mic_aec.wav", "mic_aec"),
    ] {
        assert_eq!(
            note_for(&list, &first.join(name), channel)["trim_id"],
            second_id.as_str()
        );
    }
}

#[test]
fn a_known_note_is_not_added_twice_and_foreign_notes_survive() {
    let mut root = serde_json::json!({
        SUPERSEDED_KEY: [
            {"path": "a/mic.wav", "trim_id": ulid_of('a'), "channel": "mic", "at": 1},
            "fremder Eintrag"
        ],
        "timeline": {"basis": "qpc"}
    });
    let note = |path: &str, channel: Channel, fill: char| Superseded {
        path: path.to_string(),
        trim_id: TrimId::parse(&ulid_of(fill)).unwrap(),
        channel,
        at: 2,
    };
    // Derselbe Pfad und dieselbe Spur: schon vermerkt, nichts aendert sich.
    assert!(!push_superseded(
        &mut root,
        &[note("a/mic.wav", Channel::Mic, 'b')]
    ));
    assert_eq!(root[SUPERSEDED_KEY].as_array().unwrap().len(), 2);
    // Dieselbe Datei als andere Spur und eine neue Datei kommen dazu; Fremdes bleibt.
    assert!(push_superseded(
        &mut root,
        &[
            note("a/mic.wav", Channel::MicAec, 'b'),
            note("b/mic.wav", Channel::Mic, 'b')
        ]
    ));
    let list = root[SUPERSEDED_KEY].as_array().unwrap();
    assert_eq!(list.len(), 4);
    assert_eq!(list[1], "fremder Eintrag");
    assert_eq!(list[3]["trim_id"], ulid_of('b').as_str());
    assert_eq!(root["timeline"]["basis"], "qpc");
}

#[test]
fn the_list_of_notes_is_capped_and_keeps_the_newest() {
    let mut root = serde_json::json!({});
    for n in 0..SUPERSEDED_MAX + 40 {
        let entry = Superseded {
            path: format!("alt/{n}/mic.wav"),
            trim_id: TrimId::parse(&ulid_of('a')).unwrap(),
            channel: Channel::Mic,
            at: n as i64,
        };
        assert!(push_superseded(&mut root, &[entry]));
    }
    let list = root[SUPERSEDED_KEY].as_array().unwrap();
    assert_eq!(list.len(), SUPERSEDED_MAX);
    assert_eq!(
        list.last().unwrap()["path"],
        format!("alt/{}/mic.wav", SUPERSEDED_MAX + 39).as_str()
    );
    assert_eq!(list[0]["path"], "alt/40/mic.wav");
}

/// Beschaedigte Vermerke werden nie ueberschrieben: der Zuschnitt lehnt ab.
#[test]
fn damaged_notes_abort_the_trim_and_changes_nothing() {
    for broken in [r#"{"trim_superseded":"text"}"#, r#"{"trim_superseded":{}}"#] {
        let f = fixture();
        set_metadata(&f, broken);
        let err = trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap_err();
        assert_untouched(&f, &err, "metadata_unreadable");
        assert_eq!(metadata_raw(&f), broken);
    }
    // `null` und eine leere Liste sind lesbar.
    for fine in [r#"{"trim_superseded":null}"#, r#"{"trim_superseded":[]}"#] {
        let f = fixture();
        set_metadata(&f, fine);
        trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();
        assert!(!superseded(&f).is_empty());
    }
}

/// Neue Spuren haben feste Namen je Kanal und werden nie ueberschrieben; die Quellen
/// ausserhalb des Besprechungsordners bleiben liegen und sind vermerkt.
#[test]
fn sources_whose_names_differ_only_in_case_get_two_separate_files() {
    let f = fixture();
    let (a, b) = (f._dir.path().join("A"), f._dir.path().join("B"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    write_wav(&a.join("mic.wav"), 10_000);
    write_wav(&b.join("MIC.WAV"), 8_000);
    f.store
        .set_audio_paths(
            &f.id,
            a.join("mic.wav").to_str(),
            b.join("MIC.WAV").to_str(),
            Some(10_000),
        )
        .unwrap();

    trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();
    let m = f.store.get_meeting(&f.id).unwrap().unwrap();
    let (mic, system) = (
        PathBuf::from(m.mic_audio_path.unwrap()),
        PathBuf::from(m.system_audio_path.unwrap()),
    );
    assert_eq!(mic.parent(), system.parent());
    assert_eq!(mic.file_name().unwrap(), "mic.wav");
    assert_eq!(system.file_name().unwrap(), "system.wav");
    assert_ne!(
        path_text(&mic).to_lowercase(),
        path_text(&system).to_lowercase(),
        "auch ohne Gross-/Kleinschreibung verschieden"
    );
    let (mic_samples, system_samples) = (read_i16(&mic), read_i16(&system));
    assert_eq!((mic_samples.len(), system_samples.len()), (64_000, 64_000));
    assert_eq!((mic_samples[0], system_samples[0]), (2_000, 2_000));
    // Beide Quellen liegen ausserhalb des Besprechungsordners: unangetastet und vermerkt.
    assert!(a.join("mic.wav").exists() && b.join("MIC.WAV").exists());
    let list = superseded(&f);
    note_for(&list, &a.join("mic.wav"), "mic");
    note_for(&list, &b.join("MIC.WAV"), "system");
}

#[test]
fn the_new_names_are_fixed_whatever_the_old_ones_were() {
    let f = fixture();
    let old_mic = f.dir.join("Sitzung-mic.wav");
    let old_system = f.dir.join("import.wav");
    write_wav(&old_mic, 10_000);
    write_wav(&old_system, 6_000);
    f.store
        .set_audio_paths(&f.id, old_mic.to_str(), old_system.to_str(), Some(10_000))
        .unwrap();
    trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();
    let m = f.store.get_meeting(&f.id).unwrap().unwrap();
    let (mic, system) = (
        PathBuf::from(m.mic_audio_path.unwrap()),
        PathBuf::from(m.system_audio_path.unwrap()),
    );
    assert_eq!(mic.file_name().unwrap(), "mic.wav");
    assert_eq!(system.file_name().unwrap(), "system.wav");
    assert!(old_mic.exists() && old_system.exists());
    let list = superseded(&f);
    note_for(&list, &old_mic, "mic");
    note_for(&list, &old_system, "system");
}

#[test]
fn the_origin_must_match_the_folder_of_the_new_files() {
    let f = fixture();
    let new_dir = f.dir.join(ulid_dir('a'));
    std::fs::create_dir_all(&new_dir).unwrap();
    write_wav(&new_dir.join("mic.wav"), 4_000);
    let new_mic = path_text(&new_dir.join("mic.wav"));
    for trim_id in [ulid_of('b'), "abc".to_string(), String::new()] {
        let err = f
            .store
            .apply_trim(&TrimWrite {
                meeting_id: &f.id,
                trim_id: &trim_id,
                old_mic: f.mic.to_str(),
                old_system: f.system.to_str(),
                old_aec: None,
                new_mic: Some(&new_mic),
                new_system: None,
                duration_ms: 4_000,
                range: range(2_000, 6_000),
                aec_kept: false,
            })
            .unwrap_err();
        assert_eq!(err, "store_failed: trim_id", "{trim_id:?}");
    }
    assert_eq!(current_mic(&f), f.mic);
    assert_eq!(f.store.segment_epoch(&f.id).unwrap(), 0);
    assert!(superseded(&f).is_empty());
}

/// Abbruch mitten im Vorgang: die App stirbt, nachdem die WAVs geschrieben sind, aber
/// vor dem Commit. Die Datenbank zeigt unveraendert auf die alten Dateien; uebrig bleibt nur
/// ein unbenutzter Ordner (Speicher, kein Datenverlust), und ein neuer Zuschnitt geht.
#[test]
fn a_crash_before_the_commit_leaves_the_old_state_intact() {
    let f = fixture();
    let home = open_folder(&f.store, &f.id).unwrap();
    let mut out = NewDir::create(&home.dir, &ulid_dir('c')).unwrap();
    let plans = vec![
        Some(TrackPlan {
            path: f.mic.clone(),
            len_ms: 10_000,
            aec: Some(f.dir.join("mic_aec.wav")),
        }),
        None,
    ];
    write_tracks(&mut out, &plans, range(2_000, 6_000)).unwrap();
    drop(out); // kein discard: die App ist weg
    drop(home);

    assert_eq!(current_mic(&f), f.mic);
    assert!(superseded(&f).is_empty());
    assert_eq!(f.store.segment_epoch(&f.id).unwrap(), 0);
    assert!(f.mic.exists() && f.system.exists());
    assert_eq!(trim_dirs(&f.dir).len(), 1, "der unbenutzte Ordner bleibt");

    let result = trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();
    assert_eq!(result.duration_ms, 4_000);
    assert_eq!(trim_dirs(&f.dir).len(), 2);
}

#[test]
fn a_failing_aec_cut_leaves_no_half_copy_next_to_the_new_mic() {
    let f = fixture();
    let broken = f._dir.path().join("kaputt.wav");
    write_truncated_wav(&broken);
    let home = open_folder(&f.store, &f.id).unwrap();
    let name = ulid_dir('d');
    let mut out = NewDir::create(&home.dir, &name).unwrap();
    let plans = vec![
        Some(TrackPlan {
            path: f.mic.clone(),
            len_ms: 10_000,
            aec: Some(broken),
        }),
        None,
    ];
    let written = write_tracks(&mut out, &plans, range(0, 400)).unwrap();
    assert!(!written.aec_kept);
    assert_eq!(written.names, [Some("mic.wav"), None]);
    let folder = f.dir.join(&name);
    assert_eq!(names_in(&folder), ["mic.wav"], "keine halbe mic_aec.wav");
}

/// Ein fremder Ordner im Zuschnittsordner-Namen darf nicht ueberschrieben oder entfernt
/// werden: der Zuschnitt legt ihn nur NEU an.
#[test]
fn the_trim_folder_is_created_new_and_an_existing_one_is_never_touched() {
    let f = fixture();
    let home = open_folder(&f.store, &f.id).unwrap();
    let name = ulid_dir('e');
    let existing = f.dir.join(&name);
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("mic.wav"), b"fremd").unwrap();
    let err = NewDir::create(&home.dir, &name).err().unwrap();
    assert!(err.starts_with("wav_write_failed"), "{err}");
    assert_eq!(std::fs::read(existing.join("mic.wav")).unwrap(), b"fremd");
}

/// Bereinigung nach einem Fehler: nur die selbst angelegten Dateien gehen, Fremdes im Ordner
/// bleibt, und der Ordner bleibt dann auch (nicht leer, `remove_dir` ist nie rekursiv).
#[test]
fn the_cleanup_removes_only_what_the_trim_created() {
    let f = fixture();
    let home = open_folder(&f.store, &f.id).unwrap();
    let name = ulid_dir('f');
    let mut out = NewDir::create(&home.dir, &name).unwrap();
    let folder = f.dir.join(&name);
    drop(out.create_file("mic.wav").unwrap());
    std::fs::write(folder.join("fremd.txt"), b"fremd").unwrap();
    out.discard(&home.dir);
    assert_eq!(names_in(&folder), ["fremd.txt"]);
    assert_eq!(std::fs::read(folder.join("fremd.txt")).unwrap(), b"fremd");
}

/// J: nach dem Oeffnen wird der Zuschnittsordner gegen einen Verweis auf einen Ordner
/// AUSSERHALB getauscht (der Alte wandert weg), dann scheitert der Schnitt an der zweiten
/// Spur. Dateien entstehen UND vergehen ueber das festgehaltene Handle: das Ziel des
/// Verweises bleibt Byte fuer Byte, wie es war, und es kommt nichts hinzu.
#[test]
fn swapping_the_new_folder_after_opening_never_touches_anything_outside() {
    let f = fixture();
    let outside = f._dir.path().join("aussen");
    std::fs::create_dir_all(&outside).unwrap();
    // Dieselben Namen wie die neuen Dateien: ein Ordner-Tausch haette sie sonst getroffen.
    let victims = ["mic.wav", "system.wav", "mic_aec.wav", "fremd.txt"];
    for name in victims {
        std::fs::write(outside.join(name), b"fremd").unwrap();
    }

    let home = open_folder(&f.store, &f.id).unwrap();
    let name = ulid_dir('c');
    let mut out = NewDir::create(&home.dir, &name).unwrap();
    let moved = f.dir.join("verschoben");
    std::fs::rename(f.dir.join(&name), &moved).unwrap();
    if !link_dir(&outside, &f.dir.join(&name)) {
        return;
    }

    let broken = f._dir.path().join("kaputt.wav");
    write_truncated_wav(&broken);
    let plans = vec![
        Some(TrackPlan {
            path: f.mic.clone(),
            len_ms: 10_000,
            aec: Some(f.dir.join("mic_aec.wav")),
        }),
        Some(TrackPlan {
            path: broken,
            len_ms: 500,
            aec: None,
        }),
    ];
    let err = match write_tracks(&mut out, &plans, range(0, 400)) {
        Err(e) => e,
        Ok(_) => panic!("der Schnitt der zweiten Spur haette scheitern muessen"),
    };
    assert!(err.starts_with("wav_unreadable"), "{err}");
    assert_eq!(
        names_in(&moved),
        ["mic.wav", "mic_aec.wav", "system.wav"],
        "geschrieben wurde in den festgehaltenen Ordner, nicht ueber den Namen"
    );
    out.discard(&home.dir);

    for name in victims {
        assert_eq!(
            std::fs::read(outside.join(name)).unwrap(),
            b"fremd",
            "{name}"
        );
    }
    assert_eq!(names_in(&outside), {
        let mut v: Vec<String> = victims.iter().map(|n| n.to_string()).collect();
        v.sort();
        v
    });
    assert!(
        names_in(&moved).is_empty(),
        "die eigenen Dateien sind ueber das Handle weg"
    );
}

/// J, zweiter Tausch: an den Namen des Ordners tritt ein echter, nicht leerer Ordner. Die
/// Bereinigung loescht darin nichts und entfernt ihn nicht.
#[test]
fn swapping_the_new_folder_for_a_populated_one_loses_nothing() {
    let f = fixture();
    let home = open_folder(&f.store, &f.id).unwrap();
    let name = ulid_dir('g');
    let mut out = NewDir::create(&home.dir, &name).unwrap();
    let theirs = f.dir.join(&name);
    let moved = f.dir.join("verschoben");
    std::fs::rename(&theirs, &moved).unwrap();
    std::fs::create_dir_all(&theirs).unwrap();
    std::fs::write(theirs.join("mic.wav"), b"fremd").unwrap();
    drop(out.create_file("mic.wav").unwrap());
    assert!(moved.join("mic.wav").exists());
    out.discard(&home.dir);
    assert_eq!(std::fs::read(theirs.join("mic.wav")).unwrap(), b"fremd");
    assert!(names_in(&moved).is_empty());
}

#[test]
fn the_folder_comes_from_the_data_root_not_from_stored_paths() {
    let f = fixture();
    // Die Zeile zeigt auf Audio in einem fremden Ordner; der neue Ordner entsteht trotzdem
    // nur unter Datenwurzel/ID, und im fremden Ordner kommt nichts hinzu.
    let elsewhere = f._dir.path().join("anderswo");
    std::fs::create_dir_all(&elsewhere).unwrap();
    write_wav(&elsewhere.join("mic.wav"), 10_000);
    f.store
        .set_audio_paths(
            &f.id,
            path_text(&elsewhere.join("mic.wav")).as_str().into(),
            None,
            Some(10_000),
        )
        .unwrap();
    trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();
    let mic = current_mic(&f);
    assert_eq!(mic.parent().unwrap().parent().unwrap(), f.dir);
    assert_eq!(names_in(&elsewhere), ["mic.wav"]);
}

#[cfg(unix)]
fn link_file(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}
#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}
#[cfg(windows)]
fn link_file(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}
#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}

/// Eine alte Datei, die ein Verweis ist, wird nie aufgeloest oder angefasst: der Zuschnitt
/// liest sie nur, schreibt in den neuen Ordner und vermerkt den Pfad.
#[test]
fn an_old_file_that_is_a_link_is_only_read_and_recorded() {
    let f = fixture();
    let outside = f._dir.path().join("aussen");
    std::fs::create_dir_all(&outside).unwrap();
    write_wav(&outside.join("echt.wav"), 10_000);
    let link = f.dir.join("verweis.wav");
    if !link_file(&outside.join("echt.wav"), &link) {
        return;
    }
    f.store
        .set_audio_paths(&f.id, link.to_str(), None, Some(10_000))
        .unwrap();
    let before = std::fs::read(outside.join("echt.wav")).unwrap();
    trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();
    assert_eq!(std::fs::read(outside.join("echt.wav")).unwrap(), before);
    assert!(std::fs::symlink_metadata(&link).is_ok());
    note_for(&superseded(&f), &link, "mic");
}

#[test]
fn a_meeting_folder_that_is_a_link_is_never_used() {
    let f = fixture();
    // Der Besprechungsordner selbst wird zum Verweis auf einen Ordner ausserhalb.
    let real = f._dir.path().join("echt");
    std::fs::rename(&f.dir, &real).unwrap();
    if !link_dir(&real, &f.dir) {
        return;
    }
    let err = trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap_err();
    assert_eq!(err, "audio_missing");
    assert!(trim_dirs(&real).is_empty(), "dort entsteht nichts");
    assert!(superseded(&f).is_empty());
}

#[test]
fn a_meeting_folder_that_links_to_another_meeting_inside_the_data_root_is_never_used() {
    let f = fixture();
    // Der Verweis zeigt auf einen Nachbarordner INNERHALB der Datenwurzel: `cap-std` wuerde
    // ihm folgen, deshalb prueft das Oeffnen den Besprechungsordner selbst als Nicht-Verweis.
    let sibling = f._dir.path().join("nachbar");
    std::fs::create_dir_all(&sibling).unwrap();
    let moved = f._dir.path().join("verschoben");
    std::fs::rename(&f.dir, &moved).unwrap();
    if !link_dir(&sibling, &f.dir) {
        return;
    }
    assert_eq!(
        open_folder(&f.store, &f.id).err(),
        Some(Unusable::Unsafe),
        "ein Verweis ist kein Besprechungsordner"
    );
    let err = trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap_err();
    assert_eq!(err, "audio_missing");
    assert!(
        names_in(&sibling).is_empty(),
        "im Nachbarordner entsteht nichts"
    );
}

#[test]
fn only_a_single_plain_name_opens_as_a_meeting_folder() {
    let f = fixture();
    assert!(open_folder(&f.store, &f.id).is_ok());
    for bad in ["", ".", "..", "../x", "a/b", "C:x", "x:y"] {
        assert_eq!(
            open_folder(&f.store, bad).err(),
            Some(Unusable::Unsafe),
            "{bad}"
        );
    }
    assert_eq!(
        open_folder(&f.store, "gibt-es-nicht").err(),
        Some(Unusable::Missing)
    );
}

// -- C: Metadaten ------------------------------------------------------------

fn assert_untouched(f: &Fixture, err: &str, expected: &str) {
    assert_eq!(err, expected);
    assert!(f.mic.exists() && f.system.exists() && f.dir.join("mic_aec.wav").exists());
    assert!(trim_dirs(&f.dir).is_empty(), "der neue Ordner ist weg");
    assert_eq!(f.store.segment_epoch(&f.id).unwrap(), 0);
    assert_eq!(current_mic(f), f.mic);
}

#[test]
fn unreadable_metadata_aborts_the_trim_and_changes_nothing() {
    for broken in ["kaputt{", "", "[1,2]", "\"text\"", "5", "null"] {
        let f = fixture();
        set_metadata(&f, broken);
        let err = trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap_err();
        assert_untouched(&f, &err, "metadata_unreadable");
        assert_eq!(
            metadata_raw(&f),
            broken,
            "die Metadaten bleiben, wie sie waren"
        );
    }
}

#[test]
fn missing_metadata_counts_as_an_empty_object() {
    let f = fixture();
    f.store
        .get_connection()
        .unwrap()
        .execute(
            "UPDATE meetings SET metadata_json = NULL WHERE id = ?1",
            params![f.id],
        )
        .unwrap();
    let result = trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();
    assert_eq!(result.duration_ms, 4_000);
    assert!(f.mic.exists(), "die alte Datei bleibt");
    assert!(!superseded(&f).is_empty());
}

#[test]
fn the_transaction_itself_refuses_unreadable_metadata_too() {
    let f = fixture();
    set_metadata(&f, "kaputt{");
    let new_dir = f.dir.join(ulid_dir('j'));
    std::fs::create_dir_all(&new_dir).unwrap();
    let err = f
        .store
        .apply_trim(&TrimWrite {
            meeting_id: &f.id,
            trim_id: &ulid_of('j'),
            old_mic: f.mic.to_str(),
            old_system: f.system.to_str(),
            old_aec: None,
            new_mic: None,
            new_system: None,
            duration_ms: 1,
            range: range(2_000, 6_000),
            aec_kept: false,
        })
        .unwrap_err();
    assert_eq!(err, "metadata_unreadable");
    assert_eq!(current_mic(&f), f.mic);
    assert_eq!(f.store.segment_epoch(&f.id).unwrap(), 0);
}

// -- D: angehaltene Neu-Transkription -------------------------------------------

#[test]
fn an_open_rerun_marker_blocks_the_trim_in_every_phase() {
    for phase in [
        rerun_state::PHASE_START,
        rerun_state::PHASE_RUNNING,
        rerun_state::PHASE_ROLLBACK,
        rerun_state::PHASE_KEPT,
    ] {
        let f = fixture();
        set_metadata(
            &f,
            &serde_json::json!({
                "aec": {"file": "mic_aec.wav", "wav_kept": true},
                rerun_state::KEY: {"phase": phase, "model": "m"}
            })
            .to_string(),
        );
        let err = trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap_err();
        assert_untouched(&f, &err, "rerun_pending");
    }
}

#[test]
fn a_finished_run_leaves_a_null_marker_and_does_not_block() {
    let f = fixture();
    rerun_state::write(&f.store, &f.id, Value::Null).unwrap();
    trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap();
}

#[test]
fn an_open_working_copy_blocks_the_trim_even_without_a_marker() {
    let f = fixture();
    {
        let mut c = f.store.get_connection().unwrap();
        crate::managers::meetings::variants::begin_rerun(&mut c, &f.id).unwrap();
    }
    let err = trim_meeting(&f.store, &f.id, range(2_000, 6_000), &lock(&f.id)).unwrap_err();
    assert_untouched(&f, &err, "rerun_pending");
}

#[test]
fn the_transaction_refuses_a_marker_that_appeared_after_the_precheck() {
    let f = fixture();
    // Wie ein Lauf, der zwischen Vorabpruefung und Commit eine Marke schreibt.
    rerun_state::write(
        &f.store,
        &f.id,
        serde_json::json!({"phase": "kept", "model": "m", "variant_id": "v"}),
    )
    .unwrap();
    let err = f
        .store
        .apply_trim(&TrimWrite {
            meeting_id: &f.id,
            trim_id: &ulid_of('j'),
            old_mic: f.mic.to_str(),
            old_system: f.system.to_str(),
            old_aec: None,
            new_mic: None,
            new_system: None,
            duration_ms: 1,
            range: range(2_000, 6_000),
            aec_kept: false,
        })
        .unwrap_err();
    assert_eq!(err, "rerun_pending");
    assert_eq!(f.store.segment_epoch(&f.id).unwrap(), 0);
    assert_eq!(current_mic(&f), f.mic);
}
