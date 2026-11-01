//! Bibliothek (lva#152): Bücher und Hörbücher als **ein** Eintrag je Werk.
//!
//! Ein Buch hat eine Textfassung (eine Vorlesen-Seite, vertonbar), eine
//! Audiofassung (ein importiertes Hörbuch, transkribierbar) oder beide. Kommt
//! eine zweite Fassung desselben Werks dazu, wird sie an das vorhandene Buch
//! gehängt ([`same_work`]); es entsteht nie ein zweiter Eintrag.
//!
//! Ablage unter `<appdata>/library/`:
//! - `<id>/buch.json` – Metadaten und Fassungen,
//! - `<id>/cover.jpg|png` – Cover aus Open Library, der EPUB oder einer Datei,
//! - `<id>/audio/` – das importierte Hörbuch,
//! - `fortschritt.json` – Fortschritt aller Bücher, Seiten und abgeleiteten
//!   Dokumente (Schlüssel → JSON-Text, Schema gehört der Oberfläche).
//!
//! Netz nur auf Knopfdruck: Metadaten und Cover kommen aus Open Library, dabei
//! gehen allein ISBN oder Titel und Autor hinaus, nie Buchtext. Ohne Netz
//! zeichnet die Oberfläche ein Cover aus Titel und Autor.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tauri::AppHandle;

use crate::commands::pages::write_atomic;

static LIBRARY_LOCK: Mutex<()> = Mutex::new(());
fn lock() -> MutexGuard<'static, ()> {
    LIBRARY_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

const OPEN_LIBRARY_SEARCH: &str = "https://openlibrary.org/search.json";
const OPEN_LIBRARY_COVERS: &str = "https://covers.openlibrary.org/b";
const OFFLINE: &str =
    "Open Library ist gerade nicht erreichbar. Prüfe die Internetverbindung und versuche es noch einmal.";
/// Größer ist kein Cover, sondern ein Irrläufer.
const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROGRESS_KEY: usize = 200;
const MAX_PROGRESS_VALUE: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct TextVersion {
    /// Vorlesen-Seite mit dem Text. Sie bleibt die Quelle, damit Leseposition,
    /// Lesezeichen und Seitenmarken erhalten bleiben.
    pub page_id: String,
    /// `document` (EPUB, PDF, DOCX, TXT, MD) oder `transcript` (aus dem Hörbuch).
    pub origin: String,
    /// Dateiname der Quelle, nur zur Anzeige.
    pub source_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct AudioVersion {
    /// Dateiname unter `<id>/audio/`.
    pub file: String,
    pub duration_ms: Option<f64>,
    pub narrator: Option<String>,
    /// Laufende Transkription (Import-Warteschlange), bis der Text übernommen ist.
    pub transcript_job: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct LibraryBook {
    /// Leer beim Anlegen; das Backend vergibt `book_<ms>`.
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub isbn: Option<String>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub pages: Option<u32>,
    pub description: Option<String>,
    /// Woher die Metadaten stammen: `openlibrary`, `epub`, `manual`.
    pub metadata_source: Option<String>,
    /// Woher das Cover stammt: `openlibrary`, `epub`, `file` oder `generated`.
    pub cover_source: String,
    pub text: Option<TextVersion>,
    pub audio: Option<AudioVersion>,
    pub added_ms: f64,
    pub finished: bool,
    /// Nur zur Anzeige, wird beim Auflisten gesetzt: absoluter Pfad des Covers
    /// samt Änderungszeit (`?v=`), damit ein neues Cover sofort erscheint.
    pub cover_path: Option<String>,
    /// Nur zur Anzeige: absoluter Pfad des importierten Hörbuchs.
    pub audio_path: Option<String>,
}

fn generated() -> String {
    "generated".to_string()
}

/// Ein Treffer aus Open Library, als Vorschlag für den Dialog.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct MetadataCandidate {
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub first_publish_year: Option<u32>,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
    pub language: Option<String>,
    pub pages: Option<u32>,
    /// Open-Library-Cover-Kennung (`cover_i`).
    pub cover_id: Option<i64>,
    /// Werk-Schlüssel (`/works/OL…W`) für den Klappentext.
    pub work_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ProgressEntry {
    pub key: String,
    /// `None` löscht den Eintrag.
    pub value: Option<String>,
}

// ---------------------------------------------------------------------------
// Pfade
// ---------------------------------------------------------------------------

fn library_root(app: &AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let base = crate::portable::data_dir()
        .cloned()
        .or_else(|| app.path().app_local_data_dir().ok())
        .ok_or("Kein Datenverzeichnis verfügbar")?;
    let root = base.join("library");
    std::fs::create_dir_all(&root).map_err(|e| format!("could not create library dir: {e}"))?;
    Ok(root)
}

/// Kennungen entstehen nur hier; alles andere ist ein manipulierter Aufruf.
fn checked_id(id: &str) -> Result<&str, String> {
    let valid = id
        .strip_prefix("book_")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric()));
    if valid {
        Ok(id)
    } else {
        Err(format!("Ungültige Buch-Kennung: {id}"))
    }
}

