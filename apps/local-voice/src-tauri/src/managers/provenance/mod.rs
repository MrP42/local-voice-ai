//! Provenienz: zu jedem erzeugten Inhalt, woher er kommt (A1, Goal
//! „Integrationen“, AK1/AK6; Querschnitt fuer A, B und C).
//!
//! Die Tabelle `provenance` liegt in `meetings.db` (Migration Index 5, siehe
//! `integrations::schema`). Je erzeugtem Inhalt gibt es einen oder mehrere
//! Eintraege: Inhaltsart + ID, Operation, Ausloeser, Anbieter (lokal/entfernt),
//! Modell, Token ein/aus, Dauer, Quellen, Konfidenz, Verweis auf das Ereignis
//! im Verbrauchs-Ledger (`usage.db`, dort bleiben Kosten und Preise).
//!
//! - **Schreiben** (`record`): ein einzelnes INSERT, also atomar. Eingaben werden
//!   geprueft und begrenzt (Quellen, Textlaengen, `params`), damit ein Fehler
//!   in einer Erzeugungsstelle nie die Tabelle sprengt. `params`, `actor_ref` und
//!   Quellen-Adressen laufen durch dieselbe Schwaerzung wie das Audit
//!   (`integrations::audit::redact_params`/`redact_text`): kein Zugangsschluessel
//!   in der Herkunft, aber Listen und Pfade bleiben vollstaendig.
//! - **Lesen** (`get`): die gespeicherten Eintraege; gibt es keine (Inhalt aus der
//!   Zeit vor A1, oder der Eintrag liess sich nicht schreiben), wird die Herkunft
//!   aus den vorhandenen Daten abgeleitet (`derive_legacy`): bei Dokumenten aus
//!   `generation_metadata_json`, beim Transkript aus Modell/Anbieter der
//!   Transkriptzeile. Solche Eintraege tragen `origin: derived` und nur, was dort
//!   stand (keine Token, keine Dauer, kein Ausloeser).
//! - **Erzeugungsstellen** (`generation`): Protokoll, KI-Notizen, Follow-up, STT.
//!   Das Schreiben der Provenienz darf eine Erzeugung NIE scheitern lassen
//!   (Fehler -> Warnung, der Rueckfall auf die Altdaten greift).
//!
//! Fehlerfaelle und ihre Absicherung:
//! - Abbruch zwischen Dokument und Provenienz: zwei getrennte Schreibwege; fehlt
//!   der Eintrag, liefert `get` die Herkunft aus `generation_metadata_json`
//!   (`generation::tests::a_broken_provenance_table_never_fails_the_generation`,
//!   `tests::a_document_without_an_entry_falls_back_to_its_metadata`).
//! - Voller Datentraeger / gesperrte Datenbank: `record` meldet `Store`, die
//!   Erzeugungsstelle loggt und liefert ihr Ergebnis trotzdem.
//! - Nebenlaeufigkeit: jeder Eintrag hat eine eigene ULID; parallele Schreiber
//!   (App und Headless) stoeren sich nicht (`tests::parallel_writers_keep_every_entry`).
//! - Eingaben ausser der Reihe (Konfidenz NaN, riesige Quellenliste): abgewiesen
//!   bzw. gekuerzt (`tests::invalid_input_is_refused_not_stored`).
//! - Kindprozess, fehlendes Geraet, Audio-Callback: nicht beteiligt (reine
//!   Metadaten nach dem Erzeugen).

pub mod generation;
pub mod model;

pub use model::{
    ActorKind, Locality, NewProvenance, ProvenanceEntry, ProvenanceError, ProvenanceOrigin,
    SourceRef, SubjectKind,
};

use rusqlite::{params, Connection, OptionalExtension};
use ulid::Ulid;

use crate::managers::integrations::audit::{redact_params, redact_text};

/// Hoechstzahl Quellen je Eintrag.
pub const MAX_SOURCES: usize = 200;
/// Laengste Textangabe (Modell, Titel, Verweis) in Zeichen; mehr wird gekuerzt.
pub const MAX_FIELD_CHARS: usize = 512;
/// Groesste `params`-Angabe in Bytes; mehr wird abgewiesen.
pub const MAX_PARAMS_BYTES: usize = 16 * 1024;

