//! lva#806 P1a: dauerhafte Steuerung und Blockcursor nur fuer Hoerbuch-Importe.
//! Besprechungen und Neu-Transkriptionen behalten ihre bisherigen Vertraege.

use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::store::{MeetingStore, TranscriptDelta};

pub(super) const MIGRATION: &str = include_str!("audiobook.sql");
pub const CHUNK_TARGET_MS: u64 = 20_000;
pub const YIELDED: &str = "audiobook_yielded";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AudiobookTempo {
    Gentle,
    #[default]
    Balanced,
    Fast,
}

/// Gesamtbudget und gleichzeitige Laeufe. ONNX nutzt bis P1b sein eigenes Budget.
pub fn budget(cpus: usize, tempo: AudiobookTempo) -> (usize, usize) {
    let cpus = cpus.max(1);
    let threads = crate::process_guard::cpu_threads_for(cpus).min(cpus);
    match tempo {
        AudiobookTempo::Gentle => (1, (threads / 2).max(1)),
        AudiobookTempo::Balanced => ((cpus / 4).clamp(1, 2), threads),
        AudiobookTempo::Fast => (super::queue::bulk_parallel(cpus), threads),
    }
}

/// Nur dieser Import-Thread sieht das Budget; Diktat und Neu-Laeufe nicht.
thread_local! {
    static THREAD_BUDGET: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
}

pub fn thread_budget() -> Option<i32> {
    THREAD_BUDGET.with(|budget| budget.get())
}

pub struct BudgetGuard(Option<i32>, std::marker::PhantomData<*const ()>);
impl BudgetGuard {
    pub fn enter(threads: usize) -> Self {
        Self(
            THREAD_BUDGET.with(|budget| budget.replace(Some(threads.max(1) as i32))),
            std::marker::PhantomData,
        )
    }
}
impl Drop for BudgetGuard {
    fn drop(&mut self) {
        THREAD_BUDGET.with(|budget| budget.set(self.0));
    }
}

pub fn foreground_hold(app: &tauri::AppHandle) -> bool {
    use std::sync::Arc;
    use tauri::Manager;
    crate::background_activity::tts_start_active()
        || app
            .try_state::<Arc<crate::managers::audio::AudioRecordingManager>>()
            .is_some_and(|audio| audio.is_recording())
        || app
            .try_state::<Arc<super::recorder::MeetingRecorderManager>>()
            .is_some_and(|recorder| recorder.is_recording())
        || app
            .try_state::<Arc<crate::managers::tts::TtsManager>>()
            .is_some_and(|tts| {
                tts.status().phase == crate::managers::tts::state::TtsPhase::Starting
            })
}

pub fn app_budget(app: &tauri::AppHandle) -> (usize, usize) {
    use std::sync::Arc;
    use tauri::Manager;
    let reading = app
        .try_state::<Arc<crate::managers::tts::TtsManager>>()
        .is_some_and(|tts| tts.status().phase == crate::managers::tts::state::TtsPhase::Speaking);
    let tempo = if reading {
        AudiobookTempo::Gentle
    } else {
        crate::settings::get_settings(app).audiobook_tempo
    };
    budget(crate::process_guard::logical_cpus(), tempo)
}

fn hold_block(user_paused: bool, foreground: bool, lane: usize, parallel: usize) -> bool {
    user_paused || foreground || lane >= parallel.max(1)
}

