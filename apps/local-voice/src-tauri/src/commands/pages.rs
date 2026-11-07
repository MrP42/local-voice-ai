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
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use tauri::AppHandle;

use crate::library_layout::{self, Aufloesung};

/// Eine Sperre fuer alle Lese-Aendern-Schreib-Zyklen an Index und
/// Seitenstand: die Oberflaeche (Befehle unten) und der Geraete-Sync
/// schreiben dieselben Dateien. Ohne Sperre ueberschreibt der eine still,
/// was der andere gerade gelesen hat (Review-Befund 14.09.).
static PAGES_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    /// Wie oft dieser Thread die Seitensperre gerade hält.
    static PAGES_LOCK_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// Der echte Mutex-Wächter dieses Threads. Er wird erst freigegeben, wenn
    /// der letzte `PagesGuard` des Threads fällt – gleich in welcher
    /// Reihenfolge die Wächter fallen.
    static PAGES_LOCK_HELD: RefCell<Option<MutexGuard<'static, ()>>> = const { RefCell::new(None) };
}

/// Wächter der Seitensperre. Die Sperre ist je Thread wiedereintrittsfähig:
/// `page_path` löst seit lva#365 unter der Seitensperre auf (Journal der
/// Buch-Ordner-Migration), und viele Aufrufer halten sie dabei schon. Wie ein
/// `MutexGuard` ist der Wächter nicht `Send` – er bleibt auf seinem Thread.
pub(crate) struct PagesGuard {
    _not_send: PhantomData<*const ()>,
}

impl Drop for PagesGuard {
    fn drop(&mut self) {
        let rest = PAGES_LOCK_DEPTH.with(|d| {
            let rest = d.get().saturating_sub(1);
            d.set(rest);
            rest
        });
        if rest == 0 {
            let held = PAGES_LOCK_HELD.with(|h| h.borrow_mut().take());
            drop(held);
        }
    }
}

pub(crate) fn lock() -> PagesGuard {
    if PAGES_LOCK_DEPTH.with(|d| d.get()) == 0 {
        let guard = PAGES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        PAGES_LOCK_HELD.with(|h| *h.borrow_mut() = Some(guard));
    }
    PAGES_LOCK_DEPTH.with(|d| d.set(d.get() + 1));
    PagesGuard {
        _not_send: PhantomData,
    }
}

/// Läuft gerade ein Umbau der Ablage (Migration, Rückfall, Zurückspielen einer
/// Sicherung)? Der hält die Seitensperre über die ganze Sicherung samt
/// Prüfsummen; Leser auf dem Hauptthread sollen dann sofort eine Meldung
/// bekommen, statt die Oberfläche einzufrieren.
static UMBAU: AtomicBool = AtomicBool::new(false);

/// Hält den Umbau-Merker, solange er lebt.
pub(crate) struct UmbauGuard(());

impl Drop for UmbauGuard {
    fn drop(&mut self) {
        UMBAU.store(false, Ordering::SeqCst);
    }
}

/// Den Merker setzen, bevor der Umbau die Sperren nimmt.
pub(crate) fn umbau_beginnen() -> UmbauGuard {
    UMBAU.store(true, Ordering::SeqCst);
    UmbauGuard(())
}

/// Für Leser: die Seitensperre, außer während eines Umbaus (dann sofort eine
/// Meldung). Wer die Sperre auf diesem Thread schon hält (der Umbau selbst,
/// Schreiber unten), sperrt wie gewohnt. Schreibende Befehle warten dagegen
/// auf das Ende des Umbaus: eine Eingabe darf nicht verloren gehen.
pub(crate) fn lock_lesend() -> Result<PagesGuard, String> {
    if PAGES_LOCK_DEPTH.with(|d| d.get()) > 0 {
        return Ok(lock());
    }
    // Nicht blockierend sperren: beginnt der Umbau zwischen Prüfen und
    // Sperren, wartet der Leser nicht die ganze Sicherung ab, sondern meldet.
    loop {
        if UMBAU.load(Ordering::SeqCst) {
            return Err(
                "Die Bibliothek wird gerade neu geordnet. Bitte einen Moment warten.".into(),
            );
        }
        let guard = match PAGES_LOCK.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(2));
                continue;
            }
        };
        PAGES_LOCK_HELD.with(|h| *h.borrow_mut() = Some(guard));
        PAGES_LOCK_DEPTH.with(|d| d.set(1));
        return Ok(PagesGuard {
            _not_send: PhantomData,
        });
    }
}

