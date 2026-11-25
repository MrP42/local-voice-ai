//! lva#809: ein Hoerbuch-Lauf friert Modell und Sprache fuer alle Teile ein.
//! Kein Download, kein Rueckgriff auf die Besprechungssprache. Altauftraege ohne
//! Laufkontext behalten ihr Modell und erhalten Metadatensprache oder auto.

use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Manager;

use super::store::MeetingStore;
use crate::managers::model::ModelManager;
use crate::managers::transcription::{LanguageOverride, TranscriptionManager};

pub const DEFAULT_MODEL: &str =
    "handy-computer/whisper-large-v3-turbo-gguf/whisper-large-v3-turbo-Q8_0.gguf";

pub(super) const AUDIOBOOK_MIGRATION: &str = "CREATE TABLE audiobook_runs (
    id TEXT PRIMARY KEY, book_id TEXT NOT NULL, requested_model_id TEXT,
    effective_model_id TEXT NOT NULL, language_mode TEXT NOT NULL,
    language TEXT NOT NULL, language_source TEXT NOT NULL, language_state TEXT NOT NULL,
    model_fallback_reason TEXT);
    ALTER TABLE import_part ADD COLUMN audiobook_run_id TEXT REFERENCES audiobook_runs(id);
    CREATE INDEX idx_import_part_audiobook_run ON import_part(audiobook_run_id);
    CREATE TABLE audiobook_run_jobs (
        meeting_id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES audiobook_runs(id));
    CREATE TRIGGER audiobook_run_jobs_insert AFTER INSERT ON import_part
    WHEN NEW.audiobook_run_id IS NOT NULL BEGIN
        INSERT OR REPLACE INTO audiobook_run_jobs VALUES (NEW.meeting_id, NEW.audiobook_run_id);
    END;
    CREATE TRIGGER audiobook_run_jobs_update AFTER UPDATE OF audiobook_run_id ON import_part BEGIN
        DELETE FROM audiobook_run_jobs WHERE meeting_id=NEW.meeting_id;
        INSERT INTO audiobook_run_jobs SELECT NEW.meeting_id, NEW.audiobook_run_id
            WHERE NEW.audiobook_run_id IS NOT NULL;
    END;";

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct AudiobookRun {
    pub id: String,
    pub book_id: String,
    pub requested_model_id: Option<String>,
    pub effective_model_id: String,
    pub language_mode: String,
    pub language: String,
    pub language_source: String,
    pub language_state: String,
    pub model_fallback_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct AudiobookRunJobInfo {
    pub meeting_id: String,
    pub run_id: String,
    pub effective_model_id: String,
    pub language: String,
    pub language_source: String,
    pub language_state: String,
    pub model_fallback_reason: Option<String>,
    pub worker_code: Option<String>,
}

/// Buchmetadaten duerfen Namen und ISO-639-2 enthalten. Unbekanntes ist keine Sprache.
pub fn normalize_language(raw: &str) -> Option<String> {
    let lower = raw.trim().to_lowercase();
    let alias = match lower.as_str() {
        "ger" | "deu" => "de",
        "eng" => "en",
        "fre" | "fra" => "fr",
        "spa" => "es",
        "ita" => "it",
        "dut" | "nld" => "nl",
        "por" => "pt",
        _ => lower.as_str(),
    };
    if let Some((code, _, _)) = super::language::LANGUAGES.iter().find(|(code, en, de)| {
        *code == alias || en.to_lowercase() == lower || de.to_lowercase() == lower
    }) {
        return Some((*code).to_string());
    }
    // Sprachcodes mit Region (de-DE), aber keine frei erfundenen Codes.
    let code = super::language::normalize_code(alias)?;
    super::language::LANGUAGES
        .iter()
        .any(|(c, _, _)| *c == code)
        .then_some(code)
}

/// Die feste Auswahl kennt auch die weiteren Whisper-Codes und Schriftvarianten
/// der bestehenden Sprachauswahl. Fuer Buchmetadaten gilt die strengere Normalisierung.
pub fn fixed_language(raw: &str) -> Option<String> {
    match raw.trim() {
        "zh-Hans" | "zh-Hant" => Some(raw.trim().to_string()),
        _ => normalize_language(raw).or_else(|| super::language::normalize_code(raw)),
    }
}

pub fn language_intent(
    setting: &str,
    metadata: Option<&str>,
) -> Result<(String, String, String), String> {
    let (language, source) = if setting != "auto" {
        (
            Some(fixed_language(setting).ok_or("audiobook_language_invalid")?),
            "setting",
        )
    } else {
        (metadata.and_then(normalize_language), "metadata")
    };
    Ok(match language {
        Some(code) => (code, source.to_string(), "resolved".to_string()),
        None => (
            "auto".to_string(),
            "detection".to_string(),
            "pending".to_string(),
        ),
    })
}

pub fn choose_model(
    requested: Option<&str>,
    meeting: Option<&str>,
    dictation: &str,
    installed: impl Fn(&str) -> bool,
) -> Result<(String, Option<String>), String> {
    if let Some(id) = requested {
        return installed(id)
            .then(|| (id.to_string(), None))
            .ok_or_else(|| "audiobook_model_not_installed".to_string());
    }
    if installed(DEFAULT_MODEL) {
        return Ok((DEFAULT_MODEL.to_string(), None));
    }
    for (id, reason) in meeting
        .into_iter()
        .map(|id| (id, "meeting_model"))
        .chain(std::iter::once((dictation, "dictation_model")))
    {
        if installed(id) {
            return Ok((id.to_string(), Some(reason.to_string())));
        }
    }
    Err("audiobook_no_installed_model".to_string())
}

pub fn begin(
    app: &tauri::AppHandle,
    store: &MeetingStore,
    book_id: &str,
) -> Result<AudiobookRun, String> {
    begin_requested(app, store, book_id, None, None)
}

pub fn begin_requested(
    app: &tauri::AppHandle,
    store: &MeetingStore,
    book_id: &str,
    model: Option<&str>,
    language: Option<&str>,
) -> Result<AudiobookRun, String> {
    let metadata = if book_id.is_empty() {
        None
    } else {
        let book = crate::library::library_list(app.clone())?
            .into_iter()
            .find(|book| book.id == book_id)
            .ok_or("book_not_found")?;
        book.language
    };
    let mut settings = crate::settings::get_settings(app);
    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        settings.audiobook_model = Some(model.to_string());
    }
    if let Some(language) = language.map(str::trim).filter(|l| !l.is_empty()) {
        settings.audiobook_language = language.to_string();
    }
    let models = app.state::<Arc<ModelManager>>();
    let meeting_model = settings
        .meeting_model
        .as_deref()
        .unwrap_or(crate::managers::transcription::MEETING_DEFAULT_MODEL_ID);
    let (model, fallback) = choose_model(
        settings.audiobook_model.as_deref(),
        Some(meeting_model),
        &settings.selected_model,
        |id| models.get_model_info(id).is_some_and(|m| m.is_downloaded),
    )?;
    let (language, source, state) =
        language_intent(&settings.audiobook_language, metadata.as_deref())?;
    let run = AudiobookRun {
        id: ulid::Ulid::new().to_string(),
        book_id: book_id.to_string(),
        requested_model_id: settings.audiobook_model,
        effective_model_id: model,
        language_mode: settings.audiobook_language,
        language_source: source,
        language_state: state,
        language,
        model_fallback_reason: fallback,
    };
    store
        .audiobook_insert_run(&run)
        .map_err(|e| e.to_string())?;
    Ok(run)
}

