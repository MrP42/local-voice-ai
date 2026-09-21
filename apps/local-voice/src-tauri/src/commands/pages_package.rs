//! Seiten als Paket: eine Seite mit allem, was dazugehört (Arbeitsstand,
//! Projektdateien, auf Wunsch die verwendeten Stimmen) in EINER Datei
//! (`.lvpage`, ein Zip) — zum Aufheben, Weitergeben, Einspielen auf einem
//! anderen Rechner.
//!
//! Aufbau des Pakets:
//!   manifest.json          Titel, Zeitpunkt, Stimmenliste, Rechtebestätigung
//!   state.json             der Arbeitsstand der Seite (Schema gehört der UI)
//!   files/<name>           die Dateien des Projektordners
//!   voices/<id>.lvvoice    je mitgenommene Stimme ein Stimmen-Archiv
//!
//! Stimmen sind Aufnahmen von Menschen. Wer sie weitergibt, braucht dafür
//! das Recht (Urheber- und Persönlichkeitsrecht); der Export verlangt
//! deshalb eine ausdrückliche Bestätigung, sobald Stimmen dabei sind, und
//! schreibt sie ins Manifest.
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

use super::pages::{
    checked_name, fresh_id, load_index, lock, page_path, store_index, write_atomic, PageInfo,
};
use crate::managers::tts::{portable, protocol, registry, voices, TtsManager};

pub const PACKAGE_EXTENSION: &str = "lvpage";
const MANIFEST: &str = "manifest.json";
const STATE: &str = "state.json";
const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PackageVoice {
    pub id: String,
    pub display_name: String,
    /// Beim Export: ob die Stimme vollständig vorliegt (sonst nicht wählbar).
    /// Beim Import: ob sie auf diesem Rechner schon existiert (dann wird sie
    /// nicht überschrieben).
    pub present: bool,
}

/// Vorschau vor Export und Import — dieselbe Form für beide Richtungen.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct PackagePreview {
    pub title: String,
    pub files: Vec<String>,
    pub voices: Vec<PackageVoice>,
    pub rights_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    title: String,
    exported_at_ms: u64,
    voices: Vec<PackageVoice>,
    rights_confirmed: bool,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Stimmen, die der Text der Seite tatsächlich benutzt — über die
/// Sprechermarker, gegen die Registry aufgelöst. Nur die stehen zur Wahl.
fn voices_used(app: &AppHandle, state_raw: &str) -> Vec<PackageVoice> {
    let tts = app.state::<Arc<TtsManager>>();
    let fish_dir = tts.fish_dir_public();
    let speakers = tts.known_speakers();
    let value: serde_json::Value = serde_json::from_str(state_raw).unwrap_or_default();
    let mut ids: Vec<String> = Vec::new();
    for key in ["text", "translation", "summary"] {
        let Some(text) = value.get(key).and_then(|v| v.as_str()) else {
            continue;
        };
        for seg in protocol::split_speaker_segments(text, &speakers) {
            if let Some(voice) = seg.voice {
                if !ids.contains(&voice) {
                    ids.push(voice);
                }
            }
        }
    }
    ids.into_iter()
        .map(|id| PackageVoice {
            display_name: registry::read_meta(&fish_dir, &id).display_name,
            present: voices::voice_is_complete(&fish_dir, &id),
            id,
        })
        .collect()
}

fn page_files_list(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != STATE && !n.starts_with("state.json.tmp"))
        .collect();
    names.sort();
    names
}

/// Was ein Export enthalten würde: Dateien und die benutzten Stimmen.
#[tauri::command]
#[specta::specta]
pub fn pages_export_preview(app: AppHandle, id: String) -> Result<PackagePreview, String> {
    let dir = page_path(&app, &id)?;
    let index = load_index(&app)?;
    let title = index
        .pages
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.title.clone())
        .unwrap_or_else(|| id.clone());
    let state = std::fs::read_to_string(dir.join(STATE)).unwrap_or_default();
    Ok(PackagePreview {
        title,
        files: page_files_list(&dir),
        voices: voices_used(&app, &state),
        rights_confirmed: false,
    })
}

/// Seite als Paket schreiben. `voice_ids` = mitzunehmende Stimmen; sobald
/// eine dabei ist, muss `rights_confirmed` gesetzt sein.
#[tauri::command]
#[specta::specta]
pub fn pages_export(
    app: AppHandle,
    id: String,
    out_path: String,
    voice_ids: Vec<String>,
    rights_confirmed: bool,
) -> Result<String, String> {
    if !voice_ids.is_empty() && !rights_confirmed {
        return Err("Für die Weitergabe von Stimmen ist die Bestätigung der Rechte erforderlich".into());
    }
    let preview = pages_export_preview(app.clone(), id.clone())?;
    let dir = page_path(&app, &id)?;
    let tts = app.state::<Arc<TtsManager>>();
    let fish_dir = tts.fish_dir_public();

    let mut out = PathBuf::from(&out_path);
    if out.extension().and_then(|e| e.to_str()) != Some(PACKAGE_EXTENSION) {
        out.set_extension(PACKAGE_EXTENSION);
    }
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
    }
    let tmp = out.with_extension(format!("{PACKAGE_EXTENSION}.tmp-{}", std::process::id()));
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

    // Stimmen: nur die gewählten, nur die vollständigen.
    let mut packed_voices = Vec::new();
    for voice in &preview.voices {
        if !voice_ids.contains(&voice.id) || !voice.present {
            continue;
        }
        let tmp_voice = std::env::temp_dir().join(format!("lv-page-voice-{}-{}.lvvoice", std::process::id(), voice.id));
        portable::export_voice(&fish_dir, &voice.id, &tmp_voice)?;
        let bytes = std::fs::read(&tmp_voice).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&tmp_voice);
        put(&mut zip, &format!("voices/{}.lvvoice", voice.id), &bytes, stored)?;
        packed_voices.push(voice.clone());
    }

    let manifest = Manifest {
        version: 1,
        title: preview.title.clone(),
        exported_at_ms: now_ms(),
        voices: packed_voices,
        rights_confirmed,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    put(&mut zip, MANIFEST, &manifest_json, deflated)?;
    let state = std::fs::read(dir.join(STATE)).unwrap_or_default();
    put(&mut zip, STATE, &state, deflated)?;
    for name in &preview.files {
        let bytes = std::fs::read(dir.join(name)).map_err(|e| format!("could not read {name}: {e}"))?;
        // Audio ist schon komprimiert; Text lohnt Deflate.
        let opts = if name.ends_with(".wav") || name.ends_with(".mp3") { stored } else { deflated };
        put(&mut zip, &format!("files/{name}"), &bytes, opts)?;
    }
    zip.finish().map_err(|e| format!("could not finish package: {e}"))?;
    std::fs::rename(&tmp, &out).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not write {}: {e}", out.display())
    })?;
    Ok(out.to_string_lossy().into_owned())
}

