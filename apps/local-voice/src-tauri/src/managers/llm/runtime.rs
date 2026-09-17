//! Laufzeit und Modelle des lokalen Sprachmodells: Katalog, Download,
//! Entpacken, Aufloesen, Backend-Wahl.
//!
//! Dieselbe Mechanik wie bei der Piper-Laufzeit: der Katalog nennt je
//! Plattform ein Paket mit Pruefsumme, der Download ist fortsetzbar und wird
//! verifiziert, das Archiv wird atomisch entpackt. Neu ist die Backend-Wahl
//! per Selbsttest -- nicht nach Registry oder `nvidia-smi`, sondern indem das
//! Paket selbst gefragt wird, welche Geraete es sieht.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter, Manager};
use hf_hub::api::tokio::CancellationToken;

use crate::catalog::{self, Purpose, TtsCatalogEntry};
use crate::managers::model::download::{HttpDownloadEvent, HttpDownloadOutcome};
use crate::managers::model::ModelManager;
use crate::managers::tts::models::TtsModelManager;

/// Backends in der Reihenfolge, in der sie probiert werden: das schnellste
/// zuerst, das genuegsamste zuletzt. Die Kennung ist das Suffix der
/// Katalog-Kennung (`llm-runtime-<plattform>-<backend>`).
const BACKEND_ORDER: [&str; 3] = ["cuda", "vulkan", "cpu"];

/// Wie lange der Selbsttest (`--list-devices`) dauern darf. Ein haengender
/// Treiber darf den Start der App nicht mitnehmen.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LlmDownloadKind {
    Runtime,
    Model,
}

/// Ein Eintrag fuer die Modellseite: Laufzeitpaket oder Modell.
#[derive(Debug, Clone, Serialize, Type)]
pub struct LlmDownloadInfo {
    pub id: String,
    pub kind: LlmDownloadKind,
    pub name: String,
    pub description: String,
    pub size_mb: u64,
    pub is_downloaded: bool,
    pub is_downloading: bool,
    /// Kuratierte Merkmale (nur Modelle).
    pub tags: Vec<String>,
    /// Backend-Kennung (nur Laufzeiten): "cuda", "vulkan", "cpu", "metal".
    pub backend: Option<String>,
    /// Ob dieses Paket fuer diesen Rechner gedacht ist. Fremde Plattformen
    /// bleiben im Katalog sichtbar, aber nicht ladbar.
    pub for_this_platform: bool,
}

/// Plattform-Kennung wie in den Katalog-Kennungen (`llm-runtime-<hier>-…`).
fn current_platform() -> Option<&'static str> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-aarch64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("macos-x64")
    } else {
        None
    }
}

/// Backend-Kennung aus einer Laufzeit-Kennung: `llm-runtime-windows-x64-vulkan`
/// → `vulkan`; macOS-Pakete tragen kein Suffix und laufen ueber Metal.
pub(crate) fn backend_of(runtime_id: &str, platform: &str) -> Option<String> {
    let prefix = format!("llm-runtime-{platform}");
    let rest = runtime_id.strip_prefix(&prefix)?;
    match rest.strip_prefix('-') {
        Some(backend) if !backend.is_empty() => Some(backend.to_string()),
        Some(_) => None,
        None if rest.is_empty() => Some("metal".to_string()),
        None => None,
    }
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// Sucht die ausfuehrbare Datei im entpackten Paket. Windows-Archive liegen
/// flach, macOS-Archive unter `build/bin/` -- statt das zu wissen, suchen
/// wir, begrenzt auf drei Ebenen.
fn find_binary(dir: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: u8) -> Option<PathBuf> {
        let entries = std::fs::read_dir(dir).ok()?;
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.file_name().is_some_and(|n| n == binary_name()) {
                return Some(path);
            }
            if path.is_dir() && depth > 0 {
                subdirs.push(path);
            }
        }
        subdirs.into_iter().find_map(|d| walk(&d, depth - 1))
    }
    walk(dir, 3)
}

pub struct LlmRuntimeManager {
    app_handle: AppHandle,
    base_dir: PathBuf,
    cancel_flags: Mutex<HashMap<String, CancellationToken>>,
    /// Ergebnis des letzten Selbsttests: welches Backend laeuft hier.
    resolved_backend: Mutex<Option<String>>,
}

