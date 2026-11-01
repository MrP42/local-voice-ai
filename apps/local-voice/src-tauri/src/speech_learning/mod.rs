//! Lernen aus Korrekturen und Einrichtung der Spracherkennung durch Vorlesen (Stufe 3 des
//! Sprachgesprächs, wai-sdk ADR-0021).
//!
//! * `diff`: Wort-Diff, trennt Verhörer von Umformulierungen (rein, ohne Tauri).
//! * `store`: SQLite-Lernspeicher `speech_learning.db` (Paare je Bereich).
//! * `setup`: Vorlesetext.
//!
//! Der Lernspeicher baut keinen zweiten Weg zur Erkennung: Übernommene Begriffe landen im
//! vorhandenen eigenen Wortschatz (`custom_words`), den die Erkennung schon nutzt (Hinweis
//! an Whisper und unscharfe Korrektur im Textpfad). Lernen ist standardmäßig AUS
//! (`speech_learning_enabled`); `speech_learning_auto_apply` gilt nur, wenn Lernen an ist.
//!
//! Alle Befehle laufen auf einem Blockier-Thread. Weder Texte noch Paare stehen im Log oder in
//! Fehlermeldungen. Audio der Einrichtung bleibt im Speicher und wird nie gespeichert.

pub mod diff;
pub mod setup;
pub mod store;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::managers::transcription::TranscriptionManager;
use crate::settings::{get_settings, write_settings};
use setup::SetupText;
use store::{FeedbackOutcome, LearnedPair, SetupOutcome, SpeechLearningStore, WordsDelta};

/// Öffnet den Speicher im App-Datenordner (für `lib.rs`).
pub fn open_store(app: &AppHandle) -> anyhow::Result<SpeechLearningStore> {
    let dir = crate::portable::app_data_dir(app)?;
    std::fs::create_dir_all(&dir)?;
    SpeechLearningStore::open(&dir.join("speech_learning.db"))
}

/// Führt `f` auf einem Blockier-Thread gegen den Speicher aus.
async fn run<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&SpeechLearningStore) -> anyhow::Result<T> + Send + 'static,
{
    let store = app
        .try_state::<Arc<SpeechLearningStore>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "Der Lernspeicher ist nicht verfügbar.".to_string())?;
    tauri::async_runtime::spawn_blocking(move || f(&store))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Schreibt die Änderung am Wortschatz in die Einstellungen.
fn apply_delta(app: &AppHandle, delta: &WordsDelta) {
    if delta.is_empty() {
        return;
    }
    let mut settings = get_settings(app);
    if delta.apply(&mut settings.custom_words) {
        write_settings(app, settings);
    }
}

fn current_words(app: &AppHandle) -> Vec<String> {
    get_settings(app).custom_words
}

/// Meldet eine Korrektur: `erkannt` ist, was die Erkennung geliefert hat, `korrigiert`, was der
/// Nutzer daraus gemacht hat. Ist Lernen aus (Standard), passiert nichts (`reason` = `off`).
#[tauri::command]
#[specta::specta]
pub async fn speech_learning_feedback(
    app: AppHandle,
    bereich: String,
    erkannt: String,
    korrigiert: String,
) -> Result<FeedbackOutcome, String> {
    diff::check_bereich(&bereich)?;
    diff::check_text(&erkannt)?;
    diff::check_text(&korrigiert)?;
    let settings = get_settings(&app);
    if !settings.speech_learning_enabled {
        return Ok(FeedbackOutcome::empty("off"));
    }
    let auto_apply = settings.speech_learning_auto_apply;
    let words = settings.custom_words;
    let (outcome, delta) = run(&app, move |s| {
        s.feedback(&bereich, &erkannt, &korrigiert, auto_apply, &words)
    })
    .await?;
    apply_delta(&app, &delta);
    Ok(outcome)
}

