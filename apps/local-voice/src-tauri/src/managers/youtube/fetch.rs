//! Gemeinsame Bausteine der yt-dlp-Wege (A3): Zugang (Schalter „privat“ + Programm
//! gefunden), Tor und Audit `media.fetch`, Temp-Ordner, Einordnung von Fehlern und
//! der Audio-Download fuer die eigene Transkription.
//!
//! Alles hier ist der bewusste, vom Nutzer ausgeloeste Netzverkehr ueber ein selbst
//! installiertes Programm (E1, Variante B'). Davon steht jeder Schritt im Audit
//! (`media.fetch`, Ziel `youtube:<ID>`, Phase `subtitles`/`audio`), und ohne
//! schreibbares Audit geht nichts hinaus (fail closed, wie im Tor aus A1).
//!
//! Temp-Dateien: jeder Lauf bekommt einen eigenen Ordner `lva-yt-*` unter dem
//! Temp-Verzeichnis des Benutzers; `TempDir` loescht ihn beim Drop (Erfolg, Fehler,
//! Stopp, Panik mit Unwinding). Ein harter Absturz laesst ihn liegen: der naechste
//! Lauf raeumt Ordner mit diesem Praefix, die aelter als 24 Stunden sind, weg
//! (nur Ordner, nie Verknuepfungen, nie andere Namen).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use serde_json::json;

use super::run::{self, RunEnd, RunOpts};
use super::subtitles::{base_args, watch_url};
use super::tool::ToolStatus;
use super::{YoutubeError, INTEGRATION_ID};
use crate::managers::integrations::audit;
use crate::managers::integrations::gate::{self, Decision, Request};
use crate::managers::integrations::model::{AuditOutcome, Caller, Capability, NewAudit};
use crate::managers::integrations::store as register;
use crate::managers::meetings::store::MeetingStore;

pub const TEMP_PREFIX: &str = "lva-yt-";
const STALE_AFTER: Duration = Duration::from_secs(24 * 3600);
/// Laengster Audio-Download (eine Sitzung von mehreren Stunden bei guter Leitung).
pub const AUDIO_TIMEOUT: Duration = Duration::from_secs(45 * 60);
/// Groesste Audiodatei (yt-dlp `--max-filesize`): schuetzt den Datentraeger.
pub const MAX_AUDIO_SIZE: &str = "500M";

/// Das Programm und wohin es Dateien legen darf.
pub struct ToolRun<'a> {
    pub exe: &'a Path,
    /// `None`: Temp-Verzeichnis des Benutzers (Tests geben einen eigenen Ordner).
    pub temp_base: Option<&'a Path>,
}

/// Darf der Weg benutzt werden, und mit welchem Programm?
pub fn resolve(private: bool, status: &ToolStatus) -> Result<PathBuf, YoutubeError> {
    if !private {
        return Err(YoutubeError::PrivateOff);
    }
    if !status.found {
        return Err(YoutubeError::ToolMissing);
    }
    status
        .path
        .as_deref()
        .map(PathBuf::from)
        .ok_or(YoutubeError::ToolMissing)
}

// ---------------------------------------------------------------------------
// Temp-Ordner
// ---------------------------------------------------------------------------

pub struct TempDir {
    dir: tempfile::TempDir,
}

