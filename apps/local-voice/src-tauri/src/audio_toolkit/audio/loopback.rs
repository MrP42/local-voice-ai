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

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{
        downmix_to_mono, f32_to_i16, max_plausible_pad_frames, QpcStamper, TARGET_SAMPLE_RATE,
    };
    use crate::audio_toolkit::audio::{FrameResampler, LoopbackTimeline, TimelineAction};
    use anyhow::{anyhow, Result};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Duration;
    use wasapi::{initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

    /// Ausgabeblockdauer: 30 ms bei 16 kHz = 480 Samples.
    const FRAME_DURATION: Duration = Duration::from_millis(30);
    /// Stille wird in Häppchen dieser Größe in den Resampler geschoben.
    const SILENCE_CHUNK_FRAMES: usize = 4096;
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

        let mut resampler = FrameResampler::new(mix_rate, TARGET_SAMPLE_RATE, FRAME_DURATION);
        let mut stamper = QpcStamper::new(mix_rate, TARGET_SAMPLE_RATE);
        let mut timeline = LoopbackTimeline::new();
        let mut byte_buf: Vec<u8> = Vec::new();
        let silence_chunk = vec![0.0f32; SILENCE_CHUNK_FRAMES];
        let mut dropped_buffers: u64 = 0;
        let mut padded_frames: u64 = 0;
        // #15: Wanduhrzeit des letzten gelesenen Pakets, nur als Plausibilitaetsgrenze
        // fuer die Stille-Polsterung (die Zeitachse bleibt allein die Geraeteposition).
        let mut last_packet_at = std::time::Instant::now();

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

                // SPEC C1: Die Zeitachse kommt AUSSCHLIESSLICH aus der
                // Device-Position (`BufferInfo::index`, das pu64DevicePosition
                // von IAudioCaptureClient::GetBuffer) — niemals aus gezählten
                // Buffern oder Wall-Clock-Zeit.
                let device_position = info.index;
                // Instant::now() ist ein Zaehlerlesen: keine Allokation, keine Sperre, kein I/O.
                let now = std::time::Instant::now();
                let max_pad = max_plausible_pad_frames(
                    now.duration_since(last_packet_at),
                    mix_rate,
                );
                last_packet_at = now;
                let capped_before = timeline.capped_gaps();
                let pad_frames = match timeline.on_buffer_capped(
                    device_position,
                    frames_read as u64,
                    max_pad,
                ) {
                    TimelineAction::Drop => {
                        dropped_buffers += 1;
                        if dropped_buffers % 100 == 1 {
                            log::warn!(
                                "loopback: backwards device position, dropped {dropped_buffers} buffer(s)"
                            );
                        }
                        continue;
                    }
                    TimelineAction::PadSilence(gap_frames) => gap_frames,
                    TimelineAction::Append => 0,
                };
                if timeline.capped_gaps() != capped_before {
                    let capped = timeline.capped_gaps();
                    if capped % 100 == 1 {
                        log::warn!(
                            "loopback: implausible position jump capped at {pad_frames} frames ({capped} so far, {} frames not padded)",
                            timeline.skipped_frames()
                        );
                    }
                }
                // M2-P2c2: QPC des ersten Frames DIESES Puffers (hinter der
                // Stille); die eingeschobene Stille bekommt davon
                // zurueckgerechnete Stempel. Kein gueltiger Stempel: alter Anker.
                let qpc = (!info.flags.timestamp_error && info.timestamp > 0)
                    .then_some(info.timestamp);
                stamper.mark(qpc, pad_frames);
                if pad_frames > 0 {
                    padded_frames += pad_frames;
                    log::debug!(
                        "loopback: gap of {pad_frames} frames padded with silence (total {padded_frames})"
                    );
                    let mut remaining = pad_frames;
                    while remaining > 0 {
                        let take = remaining.min(SILENCE_CHUNK_FRAMES as u64) as usize;
                        stamper.advance_input(take as u64);
                        resampler.push(&silence_chunk[..take], |frame| {
                            let stamp = stamper.next_block(frame.len());
                            on_samples(&f32_to_i16(frame), stamp)
                        });
                        remaining -= take as u64;
                    }
                }

                let interleaved: Vec<f32> = byte_buf[..valid]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                let mono = downmix_to_mono(&interleaved, channels);
                stamper.advance_input(mono.len() as u64);
                resampler.push(&mono, |frame| {
                    let stamp = stamper.next_block(frame.len());
                    on_samples(&f32_to_i16(frame), stamp)
                });
            }

            // Timeout ist normal (kein Ton) — dann nur das Stop-Flag prüfen.
            let _ = h_event.wait_for_event(EVENT_TIMEOUT_MS);
        }

        resampler.finish(|frame| {
            let stamp = stamper.next_block(frame.len());
            on_samples(&f32_to_i16(frame), stamp)
        });
        audio_client.stop_stream()?;
        log::info!(
            "loopback capture stopped (padded {padded_frames} silence frames, dropped {dropped_buffers} buffers, capped {} implausible gaps = {} frames not padded)",
            timeline.capped_gaps(),
            timeline.skipped_frames()
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
        pub fn start(_on_samples: impl FnMut(&[i16], Option<u64>) + Send + 'static) -> Result<Self> {
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
        assert_eq!(s.next_block(480), Some(1_000_000 + 300_000), "30 ms spaeter");
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
}
