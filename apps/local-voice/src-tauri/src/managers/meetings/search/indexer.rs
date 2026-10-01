//! Indexer fuer Besprechungen (M4 §4, P4b): haelt den Such-Index aktuell.
//!
//! Zwei Stufen:
//! 1. **Lexikalisch, sofort** (ein Thread, Millisekunden je Besprechung):
//!    Titel, Transkript, Nutzernotizen und juengste KI-Notizen werden in Chunks
//!    zerlegt und mit FTS in `meetings.db` geschrieben (`replace_meeting_chunks`,
//!    EINE Transaktion je Besprechung). Nur geaenderte Quellen werden ersetzt.
//! 2. **Vektoren, im Hintergrund**: Chunks ohne Vektor gehen in Chargen zu
//!    `EMBED_BATCH` an den Embedder (zweiter `llama-server`, BGE-M3). Nur wenn
//!    alle Gates frei sind (keine Aufnahme, kein Enddurchlauf, kein
//!    KI-Notizen-Lauf), die Einstellung an ist und das Modell vorliegt. Nach
//!    jeder Charge werden die Gates neu geprueft; schliesst eines, wird der
//!    Server sofort beendet. Fehler (kein RAM, Absturz) → Backoff 10 min.
//!
//! Der Kern (`IndexerCore`) ist synchron und bekommt die Zeit von aussen, damit
//! Gates, Entprellung und Backoff ohne Threads und ohne Warten testbar sind.
//! `MeetingIndexer` ist der duenne Thread darum; `start_for_app` die Anbindung
//! an Tauri (Ereignisse, Gates, Einstellung).
//!
//! Datenschutz: kein Besprechungstext im Log, nur Codes, IDs und Zahlen.

use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::super::notes::enhance::DOC_FORMAT;
use super::super::notes::model::{EnhancedNotes, NoteBlock};
use super::super::store::{MeetingStore, StoredSegment};
use super::chunking::{
    chunk_enhanced, chunk_title, chunk_transcript_with, chunk_user_notes, ChunkDraft, ChunkHead,
    ChunkSource,
};
use super::super::speakers::SpeakerDirectory;
use super::embed::{EmbedError, EmbedKind, Embedder};
use super::index::{IndexState, STATUS_ERROR, STATUS_LEXICAL, STATUS_PENDING};
use super::vectors::{global_cache, VectorCache, IDLE_TTL};

/// Chunks je Embedding-Anfrage (M4 §4).
pub const EMBED_BATCH: usize = 16;
/// Pause nach einem Fehler der Vektorstufe (kein Retry-Sturm, M4 §10).
pub const BACKOFF: Duration = Duration::from_secs(10 * 60);
/// Der Embedding-Server wird nach so langer Nichtnutzung beendet.
pub const EMBED_IDLE_STOP: Duration = Duration::from_secs(5 * 60);
/// Entprellung fuer `meeting_notes_save` (jede Taste speichert).
pub const NOTES_DEBOUNCE: Duration = Duration::from_secs(30);
/// Laengstes Warten des Threads ohne Auftrag (Gates, Leerlauf pruefen).
pub const TICK: Duration = Duration::from_secs(30);
/// Chargen je Durchlauf, bevor der Thread neue Auftraege annimmt.
pub const SLICE_BATCHES: usize = 8;
/// Wie oft ein veralteter Schreibversuch (`stale_*`) neu eingereiht wird.
const MAX_STALE_RETRIES: u8 = 3;
/// Mehr uebersprungene ("giftige") Chunks als das: systemischer Fehler.
const MAX_SKIPPED_CHUNKS: usize = 256;
const ALL_SOURCES: [ChunkSource; 4] = [
    ChunkSource::Title,
    ChunkSource::Transcript,
    ChunkSource::UserNotes,
    ChunkSource::AiNotes,
];

// ---------------------------------------------------------------------------
// Vertrag
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexJob {
    /// Eine Besprechung (neu/geaendert) lexikalisch indexieren, dann Vektoren.
    Meeting(String),
    /// Besprechung geloescht: Queue und Vektor-Cache bereinigen (die Zeilen
    /// entfernt `soft_delete_meeting` schon in seiner Transaktion).
    Deleted(String),
    /// App-Start: alle veralteten Besprechungen nachholen.
    Backfill,
    /// Vektoren fuer alle Chunks ohne Vektor nachholen (Modell geladen,
    /// Einstellung an). Hebt einen laufenden Backoff auf.
    EmbedPending,
}

/// Wann die Vektorstufe NICHT laufen darf. Die lexikalische Stufe laeuft immer.
pub trait IndexGates: Send + Sync {
    fn recording_active(&self) -> bool;
    fn processing_active(&self) -> bool;
    fn enhance_running(&self) -> bool;
    /// Einstellung `meeting_semantic_search`.
    fn semantic_enabled(&self) -> bool {
        true
    }
}

/// Gates ohne Einschraenkung (Headless-Lauf `--reindex-meetings`).
pub struct NoGates;

impl IndexGates for NoGates {
    fn recording_active(&self) -> bool {
        false
    }
    fn processing_active(&self) -> bool {
        false
    }
    fn enhance_running(&self) -> bool {
        false
    }
}

/// Fortschritt fuer die Einstellungszeile: `(fertig eingebettete, alle)`
/// Besprechungen.
pub type ProgressFn = Arc<dyn Fn(u32, u32) + Send + Sync>;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct IndexerStats {
    pub meetings_indexed: u32,
    pub stale_retries: u32,
    pub vectors_written: u64,
    pub embed_batches: u32,
    pub embed_errors: u32,
    pub skipped_chunks: u32,
}

