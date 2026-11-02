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
pub mod terms;

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
    /// "summary" (Zusammenfassung) oder "terms" (Begriffe).
    pub kind: String,
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
                kind: "summary".into(),
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

// ---------------------------------------------------------------- Begriffe --

/// `begriffe.json` wird vom Lauf und von Nutzerkorrekturen geschrieben.
static TERMS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn terms_lock() -> std::sync::MutexGuard<'static, ()> {
    TERMS_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ergebnis eines Begriffslaufs.
#[derive(Debug, Clone, Serialize, Type)]
pub struct TermsRun {
    /// Kapitel, die in diesem Lauf bewertet wurden.
    pub chapters_done: u32,
    /// Kapitel, die schon aktuell waren.
    pub chapters_skipped: u32,
    /// Kapitel ohne lesbare Modellantwort; sie kommen im nächsten Lauf wieder.
    pub chapters_failed: u32,
    pub terms: Vec<terms::BookTerm>,
}

/// Begriffe des Buchs erkennen (lva#184). Nur neue oder geänderte Kapitel;
/// jedes Kapitel wird sofort gespeichert, ein Abbruch verliert nichts.
#[tauri::command]
#[specta::specta]
pub async fn book_ai_terms_extract(app: AppHandle, page_id: String) -> Result<TermsRun, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("book_ai:busy".into());
    }
    let _guard = RunGuard;
    CANCEL.store(false, Ordering::SeqCst);

    let dir = page_path(&app, &page_id)?;
    let text = page_text(&dir)?;
    let sections = source::sections(&text);
    let mut file = terms::load(&dir)?;
    let todo: Vec<(usize, source::Section)> = terms::pending(&file, &sections)
        .into_iter()
        .map(|(i, s)| (i, s.clone()))
        .collect();
    let skipped = sections.len() - todo.len();
    let settings = crate::settings::get_settings(&app);
    let language = crate::summarizer::language_clause(&text);
    let limit = crate::summarizer::chunk_chars(&settings);

    let total = todo.len() as u32;
    let emit = |done: u32| {
        let _ = app.emit(
            "book-ai-progress",
            BookAiProgress {
                page_id: page_id.clone(),
                kind: "terms".into(),
                done,
                total,
            },
        );
    };
    emit(0);
    let mut done = 0u32;
    let mut failed = 0u32;
    for (i, section) in &todo {
        if CANCEL.load(Ordering::SeqCst) {
            return Err(engine::CANCELLED.into());
        }
        let label = section.label(*i);
        // Bekannte Begriffe: ausgeblendete zuerst, damit sie nicht wiederkommen.
        let mut known: Vec<&terms::BookTerm> = file.terms.iter().collect();
        known.sort_by_key(|t| !t.hidden);
        let known: Vec<String> = known.iter().map(|t| t.term.clone()).take(200).collect();
        let mut found = Vec::new();
        let mut readable = true;
        // Lange Kapitel blockweise; Begriffe zählen je Block.
        for chunk in
            crate::summarizer::chunk_text(&section.text, limit.saturating_sub(2_000).max(4_000))
        {
            let answer = crate::summarizer::ask_llm(
                &settings,
                terms::extract_prompt(&label, &chunk, &language, &known),
            )
            .await?;
            if CANCEL.load(Ordering::SeqCst) {
                return Err(engine::CANCELLED.into());
            }
            match terms::parse_answer(&answer, &label, &chunk) {
                Some(t) => found.extend(t),
                None => readable = false,
            }
        }
        done += 1;
        if !readable {
            // Nicht als bewertet markieren: der nächste Lauf versucht es erneut.
            failed += 1;
            emit(done);
            continue;
        }
        // Frisch laden und unter der Sperre schreiben: Korrekturen, die der
        // Nutzer während des Laufs gemacht hat, gehen nicht verloren.
        {
            let _lock = terms_lock();
            let mut current = terms::load(&dir)?;
            terms::drop_chapter(&mut current, &label);
            terms::merge(&mut current, found);
            current.chapters.insert(
                terms::chapter_key(*i, section),
                terms::chapter_hash(section),
            );
            terms::save(&dir, &current)?;
            file = current;
        }
        emit(done);
    }
    log::info!(
        "book_ai: Begriffe aus {} Kapiteln, {} aktuell, {} gesamt",
        done,
        skipped,
        file.terms.len()
    );
    Ok(TermsRun {
        chapters_failed: failed,
        chapters_done: done - failed,
        chapters_skipped: skipped as u32,
        terms: file.terms,
    })
}

