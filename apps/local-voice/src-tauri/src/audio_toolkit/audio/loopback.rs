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
/// lva#538: so weit darf die Position eines Pakets hinter dem Ende der Zeitachse liegen (Takt von
/// Gerät und QPC läuft nie ganz gleich), bevor sie als zurückspringend gilt.
pub const STAMP_BACK_TOLERANCE: std::time::Duration = std::time::Duration::from_millis(250);
/// lva#538: ab dieser Lücke zwischen dem Ende der Zeitachse und der Position eines Pakets wird AN
/// DIESER STELLE aufgefüllt. Die Positionen sind Gerätezeit (kein Ankunftsjitter): eine Pause
/// bleibt dort, wo sie war. Darunter bleibt der Rest stehen und zählt beim nächsten Paket mit
/// (Takt-Drift von Gerät und QPC), der Fehler ist höchstens diese Schwelle.
pub const STAMP_GAP_MIN: std::time::Duration = std::time::Duration::from_millis(10);
/// lva#538: so lange (in Audiozeit) dürfen Positionen in Folge unplausibel sein, bevor die jetzige
/// als neue Referenz gilt (echte Uhrenverstellung, Gerätewechsel). Ein einzelner Ausreisser bleibt
/// so ohne Wirkung, eine bleibende Verschiebung wird übernommen.
pub const STAMP_REANCHOR: std::time::Duration = std::time::Duration::from_secs(1);
/// lva#538: so lang darf EINE Lücke höchstens sein, damit ihre Position glaubwürdig ist. Was
/// darüber liegt (korrupter Stempel, Standby) wird NICHT als Nullen in die WAV geschrieben
/// (Platte: 115 MB je Stunde), sondern wie ein unplausibler Stempel behandelt: das Paket hängt
/// lückenlos an.
pub const MAX_GAP: std::time::Duration = std::time::Duration::from_secs(2 * 3_600);
/// lva#538: so viele Frames (Gerätetakt) verarbeitet ein Schritt von `pump` höchstens, bevor
/// der Capture-Thread wieder das Stop-Flag prüft und das Gerät liest. Auch ein einzelnes
/// grosses Datenpaket wird in Teilen verarbeitet (nie mehr als der Rest des Budgets).
pub const PUMP_STEP_FRAMES: u64 = 16_384;
/// lva#538 (Runde 3): so viel Audio (Gerätetakt) darf in der Arbeitsschlange des Feeders warten.
/// Ist sie voll, liest `read_ready` nicht weiter: das Gerät puffert, der Capture-Thread arbeitet
/// erst ab. Die Schlange ist so nie grösser als die Grenze plus ein Paket (Speicher). Die Grenze
/// gilt nur für Audio: die Stille einer Lücke ist ein Zähler, kein Speicher, und verschiebt keine
/// Position.
pub const QUEUE_LIMIT: std::time::Duration = std::time::Duration::from_secs(8);
/// So viele Pakete holt ein Lesedurchgang höchstens (sonst fände ein Gerät, das nie leer
/// wird, nie den Weg zum Stop-Flag).
const MAX_PACKETS_PER_READ: usize = 256;

/// Ein Geräte-Paket, wie der Capture-Thread es liest (#537).
#[derive(Clone, Copy, Debug)]
pub struct DevicePacket<'a> {
    /// Interleaved f32 beim Gerätetakt; `SILENT`-Pakete schon als Nullen.
    pub interleaved: &'a [f32],
    /// Position des ersten Frames in Frames seit Stream-Start (`BufferInfo::index`). Nur der
    /// Maßstab, wenn das Gerät keine QPC-Stempel nennt.
    pub device_position: u64,
    /// QPC (100 ns) des ersten Frames; `None`, wenn das Gerät keinen gültigen nennt.
    pub qpc: Option<u64>,
}

/// Ein gelesenes Paket mit eigenem Speicher (der Capture-Thread reicht es weiter).
#[derive(Clone, Debug)]
pub struct OwnedPacket {
    pub interleaved: Vec<f32>,
    pub device_position: u64,
    pub qpc: Option<u64>,
}

impl OwnedPacket {
    pub fn as_packet(&self) -> DevicePacket<'_> {
        DevicePacket {
            interleaved: &self.interleaved,
            device_position: self.device_position,
            qpc: self.qpc,
        }
    }
}

/// Die Quelle der Pakete (das Gerät; im Test eine Nachbildung). `Ok(None)`: gerade ist
/// nichts bereit.
pub trait PacketSource {
    fn next_packet(&mut self) -> anyhow::Result<Option<OwnedPacket>>;
}

impl<F> PacketSource for F
where
    F: FnMut() -> anyhow::Result<Option<OwnedPacket>>,
{
    fn next_packet(&mut self) -> anyhow::Result<Option<OwnedPacket>> {
        self()
    }
}

/// Ein Auftrag der Arbeitsschlange des Feeders: erst `pad_left` Frames Stille, dann die Daten.
struct Job {
    /// Das Paket als Mono beim Gerätetakt.
    mono: Vec<f32>,
    /// So viele Frames des Pakets sind schon im Resampler (Teilverarbeitung: ein grosses Paket
    /// frisst nie mehr als das Budget eines Schritts).
    offset: usize,
    /// Der Stempel des ersten Frames (`None`: keiner, oder unplausibel).
    qpc: Option<u64>,
    /// So viele Frames Stille gehen VOR dem Paket in den Resampler (ganze Lücke): ein Zähler,
    /// kein Speicher.
    pad_total: u64,
    pad_left: u64,
    /// Der Stempler kennt den Anker dieses Pakets (gesetzt mit dem ersten Schritt).
    marked: bool,
}

/// Woran die Position eines Pakets gemessen wird (entschieden mit dem ersten Paket).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    /// QPC-Stempel gegen den Stream-Start: auch die Stille ohne Paket zählt mit.
    Qpc,
    /// Geräteposition (nur, wenn das Gerät gar keine Stempel nennt).
    Position,
}

/// Der plattformunabhängige Kern des Capture-Threads: Pakete rein, zeitachsen-
/// korrekte 16-kHz-Mono-Blöcke samt QPC-Stempel raus. Eine Zustandsmaschine mit fester
/// Schlangengrenze: `push` reiht ein, `pump` arbeitet in begrenzten Schritten ab, `finish`
/// schliesst mit einem Budget ab.
///
/// WASAPI-Loopback liefert bei Stille GAR KEINE Pakete (gemessen auf Windows: nach drei
/// Sekunden ohne Ton null Blöcke, kein Fehler). Die Zeitachse darf deshalb nicht an den Paketen
/// hängen. Sie hat EINEN Maßstab (lva#538, Runde 4):
///
/// - **Position jedes Pakets** = (QPC des Pakets − QPC beim Stream-Start) in Frames. Die
///   Anfangsstille ist damit einfach die Position des ersten Pakets: keine Wanduhr, kein
///   Rückstau-Saldo, keine Ankunftszeit. Wann der Capture-Thread ein Paket LIEST, hat auf keine
///   Position Einfluss; auch eine volle Schlange verschiebt nichts.
/// - **Lücke** = Position des Pakets − Frames, die die Zeitachse bis hierher belegt (Daten und
///   Stille). Ab [`STAMP_GAP_MIN`] wird sie AN DIESER STELLE mit Nullen aufgefüllt, darunter bleibt
///   der Rest stehen und zählt beim nächsten Paket mit (Takt-Drift).
/// - **Unplausible Position** (mehr als [`STAMP_BACK_TOLERANCE`] vor dem Ende der Zeitachse,
///   also nicht monoton, oder weiter voraus als [`MAX_GAP`]), gemessen allein an Gerätestempel
///   gegen Sampleanzahl: der Stempel wird ignoriert, das Paket hängt lückenlos an (kein Sample
///   geht verloren, kein Paket wird verworfen). Dauert sie über [`STAMP_REANCHOR`] an, gilt sie
///   als neue Referenz.
/// - **Stille ist ein Zähler**: eine Lücke steht als ausstehende Nullframes am Auftrag ihres
///   Pakets und wird in Schritten ausgegeben ([`PUMP_STEP_FRAMES`]); die Schlangengrenze
///   ([`QUEUE_LIMIT`]) gilt nur für Audio.
/// - **Abschluss mit Budget**: was `finish` nicht mehr schreiben kann (Audio über dem Budget,
///   oder eine Lücke, die nicht mehr hineinpasst und das Audio dahinter zu früh stehen liesse),
///   wird gezählt und zurückgegeben; der Aufrufer meldet es als unvollständige Aufnahme.
///
/// Ohne QPC-Stempel (Gerät nennt keine) gilt die Geräteposition als Maßstab, mit denselben
/// Regeln.
pub struct LoopbackFeeder {
    rate: usize,
    channels: usize,
    resampler: crate::audio_toolkit::audio::FrameResampler,
    stamper: QpcStamper,
    silence: Vec<f32>,
    /// QPC (100 ns) beim Stream-Start, soweit der Aufrufer ihn kennt.
    origin_qpc: Option<u64>,
    /// Maßstab, entschieden mit dem ersten Paket.
    axis: Option<Axis>,
    /// Frames (Gerätetakt) des Stream-Starts auf der Skala des Maßstabs: Position = roh − `base`.
    base: i128,
    /// Frames (Daten und Stille), die die Zeitachse bis hierher belegt.
    end: u64,
    /// Frames in Folge, deren Position unplausibel war.
    bad_stamp_frames: u64,
    queue: std::collections::VecDeque<Job>,
    /// Frames (Daten), die noch in der Schlange warten; die Stille zählt nicht mit.
    queued_frames: u64,
    queue_limit_frames: u64,
    peak_queued_frames: u64,
    /// Obergrenze einer glaubwürdigen Lücke ([`MAX_GAP`]; der Test setzt eine kleinere).
    max_gap: std::time::Duration,
    padded_frames: u64,
    bad_stamps: u64,
    lost_at_stop_frames: u64,
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
impl LoopbackFeeder {
    /// `origin_qpc`: QPC (100 ns) kurz VOR dem Start des Streams; `None`, wenn der Aufrufer ihn
    /// nicht lesen konnte (dann gilt der erste Stempel abzüglich seiner Geräteposition als Start,
    /// ohne erfundene Anfangsstille).
    pub fn new(mix_rate: usize, channels: usize, origin_qpc: Option<u64>) -> Self {
        let rate = mix_rate.max(1);
        Self {
            rate,
            channels: channels.max(1),
            resampler: crate::audio_toolkit::audio::FrameResampler::new(
                mix_rate,
                TARGET_SAMPLE_RATE,
                FRAME_DURATION,
            ),
            stamper: QpcStamper::new(mix_rate, TARGET_SAMPLE_RATE),
            silence: vec![0.0f32; SILENCE_CHUNK_FRAMES],
            origin_qpc: origin_qpc.filter(|q| *q > 0),
            axis: None,
            base: 0,
            end: 0,
            bad_stamp_frames: 0,
            queue: std::collections::VecDeque::new(),
            queued_frames: 0,
            queue_limit_frames: QUEUE_LIMIT.as_secs().saturating_mul(rate as u64),
            peak_queued_frames: 0,
            max_gap: MAX_GAP,
            padded_frames: 0,
            bad_stamps: 0,
            lost_at_stop_frames: 0,
        }
    }

