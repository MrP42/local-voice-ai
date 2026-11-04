//! Kaldi-Filterbank (80 Mel-Bänder, 25 ms / 10 ms) wie `torchaudio.compliance.kaldi.fbank`
//! mit `dither = 0`: Eingang des Stimmabdruck-Modells (3D-Speaker CAM++).
//!
//! Schritte je Rahmen: Gleichanteil abziehen, Preemphase 0,97, Povey-Fenster,
//! FFT 512, Leistungsspektrum, Mel-Dreiecke (20 Hz bis Nyquist), Logarithmus.
//! Danach zieht [`subtract_mean`] den Mittelwert je Band ab (`global-mean`).

use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;

pub const SAMPLE_RATE: usize = 16_000;
pub const NUM_BINS: usize = 80;
const FRAME_LEN: usize = 400; // 25 ms
const FRAME_SHIFT: usize = 160; // 10 ms
const FFT_LEN: usize = 512;
const PREEMPH: f32 = 0.97;
const LOW_FREQ: f32 = 20.0;
const EPS: f32 = f32::EPSILON;

fn mel(freq: f32) -> f32 {
    1127.0 * (1.0 + freq / 700.0).ln()
}

/// Rahmenzahl mit `snip_edges = true`: nur ganze Rahmen.
pub fn num_frames(samples: usize) -> usize {
    if samples < FRAME_LEN {
        0
    } else {
        1 + (samples - FRAME_LEN) / FRAME_SHIFT
    }
}

/// Mel-Gewichte je Band über den FFT-Bins `0..FFT_LEN/2` (Kaldi `MelBanks`).
fn mel_banks() -> Vec<Vec<(usize, f32)>> {
    let fft_bins = FFT_LEN / 2;
    let bin_width = SAMPLE_RATE as f32 / FFT_LEN as f32;
    let mel_low = mel(LOW_FREQ);
    let mel_high = mel(SAMPLE_RATE as f32 / 2.0);
    let delta = (mel_high - mel_low) / (NUM_BINS as f32 + 1.0);
    (0..NUM_BINS)
        .map(|b| {
            let left = mel_low + b as f32 * delta;
            let center = left + delta;
            let right = center + delta;
            (0..fft_bins)
                .filter_map(|i| {
                    let m = mel(bin_width * i as f32);
                    if m <= left || m >= right {
                        return None;
                    }
                    let w = if m <= center {
                        (m - left) / (center - left)
                    } else {
                        (right - m) / (right - center)
                    };
                    Some((i, w))
                })
                .collect()
        })
        .collect()
}

fn povey_window() -> Vec<f32> {
    let a = 2.0 * std::f64::consts::PI / (FRAME_LEN - 1) as f64;
    (0..FRAME_LEN)
        .map(|i| (0.5 - 0.5 * (a * i as f64).cos()).powf(0.85) as f32)
        .collect()
}

/// Filterbank für `pcm` (16 kHz, Werte in [-1, 1]): `num_frames × NUM_BINS`,
/// zeilenweise.
pub fn compute(pcm: &[f32]) -> Vec<f32> {
    let frames = num_frames(pcm.len());
    let mut out = Vec::with_capacity(frames * NUM_BINS);
    if frames == 0 {
        return out;
    }
    let banks = mel_banks();
    let window = povey_window();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_LEN);
    let mut frame = vec![0f32; FRAME_LEN];
    let mut buf = vec![Complex32::new(0.0, 0.0); FFT_LEN];
    let mut power = vec![0f32; FFT_LEN / 2 + 1];
    for f in 0..frames {
        let start = f * FRAME_SHIFT;
        frame.copy_from_slice(&pcm[start..start + FRAME_LEN]);
        let mean = frame.iter().map(|&x| f64::from(x)).sum::<f64>() / FRAME_LEN as f64;
        for x in frame.iter_mut() {
            *x -= mean as f32;
        }
        for i in (1..FRAME_LEN).rev() {
            frame[i] -= PREEMPH * frame[i - 1];
        }
        frame[0] -= PREEMPH * frame[0];
        for (i, slot) in buf.iter_mut().enumerate() {
            let v = if i < FRAME_LEN {
                frame[i] * window[i]
            } else {
                0.0
            };
            *slot = Complex32::new(v, 0.0);
        }
        fft.process(&mut buf);
        for (p, c) in power.iter_mut().zip(buf.iter()) {
            *p = c.norm_sqr();
        }
        for bank in &banks {
            let e: f32 = bank.iter().map(|&(i, w)| w * power[i]).sum();
            out.push(e.max(EPS).ln());
        }
    }
    out
}

/// Mittelwert je Band über alle Rahmen abziehen (`global-mean`).
pub fn subtract_mean(feats: &mut [f32]) {
    let frames = feats.len() / NUM_BINS;
    if frames == 0 {
        return;
    }
    let mut mean = [0f64; NUM_BINS];
    for row in feats.chunks_exact(NUM_BINS) {
        for (m, &v) in mean.iter_mut().zip(row) {
            *m += f64::from(v);
        }
    }
    for m in mean.iter_mut() {
        *m /= frames as f64;
    }
    for row in feats.chunks_exact_mut(NUM_BINS) {
        for (v, &m) in row.iter_mut().zip(mean.iter()) {
            *v -= m as f32;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 0,5 s Testsignal: zwei Sinustöne (440 Hz, 1 kHz) mit fester Hüllkurve.
    pub(crate) fn test_signal() -> Vec<f32> {
        (0..8_000)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                let env = 0.5 + 0.5 * (2.0 * std::f32::consts::PI * 3.0 * t).sin();
                env * (0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
                    + 0.1 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin())
            })
            .collect()
    }

    #[test]
    fn frame_count_follows_snip_edges() {
        assert_eq!(num_frames(0), 0);
        assert_eq!(num_frames(399), 0);
        assert_eq!(num_frames(400), 1);
        assert_eq!(num_frames(559), 1);
        assert_eq!(num_frames(560), 2);
        assert_eq!(num_frames(16_000), 98);
    }

    /// Referenzwerte aus `kaldi_native_fbank` 1.22.5 (dither 0, 80 Bänder,
    /// high_freq 0, snip_edges) für dasselbe Signal.
    #[test]
    fn matches_kaldi_reference() {
        let feats = compute(&test_signal());
        assert_eq!(feats.len(), 48 * NUM_BINS);
        let at = |f: usize, b: usize| feats[f * NUM_BINS + b];
        for &(f, b, want) in super::reference::VALUES {
            let got = at(f, b);
            assert!(
                (got - want).abs() < 2e-3 * want.abs().max(1.0),
                "Rahmen {f}, Band {b}: {got} statt {want}"
            );
        }
    }

    #[test]
    fn silence_is_floored_not_nan() {
        let feats = compute(&vec![0.0; 1_600]);
        assert!(feats.iter().all(|v| v.is_finite()));
        assert!((feats[0] - EPS.ln()).abs() < 1e-4);
    }

    #[test]
    fn mean_subtraction_centres_every_band() {
        let mut feats = compute(&test_signal());
        subtract_mean(&mut feats);
        for b in 0..NUM_BINS {
            let m: f32 = feats.chunks_exact(NUM_BINS).map(|r| r[b]).sum::<f32>() / 48.0;
            assert!(m.abs() < 1e-3, "Band {b}: {m}");
        }
    }
}

#[cfg(test)]
#[path = "fbank_reference.rs"]
mod reference;
