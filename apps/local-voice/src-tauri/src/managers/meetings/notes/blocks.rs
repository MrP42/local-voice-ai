//! Arbeitsstapel fuer lange Transkripte: Bloecke, Halbieren, Luecken (P1k, B12).
//!
//! Dieselbe Regel wie in den KI-Notizen (P1i, `enhance::map_reduce`), hier als
//! allgemeine Schleife fuer das Protokoll: ein Block, der dem Modell zu gross
//! war (Antwort abgeschnitten oder ungueltig, Prompt ohne Platz fuer die
//! Antwort), wird in zwei Haelften gefragt, bis zu `MAX_SPLIT_DEPTH` Stufen; was
//! auch als Viertel nicht auswertbar ist, bleibt eine sichtbare Luecke
//! (`Leaf::value == None`), nie ein stilles Verwerfen. Die Schleife kennt weder
//! Modell noch Prompt: der Aufrufer liefert je Arbeitsstueck einen [`Step`].
//!
//! Rein und ohne I/O; die Tests fahren sie mit gespielten Schritten.

use std::future::Future;
use std::ops::Range;

use super::budget::{self, MAX_SPLIT_DEPTH};

/// Ein Stueck Transkript fuer einen Aufruf: ein Block der Planung oder, nach
/// dem Halbieren, ein Teil davon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Work {
    /// Indexbereich der Zeilen (Segmente) im Transkript.
    pub range: Range<usize>,
    /// Nummer des Blocks der Planung (ab 0).
    pub block_index: usize,
    /// Weg der Halbierungen (0 links, 1 rechts); leer = ganzer Block.
    pub path: Vec<u8>,
}

impl Work {
    /// Noch eine Stufe erlaubt, und mindestens zwei Zeilen (eine einzelne Zeile
    /// wird nie zerschnitten: Zeitmarken bleiben ganz).
    pub fn can_split(&self) -> bool {
        (self.path.len() as u32) < MAX_SPLIT_DEPTH && self.range.len() >= 2
    }

    /// Bezeichnung im Prompt und im Log: `2`, `2.1`, `2.2.1` (`budget::part_label`).
    pub fn label(&self) -> String {
        budget::part_label(self.block_index, &self.path)
    }
}

/// Was aus einem Versuch wird.
#[derive(Debug)]
pub enum Step<T, E> {
    Done(T),
    /// Zu gross (Prompt passt nicht, Antwort abgeschnitten oder ungueltig): in
    /// zwei Haelften noch einmal versuchen.
    Split,
    /// Auch nach dem Halbieren nicht auswertbar (der letzte Fehler).
    Failed(String),
    /// Abbruch des ganzen Laufs (RAM knapp, Nutzer hat gestoppt).
    Abort(E),
}

/// Ein ausgewerteter oder verlorener Teil, in Transkriptreihenfolge.
#[derive(Debug)]
pub struct Leaf<T> {
    pub range: Range<usize>,
    pub value: Option<T>,
}

#[derive(Debug)]
pub struct BlocksOutcome<T> {
    pub leaves: Vec<Leaf<T>>,
    /// Wie oft ein Block halbiert wurde.
    pub splits: u32,
}

impl<T> BlocksOutcome<T> {
    /// Nummern (ab 1, in Transkriptreihenfolge) der Teile ohne Ergebnis.
    pub fn failed_numbers(&self) -> Vec<u32> {
        self.leaves
            .iter()
            .enumerate()
            .filter(|(_, leaf)| leaf.value.is_none())
            .map(|(index, _)| index as u32 + 1)
            .collect()
    }

    /// Die Bereiche ohne Ergebnis; aneinandergrenzende (Viertel desselben
    /// Blocks) sind zu einem verschmolzen.
    pub fn gap_ranges(&self) -> Vec<Range<usize>> {
        let mut merged: Vec<Range<usize>> = Vec::new();
        for leaf in self.leaves.iter().filter(|leaf| leaf.value.is_none()) {
            match merged.last_mut() {
                Some(last) if last.end == leaf.range.start => last.end = leaf.range.end,
                _ => merged.push(leaf.range.clone()),
            }
        }
        merged
    }
}

