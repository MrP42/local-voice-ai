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

// ------------------------------------------------------------------ Export --

/// M6-P6a: Anzeigename des Sprechers fuer Untertitel und Transkript-Export.
/// `names` sind die vom Nutzer vergebenen Namen je `speaker_index` (leer, bis
/// die Sprecherverwaltung sie liefert). Rueckgabe leer = kein Praefix: ein
/// Mischkanal ohne Sprechertrennung (Import) wuerde sonst jede Zeile mit
/// "Aufnahme:" beginnen lassen.
pub fn speaker_label(
    segment: &StoredSegment,
    names: &std::collections::BTreeMap<u32, String>,
) -> String {
    if segment.channel == 0 {
        return "Ich".to_string();
    }
    match segment.speaker_index {
        Some(index) => names
            .get(&index)
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Sprecher {}", index + 1)),
        None if segment.channel == 1 => "Gegenseite".to_string(),
        None => String::new(),
    }
}

/// Zeitmarke `HH:MM:SS<sep>mmm` (SRT: `,`, VTT: `.`).
fn timecode(ms: u64, separator: char) -> String {
    format!(
        "{:02}:{:02}:{:02}{}{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1_000) % 60,
        separator,
        ms % 1_000
    )
}

/// Cue-Text: eine Leerzeile beendet in beiden Formaten den Cue, deshalb
/// werden Leerzeilen im Segmenttext zu einem Zeilenumbruch.
fn cue_text(label: &str, text: &str) -> Option<String> {
    let body = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if body.is_empty() {
        return None;
    }
    let label = label.trim();
    Some(if label.is_empty() {
        body
    } else {
        format!("{label}: {body}")
    })
}

/// Segmente in Zeitreihenfolge; Segmente ohne Text entfallen (ein leerer Cue
/// waere in SRT nicht von einem Blockende zu unterscheiden).
fn cues<'a>(segments: &'a [StoredSegment]) -> Vec<&'a StoredSegment> {
    let mut list: Vec<&StoredSegment> = segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    list.sort_by_key(|s| (s.start_ms, s.segment_index));
    list
}

/// Transkript als SRT. `label` liefert den Sprecher-Praefix je Segment
/// (leer = keiner), z. B. "Ich:" / Name.
pub fn segments_to_srt(
    segments: &[StoredSegment],
    label: &dyn Fn(&StoredSegment) -> String,
) -> String {
    let mut out = String::new();
    for (i, segment) in cues(segments).into_iter().enumerate() {
        let Some(text) = cue_text(&label(segment), &segment.text) else {
            continue;
        };
        let end = segment.end_ms.max(segment.start_ms);
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            timecode(segment.start_ms, ','),
            timecode(end, ','),
            text
        ));
    }
    out
}

