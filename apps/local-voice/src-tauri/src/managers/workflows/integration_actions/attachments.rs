//! Anhaenge der Mail (B5): nur Dateien in den Ordner-Integrationen des Nutzers.
//!
//! Ein Anhang ist ein Pfad (meist `steps.<schritt>.path` eines Exports). Er wird geprueft, bevor
//! das Tor die Mail sieht, und gelesen, wenn sie gesendet wird:
//!
//! 1. absolut, ohne Steuerzeichen, hoechstens 500 Zeichen; kein Platzhalter `{{...}}` (der Lauf
//!    hat Verweise laengst eingesetzt);
//! 2. eine vorhandene gewoehnliche Datei, KEIN Symlink, kein Platzhalter der Cloud
//!    (OneDrive „nur online“: das Lesen wuerde einen Download ausloesen);
//! 3. nach `canonicalize` unter der Wurzel einer eingeschalteten Ordner-Integration
//!    (`integrations::folder::Sandbox`: `..`, Junctions und Symlinks nach aussen zaehlen nicht);
//! 4. das Lesen darf nicht `aus` sein (`files.read` fuer Workflows an diesem Ordner); steht es auf
//!    „fragen“, verlangt die Mail selbst eine Freigabe (`Checked::read_mode`);
//! 5. hoechstens [`MAX_ATTACHMENTS`] Dateien, zusammen hoechstens `MAX_TOTAL_BYTES`, keine leere.
//!
//! Der Anhang ist an den Inhalt gebunden: Groesse und Pruefsumme stehen in den Argumenten des
//! Tors (und damit in der Freigabe). Aendert sich die Datei, bis der Nutzer entschieden hat,
//! passt die Bindung nicht mehr, und der Schritt wird abgelehnt.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::managers::integrations::folder::{FolderConfig, Sandbox};
use crate::managers::integrations::grants::explain;
use crate::managers::integrations::model::{Caller, Capability, GrantMode, Kind};
use crate::managers::integrations::smtp;
use crate::managers::integrations::store as integrations_store;

pub const MAX_ATTACHMENTS: usize = smtp::MAX_ATTACHMENTS;
/// Zusammen hoechstens so gross (Mail ueber Graph oder SMTP; siehe dort).
pub const MAX_TOTAL_BYTES: u64 = smtp::MAX_ATTACHMENT_BYTES as u64;
const MAX_PATH_CHARS: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachError {
    /// Pfad leer, relativ, mit Steuerzeichen oder zu lang.
    BadPath(String),
    Missing(String),
    /// Kein gewoehnliche Datei (Ordner, Symlink, Cloud-Platzhalter).
    NotAFile(String, &'static str),
    /// Liegt in keinem erlaubten Ordner.
    Outside(String),
    /// Der Ordner ist da, das Lesen fuer Workflows aber ausgeschaltet.
    ReadOff {
        name: String,
        folder: String,
        reason: &'static str,
    },
    TooMany(usize),
    TooBig,
    Empty(String),
    /// Lesen scheiterte (Datei gesperrt): es wurde nichts gesendet.
    Io(String),
    /// Die Datei ist nicht mehr die geprueft-gebundene (Groesse oder Pruefsumme).
    Changed(String),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttachError::BadPath(p) => write!(
                f,
                "Der Anhang „{p}“ ist kein gültiger Pfad (vollständiger Pfad ohne Sonderzeichen nötig)."
            ),
            AttachError::Missing(n) => write!(f, "Der Anhang „{n}“ gibt es nicht."),
            AttachError::NotAFile(n, why) => {
                write!(f, "Der Anhang „{n}“ ist keine gewöhnliche Datei ({why}).")
            }
            AttachError::Outside(n) => write!(
                f,
                "Der Anhang „{n}“ liegt in keinem Ordner, den du als Ordner-Integration eingerichtet hast; er wird nicht gesendet."
            ),
            AttachError::ReadOff { name, folder, .. } => write!(
                f,
                "Der Anhang „{name}“ liegt im Ordner „{folder}“, dessen Lesen für Abläufe ausgeschaltet ist."
            ),
            AttachError::TooMany(n) => write!(
                f,
                "Zu viele Anhänge ({n}, höchstens {MAX_ATTACHMENTS})."
            ),
            AttachError::TooBig => write!(
                f,
                "Die Anhänge sind zu groß (höchstens {} KiB zusammen).",
                MAX_TOTAL_BYTES / 1024
            ),
            AttachError::Empty(n) => write!(f, "Der Anhang „{n}“ ist leer."),
            AttachError::Io(m) => write!(f, "Der Anhang ließ sich nicht lesen ({m})."),
            AttachError::Changed(n) => write!(
                f,
                "Der Anhang „{n}“ hat sich geändert, seit er geprüft wurde."
            ),
        }
    }
}

impl std::error::Error for AttachError {}

