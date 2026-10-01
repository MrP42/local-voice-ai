//! M2 / P2e: Ausfallwaechter je Kanal (Granola-Schwaeche "stille Aufnahmeausfaelle").
//!
//! Reines Zustandsmodul ohne Uhr, Thread und I/O: der DSP-Thread (`dsp.rs`)
//! fuettert es mit den Bloecken, die er ohnehin bekommt, und mit seinem
//! Wanduhr-Tick. Ausgewertet wird also nie im Capture-Callback (Echtzeitregel,
//! `mic_capture.rs`). Jede Methode liefert hoechstens einen Zustandswechsel;
//! solange sich nichts aendert, kommt nichts zurueck (keine Event-Flut).
//!
//! Bedingungen (je Kanal steht hoechstens EINE, in dieser Rangfolge):
//! - `NoData`: seit [`WatchConfig::no_data_ms`] (Wanduhr) kein Puffer.
//! - `DigitalZero`: seit 10 s (Audiozeit) nur exakte Nullen.
//! - `Silent`: seit 30 s liegt der Pegel je Sekundenfenster unter -65 dBFS.
//! - `Clipping`: mehr als 1 % Vollaussteuerung im letzten 5-s-Fenster.
//! - `QueueOverflow`: die DSP-Queue ist uebergelaufen (haelt 10 s Audio an).
//!
//! Der Loopback meldet standardmaessig nichts davon: Stille der Gegenseite ist
//! normal, und WASAPI liefert dann gar keine Pakete (`loopback.rs`: "Timeout ist
//! normal (kein Ton)"), ein 3-s-`NoData` waere Dauer-Fehlalarm. Sein echter
//! Ausfall (`LoopbackDied`) kommt vom Watchdog im Recorder. Nur der
//! Warteschlangen-Ueberlauf wird auf beiden Kanaelen gemeldet.
//!
//! Nach einer Pause ([`SignalWatch::on_pause`]) zaehlen alle Fenster neu; eine
//! stehende Warnung wird dabei mit `Recovered` zurueckgenommen, denn die Pause
//! ist kein Ausfall.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use specta::Type;

/// Abtastrate der Live-Pfade (16 kHz mono).
const SAMPLES_PER_MS: u64 = 16;

/// Ein Kanalzustand fuer die Oberflaeche (`MeetingEvent::Health`).
/// `Recovered` nimmt die Bedingung des Kanals zurueck; `VadUnavailable` und
/// `LoopbackDied` sind einmalige Meldungen des Recorders und bleiben stehen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    NoData,
    DigitalZero,
    Silent,
    Clipping,
    QueueOverflow,
    VadUnavailable,
    LoopbackDied,
    Recovered,
}

/// Schwellen. `None` schaltet die Bedingung fuer den Kanal ab.
#[derive(Clone, Debug)]
pub struct WatchConfig {
    /// Wanduhr-Zeit ohne Puffer bis `NoData`.
    pub no_data_ms: Option<u64>,
    /// Zuschlag auf `no_data_ms`, solange noch nie ein Block ankam (das Geraet
    /// braucht etwas, bis es liefert).
    pub start_grace_ms: u64,
    pub digital_zero_ms: Option<u64>,
    pub silent_ms: Option<u64>,
    pub silent_dbfs: f32,
    /// Fenster fuer die Uebersteuerung; `None` = aus.
    pub clip_window_ms: Option<u64>,
    /// Anteil Vollaussteuerung, ab dem (echt darueber) gewarnt wird.
    pub clip_ratio: f32,
    /// So lange (Audiozeit) haelt ein Queue-Ueberlauf die Warnung.
    pub overflow_hold_ms: u64,
}

/// Loopback-`NoData` ist aus (siehe Moduldoku).
pub const LOOPBACK_NO_DATA_MS: Option<u64> = None;

