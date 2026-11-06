//! Bibliothek im Geräte-Sync (lva#210 P5, Spec
//! `docs/superpowers/specs/2026-11-02-bibliothek-sync.md`).
//!
//! Drei Sammlungen:
//! - `book`: `buch.json` ohne lokale Felder, dazu `files` (Verweise in die
//!   Dateiablage) und `updated` (ms);
//! - `progress`: ein Fortschritts-Datensatz (`progress.<schlüssel>` aus
//!   `fortschritt.json`), Kennung `p_` + 40 Hex von SHA-256(schlüssel);
//! - `bookmark`: Lesezeichen und Weiterlese-Stellen (`tts.bookmarks.*`,
//!   `tts.resume.*`), Kennung `b_` + 40 Hex von SHA-256(schlüssel).
//!
//! `updated` eines Buchs führt der Desktop in `<buch>/sync.json`: ändert sich
//! der Inhalt des Buchs, rückt der Zeitpunkt nach. Dort stehen auch die
//! Dateiverweise, die von einem anderen Gerät kamen und noch nicht geladen
//! sind (`pending`).

use super::blobs::{self, FileStamp, Source};
use super::collect::{self, LocalObject};
use super::crypto::KEY_LEN;
use crate::library::{self, LibraryBook};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

pub const COLLECTION_BOOK: &str = "book";
pub const COLLECTION_PROGRESS: &str = "progress";
pub const COLLECTION_BOOKMARK: &str = "bookmark";

const SIDECAR: &str = "sync.json";
const PROGRESS_PREFIX: &str = "progress.";
const BOOKMARK_PREFIXES: [&str; 2] = ["tts.bookmarks.", "tts.resume."];
/// Ein Buchtext größer als das ist kein Buch, sondern ein Irrläufer.
pub const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;
/// Gelöschte Bücher wandern hierhin statt ins Nichts (wie „Entfernen“).
const TRASH: &str = ".papierkorb";

// ---------------------------------------------------------------------------
// Datenformate
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BlobRef {
    pub blob: String,
    pub size: u64,
    pub chunks: u64,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl BlobRef {
    fn new(key: &[u8; KEY_LEN], stamp: &FileStamp) -> Option<Self> {
        Some(Self {
            blob: blobs::blob_id_of(key, stamp)?,
            size: stamp.size,
            chunks: blobs::chunk_count(stamp.size),
            kind: None,
            file: None,
        })
    }

    fn valid(&self) -> bool {
        blobs::valid_blob_id(&self.blob)
            && (self.chunks == blobs::chunk_count(self.size)
                || (self.chunks == 1 && self.size <= blobs::LEGACY_ONE_CHUNK_MAX))
            && self.chunks <= blobs::MAX_CHUNKS
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BookFiles {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<BlobRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<BlobRef>,
    #[serde(default)]
    pub audio: Vec<BlobRef>,
    /// Kurzbücher, je Sprache das neueste (`kurzbuch-<sprache>.md`). Andere Geräte lesen
    /// sie; erzeugt werden sie hier aus den KI-Dokumenten, geladen wird nichts zurück.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kurzbuecher: Vec<BlobRef>,
}

impl BookFiles {
    pub fn refs(&self) -> impl Iterator<Item = &BlobRef> {
        self.text
            .iter()
            .chain(self.cover.iter())
            .chain(self.audio.iter())
            .chain(self.kurzbuecher.iter())
    }

    fn is_empty(&self) -> bool {
        self.text.is_none() && self.cover.is_none() && self.audio.is_empty()
    }
}

/// `<buch>/sync.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Sidecar {
    /// Zuletzt geändert (Unix-ms), wie es im `book`-Objekt steht.
    #[serde(default)]
    pub updated: f64,
    /// Hash des Buchs ohne `updated`: weicht er ab, ist das Buch geändert.
    #[serde(default)]
    pub hash: String,
    /// Von einem anderen Gerät gemeldet, hier noch nicht vorhanden.
    #[serde(default)]
    pub pending: BookFiles,
}

fn read_sidecar(dir: &Path) -> Sidecar {
    std::fs::read_to_string(dir.join(SIDECAR))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_sidecar(dir: &Path, s: &Sidecar) -> Result<(), String> {
    let raw = serde_json::to_vec(s).map_err(|e| e.to_string())?;
    crate::commands::pages::write_atomic(&dir.join(SIDECAR), &raw)
}

/// Werte, die zwischen Rust und Swift gleich aussehen sollen: ohne `null`,
/// ganzzahlige Gleitkommazahlen als Ganzzahl (Swift schreibt `1.0` als `1`).
pub fn normalize(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), normalize(v)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(normalize).collect()),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9.0e15 => json!(f as i64),
            _ => v.clone(),
        },
        other => other.clone(),
    }
}

fn hex40(key: &str) -> String {
    blobs::hex(&Sha256::digest(key.as_bytes()))[..40].to_string()
}

pub fn progress_object_id(key: &str) -> String {
    format!("p_{}", hex40(key))
}

pub fn bookmark_object_id(key: &str) -> String {
    format!("b_{}", hex40(key))
}

fn is_bookmark_key(k: &str) -> bool {
    BOOKMARK_PREFIXES.iter().any(|p| k.starts_with(p))
}

/// Das Buch als Sync-Wert ohne `files`/`updated`: Anzeigefelder und
/// gerätelokale Transkriptions-Aufträge fallen weg.
pub fn book_core(book: &LibraryBook) -> Value {
    let mut v = serde_json::to_value(book).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        for k in ["cover_path", "audio_path", "audio_paths"] {
            m.remove(k);
        }
        if let Some(Value::Object(audio)) = m.get_mut("audio") {
            audio.remove("transcript_job");
            if let Some(Value::Array(tracks)) = audio.get_mut("tracks") {
                for t in tracks.iter_mut() {
                    if let Value::Object(t) = t {
                        t.remove("transcript_job");
                        t.remove("transcript_jobs");
                    }
                }
            }
        }
    }
    normalize(&v)
}

fn with_files(core: &Value, files: &BookFiles) -> Value {
    let mut v = core.clone();
    if let Value::Object(m) = &mut v {
        m.insert(
            "files".into(),
            normalize(&serde_json::to_value(files).unwrap_or(Value::Null)),
        );
    }
    v
}

