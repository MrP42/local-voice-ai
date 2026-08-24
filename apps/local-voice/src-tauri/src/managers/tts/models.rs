//! Piper-Downloads: Katalog-Konsum + Verwaltung von Laufzeit und Stimmen (Paket B-E3).
//!
//! Spiegelt für TTS, was `managers::model::ModelManager` für ASR tut, OHNE es
//! zu duplizieren: die eigentliche Download-Maschinerie (Resume, Stall-Timeout,
//! SHA-256-Prüfung — `ModelManager::download_http_resumable_with_events`) wird
//! direkt wiederverwendet; nur Katalog-Konsum, Zielpfade und Entpacken sind
//! TTS-eigen, weil ein Piper-Eintrag (Laufzeit-Archiv je Plattform, Stimme aus
//! zwei Dateien) nicht in die ASR-Form ([`crate::managers::model::ModelDescriptor`],
//! immer genau EINE Hugging-Face-Datei) passt.
//!
//! Zielpfade (von einem parallelen Paket gelesen — siehe Task-Brief):
//! - Laufzeit: `<app_data>/tts/piper/<plattform>/` (aus dem Release-Archiv entpackt)
//! - Stimmen: `<app_data>/tts/piper/voices/<voice_id>.onnx` (+ `.onnx.json`)

use crate::catalog::{self, Purpose};
use crate::managers::model::download::{HttpDownloadEvent, HttpDownloadOutcome};
use crate::managers::model::ModelManager;
use anyhow::Result;
use hf_hub::api::tokio::CancellationToken;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

/// Frontend-facing id of the (single, platform-resolved) Piper runtime entry.
/// Stable across platforms — which underlying `piper-runtime-<platform>`
/// catalog entry backs it is an implementation detail resolved here.
pub const RUNTIME_ID: &str = "piper-runtime";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TtsDownloadKind {
    Runtime,
    Voice,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TtsDownloadInfo {
    pub id: String,
    pub kind: TtsDownloadKind,
    pub name: String,
    pub description: String,
    /// Primary language of a voice ("de", "en", …); `None` for the runtime.
    pub language: Option<String>,
    pub size_mb: u64,
    pub is_downloaded: bool,
    pub is_downloading: bool,
}

/// `<piper_dir>/<platform>` — where a platform's Piper binary and its
/// libraries/data land once the release archive is unpacked. Pure so it's
/// testable without touching disk or an `AppHandle`.
fn runtime_dir(piper_dir: &Path, platform: &str) -> PathBuf {
    piper_dir.join(platform)
}

/// Whether a COMPLETE Piper runtime sits at `runtime_dir(piper_dir, platform)`
/// — the binary itself AND its `espeak-ng-data` directory, both required to
/// run. Mirrors the voice check (`.onnx` AND `.onnx.json`, not just "the
/// directory exists"): a bare `is_dir()` also reads a leftover empty
/// directory, or one an interrupted extraction only half-filled, as
/// "installed" — this is the one place that decides "installed", used by
/// every caller instead of each re-deriving its own (looser) check.
fn runtime_is_installed(piper_dir: &Path, platform: &str) -> bool {
    let dir = runtime_dir(piper_dir, platform);
    let binary_name = if platform.starts_with("windows") {
        "piper.exe"
    } else {
        "piper"
    };
    dir.join(binary_name).is_file() && dir.join("espeak-ng-data").is_dir()
}

/// `(<piper_dir>/voices/<voice_id>.onnx, <piper_dir>/voices/<voice_id>.onnx.json)`
/// — the model file and its sidecar config, Piper's own naming convention.
/// Pure so it's testable without touching disk or an `AppHandle`.
fn voice_paths(piper_dir: &Path, voice_id: &str) -> (PathBuf, PathBuf) {
    let dir = piper_dir.join("voices");
    (
        dir.join(format!("{voice_id}.onnx")),
        dir.join(format!("{voice_id}.onnx.json")),
    )
}

/// The current platform's Piper runtime catalog-id suffix (`piper-runtime-<this>`),
/// or `None` on a platform Local Voice AI ships no Piper binary for (voices can
/// still be downloaded there — they aren't platform-specific).
///
/// Ids (`windows-x64`, `macos-aarch64`, `macos-x64`, `linux-x64`) are a
/// contract shared with the Piper ENGINE package (B-E2), which resolves the
/// binary at `<piper_dir>/<this>/`. `linux-x64` has no catalog entry yet (this
/// package only shipped/verified Windows + macOS downloads — see the task
/// brief and report) but the id is recognised here already so the engine's
/// lookup path matches the moment one is added.
fn current_platform() -> Option<&'static str> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-aarch64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("macos-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x64")
    } else {
        None
    }
}

