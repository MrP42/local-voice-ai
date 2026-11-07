//! Tauri-Hüllen der Buch-Ordner-Ablage (lva#365, P2a). Die Logik steht in
//! `crate::library_layout` (rein über den Datenordner, ohne AppHandle); hier
//! werden nur Sperren gehalten, der freie Platz ermittelt und die Ergebnisse
//! für die Oberfläche übersetzt.
//!
//! Sperrreihenfolge, wie im Geräte-Sync: Sync-Laufsperre (`run_lock`) →
//! Bibliothekssperre → Seitensperre. Wer anders herum sperrt, riskiert eine
//! Verklemmung mit `sync/books.rs`, das unter der Bibliothekssperre Seiten liest.
//!
//! Die Migration läuft nie von selbst; die Oberfläche (P2b) fragt. Nur ein
//! offenes Journal wird beim Start nach der Tabelle in Spec 4.3 aufgelöst.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

use crate::commands::pages;
use crate::library_layout::{self, Bericht, Fundortkarte, Optionen, Problem};
use crate::sync::SyncEngine;

// ---------------------------------------------------------------------------
// Fundortkarte (Cache)
// ---------------------------------------------------------------------------

static KARTE: Mutex<Option<(PathBuf, Arc<Fundortkarte>)>> = Mutex::new(None);

/// Fundortkarte des Datenordners; beim ersten Gebrauch aufgebaut. Ein Cache:
/// ob ein Ort existiert, prüft `page_path` bei jedem Aufruf auf der Platte.
pub(crate) fn fundortkarte(data: &Path) -> Arc<Fundortkarte> {
    let mut slot = KARTE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((dir, karte)) = slot.as_ref() {
        if dir == data {
            return karte.clone();
        }
    }
    let karte = Arc::new(Fundortkarte::aufbauen(data));
    *slot = Some((data.to_path_buf(), karte.clone()));
    karte
}

/// Nach jeder Verschiebung (Migration, Rückfall, Zurückspielen, Papierkorb
/// eines Buchs) neu aufbauen. Der Aufrufer hält die Seitensperre.
pub(crate) fn fundortkarte_neu(data: &Path) {
    let karte = Arc::new(Fundortkarte::aufbauen(data));
    *KARTE.lock().unwrap_or_else(|e| e.into_inner()) = Some((data.to_path_buf(), karte));
}

// ---------------------------------------------------------------------------
// Freier Platz
// ---------------------------------------------------------------------------

/// Freier Platz (für den Nutzer verfügbar) auf dem Datenträger von `dir`.
#[cfg(windows)]
fn freier_platz(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut frei: u64 = 0;
    // SAFETY: `wide` ist nullterminiert und überlebt den Aufruf; `frei` ist ein
    // gültiger Zeiger auf einen u64.
    let ergebnis = unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide.as_ptr()),
            Some(&mut frei as *mut u64),
            None,
            None,
        )
    };
    ergebnis.ok()?;
    Some(frei)
}

