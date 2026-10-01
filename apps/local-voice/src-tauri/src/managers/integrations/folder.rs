//! Ordner-Ziel (A6, Goal „Integrationen“): Dateien in einen vom Nutzer
//! gewaehlten Ordner ablegen, ohne dass irgendetwas ausserhalb dieses Ordners
//! beschrieben wird.
//!
//! Die Pfad-Sandbox ist der Kern dieses Moduls und wird auch vom Obsidian-Vault
//! (`obsidian`) benutzt. Regeln, von aussen nach innen:
//!
//! 1. **Wurzel**: absolut, vorhanden, ein Ordner; sie wird kanonisiert
//!    (`fs::canonicalize`), also ohne `..`, ohne Verknuepfungen im Pfad bis zur
//!    Wurzel, mit der tatsaechlichen Gross-/Kleinschreibung. Ist die Wurzel selbst
//!    eine Junction, ist das die Entscheidung des Nutzers: gearbeitet wird in ihrem
//!    Ziel.
//! 2. **Relative Pfade** (`resolve_dir`, `resolve_file`): kein `..`, kein
//!    Laufwerk/UNC/Wurzel (`C:x`, `\\server\share`, `/etc`), kein `:` (Laufwerk und
//!    NTFS-Datenstrom `datei:strom`), keine Steuerzeichen, keine Windows-Geraetenamen
//!    (`CON`, `NUL`, `COM1` ...), kein Punkt oder Leerzeichen am Ende. Nichts davon
//!    wird „repariert“, es wird abgelehnt.
//! 3. **Verknuepfungen**: Jede Ebene wird nach dem Anlegen oder Finden erneut
//!    kanonisiert und muss unter der kanonischen Wurzel liegen (`Path::starts_with`
//!    vergleicht Komponenten, `C:\Ablage-boese` liegt also nicht unter `C:\Ablage`).
//!    Eine Junction oder ein Symlink, die nach aussen zeigen, werden dadurch
//!    abgelehnt; eine, die innerhalb der Wurzel bleibt, ist harmlos und erlaubt
//!    (OneDrive-Platzhalter sind solche Reparse-Punkte). Eine Datei, die selbst ein
//!    Symlink ist, wird nie ueberschrieben.
//! 4. **Schreiben**: der Name wird exklusiv angelegt (`create_new`), nie wird eine
//!    vorhandene Datei ueberschrieben (`name (2).md`). Nach dem Schreiben wird der
//!    Pfad noch einmal geprueft; scheitert das oder der Schreiber, bleibt keine
//!    Datei zurueck.
//!
//! Restrisiko (benannt, nicht geloest): Zwischen Pruefung und Schreiben kann ein
//! anderer Prozess mit Schreibrecht im Ordner eine Junction austauschen. Wer das
//! kann, kann im Ordner ohnehin beliebig schreiben; die Nachpruefung entfernt eine
//! so entstandene Datei wieder, ein Abfluss vor der Pruefung laesst sich ohne
//! handle-basierte Dateisystem-API nicht ausschliessen.
//!
//! Speicher: Dateien sind auf `MAX_FILE_BYTES` begrenzt; eine voll gelaufene Platte
//! bricht das Schreiben mit einem Fehler ab (die halbe Datei wird entfernt).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Groesste Datei, die dieses Ziel schreibt (Protokolle, Dokumente; kein Audio).
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
/// Laengster einzelner Pfadbestandteil.
pub const MAX_COMPONENT_CHARS: usize = 120;
/// Laengster relativer Pfad.
pub const MAX_REL_CHARS: usize = 240;
/// So viele „name (n).ext“ werden probiert, bevor es aufgegeben wird.
const MAX_UNIQUE_TRIES: u32 = 200;

/// Windows-Geraetenamen, auch mit Endung gesperrt (`nul.txt`).
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "CONIN$",
    "CONOUT$",
];

/// Konfiguration einer Ordner-Integration (`config_json`, ohne Geheimnis).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderConfig {
    /// Wurzel: absoluter Pfad.
    pub path: String,
    /// Unterordner in der Wurzel, in den Exporte kommen (leer: die Wurzel selbst).
    #[serde(default)]
    pub subfolder: String,
}

impl FolderConfig {
    pub fn from_config_json(json: &str) -> Result<Self, FolderError> {
        let v: Value = serde_json::from_str(json).map_err(|_| FolderError::Config)?;
        let path = v
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let subfolder = v
            .get("subfolder")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if path.is_empty() {
            return Err(FolderError::RootMissing);
        }
        Ok(Self { path, subfolder })
    }
}

