//! Modelle der Sprechertrennung (Katalog `Purpose::Diarization`): Stand
//! abfragen und auf Knopfdruck laden.
//!
//! Bis 05.11.2026 gab es dafuer keinen Weg in der App: das Modell wird
//! bewusst nicht mitgeliefert (NVIDIA Open Model License, 139 MB, jedes Update
//! waere sonst so viel groesser), aber es liess sich auch nirgends laden. Jede
//! Besprechung zeigte deshalb nur "Modell zur Sprechertrennung ist nicht
//! installiert". Abgelegt wird es dort, wo `diarize::resolve_model_path` sucht.
//!
//! N1: Ein Eintrag kann aus mehreren Dateien bestehen (Nemotron: ONNX-Graph und
//! Gewichte, die nebeneinander liegen muessen). Der Ladeweg arbeitet je Kennung
//! ([`download_model`], [`model_status`]); die Tauri-Befehle bleiben auf dem
//! Standardmodell (Sortformer, unveraenderte Signatur).
//!
//! Fehlerfaelle des Ladens:
//! - Abbruch/Absturz mitten in einer Datei: sie liegt als `<name>.partial`
//!   neben dem Ziel und wird beim naechsten Aufruf fortgesetzt; umbenannt wird
//!   erst nach bestandener Groessen- und SHA-256-Pruefung, das Ziel ist also nie
//!   halb geschrieben;
//! - vorhandene Dateien (abgeschnitten, beschaedigt, von Hand hineinkopiert):
//!   das Bereitstellen ([`download_model`], damit auch `--provision`) prueft JEDE
//!   Datei des Eintrags nach Groesse und SHA-256 und ersetzt jede ungueltige
//!   durch einen verifizierten Download (Umbenennen ueber das Ziel, die alte
//!   Datei bleibt bis dahin liegen). Nur wenn alle gueltig sind, wird nichts
//!   geladen. Die Statusabfrage der Oberflaeche hasht dagegen nie (bei jedem
//!   Oeffnen waeren das 120 MB Lesen): sie prueft Vorhandensein und exakte
//!   Groesse, das faengt abgeschnittene Dateien. Gleich grosse, aber
//!   beschaedigte Bytes findet erst das Bereitstellen; eine Pruefmarke haette
//!   dieselbe Luecke (sie belegt nur, dass die Datei einmal gut war) und liesse
//!   Altbestaende ohne Marke als „nicht installiert“ erscheinen;
//! - Reihenfolge: die Datei, die der Motor oeffnet (die erste des Eintrags),
//!   kommt zuletzt; sie liegt nur da, wenn alle anderen verifiziert liegen. Muss
//!   eine andere Datei ersetzt werden, wird sie vorher zurueckgezogen (nicht
//!   geloescht, sondern als `.partial` abgelegt, damit eine gueltige Datei nicht
//!   neu geladen werden muss), der Motor bekommt also nie einen Graphen neben
//!   ungeprueften oder defekten Gewichten, auch nicht, wenn die Reparatur
//!   scheitert;
//! - Ende nur mit verifiziertem Paar: unmittelbar vor dem Erfolg werden ALLE
//!   Dateien des Eintrags erneut nach Groesse und SHA-256 geprueft ([`confirm`]).
//!   Wurde dazwischen eine Datei getauscht (auch eine, die schon verifiziert
//!   dalag), gibt es keinen Erfolg: Fehler `verification_failed`, der Graph wird
//!   zurueckgezogen. Dasselbe bei Abbruch oder Lesefehler in dieser letzten
//!   Pruefung. Erfolg heisst also: zum Zeitpunkt des Erfolgs stimmten alle Dateien.
//!   Ein Tausch NACH dem Erfolg (zwischen Bereitstellen und Laden) bleibt moeglich;
//!   der Lader prueft dort nur die Groessen (`nemotron::engine::check_catalog_sizes`);
//! - Verknuepfungen: das Bereitstellen aendert nur eigene, normale Dateien
//!   unterhalb des Modellordners. Ein Symlink (unter Windows jeder Reparse-Punkt,
//!   also auch Junctions) als Ziel oder als `.partial`, ein Ordner dazwischen, der
//!   ein Link ist oder aus dem Modellordner hinauszeigt, oder etwas, das keine
//!   normale Datei ist, wird mit dem Fehler `unsafe_path` abgelehnt, bevor
//!   geschrieben, umbenannt oder geloescht wird (`download::safe`). Die Wurzel
//!   selbst darf ein Link sein (verschobener Datenordner). Ein Katalogwert, der
//!   aus dem Modellordner hinausfuehren koennte (absoluter Pfad, `..`,
//!   Backslash, Laufwerksbuchstabe), ergibt `catalog_invalid`;
//! - voller Datentraeger: der Schreibfehler bricht ab, die Teildatei bleibt fuer
//!   die Fortsetzung, bereits fertige Dateien bleiben liegen;
//! - zwei Aufrufe gleichzeitig: einer laeuft, der andere meldet `download_running`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use hf_hub::api::tokio::CancellationToken;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter};

use crate::catalog::{TtsCatalogEntry, TtsCatalogFile};
use crate::managers::meetings::diarize;
use crate::managers::model::download::{is_refused, safe, HttpDownloadEvent, HttpDownloadOutcome};
use crate::managers::model::ModelManager;

#[derive(Debug, Clone, Serialize, Type)]
pub struct DiarizationModelStatus {
    /// Katalogkennung (fuer die Fortschrittsereignisse `model-download-*`).
    pub model_id: String,
    pub model_ready: bool,
    pub downloading: bool,
    pub size_mb: u32,
}

static DOWNLOADING: AtomicBool = AtomicBool::new(false);
/// Abbruch des laufenden Downloads (Knopf „Abbrechen“).
static CANCEL: std::sync::Mutex<Option<CancellationToken>> = std::sync::Mutex::new(None);

fn models_root(app: &AppHandle) -> PathBuf {
    crate::portable::app_data_dir(app)
        .map(|d| d.join("models"))
        .unwrap_or_default()
}

fn catalog_entry(id: &str) -> Option<TtsCatalogEntry> {
    crate::catalog::tts_entries(crate::catalog::Purpose::Diarization)
        .into_iter()
        .find(|e| e.id == id)
}

