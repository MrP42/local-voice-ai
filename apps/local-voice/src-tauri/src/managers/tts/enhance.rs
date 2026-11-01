//! Klangbearbeitung für Sprachaufnahmen: Hochpass, Rauschgatter, Kompressor,
//! Begrenzer — in dieser Reihenfolge, und die Reihenfolge ist keine Willkür.
//!
//! 1. **Hochpass.** Trittschall, Griffgeräusche und Netzbrummen liegen unter
//!    der Stimme. Sie zuerst zu entfernen heißt, dass alle folgenden Stufen
//!    sie nicht mehr für Signal halten — ein Gatter, das auf 50-Hz-Brummen
//!    reagiert, öffnet in jeder Pause.
//! 2. **Rauschgatter.** Was zwischen den Sätzen übrig bleibt, ist der
//!    Rauschteppich. Er wird *gemessen*, nicht geraten, und leise Stellen
//!    werden abgesenkt statt hart abgeschnitten.
//! 3. **Kompressor.** Erst jetzt lohnt es, laute und leise Stellen
//!    anzugleichen — vor dem Gatter würde er das Rauschen mit anheben.
//! 4. **Begrenzer.** Die letzte Instanz vor der Aussteuerungsgrenze.
//!
//! Warum das bei einer *Referenzaufnahme* am meisten bringt: Fish Speech
//! bildet die Stimme aus ihr nach — mitsamt Lüfterrauschen und Raumhall.
//! Eine verrauschte Referenz ergibt eine verrauschte Stimme, und die
//! hinterher zu säubern repariert nur, was vorne hineingeraten ist.
//!
//! Bewusst zurückhaltend eingestellt. Eine zu kräftige Kette klingt
//! atemlos und metallisch; das ist schlimmer als etwas Rauschen.

use super::dsp::Biquad;

/// Wie stark die Kette eingreift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Strength {
    /// Aufräumen, nichts formen. Für gute Aufnahmen.
    Gentle,
    /// Der Normalfall: hörbar ruhiger, ohne dass die Stimme sich ändert.
    Medium,
    /// Für hörbar verrauschte Aufnahmen. Kann die Stimme etwas verfärben.
    Strong,
}

impl Default for Strength {
    fn default() -> Self {
        Self::Gentle
    }
}

/// Die Zahlenwerte einer Stufe. Getrennt von der Verarbeitung, damit sich
/// nachlesen lässt, was „mittel" eigentlich bedeutet.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// Grenzfrequenz des Hochpasses in Hz. Männliche Grundtöne beginnen bei
    /// etwa 85 Hz — darüber darf nicht geschnitten werden.
    pub highpass_hz: f64,
    /// Wie weit über dem gemessenen Rauschteppich das Gatter öffnet (dB).
    pub gate_above_noise_db: f64,
    /// Wie stark geschlossen wird (dB). Nicht bis zur Stille: eine Pause, in
    /// der das Rauschen abrupt verschwindet, klingt kaputter als eine mit.
    pub gate_reduction_db: f64,
    /// Kompressionsverhältnis über der Schwelle (n:1).
    pub compress_ratio: f64,
    /// Schwelle des Kompressors, bezogen auf den gemessenen Sprachpegel (dB).
    pub compress_below_speech_db: f64,
}

impl Strength {
    pub fn settings(self) -> Settings {
        match self {
            Strength::Gentle => Settings {
                highpass_hz: 65.0,
                gate_above_noise_db: 6.0,
                gate_reduction_db: -8.0,
                compress_ratio: 2.0,
                compress_below_speech_db: -6.0,
            },
            Strength::Medium => Settings {
                highpass_hz: 80.0,
                gate_above_noise_db: 8.0,
                gate_reduction_db: -14.0,
                compress_ratio: 3.0,
                compress_below_speech_db: -9.0,
            },
            Strength::Strong => Settings {
                highpass_hz: 95.0,
                gate_above_noise_db: 10.0,
                gate_reduction_db: -20.0,
                compress_ratio: 4.0,
                compress_below_speech_db: -12.0,
            },
        }
    }
}