fn book_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    Ok(root.join(checked_id(id)?))
}

fn fresh_id(root: &Path) -> String {
    let mut ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    // Zwei Importe in derselben Millisekunde dürfen sich nicht überschreiben.
    while root.join(format!("book_{ms}")).exists() {
        ms += 1;
    }
    format!("book_{ms}")
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or_default()
}

fn cover_file(dir: &Path) -> Option<PathBuf> {
    ["cover.jpg", "cover.png", "cover.webp"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

fn read_book(dir: &Path) -> Option<LibraryBook> {
    let raw = std::fs::read_to_string(dir.join("buch.json")).ok()?;
    let mut book: LibraryBook = serde_json::from_str(&raw)
        .map_err(|e| log::warn!("Bibliothek: {} unlesbar: {e}", dir.display()))
        .ok()?;
    if let Some(cover) = cover_file(dir) {
        let stamp = std::fs::metadata(&cover)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis())
            .unwrap_or_default();
        book.cover_path = Some(format!("{}?v={stamp}", cover.to_string_lossy()));
    } else {
        book.cover_source = generated();
    }
    if let Some(audio) = &book.audio {
        let path = dir.join("audio").join(&audio.file);
        book.audio_path = path.is_file().then(|| path.to_string_lossy().into_owned());
    }
    Some(book)
}

fn write_book(dir: &Path, book: &LibraryBook) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create book dir: {e}"))?;
    let mut stored = book.clone();
    stored.cover_path = None;
    stored.audio_path = None;
    let json = serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?;
    write_atomic(&dir.join("buch.json"), &json)
}

fn list_books(root: &Path) -> Vec<LibraryBook> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut books: Vec<LibraryBook> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| checked_id(&e.file_name().to_string_lossy()).is_ok())
        .filter_map(|e| read_book(&e.path()))
        .collect();
    books.sort_by(|a, b| b.added_ms.total_cmp(&a.added_ms));
    books
}

// ---------------------------------------------------------------------------
// Dubletten
// ---------------------------------------------------------------------------

/// Nur Buchstaben und Ziffern, klein, Umlaute gefaltet: „Faust. Der
/// Tragödie erster Teil“ und „faust der tragoedie erster teil“ sind gleich.
fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            'à' | 'á' | 'â' => out.push('a'),
            'è' | 'é' | 'ê' => out.push('e'),
            'ì' | 'í' | 'î' => out.push('i'),
            'ò' | 'ó' | 'ô' => out.push('o'),
            'ù' | 'ú' | 'û' => out.push('u'),
            c if c.is_alphanumeric() => out.push(c),
            _ => {}
        }
    }
    out
}

