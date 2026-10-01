//! Transkript-Fassungen (Goal „Integrationen“, Paket A3; loest B17).
//!
//! Eine Besprechung kann mehrere Fassungen ihres Transkripts haben (Untertitel,
//! eigene Transkription, Neu-Transkription, KI-Zusammenfuehrung), genau EINE ist
//! aktiv. Die aktive Fassung lebt weiter in `transcripts` (alle bisherigen Leser
//! bleiben unberuehrt: Anzeige, Notizen, Suche, Export); `transcript_variants`
//! haelt die uebrigen als Schnappschuss. Wechselt die aktive Fassung, wird der
//! aktuelle Stand von `transcripts` (samt Korrekturen von Hand) zuerst in die
//! bisher aktive Zeile zurueckgeschrieben, dann die gewaehlte eingelesen
//! (`segment_epoch` und `content_revision` steigen wie bei `clear_segments`:
//! Belege in KI-Notizen und der Such-Index erkennen den Wechsel).
//!
//! B17 (Neu-Transkription loescht das alte Transkript beim Start): die Pipeline
//! schreibt weiter live in `transcripts`; damit das alte Transkript nicht
//! verloren geht, sichert `begin_rerun` es als Fassung und setzt eine Marke
//! (`rerun_started_at`). `finish_rerun` macht das Ergebnis zur NEUEN aktiven
//! Fassung, `abort_rerun` (Stopp, Fehler) stellt die alte wieder her.
//!
//! Fehlerfaelle und Absicherung:
//! - **Abbruch mitten im Lauf / Absturz**: die Marke ueberlebt, die alte Fassung
//!   liegt vollstaendig in der Tabelle; `recover_interrupted` (beim Oeffnen der
//!   Fassungsliste und vor jedem neuen Lauf) stellt sie her. Test:
//!   `a_crash_during_a_rerun_is_repaired_by_recover`.
//! - **Zwei Schreiber gleichzeitig**: jede Aenderung in einer IMMEDIATE-Transaktion,
//!   ein Teilindex erzwingt hoechstens eine aktive Fassung je Besprechung.
//! - **Wechsel waehrend der Verarbeitung**: verweigert (`Busy`), solange die Marke
//!   steht oder die Besprechung `recording`/`processing` ist.
//! - **Voller Datentraeger**: Schreibfehler rollen die Transaktion zurueck; nie eine
//!   halb gewechselte Fassung (`activation_is_all_or_nothing`).
//! - **Migration**: idempotent (feste Kennung `v1-<Besprechung>`, `INSERT ... WHERE NOT
//!   EXISTS`); Altdaten werden nur gelesen. Test in `variants/tests.rs`.

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use specta::Type;
use ulid::Ulid;

use super::store::StoredSegment;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod translation_tests;

pub const KIND_SUBTITLES_MANUAL: &str = "subtitles_manual";
pub const KIND_SUBTITLES_AUTO: &str = "subtitles_auto";
pub const KIND_OWN: &str = "stt";
pub const KIND_MERGED: &str = "merged";
pub const KIND_RETRANSCRIBED: &str = "retranscribed";
/// G5: Uebersetzung einer anderen Fassung (Satz fuer Satz, gleiche Zeitmarken).
pub const KIND_TRANSLATION: &str = "translation";
pub const KINDS: [&str; 6] = [
    KIND_SUBTITLES_MANUAL,
    KIND_SUBTITLES_AUTO,
    KIND_OWN,
    KIND_MERGED,
    KIND_RETRANSCRIBED,
    KIND_TRANSLATION,
];

