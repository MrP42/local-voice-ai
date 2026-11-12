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
use crate::managers::model::download::{HttpDownloadEvent, HttpDownloadOutcome};
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
fn targets(models_root: &Path, entry: &TtsCatalogEntry) -> Vec<(TtsCatalogFile, PathBuf)> {
    let dir = diarize::catalog_model_dir(models_root, entry);
    entry
        .files
        .iter()
        .map(|f| (f.clone(), dir.join(&f.filename)))
        .collect()
}

/// Bereit (fuer die Anzeige) heisst: alle Dateien des Eintrags liegen an ihrem
/// Platz und haben genau die Groesse aus dem Katalog. Nur `stat`, kein Lesen der
/// Inhalte; die volle Pruefung (SHA-256) macht [`provision`].
fn looks_complete(targets: &[(TtsCatalogFile, PathBuf)]) -> bool {
    !targets.is_empty()
        && targets.iter().all(|(file, path)| {
            std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == file.size_bytes)
        })
}

fn status_for(app: &AppHandle, id: &str) -> Result<DiarizationModelStatus, String> {
    let entry = catalog_entry(id).ok_or_else(|| "model_missing".to_string())?;
    let size: u64 = entry.files.iter().map(|f| f.size_bytes).sum();
    Ok(DiarizationModelStatus {
        model_id: entry.id.clone(),
        model_ready: looks_complete(&targets(&models_root(app), &entry)),
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
/// waehrend des Lesens abgebrochen.
fn check_file(
    file: &TtsCatalogFile,
    dest: &Path,
    cancel: &CancellationToken,
) -> std::io::Result<Option<FileState>> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let meta = match std::fs::metadata(dest) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Some(FileState::Missing)),
        Err(e) => return Err(e),
    };
    if !meta.is_file() || meta.len() != file.size_bytes {
        return Ok(Some(FileState::Invalid));
    }
    let Some(expected) = file.sha256.as_deref() else {
        return Ok(Some(FileState::Valid));
    };
    let mut reader = std::fs::File::open(dest)?;
    let mut hasher = Sha256::new();
    let mut block = vec![0u8; 1 << 20];
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let n = reader.read(&mut block)?;
        if n == 0 {
            break;
        }
        hasher.update(&block[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    Ok(Some(if actual.eq_ignore_ascii_case(expected) {
        FileState::Valid
    } else {
        FileState::Invalid
    }))
}

/// Stellt alle Dateien des Eintrags verifiziert bereit.
///
/// 1. Jede vorhandene Datei wird nach Groesse und SHA-256 geprueft. Sind alle
///    gueltig, ist nichts zu tun (kein Netz).
/// 2. Muss eine andere als die erste Datei ersetzt werden, wird die erste (die
///    der Motor oeffnet) zurueckgezogen: als `<ziel>.partial` abgelegt, ein
///    gueltiger Graph wird dadurch ohne Netz wieder uebernommen.
/// 3. Jede fehlende oder ungueltige Datei wird nach `<ziel>.partial` geladen
///    (fortsetzbar, Groesse und SHA-256 aus dem Katalog geprueft) und erst
///    dann ueber das Ziel umbenannt; die erste Datei kommt zuletzt.
async fn provision(
    model_id: &str,
    targets: &[(TtsCatalogFile, PathBuf)],
    cancel: &CancellationToken,
    emit: &(dyn Fn(HttpDownloadEvent<'_>) + Send + Sync),
) -> Result<Fetch, String> {
    let mut states = Vec::with_capacity(targets.len());
    for (file, dest) in targets {
        if cancel.is_cancelled() {
            return Ok(Fetch::Cancelled);
        }
        let (file, dest, token) = (file.clone(), dest.clone(), cancel.clone());
        let state = tokio::task::spawn_blocking(move || check_file(&file, &dest, &token))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        match state {
            Some(state) => states.push(state),
            None => return Ok(Fetch::Cancelled),
        }
    }
    if states.iter().all(|s| *s == FileState::Valid) {
        return Ok(Fetch::Done);
    }
    // Die erste Datei oeffnet der Motor: sie darf nicht neben Dateien liegen, die
    // gleich ersetzt werden. Eine gueltige wandert als Teildatei beiseite (kommt
    // ohne Netz zurueck), eine ungueltige wird ohnehin neu geladen.
    let entry_dest = &targets[0].1;
    let others_broken = states[1..].iter().any(|s| *s != FileState::Valid);
    if others_broken {
        match states[0] {
            FileState::Valid => {
                std::fs::rename(entry_dest, partial_path(entry_dest)).map_err(|e| e.to_string())?;
                states[0] = FileState::Missing;
            }
            FileState::Invalid => {
                std::fs::remove_file(entry_dest).map_err(|e| e.to_string())?;
                states[0] = FileState::Missing;
            }
            FileState::Missing => {}
        }
    }
    for ((file, dest), state) in targets.iter().zip(&states).rev() {
        if *state == FileState::Valid {
            continue;
        }
        if cancel.is_cancelled() {
            return Ok(Fetch::Cancelled);
        }
        let dir = dest.parent().ok_or_else(|| "model_missing".to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
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
        .map_err(|e| e.to_string())?;
        match outcome {
            HttpDownloadOutcome::Completed => {
                std::fs::rename(&partial, dest).map_err(|e| e.to_string())?;
            }
            HttpDownloadOutcome::Cancelled => return Ok(Fetch::Cancelled),
        }
    }
    Ok(Fetch::Done)
}

/// Stellt das Modell `id` bereit (alle seine Dateien), nur auf Knopfdruck oder
/// `--provision`: prueft jede vorhandene Datei nach Groesse und SHA-256, ersetzt
/// ungueltige und laedt fehlende (siehe [`provision`]). Fortschritt ueber
/// `model-download-progress`, Ende ueber `model-download-complete`. Wirkt ab der
/// naechsten Sprechertrennung.
pub async fn download_model(app: &AppHandle, id: &str) -> Result<(), String> {
    let entry = catalog_entry(id).ok_or_else(|| "model_missing".to_string())?;
    let files = targets(&models_root(app), &entry);
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
    match provision(&entry.id, &files, &cancel, &emit).await? {
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
        let t = targets(root, &entry);
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
        let t = targets(root, &old);
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
        let out = provision("test/nemotron", &files, &CancellationToken::new(), &|_| {})
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
        provision("t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        let asked = server.requests.lock().unwrap().len();
        provision("t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(
            server.requests.lock().unwrap().len(),
            asked,
            "Nachladen trotz vollstaendiger Dateien"
        );

        // Eine fehlende Datei wird einzeln nachgeladen, die vorhandene bleibt.
        std::fs::remove_file(&files[0].1).unwrap();
        provision("t", &files, &CancellationToken::new(), &|_| {})
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
        let err = provision("t", &files, &CancellationToken::new(), &|_| {})
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
        assert!(provision("t", &files, &CancellationToken::new(), &|_| {})
            .await
            .is_err());
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
        let out = provision("t", &files, &cancel, &|_| {}).await.unwrap();
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
        provision("t", &files, &CancellationToken::new(), &|_| {})
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
        let out = provision("t", &files, &CancellationToken::new(), &|_| {})
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

        let out = provision("t", &files, &CancellationToken::new(), &|_| {})
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
        let out = provision("t", &files, &CancellationToken::new(), &|_| {})
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
        provision("t", &files, &CancellationToken::new(), &|_| {})
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
        assert!(provision("t", &files, &CancellationToken::new(), &|_| {})
            .await
            .is_err());
        assert!(!files[0].1.exists(), "Graph neben defekten Gewichten");
        assert!(!looks_complete(&files));
        assert_eq!(
            std::fs::read(dir.path().join("model.onnx.partial")).unwrap(),
            GRAPH,
            "der gueltige Graph liegt als Teildatei bereit"
        );

        let healthy = serve(both_routes()).await;
        let files = rebase(two_files(&healthy, None), dir.path());
        let out = provision("t", &files, &CancellationToken::new(), &|_| {})
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
        assert!(provision("t", &files, &CancellationToken::new(), &|_| {})
            .await
            .is_err());
        assert_eq!(std::fs::read(&files[0].1).unwrap(), b"alte-datei-bytes");

        let up = serve(both_routes()).await;
        let files = vec![one(&up)];
        let out = provision("t", &files, &CancellationToken::new(), &|_| {})
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
        let out = provision("t", &files, &cancel, &|_| {}).await.unwrap();
        assert_eq!(out, Fetch::Cancelled);
        assert!(server.requests.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), &WEIGHTS[..5]);
    }
}
