//! D1 (Goal issues-abschluss #70, M7 = #69 Bild- und Videoinhalte): Folien aus
//! einem Video erkennen.
//!
//! Ablauf (Messungen und Begruendung: `koordination/bild-video/spike/spike-bericht.md`):
//!
//! 1. **Abtasten** ([`ffmpeg::sample_video`]): ffmpeg dekodiert, verkleinert auf
//!    160x90 Graustufen und liefert 1 Bild je Sekunde ueber eine Pipe. Jedes Bild
//!    wird sofort gehasht ([`dhash64`]) und verworfen: konstanter Speicher, einige
//!    Byte je Sekunde Video.
//! 2. **Segmentieren** ([`segment`]): ein Wechsel ist ein Hamming-Abstand ueber der
//!    Schwelle zum Hash der laufenden Folie, der sich zwei Abtastungen haelt
//!    (filtert Ueberblendungen, Mauszeiger, Kameraruckler). Repraesentant eines
//!    Abschnitts ist das LETZTE Bild davor: bei Aufbaustufen der vollstaendige
//!    Endstand. Schwarzbilder (mittlere Helligkeit unter 8) werden markiert und
//!    nie zur Folie.
//! 3. **Zusammenfassen** ([`group_by_hash`]): kehrt ein Vortrag zu einer Folie
//!    zurueck, ist das ein weiteres Vorkommen DERSELBEN Folie, keine neue.
//!    D2 ([`ocr`]): je Hash-Gruppe wird ein Bild gelesen (Windows-OCR); Folien mit
//!    gleichem Text (Wort-Jaccard, [`SlideDetectConfig::text_jaccard`]) werden
//!    trotz weit entferntem Hash zusammengefuehrt, Folien unter 3 Woertern sind
//!    `ohne_text`. Ohne Texterkennung bleibt es beim Hash.
//! 4. **Bilder** ([`ffmpeg::extract_frame`]) je Folie als Vollbild und Vorschau in
//!    `<Besprechungsordner>/slides/`, danach die Zeile in `meeting_slides`
//!    ([`store`]). Der Auftrag ([`run`]) ist die Job-Phase
//!    [`JobPhase::Slides`](super::job::JobPhase::Slides): pausierbar, stoppbar.
//!
//! # Fehlerfaelle und Absicherung
//!
//! | Fall | Verhalten | Absicherung (Test) |
//! |---|---|---|
//! | Datei ohne Videospur (Audio, Cover-Bild) | Phase uebersprungen (`slides_no_video`), kein Fehler der Besprechung, nichts geschrieben | `an_audio_only_file_is_skipped_without_writing_anything`, `a_cover_art_stream_is_no_video` |
//! | ffmpeg fehlt | Phase uebersprungen (`slides_ffmpeg_missing`), kein Haenger | `a_missing_ffmpeg_is_reported_not_a_hang`, `sampling_errors_map_to_outcomes` |
//! | Datei weg / unlesbar / kaputt | Fehler mit Code (`slides_video_missing`, `slides_ffmpeg_failed`), vorhandene Folien bleiben | `a_corrupt_file_fails_with_the_ffmpeg_tail` |
//! | Sehr langes Video (3 h, 4K) | Abtastung streamt (RAM unabhaengig von der Laenge, 14 400 B Puffer), 4K verkleinert ffmpeg selbst; Deckel [`MAX_SAMPLES`] meldet `slides_too_long` statt zu wuchern | `the_reader_streams_a_very_long_video_in_constant_memory`, `too_many_samples_stop_the_decoder_and_report_it` |
//! | Abbruch / Stopp mitten im Lauf | ffmpeg wird ueber SEIN Handle beendet (Job-Objekt, nie ueber den Namen), kein Prozess bleibt; fertige Folien bleiben, der Lauf ist wiederholbar | `stopping_the_sampler_leaves_no_ffmpeg_process`, `a_stop_between_two_slides_keeps_the_finished_ones_and_a_rerun_completes_them` |
//! | Abbruch beim Schreiben eines Bildes / Absturz | Bild zuerst unter `*.part.jpg`, erst fertig umbenannt; Zeile erst NACH den Dateien; Reste werden beim naechsten Lauf geraeumt | `a_failed_extraction_leaves_no_partial_file`, `leftover_partial_files_are_cleaned_before_a_run` |
//! | Platte voll | ffmpeg-Text und Betriebssystemfehler werden als `slides_disk_full` erkannt; kein halbes Bild, keine Zeile ohne Datei | `a_full_disk_is_recognised_in_text_and_error_code`, `a_failed_insert_removes_the_images_it_just_wrote` |
//! | Besprechung waehrend des Laufs geloescht | die Zeile wird abgelehnt (`meeting_not_found`), die eben geschriebenen Bilder werden entfernt | `a_failed_insert_removes_the_images_it_just_wrote` |
//! | Zwei Laeufe fuer dieselbe Besprechung | der Auftragsverzeichnis-Schluessel (`job::global().try_start`) laesst nur einen zu (`job_busy`); Wiederholung fuegt nichts doppelt ein | `a_rerun_adds_nothing_and_keeps_hidden_flags` |
//! | Migration mit Altdaten / Abbruch | nur `CREATE ... IF NOT EXISTS`, Altdaten unberuehrt, Abbruch rollt vollstaendig zurueck, Sicherung vor dem Schritt | `the_migration_keeps_every_existing_row_and_is_idempotent`, `an_aborted_migration_leaves_the_old_database_untouched` |
//!
//! Speicherbedarf: der Lauf haelt hoechstens EIN 14 400-Byte-Bild und je Abtastung
//! 16 Byte (3 h = 10 800 Abtastungen = 170 kB). Bei knappem RAM passiert nichts
//! Besonderes: ffmpeg laeuft in einem Job-Objekt (CPU-Deckel, niedrige Prioritaet,
//! `KILL_ON_JOB_CLOSE`), und der Speicherwaechter der App betrifft nur Server.

