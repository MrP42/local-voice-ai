//! Reine Logik der Zusammenführungs-Vorschläge (Konzept Stimmprofile, 4A):
//! welche Ausschnitte je Sprecher, Schwerpunkt der Stimmabdrücke, Ähnlichkeit
//! und Auswahl der Paare. Kein Modell, keine Dateien.

use crate::managers::meetings::diarize::Turn;
use std::collections::BTreeMap;

/// Ein Ausschnitt muss nach dem Beschneiden so lang sein: 200 Rahmen, siehe
/// `super::SEGMENT_FRAMES`.
pub const MIN_CLIP_MS: u64 = 2_020;
/// Längere Ausschnitte werden geteilt (Rechenzeit, gleichmäßige Gewichte; 1000
/// Rahmen). Ein Rest unter [`MIN_CLIP_MS`] bleibt am Stück, das dann bis 12 s lang ist.
pub const MAX_CLIP_MS: u64 = 10_020;
/// Höchstens so viel Sprache je Sprecher.
pub const MAX_SPEECH_MS: u64 = 60_000;
/// Rand je Seite, an dem oft noch der Vorredner zu hören ist.
pub const EDGE_MS: u64 = 200;
/// Schwelle (Kosinus). Kalibriert an Mini-LibriSpeech dev-clean-2 (26 Sprecher,
/// je zwei getrennte Hälften): ab 8 s Sprache je Seite liegen alle gleichen
/// Sprecher über 0,67, alle verschiedenen unter 0,52. Siehe
/// `docs/konzept-stimmprofile.md`.
pub const DEFAULT_THRESHOLD: f32 = 0.65;
/// Weniger Sprache gibt keinen Vorschlag (bei 4 s streuen gleiche Sprecher bis 0,33).
pub const MIN_VOICE_MS: u64 = 8_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl Clip {
    pub fn len_ms(&self) -> u64 {
        self.end_ms - self.start_ms
    }
}

/// Zeitspannen von `speaker`, in denen kein anderer Sprecher spricht.
fn solo_spans(turns: &[Turn], speaker: u32) -> Vec<(u64, u64)> {
    let others: Vec<(u64, u64)> = turns
        .iter()
        .filter(|t| t.speaker != speaker)
        .map(|t| (t.start_ms, t.end_ms))
        .collect();
    // Aneinanderstoßende Turns desselben Sprechers zuerst verbinden.
    let mut own: Vec<(u64, u64)> = turns
        .iter()
        .filter(|t| t.speaker == speaker)
        .map(|t| (t.start_ms, t.end_ms))
        .collect();
    own.sort_unstable();
    let mut joined: Vec<(u64, u64)> = Vec::new();
    for (s, e) in own {
        match joined.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => joined.push((s, e)),
        }
    }
    let mut out = Vec::new();
    for (ts, te) in joined {
        let mut pieces = vec![(ts, te)];
        for &(os, oe) in &others {
            pieces = pieces
                .into_iter()
                .flat_map(|(s, e)| {
                    if oe <= s || os >= e {
                        vec![(s, e)]
                    } else {
                        let mut v = Vec::new();
                        if os > s {
                            v.push((s, os));
                        }
                        if oe < e {
                            v.push((oe, e));
                        }
                        v
                    }
                })
                .collect();
        }
        out.extend(pieces);
    }
    out
}

/// Ausschnitte für den Stimmabdruck von `speaker`: nur Stellen ohne
/// Überlappung, Ränder beschnitten, mindestens [`MIN_CLIP_MS`], die längsten
/// zuerst, zusammen höchstens [`MAX_SPEECH_MS`], je Stück etwa
/// [`MAX_CLIP_MS`]. Leer, wenn der Sprecher zu wenig allein spricht.
pub fn clips_for_speaker(turns: &[Turn], speaker: u32) -> Vec<Clip> {
    let mut spans: Vec<(u64, u64)> = solo_spans(turns, speaker)
        .into_iter()
        .filter_map(|(s, e)| {
            let (s, e) = (s + EDGE_MS, e.saturating_sub(EDGE_MS));
            (e > s && e - s >= MIN_CLIP_MS).then_some((s, e))
        })
        .collect();
    spans.sort_by(|a, b| (b.1 - b.0).cmp(&(a.1 - a.0)).then(a.0.cmp(&b.0)));
    let mut out = Vec::new();
    let mut left = MAX_SPEECH_MS;
    for (s, e) in spans {
        let mut start = s;
        while left >= MIN_CLIP_MS && e - start >= MIN_CLIP_MS {
            let len = (e - start).min(MAX_CLIP_MS).min(left);
            // Ein Rest unter der Mindestlänge geht an das vorige Stück.
            let len = if e - start - len < MIN_CLIP_MS && e - start <= MAX_CLIP_MS + MIN_CLIP_MS {
                (e - start).min(left)
            } else {
                len
            };
            out.push(Clip {
                start_ms: start,
                end_ms: start + len,
            });
            start += len;
            left -= len;
        }
        if left < MIN_CLIP_MS {
            break;
        }
    }
    out.sort_by_key(|c| c.start_ms);
    out
}