/// Transkript als WebVTT (Punkt vor den Millisekunden, `&`/`<` maskiert).
pub fn segments_to_vtt(
    segments: &[StoredSegment],
    label: &dyn Fn(&StoredSegment) -> String,
) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for segment in cues(segments) {
        let Some(text) = cue_text(&label(segment), &segment.text) else {
            continue;
        };
        let text = text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let end = segment.end_ms.max(segment.start_ms);
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            timecode(segment.start_ms, '.'),
            timecode(end, '.'),
            text
        ));
    }
    out
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

    fn seg(index: u32, start: u64, end: u64, channel: u8, text: &str) -> StoredSegment {
        StoredSegment {
            segment_index: index,
            text: text.to_string(),
            start_ms: start,
            end_ms: end,
            channel,
            speaker_index: None,
            words: None,
        }
    }

    fn no_names() -> std::collections::BTreeMap<u32, String> {
        std::collections::BTreeMap::new()
    }

    #[test]
    fn srt_export_hat_index_komma_und_praefix() {
        let segs = vec![
            seg(0, 1_000, 3_500, 0, "Guten Morgen."),
            seg(1, 3_600_000 + 61_005, 3_600_000 + 65_000, 1, "Hallo."),
        ];
        let srt = segments_to_srt(&segs, &|s| speaker_label(s, &no_names()));
        assert_eq!(
            srt,
            "1\n00:00:01,000 --> 00:00:03,500\nIch: Guten Morgen.\n\n2\n01:01:01,005 --> 01:01:05,000\nGegenseite: Hallo.\n\n"
        );
    }

    #[test]
    fn vtt_export_hat_kopfzeile_punkt_und_maskiert_sonderzeichen() {
        let segs = vec![seg(0, 500, 2_000, 2, "Soll & Haben <neu>")];
        let vtt = segments_to_vtt(&segs, &|s| speaker_label(s, &no_names()));
        assert_eq!(
            vtt,
            "WEBVTT\n\n00:00:00.500 --> 00:00:02.000\nSoll &amp; Haben &lt;neu&gt;\n\n"
        );
    }

    /// Der eigene Import muss den eigenen Export wieder lesen koennen.
    #[test]
    fn srt_roundtrip_via_parse_subtitles() {
        let segs = vec![
            seg(0, 1_000, 3_500, 0, "Guten Morgen zusammen."),
            seg(1, 4_000, 6_000, 1, "Beginnen wir mit dem Status."),
            seg(2, 3_723_456, 3_725_000, 1, "Zeile eins\nZeile zwei"),
        ];
        // Ohne Praefix: Text, Start und Ende muessen exakt zurueckkommen.
        let plain = segments_to_srt(&segs, &|_| String::new());
        let back = parse_subtitles(&plain).unwrap();
        assert_eq!(back.len(), 3);
        for (a, b) in segs.iter().zip(&back) {
            assert_eq!((a.start_ms, a.end_ms), (b.start_ms, b.end_ms));
        }
        assert_eq!(back[0].text, "Guten Morgen zusammen.");
        assert_eq!(back[2].text, "Zeile eins Zeile zwei");
        // Mit Praefix steht er vorn im Text.
        let labelled = segments_to_srt(&segs, &|s| speaker_label(s, &no_names()));
        let back = parse_subtitles(&labelled).unwrap();
        assert_eq!(back[0].text, "Ich: Guten Morgen zusammen.");
        assert_eq!(back[1].text, "Gegenseite: Beginnen wir mit dem Status.");
    }

    #[test]
    fn vtt_roundtrip_via_parse_subtitles() {
        let segs = vec![
            seg(0, 500, 2_000, 0, "Hallo."),
            seg(1, 60_000, 62_250, 1, "Zweiter Satz."),
        ];
        let back = parse_subtitles(&segments_to_vtt(&segs, &|_| String::new())).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!((back[1].start_ms, back[1].end_ms), (60_000, 62_250));
        assert_eq!(back[1].text, "Zweiter Satz.");
    }

    #[test]
    fn leere_segmente_entfallen_und_die_reihenfolge_folgt_der_zeit() {
        let segs = vec![
            seg(1, 5_000, 6_000, 0, "spaeter"),
            seg(0, 1_000, 2_000, 0, "frueh"),
            seg(2, 3_000, 4_000, 0, "   "),
        ];
        let srt = segments_to_srt(&segs, &|_| String::new());
        assert!(srt.starts_with("1\n00:00:01,000"));
        assert!(srt.contains("2\n00:00:05,000"));
        assert!(
            !srt.contains("00:00:03,000"),
            "leerer Cue exportiert:\n{srt}"
        );
    }

    #[test]
    fn eine_leerzeile_im_text_beendet_den_cue_nicht() {
        let segs = vec![seg(0, 0, 1_000, 0, "Absatz eins\n\nAbsatz zwei")];
        let srt = segments_to_srt(&segs, &|_| String::new());
        assert_eq!(
            srt,
            "1\n00:00:00,000 --> 00:00:01,000\nAbsatz eins\nAbsatz zwei\n\n"
        );
    }

    #[test]
    fn das_ende_liegt_nie_vor_dem_start() {
        let srt = segments_to_srt(&[seg(0, 2_000, 1_000, 0, "x")], &|_| String::new());
        assert!(srt.contains("00:00:02,000 --> 00:00:02,000"), "{srt}");
    }

    #[test]
    fn sprecherlabel_nach_kanal_index_und_name() {
        let mut names = no_names();
        names.insert(1, "Anna Berg".to_string());
        assert_eq!(speaker_label(&seg(0, 0, 1, 0, "a"), &names), "Ich");
        assert_eq!(speaker_label(&seg(0, 0, 1, 1, "a"), &names), "Gegenseite");
        assert_eq!(speaker_label(&seg(0, 0, 1, 2, "a"), &names), "");
        let mut s = seg(0, 0, 1, 1, "a");
        s.speaker_index = Some(0);
        assert_eq!(speaker_label(&s, &names), "Sprecher 1");
        s.speaker_index = Some(1);
        assert_eq!(speaker_label(&s, &names), "Anna Berg");
        // Ein leerer Name faellt auf die Nummer zurueck.
        names.insert(1, "  ".to_string());
        assert_eq!(speaker_label(&s, &names), "Sprecher 2");
    }
}