/// Titel ohne Untertitel (vor „:“, „ – “ oder „. “) und ohne führenden Artikel.
fn core_title(title: &str) -> String {
    let head = title
        .split([':', '|', '(', '['])
        .next()
        .unwrap_or(title)
        .split(" – ")
        .next()
        .unwrap_or(title)
        .split(" - ")
        .next()
        .unwrap_or(title);
    let folded = fold(head);
    for article in ["der", "die", "das", "the", "a", "an", "le", "la", "les"] {
        if let Some(rest) = folded.strip_prefix(article) {
            // Nur echte Artikel: „Dieb“ beginnt nicht mit „die“ + Wort.
            let words: Vec<&str> = head.split_whitespace().collect();
            if words.len() > 1 && fold(words[0]) == article && !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    folded
}

/// Nachnamen der Autoren (letztes Wort, gefaltet).
fn surnames(authors: &[String]) -> Vec<String> {
    authors
        .iter()
        .flat_map(|a| a.split([',', '&', ';']))
        .flat_map(|a| a.split(" und ").map(str::to_string).collect::<Vec<_>>())
        .filter_map(|a| a.split_whitespace().last().map(fold))
        .filter(|s| s.len() > 1)
        .collect()
}

fn digits(isbn: &str) -> String {
    isbn.chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .collect::<String>()
        .to_uppercase()
}

/// Gleiches Werk? Bei passender ISBN sicher; sonst gleicher Kerntitel und,
/// wenn beide Autoren kennen, mindestens ein gemeinsamer Nachname.
pub fn same_work(
    a_title: &str,
    a_authors: &[String],
    a_isbn: Option<&str>,
    b: &LibraryBook,
) -> bool {
    if let (Some(x), Some(y)) = (a_isbn, b.isbn.as_deref()) {
        let (x, y) = (digits(x), digits(y));
        if !x.is_empty() && x == y {
            return true;
        }
    }
    let (ta, tb) = (core_title(a_title), core_title(&b.title));
    if ta.is_empty() || ta != tb {
        return false;
    }
    let (sa, sb) = (surnames(a_authors), surnames(&b.authors));
    sa.is_empty() || sb.is_empty() || sa.iter().any(|s| sb.contains(s))
}

// ---------------------------------------------------------------------------
// Open Library
// ---------------------------------------------------------------------------

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("LocalVoiceAI (Bibliothek; Metadaten und Cover)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// Treffer aus `search.json` lesen. Höchstens `limit`, ohne leere Titel.
pub fn parse_search(body: &str, limit: usize) -> Result<Vec<MetadataCandidate>, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("Open Library: unerwartete Antwort ({e})"))?;
    let docs = value["docs"].as_array().cloned().unwrap_or_default();
    let strings = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::trim).map(str::to_string))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(docs
        .iter()
        .filter_map(|d| {
            let title = d["title"].as_str()?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let isbns = strings(&d["isbn"]);
            Some(MetadataCandidate {
                title,
                subtitle: d["subtitle"].as_str().map(str::to_string),
                authors: strings(&d["author_name"]),
                first_publish_year: d["first_publish_year"].as_u64().map(|y| y as u32),
                publisher: strings(&d["publisher"]).into_iter().next(),
                // ISBN-13 bevorzugt, sie ist auf heutigen Ausgaben gedruckt.
                isbn: isbns
                    .iter()
                    .find(|i| i.len() == 13)
                    .or_else(|| isbns.first())
                    .cloned(),
                language: strings(&d["language"]).into_iter().next(),
                pages: d["number_of_pages_median"].as_u64().map(|p| p as u32),
                cover_id: d["cover_i"].as_i64().filter(|i| *i > 0),
                work_key: d["key"]
                    .as_str()
                    .filter(|k| k.starts_with("/works/"))
                    .map(str::to_string),
            })
        })
        .take(limit)
        .collect())
}

/// Klappentext aus einem Werk-Datensatz (`description` als Text oder Objekt).
pub fn parse_description(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let d = &value["description"];
    let text = d.as_str().or_else(|| d["value"].as_str())?.trim();
    // Open Library hängt gern Quellenblöcke an („----------“ und Links).
    let text = text.split("\n----------").next().unwrap_or(text).trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Bildformat aus den ersten Bytes. Nur echte Bilder werden Cover.
fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

fn store_cover(dir: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_COVER_BYTES {
        return Err("Das Bild ist zu groß für ein Cover (höchstens 8 MB).".into());
    }
    // Open Library liefert für fehlende Cover ein 1×1-GIF bzw. 43 Byte.
    if bytes.len() < 512 {
        return Err("Für diese Ausgabe gibt es kein Cover.".into());
    }
    let ext = image_ext(bytes).ok_or("Die Datei ist kein JPEG-, PNG- oder WebP-Bild.")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for old in ["cover.jpg", "cover.png", "cover.webp"] {
        let _ = std::fs::remove_file(dir.join(old));
    }
    write_atomic(&dir.join(format!("cover.{ext}")), bytes)
}

// ---------------------------------------------------------------------------
// EPUB-Cover
// ---------------------------------------------------------------------------

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let start = tag.find(&needle)? + needle.len();
    let quote = tag[start..].chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &tag[start + 1..];
    rest.find(quote).map(|end| &rest[..end])
}

