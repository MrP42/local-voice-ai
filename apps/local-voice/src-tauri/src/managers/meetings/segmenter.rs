//! M2 / P2a: VAD-Segmentierer fuer den Live-Pfad einer Besprechung.
//!
//! Ein Kanal (16 kHz mono i16) geht hinein, sprachgetrennte Segmente kommen
//! heraus. Rein und ohne I/O: der Detektor wird als `VoiceActivityDetector`
//! hineingereicht (Produktion: `SileroVad` v4, Tests: ein Energie-Detektor),
//! der Segmentierer selbst kennt weder Tauri noch Threads.
//!
//! Regeln (Spec m2-audio-stt.md, 3.1):
//! - Vorlauf 300 ms vor dem ersten Sprachframe, damit kein Wortanfang fehlt.
//! - Ein Segment endet, wenn 600 ms lang keine Sprache mehr erkannt wurde
//!   (Nachlauf). Enthalten sind dann Sprache plus 300 ms Auslauf.
//! - Weniger als 400 ms Sprache: Geraeusch, kein Segment (nur gezaehlt).
//! - Laenger als 15 s: Schnitt an der leisesten 200-ms-Stelle im letzten
//!   Viertel (`chunker::cut_point`, dieselbe Logik wie der 20-s-Chunker).
//! - Frames unter -70 dBFS gehen nie an den Detektor; ein Segment, dessen
//!   Gesamtpegel unter -70 dBFS liegt, wird verworfen (digitale Nullen).
//!
//! Zeitachse: `offset_ms` zaehlt die Samples, die der Segmentierer bekam, plus
//! die ueber `skip` gemeldeten Luecken. Das ist exakt die Achse der rohen
//! Kanal-WAV (Pausen werden dort ebenfalls nicht geschrieben).
//!
//! Faellt der Detektor aus (Fehler je Frame), zaehlt der Frame als Sprache
//! ("fail open"): lieber ein Segment zu viel als ein verlorenes.

use super::chunker::cut_point;
use crate::audio_toolkit::VoiceActivityDetector;

/// Samples je Millisekunde (16 kHz).
const SAMPLES_PER_MS: u64 = 16;
/// Frame des Silero-Detektors: 30 ms = 480 Samples.
pub const FRAME_SAMPLES: usize = 480;
pub const FRAME_MS: u64 = 30;

#[derive(Clone, Debug)]
pub struct SegmenterConfig {
    pub preroll_ms: u64,
    pub hangover_ms: u64,
    pub min_speech_ms: u64,
    pub max_segment_ms: u64,
    pub trailing_pad_ms: u64,
    /// Aufeinanderfolgende Sprachframes, bevor ein Segment beginnt.
    pub onset_frames: u32,
    pub silence_floor_dbfs: f32,
}

impl Default for SegmenterConfig {
    fn default() -> Self {
        Self {
            preroll_ms: 300,
            hangover_ms: 600,
            min_speech_ms: 400,
            max_segment_ms: 15_000,
            trailing_pad_ms: 300,
            onset_frames: 2,
            silence_floor_dbfs: -70.0,
        }
    }
}

/// Ein sprachgetrenntes Stueck Audio eines Kanals.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub samples: Vec<i16>,
    /// Beginn auf der Kanal-Zeitachse (= WAV-Achse), ms.
    pub offset_ms: u64,
    /// Ende der Aeusserung (letzter Sprachframe) bzw. Schnittpunkt, ms.
    pub vad_end_ms: u64,
    /// Zeitpunkt der Entscheidung (Audiozeit), ms. `decided - vad_end` ist die
    /// Nachlauf-Latenz des Segmentierers.
    pub decided_at_ms: u64,
    /// Summe der Sprachframes, ms.
    pub speech_ms: u64,
    /// Erster bis letzter Sprachframe, ms.
    pub span_ms: u64,
}

#[cfg(test)]
impl Segment {
    pub fn duration_ms(&self) -> u64 {
        self.samples.len() as u64 / SAMPLES_PER_MS
    }
}

/// Zaehler, die der DSP-Thread nach jedem Aufruf abholt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SegmenterStats {
    pub emitted: u64,
    pub dropped_short: u64,
    pub dropped_silent: u64,
    pub vad_errors: u64,
    pub cut_at_max: u64,
}

