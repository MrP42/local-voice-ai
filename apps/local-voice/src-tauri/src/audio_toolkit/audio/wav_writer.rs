//! Incremental WAV writer (PCM i16 mono) that patches its own RIFF/data size
//! fields on demand, so a recording that crashes mid-stream stays a readable
//! WAV file up to the last `flush_header()` call.
//!
//! `hound` is deliberately not used for writing: it has no support for
//! patching the size fields of an already-open file, which is exactly what
//! crash-safety here depends on.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

const HEADER_LEN: u64 = 44;
const BITS_PER_SAMPLE: u16 = 16;
const CHANNELS: u16 = 1;
const BYTES_PER_SAMPLE: u32 = (BITS_PER_SAMPLE / 8) as u32;

/// The classic WAV/RIFF format stores chunk sizes in a `u32`, so a single
/// file can describe at most `u32::MAX` bytes of header+data. This is the
/// largest `data` chunk that still leaves the RIFF size (`36 + data_len`)
/// representable in that `u32` — about 37 hours at 16 kHz mono i16.
const MAX_DATA_BYTES: u64 = u32::MAX as u64 - 36;

/// Writes a 44-byte canonical PCM WAV header (mono, i16) with the given
/// data length into `data_len` field, and RIFF size derived from it.
fn write_header(file: &mut File, sample_rate: u32, data_len: u32) -> io::Result<()> {
    let byte_rate = sample_rate * CHANNELS as u32 * BYTES_PER_SAMPLE;
    let block_align = CHANNELS * BYTES_PER_SAMPLE as u16;

    let mut header = [0u8; HEADER_LEN as usize];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + data_len).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    header[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    header[22..24].copy_from_slice(&CHANNELS.to_le_bytes());
    header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_len.to_le_bytes());

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    Ok(())
}

/// Streams i16 mono PCM samples to a WAV file, patching the RIFF/data size
/// fields via `flush_header()` so the file stays readable even if the
/// process crashes before `finalize()` runs. Call `flush_header()` roughly
/// every second during a long recording.
pub struct StreamingWavWriter {
    file: File,
    sample_rate: u32,
    frames_written: u64,
}

impl StreamingWavWriter {
    /// Creates `path` and writes a 44-byte header with size fields set to 0.
    pub fn create(path: &Path, sample_rate: u32) -> io::Result<Self> {
        let mut file = File::create(path)?;
        write_header(&mut file, sample_rate, 0)?;
        Ok(Self {
            file,
            sample_rate,
            frames_written: 0,
        })
    }

    /// Appends samples (little-endian i16) and advances `frames_written`.
    ///
    /// Refuses (without writing anything) if this would push the `data`
    /// chunk past `MAX_DATA_BYTES`, the RIFF/WAV `u32` size-field limit.
    /// Callers should end the recording cleanly on this error.
    pub fn append(&mut self, samples: &[i16]) -> io::Result<()> {
        let additional_bytes = samples.len() as u64 * BYTES_PER_SAMPLE as u64;
        let current_bytes = self.frames_written * BYTES_PER_SAMPLE as u64;
        if current_bytes + additional_bytes > MAX_DATA_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "wav size limit reached (RIFF u32)",
            ));
        }

        let mut buf = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        self.file.write_all(&buf)?;
        self.frames_written += samples.len() as u64;
        Ok(())
    }

    /// Test-only hook to move the internal frame counter near the RIFF
    /// size limit without actually writing gigabytes of samples to disk.
    #[cfg(test)]
    fn set_frames_for_test(&mut self, frames: u64) {
        self.frames_written = frames;
    }

    /// Patches the RIFF and data size fields to reflect `frames_written` so
    /// far, then syncs to disk. Seeks back to the end afterwards so
    /// subsequent `append()` calls keep writing at the tail.
    pub fn flush_header(&mut self) -> io::Result<()> {
        let data_len_u64 = self.frames_written * BYTES_PER_SAMPLE as u64;
        debug_assert!(
            data_len_u64 <= MAX_DATA_BYTES,
            "append() must reject writes before frames_written crosses MAX_DATA_BYTES"
        );
        let data_len = data_len_u64 as u32;
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&(36 + data_len).to_le_bytes())?;
        self.file.seek(SeekFrom::Start(40))?;
        self.file.write_all(&data_len.to_le_bytes())?;
        self.file.seek(SeekFrom::End(0))?;
        self.file.sync_data()?;
        Ok(())
    }

    /// Patches the header a final time and consumes the writer, returning
    /// the recording's duration in milliseconds.
    pub fn finalize(mut self) -> io::Result<u64> {
        self.flush_header()?;
        Ok(frames_to_ms(self.frames_written, self.sample_rate))
    }

    pub fn frames_written(&self) -> u64 {
        self.frames_written
    }

    /// Audio position in milliseconds: everything appended so far, on the
    /// same timeline as the transcript (`StoredSegment.start_ms`). Uses the
    /// writer's own sample rate, so it stays right if that ever changes.
    pub fn position_ms(&self) -> u64 {
        frames_to_ms(self.frames_written, self.sample_rate)
    }
}