impl TempDir {
    pub fn create(base: Option<&Path>) -> Result<Self, YoutubeError> {
        let root = base
            .map(Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir);
        sweep_stale(&root, STALE_AFTER);
        let dir = tempfile::Builder::new()
            .prefix(TEMP_PREFIX)
            .tempdir_in(&root)
            .map_err(|e| YoutubeError::Store(format!("Temp-Ordner: {}", e.kind())))?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// Raeumt liegengebliebene Ordner eines abgestuerzten Laufs weg. Gibt die Zahl der
/// entfernten Ordner zurueck.
pub fn sweep_stale(root: &Path, older_than: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(TEMP_PREFIX) {
            continue;
        }
        // Nie einer Verknuepfung folgen, nie eine Datei anfassen.
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        let age = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .unwrap_or_default();
        if age >= older_than && std::fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

// ---------------------------------------------------------------------------
// Fehler einordnen
// ---------------------------------------------------------------------------

/// Ein beendeter oder abgebrochener Lauf als Fehler. `Finished` mit Code 0 gehoert
/// dem Aufrufer; kommt er doch hier an, heisst das ein Fehler ohne Code.
pub fn classify(end: RunEnd) -> YoutubeError {
    match end {
        RunEnd::TimedOut => YoutubeError::Timeout,
        RunEnd::Cancelled => YoutubeError::Cancelled,
        RunEnd::Finished {
            code, stderr_tail, ..
        } => {
            let text = stderr_tail.to_ascii_lowercase();
            if [
                "private video",
                "video unavailable",
                "this video is",
                "members-only",
            ]
            .iter()
            .any(|p| text.contains(p))
            {
                YoutubeError::Unavailable
            } else if [
                "unable to extract",
                "http error 403",
                "nsig",
                "please update",
                "update yt-dlp",
                "player response",
                "po token",
            ]
            .iter()
            .any(|p| text.contains(p))
            {
                YoutubeError::ToolOutdated
            } else {
                YoutubeError::ToolFailed(code)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tor und Audit
// ---------------------------------------------------------------------------

/// Der offene Audit-Eintrag eines Laufs.
pub struct Guard {
    integration_id: String,
    audit_id: i64,
}

/// Tor `media.fetch` fuer den Nutzer und Audit `pending` VOR dem ersten Byte.
/// `Err` heisst: nichts geht hinaus.
pub fn begin(store: &MeetingStore, video_id: &str, phase: &str) -> Result<Guard, YoutubeError> {
    let now = chrono::Utc::now().timestamp_millis();
    let conn = store
        .get_connection()
        .map_err(|e| YoutubeError::Store(e.to_string()))?;
    let integration = super::source::ensure_integration(&conn, now)?;
    let decision = gate::check(
        &conn,
        &Request {
            caller: Caller::User,
            integration_id: &integration.id,
            capability: Capability::MediaFetch,
            target: None,
            args: None,
            tool_mode: None,
        },
        now,
    )?;
    match decision {
        Decision::Allowed => {}
        Decision::Denied { reason, .. } => return Err(YoutubeError::Disabled(reason)),
        Decision::NeedsApproval { .. } => {
            return Err(YoutubeError::Disabled(
                "Für diesen Schritt ist eine Freigabe nötig.".to_string(),
            ))
        }
    }
    let audit_id = audit::record_at(
        &conn,
        &NewAudit {
            caller: Caller::User.as_str().to_string(),
            integration_id: Some(integration.id.clone()),
            capability: Some(Capability::MediaFetch.as_str().to_string()),
            target: Some(format!("youtube:{video_id}")),
            outcome: AuditOutcome::Pending,
            detail: Some(json!({ "phase": phase, "tool": "yt-dlp" })),
        },
        now,
    )?;
    Ok(Guard {
        integration_id: integration.id,
        audit_id,
    })
}

/// Ergebnis festhalten. Ein Fehler hier ist nur eine Warnung: der Lauf ist
/// vorbei, der Eintrag bleibt dann `pending` (sichtbar, Semantik des Tores).
pub fn end(store: &MeetingStore, guard: Guard, result: &Result<(), YoutubeError>) {
    let now = chrono::Utc::now().timestamp_millis();
    let conn = match store.get_connection() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("youtube: Ergebnis nicht festgehalten: {e}");
            return;
        }
    };
    let (outcome, detail) = match result {
        Ok(()) => (AuditOutcome::Ok, json!({ "phase": "done" })),
        Err(e) => (
            AuditOutcome::Error,
            json!({ "phase": "done", "error": e.code() }),
        ),
    };
    if let Err(e) = audit::set_outcome(&conn, guard.audit_id, outcome, Some(detail)) {
        log::warn!("youtube: Audit-Ergebnis nicht gesetzt: {e}");
    }
    let marked = match result {
        Ok(()) => register::mark_ok(&conn, &guard.integration_id, now),
        Err(e) => register::mark_error(&conn, &guard.integration_id, &e.to_string(), now),
    };
    if let Err(e) = marked {
        log::warn!("youtube: Register nicht aktualisiert: {e}");
    }
    debug_assert_eq!(guard.integration_id, INTEGRATION_ID);
}

// ---------------------------------------------------------------------------
// Audio fuer die eigene Transkription
// ---------------------------------------------------------------------------

/// Die heruntergeladene Audiodatei; der Ordner verschwindet mit diesem Wert.
pub struct DownloadedAudio {
    pub file: PathBuf,
    _tmp: TempDir,
}

pub fn audio_args(video_id: &str) -> Vec<String> {
    let mut a = base_args();
    a.extend([
        "-f".into(),
        "bestaudio/best".into(),
        "--max-filesize".into(),
        MAX_AUDIO_SIZE.into(),
        "--no-part".into(),
        "--restrict-filenames".into(),
        "--no-mtime".into(),
        "-o".into(),
        "%(id)s.%(ext)s".into(),
    ]);
    a.extend(["--".into(), watch_url(video_id)]);
    a
}

/// Laedt die beste Audiospur in einen Temp-Ordner (Stopp ueber `cancel`).
pub fn download_audio(
    tool: &ToolRun<'_>,
    video_id: &str,
    cancel: &AtomicBool,
) -> Result<DownloadedAudio, YoutubeError> {
    let tmp = TempDir::create(tool.temp_base)?;
    let end = run::run(
        tool.exe,
        &audio_args(video_id),
        &RunOpts {
            timeout: AUDIO_TIMEOUT,
            max_stdout: 4096,
            cancel: Some(cancel),
            cwd: Some(tmp.path()),
        },
    )
    .map_err(YoutubeError::ToolStart)?;
    match end {
        RunEnd::Finished { code: 0, .. } => {}
        other => return Err(classify(other)),
    }
    // Die groesste fertige Datei (keine Reste wie `.part`, `.ytdl`).
    let file = std::fs::read_dir(tmp.path())
        .map_err(|e| YoutubeError::Store(e.to_string()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && !p
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| matches!(e, "part" | "ytdl" | "tmp"))
        })
        .max_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .ok_or_else(|| YoutubeError::BadResponse("keine Audiodatei".to_string()))?;
    Ok(DownloadedAudio { file, _tmp: tmp })
}
