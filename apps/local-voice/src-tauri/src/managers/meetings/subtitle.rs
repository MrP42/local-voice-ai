//! M8 meetings: pure VTT/SRT subtitle parsing for file import. No I/O — the
//! caller reads the file, this turns its text into `StoredSegment`s on
//! `channel = 2` (`MixedCapture`, no speaker split available from subtitles).
//!
//! Line-based, not a full parser: a timecode line (`HH:MM:SS[.,]mmm -->
//! HH:MM:SS[.,]mmm`, optional trailing VTT cue settings ignored) opens a cue;
//! every non-empty line after it joins the cue text until the next blank
//! line. `WEBVTT` headers, `NOTE` blocks and SRT's numeric cue-index lines are
//! skipped. No timecode anywhere in the input is treated as "not a subtitle
//! file" — an explicit error beats a silent empty import.

use regex::{Captures, Regex};

use super::store::StoredSegment;

/// VTT uses `.` before milliseconds, SRT uses `,` — both accepted by `[.,]`.
/// Not anchored at the end so VTT cue settings (`align:start` etc.) after the
/// second timestamp don't prevent a match.
fn timecode_regex() -> Regex {
    Regex::new(r"^(\d{2}):(\d{2}):(\d{2})[.,](\d{3})\s*-->\s*(\d{2}):(\d{2}):(\d{2})[.,](\d{3})")
        .expect("static regex")
}

fn captured_ms(caps: &Captures, first_group: usize) -> u64 {
    let h: u64 = caps[first_group].parse().unwrap_or(0);
    let m: u64 = caps[first_group + 1].parse().unwrap_or(0);
    let s: u64 = caps[first_group + 2].parse().unwrap_or(0);
    let ms: u64 = caps[first_group + 3].parse().unwrap_or(0);
    h * 3_600_000 + m * 60_000 + s * 1_000 + ms
}

/// VTT- oder SRT-Text -> Segmente (channel = 2 / MixedCapture).
pub fn parse_subtitles(content: &str) -> Result<Vec<StoredSegment>, String> {
    let time_re = timecode_regex();
    let lines: Vec<&str> = content.lines().collect();
    let mut segments = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if line.is_empty() || line.starts_with("WEBVTT") {
            i += 1;
            continue;
        }

        if line.starts_with("NOTE") {
            i += 1;
            while i < lines.len() && !lines[i].trim().is_empty() {
                i += 1;
            }
            continue;
        }

        if let Some(caps) = time_re.captures(line) {
            let start_ms = captured_ms(&caps, 1);
            let end_ms = captured_ms(&caps, 5);
            i += 1;
            let mut text_lines: Vec<&str> = Vec::new();
            while i < lines.len() && !lines[i].trim().is_empty() {
                text_lines.push(lines[i].trim());
                i += 1;
            }
            segments.push(StoredSegment {
                segment_index: segments.len() as u32,
                text: text_lines.join(" "),
                start_ms,
                end_ms,
                channel: 2,
                speaker_index: None,
                words: None,
            });
            continue;
        }

        // SRT cue-index line ("1", "2", ...) or anything else we don't
        // recognize — tolerated, skipped.
        i += 1;
    }

    if segments.is_empty() {
        return Err("Kein gültiges Untertitelformat erkannt (VTT/SRT erwartet)".to_string());
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_blocks_become_segments_with_ms_times() {
        let srt = "1\n00:00:01,000 --> 00:00:03,500\nGuten Morgen zusammen.\n\n2\n00:00:04,000 --> 00:00:06,000\nBeginnen wir mit dem Status.\n";
        let segs = parse_subtitles(srt).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!((segs[0].start_ms, segs[0].end_ms), (1_000, 3_500));
        assert_eq!(segs[1].text, "Beginnen wir mit dem Status.");
        assert!(segs.iter().all(|s| s.channel == 2));
    }

    #[test]
    fn vtt_header_and_cue_settings_are_tolerated() {
        let vtt = "WEBVTT\n\n00:00:00.500 --> 00:00:02.000 align:start\nHallo.\n\nNOTE irrelevant\n\n00:01:00.000 --> 00:01:02.250\nZweiter Satz.\n";
        let segs = parse_subtitles(vtt).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[1].start_ms, 60_000);
        assert_eq!(segs[0].text, "Hallo.");
    }

    #[test]
    fn garbage_is_an_error_not_an_empty_import() {
        assert!(parse_subtitles("kein untertitelformat").is_err());
    }

    #[test]
    fn multiline_cues_join_with_spaces() {
        let srt = "1\n00:00:01,000 --> 00:00:03,000\nZeile eins\nZeile zwei\n";
        assert_eq!(
            parse_subtitles(srt).unwrap()[0].text,
            "Zeile eins Zeile zwei"
        );
    }
}