#[tauri::command]
#[specta::specta]
pub fn book_ai_terms(app: AppHandle, page_id: String) -> Result<Vec<terms::BookTerm>, String> {
    Ok(terms::load(&page_path(&app, &page_id)?)?.terms)
}

/// Begriff anlegen, ändern oder ausblenden (`hidden`). Der Eintrag gilt
/// danach als Nutzerkorrektur und wird von Läufen nicht mehr überschrieben.
#[tauri::command]
#[specta::specta]
pub fn book_ai_term_save(
    app: AppHandle,
    page_id: String,
    term: terms::BookTerm,
) -> Result<terms::BookTerm, String> {
    let dir = page_path(&app, &page_id)?;
    let _lock = terms_lock();
    let mut file = terms::load(&dir)?;
    let out = terms::apply_user_edit(&mut file, term)?;
    terms::save(&dir, &file)?;
    Ok(out)
}

/// Fundstellen für die Hervorhebung im aktuellen Seitentext.
#[tauri::command]
#[specta::specta]
pub async fn book_ai_term_hits(
    app: AppHandle,
    page_id: String,
) -> Result<Vec<terms::TermHit>, String> {
    let dir = page_path(&app, &page_id)?;
    // Ganze Bücher mit vielen Begriffen: nicht auf dem Hauptthread suchen.
    tauri::async_runtime::spawn_blocking(move || {
        let file = terms::load(&dir)?;
        if file.terms.is_empty() {
            return Ok(Vec::new());
        }
        Ok(terms::hits(&page_text(&dir)?, &file.terms))
    })
    .await
    .map_err(|_| "book_ai:terms_failed".to_string())?
}

/// Text als KI-Dokument übernehmen, z. B. die bisherige Zusammenfassung einer
/// Seite (aus der Zeit vor den Vorlagen, lva#183).
#[tauri::command]
#[specta::specta]
pub fn book_ai_document_import(
    app: AppHandle,
    page_id: String,
    title: String,
    text: String,
) -> Result<KiDocument, String> {
    let dir = page_path(&app, &page_id)?;
    if text.trim().is_empty() {
        return Err("book_ai:empty_source".into());
    }
    if let Some(existing) = store::find_import(&dir, text.trim())? {
        return Ok(existing);
    }
    let now = now_ms();
    store::create(
        &dir,
        KiDocument {
            id: String::new(),
            title: if title.trim().is_empty() {
                "Zusammenfassung".into()
            } else {
                title
            },
            template_id: "import".into(),
            template_title: "Zusammenfassung".into(),
            source: SummarySource::Book,
            source_label: "Buch".into(),
            created_ms: now,
            modified_ms: now,
            words: 0,
            model: String::new(),
        },
        text.trim(),
    )
}

/// Ein KI-Dokument exportieren. Das Format ergibt sich aus der Endung (`md`, `txt`,
/// `docx`, `pdf`); Word und PDF tragen den eingestellten Briefkopf. Der Titel
/// steht als Überschrift über dem Text.
#[tauri::command]
#[specta::specta]
pub async fn book_ai_document_export(
    app: AppHandle,
    page_id: String,
    id: String,
    path: String,
) -> Result<(), String> {
    let dir = page_path(&app, &page_id)?;
    let doc = store::list(&dir)?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or("book_ai:document_missing")?;
    let text = store::read_text(&dir, &id)?;
    // Beginnt der Text schon mit einer Hauptüberschrift, keine zweite davor.
    let body = text.trim();
    let markdown = if body.starts_with("# ") {
        format!("{body}\n")
    } else {
        format!("# {}\n\n{body}\n", doc.title.trim())
    };
    tauri::async_runtime::spawn_blocking(move || {
        crate::commands::meeting_documents::export_with_letterhead(&app, path, markdown, None)
    })
    .await
    .map_err(|_| "book_ai:export_failed".to_string())?
}