/// Migration Index 6 (A3). Nur CREATE und eine Rueckfuellung aus `transcripts`:
/// jedes vorhandene, nicht leere Transkript wird Fassung 1 (aktiv). Laeuft nur
/// einmal; die Rueckfuellung ist zusaetzlich selbst idempotent.
pub const VARIANTS_MIGRATION: &str = "CREATE TABLE transcript_variants (
      id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL,
      kind TEXT NOT NULL CHECK (kind IN ('subtitles_manual','subtitles_auto','stt','merged','retranscribed')),
      language TEXT, model TEXT,
      segments_json TEXT NOT NULL DEFAULT '[]', speaker_hints_json TEXT,
      number INTEGER NOT NULL, created_at INTEGER NOT NULL,
      active INTEGER NOT NULL DEFAULT 0, rerun_started_at INTEGER, deleted_at INTEGER);
    CREATE INDEX idx_variants_meeting ON transcript_variants(meeting_id, number);
    CREATE UNIQUE INDEX idx_variants_one_active ON transcript_variants(meeting_id)
      WHERE active = 1 AND deleted_at IS NULL;
    INSERT INTO transcript_variants (id, meeting_id, kind, language, model, segments_json,
        speaker_hints_json, number, created_at, active)
    SELECT 'v1-' || t.meeting_id, t.meeting_id,
           CASE m.source WHEN 'subtitle' THEN 'subtitles_manual' ELSE 'stt' END,
           COALESCE(t.language, m.language), t.model, t.segments_json, t.speaker_hints_json,
           1, t.created_at, 1
    FROM transcripts t JOIN meetings m ON m.id = t.meeting_id
    WHERE t.deleted_at IS NULL AND m.deleted_at IS NULL AND t.segments_json <> '[]'
      AND NOT EXISTS (SELECT 1 FROM transcript_variants v WHERE v.meeting_id = t.meeting_id);";

/// Migration Index 8 (G5). Die Fassungsart `translation` und die Herkunft einer
/// Uebersetzung (Quellfassung, Ausgangssprache, Pruefbericht). SQLite kann eine
/// CHECK-Bedingung nicht aendern: die Tabelle wird neu gebaut (neu anlegen, kopieren,
/// alte loeschen, umbenennen, Indizes neu). Alles in EINER Transaktion der Kette: bricht
/// etwas ab, bleibt die alte Tabelle vollstaendig, und die Ersatztabelle verschwindet
/// mit. Jede Zeile wird mit denselben Werten kopiert (auch Neu-Lauf-Marke und
/// geloeschte); die neuen Spalten bleiben leer.
pub const VARIANTS_TRANSLATION_MIGRATION: &str = "CREATE TABLE transcript_variants_g5 (
      id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL,
      kind TEXT NOT NULL CHECK (kind IN ('subtitles_manual','subtitles_auto','stt','merged','retranscribed','translation')),
      language TEXT, model TEXT,
      segments_json TEXT NOT NULL DEFAULT '[]', speaker_hints_json TEXT,
      number INTEGER NOT NULL, created_at INTEGER NOT NULL,
      active INTEGER NOT NULL DEFAULT 0, rerun_started_at INTEGER, deleted_at INTEGER,
      source_variant_id TEXT, source_language TEXT, meta_json TEXT);
    INSERT INTO transcript_variants_g5 (id, meeting_id, kind, language, model, segments_json,
        speaker_hints_json, number, created_at, active, rerun_started_at, deleted_at)
    SELECT id, meeting_id, kind, language, model, segments_json, speaker_hints_json, number,
           created_at, active, rerun_started_at, deleted_at
    FROM transcript_variants;
    DROP TABLE transcript_variants;
    ALTER TABLE transcript_variants_g5 RENAME TO transcript_variants;
    CREATE INDEX idx_variants_meeting ON transcript_variants(meeting_id, number);
    CREATE UNIQUE INDEX idx_variants_one_active ON transcript_variants(meeting_id)
      WHERE active = 1 AND deleted_at IS NULL;";