impl WatchConfig {
    /// Mikrofon: liefert dauernd Puffer, jede Bedingung gilt.
    pub fn mic() -> Self {
        Self {
            no_data_ms: Some(3_000),
            start_grace_ms: 5_000,
            digital_zero_ms: Some(10_000),
            silent_ms: Some(30_000),
            silent_dbfs: -65.0,
            clip_window_ms: Some(5_000),
            clip_ratio: 0.01,
            overflow_hold_ms: 10_000,
        }
    }

    /// Systemton: nur der Warteschlangen-Ueberlauf (und ein optionales `NoData`).
    pub fn loopback() -> Self {
        Self {
            no_data_ms: LOOPBACK_NO_DATA_MS,
            digital_zero_ms: None,
            silent_ms: None,
            clip_window_ms: None,
            ..Self::mic()
        }
    }
}

/// Kennzahlen eines Blocks (i16 mono). Nur Zahlen, nie Inhalt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockStats {
    pub samples: u32,
    /// Samples, die exakt 0 sind.
    pub zeros: u32,
    /// Samples bei Vollaussteuerung (|x| >= 32 767).
    pub clipped: u32,
    /// Summe der Quadrate (auf +-1 normiert).
    pub sum_sq: f64,
}

impl BlockStats {
    pub fn of(samples: &[i16]) -> Self {
        let mut s = Self {
            samples: samples.len() as u32,
            ..Self::default()
        };
        for &x in samples {
            if x == 0 {
                s.zeros += 1;
            }
            if x.unsigned_abs() >= i16::MAX as u16 {
                s.clipped += 1;
            }
            let v = x as f64 / 32_768.0;
            s.sum_sq += v * v;
        }
        s
    }

    /// RMS in dBFS; leerer oder stiller Block liegt weit unter -120.
    pub fn dbfs(&self) -> f32 {
        dbfs(self.sum_sq, self.samples as u64)
    }
}

fn dbfs(sum_sq: f64, samples: u64) -> f32 {
    if samples == 0 || sum_sq <= 0.0 {
        return -200.0;
    }
    let rms = (sum_sq / samples as f64).sqrt();
    (20.0 * rms.log10()) as f32
}

/// Der Wachter eines Kanals.
pub struct SignalWatch {
    cfg: WatchConfig,
    /// Erfuellte Audiozeit seit dem Start, ohne Pausen und Luecken.
    audio_samples: u64,
    last_block_wall_ms: u64,
    seen_block: bool,
    paused: bool,
    no_data: bool,
    /// Laenge des aktuellen Laufs exakter Nullen (Samples).
    zero_run: u64,
    /// Sekundenfenster fuer den Pegel und die Laenge des leisen Laufs.
    win_samples: u64,
    win_sum_sq: f64,
    quiet_run: u64,
    /// Bloecke des Uebersteuerungsfensters (Samples, davon Vollaussteuerung).
    clip_blocks: VecDeque<(u32, u32)>,
    clip_total: u64,
    clip_hits: u64,
    clip_active: bool,
    overflow_until: Option<u64>,
    /// Was zuletzt gemeldet wurde (`None` = Kanal gesund).
    reported: Option<HealthState>,
}

impl SignalWatch {
    pub fn new(cfg: WatchConfig, start_wall_ms: u64) -> Self {
        Self {
            cfg,
            audio_samples: 0,
            last_block_wall_ms: start_wall_ms,
            seen_block: false,
            paused: false,
            no_data: false,
            zero_run: 0,
            win_samples: 0,
            win_sum_sq: 0.0,
            quiet_run: 0,
            clip_blocks: VecDeque::with_capacity(512),
            clip_total: 0,
            clip_hits: 0,
            clip_active: false,
            overflow_until: None,
            reported: None,
        }
    }

