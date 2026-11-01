//! KI-Funktionen im Buch-Reader (Epic lva#181). Stufe B1 (lva#182):
//! Zusammenfassungen nach Vorlage für das ganze Buch oder einen Bereich,
//! abgelegt als KI-Dokumente beim Buch.
//!
//! - `templates`: Vorlagenformat, 20 mitgelieferte Vorlagen, Import/Export.
//! - `source`: Auswahl, Säubern (Seitenmarken, Sprecher, Stilmarken), Kapitel.
//! - `engine`: Ablauf mit Zwischenfassungen, Cache, Fortschritt und Abbruch.
//! - `store`: KI-Dokumente unter `projects/<page>/ki/`.
//!
//! Datenschutz: kein Buch-, Prompt- oder Antworttext im Log oder in
//! Fehlermeldungen; nur Codes und Zähler.

pub mod engine;
pub mod source;
pub mod store;
pub mod templates;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter};

use crate::commands::pages::{page_path, write_atomic};
use source::SummarySource;
use store::KiDocument;
use templates::BookTemplate;

/// Abbruch des laufenden Laufs (`book_ai_cancel`).
static CANCEL: AtomicBool = AtomicBool::new(false);
/// Es läuft höchstens ein Lauf zur Zeit (das Modell rechnet ohnehin seriell).
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Fortschritt eines Laufs (Ereignis `book-ai-progress`).
#[derive(Debug, Clone, Serialize, Type)]
pub struct BookAiProgress {
    pub page_id: String,
    pub done: u32,
    pub total: u32,
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or_default()
}

// ---------------------------------------------------------------- Vorlagen --

fn user_templates_path(app: &AppHandle) -> Result<PathBuf, String> {
    let base = crate::portable::app_data_dir(app).map_err(|e| e.to_string())?;
    let dir = base.join("book_ai");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create book_ai dir: {e}"))?;
    Ok(dir.join("vorlagen.json"))
}

fn load_user_templates(app: &AppHandle) -> Result<Vec<BookTemplate>, String> {
    match std::fs::read_to_string(user_templates_path(app)?) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|_| "book_ai:templates_broken".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err("book_ai:templates_unreadable".to_string()),
    }
}

fn save_user_templates(app: &AppHandle, list: &[BookTemplate]) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    write_atomic(&user_templates_path(app)?, raw.as_bytes())
}

fn all_templates(app: &AppHandle) -> Result<Vec<BookTemplate>, String> {
    let mut all = templates::builtins();
    all.extend(load_user_templates(app)?);
    Ok(all)
}

/// Mitgelieferte und eigene Vorlagen.
#[tauri::command]
#[specta::specta]
pub fn book_ai_templates(app: AppHandle) -> Result<Vec<BookTemplate>, String> {
    all_templates(&app)
}

/// Eigene Vorlage anlegen oder ändern. Eine mitgelieferte Vorlage (oder eine
/// ohne ID) wird als neue eigene Vorlage gespeichert, also kopiert.
#[tauri::command]
#[specta::specta]
pub fn book_ai_template_save(
    app: AppHandle,
    template: BookTemplate,
) -> Result<BookTemplate, String> {
    let mut t = template;
    t.builtin = false;
    if t.id.is_empty() || templates::is_builtin_id(&t.id) {
        t.id = ulid::Ulid::new().to_string();
    }
    templates::validate(&t)?;
    let mut list = load_user_templates(&app)?;
    match list.iter_mut().find(|x| x.id == t.id) {
        Some(slot) => *slot = t.clone(),
        None => list.push(t.clone()),
    }
    save_user_templates(&app, &list)?;
    Ok(t)
}

#[tauri::command]
#[specta::specta]
pub fn book_ai_template_delete(app: AppHandle, id: String) -> Result<(), String> {
    if templates::is_builtin_id(&id) {
        return Err("book_ai:builtin_readonly".into());
    }
    let mut list = load_user_templates(&app)?;
    list.retain(|t| t.id != id);
    save_user_templates(&app, &list)
}

#[tauri::command]
#[specta::specta]
pub fn book_ai_template_export(app: AppHandle, id: String, path: String) -> Result<(), String> {
    let t = all_templates(&app)?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or("book_ai:template_missing")?;
    write_atomic(&PathBuf::from(path), templates::to_file(&t)?.as_bytes())
}

#[tauri::command]
#[specta::specta]
pub fn book_ai_template_import(app: AppHandle, path: String) -> Result<BookTemplate, String> {
    use std::io::Read;
    // Begrenzt lesen: Bei Gerätedateien oder FIFOs sagt die Dateigröße nichts.
    let file = std::fs::File::open(&path).map_err(|_| "template_invalid:file_unreadable")?;
    let mut raw = String::new();
    file.take(templates::MAX_FILE_BYTES as u64 + 1)
        .read_to_string(&mut raw)
        .map_err(|_| "template_invalid:file_unreadable")?;
    let t = templates::from_file(&raw, ulid::Ulid::new().to_string())?;
    let mut list = load_user_templates(&app)?;
    list.push(t.clone());
    save_user_templates(&app, &list)?;
    Ok(t)
}

