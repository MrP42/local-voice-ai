//! Sprechtempo ändern, ohne dass die Stimme leidet (Issue #314).
//!
//! Tonhöhensynchrones Überspringen bzw. Einfügen ganzer Stimmperioden, das
//! Prinzip von PICOLA und Sonic (Bill Cox, Apache-2.0; hier eigenständig
//! umgesetzt): Die Grundperiode wird per normierter Korrelation gesucht, genau EINE Periode
//! wird mit einer linearen Überblendung entfernt (schneller) oder verdoppelt
//! (langsamer), und alles bis zur nächsten Schnittstelle wird Sample für
//! Sample unverändert kopiert. Bei 1,5x bleibt so die Hälfte des Signals
//! unangetastet, und jede Überblendung verbindet zwei Stellen gleicher Phase.
//!
//! Die frühere WSOLA mit festen 25-ms-Rahmen schnitt quer durch die
//! Stimmperioden und richtete die Rahmen nach nicht normierter Korrelation
//! aus; die Phasenauslöschungen klangen robotisch.
//!
//! Die Länge stimmt exakt im Mittel (Bruchteil-Übertrag), damit Hervorhebung,
//! Spulen und die Hörstellen-Rechnung des Players synchron bleiben.

/// Tiefste und höchste erwartete Grundfrequenz einer Sprechstimme.
const MIN_PITCH_HZ: u32 = 65;
const MAX_PITCH_HZ: u32 = 400;

/// Zeitstrecker mit Zustand: Stücke eines Stroms lassen sich nacheinander
/// einspeisen, ohne dass an den Stückgrenzen Brüche entstehen.
pub struct SpeechStretcher {
    channels: usize,
    speed: f64,
    min_period: usize,
    max_period: usize,
    /// Noch nicht verarbeitete Eingabe (verschachtelt) und ihre Monosumme.
    input: Vec<f32>,
    mono: Vec<f32>,
    /// Erster noch nicht verbrauchter Frame in `input`.
    pos: usize,
    /// Frames, die noch unverändert kopiert werden (mit Bruchteil).
    to_copy: f64,
    /// Bruchteil der Überblendlänge bei Tempo ≥ 2 bzw. < 0,5.
    blend_carry: f64,
    out: Vec<f32>,
    /// Frames insgesamt eingespeist bzw. ausgegeben (für die exakte Länge).
    total_in: u64,
    total_out: u64,
}

impl SpeechStretcher {
    pub fn new(channels: u16, sample_rate: u32, speed: f32) -> Self {
        let rate = sample_rate.max(8_000);
        let speed = if speed.is_finite() {
            (speed as f64).clamp(0.25, 4.0)
        } else {
            1.0
        };
        Self {
            channels: channels.max(1) as usize,
            speed,
            min_period: (rate / MAX_PITCH_HZ) as usize,
            max_period: (rate / MIN_PITCH_HZ) as usize,
            input: Vec::new(),
            mono: Vec::new(),
            pos: 0,
            to_copy: 0.0,
            blend_carry: 0.0,
            out: Vec::new(),
            total_in: 0,
            total_out: 0,
        }
    }

    fn passthrough(&self) -> bool {
        (self.speed - 1.0).abs() < 0.01
    }

    /// Eingabe anhängen und alles zurückgeben, was schon fertig ist.
    pub fn push(&mut self, samples: &[f32]) -> Vec<f32> {
        if self.passthrough() {
            return samples.to_vec();
        }
        let ch = self.channels;
        let whole = samples.len() - samples.len() % ch;
        self.input.extend_from_slice(&samples[..whole]);
        self.mono.extend(
            samples[..whole]
                .chunks_exact(ch)
                .map(|f| f.iter().sum::<f32>() / ch as f32),
        );
        self.total_in += (whole / ch) as u64;
        self.process();
        self.compact();
        self.take_out()
    }

    fn take_out(&mut self) -> Vec<f32> {
        self.total_out += (self.out.len() / self.channels) as u64;
        std::mem::take(&mut self.out)
    }

    /// Rest ausgeben. Was für eine weitere Schnittstelle zu kurz ist (unter
    /// zwei Stimmperioden, also höchstens rund 30 ms), wird so gekürzt bzw.
    /// gedehnt, dass die Gesamtlänge stimmt.
    pub fn flush(&mut self) -> Vec<f32> {
        if self.passthrough() {
            return Vec::new();
        }
        let ch = self.channels;
        let rest = self.frames() - self.pos;
        let target = (self.total_in as f64 / self.speed).round() as u64;
        let want = target.saturating_sub(self.total_out) as usize;
        let take = want.min(rest);
        self.out
            .extend_from_slice(&self.input[self.pos * ch..(self.pos + take) * ch]);
        // Langsamer: der Rest reicht nicht für die Wunschlänge, Stille füllt
        // höchstens Millisekunden auf und hält die Uhr synchron.
        if want > take {
            let last = self.out.len().saturating_sub(ch);
            let fade = self.out.len() > ch;
            for i in 0..(want - take) {
                for c in 0..ch {
                    let v = if fade {
                        self.out[last + c] * (1.0 - (i + 1) as f32 / (want - take) as f32)
                    } else {
                        0.0
                    };
                    self.out.push(v);
                }
            }
        }
        self.input.clear();
        self.mono.clear();
        self.pos = 0;
        self.to_copy = 0.0;
        self.blend_carry = 0.0;
        let out = self.take_out();
        self.total_in = 0;
        self.total_out = 0;
        out
    }

