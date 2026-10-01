//! M3-P3b: Zuordnung Wort -> Sprecher (rein, ohne I/O). Entwurf `m3-sprecher.md`
//! 3.3.
//!
//! Eingabe: die Segmente eines Kanals und die Turns desselben Kanals
//! (`diarize::Turn`, Sprecher 1-basiert, Ueberlappung erlaubt). Ausgabe:
//! Segmente mit `speaker_index`, an Sprecherwechseln geteilt.
//!
//! - Wort mit Zeit: Sprecher mit der groessten Zeitueberlappung (Summe ueber
//!   seine Turns). Gleichstand: der Turn, der das Wortzentrum enthaelt. Ohne
//!   Ueberlappung gilt der naechste Turn hoechstens [`NEAR_TURN_MS`] entfernt,
//!   sonst der Sprecher des Vorgaengerworts (fuehrende Woerter: der erste
//!   aufgeloeste).
//! - Glaettung: ein Sprecherlauf mit weniger als [`MIN_RUN_WORDS`] Woertern
//!   UND unter [`MIN_RUN_MS`] geht an den Nachbarn (kein Flackern an Grenzen).
//! - Teilen: an jedem verbleibenden Wechsel entsteht ein eigenes Segment (Text
//!   = seine Woerter, Zeiten aus den Wortgrenzen, `words` aufgeteilt).
//! - Ohne `words` (Whisper-Endmodell, Altdaten): Sprecher = groesste
//!   Ueberlappung des ganzen Segments, KEIN Teilen; ohne Ueberlappung bleibt
//!   `speaker_index` leer.
//! - Ein Luecken-Platzhalter ("[Nicht transkribiert ...]") gehoert keiner
//!   Person und bleibt ohne Sprecher.
//!
//! `segment_index` fasst diese Funktion nicht an: geteilte Teile tragen den
//! Index des Ursprungssegments, der Aufrufer nummeriert neu (Epoche).

use std::collections::BTreeMap;

use super::Turn;
use crate::managers::meetings::import::is_gap_placeholder;
use crate::managers::meetings::store::StoredSegment;
use crate::managers::transcription::WordTime;

/// Ohne Ueberlappung zaehlt der naechste Turn, wenn er hoechstens so weit
/// (ms) vom Wort entfernt liegt.
pub const NEAR_TURN_MS: u64 = 500;
/// Ein Sprecherlauf unter dieser Wortzahl UND unter [`MIN_RUN_MS`] wird
/// dem Nachbarn zugeschlagen.
pub const MIN_RUN_WORDS: usize = 2;
pub const MIN_RUN_MS: u64 = 600;

fn overlap_ms(a0: u64, a1: u64, b0: u64, b1: u64) -> u64 {
    a1.min(b1).saturating_sub(a0.max(b0))
}

/// Abstand einer Zeitspanne zu einem Turn (0 = beruehrt/ueberlappt).
fn distance_ms(t: &Turn, start_ms: u64, end_ms: u64) -> u64 {
    // Liegt die Spanne vor dem Turn, ist nur der erste Term > 0, dahinter nur
    // der zweite, sonst beruehren oder ueberlappen sie sich.
    t.start_ms
        .saturating_sub(end_ms)
        .max(start_ms.saturating_sub(t.end_ms))
}