/// Warum eine Datei nicht geschrieben wurde. `code()` ist der Schluessel der
/// Oberflaeche, `Display` der Klartext fuer Protokoll und Meldungen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderError {
    Config,
    RootMissing,
    RootRelative,
    RootNotFound,
    RootNotAFolder,
    /// Pfad verlaesst die Wurzel oder ist nicht zulaessig (Grund als Code).
    Escape(&'static str),
    /// Dateiname ist leer oder nach der Bereinigung unbrauchbar.
    BadName,
    TooLarge,
    NoFreeName,
    Io(String),
}

impl FolderError {
    pub fn code(&self) -> &'static str {
        match self {
            FolderError::Config => "folder_config_invalid",
            FolderError::RootMissing => "folder_path_missing",
            FolderError::RootRelative => "folder_path_relative",
            FolderError::RootNotFound => "folder_path_not_found",
            FolderError::RootNotAFolder => "folder_path_not_a_folder",
            FolderError::Escape(_) => "folder_path_escape",
            FolderError::BadName => "folder_name_invalid",
            FolderError::TooLarge => "folder_file_too_large",
            FolderError::NoFreeName => "folder_no_free_name",
            FolderError::Io(_) => "folder_io",
        }
    }
}

impl std::fmt::Display for FolderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FolderError::Config => write!(f, "Die Ordner-Einstellungen sind unvollständig."),
            FolderError::RootMissing => write!(f, "Es ist kein Ordner angegeben."),
            FolderError::RootRelative => write!(f, "Der Pfad muss vollständig sein (zum Beispiel C:\\Ablage)."),
            FolderError::RootNotFound => write!(f, "Der Ordner wurde nicht gefunden."),
            FolderError::RootNotAFolder => write!(f, "Der Pfad ist kein Ordner."),
            FolderError::Escape(why) => write!(
                f,
                "Der Pfad ist nicht zulässig ({why}): geschrieben wird nur innerhalb des gewählten Ordners."
            ),
            FolderError::BadName => write!(f, "Der Dateiname ist nicht zulässig."),
            FolderError::TooLarge => write!(f, "Die Datei ist zu groß für dieses Ziel."),
            FolderError::NoFreeName => write!(f, "Es gibt schon zu viele Dateien mit diesem Namen."),
            FolderError::Io(m) => write!(f, "Schreiben nicht möglich: {m}"),
        }
    }
}

impl std::error::Error for FolderError {}

fn io_err(e: std::io::Error) -> FolderError {
    FolderError::Io(e.to_string())
}

/// Eine abgelegte Datei, fuer Protokoll und Anzeige.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacedFile {
    /// Pfad relativ zur Wurzel, mit `/` (nie der absolute Pfad im Protokoll).
    pub rel: String,
    pub path: PathBuf,
    pub bytes: u64,
}

/// Die Sandbox: eine kanonische Wurzel, unter der alles liegen muss.
#[derive(Clone, Debug)]
pub struct Sandbox {
    root: PathBuf,
}

/// Prueft den Wurzel-Pfad einer Einstellung (ohne ihn zu kanonisieren) und liefert
/// den bereinigten Text. Dieselben Codes wie beim Anlegen in der Oberflaeche.
pub fn check_root_text(raw: &str) -> Result<String, FolderError> {
    let path = raw.trim();
    if path.is_empty() {
        return Err(FolderError::RootMissing);
    }
    if path.contains('\0') {
        return Err(FolderError::Escape("Steuerzeichen"));
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(FolderError::RootRelative);
    }
    match std::fs::metadata(p) {
        Err(_) => Err(FolderError::RootNotFound),
        Ok(m) if !m.is_dir() => Err(FolderError::RootNotAFolder),
        Ok(_) => Ok(path.to_string()),
    }
}

/// Prueft einen relativen Pfad rein formal (kein `..`, kein Laufwerk, keine
/// Sonderzeichen), ohne das Dateisystem anzufassen: fuer Einstellungen, deren
/// Ordner erst beim ersten Schreiben entsteht.
pub fn check_relative(rel: &str) -> Result<(), FolderError> {
    Sandbox::components(rel).map(|_| ())
}