/// Eine ausdrueckliche Neu-Transkription erkennt auch Altauftraege ohne Lauf.
/// Der neue Kontext wird erst mit der Arbeitsfassung verbunden.
pub(super) fn for_retranscription(
    store: &MeetingStore,
    meeting_id: &str,
    model: Option<&str>,
    language: Option<&str>,
    books: impl FnOnce() -> Result<Vec<crate::library::LibraryBook>, String>,
    begin: impl FnOnce(&str, Option<&str>, Option<&str>) -> Result<AudiobookRun, String>,
) -> Result<Option<AudiobookRun>, String> {
    let book_id = if let Some(run) = store
        .audiobook_run_for(meeting_id)
        .map_err(|e| e.to_string())?
    {
        run.book_id
    } else if store.queue_part(meeting_id).map_err(|e| e.to_string())?.0
        == super::queue_store::KIND_AUDIOBOOK
    {
        legacy_book_context(&books()?, meeting_id)
            .map(|b| b.0)
            .unwrap_or_default()
    } else {
        return Ok(None);
    };
    begin(&book_id, model, language).map(Some)
}

fn read_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<AudiobookRun> {
    Ok(AudiobookRun {
        id: row.get(0)?,
        book_id: row.get(1)?,
        requested_model_id: row.get(2)?,
        effective_model_id: row.get(3)?,
        language_mode: row.get(4)?,
        language: row.get(5)?,
        language_source: row.get(6)?,
        language_state: row.get(7)?,
        model_fallback_reason: row.get(8)?,
    })
}

impl MeetingStore {
    pub fn audiobook_attach_run(&self, meeting_id: &str, run_id: &str) -> Result<()> {
        let changed = self.get_connection()?.execute("UPDATE import_part SET audiobook_run_id=?2
            WHERE meeting_id=?1 AND kind='audiobook' AND EXISTS(SELECT 1 FROM audiobook_runs WHERE id=?2)",
            params![meeting_id, run_id])?;
        if changed != 1 {
            return Err(anyhow!("audiobook_run_missing"));
        }
        Ok(())
    }