fn clip(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_FIELD_CHARS).collect();
    if s.chars().count() > MAX_FIELD_CHARS {
        out.push('…');
    }
    out
}

fn clip_opt(s: &Option<String>) -> Option<String> {
    s.as_deref().map(clip).filter(|t| !t.trim().is_empty())
}

/// Wie `clip_opt`, und Zugangsdaten in Adressen, Tokens und `key=wert`-Zuweisungen
/// fallen weg (dieselbe Schwaerzung wie im Audit: `audit::redact_text`).
fn clean_opt(s: &Option<String>) -> Option<String> {
    clip_opt(s).map(|t| redact_text(&t))
}

/// Kleinbuchstaben, Ziffern, `_`; 1 bis 48 Zeichen.
fn valid_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Prueft die Eingabe; nie gueltig ist: leere ID, unbekannte Schreibweise der
/// Operation oder einer Quellenart, Konfidenz ausser 0..=1 (auch NaN),
/// Ereignisnummer <= 0, zu viele Quellen, zu grosse `params`.
pub fn validate(e: &NewProvenance) -> Result<(), ProvenanceError> {
    let bad = |m: &str| Err(ProvenanceError::Invalid(m.to_string()));
    if e.subject_id.trim().is_empty() {
        return bad("subject_id fehlt");
    }
    if !valid_token(&e.operation) {
        return bad("operation: nur Kleinbuchstaben, Ziffern und _ (1 bis 48 Zeichen)");
    }
    if let Some(c) = e.confidence {
        if !c.is_finite() || !(0.0..=1.0).contains(&c) {
            return bad("confidence muss zwischen 0 und 1 liegen");
        }
    }
    if e.usage_event_id.is_some_and(|id| id <= 0) {
        return bad("usage_event_id muss positiv sein");
    }
    if e.sources.len() > MAX_SOURCES {
        return bad("zu viele Quellen");
    }
    if e.sources
        .iter()
        .any(|s| !valid_token(&s.kind) || s.reference.trim().is_empty())
    {
        return bad("Quelle: Art (Kleinbuchstaben/Ziffern/_) und Verweis sind Pflicht");
    }
    if let Some(p) = &e.params {
        if !p.is_object() {
            return bad("params muss ein JSON-Objekt sein");
        }
        if p.to_string().len() > MAX_PARAMS_BYTES {
            return bad("params zu gross");
        }
    }
    Ok(())
}

/// Schreibt einen Eintrag und gibt dessen ID zurueck.
pub fn record(conn: &Connection, e: &NewProvenance) -> Result<String, ProvenanceError> {
    record_at(conn, e, chrono::Utc::now().timestamp_millis())
}

pub fn record_at(
    conn: &Connection,
    e: &NewProvenance,
    now_ms: i64,
) -> Result<String, ProvenanceError> {
    validate(e)?;
    let sources: Vec<SourceRef> = e
        .sources
        .iter()
        .map(|s| SourceRef {
            kind: s.kind.clone(),
            reference: redact_text(&clip(&s.reference)),
            title: clip_opt(&s.title),
            url: clean_opt(&s.url),
        })
        .collect();
    let sources_json =
        serde_json::to_string(&sources).map_err(|err| ProvenanceError::Invalid(err.to_string()))?;
    let id = Ulid::new().to_string();
    conn.execute(
        "INSERT INTO provenance (id, subject_kind, subject_id, subject_revision, created_at,
            operation, actor_kind, actor_ref, provider, locality, model_id, model_label,
            usage_event_id, prompt_tokens, completion_tokens, duration_ms, sources_json,
            confidence, params_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            id,
            e.subject_kind.as_str(),
            e.subject_id.trim(),
            e.subject_revision,
            now_ms,
            e.operation,
            e.actor_kind.as_str(),
            clean_opt(&e.actor_ref),
            clip_opt(&e.provider),
            e.locality.map(Locality::as_str),
            clip_opt(&e.model_id),
            clip_opt(&e.model_label),
            e.usage_event_id,
            e.prompt_tokens.map(to_i64),
            e.completion_tokens.map(to_i64),
            e.duration_ms.map(to_i64),
            sources_json,
            e.confidence,
            e.params.as_ref().map(|p| redact_params(p).to_string()),
        ],
    )?;
    Ok(id)
}

