//! Skript-Werkstatt: Bücher, Gedächtnis, Vorlagen und KI-gestützte
//! Skripterzeugung.
//!
//! Ein **Buch** bündelt Seiten des Vorlesens als Teile einer Geschichte und
//! trägt ein **Gedächtnis** aus vier Markdown-Dateien im Anwendungsordner
//! (`books/<id>/gedaechtnis/`): `welt.md` (Schauplätze, Regeln),
//! `figuren.md` (wer ist wer, wie spricht er), `verlauf.md` (was in Teil
//! 1…n geschah) und `stil.md`. Markdown, damit man es lesen, von Hand
//! ändern und mit dem Buch exportieren kann.
//!
//! **Vorlagen** sind Prompt-Texte mit Platzhaltern (`{{prompt}}`,
//! `{{figuren}}`, `{{verlauf}}` …). Vier sind eingebaut; jede lässt sich
//! überschreiben und zurücksetzen, eigene kommen dazu — abgelegt unter
//! `books/_vorlagen/<id>.md`.
//!
//! Die **Erzeugung** ruft das eingestellte Sprachmodell mit Gedächtnis und
//! Vorlage und verlangt JSON: Titel, Skript und einen Vorschlag, wie das
//! Gedächtnis fortzuschreiben ist. Gespeichert wird davon nichts, bis der
//! Nutzer es im Dialog übernimmt.
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

use crate::commands::pages;
use crate::settings::AppSettings;

