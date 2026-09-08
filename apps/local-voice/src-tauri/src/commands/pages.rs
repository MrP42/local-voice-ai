//! Seiten des Vorlesens: je Seite ein Ordner, darin der Arbeitsstand und die
//! Dateien des Projekts.
//!
//! Eine „Seite" ist ein Vorlese-Arbeitsblatt — Originaltext, Übersetzung,
//! Zusammenfassung, offener Reiter — plus ein Projektordner, in dem erzeugte
//! Audiodateien und mitgebrachte Dokumente liegen. Ablage unter
//! `<appdata>/projects/<id>/`; die Reihenfolge und die Titel stehen in
//! `projects/index.json`.
//!
//! Der Arbeitsstand (`state.json`) ist für das Backend ein undurchsichtiger
//! Text: sein Schema gehört der Oberfläche. Das Backend garantiert nur, dass
//! er die Seite nicht verlässt und einen Neustart überlebt — so kommt keine
//! Migration an, wenn die Oberfläche ein Feld dazuerfindet.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::AppHandle;

#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
pub struct PageInfo {
    pub id: String,
    pub title: String,
}

#[derive(Serialize, Deserialize, Default)]
struct PagesIndex {
    pages: Vec<PageInfo>,
}

#[derive(Serialize, Clone, Debug, specta::Type)]
pub struct PageFile {
    pub name: String,
    pub size: u32,
    pub modified_ms: f64,
}

fn projects_root(app: &AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let base = crate::portable::data_dir()
        .cloned()
        .or_else(|| app.path().app_local_data_dir().ok())
        .ok_or("Kein Datenverzeichnis verfügbar")?;
    let root = base.join("projects");
    std::fs::create_dir_all(&root).map_err(|e| format!("could not create projects dir: {e}"))?;
    Ok(root)
}

/// Seiten-Kennungen entstehen nur in `pages_create` — alles andere ist ein
/// manipulierter Aufruf und wird abgewiesen, bevor ein Pfad daraus wird.
fn checked_id(id: &str) -> Result<&str, String> {
    let valid = id
        .strip_prefix("page_")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric()));
    if valid {
        Ok(id)
    } else {
        Err(format!("Ungültige Seiten-Kennung: {id}"))
    }
}

/// Dateinamen aus der Oberfläche: keine Pfade, keine Aufstiege. Was hier
/// durchgeht, bleibt im Seitenordner.
fn checked_name(name: &str) -> Result<&str, String> {
    let bad = name.is_empty()
        || name.len() > 150
        || name.contains(['/', '\\', ':'])
        || name == "."
        || name.contains("..")
        || name == "state.json";
    if bad {
        Err(format!("Ungültiger Dateiname: {name}"))
    } else {
        Ok(name)
    }
}

fn page_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(projects_root(app)?.join(checked_id(id)?))
}

fn load_index(app: &AppHandle) -> Result<PagesIndex, String> {
    let path = projects_root(app)?.join("index.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(PagesIndex::default());
    };
    // Ein zerstörter Index (Absturz beim Schreiben) darf die Seiten nicht
    // verstecken: dann wird er aus den vorhandenen Ordnern neu aufgebaut.
    match serde_json::from_str(&raw) {
        Ok(index) => Ok(index),
        Err(e) => {
            log::warn!("pages index unreadable ({e}) — rebuilding from folders");
            let mut pages = Vec::new();
            if let Ok(entries) = std::fs::read_dir(projects_root(app)?) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if entry.path().is_dir() && checked_id(&name).is_ok() {
                        pages.push(PageInfo {
                            id: name.clone(),
                            title: name,
                        });
                    }
                }
            }
            Ok(PagesIndex { pages })
        }
    }
}

fn store_index(app: &AppHandle, index: &PagesIndex) -> Result<(), String> {
    let path = projects_root(app)?.join("index.json");
    let raw = serde_json::to_string_pretty(index).map_err(|e| e.to_string())?;
    std::fs::write(&path, raw).map_err(|e| format!("could not write pages index: {e}"))
}