    fn frames(&self) -> usize {
        self.mono.len()
    }

    fn copy_frames(&mut self, n: usize) {
        let ch = self.channels;
        self.out
            .extend_from_slice(&self.input[self.pos * ch..(self.pos + n) * ch]);
        self.pos += n;
    }

    fn process(&mut self) {
        let needed = 2 * self.max_period + 1;
        loop {
            let avail = self.frames() - self.pos;
            if self.to_copy >= 1.0 {
                let n = (self.to_copy.floor() as usize).min(avail);
                if n == 0 {
                    return;
                }
                self.copy_frames(n);
                self.to_copy -= n as f64;
                if self.to_copy >= 1.0 {
                    return;
                }
                continue;
            }
            if avail < needed {
                return;
            }
            let period = self.find_period(self.pos);
            if self.speed > 1.0 {
                self.skip_period(period);
            } else {
                self.insert_period(period);
            }
        }
    }

    /// Länge der Überblendung mit Bruchteil-Übertrag.
    fn blend_len(&mut self, exact: f64, period: usize) -> usize {
        let total = exact + self.blend_carry;
        let n = (total.floor() as usize).clamp(1, period);
        self.blend_carry = total - n as f64;
        n
    }

    /// Schneller: eine Periode entfällt. Ausgabe = Überblendung von „hier“
    /// nach „eine Periode weiter“, danach unverändert weiter.
    fn skip_period(&mut self, period: usize) {
        let s = self.speed;
        let blend = if s >= 2.0 {
            self.blend_len(period as f64 / (s - 1.0), period)
        } else {
            self.to_copy += period as f64 * (2.0 - s) / (s - 1.0);
            period
        };
        let (a, b) = (self.pos, self.pos + period);
        self.overlap_add(blend, a, b);
        self.pos += period + blend;
    }

    /// Langsamer: eine Periode kommt doppelt. Erst die Periode, dann eine
    /// Überblendung zurück an ihren Anfang, danach unverändert weiter.
    fn insert_period(&mut self, period: usize) {
        let s = self.speed;
        let blend = if s < 0.5 {
            self.blend_len(period as f64 * s / (1.0 - s), period)
        } else {
            self.to_copy += period as f64 * (2.0 * s - 1.0) / (1.0 - s);
            period
        };
        let ch = self.channels;
        self.out
            .extend_from_slice(&self.input[self.pos * ch..(self.pos + period) * ch]);
        let (a, b) = (self.pos + period, self.pos);
        self.overlap_add(blend, a, b);
        self.pos += blend;
    }

    /// `n` Frames: `down` (ab Frame `a`) blendet aus, `up` (ab `b`) ein.
    fn overlap_add(&mut self, n: usize, a: usize, b: usize) {
        let ch = self.channels;
        for i in 0..n {
            let up = (i as f32 + 0.5) / n as f32;
            let down = 1.0 - up;
            for c in 0..ch {
                let v = self.input[(a + i) * ch + c] * down + self.input[(b + i) * ch + c] * up;
                self.out.push(v);
            }
        }
    }

    /// Grundperiode ab `at`: die kürzeste Verschiebung, deren normierte
    /// Korrelation mit der folgenden Periode nahe am Bestwert liegt. Auf
    /// voller Rate gerechnet — eine grobe Suche verfehlt hohe Stimmen, und
    /// jede schlecht ausgerichtete Überblendung ist als Rauheit hörbar. In
    /// stimmlosen Lauten passt keine Periode gut; dort ist das Überblenden
    /// unhörbar.
    fn find_period(&self, at: usize) -> usize {
        best_period(&self.mono[at..], self.min_period, self.max_period)
    }

    /// Verbrauchte Eingabe vorn abschneiden, damit der Puffer nicht wächst.
    fn compact(&mut self) {
        if self.pos > 4 * self.max_period {
            let ch = self.channels;
            self.input.drain(..self.pos * ch);
            self.mono.drain(..self.pos);
            self.pos = 0;
        }
    }
}