struct Active {
    /// Erster Frame des Segments (lokaler Frameindex, inkl. Vorlauf).
    start: u64,
    /// Sprachmerker je Frame ab `start`.
    flags: Vec<bool>,
    silence_run: u64,
}

impl Active {
    fn last_voiced(&self) -> Option<usize> {
        self.flags.iter().rposition(|v| *v)
    }
}

pub struct VadSegmenter {
    vad: Box<dyn VoiceActivityDetector>,
    cfg: SegmenterConfig,
    preroll: u64,
    hangover: u64,
    pad: u64,
    max_frames: u64,
    floor_amp: f64,

    /// Absolute Kanalposition (Samples) des lokalen Samples 0.
    origin: u64,
    /// Seit `origin` empfangene Samples.
    received: u64,
    /// Samples ab lokalem Frame `buf_first`.
    buf: Vec<i16>,
    buf_first: u64,
    frames_done: u64,
    onset_run: u32,
    /// Fruehester erlaubter Segmentbeginn (Frame): Ende des letzten Segments.
    min_start: u64,
    active: Option<Active>,
    scratch: Vec<f32>,
    stats: SegmenterStats,
}

impl VadSegmenter {
    pub fn new(vad: Box<dyn VoiceActivityDetector>, cfg: SegmenterConfig) -> Self {
        let frames = |ms: u64| ms.div_ceil(FRAME_MS);
        // dBFS -> RMS in i16-Einheiten.
        let floor_amp = 32_768.0 * 10f64.powf(cfg.silence_floor_dbfs as f64 / 20.0);
        Self {
            vad,
            preroll: frames(cfg.preroll_ms),
            hangover: frames(cfg.hangover_ms).max(1),
            pad: frames(cfg.trailing_pad_ms),
            max_frames: frames(cfg.max_segment_ms).max(1),
            floor_amp,
            cfg,
            origin: 0,
            received: 0,
            buf: Vec::new(),
            buf_first: 0,
            frames_done: 0,
            onset_run: 0,
            min_start: 0,
            active: None,
            scratch: vec![0.0; FRAME_SAMPLES],
            stats: SegmenterStats::default(),
        }
    }

    pub fn take_stats(&mut self) -> SegmenterStats {
        std::mem::take(&mut self.stats)
    }

    /// Fuettert Samples ein und gibt die dabei abgeschlossenen Segmente zurueck.
    pub fn push(&mut self, samples: &[i16]) -> Vec<Segment> {
        let mut out = Vec::new();
        self.buf.extend_from_slice(samples);
        self.received += samples.len() as u64;
        while (self.frames_done + 1) * FRAME_SAMPLES as u64 <= self.received {
            let f = self.frames_done;
            let voiced = self.classify(f);
            self.frames_done += 1;
            self.on_frame(f, voiced, &mut out);
        }
        self.trim();
        out
    }

    /// Ende des Stroms: das offene Segment (falls es genug Sprache hat) heraus.
    pub fn flush(&mut self) -> Vec<Segment> {
        let mut out = Vec::new();
        if let Some(a) = self.active.take() {
            if let Some(last) = a.last_voiced() {
                let voiced_end = a.start + last as u64 + 1;
                let end_sample = self
                    .received
                    .min((voiced_end + self.pad) * FRAME_SAMPLES as u64);
                if let Some(seg) = self.finish(a.start, end_sample, &a.flags, self.received) {
                    out.push(seg);
                }
            }
        }
        self.onset_run = 0;
        out
    }

    /// Pause: das offene Segment wird beendet, Detektor und Vorlauf beginnen
    /// nach dem Fortsetzen neu. Die Zeitachse laeuft weiter (die Pause selbst
    /// steht nicht in der WAV).
    pub fn boundary(&mut self) -> Vec<Segment> {
        let out = self.flush();
        self.reset_timeline(0);
        out
    }

    /// Luecke: `samples` Samples fehlen in der Auslieferung, sind aber in der
    /// WAV. Die Zeitachse springt entsprechend weiter.
    pub fn skip(&mut self, samples: u64) -> Vec<Segment> {
        let out = self.flush();
        self.reset_timeline(samples);
        out
    }

    fn reset_timeline(&mut self, advance: u64) {
        self.origin += self.received + advance;
        self.received = 0;
        self.buf.clear();
        self.buf_first = 0;
        self.frames_done = 0;
        self.onset_run = 0;
        self.min_start = 0;
        self.active = None;
        self.vad.reset();
    }