fn fresh_id() -> String {
    format!(
        "page_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    )
}

/// Alle Seiten in Anzeige-Reihenfolge. Gibt es keine, entsteht die erste —
/// eine leere Liste hieße für die Oberfläche „nichts, worin man arbeiten
/// kann", und diesen Zustand soll es nie geben.
#[tauri::command]
#[specta::specta]
pub fn pages_list(app: AppHandle) -> Result<Vec<PageInfo>, String> {
    let mut index = load_index(&app)?;
    if index.pages.is_empty() {
        let page = PageInfo {
            id: fresh_id(),
            title: "Erste Seite".to_string(),
        };
        std::fs::create_dir_all(page_path(&app, &page.id)?)
            .map_err(|e| format!("could not create page dir: {e}"))?;
        index.pages.push(page);
        store_index(&app, &index)?;
    }
    Ok(index.pages)
}

#[tauri::command]
#[specta::specta]
pub fn pages_create(app: AppHandle, title: String) -> Result<PageInfo, String> {
    let title = title.trim();
    let page = PageInfo {
        id: fresh_id(),
        title: if title.is_empty() {
            "Neue Seite".to_string()
        } else {
            title.to_string()
        },
    };
    std::fs::create_dir_all(page_path(&app, &page.id)?)
        .map_err(|e| format!("could not create page dir: {e}"))?;
    let mut index = load_index(&app)?;
    index.pages.push(page.clone());
    store_index(&app, &index)?;
    Ok(page)
}

#[tauri::command]
#[specta::specta]
pub fn pages_rename(app: AppHandle, id: String, title: String) -> Result<(), String> {
    checked_id(&id)?;
    let title = title.trim();
    if title.is_empty() {
        return Err("Der Titel darf nicht leer sein".to_string());
    }
    let mut index = load_index(&app)?;
    let page = index
        .pages
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("Unbekannte Seite")?;
    page.title = title.to_string();
    store_index(&app, &index)
}

/// Seite löschen — mitsamt ihrem Ordner und allen Dateien darin. Die
/// Rückfrage dazu stellt die Oberfläche; hier wird nur noch ausgeführt.
#[tauri::command]
#[specta::specta]
pub fn pages_delete(app: AppHandle, id: String) -> Result<(), String> {
    let dir = page_path(&app, &id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("could not delete page: {e}"))?;
    }
    let mut index = load_index(&app)?;
    index.pages.retain(|p| p.id != id);
    store_index(&app, &index)
}

/// Neue Reihenfolge, als vollständige Liste der Kennungen. Unbekannte werden
/// übergangen, vergessene hinten angehängt — die Liste der Oberfläche kann
/// einen Moment alt sein, und deshalb darf hier keine Seite verloren gehen.
#[tauri::command]
#[specta::specta]
pub fn pages_reorder(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    let mut index = load_index(&app)?;
    let mut reordered: Vec<PageInfo> = Vec::with_capacity(index.pages.len());
    for id in &ids {
        if let Some(pos) = index.pages.iter().position(|p| &p.id == id) {
            reordered.push(index.pages.remove(pos));
        }
    }
    reordered.append(&mut index.pages);
    store_index(&app, &PagesIndex { pages: reordered })
}

#[tauri::command]
#[specta::specta]
pub fn page_state_load(app: AppHandle, id: String) -> Result<String, String> {
    let path = page_path(&app, &id)?.join("state.json");
    Ok(std::fs::read_to_string(path).unwrap_or_default())
}

#[tauri::command]
#[specta::specta]
pub fn page_state_save(app: AppHandle, id: String, state: String) -> Result<(), String> {
    let dir = page_path(&app, &id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create page dir: {e}"))?;
    std::fs::write(dir.join("state.json"), state)
        .map_err(|e| format!("could not save page state: {e}"))
}

#[tauri::command]
#[specta::specta]
pub fn page_dir(app: AppHandle, id: String) -> Result<String, String> {
    let dir = page_path(&app, &id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create page dir: {e}"))?;
    Ok(dir.to_string_lossy().into_owned())
}

/// Die Dateien einer Seite, jüngste zuerst. `state.json` gehört der App und
/// erscheint nicht — für den Nutzer ist sie kein Inhalt, und löschen soll er
/// sie erst recht nicht.
/// Herkunft einer erzeugten Aufnahme: aus welchem Text sie entstand und wer
/// sie gesprochen hat. `None`, wenn die Datei vor dieser Fassung entstand
/// oder von Hand hinzugefuegt wurde.
#[tauri::command]
#[specta::specta]
pub fn page_audio_note(
    app: AppHandle,
    id: String,
    name: String,
) -> Result<Option<crate::managers::tts::notes::AudioNote>, String> {
    let dir = page_path(&app, &id)?;
    // Nur Dateien im Ordner dieses Arbeitsblatts, kein Weg nach draussen.
    let file = dir.join(&name);
    if file.parent() != Some(dir.as_path()) {
        return Err("Datei liegt nicht in diesem Arbeitsblatt".to_string());
    }
    Ok(crate::managers::tts::notes::read(&file))
}

#[tauri::command]
#[specta::specta]
pub fn page_files(app: AppHandle, id: String) -> Result<Vec<PageFile>, String> {
    let dir = page_path(&app, &id)?;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut files: Vec<PageFile> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // `state.json` ist der Zustand des Arbeitsblatts, ein Beileger
            // gehoert zu der Aufnahme daneben — beides ist keine Datei, die
            // jemand in seiner Ablage sehen will.
            if name == "state.json" || crate::managers::tts::notes::is_note(&name) {
                return None;
            }
            let meta = e.metadata().ok()?;
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as f64)
                .unwrap_or(0.0);
            Some(PageFile {
                name,
                size: meta.len().min(u32::MAX as u64) as u32,
                modified_ms,
            })
        })
        .collect();
    files.sort_by(|a, b| b.modified_ms.total_cmp(&a.modified_ms));
    Ok(files)
}