fn with_updated(v: &Value, updated: f64) -> Value {
    let mut v = v.clone();
    if let Value::Object(m) = &mut v {
        m.insert("updated".into(), normalize(&json!(updated)));
    }
    v
}

// ---------------------------------------------------------------------------
// Fortschritt zusammenführen (wie `mergeRecordText` in progress.ts)
// ---------------------------------------------------------------------------

fn ranges(v: Option<&Value>) -> Vec<(f64, f64)> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let r = r.as_array()?;
                    Some((r.first()?.as_f64()?, r.get(1)?.as_f64()?))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn merge_ranges(mut rs: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    rs.retain(|(a, b)| a.is_finite() && b.is_finite() && b > a);
    rs.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for r in rs {
        match out.last_mut() {
            Some(last) if r.0 <= last.1 => last.1 = last.1.max(r.1),
            _ => out.push(r),
        }
    }
    out
}

/// Zwei Fassungen eines Datensatzes: Stelle und Anker vom neueren `updated`
/// (bei Gleichstand gewinnt `local`), Bereiche vereinigt, Länge = Maximum,
/// „fertig“ bleibt. `None`, wenn eine Seite kein Datensatz ist.
pub fn merge_record(local: &Value, remote: &Value) -> Option<Value> {
    let (x, y) = (local.as_object()?, remote.as_object()?);
    x.get("offset")?.as_f64()?;
    y.get("offset")?.as_f64()?;
    let upd = |m: &Map<String, Value>| m.get("updated").and_then(Value::as_f64).unwrap_or(0.0);
    let (older, newer) = if upd(x) >= upd(y) { (y, x) } else { (x, y) };
    let mut merged = older.clone();
    for (k, v) in newer {
        merged.insert(k.clone(), v.clone());
    }
    for field in ["heard", "read"] {
        let mut all = ranges(older.get(field));
        all.extend(ranges(newer.get(field)));
        let joined: Vec<Value> = merge_ranges(all)
            .into_iter()
            .map(|(a, b)| json!([a, b]))
            .collect();
        merged.insert(field.into(), Value::Array(joined));
    }
    let len = |m: &Map<String, Value>| m.get("length").and_then(Value::as_f64).unwrap_or(0.0);
    merged.insert("length".into(), json!(len(newer).max(len(older))));
    let fin = |m: &Map<String, Value>| m.get("finished").and_then(Value::as_bool) == Some(true);
    if fin(older) || fin(newer) {
        merged.insert("finished".into(), json!(true));
    }
    Some(normalize(&Value::Object(merged)))
}

// ---------------------------------------------------------------------------
// Sammeln
// ---------------------------------------------------------------------------

/// Eine Datei, die vor dem Push des Buchs in der Ablage sein muss.
#[derive(Clone, Debug)]
pub struct PendingUpload {
    pub blob: BlobRef,
    pub source: Source,
    /// Bei Dateien: Größe, Zeit und Prüfsumme, aus denen die Kennung stammt.
    /// Weicht die Datei beim Hochladen davon ab, wird nicht abgeschlossen.
    pub stamp: Option<FileStamp>,
}

#[derive(Default)]
pub struct LibraryLocal {
    pub objects: Vec<LocalObject>,
    /// Buch-Kennung → Dateien dieses Buchs, die lokal vorliegen.
    pub uploads: BTreeMap<String, Vec<PendingUpload>>,
    /// Bücher, die es gibt, die aber in diesem Lauf nicht gepusht werden
    /// (Prüfsummen noch nicht fertig, `buch.json` unlesbar). Sie gelten
    /// nicht als gelöscht.
    pub held: BTreeSet<String>,
    /// Einzelne Bücher, die nicht gesammelt werden konnten (für den Status).
    pub warnings: Vec<String>,
}

struct BookBuild {
    core_files: Value,
    uploads: Vec<PendingUpload>,
    ready: bool,
}

fn page_text(app: &AppHandle, page_id: &str) -> Option<String> {
    collect::read_page(app, page_id)
        .ok()
        .flatten()
        .and_then(|v| v.get("state")?.get("text")?.as_str().map(str::to_owned))
}

/// Neuestes Kurzbuch je Sprache einer Vorlesen-Seite, nach Sprache sortiert.
fn kurzbuecher(app: &AppHandle, page_id: &str) -> Vec<(String, String)> {
    let Some(dir) = crate::commands::pages::page_path(app, page_id).ok().filter(|d| d.is_dir()) else {
        return Vec::new();
    };
    let docs = crate::book_ai::store::list(&dir).unwrap_or_default();
    newest_per_language(&docs)
        .into_iter()
        .filter_map(|(language, id)| {
            let text = crate::book_ai::store::read_text(&dir, &id).ok()?;
            Some((language, text))
        })
        .collect()
}

/// Sprache → Dokument; ohne Angabe gilt Deutsch wie bei älteren Kurzbüchern.
fn newest_per_language(docs: &[crate::book_ai::store::KiDocument]) -> Vec<(String, String)> {
    let mut newest: BTreeMap<String, &crate::book_ai::store::KiDocument> = BTreeMap::new();
    for doc in docs.iter().filter(|d| d.template_id == crate::book_ai::kurzbuch::TEMPLATE_ID) {
        let language = doc.language.clone().filter(|l| library::sync_plain_name(l)).unwrap_or_else(|| "de".into());
        let known = newest.get(&language).map_or(f64::MIN, |d| d.modified_ms);
        if doc.modified_ms >= known {
            newest.insert(language, doc);
        }
    }
    newest.into_iter().map(|(l, d)| (l, d.id.clone())).collect()
}

