//! Playback hinter einem Trait, damit der Manager ohne Soundkarte testbar ist.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Tempo und Lautstärke, die eine LAUFENDE Wiedergabe mitliest.
///
/// Beide Werte wurden früher einmal beim Start eines Satzes gelesen und der
/// Wiedergabe mitgegeben. Wer während des Vorlesens am Tempo drehte, hörte
/// die Änderung deshalb erst beim nächsten Satz — bei einem langen Absatz
/// also gefühlt gar nicht. Genau dann will man aber stellen: man merkt beim
/// Hören, dass es zu langsam ist, nicht vorher.
///
/// Als Atomics statt hinter einem Mutex: der Wiedergabe-Thread liest sie
/// zwanzigmal pro Sekunde, und ein Regler darf nie auf ein Schloss warten.
/// `f32` liegt dabei als Bitmuster in einem `u32` — `AtomicF32` gibt es in
/// der Standardbibliothek nicht.
#[derive(Debug)]
pub struct PlaybackControls {
    volume_bits: AtomicU32,
    speed_bits: AtomicU32,
}

impl Default for PlaybackControls {
    fn default() -> Self {
        Self {
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            speed_bits: AtomicU32::new(1.0f32.to_bits()),
        }
    }
}

impl PlaybackControls {
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn speed(&self) -> f32 {
        f32::from_bits(self.speed_bits.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, value: f32) {
        self.volume_bits
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Tempo entsteht per Zeitstreckung (WSOLA, [`super::dsp::time_stretch`]):
    /// die Stimme bleibt in Tonhöhe und Klang gleich.
    pub fn set_speed(&self, value: f32) {
        self.speed_bits
            .store(value.clamp(0.5, 2.0).to_bits(), Ordering::Relaxed);
    }
}

/// Ein Stück Audio für die Stream-Wiedergabe ([`Player::play_stream`]):
/// verschachtelte Abtastwerte (`channels` je Frame), Wertebereich -1,0 bis 1,0.
#[derive(Debug, Clone, PartialEq)]
pub struct PcmSegment {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

pub trait Player: Send + Sync {
    /// Spielt einen kompletten WAV-Blob ab; kehrt erst nach Ende oder Abbruch
    /// zurück. `cancelled` wird von cancel()/neuen Aufträgen gesetzt.
    ///
    /// `gain` ist der Pegelausgleich DIESES Satzes (siehe `playback_gain`) und
    /// steht fest; `controls` trägt die Nutzerwerte und darf sich währenddessen
    /// ändern.
    fn play(
        &self,
        wav: Vec<u8>,
        device: Option<String>,
        gain: f32,
        controls: Arc<PlaybackControls>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), String>;

    /// Spielt Audio ab, SOBALD es eintrifft (Probestimme der Anbieter-Schicht,
    /// Paket P3a): `segments` liefert Stücke, die Wiedergabe endet, wenn der
    /// Sender fallengelassen ist und alles gespielt wurde — oder bei
    /// `cancelled` (dann sofort, ohne weiteres Audio). Blockiert bis dahin.
    ///
    /// Die Standardumsetzung ist NICHT streamend: sie sammelt alle Stücke zu
    /// EINEM WAV und spielt es über [`Self::play`] (Verhalten für Player ohne
    /// eigene Stream-Fähigkeit; gleiche Abtastrate und Kanalzahl vorausgesetzt).
    /// [`RodioPlayer`] überschreibt sie mit echter Stream-Wiedergabe.
    fn play_stream(
        &self,
        segments: std::sync::mpsc::Receiver<PcmSegment>,
        device: Option<String>,
        controls: Arc<PlaybackControls>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), String> {
        use std::sync::mpsc::RecvTimeoutError;
        let mut format: Option<(u32, u16)> = None;
        let mut pcm: Vec<u8> = Vec::new();
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            match segments.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(seg) => {
                    match format {
                        None => format = Some((seg.sample_rate, seg.channels)),
                        Some(f) if f != (seg.sample_rate, seg.channels) => {
                            return Err("Abtastrate oder Kanalzahl wechselt mitten im Stream".into())
                        }
                        Some(_) => {}
                    }
                    for sample in seg.samples {
                        pcm.extend_from_slice(&sample.to_le_bytes());
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let Some((rate, channels)) = format else {
            return Ok(());
        };
        let wav = crate::managers::tts::providers::wav::wav_from_pcm(
            crate::managers::tts::providers::contract::AudioEncoding::F32Le,
            rate,
            channels,
            &pcm,
        )
        .map_err(|e| e.message)?;
        self.play(wav, device, 1.0, controls, cancelled)
    }
}

pub struct RodioPlayer;

/// Ausgabestrom auf dem gewählten Gerät öffnen (Default-Fallback wie
/// `audio_feedback::play_audio_file`).
fn open_output_stream(device: Option<String>) -> Result<rodio::OutputStream, String> {
    use rodio::OutputStreamBuilder;
    let stream_builder = match device {
        Some(name) if name != "Default" => {
            use cpal::traits::{DeviceTrait, HostTrait};
            let host = crate::audio_toolkit::get_cpal_host();
            let found = host
                .output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map(|n| n == name).unwrap_or(false));
            match found {
                Some(d) => OutputStreamBuilder::from_device(d).map_err(|e| e.to_string())?,
                None => {
                    log::warn!("TTS output device '{name}' not found, using default");
                    OutputStreamBuilder::from_default_device().map_err(|e| e.to_string())?
                }
            }
        }
        _ => OutputStreamBuilder::from_default_device().map_err(|e| e.to_string())?,
    };
    stream_builder.open_stream().map_err(|e| e.to_string())
}

/// Ein WAV als verschachtelte f32-Abtastwerte (Ganzzahl- oder Gleitkomma-PCM).
fn decode_interleaved(wav: &[u8]) -> Option<PcmSegment> {
    let reader = hound::WavReader::new(std::io::Cursor::new(wav)).ok()?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        return None;
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<_, _>>()
            .ok()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .ok()?
        }
    };
    Some(PcmSegment {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        samples,
    })
}

/// Ab Frame `from` zeitgestreckt anhängen.
fn append_stretched(sink: &rodio::Sink, pcm: &PcmSegment, from: usize, speed: f32) {
    let ch = pcm.channels as usize;
    let rest = &pcm.samples[(from * ch).min(pcm.samples.len())..];
    if rest.is_empty() {
        return;
    }
    let stretched = super::dsp::time_stretch(rest, pcm.channels, pcm.sample_rate, speed);
    sink.append(rodio::buffer::SamplesBuffer::new(
        pcm.channels,
        pcm.sample_rate,
        stretched,
    ));
}

/// Einen Satz mit gleichbleibender Tonhöhe abspielen. Dreht der Nutzer
/// mitten im Satz am Tempo, wird der noch nicht gehörte Rest neu gestreckt —
/// die Änderung wirkt sofort, nicht erst beim nächsten Satz.
fn play_stretched(
    stream: &rodio::OutputStream,
    pcm: PcmSegment,
    gain: f32,
    controls: &PlaybackControls,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let sink = rodio::Sink::connect_new(stream.mixer());
    let frames = pcm.samples.len() / pcm.channels.max(1) as usize;
    let mut applied_volume = controls.volume() * gain;
    sink.set_volume(applied_volume);
    let mut speed = controls.speed();
    // Ab welchem Frame das laufende Stück gestreckt wurde und seit wann es spielt.
    let mut from = 0usize;
    let mut started = std::time::Instant::now();
    append_stretched(&sink, &pcm, from, speed);
    while !sink.empty() {
        if cancelled.load(Ordering::Acquire) {
            sink.stop();
            break;
        }
        let wanted_volume = controls.volume() * gain;
        if (wanted_volume - applied_volume).abs() > f32::EPSILON {
            sink.set_volume(wanted_volume);
            applied_volume = wanted_volume;
        }
        let wanted_speed = controls.speed();
        if (wanted_speed - speed).abs() > f32::EPSILON {
            let heard = started.elapsed().as_secs_f64() * speed as f64 * pcm.sample_rate as f64;
            from = (from + heard as usize).min(frames);
            speed = wanted_speed;
            sink.clear();
            append_stretched(&sink, &pcm, from, speed);
            sink.play();
            started = std::time::Instant::now();
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}

impl Player for RodioPlayer {
    fn play(
        &self,
        wav: Vec<u8>,
        device: Option<String>,
        gain: f32,
        controls: Arc<PlaybackControls>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let stream = open_output_stream(device)?;
        // Tempo per Zeitstreckung statt `Sink::set_speed`: das resampelt und
        // zöge die Stimme hoch oder tief. Nur ein unlesbares WAV läuft noch
        // über den alten Weg.
        if let Some(pcm) = decode_interleaved(&wav) {
            return play_stretched(&stream, pcm, gain, &controls, &cancelled);
        }
        let sink =
            rodio::play(stream.mixer(), std::io::Cursor::new(wav)).map_err(|e| e.to_string())?;

        let mut applied_volume = controls.volume() * gain;
        let mut applied_speed = controls.speed();
        sink.set_volume(applied_volume);
        sink.set_speed(applied_speed);

        // Abbrechbar warten statt sleep_until_end: cancel() wirkt in <=50 ms.
        // Dieselbe Schleife übernimmt Regleränderungen — damit wirkt ein Dreh
        // am Tempo mitten im Satz und nicht erst beim nächsten.
        while !sink.empty() {
            if cancelled.load(Ordering::Acquire) {
                sink.stop();
                break;
            }
            let wanted_volume = controls.volume() * gain;
            if (wanted_volume - applied_volume).abs() > f32::EPSILON {
                sink.set_volume(wanted_volume);
                applied_volume = wanted_volume;
            }
            let wanted_speed = controls.speed();
            if (wanted_speed - applied_speed).abs() > f32::EPSILON {
                sink.set_speed(wanted_speed);
                applied_speed = wanted_speed;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Ok(())
    }

    /// Echte Stream-Wiedergabe: EIN Ausgabestrom, jedes eintreffende Stück
    /// wird sofort an die laufende Wiedergabe angehängt — kein Warten auf das
    /// Ende der Synthese. Abbruch wirkt binnen 50 ms und verwirft alles noch
    /// nicht Gespielte. Lautstärke und Tempo folgen den Reglern live.
    fn play_stream(
        &self,
        segments: std::sync::mpsc::Receiver<PcmSegment>,
        device: Option<String>,
        controls: Arc<PlaybackControls>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), String> {
        use std::sync::mpsc::RecvTimeoutError;
        let stream = open_output_stream(device)?;
        let sink = rodio::Sink::connect_new(stream.mixer());
        let mut applied_volume = controls.volume();
        sink.set_volume(applied_volume);
        // Tempo: jedes Stück wird beim Anhängen zeitgestreckt (gleiche
        // Tonhöhe); ein Dreh am Regler gilt ab dem nächsten Stück.
        let mut follow_controls = |sink: &rodio::Sink| {
            let volume = controls.volume();
            if (volume - applied_volume).abs() > f32::EPSILON {
                sink.set_volume(volume);
                applied_volume = volume;
            }
        };
        // Annehmen, solange der Sender lebt.
        loop {
            if cancelled.load(Ordering::Acquire) {
                sink.stop();
                return Ok(());
            }
            follow_controls(&sink);
            match segments.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(seg) => {
                    if seg.channels == 0 || seg.sample_rate == 0 || seg.samples.is_empty() {
                        continue;
                    }
                    let samples = super::dsp::time_stretch(
                        &seg.samples,
                        seg.channels,
                        seg.sample_rate,
                        controls.speed(),
                    );
                    sink.append(rodio::buffer::SamplesBuffer::new(
                        seg.channels,
                        seg.sample_rate,
                        samples,
                    ));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        // Ausspielen, was noch angehängt ist.
        while !sink.empty() {
            if cancelled.load(Ordering::Acquire) {
                sink.stop();
                break;
            }
            follow_controls(&sink);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Ok(())
    }
}

/// Testdouble: registriert nur die Byte-Zahl des letzten Auftrags.
#[cfg(test)]
pub struct CountingPlayer(pub std::sync::Mutex<usize>);

#[cfg(test)]
impl Player for CountingPlayer {
    fn play(
        &self,
        wav: Vec<u8>,
        _device: Option<String>,
        _gain: f32,
        _controls: Arc<PlaybackControls>,
        _cancelled: Arc<AtomicBool>,
    ) -> Result<(), String> {
        *self.0.lock().unwrap() = wav.len();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn die_regler_halten_sich_an_ihre_grenzen() {
        let c = PlaybackControls::default();
        c.set_speed(5.0);
        assert_eq!(c.speed(), 2.0);
        c.set_speed(0.1);
        assert_eq!(c.speed(), 0.5);
        c.set_volume(3.0);
        assert_eq!(c.volume(), 1.0);
        c.set_volume(-1.0);
        assert_eq!(c.volume(), 0.0);
    }

    /// Der Zweck der Atomics: ein anderer Thread aendert den Wert, und die
    /// Wiedergabe sieht ihn beim naechsten Blick — ohne Schloss, ohne Warten.
    #[test]
    fn eine_aenderung_ist_sofort_von_aussen_sichtbar() {
        let c = Arc::new(PlaybackControls::default());
        let writer = Arc::clone(&c);
        std::thread::spawn(move || writer.set_speed(1.75))
            .join()
            .unwrap();
        assert_eq!(c.speed(), 1.75);
    }

    /// Ein leiser Sinus-Ton als Stück (24 kHz mono).
    fn tone(samples: usize) -> PcmSegment {
        PcmSegment {
            sample_rate: 24_000,
            channels: 1,
            samples: (0..samples)
                .map(|i| 0.2 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 24_000.0).sin())
                .collect(),
        }
    }

    /// Rauchtest der echten Stream-Wiedergabe. Braucht ein Ausgabegerät und
    /// spielt kurz einen leisen Ton: `cargo test --lib -- --ignored rodio_stream`.
    /// Prüft, dass die Wiedergabe bis zum Ende der Stücke läuft und dass ein
    /// Abbruch binnen weniger hundert Millisekunden wirkt.
    #[test]
    #[ignore = "braucht ein Ausgabegerät"]
    fn rodio_stream_spielt_bis_zum_ende_und_bricht_ab() {
        use std::time::{Duration, Instant};
        let controls = Arc::new(PlaybackControls::default());
        controls.set_volume(0.15);

        // 1) Vier Stücke à 0,15 s, mit Lücke dazwischen geliefert: Gesamtdauer 0,6 s.
        let (tx, rx) = std::sync::mpsc::channel();
        let feeder = std::thread::spawn(move || {
            for _ in 0..4 {
                tx.send(tone(3600)).unwrap();
                std::thread::sleep(Duration::from_millis(40));
            }
        });
        let t0 = Instant::now();
        RodioPlayer
            .play_stream(rx, None, controls.clone(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        let took = t0.elapsed();
        feeder.join().unwrap();
        assert!(
            took >= Duration::from_millis(550),
            "endete zu früh: {took:?}"
        );
        assert!(took < Duration::from_secs(3), "hing: {took:?}");

        // 2) Ein 3-Sekunden-Stück, nach 200 ms abgebrochen.
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(tone(24_000 * 3)).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::Release);
        });
        let t0 = Instant::now();
        RodioPlayer.play_stream(rx, None, controls, cancel).unwrap();
        assert!(
            t0.elapsed() < Duration::from_millis(800),
            "{:?}",
            t0.elapsed()
        );
        drop(tx);
    }
}