/// Die Dateien eines Eintrags mit ihrem Zielpfad, in Katalogreihenfolge.
/// `catalog_invalid`, wenn ein Katalogwert (Dateiname, Kennung) aus dem
/// Modellordner hinausfuehren koennte.
fn targets(
    models_root: &Path,
    entry: &TtsCatalogEntry,
) -> Result<Vec<(TtsCatalogFile, PathBuf)>, String> {
    let dir = diarize::catalog_model_dir(models_root, entry)
        .ok_or_else(|| "catalog_invalid".to_string())?;
    Ok(entry
        .files
        .iter()
        .map(|f| (f.clone(), dir.join(&f.filename)))
        .collect())
}

/// Fehlertext eines E/A-Fehlers; eine Ablehnung wegen Verknuepfung heisst
/// `unsafe_path` (kein Pfad im Text).
fn io_msg(e: &std::io::Error) -> String {
    if safe::is_refusal(e) {
        "unsafe_path".to_string()
    } else {
        e.to_string()
    }
}

/// Wie [`io_msg`] fuer Fehler des Downloaders.
fn net_msg(e: &anyhow::Error) -> String {
    if is_refused(e) {
        "unsafe_path".to_string()
    } else {
        e.to_string()
    }
}

/// Bereit (fuer die Anzeige) heisst: alle Dateien des Eintrags liegen an ihrem
/// Platz als normale Dateien (kein Link) und haben genau die Groesse aus dem
/// Katalog. Nur `stat`, kein Lesen der Inhalte; die volle Pruefung (SHA-256)
/// macht [`provision`].
fn looks_complete(targets: &[(TtsCatalogFile, PathBuf)]) -> bool {
    !targets.is_empty()
        && targets
            .iter()
            .all(|(file, path)| safe::plain_len(path).is_ok_and(|len| len == Some(file.size_bytes)))
}

fn status_for(app: &AppHandle, id: &str) -> Result<DiarizationModelStatus, String> {
    let entry = catalog_entry(id).ok_or_else(|| "model_missing".to_string())?;
    let size: u64 = entry.files.iter().map(|f| f.size_bytes).sum();
    Ok(DiarizationModelStatus {
        model_id: entry.id.clone(),
        model_ready: looks_complete(&targets(&models_root(app), &entry)?),
        downloading: DOWNLOADING.load(Ordering::Relaxed),
        size_mb: size.div_ceil(1_000_000).min(u64::from(u32::MAX)) as u32,
    })
}

/// Stand des Modells `id` (Kennung aus dem Katalog, Zweck Sprechertrennung).
pub fn model_status(app: &AppHandle, id: &str) -> Result<DiarizationModelStatus, String> {
    status_for(app, id)
}

#[tauri::command]
#[specta::specta]
pub fn meeting_diarization_model_status(app: AppHandle) -> Result<DiarizationModelStatus, String> {
    status_for(&app, diarize::DEFAULT_MODEL_ID)
}

/// Wie ein Download endete: Abbruch ist ein Ergebnis, kein Fehler (die
/// Teildatei bleibt fuer die Fortsetzung).
#[derive(Debug, PartialEq, Eq)]
enum Fetch {
    Done,
    Cancelled,
}

/// Die fortsetzbare Teildatei neben dem Ziel.
fn partial_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_owned();
    name.push(".partial");
    PathBuf::from(name)
}

/// Zustand einer Datei am Ziel gegenueber dem Katalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileState {
    Valid,
    Missing,
    /// Liegt da, hat aber nicht die Groesse oder die SHA-256 aus dem Katalog.
    Invalid,
}

/// Prueft die Datei am Ziel: erst die Groesse (billig), dann die SHA-256 ueber
/// den ganzen Inhalt, gelesen in 1-MiB-Bloecken (kein Speicherbedarf nach der
/// Dateigroesse). Ohne Pruefsumme im Katalog zaehlt nur die Groesse. `None`:
/// waehrend des Lesens abgebrochen. Ein Link oder eine Nicht-Datei am Ziel ist
/// ein Fehler (Ablehnung, [`safe::is_refusal`]), nie eine Datei, der gefolgt wird;
/// gelesen wird ohne Link zu folgen.
fn check_file(
    file: &TtsCatalogFile,
    dest: &Path,
    cancel: &CancellationToken,
) -> std::io::Result<Option<FileState>> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let Some(len) = safe::plain_len(dest)? else {
        return Ok(Some(FileState::Missing));
    };
    if len != file.size_bytes {
        return Ok(Some(FileState::Invalid));
    }
    let Some(expected) = file.sha256.as_deref() else {
        return Ok(Some(FileState::Valid));
    };
    let mut reader = safe::open_read(dest)?;
    let mut hasher = Sha256::new();
    let mut block = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let n = reader.read(&mut block)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&block[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    // Gewachsen oder geschrumpft zwischen `stat` und Lesen: nicht die Datei aus dem Katalog.
    Ok(Some(
        if total == file.size_bytes && actual.eq_ignore_ascii_case(expected) {
            FileState::Valid
        } else {
            FileState::Invalid
        },
    ))
}

/// Prueft alle Dateien ([`check_file`]), im Hintergrundthread. `None`: abgebrochen.
async fn verify_all(
    targets: &[(TtsCatalogFile, PathBuf)],
    cancel: &CancellationToken,
) -> Result<Option<Vec<FileState>>, String> {
    let mut states = Vec::with_capacity(targets.len());
    for (file, dest) in targets {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let (file, dest, token) = (file.clone(), dest.clone(), cancel.clone());
        let state = tokio::task::spawn_blocking(move || check_file(&file, &dest, &token))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| io_msg(&e))?;
        match state {
            Some(state) => states.push(state),
            None => return Ok(None),
        }
    }
    Ok(Some(states))
}

