// M8 meetings fundament (docs/superpowers/plans/2026-08-19-m8-meetings-fundament.md):
// pure audio chunker for the live-transcription path. No I/O, no async, no
// Tauri types — a channel's i16 PCM stream in, ~20-s f32 blocks out, cut at
// the quietest spot so a chunk boundary never lands mid-word.

/// Samples per channel per millisecond. The meetings pipeline runs 16 kHz
/// mono throughout (see `transcribe_segments`'s `audio_ms` math), so the
/// chunker assumes the same rate rather than taking it as a parameter.
const SAMPLE_RATE_HZ: u64 = 16_000;
const SAMPLES_PER_MS: u64 = SAMPLE_RATE_HZ / 1_000;
/// Width of the energy window used to find the quietest cut point.
const CUT_WINDOW_MS: u64 = 200;

/// Sammelt i16-Samples eines Kanals und schneidet ~20-s-Blöcke für die
/// Transkription. Schnittpunkt: energieärmste 200-ms-Stelle im letzten
/// Viertel des Fensters, damit nicht mitten im Wort geschnitten wird. Pure
/// Logik, kein I/O.
pub struct ChannelChunker {
    buffer: Vec<i16>,
    target_len: usize,
    consumed_ms: u64,
}

/// One emitted block, ready for `TranscriptionManager::transcribe_segments`
/// (hence f32 samples) plus the offset into the channel's timeline where it
/// starts, in milliseconds.
pub struct Chunk {
    pub samples: Vec<f32>,
    pub offset_ms: u64,
}

impl ChannelChunker {
    /// `target_ms`: desired block length in milliseconds (production: 20_000).
    pub fn new(target_ms: u64) -> Self {
        let target_len = (target_ms * SAMPLES_PER_MS) as usize;
        Self {
            buffer: Vec::new(),
            target_len,
            consumed_ms: 0,
        }
    }

    /// Feeds more samples in. Returns a chunk once the buffer holds at least
    /// `target_ms` worth of audio; otherwise buffers and returns `None`.
    pub fn push(&mut self, samples: &[i16]) -> Option<Chunk> {
        self.buffer.extend_from_slice(samples);
        if self.buffer.len() < self.target_len {
            return None;
        }
        let cut = cut_point(&self.buffer, self.target_len);
        Some(self.emit(cut))
    }

    /// Emits whatever is left in the buffer (e.g. on stop/pause), even if
    /// shorter than `target_ms`. Returns `None` once the buffer is empty.
    pub fn flush(&mut self) -> Option<Chunk> {
        if self.buffer.is_empty() {
            return None;
        }
        let cut = self.buffer.len();
        Some(self.emit(cut))
    }

    fn emit(&mut self, cut: usize) -> Chunk {
        let tail: Vec<i16> = self.buffer.drain(0..cut).collect();
        let offset_ms = self.consumed_ms;
        self.consumed_ms += (tail.len() as u64 * 1_000) / SAMPLE_RATE_HZ;
        let samples = tail.iter().map(|&s| s as f32 / 32_768.0).collect();
        Chunk { samples, offset_ms }
    }
}

/// Finds where to cut `buffer` (at least `target_len` samples long): the end
/// of the minimum-RMS 200-ms window searched within `[0.75 * target_len,
/// buffer.len()]`. Falls back to `target_len` if the buffer is too short for
/// a full search window in that range (should not happen once `push` has
/// already checked `buffer.len() >= target_len` for any sane `target_len`).
pub(super) fn cut_point(buffer: &[i16], target_len: usize) -> usize {
    let window_len = (CUT_WINDOW_MS * SAMPLES_PER_MS) as usize;
    let search_start = target_len * 3 / 4;
    let buffer_len = buffer.len();

    if window_len == 0 || search_start + window_len > buffer_len {
        return target_len.min(buffer_len);
    }

    let mut sum_sq: f64 = buffer[search_start..search_start + window_len]
        .iter()
        .map(|&s| (s as f64) * (s as f64))
        .sum();
    let mut best_end = search_start + window_len;
    let mut best_sum = sum_sq;

    let mut i = search_start;
    while i + window_len < buffer_len {
        let outgoing = buffer[i] as f64;
        let incoming = buffer[i + window_len] as f64;
        sum_sq += incoming * incoming - outgoing * outgoing;
        i += 1;
        let end = i + window_len;
        if sum_sq < best_sum {
            best_sum = sum_sq;
            best_end = end;
        }
    }

    best_end
}

#[cfg(test)]
mod tests {
    use super::*;
    const RATE: usize = 16_000;

    fn loud(ms: usize) -> Vec<i16> {
        vec![12_000; ms * RATE / 1000]
    }
    fn quiet(ms: usize) -> Vec<i16> {
        vec![50; ms * RATE / 1000]
    }

    #[test]
    fn no_chunk_before_the_target_is_reached() {
        let mut c = ChannelChunker::new(20_000);
        assert!(c.push(&loud(19_000)).is_none());
    }

    #[test]
    fn the_cut_lands_in_the_quiet_zone_not_mid_word() {
        let mut c = ChannelChunker::new(20_000);
        // 17 s laut, 1 s leise, 3 s laut -> Schnitt muss in der leisen Zone liegen (17-18 s)
        let mut audio = loud(17_000);
        audio.extend(quiet(1_000));
        audio.extend(loud(3_000));
        let chunk = c.push(&audio).expect("21 s > 20 s Ziel");
        let cut_ms = chunk.samples.len() * 1000 / RATE;
        assert!(
            (17_000..=18_000).contains(&cut_ms),
            "Schnitt bei {cut_ms} ms statt in der Pause"
        );
        assert_eq!(chunk.offset_ms, 0);
    }

    #[test]
    fn offsets_accumulate_across_chunks() {
        let mut c = ChannelChunker::new(20_000);
        let first = c.push(&loud(21_000)).unwrap();
        let consumed = first.samples.len() * 1000 / RATE;
        let second_input = loud(21_000);
        let second = c.push(&second_input).unwrap();
        assert_eq!(second.offset_ms, consumed as u64);
    }

    #[test]
    fn flush_returns_the_tail_and_then_nothing() {
        let mut c = ChannelChunker::new(20_000);
        c.push(&loud(5_000));
        let tail = c.flush().expect("Rest muss kommen");
        assert_eq!(tail.samples.len(), 5 * RATE);
        assert!(c.flush().is_none());
    }

    #[test]
    fn i16_becomes_normalized_f32() {
        let mut c = ChannelChunker::new(1_000);
        let chunk = c.push(&vec![i16::MAX; RATE + 160]).unwrap();
        assert!((chunk.samples[0] - 1.0).abs() < 1e-3);
    }
}
