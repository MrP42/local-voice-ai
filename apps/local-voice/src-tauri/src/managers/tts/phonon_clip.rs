//! Das Stück Sprache, das zum Erstellen einer Phonon-Stimme an Gradium geht.
//!
//! Gradium nutzt laut `scripts/clone-voice.py` (gradium-ai/xn-ptts) „10 seconds of clear speech
//! from `--start` on“. Statt die ganze Datei zu senden, schneidet die App genau dieses Fenster
//! selbst aus: Was die Oberfläche als „ca. 10 s“ ankündigt, ist auch das, was den Rechner
//! verlässt (Datensparsamkeit; der Dateiname verlässt den Rechner ebenfalls nicht). Das Fenster
//! geht als Mono-WAV (16 Bit, Abtastrate der Quelle) hinaus; die Startsekunde ist danach 0.
//!
//! Gelesen wird nur das Fenster (`seek`), nie die ganze Datei. Stille oder ein zu kurzes Stück
//! scheitern HIER, vor dem Netzzugriff — ein bezahlter Aufruf für nichts wäre die schlechtere
//! Rückmeldung.

use std::fmt;
use std::io::Cursor;
use std::path::Path;

/// So viele Sekunden nutzt Gradium ab der Startsekunde.
pub const CLIP_SECONDS: u32 = 10;
/// Kürzer taugt nichts (Klick, Räuspern); lieber vor dem Senden melden.
pub const MIN_CLIP_SECONDS: f32 = 3.0;
/// Spitzenpegel unter diesem Wert gilt als Stille.
const SILENCE_PEAK: f32 = 0.003;
/// Obergrenze der Startsekunde (zehn Stunden).
const MAX_START_S: f32 = 36_000.0;

/// Das ausgeschnittene Stück als WAV-Bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub wav: Vec<u8>,
    pub seconds: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClipError {
    /// Startsekunde unbrauchbar (negativ, keine Zahl, absurd groß).
    BadStart,
    /// Die Startsekunde liegt hinter dem Ende der Aufnahme.
    PastEnd { length_s: f32 },
    /// Ab der Startsekunde bleibt weniger als [`MIN_CLIP_SECONDS`].
    TooShort { available_s: f32 },
    /// Kein hörbarer Pegel.
    Silent,
    /// Die Datei ist keine lesbare WAV-Datei.
    BadWav(String),
}

impl fmt::Display for ClipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadStart => write!(f, "Die Startsekunde muss eine Zahl von 0 aufwärts sein."),
            Self::PastEnd { length_s } => write!(
                f,
                "Die Startsekunde liegt hinter dem Ende der Aufnahme (Länge {length_s:.1} s)."
            ),
            Self::TooShort { available_s } => write!(
                f,
                "Ab der Startsekunde bleiben nur {available_s:.1} s; für eine brauchbare Stimme \
                 werden mindestens {MIN_CLIP_SECONDS:.0} s klare Sprache gebraucht (besser {CLIP_SECONDS} s)."
            ),
            Self::Silent => write!(
                f,
                "Im gewählten Ausschnitt ist keine Sprache zu hören (zu leise oder Stille)."
            ),
            Self::BadWav(why) => write!(f, "Die Audiodatei ist nicht lesbar: {why}"),
        }
    }
}

impl std::error::Error for ClipError {}

fn check_start(start_s: f32) -> Result<(), ClipError> {
    if start_s.is_finite() && (0.0..=MAX_START_S).contains(&start_s) {
        Ok(())
    } else {
        Err(ClipError::BadStart)
    }
}

/// Ausschnitt aus Mono-Samples (z. B. einer Mikrofonaufnahme mit 16 kHz).
pub fn clip_from_samples(samples: &[f32], rate: u32, start_s: f32) -> Result<Clip, ClipError> {
    check_start(start_s)?;
    if rate == 0 {
        return Err(ClipError::BadWav("Abtastrate 0".to_string()));
    }
    let start = (start_s as f64 * rate as f64) as usize;
    if start >= samples.len() {
        return Err(ClipError::PastEnd {
            length_s: samples.len() as f32 / rate as f32,
        });
    }
    let end = (start + CLIP_SECONDS as usize * rate as usize).min(samples.len());
    finish(&samples[start..end], rate)
}

/// Ausschnitt aus einer WAV-Datei (jede Kanalzahl, 8 bis 32 Bit oder Float). Liest nur das
/// Fenster.
pub fn clip_from_wav_file(path: &Path, start_s: f32) -> Result<Clip, ClipError> {
    check_start(start_s)?;
    let bad = |e: hound::Error| ClipError::BadWav(e.to_string());
    let mut reader = hound::WavReader::open(path).map_err(bad)?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let rate = spec.sample_rate;
    if rate == 0 {
        return Err(ClipError::BadWav("Abtastrate 0".to_string()));
    }
    let total = reader.duration();
    let start = (start_s as f64 * rate as f64) as u64;
    if start >= total as u64 {
        return Err(ClipError::PastEnd {
            length_s: total as f32 / rate as f32,
        });
    }
    let start = start as u32;
    reader
        .seek(start)
        .map_err(|e| ClipError::BadWav(e.to_string()))?;
    let frames = (CLIP_SECONDS as u64 * rate as u64).min((total - start) as u64) as usize;
    let want = frames * channels;
    let interleaved: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .take(want)
            .collect::<Result<_, _>>()
            .map_err(bad)?,
        (hound::SampleFormat::Int, bits @ 1..=32) => {
            let scale = (1i64 << (bits - 1)) as f32;
            reader
                .samples::<i32>()
                .take(want)
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
                .map_err(bad)?
        }
        (format, bits) => {
            return Err(ClipError::BadWav(format!(
                "Format {format:?} mit {bits} Bit wird nicht unterstützt"
            )))
        }
    };
    let mono: Vec<f32> = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    finish(&mono, rate)
}

fn finish(mono: &[f32], rate: u32) -> Result<Clip, ClipError> {
    let seconds = mono.len() as f32 / rate as f32;
    if seconds < MIN_CLIP_SECONDS {
        return Err(ClipError::TooShort {
            available_s: seconds,
        });
    }
    let peak = mono.iter().fold(0.0f32, |p, s| p.max(s.abs()));
    if peak < SILENCE_PEAK {
        return Err(ClipError::Silent);
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Vec::with_capacity(mono.len() * 2 + 64);
    {
        let mut writer = hound::WavWriter::new(Cursor::new(&mut buf), spec)
            .map_err(|e| ClipError::BadWav(e.to_string()))?;
        for &s in mono {
            writer
                .write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .map_err(|e| ClipError::BadWav(e.to_string()))?;
        }
        writer
            .finalize()
            .map_err(|e| ClipError::BadWav(e.to_string()))?;
    }
    Ok(Clip { wav: buf, seconds })
}

#[cfg(test)]
mod tests;