pub mod ffmpeg;
pub mod ocr;
pub mod run;
pub mod store;
#[cfg(test)]
pub(crate) mod test_support;

use serde::{Deserialize, Serialize};
use specta::Type;

/// Breite und Hoehe der Abtastbilder. 160x90 reicht fuer den dHash und ist nach
/// dem Spike (Messung 1) das billigste Mass, das Folien noch trennt.
pub const SAMPLE_W: usize = 160;
pub const SAMPLE_H: usize = 90;
/// Bytes eines Abtastbildes (Graustufen).
pub const FRAME_BYTES: usize = SAMPLE_W * SAMPLE_H;
/// So viele Abtastungen gelten als Obergrenze (27 h bei 1 fps): darueber bricht
/// der Lauf mit `slides_too_long` ab, statt unbegrenzt zu wachsen.
pub const MAX_SAMPLES: usize = 100_000;
/// Unterordner der Besprechung fuer Vollbilder und Vorschauen.
pub const SLIDES_DIR: &str = "slides";
/// Zwei Vorkommen derselben Folie, die hoechstens so weit auseinander liegen,
/// werden zu einem Zeitbereich (ms).
pub const MERGE_GAP_MS: u64 = 1_000;

// ---------------------------------------------------------------------------
// Einstellungen und Fehler
// ---------------------------------------------------------------------------

/// Parameter der Erkennung. Die Voreinstellung ist die im Spike gemessene.
#[derive(Clone, Debug, PartialEq)]
pub struct SlideDetectConfig {
    /// Abtastungen je Sekunde Video (1.0 = ein Bild je Sekunde).
    pub sample_fps: f32,
    /// Hamming-Abstand (von 64 Bit), ab dem ein Bild "anders" ist (Schwelle 4).
    pub hash_threshold: u32,
    /// So viele Abtastungen muss der neue Zustand halten (2: eine Folgeabtastung).
    pub stable_samples: u32,
    /// Wort-Jaccard, ab dem zwei Folientexte dieselbe Folie sind (D2).
    pub text_jaccard: f32,
    /// Mittlere Helligkeit (0..255) unter der ein Bild als Schwarzbild gilt.
    pub black_luma: f32,
    /// Obergrenze der Abtastungen ([`MAX_SAMPLES`]).
    pub max_samples: usize,
}

impl Default for SlideDetectConfig {
    fn default() -> Self {
        Self {
            sample_fps: 1.0,
            hash_threshold: 4,
            stable_samples: 2,
            text_jaccard: 0.5,
            black_luma: 8.0,
            max_samples: MAX_SAMPLES,
        }
    }
}

/// Optionen eines Starts von der Oberflaeche (`detect_meeting_slides`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct SlideOptions {
    /// Videodatei; ohne Angabe die Quelle des Imports (`Meeting::source_path`).
    pub video_path: Option<String>,
    /// Abtastabstand in Sekunden (Voreinstellung 1, zulaessig 0,25 bis 10).
    pub sample_interval_s: Option<f32>,
}

impl SlideOptions {
    /// Die Erkennungsparameter: Voreinstellung, mit dem gewuenschten Abstand.
    pub fn to_config(&self) -> SlideDetectConfig {
        let mut cfg = SlideDetectConfig::default();
        if let Some(interval) = self.sample_interval_s {
            if interval.is_finite() {
                cfg.sample_fps = 1.0 / interval.clamp(0.25, 10.0);
            }
        }
        cfg
    }
}

