//! lva#365 P3: Sync-Sammlung `book_file` für Notizen, Anhänge und Transkript-Text im
//! Buch-Ordner (Spec `2026-11-07-buch-ordner-und-projekte.md`, 6.2). Der Kern ohne Tauri
//! liegt in `crate::buch_dateien`; hier kommen Schlüssel, Blobs und Bibliotheks-Sperre dazu.

use super::blobs::{FileStamp, Source};
use super::books::{self, Applied, ApplyError, BlobRef, PendingUpload};
use super::collect::{self, LocalObject};
use super::crypto::KEY_LEN;
use crate::buch_dateien as kern;
use crate::library;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

pub const COLLECTION_BOOK_FILE: &str = kern::SAMMLUNG;

/// Die `book_file`-Objekte dieses Geräts.
#[derive(Default)]
pub struct FilesLocal {
    pub objects: Vec<LocalObject>,
    /// Objekt-Kennung → Datei, die vor dem Push in der Ablage sein muss.
    pub uploads: BTreeMap<String, PendingUpload>,
    /// Objekte, die es gibt, die aber in diesem Lauf nicht gesendet werden (neuere
    /// Version, Prüfsumme über dem Budget, wartend auf ihr Buch).
    pub present: BTreeSet<String>,
    /// Ein Buch-Ordner war nicht lesbar: in diesem Lauf keine Grabsteine für die Sammlung.
    pub hold_all: bool,
    pub warnings: Vec<String>,
}

fn now_ms() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64
}

/// Den Datensatz um den Blob ergänzen, den dieses Gerät aus der Datei bildet.
fn with_blob(
    key: &[u8; KEY_LEN],
    lokal: kern::Lokal,
) -> Option<(LocalObject, Option<PendingUpload>)> {
    let mut value = lokal.wert;
    let upload = match lokal.quelle {
        None => None,
        Some(q) => {
            let stamp = FileStamp {
                size: q.groesse,
                mtime_ms: q.mtime_ms,
                sha256: q.sha256,
            };
            let blob = BlobRef::new(key, &stamp)?;
            value["blob"] = serde_json::to_value(&blob).ok()?;
            Some(PendingUpload {
                blob,
                source: Source::File(q.pfad),
                stamp: Some(stamp),
            })
        }
    };
    Some((
        LocalObject {
            collection: COLLECTION_BOOK_FILE,
            id: lokal.id,
            value,
        },
        upload,
    ))
}

/// Geparkte Datensätze anwenden, deren Buch inzwischen da ist. Der Aufrufer hält die
/// Bibliotheks-Sperre.
fn unpark_ready(root: &Path, device_name: &str) {
    for (id, value) in kern::geparkt(root) {
        let Some(buch) = value.get("book").and_then(Value::as_str) else {
            let _ = kern::ausparken(root, &id);
            continue;
        };
        let Ok(dir) = library::sync_book_dir(root, buch) else {
            let _ = kern::ausparken(root, &id);
            continue;
        };
        if !dir.join("buch.json").is_file() {
            continue;
        }
        match kern::anwenden(&dir, buch, &id, &value, false, None, device_name) {
            Ok(_) | Err(kern::AnwendFehler::Ungueltig(_)) => {
                let _ = kern::ausparken(root, &id);
            }
            Err(kern::AnwendFehler::Wiederholen(_)) => {
                log::warn!("Sync: geparkte Datei {id} noch nicht ablegbar");
            }
        }
    }
}

/// Alle `book_file`-Objekte. `budget` begrenzt, was ein Lauf an Prüfsummen neu liest.
pub fn collect(
    app: &AppHandle,
    key: &[u8; KEY_LEN],
    device_id: &str,
    device_name: &str,
    budget: &mut u64,
) -> Result<FilesLocal, String> {
    let root = library::sync_root(app)?;
    let _guard = library::sync_lock();
    unpark_ready(&root, device_name);
    let mut out = FilesLocal::default();
    out.present
        .extend(kern::geparkt(&root).into_iter().map(|(id, _)| id));
    let now = now_ms();
    for (buch, dir) in books::book_dirs(&root)? {
        let found = match kern::sammeln(&dir, &buch, device_id, now, budget) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("Sync: Dateien von Buch {buch} übersprungen: {e}");
                out.warnings.push(format!("Dateien eines Buchs: {e}"));
                out.hold_all = true;
                continue;
            }
        };
        out.present.extend(found.vorhanden);
        // Hinweise nennen Dateinamen: nur in den Status, nie ins Log.
        out.warnings.extend(found.hinweise);
        for l in found.lokal {
            let id = l.id.clone();
            let Some((obj, upload)) = with_blob(key, l) else {
                out.present.insert(id);
                continue;
            };
            if collect::canonical(&obj.value).len() > collect::MAX_PLAINTEXT {
                out.present.insert(id);
                continue;
            }
            if let Some(u) = upload {
                out.uploads.insert(id, u);
            }
            out.objects.push(obj);
        }
    }
    Ok(out)
}