/// Pfad des Coverbilds im Archiv aus dem OPF: EPUB 3 `properties="cover-image"`,
/// sonst EPUB 2 `<meta name="cover" content="id">`.
pub fn epub_cover_href(opf: &str) -> Option<String> {
    let items: Vec<&str> = opf
        .split('<')
        .filter(|t| t.starts_with("item ") || t.starts_with("opf:item "))
        .collect();
    if let Some(item) = items.iter().find(|t| {
        attr(t, "properties").is_some_and(|p| p.split_whitespace().any(|p| p == "cover-image"))
    }) {
        return attr(item, "href").map(str::to_string);
    }
    let cover_id = opf
        .split('<')
        .filter(|t| t.starts_with("meta ") || t.starts_with("opf:meta "))
        .find(|t| attr(t, "name") == Some("cover"))
        .and_then(|t| attr(t, "content"))?;
    items
        .iter()
        .find(|t| attr(t, "id") == Some(cover_id))
        .and_then(|t| attr(t, "href"))
        .map(str::to_string)
}

fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    max: usize,
) -> Option<Vec<u8>> {
    let entry = archive.by_name(name).ok()?;
    if entry.size() as usize > max {
        return None;
    }
    let mut buf = Vec::new();
    entry.take(max as u64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn epub_cover(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("kein EPUB (zip): {e}"))?;
    let container = read_zip_entry(&mut archive, "META-INF/container.xml", 64 * 1024)
        .ok_or("EPUB ohne container.xml")?;
    let container = String::from_utf8_lossy(&container);
    let opf_path = container
        .split('<')
        .find(|t| t.starts_with("rootfile "))
        .and_then(|t| attr(t, "full-path"))
        .ok_or("EPUB ohne OPF")?
        .to_string();
    let opf = read_zip_entry(&mut archive, &opf_path, 2 * 1024 * 1024).ok_or("OPF unlesbar")?;
    let href =
        epub_cover_href(&String::from_utf8_lossy(&opf)).ok_or("Die EPUB nennt kein Cover.")?;
    let base = opf_path.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
    let joined = if base.is_empty() {
        href.clone()
    } else {
        format!("{base}/{href}")
    };
    // `../` im href auflösen, ohne das Archiv zu verlassen.
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            p => parts.push(p),
        }
    }
    let name = parts.join("/");
    let name = percent_decode(&name);
    read_zip_entry(&mut archive, &name, MAX_COVER_BYTES)
        .ok_or_else(|| "Coverbild fehlt in der EPUB.".into())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Fortschritt
// ---------------------------------------------------------------------------

fn progress_path(root: &Path) -> PathBuf {
    root.join("fortschritt.json")
}

/// Gespeicherten Fortschritt lesen. Fehlt die Datei, ist er leer. Ist sie
/// unlesbar, wird sie als `fortschritt.json.defekt-<ms>` beiseitegelegt statt
/// beim nächsten Sichern überschrieben – so bleibt der alte Stand rettbar.
fn load_progress(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let path = progress_path(root);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("Fortschritt nicht lesbar: {e}")),
    };
    match serde_json::from_str(&raw) {
        Ok(map) => Ok(map),
        Err(e) => {
            let aside = root.join(format!("fortschritt.json.defekt-{}", now_ms() as i64));
            std::fs::rename(&path, &aside).map_err(|r| {
                format!("Fortschritt beschädigt ({e}), Sichern fehlgeschlagen: {r}")
            })?;
            log::warn!("fortschritt.json beschädigt ({e}), beiseitegelegt");
            Ok(BTreeMap::new())
        }
    }
}