/// Woran die Folienerkennung scheitert oder endet. `code()` geht als Text an die
/// Oberflaeche (`slides_*`), das Detail nur ins Log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlideError {
    /// ffmpeg ist nicht installiert (nicht im PATH).
    FfmpegMissing,
    /// Die Datei hat keine Videospur (Audio, nur ein Titelbild).
    NoVideoStream,
    /// Die Videodatei gibt es nicht (mehr).
    VideoMissing,
    /// Der Nutzer hat gestoppt (kein Fehler).
    Cancelled,
    /// Der Datentraeger ist voll.
    DiskFull,
    /// Mehr Abtastungen als [`MAX_SAMPLES`].
    TooLong,
    /// ffmpeg ist gescheitert; die letzten Zeilen seiner Fehlerausgabe.
    Ffmpeg(String),
    /// Dateizugriff (Ordner, Bild umbenennen, ...).
    Io(String),
    /// Die Datenbank hat die Zeile abgelehnt; der Text ist der Code des Stores.
    Store(String),
}

impl SlideError {
    pub fn code(&self) -> &'static str {
        match self {
            SlideError::FfmpegMissing => "slides_ffmpeg_missing",
            SlideError::NoVideoStream => "slides_no_video",
            SlideError::VideoMissing => "slides_video_missing",
            SlideError::Cancelled => "slides_cancelled",
            SlideError::DiskFull => "slides_disk_full",
            SlideError::TooLong => "slides_too_long",
            SlideError::Ffmpeg(_) => "slides_ffmpeg_failed",
            SlideError::Io(_) => "slides_io_failed",
            SlideError::Store(_) => "slides_store_failed",
        }
    }
}

impl std::fmt::Display for SlideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SlideError::Ffmpeg(detail) | SlideError::Io(detail) | SlideError::Store(detail) => {
                write!(f, "{}: {detail}", self.code())
            }
            other => f.write_str(other.code()),
        }
    }
}

impl std::error::Error for SlideError {}