impl Sandbox {
    /// Oeffnet die Sandbox an einer Wurzel (kanonisiert sie).
    pub fn open(root: &str) -> Result<Self, FolderError> {
        let checked = check_root_text(root)?;
        let canonical = std::fs::canonicalize(&checked).map_err(|_| FolderError::RootNotFound)?;
        Ok(Self { root: canonical })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Liegt `path` (kanonisch) unter der Wurzel?
    pub fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.root)
    }

    fn components(rel: &str) -> Result<Vec<String>, FolderError> {
        if rel.chars().count() > MAX_REL_CHARS {
            return Err(FolderError::Escape("Pfad zu lang"));
        }
        if rel.contains('\0') {
            return Err(FolderError::Escape("Steuerzeichen"));
        }
        // Laufwerk, UNC, Wurzel: nie relativ.
        let lead = rel.trim_start();
        if lead.starts_with('/') || lead.starts_with('\\') {
            return Err(FolderError::Escape("absoluter Pfad"));
        }
        let mut out = Vec::new();
        for part in rel.split(['/', '\\']) {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                return Err(FolderError::Escape("übergeordneter Ordner"));
            }
            check_component(part)?;
            out.push(part.to_string());
        }
        Ok(out)
    }

    /// Ein Ordner unter der Wurzel; `create`: fehlende Ebenen anlegen. Jede
    /// Ebene wird nach dem Anlegen kanonisiert und gegen die Wurzel geprueft.
    pub fn resolve_dir(&self, rel: &str, create: bool) -> Result<PathBuf, FolderError> {
        let mut current = self.root.clone();
        for part in Self::components(rel)? {
            let next = current.join(&part);
            match std::fs::symlink_metadata(&next) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if !create {
                        return Err(FolderError::RootNotFound);
                    }
                    std::fs::create_dir(&next).map_err(io_err)?;
                }
                Err(e) => return Err(io_err(e)),
            }
            let canonical = std::fs::canonicalize(&next).map_err(io_err)?;
            if !self.contains(&canonical) {
                return Err(FolderError::Escape("Verknüpfung nach außen"));
            }
            if !canonical.is_dir() {
                return Err(FolderError::Escape("kein Ordner"));
            }
            current = canonical;
        }
        Ok(current)
    }

    /// Pfad einer (neuen oder vorhandenen) Datei unter der Wurzel. Der Ordner wird
    /// aufgeloest und geprueft; die Datei selbst darf kein Symlink sein und muss,
    /// falls vorhanden, kanonisch unter der Wurzel liegen.
    pub fn resolve_file(&self, rel: &str, create_dirs: bool) -> Result<PathBuf, FolderError> {
        let mut parts = Self::components(rel)?;
        let Some(file) = parts.pop() else {
            return Err(FolderError::BadName);
        };
        let dir = self.resolve_dir(&parts.join("/"), create_dirs)?;
        let target = dir.join(&file);
        match std::fs::symlink_metadata(&target) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    return Err(FolderError::Escape("Verknüpfung"));
                }
                let canonical = std::fs::canonicalize(&target).map_err(io_err)?;
                if !self.contains(&canonical) {
                    return Err(FolderError::Escape("Verknüpfung nach außen"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(e)),
        }
        Ok(target)
    }

    /// Relativer Pfad mit `/` fuer Protokoll und Anzeige.
    pub fn rel_of(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .map(|p| {
                p.components()
                    .filter_map(|c| match c {
                        Component::Normal(s) => s.to_str().map(str::to_string),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default()
    }

    /// Legt eine NEUE Datei an (`rel_dir/<name>`, bei Kollision `name (2).ext`) und
    /// laesst `writer` sie fuellen. Der Schreiber bekommt den Pfad (Exporte wie PDF
    /// schreiben selbst). Scheitert er oder die Nachpruefung, wird die Datei entfernt.
    pub fn write_new(
        &self,
        rel_dir: &str,
        file_name: &str,
        writer: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<PlacedFile, FolderError> {
        let dir = self.resolve_dir(rel_dir, true)?;
        let name = sanitize_file_name(file_name).ok_or(FolderError::BadName)?;
        let (stem, ext) = split_name(&name);
        let mut reserved: Option<PathBuf> = None;
        for n in 0..MAX_UNIQUE_TRIES {
            let candidate = if n == 0 {
                name.clone()
            } else {
                format!("{stem} ({}){ext}", n + 1)
            };
            let path = dir.join(&candidate);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    drop(file);
                    reserved = Some(path);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io_err(e)),
            }
        }
        let path = reserved.ok_or(FolderError::NoFreeName)?;
        let cleanup = |p: &Path| {
            let _ = std::fs::remove_file(p);
        };
        // Letzte Pruefung vor dem Schreiben: die frisch angelegte Datei liegt wirklich hier.
        match std::fs::canonicalize(&path) {
            Ok(c) if self.contains(&c) => {}
            _ => {
                cleanup(&path);
                return Err(FolderError::Escape("Verknüpfung nach außen"));
            }
        }
        if let Err(e) = writer(&path) {
            cleanup(&path);
            return Err(FolderError::Io(e));
        }
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let still_inside = std::fs::canonicalize(&path)
            .map(|c| self.contains(&c))
            .unwrap_or(false);
        if !still_inside {
            cleanup(&path);
            return Err(FolderError::Escape("Verknüpfung nach außen"));
        }
        if bytes > MAX_FILE_BYTES {
            cleanup(&path);
            return Err(FolderError::TooLarge);
        }
        Ok(PlacedFile {
            rel: self.rel_of(&path),
            path,
            bytes,
        })
    }

    /// Wie `write_new` mit fertigen Bytes.
    pub fn write_bytes_new(
        &self,
        rel_dir: &str,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<PlacedFile, FolderError> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(FolderError::TooLarge);
        }
        self.write_new(rel_dir, file_name, |path| {
            let mut f = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(path)
                .map_err(|e| e.to_string())?;
            f.write_all(bytes).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())
        })
    }

    /// Ersetzt eine VORHANDENE Datei (z. B. die Notiz im Vault) atomar: erst in eine
    /// Nachbardatei, dann umbenennen. `path` muss von `resolve_file` stammen oder aus
    /// einem Fund unter der Wurzel (wird erneut geprueft).
    pub fn replace_bytes(&self, path: &Path, bytes: &[u8]) -> Result<PlacedFile, FolderError> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(FolderError::TooLarge);
        }
        let meta = std::fs::symlink_metadata(path).map_err(io_err)?;
        if meta.file_type().is_symlink() {
            return Err(FolderError::Escape("Verknüpfung"));
        }
        let canonical = std::fs::canonicalize(path).map_err(io_err)?;
        if !self.contains(&canonical) {
            return Err(FolderError::Escape("Verknüpfung nach außen"));
        }
        let parent = canonical.parent().ok_or(FolderError::BadName)?;
        let tmp = parent.join(format!(
            ".lva-{}-{}.tmp",
            std::process::id(),
            ulid::Ulid::new()
        ));
        let result = (|| -> Result<(), std::io::Error> {
            let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, &canonical)
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(io_err(e));
        }
        Ok(PlacedFile {
            rel: self.rel_of(&canonical),
            path: canonical,
            bytes: bytes.len() as u64,
        })
    }
}

