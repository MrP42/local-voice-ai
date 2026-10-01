//! Meeting-Mikrofonaufnahme: eigener cpal-Stream, KEIN VAD (die WAV muss
//! lückenlos sein), 16 kHz mono i16 an den Callback. Bewusst getrennt vom
//! Diktat-`AudioRecorder` (`audio_toolkit/audio/recorder.rs`): der ist
//! M3-stabilisiert und sammelt in RAM — beides wollen wir hier nicht
//! anfassen, dieser Capture-Pfad steht komplett für sich.
//!
//! Der cpal-Audio-Callback tut nur das Nötigste (Samples in einen Channel
//! schieben); Downmix, Resampling auf 16 kHz und die i16-Konvertierung laufen
//! auf einem separaten Konsumenten-Thread, damit kein teurer Schritt im
//! Echtzeit-Audio-Callback die Hardware-Puffer überlaufen lässt (das würde
//! genau die Lücken erzeugen, die die Meeting-WAV nicht haben darf).

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SizedSample};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::audio_toolkit::audio::{
    downmix_to_mono, f32_to_i16, CpalDeviceInfo, FrameResampler, QpcStamper,
};
use crate::audio_toolkit::{get_cpal_host, list_input_devices};

/// Ziel-Samplerate der Meeting-Pipeline (wie die Loopback-Capture, Task 4).
pub const TARGET_SAMPLE_RATE: usize = 16_000;
/// Ausgabeblockdauer: 30 ms bei 16 kHz = 480 Samples.
const FRAME_DURATION: Duration = Duration::from_millis(30);

/// Nachrichten (je ein cpal-Callback, meist ~10 ms) zwischen Callback und
/// Konsument. Begrenzt (#15): blockiert der Konsument je auf Disk-I/O
/// (Virenscanner, USB-Platte), waechst der Rueckstand sonst unbegrenzt im RAM.
/// 1 024 Nachrichten sind rund 10 s Rueckstand, bei 48 kHz Stereo ~4 MB.
pub(crate) const QUEUE_CAPACITY: usize = 1_024;
/// Stille wird in Haeppchen dieser Groesse (Frames beim Geraetetakt) in den
/// Resampler geschoben, wie in der Loopback-Capture.
const SILENCE_CHUNK_FRAMES: usize = 4_096;

pub(crate) enum Msg {
    /// Rohe, interleaved f32-Samples direkt aus dem cpal-Callback (noch nicht
    /// downgemischt oder resampled — das passiert auf dem Konsumenten-Thread),
    /// dazu der QPC-Zeitstempel (100 ns) des ersten Frames, sofern das Gerät
    /// einen liefert (M2-P2c2).
    Samples {
        interleaved: Vec<f32>,
        qpc: Option<u64>,
        /// Frames (Geraetetakt), die davor wegen voller Queue verworfen wurden:
        /// der Konsument fuellt sie mit Stille, damit WAV und Zeitachse ganz bleiben.
        gap_frames: u64,
    },
    /// Sentinel: der Stream wurde gestoppt, keine weiteren `Samples` folgen.
    End,
}

/// Callback-Seite der Queue: `push` wartet nie, sperrt nie und macht kein I/O.
/// Ist die Queue voll, werden die Samples verworfen, GEZAEHLT (`on_overflow`,
/// in 16-kHz-Samples; der Recorder legt sie auf `DspStats::overflow_samples`)
/// und mit der naechsten erfolgreichen Nachricht als Luecke gemeldet.
pub(crate) struct MicQueueFeed {
    tx: SyncSender<Msg>,
    channels: usize,
    sample_rate: usize,
    pending_gap_frames: u64,
    on_overflow: Arc<dyn Fn(u64) + Send + Sync>,
}

