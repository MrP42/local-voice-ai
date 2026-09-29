//! M3-P3a: Nachbearbeitung der Sprecher-Turns (rein, ohne I/O).
//!
//! Sortformer liefert Turns, die an jeder Wortpause abreissen. Die
//! AMI-Referenz (und das Ohr) zaehlen solche Pausen als Sprache. Deshalb
//! werden je Sprecher Luecken bis `gap_ms` geschlossen und jeder Turn um
//! `pad_ms` nach vorn und hinten verlaengert (Spike `postproc.py`: Pad zuerst,
//! dann Luecken messen; so gemessen: AMI 35 % -> 15 % DER).

use super::Turn;

/// Roher Turn aus dem Modell: Zeiten in ms, Sprecher 1-basiert (0 oder
/// negativ = keine Zuordnung).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawTurn {
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub speaker_id: i32,
}

/// Unbrauchbare Modellzeilen verwerfen und auf die Audiodauer begrenzen:
/// Sprecher <= 0, negative/umgedrehte Zeiten, leere Turns.
pub fn sanitize(raw: &[RawTurn], audio_ms: u64) -> Vec<Turn> {
    raw.iter()
        .filter(|r| r.speaker_id > 0)
        .filter_map(|r| {
            let start = r.t0_ms.max(0) as u64;
            let end = (r.t1_ms.max(0) as u64).min(audio_ms);
            (end > start).then_some(Turn {
                start_ms: start,
                end_ms: end,
                speaker: r.speaker_id as u32,
            })
        })
        .collect()
}

/// Je Sprecher: jeden Turn um `pad_ms` verlaengern (vorn nicht unter 0,
/// hinten nicht ueber `audio_ms`), dann Turns verbinden, deren Abstand
/// hoechstens `gap_ms` betraegt. Turns verschiedener Sprecher bleiben
/// unberuehrt (Ueberlappung ist erlaubt). Ergebnis nach Start sortiert.
pub fn close_gaps(turns: &[Turn], gap_ms: u64, pad_ms: u64, audio_ms: u64) -> Vec<Turn> {
    let mut speakers: Vec<u32> = turns.iter().map(|t| t.speaker).collect();
    speakers.sort_unstable();
    speakers.dedup();
    let mut out = Vec::with_capacity(turns.len());
    for spk in speakers {
        let mut own: Vec<(u64, u64)> = turns
            .iter()
            .filter(|t| t.speaker == spk)
            .map(|t| {
                (
                    t.start_ms.saturating_sub(pad_ms),
                    t.end_ms.saturating_add(pad_ms).min(audio_ms.max(t.end_ms)),
                )
            })
            .collect();
        own.sort_unstable();
        let mut cur: Option<(u64, u64)> = None;
        for (s, e) in own {
            cur = match cur {
                Some((cs, ce)) if s <= ce.saturating_add(gap_ms) => Some((cs, ce.max(e))),
                Some((cs, ce)) => {
                    out.push(Turn {
                        start_ms: cs,
                        end_ms: ce,
                        speaker: spk,
                    });
                    Some((s, e))
                }
                None => Some((s, e)),
            };
        }
        if let Some((cs, ce)) = cur {
            out.push(Turn {
                start_ms: cs,
                end_ms: ce,
                speaker: spk,
            });
        }
    }
    out.retain(|t| t.end_ms > t.start_ms);
    sort_turns(&mut out);
    out
}

/// Sprecher 1..n in Ankunftsreihenfolge umnummerieren (wer zuerst spricht,
/// ist Sprecher 1). Die Modell-IDs sind Slot-Nummern ohne Bedeutung.
pub fn renumber_by_arrival(turns: &[Turn]) -> Vec<Turn> {
    let mut sorted = turns.to_vec();
    sort_turns(&mut sorted);
    let mut order: Vec<u32> = Vec::new();
    for t in &sorted {
        if !order.contains(&t.speaker) {
            order.push(t.speaker);
        }
    }
    sorted
        .into_iter()
        .map(|t| Turn {
            speaker: order.iter().position(|&s| s == t.speaker).unwrap_or(0) as u32 + 1,
            ..t
        })
        .collect()
}