/// A voice id's primary language subtag (`de_DE-thorsten-high` -> `de`).
/// `None` if the id doesn't start with a `xx`/`xx_YY` locale prefix. Pure.
fn language_of_voice_id(voice_id: &str) -> Option<String> {
    let locale = voice_id.split('-').next()?;
    let lang = locale.split('_').next()?;
    (!lang.is_empty()).then(|| lang.to_string())
}

pub struct TtsModelManager {
    app_handle: AppHandle,
    /// `<app_data>/tts/piper` — everything else is derived from this.
    piper_dir: PathBuf,
    /// One cancellation token per in-flight download, keyed by the
    /// frontend-facing id ([`RUNTIME_ID`] or a voice id).
    cancel_flags: Mutex<HashMap<String, CancellationToken>>,
}

impl TtsModelManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        // Same base dir the rest of the TTS subsystem uses (tts_cache, voice
        // storage, …) — NOT `portable::app_data_dir`, which resolves Tauri's
        // plain app-data dir and can differ from `app_local_data_dir` (notably
        // on macOS). Mirrors the pattern repeated in `managers::tts::mod`
        // (e.g. around its `tts_cache` setup) rather than calling a shared
        // helper: at the time this package was written that module had no
        // such helper yet to import (it's `managers/tts/mod.rs`, off-limits —
        // parallel package). CONSOLIDATION POINT: once merged, this should
        // call the same `data_base_dir()`-style helper instead of duplicating
        // the fallback chain.
        let base = crate::portable::data_dir()
            .cloned()
            .or_else(|| app_handle.path().app_local_data_dir().ok())
            .ok_or_else(|| anyhow::anyhow!("Failed to get app data dir"))?;
        Ok(Self {
            app_handle: app_handle.clone(),
            piper_dir: base.join("tts").join("piper"),
            cancel_flags: Mutex::new(HashMap::new()),
        })
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

    /// Runtime entry + voices, in that order (the runtime is the "program"
    /// row and belongs at the top — see the task brief).
    pub fn list_downloads(&self) -> Vec<TtsDownloadInfo> {
        Self::build_downloads(&self.piper_dir, |id| self.is_downloading(id))
    }

    /// Pure core of [`Self::list_downloads`] — everything except the live
    /// downloading-flag lookup (which needs `&self`/`cancel_flags`), so it's
    /// directly testable against a tempdir fixture without a
    /// `TtsModelManager` (and so without an `AppHandle`) at all.
    fn build_downloads(
        piper_dir: &Path,
        is_downloading: impl Fn(&str) -> bool,
    ) -> Vec<TtsDownloadInfo> {
        let mut out = Vec::new();

        if let Some(platform) = current_platform() {
            let catalog_id = format!("piper-runtime-{platform}");
            if let Some(entry) = catalog::tts_entries(Purpose::TtsRuntime)
                .into_iter()
                .find(|e| e.id == catalog_id)
            {
                let size_bytes: u64 = entry.files.iter().map(|f| f.size_bytes).sum();
                out.push(TtsDownloadInfo {
                    id: RUNTIME_ID.to_string(),
                    kind: TtsDownloadKind::Runtime,
                    name: entry.name,
                    description: entry.description,
                    language: None,
                    size_mb: size_bytes / (1024 * 1024),
                    is_downloaded: runtime_is_installed(piper_dir, platform),
                    is_downloading: is_downloading(RUNTIME_ID),
                });
            }
        }

        for entry in catalog::tts_entries(Purpose::TtsVoice) {
            let size_bytes: u64 = entry.files.iter().map(|f| f.size_bytes).sum();
            let (onnx_path, json_path) = voice_paths(piper_dir, &entry.id);
            out.push(TtsDownloadInfo {
                language: language_of_voice_id(&entry.id),
                is_downloaded: onnx_path.is_file() && json_path.is_file(),
                is_downloading: is_downloading(&entry.id),
                id: entry.id,
                kind: TtsDownloadKind::Voice,
                name: entry.name,
                description: entry.description,
                size_mb: size_bytes / (1024 * 1024),
            });
        }

        out
    }

    /// Drive one file through the shared resumable HTTP downloader, emitting
    /// onto the SAME Tauri events the ASR model downloader uses
    /// (`model-download-progress`/`model-verification-started`/`-completed`)
    /// so the frontend needs no new event plumbing — only `id` differs.
    async fn run_download(
        &self,
        id: &str,
        url: &str,
        dest: &Path,
        size_bytes: u64,
        sha256: Option<&str>,
        cancel_token: &CancellationToken,
    ) -> Result<HttpDownloadOutcome> {
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

    pub async fn download(&self, id: &str) -> Result<(), String> {
        if id == RUNTIME_ID {
            self.download_runtime().await
        } else {
            self.download_voice(id).await
        }
    }

    async fn download_runtime(&self) -> Result<(), String> {
        let platform = current_platform()
            .ok_or_else(|| "No Piper runtime is available for this platform".to_string())?;
        let dest_dir = runtime_dir(&self.piper_dir, platform);
        if runtime_is_installed(&self.piper_dir, platform) {
            return Ok(());
        }
        let catalog_id = format!("piper-runtime-{platform}");
        let entry = catalog::tts_entries(Purpose::TtsRuntime)
            .into_iter()
            .find(|e| e.id == catalog_id)
            .ok_or_else(|| format!("No catalog entry for {catalog_id}"))?;
        let file = entry
            .files
            .first()
            .ok_or_else(|| format!("{catalog_id}: catalog entry has no file"))?;

        fs::create_dir_all(&self.piper_dir).map_err(|e| e.to_string())?;
        let archive_path = self.piper_dir.join(format!("{platform}.download"));

        let cancel_token = self.claim(RUNTIME_ID);
        let outcome = self
            .run_download(
                RUNTIME_ID,
                &file.url,
                &archive_path,
                file.size_bytes,
                file.sha256.as_deref(),
                &cancel_token,
            )
            .await;
        self.release(RUNTIME_ID);

        match outcome.map_err(|e| e.to_string())? {
            HttpDownloadOutcome::Cancelled => Ok(()),
            HttpDownloadOutcome::Completed => {
                let extracted = Self::extract_runtime_archive(&archive_path, &dest_dir);
                let _ = fs::remove_file(&archive_path);
                extracted.map_err(|e| e.to_string())?;
                let _ = self.app_handle.emit("model-download-complete", RUNTIME_ID);
                Ok(())
            }
        }
    }

    async fn download_voice(&self, voice_id: &str) -> Result<(), String> {
        let entry = catalog::tts_entries(Purpose::TtsVoice)
            .into_iter()
            .find(|e| e.id == voice_id)
            .ok_or_else(|| format!("Unknown Piper voice: {voice_id}"))?;

        // Auto-fetch the runtime on the first voice download (task brief).
        // On a platform with NO Piper runtime CATALOG ENTRY (e.g. linux-x64 —
        // not shipped yet, see `current_platform`) this must be a true
        // no-op: `download_runtime()` would return an error there ("No
        // catalog entry for …"), which must NOT abort the voice download —
        // the voice files themselves aren't platform-specific. A platform
        // that DOES have an entry still propagates real download failures
        // (network, SHA mismatch) via `?`, same as calling it directly.
        if let Some(platform) = current_platform() {
            if !runtime_is_installed(&self.piper_dir, platform) {
                let catalog_id = format!("piper-runtime-{platform}");
                let has_runtime_entry = catalog::tts_entries(Purpose::TtsRuntime)
                    .into_iter()
                    .any(|e| e.id == catalog_id);
                if has_runtime_entry {
                    self.download_runtime().await?;
                } else {
                    log::warn!(
                        "No Piper runtime catalog entry for platform {platform}; \
                         downloading voice {voice_id} without a bundled runtime"
                    );
                }
            }
        }

        let (onnx_path, json_path) = voice_paths(&self.piper_dir, voice_id);
        if onnx_path.is_file() && json_path.is_file() {
            return Ok(());
        }
        let voices_dir = onnx_path
            .parent()
            .ok_or_else(|| "Invalid voice destination path".to_string())?;
        fs::create_dir_all(voices_dir).map_err(|e| e.to_string())?;

        let cancel_token = self.claim(voice_id);
        for file in &entry.files {
            let dest: &Path = if file.filename.ends_with(".onnx.json") {
                &json_path
            } else {
                &onnx_path
            };
            if dest.is_file() {
                continue; // resuming a voice that already has one of its two files
            }
            let partial_path = PathBuf::from(format!("{}.partial", dest.display()));
            let outcome = self
                .run_download(
                    voice_id,
                    &file.url,
                    &partial_path,
                    file.size_bytes,
                    file.sha256.as_deref(),
                    &cancel_token,
                )
                .await;
            match outcome {
                Ok(HttpDownloadOutcome::Cancelled) => {
                    self.release(voice_id);
                    return Ok(());
                }
                Ok(HttpDownloadOutcome::Completed) => {
                    if let Err(e) = fs::rename(&partial_path, dest) {
                        self.release(voice_id);
                        return Err(e.to_string());
                    }
                }
                Err(e) => {
                    self.release(voice_id);
                    return Err(e.to_string());
                }
            }
        }
        self.release(voice_id);
        let _ = self.app_handle.emit("model-download-complete", voice_id);
        Ok(())
    }

    /// Trigger the cancellation token for an in-flight download, if any. The
    /// in-flight `download()` call notices it on its next chunk/file and
    /// cleans up itself — mirrors `ModelManager::cancel_download`.
    pub fn cancel(&self, id: &str) {
        if let Some(token) = self.cancel_flags.lock().unwrap().get(id) {
            token.cancel();
        }
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        if id == RUNTIME_ID {
            let platform = current_platform()
                .ok_or_else(|| "No Piper runtime for this platform".to_string())?;
            let dir = runtime_dir(&self.piper_dir, platform);
            // Deliberately `is_dir()`, not `runtime_is_installed`: deletion
            // must be able to clear away a stray/incomplete directory (e.g.
            // an interrupted extraction) too, not just a complete install.
            if !dir.is_dir() {
                return Err("Piper runtime is not installed".to_string());
            }
            fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            let _ = self.app_handle.emit("model-deleted", RUNTIME_ID);
            return Ok(());
        }

        let (onnx_path, json_path) = voice_paths(&self.piper_dir, id);
        let mut deleted = false;
        for path in [onnx_path, json_path] {
            if path.exists() {
                fs::remove_file(&path).map_err(|e| e.to_string())?;
                deleted = true;
            }
        }
        if !deleted {
            return Err(format!("No files found for voice {id}"));
        }
        let _ = self.app_handle.emit("model-deleted", id);
        Ok(())
    }

    /// Unpack a Piper release archive (`.zip` on Windows, `.tar.gz` on macOS)
    /// into `dest_dir`. Piper's release archives nest everything under one
    /// top-level `piper/` directory; that gets flattened away so `dest_dir`
    /// ends up being exactly the directory the binary lives in — the same
    /// "extract to temp, then flatten a lone subdirectory" shape
    /// `ModelManager::download_model` uses for its own directory-based models.
    fn extract_runtime_archive(archive_path: &Path, dest_dir: &Path) -> Result<()> {
        let temp_dir = dest_dir.with_file_name(format!(
            "{}.extracting",
            dest_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("piper")
        ));
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir)?;
        }
        fs::create_dir_all(&temp_dir)?;

        let is_zip = archive_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        let extraction = if is_zip {
            Self::extract_zip(archive_path, &temp_dir)
        } else {
            Self::extract_tar_gz(archive_path, &temp_dir)
        };
        if let Err(e) = extraction {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(e);
        }

        let entries: Vec<_> = fs::read_dir(&temp_dir)?.filter_map(|e| e.ok()).collect();
        if dest_dir.exists() {
            fs::remove_dir_all(dest_dir)?;
        }
        if entries.len() == 1 && entries[0].file_type()?.is_dir() {
            fs::rename(entries[0].path(), dest_dir)?;
            let _ = fs::remove_dir_all(&temp_dir);
        } else {
            fs::rename(&temp_dir, dest_dir)?;
        }
        Ok(())
    }

    fn extract_zip(archive_path: &Path, dest_dir: &Path) -> Result<()> {
        let file = File::open(archive_path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            // `enclosed_name()` rejects absolute paths / `..` components —
            // zip-slip protection for an archive that, unlike our catalog
            // files, isn't hash-pinned by content addressed at this layer.
            let Some(rel_path) = entry.enclosed_name() else {
                continue;
            };
            let out_path = dest_dir.join(rel_path);
            if entry.is_dir() {
                fs::create_dir_all(&out_path)?;
            } else {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut out = File::create(&out_path)?;
                std::io::copy(&mut entry, &mut out)?;
            }
        }
        Ok(())
    }

    fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<()> {
        let file = File::open(archive_path)?;
        let tar = flate2::read::GzDecoder::new(file);
        tar::Archive::new(tar).unpack(dest_dir)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── pure target-path derivation ────────────────────────────────────────

    #[test]
    fn runtime_dir_is_pure_and_platform_scoped() {
        let base = Path::new("/app-data/tts/piper");
        assert_eq!(
            runtime_dir(base, "windows-x64"),
            Path::new("/app-data/tts/piper/windows-x64")
        );
        assert_eq!(
            runtime_dir(base, "macos-aarch64"),
            Path::new("/app-data/tts/piper/macos-aarch64")
        );
    }

    #[test]
    fn voice_paths_place_onnx_and_sidecar_json_in_the_voices_dir() {
        let base = Path::new("/app-data/tts/piper");
        let (onnx, json) = voice_paths(base, "de_DE-thorsten-high");
        assert_eq!(
            onnx,
            Path::new("/app-data/tts/piper/voices/de_DE-thorsten-high.onnx")
        );
        assert_eq!(
            json,
            Path::new("/app-data/tts/piper/voices/de_DE-thorsten-high.onnx.json")
        );
    }

    #[test]
    fn language_of_voice_id_extracts_the_primary_subtag() {
        assert_eq!(
            language_of_voice_id("de_DE-thorsten-high").as_deref(),
            Some("de")
        );
        assert_eq!(
            language_of_voice_id("en_US-lessac-medium").as_deref(),
            Some("en")
        );
        assert_eq!(language_of_voice_id(""), None);
    }

    #[test]
    fn current_platform_is_one_of_the_engine_contracts_ids_or_none() {
        assert!(matches!(
            current_platform(),
            None | Some("windows-x64")
                | Some("macos-aarch64")
                | Some("macos-x64")
                | Some("linux-x64")
        ));
    }

    // ── list_downloads reflects the catalog AND real on-disk completeness ───
    // Drives the actual `build_downloads` (the pure core of `list_downloads`)
    // against a tempdir fixture — no AppHandle needed, since the live
    // downloading-flag lookup is injected as a plain closure.

    #[test]
    fn build_downloads_orders_runtime_first_and_lists_all_five_voices() {
        let dir = tempfile::TempDir::new().unwrap();
        let piper_dir = dir.path().join("piper");

        let downloads = TtsModelManager::build_downloads(&piper_dir, |_| false);

        let voice_rows: Vec<_> = downloads
            .iter()
            .filter(|d| d.kind == TtsDownloadKind::Voice)
            .collect();
        assert_eq!(voice_rows.len(), 5, "expected the 5 curated Piper voices");
        assert!(
            voice_rows.iter().all(|d| !d.is_downloaded),
            "nothing on disk yet — no voice may read as downloaded"
        );

        if current_platform().is_some() {
            assert_eq!(
                downloads[0].kind,
                TtsDownloadKind::Runtime,
                "the runtime row belongs at the top"
            );
            assert_eq!(downloads[0].id, RUNTIME_ID);
            assert!(!downloads[0].is_downloaded);
        } else {
            assert_eq!(
                downloads.len(),
                5,
                "no runtime row at all on a platform current_platform() doesn't recognise"
            );
        }
    }

    #[test]
    fn build_downloads_needs_both_voice_files_not_just_the_onnx() {
        let dir = tempfile::TempDir::new().unwrap();
        let piper_dir = dir.path().join("piper");
        let voice_id = &catalog::tts_entries(Purpose::TtsVoice)[0].id.clone();
        let (onnx_path, json_path) = voice_paths(&piper_dir, voice_id);
        fs::create_dir_all(onnx_path.parent().unwrap()).unwrap();

        fs::write(&onnx_path, b"fake onnx").unwrap();
        let downloads = TtsModelManager::build_downloads(&piper_dir, |_| false);
        let row = downloads.iter().find(|d| &d.id == voice_id).unwrap();
        assert!(
            !row.is_downloaded,
            "the .onnx alone must not count as installed"
        );

        fs::write(&json_path, b"{}").unwrap();
        let downloads = TtsModelManager::build_downloads(&piper_dir, |_| false);
        let row = downloads.iter().find(|d| &d.id == voice_id).unwrap();
        assert!(
            row.is_downloaded,
            ".onnx + .onnx.json together must count as installed"
        );
    }

    #[test]
    fn build_downloads_needs_the_binary_and_espeak_data_not_just_the_directory() {
        let Some(platform) = current_platform() else {
            return; // nothing to assert on a platform with no runtime entry
        };
        let dir = tempfile::TempDir::new().unwrap();
        let piper_dir = dir.path().join("piper");
        let rt_dir = runtime_dir(&piper_dir, platform);

        // An interrupted/empty extraction — the directory exists, nothing in it.
        fs::create_dir_all(&rt_dir).unwrap();
        let downloads = TtsModelManager::build_downloads(&piper_dir, |_| false);
        assert!(
            !downloads[0].is_downloaded,
            "an empty runtime directory must not read as installed"
        );

        // Binary present, espeak-ng-data still missing.
        let binary_name = if platform.starts_with("windows") {
            "piper.exe"
        } else {
            "piper"
        };
        fs::write(rt_dir.join(binary_name), b"fake binary").unwrap();
        let downloads = TtsModelManager::build_downloads(&piper_dir, |_| false);
        assert!(
            !downloads[0].is_downloaded,
            "the binary alone, without espeak-ng-data, must not read as installed"
        );

        // Both present: now it's complete.
        fs::create_dir_all(rt_dir.join("espeak-ng-data")).unwrap();
        let downloads = TtsModelManager::build_downloads(&piper_dir, |_| false);
        assert!(
            downloads[0].is_downloaded,
            "binary + espeak-ng-data together must count as installed"
        );
    }

    #[test]
    fn build_downloads_reports_the_injected_downloading_flag() {
        let dir = tempfile::TempDir::new().unwrap();
        let piper_dir = dir.path().join("piper");
        let voice_id = catalog::tts_entries(Purpose::TtsVoice)[0].id.clone();

        let downloads = TtsModelManager::build_downloads(&piper_dir, |id| id == voice_id);
        let row = downloads.iter().find(|d| d.id == voice_id).unwrap();
        assert!(row.is_downloading);
        let other = downloads.iter().find(|d| d.id != voice_id).unwrap();
        assert!(!other.is_downloading);
    }

    // ── SHA-mismatch on the REUSED downloader is still an error ─────────────
    // Proves the reuse actually wires into real verification, not just that
    // the (already exhaustively tested) transport itself works — that's
    // covered by `managers::model::download::tests`.

    #[tokio::test]
    async fn sha_mismatch_on_the_reused_downloader_is_an_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let body = b"totally not a real piper archive";
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match sock.read(&mut byte).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => head.push(byte[0]),
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.write_all(body).await;
        });

        let dir = tempfile::TempDir::new().unwrap();
        let partial = dir.path().join("archive.partial");
        let wrong_hash = "0".repeat(64);

        let err = ModelManager::download_http_resumable_with_events(
            RUNTIME_ID,
            &format!("http://{addr}/file"),
            &partial,
            Some(body.len() as u64),
            Some(&wrong_hash),
            &CancellationToken::new(),
            &|_| {},
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("corrupt"), "{err}");
        assert!(
            !partial.exists(),
            "a failed verification must clear the partial, same as the ASR path"
        );
    }
}
