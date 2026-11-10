//! Dauerhafte Sprachgespräche (Paket U3).
//!
//! `store` hält die SQLite-Datei `conversations.db` im Format des
//! Unterhaltungs-Vertrags der Suite; dieses Modul stellt sie als Tauri-Befehle
//! bereit. Die Gesprächsführung selbst bleibt in `voice_call` und im Frontend.
//!
//! Alle Befehle laufen auf einem Blockier-Thread (SQLite). Fehlermeldungen
//! enthalten nie Titel oder Beitragstexte, und es wird nichts davon geloggt.

pub mod store;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use store::{
    Conversation, ConversationDetail, ConversationEntry, ConversationStore, ConversationSummary,
    NewEntry,
};

/// Führt `f` auf einem Blockier-Thread gegen den Speicher aus.
async fn run<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&ConversationStore) -> anyhow::Result<T> + Send + 'static,
{
    let store = app
        .try_state::<Arc<ConversationStore>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "Der Gesprächsverlauf ist nicht verfügbar.".to_string())?;
    tauri::async_runtime::spawn_blocking(move || f(&store))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Zusammenfassungen der Gespräche: Angeheftete zuerst, dann nach letzter
/// Änderung absteigend. Archivierte nur mit `include_archived`.
#[tauri::command]
#[specta::specta]
pub async fn conversations_list(
    app: AppHandle,
    include_archived: bool,
) -> Result<Vec<ConversationSummary>, String> {
    run(&app, move |s| s.list(include_archived)).await
}

/// Eine Unterhaltung mit allen Beiträgen, älteste zuerst.
#[tauri::command]
#[specta::specta]
pub async fn conversation_get(app: AppHandle, id: String) -> Result<ConversationDetail, String> {
    run(&app, move |s| s.get(&id)).await
}

/// Legt eine leere Unterhaltung an (`kind` = `voice`, Datenklasse INTERNAL). Mit `project_id` gehört
/// sie zu einem Suite-Projekt (`scope.project_id`, lva#365 P4c-2): Das Projekt muss es geben und darf
/// nicht archiviert sein. Die Bindung wird nur hier gesetzt; kein Befehl ändert sie danach.
#[tauri::command]
#[specta::specta]
pub async fn conversation_create(
    app: AppHandle,
    title: Option<String>,
    project_id: Option<String>,
) -> Result<Conversation, String> {
    let Some(projekt) = project_id else {
        return run(&app, move |s| s.create(title)).await;
    };
    crate::projekte::zustand::projekt_fuer_gespraech(&app, &projekt).await?;
    run(&app, move |s| s.create_in_project(title, &projekt)).await
}

/// Ändert Titel, Anheften und Archiv; nicht gesetzte Felder bleiben.
#[tauri::command]
#[specta::specta]
pub async fn conversation_update(
    app: AppHandle,
    id: String,
    title: Option<String>,
    pinned: Option<bool>,
    archived: Option<bool>,
) -> Result<Conversation, String> {
    run(&app, move |s| s.update(&id, title, pinned, archived)).await
}

/// Heftet ein Gespräch an oder löst es.
#[tauri::command]
#[specta::specta]
pub async fn conversation_set_pinned(
    app: AppHandle,
    id: String,
    pinned: bool,
) -> Result<Conversation, String> {
    run(&app, move |s| s.set_pinned(&id, pinned)).await
}

/// Löscht die Unterhaltung samt aller Beiträge. Gehörte sie zu einem Projekt, vergisst das Versandtor
/// ihre Antworten (`gespraech_vergessen`, lva#365 P4c-2) und merkt sich die Kennung als gelöscht.
#[tauri::command]
#[specta::specta]
pub async fn conversation_delete(app: AppHandle, id: String) -> Result<(), String> {
    let id_lesen = id.clone();
    let war_projekt = run(&app, move |s| s.project_binding(&id_lesen))
        .await
        .map(|b| matches!(b, store::ProjectBinding::Project(_)))
        // Nicht lesbar: im Zweifel vergessen (schadet einem gewöhnlichen Gespräch nicht).
        .unwrap_or(true);
    let id_loeschen = id.clone();
    run(&app, move |s| s.delete(&id_loeschen)).await?;
    if war_projekt {
        crate::voice_call::projekt::gespraech_vergessen(&app, &id).await;
    }
    Ok(())
}

/// Hängt einen Beitrag an und setzt `updated_at` der Unterhaltung.
#[tauri::command]
#[specta::specta]
pub async fn conversation_append_entry(
    app: AppHandle,
    conversation_id: String,
    entry: NewEntry,
) -> Result<ConversationEntry, String> {
    run(&app, move |s| s.append_entry(&conversation_id, entry)).await
}

/// Ändert Text und Status eines Beitrags (unterbrochene, schon gespeicherte Antwort).
#[tauri::command]
#[specta::specta]
pub async fn conversation_update_entry(
    app: AppHandle,
    id: String,
    text: String,
    status: String,
) -> Result<ConversationEntry, String> {
    run(&app, move |s| s.update_entry(&id, text, status)).await
}