    /// Test: eine kleinere Obergrenze fuer eine glaubwuerdige Luecke.
    #[cfg(test)]
    fn with_max_gap(mut self, max_gap: std::time::Duration) -> Self {
        self.max_gap = max_gap;
        self
    }

    /// Test: eine kleinere Schlangengrenze (in Frames beim Gerätetakt).
    #[cfg(test)]
    fn with_queue_limit(mut self, frames: u64) -> Self {
        self.queue_limit_frames = frames;
        self
    }

    /// Frames (Gerätetakt) einer Dauer.
    fn frames_of(&self, duration: std::time::Duration) -> u64 {
        let frames = duration.as_nanos().saturating_mul(self.rate as u128) / 1_000_000_000;
        u64::try_from(frames).unwrap_or(u64::MAX)
    }

    /// Frames (Gerätetakt) einer QPC-Differenz (negativ: rückwärts).
    fn frames_of_units(&self, units: i128) -> i128 {
        units * self.rate as i128 / QPC_UNITS_PER_SEC
    }

    /// Hat die Schlange noch Platz fuer Audio? (`false`: erst abarbeiten, dann weiterlesen.)
    pub fn has_room(&self) -> bool {
        self.queued_frames < self.queue_limit_frames
    }

    /// So viele Frames darf der naechste Lesedurchgang noch einreihen.
    pub fn room_frames(&self) -> u64 {
        self.queue_limit_frames.saturating_sub(self.queued_frames)
    }

    /// Frames, die gerade auf Verarbeitung warten.
    pub fn queued_frames(&self) -> u64 {
        self.queued_frames
    }

    /// Das Maximum von [`Self::queued_frames`] seit dem Start (fuer Log und Test der Grenze).
    pub fn peak_queued_frames(&self) -> u64 {
        self.peak_queued_frames
    }

    /// Reiht ein Paket ein (nur einreihen, kein Ausgeben: `pump` arbeitet). Die Position und
    /// damit die Stille davor stehen sofort fest, unabhängig davon, wie voll die Schlange ist
    /// oder wann das Paket gelesen wurde.
    pub fn push(&mut self, packet: &DevicePacket<'_>) {
        let frames = (packet.interleaved.len() / self.channels) as u64;
        if frames == 0 {
            return;
        }
        let (pad_frames, stamp) = self.place(packet, frames);
        self.end = self.end.saturating_add(pad_frames).saturating_add(frames);
        self.queued_frames += frames;
        self.peak_queued_frames = self.peak_queued_frames.max(self.queued_frames);
        self.queue.push_back(Job {
            mono: downmix_to_mono(packet.interleaved, self.channels),
            offset: 0,
            qpc: stamp,
            pad_total: pad_frames,
            pad_left: pad_frames,
            marked: false,
        });
    }

    /// Ein Paket einreihen und sofort vollständig abarbeiten (ein Durchgang aus einem Paket).
    /// Für Aufrufer ohne Stop-Flag und für Tests; der Capture-Thread nimmt `run_cycle`.
    pub fn on_packet(
        &mut self,
        packet: DevicePacket<'_>,
        sink: &mut impl FnMut(&[i16], Option<u64>),
    ) {
        self.push(&packet);
        self.pump(u64::MAX, sink);
    }

    /// Die Stille, die vor diesem Paket steht, und der Stempel, der gelten darf (`None`: keiner
    /// oder unplausibel). Die EINE Regel der Zeitachse (siehe [`LoopbackFeeder`]).
    fn place(&mut self, packet: &DevicePacket<'_>, frames: u64) -> (u64, Option<u64>) {
        let stamp = packet.qpc.filter(|q| *q > 0);
        let first = self.axis.is_none();
        if first {
            let axis = if stamp.is_some() {
                Axis::Qpc
            } else {
                Axis::Position
            };
            self.axis = Some(axis);
            self.base = match (axis, stamp, self.origin_qpc) {
                (Axis::Qpc, _, Some(origin)) => self.frames_of_units(origin as i128),
                (Axis::Qpc, Some(q), None) => {
                    self.frames_of_units(q as i128) - packet.device_position as i128
                }
                _ => 0,
            };
        }
        let (raw, stamp) = match self.axis {
            Some(Axis::Qpc) => (stamp.map(|q| self.frames_of_units(q as i128)), stamp),
            _ => (Some(packet.device_position as i128), None),
        };
        // Ein Gerät mit Stempeln, dessen Paket gerade keinen nennt: nichts zu messen, lückenlos.
        let Some(raw) = raw else {
            return (0, None);
        };
        let mut gap = raw - self.base - self.end as i128;
        let back = self.frames_of(STAMP_BACK_TOLERANCE) as i128;
        let limit = self.frames_of(self.max_gap) as i128;
        if first && (gap < 0 || gap > limit) {
            // Der Anfang hat nichts, woran er sich messen liesse, ausser dem Stream-Start: ein
            // Paket davor (Grenzfall des Geräts) ist der Anfang selbst.
            self.base += gap;
            gap = 0;
        }
        if gap < -back || gap > limit {
            self.bad_stamps += 1;
            if self.bad_stamps % 100 == 1 {
                log::warn!(
                    "loopback: implausible packet position ignored ({gap} frames off the time axis, {} so far)",
                    self.bad_stamps
                );
            }
            self.bad_stamp_frames = self.bad_stamp_frames.saturating_add(frames);
            if self.bad_stamp_frames >= self.frames_of(STAMP_REANCHOR) {
                // Bleibt sie, ist es kein Ausreisser mehr: neue Referenz, ohne Stille.
                self.bad_stamp_frames = 0;
                self.base += gap;
                return (0, stamp);
            }
            return (0, None);
        }
        self.bad_stamp_frames = 0;
        let min = self.frames_of(STAMP_GAP_MIN) as i128;
        if gap >= min {
            (gap as u64, stamp)
        } else {
            // Unter der Schwelle: Rest, nicht Verlust (er bleibt in der Differenz).
            (0, stamp)
        }
    }

    /// Arbeitet die Schlange ab, hoechstens `budget` Frames (Gerätetakt) Stille und Daten, auch
    /// von EINEM grossen Paket (der Rest bleibt in der Schlange), und meldet, ob noch Arbeit
    /// bleibt. Aufrufer mit Stop-Flag rufen es in Schritten ([`PUMP_STEP_FRAMES`]) und lesen
    /// dazwischen das Gerät (`run_cycle`).
    pub fn pump(&mut self, budget: u64, sink: &mut impl FnMut(&[i16], Option<u64>)) -> bool {
        let mut left = budget;
        let Self {
            queue,
            queued_frames,
            stamper,
            resampler,
            silence,
            padded_frames,
            ..
        } = self;
        while left > 0 {
            let Some(job) = queue.front_mut() else {
                break;
            };
            if !job.marked {
                // M2-P2c2: QPC des ersten Frames DIESES Pakets (hinter der Stille); die
                // eingeschobene Stille bekommt davon zurückgerechnete Stempel.
                stamper.mark(job.qpc, job.pad_total);
                job.marked = true;
                if job.pad_total > 0 {
                    log::debug!(
                        "loopback: gap of {} frames is padded with silence",
                        job.pad_total
                    );
                }
            }
            if job.pad_left > 0 {
                let take = job.pad_left.min(SILENCE_CHUNK_FRAMES as u64).min(left) as usize;
                stamper.advance_input(take as u64);
                resampler.push(&silence[..take], |frame| {
                    let stamp = stamper.next_block(frame.len());
                    sink(&f32_to_i16(frame), stamp)
                });
                job.pad_left -= take as u64;
                *padded_frames += take as u64;
                left -= take as u64;
                continue;
            }
            let rest = job.mono.len() - job.offset;
            let take = (rest as u64).min(left) as usize;
            stamper.advance_input(take as u64);
            resampler.push(&job.mono[job.offset..job.offset + take], |frame| {
                let stamp = stamper.next_block(frame.len());
                sink(&f32_to_i16(frame), stamp)
            });
            job.offset += take;
            *queued_frames = queued_frames.saturating_sub(take as u64);
            left -= take as u64;
            if job.offset >= job.mono.len() {
                queue.pop_front();
            }
        }
        !queue.is_empty()
    }

