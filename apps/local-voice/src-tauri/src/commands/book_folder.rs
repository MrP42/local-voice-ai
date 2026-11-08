//! Tauri-Hüllen für Notizen, Anhänge und Transkripte im Buch-Ordner (lva#365 P3b). Die
//! Logik steht in `crate::buch_dateien`; hier werden der Buch-Ordner geprüft, die
//! Bibliotheks-Sperre gehalten und der Abgleich angestoßen.

use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, State};

use crate::buch_dateien::{self as kern, Rolle};
use crate::commands::pages;
use crate::library;
use crate::library_layout;
use crate::sync::SyncEngine;

/// Eine Notiz, ein Anhang oder ein Transkript im Buch-Ordner.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct BookFolderFile {
    /// Pfad relativ zum Buch-Ordner, mit `/` (etwa `notizen/Gedanken.md`).
    pub rel: String,
    /// `notiz`, `anhang` oder `transkript`.
    pub role: String,
    pub name: String,
    pub bytes: f64,
    pub modified_ms: f64,
    /// Fassung (Transkript), sonst leer.
    pub edition: String,
    /// Von einem anderen Gerät gemeldet, noch nicht geladen.
    pub pending: bool,
    /// Konfliktkopie aus dem Abgleich.
    pub conflict: bool,
    /// Bekannt, fehlt aber lokal (im Explorer gelöscht, Ordner fehlt): vom Hub
    /// wiederherstellen oder in der App löschen.
    pub missing: bool,
    /// Von einem anderen Gerät gemeldet, zurückgehalten: der Name fiele mit einer anderen
    /// Datei zusammen (Groß-/Kleinschreibung, Unicode-Form).
    pub held: bool,
}

fn buch_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    let data = pages::data_root(app)?;
    library_layout::buch_ordner(&data, id)
}

/// Notizen, Anhänge und Transkripte eines Buchs (auch noch nicht geladene).
#[tauri::command]
#[specta::specta]
pub fn library_book_folder_files(
    app: AppHandle,
    id: String,
) -> Result<Vec<BookFolderFile>, String> {
    let dir = buch_dir(&app, &id)?;
    let _guard = library::sync_lock();
    Ok(kern::auflisten(&dir, &id)?
        .into_iter()
        .map(|d| BookFolderFile {
            rel: d.rel,
            role: d.rolle.name().to_string(),
            name: d.name,
            bytes: d.groesse as f64,
            modified_ms: d.geaendert_ms as f64,
            edition: d.fassung,
            pending: d.erwartet,
            conflict: d.konflikt,
            missing: d.fehlt,
            held: d.zurueckgehalten,
        })
        .collect())
}

/// Neue Notiz anlegen; liefert ihren Pfad im Buch-Ordner.
#[tauri::command]
#[specta::specta]
pub fn library_book_note_create(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
    id: String,
    title: String,
) -> Result<String, String> {
    let dir = buch_dir(&app, &id)?;
    let rel = {
        let _guard = library::sync_lock();
        kern::notiz_anlegen(&dir, &title)?
    };
    engine.touch();
    Ok(rel)
}

/// Dateien als Anhänge in den Buch-Ordner kopieren; liefert die neuen Pfade.
#[tauri::command]
#[specta::specta]
pub async fn library_book_attachments_add(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
    id: String,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let dir = buch_dir(&app, &id)?;
    let quellen: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let rels = tauri::async_runtime::spawn_blocking(move || {
        let _guard = library::sync_lock();
        kern::anhaenge_hinzufuegen(&dir, &quellen)
    })
    .await
    .map_err(|e| e.to_string())??;
    engine.touch();
    Ok(rels)
}

/// Eine Datei mit dem Standardprogramm des Systems öffnen.
#[tauri::command]
#[specta::specta]
pub fn library_book_file_open(app: AppHandle, id: String, rel: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = buch_dir(&app, &id)?;
    let pfad = kern::pfad(&dir, &rel)?;
    app.opener()
        .open_path(pfad.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("Datei nicht zu öffnen: {e}"))
}

/// Eine Datei in den Papierkorb der Bibliothek legen (auf allen Geräten nach dem
/// nächsten Abgleich).
#[tauri::command]
#[specta::specta]
pub fn library_book_file_remove(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
    id: String,
    rel: String,
) -> Result<(), String> {
    let dir = buch_dir(&app, &id)?;
    if Rolle::aus_pfad(&rel).is_none() {
        return Err("Nur Notizen, Anhänge und Transkripte lassen sich hier entfernen.".into());
    }
    {
        let _guard = library::sync_lock();
        kern::entfernen(&dir, &id, &rel)?;
    }
    engine.touch();
    Ok(())
}

/// Eine lokal fehlende Datei vom Hub zurückholen (der nächste Abgleich lädt sie).
#[tauri::command]
#[specta::specta]
pub fn library_book_file_restore(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
    id: String,
    rel: String,
) -> Result<(), String> {
    let dir = buch_dir(&app, &id)?;
    {
        let _guard = library::sync_lock();
        kern::wiederherstellen(&dir, &rel)?;
    }
    engine.touch();
    Ok(())
}
