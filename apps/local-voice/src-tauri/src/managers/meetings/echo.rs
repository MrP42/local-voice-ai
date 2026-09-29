//! Echo-Unterdrückung (AEC) für die Ich-Spur einer Besprechung (M2, Paket P2c1).
//!
//! Läuft ein Besprechungspartner über die Lautsprecher, hört das Mikrofon ihn
//! ein zweites Mal – die Gegenseite stünde im Transkript doppelt, einmal als
//! „Gegenseite" (Systemton) und einmal als „Ich" (Mikrofon-Echo). Dieses Modul
//! nimmt das Echo aus dem Mikrofonsignal, mit dem Systemton als Referenz.
//!
//! Drei reine, ohne Gerät testbare Bausteine:
//!
//! * [`EchoCanceller`] – Hülle um sonora AEC3 (16 kHz mono, 10-ms-Frames zu
//!   160 Samples). `process(mic, referenz, out)`, `reset()`, [`EchoStats`]
//!   (ERL, ERLE, geschätzte Verzögerung).
//! * [`Aligner`] – legt Referenz-Frames (Loopback) anhand ihrer Zeitstempel auf
//!   die Mikrofon-Achse und liefert je Mikrofon-Frame den passenden
//!   Referenz-Frame. AEC3 verkraftet eine konstante Verzögerung bis ~500 ms,
//!   aber keine Referenz, die NACH dem Echo liegt; deshalb die gemeinsame Achse.
//! * [`should_drop_duplicate`] – Sicherheitsnetz auf Textebene (Konzept §3.3),
//!   falls trotz AEC ein Echo als „Ich"-Segment durchkommt.
//!
//! # Einbau (P2c2: `dsp.rs`, `EchoStage`)
//!
//! Im „meeting-dsp"-Thread, ein Thread für Aligner **und** Canceller:
//!
//! ```text
//! offset_ms = mic_qpc0 - sys_qpc0                      // einmal, in ms
//! aligner.set_offset_ms(offset_ms);
//! je Loopback-Block:  if aligner.push_reference(t_ref_ms, block) == Reset { canceller.reset() }
//! je Mikrofon-Frame:  if aligner.ready_for(mic_pos, 160) {
//!                         aligner.pull(mic_pos, &mut ref_frame);
//!                         canceller.process(mic, &ref_frame, &mut out)
//!                     }
//! Pause / Fortsetzen: aligner.reset(); canceller.reset();
//! ```
//!
//! Zeitachsen: `t_ref_ms` und die Mikrofon-Position müssen aus QPC-Zeitstempeln
//! stammen. Wer beide aus gezählten Samples ableitet, sieht die Uhrendrift der
//! zwei Geräte nie.
//!
//! # Echtzeit und Speicher
//!
//! * Kein I/O, keine Sperre, keine Wartezeit. Alles Nötige wird in
//!   `EchoCanceller::new()` / `Aligner::new()` angelegt (Ring 64 KB, AEC3-
//!   Zustände ~650 KB). Je Frame allokieren **Aligner und Hülle nichts**
//!   (Test `no_allocation_per_frame_in_steady_state`); sonora selbst allokiert
//!   im Capture-Pfad intern ~35-mal je Frame (gemessen, im Test protokolliert).
//!   Das ist im DSP-Thread unkritisch (~µs je Frame) und gehört nie in einen
//!   Audio-Callback – dort läuft nur das Weiterreichen der Samples.
//!   `reset()` des Cancellers baut AEC3 neu auf (einmalig ~650 KB), nur bei
//!   Pause/Realign, nie je Frame. Ein fehlgeschlagenes Anlegen (Speicher voll)
//!   bricht wie jede Rust-Allokation ab, deshalb legt der Aufrufer beides beim
//!   Start der Besprechung an, nicht mitten im Betrieb.
//! * Ein Überlauf des Referenz-Rings wird nicht verschluckt, sondern in
//!   [`AlignerStats::overruns`] gezählt; ebenso zu späte Referenz
//!   (`underruns`), Kompensationen und Resets.
//! * Nicht `Sync`-geteilt: alle Methoden nehmen `&mut self`, die Typen sind
//!   `Send` und gehören dem DSP-Thread.
//!
//! Einige Zaehler/Varianten dienen nur Tests und dem Bericht.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};

use sonora::config::EchoCanceller as AecConfig;
use sonora::{AudioProcessing, Config, StreamConfig};

use crate::selftest::normalize_word;

/// Abtastrate der Besprechungs-Pipeline.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
/// Frame-Länge der AEC: 10 ms bei 16 kHz.
pub const FRAME_SAMPLES: usize = 160;

const SAMPLES_PER_MS: i64 = 16;

// ───────────────────────────── Pegel-Hilfe ──────────────────────────────

/// RMS-Pegel in dBFS (Vollaussteuerung = 0 dB); digitale Stille = −120 dB.
/// Gebraucht für `rms_db_diff` von [`should_drop_duplicate`] und für die Tests.
pub fn rms_dbfs(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return -120.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    let rms = (sum / samples.len() as f64).sqrt() / 32768.0;
    (20.0 * rms.max(1e-6).log10()) as f32
}

// ─────────────────────────── EchoCanceller ─────────────────────────────

/// Fehler von [`EchoCanceller::process`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EchoError {
    /// Mikrofon-, Referenz- oder Ausgabe-Slice hat nicht [`FRAME_SAMPLES`] Samples.
    FrameLength {
        mic: usize,
        reference: usize,
        out: usize,
    },
}

impl fmt::Display for EchoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameLength {
                mic,
                reference,
                out,
            } => write!(
                f,
                "AEC braucht Frames zu {FRAME_SAMPLES} Samples (mic {mic}, Referenz {reference}, out {out})"
            ),
        }
    }
}

impl std::error::Error for EchoError {}

/// Kennzahlen der Echo-Unterdrückung. `erl_db`/`erle_db`/`delay_ms` stammen von
/// AEC3 selbst und sind `None`, solange dort noch nichts gemessen wurde.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EchoStats {
    /// Verarbeitete Frames seit dem Anlegen (überlebt `reset()`).
    pub frames: u64,
    /// Echo-Dämpfung durch den Raum (Referenz zu Echo), dB.
    pub erl_db: Option<f32>,
    /// Zusätzliche Dämpfung durch die AEC (Echo zu Ausgabe), dB.
    pub erle_db: Option<f32>,
    /// Aktuell geschätzte Verzögerung Referenz → Echo, ms.
    pub delay_ms: Option<i32>,
    /// Frames, bei denen AEC3 einen Fehler meldete; das Mikrofon lief unverändert durch.
    pub errors: u64,
    /// Panics innerhalb von AEC3 (abgefangen); danach läuft das Mikrofon
    /// unverändert durch, bis `reset()` AEC3 neu aufbaut.
    pub panics: u64,
    /// Anzahl `reset()`-Aufrufe.
    pub resets: u32,
}

/// Führt `f` aus und macht aus einem Panic ein `None` (kein Weiterwerfen in den
/// DSP-Thread: ein AEC-Fehler darf die Aufnahme nicht beenden).
fn guarded<R>(f: impl FnOnce() -> R) -> Option<R> {
    catch_unwind(AssertUnwindSafe(f)).ok()
}

fn build_apm() -> AudioProcessing {
    let stream = StreamConfig::new(SAMPLE_RATE_HZ, 1);
    // Nur AEC3: Rauschunterdrückung und AGC bleiben aus, weil die Spracherkennung
    // das Rohsignal bevorzugt (Konzept §3.3); NS verbessert nur die ERLE-Zahl.
    let config = Config {
        echo_canceller: Some(AecConfig::default()),
        ..Default::default()
    };
    AudioProcessing::builder()
        .config(config)
        .capture_config(stream)
        .render_config(stream)
        .build()
}

/// Hülle um sonora AEC3 für 16 kHz mono, Frames zu 160 Samples.
pub struct EchoCanceller {
    apm: AudioProcessing,
    render_in: [f32; FRAME_SAMPLES],
    render_out: [f32; FRAME_SAMPLES],
    mic_in: [f32; FRAME_SAMPLES],
    mic_out: [f32; FRAME_SAMPLES],
    /// Nach einem Panic in AEC3: Durchreichen, bis `reset()`.
    failed: bool,
    frames: u64,
    errors: u64,
    panics: u64,
    resets: u32,
}

impl Default for EchoCanceller {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoCanceller {
    pub fn new() -> Self {
        Self {
            apm: build_apm(),
            render_in: [0.0; FRAME_SAMPLES],
            render_out: [0.0; FRAME_SAMPLES],
            mic_in: [0.0; FRAME_SAMPLES],
            mic_out: [0.0; FRAME_SAMPLES],
            failed: false,
            frames: 0,
            errors: 0,
            panics: 0,
            resets: 0,
        }
    }

