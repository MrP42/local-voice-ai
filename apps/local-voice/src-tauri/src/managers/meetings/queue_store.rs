//! U7 (Issue #64): Ablage der Import-Warteschlange.
//!
//! Eine Zeile je wartender oder laufender Datei in `import_queue`; die
//! Besprechung selbst (Titel, Quelle, Einwilligung) steht wie immer in
//! `meetings`, mit dem Status `queued` solange sie wartet. So ueberlebt die
//! Warteschlange einen Neustart, und es gibt keine zweite Wahrheit ueber
//! Dateipfad und Einwilligung.
//!
//! Jede Aenderung ist EINE Transaktion (IMMEDIATE): Besprechung und
//! Warteschlangenzeile stimmen nach einem Abbruch an jeder Stelle ueberein.
//! Dieses Modul kennt weder Tauri noch Threads; die Planung steht in `queue.rs`.

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use ulid::Ulid;

use super::empty::{fill_empty_tx, EmptyFill};
use super::store::{Meeting, MeetingSource, MeetingStatus, MeetingStore};

/// Migration U7 (nur ADD COLUMN und CREATE, vorhandene Zeilen bleiben
/// unveraendert): `meetings.description` und die Warteschlange.
///
/// `seq` ordnet die Warteschlange: neue Eintraege bekommen MAX+1, "nach vorn"
/// MIN-1 der wartenden (darf negativ werden). `state`: `waiting` | `running`.
pub(super) const QUEUE_MIGRATION: &str = "ALTER TABLE meetings ADD COLUMN description TEXT;
    CREATE TABLE import_queue (
      meeting_id TEXT PRIMARY KEY, seq INTEGER NOT NULL,
      state TEXT NOT NULL DEFAULT 'waiting', enqueued_at INTEGER NOT NULL);
    CREATE INDEX idx_import_queue_seq ON import_queue(seq);";

/// Status der Besprechung, solange ihre Datei in der Warteschlange steht.
pub const STATUS_QUEUED: &str = "queued";
pub const STATE_WAITING: &str = "waiting";
pub const STATE_RUNNING: &str = "running";

/// Eine Zeile der Warteschlange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueRow {
    pub meeting_id: String,
    pub seq: i64,
    /// `waiting` oder `running`.
    pub state: String,
    pub enqueued_at: i64,
}

/// Was der Neustart mit der Warteschlange gemacht hat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueueRecovery {
    /// Liefen beim Absturz noch vor dem ersten Audio (Dekodieren): wieder
    /// wartend, an ihrer alten Stelle.
    pub requeued: Vec<String>,
    /// Hatten schon Audio und Teiltranskript: die Wiederherstellung nach einem
    /// Absturz (P2d/P8a, `recover_orphans`) holt den Rest nach.
    pub handed_to_recovery: Vec<String>,
    /// Zeilen geloeschter oder sonst verwaister Besprechungen.
    pub dropped: usize,
}

/// Wie eine wartende Datei aus der Warteschlange genommen wurde.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelWaiting {
    /// Stand wartend: Zeile weg, Besprechung `cancelled`.
    Cancelled,
    /// Nicht (mehr) wartend (laeuft, fertig oder unbekannt).
    NotWaiting,
}

