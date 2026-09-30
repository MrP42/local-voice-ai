//! Brueckenbau zwischen den Erzeugungsstellen und `provenance::record`:
//! aus den Aufrufen, die ein Erfassungsbereich (`usage::with_capture`)
//! mitgeschrieben hat, wird ein Eintrag (Modell, Token, Verweis auf das
//! Ledger-Ereignis, Dauer).
//!
//! Regel: Die Provenienz darf eine Erzeugung NIE scheitern lassen. Jeder Fehler
//! hier wird protokolliert (ohne Inhalt), der Aufrufer bekommt `None`; `get`
//! faellt dann auf die Altdaten zurueck.

use std::time::Instant;

use serde_json::{json, Value};

use super::{
    ActorKind, Locality, NewProvenance, ProvenanceError, SourceRef, SubjectKind, MAX_PARAMS_BYTES,
};
use crate::managers::meetings::store::MeetingStore;
use crate::managers::usage::{self, CapturedCall};
use crate::settings::PostProcessProvider;

/// Hoechstzahl Ereignisnummern in `params.usage_event_ids`.
const MAX_EVENT_IDS: usize = 100;

/// Anbieter und Modell des Laufs fuer den Fall, dass keine Aufrufe erfasst
/// wurden (kein Erfassungsbereich, kein Ledger).
pub struct Fallback<'a> {
    pub provider: &'a PostProcessProvider,
    pub model: &'a str,
}

/// Was eine Erzeugungsstelle ueber den Lauf weiss.
pub struct Generation<'a> {
    pub subject_kind: SubjectKind,
    pub subject_id: &'a str,
    pub subject_revision: Option<i64>,
    pub operation: &'a str,
    pub actor_kind: ActorKind,
    pub actor_ref: Option<&'a str>,
    /// Beginn des Laufs: die Dauer im Eintrag ist die Wanduhrzeit bis jetzt.
    pub started: Instant,
    pub sources: Vec<SourceRef>,
    /// Weitere Angaben (JSON-Objekt oder `Null`).
    pub params: Value,
    pub fallback: Option<Fallback<'a>>,
}

/// Baut den Eintrag aus den erfassten Aufrufen (rein, ohne Datenbank).
///
/// - Modell/Anbieter: der letzte erfolgreiche Aufruf, sonst der letzte, sonst
///   `fallback`.
/// - Token: Summe aller Aufrufe. Meldet der Anbieter keine (Summe 0), bleiben
///   sie leer -- "unbekannt" statt einer falschen Null.
/// - `usage_event_id`: die kleinste Nummer des Laufs; alle stehen in
///   `params.usage_event_ids`.
/// - Dauer: Wanduhrzeit des ganzen Laufs; die Summe der Modellzeiten steht in
///   `params.llm_ms`.
pub fn build(g: &Generation<'_>, calls: &[CapturedCall], dropped: u32) -> NewProvenance {
    let mut e = NewProvenance::new(g.subject_kind, g.subject_id, g.operation, g.actor_kind);
    e.subject_revision = g.subject_revision;
    e.actor_ref = g.actor_ref.map(str::to_string);
    e.sources = g.sources.clone();
    e.duration_ms = Some(u64::try_from(g.started.elapsed().as_millis()).unwrap_or(u64::MAX));

    let main = calls.iter().rev().find(|c| c.ok).or_else(|| calls.last());
    match (main, &g.fallback) {
        (Some(c), _) => {
            e.provider = Some(c.provider_id.clone());
            e.locality = Some(if c.local {
                Locality::Local
            } else {
                Locality::Remote
            });
            e.model_id = Some(c.model_id.clone());
            e.model_label = Some(c.model_label.clone());
        }
        (None, Some(f)) => {
            e.provider = Some(f.provider.id.clone());
            e.locality = Some(if usage::provider_is_local(f.provider) {
                Locality::Local
            } else {
                Locality::Remote
            });
            e.model_id = Some(f.model.to_string());
            e.model_label = Some(f.model.to_string());
        }
        (None, None) => {}
    }

    let prompt: u64 = calls.iter().map(|c| c.prompt_tokens).sum();
    let completion: u64 = calls.iter().map(|c| c.completion_tokens).sum();
    if prompt + completion > 0 {
        e.prompt_tokens = Some(prompt);
        e.completion_tokens = Some(completion);
    }
    let ids: Vec<i64> = calls.iter().filter_map(|c| c.usage_event_id).collect();
    e.usage_event_id = ids.iter().copied().min();

    let mut params = match &g.params {
        Value::Object(map) => Value::Object(map.clone()),
        _ => json!({}),
    };
    if !calls.is_empty() {
        params["calls"] = json!(calls.len() as u64 + u64::from(dropped));
        params["failed_calls"] = json!(calls.iter().filter(|c| !c.ok).count());
        params["llm_ms"] = json!(calls.iter().map(|c| u64::from(c.duration_ms)).sum::<u64>());
        params["usage_event_ids"] = json!(ids.iter().take(MAX_EVENT_IDS).collect::<Vec<_>>());
        if let Some(c) = main {
            params["connection"] = json!(c.connection_label);
        }
        let mut models: Vec<&str> = calls.iter().map(|c| c.model_id.as_str()).collect();
        models.sort_unstable();
        models.dedup();
        if models.len() > 1 {
            params["models"] = json!(models);
        }
    }
    if dropped > 0 {
        params["dropped_calls"] = json!(dropped);
    }
    e.params = Some(params);
    e
}