    /// Ein 10-ms-Frame: `mic` (Mikrofon, roh) und `reference` (Systemton auf der
    /// Mikrofon-Achse, siehe [`Aligner::pull`]) → `out` (Mikrofon ohne Echo).
    ///
    /// Alle drei Slices müssen genau [`FRAME_SAMPLES`] lang sein, sonst
    /// `Err(FrameLength)` und `out` bleibt unangetastet. Meldet AEC3 selbst
    /// einen Fehler oder panict es, wird das Mikrofon unverändert nach `out`
    /// kopiert (Zähler in [`EchoStats`]) – die Aufnahme geht nie verloren.
    pub fn process(
        &mut self,
        mic: &[i16],
        reference: &[i16],
        out: &mut [i16],
    ) -> Result<(), EchoError> {
        if mic.len() != FRAME_SAMPLES
            || reference.len() != FRAME_SAMPLES
            || out.len() != FRAME_SAMPLES
        {
            return Err(EchoError::FrameLength {
                mic: mic.len(),
                reference: reference.len(),
                out: out.len(),
            });
        }
        self.frames += 1;
        if self.failed {
            out.copy_from_slice(mic);
            return Ok(());
        }
        for i in 0..FRAME_SAMPLES {
            self.render_in[i] = f32::from(reference[i]) / 32768.0;
            self.mic_in[i] = f32::from(mic[i]) / 32768.0;
        }
        let Self {
            apm,
            render_in,
            render_out,
            mic_in,
            mic_out,
            ..
        } = self;
        let result = guarded(|| {
            apm.process_render_f32(&[&render_in[..]], &mut [&mut render_out[..]])?;
            apm.process_capture_f32(&[&mic_in[..]], &mut [&mut mic_out[..]])
        });
        match result {
            Some(Ok(())) => {
                for i in 0..FRAME_SAMPLES {
                    // NaN wird beim Cast zu 0; Übersteuerung wird begrenzt.
                    out[i] = (self.mic_out[i] * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
                }
            }
            Some(Err(_)) => {
                self.errors += 1;
                out.copy_from_slice(mic);
            }
            None => {
                self.panics += 1;
                self.failed = true;
                out.copy_from_slice(mic);
            }
        }
        Ok(())
    }

    /// Verwirft alles Gelernte (Pause, Realign, Neustart der Referenz). Baut AEC3
    /// neu auf – einmalige Allokation, nicht für den Frame-Takt gedacht.
    pub fn reset(&mut self) {
        self.apm = build_apm();
        self.failed = false;
        self.resets += 1;
    }

    /// Kennzahlen; ERL/ERLE/Verzögerung kommen aus AEC3 und sind kurz nach dem
    /// Start bzw. nach `reset()` noch `None`.
    pub fn stats(&self) -> EchoStats {
        let s = self.apm.statistics();
        EchoStats {
            frames: self.frames,
            erl_db: s.echo_return_loss.map(|v| v as f32),
            erle_db: s.echo_return_loss_enhancement.map(|v| v as f32),
            delay_ms: s.delay_ms,
            errors: self.errors,
            panics: self.panics,
            resets: self.resets,
        }
    }
}

// ────────────────────────────── Aligner ────────────────────────────────

/// Ringgröße: 2^15 Samples = 2,048 s bei 16 kHz.
pub const RING_SAMPLES: usize = 32_768;
const RING_MASK: usize = RING_SAMPLES - 1;

/// Abweichung Zeitstempel ↔ gezählte Samples, ab der nachgestellt wird.
pub const DRIFT_TOLERANCE_MS: i64 = 20;
/// Beim Nachstellen wird bis unter diese Grenze korrigiert (Hysterese gegen
/// Zeitstempel-Jitter).
pub const DRIFT_SETTLE_MS: i64 = 4;
/// Über dieser Abweichung wird nicht mehr sample-weise nachgestellt, sondern
/// einmalig angesprungen (AEC3 muss ohnehin neu einschwingen).
pub const DRIFT_JUMP_MS: i64 = 100;
/// Über dieser Abweichung meldet der Aligner [`AlignEvent::Reset`].
pub const DRIFT_RESET_MS: i64 = 500;
/// Höchstens `Blocklänge / SLEW_DIVISOR` Samples je Block einfügen/verwerfen
/// (≈ 1,5 %), damit die Referenz nie hörbar gestaucht wird.
const SLEW_DIVISOR: usize = 64;

/// Ergebnis von [`Aligner::push_reference`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum AlignEvent {
    /// Nichts zu tun.
    None,
    /// Drift nachgestellt: `samples` > 0 = eingefügt (Referenz lief zu langsam),
    /// < 0 = verworfen (Referenz lief zu schnell). Je Block bei Slew klein,
    /// bei einem Sprung (> [`DRIFT_JUMP_MS`]) die ganze Abweichung.
    Compensated { samples: i32 },
    /// |Drift| > [`DRIFT_RESET_MS`]: der Aligner hat sich neu verankert.
    /// **Der Aufrufer muss `EchoCanceller::reset()` rufen** und `aec_realign`
    /// loggen. `drift_ms` mit Vorzeichen wie `Compensated`.
    Reset { drift_ms: i32 },
}

/// Was [`Aligner::pull`] geliefert hat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pulled {
    /// Referenzdaten geliefert (Teile davor/dahinter können Nullen sein).
    Data,
    /// Der Frame liegt vor dem ersten Referenz-Sample oder es kam noch keine
    /// Referenz: Nullen (Loopback startet später, oder gar kein Systemton).
    BeforeStart,
    /// Referenz kam zu spät (Frame reicht über das neueste Sample hinaus); der
    /// Rest ist mit Nullen gefüllt. Gezählt in `underruns`.
    Underrun,
    /// Referenz wurde schon überschrieben (Ring zu klein für den Abstand
    /// zwischen Schreiben und Lesen); der Rest ist mit Nullen gefüllt. Gezählt
    /// in `overruns`.
    Overrun,
}

/// Zähler des Aligners; nichts davon wird verschluckt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlignerStats {
    pub pushed_samples: u64,
    pub pulled_frames: u64,
    /// Frames, deren Referenz noch nicht (vollständig) da war.
    pub underruns: u64,
    /// Frames bzw. Blöcke, bei denen Referenz überschrieben wurde/verloren ging.
    pub overruns: u64,
    pub inserted_samples: u64,
    pub dropped_samples: u64,
    /// Einmalige Sprünge (100–500 ms).
    pub jumps: u64,
    /// Neuverankerungen wegen |Drift| > 500 ms sowie explizite `reset()`-Aufrufe.
    pub resets: u64,
    /// Zeitstempel, die nicht endlich/unplausibel waren (Block wurde nach
    /// Sample-Zahl angehängt).
    pub bad_timestamps: u64,
    /// Größte beobachtete |Abweichung| Zeitstempel ↔ Sample-Zahl, ms.
    pub max_abs_drift_ms: i64,
}

/// Legt Referenz-(Loopback-)Frames auf die Mikrofon-Achse.
///
/// **Achsen und Vorzeichen.** Die Referenz hat eine eigene Achse: `t_ref_ms` = Zeit
/// seit dem ersten Loopback-Puffer. Die Mikrofon-Achse zählt ab dem ersten
/// Mikrofon-Sample (`mic_pos` in Samples = Frames von `mic.wav`). Für denselben
/// Zeitpunkt gilt `t_ref − t_mic = offset_ms = mic_qpc0 − sys_qpc0`. Startet der
/// Loopback später als das Mikrofon (der Normalfall, ≤ 5 s), ist der Versatz
/// **negativ**: Mikrofon-Frames vor Beginn der Referenz bekommen Nullen. Startete
/// er früher, ist er **positiv**, und der Anfang der Referenz wird übersprungen.
///
/// **Drift.** Jeder Referenz-Block trägt den Zeitstempel seines ersten Samples.
/// Weicht dieser von der Position ab, die die gezählten Samples ergeben, ist das
/// die Uhrendrift (oder Verlust): bis 20 ms wird sie ignoriert (Zeitstempel-Jitter,
/// AEC3 fängt so etwas selbst auf), darüber wird mit einzelnen eingefügten bzw.
/// verworfenen Samples nachgestellt (höchstens 1,5 % je Block), über 100 ms
/// einmalig angesprungen, über 500 ms gemeldet ([`AlignEvent::Reset`]).
pub struct Aligner {
    ring: Box<[i16]>,
    /// Referenz-Achse: Index des nächsten zu schreibenden Samples; `None` bis zum
    /// ersten Block nach Anlegen/`reset()`.
    write_pos: Option<i64>,
    /// Referenz-Achse: Index des ersten gültigen Samples (davor: Nullen).
    first_pos: i64,
    offset_samples: i64,
    slewing: bool,
    stats: AlignerStats,
}

