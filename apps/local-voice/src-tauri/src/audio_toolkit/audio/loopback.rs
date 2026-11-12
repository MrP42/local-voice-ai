//! WASAPI-Loopback-Capture des Default-Render-Endpoints (Systemton).
//!
//! Liefert 16-kHz-Mono-i16-Blöcke an einen Callback. Die puren Hilfsfunktionen
//! (`downmix_to_mono`, `f32_to_i16`) sind plattformunabhängig und getestet; der
//! eigentliche Capture-Thread ist Windows-only (Abnahme im Harness, Task 15).

/// Ziel-Samplerate der Meeting-Pipeline.
pub const TARGET_SAMPLE_RATE: usize = 16_000;

/// Pure: f32-interleaved mit beliebiger Kanalzahl -> Mono (Mittelwert je Frame).
pub fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// QPC-Positionen von WASAPI (`BufferInfo::timestamp`) und cpal
/// (`InputCallbackInfo::timestamp().capture` auf Windows) zählen in 100 ns.
pub const QPC_UNITS_PER_SEC: i128 = 10_000_000;

/// M2-P2c2: Zeitstempel für resampelte Ausgabeblöcke. Die Capture-Threads
/// kennen den QPC-Zeitstempel je Geräte-Puffer; weitergereicht werden aber
/// 30-ms-Blöcke nach dem Resampler. Der Stempler rechnet für jeden Block den
/// QPC-Zeitpunkt seines ersten Samples aus dem zuletzt gesehenen Geräte-Stempel
/// (lineare Fortschreibung mit der Nennrate, Fehler bei 30 ms Abstand im
/// µs-Bereich). Rein, ohne Allokation, ohne Sperre: läuft im Capture-Thread.
///
/// Ablauf je Geräte-Puffer: `mark(qpc, frames_ahead)` (Stempel des ersten
/// Frames dieses Puffers, `frames_ahead` = davor noch einzuschiebende
/// Stille-Frames), dann `advance_input` für alles, was in den Resampler geht,
/// und je ausgegebenem Block `next_block(len)`.
#[derive(Debug, Clone)]
pub struct QpcStamper {
    in_rate: u64,
    out_rate: u64,
    in_frames: u64,
    out_samples: u64,
    /// (Eingangs-Frame-Index, QPC in 100 ns)
    anchor: Option<(u64, u64)>,
}

impl QpcStamper {
    pub fn new(in_rate: usize, out_rate: usize) -> Self {
        Self {
            in_rate: in_rate.max(1) as u64,
            out_rate: out_rate.max(1) as u64,
            in_frames: 0,
            out_samples: 0,
            anchor: None,
        }
    }

    /// Setzt den Anker: der Eingangs-Frame `in_frames + frames_ahead` wurde zum
    /// Zeitpunkt `qpc` aufgenommen. `None`/0 (Gerät meldet keinen gültigen
    /// Stempel) lässt den alten Anker stehen.
    pub fn mark(&mut self, qpc: Option<u64>, frames_ahead: u64) {
        if let Some(q) = qpc.filter(|q| *q > 0) {
            self.anchor = Some((self.in_frames + frames_ahead, q));
        }
    }

    /// So viele Eingangs-Frames (Gerätetakt) gehen in den Resampler.
    pub fn advance_input(&mut self, frames: u64) {
        self.in_frames += frames;
    }

    /// Stempel des nächsten Ausgabeblocks (`len` Samples im Zieltakt), oder
    /// `None`, solange kein Anker bekannt ist.
    pub fn next_block(&mut self, len: usize) -> Option<u64> {
        let stamp = self.anchor.and_then(|(i0, q0)| {
            let in_idx = self.out_samples as i128 * self.in_rate as i128 / self.out_rate as i128;
            let q = q0 as i128 + (in_idx - i0 as i128) * QPC_UNITS_PER_SEC / self.in_rate as i128;
            u64::try_from(q).ok()
        });
        self.out_samples += len as u64;
        stamp
    }
}

/// Toleranz auf die verstrichene Wanduhrzeit: so viel Verzug (Planung des
/// Capture-Threads unter Last, Pufferlatenz des Treibers) darf die gemeldete
/// Lücke größer sein als die Zeit seit dem letzten gelesenen Paket.
pub const PAD_PLAUSIBILITY_SLACK: std::time::Duration = std::time::Duration::from_secs(2);

