use super::*;

fn sine(rate: u32, seconds: f32, freq: f32) -> Vec<f32> {
    (0..(rate as f32 * seconds) as usize)
        .map(|i| 0.5 * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
        .collect()
}

fn read_back(wav: &[u8]) -> (hound::WavSpec, Vec<i16>) {
    let mut r = hound::WavReader::new(Cursor::new(wav)).unwrap();
    let spec = r.spec();
    (spec, r.samples::<i16>().map(|s| s.unwrap()).collect())
}

fn write_wav(path: &Path, spec: hound::WavSpec, frames: usize, f: impl Fn(usize, u16) -> f32) {
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..frames {
        for ch in 0..spec.channels {
            let v = f(i, ch).clamp(-1.0, 1.0);
            match spec.sample_format {
                hound::SampleFormat::Float => w.write_sample(v).unwrap(),
                hound::SampleFormat::Int => w
                    .write_sample((v * ((1i64 << (spec.bits_per_sample - 1)) - 1) as f32) as i32)
                    .unwrap(),
            }
        }
    }
    w.finalize().unwrap();
}

/// Das Fenster beginnt an der Startsekunde und ist genau zehn Sekunden lang; mehr verlässt den
/// Rechner nicht.
#[test]
fn the_window_starts_at_the_start_second_and_is_ten_seconds_long() {
    // Rampe: der Wert verrät die Position (1 pro Sekunde).
    let rate = 16_000;
    let samples: Vec<f32> = (0..25 * rate as usize)
        .map(|i| 0.01 + 0.9 * (i as f32 / rate as f32) / 25.0)
        .collect();
    let clip = clip_from_samples(&samples, rate, 5.0).unwrap();
    let (spec, back) = read_back(&clip.wav);
    assert_eq!(
        (spec.channels, spec.sample_rate, spec.bits_per_sample),
        (1, 16_000, 16)
    );
    assert_eq!(back.len(), 10 * rate as usize);
    let first = back[0] as f32 / i16::MAX as f32;
    let expect = 0.01 + 0.9 * 5.0 / 25.0;
    assert!((first - expect).abs() < 0.001, "{first} vs {expect}");
    assert!((clip.seconds - 10.0).abs() < 1e-3);
}

#[test]
fn a_shorter_remainder_is_kept_but_never_padded() {
    let samples = sine(16_000, 6.0, 220.0);
    let clip = clip_from_samples(&samples, 16_000, 0.0).unwrap();
    assert_eq!(read_back(&clip.wav).1.len(), 6 * 16_000);
}

#[test]
fn too_short_past_the_end_and_silence_fail_before_any_upload() {
    let short = sine(16_000, 2.0, 220.0);
    assert!(matches!(
        clip_from_samples(&short, 16_000, 0.0),
        Err(ClipError::TooShort { .. })
    ));
    // Ab Sekunde 8 eines 10-Sekunden-Stücks bleiben 2 s.
    let ten = sine(16_000, 10.0, 220.0);
    assert!(matches!(
        clip_from_samples(&ten, 16_000, 8.0),
        Err(ClipError::TooShort { .. })
    ));
    assert!(matches!(
        clip_from_samples(&ten, 16_000, 10.0),
        Err(ClipError::PastEnd { .. })
    ));
    let silence = vec![0.0f32; 16_000 * 8];
    assert_eq!(
        clip_from_samples(&silence, 16_000, 0.0),
        Err(ClipError::Silent)
    );
    let hiss = vec![0.001f32; 16_000 * 8];
    assert_eq!(
        clip_from_samples(&hiss, 16_000, 0.0),
        Err(ClipError::Silent)
    );
}

#[test]
fn an_unusable_start_second_is_refused() {
    let s = sine(16_000, 12.0, 220.0);
    for bad in [-1.0, f32::NAN, f32::INFINITY, 1.0e9] {
        assert_eq!(clip_from_samples(&s, 16_000, bad), Err(ClipError::BadStart));
    }
}

/// Stereo mit 48 kHz: Mono-Mischung, Abtastrate der Quelle bleibt (kein Qualitätsverlust).
#[test]
fn stereo_is_mixed_down_and_the_source_rate_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("stereo.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    // links 0.4, rechts 0.2 -> Mittel 0.3
    write_wav(
        &p,
        spec,
        48_000 * 12,
        |_, ch| if ch == 0 { 0.4 } else { 0.2 },
    );
    let clip = clip_from_wav_file(&p, 0.0).unwrap();
    let (spec, back) = read_back(&clip.wav);
    assert_eq!((spec.channels, spec.sample_rate), (1, 48_000));
    assert_eq!(back.len(), 10 * 48_000);
    let v = back[1000] as f32 / i16::MAX as f32;
    assert!((v - 0.3).abs() < 0.002, "{v}");
}

/// Aus einer langen Datei wird nur das Fenster gelesen und an der richtigen Stelle.
#[test]
fn a_long_file_is_read_from_the_start_second_only() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("lang.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    // Wert = Sekunde / 100 (30 s lang).
    write_wav(&p, spec, 8_000 * 30, |i, _| {
        0.02 + (i / 8_000) as f32 / 100.0
    });
    let clip = clip_from_wav_file(&p, 12.5).unwrap();
    let (_, back) = read_back(&clip.wav);
    assert_eq!(back.len(), 10 * 8_000);
    let first = back[0] as f32 / i16::MAX as f32;
    assert!((first - (0.02 + 0.12)).abs() < 0.002, "{first}");
    let last = *back.last().unwrap() as f32 / i16::MAX as f32;
    assert!((last - (0.02 + 0.22)).abs() < 0.002, "{last}");
}

#[test]
fn float_and_24_bit_files_are_understood() {
    let dir = tempfile::tempdir().unwrap();
    for (name, spec) in [
        (
            "f32.wav",
            hound::WavSpec {
                channels: 1,
                sample_rate: 22_050,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        ),
        (
            "i24.wav",
            hound::WavSpec {
                channels: 1,
                sample_rate: 22_050,
                bits_per_sample: 24,
                sample_format: hound::SampleFormat::Int,
            },
        ),
    ] {
        let p = dir.path().join(name);
        write_wav(&p, spec, 22_050 * 5, |i, _| 0.5 * (i as f32 * 0.05).sin());
        let clip = clip_from_wav_file(&p, 0.0).unwrap();
        assert!((clip.seconds - 5.0).abs() < 1e-3, "{name}");
    }
}

#[test]
fn a_file_that_is_no_wav_and_a_start_past_the_end_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("kaputt.wav");
    std::fs::write(&junk, b"das ist keine wav-datei, nur text").unwrap();
    assert!(matches!(
        clip_from_wav_file(&junk, 0.0),
        Err(ClipError::BadWav(_))
    ));
    assert!(matches!(
        clip_from_wav_file(&dir.path().join("fehlt.wav"), 0.0),
        Err(ClipError::BadWav(_))
    ));
    let p = dir.path().join("kurz.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    write_wav(&p, spec, 8_000 * 5, |i, _| 0.3 * (i as f32 * 0.1).sin());
    assert!(matches!(
        clip_from_wav_file(&p, 6.0),
        Err(ClipError::PastEnd { .. })
    ));
}
