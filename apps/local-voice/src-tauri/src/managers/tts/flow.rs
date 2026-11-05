//! Sprachfluss beim Vorlesen (#318): Pausen an Komma, Satz- und Absatzende.
//!
//! Gleiche Werte wie auf dem iPhone (`VoiceCore/SpeechFlow.swift`), damit beide
//! gleich klingen. Abkürzungen schreibt `abbreviations` aus.

/// 0 zügig, 0,5 Hörbuch-Standard, 1 ruhig; dazwischen stufenlos.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechFlow(f32);

impl Default for SpeechFlow {
    fn default() -> Self {
        Self(Self::AUDIOBOOK)
    }
}

impl SpeechFlow {
    pub const BRISK: f32 = 0.0;
    pub const AUDIOBOOK: f32 = 0.5;
    pub const CALM: f32 = 1.0;
    /// Stille, die vor dem ersten Laut eines Satzes stehen bleibt (Anlaut nie
    /// abschneiden). Teil der Satzpause.
    pub const LEAD_MS: f32 = 30.0;

    pub fn new(value: f32) -> Self {
        if value.is_finite() {
            Self(value.clamp(0.0, 1.0))
        } else {
            Self::default()
        }
    }

    pub fn value(self) -> f32 {
        self.0
    }

    fn lerp(self, a: f32, b: f32, c: f32) -> f32 {
        let v = self.0;
        if v <= 0.5 {
            a + (b - a) * v * 2.0
        } else {
            b + (c - b) * (v - 0.5) * 2.0
        }
    }

    /// Längste Pause im Satz (Komma, Abschnittsgrenzen des Modells).
    pub fn comma_ms(self) -> f32 {
        self.lerp(120.0, 200.0, 320.0)
    }

    /// Pause zwischen zwei Sätzen eines Absatzes.
    pub fn sentence_ms(self) -> f32 {
        self.lerp(170.0, 260.0, 420.0)
    }

    /// Pause am Absatzende.
    pub fn paragraph_ms(self) -> f32 {
        self.lerp(380.0, 600.0, 900.0)
    }

    /// Nachklang hinter dem letzten Laut: Satzpause minus Vorlauf des nächsten.
    pub fn tail_ms(self) -> f32 {
        (self.sentence_ms() - Self::LEAD_MS).max(40.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vorgaben_sind_geordnet() {
        let (b, h, r) = (
            SpeechFlow::new(SpeechFlow::BRISK),
            SpeechFlow::new(SpeechFlow::AUDIOBOOK),
            SpeechFlow::new(SpeechFlow::CALM),
        );
        assert!(b.sentence_ms() < h.sentence_ms() && h.sentence_ms() < r.sentence_ms());
        assert!(h.comma_ms() < h.sentence_ms() && h.sentence_ms() < h.paragraph_ms());
        assert_eq!(SpeechFlow::new(7.0).value(), 1.0);
        assert_eq!(SpeechFlow::new(f32::NAN), SpeechFlow::default());
    }
}