/// Zieht die Datei zurueck, die der Motor oeffnet (die erste des Eintrags), damit
/// er sie nicht neben ungeprueften oder defekten Dateien findet. Ungueltig:
/// geloescht; fehlend: nichts zu tun. Gueltig oder unbekannt (`None`, etwa nach
/// Abbruch oder Lesefehler): als `.partial` ablegen (kommt ohne Netz zurueck, der
/// Downloader prueft sie dann erneut); geht das nicht (ein Link, ein Hindernis,
/// fehlende Rechte), die Datei selbst entfernen; ein Link wird dabei ohne
/// Folgen entfernt, nie sein Ziel. Scheitert auch das, ist es ein Fehler: der
/// Aufrufer meldet dann keinen Erfolg.
fn withdraw_graph(entry_dest: &Path, state: Option<FileState>) -> Result<(), String> {
    match state {
        Some(FileState::Missing) => Ok(()),
        Some(FileState::Invalid) => safe::remove(entry_dest).map_err(|e| io_msg(&e)),
        Some(FileState::Valid) | None => {
            match safe::rename_over(entry_dest, &partial_path(entry_dest)) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => match std::fs::remove_file(entry_dest) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(io_msg(&e)),
                },
            }
        }
    }
}

/// Die letzte Pruefung vor dem Erfolg: alle Dateien des Eintrags erneut nach
/// Groesse und SHA-256, jetzt, nachdem alles an seinem Platz liegt. Stimmt eine
/// nicht (getauscht, beschaedigt, durch einen Link ersetzt) oder wird die Pruefung
/// abgebrochen, gibt es keinen Erfolg und der Graph wird zurueckgezogen
/// ([`withdraw_graph`]): der Motor findet nie ein Paar, das nicht gerade
/// verifiziert war. `verification_failed`, wenn der Inhalt nicht stimmt.
async fn confirm(
    targets: &[(TtsCatalogFile, PathBuf)],
    cancel: &CancellationToken,
) -> Result<Fetch, String> {
    let outcome = verify_all(targets, cancel).await;
    if let Ok(Some(states)) = &outcome {
        if states.iter().all(|s| *s == FileState::Valid) {
            return Ok(Fetch::Done);
        }
    }
    let graph_state = match &outcome {
        Ok(Some(states)) => states.first().copied(),
        _ => None,
    };
    let reason = match &outcome {
        Ok(None) => None,
        Ok(Some(_)) => Some("verification_failed".to_string()),
        Err(e) => Some(e.clone()),
    };
    if let Err(w) = withdraw_graph(&targets[0].1, graph_state) {
        return Err(format!(
            "{}; graph_not_withdrawn: {w}",
            reason.as_deref().unwrap_or("cancelled")
        ));
    }
    match reason {
        None => Ok(Fetch::Cancelled),
        Some(reason) => Err(reason),
    }
}

/// Stellt alle Dateien des Eintrags verifiziert bereit.
///
/// 0. Jeder Ordner der Ziele liegt unterhalb von `root` ohne Link, der
///    hinauszeigt ([`safe::dir_inside`]); jede Datei am Ziel und jede
///    `.partial` ist eine eigene normale Datei (`unsafe_path` sonst).
/// 1. Jede vorhandene Datei wird nach Groesse und SHA-256 geprueft. Sind alle
///    gueltig, ist nichts zu tun (kein Netz).
/// 2. Muss eine andere als die erste Datei ersetzt werden, wird die erste (die
///    der Motor oeffnet) zurueckgezogen: als `<ziel>.partial` abgelegt, ein
///    gueltiger Graph wird dadurch ohne Netz wieder uebernommen.
/// 3. Jede fehlende oder ungueltige Datei wird nach `<ziel>.partial` geladen
///    (fortsetzbar, Groesse und SHA-256 aus dem Katalog geprueft) und erst
///    dann ueber das Ziel umbenannt; die erste Datei kommt zuletzt.
/// 4. Zuletzt pruefen alle Dateien noch einmal ([`confirm`]); nur dann Erfolg.
async fn provision(
    root: &Path,
    model_id: &str,
    targets: &[(TtsCatalogFile, PathBuf)],
    cancel: &CancellationToken,
    emit: &(dyn Fn(HttpDownloadEvent<'_>) + Send + Sync),
) -> Result<Fetch, String> {
    if targets.is_empty() {
        return Err("model_missing".into());
    }
    for (_, dest) in targets {
        let dir = dest.parent().ok_or_else(|| "model_missing".to_string())?;
        safe::dir_inside(root, dir, false).map_err(|e| io_msg(&e))?;
    }
    let Some(mut states) = verify_all(targets, cancel).await? else {
        return Ok(Fetch::Cancelled);
    };
    if states.iter().all(|s| *s == FileState::Valid) {
        return Ok(Fetch::Done);
    }
    // Die erste Datei oeffnet der Motor: sie darf nicht neben Dateien liegen, die
    // gleich ersetzt werden. Eine gueltige wandert als Teildatei beiseite (kommt
    // ohne Netz zurueck), eine ungueltige wird ohnehin neu geladen.
    let entry_dest = &targets[0].1;
    let others_broken = states[1..].iter().any(|s| *s != FileState::Valid);
    if others_broken && states[0] != FileState::Missing {
        withdraw_graph(entry_dest, Some(states[0]))?;
        states[0] = FileState::Missing;
    }
    for ((file, dest), state) in targets.iter().zip(&states).rev() {
        if *state == FileState::Valid {
            continue;
        }
        if cancel.is_cancelled() {
            return Ok(Fetch::Cancelled);
        }
        let dir = dest.parent().ok_or_else(|| "model_missing".to_string())?;
        safe::dir_inside(root, dir, true).map_err(|e| io_msg(&e))?;
        let partial = partial_path(dest);
        let outcome = ModelManager::download_http_resumable_with_events(
            model_id,
            &file.url,
            &partial,
            Some(file.size_bytes),
            file.sha256.as_deref(),
            cancel,
            emit,
        )
        .await
        .map_err(|e| net_msg(&e))?;
        match outcome {
            HttpDownloadOutcome::Completed => {
                safe::rename_over(&partial, dest).map_err(|e| io_msg(&e))?;
            }
            HttpDownloadOutcome::Cancelled => return Ok(Fetch::Cancelled),
        }
    }
    confirm(targets, cancel).await
}

/// Stellt das Modell `id` bereit (alle seine Dateien), nur auf Knopfdruck oder
/// `--provision`: prueft jede vorhandene Datei nach Groesse und SHA-256, ersetzt
/// ungueltige und laedt fehlende (siehe [`provision`]). Fortschritt ueber
/// `model-download-progress`, Ende ueber `model-download-complete`. Wirkt ab der
/// naechsten Sprechertrennung.
pub async fn download_model(app: &AppHandle, id: &str) -> Result<(), String> {
    let entry = catalog_entry(id).ok_or_else(|| "model_missing".to_string())?;
    let root = models_root(app);
    if root.as_os_str().is_empty() {
        return Err("data_dir_missing".into());
    }
    let files = targets(&root, &entry)?;
    if files.is_empty() {
        return Err("model_missing".into());
    }
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Err("download_running".into());
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            *CANCEL.lock().unwrap_or_else(|e| e.into_inner()) = None;
            DOWNLOADING.store(false, Ordering::SeqCst);
        }
    }
    let _reset = Reset;
    let handle = app.clone();
    let emit = move |event: HttpDownloadEvent<'_>| {
        if let HttpDownloadEvent::Progress(p) = event {
            let _ = handle.emit("model-download-progress", p);
        }
    };
    let cancel = CancellationToken::new();
    *CANCEL.lock().unwrap_or_else(|e| e.into_inner()) = Some(cancel.clone());
    match provision(&root, &entry.id, &files, &cancel, &emit).await? {
        Fetch::Done => {
            let _ = app.emit("model-download-complete", entry.id.as_str());
            Ok(())
        }
        Fetch::Cancelled => Err("cancelled".into()),
    }
}