#[cfg(unix)]
fn freier_platz(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let pfad = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `pfad` ist nullterminiert, `stat` ein gültiger Puffer.
    if unsafe { libc::statvfs(pfad.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

#[cfg(not(any(windows, unix)))]
fn freier_platz(_dir: &Path) -> Option<u64> {
    None
}

fn optionen() -> Optionen<'static> {
    Optionen {
        platz_frei: &freier_platz,
        nach_sicherung: None,
    }
}

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

/// Beim Start, vor dem ersten Lesen der Bibliothek: ein offenes Journal nach
/// der Tabelle in 4.3 auflösen. Bei Doppelung oder Fehlen hält der Kern an und
/// verschiebt nichts; den Zustand meldet `library_layout_status`. Danach wird
/// die Fundortkarte gebaut. Im Protokoll stehen nur Zahlen, keine Inhalte.
pub(crate) fn start_hook(app: &AppHandle) {
    let Ok(data) = pages::data_root(app) else {
        return;
    };
    {
        let _library = crate::library::sync_lock();
        let _pages = pages::lock();
        // Zuerst ein unterbrochenes Zurückspielen einer Sicherung zurücknehmen;
        // gelingt das nicht, wird nichts weiter umgebaut.
        match library_layout::zurueckspielen_wiederanlauf(&data) {
            Ok(true) => log::warn!("Buch-Ordner: unterbrochenes Zurückspielen zurückgenommen"),
            Ok(false) => {}
            Err(e) => {
                log::warn!("Buch-Ordner: {e}");
                fundortkarte_neu(&data);
                return;
            }
        }
        match library_layout::wiederanlauf(&data, &optionen()) {
            Ok(b) if b.angehalten => log::warn!(
                "Buch-Ordner: Wiederanlauf angehalten ({} Problem(e), {} Schritt(e) erledigt)",
                b.probleme.len(),
                b.erledigt
            ),
            Ok(b) if b.erledigt > 0 => log::info!(
                "Buch-Ordner: Wiederanlauf abgeschlossen ({} Schritt(e))",
                b.erledigt
            ),
            Ok(_) => {}
            Err(e) => log::warn!("Buch-Ordner: Wiederanlauf nicht möglich: {e}"),
        }
        fundortkarte_neu(&data);
    }
}

// ---------------------------------------------------------------------------
// Typen für die Oberfläche
// ---------------------------------------------------------------------------

/// Ein Befund, der die Buch-Ordner-Ablage anhält.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct LibraryLayoutProblem {
    /// Seiten-ID (leer bei einem allgemeinen Fehler).
    pub seite: String,
    /// `doppelt`, `fehlt`, `unerwartet` oder `fehler`.
    pub art: String,
    pub meldung: String,
    /// Betroffene Pfade relativ zum Datenordner.
    pub pfade: Vec<String>,
}

/// Zustand der Buch-Ordner-Ablage.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct LibraryLayoutStatus {
    /// 1 = Seiten unter `projects/`, 2 = Fassungen im Buch-Ordner.
    pub layout: u32,
    /// Phase des Journals: `geplant`, `gesichert`, `fertig`, `rueckfall`,
    /// `zurueckgenommen`; `None`, wenn es keines gibt.
    pub phase: Option<String>,
    pub probleme: Vec<LibraryLayoutProblem>,
    /// Übersprungene Seiten mit Grund.
    pub hinweise: Vec<String>,
    /// Seiten, die noch zu verschieben sind.
    pub zu_verschieben: u32,
    /// Nötiger Platz für die Sicherung (Bytes).
    pub platz_benoetigt: f64,
    /// Freier Platz auf dem Datenträger (Bytes), falls bekannt.
    pub platz_frei: Option<f64>,
}

/// Ergebnis von Migration oder Rückfall.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct LibraryLayoutReport {
    pub phase: Option<String>,
    /// Schritte, die in diesem Lauf etwas geändert haben.
    pub erledigt: u32,
    /// Angehalten: an dieser Stelle wurde nichts weiter verschoben.
    pub angehalten: bool,
    pub probleme: Vec<LibraryLayoutProblem>,
    pub hinweise: Vec<String>,
}

/// Eine Sicherung unter `sicherungen/`.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct LibraryBackup {
    pub name: String,
    /// Erstellt am (Unix-ms).
    pub erstellt_ms: f64,
    pub dateien: u32,
    pub bytes: f64,
}

/// Ergebnis von „Sicherung zurückspielen“.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct LibraryBackupRestore {
    pub dateien: u32,
    /// Papierkorb mit den Zielen, die weichen mussten (relativ zum Datenordner).
    pub papierkorb: Option<String>,
}

fn problem(p: Problem) -> LibraryLayoutProblem {
    LibraryLayoutProblem {
        seite: p.seite,
        art: p.art.name().to_string(),
        meldung: p.meldung,
        pfade: p.pfade,
    }
}

fn report(b: Bericht) -> LibraryLayoutReport {
    LibraryLayoutReport {
        phase: b.phase.map(|p| p.name().to_string()),
        erledigt: b.erledigt as u32,
        angehalten: b.angehalten,
        probleme: b.probleme.into_iter().map(problem).collect(),
        hinweise: b.hinweise,
    }
}

// ---------------------------------------------------------------------------
// Befehle
// ---------------------------------------------------------------------------

/// Läuft ein Hintergrundauftrag, der Seiten liest oder schreibt? Dann wird
/// der Start verschoben (Hinweis an die Oberfläche).
fn hintergrund(engine: &SyncEngine) -> Result<(), String> {
    let laufend = if crate::book_ai::is_running() {
        Some("ein KI-Lauf im Buch")
    } else if crate::book_ai::talk::is_running() {
        Some("ein Gespräch mit dem Buch")
    } else if engine.status().running {
        Some("der Geräte-Sync")
    } else {
        None
    };
    match laufend {
        Some(was) => Err(format!(
            "Gerade läuft {was}. Bitte warten, bis er fertig ist, und dann erneut starten."
        )),
        None => Ok(()),
    }
}