fn enum_error(column: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("unbekannter Wert in {column}: {value}").into(),
    )
}

fn map_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProvenanceEntry> {
    let subject_kind: String = row.get("subject_kind")?;
    let actor_kind: String = row.get("actor_kind")?;
    let locality: Option<String> = row.get("locality")?;
    let sources_json: String = row.get("sources_json")?;
    let count = |name: &str| -> rusqlite::Result<Option<u64>> {
        Ok(row.get::<_, Option<i64>>(name)?.map(|v| v.max(0) as u64))
    };
    Ok(ProvenanceEntry {
        id: row.get("id")?,
        subject_kind: SubjectKind::parse(&subject_kind)
            .ok_or_else(|| enum_error("subject_kind", &subject_kind))?,
        subject_id: row.get("subject_id")?,
        subject_revision: row.get("subject_revision")?,
        created_at: row.get("created_at")?,
        operation: row.get("operation")?,
        actor_kind: Some(
            ActorKind::parse(&actor_kind).ok_or_else(|| enum_error("actor_kind", &actor_kind))?,
        ),
        actor_ref: row.get("actor_ref")?,
        provider: row.get("provider")?,
        locality: locality.as_deref().and_then(Locality::parse),
        model_id: row.get("model_id")?,
        model_label: row.get("model_label")?,
        usage_event_id: row.get("usage_event_id")?,
        prompt_tokens: count("prompt_tokens")?,
        completion_tokens: count("completion_tokens")?,
        duration_ms: count("duration_ms")?,
        sources: serde_json::from_str(&sources_json).unwrap_or_default(),
        confidence: row.get("confidence")?,
        params_json: row.get("params_json")?,
        origin: ProvenanceOrigin::Recorded,
    })
}

/// Die gespeicherten Eintraege eines Inhalts, aelteste zuerst.
pub fn list(
    conn: &Connection,
    kind: SubjectKind,
    id: &str,
) -> Result<Vec<ProvenanceEntry>, ProvenanceError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM provenance WHERE subject_kind = ?1 AND subject_id = ?2
         ORDER BY created_at, id",
    )?;
    let rows = stmt
        .query_map(params![kind.as_str(), id], map_entry)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Herkunft eines Inhalts fuer die Oberflaeche: die gespeicherten Eintraege;
/// gibt es keine, der aus Altdaten abgeleitete (siehe Moduldoku); sonst leer.
pub fn get(
    conn: &Connection,
    kind: SubjectKind,
    id: &str,
) -> Result<Vec<ProvenanceEntry>, ProvenanceError> {
    let recorded = list(conn, kind, id)?;
    if !recorded.is_empty() {
        return Ok(recorded);
    }
    Ok(derive_legacy(conn, kind, id)?.into_iter().collect())
}

/// Anbieter-Kennung -> lokal? Aus Altdaten kennen wir nur die Kennung: der
/// eingebaute lokale Anbieter ist lokal, alles andere bleibt unbekannt (ein
/// selbst betriebener Ollama laesst sich an der Kennung nicht erkennen).
fn legacy_locality(provider: Option<&str>) -> Option<Locality> {
    (provider == Some(crate::managers::llm::LOCAL_PROVIDER_ID)).then_some(Locality::Local)
}

fn operation_for_document_kind(kind: &str) -> String {
    let op = match kind {
        "enhanced_notes" => "notes",
        other => other,
    };
    let cleaned: String = op
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(48)
        .collect();
    if cleaned.is_empty() {
        "document".to_string()
    } else {
        cleaned
    }
}