/// Ein geprueft-gelesener Anhang.
#[derive(Clone, PartialEq, Eq)]
pub struct Checked {
    /// Der Pfad, wie er in Vorschau und Protokoll steht (ohne `\\?\`).
    pub display: String,
    pub name: String,
    pub content_type: &'static str,
    /// Kennung der Ordner-Integration, unter der er liegt.
    pub folder: String,
    /// Recht zum Lesen fuer Workflows an diesem Ordner.
    pub read_mode: GrantMode,
    pub size: u64,
    /// Erste 16 Hexzeichen von SHA-256 (zur Bindung der Freigabe, kein Sicherheitsmerkmal).
    pub sha256: String,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for Checked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Nie den Inhalt in ein Protokoll.
        write!(
            f,
            "Checked {{ name: {:?}, size: {}, sha256: {} }}",
            self.name, self.size, self.sha256
        )
    }
}

fn content_type_of(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("pdf") => "application/pdf",
        Some("md") => "text/markdown",
        Some("txt") => "text/plain",
        Some("html") => "text/html",
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

/// `\\?\C:\x` -> `C:\x` (die Pruefung kanonisiert; so steht der Pfad in Vorschau und Protokoll).
fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.chars().nth(1) == Some(':') => rest.to_string(),
        _ => s,
    }
}

fn file_name_of(path: &Path) -> String {
    let raw = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    raw.chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .collect()
}

#[cfg(windows)]
fn is_cloud_only(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    crate::managers::workflows::trigger::folder::attributes_cloud_only(meta.file_attributes())
}

#[cfg(not(windows))]
fn is_cloud_only(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Der Ordner, unter dem die Datei liegt (kanonischer Pfad) samt Recht zum Lesen.
fn folder_of(
    conn: &rusqlite::Connection,
    canonical: &Path,
    name: &str,
) -> Result<(String, GrantMode), AttachError> {
    let all = integrations_store::list(conn).map_err(|e| AttachError::Io(e.to_string()))?;
    let mut refused: Option<(String, &'static str)> = None;
    for i in all.iter().filter(|i| i.kind == Kind::Folder && i.enabled) {
        let Ok(cfg) = FolderConfig::from_config_json(&i.config_json) else {
            continue;
        };
        let Ok(sandbox) = Sandbox::open(&cfg.path) else {
            continue;
        };
        if !sandbox.contains(canonical) {
            continue;
        }
        let grants = integrations_store::grants_for(conn, &i.id)
            .map_err(|e| AttachError::Io(e.to_string()))?;
        let (mode, reason) = explain(i, Capability::FilesRead, Caller::Workflow, &grants, None);
        if mode == GrantMode::Off {
            refused.get_or_insert((
                i.label.clone(),
                reason.map(|r| r.as_str()).unwrap_or("grant_off"),
            ));
            continue;
        }
        return Ok((i.id.clone(), mode));
    }
    match refused {
        Some((folder, reason)) => Err(AttachError::ReadOff {
            name: name.to_string(),
            folder,
            reason,
        }),
        None => Err(AttachError::Outside(name.to_string())),
    }
}

/// Prueft und liest die Anhaenge (siehe Moduldoku). Leer: keine.
pub fn check(conn: &rusqlite::Connection, paths: &[String]) -> Result<Vec<Checked>, AttachError> {
    if paths.len() > MAX_ATTACHMENTS {
        return Err(AttachError::TooMany(paths.len()));
    }
    let mut out: Vec<Checked> = Vec::new();
    let mut total: u64 = 0;
    for raw in paths {
        let text = raw.trim();
        let shown: String = text.chars().take(80).collect();
        let path = Path::new(text);
        if text.is_empty()
            || text.chars().count() > MAX_PATH_CHARS
            || text.chars().any(char::is_control)
            || text.contains("{{")
            || !path.is_absolute()
        {
            return Err(AttachError::BadPath(shown));
        }
        let name = file_name_of(path);
        let meta = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AttachError::Missing(name));
            }
            Err(e) => return Err(AttachError::Io(e.to_string())),
        };
        if meta.file_type().is_symlink() {
            return Err(AttachError::NotAFile(name, "Verknüpfung"));
        }
        if !meta.is_file() {
            return Err(AttachError::NotAFile(name, "kein Dokument"));
        }
        if is_cloud_only(&meta) {
            return Err(AttachError::NotAFile(
                name,
                "nur in der Cloud, nicht auf diesem Rechner",
            ));
        }
        let canonical: PathBuf =
            std::fs::canonicalize(path).map_err(|e| AttachError::Io(e.to_string()))?;
        let (folder, read_mode) = folder_of(conn, &canonical, &name)?;
        if meta.len() == 0 {
            return Err(AttachError::Empty(name));
        }
        total = total.saturating_add(meta.len());
        if total > MAX_TOTAL_BYTES {
            return Err(AttachError::TooBig);
        }
        let bytes = std::fs::read(&canonical).map_err(|e| AttachError::Io(e.to_string()))?;
        // Waechst die Datei zwischen Pruefung und Lesen, zaehlt, was gelesen wurde.
        if bytes.len() as u64 > MAX_TOTAL_BYTES || bytes.len() as u64 != meta.len() {
            return Err(AttachError::Changed(name));
        }
        let digest = Sha256::digest(&bytes);
        let sha256: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        out.push(Checked {
            display: display_path(&canonical),
            content_type: content_type_of(&name),
            name,
            folder,
            read_mode,
            size: bytes.len() as u64,
            sha256,
            bytes,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