#[tauri::command]
#[specta::specta]
pub fn page_file_delete(app: AppHandle, id: String, name: String) -> Result<(), String> {
    let path = page_path(&app, &id)?.join(checked_name(&name)?);
    std::fs::remove_file(&path).map_err(|e| format!("could not delete {name}: {e}"))
}

#[tauri::command]
#[specta::specta]
pub fn page_file_rename(
    app: AppHandle,
    id: String,
    name: String,
    new_name: String,
) -> Result<(), String> {
    let dir = page_path(&app, &id)?;
    let target = dir.join(checked_name(&new_name)?);
    if target.exists() {
        return Err(format!("Es gibt bereits eine Datei namens {new_name}"));
    }
    std::fs::rename(dir.join(checked_name(&name)?), target)
        .map_err(|e| format!("could not rename {name}: {e}"))
}

/// Eine vorhandene Datei in den Seitenordner kopieren (nicht verschieben:
/// das Original gehört dem Nutzer und bleibt, wo es ist).
#[tauri::command]
#[specta::specta]
pub fn page_file_add(app: AppHandle, id: String, source: String) -> Result<String, String> {
    let source_path = std::path::PathBuf::from(&source);
    let name = source_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Quelle ohne Dateinamen")?
        .to_string();
    checked_name(&name)?;
    let target = page_path(&app, &id)?.join(&name);
    if target.exists() {
        return Err(format!("Es gibt bereits eine Datei namens {name}"));
    }
    std::fs::copy(&source_path, &target).map_err(|e| format!("could not copy {name}: {e}"))?;
    Ok(name)
}

/// Datei mit ihrer Standardanwendung öffnen. Über `explorer`, weil das
/// opener-Plugin beliebige Pfade nur mit erweiterten Berechtigungen öffnet —
/// und der Pfad hier ohnehin aus dem eigenen Seitenordner stammt.
#[tauri::command]
#[specta::specta]
pub fn page_file_open(app: AppHandle, id: String, name: String) -> Result<(), String> {
    let path = page_path(&app, &id)?.join(checked_name(&name)?);
    if !path.is_file() {
        return Err(format!("{name} existiert nicht mehr"));
    }
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("could not open {name}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Die Schranke gegen Pfad-Ausbrüche: nur selbst vergebene Kennungen.
    #[test]
    fn fremde_kennungen_werden_abgewiesen() {
        assert!(checked_id("page_1755730000000").is_ok());
        assert!(checked_id("page_..").is_err());
        assert!(checked_id("page_a/b").is_err());
        assert!(checked_id("..").is_err());
        assert!(checked_id("page_").is_err());
        assert!(checked_id("").is_err());
    }

    #[test]
    fn dateinamen_bleiben_im_ordner() {
        assert!(checked_name("vorlesen.wav").is_ok());
        assert!(checked_name("Bericht 2026.docx").is_ok());
        assert!(checked_name("..\\settings.json").is_err());
        assert!(checked_name("a/b.txt").is_err());
        assert!(checked_name("C:whatever").is_err());
        assert!(checked_name("state.json").is_err(), "state.json ist tabu");
        assert!(checked_name("").is_err());
    }
}