/// Eine Fassung, wie die Oberflaeche sie liest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TranscriptVariant {
    pub id: String,
    pub meeting_id: String,
    /// `subtitles_manual`, `subtitles_auto`, `stt`, `merged`, `retranscribed`, `translation`.
    pub kind: String,
    pub language: Option<String>,
    pub model: Option<String>,
    /// Laufende Nummer je Besprechung (v1, v2, ...).
    pub number: u32,
    /// Sekunden UTC.
    pub created_at: i64,
    pub active: bool,
    pub segment_count: u32,
    /// G5: bei einer Uebersetzung die Fassung, aus der sie entstand.
    pub source_variant_id: Option<String>,
    /// G5: bei einer Uebersetzung die Sprache der Quellfassung.
    pub source_language: Option<String>,
    /// G5: bei einer Uebersetzung die Zahl der Saetze, die die Treuepruefung markiert hat.
    pub flagged: u32,
}

/// Was `add` anlegt.
#[derive(Clone, Debug)]
pub struct NewVariant {
    pub meeting_id: String,
    pub kind: &'static str,
    pub language: Option<String>,
    pub model: Option<String>,
    pub segments: Vec<StoredSegment>,
    pub activate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariantError {
    NotFound,
    /// Aufnahme oder Verarbeitung laeuft, oder ein Neu-Lauf ist nicht abgeschlossen.
    Busy,
    /// Eine Fassung ohne Segmente gibt es nicht.
    Empty,
    /// Es gibt keinen begonnenen Neu-Lauf.
    NoRerun,
    Store(String),
}

impl VariantError {
    pub fn code(&self) -> &'static str {
        match self {
            VariantError::NotFound => "variant_not_found",
            VariantError::Busy => "variant_busy",
            VariantError::Empty => "variant_empty",
            VariantError::NoRerun => "variant_no_rerun",
            VariantError::Store(_) => "variant_store_failed",
        }
    }
}

impl std::fmt::Display for VariantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VariantError::Store(m) => write!(f, "{}: {m}", self.code()),
            other => write!(f, "{}", other.code()),
        }
    }
}

impl std::error::Error for VariantError {}

impl From<rusqlite::Error> for VariantError {
    fn from(e: rusqlite::Error) -> Self {
        VariantError::Store(e.to_string())
    }
}

fn now_s() -> i64 {
    chrono::Utc::now().timestamp()
}

fn count_segments(json: &str) -> u32 {
    serde_json::from_str::<Vec<serde::de::IgnoredAny>>(json)
        .map(|v| u32::try_from(v.len()).unwrap_or(u32::MAX))
        .unwrap_or(0)
}

struct TranscriptRow {
    segments_json: String,
    hints: Option<String>,
    model: Option<String>,
}

fn transcript_row(conn: &Connection, meeting_id: &str) -> rusqlite::Result<Option<TranscriptRow>> {
    conn.query_row(
        "SELECT segments_json, speaker_hints_json, model FROM transcripts
         WHERE meeting_id = ?1 AND deleted_at IS NULL",
        params![meeting_id],
        |r| {
            Ok(TranscriptRow {
                segments_json: r.get(0)?,
                hints: r.get(1)?,
                model: r.get(2)?,
            })
        },
    )
    .optional()
}

fn meeting_status(conn: &Connection, meeting_id: &str) -> Result<Option<String>, VariantError> {
    Ok(conn
        .query_row(
            "SELECT status FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
            params![meeting_id],
            |r| r.get(0),
        )
        .optional()?)
}

fn require_meeting(conn: &Connection, meeting_id: &str) -> Result<String, VariantError> {
    meeting_status(conn, meeting_id)?.ok_or(VariantError::NotFound)
}

fn has_variants(conn: &Connection, meeting_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM transcript_variants WHERE meeting_id = ?1 AND deleted_at IS NULL)",
        params![meeting_id],
        |r| r.get(0),
    )
}