    /// Gibt es noch Arbeit in der Schlange?
    pub fn has_work(&self) -> bool {
        !self.queue.is_empty()
    }

    /// So viel Arbeit (Frames) darf der Abschluss hoechstens leisten: die Schlange ist nie
    /// groesser als ihre Grenze plus ein Paket, also verliert er im Normalfall nichts.
    pub fn finish_budget(&self) -> u64 {
        self.queue_limit_frames.saturating_mul(2)
    }

    /// Ende des Streams, mit Budget: die Daten in der Schlange gehen noch hinaus, und eine
    /// Lücke, die ins Budget passt, wird ganz geschrieben, damit das Audio dahinter an seiner
    /// Stelle steht. Was darüber bleibt, geht NICHT hinaus und wird GEZÄHLT: Audio über dem
    /// Budget, und eine Lücke, die nicht mehr hineinpasst (das Audio dahinter stünde dann zu
    /// früh). Die Summe wird zurückgegeben (`lost_at_stop`): der Aufrufer meldet die Aufnahme
    /// dann als unvollständig, nichts wird still verschluckt. Danach der Rest im Resampler.
    pub fn finish(&mut self, budget: u64, sink: &mut impl FnMut(&[i16], Option<u64>)) -> u64 {
        let mut left = budget;
        let mut dropped_gap = 0u64;
        for job in self.queue.iter_mut() {
            if job.pad_left > left {
                dropped_gap += job.pad_left;
                job.pad_left = 0;
                job.pad_total = 0;
                // Der Stempler setzt seinen Anker mit dem Paket neu (ohne Stille davor).
                job.marked = false;
            } else {
                left -= job.pad_left;
            }
            left = left.saturating_sub((job.mono.len() - job.offset) as u64);
        }
        if dropped_gap > 0 {
            log::warn!(
                "loopback: stopped inside a gap, {dropped_gap} frames of silence not written"
            );
        }
        self.pump(budget, sink);
        let dropped_audio = self.queued_frames;
        if dropped_audio > 0 {
            self.queue.clear();
            self.queued_frames = 0;
            log::warn!(
                "loopback: stop budget exhausted, {dropped_audio} frames of audio not written"
            );
        }
        let lost = dropped_gap.saturating_add(dropped_audio);
        self.lost_at_stop_frames = self.lost_at_stop_frames.saturating_add(lost);
        let Self {
            resampler, stamper, ..
        } = self;
        resampler.finish(|frame| {
            let stamp = stamper.next_block(frame.len());
            sink(&f32_to_i16(frame), stamp)
        });
        lost
    }

    /// Für das Log am Ende: (gepolsterte Frames, ignorierte Positionen, beim Stopp nicht
    /// geschriebene Frames).
    pub fn summary(&self) -> (u64, u64, u64) {
        (
            self.padded_frames,
            self.bad_stamps,
            self.lost_at_stop_frames,
        )
    }

    /// Frames, die der Abschluss wegen seines Budgets nicht mehr schreiben konnte (Audio oder
    /// eine Lücke vor Audio): die Aufnahme ist dann nicht vollständig.
    pub fn lost_at_stop(&self) -> u64 {
        self.lost_at_stop_frames
    }

    /// Wie oft eine Position als unplausibel ignoriert wurde.
    pub fn bad_stamps(&self) -> u64 {
        self.bad_stamps
    }
}

/// Wie ein Lesedurchgang endete.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadEnd {
    /// Das Gerät hatte nichts mehr bereit.
    Drained,
    /// `MAX_PACKETS_PER_READ` erreicht: es kann mehr bereitstehen.
    Cap,
    /// Die Schlange ist voll: erst abarbeiten.
    Full,
}

/// Ein Durchgang des Capture-Threads (lva#538): lesen, was das Gerät bereit hat (solange die
/// Schlange Platz hat, höchstens [`MAX_PACKETS_PER_READ`] Pakete je Lesen), einen begrenzten
/// Schritt der Schlange abarbeiten ([`PUMP_STEP_FRAMES`]), und das Stop-Flag prüfen. Das wiederholt
/// sich, bis nichts mehr zu lesen und die Schlange leer ist, oder bis `stop` gesetzt wurde. Das
/// Stop-Flag wirkt so nach JEDEM Lesen, auch wenn das Gerät nie leer wird, und ein Gerät, das
/// während einer langen Stille weiterliefert, läuft nicht über. Ein Lesefehler kommt NACH dem
/// Einreihen dessen, was schon gelesen war.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub fn run_cycle(
    source: &mut impl PacketSource,
    feeder: &mut LoopbackFeeder,
    stop: &std::sync::atomic::AtomicBool,
    sink: &mut impl FnMut(&[i16], Option<u64>),
) -> anyhow::Result<()> {
    loop {
        let end = read_ready(source, feeder)?;
        feeder.pump(PUMP_STEP_FRAMES, sink);
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        // Nur ein leeres Gerät UND eine leere Schlange beenden den Durchgang: bei `Cap` und
        // `Full` kann das Gerät noch mehr bereithaben, und sein Puffer läuft sonst über.
        if end == ReadEnd::Drained && !feeder.has_work() {
            return Ok(());
        }
    }
}

/// Wie eine Sitzung des Capture-Threads endete: ein Fehler, der sie beendet hat, und was der
/// Abschluss nicht mehr schreiben konnte. Beides kommt erst NACH dem Abschluss.
#[derive(Debug, Default)]
pub struct SessionEnd {
    /// Ein Lesefehler des Geräts (Endpoint entfernt, Treiberfehler).
    pub error: Option<anyhow::Error>,
    /// Frames Audio (oder Lücke vor Audio), die der Abschluss wegen seines Budgets nicht mehr
    /// geschrieben hat: 0 = nichts verloren.
    pub lost_frames: u64,
}

impl SessionEnd {
    /// `true`, wenn die Aufnahme dieses Kanals nicht vollständig ist (der Recorder setzt dann
    /// dauerhaft `audio_incomplete` und lässt `capture_open` stehen).
    pub fn audio_lost(&self) -> bool {
        self.lost_frames > 0
    }
}

/// Die Sitzung des Capture-Threads: Durchgänge, bis `stop` gesetzt ist oder das Gerät einen Fehler
/// meldet; `wait` wartet zwischen den Durchgängen auf das nächste Ereignis. Der Abschluss
/// (`finish`) läuft IMMER, auch nach einem Fehler: was das Gerät schon geliefert hat und in der
/// Schlange wartet, geht noch hinaus. Der Abschluss hat ein Budget (`finish_budget`), der Stopp
/// wartet also nie auf unbegrenzte Arbeit; was er deshalb nicht mehr schreibt, steht in
/// [`SessionEnd::lost_frames`].
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub fn capture_session(
    source: &mut impl PacketSource,
    feeder: &mut LoopbackFeeder,
    stop: &std::sync::atomic::AtomicBool,
    wait: &mut dyn FnMut(),
    sink: &mut impl FnMut(&[i16], Option<u64>),
) -> SessionEnd {
    let mut error = None;
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        if let Err(e) = run_cycle(source, feeder, stop, sink) {
            error = Some(e);
            break;
        }
        wait();
    }
    let budget = feeder.finish_budget();
    let lost_frames = feeder.finish(budget, sink);
    SessionEnd { error, lost_frames }
}