impl MicQueueFeed {
    pub(crate) fn new(
        tx: SyncSender<Msg>,
        sample_rate: usize,
        channels: usize,
        on_overflow: Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Self {
        Self {
            tx,
            channels: channels.max(1),
            sample_rate: sample_rate.max(1),
            pending_gap_frames: 0,
            on_overflow,
        }
    }

    /// Kein `send` (das wartete), kein Lock, kein I/O, keine Allokation: `try_send`
    /// auf eine begrenzte Queue. Der `Vec` kommt vom Aufrufer.
    pub(crate) fn push(&mut self, interleaved: Vec<f32>, qpc: Option<u64>) {
        let frames = (interleaved.len() / self.channels) as u64;
        let msg = Msg::Samples {
            interleaved,
            qpc,
            gap_frames: self.pending_gap_frames,
        };
        match self.tx.try_send(msg) {
            Ok(()) => self.pending_gap_frames = 0,
            Err(TrySendError::Full(_)) => {
                // Der Konsument kommt nicht hinterher (Disk-I/O): verwerfen,
                // zaehlen (in 16-kHz-Samples, wie `DspStats::overflow_samples`) und
                // die Luecke mit der naechsten Nachricht melden.
                self.pending_gap_frames += frames;
                (self.on_overflow)(frames * TARGET_SAMPLE_RATE as u64 / self.sample_rate as u64);
            }
            // Der Konsument ist weg: der Stream wird gerade abgebaut.
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

/// Konsumenten-Schleife: Downmix, Stille fuer verworfene Luecken, Resampling
/// auf 16 kHz und die i16-Konvertierung; jeder fertige Block geht an
/// `on_samples`. Endet bei `Msg::End` oder wenn alle Sender weg sind.
pub(crate) fn run_consumer(
    rx: Receiver<Msg>,
    sample_rate: usize,
    channels: usize,
    mut on_samples: impl FnMut(&[i16], Option<u64>),
) {
    let mut resampler = FrameResampler::new(sample_rate, TARGET_SAMPLE_RATE, FRAME_DURATION);
    let mut stamper = QpcStamper::new(sample_rate, TARGET_SAMPLE_RATE);
    let silence = vec![0.0f32; SILENCE_CHUNK_FRAMES];
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Samples {
                interleaved,
                qpc,
                gap_frames,
            } => {
                let mono = downmix_to_mono(&interleaved, channels);
                if gap_frames > 0 {
                    // Gemeldet auf dem Konsumenten-Thread (nie im Audio-Callback);
                    // gezaehlt wurde schon im Callback (`DspStats::overflow_samples`).
                    log::warn!(
                        "meetings: mic capture queue overflowed - {} ms replaced by silence (the consumer is not keeping up, e.g. slow disk)",
                        gap_frames * 1_000 / sample_rate.max(1) as u64
                    );
                }
                // Stempel des ersten echten Frames; die davor verworfene Luecke
                // bekommt zurueckgerechnete Stempel (wie die Stille im Loopback).
                stamper.mark(qpc, gap_frames);
                let mut remaining = gap_frames;
                while remaining > 0 {
                    let take = remaining.min(SILENCE_CHUNK_FRAMES as u64) as usize;
                    stamper.advance_input(take as u64);
                    resampler.push(&silence[..take], |frame| {
                        let stamp = stamper.next_block(frame.len());
                        on_samples(&f32_to_i16(frame), stamp)
                    });
                    remaining -= take as u64;
                }
                stamper.advance_input(mono.len() as u64);
                resampler.push(&mono, |frame| {
                    let stamp = stamper.next_block(frame.len());
                    on_samples(&f32_to_i16(frame), stamp)
                });
            }
            Msg::End => break,
        }
    }
    resampler.finish(|frame| {
        let stamp = stamper.next_block(frame.len());
        on_samples(&f32_to_i16(frame), stamp)
    });
}

/// Eigenständige Mikrofonaufnahme für Meetings. Kein VAD, kein
/// RAM-Gesamtpuffer — jeder resamplete Block geht sofort per Callback an den
/// Aufrufer (der ihn z. B. streamend in eine WAV-Datei schreibt).
///
/// Der `cpal::Stream` lebt NICHT in diesem Struct: auf macOS ist der
/// CoreAudio-Stream `!Send`/`!Sync`, und dieses Struct steckt (über den
/// `MeetingRecorderManager`) im Tauri-State, der `Send + Sync` verlangt.
/// Deshalb besitzt ein eigener Thread den Stream und hält ihn am Leben, bis
/// `stop_tx` signalisiert (oder gedroppt) wird.
pub struct MeetingMicCapture {
    stream_stop_tx: Option<mpsc::Sender<()>>,
    stream_handle: Option<JoinHandle<()>>,
    consumer_handle: Option<JoinHandle<()>>,
    msg_tx: Option<SyncSender<Msg>>,
    error_flag: Arc<AtomicBool>,
}

impl MeetingMicCapture {
    /// Startet die Aufnahme. `device_name` wird wie beim Diktat-Pfad
    /// (`managers/audio.rs:408-447`) per Namensabgleich aufgelöst; findet sich
    /// der Name nicht (oder ist keiner angegeben), fällt es auf das
    /// System-Standardgerät zurück. `on_samples` erhält 16-kHz-Mono-i16-Blöcke
    /// bis `stop()` gerufen wird (läuft auf dem Konsumenten-Thread, nicht im
    /// Audio-Callback). Zweites Argument: QPC-Zeitstempel (100 ns) des ersten
    /// Samples im Block (Windows/WASAPI: `InputCallbackInfo::timestamp().capture`),
    /// `None`, solange das Gerät keinen geliefert hat. Der Stempel des ersten
    /// Blocks ist `mic_qpc0`, der Nullpunkt von `mic.wav`.
    ///
    /// #15: die Queue zwischen Callback und Konsument ist begrenzt
    /// (`QUEUE_CAPACITY`). Ist sie voll, werden die Samples verworfen, mit
    /// `on_overflow` (16-kHz-Samples, lock-frei aufrufbar) gezaehlt und im
    /// Konsumenten als Stille ersetzt: Datei und Zeitachse bleiben ganz.
    pub fn start(
        device_name: Option<String>,
        on_overflow: Arc<dyn Fn(u64) + Send + Sync>,
        on_samples: impl FnMut(&[i16], Option<u64>) + Send + 'static,
    ) -> Result<Self> {
        let error_flag = Arc::new(AtomicBool::new(false));
        let (msg_tx, msg_rx) = mpsc::sync_channel::<Msg>(QUEUE_CAPACITY);
        let (stream_stop_tx, stream_stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(usize, usize)>>();

        // Gerät auflösen, Stream bauen und BESITZEN passiert komplett auf
        // diesem Thread (cpal::Stream ist auf macOS !Send). Der Thread parkt
        // dann auf `stream_stop_rx` und droppt den Stream beim Aufwachen —
        // recv() endet sowohl bei send(()) als auch beim Drop des Senders.
        let stream_msg_tx = msg_tx.clone();
        let stream_error_flag = Arc::clone(&error_flag);
        let stream_handle = std::thread::Builder::new()
            .name("meeting-mic-stream".to_string())
            .spawn(move || {
                let setup = (|| -> Result<(cpal::Stream, usize, usize)> {
                    let device = resolve_device(device_name.as_deref())?;
                    let config = device
                        .default_input_config()
                        .map_err(|e| anyhow!("Failed to get default input config: {e}"))?;
                    let sample_rate = config.sample_rate().0 as usize;
                    let channels = config.channels() as usize;
                    let feed = MicQueueFeed::new(stream_msg_tx, sample_rate, channels, on_overflow);
                    let stream = build_stream(&device, &config, feed, stream_error_flag)?;
                    stream
                        .play()
                        .map_err(|e| anyhow!("Failed to start meeting mic stream: {e}"))?;
                    Ok((stream, sample_rate, channels))
                })();
                match setup {
                    Ok((stream, sample_rate, channels)) => {
                        let _ = ready_tx.send(Ok((sample_rate, channels)));
                        let _ = stream_stop_rx.recv();
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| anyhow!("Failed to spawn meeting mic stream thread: {e}"))?;

        let (sample_rate, channels) = match ready_rx.recv() {
            Ok(Ok(rates)) => rates,
            Ok(Err(e)) => {
                let _ = stream_handle.join();
                return Err(e);
            }
            Err(_) => {
                let _ = stream_handle.join();
                return Err(anyhow!("Meeting mic stream thread died during setup"));
            }
        };

        let consumer_handle = std::thread::Builder::new()
            .name("meeting-mic-consumer".to_string())
            .spawn(move || run_consumer(msg_rx, sample_rate, channels, on_samples))
            .map_err(|e| anyhow!("Failed to spawn meeting mic consumer thread: {e}"))?;

        Ok(Self {
            stream_stop_tx: Some(stream_stop_tx),
            stream_handle: Some(stream_handle),
            consumer_handle: Some(consumer_handle),
            msg_tx: Some(msg_tx),
            error_flag,
        })
    }

    /// Stoppt die Aufnahme und wartet, bis der letzte resamplete Block den
    /// Callback erreicht hat.
    pub fn stop(mut self) {
        self.stop_inner();
    }

    /// Liefert `true`, wenn der cpal-Fehler-Callback seit dem Start gefeuert
    /// hat (z. B. Gerät wurde während der Aufnahme entfernt). Der Manager
    /// (Task 8) meldet das als `recording-error`-Event.
    pub fn had_error(&self) -> bool {
        self.error_flag.load(Ordering::Relaxed)
    }

    fn stop_inner(&mut self) {
        // Stream zuerst droppen: cpal stoppt den Audio-Client synchron, damit
        // danach garantiert keine weiteren `Msg::Samples` mehr eintrudeln.
        // Das Droppen macht der Besitzer-Thread; der join() stellt sicher,
        // dass es passiert ist, BEVOR wir `Msg::End` senden.
        self.stream_stop_tx.take();
        if let Some(handle) = self.stream_handle.take() {
            let _ = handle.join();
        }
        if let Some(tx) = self.msg_tx.take() {
            // Blockierend, aber nicht im Audio-Callback: der Stream ist hier schon
            // weg, und der Konsument leert die Queue (ist er tot, endet `send`
            // sofort mit einem Fehler).
            let _ = tx.send(Msg::End);
        }
        if let Some(handle) = self.consumer_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for MeetingMicCapture {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

/// Pure: entscheidet, welchen Gerätenamen wir öffnen sollen. `None` bedeutet
/// "System-Standardgerät verwenden" — sowohl wenn kein Name gewünscht ist als
/// auch wenn der gewünschte Name unter den verfügbaren Geräten nicht auftaucht
/// (z. B. abgestecktes USB-Mikro). Kein Cache nötig: Task 5 läuft einmal pro
/// Meeting-Aufnahme, nicht auf dem Keypress-Pfad wie der Diktat-Recorder.
fn resolve_device_name(requested: Option<&str>, available: &[String]) -> Option<String> {
    let name = requested?.trim();
    if name.is_empty() {
        return None;
    }
    available.iter().find(|n| n.as_str() == name).cloned()
}

fn resolve_device(device_name: Option<&str>) -> Result<cpal::Device> {
    let host = get_cpal_host();

    let devices: Vec<CpalDeviceInfo> = match list_input_devices() {
        Ok(devices) => devices,
        Err(e) => {
            log::warn!("meeting mic: failed to list input devices ({e}), using system default");
            Vec::new()
        }
    };
    let names: Vec<String> = devices.iter().map(|d| d.name.clone()).collect();

    let resolved_name = resolve_device_name(device_name, &names);
    let device = match resolved_name {
        Some(name) => devices
            .into_iter()
            .find(|d| d.name == name)
            .map(|d| d.device),
        None => None,
    };

    device
        .or_else(|| host.default_input_device())
        .ok_or_else(|| anyhow!("No input device available for meeting mic capture"))
}

/// Aufnahmezeitpunkt des ersten Frames in 100-ns-QPC-Einheiten (dieselbe
/// Einheit wie `wasapi::BufferInfo::timestamp` des Loopbacks). cpal rechnet
/// auf WASAPI die `qpc_position` von `GetBuffer` in einen `StreamInstant` um;
/// hier geht es zurück. Nur Arithmetik, keine Allokation (läuft im
/// Audio-Callback). Andere Hosts haben eine andere Zeitbasis; der DSP-Thread
/// erkennt das am unplausiblen Versatz und fällt auf die Ankunftszeit zurück.
fn capture_qpc(info: &cpal::InputCallbackInfo) -> Option<u64> {
    let since_zero = info
        .timestamp()
        .capture
        .duration_since(&cpal::StreamInstant::new(0, 0))?;
    u64::try_from(since_zero.as_nanos() / 100)
        .ok()
        .filter(|q| *q > 0)
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    feed: MicQueueFeed,
    error_flag: Arc<AtomicBool>,
) -> Result<cpal::Stream> {
    match config.sample_format() {
        cpal::SampleFormat::U8 => build_typed_stream::<u8>(device, config, feed, error_flag),
        cpal::SampleFormat::I8 => build_typed_stream::<i8>(device, config, feed, error_flag),
        cpal::SampleFormat::I16 => build_typed_stream::<i16>(device, config, feed, error_flag),
        cpal::SampleFormat::I32 => build_typed_stream::<i32>(device, config, feed, error_flag),
        cpal::SampleFormat::F32 => build_typed_stream::<f32>(device, config, feed, error_flag),
        fmt => Err(anyhow!("Unsupported sample format: {fmt:?}")),
    }
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mut feed: MicQueueFeed,
    error_flag: Arc<AtomicBool>,
) -> Result<cpal::Stream>
where
    T: Sample + SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    let stream_cb = move |data: &[T], info: &cpal::InputCallbackInfo| {
        let interleaved: Vec<f32> = data.iter().map(|&s| s.to_sample::<f32>()).collect();
        // Begrenzte Queue, `try_send`: nie warten (#15). Voll = verwerfen und
        // zaehlen; ein weggefallener Empfaenger beim Stoppen ist unkritisch.
        feed.push(interleaved, capture_qpc(info));
    };
    let err_cb = move |err: cpal::StreamError| {
        log::error!("meeting mic capture stream error: {err}");
        error_flag.store(true, Ordering::Relaxed);
    };

    device
        .build_input_stream(&config.clone().into(), stream_cb, err_cb, None)
        .map_err(|e| anyhow!("Failed to build meeting mic input stream: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_requested_name_means_system_default() {
        assert_eq!(resolve_device_name(None, &names(&["Mic A", "Mic B"])), None);
    }

    #[test]
    fn empty_requested_name_means_system_default() {
        assert_eq!(resolve_device_name(Some(""), &names(&["Mic A"])), None);
    }

    #[test]
    fn whitespace_only_requested_name_means_system_default() {
        assert_eq!(resolve_device_name(Some("   "), &names(&["Mic A"])), None);
    }

    #[test]
    fn matching_name_is_resolved() {
        assert_eq!(
            resolve_device_name(Some("Mic B"), &names(&["Mic A", "Mic B"])),
            Some("Mic B".to_string())
        );
    }

    #[test]
    fn unknown_name_falls_back_to_system_default() {
        assert_eq!(
            resolve_device_name(Some("Unplugged USB Mic"), &names(&["Mic A", "Mic B"])),
            None
        );
    }

    #[test]
    fn requested_name_is_trimmed_before_matching() {
        assert_eq!(
            resolve_device_name(Some("  Mic A  "), &names(&["Mic A"])),
            Some("Mic A".to_string())
        );
    }

    // ---- #15: begrenzte Queue, Verwerfen mit Zaehlen -------------------------------

    fn counter() -> (Arc<AtomicU64>, Arc<dyn Fn(u64) + Send + Sync>) {
        let total = Arc::new(AtomicU64::new(0));
        let sink = Arc::clone(&total);
        (
            total,
            Arc::new(move |n| {
                sink.fetch_add(n, Ordering::Relaxed);
            }),
        )
    }

    /// 10 ms bei 48 kHz mono = 480 Frames.
    fn block(value: f32) -> Vec<f32> {
        vec![value; 480]
    }

    #[test]
    fn a_full_queue_drops_and_counts_and_never_blocks_the_callback() {
        let (tx, _rx) = sync_channel::<Msg>(2); // niemand liest: der Konsument haengt
        let (dropped, on_overflow) = counter();
        let mut feed = MicQueueFeed::new(tx, 48_000, 1, on_overflow);
        // 5 Bloecke in eine Queue fuer 2: ein blockierendes send() haengte hier.
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            for _ in 0..5 {
                feed.push(block(0.1), Some(1));
            }
            done_tx.send(()).unwrap();
        });
        assert!(
            done_rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "push blockiert bei voller Queue"
        );
        worker.join().unwrap();
        // 3 verworfene Bloecke a 480 Frames bei 48 kHz = je 160 Samples bei 16 kHz.
        assert_eq!(dropped.load(Ordering::Relaxed), 3 * 160);
    }

    #[test]
    fn a_dead_consumer_is_not_counted_as_overflow_and_does_not_panic() {
        let (tx, rx) = sync_channel::<Msg>(2);
        drop(rx); // Konsument weg (Stream wird gerade abgebaut)
        let (dropped, on_overflow) = counter();
        let mut feed = MicQueueFeed::new(tx, 48_000, 1, on_overflow);
        feed.push(block(0.1), None);
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn dropped_blocks_come_back_as_silence_so_the_wav_timeline_stays_whole() {
        // 16 kHz rein = raus: der Resampler reicht 480-Sample-Bloecke 1:1 durch.
        let (tx, rx) = sync_channel::<Msg>(1);
        let (_, on_overflow) = counter();
        let mut feed = MicQueueFeed::new(tx, 16_000, 1, on_overflow);
        let (relay_tx, relay_rx) = sync_channel::<Msg>(8);
        feed.push(vec![0.5; 480], Some(10)); // kommt an
        feed.push(vec![0.5; 480], Some(20)); // Queue voll: verworfen (Luecke 480)

        // Der Konsument liest den ersten Block, danach passt der dritte hinein.
        relay_tx.send(rx.recv().unwrap()).unwrap();
        feed.push(vec![0.5; 480], Some(30));
        relay_tx.send(rx.recv().unwrap()).unwrap();
        relay_tx.send(Msg::End).unwrap();

        let mut out: Vec<(usize, i16)> = Vec::new();
        run_consumer(relay_rx, 16_000, 1, |frame, _| {
            out.push((frame.len(), frame[0]));
        });
        let total: usize = out.iter().map(|(n, _)| n).sum();
        assert_eq!(
            total,
            3 * 480,
            "erster + verworfener (als Stille) + dritter Block"
        );
        // Mitte: Stille (0), Raender: Signal.
        assert!(out[0].1 != 0 && out[1].1 == 0 && out[2].1 != 0, "{out:?}");
    }

    #[test]
    fn the_consumer_stops_on_the_end_message_and_when_every_sender_is_gone() {
        let (tx, rx) = sync_channel::<Msg>(4);
        tx.send(Msg::End).unwrap();
        run_consumer(rx, 16_000, 1, |_, _| {});
        let (tx, rx) = sync_channel::<Msg>(4);
        drop(tx);
        run_consumer(rx, 16_000, 1, |_, _| {});
    }

    #[test]
    fn pushing_into_the_queue_allocates_nothing_in_the_callback() {
        use crate::managers::meetings::echo::alloc_probe::count_allocs;
        let (tx, rx) = sync_channel::<Msg>(1);
        let (_, on_overflow) = counter();
        let mut feed = MicQueueFeed::new(tx, 48_000, 1, on_overflow);
        let (first, second) = (block(0.1), block(0.1));
        // Fall "Queue hat Platz" und Fall "Queue voll" (Verwerfen + Zaehlen).
        let ((), allocs, _) = count_allocs(|| {
            feed.push(first, Some(1));
            feed.push(second, Some(2));
        });
        assert_eq!(
            allocs, 0,
            "push darf nicht allozieren (der Vec kommt vom Aufrufer)"
        );
        drop(rx);
    }
}