impl Default for Aligner {
    fn default() -> Self {
        Self::new()
    }
}

impl Aligner {
    pub fn new() -> Self {
        Self {
            ring: vec![0i16; RING_SAMPLES].into_boxed_slice(),
            write_pos: None,
            first_pos: 0,
            offset_samples: 0,
            slewing: false,
            stats: AlignerStats::default(),
        }
    }

    /// Setzt den Versatz Referenz-Achse − Mikrofon-Achse (siehe Typ-Doku:
    /// `mic_qpc0 − sys_qpc0` in ms). Wirkt ab dem nächsten `pull`. Nicht endliche
    /// Werte werden ignoriert.
    pub fn set_offset_ms(&mut self, offset_ms: f64) {
        if offset_ms.is_finite() && offset_ms.abs() < 1.0e9 {
            self.offset_samples = (offset_ms * SAMPLES_PER_MS as f64).round() as i64;
        }
    }

    /// Ein Block Referenz (16 kHz mono i16, beliebige Länge) samt Zeitstempel
    /// `t_ref_ms` seines ersten Samples auf der Referenz-Achse.
    pub fn push_reference(&mut self, t_ref_ms: f64, samples: &[i16]) -> AlignEvent {
        if samples.is_empty() {
            return AlignEvent::None;
        }
        self.stats.pushed_samples += samples.len() as u64;
        let timestamp_ok = t_ref_ms.is_finite() && t_ref_ms.abs() < 1.0e9;
        let expected = if timestamp_ok {
            Some((t_ref_ms * SAMPLES_PER_MS as f64).round() as i64)
        } else {
            self.stats.bad_timestamps += 1;
            None
        };

        let Some(wp) = self.write_pos else {
            let start = expected.unwrap_or(0);
            self.write_pos = Some(start);
            self.first_pos = start;
            self.write_block(samples);
            return AlignEvent::None;
        };
        let Some(expected) = expected else {
            self.write_block(samples); // ohne Zeitstempel: nach Sample-Zahl anhängen
            return AlignEvent::None;
        };

        let gap = expected - wp; // > 0: Referenz hinkt hinterher (fehlende Samples)
        let abs = gap.abs();
        self.stats.max_abs_drift_ms = self.stats.max_abs_drift_ms.max(abs / SAMPLES_PER_MS);

        if abs > DRIFT_RESET_MS * SAMPLES_PER_MS {
            self.write_pos = Some(expected);
            self.first_pos = expected;
            self.slewing = false;
            self.stats.resets += 1;
            self.write_block(samples);
            return AlignEvent::Reset {
                drift_ms: (gap / SAMPLES_PER_MS) as i32,
            };
        }
        if abs > DRIFT_JUMP_MS * SAMPLES_PER_MS {
            self.slewing = false;
            self.stats.jumps += 1;
            if gap > 0 {
                self.write_fill(0, gap as usize);
                self.stats.inserted_samples += gap as u64;
            } else {
                // Zeitstempel liegt vor unserem Schreibzeiger: dort weiterschreiben,
                // das Überlappende wird überschrieben.
                self.write_pos = Some(expected);
                self.first_pos = self.first_pos.min(expected);
                self.stats.dropped_samples += abs as u64;
            }
            self.write_block(samples);
            return AlignEvent::Compensated {
                samples: gap as i32,
            };
        }

        // Sample-weise nachstellen, mit Hysterese: ab 20 ms Abweichung an, bei
        // ≤ 4 ms wieder aus.
        if self.slewing && abs <= DRIFT_SETTLE_MS * SAMPLES_PER_MS {
            self.slewing = false;
        } else if !self.slewing && abs > DRIFT_TOLERANCE_MS * SAMPLES_PER_MS {
            self.slewing = true;
        }
        if !self.slewing {
            self.write_block(samples);
            return AlignEvent::None;
        }
        let max_step = (samples.len() / SLEW_DIVISOR).max(1) as i64;
        let step = gap.clamp(-max_step, max_step);
        if step > 0 {
            let hold = samples[samples.len() - 1];
            self.write_block(samples);
            self.write_fill(hold, step as usize);
            self.stats.inserted_samples += step as u64;
        } else {
            let keep = samples.len().saturating_sub((-step) as usize);
            self.write_block(&samples[..keep]);
            self.stats.dropped_samples += (samples.len() - keep) as u64;
        }
        AlignEvent::Compensated {
            samples: step as i32,
        }
    }

    /// Ist die Referenz für den Mikrofon-Frame `[mic_pos, mic_pos + len)` da?
    /// Wahr, wenn die Referenz den Frame ganz abdeckt **oder** der Frame ganz vor
    /// dem ersten Referenz-Sample liegt (dann sind Nullen richtig – auch vor dem
    /// allerersten Block, solange der Frame vor Referenz-Achse 0 liegt, damit ein
    /// später startender Loopback die Mikrofon-Frames nicht aufhält). Falsch, wenn
    /// noch Referenz fehlt, die kommen sollte: der Aufrufer wartet dann eine
    /// begrenzte Zeit auf den Loopback und arbeitet danach mit Nullen weiter
    /// (der Loopback kann ganz fehlen).
    pub fn ready_for(&self, mic_pos: u64, len: usize) -> bool {
        let end = mic_pos as i64 + self.offset_samples + len as i64;
        match self.write_pos {
            None => end <= 0,
            Some(wp) => end <= wp || end <= self.first_pos,
        }
    }

    /// Schreibt den zum Mikrofon-Frame `[mic_pos, mic_pos + out.len())` gehörenden
    /// Referenz-Frame nach `out`. Was nicht (mehr) im Ring liegt, wird mit Nullen
    /// gefüllt. Allokiert nicht.
    pub fn pull(&mut self, mic_pos: u64, out: &mut [i16]) -> Pulled {
        self.stats.pulled_frames += 1;
        let n = out.len() as i64;
        let start = mic_pos as i64 + self.offset_samples;
        let end = start + n;
        let Some(wp) = self.write_pos else {
            out.fill(0);
            return Pulled::BeforeStart;
        };
        // Gültiger Bereich [lo, hi): nach dem Start, nicht überschrieben, schon geschrieben.
        let oldest = wp - RING_SAMPLES as i64;
        let lo = start.max(self.first_pos).max(oldest);
        let hi = end.min(wp);
        if hi <= lo {
            out.fill(0);
            return if end <= self.first_pos {
                Pulled::BeforeStart
            } else if start >= wp {
                self.stats.underruns += 1;
                Pulled::Underrun
            } else {
                self.stats.overruns += 1;
                Pulled::Overrun
            };
        }
        let (lo_off, hi_off) = ((lo - start) as usize, (hi - start) as usize);
        out[..lo_off].fill(0);
        out[hi_off..].fill(0);
        let mut src = lo.rem_euclid(RING_SAMPLES as i64) as usize;
        let mut dst = lo_off;
        while dst < hi_off {
            let chunk = (RING_SAMPLES - src).min(hi_off - dst);
            out[dst..dst + chunk].copy_from_slice(&self.ring[src..src + chunk]);
            dst += chunk;
            src = (src + chunk) & RING_MASK;
        }
        if lo > start.max(self.first_pos) {
            self.stats.overruns += 1;
            Pulled::Overrun
        } else if hi < end {
            self.stats.underruns += 1;
            Pulled::Underrun
        } else {
            Pulled::Data
        }
    }

    /// Pause/Fortsetzen oder Neustart: vergisst Referenz und Verankerung (der
    /// Versatz bleibt). Danach liefert `pull` Nullen, bis ein neuer Block kommt.
    /// Der Aufrufer setzt den [`EchoCanceller`] ebenfalls zurück.
    pub fn reset(&mut self) {
        self.write_pos = None;
        self.first_pos = 0;
        self.slewing = false;
        self.stats.resets += 1;
        // Der Ring wird nicht gelöscht: `first_pos`/`write_pos` schirmen alte
        // Inhalte ab, und jeder Bereich wird beschrieben, bevor er lesbar wird.
    }

    pub fn stats(&self) -> &AlignerStats {
        &self.stats
    }

    /// Schreibt `samples` ab `write_pos`; ein Block größer als der Ring behält nur
    /// seinen Schluss (und zählt als Überlauf).
    fn write_block(&mut self, samples: &[i16]) {
        let Some(wp) = self.write_pos else { return };
        let (skip, tail) = if samples.len() > RING_SAMPLES {
            self.stats.overruns += 1;
            let skip = samples.len() - RING_SAMPLES;
            (skip, &samples[skip..])
        } else {
            (0, samples)
        };
        let start = (wp + skip as i64).rem_euclid(RING_SAMPLES as i64) as usize;
        let first = (RING_SAMPLES - start).min(tail.len());
        self.ring[start..start + first].copy_from_slice(&tail[..first]);
        self.ring[..tail.len() - first].copy_from_slice(&tail[first..]);
        self.write_pos = Some(wp + samples.len() as i64);
    }