// --------------------------------------------------------------- Erzeugen --

/// Originaltext der Seite aus ihrem Arbeitsstand (`state.json`, Feld `text`).
fn page_text(dir: &std::path::Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(dir.join("state.json"))
        .map_err(|_| "book_ai:page_without_text".to_string())?;
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str().map(str::to_owned)))
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| "book_ai:page_without_text".to_string())
}

/// Setzt `RUNNING` beim Verlassen zurück, auch bei Fehlern.
struct RunGuard;
impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Zusammenfassung nach Vorlage erzeugen und als KI-Dokument ablegen.
#[tauri::command]
#[specta::specta]
pub async fn book_ai_summarize(
    app: AppHandle,
    page_id: String,
    source: SummarySource,
    source_label: String,
    template_id: String,
) -> Result<KiDocument, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("book_ai:busy".into());
    }
    let _guard = RunGuard;
    CANCEL.store(false, Ordering::SeqCst);

    let dir = page_path(&app, &page_id)?;
    let template = all_templates(&app)?
        .into_iter()
        .find(|t| t.id == template_id)
        .ok_or("book_ai:template_missing")?;
    let selected = source::select(&page_text(&dir)?, &source)?;
    let sections = source::sections(&selected);

    let settings = crate::settings::get_settings(&app);
    let provider = settings
        .active_post_process_provider()
        .map(|p| p.id.clone())
        .unwrap_or_default();
    let model = settings
        .active_post_process_provider()
        .and_then(|p| settings.post_process_models.get(&p.id).cloned())
        .unwrap_or_default();
    let model_key = format!("{provider}/{model}");
    let language = crate::summarizer::language_clause(&selected);

    let emitter = app.clone();
    let pid = page_id.clone();
    let progress = move |done: usize, total: usize| {
        let _ = emitter.emit(
            "book-ai-progress",
            BookAiProgress {
                page_id: pid.clone(),
                done: done as u32,
                total: total as u32,
            },
        );
    };
    let text = engine::run(
        engine::RunInput {
            sections: &sections,
            template: &template,
            limit: crate::summarizer::chunk_chars(&settings),
            model_key: &model_key,
            language: &language,
        },
        &engine::DigestCache {
            dir: Some(store::cache_dir(&dir)),
        },
        |prompt| {
            let settings = settings.clone();
            async move { crate::summarizer::ask_llm(&settings, prompt).await }
        },
        &progress,
        &CANCEL,
    )
    .await?;

    // Die Seite wurde während des Laufs gelöscht: keinen Geisterordner anlegen.
    if !dir.join("state.json").exists() {
        return Err("book_ai:page_missing".into());
    }
    let now = now_ms();
    let label: String = source_label.trim().chars().take(80).collect();
    let label = if label.is_empty() {
        "Buch".to_string()
    } else {
        label
    };
    store::create(
        &dir,
        KiDocument {
            id: String::new(),
            title: format!("{} – {label}", template.title),
            template_id: template.id.clone(),
            template_title: template.title.clone(),
            source,
            source_label: label.clone(),
            created_ms: now,
            modified_ms: now,
            words: 0,
            model: model_key,
        },
        text.trim(),
    )
}

/// Laufende Zusammenfassung abbrechen; sie endet nach dem aktuellen Schritt.
#[tauri::command]
#[specta::specta]
pub fn book_ai_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

// -------------------------------------------------------------- Dokumente --

#[tauri::command]
#[specta::specta]
pub fn book_ai_documents(app: AppHandle, page_id: String) -> Result<Vec<KiDocument>, String> {
    store::list(&page_path(&app, &page_id)?)
}

#[tauri::command]
#[specta::specta]
pub fn book_ai_document_text(
    app: AppHandle,
    page_id: String,
    id: String,
) -> Result<String, String> {
    store::read_text(&page_path(&app, &page_id)?, &id)
}

#[tauri::command]
#[specta::specta]
pub fn book_ai_document_update(
    app: AppHandle,
    page_id: String,
    id: String,
    title: Option<String>,
    text: Option<String>,
) -> Result<KiDocument, String> {
    store::update(&page_path(&app, &page_id)?, &id, title, text, now_ms())
}

#[tauri::command]
#[specta::specta]
pub fn book_ai_document_delete(app: AppHandle, page_id: String, id: String) -> Result<(), String> {
    store::delete(&page_path(&app, &page_id)?, &id)
}