/// Hat die Besprechung ein Transkript, aber noch keine Fassung (angelegt nach der
/// Migration), wird es Fassung 1. Idempotent (feste Kennung).
fn ensure_baseline(tx: &Transaction<'_>, meeting_id: &str) -> Result<(), VariantError> {
    if has_variants(tx, meeting_id)? {
        return Ok(());
    }
    let Some(row) = transcript_row(tx, meeting_id)? else {
        return Ok(());
    };
    if count_segments(&row.segments_json) == 0 {
        return Ok(());
    }
    let source: Option<String> = tx
        .query_row(
            "SELECT source FROM meetings WHERE id = ?1",
            params![meeting_id],
            |r| r.get(0),
        )
        .optional()?;
    let kind = if source.as_deref() == Some("subtitle") {
        KIND_SUBTITLES_MANUAL
    } else {
        KIND_OWN
    };
    tx.execute(
        "INSERT OR IGNORE INTO transcript_variants (id, meeting_id, kind, language, model,
             segments_json, speaker_hints_json, number, created_at, active)
         SELECT ?1, ?2, ?3, COALESCE(t.language, m.language), t.model, t.segments_json,
                t.speaker_hints_json, 1, ?4, 1
         FROM transcripts t JOIN meetings m ON m.id = t.meeting_id WHERE t.meeting_id = ?2",
        params![format!("v1-{meeting_id}"), meeting_id, kind, now_s()],
    )?;
    Ok(())
}

fn active_marker(tx: &Connection, meeting_id: &str) -> rusqlite::Result<Option<(String, bool)>> {
    tx.query_row(
        "SELECT id, rerun_started_at IS NOT NULL FROM transcript_variants
         WHERE meeting_id = ?1 AND active = 1 AND deleted_at IS NULL",
        params![meeting_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

/// Den aktuellen Stand von `transcripts` in die aktive Fassung zurueckschreiben
/// (Korrekturen von Hand). Nicht waehrend eines Neu-Laufs: dann ist `transcripts`
/// nur die Arbeitskopie des neuen Laufs.
fn sync_active(tx: &Transaction<'_>, meeting_id: &str) -> rusqlite::Result<()> {
    let Some(row) = transcript_row(tx, meeting_id)? else {
        return Ok(());
    };
    tx.execute(
        "UPDATE transcript_variants
         SET segments_json = ?1, speaker_hints_json = ?2, model = COALESCE(?3, model)
         WHERE meeting_id = ?4 AND active = 1 AND deleted_at IS NULL
           AND rerun_started_at IS NULL",
        params![row.segments_json, row.hints, row.model, meeting_id],
    )?;
    Ok(())
}

/// Schreibt eine Fassung als aktives Transkript (wie `clear_segments` + neue
/// Segmente): Deltas weg, Epoche und Revision hoch.
fn write_transcript(
    tx: &Transaction<'_>,
    meeting_id: &str,
    segments_json: &str,
    hints: Option<&str>,
    model: Option<&str>,
    language: Option<&str>,
) -> rusqlite::Result<()> {
    let now = now_s();
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM transcripts WHERE meeting_id = ?1",
            params![meeting_id],
            |r| r.get(0),
        )
        .optional()?;
    match existing {
        Some(id) => {
            tx.execute(
                "DELETE FROM transcript_deltas WHERE transcript_id = ?1",
                params![id],
            )?;
            tx.execute(
                "UPDATE transcripts SET segments_json = ?1, speaker_hints_json = ?2, model = ?3,
                     language = COALESCE(?4, language), deleted_at = NULL,
                     content_revision = content_revision + 1,
                     segment_epoch = segment_epoch + 1, updated_at = ?5
                 WHERE id = ?6",
                params![segments_json, hints, model, language, now, id],
            )?;
        }
        None => {
            tx.execute(
                "INSERT INTO transcripts (id, meeting_id, model, language, segments_json,
                     speaker_hints_json, content_revision, segment_epoch, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 1, ?7, ?7)",
                params![
                    Ulid::new().to_string(),
                    meeting_id,
                    model,
                    language,
                    segments_json,
                    hints,
                    now
                ],
            )?;
        }
    }
    Ok(())
}

type VariantRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    bool,
);

fn load_row(tx: &Connection, variant_id: &str) -> Result<VariantRow, VariantError> {
    tx.query_row(
        "SELECT meeting_id, kind, language, model, segments_json, speaker_hints_json, active
         FROM transcript_variants WHERE id = ?1 AND deleted_at IS NULL",
        params![variant_id],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        },
    )
    .optional()?
    .ok_or(VariantError::NotFound)
}