    fn classify(&mut self, f: u64) -> bool {
        let off = ((f - self.buf_first) * FRAME_SAMPLES as u64) as usize;
        let frame = &self.buf[off..off + FRAME_SAMPLES];
        let energy: f64 = frame.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        let rms = (energy / FRAME_SAMPLES as f64).sqrt();
        if rms < self.floor_amp {
            return false;
        }
        for (dst, src) in self.scratch.iter_mut().zip(frame.iter()) {
            *dst = *src as f32 / 32_768.0;
        }
        match self.vad.is_voice(&self.scratch) {
            Ok(v) => v,
            Err(_) => {
                self.stats.vad_errors += 1;
                true
            }
        }
    }

    fn on_frame(&mut self, f: u64, voiced: bool, out: &mut Vec<Segment>) {
        let cur_end = f + 1;
        let Some(active) = self.active.as_mut() else {
            if voiced {
                self.onset_run += 1;
                if self.onset_run >= self.cfg.onset_frames.max(1) {
                    let first_voice = cur_end - self.onset_run as u64;
                    let start = first_voice
                        .saturating_sub(self.preroll)
                        .max(self.min_start)
                        .max(self.buf_first);
                    let mut flags = vec![false; (cur_end - start) as usize];
                    let n = flags.len();
                    for flag in flags.iter_mut().skip(n - self.onset_run as usize) {
                        *flag = true;
                    }
                    self.active = Some(Active {
                        start,
                        flags,
                        silence_run: 0,
                    });
                    self.onset_run = 0;
                }
            } else {
                self.onset_run = 0;
            }
            return;
        };

        active.flags.push(voiced);
        if voiced {
            active.silence_run = 0;
        } else {
            active.silence_run += 1;
        }

        if active.silence_run >= self.hangover {
            let a = self.active.take().expect("active checked above");
            let last = a.last_voiced().unwrap_or(0) as u64;
            let end_frame = (a.start + last + 1 + self.pad).min(cur_end);
            let end_sample = end_frame * FRAME_SAMPLES as u64;
            if let Some(seg) = self.finish(
                a.start,
                end_sample,
                &a.flags,
                cur_end * FRAME_SAMPLES as u64,
            ) {
                out.push(seg);
            }
            self.min_start = end_frame;
            self.onset_run = 0;
            return;
        }

        if cur_end - active.start >= self.max_frames {
            self.cut_at_max(cur_end, out);
        }
    }

    /// Segment hat die Hoechstlaenge erreicht: an der leisesten Stelle im
    /// letzten Viertel schneiden, der Rest laeuft als naechstes Segment weiter.
    fn cut_at_max(&mut self, cur_end: u64, out: &mut Vec<Segment>) {
        let Some(a) = self.active.take() else {
            return;
        };
        let target = (self.max_frames * FRAME_SAMPLES as u64) as usize;
        let off = ((a.start - self.buf_first) * FRAME_SAMPLES as u64) as usize;
        let len = ((cur_end - a.start) * FRAME_SAMPLES as u64) as usize;
        let seg_audio = &self.buf[off..off + len];
        let cut_frames =
            ((cut_point(seg_audio, target) / FRAME_SAMPLES) as u64).clamp(1, cur_end - a.start);
        let end_sample = (a.start + cut_frames) * FRAME_SAMPLES as u64;
        let decided = cur_end * FRAME_SAMPLES as u64;
        // `vad_end` eines Schnitts mitten in der Rede ist der Schnittpunkt.
        if let Some(mut seg) = self.finish(a.start, end_sample, &a.flags, decided) {
            seg.vad_end_ms = (self.origin + end_sample) / SAMPLES_PER_MS;
            out.push(seg);
        }
        self.stats.cut_at_max += 1;

        let new_start = a.start + cut_frames;
        let tail: Vec<bool> = a.flags[cut_frames as usize..].to_vec();
        self.min_start = new_start;
        if tail.iter().any(|v| *v) {
            let silence_run = tail.iter().rev().take_while(|v| !**v).count() as u64;
            self.active = Some(Active {
                start: new_start,
                flags: tail,
                silence_run,
            });
        } else {
            self.onset_run = 0;
        }
    }

