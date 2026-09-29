//! Chat waehrend der Aufnahme (M4 §6 "Live"): kein Index, keine Embeddings.
//!
//! Aus einer Momentaufnahme (Segmente, Notizblock, Audioposition) entstehen
//! die Auszuege: die letzten 8 Minuten komplett (fuer "Was habe ich
//! verpasst?"), aeltere Stellen per BM25 im Speicher, der Notizblock immer.
//! Die Zitate tragen die Live-Epoche; nach `TranscriptFinal` setzt die UI sie
//! per `remap_sources` (M2/P2d) um.

use anyhow::Result;

use super::context::{
    bm25_rank, notes_excerpts, total_cost, transcript_blocks, Excerpt, ExcerptPlan, MeetingRef,
};
use crate::managers::meetings::notes::model::NoteBlock;
use crate::managers::meetings::search::chunking::TARGET_CHARS;
use crate::managers::meetings::store::{MeetingStore, StoredSegment};

/// Dieses Fenster vor dem Transkript-Ende ist immer im Prompt.
pub const RECENT_WINDOW_MS: u64 = 8 * 60_000;
/// Hoechstens dieser Anteil des Budgets geht an den Notizblock.
const NOTES_SHARE_PERCENT: usize = 25;

#[derive(Clone, Debug)]
pub struct LiveSnapshot {
    pub meeting: MeetingRef,
    pub epoch: u32,
    pub segments: Vec<StoredSegment>,
    pub notes: Vec<NoteBlock>,
    /// Audioposition der Aufnahme (B2); `None` = Ende des letzten Segments.
    pub position_ms: Option<u64>,
}

impl LiveSnapshot {
    /// Momentaufnahme aus dem Store. `None`, wenn es die Besprechung nicht
    /// (mehr) gibt. Ein unlesbarer Notizblock kostet nur die Notizen.
    pub fn from_store(
        store: &MeetingStore,
        meeting_id: &str,
        position_ms: Option<u64>,
    ) -> Result<Option<Self>> {
        let Some(meeting) = store.get_meeting(meeting_id)? else {
            return Ok(None);
        };
        if meeting.deleted_at.is_some() {
            return Ok(None);
        }
        let notes = match store.get_notes(meeting_id) {
            Ok(n) => n.blocks,
            Err(e) => {
                log::warn!("Live-Chat: Notizblock nicht lesbar ({e})");
                Vec::new()
            }
        };
        Ok(Some(Self {
            meeting: MeetingRef {
                id: meeting.id,
                title: meeting.title,
                started_at: meeting.started_at.or(Some(meeting.created_at)),
            },
            epoch: store.segment_epoch(meeting_id)?,
            segments: store.get_segments(meeting_id)?,
            notes,
            position_ms,
        }))
    }

    /// Ende des Transkripts: die Audioposition, sonst das spaeteste Segmentende.
    pub fn end_ms(&self) -> u64 {
        self.position_ms
            .unwrap_or_else(|| self.segments.iter().map(|s| s.end_ms).max().unwrap_or(0))
    }
}