/// #15, Sanity-Cap: die größte Lücke (in Frames beim Gerätetakt `rate`), die
/// nach `elapsed` Wanduhrzeit seit dem letzten Paket überhaupt echt sein kann.
/// Die Gerätezeit läuft nie schneller als die Uhr; ein größerer Sprung ist ein
/// korrupter Positionswert und würde den Thread sonst Stunden Stille schieben
/// lassen. Rein, ohne Allokation: läuft im Capture-Thread.
pub fn max_plausible_pad_frames(elapsed: std::time::Duration, rate: usize) -> u64 {
    let nanos = elapsed.saturating_add(PAD_PLAUSIBILITY_SLACK).as_nanos();
    let frames = nanos.saturating_mul(rate as u128) / 1_000_000_000;
    u64::try_from(frames).unwrap_or(u64::MAX)
}

/// Pure: f32 [-1, 1] -> i16 mit Clamping (Werte außerhalb werden begrenzt).
pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&s| {
            let scaled = if s < 0.0 { s * 32768.0 } else { s * 32767.0 };
            scaled.clamp(-32768.0, 32767.0) as i16
        })
        .collect()
}

/// Ausgabeblockdauer: 30 ms bei 16 kHz = 480 Samples.
const FRAME_DURATION: std::time::Duration = std::time::Duration::from_millis(30);
/// Stille wird in Häppchen dieser Größe in den Resampler geschoben.
const SILENCE_CHUNK_FRAMES: usize = 4096;
/// Ab so langer Stille (Wanduhr bzw. QPC) ohne Paket wird aufgefüllt. Darunter sind es
/// Schwankungen der Paketankunft (Planung des Threads, Pufferlatenz), kein Schweigen.
pub const QUIET_GAP_MIN: std::time::Duration = std::time::Duration::from_millis(250);

/// Ein Geräte-Paket, wie der Capture-Thread es liest (#537).
#[derive(Clone, Copy, Debug)]
pub struct DevicePacket<'a> {
    /// Interleaved f32 beim Gerätetakt; `SILENT`-Pakete schon als Nullen.
    pub interleaved: &'a [f32],
    /// Position des ersten Frames in Frames seit Stream-Start (`BufferInfo::index`).
    pub device_position: u64,
    /// QPC (100 ns) des ersten Frames; `None`, wenn das Gerät keinen gültigen nennt.
    pub qpc: Option<u64>,
    /// Wanduhr seit dem Start des Streams (nur für Stille ohne Paket und die
    /// Plausibilitätsgrenze; die Zeitachse bleibt Geräteposition bzw. QPC).
    pub since_start: std::time::Duration,
}

/// Der plattformunabhängige Kern des Capture-Threads: Pakete rein, zeitachsen-
/// korrekte 16-kHz-Mono-Blöcke samt QPC-Stempel raus.
///
/// WASAPI-Loopback liefert bei Stille GAR KEINE Pakete (gemessen auf Windows: nach
/// drei Sekunden ohne Ton null Blöcke, kein Fehler). Die Zeitachse darf deshalb
/// nicht an den Paketen hängen: Stille VOR dem ersten Paket und Pausen zwischen zwei
/// Paketen werden mit Nullen aufgefüllt, und zwar nach der größeren von zwei
/// Angaben: Lücke der Geräteposition (wo der Treiber sie mitzählt) und Lücke der
/// QPC-Stempel bzw. der Wanduhr (wo er es nicht tut). Ohne das verrutschen alle
/// Offsets des Transkripts um die Stille, die nie ankam.
pub struct LoopbackFeeder {
    rate: usize,
    channels: usize,
    resampler: crate::audio_toolkit::audio::FrameResampler,
    stamper: QpcStamper,
    timeline: crate::audio_toolkit::audio::LoopbackTimeline,
    silence: Vec<f32>,
    first: bool,
    last_since_start: std::time::Duration,
    /// QPC, bei dem das nächste Paket ohne Lücke beginnen müsste.
    next_qpc: Option<u64>,
    padded_frames: u64,
    dropped_buffers: u64,
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
impl LoopbackFeeder {
    pub fn new(mix_rate: usize, channels: usize) -> Self {
        Self {
            rate: mix_rate.max(1),
            channels: channels.max(1),
            resampler: crate::audio_toolkit::audio::FrameResampler::new(
                mix_rate,
                TARGET_SAMPLE_RATE,
                FRAME_DURATION,
            ),
            stamper: QpcStamper::new(mix_rate, TARGET_SAMPLE_RATE),
            timeline: crate::audio_toolkit::audio::LoopbackTimeline::new(),
            silence: vec![0.0f32; SILENCE_CHUNK_FRAMES],
            first: true,
            last_since_start: std::time::Duration::ZERO,
            next_qpc: None,
            padded_frames: 0,
            dropped_buffers: 0,
        }
    }