    /// Baut das Segment [start_frame, end_sample) und wendet Mindestsprache
    /// sowie Pegel-Gate an. `None` = verworfen (gezaehlt).
    fn finish(
        &mut self,
        start: u64,
        end_sample: u64,
        flags: &[bool],
        decided_sample: u64,
    ) -> Option<Segment> {
        let start_sample = start * FRAME_SAMPLES as u64;
        if end_sample <= start_sample {
            return None;
        }
        let frames_in = ((end_sample - start_sample) / FRAME_SAMPLES as u64) as usize;
        let covered = &flags[..frames_in.min(flags.len())];
        let voiced = covered.iter().filter(|v| **v).count() as u64;
        let speech_ms = voiced * FRAME_MS;
        if speech_ms < self.cfg.min_speech_ms {
            self.stats.dropped_short += 1;
            return None;
        }
        let first = covered.iter().position(|v| *v).unwrap_or(0) as u64;
        let last = covered.iter().rposition(|v| *v).unwrap_or(0) as u64;
        let span_ms = (last + 1 - first) * FRAME_MS;

        let from = (start_sample - self.buf_first * FRAME_SAMPLES as u64) as usize;
        let to = (end_sample - self.buf_first * FRAME_SAMPLES as u64) as usize;
        let samples = self.buf[from..to.min(self.buf.len())].to_vec();
        let energy: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        let rms = (energy / samples.len().max(1) as f64).sqrt();
        if rms < self.floor_amp {
            self.stats.dropped_silent += 1;
            return None;
        }

        self.stats.emitted += 1;
        Some(Segment {
            samples,
            offset_ms: (self.origin + start_sample) / SAMPLES_PER_MS,
            vad_end_ms: (self.origin + (start + last + 1) * FRAME_SAMPLES as u64) / SAMPLES_PER_MS,
            decided_at_ms: (self.origin + decided_sample) / SAMPLES_PER_MS,
            speech_ms,
            span_ms,
        })
    }

    /// Wirft Audio weg, das kein kuenftiges Segment mehr braucht.
    fn trim(&mut self) {
        let keep_from = match &self.active {
            Some(a) => a.start,
            None => self
                .frames_done
                .saturating_sub(self.preroll + self.onset_run as u64)
                .max(self.min_start),
        };
        if keep_from > self.buf_first {
            let drop = ((keep_from - self.buf_first) * FRAME_SAMPLES as u64) as usize;
            self.buf.drain(..drop.min(self.buf.len()));
            self.buf_first = keep_from;
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::audio_toolkit::vad::VadFrame;
    use anyhow::Result;

    /// Energie-Detektor fuer Tests: Sprache = RMS ueber der Schwelle.
    pub struct EnergyVad {
        pub threshold: f32,
    }

    impl EnergyVad {
        pub fn boxed() -> Box<dyn VoiceActivityDetector> {
            Box::new(Self { threshold: 0.02 })
        }
    }

    impl VoiceActivityDetector for EnergyVad {
        fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> Result<VadFrame<'a>> {
            let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
            if rms > self.threshold {
                Ok(VadFrame::Speech(frame))
            } else {
                Ok(VadFrame::Noise)
            }
        }
    }

    pub const RATE: usize = 16_000;

    /// "Sprache": lauter Ton (Amplitude 6000, gut ueber Schwelle und Boden).
    pub fn loud(ms: usize) -> Vec<i16> {
        (0..ms * RATE / 1000)
            .map(|i| if i % 2 == 0 { 6_000 } else { -6_000 })
            .collect()
    }

    /// Hoerbare Stille (unter der Detektor-Schwelle, ueber dem -70-dBFS-Boden).
    pub fn quiet(ms: usize) -> Vec<i16> {
        (0..ms * RATE / 1000)
            .map(|i| if i % 2 == 0 { 40 } else { -40 })
            .collect()
    }

    pub fn zeros(ms: usize) -> Vec<i16> {
        vec![0; ms * RATE / 1000]
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    fn seg() -> VadSegmenter {
        VadSegmenter::new(EnergyVad::boxed(), SegmenterConfig::default())
    }

    /// Fuettert in 30-ms-Bloecken wie der Capture-Pfad, sammelt alles.
    fn run(s: &mut VadSegmenter, audio: &[i16]) -> Vec<Segment> {
        let mut out = Vec::new();
        for block in audio.chunks(480) {
            out.extend(s.push(block));
        }
        out
    }

    fn cat(parts: &[Vec<i16>]) -> Vec<i16> {
        parts.iter().flatten().copied().collect()
    }

    #[test]
    fn digital_silence_yields_no_segment_and_no_detector_call() {
        let mut s = seg();
        let mut out = run(&mut s, &zeros(60_000));
        out.extend(s.flush());
        assert!(out.is_empty());
        assert_eq!(s.take_stats(), SegmenterStats::default());
    }

    #[test]
    fn a_channel_below_minus_70_dbfs_is_never_transcribed() {
        // Rauschen mit +-2 LSB (~ -96 dBFS). Ein Detektor, der IMMER "Sprache"
        // sagt, darf trotzdem nichts durchbekommen.
        struct AlwaysVoice;
        impl VoiceActivityDetector for AlwaysVoice {
            fn push_frame<'a>(
                &'a mut self,
                frame: &'a [f32],
            ) -> anyhow::Result<crate::audio_toolkit::vad::VadFrame<'a>> {
                Ok(crate::audio_toolkit::vad::VadFrame::Speech(frame))
            }
        }
        let mut s = VadSegmenter::new(Box::new(AlwaysVoice), SegmenterConfig::default());
        let noise: Vec<i16> = (0..RATE * 30)
            .map(|i| if i % 2 == 0 { 2 } else { -2 })
            .collect();
        let mut out = run(&mut s, &noise);
        out.extend(s.flush());
        assert!(out.is_empty());
    }