    pub fn audiobook_insert_run(&self, run: &AudiobookRun) -> Result<()> {
        self.get_connection()?.execute(
            "INSERT INTO audiobook_runs VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                run.id,
                run.book_id,
                run.requested_model_id,
                run.effective_model_id,
                run.language_mode,
                run.language,
                run.language_source,
                run.language_state,
                run.model_fallback_reason
            ],
        )?;
        Ok(())
    }

    pub fn audiobook_run(&self, id: &str) -> Result<Option<AudiobookRun>> {
        Ok(self
            .get_connection()?
            .query_row("SELECT * FROM audiobook_runs WHERE id = ?1", [id], read_run)
            .optional()?)
    }

    pub fn audiobook_run_for(&self, meeting_id: &str) -> Result<Option<AudiobookRun>> {
        let conn = self.get_connection()?;
        let id: Option<String> = conn
            .query_row(
                "SELECT audiobook_run_id FROM import_part WHERE meeting_id = ?1",
                [meeting_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        match id {
            Some(id) => self
                .audiobook_run(&id)?
                .map(Some)
                .ok_or_else(|| anyhow!("audiobook_run_missing")),
            None => Ok(None),
        }
    }

    /// Nur Queue- und aktive Recovery-Kandidaten; keine abgeschlossene Historie.
    pub fn audiobook_contexts(
        &self,
        ids: &std::collections::HashSet<String>,
    ) -> Result<std::collections::HashMap<String, AudiobookRun>> {
        let mut rows = std::collections::HashMap::new();
        if ids.is_empty() {
            return Ok(rows);
        }
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare("SELECT r.*, p.meeting_id FROM import_part p
            LEFT JOIN audiobook_runs r ON r.id=p.audiobook_run_id WHERE p.meeting_id=?1 AND p.audiobook_run_id IS NOT NULL")?;
        for id in ids {
            if let Some(run) = stmt.query_row([id], read_run).optional()? {
                rows.insert(id.clone(), run);
            }
        }
        Ok(rows)
    }

    /// Der abgeschlossene Versuch wird VOR der Freigabe weiterer Teile atomar gespeichert.
    pub fn audiobook_settle(&self, id: &str, language: Option<&str>, source: &str) -> Result<()> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE audiobook_runs SET language=?2, language_source=?3, language_state=?4
            WHERE id=?1 AND language_state='pending'",
            params![
                id,
                language.unwrap_or("auto"),
                source,
                if language.is_some() {
                    "resolved"
                } else {
                    "unknown"
                }
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn audiobook_job_info(&self, ids: &[String]) -> Result<Vec<AudiobookRunJobInfo>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT r.* FROM audiobook_run_jobs j
            JOIN audiobook_runs r ON r.id=j.run_id WHERE j.meeting_id=?1",
        )?;
        let mut result = Vec::new();
        for id in ids {
            if let Some(run) = stmt.query_row([id], read_run).optional()? {
                result.push(AudiobookRunJobInfo {
                    meeting_id: id.clone(),
                    run_id: run.id,
                    effective_model_id: run.effective_model_id,
                    language: run.language,
                    language_source: run.language_source,
                    language_state: run.language_state,
                    model_fallback_reason: run.model_fallback_reason,
                    worker_code: self.metadata_json(id)?.and_then(|meta| {
                        meta.get("worker_failure_code").or_else(|| meta.get("worker_fallback_code"))
                            .and_then(|v| v.as_str()).map(str::to_string)
                    }),
                });
            }
        }
        Ok(result)
    }
}

/// Altauftraege behalten das bisherige Modell, auch wenn neue Hoerbuch-Einstellungen
/// inzwischen anders sind. Die Buchsuche erfolgt ueber die gespeicherten Auftrags-IDs.
pub fn ensure_context(
    app: &tauri::AppHandle,
    store: &MeetingStore,
    meeting_id: &str,
    model: &str,
) -> Result<Option<AudiobookRun>, String> {
    if let Some(run) = store
        .audiobook_run_for(meeting_id)
        .map_err(|e| e.to_string())?
    {
        return Ok(Some(run));
    }
    if store.queue_part(meeting_id).map_err(|e| e.to_string())?.0
        != super::queue_store::KIND_AUDIOBOOK
    {
        return Ok(None);
    }
    let books = crate::library::library_list(app.clone())?;
    let book = legacy_book_context(&books, meeting_id);
    let run = legacy_context(
        model,
        book.as_ref().map(|b| b.0.as_str()),
        book.as_ref().and_then(|b| b.1.as_deref()),
    );
    // Zweiter Leser darf einen inzwischen angelegten Kontext nicht ersetzen.
    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let current: Option<String> = tx
        .query_row(
            "SELECT audiobook_run_id FROM import_part WHERE meeting_id=?1",
            [meeting_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if let Some(id) = current {
        drop(tx);
        return store.audiobook_run(&id).map_err(|e| e.to_string());
    }
    tx.execute(
        "INSERT INTO audiobook_runs VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            run.id,
            run.book_id,
            run.requested_model_id,
            run.effective_model_id,
            run.language_mode,
            run.language,
            run.language_source,
            run.language_state,
            run.model_fallback_reason
        ],
    )
    .map_err(|e| e.to_string())?;
    audiobook_attach_run_tx(&tx, meeting_id, &run.id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(Some(run))
}

/// Bibliotheks-Fortsetzen: vor dem Ersatz auch verschwundene Original-IDs binden.
/// Vorhandene Laeufe bleiben; alle noch ungebundenen Teile teilen einen Kontext.
pub fn for_resume(
    store: &MeetingStore,
    book_id: &str,
    ids: &[String],
    books: &[crate::library::LibraryBook],
    model: &str,
) -> Result<AudiobookRun, String> {
    let mut metadata = None;
    for id in ids {
        let (owner, language) = legacy_book_context(books, id).ok_or("audiobook_run_missing")?;
        if owner != book_id {
            return Err("audiobook_run_missing".into());
        }
        metadata = metadata.or(language);
    }
    let candidate = legacy_context(model, Some(book_id), metadata.as_deref());
    (|| -> Result<AudiobookRun> {
        let mut conn = store.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut existing = None;
        for id in ids {
            existing = tx
                .query_row(
                    "SELECT r.* FROM audiobook_run_jobs j JOIN audiobook_runs r ON r.id=j.run_id
                 WHERE j.meeting_id=?1",
                    [id],
                    read_run,
                )
                .optional()?;
            if existing.is_some() {
                break;
            }
        }
        let run = if let Some(run) = existing {
            run
        } else {
            tx.execute(
                "INSERT INTO audiobook_runs VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    candidate.id,
                    candidate.book_id,
                    candidate.requested_model_id,
                    candidate.effective_model_id,
                    candidate.language_mode,
                    candidate.language,
                    candidate.language_source,
                    candidate.language_state,
                    candidate.model_fallback_reason
                ],
            )?;
            candidate
        };
        for id in ids {
            tx.execute(
                "UPDATE import_part SET audiobook_run_id=?2
                 WHERE meeting_id=?1 AND kind='audiobook' AND audiobook_run_id IS NULL",
                params![id, run.id],
            )?;
            // import_part kann nach Loeschen fehlen; die dauerhafte Zuordnung bleibt.
            tx.execute(
                "INSERT OR IGNORE INTO audiobook_run_jobs VALUES (?1,?2)",
                params![id, run.id],
            )?;
        }
        tx.commit()?;
        Ok(run)
    })()
    .map_err(|e| e.to_string())
}

fn legacy_context(model: &str, book: Option<&str>, metadata: Option<&str>) -> AudiobookRun {
    let language = metadata.and_then(normalize_language);
    let (language, source, state) = match language {
        Some(code) => (code, "metadata", "resolved"),
        None => ("auto".to_string(), "detection", "pending"),
    };
    AudiobookRun {
        id: ulid::Ulid::new().to_string(),
        book_id: book.unwrap_or("").to_string(),
        requested_model_id: None,
        effective_model_id: model.to_string(),
        language_mode: "auto".into(),
        language,
        language_source: source.into(),
        language_state: state.into(),
        model_fallback_reason: None,
    }
}

fn legacy_book_context(
    books: &[crate::library::LibraryBook],
    meeting_id: &str,
) -> Option<(String, Option<String>)> {
    let contains = |audio: &crate::library::AudioVersion| {
        crate::library::tracks_of(audio).iter().any(|track| {
            track.transcript_job.as_deref() == Some(meeting_id)
                || track
                    .transcript_jobs
                    .iter()
                    .any(|part| part.job == meeting_id)
        })
    };
    books.iter().find_map(|book| {
        if book.audio.as_ref().is_some_and(&contains) {
            return Some((book.id.clone(), book.language.clone()));
        }
        book.editions
            .iter()
            .find(|edition| edition.audio.as_ref().is_some_and(&contains))
            .map(|edition| (book.id.clone(), Some(edition.lang.clone())))
    })
}

pub(super) fn audiobook_attach_run_tx(
    conn: &rusqlite::Connection,
    meeting_id: &str,
    run_id: &str,
) -> Result<(), String> {
    let changed = conn.execute("UPDATE import_part SET audiobook_run_id=?2
        WHERE meeting_id=?1 AND kind='audiobook' AND EXISTS(SELECT 1 FROM audiobook_runs WHERE id=?2)",
        params![meeting_id, run_id]).map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("audiobook_run_missing".into());
    }
    Ok(())
}

pub(super) fn audiobook_restore_run_tx(
    conn: &rusqlite::Connection,
    meeting_id: &str,
    before: &serde_json::Value,
) -> Result<(), String> {
    if before.is_object() {
        conn.execute(
            "UPDATE import_part SET audiobook_run_id=?2 WHERE meeting_id=?1 AND kind='audiobook'",
            params![meeting_id, before["run_id"].as_str()],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Auch Recovery ausserhalb der Queue kann vorbereiten. Die Sperre gilt nur je Lauf;
/// bei Abbruch/Panik faellt sie, der naechste Teil uebernimmt. Kein persistierter Besitzer.
fn probe_lock(id: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<std::collections::HashMap<String, std::sync::Weak<Mutex<()>>>>> =
        OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(id).and_then(std::sync::Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(id.to_string(), Arc::downgrade(&lock));
    lock
}

pub fn bind_model(
    store: &MeetingStore,
    meeting_id: &str,
) -> Result<Option<LanguageOverride>, String> {
    Ok(store
        .audiobook_run_for(meeting_id)
        .map_err(|e| e.to_string())?
        .map(|run| LanguageOverride::set(Some(run.language), Some(run.effective_model_id))))
}

/// Sprach- und Modellherkunft stammen auch unter einer Extra-Engine aus dem Lauf.
pub fn persist_language(store: &MeetingStore, meeting_id: &str) -> bool {
    let run = match store.audiobook_run_for(meeting_id) {
        Ok(Some(run)) => run,
        Ok(None) => return false,
        Err(_) => return true,
    };
    let stored = super::language::StoredLanguage {
        code: run.language.clone(),
        source: run.language_source.clone(),
        confidence: None,
        forced: None,
        mismatch: None,
        model_id: Some(run.effective_model_id.clone()),
    };
    let result = (|| -> Result<()> {
        super::language_run::persist(store, meeting_id, &stored).map_err(|e| anyhow!(e))?;
        store.get_connection()?.execute(
            "UPDATE transcripts SET model=?2 WHERE meeting_id=?1",
            params![meeting_id, run.effective_model_id],
        )?;
        store.set_metadata_key(meeting_id, "audiobook_run", serde_json::to_value(&run)?)?;
        Ok(())
    })();
    if result.is_err() {
        log::warn!("audiobook: run provenance could not be stored");
    }
    true
}

pub fn prepare(
    tm: &TranscriptionManager,
    store: &MeetingStore,
    meeting_id: &str,
    samples: &[i16],
    run: AudiobookRun,
    job: Option<&Arc<super::job::JobHandle>>,
) -> Result<super::language_run::LanguagePlan, String> {
    let run = resolve_language(
        store,
        &run.id,
        || job.is_some_and(|j| j.is_stopped()),
        || {
            let _guard =
                LanguageOverride::set(Some("auto".into()), Some(run.effective_model_id.clone()));
            super::language_run::probe(
                tm,
                store,
                meeting_id,
                super::language::sample_windows(samples),
            )
        },
    )?;
    Ok(super::language_run::LanguagePlan::audiobook(&run))
}

fn resolve_language(
    store: &MeetingStore,
    run_id: &str,
    stopped: impl Fn() -> bool,
    detect: impl FnOnce() -> Result<super::language::ProbeResult, String>,
) -> Result<AudiobookRun, String> {
    let lock = probe_lock(run_id);
    let _lock = lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut run = store
        .audiobook_run(run_id)
        .map_err(|e| e.to_string())?
        .ok_or("audiobook_run_missing")?;
    if run.language_state == "pending" {
        if stopped() {
            return Err("audiobook_detection_stopped".into());
        }
        let detected = detect()?;
        if stopped() {
            return Err("audiobook_detection_stopped".into());
        }
        let language = super::language::decide_probe(&detected)
            .filter(|(_, _, confidence)| *confidence >= 0.8)
            .and_then(|(code, _, _)| normalize_language(&code));
        store
            .audiobook_settle(
                &run.id,
                language.as_deref(),
                if language.is_some() {
                    "probe"
                } else {
                    "unknown"
                },
            )
            .map_err(|e| e.to_string())?;
        run = store
            .audiobook_run(&run.id)
            .map_err(|e| e.to_string())?
            .ok_or("audiobook_run_missing")?;
    }
    Ok(run)
}

#[cfg(test)]
pub(super) fn test_run(id: &str, language: &str) -> AudiobookRun {
    AudiobookRun {
        id: id.to_string(),
        book_id: format!("book_{id}"),
        requested_model_id: None,
        effective_model_id: format!("synthetic_model_{id}"),
        language_mode: "auto".to_string(),
        language: language.to_string(),
        language_source: "metadata".to_string(),
        language_state: if language == "auto" {
            "pending"
        } else {
            "resolved"
        }
        .to_string(),
        model_fallback_reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_contexts_only_read_candidates_and_skip_an_empty_set() {
        let (_dir, store) = super::super::search::index::tests::tmp_store();
        for id in ["history", "waiting", "running", "recovery"] {
            store.audiobook_insert_run(&test_run(id, "auto")).unwrap();
        }
        let mut candidates = std::collections::HashSet::new();
        for id in ["history", "waiting", "running", "recovery"] {
            let part = store
                .queue_enqueue_part_run("Synthetic", "synthetic.wav", Some(1), None, Some(id))
                .unwrap();
            if id == "history" || id == "recovery" {
                store.queue_finish(&part.id).unwrap();
            }
            if id == "running" {
                store.queue_mark_running(&part.id).unwrap();
            }
            if id != "history" {
                candidates.insert(part.id);
            }
        }
        let contexts = store.audiobook_contexts(&candidates).unwrap();
        assert_eq!(contexts.len(), 3);
        assert!(contexts.keys().all(|id| candidates.contains(id)));
        assert!(contexts.values().any(|run| run.id == "recovery"));
        // Ohne Kandidaten darf selbst die SQL-Vorbereitung nicht stattfinden.
        store
            .get_connection()
            .unwrap()
            .execute_batch("DROP TABLE import_part;")
            .unwrap();
        assert!(store
            .audiobook_contexts(&std::collections::HashSet::new())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn legacy_resume_shares_metadata_context_before_replacing_original_jobs() {
        for (metadata, language) in [
            (Some("English"), "en"),
            (Some("unknown"), "auto"),
            (None, "auto"),
        ] {
            let (_dir, store) = super::super::search::index::tests::tmp_store();
            let mut explicit = test_run("explicit", "fr");
            explicit.book_id = "synthetic_book".into();
            explicit.effective_model_id = "model_X".into();
            store.audiobook_insert_run(&explicit).unwrap();
            let sibling = store
                .queue_enqueue_part_run(
                    "Synthetic",
                    "explicit.wav",
                    Some(1),
                    None,
                    Some(&explicit.id),
                )
                .unwrap();
            let a = store
                .queue_enqueue_part("Synthetic", "a.wav", Some(1), None)
                .unwrap();
            let b = store
                .queue_enqueue_part("Synthetic", "b.wav", Some(1), None)
                .unwrap();
            let ids = vec![a.id.clone(), b.id.clone()];
            let book = crate::library::LibraryBook {
                id: "synthetic_book".into(),
                language: metadata.map(str::to_string),
                audio: Some(crate::library::AudioVersion {
                    file: "synthetic.wav".into(),
                    tracks: ids
                        .iter()
                        .chain(std::iter::once(&sibling.id))
                        .map(|id| crate::library::AudioTrack {
                            file: "synthetic.wav".into(),
                            transcript_job: Some(id.clone()),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            // Auch ein verschwundener Altauftrag braucht vor dem Ersatz seinen Kontext.
            store.soft_delete_meeting(&a.id).unwrap();
            let run = for_resume(
                &store,
                &book.id,
                &ids,
                std::slice::from_ref(&book),
                "model_A",
            )
            .unwrap();
            assert_eq!(run.book_id, "synthetic_book");
            assert_eq!(run.effective_model_id, "model_A");
            assert_eq!(run.language, language);
            assert_eq!(
                run.language_state,
                if language == "auto" { "pending" } else { "resolved" }
            );
            let info = store.audiobook_job_info(&ids).unwrap();
            assert_eq!(info.len(), 2);
            assert!(info.iter().all(|job| job.run_id == run.id));
            assert_eq!(store.audiobook_run_for(&b.id).unwrap().unwrap().id, run.id);
            let sibling_run = store.audiobook_run_for(&sibling.id).unwrap().unwrap();
            assert_eq!(sibling_run.effective_model_id, "model_X");
            assert_eq!(sibling_run.language, "fr");
            // Erneutes Fortsetzen ignoriert eine inzwischen andere Modellwahl.
            let resumed = for_resume(
                &store,
                &book.id,
                &ids,
                std::slice::from_ref(&book),
                "model_B",
            )
            .unwrap();
            assert_eq!(resumed.id, run.id);
            for _ in &ids {
                let replacement = store
                    .queue_enqueue_part_run(
                        "Synthetic",
                        "replacement.wav",
                        Some(1),
                        None,
                        Some(&run.id),
                    )
                    .unwrap();
                let context = store.audiobook_run_for(&replacement.id).unwrap().unwrap();
                assert_eq!(context.id, run.id);
                assert_eq!(context.effective_model_id, "model_A");
                assert_eq!(context.language, language);
            }
        }
    }

    #[test]
    fn explicit_legacy_rerun_uses_requested_context_before_attaching() {
        let (_dir, store) = super::super::search::index::tests::tmp_store();
        let part = store
            .queue_enqueue_part("Synthetic", "synthetic.wav", Some(1), None)
            .unwrap();
        let book = crate::library::LibraryBook {
            id: "synthetic_book".into(),
            language: Some("English".into()),
            audio: Some(crate::library::AudioVersion {
                file: "synthetic.wav".into(),
                transcript_job: Some(part.id.clone()),
                ..Default::default()
            }),
            ..Default::default()
        };
        for installed in [true, false] {
            let result = for_retranscription(
                &store,
                &part.id,
                Some("B"),
                Some("fr"),
                || Ok(vec![book.clone()]),
                |book_id, model, language| {
                    assert_eq!(book_id, "synthetic_book");
                    let (model, fallback) =
                        choose_model(model, Some("A"), "A", |id| id == "A" || installed)?;
                    let (language, source, state) =
                        language_intent(language.unwrap_or("auto"), book.language.as_deref())?;
                    let mut run = test_run("requested", &language);
                    run.book_id = book_id.into();
                    run.effective_model_id = model;
                    run.language_source = source;
                    run.language_state = state;
                    run.model_fallback_reason = fallback;
                    store.audiobook_insert_run(&run).map_err(|e| e.to_string())?;
                    Ok(run)
                },
            );
            if installed {
                let run = result
                    .unwrap()
                    .expect("legacy audiobook needs an explicit new run");
                let _plan = super::super::language_run::LanguagePlan::audiobook(&run);
                assert_eq!(LanguageOverride::current_model().as_deref(), Some("B"));
                assert_eq!(LanguageOverride::current().as_deref(), Some("fr"));
                assert_eq!(run.language_source, "setting");
                assert!(run.model_fallback_reason.is_none());
            } else {
                assert_eq!(result.unwrap_err(), "audiobook_model_not_installed");
            }
            assert!(store.audiobook_run_for(&part.id).unwrap().is_none());
        }
        let meeting = store
            .queue_enqueue("Synthetic meeting", "meeting.wav", Some(1))
            .unwrap();
        assert!(for_retranscription(
            &store,
            &meeting.id,
            Some("B"),
            Some("fr"),
            || panic!("no book lookup"),
            |_, _, _| panic!("no audiobook run"),
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn deleted_jobs_keep_their_context_even_without_siblings_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("meetings.db");
        let store = MeetingStore::open_at(&db).unwrap();
        store
            .audiobook_insert_run(&test_run("original", "en"))
            .unwrap();
        let part = store
            .queue_enqueue_part_run(
                "Synthetic",
                "synthetic.wav",
                Some(1),
                Some((100, Some(200))),
                Some("original"),
            )
            .unwrap();
        store.soft_delete_meeting(&part.id).unwrap();
        assert!(store.audiobook_run_for(&part.id).unwrap().is_none());
        drop(store);
        let store = MeetingStore::open_at(&db).unwrap();
        let info = store.audiobook_job_info(&[part.id]).unwrap();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].run_id, "original");
        assert_eq!(info[0].language, "en");
        let replacement = store
            .queue_enqueue_part_run(
                "Synthetic",
                "synthetic.wav",
                Some(1),
                Some((100, Some(200))),
                Some(&info[0].run_id),
            )
            .unwrap();
        assert_eq!(
            store
                .audiobook_run_for(&replacement.id)
                .unwrap()
                .unwrap()
                .effective_model_id,
            "synthetic_model_original"
        );
    }

    #[test]
    fn legacy_books_use_metadata_or_auto_and_keep_the_meeting_model() {
        let _meeting = LanguageOverride::set(Some("de".into()), Some("meeting_model".into()));
        for (metadata, expected) in [
            (Some("English"), "en"),
            (Some("unknown"), "auto"),
            (None, "auto"),
        ] {
            let run = legacy_context("old_meeting_model", Some("synthetic_book"), metadata);
            let _plan = super::super::language_run::LanguagePlan::audiobook(&run);
            assert_eq!(LanguageOverride::current().as_deref(), Some(expected));
            assert_eq!(
                LanguageOverride::current_model().as_deref(),
                Some("old_meeting_model")
            );
            assert_eq!(
                run.language_state,
                if expected == "auto" {
                    "pending"
                } else {
                    "resolved"
                }
            );
        }
        let book = crate::library::LibraryBook {
            id: "synthetic_book".into(),
            language: Some("English".into()),
            audio: Some(crate::library::AudioVersion {
                file: "synthetic.wav".into(),
                transcript_job: Some("legacy_job".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            legacy_book_context(&[book.clone()], "legacy_job"),
            Some(("synthetic_book".into(), Some("English".into())))
        );
        assert_eq!(legacy_book_context(&[book], "missing"), None);
        let missing = legacy_context("old_meeting_model", None, None);
        assert_eq!(missing.language, "auto");
        assert_eq!(missing.effective_model_id, "old_meeting_model");
    }

    #[test]
    fn audiobook_old_settings_never_inherit_german_meetings() {
        let settings: crate::settings::AppSettings =
            serde_json::from_value(serde_json::json!({"meeting_language":"de"})).unwrap();
        assert_eq!(settings.audiobook_language, "auto");
        assert_eq!(settings.audiobook_model, None);
        assert_eq!(normalize_language("English").as_deref(), Some("en"));
        for raw in ["German", "Deutsch", "ger", "de-DE"] {
            assert_eq!(normalize_language(raw).as_deref(), Some("de"));
        }
        assert_eq!(normalize_language("unknown"), None);
        assert_eq!(normalize_language("xyz"), None);
    }

    #[test]
    fn audiobook_explicit_models_never_fall_back_and_auto_never_downloads() {
        assert!(
            choose_model(Some("missing"), Some("meeting"), "dictation", |id| id
                == "meeting")
            .is_err()
        );
        assert_eq!(
            choose_model(None, Some("meeting"), "dictation", |id| id == "meeting").unwrap(),
            ("meeting".into(), Some("meeting_model".into()))
        );
        assert_eq!(
            choose_model(None, Some("meeting"), "dictation", |_| true)
                .unwrap()
                .0,
            DEFAULT_MODEL
        );
        assert_eq!(
            choose_model(None, Some("missing"), "dictation", |id| id == "dictation")
                .unwrap()
                .0,
            "dictation"
        );
        assert!(choose_model(None, None, "missing", |_| false).is_err());
    }

    #[test]
    fn audiobook_language_precedence_and_unknown_metadata() {
        assert_eq!(
            language_intent("auto", Some("English")).unwrap(),
            ("en".into(), "metadata".into(), "resolved".into())
        );
        assert_eq!(
            language_intent("de", Some("English")).unwrap(),
            ("de".into(), "setting".into(), "resolved".into())
        );
        assert_eq!(
            language_intent("auto", Some("unknown")).unwrap(),
            ("auto".into(), "detection".into(), "pending".into())
        );
        assert_eq!(language_intent("auto", None).unwrap().2, "pending");
        assert_eq!(fixed_language("zh-Hant").as_deref(), Some("zh-Hant"));
        assert_eq!(fixed_language("af").as_deref(), Some("af"));
    }

    #[test]
    fn audiobook_retry_restart_and_both_engine_slots_keep_the_saved_context() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("meetings.db");
        let store = MeetingStore::open_at(&db).unwrap();
        let run = test_run("en", "en");
        store.audiobook_insert_run(&run).unwrap();
        let part = store
            .queue_enqueue_part_run(
                "synthetic",
                "synthetic.wav",
                Some(1),
                Some((10, Some(20))),
                Some(&run.id),
            )
            .unwrap();
        store.queue_cancel_waiting(&part.id).unwrap();
        assert!(store.queue_requeue(&part.id).unwrap());
        drop(store);
        let store = Arc::new(MeetingStore::open_at(&db).unwrap());
        assert_eq!(store.queue_part(&part.id).unwrap().1, Some((10, Some(20))));
        // Primaer- und Extra-Pfad binden dieselbe DB-Zeile vor jeder Modellwahl.
        for _slot in [
            super::super::queue::Slot::Primary,
            super::super::queue::Slot::Extra,
        ] {
            let (store, id) = (store.clone(), part.id.clone());
            std::thread::spawn(move || {
                let _meeting = LanguageOverride::set(Some("de".into()), Some("meeting".into()));
                {
                    let _guard = bind_model(&store, &id).unwrap();
                    assert_eq!(LanguageOverride::current().as_deref(), Some("en"));
                    assert_eq!(
                        LanguageOverride::current_model().as_deref(),
                        Some("synthetic_model_en")
                    );
                }
                assert_eq!(LanguageOverride::current().as_deref(), Some("de"));
                assert_eq!(
                    LanguageOverride::current_model().as_deref(),
                    Some("meeting")
                );
            })
            .join()
            .unwrap();
        }
        let other = test_run("de", "de");
        store.audiobook_insert_run(&other).unwrap();
        assert_eq!(store.audiobook_run("en").unwrap().unwrap().language, "en");
        let legacy = store
            .queue_enqueue_part("legacy", "legacy.wav", Some(1), None)
            .unwrap();
        assert!(bind_model(&store, &legacy.id).unwrap().is_none());
    }

    #[test]
    fn audiobook_stopped_or_crashed_probe_leaves_the_next_part_eligible() {
        let (_dir, store) = super::super::search::index::tests::tmp_store();
        let run = test_run("probe_retry", "auto");
        store.audiobook_insert_run(&run).unwrap();
        assert!(resolve_language(&store, &run.id, || true, || panic!("must not probe")).is_err());
        assert_eq!(
            store
                .audiobook_run(&run.id)
                .unwrap()
                .unwrap()
                .language_state,
            "pending"
        );
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = resolve_language(
                &store,
                &run.id,
                || false,
                || panic!("synthetic probe crash"),
            );
        }))
        .is_err());
        let unknown =
            resolve_language(&store, &run.id, || false, || Ok(Default::default())).unwrap();
        assert_eq!(unknown.language_state, "unknown");
        let _meeting = LanguageOverride::set(Some("de".into()), None);
        let _plan = super::super::language_run::LanguagePlan::audiobook(&unknown);
        assert_eq!(LanguageOverride::current().as_deref(), Some("auto"));
    }

    #[test]
    fn audiobook_explicit_new_transcription_changes_only_its_own_context() {
        let (_dir, store) = super::super::search::index::tests::tmp_store();
        let original = test_run("old", "en");
        let fresh = test_run("new", "fr");
        store.audiobook_insert_run(&original).unwrap();
        store.audiobook_insert_run(&fresh).unwrap();
        let a = store
            .queue_enqueue_part_run("a", "a.wav", Some(1), None, Some("old"))
            .unwrap();
        let b = store
            .queue_enqueue_part_run("b", "b.wav", Some(1), None, Some("old"))
            .unwrap();
        store.audiobook_attach_run(&a.id, "new").unwrap();
        let _context = bind_model(&store, &a.id).unwrap();
        assert_eq!(
            LanguageOverride::current_model().as_deref(),
            Some("synthetic_model_new")
        );
        assert_eq!(LanguageOverride::current().as_deref(), Some("fr"));
        assert_eq!(store.audiobook_run_for(&b.id).unwrap().unwrap().id, "old");
        assert_eq!(
            store.audiobook_run_for(&b.id).unwrap().unwrap().language,
            "en"
        );
        assert!(store.audiobook_attach_run(&a.id, "missing").is_err());
        assert_eq!(store.audiobook_run_for(&a.id).unwrap().unwrap().id, "new");
    }

    #[test]
    fn audiobook_migration_chain_keeps_legacy_ranges_and_transcripts() {
        use rusqlite_migration::Migrations;
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        let migrations = super::super::store::MIGRATIONS;
        Migrations::new(migrations[..migrations.len() - 1].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        conn.execute("INSERT INTO meetings (id,title,status,source,created_at,updated_at) VALUES ('synthetic','Synthetic','ready','import',1,1)", []).unwrap();
        conn.execute(
            "INSERT INTO import_part VALUES ('synthetic', 'audiobook', 100, 200)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO transcripts (id,meeting_id,segments_json,created_at,updated_at) VALUES ('t','synthetic','[]',1,1)", []).unwrap();
        Migrations::new(migrations.to_vec())
            .to_latest(&mut conn)
            .unwrap();
        let part: (i64, i64, Option<String>) = conn.query_row("SELECT range_start_ms,range_end_ms,audiobook_run_id FROM import_part WHERE meeting_id='synthetic'", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(part, (100, 200, None));
        assert_eq!(
            conn.query_row(
                "SELECT segments_json FROM transcripts WHERE id='t'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "[]"
        );
    }

    #[test]
    fn audiobook_other_parts_wait_for_the_committed_probe_result() {
        use std::sync::mpsc;
        use std::time::Duration;
        let (_dir, store) = super::super::search::index::tests::tmp_store();
        let store = Arc::new(store);
        let run = test_run("probe_group", "auto");
        store.audiobook_insert_run(&run).unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_store = store.clone();
        let first = std::thread::spawn(move || {
            resolve_language(
                &first_store,
                "probe_group",
                || false,
                || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(super::super::language::ProbeResult {
                        native: vec![Some("en".into()), Some("en".into())],
                        text: String::new(),
                    })
                },
            )
            .unwrap()
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let second_store = store.clone();
        let second = std::thread::spawn(move || {
            let result = resolve_language(
                &second_store,
                "probe_group",
                || false,
                || panic!("only one probe"),
            )
            .unwrap();
            done_tx.send(result.language).unwrap();
        });
        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        release_tx.send(()).unwrap();
        assert_eq!(first.join().unwrap().language, "en");
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap(), "en");
        second.join().unwrap();
        assert_eq!(
            store
                .audiobook_run("probe_group")
                .unwrap()
                .unwrap()
                .language_state,
            "resolved"
        );
    }

    #[test]
    fn worker_notice_is_per_job_and_terminal_code_takes_precedence() {
        let (_dir, store) = super::super::search::index::tests::tmp_store();
        let run = test_run("worker_notice", "en");
        store.audiobook_insert_run(&run).unwrap();
        let part = store.queue_enqueue_part_run("Synthetic", "synthetic.wav", Some(1), None, Some(&run.id)).unwrap();
        assert!(store.audiobook_job_info(&[part.id.clone()]).unwrap()[0].worker_code.is_none());
        store.set_metadata_key(&part.id, "worker_fallback_code", serde_json::json!("audiobook_worker_start_failed")).unwrap();
        assert_eq!(store.audiobook_job_info(&[part.id.clone()]).unwrap()[0].worker_code.as_deref(), Some("audiobook_worker_start_failed"));
        store.set_metadata_key(&part.id, "worker_failure_code", serde_json::json!("audiobook_worker_crashed")).unwrap();
        assert_eq!(store.audiobook_job_info(&[part.id]).unwrap()[0].worker_code.as_deref(), Some("audiobook_worker_crashed"));
    }
}