/// Sprecher einer Zeitspanne: groesste Ueberlappung (Summe je Sprecher),
/// bei Gleichstand der Turn mit dem Zentrum der Spanne, dann die kleinere
/// Sprecher-Nummer. Ohne Ueberlappung der naechste Turn, wenn er hoechstens
/// `near_ms` entfernt liegt; sonst `None`.
pub fn speaker_for_span(turns: &[Turn], start_ms: u64, end_ms: u64, near_ms: u64) -> Option<u32> {
    let end_ms = end_ms.max(start_ms);
    let mid = start_ms + (end_ms - start_ms) / 2;
    // Sprecher -> (Ueberlappung, Zentrum in einem seiner ueberlappenden Turns).
    let mut per: BTreeMap<u32, (u64, bool)> = BTreeMap::new();
    for t in turns {
        let ov = overlap_ms(start_ms, end_ms, t.start_ms, t.end_ms);
        if ov == 0 {
            continue;
        }
        let entry = per.entry(t.speaker).or_insert((0, false));
        entry.0 += ov;
        entry.1 |= t.start_ms <= mid && mid < t.end_ms;
    }
    let best = per
        .iter()
        // Ueberlappung, dann Zentrum, dann die KLEINERE Sprecher-Nummer
        // (deshalb `b` gegen `a`): eine totale Ordnung, kein echter Gleichstand.
        .max_by(|a, b| {
            a.1 .0
                .cmp(&b.1 .0)
                .then(a.1 .1.cmp(&b.1 .1))
                .then(b.0.cmp(a.0))
        })
        .map(|(spk, _)| *spk);
    if best.is_some() {
        return best;
    }
    turns
        .iter()
        .map(|t| (distance_ms(t, start_ms, end_ms), t.speaker))
        .filter(|(d, _)| *d <= near_ms)
        .min()
        .map(|(_, spk)| spk)
}

/// Ein zusammenhaengender Lauf von Woertern desselben Sprechers.
#[derive(Debug, Clone)]
struct Run {
    speaker: u32,
    words: Vec<WordTime>,
}

impl Run {
    fn start_ms(&self) -> u64 {
        self.words.first().map_or(0, |w| w.start_ms)
    }
    fn end_ms(&self) -> u64 {
        self.words.last().map_or(0, |w| w.end_ms)
    }
    fn span_ms(&self) -> u64 {
        self.end_ms().saturating_sub(self.start_ms())
    }
    fn is_short(&self) -> bool {
        self.words.len() < MIN_RUN_WORDS && self.span_ms() < MIN_RUN_MS
    }
}

/// Sprecher je Wort (mit Vorgaenger-Rueckfall); `None` nur, wenn kein einziges
/// Wort einem Turn zugeordnet werden kann.
fn word_speakers(words: &[WordTime], turns: &[Turn]) -> Option<Vec<u32>> {
    let mut out: Vec<Option<u32>> = words
        .iter()
        .map(|w| speaker_for_span(turns, w.start_ms, w.end_ms, NEAR_TURN_MS))
        .collect();
    let first = out.iter().flatten().next().copied()?;
    let mut last = first; // fuehrende Woerter: der erste aufgeloeste Sprecher
    Some(
        out.iter_mut()
            .map(|s| {
                if let Some(v) = *s {
                    last = v;
                }
                last
            })
            .collect(),
    )
}

fn runs_of(words: &[WordTime], speakers: &[u32]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (w, &spk) in words.iter().zip(speakers) {
        match runs.last_mut() {
            Some(r) if r.speaker == spk => r.words.push(w.clone()),
            _ => runs.push(Run {
                speaker: spk,
                words: vec![w.clone()],
            }),
        }
    }
    runs
}

fn merge_equal_neighbours(runs: &mut Vec<Run>) {
    let mut i = 1;
    while i < runs.len() {
        if runs[i - 1].speaker == runs[i].speaker {
            let next = runs.remove(i);
            runs[i - 1].words.extend(next.words);
        } else {
            i += 1;
        }
    }
}

/// Kurze Laeufe an den Nachbarn geben, bis keiner mehr uebrig ist. Nachbar:
/// gleiche Sprecher links und rechts -> dieser; sonst der zeitlich naehere,
/// bei Gleichstand der vorherige. Ein einzelner Lauf bleibt.
fn smooth(mut runs: Vec<Run>) -> Vec<Run> {
    loop {
        if runs.len() < 2 {
            return runs;
        }
        let Some(i) = runs.iter().position(Run::is_short) else {
            return runs;
        };
        let prev = i.checked_sub(1);
        let next = (i + 1 < runs.len()).then_some(i + 1);
        let into_prev = match (prev, next) {
            (Some(p), Some(n)) => {
                if runs[p].speaker == runs[n].speaker {
                    true
                } else {
                    let gap_prev = runs[i].start_ms().saturating_sub(runs[p].end_ms());
                    let gap_next = runs[n].start_ms().saturating_sub(runs[i].end_ms());
                    gap_prev <= gap_next
                }
            }
            (Some(_), None) => true,
            _ => false,
        };
        let run = runs.remove(i);
        if into_prev {
            runs[i - 1].words.extend(run.words);
        } else {
            let mut words = run.words;
            words.extend(std::mem::take(&mut runs[i].words));
            runs[i].words = words;
        }
        merge_equal_neighbours(&mut runs);
    }
}