fn frames_to_ms(frames: u64, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }
    frames * 1000 / sample_rate as u64
}

/// Repairs a WAV file that was left behind without a `finalize()` call
/// (e.g. the process crashed mid-recording): reconstructs the RIFF/data
/// size fields from the actual file length and patches them in place.
///
/// Returns the repaired recording's duration in milliseconds, or `None`
/// if `path` isn't a file this can recognize as a WAV header (too short,
/// or missing the RIFF/WAVE magic).
pub fn repair_orphan_wav(path: &Path) -> io::Result<Option<u64>> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let len = file.metadata()?.len();
    if len < HEADER_LEN {
        return Ok(None);
    }

    let mut header = [0u8; HEADER_LEN as usize];
    file.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Ok(None);
    }

    let sample_rate = u32::from_le_bytes(header[24..28].try_into().unwrap());
    if sample_rate == 0 {
        return Ok(None);
    }

    // Clamp rather than refuse: a file that grew past the RIFF u32 limit
    // (should not happen given append()'s guard, but could via manual
    // tampering or a foreign writer) is still repairable up to the limit —
    // that beats leaving it unplayable.
    let data_len = (len - HEADER_LEN).min(MAX_DATA_BYTES) as u32;
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&(36 + data_len).to_le_bytes())?;
    file.seek(SeekFrom::Start(40))?;
    file.write_all(&data_len.to_le_bytes())?;
    file.sync_data()?;

    let frames = data_len as u64 / BYTES_PER_SAMPLE as u64;
    Ok(Some(frames_to_ms(frames, sample_rate)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_wav_is_readable_by_hound_after_finalize() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        let mut w = StreamingWavWriter::create(&p, 16_000).unwrap();
        w.append(&vec![0i16; 16_000]).unwrap(); // 1 s
        let ms = w.finalize().unwrap();
        assert_eq!(ms, 1000);
        let r = hound::WavReader::open(&p).unwrap();
        assert_eq!(r.spec().sample_rate, 16_000);
        assert_eq!(r.spec().channels, 1);
        assert_eq!(r.len(), 16_000);
    }

    #[test]
    fn a_crashed_file_is_repairable_and_loses_nothing_flushed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("crash.wav");
        {
            let mut w = StreamingWavWriter::create(&p, 16_000).unwrap();
            w.append(&vec![7i16; 32_000]).unwrap(); // 2 s
            w.flush_header().unwrap();
            // KEIN finalize — Drop simuliert den Crash
        }
        let ms = repair_orphan_wav(&p)
            .unwrap()
            .expect("muss reparierbar sein");
        assert_eq!(ms, 2000);
        let r = hound::WavReader::open(&p).unwrap();
        assert_eq!(r.len(), 32_000);
    }

    #[test]
    fn append_refuses_to_cross_the_riff_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("huge.wav");
        let mut w = StreamingWavWriter::create(&p, 16_000).unwrap();

        // Put the writer just short of the limit without writing gigabytes.
        let frames_near_limit = MAX_DATA_BYTES / BYTES_PER_SAMPLE as u64 - 10;
        w.set_frames_for_test(frames_near_limit);

        // A large chunk that would cross the limit must be refused, and
        // nothing should be written.
        let big_chunk = vec![0i16; 1_000];
        assert!(w.append(&big_chunk).is_err());
        assert_eq!(w.frames_written(), frames_near_limit);
    }

    #[test]
    fn position_ms_follows_the_frames_appended_so_far() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("pos.wav");
        let mut w = StreamingWavWriter::create(&p, 16_000).unwrap();
        assert_eq!(w.position_ms(), 0);
        w.append(&vec![0i16; 8_000]).unwrap();
        assert_eq!(w.frames_written(), 8_000);
        assert_eq!(w.position_ms(), 500);
        w.append(&vec![0i16; 8_000 + 16]).unwrap();
        assert_eq!(w.position_ms(), 1_001); // 16 Frames = 1 ms bei 16 kHz
    }

    #[test]
    fn repair_rejects_non_wav_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("junk.wav");
        std::fs::write(&p, b"not a wav").unwrap();
        assert!(repair_orphan_wav(&p).unwrap().is_none());
    }
}
