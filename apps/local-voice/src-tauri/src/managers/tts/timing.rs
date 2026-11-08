//! Wann in einem Satz tatsächlich gesprochen wird.
//!
//! Die Lesehervorhebung (Reader) läuft innerhalb eines Satzes nach der Zeit
//! mit. Bisher kannte sie nur die Gesamtdauer des Satzes und verteilte
//! die Zeichen gleichmäßig darauf. Diese Dauer enthält aber den Nachklang
//! (`SpeechFlow::tail_ms`, rund 230 ms), am Absatzende die Absatzpause
//! (rund 340 ms mehr) und ausdrückliche Pausen. Die Markierung lief dadurch
//! langsamer als die Stimme und hing gegen Satzende um genau diese Stille
//! hinterher -- „meist etwas zu spät“ (Patrick 08.11.).
//!
//! lva#405. Hier werden die stimmhaften Abschnitte eines Satzes aus dem Audio selbst
//! bestimmt, das gleich gespielt wird: unabhängig vom Anbieter (Phonon,
//! Piper, Gradium …) und von dessen Rand- und Innenstille. Die Oberfläche
//! verteilt den Text nur noch über diese Abschnitte.

/// Rahmenlänge der Pegelmessung.
const FRAME_MS: u32 = 10;
/// Ab diesem Ausschlag (halbe Spitze-Spitze im Rahmen) zählt ein Rahmen als
/// Laut: rund −44 dBFS, derselbe Wert wie beim Kürzen der Randstille.
const VOICED_THRESHOLD: f32 = 0.006;
/// Kürzere Stille gehört zum Wort (Verschlusslaute, Silbengrenzen). Die
/// kürzeste Kommapause des Sprachflusses ist 120 ms, sie bleibt eine Lücke.
const MIN_GAP_MS: u32 = 100;
/// Kürzere Laute sind Knacken oder Atem, keine Sprache.
const MIN_SPAN_MS: u32 = 30;

/// Zeitangaben zu einem Satz, der gerade zu spielen beginnt (ms bei Tempo 1).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SentenceTiming {
    /// Dauer aller Stücke des Satzes samt Pausen.
    pub duration_ms: u32,
    /// Stimmhafte Abschnitte `[Beginn, Ende)` ab Satzbeginn, aufsteigend.
    /// Leer, wenn das Audio nicht lesbar war: dann gilt die Gesamtdauer.
    pub speech_ms: Vec<[u32; 2]>,
}

/// Die stimmhaften Abschnitte der Stücke eines Satzes, in Spielreihenfolge.
pub fn sentence_timing<'a>(blobs: impl IntoIterator<Item = &'a [u8]>) -> SentenceTiming {
    let mut timing = SentenceTiming::default();
    for wav in blobs {
        let offset = timing.duration_ms;
        let Some((mono, rate)) = decode_mono(wav) else {
            timing.duration_ms = offset.saturating_add(super::wav_duration_ms(wav));
            continue;
        };
        for [a, b] in voiced_spans_ms(&mono, rate) {
            let span = [offset.saturating_add(a), offset.saturating_add(b)];
            // Über eine Stückgrenze hinweg ohne echte Lücke: ein Abschnitt.
            match timing.speech_ms.last_mut() {
                Some(last) if span[0].saturating_sub(last[1]) < MIN_GAP_MS => last[1] = span[1],
                _ => timing.speech_ms.push(span),
            }
        }
        timing.duration_ms = offset.saturating_add(ms_of(mono.len(), rate));
    }
    timing
}

/// Stimmhafte Abschnitte eines Mono-Signals in ms; kurze Lücken sind
/// überbrückt, Knackser verworfen.
pub fn voiced_spans_ms(samples: &[f32], rate: u32) -> Vec<[u32; 2]> {
    let rate = rate.max(1);
    let frame = ((rate * FRAME_MS / 1000) as usize).max(1);
    let mut spans: Vec<[u32; 2]> = Vec::new();
    for (index, chunk) in samples.chunks(frame).enumerate() {
        let (lo, hi) = chunk
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        if (hi - lo) / 2.0 < VOICED_THRESHOLD {
            continue;
        }
        let start = ms_of(index * frame, rate);
        let end = ms_of(index * frame + chunk.len(), rate);
        match spans.last_mut() {
            Some(last) if start.saturating_sub(last[1]) < MIN_GAP_MS => last[1] = end,
            _ => spans.push([start, end]),
        }
    }
    spans.retain(|[a, b]| b - a >= MIN_SPAN_MS);
    spans
}

fn ms_of(samples: usize, rate: u32) -> u32 {
    (samples as u64 * 1000 / u64::from(rate.max(1))).min(u64::from(u32::MAX)) as u32
}