/// Ergebnis eines Durchlaufs der Vektorstufe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VectorStep {
    /// Nichts zu tun.
    Idle,
    /// Darf gerade nicht (Gate, Einstellung, kein Modell, Backoff).
    Blocked(&'static str),
    /// Chargen geschrieben, es bleibt Arbeit.
    Progress(usize),
    /// Alle Chunks haben Vektoren.
    Done,
    /// Fehler; Backoff laeuft.
    Error(&'static str),
}

// ---------------------------------------------------------------------------
// Store-Hilfen (nur fuer den Indexer)
// ---------------------------------------------------------------------------

/// Alles, woraus der Index einer Besprechung entsteht, aus EINER
/// Lesetransaktion (konsistenter Stand).
#[derive(Clone, Debug)]
struct Snapshot {
    meeting_id: String,
    title: String,
    /// U7: Beschreibung (leer = keine).
    description: String,
    status: String,
    started_at: Option<i64>,
    folders: Vec<String>,
    epoch: u32,
    rev: u64,
    segments: Vec<StoredSegment>,
    notes_revision: u64,
    blocks: Vec<NoteBlock>,
    /// (Dokument-ID, `updated_at`, lesbarer Inhalt)
    doc: Option<(String, i64, Option<EnhancedNotes>)>,
}

impl MeetingStore {
    fn index_snapshot(&self, meeting_id: &str) -> Result<Option<Snapshot>> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction()?;
        let Some((title, description, status, started_at)) = tx
            .query_row(
                "SELECT title, COALESCE(description, ''), status, COALESCE(started_at, created_at)
                 FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let (segments_json, epoch, rev): (String, i64, i64) = tx
            .query_row(
                "SELECT segments_json, segment_epoch, content_revision FROM transcripts
                 WHERE meeting_id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .unwrap_or_else(|| ("[]".to_string(), 0, 0));
        // Beschaedigtes JSON kostet diese Quelle, nicht den ganzen Index.
        let segments: Vec<StoredSegment> = serde_json::from_str(&segments_json).unwrap_or_else(|_| {
            log::warn!("Indexer: Transkript von {meeting_id} nicht lesbar, ohne Transkript indexiert");
            Vec::new()
        });
        let (blocks_json, notes_revision): (String, i64) = tx
            .query_row(
                "SELECT blocks_json, revision FROM meeting_notes
                 WHERE meeting_id = ?1 AND deleted_at IS NULL",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .unwrap_or_else(|| ("[]".to_string(), 0));
        let blocks: Vec<NoteBlock> = serde_json::from_str(&blocks_json).unwrap_or_else(|_| {
            log::warn!("Indexer: Notizen von {meeting_id} nicht lesbar, ohne Notizen indexiert");
            Vec::new()
        });
        let doc = tx
            .query_row(
                "SELECT id, body_format, body, updated_at FROM meeting_documents
                 WHERE meeting_id = ?1 AND kind = 'enhanced_notes' AND deleted_at IS NULL
                 ORDER BY version DESC LIMIT 1",
                params![meeting_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?
            .map(|(id, format, body, updated_at)| {
                let notes = (format == DOC_FORMAT)
                    .then(|| serde_json::from_str::<EnhancedNotes>(&body).ok())
                    .flatten();
                (id, updated_at, notes)
            });
        let folders = {
            let mut stmt = tx.prepare(
                "SELECT f.name FROM meeting_folder_items fi
                 JOIN meeting_folders f ON f.id = fi.folder_id AND f.deleted_at IS NULL
                 WHERE fi.meeting_id = ?1 ORDER BY f.sort, f.name",
            )?;
            let names = stmt
                .query_map(params![meeting_id], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            names
        };
        Ok(Some(Snapshot {
            meeting_id: meeting_id.to_string(),
            title,
            description,
            status,
            started_at,
            folders,
            epoch: epoch.max(0) as u32,
            rev: rev.max(0) as u64,
            segments,
            notes_revision: notes_revision.max(0) as u64,
            blocks,
            doc,
        }))
    }

    /// Setzt alle `lexical`-Besprechungen, deren Chunks jetzt alle einen
    /// Vektor fuer `model` haben, auf `ready`. Liefert die Zahl.
    fn mark_embedded(&self, model: &str) -> Result<usize> {
        let now = chrono::Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let changed = tx.execute(
            "UPDATE meeting_index_state
                SET status = 'ready', embed_model = ?1, embedded_at = ?2, error = NULL, updated_at = ?2
              WHERE status = 'lexical'
                AND EXISTS (SELECT 1 FROM meetings m
                             WHERE m.id = meeting_index_state.meeting_id AND m.deleted_at IS NULL)
                AND NOT EXISTS (
                    SELECT 1 FROM meeting_chunks c
                     WHERE c.meeting_id = meeting_index_state.meeting_id
                       AND NOT EXISTS (SELECT 1 FROM meeting_chunk_vectors v
                                        WHERE v.chunk_id = c.id AND v.model = ?1))",
            params![model, now],
        )?;
        tx.commit()?;
        Ok(changed)
    }

    /// Zahl gespeicherter Vektoren fuer `model` (nur zu lebenden Chunks).
    pub fn vector_count(&self, model: &str) -> Result<u32> {
        let conn = self.get_connection()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM meeting_chunk_vectors v
             JOIN meeting_chunks c ON c.id = v.chunk_id
             WHERE v.model = ?1",
            params![model],
            |row| row.get(0),
        )?;
        Ok(n.max(0) as u32)
    }
}

/// Schluessel des Index-Kopfes (`IndexState.title`): der Titel, bei einer
/// Beschreibung dazu ein Trennzeichen und die Beschreibung. Ohne Beschreibung
/// ist er der Titel selbst, wie vor U7 (kein Neuaufbau aller Indizes beim Update).
fn head_key(title: &str, description: &str) -> String {
    if description.trim().is_empty() {
        title.to_string()
    } else {
        format!("{title}\u{1e}{}", description.trim())
    }
}

/// Fehlercode aus einem Store-Fehler (die Codes stehen vorn im Text).
fn error_code(e: &anyhow::Error) -> String {
    let text = e.to_string();
    text.split([':', ' ']).next().unwrap_or("error").to_string()
}

fn is_stale(e: &anyhow::Error) -> bool {
    matches!(
        error_code(e).as_str(),
        "stale_epoch" | "stale_transcript" | "stale_notes" | "stale_document"
    )
}

fn normalized(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>().sqrt();
    if norm <= 0.0 || !norm.is_finite() {
        return v.to_vec();
    }
    v.iter().map(|x| (f64::from(*x) / norm) as f32).collect()
}

// ---------------------------------------------------------------------------
// Kern (synchron, Zeit von aussen)
// ---------------------------------------------------------------------------

/// Was beim lexikalischen Indexieren einer Besprechung herauskam.
#[derive(Clone, Debug, PartialEq, Eq)]
enum LexicalOutcome {
    /// Quellen ersetzt (Zahl der neuen Chunks).
    Indexed(usize),
    /// Nichts geaendert.
    Unchanged,
    /// Nicht (mehr) indexierbar: geloescht oder nicht `ready`.
    Skipped,
    /// Quelle hat sich zwischen Lesen und Schreiben geaendert.
    Stale,
}

pub struct IndexerCore {
    store: Arc<MeetingStore>,
    embed: Arc<dyn Embedder>,
    gates: Arc<dyn IndexGates>,
    cache: &'static VectorCache,
    queue: VecDeque<String>,
    queued: HashSet<String>,
    retries: HashMap<String, u8>,
    debounced: HashMap<String, Instant>,
    want_vectors: bool,
    backoff_until: Option<Instant>,
    skip_chunks: HashSet<i64>,
    /// Die Vektorstufe hat den Server in diesem Lauf benutzt: schliesst ein
    /// Gate, wird er sofort beendet.
    embedding_active: bool,
    last_error: Option<&'static str>,
    progress: Option<ProgressFn>,
    stats: IndexerStats,
}

impl IndexerCore {
    pub fn new(
        store: Arc<MeetingStore>,
        embed: Arc<dyn Embedder>,
        gates: Arc<dyn IndexGates>,
        cache: &'static VectorCache,
    ) -> Self {
        Self {
            store,
            embed,
            gates,
            cache,
            queue: VecDeque::new(),
            queued: HashSet::new(),
            retries: HashMap::new(),
            debounced: HashMap::new(),
            want_vectors: false,
            backoff_until: None,
            skip_chunks: HashSet::new(),
            embedding_active: false,
            last_error: None,
            progress: None,
            stats: IndexerStats::default(),
        }
    }

    pub fn with_progress(mut self, progress: ProgressFn) -> Self {
        self.progress = Some(progress);
        self
    }

    pub fn stats(&self) -> &IndexerStats {
        &self.stats
    }

    pub fn last_error(&self) -> Option<&'static str> {
        self.last_error
    }

    fn enqueue(&mut self, meeting_id: String) {
        if self.queued.insert(meeting_id.clone()) {
            self.queue.push_back(meeting_id);
        }
    }

    pub fn handle(&mut self, job: IndexJob, _now: Instant) {
        match job {
            IndexJob::Meeting(id) => {
                // Ein direkter Auftrag liest ohnehin den neuesten Stand.
                self.debounced.remove(&id);
                self.enqueue(id);
            }
            IndexJob::Deleted(id) => {
                self.debounced.remove(&id);
                self.retries.remove(&id);
                if self.queued.remove(&id) {
                    self.queue.retain(|q| q != &id);
                }
                self.cache.update(|index| index.remove_meeting(&id));
            }
            IndexJob::Backfill => {
                match self.store.stale_meetings(u32::MAX) {
                    Ok(ids) => {
                        if !ids.is_empty() {
                            log::info!("Indexer: {} Besprechungen nachzuholen", ids.len());
                        }
                        for id in ids {
                            self.enqueue(id);
                        }
                    }
                    Err(e) => log::warn!("Indexer: Nachholen nicht lesbar ({})", error_code(&e)),
                }
                self.want_vectors = true;
            }
            IndexJob::EmbedPending => {
                self.want_vectors = true;
                self.backoff_until = None;
            }
        }
    }

    /// `meeting_notes_save`: erst nach `NOTES_DEBOUNCE` ohne weiteres
    /// Speichern indexieren.
    pub fn debounce(&mut self, meeting_id: String, now: Instant) {
        self.debounced.insert(meeting_id, now + NOTES_DEBOUNCE);
    }

    /// Faellige Entprellungen einreihen und die Queue abarbeiten.
    pub fn run_lexical(&mut self, now: Instant) -> usize {
        let due: Vec<String> = self
            .debounced
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.debounced.remove(&id);
            self.enqueue(id);
        }
        let mut indexed = 0usize;
        // Neu eingereihte Veraltete kommen hinten an; die Schleife endet, weil
        // jede Besprechung hoechstens MAX_STALE_RETRIES-mal zurueckkommt.
        while let Some(id) = self.queue.pop_front() {
            self.queued.remove(&id);
            match self.index_meeting(&id) {
                Ok(LexicalOutcome::Indexed(_)) => {
                    indexed += 1;
                    self.retries.remove(&id);
                    self.want_vectors = true;
                }
                Ok(LexicalOutcome::Unchanged) => {
                    self.retries.remove(&id);
                }
                Ok(LexicalOutcome::Skipped) => {
                    self.retries.remove(&id);
                    self.cache.update(|index| index.remove_meeting(&id));
                }
                Ok(LexicalOutcome::Stale) => {
                    let n = self.retries.entry(id.clone()).or_insert(0);
                    *n += 1;
                    self.stats.stale_retries += 1;
                    if *n <= MAX_STALE_RETRIES {
                        self.enqueue(id);
                    } else {
                        log::warn!("Indexer: {id} aendert sich laufend, spaeter erneut");
                        self.retries.remove(&id);
                    }
                }
                Err(e) => {
                    let code = error_code(&e);
                    log::warn!("Indexer: {id} nicht indexiert ({code})");
                    let _ = self
                        .store
                        .set_index_status(&id, STATUS_ERROR, Some(&code), None);
                }
            }
        }
        if indexed > 0 {
            self.stats.meetings_indexed += indexed as u32;
            self.emit_progress();
        }
        indexed
    }

    fn index_meeting(&mut self, meeting_id: &str) -> Result<LexicalOutcome> {
        let Some(snapshot) = self.store.index_snapshot(meeting_id)? else {
            return Ok(LexicalOutcome::Skipped);
        };
        self.write_snapshot(&snapshot)
    }

    /// Schreibt den Index aus einem gelesenen Stand. Getrennt vom Lesen, damit
    /// ein Test "Quelle aendert sich dazwischen" nachstellen kann.
    fn write_snapshot(&mut self, snap: &Snapshot) -> Result<LexicalOutcome> {
        // P8a: auch ein vom Nutzer gestopptes Teil-Transkript (`cancelled`) ist
        // durchsuchbar; laufende und fehlgeschlagene Besprechungen nicht.
        if !matches!(snap.status.as_str(), "ready" | "cancelled") {
            return Ok(LexicalOutcome::Skipped);
        }
        let prev = self.store.index_state(&snap.meeting_id)?;
        let (doc_id, doc_updated) = match &snap.doc {
            Some((id, at, _)) => (Some(id.clone()), Some(*at)),
            None => (None, None),
        };
        // U7: Titel und Beschreibung bilden zusammen den "Kopf" des Index; aendert
        // sich einer von beiden, wird alles neu aufgebaut (die Kopfzeile steckt in
        // jedem Einbettungstext).
        let head_key = head_key(&snap.title, &snap.description);
        let full = match &prev {
            None => true,
            Some(p) => {
                p.status == STATUS_PENDING
                    || p.status == STATUS_ERROR
                    || p.title.as_deref() != Some(head_key.as_str())
            }
        };
        let mut sources: Vec<ChunkSource> = Vec::new();
        if full {
            sources.extend(ALL_SOURCES);
        } else if let Some(p) = &prev {
            if p.transcript_epoch.unwrap_or(0) != snap.epoch
                || p.transcript_rev.unwrap_or(0) != snap.rev
            {
                sources.push(ChunkSource::Transcript);
            }
            if p.notes_revision.unwrap_or(0) != snap.notes_revision {
                sources.push(ChunkSource::UserNotes);
            }
            if p.enhanced_doc_id != doc_id
                || p.enhanced_updated_at.unwrap_or(0) != doc_updated.unwrap_or(0)
            {
                sources.push(ChunkSource::AiNotes);
            }
        }
        if sources.is_empty() {
            return Ok(LexicalOutcome::Unchanged);
        }

        let head = ChunkHead {
            title: snap.title.clone(),
            description: snap.description.clone(),
            started_at: snap.started_at,
            folder_names: snap.folders.clone(),
        };
        let mut drafts: Vec<ChunkDraft> = Vec::new();
        for source in &sources {
            match source {
                ChunkSource::Title => drafts.extend(chunk_title(&head)),
                ChunkSource::Transcript => {
                    let speakers = SpeakerDirectory::load(&self.store, &snap.meeting_id);
                    drafts.extend(chunk_transcript_with(
                        &snap.segments,
                        snap.epoch,
                        &head,
                        &speakers,
                    ))
                }
                ChunkSource::UserNotes => drafts.extend(chunk_user_notes(&snap.blocks, &head)),
                ChunkSource::AiNotes => {
                    if let Some((id, _, Some(notes))) = &snap.doc {
                        drafts.extend(chunk_enhanced(id, notes, &head));
                    }
                }
            }
        }
        let state = IndexState {
            transcript_epoch: Some(snap.epoch),
            transcript_rev: Some(snap.rev),
            notes_revision: Some(snap.notes_revision),
            enhanced_doc_id: doc_id,
            enhanced_updated_at: doc_updated,
            title: Some(head_key),
            embed_model: None,
            embedded_at: None,
            status: STATUS_LEXICAL.to_string(),
            error: None,
            updated_at: 0,
        };
        match self
            .store
            .replace_meeting_chunks(&snap.meeting_id, &sources, &drafts, &state)
        {
            Ok(ids) => {
                if full {
                    // Alle alten Zeilen sind weg; Teilersetzungen laesst der
                    // Cache stehen (veraltete IDs filtert `rescore`).
                    self.cache
                        .update(|index| index.remove_meeting(&snap.meeting_id));
                }
                Ok(LexicalOutcome::Indexed(ids.len()))
            }
            Err(e) if is_stale(&e) => Ok(LexicalOutcome::Stale),
            Err(e) => Err(e),
        }
    }

    fn gate_closed(&self) -> Option<&'static str> {
        if self.gates.recording_active() {
            Some("recording_active")
        } else if self.gates.processing_active() {
            Some("processing_active")
        } else if self.gates.enhance_running() {
            Some("enhance_running")
        } else {
            None
        }
    }

    /// Darf die Vektorstufe jetzt grundsaetzlich laufen (ohne Gates)?
    fn vectors_allowed(&self, now: Instant) -> Result<(), &'static str> {
        if !self.gates.semantic_enabled() {
            return Err("disabled");
        }
        if !self.embed.available() {
            return Err("no_model");
        }
        if self.backoff_until.is_some_and(|until| now < until) {
            return Err("backoff");
        }
        Ok(())
    }

    fn release_if_active(&mut self) {
        if self.embedding_active {
            self.embed.release_now();
            self.embedding_active = false;
        }
    }

    fn fail(&mut self, now: Instant, code: &'static str) -> VectorStep {
        log::warn!("Indexer: Vektorstufe pausiert 10 min ({code})");
        self.stats.embed_errors += 1;
        self.last_error = Some(code);
        self.backoff_until = Some(now + BACKOFF);
        // Ein abgestuerzter oder knapper Server gibt seinen Speicher frei.
        self.embed.release_now();
        self.embedding_active = false;
        VectorStep::Error(code)
    }

    /// Chunks ohne Vektor (ohne die uebersprungenen), hoechstens `EMBED_BATCH`.
    fn next_batch(&self) -> Result<Vec<(i64, String)>> {
        let limit = (EMBED_BATCH + self.skip_chunks.len()) as u32;
        Ok(self
            .store
            .chunks_without_vectors(self.embed.model_id(), limit)?
            .into_iter()
            .filter(|(id, _)| !self.skip_chunks.contains(id))
            .take(EMBED_BATCH)
            .collect())
    }

    fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let result =
            tauri::async_runtime::block_on(self.embed.embed(texts, EmbedKind::Document))?;
        if result.len() != texts.len() {
            return Err(EmbedError::Failed("embedding_count_mismatch".into()));
        }
        Ok(result)
    }

    /// Speichert Vektoren und zieht den RAM-Cache nach.
    fn store_vectors(&mut self, rows: &[(i64, Vec<f32>)]) -> Result<usize> {
        let model = self.embed.model_id().to_string();
        let stored = self.store.put_vectors(&model, rows)?;
        let ids: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
        let owners: HashMap<i64, String> = self
            .store
            .get_chunks(&ids)?
            .into_iter()
            .map(|c| (c.id, c.meeting_id))
            .collect();
        let upserts: Vec<(i64, String, Vec<f32>)> = rows
            .iter()
            .filter_map(|(id, v)| owners.get(id).map(|m| (*id, m.clone(), normalized(v))))
            .collect();
        let upsert_failed = self
            .cache
            .update(|index| index.model() == model && index.upsert(&upserts).is_err())
            .unwrap_or(false);
        if upsert_failed {
            self.cache.invalidate();
        }
        self.stats.vectors_written += stored as u64;
        Ok(stored)
    }

    /// Eine Charge, die als Ganzes scheitert (`Failed`), wird einzeln
    /// wiederholt: ein einzelner zu langer Chunk soll nicht alle anderen
    /// blockieren. Einzeln scheiternde Chunks werden bis zum Neustart
    /// uebersprungen (die Besprechung bleibt `lexical`).
    fn embed_one_by_one(&mut self, batch: &[(i64, String)]) -> Result<usize, EmbedError> {
        let mut stored = 0usize;
        for (id, text) in batch {
            match self.embed_texts(std::slice::from_ref(text)) {
                Ok(mut v) => {
                    let row = (*id, v.pop().unwrap_or_default());
                    stored += self
                        .store_vectors(std::slice::from_ref(&row))
                        .map_err(|e| EmbedError::Failed(error_code(&e)))?;
                }
                Err(EmbedError::Failed(_)) => {
                    log::warn!("Indexer: Chunk {id} nicht einbettbar, uebersprungen");
                    self.skip_chunks.insert(*id);
                    self.stats.skipped_chunks += 1;
                }
                Err(other) => return Err(other),
            }
        }
        Ok(stored)
    }

    /// Hoechstens `max_batches` Chargen einbetten. Prueft vor jeder Charge die
    /// Gates; schliesst eines, wird der Server beendet und abgebrochen.
    pub fn run_vectors(&mut self, now: Instant, max_batches: usize) -> VectorStep {
        if !self.want_vectors {
            return VectorStep::Idle;
        }
        if let Err(reason) = self.vectors_allowed(now) {
            if reason != "backoff" {
                // Ohne Modell oder mit ausgeschalteter Einstellung wartet der
                // Indexer auf `EmbedPending` statt zu pollen.
                self.want_vectors = false;
            }
            self.release_if_active();
            return VectorStep::Blocked(reason);
        }
        let mut batches = 0usize;
        loop {
            if let Some(gate) = self.gate_closed() {
                self.release_if_active();
                return VectorStep::Blocked(gate);
            }
            if batches >= max_batches {
                self.after_slice();
                return VectorStep::Progress(batches);
            }
            if self.skip_chunks.len() > MAX_SKIPPED_CHUNKS {
                return self.fail(now, "too_many_skipped");
            }
            let batch = match self.next_batch() {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("Indexer: Chunks nicht lesbar ({})", error_code(&e));
                    return self.fail(now, "store_failed");
                }
            };
            if batch.is_empty() {
                self.after_slice();
                self.want_vectors = false;
                self.last_error = None;
                return VectorStep::Done;
            }
            self.embedding_active = true;
            let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
            let outcome = match self.embed_texts(&texts) {
                Ok(vectors) => {
                    let rows: Vec<(i64, Vec<f32>)> =
                        batch.iter().map(|(id, _)| *id).zip(vectors).collect();
                    self.store_vectors(&rows)
                        .map_err(|e| EmbedError::Failed(error_code(&e)))
                }
                Err(EmbedError::Failed(_)) if batch.len() > 1 => self.embed_one_by_one(&batch),
                Err(EmbedError::Failed(_)) => {
                    // Einzelner Chunk: ueberspringen statt ewig zu blockieren.
                    self.skip_chunks.insert(batch[0].0);
                    self.stats.skipped_chunks += 1;
                    Ok(0)
                }
                Err(e) => Err(e),
            };
            match outcome {
                Ok(_) => {
                    self.stats.embed_batches += 1;
                    batches += 1;
                }
                Err(e) => return self.fail(now, e.code()),
            }
        }
    }

    fn after_slice(&mut self) {
        if let Err(e) = self.store.mark_embedded(self.embed.model_id()) {
            log::warn!("Indexer: Status nicht gesetzt ({})", error_code(&e));
        }
        self.emit_progress();
    }

    fn emit_progress(&self) {
        if let Some(progress) = &self.progress {
            if let Ok(counts) = self.store.index_counts() {
                progress(counts.embedded, counts.meetings);
            }
        }
    }

    /// Leerlauf: Server nach `EMBED_IDLE_STOP` beenden, RAM-Cache nach 10 min.
    pub fn maintenance(&mut self) {
        self.embed.release_if_idle(EMBED_IDLE_STOP);
        if !self.embed.server_running() {
            self.embedding_active = false;
        }
        self.cache.drop_if_idle(IDLE_TTL);
    }

    /// Ein Durchlauf des Threads.
    pub fn tick(&mut self, now: Instant) {
        self.run_lexical(now);
        let _ = self.run_vectors(now, SLICE_BATCHES);
        self.maintenance();
    }

    /// Wie lange der Thread hoechstens auf den naechsten Auftrag wartet.
    pub fn next_wakeup(&self, now: Instant) -> Duration {
        if !self.queue.is_empty() {
            return Duration::ZERO;
        }
        let mut wait = TICK;
        if self.want_vectors
            && self.vectors_allowed(now).is_ok()
            && self.gate_closed().is_none()
        {
            return Duration::ZERO;
        }
        if self.want_vectors {
            if let Some(until) = self.backoff_until {
                wait = wait.min(until.saturating_duration_since(now));
            }
        }
        if let Some(next) = self.debounced.values().min() {
            wait = wait.min(next.saturating_duration_since(now));
        }
        wait
    }

    pub fn has_pending_work(&self) -> bool {
        !self.queue.is_empty() || !self.debounced.is_empty() || self.want_vectors
    }
}