    #[test]
    fn a_short_burst_below_400_ms_is_noise() {
        let mut s = seg();
        let audio = cat(&[quiet(2_000), loud(300), quiet(2_000)]);
        let mut out = run(&mut s, &audio);
        out.extend(s.flush());
        assert!(out.is_empty());
        assert_eq!(s.take_stats().dropped_short, 1);
    }

    #[test]
    fn speech_gets_preroll_and_trailing_pad_and_is_emitted_after_the_hangover() {
        let mut s = seg();
        let audio = cat(&[quiet(1_000), loud(2_000), quiet(2_000)]);
        let out = run(&mut s, &audio);
        assert_eq!(out.len(), 1);
        let g = &out[0];
        // Sprache 1000..3000 ms; Vorlauf 300 ms, Auslauf 300 ms.
        assert!(
            (690..=720).contains(&g.offset_ms),
            "offset {} ms",
            g.offset_ms
        );
        assert!((2_990..=3_040).contains(&g.vad_end_ms), "{}", g.vad_end_ms);
        let end_ms = g.offset_ms + g.duration_ms();
        assert!((3_290..=3_340).contains(&end_ms), "end {end_ms}");
        // Entscheidung 600 ms nach dem Ende der Aeusserung.
        let latency = g.decided_at_ms - g.vad_end_ms;
        assert!((570..=660).contains(&latency), "latency {latency} ms");
        assert!(g.speech_ms >= 1_900);
    }

    #[test]
    fn a_pause_shorter_than_the_hangover_does_not_split() {
        let mut s = seg();
        let audio = cat(&[loud(1_500), quiet(500), loud(1_500), quiet(1_500)]);
        let out = run(&mut s, &audio);
        assert_eq!(out.len(), 1);
        assert!(out[0].duration_ms() >= 3_400);
    }

    #[test]
    fn a_pause_of_800_ms_splits_into_two_segments_without_overlap() {
        let mut s = seg();
        let audio = cat(&[loud(1_500), quiet(800), loud(1_500), quiet(1_500)]);
        let out = run(&mut s, &audio);
        assert_eq!(out.len(), 2);
        let first_end = out[0].offset_ms + out[0].duration_ms();
        assert!(first_end <= out[1].offset_ms, "Segmente ueberlappen");
        // Zusammen ist keine Sprache verloren: alles ~1500 ms je Segment.
        assert!(out[0].speech_ms >= 1_400 && out[1].speech_ms >= 1_400);
    }

    #[test]
    fn forty_seconds_of_continuous_speech_are_cut_at_15_s_or_less() {
        let mut s = seg();
        // Alle 3 s ein 300-ms-Einbruch (kein Split: unter dem Nachlauf).
        let mut audio = Vec::new();
        for _ in 0..13 {
            audio.extend(loud(2_700));
            audio.extend(quiet(300));
        }
        audio.extend(quiet(2_000));
        let mut out = run(&mut s, &audio);
        out.extend(s.flush());
        assert!(out.len() >= 3, "{} Segmente", out.len());
        for g in &out {
            assert!(g.duration_ms() <= 15_000, "Segment {} ms", g.duration_ms());
        }
        // Lueckenlos: jedes Segment beginnt dort, wo das vorige aufhoerte.
        for pair in out.windows(2) {
            let end = pair[0].offset_ms + pair[0].duration_ms();
            assert!(pair[1].offset_ms >= end);
            assert!(
                pair[1].offset_ms - end <= 60,
                "Loch vor {}",
                pair[1].offset_ms
            );
        }
        assert!(s.take_stats().cut_at_max >= 2);
    }