/// Fensterlänge der Pegelverfolgung: 20 ms. Kurz genug, um einem Satzanfang
/// zu folgen, lang genug, um nicht auf einzelne Schwingungen zu reagieren.
const FRAME_MS: f64 = 20.0;

/// Sicherheitsgrenze -1 dBFS, dieselbe wie beim Pegeln.
const PEAK_CEILING: f32 = 0.891;

/// Ein Signal durch die Kette schicken. Mono, Werte in -1..1.
///
/// Verändert `samples` an Ort und Stelle. Zu kurze Signale (unter drei
/// Fenstern) bleiben unangetastet: über sie lässt sich kein Rauschteppich
/// messen, und Raten wäre schlimmer als Nichtstun.
pub fn process(samples: &mut [f32], sample_rate: u32, strength: Strength) {
    let cfg = strength.settings();
    let rate = sample_rate as f64;
    let frame = ((FRAME_MS / 1000.0) * rate).round() as usize;
    if frame == 0 || samples.len() < frame * 3 {
        return;
    }

    // 1. Hochpass.
    let mut hp = Biquad::highpass(cfg.highpass_hz, std::f64::consts::FRAC_1_SQRT_2, rate);
    for s in samples.iter_mut() {
        *s = hp.process(*s as f64) as f32;
    }

    // Pegel je Fenster — Grundlage für Gatter und Kompressor.
    let levels: Vec<f64> = samples
        .chunks(frame)
        .map(|c| {
            let sum: f64 = c.iter().map(|v| (*v as f64) * (*v as f64)).sum();
            (sum / c.len() as f64).sqrt()
        })
        .collect();

    let Some(noise) = noise_floor(&levels) else {
        return;
    };
    let speech = speech_level(&levels);

    // Gegattet wird nur, wenn es ueberhaupt messbare Pausen gibt.
    //
    // Ohne diese Pruefung liegt der "Rauschteppich" einer durchgehend
    // gesprochenen Aufnahme auf Sprachniveau — und eine Schwelle darueber
    // schliesst das Gatter auf ALLES. Im Test verschwanden so 91 % des
    // Signals. Unter 12 dB Abstand zwischen Pause und Sprache gibt es nichts
    // zu gatten, und Raten waere schlimmer als Nichtstun.
    const MIN_GATE_RANGE_DB: f64 = 12.0;
    let dynamic_range_db = 20.0 * (speech / noise).log10();
    let gating = dynamic_range_db >= MIN_GATE_RANGE_DB;

    let gate_threshold = noise * db_to_ratio(cfg.gate_above_noise_db);
    let gate_floor = db_to_ratio(cfg.gate_reduction_db);
    let compress_threshold = speech * db_to_ratio(cfg.compress_below_speech_db);

    // 2. + 3. Gatter und Kompressor als EIN Verstärkungsverlauf je Fenster,
    // danach geglättet. Zwei getrennte Durchgänge klängen identisch, kosteten
    // aber zwei Glättungen und damit zwei Quellen für Pumpen.
    let mut targets: Vec<f32> = Vec::with_capacity(levels.len());
    for &level in &levels {
        let mut gain = 1.0f64;
        if gating && level < gate_threshold {
            // Weicher Übergang statt harter Kante: zwischen Rauschteppich und
            // Schwelle wird anteilig abgesenkt. Ein hartes Gatter schneidet
            // Wortenden ab, und das hoert man sofort.
            let span = (gate_threshold - noise).max(f64::EPSILON);
            let openness = ((level - noise) / span).clamp(0.0, 1.0);
            gain *= gate_floor + (1.0 - gate_floor) * openness;
        }
        if level > compress_threshold && compress_threshold > 0.0 {
            let over_db = 20.0 * (level / compress_threshold).log10();
            let allowed_db = over_db / cfg.compress_ratio;
            gain *= db_to_ratio(allowed_db - over_db);
        }
        targets.push(gain as f32);
    }
    smooth(&mut targets);

    // Verstärkung linear zwischen den Fenstermitten interpolieren, sonst
    // entstehen an jeder Fenstergrenze hoerbare Stufen.
    apply_gain_curve(samples, &targets, frame);

    // Aufholverstärkung: die Kette muss pegelneutral sein.
    //
    // Ein Kompressor senkt — ohne Ausgleich klang die Stimme hinterher um
    // 6 dB leiser, und "verbessert" hiesse dann schlicht "leiser". Gemessen
    // wird derselbe Sprachpegel wie vorher (80-%-Quantil) und darauf
    // zurueckgezogen. Weil das die PAUSEN mit anhebt, bleibt der Abstand
    // zwischen Sprache und Pause genau der, den das Gatter erzeugt hat —
    // die Wirkung des Gatters geht also nicht verloren, nur die Absenkung
    // der Stimme.
    let after: Vec<f64> = samples
        .chunks(frame)
        .map(|c| {
            let sum: f64 = c.iter().map(|v| (*v as f64) * (*v as f64)).sum();
            (sum / c.len() as f64).sqrt()
        })
        .collect();
    let speech_after = speech_level(&after);
    if speech_after > 1e-9 {
        let makeup = (speech / speech_after) as f32;
        for s in samples.iter_mut() {
            *s *= makeup;
        }
    }

    // 4. Begrenzer: eine einzelne Spitze darf nicht ins Clipping laufen.
    let peak = samples.iter().fold(0.0f32, |a, s| a.max(s.abs()));
    if peak > PEAK_CEILING {
        let scale = PEAK_CEILING / peak;
        for s in samples.iter_mut() {
            *s *= scale;
        }
    }
}

