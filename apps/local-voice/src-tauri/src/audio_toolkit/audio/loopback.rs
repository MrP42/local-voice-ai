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
/// Schwankungen der Paketankunft (Planung des Threads, Pufferlatenz), kein Schweigen. Eine
/// Lücke unter dieser Schwelle geht NICHT verloren: sie bleibt als Rest auf der Zeitachse
/// stehen und wird mit der nächsten aufgefüllt (siehe `LoopbackFeeder::axis`).
pub const QUIET_GAP_MIN: std::time::Duration = std::time::Duration::from_millis(250);
/// lva#538: so weit darf ein QPC-Stempel hinter der Zeitachse liegen (Gerätetakt gegen QPC),
/// bevor er als ungültig gilt (zurückspringender Stempel).
pub const STAMP_BACK_TOLERANCE: std::time::Duration = std::time::Duration::from_millis(250);
/// lva#538 (Runde 3): ab dieser Lücke zwischen Zeitachse und Stempel eines Pakets wird AN DIESER
/// STELLE aufgefüllt. Die Stempel sind Gerätezeit (kein Ankunftsjitter): eine bestätigte Pause
/// bleibt dort, wo sie war. Darunter bleibt der Rest in der Zeitachse stehen und zählt beim
/// nächsten Paket mit (Takt-Drift von Gerät und QPC), der Fehler ist höchstens diese Schwelle.
pub const STAMP_GAP_MIN: std::time::Duration = std::time::Duration::from_millis(10);
/// lva#538: so lange (in Audiozeit) dürfen Stempel in Folge unplausibel sein, bevor der
/// jetzige als neue Referenz gilt (echte Uhrenverstellung, Takt-Drift). Ein einzelner
/// Ausreisser bleibt so ohne Wirkung, eine bleibende Verschiebung wird übernommen.
pub const STAMP_REANCHOR: std::time::Duration = std::time::Duration::from_secs(1);
/// lva#538: Obergrenze EINER Lücke. Was darüber liegt (Stunden Stille nach dem Start, Standby),
/// wird nicht mehr als Nullen in die WAV geschrieben (Platte: 115 MB je Stunde), sondern
/// gezählt (`summary`); die Zeitachse ist danach um den Rest kürzer.
pub const MAX_GAP: std::time::Duration = std::time::Duration::from_secs(2 * 3_600);
/// lva#538: so viele Frames (Gerätetakt) verarbeitet ein Schritt von `pump` höchstens, bevor
/// der Capture-Thread wieder das Stop-Flag prüft und das Gerät liest. Auch ein einzelnes
/// grosses Datenpaket wird in Teilen verarbeitet (nie mehr als der Rest des Budgets).
pub const PUMP_STEP_FRAMES: u64 = 16_384;
/// lva#538 (Runde 3): so viel Audio (Gerätetakt) darf in der Arbeitsschlange des Feeders warten.
/// Ist sie voll, liest `read_ready` nicht weiter: das Gerät puffert, der Capture-Thread arbeitet
/// erst ab. Die Schlange ist so nie grösser als die Grenze plus ein Paket (Speicher).
pub const QUEUE_LIMIT: std::time::Duration = std::time::Duration::from_secs(8);
/// So viele Pakete holt ein Lesedurchgang höchstens (sonst fände ein Gerät, das nie leer
/// wird, nie den Weg zum Stop-Flag).
const MAX_PACKETS_PER_READ: usize = 256;

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

/// Ein gelesenes Paket mit eigenem Speicher (der Capture-Thread reicht es weiter).
#[derive(Clone, Debug)]
pub struct OwnedPacket {
    pub interleaved: Vec<f32>,
    pub device_position: u64,
    pub qpc: Option<u64>,
    pub since_start: std::time::Duration,
}

