//! Bibliothek im Geräte-Sync (lva#210 P5, Spec
//! `docs/superpowers/specs/2026-11-02-bibliothek-sync.md`).
//!
//! Drei Sammlungen:
//! - `book`: `buch.json` ohne lokale Felder, dazu `files` (Verweise in die
//!   Dateiablage) und `updated` (ms);
//! - `progress`: ein Fortschritts-Datensatz (`progress.<schlüssel>` aus
//!   `fortschritt.json`), Kennung `p_` + 40 Hex von SHA-256(schlüssel);
//! - `bookmark`: Lesezeichen und Weiterlese-Stellen (`tts.bookmarks.*`,
//!   `tts.resume.*`) sowie die Stimmen je Buchsprache (`tts.voice.*`,
//!   lva#352), Kennung `b_` + 40 Hex von SHA-256(schlüssel).
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
const BOOKMARK_PREFIXES: [&str; 3] = ["tts.bookmarks.", "tts.resume.", VOICE_PREFIX];
/// Stimme je Buchsprache (lva#352): `tts.voice.lang.<sprache>`,
/// `tts.voice.book.<buch>.<sprache>`; Wert JSON mit `updated` (ms).
const VOICE_PREFIX: &str = "tts.voice.";
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
    /// Weitere Sprachfassungen (lva#344): Text und Hörbuch je Fassungskennung.
    /// Altgeräte ignorieren das Feld.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub editions: BTreeMap<String, EditionFiles>,
    /// Kurzbücher, je Sprache das neueste (`kurzbuch-<sprache>.md`). Andere Geräte lesen
    /// sie; erzeugt werden sie aus den KI-Dokumente eines PCs, geladen wird hier nichts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kurzbuecher: Vec<KurzbuchRef>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct EditionFiles {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<BlobRef>,
    /// Eigenes Cover der Fassung (`covers/<fassung>.*`; bei der Hauptfassung `cover.*`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<BlobRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio: Vec<BlobRef>,
}

/// Datei eines Kurzbuchs samt Herkunft für die Anzeige.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct KurzbuchRef {
    #[serde(flatten)]
    pub blob: BlobRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_ms: Option<f64>,
    /// Quellenumfang, z. B. „Ganzes Buch“.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Sprachcode wie `de` oder `pt-br`; er steht im Dateinamen und in Pfaden der Clients.