/// Unter Bibliotheks- und Seitensperre ausführen und danach die Fundortkarte
/// neu aufbauen. Der Aufrufer hält die Sync-Laufsperre.
fn gesperrt<T>(
    app: &AppHandle,
    lauf: impl FnOnce(&Path) -> Result<T, String>,
) -> Result<T, String> {
    let data = pages::data_root(app)?;
    // Erst den Merker, dann die Sperren: Leser warten nicht auf den Umbau,
    // sondern melden ihn (sonst „Keine Rückmeldung“ auf dem Hauptthread).
    let _umbau = pages::umbau_beginnen();
    let _library = crate::library::sync_lock();
    let _pages = pages::lock();
    let ergebnis = lauf(&data);
    fundortkarte_neu(&data);
    // Seitenordner können umgezogen sein: die Oberfläche holt gemerkte Pfade
    // neu, statt unter dem alten Ort einen zweiten Ordner anzulegen.
    let _ = app.emit("library-layout-changed", ());
    ergebnis
}

/// Layout, Journalphase, offene Probleme, Anzahl der zu verschiebenden Seiten
/// und nötiger Platz.
#[tauri::command]
#[specta::specta]
pub async fn library_layout_status(app: AppHandle) -> Result<LibraryLayoutStatus, String> {
    // Abseits des Hauptthreads: während einer Migration wartet die Abfrage
    // auf die Sperren, ohne die Oberfläche einzufrieren.
    tauri::async_runtime::spawn_blocking(move || status_gesperrt(&app))
        .await
        .map_err(|e| e.to_string())?
}

fn status_gesperrt(app: &AppHandle) -> Result<LibraryLayoutStatus, String> {
    let data = pages::data_root(app)?;
    let _library = crate::library::sync_lock();
    let _pages = pages::lock();
    let s = library_layout::status(&data, &freier_platz);
    Ok(LibraryLayoutStatus {
        layout: s.layout,
        phase: s.phase.map(|p| p.name().to_string()),
        probleme: s.probleme.into_iter().map(problem).collect(),
        hinweise: s.hinweise,
        zu_verschieben: s.zu_verschieben as u32,
        platz_benoetigt: s.platz_benoetigt as f64,
        platz_frei: s.platz_frei.map(|f| f as f64),
    })
}

/// Bücher in Buch-Ordner umziehen (Sicherung → Journal → Schritte). Startet
/// nur, wenn kein Hintergrundauftrag läuft; hält Sync-Lauf-, Bibliotheks- und
/// Seitensperre über den ganzen Lauf.
#[tauri::command]
#[specta::specta]
pub async fn library_layout_migrate(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
) -> Result<LibraryLayoutReport, String> {
    hintergrund(&engine)?;
    let _run = engine.account_guard().await;
    hintergrund(&engine)?;
    tauri::async_runtime::spawn_blocking(move || {
        gesperrt(&app, |data| library_layout::migrieren(data, &optionen()))
    })
    .await
    .map_err(|e| e.to_string())?
    .map(report)
}

/// Rückfall auf Wunsch: dieselben Schritte rückwärts, ebenfalls im Journal.
#[tauri::command]
#[specta::specta]
pub async fn library_layout_rollback(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
) -> Result<LibraryLayoutReport, String> {
    hintergrund(&engine)?;
    let _run = engine.account_guard().await;
    hintergrund(&engine)?;
    tauri::async_runtime::spawn_blocking(move || gesperrt(&app, library_layout::rueckfall))
        .await
        .map_err(|e| e.to_string())?
        .map(report)
}

/// Vorhandene Sicherungen der Buch-Ordner-Migration, neueste zuerst.
#[tauri::command]
#[specta::specta]
pub fn library_backup_list(app: AppHandle) -> Result<Vec<LibraryBackup>, String> {
    let data = pages::data_root(&app)?;
    Ok(library_layout::sicherungen_auflisten(&data)
        .into_iter()
        .map(|s| LibraryBackup {
            name: s.name,
            erstellt_ms: s.erstellt_ms as f64,
            dateien: s.dateien as u32,
            bytes: s.bytes as f64,
        })
        .collect())
}

/// Sicherung zurückspielen: erst gegen das Manifest prüfen, dann jedes
/// vorhandene Ziel in den Papierkorb (`sicherungen/.papierkorb/`), dann den
/// Stand des Manifests anlegen. Nichts wird überschrieben.
#[tauri::command]
#[specta::specta]
pub async fn library_backup_restore(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
    name: String,
) -> Result<LibraryBackupRestore, String> {
    hintergrund(&engine)?;
    let _run = engine.account_guard().await;
    hintergrund(&engine)?;
    tauri::async_runtime::spawn_blocking(move || {
        gesperrt(&app, |data| {
            library_layout::sicherung_zurueckspielen(data, &name, freier_platz(data))
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map(|w| LibraryBackupRestore {
        dateien: w.dateien as u32,
        papierkorb: w.papierkorb,
    })
}
