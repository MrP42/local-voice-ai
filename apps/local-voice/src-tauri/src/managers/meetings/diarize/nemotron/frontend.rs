//! Mel-Vorstufe des Modells (Paket N1), gleich dem Merkmalsextraktor der
//! Referenz (`NemotronAsrStreamingFeatureExtractor`, transformers, Apache-2.0)
//! im Offline-Aufruf (`center = True`).
//!
//! Schritte: Preemphase `y[0] = x[0]`, `y[n] = x[n] - 0,97 * x[n-1]`; STFT mit
//! FFT 512, symmetrischem Hann-Fenster der Laenge 400 (mittig auf 512 aufgefuellt),
//! Schritt 160, Rand mit Nullen aufgefuellt (Rahmen `t` ist um Sample `160 * t`
//! zentriert); Leistungsspektrum; 128 Slaney-Mel-Baender (wie
//! `librosa.filters.mel(norm="slaney")`); `ln(Mel + 2^-24)`. Keine Normierung.
//!
//! Gueltige Rahmen: `floor(Samples / 160)`. Die Referenz liefert einen Rahmen
//! mehr, den ihre Maske auf 0 setzt; [`MelFrontend::rows`] liefert Zeilen ab
//! `num_frames` ebenfalls als 0.
//!
//! Speicher: die Zeilen werden je Block berechnet ([`MelFrontend::rows`]), nie
//! fuer die ganze Datei auf einmal.

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use super::MEL_BINS;

pub const SAMPLE_RATE: usize = 16_000;
pub const N_FFT: usize = 512;
pub const WIN_LENGTH: usize = 400;
pub const HOP: usize = 160;
const PREEMPHASIS: f32 = 0.97;
/// `2^-24`: Untergrenze vor dem Logarithmus (`LOG_ZERO_GUARD_VALUE`).
const LOG_GUARD: f32 = 5.960_464_5e-8;
const FREQ_BINS: usize = N_FFT / 2 + 1;
/// Links freier Rand des Fensters in der FFT-Eingabe: `(512 - 400) / 2`.
const WINDOW_OFFSET: usize = (N_FFT - WIN_LENGTH) / 2;

/// Gueltige Mel-Rahmen fuer `samples` Abtastwerte (`center = True`).
pub fn num_frames(samples: usize) -> usize {
    samples / HOP
}

/// Ein Mel-Band: Gewichte ab FFT-Bin `first`.
struct Band {
    first: usize,
    weights: Vec<f32>,
}

pub struct MelFrontend {
    window: Vec<f32>,
    bands: Vec<Band>,
    fft: Arc<dyn Fft<f32>>,
}

impl Default for MelFrontend {
    fn default() -> Self {
        Self::new()
    }
}

fn hz_to_mel(hz: f64) -> f64 {
    const F_SP: f64 = 200.0 / 3.0;
    const MIN_LOG_HZ: f64 = 1000.0;
    if hz >= MIN_LOG_HZ {
        let log_step = 6.4_f64.ln() / 27.0;
        MIN_LOG_HZ / F_SP + (hz / MIN_LOG_HZ).ln() / log_step
    } else {
        hz / F_SP
    }
}

fn mel_to_hz(mel: f64) -> f64 {
    const F_SP: f64 = 200.0 / 3.0;
    const MIN_LOG_HZ: f64 = 1000.0;
    const MIN_LOG_MEL: f64 = MIN_LOG_HZ / F_SP;
    if mel >= MIN_LOG_MEL {
        let log_step = 6.4_f64.ln() / 27.0;
        MIN_LOG_HZ * (log_step * (mel - MIN_LOG_MEL)).exp()
    } else {
        mel * F_SP
    }
}