/// WAV → Mono-Abtastwerte und (Kopf-)Abtastrate. Die Kopfrate trägt das
/// Stimmtempo (`scale_wav_rate`), passt also zur Spieldauer.
fn decode_mono(wav: &[u8]) -> Option<(Vec<f32>, u32)> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(wav)).ok()?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>().ok()?,
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
                .ok()?
        }
    };
    let channels = usize::from(spec.channels.max(1));
    let mono = if channels == 1 {
        samples
    } else {
        samples
            .chunks(channels)
            .map(|f| f.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    Some((mono, spec.sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 24_000;

    fn tone(ms: u32) -> Vec<f32> {
        (0..RATE * ms / 1000)
            .map(|i| 0.3 * (i as f32 * 0.07).sin())
            .collect()
    }

    fn quiet(ms: u32) -> Vec<f32> {
        vec![0.0; (RATE * ms / 1000) as usize]
    }

    fn wav(samples: &[f32]) -> Vec<u8> {
        super::super::encode_wav_mono(samples, RATE)
    }

    fn near(actual: [u32; 2], expected: [u32; 2]) -> bool {
        actual[0].abs_diff(expected[0]) <= FRAME_MS && actual[1].abs_diff(expected[1]) <= FRAME_MS
    }

    #[test]
    fn nachklang_und_vorlauf_zaehlen_nicht_als_sprache() {
        // 30 ms Vorlauf, 1,5 s Stimme, 230 ms Nachklang: so verlässt ein Satz
        // `prepare_sentence_audio`.
        let samples = [quiet(30), tone(1500), quiet(230)].concat();
        let spans = voiced_spans_ms(&samples, RATE);
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert!(near(spans[0], [30, 1530]), "{spans:?}");
    }

    #[test]
    fn kommapause_trennt_kurze_luecke_nicht() {
        let samples = [tone(400), quiet(60), tone(300), quiet(200), tone(500)].concat();
        let spans = voiced_spans_ms(&samples, RATE);
        assert_eq!(
            spans.len(),
            2,
            "60 ms gehört zum Wort, 200 ms ist eine Pause: {spans:?}"
        );
        assert!(near(spans[0], [0, 760]), "{spans:?}");
        assert!(near(spans[1], [960, 1460]), "{spans:?}");
    }

    #[test]
    fn knackser_ist_keine_sprache() {
        let samples = [quiet(300), tone(10), quiet(300)].concat();
        assert!(voiced_spans_ms(&samples, RATE).is_empty());
    }

    #[test]
    fn satz_mit_absatzpause_meldet_dauer_und_sprechzeit() {
        // Wie die Pipeline: gesprochenes Stück, dann die Absatzpause als eigenes Stück.
        let speech = wav(&[quiet(30), tone(2000), quiet(230)].concat());
        let pause = wav(&quiet(340));
        let timing = sentence_timing([speech.as_slice(), pause.as_slice()]);
        assert!(timing.duration_ms.abs_diff(2600) <= 2, "{timing:?}");
        assert_eq!(timing.speech_ms.len(), 1, "{timing:?}");
        assert!(near(timing.speech_ms[0], [30, 2030]), "{timing:?}");
        // Genau das lief vorher falsch: rund 600 ms Stille wurden auf die
        // Wörter verteilt, die Markierung hing am Satzende so weit zurück.
        let speech_ms = timing.speech_ms[0][1] - timing.speech_ms[0][0];
        assert!(timing.duration_ms - speech_ms >= 550);
    }

    #[test]
    fn ausdrueckliche_pause_dazwischen_bleibt_eine_luecke() {
        let a = wav(&tone(800));
        let gap = wav(&quiet(500));
        let b = wav(&tone(600));
        let timing = sentence_timing([a.as_slice(), gap.as_slice(), b.as_slice()]);
        assert_eq!(timing.speech_ms.len(), 2, "{timing:?}");
        assert!(near(timing.speech_ms[1], [1300, 1900]), "{timing:?}");
    }

    #[test]
    fn stimmtempo_im_kopf_streckt_die_zeiten() {
        // `scale_wav_rate` ändert nur die Kopfrate: doppelt so schnell = halbe Zeit.
        let fast = super::super::scale_wav_rate(wav(&[quiet(100), tone(1000)].concat()), 2.0);
        let timing = sentence_timing([fast.as_slice()]);
        assert!(timing.duration_ms.abs_diff(550) <= 2, "{timing:?}");
        assert!(near(timing.speech_ms[0], [50, 550]), "{timing:?}");
    }

    #[test]
    fn unlesbares_audio_liefert_nur_die_dauer() {
        let timing = sentence_timing([b"kein wav".as_slice()]);
        assert_eq!(timing, SentenceTiming::default());
    }
}