    #[test]
    fn the_max_cut_lands_in_the_quiet_dip_not_mid_word() {
        let mut s = seg();
        // 13 s laut, 300 ms Einbruch, 5 s laut: Schnitt gehoert in den Einbruch
        // (13,0-13,3 s liegt im Suchfenster 11,25-15 s).
        let audio = cat(&[loud(13_000), quiet(300), loud(5_000), quiet(2_000)]);
        let out = run(&mut s, &audio);
        assert!(out.len() >= 2);
        let cut_ms = out[0].offset_ms + out[0].duration_ms();
        assert!(
            (12_900..=13_400).contains(&cut_ms),
            "Schnitt bei {cut_ms} ms statt im Einbruch"
        );
    }

    #[test]
    fn flush_at_stop_emits_the_open_segment() {
        let mut s = seg();
        let out = run(&mut s, &cat(&[quiet(500), loud(3_000)]));
        assert!(out.is_empty(), "noch offen");
        let tail = s.flush();
        assert_eq!(tail.len(), 1);
        assert!(tail[0].speech_ms >= 2_900);
        assert!(s.flush().is_empty());
    }

    #[test]
    fn boundary_ends_the_segment_and_resumes_on_the_same_timeline() {
        let mut s = seg();
        run(&mut s, &loud(2_000));
        let before = s.boundary();
        assert_eq!(
            before.len(),
            1,
            "Sprache vor der Pause wird nicht verschluckt"
        );
        // Nach dem Fortsetzen: neuer Vorlauf, Achse laeuft bei 2 s weiter.
        let out = run(&mut s, &cat(&[quiet(1_000), loud(2_000), quiet(1_500)]));
        assert_eq!(out.len(), 1);
        assert!(
            (2_690..=2_720).contains(&out[0].offset_ms),
            "offset {}",
            out[0].offset_ms
        );
        // Vorlauf kommt nicht aus der Zeit vor der Pause.
        assert!(out[0].offset_ms >= 2_000);
    }

    #[test]
    fn skip_advances_the_timeline_by_the_lost_samples() {
        let mut s = seg();
        run(&mut s, &quiet(1_000));
        assert!(s.skip((RATE * 5) as u64).is_empty());
        let out = run(&mut s, &cat(&[loud(2_000), quiet(1_500)]));
        assert_eq!(out.len(), 1);
        // 1 s + 5 s Luecke = 6 s; Vorlauf gibt es nach der Luecke nicht.
        assert!(
            (5_990..=6_040).contains(&out[0].offset_ms),
            "offset {}",
            out[0].offset_ms
        );
    }

    #[test]
    fn a_failing_detector_fails_open_instead_of_losing_speech() {
        struct Broken;
        impl VoiceActivityDetector for Broken {
            fn push_frame<'a>(
                &'a mut self,
                _frame: &'a [f32],
            ) -> anyhow::Result<crate::audio_toolkit::vad::VadFrame<'a>> {
                anyhow::bail!("ort kaputt")
            }
        }
        let mut s = VadSegmenter::new(Box::new(Broken), SegmenterConfig::default());
        let mut out = run(&mut s, &loud(20_000));
        out.extend(s.flush());
        let total: u64 = out.iter().map(|g| g.duration_ms()).sum();
        assert!(total >= 19_500, "nur {total} ms uebrig");
        assert!(out.iter().all(|g| g.duration_ms() <= 15_000));
        assert!(s.take_stats().vad_errors > 100);
    }

    #[test]
    fn block_size_does_not_change_the_result() {
        let audio = cat(&[
            quiet(500),
            loud(2_000),
            quiet(1_000),
            loud(1_000),
            quiet(1_500),
        ]);
        let mut a = seg();
        let mut b = seg();
        let out_a = run(&mut a, &audio);
        let mut out_b = Vec::new();
        for block in audio.chunks(777) {
            out_b.extend(b.push(block));
        }
        assert_eq!(out_a, out_b);
    }
}