impl From<std::io::Error> for SlideError {
    fn from(e: std::io::Error) -> Self {
        if ffmpeg::is_disk_full_io(&e) {
            SlideError::DiskFull
        } else {
            SlideError::Io(e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Hash (rein)
// ---------------------------------------------------------------------------

/// Differenz-Hash (dHash) mit 64 Bit: das Bild wird auf 9x8 Zellen gemittelt
/// (Boxfilter, ganzzahlig), jedes Bit sagt "die Zelle rechts ist heller als die
/// links" (Zeile fuer Zeile, hoechstes Bit zuerst, wie `imagehash.dhash`).
///
/// Gradienten statt Frequenzanteile: ein dunkles Kamerafeld in einer Ecke kippt
/// nur die Bits seiner Zellen, waehrend pHash wegen der DCT-Anteile einbrach
/// (Spike, Messung 1). Zu kleine oder zu kurze Eingaben ergeben 0.
pub fn dhash64(gray: &[u8], w: usize, h: usize) -> u64 {
    if w < 9 || h < 8 || gray.len() < w * h {
        return 0;
    }
    let mut cells = [[0u32; 9]; 8];
    for (cy, row) in cells.iter_mut().enumerate() {
        let (y0, y1) = (cy * h / 8, (cy + 1) * h / 8);
        for (cx, cell) in row.iter_mut().enumerate() {
            let (x0, x1) = (cx * w / 9, (cx + 1) * w / 9);
            let mut sum = 0u32;
            for y in y0..y1 {
                sum += gray[y * w + x0..y * w + x1]
                    .iter()
                    .map(|&p| u32::from(p))
                    .sum::<u32>();
            }
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            *cell = (sum + count / 2) / count;
        }
    }
    let mut hash = 0u64;
    for row in &cells {
        for cx in 0..8 {
            hash <<= 1;
            if row[cx + 1] > row[cx] {
                hash |= 1;
            }
        }
    }
    hash
}

/// Anzahl der Bits, in denen sich zwei Hashes unterscheiden.
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Mittlere Helligkeit eines Graustufenbildes (0..255); leeres Bild: 0.
pub fn mean_luma(gray: &[u8]) -> f32 {
    if gray.is_empty() {
        return 0.0;
    }
    let sum: u64 = gray.iter().map(|&p| u64::from(p)).sum();
    sum as f32 / gray.len() as f32
}

// ---------------------------------------------------------------------------
// Segmentierung (rein)
// ---------------------------------------------------------------------------

/// Ein Zeitbereich, in dem eine Folie zu sehen ist (ms im Video).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SlideOccurrence {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Ein Abschnitt des Videos mit gleichbleibendem Bild.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideSegment {
    /// Erste Abtastung des Abschnitts (ms).
    pub start_ms: u64,
    /// Beginn des naechsten Abschnitts, beim letzten das Ende der Abtastung (ms).
    pub end_ms: u64,
    /// Zeit des Repraesentanten: das letzte Bild vor dem naechsten Wechsel (ms).
    pub rep_ms: u64,
    /// Hash des Repraesentanten.
    pub hash: u64,
    /// Der Repraesentant ist ein Schwarzbild.
    pub black: bool,
}

impl SlideSegment {
    pub fn occurrence(&self) -> SlideOccurrence {
        SlideOccurrence {
            start_ms: self.start_ms,
            end_ms: self.end_ms,
        }
    }
}

fn is_black(luma: f32, cfg: &SlideDetectConfig) -> bool {
    luma < cfg.black_luma
}

/// Sind zwei Abtastungen "dasselbe Bild"? Schwarz ist nie dasselbe wie Nicht-
/// Schwarz (der dHash eines Schwarzbildes ist 0, wie der jeder einfarbigen Flaeche).
fn similar(a: (u64, f32), b: (u64, f32), cfg: &SlideDetectConfig) -> bool {
    let (black_a, black_b) = (is_black(a.1, cfg), is_black(b.1, cfg));
    if black_a || black_b {
        return black_a == black_b;
    }
    hamming(a.0, b.0) <= cfg.hash_threshold
}

/// Zerlegt die Abtastungen (Hash und mittlere Helligkeit je Bild) in Abschnitte.
///
/// Wechsel bei Abtastung `i`: `i` unterscheidet sich vom Hash der laufenden Folie
/// UND die naechsten `stable_samples - 1` Abtastungen sehen aus wie `i` (der neue
/// Zustand haelt). Ein einzelnes abweichendes Bild (Ueberblendung, Zuckler) ist
/// kein Wechsel. Ein Wechsel in der allerletzten Abtastung wird nicht gemeldet
/// (es fehlt die Bestaetigung; eine Folie, die weniger als zwei Abtastungen
/// steht, ist keine).
pub fn segment(samples: &[(u64, f32)], fps: f32, cfg: &SlideDetectConfig) -> Vec<SlideSegment> {
    let n = samples.len();
    if n == 0 {
        return Vec::new();
    }
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        1.0
    };
    let to_ms = |i: usize| (i as f64 * 1000.0 / f64::from(fps)).round() as u64;
    let stable = cfg.stable_samples.max(1) as usize;

    let mut starts = vec![0usize];
    let mut current = samples[0];
    for i in 1..n {
        if similar(samples[i], current, cfg) {
            continue;
        }
        let holds = (1..stable).all(|k| i + k < n && similar(samples[i + k], samples[i], cfg));
        if holds {
            starts.push(i);
            current = samples[i];
        }
    }

    starts
        .iter()
        .enumerate()
        .map(|(s, &start)| {
            let end = starts.get(s + 1).copied().unwrap_or(n);
            let rep = end - 1;
            SlideSegment {
                start_ms: to_ms(start),
                end_ms: to_ms(end),
                rep_ms: to_ms(rep),
                hash: samples[rep].0,
                black: is_black(samples[rep].1, cfg),
            }
        })
        .collect()
}

/// Das Ende des letzten Abschnitts ist die Dauer der Abtastung (Anzahl x Abstand),
/// nicht die des Videos: mit der echten Dauer (aus dem ffmpeg-Kopf) setzen. Nie
/// vor den Beginn des Abschnitts.
pub fn clamp_last_end(segments: &mut [SlideSegment], duration_ms: Option<u64>) {
    if let (Some(last), Some(duration)) = (segments.last_mut(), duration_ms) {
        last.end_ms = duration.max(last.start_ms);
    }
}

// ---------------------------------------------------------------------------
// Zusammenfassen (rein)
// ---------------------------------------------------------------------------

/// Eine Folie mit allen Stellen, an denen sie im Video zu sehen ist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideGroup {
    /// Index des ersten Abschnitts (aus ihm kommt das Bild).
    pub first: usize,
    /// Hash des ersten Abschnitts.
    pub hash: u64,
    /// Zeit des Bildes (ms): der Repraesentant des ersten Abschnitts.
    pub rep_ms: u64,
    /// Alle Vorkommen, zeitlich geordnet, Beruehrendes zusammengefasst.
    pub occurrences: Vec<SlideOccurrence>,
    /// Indizes aller Abschnitte dieser Folie.
    pub segments: Vec<usize>,
}

/// Fasst Abschnitte zusammen, die `distance` fuer dieselbe Folie haelt. `distance`
/// liefert `Some(Abstand)` fuer "dieselbe Folie" und `None` sonst; der Abschnitt
/// kommt zur Folie mit dem kleinsten Abstand (bei Gleichstand zur frueheren).
/// Schwarzbilder bleiben aussen vor. Die Folien sind nach ihrem ersten Auftreten
/// geordnet. D2 haengt hier den Textabgleich ein.
pub fn group_with(
    segments: &[SlideSegment],
    distance: &dyn Fn(&SlideSegment, &SlideSegment) -> Option<u32>,
) -> Vec<SlideGroup> {
    let mut groups: Vec<SlideGroup> = Vec::new();
    for (index, seg) in segments.iter().enumerate() {
        if seg.black {
            continue;
        }
        let best = groups
            .iter()
            .enumerate()
            .filter_map(|(g, group)| distance(&segments[group.first], seg).map(|d| (d, g)))
            .min();
        match best {
            Some((_, g)) => {
                let group = &mut groups[g];
                group.occurrences = merge_occurrences(&group.occurrences, &[seg.occurrence()]);
                group.segments.push(index);
            }
            None => groups.push(SlideGroup {
                first: index,
                hash: seg.hash,
                rep_ms: seg.rep_ms,
                occurrences: vec![seg.occurrence()],
                segments: vec![index],
            }),
        }
    }
    groups
}

/// Dieselbe Folie = Hamming-Abstand der Hashes hoechstens die Schwelle.
pub fn group_by_hash(segments: &[SlideSegment], cfg: &SlideDetectConfig) -> Vec<SlideGroup> {
    group_with(segments, &|a, b| {
        let d = hamming(a.hash, b.hash);
        (d <= cfg.hash_threshold).then_some(d)
    })
}

/// Vereint zwei Listen von Vorkommen: geordnet, Ueberlappendes und Beruehrendes
/// (Abstand hoechstens [`MERGE_GAP_MS`]) in einem Bereich. Idempotent: wer
/// dasselbe noch einmal hinzufuegt, aendert nichts.
pub fn merge_occurrences(a: &[SlideOccurrence], b: &[SlideOccurrence]) -> Vec<SlideOccurrence> {
    let mut all: Vec<SlideOccurrence> = a.iter().chain(b.iter()).copied().collect();
    all.sort_by_key(|o| (o.start_ms, o.end_ms));
    let mut out: Vec<SlideOccurrence> = Vec::with_capacity(all.len());
    for o in all {
        match out.last_mut() {
            Some(last) if o.start_ms <= last.end_ms.saturating_add(MERGE_GAP_MS) => {
                last.end_ms = last.end_ms.max(o.end_ms);
            }
            _ => out.push(o),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SlideDetectConfig {
        SlideDetectConfig::default()
    }

    /// Ein Bild aus 160x90 mit senkrechten Streifen: `bright` Spalten (in Zellen
    /// zu je 17 px) sind hell. Verschiedene Muster haben weit auseinanderliegende Hashes.
    fn frame(pattern: u64) -> Vec<u8> {
        // Dieselben Zellgrenzen wie `dhash64` (Zelle c = [c*w/9, (c+1)*w/9)).
        let cell_of =
            |v: usize, n: usize, len: usize| (0..n).find(|c| v < (c + 1) * len / n).unwrap();
        let mut px = vec![40u8; FRAME_BYTES];
        for y in 0..SAMPLE_H {
            for x in 0..SAMPLE_W {
                let cell_x = cell_of(x, 9, SAMPLE_W);
                let cell_y = cell_of(y, 8, SAMPLE_H);
                let bit = (pattern >> ((cell_y * 9 + cell_x) % 64)) & 1;
                px[y * SAMPLE_W + x] = if bit == 1 { 200 } else { 40 };
            }
        }
        px
    }

    /// Hashes, die sich um `bits` Bit unterscheiden (die niedrigsten Bits kippen).
    fn flip(hash: u64, bits: u32) -> u64 {
        hash ^ ((1u64 << bits) - 1)
    }

    const A: u64 = 0xA5A5_A5A5_0F0F_F0F0;
    const B: u64 = 0x1234_5678_9ABC_DEF0;
    const C: u64 = 0xF0E1_D2C3_B4A5_9687;
    const LIGHT: f32 = 120.0;

    fn seq(parts: &[(u64, usize)]) -> Vec<(u64, f32)> {
        parts
            .iter()
            .flat_map(|&(h, n)| std::iter::repeat((h, LIGHT)).take(n))
            .collect()
    }

    // -- Hash -----------------------------------------------------------------

    #[test]
    fn a_rising_gradient_sets_every_bit_and_a_falling_one_none() {
        let rising: Vec<u8> = (0..FRAME_BYTES)
            .map(|i| ((i % SAMPLE_W) * 255 / SAMPLE_W) as u8)
            .collect();
        assert_eq!(dhash64(&rising, SAMPLE_W, SAMPLE_H), u64::MAX);
        let falling: Vec<u8> = rising.iter().map(|p| 255 - p).collect();
        assert_eq!(dhash64(&falling, SAMPLE_W, SAMPLE_H), 0);
        assert_eq!(
            dhash64(&vec![77u8; FRAME_BYTES], SAMPLE_W, SAMPLE_H),
            0,
            "flach"
        );
    }

    #[test]
    fn the_hash_is_stable_against_noise_and_a_small_moving_field() {
        let base = frame(A);
        let h = dhash64(&base, SAMPLE_W, SAMPLE_H);
        // Rauschen +-3 Stufen (Kompression): Mittelwerte je Zelle bleiben.
        let noisy: Vec<u8> = base
            .iter()
            .enumerate()
            .map(|(i, &p)| (i64::from(p) + [(0i64), 3, -3, 2, -2][i % 5]) as u8)
            .collect();
        assert!(hamming(h, dhash64(&noisy, SAMPLE_W, SAMPLE_H)) <= 2);
        // Ein 20x12-Feld (rund 2 % der Flaeche, "Webcam") in der Ecke wechselt dauernd.
        let mut with_cam = base.clone();
        for step in 0..30u32 {
            for y in 76..88 {
                for x in 136..156 {
                    with_cam[y * SAMPLE_W + x] =
                        ((x as u32 * 7 + y as u32 * 3 + step * 29) % 256) as u8;
                }
            }
            assert!(
                hamming(h, dhash64(&with_cam, SAMPLE_W, SAMPLE_H)) <= cfg().hash_threshold,
                "das Kamerafeld darf in Schritt {step} keinen Wechsel ausloesen"
            );
        }
    }

    #[test]
    fn different_layouts_are_far_apart() {
        let (a, b, c) = (
            dhash64(&frame(A), SAMPLE_W, SAMPLE_H),
            dhash64(&frame(B), SAMPLE_W, SAMPLE_H),
            dhash64(&frame(C), SAMPLE_W, SAMPLE_H),
        );
        for (x, y) in [(a, b), (a, c), (b, c)] {
            assert!(hamming(x, y) > 8, "Muster liegen weit auseinander");
        }
    }

    #[test]
    fn too_small_or_short_input_hashes_to_zero_instead_of_panicking() {
        assert_eq!(dhash64(&[1, 2, 3], 160, 90), 0, "zu kurz");
        assert_eq!(dhash64(&vec![5u8; 64], 8, 8), 0, "zu schmal");
        assert_eq!(dhash64(&[], 0, 0), 0);
        assert_eq!(hamming(0, u64::MAX), 64);
        assert_eq!(mean_luma(&[]), 0.0);
        assert_eq!(mean_luma(&[0, 255]), 127.5);
    }

    // -- Segmentierung ----------------------------------------------------------

    #[test]
    fn three_slides_become_three_segments_with_their_times() {
        let s = seq(&[(A, 10), (B, 10), (C, 10)]);
        let segs = segment(&s, 1.0, &cfg());
        let times: Vec<(u64, u64, u64)> = segs
            .iter()
            .map(|g| (g.start_ms, g.end_ms, g.rep_ms))
            .collect();
        assert_eq!(
            times,
            vec![
                (0, 10_000, 9_000),
                (10_000, 20_000, 19_000),
                (20_000, 30_000, 29_000)
            ]
        );
        assert_eq!(
            segs[1].hash, B,
            "Repraesentant = letztes Bild vor dem Wechsel"
        );
        assert!(segs.iter().all(|g| !g.black));
    }

    #[test]
    fn a_single_odd_sample_is_a_fade_not_a_change() {
        // Ueberblendung/Zuckler: ein Bild weicht ab, das naechste ist wieder die alte Folie.
        let mut s = seq(&[(A, 6), (B, 1), (A, 6)]);
        let segs = segment(&s, 1.0, &cfg());
        assert_eq!(segs.len(), 1, "ein einzelnes Bild trennt nichts");
        // Zwei abweichende Bilder in Folge, danach die alte: ebenfalls kein Wechsel
        // (die Stabilitaet wird gegen das erste abweichende Bild geprueft, nicht gegen die alte Folie).
        s = seq(&[(A, 6), (B, 1), (C, 1), (A, 6)]);
        assert_eq!(segment(&s, 1.0, &cfg()).len(), 1);
    }

    #[test]
    fn small_differences_below_the_threshold_do_not_split() {
        let s = seq(&[(A, 3), (flip(A, 4), 3), (flip(A, 2), 3)]);
        assert_eq!(
            segment(&s, 1.0, &cfg()).len(),
            1,
            "4 Bit Abstand = noch dieselbe Folie"
        );
        let s5 = seq(&[(A, 3), (flip(A, 5), 3)]);
        assert_eq!(
            segment(&s5, 1.0, &cfg()).len(),
            2,
            "5 Bit Abstand = Wechsel"
        );
    }

    #[test]
    fn build_up_stages_keep_the_final_state_as_representative() {
        // Stufe 1 und 2 (eine Zeile mehr, unter der Schwelle) bilden EINEN Abschnitt,
        // dessen Repraesentant die Stufe 2 ist; Stufe 3 (deutlich mehr) ist der naechste.
        let stage1 = flip(A, 0);
        let stage2 = flip(A, 3);
        let stage3 = flip(A, 9);
        let s = seq(&[(stage1, 4), (stage2, 4), (stage3, 5)]);
        let segs = segment(&s, 1.0, &cfg());
        assert_eq!(segs.len(), 2);
        assert_eq!(
            segs[0].hash, stage2,
            "Endstand der ersten Gruppe, nicht Stufe 1"
        );
        assert_eq!(segs[0].rep_ms, 7_000);
        assert_eq!(segs[1].hash, stage3);
        // Der Aufbau-Endstand ist ein eigenes Bild (Spike: 14 verschiedene Folien
        // plus der Endstand getrennt von Stufe 1 und 2).
        let groups = group_by_hash(&segs, &cfg());
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn a_return_to_an_earlier_slide_is_a_second_occurrence_of_the_same_slide() {
        let s = seq(&[(A, 5), (B, 5), (A, 5), (C, 5), (A, 5)]);
        let segs = segment(&s, 1.0, &cfg());
        assert_eq!(segs.len(), 5);
        let groups = group_by_hash(&segs, &cfg());
        assert_eq!(groups.len(), 3, "A, B, C");
        assert_eq!(groups[0].segments, vec![0, 2, 4]);
        assert_eq!(
            groups[0].occurrences,
            vec![
                SlideOccurrence {
                    start_ms: 0,
                    end_ms: 5_000
                },
                SlideOccurrence {
                    start_ms: 10_000,
                    end_ms: 15_000
                },
                SlideOccurrence {
                    start_ms: 20_000,
                    end_ms: 25_000
                },
            ]
        );
        assert_eq!(
            groups[0].rep_ms, 4_000,
            "das Bild kommt aus dem ersten Auftreten"
        );
        assert_eq!(groups[1].first, 1);
    }

    #[test]
    fn black_frames_are_marked_and_never_become_slides() {
        let mut s = seq(&[(A, 5)]);
        s.extend(std::iter::repeat((0u64, 1.0f32)).take(4)); // Schwarzbild
        s.extend(seq(&[(B, 5)]));
        let segs = segment(&s, 1.0, &cfg());
        assert_eq!(segs.len(), 3);
        assert!(segs[1].black);
        let groups = group_by_hash(&segs, &cfg());
        assert_eq!(groups.len(), 2, "das Schwarzbild ist keine Folie");
        assert!(groups.iter().all(|g| !g.segments.contains(&1)));
    }

    #[test]
    fn black_and_a_flat_grey_slide_are_told_apart_although_both_hash_to_zero() {
        let mut s: Vec<(u64, f32)> = std::iter::repeat((0u64, 2.0f32)).take(4).collect();
        s.extend(std::iter::repeat((0u64, 128.0f32)).take(4));
        let segs = segment(&s, 1.0, &cfg());
        assert_eq!(segs.len(), 2, "gleicher Hash, aber schwarz gegen grau");
        assert!(segs[0].black && !segs[1].black);
    }

    #[test]
    fn a_change_in_the_very_last_sample_is_not_reported() {
        let s = seq(&[(A, 6), (B, 1)]);
        let segs = segment(&s, 1.0, &cfg());
        assert_eq!(segs.len(), 1, "keine Bestaetigung mehr moeglich");
        assert_eq!(segs[0].end_ms, 7_000);
        // Zwei Abtastungen reichen.
        assert_eq!(segment(&seq(&[(A, 6), (B, 2)]), 1.0, &cfg()).len(), 2);
    }

    #[test]
    fn times_follow_the_sampling_rate() {
        let s = seq(&[(A, 4), (B, 4)]);
        let segs = segment(&s, 0.5, &cfg()); // ein Bild alle 2 s
        assert_eq!(
            (segs[1].start_ms, segs[1].rep_ms, segs[1].end_ms),
            (8_000, 14_000, 16_000)
        );
        let fast = segment(&s, 4.0, &cfg());
        assert_eq!(fast[1].start_ms, 1_000);
        // Unsinnige Rate: wie 1 fps, kein Absturz.
        assert_eq!(segment(&s, 0.0, &cfg())[1].start_ms, 4_000);
        assert_eq!(segment(&s, f32::NAN, &cfg())[1].start_ms, 4_000);
    }

    #[test]
    fn empty_and_tiny_inputs_are_handled() {
        assert!(segment(&[], 1.0, &cfg()).is_empty());
        let one = segment(&[(A, LIGHT)], 1.0, &cfg());
        assert_eq!(one.len(), 1);
        assert_eq!(
            (one[0].start_ms, one[0].end_ms, one[0].rep_ms),
            (0, 1_000, 0)
        );
    }

    #[test]
    fn without_lookahead_a_single_deviating_sample_does_split() {
        let c = SlideDetectConfig {
            stable_samples: 1,
            ..cfg()
        };
        assert_eq!(segment(&seq(&[(A, 3), (B, 1), (A, 3)]), 1.0, &c).len(), 3);
    }

    #[test]
    fn the_last_segment_ends_with_the_real_duration() {
        let mut segs = segment(&seq(&[(A, 5), (B, 5)]), 1.0, &cfg());
        clamp_last_end(&mut segs, Some(9_600));
        assert_eq!(segs[1].end_ms, 9_600);
        clamp_last_end(&mut segs, Some(3_000));
        assert_eq!(segs[1].end_ms, 5_000, "nie vor dem Beginn");
        clamp_last_end(&mut segs, None);
        assert_eq!(segs[1].end_ms, 5_000);
        clamp_last_end(&mut [], Some(1));
    }

    // -- Zusammenfassen -----------------------------------------------------------

    #[test]
    fn grouping_picks_the_nearest_slide_and_lets_a_custom_rule_decide() {
        let seg = |h: u64, start: u64| SlideSegment {
            start_ms: start,
            end_ms: start + 1_000,
            rep_ms: start,
            hash: h,
            black: false,
        };
        // G1 und G2 sind 6 Bit auseinander (zwei Folien); das dritte Bild liegt 4 Bit von G1 und 2 von G2.
        let (g1, g2) = (A, flip(A, 6));
        let third = g2 ^ 0b11; // 2 Bit von g2, 4 von g1
        assert_eq!(hamming(g1, third), 4);
        assert_eq!(hamming(g2, third), 2);
        let groups = group_by_hash(&[seg(g1, 0), seg(g2, 1_000), seg(third, 2_000)], &cfg());
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[1].segments, vec![1, 2], "zur naeheren Folie");
        // Eine eigene Regel (Text-Abgleich in D2): alles ist dieselbe Folie.
        let all_same = group_with(&[seg(g1, 0), seg(B, 1_000)], &|_, _| Some(0));
        assert_eq!(all_same.len(), 1);
        // ... und nichts ist dieselbe Folie.
        let none = group_with(&[seg(g1, 0), seg(g1, 1_000)], &|_, _| None);
        assert_eq!(none.len(), 2);
    }

    #[test]
    fn touching_occurrences_merge_and_merging_is_idempotent() {
        let o = |s, e| SlideOccurrence {
            start_ms: s,
            end_ms: e,
        };
        let a = vec![o(0, 5_000), o(10_000, 15_000)];
        assert_eq!(
            merge_occurrences(&a, &a),
            a,
            "dasselbe noch einmal aendert nichts"
        );
        assert_eq!(merge_occurrences(&a, &[]), a);
        assert_eq!(
            merge_occurrences(&a, &[o(5_500, 9_500)]),
            vec![o(0, 15_000)],
            "Luecken unter einer Sekunde schliessen sich"
        );
        assert_eq!(
            merge_occurrences(&[o(0, 5_000)], &[o(7_000, 8_000)]),
            vec![o(0, 5_000), o(7_000, 8_000)]
        );
        assert_eq!(
            merge_occurrences(&[o(4_000, 6_000)], &[o(0, 5_000)]),
            vec![o(0, 6_000)]
        );
    }

    #[test]
    fn options_map_to_the_sampling_rate_within_limits() {
        assert_eq!(
            SlideOptions::default().to_config(),
            SlideDetectConfig::default()
        );
        let fps = |s: f32| {
            SlideOptions {
                sample_interval_s: Some(s),
                video_path: None,
            }
            .to_config()
            .sample_fps
        };
        assert_eq!(fps(2.0), 0.5);
        assert_eq!(fps(0.01), 4.0, "nie dichter als 0,25 s");
        assert_eq!(fps(1000.0), 0.1, "nie weiter als 10 s");
        assert_eq!(fps(f32::NAN), 1.0);
    }

    #[test]
    fn error_codes_are_stable_strings() {
        assert_eq!(SlideError::NoVideoStream.code(), "slides_no_video");
        assert_eq!(SlideError::FfmpegMissing.code(), "slides_ffmpeg_missing");
        assert_eq!(SlideError::DiskFull.to_string(), "slides_disk_full");
        assert_eq!(
            SlideError::Ffmpeg("boom".into()).to_string(),
            "slides_ffmpeg_failed: boom"
        );
    }
}