/// Schreiben ueber eine Temp-Datei plus Umbenennen: ein Absturz mittendrin
/// hinterlaesst die alte Datei, nie eine halbe.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not replace {}: {e}", path.display())
    })
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, specta::Type)]
pub struct PageInfo {
    pub id: String,
    pub title: String,
    /// Wann der Arbeitsstand zuletzt gespeichert wurde (Unix-Millisekunden,
    /// 0 = nie). Wird bei jeder Auflistung aus `state.json` frisch gelesen;
    /// der Wert im Index ist nur ein Abdruck und zählt nicht.
    #[serde(default)]
    pub modified_ms: f64,
    /// Anfang des Originaltexts, damit die Seitenliste als Verlauf taugt:
    /// man erkennt eine Seite am Inhalt, nicht nur am Titel. Best-effort aus
    /// `state.json` gelesen — das Schema gehört der Oberfläche, fehlt das
    /// Feld, bleibt die Vorschau leer.
    #[serde(default)]
    pub preview: String,
    /// Buch, in dessen Ordner die Seite liegt (`library/<buch>/fassungen/<id>`,
    /// lva#365). Leer: die Seite liegt unter `projects/<id>`. Fehlt in älteren
    /// Indizes; dann findet `page_path` die Seite über die Fundortkarte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buch: Option<String>,
}

/// Wie viele Zeichen der Vorschau die Seitenliste höchstens zeigt.
const PREVIEW_CHARS: usize = 120;

/// Vorschau aus dem Arbeitsstand: Whitespace zusammenziehen, hart kappen.
fn preview_from_state(raw: &str) -> String {
    let text = serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str().map(str::to_owned)))
        .unwrap_or_default();
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > PREVIEW_CHARS {
        let cut: String = collapsed.chars().take(PREVIEW_CHARS).collect();
        format!("{}…", cut.trim_end())
    } else {
        collapsed
    }
}