    /// Schreibt `count`-mal `value` (Lücke füllen bzw. Sample einfügen).
    fn write_fill(&mut self, value: i16, count: usize) {
        let Some(wp) = self.write_pos else { return };
        let shown = count.min(RING_SAMPLES);
        let first_idx = wp + (count - shown) as i64;
        for k in 0..shown as i64 {
            self.ring[(first_idx + k).rem_euclid(RING_SAMPLES as i64) as usize] = value;
        }
        self.write_pos = Some(wp + count as i64);
    }
}

// ───────────────────── Sicherheitsnetz gegen Doppeltext ─────────────────

/// Anteil der Ich-Wörter, der in der Gegenseite vorkommen muss.
pub const DUPLICATE_WORD_SHARE: f32 = 0.8;
/// So viel leiser (dB) als das Systemsegment muss das Ich-Segment mindestens sein.
pub const DUPLICATE_MIN_RMS_DB: f32 = 15.0;

/// Sicherheitsnetz (Konzept §3.3): Ein Ich-Segment ist ein durchgerutschtes Echo
/// und wird verworfen, wenn
///
/// * mindestens 80 % seiner Wörter in den Wörtern des Gegenseite-Segments
///   (±2 s um das Ich-Segment, vom Aufrufer gewählt) vorkommen **und**
/// * sein RMS mindestens 15 dB unter dem des Systemsegments liegt.
///
/// `rms_db_diff` = Pegel Gegenseite − Pegel Ich in dB (positiv = Ich leiser),
/// z. B. `rms_dbfs(system) - rms_dbfs(ich)`.
///
/// Wörter werden wie in `selftest::normalize_word` normalisiert (Kleinschreibung,
/// Satzzeichen, Umlaute). Jedes Gegenseite-Wort deckt nur ein Ich-Wort ab, damit
/// „ja ja ja ja" nicht durch ein einzelnes „ja" als Echo gilt. Leere Listen,
/// nicht endliche Pegel: `false` (im Zweifel nichts verwerfen).
pub fn should_drop_duplicate<A: AsRef<str>, B: AsRef<str>>(
    ich_words: &[A],
    gegen_words: &[B],
    rms_db_diff: f32,
) -> bool {
    if !rms_db_diff.is_finite() || rms_db_diff < DUPLICATE_MIN_RMS_DB {
        return false;
    }
    let mut available: HashMap<String, usize> = HashMap::new();
    for w in gegen_words {
        let n = normalize_word(w.as_ref());
        if !n.is_empty() {
            *available.entry(n).or_insert(0) += 1;
        }
    }
    if available.is_empty() {
        return false;
    }
    let (mut total, mut matched) = (0usize, 0usize);
    for w in ich_words {
        let n = normalize_word(w.as_ref());
        if n.is_empty() {
            continue;
        }
        total += 1;
        if let Some(c) = available.get_mut(&n) {
            if *c > 0 {
                *c -= 1;
                matched += 1;
            }
        }
    }
    total > 0 && matched as f32 >= DUPLICATE_WORD_SHARE * total as f32 - 1e-6
}

// ──────────────────── Test-Hilfe: Allokationen zählen ───────────────────

/// Zählt Allokationen des aktuellen Threads, solange `count_allocs` läuft. Gilt
/// für das ganze Test-Binary (es gibt nur einen `#[global_allocator]`); andere
/// Module, die Allokationsfreiheit prüfen wollen (z. B. der DSP-Thread), nutzen
/// `crate::managers::meetings::echo::alloc_probe::count_allocs`.
#[cfg(test)]
pub(crate) mod alloc_probe {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static ARMED: Cell<bool> = const { Cell::new(false) };
        static ALLOCS: Cell<u64> = const { Cell::new(0) };
        static BYTES: Cell<u64> = const { Cell::new(0) };
    }

    fn note(bytes: usize) {
        let _ = ARMED.try_with(|armed| {
            if armed.get() {
                let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
                let _ = BYTES.try_with(|c| c.set(c.get() + bytes as u64));
            }
        });
    }

    struct Counting;

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            note(layout.size());
            System.alloc(layout)
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            note(layout.size());
            System.alloc_zeroed(layout)
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout)
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            note(new_size);
            System.realloc(ptr, layout, new_size)
        }
    }

    #[global_allocator]
    static COUNTING: Counting = Counting;

    /// `(Ergebnis, Anzahl Allokationen, angeforderte Bytes)` von `f` auf diesem Thread.
    pub(crate) fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, u64, u64) {
        ALLOCS.with(|c| c.set(0));
        BYTES.with(|c| c.set(0));
        ARMED.with(|a| a.set(true));
        let r = f();
        ARMED.with(|a| a.set(false));
        (r, ALLOCS.with(Cell::get), BYTES.with(Cell::get))
    }
}

// ───────────────────────────────── Tests ────────────────────────────────

#[cfg(test)]
mod tests {
    use super::alloc_probe::count_allocs;
    use super::*;
    use std::path::PathBuf;
    use std::time::Instant;

    // ── Zeitplan der Fixtures (Sekunden), siehe scripts/make-m2-fixtures.py ──
    /// Nur die Gegenseite spricht, ab 5 s Einschwingzeit.
    const REGION_FAR_ONLY: (f64, f64) = (6.0, 9.8);
    /// Nur der Nahsprecher (Ich) spricht.
    const REGION_NEAR_ONLY: (f64, f64) = (11.5, 13.9);
    /// Beide sprechen.
    const REGION_DOUBLE_TALK: (f64, f64) = (16.0, 18.5);

    fn fixture(name: &str) -> Vec<i16> {
        let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
            .iter()
            .collect();
        let mut reader = hound::WavReader::open(&path).unwrap_or_else(|e| {
            panic!(
                "Fixture {name} fehlt ({e}). Erzeugen: pwsh apps/local-voice/scripts/make-m2-fixtures.ps1"
            )
        });
        let spec = reader.spec();
        assert_eq!(
            (spec.sample_rate, spec.channels, spec.bits_per_sample),
            (16_000, 1, 16),
            "{name}: erwartet 16 kHz mono PCM16"
        );
        reader.samples::<i16>().map(|s| s.unwrap()).collect()
    }

    fn region(x: &[i16], r: (f64, f64)) -> &[i16] {
        &x[(r.0 * 16_000.0) as usize..(r.1 * 16_000.0) as usize]
    }

    struct FixtureRun {
        mic: Vec<i16>,
        render: Vec<i16>,
        out: Vec<i16>,
        frame_ns: Vec<u64>,
        stats: EchoStats,
    }

    fn run_canceller(mic: &[i16], render: &[i16]) -> FixtureRun {
        let frames = mic.len().min(render.len()) / FRAME_SAMPLES;
        let mut ec = EchoCanceller::new();
        let mut out = vec![0i16; frames * FRAME_SAMPLES];
        let mut frame_ns = Vec::with_capacity(frames);
        for f in 0..frames {
            let r = f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES;
            let t = Instant::now();
            ec.process(&mic[r.clone()], &render[r.clone()], &mut out[r])
                .unwrap();
            frame_ns.push(t.elapsed().as_nanos() as u64);
        }
        FixtureRun {
            mic: mic[..frames * FRAME_SAMPLES].to_vec(),
            render: render[..frames * FRAME_SAMPLES].to_vec(),
            out,
            frame_ns,
            stats: ec.stats(),
        }
    }

    fn percentile(sorted: &[u64], p: f64) -> u64 {
        sorted[((sorted.len() - 1) as f64 * p).round() as usize]
    }

    // ───────────────────────────── EchoCanceller ─────────────────────────