    /// Ein Block ist angekommen. `wall_ms`: monotone Wanduhr (nur fuer `NoData`).
    pub fn on_block(&mut self, wall_ms: u64, block: &BlockStats) -> Option<HealthState> {
        self.last_block_wall_ms = wall_ms;
        self.seen_block = true;
        self.paused = false;
        self.no_data = false;
        if block.samples == 0 {
            return self.settle();
        }
        self.audio_samples += block.samples as u64;
        let n = block.samples as u64;

        // Exakte Nullen.
        if block.zeros == block.samples {
            self.zero_run += n;
        } else {
            self.zero_run = 0;
        }

        // Pegel je Sekundenfenster: ein leises Fenster verlaengert den Lauf,
        // ein lautes beendet ihn.
        self.win_samples += n;
        self.win_sum_sq += block.sum_sq;
        if self.win_samples >= 1_000 * SAMPLES_PER_MS {
            if dbfs(self.win_sum_sq, self.win_samples) < self.cfg.silent_dbfs {
                self.quiet_run += self.win_samples;
            } else {
                self.quiet_run = 0;
            }
            self.win_samples = 0;
            self.win_sum_sq = 0.0;
        }

        // Uebersteuerung im gleitenden Fenster.
        if let Some(window_ms) = self.cfg.clip_window_ms {
            let window = window_ms * SAMPLES_PER_MS;
            self.clip_blocks.push_back((block.samples, block.clipped));
            self.clip_total += n;
            self.clip_hits += block.clipped as u64;
            while let Some(&(s, c)) = self.clip_blocks.front() {
                if self.clip_total - (s as u64) < window {
                    break;
                }
                self.clip_total -= s as u64;
                self.clip_hits -= c as u64;
                self.clip_blocks.pop_front();
            }
            if self.clip_total >= window {
                let ratio = self.clip_hits as f64 / self.clip_total as f64;
                let limit = self.cfg.clip_ratio as f64;
                if ratio > limit {
                    self.clip_active = true;
                } else if ratio <= limit / 2.0 {
                    // Hysterese: erst deutlich darunter ist es wieder gut.
                    self.clip_active = false;
                }
            }
        }
        self.settle()
    }

    /// Wanduhr-Tick des DSP-Threads (auch ohne Block): prueft `NoData`.
    pub fn on_tick(&mut self, wall_ms: u64) -> Option<HealthState> {
        let limit = self.cfg.no_data_ms?;
        if self.paused {
            return None;
        }
        let grace = if self.seen_block {
            0
        } else {
            self.cfg.start_grace_ms
        };
        if wall_ms.saturating_sub(self.last_block_wall_ms) >= limit + grace {
            self.no_data = true;
        }
        self.settle()
    }

    /// Die DSP-Queue ist uebergelaufen (der Block danach loest die Meldung aus).
    pub fn on_overflow(&mut self) {
        self.overflow_until = Some(self.audio_samples + self.cfg.overflow_hold_ms * SAMPLES_PER_MS);
    }

    /// Pause: alle Fenster von vorn, stehende Warnung zurueckgenommen.
    pub fn on_pause(&mut self) -> Option<HealthState> {
        self.paused = true;
        self.no_data = false;
        self.zero_run = 0;
        self.win_samples = 0;
        self.win_sum_sq = 0.0;
        self.quiet_run = 0;
        self.clip_blocks.clear();
        self.clip_total = 0;
        self.clip_hits = 0;
        self.clip_active = false;
        self.overflow_until = None;
        self.settle()
    }

    fn condition(&self) -> Option<HealthState> {
        let reached = |run: u64, ms: Option<u64>| ms.is_some_and(|m| run >= m * SAMPLES_PER_MS);
        if self.no_data {
            Some(HealthState::NoData)
        } else if reached(self.zero_run, self.cfg.digital_zero_ms) {
            Some(HealthState::DigitalZero)
        } else if reached(self.quiet_run, self.cfg.silent_ms) {
            Some(HealthState::Silent)
        } else if self.clip_active {
            Some(HealthState::Clipping)
        } else if self.overflow_until.is_some_and(|u| self.audio_samples < u) {
            Some(HealthState::QueueOverflow)
        } else {
            None
        }
    }