/// Slaney-Mel-Dreiecke wie `librosa.filters.mel(norm="slaney")` fuer 0 Hz bis
/// Nyquist; in `f64` gerechnet, als `f32` gespeichert.
fn mel_bands() -> Vec<Band> {
    let mel_max = hz_to_mel(SAMPLE_RATE as f64 / 2.0);
    let points: Vec<f64> = (0..MEL_BINS + 2)
        .map(|i| mel_to_hz(mel_max * i as f64 / (MEL_BINS + 1) as f64))
        .collect();
    let bin_hz = |bin: usize| bin as f64 * (SAMPLE_RATE as f64 / 2.0) / (FREQ_BINS - 1) as f64;
    (0..MEL_BINS)
        .map(|m| {
            let (lo, mid, hi) = (points[m], points[m + 1], points[m + 2]);
            let norm = 2.0 / (hi - lo);
            let weight = |bin: usize| {
                let f = bin_hz(bin);
                let lower = (f - lo) / (mid - lo);
                let upper = (hi - f) / (hi - mid);
                lower.min(upper).max(0.0) * norm
            };
            let first = (0..FREQ_BINS).find(|&b| weight(b) > 0.0).unwrap_or(0);
            let last = (0..FREQ_BINS)
                .rev()
                .find(|&b| weight(b) > 0.0)
                .unwrap_or(first);
            Band {
                first,
                weights: (first..=last).map(|b| weight(b) as f32).collect(),
            }
        })
        .collect()
}

/// Symmetrisches Hann-Fenster (`torch.hann_window(periodic=False)`).
fn hann_window() -> Vec<f32> {
    (0..WIN_LENGTH)
        .map(|n| {
            let phase = 2.0 * std::f64::consts::PI * n as f64 / (WIN_LENGTH - 1) as f64;
            (0.5 - 0.5 * phase.cos()) as f32
        })
        .collect()
}

impl MelFrontend {
    pub fn new() -> Self {
        Self {
            window: hann_window(),
            bands: mel_bands(),
            fft: FftPlanner::<f32>::new().plan_fft_forward(N_FFT),
        }
    }

    /// Zeilen `first .. first + count` der Mel-Matrix (`Zeile * 128` Werte,
    /// angehaengt an `out`). Zeilen ab [`num_frames`] sind 0.
    pub fn rows(&self, pcm: &[f32], first: usize, count: usize, out: &mut Vec<f32>) {
        let valid = num_frames(pcm.len());
        let mut buf = vec![Complex32::new(0.0, 0.0); N_FFT];
        let mut scratch = vec![Complex32::new(0.0, 0.0); self.fft.get_inplace_scratch_len()];
        let mut power = [0f32; FREQ_BINS];
        out.reserve(count * MEL_BINS);
        for row in first..first + count {
            if row >= valid {
                out.extend(std::iter::repeat(0.0).take(MEL_BINS));
                continue;
            }
            // Fenster [160 * row - 200, 160 * row + 200) im Signal.
            let origin = (row * HOP) as isize - (WIN_LENGTH / 2) as isize;
            for (k, slot) in buf.iter_mut().enumerate() {
                let v = if (WINDOW_OFFSET..WINDOW_OFFSET + WIN_LENGTH).contains(&k) {
                    let j = k - WINDOW_OFFSET;
                    preemphasised(pcm, origin + j as isize) * self.window[j]
                } else {
                    0.0
                };
                *slot = Complex32::new(v, 0.0);
            }
            self.fft.process_with_scratch(&mut buf, &mut scratch);
            for (p, c) in power.iter_mut().zip(buf.iter()) {
                *p = c.re * c.re + c.im * c.im;
            }
            for band in &self.bands {
                let spectrum = &power[band.first..band.first + band.weights.len()];
                let energy: f32 = band.weights.iter().zip(spectrum).map(|(w, p)| w * p).sum();
                out.push((energy + LOG_GUARD).ln());
            }
        }
    }

    /// Alle gueltigen Zeilen (`num_frames(pcm.len()) * 128` Werte).
    pub fn compute(&self, pcm: &[f32]) -> Vec<f32> {
        let mut out = Vec::new();
        self.rows(pcm, 0, num_frames(pcm.len()), &mut out);
        out
    }
}