/// Laedt das Standardmodell (Sortformer), nur auf Knopfdruck (oder
/// `--provision`). Eine vorhandene Datei wird vorher nach Groesse und SHA-256
/// geprueft und bei Abweichung ersetzt. Fortschritt ueber
/// `model-download-progress`, Ende ueber `model-download-complete`. Wirkt ab der
/// naechsten Sprechertrennung (Stopp, Import, Neu-Transkription).
#[tauri::command]
#[specta::specta]
pub async fn meeting_diarization_model_download(app: AppHandle) -> Result<(), String> {
    download_model(&app, diarize::DEFAULT_MODEL_ID).await
}

/// Bricht einen laufenden Download ab; die Teildatei bleibt fuer die
/// Fortsetzung liegen. Ohne laufenden Download ohne Wirkung.
#[tauri::command]
#[specta::specta]
pub fn meeting_diarization_model_cancel() -> Result<(), String> {
    if let Some(token) = CANCEL.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        token.cancel();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Die Kennung, die der Knopf laedt, ist genau die, die die
    /// Sprechertrennung sucht, und sie steht mit Datei und Pruefsumme im Katalog.
    #[test]
    fn the_downloadable_entry_is_the_one_diarization_looks_for() {
        let entry = catalog_entry(diarize::DEFAULT_MODEL_ID).expect("Sortformer steht im Katalog");
        let file = entry.files.first().expect("eine Datei");
        assert!(file.sha256.is_some(), "Download braucht eine Pruefsumme");
        assert!(file.size_bytes > 0);
        let root = std::path::Path::new("models");
        assert_eq!(
            diarize::catalog_model_path(root, &entry.id),
            Some(diarize::resolve_model_path(root, None))
        );
    }

    /// Nemotron: beide Dateien in einem eigenen Ordner, der Graph (die Datei,
    /// die der Motor oeffnet) ist die erste, damit kommt er beim Laden zuletzt.
    #[test]
    fn the_nemotron_entry_has_two_targets_in_its_own_folder_graph_first() {
        let root = Path::new("models");
        let entry = catalog_entry(diarize::nemotron::MODEL_ID).expect("Nemotron im Katalog");
        let t = targets(root, &entry).unwrap();
        assert_eq!(t.len(), 2);
        let dir = root.join("diarization").join(&entry.id);
        assert_eq!(t[0].1, dir.join("model_quantized.onnx"));
        assert_eq!(t[1].1, dir.join("model_quantized.onnx_data"));
        assert_eq!(
            diarize::catalog_model_path(root, &entry.id).as_ref(),
            Some(&t[0].1)
        );
        // Sortformer bleibt, wo es war (Altbestand).
        let old = catalog_entry(diarize::DEFAULT_MODEL_ID).unwrap();
        let t = targets(root, &old).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].1.parent().unwrap(), root.join("diarization"));
    }

    // ---- Ladeweg gegen einen lokalen Server --------------------------------

    fn sha_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Kleiner HTTP-Server: antwortet je Pfad mit dem Inhalt oder 500, und
    /// merkt sich die Reihenfolge der Anfragen.
    struct Server {
        base: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    async fn serve(routes: Vec<(&'static str, Option<Vec<u8>>)>) -> Server {
        serve_with(routes, Arc::new(|_| {})).await
    }

    /// Wie [`serve`]; `on_request` laeuft bei jeder Anfrage, bevor geantwortet wird
    /// (Tests schauen darin in den Ordner).
    async fn serve_with(
        routes: Vec<(&'static str, Option<Vec<u8>>)>,
        on_request: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (routes, log, on_request) = (routes.clone(), log.clone(), on_request.clone());
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if sock.read_exact(&mut byte).await.is_err() {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    log.lock().unwrap().push(path.clone());
                    on_request(&path);
                    let body = routes
                        .iter()
                        .find(|(p, _)| *p == path)
                        .and_then(|(_, b)| b.clone());
                    let response = match body {
                        Some(b) => {
                            let mut r = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                b.len()
                            )
                            .into_bytes();
                            r.extend(b);
                            r
                        }
                        None => b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec(),
                    };
                    let _ = sock.write_all(&response).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Server { base, requests }
    }

    const GRAPH: &[u8] = b"graph-bytes";
    const WEIGHTS: &[u8] = b"weights-bytes-weights-bytes";

    /// Zwei Dateien wie bei Nemotron: Graph zuerst, Gewichte danach.
    fn two_files(server: &Server, weights_sha: Option<String>) -> Vec<(TtsCatalogFile, PathBuf)> {
        let dir = PathBuf::from("zielordner");
        let file = |name: &str, body: &[u8], sha: Option<String>| TtsCatalogFile {
            filename: name.to_string(),
            url: format!("{}/{name}", server.base),
            size_bytes: body.len() as u64,
            sha256: Some(sha.unwrap_or_else(|| sha_hex(body))),
        };
        vec![
            (file("model.onnx", GRAPH, None), dir.join("model.onnx")),
            (
                file("model.onnx_data", WEIGHTS, weights_sha),
                dir.join("model.onnx_data"),
            ),
        ]
    }

    fn rebase(t: Vec<(TtsCatalogFile, PathBuf)>, dir: &Path) -> Vec<(TtsCatalogFile, PathBuf)> {
        t.into_iter()
            .map(|(f, p)| {
                let dest = dir.join(p.file_name().unwrap());
                (f, dest)
            })
            .collect()
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn both_files_arrive_graph_last_and_no_partial_is_left() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        assert!(!looks_complete(&files));
        let out = provision(
            dir.path(),
            "test/nemotron",
            &files,
            &CancellationToken::new(),
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(looks_complete(&files));
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
        // Die Gewichte zuerst, der Graph (der, den der Motor oeffnet) zuletzt.
        assert_eq!(
            *server.requests.lock().unwrap(),
            ["/model.onnx_data", "/model.onnx"]
        );
    }

    #[tokio::test]
    async fn a_second_run_asks_the_server_for_nothing_and_resumes_after_a_failed_file() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        let asked = server.requests.lock().unwrap().len();
        provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(
            server.requests.lock().unwrap().len(),
            asked,
            "Nachladen trotz vollstaendiger Dateien"
        );

        // Eine fehlende Datei wird einzeln nachgeladen, die vorhandene bleibt.
        std::fs::remove_file(&files[0].1).unwrap();
        provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(server.requests.lock().unwrap().len(), asked + 1);
        assert_eq!(
            *server.requests.lock().unwrap().last().unwrap(),
            "/model.onnx"
        );
        assert!(looks_complete(&files));
    }

    /// Faellt die Gewichte-Datei aus (Server 500), gibt es den Graphen nicht:
    /// das Modell gilt nicht als installiert, und der Motor bekommt nie einen
    /// Graphen ohne Gewichte.
    #[tokio::test]
    async fn a_failing_weights_download_leaves_no_graph_behind() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", None),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        let err = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap_err();
        assert!(!err.is_empty());
        assert!(!files[0].1.exists(), "Graph ohne Gewichte");
        assert!(!looks_complete(&files));
        assert!(!leftovers(dir.path()).iter().any(|n| n == "model.onnx_data"));
    }

    /// Falsche Pruefsumme: die Datei wird nicht umbenannt, der Graph nicht geladen.
    #[tokio::test]
    async fn a_checksum_mismatch_installs_nothing() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, Some("0".repeat(64))), dir.path());
        assert!(
            provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .is_err()
        );
        assert!(!files[0].1.exists() && !files[1].1.exists());
        assert!(!looks_complete(&files));
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx_data"]);
    }

    #[tokio::test]
    async fn cancelling_before_the_start_downloads_nothing() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        let cancel = CancellationToken::new();
        cancel.cancel();
        let out = provision(dir.path(), "t", &files, &cancel, &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Cancelled);
        assert!(server.requests.lock().unwrap().is_empty());
        assert!(!looks_complete(&files));
    }

    /// Eine abgebrochene Teildatei (Absturz mitten im Laden) bleibt als
    /// `.partial` liegen und zaehlt nicht als installiert; ein vollstaendig
    /// geschriebener, aber nie umbenannter Rest wird geprueft und uebernommen.
    #[tokio::test]
    async fn a_leftover_partial_is_not_an_installed_file_and_a_full_one_is_adopted() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        // Nach einem Absturz: Gewichte komplett als .partial (Rename fehlte), Graph nicht begonnen.
        std::fs::write(dir.path().join("model.onnx_data.partial"), WEIGHTS).unwrap();
        assert!(
            !looks_complete(&files),
            "Teildatei darf nicht als installiert gelten"
        );
        provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert!(looks_complete(&files));
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        // Die volle Teildatei wurde nur geprueft, nicht erneut geladen.
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx"]);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    // ---- Vorhandene Dateien: pruefen, ersetzen, nie Graph neben defekten Gewichten

    /// Die Dateien so, wie sie nach erfolgreichem Laden liegen.
    fn install(files: &[(TtsCatalogFile, PathBuf)]) {
        std::fs::write(&files[0].1, GRAPH).unwrap();
        std::fs::write(&files[1].1, WEIGHTS).unwrap();
    }

    fn both_routes() -> Vec<(&'static str, Option<Vec<u8>>)> {
        vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ]
    }

    /// Alles vorhanden und gueltig: kein Netzzugriff, nichts angefasst.
    #[tokio::test]
    async fn valid_existing_files_are_verified_and_nothing_is_downloaded() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        assert!(looks_complete(&files));
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(server.requests.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    /// Abgeschnittene Gewichte neben vollstaendigem Graph: die Statusabfrage
    /// sieht es an der Groesse, das Bereitstellen repariert nur die Gewichte, und
    /// waehrend sie geladen werden, liegt der Graph nicht am Ziel.
    #[tokio::test]
    async fn truncated_weights_next_to_a_complete_graph_are_repaired_without_exposing_the_graph() {
        let dir = tempfile::tempdir().unwrap();
        let graph = dir.path().join("model.onnx");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hook = {
            let seen = seen.clone();
            Arc::new(move |path: &str| {
                seen.lock()
                    .unwrap()
                    .push((path.to_string(), graph.exists()))
            })
        };
        let server = serve_with(both_routes(), hook).await;
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        std::fs::write(&files[1].1, &WEIGHTS[..5]).unwrap();
        assert!(
            !looks_complete(&files),
            "abgeschnitten gilt nicht als bereit"
        );

        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(looks_complete(&files));
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        // Nur die Gewichte aus dem Netz; der gueltige Graph kam aus der Rueckstellung.
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx_data"]);
        assert_eq!(
            *seen.lock().unwrap(),
            [("/model.onnx_data".to_string(), false)],
            "Graph lag waehrend des Ladens der Gewichte am Ziel"
        );
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    /// Graph fehlt, Gewichte haben die richtige Groesse, aber falsche Bytes (nur
    /// die SHA-256 sieht das): Gewichte ersetzt, dann der Graph.
    #[tokio::test]
    async fn corrupt_weights_of_the_right_size_are_replaced_then_the_graph_follows() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        std::fs::write(&files[1].1, vec![b'x'; WEIGHTS.len()]).unwrap();
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(
            *server.requests.lock().unwrap(),
            ["/model.onnx_data", "/model.onnx"]
        );
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    /// Der Vertrag zwischen Anzeige und Bereitstellen: die Anzeige schaut nur auf
    /// die Groesse (kein Hashlauf je Abfrage), das Bereitstellen auf den Inhalt.
    #[tokio::test]
    async fn the_status_checks_sizes_and_provisioning_checks_content() {
        assert!(!looks_complete(&[]), "ohne Dateien nie bereit");
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        std::fs::write(&files[1].1, vec![b'x'; WEIGHTS.len()]).unwrap();
        assert!(looks_complete(&files), "die Anzeige liest keine Inhalte");
        provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx_data"]);
    }

    /// Scheitert die Reparatur der Gewichte, bleibt der Graph zurueckgezogen
    /// (nicht am Ziel); die naechste Runde holt nur die Gewichte nach.
    #[tokio::test]
    async fn a_failed_repair_never_leaves_the_graph_next_to_defective_weights() {
        let dir = tempfile::tempdir().unwrap();
        let broken = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", None),
        ])
        .await;
        let files = rebase(two_files(&broken, None), dir.path());
        install(&files);
        std::fs::write(&files[1].1, &WEIGHTS[..5]).unwrap();
        assert!(
            provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .is_err()
        );
        assert!(!files[0].1.exists(), "Graph neben defekten Gewichten");
        assert!(!looks_complete(&files));
        assert_eq!(
            std::fs::read(dir.path().join("model.onnx.partial")).unwrap(),
            GRAPH,
            "der gueltige Graph liegt als Teildatei bereit"
        );

        let healthy = serve(both_routes()).await;
        let files = rebase(two_files(&healthy, None), dir.path());
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert_eq!(*healthy.requests.lock().unwrap(), ["/model.onnx_data"]);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    /// Eine einzelne Datei (Sortformer): eine ungueltige wird ersetzt, aber erst
    /// wenn die neue verifiziert ist; scheitert das Laden, bleibt die alte liegen
    /// (nicht schlechter als vorher).
    #[tokio::test]
    async fn a_single_invalid_file_is_replaced_only_by_a_verified_download() {
        let dir = tempfile::tempdir().unwrap();
        let one = |server: &Server| rebase(two_files(server, None), dir.path()).remove(0);
        let down = serve(vec![("/model.onnx", None)]).await;
        let files = vec![one(&down)];
        std::fs::write(&files[0].1, b"alte-datei-bytes").unwrap();
        assert!(!looks_complete(&files));
        assert!(
            provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&files[0].1).unwrap(), b"alte-datei-bytes");

        let up = serve(both_routes()).await;
        let files = vec![one(&up)];
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert!(looks_complete(&files));
        assert_eq!(leftovers(dir.path()), ["model.onnx"]);
    }

    /// Abbruch vor dem Start laesst vorhandene Dateien unberuehrt.
    #[tokio::test]
    async fn cancelling_leaves_existing_files_untouched() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        std::fs::write(&files[1].1, &WEIGHTS[..5]).unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let out = provision(dir.path(), "t", &files, &cancel, &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Cancelled);
        assert!(server.requests.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), &WEIGHTS[..5]);
    }
    // ---- Ende nur mit verifiziertem Paar ------------------------------------

    /// Gewichte gleicher Groesse mit falschen Bytes: nur die SHA-256 sieht es.
    fn same_size_wrong() -> Vec<u8> {
        vec![b'x'; WEIGHTS.len()]
    }

    /// Hook, der bei der Anfrage `trigger` die Datei `victim` durch gleich grosse
    /// falsche Bytes ersetzt (ein Tausch mitten im Bereitstellen).
    fn swap_on(trigger: &'static str, victim: PathBuf) -> Arc<dyn Fn(&str) + Send + Sync> {
        Arc::new(move |path: &str| {
            if path == trigger {
                std::fs::write(&victim, same_size_wrong()).unwrap();
            }
        })
    }

    /// Gewichte gueltig, Graph fehlt; waehrend der Graph geladen wird, werden die
    /// Gewichte gegen gleich grosse falsche Bytes getauscht. Das ist kein Erfolg:
    /// Fehler, der Graph liegt nicht am Ziel (er wartet als verifizierte Teildatei),
    /// die Anzeige sagt "nicht bereit"; die naechste Runde holt nur die Gewichte.
    #[tokio::test]
    async fn a_swap_of_the_weights_while_the_graph_downloads_is_no_success() {
        let dir = tempfile::tempdir().unwrap();
        let hook = swap_on("/model.onnx", dir.path().join("model.onnx_data"));
        let server = serve_with(both_routes(), hook).await;
        let files = rebase(two_files(&server, None), dir.path());
        std::fs::write(&files[1].1, WEIGHTS).unwrap();

        let err = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap_err();
        assert_eq!(err, "verification_failed");
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx"]);
        assert!(!files[0].1.exists(), "Graph neben getauschten Gewichten");
        assert_eq!(
            std::fs::read(&files[1].1).unwrap(),
            same_size_wrong(),
            "Testannahme: der Tausch hat stattgefunden"
        );
        assert!(
            !looks_complete(&files),
            "Anzeige: bereit trotz fehlendem Graph"
        );
        assert_eq!(
            std::fs::read(dir.path().join("model.onnx.partial")).unwrap(),
            GRAPH,
            "der verifizierte Graph wartet als Teildatei"
        );

        let healthy = serve(both_routes()).await;
        let files = rebase(two_files(&healthy, None), dir.path());
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert_eq!(*healthy.requests.lock().unwrap(), ["/model.onnx_data"]);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    /// Dasselbe bei frischer Installation: die Gewichte liegen gerade erst
    /// verifiziert am Ziel, als sie waehrend der Graph-Anfrage getauscht werden.
    #[tokio::test]
    async fn a_swap_after_a_fresh_weights_install_is_caught_too() {
        let dir = tempfile::tempdir().unwrap();
        let hook = swap_on("/model.onnx", dir.path().join("model.onnx_data"));
        let server = serve_with(both_routes(), hook).await;
        let files = rebase(two_files(&server, None), dir.path());

        let err = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap_err();
        assert_eq!(err, "verification_failed");
        assert_eq!(
            *server.requests.lock().unwrap(),
            ["/model.onnx_data", "/model.onnx"]
        );
        assert!(!files[0].1.exists());
        assert!(!looks_complete(&files));
    }

    /// Die letzte Pruefung allein: alles gueltig ist Erfolg und fasst nichts an;
    /// sonst kein Erfolg, und der Graph ist je nach Befund zurueckgezogen
    /// (gueltig: als Teildatei, ungueltig: weg).
    #[tokio::test]
    async fn the_final_check_passes_only_a_verified_pair_and_withdraws_the_graph_otherwise() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        let none = CancellationToken::new();

        assert_eq!(confirm(&files, &none).await.unwrap(), Fetch::Done);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);

        // Gewichte getauscht, Graph gut: Graph als Teildatei beiseite.
        std::fs::write(&files[1].1, same_size_wrong()).unwrap();
        assert_eq!(
            confirm(&files, &none).await.unwrap_err(),
            "verification_failed"
        );
        assert!(!files[0].1.exists());
        assert_eq!(
            std::fs::read(dir.path().join("model.onnx.partial")).unwrap(),
            GRAPH
        );
        assert_eq!(std::fs::read(&files[1].1).unwrap(), same_size_wrong());

        // Graph getauscht (gleich gross, falsche Bytes): weg, auch keine Teildatei.
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        std::fs::write(&files[0].1, vec![b'y'; GRAPH.len()]).unwrap();
        assert_eq!(
            confirm(&files, &none).await.unwrap_err(),
            "verification_failed"
        );
        assert_eq!(leftovers(dir.path()), ["model.onnx_data"]);
    }

    /// Abbruch in der letzten Pruefung ist kein Erfolg und laesst keinen
    /// ungeprueften Graphen am Ziel liegen.
    #[tokio::test]
    async fn cancelling_the_final_check_is_no_success_and_withdraws_the_graph() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(confirm(&files, &cancel).await.unwrap(), Fetch::Cancelled);
        assert!(!files[0].1.exists());
        assert_eq!(
            std::fs::read(dir.path().join("model.onnx.partial")).unwrap(),
            GRAPH
        );
        assert!(!looks_complete(&files));
        // Die naechste Runde stellt ohne Netz wieder her.
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(server.requests.lock().unwrap().is_empty());
        assert!(looks_complete(&files));
    }

    /// Fehlende Ordner unterhalb der Wurzel legt das Bereitstellen an.
    #[tokio::test]
    async fn provisioning_creates_the_folders_below_the_root() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("models");
        let sub = root.join("diarization").join("id");
        let files = rebase(two_files(&server, None), &sub);
        let out = provision(&root, "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(looks_complete(&files));
        assert_eq!(leftovers(&sub), ["model.onnx", "model.onnx_data"]);
    }

    // ---- Katalogwerte, die aus dem Modellordner hinausfuehren -----------------

    #[test]
    fn targets_refuse_catalog_values_that_could_leave_the_model_folder() {
        let root = Path::new("models");
        let entry = catalog_entry(diarize::nemotron::MODEL_ID).unwrap();
        assert_eq!(targets(root, &entry).unwrap().len(), 2);
        let single = catalog_entry(diarize::DEFAULT_MODEL_ID).unwrap();
        assert_eq!(targets(root, &single).unwrap().len(), 1);
        for bad in ["..", "../x", "a/b", "/abs", "a\\b", "C:\\x", "C:", ""] {
            let mut by_id = entry.clone();
            by_id.id = bad.to_string();
            assert_eq!(
                targets(root, &by_id).unwrap_err(),
                "catalog_invalid",
                "{bad:?}"
            );
            let mut by_name = entry.clone();
            by_name.files[1].filename = bad.to_string();
            assert_eq!(
                targets(root, &by_name).unwrap_err(),
                "catalog_invalid",
                "{bad:?}"
            );
            let mut one = single.clone();
            one.files[0].filename = bad.to_string();
            assert_eq!(
                targets(root, &one).unwrap_err(),
                "catalog_invalid",
                "{bad:?}"
            );
        }
    }

    // ---- Verknuepfungen: nur eigene, normale Dateien im Modellordner --------

    const FOREIGN: &[u8] = b"ABCDE";

    /// Prueft, dass `provision` mit `unsafe_path` ablehnt, ohne das Netz zu fragen.
    async fn assert_refused_without_network(
        root: &Path,
        files: &[(TtsCatalogFile, PathBuf)],
        server: &Server,
    ) {
        let err = provision(root, "t", files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap_err();
        assert_eq!(err, "unsafe_path");
        assert!(
            server.requests.lock().unwrap().is_empty(),
            "Netzzugriff trotz Ablehnung"
        );
    }

    #[cfg(unix)]
    mod links {
        use super::*;
        use std::os::unix::fs::symlink;

        fn foreign(dir: &Path, bytes: &[u8]) -> PathBuf {
            let path = dir.join("fremd.bin");
            std::fs::write(&path, bytes).unwrap();
            path
        }

        fn is_link(path: &Path) -> bool {
            std::fs::symlink_metadata(path)
                .unwrap()
                .file_type()
                .is_symlink()
        }

        /// Ein Link als Ziel wird abgelehnt, auch wenn dahinter genau die richtigen
        /// Bytes liegen: die fremde Datei wird weder gelesen als "gueltig" noch
        /// ersetzt, noch umbenannt, noch geloescht.
        #[tokio::test]
        async fn a_symlinked_target_is_refused_even_when_it_holds_the_right_bytes() {
            for victim in [1usize, 0] {
                let server = serve(both_routes()).await;
                let dir = tempfile::tempdir().unwrap();
                let outside = tempfile::tempdir().unwrap();
                let files = rebase(two_files(&server, None), dir.path());
                install(&files);
                let right = if victim == 1 { WEIGHTS } else { GRAPH };
                let target = foreign(outside.path(), right);
                std::fs::remove_file(&files[victim].1).unwrap();
                symlink(&target, &files[victim].1).unwrap();
                assert!(
                    !looks_complete(&files),
                    "Anzeige: bereit trotz Link an Stelle {victim}"
                );

                assert_refused_without_network(dir.path(), &files, &server).await;

                assert_eq!(
                    std::fs::read(&target).unwrap(),
                    right,
                    "fremde Datei veraendert"
                );
                assert!(is_link(&files[victim].1), "Link ersetzt");
                let other = 1 - victim;
                assert!(!is_link(&files[other].1) && files[other].1.is_file());
            }
        }

        /// Ein Link als Teildatei: die fremde Datei dahinter (kuerzer als die
        /// Gewichte) bekommt keine Bytes angehaengt, das Netz wird nie gefragt.
        #[tokio::test]
        async fn a_symlinked_partial_is_refused_and_the_foreign_file_stays_byte_identical() {
            let server = serve(both_routes()).await;
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let files = rebase(two_files(&server, None), dir.path());
            let target = foreign(outside.path(), FOREIGN);
            let partial = partial_path(&files[1].1);
            symlink(&target, &partial).unwrap();

            assert_refused_without_network(dir.path(), &files, &server).await;

            assert_eq!(std::fs::read(&target).unwrap(), FOREIGN);
            assert!(is_link(&partial));
            assert!(!files[0].1.exists() && !files[1].1.exists());
        }

        /// Ein Link ins Leere als Teildatei: hinter dem Link entsteht nichts.
        #[tokio::test]
        async fn a_dangling_symlinked_partial_creates_nothing() {
            let server = serve(both_routes()).await;
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let files = rebase(two_files(&server, None), dir.path());
            let behind = outside.path().join("noch-nicht-da.bin");
            symlink(&behind, partial_path(&files[1].1)).unwrap();

            assert_refused_without_network(dir.path(), &files, &server).await;
            assert!(!behind.exists());
        }

        /// Ein Zwischenordner, der ein Link aus dem Modellordner heraus ist: es wird
        /// weder dahinter etwas angelegt noch geschrieben, und das Netz nicht gefragt.
        #[tokio::test]
        async fn a_linked_folder_pointing_outside_the_model_root_is_refused() {
            for leaf_only in [false, true] {
                let server = serve(both_routes()).await;
                let dir = tempfile::tempdir().unwrap();
                let outside = tempfile::tempdir().unwrap();
                let root = dir.path().join("models");
                std::fs::create_dir_all(&root).unwrap();
                let sub = root.join("diarization").join("id");
                if leaf_only {
                    std::fs::create_dir_all(root.join("diarization")).unwrap();
                    symlink(outside.path(), &sub).unwrap();
                } else {
                    symlink(outside.path(), root.join("diarization")).unwrap();
                }
                let files = rebase(two_files(&server, None), &sub);

                assert_refused_without_network(&root, &files, &server).await;

                assert!(
                    std::fs::read_dir(outside.path()).unwrap().next().is_none(),
                    "ausserhalb des Modellordners wurde etwas angelegt (leaf_only={leaf_only})"
                );
            }
        }

        /// Gegenprobe: ein verschobener Datenordner (die Wurzel ist ein Link) ist
        /// kein Verstoss, alles darunter liegt dann dort, wohin die Wurzel zeigt.
        #[tokio::test]
        async fn a_model_root_that_is_itself_a_link_still_works() {
            let server = serve(both_routes()).await;
            let dir = tempfile::tempdir().unwrap();
            let real = dir.path().join("platte-d");
            std::fs::create_dir_all(&real).unwrap();
            let root = dir.path().join("models");
            symlink(&real, &root).unwrap();
            let sub = root.join("diarization").join("id");
            let files = rebase(two_files(&server, None), &sub);

            let out = provision(&root, "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .unwrap();
            assert_eq!(out, Fetch::Done);
            assert!(looks_complete(&files));
            assert_eq!(
                std::fs::read(real.join("diarization").join("id").join("model.onnx")).unwrap(),
                GRAPH
            );
        }

        /// Ein Link als Gewichte-Ziel, der erst in der letzten Pruefung auffaellt
        /// (eingeschoben waehrend des Ladens): kein Erfolg, Graph zurueckgezogen,
        /// fremde Datei unberuehrt.
        #[tokio::test]
        async fn a_link_slipped_in_during_the_download_is_caught_by_the_final_check() {
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let target = foreign(outside.path(), WEIGHTS);
            let weights = dir.path().join("model.onnx_data");
            let hook: Arc<dyn Fn(&str) + Send + Sync> = {
                let (weights, target) = (weights.clone(), target.clone());
                Arc::new(move |path: &str| {
                    if path == "/model.onnx" {
                        std::fs::remove_file(&weights).unwrap();
                        symlink(&target, &weights).unwrap();
                    }
                })
            };
            let server = serve_with(both_routes(), hook).await;
            let files = rebase(two_files(&server, None), dir.path());
            std::fs::write(&files[1].1, WEIGHTS).unwrap();

            let err = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .unwrap_err();
            assert_eq!(err, "unsafe_path");
            assert!(!files[0].1.exists(), "Graph neben einem Link als Gewichte");
            assert_eq!(std::fs::read(&target).unwrap(), WEIGHTS);
            assert!(is_link(&weights));
        }
    }

    /// Windows: ein Symlink braucht Entwicklermodus oder Administrator; laesst er
    /// sich nicht anlegen, wird der Fall mit Meldung uebersprungen.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_symlinked_target_is_refused_when_a_symlink_can_be_created() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        let target = outside.path().join("fremd.bin");
        std::fs::write(&target, WEIGHTS).unwrap();
        std::fs::remove_file(&files[1].1).unwrap();
        if let Err(e) = std::os::windows::fs::symlink_file(&target, &files[1].1) {
            eprintln!("uebersprungen: Symlink nicht anlegbar ({e})");
            return;
        }
        assert!(!looks_complete(&files));
        assert_refused_without_network(dir.path(), &files, &server).await;
        assert_eq!(std::fs::read(&target).unwrap(), WEIGHTS);
    }
}