impl LlmRuntimeManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        let base = crate::portable::data_dir()
            .cloned()
            .or_else(|| app_handle.path().app_local_data_dir().ok())
            .ok_or_else(|| anyhow::anyhow!("Failed to get app data dir"))?;
        Ok(Self {
            app_handle: app_handle.clone(),
            base_dir: base.join("llm"),
            cancel_flags: Mutex::new(HashMap::new()),
            resolved_backend: Mutex::new(None),
        })
    }

    fn runtime_dir(&self, runtime_id: &str) -> PathBuf {
        self.base_dir.join("runtime").join(runtime_id)
    }

    fn models_dir(&self) -> PathBuf {
        self.base_dir.join("models")
    }

    pub fn log_path(&self) -> PathBuf {
        self.base_dir.join("server.log")
    }

    /// Der Pfad einer Modelldatei -- ob sie existiert, sagt `is_file()`.
    pub fn model_path(&self, model_id: &str) -> Option<PathBuf> {
        let entry = catalog::tts_entries(Purpose::LlmModel)
            .into_iter()
            .find(|e| e.id == model_id)?;
        let file = entry.files.first()?;
        Some(self.models_dir().join(&file.filename))
    }

    /// Dateigroesse und Quelle eines Modells laut Katalog (fuer die
    /// Speicherprognose, auch vor dem Download).
    pub fn model_source(&self, model_id: &str) -> Option<(u64, String)> {
        let entry = catalog::tts_entries(Purpose::LlmModel)
            .into_iter()
            .find(|e| e.id == model_id)?;
        let file = entry.files.first()?;
        Some((file.size_bytes, file.url.clone()))
    }

    /// Ausfuehrbare Datei eines installierten Laufzeitpakets.
    pub fn binary_for(&self, runtime_id: &str) -> Option<PathBuf> {
        let dir = self.runtime_dir(runtime_id);
        if !dir.is_dir() {
            return None;
        }
        find_binary(&dir)
    }

    fn runtime_ids_for_platform(&self, platform: &str) -> Vec<(String, String)> {
        // (runtime_id, backend) in Vorzugsreihenfolge
        let entries = catalog::tts_entries(Purpose::LlmRuntime);
        let mut out = Vec::new();
        for backend in BACKEND_ORDER.iter().chain(std::iter::once(&"metal")) {
            for e in &entries {
                if backend_of(&e.id, platform).as_deref() == Some(backend) {
                    out.push((e.id.clone(), backend.to_string()));
                }
            }
        }
        out
    }

    /// Installierte Laufzeiten dieses Rechners in Vorzugsreihenfolge.
    pub fn installed_runtimes(&self) -> Vec<(String, String, PathBuf)> {
        let Some(platform) = current_platform() else {
            return Vec::new();
        };
        self.runtime_ids_for_platform(platform)
            .into_iter()
            .filter_map(|(id, backend)| self.binary_for(&id).map(|bin| (id, backend, bin)))
            .collect()
    }

    /// Selbsttest eines Pakets: welche Geraete sieht es? Leer heisst: laeuft
    /// nur auf der CPU oder gar nicht -- beides kein Grund, es vorzuziehen.
    pub async fn probe_devices(&self, binary: &Path) -> Result<Vec<String>, String> {
        let binary = binary.to_path_buf();
        // Auf einen Blocking-Thread: `std::process` mit eigenem Zeitlimit,
        // weil tokio hier ohne `process`-Feature gebaut ist -- und ein
        // haengender Treiber den Async-Worker nicht blockieren darf.
        tokio::task::spawn_blocking(move || run_probe(&binary, PROBE_TIMEOUT))
            .await
            .map_err(|e| format!("Selbsttest abgebrochen: {e}"))?
            .map(|text| parse_devices(&text))
    }

    /// Waehlt die Laufzeit fuer diesen Rechner: das schnellste installierte
    /// Backend, das im Selbsttest ein Geraet meldet; sonst CPU. Das Ergebnis
    /// wird behalten, bis eine Laufzeit dazukommt oder verschwindet.
    pub async fn resolve_runtime(&self) -> Result<(String, String, PathBuf), String> {
        let installed = self.installed_runtimes();
        if installed.is_empty() {
            return Err("Keine Sprachmodell-Laufzeit installiert".to_string());
        }
        if let Some(chosen) = self.resolved_backend.lock().unwrap().clone() {
            if let Some(hit) = installed.iter().find(|(_, b, _)| *b == chosen) {
                return Ok(hit.clone());
            }
        }
        let mut fallback = None;
        for (id, backend, bin) in &installed {
            if backend == "cpu" {
                fallback = Some((id.clone(), backend.clone(), bin.clone()));
                continue;
            }
            match self.probe_devices(bin).await {
                Ok(devices) if !devices.is_empty() => {
                    log::info!("Sprachmodell-Backend {backend}: {}", devices.join(", "));
                    *self.resolved_backend.lock().unwrap() = Some(backend.clone());
                    return Ok((id.clone(), backend.clone(), bin.clone()));
                }
                Ok(_) => log::info!("Backend {backend} sieht kein Geraet, naechstes"),
                Err(e) => log::warn!("Backend {backend} faellt aus: {e}"),
            }
        }
        let chosen = fallback
            .or_else(|| installed.first().cloned())
            .ok_or_else(|| "Keine Laufzeit nutzbar".to_string())?;
        *self.resolved_backend.lock().unwrap() = Some(chosen.1.clone());
        Ok(chosen)
    }

    pub fn forget_backend(&self) {
        *self.resolved_backend.lock().unwrap() = None;
    }

    fn is_downloading(&self, id: &str) -> bool {
        self.cancel_flags.lock().unwrap().contains_key(id)
    }

    fn claim(&self, id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.cancel_flags
            .lock()
            .unwrap()
            .insert(id.to_string(), token.clone());
        token
    }

    fn release(&self, id: &str) {
        self.cancel_flags.lock().unwrap().remove(id);
    }

    pub fn list_downloads(&self) -> Vec<LlmDownloadInfo> {
        let platform = current_platform();
        let mut out = Vec::new();
        for e in catalog::tts_entries(Purpose::LlmRuntime) {
            let backend = platform.and_then(|p| backend_of(&e.id, p));
            out.push(LlmDownloadInfo {
                is_downloaded: self.binary_for(&e.id).is_some(),
                is_downloading: self.is_downloading(&e.id),
                size_mb: e.files.iter().map(|f| f.size_bytes).sum::<u64>() / (1024 * 1024),
                for_this_platform: backend.is_some(),
                backend,
                tags: Vec::new(),
                id: e.id,
                kind: LlmDownloadKind::Runtime,
                name: e.name,
                description: e.description,
            });
        }
        for e in catalog::tts_entries(Purpose::LlmModel) {
            let downloaded = e
                .files
                .first()
                .is_some_and(|f| self.models_dir().join(&f.filename).is_file());
            out.push(LlmDownloadInfo {
                is_downloaded: downloaded,
                is_downloading: self.is_downloading(&e.id),
                size_mb: e.files.iter().map(|f| f.size_bytes).sum::<u64>() / (1024 * 1024),
                for_this_platform: true,
                backend: None,
                tags: e.tags,
                id: e.id,
                kind: LlmDownloadKind::Model,
                name: e.name,
                description: e.description,
            });
        }
        out
    }

    async fn run_download(
        &self,
        id: &str,
        url: &str,
        dest: &Path,
        size_bytes: u64,
        sha256: Option<&str>,
        cancel_token: &CancellationToken,
    ) -> Result<HttpDownloadOutcome> {
        // Dieselben Ereignisse wie bei Diktat- und Piper-Downloads: die
        // Oberflaeche hat einen Fortschrittsbalken, nicht drei.
        let emit = |event: HttpDownloadEvent<'_>| {
            let _ = match event {
                HttpDownloadEvent::Progress(progress) => {
                    self.app_handle.emit("model-download-progress", progress)
                }
                HttpDownloadEvent::VerificationStarted => {
                    self.app_handle.emit("model-verification-started", id)
                }
                HttpDownloadEvent::VerificationCompleted => {
                    self.app_handle.emit("model-verification-completed", id)
                }
            };
        };
        ModelManager::download_http_resumable_with_events(
            id,
            url,
            dest,
            Some(size_bytes),
            sha256,
            cancel_token,
            &emit,
        )
        .await
    }

    fn entry(&self, id: &str) -> Option<(TtsCatalogEntry, LlmDownloadKind)> {
        if let Some(e) = catalog::tts_entries(Purpose::LlmRuntime)
            .into_iter()
            .find(|e| e.id == id)
        {
            return Some((e, LlmDownloadKind::Runtime));
        }
        catalog::tts_entries(Purpose::LlmModel)
            .into_iter()
            .find(|e| e.id == id)
            .map(|e| (e, LlmDownloadKind::Model))
    }

    pub async fn download(&self, id: &str) -> Result<(), String> {
        let (entry, kind) = self
            .entry(id)
            .ok_or_else(|| format!("Unbekannter Katalogeintrag: {id}"))?;
        if self.is_downloading(id) {
            return Ok(());
        }
        match kind {
            LlmDownloadKind::Runtime => self.download_runtime(&entry).await,
            LlmDownloadKind::Model => self.download_model(&entry).await,
        }
    }

    /// Ein Laufzeitpaket kann aus mehreren Archiven bestehen (CUDA: Server +
    /// Runtime-Bibliotheken). Alle landen im selben Ordner; fehlt eines,
    /// gilt die Laufzeit als nicht installiert.
    async fn download_runtime(&self, entry: &TtsCatalogEntry) -> Result<(), String> {
        let platform = current_platform().ok_or_else(|| "Plattform nicht unterstuetzt".to_string())?;
        if backend_of(&entry.id, platform).is_none() {
            return Err(format!("{} ist nicht fuer diesen Rechner", entry.name));
        }
        let dest_dir = self.runtime_dir(&entry.id);
        std::fs::create_dir_all(self.base_dir.join("runtime")).map_err(|e| e.to_string())?;
        let cancel = self.claim(&entry.id);
        let result: Result<(), String> = async {
            for (index, file) in entry.files.iter().enumerate() {
                let archive = self
                    .base_dir
                    .join("runtime")
                    .join(format!("{}.{index}.download", entry.id));
                let outcome = self
                    .run_download(&entry.id, &file.url, &archive, file.size_bytes, file.sha256.as_deref(), &cancel)
                    .await
                    .map_err(|e| e.to_string())?;
                if matches!(outcome, HttpDownloadOutcome::Cancelled) {
                    return Ok(());
                }
                let is_zip = file.filename.to_ascii_lowercase().ends_with(".zip");
                // Erstes Archiv legt den Ordner an, weitere ergaenzen ihn --
                // deshalb je Archiv in einen eigenen Zwischenordner und dann
                // hineinkopieren.
                let part_dir = dest_dir.with_extension(format!("part{index}"));
                TtsModelManager::extract_runtime_archive(&archive, &part_dir, is_zip)
                    .map_err(|e| e.to_string())?;
                let _ = std::fs::remove_file(&archive);
                merge_dir(&part_dir, &dest_dir).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_dir_all(&part_dir);
            }
            Ok(())
        }
        .await;
        self.release(&entry.id);
        result?;
        self.forget_backend();
        let _ = self.app_handle.emit("model-download-complete", entry.id.clone());
        Ok(())
    }

    async fn download_model(&self, entry: &TtsCatalogEntry) -> Result<(), String> {
        let file = entry.files.first().ok_or_else(|| "Eintrag ohne Datei".to_string())?;
        let dir = self.models_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let partial = dir.join(format!("{}.download", file.filename));
        let final_path = dir.join(&file.filename);
        let cancel = self.claim(&entry.id);
        let outcome = self
            .run_download(&entry.id, &file.url, &partial, file.size_bytes, file.sha256.as_deref(), &cancel)
            .await;
        self.release(&entry.id);
        match outcome.map_err(|e| e.to_string())? {
            HttpDownloadOutcome::Cancelled => Ok(()),
            HttpDownloadOutcome::Completed => {
                std::fs::rename(&partial, &final_path).map_err(|e| e.to_string())?;
                let _ = self.app_handle.emit("model-download-complete", entry.id.clone());
                Ok(())
            }
        }
    }

    pub fn cancel(&self, id: &str) {
        if let Some(token) = self.cancel_flags.lock().unwrap().get(id) {
            token.cancel();
        }
        // Die Fussleiste hoert auf die gemeinsamen Download-Ereignisse aller
        // Downloader und raeumt einen Eintrag nur bei complete/failed/
        // cancelled weg. Ohne dieses Ereignis blieb ein abgebrochener
        // Download dort als "11 %" stehen (17.09.2026).
        let _ = self.app_handle.emit("model-download-cancelled", id);
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let (entry, kind) = self
            .entry(id)
            .ok_or_else(|| format!("Unbekannter Katalogeintrag: {id}"))?;
        match kind {
            LlmDownloadKind::Runtime => {
                let dir = self.runtime_dir(&entry.id);
                if dir.exists() {
                    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
                }
                self.forget_backend();
            }
            LlmDownloadKind::Model => {
                if let Some(file) = entry.files.first() {
                    let path = self.models_dir().join(&file.filename);
                    if path.exists() {
                        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        let _ = self.app_handle.emit("model-deleted", id.to_string());
        Ok(())
    }
}

/// Fuehrt `llama-server --list-devices` aus und liefert die Ausgabe. Laeuft
/// der Prozess laenger als `timeout`, wird er beendet und die Probe gilt als
/// gescheitert -- nicht als "kein Geraet".
fn run_probe(binary: &Path, timeout: Duration) -> Result<String, String> {
    use std::io::Read;
    let mut cmd = std::process::Command::new(binary);
    cmd.arg("--list-devices")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null());
    if let Some(dir) = binary.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Selbsttest nicht ausfuehrbar: {e}"))?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    // Ausgabe nebenlaeufig lesen, damit ein volles Pipe-Fenster den Prozess
    // nicht selbst zum Haengen bringt.
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(out) = stdout.as_mut() {
            let _ = out.read_to_string(&mut text);
        }
        if let Some(err) = stderr.as_mut() {
            let _ = err.read_to_string(&mut text);
        }
        text
    });
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Selbsttest der Laufzeit haengt".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("Selbsttest: {e}")),
        }
    }
    reader
        .join()
        .map_err(|_| "Selbsttest: Ausgabe nicht lesbar".to_string())
}