fn build_book(
    app: &AppHandle,
    key: &[u8; KEY_LEN],
    dir: &Path,
    book: &LibraryBook,
    side: &Sidecar,
    cache: &mut BTreeMap<String, FileStamp>,
    budget: &mut u64,
) -> Result<BookBuild, String> {
    let mut files = BookFiles::default();
    let mut uploads = Vec::new();
    let mut ready = true;

    // Text: die Vorlesen-Seite ist die Quelle; fehlt sie, der gemeldete Verweis.
    let text = book
        .text
        .as_ref()
        .filter(|t| !t.page_id.is_empty())
        .and_then(|t| page_text(app, &t.page_id))
        .filter(|t| !t.is_empty());
    match text {
        Some(t) => {
            let bytes = t.into_bytes();
            if let Some(r) = BlobRef::new(key, &blobs::stamp_bytes(&bytes)) {
                uploads.push(PendingUpload {
                    blob: r.clone(),
                    source: Source::Bytes(bytes),
                    stamp: None,
                });
                files.text = Some(r);
            }
        }
        None => files.text = side.pending.text.clone().filter(|_| book.text.is_some()),
    }

    match library::sync_cover_file(dir) {
        Some(path) => match blobs::stamp(cache, &path, budget)? {
            Some(s) => {
                if let Some(mut r) = BlobRef::new(key, &s) {
                    r.kind = path.extension().map(|e| e.to_string_lossy().to_lowercase());
                    uploads.push(PendingUpload {
                        blob: r.clone(),
                        source: Source::File(path),
                        stamp: Some(s),
                    });
                    files.cover = Some(r);
                }
            }
            None => ready = false,
        },
        None => files.cover = side.pending.cover.clone(),
    }

    if let Some(audio) = &book.audio {
        for track in library::tracks_of(audio) {
            let path = dir.join("audio").join(&track.file);
            if path.is_file() {
                match blobs::stamp(cache, &path, budget)? {
                    Some(s) => {
                        if let Some(mut r) = BlobRef::new(key, &s) {
                            r.file = Some(track.file.clone());
                            uploads.push(PendingUpload {
                                blob: r.clone(),
                                source: Source::File(path),
                                stamp: Some(s),
                            });
                            files.audio.push(r);
                        }
                    }
                    None => ready = false,
                }
            } else if let Some(r) = side
                .pending
                .audio
                .iter()
                .find(|r| r.file.as_deref() == Some(track.file.as_str()))
            {
                files.audio.push(r.clone());
            }
        }
    }
    for (language, text) in book.text.as_ref().map(|t| kurzbuecher(app, &t.page_id)).unwrap_or_default() {
        let bytes = text.into_bytes();
        if let Some(mut r) = BlobRef::new(key, &blobs::stamp_bytes(&bytes)) {
            r.kind = Some("md".into());
            r.file = Some(format!("kurzbuch-{language}.md"));
            uploads.push(PendingUpload {
                blob: r.clone(),
                source: Source::Bytes(bytes),
                stamp: None,
            });
            files.kurzbuecher.push(r);
        }
    }
    Ok(BookBuild {
        core_files: with_files(&book_core(book), &files),
        uploads,
        ready,
    })
}

/// Prüfsummen der Dateien eines Buchs vorab berechnen, OHNE die
/// Bibliotheks-Sperre: das Einlesen großer Hörbücher dauert, und die
/// Oberfläche sichert währenddessen Fortschritt über dieselbe Sperre. Unter
/// der Sperre reicht danach der Zwischenspeicher.
fn prewarm(
    dir: &Path,
    book: &LibraryBook,
    cache: &mut BTreeMap<String, FileStamp>,
    budget: &mut u64,
) {
    let mut paths: Vec<PathBuf> = library::sync_cover_file(dir).into_iter().collect();
    if let Some(audio) = &book.audio {
        paths.extend(
            library::tracks_of(audio)
                .iter()
                .map(|t| dir.join("audio").join(&t.file))
                .filter(|p| p.is_file()),
        );
    }
    for p in paths {
        if let Err(e) = blobs::stamp(cache, &p, budget) {
            log::warn!("Sync: Prüfsumme von {} nicht lesbar: {e}", p.display());
        }
    }
}

fn mtime_ms(path: &Path) -> f64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

fn now_ms() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64
}

/// Den Zeitpunkt eines Buchs fortschreiben: ändert sich sein Inhalt, gilt
/// „jetzt“; das erste Mal die Änderungszeit von `buch.json`.
fn stamp_updated(dir: &Path, side: &mut Sidecar, core_files: &Value) -> Result<f64, String> {
    let h = collect::hash(core_files);
    if side.hash != h {
        side.updated = if side.hash.is_empty() {
            mtime_ms(&dir.join("buch.json"))
        } else {
            now_ms()
        };
        side.hash = h;
        write_sidecar(dir, side)?;
    }
    Ok(side.updated)
}

/// Bücher mit gültiger Kennung, auch wenn `buch.json` gerade unlesbar ist.
fn book_dirs(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let entries = std::fs::read_dir(root).map_err(|e| format!("Bibliothek nicht lesbar: {e}"))?;
    Ok(entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            library::sync_book_dir(root, &id).ok().map(|d| (id, d))
        })
        .collect())
}

/// Alle Bibliotheks-Objekte. `cache` sind die Prüfsummen der Dateien (im
/// Ledger), `budget` begrenzt, was ein Lauf neu einlesen darf.
pub fn collect_library(
    app: &AppHandle,
    key: &[u8; KEY_LEN],
    cache: &mut BTreeMap<String, FileStamp>,
    budget: &mut u64,
) -> Result<LibraryLocal, String> {
    let root = library::sync_root(app)?;
    for (_, dir) in book_dirs(&root)? {
        if let Some(book) = library::sync_read_book(&dir) {
            prewarm(&dir, &book, cache, budget);
        }
    }
    let _guard = library::sync_lock();
    let mut out = LibraryLocal::default();
    for (id, dir) in book_dirs(&root)? {
        let Some(book) = library::sync_read_book(&dir) else {
            out.held.insert(id);
            continue;
        };
        let mut side = read_sidecar(&dir);
        // Nur noch der Zwischenspeicher: wer jetzt ungeprüft ist, wartet.
        let mut no_budget = 0;
        let built = match build_book(app, key, &dir, &book, &side, cache, &mut no_budget) {
            Ok(b) => b,
            Err(e) => {
                // Ein Buch mit unlesbarer Datei hält den Abgleich nicht an.
                log::warn!("Sync: Buch {id} übersprungen: {e}");
                out.warnings.push(format!("Buch „{}“: {e}", book.title));
                out.held.insert(id);
                continue;
            }
        };
        if !built.ready {
            out.held.insert(id);
            continue;
        }
        let updated = match stamp_updated(&dir, &mut side, &built.core_files) {
            Ok(u) => u,
            Err(e) => {
                out.warnings.push(format!("Buch „{}“: {e}", book.title));
                out.held.insert(id);
                continue;
            }
        };
        let value = with_updated(&built.core_files, updated);
        if collect::canonical(&value).len() > collect::MAX_PLAINTEXT {
            // Vor dem Hochladen prüfen, sonst gehen Gigabytes für ein Buch
            // hinaus, das der Hub dann abweist.
            out.warnings.push(format!(
                "Buch „{}“ ist für den Abgleich zu groß",
                book.title
            ));
            out.held.insert(id);
            continue;
        }
        out.uploads.insert(id.clone(), built.uploads);
        out.objects.push(LocalObject {
            collection: COLLECTION_BOOK,
            id,
            value,
        });
    }
    // Prüfsummen verschwundener Dateien nicht ewig mitschleppen.
    cache.retain(|p, _| Path::new(p).is_file());

    let map = progress_for_sync(&root)?;
    out.objects.extend(progress_objects(&map));
    Ok(out)
}