/// Liest, was bereitsteht, als einen Durchgang: solange die Schlange Platz hat, höchstens
/// `MAX_PACKETS_PER_READ` Pakete. Jedes Paket wird sofort eingereiht; ein Lesefehler beendet den
/// Durchgang erst danach (das Gelesene ist echtes Audio).
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn read_ready(
    source: &mut impl PacketSource,
    feeder: &mut LoopbackFeeder,
) -> anyhow::Result<ReadEnd> {
    for _ in 0..MAX_PACKETS_PER_READ {
        if !feeder.has_room() {
            return Ok(ReadEnd::Full);
        }
        match source.next_packet()? {
            Some(packet) => feeder.push(&packet.as_packet()),
            None => return Ok(ReadEnd::Drained),
        }
    }
    Ok(ReadEnd::Cap)
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{capture_session, LoopbackFeeder, OwnedPacket};
    use anyhow::{anyhow, Result};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use wasapi::{initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    /// Wartezeit auf das WASAPI-Event; läuft sie ab, prüfen wir das Stop-Flag.
    const EVENT_TIMEOUT_MS: u32 = 200;

    /// Die QPC-Uhr jetzt, in 100 ns (dieselbe Uhr und Einheit wie `BufferInfo::timestamp`):
    /// der Stream-Start, an dem der Feeder die Position jedes Pakets misst. `None`, wenn
    /// Windows sie nicht liefert (dann gilt der erste Stempel als Start, ohne Anfangsstille).
    fn qpc_now_100ns() -> Option<u64> {
        let mut counter = 0i64;
        let mut frequency = 0i64;
        // SAFETY: beide Zeiger zeigen auf lokale i64, die für den Aufruf gültig sind.
        unsafe {
            QueryPerformanceFrequency(&mut frequency).ok()?;
            QueryPerformanceCounter(&mut counter).ok()?;
        }
        if counter <= 0 || frequency <= 0 {
            return None;
        }
        u64::try_from(counter as i128 * super::QPC_UNITS_PER_SEC / frequency as i128).ok()
    }

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
        /// lva#538 (Runde 4): wird gesetzt, wenn der Abschluss des Capture-Threads bereits
        /// gelesenes Audio nicht mehr schreiben konnte (Budget). Der Recorder macht daraus nach
        /// dem Stopp einen Schreibfehler des Systemkanals: `audio_incomplete` bleibt dauerhaft.
        lost: Arc<AtomicBool>,
        handle: Option<JoinHandle<()>>,
    }

    impl LoopbackCapture {
        pub fn start(on_samples: impl FnMut(&[i16], Option<u64>) + Send + 'static) -> Result<Self> {
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let error = Arc::new(AtomicBool::new(false));
            let thread_error = Arc::clone(&error);
            let lost = Arc::new(AtomicBool::new(false));
            let thread_lost = Arc::clone(&lost);
            // Der COM-Init und das Öffnen des Endpoints müssen IM Thread passieren
            // (MTA gilt pro Thread); das Ergebnis kommt über diesen Kanal zurück,
            // damit start() echte Fehler melden kann statt still zu scheitern.
            let (init_tx, init_rx) = mpsc::channel::<Result<(), String>>();

            let handle = std::thread::Builder::new()
                .name("loopback-capture".to_string())
                .spawn(move || {
                    if let Err(e) = capture_loop(thread_stop, thread_lost, on_samples, &init_tx) {
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
                    lost,
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

        /// Beendet die Aufnahme und wartet auf den Abschluss des Capture-Threads. `true`, wenn der
        /// Abschluss bereits gelesenes Audio nicht mehr schreiben konnte: der Aufrufer behandelt
        /// den Systemkanal dann wie einen Schreibfehler (`audio_incomplete`, lva#538 Runde 4).
        pub fn stop(mut self) -> bool {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
            self.lost.load(Ordering::Acquire)
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
        lost: Arc<AtomicBool>,
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
        // lva#538 (Runde 4): der Stream-Start auf der QPC-Uhr, VOR dem Start gelesen (kein Paket
        // kann davor beginnen): die Position jedes Pakets ist sein Stempel minus dieser Wert.
        let origin_qpc = qpc_now_100ns();
        audio_client.start_stream()?;

        // Ab hier steht der Stream — Startmeldung an start().
        let _ = init_tx.send(Ok(()));

        // #537: Stille VOR dem ersten Paket und Pausen ohne Paket liest der Feeder aus den
        // Stempeln gegen den Stream-Start (WASAPI-Loopback liefert bei Stille gar keine Pakete).
        let mut feeder = LoopbackFeeder::new(mix_rate, channels, origin_qpc);
        let mut byte_buf: Vec<u8> = Vec::new();

        // Das Lesen EINES Pakets (ohne das Weiterreichen): `run_cycle` ruft es, bis nichts mehr
        // bereitsteht, und zwischen den begrenzten Arbeitsschritten des Feeders erneut.
        let mut read_packet = || -> Result<Option<OwnedPacket>> {
            let next_frames = capture_client.get_next_packet_size()?.unwrap_or(0) as usize;
            if next_frames == 0 {
                return Ok(None);
            }
            let needed = next_frames * block_align;
            if byte_buf.len() < needed {
                byte_buf.resize(needed, 0);
            }
            let (frames_read, info) = capture_client.read_from_device(&mut byte_buf[..needed])?;
            if frames_read == 0 {
                return Ok(None);
            }
            let valid = frames_read as usize * block_align;
            if info.flags.silent {
                // SILENT: Inhalt ist bedeutungslos -> Nullen. Die Position
                // zählt trotzdem normal weiter, also NICHT überspringen.
                byte_buf[..valid].fill(0);
            }
            // SPEC C1: Die Zeitachse kommt aus den QPC-Stempeln des Geraets gegen den
            // Stream-Start (`LoopbackFeeder`), nie aus gezaehlten Buffern und nie aus der
            // Ankunftszeit des Pakets.
            let interleaved: Vec<f32> = byte_buf[..valid]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            let qpc = (!info.flags.timestamp_error && info.timestamp > 0).then_some(info.timestamp);
            Ok(Some(OwnedPacket {
                interleaved,
                device_position: info.index,
                qpc,
            }))
        };

        // lva#538: Durchgaenge (alles lesen, die Schlange des Feeders in begrenzten Schritten
        // abarbeiten, Stop-Flag und Geraet dazwischen), dann aufs naechste Event warten. Der
        // Abschluss laeuft IMMER, auch nach einem Geraetefehler: was schon gelesen wurde, wird
        // noch geschrieben, erst dann geht der Fehler zurueck.
        // Timeout ist normal (kein Ton) - dann nur das Stop-Flag pruefen.
        let mut wait = || {
            let _ = h_event.wait_for_event(EVENT_TIMEOUT_MS);
        };
        let end = capture_session(
            &mut read_packet,
            &mut feeder,
            &stop,
            &mut wait,
            &mut on_samples,
        );
        let stopped = audio_client.stop_stream();
        let (padded_frames, bad_stamps, lost_frames) = feeder.summary();
        log::info!(
            "loopback capture stopped (padded {padded_frames} silence frames, {bad_stamps} implausible positions ignored, {lost_frames} frames lost at stop, queue peak {} frames)",
            feeder.peak_queued_frames()
        );
        // Verlorenes Audio ist KEIN Absturz des Captures: das Flag geht an den Recorder, der
        // die Aufnahme als unvollstaendig markiert.
        if end.audio_lost() {
            lost.store(true, Ordering::Release);
        }
        if let Some(error) = end.error {
            return Err(error);
        }
        stopped?;
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

        pub fn stop(self) -> bool {
            false
        }

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

    /// Ein Geraetepaket der Nachbildung: Frames, Position, QPC (100 ns) und Pegel (Gleichanteil;
    /// 0.5 = klar hoerbar). Es gibt KEINE Ankunftszeit: die Zeitachse haengt allein an den
    /// Stempeln gegen den Stream-Start.
    struct Pkt {
        frames: usize,
        pos: u64,
        qpc: Option<u64>,
        level: f32,
    }

    /// Schiebt die Pakete durch den Feeder, sammelt Samples und Block-Stempel
    /// (Index des ersten Samples des Blocks, QPC).
    fn feed_into(
        feeder: &mut LoopbackFeeder,
        channels: usize,
        pkts: &[Pkt],
    ) -> (Vec<i16>, Vec<(usize, Option<u64>)>) {
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
                    },
                    &mut sink,
                );
            }
            let budget = feeder.finish_budget();
            feeder.finish(budget, &mut sink);
        }
        (out, stamps)
    }

    fn feed(
        rate: usize,
        channels: usize,
        origin: Option<u64>,
        pkts: &[Pkt],
    ) -> (Vec<i16>, Vec<(usize, Option<u64>)>) {
        feed_into(
            &mut LoopbackFeeder::new(rate, channels, origin),
            channels,
            pkts,
        )
    }

    /// `n` zusammenhaengende 10-ms-Pakete bei 16 kHz (160 Frames), ab Position/QPC.
    fn run_of(n: u64, pos: u64, qpc: Option<u64>) -> Vec<Pkt> {
        (0..n)
            .map(|k| Pkt {
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

    /// Der Stream-Start auf der QPC-Uhr (100 ns).
    const Q0: u64 = 1_000_000_000;
    /// Eine Sekunde in 100 ns.
    const SEC: u64 = 10_000_000;

    /// Der Befund von der echten Probe: kein Paket in der Stille. Das erste Paket
    /// kommt erst nach 10 s; der Ton muss bei 10 s im Transkript stehen, nicht bei 0.
    /// Danach eine Pause von 5 s ohne Paket: danach liegt der Ton wieder 5 s spaeter.
    /// Der Treiber zaehlt die Stille NICHT in der Position mit (Position laeuft nur
    /// ueber gelieferte Pakete), die QPC-Stempel tun es.
    #[test]
    fn silence_before_the_first_packet_and_a_pause_are_padded_with_zeros_by_qpc() {
        let mut pkts = run_of(100, 0, Some(Q0 + 10 * SEC));
        // 1 s Ton, dann 5 s ohne Paket, Position laeuft nicht weiter.
        pkts.extend(run_of(100, 100 * 160, Some(Q0 + 11 * SEC + 5 * SEC)));
        let (out, stamps) = feed(16_000, 1, Some(Q0), &pkts);
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "zwei Toene erwartet: {on:?}");
        // Die Anfangsstille ist die Position des ersten Pakets: genau 10 s.
        assert!(
            on[0].abs_diff(160_000) <= 480,
            "erster Ton bei {} statt ~160 000",
            on[0]
        );
        // 1 s Ton, 5 s Pause: der zweite Ton folgt 80 000 Samples nach dem Ende des ersten.
        assert!(
            on[1].abs_diff(160_000 + 16_000 + 80_000) <= 480,
            "zweiter Ton bei {}",
            on[1]
        );
        // Alles davor sind Nullen, nicht verschluckte Zeit.
        assert!(out[..on[0] - 480].iter().all(|&x| x == 0));
        // Die QPC-Stempel gehen mit: der erste Stille-Block liegt auf dem Stream-Start, der
        // Block kurz vor dem Ton 100 000 x 100 ns davor (160 Frames vor dem Ton).
        assert_eq!(stamps[0], (0, Some(Q0)));
        assert!(stamps.contains(&(159_840, Some(Q0 + 10 * SEC - 100_000))));
    }

    /// Ohne gueltigen QPC misst die Geraeteposition (der Treiber zaehlt die Stille mit): sie
    /// traegt die Anfangsstille und die Pause, und es gibt keine Stempel.
    #[test]
    fn a_device_without_stamps_is_measured_by_its_position() {
        let mut pkts = run_of(100, 160_000, None);
        pkts.extend(run_of(100, 160_000 + 100 * 160 + 80_000, None));
        let (out, stamps) = feed(16_000, 1, Some(Q0), &pkts);
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        assert!(on[0].abs_diff(160_000) <= 480, "{on:?}");
        assert!(on[1].abs_diff(160_000 + 16_000 + 80_000) <= 480, "{on:?}");
        assert!(
            stamps.iter().all(|(_, s)| s.is_none()),
            "ohne QPC kein Stempel"
        );
    }

    /// Ton ab dem ersten Paket, nur eine Pause in der Mitte: der Anfang bleibt bei 0.
    #[test]
    fn a_stream_that_starts_with_sound_has_no_leading_padding() {
        let mut pkts = run_of(50, 0, Some(Q0));
        pkts.extend(run_of(50, 50 * 160, Some(Q0 + 50 * 100_000 + 25_000_000)));
        let (out, _) = feed(16_000, 1, Some(Q0), &pkts);
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        assert!(on[0] <= 480, "Ton ab Anfang, nicht bei {}", on[0]);
        // 0,5 s Ton, 2,5 s Pause.
        assert!(on[1].abs_diff(8_000 + 40_000) <= 480, "{on:?}");
    }

    /// Ein erstes Paket wenige Millisekunden nach dem Start (unter der Schwelle) wird nicht
    /// aufgefuellt; eines VOR dem Start (Grenzfall des Geraets) ist der Anfang selbst, und eine
    /// spaetere Pause wird danach richtig gemessen.
    #[test]
    fn a_first_packet_near_the_start_is_not_padded() {
        let (out, _) = feed(16_000, 1, Some(Q0), &run_of(100, 0, Some(Q0 + 50_000)));
        assert!(onsets(&out)[0] <= 480);
        assert!(out.len().abs_diff(16_000) <= 480, "{} Samples", out.len());

        let before = Q0 - 400_000;
        let mut pkts = run_of(50, 0, Some(before));
        pkts.extend(run_of(
            50,
            50 * 160,
            Some(before + 50 * 100_000 + 3_000_000),
        ));
        let (out, _) = feed(16_000, 1, Some(Q0), &pkts);
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        assert!(on[0] <= 480, "{on:?}");
        // 0,5 s Ton, 0,3 s Pause.
        assert!(on[1].abs_diff(8_000 + 4_800) <= 160, "{on:?}");
    }

    /// Kennt der Aufrufer den Stream-Start nicht, ist das erste Paket der Anfang: keine
    /// erfundene Anfangsstille.
    #[test]
    fn without_an_origin_the_first_packet_is_the_start() {
        let (out, _) = feed(16_000, 1, None, &run_of(100, 0, Some(Q0 + 10 * SEC)));
        assert!(onsets(&out)[0] <= 480);
        assert!(out.len().abs_diff(16_000) <= 480, "{} Samples", out.len());
    }

    /// Das Paket eines Stempel-Geraets ohne Stempel hat nichts zu messen: es haengt lueckenlos
    /// an, nichts geht verloren, die Pause danach steht an ihrer Stelle.
    #[test]
    fn a_missing_stamp_in_a_stamped_stream_appends_without_a_gap() {
        let mut pkts = run_of(10, 0, Some(Q0));
        pkts.extend(run_of(10, 10 * 160, None));
        pkts.extend(run_of(10, 20 * 160, Some(Q0 + 20 * 100_000 + 10_000_000)));
        let (out, _) = feed(16_000, 1, Some(Q0), &pkts);
        assert_eq!(loud(&out), 30 * 160, "alles ist da");
        let on = onsets(&out);
        // Ein Ton (die ersten 200 ms), dann 1 s Pause, dann der letzte Block.
        assert_eq!(on.len(), 2, "{on:?}");
        assert!(on[1].abs_diff(3_200 + 16_000) <= 160, "{on:?}");
    }

    /// Der Weg ueber den Resampler (48 kHz Stereo wie ein echtes Geraet): 3 s Stille am
    /// Anfang, danach 1 s Ton, 2 s Pause, 1 s Ton. Die Offsets stehen beim 16-kHz-Ausgang.
    #[test]
    fn the_padding_survives_resampling_from_48_khz_stereo() {
        let packet = |pos: u64, qpc: u64| Pkt {
            frames: 480,
            pos,
            qpc: Some(qpc),
            level: 0.5,
        };
        // 480 Frames bei 48 kHz = 10 ms = 100 000 (100 ns).
        let first: Vec<Pkt> = (0..100)
            .map(|k| packet(480 * k, Q0 + 3 * SEC + 100_000 * k))
            .collect();
        let second: Vec<Pkt> = (0..100)
            .map(|k| {
                packet(
                    480 * (100 + k),
                    Q0 + 3 * SEC + 100_000 * (100 + k) + 20_000_000,
                )
            })
            .collect();
        let pkts: Vec<Pkt> = first.into_iter().chain(second).collect();
        let (out, _) = feed(48_000, 2, Some(Q0), &pkts);
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        // 3 s Stille, dann der Ton (der Resampler verschmiert um einige Millisekunden).
        assert!(on[0].abs_diff(48_000) <= 1_600, "erster Ton bei {}", on[0]);
        // Ton 1 s, Pause 2 s: der zweite Ton 3 s nach dem Beginn des ersten.
        assert!(
            on[1].abs_diff(on[0] + 16_000 + 32_000) <= 1_600,
            "zweiter Ton bei {} nach {}",
            on[1],
            on[0]
        );
    }

    /// Ein korrupter QPC-Sprung (Stunden) wird nicht stundenlang aufgefuellt: der Stempel gilt
    /// als unplausibel, die Pakete haengen lueckenlos an (kein Sample geht verloren). Bleibt der
    /// Sprung, wird er nach `STAMP_REANCHOR` die neue Referenz, und die Pause danach stimmt wieder.
    #[test]
    fn an_absurd_qpc_gap_is_not_trusted_and_loses_no_audio() {
        let mut pkts = run_of(10, 0, Some(Q0));
        // 10 h voraus, in sich stimmig.
        let far = Q0 + 36_000 * SEC;
        pkts.extend(run_of(300, 10 * 160, Some(far + 10 * 100_000)));
        // Danach eine echte Pause von 1 s, auf der neuen Referenz gemessen.
        pkts.extend(run_of(10, 310 * 160, Some(far + 310 * 100_000 + SEC)));
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let (out, _) = feed_into(&mut feeder, 1, &pkts);
        assert_eq!(loud(&out), 320 * 160, "kein Sample geht verloren");
        // Stunden Nullen sind nicht entstanden: 3,2 s Ton + 1 s Pause.
        assert!(
            out.len().abs_diff(3_200 * 16 + 16_000) <= 480,
            "{} Samples",
            out.len()
        );
        assert_eq!(feeder.bad_stamps(), 100, "eine Sekunde lang ignoriert");
    }

    /// Eine Lücke über `max_gap` ist unglaubwürdig, auch als erster Abstand: der Anfang gilt als
    /// Stream-Start, es entstehen keine Nullen.
    #[test]
    fn a_first_gap_beyond_the_limit_is_not_padded() {
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0))
            .with_max_gap(std::time::Duration::from_secs(10));
        let (out, _) = feed_into(&mut feeder, 1, &run_of(10, 0, Some(Q0 + 3_600 * SEC)));
        assert_eq!(loud(&out), 10 * 160);
        assert!(out.len().abs_diff(1_600) <= 480, "{} Samples", out.len());
        assert_eq!(feeder.summary().0, 0, "nichts aufgefuellt");
    }

    // ---- lva#538: Zeitachse bei kurzen Luecken, QPC-Ruecksprung -----------------------

    /// Ein Paket-Strom aus `blocks` Bloecken zu je `n` 10-ms-Paketen, zwischen den Bloecken eine
    /// Pause von `pause_ms` OHNE Paket. Die Geraeteposition zaehlt nur gelieferte Pakete (wie
    /// WASAPI-Loopback), der QPC-Stempel die Pause mit.
    fn paused_stream(blocks: u64, n: u64, pause_ms: u64) -> Vec<Pkt> {
        let mut pkts = Vec::new();
        let mut qpc = Q0;
        let mut pos = 0u64;
        for _ in 0..blocks {
            for _ in 0..n {
                pkts.push(Pkt {
                    frames: 160,
                    pos,
                    qpc: Some(qpc),
                    level: 0.5,
                });
                qpc += 100_000;
                pos += 160;
            }
            qpc += pause_ms * 10_000;
        }
        pkts
    }

    /// 100 Pausen zu je 200 ms. Jede bleibt AN IHRER STELLE: der k-te Ton beginnt bei
    /// k x 300 ms, nicht nur die Gesamtlaenge stimmt.
    #[test]
    fn repeated_short_pauses_keep_the_time_axis() {
        let (out, _) = feed(16_000, 1, Some(Q0), &paused_stream(100, 10, 200));
        // 100 x 100 ms Ton + 99 x 200 ms Pause = 29,8 s.
        let expected = 29_800 * 16;
        assert!(
            out.len().abs_diff(expected) <= 480,
            "{} Samples statt ~{expected}",
            out.len()
        );
        assert_eq!(loud(&out), 100 * 1_600, "alle Toene sind da");
        let on = onsets(&out);
        assert_eq!(on.len(), 100, "jeder Ton hat seinen Anfang");
        for (k, onset) in on.iter().enumerate() {
            let want = k * 300 * 16;
            assert!(
                onset.abs_diff(want) <= 160,
                "Ton {k} beginnt bei {onset} statt {want}"
            );
        }
    }

    /// Zwei 100-ms-Toene, dazwischen EINE bestaetigte Pause von 200 ms: der zweite Ton beginnt
    /// bei 300 ms (4 800 Samples), nicht bei 100 ms.
    #[test]
    fn a_single_200_ms_pause_keeps_its_position() {
        let (out, _) = feed(16_000, 1, Some(Q0), &paused_stream(2, 10, 200));
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        assert!(on[0] <= 160, "{on:?}");
        assert!(on[1].abs_diff(4_800) <= 160, "zweiter Ton bei {}", on[1]);
        assert!(out.len().abs_diff(6_400) <= 480, "{} Samples", out.len());
    }

    /// Ein Stempel, der gleichmaessig 4 ms neben der Zeitachse liegt (unter `STAMP_GAP_MIN`),
    /// fuellt nichts auf: kein Rauschen der Stempel wird zu Stille.
    #[test]
    fn a_small_constant_stamp_offset_is_not_padded() {
        let mut pkts = run_of(100, 0, Some(Q0));
        pkts.extend(run_of(100, 100 * 160, Some(Q0 + 100 * 100_000 + 40_000)));
        let (out, _) = feed(16_000, 1, Some(Q0), &pkts);
        assert!(
            out.len().abs_diff(200 * 160) <= 480,
            "{} Samples",
            out.len()
        );
    }

    /// Takt-Drift zwischen Geraet und QPC: der Stempel laeuft je Paket 1 ms voraus. Der Rest
    /// sammelt sich unter der Schwelle und wird in Schritten zurueckgezahlt, die Zeitachse
    /// driftet nicht weg (Fehler hoechstens eine Schwelle).
    #[test]
    fn a_drift_between_stamps_and_samples_is_paid_back_in_small_steps() {
        let pkts: Vec<Pkt> = (0..100u64)
            .map(|k| Pkt {
                frames: 160,
                pos: 160 * k,
                qpc: Some(Q0 + 110_000 * k),
                level: 0.5,
            })
            .collect();
        let (out, _) = feed(16_000, 1, Some(Q0), &pkts);
        // 100 x 10 ms Ton + 99 x 1 ms Drift = 1 099 ms.
        let expected = 1_099 * 16;
        assert!(
            out.len().abs_diff(expected) <= (STAMP_GAP_MIN.as_millis() as usize * 16) + 480,
            "{} Samples statt ~{expected}",
            out.len()
        );
    }

    /// Ein Paket mit rueckwaerts springendem QPC (Geraeteposition laeuft durch). Der Stempel wird
    /// nicht uebernommen, das Paket haengt lueckenlos an, und die Rueckkehr zum normalen Wert
    /// erzeugt keine kuenstliche Stille.
    #[test]
    fn a_backwards_qpc_jump_is_ignored_and_its_return_adds_no_silence() {
        let mut pkts = run_of(30, 0, Some(Q0));
        // 5 Pakete mit einem Stempel 10 s in der Vergangenheit.
        pkts.extend(run_of(5, 30 * 160, Some(Q0 - 10 * SEC + 30 * 100_000)));
        // Zurueck zum normalen Wert.
        pkts.extend(run_of(30, 35 * 160, Some(Q0 + 35 * 100_000)));
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let (out, stamps) = feed_into(&mut feeder, 1, &pkts);
        assert!(out.len().abs_diff(65 * 160) <= 480, "{} Samples", out.len());
        assert_eq!(loud(&out), 65 * 160);
        // Kein Stempel stammt aus der Vergangenheit.
        assert!(
            stamps
                .iter()
                .filter_map(|(_, s)| *s)
                .all(|s| (Q0..=Q0 + 65 * 100_000 + 1_000_000).contains(&s)),
            "ein zurueckgesprungener Stempel wurde uebernommen: {stamps:?}"
        );
        assert_eq!(feeder.bad_stamps(), 5);
    }

    /// Ein Stempel, der NICHT zurueckkehrt (echte Verschiebung der Uhr), gilt nach
    /// `STAMP_REANCHOR` als neue Referenz: ohne Stille, und die Stempel folgen ihm wieder.
    #[test]
    fn a_stamp_that_stays_off_becomes_the_new_reference() {
        let shifted = Q0 - 5 * SEC; // 5 s zurueck, bleibt so
        let mut pkts = run_of(30, 0, Some(Q0));
        pkts.extend(run_of(300, 30 * 160, Some(shifted + 30 * 100_000)));
        let (out, stamps) = feed(16_000, 1, Some(Q0), &pkts);
        assert!(
            out.len().abs_diff(330 * 160) <= 480,
            "{} Samples",
            out.len()
        );
        let last = stamps.iter().rev().find_map(|(_, s)| *s).unwrap();
        let expected = shifted + 330 * 100_000;
        assert!(
            last.abs_diff(expected) <= 1_000_000,
            "der letzte Stempel {last} folgt der neuen Referenz {expected} nicht"
        );
    }

    // ---- lva#538, Runde 4: Befund 1 und 2 (Position unabhaengig von Schlange und Lesen) ----

    /// Eine Nachbildung des Geraets: gibt die Pakete der Reihe nach heraus, jedes erst, wenn
    /// der Feeder schon `release_after` Mal abgefragt wurde (so "trifft" ein Paket ein, WAEHREND
    /// der Feeder eine lange Stille abarbeitet). Ab der `fail_at`-ten Abfrage meldet es einen
    /// Geraetefehler (Endpoint entfernt).
    struct ScriptedSource {
        packets: std::collections::VecDeque<(usize, OwnedPacket)>,
        polls: usize,
        fail_at: Option<usize>,
    }

    impl ScriptedSource {
        /// Alle Pakete sofort bereit.
        fn ready(packets: impl IntoIterator<Item = OwnedPacket>) -> Self {
            Self {
                packets: packets.into_iter().map(|p| (0usize, p)).collect(),
                polls: 0,
                fail_at: None,
            }
        }
    }

    impl PacketSource for ScriptedSource {
        fn next_packet(&mut self) -> anyhow::Result<Option<OwnedPacket>> {
            self.polls += 1;
            if self.fail_at.is_some_and(|at| self.polls >= at) {
                return Err(anyhow::anyhow!("endpoint removed"));
            }
            if let Some((after, _)) = self.packets.front() {
                if self.polls > *after {
                    return Ok(self.packets.pop_front().map(|(_, p)| p));
                }
            }
            Ok(None)
        }
    }

    fn owned(pos: u64, qpc: u64, level: f32) -> OwnedPacket {
        OwnedPacket {
            interleaved: vec![level; 160],
            device_position: pos,
            qpc: Some(qpc),
        }
    }

    fn loud(samples: &[i16]) -> usize {
        samples.iter().filter(|x| x.unsigned_abs() > 8_000).count()
    }

    /// Befund 1: der erste Zugriff kommt nach 12 s, davon 3 s Anfangsstille und 9 s
    /// zusammenhaengender Ton. Nach 8 s eingelesenem Ton ist die Schlange voll: die Ausgabe darf
    /// trotzdem nicht bei 0 beginnen. Der erste Ton steht bei 3 s, alle Nutzsamples sind da.
    #[test]
    fn a_full_queue_never_moves_the_start_of_the_recorded_sound() {
        let mut source = ScriptedSource::ready(
            (0..900u64).map(|k| owned(160 * k, Q0 + 3 * SEC + 100_000 * k, 0.5)),
        );
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let limit = 8 * 16_000u64;
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut out: Vec<i16> = Vec::new();
        run_cycle(
            &mut source,
            &mut feeder,
            &stop,
            &mut |b: &[i16], _: Option<u64>| out.extend_from_slice(b),
        )
        .unwrap();
        assert!(
            feeder.peak_queued_frames() >= limit,
            "Voraussetzung: die Schlange war voll ({} Frames)",
            feeder.peak_queued_frames()
        );
        assert!(feeder.peak_queued_frames() <= limit + 160);
        let budget = feeder.finish_budget();
        assert_eq!(
            feeder.finish(budget, &mut |b: &[i16], _: Option<u64>| out
                .extend_from_slice(b)),
            0
        );
        let on = onsets(&out);
        assert_eq!(on.len(), 1, "{on:?}");
        assert!(
            on[0].abs_diff(3 * 16_000) <= 480,
            "erster Ton bei {} statt ~48 000",
            on[0]
        );
        assert_eq!(loud(&out), 9 * 16_000, "alle Nutzsamples erhalten");
        assert!(
            out.len().abs_diff(12 * 16_000) <= 480,
            "{} Samples",
            out.len()
        );
    }

    /// Befund 2: zwei Tonbloecke in EINEM Lesedurchgang (der Rueckstau wird gemeinsam abgeholt),
    /// 3,1 s QPC-Abstand. Die Pause bleibt zwischen den Toenen: Tonanfaenge bei 0 und 3,1 s. Die
    /// Ankunft der Pakete spielt keine Rolle (sie liegen "gleichauf").
    #[test]
    fn a_long_pause_inside_one_burst_keeps_its_position() {
        let first = (0..10u64).map(|k| owned(160 * k, Q0 + 100_000 * k, 0.5));
        let second =
            (0..10u64).map(|k| owned(1_600 + 160 * k, Q0 + 31 * SEC / 10 + 100_000 * k, 0.5));
        let mut source = ScriptedSource::ready(first.chain(second));
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut out: Vec<i16> = Vec::new();
        run_cycle(
            &mut source,
            &mut feeder,
            &stop,
            &mut |b: &[i16], _: Option<u64>| out.extend_from_slice(b),
        )
        .unwrap();
        let budget = feeder.finish_budget();
        feeder.finish(budget, &mut |b: &[i16], _: Option<u64>| {
            out.extend_from_slice(b)
        });
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        assert!(on[0] <= 160, "{on:?}");
        assert!(on[1].abs_diff(49_600) <= 160, "zweiter Ton bei {}", on[1]);
        assert_eq!(loud(&out), 3_200);
        assert_eq!(feeder.bad_stamps(), 0, "die Pause wurde nicht verworfen");
    }

    /// Dasselbe mit Anfangsstille im selben Durchgang: 1 s davor, 3,1 s Pause dazwischen.
    #[test]
    fn leading_silence_and_a_pause_inside_the_first_burst_both_keep_their_place() {
        let first = (0..10u64).map(|k| owned(160 * k, Q0 + SEC + 100_000 * k, 0.5));
        let second =
            (0..10u64).map(|k| owned(1_600 + 160 * k, Q0 + 41 * SEC / 10 + 100_000 * k, 0.5));
        let mut source = ScriptedSource::ready(first.chain(second));
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut out: Vec<i16> = Vec::new();
        run_cycle(
            &mut source,
            &mut feeder,
            &stop,
            &mut |b: &[i16], _: Option<u64>| out.extend_from_slice(b),
        )
        .unwrap();
        let budget = feeder.finish_budget();
        feeder.finish(budget, &mut |b: &[i16], _: Option<u64>| {
            out.extend_from_slice(b)
        });
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        assert!(on[0].abs_diff(16_000) <= 160, "{on:?}");
        assert!(on[1].abs_diff(16_000 + 49_600) <= 160, "{on:?}");
    }

    // ---- lva#538, Runde 4: Befund 3 (Stop in jeder Leseschleife) ------------------------

    /// Befund 3: ein gueltiges erstes Paket, danach nur noch Pakete mit zurueckgesetzter
    /// Position (die Quelle liefert endlos). Beim 256. Abruf wird gestoppt; ein weiterer Abruf ist
    /// ausdruecklich verboten (die Quelle bricht den Test ab). Es gibt keinen Zeitbeweis: die
    /// Sitzung kehrt zurueck, der Abschluss schreibt alles Gelesene, nichts geht verloren.
    #[test]
    fn reading_stops_at_the_flag_even_when_the_device_keeps_delivering_resets() {
        let stop = std::sync::atomic::AtomicBool::new(false);
        let calls = std::cell::Cell::new(0usize);
        let mut source = || -> anyhow::Result<Option<OwnedPacket>> {
            calls.set(calls.get() + 1);
            assert!(calls.get() <= 256, "ein Abruf nach dem Stopp");
            if calls.get() == 256 {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            // Das erste Paket stimmt; alle weiteren melden Position und Stempel von vorn.
            Ok(Some(if calls.get() == 1 {
                owned(0, Q0, 0.5)
            } else {
                owned(0, Q0 - 5 * SEC, 0.5)
            }))
        };
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let mut out: Vec<i16> = Vec::new();
        let end = capture_session(
            &mut source,
            &mut feeder,
            &stop,
            &mut || {},
            &mut |b: &[i16], _: Option<u64>| out.extend_from_slice(b),
        );
        assert_eq!(calls.get(), 256, "genau ein Lesedurchgang");
        assert!(end.error.is_none());
        assert!(!end.audio_lost());
        assert_eq!(
            loud(&out),
            256 * 160,
            "kein Paket verworfen, kein Sample verloren"
        );
        assert!(
            feeder.bad_stamps() > 0,
            "die Ruecksprung-Stempel wurden ignoriert"
        );
        assert!(!feeder.has_work());
    }

    /// Auch bei einem Gerät, das nie leer wird, ohne ungültige Stempel: das Stop-Flag wirkt nach
    /// einem einzigen Lesedurchgang (Cap), nicht erst, wenn das Gerät leer ist.
    #[test]
    fn the_flag_is_checked_after_every_read_pass_not_only_when_the_device_is_empty() {
        let stop = std::sync::atomic::AtomicBool::new(false);
        let calls = std::cell::Cell::new(0u64);
        let mut source = || -> anyhow::Result<Option<OwnedPacket>> {
            calls.set(calls.get() + 1);
            assert!(calls.get() <= 256, "ein Abruf nach dem Stopp");
            if calls.get() == 256 {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            let k = calls.get() - 1;
            Ok(Some(owned(160 * k, Q0 + 100_000 * k, 0.5)))
        };
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let mut out: Vec<i16> = Vec::new();
        let end = capture_session(
            &mut source,
            &mut feeder,
            &stop,
            &mut || {},
            &mut |b: &[i16], _: Option<u64>| out.extend_from_slice(b),
        );
        assert_eq!(calls.get(), 256);
        assert!(end.error.is_none() && !end.audio_lost());
        assert_eq!(loud(&out), 256 * 160);
    }

    // ---- lva#538, Befund 9: grosse Luecken sind begrenzt, unterbrechbar, ohne Paketverlust ----

    /// Das erste Audio kommt nach 1,5 Stunden. Der Capture-Thread schreibt nicht Stunden
    /// Nullen am Stueck: er arbeitet in Schritten, liest dazwischen das Geraet (das Paket, das
    /// WAEHREND der Stille eintrifft, geht nicht verloren) und beachtet das Stop-Flag nach
    /// jedem Schritt. Beim Stopp mitten in der Luecke bleibt der Rest der Stille ungeschrieben und
    /// wird als Verlust gemeldet (das Audio dahinter stuende sonst an der falschen Stelle).
    #[test]
    fn a_huge_gap_is_worked_in_steps_stoppable_and_loses_no_packet() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let hour_and_half = 5_400 * SEC;
        let mut source = ScriptedSource {
            packets: std::collections::VecDeque::from(vec![
                // Das erste Paket nach 1,5 h ...
                (0, owned(0, Q0 + hour_and_half, 0.5)),
                // ... und eines, das eintrifft, waehrend die Stille laeuft (nach 3 Abfragen).
                (3, owned(160, Q0 + hour_and_half + 100_000, 0.5)),
            ]),
            polls: 0,
            fail_at: None,
        };
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let stop = AtomicBool::new(false);
        let mut samples: Vec<i16> = Vec::new();
        {
            let mut sink = |block: &[i16], _: Option<u64>| {
                samples.extend_from_slice(block);
                // Der Nutzer stoppt, nachdem etwas Stille da ist.
                if samples.len() >= 2 * PUMP_STEP_FRAMES as usize {
                    stop.store(true, Ordering::Relaxed);
                }
            };
            run_cycle(&mut source, &mut feeder, &stop, &mut sink).unwrap();
        }
        assert!(stop.load(Ordering::Relaxed));
        assert!(
            feeder.has_work(),
            "die Stunden Stille wurden nicht abgearbeitet"
        );
        assert!(
            samples.len() < 4 * PUMP_STEP_FRAMES as usize + 480,
            "{} Samples: die Arbeit je Schritt ist nicht begrenzt",
            samples.len()
        );
        assert!(
            source.polls >= 3,
            "das Geraet wurde zwischen den Schritten gelesen"
        );
        // Stopp: beide Pakete gehen noch hinaus, die Stille nicht.
        let mut tail: Vec<i16> = Vec::new();
        let mut sink = |block: &[i16], _: Option<u64>| tail.extend_from_slice(block);
        let budget = feeder.finish_budget();
        let lost = feeder.finish(budget, &mut sink);
        assert!(!feeder.has_work());
        assert_eq!(
            loud(&tail),
            2 * 160,
            "das Paket, das waehrend der Stille eintraf, ging verloren"
        );
        assert!(
            tail.len() < 4 * PUMP_STEP_FRAMES as usize,
            "{} Samples im Rest",
            tail.len()
        );
        let in_hours = 5_400 * 16_000u64;
        assert_eq!(
            lost,
            in_hours - feeder.summary().0,
            "der Rest der Luecke ist gezaehlt und gemeldet"
        );
    }

    // ---- lva#538 Runde 3, Befund D: durchgehende Arbeits- und Speichergrenze ----

    /// Ein Gerät, das nie leer wird: die Schlange bleibt unter ihrer Grenze (plus ein Paket),
    /// egal wie viele volle Lesedurchgaenge kommen, und die Arbeit geht voran. Das Stop-Flag
    /// beendet den Durchgang nach wenigen Schritten.
    #[test]
    fn repeated_full_bursts_respect_the_queue_limit() {
        struct Endless(u64);
        impl PacketSource for Endless {
            fn next_packet(&mut self) -> anyhow::Result<Option<OwnedPacket>> {
                self.0 += 1;
                let k = self.0;
                Ok(Some(owned(160 * k, Q0 + 100_000 * k, 0.5)))
            }
        }
        let limit = 16_000u64;
        let mut source = Endless(0);
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0)).with_queue_limit(limit);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut written = 0usize;
        run_cycle(
            &mut source,
            &mut feeder,
            &stop,
            &mut |b: &[i16], _: Option<u64>| {
                written += b.len();
                if written >= 10 * limit as usize {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            },
        )
        .unwrap();
        assert!(
            feeder.peak_queued_frames() <= limit + 160,
            "Schlange bis {} Frames (Grenze {limit})",
            feeder.peak_queued_frames()
        );
        assert!(written >= 10 * limit as usize, "die Arbeit ging voran");
        // Es wurde nur gelesen, was die Schlange fasste (plus die Pakete, die waehrend der
        // Schritte Platz machten), nie "256 Pakete je Schritt" ohne Rueckblick.
        assert!(
            source.0 <= (written as u64 / 160) + limit / 160 + 2,
            "{} Pakete gelesen, {written} Samples ausgegeben",
            source.0
        );
    }

    /// Ein grosses Datenpaket (1 s) frisst nie mehr als das Budget eines Schritts; der Rest
    /// bleibt in der Schlange, nichts geht verloren.
    #[test]
    fn a_big_packet_is_worked_in_parts_within_the_budget() {
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let data = vec![0.5f32; 16_000];
        feeder.push(&DevicePacket {
            interleaved: &data,
            device_position: 0,
            qpc: Some(Q0),
        });
        let out: std::cell::RefCell<Vec<i16>> = std::cell::RefCell::new(Vec::new());
        let mut sink = |b: &[i16], _: Option<u64>| out.borrow_mut().extend_from_slice(b);
        assert!(feeder.pump(100, &mut sink));
        assert_eq!(
            feeder.queued_frames(),
            15_900,
            "genau das Budget verbraucht"
        );
        assert!(
            out.borrow().len() <= 100,
            "{} Samples fuer 100 Frames Budget",
            out.borrow().len()
        );
        assert!(feeder.pump(1_000, &mut sink));
        assert_eq!(feeder.queued_frames(), 14_900);
        assert!(out.borrow().len() <= 1_100);
        while feeder.pump(PUMP_STEP_FRAMES, &mut sink) {}
        let budget = feeder.finish_budget();
        feeder.finish(budget, &mut sink);
        assert_eq!(loud(&out.borrow()), 16_000, "kein Sample verloren");
    }

    /// Der Abschluss hat ein Budget: was darueber liegt, wird GEZAEHLT und gemeldet, nie
    /// verschluckt. Mit `finish_budget()` geht eine volle Schlange ohne Verlust hinaus.
    #[test]
    fn the_stop_work_is_bounded_by_its_budget_and_counts_what_it_drops() {
        let data = vec![0.5f32; 160];
        // Mit knappem Budget: 800 Frames in der Schlange, 300 Budget.
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        for k in 0..5u64 {
            feeder.push(&DevicePacket {
                interleaved: &data,
                device_position: 160 * k,
                qpc: Some(Q0 + 100_000 * k),
            });
        }
        let mut out: Vec<i16> = Vec::new();
        let lost = feeder.finish(300, &mut |b: &[i16], _: Option<u64>| {
            out.extend_from_slice(b)
        });
        assert_eq!(lost, 500);
        assert_eq!(feeder.lost_at_stop(), 500);
        assert_eq!(loud(&out), 300, "genau das Budget geschrieben");
        assert!(!feeder.has_work());
        // Mit dem vorgesehenen Budget geht auch eine bis zur Grenze gefuellte Schlange hinaus.
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0)).with_queue_limit(1_600);
        let mut source =
            ScriptedSource::ready((0..50u64).map(|k| owned(160 * k, Q0 + 100_000 * k, 0.5)));
        assert_eq!(read_ready(&mut source, &mut feeder).unwrap(), ReadEnd::Full);
        let queued = feeder.queued_frames();
        assert!((1_600..=1_760).contains(&queued), "{queued} Frames");
        let mut out: Vec<i16> = Vec::new();
        let budget = feeder.finish_budget();
        let lost = feeder.finish(budget, &mut |b: &[i16], _: Option<u64>| {
            out.extend_from_slice(b)
        });
        assert_eq!(lost, 0);
        assert_eq!(loud(&out) as u64, queued, "nichts verloren");
    }

    /// Eine Luecke, die ins Budget des Abschlusses passt, wird ganz geschrieben (das Audio dahinter
    /// steht an seiner Stelle); eine, die nicht mehr passt, geht nicht hinaus und wird gezaehlt.
    #[test]
    fn finish_writes_a_gap_that_fits_and_counts_one_that_does_not() {
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let frame = 625u64; // 100 ns je Frame bei 16 kHz
        let data = vec![0.5f32; 160];
        let stamps = [
            0,
            // 1 s Luecke nach dem ersten Paket ...
            (160 + 16_000) * frame,
            // ... und noch einmal 1 s nach dem zweiten.
            (160 + 16_000 + 160 + 16_000) * frame,
        ];
        let mut pos = 0u64;
        for stamp in stamps {
            feeder.push(&DevicePacket {
                interleaved: &data,
                device_position: pos,
                qpc: Some(Q0 + stamp),
            });
            pos += 160;
        }
        let mut out: Vec<i16> = Vec::new();
        // Budget: die erste Luecke (16 000) plus die Daten passen, die zweite nicht mehr.
        let lost = feeder.finish(20_000, &mut |b: &[i16], _: Option<u64>| {
            out.extend_from_slice(b)
        });
        assert_eq!(lost, 16_000, "genau die zweite Luecke");
        assert_eq!(feeder.lost_at_stop(), 16_000);
        assert_eq!(loud(&out), 3 * 160, "alle drei Pakete sind geschrieben");
        let on = onsets(&out);
        assert_eq!(on.len(), 2, "{on:?}");
        assert!(on[1].abs_diff(160 + 16_000) <= 160, "{on:?}");
    }

    // ---- lva#538, Runde 4: Befund 4 (Verlust beim Abschluss ist sichtbar) ----------------

    /// Befund 4: ein uebergrosses Paket (20 s) liegt in der Schlange, der Stopp kommt vor dem
    /// Abarbeiten. Das 16-s-Budget des Abschlusses schreibt nicht alles; der Rest wird nicht still
    /// verschluckt, sondern kommt als Verlust aus `capture_session` heraus.
    #[test]
    fn an_oversize_packet_at_stop_is_reported_as_lost_audio() {
        let big = OwnedPacket {
            interleaved: vec![0.5; 20 * 16_000],
            device_position: 0,
            qpc: Some(Q0),
        };
        let mut source = ScriptedSource::ready([big]);
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut out: Vec<i16> = Vec::new();
        let end = capture_session(
            &mut source,
            &mut feeder,
            &stop,
            &mut || {},
            &mut |b: &[i16], _: Option<u64>| {
                out.extend_from_slice(b);
                // Der Stopp kommt, noch bevor das grosse Paket abgearbeitet ist.
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            },
        );
        assert!(end.error.is_none());
        assert!(end.audio_lost(), "der Verlust muss sichtbar werden");
        // Ein Schritt (16 384) lief, das Abschlussbudget (256 000) schrieb den Rest bis dorthin.
        let expected_lost = 20 * 16_000 - PUMP_STEP_FRAMES - 256_000;
        assert_eq!(end.lost_frames, expected_lost);
        assert_eq!(feeder.lost_at_stop(), expected_lost);
        assert_eq!(loud(&out) as u64, PUMP_STEP_FRAMES + 256_000);
    }

    /// Das Gegenstueck: ein Paket in Groesse der Schlangengrenze (8 s) geht beim Stopp ganz
    /// hinaus. Sonst bewiese der Test oben nichts.
    #[test]
    fn a_packet_within_the_queue_limit_is_never_lost_at_stop() {
        let packet = OwnedPacket {
            interleaved: vec![0.5; 8 * 16_000],
            device_position: 0,
            qpc: Some(Q0),
        };
        let mut source = ScriptedSource::ready([packet]);
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut out: Vec<i16> = Vec::new();
        let end = capture_session(
            &mut source,
            &mut feeder,
            &stop,
            &mut || {},
            &mut |b: &[i16], _: Option<u64>| {
                out.extend_from_slice(b);
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            },
        );
        assert!(!end.audio_lost());
        assert_eq!(loud(&out), 8 * 16_000);
    }

    // ---- lva#538 Runde 3, Befund E: ein Geraetefehler vernichtet nichts Gelesenes ----

    /// Die Quelle liefert markiertes PCM und meldet dann einen Fehler. Der Fehler kommt zurueck,
    /// aber alles, was vorher gelesen war, ist geschrieben.
    #[test]
    fn a_device_error_keeps_the_audio_that_was_already_read() {
        let mut source =
            ScriptedSource::ready((0..5u64).map(|k| owned(160 * k, Q0 + 100_000 * k, 0.5)));
        // 5 Pakete, dann der Fehler (beim Abfragen nach dem fuenften).
        source.fail_at = Some(6);
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut out: Vec<i16> = Vec::new();
        let end = capture_session(
            &mut source,
            &mut feeder,
            &stop,
            &mut || {},
            &mut |b: &[i16], _: Option<u64>| out.extend_from_slice(b),
        );
        assert!(end.error.is_some(), "der Fehler geht zurueck");
        assert!(!end.audio_lost());
        assert_eq!(loud(&out), 5 * 160, "alles Gelesene ist geschrieben");
        assert!(!feeder.has_work());
    }

    /// Der Fehler kommt, WAEHREND eine lange Stille abgearbeitet wird: beide Pakete (das davor
    /// und das danach, schon gelesen, in der Schlange) gehen trotzdem hinaus; die Luecke, die nicht
    /// mehr ins Budget passt, wird gezaehlt und als Verlust gemeldet.
    #[test]
    fn a_device_error_during_pending_padding_keeps_the_data_around_it() {
        let mut source = ScriptedSource {
            packets: std::collections::VecDeque::from(vec![
                (0, owned(0, Q0, 0.5)),
                // Das zweite kommt eine Minute spaeter (Stille davor), im zweiten Durchgang.
                (2, owned(160, Q0 + 60 * SEC, 0.5)),
            ]),
            polls: 0,
            // Fehler, sobald nach dem ersten Schritt der Stille gelesen wird.
            fail_at: Some(5),
        };
        let mut feeder = LoopbackFeeder::new(16_000, 1, Some(Q0));
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut out: Vec<i16> = Vec::new();
        let end = capture_session(
            &mut source,
            &mut feeder,
            &stop,
            &mut || {},
            &mut |b: &[i16], _: Option<u64>| out.extend_from_slice(b),
        );
        assert!(end.error.is_some());
        assert_eq!(loud(&out), 2 * 160, "beide Pakete sind geschrieben");
        // Eine Minute (960 000 Frames) Luecke, abzueglich der 160 Frames des ersten Pakets und
        // des einen Schritts, der noch lief.
        let (padded, _, _) = feeder.summary();
        assert_eq!(padded, PUMP_STEP_FRAMES);
        assert_eq!(end.lost_frames, 960_000 - 160 - PUMP_STEP_FRAMES);
        assert!(end.audio_lost());
    }
}
