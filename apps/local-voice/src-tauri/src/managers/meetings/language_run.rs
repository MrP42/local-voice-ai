//! G5: die Anbindung der Spracherkennung an die Pipelines (Import, Neu-Transkription,
//! YouTube-Audio, Enddurchlauf). Die Entscheidungen selbst (`language.rs`) sind rein und
//! dort getestet; hier steht, was Modell, Speicher und Store braucht.
//!
//! Ablauf eines Laufs:
//! 1. [`prepare`] vor der ersten Transkription: steht die Sprache fest (Dialog der
//!    Neu-Transkription, feste Einstellung `meeting_language`), gilt sie. Sonst eine Probe
//!    aus dem Audio (bis zu drei kurze Fenster) auf dem schon geladenen Modell. Dann die
//!    Modellwahl (`choose_model`): Nutzerwahl bleibt; ein Modell, das die Sprache nicht
//!    abdeckt, wird durch ein installiertes mehrsprachiges ersetzt, aber nur durch das
//!    RAM-Tor (`process_guard::check_ram_for_start`) und nie unter einer eigenen Engine
//!    der Warteschlange. Gelingt das Laden nicht, bleibt das bisherige Modell.
//! 2. Der Lauf transkribiert mit der Vorgabe (`LanguageOverride`, je Thread).
//! 3. [`finalize`] danach: die Gegenprobe am fertigen Transkript (`settle`) und die
//!    Ablage (Spalte der Besprechung, Transkript, aktive Fassung, `metadata_json`).
//!
//! Fehlerfaelle: eine gescheiterte Probe (Modell laedt nicht, Panik der Engine) ergibt
//! `None` und der Lauf geht weiter wie vor G5 (Sprache wie eingestellt). Nichts hier darf
//! einen Import scheitern lassen; Fehler stehen nur im Log.

use std::sync::Arc;
use std::time::Duration;

use log::{info, warn};
use tauri::Manager;

use super::job::JobHandle;
use super::language::{
    build_info, decide_run, detect_text, needs_probe, normalize_code, sample_windows, settle,
    LanguageInfo, ModelCandidate, ModelReason, ProbeResult, RunInput, StoredLanguage,
    METADATA_KEY, SOURCE_USER,
};
use super::store::MeetingStore;
use super::variants;
use crate::managers::model::{ModelInfo, ModelManager};
use crate::managers::transcription::{
    extra_engine_bound, LanguageOverride, MeetingModelChoice, TranscriptionManager,
};

/// So lange wartet ein Modellwechsel auf das Laden, bevor er aufgegeben wird.
const SWITCH_LOAD_TIMEOUT: Duration = Duration::from_secs(300);
/// Wie viel Text des Transkripts die Gegenprobe liest (Zeichen).
const TEXT_SAMPLE_CHARS: usize = 3_000;

/// Was der Nutzer fuer diesen Lauf ausdruecklich gewaehlt hat (Dialog der Neu-Transkription).
#[derive(Clone, Debug, Default)]
pub struct RunRequest {
    /// Sprachcode; `auto` oder leer = erkennen.
    pub language: Option<String>,
    /// Modell-ID; leer = die Einstellungen entscheiden.
    pub model_id: Option<String>,
}

impl RunRequest {
    fn model(&self) -> Option<&str> {
        self.model_id.as_deref().map(str::trim).filter(|m| !m.is_empty())
    }
}