/// Leitet die Herkunft aus den Daten der Zeit vor A1 ab (siehe Moduldoku).
/// `None`: nichts Ableitbares (unbekannter Inhalt, Dokument ohne Metadaten).
pub fn derive_legacy(
    conn: &Connection,
    kind: SubjectKind,
    id: &str,
) -> Result<Option<ProvenanceEntry>, ProvenanceError> {
    match kind {
        SubjectKind::Document => derive_document(conn, id),
        SubjectKind::Transcript => derive_transcript(conn, id),
        _ => Ok(None),
    }
}

fn derive_document(
    conn: &Connection,
    id: &str,
) -> Result<Option<ProvenanceEntry>, ProvenanceError> {
    type DocRow = (String, String, Option<String>, i64, i64, Option<String>);
    let row: Option<DocRow> = conn
        .query_row(
            "SELECT meeting_id, kind, template_id, version, created_at, generation_metadata_json
             FROM meeting_documents WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((meeting_id, doc_kind, template_id, version, created_secs, metadata)) = row else {
        return Ok(None);
    };
    let Some(metadata) = metadata.filter(|m| !m.trim().is_empty()) else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&metadata) else {
        return Ok(None);
    };
    if !value.is_object() {
        return Ok(None);
    }
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(clip)
            .filter(|t| !t.trim().is_empty())
    };
    let provider = text("provider");
    let model = text("model");
    let mut params = redact_params(&value);
    if let Some(template) = &template_id {
        if params.get("template_id").is_none() {
            params["template_id"] = serde_json::json!(template);
        }
    }
    let params_json = Some(params.to_string()).filter(|p| p.len() <= MAX_PARAMS_BYTES);
    Ok(Some(ProvenanceEntry {
        id: format!("legacy:{id}"),
        subject_kind: SubjectKind::Document,
        subject_id: id.to_string(),
        subject_revision: Some(version),
        created_at: created_secs.saturating_mul(1000),
        operation: operation_for_document_kind(&doc_kind),
        actor_kind: None,
        actor_ref: None,
        locality: legacy_locality(provider.as_deref()),
        provider,
        model_label: model.clone(),
        model_id: model,
        usage_event_id: None,
        prompt_tokens: None,
        completion_tokens: None,
        duration_ms: None,
        sources: vec![SourceRef::new("transcript", &meeting_id, None)],
        confidence: None,
        params_json,
        origin: ProvenanceOrigin::Derived,
    }))
}

fn derive_transcript(
    conn: &Connection,
    meeting_id: &str,
) -> Result<Option<ProvenanceEntry>, ProvenanceError> {
    let row: Option<(Option<String>, Option<String>, Option<String>, i64, i64)> = conn
        .query_row(
            "SELECT provider, model, language, content_revision, updated_at
             FROM transcripts WHERE meeting_id = ?1 AND deleted_at IS NULL",
            params![meeting_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((provider, model, language, revision, updated_secs)) = row else {
        return Ok(None);
    };
    let Some(model) = model.filter(|m| !m.trim().is_empty()) else {
        return Ok(None);
    };
    let provider = provider.filter(|p| !p.trim().is_empty());
    let params = language
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::json!({ "language": l }).to_string());
    Ok(Some(ProvenanceEntry {
        id: format!("legacy:transcript:{meeting_id}"),
        subject_kind: SubjectKind::Transcript,
        subject_id: meeting_id.to_string(),
        subject_revision: Some(revision),
        created_at: updated_secs.saturating_mul(1000),
        operation: "stt".to_string(),
        actor_kind: None,
        actor_ref: None,
        // Die Spracherkennung der App laeuft immer auf diesem Rechner.
        locality: Some(Locality::Local),
        provider,
        model_label: Some(clip(&model)),
        model_id: Some(clip(&model)),
        usage_event_id: None,
        prompt_tokens: None,
        completion_tokens: None,
        duration_ms: None,
        sources: vec![SourceRef::new("audio", meeting_id, None)],
        confidence: None,
        params_json: params,
        origin: ProvenanceOrigin::Derived,
    }))
}

#[cfg(test)]
mod tests;