/// Schreibt die Provenienz eines erzeugten Inhalts aus dem aktuellen
/// Erfassungsbereich. Gibt die ID des Eintrags zurueck, oder `None` (mit
/// Warnung), wenn er sich nicht schreiben liess.
pub fn record_generation(store: &MeetingStore, g: Generation<'_>) -> Option<String> {
    let (calls, dropped) = usage::captured_calls();
    let entry = build(&g, &calls, dropped);
    write(store, &entry, g.operation)
}

fn write(store: &MeetingStore, entry: &NewProvenance, operation: &str) -> Option<String> {
    let result = (|| -> Result<String, ProvenanceError> {
        let conn = store
            .get_connection()
            .map_err(|e| ProvenanceError::Store(e.to_string()))?;
        super::record(&conn, entry)
    })();
    match result {
        Ok(id) => Some(id),
        Err(e) => {
            log::warn!("Provenienz ({operation}) nicht geschrieben: {e}");
            None
        }
    }
}

/// Provenienz einer Spracherkennung (Enddurchlauf, Import): Modell, Dauer,
/// Quelle. Keine Token, kein Ledger-Ereignis (kein Sprachmodell-Aufruf).
pub struct SttRun<'a> {
    /// Besprechungs-ID (= `subject_id` des Transkripts).
    pub meeting_id: &'a str,
    /// `stt` (Enddurchlauf) oder `import`.
    pub operation: &'a str,
    pub actor_kind: ActorKind,
    pub model_id: &'a str,
    pub revision: Option<i64>,
    pub duration_ms: u64,
    /// Quelle: `audio` (Aufnahme) oder `import` (Datei), mit Verweis/Titel.
    pub sources: Vec<SourceRef>,
    pub params: Value,
}

pub fn build_stt(run: &SttRun<'_>) -> NewProvenance {
    let mut e = NewProvenance::new(
        SubjectKind::Transcript,
        run.meeting_id,
        run.operation,
        run.actor_kind,
    );
    e.subject_revision = run.revision;
    e.provider = Some("stt".to_string());
    // Die Spracherkennung laeuft immer auf diesem Rechner.
    e.locality = Some(Locality::Local);
    e.model_id = Some(run.model_id.to_string());
    e.model_label = Some(run.model_id.to_string());
    e.duration_ms = Some(run.duration_ms);
    e.sources = run.sources.clone();
    if run.params.is_object() && run.params.to_string().len() <= MAX_PARAMS_BYTES {
        e.params = Some(run.params.clone());
    }
    e
}

pub fn record_stt(store: &MeetingStore, run: SttRun<'_>) -> Option<String> {
    let entry = build_stt(&run);
    write(store, &entry, run.operation)
}

/// Wie `record_stt`, aber fuer eine Transkript-FASSUNG (A3): der Eintrag haengt an
/// der Kennung der Fassung, nicht an der Besprechung.
pub fn record_stt_variant(
    store: &MeetingStore,
    variant_id: &str,
    run: SttRun<'_>,
) -> Option<String> {
    let mut entry = build_stt(&run);
    entry.subject_kind = SubjectKind::TranscriptVariant;
    entry.subject_id = variant_id.to_string();
    write(store, &entry, run.operation)
}

/// Provenienz eines Transkripts aus einer Untertiteldatei (VTT/SRT): kein Modell,
/// kein Sprachmodell-Aufruf; Quelle ist die Datei.
pub fn build_subtitle_import(meeting_id: &str, file_name: &str, segments: usize) -> NewProvenance {
    let mut e = NewProvenance::new(
        SubjectKind::Transcript,
        meeting_id,
        "subtitles_import",
        ActorKind::User,
    );
    e.sources = vec![SourceRef::new("subtitle", file_name, Some(file_name))];
    e.params = Some(json!({ "segments": segments }));
    e
}

pub fn record_subtitle_import(
    store: &MeetingStore,
    meeting_id: &str,
    file_name: &str,
    segments: usize,
) -> Option<String> {
    write(
        store,
        &build_subtitle_import(meeting_id, file_name, segments),
        "subtitles_import",
    )
}

#[cfg(test)]
mod tests;