/// Das Ergebnis von [`prepare`]. Haelt die Vorgabe fuer den Thread, solange es lebt.
pub struct LanguagePlan {
    /// Sprache des Laufs mit Herkunft und Sicherheit; `None`: unbekannt.
    pub planned: Option<(String, &'static str, f64)>,
    /// Das Modell, mit dem der Lauf rechnet.
    pub model_id: String,
    pub reason: ModelReason,
    _guard: LanguageOverride,
}

impl LanguagePlan {
    /// Ein Plan mit gegebener Sprache, ohne Modell und Speicher (Tests).
    #[cfg(test)]
    pub fn planned_for_test(planned: Option<(String, &'static str, f64)>, model_id: &str) -> Self {
        Self {
            planned,
            model_id: model_id.to_string(),
            reason: ModelReason::Covers,
            _guard: LanguageOverride::set(None, None),
        }
    }

    /// Ein Plan ohne Vorgabe (kein Audio vorbereitet): der Lauf rechnet wie vor G5.
    pub fn none(model_id: &str) -> Self {
        Self {
            planned: None,
            model_id: model_id.to_string(),
            reason: ModelReason::Unknown,
            _guard: LanguageOverride::set(None, None),
        }
    }
}

pub fn candidate_of(info: &ModelInfo) -> ModelCandidate {
    ModelCandidate {
        id: info.id.clone(),
        name: info.name.clone(),
        languages: info.supported_languages.clone(),
        downloaded: info.is_downloaded,
        size_mb: info.size_mb,
        accuracy: (info.accuracy_score.clamp(0.0, 1.0) * 100.0).round() as u32,
    }
}

/// Alle Modelle des Katalogs, wie die Wahl sie sieht.
pub fn candidates(models: &ModelManager) -> Vec<ModelCandidate> {
    models.get_available_models().iter().map(candidate_of).collect()
}

fn unknown_candidate(id: &str) -> ModelCandidate {
    ModelCandidate {
        id: id.to_string(),
        name: id.to_string(),
        languages: Vec::new(),
        downloaded: true,
        size_mb: 0,
        accuracy: 0,
    }
}

/// Die Probe: je Fenster ein Lauf auf dem geladenen Modell mit Sprache `auto`. Ein Fehler
/// in einem Fenster zaehlt als "keine Erkennung", nie als Abbruch.
pub fn probe(tm: &TranscriptionManager, windows: Vec<Vec<f32>>) -> ProbeResult {
    let _auto = LanguageOverride::set(Some("auto".to_string()), LanguageOverride::current_model());
    let mut result = ProbeResult::default();
    for window in windows {
        match tm.transcribe_segments_detecting(window) {
            Ok((segments, detected)) => {
                result.native.push(detected);
                for segment in segments {
                    result.text.push_str(segment.text.trim());
                    result.text.push(' ');
                }
            }
            Err(e) => {
                warn!("meetings: language probe window failed: {e}");
                result.native.push(None);
            }
        }
    }
    result
}

/// Ein Modellwechsel mit Tor und Rueckweg: nie unter einer eigenen Engine der
/// Warteschlange, nie ohne freien Speicher, und gelingt das Laden nicht, ist das
/// bisherige Modell wieder da. `true`: `target` ist geladen.
fn switch_model(
    app: &tauri::AppHandle,
    tm: &TranscriptionManager,
    current: &str,
    target: &str,
) -> bool {
    if extra_engine_bound() {
        info!("meetings: model switch to '{target}' skipped (own engine of the import queue)");
        return false;
    }
    let size_mb = app
        .try_state::<Arc<ModelManager>>()
        .and_then(|m| m.get_model_info(target))
        .map(|i| i.size_mb)
        .unwrap_or(0);
    if let Err(message) =
        crate::process_guard::check_ram_for_start(super::final_pass::ram_need_mb(size_mb))
    {
        warn!("meetings: model switch to '{target}' refused by the RAM gate: {message}");
        return false;
    }
    tm.initiate_model_load_target(target);
    if tm.wait_until_loaded(target, SWITCH_LOAD_TIMEOUT) {
        info!("meetings: model switched to '{target}' for the detected language");
        return true;
    }
    warn!("meetings: model '{target}' did not load, back to '{current}'");
    tm.initiate_model_load_target(current);
    let _ = tm.wait_until_loaded(current, SWITCH_LOAD_TIMEOUT);
    false
}

/// Vor der ersten Transkription: Sprache bestimmen, Modell waehlen, Vorgabe fuer den
/// aktuellen Thread setzen. Die Vorgabe gilt, bis der Plan faellt.
pub fn prepare(
    app: &tauri::AppHandle,
    tm: &TranscriptionManager,
    samples: &[i16],
    request: &RunRequest,
    job: Option<&Arc<JobHandle>>,
) -> LanguagePlan {
    let settings = crate::settings::get_settings(app);
    let Some(models) = app.try_state::<Arc<ModelManager>>() else {
        return LanguagePlan::none(&tm.meeting_model_target(&settings));
    };
    let catalog = candidates(&models);
    let choice_before = tm.meeting_model_choice(&settings);
    let (current_id, user_model) = match request.model() {
        Some(id) => (id.to_string(), true),
        None => (
            choice_before.id().to_string(),
            matches!(choice_before, MeetingModelChoice::Explicit(_)),
        ),
    };
    let current = catalog
        .iter()
        .find(|c| c.id == current_id)
        .cloned()
        .unwrap_or_else(|| unknown_candidate(&current_id));

    // Die Probe nur, wenn nichts die Sprache festlegt und das Modell sie erkennen kann.
    let can_detect = models
        .get_model_info(&current_id)
        .is_some_and(|i| i.supports_language_detection);
    let probe_result = if needs_probe(request.language.as_deref(), &settings.meeting_language) {
        if can_detect {
            if let Some(job) = job {
                job.begin_phase(super::job::JobPhase::Prepare, 0);
            }
            Some(probe(tm, sample_windows(samples)))
        } else {
            None
        }
    } else {
        None
    };

    let decision = decide_run(
        &RunInput {
            user_language: request.language.as_deref(),
            setting_language: &settings.meeting_language,
            current: &current,
            user_model,
            catalog: &catalog,
        },
        probe_result.as_ref(),
    );
    let mut model_id = current_id.clone();
    let mut force = decision.force.clone();
    let mut reason = decision.choice.reason;
    if decision.choice.reason == ModelReason::Switched && decision.choice.id != current_id {
        if switch_model(app, tm, &current_id, &decision.choice.id) {
            model_id = decision.choice.id.clone();
        } else {
            // Nicht gewechselt: das bisherige Modell deckt die Sprache nicht ab, sie wird
            // ihm nicht aufgezwungen (sein eigenes `auto` bzw. Englisch gilt).
            force = None;
            reason = ModelReason::NoneInstalled;
        }
    }
    info!(
        "meetings: language {:?}, model '{model_id}' ({reason:?})",
        decision.planned.as_ref().map(|p| (&p.0, p.1))
    );
    let model_override = (model_id != choice_before.id()).then(|| model_id.clone());
    LanguagePlan {
        planned: decision.planned,
        model_id,
        reason,
        _guard: LanguageOverride::set(force, model_override),
    }
}

/// Ein Auszug des aktiven Transkripts fuer die Gegenprobe.
fn transcript_sample(store: &MeetingStore, meeting_id: &str) -> String {
    let Ok(segments) = store.get_segments(meeting_id) else {
        return String::new();
    };
    let mut sample = String::new();
    for segment in segments {
        if super::import::is_gap_placeholder(&segment.text) {
            continue;
        }
        sample.push_str(segment.text.trim());
        sample.push(' ');
        if sample.chars().count() >= TEXT_SAMPLE_CHARS {
            break;
        }
    }
    sample
}

/// Die Sprache ablegen: Spalte der Besprechung, Transkript und aktive Fassung (eine
/// Transaktion in `variants::set_language`) und die Herkunft in `metadata_json`.
pub fn persist(store: &MeetingStore, meeting_id: &str, stored: &StoredLanguage) -> Result<(), String> {
    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    variants::set_language(&mut conn, meeting_id, &stored.code).map_err(|e| e.to_string())?;
    let value = serde_json::to_value(stored).map_err(|e| e.to_string())?;
    store
        .set_metadata_key(meeting_id, METADATA_KEY, value)
        .map_err(|e| e.to_string())
}

/// Die gespeicherte Herkunft der Sprache (leer bei Altbestand).
pub fn stored_language(store: &MeetingStore, meeting_id: &str) -> Option<StoredLanguage> {
    store
        .metadata_json(meeting_id)
        .ok()
        .flatten()
        .and_then(|meta| meta.get(METADATA_KEY).cloned())
        .and_then(|value| serde_json::from_value(value).ok())
}

/// Nach dem Lauf: Gegenprobe am Transkript und Ablage. Nie ein Fehler nach aussen.
pub fn finalize(store: &MeetingStore, meeting_id: &str, plan: &LanguagePlan, model_id: Option<String>) {
    let text = detect_text(&transcript_sample(store, meeting_id));
    let model_id = model_id.or_else(|| Some(plan.model_id.clone()));
    let Some(stored) = settle(plan.planned.clone(), text, model_id) else {
        return;
    };
    if let Err(e) = persist(store, meeting_id, &stored) {
        warn!("meetings: language not stored ({meeting_id}): {e}");
    }
}

/// Fuer Wege ohne Plan (Enddurchlauf einer Live-Aufnahme): die Sprache aus dem Text. Eine
/// Sprache, die der Nutzer gewaehlt hat, bleibt unberuehrt.
pub fn finalize_from_text(store: &MeetingStore, meeting_id: &str, model_id: Option<String>) {
    if stored_language(store, meeting_id).is_some_and(|s| s.source == SOURCE_USER) {
        return;
    }
    let Some(stored) = settle(None, detect_text(&transcript_sample(store, meeting_id)), model_id)
    else {
        return;
    };
    if let Err(e) = persist(store, meeting_id, &stored) {
        warn!("meetings: language not stored ({meeting_id}): {e}");
    }
}

/// Das Modell, das das aktive Transkript geschrieben hat (Spalte `transcripts.model`).
fn transcript_model(store: &MeetingStore, meeting_id: &str) -> Option<String> {
    let conn = store.get_connection().ok()?;
    conn.query_row(
        "SELECT model FROM transcripts WHERE meeting_id = ?1 AND deleted_at IS NULL",
        rusqlite::params![meeting_id],
        |r| r.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

/// Der Chip im Kopf: Sprache, Herkunft, Modell und ein besserer Vorschlag.
pub fn info(
    store: &MeetingStore,
    catalog: &[ModelCandidate],
    meeting_id: &str,
) -> Result<LanguageInfo, String> {
    let meeting = store
        .get_meeting(meeting_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "meeting_not_found".to_string())?;
    let stored = stored_language(store, meeting_id);
    let model = stored
        .as_ref()
        .and_then(|s| s.model_id.clone())
        .or_else(|| transcript_model(store, meeting_id));
    Ok(build_info(
        stored.as_ref(),
        meeting.language.as_deref(),
        model.as_deref(),
        catalog,
    ))
}

/// Das Modell, das die App fuer eine Sprache waehlen wuerde (fuer den Dialog der
/// Neu-Transkription): die Nutzerwahl der Einstellung bleibt ausserhalb, hier zaehlt nur
/// die Eignung. `None`, wenn der Katalog keine Antwort hat.
pub fn model_for_language(
    catalog: &[ModelCandidate],
    current_id: &str,
    language: &str,
) -> Option<super::language::ModelSuggestion> {
    let code = normalize_code(language)?;
    let current = catalog
        .iter()
        .find(|c| c.id == current_id)
        .cloned()
        .unwrap_or_else(|| unknown_candidate(current_id));
    let choice = super::language::choose_model(Some(&code), &current, false, catalog);
    let id = match choice.reason {
        ModelReason::Switched => choice.id,
        _ => choice.suggestion.unwrap_or(choice.id),
    };
    catalog.iter().find(|m| m.id == id).map(|m| super::language::ModelSuggestion {
        model_id: m.id.clone(),
        name: m.name.clone(),
        downloaded: m.downloaded,
    })
}

#[cfg(test)]
mod tests;