/// `y[pos]` der Preemphase; ausserhalb des Signals 0 (Nullrand der STFT).
fn preemphasised(pcm: &[f32], pos: isize) -> f32 {
    if pos < 0 || pos as usize >= pcm.len() {
        return 0.0;
    }
    let i = pos as usize;
    if i == 0 {
        pcm[0]
    } else {
        pcm[i] - PREEMPHASIS * pcm[i - 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1,2 s: 0,2 s digitale Stille, dann 1,0 s Sprache (16-bit PCM aus
    /// `m2_echo_mic.wav`), die Referenz-Merkmale aus transformers 5.22.2
    /// (`koordination/nvidia-speech/tools/make_reference.py`).
    const CLIP: &[u8] = include_bytes!("../../../../../tests/fixtures/nemotron/mel_clip.i16");
    const REF: &[u8] = include_bytes!("../../../../../tests/fixtures/nemotron/mel_ref.f32");

    fn clip() -> Vec<f32> {
        CLIP.chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect()
    }

    fn reference() -> Vec<f32> {
        REF.chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    #[test]
    fn frame_count_is_floor_of_samples_over_hop() {
        assert_eq!(num_frames(0), 0);
        assert_eq!(num_frames(159), 0);
        assert_eq!(num_frames(160), 1);
        assert_eq!(num_frames(32_000), 200);
        assert_eq!(num_frames(32_159), 200);
    }

    #[test]
    fn matches_the_transformers_features_on_a_speech_clip() {
        let pcm = clip();
        let got = MelFrontend::new().compute(&pcm);
        let want = reference();
        assert_eq!(pcm.len(), 19_200);
        assert_eq!(got.len(), want.len(), "Rahmenzahl");
        assert_eq!(got.len(), 120 * MEL_BINS);
        let mut worst = (0usize, 0f32);
        let mut sum = 0f64;
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            let d = (g - w).abs();
            sum += f64::from(d);
            if d > worst.1 {
                worst = (i, d);
            }
        }
        let mean = sum / got.len() as f64;
        assert!(
            worst.1 < 2e-3,
            "groesste Abweichung {} bei Rahmen {}, Band {}",
            worst.1,
            worst.0 / MEL_BINS,
            worst.0 % MEL_BINS
        );
        assert!(mean < 1e-4, "mittlere Abweichung {mean}");
    }

    #[test]
    fn digital_silence_is_the_log_floor_not_nan() {
        let got = MelFrontend::new().compute(&vec![0.0; 3_200]);
        assert_eq!(got.len(), 20 * MEL_BINS);
        let floor = LOG_GUARD.ln();
        assert!(
            got.iter().all(|v| (v - floor).abs() < 1e-6),
            "{:?}",
            &got[..4]
        );
    }

    #[test]
    fn rows_past_the_valid_frames_are_zero_and_blocks_equal_the_whole() {
        let pcm = clip();
        let front = MelFrontend::new();
        let whole = front.compute(&pcm);
        let valid = num_frames(pcm.len());
        // Block aus der Mitte bis ueber das Ende hinaus.
        let mut part = Vec::new();
        front.rows(&pcm, valid - 10, 13, &mut part);
        assert_eq!(part.len(), 13 * MEL_BINS);
        assert_eq!(&part[..10 * MEL_BINS], &whole[(valid - 10) * MEL_BINS..]);
        assert!(part[10 * MEL_BINS..].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn mel_filters_are_slaney_normalised() {
        // Jedes Dreieck hat die Flaeche 1 (Slaney: 2 / Breite): die Summe der
        // Gewichte mal Bin-Breite entspricht grob der Dreiecksflaeche.
        let bands = mel_bands();
        assert_eq!(bands.len(), MEL_BINS);
        assert!(bands.iter().all(|b| !b.weights.is_empty()));
        // Das unterste Band beginnt direkt ueber 0 Hz, das oberste endet vor Nyquist.
        assert_eq!(bands[0].first, 1);
        let last = bands.last().unwrap();
        assert!(last.first + last.weights.len() <= FREQ_BINS);
        let area: f32 = bands[64].weights.iter().sum::<f32>() * 31.25;
        assert!((area - 1.0).abs() < 0.1, "{area}");
    }
}