    /// Meldet nur, wenn sich die Bedingung gegenueber der letzten Meldung aenderte.
    fn settle(&mut self) -> Option<HealthState> {
        let now = self.condition();
        if now == self.reported {
            return None;
        }
        self.reported = now;
        Some(now.unwrap_or(HealthState::Recovered))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 30 ms bei 16 kHz.
    const BLOCK: usize = 480;

    fn block_ms() -> u64 {
        BLOCK as u64 / 16
    }

    fn silence() -> BlockStats {
        BlockStats::of(&[0i16; BLOCK])
    }

    /// Leises Rauschen, ~ -50 dBFS: klar ueber der Stille-Schwelle.
    fn quiet_noise() -> BlockStats {
        let s: Vec<i16> = (0..BLOCK)
            .map(|i| if i % 2 == 0 { 100 } else { -100 })
            .collect();
        BlockStats::of(&s)
    }

    /// Unterhalb von -65 dBFS, aber nicht digital null (Amplitude 5).
    fn hiss() -> BlockStats {
        let s: Vec<i16> = (0..BLOCK)
            .map(|i| if i % 2 == 0 { 5 } else { -5 })
            .collect();
        BlockStats::of(&s)
    }

    fn clipped(clipped_samples: usize) -> BlockStats {
        let s: Vec<i16> = (0..BLOCK)
            .map(|i| if i < clipped_samples { i16::MAX } else { 300 })
            .collect();
        BlockStats::of(&s)
    }

    /// Fuettert `ms` Audio in 30-ms-Bloecken; `wall` laeuft mit. Liefert alle
    /// gemeldeten Zustandswechsel mit ihrem Audio-Zeitpunkt.
    fn feed(
        w: &mut SignalWatch,
        wall: &mut u64,
        ms: u64,
        b: &BlockStats,
    ) -> Vec<(u64, HealthState)> {
        let mut out = Vec::new();
        let mut fed = 0;
        while fed < ms {
            *wall += block_ms();
            fed += block_ms();
            if let Some(s) = w.on_block(*wall, b) {
                out.push((fed, s));
            }
        }
        out
    }

    fn states(v: &[(u64, HealthState)]) -> Vec<HealthState> {
        v.iter().map(|(_, s)| *s).collect()
    }

    #[test]
    fn a_healthy_microphone_never_reports_anything() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 0);
        let mut wall = 0;
        assert!(feed(&mut w, &mut wall, 120_000, &quiet_noise()).is_empty());
        assert_eq!(w.on_tick(wall + 1_000), None);
    }

    #[test]
    fn no_data_after_three_seconds_and_recovered_with_the_next_block() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 0);
        let mut wall = 0;
        feed(&mut w, &mut wall, 1_000, &quiet_noise());
        assert_eq!(w.on_tick(wall + 2_900), None);
        assert_eq!(w.on_tick(wall + 3_000), Some(HealthState::NoData));
        // Zustandswechsel, keine Flut: weitere Ticks melden nichts.
        assert_eq!(w.on_tick(wall + 4_000), None);
        assert_eq!(w.on_tick(wall + 9_000), None);
        let back = feed(&mut w, &mut { wall + 9_030 }, 30, &quiet_noise());
        assert_eq!(states(&back), vec![HealthState::Recovered]);
    }

    #[test]
    fn a_mic_that_never_delivers_is_reported_after_the_start_grace() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 10_000);
        // Vor dem ersten Block gilt die Anlaufzeit obendrauf.
        assert_eq!(w.on_tick(10_000 + 3_000), None);
        let cfg = WatchConfig::mic();
        let due = 10_000 + cfg.no_data_ms.unwrap() + cfg.start_grace_ms;
        assert_eq!(w.on_tick(due), Some(HealthState::NoData));
    }

    #[test]
    fn ten_seconds_of_exact_zeros_are_a_digital_zero_and_speech_recovers() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 0);
        let mut wall = 0;
        assert!(feed(&mut w, &mut wall, 2_000, &quiet_noise()).is_empty());
        let zeros = feed(&mut w, &mut wall, 10_200, &silence());
        assert_eq!(states(&zeros), vec![HealthState::DigitalZero]);
        // Gemeldet kurz nach 10 s Nullen, nicht frueher.
        assert!(zeros[0].0 >= 10_000 && zeros[0].0 < 10_100, "{zeros:?}");
        let back = feed(&mut w, &mut wall, 30, &quiet_noise());
        assert_eq!(states(&back), vec![HealthState::Recovered]);
    }

    #[test]
    fn a_mic_below_minus_65_dbfs_is_silent_after_30_seconds_not_before() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 0);
        let mut wall = 0;
        let first = feed(&mut w, &mut wall, 29_000, &hiss());
        assert!(first.is_empty(), "{first:?}");
        let then = feed(&mut w, &mut wall, 2_000, &hiss());
        assert_eq!(states(&then), vec![HealthState::Silent]);
        assert!(feed(&mut w, &mut wall, 20_000, &hiss()).is_empty());
        let back = feed(&mut w, &mut wall, 2_500, &quiet_noise());
        assert_eq!(states(&back), vec![HealthState::Recovered]);
    }

    #[test]
    fn digital_zero_outranks_silent_and_hands_over_to_it_without_a_recovered() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 0);
        let mut wall = 0;
        // Null-Bloecke sind auch leiser als -65 dBFS. Solange Nullen kommen,
        // gilt DigitalZero (hoehere Rangfolge), auch nach 30 s.
        let zeros = feed(&mut w, &mut wall, 35_000, &silence());
        assert_eq!(states(&zeros), vec![HealthState::DigitalZero]);
        // Erste Nicht-Null-Sekunden, aber weiter leise: das Stille-Fenster war
        // schon voll, also direkt Silent, kein Recovered dazwischen.
        let hiss_then = feed(&mut w, &mut wall, 30, &hiss());
        assert_eq!(states(&hiss_then), vec![HealthState::Silent]);
    }

    #[test]
    fn clipping_needs_more_than_one_percent_of_full_scale_within_five_seconds() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 0);
        let mut wall = 0;
        // 4 von 480 Samples = 0,83 %: kein Alarm.
        assert!(feed(&mut w, &mut wall, 20_000, &clipped(4)).is_empty());
        // 10 von 480 = 2,1 %: Alarm, sobald das 5-s-Fenster voll ist.
        let hot = feed(&mut w, &mut wall, 10_000, &clipped(10));
        assert_eq!(states(&hot), vec![HealthState::Clipping]);
        // Wieder sauber: nach hoechstens einem Fenster ist der Alarm weg.
        let back = feed(&mut w, &mut wall, 6_000, &quiet_noise());
        assert_eq!(states(&back), vec![HealthState::Recovered]);
    }

    #[test]
    fn a_single_clipped_block_is_below_one_percent_of_the_window() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 0);
        let mut wall = 0;
        feed(&mut w, &mut wall, 10_000, &quiet_noise());
        // 30 ms voll = 0,6 % eines 5-s-Fensters: kein Alarm.
        assert!(feed(&mut w, &mut wall, 30, &clipped(BLOCK)).is_empty());
        assert!(feed(&mut w, &mut wall, 10_000, &quiet_noise()).is_empty());
    }

    #[test]
    fn the_loopback_reports_neither_silence_nor_zeros_nor_clipping() {
        let mut w = SignalWatch::new(WatchConfig::loopback(), 0);
        let mut wall = 0;
        assert!(feed(&mut w, &mut wall, 60_000, &silence()).is_empty());
        assert!(feed(&mut w, &mut wall, 60_000, &hiss()).is_empty());
        assert!(feed(&mut w, &mut wall, 10_000, &clipped(BLOCK)).is_empty());
        // Ohne Puffer meldet der Standard-Loopback nichts: WASAPI liefert bei
        // Stille der Gegenseite gar keine Pakete.
        assert_eq!(w.on_tick(wall + 600_000), None);
    }

    #[test]
    fn a_loopback_with_a_no_data_limit_reports_no_data_only() {
        let cfg = WatchConfig {
            no_data_ms: Some(3_000),
            ..WatchConfig::loopback()
        };
        let mut w = SignalWatch::new(cfg, 0);
        let mut wall = 0;
        feed(&mut w, &mut wall, 1_000, &quiet_noise());
        assert_eq!(w.on_tick(wall + 3_000), Some(HealthState::NoData));
        let back = feed(&mut w, &mut { wall + 3_100 }, 30, &silence());
        assert_eq!(states(&back), vec![HealthState::Recovered]);
    }

    #[test]
    fn queue_overflow_is_reported_once_and_clears_after_ten_quiet_seconds() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 0);
        let mut wall = 0;
        feed(&mut w, &mut wall, 1_000, &quiet_noise());
        // Mehrere Ueberlaeufe hintereinander: EIN Ereignis.
        w.on_overflow();
        let first = feed(&mut w, &mut wall, 30, &quiet_noise());
        assert_eq!(states(&first), vec![HealthState::QueueOverflow]);
        for _ in 0..5 {
            w.on_overflow();
            assert!(feed(&mut w, &mut wall, 300, &quiet_noise()).is_empty());
        }
        let gone = feed(&mut w, &mut wall, 11_000, &quiet_noise());
        assert_eq!(states(&gone), vec![HealthState::Recovered]);
    }

    #[test]
    fn a_pause_is_not_a_failure_and_clears_a_standing_warning() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 0);
        let mut wall = 0;
        feed(&mut w, &mut wall, 11_000, &silence());
        assert_eq!(w.on_pause(), Some(HealthState::Recovered));
        // Waehrend der Pause bleibt der Wachter stumm, auch nach Minuten.
        assert_eq!(w.on_tick(wall + 300_000), None);
        // Nach der Pause zaehlen die Fenster neu.
        let mut wall = wall + 300_000;
        assert!(feed(&mut w, &mut wall, 9_000, &silence()).is_empty());
        assert_eq!(
            states(&feed(&mut w, &mut wall, 2_000, &silence())),
            vec![HealthState::DigitalZero]
        );
    }

    #[test]
    fn a_pause_without_a_warning_reports_nothing() {
        let mut w = SignalWatch::new(WatchConfig::mic(), 0);
        let mut wall = 0;
        feed(&mut w, &mut wall, 1_000, &quiet_noise());
        assert_eq!(w.on_pause(), None);
    }

    #[test]
    fn block_stats_count_zeros_full_scale_and_level() {
        let b = BlockStats::of(&[0, 0, i16::MAX, i16::MIN, 100]);
        assert_eq!(b.samples, 5);
        assert_eq!(b.zeros, 2);
        assert_eq!(b.clipped, 2);
        assert!(BlockStats::of(&[0i16; 10]).dbfs() < -120.0);
        let full = BlockStats::of(&[i16::MAX; 16]).dbfs();
        assert!(full > -0.1 && full <= 0.0, "{full}");
        assert!(BlockStats::of(&[]).dbfs() < -120.0);
    }

    #[test]
    fn health_states_serialize_as_snake_case_codes() {
        assert_eq!(
            serde_json::to_string(&HealthState::DigitalZero).unwrap(),
            "\"digital_zero\""
        );
        assert_eq!(
            serde_json::to_string(&HealthState::QueueOverflow).unwrap(),
            "\"queue_overflow\""
        );
        assert_eq!(
            serde_json::to_string(&HealthState::VadUnavailable).unwrap(),
            "\"vad_unavailable\""
        );
    }
}