/// Einträge einpflegen (`None` löscht). Ungültige Schlüssel oder übergroße
/// Werte werden verworfen statt die ganze Datei zu gefährden.
pub fn merge_progress(map: &mut BTreeMap<String, String>, entries: Vec<ProgressEntry>) {
    for entry in entries {
        if entry.key.is_empty() || entry.key.len() > MAX_PROGRESS_KEY {
            log::warn!("Fortschritts-Schlüssel ungültig, verworfen");
            continue;
        }
        match entry.value {
            Some(v) if v.len() <= MAX_PROGRESS_VALUE => {
                map.insert(entry.key, v);
            }
            Some(_) => log::warn!(
                "Fortschritt zu groß, verworfen ({} Zeichen Schlüssel)",
                entry.key.len()
            ),
            None => {
                map.remove(&entry.key);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Befehle
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub fn library_list(app: AppHandle) -> Result<Vec<LibraryBook>, String> {
    let root = library_root(&app)?;
    Ok(list_books(&root))
}

/// Anlegen (leere Kennung) oder ändern. Gibt das gespeicherte Buch zurück.
#[tauri::command]
#[specta::specta]
pub fn library_save(app: AppHandle, book: LibraryBook) -> Result<LibraryBook, String> {
    let _guard = lock();
    let root = library_root(&app)?;
    let mut book = book;
    book.title = book.title.trim().to_string();
    if book.title.is_empty() {
        return Err("Ein Buch braucht einen Titel.".into());
    }
    if book.id.is_empty() {
        book.id = fresh_id(&root);
        if book.added_ms <= 0.0 {
            book.added_ms = now_ms();
        }
    }
    let dir = book_dir(&root, &book.id)?;
    if let Some(audio) = &book.audio {
        // Nur ein Dateiname im eigenen audio-Ordner, kein Pfad.
        let plain = Path::new(&audio.file)
            .file_name()
            .is_some_and(|n| n == std::ffi::OsStr::new(&audio.file));
        if !plain || audio.file.contains(['/', '\\', ':']) || audio.file.starts_with('.') {
            return Err("Ungültiger Dateiname der Audiofassung.".into());
        }
    }
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch nach dem Speichern unlesbar.".into())
}

/// Buch aus der Bibliothek entfernen, samt Cover und importiertem Hörbuch.
/// Die Vorlesen-Seite der Textfassung bleibt (sie löscht die Oberfläche auf
/// Wunsch selbst), ebenso der Fortschritt bis zum nächsten Aufräumen.
#[tauri::command]
#[specta::specta]
pub fn library_delete(app: AppHandle, id: String) -> Result<(), String> {
    let _guard = lock();
    let root = library_root(&app)?;
    let dir = book_dir(&root, &id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("could not delete book: {e}"))?;
    }
    Ok(())
}

/// Gibt es das Werk schon? Dann soll die neue Fassung angehängt werden.
#[tauri::command]
#[specta::specta]
pub fn library_find_duplicate(
    app: AppHandle,
    title: String,
    authors: Vec<String>,
    isbn: Option<String>,
) -> Result<Option<LibraryBook>, String> {
    let root = library_root(&app)?;
    Ok(list_books(&root)
        .into_iter()
        .find(|b| same_work(&title, &authors, isbn.as_deref(), b)))
}

/// Ausgaben bei Open Library suchen: per ISBN oder per Titel und Autor.
#[tauri::command]
#[specta::specta]
pub async fn library_metadata_search(
    title: String,
    author: String,
    isbn: Option<String>,
) -> Result<Vec<MetadataCandidate>, String> {
    const FIELDS: &str = "key,title,subtitle,author_name,first_publish_year,publisher,isbn,cover_i,language,number_of_pages_median";
    let mut query: Vec<(&str, String)> = vec![("fields", FIELDS.into()), ("limit", "8".into())];
    match isbn.as_deref().and_then(crate::isbn::normalize_isbn) {
        Some(isbn) => query.push(("isbn", isbn)),
        None => {
            let title = title.trim();
            if title.is_empty() {
                return Ok(Vec::new());
            }
            query.push(("title", title.chars().take(200).collect()));
            let author = author.trim();
            if !author.is_empty() {
                query.push(("author", author.chars().take(120).collect()));
            }
        }
    }
    let response = http()?
        .get(OPEN_LIBRARY_SEARCH)
        .query(&query)
        .send()
        .await
        .map_err(|e| {
            log::warn!("Bibliothek: Open Library nicht erreichbar: {e}");
            OFFLINE.to_string()
        })?;
    if !response.status().is_success() {
        return Err(format!("Open Library antwortete mit {}", response.status()));
    }
    parse_search(&response.text().await.map_err(|e| e.to_string())?, 6)
}

/// Klappentext eines Werks (`/works/OL…W`).
#[tauri::command]
#[specta::specta]
pub async fn library_description(work_key: String) -> Result<Option<String>, String> {
    let valid = work_key
        .strip_prefix("/works/OL")
        .and_then(|r| r.strip_suffix('W'))
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    if !valid {
        return Err("Ungültiger Werk-Schlüssel.".into());
    }
    let response = http()?
        .get(format!("https://openlibrary.org{work_key}.json"))
        .send()
        .await
        .map_err(|_| OFFLINE.to_string())?;
    if !response.status().is_success() {
        return Ok(None);
    }
    Ok(parse_description(
        &response.text().await.map_err(|e| e.to_string())?,
    ))
}

/// Cover aus Open Library laden (per Cover-Kennung oder ISBN) und ablegen.
#[tauri::command]
#[specta::specta]
pub async fn library_cover_fetch(
    app: AppHandle,
    id: String,
    cover_id: Option<i64>,
    isbn: Option<String>,
) -> Result<LibraryBook, String> {
    let root = library_root(&app)?;
    let dir = book_dir(&root, &id)?;
    let url = match (
        cover_id.filter(|c| *c > 0),
        isbn.as_deref().and_then(crate::isbn::normalize_isbn),
    ) {
        (Some(c), _) => format!("{OPEN_LIBRARY_COVERS}/id/{c}-L.jpg"),
        (None, Some(i)) => format!("{OPEN_LIBRARY_COVERS}/isbn/{i}-L.jpg?default=false"),
        _ => return Err("Ohne Cover-Kennung oder ISBN lässt sich kein Cover laden.".into()),
    };
    let response = http()?
        .get(url)
        .send()
        .await
        .map_err(|_| OFFLINE.to_string())?;
    if !response.status().is_success() {
        return Err("Für diese Ausgabe gibt es kein Cover.".into());
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_COVER_BYTES as u64)
    {
        return Err("Das Cover ist zu groß.".into());
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_COVER_BYTES {
            return Err("Das Cover ist zu groß.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let _guard = lock();
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    store_cover(&dir, &bytes)?;
    book.cover_source = "openlibrary".into();
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Cover aus einer Bilddatei oder dem Coverbild einer EPUB übernehmen.
#[tauri::command]
#[specta::specta]
pub async fn library_cover_from_file(
    app: AppHandle,
    id: String,
    path: String,
) -> Result<LibraryBook, String> {
    let root = library_root(&app)?;
    let dir = book_dir(&root, &id)?;
    let source = PathBuf::from(&path);
    let is_epub = source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("epub"));
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        if is_epub {
            epub_cover(&source)
        } else {
            let meta = std::fs::metadata(&source).map_err(|e| e.to_string())?;
            if meta.len() as usize > MAX_COVER_BYTES {
                return Err("Das Bild ist zu groß für ein Cover (höchstens 8 MB).".into());
            }
            std::fs::read(&source).map_err(|e| e.to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    let _guard = lock();
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    store_cover(&dir, &bytes)?;
    book.cover_source = if is_epub { "epub" } else { "file" }.into();
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Eigenes Cover entfernen: die Oberfläche zeichnet wieder das generierte.
#[tauri::command]
#[specta::specta]
pub fn library_cover_clear(app: AppHandle, id: String) -> Result<LibraryBook, String> {
    let _guard = lock();
    let root = library_root(&app)?;
    let dir = book_dir(&root, &id)?;
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    for old in ["cover.jpg", "cover.png", "cover.webp"] {
        let _ = std::fs::remove_file(dir.join(old));
    }
    book.cover_source = generated();
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Hörbuch in die Bibliothek kopieren und als Audiofassung eintragen. Die
/// Kopie macht das Buch unabhängig davon, ob die Quelldatei liegen bleibt.
#[tauri::command]
#[specta::specta]
pub async fn library_audio_import(
    app: AppHandle,
    id: String,
    path: String,
) -> Result<LibraryBook, String> {
    let source = PathBuf::from(&path);
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    const AUDIO: [&str; 9] = [
        "mp3", "m4b", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma",
    ];
    if !AUDIO.contains(&ext.as_str()) {
        return Err(format!(
            "Kein Hörbuch-Format: .{ext} (unterstützt: {}).",
            AUDIO.join(", ")
        ));
    }
    let root = library_root(&app)?;
    let dir = book_dir(&root, &id)?;
    if read_book(&dir).is_none() {
        return Err("Buch nicht gefunden.".into());
    }
    let audio_dir = dir.join("audio");
    let file = format!("hoerbuch.{ext}");
    let target = audio_dir.join(&file);
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        std::fs::create_dir_all(&audio_dir).map_err(|e| e.to_string())?;
        // Erst vollständig kopieren, dann umbenennen: ein Abbruch hinterlässt
        // keine halbe Datei unter dem endgültigen Namen.
        // Altes Hörbuch erst entfernen, wenn das neue an seinem Platz ist.
        let tmp = audio_dir.join(format!(
            "{}.part",
            target.file_name().unwrap_or_default().to_string_lossy()
        ));
        std::fs::copy(&source, &tmp).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Kopieren fehlgeschlagen: {e}")
        })?;
        std::fs::rename(&tmp, &target).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })?;
        for entry in std::fs::read_dir(&audio_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            let p = entry.path();
            if p != target && p.file_stem().is_some_and(|s| s == "hoerbuch") {
                let _ = std::fs::remove_file(p);
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    let _guard = lock();
    let Some(mut book) = read_book(&dir) else {
        // Während des Kopierens gelöscht: keinen verwaisten Ordner zurücklassen.
        if !dir.join("buch.json").exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        return Err("Buch nicht gefunden.".into());
    };
    let previous = book.audio.take().unwrap_or_default();
    book.audio = Some(AudioVersion {
        file,
        duration_ms: None,
        narrator: previous.narrator,
        transcript_job: None,
    });
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Gesamter gespeicherter Fortschritt (Schlüssel → JSON-Text).
#[tauri::command]
#[specta::specta]
pub fn library_progress_load(app: AppHandle) -> Result<BTreeMap<String, String>, String> {
    let root = library_root(&app)?;
    let _guard = lock();
    load_progress(&root)
}

/// Fortschritt sichern: Einträge einpflegen und die Datei atomar ersetzen.
#[tauri::command]
#[specta::specta]
pub fn library_progress_put(app: AppHandle, entries: Vec<ProgressEntry>) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    let _guard = lock();
    let root = library_root(&app)?;
    let mut map = load_progress(&root)?;
    merge_progress(&mut map, entries);
    let json = serde_json::to_vec(&map).map_err(|e| e.to_string())?;
    write_atomic(&progress_path(&root), &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(title: &str, authors: &[&str], isbn: Option<&str>) -> LibraryBook {
        LibraryBook {
            id: "book_1".into(),
            title: title.into(),
            authors: authors.iter().map(|a| a.to_string()).collect(),
            isbn: isbn.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn same_work_by_isbn_title_and_author() {
        let effi = book(
            "Effi Briest",
            &["Theodor Fontane"],
            Some("978-3-15-006961-6"),
        );
        assert!(same_work("Irgendwas", &[], Some("9783150069616"), &effi));
        assert!(same_work(
            "effi briest",
            &["Fontane, Theodor".into()],
            None,
            &effi
        ));
        assert!(same_work(
            "Effi Briest: Roman",
            &["T. Fontane".into()],
            None,
            &effi
        ));
        assert!(
            same_work("Effi Briest", &[], None, &effi),
            "ohne Autor zählt der Titel"
        );
        assert!(!same_work(
            "Effi Briest",
            &["Thomas Mann".into()],
            None,
            &effi
        ));
        assert!(!same_work(
            "Frau Jenny Treibel",
            &["Theodor Fontane".into()],
            None,
            &effi
        ));
    }

    #[test]
    fn titles_fold_umlauts_articles_and_subtitles() {
        let faust = book("Faust. Der Tragödie erster Teil", &["Goethe"], None);
        assert!(same_work(
            "FAUST. DER TRAGOEDIE ERSTER TEIL",
            &[],
            None,
            &faust
        ));
        let verw = book("Die Verwandlung", &["Franz Kafka"], None);
        assert!(same_work("Verwandlung", &["Kafka".into()], None, &verw));
        assert!(same_work(
            "Die Verwandlung – Erzählung",
            &["Franz Kafka".into()],
            None,
            &verw
        ));
        let dieb = book("Diebe im Haus", &[], None);
        assert!(
            !same_work("Be im Haus", &[], None, &dieb),
            "„Diebe“ ist kein Artikel"
        );
    }

    #[test]
    fn parses_search_results() {
        let body = r#"{"numFound":2,"docs":[
          {"key":"/works/OL123W","title":"Effi Briest","author_name":["Theodor Fontane"],
           "first_publish_year":1895,"publisher":["Reclam"],"isbn":["3150069618","9783150069616"],
           "cover_i":8231856,"language":["ger"],"number_of_pages_median":336},
          {"key":"/works/OL9W","title":"  ","cover_i":-1},
          {"key":"/books/OL1M","title":"Effi","cover_i":-1}]}"#;
        let hits = parse_search(body, 6).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].isbn.as_deref(), Some("9783150069616"));
        assert_eq!(hits[0].cover_id, Some(8231856));
        assert_eq!(hits[0].work_key.as_deref(), Some("/works/OL123W"));
        assert_eq!(hits[0].pages, Some(336));
        assert_eq!(hits[1].cover_id, None);
        assert_eq!(hits[1].work_key, None);
        assert!(parse_search("kein json", 6).is_err());
    }

    #[test]
    fn parses_descriptions() {
        assert_eq!(
            parse_description(r#"{"description":"Ein Roman."}"#).as_deref(),
            Some("Ein Roman.")
        );
        assert_eq!(
            parse_description(
                r#"{"description":{"type":"/type/text","value":"Text.\n----------\nSiehe auch"}}"#
            )
            .as_deref(),
            Some("Text.")
        );
        assert_eq!(parse_description(r#"{"title":"x"}"#), None);
    }

    #[test]
    fn finds_epub_cover_in_opf() {
        let epub3 = r#"<manifest><item id="c" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/></manifest>"#;
        assert_eq!(epub_cover_href(epub3).as_deref(), Some("images/cover.jpg"));
        let epub2 = r#"<metadata><meta name="cover" content="img1"/></metadata><manifest><item id="x" href="a.html"/><item id="img1" href="Cover%20Bild.png" media-type="image/png"/></manifest>"#;
        assert_eq!(epub_cover_href(epub2).as_deref(), Some("Cover%20Bild.png"));
        assert_eq!(percent_decode("Cover%20Bild.png"), "Cover Bild.png");
        assert_eq!(epub_cover_href("<manifest/>"), None);
    }

    #[test]
    fn covers_must_be_images() {
        let dir = std::env::temp_dir().join(format!("lv-lib-test-{}", std::process::id()));
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpg.resize(2048, 0);
        assert!(store_cover(&dir, &jpg).is_ok());
        assert!(dir.join("cover.jpg").is_file());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(2048, 0);
        assert!(store_cover(&dir, &png).is_ok());
        assert!(!dir.join("cover.jpg").exists(), "altes Cover ersetzt");
        assert!(store_cover(&dir, &[0u8; 2048]).is_err());
        assert!(
            store_cover(&dir, &[0xFF, 0xD8, 0xFF]).is_err(),
            "Platzhalter-Bild"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_foreign_ids() {
        assert!(checked_id("book_123").is_ok());
        assert!(checked_id("book_../x").is_err());
        assert!(checked_id("page_1").is_err());
        assert!(checked_id("book_").is_err());
    }

    #[test]
    fn progress_merge_inserts_replaces_and_deletes() {
        let mut map = BTreeMap::new();
        merge_progress(
            &mut map,
            vec![
                ProgressEntry {
                    key: "a".into(),
                    value: Some("1".into()),
                },
                ProgressEntry {
                    key: "b".into(),
                    value: Some("2".into()),
                },
                ProgressEntry {
                    key: String::new(),
                    value: Some("x".into()),
                },
                ProgressEntry {
                    key: "big".into(),
                    value: Some("x".repeat(MAX_PROGRESS_VALUE + 1)),
                },
            ],
        );
        merge_progress(
            &mut map,
            vec![
                ProgressEntry {
                    key: "a".into(),
                    value: Some("3".into()),
                },
                ProgressEntry {
                    key: "b".into(),
                    value: None,
                },
            ],
        );
        assert_eq!(map.len(), 1);
        assert_eq!(map["a"], "3");
    }

    #[test]
    fn books_roundtrip_without_display_paths() {
        let root = std::env::temp_dir().join(format!("lv-lib-rt-{}", std::process::id()));
        let dir = root.join("book_42");
        let mut b = book("Woyzeck", &["Georg Büchner"], None);
        b.id = "book_42".into();
        b.cover_path = Some("/x".into());
        b.audio = Some(AudioVersion {
            file: "hoerbuch.mp3".into(),
            ..Default::default()
        });
        write_book(&dir, &b).unwrap();
        let raw = std::fs::read_to_string(dir.join("buch.json")).unwrap();
        assert!(!raw.contains("cover_path\": \"/x"));
        let listed = list_books(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].cover_source, "generated");
        assert_eq!(listed[0].audio_path, None, "Hörbuchdatei fehlt");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn broken_progress_file_is_kept_aside_not_overwritten() {
        let root = std::env::temp_dir().join(format!("lv-lib-pr-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            load_progress(&root).unwrap().is_empty(),
            "fehlende Datei = leer"
        );
        std::fs::write(progress_path(&root), "{ kaputt").unwrap();
        assert!(load_progress(&root).unwrap().is_empty());
        let aside: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("fortschritt.json.defekt-")
            })
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(
            std::fs::read_to_string(aside[0].path()).unwrap(),
            "{ kaputt"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn percent_decode_survives_multibyte_after_percent() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("x%é.jpg"), "x%é.jpg");
        assert_eq!(percent_decode("Cover%C3%A4.png"), "Coverä.png");
    }
}
