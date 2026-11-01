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

use std::sync::{Arc, Mutex};
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

/// Eine Sperre für alle Schreiber des Wortschatzes im Lernpfad: lesen, ändern, schreiben läuft
/// am Stück, damit sich zwei Aufrufe nichts überschreiben.
static WORDS_LOCK: Mutex<()> = Mutex::new(());

/// Führt `f` auf einem Blockier-Thread gegen den Speicher aus, mit frisch gelesenem Wortschatz.
/// Die Änderung am Wortschatz, die `f` meldet, geht noch unter derselben Sperre in die
/// Einstellungen; entfernt wird nur ein Begriff in genau der Schreibweise, die dort steht.
async fn run_words<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&SpeechLearningStore, &[String]) -> anyhow::Result<(T, WordsDelta)> + Send + 'static,
{
    let handle = app.clone();
    run(app, move |store| {
        let _guard = WORDS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut settings = get_settings(&handle);
        let (value, delta) = f(store, &settings.custom_words)?;
        if !delta.is_empty() && delta.apply(&mut settings.custom_words) {
            write_settings(&handle, settings);
        }
        Ok(value)
    })
    .await
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
    run_words(&app, move |s, words| {
        s.feedback(&bereich, &erkannt, &korrigiert, auto_apply, words)
    })
    .await
}

/// Die gelernten Paare eines Bereichs, jüngste zuerst. Gleicht dabei ab, welche übernommenen
/// Begriffe noch im Wortschatz stehen.
#[tauri::command]
#[specta::specta]
pub async fn speech_learning_list(
    app: AppHandle,
    bereich: String,
) -> Result<Vec<LearnedPair>, String> {
    run_words(&app, move |s, words| {
        Ok((s.list(&bereich, words)?, WordsDelta::default()))
    })
    .await
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
    run_words(&app, move |s, words| s.set_status(id, &status, words)).await
}

/// Löscht ein Paar wirklich.
#[tauri::command]
#[specta::specta]
pub async fn speech_learning_delete(app: AppHandle, id: u32) -> Result<(), String> {
    run_words(&app, move |s, words| Ok(((), s.delete(id, words)?))).await
}

/// Löscht alle Paare eines Bereichs wirklich; Rückgabe: wie viele es waren.
#[tauri::command]
#[specta::specta]
pub async fn speech_learning_clear(app: AppHandle, bereich: String) -> Result<u32, String> {
    run_words(&app, move |s, words| s.clear(&bereich, words)).await
}

/// Der Vorlesetext für die Einrichtung: fester Grundtext plus der eigene Wortschatz.
#[tauri::command]
#[specta::specta]
pub fn speech_learning_setup_text(app: AppHandle) -> SetupText {
    setup::setup_text(&get_settings(&app).custom_words)
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
    run_words(&app, move |s, words| {
        Ok((
            s.setup_evaluate(&bereich, &reference, &transcript, words)?,
            WordsDelta::default(),
        ))
    })
    .await
}

/// „Aus Korrekturen lernen“ (Standard aus).
#[tauri::command]
#[specta::specta]
pub fn change_speech_learning_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let _guard = WORDS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _guard = WORDS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut settings = get_settings(&app);
    settings.speech_learning_auto_apply = enabled;
    write_settings(&app, settings);
    Ok(())
}