/// Die ganze Nachbearbeitung wie im Enddurchlauf: bereinigen, Luecken
/// schliessen, umnummerieren.
pub fn apply(raw: &[RawTurn], gap_ms: u64, pad_ms: u64, audio_ms: u64) -> Vec<Turn> {
    let clean = sanitize(raw, audio_ms);
    renumber_by_arrival(&close_gaps(&clean, gap_ms, pad_ms, audio_ms))
}

/// Nur bereinigen und umnummerieren (fuer die Messung "roh").
pub fn apply_raw(raw: &[RawTurn], audio_ms: u64) -> Vec<Turn> {
    renumber_by_arrival(&sanitize(raw, audio_ms))
}

fn sort_turns(turns: &mut [Turn]) {
    turns.sort_by_key(|t| (t.start_ms, t.end_ms, t.speaker));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(start_ms: u64, end_ms: u64, speaker: u32) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    fn r(t0_ms: i64, t1_ms: i64, speaker_id: i32) -> RawTurn {
        RawTurn {
            t0_ms,
            t1_ms,
            speaker_id,
        }
    }

    #[test]
    fn a_short_pause_of_one_speaker_is_closed() {
        let got = close_gaps(&[t(0, 1000, 1), t(1800, 3000, 1)], 1000, 0, 10_000);
        assert_eq!(got, vec![t(0, 3000, 1)]);
    }

    #[test]
    fn a_long_pause_stays_a_pause() {
        let got = close_gaps(&[t(0, 1000, 1), t(2500, 3000, 1)], 1000, 0, 10_000);
        assert_eq!(got, vec![t(0, 1000, 1), t(2500, 3000, 1)]);
    }

    #[test]
    fn padding_is_applied_before_the_gap_is_measured() {
        // 1,2 s Luecke; mit je 100 ms Pad bleiben 1,0 s -> wird geschlossen
        // (wie postproc.py: erst verlaengern, dann verbinden).
        let got = close_gaps(&[t(1000, 2000, 1), t(3200, 4000, 1)], 1000, 100, 10_000);
        assert_eq!(got, vec![t(900, 4100, 1)]);
    }

    #[test]
    fn padding_never_leaves_the_audio() {
        let got = close_gaps(&[t(50, 1000, 1), t(9000, 9980, 2)], 1000, 100, 10_000);
        assert_eq!(got, vec![t(0, 1100, 1), t(8900, 10_000, 2)]);
    }

    #[test]
    fn gaps_are_closed_per_speaker_and_overlap_survives() {
        // Sprecher 2 redet in die Pause von Sprecher 1: beide Turns bleiben,
        // Sprecher 1 wird trotzdem verbunden (Ueberlappung ist erlaubt).
        let got = close_gaps(
            &[t(0, 1000, 1), t(1200, 1600, 2), t(1500, 2500, 1)],
            1000,
            0,
            10_000,
        );
        assert_eq!(got, vec![t(0, 2500, 1), t(1200, 1600, 2)]);
    }

    #[test]
    fn invalid_model_rows_are_dropped_and_clamped() {
        let got = sanitize(
            &[
                r(0, 500, 0),         // keine Zuordnung
                r(900, 800, 1),       // umgedreht
                r(-200, 300, 2),      // negativer Start
                r(9500, 12_000, 3),   // ueber das Ende hinaus
                r(10_500, 11_000, 1), // ganz hinter dem Ende
                r(100, 100, 1),       // leer
            ],
            10_000,
        );
        assert_eq!(got, vec![t(0, 300, 2), t(9500, 10_000, 3)]);
    }

    #[test]
    fn speakers_are_numbered_by_first_appearance() {
        let got = renumber_by_arrival(&[t(5000, 6000, 1), t(0, 1000, 3), t(2000, 3000, 1)]);
        assert_eq!(got, vec![t(0, 1000, 1), t(2000, 3000, 2), t(5000, 6000, 2)]);
    }

    #[test]
    fn apply_runs_the_whole_chain_and_empty_stays_empty() {
        assert!(apply(&[], 1000, 100, 10_000).is_empty());
        let got = apply(
            &[r(3000, 4000, 2), r(4500, 5000, 2), r(0, 1000, 4)],
            1000,
            100,
            10_000,
        );
        assert_eq!(got, vec![t(0, 1100, 1), t(2900, 5100, 2)]);
    }
}
