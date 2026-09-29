//! Store-Erweiterung fuer den Such-Index (M4 §4): Chunks, Index-Zustand,
//! Volltextsuche, Ordner, Recipes und Chat-Verlaeufe als reine
//! `MeetingStore`-Funktionen (keine Commands, kein Embedding-Server).
//!
//! Regeln, die hier durchgehend gelten:
//! - Jede Schreibfunktion ist EINE `BEGIN IMMEDIATE`-Transaktion (kurz, je
//!   Besprechung). Ist die Datenbank gesperrt, wartet SQLite bis zum
//!   Busy-Timeout von rusqlite (5 s); danach kommt ein Fehler und nichts ist
//!   geschrieben. Bei jedem Fehler mitten drin (auch voller Datentraeger)
//!   rollt die Transaktion vollstaendig zurueck.
//! - Fehler, auf die Aufrufer reagieren, sind Codes im Fehlertext
//!   (`stale_epoch`, `folder_not_found`, ...), wie in M1.
//! - Gesucht wird nur in LEBENDEN Besprechungen (`deleted_at IS NULL`); der
//!   Loeschpfad entfernt den Index ohnehin in derselben Transaktion.
//! - Kein Text (Frage, Chunk, Notiz) landet im Log oder in einer Fehlermeldung.

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{HashMap, HashSet};
use ulid::Ulid;

use super::super::store::{Meeting, MeetingStore};
use super::chunking::{
    and_of_terms, embed_text_for, fts_query_trigram, query_terms, ChunkDraft, ChunkSource,
};

/// Mehr Chunk-Text als das nimmt der Store nicht (ein Chunker-Fehler soll
/// keine Megabyte grossen Zeilen in die Datenbank und den FTS-Index tragen).
const MAX_CHUNK_BYTES: usize = 64 * 1024;
/// Kandidaten, die die Listensuche nach BM25 behaelt, bevor sie nach
/// Besprechung gruppiert (Obergrenze fuer Zeit und Speicher bei Allerwelts-Woertern).
const SEARCH_CANDIDATES: u32 = 2_000;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_THREAD_MESSAGES: i64 = 1_000;

/// Werte von `IndexState.status`.
pub const STATUS_PENDING: &str = "pending";
pub const STATUS_LEXICAL: &str = "lexical";
pub const STATUS_READY: &str = "ready";
pub const STATUS_ERROR: &str = "error";

const MEETING_COLUMNS: &str = "m.id, m.title, m.status, m.source, m.started_at, m.ended_at, \
     m.language, m.mic_audio_path, m.system_audio_path, m.duration_ms, m.consent_confirmed_at, \
     m.audio_retention_until, m.source_path, m.created_at, m.deleted_at";

// ---------------------------------------------------------------------------
// Typen
// ---------------------------------------------------------------------------

/// Woraus der Index einer Besprechung zuletzt gebaut wurde (Zeile in
/// `meeting_index_state`). Der Vergleich mit dem aktuellen Stand der
/// Besprechung entscheidet, ob neu indexiert werden muss.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct IndexState {
    pub transcript_epoch: Option<u32>,
    pub transcript_rev: Option<u64>,
    pub notes_revision: Option<u64>,
    pub enhanced_doc_id: Option<String>,
    pub enhanced_updated_at: Option<i64>,
    pub title: Option<String>,
    pub embed_model: Option<String>,
    pub embedded_at: Option<i64>,
    /// `pending` | `lexical` | `ready` | `error`
    pub status: String,
    pub error: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct IndexCounts {
    /// Lebende, fertige Besprechungen (nur die sind indexierbar).
    pub meetings: u32,
    pub pending: u32,
    pub lexical: u32,
    pub embedded: u32,
    pub error: u32,
    pub chunks: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ChunkRow {
    pub id: i64,
    pub meeting_id: String,
    pub source: ChunkSource,
    pub epoch: u32,
    pub segment_ids: Vec<u32>,
    pub ref_keys: Vec<String>,
    pub document_id: Option<String>,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub channel: Option<u8>,
    pub text: String,
    pub started_at: Option<i64>,
}

/// Filter der Listensuche. `source` ist die HERKUNFT der Besprechung
/// (`live` | `import` | `subtitle`), nicht die Chunk-Quelle.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct MeetingFilter {
    pub folder_id: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub source: Option<String>,
    pub has_notes: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingSearchItem {
    pub meeting: Meeting,
    /// HTML-sicher: alles ausser den Treffermarkierungen `<mark>...</mark>` ist maskiert.
    pub snippet: Option<String>,
    pub hit_source: Option<ChunkSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingSearchPage {
    pub items: Vec<MeetingSearchItem>,
    /// Besprechungen mit Treffern. Bei `truncated` eine Untergrenze.
    pub total: u32,
    /// Die Trefferliste wurde bei `SEARCH_CANDIDATES` Chunks gekappt (Allerwelts-Suchwort).
    pub truncated: bool,
}

/// Eingrenzung fuer Chat und Suche ueber viele Besprechungen (M4 §6). Alle
/// gesetzten Felder gelten zugleich (UND).
#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct ScopeFilter {
    pub meeting_ids: Option<Vec<String>>,
    pub folder_id: Option<String>,
    pub person: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub sort: i64,
    pub meeting_count: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Recipe mit Rohtext der Spezifikation; Form und Inhalt prueft das Chat-Modul (P4c).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RecipeInfo {
    pub id: String,
    pub title: String,
    pub spec_json: String,
    pub builtin: bool,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ChatThread {
    pub id: String,
    pub scope_json: String,
    pub meeting_id: Option<String>,
    pub title: Option<String>,
    pub message_count: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ChatMessageRow {
    pub id: String,
    pub thread_id: String,
    /// `user` | `assistant`
    pub role: String,
    pub content: String,
    pub citations_json: Option<String>,
    pub coverage_json: Option<String>,
    pub created_at: i64,
}

/// Welche Verlaeufe `thread_list` liefert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadScope {
    Meeting(String),
    Global,
}

pub const BUILTIN_PREFIX: &str = "builtin:";

// ---------------------------------------------------------------------------
// Hilfen
// ---------------------------------------------------------------------------

fn json_ids<T: Serialize>(items: &[T]) -> Result<String> {
    Ok(serde_json::to_string(items)?)
}

fn html_escape_snippet(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{1}' => out.push_str("<mark>"),
            '\u{2}' => out.push_str("</mark>"),
            other => out.push(other),
        }
    }
    out
}

fn is_valid_status(status: &str) -> bool {
    matches!(
        status,
        STATUS_PENDING | STATUS_LEXICAL | STATUS_READY | STATUS_ERROR
    )
}

fn map_index_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<IndexState> {
    Ok(IndexState {
        transcript_epoch: row
            .get::<_, Option<i64>>("transcript_epoch")?
            .map(|v| v as u32),
        transcript_rev: row
            .get::<_, Option<i64>>("transcript_rev")?
            .map(|v| v as u64),
        notes_revision: row
            .get::<_, Option<i64>>("notes_revision")?
            .map(|v| v as u64),
        enhanced_doc_id: row.get("enhanced_doc_id")?,
        enhanced_updated_at: row.get("enhanced_updated_at")?,
        title: row.get("title")?,
        embed_model: row.get("embed_model")?,
        embedded_at: row.get("embedded_at")?,
        status: row.get("status")?,
        error: row.get("error")?,
        updated_at: row.get("updated_at")?,
    })
}

fn map_chunk_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChunkRow> {
    // Beschaedigtes JSON kostet die Quellverweise eines Chunks, nicht die Abfrage.
    let segment_ids: String = row.get("segment_ids")?;
    let ref_keys: String = row.get("ref_keys")?;
    Ok(ChunkRow {
        id: row.get("id")?,
        meeting_id: row.get("meeting_id")?,
        source: ChunkSource::parse(&row.get::<_, String>("source")?)
            .unwrap_or(ChunkSource::Transcript),
        epoch: row.get::<_, i64>("epoch")? as u32,
        segment_ids: serde_json::from_str(&segment_ids).unwrap_or_default(),
        ref_keys: serde_json::from_str(&ref_keys).unwrap_or_default(),
        document_id: row.get("document_id")?,
        start_ms: row.get::<_, Option<i64>>("start_ms")?.map(|v| v as u64),
        end_ms: row.get::<_, Option<i64>>("end_ms")?.map(|v| v as u64),
        channel: row.get::<_, Option<i64>>("channel")?.map(|v| v as u8),
        text: row.get("text")?,
        started_at: row.get("started_at")?,
    })
}

/// Gemeinsame Einschraenkungen auf `meetings m` fuer Listensuche und
/// Scope-Aufloesung. Haengt Parameter an `params` an und liefert die
/// `AND ...`-Fragmente.
fn meeting_constraints(
    folder_id: Option<&str>,
    from: Option<i64>,
    to: Option<i64>,
    source: Option<&str>,
    has_notes: bool,
    params: &mut Vec<Value>,
) -> String {
    let mut sql = String::new();
    if let Some(folder_id) = folder_id {
        params.push(Value::Text(folder_id.to_string()));
        sql.push_str(&format!(
            " AND m.id IN (SELECT fi.meeting_id FROM meeting_folder_items fi
                           JOIN meeting_folders f ON f.id = fi.folder_id AND f.deleted_at IS NULL
                           WHERE fi.folder_id = ?{})",
            params.len()
        ));
    }
    if let Some(from) = from {
        params.push(Value::Integer(from));
        sql.push_str(&format!(
            " AND COALESCE(m.started_at, m.created_at) >= ?{}",
            params.len()
        ));
    }
    if let Some(to) = to {
        params.push(Value::Integer(to));
        sql.push_str(&format!(
            " AND COALESCE(m.started_at, m.created_at) <= ?{}",
            params.len()
        ));
    }
    if let Some(source) = source {
        params.push(Value::Text(source.to_string()));
        sql.push_str(&format!(" AND m.source = ?{}", params.len()));
    }
    if has_notes {
        sql.push_str(
            " AND m.id IN (SELECT meeting_id FROM meeting_notes
                            WHERE deleted_at IS NULL AND blocks_json <> '[]'
                           UNION
                           SELECT meeting_id FROM meeting_documents
                            WHERE kind = 'enhanced_notes' AND deleted_at IS NULL)",
        );
    }
    sql
}

impl MeetingStore {
    // ---- Chunks und Index-Zustand ----------------------------------------

