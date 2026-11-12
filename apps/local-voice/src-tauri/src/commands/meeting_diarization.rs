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
//! - Reihenfolge: die Datei, die der Motor oeffnet (die erste des Eintrags),
//!   kommt zuletzt; sie ist nur da, wenn alle anderen schon verifiziert liegen;
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

/// Bereit heisst: alle Dateien des Eintrags liegen an ihrem Platz. Eine Datei
/// bekommt ihren Namen erst nach bestandener Pruefung (siehe oben).
fn all_present(targets: &[(TtsCatalogFile, PathBuf)]) -> bool {
    !targets.is_empty() && targets.iter().all(|(_, path)| path.is_file())
}

fn status_for(app: &AppHandle, id: &str) -> Result<DiarizationModelStatus, String> {
    let entry = catalog_entry(id).ok_or_else(|| "model_missing".to_string())?;
    let size: u64 = entry.files.iter().map(|f| f.size_bytes).sum();
    Ok(DiarizationModelStatus {
        model_id: entry.id.clone(),
        model_ready: all_present(&targets(&models_root(app), &entry)),
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

/// Laedt jede fehlende Datei nach ihrem Ziel: Teildatei `<ziel>.partial`
/// (fortsetzbar, Groesse und SHA-256 aus dem Katalog geprueft), dann
/// umbenennen. Die erste Datei des Eintrags kommt zuletzt.
async fn fetch_missing(
    model_id: &str,
    targets: &[(TtsCatalogFile, PathBuf)],
    cancel: &CancellationToken,
    emit: &(dyn Fn(HttpDownloadEvent<'_>) + Send + Sync),
) -> Result<Fetch, String> {
    for (file, dest) in targets.iter().rev() {
        if dest.is_file() {
            continue;
        }
        if cancel.is_cancelled() {
            return Ok(Fetch::Cancelled);
        }
        let dir = dest.parent().ok_or_else(|| "model_missing".to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let partial = PathBuf::from(format!("{}.partial", dest.display()));
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

/// Laedt das Modell `id` (alle seine Dateien), nur auf Knopfdruck oder
/// `--provision`. Fortschritt ueber `model-download-progress`, Ende ueber
/// `model-download-complete`. Wirkt ab der naechsten Sprechertrennung.
pub async fn download_model(app: &AppHandle, id: &str) -> Result<(), String> {
    let entry = catalog_entry(id).ok_or_else(|| "model_missing".to_string())?;
    let files = targets(&models_root(app), &entry);
    if files.is_empty() {
        return Err("model_missing".into());
    }
    if all_present(&files) {
        return Ok(());
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
    // Ein paralleler Aufruf kann inzwischen fertig sein.
    if all_present(&files) {
        return Ok(());
    }
    let handle = app.clone();
    let emit = move |event: HttpDownloadEvent<'_>| {
        if let HttpDownloadEvent::Progress(p) = event {
            let _ = handle.emit("model-download-progress", p);
        }
    };
    let cancel = CancellationToken::new();
    *CANCEL.lock().unwrap_or_else(|e| e.into_inner()) = Some(cancel.clone());
    match fetch_missing(&entry.id, &files, &cancel, &emit).await? {
        Fetch::Done => {
            let _ = app.emit("model-download-complete", entry.id.as_str());
            Ok(())
        }
        Fetch::Cancelled => Err("cancelled".into()),
    }
}

/// Laedt das Standardmodell (Sortformer), nur auf Knopfdruck (oder
/// `--provision`). Fortschritt ueber `model-download-progress`, Ende ueber
/// `model-download-complete`. Wirkt ab der naechsten Sprechertrennung (Stopp,
/// Import, Neu-Transkription).
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (routes, log) = (routes.clone(), log.clone());
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
        assert!(!all_present(&files));
        let out = fetch_missing("test/nemotron", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(all_present(&files));
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
        fetch_missing("t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        let asked = server.requests.lock().unwrap().len();
        fetch_missing("t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(
            server.requests.lock().unwrap().len(),
            asked,
            "Nachladen trotz vollstaendiger Dateien"
        );

        // Eine fehlende Datei wird einzeln nachgeladen, die vorhandene bleibt.
        std::fs::remove_file(&files[0].1).unwrap();
        fetch_missing("t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(server.requests.lock().unwrap().len(), asked + 1);
        assert_eq!(
            *server.requests.lock().unwrap().last().unwrap(),
            "/model.onnx"
        );
        assert!(all_present(&files));
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
        let err = fetch_missing("t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap_err();
        assert!(!err.is_empty());
        assert!(!files[0].1.exists(), "Graph ohne Gewichte");
        assert!(!all_present(&files));
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
            fetch_missing("t", &files, &CancellationToken::new(), &|_| {})
                .await
                .is_err()
        );
        assert!(!files[0].1.exists() && !files[1].1.exists());
        assert!(!all_present(&files));
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
        let out = fetch_missing("t", &files, &cancel, &|_| {}).await.unwrap();
        assert_eq!(out, Fetch::Cancelled);
        assert!(server.requests.lock().unwrap().is_empty());
        assert!(!all_present(&files));
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
            !all_present(&files),
            "Teildatei darf nicht als installiert gelten"
        );
        fetch_missing("t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert!(all_present(&files));
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        // Die volle Teildatei wurde nur geprueft, nicht erneut geladen.
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx"]);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }
}
