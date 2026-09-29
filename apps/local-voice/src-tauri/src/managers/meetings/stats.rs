//! M8 meetings: deterministic speaking shares. Pure aggregation over the
//! already-stored segment durations — no diarization, no heuristics beyond
//! "sum each channel's segment time, divide by the total". M8 labels are
//! per-channel display fallbacks only; the frontend is expected to translate
//! via `channel`, not to rely on `label` for logic.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::store::StoredSegment;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct SpeakerShare {
    pub label: String,
    pub channel: u8,
    pub speech_ms: u64,
    pub percent: f64,
}

/// M8: one fixed label per channel (0=DirectMic/"Ich", 1=RemoteParty/
/// "Gegenseite", 2=MixedCapture/"Aufnahme"); anything else falls back to its
/// channel number rather than panicking on unexpected data.
pub fn label_for_channel(channel: u8) -> String {
    match channel {
        0 => "Ich".to_string(),
        1 => "Gegenseite".to_string(),
        2 => "Aufnahme".to_string(),
        other => format!("Kanal {other}"),
    }
}

/// Redeanteile aus Segmentdauern. M8: Label je Kanal ("Ich" / "Gegenseite" /
/// "Aufnahme"). Die Labels sind Anzeige-Fallbacks; das Frontend übersetzt
/// über channel.
pub fn speaking_shares(segments: &[StoredSegment]) -> Vec<SpeakerShare> {
    if segments.is_empty() {
        return Vec::new();
    }

    let mut channels: Vec<u8> = Vec::new();
    let mut speech_ms_by_channel: std::collections::HashMap<u8, u64> =
        std::collections::HashMap::new();
    for segment in segments {
        let duration = segment.end_ms.saturating_sub(segment.start_ms);
        let entry = speech_ms_by_channel.entry(segment.channel).or_insert(0);
        *entry += duration;
        if !channels.contains(&segment.channel) {
            channels.push(segment.channel);
        }
    }

    let total_ms: u64 = speech_ms_by_channel.values().sum();
    if total_ms == 0 {
        return Vec::new();
    }

    channels
        .into_iter()
        .map(|channel| {
            let speech_ms = speech_ms_by_channel[&channel];
            SpeakerShare {
                label: label_for_channel(channel),
                channel,
                speech_ms,
                percent: (speech_ms as f64 / total_ms as f64) * 100.0,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(channel: u8, start_ms: u64, end_ms: u64) -> StoredSegment {
        StoredSegment {
            segment_index: 0,
            text: "x".into(),
            start_ms,
            end_ms,
            channel,
            speaker_index: None,
            words: None,
        }
    }

    #[test]
    fn shares_sum_to_100_and_split_by_channel() {
        let segs = vec![
            StoredSegment {
                segment_index: 0,
                text: "a".into(),
                start_ms: 0,
                end_ms: 6_000,
                channel: 0,
                speaker_index: None,
                words: None,
            },
            StoredSegment {
                segment_index: 1,
                text: "b".into(),
                start_ms: 6_000,
                end_ms: 8_000,
                channel: 1,
                speaker_index: None,
                words: None,
            },
        ];
        let shares = speaking_shares(&segs);
        assert_eq!(shares.len(), 2);
        assert_eq!(shares[0].speech_ms, 6_000);
        assert!((shares[0].percent - 75.0).abs() < 0.01);
        assert!((shares.iter().map(|s| s.percent).sum::<f64>() - 100.0).abs() < 0.01);
    }

    #[test]
    fn a_single_channel_import_yields_one_share_of_100() {
        let segs = vec![StoredSegment {
            segment_index: 0,
            text: "x".into(),
            start_ms: 0,
            end_ms: 1_000,
            channel: 2,
            speaker_index: None,
            words: None,
        }];
        let shares = speaking_shares(&segs);
        assert_eq!(shares.len(), 1);
        assert!((shares[0].percent - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn no_segments_no_shares_no_division_by_zero() {
        assert!(speaking_shares(&[]).is_empty());
    }

    #[test]
    fn channel_order_follows_first_appearance() {
        let segs = vec![seg(1, 0, 1_000), seg(0, 1_000, 2_000)];
        let shares = speaking_shares(&segs);
        assert_eq!(shares[0].channel, 1);
        assert_eq!(shares[1].channel, 0);
    }
}