/// Zeitstempel und Vorschau einer Seite aus ihrem `state.json` nachtragen.
fn enrich(app: &AppHandle, page: &mut PageInfo) {
    let Ok(Some(dir)) = page_lookup_in(app, &page.id, Some(page.buch.as_deref())) else {
        return;
    };
    let state = dir.join("state.json");
    page.modified_ms = std::fs::metadata(&state)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    page.preview = std::fs::read_to_string(&state)
        .map(|raw| preview_from_state(&raw))
        .unwrap_or_default();
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct PagesIndex {
    pub(crate) pages: Vec<PageInfo>,
}

#[derive(Serialize, Clone, Debug, specta::Type)]
pub struct PageFile {
    pub name: String,
    pub size: u32,
    pub modified_ms: f64,
}

/// Datenordner der App (Wurzel von `projects/`, `library/`, `sicherungen/`).
pub(crate) fn data_root(app: &AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    crate::portable::data_dir()
        .cloned()
        .or_else(|| app.path().app_local_data_dir().ok())
        .ok_or_else(|| "Kein Datenverzeichnis verfügbar".to_string())
}

pub(crate) fn projects_root(app: &AppHandle) -> Result<PathBuf, String> {
    let root = data_root(app)?.join("projects");
    std::fs::create_dir_all(&root).map_err(|e| format!("could not create projects dir: {e}"))?;
    Ok(root)
}

/// Seiten-Kennungen entstehen nur in `pages_create` — alles andere ist ein
/// manipulierter Aufruf und wird abgewiesen, bevor ein Pfad daraus wird.
fn checked_id(id: &str) -> Result<&str, String> {
    if library_layout::gueltige_seite(id) {
        Ok(id)
    } else {
        Err(format!("Ungültige Seiten-Kennung: {id}"))
    }
}

/// Dateinamen aus der Oberfläche: keine Pfade, keine Aufstiege. Was hier
/// durchgeht, bleibt im Seitenordner.
pub(crate) fn checked_name(name: &str) -> Result<&str, String> {
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

/// Ist `id` eine Seiten-Kennung, wie `pages_create` sie vergibt? Nur die
/// Form, nicht ob die Seite existiert (dafür `page_lookup`).
pub(crate) fn valid_id(id: &str) -> bool {
    checked_id(id).is_ok()
}

/// Auflösung nach Spec lva#365 4.3, unter der Seitensperre: Index-Ort,
/// Fundortkarte, Journal (`von`/`nach`), `projects/<id>`. Alle Kandidaten
/// werden auf Existenz geprüft, bevor entschieden wird.
///
/// `known`: das Feld `buch` des Index-Eintrags, wenn der Aufrufer den Index
/// schon unter der Sperre geladen hat; sonst (`None`) wird er hier gelesen.
fn resolve(
    app: &AppHandle,
    id: &str,
    known: Option<Option<&str>>,
) -> Result<(PathBuf, Option<String>, Aufloesung), String> {
    checked_id(id)?;
    let _guard = lock_lesend()?;
    let data = data_root(app)?;
    let buch = match known {
        Some(buch) => buch.map(str::to_owned),
        None => load_index(app)?
            .pages
            .into_iter()
            .find(|p| p.id == id)
            .and_then(|p| p.buch),
    };
    let karte = crate::commands::library_layout::fundortkarte(&data);
    let found = library_layout::aufloesen(&data, id, buch.as_deref(), &karte)?;
    Ok((data, buch, found))
}

/// Ordner einer vorhandenen Seite. Liegt sie an zwei oder mehr Orten:
/// „Seite doppelt vorhanden“ mit allen Pfaden; nirgends: „Seite nicht
/// gefunden“. Es wird nie ein Ort geraten.
pub(crate) fn page_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    match resolve(app, id, None)? {
        (_, _, Aufloesung::Gefunden(dir)) => Ok(dir),
        (data, _, Aufloesung::Doppelt(dirs)) => {
            Err(library_layout::meldung_doppelt(&data, id, &dirs))
        }
        (_, _, Aufloesung::Fehlt) => Err(library_layout::meldung_fehlt(id)),
    }
}

/// Wie `page_path`, aber „nicht gefunden“ ist kein Fehler (`None`): für
/// Leser, die eine fehlende Seite wie eine leere behandeln.
pub(crate) fn page_lookup(app: &AppHandle, id: &str) -> Result<Option<PathBuf>, String> {
    page_lookup_in(app, id, None)
}

/// Wie [`page_lookup`]; `known` ist das Feld `buch` aus einem Index, den der
/// Aufrufer schon unter der Seitensperre geladen hat (spart das Neulesen).
pub(crate) fn page_lookup_in(
    app: &AppHandle,
    id: &str,
    known: Option<Option<&str>>,
) -> Result<Option<PathBuf>, String> {
    match resolve(app, id, known)? {
        (_, _, Aufloesung::Gefunden(dir)) => Ok(Some(dir)),
        (data, _, Aufloesung::Doppelt(dirs)) => {
            Err(library_layout::meldung_doppelt(&data, id, &dirs))
        }
        (_, _, Aufloesung::Fehlt) => Ok(None),
    }
}

/// Ordner zum Schreiben: die vorhandene Seite oder, wenn es sie noch nicht
/// gibt, ihr Ort laut Index (Buch-Ordner, sonst `projects/<id>`). Der Ordner
/// wird nicht angelegt. Ein Buch-Ordner, den es nicht (mehr) gibt (z. B. im
/// Papierkorb), entsteht hier nicht als leerer Geist neu.
pub(crate) fn page_path_or_new(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    match resolve(app, id, None)? {
        (_, _, Aufloesung::Gefunden(dir)) => Ok(dir),
        (data, _, Aufloesung::Doppelt(dirs)) => {
            Err(library_layout::meldung_doppelt(&data, id, &dirs))
        }
        (data, Some(buch), Aufloesung::Fehlt) if library_layout::gueltiges_buch(&buch) => {
            let book_dir = data.join(library_layout::BIBLIOTHEK).join(&buch);
            if !book_dir.join(library_layout::BUCH_JSON).is_file() {
                return Err(format!("Das Buch der Seite {id} ist nicht vorhanden"));
            }
            Ok(book_dir.join(library_layout::FASSUNGEN).join(id))
        }
        (_, _, Aufloesung::Fehlt) => Ok(projects_root(app)?.join(id)),
    }
}

pub(crate) fn load_index(app: &AppHandle) -> Result<PagesIndex, String> {
    let path = projects_root(app)?.join("index.json");
    // Nur "gibt es nicht" heisst "leer". Jeder andere Lesefehler (Rechte,
    // Platte) waere sonst ein leerer Index -- und der Sync wuerde alle
    // Seiten fuer geloescht halten.
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(PagesIndex::default()),
        Err(e) => return Err(format!("could not read pages index: {e}")),
    };
    // Ein zerstörter Index (Absturz beim Schreiben) darf die Seiten nicht
    // verstecken: dann wird er aus den vorhandenen Ordnern neu aufgebaut.
    match serde_json::from_str(&raw) {
        Ok(index) => Ok(index),
        Err(e) => {
            log::warn!("pages index unreadable ({e}) — rebuilding from folders");
            // Auch Seiten im Buch-Ordner (lva#365) gehören dazu, mit ihrem Buch.
            let pages = library_layout::seiten_bestand(&data_root(app)?)
                .into_iter()
                .map(|(id, buch)| PageInfo {
                    title: id.clone(),
                    id,
                    buch,
                    ..Default::default()
                })
                .collect();
            Ok(PagesIndex { pages })
        }
    }
}