fn db_to_ratio(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Der Rauschteppich: das 10-%-Quantil der Fensterpegel.
///
/// Nicht das Minimum — ein einzelnes stummes Fenster (eine Lücke in der
/// Datei, ein Schnitt) läge bei null und machte die Schwelle unbrauchbar.
/// Das Quantil trifft die typische Pause.
fn noise_floor(levels: &[f64]) -> Option<f64> {
    if levels.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = levels.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = (sorted.len() as f64 * 0.10) as usize;
    let value = sorted[idx.min(sorted.len() - 1)];
    // Digitale Stille: es gibt nichts zu gatten.
    (value > 1e-6).then_some(value)
}

/// Der Sprachpegel: das 80-%-Quantil. Die lautesten 20 % sind Betonungen und
/// Zischlaute; sie als „normal" zu nehmen setzte die Schwelle zu hoch.
fn speech_level(levels: &[f64]) -> f64 {
    let mut sorted: Vec<f64> = levels.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((sorted.len() as f64 * 0.80) as usize).min(sorted.len() - 1);
    sorted[idx]
}

/// Die Verstärkungskurve glätten: schnell öffnen (5 ms Ansprechzeit, ein
/// Wortanfang darf nicht verschluckt werden), langsam schließen (150 ms
/// Rückstellzeit, sonst pumpt es hörbar zwischen den Wörtern).
///
/// Bei 20-ms-Fenstern heißt das: aufwärts sofort, abwärts in Schritten.
fn smooth(gains: &mut [f32]) {
    const RELEASE_PER_FRAME: f32 = 0.15;
    for i in 1..gains.len() {
        if gains[i] < gains[i - 1] {
            let limited = gains[i - 1] - RELEASE_PER_FRAME;
            gains[i] = gains[i].max(limited);
        }
    }
    // Rückwärts: vor einem lauten Fenster schon öffnen, damit der Einsatz
    // nicht abgeschnitten wird (das ist die Ansprechzeit).
    for i in (0..gains.len().saturating_sub(1)).rev() {
        if gains[i] < gains[i + 1] {
            gains[i] = gains[i].max(gains[i + 1] - 0.5);
        }
    }
}

/// Verstärkungswerte je Fenster auf die Samples anwenden, linear zwischen den
/// Fenstermitten interpoliert.
fn apply_gain_curve(samples: &mut [f32], gains: &[f32], frame: usize) {
    if gains.is_empty() {
        return;
    }
    let half = frame as f32 / 2.0;
    for (i, s) in samples.iter_mut().enumerate() {
        let pos = (i as f32 - half) / frame as f32;
        let lower = pos.floor();
        let frac = pos - lower;
        let a = gains[(lower.max(0.0) as usize).min(gains.len() - 1)];
        let b = gains[((lower + 1.0).max(0.0) as usize).min(gains.len() - 1)];
        *s *= a + (b - a) * frac;
    }
}

/// Länge der Ein- und Ausblendung an den Rändern: 8 ms.
///
/// Kurz genug, dass kein Laut verlorengeht — ein Konsonant dauert 30 bis
/// 100 ms —, lang genug, um einen Sprung unhörbar zu machen.
const EDGE_FADE_MS: f64 = 8.0;

/// Ränder entschärfen: Gleichspannung entfernen, Anfang und Ende sanft
/// ein- und ausblenden.
///
/// Warum das nötig ist: ein Tonstück, das bei einem Wert ungleich null
/// beginnt, ist für den Lautsprecher ein Sprung — und ein Sprung ist ein
/// Knacken. Beim Vorlesen ist jeder Satz ein eigenes Tonstück, also gibt es
/// diese Stelle bei jedem Satz. Am deutlichsten bei der Standardstimme, die
/// bis v0.8.7 ohne Referenz erzeugt wurde und deshalb bei jedem Satz an
/// einer anderen Stelle der Schwingung begann.
///
/// Zusätzlich wird der Gleichanteil abgezogen: ein Tonstück, dessen Mittelwert
/// nicht null ist, springt schon beim ersten Sample — und beim Übergang zum
/// nächsten Satz noch einmal.
///
/// Läuft IMMER, unabhängig von der Klangbearbeitung: ein Knacken ist kein
/// Geschmacksfrage, und niemand schaltet eine Verbesserung ab, um es zu
/// bekommen.
pub fn soften_edges(samples: &mut [f32], channels: usize, sample_rate: u32) {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    if frames == 0 {
        return;
    }

    // Gleichanteil je Kanal abziehen.
    for channel in 0..channels {
        let mut sum = 0.0f64;
        for frame in 0..frames {
            sum += samples[frame * channels + channel] as f64;
        }
        let offset = (sum / frames as f64) as f32;
        if offset.abs() > 1e-5 {
            for frame in 0..frames {
                samples[frame * channels + channel] -= offset;
            }
        }
    }

    // Erhobener Kosinus statt einer Geraden: die Steigung ist an beiden
    // Enden null, der Übergang also auch in der Ableitung stetig. Eine
    // lineare Blende hat an ihrem Beginn einen Knick, und Knicke hört man.
    let fade = (((EDGE_FADE_MS / 1000.0) * sample_rate as f64) as usize).min(frames / 2);
    if fade == 0 {
        return;
    }
    for i in 0..fade {
        let phase = std::f64::consts::PI * i as f64 / fade as f64;
        let factor = (0.5 - 0.5 * phase.cos()) as f32;
        for channel in 0..channels {
            samples[i * channels + channel] *= factor;
            let tail = (frames - 1 - i) * channels + channel;
            samples[tail] *= factor;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples
            .iter()
            .map(|v| (*v as f64) * (*v as f64))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt()
    }

    /// Eine Sekunde „Sprache" (Ton) gefolgt von einer Sekunde Rauschen —
    /// das Muster jeder Aufnahme mit Pausen.
    ///
    /// Die Toene werden ueber 5 ms ein- und ausgeblendet. Das ist keine
    /// Kosmetik: ein Ton, der bei voller Amplitude hart auf null springt, ist
    /// ein Sprung, und an einem Sprung klingelt JEDER Hochpass aus. Gemessen
    /// am 21.08.2026 hob dieser Einschwinger das erste Pausenfenster um 20 dB
    /// und beherrschte damit die Messung des ganzen Blocks — ein Artefakt des
    /// Testsignals, das in keiner echten Aufnahme vorkommt.
    fn speech_then_noise(rate: u32) -> Vec<f32> {
        let n = rate as usize;
        let fade = (rate as f32 * 0.005) as usize;
        let mut out = Vec::with_capacity(n * 4);
        let mut seed = 12345u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            ((seed >> 8) as f32 / 8_388_608.0 - 1.0) * 0.01
        };
        for block in 0..4 {
            for i in 0..n {
                let t = i as f32 / rate as f32;
                let sample = if block % 2 == 0 {
                    let envelope = (i as f32 / fade as f32)
                        .min((n - 1 - i) as f32 / fade as f32)
                        .clamp(0.0, 1.0);
                    0.3 * envelope * (2.0 * std::f32::consts::PI * 200.0 * t).sin()
                } else {
                    0.0
                };
                out.push(sample + noise());
            }
        }
        out
    }

    /// Der Knacks-Test: ein Tonstueck, das mitten in der Schwingung beginnt
    /// und endet. Danach muessen beide Raender bei praktisch null liegen.
    #[test]
    fn die_raender_beginnen_und_enden_bei_null() {
        let rate = 16_000u32;
        // Kosinus ohne Phasenverschiebung: beginnt beim Maximum, also im
        // schlimmstmoeglichen Fall — genau dort knackt es am lautesten.
        let mut samples: Vec<f32> = (0..rate as usize)
            .map(|i| {
                let t = i as f32 / rate as f32;
                0.8 * (2.0 * std::f32::consts::PI * 200.0 * t).cos()
            })
            .collect();
        assert!(
            samples[0].abs() > 0.7,
            "Testsignal beginnt nicht am Maximum"
        );
        soften_edges(&mut samples, 1, rate);
        assert!(samples[0].abs() < 0.01, "Anfang bei {}", samples[0]);
        assert!(
            samples[samples.len() - 1].abs() < 0.01,
            "Ende bei {}",
            samples[samples.len() - 1]
        );
    }

    /// Die Mitte bleibt unangetastet — geblendet wird nur an den Raendern.
    #[test]
    fn die_mitte_bleibt_unveraendert() {
        let rate = 16_000u32;
        let original: Vec<f32> = (0..rate as usize)
            .map(|i| {
                let t = i as f32 / rate as f32;
                0.5 * (2.0 * std::f32::consts::PI * 200.0 * t).sin()
            })
            .collect();
        let mut samples = original.clone();
        soften_edges(&mut samples, 1, rate);
        let middle = rate as usize / 2;
        assert!(
            (samples[middle] - original[middle]).abs() < 1e-4,
            "Mitte veraendert: {} statt {}",
            samples[middle],
            original[middle]
        );
    }

    /// Ein Gleichanteil ist ein Sprung schon beim ersten Sample.
    #[test]
    fn der_gleichanteil_wird_entfernt() {
        let rate = 16_000u32;
        let mut samples: Vec<f32> = (0..rate as usize)
            .map(|i| {
                let t = i as f32 / rate as f32;
                0.3 * (2.0 * std::f32::consts::PI * 200.0 * t).sin() + 0.2
            })
            .collect();
        soften_edges(&mut samples, 1, rate);
        let mean: f64 = samples.iter().map(|v| *v as f64).sum::<f64>() / samples.len() as f64;
        assert!(mean.abs() < 0.01, "Gleichanteil blieb: {mean}");
    }

    /// Bei Stereo muss jeder Kanal fuer sich behandelt werden.
    #[test]
    fn stereo_wird_kanalweise_behandelt() {
        let rate = 16_000u32;
        let frames = rate as usize;
        let mut samples: Vec<f32> = (0..frames * 2)
            .map(|i| if i % 2 == 0 { 0.8 } else { -0.6 })
            .collect();
        soften_edges(&mut samples, 2, rate);
        assert!(samples[0].abs() < 0.01, "links: {}", samples[0]);
        assert!(samples[1].abs() < 0.01, "rechts: {}", samples[1]);
    }

    #[test]
    fn ein_leeres_signal_bringt_nichts_zum_absturz() {
        let mut leer: Vec<f32> = Vec::new();
        soften_edges(&mut leer, 1, 16_000);
        let mut winzig = vec![0.5f32; 3];
        soften_edges(&mut winzig, 1, 16_000);
    }

    #[test]
    fn das_rauschen_in_den_pausen_wird_leiser() {
        let rate = 16_000;
        let mut samples = speech_then_noise(rate);
        let pause_before = rms(&samples[rate as usize..2 * rate as usize]);
        process(&mut samples, rate, Strength::Medium);
        let pause_after = rms(&samples[rate as usize..2 * rate as usize]);
        let reduction_db = 20.0 * (pause_after / pause_before).log10();
        assert!(reduction_db < -6.0, "Pause nur um {reduction_db} dB leiser");
    }

    #[test]
    fn die_sprache_selbst_bleibt_erhalten() {
        let rate = 16_000;
        let mut samples = speech_then_noise(rate);
        let speech_before = rms(&samples[..rate as usize]);
        process(&mut samples, rate, Strength::Medium);
        let speech_after = rms(&samples[..rate as usize]);
        let change_db = 20.0 * (speech_after / speech_before).log10();
        assert!(
            change_db.abs() < 3.0,
            "Sprache um {change_db} dB veraendert — zu viel"
        );
    }

    /// Staerker heisst staerker: die Stufen muessen sich unterscheiden, sonst
    /// ist die Einstellung eine Attrappe.
    #[test]
    fn die_stufen_wirken_unterschiedlich_stark() {
        let rate = 16_000;
        let pause = |strength| {
            let mut s = speech_then_noise(rate);
            process(&mut s, rate, strength);
            rms(&s[rate as usize..2 * rate as usize])
        };
        let gentle = pause(Strength::Gentle);
        let medium = pause(Strength::Medium);
        let strong = pause(Strength::Strong);
        assert!(
            medium < gentle,
            "mittel ({medium}) nicht unter sanft ({gentle})"
        );
        assert!(
            strong < medium,
            "stark ({strong}) nicht unter mittel ({medium})"
        );
    }

    /// Tieffrequentes Brummen unter der Stimme muss verschwinden.
    #[test]
    fn netzbrummen_wird_entfernt() {
        let rate = 16_000;
        let n = rate as usize * 3;
        let mut samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                0.3 * (2.0 * std::f32::consts::PI * 200.0 * t).sin()
                    + 0.2 * (2.0 * std::f32::consts::PI * 50.0 * t).sin()
            })
            .collect();
        let before = rms(&samples);
        process(&mut samples, rate, Strength::Medium);
        let after = rms(&samples);
        // Das Brummen traegt rund ein Drittel der Leistung; ohne es muss der
        // Effektivwert messbar sinken, ohne dass die Stimme verschwindet.
        assert!(after < before * 0.95, "Brummen blieb: {before} -> {after}");
        assert!(
            after > before * 0.4,
            "zu viel entfernt: {before} -> {after}"
        );
    }

    #[test]
    fn zu_kurze_und_stille_signale_bleiben_unveraendert() {
        let mut kurz = vec![0.5f32; 100];
        let kopie = kurz.clone();
        process(&mut kurz, 16_000, Strength::Medium);
        assert_eq!(kurz, kopie, "zu kurzes Signal wurde veraendert");

        let mut still = vec![0.0f32; 16_000];
        process(&mut still, 16_000, Strength::Medium);
        assert!(still.iter().all(|s| s.abs() < 1e-6));
    }

    /// Nach der Kette darf nichts ueber der Aussteuerungsgrenze liegen.
    #[test]
    fn der_begrenzer_haelt_die_aussteuerungsgrenze() {
        let rate = 16_000;
        let n = rate as usize * 3;
        let mut samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                0.95 * (2.0 * std::f32::consts::PI * 200.0 * t).sin()
            })
            .collect();
        process(&mut samples, rate, Strength::Strong);
        let peak = samples.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak <= PEAK_CEILING + 1e-6, "Spitze {peak}");
    }
}
