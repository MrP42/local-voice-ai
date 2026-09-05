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

#[cfg(test)]
mod tests {
    use super::*;

    /// Effektivwert eines Sinus nach dem Filter, bezogen auf den davor —
    /// misst die Dämpfung des Filters bei dieser Frequenz.
    fn attenuation_db(freq: f32, cutoff: f64, rate: f64) -> f64 {
        let n = (rate * 2.0) as usize;
        let mut filter = Biquad::highpass(cutoff, 0.7071, rate);
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