/// `fortschritt.json` für den Sync. Anders als der Befehl der Oberfläche
/// legt ein unlesbarer Stand hier nichts beiseite und gilt nicht als leer:
/// sonst schickte der Sync für jeden Eintrag einen Grabstein.
fn progress_for_sync(root: &Path) -> Result<BTreeMap<String, String>, String> {
    match std::fs::read_to_string(root.join("fortschritt.json")) {
        Ok(raw) => {
            serde_json::from_str(&raw).map_err(|e| format!("fortschritt.json unlesbar: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(format!("fortschritt.json nicht lesbar: {e}")),
    }
}

/// So, wie das iPhone einen Datensatz liest: `offset` Pflicht, die übrigen
/// Felder vom erwarteten Typ, wenn vorhanden.
fn valid_record(r: &Value) -> bool {
    let Some(m) = r.as_object() else {
        return false;
    };
    let num = |k: &str| m.get(k).is_none_or(|v| v.is_number() || v.is_null());
    m.get("offset").is_some_and(Value::is_number)
        && num("length")
        && num("updated")
        && m.get("anchor").is_none_or(|v| v.is_string() || v.is_null())
        && m.get("finished")
            .is_none_or(|v| v.is_boolean() || v.is_null())
}

pub fn progress_objects(map: &BTreeMap<String, String>) -> Vec<LocalObject> {
    let mut out = Vec::new();
    for (k, raw) in map {
        if let Some(sub) = k.strip_prefix(PROGRESS_PREFIX) {
            let Ok(record) = serde_json::from_str::<Value>(raw) else {
                continue;
            };
            if !valid_record(&record) {
                continue;
            }
            out.push(LocalObject {
                collection: COLLECTION_PROGRESS,
                id: progress_object_id(sub),
                value: json!({ "key": sub, "record": normalize(&record) }),
            });
        } else if is_bookmark_key(k) {
            out.push(LocalObject {
                collection: COLLECTION_BOOKMARK,
                id: bookmark_object_id(k),
                value: json!({ "key": k, "value": raw }),
            });
        }
    }
    out
}

/// Hash des lokalen Stands eines Objekts (für das Ledger nach dem Anwenden),
/// `None`, wenn es lokal nicht existiert.
pub fn local_hash(
    app: &AppHandle,
    key: &[u8; KEY_LEN],
    collection: &str,
    id: &str,
    cache: &mut BTreeMap<String, FileStamp>,
) -> Result<Option<String>, String> {
    let root = library::sync_root(app)?;
    match collection {
        COLLECTION_BOOK => {
            let Ok(dir) = library::sync_book_dir(&root, id) else {
                return Ok(None);
            };
            if let Some(book) = library::sync_read_book(&dir) {
                prewarm(&dir, &book, cache, &mut u64::MAX);
            }
            let _guard = library::sync_lock();
            book_hash_locked(app, key, &dir, cache)
        }
        _ => {
            let map = {
                let _guard = library::sync_lock();
                progress_for_sync(&root)?
            };
            Ok(progress_objects(&map)
                .into_iter()
                .find(|o| o.collection == collection && o.id == id)
                .map(|o| collect::hash(&o.value)))
        }
    }
}

// ---------------------------------------------------------------------------
// Anwenden
// ---------------------------------------------------------------------------

/// Was das Anwenden eines Objekts bewirkt hat.
#[derive(Default, Debug)]
pub struct Applied {
    pub changed: bool,
    /// Blobs, die laut Absender in der Ablage liegen.
    pub remote_blobs: Vec<String>,
    /// Fortschritts-/Lesezeichen-Schlüssel, die ein Grabstein entfernt hat
    /// (die Oberfläche löscht genau diese aus ihrem Zwischenspeicher).
    pub removed_keys: Vec<String>,
}

/// Fehler beim Anwenden: ein unbrauchbares Objekt wird übersprungen, ein
/// Fehler der Ablage (Datei gesperrt, Platte voll) im nächsten Lauf erneut
/// versucht, ohne dass das Ledger die Revision als angewendet verbucht.
#[derive(Debug)]
pub enum ApplyError {
    Invalid(String),
    Retry(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::Invalid(s) | ApplyError::Retry(s) => write!(f, "{s}"),
        }
    }
}

impl From<String> for ApplyError {
    fn from(s: String) -> Self {
        ApplyError::Retry(s)
    }
}

impl From<&str> for ApplyError {
    fn from(s: &str) -> Self {
        ApplyError::Retry(s.to_string())
    }
}

fn invalid(s: impl Into<String>) -> ApplyError {
    ApplyError::Invalid(s.into())
}

fn trash_target(root: &Path, name: &str) -> Result<PathBuf, String> {
    let trash = root.join(TRASH);
    std::fs::create_dir_all(&trash).map_err(|e| format!("Papierkorb: {e}"))?;
    let target = trash.join(name);
    Ok(if target.exists() {
        trash.join(format!("{name}-{}", now_ms() as i64))
    } else {
        target
    })
}

fn move_to_trash(root: &Path, dir: &Path, id: &str) -> Result<(), String> {
    let target = trash_target(root, id)?;
    std::fs::rename(dir, &target).map_err(|e| format!("Buch verschieben: {e}"))
}

/// Hash des Buchs wie beim Sammeln; der Aufrufer hält die Sperre und hat
/// die Prüfsummen vorher berechnet.
fn book_hash_locked(
    app: &AppHandle,
    key: &[u8; KEY_LEN],
    dir: &Path,
    cache: &mut BTreeMap<String, FileStamp>,
) -> Result<Option<String>, String> {
    let Some(book) = library::sync_read_book(dir) else {
        return Ok(None);
    };
    let mut side = read_sidecar(dir);
    let mut budget = u64::MAX;
    let built = build_book(app, key, dir, &book, &side, cache, &mut budget)?;
    let updated = stamp_updated(dir, &mut side, &built.core_files)?;
    Ok(Some(collect::hash(&with_updated(
        &built.core_files,
        updated,
    ))))
}

/// Ein `book`-Objekt anwenden. Neueres `updated` gewinnt; ist der lokale
/// Stand ungesichert und neuer, bleibt er (und wird gepusht).
pub fn apply_book(
    app: &AppHandle,
    key: &[u8; KEY_LEN],
    id: &str,
    value: Option<&Value>,
    synced_hash: Option<&str>,
    cache: &mut BTreeMap<String, FileStamp>,
) -> Result<Applied, ApplyError> {
    let root = library::sync_root(app)?;
    let dir = library::sync_book_dir(&root, id).map_err(invalid)?;

    // Prüfen, bevor etwas geschrieben wird.
    let parsed = match value {
        None => None,
        Some(value) => {
            let remote_updated = value.get("updated").and_then(Value::as_f64).unwrap_or(0.0);
            let remote_files: BookFiles = value
                .get("files")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| invalid(format!("Dateiverweise unlesbar: {e}")))?
                .unwrap_or_default();
            if remote_files.refs().any(|r| !r.valid()) {
                return Err(invalid("Dateiverweis ungültig"));
            }
            let mut core = value.clone();
            if let Value::Object(m) = &mut core {
                m.remove("files");
                m.remove("updated");
                m.insert("id".into(), json!(id));
            }
            let mut incoming: LibraryBook =
                serde_json::from_value(core).map_err(|e| invalid(format!("Buch unlesbar: {e}")))?;
            if incoming.title.trim().is_empty() {
                return Err(invalid("Buch ohne Titel"));
            }
            if let Some(audio) = &incoming.audio {
                if library::tracks_of(audio)
                    .iter()
                    .any(|t| !library::sync_plain_name(&t.file))
                {
                    return Err(invalid("Ungültiger Dateiname im Hörbuch"));
                }
            }
            incoming.cover_path = None;
            incoming.audio_path = None;
            incoming.audio_paths = Vec::new();
            Some((remote_updated, remote_files, incoming))
        }
    };

    if let Some(book) = library::sync_read_book(&dir) {
        prewarm(&dir, &book, cache, &mut u64::MAX);
    }
    let _guard = library::sync_lock();
    let local_hash_now = if dir.is_dir() {
        book_hash_locked(app, key, &dir, cache)?
    } else {
        None
    };
    let local_clean = match (&local_hash_now, synced_hash) {
        (None, _) => true,
        (Some(h), Some(s)) => h == s,
        (Some(_), None) => false,
    };

    let Some((remote_updated, remote_files, mut incoming)) = parsed else {
        // Grabstein: nur ein gesicherter lokaler Stand geht in den Papierkorb.
        if dir.is_dir() && local_clean {
            move_to_trash(&root, &dir, id)?;
            return Ok(Applied {
                changed: true,
                ..Default::default()
            });
        }
        return Ok(Applied::default());
    };

    let existing = library::sync_read_book(&dir);
    let mut side = read_sidecar(&dir);
    if existing.is_some() && !local_clean && side.updated > remote_updated {
        return Ok(Applied::default());
    }
    // Gerätelokale Aufträge der vorhandenen Fassung behalten.
    if let (Some(old), Some(new)) = (
        existing.as_ref().and_then(|b| b.audio.as_ref()),
        incoming.audio.as_mut(),
    ) {
        if old.file == new.file {
            new.transcript_job = old.transcript_job.clone();
        }
        for t in new.tracks.iter_mut() {
            if let Some(o) = old.tracks.iter().find(|o| o.file == t.file) {
                t.transcript_job = o.transcript_job.clone();
                t.transcript_jobs = o.transcript_jobs.clone();
            }
        }
    }
    library::sync_write_book(&dir, &incoming)?;

    // Was fehlt hier noch? Nur das kommt nach `pending`.
    let mut pending = BookFiles::default();
    let have_text = incoming
        .text
        .as_ref()
        .and_then(|t| page_text(app, &t.page_id))
        .is_some();
    if !have_text {
        pending.text = remote_files.text.clone();
    }
    if let Some(r) = &remote_files.cover {
        let mut b = u64::MAX;
        let local = library::sync_cover_file(&dir)
            .and_then(|p| blobs::stamp(cache, &p, &mut b).ok().flatten())
            .and_then(|s| blobs::blob_id_of(key, &s));
        if local.as_deref() != Some(r.blob.as_str()) {
            pending.cover = Some(r.clone());
        }
    }
    for r in &remote_files.audio {
        let Some(name) = r.file.as_deref().filter(|n| library::sync_plain_name(n)) else {
            continue;
        };
        let path = dir.join("audio").join(name);
        let mut b = u64::MAX;
        let local = path
            .is_file()
            .then(|| blobs::stamp(cache, &path, &mut b).ok().flatten())
            .flatten()
            .and_then(|s| blobs::blob_id_of(key, &s));
        if local.as_deref() != Some(r.blob.as_str()) {
            pending.audio.push(r.clone());
        }
    }
    side.pending = pending;
    side.updated = remote_updated;
    let book = library::sync_read_book(&dir).ok_or("Buch nach dem Schreiben unlesbar")?;
    let mut budget = u64::MAX;
    let built = build_book(app, key, &dir, &book, &side, cache, &mut budget)?;
    side.hash = collect::hash(&built.core_files);
    write_sidecar(&dir, &side)?;
    Ok(Applied {
        changed: true,
        remote_blobs: remote_files.refs().map(|r| r.blob.clone()).collect(),
        ..Default::default()
    })
}

/// Zwei Lesezeichen-Listen vereinigen (nach `id`, lokale zuerst). `None`,
/// wenn eine Seite keine Liste ist.
pub fn merge_bookmarks(local: &str, remote: &str) -> Option<String> {
    let mut mine: Vec<Value> = serde_json::from_str(local).ok()?;
    let theirs: Vec<Value> = serde_json::from_str(remote).ok()?;
    let ids: BTreeSet<String> = mine
        .iter()
        .filter_map(|b| b.get("id")?.as_str().map(str::to_owned))
        .collect();
    for b in theirs {
        let new = b
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !ids.contains(id));
        if new {
            mine.push(b);
        }
    }
    serde_json::to_string(&mine).ok()
}