/// Die gelernten Paare eines Bereichs, jüngste zuerst.
#[tauri::command]
#[specta::specta]
pub async fn speech_learning_list(
    app: AppHandle,
    bereich: String,
) -> Result<Vec<LearnedPair>, String> {
    run(&app, move |s| s.list(&bereich)).await
}

/// Setzt den Status eines Paares: `vorschlag`, `uebernommen` oder `abgelehnt`. Übernehmen trägt
/// den Begriff in den eigenen Wortschatz ein, alles andere nimmt zurück, was von hier stammt.
#[tauri::command]
#[specta::specta]
pub async fn speech_learning_set_status(
    app: AppHandle,
    id: u32,
    status: String,
) -> Result<LearnedPair, String> {
    let words = current_words(&app);
    let (pair, delta) = run(&app, move |s| s.set_status(id, &status, &words)).await?;
    apply_delta(&app, &delta);
    Ok(pair)
}

/// Löscht ein Paar wirklich.
#[tauri::command]
#[specta::specta]
pub async fn speech_learning_delete(app: AppHandle, id: u32) -> Result<(), String> {
    let delta = run(&app, move |s| s.delete(id)).await?;
    apply_delta(&app, &delta);
    Ok(())
}

/// Löscht alle Paare eines Bereichs wirklich; Rückgabe: wie viele es waren.
#[tauri::command]
#[specta::specta]
pub async fn speech_learning_clear(app: AppHandle, bereich: String) -> Result<u32, String> {
    let (count, delta) = run(&app, move |s| s.clear(&bereich)).await?;
    apply_delta(&app, &delta);
    Ok(count)
}

/// Der Vorlesetext für die Einrichtung: fester Grundtext plus der eigene Wortschatz.
#[tauri::command]
#[specta::specta]
pub fn speech_learning_setup_text(app: AppHandle) -> SetupText {
    setup::setup_text(&current_words(&app))
}

/// Lädt das Spracherkennungsmodell, damit das Vorlesen gleich erkannt wird (die Aufnahme geht
/// danach wie im Gespräch an `voice_call_transcribe`).
#[tauri::command]
#[specta::specta]
pub async fn speech_learning_setup_prepare(app: AppHandle) -> Result<(), String> {
    let stt = app.state::<Arc<TranscriptionManager>>().inner().clone();
    let model = get_settings(&app).selected_model;
    let ready = tokio::task::spawn_blocking(move || {
        stt.initiate_model_load();
        stt.wait_until_loaded(&model, Duration::from_secs(90))
    })
    .await
    .unwrap_or(false);
    if ready {
        Ok(())
    } else {
        Err("Das Spracherkennungsmodell konnte nicht geladen werden (Einstellungen → Eingabe → Modell).".into())
    }
}

/// Vergleicht die Erkennung des Vorlesens mit dem Vorlesetext und legt die Verhörer als
/// Vorschläge ab. Bleibt vom Schalter „Aus Korrekturen lernen“ unberührt: Der Nutzer hat die
/// Einrichtung selbst gestartet. Übernommen wird nichts ohne seinen Klick.
#[tauri::command]
#[specta::specta]
pub async fn speech_learning_setup_evaluate(
    app: AppHandle,
    bereich: String,
    reference: String,
    transcript: String,
) -> Result<SetupOutcome, String> {
    run(&app, move |s| {
        s.setup_evaluate(&bereich, &reference, &transcript)
    })
    .await
}

/// „Aus Korrekturen lernen“ (Standard aus).
#[tauri::command]
#[specta::specta]
pub fn change_speech_learning_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.speech_learning_enabled = enabled;
    write_settings(&app, settings);
    Ok(())
}

/// „Vorschläge automatisch übernehmen“ (Standard aus; wirkt nur bei eingeschaltetem Lernen).
#[tauri::command]
#[specta::specta]
pub fn change_speech_learning_auto_apply_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.speech_learning_auto_apply = enabled;
    write_settings(&app, settings);
    Ok(())
}