fn activate_in_tx(tx: &Transaction<'_>, variant_id: &str) -> Result<(), VariantError> {
    let (meeting_id, _kind, language, model, segments_json, hints, active) =
        load_row(tx, variant_id)?;
    if active {
        return Ok(());
    }
    if matches!(
        meeting_status(tx, &meeting_id)?.as_deref(),
        Some("recording") | Some("processing")
    ) {
        return Err(VariantError::Busy);
    }
    if matches!(active_marker(tx, &meeting_id)?, Some((_, true))) {
        return Err(VariantError::Busy);
    }
    sync_active(tx, &meeting_id)?;
    tx.execute(
        "UPDATE transcript_variants SET active = 0
         WHERE meeting_id = ?1 AND active = 1 AND deleted_at IS NULL",
        params![meeting_id],
    )?;
    tx.execute(
        "UPDATE transcript_variants SET active = 1 WHERE id = ?1",
        params![variant_id],
    )?;
    write_transcript(
        tx,
        &meeting_id,
        &segments_json,
        hints.as_deref(),
        model.as_deref(),
        language.as_deref(),
    )?;
    Ok(())
}

/// Wie viele Saetze der Pruefbericht einer Uebersetzung markiert hat (`flagged`-Feld).
fn flagged_count(meta_json: Option<&str>) -> u32 {
    meta_json
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v.get("flagged").and_then(|f| f.as_array().map(Vec::len)))
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        .unwrap_or(0)
}

fn map_variant(conn: &Connection, id: &str) -> Result<TranscriptVariant, VariantError> {
    type Row = (
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        u32,
        i64,
        bool,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row: Row = conn
        .query_row(
            "SELECT meeting_id, kind, language, model, segments_json, number, created_at, active,
                    source_variant_id, source_language, meta_json
             FROM transcript_variants WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ))
            },
        )
        .optional()?
        .ok_or(VariantError::NotFound)?;
    let (
        meeting_id,
        kind,
        language,
        model,
        json,
        number,
        created_at,
        active,
        source_variant_id,
        source_language,
        meta_json,
    ) = row;
    // Die aktive Fassung ist der Stand von `transcripts` (Korrekturen von Hand).
    let effective_json = if active {
        transcript_row(conn, &meeting_id)?
            .filter(|_| !matches!(active_marker(conn, &meeting_id), Ok(Some((_, true)))))
            .map(|t| t.segments_json)
            .unwrap_or(json)
    } else {
        json
    };
    Ok(TranscriptVariant {
        id: id.to_string(),
        meeting_id,
        kind,
        language,
        model,
        number,
        created_at,
        active,
        segment_count: count_segments(&effective_json),
        flagged: flagged_count(meta_json.as_deref()),
        source_variant_id,
        source_language,
    })
}

// ---------------------------------------------------------------------------
// Lesen und Anlegen
// ---------------------------------------------------------------------------

/// Die Fassungen einer Besprechung (nach Nummer), ohne leere. Legt fuer ein
/// Transkript ohne Fassung die Fassung 1 an (idempotent). Eine unterbrochene
/// Neu-Transkription stellt der Aufrufer vorher mit `recover_interrupted` her.
pub fn list(
    conn: &mut Connection,
    meeting_id: &str,
) -> Result<Vec<TranscriptVariant>, VariantError> {
    require_meeting(conn, meeting_id)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_baseline(&tx, meeting_id)?;
    tx.commit()?;
    let ids: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM transcript_variants WHERE meeting_id = ?1 AND deleted_at IS NULL
             ORDER BY number",
        )?;
        let rows = stmt.query_map(params![meeting_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let variant = map_variant(conn, &id)?;
        if variant.segment_count > 0 {
            out.push(variant);
        }
    }
    Ok(out)
}

