//! M8 meetings: deterministic speaking shares. Pure aggregation over the
//! already-stored segment durations — no diarization, no heuristics beyond
//! "sum each speaker's segment time, divide by the total". Without a
//! diarization (`speaker_index` unset) a speaker is a whole channel and the
//! label is the per-channel display fallback of M8; with one (M3-P3b) every
//! (channel, speaker) gets its own share and the label comes from the
//! `SpeakerDirectory` (name, else "Gegenseite 2"). The frontend is expected
//! to translate via `channel`, not to rely on `label` for logic.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::speakers::SpeakerDirectory;
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

/// Redeanteile aus Segmentdauern, ohne Namen: Label je Kanal ("Ich" /
/// "Gegenseite" / "Aufnahme"), bei Sprechertrennung je Sprecher ("Gegenseite
/// 2"). Die Labels sind Anzeige-Fallbacks; das Frontend übersetzt über channel.
// Die App ruft `speaking_shares_with` (mit den Namen der Besprechung); diese
// Fassung ohne Namen bleibt fuer Aufrufer ohne Store und fuer die Tests.
#[allow(dead_code)]
pub fn speaking_shares(segments: &[StoredSegment]) -> Vec<SpeakerShare> {
    speaking_shares_with(segments, &SpeakerDirectory::from_segments(segments))
}

/// Wie [`speaking_shares`], mit den Namen der Besprechung (M3-P3b): je
/// (Kanal, Sprecher) ein Anteil, in der Reihenfolge des ersten Auftretens.
/// Segmente ohne `speaker_index` (Kanal ohne Sprechertrennung, "Ich") bilden
/// je Kanal einen eigenen Anteil.
pub fn speaking_shares_with(
    segments: &[StoredSegment],
    labels: &SpeakerDirectory,
) -> Vec<SpeakerShare> {
    if segments.is_empty() {
        return Vec::new();
    }

    let mut order: Vec<(u8, Option<u32>)> = Vec::new();
    let mut speech_ms_by_speaker: std::collections::HashMap<(u8, Option<u32>), u64> =
        std::collections::HashMap::new();
    for segment in segments {
        let duration = segment.end_ms.saturating_sub(segment.start_ms);
        let key = (segment.channel, segment.speaker_index);
        *speech_ms_by_speaker.entry(key).or_insert(0) += duration;
        if !order.contains(&key) {
            order.push(key);
        }
    }

    let total_ms: u64 = speech_ms_by_speaker.values().sum();
    if total_ms == 0 {
        return Vec::new();
    }

    order
        .into_iter()
        .map(|key| {
            let speech_ms = speech_ms_by_speaker[&key];
            SpeakerShare {
                label: labels.label_for(key.0, key.1),
                channel: key.0,
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

    fn spk(channel: u8, speaker: Option<u32>, start_ms: u64, end_ms: u64) -> StoredSegment {
        StoredSegment {
            speaker_index: speaker,
            ..seg(channel, start_ms, end_ms)
        }
    }

    #[test]
    fn shares_run_per_speaker_and_use_names_when_there_are_any() {
        // Ich (Kanal 0, keine Sprechertrennung) 4 s, Gegenseite: Sprecher 1 4 s, Sprecher 2 2 s.
        let segs = vec![
            spk(0, None, 0, 4_000),
            spk(1, Some(1), 4_000, 8_000),
            spk(1, Some(2), 8_000, 10_000),
            spk(1, Some(1), 10_000, 11_000),
        ];
        let shares = speaking_shares(&segs);
        let labels: Vec<&str> = shares.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["Ich", "Gegenseite 1", "Gegenseite 2"]);
        assert_eq!(shares[1].speech_ms, 5_000);
        assert!(shares.iter().all(|s| s.channel <= 1));
        assert!((shares.iter().map(|s| s.percent).sum::<f64>() - 100.0).abs() < 0.01);
        // Mit Namen: der Name ersetzt die Nummer, die Anteile bleiben gleich.
        let dir = SpeakerDirectory::new([((1, 2), "Anna Berg".to_string())], true);
        let named = speaking_shares_with(&segs, &dir);
        assert_eq!(named[2].label, "Anna Berg");
        assert_eq!(named[2].speech_ms, shares[2].speech_ms);
    }

    #[test]
    fn a_diarized_import_has_one_share_per_person() {
        let segs = vec![
            spk(2, Some(1), 0, 3_000),
            spk(2, Some(2), 3_000, 4_000),
            spk(2, Some(3), 4_000, 8_000),
        ];
        let shares = speaking_shares(&segs);
        let labels: Vec<&str> = shares.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["Person 1", "Person 2", "Person 3"]);
        assert!((shares[2].percent - 50.0).abs() < 0.01);
    }

    #[test]
    fn channel_order_follows_first_appearance() {
        let segs = vec![seg(1, 0, 1_000), seg(0, 1_000, 2_000)];
        let shares = speaking_shares(&segs);
        assert_eq!(shares[0].channel, 1);
        assert_eq!(shares[1].channel, 0);
    }
}