pub const BOOK_EXTENSION: &str = "lvbook";
pub const MEMORY_KINDS: [&str; 4] = ["welt", "figuren", "verlauf", "stil"];

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct BookCharacter {
    pub name: String,
    /// voice_id der Stimme, mit der die Figur spricht (`None` = Standard).
    pub voice_id: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub created_ms: f64,
    /// Seiten in Reihenfolge der Teile.
    pub page_ids: Vec<String>,
    pub characters: Vec<BookCharacter>,
    /// Sprache der Geschichte (BCP-47-Kurzform), Vorgabe "de".
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_language() -> String {
    "de".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct MemoryFile {
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ScriptTemplate {
    pub id: String,
    pub name: String,
    pub body: String,
    pub builtin: bool,
    /// Eingebaute Vorlage, deren Text der Nutzer geändert hat.
    pub modified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct GenerateOptions {
    pub book_id: Option<String>,
    pub template_id: String,
    pub prompt: String,
    pub part_title: String,
    pub length_words: u32,
    pub audience: String,
    pub tone: String,
    pub language: String,
    pub with_tags: bool,
    pub allowed_tags: Vec<String>,
    /// Figuren, die vorkommen sollen (Namen aus dem Buch).
    pub character_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct MemoryProposal {
    pub verlauf: String,
    pub figuren: String,
    pub welt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct GeneratedScript {
    pub title: String,
    pub script: String,
    pub memory: MemoryProposal,
}

// ---------------------------------------------------------------- Ablage --

fn books_root(app: &AppHandle) -> Result<PathBuf, String> {
    let base = crate::portable::app_data_dir(app).map_err(|e| e.to_string())?;
    let root = base.join("books");
    std::fs::create_dir_all(&root).map_err(|e| format!("could not create books dir: {e}"))?;
    Ok(root)
}

fn checked_book_id(id: &str) -> Result<&str, String> {
    let ok = id
        .strip_prefix("book_")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric()));
    if ok {
        Ok(id)
    } else {
        Err(format!("Ungültige Buch-Kennung: {id}"))
    }
}

fn book_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(books_root(app)?.join(checked_book_id(id)?))
}

fn read_book(dir: &Path) -> Option<Book> {
    let raw = std::fs::read_to_string(dir.join("buch.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_book(app: &AppHandle, book: &Book) -> Result<(), String> {
    let dir = book_dir(app, &book.id)?;
    std::fs::create_dir_all(dir.join("gedaechtnis"))
        .map_err(|e| format!("could not create book dir: {e}"))?;
    let raw = serde_json::to_string_pretty(book).map_err(|e| e.to_string())?;
    pages::write_atomic(&dir.join("buch.json"), raw.as_bytes())
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or_default()
}

fn memory_path(app: &AppHandle, book_id: &str, kind: &str) -> Result<PathBuf, String> {
    if !MEMORY_KINDS.contains(&kind) {
        return Err(format!("Unbekannter Gedächtnisteil: {kind}"));
    }
    Ok(book_dir(app, book_id)?.join("gedaechtnis").join(format!("{kind}.md")))
}

fn read_memory(app: &AppHandle, book_id: &str) -> Result<Vec<MemoryFile>, String> {
    MEMORY_KINDS
        .iter()
        .map(|kind| {
            Ok(MemoryFile {
                kind: kind.to_string(),
                text: std::fs::read_to_string(memory_path(app, book_id, kind)?).unwrap_or_default(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------- Bücher --

#[tauri::command]
#[specta::specta]
pub fn books_list(app: AppHandle) -> Result<Vec<Book>, String> {
    let root = books_root(&app)?;
    let mut books: Vec<Book> = std::fs::read_dir(&root)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| read_book(&e.path()))
        .collect();
    books.sort_by(|a, b| a.created_ms.partial_cmp(&b.created_ms).unwrap_or(std::cmp::Ordering::Equal));
    Ok(books)
}

#[tauri::command]
#[specta::specta]
pub fn books_create(app: AppHandle, title: String) -> Result<Book, String> {
    let title = title.trim();
    let book = Book {
        id: format!("book_{}", now_ms() as u64),
        title: if title.is_empty() { "Neues Buch".to_string() } else { title.to_string() },
        created_ms: now_ms(),
        page_ids: Vec::new(),
        characters: Vec::new(),
        language: default_language(),
    };
    write_book(&app, &book)?;
    Ok(book)
}

/// Titel, Figuren, Sprache, Seitenreihenfolge speichern (Id bleibt).
#[tauri::command]
#[specta::specta]
pub fn books_update(app: AppHandle, book: Book) -> Result<(), String> {
    let dir = book_dir(&app, &book.id)?;
    let existing = read_book(&dir).ok_or("Unbekanntes Buch")?;
    let updated = Book {
        created_ms: existing.created_ms,
        ..book
    };
    write_book(&app, &updated)
}

#[tauri::command]
#[specta::specta]
pub fn books_delete(app: AppHandle, id: String) -> Result<(), String> {
    let dir = book_dir(&app, &id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("could not delete book: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn books_add_page(app: AppHandle, book_id: String, page_id: String) -> Result<Book, String> {
    let dir = book_dir(&app, &book_id)?;
    let mut book = read_book(&dir).ok_or("Unbekanntes Buch")?;
    if !book.page_ids.contains(&page_id) {
        book.page_ids.push(page_id);
    }
    write_book(&app, &book)?;
    Ok(book)
}

#[tauri::command]
#[specta::specta]
pub fn books_memory_read(app: AppHandle, book_id: String) -> Result<Vec<MemoryFile>, String> {
    read_memory(&app, &book_id)
}

#[tauri::command]
#[specta::specta]
pub fn books_memory_write(
    app: AppHandle,
    book_id: String,
    kind: String,
    text: String,
) -> Result<(), String> {
    let path = memory_path(&app, &book_id, &kind)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    pages::write_atomic(&path, text.as_bytes())
}

// -------------------------------------------------------------- Vorlagen --

const BUILTIN_TEMPLATES: [(&str, &str, &str); 4] = [
    (
        "geschichte",
        "Geschichte (Vorlesen)",
        "Schreibe eine in sich geschlossene Geschichte zum Vorlesen.\n\
         Thema und Wünsche: {{prompt}}\n\
         Länge: etwa {{laenge}} Wörter. Zielgruppe: {{zielgruppe}}. Ton: {{ton}}. Sprache: {{sprache}}.\n\n\
         {{figuren}}\n{{welt}}\n{{stil}}\n{{verlauf}}\n\n\
         Regeln: kurze, gut sprechbare Sätze; keine Kapitelüberschriften im Text; \
         Figuren bleiben in Charakter und Sprechweise konsistent mit dem Gedächtnis.\n{{tags}}",
    ),
    (
        "hoerspiel",
        "Hörspiel (Dialog mit Sprechern)",
        "Schreibe ein Hörspiel mit verteilten Rollen zum Vorlesen mit mehreren Stimmen.\n\
         Thema und Wünsche: {{prompt}}\n\
         Länge: etwa {{laenge}} Wörter. Zielgruppe: {{zielgruppe}}. Ton: {{ton}}. Sprache: {{sprache}}.\n\n\
         {{figuren}}\n{{welt}}\n{{stil}}\n{{verlauf}}\n\n\
         Regeln: JEDE Zeile beginnt mit einem Sprechermarker in Spitzklammern, z. B. <Erzähler> oder <Mara>; \
         nur Figuren aus der Liste; ein Erzähler darf Szenen verbinden; keine Regieanweisungen in Klammern außer Tags.\n{{tags}}",
    ),
    (
        "fortsetzung",
        "Fortsetzung (nächster Teil)",
        "Schreibe den NÄCHSTEN Teil einer laufenden Geschichte. Er schließt nahtlos an den Verlauf an, \
         wiederholt nichts, führt offene Fäden weiter und endet mit einem kleinen Haken für den Folgeteil.\n\
         Wünsche für diesen Teil: {{prompt}}\n\
         Länge: etwa {{laenge}} Wörter. Zielgruppe: {{zielgruppe}}. Ton: {{ton}}. Sprache: {{sprache}}.\n\n\
         {{figuren}}\n{{welt}}\n{{stil}}\n{{verlauf}}\n\n\
         Regeln: Figuren, Orte und Regeln der Welt bleiben exakt wie im Gedächtnis; neue Figuren nur, wenn der Auftrag sie verlangt.\n{{tags}}",
    ),
    (
        "sachtext",
        "Sachtext / Erklärung",
        "Schreibe einen klaren Sachtext zum Vorlesen.\n\
         Thema: {{prompt}}\n\
         Länge: etwa {{laenge}} Wörter. Zielgruppe: {{zielgruppe}}. Ton: {{ton}}. Sprache: {{sprache}}.\n\n\
         {{stil}}\n\n\
         Regeln: kurze Sätze, Fachbegriffe erklären, keine Aufzählungszeichen (sie werden vorgelesen), \
         keine Überschriften.\n{{tags}}",
    ),
];

fn templates_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = books_root(app)?.join("_vorlagen");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn checked_template_id(id: &str) -> Result<&str, String> {
    let ok = !id.is_empty()
        && id.len() <= 60
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(id)
    } else {
        Err(format!("Ungültige Vorlagen-Kennung: {id}"))
    }
}

#[tauri::command]
#[specta::specta]
pub fn books_templates(app: AppHandle) -> Result<Vec<ScriptTemplate>, String> {
    let dir = templates_dir(&app)?;
    let mut out: Vec<ScriptTemplate> = BUILTIN_TEMPLATES
        .iter()
        .map(|(id, name, body)| {
            let custom = std::fs::read_to_string(dir.join(format!("{id}.md"))).ok();
            ScriptTemplate {
                id: id.to_string(),
                name: name.to_string(),
                modified: custom.is_some(),
                body: custom.unwrap_or_else(|| body.to_string()),
                builtin: true,
            }
        })
        .collect();
    let mut custom: Vec<ScriptTemplate> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".md")?.to_string();
            if BUILTIN_TEMPLATES.iter().any(|(b, _, _)| *b == id) {
                return None;
            }
            let body = std::fs::read_to_string(e.path()).ok()?;
            // Erste Zeile "# Name" ist der Anzeigename.
            let (name, body) = match body.strip_prefix("# ") {
                Some(rest) => {
                    let (n, b) = rest.split_once('\n').unwrap_or((rest, ""));
                    (n.trim().to_string(), b.trim_start().to_string())
                }
                None => (id.clone(), body),
            };
            Some(ScriptTemplate { id, name, body, builtin: false, modified: false })
        })
        .collect();
    custom.sort_by(|a, b| a.name.cmp(&b.name));
    out.append(&mut custom);
    Ok(out)
}

#[tauri::command]
#[specta::specta]
pub fn books_template_save(app: AppHandle, template: ScriptTemplate) -> Result<(), String> {
    let dir = templates_dir(&app)?;
    let id = checked_template_id(&template.id)?;
    let builtin = BUILTIN_TEMPLATES.iter().any(|(b, _, _)| *b == id);
    let content = if builtin {
        template.body.clone()
    } else {
        format!("# {}\n\n{}", template.name.trim(), template.body)
    };
    pages::write_atomic(&dir.join(format!("{id}.md")), content.as_bytes())
}

/// Eingebaute Vorlage auf den Standard zurücksetzen, eigene löschen.
#[tauri::command]
#[specta::specta]
pub fn books_template_reset(app: AppHandle, id: String) -> Result<(), String> {
    let dir = templates_dir(&app)?;
    let path = dir.join(format!("{}.md", checked_template_id(&id)?));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ------------------------------------------------------------- Erzeugung --

fn section(title: &str, body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        String::new()
    } else {
        format!("## {title}\n{body}\n")
    }
}

/// Der fertige Prompt aus Vorlage, Optionen und Gedächtnis.
pub fn build_prompt(
    template: &str,
    options: &GenerateOptions,
    book: Option<&Book>,
    memory: &[MemoryFile],
) -> String {
    let mem = |kind: &str| {
        memory
            .iter()
            .find(|m| m.kind == kind)
            .map(|m| m.text.clone())
            .unwrap_or_default()
    };
    let mut figuren = String::new();
    if let Some(book) = book {
        let chosen: Vec<&BookCharacter> = book
            .characters
            .iter()
            .filter(|c| options.character_names.is_empty() || options.character_names.contains(&c.name))
            .collect();
        if !chosen.is_empty() {
            figuren.push_str("Figuren, die vorkommen (Sprechermarker exakt so schreiben, z. B. <Name>):\n");
            for c in chosen {
                figuren.push_str(&format!("- <{}>: {}\n", c.name, c.description.trim()));
            }
        }
    }
    let figuren_md = mem("figuren");
    if !figuren_md.trim().is_empty() {
        figuren.push_str(&section("Figuren (Gedächtnis)", &figuren_md));
    }
    let tags = if options.with_tags && !options.allowed_tags.is_empty() {
        format!(
            "Setze sparsam Vortrags-Tags in eckigen Klammern direkt vor die Stelle, wo sie wirken sollen \
             (höchstens eines pro Satz, nur wo es den Vortrag verbessert). Erlaubte Tags: {}.",
            options.allowed_tags.join(", ")
        )
    } else {
        "Keine Tags in eckigen Klammern verwenden.".to_string()
    };
    template
        .replace("{{prompt}}", options.prompt.trim())
        .replace("{{titel}}", options.part_title.trim())
        .replace("{{laenge}}", &options.length_words.to_string())
        .replace("{{zielgruppe}}", options.audience.trim())
        .replace("{{ton}}", options.tone.trim())
        .replace("{{sprache}}", options.language.trim())
        .replace("{{figuren}}", figuren.trim_end())
        .replace("{{welt}}", section("Welt (Gedächtnis)", &mem("welt")).trim_end())
        .replace("{{stil}}", section("Stil (Gedächtnis)", &mem("stil")).trim_end())
        .replace("{{verlauf}}", section("Bisheriger Verlauf (Gedächtnis)", &mem("verlauf")).trim_end())
        .replace("{{tags}}", &tags)
}

fn system_prompt(language: &str) -> String {
    format!(
        "Du bist Autor für vorgelesene Geschichten und Hörspiele. Du schreibst in der Sprache '{language}'. \
         Antworte NUR mit einem JSON-Objekt mit den Feldern title (kurzer Titel dieses Teils), script (der \
         vollständige Text zum Vorlesen, Sprechermarker als <Name> am Zeilenanfang, keine Markdown-Auszeichnung) \
         und memory mit den Feldern verlauf (2–5 Sätze: was in diesem Teil geschah, für das Gedächtnis), \
         figuren (nur NEUE oder geänderte Fakten zu Figuren, sonst leer) und welt (nur NEUE Fakten zur Welt, sonst leer)."
    )
}

fn script_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "script", "memory"],
        "properties": {
            "title": { "type": "string" },
            "script": { "type": "string" },
            "memory": {
                "type": "object",
                "additionalProperties": false,
                "required": ["verlauf", "figuren", "welt"],
                "properties": {
                    "verlauf": { "type": "string" },
                    "figuren": { "type": "string" },
                    "welt": { "type": "string" }
                }
            }
        }
    })
}

fn strip_code_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let body = match rest.find('\n') {
        Some(nl) => &rest[nl + 1..],
        None => rest,
    };
    body.trim_end().trim_end_matches("```").trim()
}

pub async fn generate(
    settings: &AppSettings,
    template_body: &str,
    options: &GenerateOptions,
    book: Option<&Book>,
    memory: &[MemoryFile],
) -> Result<GeneratedScript, String> {
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or_else(|| "Kein LLM-Provider konfiguriert (Einstellungen → Nachbearbeitung)".to_string())?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return Err(format!(
            "Für '{}' ist kein Modell eingetragen (Einstellungen → Nachbearbeitung → Modell).",
            provider.label
        ));
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    let prompt = build_prompt(template_body, options, book, memory);
    let language = if options.language.trim().is_empty() { "de" } else { options.language.trim() };
    let response = crate::llm_client::send_chat_completion_with_schema(
        crate::managers::usage::Purpose::Summary,
        &provider,
        api_key,
        &model,
        prompt,
        Some(system_prompt(language)),
        Some(script_schema()),
        None,
        None,
    )
    .await
    .map_err(|e| format!("Skript-Erzeugung fehlgeschlagen: {e}"))?
    .ok_or_else(|| "Antwort ohne Inhalt".to_string())?;
    let parsed: GeneratedScript = serde_json::from_str(strip_code_fence(&response))
        .map_err(|e| format!("Antwort war kein gültiges JSON: {e}"))?;
    if parsed.script.trim().is_empty() {
        return Err("Das Modell hat kein Skript geliefert".into());
    }
    Ok(parsed)
}

#[tauri::command]
#[specta::specta]
pub async fn books_generate(app: AppHandle, options: GenerateOptions) -> Result<GeneratedScript, String> {
    use tauri::Emitter;
    let settings = crate::settings::get_settings(&app);
    let templates = books_templates(app.clone())?;
    let template = templates
        .into_iter()
        .find(|t| t.id == options.template_id)
        .ok_or_else(|| format!("Unbekannte Vorlage: {}", options.template_id))?;
    let (book, memory) = match &options.book_id {
        Some(id) => (read_book(&book_dir(&app, id)?), read_memory(&app, id)?),
        None => (None, Vec::new()),
    };
    let _ = app.emit("llm-activity", serde_json::json!({ "busy": true }));
    let result = generate(&settings, &template.body, &options, book.as_ref(), &memory).await;
    let _ = app.emit(
        "llm-activity",
        serde_json::json!({ "busy": false, "error": result.as_ref().err().cloned() }),
    );
    result
}

// ------------------------------------------------------- Export / Import --

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BookManifest {
    version: u32,
    title: String,
    exported_at_ms: f64,
    pages: u32,
    voices: Vec<crate::commands::pages_package::PackageVoice>,
    rights_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct BookPreview {
    pub title: String,
    pub pages: u32,
    pub voices: Vec<crate::commands::pages_package::PackageVoice>,
    pub rights_confirmed: bool,
}

/// Stimmen aller Seiten des Buchs (Vereinigung der Seiten-Vorschauen).
#[tauri::command]
#[specta::specta]
pub fn books_export_preview(app: AppHandle, id: String) -> Result<BookPreview, String> {
    let book = read_book(&book_dir(&app, &id)?).ok_or("Unbekanntes Buch")?;
    let mut voices: Vec<crate::commands::pages_package::PackageVoice> = Vec::new();
    for page_id in &book.page_ids {
        if let Ok(preview) = crate::commands::pages_package::pages_export_preview(app.clone(), page_id.clone()) {
            for v in preview.voices {
                if !voices.iter().any(|x| x.id == v.id) {
                    voices.push(v);
                }
            }
        }
    }
    Ok(BookPreview {
        title: book.title,
        pages: book.page_ids.len() as u32,
        voices,
        rights_confirmed: false,
    })
}

#[tauri::command]
#[specta::specta]
pub fn books_export(
    app: AppHandle,
    id: String,
    out_path: String,
    voice_ids: Vec<String>,
    rights_confirmed: bool,
) -> Result<String, String> {
    if !voice_ids.is_empty() && !rights_confirmed {
        return Err("Für die Weitergabe von Stimmen ist die Bestätigung der Rechte erforderlich".into());
    }
    let dir = book_dir(&app, &id)?;
    let book = read_book(&dir).ok_or("Unbekanntes Buch")?;
    let preview = books_export_preview(app.clone(), id.clone())?;
    let mut out = PathBuf::from(&out_path);
    if out.extension().and_then(|e| e.to_str()) != Some(BOOK_EXTENSION) {
        out.set_extension(BOOK_EXTENSION);
    }
    let tmp = out.with_extension(format!("{BOOK_EXTENSION}.tmp-{}", std::process::id()));
    let file = std::fs::File::create(&tmp).map_err(|e| format!("could not create {}: {e}", tmp.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let deflated: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let stored: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let put = |zip: &mut zip::ZipWriter<std::fs::File>, name: &str, bytes: &[u8], opts| -> Result<(), String> {
        zip.start_file(name, opts).map_err(|e| format!("could not add {name}: {e}"))?;
        zip.write_all(bytes).map_err(|e| format!("could not write {name}: {e}"))
    };
    // Seiten als einzelne Pakete; Stimmen NUR im ersten Paket, das sie
    // braucht (jede Stimme einmal), damit das Buch nicht n-fach schwer wird.
    let mut packed_voices: Vec<String> = Vec::new();
    for (n, page_id) in book.page_ids.iter().enumerate() {
        let page_preview = crate::commands::pages_package::pages_export_preview(app.clone(), page_id.clone())?;
        let wanted: Vec<String> = page_preview
            .voices
            .iter()
            .filter(|v| voice_ids.contains(&v.id) && v.present && !packed_voices.contains(&v.id))
            .map(|v| v.id.clone())
            .collect();
        let tmp_page = std::env::temp_dir().join(format!("lv-book-{}-{n}.lvpage", std::process::id()));
        let written = crate::commands::pages_package::pages_export(
            app.clone(),
            page_id.clone(),
            tmp_page.to_string_lossy().into_owned(),
            wanted.clone(),
            rights_confirmed,
        )?;
        let bytes = std::fs::read(&written).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&written);
        put(&mut zip, &format!("pages/{:03}.lvpage", n + 1), &bytes, stored)?;
        packed_voices.extend(wanted);
    }
    let mut book_json = book.clone();
    book_json.page_ids = Vec::new(); // Ids entstehen beim Import neu
    put(&mut zip, "buch.json", &serde_json::to_vec_pretty(&book_json).map_err(|e| e.to_string())?, deflated)?;
    for m in read_memory(&app, &id)? {
        put(&mut zip, &format!("gedaechtnis/{}.md", m.kind), m.text.as_bytes(), deflated)?;
    }
    let manifest = BookManifest {
        version: 1,
        title: book.title.clone(),
        exported_at_ms: now_ms(),
        pages: book.page_ids.len() as u32,
        voices: preview.voices.into_iter().filter(|v| packed_voices.contains(&v.id)).collect(),
        rights_confirmed,
    };
    put(&mut zip, "manifest.json", &serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?, deflated)?;
    zip.finish().map_err(|e| format!("could not finish book: {e}"))?;
    std::fs::rename(&tmp, &out).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not write {}: {e}", out.display())
    })?;
    Ok(out.to_string_lossy().into_owned())
}

fn open_book(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let zip = zip::ZipArchive::new(file).map_err(|e| format!("kein gültiges Buch-Paket: {e}"))?;
    for i in 0..zip.len() {
        let name = zip.name_for_index(i).unwrap_or_default();
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') || name.contains(':') {
            return Err(format!("Paket enthält einen unzulässigen Pfad: {name}"));
        }
    }
    Ok(zip)
}

fn read_zip_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<Vec<u8>, String> {
    let mut entry = zip.by_name(name).map_err(|e| format!("{name} fehlt im Paket: {e}"))?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).map_err(|e| format!("could not read {name}: {e}"))?;
    Ok(bytes)
}

#[tauri::command]
#[specta::specta]
pub fn books_inspect(app: AppHandle, path: String) -> Result<BookPreview, String> {
    let mut zip = open_book(Path::new(&path))?;
    let manifest: BookManifest = serde_json::from_slice(&read_zip_entry(&mut zip, "manifest.json")?)
        .map_err(|e| format!("manifest.json unlesbar: {e}"))?;
    let fish_dir = {
        use tauri::Manager;
        app.state::<std::sync::Arc<crate::managers::tts::TtsManager>>().fish_dir_public()
    };
    Ok(BookPreview {
        title: manifest.title,
        pages: manifest.pages,
        voices: manifest
            .voices
            .into_iter()
            .map(|v| crate::commands::pages_package::PackageVoice {
                present: crate::managers::tts::voices::voice_is_complete(&fish_dir, &v.id),
                ..v
            })
            .collect(),
        rights_confirmed: manifest.rights_confirmed,
    })
}

#[tauri::command]
#[specta::specta]
pub fn books_import(app: AppHandle, path: String, import_voices: bool) -> Result<Book, String> {
    let mut zip = open_book(Path::new(&path))?;
    let mut book: Book = serde_json::from_slice(&read_zip_entry(&mut zip, "buch.json")?)
        .map_err(|e| format!("buch.json unlesbar: {e}"))?;
    book.id = format!("book_{}", now_ms() as u64);
    book.created_ms = now_ms();
    book.page_ids = Vec::new();
    write_book(&app, &book)?;
    for kind in MEMORY_KINDS {
        if let Ok(bytes) = read_zip_entry(&mut zip, &format!("gedaechtnis/{kind}.md")) {
            let path = memory_path(&app, &book.id, kind)?;
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        }
    }
    let mut names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.name_for_index(i).map(str::to_owned))
        .filter(|n| n.starts_with("pages/") && n.ends_with(".lvpage"))
        .collect();
    names.sort();
    for (n, name) in names.iter().enumerate() {
        let bytes = read_zip_entry(&mut zip, name)?;
        let tmp = std::env::temp_dir().join(format!("lv-book-import-{}-{n}.lvpage", std::process::id()));
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        let page = crate::commands::pages_package::pages_import(
            app.clone(),
            tmp.to_string_lossy().into_owned(),
            import_voices,
        );
        let _ = std::fs::remove_file(&tmp);
        book.page_ids.push(page?.id);
    }
    write_book(&app, &book)?;
    Ok(book)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> GenerateOptions {
        GenerateOptions {
            book_id: None,
            template_id: "geschichte".into(),
            prompt: "Ein Drache lernt schwimmen".into(),
            part_title: "Teil 2".into(),
            length_words: 400,
            audience: "Kinder ab 6".into(),
            tone: "warm".into(),
            language: "de".into(),
            with_tags: true,
            allowed_tags: vec!["whisper".into(), "excited".into()],
            character_names: vec!["Mara".into()],
        }
    }

    #[test]
    fn der_prompt_traegt_optionen_figuren_und_gedaechtnis() {
        let book = Book {
            characters: vec![
                BookCharacter { name: "Mara".into(), voice_id: Some("mara".into()), description: "neugierig".into() },
                BookCharacter { name: "Sofie".into(), voice_id: None, description: "still".into() },
            ],
            ..Default::default()
        };
        let memory = vec![
            MemoryFile { kind: "verlauf".into(), text: "Teil 1: Der Drache fiel ins Wasser.".into() },
            MemoryFile { kind: "welt".into(), text: String::new() },
        ];
        let body = BUILTIN_TEMPLATES[2].2;
        let p = build_prompt(body, &opts(), Some(&book), &memory);
        assert!(p.contains("Ein Drache lernt schwimmen"));
        assert!(p.contains("etwa 400 Wörter"));
        assert!(p.contains("- <Mara>: neugierig"));
        assert!(!p.contains("<Sofie>"), "nicht gewaehlte Figur bleibt draussen");
        assert!(p.contains("Bisheriger Verlauf (Gedächtnis)"));
        assert!(!p.contains("Welt (Gedächtnis)"), "leerer Gedaechtnisteil erzeugt keinen Abschnitt");
        assert!(p.contains("Erlaubte Tags: whisper, excited"));
        // Ohne Tags: ausdrueckliches Verbot.
        let mut o = opts();
        o.with_tags = false;
        assert!(build_prompt(body, &o, Some(&book), &memory).contains("Keine Tags"));
    }

    #[test]
    fn das_schema_verlangt_titel_skript_und_gedaechtnis() {
        let s = script_schema();
        assert_eq!(s["required"], serde_json::json!(["title", "script", "memory"]));
        assert_eq!(s["properties"]["memory"]["required"], serde_json::json!(["verlauf", "figuren", "welt"]));
    }
}