    /// Ersetzt die Chunks der genannten Quellen einer Besprechung und schreibt
    /// den Index-Zustand, alles in EINER Transaktion. Andere Quellen bleiben
    /// unberuehrt (samt Vektoren). Liefert die neuen Chunk-IDs in Reihenfolge
    /// der `drafts` (leere Texte entfallen).
    ///
    /// Fehlercodes: `chunk_source_not_replaced` (ein Entwurf hat eine Quelle
    /// ausserhalb von `sources`), `chunk_too_large`, `invalid_status`, und die
    /// Veraltet-Waechter `stale_epoch` / `stale_transcript` / `stale_notes` /
    /// `stale_document`: hat sich die Quelle seit dem Lesen des Indexers
    /// geaendert (Neu-Transkription, Bearbeitung), wird NICHTS geschrieben und
    /// der Indexer reiht den Auftrag neu ein.
    pub fn replace_meeting_chunks(
        &self,
        meeting_id: &str,
        sources: &[ChunkSource],
        drafts: &[ChunkDraft],
        state: &IndexState,
    ) -> Result<Vec<i64>> {
        if !is_valid_status(&state.status) {
            return Err(anyhow!("invalid_status"));
        }
        for draft in drafts {
            if !sources.contains(&draft.source) {
                return Err(anyhow!("chunk_source_not_replaced"));
            }
            if draft.text.len() > MAX_CHUNK_BYTES {
                return Err(anyhow!("chunk_too_large"));
            }
        }
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;

        let started_at: Option<i64> = tx
            .query_row(
                "SELECT COALESCE(started_at, created_at) FROM meetings
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("Meeting {} not found", meeting_id))?;

        if sources.contains(&ChunkSource::Transcript) {
            let (epoch, rev): (i64, i64) = tx
                .query_row(
                    "SELECT segment_epoch, content_revision FROM transcripts
                     WHERE meeting_id = ?1 AND deleted_at IS NULL",
                    params![meeting_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .unwrap_or((0, 0));
            if i64::from(state.transcript_epoch.unwrap_or(0)) != epoch {
                return Err(anyhow!("stale_epoch"));
            }
            if state.transcript_rev.unwrap_or(0) as i64 != rev {
                return Err(anyhow!("stale_transcript"));
            }
        }
        if sources.contains(&ChunkSource::UserNotes) {
            let revision: i64 = tx
                .query_row(
                    "SELECT revision FROM meeting_notes WHERE meeting_id = ?1 AND deleted_at IS NULL",
                    params![meeting_id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(0);
            if state.notes_revision.unwrap_or(0) as i64 != revision {
                return Err(anyhow!("stale_notes"));
            }
        }
        if sources.contains(&ChunkSource::AiNotes) {
            let latest: Option<String> = tx
                .query_row(
                    "SELECT id FROM meeting_documents
                     WHERE meeting_id = ?1 AND kind = 'enhanced_notes' AND deleted_at IS NULL
                     ORDER BY version DESC LIMIT 1",
                    params![meeting_id],
                    |row| row.get(0),
                )
                .optional()?;
            if state.enhanced_doc_id != latest {
                return Err(anyhow!("stale_document"));
            }
        }

        for source in sources {
            // Die Trigger entfernen FTS-Eintraege und Vektoren mit.
            tx.execute(
                "DELETE FROM meeting_chunks WHERE meeting_id = ?1 AND source = ?2",
                params![meeting_id, source.as_str()],
            )?;
        }

        let mut ids = Vec::with_capacity(drafts.len());
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO meeting_chunks
                   (meeting_id, source, epoch, segment_ids, ref_keys, document_id,
                    start_ms, end_ms, channel, text, started_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;
            for draft in drafts {
                if draft.text.trim().is_empty() {
                    continue;
                }
                insert.execute(params![
                    meeting_id,
                    draft.source.as_str(),
                    i64::from(draft.epoch),
                    json_ids(&draft.segment_ids)?,
                    json_ids(&draft.ref_keys)?,
                    draft.document_id,
                    draft.start_ms.map(|v| v as i64),
                    draft.end_ms.map(|v| v as i64),
                    draft.channel.map(i64::from),
                    draft.text,
                    started_at,
                    now
                ])?;
                ids.push(tx.last_insert_rowid());
            }
        }

        tx.execute(
            "INSERT INTO meeting_index_state
               (meeting_id, transcript_epoch, transcript_rev, notes_revision, enhanced_doc_id,
                enhanced_updated_at, title, embed_model, embedded_at, status, error, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(meeting_id) DO UPDATE SET
               transcript_epoch = excluded.transcript_epoch,
               transcript_rev = excluded.transcript_rev,
               notes_revision = excluded.notes_revision,
               enhanced_doc_id = excluded.enhanced_doc_id,
               enhanced_updated_at = excluded.enhanced_updated_at,
               title = excluded.title,
               embed_model = excluded.embed_model,
               embedded_at = excluded.embedded_at,
               status = excluded.status,
               error = excluded.error,
               updated_at = excluded.updated_at",
            params![
                meeting_id,
                state.transcript_epoch.map(i64::from),
                state.transcript_rev.map(|v| v as i64),
                state.notes_revision.map(|v| v as i64),
                state.enhanced_doc_id,
                state.enhanced_updated_at,
                state.title,
                state.embed_model,
                state.embedded_at,
                state.status,
                state.error,
                now
            ],
        )?;
        tx.commit()?;
        Ok(ids)
    }

    pub fn index_state(&self, meeting_id: &str) -> Result<Option<IndexState>> {
        let conn = self.get_connection()?;
        Ok(conn
            .query_row(
                "SELECT transcript_epoch, transcript_rev, notes_revision, enhanced_doc_id,
                        enhanced_updated_at, title, embed_model, embedded_at, status, error, updated_at
                 FROM meeting_index_state WHERE meeting_id = ?1",
                params![meeting_id],
                map_index_state,
            )
            .optional()?)
    }

    /// Setzt Status (und Fehlertext) einer schon indexierten Besprechung; mit
    /// `embed_model` zugleich `embed_model`/`embedded_at` (Vektorstufe fertig).
    pub fn set_index_status(
        &self,
        meeting_id: &str,
        status: &str,
        error: Option<&str>,
        embed_model: Option<&str>,
    ) -> Result<()> {
        if !is_valid_status(status) {
            return Err(anyhow!("invalid_status"));
        }
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let changed = tx.execute(
            "UPDATE meeting_index_state SET status = ?1, error = ?2, updated_at = ?3,
               embed_model = COALESCE(?4, embed_model),
               embedded_at = CASE WHEN ?4 IS NULL THEN embedded_at ELSE ?3 END
             WHERE meeting_id = ?5",
            params![status, error, now, embed_model, meeting_id],
        )?;
        if changed == 0 {
            return Err(anyhow!("index_state_not_found"));
        }
        tx.commit()?;
        Ok(())
    }

    /// Fertige, lebende Besprechungen, deren Index nicht (mehr) zum aktuellen
    /// Stand passt: noch nie indexiert oder Epoche, Transkript-Revision,
    /// Notizen-Revision, neueste KI-Notizen-Version (Id oder Stempel) oder
    /// Titel weichen ab. Neueste zuerst. Fehlende Werte zaehlen als 0/leer, so
    /// dass ein Indexer, der `None` statt 0 schreibt, keine Endlosschleife ausloest.
    pub fn stale_meetings(&self, limit: u32) -> Result<Vec<String>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT m.id FROM meetings m
             LEFT JOIN meeting_index_state s ON s.meeting_id = m.id
             LEFT JOIN transcripts t ON t.meeting_id = m.id AND t.deleted_at IS NULL
             LEFT JOIN meeting_notes n ON n.meeting_id = m.id AND n.deleted_at IS NULL
             LEFT JOIN meeting_documents d ON d.id = (
                 SELECT id FROM meeting_documents
                 WHERE meeting_id = m.id AND kind = 'enhanced_notes' AND deleted_at IS NULL
                 ORDER BY version DESC LIMIT 1)
             WHERE m.deleted_at IS NULL AND m.status = 'ready'
               AND (s.meeting_id IS NULL
                    OR COALESCE(s.transcript_epoch, 0) <> COALESCE(t.segment_epoch, 0)
                    OR COALESCE(s.transcript_rev, 0) <> COALESCE(t.content_revision, 0)
                    OR COALESCE(s.notes_revision, 0) <> COALESCE(n.revision, 0)
                    OR COALESCE(s.enhanced_doc_id, '') <> COALESCE(d.id, '')
                    OR COALESCE(s.enhanced_updated_at, 0) <> COALESCE(d.updated_at, 0)
                    OR COALESCE(s.title, '') <> m.title)
             ORDER BY m.created_at DESC, m.id
             LIMIT ?1",
        )?;
        let ids = stmt
            .query_map(params![limit], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    pub fn index_counts(&self) -> Result<IndexCounts> {
        let conn = self.get_connection()?;
        let meetings: i64 = conn.query_row(
            "SELECT COUNT(*) FROM meetings WHERE deleted_at IS NULL AND status = 'ready'",
            [],
            |row| row.get(0),
        )?;
        let by_status = |status: &str| -> Result<i64> {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM meeting_index_state s
                 JOIN meetings m ON m.id = s.meeting_id
                 WHERE m.deleted_at IS NULL AND m.status = 'ready' AND s.status = ?1",
                params![status],
                |row| row.get(0),
            )?)
        };
        let lexical = by_status(STATUS_LEXICAL)?;
        let embedded = by_status(STATUS_READY)?;
        let error = by_status(STATUS_ERROR)?;
        // Der Index von `meeting_id` deckt COUNT(*) ab: kein Lesen der Chunk-Texte.
        let chunks: i64 =
            conn.query_row("SELECT COUNT(*) FROM meeting_chunks", [], |row| row.get(0))?;
        Ok(IndexCounts {
            meetings: meetings as u32,
            pending: (meetings - lexical - embedded - error).max(0) as u32,
            lexical: lexical as u32,
            embedded: embedded as u32,
            error: error as u32,
            chunks: chunks as u32,
        })
    }

    /// Chunks ohne Vektor fuer `model` als `(chunk_id, Einbettungstext)`,
    /// aelteste zuerst. Der Einbettungstext wird aus der AKTUELLEN Besprechung
    /// (Titel, Datum, Ordner) neu aufgebaut; gespeichert ist er nicht.
    pub fn chunks_without_vectors(&self, model: &str, limit: u32) -> Result<Vec<(i64, String)>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT c.id, c.source, c.text, m.title, COALESCE(m.started_at, m.created_at),
                    CASE WHEN c.source = 'title' THEN
                      (SELECT group_concat(f.name, char(10))
                         FROM meeting_folder_items fi
                         JOIN meeting_folders f ON f.id = fi.folder_id AND f.deleted_at IS NULL
                        WHERE fi.meeting_id = m.id)
                    END
             FROM meeting_chunks c
             JOIN meetings m ON m.id = c.meeting_id AND m.deleted_at IS NULL
             WHERE NOT EXISTS (SELECT 1 FROM meeting_chunk_vectors v
                               WHERE v.chunk_id = c.id AND v.model = ?1)
             ORDER BY c.id
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![model, limit], |row| {
                let source: String = row.get(1)?;
                let text: String = row.get(2)?;
                let title: String = row.get(3)?;
                let started_at: Option<i64> = row.get(4)?;
                let folders: Option<String> = row.get(5)?;
                let folders: Vec<String> = folders
                    .map(|f| f.lines().map(str::to_string).collect())
                    .unwrap_or_default();
                Ok((
                    row.get::<_, i64>(0)?,
                    embed_text_for(
                        ChunkSource::parse(&source).unwrap_or(ChunkSource::Transcript),
                        &title,
                        started_at,
                        &folders,
                        &text,
                    ),
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Speichert Vektoren (f32, L2-normalisiert) fuer `model`. Alle Vektoren
    /// eines Aufrufs muessen dieselbe Laenge haben, endlich und ungleich Null
    /// sein (`invalid_vector`); die Laenge muss zu vorhandenen Vektoren desselben
    /// Modells passen (`vector_dim_mismatch`). Chunks, die zwischenzeitlich
    /// verschwunden sind (neu indexiert), werden STILL uebersprungen; das
    /// verhindert Vektoren ohne Chunk. Liefert die Zahl gespeicherter Vektoren.
    pub fn put_vectors(&self, model: &str, rows: &[(i64, Vec<f32>)]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        if model.trim().is_empty() || model.len() > 128 {
            return Err(anyhow!("invalid_vector"));
        }
        let dim = rows[0].1.len();
        if dim == 0 || dim > 8_192 {
            return Err(anyhow!("invalid_vector"));
        }
        let mut blobs: Vec<(i64, Vec<u8>)> = Vec::with_capacity(rows.len());
        for (chunk_id, vec) in rows {
            if vec.len() != dim || vec.iter().any(|x| !x.is_finite()) {
                return Err(anyhow!("invalid_vector"));
            }
            let norm = vec
                .iter()
                .map(|x| f64::from(*x) * f64::from(*x))
                .sum::<f64>()
                .sqrt();
            if norm <= 0.0 || !norm.is_finite() {
                return Err(anyhow!("invalid_vector"));
            }
            let mut blob = Vec::with_capacity(dim * 4);
            for x in vec {
                blob.extend_from_slice(&((f64::from(*x) / norm) as f32).to_le_bytes());
            }
            blobs.push((*chunk_id, blob));
        }

        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let existing_dim: Option<i64> = tx
            .query_row(
                "SELECT dim FROM meeting_chunk_vectors WHERE model = ?1 LIMIT 1",
                params![model],
                |row| row.get(0),
            )
            .optional()?;
        if existing_dim.is_some_and(|d| d as usize != dim) {
            return Err(anyhow!("vector_dim_mismatch"));
        }
        let mut stored = 0usize;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO meeting_chunk_vectors (chunk_id, model, dim, vec)
                 SELECT ?1, ?2, ?3, ?4
                 WHERE EXISTS (SELECT 1 FROM meeting_chunks WHERE id = ?1)",
            )?;
            for (chunk_id, blob) in &blobs {
                stored += insert.execute(params![chunk_id, model, dim as i64, blob])?;
            }
        }
        tx.commit()?;
        Ok(stored)
    }

    /// Leert den gesamten Such-Index (Chunks, FTS, Vektoren, Zustand) und baut
    /// die FTS-Strukturen aus dem (dann leeren) Inhalt neu auf. Wiederherstellung
    /// bei beschaedigtem Index; Transkript und Notizen bleiben unberuehrt.
    pub fn clear_search_index(&self) -> Result<()> {
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        tx.execute("DELETE FROM meeting_chunks", [])?;
        tx.execute("DELETE FROM meeting_chunk_vectors", [])?;
        tx.execute("DELETE FROM meeting_index_state", [])?;
        tx.execute_batch(
            "INSERT INTO meeting_chunks_fts_words(meeting_chunks_fts_words) VALUES ('rebuild');
             INSERT INTO meeting_chunks_fts_tri(meeting_chunks_fts_tri) VALUES ('rebuild');",
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Prueft, ob beide FTS-Tabellen zum Inhalt von `meeting_chunks` passen.
    pub fn search_index_is_consistent(&self) -> Result<bool> {
        let conn = self.get_connection()?;
        for sql in [
            "INSERT INTO meeting_chunks_fts_words(meeting_chunks_fts_words, rank) VALUES ('integrity-check', 1)",
            "INSERT INTO meeting_chunks_fts_tri(meeting_chunks_fts_tri, rank) VALUES ('integrity-check', 1)",
        ] {
            if conn.execute(sql, []).is_err() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    // ---- Suche -----------------------------------------------------------

    /// BM25-Wortsuche (unicode61) ueber die Chunks der Besprechungen in `scope`,
    /// beste zuerst. `fts` kommt aus `chunking::fts_query_words`. Die Zahl je
    /// Chunk ist die Relevanz (groesser = besser; = -bm25). Leerer Scope = keine
    /// Treffer (bewusst: "nichts im Scope" ist nicht "alles").
    pub fn search_words(&self, fts: &str, scope: &[String], limit: u32) -> Result<Vec<(i64, f64)>> {
        if scope.is_empty() || limit == 0 || fts.trim().is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.get_connection()?;
        // CROSS JOIN fixiert die Reihenfolge: erst der FTS-Treffer, dann der Chunk.
        let mut stmt = conn.prepare_cached(
            "SELECT meeting_chunks_fts_words.rowid, meeting_chunks_fts_words.rank
             FROM meeting_chunks_fts_words
             CROSS JOIN meeting_chunks c ON c.id = meeting_chunks_fts_words.rowid
             JOIN meetings m ON m.id = c.meeting_id
             WHERE meeting_chunks_fts_words MATCH ?1
               AND m.deleted_at IS NULL
               AND c.meeting_id IN (SELECT value FROM json_each(?2))
             ORDER BY meeting_chunks_fts_words.rank
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![fts, json_ids(scope)?, limit], |row| {
                Ok((row.get::<_, i64>(0)?, -row.get::<_, f64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Listensuche: Besprechungen mit Treffern, bester Treffer zuerst, je
    /// Besprechung ein Snippet. Ohne Suchterme (leer/Satzzeichen) ist es eine
    /// reine Filterliste (neueste zuerst, ohne Snippet).
    ///
    /// Terme ab 3 Zeichen laufen ueber die Trigram-FTS (Teilwoerter, Komposita,
    /// Umlaut-/Grossschreibung egal), kuerzere ("KI") ueber die Wort-FTS; sind
    /// beide da, muessen beide passen.
    pub fn search_meetings(
        &self,
        query: &str,
        filter: &MeetingFilter,
        offset: u32,
        limit: u32,
    ) -> Result<MeetingSearchPage> {
        let terms = query_terms(query);
        let conn = self.get_connection()?;
        if terms.is_empty() {
            return Self::list_by_filter(&conn, filter, offset, limit);
        }
        let (long, short): (Vec<String>, Vec<String>) =
            terms.into_iter().partition(|t| t.chars().count() >= 3);
        let (table, primary, secondary) = if long.is_empty() {
            ("meeting_chunks_fts_words", and_of_terms(&short), None)
        } else {
            (
                "meeting_chunks_fts_tri",
                and_of_terms(&long),
                and_of_terms(&short),
            )
        };
        let Some(primary) = primary else {
            return Self::list_by_filter(&conn, filter, offset, limit);
        };

        let mut params: Vec<Value> = vec![Value::Text(primary.clone())];
        let constraints = meeting_constraints(
            filter.folder_id.as_deref(),
            filter.from,
            filter.to,
            filter.source.as_deref(),
            filter.has_notes == Some(true),
            &mut params,
        );
        let mut extra = String::new();
        if let Some(words) = &secondary {
            params.push(Value::Text(words.clone()));
            extra = format!(
                " AND c.id IN (SELECT rowid FROM meeting_chunks_fts_words
                              WHERE meeting_chunks_fts_words MATCH ?{})",
                params.len()
            );
        }
        params.push(Value::Integer(i64::from(SEARCH_CANDIDATES) + 1));
        let sql = format!(
            "SELECT c.id, c.meeting_id, c.source
             FROM {table}
             CROSS JOIN meeting_chunks c ON c.id = {table}.rowid
             JOIN meetings m ON m.id = c.meeting_id
             WHERE {table} MATCH ?1 AND m.deleted_at IS NULL{constraints}{extra}
             ORDER BY {table}.rank
             LIMIT ?{}",
            params.len()
        );
        let mut stmt = conn.prepare(&sql)?;
        let hits = stmt
            .query_map(params_from_iter(params.iter()), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let truncated = hits.len() > SEARCH_CANDIDATES as usize;

        // Bester Chunk je Besprechung, in Rangfolge.
        let mut seen: HashSet<&str> = HashSet::new();
        let mut best: Vec<(&str, i64, &str)> = Vec::new();
        for (chunk_id, meeting_id, source) in hits.iter().take(SEARCH_CANDIDATES as usize) {
            if seen.insert(meeting_id.as_str()) {
                best.push((meeting_id.as_str(), *chunk_id, source.as_str()));
            }
        }
        let total = best.len() as u32;
        let page: Vec<&(&str, i64, &str)> = best
            .iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();

        let page_ids: Vec<&str> = page.iter().map(|(m, _, _)| *m).collect();
        let mut meetings = Self::meetings_by_ids(&conn, &page_ids)?;
        // Trigram-Tokens sind einzelne Zeichen (Drei-Zeichen-Fenster): die 64 Tokens
        // des Maximums ergeben ~66 Zeichen, bei Woertern reichen weniger.
        let window = if table == "meeting_chunks_fts_tri" {
            64
        } else {
            24
        };
        let mut snippet_stmt = conn.prepare(&format!(
            "SELECT snippet({table}, 0, char(1), char(2), '…', {window}) FROM {table}
             WHERE {table} MATCH ?1 AND rowid = ?2"
        ))?;
        let mut items = Vec::with_capacity(page.len());
        for (meeting_id, chunk_id, source) in page {
            let Some(meeting) = meetings.remove(*meeting_id) else {
                continue;
            };
            let snippet: Option<String> = snippet_stmt
                .query_row(params![primary, chunk_id], |row| row.get(0))
                .optional()?;
            items.push(MeetingSearchItem {
                meeting,
                snippet: snippet.map(|s| html_escape_snippet(&s)),
                hit_source: ChunkSource::parse(source),
            });
        }
        Ok(MeetingSearchPage {
            items,
            total,
            truncated,
        })
    }

    fn list_by_filter(
        conn: &Connection,
        filter: &MeetingFilter,
        offset: u32,
        limit: u32,
    ) -> Result<MeetingSearchPage> {
        let mut params: Vec<Value> = Vec::new();
        let constraints = meeting_constraints(
            filter.folder_id.as_deref(),
            filter.from,
            filter.to,
            filter.source.as_deref(),
            filter.has_notes == Some(true),
            &mut params,
        );
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM meetings m WHERE m.deleted_at IS NULL{constraints}"),
            params_from_iter(params.iter()),
            |row| row.get(0),
        )?;
        params.push(Value::Integer(i64::from(limit)));
        let limit_ix = params.len();
        params.push(Value::Integer(i64::from(offset)));
        let offset_ix = params.len();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MEETING_COLUMNS} FROM meetings m WHERE m.deleted_at IS NULL{constraints}
             ORDER BY m.created_at DESC, m.id LIMIT ?{limit_ix} OFFSET ?{offset_ix}"
        ))?;
        let items = stmt
            .query_map(params_from_iter(params.iter()), Self::map_meeting)?
            .map(|m| {
                m.map(|meeting| MeetingSearchItem {
                    meeting,
                    snippet: None,
                    hit_source: None,
                })
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(MeetingSearchPage {
            items,
            total: total as u32,
            truncated: false,
        })
    }

    fn meetings_by_ids(conn: &Connection, ids: &[&str]) -> Result<HashMap<String, Meeting>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut stmt = conn.prepare(&format!(
            "SELECT {MEETING_COLUMNS} FROM meetings m
             WHERE m.deleted_at IS NULL AND m.id IN (SELECT value FROM json_each(?1))"
        ))?;
        let rows = stmt
            .query_map(params![json_ids(ids)?], Self::map_meeting)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().map(|m| (m.id.clone(), m)).collect())
    }

    /// Chunks zu den IDs, in der Reihenfolge von `ids`; unbekannte und solche
    /// geloeschter Besprechungen fehlen einfach.
    pub fn get_chunks(&self, ids: &[i64]) -> Result<Vec<ChunkRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT c.id, c.meeting_id, c.source, c.epoch, c.segment_ids, c.ref_keys,
                    c.document_id, c.start_ms, c.end_ms, c.channel, c.text, c.started_at
             FROM meeting_chunks c
             JOIN meetings m ON m.id = c.meeting_id AND m.deleted_at IS NULL
             WHERE c.id IN (SELECT value FROM json_each(?1))",
        )?;
        let mut by_id: HashMap<i64, ChunkRow> = stmt
            .query_map(params![json_ids(ids)?], map_chunk_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(|c| (c.id, c))
            .collect();
        Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }

    /// Besprechungs-IDs, auf die sich Chat und Suche beziehen: lebend UND
    /// fertig (`ready`), neueste zuerst. Alle gesetzten Felder gelten zugleich.
    /// `person` trifft auf Sprechernamen (`speakers.display_name`) oder auf
    /// Text in Titel, Notizen und Transkript (Trigram-Suche).
    pub fn resolve_scope(&self, scope: &ScopeFilter) -> Result<Vec<String>> {
        let conn = self.get_connection()?;
        let mut params: Vec<Value> = Vec::new();
        let mut sql = String::from(
            "SELECT m.id FROM meetings m WHERE m.deleted_at IS NULL AND m.status = 'ready'",
        );
        sql.push_str(&meeting_constraints(
            scope.folder_id.as_deref(),
            scope.from,
            scope.to,
            None,
            false,
            &mut params,
        ));
        if let Some(ids) = &scope.meeting_ids {
            params.push(Value::Text(json_ids(ids)?));
            sql.push_str(&format!(
                " AND m.id IN (SELECT value FROM json_each(?{}))",
                params.len()
            ));
        }
        sql.push_str(" ORDER BY COALESCE(m.started_at, m.created_at) DESC, m.id");
        let mut stmt = conn.prepare(&sql)?;
        let mut ids = stmt
            .query_map(params_from_iter(params.iter()), |row| {
                row.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if let Some(person) = scope
            .person
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            let matching = Self::meetings_mentioning(&conn, person)?;
            ids.retain(|id| matching.contains(id));
        }
        Ok(ids)
    }

    fn meetings_mentioning(conn: &Connection, person: &str) -> Result<HashSet<String>> {
        let mut found: HashSet<String> = HashSet::new();
        let needle = person.to_lowercase();
        let mut speakers = conn.prepare(
            "SELECT meeting_id, display_name FROM speakers
             WHERE deleted_at IS NULL AND display_name IS NOT NULL",
        )?;
        for row in speakers.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (meeting_id, name) = row?;
            if name.to_lowercase().contains(&needle) {
                found.insert(meeting_id);
            }
        }
        let (table, expr) = match fts_query_trigram(person) {
            Some(expr) => ("meeting_chunks_fts_tri", Some(expr)),
            None => (
                "meeting_chunks_fts_words",
                and_of_terms(&query_terms(person)),
            ),
        };
        if let Some(expr) = expr {
            let mut stmt = conn.prepare(&format!(
                "SELECT DISTINCT c.meeting_id FROM {table}
                 CROSS JOIN meeting_chunks c ON c.id = {table}.rowid
                 WHERE {table} MATCH ?1"
            ))?;
            for row in stmt.query_map(params![expr], |row| row.get::<_, String>(0))? {
                found.insert(row?);
            }
        }
        Ok(found)
    }

    // ---- Ordner ----------------------------------------------------------

    pub fn folders_list(&self) -> Result<Vec<Folder>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT f.id, f.name, f.color, f.sort,
                    (SELECT COUNT(*) FROM meeting_folder_items fi
                       JOIN meetings m ON m.id = fi.meeting_id AND m.deleted_at IS NULL
                      WHERE fi.folder_id = f.id),
                    f.created_at, f.updated_at
             FROM meeting_folders f WHERE f.deleted_at IS NULL
             ORDER BY f.sort, f.name COLLATE NOCASE, f.id",
        )?;
        let folders = stmt
            .query_map([], |row| {
                Ok(Folder {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                    sort: row.get(3)?,
                    meeting_count: row.get::<_, i64>(4)? as u32,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(folders)
    }

    /// Legt einen Ordner an (`id = None`) oder benennt/faerbt einen um. Der
    /// Name ist getrimmt 1 bis 60 Zeichen ohne Steuerzeichen und unter den
    /// lebenden Ordnern eindeutig (ohne Beachtung der Gross-/Kleinschreibung):
    /// `folder_name_invalid` / `folder_name_taken`; `folder_not_found` bei
    /// unbekannter oder geloeschter ID.
    pub fn folder_save(&self, id: Option<&str>, name: &str, color: Option<&str>) -> Result<Folder> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
            return Err(anyhow!("folder_name_invalid"));
        }
        let color = color.map(str::trim).filter(|c| !c.is_empty());
        if color.is_some_and(|c| c.chars().count() > 32 || c.chars().any(char::is_control)) {
            return Err(anyhow!("folder_color_invalid"));
        }
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;

        let wanted = name.to_lowercase();
        {
            let mut stmt =
                tx.prepare("SELECT id, name FROM meeting_folders WHERE deleted_at IS NULL")?;
            for row in stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })? {
                let (other_id, other_name) = row?;
                if Some(other_id.as_str()) != id && other_name.to_lowercase() == wanted {
                    return Err(anyhow!("folder_name_taken"));
                }
            }
        }
        let folder_id = match id {
            None => {
                let new_id = Ulid::new().to_string();
                tx.execute(
                    "INSERT INTO meeting_folders (id, name, color, sort, created_at, updated_at)
                     VALUES (?1, ?2, ?3,
                             (SELECT COALESCE(MAX(sort), 0) + 1 FROM meeting_folders), ?4, ?4)",
                    params![new_id, name, color, now],
                )?;
                new_id
            }
            Some(existing) => {
                let changed = tx.execute(
                    "UPDATE meeting_folders SET name = ?1, color = ?2, updated_at = ?3
                     WHERE id = ?4 AND deleted_at IS NULL",
                    params![name, color, now, existing],
                )?;
                if changed == 0 {
                    return Err(anyhow!("folder_not_found"));
                }
                existing.to_string()
            }
        };
        tx.commit()?;
        self.folders_list()?
            .into_iter()
            .find(|f| f.id == folder_id)
            .ok_or_else(|| anyhow!("folder_not_found"))
    }

    /// Loescht einen Ordner (weich). Die Zuordnungen entfallen, die
    /// Besprechungen selbst bleiben.
    pub fn folder_delete(&self, id: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let changed = tx.execute(
            "UPDATE meeting_folders SET deleted_at = ?1, updated_at = ?1
             WHERE id = ?2 AND deleted_at IS NULL",
            params![now, id],
        )?;
        if changed == 0 {
            return Err(anyhow!("folder_not_found"));
        }
        tx.execute(
            "DELETE FROM meeting_folder_items WHERE folder_id = ?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Setzt die Ordner einer Besprechung auf genau `folder_ids` (n:m).
    /// Bestehende Zuordnungen behalten ihren Zeitstempel. `folder_not_found`,
    /// wenn ein Ordner fehlt; dann bleibt alles wie es war.
    pub fn set_meeting_folders(&self, meeting_id: &str, folder_ids: &[String]) -> Result<()> {
        let now = Utc::now().timestamp();
        let mut wanted: Vec<&str> = Vec::new();
        for id in folder_ids {
            if !wanted.contains(&id.as_str()) {
                wanted.push(id);
            }
        }
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        Self::ensure_meeting_is_live(&tx, meeting_id)?;
        for folder_id in &wanted {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM meeting_folders WHERE id = ?1 AND deleted_at IS NULL)",
                params![folder_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(anyhow!("folder_not_found"));
            }
        }
        let current: Vec<String> = {
            let mut stmt =
                tx.prepare("SELECT folder_id FROM meeting_folder_items WHERE meeting_id = ?1")?;
            let rows = stmt
                .query_map(params![meeting_id], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        for folder_id in &current {
            if !wanted.contains(&folder_id.as_str()) {
                tx.execute(
                    "DELETE FROM meeting_folder_items WHERE folder_id = ?1 AND meeting_id = ?2",
                    params![folder_id, meeting_id],
                )?;
            }
        }
        for folder_id in wanted {
            if !current.iter().any(|c| c == folder_id) {
                tx.execute(
                    "INSERT INTO meeting_folder_items (folder_id, meeting_id, added_at)
                     VALUES (?1, ?2, ?3)",
                    params![folder_id, meeting_id, now],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Ordner-IDs einer Besprechung (lebende Ordner).
    pub fn meeting_folder_ids(&self, meeting_id: &str) -> Result<Vec<String>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT fi.folder_id FROM meeting_folder_items fi
             JOIN meeting_folders f ON f.id = fi.folder_id AND f.deleted_at IS NULL
             WHERE fi.meeting_id = ?1 ORDER BY f.sort, f.id",
        )?;
        let ids = stmt
            .query_map(params![meeting_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    // ---- Recipes ---------------------------------------------------------

    fn recipe_from(id: String, title: String, spec_json: String, updated_at: i64) -> RecipeInfo {
        RecipeInfo {
            builtin: id.starts_with(BUILTIN_PREFIX),
            id,
            title,
            spec_json,
            updated_at,
        }
    }

    fn validate_recipe(title: &str, spec_json: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 80 || title.chars().any(char::is_control) {
            return Err(anyhow!("recipe_invalid:title"));
        }
        if spec_json.len() > 32 * 1024 {
            return Err(anyhow!("recipe_invalid:spec_too_large"));
        }
        match serde_json::from_str::<serde_json::Value>(spec_json) {
            Ok(v) if v.is_object() => Ok(()),
            _ => Err(anyhow!("recipe_invalid:spec")),
        }
    }

    /// Mitgelieferte zuerst (in Einfuegereihenfolge), danach die eigenen.
    pub fn recipes_list(&self) -> Result<Vec<RecipeInfo>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, spec_json, updated_at FROM chat_recipes
             WHERE deleted_at IS NULL
             ORDER BY CASE WHEN id LIKE 'builtin:%' THEN 0 ELSE 1 END, rowid",
        )?;
        let recipes = stmt
            .query_map([], |row| {
                Ok(Self::recipe_from(
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(recipes)
    }

    pub fn recipe_get(&self, id: &str) -> Result<Option<RecipeInfo>> {
        let conn = self.get_connection()?;
        Ok(conn
            .query_row(
                "SELECT id, title, spec_json, updated_at FROM chat_recipes
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |row| {
                    Ok(Self::recipe_from(
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .optional()?)
    }

    /// Legt ein Recipe an (`id = None`) oder aendert ein eigenes. Mitgelieferte
    /// sind schreibgeschuetzt (`recipe_readonly`; die UI dupliziert stattdessen).
    /// `recipe_invalid:<grund>` bei ungueltigem Titel oder Spec-JSON (muss ein
    /// Objekt sein), `recipe_not_found` bei unbekannter ID. Die inhaltliche
    /// Pruefung der Spec (Variablen, Platzhalter) macht das Chat-Modul.
    pub fn recipe_save(
        &self,
        id: Option<&str>,
        title: &str,
        spec_json: &str,
    ) -> Result<RecipeInfo> {
        if id.is_some_and(|i| i.starts_with(BUILTIN_PREFIX)) {
            return Err(anyhow!("recipe_readonly"));
        }
        Self::validate_recipe(title, spec_json)?;
        let title = title.trim();
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let recipe_id = match id {
            None => {
                let new_id = Ulid::new().to_string();
                tx.execute(
                    "INSERT INTO chat_recipes (id, title, spec_json, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![new_id, title, spec_json, now],
                )?;
                new_id
            }
            Some(existing) => {
                let changed = tx.execute(
                    "UPDATE chat_recipes SET title = ?1, spec_json = ?2, updated_at = ?3
                     WHERE id = ?4 AND deleted_at IS NULL",
                    params![title, spec_json, now, existing],
                )?;
                if changed == 0 {
                    return Err(anyhow!("recipe_not_found"));
                }
                existing.to_string()
            }
        };
        tx.commit()?;
        Ok(Self::recipe_from(
            recipe_id,
            title.to_string(),
            spec_json.to_string(),
            now,
        ))
    }

    pub fn recipe_delete(&self, id: &str) -> Result<()> {
        if id.starts_with(BUILTIN_PREFIX) {
            return Err(anyhow!("recipe_readonly"));
        }
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let changed = tx.execute(
            "UPDATE chat_recipes SET deleted_at = ?1, updated_at = ?1
             WHERE id = ?2 AND deleted_at IS NULL",
            params![now, id],
        )?;
        if changed == 0 {
            return Err(anyhow!("recipe_not_found"));
        }
        tx.commit()?;
        Ok(())
    }

    /// Bringt die mitgelieferten Recipes (`(id, title, spec_json)`, IDs
    /// `builtin:<key>`) auf den Stand dieser App-Version; wie bei den Vorlagen
    /// idempotent: aktuelle Zeilen bleiben unberuehrt, veraltete, bearbeitete
    /// oder geloeschte werden wiederhergestellt. Eine Transaktion.
    pub fn recipes_seed_builtin(&self, items: &[(String, String, String)]) -> Result<()> {
        for (id, title, spec_json) in items {
            if !id.starts_with(BUILTIN_PREFIX) {
                return Err(anyhow!("recipe_invalid:id"));
            }
            Self::validate_recipe(title, spec_json)?;
        }
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        for (id, title, spec_json) in items {
            tx.execute(
                "INSERT INTO chat_recipes (id, title, spec_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                   title = excluded.title, spec_json = excluded.spec_json,
                   updated_at = excluded.updated_at, deleted_at = NULL
                 WHERE title IS NOT excluded.title
                    OR spec_json IS NOT excluded.spec_json
                    OR deleted_at IS NOT NULL",
                params![id, title.trim(), spec_json, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- Chat-Verlaeufe --------------------------------------------------

    fn map_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatThread> {
        Ok(ChatThread {
            id: row.get(0)?,
            scope_json: row.get(1)?,
            meeting_id: row.get(2)?,
            title: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
            message_count: row.get::<_, i64>(6)? as u32,
        })
    }

    const THREAD_COLUMNS: &'static str = "t.id, t.scope_json, t.meeting_id, t.title, t.created_at,
         t.updated_at, (SELECT COUNT(*) FROM chat_messages cm WHERE cm.thread_id = t.id)";

    /// Legt einen Verlauf an. `meeting_id` bindet ihn an eine (lebende)
    /// Besprechung: er geht mit ihr verloren. `scope_json` ist beliebiges JSON
    /// (das Chat-Modul legt dort den `ChatScope` ab).
    pub fn thread_create(
        &self,
        scope_json: &str,
        meeting_id: Option<&str>,
        title: Option<&str>,
    ) -> Result<ChatThread> {
        if scope_json.len() > 8 * 1024
            || serde_json::from_str::<serde_json::Value>(scope_json).is_err()
        {
            return Err(anyhow!("thread_invalid:scope"));
        }
        let title = title.map(str::trim).filter(|t| !t.is_empty());
        let id = Ulid::new().to_string();
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        if let Some(meeting_id) = meeting_id {
            Self::ensure_meeting_is_live(&tx, meeting_id)?;
        }
        tx.execute(
            "INSERT INTO chat_threads (id, scope_json, meeting_id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, scope_json, meeting_id, title, now],
        )?;
        tx.commit()?;
        Ok(ChatThread {
            id,
            scope_json: scope_json.to_string(),
            meeting_id: meeting_id.map(str::to_string),
            title: title.map(str::to_string),
            message_count: 0,
            created_at: now,
            updated_at: now,
        })
    }

    /// Haengt eine Nachricht an. `role` ist `user` oder `assistant`
    /// (`invalid_role`); hoechstens 64 KiB Text (`message_too_large`) und
    /// `MAX_THREAD_MESSAGES` je Verlauf (`thread_full`); `thread_not_found`
    /// bei unbekanntem oder geloeschtem Verlauf. Die erste Nutzerfrage wird
    /// zum Titel, solange keiner gesetzt ist.
    pub fn thread_append(
        &self,
        thread_id: &str,
        role: &str,
        content: &str,
        citations_json: Option<&str>,
        coverage_json: Option<&str>,
    ) -> Result<ChatMessageRow> {
        if role != "user" && role != "assistant" {
            return Err(anyhow!("invalid_role"));
        }
        if content.len() > MAX_MESSAGE_BYTES
            || citations_json.is_some_and(|c| c.len() > MAX_MESSAGE_BYTES)
            || coverage_json.is_some_and(|c| c.len() > MAX_MESSAGE_BYTES)
        {
            return Err(anyhow!("message_too_large"));
        }
        for json in [citations_json, coverage_json].into_iter().flatten() {
            if serde_json::from_str::<serde_json::Value>(json).is_err() {
                return Err(anyhow!("thread_invalid:json"));
            }
        }
        let id = Ulid::new().to_string();
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let title: Option<Option<String>> = tx
            .query_row(
                "SELECT title FROM chat_threads WHERE id = ?1 AND deleted_at IS NULL",
                params![thread_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(title) = title else {
            return Err(anyhow!("thread_not_found"));
        };
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM chat_messages WHERE thread_id = ?1",
            params![thread_id],
            |row| row.get(0),
        )?;
        if count >= MAX_THREAD_MESSAGES {
            return Err(anyhow!("thread_full"));
        }
        tx.execute(
            "INSERT INTO chat_messages
               (id, thread_id, role, content, citations_json, coverage_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                thread_id,
                role,
                content,
                citations_json,
                coverage_json,
                now
            ],
        )?;
        let new_title = if title.is_none() && role == "user" {
            let one_line = content.split_whitespace().collect::<Vec<_>>().join(" ");
            Some(one_line.chars().take(60).collect::<String>()).filter(|t| !t.is_empty())
        } else {
            None
        };
        tx.execute(
            "UPDATE chat_threads SET updated_at = ?1, title = COALESCE(title, ?2) WHERE id = ?3",
            params![now, new_title, thread_id],
        )?;
        tx.commit()?;
        Ok(ChatMessageRow {
            id,
            thread_id: thread_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            citations_json: citations_json.map(str::to_string),
            coverage_json: coverage_json.map(str::to_string),
            created_at: now,
        })
    }

    /// Verlaeufe eines Scopes, zuletzt benutzte zuerst.
    pub fn thread_list(&self, scope: &ThreadScope) -> Result<Vec<ChatThread>> {
        let conn = self.get_connection()?;
        let (filter, arg): (&str, Option<&str>) = match scope {
            ThreadScope::Meeting(id) => ("t.meeting_id = ?1", Some(id.as_str())),
            ThreadScope::Global => ("t.meeting_id IS NULL AND ?1 IS NULL", None),
        };
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM chat_threads t WHERE t.deleted_at IS NULL AND {filter}
             ORDER BY t.updated_at DESC, t.rowid DESC",
            Self::THREAD_COLUMNS
        ))?;
        let threads = stmt
            .query_map(params![arg], Self::map_thread)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(threads)
    }

    pub fn thread_get(&self, id: &str) -> Result<Option<(ChatThread, Vec<ChatMessageRow>)>> {
        let conn = self.get_connection()?;
        let thread = conn
            .query_row(
                &format!(
                    "SELECT {} FROM chat_threads t WHERE t.id = ?1 AND t.deleted_at IS NULL",
                    Self::THREAD_COLUMNS
                ),
                params![id],
                Self::map_thread,
            )
            .optional()?;
        let Some(thread) = thread else {
            return Ok(None);
        };
        let mut stmt = conn.prepare(
            "SELECT id, thread_id, role, content, citations_json, coverage_json, created_at
             FROM chat_messages WHERE thread_id = ?1 ORDER BY created_at, rowid",
        )?;
        let messages = stmt
            .query_map(params![id], |row| {
                Ok(ChatMessageRow {
                    id: row.get(0)?,
                    thread_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    citations_json: row.get(4)?,
                    coverage_json: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Some((thread, messages)))
    }

    /// Loescht einen Verlauf: die Nachrichten HART (der Text soll nicht
    /// ueberleben), die Zeile weich. `thread_not_found` bei unbekannter ID.
    pub fn thread_delete(&self, id: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let changed = tx.execute(
            "UPDATE chat_threads SET deleted_at = ?1, updated_at = ?1
             WHERE id = ?2 AND deleted_at IS NULL",
            params![now, id],
        )?;
        if changed == 0 {
            return Err(anyhow!("thread_not_found"));
        }
        tx.execute(
            "DELETE FROM chat_messages WHERE thread_id = ?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::managers::meetings::notes::model::{NoteBlock, NoteBlockKind};
    use crate::managers::meetings::store::{
        MeetingSource, MeetingStatus, StoredSegment, TranscriptDelta,
    };
    use std::sync::Barrier;

    pub(crate) fn tmp_store() -> (tempfile::TempDir, MeetingStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        (dir, s)
    }

    /// Fertige Besprechung mit Titel und Startzeit.
    pub(crate) fn ready_meeting(s: &MeetingStore, title: &str, started_at: i64) -> Meeting {
        let m = s
            .create_meeting(title, MeetingSource::Live, Some(1))
            .unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE meetings SET started_at = ?1 WHERE id = ?2",
                params![started_at, m.id],
            )
            .unwrap();
        s.set_status(&m.id, MeetingStatus::Ready).unwrap();
        m
    }

    pub(crate) fn draft(source: ChunkSource, text: &str) -> ChunkDraft {
        ChunkDraft {
            source,
            epoch: 0,
            segment_ids: vec![0],
            ref_keys: vec![],
            document_id: None,
            start_ms: Some(0),
            end_ms: Some(1_000),
            channel: Some(0),
            text: text.into(),
            embed_text: text.into(),
        }
    }

    pub(crate) fn state(status: &str) -> IndexState {
        IndexState {
            transcript_epoch: Some(0),
            transcript_rev: Some(0),
            notes_revision: Some(0),
            status: status.into(),
            ..IndexState::default()
        }
    }

    fn index_texts(s: &MeetingStore, meeting: &Meeting, texts: &[&str]) -> Vec<i64> {
        let drafts: Vec<ChunkDraft> = texts
            .iter()
            .map(|t| draft(ChunkSource::Transcript, t))
            .collect();
        s.replace_meeting_chunks(
            &meeting.id,
            &[ChunkSource::Transcript],
            &drafts,
            &state(STATUS_LEXICAL),
        )
        .unwrap()
    }

    fn scalar(s: &MeetingStore, sql: &str) -> i64 {
        s.get_connection()
            .unwrap()
            .query_row(sql, [], |r| r.get(0))
            .unwrap()
    }

    fn chunk_ids(s: &MeetingStore) -> Vec<i64> {
        let conn = s.get_connection().unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM meeting_chunks ORDER BY id")
            .unwrap();
        let ids = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        ids
    }

    fn search_ids(s: &MeetingStore, q: &str, scope: &[&Meeting]) -> Vec<i64> {
        let scope: Vec<String> = scope.iter().map(|m| m.id.clone()).collect();
        s.search_words(&fts_query_words_for_test(q), &scope, 50)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    fn fts_query_words_for_test(q: &str) -> String {
        super::super::chunking::fts_query_words(q).expect("suchbare Terme")
    }

    fn meeting_titles(page: &MeetingSearchPage) -> Vec<String> {
        page.items.iter().map(|i| i.meeting.title.clone()).collect()
    }

    // ---- Wort- und Trigram-Suche -----------------------------------------

    #[test]
    fn words_finds_two_letter_term() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "Strategie", 1_000);
        let ids = index_texts(
            &s,
            &m,
            &[
                "S0 00:00 Ich: Wir brauchen eine KI-Strategie fuer den Vertrieb.",
                "S1 00:09 Ich: Das Budget bleibt unveraendert.",
            ],
        );
        // Wort-FTS: das Zwei-Buchstaben-Wort "KI" wird gefunden ...
        assert_eq!(search_ids(&s, "KI", &[&m]), vec![ids[0]]);
        // ... die Trigram-FTS kennt nichts unter 3 Zeichen (Listensuche faellt auf Wort-FTS zurueck).
        assert_eq!(super::super::chunking::fts_query_trigram("KI"), None);
        let page = s
            .search_meetings("KI", &MeetingFilter::default(), 0, 25)
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].meeting.id, m.id);
        assert!(page.items[0]
            .snippet
            .as_deref()
            .unwrap()
            .contains("<mark>KI</mark>"));
    }

    #[test]
    fn trigram_finds_compound_part() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "Planung", 1_000);
        index_texts(
            &s,
            &m,
            &["S0 00:00 Ich: Das Marketingbudget fuer Q4 ist knapp und die Kundenbetreuung leidet."],
        );
        let conn = s.get_connection().unwrap();
        let count = |table: &str, expr: &str| -> i64 {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {table} MATCH ?1"),
                params![expr],
                |r| r.get(0),
            )
            .unwrap()
        };
        // Die Wort-FTS findet "budget" NICHT in "Marketingbudget", die Trigram-FTS schon.
        assert_eq!(count("meeting_chunks_fts_words", "\"budget\""), 0);
        assert_eq!(count("meeting_chunks_fts_tri", "\"budget\""), 1);
        drop(conn);
        // Und die Listensuche nutzt sie.
        for q in ["budget", "betreuung", "Marketing Budget", "MARKETINGBUDGET"] {
            let page = s
                .search_meetings(q, &MeetingFilter::default(), 0, 25)
                .unwrap();
            assert_eq!(page.total, 1, "Suche nach {q}");
        }
        let miss = s
            .search_meetings("Kundenbudget", &MeetingFilter::default(), 0, 25)
            .unwrap();
        assert_eq!(miss.total, 0);
    }

    #[test]
    fn trigram_folds_umlauts_and_case() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "Termin", 1_000);
        index_texts(
            &s,
            &m,
            &["S0 00:00 Ich: Das Gespräch verlief gut, Größe und Übergang passen."],
        );
        // "grosse" ist die ss-Schreibung von "Größe": ß faltet keiner der Tokenizer,
        // die ss/ß-Varianten aus Rust holen es trotzdem.
        for q in [
            "gesprach",
            "GESPRÄCH",
            "gespräch",
            "grosse",
            "Größe",
            "ubergang",
        ] {
            let page = s
                .search_meetings(q, &MeetingFilter::default(), 0, 25)
                .unwrap();
            assert_eq!(page.total, 1, "Suche nach {q}");
        }
        let miss = s
            .search_meetings("gesprachs", &MeetingFilter::default(), 0, 25)
            .unwrap();
        assert_eq!(miss.total, 0);
        // Die Wort-FTS faltet Umlaute ebenfalls.
        let m2 = ready_meeting(&s, "Zweite", 2_000);
        let ids = index_texts(&s, &m2, &["Wir sprechen über Übergänge und Ärger."]);
        assert_eq!(search_ids(&s, "ubergange", &[&m2]), ids);
    }

    #[test]
    fn ss_and_sharp_s_find_each_other_in_both_indexes() {
        let (_d, s) = tmp_store();
        let a = ready_meeting(&s, "A", 1_000);
        let b = ready_meeting(&s, "B", 2_000);
        let ids_a = index_texts(&s, &a, &["Die Straße ist gesperrt."]);
        let ids_b = index_texts(&s, &b, &["Die Strasse ist frei."]);
        for q in ["Straße", "Strasse", "STRASSE"] {
            assert_eq!(
                search_ids(&s, q, &[&a, &b]).len(),
                2,
                "Wortsuche {q} findet beide Schreibweisen"
            );
            let page = s
                .search_meetings(q, &MeetingFilter::default(), 0, 25)
                .unwrap();
            assert_eq!(page.total, 2, "Listensuche {q} findet beide Schreibweisen");
        }
        assert_eq!(search_ids(&s, "Straße", &[&a]), vec![ids_a[0]]);
        assert_eq!(search_ids(&s, "Strasse", &[&b]), vec![ids_b[0]]);
    }

    #[test]
    fn word_search_respects_scope_and_ranks_by_relevance() {
        let (_d, s) = tmp_store();
        let a = ready_meeting(&s, "A", 1_000);
        let b = ready_meeting(&s, "B", 2_000);
        let ia = index_texts(
            &s,
            &a,
            &[
                "Budget Budget Budget genehmigt",
                "Wetter heute",
                "Anderes Thema",
                "Noch etwas ganz anderes",
                "Urlaubsplanung im Sommer",
            ],
        );
        let ib = index_texts(&s, &b, &["Das Budget wurde erwaehnt"]);

        let both = search_ids(&s, "Budget", &[&a, &b]);
        assert_eq!(both.len(), 2);
        assert_eq!(both[0], ia[0], "haeufigeres Vorkommen zuerst");
        assert_eq!(search_ids(&s, "Budget", &[&b]), vec![ib[0]]);
        // Leerer Scope heisst: nichts im Scope.
        assert!(s
            .search_words(&fts_query_words_for_test("Budget"), &[], 10)
            .unwrap()
            .is_empty());
        assert!(s
            .search_words(&fts_query_words_for_test("Budget"), &[a.id.clone()], 0)
            .unwrap()
            .is_empty());
        // Relevanz: groesser = besser.
        let scored = s
            .search_words(
                &fts_query_words_for_test("Budget"),
                &[a.id.clone(), b.id.clone()],
                10,
            )
            .unwrap();
        assert!(scored[0].1 >= scored[1].1);
        // Geloeschte Besprechung liefert nichts mehr.
        s.soft_delete_meeting(&a.id).unwrap();
        assert_eq!(search_ids(&s, "Budget", &[&a, &b]), vec![ib[0]]);
    }

    #[test]
    fn list_search_groups_by_meeting_pages_and_escapes_the_snippet() {
        let (_d, s) = tmp_store();
        let mut meetings = Vec::new();
        for i in 0..5 {
            let m = ready_meeting(&s, &format!("Besprechung {i}"), 1_000 + i);
            index_texts(&s, &m, &["Angebot <b>fett</b> & Co", "Zweites Thema"]);
            meetings.push(m);
        }
        let all = s
            .search_meetings("Angebot", &MeetingFilter::default(), 0, 25)
            .unwrap();
        assert_eq!(all.total, 5, "je Besprechung genau einmal");
        assert!(!all.truncated);
        let mut ids: Vec<&str> = all.items.iter().map(|i| i.meeting.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 5);

        let page2 = s
            .search_meetings("Angebot", &MeetingFilter::default(), 2, 2)
            .unwrap();
        assert_eq!(page2.total, 5);
        assert_eq!(page2.items.len(), 2);
        let page3 = s
            .search_meetings("Angebot", &MeetingFilter::default(), 4, 2)
            .unwrap();
        assert_eq!(page3.items.len(), 1);

        for item in &all.items {
            let snip = item.snippet.as_deref().unwrap();
            assert!(snip.contains("<mark>"), "{snip}");
            assert!(
                snip.contains("&lt;b&gt;fett"),
                "HTML muss maskiert sein: {snip}"
            );
            assert!(snip.contains("&amp; Co"), "{snip}");
            // Ausser den Markierungen kein rohes Tag.
            let without_marks = snip.replace("<mark>", "").replace("</mark>", "");
            assert!(
                !without_marks.contains('<') && !without_marks.contains('>'),
                "{snip}"
            );
            assert_eq!(item.hit_source, Some(ChunkSource::Transcript));
        }
    }

    #[test]
    fn list_search_filters_by_folder_date_source_and_notes() {
        let (_d, s) = tmp_store();
        let a = ready_meeting(&s, "Alt mit Ordner", 1_000);
        let b = ready_meeting(&s, "Neu ohne Ordner", 9_000);
        for m in [&a, &b] {
            index_texts(&s, m, &["Das Angebot liegt vor."]);
        }
        let folder = s.folder_save(None, "Vertrieb", None).unwrap();
        s.set_meeting_folders(&a.id, &[folder.id.clone()]).unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE meetings SET source = 'import' WHERE id = ?1",
                params![b.id],
            )
            .unwrap();
        s.save_notes(
            &b.id,
            &[NoteBlock {
                id: "b1".into(),
                kind: NoteBlockKind::Paragraph,
                text: "Notiz".into(),
                at_ms: None,
                checked: false,
            }],
            0,
        )
        .unwrap();

        let run =
            |f: MeetingFilter| meeting_titles(&s.search_meetings("Angebot", &f, 0, 25).unwrap());
        assert_eq!(
            run(MeetingFilter {
                folder_id: Some(folder.id.clone()),
                ..Default::default()
            }),
            vec!["Alt mit Ordner"]
        );
        assert_eq!(
            run(MeetingFilter {
                from: Some(5_000),
                ..Default::default()
            }),
            vec!["Neu ohne Ordner"]
        );
        assert_eq!(
            run(MeetingFilter {
                to: Some(5_000),
                ..Default::default()
            }),
            vec!["Alt mit Ordner"]
        );
        assert_eq!(
            run(MeetingFilter {
                source: Some("import".into()),
                ..Default::default()
            }),
            vec!["Neu ohne Ordner"]
        );
        assert_eq!(
            run(MeetingFilter {
                has_notes: Some(true),
                ..Default::default()
            }),
            vec!["Neu ohne Ordner"]
        );
        // Ohne Suchtext: reine Filterliste, neueste zuerst, ohne Snippet.
        let listing = s
            .search_meetings("", &MeetingFilter::default(), 0, 25)
            .unwrap();
        assert_eq!(listing.total, 2);
        assert!(listing.items.iter().all(|i| i.snippet.is_none()));
        let only_folder = s
            .search_meetings(
                "  ?! ",
                &MeetingFilter {
                    folder_id: Some(folder.id),
                    ..Default::default()
                },
                0,
                25,
            )
            .unwrap();
        assert_eq!(meeting_titles(&only_folder), vec!["Alt mit Ordner"]);
    }

    #[test]
    fn a_short_and_a_long_term_must_both_match() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        index_texts(
            &s,
            &m,
            &[
                "KI Strategie fuer 2027",
                "Strategie ohne das Zwei-Buchstaben-Wort",
            ],
        );
        let page = s
            .search_meetings("KI Strategie", &MeetingFilter::default(), 0, 25)
            .unwrap();
        assert_eq!(page.total, 1);
        let snippet = page.items[0].snippet.as_deref().unwrap();
        assert!(snippet.contains("KI"), "{snippet}");
    }

    // ---- Chunks, Zustand, Atomaritaet -------------------------------------

    #[test]
    fn replace_swaps_only_the_named_sources_and_returns_ids_in_order() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        let first = s
            .replace_meeting_chunks(
                &m.id,
                &[ChunkSource::Transcript, ChunkSource::UserNotes],
                &[
                    draft(ChunkSource::Transcript, "alt eins"),
                    draft(ChunkSource::UserNotes, "notiz"),
                    draft(ChunkSource::Transcript, "alt zwei"),
                ],
                &state(STATUS_LEXICAL),
            )
            .unwrap();
        assert_eq!(first.len(), 3);
        assert!(first.windows(2).all(|w| w[0] < w[1]));

        let second = s
            .replace_meeting_chunks(
                &m.id,
                &[ChunkSource::Transcript],
                &[draft(ChunkSource::Transcript, "neu")],
                &state(STATUS_LEXICAL),
            )
            .unwrap();
        let rows = s.get_chunks(&[first[1], second[0], first[0]]).unwrap();
        assert_eq!(rows.len(), 2, "alte Transkript-Chunks sind weg");
        assert_eq!(rows[0].id, first[1]);
        assert_eq!(rows[0].source, ChunkSource::UserNotes, "Notizen blieben");
        assert_eq!(rows[1].id, second[0]);
        assert_eq!(rows[1].text, "neu");
        assert!(
            second[0] > first[2],
            "IDs werden nie wiederverwendet (AUTOINCREMENT)"
        );
        assert!(
            search_ids(&s, "alt", &[&m]).is_empty(),
            "FTS folgt dem Loeschen"
        );
    }

    #[test]
    fn stored_chunk_fields_round_trip() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 5_000);
        let mut d = draft(
            ChunkSource::AiNotes,
            "## Entscheidungen\n- Start im Oktober",
        );
        d.epoch = 3;
        d.segment_ids = vec![2, 4, 9];
        d.ref_keys = vec!["E1".into(), "E2".into()];
        d.document_id = Some("DOC".into());
        d.start_ms = None;
        d.end_ms = None;
        d.channel = None;
        let doc = s
            .insert_document(&m.id, "enhanced_notes", "enhanced@1", "{}", None, None)
            .unwrap();
        d.document_id = Some(doc.clone());
        let mut st = state(STATUS_LEXICAL);
        st.enhanced_doc_id = Some(doc.clone());
        let ids = s
            .replace_meeting_chunks(&m.id, &[ChunkSource::AiNotes], &[d], &st)
            .unwrap();
        let row = &s.get_chunks(&ids).unwrap()[0];
        assert_eq!(row.epoch, 3);
        assert_eq!(row.segment_ids, vec![2, 4, 9]);
        assert_eq!(row.ref_keys, vec!["E1", "E2"]);
        assert_eq!(row.document_id.as_deref(), Some(doc.as_str()));
        assert_eq!((row.start_ms, row.end_ms, row.channel), (None, None, None));
        assert_eq!(row.started_at, Some(5_000), "Startzeit aus der Besprechung");
    }

    #[test]
    fn replace_validates_before_it_writes_anything() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        index_texts(&s, &m, &["bleibt"]);
        let err = |r: Result<Vec<i64>>| r.unwrap_err().to_string();

        assert_eq!(
            err(s.replace_meeting_chunks(
                &m.id,
                &[ChunkSource::Transcript],
                &[draft(ChunkSource::UserNotes, "falsche Quelle")],
                &state(STATUS_LEXICAL),
            )),
            "chunk_source_not_replaced"
        );
        assert_eq!(
            err(s.replace_meeting_chunks(
                &m.id,
                &[ChunkSource::Transcript],
                &[draft(ChunkSource::Transcript, &"x".repeat(70_000))],
                &state(STATUS_LEXICAL),
            )),
            "chunk_too_large"
        );
        assert_eq!(
            err(s.replace_meeting_chunks(&m.id, &[ChunkSource::Transcript], &[], &state("kaputt"),)),
            "invalid_status"
        );
        assert!(err(s.replace_meeting_chunks(
            "gibt-es-nicht",
            &[ChunkSource::Transcript],
            &[],
            &state(STATUS_LEXICAL),
        ))
        .contains("not found"));
        s.soft_delete_meeting(&m.id).unwrap();
        assert!(err(s.replace_meeting_chunks(
            &m.id,
            &[ChunkSource::Transcript],
            &[draft(ChunkSource::Transcript, "zu spaet")],
            &state(STATUS_LEXICAL),
        ))
        .contains("not found"));
        assert_eq!(scalar(&s, "SELECT COUNT(*) FROM meeting_chunks"), 0);
        // Leere Entwurfstexte entfallen.
        let m2 = ready_meeting(&s, "M2", 1_000);
        let ids = s
            .replace_meeting_chunks(
                &m2.id,
                &[ChunkSource::Transcript],
                &[
                    draft(ChunkSource::Transcript, "  "),
                    draft(ChunkSource::Transcript, "echt"),
                ],
                &state(STATUS_LEXICAL),
            )
            .unwrap();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn a_stale_indexer_writes_nothing() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        s.append_delta(
            &m.id,
            &TranscriptDelta {
                new_segments: vec![StoredSegment {
                    segment_index: 0,
                    text: "Alt.".into(),
                    start_ms: 0,
                    end_ms: 900,
                    channel: 0,
                    speaker_index: None,
                    words: None,
                }],
            },
        )
        .unwrap();
        let mut fresh = state(STATUS_LEXICAL);
        fresh.transcript_rev = Some(1);
        s.replace_meeting_chunks(
            &m.id,
            &[ChunkSource::Transcript],
            &[draft(ChunkSource::Transcript, "bleibt stehen")],
            &fresh,
        )
        .unwrap();
        // Der Indexer las bei Epoche 0 / Revision 0 ...
        let stale_rev = state(STATUS_LEXICAL);
        // ... aber der Transkript-Inhalt hat inzwischen Revision 1 (append_delta).
        let err = s
            .replace_meeting_chunks(
                &m.id,
                &[ChunkSource::Transcript],
                &[draft(ChunkSource::Transcript, "veraltet")],
                &stale_rev,
            )
            .unwrap_err();
        assert_eq!(err.to_string(), "stale_transcript");

        // Neu-Transkription: Epoche 1.
        s.clear_segments(&m.id).unwrap();
        let err = s
            .replace_meeting_chunks(
                &m.id,
                &[ChunkSource::Transcript],
                &[draft(ChunkSource::Transcript, "veraltet")],
                &fresh,
            )
            .unwrap_err();
        assert_eq!(err.to_string(), "stale_epoch");
        assert_eq!(
            scalar(&s, "SELECT COUNT(*) FROM meeting_chunks"),
            1,
            "alter Stand unveraendert"
        );

        // Nur die Quellen, die ersetzt werden, sind geschuetzt.
        s.replace_meeting_chunks(
            &m.id,
            &[ChunkSource::Title],
            &[draft(ChunkSource::Title, "M")],
            &stale_rev,
        )
        .unwrap();

        // Notizen und KI-Notizen.
        s.save_notes(
            &m.id,
            &[NoteBlock {
                id: "b".into(),
                kind: NoteBlockKind::Paragraph,
                text: "x".into(),
                at_ms: None,
                checked: false,
            }],
            0,
        )
        .unwrap();
        let err = s
            .replace_meeting_chunks(&m.id, &[ChunkSource::UserNotes], &[], &stale_rev)
            .unwrap_err();
        assert_eq!(err.to_string(), "stale_notes");
        s.insert_document(&m.id, "enhanced_notes", "enhanced@1", "{}", None, None)
            .unwrap();
        let err = s
            .replace_meeting_chunks(&m.id, &[ChunkSource::AiNotes], &[], &stale_rev)
            .unwrap_err();
        assert_eq!(err.to_string(), "stale_document");
    }

    /// Voller Datentraeger/Abbruch mitten im Schreiben: ein Trigger, der beim
    /// zweiten Einfuegen scheitert, steht fuer SQLITE_FULL. Es bleibt der alte
    /// Stand: Chunks, FTS und Zustand.
    #[test]
    fn replace_is_all_or_nothing_when_a_write_fails_midway() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        index_texts(&s, &m, &["alt eins", "alt zwei"]);
        let old_state = s.index_state(&m.id).unwrap().unwrap();
        let old_ids = chunk_ids(&s);

        s.get_connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_insert BEFORE INSERT ON meeting_chunks
                 WHEN new.text = 'kaputt' BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
            )
            .unwrap();
        let mut new_state = state(STATUS_READY);
        new_state.title = Some("anders".into());
        assert!(s
            .replace_meeting_chunks(
                &m.id,
                &[ChunkSource::Transcript],
                &[
                    draft(ChunkSource::Transcript, "neu eins"),
                    draft(ChunkSource::Transcript, "kaputt"),
                ],
                &new_state,
            )
            .is_err());
        s.get_connection()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_insert;")
            .unwrap();

        assert_eq!(chunk_ids(&s), old_ids, "alte Chunks unveraendert");
        assert_eq!(s.index_state(&m.id).unwrap().unwrap(), old_state);
        assert_eq!(search_ids(&s, "alt", &[&m]).len(), 2, "FTS ebenfalls");
        assert!(search_ids(&s, "neu", &[&m]).is_empty());
        assert!(s.search_index_is_consistent().unwrap());
        // Danach laeuft der Store ohne Neustart weiter.
        index_texts(&s, &m, &["neu drei"]);
        assert_eq!(search_ids(&s, "drei", &[&m]).len(), 1);
    }

    #[test]
    fn fts_stays_consistent_with_the_content_table_through_replace_update_and_delete() {
        let (_d, s) = tmp_store();
        let a = ready_meeting(&s, "A", 1_000);
        let b = ready_meeting(&s, "B", 2_000);
        for round in 0..4 {
            index_texts(
                &s,
                &a,
                &[&format!("Runde {round} Alpha Größe"), "Beta Ärger"],
            );
            index_texts(&s, &b, &[&format!("Runde {round} Gamma")]);
            assert!(s.search_index_is_consistent().unwrap(), "Runde {round}");
        }
        let conn = s.get_connection().unwrap();
        conn.execute(
            "UPDATE meeting_chunks SET text = 'Umgeschrieben Delta' WHERE meeting_id = ?1",
            params![a.id],
        )
        .unwrap();
        drop(conn);
        assert!(s.search_index_is_consistent().unwrap());
        assert_eq!(search_ids(&s, "Delta", &[&a]).len(), 2);
        assert!(
            search_ids(&s, "Alpha", &[&a]).is_empty(),
            "UPDATE-Trigger raeumt den alten Text"
        );
        s.soft_delete_meeting(&a.id).unwrap();
        assert!(s.search_index_is_consistent().unwrap());
        assert_eq!(
            scalar(&s, "SELECT COUNT(*) FROM meeting_chunks_fts_tri_docsize"),
            1
        );
        assert_eq!(
            scalar(&s, "SELECT COUNT(*) FROM meeting_chunks_fts_words_docsize"),
            1
        );
    }

    #[test]
    fn an_update_of_the_text_drops_the_stale_vector() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        let ids = index_texts(&s, &m, &["Text eins", "Text zwei"]);
        s.put_vectors("mdl", &[(ids[0], vec![1.0, 0.0]), (ids[1], vec![0.0, 1.0])])
            .unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE meeting_chunks SET text = 'Text neu' WHERE id = ?1",
                params![ids[0]],
            )
            .unwrap();
        assert_eq!(scalar(&s, "SELECT COUNT(*) FROM meeting_chunk_vectors"), 1);
        assert_eq!(
            s.chunks_without_vectors("mdl", 10).unwrap().len(),
            1,
            "der geaenderte Chunk braucht einen neuen Vektor"
        );
    }

    #[test]
    fn soft_delete_purges_chunks_fts_vectors() {
        let (_d, s) = tmp_store();
        let gone = ready_meeting(&s, "Wird geloescht", 1_000);
        let keep = ready_meeting(&s, "Bleibt", 2_000);
        let gone_ids = index_texts(
            &s,
            &gone,
            &["Geheimprojekt Alpha", "Zweiter Chunk Geheimprojekt"],
        );
        let keep_ids = index_texts(&s, &keep, &["Geheimprojekt bleibt"]);
        s.put_vectors(
            "mdl",
            &[
                (gone_ids[0], vec![1.0, 0.0]),
                (gone_ids[1], vec![0.0, 1.0]),
                (keep_ids[0], vec![1.0, 1.0]),
            ],
        )
        .unwrap();
        let folder = s.folder_save(None, "Vertrieb", None).unwrap();
        s.set_meeting_folders(&gone.id, &[folder.id.clone()])
            .unwrap();
        s.set_meeting_folders(&keep.id, &[folder.id.clone()])
            .unwrap();
        let own_thread = s.thread_create("{}", Some(&gone.id), None).unwrap();
        s.thread_append(
            &own_thread.id,
            "user",
            "Frage zu dieser Besprechung",
            None,
            None,
        )
        .unwrap();
        let global_thread = s.thread_create("{\"global\":true}", None, None).unwrap();
        s.thread_append(&global_thread.id, "user", "Globale Frage", None, None)
            .unwrap();
        let keep_thread = s.thread_create("{}", Some(&keep.id), None).unwrap();
        assert_eq!(scalar(&s, "SELECT COUNT(*) FROM meeting_chunk_vectors"), 3);

        s.soft_delete_meeting(&gone.id).unwrap();

        // Chunks, Vektoren, Zustand, Zuordnung, Verlaeufe: weg.
        let of_gone = |table: &str, col: &str| {
            scalar(
                &s,
                &format!("SELECT COUNT(*) FROM {table} WHERE {col} = '{}'", gone.id),
            )
        };
        assert_eq!(of_gone("meeting_chunks", "meeting_id"), 0);
        assert_eq!(of_gone("meeting_index_state", "meeting_id"), 0);
        assert_eq!(of_gone("meeting_folder_items", "meeting_id"), 0);
        assert_eq!(of_gone("chat_threads", "meeting_id"), 0);
        assert_eq!(
            scalar(&s, "SELECT COUNT(*) FROM meeting_chunk_vectors"),
            1,
            "nur der Vektor der anderen Besprechung"
        );
        assert_eq!(
            scalar(
                &s,
                &format!(
                    "SELECT COUNT(*) FROM chat_messages WHERE thread_id = '{}'",
                    own_thread.id
                )
            ),
            0
        );
        // FTS: nichts mehr auffindbar (beide Indizes), FTS konsistent.
        let hits = |table: &str| -> i64 {
            s.get_connection()
                .unwrap()
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} WHERE {table} MATCH '\"geheimprojekt\"'"
                    ),
                    [],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(
            hits("meeting_chunks_fts_words"),
            1,
            "nur der Chunk der anderen"
        );
        assert_eq!(hits("meeting_chunks_fts_tri"), 1);
        assert!(s.search_index_is_consistent().unwrap());
        let page = s
            .search_meetings("Geheimprojekt", &MeetingFilter::default(), 0, 25)
            .unwrap();
        assert_eq!(meeting_titles(&page), vec!["Bleibt"]);
        // Fremdes bleibt: Ordner (mit korrekter Zaehlung), globaler Verlauf, andere Besprechung.
        let folders = s.folders_list().unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].meeting_count, 1);
        let (_, msgs) = s.thread_get(&global_thread.id).unwrap().unwrap();
        assert_eq!(msgs.len(), 1, "globaler Verlauf behaelt seinen Text");
        assert!(s.thread_get(&own_thread.id).unwrap().is_none());
        assert!(s.thread_get(&keep_thread.id).unwrap().is_some());
        assert_eq!(s.get_chunks(&keep_ids).unwrap().len(), 1);
        assert_eq!(s.get_chunks(&gone_ids).unwrap().len(), 0);
    }

    #[test]
    fn soft_delete_stays_all_or_nothing_with_the_index_in_play() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        let ids = index_texts(&s, &m, &["Text"]);
        s.put_vectors("mdl", &[(ids[0], vec![1.0, 0.0])]).unwrap();
        let t = s.thread_create("{}", Some(&m.id), None).unwrap();
        s.get_connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_threads BEFORE DELETE ON chat_threads
                 BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;",
            )
            .unwrap();
        assert!(s.soft_delete_meeting(&m.id).is_err());
        s.get_connection()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_threads;")
            .unwrap();
        assert!(
            s.get_meeting(&m.id).unwrap().is_some(),
            "nicht halb geloescht"
        );
        assert_eq!(s.get_chunks(&ids).unwrap().len(), 1);
        assert_eq!(scalar(&s, "SELECT COUNT(*) FROM meeting_chunk_vectors"), 1);
        assert!(s.index_state(&m.id).unwrap().is_some());
        assert!(s.thread_get(&t.id).unwrap().is_some());
        assert!(s.search_index_is_consistent().unwrap());
    }

    // ---- Vektoren ---------------------------------------------------------

    #[test]
    fn put_vectors_normalizes_and_skips_chunks_that_vanished() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        let ids = index_texts(&s, &m, &["eins", "zwei"]);
        // Der Indexer holt (id, Text), embeddet ... und waehrenddessen wird neu indexiert.
        index_texts(&s, &m, &["ganz neu"]);
        let stored = s
            .put_vectors("mdl", &[(ids[0], vec![3.0, 4.0]), (ids[1], vec![1.0, 0.0])])
            .unwrap();
        assert_eq!(
            stored, 0,
            "kein Vektor fuer verschwundene Chunks (keine Rowid-Wiederverwendung)"
        );
        assert_eq!(scalar(&s, "SELECT COUNT(*) FROM meeting_chunk_vectors"), 0);

        let new_ids = chunk_ids(&s);
        assert_eq!(
            s.put_vectors("mdl", &[(new_ids[0], vec![3.0, 4.0])])
                .unwrap(),
            1
        );
        let blob: Vec<u8> = s
            .get_connection()
            .unwrap()
            .query_row("SELECT vec FROM meeting_chunk_vectors", [], |r| r.get(0))
            .unwrap();
        let v: Vec<f32> = blob
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert!(
            (v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6,
            "L2-normalisiert: {v:?}"
        );
    }

    #[test]
    fn put_vectors_rejects_broken_input_and_writes_nothing() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        let ids = index_texts(&s, &m, &["eins", "zwei"]);
        let code = |rows: &[(i64, Vec<f32>)], model: &str| {
            s.put_vectors(model, rows).unwrap_err().to_string()
        };
        assert_eq!(
            code(&[(ids[0], vec![f32::NAN, 1.0])], "mdl"),
            "invalid_vector"
        );
        assert_eq!(
            code(&[(ids[0], vec![f32::INFINITY, 1.0])], "mdl"),
            "invalid_vector"
        );
        assert_eq!(code(&[(ids[0], vec![0.0, 0.0])], "mdl"), "invalid_vector");
        assert_eq!(code(&[(ids[0], vec![])], "mdl"), "invalid_vector");
        assert_eq!(
            code(&[(ids[0], vec![1.0, 0.0]), (ids[1], vec![1.0])], "mdl"),
            "invalid_vector",
            "unterschiedliche Laengen in einem Aufruf"
        );
        assert_eq!(code(&[(ids[0], vec![1.0, 0.0])], " "), "invalid_vector");
        assert_eq!(scalar(&s, "SELECT COUNT(*) FROM meeting_chunk_vectors"), 0);

        s.put_vectors("mdl", &[(ids[0], vec![1.0, 0.0])]).unwrap();
        assert_eq!(
            code(&[(ids[1], vec![1.0, 0.0, 0.0])], "mdl"),
            "vector_dim_mismatch"
        );
        // Anderes Modell darf eine andere Laenge haben.
        s.put_vectors("anderes", &[(ids[1], vec![1.0, 0.0, 0.0])])
            .unwrap();
    }

    #[test]
    fn chunks_without_vectors_returns_embedding_text_for_the_current_meeting() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "Kundengespräch", 1_789_214_400);
        let folder = s.folder_save(None, "Vertrieb", None).unwrap();
        s.set_meeting_folders(&m.id, &[folder.id]).unwrap();
        let ids = s
            .replace_meeting_chunks(
                &m.id,
                &[ChunkSource::Title, ChunkSource::Transcript],
                &[
                    draft(ChunkSource::Title, "Kundengespräch"),
                    draft(ChunkSource::Transcript, "S0 00:00 Ich: Hallo."),
                ],
                &state(STATUS_LEXICAL),
            )
            .unwrap();
        let todo = s.chunks_without_vectors("mdl", 10).unwrap();
        assert_eq!(todo.len(), 2);
        assert_eq!(todo[0].0, ids[0]);
        assert_eq!(
            todo[0].1,
            "Besprechung: Kundengespräch, 12.09.2026\nOrdner: Vertrieb"
        );
        assert_eq!(
            todo[1].1,
            "Besprechung: Kundengespräch, 12.09.2026\nS0 00:00 Ich: Hallo."
        );
        // Nach dem Umbenennen zieht der Text den neuen Titel.
        s.set_title(&m.id, "Neuer Titel").unwrap();
        assert!(s.chunks_without_vectors("mdl", 10).unwrap()[1]
            .1
            .starts_with("Besprechung: Neuer Titel"));

        s.put_vectors("mdl", &[(ids[0], vec![1.0, 0.0])]).unwrap();
        assert_eq!(s.chunks_without_vectors("mdl", 10).unwrap().len(), 1);
        assert_eq!(s.chunks_without_vectors("mdl", 0).unwrap().len(), 0);
        assert_eq!(
            s.chunks_without_vectors("anderes-modell", 10)
                .unwrap()
                .len(),
            2,
            "Vektoren eines anderen Modells zaehlen nicht"
        );
    }

    // ---- Zustand, veraltete Besprechungen --------------------------------

    #[test]
    fn stale_meetings_reports_exactly_what_changed_since_indexing() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "Titel", 1_000);
        let unfinished = s
            .create_meeting("Laeuft noch", MeetingSource::Live, Some(1))
            .unwrap();
        assert_eq!(
            s.stale_meetings(10).unwrap(),
            vec![m.id.clone()],
            "noch nie indexiert; nur fertige Besprechungen zaehlen"
        );