/// Eine Fassung samt Segmenten; die aktive liest `transcripts` (Stand von Hand).
pub fn get_segments(
    conn: &Connection,
    variant_id: &str,
) -> Result<(TranscriptVariant, Vec<StoredSegment>), VariantError> {
    let variant = map_variant(conn, variant_id)?;
    let json: String = if variant.active
        && !matches!(active_marker(conn, &variant.meeting_id)?, Some((_, true)))
    {
        transcript_row(conn, &variant.meeting_id)?
            .map(|t| t.segments_json)
            .unwrap_or_else(|| "[]".to_string())
    } else {
        conn.query_row(
            "SELECT segments_json FROM transcript_variants WHERE id = ?1",
            params![variant_id],
            |r| r.get(0),
        )?
    };
    let segments: Vec<StoredSegment> =
        serde_json::from_str(&json).map_err(|e| VariantError::Store(e.to_string()))?;
    Ok((variant, segments))
}

/// Legt eine Fassung an. Ohne aktive Fassung (Besprechung ohne Transkript) wird sie
/// sofort aktiv. Gibt die Kennung zurueck.
pub fn add(conn: &mut Connection, new: NewVariant) -> Result<String, VariantError> {
    if new.segments.is_empty() {
        return Err(VariantError::Empty);
    }
    if !KINDS.contains(&new.kind) {
        return Err(VariantError::Store(format!("unknown kind {}", new.kind)));
    }
    let json =
        serde_json::to_string(&new.segments).map_err(|e| VariantError::Store(e.to_string()))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_meeting(&tx, &new.meeting_id)?;
    ensure_baseline(&tx, &new.meeting_id)?;
    let number: u32 = tx.query_row(
        "SELECT COALESCE(MAX(number), 0) + 1 FROM transcript_variants WHERE meeting_id = ?1",
        params![new.meeting_id],
        |r| r.get(0),
    )?;
    let id = Ulid::new().to_string();
    tx.execute(
        "INSERT INTO transcript_variants (id, meeting_id, kind, language, model, segments_json,
             number, created_at, active)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)",
        params![
            id,
            new.meeting_id,
            new.kind,
            new.language,
            new.model,
            json,
            number,
            now_s()
        ],
    )?;
    let has_active = active_marker(&tx, &new.meeting_id)?.is_some();
    if new.activate || !has_active {
        activate_in_tx(&tx, &id)?;
    }
    tx.commit()?;
    Ok(id)
}

/// G5: Herkunft einer Uebersetzung.
#[derive(Clone, Debug)]
pub struct TranslationExtra {
    /// Die Fassung, aus der uebersetzt wurde (gehoert zur selben Besprechung).
    pub source_variant_id: String,
    pub source_language: Option<String>,
    /// Pruefbericht als JSON (`flagged`: Liste der markierten Saetze, weitere Felder frei).
    pub meta_json: String,
}

/// G5: legt eine UEBERSETZUNG als Fassung an (nicht aktiv, ausser es gibt noch keine
/// aktive). Das Original bleibt unveraendert: die Quellfassung wird nur geprueft, nie
/// geschrieben. Die Quelle muss eine Fassung DERSELBEN Besprechung sein (`NotFound`).
/// Eine Transaktion: bricht etwas ab, entsteht keine halbe Fassung.
pub fn add_translation(
    conn: &mut Connection,
    new: NewVariant,
    extra: TranslationExtra,
) -> Result<String, VariantError> {
    if new.kind != KIND_TRANSLATION {
        return Err(VariantError::Store(format!(
            "add_translation needs kind {KIND_TRANSLATION}, got {}",
            new.kind
        )));
    }
    if new.segments.is_empty() {
        return Err(VariantError::Empty);
    }
    let json =
        serde_json::to_string(&new.segments).map_err(|e| VariantError::Store(e.to_string()))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_meeting(&tx, &new.meeting_id)?;
    ensure_baseline(&tx, &new.meeting_id)?;
    let source_ok: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM transcript_variants
                       WHERE id = ?1 AND meeting_id = ?2 AND deleted_at IS NULL)",
        params![extra.source_variant_id, new.meeting_id],
        |r| r.get(0),
    )?;
    if !source_ok {
        return Err(VariantError::NotFound);
    }
    let number: u32 = tx.query_row(
        "SELECT COALESCE(MAX(number), 0) + 1 FROM transcript_variants WHERE meeting_id = ?1",
        params![new.meeting_id],
        |r| r.get(0),
    )?;
    let id = Ulid::new().to_string();
    tx.execute(
        "INSERT INTO transcript_variants (id, meeting_id, kind, language, model, segments_json,
             number, created_at, active, source_variant_id, source_language, meta_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11)",
        params![
            id,
            new.meeting_id,
            new.kind,
            new.language,
            new.model,
            json,
            number,
            now_s(),
            extra.source_variant_id,
            extra.source_language,
            extra.meta_json,
        ],
    )?;
    if new.activate || active_marker(&tx, &new.meeting_id)?.is_none() {
        activate_in_tx(&tx, &id)?;
    }
    tx.commit()?;
    Ok(id)
}