/// Pollt getrennt von der Nutzerpause: ein endender Vordergrund-Hold kann
/// niemals eine gespeicherte Nutzerpause aufheben. Vor JEDEM Hoerbuchblock.
pub fn checkpoint(
    app: &tauri::AppHandle,
    store: &MeetingStore,
    id: &str,
    job: &super::job::JobHandle,
) -> Result<bool, String> {
    use super::job::Gate;
    use std::time::{Duration, Instant};
    let started = Instant::now();
    let mut held = false;
    loop {
        let ctl = store
            .audiobook_control(id)
            .map_err(|_| "audiobook_control_unreadable")?
            .ok_or("not_audiobook")?;
        if ctl.desired_state == "cancelled" || job.is_stopped() {
            return Ok(false);
        }
        // Der Cursor ist am Blockende gesichert. Thread und Engine gehen frei;
        // Fortsetzen beansprucht seinen Platz erneut ueber die Warteschlange.
        if ctl.desired_state == "paused" {
            return Err(YIELDED.into());
        }
        let parallel = app_budget(app).0;
        let rows = store
            .queue_rows()
            .map_err(|_| "audiobook_queue_unreadable")?;
        let lane = rows
            .iter()
            .filter(|r| r.is_audiobook() && r.state == super::queue_store::STATE_RUNNING)
            .position(|r| r.meeting_id == id)
            .unwrap_or(0);
        let hold = hold_block(
            ctl.desired_state == "paused",
            foreground_hold(app),
            lane,
            parallel,
        );
        if hold {
            held = true;
            if !job.is_paused() {
                let _ = job.pause();
            }
            if started.elapsed() >= Duration::from_secs(10) {
                crate::managers::transcription::suspend_bound_engine();
            }
        } else if held {
            let _ = job.resume();
        }
        // In Prepare kann JobControl noch nicht pausieren; das dauerhafte Tor
        // haelt trotzdem vor Modellaufbau und Sprachprobe.
        if !hold {
            if let Some(gate) = job.checkpoint_once(false) {
                return Ok(matches!(gate, Gate::Go { .. }));
            }
        } else {
            let _ = job.checkpoint_once(false);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Windows: nur den Hoerbuch-Arbeitsthread, mit Rueckkehr zur alten Prioritaet.
/// Native Modellpools werden dadurch nicht vollstaendig erfasst (P1b).
pub struct ThreadPriority {
    #[cfg(windows)]
    previous: i32,
    _not_send: std::marker::PhantomData<*const ()>,
}
impl ThreadPriority {
    pub fn enter() -> Self {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Threading::{
                GetCurrentThread, GetThreadPriority, SetThreadPriority,
                THREAD_PRIORITY_BELOW_NORMAL,
            };
            let previous = GetThreadPriority(GetCurrentThread());
            if previous != i32::MAX {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
            }
            Self {
                previous,
                _not_send: std::marker::PhantomData,
            }
        }
        #[cfg(not(windows))]
        Self {
            _not_send: std::marker::PhantomData,
        }
    }
}
impl Drop for ThreadPriority {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_PRIORITY,
            };
            if self.previous != i32::MAX {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(self.previous));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    pub desired_state: String,
    pub keep_partial: bool,
    pub generation: u64,
    pub next_sample: Option<u64>,
}

impl MeetingStore {
    /// Alle Generationen pruefen und sperren, bevor das Frontend Seiten aendert.
    pub fn audiobook_claim(&self, book_id: &str, generations: &[(String, u64)]) -> Result<String> {
        if generations.is_empty() {
            return Err(anyhow!("stale_audiobook_completion"));
        }
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let token = ulid::Ulid::new().to_string();
        for (id, generation) in generations {
            let valid: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM audiobook_control c JOIN meetings m ON m.id = c.meeting_id
                 WHERE c.meeting_id = ?1 AND c.generation = ?2 AND c.desired_state = 'running'
                 AND m.status = 'ready' AND m.deleted_at IS NULL
                 AND NOT EXISTS(SELECT 1 FROM audiobook_adoption a WHERE a.meeting_id = c.meeting_id))",
                params![id, *generation as i64], |r| r.get(0),
            )?;
            if !valid {
                return Err(anyhow!("stale_audiobook_completion"));
            }
            tx.execute(
                "INSERT INTO audiobook_adoption(meeting_id, book_id, token) VALUES (?1, ?2, ?3)",
                params![id, book_id, token],
            )?;
        }
        tx.commit()?;
        Ok(token)
    }

    pub fn audiobook_check_claim(&self, book_id: &str, token: &str, ids: &[String]) -> Result<()> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare("SELECT meeting_id FROM audiobook_adoption WHERE book_id = ?1 AND token = ?2 ORDER BY meeting_id")?;
        let claimed = stmt
            .query_map(params![book_id, token], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut expected = ids.to_vec();
        expected.sort();
        if claimed.is_empty() || claimed != expected {
            return Err(anyhow!("stale_audiobook_completion"));
        }
        Ok(())
    }

    pub fn audiobook_release_claim(&self, book_id: &str, token: &str) -> Result<()> {
        self.get_connection()?.execute(
            "DELETE FROM audiobook_adoption WHERE book_id = ?1 AND token = ?2",
            params![book_id, token],
        )?;
        Ok(())
    }

    /// Requeue und Abbruch lesen denselben dauerhaften Stand in EINER Transaktion.
    pub fn audiobook_yield(&self, id: &str) -> Result<bool> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: String = tx.query_row(
            "SELECT desired_state FROM audiobook_control WHERE meeting_id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        let waiting = state != "cancelled";
        if waiting {
            tx.execute(
                "INSERT INTO import_queue(meeting_id, seq, state, enqueued_at)
                VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM import_queue), 'waiting', 0)
                ON CONFLICT(meeting_id) DO UPDATE SET state = 'waiting'",
                params![id],
            )?;
            tx.execute("UPDATE meetings SET status = 'queued' WHERE id = ?1 AND deleted_at IS NULL AND status = 'processing'", params![id])?;
        } else {
            tx.execute(
                "DELETE FROM import_queue WHERE meeting_id = ?1",
                params![id],
            )?;
            tx.execute(
                "UPDATE meetings SET status = 'cancelled' WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
            )?;
        }
        tx.commit()?;
        Ok(waiting)
    }

    /// Fehlerabschluss ebenfalls atomar; Abbruch hat weiterhin Vorrang.
    pub fn audiobook_fail_yield(&self, id: &str) -> Result<bool> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE meetings SET status = CASE WHEN EXISTS(
            SELECT 1 FROM audiobook_control WHERE meeting_id = ?1 AND desired_state = 'cancelled')
            THEN 'cancelled' ELSE 'failed' END WHERE id = ?1 AND deleted_at IS NULL
            AND status IN ('processing', 'queued')",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM import_queue WHERE meeting_id = ?1",
            params![id],
        )?;
        let failed = tx.query_row("SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1 AND status = 'failed' AND deleted_at IS NULL)", params![id], |r| r.get(0))?;
        tx.commit()?;
        Ok(failed)
    }

    pub fn audiobook_completion_matches(&self, id: &str, generation: u64) -> Result<bool> {
        Ok(self.get_connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM audiobook_control c JOIN meetings m ON m.id = c.meeting_id
             WHERE c.meeting_id = ?1 AND c.generation = ?2 AND c.desired_state = 'running'
             AND m.status = 'ready' AND m.deleted_at IS NULL)",
            params![id, generation as i64],
            |r| r.get(0),
        )?)
    }

    pub fn audiobook_control(&self, id: &str) -> Result<Option<Control>> {
        Ok(self.get_connection()?.query_row(
            "SELECT desired_state, keep_partial, generation, next_sample FROM audiobook_control WHERE meeting_id = ?1",
            params![id],
            |r| Ok(Control { desired_state: r.get(0)?, keep_partial: r.get(1)?, generation: r.get::<_, i64>(2)? as u64, next_sample: r.get::<_, Option<i64>>(3)?.map(|v| v as u64) }),
        ).optional()?)
    }

    /// Alle Teile eines Buchs aendern sich gemeinsam. Laufende Bloecke enden am
    /// Kontrollpunkt; beim Verwerfen sperrt die Generation auch spaete Ergebnisse.
    pub fn audiobook_set_state(&self, ids: &[String], state: &str, keep: bool) -> Result<()> {
        self.audiobook_set_state_active(ids, state, keep, &[])
    }

    pub fn audiobook_set_state_active(
        &self,
        ids: &[String],
        state: &str,
        keep: bool,
        active: &[String],
    ) -> Result<()> {
        if !matches!(state, "running" | "paused" | "cancelled") {
            return Err(anyhow!("invalid_audiobook_state"));
        }
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for id in ids {
            let adopting: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM audiobook_adoption WHERE meeting_id = ?1)",
                params![id],
                |r| r.get(0),
            )?;
            if adopting {
                return Err(anyhow!("audiobook_adopting"));
            }
            let valid: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM import_part p JOIN meetings m ON m.id = p.meeting_id
                 WHERE p.meeting_id = ?1 AND p.kind = 'audiobook' AND m.deleted_at IS NULL)",
                params![id],
                |r| r.get(0),
            )?;
            if !valid {
                return Err(anyhow!("not_audiobook"));
            }
            // Auch fertige Teile sperren die Uebernahme bei Pause/Abbruch.
            // Ihr Text bleibt bis DiscardPartial unveraendert.
            let ready: bool = tx.query_row(
                "SELECT status = 'ready' FROM meetings WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )?;
            tx.execute(
                "UPDATE audiobook_control SET desired_state = ?2, keep_partial = ?3,
                 generation = generation + CASE WHEN ?2 = 'cancelled' AND ?3 = 0 THEN 1 ELSE 0 END
                 WHERE meeting_id = ?1",
                params![id, state, keep],
            )?;
            if ready && state != "cancelled" {
                continue;
            }
            if state == "cancelled" {
                tx.execute(
                    "DELETE FROM import_queue WHERE meeting_id = ?1 AND state = 'waiting'",
                    params![id],
                )?;
                tx.execute("UPDATE meetings SET status = 'cancelled' WHERE id = ?1 AND (status = 'queued' OR (status = 'ready' AND ?2 = 0))", params![id, keep])?;
            } else if state == "running" {
                // Ein bereits laufender Auftrag bleibt beim selben Thread. Ansonsten
                // denselben Import mit Text, Bereich und Cursor erneut einreihen.
                if active.contains(id) {
                    continue;
                }
                tx.execute(
                    "INSERT INTO import_queue (meeting_id, seq, state, enqueued_at)
                     VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM import_queue), 'waiting', 0)
                     ON CONFLICT(meeting_id) DO UPDATE SET state = 'waiting'",
                    params![id],
                )?;
                tx.execute("UPDATE meetings SET status = 'queued' WHERE id = ?1 AND status IN ('cancelled', 'failed', 'processing', 'queued')", params![id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Altauftraege: konservativ ab letztem Segmentende, einmalig initialisieren.
    pub fn audiobook_cursor(&self, id: &str) -> Result<(u64, u64)> {
        let segments = self.get_segments(id)?;
        let legacy = segments
            .iter()
            .map(|s| s.end_ms)
            .max()
            .unwrap_or(0)
            .saturating_mul(16);
        let conn = self.get_connection()?;
        conn.execute("UPDATE audiobook_control SET next_sample = ?2 WHERE meeting_id = ?1 AND next_sample IS NULL", params![id, legacy as i64])?;
        let ctl = self
            .audiobook_control(id)?
            .ok_or_else(|| anyhow!("not_audiobook"))?;
        Ok((ctl.generation, ctl.next_sample.unwrap_or(0)))
    }

    /// Cursor und Delta sind EINE Transaktion, auch fuer einen stillen Block.
    /// `false` bedeutet: genau dieser Block wurde schon bestaetigt.
    pub fn append_audiobook_block(
        &self,
        id: &str,
        generation: u64,
        start: u64,
        end: u64,
        delta: &TranscriptDelta,
    ) -> Result<bool> {
        let _tor =
            crate::managers::planning::schreibschutz(self, id).map_err(|code| anyhow!(code))?;
        if end <= start {
            return Err(anyhow!("invalid_audiobook_block"));
        }
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ctl: (u64, Option<u64>, bool) = tx.query_row(
            "SELECT generation, next_sample, desired_state = 'cancelled' AND keep_partial = 0 FROM audiobook_control WHERE meeting_id = ?1",
            params![id], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, Option<i64>>(1)?.map(|v| v as u64), r.get(2)?)),
        )?;
        if ctl.0 != generation || ctl.2 {
            return Err(anyhow!("stale_audiobook_block"));
        }
        let committed: Option<u64> = tx.query_row(
            "SELECT end_sample FROM audiobook_chunk WHERE meeting_id = ?1 AND generation = ?2 AND start_sample = ?3",
            params![id, generation as i64, start as i64], |r| Ok(r.get::<_, i64>(0)? as u64),
        ).optional()?;
        if committed == Some(end) {
            return Ok(false);
        }
        if committed.is_some() || ctl.1 != Some(start) {
            return Err(anyhow!("audiobook_cursor_mismatch"));
        }
        Self::append_delta_tx(&tx, id, delta)?;
        tx.execute("INSERT INTO audiobook_chunk (meeting_id, generation, start_sample, end_sample) VALUES (?1, ?2, ?3, ?4)", params![id, generation as i64, start as i64, end as i64])?;
        tx.execute(
            "UPDATE audiobook_control SET next_sample = ?2 WHERE meeting_id = ?1",
            params![id, end as i64],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Nur die Arbeitskopie dieses Hilfsimports, niemals Audio oder Fassungen.
    /// Nach dem Ende des Job-Threads (oder beim Neustart) aufrufen.
    pub fn audiobook_discard_cancelled(&self, id: &str) -> Result<()> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let discard: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM audiobook_control WHERE meeting_id = ?1 AND desired_state = 'cancelled' AND keep_partial = 0)",
            params![id], |r| r.get(0),
        )?;
        if discard {
            tx.execute("DELETE FROM transcript_deltas WHERE transcript_id IN (SELECT id FROM transcripts WHERE meeting_id = ?1)", params![id])?;
            tx.execute("UPDATE transcripts SET segments_json = '[]', content_revision = content_revision + 1 WHERE meeting_id = ?1", params![id])?;
            super::store::bump_ki_revision(&tx)?;
            tx.execute(
                "DELETE FROM audiobook_chunk WHERE meeting_id = ?1",
                params![id],
            )?;
            tx.execute(
                "UPDATE audiobook_control SET next_sample = 0 WHERE meeting_id = ?1",
                params![id],
            )?;
        }
        tx.execute(
            "UPDATE meetings SET status = 'cancelled' WHERE id = ?1 AND deleted_at IS NULL
             AND EXISTS(SELECT 1 FROM audiobook_control WHERE meeting_id = ?1 AND desired_state = 'cancelled')",
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn audiobook_retains_audio(&self, id: &str) -> Result<bool> {
        Ok(self.get_connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM audiobook_control c JOIN meetings m ON m.id = c.meeting_id
             WHERE c.meeting_id = ?1 AND m.status <> 'ready' AND c.keep_partial = 1)",
            params![id],
            |r| r.get(0),
        )?)
    }

    pub fn audiobook_mark_ready(&self, id: &str) -> Result<bool> {
        let conn = self.get_connection()?;
        Ok(conn.execute(
            "UPDATE meetings SET status = 'ready' WHERE id = ?1 AND deleted_at IS NULL
             AND EXISTS (SELECT 1 FROM audiobook_control c WHERE c.meeting_id = ?1 AND c.desired_state = 'running')",
            params![id],
        )? == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::search::index::tests::tmp_store;
    use crate::managers::meetings::store::StoredSegment;

    fn part(s: &MeetingStore) -> String {
        s.queue_enqueue_part("synthetic", "synthetic.wav", Some(1), None)
            .unwrap()
            .id
    }

    #[test]
    fn upgrade_21_to_22_preserves_existing_parts() {
        use rusqlite_migration::Migrations;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            Migrations::new(super::super::store::MIGRATIONS[..21].to_vec())
                .to_latest(&mut conn)
                .unwrap();
            conn.execute_batch(
                "INSERT INTO meetings(id,title,status,source,created_at,updated_at)
                 VALUES ('legacy','synthetic','cancelled','import',0,0);
                 INSERT INTO import_part(meeting_id,kind,range_start_ms,range_end_ms)
                 VALUES ('legacy','audiobook',1000,2000);",
            )
            .unwrap();
        }
        let s = MeetingStore::open_at(&path).unwrap();
        assert_eq!(s.queue_part("legacy").unwrap().1, Some((1000, Some(2000))));
        let ctl = s.audiobook_control("legacy").unwrap().unwrap();
        assert_eq!(
            (ctl.desired_state.as_str(), ctl.generation, ctl.next_sample),
            ("running", 0, None)
        );
        assert_eq!(
            s.get_meeting("legacy").unwrap().unwrap().status,
            "cancelled"
        );
    }

    #[test]
    fn completion_rejects_discard_even_after_resume_with_the_same_id() {
        let (_dir, s) = tmp_store();
        let id = part(&s);
        s.set_status(&id, super::super::store::MeetingStatus::Ready)
            .unwrap();
        assert!(s.audiobook_completion_matches(&id, 0).unwrap());
        s.audiobook_set_state(&[id.clone()], "cancelled", false)
            .unwrap();
        assert!(!s.audiobook_completion_matches(&id, 0).unwrap());
        s.audiobook_set_state(&[id.clone()], "running", true)
            .unwrap();
        s.set_status(&id, super::super::store::MeetingStatus::Ready)
            .unwrap();
        assert!(!s.audiobook_completion_matches(&id, 0).unwrap());
        assert!(s.audiobook_completion_matches(&id, 1).unwrap());
    }

    #[test]
    fn adoption_claim_blocks_control_and_requires_its_token_until_release_or_restart() {
        let (dir, s) = tmp_store();
        let ids = [part(&s), part(&s)];
        for id in &ids {
            s.set_status(id, super::super::store::MeetingStatus::Ready)
                .unwrap();
        }
        let generations = ids.iter().map(|id| (id.clone(), 0)).collect::<Vec<_>>();
        assert!(s
            .audiobook_claim("book", &[(ids[0].clone(), 0), (ids[1].clone(), 1)])
            .is_err());
        let token = s.audiobook_claim("book", &generations).unwrap();
        assert!(s.audiobook_claim("book", &generations).is_err());
        for state in ["paused", "cancelled", "running"] {
            assert_eq!(
                s.audiobook_set_state(&ids, state, false)
                    .unwrap_err()
                    .to_string(),
                "audiobook_adopting"
            );
        }
        assert!(s.audiobook_check_claim("book", "wrong", &ids).is_err());
        s.audiobook_check_claim("book", &token, &ids).unwrap();
        s.audiobook_release_claim("book", "wrong").unwrap();
        s.audiobook_check_claim("book", &token, &ids).unwrap();
        s.audiobook_release_claim("book", &token).unwrap();
        assert!(s.audiobook_check_claim("book", &token, &ids).is_err());
        s.audiobook_set_state(&ids, "cancelled", false).unwrap();
        assert!(s.audiobook_claim("book", &generations).is_err());
        s.audiobook_set_state(&ids, "running", true).unwrap();
        for id in &ids {
            s.set_status(id, super::super::store::MeetingStatus::Ready)
                .unwrap();
        }
        let token = s
            .audiobook_claim(
                "book",
                &ids.iter().map(|id| (id.clone(), 1)).collect::<Vec<_>>(),
            )
            .unwrap();
        drop(s);
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        s.audiobook_check_claim("book", &token, &ids).unwrap();
        s.queue_recover().unwrap();
        s.audiobook_set_state(&ids, "paused", true).unwrap();
    }

    #[test]
    fn upgrade_22_to_23_keeps_the_audiobook_cursor_and_pause() {
        use rusqlite_migration::Migrations;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            Migrations::new(super::super::store::MIGRATIONS[..22].to_vec())
                .to_latest(&mut conn)
                .unwrap();
            conn.execute_batch("INSERT INTO meetings(id,title,status,source,created_at,updated_at)
                VALUES ('legacy','synthetic','queued','import',0,0);
                INSERT INTO import_part(meeting_id,kind) VALUES ('legacy','audiobook');
                UPDATE audiobook_control SET desired_state = 'paused', generation = 3, next_sample = 160
                WHERE meeting_id = 'legacy';").unwrap();
        }
        let s = MeetingStore::open_at(&path).unwrap();
        let ctl = s.audiobook_control("legacy").unwrap().unwrap();
        assert_eq!(
            (ctl.desired_state.as_str(), ctl.generation, ctl.next_sample),
            ("paused", 3, Some(160))
        );
        s.audiobook_release_claim("book", "missing").unwrap();
    }

    #[test]
    fn resume_repairs_processing_without_an_active_run_and_survives_prune() {
        let (_dir, s) = tmp_store();
        let id = part(&s);
        s.queue_mark_running(&id).unwrap();
        s.queue_finish(&id).unwrap();
        s.audiobook_set_state(&[id.clone()], "running", true)
            .unwrap();
        s.queue_prune().unwrap();
        assert_eq!(s.get_meeting(&id).unwrap().unwrap().status, "queued");
        assert_eq!(s.queue_rows().unwrap()[0].state, "waiting");
    }

    #[test]
    fn failed_yield_is_retryable_and_never_overrides_cancel() {
        let (_dir, s) = tmp_store();
        let id = part(&s);
        s.queue_mark_running(&id).unwrap();
        let conn = s.get_connection().unwrap();
        conn.execute_batch("CREATE TRIGGER reject_yield BEFORE UPDATE ON import_queue BEGIN SELECT RAISE(ABORT, 'synthetic'); END;").unwrap();
        assert!(s.audiobook_yield(&id).is_err());
        assert_eq!(s.get_meeting(&id).unwrap().unwrap().status, "processing");
        conn.execute_batch("DROP TRIGGER reject_yield;").unwrap();
        assert!(s.audiobook_yield(&id).unwrap());
        s.audiobook_set_state(&[id.clone()], "cancelled", true)
            .unwrap();
        assert!(!s.audiobook_yield(&id).unwrap());
        assert_eq!(s.get_meeting(&id).unwrap().unwrap().status, "cancelled");
        assert!(s.queue_rows().unwrap().is_empty());
    }

    #[test]
    fn tempo_budget_including_small_machines() {
        for c in [0, 1, 2, 4, 8, 16, 32, 64] {
            let (p, t) = budget(c, AudiobookTempo::Gentle);
            assert_eq!(p, 1);
            assert!(t >= 1 && t <= c.max(1));
            assert_eq!(
                budget(c, AudiobookTempo::Balanced).0,
                (c.max(1) / 4).clamp(1, 2)
            );
            assert_eq!(
                budget(c, AudiobookTempo::Fast).0,
                super::super::queue::bulk_parallel(c)
            );
        }
        assert_eq!(budget(4, AudiobookTempo::Gentle), (1, 2));
        assert_eq!(budget(8, AudiobookTempo::Balanced), (2, 4));
        assert_eq!(budget(32, AudiobookTempo::Fast), (4, 16));
    }

    #[test]
    fn foreground_end_and_tempo_changes_never_clear_user_pause() {
        assert!(hold_block(false, true, 0, 2));
        assert!(!hold_block(false, false, 0, 2));
        assert!(hold_block(true, false, 0, 2));
        assert!(hold_block(false, false, 1, 1));
        assert!(!hold_block(false, false, 1, 2));
    }

    #[test]
    fn silence_duplicate_and_crash_before_ack_keep_exact_cursor() {
        let (dir, s) = tmp_store();
        let id = part(&s);
        let (g, start) = s.audiobook_cursor(&id).unwrap();
        assert_eq!(start, 0);
        let silence = TranscriptDelta {
            new_segments: vec![],
        };
        assert!(s
            .append_audiobook_block(&id, g, 0, 320_000, &silence)
            .unwrap());
        drop(s);
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        assert!(!s
            .append_audiobook_block(&id, g, 0, 320_000, &silence)
            .unwrap());
        assert_eq!(s.audiobook_cursor(&id).unwrap(), (g, 320_000));
        assert!(s.get_segments(&id).unwrap().is_empty());
        assert!(s
            .append_audiobook_block(&id, g, 0, 320_001, &silence)
            .is_err());
    }

    #[test]
    fn failed_cursor_write_rolls_back_text_and_delta() {
        let (_dir, s) = tmp_store();
        let id = part(&s);
        s.audiobook_cursor(&id).unwrap();
        s.get_connection().unwrap().execute_batch(
            "CREATE TRIGGER fail_cursor BEFORE UPDATE OF next_sample ON audiobook_control BEGIN SELECT RAISE(ABORT, 'synthetic'); END;"
        ).unwrap();
        let delta = TranscriptDelta {
            new_segments: vec![StoredSegment {
                segment_index: 0,
                text: "synthetic".into(),
                start_ms: 0,
                end_ms: 10,
                channel: 0,
                speaker_index: None,
                words: None,
            }],
        };
        assert!(s
            .append_audiobook_block(&id, 0, 0, 320_000, &delta)
            .is_err());
        assert!(s.get_segments(&id).unwrap().is_empty());
        assert_eq!(
            s.audiobook_control(&id).unwrap().unwrap().next_sample,
            Some(0)
        );
        let n: i64 = s
            .get_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM transcript_deltas", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn pause_between_claim_and_start_survives_recovery() {
        let (dir, s) = tmp_store();
        let id = part(&s);
        assert!(s.queue_mark_running(&id).unwrap());
        s.audiobook_set_state(&[id.clone()], "paused", true)
            .unwrap();
        s.queue_recover().unwrap();
        drop(s);
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        assert_eq!(
            s.audiobook_control(&id).unwrap().unwrap().desired_state,
            "paused"
        );
        assert!(!s.queue_mark_running(&id).unwrap());
        s.audiobook_set_state(&[id.clone()], "running", true)
            .unwrap();
        assert!(s.queue_mark_running(&id).unwrap());
    }

    #[test]
    fn cancel_keeps_cursor_or_discards_only_this_result() {
        let (_dir, s) = tmp_store();
        let id = part(&s);
        let other = part(&s);
        s.audiobook_cursor(&id).unwrap();
        s.audiobook_cursor(&other).unwrap();
        let delta = TranscriptDelta {
            new_segments: vec![StoredSegment {
                segment_index: 0,
                text: "synthetic".into(),
                start_ms: 0,
                end_ms: 10,
                channel: 2,
                speaker_index: None,
                words: None,
            }],
        };
        s.append_audiobook_block(&id, 0, 0, 160, &delta).unwrap();
        assert!(!s.append_audiobook_block(&id, 0, 0, 160, &delta).unwrap());
        s.append_audiobook_block(&other, 0, 0, 160, &delta).unwrap();
        s.audiobook_set_state(&[id.clone()], "cancelled", true)
            .unwrap();
        assert!(s.audiobook_retains_audio(&id).unwrap());
        s.audiobook_discard_cancelled(&id).unwrap();
        assert_eq!(s.audiobook_cursor(&id).unwrap().1, 160);
        assert_eq!(s.get_segments(&id).unwrap().len(), 1);
        s.set_status(&id, super::super::store::MeetingStatus::Ready)
            .unwrap();
        s.audiobook_set_state(&[id.clone()], "cancelled", false)
            .unwrap();
        s.audiobook_discard_cancelled(&id).unwrap();
        assert_eq!(s.audiobook_cursor(&id).unwrap(), (1, 0));
        assert!(s.get_segments(&id).unwrap().is_empty());
        assert_eq!(s.get_segments(&other).unwrap().len(), 1);
        assert!(s
            .append_audiobook_block(
                &id,
                0,
                160,
                320,
                &TranscriptDelta {
                    new_segments: vec![]
                }
            )
            .is_err());
        assert!(s.get_meeting(&other).unwrap().is_some());
        assert!(s.audiobook_control(&other).unwrap().is_some());
    }

    #[test]
    fn automatic_retention_preserves_resumable_audio_but_still_purges_meetings() {
        let (dir, s) = tmp_store();
        let id = part(&s);
        let meeting = s
            .queue_enqueue("synthetic", "synthetic.wav", Some(1))
            .unwrap()
            .id;
        let book_wav = dir.path().join("retained.wav");
        let meeting_wav = dir.path().join("expired.wav");
        std::fs::write(&book_wav, b"synthetic").unwrap();
        std::fs::write(&meeting_wav, b"synthetic").unwrap();
        for (id, wav) in [(&id, &book_wav), (&meeting, &meeting_wav)] {
            s.set_audio_paths(id, wav.to_str(), None, Some(10)).unwrap();
            s.set_retention_until(id, Some(0)).unwrap();
        }
        s.audiobook_set_state(&[id], "cancelled", true).unwrap();
        assert_eq!(super::super::retention::purge_due_audio(&s, 1).unwrap(), 1);
        assert!(book_wav.exists());
        assert!(!meeting_wav.exists());
    }

    #[test]
    fn ordinary_meetings_keep_their_claim_and_delta_contract() {
        let (_dir, s) = tmp_store();
        let meeting = s
            .queue_enqueue("synthetic", "synthetic.wav", Some(1))
            .unwrap();
        assert!(s.audiobook_control(&meeting.id).unwrap().is_none());
        assert!(s.queue_mark_running(&meeting.id).unwrap());
        assert!(!s.queue_mark_running(&meeting.id).unwrap());
        let silence = TranscriptDelta {
            new_segments: vec![],
        };
        assert_eq!(s.append_delta(&meeting.id, &silence).unwrap(), 1);
        assert_eq!(s.append_delta(&meeting.id, &silence).unwrap(), 2);
        assert!(!s.audiobook_retains_audio(&meeting.id).unwrap());
        assert!(thread_budget().is_none());
        {
            let _budget = BudgetGuard::enter(1);
            assert_eq!(thread_budget(), Some(1));
            assert!(std::thread::spawn(thread_budget).join().unwrap().is_none());
        }
        assert!(thread_budget().is_none());
    }

    #[test]
    fn legacy_import_initializes_cursor_at_last_segment_end_only_once() {
        let (_dir, s) = tmp_store();
        let id = part(&s);
        s.append_delta(
            &id,
            &TranscriptDelta {
                new_segments: vec![StoredSegment {
                    segment_index: 0,
                    text: "synthetic".into(),
                    start_ms: 0,
                    end_ms: 1500,
                    channel: 2,
                    speaker_index: None,
                    words: None,
                }],
            },
        )
        .unwrap();
        assert_eq!(s.audiobook_cursor(&id).unwrap(), (0, 24_000));
        s.append_audiobook_block(
            &id,
            0,
            24_000,
            32_000,
            &TranscriptDelta {
                new_segments: vec![],
            },
        )
        .unwrap();
        assert_eq!(s.audiobook_cursor(&id).unwrap(), (0, 32_000));
        assert_eq!(s.get_segments(&id).unwrap().len(), 1);
    }

    #[test]
    fn an_uncommitted_chunk_can_be_retried_after_reopening() {
        let (dir, s) = tmp_store();
        let id = part(&s);
        let checkpoint = s.audiobook_cursor(&id).unwrap();
        // Ein synthetisches Modellresultat vor dem Commit geht beim Absturz
        // verloren. Es hat weder Text noch Audiocursor veraendert.
        drop(s);
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        assert_eq!(s.audiobook_cursor(&id).unwrap(), checkpoint);
        assert!(s
            .append_audiobook_block(
                &id,
                checkpoint.0,
                0,
                320_000,
                &TranscriptDelta {
                    new_segments: vec![]
                }
            )
            .unwrap());
        assert_eq!(s.audiobook_cursor(&id).unwrap().1, 320_000);
    }

    #[cfg(windows)]
    #[test]
    fn priority_is_restored_and_other_threads_are_unchanged() {
        use windows::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        let caller = unsafe { GetThreadPriority(GetCurrentThread()) };
        std::thread::spawn(|| {
            let before = unsafe { GetThreadPriority(GetCurrentThread()) };
            {
                let _priority = ThreadPriority::enter();
                assert_eq!(
                    unsafe { GetThreadPriority(GetCurrentThread()) },
                    THREAD_PRIORITY_BELOW_NORMAL.0
                );
            }
            assert_eq!(unsafe { GetThreadPriority(GetCurrentThread()) }, before);
        })
        .join()
        .unwrap();
        assert_eq!(unsafe { GetThreadPriority(GetCurrentThread()) }, caller);
    }
}