        let mut st = state(STATUS_LEXICAL);
        st.title = Some("Titel".into());
        s.replace_meeting_chunks(
            &m.id,
            &[ChunkSource::Title],
            &[draft(ChunkSource::Title, "Titel")],
            &st,
        )
        .unwrap();
        assert!(s.stale_meetings(10).unwrap().is_empty(), "aktuell");

        // Umbenennen.
        s.set_title(&m.id, "Neu").unwrap();
        assert_eq!(s.stale_meetings(10).unwrap(), vec![m.id.clone()]);
        st.title = Some("Neu".into());
        s.replace_meeting_chunks(&m.id, &[ChunkSource::Title], &[], &st)
            .unwrap();
        assert!(s.stale_meetings(10).unwrap().is_empty());

        // Notizen.
        let block = |t: &str| NoteBlock {
            id: "b".into(),
            kind: NoteBlockKind::Paragraph,
            text: t.into(),
            at_ms: None,
            checked: false,
        };
        s.save_notes(&m.id, &[block("x")], 0).unwrap();
        assert_eq!(s.stale_meetings(10).unwrap().len(), 1);
        st.notes_revision = Some(1);
        s.replace_meeting_chunks(&m.id, &[ChunkSource::UserNotes], &[], &st)
            .unwrap();
        assert!(s.stale_meetings(10).unwrap().is_empty());