    #[test]
    fn aec_on_fixture_reaches_erle_keeps_near_speech_and_logs_frame_time() {
        let run = run_canceller(&fixture("m2_echo_mic.wav"), &fixture("m2_echo_render.wav"));

        // Der Zeitplan im Test muss zum Fixture passen (sonst wurde neu erzeugt).
        assert!(
            rms_dbfs(region(&run.render, REGION_FAR_ONLY)) > -40.0,
            "Gegenseite muss im Far-only-Fenster sprechen"
        );
        assert!(
            rms_dbfs(region(&run.render, REGION_NEAR_ONLY)) < -90.0,
            "Referenz muss im Near-only-Fenster still sein"
        );
        assert!(
            rms_dbfs(region(&run.mic, REGION_NEAR_ONLY)) > -35.0,
            "Nahsprecher muss im Near-only-Fenster sprechen"
        );
        assert!(
            rms_dbfs(region(&run.render, REGION_DOUBLE_TALK)) > -40.0
                && rms_dbfs(region(&run.mic, REGION_DOUBLE_TALK)) > -30.0,
            "beide müssen im Doppelsprech-Fenster sprechen"
        );
        // Der Nahsprecher schweigt bis 11 s; in den letzten Zehntelsekunden vor
        // 5 s ist auch das Echo abgeklungen: nur Rauschen bleibt.
        assert!(
            rms_dbfs(region(&run.mic, (4.7, 5.4))) < -50.0,
            "Nahsprecher muss in den ersten 5 s schweigen"
        );
        // ERLE nach 5 s Einschwingzeit, nur die Gegenseite spricht.
        let erle = rms_dbfs(region(&run.mic, REGION_FAR_ONLY))
            - rms_dbfs(region(&run.out, REGION_FAR_ONLY));
        // Nahsprache: Pegel nach der AEC gegen den Pegel davor.
        let near_delta = rms_dbfs(region(&run.out, REGION_NEAR_ONLY))
            - rms_dbfs(region(&run.mic, REGION_NEAR_ONLY));
        let dt_delta = rms_dbfs(region(&run.out, REGION_DOUBLE_TALK))
            - rms_dbfs(region(&run.mic, REGION_DOUBLE_TALK));

        let mut sorted = run.frame_ns.clone();
        sorted.sort_unstable();
        let (p50, p99, max) = (
            percentile(&sorted, 0.50),
            percentile(&sorted, 0.99),
            *sorted.last().unwrap(),
        );
        println!(
            "echo-messung: erle_far_only={erle:.1} dB near_pegel_delta={near_delta:.1} dB \
             doppelsprechen_pegel_delta={dt_delta:.1} dB | frame_zeit_us p50={:.0} p99={:.0} max={:.0} \
             ({} Frames) | aec3-stats: erl={:?} erle={:?} delay_ms={:?} errors={} panics={}",
            p50 as f64 / 1000.0,
            p99 as f64 / 1000.0,
            max as f64 / 1000.0,
            sorted.len(),
            run.stats.erl_db,
            run.stats.erle_db,
            run.stats.delay_ms,
            run.stats.errors,
            run.stats.panics,
        );

        assert!(erle >= 20.0, "ERLE {erle:.1} dB < 20 dB");
        assert!(
            near_delta >= -3.0,
            "Nahsprache {near_delta:.1} dB leiser, erlaubt sind höchstens 3 dB"
        );
        assert_eq!((run.stats.errors, run.stats.panics), (0, 0));
        // Die Fixture hat 50 ms Laufzeit + Hall; AEC3 muss das grob finden.
        let delay = run.stats.delay_ms.expect("AEC3 liefert eine Verzögerung");
        assert!((20..=120).contains(&delay), "Verzögerung {delay} ms");
    }

    #[test]
    fn without_a_reference_the_microphone_passes_almost_unchanged() {
        // Headset / kein Loopback: Referenz ist Stille. Nahsprache darf kaum leiden.
        let mic = fixture("m2_echo_mic.wav");
        let silence = vec![0i16; mic.len()];
        let run = run_canceller(&mic, &silence);
        let delta = rms_dbfs(region(&run.out, REGION_NEAR_ONLY))
            - rms_dbfs(region(&run.mic, REGION_NEAR_ONLY));
        assert!(
            delta >= -1.5,
            "Nahsprache ohne Referenz {delta:.1} dB leiser"
        );
        // Gegenprobe zur ERLE-Messung: ohne Referenz gibt es nichts zu löschen.
        let erle = rms_dbfs(region(&run.mic, REGION_FAR_ONLY))
            - rms_dbfs(region(&run.out, REGION_FAR_ONLY));
        println!("ohne Referenz: erle={erle:.1} dB near_delta={delta:.1} dB");
        assert!(
            erle < 6.0,
            "ohne Referenz dürfte die AEC nicht dämpfen: {erle:.1} dB"
        );
        assert_eq!((run.stats.errors, run.stats.panics), (0, 0));
    }

    /// Aligner + AEC wie im DSP-Thread: Der Loopback beginnt `loop_start_ms` nach
    /// dem Mikrofon und liefert 30-ms-Blöcke; der Mikrofon-Frame m wird 40 ms nach
    /// seinem Ende verarbeitet. `offset_used_ms` ist der dem Aligner mitgeteilte Versatz.
    fn run_pipeline(
        mic: &[i16],
        render: &[i16],
        loop_start_ms: f64,
        offset_used_ms: f64,
    ) -> Vec<i16> {
        const BLOCK: usize = 480;
        let reference = &render[(loop_start_ms * 16.0) as usize..];
        let mut al = Aligner::new();
        al.set_offset_ms(offset_used_ms);
        let mut ec = EchoCanceller::new();
        let frames = mic.len() / FRAME_SAMPLES;
        let mut out = vec![0i16; frames * FRAME_SAMPLES];
        let mut ref_frame = [0i16; FRAME_SAMPLES];
        let mut n_next = 0usize;
        for m in 0..frames {
            let wall_ms = (m as f64 + 1.0) * 10.0 + 40.0;
            let avail = ((wall_ms - loop_start_ms).max(0.0) * 16.0) as usize;
            while n_next + BLOCK <= avail.min(reference.len()) {
                if let AlignEvent::Reset { .. } =
                    al.push_reference(n_next as f64 / 16.0, &reference[n_next..n_next + BLOCK])
                {
                    ec.reset();
                }
                n_next += BLOCK;
            }
            let pos = (m * FRAME_SAMPLES) as u64;
            if al.ready_for(pos, FRAME_SAMPLES) {
                al.pull(pos, &mut ref_frame);
            } else {
                ref_frame.fill(0);
            }
            let r = m * FRAME_SAMPLES..(m + 1) * FRAME_SAMPLES;
            ec.process(&mic[r.clone()], &ref_frame, &mut out[r])
                .unwrap();
        }
        out
    }

    #[test]
    fn aligned_reference_from_a_late_loopback_still_cancels_the_echo() {
        // Loopback startet 1,2 s nach dem Mikrofon: offset = -1200 ms.
        let mic = fixture("m2_echo_mic.wav");
        let render = fixture("m2_echo_render.wav");
        let out = run_pipeline(&mic, &render, 1_200.0, -1_200.0);
        let erle =
            rms_dbfs(region(&mic, REGION_FAR_ONLY)) - rms_dbfs(region(&out, REGION_FAR_ONLY));
        let near =
            rms_dbfs(region(&out, REGION_NEAR_ONLY)) - rms_dbfs(region(&mic, REGION_NEAR_ONLY));
        println!("pipeline (Loopback +1200 ms, Versatz bekannt): erle={erle:.1} dB near_delta={near:.1} dB");
        assert!(erle >= 20.0, "ERLE {erle:.1} dB");
        assert!(near >= -3.0, "Nahsprache {near:.1} dB");
    }

    #[test]
    fn ignoring_the_offset_leaves_the_echo_in_place() {
        // Gegenprobe: Referenz ohne Zeitachse (erster Loopback-Puffer = Mikrofon-
        // Beginn) liegt 1,2 s vor dem Echo; AEC3 kann das nicht löschen.
        let mic = fixture("m2_echo_mic.wav");
        let render = fixture("m2_echo_render.wav");
        let run = run_canceller(&mic, &render[(1.2 * 16_000.0) as usize..]);
        let erle = rms_dbfs(region(&run.mic, REGION_FAR_ONLY))
            - rms_dbfs(region(&run.out, REGION_FAR_ONLY));
        println!("ohne Zeitachse (Referenz 1200 ms zu früh): erle={erle:.1} dB");
        assert!(
            erle < 10.0,
            "ERLE {erle:.1} dB: die Zeitachse wäre überflüssig"
        );
    }

    #[test]
    fn frames_of_the_wrong_length_are_rejected_and_leave_out_untouched() {
        let mut ec = EchoCanceller::new();
        let ok = [0i16; FRAME_SAMPLES];
        let mut out = [7i16; FRAME_SAMPLES];
        for (m, r, o) in [(159, 160, 160), (160, 161, 160), (160, 160, 480)] {
            let mic = vec![0i16; m];
            let reference = vec![0i16; r];
            let mut out_v = vec![7i16; o];
            assert_eq!(
                ec.process(&mic, &reference, &mut out_v),
                Err(EchoError::FrameLength {
                    mic: m,
                    reference: r,
                    out: o
                })
            );
            assert!(out_v.iter().all(|&s| s == 7));
        }
        assert!(ec.process(&ok, &ok, &mut out).is_ok());
        assert_eq!(ec.stats().frames, 1, "abgelehnte Frames zählen nicht");
    }

