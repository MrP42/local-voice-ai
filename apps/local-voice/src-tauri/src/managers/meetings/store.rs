use anyhow::{anyhow, Result};
use chrono::Utc;
use log::{debug, info, warn};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use ulid::Ulid;

use super::notes::model::{
    ActionItem, MeetingNotes, NoteBlock, TemplateInfo, TemplateSpec, SOURCE_AI, SOURCE_MANUAL,
    SOURCE_USER, STATUS_DONE, STATUS_TODO,
};
use super::notes::templates::{self, builtin_id, builtin_templates, is_builtin_id};

/// Database migrations for the meetings store. One migration creates every
/// table for M8; later milestones (M9/M10) add migrations rather than
/// editing this one, matching the pattern in `history.rs`.
static MIGRATIONS: &[M] = &[
    M::up(
    "CREATE TABLE meetings (
      id TEXT PRIMARY KEY, title TEXT NOT NULL, status TEXT NOT NULL,
      source TEXT NOT NULL, started_at INTEGER, ended_at INTEGER, language TEXT,
      mic_audio_path TEXT, system_audio_path TEXT, duration_ms INTEGER,
      consent_confirmed_at INTEGER, audio_retention_until INTEGER,
      metadata_json TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
    CREATE TABLE meeting_documents (
      id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL, kind TEXT NOT NULL, template_id TEXT,
      title TEXT, body_format TEXT NOT NULL, body TEXT NOT NULL,
      generation_metadata_json TEXT, version INTEGER NOT NULL DEFAULT 1,
      created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
    CREATE TABLE transcripts (
      id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL UNIQUE, provider TEXT, model TEXT, language TEXT,
      granularity TEXT NOT NULL DEFAULT 'segment@1', segments_json TEXT NOT NULL DEFAULT '[]',
      speaker_hints_json TEXT, content_revision INTEGER NOT NULL DEFAULT 0,
      created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
    CREATE TABLE transcript_deltas (
      transcript_id TEXT NOT NULL, sequence INTEGER NOT NULL, delta_json TEXT NOT NULL,
      created_at INTEGER NOT NULL, PRIMARY KEY (transcript_id, sequence));
    CREATE TABLE speakers (
      id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL, channel INTEGER NOT NULL,
      speaker_index INTEGER, human_id TEXT, display_name TEXT, consent_state TEXT,
      created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
    CREATE TABLE humans (
      id TEXT PRIMARY KEY, name TEXT NOT NULL, email TEXT, memo TEXT,
      created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
    CREATE TABLE action_items (
      id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL, text TEXT NOT NULL,
      assignee_human_id TEXT, due_at INTEGER, status TEXT NOT NULL DEFAULT 'todo',
      source TEXT NOT NULL, kind TEXT NOT NULL,
      created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
    CREATE TABLE meeting_templates (
      id TEXT PRIMARY KEY, title TEXT NOT NULL, sections_json TEXT NOT NULL,
      pinned INTEGER NOT NULL DEFAULT 0,
      created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);",
    ),
    // The title is user-editable from M9 on, so it can no longer double as the
    // record of where an imported meeting came from. `source_path` keeps the
    // original file path (import only; NULL for live recordings), which the
    // detail view shows underneath the title.
    M::up("ALTER TABLE meetings ADD COLUMN source_path TEXT;"),
    // M1 (Notizblock, KI-Notizen, Vorlagen). Nur CREATE und ADD COLUMN mit
    // Defaults: vorhandene Zeilen bleiben unveraendert, die Migration ist
    // (wie jede) eine einzige Transaktion und rollt bei Abbruch vollstaendig
    // zurueck. `meeting_notes`: eine lebende Datei je Besprechung (Bloecke als
    // JSON, `revision` fuer die optimistische Sperre) statt Dokumentversionen.
    // `transcripts.segment_epoch` steigt bei jedem `clear_segments`, damit
    // KI-Notizen erkennen, dass ihre Quellverweise veraltet sind.
    M::up(
        "CREATE TABLE meeting_notes (
      meeting_id TEXT PRIMARY KEY, blocks_json TEXT NOT NULL DEFAULT '[]',
      revision INTEGER NOT NULL DEFAULT 0,
      created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
    ALTER TABLE meetings ADD COLUMN template_id TEXT;
    ALTER TABLE transcripts ADD COLUMN segment_epoch INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE action_items ADD COLUMN document_id TEXT;
    ALTER TABLE action_items ADD COLUMN entry_id TEXT;
    ALTER TABLE action_items ADD COLUMN assignee_label TEXT;
    ALTER TABLE action_items ADD COLUMN sources_json TEXT;
    CREATE INDEX idx_action_items_meeting ON action_items(meeting_id, deleted_at);",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeetingSource {
    Live,
    Import,
    Subtitle,
}

impl MeetingSource {
    fn as_str(&self) -> &'static str {
        match self {
            MeetingSource::Live => "live",
            MeetingSource::Import => "import",
            MeetingSource::Subtitle => "subtitle",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeetingStatus {
    Recording,
    Processing,
    Ready,
    Failed,
}

impl MeetingStatus {
    fn as_str(&self) -> &'static str {
        match self {
            MeetingStatus::Recording => "recording",
            MeetingStatus::Processing => "processing",
            MeetingStatus::Ready => "ready",
            MeetingStatus::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub status: String,
    pub source: String,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub language: Option<String>,
    pub mic_audio_path: Option<String>,
    pub system_audio_path: Option<String>,
    pub duration_ms: Option<u64>,
    pub consent_confirmed_at: Option<i64>,
    pub audio_retention_until: Option<i64>,
    /// Original file path an imported meeting came from. `None` for live
    /// recordings. Kept separately from `title` because the title is
    /// user-editable (M9) and must be allowed to diverge from the file name.
    pub source_path: Option<String>,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
}

impl Meeting {
    /// The meeting's own audio files, mic and system in that order, skipping
    /// whichever side is unset. Shared by every caller that needs to collect
    /// this meeting's WAVs for deletion (retention purge, minutes' inline
    /// purge) so the two-field collection lives in exactly one place.
    pub fn audio_paths(&self) -> Vec<String> {
        [&self.mic_audio_path, &self.system_audio_path]
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct StoredSegment {
    pub segment_index: u32,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub channel: u8, // 0=DirectMic, 1=RemoteParty, 2=MixedCapture
    pub speaker_index: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct TranscriptDelta {
    pub new_segments: Vec<StoredSegment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingDocument {
    pub id: String,
    pub meeting_id: String,
    pub kind: String,
    pub body_format: String,
    pub body: String,
    pub version: u32,
    /// Seconds since the epoch.
    pub created_at: i64,
    /// Template the document was generated from (KI-Notizen); `None` for
    /// minutes and for rows written before M1.
    pub template_id: Option<String>,
    /// Version stamp for the optimistic lock of `update_document_body`.
    /// Milliseconds since the epoch for rows written from M1 on; rows written
    /// earlier still carry seconds — the value is only ever compared for
    /// equality, never interpreted, so both work.
    pub updated_at: i64,
}

// M9/M10 will let users pick a pinned template when generating minutes; the
// `meeting_templates` table and its read path exist already so that later
// milestone doesn't need a migration, but nothing calls `list_templates` yet.
#[allow(dead_code)]
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingTemplate {
    pub id: String,
    pub title: String,
    pub sections_json: String,
    pub pinned: bool,
}

pub struct MeetingStore {
    db_path: PathBuf,
}

impl MeetingStore {
    /// Opens (and, on first run, creates + migrates) `<meetings_dir>/meetings.db`
    /// — normally `<appdata>/meetings`, overridable via `LVA_MEETINGS_DIR`
    /// (harness sandbox, see `super::meetings_data_dir`).
    pub fn new(app: &tauri::AppHandle) -> Result<Self> {
        let meetings_dir = super::meetings_data_dir(app)?;
        if !meetings_dir.exists() {
            std::fs::create_dir_all(&meetings_dir)?;
            debug!("Created meetings directory: {:?}", meetings_dir);
        }
        let db_path = meetings_dir.join("meetings.db");
        Self::open_at(&db_path)
    }

    /// Where this store's SQLite file lives. Reported by the headless
    /// `--import-meeting` / `--dump-meeting` runs so a harness can point at
    /// the very database the app just wrote.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Opens a store at an explicit path, running migrations. Used directly by tests.
    pub fn open_at(path: &Path) -> Result<Self> {
        let store = Self {
            db_path: path.to_path_buf(),
        };
        store.init_database()?;
        store.seed_default_template()?;
        // The bundled templates are a convenience, not the data: a failure here
        // (full disk, locked file) must not lock the user out of every meeting
        // and recording. Log it and carry on; the next open retries.
        if let Err(e) = store.seed_builtin_templates() {
            warn!("Could not seed the bundled meeting templates: {e}");
        }
        Ok(store)
    }

    fn init_database(&self) -> Result<()> {
        info!("Initializing meetings database at {:?}", self.db_path);
        let mut conn = Connection::open(&self.db_path)?;

        let migrations = Migrations::new(MIGRATIONS.to_vec());
        #[cfg(debug_assertions)]
        migrations.validate().expect("Invalid migrations");

        // `to_latest` reads the schema version before it opens its transaction.
        // If a second process (e.g. a headless import next to the running app)
        // migrated in between, our attempt fails on an already-applied step and
        // rolls back untouched. One retry re-reads the version and finds nothing
        // left to do; a real failure fails the same way again and is returned.
        if let Err(first) = migrations.to_latest(&mut conn) {
            warn!("Meetings migration failed, retrying once: {first}");
            migrations.to_latest(&mut conn)?;
        }
        Ok(())
    }

    /// Brings the bundled templates (`builtin:<key>`) to the state of this app
    /// version. Idempotent: rows that are already current are not touched (no
    /// `updated_at` churn), a stale, edited or soft-deleted row is restored. One
    /// transaction, so a crash leaves either all or none of the new state. The
    /// legacy "Standardprotokoll" row is not part of this and stays as it is.
    fn seed_builtin_templates(&self) -> Result<()> {
        let mut conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (key, title, spec) in builtin_templates() {
            let sections_json = serde_json::to_string(&spec)?;
            tx.execute(
                "INSERT INTO meeting_templates (id, title, sections_json, pinned, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 0, ?4, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                   title = excluded.title, sections_json = excluded.sections_json,
                   updated_at = excluded.updated_at, deleted_at = NULL
                 WHERE title IS NOT excluded.title
                    OR sections_json IS NOT excluded.sections_json
                    OR deleted_at IS NOT NULL",
                params![builtin_id(key), title, sections_json, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn seed_default_template(&self) -> Result<()> {
        let conn = self.get_connection()?;
        // Bundled templates live in the same table; they must not count as
        // "the table is already seeded", or a fresh database would never get
        // its "Standardprotokoll".
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM meeting_templates WHERE id NOT LIKE 'builtin:%'",
            [],
            |row| row.get(0),
        )?;
        if count > 0 {
            return Ok(());
        }

        let sections = serde_json::json!([
            "summary",
            "scope",
            "speakers",
            "speaking_shares",
            "decisions",
            "tasks",
            "next_steps",
            "follow_ups",
            "open_questions"
        ])
        .to_string();
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO meeting_templates (id, title, sections_json, pinned, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?4)",
            params![Ulid::new().to_string(), "Standardprotokoll", sections, now],
        )?;
        Ok(())
    }

    fn get_connection(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    /// Guards write paths that key off a `meeting_id` but don't otherwise
    /// touch the `meetings` row (`append_delta`, `upsert_document`): without
    /// this, a typo'd id or a write arriving after `soft_delete_meeting`
    /// would silently create orphaned/invisible rows instead of failing.
    fn ensure_meeting_is_live(conn: &Connection, meeting_id: &str) -> Result<()> {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1 AND deleted_at IS NULL)",
            params![meeting_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(anyhow!("Meeting {} not found", meeting_id));
        }
        Ok(())
    }

    fn map_meeting(row: &rusqlite::Row<'_>) -> rusqlite::Result<Meeting> {
        Ok(Meeting {
            id: row.get("id")?,
            title: row.get("title")?,
            status: row.get("status")?,
            source: row.get("source")?,
            started_at: row.get("started_at")?,
            ended_at: row.get("ended_at")?,
            language: row.get("language")?,
            mic_audio_path: row.get("mic_audio_path")?,
            system_audio_path: row.get("system_audio_path")?,
            duration_ms: row.get::<_, Option<i64>>("duration_ms")?.map(|v| v as u64),
            consent_confirmed_at: row.get("consent_confirmed_at")?,
            audio_retention_until: row.get("audio_retention_until")?,
            source_path: row.get("source_path")?,
            created_at: row.get("created_at")?,
            deleted_at: row.get("deleted_at")?,
        })
    }

    pub fn create_meeting(
        &self,
        title: &str,
        source: MeetingSource,
        consent_confirmed_at: Option<i64>,
    ) -> Result<Meeting> {
        let status = match source {
            MeetingSource::Live => MeetingStatus::Recording,
            MeetingSource::Import | MeetingSource::Subtitle => MeetingStatus::Processing,
        };

        let id = Ulid::new().to_string();
        let now = Utc::now().timestamp();
        let conn = self.get_connection()?;
        conn.execute(
            "INSERT INTO meetings (id, title, status, source, consent_confirmed_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![id, title, status.as_str(), source.as_str(), consent_confirmed_at, now],
        )?;

        Ok(Meeting {
            id,
            title: title.to_string(),
            status: status.as_str().to_string(),
            source: source.as_str().to_string(),
            started_at: None,
            ended_at: None,
            language: None,
            mic_audio_path: None,
            system_audio_path: None,
            duration_ms: None,
            consent_confirmed_at,
            audio_retention_until: None,
            source_path: None,
            created_at: now,
            deleted_at: None,
        })
    }

    /// Renames a meeting. The title is free text chosen by the user and is
    /// deliberately independent of `source_path` — see the field's doc.
    pub fn set_title(&self, id: &str, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() {
            return Err(anyhow!("Meeting title must not be empty"));
        }
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let updated = conn.execute(
            "UPDATE meetings SET title = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            params![title, now, id],
        )?;
        if updated == 0 {
            return Err(anyhow!("Meeting {} not found", id));
        }
        Ok(())
    }

    /// Records where an imported meeting came from.
    pub fn set_source_path(&self, id: &str, source_path: &str) -> Result<()> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE meetings SET source_path = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            params![source_path, now, id],
        )?;
        Ok(())
    }

    /// Drops every stored segment of a meeting, leaving the transcript row in
    /// place with an empty segment list. Used before a re-transcription: the
    /// new run starts its `segment_index` at 0, so old segments must not
    /// survive alongside it. The raw `transcript_deltas` are cleared too —
    /// they are the replay log of exactly the segments being discarded, and
    /// keeping them would make a later replay resurrect the old text. The
    /// transcript's `segment_epoch` goes up by one in the same transaction:
    /// KI-Notizen remember the epoch they were built from, and every source
    /// reference in them points at segment indexes that no longer exist.
    pub fn clear_segments(&self, meeting_id: &str) -> Result<()> {
        let mut conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let tx = conn.transaction()?;
        Self::ensure_meeting_is_live(&tx, meeting_id)?;

        let transcript_id: Option<String> = tx
            .query_row(
                "SELECT id FROM transcripts WHERE meeting_id = ?1",
                params![meeting_id],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(transcript_id) = transcript_id {
            tx.execute(
                "DELETE FROM transcript_deltas WHERE transcript_id = ?1",
                params![transcript_id],
            )?;
            tx.execute(
                "UPDATE transcripts SET segments_json = '[]', content_revision = content_revision + 1,
                     segment_epoch = segment_epoch + 1, updated_at = ?1
                 WHERE id = ?2",
                params![now, transcript_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_status(&self, id: &str, status: MeetingStatus) -> Result<()> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let updated = conn.execute(
            "UPDATE meetings SET status = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            params![status.as_str(), now, id],
        )?;
        if updated == 0 {
            return Err(anyhow!("Meeting {} not found", id));
        }
        Ok(())
    }

    /// Records when the meeting actually ended (recording stop / import
    /// completion). Used as the anchor for `Days(n)` retention — without
    /// this, `ended_at` stayed `NULL` forever and retention math fell back
    /// to "now" wherever it was computed, which drifts every time it's
    /// recomputed (e.g. minutes generated well after the meeting ended).
    pub fn set_ended_at(&self, id: &str, ended_at: i64) -> Result<()> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let updated = conn.execute(
            "UPDATE meetings SET ended_at = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            params![ended_at, now, id],
        )?;
        if updated == 0 {
            return Err(anyhow!("Meeting {} not found", id));
        }
        Ok(())
    }

    pub fn set_audio_paths(
        &self,
        id: &str,
        mic: Option<&str>,
        system: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<()> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let updated = conn.execute(
            "UPDATE meetings SET mic_audio_path = ?1, system_audio_path = ?2, duration_ms = ?3, updated_at = ?4
             WHERE id = ?5 AND deleted_at IS NULL",
            params![mic, system, duration_ms.map(|v| v as i64), now, id],
        )?;
        if updated == 0 {
            return Err(anyhow!("Meeting {} not found", id));
        }
        Ok(())
    }

    /// Sets (or clears) the audio expiry timestamp computed by
    /// `retention::retention_until`. Task 12.
    pub fn set_retention_until(&self, id: &str, until: Option<i64>) -> Result<()> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let updated = conn.execute(
            "UPDATE meetings SET audio_retention_until = ?1, updated_at = ?2
             WHERE id = ?3 AND deleted_at IS NULL",
            params![until, now, id],
        )?;
        if updated == 0 {
            return Err(anyhow!("Meeting {} not found", id));
        }
        Ok(())
    }

    /// Meetings whose audio is due for hard-deletion: not soft-deleted (that
    /// cascade already hard-deletes on its own path) and past their
    /// `audio_retention_until`. Task 12 (`retention::purge_due_audio`).
    pub fn meetings_with_due_audio(&self, now_unix: i64) -> Result<Vec<Meeting>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, status, source, started_at, ended_at, language,
                    mic_audio_path, system_audio_path, duration_ms, consent_confirmed_at,
                    audio_retention_until, source_path, created_at, deleted_at
             FROM meetings
             WHERE deleted_at IS NULL
               AND audio_retention_until IS NOT NULL
               AND audio_retention_until <= ?1",
        )?;
        let meetings = stmt
            .query_map(params![now_unix], Self::map_meeting)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(meetings)
    }

    pub fn get_meeting(&self, id: &str) -> Result<Option<Meeting>> {
        let conn = self.get_connection()?;
        let meeting = conn
            .query_row(
                "SELECT id, title, status, source, started_at, ended_at, language,
                        mic_audio_path, system_audio_path, duration_ms, consent_confirmed_at,
                        audio_retention_until, source_path, created_at, deleted_at
                 FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                Self::map_meeting,
            )
            .optional()?;
        Ok(meeting)
    }

    pub fn list_meetings(&self, offset: u32, limit: u32) -> Result<Vec<Meeting>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, status, source, started_at, ended_at, language,
                    mic_audio_path, system_audio_path, duration_ms, consent_confirmed_at,
                    audio_retention_until, source_path, created_at, deleted_at
             FROM meetings WHERE deleted_at IS NULL
             ORDER BY created_at DESC
             LIMIT ?1 OFFSET ?2",
        )?;
        let meetings = stmt
            .query_map(params![limit, offset], Self::map_meeting)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(meetings)
    }

    /// Soft-deletes a meeting and all of its child rows. Returns the audio
    /// file paths that existed on the meeting so the caller can delete them
    /// from disk (this store never touches the filesystem itself).
    pub fn soft_delete_meeting(&self, id: &str) -> Result<Vec<String>> {
        let mut conn = self.get_connection()?;
        let now = Utc::now().timestamp();

        let tx = conn.transaction()?;

        let paths: Option<(Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT mic_audio_path, system_audio_path FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let Some((mic, system)) = paths else {
            return Err(anyhow!("Meeting {} not found", id));
        };

        tx.execute(
            "UPDATE meetings SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        tx.execute(
            "UPDATE meeting_documents SET deleted_at = ?1, updated_at = ?1 WHERE meeting_id = ?2",
            params![now, id],
        )?;
        tx.execute(
            "UPDATE transcripts SET deleted_at = ?1, updated_at = ?1 WHERE meeting_id = ?2",
            params![now, id],
        )?;
        tx.execute(
            "UPDATE speakers SET deleted_at = ?1, updated_at = ?1 WHERE meeting_id = ?2",
            params![now, id],
        )?;
        tx.execute(
            "UPDATE action_items SET deleted_at = ?1, updated_at = ?1 WHERE meeting_id = ?2",
            params![now, id],
        )?;
        tx.execute(
            "UPDATE meeting_notes SET deleted_at = ?1, updated_at = ?1 WHERE meeting_id = ?2",
            params![now, id],
        )?;

        tx.commit()?;

        let mut audio_paths = Vec::new();
        if let Some(mic) = mic {
            audio_paths.push(mic);
        }
        if let Some(system) = system {
            audio_paths.push(system);
        }
        Ok(audio_paths)
    }

    /// Appends a transcript delta in a single transaction: computes the next
    /// sequence number, persists the raw delta, and materializes the new
    /// segments into `transcripts.segments_json` so `get_segments` stays a
    /// simple read and later crash-replay can diff deltas against it.
    pub fn append_delta(&self, meeting_id: &str, delta: &TranscriptDelta) -> Result<u64> {
        let mut conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let tx = conn.transaction()?;

        Self::ensure_meeting_is_live(&tx, meeting_id)?;

        // Lazily create the transcript row on first delta.
        let transcript_id: Option<String> = tx
            .query_row(
                "SELECT id FROM transcripts WHERE meeting_id = ?1",
                params![meeting_id],
                |row| row.get(0),
            )
            .optional()?;

        let transcript_id = match transcript_id {
            Some(id) => id,
            None => {
                let id = Ulid::new().to_string();
                tx.execute(
                    "INSERT INTO transcripts (id, meeting_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?3)",
                    params![id, meeting_id, now],
                )?;
                id
            }
        };

        let next_sequence: i64 = tx.query_row(
            "SELECT 1 + COALESCE(MAX(sequence), 0) FROM transcript_deltas WHERE transcript_id = ?1",
            params![transcript_id],
            |row| row.get(0),
        )?;

        let delta_json = serde_json::to_string(delta)?;
        tx.execute(
            "INSERT INTO transcript_deltas (transcript_id, sequence, delta_json, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![transcript_id, next_sequence, delta_json, now],
        )?;

        let existing_json: String = tx.query_row(
            "SELECT segments_json FROM transcripts WHERE id = ?1",
            params![transcript_id],
            |row| row.get(0),
        )?;
        let mut segments: Vec<StoredSegment> = serde_json::from_str(&existing_json)?;
        segments.extend(delta.new_segments.iter().cloned());

        let segments_json = serde_json::to_string(&segments)?;
        tx.execute(
            "UPDATE transcripts SET segments_json = ?1, content_revision = content_revision + 1, updated_at = ?2
             WHERE id = ?3",
            params![segments_json, now, transcript_id],
        )?;

        tx.commit()?;
        Ok(next_sequence as u64)
    }

    pub fn get_segments(&self, meeting_id: &str) -> Result<Vec<StoredSegment>> {
        let conn = self.get_connection()?;
        let segments_json: Option<String> = conn
            .query_row(
                "SELECT segments_json FROM transcripts WHERE meeting_id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| row.get(0),
            )
            .optional()?;

        match segments_json {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }

    pub fn update_segment_text(
        &self,
        meeting_id: &str,
        segment_index: u32,
        text: &str,
    ) -> Result<()> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();

        let (transcript_id, segments_json): (String, String) = conn
            .query_row(
                "SELECT id, segments_json FROM transcripts WHERE meeting_id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| anyhow!("No transcript for meeting {}", meeting_id))?;

        let mut segments: Vec<StoredSegment> = serde_json::from_str(&segments_json)?;
        let segment = segments
            .iter_mut()
            .find(|s| s.segment_index == segment_index)
            .ok_or_else(|| {
                anyhow!(
                    "Segment {} not found for meeting {}",
                    segment_index,
                    meeting_id
                )
            })?;
        segment.text = text.to_string();

        let updated_json = serde_json::to_string(&segments)?;
        conn.execute(
            "UPDATE transcripts SET segments_json = ?1, content_revision = content_revision + 1, updated_at = ?2
             WHERE id = ?3",
            params![updated_json, now, transcript_id],
        )?;
        Ok(())
    }

    pub fn upsert_document(
        &self,
        meeting_id: &str,
        kind: &str,
        body_format: &str,
        body: &str,
        generation_metadata: Option<&str>,
    ) -> Result<String> {
        self.insert_document(
            meeting_id,
            kind,
            body_format,
            body,
            None,
            generation_metadata,
        )
    }

    /// Stores a new version of a document (`kind` + `meeting_id` count up from
    /// 1). Version allocation and insert run in one write transaction, so two
    /// concurrent runs cannot end up with the same version number.
    pub fn insert_document(
        &self,
        meeting_id: &str,
        kind: &str,
        body_format: &str,
        body: &str,
        template_id: Option<&str>,
        generation_metadata: Option<&str>,
    ) -> Result<String> {
        let mut conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let now_ms = Utc::now().timestamp_millis();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        Self::ensure_meeting_is_live(&tx, meeting_id)?;

        let current_max_version: Option<i64> = tx
            .query_row(
                "SELECT MAX(version) FROM meeting_documents WHERE meeting_id = ?1 AND kind = ?2 AND deleted_at IS NULL",
                params![meeting_id, kind],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let version = current_max_version.unwrap_or(0) + 1;
        let id = Ulid::new().to_string();
        tx.execute(
            "INSERT INTO meeting_documents (id, meeting_id, kind, template_id, body_format, body, generation_metadata_json, version, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                meeting_id,
                kind,
                template_id,
                body_format,
                body,
                generation_metadata,
                version,
                now,
                now_ms
            ],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn get_documents(&self, meeting_id: &str) -> Result<Vec<MeetingDocument>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, kind, body_format, body, version, created_at, template_id, updated_at
             FROM meeting_documents WHERE meeting_id = ?1 AND deleted_at IS NULL
             ORDER BY version ASC",
        )?;
        let docs = stmt
            .query_map(params![meeting_id], Self::map_document)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(docs)
    }

    fn map_document(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingDocument> {
        Ok(MeetingDocument {
            id: row.get("id")?,
            meeting_id: row.get("meeting_id")?,
            kind: row.get("kind")?,
            body_format: row.get("body_format")?,
            body: row.get("body")?,
            version: row.get::<_, i64>("version")? as u32,
            created_at: row.get("created_at")?,
            template_id: row.get("template_id")?,
            updated_at: row.get("updated_at")?,
        })
    }

    /// The protocol templates (`sections_json` is a plain array of section
    /// keys, e.g. "Standardprotokoll"). Rows holding a `TemplateSpec` object —
    /// the bundled and user templates of M1 — belong to `list_template_infos`.
    /// Not called yet — see the `#[allow(dead_code)]` note on `MeetingTemplate`.
    #[allow(dead_code)]
    pub fn list_templates(&self) -> Result<Vec<MeetingTemplate>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, sections_json, pinned FROM meeting_templates
             WHERE deleted_at IS NULL AND sections_json LIKE '[%'
             ORDER BY created_at ASC",
        )?;
        let templates = stmt
            .query_map([], |row| {
                Ok(MeetingTemplate {
                    id: row.get("id")?,
                    title: row.get("title")?,
                    sections_json: row.get("sections_json")?,
                    pinned: row.get("pinned")?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(templates)
    }
}

/// Storage for M1 (Notizblock, KI-Notizen, Vorlagen, Aufgaben). Every write is
/// one transaction opened with `BEGIN IMMEDIATE`: the read that decides
/// (revision, staleness, version number) and the write that follows cannot be
/// interleaved with another connection's write, and any error on the way
/// (including a full disk) rolls the whole step back.
///
/// Errors that callers act on are plain codes in the error message
/// (`revision_conflict`, `stale_document`, `template_readonly`, ...); they are
/// matched by the command layer, not shown to the user.
// Consumers arrive with P1b/P1c; remove the allow when they do.
#[allow(dead_code)]
impl MeetingStore {
    fn write_tx(conn: &mut Connection) -> Result<rusqlite::Transaction<'_>> {
        Ok(conn.transaction_with_behavior(TransactionBehavior::Immediate)?)
    }

    // ---- Notizblock ------------------------------------------------------

    /// The user's notes for a meeting; an empty notepad (revision 0) when
    /// nothing was saved yet or the meeting was deleted.
    pub fn get_notes(&self, meeting_id: &str) -> Result<MeetingNotes> {
        let conn = self.get_connection()?;
        let row: Option<(String, i64, i64)> = conn
            .query_row(
                "SELECT blocks_json, revision, updated_at FROM meeting_notes
                 WHERE meeting_id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        match row {
            None => Ok(MeetingNotes {
                meeting_id: meeting_id.to_string(),
                blocks: Vec::new(),
                revision: 0,
                updated_at: 0,
            }),
            Some((json, revision, updated_at)) => Ok(MeetingNotes {
                meeting_id: meeting_id.to_string(),
                // The error carries no text: notes are user content and stay out of logs.
                blocks: serde_json::from_str(&json).map_err(|_| anyhow!("notes_corrupt"))?,
                revision: revision as u64,
                updated_at,
            }),
        }
    }

    /// Saves the whole notepad if `base_revision` is still the stored revision
    /// (0 = nothing stored yet) and returns the new revision. On a mismatch
    /// nothing is written and the error is `revision_conflict`; the caller
    /// reloads and keeps its unsaved blocks.
    pub fn save_notes(
        &self,
        meeting_id: &str,
        blocks: &[NoteBlock],
        base_revision: u64,
    ) -> Result<u64> {
        let blocks_json = serde_json::to_string(blocks)?;
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        Self::ensure_meeting_is_live(&tx, meeting_id)?;

        let stored: Option<i64> = tx
            .query_row(
                "SELECT revision FROM meeting_notes WHERE meeting_id = ?1",
                params![meeting_id],
                |row| row.get(0),
            )
            .optional()?;
        let new_revision = match stored {
            None => {
                if base_revision != 0 {
                    return Err(anyhow!("revision_conflict"));
                }
                tx.execute(
                    "INSERT INTO meeting_notes (meeting_id, blocks_json, revision, created_at, updated_at)
                     VALUES (?1, ?2, 1, ?3, ?3)",
                    params![meeting_id, blocks_json, now],
                )?;
                1
            }
            Some(revision) => {
                if revision as u64 != base_revision {
                    return Err(anyhow!("revision_conflict"));
                }
                // The WHERE on `revision` is the compare-and-swap; the
                // IMMEDIATE transaction already excludes other writers.
                let changed = tx.execute(
                    "UPDATE meeting_notes SET blocks_json = ?1, revision = revision + 1, updated_at = ?2
                     WHERE meeting_id = ?3 AND revision = ?4",
                    params![blocks_json, now, meeting_id, revision],
                )?;
                if changed != 1 {
                    return Err(anyhow!("revision_conflict"));
                }
                revision + 1
            }
        };
        tx.commit()?;
        Ok(new_revision as u64)
    }

    // ---- Vorlage der Besprechung, Epoche ---------------------------------

    /// Remembers which template a meeting uses (chosen before, during or after
    /// the recording). `None` clears the choice (= standard template). The id
    /// is not checked against the template table: a deleted template falls
    /// back to the standard template where it is used.
    pub fn set_meeting_template(&self, meeting_id: &str, template_id: Option<&str>) -> Result<()> {
        let template_id = template_id.map(str::trim).filter(|t| !t.is_empty());
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let updated = conn.execute(
            "UPDATE meetings SET template_id = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            params![template_id, now, meeting_id],
        )?;
        if updated == 0 {
            return Err(anyhow!("Meeting {} not found", meeting_id));
        }
        Ok(())
    }

    /// The template chosen for a meeting, if any. (`Meeting` itself does not
    /// carry it: adding a field there would break every struct literal.)
    pub fn meeting_template_id(&self, meeting_id: &str) -> Result<Option<String>> {
        let conn = self.get_connection()?;
        let template_id: Option<Option<String>> = conn
            .query_row(
                "SELECT template_id FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(template_id.flatten())
    }

    /// Generation of the transcript's segments; goes up with every
    /// `clear_segments` (re-transcription). 0 for a meeting without transcript.
    pub fn segment_epoch(&self, meeting_id: &str) -> Result<u32> {
        let conn = self.get_connection()?;
        let epoch: Option<i64> = conn
            .query_row(
                "SELECT segment_epoch FROM transcripts WHERE meeting_id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(epoch.unwrap_or(0) as u32)
    }

    // ---- Dokumente -------------------------------------------------------

    pub fn get_document(&self, document_id: &str) -> Result<Option<MeetingDocument>> {
        let conn = self.get_connection()?;
        let doc = conn
            .query_row(
                "SELECT id, meeting_id, kind, body_format, body, version, created_at, template_id, updated_at
                 FROM meeting_documents WHERE id = ?1 AND deleted_at IS NULL",
                params![document_id],
                Self::map_document,
            )
            .optional()?;
        Ok(doc)
    }

    /// Replaces the body of a document in place (hand edit of KI-Notizen) if
    /// `expected_updated_at` is still the stored stamp; returns the new stamp.
    /// A mismatch writes nothing and fails with `stale_document`; an unknown or
    /// deleted document with `document_not_found`. The new stamp is always
    /// greater than the old one, even for two saves within one millisecond.
    pub fn update_document_body(
        &self,
        document_id: &str,
        body: &str,
        expected_updated_at: i64,
    ) -> Result<i64> {
        let now_ms = Utc::now().timestamp_millis();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let changed = tx.execute(
            "UPDATE meeting_documents SET body = ?1, updated_at = MAX(?2, updated_at + 1)
             WHERE id = ?3 AND updated_at = ?4 AND deleted_at IS NULL",
            params![body, now_ms, document_id, expected_updated_at],
        )?;
        if changed == 0 {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM meeting_documents WHERE id = ?1 AND deleted_at IS NULL)",
                params![document_id],
                |row| row.get(0),
            )?;
            return Err(anyhow!(if exists {
                "stale_document"
            } else {
                "document_not_found"
            }));
        }
        let new_stamp: i64 = tx.query_row(
            "SELECT updated_at FROM meeting_documents WHERE id = ?1",
            params![document_id],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(new_stamp)
    }

    // ---- Vorlagen --------------------------------------------------------

    /// Parses one `meeting_templates` row into a `TemplateInfo`. `None` for
    /// rows that are not a valid `TemplateSpec` (the legacy protocol template,
    /// a spec from a newer format version, damaged JSON): they are not offered.
    fn template_info_from(
        id: String,
        title: String,
        sections_json: &str,
        updated_at: i64,
    ) -> Option<TemplateInfo> {
        let spec: TemplateSpec = serde_json::from_str(sections_json).ok()?;
        templates::validate_spec(&spec).ok()?;
        Some(TemplateInfo {
            builtin: is_builtin_id(&id),
            id,
            title,
            spec,
            updated_at,
        })
    }

    /// All selectable templates: the bundled ones first in catalog order, then
    /// the user's own by creation time. Rows that are not a valid `TemplateSpec`
    /// (notably the legacy "Standardprotokoll") are left out.
    pub fn list_template_infos(&self) -> Result<Vec<TemplateInfo>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, sections_json, updated_at, created_at FROM meeting_templates
             WHERE deleted_at IS NULL",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let catalog: Vec<String> = builtin_templates()
            .iter()
            .map(|(key, _, _)| builtin_id(key))
            .collect();
        // (bundled? 0 : 1, catalog position, created_at, id) -> stable order.
        let mut keyed: Vec<((u8, usize, i64, String), TemplateInfo)> = rows
            .into_iter()
            .filter_map(|(id, title, json, updated_at, created_at)| {
                let info = Self::template_info_from(id.clone(), title, &json, updated_at)?;
                let key = if info.builtin {
                    let pos = catalog.iter().position(|c| *c == id).unwrap_or(usize::MAX);
                    (0, pos, 0, id)
                } else {
                    (1, 0, created_at, id)
                };
                Some((key, info))
            })
            .collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(keyed.into_iter().map(|(_, info)| info).collect())
    }

    /// One selectable template by id (`None` if unknown, deleted or not a spec).
    pub fn get_template_info(&self, id: &str) -> Result<Option<TemplateInfo>> {
        let conn = self.get_connection()?;
        let row: Option<(String, String, i64)> = conn
            .query_row(
                "SELECT title, sections_json, updated_at FROM meeting_templates
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(title, json, updated_at)| {
            Self::template_info_from(id.to_string(), title, &json, updated_at)
        }))
    }

    /// Creates (`id = None`) or updates a user template. Bundled templates are
    /// read-only (`template_readonly`; the UI duplicates instead). Updating an
    /// unknown, deleted or legacy-protocol row fails with `template_not_found`;
    /// an invalid title or spec with `template_invalid:<reason>`.
    pub fn save_template(
        &self,
        id: Option<&str>,
        title: &str,
        spec: &TemplateSpec,
    ) -> Result<TemplateInfo> {
        if id.is_some_and(is_builtin_id) {
            return Err(anyhow!("template_readonly"));
        }
        templates::validate_title(title).map_err(|e| anyhow!(e))?;
        templates::validate_spec(spec).map_err(|e| anyhow!(e))?;
        let title = title.trim();
        let sections_json = serde_json::to_string(spec)?;
        let now = Utc::now().timestamp();

        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let template_id = match id {
            None => {
                let new_id = Ulid::new().to_string();
                tx.execute(
                    "INSERT INTO meeting_templates (id, title, sections_json, pinned, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 0, ?4, ?4)",
                    params![new_id, title, sections_json, now],
                )?;
                new_id
            }
            Some(existing_id) => {
                let stored: Option<String> = tx
                    .query_row(
                        "SELECT sections_json FROM meeting_templates WHERE id = ?1 AND deleted_at IS NULL",
                        params![existing_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                match stored {
                    Some(json) if serde_json::from_str::<TemplateSpec>(&json).is_ok() => {}
                    _ => return Err(anyhow!("template_not_found")),
                }
                tx.execute(
                    "UPDATE meeting_templates SET title = ?1, sections_json = ?2, updated_at = ?3 WHERE id = ?4",
                    params![title, sections_json, now, existing_id],
                )?;
                existing_id.to_string()
            }
        };
        tx.commit()?;
        Ok(TemplateInfo {
            id: template_id,
            title: title.to_string(),
            builtin: false,
            spec: spec.clone(),
            updated_at: now,
        })
    }

    /// Soft-deletes a user template. Bundled templates cannot be deleted
    /// (`template_readonly`); the legacy protocol template and unknown ids are
    /// `template_not_found`.
    pub fn delete_template(&self, id: &str) -> Result<()> {
        if is_builtin_id(id) {
            return Err(anyhow!("template_readonly"));
        }
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let stored: Option<String> = tx
            .query_row(
                "SELECT sections_json FROM meeting_templates WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        match stored {
            Some(json) if serde_json::from_str::<TemplateSpec>(&json).is_ok() => {}
            _ => return Err(anyhow!("template_not_found")),
        }
        tx.execute(
            "UPDATE meeting_templates SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        tx.commit()?;
        Ok(())
    }

    // ---- Aufgaben --------------------------------------------------------

    fn normalize_task_text(text: &str) -> String {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }

    /// Replaces the tasks derived from KI-Notizen with those of a new document
    /// version: the meeting's earlier `ai`/`user` rows are soft-deleted, `items`
    /// are inserted with fresh ids, `manual` rows stay. A task whose normalized
    /// text (lower case, whitespace collapsed) was `done` in the replaced rows
    /// stays `done`. `meeting_id` and `document_id` of the items are taken from
    /// the arguments; blank texts are skipped. Returns the rows as stored.
    /// All or nothing.
    pub fn replace_action_items(
        &self,
        meeting_id: &str,
        document_id: &str,
        items: &[ActionItem],
    ) -> Result<Vec<ActionItem>> {
        for item in items {
            if item.status != STATUS_TODO && item.status != STATUS_DONE {
                return Err(anyhow!("invalid_status"));
            }
            if ![SOURCE_AI, SOURCE_USER, SOURCE_MANUAL].contains(&item.source.as_str()) {
                return Err(anyhow!("invalid_source"));
            }
        }

        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        Self::ensure_meeting_is_live(&tx, meeting_id)?;
        let document_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM meeting_documents WHERE id = ?1 AND meeting_id = ?2 AND deleted_at IS NULL)",
            params![document_id, meeting_id],
            |row| row.get(0),
        )?;
        if !document_exists {
            return Err(anyhow!("document_not_found"));
        }

        let mut done_before: std::collections::HashSet<String> = std::collections::HashSet::new();
        {
            let mut stmt = tx.prepare(
                "SELECT text FROM action_items
                 WHERE meeting_id = ?1 AND deleted_at IS NULL AND source <> ?2 AND status = ?3",
            )?;
            let texts = stmt.query_map(params![meeting_id, SOURCE_MANUAL, STATUS_DONE], |row| {
                row.get::<_, String>(0)
            })?;
            for text in texts {
                done_before.insert(Self::normalize_task_text(&text?));
            }
        }
        tx.execute(
            "UPDATE action_items SET deleted_at = ?1, updated_at = ?1
             WHERE meeting_id = ?2 AND deleted_at IS NULL AND source <> ?3",
            params![now, meeting_id, SOURCE_MANUAL],
        )?;

        let mut stored = Vec::with_capacity(items.len());
        for item in items {
            if item.text.trim().is_empty() {
                continue;
            }
            let done = item.status == STATUS_DONE
                || done_before.contains(&Self::normalize_task_text(&item.text));
            let row = ActionItem {
                id: Ulid::new().to_string(),
                meeting_id: meeting_id.to_string(),
                text: item.text.clone(),
                status: if done { STATUS_DONE } else { STATUS_TODO }.to_string(),
                assignee_label: item.assignee_label.clone(),
                document_id: Some(document_id.to_string()),
                entry_id: item.entry_id.clone(),
                source_segment_ids: item.source_segment_ids.clone(),
                source: item.source.clone(),
            };
            tx.execute(
                "INSERT INTO action_items
                   (id, meeting_id, text, status, source, kind, document_id, entry_id,
                    assignee_label, sources_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'task', ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    row.id,
                    row.meeting_id,
                    row.text,
                    row.status,
                    row.source,
                    row.document_id,
                    row.entry_id,
                    row.assignee_label,
                    serde_json::to_string(&row.source_segment_ids)?,
                    now
                ],
            )?;
            stored.push(row);
        }
        tx.commit()?;
        Ok(stored)
    }

    /// The meeting's tasks in insertion order: those of the newest KI-Notizen
    /// version plus `manual` ones. Replaced rows are already soft-deleted.
    pub fn list_action_items(&self, meeting_id: &str) -> Result<Vec<ActionItem>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, text, status, assignee_label, document_id, entry_id, sources_json, source
             FROM action_items
             WHERE meeting_id = ?1 AND deleted_at IS NULL
               AND (source = ?2
                    OR document_id = (SELECT id FROM meeting_documents
                                      WHERE meeting_id = ?1 AND kind = 'enhanced_notes' AND deleted_at IS NULL
                                      ORDER BY version DESC LIMIT 1))
             ORDER BY rowid",
        )?;
        let items = stmt
            .query_map(params![meeting_id, SOURCE_MANUAL], |row| {
                let sources_json: Option<String> = row.get(7)?;
                Ok(ActionItem {
                    id: row.get(0)?,
                    meeting_id: row.get(1)?,
                    text: row.get(2)?,
                    status: row.get(3)?,
                    assignee_label: row.get(4)?,
                    document_id: row.get(5)?,
                    entry_id: row.get(6)?,
                    // Only a list of segment numbers; a damaged value costs the
                    // source links of one task, not the whole list.
                    source_segment_ids: sources_json
                        .and_then(|json| serde_json::from_str(&json).ok())
                        .unwrap_or_default(),
                    source: row.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(items)
    }

    /// Ticks or unticks a task. `action_item_not_found` for an unknown or
    /// already replaced/deleted row.
    pub fn set_action_item_status(&self, id: &str, done: bool) -> Result<()> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let status = if done { STATUS_DONE } else { STATUS_TODO };
        let updated = conn.execute(
            "UPDATE action_items SET status = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            params![status, now, id],
        )?;
        if updated == 0 {
            return Err(anyhow!("action_item_not_found"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> MeetingStore {
        // open_at mit Tempdir-Datei — In-Memory geht nicht, weil der Store pro Aufruf öffnet (History-Muster)
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let s = MeetingStore::open_at(&path).unwrap();
        std::mem::forget(dir); // Tempdir bis Prozessende behalten
        s
    }

    #[test]
    fn a_meeting_without_consent_timestamp_is_storable_but_marked() {
        let s = store();
        let m = s
            .create_meeting("Jour fixe", MeetingSource::Import, None)
            .unwrap();
        assert!(m.consent_confirmed_at.is_none());
        assert_eq!(m.status, "processing");
    }

    #[test]
    fn live_meetings_start_in_recording_state_with_consent() {
        let s = store();
        let m = s
            .create_meeting("Standup", MeetingSource::Live, Some(1_755_600_000))
            .unwrap();
        assert_eq!(m.status, "recording");
        assert_eq!(m.consent_confirmed_at, Some(1_755_600_000));
    }

    #[test]
    fn the_title_is_renamable_and_independent_of_the_source_file() {
        let s = store();
        let m = s
            .create_meeting("voicemail-4ADF923F", MeetingSource::Import, None)
            .unwrap();
        s.set_source_path(&m.id, "C:/in/voicemail-4ADF923F.wav")
            .unwrap();
        s.set_title(&m.id, "  Rueckruf Agentur fuer Arbeit  ")
            .unwrap();

        let reloaded = s.get_meeting(&m.id).unwrap().unwrap();
        assert_eq!(
            reloaded.title, "Rueckruf Agentur fuer Arbeit",
            "wird getrimmt"
        );
        assert_eq!(
            reloaded.source_path.as_deref(),
            Some("C:/in/voicemail-4ADF923F.wav"),
            "Umbenennen darf die Herkunft nicht verlieren"
        );
    }

    #[test]
    fn an_empty_title_is_rejected_rather_than_stored() {
        let s = store();
        let m = s
            .create_meeting("Jour fixe", MeetingSource::Live, None)
            .unwrap();
        assert!(s.set_title(&m.id, "   ").is_err());
        assert_eq!(s.get_meeting(&m.id).unwrap().unwrap().title, "Jour fixe");
    }

    #[test]
    fn clear_segments_empties_the_transcript_and_its_replay_log() {
        let s = store();
        let m = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();
        s.append_delta(
            &m.id,
            &TranscriptDelta {
                new_segments: vec![StoredSegment {
                    segment_index: 0,
                    text: "Alt.".into(),
                    start_ms: 0,
                    end_ms: 1_000,
                    channel: 0,
                    speaker_index: None,
                }],
            },
        )
        .unwrap();
        assert_eq!(s.get_segments(&m.id).unwrap().len(), 1);

        s.clear_segments(&m.id).unwrap();
        assert!(s.get_segments(&m.id).unwrap().is_empty());

        // Der naechste Lauf faengt wieder bei Sequenz 1 an — der alte
        // Delta-Log ist mitgeloescht, sonst wuerde ein Replay den alten Text
        // wiederauferstehen lassen.
        let sequence = s
            .append_delta(
                &m.id,
                &TranscriptDelta {
                    new_segments: vec![StoredSegment {
                        segment_index: 0,
                        text: "Neu.".into(),
                        start_ms: 0,
                        end_ms: 1_000,
                        channel: 0,
                        speaker_index: None,
                    }],
                },
            )
            .unwrap();
        assert_eq!(sequence, 1);
        let segments = s.get_segments(&m.id).unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "Neu.");
    }

    #[test]
    fn deltas_are_sequenced_and_segments_materialize_in_order() {
        let s = store();
        let m = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();
        let d1 = TranscriptDelta {
            new_segments: vec![StoredSegment {
                segment_index: 0,
                text: "Hallo.".into(),
                start_ms: 0,
                end_ms: 900,
                channel: 0,
                speaker_index: None,
            }],
        };
        let d2 = TranscriptDelta {
            new_segments: vec![StoredSegment {
                segment_index: 1,
                text: "Guten Morgen.".into(),
                start_ms: 950,
                end_ms: 2100,
                channel: 1,
                speaker_index: None,
            }],
        };
        assert_eq!(s.append_delta(&m.id, &d1).unwrap(), 1);
        assert_eq!(s.append_delta(&m.id, &d2).unwrap(), 2);
        let segs = s.get_segments(&m.id).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[1].text, "Guten Morgen.");
        assert_eq!(segs[1].channel, 1);
    }

    #[test]
    fn segment_text_can_be_corrected() {
        let s = store();
        let m = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();
        s.append_delta(
            &m.id,
            &TranscriptDelta {
                new_segments: vec![StoredSegment {
                    segment_index: 0,
                    text: "Falsch erkannt".into(),
                    start_ms: 0,
                    end_ms: 800,
                    channel: 0,
                    speaker_index: None,
                }],
            },
        )
        .unwrap();
        s.update_segment_text(&m.id, 0, "Richtig erkannt").unwrap();
        assert_eq!(s.get_segments(&m.id).unwrap()[0].text, "Richtig erkannt");
    }

    #[test]
    fn soft_delete_hides_the_meeting_and_returns_audio_paths() {
        let s = store();
        let m = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();
        s.set_audio_paths(
            &m.id,
            Some("C:/x/mic.wav"),
            Some("C:/x/system.wav"),
            Some(60_000),
        )
        .unwrap();
        let paths = s.soft_delete_meeting(&m.id).unwrap();
        assert_eq!(
            paths,
            vec!["C:/x/mic.wav".to_string(), "C:/x/system.wav".to_string()]
        );
        assert!(s.list_meetings(0, 50).unwrap().is_empty());
        assert!(s.get_meeting(&m.id).unwrap().is_none());
    }

    #[test]
    fn the_default_template_is_seeded_once() {
        let s = store();
        let t = s.list_templates().unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].title, "Standardprotokoll");
        // Alle Spec-Sektionen enthalten:
        for key in [
            "summary",
            "scope",
            "decisions",
            "tasks",
            "next_steps",
            "follow_ups",
            "open_questions",
        ] {
            assert!(
                t[0].sections_json.contains(key),
                "Sektion {key} fehlt im Seed"
            );
        }
    }

    #[test]
    fn documents_version_instead_of_overwrite() {
        let s = store();
        let m = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();
        s.upsert_document(&m.id, "minutes", "markdown@1", "# V1", None)
            .unwrap();
        s.upsert_document(&m.id, "minutes", "markdown@1", "# V2", None)
            .unwrap();
        let docs = s.get_documents(&m.id).unwrap();
        assert_eq!(
            docs.len(),
            2,
            "Regenerieren erzeugt neue Version statt Überschreiben (Spec M10-Vorgriff)"
        );
        assert_eq!(docs.iter().map(|d| d.version).max(), Some(2));
    }

    #[test]
    fn append_delta_to_a_deleted_or_unknown_meeting_is_an_error() {
        let s = store();
        let m = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();
        s.soft_delete_meeting(&m.id).unwrap();

        let delta = TranscriptDelta {
            new_segments: vec![StoredSegment {
                segment_index: 0,
                text: "Zu spät.".into(),
                start_ms: 0,
                end_ms: 500,
                channel: 0,
                speaker_index: None,
            }],
        };
        assert!(s.append_delta(&m.id, &delta).is_err());
        assert!(s.append_delta(&Ulid::new().to_string(), &delta).is_err());
    }

    #[test]
    fn upsert_document_requires_a_live_meeting() {
        let s = store();
        let m = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();
        s.soft_delete_meeting(&m.id).unwrap();

        assert!(s
            .upsert_document(&m.id, "minutes", "markdown@1", "# V1", None)
            .is_err());
        assert!(s
            .upsert_document(
                &Ulid::new().to_string(),
                "minutes",
                "markdown@1",
                "# V1",
                None
            )
            .is_err());
    }

    // ---------------------------------------------------------------------
    // M1 / P1a: Migration, Notizblock, Vorlagen, Dokumente, Aufgaben
    // ---------------------------------------------------------------------
    // Alle Tests arbeiten auf temporaeren Datenbanken (tempfile), nie auf der
    // produktiven meetings.db.

    use super::super::notes::model::{NoteBlockKind, SectionKind, TemplateSection};
    use rusqlite::types::Value;
    use std::sync::Barrier;

    fn tmp_store() -> (tempfile::TempDir, MeetingStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        (dir, s)
    }

    fn live_meeting(s: &MeetingStore) -> Meeting {
        s.create_meeting("Besprechung", MeetingSource::Live, Some(1))
            .unwrap()
    }

    fn block(id: &str, text: &str) -> NoteBlock {
        NoteBlock {
            id: id.into(),
            kind: NoteBlockKind::Bullet,
            text: text.into(),
            at_ms: Some(1_000),
            checked: false,
        }
    }

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    fn scalar(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|c| c.unwrap())
            .collect();
        cols
    }

    /// All rows of `cols` of a table as raw SQLite values, in insertion order.
    fn dump(conn: &Connection, table: &str, cols: &[String], filter: &str) -> Vec<Vec<Value>> {
        let sql = format!(
            "SELECT {} FROM {table} {filter} ORDER BY rowid",
            cols.join(", ")
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let n = cols.len();
        let rows = stmt
            .query_map([], |row| {
                (0..n)
                    .map(|i| row.get::<_, Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows
    }

    const LEGACY_TABLES: [&str; 8] = [
        "meetings",
        "meeting_documents",
        "transcripts",
        "transcript_deltas",
        "speakers",
        "humans",
        "action_items",
        "meeting_templates",
    ];

    /// A database exactly as an app from before M1 leaves it (only the first two
    /// migrations applied), with rows in every table, umlauts, emoji, a
    /// soft-deleted meeting and the seeded "Standardprotokoll".
    fn create_legacy_db(path: &Path) {
        let mut conn = Connection::open(path).unwrap();
        Migrations::new(MIGRATIONS[..2].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO meetings (id, title, status, source, started_at, ended_at, language,
                mic_audio_path, system_audio_path, duration_ms, consent_confirmed_at,
                audio_retention_until, metadata_json, created_at, updated_at, deleted_at, source_path)
            VALUES
              ('M1', 'Kundengespräch Größe 🚀', 'ready', 'live', 1755600000, 1755603600, 'de',
               'C:/a/mic.wav', 'C:/a/sys.wav', 3600000, 1755599990, 1758200000,
               '{"k":"v"}', 1755599990, 1755603700, NULL, NULL),
              ('M2', 'Gelöscht', 'ready', 'import', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
               NULL, 1755500000, 1755500001, 1755500002, 'C:/in/x.wav');
            INSERT INTO meeting_documents (id, meeting_id, kind, template_id, title, body_format,
                body, generation_metadata_json, version, created_at, updated_at, deleted_at)
            VALUES
              ('D1', 'M1', 'minutes', NULL, NULL, 'markdown@1', '# Protokoll v1 ü', '{"m":"x"}', 1, 1755603701, 1755603701, NULL),
              ('D2', 'M1', 'minutes', NULL, 'Titel', 'markdown@1', '# Protokoll v2 ß', NULL, 2, 1755603800, 1755603801, NULL),
              ('D3', 'M2', 'minutes', NULL, NULL, 'markdown@1', 'weg', NULL, 1, 1755500001, 1755500002, 1755500002);
            INSERT INTO transcripts (id, meeting_id, provider, model, language, granularity,
                segments_json, speaker_hints_json, content_revision, created_at, updated_at, deleted_at)
            VALUES
              ('T1', 'M1', 'local', 'whisper', 'de', 'segment@1',
               '[{"segment_index":0,"text":"Guten Tag, schön dass Sie da sind.","start_ms":0,"end_ms":2000,"channel":0,"speaker_index":null},{"segment_index":1,"text":"Danke, gern.","start_ms":2100,"end_ms":3000,"channel":1,"speaker_index":2}]',
               '{"h":1}', 3, 1755599995, 1755603600, NULL);
            INSERT INTO transcript_deltas (transcript_id, sequence, delta_json, created_at)
            VALUES ('T1', 1, '{"new_segments":[]}', 1755599996), ('T1', 2, '{"new_segments":[]}', 1755599997);
            INSERT INTO speakers (id, meeting_id, channel, speaker_index, human_id, display_name,
                consent_state, created_at, updated_at, deleted_at)
            VALUES ('S1', 'M1', 1, 2, 'H1', 'Frau Müller', 'confirmed', 1755599990, 1755599991, NULL);
            INSERT INTO humans (id, name, email, memo, created_at, updated_at, deleted_at)
            VALUES ('H1', 'Frau Müller', 'mueller@example.org', 'Kundin', 1755599990, 1755599991, NULL);
            INSERT INTO action_items (id, meeting_id, text, assignee_human_id, due_at, status, source,
                kind, created_at, updated_at, deleted_at)
            VALUES
              ('A1', 'M1', 'Angebot schicken', 'H1', 1756000000, 'todo', 'ai', 'task', 1755603700, 1755603700, NULL),
              ('A2', 'M1', 'Termin klären', NULL, NULL, 'done', 'manual', 'task', 1755603700, 1755603710, NULL),
              ('A3', 'M2', 'Weg', NULL, NULL, 'todo', 'ai', 'task', 1755500001, 1755500002, 1755500002);
            INSERT INTO meeting_templates (id, title, sections_json, pinned, created_at, updated_at, deleted_at)
            VALUES ('LEGACY-STD', 'Standardprotokoll', '["summary","scope","decisions","tasks"]', 1, 1755599000, 1755599000, NULL);
            "#,
        )
        .unwrap();
    }

    fn dump_all(
        conn: &Connection,
        only_legacy_columns_of: &[(String, Vec<String>)],
    ) -> Vec<Vec<Vec<Value>>> {
        only_legacy_columns_of
            .iter()
            .map(|(table, cols)| {
                let filter = if table == "meeting_templates" {
                    "WHERE id NOT LIKE 'builtin:%'"
                } else {
                    ""
                };
                dump(conn, table, cols, filter)
            })
            .collect()
    }

    #[test]
    fn migration_keeps_legacy_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        create_legacy_db(&path);

        // Zustand VOR der Migration: alle Spalten aller Tabellen, roh.
        let (legacy_columns, before) = {
            let conn = Connection::open(&path).unwrap();
            assert_eq!(user_version(&conn), 2, "Altdatenbank steht auf Version 2");
            let cols: Vec<(String, Vec<String>)> = LEGACY_TABLES
                .iter()
                .map(|t| (t.to_string(), table_columns(&conn, t)))
                .collect();
            let before = dump_all(&conn, &cols);
            (cols, before)
        };
        assert!(
            before.iter().all(|rows| !rows.is_empty()),
            "jede Tabelle hat Altzeilen"
        );

        let s = MeetingStore::open_at(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
        assert_eq!(
            dump_all(&conn, &legacy_columns),
            before,
            "Altzeilen muessen nach der Migration wertgleich sein (alle Alt-Spalten)"
        );

        // Neue Spalten tragen den Default, die neue Tabelle ist leer.
        assert_eq!(
            scalar(
                &conn,
                "SELECT COUNT(*) FROM meetings WHERE template_id IS NOT NULL"
            ),
            0
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT COUNT(*) FROM transcripts WHERE segment_epoch <> 0"
            ),
            0
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT COUNT(*) FROM action_items WHERE document_id IS NOT NULL OR entry_id IS NOT NULL
                 OR assignee_label IS NOT NULL OR sources_json IS NOT NULL"
            ),
            0
        );
        assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM meeting_notes"), 0);
        assert_eq!(
            scalar(
                &conn,
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_action_items_meeting'"
            ),
            1
        );

        // Der neue Code liest die Altdaten.
        let m = s.get_meeting("M1").unwrap().unwrap();
        assert_eq!(m.title, "Kundengespräch Größe 🚀");
        assert!(
            s.get_meeting("M2").unwrap().is_none(),
            "geloeschte bleibt geloescht"
        );
        let docs = s.get_documents("M1").unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].template_id, None);
        assert_eq!(
            docs[1].updated_at, 1755603801,
            "Alt-Stempel (Sekunden) unveraendert"
        );
        assert_eq!(s.get_segments("M1").unwrap().len(), 2);
        assert_eq!(s.segment_epoch("M1").unwrap(), 0);
        let notes = s.get_notes("M1").unwrap();
        assert!(notes.blocks.is_empty());
        assert_eq!(notes.revision, 0);
        assert_eq!(s.meeting_template_id("M1").unwrap(), None);
        assert_eq!(
            s.list_templates().unwrap().len(),
            1,
            "nur das Standardprotokoll"
        );
        assert_eq!(s.list_template_infos().unwrap().len(), 8);
        // Alt-Aufgaben ohne Dokumentbezug erscheinen nicht in der neuen Liste,
        // die manuelle schon.
        let items = s.list_action_items("M1").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "A2");
        assert_eq!(items[0].status, "done");

        // Und die neuen Schreibpfade laufen auf den Altdaten.
        assert_eq!(s.save_notes("M1", &[block("b1", "Größe")], 0).unwrap(), 1);
        s.clear_segments("M1").unwrap();
        assert_eq!(s.segment_epoch("M1").unwrap(), 1);
    }

    #[test]
    fn migration_is_all_or_nothing_when_a_step_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        create_legacy_db(&path);
        let cols: Vec<(String, Vec<String>)>;
        let before;
        {
            let conn = Connection::open(&path).unwrap();
            cols = LEGACY_TABLES
                .iter()
                .map(|t| (t.to_string(), table_columns(&conn, t)))
                .collect();
            before = dump_all(&conn, &cols);
        }

        // Ein Schritt, der mittendrin scheitert (wie ein Abbruch durch volle
        // Platte), nachdem er schon Schema geaendert hat.
        let mut broken = MIGRATIONS[..2].to_vec();
        broken.push(M::up(
            "CREATE TABLE half_done (x TEXT);
             ALTER TABLE meetings ADD COLUMN broken TEXT;
             ALTER TABLE no_such_table ADD COLUMN y TEXT;",
        ));
        let mut conn = Connection::open(&path).unwrap();
        assert!(Migrations::new(broken).to_latest(&mut conn).is_err());
        drop(conn);

        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn), 2, "Version unveraendert");
        assert_eq!(
            scalar(
                &conn,
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'half_done'"
            ),
            0,
            "kein halb angelegtes Schema"
        );
        assert!(!table_columns(&conn, "meetings").contains(&"broken".to_string()));
        assert_eq!(dump_all(&conn, &cols), before, "Daten unberuehrt");
        drop(conn);

        // Danach laeuft die echte Migration sauber durch.
        MeetingStore::open_at(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
        assert_eq!(dump_all(&conn, &cols), before);
    }

    #[test]
    fn opening_a_legacy_database_from_several_threads_migrates_it_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        create_legacy_db(&path);

        let barrier = Barrier::new(6);
        let results: Vec<Result<()>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..6)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        MeetingStore::open_at(&path).map(|_| ())
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for r in &results {
            assert!(
                r.is_ok(),
                "gleichzeitiges Oeffnen darf nicht scheitern: {r:?}"
            );
        }
        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
        assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM meeting_templates"), 9);
    }

    #[test]
    fn builtin_templates_seed_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        create_legacy_db(&path);

        MeetingStore::open_at(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let all_cols = table_columns(&conn, "meeting_templates");
        let after_first = dump(&conn, "meeting_templates", &all_cols, "");
        assert_eq!(after_first.len(), 9, "Standardprotokoll + 8 mitgelieferte");
        assert_eq!(
            scalar(
                &conn,
                "SELECT COUNT(*) FROM meeting_templates WHERE id LIKE 'builtin:%'"
            ),
            8
        );

        // Ein Marker im Stempel zeigt, ob ein erneutes Oeffnen aktuelle Zeilen anfasst.
        conn.execute(
            "UPDATE meeting_templates SET updated_at = 42 WHERE id LIKE 'builtin:%'",
            [],
        )
        .unwrap();
        let marked = dump(&conn, "meeting_templates", &all_cols, "");
        drop(conn);

        MeetingStore::open_at(&path).unwrap();
        let s = MeetingStore::open_at(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            dump(&conn, "meeting_templates", &all_cols, ""),
            marked,
            "zweimal Oeffnen: gleiche 9 Zeilen, aktuelle Zeilen unberuehrt (kein updated_at-Rauschen)"
        );
        assert_eq!(
            dump(
                &conn,
                "meeting_templates",
                &all_cols,
                "WHERE id = 'LEGACY-STD'"
            ),
            after_first
                .iter()
                .filter(|r| r[0] == Value::Text("LEGACY-STD".into()))
                .cloned()
                .collect::<Vec<_>>(),
            "Standardprotokoll unangetastet"
        );

        let infos = s.list_template_infos().unwrap();
        assert_eq!(infos.len(), 8);
        assert_eq!(
            infos[0].id, "builtin:allgemein",
            "Katalogreihenfolge, Standard zuerst"
        );
        assert!(infos.iter().all(|i| i.builtin));
        let catalog: Vec<String> = builtin_templates()
            .iter()
            .map(|(k, _, _)| builtin_id(k))
            .collect();
        assert_eq!(
            infos.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
            catalog
        );
    }

    #[test]
    fn a_fresh_database_gets_the_protocol_template_and_the_bundled_ones() {
        let (_dir, s) = tmp_store();
        assert_eq!(s.list_templates().unwrap().len(), 1);
        assert_eq!(s.list_templates().unwrap()[0].title, "Standardprotokoll");
        assert_eq!(s.list_template_infos().unwrap().len(), 8);
        let conn = s.get_connection().unwrap();
        assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM meeting_templates"), 9);
    }

    #[test]
    fn seeding_restores_edited_or_deleted_bundled_templates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let s = MeetingStore::open_at(&path).unwrap();
        let original = s.get_template_info("builtin:vertrieb").unwrap().unwrap();
        {
            let conn = s.get_connection().unwrap();
            conn.execute(
                "UPDATE meeting_templates SET title = 'Manipuliert', sections_json = '{}' WHERE id = 'builtin:vertrieb'",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE meeting_templates SET deleted_at = 5 WHERE id = 'builtin:kickoff'",
                [],
            )
            .unwrap();
        }
        assert!(s.get_template_info("builtin:vertrieb").unwrap().is_none());
        assert!(s.get_template_info("builtin:kickoff").unwrap().is_none());

        let s = MeetingStore::open_at(&path).unwrap();
        let restored = s.get_template_info("builtin:vertrieb").unwrap().unwrap();
        assert_eq!(restored.title, original.title);
        assert_eq!(restored.spec, original.spec);
        assert!(s.get_template_info("builtin:kickoff").unwrap().is_some());
        assert_eq!(s.list_template_infos().unwrap().len(), 8);
    }

    #[test]
    fn a_seeding_failure_does_not_lock_the_user_out_of_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        create_legacy_db(&path);
        {
            // Simuliert eine volle Platte beim Anlegen neuer Zeilen.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TRIGGER block_templates BEFORE INSERT ON meeting_templates
                 BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
            )
            .unwrap();
        }
        let s = MeetingStore::open_at(&path).expect("Store muss trotzdem oeffnen");
        assert_eq!(
            s.get_meeting("M1").unwrap().unwrap().title,
            "Kundengespräch Größe 🚀"
        );
        assert!(
            s.list_template_infos().unwrap().is_empty(),
            "nichts halb angelegt"
        );
        assert_eq!(s.list_templates().unwrap().len(), 1, "Altzeile heil");

        Connection::open(&path)
            .unwrap()
            .execute_batch("DROP TRIGGER block_templates;")
            .unwrap();
        let s = MeetingStore::open_at(&path).unwrap();
        assert_eq!(
            s.list_template_infos().unwrap().len(),
            8,
            "naechstes Oeffnen holt es nach"
        );
    }

    // ---- Notizblock ------------------------------------------------------

    #[test]
    fn notes_start_empty_and_round_trip_byte_exact() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let empty = s.get_notes(&m.id).unwrap();
        assert_eq!(
            (empty.revision, empty.blocks.len(), empty.updated_at),
            (0, 0, 0)
        );

        let blocks = vec![
            NoteBlock {
                id: "01A".into(),
                kind: NoteBlockKind::Heading,
                text: "  Führende Leerzeichen & Größe 🚀\nzweite Zeile  ".into(),
                at_ms: None,
                checked: false,
            },
            NoteBlock {
                id: "01B".into(),
                kind: NoteBlockKind::Todo,
                text: "Angebot schicken".into(),
                at_ms: Some(195_000),
                checked: true,
            },
        ];
        assert_eq!(s.save_notes(&m.id, &blocks, 0).unwrap(), 1);
        let loaded = s.get_notes(&m.id).unwrap();
        assert_eq!(loaded.blocks, blocks);
        assert_eq!(loaded.revision, 1);
        assert!(loaded.updated_at > 1_700_000_000, "Sekunden seit Epoche");
    }

    #[test]
    fn notes_revision_conflict_rejects_write() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        assert_eq!(s.save_notes(&m.id, &[block("b1", "eins")], 0).unwrap(), 1);
        assert_eq!(
            s.save_notes(&m.id, &[block("b1", "eins"), block("b2", "zwei")], 1)
                .unwrap(),
            2
        );

        for stale_base in [0u64, 1, 3, 99] {
            let err = s
                .save_notes(&m.id, &[block("x", "ueberschreibt")], stale_base)
                .unwrap_err();
            assert_eq!(err.to_string(), "revision_conflict", "base {stale_base}");
        }
        let notes = s.get_notes(&m.id).unwrap();
        assert_eq!(notes.revision, 2, "abgelehnte Speicherungen zaehlen nicht");
        assert_eq!(notes.blocks, vec![block("b1", "eins"), block("b2", "zwei")]);

        // Auch die allererste Speicherung braucht Basis 0.
        let other = live_meeting(&s);
        let err = s.save_notes(&other.id, &[block("y", "z")], 5).unwrap_err();
        assert_eq!(err.to_string(), "revision_conflict");
        assert_eq!(
            s.get_notes(&other.id).unwrap().revision,
            0,
            "nichts angelegt"
        );
    }

    #[test]
    fn notes_of_an_unknown_or_deleted_meeting_cannot_be_saved() {
        let (_dir, s) = tmp_store();
        assert!(s
            .save_notes("gibt-es-nicht", &[block("a", "b")], 0)
            .is_err());
        let m = live_meeting(&s);
        s.soft_delete_meeting(&m.id).unwrap();
        assert!(s.save_notes(&m.id, &[block("a", "b")], 0).is_err());
    }

    #[test]
    fn concurrent_saves_with_the_same_base_revision_have_exactly_one_winner() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let barrier = Barrier::new(8);
        let results: Vec<Result<u64>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let (s, m, barrier) = (&s, &m, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        s.save_notes(&m.id, &[block(&format!("b{i}"), "Text")], 0)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let wins = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(wins, 1, "genau ein Schreiber gewinnt: {results:?}");
        for r in results.iter().filter_map(|r| r.as_ref().err()) {
            assert_eq!(
                r.to_string(),
                "revision_conflict",
                "Verlierer bekommen den Konflikt, keinen DB-Fehler"
            );
        }
        assert_eq!(s.get_notes(&m.id).unwrap().revision, 1);
    }

    #[test]
    fn concurrent_read_modify_write_loses_no_update() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        std::thread::scope(|scope| {
            for t in 0..6 {
                let (s, m) = (&s, &m);
                scope.spawn(move || {
                    for i in 0..5 {
                        loop {
                            let current = s.get_notes(&m.id).unwrap();
                            let mut blocks = current.blocks.clone();
                            blocks.push(block(&format!("t{t}-{i}"), "x"));
                            match s.save_notes(&m.id, &blocks, current.revision) {
                                Ok(_) => break,
                                Err(e) if e.to_string() == "revision_conflict" => continue,
                                Err(e) => panic!("unerwarteter Fehler: {e}"),
                            }
                        }
                    }
                });
            }
        });
        let notes = s.get_notes(&m.id).unwrap();
        assert_eq!(notes.blocks.len(), 30, "kein Update geht verloren");
        assert_eq!(notes.revision, 30);
        let mut ids: Vec<&str> = notes.blocks.iter().map(|b| b.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 30, "jeder Block genau einmal");
    }

    #[test]
    fn a_write_that_aborts_midway_changes_nothing() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        s.save_notes(&m.id, &[block("b1", "alt")], 0).unwrap();
        s.get_connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_notes BEFORE UPDATE ON meeting_notes
                 BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
            )
            .unwrap();

        assert!(s.save_notes(&m.id, &[block("b1", "neu")], 1).is_err());
        s.get_connection()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_notes;")
            .unwrap();
        let notes = s.get_notes(&m.id).unwrap();
        assert_eq!(notes.revision, 1, "Revision nicht erhoeht");
        assert_eq!(
            notes.blocks,
            vec![block("b1", "alt")],
            "alter Stand erhalten"
        );
        // Nach dem Fehler ist der Store ohne Neustart weiter benutzbar.
        assert_eq!(s.save_notes(&m.id, &[block("b1", "neu")], 1).unwrap(), 2);
    }

    // ---- Loeschen, Epoche -------------------------------------------------

    fn enhanced_doc(s: &MeetingStore, meeting_id: &str) -> String {
        s.insert_document(
            meeting_id,
            "enhanced_notes",
            "enhanced@1",
            "{}",
            Some("builtin:allgemein"),
            None,
        )
        .unwrap()
    }

    fn task(text: &str, source: &str) -> ActionItem {
        ActionItem {
            id: String::new(),
            meeting_id: String::new(),
            text: text.into(),
            status: "todo".into(),
            assignee_label: None,
            document_id: None,
            entry_id: None,
            source_segment_ids: vec![],
            source: source.into(),
        }
    }

    #[test]
    fn soft_delete_hides_notes_and_action_items() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let keep = live_meeting(&s);
        s.save_notes(&m.id, &[block("b1", "Notiz")], 0).unwrap();
        s.save_notes(&keep.id, &[block("k1", "Bleibt")], 0).unwrap();
        let doc = enhanced_doc(&s, &m.id);
        s.replace_action_items(&m.id, &doc, &[task("Aufgabe", "ai")])
            .unwrap();
        assert_eq!(s.list_action_items(&m.id).unwrap().len(), 1);

        s.soft_delete_meeting(&m.id).unwrap();

        assert!(s.get_notes(&m.id).unwrap().blocks.is_empty());
        assert_eq!(s.get_notes(&m.id).unwrap().revision, 0);
        assert!(s.list_action_items(&m.id).unwrap().is_empty());
        let conn = s.get_connection().unwrap();
        assert_eq!(
            scalar(&conn, &format!("SELECT COUNT(*) FROM meeting_notes WHERE meeting_id = '{}' AND deleted_at IS NOT NULL", m.id)),
            1,
            "meeting_notes bekommt deleted_at"
        );
        assert_eq!(
            scalar(&conn, &format!("SELECT COUNT(*) FROM action_items WHERE meeting_id = '{}' AND deleted_at IS NULL", m.id)),
            0,
            "action_items bekommen deleted_at"
        );
        // Fremde Besprechung unberuehrt.
        assert_eq!(s.get_notes(&keep.id).unwrap().blocks.len(), 1);
    }

    #[test]
    fn soft_delete_is_all_or_nothing() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        s.save_notes(&m.id, &[block("b1", "Notiz")], 0).unwrap();
        s.get_connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_notes BEFORE UPDATE ON meeting_notes
                 BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;",
            )
            .unwrap();
        assert!(s.soft_delete_meeting(&m.id).is_err());
        s.get_connection()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_notes;")
            .unwrap();
        assert!(
            s.get_meeting(&m.id).unwrap().is_some(),
            "Besprechung nicht halb geloescht"
        );
        assert_eq!(s.get_notes(&m.id).unwrap().blocks.len(), 1);
    }

    #[test]
    fn clear_segments_bumps_the_segment_epoch() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        assert_eq!(s.segment_epoch(&m.id).unwrap(), 0, "ohne Transkript");
        s.clear_segments(&m.id).unwrap();
        assert_eq!(
            s.segment_epoch(&m.id).unwrap(),
            0,
            "ohne Transkriptzeile gibt es nichts zu entwerten"
        );

        let delta = |i: u32| TranscriptDelta {
            new_segments: vec![StoredSegment {
                segment_index: i,
                text: "x".into(),
                start_ms: 0,
                end_ms: 1,
                channel: 0,
                speaker_index: None,
            }],
        };
        s.append_delta(&m.id, &delta(0)).unwrap();
        assert_eq!(
            s.segment_epoch(&m.id).unwrap(),
            0,
            "Anhaengen aendert die Epoche nicht"
        );
        s.clear_segments(&m.id).unwrap();
        assert_eq!(s.segment_epoch(&m.id).unwrap(), 1);
        s.append_delta(&m.id, &delta(0)).unwrap();
        assert_eq!(
            s.segment_epoch(&m.id).unwrap(),
            1,
            "neue Segmente gehoeren zur neuen Epoche"
        );
        s.clear_segments(&m.id).unwrap();
        assert_eq!(s.segment_epoch(&m.id).unwrap(), 2);
        assert_eq!(s.segment_epoch("gibt-es-nicht").unwrap(), 0);
    }

    // ---- Dokumente -------------------------------------------------------

    #[test]
    fn documents_carry_their_template_and_a_millisecond_stamp() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let id = enhanced_doc(&s, &m.id);
        let doc = s.get_document(&id).unwrap().unwrap();
        assert_eq!(doc.template_id.as_deref(), Some("builtin:allgemein"));
        assert_eq!(doc.kind, "enhanced_notes");
        assert_eq!(doc.version, 1);
        assert!(
            doc.created_at < 100_000_000_000,
            "created_at bleibt in Sekunden"
        );
        assert!(
            doc.updated_at > 1_000_000_000_000,
            "updated_at in Millisekunden"
        );

        // upsert_document delegiert: gleiche Versionszaehlung, ohne Vorlage.
        let minutes = s
            .upsert_document(&m.id, "minutes", "markdown@1", "# P", Some("{}"))
            .unwrap();
        let minutes = s.get_document(&minutes).unwrap().unwrap();
        assert_eq!((minutes.version, minutes.template_id), (1, None));
        assert_eq!(s.get_documents(&m.id).unwrap().len(), 2);
        assert!(s.get_document("gibt-es-nicht").unwrap().is_none());
    }

    #[test]
    fn concurrent_inserts_get_distinct_versions() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let barrier = Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let (s, m, barrier) = (&s, &m, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    enhanced_doc(s, &m.id);
                });
            }
        });
        let mut versions: Vec<u32> = s
            .get_documents(&m.id)
            .unwrap()
            .iter()
            .map(|d| d.version)
            .collect();
        versions.sort_unstable();
        assert_eq!(versions, (1..=8).collect::<Vec<_>>());
    }

    #[test]
    fn update_document_body_detects_stale_writers() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let id = enhanced_doc(&s, &m.id);
        let v0 = s.get_document(&id).unwrap().unwrap().updated_at;

        let v1 = s.update_document_body(&id, "{\"a\":1}", v0).unwrap();
        assert!(v1 > v0);
        assert_eq!(s.get_document(&id).unwrap().unwrap().body, "{\"a\":1}");

        // Zweites Fenster mit altem Stempel: abgelehnt, nichts ueberschrieben.
        let err = s.update_document_body(&id, "{\"a\":2}", v0).unwrap_err();
        assert_eq!(err.to_string(), "stale_document");
        assert_eq!(s.get_document(&id).unwrap().unwrap().body, "{\"a\":1}");

        // Mit dem aktuellen Stempel geht es weiter; der Stempel steigt streng.
        let mut stamp = v1;
        for i in 0..50 {
            let next = s
                .update_document_body(&id, &format!("{{\"i\":{i}}}"), stamp)
                .unwrap();
            assert!(
                next > stamp,
                "auch zwei Speicherungen in derselben Millisekunde unterscheiden sich"
            );
            stamp = next;
        }

        assert_eq!(
            s.update_document_body("gibt-es-nicht", "x", 1)
                .unwrap_err()
                .to_string(),
            "document_not_found"
        );
        s.soft_delete_meeting(&m.id).unwrap();
        assert_eq!(
            s.update_document_body(&id, "x", stamp)
                .unwrap_err()
                .to_string(),
            "document_not_found",
            "geloeschte Dokumente sind nicht bearbeitbar"
        );
    }

    #[test]
    fn update_document_body_accepts_a_legacy_second_stamp() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let id = s
            .upsert_document(&m.id, "minutes", "markdown@1", "alt", None)
            .unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE meeting_documents SET updated_at = 1755603801 WHERE id = ?1",
                params![id],
            )
            .unwrap();
        let next = s.update_document_body(&id, "neu", 1755603801).unwrap();
        assert!(next > 1_000_000_000_000, "ab jetzt Millisekunden");
    }

    #[test]
    fn concurrent_stale_updates_have_exactly_one_winner() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let id = enhanced_doc(&s, &m.id);
        let stamp = s.get_document(&id).unwrap().unwrap().updated_at;
        let barrier = Barrier::new(6);
        let results: Vec<Result<i64>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..6)
                .map(|i| {
                    let (s, id, barrier) = (&s, &id, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        s.update_document_body(id, &format!("{{\"w\":{i}}}"), stamp)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "{results:?}"
        );
        for e in results.iter().filter_map(|r| r.as_ref().err()) {
            assert_eq!(e.to_string(), "stale_document");
        }
    }

    // ---- Vorlagen --------------------------------------------------------

    fn user_spec() -> TemplateSpec {
        TemplateSpec {
            version: 1,
            context: "Eigener Zweck".into(),
            sections: vec![
                TemplateSection {
                    id: "kurz".into(),
                    title: "Kurzfassung".into(),
                    instruction: "Zwei Sätze.".into(),
                    kind: SectionKind::Text,
                },
                TemplateSection {
                    id: "todo".into(),
                    title: "To-dos".into(),
                    instruction: String::new(),
                    kind: SectionKind::Tasks,
                },
            ],
        }
    }

    #[test]
    fn bundled_templates_are_read_only() {
        let (_dir, s) = tmp_store();
        let before = s.get_template_info("builtin:allgemein").unwrap().unwrap();
        let err = s
            .save_template(Some("builtin:allgemein"), "Umbenannt", &user_spec())
            .unwrap_err();
        assert_eq!(err.to_string(), "template_readonly");
        // Auch mit ungueltigem Inhalt: Schreibschutz kommt zuerst.
        let mut bad = user_spec();
        bad.sections.clear();
        assert_eq!(
            s.save_template(Some("builtin:vertrieb"), "x", &bad)
                .unwrap_err()
                .to_string(),
            "template_readonly"
        );
        assert_eq!(
            s.delete_template("builtin:allgemein")
                .unwrap_err()
                .to_string(),
            "template_readonly"
        );
        assert_eq!(
            s.get_template_info("builtin:allgemein").unwrap().unwrap(),
            before
        );
        assert_eq!(s.list_template_infos().unwrap().len(), 8);
    }

    #[test]
    fn user_templates_can_be_created_updated_and_deleted() {
        let (_dir, s) = tmp_store();
        let created = s
            .save_template(None, "  Mein Format  ", &user_spec())
            .unwrap();
        assert!(!created.builtin);
        assert_eq!(created.title, "Mein Format", "Titel wird getrimmt");
        assert!(!created.id.starts_with("builtin:"));

        let list = s.list_template_infos().unwrap();
        assert_eq!(list.len(), 9);
        assert_eq!(
            list[8], created,
            "eigene Vorlagen stehen hinter den mitgelieferten"
        );

        let mut spec = user_spec();
        spec.context = "Neuer Zweck".into();
        let updated = s
            .save_template(Some(&created.id), "Umbenannt", &spec)
            .unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(
            s.get_template_info(&created.id)
                .unwrap()
                .unwrap()
                .spec
                .context,
            "Neuer Zweck"
        );
        assert_eq!(
            s.list_template_infos().unwrap().len(),
            9,
            "Update legt keine zweite an"
        );

        s.delete_template(&created.id).unwrap();
        assert_eq!(s.list_template_infos().unwrap().len(), 8);
        assert!(s.get_template_info(&created.id).unwrap().is_none());
        assert_eq!(
            s.delete_template(&created.id).unwrap_err().to_string(),
            "template_not_found"
        );
        assert_eq!(
            s.save_template(Some(&created.id), "Zombie", &user_spec())
                .unwrap_err()
                .to_string(),
            "template_not_found",
            "geloeschte Vorlage wird nicht wiederbelebt"
        );
    }

    #[test]
    fn invalid_templates_are_rejected_before_anything_is_written() {
        let (_dir, s) = tmp_store();
        assert_eq!(
            s.save_template(None, "   ", &user_spec())
                .unwrap_err()
                .to_string(),
            "template_invalid:title"
        );
        let mut two_tasks = user_spec();
        two_tasks.sections[0].kind = SectionKind::Tasks;
        assert_eq!(
            s.save_template(None, "Zwei", &two_tasks)
                .unwrap_err()
                .to_string(),
            "template_invalid:tasks_sections"
        );
        assert_eq!(s.list_template_infos().unwrap().len(), 8);
        assert_eq!(
            scalar(
                &s.get_connection().unwrap(),
                "SELECT COUNT(*) FROM meeting_templates"
            ),
            9
        );
    }

    #[test]
    fn the_legacy_protocol_template_is_neither_listed_nor_editable() {
        let (_dir, s) = tmp_store();
        let legacy = s.list_templates().unwrap().remove(0);
        assert!(s
            .list_template_infos()
            .unwrap()
            .iter()
            .all(|i| i.id != legacy.id));
        assert_eq!(
            s.save_template(Some(&legacy.id), "Ueberschrieben", &user_spec())
                .unwrap_err()
                .to_string(),
            "template_not_found"
        );
        assert_eq!(
            s.delete_template(&legacy.id).unwrap_err().to_string(),
            "template_not_found"
        );
        let still = s.list_templates().unwrap();
        assert_eq!(still.len(), 1);
        assert_eq!(
            still[0].sections_json, legacy.sections_json,
            "Inhalt unveraendert"
        );
    }

    #[test]
    fn damaged_or_future_template_rows_are_hidden_not_fatal() {
        let (_dir, s) = tmp_store();
        {
            let conn = s.get_connection().unwrap();
            for (id, json) in [
                ("bad-json", "das ist kein json".to_string()),
                (
                    "future",
                    serde_json::to_string(&TemplateSpec {
                        version: 2,
                        ..user_spec()
                    })
                    .unwrap(),
                ),
                (
                    "no-sections",
                    "{\"version\":1,\"context\":\"\",\"sections\":[]}".to_string(),
                ),
            ] {
                conn.execute(
                    "INSERT INTO meeting_templates (id, title, sections_json, pinned, created_at, updated_at)
                     VALUES (?1, 'Kaputt', ?2, 0, 1, 1)",
                    params![id, json],
                )
                .unwrap();
            }
        }
        assert_eq!(s.list_template_infos().unwrap().len(), 8);
        assert!(s.get_template_info("future").unwrap().is_none());
    }

    #[test]
    fn user_templates_are_ordered_by_creation_time() {
        let (_dir, s) = tmp_store();
        let first = s.save_template(None, "Erste", &user_spec()).unwrap();
        let second = s.save_template(None, "Zweite", &user_spec()).unwrap();
        {
            let conn = s.get_connection().unwrap();
            conn.execute(
                "UPDATE meeting_templates SET created_at = 200 WHERE id = ?1",
                params![first.id],
            )
            .unwrap();
            conn.execute(
                "UPDATE meeting_templates SET created_at = 100 WHERE id = ?1",
                params![second.id],
            )
            .unwrap();
        }
        let ids: Vec<String> = s
            .list_template_infos()
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(&ids[8..], &[second.id, first.id]);
    }

    #[test]
    fn a_template_survives_export_and_import_through_the_store() {
        let (_dir, s) = tmp_store();
        let mut spec = user_spec();
        spec.context = "Größe & Übung – 🚀".into();
        let saved = s.save_template(None, "Übergabe", &spec).unwrap();

        let file = templates::export_file(&saved);
        let (title, imported_spec) = templates::import_file(&file).unwrap();
        let copy = s.save_template(None, &title, &imported_spec).unwrap();

        assert_ne!(copy.id, saved.id, "Import legt eine neue Vorlage an");
        assert_eq!(copy.title, saved.title);
        assert_eq!(copy.spec, saved.spec, "Rundreise verlustfrei");
        assert_eq!(s.list_template_infos().unwrap().len(), 10);

        // Auch ein mitgeliefertes Original laesst sich exportieren und als eigene Kopie importieren.
        let builtin = s.get_template_info("builtin:jour_fixe").unwrap().unwrap();
        let (t, sp) = templates::import_file(&templates::export_file(&builtin)).unwrap();
        let dup = s.save_template(None, &t, &sp).unwrap();
        assert!(!dup.builtin);
        assert_eq!(dup.spec, builtin.spec);
    }

    #[test]
    fn the_template_choice_of_a_meeting_is_stored() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        assert_eq!(s.meeting_template_id(&m.id).unwrap(), None);
        s.set_meeting_template(&m.id, Some("builtin:vertrieb"))
            .unwrap();
        assert_eq!(
            s.meeting_template_id(&m.id).unwrap().as_deref(),
            Some("builtin:vertrieb")
        );
        s.set_meeting_template(&m.id, Some("   ")).unwrap();
        assert_eq!(
            s.meeting_template_id(&m.id).unwrap(),
            None,
            "leer = Standardvorlage"
        );
        s.set_meeting_template(&m.id, Some("builtin:kickoff"))
            .unwrap();
        s.set_meeting_template(&m.id, None).unwrap();
        assert_eq!(s.meeting_template_id(&m.id).unwrap(), None);

        assert!(s.set_meeting_template("gibt-es-nicht", Some("x")).is_err());
        s.soft_delete_meeting(&m.id).unwrap();
        assert!(s.set_meeting_template(&m.id, Some("x")).is_err());
        assert_eq!(s.meeting_template_id(&m.id).unwrap(), None);
    }

    // ---- Aufgaben --------------------------------------------------------

    #[test]
    fn replace_action_items_swaps_versions_keeps_done_and_manual() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let doc1 = enhanced_doc(&s, &m.id);
        let mut a = task("Angebot senden", "ai");
        a.assignee_label = Some("Müller".into());
        a.entry_id = Some("E4".into());
        a.source_segment_ids = vec![3, 12];
        let stored = s
            .replace_action_items(&m.id, &doc1, &[a, task("Termin klären", "user")])
            .unwrap();
        assert_eq!(stored.len(), 2);
        assert!(stored
            .iter()
            .all(|t| !t.id.is_empty() && t.meeting_id == m.id));
        assert!(stored
            .iter()
            .all(|t| t.document_id.as_deref() == Some(doc1.as_str())));
        assert_eq!(stored[0].status, "todo");

        let listed = s.list_action_items(&m.id).unwrap();
        assert_eq!(
            listed, stored,
            "Liste = gespeicherte Zeilen, in Reihenfolge"
        );
        assert_eq!(listed[0].source_segment_ids, vec![3, 12]);
        assert_eq!(listed[0].assignee_label.as_deref(), Some("Müller"));
        assert_eq!(listed[0].entry_id.as_deref(), Some("E4"));

        s.set_action_item_status(&stored[0].id, true).unwrap();
        assert_eq!(s.list_action_items(&m.id).unwrap()[0].status, "done");
        s.set_action_item_status(&stored[0].id, false).unwrap();
        s.set_action_item_status(&stored[0].id, true).unwrap();

        // Eine manuelle Aufgabe (kommt in M9 aus der UI; hier direkt eingefuegt).
        s.get_connection()
            .unwrap()
            .execute(
                "INSERT INTO action_items (id, meeting_id, text, status, source, kind, created_at, updated_at)
                 VALUES ('MAN1', ?1, 'Von Hand', 'todo', 'manual', 'task', 1, 1)",
                params![m.id],
            )
            .unwrap();

        // Neue Version: gleicher Text in anderer Schreibweise bleibt erledigt.
        let doc2 = enhanced_doc(&s, &m.id);
        let stored2 = s
            .replace_action_items(
                &m.id,
                &doc2,
                &[
                    task("  ANGEBOT   senden ", "ai"),
                    task("Neue Aufgabe", "ai"),
                    task("   ", "ai"),
                ],
            )
            .unwrap();
        assert_eq!(stored2.len(), 2, "leere Texte werden uebersprungen");
        assert_eq!(
            stored2[0].status, "done",
            "erledigt bleibt erledigt (normalisierter Text)"
        );
        assert_eq!(stored2[1].status, "todo");

        let listed2: Vec<String> = s
            .list_action_items(&m.id)
            .unwrap()
            .into_iter()
            .map(|t| t.text)
            .collect();
        assert_eq!(
            listed2,
            vec!["Von Hand", "  ANGEBOT   senden ", "Neue Aufgabe"],
            "Vorversion weg (auch 'Termin klaeren'), manuelle bleibt"
        );
        let conn = s.get_connection().unwrap();
        assert_eq!(
            scalar(
                &conn,
                "SELECT COUNT(*) FROM action_items WHERE deleted_at IS NOT NULL"
            ),
            2,
            "Vorversion ist soft-deleted, nicht entfernt"
        );
        assert_eq!(
            s.set_action_item_status(&stored[0].id, false)
                .unwrap_err()
                .to_string(),
            "action_item_not_found",
            "ersetzte Zeile ist nicht mehr bedienbar"
        );
        assert_eq!(
            s.set_action_item_status("gibt-es-nicht", true)
                .unwrap_err()
                .to_string(),
            "action_item_not_found"
        );
    }

    #[test]
    fn a_task_a_user_ticked_in_the_notepad_starts_done() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let doc = enhanced_doc(&s, &m.id);
        let mut t = task("Bereits erledigt", "user");
        t.status = "done".into();
        let stored = s.replace_action_items(&m.id, &doc, &[t]).unwrap();
        assert_eq!(stored[0].status, "done");
    }

    #[test]
    fn replace_action_items_validates_and_never_half_applies() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let other = live_meeting(&s);
        let doc = enhanced_doc(&s, &m.id);
        let other_doc = enhanced_doc(&s, &other.id);
        s.replace_action_items(&m.id, &doc, &[task("Bleibt", "ai")])
            .unwrap();

        let mut bad_status = task("x", "ai");
        bad_status.status = "erledigt".into();
        assert_eq!(
            s.replace_action_items(&m.id, &doc, &[bad_status])
                .unwrap_err()
                .to_string(),
            "invalid_status"
        );
        assert_eq!(
            s.replace_action_items(&m.id, &doc, &[task("x", "robot")])
                .unwrap_err()
                .to_string(),
            "invalid_source"
        );
        assert_eq!(
            s.replace_action_items(&m.id, "gibt-es-nicht", &[task("x", "ai")])
                .unwrap_err()
                .to_string(),
            "document_not_found"
        );
        assert_eq!(
            s.replace_action_items(&m.id, &other_doc, &[task("x", "ai")])
                .unwrap_err()
                .to_string(),
            "document_not_found",
            "Dokument einer anderen Besprechung"
        );
        assert_eq!(
            s.list_action_items(&m.id).unwrap().len(),
            1,
            "alter Stand nach jedem Fehler intakt"
        );

        // Fehler mitten im Einfuegen: der Soft-Delete davor wird zurueckgerollt.
        s.get_connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER boom BEFORE INSERT ON action_items WHEN NEW.text = 'BOOM'
                 BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
            )
            .unwrap();
        assert!(s
            .replace_action_items(&m.id, &doc, &[task("Neu 1", "ai"), task("BOOM", "ai")])
            .is_err());
        let listed = s.list_action_items(&m.id).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].text, "Bleibt");

        s.soft_delete_meeting(&m.id).unwrap();
        assert!(s
            .replace_action_items(&m.id, &doc, &[task("x", "ai")])
            .is_err());
    }

    #[test]
    fn only_tasks_of_the_newest_document_version_are_listed() {
        let (_dir, s) = tmp_store();
        let m = live_meeting(&s);
        let old_doc = enhanced_doc(&s, &m.id);
        let _newest = enhanced_doc(&s, &m.id);
        s.replace_action_items(&m.id, &old_doc, &[task("Von der alten Version", "ai")])
            .unwrap();
        assert!(
            s.list_action_items(&m.id).unwrap().is_empty(),
            "Zeilen einer aelteren Dokumentversion erscheinen nicht"
        );
    }
}