impl MeetingStore {
    /// Legt die Besprechung der Datei an (Status `queued`) und reiht sie hinten
    /// ein: beides in einer Transaktion. Bei einem Fehler (Platte voll, Datei
    /// gesperrt) entsteht weder das eine noch das andere.
    pub fn queue_enqueue(
        &self,
        title: &str,
        source_path: &str,
        consent_confirmed_at: Option<i64>,
    ) -> Result<Meeting> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = Ulid::new().to_string();
        let now = Utc::now().timestamp();
        tx.execute(
            "INSERT INTO meetings (id, title, status, source, source_path, consent_confirmed_at,
                                   created_at, updated_at)
             VALUES (?1, ?2, ?3, 'import', ?4, ?5, ?6, ?6)",
            params![
                id,
                title,
                STATUS_QUEUED,
                source_path,
                consent_confirmed_at,
                now
            ],
        )?;
        let seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM import_queue",
            [],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO import_queue (meeting_id, seq, state, enqueued_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, seq, STATE_WAITING, now],
        )?;
        tx.commit()?;
        self.get_meeting(&id)?
            .ok_or_else(|| anyhow!("meeting_not_found"))
    }

    /// G1 (#70): wie `queue_enqueue`, aber die Datei fuellt einen vorhandenen
    /// LEEREN Eintrag (Titel, Projekte und Notizen bleiben; der Titel wird nur
    /// ersetzt, solange er noch der vorgeschlagene ist). `target_not_empty`, wenn
    /// das Ziel nicht (mehr) leer ist, `meeting_not_found`, wenn es fehlt; dann
    /// geschieht nichts. Besprechung und Warteschlangenzeile in EINER Transaktion.
    pub fn queue_enqueue_into(
        &self,
        target_id: &str,
        title: &str,
        source_path: &str,
        consent_confirmed_at: Option<i64>,
    ) -> Result<Meeting> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().timestamp();
        let mut fill = EmptyFill::new(MeetingSource::Import, MeetingStatus::Ready);
        fill.consent_confirmed_at = consent_confirmed_at;
        fill.source_path = Some(source_path);
        fill.title = Some(title);
        fill.only_if_default = true;
        fill_empty_tx(&tx, target_id, &fill)?;
        // Der Status ist hier `queued` (nicht in `MeetingStatus`: ein Wert der Warteschlange).
        tx.execute(
            "UPDATE meetings SET status = ?2 WHERE id = ?1",
            params![target_id, STATUS_QUEUED],
        )?;
        let seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM import_queue",
            [],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO import_queue (meeting_id, seq, state, enqueued_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![target_id, seq, STATE_WAITING, now],
        )?;
        tx.commit()?;
        self.get_meeting(target_id)?
            .ok_or_else(|| anyhow!("meeting_not_found"))
    }

    /// Reiht eine vorhandene Besprechung (gestoppter Import ohne Audio) hinten
    /// wieder ein. `false`, wenn sie nicht `cancelled`/`failed` ist oder schon in
    /// der Warteschlange steht.
    pub fn queue_requeue(&self, meeting_id: &str) -> Result<bool> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().timestamp();
        let changed = tx.execute(
            "UPDATE meetings SET status = ?2, updated_at = ?3
             WHERE id = ?1 AND deleted_at IS NULL AND status IN ('cancelled', 'failed')
               AND source = 'import' AND source_path IS NOT NULL
               AND mic_audio_path IS NULL AND system_audio_path IS NULL",
            params![meeting_id, STATUS_QUEUED, now],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        let seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM import_queue",
            [],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO import_queue (meeting_id, seq, state, enqueued_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![meeting_id, seq, STATE_WAITING, now],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Die Warteschlange: laufende zuerst, dann wartende in der Reihenfolge des
    /// Hinzufuegens (bei gleicher Nummer die zuerst eingetragene, `rowid`). Zeilen geloeschter Besprechungen fehlen.
    pub fn queue_rows(&self) -> Result<Vec<QueueRow>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT q.meeting_id, q.seq, q.state, q.enqueued_at
             FROM import_queue q
             JOIN meetings m ON m.id = q.meeting_id AND m.deleted_at IS NULL
             ORDER BY (q.state = 'running') DESC, q.seq ASC, q.rowid ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(QueueRow {
                    meeting_id: row.get(0)?,
                    seq: row.get(1)?,
                    state: row.get(2)?,
                    enqueued_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Die Quelldatei (`source_path`) der Besprechung.
    pub fn queue_source(&self, meeting_id: &str) -> Result<Option<String>> {
        let conn = self.get_connection()?;
        let path: Option<Option<String>> = conn
            .query_row(
                "SELECT source_path FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(path.flatten())
    }

    /// wartend -> laufend (Besprechung `processing`), atomar. `false`, wenn die
    /// Zeile nicht mehr wartet (entfernt, schon gestartet) oder die Besprechung
    /// geloescht wurde; dann ist auch die Zeile weg.
    pub fn queue_mark_running(&self, meeting_id: &str) -> Result<bool> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let claimed = tx.execute(
            "UPDATE import_queue SET state = ?2 WHERE meeting_id = ?1 AND state = ?3",
            params![meeting_id, STATE_RUNNING, STATE_WAITING],
        )?;
        if claimed == 0 {
            return Ok(false);
        }
        let now = Utc::now().timestamp();
        let updated = tx.execute(
            "UPDATE meetings SET status = 'processing', updated_at = ?2
             WHERE id = ?1 AND deleted_at IS NULL",
            params![meeting_id, now],
        )?;
        if updated == 0 {
            tx.execute(
                "DELETE FROM import_queue WHERE meeting_id = ?1",
                params![meeting_id],
            )?;
            tx.commit()?;
            return Ok(false);
        }
        tx.commit()?;
        Ok(true)
    }

    /// laufend -> wartend (der Lauf konnte nicht beginnen, z. B. kein Speicher
    /// fuer die zweite Engine): Besprechung wieder `queued`, Platz bleibt.
    pub fn queue_mark_waiting(&self, meeting_id: &str) -> Result<()> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE import_queue SET state = ?2 WHERE meeting_id = ?1",
            params![meeting_id, STATE_WAITING],
        )?;
        tx.execute(
            "UPDATE meetings SET status = ?2, updated_at = ?3
             WHERE id = ?1 AND deleted_at IS NULL AND status = 'processing'",
            params![meeting_id, STATUS_QUEUED, Utc::now().timestamp()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Der Lauf ist zu Ende (fertig, gestoppt, gescheitert): die Zeile entfaellt.
    pub fn queue_finish(&self, meeting_id: &str) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "DELETE FROM import_queue WHERE meeting_id = ?1",
            params![meeting_id],
        )?;
        Ok(())
    }

    /// Nimmt eine WARTENDE Datei aus der Warteschlange: Zeile weg, Besprechung
    /// `cancelled` (Endzustand wie nach einem Stopp; die Datei selbst bleibt
    /// unberuehrt, "Fortsetzen" reiht sie wieder ein).
    pub fn queue_cancel_waiting(&self, meeting_id: &str) -> Result<CancelWaiting> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let removed = tx.execute(
            "DELETE FROM import_queue WHERE meeting_id = ?1 AND state = ?2",
            params![meeting_id, STATE_WAITING],
        )?;
        if removed == 0 {
            return Ok(CancelWaiting::NotWaiting);
        }
        let now = Utc::now().timestamp();
        tx.execute(
            "UPDATE meetings SET status = 'cancelled', ended_at = ?2, updated_at = ?2
             WHERE id = ?1 AND deleted_at IS NULL AND status = ?3",
            params![meeting_id, now, STATUS_QUEUED],
        )?;
        tx.commit()?;
        Ok(CancelWaiting::Cancelled)
    }

    /// Zieht eine wartende Datei an die erste Stelle der wartenden. `false`,
    /// wenn sie nicht wartet.
    pub fn queue_move_to_front(&self, meeting_id: &str) -> Result<bool> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let front: i64 = tx.query_row(
            "SELECT COALESCE(MIN(seq), 1) - 1 FROM import_queue WHERE state = ?1",
            params![STATE_WAITING],
            |row| row.get(0),
        )?;
        let changed = tx.execute(
            "UPDATE import_queue SET seq = ?2 WHERE meeting_id = ?1 AND state = ?3",
            params![meeting_id, front, STATE_WAITING],
        )?;
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Setzt eine Besprechung, deren Lauf abgestuerzt ist (Panik im Lauf),
    /// auf `failed`, wenn sie noch `queued`/`processing` steht.
    pub fn queue_fail_meeting(&self, meeting_id: &str) -> Result<bool> {
        let conn = self.get_connection()?;
        let changed = conn.execute(
            "UPDATE meetings SET status = 'failed', updated_at = ?2
             WHERE id = ?1 AND deleted_at IS NULL AND status IN ('queued', 'processing')",
            params![meeting_id, Utc::now().timestamp()],
        )?;
        Ok(changed > 0)
    }

    /// Loescht Zeilen geloeschter Besprechungen und wartende Zeilen, deren
    /// Besprechung nicht `queued` ist (halb geschriebener oder von Hand
    /// veraenderter Zustand). Liefert die Zahl.
    pub fn queue_prune(&self) -> Result<usize> {
        let conn = self.get_connection()?;
        let gone = conn.execute(
            "DELETE FROM import_queue
             WHERE meeting_id NOT IN (SELECT id FROM meetings WHERE deleted_at IS NULL)",
            [],
        )?;
        let stale = conn.execute(
            "DELETE FROM import_queue
             WHERE state = 'waiting'
               AND meeting_id IN (SELECT id FROM meetings WHERE status <> 'queued')",
            [],
        )?;
        Ok(gone + stale)
    }

    /// Beim App-Start, VOR `recover_orphans`: was beim Absturz lief.
    ///
    /// - lief ohne Audio (Dekodieren, vor dem ersten Block): wieder wartend;
    ///   `recover_orphans` sieht die Besprechung dann nicht als Waise.
    /// - lief mit Audio (Transkription, Sprecher): Zeile weg, die
    ///   Wiederherstellung holt den Rest aus der WAV nach.
    ///
    /// Wartende bleiben unveraendert wartend, in ihrer Reihenfolge.
    pub fn queue_recover(&self) -> Result<QueueRecovery> {
        let mut report = QueueRecovery {
            dropped: self.queue_prune()?,
            ..QueueRecovery::default()
        };
        let running: Vec<(String, bool)> = {
            let conn = self.get_connection()?;
            let mut stmt = conn.prepare(
                "SELECT q.meeting_id,
                        (m.mic_audio_path IS NOT NULL OR m.system_audio_path IS NOT NULL)
                 FROM import_queue q JOIN meetings m ON m.id = q.meeting_id
                 WHERE q.state = 'running' AND m.deleted_at IS NULL
                 ORDER BY q.seq",
            )?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get::<_, bool>(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        for (id, has_audio) in running {
            if has_audio {
                self.queue_finish(&id)?;
                report.handed_to_recovery.push(id);
            } else {
                self.queue_mark_waiting(&id)?;
                report.requeued.push(id);
            }
        }
        Ok(report)
    }

    /// Die Beschreibung einer Besprechung (leer = keine).
    pub fn description_of(&self, meeting_id: &str) -> Result<Option<String>> {
        Ok(self
            .get_meeting(meeting_id)?
            .and_then(|m| m.description)
            .filter(|d| !d.trim().is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::search::index::tests::tmp_store;
    use crate::managers::meetings::store::{MeetingSource, MeetingStatus};

    fn enqueue(s: &MeetingStore, name: &str) -> String {
        s.queue_enqueue(name, &format!("C:/in/{name}.wav"), Some(1))
            .unwrap()
            .id
    }

    fn order(s: &MeetingStore) -> Vec<String> {
        s.queue_rows()
            .unwrap()
            .into_iter()
            .map(|r| r.meeting_id)
            .collect()
    }

    fn status(s: &MeetingStore, id: &str) -> String {
        s.get_meeting(id).unwrap().unwrap().status
    }

    #[test]
    fn enqueued_files_wait_in_the_order_they_were_added() {
        let (_dir, s) = tmp_store();
        let a = enqueue(&s, "a");
        let b = enqueue(&s, "b");
        let c = enqueue(&s, "c");
        assert_eq!(order(&s), vec![a.clone(), b, c]);
        let meeting = s.get_meeting(&a).unwrap().unwrap();
        assert_eq!(
            meeting.status, STATUS_QUEUED,
            "sofort eine Besprechung, Status wartet"
        );
        assert_eq!(meeting.source, "import");
        assert_eq!(meeting.source_path.as_deref(), Some("C:/in/a.wav"));
        assert_eq!(
            meeting.consent_confirmed_at,
            Some(1),
            "die Einwilligung bleibt an der Besprechung"
        );
    }

    #[test]
    fn a_move_to_front_puts_the_file_before_every_waiting_one_but_not_before_a_running_one() {
        let (_dir, s) = tmp_store();
        let a = enqueue(&s, "a");
        let b = enqueue(&s, "b");
        let c = enqueue(&s, "c");
        assert!(s.queue_mark_running(&a).unwrap());
        assert!(s.queue_move_to_front(&c).unwrap());
        assert_eq!(order(&s), vec![a.clone(), c.clone(), b.clone()]);
        // Ein zweites Mal an die Spitze, dann wieder ein anderer.
        assert!(s.queue_move_to_front(&b).unwrap());
        assert_eq!(order(&s), vec![a.clone(), b, c]);
        assert!(
            !s.queue_move_to_front(&a).unwrap(),
            "eine laufende Datei wird nicht umgereiht"
        );
        assert!(!s.queue_move_to_front("gibt-es-nicht").unwrap());
    }

    #[test]
    fn a_file_added_after_a_move_to_front_goes_to_the_end() {
        let (_dir, s) = tmp_store();
        let a = enqueue(&s, "a");
        let b = enqueue(&s, "b");
        s.queue_move_to_front(&b).unwrap();
        let c = enqueue(&s, "c");
        assert_eq!(order(&s), vec![b, a, c]);
    }

    #[test]
    fn claiming_is_won_by_exactly_one_caller() {
        let (_dir, s) = tmp_store();
        let a = enqueue(&s, "a");
        assert!(s.queue_mark_running(&a).unwrap());
        assert!(
            !s.queue_mark_running(&a).unwrap(),
            "zweiter Zugriff verliert"
        );
        assert_eq!(status(&s, &a), "processing");
        let rows = s.queue_rows().unwrap();
        assert_eq!(rows[0].state, STATE_RUNNING);
    }

    #[test]
    fn cancelling_a_waiting_file_ends_the_meeting_as_cancelled_and_leaves_the_others() {
        let (_dir, s) = tmp_store();
        let a = enqueue(&s, "a");
        let b = enqueue(&s, "b");
        assert_eq!(
            s.queue_cancel_waiting(&a).unwrap(),
            CancelWaiting::Cancelled
        );
        assert_eq!(status(&s, &a), "cancelled");
        assert_eq!(order(&s), vec![b.clone()]);
        assert_eq!(
            s.queue_cancel_waiting(&a).unwrap(),
            CancelWaiting::NotWaiting
        );
        s.queue_mark_running(&b).unwrap();
        assert_eq!(
            s.queue_cancel_waiting(&b).unwrap(),
            CancelWaiting::NotWaiting,
            "eine laufende Datei stoppt man ueber den Auftrag, nicht ueber die Warteschlange"
        );
        assert_eq!(status(&s, &b), "processing");
    }

    #[test]
    fn a_cancelled_file_can_be_queued_again_at_the_end() {
        let (_dir, s) = tmp_store();
        let a = enqueue(&s, "a");
        let b = enqueue(&s, "b");
        s.queue_cancel_waiting(&a).unwrap();
        assert!(s.queue_requeue(&a).unwrap());
        assert_eq!(status(&s, &a), STATUS_QUEUED);
        assert_eq!(order(&s), vec![b.clone(), a.clone()]);
        assert!(!s.queue_requeue(&a).unwrap(), "wartet schon: nichts zu tun");
        // Ein fertiges Transkript oder eine Live-Aufnahme wird nie eingereiht.
        let live = s
            .create_meeting("live", MeetingSource::Live, Some(1))
            .unwrap();
        s.set_status(&live.id, MeetingStatus::Cancelled).unwrap();
        assert!(!s.queue_requeue(&live.id).unwrap());
        let ready = enqueue(&s, "r");
        s.queue_cancel_waiting(&ready).unwrap();
        s.set_audio_paths(&ready, Some("C:/m/import.wav"), None, Some(1000))
            .unwrap();
        assert!(
            !s.queue_requeue(&ready).unwrap(),
            "mit Audio gilt 'Fortsetzen', nicht die Warteschlange"
        );
    }

    #[test]
    fn a_deleted_meeting_drops_out_of_the_queue_and_cannot_be_claimed() {
        let (_dir, s) = tmp_store();
        let a = enqueue(&s, "a");
        let b = enqueue(&s, "b");
        s.soft_delete_meeting(&a).unwrap();
        assert_eq!(order(&s), vec![b.clone()], "die Liste zeigt sie nie");
        assert!(!s.queue_mark_running(&a).unwrap(), "auch nicht zum Start");
        assert!(s.queue_rows().unwrap().iter().all(|r| r.meeting_id != a));
        assert_eq!(
            s.queue_prune().unwrap(),
            0,
            "die Zeile ist beim Claim schon gegangen"
        );
    }

    #[test]
    fn prune_removes_rows_of_deleted_meetings_and_of_meetings_that_no_longer_wait() {
        let (_dir, s) = tmp_store();
        let a = enqueue(&s, "a");
        let b = enqueue(&s, "b");
        let c = enqueue(&s, "c");
        s.soft_delete_meeting(&a).unwrap();
        // Von Hand auf 'ready' gesetzt, die Zeile wartet noch: verwaist.
        s.set_status(&b, MeetingStatus::Ready).unwrap();
        assert_eq!(s.queue_prune().unwrap(), 2);
        assert_eq!(order(&s), vec![c]);
    }

    #[test]
    fn recovery_requeues_what_had_no_audio_yet_and_hands_the_rest_to_orphan_recovery() {
        let (_dir, s) = tmp_store();
        let decoding = enqueue(&s, "decoding");
        let transcribing = enqueue(&s, "transcribing");
        let waiting = enqueue(&s, "waiting");
        let deleted = enqueue(&s, "deleted");
        s.queue_mark_running(&decoding).unwrap();
        s.queue_mark_running(&transcribing).unwrap();
        s.set_audio_paths(&transcribing, Some("C:/m/import.wav"), None, Some(60_000))
            .unwrap();
        s.queue_mark_running(&deleted).unwrap();
        s.soft_delete_meeting(&deleted).unwrap();

        let report = s.queue_recover().unwrap();
        assert_eq!(report.requeued, vec![decoding.clone()]);
        assert_eq!(report.handed_to_recovery, vec![transcribing.clone()]);
        assert!(
            report.dropped >= 1,
            "die Zeile der geloeschten Besprechung entfaellt"
        );

        // Der abgestuerzte Dekodierlauf wartet wieder, an seiner alten Stelle
        // (vor 'waiting'); der mit Audio ist kein Teil der Warteschlange mehr und
        // bleibt `processing`, damit `recover_orphans` ihn aufgreift.
        assert_eq!(order(&s), vec![decoding.clone(), waiting.clone()]);
        assert_eq!(status(&s, &decoding), STATUS_QUEUED);
        assert_eq!(status(&s, &transcribing), "processing");
        assert_eq!(status(&s, &waiting), STATUS_QUEUED);

        // Zweimal ist dasselbe wie einmal.
        let again = s.queue_recover().unwrap();
        assert_eq!(again, QueueRecovery::default());
        assert_eq!(order(&s), vec![decoding, waiting]);
    }

    #[test]
    fn the_queue_survives_reopening_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let (a, b) = {
            let s = MeetingStore::open_at(&path).unwrap();
            let a = enqueue(&s, "a");
            let b = enqueue(&s, "b");
            s.queue_move_to_front(&b).unwrap();
            (a, b)
        };
        let reopened = MeetingStore::open_at(&path).unwrap();
        assert_eq!(
            order(&reopened),
            vec![b, a],
            "Reihenfolge samt Vorziehen bleibt"
        );
    }

    #[test]
    fn a_failed_insert_leaves_neither_meeting_nor_queue_row() {
        let (_dir, s) = tmp_store();
        // Die Warteschlangentabelle fehlt: das Einreihen scheitert NACH dem
        // Anlegen der Besprechung und muss sie mit zurueckrollen.
        s.get_connection()
            .unwrap()
            .execute("DROP TABLE import_queue", [])
            .unwrap();
        assert!(s.queue_enqueue("x", "C:/in/x.wav", Some(1)).is_err());
        let count: i64 = s
            .get_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM meetings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "keine halbe Besprechung ohne Warteschlangenzeile");
    }

    #[test]
    fn concurrent_enqueues_get_unique_gapless_positions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let store = std::sync::Arc::new(MeetingStore::open_at(&path).unwrap());
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for j in 0..5 {
                        store
                            .queue_enqueue(
                                &format!("t{i}-{j}"),
                                &format!("C:/in/{i}-{j}.wav"),
                                Some(1),
                            )
                            .unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let rows = store.queue_rows().unwrap();
        assert_eq!(rows.len(), 40);
        let seqs: Vec<i64> = rows.iter().map(|r| r.seq).collect();
        assert_eq!(
            seqs,
            (1..=40).collect::<Vec<i64>>(),
            "eindeutig und lueckenlos"
        );
    }

    #[test]
    fn the_migration_is_idempotent_and_keeps_existing_meetings() {
        use rusqlite::Connection;
        use rusqlite_migration::{Migrations, M};
        // Altstand vor U7 (Index 0 bis 6: bis A1 das Register, A3 die Fassungen)
        // mit einer Besprechung, dann migrieren. Der Sprung von 0.20.9 (Index 0
        // bis 4) ueber alle drei Schritte steht in `migration_chain.rs`.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            // Die sieben Schritte, die es vor U7 gab (Index 0 bis 6).
            let before_u7: Vec<M> = crate::managers::meetings::store::MIGRATIONS[..7].to_vec();
            Migrations::new(before_u7).to_latest(&mut conn).unwrap();
            conn.execute(
                "INSERT INTO meetings (id, title, status, source, created_at, updated_at)
                 VALUES ('alt', 'Alte Besprechung', 'ready', 'live', 1, 1)",
                [],
            )
            .unwrap();
        }
        let store = MeetingStore::open_at(&path).unwrap();
        let old = store.get_meeting("alt").unwrap().unwrap();
        assert_eq!(old.title, "Alte Besprechung");
        assert_eq!(
            old.description, None,
            "Altdaten bekommen keine Beschreibung"
        );
        assert!(store.queue_rows().unwrap().is_empty());
        // Ein zweites Oeffnen aendert nichts und scheitert nicht.
        drop(store);
        let store = MeetingStore::open_at(&path).unwrap();
        assert_eq!(
            store.get_meeting("alt").unwrap().unwrap().title,
            "Alte Besprechung"
        );
        let version: i64 = store
            .get_connection()
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            version as usize,
            crate::managers::meetings::store::MIGRATIONS.len()
        );
    }

    // ---- G1 (#70): Datei in einen leeren Eintrag --------------------------------

    #[test]
    fn a_file_enqueued_into_an_empty_entry_fills_that_very_meeting() {
        let (_dir, s) = tmp_store();
        let folder = s.folder_save(None, "Kunde", None).unwrap().id;
        let empty = s.create_empty_meeting("Neue Besprechung", Some(&folder)).unwrap();
        let other = enqueue(&s, "davor");
        let filled = s
            .queue_enqueue_into(&empty.id, "montag", "C:/in/montag.m4a", Some(5))
            .unwrap();
        assert_eq!(filled.id, empty.id, "kein neuer Eintrag");
        assert_eq!(filled.status, STATUS_QUEUED);
        assert_eq!(filled.source, "import");
        assert_eq!(filled.source_path.as_deref(), Some("C:/in/montag.m4a"));
        assert_eq!(filled.consent_confirmed_at, Some(5));
        assert_eq!(filled.title, "montag", "vorgeschlagener Titel wird ersetzt");
        assert_eq!(s.meeting_folder_ids(&empty.id).unwrap(), vec![folder]);
        assert_eq!(order(&s), vec![other, empty.id.clone()], "hinten eingereiht");
        assert_eq!(s.list_meetings(0, 10).unwrap().len(), 2, "keine Dublette");
    }

    #[test]
    fn a_renamed_empty_entry_keeps_its_title_when_a_file_arrives() {
        let (_dir, s) = tmp_store();
        let empty = s.create_empty_meeting("Neue Besprechung", None).unwrap();
        s.set_title(&empty.id, "Mein Kick-off").unwrap();
        let filled = s
            .queue_enqueue_into(&empty.id, "montag", "C:/in/montag.m4a", Some(5))
            .unwrap();
        assert_eq!(filled.title, "Mein Kick-off");
    }

    #[test]
    fn a_file_is_refused_for_a_target_that_is_not_empty_and_nothing_is_queued() {
        let (_dir, s) = tmp_store();
        let live = s.create_meeting("Live", MeetingSource::Live, Some(1)).unwrap();
        let err = s
            .queue_enqueue_into(&live.id, "x", "C:/in/x.wav", Some(1))
            .unwrap_err();
        assert_eq!(err.to_string(), "target_not_empty");
        let empty = s.create_empty_meeting("Neue Besprechung", None).unwrap();
        s.queue_enqueue_into(&empty.id, "x", "C:/in/x.wav", Some(1)).unwrap();
        let again = s
            .queue_enqueue_into(&empty.id, "y", "C:/in/y.wav", Some(1))
            .unwrap_err();
        assert_eq!(again.to_string(), "target_not_empty", "ein zweiter Import");
        let missing = s
            .queue_enqueue_into("gibt-es-nicht", "x", "C:/in/x.wav", Some(1))
            .unwrap_err();
        assert_eq!(missing.to_string(), "meeting_not_found");
        assert_eq!(order(&s), vec![empty.id], "nur die erste Datei wartet");
    }
}
