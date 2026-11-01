//! Der eine Filterbaustein, den alles hier benutzt.
//!
//! Ein Biquad ist ein Filter zweiter Ordnung: zwei Verzögerungen im Vorwärts-
//! und zwei im Rückwärtszweig. Damit lassen sich Hoch-, Tief- und
//! Kuhschwanzfilter bauen — die Bauform ist dieselbe, nur die fünf
//! Koeffizienten unterscheiden sich.
//!
//! Zwei Nutzer: die K-Gewichtung der Lautheitsmessung ([`super::loudness`])
//! bringt ihre Koeffizienten aus der Norm mit, die Klangbearbeitung
//! ([`super::enhance`]) rechnet sie sich aus Frequenz und Güte aus.

/// Transponierte Direktform II — die numerisch gutmütige Bauform und die
/// übliche Wahl für f64-Verarbeitung.
#[derive(Debug, Clone)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// Aus fertigen, bereits auf `a0` normierten Koeffizienten.
    pub fn new(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Hochpass zweiter Ordnung nach der Audio-EQ-Cookbook-Formel.
    ///
    /// `q = 0.7071` ergibt den Butterworth-Verlauf: so flach wie möglich im
    /// Durchlassbereich, ohne Überhöhung an der Grenzfrequenz.
    pub fn highpass(freq_hz: f64, q: f64, sample_rate: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * freq_hz / sample_rate;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self::new(
            ((1.0 + cos_w0) / 2.0) / a0,
            (-(1.0 + cos_w0)) / a0,
            ((1.0 + cos_w0) / 2.0) / a0,
            (-2.0 * cos_w0) / a0,
            (1.0 - alpha) / a0,
        )
    }

    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Streckt ein Signal per linearer Interpolation: `factor` 1,15 macht es 15 %
/// laenger und damit hoerbar tiefer — Tonhoehe UND Formanten sinken zusammen,
/// die klassische "tiefer und aelter"-Methode.
///
/// Bewusst linear und nicht bandbegrenzt: das Ergebnis ist die REFERENZ, aus
/// der das Modell anschliessend neu synthetisiert. Was an Interpolationsrauschen
/// entsteht, ueberlebt diesen Schritt nicht — ein teurerer Resampler brachte
/// hier nichts ausser Rechenzeit.
///
/// `factor <= 1.0` gibt eine unveraenderte Kopie: der Regler senkt nur.
pub fn resample_stretch(samples: &[f32], factor: f32) -> Vec<f32> {
    if samples.is_empty() || !(factor > 1.0) || !factor.is_finite() {
        return samples.to_vec();
    }
    let out_len = ((samples.len() as f32) * factor).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        // Position im Original: rueckwaerts gerechnet, damit der erste Wert
        // exakt der erste bleibt.
        let pos = (i as f32) / factor;
        let left = pos.floor() as usize;
        if left + 1 >= samples.len() {
            out.push(samples[samples.len() - 1]);
            continue;
        }
        let frac = pos - (left as f32);
        out.push(samples[left] * (1.0 - frac) + samples[left + 1] * frac);
    }
    out
}

