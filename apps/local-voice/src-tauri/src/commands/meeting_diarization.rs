//! Modell der Sprechertrennung (Sortformer, Katalog `Purpose::Diarization`):
//! Stand abfragen und auf Knopfdruck laden.
//!
//! Bis 05.11.2026 gab es dafuer keinen Weg in der App: das Modell wird
//! bewusst nicht mitgeliefert (NVIDIA Open Model License, 139 MB, jedes Update
//! waere sonst so viel groesser), aber es liess sich auch nirgends laden. Jede
//! Besprechung zeigte deshalb nur "Modell zur Sprechertrennung ist nicht
//! installiert". Abgelegt wird es dort, wo `diarize::resolve_model_path` sucht.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter};

use crate::managers::meetings::diarize;

#[derive(Debug, Clone, Serialize, Type)]
pub struct DiarizationModelStatus {
    /// Katalogkennung (fuer die Fortschrittsereignisse `model-download-*`).
    pub model_id: String,
    pub model_ready: bool,
    pub downloading: bool,
    pub size_mb: u32,
}

static DOWNLOADING: AtomicBool = AtomicBool::new(false);
/// Abbruch des laufenden Downloads (Knopf „Abbrechen“).
static CANCEL: std::sync::Mutex<Option<hf_hub::api::tokio::CancellationToken>> =
    std::sync::Mutex::new(None);

fn models_root(app: &AppHandle) -> PathBuf {
    crate::portable::app_data_dir(app)
        .map(|d| d.join("models"))
        .unwrap_or_default()
}

fn catalog_entry() -> Option<crate::catalog::TtsCatalogEntry> {
    crate::catalog::tts_entries(crate::catalog::Purpose::Diarization)
        .into_iter()
        .find(|e| e.id == diarize::DEFAULT_MODEL_ID)
}

fn model_file(app: &AppHandle) -> Result<PathBuf, String> {
    diarize::catalog_model_path(&models_root(app), diarize::DEFAULT_MODEL_ID)
        .ok_or_else(|| "model_missing".to_string())
}

#[tauri::command]
#[specta::specta]
pub fn meeting_diarization_model_status(app: AppHandle) -> Result<DiarizationModelStatus, String> {
    let entry = catalog_entry().ok_or_else(|| "model_missing".to_string())?;
    let size: u64 = entry.files.iter().map(|f| f.size_bytes).sum();
    Ok(DiarizationModelStatus {
        model_id: entry.id.clone(),
        model_ready: model_file(&app)?.is_file(),
        downloading: DOWNLOADING.load(Ordering::Relaxed),
        size_mb: size.div_ceil(1_000_000).min(u64::from(u32::MAX)) as u32,
    })
}

/// Laedt das Modell, nur auf Knopfdruck (oder `--provision`). Fortschritt
/// ueber `model-download-progress`, Ende ueber `model-download-complete`.
/// Wirkt ab der naechsten Sprechertrennung (Stopp, Import, Neu-Transkription).
#[tauri::command]
#[specta::specta]
pub async fn meeting_diarization_model_download(app: AppHandle) -> Result<(), String> {
    use crate::managers::model::download::{HttpDownloadEvent, HttpDownloadOutcome};
    use crate::managers::model::ModelManager;
    let entry = catalog_entry().ok_or_else(|| "model_missing".to_string())?;
    let file = entry
        .files
        .first()
        .ok_or_else(|| "model_missing".to_string())?;
    let dest = model_file(&app)?;
    if dest.is_file() {
        return Ok(());
    }
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Err("download_running".into());
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            *CANCEL.lock().unwrap_or_else(|e| e.into_inner()) = None;
            DOWNLOADING.store(false, Ordering::SeqCst);
        }
    }
    let _reset = Reset;
    // Ein paralleler Aufruf kann inzwischen fertig sein.
    if dest.is_file() {
        return Ok(());
    }
    let dir = dest.parent().ok_or_else(|| "model_missing".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let partial = PathBuf::from(format!("{}.partial", dest.display()));
    let handle = app.clone();
    let emit = move |event: HttpDownloadEvent<'_>| {
        if let HttpDownloadEvent::Progress(p) = event {
            let _ = handle.emit("model-download-progress", p);
        }
    };
    let cancel = hf_hub::api::tokio::CancellationToken::new();
    *CANCEL.lock().unwrap_or_else(|e| e.into_inner()) = Some(cancel.clone());
    let outcome = ModelManager::download_http_resumable_with_events(
        &entry.id,
        &file.url,
        &partial,
        Some(file.size_bytes),
        file.sha256.as_deref(),
        &cancel,
        &emit,
    )
    .await
    .map_err(|e| e.to_string())?;
    if matches!(outcome, HttpDownloadOutcome::Completed) {
        std::fs::rename(&partial, &dest).map_err(|e| e.to_string())?;
        let _ = app.emit("model-download-complete", entry.id.as_str());
        return Ok(());
    }
    Err("cancelled".into())
}

/// Bricht einen laufenden Download ab; die Teildatei bleibt fuer die
/// Fortsetzung liegen. Ohne laufenden Download ohne Wirkung.
#[tauri::command]
#[specta::specta]
pub fn meeting_diarization_model_cancel() -> Result<(), String> {
    if let Some(token) = CANCEL.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        token.cancel();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Die Kennung, die der Knopf laedt, ist genau die, die die
    /// Sprechertrennung sucht, und sie steht mit Datei und Pruefsumme im Katalog.
    #[test]
    fn the_downloadable_entry_is_the_one_diarization_looks_for() {
        let entry = catalog_entry().expect("Sortformer steht im Katalog");
        let file = entry.files.first().expect("eine Datei");
        assert!(file.sha256.is_some(), "Download braucht eine Pruefsumme");
        assert!(file.size_bytes > 0);
        let root = std::path::Path::new("models");
        assert_eq!(
            diarize::catalog_model_path(root, &entry.id),
            Some(diarize::resolve_model_path(root, None))
        );
    }
}