    /// Die Stille (in Frames beim Gerätetakt), die VOR diesem Paket nie ankam und
    /// über die Geräteposition nicht zu erkennen ist: vor dem ersten Paket die
    /// Wanduhr seit dem Start, danach die Lücke der QPC-Stempel. Unter
    /// [`QUIET_GAP_MIN`] ist es keine Stille; über `max_pad` kein Schweigen mehr,
    /// sondern ein kaputter Wert.
    fn clock_gap(
        &self,
        frames: u64,
        qpc: Option<u64>,
        since_start: std::time::Duration,
        max_pad: u64,
    ) -> u64 {
        let rate = self.rate as u64;
        let gap = if self.first {
            let wall = since_start.as_nanos().saturating_mul(rate as u128) / 1_000_000_000;
            u64::try_from(wall)
                .unwrap_or(u64::MAX)
                .saturating_sub(frames)
        } else {
            match (qpc, self.next_qpc) {
                (Some(q), Some(expected)) if q > expected => {
                    (q - expected).saturating_mul(rate) / QPC_UNITS_PER_SEC as u64
                }
                _ => 0,
            }
        };
        let min = (QUIET_GAP_MIN.as_millis() as u64).saturating_mul(rate) / 1_000;
        if gap >= min {
            gap.min(max_pad)
        } else {
            0
        }
    }

    /// Ein Paket verarbeiten; `sink` bekommt die fertigen 16-kHz-Blöcke mit QPC-Stempel
    /// (der Block, der das erste Sample enthält, mit dessen Zeitpunkt).
    pub fn on_packet(
        &mut self,
        packet: DevicePacket<'_>,
        sink: &mut impl FnMut(&[i16], Option<u64>),
    ) {
        let frames = (packet.interleaved.len() / self.channels) as u64;
        if frames == 0 {
            return;
        }
        let since_last = packet.since_start.saturating_sub(self.last_since_start);
        self.last_since_start = packet.since_start;
        // #15: die Grenze folgt der Wanduhr seit dem letzten Paket.
        let max_pad = max_plausible_pad_frames(since_last, self.rate);
        let capped_before = self.timeline.capped_gaps();
        let position_pad =
            match self
                .timeline
                .on_buffer_capped(packet.device_position, frames, max_pad)
            {
                crate::audio_toolkit::audio::TimelineAction::Drop => {
                    self.dropped_buffers += 1;
                    if self.dropped_buffers % 100 == 1 {
                        log::warn!(
                            "loopback: backwards device position, dropped {} buffer(s)",
                            self.dropped_buffers
                        );
                    }
                    return;
                }
                crate::audio_toolkit::audio::TimelineAction::PadSilence(gap) => gap,
                crate::audio_toolkit::audio::TimelineAction::Append => 0,
            };
        if self.timeline.capped_gaps() != capped_before {
            let capped = self.timeline.capped_gaps();
            if capped % 100 == 1 {
                log::warn!(
                    "loopback: implausible position jump capped at {position_pad} frames ({capped} so far, {} frames not padded)",
                    self.timeline.skipped_frames()
                );
            }
        }
        let clock_pad = self.clock_gap(frames, packet.qpc, packet.since_start, max_pad);
        let pad_frames = position_pad.max(clock_pad);
        self.next_qpc = packet.qpc.map(|q| {
            q.saturating_add(frames.saturating_mul(QPC_UNITS_PER_SEC as u64) / self.rate as u64)
        });
        self.first = false;

        // M2-P2c2: QPC des ersten Frames DIESES Pakets (hinter der Stille); die
        // eingeschobene Stille bekommt davon zurückgerechnete Stempel.
        self.stamper.mark(packet.qpc, pad_frames);
        if pad_frames > 0 {
            self.padded_frames += pad_frames;
            log::debug!(
                "loopback: gap of {pad_frames} frames padded with silence (total {})",
                self.padded_frames
            );
            let mut remaining = pad_frames;
            while remaining > 0 {
                let take = remaining.min(SILENCE_CHUNK_FRAMES as u64) as usize;
                self.stamper.advance_input(take as u64);
                let Self {
                    resampler,
                    stamper,
                    silence,
                    ..
                } = self;
                resampler.push(&silence[..take], |frame| {
                    let stamp = stamper.next_block(frame.len());
                    sink(&f32_to_i16(frame), stamp)
                });
                remaining -= take as u64;
            }
        }

        let mono = downmix_to_mono(packet.interleaved, self.channels);
        self.stamper.advance_input(mono.len() as u64);
        let Self {
            resampler, stamper, ..
        } = self;
        resampler.push(&mono, |frame| {
            let stamp = stamper.next_block(frame.len());
            sink(&f32_to_i16(frame), stamp)
        });
    }