        // KI-Notizen: neue Version, dann Bearbeitung derselben Version.
        let doc = s
            .insert_document(&m.id, "enhanced_notes", "enhanced@1", "{}", None, None)
            .unwrap();
        assert_eq!(s.stale_meetings(10).unwrap().len(), 1);
        let stamp = s.get_document(&doc).unwrap().unwrap().updated_at;
        st.enhanced_doc_id = Some(doc.clone());
        st.enhanced_updated_at = Some(stamp);
        s.replace_meeting_chunks(&m.id, &[ChunkSource::AiNotes], &[], &st)
            .unwrap();
        assert!(s.stale_meetings(10).unwrap().is_empty());
        s.update_document_body(&doc, "{\"x\":1}", stamp).unwrap();
        assert_eq!(s.stale_meetings(10).unwrap().len(), 1, "Handbearbeitung");
        let stamp = s.get_document(&doc).unwrap().unwrap().updated_at;
        st.enhanced_updated_at = Some(stamp);
        s.replace_meeting_chunks(&m.id, &[ChunkSource::AiNotes], &[], &st)
            .unwrap();

        // Transkript: neues Segment (Revision), Neu-Transkription (Epoche).
        s.append_delta(
            &m.id,
            &TranscriptDelta {
                new_segments: vec![StoredSegment {
                    segment_index: 0,
                    text: "Hallo".into(),
                    start_ms: 0,
                    end_ms: 1,
                    channel: 0,
                    speaker_index: None,
                    words: None,
                }],
            },
        )
        .unwrap();
        assert_eq!(s.stale_meetings(10).unwrap().len(), 1);
        st.transcript_rev = Some(1);
        s.replace_meeting_chunks(&m.id, &[ChunkSource::Transcript], &[], &st)
            .unwrap();
        assert!(s.stale_meetings(10).unwrap().is_empty());
        s.clear_segments(&m.id).unwrap();
        assert_eq!(s.stale_meetings(10).unwrap().len(), 1, "Neu-Transkription");