fn valid_language(code: &str) -> bool {
    let mut parts = code.split('-');
    let main = parts.next().unwrap_or("");
    let region = parts.next();
    (2..=3).contains(&main.len())
        && main.bytes().all(|b| b.is_ascii_lowercase())
        && region.is_none_or(|r| {
            (2..=8).contains(&r.len())
                && r.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
        && parts.next().is_none()
}

/// Je Sprache das zuletzt erstellte Kurzbuch, egal von welchem Gerät. Bei gleichem
/// Datum entscheiden Blob-Kennung, Modell und Quelle: Alle Geräte wählen dasselbe,
/// gleich in welcher Reihenfolge die Stände ankommen.
fn merge_kurzbuecher(own: &[KurzbuchRef], received: &[KurzbuchRef]) -> Vec<KurzbuchRef> {
    fn order(a: &KurzbuchRef, b: &KurzbuchRef) -> std::cmp::Ordering {
        // Ohne Datum gilt als älter als jedes Datum.
        let date = match (a.created_ms, b.created_ms) {
            (Some(x), Some(y)) => x.total_cmp(&y),
            (x, y) => x.is_some().cmp(&y.is_some()),
        };
        date.then_with(|| a.blob.blob.cmp(&b.blob.blob))
            .then_with(|| a.blob.size.cmp(&b.blob.size))
            .then_with(|| a.blob.chunks.cmp(&b.blob.chunks))
            .then_with(|| a.blob.kind.cmp(&b.blob.kind))
            .then_with(|| a.model.cmp(&b.model))
            .then_with(|| a.source.cmp(&b.source))
    }
    let mut best: BTreeMap<String, KurzbuchRef> = BTreeMap::new();
    for k in own.iter().chain(received) {
        let Some(language) = kurzbuch_language(k.blob.file.as_deref()) else {
            continue;
        };
        let better = best
            .get(language)
            .is_none_or(|known| order(k, known).is_gt());
        if better {
            best.insert(language.to_string(), k.clone());
        }
    }
    best.into_values().collect()
}

fn received_kurzbuecher(files: &BookFiles) -> Vec<KurzbuchRef> {
    files
        .kurzbuecher
        .iter()
        .filter(|k| kurzbuch_language(k.blob.file.as_deref()).is_some())
        .cloned()
        .collect()
}

fn kurzbuch_language(file: Option<&str>) -> Option<&str> {
    file?
        .strip_prefix("kurzbuch-")?
        .strip_suffix(".md")
        .filter(|l| valid_language(l))
}

impl BookFiles {
    pub fn refs(&self) -> impl Iterator<Item = &BlobRef> {
        self.text
            .iter()
            .chain(self.cover.iter())
            .chain(self.audio.iter())
            .chain(
                self.editions
                    .values()
                    .flat_map(|e| e.text.iter().chain(e.cover.iter()).chain(e.audio.iter())),
            )
            .chain(self.kurzbuecher.iter().map(|k| &k.blob))
    }

    fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.cover.is_none()
            && self.audio.is_empty()
            && self
                .editions
                .values()
                .all(|e| e.text.is_none() && e.cover.is_none() && e.audio.is_empty())
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
/// Gerätelokale Aufträge aus einer Audiofassung entfernen.
fn strip_jobs(audio: &mut Map<String, Value>) {
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

pub fn book_core(book: &LibraryBook) -> Value {
    let mut v = serde_json::to_value(book).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        for k in ["cover_path", "audio_path", "audio_paths"] {
            m.remove(k);
        }
        if let Some(Value::Object(audio)) = m.get_mut("audio") {
            strip_jobs(audio);
        }
        if let Some(Value::Array(eds)) = m.get_mut("editions") {
            for e in eds.iter_mut() {
                if let Value::Object(e) = e {
                    e.remove("audio_paths");
                    if let Some(Value::Object(audio)) = e.get_mut("audio") {
                        strip_jobs(audio);
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

struct KurzbuchDoc {
    language: String,
    text: String,
    model: String,
    created_ms: f64,
    source: String,
}

/// Neuestes Kurzbuch je Sprache einer Vorlesen-Seite, nach Sprache sortiert.
fn kurzbuecher(app: &AppHandle, page_id: &str) -> Vec<KurzbuchDoc> {
    let Some(dir) = crate::commands::pages::page_path(app, page_id)
        .ok()
        .filter(|d| d.is_dir())
    else {
        return Vec::new();
    };
    let docs: Vec<_> = crate::book_ai::store::list(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.template_id == crate::book_ai::kurzbuch::TEMPLATE_ID)
        .collect();
    let keys = docs
        .iter()
        .map(|d| (d.language.as_deref(), d.modified_ms, d.id.as_str()));
    newest_per_language(keys)
        .into_iter()
        .filter_map(|(language, id)| {
            let doc = docs.iter().find(|d| d.id == id)?;
            let text = crate::book_ai::store::read_text(&dir, &id).ok()?;
            (!text.trim().is_empty()).then(|| KurzbuchDoc {
                language,
                text,
                model: doc.model.clone(),
                created_ms: doc.created_ms,
                source: doc.source_label.clone(),
            })
        })
        .collect()
}

/// Sprache → Dokument. Ohne gültige Angabe gilt Deutsch wie bei älteren Kurzbüchern,
/// bei gleichem Zeitstempel entscheidet die Kennung, damit jeder Lauf gleich ausgeht.
fn newest_per_language<'a>(
    docs: impl Iterator<Item = (Option<&'a str>, f64, &'a str)>,
) -> Vec<(String, String)> {
    let mut newest: BTreeMap<String, (f64, &str)> = BTreeMap::new();
    for (language, modified, id) in docs {
        let language = language.filter(|l| valid_language(l)).unwrap_or("de");
        let better = newest
            .get(language)
            .is_none_or(|(ms, known)| modified > *ms || (modified == *ms && id > *known));
        if better {
            newest.insert(language.to_string(), (modified, id));
        }
    }
    newest
        .into_iter()
        .map(|(l, (_, id))| (l, id.to_string()))
        .collect()
}

fn build_book(
    host: &dyn Host,
    key: &[u8; KEY_LEN],
    dir: &Path,
    book: &LibraryBook,
    side: &Sidecar,
    cache: &mut BTreeMap<String, FileStamp>,
    budget: &mut u64,
) -> Result<BookBuild, String> {
    // Altbestand (flach) gehört mit Fassungen zu `ed_<buch>`, auch wenn
    // `wanted` noch nicht lief – sonst ginge er beim Versand verloren.
    let mut upgraded = side.clone();
    let side = if upgrade_pending(&mut upgraded.pending, book) {
        &upgraded
    } else {
        side
    };
    let mut files = BookFiles::default();
    let mut uploads = Vec::new();
    let mut ready = true;

    // Text: die Vorlesen-Seite ist die Quelle; fehlt sie, der gemeldete Verweis.
    let text = book
        .text
        .as_ref()
        .filter(|t| !t.page_id.is_empty())
        .and_then(|t| host.text(&t.page_id))
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
        None => {
            files.text = side
                .pending
                .text
                .clone()
                .filter(|_| book.editions.is_empty())
                .or_else(|| {
                    // Mit Fassungen steht der ausstehende Text unter der Kennung.
                    library::editions::primary_id(book)
                        .and_then(|p| side.pending.editions.get(p))
                        .and_then(|ef| ef.text.clone())
                })
                .filter(|_| book.text.is_some())
        }
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
        None => {
            // Mit Fassungen steht das ausstehende Cover unter der Kennung.
            files.cover = side
                .pending
                .cover
                .clone()
                .filter(|_| book.editions.is_empty())
                .or_else(|| {
                    library::editions::primary_id(book)
                        .and_then(|p| side.pending.editions.get(p))
                        .and_then(|ef| ef.cover.clone())
                })
        }
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
    // Weitere Sprachfassungen: Text der eigenen Seite, Hörbuchteile flach.
    let primary = library::editions::primary_id(book).map(str::to_string);
    for e in book
        .editions
        .iter()
        .filter(|e| Some(&e.id) != primary.as_ref())
    {
        let mut ef = EditionFiles::default();
        let pending = side.pending.editions.get(&e.id);
        let text = e
            .text
            .as_ref()
            .filter(|t| !t.page_id.is_empty())
            .and_then(|t| host.text(&t.page_id))
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
                    ef.text = Some(r);
                }
            }
            None => {
                ef.text = pending
                    .and_then(|p| p.text.clone())
                    .filter(|_| e.text.is_some())
            }
        }
        if let Some(audio) = &e.audio {
            for track in library::tracks_of(audio) {
                let path = dir.join("audio").join(&track.file);
                if path.is_file() {
                    match blobs::stamp(cache, &path, budget)? {
                        Some(st) => {
                            if let Some(mut r) = BlobRef::new(key, &st) {
                                r.file = Some(track.file.clone());
                                uploads.push(PendingUpload {
                                    blob: r.clone(),
                                    source: Source::File(path),
                                    stamp: Some(st),
                                });
                                ef.audio.push(r);
                            }
                        }
                        None => ready = false,
                    }
                } else if let Some(r) = pending.and_then(|p| {
                    p.audio
                        .iter()
                        .find(|r| r.file.as_deref() == Some(track.file.as_str()))
                }) {
                    ef.audio.push(r.clone());
                }
            }
        }
        match library::editions::edition_cover(dir, &e.id) {
            Some(path) => match blobs::stamp(cache, &path, budget)? {
                Some(st) => {
                    if let Some(mut r) = BlobRef::new(key, &st) {
                        r.kind = path.extension().map(|x| x.to_string_lossy().to_lowercase());
                        uploads.push(PendingUpload {
                            blob: r.clone(),
                            source: Source::File(path),
                            stamp: Some(st),
                        });
                        ef.cover = Some(r);
                    }
                }
                None => ready = false,
            },
            None => ef.cover = pending.and_then(|p| p.cover.clone()),
        }
        if ef.text.is_some() || ef.cover.is_some() || !ef.audio.is_empty() {
            files.editions.insert(e.id.clone(), ef);
        }
    }
    for doc in book
        .text
        .as_ref()
        .map(|t| host.kurzbuecher(&t.page_id))
        .unwrap_or_default()
    {
        let bytes = doc.text.into_bytes();
        if let Some(mut r) = BlobRef::new(key, &blobs::stamp_bytes(&bytes)) {
            r.kind = Some("md".into());
            r.file = Some(format!("kurzbuch-{}.md", doc.language));
            uploads.push(PendingUpload {
                blob: r.clone(),
                source: Source::Bytes(bytes),
                stamp: None,
            });
            files.kurzbuecher.push(KurzbuchRef {
                blob: r,
                model: Some(doc.model),
                created_ms: Some(doc.created_ms),
                source: Some(doc.source),
            });
        }
    }
    // Ein PC ohne diese KI-Dokumente gibt empfangene Kurzbücher unverändert weiter.
    files.kurzbuecher = merge_kurzbuecher(&files.kurzbuecher, &side.pending.kurzbuecher);
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
    paths.extend(
        library::editions::all_tracks(book)
            .iter()
            .map(|t| dir.join("audio").join(&t.file))
            .filter(|p| p.is_file()),
    );
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
        // Altbestand (flach) vor dem Versand den Fassungen zuordnen.
        let mut side = read_sidecar_for(&dir, Some(&book));
        // Nur noch der Zwischenspeicher: wer jetzt ungeprüft ist, wartet.
        let mut no_budget = 0;
        let host = AppHost {
            app,
            device_name: "",
        };
        let built = match build_book(&host, key, &dir, &book, &side, cache, &mut no_budget) {
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
            book_hash_locked(
                &AppHost {
                    app,
                    device_name: "",
                },
                key,
                &dir,
                cache,
            )
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
    host: &dyn Host,
    key: &[u8; KEY_LEN],
    dir: &Path,
    cache: &mut BTreeMap<String, FileStamp>,
) -> Result<Option<String>, String> {
    let Some(book) = library::sync_read_book(dir) else {
        return Ok(None);
    };
    let mut side = read_sidecar_for(dir, Some(&book));
    let mut budget = u64::MAX;
    let built = build_book(host, key, dir, &book, &side, cache, &mut budget)?;
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
    let host = AppHost {
        app,
        device_name: "",
    };
    apply_book_on(&host, key, id, value, synced_hash, cache)
}

fn apply_book_on(
    host: &dyn Host,
    key: &[u8; KEY_LEN],
    id: &str,
    value: Option<&Value>,
    synced_hash: Option<&str>,
    cache: &mut BTreeMap<String, FileStamp>,
) -> Result<Applied, ApplyError> {
    let root = host.root()?;
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
            if library::editions::all_tracks(&incoming)
                .iter()
                .any(|t| !library::sync_plain_name(&t.file))
            {
                return Err(invalid("Ungültiger Dateiname im Hörbuch"));
            }
            if incoming
                .editions
                .iter()
                .any(|e| !library::editions::valid_edition_id(&e.id))
                || remote_files
                    .editions
                    .keys()
                    .any(|k| !library::editions::valid_edition_id(k))
            {
                return Err(invalid("Ungültige Kennung einer Sprachfassung"));
            }
            for e in incoming.editions.iter_mut() {
                e.audio_paths = Vec::new();
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
        book_hash_locked(host, key, &dir, cache)?
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

    // Löschkaskade (lva#365 P2b-2): Liegt das Buch hier im Papierkorb und kommt vom
    // Server wieder (anderes Gerät hat es wiederhergestellt), kehrt der Papierkorb-Ordner
    // samt Fassungen zurück, statt daneben einen leeren Ordner anzulegen; die Seiten
    // des Buchs sind damit wieder auffindbar. Danach wird wie gewohnt angeglichen.
    if !dir.exists() {
        let korb = root.join(TRASH).join(id);
        if crate::library_layout::loeschung_offen(&korb)? {
            // Endgültige Löschung war unterbrochen: erst abschließen, nichts
            // Halbgelöschtes zurückholen. Danach entsteht das Buch neu vom Server.
            let _pages = crate::commands::pages::lock();
            if let Some(data) = root.parent() {
                let r = crate::library_layout::buch_endgueltig_loeschen(data, &korb);
                crate::commands::library_layout::fundortkarte_neu(data);
                r.map_err(|e| format!("Unterbrochenes Löschen nicht abschließbar: {e}"))?;
            }
        } else if korb.join("buch.json").is_file() {
            // Umbenennen und Fundortkarte gemeinsam unter Bibliotheks- (oben genommen)
            // und Seitensperre: kein Leser sieht die Fassungen
            // zurück, aber die Karte noch alt. Die Karte wird auch nach einem Fehler neu.
            let _pages = crate::commands::pages::lock();
            let zurueck = std::fs::rename(&korb, &dir);
            if zurueck.is_ok() {
                let _ = std::fs::remove_file(dir.join(".entfernt"));
            }
            if let Some(data) = root.parent() {
                crate::commands::library_layout::fundortkarte_neu(data);
            }
            zurueck.map_err(|e| format!("Buch aus dem Papierkorb zurückholen: {e}"))?;
        }
    }

    let existing = library::sync_read_book(&dir);
    let mut side = read_sidecar_for(&dir, existing.as_ref());
    // Die flachen Dateiverweise (`files.text/audio/cover`) gehören zur
    // Hauptfassung des Senders, nicht zwingend zur Hauptfassung nach dem Merge.
    let sender_primary = library::editions::sender_primary(existing.as_ref(), &incoming);
    let local_primary = existing
        .as_ref()
        .and_then(|b| library::editions::primary_id(b).map(str::to_string));
    if let (Some(local), false, true) = (&existing, local_clean, side.updated > remote_updated) {
        // Lokaler Stand ist neuer und ungesichert: Metadaten bleiben lokal,
        // der Fassungsteil wird trotzdem zusammengeführt (§5.1) und mit dem
        // nächsten Push verteilt. Empfangene Kurzbücher gelten trotzdem.
        side.pending.kurzbuecher = merge_kurzbuecher(
            &side.pending.kurzbuecher,
            &received_kurzbuecher(&remote_files),
        );
        if incoming.editions.is_empty() {
            write_sidecar(&dir, &side)?;
            return Ok(Applied::default());
        }
        let mut merged = local.clone();
        let mut theirs = incoming.clone();
        let lift = library::editions::merge_incoming(Some(local), &mut theirs);
        library::editions::apply_state(&mut merged, library::editions::state_of(&theirs));
        carry_jobs(Some(local), &mut merged);
        if &merged == local && lift.is_none() {
            write_sidecar(&dir, &side)?;
            return Ok(Applied::default());
        }
        let moved = follow_primary_cover(&dir, local_primary.as_deref(), &merged)?;
        write_with_covers(&dir, &merged, moved)?;
        lift_progress_locked(&root, lift)?;
        // Dateien neuer Fassungen trotzdem holen; `updated` bleibt lokal.
        let kurzbuecher = std::mem::take(&mut side.pending.kurzbuecher);
        side.pending = pending_files(
            &|id| host.text(id).is_some(),
            key,
            &dir,
            cache,
            &merged,
            &remote_files,
            sender_primary.as_deref(),
        );
        side.pending.kurzbuecher = kurzbuecher;
        write_sidecar(&dir, &side)?;
        return Ok(Applied {
            changed: true,
            remote_blobs: remote_files.refs().map(|r| r.blob.clone()).collect(),
            ..Default::default()
        });
    }
    let lift = library::editions::merge_incoming(existing.as_ref(), &mut incoming);
    // Gerätelokale Aufträge erst nach dem Merge zurück (Fassung und
    // Arbeitskopie gleich), sonst sähe der Merge eine Änderung.
    carry_jobs(existing.as_ref(), &mut incoming);
    let moved = follow_primary_cover(&dir, local_primary.as_deref(), &incoming)?;
    write_with_covers(&dir, &incoming, moved)?;
    lift_progress_locked(&root, lift)?;
    let mut pending = pending_files(
        &|id| host.text(id).is_some(),
        key,
        &dir,
        cache,
        &incoming,
        &remote_files,
        sender_primary.as_deref(),
    );
    // Nur gültige Verweise weitergeben; sie stehen in Pfaden der anderen Geräte.
    pending.kurzbuecher = merge_kurzbuecher(
        &side.pending.kurzbuecher,
        &received_kurzbuecher(&remote_files),
    );
    side.pending = pending;
    side.updated = remote_updated;
    let book = library::sync_read_book(&dir).ok_or("Buch nach dem Schreiben unlesbar")?;
    let mut budget = u64::MAX;
    let built = build_book(host, key, &dir, &book, &side, cache, &mut budget)?;
    side.hash = collect::hash(&built.core_files);
    write_sidecar(&dir, &side)?;
    Ok(Applied {
        changed: true,
        remote_blobs: remote_files.refs().map(|r| r.blob.clone()).collect(),
        ..Default::default()
    })
}

/// Gerätelokale Transkriptionsaufträge aus `old` übernehmen, in jeder Fassung
/// und in der Arbeitskopie gleich (sie reisen nicht mit dem Sync).
fn carry_jobs(old: Option<&LibraryBook>, book: &mut LibraryBook) {
    let Some(old) = old else {
        return;
    };
    fn carry(from: Option<&library::AudioVersion>, to: Option<&mut library::AudioVersion>) {
        let (Some(o), Some(n)) = (from, to) else {
            return;
        };
        if o.file == n.file {
            n.transcript_job = o.transcript_job.clone();
        }
        for t in n.tracks.iter_mut() {
            if let Some(x) = o.tracks.iter().find(|x| x.file == t.file) {
                t.transcript_job = x.transcript_job.clone();
                t.transcript_jobs = x.transcript_jobs.clone();
            }
        }
    }
    for e in book.editions.iter_mut() {
        let prev = old.editions.iter().find(|o| o.id == e.id);
        carry(prev.and_then(|p| p.audio.as_ref()), e.audio.as_mut());
    }
    if old.editions.is_empty() {
        // Schema 1 lokal: die Arbeitskopie ist die einzige Fassung.
        let pid = library::editions::primary_id(book).map(str::to_string);
        if let Some(e) = book
            .editions
            .iter_mut()
            .find(|e| Some(&e.id) == pid.as_ref())
        {
            carry(old.audio.as_ref(), e.audio.as_mut());
        }
    }
    let pid = library::editions::primary_id(book).map(str::to_string);
    if let Some(e) = book.editions.iter().find(|e| Some(&e.id) == pid.as_ref()) {
        book.audio = e.audio.clone();
    } else {
        carry(old.audio.as_ref(), book.audio.as_mut());
    }
}

/// Wechselt die Hauptfassung durch den Empfang, wandert das Cover mit
/// (`cover.*` ↔ `covers/<fassung>.*`), wie beim Wechsel am Gerät.
fn follow_primary_cover(
    dir: &Path,
    before: Option<&str>,
    after: &LibraryBook,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut moved = Vec::new();
    if let (Some(old), Some(new)) = (before, library::editions::primary_id(after)) {
        library::editions::swap_covers(dir, old, new, &mut moved)?;
    }
    Ok(moved)
}

/// Buch schreiben; scheitert es, geht der Cover-Tausch zurück (Dateien und
/// Register der Hauptfassung bleiben sich einig).
fn write_with_covers(
    dir: &Path,
    book: &LibraryBook,
    moved: Vec<(PathBuf, PathBuf)>,
) -> Result<(), String> {
    library::sync_write_book(dir, book).inspect_err(|_| library::editions::undo_moves(&moved))
}

/// Was fehlt hier noch? Nur das kommt nach `pending`.
fn pending_files(
    page_exists: &dyn Fn(&str) -> bool,
    key: &[u8; KEY_LEN],
    dir: &Path,
    cache: &mut BTreeMap<String, FileStamp>,
    book: &LibraryBook,
    remote_files: &BookFiles,
    sender_primary: Option<&str>,
) -> BookFiles {
    fn stale(
        key: &[u8; KEY_LEN],
        dir: &Path,
        cache: &mut BTreeMap<String, FileStamp>,
        r: &BlobRef,
    ) -> bool {
        let Some(name) = r.file.as_deref().filter(|n| library::sync_plain_name(n)) else {
            return false;
        };
        let path = dir.join("audio").join(name);
        let mut b = u64::MAX;
        let local = path
            .is_file()
            .then(|| blobs::stamp(cache, &path, &mut b).ok().flatten())
            .flatten()
            .and_then(|s| blobs::blob_id_of(key, &s));
        local.as_deref() != Some(r.blob.as_str())
    }
    let has_page = |t: Option<&library::TextVersion>| t.is_some_and(|t| page_exists(&t.page_id));
    let mut pending = BookFiles::default();
    let primary_now = library::editions::primary_id(book);
    let same_primary =
        book.editions.is_empty() || sender_primary.is_none() || primary_now == sender_primary;
    if same_primary && !has_page(book.text.as_ref()) {
        pending.text = remote_files.text.clone();
    }
    if !same_primary {
        // Text der Sender-Hauptfassung gehört zu dieser Fassung, nicht in die
        // hiesige Arbeitskopie.
        if let Some(sp) = sender_primary {
            if let Some(e) = book.editions.iter().find(|e| e.id == sp) {
                if !has_page(e.text.as_ref()) && remote_files.text.is_some() {
                    pending.editions.entry(sp.to_string()).or_default().text =
                        remote_files.text.clone();
                }
            }
        }
    }
    let wants = cover_wants(key, dir, cache, book, remote_files, sender_primary);
    for r in &remote_files.audio {
        if stale(key, dir, cache, r) {
            pending.audio.push(r.clone());
        }
    }
    for (eid, ef) in &remote_files.editions {
        let Some(e) = book.editions.iter().find(|e| &e.id == eid) else {
            continue;
        };
        let mut pe = pending.editions.remove(eid).unwrap_or_default();
        if pe.text.is_none() && !has_page(e.text.as_ref()) {
            pe.text = ef.text.clone();
        }
        for r in &ef.audio {
            if stale(key, dir, cache, r) {
                pe.audio.push(r.clone());
            }
        }
        if pe.text.is_some() || pe.cover.is_some() || !pe.audio.is_empty() {
            pending.editions.insert(eid.clone(), pe);
        }
    }
    add_cover_wants(&mut pending, book, wants);
    // Mit Fassungen steht auch der Text der Hauptfassung unter ihrer
    // Kennung: welche Seite er füllt, entscheidet erst der Abschluss.
    if let (Some(t), Some(pid)) = (
        pending.text.take_if(|_| !book.editions.is_empty()),
        library::editions::primary_id(book),
    ) {
        let ef = pending.editions.entry(pid.to_string()).or_default();
        if ef.text.is_none() {
            ef.text = Some(t);
        }
    }
    pending
}

/// Altbestand (lva#344): flache `pending.text/cover` stammen aus der Zeit vor
/// den Fassungen bzw. von einem Schema-1-Stand. Sie gehören zur Arbeitskopie
/// von damals, und die wird deterministisch zur Fassung `ed_<buch>`. Hat das
/// Buch Fassungen, kommen sie dorthin (oder entfallen, wenn es sie nicht mehr
/// gibt) – nie in die gerade gültige Hauptfassung. Liefert, ob sich etwas
/// geändert hat.
fn upgrade_pending(pending: &mut BookFiles, book: &LibraryBook) -> bool {
    if book.editions.is_empty() || (pending.text.is_none() && pending.cover.is_none()) {
        return false;
    }
    let mid = library::editions::migrated_id(&book.id);
    let (text, cover) = (pending.text.take(), pending.cover.take());
    if book.editions.iter().any(|e| e.id == mid) {
        let ef = pending.editions.entry(mid).or_default();
        if ef.text.is_none() {
            ef.text = text;
        }
        if ef.cover.is_none() {
            ef.cover = cover;
        }
    }
    true
}

/// `sync.json` lesen, Altbestand nach [`upgrade_pending`] zugeordnet.
fn read_sidecar_for(dir: &Path, book: Option<&LibraryBook>) -> Sidecar {
    let mut side = read_sidecar(dir);
    if let Some(b) = book {
        upgrade_pending(&mut side.pending, b);
    }
    side
}

/// Ausstehende Cover eintragen. Mit Fassungen immer mit Kennung: welche Datei
/// (`cover.*` oder `covers/<ed>.*`) es wird, entscheidet erst der Abschluss
/// des Downloads nach der dann gültigen Hauptfassung.
fn add_cover_wants(
    pending: &mut BookFiles,
    book: &LibraryBook,
    (primary, others): (Option<BlobRef>, BTreeMap<String, BlobRef>),
) {
    match library::editions::primary_id(book) {
        Some(pid) if !book.editions.is_empty() => {
            if let Some(r) = primary {
                pending.editions.entry(pid.to_string()).or_default().cover = Some(r);
            }
        }
        _ => pending.cover = primary,
    }
    for (eid, r) in others {
        pending.editions.entry(eid).or_default().cover = Some(r);
    }
}

/// Welche Cover fehlen hier? Das flache `files.cover` gehört zur Hauptfassung
/// des Senders, `files.editions[ed].cover` zur Fassung `ed`. Abgelegt wird je
/// nach hiesiger Hauptfassung: sie nach `cover.*`, alle anderen nach
/// `covers/<ed>.*`. Liefert (Cover der Hauptfassung, Cover weiterer Fassungen).
fn cover_wants(
    key: &[u8; KEY_LEN],
    dir: &Path,
    cache: &mut BTreeMap<String, FileStamp>,
    book: &LibraryBook,
    remote_files: &BookFiles,
    sender_primary: Option<&str>,
) -> (Option<BlobRef>, BTreeMap<String, BlobRef>) {
    let primary_now = library::editions::primary_id(book);
    let mut received: Vec<(Option<&str>, &BlobRef)> = Vec::new();
    if let Some(r) = &remote_files.cover {
        received.push((sender_primary, r));
    }
    for (eid, ef) in &remote_files.editions {
        if let Some(r) = &ef.cover {
            received.push((Some(eid.as_str()), r));
        }
    }
    let mut same = |path: Option<PathBuf>, r: &BlobRef| -> bool {
        let mut b = u64::MAX;
        path.and_then(|p| blobs::stamp(cache, &p, &mut b).ok().flatten())
            .and_then(|st| blobs::blob_id_of(key, &st))
            .as_deref()
            == Some(r.blob.as_str())
    };
    let mut primary = None;
    let mut others = BTreeMap::new();
    for (eid, r) in received {
        let is_primary = book.editions.is_empty() || eid.is_none() || eid == primary_now;
        if is_primary {
            if !same(library::sync_cover_file(dir), r) {
                primary = Some(r.clone());
            }
        } else if let Some(eid) = eid.filter(|e| book.editions.iter().any(|x| x.id == *e)) {
            if !same(library::editions::edition_cover(dir, eid), r) {
                others.insert(eid.to_string(), r.clone());
            }
        }
    }
    (primary, others)
}

/// Geladenes Cover der Fassung `eid` ablegen: als `cover.*`, wenn sie jetzt
/// Hauptfassung ist, sonst als `covers/<eid>.*`. Erst ablegen, dann die
/// anderen Endungen entfernen. Gibt es die Fassung nicht mehr, entfällt es.
fn place_edition_cover(
    dir: &Path,
    book: &LibraryBook,
    eid: &str,
    tmp: &Path,
    ext: &str,
) -> Result<Option<PathBuf>, String> {
    let (folder, stem) = if library::editions::primary_id(book) == Some(eid) {
        (dir.to_path_buf(), "cover".to_string())
    } else if book.editions.iter().any(|e| e.id == eid) {
        let covers = dir.join("covers");
        std::fs::create_dir_all(&covers).map_err(|e| e.to_string())?;
        (covers, eid.to_string())
    } else {
        let _ = std::fs::remove_file(tmp);
        return Ok(None);
    };
    let target = folder.join(format!("{stem}.{ext}"));
    std::fs::rename(tmp, &target).map_err(|e| format!("Cover ablegen: {e}"))?;
    for other in ["jpg", "png", "webp"] {
        if other != ext {
            let _ = std::fs::remove_file(folder.join(format!("{stem}.{other}")));
        }
    }
    Ok(Some(target))
}

/// Gewonnene Auswahl (§5.2): Fortschritt der Zielfassung auf die Stelle heben.
fn lift_progress_locked(
    root: &Path,
    lift: Option<(library::ReadingPosition, f64)>,
) -> Result<(), String> {
    let Some((pos, at)) = lift else {
        return Ok(());
    };
    let mut map = library::sync_progress_load(root)?;
    let before = map.clone();
    library::editions::lift_progress(&mut map, &pos, at);
    if map != before {
        library::sync_progress_store(root, &map)?;
    }
    Ok(())
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

/// `updated` einer Stimmwahl (`tts.voice.*`); unlesbar zählt als 0.
fn voice_updated(raw: &str) -> f64 {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| v.get("updated").and_then(Value::as_f64))
        .unwrap_or(0.0)
}

/// Gewinnt die eingehende Stimmwahl gegen die lokale? Nur wenn sie neuer ist.
fn voice_remote_wins(local: &str, remote: &str) -> bool {
    voice_updated(remote) > voice_updated(local)
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
                // Stimmen: immer der neuere Stand, auch bei sauberem lokalem
                // Stand (eine spät eintreffende ältere Wahl gewinnt nie).
                Some(mine) if sub.starts_with(VOICE_PREFIX) => {
                    if voice_remote_wins(mine, raw) {
                        raw.to_string()
                    } else {
                        return Ok(Applied::default());
                    }
                }
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
    /// Text einer weiteren Sprachfassung (Kennung).
    EditionText(String),
    /// Cover einer Sprachfassung (Kennung); abgelegt je nach Hauptfassung.
    EditionCover(String),
}

/// Alles, was andere Geräte gemeldet haben und hier fehlt. Kleines zuerst:
/// Text und Cover vor Hörbuchteilen.
pub fn wanted(app: &AppHandle) -> Result<Vec<Wanted>, String> {
    let _guard = library::sync_lock();
    let root = library::sync_root(app)?;
    wanted_in(&root)
}

fn wanted_in(root: &Path) -> Result<Vec<Wanted>, String> {
    let root = root.to_path_buf();
    let mut out = Vec::new();
    for (id, dir) in book_dirs(&root)? {
        let mut side = read_sidecar(&dir);
        if side.pending.is_empty() {
            continue;
        }
        if let Some(book) = library::sync_read_book(&dir) {
            if upgrade_pending(&mut side.pending, &book) {
                write_sidecar(&dir, &side)?;
            }
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
        for (eid, ef) in &side.pending.editions {
            if let Some(t) = ef.text.as_ref().filter(|t| t.size <= MAX_TEXT_BYTES) {
                push(t, WantedKind::EditionText(eid.clone()));
            }
            if let Some(c) = &ef.cover {
                push(c, WantedKind::EditionCover(eid.clone()));
            }
            for a in &ef.audio {
                if let Some(f) = a.file.as_deref().filter(|f| library::sync_plain_name(f)) {
                    push(a, WantedKind::Audio(f.to_string()));
                }
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
        WantedKind::EditionText(e) => w.dir.join(format!("sync-text-{e}.download")),
        WantedKind::EditionCover(e) => w.dir.join(format!("cover-{e}.download")),
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
    finish_download_on(&mut AppHost { app, device_name }, w)
}

fn finish_download_on(host: &mut dyn Host, w: &Wanted) -> Result<Option<PathBuf>, String> {
    let tmp = download_target(w);
    let _guard = library::sync_lock();
    if !w.dir.join("buch.json").is_file() {
        let _ = std::fs::remove_file(&tmp);
        return Err("Buch verschwunden".into());
    }
    // Ein laufender Altbestand-Download (ohne Kennung) auf einem Buch mit
    // Fassungen gehört zu `ed_<buch>` (siehe upgrade_pending).
    if matches!(w.what, WantedKind::Text | WantedKind::Cover) {
        if let Some(book) = library::sync_read_book(&w.dir).filter(|b| !b.editions.is_empty()) {
            let mid = library::editions::migrated_id(&book.id);
            let (done, kind) = if w.what == WantedKind::Text {
                (
                    finish_edition_text(w, &mid, &tmp, host),
                    WantedKind::EditionText(mid),
                )
            } else {
                (
                    finish_edition_cover(w, &mid, &tmp),
                    WantedKind::EditionCover(mid),
                )
            };
            if done.is_ok() {
                clear_pending(&w.dir, &kind)?;
            }
            return done;
        }
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
                .filter(|p| host.valid(p))
                .unwrap_or_else(crate::commands::pages::fresh_id);
            if !host.exists(&page_id)? {
                host.create(
                    &page_id,
                    &json!({ "title": book.title, "state": { "text": text, "tab": "original" } }),
                )?;
            }
            if let Some(t) = book.text.as_mut() {
                if t.page_id != page_id {
                    t.page_id = page_id;
                    library::sync_write_book(&w.dir, &book)?;
                }
            }
            clear_pending(&w.dir, &w.what)?;
            // Neue Textseite direkt in den Buch-Ordner (lva#365 P2b-2).
            if let Some(root) = w.dir.parent() {
                library::seiten_einlagern(root, &book.id);
            }
            Ok(None)
        }
        WantedKind::EditionText(eid) => finish_edition_text(w, eid, &tmp, host),
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
        WantedKind::EditionCover(eid) => finish_edition_cover(w, eid, &tmp),
        WantedKind::Audio(f) => {
            let target = w.dir.join("audio").join(f);
            // Eine abweichende Datei gleichen Namens geht in den Papierkorb.
            if target.is_file() {
                let root = host.root()?;
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

/// Umgebung des Abgleichs (Bibliotheksordner, Seitenablage); im Test
/// ersetzbar, damit `apply_book`/`finish_download` echt laufen.
trait Host {
    fn root(&self) -> Result<PathBuf, String>;
    fn text(&self, page_id: &str) -> Option<String>;
    fn kurzbuecher(&self, _page_id: &str) -> Vec<KurzbuchDoc> {
        Vec::new()
    }
    fn valid(&self, page_id: &str) -> bool;
    fn exists(&self, page_id: &str) -> Result<bool, String>;
    fn create(&mut self, page_id: &str, page: &Value) -> Result<(), String>;
}

struct AppHost<'a> {
    app: &'a AppHandle,
    device_name: &'a str,
}

impl Host for AppHost<'_> {
    fn root(&self) -> Result<PathBuf, String> {
        library::sync_root(self.app)
    }
    fn text(&self, page_id: &str) -> Option<String> {
        page_text(self.app, page_id)
    }
    fn kurzbuecher(&self, page_id: &str) -> Vec<KurzbuchDoc> {
        kurzbuecher(self.app, page_id)
    }
    fn valid(&self, page_id: &str) -> bool {
        // Nur die Form der Kennung: eine Seite, die hier noch nicht existiert,
        // entsteht gleich unter genau dieser Kennung (lva#365: `page_path`
        // meldet seither „Seite nicht gefunden“).
        crate::commands::pages::valid_id(page_id)
    }
    fn exists(&self, page_id: &str) -> Result<bool, String> {
        Ok(collect::read_page(self.app, page_id)?.is_some())
    }
    fn create(&mut self, page_id: &str, page: &Value) -> Result<(), String> {
        collect::apply_page(self.app, page_id, page, None, self.device_name).map(|_| ())
    }
}

/// Abschluss eines Fassungstexts (Sperre hält der Aufrufer). Der Text gehört
/// zur Fassung `eid`, gleich welche gerade Hauptfassung ist; ist sie es,
/// führt die Arbeitskopie dieselbe Seite.
fn finish_edition_text(
    w: &Wanted,
    eid: &str,
    tmp: &Path,
    pages: &mut dyn Host,
) -> Result<Option<PathBuf>, String> {
    let bytes = std::fs::read(tmp).map_err(|e| format!("Buchtext: {e}"));
    let _ = std::fs::remove_file(tmp);
    let text = String::from_utf8_lossy(&bytes?).into_owned();
    let mut book = library::sync_read_book(&w.dir).ok_or("Buch unlesbar")?;
    let is_primary = library::editions::primary_id(&book) == Some(eid);
    let Some(e) = book.editions.iter_mut().find(|e| e.id == eid) else {
        clear_pending(&w.dir, &w.what)?;
        return Ok(None);
    };
    let page_id = e
        .text
        .as_ref()
        .map(|t| t.page_id.clone())
        .filter(|p| pages.valid(p))
        .unwrap_or_else(crate::commands::pages::fresh_id);
    if !pages.exists(&page_id)? {
        pages.create(
            &page_id,
            &json!({ "title": e.title, "state": { "text": text, "tab": "original", "lang": e.lang } }),
        )?;
    }
    let mut changed = false;
    if let Some(t) = e.text.as_mut() {
        if t.page_id != page_id {
            t.page_id = page_id;
            changed = true;
        }
    }
    if changed {
        if is_primary {
            book.text = e.text.clone();
        }
        library::sync_write_book(&w.dir, &book)?;
    }
    clear_pending(&w.dir, &w.what)?;
    // Neue Fassungsseite direkt in den Buch-Ordner (lva#365 P2b-2).
    if let Some(root) = w.dir.parent() {
        library::seiten_einlagern(root, &book.id);
    }
    Ok(None)
}

/// Abschluss eines Fassungs-Covers (Sperre hält der Aufrufer): erst jetzt
/// entscheidet die gültige Hauptfassung über den Ablageort.
fn finish_edition_cover(w: &Wanted, eid: &str, tmp: &Path) -> Result<Option<PathBuf>, String> {
    let book = library::sync_read_book(&w.dir).ok_or("Buch unlesbar")?;
    let ext = cover_ext(w.blob.kind.as_deref());
    let target = place_edition_cover(&w.dir, &book, eid, tmp, ext).inspect_err(|_| {
        let _ = std::fs::remove_file(tmp);
    })?;
    clear_pending(&w.dir, &w.what)?;
    Ok(target)
}

fn clear_pending(dir: &Path, what: &WantedKind) -> Result<(), String> {
    let mut side = read_sidecar(dir);
    match what {
        WantedKind::Text => side.pending.text = None,
        WantedKind::Cover => side.pending.cover = None,
        WantedKind::Audio(f) => {
            side.pending
                .audio
                .retain(|r| r.file.as_deref() != Some(f.as_str()));
            for ef in side.pending.editions.values_mut() {
                ef.audio.retain(|r| r.file.as_deref() != Some(f.as_str()));
            }
        }
        WantedKind::EditionText(e) => {
            if let Some(ef) = side.pending.editions.get_mut(e) {
                ef.text = None;
            }
        }
        WantedKind::EditionCover(e) => {
            if let Some(ef) = side.pending.editions.get_mut(e) {
                ef.cover = None;
            }
        }
    }
    side.pending
        .editions
        .retain(|_, ef| ef.text.is_some() || ef.cover.is_some() || !ef.audio.is_empty());
    write_sidecar(dir, &side)
}

#[cfg(test)]
mod tests {
    #[test]
    fn neuestes_kurzbuch_je_sprache() {
        let docs = [
            (Some("de"), 1.0, "a"),
            (Some("de"), 2.0, "b"),
            (Some("en"), 5.0, "c"),
            (None, 2.0, "d"),
            (Some("../x"), 1.0, "e"),
        ];
        assert_eq!(
            newest_per_language(docs.into_iter()),
            vec![
                ("de".to_string(), "d".to_string()),
                ("en".to_string(), "c".to_string())
            ],
            "gleiche Zeit: die Kennung entscheidet; ungültige Sprache gilt als Deutsch"
        );
    }

    #[test]
    fn kurzbuecher_zusammenfuehren_nach_datum() {
        let k = |blob: &str, file: &str, created: f64| KurzbuchRef {
            blob: BlobRef {
                blob: blob.into(),
                file: Some(file.into()),
                ..Default::default()
            },
            created_ms: Some(created),
            ..Default::default()
        };
        let own = [k("alt", "kurzbuch-de.md", 1.0)];
        let received = [
            k("neu", "kurzbuch-de.md", 2.0),
            k("en", "kurzbuch-en.md", 1.0),
            k("x", "../x", 9.0),
        ];
        let merged = merge_kurzbuecher(&own, &received);
        assert_eq!(
            merged
                .iter()
                .map(|m| m.blob.blob.as_str())
                .collect::<Vec<_>>(),
            ["neu", "en"],
            "neuere Fassung gewinnt, andere Sprachen bleiben, ungültige fallen weg"
        );
        // Gleiches Datum: Beide Seiten wählen dieselbe Fassung.
        let a = [k("a", "kurzbuch-de.md", 1.0)];
        let b = [k("b", "kurzbuch-de.md", 1.0)];
        assert_eq!(merge_kurzbuecher(&a, &b), merge_kurzbuecher(&b, &a));
        // Unterscheidet sich nur ein Feld, wählen beide Reihenfolgen dieselbe Fassung.
        let base = k("c", "kurzbuch-de.md", 1.0);
        let variants: [fn(&mut KurzbuchRef); 7] = [
            |r| r.created_ms = None,
            |r| r.blob.blob = "d".into(),
            |r| r.blob.size = 9,
            |r| r.blob.chunks = 9,
            |r| r.blob.kind = Some("md".into()),
            |r| r.model = Some("m".into()),
            |r| r.source = Some("Kapitel 1".into()),
        ];
        for change in variants {
            let mut other = base.clone();
            change(&mut other);
            assert_ne!(base, other);
            let one = merge_kurzbuecher(std::slice::from_ref(&base), std::slice::from_ref(&other));
            let two = merge_kurzbuecher(std::slice::from_ref(&other), std::slice::from_ref(&base));
            assert_eq!(one, two);
        }
        // Ohne Datum verliert gegen jedes Datum, auch gegen 0.
        let mut none = k("z", "kurzbuch-de.md", 0.0);
        none.created_ms = None;
        let zero = k("a", "kurzbuch-de.md", 0.0);
        assert_eq!(
            merge_kurzbuecher(&[none], std::slice::from_ref(&zero)),
            vec![zero]
        );
    }

    #[test]
    fn sprachcode_nur_streng() {
        for good in ["de", "pt-br", "zh-hans"] {
            assert!(valid_language(good), "{good}");
        }
        for bad in ["", "d", "DE", "de/x", "../de", "de-", "de-x", "de-br-x"] {
            assert!(!valid_language(bad), "{bad}");
        }
        assert_eq!(kurzbuch_language(Some("kurzbuch-de.md")), Some("de"));
        assert_eq!(kurzbuch_language(Some("kurzbuch-../x.md")), None);
    }

    #[test]
    fn leere_kurzbuch_liste_fehlt_im_format() {
        let json = serde_json::to_value(BookFiles::default()).unwrap();
        assert!(json.get("kurzbuecher").is_none());
        let k: KurzbuchRef = serde_json::from_value(serde_json::json!({
            "blob": "a", "size": 3, "chunks": 1, "file": "kurzbuch-de.md", "model": "m"
        }))
        .unwrap();
        assert_eq!(k.blob.size, 3);
        assert_eq!(k.model.as_deref(), Some("m"));
    }

    use super::*;

    fn cover_tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "lv-cover-{}-{}",
            std::process::id(),
            blobs::sha256_hex(format!("{:?}", std::time::Instant::now()).as_bytes())
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn two_editions(primary: &str) -> LibraryBook {
        let ed = |id: &str, lang: &str| library::Edition {
            id: id.into(),
            lang: lang.into(),
            title: id.into(),
            audio_key: format!("audio:b:{id}"),
            ..Default::default()
        };
        LibraryBook {
            id: "b".into(),
            title: "Buch".into(),
            schema: 2,
            editions: vec![ed("ed_de", "de"), ed("ed_en", "en")],
            primary: Some(library::Stamp {
                edition: primary.into(),
                updated: 1.0,
                device: String::new(),
            }),
            ..Default::default()
        }
    }

    fn cref(key: &[u8; KEY_LEN], bytes: &[u8], kind: &str) -> BlobRef {
        let mut r = BlobRef::new(key, &blobs::stamp_bytes(bytes)).unwrap();
        r.kind = Some(kind.into());
        r
    }

    /// Codex R5/1: Zwei Geräte mit verschiedener Hauptfassung. A (DE vorne)
    /// sendet DE flach und EN je Fassung; B (EN vorne) legt EN nach cover.*
    /// und DE nach covers/ed_de.* – kein Cover geht verloren.
    #[test]
    fn cover_je_fassung_zwischen_zwei_geraeten() {
        let key = [7u8; KEY_LEN];
        // Gerät A: was es sendet (wie build_book).
        let mut files = BookFiles {
            cover: Some(cref(&key, b"DE", "jpg")),
            ..Default::default()
        };
        files.editions.insert(
            "ed_en".into(),
            EditionFiles {
                cover: Some(cref(&key, b"EN", "png")),
                ..Default::default()
            },
        );
        // Gerät B: EN ist Hauptfassung, noch kein Cover.
        let dir = cover_tmp();
        let book = two_editions("ed_en");
        let mut cache = BTreeMap::new();
        let (primary, others) = cover_wants(&key, &dir, &mut cache, &book, &files, Some("ed_de"));
        assert_eq!(
            primary.as_ref().map(|r| r.blob.clone()),
            Some(cref(&key, b"EN", "png").blob)
        );
        assert_eq!(
            others.get("ed_de").map(|r| r.blob.clone()),
            Some(cref(&key, b"DE", "jpg").blob)
        );
        // Laden und ablegen (wie finish_download).
        std::fs::write(dir.join("t1"), b"EN").unwrap();
        place_edition_cover(&dir, &book, "ed_en", &dir.join("t1"), "png").unwrap();
        std::fs::write(dir.join("t2"), b"DE").unwrap();
        place_edition_cover(&dir, &book, "ed_de", &dir.join("t2"), "jpg").unwrap();
        assert_eq!(std::fs::read(dir.join("cover.png")).unwrap(), b"EN");
        assert_eq!(std::fs::read(dir.join("covers/ed_de.jpg")).unwrap(), b"DE");
        // Danach fehlt nichts mehr.
        let mut cache = BTreeMap::new();
        let (p2, o2) = cover_wants(&key, &dir, &mut cache, &book, &files, Some("ed_de"));
        assert!(p2.is_none() && o2.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Codex R6: über pending_files, wanted und den Abschluss des Downloads.
    /// Ein Eintrag nur mit Cover geht nicht verloren, und wechselt die
    /// Hauptfassung während des Downloads, landet jedes Cover bei seiner Fassung.
    #[test]
    fn cover_download_ueberlebt_wechsel_der_hauptfassung() {
        let key = [9u8; KEY_LEN];
        let root = cover_tmp();
        let dir = root.join("book_1");
        std::fs::create_dir_all(&dir).unwrap();
        let mut book = two_editions("ed_de");
        book.id = "book_1".into();
        library::sync_write_book(&dir, &book).unwrap();
        let mut files = BookFiles {
            cover: Some(cref(&key, b"DE", "jpg")),
            ..Default::default()
        };
        files.editions.insert(
            "ed_en".into(),
            EditionFiles {
                cover: Some(cref(&key, b"EN", "png")),
                ..Default::default()
            },
        );
        let mut cache = BTreeMap::new();
        let pending = pending_files(
            &|_| false,
            &key,
            &dir,
            &mut cache,
            &book,
            &files,
            Some("ed_de"),
        );
        assert!(pending.cover.is_none(), "mit Fassungen immer mit Kennung");
        assert!(pending.editions["ed_de"].cover.is_some());
        assert!(
            pending.editions["ed_en"].cover.is_some(),
            "reiner Cover-Eintrag bleibt"
        );
        write_sidecar(
            &dir,
            &Sidecar {
                pending,
                ..Default::default()
            },
        )
        .unwrap();
        let wanted = wanted_in(&root).unwrap();
        assert_eq!(wanted.len(), 2);
        // Während des Downloads wird EN Hauptfassung.
        book.primary = Some(library::Stamp {
            edition: "ed_en".into(),
            updated: 2.0,
            device: "pc".into(),
        });
        library::sync_write_book(&dir, &book).unwrap();
        for w in &wanted {
            let WantedKind::EditionCover(eid) = &w.what else {
                panic!("nur Cover erwartet");
            };
            let bytes: &[u8] = if eid == "ed_de" { b"DE" } else { b"EN" };
            std::fs::write(download_target(w), bytes).unwrap();
            finish_edition_cover(w, eid, &download_target(w)).unwrap();
        }
        let cover = library::sync_cover_file(&dir).unwrap();
        assert_eq!(std::fs::read(cover).unwrap(), b"EN");
        assert_eq!(std::fs::read(dir.join("covers/ed_de.jpg")).unwrap(), b"DE");
        assert!(read_sidecar(&dir).pending.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Seiten im Speicher; `.1` ist der Bibliotheksordner.
    #[derive(Default)]
    struct MemPages(BTreeMap<String, Value>, PathBuf);
    impl Host for MemPages {
        fn root(&self) -> Result<PathBuf, String> {
            Ok(self.1.clone())
        }
        fn text(&self, page_id: &str) -> Option<String> {
            self.0.get(page_id)?["state"]["text"]
                .as_str()
                .map(str::to_owned)
        }
        fn valid(&self, page_id: &str) -> bool {
            !page_id.is_empty()
        }
        fn exists(&self, page_id: &str) -> Result<bool, String> {
            Ok(self.0.contains_key(page_id))
        }
        fn create(&mut self, page_id: &str, page: &Value) -> Result<(), String> {
            self.0.insert(page_id.into(), page.clone());
            Ok(())
        }
    }

    /// Codex R7: Text wie Cover – DE (flach, Sender-Hauptfassung) und EN
    /// stehen mit Kennung aus; wechselt die Hauptfassung während des
    /// Downloads, landet jeder Text auf der Seite seiner Fassung.
    #[test]
    fn text_download_ueberlebt_wechsel_der_hauptfassung() {
        let key = [5u8; KEY_LEN];
        let root = cover_tmp();
        let dir = root.join("book_1");
        std::fs::create_dir_all(&dir).unwrap();
        let mut book = two_editions("ed_de");
        book.id = "book_1".into();
        for e in book.editions.iter_mut() {
            e.text = Some(library::TextVersion {
                page_id: format!("p_{}", e.lang),
                origin: "document".into(),
                source_name: None,
            });
        }
        book.text = book.editions[0].text.clone();
        library::sync_write_book(&dir, &book).unwrap();
        let tref = |b: &[u8]| BlobRef::new(&key, &blobs::stamp_bytes(b)).unwrap();
        let mut files = BookFiles {
            text: Some(tref(b"Deutsch")),
            ..Default::default()
        };
        files.editions.insert(
            "ed_en".into(),
            EditionFiles {
                text: Some(tref(b"English")),
                ..Default::default()
            },
        );
        let mut cache = BTreeMap::new();
        let pending = pending_files(
            &|_| false,
            &key,
            &dir,
            &mut cache,
            &book,
            &files,
            Some("ed_de"),
        );
        assert!(pending.text.is_none(), "mit Fassungen immer mit Kennung");
        assert!(pending.editions["ed_de"].text.is_some());
        assert!(pending.editions["ed_en"].text.is_some());
        write_sidecar(
            &dir,
            &Sidecar {
                pending,
                ..Default::default()
            },
        )
        .unwrap();
        let wanted = wanted_in(&root).unwrap();
        assert_eq!(wanted.len(), 2);
        book.primary = Some(library::Stamp {
            edition: "ed_en".into(),
            updated: 2.0,
            device: "pc".into(),
        });
        book.text = book.editions[1].text.clone();
        library::sync_write_book(&dir, &book).unwrap();
        let mut pages = MemPages::default();
        for w in &wanted {
            let WantedKind::EditionText(eid) = &w.what else {
                panic!("nur Texte erwartet");
            };
            let bytes: &[u8] = if eid == "ed_de" {
                b"Deutsch"
            } else {
                b"English"
            };
            std::fs::write(download_target(w), bytes).unwrap();
            finish_edition_text(w, eid, &download_target(w), &mut pages).unwrap();
        }
        let text = |p: &str| pages.0[p]["state"]["text"].as_str().unwrap().to_string();
        assert_eq!(text("p_de"), "Deutsch");
        assert_eq!(text("p_en"), "English");
        assert!(read_sidecar(&dir).pending.is_empty());
        let after = library::sync_read_book(&dir).unwrap();
        assert_eq!(after.text.unwrap().page_id, "p_en");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Codex R8/2: Upgrade, dann Wechsel, dann Abschluss. Flacher Altbestand
    /// (vor den Fassungen gespeichert) gehört zu ed_<buch>, nicht zur gerade
    /// gültigen Hauptfassung.
    #[test]
    fn altbestand_pending_nach_upgrade_und_wechsel() {
        let key = [3u8; KEY_LEN];
        let root = cover_tmp();
        let dir = root.join("book_1");
        std::fs::create_dir_all(&dir).unwrap();
        // Schema 1 auf der Platte, Altbestand im sync.json.
        std::fs::write(
            dir.join("buch.json"),
            r#"{"id":"book_1","title":"Der Weg","authors":[],"language":"de","cover_source":"generated",
               "text":{"page_id":"p_1","origin":"document"},"added_ms":1,"finished":false}"#,
        )
        .unwrap();
        let tref = |b: &[u8], kind: Option<&str>| {
            let mut r = BlobRef::new(&key, &blobs::stamp_bytes(b)).unwrap();
            r.kind = kind.map(str::to_string);
            r
        };
        let legacy = BookFiles {
            text: Some(tref(b"Deutsch", None)),
            cover: Some(tref(b"DE", Some("jpg"))),
            ..Default::default()
        };
        write_sidecar(
            &dir,
            &Sidecar {
                pending: legacy,
                ..Default::default()
            },
        )
        .unwrap();
        // Upgrade (Lesen migriert), dann EN dazu und Hauptfassung wechseln.
        let mut book = library::sync_read_book(&dir).unwrap();
        assert_eq!(library::editions::primary_id(&book), Some("ed_book_1"));
        book.editions.push(library::Edition {
            id: "ed_en".into(),
            lang: "en".into(),
            title: "The Way".into(),
            text: Some(library::TextVersion {
                page_id: "p_en".into(),
                origin: "document".into(),
                source_name: None,
            }),
            added_ms: 2.0,
            ..Default::default()
        });
        library::editions::set_primary(&mut book, "ed_en", "pc").unwrap();
        library::sync_write_book(&dir, &book).unwrap();
        std::fs::write(dir.join("cover.png"), b"EN").unwrap();
        // Abschluss.
        let wanted = wanted_in(&root).unwrap();
        let kinds: Vec<_> = wanted.iter().map(|w| w.what.clone()).collect();
        assert!(kinds.contains(&WantedKind::EditionText("ed_book_1".into())));
        assert!(kinds.contains(&WantedKind::EditionCover("ed_book_1".into())));
        assert!(!kinds.contains(&WantedKind::Text) && !kinds.contains(&WantedKind::Cover));
        let mut pages = MemPages::default();
        for w in &wanted {
            match &w.what {
                WantedKind::EditionText(eid) => {
                    std::fs::write(download_target(w), b"Deutsch").unwrap();
                    finish_edition_text(w, eid, &download_target(w), &mut pages).unwrap();
                }
                WantedKind::EditionCover(eid) => {
                    std::fs::write(download_target(w), b"DE").unwrap();
                    finish_edition_cover(w, eid, &download_target(w)).unwrap();
                }
                other => panic!("unerwartet: {other:?}"),
            }
        }
        assert_eq!(pages.0["p_1"]["state"]["text"], "Deutsch");
        assert!(!pages.0.contains_key("p_en"));
        assert_eq!(
            std::fs::read(dir.join("cover.png")).unwrap(),
            b"EN",
            "Hauptfassung unberührt"
        );
        assert_eq!(
            std::fs::read(dir.join("covers/ed_book_1.jpg")).unwrap(),
            b"DE"
        );
        assert!(read_sidecar(&dir).pending.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Codex-Auflage R9 (PR 2): Altbestand → Upgrade → Hauptfassungswechsel →
    /// Versand, bevor `wanted` lief. Der ausstehende Text und das Cover der
    /// alten Hauptfassung gehen unter ihrer Kennung hinaus, überstehen einen
    /// Empfang (`apply_book`) und landen über `finish_download` dort.
    #[test]
    fn altbestand_versand_vor_wanted_ueber_echte_einstiege() {
        let key = [6u8; KEY_LEN];
        let root = cover_tmp();
        let dir = root.join("book_1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("buch.json"),
            r#"{"id":"book_1","title":"Der Weg","authors":[],"language":"de","cover_source":"generated",
               "text":{"page_id":"p_1","origin":"document"},"added_ms":1,"finished":false}"#,
        )
        .unwrap();
        let tref = |b: &[u8], kind: Option<&str>| {
            let mut r = BlobRef::new(&key, &blobs::stamp_bytes(b)).unwrap();
            r.kind = kind.map(str::to_string);
            r
        };
        let (de_text, de_cover) = (tref(b"Deutsch", None), tref(b"DE", Some("jpg")));
        write_sidecar(
            &dir,
            &Sidecar {
                pending: BookFiles {
                    text: Some(de_text.clone()),
                    cover: Some(de_cover.clone()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        let mut book = library::sync_read_book(&dir).unwrap();
        book.editions.push(library::Edition {
            id: "ed_en".into(),
            lang: "en".into(),
            title: "The Way".into(),
            text: Some(library::TextVersion {
                page_id: "p_en".into(),
                origin: "document".into(),
                source_name: None,
            }),
            added_ms: 2.0,
            ..Default::default()
        });
        library::editions::set_primary(&mut book, "ed_en", "pc").unwrap();
        library::sync_write_book(&dir, &book).unwrap();
        std::fs::write(dir.join("cover.png"), b"EN").unwrap();
        let mut host = MemPages(BTreeMap::new(), root.clone());
        host.0.insert(
            "p_en".into(),
            json!({ "title": "The Way", "state": { "text": "English" } }),
        );
        // Versand vor `wanted`: sync.json ist noch flach.
        assert!(read_sidecar(&dir).pending.text.is_some());
        let book = library::sync_read_book(&dir).unwrap();
        let side = read_sidecar(&dir);
        let mut cache = BTreeMap::new();
        let built = build_book(&host, &key, &dir, &book, &side, &mut cache, &mut u64::MAX).unwrap();
        let files: BookFiles = serde_json::from_value(built.core_files["files"].clone()).unwrap();
        let de = &files.editions["ed_book_1"];
        assert_eq!(de.text.as_ref().map(|r| &r.blob), Some(&de_text.blob));
        assert_eq!(de.cover.as_ref().map(|r| &r.blob), Some(&de_cover.blob));
        let en_text = BlobRef::new(&key, &blobs::stamp_bytes(b"English")).unwrap();
        assert_eq!(files.text.as_ref().map(|r| &r.blob), Some(&en_text.blob));
        assert!(book_hash_locked(&host, &key, &dir, &mut cache)
            .unwrap()
            .is_some());
        // Ein Gerät mit Schema 2 schickt diesen Stand neuer zurück (als Favorit).
        let mut echo = built.core_files.clone();
        echo["favorite"] = json!(true);
        echo["updated"] = json!(f64::MAX / 2.0);
        let applied = apply_book_on(&host, &key, "book_1", Some(&echo), None, &mut cache)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            applied.remote_blobs.contains(&de_text.blob),
            "{:?} / {}",
            applied.remote_blobs,
            built.core_files["files"]
        );
        // Abschluss über den echten Einstieg.
        let wanted = wanted_in(&root).unwrap();
        let kinds: Vec<_> = wanted.iter().map(|w| w.what.clone()).collect();
        assert!(kinds.contains(&WantedKind::EditionText("ed_book_1".into())));
        assert!(kinds.contains(&WantedKind::EditionCover("ed_book_1".into())));
        for w in &wanted {
            let bytes: &[u8] = match &w.what {
                WantedKind::EditionText(_) => b"Deutsch",
                WantedKind::EditionCover(_) => b"DE",
                other => panic!("unerwartet: {other:?}"),
            };
            std::fs::write(download_target(w), bytes).unwrap();
            finish_download_on(&mut host, w).unwrap();
        }
        assert!(library::sync_read_book(&dir).unwrap().favorite);
        assert_eq!(host.0["p_1"]["state"]["text"], "Deutsch");
        assert_eq!(host.0["p_en"]["state"]["text"], "English");
        assert_eq!(std::fs::read(dir.join("cover.png")).unwrap(), b"EN");
        assert_eq!(
            std::fs::read(dir.join("covers/ed_book_1.jpg")).unwrap(),
            b"DE"
        );
        assert!(read_sidecar(&dir).pending.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Codex R8/1: Ein Altgerät (Schema 1) sendet seine Arbeitskopie; sie
    /// gehört wie im Merge zur hiesigen Hauptfassung – auch ihre Dateien.
    #[test]
    fn altgeraet_dateien_gehoeren_zur_hiesigen_hauptfassung() {
        let key = [4u8; KEY_LEN];
        let dir = cover_tmp();
        let mut local = two_editions("ed_en");
        local.id = "book_1".into();
        let incoming = LibraryBook {
            id: "book_1".into(),
            title: "Buch".into(),
            text: Some(library::TextVersion {
                page_id: "p_alt".into(),
                origin: "document".into(),
                source_name: None,
            }),
            ..Default::default()
        };
        let sender = library::editions::sender_primary(Some(&local), &incoming);
        assert_eq!(sender.as_deref(), Some("ed_en"));
        let mut merged = incoming.clone();
        library::editions::merge_incoming(Some(&local), &mut merged);
        assert_eq!(library::editions::primary_id(&merged), Some("ed_en"));
        let files = BookFiles {
            text: Some(BlobRef::new(&key, &blobs::stamp_bytes(b"alt")).unwrap()),
            cover: Some(cref(&key, b"ALT", "jpg")),
            ..Default::default()
        };
        let mut cache = BTreeMap::new();
        let pending = pending_files(
            &|_| false,
            &key,
            &dir,
            &mut cache,
            &merged,
            &files,
            sender.as_deref(),
        );
        assert!(pending.text.is_none() && pending.cover.is_none());
        assert!(pending.editions["ed_en"].text.is_some());
        assert!(pending.editions["ed_en"].cover.is_some());
        assert!(!pending.editions.contains_key("ed_de"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Codex R5/3: scheitert das Schreiben nach dem Cover-Tausch im Empfang,
    /// gehen die Cover zurück.
    #[test]
    fn empfang_schreibfehler_nimmt_cover_tausch_zurueck() {
        let dir = cover_tmp();
        std::fs::write(dir.join("cover.jpg"), b"DE").unwrap();
        std::fs::create_dir_all(dir.join("covers")).unwrap();
        std::fs::write(dir.join("covers/ed_en.jpg"), b"EN").unwrap();
        std::fs::create_dir_all(dir.join("buch.json/blockiert")).unwrap();
        let after = two_editions("ed_en");
        let moved = follow_primary_cover(&dir, Some("ed_de"), &after).unwrap();
        assert_eq!(std::fs::read(dir.join("cover.jpg")).unwrap(), b"EN");
        assert!(write_with_covers(&dir, &after, moved).is_err());
        assert_eq!(std::fs::read(dir.join("cover.jpg")).unwrap(), b"DE");
        assert_eq!(std::fs::read(dir.join("covers/ed_en.jpg")).unwrap(), b"EN");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
        map.insert(
            "tts.voice.lang.de".into(),
            r#"{"phonon":"de:Anton","updated":7}"#.into(),
        );
        map.insert("anderes".into(), "1".into());
        map.insert("progress.kaputt".into(), "{".into());
        let objs = progress_objects(&map);
        assert_eq!(objs.len(), 4);
        assert!(objs
            .iter()
            .any(|o| o.id == bookmark_object_id("tts.voice.lang.de")));
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

    #[test]
    fn stimmwahl_neuerer_stand_gewinnt() {
        assert_eq!(voice_updated(r#"{"updated":12}"#), 12.0);
        assert_eq!(voice_updated("kaputt"), 0.0);
        assert!(is_bookmark_key("tts.voice.book.book_1.en"));
        // Lokal neuer (auch sauber): eine ältere Wahl oder Aufhebung bleibt draußen.
        let local = r#"{"phonon":"en:Zoey","updated":20}"#;
        assert!(!voice_remote_wins(
            local,
            r#"{"phonon":"en:Mark","updated":10}"#
        ));
        assert!(!voice_remote_wins(local, r#"{"updated":15}"#));
        assert!(voice_remote_wins(local, r#"{"updated":30}"#));
    }

    /// Seiten auf der Platte, aufgelöst wie `page_path` (Index, Fundortkarte, Journal).
    struct DiskPages(PathBuf);
    impl DiskPages {
        fn data(&self) -> PathBuf {
            self.0.parent().unwrap().to_path_buf()
        }
        fn ort(&self, page_id: &str) -> Option<PathBuf> {
            let data = self.data();
            let index_buch = crate::library_layout::index_buch(&data, page_id).ok()?;
            let karte = crate::library_layout::Fundortkarte::aufbauen(&data);
            match crate::library_layout::aufloesen(&data, page_id, index_buch.as_deref(), &karte) {
                Ok(crate::library_layout::Aufloesung::Gefunden(p)) => Some(p),
                _ => None,
            }
        }
    }
    impl Host for DiskPages {
        fn root(&self) -> Result<PathBuf, String> {
            Ok(self.0.clone())
        }
        fn text(&self, page_id: &str) -> Option<String> {
            let raw = std::fs::read(self.ort(page_id)?.join("state.json")).ok()?;
            let v: Value = serde_json::from_slice(&raw).ok()?;
            v["text"].as_str().map(str::to_owned)
        }
        fn valid(&self, page_id: &str) -> bool {
            crate::library_layout::gueltige_seite(page_id)
        }
        fn exists(&self, page_id: &str) -> Result<bool, String> {
            Ok(self.ort(page_id).is_some())
        }
        fn create(&mut self, page_id: &str, _page: &Value) -> Result<(), String> {
            Err(format!("unerwartet angelegt: {page_id}"))
        }
    }

    /// lva#365 Spez. 6.1: Der Buch-Datensatz für den Sync ist vor und nach der
    /// Migration in Buch-Ordner gleich (bis auf `updated`). Die Texte kommen nach
    /// der Migration über den neuen Seitenort, die Fassungsordner erscheinen nicht
    /// als Buchdateien.
    #[test]
    fn buch_datensatz_ueberlebt_migration_in_buch_ordner() {
        let key = [6u8; KEY_LEN];
        let data = cover_tmp();
        let root = data.join("library");
        let dir = root.join("book_1");
        std::fs::create_dir_all(&dir).unwrap();
        let mut book = two_editions("ed_de");
        book.id = "book_1".into();
        for (e, page) in book.editions.iter_mut().zip(["page_1", "page_2"]) {
            e.text = Some(library::TextVersion {
                page_id: page.into(),
                origin: "document".into(),
                source_name: None,
            });
        }
        // Die Arbeitskopie spiegelt die Hauptfassung (sonst setzt `normalize` deren
        // Text beim Schreiben auf die leere Arbeitskopie zurück).
        book.text = book.editions[0].text.clone();
        library::sync_write_book(&dir, &book).unwrap();
        assert_eq!(
            crate::library_layout::buch_seiten(&data, "book_1"),
            vec!["page_1", "page_2"]
        );
        std::fs::write(dir.join("cover.png"), b"DE").unwrap();
        let projects = data.join("projects");
        for (page, text) in [
            ("page_1", "Deutsch"),
            ("page_2", "English"),
            ("page_9", "lose"),
        ] {
            std::fs::create_dir_all(projects.join(page)).unwrap();
            std::fs::write(
                projects.join(page).join("state.json"),
                json!({ "text": text }).to_string(),
            )
            .unwrap();
        }
        std::fs::write(
            projects.join("index.json"),
            json!({ "pages": [
                { "id": "page_1", "title": "Deutsch" },
                { "id": "page_2", "title": "English" },
                { "id": "page_9", "title": "lose" },
            ] })
            .to_string(),
        )
        .unwrap();

        let host = DiskPages(root.clone());
        let datensatz = |host: &DiskPages| {
            let book = library::sync_read_book(&dir).unwrap();
            let side = read_sidecar(&dir);
            let mut cache = BTreeMap::new();
            let built =
                build_book(host, &key, &dir, &book, &side, &mut cache, &mut u64::MAX).unwrap();
            let mut core = built.core_files;
            core.as_object_mut().unwrap().remove("updated");
            core
        };
        let vorher = datensatz(&host);
        assert_eq!(host.text("page_2").as_deref(), Some("English"));

        fn frei(_: &Path) -> Option<u64> {
            None
        }
        let optionen = crate::library_layout::Optionen {
            platz_frei: &frei,
            nach_sicherung: None,
        };
        let bericht = crate::library_layout::migrieren(&data, &optionen).unwrap();
        assert!(!bericht.angehalten, "{:?}", bericht.probleme);
        assert!(dir.join("fassungen/page_1/state.json").is_file());
        assert!(!projects.join("page_1").exists());
        assert!(!projects.join("page_2").exists());
        assert_eq!(host.text("page_1").as_deref(), Some("Deutsch"));
        assert_eq!(host.text("page_2").as_deref(), Some("English"));

        let nachher = datensatz(&host);
        assert_eq!(vorher, nachher);
        let _ = std::fs::remove_dir_all(&data);
    }
}