fn open_package(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let zip = zip::ZipArchive::new(file).map_err(|e| format!("kein gültiges Seiten-Paket: {e}"))?;
    for i in 0..zip.len() {
        let name = zip.name_for_index(i).unwrap_or_default();
        let bad = name.contains("..") || name.starts_with('/') || name.starts_with('\\') || name.contains(':');
        if bad {
            return Err(format!("Paket enthält einen unzulässigen Pfad: {name}"));
        }
    }
    Ok(zip)
}

fn read_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<Vec<u8>, String> {
    let mut entry = zip.by_name(name).map_err(|e| format!("{name} fehlt im Paket: {e}"))?;
    if entry.size() > MAX_ENTRY_BYTES {
        return Err(format!("{name} ist zu groß"));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).map_err(|e| format!("could not read {name}: {e}"))?;
    Ok(bytes)
}

fn read_manifest(zip: &mut zip::ZipArchive<std::fs::File>) -> Result<Manifest, String> {
    let bytes = read_entry(zip, MANIFEST)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("manifest.json unlesbar: {e}"))
}

/// Was im Paket steckt, ohne es einzuspielen.
#[tauri::command]
#[specta::specta]
pub fn pages_package_inspect(app: AppHandle, path: String) -> Result<PackagePreview, String> {
    let mut zip = open_package(Path::new(&path))?;
    let manifest = read_manifest(&mut zip)?;
    let fish_dir = app.state::<Arc<TtsManager>>().fish_dir_public();
    let files: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.name_for_index(i).map(str::to_owned))
        .filter_map(|n| n.strip_prefix("files/").map(str::to_owned))
        .filter(|n| !n.is_empty())
        .collect();
    let voices = manifest
        .voices
        .into_iter()
        .map(|v| PackageVoice {
            present: voices::voice_is_complete(&fish_dir, &v.id),
            ..v
        })
        .collect();
    Ok(PackagePreview {
        title: manifest.title,
        files,
        voices,
        rights_confirmed: manifest.rights_confirmed,
    })
}

/// Paket als NEUE Seite einspielen (nie eine bestehende überschreiben).
/// Stimmen nur auf Wunsch und nur, wenn sie hier noch nicht existieren.
#[tauri::command]
#[specta::specta]
pub fn pages_import(app: AppHandle, path: String, import_voices: bool) -> Result<PageInfo, String> {
    let mut zip = open_package(Path::new(&path))?;
    let manifest = read_manifest(&mut zip)?;
    let state = read_entry(&mut zip, STATE).unwrap_or_default();

    let page = {
        let _guard = lock();
        let mut index = load_index(&app)?;
        let base = manifest.title.trim();
        let base = if base.is_empty() { "Importierte Seite" } else { base };
        let mut title = base.to_string();
        let mut n = 2;
        while index.pages.iter().any(|p| p.title == title) {
            title = format!("{base} ({n})");
            n += 1;
        }
        let page = PageInfo { id: fresh_id(), title, ..Default::default() };
        let dir = page_path(&app, &page.id)?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("could not create page dir: {e}"))?;
        write_atomic(&dir.join(STATE), &state)?;
        index.pages.push(page.clone());
        store_index(&app, &index)?;
        page
    };
    let dir = page_path(&app, &page.id)?;

    let names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.name_for_index(i).map(str::to_owned))
        .collect();
    for name in &names {
        if let Some(file) = name.strip_prefix("files/") {
            if file.is_empty() || checked_name(file).is_err() {
                continue;
            }
            let bytes = read_entry(&mut zip, name)?;
            std::fs::write(dir.join(file), bytes).map_err(|e| format!("could not write {file}: {e}"))?;
        }
    }

    if import_voices {
        let fish_dir = app.state::<Arc<TtsManager>>().fish_dir_public();
        for voice in &manifest.voices {
            if voices::voice_is_complete(&fish_dir, &voice.id) {
                continue;
            }
            let entry = format!("voices/{}.lvvoice", voice.id);
            if !names.contains(&entry) {
                continue;
            }
            let bytes = read_entry(&mut zip, &entry)?;
            let tmp = std::env::temp_dir().join(format!("lv-page-import-{}-{}.lvvoice", std::process::id(), voice.id));
            std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
            let result = portable::import_voice(&fish_dir, &tmp, Some(&voice.display_name));
            let _ = std::fs::remove_file(&tmp);
            if let Err(e) = result {
                log::warn!("page import: voice {} not imported: {e}", voice.id);
            }
        }
    }
    Ok(page)
}