fn check_component(c: &str) -> Result<(), FolderError> {
    if c.chars().count() > MAX_COMPONENT_CHARS {
        return Err(FolderError::Escape("Name zu lang"));
    }
    if c.chars()
        .any(|ch| ch.is_control() || "<>:\"|?*".contains(ch))
    {
        // `:` deckt Laufwerke (`C:x`) und NTFS-Datenstroeme (`datei:strom`) ab.
        return Err(FolderError::Escape("unzulässiges Zeichen"));
    }
    if c.ends_with('.') || c.ends_with(' ') {
        return Err(FolderError::Escape("Punkt oder Leerzeichen am Ende"));
    }
    if is_reserved(c) {
        return Err(FolderError::Escape("reservierter Name"));
    }
    Ok(())
}

fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let upper = stem.to_uppercase();
    RESERVED.contains(&upper.as_str())
}

/// Macht aus einem Titel einen Dateinamen (eine Ebene, keine Trenner, keine
/// Windows-Sonderfaelle). `None`, wenn nichts Brauchbares bleibt. Die Endung bleibt
/// erhalten.
pub fn sanitize_file_name(raw: &str) -> Option<String> {
    let mut s: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    s = s.trim().to_string();
    // Punkte und Leerzeichen am Ende schluckt Windows still.
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    // Fuehrende Punkte (`.hidden`, `..`) vermeiden: kein Verstecken, kein `..`.
    let s = s.trim_start_matches('.').trim().to_string();
    if s.is_empty() {
        return None;
    }
    let mut s: String = s.chars().take(MAX_COMPONENT_CHARS).collect();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s.is_empty() {
        return None;
    }
    if is_reserved(&s) {
        s.insert(0, '_');
    }
    Some(s)
}

/// „Bericht.final.md“ -> („Bericht.final“, „.md“); ohne Endung ("name", "").
fn split_name(name: &str) -> (String, String) {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 8 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests;