/// Arbeitet alle Bloecke der Reihe nach ab. `line_chars[i]` ist die Laenge der
/// Zeile `i` (bestimmt, wo halbiert wird). `before_step` laeuft VOR jedem
/// Aufruf und kann den Lauf abbrechen (Stopp des Nutzers); `block_done` meldet
/// nach jedem fertigen Block der Planung, wie viele es schon sind.
pub async fn run_blocks<T, E, F, Fut>(
    ranges: &[Range<usize>],
    line_chars: &[usize],
    mut before_step: impl FnMut() -> Result<(), E>,
    mut block_done: impl FnMut(usize),
    mut step: F,
) -> Result<BlocksOutcome<T>, E>
where
    F: FnMut(Work) -> Fut,
    Fut: Future<Output = Step<T, E>>,
{
    let mut leaves: Vec<Leaf<T>> = Vec::new();
    let mut splits = 0u32;
    for (index, range) in ranges.iter().enumerate() {
        // Arbeitsstapel: die linke Haelfte kommt zuerst dran, und ihr ganzer
        // Teilbaum ist fertig, bevor die rechte beginnt: die Teile landen in
        // Transkriptreihenfolge.
        let mut stack = vec![Work {
            range: range.clone(),
            block_index: index,
            path: Vec::new(),
        }];
        while let Some(work) = stack.pop() {
            before_step()?;
            match step(work.clone()).await {
                Step::Done(value) => leaves.push(Leaf {
                    range: work.range,
                    value: Some(value),
                }),
                Step::Split => {
                    let Some((left, right)) = budget::halve(&work.range, line_chars) else {
                        // Unerreichbar (`can_split` prueft der Aufrufer); sicher statt
                        // Endlosschleife.
                        leaves.push(Leaf {
                            range: work.range,
                            value: None,
                        });
                        continue;
                    };
                    splits += 1;
                    let (mut left_path, mut right_path) = (work.path.clone(), work.path);
                    left_path.push(0);
                    right_path.push(1);
                    stack.push(Work {
                        range: right,
                        block_index: work.block_index,
                        path: right_path,
                    });
                    stack.push(Work {
                        range: left,
                        block_index: work.block_index,
                        path: left_path,
                    });
                }
                Step::Failed(_) => leaves.push(Leaf {
                    range: work.range,
                    value: None,
                }),
                Step::Abort(err) => return Err(err),
            }
        }
        block_done(index + 1);
    }
    Ok(BlocksOutcome { leaves, splits })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Ein Aufruf, den der Test spielt: `label -> Step`.
    fn play<'a>(
        script: &'a RefCell<Vec<String>>,
        rule: impl Fn(&Work) -> Step<String, &'static str> + 'a,
    ) -> impl FnMut(Work) -> std::future::Ready<Step<String, &'static str>> + 'a {
        move |work: Work| {
            script.borrow_mut().push(work.label());
            std::future::ready(rule(&work))
        }
    }

    fn lens(n: usize) -> Vec<usize> {
        vec![100; n]
    }

    #[tokio::test]
    async fn every_block_runs_once_in_order_when_nothing_is_too_big() {
        let script = RefCell::new(Vec::new());
        let ranges = vec![0..4, 4..8, 8..12];
        let done = RefCell::new(Vec::new());
        let out = run_blocks(
            &ranges,
            &lens(12),
            || Ok::<(), &str>(()),
            |n| done.borrow_mut().push(n),
            play(&script, |w| Step::Done(w.label())),
        )
        .await
        .unwrap();
        assert_eq!(*script.borrow(), vec!["1", "2", "3"]);
        assert_eq!(*done.borrow(), vec![1, 2, 3]);
        assert_eq!(out.splits, 0);
        assert!(out.failed_numbers().is_empty() && out.gap_ranges().is_empty());
        assert_eq!(out.leaves.len(), 3);
    }

    /// Der Kern von B12: was abgeschnitten wird, wird halbiert und
    /// ausgewertet, nicht verworfen. Reihenfolge: links vor rechts, Teilbaum
    /// vor Geschwister.
    #[tokio::test]
    async fn a_too_big_block_is_halved_and_both_halves_are_evaluated_in_order() {
        let script = RefCell::new(Vec::new());
        let ranges = vec![0..4, 4..8];
        let out = run_blocks(
            &ranges,
            &lens(8),
            || Ok::<(), &str>(()),
            |_| {},
            play(&script, |w| {
                if w.range.len() > 2 {
                    Step::Split
                } else {
                    Step::Done(format!("{}..{}", w.range.start, w.range.end))
                }
            }),
        )
        .await
        .unwrap();
        assert_eq!(*script.borrow(), vec!["1", "1.1", "1.2", "2", "2.1", "2.2"]);
        assert_eq!(out.splits, 2);
        let values: Vec<_> = out
            .leaves
            .iter()
            .map(|l| l.value.clone().unwrap())
            .collect();
        assert_eq!(
            values,
            vec!["0..2", "2..4", "4..6", "6..8"],
            "lueckenlos, in Reihenfolge"
        );
        assert!(out.gap_ranges().is_empty());
    }

    #[tokio::test]
    async fn halving_stops_after_two_levels_and_leaves_a_visible_gap() {
        // Alles ist "zu gross": Original, Haelften, Viertel -- danach eine Luecke.
        let script = RefCell::new(Vec::new());
        let ranges = vec![0..8];
        let out = run_blocks(
            &ranges,
            &lens(8),
            || Ok::<(), &str>(()),
            |_| {},
            play(&script, |w| {
                if w.can_split() {
                    Step::Split
                } else {
                    Step::Failed("Antwort abgeschnitten".into())
                }
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            *script.borrow(),
            vec!["1", "1.1", "1.1.1", "1.1.2", "1.2", "1.2.1", "1.2.2"]
        );
        assert_eq!(out.splits, 3);
        assert_eq!(out.failed_numbers(), vec![1, 2, 3, 4]);
        // Vier aneinandergrenzende Viertel sind EINE Luecke ueber den ganzen Block.
        assert_eq!(out.gap_ranges(), vec![0..8]);
    }

    #[tokio::test]
    async fn a_single_line_is_never_cut_and_becomes_a_gap_when_it_fails() {
        let script = RefCell::new(Vec::new());
        let ranges = vec![0..1, 1..3];
        let out = run_blocks(
            &ranges,
            &lens(3),
            || Ok::<(), &str>(()),
            |_| {},
            play(&script, |w| {
                assert!(!w.range.is_empty());
                if w.range == (0..1) {
                    Step::Failed("Antwort war kein gueltiges JSON".into())
                } else {
                    Step::Done("ok".into())
                }
            }),
        )
        .await
        .unwrap();
        assert_eq!(*script.borrow(), vec!["1", "2"]);
        assert_eq!(out.gap_ranges(), vec![0..1]);
        assert_eq!(out.failed_numbers(), vec![1]);
    }

    #[tokio::test]
    async fn separate_gaps_stay_separate() {
        let ranges = vec![0..2, 2..4, 4..6];
        let script = RefCell::new(Vec::new());
        let out = run_blocks(
            &ranges,
            &lens(6),
            || Ok::<(), &str>(()),
            |_| {},
            play(&script, |w| {
                if w.block_index == 1 {
                    Step::Done("ok".into())
                } else {
                    Step::Failed("x".into())
                }
            }),
        )
        .await
        .unwrap();
        assert_eq!(out.gap_ranges(), vec![0..2, 4..6]);
        assert_eq!(out.failed_numbers(), vec![1, 3]);
    }

    /// Abbruch (Speicher knapp, Stopp): kein weiterer Block wird angefasst.
    #[tokio::test]
    async fn an_abort_ends_the_run_at_once() {
        let script = RefCell::new(Vec::new());
        let ranges = vec![0..4, 4..8, 8..12];
        let result = run_blocks(
            &ranges,
            &lens(12),
            || Ok::<(), &str>(()),
            |_| {},
            play(&script, |w| {
                if w.block_index == 1 {
                    Step::Abort("memory_low")
                } else {
                    Step::Done("ok".into())
                }
            }),
        )
        .await;
        assert_eq!(result.unwrap_err(), "memory_low");
        assert_eq!(
            *script.borrow(),
            vec!["1", "2"],
            "Block 3 wurde nie gefragt"
        );
    }

    /// Stopp des Nutzers wird VOR jedem Aufruf geprueft, auch zwischen den
    /// Haelften eines halbierten Blocks.
    #[tokio::test]
    async fn the_stop_check_runs_before_every_call() {
        let script = RefCell::new(Vec::new());
        let ranges = vec![0..4, 4..8];
        let checks = RefCell::new(0usize);
        let result = run_blocks(
            &ranges,
            &lens(8),
            || {
                *checks.borrow_mut() += 1;
                // Der vierte Aufruf (nach 1, 1.1, 1.2) wird nicht mehr gestartet.
                if *checks.borrow() > 3 {
                    Err("cancelled")
                } else {
                    Ok(())
                }
            },
            |_| {},
            play(&script, |w| {
                if w.range.len() > 2 {
                    Step::Split
                } else {
                    Step::Done("ok".into())
                }
            }),
        )
        .await;
        assert_eq!(result.unwrap_err(), "cancelled");
        assert_eq!(*script.borrow(), vec!["1", "1.1", "1.2"]);
    }

    #[test]
    fn a_work_item_knows_when_it_may_be_split() {
        let work = |len: usize, depth: usize| Work {
            range: 0..len,
            block_index: 0,
            path: vec![0; depth],
        };
        assert!(work(2, 0).can_split());
        assert!(work(64, 1).can_split());
        assert!(!work(1, 0).can_split(), "eine Zeile nie");
        assert!(
            !work(64, MAX_SPLIT_DEPTH as usize).can_split(),
            "nur zwei Stufen"
        );
        assert_eq!(work(4, 2).label(), "1.1.1");
    }
}