    /// Ende des Streams: der Rest im Resampler geht noch hinaus.
    pub fn finish(&mut self, sink: &mut impl FnMut(&[i16], Option<u64>)) {
        let Self {
            resampler, stamper, ..
        } = self;
        resampler.finish(|frame| {
            let stamp = stamper.next_block(frame.len());
            sink(&f32_to_i16(frame), stamp)
        });
    }

    /// Für das Log am Ende: (gepolsterte Frames, verworfene Pakete, gekürzte Lücken,
    /// dabei nicht eingeschobene Frames).
    pub fn summary(&self) -> (u64, u64, u64, u64) {
        (
            self.padded_frames,
            self.dropped_buffers,
            self.timeline.capped_gaps(),
            self.timeline.skipped_frames(),
        )
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{DevicePacket, LoopbackFeeder};
    use anyhow::{anyhow, Result};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use wasapi::{initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

    /// Wartezeit auf das WASAPI-Event; läuft sie ab, prüfen wir das Stop-Flag.
    const EVENT_TIMEOUT_MS: u32 = 200;

    /// Startet Loopback-Capture des Default-Render-Endpoints. Liefert
    /// 16-kHz-Mono-i16-Blöcke (zeitachsen-korrekt inkl. Silence-Padding) an den
    /// Callback, bis `stop()` gerufen wird. Zweites Argument: QPC-Zeitstempel
    /// (100 ns) des ersten Samples im Block, `None`, solange WASAPI keinen
    /// gültigen Stempel geliefert hat (M2-P2c2, gemeinsame Zeitachse mit dem
    /// Mikrofon für die Echo-Unterdrückung).
    pub struct LoopbackCapture {
        stop: Arc<AtomicBool>,
        /// Wird gesetzt, wenn der Capture-Thread NACH erfolgreichem Start mit
        /// einem Fehler endet (Endpoint entfernt, Treiberfehler). Ohne dieses
        /// Flag verstummt der Callback still und die Aufnahme laeuft ohne
        /// Systemton weiter, ohne dass es jemand merkt.
        error: Arc<AtomicBool>,
        handle: Option<JoinHandle<()>>,
    }

    impl LoopbackCapture {
        pub fn start(on_samples: impl FnMut(&[i16], Option<u64>) + Send + 'static) -> Result<Self> {
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let error = Arc::new(AtomicBool::new(false));
            let thread_error = Arc::clone(&error);
            // Der COM-Init und das Öffnen des Endpoints müssen IM Thread passieren
            // (MTA gilt pro Thread); das Ergebnis kommt über diesen Kanal zurück,
            // damit start() echte Fehler melden kann statt still zu scheitern.
            let (init_tx, init_rx) = mpsc::channel::<Result<(), String>>();

            let handle = std::thread::Builder::new()
                .name("loopback-capture".to_string())
                .spawn(move || {
                    if let Err(e) = capture_loop(thread_stop, on_samples, &init_tx) {
                        log::error!("loopback capture ended with error: {e:#}");
                        thread_error.store(true, Ordering::Relaxed);
                        // Falls der Fehler vor der Init-Meldung auftrat, hier melden.
                        let _ = init_tx.send(Err(format!("{e:#}")));
                    }
                })?;

            match init_rx.recv() {
                Ok(Ok(())) => Ok(LoopbackCapture {
                    stop,
                    error,
                    handle: Some(handle),
                }),
                Ok(Err(e)) => {
                    let _ = handle.join();
                    Err(anyhow!("loopback capture failed to start: {e}"))
                }
                Err(_) => {
                    let _ = handle.join();
                    Err(anyhow!("loopback capture thread died before startup"))
                }
            }
        }

        /// True, sobald der Capture-Thread mit einem Fehler geendet ist.
        pub fn had_error(&self) -> bool {
            self.error.load(Ordering::Relaxed)
        }

        /// Beide Flags fuer einen Beobachter (Stop, Fehler) - damit ein
        /// Wachthread den stillen Ausfall bemerken kann, ohne die Capture
        /// selbst zu besitzen.
        pub fn watch_flags(&self) -> (Arc<AtomicBool>, Arc<AtomicBool>) {
            (Arc::clone(&self.stop), Arc::clone(&self.error))
        }

        pub fn stop(mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    impl Drop for LoopbackCapture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn capture_loop(
        stop: Arc<AtomicBool>,
        mut on_samples: impl FnMut(&[i16], Option<u64>) + Send + 'static,
        init_tx: &mpsc::Sender<Result<(), String>>,
    ) -> Result<()> {
        initialize_mta()
            .ok()
            .map_err(|e| anyhow!("CoInitializeEx (MTA) failed: {e}"))?;

        let enumerator = DeviceEnumerator::new()?;
        // Loopback = Capture-Richtung auf dem RENDER-Endpoint; die wasapi-Crate
        // setzt daraus AUDCLNT_STREAMFLAGS_LOOPBACK.
        let device = enumerator.get_default_device(&Direction::Render)?;
        let mut audio_client = device.get_iaudioclient()?;

        let mix_format = audio_client.get_mixformat()?;
        let mix_rate = mix_format.get_samplespersec() as usize;
        let channels = mix_format.get_nchannels() as usize;
        // Wir verlangen f32 bei Mix-Rate/-Kanalzahl und lassen WASAPI notfalls
        // konvertieren (autoconvert), damit der Puffer immer f32-interleaved ist.
        let desired_format = WaveFormat::new(32, 32, &SampleType::Float, mix_rate, channels, None);
        let block_align = desired_format.get_blockalign() as usize;

        let (_default_period, min_period) = audio_client.get_device_period()?;
        let mode = StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: min_period,
        };
        audio_client.initialize_client(&desired_format, &Direction::Capture, &mode)?;

        let h_event = audio_client.set_get_eventhandle()?;
        let capture_client = audio_client.get_audiocaptureclient()?;
        audio_client.start_stream()?;

        // Ab hier steht der Stream — Startmeldung an start().
        let _ = init_tx.send(Ok(()));

        let mut feeder = LoopbackFeeder::new(mix_rate, channels);
        let mut byte_buf: Vec<u8> = Vec::new();
        // #537: Stille VOR dem ersten Paket und Pausen ohne Paket zaehlen ab hier
        // (WASAPI-Loopback liefert bei Stille gar keine Pakete).
        let started_at = std::time::Instant::now();

        while !stop.load(Ordering::Relaxed) {
            // Alle bereitstehenden Pakete abholen, dann aufs nächste Event warten.
            loop {
                let next_frames = capture_client.get_next_packet_size()?.unwrap_or(0) as usize;
                if next_frames == 0 {
                    break;
                }
                let needed = next_frames * block_align;
                if byte_buf.len() < needed {
                    byte_buf.resize(needed, 0);
                }
                let (frames_read, info) =
                    capture_client.read_from_device(&mut byte_buf[..needed])?;
                if frames_read == 0 {
                    break;
                }
                let valid = frames_read as usize * block_align;
                if info.flags.silent {
                    // SILENT: Inhalt ist bedeutungslos -> Nullen. Die Position
                    // zählt trotzdem normal weiter, also NICHT überspringen.
                    byte_buf[..valid].fill(0);
                }

                // SPEC C1: Die Zeitachse kommt aus der Device-Position und den
                // QPC-Stempeln des Geraets (`LoopbackFeeder`), nie aus gezaehlten
                // Buffern; die Wanduhr dient nur der Stille ohne Paket und als
                // Plausibilitaetsgrenze.
                let interleaved: Vec<f32> = byte_buf[..valid]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                let qpc =
                    (!info.flags.timestamp_error && info.timestamp > 0).then_some(info.timestamp);
                feeder.on_packet(
                    DevicePacket {
                        interleaved: &interleaved,
                        device_position: info.index,
                        qpc,
                        since_start: started_at.elapsed(),
                    },
                    &mut on_samples,
                );
            }

            // Timeout ist normal (kein Ton) — dann nur das Stop-Flag prüfen.
            let _ = h_event.wait_for_event(EVENT_TIMEOUT_MS);
        }

        feeder.finish(&mut on_samples);
        audio_client.stop_stream()?;
        let (padded_frames, dropped_buffers, capped_gaps, skipped_frames) = feeder.summary();
        log::info!(
            "loopback capture stopped (padded {padded_frames} silence frames, dropped {dropped_buffers} buffers, capped {capped_gaps} implausible gaps = {skipped_frames} frames not padded)"
        );
        Ok(())
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::LoopbackCapture;

#[cfg(not(target_os = "windows"))]
mod stub_impl {
    use anyhow::{anyhow, Result};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    /// Auf Nicht-Windows-Plattformen gibt es in M8 keinen Loopback-Capture.
    pub struct LoopbackCapture {
        _private: (),
    }

    impl LoopbackCapture {
        pub fn start(
            _on_samples: impl FnMut(&[i16], Option<u64>) + Send + 'static,
        ) -> Result<Self> {
            Err(anyhow!("loopback capture is windows-only in M8"))
        }

        pub fn stop(self) {}

        pub fn had_error(&self) -> bool {
            false
        }

        pub fn watch_flags(&self) -> (Arc<AtomicBool>, Arc<AtomicBool>) {
            (
                Arc::new(AtomicBool::new(true)),
                Arc::new(AtomicBool::new(false)),
            )
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub use stub_impl::LoopbackCapture;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_downmix_averages_the_channels() {
        let mono = downmix_to_mono(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0], 2);
        assert_eq!(mono, vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn i16_conversion_clamps_out_of_range() {
        let out = f32_to_i16(&[0.0, 1.0, -1.0, 2.0, -2.0]);
        assert_eq!(out, vec![0, 32767, -32768, 32767, -32768]);
    }

    // ---- #15: Sanity-Cap der Stille-Polsterung ---------------------------------

    #[test]
    fn the_padding_cap_follows_the_wall_clock_plus_a_fixed_slack() {
        use std::time::Duration;
        // 10 s seit dem letzten Paket bei 48 kHz: 10 s + 2 s Toleranz.
        assert_eq!(
            max_plausible_pad_frames(Duration::from_secs(10), 48_000),
            12 * 48_000
        );
        // Sofort nach dem Paket: nur die Toleranz.
        assert_eq!(max_plausible_pad_frames(Duration::ZERO, 16_000), 2 * 16_000);
        // Stunden Stille ohne Paket (echt: nichts spielt) bleiben zulässig ...
        assert_eq!(
            max_plausible_pad_frames(Duration::from_secs(3 * 3_600), 48_000),
            (3 * 3_600 + 2) * 48_000
        );
        // ... und eine absurde Dauer/Rate rechnet saturierend statt zu laufen über.
        assert_eq!(
            max_plausible_pad_frames(Duration::from_secs(u64::MAX / 2), usize::MAX),
            u64::MAX
        );
    }

    // ---- M2-P2c2: QPC-Stempel je Ausgabeblock --------------------------------

    #[test]
    fn stamper_gives_each_output_block_the_time_of_its_first_sample() {
        // 48 kHz rein, 16 kHz raus, 30-ms-Bloecke = 480 Samples.
        let mut s = QpcStamper::new(48_000, 16_000);
        assert_eq!(s.next_block(480), None, "ohne Anker kein Stempel");
        let mut s = QpcStamper::new(48_000, 16_000);
        s.mark(Some(1_000_000), 0);
        s.advance_input(2_880);
        assert_eq!(s.next_block(480), Some(1_000_000));
        assert_eq!(
            s.next_block(480),
            Some(1_000_000 + 300_000),
            "30 ms spaeter"
        );
        // Naechster Geraetepuffer mit eigenem Stempel (Uhr lief 1 ms vor):
        // ab jetzt zaehlt der neue Anker.
        s.mark(Some(1_000_000 + 600_000 + 10_000), 0);
        s.advance_input(1_440);
        assert_eq!(s.next_block(480), Some(1_000_000 + 600_000 + 10_000));
    }

    #[test]
    fn stamper_back_dates_padded_silence_before_the_buffer() {
        // 100 ms Stille (4800 Frames bei 48 kHz) vor einem Puffer mit Stempel T:
        // der erste Stille-Block liegt bei T - 100 ms.
        let mut s = QpcStamper::new(48_000, 16_000);
        s.mark(Some(5_000_000), 4_800);
        s.advance_input(4_800 + 480);
        assert_eq!(s.next_block(480), Some(5_000_000 - 1_000_000));
        // Ein Stempel vor 0 waere unsinnig: kein Stempel statt Unterlauf.
        let mut s = QpcStamper::new(16_000, 16_000);
        s.mark(Some(100), 16_000);
        assert_eq!(s.next_block(160), None);
    }

    #[test]
    fn an_invalid_device_stamp_keeps_the_previous_anchor() {
        let mut s = QpcStamper::new(16_000, 16_000);
        s.mark(Some(2_000_000), 0);
        s.advance_input(480);
        assert_eq!(s.next_block(480), Some(2_000_000));
        s.mark(None, 0);
        s.mark(Some(0), 0);
        s.advance_input(480);
        assert_eq!(s.next_block(480), Some(2_000_000 + 300_000));
    }

    // ---- #537: Stille ohne Paket (WASAPI-Loopback) ------------------------------

    /// Ein Geraetepaket der Nachbildung: Wanduhr seit Stream-Start, Frames, Position,
    /// QPC (100 ns) und Pegel (Gleichanteil; 0.5 = klar hoerbar).
    struct Pkt {
        wall_ms: u64,
        frames: usize,
        pos: u64,
        qpc: Option<u64>,
        level: f32,
    }

    /// Schiebt die Pakete durch den Feeder, sammelt Samples und Block-Stempel
    /// (Index des ersten Samples des Blocks, QPC).
    fn feed(rate: usize, channels: usize, pkts: &[Pkt]) -> (Vec<i16>, Vec<(usize, Option<u64>)>) {
        let mut feeder = LoopbackFeeder::new(rate, channels);
        let mut out: Vec<i16> = Vec::new();
        let mut stamps: Vec<(usize, Option<u64>)> = Vec::new();
        {
            let mut sink = |block: &[i16], stamp: Option<u64>| {
                stamps.push((out.len(), stamp));
                out.extend_from_slice(block);
            };
            for p in pkts {
                let data = vec![p.level; p.frames * channels];
                feeder.on_packet(
                    DevicePacket {
                        interleaved: &data,
                        device_position: p.pos,
                        qpc: p.qpc,
                        since_start: std::time::Duration::from_millis(p.wall_ms),
                    },
                    &mut sink,
                );
            }
            feeder.finish(&mut sink);
        }
        (out, stamps)
    }

    /// `n` zusammenhaengende 10-ms-Pakete bei 16 kHz (160 Frames), ab Wanduhr/Position/QPC.
    fn run_of(n: u64, wall_ms: u64, pos: u64, qpc: Option<u64>) -> Vec<Pkt> {
        (0..n)
            .map(|k| Pkt {
                wall_ms: wall_ms + 10 * k,
                frames: 160,
                pos: pos + 160 * k,
                qpc: qpc.map(|q| q + 100_000 * k),
                level: 0.5,
            })
            .collect()
    }

    /// Stellen, an denen der Pegel von leise auf laut springt (Beginn eines Tons).
    fn onsets(out: &[i16]) -> Vec<usize> {
        let loud = |x: i16| x.unsigned_abs() > 8_000;
        (0..out.len())
            .filter(|&i| loud(out[i]) && (i == 0 || !loud(out[i - 1])))
            .collect()
    }

    const Q0: u64 = 1_000_000_000;

    /// Der Befund von der echten Probe: kein Paket in der Stille. Das erste Paket
    /// kommt erst nach 10 s; der Ton muss bei ~10 s im Transkript stehen, nicht bei 0.
    /// Danach eine Pause von 5 s ohne Paket: danach liegt der Ton wieder 5 s spaeter.
    /// Der Treiber zaehlt die Stille NICHT in der Position mit (Position laeuft nur
    /// ueber gelieferte Pakete), die QPC-Stempel tun es.
    #[test]
    fn silence_before_the_first_packet_and_a_pause_are_padded_with_zeros_by_qpc() {
        let mut pkts = run_of(100, 10_000, 0, Some(Q0));
        // 1 s Ton, dann 5 s ohne Paket, Position laeuft nicht weiter.
        pkts.extend(run_of(
            100,
            16_000,
            100 * 160,
            Some(Q0 + 100 * 100_000 + 50_000_000),
        ));
        let (out, stamps) = feed(16_000, 1, &pkts);
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "zwei Toene erwartet: {on:?}");
        // Stille vor dem ersten Paket: 10 s Wanduhr minus die 10 ms des Pakets.
        assert!(
            on[0].abs_diff(159_840) <= 480,
            "erster Ton bei {} statt ~159 840",
            on[0]
        );
        // 1 s Ton, 5 s Pause: der zweite Ton folgt 80 000 Samples nach dem Ende des ersten.
        assert!(
            on[1].abs_diff(159_840 + 16_000 + 80_000) <= 480,
            "zweiter Ton bei {}",
            on[1]
        );
        // Alles davor sind Nullen, nicht verschluckte Zeit.
        assert!(out[..on[0] - 480].iter().all(|&x| x == 0));
        // Die QPC-Stempel gehen mit: der erste Stille-Block liegt um die Stille VOR dem
        // ersten Paket (159 840 Frames = 99 900 000 x 100 ns), der Block mit dem Ton auf Q0.
        assert_eq!(stamps[0], (0, Some(Q0 - 99_900_000)));
        assert!(stamps.contains(&(159_840, Some(Q0))));
    }

    /// Wie oben, aber der Treiber zaehlt die Stille in der Position mit und liefert
    /// keinen gueltigen QPC: die Luecke der Position traegt die Pause.
    #[test]
    fn a_pause_is_padded_from_the_device_position_when_there_is_no_qpc() {
        let mut pkts = run_of(100, 10_000, 0, None);
        pkts.extend(run_of(100, 16_000, 100 * 160 + 80_000, None));
        let (out, stamps) = feed(16_000, 1, &pkts);
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        assert!(on[0].abs_diff(159_840) <= 480, "{on:?}");
        assert!(on[1].abs_diff(159_840 + 16_000 + 80_000) <= 480, "{on:?}");
        assert!(
            stamps.iter().all(|(_, s)| s.is_none()),
            "ohne QPC kein Stempel"
        );
    }

    /// Ton ab dem ersten Paket, nur eine Pause in der Mitte: der Anfang bleibt bei 0.
    #[test]
    fn a_stream_that_starts_with_sound_has_no_leading_padding() {
        let mut pkts = run_of(50, 0, 0, Some(Q0));
        pkts.extend(run_of(
            50,
            3_000,
            50 * 160,
            Some(Q0 + 50 * 100_000 + 25_000_000),
        ));
        let (out, _) = feed(16_000, 1, &pkts);
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        assert!(on[0] <= 480, "Ton ab Anfang, nicht bei {}", on[0]);
        // 0,5 s Ton, 2,5 s Pause.
        assert!(on[1].abs_diff(8_000 + 40_000) <= 480, "{on:?}");
    }

    /// Ein Haenger der Planung des Capture-Threads (Wanduhr springt 400 ms, die Pakete
    /// selbst schliessen nahtlos an) ist KEINE Stille: nichts wird eingeschoben.
    #[test]
    fn a_scheduling_stall_is_not_silence() {
        let mut pkts = run_of(50, 0, 0, Some(Q0));
        pkts.extend(run_of(50, 500 + 400, 50 * 160, Some(Q0 + 50 * 100_000)));
        let (out, _) = feed(16_000, 1, &pkts);
        assert_eq!(onsets(&out).len(), 1, "ein durchgehender Ton");
        // 100 Pakete a 160 Frames, nichts zusaetzlich (bis auf den Rest im Block).
        assert!(out.len().abs_diff(16_000) <= 480, "{} Samples", out.len());
    }

    /// Ein kurzer Anlauf (Treiber braucht 100 ms bis zum ersten Paket) wird nicht
    /// als Stille aufgefuellt.
    #[test]
    fn a_short_start_up_delay_is_not_padded() {
        let pkts = run_of(100, 100, 0, Some(Q0));
        let (out, _) = feed(16_000, 1, &pkts);
        assert!(onsets(&out)[0] <= 480);
        assert!(out.len().abs_diff(16_000) <= 480, "{} Samples", out.len());
    }

    /// Der Weg ueber den Resampler (48 kHz Stereo wie ein echtes Geraet): 3 s Stille am
    /// Anfang, danach 1 s Ton, 2 s Pause, 1 s Ton. Die Offsets stehen beim 16-kHz-Ausgang.
    #[test]
    fn the_padding_survives_resampling_from_48_khz_stereo() {
        let packet = |wall_ms: u64, pos: u64, qpc: u64| Pkt {
            wall_ms,
            frames: 480,
            pos,
            qpc: Some(qpc),
            level: 0.5,
        };
        // 480 Frames bei 48 kHz = 10 ms = 100 000 (100 ns).
        let first: Vec<Pkt> = (0..100)
            .map(|k| packet(3_000 + 10 * k, 480 * k, Q0 + 100_000 * k))
            .collect();
        let second: Vec<Pkt> = (0..100)
            .map(|k| {
                packet(
                    6_010 + 10 * k,
                    480 * (100 + k),
                    Q0 + 100_000 * (100 + k) + 20_000_000,
                )
            })
            .collect();
        let pkts: Vec<Pkt> = first.into_iter().chain(second).collect();
        let (out, _) = feed(48_000, 2, &pkts);
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        // 3 s minus 10 ms Stille, dann der Ton (der Resampler verschmiert um einige Millisekunden).
        assert!(on[0].abs_diff(47_840) <= 1_600, "erster Ton bei {}", on[0]);
        // Ton 1 s, Pause 2 s: der zweite Ton 3 s nach dem Beginn des ersten.
        assert!(
            on[1].abs_diff(on[0] + 16_000 + 32_000) <= 1_600,
            "zweiter Ton bei {} nach {}",
            on[1],
            on[0]
        );
    }

    /// Ein korrupter QPC-Sprung (Stunden) wird wie ein korrupter Positionssprung auf die
    /// Wanduhr-Grenze gekuerzt, nicht stundenlang aufgefuellt.
    #[test]
    fn an_absurd_qpc_gap_is_capped_by_the_wall_clock() {
        let mut pkts = run_of(10, 0, 0, Some(Q0));
        pkts.extend(run_of(
            10,
            200,
            10 * 160,
            Some(Q0 + 10 * 100_000 + 36_000_000_000),
        ));
        let (out, _) = feed(16_000, 1, &pkts);
        // Hoechstens Wanduhr (0,1 s) + 2 s Toleranz Stille dazwischen, nie Stunden.
        assert!(out.len() < 16_000 * 3, "{} Samples", out.len());
    }
}