/// Geraetezeilen aus `--list-devices`: `  Vulkan0: NVIDIA GeForce RTX 4090 (…)`.
/// Nur Zeilen mit Doppelpunkt und einem bekannten Praefix zaehlen; die
/// Ueberschrift "Available devices:" nicht.
pub(crate) fn parse_devices(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            (lower.starts_with("vulkan") || lower.starts_with("cuda") || lower.starts_with("metal"))
                && l.contains(':')
        })
        .map(|l| l.to_string())
        .collect()
}

/// Kopiert einen entpackten Teilordner in den Laufzeitordner (rekursiv).
fn merge_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            merge_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_is_read_from_the_runtime_id() {
        assert_eq!(
            backend_of("llm-runtime-windows-x64-vulkan", "windows-x64").as_deref(),
            Some("vulkan")
        );
        assert_eq!(
            backend_of("llm-runtime-windows-x64-cuda", "windows-x64").as_deref(),
            Some("cuda")
        );
        assert_eq!(
            backend_of("llm-runtime-macos-aarch64", "macos-aarch64").as_deref(),
            Some("metal")
        );
        // Fremde Plattform: kein Backend, nicht ladbar.
        assert_eq!(backend_of("llm-runtime-macos-aarch64", "windows-x64"), None);
        assert_eq!(backend_of("llm-runtime-windows-x64-vulkan", "macos-aarch64"), None);
    }

    /// Der Selbsttest liest die Geraetezeilen; die Ueberschrift und Leerzeilen
    /// zaehlen nicht. Genau so sah die Ausgabe im Spike vom 13.09. aus.
    #[test]
    fn device_probe_output_is_parsed_line_by_line() {
        let text = "Available devices:\n  Vulkan0: NVIDIA GeForce RTX 4090 (24356 MiB, 23160 MiB free)\n  Vulkan1: Intel(R) UHD Graphics 770 (32649 MiB, 48147 MiB free)\n";
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 2);
        assert!(devices[0].starts_with("Vulkan0: NVIDIA"));
        assert!(parse_devices("Available devices:\n").is_empty());
        assert!(parse_devices("").is_empty());
    }

    #[test]
    fn the_binary_is_found_at_any_depth_up_to_three() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("build").join("bin");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(binary_name()), b"x").unwrap();
        assert_eq!(find_binary(dir.path()), Some(nested.join(binary_name())));
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(find_binary(empty.path()), None);
    }

    #[test]
    fn merging_copies_nested_files_without_clobbering_siblings() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(dst.path().join("bestehend.dll"), b"a").unwrap();
        std::fs::create_dir_all(src.path().join("lib")).unwrap();
        std::fs::write(src.path().join("lib").join("neu.dll"), b"b").unwrap();
        merge_dir(src.path(), dst.path()).unwrap();
        assert!(dst.path().join("bestehend.dll").is_file());
        assert!(dst.path().join("lib").join("neu.dll").is_file());
    }
}