// ---------------------------------------------------------------------------
// Thread
// ---------------------------------------------------------------------------

enum Msg {
    Job(IndexJob),
    Debounce(String),
}

#[derive(Default)]
struct Shared {
    busy: AtomicBool,
    last_error: Mutex<Option<String>>,
}

/// Der Indexer der App: ein Hintergrund-Thread mit `IndexerCore`.
pub struct MeetingIndexer {
    tx: Mutex<Sender<Msg>>,
    shared: Arc<Shared>,
}

impl MeetingIndexer {
    /// Startet den Thread mit dem globalen Vektor-Cache.
    pub fn spawn(
        store: Arc<MeetingStore>,
        embed: Arc<dyn Embedder>,
        gates: Arc<dyn IndexGates>,
    ) -> Self {
        Self::spawn_core(IndexerCore::new(store, embed, gates, global_cache()))
    }

    pub fn spawn_core(mut core: IndexerCore) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Msg>();
        let shared = Arc::new(Shared::default());
        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("meeting-indexer".into())
            .spawn(move || worker(&mut core, rx, &thread_shared))
            .expect("meeting indexer thread");
        Self {
            tx: Mutex::new(tx),
            shared,
        }
    }

    fn send(&self, msg: Msg) {
        let tx = self.tx.lock().unwrap_or_else(PoisonError::into_inner);
        if tx.send(msg).is_err() {
            log::warn!("Indexer: Thread beendet, Auftrag verworfen");
        }
    }

    pub fn submit(&self, job: IndexJob) {
        self.send(Msg::Job(job));
    }

    /// Nach `meeting_notes_save`: indexiert erst nach 30 s Ruhe.
    pub fn submit_debounced(&self, meeting_id: &str) {
        self.send(Msg::Debounce(meeting_id.to_string()));
    }

    pub fn busy(&self) -> bool {
        self.shared.busy.load(Ordering::Acquire)
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared
            .last_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn worker(core: &mut IndexerCore, rx: Receiver<Msg>, shared: &Shared) {
    let apply = |core: &mut IndexerCore, msg: Msg| {
        let now = Instant::now();
        match msg {
            Msg::Job(job) => core.handle(job, now),
            Msg::Debounce(id) => core.debounce(id, now),
        }
    };
    loop {
        let wait = core.next_wakeup(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(msg) => apply(core, msg),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(msg) = rx.try_recv() {
            apply(core, msg);
        }
        shared.busy.store(core.has_pending_work(), Ordering::Release);
        // Ein Fehler im Indexer darf die App nicht mitnehmen und den Thread
        // nicht beenden: Panik abfangen, Backoff, weiter.
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| core.tick(Instant::now())));
        if result.is_err() {
            log::error!("Indexer: Panik im Durchlauf abgefangen, Vektorstufe pausiert");
            core.backoff_until = Some(Instant::now() + BACKOFF);
            core.last_error = Some("panic");
        }
        shared.busy.store(core.has_pending_work(), Ordering::Release);
        *shared
            .last_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = core.last_error().map(str::to_string);
    }
    core.embed.release_now();
}