    #[test]
    fn reset_forgets_what_was_learned_and_keeps_processing() {
        let mic = fixture("m2_echo_mic.wav");
        let render = fixture("m2_echo_render.wav");
        let mut ec = EchoCanceller::new();
        let mut out = [0i16; FRAME_SAMPLES];
        for f in 0..300 {
            let r = f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES;
            ec.process(&mic[r.clone()], &render[r], &mut out).unwrap();
        }
        assert!(ec.stats().delay_ms.is_some() || ec.stats().erle_db.is_some());
        ec.reset();
        let s = ec.stats();
        assert_eq!((s.resets, s.frames), (1, 300));
        assert_eq!((s.delay_ms, s.erle_db), (None, None), "AEC3 ist frisch");
        let r = 300 * FRAME_SAMPLES..301 * FRAME_SAMPLES;
        assert!(ec.process(&mic[r.clone()], &render[r], &mut out).is_ok());
    }

    #[test]
    fn a_panic_inside_aec3_is_contained() {
        assert_eq!(guarded(|| 5), Some(5));
        assert_eq!(
            guarded(|| -> u8 { panic!("simulierter AEC3-Fehler") }),
            None
        );
        // Nach einem Panic (failed) reicht process() das Mikrofon durch, bis reset().
        let mut ec = EchoCanceller::new();
        ec.failed = true;
        let mic = [123i16; FRAME_SAMPLES];
        let mut out = [0i16; FRAME_SAMPLES];
        ec.process(&mic, &[0; FRAME_SAMPLES], &mut out).unwrap();
        assert_eq!(out, mic);
        ec.reset();
        assert!(!ec.failed);
    }