        // Geloeschte tauchen nie auf; `limit` gilt.
        s.soft_delete_meeting(&m.id).unwrap();
        assert!(s.stale_meetings(10).unwrap().is_empty());
        let _ = unfinished;
        let many: Vec<Meeting> = (0..5).map(|i| ready_meeting(&s, "n", i)).collect();
        assert_eq!(s.stale_meetings(3).unwrap().len(), 3);
        assert_eq!(s.stale_meetings(10).unwrap().len(), many.len());
    }

    #[test]
    fn missing_state_values_count_as_zero_so_the_indexer_cannot_loop_forever() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        // Ein Indexer, der None statt 0 speichert.
        let st = IndexState {
            status: STATUS_LEXICAL.into(),
            title: Some("M".into()),
            ..IndexState::default()
        };
        s.replace_meeting_chunks(&m.id, &[ChunkSource::Title], &[], &st)
            .unwrap();
        assert!(s.stale_meetings(10).unwrap().is_empty());
    }

    #[test]
    fn index_state_status_and_counts() {
        let (_d, s) = tmp_store();
        let a = ready_meeting(&s, "A", 1_000);
        let b = ready_meeting(&s, "B", 2_000);
        let c = ready_meeting(&s, "C", 3_000);
        let _without_state = ready_meeting(&s, "D ohne Zustand", 4_000);
        for m in [&a, &b, &c] {
            index_texts(&s, m, &["Text"]);
        }
        let counts = s.index_counts().unwrap();
        assert_eq!(
            (
                counts.meetings,
                counts.pending,
                counts.lexical,
                counts.embedded,
                counts.error
            ),
            (4, 1, 3, 0, 0)
        );
        assert_eq!(counts.chunks, 3);

        s.set_index_status(&a.id, STATUS_READY, None, Some("bge-m3"))
            .unwrap();
        s.set_index_status(&b.id, STATUS_ERROR, Some("Modell fehlt"), None)
            .unwrap();
        let st = s.index_state(&a.id).unwrap().unwrap();
        assert_eq!(st.status, STATUS_READY);
        assert_eq!(st.embed_model.as_deref(), Some("bge-m3"));
        assert!(st.embedded_at.is_some());
        let st = s.index_state(&b.id).unwrap().unwrap();
        assert_eq!(
            (st.status.as_str(), st.error.as_deref()),
            (STATUS_ERROR, Some("Modell fehlt"))
        );
        let counts = s.index_counts().unwrap();
        assert_eq!(
            (
                counts.embedded,
                counts.error,
                counts.lexical,
                counts.pending
            ),
            (1, 1, 1, 1)
        );

        assert_eq!(
            s.set_index_status("gibt-es-nicht", STATUS_READY, None, None)
                .unwrap_err()
                .to_string(),
            "index_state_not_found"
        );
        assert_eq!(
            s.set_index_status(&a.id, "quatsch", None, None)
                .unwrap_err()
                .to_string(),
            "invalid_status"
        );
        assert!(s.index_state("gibt-es-nicht").unwrap().is_none());
    }

    #[test]
    fn clear_search_index_removes_everything_derived_and_nothing_else() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        let ids = index_texts(&s, &m, &["Alpha", "Beta"]);
        s.put_vectors("mdl", &[(ids[0], vec![1.0, 0.0])]).unwrap();
        s.save_notes(
            &m.id,
            &[NoteBlock {
                id: "b".into(),
                kind: NoteBlockKind::Paragraph,
                text: "Notiz".into(),
                at_ms: None,
                checked: false,
            }],
            0,
        )
        .unwrap();
        s.clear_search_index().unwrap();
        assert_eq!(scalar(&s, "SELECT COUNT(*) FROM meeting_chunks"), 0);
        assert_eq!(scalar(&s, "SELECT COUNT(*) FROM meeting_chunk_vectors"), 0);
        assert_eq!(scalar(&s, "SELECT COUNT(*) FROM meeting_index_state"), 0);
        assert!(s.search_index_is_consistent().unwrap());
        assert!(search_ids(&s, "Alpha", &[&m]).is_empty());
        assert_eq!(
            s.get_notes(&m.id).unwrap().blocks.len(),
            1,
            "Quelle der Wahrheit unberuehrt"
        );
        assert_eq!(s.stale_meetings(10).unwrap(), vec![m.id.clone()]);
    }

    // ---- Nebenlaeufigkeit ------------------------------------------------

    #[test]
    fn writers_on_different_meetings_do_not_lose_each_other() {
        let (_d, s) = tmp_store();
        let meetings: Vec<Meeting> = (0..6)
            .map(|i| ready_meeting(&s, &format!("M{i}"), i))
            .collect();
        let barrier = Barrier::new(meetings.len());
        std::thread::scope(|scope| {
            for m in &meetings {
                let (s, barrier) = (&s, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    for round in 0..5 {
                        let drafts: Vec<ChunkDraft> = (0..10)
                            .map(|i| {
                                draft(
                                    ChunkSource::Transcript,
                                    &format!("Runde {round} Satz {i} Wort"),
                                )
                            })
                            .collect();
                        s.replace_meeting_chunks(
                            &m.id,
                            &[ChunkSource::Transcript],
                            &drafts,
                            &state(STATUS_LEXICAL),
                        )
                        .expect("gleichzeitiges Schreiben darf nicht scheitern");
                    }
                });
            }
        });
        assert_eq!(scalar(&s, "SELECT COUNT(*) FROM meeting_chunks"), 60);
        assert!(s.search_index_is_consistent().unwrap());
        assert_eq!(
            s.search_meetings("Wort", &MeetingFilter::default(), 0, 25)
                .unwrap()
                .total,
            6
        );
    }

    #[test]
    fn a_writer_waits_for_a_short_foreign_lock_instead_of_failing() {
        let (dir, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let path = dir.path().join("meetings.db");
        let blocker = std::thread::spawn(move || {
            let mut conn = Connection::open(path).unwrap();
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            locked_tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(400));
            tx.commit().unwrap();
        });
        locked_rx.recv().unwrap();
        let started = std::time::Instant::now();
        let ids = s
            .replace_meeting_chunks(
                &m.id,
                &[ChunkSource::Transcript],
                &[draft(ChunkSource::Transcript, "trotz Sperre")],
                &state(STATUS_LEXICAL),
            )
            .expect("Busy-Timeout statt sofortigem Fehler");
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(200),
            "hat gewartet"
        );
        blocker.join().unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(search_ids(&s, "Sperre", &[&m]).len(), 1);
    }

    // ---- Scope, Person ---------------------------------------------------

    #[test]
    fn resolve_scope_combines_folder_time_selection_and_person() {
        let (_d, s) = tmp_store();
        let a = ready_meeting(&s, "Nordlicht Kickoff", 1_000);
        let b = ready_meeting(&s, "Projektrunde", 5_000);
        let c = ready_meeting(&s, "Einkauf", 9_000);
        let live = s
            .create_meeting("Laeuft", MeetingSource::Live, Some(1))
            .unwrap();
        let gone = ready_meeting(&s, "Weg", 6_000);
        s.soft_delete_meeting(&gone.id).unwrap();
        index_texts(&s, &a, &["Frau Müller von Nordlicht war dabei"]);
        index_texts(&s, &b, &["Nur Technik heute"]);
        index_texts(&s, &c, &["Herr Meier sprach"]);
        let f = s.folder_save(None, "Vertrieb", None).unwrap();
        s.set_meeting_folders(&a.id, &[f.id.clone()]).unwrap();
        s.set_meeting_folders(&c.id, &[f.id.clone()]).unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "INSERT INTO speakers (id, meeting_id, channel, display_name, created_at, updated_at)
                 VALUES ('S1', ?1, 1, 'Herr Meier', 1, 1)",
                params![b.id],
            )
            .unwrap();

        let ids = |f: ScopeFilter| s.resolve_scope(&f).unwrap();
        // Alle fertigen, lebenden, neueste zuerst: weder laufende noch geloeschte.
        assert_eq!(
            ids(ScopeFilter::default()),
            vec![c.id.clone(), b.id.clone(), a.id.clone()]
        );
        assert!(!ids(ScopeFilter::default()).contains(&live.id));
        assert_eq!(
            ids(ScopeFilter {
                folder_id: Some(f.id.clone()),
                ..Default::default()
            }),
            vec![c.id.clone(), a.id.clone()]
        );
        assert_eq!(
            ids(ScopeFilter {
                from: Some(2_000),
                to: Some(8_000),
                ..Default::default()
            }),
            vec![b.id.clone()]
        );
        assert_eq!(
            ids(ScopeFilter {
                meeting_ids: Some(vec![a.id.clone(), gone.id.clone(), "fremd".into()]),
                ..Default::default()
            }),
            vec![a.id.clone()],
            "Auswahl schneidet mit lebend+fertig"
        );
        assert!(ids(ScopeFilter {
            meeting_ids: Some(vec![]),
            ..Default::default()
        })
        .is_empty());
        // Person: Sprechername (b) oder Text (a, c).
        assert_eq!(
            ids(ScopeFilter {
                person: Some("meier".into()),
                ..Default::default()
            }),
            vec![c.id.clone(), b.id.clone()]
        );
        assert_eq!(
            ids(ScopeFilter {
                person: Some("MÜLLER".into()),
                ..Default::default()
            }),
            vec![a.id.clone()]
        );
        assert_eq!(
            ids(ScopeFilter {
                person: Some("Meier".into()),
                folder_id: Some(f.id),
                ..Default::default()
            }),
            vec![c.id.clone()],
            "Filter gelten zugleich"
        );
        assert!(ids(ScopeFilter {
            person: Some("Niemand".into()),
            ..Default::default()
        })
        .is_empty());
    }

    // ---- Ordner ----------------------------------------------------------

    #[test]
    fn folders_are_created_renamed_counted_and_deleted_without_touching_meetings() {
        let (_d, s) = tmp_store();
        let m1 = ready_meeting(&s, "M1", 1_000);
        let m2 = ready_meeting(&s, "M2", 2_000);
        let vertrieb = s
            .folder_save(None, "  Vertrieb  ", Some("#3366ff"))
            .unwrap();
        let projekte = s.folder_save(None, "Projekte", None).unwrap();
        assert_eq!(vertrieb.name, "Vertrieb");
        assert_eq!(vertrieb.color.as_deref(), Some("#3366ff"));
        assert!(projekte.sort > vertrieb.sort);

        // n:m, Mehrfachwahl, Ersetzen.
        s.set_meeting_folders(&m1.id, &[vertrieb.id.clone(), projekte.id.clone()])
            .unwrap();
        s.set_meeting_folders(&m2.id, &[vertrieb.id.clone(), vertrieb.id.clone()])
            .unwrap();
        let listed = s.folders_list().unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|f| (f.name.as_str(), f.meeting_count))
                .collect::<Vec<_>>(),
            vec![("Vertrieb", 2), ("Projekte", 1)]
        );
        s.set_meeting_folders(&m1.id, &[projekte.id.clone()])
            .unwrap();
        assert_eq!(
            s.meeting_folder_ids(&m1.id).unwrap(),
            vec![projekte.id.clone()]
        );
        assert_eq!(s.folders_list().unwrap()[0].meeting_count, 1);

        // Umbenennen behaelt Mitglieder; Namen sind eindeutig (ohne Gross/Klein).
        let renamed = s.folder_save(Some(&vertrieb.id), "Sales", None).unwrap();
        assert_eq!((renamed.name.as_str(), renamed.meeting_count), ("Sales", 1));
        assert_eq!(
            s.folder_save(None, "sales", None).unwrap_err().to_string(),
            "folder_name_taken"
        );
        assert!(
            s.folder_save(Some(&renamed.id), "SALES", None).is_ok(),
            "eigener Name erlaubt"
        );

        // Loeschen: Zuordnung weg, Besprechung bleibt.
        s.folder_delete(&renamed.id).unwrap();
        assert!(s.get_meeting(&m2.id).unwrap().is_some());
        assert!(s.meeting_folder_ids(&m2.id).unwrap().is_empty());
        assert_eq!(s.folders_list().unwrap().len(), 1);
        assert_eq!(
            s.folder_delete(&renamed.id).unwrap_err().to_string(),
            "folder_not_found"
        );
        assert_eq!(
            s.folder_save(Some(&renamed.id), "Zombie", None)
                .unwrap_err()
                .to_string(),
            "folder_not_found"
        );
        s.folder_save(None, "Sales", None)
            .expect("Name wieder frei");
    }

    #[test]
    fn folder_input_is_validated_and_a_bad_assignment_changes_nothing() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        let too_long = "x".repeat(61);
        for bad in ["", "   ", too_long.as_str(), "Zeile\numbruch"] {
            assert_eq!(
                s.folder_save(None, bad, None).unwrap_err().to_string(),
                "folder_name_invalid",
                "{bad:?}"
            );
        }
        assert_eq!(
            s.folder_save(None, "Ok", Some(&"c".repeat(40)))
                .unwrap_err()
                .to_string(),
            "folder_color_invalid"
        );
        let f = s.folder_save(None, "Ok", None).unwrap();
        s.set_meeting_folders(&m.id, &[f.id.clone()]).unwrap();
        assert_eq!(
            s.set_meeting_folders(&m.id, &["gibt-es-nicht".into()])
                .unwrap_err()
                .to_string(),
            "folder_not_found"
        );
        assert_eq!(
            s.meeting_folder_ids(&m.id).unwrap(),
            vec![f.id.clone()],
            "alles wie vorher"
        );
        assert!(s.set_meeting_folders("fremd", &[f.id]).is_err());
        s.set_meeting_folders(&m.id, &[]).unwrap();
        assert!(s.meeting_folder_ids(&m.id).unwrap().is_empty());
    }

    // ---- Recipes ---------------------------------------------------------

    #[test]
    fn recipes_can_be_saved_listed_edited_and_deleted() {
        let (_d, s) = tmp_store();
        let spec = "{\"version\":1,\"prompt\":\"Was war {{thema}}?\"}";
        let r = s.recipe_save(None, "  Mein Recipe ", spec).unwrap();
        assert_eq!(r.title, "Mein Recipe");
        assert!(!r.builtin);
        let updated = s
            .recipe_save(Some(&r.id), "Neu", "{\"version\":1}")
            .unwrap();
        assert_eq!(updated.id, r.id);
        assert_eq!(s.recipe_get(&r.id).unwrap().unwrap().title, "Neu");
        assert_eq!(s.recipes_list().unwrap().len(), 1);
        s.recipe_delete(&r.id).unwrap();
        assert!(s.recipes_list().unwrap().is_empty());
        assert!(s.recipe_get(&r.id).unwrap().is_none());
        assert_eq!(
            s.recipe_delete(&r.id).unwrap_err().to_string(),
            "recipe_not_found"
        );
        assert_eq!(
            s.recipe_save(Some(&r.id), "x", "{}")
                .unwrap_err()
                .to_string(),
            "recipe_not_found"
        );
        for (title, spec) in [
            ("", "{}"),
            ("T", "kein json"),
            ("T", "[1,2]"),
            ("T", "\"text\""),
        ] {
            assert!(
                s.recipe_save(None, title, spec)
                    .unwrap_err()
                    .to_string()
                    .starts_with("recipe_invalid"),
                "{title:?} {spec:?}"
            );
        }
        assert!(s
            .recipe_save(
                None,
                "Riesig",
                &format!("{{\"p\":\"{}\"}}", "x".repeat(40_000))
            )
            .is_err());
    }

    #[test]
    fn builtin_recipes_are_seeded_idempotently_readonly_and_restored() {
        let (_d, s) = tmp_store();
        let items = vec![
            (
                "builtin:verpasst".to_string(),
                "Was habe ich verpasst?".to_string(),
                "{\"version\":1}".to_string(),
            ),
            (
                "builtin:fragen".to_string(),
                "Was sollte ich fragen?".to_string(),
                "{\"version\":1}".to_string(),
            ),
        ];
        s.recipes_seed_builtin(&items).unwrap();
        s.recipe_save(None, "Eigenes", "{}").unwrap();
        let listed = s.recipes_list().unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|r| (r.id.as_str(), r.builtin))
                .collect::<Vec<_>>(),
            vec![
                ("builtin:verpasst", true),
                ("builtin:fragen", true),
                (listed[2].id.as_str(), false)
            ],
            "mitgelieferte zuerst, in Einfuegereihenfolge"
        );
        let stamp = |s: &MeetingStore| {
            s.recipe_get("builtin:verpasst")
                .unwrap()
                .unwrap()
                .updated_at
        };
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE chat_recipes SET updated_at = 42 WHERE id LIKE 'builtin:%'",
                [],
            )
            .unwrap();
        s.recipes_seed_builtin(&items).unwrap();
        assert_eq!(stamp(&s), 42, "aktuelle Zeilen bleiben unberuehrt");

        assert_eq!(
            s.recipe_save(Some("builtin:verpasst"), "x", "{}")
                .unwrap_err()
                .to_string(),
            "recipe_readonly"
        );
        assert_eq!(
            s.recipe_delete("builtin:verpasst").unwrap_err().to_string(),
            "recipe_readonly"
        );

        // Bearbeitete oder geloeschte Zeilen kommen zurueck.
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE chat_recipes SET title = 'kaputt', deleted_at = 1 WHERE id = 'builtin:fragen'",
                [],
            )
            .unwrap();
        s.recipes_seed_builtin(&items).unwrap();
        assert_eq!(
            s.recipe_get("builtin:fragen").unwrap().unwrap().title,
            "Was sollte ich fragen?"
        );
        assert_eq!(
            s.recipes_seed_builtin(&[("nicht-builtin".into(), "T".into(), "{}".into())])
                .unwrap_err()
                .to_string(),
            "recipe_invalid:id"
        );
    }

    // ---- Chat-Verlaeufe --------------------------------------------------

    #[test]
    fn threads_hold_messages_in_order_and_are_listed_per_scope() {
        let (_d, s) = tmp_store();
        let m = ready_meeting(&s, "M", 1_000);
        let mine = s
            .thread_create("{\"meeting\":true}", Some(&m.id), None)
            .unwrap();
        let global = s
            .thread_create("{\"global\":true}", None, Some("Manuell"))
            .unwrap();
        assert_eq!(mine.title, None);
        assert_eq!(global.title.as_deref(), Some("Manuell"));

        let q = s
            .thread_append(
                &mine.id,
                "user",
                "  Was wurde \n zum Budget  beschlossen?  ",
                None,
                None,
            )
            .unwrap();
        let a = s
            .thread_append(
                &mine.id,
                "assistant",
                "Das Budget wurde erhoeht [1].",
                Some("[{\"n\":1}]"),
                Some("{\"meetings_read\":1}"),
            )
            .unwrap();
        s.thread_append(&global.id, "user", "Global?", None, None)
            .unwrap();

        let (thread, msgs) = s.thread_get(&mine.id).unwrap().unwrap();
        assert_eq!(
            thread.title.as_deref(),
            Some("Was wurde zum Budget beschlossen?"),
            "erste Frage wird Titel"
        );
        assert_eq!(thread.message_count, 2);
        assert_eq!(
            msgs.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(),
            vec!["user", "assistant"]
        );
        assert_eq!(msgs[0].id, q.id);
        assert_eq!(msgs[1].citations_json.as_deref(), Some("[{\"n\":1}]"));
        assert_eq!(
            msgs[1].coverage_json.as_deref(),
            Some("{\"meetings_read\":1}")
        );
        assert_eq!(a.thread_id, mine.id);
        assert_eq!(
            s.thread_get(&global.id)
                .unwrap()
                .unwrap()
                .0
                .title
                .as_deref(),
            Some("Manuell"),
            "gesetzter Titel bleibt"
        );

        assert_eq!(
            s.thread_list(&ThreadScope::Meeting(m.id.clone()))
                .unwrap()
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            vec![mine.id.clone()]
        );
        assert_eq!(
            s.thread_list(&ThreadScope::Global)
                .unwrap()
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            vec![global.id.clone()]
        );

        s.thread_delete(&mine.id).unwrap();
        assert!(s.thread_get(&mine.id).unwrap().is_none());
        assert!(s
            .thread_list(&ThreadScope::Meeting(m.id.clone()))
            .unwrap()
            .is_empty());
        assert_eq!(
            scalar(
                &s,
                &format!(
                    "SELECT COUNT(*) FROM chat_messages WHERE thread_id = '{}'",
                    mine.id
                )
            ),
            0,
            "Text ist hart geloescht"
        );
        assert_eq!(
            s.thread_delete(&mine.id).unwrap_err().to_string(),
            "thread_not_found"
        );
        assert_eq!(
            s.thread_append(&mine.id, "user", "zu spaet", None, None)
                .unwrap_err()
                .to_string(),
            "thread_not_found"
        );
    }

    #[test]
    fn thread_input_is_validated() {
        let (_d, s) = tmp_store();
        let t = s.thread_create("{}", None, None).unwrap();
        let code = |r: Result<ChatMessageRow>| r.unwrap_err().to_string();
        assert_eq!(
            code(s.thread_append(&t.id, "system", "x", None, None)),
            "invalid_role"
        );
        assert_eq!(
            code(s.thread_append(&t.id, "user", &"x".repeat(70_000), None, None)),
            "message_too_large"
        );
        assert_eq!(
            code(s.thread_append(&t.id, "user", "x", Some("{kaputt"), None)),
            "thread_invalid:json"
        );
        assert!(s.thread_create("kein json", None, None).is_err());
        assert!(s
            .thread_create("{}", Some("fremd"), None)
            .unwrap_err()
            .to_string()
            .contains("not found"));
        assert_eq!(
            s.thread_get(&t.id).unwrap().unwrap().1.len(),
            0,
            "nichts halb geschrieben"
        );
        // Bis zur Obergrenze auffuellen (in einer Transaktion, sonst dauert es Sekunden).
        s.get_connection()
            .unwrap()
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?2)
                 INSERT INTO chat_messages (id, thread_id, role, content, created_at)
                 SELECT 'm' || i, ?1, 'user', 'x', 1 FROM n",
                params![t.id, MAX_THREAD_MESSAGES],
            )
            .unwrap();
        assert_eq!(
            code(s.thread_append(&t.id, "user", "eine zu viel", None, None)),
            "thread_full"
        );
    }

    // ---- Typen fuer die Commands (P4d/P4c) --------------------------------

    #[test]
    fn public_types_export_to_typescript() {
        let mut types = specta::TypeCollection::default();
        types
            .register::<MeetingFilter>()
            .register::<MeetingSearchPage>()
            .register::<ScopeFilter>()
            .register::<Folder>()
            .register::<RecipeInfo>()
            .register::<ChatThread>()
            .register::<ChatMessageRow>()
            .register::<IndexCounts>()
            .register::<IndexState>()
            .register::<ChunkRow>();
        let ts = specta_typescript::Typescript::default()
            .bigint(specta_typescript::BigIntExportBehavior::Number)
            .export(&types)
            .expect("Typen muessen exportierbar sein");
        for expected in [
            "export type MeetingSearchPage",
            "export type ChunkSource = \"title\" | \"transcript\" | \"user_notes\" | \"ai_notes\"",
            "export type MeetingFilter",
            "export type Folder",
        ] {
            assert!(ts.contains(expected), "{expected} fehlt in:\n{ts}");
        }
    }
}