pub(crate) fn store_index(app: &AppHandle, index: &PagesIndex) -> Result<(), String> {
    let path = projects_root(app)?.join("index.json");
    let raw = serde_json::to_string_pretty(index).map_err(|e| e.to_string())?;
    write_atomic(&path, raw.as_bytes())
}

pub(crate) fn fresh_id() -> String {
    format!(
        "page_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    )
}

/// Nur lesend (Sprachgespräch, Wissen „Vorlesen-Seiten“): wie `pages_list`, legt aber nie eine
/// Seite an. Titel, Änderungszeit und Textanfang; zuletzt geänderte zuerst.
pub(crate) fn pages_snapshot(app: &AppHandle) -> Result<Vec<PageInfo>, String> {
    let _guard = lock_lesend()?;
    let mut index = load_index(app)?;
    for page in &mut index.pages {
        enrich(app, page);
    }
    index
        .pages
        .sort_by(|a, b| b.modified_ms.total_cmp(&a.modified_ms));
    Ok(index.pages)
}

/// Alle Seiten in Anzeige-Reihenfolge. Gibt es keine, entsteht die erste —
/// eine leere Liste hieße für die Oberfläche „nichts, worin man arbeiten
/// kann", und diesen Zustand soll es nie geben.
#[tauri::command]
#[specta::specta]
pub fn pages_list(app: AppHandle) -> Result<Vec<PageInfo>, String> {
    let _guard = lock_lesend()?;
    let mut index = load_index(&app)?;
    if index.pages.is_empty() {
        let page = PageInfo {
            id: fresh_id(),
            title: "Erste Seite".to_string(),
            ..Default::default()
        };
        std::fs::create_dir_all(page_path_or_new(&app, &page.id)?)
            .map_err(|e| format!("could not create page dir: {e}"))?;
        index.pages.push(page);
        store_index(&app, &index)?;
    }
    for page in &mut index.pages {
        enrich(&app, page);
    }
    Ok(index.pages)
}

#[tauri::command]
#[specta::specta]
pub fn pages_create(app: AppHandle, title: String) -> Result<PageInfo, String> {
    let _guard = lock();
    let title = title.trim();
    let page = PageInfo {
        id: fresh_id(),
        title: if title.is_empty() {
            "Neue Seite".to_string()
        } else {
            title.to_string()
        },
        ..Default::default()
    };
    std::fs::create_dir_all(page_path_or_new(&app, &page.id)?)
        .map_err(|e| format!("could not create page dir: {e}"))?;
    let mut index = load_index(&app)?;
    index.pages.push(page.clone());
    store_index(&app, &index)?;
    Ok(page)
}

#[tauri::command]
#[specta::specta]
pub fn pages_rename(app: AppHandle, id: String, title: String) -> Result<(), String> {
    let _guard = lock();
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
    let _guard = lock();
    // Liegt die Seite doppelt vor, wird nichts gelöscht (Fehler mit allen Pfaden).
    if let Some(dir) = page_lookup(&app, &id)? {
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
    let _guard = lock();
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
    let Some(dir) = page_lookup(&app, &id)? else {
        return Ok(String::new());
    };
    Ok(std::fs::read_to_string(dir.join("state.json")).unwrap_or_default())
}

#[tauri::command]
#[specta::specta]
pub fn page_state_save(app: AppHandle, id: String, state: String) -> Result<(), String> {
    let _guard = lock();
    let dir = page_path_or_new(&app, &id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create page dir: {e}"))?;
    write_atomic(&dir.join("state.json"), state.as_bytes())
}

#[tauri::command]
#[specta::specta]
pub fn page_dir(app: AppHandle, id: String) -> Result<String, String> {
    let dir = page_path_or_new(&app, &id)?;
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
    let Some(dir) = page_lookup(&app, &id)? else {
        return Ok(None);
    };
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
    let Some(dir) = page_lookup(&app, &id)? else {
        return Ok(Vec::new());
    };
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

    /// `page_path` nimmt die Seitensperre, viele Aufrufer halten sie schon:
    /// derselbe Thread darf erneut sperren, ein anderer wartet.
    #[test]
    fn seitensperre_ist_je_thread_wiedereintrittsfaehig() {
        let aussen = lock();
        let innen = lock();
        drop(innen);
        let (tx, rx) = std::sync::mpsc::channel();
        let anderer = std::thread::spawn(move || {
            let _g = lock();
            tx.send(()).unwrap();
        });
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        drop(aussen);
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        anderer.join().unwrap();
    }

    /// Fällt der äußere Wächter vor dem inneren, bleibt die Sperre gehalten,
    /// bis auch der innere fällt.
    #[test]
    fn seitensperre_haelt_bis_zum_letzten_waechter() {
        let aussen = lock();
        let innen = lock();
        drop(aussen);
        let (tx, rx) = std::sync::mpsc::channel();
        let anderer = std::thread::spawn(move || {
            let _g = lock();
            tx.send(()).unwrap();
        });
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        drop(innen);
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        anderer.join().unwrap();
    }

    #[test]
    fn leser_bekommen_waehrend_des_umbaus_sofort_eine_meldung() {
        let umbau = umbau_beginnen();
        let fremd = std::thread::spawn(|| lock_lesend().is_err())
            .join()
            .unwrap();
        assert!(fremd);
        {
            let _halter = lock();
            assert!(
                lock_lesend().is_ok(),
                "der Umbau-Thread selbst liest weiter"
            );
        }
        drop(umbau);
        assert!(std::thread::spawn(|| lock_lesend().is_ok()).join().unwrap());
        // Beginnt der Umbau, während ein Leser auf die Sperre wartet, meldet
        // der Leser, statt den ganzen Umbau abzuwarten (ein Test, weil der
        // Merker global ist).
        let halter = lock();
        let (tx, rx) = std::sync::mpsc::channel();
        let leser = std::thread::spawn(move || tx.send(lock_lesend().is_err()).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(30));
        let umbau = umbau_beginnen();
        assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap());
        leser.join().unwrap();
        drop(umbau);
        drop(halter);
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

    #[test]
    fn vorschau_zieht_whitespace_zusammen_und_kappt() {
        assert_eq!(
            preview_from_state(r#"{"text":"  Guten\n  Tag  "}"#),
            "Guten Tag"
        );
        assert_eq!(preview_from_state(r#"{"summary":"x"}"#), "");
        assert_eq!(preview_from_state("kein json"), "");
        let long = format!(r#"{{"text":"{}"}}"#, "a".repeat(300));
        let p = preview_from_state(&long);
        assert_eq!(p.chars().count(), PREVIEW_CHARS + 1);
        assert!(p.ends_with('…'));
    }
}