impl OwnedPacket {
    pub fn as_packet(&self) -> DevicePacket<'_> {
        DevicePacket {
            interleaved: &self.interleaved,
            device_position: self.device_position,
            qpc: self.qpc,
            since_start: self.since_start,
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
    /// So viele Frames Stille gehen VOR dem Paket in den Resampler (ganze Lücke).
    pad_total: u64,
    pad_left: u64,
    /// Der Stempler kennt den Anker dieses Pakets (gesetzt mit dem ersten Schritt).
    marked: bool,
}

/// Der plattformunabhängige Kern des Capture-Threads: Pakete rein, zeitachsen-
/// korrekte 16-kHz-Mono-Blöcke samt QPC-Stempel raus. Eine Zustandsmaschine mit fester
/// Schlangengrenze: `on_burst` reiht ein, `pump` arbeitet in begrenzten Schritten ab,
/// `finish` schliesst mit einem Budget ab.
///
/// WASAPI-Loopback liefert bei Stille GAR KEINE Pakete (gemessen auf Windows: nach
/// drei Sekunden ohne Ton null Blöcke, kein Fehler). Die Zeitachse darf deshalb
/// nicht an den Paketen hängen: Stille VOR dem ersten Paket und Pausen zwischen zwei
/// Paketen werden mit Nullen aufgefüllt, und zwar nach der größeren von zwei
/// Angaben: Lücke der Geräteposition (wo der Treiber sie mitzählt) und Lücke der
/// QPC-Stempel (wo er es nicht tut). Ohne das verrutschen alle Offsets des Transkripts um
/// die Stille, die nie ankam.
///
/// lva#538, Zeitachse aus Gerätestempeln JE PAKET (nicht aus einem Gesamtsaldo):
/// - **Zeitachse (`axis`)**: die QPC-Zeit des nächsten Frames, fortgeschrieben aus dem, was
///   ausgegeben wurde. Der Stempel jedes Pakets wird an ihr gemessen; ab [`STAMP_GAP_MIN`]
///   wird die Lücke VOR DIESEM Paket aufgefüllt, die Pause bleibt also an ihrer Stelle. Nur ein
///   Rest darunter bleibt in der Zeitachse und zählt beim nächsten Paket mit.
/// - **Ungültige Stempel**: ein Stempel, der mehr als [`STAMP_BACK_TOLERANCE`] hinter der
///   Zeitachse liegt oder weiter voraus als die Wanduhr erlaubt, füllt nichts auf und ersetzt
///   die Zeitachse nicht (ein einzelner Ausreisser verschiebt nichts, seine Rückkehr zum
///   normalen Wert erzeugt keine Stille). Dauert er über [`STAMP_REANCHOR`] an, gilt er als neue
///   Referenz.
/// - **Erster Rückstau**: der Verzug des Capture-Threads beim ersten Lesen ist keine Stille. Der
///   Feeder hält die Ausgabe an, bis der erste Rückstau GANZ eingelesen ist (`end_of_read` mit
///   `drained`): die Anfangsstille ist die Wanduhr dann minus ALLE Frames, die er mitbrachte,
///   nicht die eines Teils. Kam der Rückstau nicht zu Ende (Schlange voll, Fehler), wird keine
///   Anfangsstille erfunden.
/// - **Begrenzte Arbeit und Speicher**: die Schlange fasst höchstens [`QUEUE_LIMIT`] Audio
///   (`has_room`; `read_ready` liest bei voller Schlange nicht weiter), `pump` verarbeitet
///   höchstens sein Budget, auch von EINEM grossen Datenpaket, und eine Lücke ist auf
///   [`MAX_GAP`] begrenzt.
pub struct LoopbackFeeder {
    rate: usize,
    channels: usize,
    resampler: crate::audio_toolkit::audio::FrameResampler,
    stamper: QpcStamper,
    timeline: crate::audio_toolkit::audio::LoopbackTimeline,
    silence: Vec<f32>,
    /// Das erste Paket wurde schon eingereiht.
    started: bool,
    /// Der erste Rückstau ist noch nicht ganz eingelesen: die Ausgabe wartet.
    first_open: bool,
    /// Frames (Daten und Stille dazwischen) des ersten Rückstaus.
    first_frames: u64,
    last_since_start: std::time::Duration,
    /// QPC (100 ns), bei dem der nächste ausgegebene Frame liegt (Fortschreibung).
    axis: Option<i128>,
    /// Frames in Folge, deren Stempel unplausibel waren.
    bad_stamp_frames: u64,
    queue: std::collections::VecDeque<Job>,
    /// Daten-Frames, die noch in der Schlange warten (ohne die Stille).
    queued_frames: u64,
    queue_limit_frames: u64,
    peak_queued_frames: u64,
    /// Obergrenze EINER Luecke ([`MAX_GAP`]; der Test setzt eine kleinere).
    max_gap: std::time::Duration,
    padded_frames: u64,
    dropped_buffers: u64,
    capped_gaps: u64,
    skipped_frames: u64,
    lost_at_stop_frames: u64,
    bad_stamps: u64,
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
impl LoopbackFeeder {
    pub fn new(mix_rate: usize, channels: usize) -> Self {
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
            timeline: crate::audio_toolkit::audio::LoopbackTimeline::new(),
            silence: vec![0.0f32; SILENCE_CHUNK_FRAMES],
            started: false,
            first_open: false,
            first_frames: 0,
            last_since_start: std::time::Duration::ZERO,
            axis: None,
            bad_stamp_frames: 0,
            queue: std::collections::VecDeque::new(),
            queued_frames: 0,
            queue_limit_frames: QUEUE_LIMIT.as_secs().saturating_mul(rate as u64),
            peak_queued_frames: 0,
            max_gap: MAX_GAP,
            padded_frames: 0,
            dropped_buffers: 0,
            capped_gaps: 0,
            skipped_frames: 0,
            lost_at_stop_frames: 0,
            bad_stamps: 0,
        }
    }

    /// Test: eine kleinere Obergrenze fuer eine Luecke.
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

    /// Die Schwelle, ab der die Stille VOR dem ersten Paket aufgefüllt wird (Wanduhr; darunter
    /// ist es Anlauf des Treibers).
    fn min_gap_frames(&self) -> u64 {
        self.frames_of(QUIET_GAP_MIN)
    }

    /// 100-ns-Einheiten der QPC-Uhr, die `frames` Gerätetakt-Frames dauern.
    fn qpc_units(&self, frames: u64) -> i128 {
        frames as i128 * QPC_UNITS_PER_SEC / self.rate as i128
    }

    /// Frames (Gerätetakt) einer QPC-Differenz (negativ: rückwärts).
    fn frames_of_units(&self, units: i128) -> i128 {
        units * self.rate as i128 / QPC_UNITS_PER_SEC
    }

    /// Die Anfangsstille, in Frames: die Wanduhr am Ende des ersten Rückstaus minus alle Frames,
    /// die er brachte (das Gerät lieferte sie, solange der Thread noch nicht las). Unter
    /// [`QUIET_GAP_MIN`] ist es Anlauf, keine Stille.
    fn initial_gap(&self, read_end: std::time::Duration, backlog_frames: u64) -> u64 {
        let gap = self.frames_of(read_end).saturating_sub(backlog_frames);
        if gap >= self.min_gap_frames() {
            gap
        } else {
            0
        }
    }

    /// Hat die Schlange noch Platz fuer Daten? (`false`: erst abarbeiten, dann weiterlesen.)
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

    /// Wartet der erste Rückstau noch auf sein Ende (`end_of_read`)? Dann gibt `pump` nichts aus.
    pub fn first_open(&self) -> bool {
        self.first_open
    }