/// Ein `progress`- oder `bookmark`-Objekt anwenden (`None` = Grabstein).
pub fn apply_progress(
    app: &AppHandle,
    collection: &str,
    id: &str,
    value: Option<&Value>,
    synced_hash: Option<&str>,
) -> Result<Applied, ApplyError> {
    let root = library::sync_root(app)?;
    let _guard = library::sync_lock();
    let mut map = progress_for_sync(&root)?;
    let current = progress_objects(&map)
        .into_iter()
        .find(|o| o.collection == collection && o.id == id);
    let local_clean = match (&current, synced_hash) {
        (None, _) => true,
        (Some(o), Some(s)) => collect::hash(&o.value) == s,
        (Some(_), None) => false,
    };

    let Some(value) = value else {
        if let Some(o) = current.filter(|_| local_clean) {
            let key = o
                .value
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let k = match collection {
                COLLECTION_PROGRESS => format!("{PROGRESS_PREFIX}{key}"),
                _ => key.to_string(),
            };
            map.remove(&k);
            library::sync_progress_store(&root, &map)?;
            return Ok(Applied {
                changed: true,
                removed_keys: vec![k],
                ..Default::default()
            });
        }
        return Ok(Applied::default());
    };

    let sub = value
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("Fortschritt ohne Schlüssel"))?;
    let (map_key, stored) = match collection {
        COLLECTION_PROGRESS => {
            if progress_object_id(sub) != id {
                return Err(invalid("Fortschritt: Schlüssel passt nicht zur Kennung"));
            }
            let record = value
                .get("record")
                .filter(|r| valid_record(r))
                .ok_or_else(|| invalid("Fortschritt ohne gültigen Datensatz"))?;
            let map_key = format!("{PROGRESS_PREFIX}{sub}");
            let merged = match map
                .get(&map_key)
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            {
                Some(local) => merge_record(&local, record).unwrap_or_else(|| record.clone()),
                None => record.clone(),
            };
            (
                map_key,
                serde_json::to_string(&merged).map_err(|e| e.to_string())?,
            )
        }
        _ => {
            if !is_bookmark_key(sub) || bookmark_object_id(sub) != id {
                return Err(invalid("Lesezeichen: Schlüssel passt nicht zur Kennung"));
            }
            let raw = value
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Lesezeichen ohne Wert"))?;
            let stored = match map.get(sub) {
                // Ungesichert lokal geändert: Lesezeichen beider Seiten
                // bleiben; eine Weiterlese-Stelle bleibt die lokale.
                Some(mine) if !local_clean => match merge_bookmarks(mine, raw) {
                    Some(m) => m,
                    None => return Ok(Applied::default()),
                },
                _ => raw.to_string(),
            };
            (sub.to_string(), stored)
        }
    };
    if map_key.len() > library::SYNC_MAX_PROGRESS_KEY
        || stored.len() > library::SYNC_MAX_PROGRESS_VALUE
    {
        return Err(invalid("Fortschritt zu groß"));
    }
    if map.get(&map_key) == Some(&stored) {
        return Ok(Applied::default());
    }
    map.insert(map_key, stored);
    library::sync_progress_store(&root, &map)?;
    Ok(Applied {
        changed: true,
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Herunterladen
// ---------------------------------------------------------------------------

/// Eine noch fehlende Datei eines Buchs.
#[derive(Clone, Debug)]
pub struct Wanted {
    pub book_id: String,
    pub dir: PathBuf,
    pub blob: BlobRef,
    pub what: WantedKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WantedKind {
    Text,
    Cover,
    Audio(String),
}

/// Alles, was andere Geräte gemeldet haben und hier fehlt. Kleines zuerst:
/// Text und Cover vor Hörbuchteilen.
pub fn wanted(app: &AppHandle) -> Result<Vec<Wanted>, String> {
    let _guard = library::sync_lock();
    let root = library::sync_root(app)?;
    let mut out = Vec::new();
    for (id, dir) in book_dirs(&root)? {
        let side = read_sidecar(&dir);
        if side.pending.is_empty() {
            continue;
        }
        let mut push = |blob: &BlobRef, what: WantedKind| {
            out.push(Wanted {
                book_id: id.clone(),
                dir: dir.clone(),
                blob: blob.clone(),
                what,
            })
        };
        if let Some(t) = &side.pending.text {
            if t.size <= MAX_TEXT_BYTES {
                push(t, WantedKind::Text);
            }
        }
        if let Some(c) = &side.pending.cover {
            push(c, WantedKind::Cover);
        }
        for a in &side.pending.audio {
            if let Some(f) = a.file.as_deref().filter(|f| library::sync_plain_name(f)) {
                push(a, WantedKind::Audio(f.to_string()));
            }
        }
    }
    out.sort_by_key(|w| (matches!(w.what, WantedKind::Audio(_)), w.blob.size));
    Ok(out)
}

fn cover_ext(kind: Option<&str>) -> &'static str {
    match kind.map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "png",
        Some("webp") => "webp",
        _ => "jpg",
    }
}

/// Temporäres Ziel des Downloads; erst [`finish_download`] legt die Datei ab.
pub fn download_target(w: &Wanted) -> PathBuf {
    match &w.what {
        WantedKind::Text => w.dir.join("sync-text.download"),
        WantedKind::Cover => w.dir.join("cover.download"),
        WantedKind::Audio(f) => w.dir.join("audio").join(format!("{f}.download")),
    }
}

/// Die geladene Datei an ihren Platz bringen und den Verweis aus `pending`
/// nehmen. Ein Text wird zur Vorlesen-Seite des Buchs. Liefert den Pfad der
/// abgelegten Datei (für den Prüfsummen-Zwischenspeicher).
pub fn finish_download(
    app: &AppHandle,
    w: &Wanted,
    device_name: &str,
) -> Result<Option<PathBuf>, String> {
    let tmp = download_target(w);
    let _guard = library::sync_lock();
    if !w.dir.join("buch.json").is_file() {
        let _ = std::fs::remove_file(&tmp);
        return Err("Buch verschwunden".into());
    }
    match &w.what {
        WantedKind::Text => {
            let bytes = std::fs::read(&tmp).map_err(|e| format!("Buchtext: {e}"));
            let _ = std::fs::remove_file(&tmp);
            let text = String::from_utf8_lossy(&bytes?).into_owned();
            let mut book = library::sync_read_book(&w.dir).ok_or("Buch unlesbar")?;
            let page_id = book
                .text
                .as_ref()
                .map(|t| t.page_id.clone())
                .filter(|p| crate::commands::pages::page_path(app, p).is_ok())
                .unwrap_or_else(crate::commands::pages::fresh_id);
            if collect::read_page(app, &page_id)?.is_none() {
                collect::apply_page(
                    app,
                    &page_id,
                    &json!({ "title": book.title, "state": { "text": text, "tab": "original" } }),
                    None,
                    device_name,
                )?;
            }
            if let Some(t) = book.text.as_mut() {
                if t.page_id != page_id {
                    t.page_id = page_id;
                    library::sync_write_book(&w.dir, &book)?;
                }
            }
            clear_pending(&w.dir, &w.what)?;
            Ok(None)
        }
        WantedKind::Cover => {
            // Erst das neue Cover ablegen, dann die alten entfernen: scheitert
            // das Umbenennen, bleibt das bisherige Cover.
            let name = format!("cover.{}", cover_ext(w.blob.kind.as_deref()));
            let target = w.dir.join(&name);
            std::fs::rename(&tmp, &target).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("Cover ablegen: {e}")
            })?;
            for old in ["cover.jpg", "cover.png", "cover.webp"] {
                if old != name {
                    let _ = std::fs::remove_file(w.dir.join(old));
                }
            }
            clear_pending(&w.dir, &w.what)?;
            Ok(Some(target))
        }
        WantedKind::Audio(f) => {
            let target = w.dir.join("audio").join(f);
            // Eine abweichende Datei gleichen Namens geht in den Papierkorb.
            if target.is_file() {
                let root = library::sync_root(app)?;
                let aside = trash_target(&root, &format!("{}-{f}", w.book_id))?;
                std::fs::rename(&target, &aside).map_err(|e| format!("Teil beiseitelegen: {e}"))?;
            }
            std::fs::rename(&tmp, &target).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("Hörbuchteil ablegen: {e}")
            })?;
            clear_pending(&w.dir, &w.what)?;
            Ok(Some(target))
        }
    }
}