/// Tests mit dem echten Silero-v4-Detektor und der deutschen TTS-Fixture
/// `m8_short_de.wav` (60 s, 5 Saetze mit Pausen; nicht im Repo, siehe
/// `scripts/make-m8-fixtures.ps1`). Fehlt die Fixture, wird uebersprungen.
#[cfg(test)]
mod silero_fixture_tests {
    use super::*;
    use crate::audio_toolkit::SileroVad;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    fn model() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/models/silero_vad_v4.onnx")
    }

    /// Sucht die (ignorierte) Fixture im eigenen Baum und in den Vorfahren
    /// (ein Worktree unter `.claude/worktrees/` teilt die des Hauptbaums).
    fn fixture(name: &str) -> Option<PathBuf> {
        let start = Path::new(env!("CARGO_MANIFEST_DIR"));
        for dir in start.ancestors() {
            for rel in [
                "tests/fixtures",
                "apps/local-voice/src-tauri/tests/fixtures",
            ] {
                let p = dir.join(rel).join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        None
    }

    fn silero() -> VadSegmenter {
        let vad = SileroVad::new(model(), 0.5).expect("Silero-Modell laedt");
        VadSegmenter::new(Box::new(vad), SegmenterConfig::default())
    }

    fn load(name: &str) -> Option<Vec<i16>> {
        let Some(path) = fixture(name) else {
            eprintln!("SKIP: Fixture {name} nicht gefunden");
            return None;
        };
        Some(super::super::import::read_wav_i16_mono_16k(&path).expect("Fixture lesbar"))
    }

    fn run_all(s: &mut VadSegmenter, audio: &[i16]) -> Vec<Segment> {
        let mut out = Vec::new();
        for block in audio.chunks(480) {
            out.extend(s.push(block));
        }
        out.extend(s.flush());
        out
    }

    fn frame_db(frame: &[i16]) -> f64 {
        let e: f64 = frame.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / frame.len() as f64;
        20.0 * (e.sqrt().max(1.0) / 32_768.0).log10()
    }

    /// Unabhaengige Referenz: Pausen >= `min_ms`, in denen jeder 30-ms-Frame
    /// unter -50 dBFS liegt. (start_ms, end_ms)
    fn reference_pauses(audio: &[i16], min_ms: u64) -> Vec<(u64, u64)> {
        let mut pauses = Vec::new();
        let mut start: Option<usize> = None;
        for (i, frame) in audio.chunks_exact(480).enumerate() {
            let quiet = frame_db(frame) < -50.0;
            match (quiet, start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    let (a, b) = (s as u64 * 30, i as u64 * 30);
                    if b - a >= min_ms {
                        pauses.push((a, b));
                    }
                    start = None;
                }
                _ => {}
            }
        }
        pauses
    }

    fn end_ms(seg: &Segment) -> u64 {
        seg.offset_ms + seg.duration_ms()
    }

    #[test]
    fn m8_short_de_is_split_at_the_tts_pauses_within_300_ms() {
        let Some(audio) = load("m8_short_de.wav") else {
            return;
        };
        let started = Instant::now();
        let mut s = silero();
        let segments = run_all(&mut s, &audio);
        let elapsed = started.elapsed();

        // Pausen ab 700 ms trennen sicher (Nachlauf 600 ms + Frame-Schlupf).
        let pauses = reference_pauses(&audio, 700);
        assert!(
            pauses.len() >= 8,
            "Referenz hat nur {} Pausen",
            pauses.len()
        );
        assert_eq!(
            segments.len(),
            pauses.len() + 1,
            "Segmente {:?} vs Pausen {:?}",
            segments
                .iter()
                .map(|g| (g.offset_ms, end_ms(g)))
                .collect::<Vec<_>>(),
            pauses
        );
        for (ps, pe) in &pauses {
            let (lo, hi) = (ps.saturating_sub(300), pe + 300);
            let ends = segments.iter().any(|g| (lo..=hi).contains(&end_ms(g)));
            let starts = segments.iter().any(|g| (lo..=hi).contains(&g.offset_ms));
            assert!(ends, "kein Segmentende bei Pause {ps}-{pe} ms");
            assert!(starts, "kein Segmentbeginn bei Pause {ps}-{pe} ms");
        }

        // Kein Wort geht verloren: jeder laute Frame liegt in einem Segment
        // (Toleranz 1 % fuer Frames am Rand).
        let mut uncovered = 0usize;
        let mut loud_frames = 0usize;
        for (i, frame) in audio.chunks_exact(480).enumerate() {
            if frame_db(frame) > -45.0 {
                loud_frames += 1;
                let t = i as u64 * 30;
                if !segments
                    .iter()
                    .any(|g| t >= g.offset_ms && t + 30 <= end_ms(g))
                {
                    uncovered += 1;
                }
            }
        }
        assert!(
            uncovered * 100 <= loud_frames,
            "{uncovered} von {loud_frames} lauten Frames ohne Segment"
        );

        // Nachlauf-Latenz je Segment (Audiozeit) und Rechenaufwand. Die Schaetzung
        // fuer "Ende der Aeusserung bis Event" addiert die STT-Zeit nach den
        // Spike-Messwerten (Parakeet CPU, RTF 14) und 100 ms fuer Speichern und
        // Event; die Wartezeit hinter dem anderen Kanal kommt on top.
        let lat: Vec<u64> = segments
            .iter()
            .map(|g| g.decided_at_ms.saturating_sub(g.vad_end_ms))
            .collect();
        let mut est: Vec<u64> = segments
            .iter()
            .map(|g| g.decided_at_ms.saturating_sub(g.vad_end_ms) + g.duration_ms() / 14 + 100)
            .collect();
        est.sort_unstable();
        eprintln!(
            "SILERO m8_short_de: {} Segmente (Dauer {:?} ms), Nachlauf min/max {}/{} ms, Latenz-Schaetzung Aeusserungsende bis Event min/median/max {}/{}/{} ms, {:.2} s Rechenzeit (Debug-Build) fuer 60 s Audio",
            segments.len(),
            segments.iter().map(|g| g.duration_ms()).collect::<Vec<_>>(),
            lat.iter().min().unwrap(),
            lat.iter().max().unwrap(),
            est[0],
            est[est.len() / 2],
            est[est.len() - 1],
            elapsed.as_secs_f64()
        );
        assert!(*lat.iter().max().unwrap() <= 700);
        assert!(elapsed.as_secs_f64() < 30.0, "VAD zu langsam: {elapsed:?}");
    }

    #[test]
    fn forty_seconds_of_real_speech_without_pauses_are_cut_at_15_s_or_less() {
        let Some(audio) = load("m8_short_de.wav") else {
            return;
        };
        // Alle Frames ueber -50 dBFS hintereinander: ~40 s ohne echte Pause.
        let mut dense: Vec<i16> = Vec::new();
        for frame in audio.chunks_exact(480) {
            if frame_db(frame) >= -50.0 {
                dense.extend_from_slice(frame);
            }
        }
        assert!(
            dense.len() >= 16_000 * 30,
            "nur {} s Sprache",
            dense.len() / 16_000
        );
        let mut s = silero();
        let segments = run_all(&mut s, &dense);
        assert!(!segments.is_empty());
        for g in &segments {
            assert!(g.duration_ms() <= 15_000, "Segment {} ms", g.duration_ms());
        }
        let covered: u64 = segments.iter().map(|g| g.duration_ms()).sum();
        let total = dense.len() as u64 / 16;
        assert!(
            covered * 100 >= total * 95,
            "{covered} von {total} ms abgedeckt"
        );
    }

    #[test]
    fn sixty_seconds_of_digital_silence_or_faint_noise_make_no_segment() {
        let mut s = silero();
        assert!(run_all(&mut s, &vec![0i16; 16_000 * 60]).is_empty());
        // Dithering-Rauschen +-3 LSB (~ -90 dBFS).
        let noise: Vec<i16> = (0..16_000 * 60).map(|i| [3i16, -2, 1, -3][i % 4]).collect();
        let mut s = silero();
        assert!(run_all(&mut s, &noise).is_empty());
        assert_eq!(s.take_stats().vad_errors, 0);
    }

    #[test]
    fn a_silent_channel_beside_speech_stays_silent() {
        // Systemkanal ohne Ton neben einem Mikrokanal mit Sprache: Kanal 1 darf
        // nichts erzeugen (kein STT-Aufruf auf Stille).
        let Some(speech) = load("m8_short_de.wav") else {
            return;
        };
        let mut mic = silero();
        let mut sys = silero();
        let mic_out = run_all(&mut mic, &speech);
        let sys_out = run_all(&mut sys, &vec![0i16; speech.len()]);
        assert!(!mic_out.is_empty());
        assert!(sys_out.is_empty());
    }
}