    /// Reiht einen Lesedurchgang des Geräts ein (nur einreihen, kein Ausgeben: `pump` arbeitet).
    /// Die Anfangsstille entscheidet erst `end_of_read`, wenn der erste Rückstau ganz da ist.
    pub fn on_burst(&mut self, packets: &[DevicePacket<'_>]) {
        for packet in packets {
            self.enqueue(packet);
        }
    }

    /// Das Ende eines Lesedurchgangs: `drained` = das Gerät hatte nichts mehr bereit, `read_end`
    /// die Wanduhr seit dem Stream-Start jetzt. Schliesst den ersten Rückstau und setzt dessen
    /// Anfangsstille VOR das erste Paket (nur wenn er ganz da ist; sonst keine erfundene Stille).
    /// Ohne Pakete im Rückstau (das Gerät schweigt noch) bleibt er offen.
    pub fn end_of_read(&mut self, drained: bool, read_end: std::time::Duration) {
        if !self.first_open {
            return;
        }
        self.first_open = false;
        if !drained {
            return;
        }
        let gap = self.initial_gap(read_end, self.first_frames);
        let gap = self.cap_gap(gap);
        if gap == 0 {
            return;
        }
        if let Some(job) = self.queue.front_mut() {
            job.pad_total += gap;
            job.pad_left += gap;
        }
    }

    /// Ein Paket einreihen und sofort vollständig abarbeiten (ein Durchgang aus einem Paket).
    /// Für Aufrufer ohne Stop-Flag und für Tests; der Capture-Thread nimmt `run_cycle`.
    pub fn on_packet(
        &mut self,
        packet: DevicePacket<'_>,
        sink: &mut impl FnMut(&[i16], Option<u64>),
    ) {
        self.on_burst(&[packet]);
        self.end_of_read(true, packet.since_start);
        self.pump(u64::MAX, sink);
    }

    /// Begrenzt EINE Luecke auf `max_gap`; der Rest wird gezaehlt, nicht geschrieben.
    fn cap_gap(&mut self, pad_frames: u64) -> u64 {
        let cap = self.frames_of(self.max_gap);
        if pad_frames <= cap {
            return pad_frames;
        }
        self.capped_gaps += 1;
        self.skipped_frames = self.skipped_frames.saturating_add(pad_frames - cap);
        log::warn!(
            "loopback: gap of {pad_frames} frames capped at {cap} ({} frames not padded)",
            pad_frames - cap
        );
        cap
    }

    /// Entscheidet für EIN Paket, was davor an Stille steht, und reiht es mit seinem Stempel ein.
    fn enqueue(&mut self, packet: &DevicePacket<'_>) {
        let frames = (packet.interleaved.len() / self.channels) as u64;
        if frames == 0 {
            return;
        }
        let first = !self.started;
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
        let (clock_pad, stamp) = self.clock_pad(packet, frames, max_pad, first);
        let pad_frames = self.cap_gap(position_pad.max(clock_pad));
        if first {
            self.started = true;
            self.first_open = true;
            self.first_frames = 0;
        }
        if self.first_open {
            self.first_frames = self
                .first_frames
                .saturating_add(frames)
                .saturating_add(pad_frames);
        }
        self.advance_axis(stamp, frames, pad_frames);
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

    /// Die Stille, die die Uhr (nicht die Geräteposition) vor diesem Paket sieht, und der
    /// Stempel, der gelten darf (`None`: keiner oder unplausibel).
    fn clock_pad(
        &mut self,
        packet: &DevicePacket<'_>,
        frames: u64,
        max_pad: u64,
        first: bool,
    ) -> (u64, Option<u64>) {
        let stamp = packet.qpc.filter(|q| *q > 0);
        if first {
            // Die Anfangsstille kommt aus dem ersten Rückstau (`end_of_read`), nicht aus dem
            // Stempel (davor gibt es keine Zeitachse, an der er sich messen liesse).
            return (0, stamp);
        }
        let (Some(q), Some(axis)) = (stamp, self.axis) else {
            return (0, stamp);
        };
        let gap_units = q as i128 - axis;
        let gap_frames = self.frames_of_units(gap_units);
        let back = self.frames_of(STAMP_BACK_TOLERANCE) as i128;
        let plausible = gap_frames >= -back && gap_frames <= max_pad as i128;
        if !plausible {
            self.bad_stamps += 1;
            if self.bad_stamps % 100 == 1 {
                log::warn!(
                    "loopback: implausible QPC stamp ignored ({gap_frames} frames off the time axis, {} so far)",
                    self.bad_stamps
                );
            }
            self.bad_stamp_frames = self.bad_stamp_frames.saturating_add(frames);
            if self.bad_stamp_frames >= self.frames_of(STAMP_REANCHOR) {
                // Bleibt er, ist es kein Ausreisser mehr: neue Referenz, ohne Stille.
                self.bad_stamp_frames = 0;
                self.axis = Some(q as i128);
                return (0, stamp);
            }
            return (0, None);
        }
        self.bad_stamp_frames = 0;
        let min = self.frames_of(STAMP_GAP_MIN) as i128;
        if gap_frames >= min {
            (gap_frames as u64, stamp)
        } else {
            // Unter der Schwelle: Rest, nicht Verlust (er bleibt in `axis`).
            (0, stamp)
        }
    }

    /// Schreibt die Zeitachse fort: um die Stille und die Daten dieses Pakets. Hat die Stille die
    /// gemessene Lücke gedeckt (oder gab es keine Zeitachse), setzt der Stempel sie neu; blieb ein
    /// Rest unter der Schwelle, bleibt er stehen.
    fn advance_axis(&mut self, stamp: Option<u64>, frames: u64, pad_frames: u64) {
        let data_units = self.qpc_units(frames);
        let pad_units = self.qpc_units(pad_frames);
        let Some(axis) = self.axis else {
            self.axis = stamp.map(|q| q as i128 + data_units);
            return;
        };
        let Some(q) = stamp else {
            self.axis = Some(axis + pad_units + data_units);
            return;
        };
        let gap_units = q as i128 - axis;
        self.axis = Some(if pad_units >= gap_units {
            q as i128 + data_units
        } else {
            axis + pad_units + data_units
        });
    }

    /// Arbeitet die Schlange ab, hoechstens `budget` Frames (Gerätetakt) Stille und Daten, auch
    /// von EINEM grossen Paket (der Rest bleibt in der Schlange), und meldet, ob noch Arbeit
    /// bleibt. Solange der erste Rückstau offen ist (`first_open`), gibt es nichts aus. Aufrufer
    /// mit Stop-Flag rufen es in Schritten ([`PUMP_STEP_FRAMES`]) und lesen dazwischen das Gerät
    /// (`run_cycle`).
    pub fn pump(&mut self, budget: u64, sink: &mut impl FnMut(&[i16], Option<u64>)) -> bool {
        if self.first_open {
            return !self.queue.is_empty();
        }
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

    /// Ende des Streams, mit Budget: die Daten in der Schlange gehen noch hinaus (Audio geht nie
    /// verloren), hoechstens `budget` Frames; was darueber bleibt, wird GEZAEHLT und gemeldet
    /// (Rueckgabe, `lost_at_stop`), nie verschluckt. Eine angefangene Stille geht nicht mehr
    /// hinaus (gezaehlt; der Stempler setzt seinen Anker auf den Stempel des Pakets zurueck), und
    /// ein noch offener erster Rueckstau bekommt keine erfundene Anfangsstille. Danach der Rest
    /// im Resampler.
    pub fn finish(&mut self, budget: u64, sink: &mut impl FnMut(&[i16], Option<u64>)) -> u64 {
        self.first_open = false;
        let mut dropped = 0u64;
        for job in self.queue.iter_mut() {
            if job.pad_left > 0 {
                dropped += job.pad_left;
                job.pad_left = 0;
                job.pad_total = 0;
                job.marked = false;
            }
        }
        if dropped > 0 {
            self.skipped_frames = self.skipped_frames.saturating_add(dropped);
            log::warn!("loopback: stopped inside a gap, {dropped} frames of silence not written");
        }
        self.pump(budget, sink);
        let lost = self.queued_frames;
        if lost > 0 {
            self.queue.clear();
            self.queued_frames = 0;
            self.lost_at_stop_frames = self.lost_at_stop_frames.saturating_add(lost);
            log::warn!("loopback: stop budget exhausted, {lost} frames of audio not written");
        }
        let Self {
            resampler, stamper, ..
        } = self;
        resampler.finish(|frame| {
            let stamp = stamper.next_block(frame.len());
            sink(&f32_to_i16(frame), stamp)
        });
        lost
    }

    /// Für das Log am Ende: (gepolsterte Frames, verworfene Pakete, gekürzte Lücken,
    /// dabei nicht eingeschobene Frames).
    pub fn summary(&self) -> (u64, u64, u64, u64) {
        (
            self.padded_frames,
            self.dropped_buffers,
            self.timeline.capped_gaps() + self.capped_gaps,
            self.timeline
                .skipped_frames()
                .saturating_add(self.skipped_frames),
        )
    }

    /// Audio-Frames, die der Abschluss wegen seines Budgets nicht mehr schreiben konnte.
    pub fn lost_at_stop(&self) -> u64 {
        self.lost_at_stop_frames
    }

    /// Wie oft ein Stempel als unplausibel ignoriert wurde.
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
/// Schlange Platz hat), den ersten Rückstau GANZ einlesen, dann die Schlange des Feeders in
/// begrenzten Schritten abarbeiten ([`PUMP_STEP_FRAMES`]) und DAZWISCHEN das Gerät erneut lesen:
/// ein Gerät, das während einer langen Stille weiterliefert, läuft nicht über (kein Paket geht
/// verloren), die Schlange wächst nie über ihre Grenze, und das Stop-Flag wirkt nach jedem
/// Schritt, nicht erst nach Stunden Nullen. Endet, wenn die Schlange leer ist oder `stop`
/// gesetzt wurde. Ein Lesefehler kommt NACH dem Einreihen dessen, was schon gelesen war.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub fn run_cycle(
    source: &mut impl PacketSource,
    feeder: &mut LoopbackFeeder,
    stop: &std::sync::atomic::AtomicBool,
    now: &dyn Fn() -> std::time::Duration,
    sink: &mut impl FnMut(&[i16], Option<u64>),
) -> anyhow::Result<()> {
    let mut end = read_ready(source, feeder)?;
    // Der erste Rückstau wird ganz eingelesen (die Anfangsstille hängt an seiner Gesamtgrösse);
    // begrenzt durch die Schlangengrenze, auch bei einem Gerät, das nie leer wird.
    while feeder.first_open() && end == ReadEnd::Cap {
        end = read_ready(source, feeder)?;
    }
    feeder.end_of_read(end == ReadEnd::Drained, now());
    while feeder.has_work() && !stop.load(std::sync::atomic::Ordering::Relaxed) {
        feeder.pump(PUMP_STEP_FRAMES, sink);
        let end = read_ready(source, feeder)?;
        feeder.end_of_read(end == ReadEnd::Drained, now());
    }
    Ok(())
}

/// Die Sitzung des Capture-Threads: Durchgänge, bis `stop` gesetzt ist oder das Gerät einen Fehler
/// meldet; `wait` wartet zwischen den Durchgängen auf das nächste Ereignis. Der Abschluss
/// (`finish`) läuft IMMER, auch nach einem Fehler: was das Gerät schon geliefert hat und in der
/// Schlange wartet, geht noch hinaus, erst dann kommt der Fehler zurück. Der Abschluss hat ein
/// Budget (`finish_budget`), der Stopp wartet also nie auf unbegrenzte Arbeit.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub fn capture_session(
    source: &mut impl PacketSource,
    feeder: &mut LoopbackFeeder,
    stop: &std::sync::atomic::AtomicBool,
    now: &dyn Fn() -> std::time::Duration,
    wait: &mut dyn FnMut(),
    sink: &mut impl FnMut(&[i16], Option<u64>),
) -> anyhow::Result<()> {
    let mut result = Ok(());
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        if let Err(e) = run_cycle(source, feeder, stop, now, sink) {
            result = Err(e);
            break;
        }
        wait();
    }
    let budget = feeder.finish_budget();
    feeder.finish(budget, sink);
    result
}

/// Liest, was bereitsteht, als einen Durchgang: solange die Schlange Platz hat, höchstens
/// `MAX_PACKETS_PER_READ` Pakete. Ein Lesefehler beendet den Durchgang, aber erst nachdem die
/// schon gelesenen Pakete eingereiht sind (sie sind echtes Audio).
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn read_ready(
    source: &mut impl PacketSource,
    feeder: &mut LoopbackFeeder,
) -> anyhow::Result<ReadEnd> {
    let channels = feeder.channels;
    let mut room = feeder.room_frames();
    let mut burst: Vec<OwnedPacket> = Vec::new();
    let mut end = ReadEnd::Cap;
    let mut failure: Option<anyhow::Error> = None;
    while burst.len() < MAX_PACKETS_PER_READ {
        if room == 0 {
            end = ReadEnd::Full;
            break;
        }
        match source.next_packet() {
            Ok(Some(packet)) => {
                room = room.saturating_sub((packet.interleaved.len() / channels) as u64);
                burst.push(packet);
            }
            Ok(None) => {
                end = ReadEnd::Drained;
                break;
            }
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    if !burst.is_empty() {
        let refs: Vec<DevicePacket<'_>> = burst.iter().map(OwnedPacket::as_packet).collect();
        feeder.on_burst(&refs);
    }
    match failure {
        Some(e) => Err(e),
        None => Ok(end),
    }
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
            // SPEC C1: Die Zeitachse kommt aus der Device-Position und den
            // QPC-Stempeln des Geraets (`LoopbackFeeder`), nie aus gezaehlten
            // Buffern; die Wanduhr dient nur der Stille ohne Paket und als
            // Plausibilitaetsgrenze.
            let interleaved: Vec<f32> = byte_buf[..valid]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            let qpc = (!info.flags.timestamp_error && info.timestamp > 0).then_some(info.timestamp);
            Ok(Some(OwnedPacket {
                interleaved,
                device_position: info.index,
                qpc,
                since_start: started_at.elapsed(),
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
        let result = capture_session(
            &mut read_packet,
            &mut feeder,
            &stop,
            &|| started_at.elapsed(),
            &mut wait,
            &mut on_samples,
        );
        let stopped = audio_client.stop_stream();
        let (padded_frames, dropped_buffers, capped_gaps, skipped_frames) = feeder.summary();
        log::info!(
            "loopback capture stopped (padded {padded_frames} silence frames, dropped {dropped_buffers} buffers, capped {capped_gaps} implausible gaps = {skipped_frames} frames not padded, {} audio frames lost at stop, queue peak {} frames)",
            feeder.lost_at_stop(),
            feeder.peak_queued_frames()
        );
        result?;
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
            let budget = feeder.finish_budget();
            feeder.finish(budget, &mut sink);
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

    // ---- lva#538 Befund 8: Zeitachse bei kurzen Luecken, QPC-Ruecksprung, Thread-Verzug ----

    /// Ein Paket-Strom aus `blocks` Bloecken zu je `n` 10-ms-Paketen, zwischen den Bloecken eine
    /// Pause von `pause_ms` OHNE Paket. Die Geraeteposition zaehlt nur gelieferte Pakete (wie
    /// WASAPI-Loopback), der QPC-Stempel und die Wanduhr die Pause mit.
    fn paused_stream(blocks: u64, n: u64, pause_ms: u64) -> Vec<Pkt> {
        let mut pkts = Vec::new();
        let mut wall = 0u64;
        let mut qpc = Q0;
        let mut pos = 0u64;
        for _ in 0..blocks {
            for _ in 0..n {
                pkts.push(Pkt {
                    wall_ms: wall,
                    frames: 160,
                    pos,
                    qpc: Some(qpc),
                    level: 0.5,
                });
                wall += 10;
                qpc += 100_000;
                pos += 160;
            }
            wall += pause_ms;
            qpc += pause_ms * 10_000;
        }
        pkts
    }

    /// Gegenbeispiel 1 (Runde 3): 100 Pausen zu je 200 ms. Jede bleibt AN IHRER STELLE: der
    /// k-te Ton beginnt bei k x 300 ms, nicht nur die Gesamtlaenge stimmt (vorher blieb jede
    /// Pause unter 250 ms als Rest stehen und wurde erst hinter bereits ausgegebenem Ton
    /// eingeschoben).
    #[test]
    fn repeated_short_pauses_keep_the_time_axis() {
        let (out, _) = feed(16_000, 1, &paused_stream(100, 10, 200));
        // 100 x 100 ms Ton + 99 x 200 ms Pause = 29,8 s.
        let expected = 29_800 * 16;
        assert!(
            out.len().abs_diff(expected) <= 480,
            "{} Samples statt ~{expected}",
            out.len()
        );
        let loud_samples = out.iter().filter(|x| x.unsigned_abs() > 8_000).count();
        assert_eq!(loud_samples, 100 * 1_600, "alle Toene sind da");
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

    /// Gegenbeispiel 1b: zwei 100-ms-Toene, dazwischen EINE bestaetigte Pause von 200 ms: der
    /// zweite Ton beginnt bei 300 ms (4 800 Samples), nicht bei 100 ms.
    #[test]
    fn a_single_200_ms_pause_keeps_its_position() {
        let (out, _) = feed(16_000, 1, &paused_stream(2, 10, 200));
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
        let mut pkts = run_of(100, 0, 0, Some(Q0));
        pkts.extend(run_of(
            100,
            1_000,
            100 * 160,
            Some(Q0 + 100 * 100_000 + 40_000),
        ));
        let (out, _) = feed(16_000, 1, &pkts);
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
                wall_ms: 11 * k,
                frames: 160,
                pos: 160 * k,
                qpc: Some(Q0 + 110_000 * k),
                level: 0.5,
            })
            .collect();
        let (out, _) = feed(16_000, 1, &pkts);
        // 100 x 10 ms Ton + 99 x 1 ms Drift = 1 099 ms.
        let expected = 1_099 * 16;
        assert!(
            out.len().abs_diff(expected) <= (STAMP_GAP_MIN.as_millis() as usize * 16) + 480,
            "{} Samples statt ~{expected}",
            out.len()
        );
    }

    /// Gegenbeispiel 2: ein Paket mit rueckwaerts springendem QPC (Geraeteposition laeuft
    /// durch). Der Stempel wird nicht uebernommen, und seine Rueckkehr zum normalen Wert erzeugt
    /// keine kuenstliche Stille.
    #[test]
    fn a_backwards_qpc_jump_is_ignored_and_its_return_adds_no_silence() {
        let mut pkts = run_of(30, 0, 0, Some(Q0));
        // 5 Pakete mit einem Stempel 10 s in der Vergangenheit (Position/Wanduhr laufen durch).
        pkts.extend(run_of(
            5,
            300,
            30 * 160,
            Some(Q0 - 100_000_000 + 30 * 100_000),
        ));
        // Zurueck zum normalen Wert.
        pkts.extend(run_of(30, 350, 35 * 160, Some(Q0 + 35 * 100_000)));
        let (out, stamps) = feed(16_000, 1, &pkts);
        assert!(out.len().abs_diff(65 * 160) <= 480, "{} Samples", out.len());
        // Kein Stempel stammt aus der Vergangenheit.
        assert!(
            stamps
                .iter()
                .filter_map(|(_, s)| *s)
                .all(|s| (Q0..=Q0 + 65 * 100_000 + 1_000_000).contains(&s)),
            "ein zurueckgesprungener Stempel wurde uebernommen: {stamps:?}"
        );
        let mut feeder = LoopbackFeeder::new(16_000, 1);
        let mut sink = |_: &[i16], _: Option<u64>| {};
        for p in &pkts {
            let data = vec![p.level; p.frames];
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
        assert_eq!(feeder.bad_stamps(), 5);
    }

    /// Ein Stempel, der NICHT zurueckkehrt (echte Verschiebung der Uhr), gilt nach
    /// `STAMP_REANCHOR` als neue Referenz: ohne Stille, und die Stempel folgen ihm wieder.
    #[test]
    fn a_stamp_that_stays_off_becomes_the_new_reference() {
        let shifted = Q0 - 50_000_000; // 5 s zurueck, bleibt so
        let mut pkts = run_of(30, 0, 0, Some(Q0));
        pkts.extend(run_of(300, 300, 30 * 160, Some(shifted + 30 * 100_000)));
        let (out, stamps) = feed(16_000, 1, &pkts);
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

    /// Gegenbeispiel 3: der Capture-Thread liest erst nach 600 ms zum ersten Mal und findet
    /// fuenf Pakete (50 ms) bereit. Die Anfangsstille ist 600 ms minus ALLE fuenf, nicht minus
    /// das erste: sonst stuende der Ton 40 ms zu spaet (und das Ende hinter der Wanduhr).
    #[test]
    fn the_first_burst_decides_the_leading_silence_not_its_first_packet() {
        let mut feeder = LoopbackFeeder::new(16_000, 1);
        let data = vec![0.5f32; 160];
        let packets: Vec<DevicePacket<'_>> = (0..5u64)
            .map(|k| DevicePacket {
                interleaved: &data,
                device_position: 160 * k,
                qpc: Some(Q0 + 100_000 * k),
                since_start: std::time::Duration::from_millis(600),
            })
            .collect();
        let mut out: Vec<i16> = Vec::new();
        let mut sink = |block: &[i16], _: Option<u64>| out.extend_from_slice(block);
        feeder.on_burst(&packets);
        // Solange der Rueckstau nicht ganz da ist, gibt der Feeder nichts aus.
        assert!(feeder.pump(u64::MAX, &mut sink));
        assert!(feeder.first_open());
        feeder.end_of_read(true, std::time::Duration::from_millis(600));
        feeder.pump(u64::MAX, &mut sink);
        let budget = feeder.finish_budget();
        feeder.finish(budget, &mut sink);
        let on = onsets(&out);
        assert_eq!(on.len(), 1, "{on:?}");
        // 600 ms - 50 ms = 550 ms = 8 800 Samples Stille.
        assert!(
            on[0].abs_diff(8_800) <= 480,
            "Ton bei {} statt ~8 800",
            on[0]
        );
        // Das Ende liegt bei der Wanduhr (600 ms), nicht dahinter.
        assert!(out.len().abs_diff(9_600) <= 480, "{} Samples", out.len());
    }

    /// Gegenbeispiel 3b (Runde 3): 300 gepufferte 10-ms-Pakete (3 s), der Thread liest zum ersten
    /// Mal nach 3 s. Ein Lesedurchgang holt hoechstens 256 Pakete; der Rest des Rueckstaus
    /// gehoert zum selben ersten Rueckstau. Es entsteht KEINE Anfangsstille (vorher ~440 ms).
    #[test]
    fn a_first_backlog_beyond_one_read_invents_no_silence() {
        let wall = std::time::Duration::from_secs(3);
        let mut source = ScriptedSource {
            packets: (0..300u64)
                .map(|k| (0usize, owned(wall, 160 * k, Q0 + 100_000 * k, 0.5)))
                .collect(),
            polls: 0,
            fail_at: None,
        };
        let mut feeder = LoopbackFeeder::new(16_000, 1);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut out: Vec<i16> = Vec::new();
        run_cycle(
            &mut source,
            &mut feeder,
            &stop,
            &|| wall,
            &mut |b: &[i16], _: Option<u64>| out.extend_from_slice(b),
        )
        .unwrap();
        let budget = feeder.finish_budget();
        feeder.finish(budget, &mut |b: &[i16], _: Option<u64>| {
            out.extend_from_slice(b)
        });
        let on = onsets(&out);
        assert_eq!(on.len(), 1, "{on:?}");
        assert!(on[0] <= 160, "erfundene Anfangsstille: Ton bei {}", on[0]);
        assert!(out.len().abs_diff(48_000) <= 480, "{} Samples", out.len());
        assert_eq!(feeder.summary().0, 0, "nichts aufgefuellt");
    }

    // ---- lva#538 Befund 9: grosse Luecken sind begrenzt, unterbrechbar, ohne Paketverlust ----

    /// Eine Nachbildung des Geraets: gibt die Pakete der Reihe nach heraus, jedes erst, wenn
    /// der Feeder schon `release_after` Mal abgefragt wurde (so "trifft" ein Paket ein, WAEHREND
    /// der Feeder eine lange Stille abarbeitet). Ab der `fail_at`-ten Abfrage meldet es einen
    /// Geraetefehler (Endpoint entfernt).
    struct ScriptedSource {
        packets: std::collections::VecDeque<(usize, OwnedPacket)>,
        polls: usize,
        fail_at: Option<usize>,
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

    fn owned(wall: std::time::Duration, pos: u64, qpc: u64, level: f32) -> OwnedPacket {
        OwnedPacket {
            interleaved: vec![level; 160],
            device_position: pos,
            qpc: Some(qpc),
            since_start: wall,
        }
    }

    fn loud(samples: &[i16]) -> usize {
        samples.iter().filter(|x| x.unsigned_abs() > 8_000).count()
    }

    /// Das erste Audio kommt nach 1,5 Stunden. Der Capture-Thread schreibt nicht Stunden
    /// Nullen am Stueck: er arbeitet in Schritten, liest dazwischen das Geraet (das Paket, das
    /// WAEHREND der Stille eintrifft, geht nicht verloren) und beachtet das Stop-Flag nach
    /// jedem Schritt.
    #[test]
    fn a_huge_gap_is_worked_in_steps_stoppable_and_loses_no_packet() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let wall = std::time::Duration::from_secs(5_400);
        let mut source = ScriptedSource {
            packets: std::collections::VecDeque::from(vec![
                // Das erste Paket nach 1,5 h ...
                (0, owned(wall, 0, Q0, 0.5)),
                // ... und eines, das eintrifft, waehrend die Stille laeuft (nach 3 Abfragen).
                (
                    3,
                    owned(
                        wall + std::time::Duration::from_millis(10),
                        160,
                        Q0 + 100_000,
                        0.5,
                    ),
                ),
            ]),
            polls: 0,
            fail_at: None,
        };
        let mut feeder = LoopbackFeeder::new(16_000, 1);
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
            run_cycle(&mut source, &mut feeder, &stop, &|| wall, &mut sink).unwrap();
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
        // Stopp: das Paket, das waehrend der Stille eintraf, geht noch hinaus, die Stille nicht.
        let mut tail: Vec<i16> = Vec::new();
        let mut sink = |block: &[i16], _: Option<u64>| tail.extend_from_slice(block);
        let budget = feeder.finish_budget();
        assert_eq!(feeder.finish(budget, &mut sink), 0, "nichts verloren");
        assert!(!feeder.has_work());
        assert!(
            tail.iter().any(|&x| x.unsigned_abs() > 8_000),
            "das Paket, das waehrend der Stille eintraf, ging verloren"
        );
        assert!(
            tail.len() < 4 * PUMP_STEP_FRAMES as usize,
            "{} Samples im Rest",
            tail.len()
        );
        let (_, _, capped, skipped) = feeder.summary();
        assert_eq!(capped, 0);
        assert!(skipped > 0, "die nicht geschriebene Stille wird gezaehlt");
    }

    /// Eine Luecke ist auf `max_gap` begrenzt; der Rest wird gezaehlt, nicht geschrieben.
    #[test]
    fn one_gap_is_bounded_and_the_rest_is_counted() {
        use std::time::Duration;
        let mut feeder = LoopbackFeeder::new(16_000, 1).with_max_gap(Duration::from_secs(10));
        let data = vec![0.5f32; 160];
        // Erstes Paket nach 1 h: 3 590 s Stille waeren faellig.
        feeder.on_packet(
            DevicePacket {
                interleaved: &data,
                device_position: 0,
                qpc: Some(Q0),
                since_start: Duration::from_secs(3_600),
            },
            &mut |_: &[i16], _: Option<u64>| {},
        );
        let (padded, _, capped, skipped) = feeder.summary();
        assert_eq!(padded, 10 * 16_000, "hoechstens max_gap geschrieben");
        assert_eq!(capped, 1);
        assert!(
            skipped >= 3_589 * 16_000,
            "{skipped} nicht geschriebene Frames"
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
                Ok(Some(owned(
                    std::time::Duration::from_millis(10 * k),
                    160 * k,
                    Q0 + 100_000 * k,
                    0.5,
                )))
            }
        }
        let limit = 16_000u64;
        let mut source = Endless(0);
        let mut feeder = LoopbackFeeder::new(16_000, 1).with_queue_limit(limit);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut written = 0usize;
        run_cycle(
            &mut source,
            &mut feeder,
            &stop,
            &|| std::time::Duration::from_secs(1),
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
        let mut feeder = LoopbackFeeder::new(16_000, 1);
        let data = vec![0.5f32; 16_000];
        feeder.on_burst(&[DevicePacket {
            interleaved: &data,
            device_position: 0,
            qpc: Some(Q0),
            since_start: std::time::Duration::ZERO,
        }]);
        feeder.end_of_read(true, std::time::Duration::ZERO);
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
        let packets: Vec<DevicePacket<'_>> = (0..5u64)
            .map(|k| DevicePacket {
                interleaved: &data,
                device_position: 160 * k,
                qpc: Some(Q0 + 100_000 * k),
                since_start: std::time::Duration::ZERO,
            })
            .collect();
        // Mit knappem Budget: 800 Frames in der Schlange, 300 Budget.
        let mut feeder = LoopbackFeeder::new(16_000, 1);
        feeder.on_burst(&packets);
        feeder.end_of_read(true, std::time::Duration::ZERO);
        let mut out: Vec<i16> = Vec::new();
        let lost = feeder.finish(300, &mut |b: &[i16], _: Option<u64>| {
            out.extend_from_slice(b)
        });
        assert_eq!(lost, 500);
        assert_eq!(feeder.lost_at_stop(), 500);
        assert_eq!(loud(&out), 300, "genau das Budget geschrieben");
        assert!(!feeder.has_work());
        // Mit dem vorgesehenen Budget geht auch eine bis zur Grenze gefuellte Schlange hinaus.
        let mut feeder = LoopbackFeeder::new(16_000, 1).with_queue_limit(1_600);
        let mut source = ScriptedSource {
            packets: (0..50u64)
                .map(|k| {
                    (
                        0usize,
                        owned(std::time::Duration::ZERO, 160 * k, Q0 + 100_000 * k, 0.5),
                    )
                })
                .collect(),
            polls: 0,
            fail_at: None,
        };
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

    // ---- lva#538 Runde 3, Befund E: ein Geraetefehler vernichtet nichts Gelesenes ----

    /// Die Quelle liefert markiertes PCM und meldet dann einen Fehler. Der Fehler kommt zurueck,
    /// aber alles, was vorher gelesen war, ist geschrieben.
    #[test]
    fn a_device_error_keeps_the_audio_that_was_already_read() {
        let mut source = ScriptedSource {
            packets: (0..5u64)
                .map(|k| {
                    (
                        0usize,
                        owned(
                            std::time::Duration::from_millis(10 * k),
                            160 * k,
                            Q0 + 100_000 * k,
                            0.5,
                        ),
                    )
                })
                .collect(),
            polls: 0,
            // 5 Pakete, dann der Fehler (beim Abfragen nach dem fuenften).
            fail_at: Some(6),
        };
        let mut feeder = LoopbackFeeder::new(16_000, 1);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut out: Vec<i16> = Vec::new();
        let result = capture_session(
            &mut source,
            &mut feeder,
            &stop,
            &|| std::time::Duration::from_millis(50),
            &mut || {},
            &mut |b: &[i16], _: Option<u64>| out.extend_from_slice(b),
        );
        assert!(result.is_err(), "der Fehler geht zurueck");
        assert_eq!(loud(&out), 5 * 160, "alles Gelesene ist geschrieben");
        assert!(!feeder.has_work());
    }

    /// Der Fehler kommt, WAEHREND eine lange Stille abgearbeitet wird: das Paket davor und das
    /// danach (schon gelesen, in der Schlange) gehen trotzdem hinaus; die angefangene Stille
    /// wird gezaehlt, nicht geschrieben.
    #[test]
    fn a_device_error_during_pending_padding_keeps_the_data_around_it() {
        let wall = std::time::Duration::from_millis(10);
        let mut source = ScriptedSource {
            packets: std::collections::VecDeque::from(vec![
                (0, owned(wall, 0, Q0, 0.5)),
                // Das zweite kommt eine Minute spaeter (Stille davor), im zweiten Durchgang.
                (
                    2,
                    owned(
                        wall + std::time::Duration::from_secs(60),
                        160,
                        Q0 + 100_000 + 600_000_000,
                        0.5,
                    ),
                ),
            ]),
            polls: 0,
            // Fehler, sobald nach dem ersten Schritt der Stille gelesen wird.
            fail_at: Some(5),
        };
        let mut feeder = LoopbackFeeder::new(16_000, 1);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut out: Vec<i16> = Vec::new();
        let result = capture_session(
            &mut source,
            &mut feeder,
            &stop,
            &|| wall,
            &mut || {},
            &mut |b: &[i16], _: Option<u64>| out.extend_from_slice(b),
        );
        assert!(result.is_err());
        assert_eq!(loud(&out), 2 * 160, "beide Pakete sind geschrieben");
        let (padded, _, _, skipped) = feeder.summary();
        assert!(padded > 0, "ein Teil der Stille lief noch");
        assert!(skipped > 0, "der Rest der Stille ist gezaehlt");
    }
}