/// Texte der Teile: die Leerraum-Woerter des Originaltexts, wenn ihre Zahl
/// zu den Wortzeilen passt (Satzzeichen und Schreibweise bleiben), sonst die
/// Wortzeilen selbst.
fn part_texts(original: &str, runs: &[Run]) -> Vec<String> {
    let total: usize = runs.iter().map(|r| r.words.len()).sum();
    let tokens: Vec<&str> = original.split_whitespace().collect();
    let mut at = 0;
    runs.iter()
        .map(|r| {
            let n = r.words.len();
            let text = if tokens.len() == total {
                tokens[at..at + n].join(" ")
            } else {
                r.words
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            at += n;
            text
        })
        .collect()
}

/// Ergebnis der Zuordnung fuer einen Kanal.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct AssignStats {
    /// Segmente (nach dem Teilen) mit gesetztem `speaker_index`.
    pub assigned: usize,
    /// Segmente ohne Sprecher (keine Ueberlappung, Luecken-Platzhalter).
    pub unassigned: usize,
    /// Wie viele Segmente durch das Teilen dazukamen.
    pub split_added: usize,
}

impl AssignStats {
    fn add(&mut self, o: &AssignStats) {
        self.assigned += o.assigned;
        self.unassigned += o.unassigned;
        self.split_added += o.split_added;
    }
}

fn assign_one(seg: &StoredSegment, turns: &[Turn], out: &mut Vec<StoredSegment>) -> AssignStats {
    let mut stats = AssignStats::default();
    let mut base = seg.clone();
    base.speaker_index = None;
    if is_gap_placeholder(&seg.text) {
        stats.unassigned += 1;
        out.push(base);
        return stats;
    }
    let words = seg.words.as_deref().filter(|w| !w.is_empty());
    let by_words = words.and_then(|w| word_speakers(w, turns).map(|s| (w, s)));
    let Some((words, speakers)) = by_words else {
        // Ohne (aufloesbare) Wortzeiten: ganzes Segment, kein Teilen.
        base.speaker_index = speaker_for_span(turns, seg.start_ms, seg.end_ms, 0);
        if base.speaker_index.is_some() {
            stats.assigned += 1;
        } else {
            stats.unassigned += 1;
        }
        out.push(base);
        return stats;
    };
    let runs = smooth(runs_of(words, &speakers));
    if runs.len() == 1 {
        base.speaker_index = Some(runs[0].speaker);
        stats.assigned += 1;
        out.push(base);
        return stats;
    }
    let texts = part_texts(&seg.text, &runs);
    stats.split_added += runs.len() - 1;
    for (run, text) in runs.into_iter().zip(texts) {
        stats.assigned += 1;
        out.push(StoredSegment {
            segment_index: seg.segment_index,
            text,
            start_ms: run.start_ms(),
            end_ms: run.end_ms().max(run.start_ms()),
            channel: seg.channel,
            speaker_index: Some(run.speaker),
            words: Some(run.words),
        });
    }
    stats
}

/// Ordnet die Segmente EINES Kanals den Turns dieses Kanals zu (siehe
/// Moduldoku). Die Reihenfolge bleibt; geteilte Segmente stehen hintereinander.
pub fn assign_channel(
    segments: &[StoredSegment],
    turns: &[Turn],
) -> (Vec<StoredSegment>, AssignStats) {
    let mut out = Vec::with_capacity(segments.len());
    let mut stats = AssignStats::default();
    for seg in segments {
        stats.add(&assign_one(seg, turns, &mut out));
    }
    (out, stats)
}