/// Auszuege fuer eine Live-Frage innerhalb von `budget` Zeichen.
///
/// Vergabe: Notizblock (hoechstens 25 %), dann die juengsten Bloecke des
/// 8-Minuten-Fensters (passt nicht alles, die neuesten), dann aeltere Bloecke
/// nach BM25. Was nicht passt, kommt als Kandidat fuer die Wiederholung in
/// `secondary` (Treffer nach Rang, dann die uebrigen, neueste zuerst).
/// Reihenfolge im Prompt: Notizen, aeltere Stellen, juengstes Fenster.
pub fn live_plan(snapshot: &LiveSnapshot, question: &str, budget: usize) -> ExcerptPlan {
    let meeting = &snapshot.meeting;
    let cutoff = snapshot.end_ms().saturating_sub(RECENT_WINDOW_MS);
    let (recent, old): (Vec<StoredSegment>, Vec<StoredSegment>) = snapshot
        .segments
        .iter()
        .cloned()
        .partition(|s| s.start_ms >= cutoff);
    let mut truncated = false;
    let mut left = budget;

    // 1. Notizblock (immer, gedeckelt).
    let notes_cap = budget * NOTES_SHARE_PERCENT / 100;
    let mut notes_used = 0usize;
    let mut notes: Vec<Excerpt> = Vec::new();
    for ex in notes_excerpts(meeting, &snapshot.notes, TARGET_CHARS) {
        if notes_used + ex.cost() <= notes_cap {
            notes_used += ex.cost();
            notes.push(ex);
        } else {
            truncated = true;
        }
    }
    left = left.saturating_sub(notes_used);

    // 2. Juengstes Fenster: von hinten, zusammenhaengend.
    let mut window: Vec<Excerpt> = Vec::new();
    for ex in transcript_blocks(meeting, &recent, snapshot.epoch, TARGET_CHARS)
        .into_iter()
        .rev()
    {
        if ex.cost() <= left {
            left -= ex.cost();
            window.push(ex);
        } else {
            // Vom Block, der nicht mehr ganz passt, die neuesten Zeilen.
            truncated = true;
            if let Some(tail) = newest_lines_within(ex, left) {
                left -= tail.cost();
                window.push(tail);
            }
            break;
        }
    }
    window.reverse();

    // 3. Aeltere Stellen nach Relevanz.
    let old_blocks = transcript_blocks(meeting, &old, snapshot.epoch, TARGET_CHARS);
    let docs: Vec<String> = old_blocks.iter().map(Excerpt::body).collect();
    let ranked = bm25_rank(question, &docs);
    let hit_count = ranked.len();
    let mut taken_ix: Vec<usize> = Vec::new();
    let mut secondary: Vec<Excerpt> = Vec::new();
    for (ix, score) in &ranked {
        let mut ex = old_blocks[*ix].clone();
        ex.score = *score;
        if ex.cost() <= left {
            left -= ex.cost();
            taken_ix.push(*ix);
        } else {
            secondary.push(ex);
        }
    }
    taken_ix.sort_unstable();
    let ranked_ix: Vec<usize> = ranked.iter().map(|(i, _)| *i).collect();
    for (ix, ex) in old_blocks.iter().enumerate().rev() {
        if !ranked_ix.contains(&ix) {
            secondary.push(ex.clone());
        }
    }
    if !secondary.is_empty() && hit_count > taken_ix.len() {
        truncated = true;
    }

    let mut primary = notes;
    primary.extend(taken_ix.iter().map(|ix| {
        let mut ex = old_blocks[*ix].clone();
        ex.score = ranked
            .iter()
            .find(|(i, _)| i == ix)
            .map(|(_, s)| *s)
            .unwrap_or(0.0);
        ex
    }));
    primary.extend(window);
    debug_assert!(total_cost(&primary) <= budget.max(1));
    ExcerptPlan {
        meetings_in_scope: 1,
        meetings_with_hits: u32::from(!primary.is_empty()),
        primary,
        secondary,
        cards: Vec::new(),
        truncated,
        lexical_only: true,
    }
}

/// Die juengsten Zeilen eines Transkript-Auszugs, die zusammen in `budget`
/// passen (`None`, wenn nicht einmal eine passt).
fn newest_lines_within(mut ex: Excerpt, budget: usize) -> Option<Excerpt> {
    while !ex.lines.is_empty() && ex.cost() > budget {
        ex.lines.remove(0);
    }
    let first = ex.lines.first()?;
    ex.start_ms = first.start_ms;
    Some(ex)
}

#[cfg(test)]
mod tests {
    use super::super::context::tests::meeting;
    use super::*;
    use crate::managers::meetings::notes::model::NoteBlockKind;

    /// 60 Segmente im Abstand von 30 s (30 Minuten); die ersten zehn handeln
    /// vom Budget, der Rest vom Wetter.
    fn snapshot(position_ms: Option<u64>) -> LiveSnapshot {
        let segments = (0..60u32)
            .map(|i| StoredSegment {
                segment_index: i,
                text: if i < 10 {
                    format!("Das Budget fuer Posten {i} liegt bei {} Euro.", i * 100)
                } else {
                    format!("Heute ist das Wetter an Stelle {i} recht ordentlich gewesen.")
                },
                start_ms: u64::from(i) * 30_000,
                end_ms: u64::from(i) * 30_000 + 25_000,
                channel: 0,
                speaker_index: None,
                words: None,
            })
            .collect();
        LiveSnapshot {
            meeting: meeting("live"),
            epoch: 2,
            segments,
            notes: vec![NoteBlock {
                id: "n1".into(),
                kind: NoteBlockKind::Bullet,
                text: "Angebot nachfassen".into(),
                at_ms: Some(60_000),
                checked: false,
            }],
            position_ms,
        }
    }