/// Der lokale Datensatz `id` aus dem gespeicherten Stand (wie beim letzten Sammeln, ohne
/// die Platte erneut zu lesen): sein Hash und sein Blob-Verweis.
fn local_in(
    key: &[u8; KEY_LEN],
    dir: &Path,
    buch: &str,
    id: &str,
) -> Result<Option<(String, Option<Value>)>, String> {
    Ok(kern::lokal_aus_stand(dir, buch, id)?
        .and_then(|l| with_blob(key, l))
        .map(|(o, _)| {
            let blob = o.value.get("blob").filter(|b| !b.is_null()).cloned();
            (collect::hash(&o.value), blob)
        }))
}

/// Ein `book_file`-Objekt anwenden. `value` ist `None` bei einer Löschung durch den Hub.
pub fn apply(
    app: &AppHandle,
    key: &[u8; KEY_LEN],
    id: &str,
    value: Option<&Value>,
    synced_hash: Option<&str>,
    device_name: &str,
) -> Result<Applied, ApplyError> {
    if !kern::gueltige_id(id) {
        return Err(ApplyError::Invalid(format!("Ungültige Datei-Kennung {id}")));
    }
    let root = library::sync_root(app)?;
    let _guard = library::sync_lock();
    let Some(value) = value else {
        kern::ausparken(&root, id)?;
        let mut changed = false;
        for (buch, dir) in books::book_dirs(&root)? {
            // Ein unlesbarer Buch-Stand hält die anderen Bücher nicht auf.
            match kern::vergessen(&dir, &buch, id) {
                Ok(c) => changed |= c,
                Err(_) => log::warn!("Sync: Datei-Stand von Buch {buch} nicht lesbar"),
            }
        }
        return Ok(Applied {
            changed,
            ..Default::default()
        });
    };
    let buch = value
        .get("book")
        .and_then(Value::as_str)
        .ok_or_else(|| ApplyError::Invalid("Datei ohne Buch".into()))?;
    let dir = library::sync_book_dir(&root, buch).map_err(ApplyError::Invalid)?;
    // Neuere Versionen bewahrt der Kern nur auf; ihr Verweis wird nicht gedeutet.
    let known = kern::version(value) == Some(kern::VERSION);
    let blob = match value.get("blob").filter(|b| known && !b.is_null()) {
        Some(b) => {
            let r: BlobRef = serde_json::from_value(b.clone())
                .map_err(|e| ApplyError::Invalid(format!("Dateiverweis unlesbar: {e}")))?;
            if !r.valid() {
                return Err(ApplyError::Invalid("Ungültiger Dateiverweis".into()));
            }
            Some(r.blob)
        }
        None => None,
    };
    if !dir.join("buch.json").is_file() {
        // Das Buch kommt noch (oder nie): aufbewahren, beim Sammeln erneut versuchen.
        kern::parken(&root, id, value)?;
        return Ok(Applied::default());
    }
    let local = local_in(key, &dir, buch, id)?;
    let open = local
        .as_ref()
        .is_some_and(|(h, _)| Some(h.as_str()) != synced_hash);
    let local_blob = local.as_ref().and_then(|(_, b)| b.as_ref());
    let changed = kern::anwenden(&dir, buch, id, value, open, local_blob, device_name).map_err(
        |e| match e {
            kern::AnwendFehler::Ungueltig(s) => ApplyError::Invalid(s),
            kern::AnwendFehler::Wiederholen(s) => ApplyError::Retry(s),
        },
    )?;
    Ok(Applied {
        changed,
        remote_blobs: blob.into_iter().collect(),
        ..Default::default()
    })
}

/// Eine Datei, die ein anderes Gerät gemeldet hat und die hier fehlt.
#[derive(Clone, Debug)]
pub struct FileWanted {
    pub book_id: String,
    pub dir: PathBuf,
    pub rel: String,
    pub blob: BlobRef,
    /// Der Verweis, wie er im Stand steht (Abgleich beim Ablegen).
    pub raw: Value,
}

pub fn wanted(app: &AppHandle) -> Result<Vec<FileWanted>, String> {
    let root = library::sync_root(app)?;
    let _guard = library::sync_lock();
    let mut out = Vec::new();
    for (buch, dir) in books::book_dirs(&root)? {
        let Ok(offen) = kern::erwartet(&dir) else {
            continue;
        };
        for e in offen {
            let Ok(blob) = serde_json::from_value::<BlobRef>(e.blob.clone()) else {
                continue;
            };
            if blob.valid() {
                out.push(FileWanted {
                    book_id: buch.clone(),
                    dir: dir.clone(),
                    rel: e.rel,
                    blob,
                    raw: e.blob,
                });
            }
        }
    }
    out.sort_by_key(|w| w.blob.size);
    Ok(out)
}

pub fn download_target(w: &FileWanted) -> PathBuf {
    kern::zwischenpfad(&w.dir, &w.blob.blob)
}

pub fn finish_download(w: &FileWanted, device_name: &str) -> Result<Option<PathBuf>, String> {
    let _guard = library::sync_lock();
    kern::ablegen(
        &w.dir,
        &w.book_id,
        &w.rel,
        &w.raw,
        &download_target(w),
        device_name,
    )
}