/// Ordnet alle Segmente zu: Kanaele mit einem Eintrag in `turns_by_channel`
/// (auch mit leerer Turn-Liste: Sprache fehlt, Sprecher werden geleert)
/// werden bearbeitet, alle anderen bleiben unveraendert ("Ich" ohne
/// Sprechertrennung, uebersprungene Kanaele). Reihenfolge wie die Eingabe.
pub fn assign_segments(
    segments: Vec<StoredSegment>,
    turns_by_channel: &BTreeMap<u8, Vec<Turn>>,
) -> (Vec<StoredSegment>, AssignStats) {
    let mut out = Vec::with_capacity(segments.len());
    let mut stats = AssignStats::default();
    for seg in segments {
        match turns_by_channel.get(&seg.channel) {
            Some(turns) => stats.add(&assign_one(&seg, turns, &mut out)),
            None => out.push(seg),
        }
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(start_ms: u64, end_ms: u64, speaker: u32) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    fn word(text: &str, start_ms: u64, end_ms: u64) -> WordTime {
        WordTime {
            text: text.to_string(),
            start_ms,
            end_ms,
        }
    }

    /// Woerter zu je `step` ms ab `from`, lueckenlos.
    fn words(texts: &[&str], from: u64, step: u64) -> Vec<WordTime> {
        texts
            .iter()
            .enumerate()
            .map(|(i, t)| word(t, from + i as u64 * step, from + (i as u64 + 1) * step))
            .collect()
    }

    fn seg(text: &str, start_ms: u64, end_ms: u64, w: Option<Vec<WordTime>>) -> StoredSegment {
        StoredSegment {
            segment_index: 7,
            text: text.to_string(),
            start_ms,
            end_ms,
            channel: 1,
            speaker_index: None,
            words: w,
        }
    }

    #[test]
    fn split_on_speaker_change_with_words() {
        // Ein 25-s-Block: zwei Sprecher hintereinander, Woerter mit Zeiten.
        let w = words(
            &[
                "Guten", "Tag,", "Frau", "Berg.", "Ja,", "gerne,", "danke", "schoen.",
            ],
            1_000,
            500,
        );
        let s = seg(
            "Guten Tag, Frau Berg. Ja, gerne, danke schoen.",
            0,
            5_000,
            Some(w.clone()),
        );
        let turns = vec![turn(0, 3_000, 1), turn(3_000, 5_000, 2)];
        let (out, stats) = assign_channel(&[s], &turns);

        assert_eq!(out.len(), 2, "am Sprecherwechsel geteilt");
        assert_eq!(out[0].speaker_index, Some(1));
        assert_eq!(out[1].speaker_index, Some(2));
        assert_eq!(out[0].text, "Guten Tag, Frau Berg.");
        assert_eq!(out[1].text, "Ja, gerne, danke schoen.");
        // Zeiten aus den Wortgrenzen, Woerter aufgeteilt, Kanal bleibt.
        assert_eq!((out[0].start_ms, out[0].end_ms), (1_000, 3_000));
        assert_eq!((out[1].start_ms, out[1].end_ms), (3_000, 5_000));
        assert_eq!(out[0].words.as_ref().unwrap(), &w[..4]);
        assert_eq!(out[1].words.as_ref().unwrap(), &w[4..]);
        assert!(out.iter().all(|p| p.channel == 1 && p.segment_index == 7));
        assert_eq!(
            stats,
            AssignStats {
                assigned: 2,
                unassigned: 0,
                split_added: 1
            }
        );
    }

    #[test]
    fn no_words_no_split() {
        // Ohne Wortzeiten: das ganze Segment geht an die groesste Ueberlappung.
        let s = seg("Guten Tag, Frau Berg. Ja, gerne.", 0, 6_000, None);
        let turns = vec![turn(0, 2_000, 1), turn(2_000, 6_000, 2)];
        let (out, stats) = assign_channel(std::slice::from_ref(&s), &turns);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].speaker_index, Some(2), "4 s gegen 2 s");
        assert_eq!(out[0].text, s.text);
        assert_eq!((out[0].start_ms, out[0].end_ms), (0, 6_000));
        assert_eq!(stats.split_added, 0);
        // Ohne jede Ueberlappung bleibt der Sprecher leer (auch nicht "nah dran").
        let far = vec![turn(6_100, 9_000, 1)];
        let (out, stats) = assign_channel(&[s], &far);
        assert_eq!(out[0].speaker_index, None);
        assert_eq!(stats.unassigned, 1);
    }

    #[test]
    fn the_largest_overlap_wins_and_a_tie_goes_to_the_word_centre() {
        // Wort 1000..1400: 300 ms bei Sprecher 1, 100 ms bei Sprecher 2.
        let turns = vec![turn(0, 1_300, 1), turn(1_300, 2_000, 2)];
        assert_eq!(speaker_for_span(&turns, 1_000, 1_400, 500), Some(1));
        // Gleichstand 100/100 (Zentrum 1100): der Turn, der das Zentrum enthaelt.
        let tie = vec![turn(0, 1_100, 1), turn(1_100, 2_000, 2)];
        assert_eq!(
            speaker_for_span(&tie, 1_000, 1_200, 500),
            Some(2),
            "Zentrum 1100 liegt im 2. Turn"
        );
        // Ueberlappende Turns: mehr Ueberlappung entscheidet, nicht der Beginn.
        let over = vec![turn(0, 1_050, 1), turn(900, 3_000, 2)];
        assert_eq!(speaker_for_span(&over, 1_000, 1_500, 500), Some(2));
        // Mehrere Turns eines Sprechers zaehlen zusammen.
        let split = vec![
            turn(0, 1_200, 1),
            turn(1_300, 1_500, 1),
            turn(1_200, 1_300, 2),
        ];
        assert_eq!(speaker_for_span(&split, 1_000, 1_500, 500), Some(1));
        // Voelliger Gleichstand ohne Zentrumsvorteil: kleinere Nummer.
        let both = vec![turn(0, 2_000, 2), turn(0, 2_000, 1)];
        assert_eq!(speaker_for_span(&both, 500, 900, 500), Some(1));
    }

    #[test]
    fn a_word_between_turns_takes_the_near_turn_or_the_previous_speaker() {
        let turns = vec![turn(0, 1_000, 1), turn(2_000, 3_000, 2)];
        // Der naehere Turn, hoechstens 500 ms entfernt.
        assert_eq!(
            speaker_for_span(&turns, 1_300, 1_400, 500),
            Some(1),
            "300 vs 600 ms"
        );
        assert_eq!(
            speaker_for_span(&turns, 1_450, 1_550, 500),
            Some(1),
            "450 vs 450: kleinere Nummer"
        );
        assert_eq!(
            speaker_for_span(&turns, 1_460, 1_560, 500),
            Some(2),
            "460 vs 440 ms"
        );
        assert_eq!(
            speaker_for_span(&turns, 3_500, 3_600, 500),
            Some(2),
            "genau 500 ms"
        );
        assert_eq!(
            speaker_for_span(&turns, 3_501, 3_600, 500),
            None,
            "501 ms entfernt"
        );
        // Im Segment: das Wort 6 s hinter dem letzten Turn erbt den Vorgaenger,
        // ein fuehrendes Wort den ersten aufgeloesten Sprecher.
        let w = vec![
            word("aeh", 0, 100), // vor jedem Turn (Turn 1 beginnt bei 1000)
            word("hallo", 1_000, 1_400),
            word("welt", 1_400, 1_800),
            word("und", 7_000, 7_300), // ohne Turn in der Naehe
            word("so", 7_300, 7_600),
        ];
        let turns = vec![turn(1_000, 2_000, 3)];
        let s = seg("aeh hallo welt und so", 0, 8_000, Some(w));
        let (out, _) = assign_channel(&[s], &turns);
        assert_eq!(out.len(), 1, "alle Woerter bei Sprecher 3");
        assert_eq!(out[0].speaker_index, Some(3));
        assert_eq!(
            out[0].text, "aeh hallo welt und so",
            "unveraenderter Text bei einem Lauf"
        );
    }

    #[test]
    fn a_lone_short_run_at_a_boundary_goes_to_the_neighbour() {
        // 6 Woerter Sprecher 1, ein einzelnes kurzes Wort Sprecher 2, dann 5 Woerter 1.
        let mut w = words(&["a", "b", "c", "d", "e", "f"], 0, 300);
        w.push(word("ja", 1_800, 1_950)); // 150 ms, 1 Wort
        w.extend(words(&["g", "h", "i", "j", "k"], 1_950, 300));
        let turns = vec![
            turn(0, 1_800, 1),
            turn(1_800, 1_950, 2),
            turn(1_950, 4_000, 1),
        ];
        let s = seg("a b c d e f ja g h i j k", 0, 4_000, Some(w));
        let (out, stats) = assign_channel(&[s], &turns);
        assert_eq!(out.len(), 1, "das Flackern wird geglaettet");
        assert_eq!(out[0].speaker_index, Some(1));
        assert_eq!(stats.split_added, 0);

        // Zwei Woerter desselben Sprechers sind kein Flackern.
        let mut w = words(&["a", "b", "c", "d", "e", "f"], 0, 300);
        w.extend(words(&["ja", "genau"], 1_800, 150));
        let turns = vec![turn(0, 1_800, 1), turn(1_800, 2_100, 2)];
        let s = seg("a b c d e f ja genau", 0, 2_100, Some(w));
        let (out, _) = assign_channel(&[s], &turns);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].text, "ja genau");
        assert_eq!(out[1].speaker_index, Some(2));

        // Ein einzelnes LANGES Wort (>= 600 ms) bleibt eigener Sprecher.
        let w = vec![
            word("a", 0, 300),
            word("b", 300, 600),
            word("c", 600, 900),
            word("Entschuldigung", 900, 1_800),
            word("d", 1_800, 2_100),
            word("e", 2_100, 2_400),
        ];
        let turns = vec![turn(0, 900, 1), turn(900, 1_800, 2), turn(1_800, 2_400, 1)];
        let s = seg("a b c Entschuldigung d e", 0, 2_400, Some(w));
        let (out, _) = assign_channel(&[s], &turns);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn a_short_first_or_last_run_joins_its_only_neighbour() {
        let mut w = vec![word("ja", 0, 200)];
        w.extend(words(&["also", "ich", "denke", "wir"], 200, 300));
        let turns = vec![turn(0, 200, 2), turn(200, 1_400, 1)];
        let s = seg("ja also ich denke wir", 0, 1_400, Some(w));
        let (out, _) = assign_channel(&[s], &turns);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].speaker_index, Some(1));
        assert_eq!((out[0].start_ms, out[0].end_ms), (0, 1_400));

        // Zwei kurze Laeufe hintereinander: am Ende bleibt ein Lauf, nie ein Absturz.
        let w = vec![word("a", 0, 100), word("b", 100, 200)];
        let turns = vec![turn(0, 100, 1), turn(100, 200, 2)];
        let s = seg("a b", 0, 200, Some(w));
        let (out, _) = assign_channel(&[s], &turns);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn split_texts_keep_punctuation_or_fall_back_to_the_word_rows() {
        let w = words(&["hallo", "du", "ja", "gern", "geschehen"], 0, 400);
        let turns = vec![turn(0, 800, 1), turn(800, 2_000, 2)];
        // Token-Zahl passt: Schreibweise des Originals bleibt.
        let s = seg("Hallo, du! Ja, gern geschehen.", 0, 2_000, Some(w.clone()));
        let (out, _) = assign_channel(&[s], &turns);
        assert_eq!(out[0].text, "Hallo, du!");
        assert_eq!(out[1].text, "Ja, gern geschehen.");
        // Token-Zahl passt nicht (Engine hat Woerter anders geschnitten): Wortzeilen.
        let s = seg("Hallo du Ja gern-geschehen", 0, 2_000, Some(w));
        let (out, _) = assign_channel(&[s], &turns);
        assert_eq!(out[0].text, "hallo du");
        assert_eq!(out[1].text, "ja gern geschehen");
    }

    #[test]
    fn unresolvable_words_fall_back_to_the_whole_segment() {
        // Kein Wort liegt bei einem Turn: kein Teilen, Segmentueberlappung entscheidet.
        let w = words(&["a", "b", "c"], 9_000, 300);
        let s = seg("a b c", 500, 9_900, Some(w));
        let turns = vec![turn(0, 4_000, 1)];
        let (out, _) = assign_channel(&[s], &turns);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].speaker_index, Some(1));
        // Leere Wortliste zaehlt wie "keine Woerter".
        let s = seg("a b c", 0, 3_000, Some(Vec::new()));
        let (out, _) = assign_channel(&[s], &turns);
        assert_eq!(out[0].speaker_index, Some(1));
    }

    #[test]
    fn a_gap_placeholder_belongs_to_nobody() {
        let text = crate::managers::meetings::import::gap_placeholder(0, 60_000);
        let s = seg(&text, 0, 60_000, None);
        let turns = vec![turn(0, 60_000, 1)];
        let (out, stats) = assign_channel(&[s], &turns);
        assert_eq!(out[0].speaker_index, None);
        assert_eq!(stats.unassigned, 1);
    }

    #[test]
    fn only_diarized_channels_are_touched_and_empty_turns_clear_the_speaker() {
        let mut ich = seg("ich", 0, 1_000, None);
        ich.channel = 0;
        ich.speaker_index = Some(9); // bleibt: Kanal 0 wurde nicht diarisiert
        let mut other = seg("gegen", 0, 1_000, None);
        other.speaker_index = Some(3);
        let mut turns = BTreeMap::new();
        turns.insert(1u8, Vec::new()); // diarisiert, aber keine Sprache gefunden
        let (out, _) = assign_segments(vec![ich, other], &turns);
        assert_eq!(out[0].speaker_index, Some(9));
        assert_eq!(out[1].speaker_index, None);

        let mut turns = BTreeMap::new();
        turns.insert(1u8, vec![turn(0, 1_000, 2)]);
        let mut ch1 = seg("gegen", 0, 1_000, None);
        ch1.channel = 1;
        let mut ch2 = seg("aufnahme", 0, 1_000, None);
        ch2.channel = 2;
        let (out, stats) = assign_segments(vec![ch1, ch2], &turns);
        assert_eq!(
            (out[0].speaker_index, out[1].speaker_index),
            (Some(2), None)
        );
        assert_eq!(stats.assigned, 1);
    }

    #[test]
    fn three_speakers_in_turn_and_an_empty_segment_list() {
        let w = words(
            &["a1", "a2", "a3", "b1", "b2", "b3", "c1", "c2", "c3"],
            0,
            400,
        );
        let turns = vec![
            turn(0, 1_200, 1),
            turn(1_200, 2_400, 2),
            turn(2_400, 3_600, 3),
        ];
        let s = seg("a1 a2 a3 b1 b2 b3 c1 c2 c3", 0, 3_600, Some(w));
        let (out, stats) = assign_channel(&[s], &turns);
        let speakers: Vec<_> = out.iter().map(|p| p.speaker_index).collect();
        assert_eq!(speakers, vec![Some(1), Some(2), Some(3)]);
        assert_eq!(stats.split_added, 2);
        // Zeiten der Teile schliessen lueckenlos aneinander.
        assert!(out.windows(2).all(|p| p[0].end_ms == p[1].start_ms));
        let (none, stats) = assign_channel(&[], &turns);
        assert!(none.is_empty());
        assert_eq!(stats, AssignStats::default());
    }
}