    fn segment_ids(excerpts: &[Excerpt]) -> Vec<u32> {
        excerpts
            .iter()
            .flat_map(|e| e.lines.iter().filter_map(|l| l.segment_index))
            .collect()
    }

    #[test]
    fn recent_window_always_included() {
        let snap = snapshot(Some(30 * 60_000));
        // Fenster: ab 22:00 -> Segmente 44..=59.
        let window: Vec<u32> = (44..60).collect();

        let plan = live_plan(&snap, "Wie hoch ist das Budget?", 20_000);
        let ids = segment_ids(&plan.primary);
        assert!(
            window.iter().all(|i| ids.contains(i)),
            "Fenster fehlt: {ids:?}"
        );
        assert!(
            ids.contains(&0) || ids.contains(&5),
            "Budget-Stellen gelesen: {ids:?}"
        );
        assert!(
            plan.primary
                .iter()
                .filter(|e| e.source
                    == crate::managers::meetings::search::chunking::ChunkSource::Transcript)
                .all(|e| e.epoch == 2),
            "Live-Epoche"
        );
        assert_eq!(
            plan.primary[0].lines[0].ref_key.as_deref(),
            Some("n1"),
            "Notizen zuerst"
        );
        assert_eq!(*ids.last().unwrap(), 59, "juengstes Fenster am Ende");

        // Auch ohne passende Frage (nur Stoppwoerter) ist das Fenster da.
        let plan = live_plan(&snap, "Was ist?", 20_000);
        let ids = segment_ids(&plan.primary);
        assert!(window.iter().all(|i| ids.contains(i)));
        assert!(!ids.contains(&0), "ohne Treffer keine alten Stellen");
        assert!(
            !plan.secondary.is_empty(),
            "aeltere Stellen bleiben Kandidaten"
        );

        // Knappes Budget: das Fenster verdraengt die aelteren Treffer, die
        // neuesten Segmente bleiben.
        let plan = live_plan(&snap, "Wie hoch ist das Budget?", 900);
        let ids = segment_ids(&plan.primary);
        assert!(ids.contains(&59), "neuestes Segment: {ids:?}");
        assert!(ids.iter().all(|i| *i >= 44), "keine alten Stellen: {ids:?}");
        assert!(plan.truncated);
        assert!(
            !plan.secondary.is_empty(),
            "Budget-Stellen fuer die Wiederholung"
        );
        assert!(total_cost(&plan.primary) <= 900);
    }

    #[test]
    fn without_a_position_the_last_segment_end_counts() {
        let snap = snapshot(None);
        assert_eq!(snap.end_ms(), 59 * 30_000 + 25_000);
        let plan = live_plan(&snap, "Budget", 50_000);
        let ids = segment_ids(&plan.primary);
        // Fenster ab 29:55 - 8:00 = 21:55 -> ab Segment 44 (22:00).
        assert!(ids.contains(&44) && ids.contains(&59));
    }

    #[test]
    fn the_snapshot_reads_segments_notes_and_epoch_from_the_store() {
        use crate::managers::meetings::search::index::tests::{ready_meeting, tmp_store};
        let (_dir, store) = tmp_store();
        let m = ready_meeting(&store, "Laufend", 1_000);
        let snap = LiveSnapshot::from_store(&store, &m.id, Some(5_000))
            .unwrap()
            .unwrap();
        assert_eq!(snap.meeting.title, "Laufend");
        assert_eq!(snap.position_ms, Some(5_000));
        assert!(snap.segments.is_empty() && snap.notes.is_empty());
        assert!(LiveSnapshot::from_store(&store, "gibt-es-nicht", None)
            .unwrap()
            .is_none());
    }
}