/// Periode in `[lo, hi]` mit der höchsten normierten Korrelation zwischen
/// `x[..p]` und `x[p..2p]`. Vielfache der Grundperiode passen fast genauso
/// gut; deshalb gewinnt die kürzeste Periode, die ein lokales Maximum ist
/// und mindestens 95 % des Bestwerts erreicht (sonst fielen ganze
/// Silbenteile weg).
fn best_period(x: &[f32], lo: usize, hi: usize) -> usize {
    let hi = hi.min(x.len() / 2);
    if hi <= lo {
        return lo.max(1);
    }
    let ncc: Vec<f32> = (lo..=hi)
        .map(|p| {
            let (mut ab, mut aa, mut bb) = (0f32, 0f32, 0f32);
            for i in 0..p {
                let (u, v) = (x[i], x[i + p]);
                ab += u * v;
                aa += u * u;
                bb += v * v;
            }
            if aa <= 1e-12 || bb <= 1e-12 {
                0.0
            } else {
                ab / (aa * bb).sqrt()
            }
        })
        .collect();
    let max = ncc.iter().copied().fold(f32::MIN, f32::max);
    let limit = max * 0.95;
    let local_max =
        |i: usize| (i == 0 || ncc[i] >= ncc[i - 1]) && (i + 1 == ncc.len() || ncc[i] >= ncc[i + 1]);
    (0..ncc.len())
        .find(|&i| ncc[i] >= limit && local_max(i))
        .map_or(lo, |i| lo + i)
}

/// Ein ganzes Stück auf einmal strecken.
pub fn stretch_all(samples: &[f32], channels: u16, sample_rate: u32, speed: f32) -> Vec<f32> {
    let mut s = SpeechStretcher::new(channels, sample_rate, speed);
    let mut out = s.push(samples);
    out.extend(s.flush());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stimmähnliches Signal: Pulsfolge (Glottis) mit langsam gleitender
    /// Grundfrequenz durch ein einfaches Formantfilter.
    fn voice(f0: f32, rate: u32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        let mut phase = 0.0f32;
        let mut y1 = 0.0f32;
        let mut y2 = 0.0f32;
        let r = 0.97f32;
        let w = 2.0 * std::f32::consts::PI * 700.0 / rate as f32;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let f = f0 * (1.0 + 0.08 * (2.0 * std::f32::consts::PI * 3.0 * t).sin());
                phase += f / rate as f32;
                let pulse = if phase >= 1.0 {
                    phase -= 1.0;
                    1.0
                } else {
                    0.0
                };
                let y = pulse + 2.0 * r * w.cos() * y1 - r * r * y2;
                y2 = y1;
                y1 = y;
                y * 0.05
            })
            .collect()
    }

    fn max_jump(s: &[f32]) -> f32 {
        s.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn laenge_stimmt_genau() {
        let rate = 24_000;
        let input = voice(110.0, rate, 3.0);
        for speed in [0.5f32, 0.75, 1.25, 1.5, 1.75, 2.0, 2.5] {
            let out = stretch_all(&input, 1, rate, speed);
            let want = input.len() as f32 / speed;
            let err = (out.len() as f32 - want).abs() / rate as f32;
            assert!(
                err < 0.005,
                "Länge bei {speed}: {} ms daneben",
                err * 1000.0
            );
        }
    }

    #[test]
    fn stream_in_kleinen_stuecken_gleicht_dem_ganzen() {
        let rate = 24_000;
        let input = voice(120.0, rate, 2.0);
        let whole = stretch_all(&input, 1, rate, 1.5);
        let mut s = SpeechStretcher::new(1, rate, 1.5);
        let mut parts = Vec::new();
        for chunk in input.chunks(480) {
            parts.extend(s.push(chunk));
        }
        parts.extend(s.flush());
        assert_eq!(parts.len(), whole.len());
        for (a, b) in parts.iter().zip(&whole) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn keine_spruenge_im_signal() {
        let rate = 24_000;
        let input = voice(100.0, rate, 2.0);
        let limit = max_jump(&input) * 1.05;
        for speed in [0.75f32, 1.5, 2.0] {
            let out = stretch_all(&input, 1, rate, speed);
            assert!(max_jump(&out) <= limit, "Sprung bei {speed}");
        }
    }

    #[test]
    fn grundperiode_wird_gefunden() {
        let rate = 24_000;
        for f0 in [80.0f32, 110.0, 180.0, 300.0] {
            let input: Vec<f32> = (0..rate as usize)
                .map(|i| {
                    let t = i as f32 / rate as f32;
                    (2.0 * std::f32::consts::PI * f0 * t).sin()
                        + 0.5 * (2.0 * std::f32::consts::PI * 2.0 * f0 * t).sin()
                })
                .collect();
            let mut s = SpeechStretcher::new(1, rate, 1.5);
            s.mono = input;
            let p = s.find_period(1000);
            let want = rate as f32 / f0;
            assert!((p as f32 - want).abs() <= 1.5, "f0 {f0}: {p} statt {want}");
        }
    }

    #[test]
    fn tempo_eins_ist_bitgenau() {
        let input = voice(150.0, 16_000, 0.5);
        assert_eq!(stretch_all(&input, 1, 16_000, 1.0), input);
    }
}
