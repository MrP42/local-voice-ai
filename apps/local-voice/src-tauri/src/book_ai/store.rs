//! Ablage der KI-Dokumente eines Buchs (lva#182).
//!
//! Ein Buch im Reader ist eine Seite (`projects/<page>/`). Die KI-Dokumente
//! liegen im Unterordner `ki/`:
//! - `dokumente/<ulid>.md`: das Ergebnis als Markdown, von Hand änderbar;
//! - `dokumente/index.json`: die Metadaten aller Dokumente;
//! - `cache/`: Zwischenfassungen (siehe `engine`).
//!
//! Wird die Seite gelöscht, verschwinden die Dokumente mit ihr.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use super::source::SummarySource;
use crate::commands::pages::write_atomic;

/// Index-Änderungen (Lesen, Ändern, Schreiben) laufen nacheinander: Ein
/// Lauf, der gerade ein Dokument ablegt, und ein Umbenennen in der
/// Oberfläche dürfen sich nicht gegenseitig überschreiben.
static INDEX_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    INDEX_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub const MAX_TITLE_CHARS: usize = 120;
/// Größter Dokumenttext in Bytes (2 MB).
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

/// Titel auf die zulässige Länge kürzen (für erzeugte Titel).
pub fn fit_title(title: &str) -> String {
    let t = title.trim();
    if t.chars().count() <= MAX_TITLE_CHARS {
        return t.to_string();
    }
    let cut: String = t.chars().take(MAX_TITLE_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct KiDocument {
    pub id: String,
    pub title: String,
    pub template_id: String,
    pub template_title: String,
    pub source: SummarySource,
    /// Anzeige der Quelle, z. B. „Ganzes Buch“ oder „Kapitel 3“.
    pub source_label: String,
    pub created_ms: f64,
    pub modified_ms: f64,
    pub words: u32,
    /// Anbieter und Modell, mit dem das Dokument entstand.
    pub model: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    documents: Vec<KiDocument>,
}

pub fn ki_dir(page_dir: &Path) -> PathBuf {
    page_dir.join("ki")
}

pub fn cache_dir(page_dir: &Path) -> PathBuf {
    ki_dir(page_dir).join("cache")
}

fn docs_dir(page_dir: &Path) -> PathBuf {
    ki_dir(page_dir).join("dokumente")
}

/// Dokument-IDs entstehen nur hier (ULID). Alles andere wird abgewiesen,
/// bevor ein Pfad daraus wird.
fn checked_id(id: &str) -> Result<&str, String> {
    let ok = id.len() == 26 && id.chars().all(|c| c.is_ascii_alphanumeric());
    if ok {
        Ok(id)
    } else {
        Err("book_ai:invalid_document_id".into())
    }
}

fn load_index(page_dir: &Path) -> Result<Index, String> {
    let path = docs_dir(page_dir).join("index.json");
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|_| "book_ai:index_broken".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Index::default()),
        Err(_) => Err("book_ai:index_unreadable".to_string()),
    }
}