fn clear_pending(dir: &Path, what: &WantedKind) -> Result<(), String> {
    let mut side = read_sidecar(dir);
    match what {
        WantedKind::Text => side.pending.text = None,
        WantedKind::Cover => side.pending.cover = None,
        WantedKind::Audio(f) => side
            .pending
            .audio
            .retain(|r| r.file.as_deref() != Some(f.as_str())),
    }
    write_sidecar(dir, &side)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kennungen_wie_swift() {
        // SHA-256("audio:book_1") beginnt mit diesen 40 Hex (gegen sha256sum geprüft).
        let id = progress_object_id("audio:book_1");
        assert!(id.starts_with("p_"));
        assert_eq!(id.len(), 42);
        assert_eq!(&id[2..], &blobs::sha256_hex(b"audio:book_1")[..40]);
        assert_eq!(
            bookmark_object_id("tts.bookmarks.page_1"),
            format!("b_{}", &blobs::sha256_hex(b"tts.bookmarks.page_1")[..40])
        );
    }

    #[test]
    fn normalisieren_wie_swift() {
        let v = json!({"a": 1.0, "b": null, "c": [2.5, {"d": null, "e": 3.0}], "f": "x"});
        assert_eq!(
            normalize(&v),
            json!({"a": 1, "c": [2.5, {"e": 3}], "f": "x"})
        );
        assert_eq!(
            collect::hash(&normalize(&json!({"t": 1730000000000.0}))),
            collect::hash(&json!({"t": 1730000000000_i64}))
        );
    }

    #[test]
    fn buch_ohne_lokale_felder() {
        let book = LibraryBook {
            id: "book_1".into(),
            title: "Effi Briest".into(),
            cover_source: "epub".into(),
            cover_path: Some("C:/x/cover.jpg?v=1".into()),
            audio_path: Some("C:/x/audio/teil_001.mp3".into()),
            audio_paths: vec!["C:/x".into()],
            audio: Some(library::AudioVersion {
                file: "teil_001.mp3".into(),
                transcript_job: Some("job".into()),
                tracks: vec![library::AudioTrack {
                    file: "teil_001.mp3".into(),
                    title: "Teil 1".into(),
                    transcript_job: Some("job".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            added_ms: 1730000000000.0,
            ..Default::default()
        };
        let v = book_core(&book);
        let m = v.as_object().unwrap();
        for k in ["cover_path", "audio_path", "audio_paths", "subtitle"] {
            assert!(!m.contains_key(k), "{k}");
        }
        assert_eq!(m["added_ms"], json!(1730000000000_i64));
        assert!(m["audio"].get("transcript_job").is_none());
        assert!(m["audio"]["tracks"][0].get("transcript_job").is_none());
        assert!(m["audio"]["tracks"][0].get("transcript_jobs").is_none());
        // Ein so geschriebenes Objekt liest der Desktop wieder ein.
        let back: LibraryBook = serde_json::from_value(v).unwrap();
        assert_eq!(back.title, "Effi Briest");
    }

    #[test]
    fn buch_von_swift_ist_lesbar() {
        // So schreibt SyncedBook.json() (sortiert, ohne nil-Felder).
        let raw = json!({
            "added_ms": 1730000000000_i64, "authors": ["Theodor Fontane"],
            "audio": {"file": "teil_001.m4b", "sync": [{"chars": 0, "ms": 0}], "tracks": []},
            "cover_source": "generated", "files": {"audio": [{"blob": "a".repeat(64), "chunks": 1, "file": "teil_001.m4b", "size": 10}]},
            "finished": false, "id": "book_1", "title": "Effi", "updated": 1730000000500_i64
        });
        let files: BookFiles = serde_json::from_value(raw["files"].clone()).unwrap();
        assert!(files.audio[0].valid());
        let mut core = raw.clone();
        core.as_object_mut().unwrap().remove("files");
        let book: LibraryBook = serde_json::from_value(core).unwrap();
        assert_eq!(book.audio.unwrap().file, "teil_001.m4b");
    }

    #[test]
    fn alt_blob_als_ein_stueck_bleibt_gueltig() {
        let alt = BlobRef {
            blob: "0".repeat(64),
            size: blobs::LEGACY_ONE_CHUNK_MAX,
            chunks: 1,
            kind: None,
            file: None,
        };
        assert!(alt.valid());
        assert!(BlobRef {
            size: blobs::ONE_CHUNK_MAX + 1,
            ..alt.clone()
        }
        .valid());
        assert!(!BlobRef {
            size: blobs::LEGACY_ONE_CHUNK_MAX + 1,
            ..alt
        }
        .valid());
    }

    #[test]
    fn verweise_pruefen() {
        let ok = BlobRef {
            blob: "0".repeat(64),
            size: 5,
            chunks: 1,
            kind: None,
            file: None,
        };
        assert!(ok.valid());
        assert!(
            !BlobRef {
                chunks: 2,
                ..ok.clone()
            }
            .valid(),
            "Stückzahl passt nicht"
        );
        assert!(!BlobRef {
            blob: "../x".into(),
            ..ok.clone()
        }
        .valid());
        assert!(
            !BlobRef {
                blob: "A".repeat(64),
                ..ok
            }
            .valid(),
            "nur Kleinbuchstaben"
        );
    }

    #[test]
    fn fortschritt_zusammenfuehren() {
        let local = json!({"offset": 100, "length": 1000, "heard": [[0, 100]], "read": [], "updated": 5, "unit": "ms"});
        let remote = json!({"offset": 400, "length": 1200, "heard": [[50, 400]], "read": [[600, 700]], "updated": 9, "finished": true});
        let m = merge_record(&local, &remote).unwrap();
        assert_eq!(m["offset"], json!(400), "Stelle vom neueren");
        assert_eq!(m["heard"], json!([[0, 400]]));
        assert_eq!(m["read"], json!([[600, 700]]));
        assert_eq!(m["length"], json!(1200));
        assert_eq!(m["finished"], json!(true));
        assert_eq!(m["unit"], json!("ms"), "Felder des älteren bleiben");
        // Gleichstand: lokal gewinnt.
        let tie = merge_record(
            &json!({"offset": 1, "updated": 3}),
            &json!({"offset": 2, "updated": 3}),
        )
        .unwrap();
        assert_eq!(tie["offset"], json!(1));
        assert!(merge_record(&json!({"x": 1}), &remote).is_none());
    }

    #[test]
    fn lesezeichen_vereinigen() {
        let mine = r#"[{"id":"a","offset":1},{"id":"b","offset":2}]"#;
        let theirs = r#"[{"id":"b","offset":9},{"id":"c","offset":3}]"#;
        let m: Vec<Value> = serde_json::from_str(&merge_bookmarks(mine, theirs).unwrap()).unwrap();
        let ids: Vec<&str> = m.iter().map(|b| b["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(m[1]["offset"], json!(2), "lokale Fassung bleibt");
        assert!(
            merge_bookmarks("42", theirs).is_none(),
            "Weiterlese-Stelle ist keine Liste"
        );
    }

    #[test]
    fn datensatz_wie_das_iphone_ihn_liest() {
        assert!(valid_record(&json!({"offset": 1})));
        assert!(valid_record(
            &json!({"offset": 1.5, "length": 2, "anchor": "x", "finished": true})
        ));
        assert!(!valid_record(&json!({"length": 2})), "offset fehlt");
        assert!(!valid_record(&json!({"offset": "1"})));
        assert!(!valid_record(&json!({"offset": 1, "anchor": 3})));
        assert!(!valid_record(&json!([1])));
    }

    #[test]
    fn bereiche() {
        assert_eq!(
            merge_ranges(vec![
                (5.0, 7.0),
                (0.0, 2.0),
                (2.0, 3.0),
                (6.0, 9.0),
                (4.0, 4.0)
            ]),
            vec![(0.0, 3.0), (5.0, 9.0)]
        );
    }

    #[test]
    fn fortschritt_und_lesezeichen_sammeln() {
        let mut map = BTreeMap::new();
        map.insert(
            "progress.audio:book_1".into(),
            r#"{"offset":5,"length":0,"heard":[],"read":[],"updated":1}"#.into(),
        );
        map.insert(
            "tts.bookmarks.page_1".into(),
            r#"[{"id":"a","offset":3,"anchor":"x","created":1}]"#.into(),
        );
        map.insert("tts.resume.page_1".into(), "42".into());
        map.insert("anderes".into(), "1".into());
        map.insert("progress.kaputt".into(), "{".into());
        let objs = progress_objects(&map);
        assert_eq!(objs.len(), 3);
        let p = objs
            .iter()
            .find(|o| o.collection == COLLECTION_PROGRESS)
            .unwrap();
        assert_eq!(p.id, progress_object_id("audio:book_1"));
        assert_eq!(p.value["key"], json!("audio:book_1"));
        assert_eq!(p.value["record"]["offset"], json!(5));
        let b = objs
            .iter()
            .find(|o| o.id == bookmark_object_id("tts.bookmarks.page_1"))
            .unwrap();
        assert_eq!(b.value["key"], json!("tts.bookmarks.page_1"));
        assert!(b.value["value"].as_str().unwrap().starts_with("[{"));
    }
}