/// G5: der Pruefbericht einer Uebersetzung (JSON); `None` bei jeder anderen Fassung.
pub fn get_meta(conn: &Connection, variant_id: &str) -> Result<Option<String>, VariantError> {
    conn.query_row(
        "SELECT meta_json FROM transcript_variants WHERE id = ?1 AND deleted_at IS NULL",
        params![variant_id],
        |r| r.get::<_, Option<String>>(0),
    )
    .optional()?
    .ok_or(VariantError::NotFound)
}

/// G5: Sprachkorrektur ueber den Chip im Kopf: gilt fuer die Besprechung, das aktive
/// Transkript und die aktive Fassung; andere Fassungen behalten ihre Sprache. Nicht
/// waehrend eines Neu-Laufs (`Busy`): dort ist `transcripts` die Arbeitskopie.
pub fn set_language(
    conn: &mut Connection,
    meeting_id: &str,
    language: &str,
) -> Result<(), VariantError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_meeting(&tx, meeting_id)?;
    if matches!(active_marker(&tx, meeting_id)?, Some((_, true))) {
        return Err(VariantError::Busy);
    }
    let now = now_s();
    tx.execute(
        "UPDATE meetings SET language = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
        params![language, now, meeting_id],
    )?;
    tx.execute(
        "UPDATE transcripts SET language = ?1, updated_at = ?2 WHERE meeting_id = ?3",
        params![language, now, meeting_id],
    )?;
    tx.execute(
        "UPDATE transcript_variants SET language = ?1
         WHERE meeting_id = ?2 AND active = 1 AND deleted_at IS NULL",
        params![language, meeting_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// „Fassung waehlen“: macht die Fassung zum aktiven Transkript.
pub fn activate(
    conn: &mut Connection,
    variant_id: &str,
) -> Result<TranscriptVariant, VariantError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    activate_in_tx(&tx, variant_id)?;
    tx.commit()?;
    map_variant(conn, variant_id)
}

// ---------------------------------------------------------------------------
// Neu-Transkription (B17)
// ---------------------------------------------------------------------------

fn restore_in_tx(tx: &Transaction<'_>, meeting_id: &str) -> Result<bool, VariantError> {
    let Some((id, true)) = active_marker(tx, meeting_id)? else {
        return Ok(false);
    };
    let (_m, _k, language, model, json, hints, _a) = load_row(tx, &id)?;
    write_transcript(
        tx,
        meeting_id,
        &json,
        hints.as_deref(),
        model.as_deref(),
        language.as_deref(),
    )?;
    tx.execute(
        "UPDATE transcript_variants SET rerun_started_at = NULL WHERE id = ?1",
        params![id],
    )?;
    Ok(true)
}

/// Vor einer Neu-Transkription: das alte Transkript als Fassung sichern und die
/// Marke setzen. Eine noch stehende Marke (Absturz) wird zuerst aufgeloest.
pub fn begin_rerun(conn: &mut Connection, meeting_id: &str) -> Result<(), VariantError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_meeting(&tx, meeting_id)?;
    restore_in_tx(&tx, meeting_id)?;
    ensure_baseline(&tx, meeting_id)?;
    if !has_variants(&tx, meeting_id)? {
        // Noch kein Transkript: eine leere aktive Fassung haelt die Marke, damit ein
        // Abbruch die halbe Arbeitskopie wieder leert.
        tx.execute(
            "INSERT INTO transcript_variants (id, meeting_id, kind, segments_json, number,
                 created_at, active)
             VALUES (?1, ?2, ?3, '[]', 1, ?4, 1)",
            params![Ulid::new().to_string(), meeting_id, KIND_OWN, now_s()],
        )?;
    }
    sync_active(&tx, meeting_id)?;
    tx.execute(
        "UPDATE transcript_variants SET rerun_started_at = ?2
         WHERE meeting_id = ?1 AND active = 1 AND deleted_at IS NULL",
        params![meeting_id, now_s()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Der Lauf ist fertig: das Ergebnis in `transcripts` wird die NEUE aktive Fassung,
/// die alte bleibt als Fassung erhalten. Gibt die Kennung der neuen zurueck.
pub fn finish_rerun(
    conn: &mut Connection,
    meeting_id: &str,
    kind: &'static str,
    language: Option<&str>,
) -> Result<String, VariantError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some((old_id, true)) = active_marker(&tx, meeting_id)? else {
        return Err(VariantError::NoRerun);
    };
    let current = transcript_row(&tx, meeting_id)?.ok_or(VariantError::Empty)?;
    if count_segments(&current.segments_json) == 0 {
        return Err(VariantError::Empty);
    }
    let old_json: String = tx.query_row(
        "SELECT segments_json FROM transcript_variants WHERE id = ?1",
        params![old_id],
        |r| r.get(0),
    )?;
    if count_segments(&old_json) == 0 {
        // Es gab vorher kein Transkript: der leere Halter entfaellt.
        tx.execute(
            "DELETE FROM transcript_variants WHERE id = ?1",
            params![old_id],
        )?;
    } else {
        tx.execute(
            "UPDATE transcript_variants SET active = 0, rerun_started_at = NULL WHERE id = ?1",
            params![old_id],
        )?;
    }
    let number: u32 = tx.query_row(
        "SELECT COALESCE(MAX(number), 0) + 1 FROM transcript_variants WHERE meeting_id = ?1",
        params![meeting_id],
        |r| r.get(0),
    )?;
    let id = Ulid::new().to_string();
    tx.execute(
        "INSERT INTO transcript_variants (id, meeting_id, kind, language, model, segments_json,
             speaker_hints_json, number, created_at, active)
         VALUES (?1, ?2, ?3, COALESCE(?4, (SELECT language FROM transcripts WHERE meeting_id = ?2)),
                 ?5, ?6, ?7, ?8, ?9, 1)",
        params![
            id,
            meeting_id,
            kind,
            language,
            current.model,
            current.segments_json,
            current.hints,
            number,
            now_s()
        ],
    )?;
    tx.commit()?;
    Ok(id)
}

/// Stopp oder Fehler: die alte Fassung ist wieder das Transkript. `false`: kein
/// Neu-Lauf begonnen, nichts getan.
pub fn abort_rerun(conn: &mut Connection, meeting_id: &str) -> Result<bool, VariantError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let restored = restore_in_tx(&tx, meeting_id)?;
    tx.commit()?;
    Ok(restored)
}

/// Eine nicht abgeschlossene Neu-Transkription (Absturz) zuruecknehmen. Darf nur
/// laufen, wenn kein Auftrag fuer die Besprechung aktiv ist (Aufrufer prueft).
pub fn recover_interrupted(conn: &mut Connection, meeting_id: &str) -> Result<bool, VariantError> {
    abort_rerun(conn, meeting_id)
}