// ---------------------------------------------------------------------------
// Headless: `--reindex-meetings`
// ---------------------------------------------------------------------------

/// Ergebnis von `reindex_all` (JSON-Ausgabe des CLI-Laufs).
#[derive(Clone, Debug, Default, Serialize)]
pub struct ReindexReport {
    pub meetings: u32,
    pub indexed: u32,
    pub chunks: u32,
    pub vectors: u32,
    pub embedded_meetings: u32,
    pub lexical_meetings: u32,
    pub model: String,
    pub model_ready: bool,
    pub embed_error: Option<String>,
    pub embed_batches: u32,
    pub skipped_chunks: u32,
    pub embed_server_running: bool,
    pub embed_server_peak_mb: Option<u64>,
    pub lexical_ms: u64,
    pub embed_ms: u64,
}

/// Arbeitsspeicher (Working Set) eines Prozesses in MB.
fn process_rss_mb(pid: u32) -> Option<u64> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let pid = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    sys.process(pid).map(|p| p.memory() / (1024 * 1024))
}

/// Baut den Index synchron neu (optional nach Leeren) und bettet alles ein;
/// beendet den Embedding-Server am Ende. `server_pid` liefert die PID des
/// Servers fuer die Speichermessung (nur der echte Embedder hat eine).
pub fn reindex_all(
    store: Arc<MeetingStore>,
    embed: Arc<dyn Embedder>,
    cache: &'static VectorCache,
    clear: bool,
    server_pid: &dyn Fn() -> Option<u32>,
) -> Result<ReindexReport> {
    let started = Instant::now();
    if clear {
        store.clear_search_index()?;
        cache.invalidate();
    }
    let mut core = IndexerCore::new(store.clone(), embed.clone(), Arc::new(NoGates), cache);
    core.handle(IndexJob::Backfill, Instant::now());
    let indexed = core.run_lexical(Instant::now()) as u32;
    let lexical_ms = started.elapsed().as_millis() as u64;

    let embed_started = Instant::now();
    let model_ready = embed.available();
    let mut embed_error = None;
    let mut peak: Option<u64> = None;
    if model_ready {
        core.handle(IndexJob::EmbedPending, Instant::now());
        loop {
            let step = core.run_vectors(Instant::now(), 1);
            if let Some(mb) = server_pid().and_then(process_rss_mb) {
                peak = Some(peak.map_or(mb, |p| p.max(mb)));
            }
            match step {
                VectorStep::Progress(_) => continue,
                VectorStep::Done | VectorStep::Idle => break,
                VectorStep::Blocked(code) | VectorStep::Error(code) => {
                    embed_error = Some(code.to_string());
                    break;
                }
            }
        }
    }
    embed.release_now();
    let counts = store.index_counts()?;
    let model = embed.model_id().to_string();
    Ok(ReindexReport {
        meetings: counts.meetings,
        indexed,
        chunks: counts.chunks,
        vectors: store.vector_count(&model)?,
        embedded_meetings: counts.embedded,
        lexical_meetings: counts.lexical,
        model,
        model_ready,
        embed_error,
        embed_batches: core.stats().embed_batches,
        skipped_chunks: core.stats().skipped_chunks,
        embed_server_running: embed.server_running(),
        embed_server_peak_mb: peak,
        lexical_ms,
        embed_ms: embed_started.elapsed().as_millis() as u64,
    })
}