fn save_index(page_dir: &Path, index: &Index) -> Result<(), String> {
    let dir = docs_dir(page_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let raw = serde_json::to_string_pretty(index).map_err(|e| e.to_string())?;
    write_atomic(&dir.join("index.json"), raw.as_bytes())
}

/// Alle Dokumente, neueste zuerst.
pub fn list(page_dir: &Path) -> Result<Vec<KiDocument>, String> {
    let mut docs = load_index(page_dir)?.documents;
    docs.sort_by(|a, b| b.created_ms.total_cmp(&a.created_ms));
    Ok(docs)
}

pub fn read_text(page_dir: &Path, id: &str) -> Result<String, String> {
    let path = docs_dir(page_dir).join(format!("{}.md", checked_id(id)?));
    std::fs::read_to_string(path).map_err(|_| "book_ai:document_missing".to_string())
}

/// Neues Dokument anlegen: erst den Text, dann den Index. Ein Abbruch
/// dazwischen hinterlässt höchstens eine verwaiste Datei, nie einen
/// Indexeintrag ohne Text.
pub fn create(page_dir: &Path, mut doc: KiDocument, text: &str) -> Result<KiDocument, String> {
    if text.len() > MAX_TEXT_BYTES {
        return Err("book_ai:text_too_large".into());
    }
    doc.id = ulid::Ulid::new().to_string();
    doc.title = fit_title(&doc.title);
    doc.words = super::source::word_count(text);
    let dir = docs_dir(page_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    write_atomic(&dir.join(format!("{}.md", doc.id)), text.as_bytes())?;
    let _guard = lock();
    let mut index = load_index(page_dir)?;
    index.documents.push(doc.clone());
    save_index(page_dir, &index)?;
    Ok(doc)
}

/// Übernommenes Dokument mit genau diesem Text, falls es schon existiert
/// (eine abgebrochene Übernahme darf kein zweites anlegen).
pub fn find_import(page_dir: &Path, text: &str) -> Result<Option<KiDocument>, String> {
    for doc in list(page_dir)? {
        if doc.template_id == "import" && read_text(page_dir, &doc.id).ok().as_deref() == Some(text)
        {
            return Ok(Some(doc));
        }
    }
    Ok(None)
}

/// Titel und/oder Text ändern.
pub fn update(
    page_dir: &Path,
    id: &str,
    title: Option<String>,
    text: Option<String>,
    now_ms: f64,
) -> Result<KiDocument, String> {
    let id = checked_id(id)?;
    if text.as_ref().is_some_and(|t| t.len() > MAX_TEXT_BYTES) {
        return Err("book_ai:text_too_large".into());
    }
    let title = match title {
        Some(t) => {
            let t = t.trim();
            if t.is_empty() || t.chars().count() > MAX_TITLE_CHARS {
                return Err("book_ai:invalid_title".into());
            }
            Some(t.to_string())
        }
        None => None,
    };
    let _guard = lock();
    let mut index = load_index(page_dir)?;
    // Erst prüfen, dann schreiben: ein unbekanntes Dokument hinterlässt keine
    // verwaiste Datei (z. B. nach dem Löschen auf einem anderen Gerät).
    let doc = index
        .documents
        .iter_mut()
        .find(|d| d.id == id)
        .ok_or("book_ai:document_missing")?;
    if let Some(title) = title {
        doc.title = title;
    }
    if let Some(text) = text {
        write_atomic(
            &docs_dir(page_dir).join(format!("{id}.md")),
            text.as_bytes(),
        )?;
        doc.words = super::source::word_count(&text);
    }
    doc.modified_ms = now_ms;
    let out = doc.clone();
    save_index(page_dir, &index)?;
    Ok(out)
}

pub fn delete(page_dir: &Path, id: &str) -> Result<(), String> {
    let id = checked_id(id)?;
    let _guard = lock();
    let mut index = load_index(page_dir)?;
    let before = index.documents.len();
    index.documents.retain(|d| d.id != id);
    if index.documents.len() == before {
        return Err("book_ai:document_missing".into());
    }
    save_index(page_dir, &index)?;
    let _ = std::fs::remove_file(docs_dir(page_dir).join(format!("{id}.md")));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> KiDocument {
        KiDocument {
            id: String::new(),
            title: "Eine Seite – Ganzes Buch".into(),
            template_id: "builtin:eine_seite".into(),
            template_title: "Eine Seite".into(),
            source: SummarySource::Book,
            source_label: "Ganzes Buch".into(),
            created_ms: 1.0,
            modified_ms: 1.0,
            words: 0,
            model: "ollama/x".into(),
        }
    }

    #[test]
    fn create_list_update_delete() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        assert!(list(p).unwrap().is_empty());
        let a = create(p, doc(), "eins zwei drei").unwrap();
        assert_eq!(a.id.len(), 26);
        assert_eq!(a.words, 3);
        let mut later = doc();
        later.created_ms = 2.0;
        let b = create(p, later, "x").unwrap();
        let ids: Vec<_> = list(p).unwrap().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![b.id.clone(), a.id.clone()], "neueste zuerst");
        assert_eq!(read_text(p, &a.id).unwrap(), "eins zwei drei");

        let u = update(p, &a.id, Some("Neu".into()), Some("a b".into()), 5.0).unwrap();
        assert_eq!((u.title.as_str(), u.words, u.modified_ms), ("Neu", 2, 5.0));
        assert_eq!(read_text(p, &a.id).unwrap(), "a b");
        assert_eq!(
            update(p, &a.id, Some("  ".into()), None, 6.0).unwrap_err(),
            "book_ai:invalid_title"
        );

        delete(p, &a.id).unwrap();
        assert_eq!(list(p).unwrap().len(), 1);
        assert_eq!(read_text(p, &a.id).unwrap_err(), "book_ai:document_missing");
        assert_eq!(delete(p, &a.id).unwrap_err(), "book_ai:document_missing");
    }

    #[test]
    fn update_of_unknown_document_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        create(p, doc(), "x").unwrap();
        let ghost = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        assert_eq!(
            update(p, ghost, None, Some("y".into()), 1.0).unwrap_err(),
            "book_ai:document_missing"
        );
        assert!(!docs_dir(p).join(format!("{ghost}.md")).exists());
        let other = tempfile::tempdir().unwrap();
        assert!(update(other.path(), ghost, None, Some("y".into()), 1.0).is_err());
        assert!(
            !docs_dir(other.path()).exists(),
            "kein Ordner in fremder Seite"
        );
    }

    #[test]
    fn import_is_found_again() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        assert!(find_import(p, "alt").unwrap().is_none());
        let mut d = doc();
        d.template_id = "import".into();
        let a = create(p, d, "alt").unwrap();
        assert_eq!(find_import(p, "alt").unwrap().map(|d| d.id), Some(a.id));
        assert!(find_import(p, "anders").unwrap().is_none());
    }

    #[test]
    fn long_titles_are_shortened_and_huge_texts_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = doc();
        d.title = "T".repeat(300);
        let a = create(dir.path(), d, "x").unwrap();
        assert_eq!(a.title.chars().count(), MAX_TITLE_CHARS);
        let big = "x".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(
            create(dir.path(), doc(), &big).unwrap_err(),
            "book_ai:text_too_large"
        );
        assert_eq!(
            update(dir.path(), &a.id, None, Some(big), 1.0).unwrap_err(),
            "book_ai:text_too_large"
        );
    }

    #[test]
    fn foreign_ids_never_become_paths() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../../etc/passwd", "", "01ARZ3NDEKTSV4RRFFQ69G5FA/", "x"] {
            assert_eq!(
                read_text(dir.path(), bad).unwrap_err(),
                "book_ai:invalid_document_id"
            );
        }
    }
}