/// Tempo ändern, ohne die Tonhöhe zu verschieben (WSOLA, Waveform
/// Similarity Overlap-Add). `speed` 1,5 spielt 1,5-mal so schnell, die Stimme
/// klingt dabei gleich — anders als beim Resampling, das die Stimme hoch
/// (schnell) oder tief (langsam) zieht.
///
/// `samples` ist verschachtelt (`channels` je Frame). Die Ähnlichkeitssuche
/// läuft auf der Monosumme; alle Kanäle werden an denselben Stellen
/// geschnitten, damit das Stereobild steht. Rahmen 25 ms, halbe Überlappung
/// mit Hann-Fenster (summiert sich zu 1), Suchbereich ±10 ms — erst grob,
/// dann fein, damit auch lange Sätze in wenigen Millisekunden fertig sind.
///
/// `speed` nahe 1, ein nicht endlicher Wert oder zu wenig Signal geben eine
/// unveränderte Kopie zurück.
pub fn time_stretch(samples: &[f32], channels: u16, sample_rate: u32, speed: f32) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if !speed.is_finite() || (speed - 1.0).abs() < 0.01 || sample_rate == 0 {
        return samples.to_vec();
    }
    let speed = speed.clamp(0.25, 4.0) as f64;
    let frames = samples.len() / ch;
    let n = ((sample_rate as f64 * 0.025) as usize).max(16) & !1;
    let hs = n / 2;
    let tol = ((sample_rate as f64 * 0.010) as usize).max(4);
    if frames < n * 2 {
        return samples.to_vec();
    }
    let mono: Vec<f32> = if ch == 1 {
        samples.to_vec()
    } else {
        samples
            .chunks_exact(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect()
    };
    let window: Vec<f32> = (0..n)
        .map(|i| {
            let x = std::f32::consts::PI * 2.0 * i as f32 / n as f32;
            0.5 - 0.5 * x.cos()
        })
        .collect();
    let out_frames = (frames as f64 / speed).round() as usize + n;
    let mut out = vec![0.0f32; out_frames * ch];
    let corr = |a: usize, b: usize, step: usize| -> f32 {
        let mut sum = 0.0f32;
        let mut i = 0;
        while i < n {
            sum += mono[a + i] * mono[b + i];
            i += step;
        }
        sum
    };
    // Erster Rahmen unverändert; danach je Rahmen die Stelle suchen, die am
    // besten an die natürliche Fortsetzung des vorigen Rahmens anschließt.
    let mut prev = 0usize;
    let mut k = 0usize;
    loop {
        let out_at = k * hs;
        if out_at + n > out_frames {
            break;
        }
        let pos = if k == 0 {
            0
        } else {
            let nominal = (k as f64 * hs as f64 * speed).round() as usize;
            let natural = prev + hs;
            if natural + n > frames || nominal + n > frames {
                break;
            }
            let lo = nominal.saturating_sub(tol);
            let hi = (nominal + tol).min(frames - n);
            let mut best = nominal.min(hi);
            let mut best_c = f32::MIN;
            let mut cand = lo;
            while cand <= hi {
                let c = corr(cand, natural, 4);
                if c > best_c {
                    best_c = c;
                    best = cand;
                }
                cand += 4;
            }
            let (flo, fhi) = (best.saturating_sub(3).max(lo), (best + 3).min(hi));
            best_c = f32::MIN;
            for cand in flo..=fhi {
                let c = corr(cand, natural, 2);
                if c > best_c {
                    best_c = c;
                    best = cand;
                }
            }
            best
        };
        for (i, &w) in window.iter().enumerate() {
            let src = (pos + i) * ch;
            let dst = (out_at + i) * ch;
            for c in 0..ch {
                out[dst + c] += samples[src + c] * w;
            }
        }
        prev = pos;
        k += 1;
    }
    // Der erste halbe Rahmen hat keinen Vorgänger, der das Fenster auffüllt.
    for (i, &w) in window.iter().enumerate().take(hs.min(out_frames)) {
        let w = w.max(1e-3);
        for c in 0..ch {
            out[i * ch + c] /= w;
        }
    }
    // Ebenso der letzte: ohne Nachfolger klänge er aus. Flach an beiden
    // Rändern lassen sich gestreckte Stücke lückenlos aneinanderhängen.
    let used = (k.saturating_sub(1) * hs + n).min(out_frames);
    if k > 0 {
        let last = (k - 1) * hs;
        for (i, &w) in window.iter().enumerate().skip(hs) {
            let at = last + i;
            if at >= used {
                break;
            }
            let w = w.max(1e-3);
            for c in 0..ch {
                out[at * ch + c] /= w;
            }
        }
    }
    out.truncate(used * ch);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    /// Nulldurchgänge je Sekunde: das Doppelte der Frequenz.
    fn crossings_per_sec(s: &[f32], rate: u32) -> f32 {
        let n = s
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        n as f32 * rate as f32 / s.len() as f32
    }

    #[test]
    fn tempo_aendert_die_laenge_aber_nicht_die_tonhoehe() {
        let rate = 24_000;
        let input = sine(220.0, rate, 2.0);
        for speed in [0.5f32, 0.75, 1.25, 1.5, 2.0] {
            let out = time_stretch(&input, 1, rate, speed);
            let want = input.len() as f32 / speed;
            let ratio = out.len() as f32 / want;
            assert!((0.95..1.05).contains(&ratio), "Länge bei {speed}: {ratio}");
            // Mitte prüfen (Ränder sind gefenstert).
            let mid = &out[out.len() / 4..out.len() * 3 / 4];
            let f = crossings_per_sec(mid, rate) / 2.0;
            assert!((f - 220.0).abs() < 6.0, "Tonhöhe bei {speed}: {f} Hz");
        }
    }

    #[test]
    fn tempo_eins_und_kurzes_signal_bleiben_unveraendert() {
        let input = sine(300.0, 16_000, 0.5);
        assert_eq!(time_stretch(&input, 1, 16_000, 1.0), input);
        let short = vec![0.1f32; 100];
        assert_eq!(time_stretch(&short, 1, 16_000, 1.5), short);
        assert_eq!(time_stretch(&input, 1, 16_000, f32::NAN), input);
    }

    #[test]
    fn tempo_stereo_bleibt_verschachtelt() {
        let rate = 16_000;
        let mono = sine(250.0, rate, 1.0);
        let stereo: Vec<f32> = mono.iter().flat_map(|&x| [x, -x]).collect();
        let out = time_stretch(&stereo, 2, rate, 1.5);
        assert_eq!(out.len() % 2, 0);
        for f in out.chunks_exact(2).skip(400).take(2000) {
            assert!((f[0] + f[1]).abs() < 1e-5, "Kanäle auseinander: {f:?}");
        }
    }

    /// Effektivwert eines Sinus nach dem Filter, bezogen auf den davor —
    /// misst die Dämpfung des Filters bei dieser Frequenz.
    fn attenuation_db(freq: f32, cutoff: f64, rate: f64) -> f64 {
        let n = (rate * 2.0) as usize;
        let mut filter = Biquad::highpass(cutoff, std::f64::consts::FRAC_1_SQRT_2, rate);
        let mut sum_in = 0.0;
        let mut sum_out = 0.0;
        for i in 0..n {
            let t = i as f64 / rate;
            let x = (2.0 * std::f64::consts::PI * freq as f64 * t).sin();
            let y = filter.process(x);
            // Die erste Zehntelsekunde ist Einschwingen und zaehlt nicht mit.
            if i > (rate / 10.0) as usize {
                sum_in += x * x;
                sum_out += y * y;
            }
        }
        10.0 * (sum_out / sum_in).log10()
    }

    /// An der Grenzfrequenz daempft ein Butterworth-Hochpass um 3 dB — der
    /// Pruefstein dafuer, dass die Koeffizienten stimmen.
    #[test]
    fn der_hochpass_daempft_an_der_grenzfrequenz_um_drei_dezibel() {
        let db = attenuation_db(80.0, 80.0, 48_000.0);
        assert!((db - (-3.01)).abs() < 0.3, "gemessen {db} dB");
    }

    /// Weit unterhalb wird stark gedaempft, weit oberhalb bleibt alles.
    #[test]
    fn tiefes_wird_gedaempft_hohes_bleibt() {
        let tief = attenuation_db(20.0, 80.0, 48_000.0);
        let hoch = attenuation_db(1000.0, 80.0, 48_000.0);
        assert!(tief < -20.0, "20 Hz nur {tief} dB gedaempft");
        assert!(hoch.abs() < 0.2, "1 kHz um {hoch} dB veraendert");
    }

    #[test]
    fn strecken_verlaengert_um_den_faktor() {
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = resample_stretch(&input, 1.15);
        // 100 Eingabewerte, 15 Prozent laenger: 115 Ausgabewerte.
        assert_eq!(out.len(), 115);
    }

    #[test]
    fn faktor_eins_und_kleiner_laesst_das_signal_unveraendert() {
        let input: Vec<f32> = vec![0.0, 0.5, -0.5, 1.0];
        assert_eq!(resample_stretch(&input, 1.0), input);
        assert_eq!(resample_stretch(&input, 0.5), input);
    }

    #[test]
    fn gestreckte_rampe_bleibt_monoton_und_haelt_die_raender() {
        // Eine Rampe ist der einfachste Fall mit pruefbarer Zwischenstufe:
        // linear interpoliert bleibt sie eine Rampe.
        let input: Vec<f32> = (0..50).map(|i| i as f32).collect();
        let out = resample_stretch(&input, 1.2);
        assert_eq!(out[0], 0.0, "der erste Wert bleibt der erste");
        assert!(
            out.windows(2).all(|w| w[1] >= w[0]),
            "eine gestreckte Rampe darf nirgends fallen"
        );
        assert!(
            *out.last().unwrap() <= 49.0,
            "es wird interpoliert, nicht extrapoliert"
        );
    }

    #[test]
    fn leeres_signal_bleibt_leer() {
        assert!(resample_stretch(&[], 1.15).is_empty());
    }
}