// ---------------------------------------------------------------------------
// Tauri-Anbindung
// ---------------------------------------------------------------------------

/// Fortschritt der Vektorstufe fuer die Einstellungszeile.
#[derive(Clone, Debug, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeetingIndexEvent {
    Progress { done: u32, total: u32 },
}

/// Gates der laufenden App.
struct AppIndexGates {
    app: tauri::AppHandle,
    /// Besprechungen im Status `processing` (Enddurchlauf, Import,
    /// Neu-Transkription), gefuehrt aus `MeetingEvent::State`. Bewusst nur im
    /// Speicher: eine nach einem Absturz haengengebliebene Zeile blockiert die
    /// Vektorstufe nicht fuer immer.
    processing: Arc<Mutex<HashSet<String>>>,
}

impl IndexGates for AppIndexGates {
    fn recording_active(&self) -> bool {
        use tauri::Manager;
        self.app
            .try_state::<Arc<super::super::recorder::MeetingRecorderManager>>()
            .is_some_and(|r| r.is_recording())
    }

    fn processing_active(&self) -> bool {
        !self
            .processing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    fn enhance_running(&self) -> bool {
        super::super::notes::enhance::enhance_running()
    }

    fn semantic_enabled(&self) -> bool {
        crate::settings::get_settings(&self.app).meeting_semantic_search
    }
}

/// Startet den Indexer der App (nach Store und Recorder), haengt ihn an die
/// Besprechungs-Ereignisse und stoesst das Nachholen an.
pub fn start_for_app(app: &tauri::AppHandle, store: Arc<MeetingStore>) {
    use super::super::recorder::MeetingEvent;
    use tauri::Manager;
    use tauri_specta::Event;

    let processing = Arc::new(Mutex::new(HashSet::new()));
    let gates = Arc::new(AppIndexGates {
        app: app.clone(),
        processing: processing.clone(),
    });
    let embed: Arc<dyn Embedder> = Arc::new(super::embed::LlamaEmbedder::new(
        crate::managers::llm::EMBED_MODEL_ID,
    ));
    let progress_app = app.clone();
    let core = IndexerCore::new(store, embed, gates, global_cache()).with_progress(Arc::new(
        move |done, total| {
            let _ = MeetingIndexEvent::Progress { done, total }.emit(&progress_app);
        },
    ));
    let indexer = Arc::new(MeetingIndexer::spawn_core(core));
    app.manage(indexer.clone());

    let listener = indexer.clone();
    MeetingEvent::listen_any(app, move |event| match event.payload {
        MeetingEvent::State {
            meeting_id, status, ..
        } => {
            let mut set = processing.lock().unwrap_or_else(PoisonError::into_inner);
            match status.as_str() {
                "processing" => {
                    set.insert(meeting_id);
                }
                // Import fertig, Neu-Transkription fertig, Aufnahme gestoppt
                // (Rueckfall ohne `TranscriptFinal`). P8a: auch `cancelled` (vom
                // Nutzer gestoppt): das Teil-Transkript ist durchsuchbar.
                "ready" | "cancelled" => {
                    set.remove(&meeting_id);
                    drop(set);
                    listener.submit(IndexJob::Meeting(meeting_id));
                }
                "failed" => {
                    set.remove(&meeting_id);
                }
                _ => {}
            }
        }
        MeetingEvent::TranscriptFinal { meeting_id, .. } => {
            listener.submit(IndexJob::Meeting(meeting_id));
        }
        _ => {}
    });
    indexer.submit(IndexJob::Backfill);
}

/// Reicht einen Auftrag an den Indexer der App (ohne Indexer: nichts).
pub fn submit(app: &tauri::AppHandle, job: IndexJob) {
    use tauri::Manager;
    if let Some(indexer) = app.try_state::<Arc<MeetingIndexer>>() {
        indexer.submit(job);
    }
}

/// `meeting_notes_save`: entprellt indexieren.
pub fn submit_debounced(app: &tauri::AppHandle, meeting_id: &str) {
    use tauri::Manager;
    if let Some(indexer) = app.try_state::<Arc<MeetingIndexer>>() {
        indexer.submit_debounced(meeting_id);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::index::tests::{ready_meeting, tmp_store};
    use super::super::index::STATUS_READY;
    use super::*;
    use crate::managers::meetings::notes::model::{
        EnhanceStats, EnhancedEntry, EnhancedSection, EntryFlags, NoteBlockKind, Origin,
        SectionKind,
    };
    use crate::managers::meetings::store::{Meeting, TranscriptDelta};
    use futures_util::future::BoxFuture;
    use std::sync::atomic::AtomicUsize;

    const MODEL: &str = "fake-emb";
    const DIM: usize = 8;

    // ---- Fakes -----------------------------------------------------------

    #[derive(Default)]
    struct FakeGates {
        recording: AtomicBool,
        processing: AtomicBool,
        enhance: AtomicBool,
        disabled: AtomicBool,
    }

    impl IndexGates for FakeGates {
        fn recording_active(&self) -> bool {
            self.recording.load(Ordering::SeqCst)
        }
        fn processing_active(&self) -> bool {
            self.processing.load(Ordering::SeqCst)
        }
        fn enhance_running(&self) -> bool {
            self.enhance.load(Ordering::SeqCst)
        }
        fn semantic_enabled(&self) -> bool {
            !self.disabled.load(Ordering::SeqCst)
        }
    }

    struct FakeEmbedder {
        available: AtomicBool,
        calls: AtomicUsize,
        texts_seen: AtomicUsize,
        releases: AtomicUsize,
        idle_checks: AtomicUsize,
        /// Jeder Aufruf scheitert mit diesem Fehler.
        fail_with: Mutex<Option<EmbedError>>,
        /// Chargen mit diesem Text scheitern (`Failed`).
        poison: Mutex<Option<String>>,
        /// Nach dem n-ten Aufruf schliesst dieses Gate.
        close_gate_after: Mutex<Option<(usize, Arc<FakeGates>)>>,
    }

    impl FakeEmbedder {
        fn new() -> Self {
            Self {
                available: AtomicBool::new(true),
                calls: AtomicUsize::new(0),
                texts_seen: AtomicUsize::new(0),
                releases: AtomicUsize::new(0),
                idle_checks: AtomicUsize::new(0),
                fail_with: Mutex::new(None),
                poison: Mutex::new(None),
                close_gate_after: Mutex::new(None),
            }
        }
    }

    fn vector_for(text: &str) -> Vec<f32> {
        let mut v = vec![0.1f32; DIM];
        for (i, b) in text.bytes().enumerate() {
            v[i % DIM] += f32::from(b) / 255.0;
        }
        v
    }

    impl Embedder for FakeEmbedder {
        fn embed<'a>(
            &'a self,
            texts: &'a [String],
            kind: EmbedKind,
        ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>> {
            Box::pin(async move {
                assert_eq!(kind, EmbedKind::Document);
                let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
                if let Some((after, gates)) = self.close_gate_after.lock().unwrap().as_ref() {
                    if n >= *after {
                        gates.recording.store(true, Ordering::SeqCst);
                    }
                }
                if let Some(err) = self.fail_with.lock().unwrap().clone() {
                    return Err(err);
                }
                if let Some(p) = self.poison.lock().unwrap().as_ref() {
                    if texts.iter().any(|t| t.contains(p.as_str())) {
                        return Err(EmbedError::Failed("input too large".into()));
                    }
                }
                self.texts_seen.fetch_add(texts.len(), Ordering::SeqCst);
                Ok(texts.iter().map(|t| vector_for(t)).collect())
            })
        }
        fn model_id(&self) -> &str {
            MODEL
        }
        fn available(&self) -> bool {
            self.available.load(Ordering::SeqCst)
        }
        fn release_if_idle(&self, idle: Duration) {
            assert_eq!(idle, EMBED_IDLE_STOP);
            self.idle_checks.fetch_add(1, Ordering::SeqCst);
        }
        fn release_now(&self) {
            self.releases.fetch_add(1, Ordering::SeqCst);
        }
    }

    // ---- Aufbau ----------------------------------------------------------

    struct Rig {
        _dir: tempfile::TempDir,
        store: Arc<MeetingStore>,
        embed: Arc<FakeEmbedder>,
        gates: Arc<FakeGates>,
        cache: &'static VectorCache,
        core: IndexerCore,
    }

    fn rig() -> Rig {
        let (dir, store) = tmp_store();
        let store = Arc::new(store);
        let embed = Arc::new(FakeEmbedder::new());
        let gates = Arc::new(FakeGates::default());
        let cache: &'static VectorCache = Box::leak(Box::new(VectorCache::new()));
        let core = IndexerCore::new(store.clone(), embed.clone(), gates.clone(), cache);
        Rig {
            _dir: dir,
            store,
            embed,
            gates,
            cache,
            core,
        }
    }

    fn seg(i: u32, text: &str) -> StoredSegment {
        StoredSegment {
            segment_index: i,
            text: text.into(),
            start_ms: u64::from(i) * 5_000,
            end_ms: u64::from(i) * 5_000 + 4_000,
            channel: (i % 2) as u8,
            speaker_index: None,
            words: None,
        }
    }

    /// Fertige Besprechung mit langem Transkript (mehrere Chunks) und Notizen.
    fn meeting_with_content(store: &MeetingStore, title: &str) -> Meeting {
        let m = ready_meeting(store, title, 1_750_000_000);
        let segments: Vec<StoredSegment> = (0..40)
            .map(|i| {
                seg(
                    i,
                    &format!("Satz {i}: Wir besprechen das Budget fuer die Kundenbetreuung und den Zeitplan im Oktober."),
                )
            })
            .collect();
        store
            .append_delta(
                &m.id,
                &TranscriptDelta {
                    new_segments: segments,
                },
            )
            .unwrap();
        store
            .save_notes(
                &m.id,
                &[
                    NoteBlock {
                        id: "b1".into(),
                        kind: NoteBlockKind::Heading,
                        text: "Offene Punkte".into(),
                        at_ms: Some(1_000),
                        checked: false,
                    },
                    NoteBlock {
                        id: "b2".into(),
                        kind: NoteBlockKind::Bullet,
                        text: "Angebot Zeppelinstrasse pruefen".into(),
                        at_ms: Some(2_000),
                        checked: false,
                    },
                ],
                0,
            )
            .unwrap();
        m
    }

    fn add_enhanced(store: &MeetingStore, meeting_id: &str, text: &str) -> String {
        let notes = EnhancedNotes {
            format: DOC_FORMAT.into(),
            template_id: None,
            template_title: "Allgemein".into(),
            segment_epoch: 0,
            sections: vec![EnhancedSection {
                id: "decisions".into(),
                title: "Entscheidungen".into(),
                kind: SectionKind::Text,
                entries: vec![EnhancedEntry {
                    id: "E1".into(),
                    origin: Origin::Ai,
                    text: text.into(),
                    note_id: None,
                    source_segment_ids: vec![1, 2],
                    assignee: None,
                    due: None,
                    flags: EntryFlags::default(),
                }],
            }],
            stats: EnhanceStats::default(),
        };
        store
            .insert_document(
                meeting_id,
                "enhanced_notes",
                DOC_FORMAT,
                &serde_json::to_string(&notes).unwrap(),
                None,
                None,
            )
            .unwrap()
    }

    fn count(store: &MeetingStore, sql: &str) -> i64 {
        store
            .get_connection()
            .unwrap()
            .query_row(sql, [], |r| r.get(0))
            .unwrap()
    }

    fn sources_of(store: &MeetingStore, meeting_id: &str) -> Vec<String> {
        let conn = store.get_connection().unwrap();
        let mut stmt = conn
            .prepare("SELECT DISTINCT source FROM meeting_chunks WHERE meeting_id = ?1 ORDER BY source")
            .unwrap();
        let rows = stmt
            .query_map(params![meeting_id], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows
    }

    fn chunks(store: &MeetingStore) -> i64 {
        count(store, "SELECT COUNT(*) FROM meeting_chunks")
    }

    fn vectors(store: &MeetingStore) -> i64 {
        count(store, "SELECT COUNT(*) FROM meeting_chunk_vectors")
    }

    fn status(store: &MeetingStore, id: &str) -> String {
        store.index_state(id).unwrap().unwrap().status
    }

    fn run_all_vectors(core: &mut IndexerCore, now: Instant) -> VectorStep {
        loop {
            match core.run_vectors(now, 4) {
                VectorStep::Progress(_) => continue,
                other => return other,
            }
        }
    }

    // ---- Tests -----------------------------------------------------------

    #[test]
    fn lexical_index_covers_title_transcript_notes_and_ai_notes() {
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Kundentermin Meyer");
        add_enhanced(&r.store, &m.id, "Lieferung per Spedition Nordlicht");
        let t0 = Instant::now();
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        assert_eq!(r.core.run_lexical(t0), 1);
        assert_eq!(
            sources_of(&r.store, &m.id),
            vec!["ai_notes", "title", "transcript", "user_notes"]
        );
        assert!(chunks(&r.store) >= 4, "langes Transkript ergibt mehrere Chunks");
        assert_eq!(status(&r.store, &m.id), STATUS_LEXICAL);
        // Titel, Notiz und KI-Notiz sind per Wortsuche findbar.
        for word in ["meyer", "zeppelinstrasse", "spedition"] {
            let hits = r
                .store
                .search_words(&format!("\"{word}\""), &[m.id.clone()], 10)
                .unwrap();
            assert!(!hits.is_empty(), "{word} nicht gefunden");
        }
        // Zweiter Auftrag ohne Aenderung: nichts neu.
        let before = chunks(&r.store);
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        assert_eq!(r.core.run_lexical(t0), 0);
        assert_eq!(chunks(&r.store), before);
    }

    /// U7: die Beschreibung ist durchsuchbar; jede Aenderung baut den Index neu.
    #[test]
    fn the_description_is_searchable_and_every_change_rebuilds_the_index() {
        use crate::managers::meetings::metadata::MetadataEdit;
        use crate::managers::meetings::search::index::MeetingFilter;
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Kundentermin Meyer");
        let t0 = Instant::now();
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        assert_eq!(r.core.run_lexical(t0), 1);
        let find = |r: &Rig, query: &str| {
            r.store
                .search_meetings(query, &MeetingFilter::default(), 0, 25)
                .unwrap()
        };
        assert_eq!(find(&r, "Lindner").total, 0, "noch keine Beschreibung");

        let describe = |r: &Rig, text: &str| {
            r.store
                .update_metadata(
                    &m.id,
                    &MetadataEdit {
                        description: Some(text.into()),
                        ..Default::default()
                    },
                )
                .unwrap();
        };
        describe(&r, "Thema: Zeppelinstrasse 12
Ansprechpartnerin Frau Lindner");
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        assert_eq!(r.core.run_lexical(t0), 1, "die Beschreibung aendert den Kopf: Neuaufbau");
        let page = find(&r, "lindner");
        assert_eq!(page.total, 1, "die Beschreibung ist durchsuchbar (Teilwort, Kleinschreibung)");
        assert_eq!(page.items[0].meeting.id, m.id);
        assert_eq!(page.items[0].hit_source, Some(ChunkSource::Title), "Treffer steht im Kopf-Chunk");
        assert_eq!(find(&r, "zeppelinstrasse").total, 1);
        assert_eq!(find(&r, "budget").total, 1, "das Transkript bleibt auffindbar");

        // Ohne Aenderung nichts Neues.
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        assert_eq!(r.core.run_lexical(t0), 0);

        // Anderer Text: der alte Begriff verschwindet, der neue ist da.
        describe(&r, "Jetzt geht es um die Lieferung");
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        assert_eq!(r.core.run_lexical(t0), 1);
        assert_eq!(find(&r, "lindner").total, 0);
        assert_eq!(find(&r, "lieferung").total, 1);

        // Leere Beschreibung: zurueck auf den Stand ohne Beschreibung.
        describe(&r, "");
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        assert_eq!(r.core.run_lexical(t0), 1);
        assert_eq!(find(&r, "lieferung").total, 0);
        assert_eq!(find(&r, "meyer").total, 1, "der Titel bleibt");
    }

    #[test]
    fn recording_gate_blocks_vectors_until_it_opens() {
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Planung");
        let t0 = Instant::now();
        r.gates.recording.store(true, Ordering::SeqCst);
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        r.core.tick(t0);
        assert!(chunks(&r.store) > 0, "lexikalisch laeuft trotz Aufnahme");
        assert_eq!(vectors(&r.store), 0, "keine Vektoren waehrend der Aufnahme");
        assert_eq!(r.embed.calls.load(Ordering::SeqCst), 0, "Server nie gefragt");
        assert_eq!(r.core.next_wakeup(t0), TICK, "Gate: pollt im Takt, kein Heisslauf");

        r.gates.recording.store(false, Ordering::SeqCst);
        assert_eq!(run_all_vectors(&mut r.core, t0), VectorStep::Done);
        assert_eq!(vectors(&r.store), chunks(&r.store));
        assert_eq!(status(&r.store, &m.id), STATUS_READY);
    }

    #[test]
    fn processing_and_enhance_gates_block_vectors_too() {
        for gate in ["processing", "enhance"] {
            let mut r = rig();
            let m = meeting_with_content(&r.store, "Runde");
            let t0 = Instant::now();
            match gate {
                "processing" => r.gates.processing.store(true, Ordering::SeqCst),
                _ => r.gates.enhance.store(true, Ordering::SeqCst),
            }
            r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
            r.core.run_lexical(t0);
            let step = r.core.run_vectors(t0, 8);
            assert!(matches!(step, VectorStep::Blocked(_)), "{gate}: {step:?}");
            assert_eq!(vectors(&r.store), 0, "{gate}");
            assert_eq!(r.embed.calls.load(Ordering::SeqCst), 0, "{gate}");
            assert_eq!(status(&r.store, &m.id), STATUS_LEXICAL, "{gate}");
        }
    }

    #[test]
    fn a_gate_closing_mid_run_stops_after_the_batch_and_releases_the_server() {
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Lange Runde");
        let t0 = Instant::now();
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        r.core.run_lexical(t0);
        *r.embed.close_gate_after.lock().unwrap() = Some((1, r.gates.clone()));
        // Der Aufbau muss mehr als eine Charge haben.
        let total = chunks(&r.store);
        assert!(total > 0);
        // 17+ Chunks erzwingen: mehr Besprechungen.
        for i in 0..3 {
            let other = meeting_with_content(&r.store, &format!("Weitere {i}"));
            r.core.handle(IndexJob::Meeting(other.id), t0);
        }
        r.core.run_lexical(t0);
        assert!(chunks(&r.store) as usize > EMBED_BATCH);
        let step = r.core.run_vectors(t0, 8);
        assert_eq!(step, VectorStep::Blocked("recording_active"));
        assert_eq!(vectors(&r.store) as usize, EMBED_BATCH, "genau die laufende Charge");
        assert_eq!(r.embed.releases.load(Ordering::SeqCst), 1, "Server sofort beendet");
        // Aufnahme vorbei: der Rest wird nachgeholt.
        r.gates.recording.store(false, Ordering::SeqCst);
        *r.embed.close_gate_after.lock().unwrap() = None;
        assert_eq!(run_all_vectors(&mut r.core, t0), VectorStep::Done);
        assert_eq!(vectors(&r.store), chunks(&r.store));
    }

    #[test]
    fn setting_off_or_missing_model_keeps_lexical_without_starting_a_server() {
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Ohne Modell");
        let t0 = Instant::now();
        r.embed.available.store(false, Ordering::SeqCst);
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        r.core.tick(t0);
        assert_eq!(r.embed.calls.load(Ordering::SeqCst), 0);
        assert_eq!(status(&r.store, &m.id), STATUS_LEXICAL);
        assert!(!r.core.has_pending_work(), "wartet auf EmbedPending, pollt nicht");

        // Modell geladen, aber Einstellung aus: weiterhin nichts.
        r.embed.available.store(true, Ordering::SeqCst);
        r.gates.disabled.store(true, Ordering::SeqCst);
        r.core.handle(IndexJob::EmbedPending, t0);
        assert_eq!(r.core.run_vectors(t0, 8), VectorStep::Blocked("disabled"));
        assert_eq!(r.embed.calls.load(Ordering::SeqCst), 0);

        // Einstellung an + EmbedPending: jetzt Vektoren.
        r.gates.disabled.store(false, Ordering::SeqCst);
        r.core.handle(IndexJob::EmbedPending, t0);
        assert_eq!(run_all_vectors(&mut r.core, t0), VectorStep::Done);
        assert_eq!(status(&r.store, &m.id), STATUS_READY);
    }

    #[test]
    fn a_changed_epoch_reindexes_the_transcript_and_keeps_other_vectors() {
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Neu transkribiert");
        let t0 = Instant::now();
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        r.core.run_lexical(t0);
        assert_eq!(run_all_vectors(&mut r.core, t0), VectorStep::Done);
        let notes_before = count(
            &r.store,
            "SELECT MIN(id) FROM meeting_chunks WHERE source = 'user_notes'",
        );

        // Neu-Transkription: Epoche + 1, neue Segmente.
        r.store.clear_segments(&m.id).unwrap();
        r.store
            .append_delta(
                &m.id,
                &TranscriptDelta {
                    new_segments: vec![seg(0, "Ganz neuer Inhalt ueber Photovoltaik")],
                },
            )
            .unwrap();
        assert_eq!(r.store.stale_meetings(10).unwrap(), vec![m.id.clone()]);
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        assert_eq!(r.core.run_lexical(t0), 1);
        let state = r.store.index_state(&m.id).unwrap().unwrap();
        assert_eq!(state.transcript_epoch, Some(1));
        assert_eq!(state.status, STATUS_LEXICAL);
        assert!(r.store.stale_meetings(10).unwrap().is_empty());
        assert_eq!(
            count(&r.store, "SELECT COUNT(*) FROM meeting_chunks WHERE source = 'transcript'"),
            1
        );
        assert_eq!(
            count(&r.store, "SELECT MIN(id) FROM meeting_chunks WHERE source = 'user_notes'"),
            notes_before,
            "Notizen-Chunks unveraendert (ihre Vektoren bleiben)"
        );
        let embedded_before = r.embed.texts_seen.load(Ordering::SeqCst);
        assert_eq!(run_all_vectors(&mut r.core, t0), VectorStep::Done);
        assert_eq!(
            r.embed.texts_seen.load(Ordering::SeqCst) - embedded_before,
            1,
            "nur der neue Transkript-Chunk wird eingebettet"
        );
        assert_eq!(vectors(&r.store), chunks(&r.store));
    }

    #[test]
    fn a_source_changing_between_read_and_write_is_requeued() {
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Wettlauf");
        let snapshot = r.store.index_snapshot(&m.id).unwrap().unwrap();
        // Waehrend der Indexer rechnet, laeuft eine Neu-Transkription.
        r.store.clear_segments(&m.id).unwrap();
        assert_eq!(
            r.core.write_snapshot(&snapshot).unwrap(),
            LexicalOutcome::Stale,
            "veralteter Stand wird nicht geschrieben"
        );
        assert_eq!(chunks(&r.store), 0);
        // Ueber die Queue: Stale -> neu eingereiht -> naechster Versuch liest frisch.
        let t0 = Instant::now();
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        assert_eq!(r.core.run_lexical(t0), 1);
        assert_eq!(
            r.store.index_state(&m.id).unwrap().unwrap().transcript_epoch,
            Some(1)
        );
    }

    #[test]
    fn deleting_removes_chunks_vectors_and_cache_rows() {
        let mut r = rig();
        let keep = meeting_with_content(&r.store, "Bleibt");
        let gone = meeting_with_content(&r.store, "Geht");
        let t0 = Instant::now();
        r.core.handle(IndexJob::Backfill, t0);
        assert_eq!(r.core.run_lexical(t0), 2);
        assert_eq!(run_all_vectors(&mut r.core, t0), VectorStep::Done);
        // Cache laden, damit auch der RAM-Teil geprueft wird.
        let loaded = r
            .cache
            .with_index(&r.store, MODEL, |i| i.len())
            .unwrap();
        assert_eq!(loaded as i64, vectors(&r.store));

        r.store.soft_delete_meeting(&gone.id).unwrap();
        r.core.handle(IndexJob::Deleted(gone.id.clone()), t0);
        assert_eq!(
            count(&r.store, &format!("SELECT COUNT(*) FROM meeting_chunks WHERE meeting_id = '{}'", gone.id)),
            0
        );
        assert_eq!(vectors(&r.store), chunks(&r.store), "keine Waisen-Vektoren");
        let after = r.cache.with_index(&r.store, MODEL, |i| i.len()).unwrap();
        assert_eq!(after as i64, vectors(&r.store), "Cache ohne die geloeschte Besprechung");
        assert!(r.store.index_state(&gone.id).unwrap().is_none());
        assert_eq!(status(&r.store, &keep.id), STATUS_READY);
        // Ein spaeter eintreffender Auftrag fuer die geloeschte Besprechung ist harmlos.
        r.core.handle(IndexJob::Meeting(gone.id.clone()), t0);
        assert_eq!(r.core.run_lexical(t0), 0);
    }

    #[test]
    fn an_embedding_error_backs_off_for_ten_minutes() {
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Knapper Speicher");
        let t0 = Instant::now();
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        r.core.run_lexical(t0);
        *r.embed.fail_with.lock().unwrap() = Some(EmbedError::MemoryLow);
        assert_eq!(r.core.run_vectors(t0, 8), VectorStep::Error("memory_low"));
        assert_eq!(r.core.last_error(), Some("memory_low"));
        assert_eq!(r.embed.calls.load(Ordering::SeqCst), 1);
        assert!(r.embed.releases.load(Ordering::SeqCst) >= 1, "Speicher freigeben");
        assert_eq!(status(&r.store, &m.id), STATUS_LEXICAL, "bleibt lexikalisch nutzbar");

        // Kein Retry-Sturm: 9 min spaeter wird nicht erneut gefragt.
        *r.embed.fail_with.lock().unwrap() = None;
        let t9 = t0 + Duration::from_secs(9 * 60);
        assert_eq!(r.core.run_vectors(t9, 8), VectorStep::Blocked("backoff"));
        assert_eq!(r.embed.calls.load(Ordering::SeqCst), 1);
        assert!(r.core.next_wakeup(t9) <= Duration::from_secs(60));
        assert!(r.core.next_wakeup(t9) > Duration::ZERO);

        // Nach 10 min: neuer Versuch, erfolgreich.
        let t10 = t0 + BACKOFF + Duration::from_secs(1);
        assert_eq!(r.core.next_wakeup(t10), Duration::ZERO);
        assert_eq!(run_all_vectors(&mut r.core, t10), VectorStep::Done);
        assert_eq!(status(&r.store, &m.id), STATUS_READY);
        assert_eq!(r.core.last_error(), None);
    }

    #[test]
    fn embed_pending_lifts_the_backoff_after_a_download() {
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Download");
        let t0 = Instant::now();
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        r.core.run_lexical(t0);
        *r.embed.fail_with.lock().unwrap() = Some(EmbedError::NoModel);
        assert_eq!(r.core.run_vectors(t0, 8), VectorStep::Error("no_model"));
        *r.embed.fail_with.lock().unwrap() = None;
        r.core.handle(IndexJob::EmbedPending, t0);
        assert_eq!(run_all_vectors(&mut r.core, t0), VectorStep::Done);
    }

    #[test]
    fn notes_saves_are_debounced_for_thirty_seconds() {
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Notizen");
        let t0 = Instant::now();
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        r.core.run_lexical(t0);
        assert_eq!(run_all_vectors(&mut r.core, t0), VectorStep::Done);
        let rev = r.store.get_notes(&m.id).unwrap().revision;
        let save = |text: &str, base: u64| {
            r.store
                .save_notes(
                    &m.id,
                    &[NoteBlock {
                        id: "b9".into(),
                        kind: NoteBlockKind::Paragraph,
                        text: text.into(),
                        at_ms: None,
                        checked: false,
                    }],
                    base,
                )
                .unwrap()
        };
        let rev = save("Erster Stand", rev);
        r.core.debounce(m.id.clone(), t0);
        let t20 = t0 + Duration::from_secs(20);
        let rev = save("Zweiter Stand Walfisch", rev);
        r.core.debounce(m.id.clone(), t20);
        assert_eq!(r.core.run_lexical(t0 + Duration::from_secs(31)), 0, "20 s nach dem letzten Speichern");
        let wait = r.core.next_wakeup(t0 + Duration::from_secs(31));
        assert!(wait <= Duration::from_secs(19) && wait > Duration::ZERO, "{wait:?}");
        assert_eq!(r.core.run_lexical(t20 + NOTES_DEBOUNCE), 1);
        let state = r.store.index_state(&m.id).unwrap().unwrap();
        assert_eq!(state.notes_revision, Some(rev));
        let hits = r
            .store
            .search_words("\"walfisch\"", &[m.id.clone()], 10)
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn a_poison_chunk_is_skipped_and_the_rest_is_embedded() {
        let mut r = rig();
        let m = meeting_with_content(&r.store, "Gift");
        add_enhanced(&r.store, &m.id, "GIFTIG unbrauchbarer Eintrag");
        let t0 = Instant::now();
        r.core.handle(IndexJob::Meeting(m.id.clone()), t0);
        r.core.run_lexical(t0);
        *r.embed.poison.lock().unwrap() = Some("GIFTIG".into());
        assert_eq!(run_all_vectors(&mut r.core, t0), VectorStep::Done);
        assert_eq!(r.core.stats().skipped_chunks, 1);
        assert_eq!(vectors(&r.store), chunks(&r.store) - 1);
        assert_eq!(
            status(&r.store, &m.id),
            STATUS_LEXICAL,
            "ehrlich: nicht vollstaendig eingebettet"
        );
    }

    #[test]
    fn backfill_indexes_every_stale_meeting_and_idle_maintenance_runs() {
        let mut r = rig();
        let ids: Vec<String> = (0..3)
            .map(|i| meeting_with_content(&r.store, &format!("Besprechung {i}")).id)
            .collect();
        // Nicht fertig: wird nicht indexiert.
        let open = r
            .store
            .create_meeting("Laeuft noch", crate::managers::meetings::store::MeetingSource::Live, Some(1))
            .unwrap();
        let t0 = Instant::now();
        r.core.handle(IndexJob::Backfill, t0);
        r.core.tick(t0);
        while r.core.next_wakeup(t0) == Duration::ZERO {
            r.core.tick(t0);
        }
        for id in &ids {
            assert_eq!(status(&r.store, id), STATUS_READY);
        }
        assert!(r.store.index_state(&open.id).unwrap().is_none());
        assert_eq!(vectors(&r.store), chunks(&r.store));
        assert!(r.embed.idle_checks.load(Ordering::SeqCst) >= 1, "Leerlauf-Stopp geprueft");
        assert!(r.store.stale_meetings(10).unwrap().is_empty());
    }

    #[test]
    fn reindex_all_rebuilds_and_ends_with_the_server_stopped() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let a = meeting_with_content(&store, "A");
        let b = meeting_with_content(&store, "B");
        let embed = Arc::new(FakeEmbedder::new());
        let cache: &'static VectorCache = Box::leak(Box::new(VectorCache::new()));
        let report = reindex_all(store.clone(), embed.clone(), cache, true, &|| None).unwrap();
        assert_eq!(report.meetings, 2);
        assert_eq!(report.indexed, 2);
        assert!(report.chunks > 0);
        assert_eq!(report.vectors, report.chunks);
        assert_eq!(report.embedded_meetings, 2);
        assert!(!report.embed_server_running);
        assert!(embed.releases.load(Ordering::SeqCst) >= 1, "Server am Ende beendet");
        for id in [&a.id, &b.id] {
            assert_eq!(status(&store, id), STATUS_READY);
        }
        // Zweiter Lauf mit Leeren: gleiches Ergebnis, nichts doppelt.
        let again = reindex_all(store.clone(), embed, cache, true, &|| None).unwrap();
        assert_eq!(again.chunks, report.chunks);
        assert_eq!(again.vectors, report.chunks);
    }

    #[test]
    fn the_thread_indexes_submitted_jobs() {
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let m = meeting_with_content(&store, "Thread");
        let cache: &'static VectorCache = Box::leak(Box::new(VectorCache::new()));
        let core = IndexerCore::new(
            store.clone(),
            Arc::new(FakeEmbedder::new()),
            Arc::new(FakeGates::default()),
            cache,
        );
        let indexer = MeetingIndexer::spawn_core(core);
        indexer.submit(IndexJob::Meeting(m.id.clone()));
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if store
                .index_state(&m.id)
                .unwrap()
                .is_some_and(|s| s.status == STATUS_READY)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(status(&store, &m.id), STATUS_READY);
        assert_eq!(indexer.last_error(), None);
    }

    /// G1 (#70): ein leerer Eintrag (nur Notizen, kein Transkript) wird ohne
    /// Fehler indexiert, ist ueber seine Notizen und seinen Titel auffindbar und
    /// faellt aus der Liste, sobald er geloescht ist.
    #[test]
    fn an_empty_entry_is_indexed_and_found_by_its_notes_and_never_errors() {
        use crate::managers::meetings::search::index::MeetingFilter;
        let (_dir, store) = tmp_store();
        let store = Arc::new(store);
        let folder = store.folder_save(None, "Kunde", None).unwrap().id;
        let empty = store
            .create_empty_meeting("Neue Besprechung", Some(&folder))
            .unwrap();
        store
            .save_notes(
                &empty.id,
                &[NoteBlock {
                    id: "n1".into(),
                    kind: NoteBlockKind::Bullet,
                    text: "Zeppelinstrasse Angebot nachfassen".into(),
                    at_ms: None,
                    checked: false,
                }],
                0,
            )
            .unwrap();
        let embed = Arc::new(FakeEmbedder::new());
        let cache: &'static VectorCache = Box::leak(Box::new(VectorCache::new()));
        let report = reindex_all(store.clone(), embed, cache, false, &|| None).unwrap();
        assert_eq!(report.meetings, 1);
        assert_eq!(report.indexed, 1, "der leere Eintrag wird indexiert");
        let found = store
            .search_meetings("Zeppelinstrasse", &MeetingFilter::default(), 0, 25)
            .unwrap();
        assert_eq!(found.items.len(), 1);
        assert_eq!(found.items[0].meeting.id, empty.id);
        assert_eq!(found.items[0].meeting.source, "empty");
        // Nach Projekt gefiltert und als reine Liste (ohne Suchwort) steht er auch da.
        let in_project = store
            .search_meetings(
                "",
                &MeetingFilter {
                    folder_id: Some(folder),
                    ..MeetingFilter::default()
                },
                0,
                25,
            )
            .unwrap();
        assert_eq!(in_project.items.len(), 1);
        // Geloescht: weg aus der Suche.
        store.soft_delete_meeting(&empty.id).unwrap();
        let gone = store
            .search_meetings("Zeppelinstrasse", &MeetingFilter::default(), 0, 25)
            .unwrap();
        assert!(gone.items.is_empty());
    }
}