fn normalized(v: &[f32]) -> Option<Vec<f32>> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (n.is_finite() && n > 1e-6).then(|| v.iter().map(|x| x / n).collect())
}

/// Nach Dauer gewichteter Mittelwert der normierten Abdrücke, wieder normiert.
pub fn centroid(parts: &[(Vec<f32>, u64)]) -> Option<Vec<f32>> {
    let dim = parts.first()?.0.len();
    let mut sum = vec![0f32; dim];
    let mut any = false;
    for (v, w) in parts {
        if v.len() != dim {
            continue;
        }
        let Some(n) = normalized(v) else { continue };
        for (s, x) in sum.iter_mut().zip(n) {
            *s += x * *w as f32;
        }
        any = true;
    }
    if !any {
        return None;
    }
    normalized(&sum)
}

/// Kosinus zweier normierter Vektoren.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Stimmabdruck eines Sprechers in einer Besprechung (nur im Speicher).
#[derive(Clone, Debug, PartialEq)]
pub struct Voice {
    pub channel: u8,
    pub speaker: u32,
    pub vector: Vec<f32>,
    pub speech_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pair {
    pub channel: u8,
    pub a: u32,
    pub b: u32,
    pub score: f32,
}

/// Paare derselben Spur mit Ähnlichkeit ab `threshold`, die beste zuerst. Nur
/// Sprecher mit mindestens [`MIN_VOICE_MS`] Sprache.
/// Jeder Sprecher steht in höchstens einem Paar. `skip(channel, a, b)` mit
/// `a < b` schließt ein Paar aus (verworfen, beide schon benannt).
pub fn pairs(voices: &[Voice], threshold: f32, skip: impl Fn(u8, u32, u32) -> bool) -> Vec<Pair> {
    let mut by_channel: BTreeMap<u8, Vec<&Voice>> = BTreeMap::new();
    for v in voices.iter().filter(|v| v.speech_ms >= MIN_VOICE_MS) {
        by_channel.entry(v.channel).or_default().push(v);
    }
    let mut all = Vec::new();
    for (channel, list) in by_channel {
        for (i, x) in list.iter().enumerate() {
            for y in &list[i + 1..] {
                let (a, b) = if x.speaker < y.speaker {
                    (x.speaker, y.speaker)
                } else {
                    (y.speaker, x.speaker)
                };
                if a == b || skip(channel, a, b) {
                    continue;
                }
                let score = cosine(&x.vector, &y.vector);
                if score >= threshold {
                    all.push(Pair {
                        channel,
                        a,
                        b,
                        score,
                    });
                }
            }
        }
    }
    all.sort_by(|p, q| q.score.total_cmp(&p.score));
    let mut used: Vec<(u8, u32)> = Vec::new();
    all.into_iter()
        .filter(|p| {
            if used.contains(&(p.channel, p.a)) || used.contains(&(p.channel, p.b)) {
                return false;
            }
            used.push((p.channel, p.a));
            used.push((p.channel, p.b));
            true
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: u64, e: u64, speaker: u32) -> Turn {
        Turn {
            start_ms: s,
            end_ms: e,
            speaker,
        }
    }

    #[test]
    fn clips_skip_overlap_and_trim_edges() {
        let turns = [t(0, 5_000, 1), t(4_000, 9_000, 2), t(10_000, 11_000, 1)];
        // Sprecher 1 allein: 0..4000 -> beschnitten 200..3800; 10_000..11_000 zu kurz.
        assert_eq!(
            clips_for_speaker(&turns, 1),
            vec![Clip {
                start_ms: 200,
                end_ms: 3_800
            }]
        );
        // Sprecher 2 allein: 5000..9000 -> 5200..8800.
        assert_eq!(
            clips_for_speaker(&turns, 2),
            vec![Clip {
                start_ms: 5_200,
                end_ms: 8_800
            }]
        );
        assert!(clips_for_speaker(&turns, 3).is_empty());
    }

    #[test]
    fn touching_turns_of_one_speaker_count_together() {
        let turns = [t(0, 1_800, 1), t(1_800, 3_600, 1)];
        assert_eq!(
            clips_for_speaker(&turns, 1),
            vec![Clip {
                start_ms: 200,
                end_ms: 3_400
            }]
        );
    }

    #[test]
    fn long_turns_are_split_and_capped() {
        let turns = [t(0, 100_000, 1)];
        let clips = clips_for_speaker(&turns, 1);
        let total: u64 = clips.iter().map(Clip::len_ms).sum();
        assert_eq!(total, MAX_SPEECH_MS);
        assert!(clips
            .iter()
            .all(|c| c.len_ms() >= MIN_CLIP_MS && c.len_ms() <= MAX_CLIP_MS));
        // 11 s: kein Rest unter 2 s, also ein Stück von 11 s.
        let clips = clips_for_speaker(&[t(0, 11_400, 1)], 1);
        assert_eq!(
            clips,
            vec![Clip {
                start_ms: 200,
                end_ms: 11_200
            }]
        );
        // 15 s: 10,02 s + 4,58 s.
        let clips = clips_for_speaker(&[t(0, 15_000, 1)], 1);
        assert_eq!(
            clips.iter().map(Clip::len_ms).collect::<Vec<_>>(),
            [10_020, 4_580]
        );
    }

    #[test]
    fn longest_turns_are_used_first() {
        let mut turns = vec![t(0, 3_000, 1)];
        for k in 0..8 {
            turns.push(t(10_000 + k * 20_000, 20_000 + k * 20_000, 1));
        }
        let clips = clips_for_speaker(&turns, 1);
        assert!(clips.iter().all(|c| c.start_ms >= 10_000), "{clips:?}");
        assert_eq!(clips.iter().map(Clip::len_ms).sum::<u64>(), MAX_SPEECH_MS);
    }

    #[test]
    fn centroid_weights_by_duration_and_normalizes() {
        let c = centroid(&[(vec![2.0, 0.0], 3_000), (vec![0.0, 5.0], 1_000)]).unwrap();
        let n = (c[0] * c[0] + c[1] * c[1]).sqrt();
        assert!((n - 1.0).abs() < 1e-5);
        assert!((c[0] / c[1] - 3.0).abs() < 1e-4);
        assert!(centroid(&[]).is_none());
        assert!(centroid(&[(vec![0.0, 0.0], 1_000)]).is_none());
    }

    fn v(channel: u8, speaker: u32, x: f32, y: f32) -> Voice {
        let n = (x * x + y * y).sqrt();
        Voice {
            channel,
            speaker,
            vector: vec![x / n, y / n],
            speech_ms: 10_000,
        }
    }

    #[test]
    fn pairs_stay_in_their_channel_and_use_each_speaker_once() {
        let voices = [
            v(1, 1, 1.0, 0.0),
            v(1, 3, 0.95, 0.1),
            v(1, 2, 0.9, 0.2),
            v(1, 4, 0.0, 1.0),
            v(0, 1, 1.0, 0.0),
        ];
        let p = pairs(&voices, 0.7, |_, _, _| false);
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!((p[0].channel, p[0].a, p[0].b), (1, 1, 3));
        // Ausgeschlossen: dann kommt das nächste Paar.
        let p = pairs(&voices, 0.7, |c, a, b| (c, a, b) == (1, 1, 3));
        assert_eq!((p[0].a, p[0].b), (2, 3));
        assert_eq!(
            p.len(),
            1,
            "1 und 2 wären auch ähnlich, 2 ist aber vergeben: {p:?}"
        );
        assert!(pairs(&voices, 0.999, |_, _, _| false).is_empty());
        // Zu wenig Sprache: kein Paar.
        let mut short = voices.to_vec();
        short[1].speech_ms = MIN_VOICE_MS - 1;
        let p = pairs(&short, 0.7, |_, _, _| false);
        assert_eq!((p[0].a, p[0].b), (1, 2));
    }
}