    #[test]
    fn types_are_send_for_the_dsp_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<EchoCanceller>();
        assert_send::<Aligner>();
        // und laufen tatsächlich auf einem fremden Thread
        let handle = std::thread::spawn(|| {
            let mut ec = EchoCanceller::new();
            let mut al = Aligner::new();
            let mut out = [0i16; FRAME_SAMPLES];
            let _ = al.push_reference(0.0, &[1i16; 320]);
            let mut r = [0i16; FRAME_SAMPLES];
            al.pull(0, &mut r);
            ec.process(&[0; FRAME_SAMPLES], &r, &mut out).unwrap();
            ec.stats().frames
        });
        assert_eq!(handle.join().unwrap(), 1);
    }

    #[test]
    fn no_allocation_per_frame_in_steady_state() {
        let mic = fixture("m2_echo_mic.wav");
        let render = fixture("m2_echo_render.wav");
        let (mut ec, _, build_bytes) = count_allocs(EchoCanceller::new);
        let (mut al, _, ring_bytes) = count_allocs(Aligner::new);
        let mut raw = build_apm(); // dieselben sonora-Aufrufe ohne unsere Hülle
        let mut out = [0i16; FRAME_SAMPLES];
        let mut reference = [0i16; FRAME_SAMPLES];
        let frame = |i: usize| i * FRAME_SAMPLES..(i + 1) * FRAME_SAMPLES;

        // Einschwingen, dann messen. Jeder Pfad bekommt dieselben Eingänge.
        let run_raw = |apm: &mut AudioProcessing, i: usize| {
            let (mut r_in, mut m_in) = ([0f32; FRAME_SAMPLES], [0f32; FRAME_SAMPLES]);
            for k in 0..FRAME_SAMPLES {
                r_in[k] = f32::from(render[frame(i)][k]) / 32768.0;
                m_in[k] = f32::from(mic[frame(i)][k]) / 32768.0;
            }
            let (mut r_out, mut m_out) = ([0f32; FRAME_SAMPLES], [0f32; FRAME_SAMPLES]);
            apm.process_render_f32(&[&r_in[..]], &mut [&mut r_out[..]])
                .unwrap();
            apm.process_capture_f32(&[&m_in[..]], &mut [&mut m_out[..]])
                .unwrap();
        };
        for i in 0..200 {
            ec.process(&mic[frame(i)], &render[frame(i)], &mut out)
                .unwrap();
            run_raw(&mut raw, i);
        }
        let ((), wrapper_allocs, _) = count_allocs(|| {
            for i in 200..700 {
                ec.process(&mic[frame(i)], &render[frame(i)], &mut out)
                    .unwrap();
            }
        });
        let ((), sonora_allocs, _) = count_allocs(|| {
            for i in 200..700 {
                run_raw(&mut raw, i);
            }
        });
        // Aufteilung der sonora-Allokationen (nur zur Auskunft im Log).
        let (mut r_in, mut m_in) = ([0f32; FRAME_SAMPLES], [0f32; FRAME_SAMPLES]);
        let (mut r_out, mut m_out) = ([0f32; FRAME_SAMPLES], [0f32; FRAME_SAMPLES]);
        for k in 0..FRAME_SAMPLES {
            r_in[k] = f32::from(render[frame(300)][k]) / 32768.0;
            m_in[k] = f32::from(mic[frame(300)][k]) / 32768.0;
        }
        let ((), render_allocs, _) = count_allocs(|| {
            for _ in 0..500 {
                raw.process_render_f32(&[&r_in[..]], &mut [&mut r_out[..]])
                    .unwrap();
            }
        });
        let ((), capture_allocs, _) = count_allocs(|| {
            for _ in 0..500 {
                raw.process_capture_f32(&[&m_in[..]], &mut [&mut m_out[..]])
                    .unwrap();
            }
        });
        // Aligner allein: Block schieben + Frame holen, 500 Frames.
        let ((), aligner_allocs, _) = count_allocs(|| {
            for i in 0..500 {
                let _ = al.push_reference(i as f64 * 10.0, &render[frame(i)]);
                al.pull(i as u64 * 160, &mut reference);
            }
        });
        println!(
            "alloc-messung: Aufbau EchoCanceller={} KB, Aligner-Ring={} KB | je 500 Frames: \
             Aligner={aligner_allocs} Allokationen, EchoCanceller.process={wrapper_allocs}, \
             rohe sonora-Aufrufe={sonora_allocs} (= {:.1} je Frame;              davon nur render={render_allocs}, nur capture={capture_allocs} je 500 Aufrufe)",
            build_bytes / 1024,
            ring_bytes / 1024,
            sonora_allocs as f64 / 500.0
        );
        assert_eq!(aligner_allocs, 0, "Aligner allokiert je Frame");
        assert_eq!(
            wrapper_allocs, sonora_allocs,
            "die Hülle darf über sonora hinaus nichts allokieren"
        );
        // Grenze gegen versehentliches Wachstum (sonora selbst allokiert intern, siehe Report).
        assert!(
            sonora_allocs / 500 <= 64,
            "sonora: {} Allokationen je Frame",
            sonora_allocs / 500
        );
    }

    // ────────────────────────────── Aligner ──────────────────────────────

    /// Kennzeichnet ein Referenz-Sample mit seinem „wahren" Index auf der Mikrofon-
    /// Achse als Sägezahn (Periode 2,048 s); so lässt sich aus dem, was der Aligner
    /// liefert, ablesen, welchen Zeitpunkt er getroffen hat. 0 bleibt für „Stille".
    fn tag(true_idx: i64) -> i16 {
        (true_idx.rem_euclid(32_768) - 16_384).max(-16_383) as i16 | 1 // nie 0
    }

    /// Abweichung (in Samples) des gelieferten Samples vom gewünschten wahren Index.
    fn tag_error(value: i16, wanted_true_idx: i64) -> i64 {
        let diff = (i64::from(value) - i64::from(tag(wanted_true_idx))).rem_euclid(32_768);
        if diff >= 16_384 {
            diff - 32_768
        } else {
            diff
        }
    }

    #[derive(Debug, Default)]
    struct DriftRun {
        max_err: i64,
        frames_checked: u64,
        compensations: u32,
        max_step: i32,
        resets: u32,
        stalls: u32,
        zero_frames_before_start: u32,
    }

    /// Simuliert einen Loopback, der `loop_start_ms` nach dem Mikrofon beginnt und
    /// dessen Geräteuhr um `eps` (relativ, +150e-6 = 150 ppm zu schnell) von der
    /// QPC-Zeit abweicht. Zeitstempel = QPC (wahr), Samples = Geräteuhr.
    fn run_drift(eps: f64, seconds: f64, loop_start_ms: f64, ref_block: usize) -> DriftRun {
        let mut al = Aligner::new();
        al.set_offset_ms(-loop_start_ms);
        let l_samples = (loop_start_ms * 16.0).round() as i64;
        let mut res = DriftRun::default();
        let mut n_next = 0i64;
        let mut out = [0i16; FRAME_SAMPLES];
        for m in 0..(seconds * 100.0) as u64 {
            // Der DSP-Thread bearbeitet den Mikrofon-Frame m 40 ms nach dessen Ende.
            let wall_ms = (m as f64 + 1.0) * 10.0 + 40.0;
            let avail = ((wall_ms - loop_start_ms).max(0.0) * 16.0 * (1.0 + eps)) as i64;
            while n_next + ref_block as i64 <= avail {
                let block: Vec<i16> = (0..ref_block as i64)
                    .map(|i| tag(l_samples + ((n_next + i) as f64 / (1.0 + eps)).round() as i64))
                    .collect();
                let t_ref_ms = n_next as f64 / (16.0 * (1.0 + eps));
                match al.push_reference(t_ref_ms, &block) {
                    AlignEvent::None => {}
                    AlignEvent::Compensated { samples } => {
                        res.compensations += 1;
                        res.max_step = res.max_step.max(samples.abs());
                    }
                    AlignEvent::Reset { .. } => res.resets += 1,
                }
                n_next += ref_block as i64;
            }
            let mic_pos = m * FRAME_SAMPLES as u64;
            if !al.ready_for(mic_pos, FRAME_SAMPLES) {
                res.stalls += 1;
                continue;
            }
            al.pull(mic_pos, &mut out);
            let first_true = mic_pos as i64;
            if first_true + FRAME_SAMPLES as i64 <= l_samples {
                assert!(
                    out.iter().all(|&s| s == 0),
                    "vor Beginn der Referenz: Nullen"
                );
                res.zero_frames_before_start += 1;
                continue;
            }
            if first_true < l_samples {
                continue; // Frame überspannt den Beginn der Referenz
            }
            for (j, &v) in out.iter().enumerate() {
                res.max_err = res.max_err.max(tag_error(v, first_true + j as i64).abs());
            }
            res.frames_checked += 1;
        }
        res
    }

    #[test]
    fn positive_offset_skips_the_start_of_the_reference() {
        // Loopback lief schon 100 ms vor dem Mikrofon: offset = +100 ms.
        let mut al = Aligner::new();
        al.set_offset_ms(100.0);
        let block: Vec<i16> = (0..4800).map(|i| (i + 1000) as i16).collect(); // 300 ms
        assert_eq!(al.push_reference(0.0, &block), AlignEvent::None);
        let mut out = [0i16; FRAME_SAMPLES];
        assert_eq!(al.pull(0, &mut out), Pulled::Data);
        // Mikrofon-Sample 0 gehört zu Referenz-Sample 1600.
        assert_eq!(out[0], 1000 + 1600);
        assert_eq!(out[159], 1000 + 1600 + 159);
        assert_eq!(al.pull(160, &mut out), Pulled::Data);
        assert_eq!(out[0], 1000 + 1760);
    }

    #[test]
    fn negative_offset_pads_zeros_until_the_reference_begins() {
        // Loopback startete 250 ms nach dem Mikrofon: offset = -250 ms = 4000 Samples.
        let mut al = Aligner::new();
        al.set_offset_ms(-250.0);
        let block: Vec<i16> = (0..1600).map(|i| (i + 1000) as i16).collect();
        assert_eq!(al.push_reference(0.0, &block), AlignEvent::None);
        let mut out = [9i16; FRAME_SAMPLES];
        for frame in 0..25u64 {
            assert!(al.ready_for(frame * 160, 160));
            assert_eq!(al.pull(frame * 160, &mut out), Pulled::BeforeStart);
            assert!(out.iter().all(|&s| s == 0), "Frame {frame} muss still sein");
        }
        assert_eq!(al.pull(25 * 160, &mut out), Pulled::Data);
        assert_eq!(&out[..3], &[1000, 1001, 1002]);
        // Beginn mitten im Frame: offset -255 ms -> 80 Nullen, dann Referenz.
        let mut al = Aligner::new();
        al.set_offset_ms(-255.0);
        let _ = al.push_reference(0.0, &block);
        assert_eq!(al.pull(25 * 160, &mut out), Pulled::Data);
        assert!(out[..80].iter().all(|&s| s == 0));
        assert_eq!(&out[80..83], &[1000, 1001, 1002]);
    }

    #[test]
    fn a_reference_that_never_arrives_gives_silence_and_is_not_ready() {
        let mut al = Aligner::new();
        let mut out = [5i16; FRAME_SAMPLES];
        assert!(
            !al.ready_for(0, 160),
            "ohne ersten Block: Aufrufer entscheidet"
        );
        assert_eq!(al.pull(0, &mut out), Pulled::BeforeStart);
        assert!(out.iter().all(|&s| s == 0));
        assert_eq!(
            al.stats().underruns,
            0,
            "fehlender Loopback ist kein Underrun"
        );
    }

    #[test]
    fn drift_of_150_ppm_over_60_s_needs_no_reset_and_stays_aligned() {
        for eps in [150e-6, -150e-6] {
            let r = run_drift(eps, 60.0, 1_500.0, 320);
            println!("drift eps={eps:+e} über 60 s: {r:?}");
            assert_eq!(r.resets, 0, "kein Reset bei {eps:+e}");
            assert_eq!(
                r.compensations, 0,
                "9 ms Drift liegen unter der 20-ms-Schwelle"
            );
            assert_eq!(r.stalls, 0);
            assert!(r.frames_checked > 5_000);
            assert!(
                r.max_err <= DRIFT_TOLERANCE_MS * 16,
                "Abweichung {} Samples > 20 ms",
                r.max_err
            );
        }
    }

    #[test]
    fn drift_beyond_20_ms_is_compensated_in_single_sample_steps_without_reset() {
        // 600 ppm über 120 s = 72 ms, in beide Richtungen.
        for eps in [600e-6, -600e-6] {
            let r = run_drift(eps, 120.0, 800.0, 320);
            println!("drift eps={eps:+e} über 120 s: {r:?}");
            assert_eq!(r.resets, 0);
            assert!(r.compensations > 0, "Nachstellen hat nie gegriffen");
            assert!(
                i64::from(r.max_step) <= (320 / SLEW_DIVISOR) as i64,
                "Schritt {} > {} Samples je Block",
                r.max_step,
                320 / SLEW_DIVISOR
            );
            assert!(
                r.max_err <= DRIFT_TOLERANCE_MS * 16 + 16,
                "Abweichung {} Samples bleibt über 20 ms",
                r.max_err
            );
        }
    }

    #[test]
    fn a_jump_of_300_ms_is_realigned_at_once() {
        let mut al = Aligner::new();
        let mut n = 0i64;
        let mut push = |al: &mut Aligner, extra_ms: f64| {
            let block: Vec<i16> = (0..320)
                .map(|i| tag(n + i + (extra_ms * 16.0) as i64))
                .collect();
            let ev = al.push_reference(n as f64 / 16.0 + extra_ms, &block);
            n += 320;
            ev
        };
        for _ in 0..100 {
            assert_eq!(push(&mut al, 0.0), AlignEvent::None);
        }
        // Loopback verlor 300 ms: Zeitstempel springt vor, Samples laufen nahtlos weiter.
        let ev = push(&mut al, 300.0);
        assert_eq!(ev, AlignEvent::Compensated { samples: 4800 });
        assert_eq!(al.stats().jumps, 1);
        // Der nächste Block liegt wieder auf der Achse: Mikrofon-Frame am Anfang
        // des gesprungenen Blocks bekommt genau dessen Anfang.
        let pos = 32_000 + 4_800; // Achsen-Index des gesprungenen Blocks
        let mut out = [0i16; FRAME_SAMPLES];
        assert_eq!(al.pull(pos as u64, &mut out), Pulled::Data);
        assert_eq!(tag_error(out[0], pos), 0, "Block liegt an seiner Zeit");
        // Die 300 ms Lücke davor sind Stille, nicht verschobene Referenz.
        assert_eq!(al.pull(33_000, &mut out), Pulled::Data);
        assert!(out.iter().all(|&s| s == 0));
    }

    #[test]
    fn a_jump_over_500_ms_asks_for_a_reset_and_reanchors() {
        let mut al = Aligner::new();
        let block = [1i16; 320];
        for k in 0..50 {
            assert_eq!(al.push_reference(k as f64 * 20.0, &block), AlignEvent::None);
        }
        let ev = al.push_reference(50.0 * 20.0 + 700.0, &block);
        assert_eq!(ev, AlignEvent::Reset { drift_ms: 700 });
        assert_eq!(al.stats().resets, 1);
        // Verankert am neuen Zeitstempel: davor Nullen, ab da die neue Referenz.
        let mut out = [9i16; FRAME_SAMPLES];
        let new_start = ((50.0 * 20.0 + 700.0) * 16.0) as u64;
        assert_eq!(al.pull(new_start - 160, &mut out), Pulled::BeforeStart);
        assert!(out.iter().all(|&s| s == 0));
        assert_eq!(al.pull(new_start, &mut out), Pulled::Data);
        assert!(out.iter().all(|&s| s == 1));
        // Rückwärtssprung > 500 ms ebenso.
        let ev = al.push_reference(0.0, &block);
        assert!(matches!(ev, AlignEvent::Reset { drift_ms } if drift_ms < -500));
    }

    #[test]
    fn a_reset_for_pause_forgets_the_reference_and_reanchors_on_the_next_block() {
        let mut al = Aligner::new();
        al.set_offset_ms(-100.0);
        let _ = al.push_reference(0.0, &[7i16; 3200]);
        let mut out = [0i16; FRAME_SAMPLES];
        assert_eq!(al.pull(1600, &mut out), Pulled::Data);
        assert!(out.iter().all(|&s| s == 7));
        al.reset(); // Pause
        assert!(!al.ready_for(1600, 160));
        assert_eq!(al.pull(1600, &mut out), Pulled::BeforeStart);
        assert!(
            out.iter().all(|&s| s == 0),
            "nichts aus der Zeit vor der Pause"
        );
        // Nach dem Fortsetzen: neue Verankerung, Versatz bleibt.
        assert_eq!(al.push_reference(5_000.0, &[3i16; 1600]), AlignEvent::None);
        assert_eq!(al.pull(5_000 * 16 + 1600, &mut out), Pulled::Data);
        assert!(out.iter().all(|&s| s == 3));
        assert_eq!(al.stats().resets, 1);
    }

    #[test]
    fn ring_overrun_and_late_reference_are_counted_not_swallowed() {
        let mut al = Aligner::new();
        // 3 s Referenz auf einmal: ein Block größer als der Ring (2,048 s).
        let big = vec![4i16; 48_000];
        let _ = al.push_reference(0.0, &big);
        assert_eq!(al.stats().overruns, 1, "Block größer als der Ring");
        let mut out = [0i16; FRAME_SAMPLES];
        // Anfang ist schon überschrieben.
        assert_eq!(al.pull(0, &mut out), Pulled::Overrun);
        assert!(out.iter().all(|&s| s == 0));
        assert_eq!(al.stats().overruns, 2);
        // Das Neueste ist da.
        assert_eq!(al.pull(47_000, &mut out), Pulled::Data);
        // Frame teilweise über das Neueste hinaus: Underrun, Rest Nullen.
        assert_eq!(al.pull(47_900, &mut out), Pulled::Underrun);
        assert!(out[..100].iter().all(|&s| s == 4) && out[100..].iter().all(|&s| s == 0));
        assert!(!al.ready_for(47_900, 160));
        assert!(al.ready_for(47_000, 160));
        assert_eq!(al.stats().underruns, 1);
        // Weit voraus: vollständig zu spät.
        assert_eq!(al.pull(60_000, &mut out), Pulled::Underrun);
        assert_eq!(al.stats().underruns, 2);
    }

    #[test]
    fn ring_wraps_correctly_over_many_seconds() {
        let mut al = Aligner::new();
        let mut out = [0i16; FRAME_SAMPLES];
        for k in 0..2_000i64 {
            // 20-s-Lauf in 10-ms-Blöcken; Wert = Blockindex
            let _ = al.push_reference(k as f64 * 10.0, &[(k % 30_000) as i16; 160]);
            assert_eq!(al.pull(k as u64 * 160, &mut out), Pulled::Data);
            assert!(out.iter().all(|&s| s == (k % 30_000) as i16), "Block {k}");
        }
        assert_eq!(al.stats().overruns, 0);
    }

    #[test]
    fn bad_timestamps_do_not_corrupt_the_stream() {
        let mut al = Aligner::new();
        let block: Vec<i16> = (0..160).map(|i| i as i16 + 1).collect();
        assert_eq!(al.push_reference(0.0, &block), AlignEvent::None);
        assert_eq!(al.push_reference(f64::NAN, &block), AlignEvent::None);
        assert_eq!(al.push_reference(f64::INFINITY, &block), AlignEvent::None);
        assert_eq!(al.stats().bad_timestamps, 2);
        let mut out = [0i16; FRAME_SAMPLES];
        assert_eq!(
            al.pull(160, &mut out),
            Pulled::Data,
            "nach Sample-Zahl angehängt"
        );
        assert_eq!(out[0], 1);
        al.set_offset_ms(f64::NAN); // ignoriert
        assert_eq!(al.pull(320, &mut out), Pulled::Data);
        // leerer Block ist ein No-op
        assert_eq!(al.push_reference(1.0, &[]), AlignEvent::None);
    }

    // ────────────────────── Sicherheitsnetz gegen Doppeltext ─────────────

    fn words(s: &str) -> Vec<&str> {
        s.split_whitespace().collect()
    }

    #[test]
    fn a_quiet_copy_of_the_far_end_is_dropped() {
        let gegen =
            words("Bitte schicken Sie mir die aktualisierte Kalkulation bis Donnerstagabend.");
        let ich = words("bitte schicken sie mir die aktualisierte kalkulation bis donnerstagabend");
        assert!(should_drop_duplicate(&ich, &gegen, 20.0));
        assert!(
            should_drop_duplicate(&ich, &gegen, 15.0),
            "Grenze 15 dB zählt"
        );
    }

    #[test]
    fn near_speech_that_is_loud_is_kept_even_if_the_words_match() {
        // Wortgleich, aber der Nutzer spricht selbst (kaum leiser als der Systemton).
        let gegen = words("Wir treffen uns am Montag um zehn");
        let ich = words("Wir treffen uns am Montag um zehn");
        assert!(!should_drop_duplicate(&ich, &gegen, 14.9));
        assert!(!should_drop_duplicate(&ich, &gegen, 0.0));
        assert!(
            !should_drop_duplicate(&ich, &gegen, -6.0),
            "Ich lauter: nie verwerfen"
        );
    }

    #[test]
    fn different_words_are_kept_even_when_quiet() {
        let gegen = words("Die Ausschreibung endet am dritten November");
        let ich = words("Ich übernehme die Abstimmung mit der Rechtsabteilung");
        assert!(!should_drop_duplicate(&ich, &gegen, 30.0));
    }

    #[test]
    fn eighty_percent_is_the_threshold() {
        let gegen = words("eins zwei drei vier fünf sechs sieben acht neun zehn");
        // 8 von 10 Wörtern stammen aus der Gegenseite -> genau 80 % -> verwerfen.
        let eight = words("eins zwei drei vier fünf sechs sieben acht hallo welt");
        assert!(should_drop_duplicate(&eight, &gegen, 20.0));
        // 7 von 10 -> behalten.
        let seven = words("eins zwei drei vier fünf sechs sieben hallo welt bitte");
        assert!(!should_drop_duplicate(&seven, &gegen, 20.0));
    }

    #[test]
    fn matching_ignores_case_punctuation_and_umlaut_spelling() {
        let gegen = words("Österreich, „Schweiz“ und Übergabe!");
        let ich = words("oesterreich schweiz und uebergabe");
        assert!(should_drop_duplicate(&ich, &gegen, 18.0));
    }

    #[test]
    fn each_far_word_covers_only_one_near_word() {
        let gegen = words("ja");
        let ich = words("ja ja ja ja");
        assert!(
            !should_drop_duplicate(&ich, &gegen, 25.0),
            "1 von 4 ist kein Echo"
        );
        let gegen = words("ja ja ja ja nein");
        assert!(should_drop_duplicate(&ich, &gegen, 25.0));
    }

    #[test]
    fn empty_or_unusable_input_never_drops() {
        let none: [&str; 0] = [];
        assert!(!should_drop_duplicate(&none, &words("hallo welt"), 30.0));
        assert!(!should_drop_duplicate(&words("hallo welt"), &none, 30.0));
        assert!(!should_drop_duplicate(
            &words("... ,"),
            &words("hallo"),
            30.0
        ));
        assert!(!should_drop_duplicate(
            &words("hallo"),
            &words("hallo"),
            f32::NAN
        ));
        assert!(!should_drop_duplicate(
            &words("hallo"),
            &words("hallo"),
            f32::INFINITY.min(f32::NAN)
        ));
        // Owned Strings gehen ebenfalls.
        let ich = vec!["hallo".to_string(), "welt".to_string()];
        let gegen = vec!["Hallo".to_string(), "Welt".to_string()];
        assert!(should_drop_duplicate(&ich, &gegen, 20.0));
    }

    #[test]
    fn rms_dbfs_is_calibrated() {
        assert_eq!(rms_dbfs(&[]), -120.0);
        assert_eq!(rms_dbfs(&[0; 160]), -120.0);
        let full_scale_square = [i16::MAX, -i16::MAX].repeat(80);
        assert!((rms_dbfs(&full_scale_square) - 0.0).abs() < 0.01);
        let half: Vec<i16> = [16384, -16384].repeat(80);
        assert!((rms_dbfs(&half) + 6.02).abs() < 0.05);
    }
}
