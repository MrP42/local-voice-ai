//! Dateisystemregeln des Downloaders: Bereitstellen aendert nur eigene, normale
//! Dateien im Modellordner.
//!
//! Der Modellordner kann Verknuepfungen enthalten, die nicht von uns stammen
//! (ein Rest, ein anderes Werkzeug, ein Eintrag von Hand). Ein symbolischer Link
//! oder ein Reparse-Punkt an einer Stelle, an der wir schreiben, umbenennen oder
//! loeschen, wuerde diese Operation auf eine fremde Datei umlenken. Deshalb gilt
//! fuer jede Datei, die der Downloader anfasst (Ziel und `.partial`):
//!
//! - vor jeder Operation `symlink_metadata`: ein Link (unter Windows auch jeder
//!   Reparse-Punkt, also Junctions und Symlinks) oder etwas, das keine normale
//!   Datei ist, wird mit [`Refused`] abgelehnt, ohne zu schreiben oder dem Link
//!   zu folgen;
//! - beim Oeffnen zusaetzlich unter Unix `O_NOFOLLOW`, unter Windows
//!   `FILE_FLAG_OPEN_REPARSE_POINT`, und danach eine Pruefung des geoeffneten
//!   Handles (normale Datei, zum Schreiben nicht mehrfach verlinkt: unter Unix
//!   `nlink`, unter Windows `nNumberOfLinks` aus `GetFileInformationByHandle`).
//!   Das schliesst das Zeitfenster zwischen Pruefung und Oeffnen fuer die letzte
//!   Pfadkomponente; ein Abschneiden geschieht erst nach dieser Pruefung (`set_len`),
//!   nie schon beim Oeffnen.
//!
//! Ordner und Umbenennen gehoeren nicht (mehr) hierher: wer Ordner anlegt oder
//! Dateien umbenennt, tut es ueber ein geoeffnetes Ordner-Handle (`anchor`), nie
//! ueber einen Pfad, den ein Dritter zwischen Pruefung und Gebrauch ersetzen koennte.
//!
//! Nicht abgedeckt: ein Link, der in einem Ordner VOR dem Modellordner liegt oder dort
//! zwischen Pruefung und Oeffnen eingeschoben wird (dafuer fehlt mit `std` ein `openat`).
//! Dieser Weg bleibt fuer die uebrigen Modelle (Piper, Sprachmodelle, ...); der Ladeweg
//! des Nemotron-Modells arbeitet stattdessen ueber ein geoeffnetes Ordner-Handle
//! (`anchor`), das diese Luecke nicht hat.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

/// Reparse-Punkt (Symlink, Junction, Cloud-Platzhalter u. a.).
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
/// Oeffnet den Reparse-Punkt selbst statt seines Ziels.
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// Eine Stelle im Modellordner ist keine eigene normale Datei (Link, Reparse-Punkt,
/// Ordner, Geraet) oder liegt ausserhalb der Wurzel. Der Text nennt keinen Pfad.
#[derive(Debug)]
pub(crate) struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("unsafe_path: not a plain file inside the model folder")
    }
}

impl std::error::Error for Refused {}

pub(super) fn refusal() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, Refused)
}

/// Ist `err` eine Ablehnung dieses Moduls (und kein gewoehnlicher E/A-Fehler)?
pub(crate) fn is_refusal(err: &io::Error) -> bool {
    err.get_ref().is_some_and(|inner| inner.is::<Refused>())
}

/// Symbolischer Link oder (unter Windows) irgendein Reparse-Punkt.
fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Laenge der normalen Datei an `path`; `None`, wenn dort nichts liegt. Ein Link,
/// ein Reparse-Punkt oder etwas anderes als eine normale Datei ist eine Ablehnung
/// ([`is_refusal`]); es wird nie einem Link gefolgt.
pub(crate) fn plain_len(path: &Path) -> io::Result<Option<u64>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if is_link(&meta) || !meta.is_file() => Err(refusal()),
        Ok(meta) => Ok(Some(meta.len())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
}

/// Scheitert das Oeffnen, weil an `path` ein Link oder etwas anderes als eine
/// normale Datei liegt (auch ein Ordner, auch ein Link, der erst waehrend des
/// Oeffnens dorthin kam), ist der Fehler eine Ablehnung; jeder andere Fehler
/// bleibt, wie er ist.
fn opened(result: io::Result<File>, path: &Path) -> io::Result<File> {
    result.map_err(|e| {
        let foreign = fs::symlink_metadata(path).is_ok_and(|m| is_link(&m) || !m.is_file());
        if foreign {
            refusal()
        } else {
            e
        }
    })
}

/// Was ein geoeffnetes Handle ueber seine Datei sagt (nicht der Pfad, den man
/// zum Oeffnen nahm): Kennung (Laufwerk, Index), Zahl der Namen, Art.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileInfo {
    pub(crate) volume: u64,
    pub(crate) index: u64,
    /// Zahl der Verzeichniseintraege der Datei (Hardlinks); 1 bei einer gewoehnlichen.
    pub(crate) links: u64,
    pub(crate) is_file: bool,
    pub(crate) is_dir: bool,
    /// Reparse-Punkt (nur unter Windows; ein Unix-Handle zeigt nie auf einen Link).
    pub(crate) reparse: bool,
}

/// [`FileInfo`] des Handles `file`.
#[cfg(unix)]
pub(crate) fn file_info(file: &File) -> io::Result<FileInfo> {
    use std::os::unix::fs::MetadataExt;
    let meta = file.metadata()?;
    Ok(FileInfo {
        volume: meta.dev(),
        index: meta.ino(),
        links: meta.nlink(),
        is_file: meta.is_file(),
        is_dir: meta.is_dir(),
        reparse: false,
    })
}

/// [`FileInfo`] des Handles `file` (`GetFileInformationByHandle`).
#[cfg(windows)]
pub(crate) fn file_info(file: &File) -> io::Result<FileInfo> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    const DIRECTORY: u32 = 0x0000_0010;
    let mut raw = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` haelt ein gueltiges Handle fuer die Dauer des Aufrufs, `raw` ist
    // ein beschreibbarer Zielpuffer der verlangten Groesse.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut raw) }
        .map_err(io::Error::from)?;
    let dir = raw.dwFileAttributes & DIRECTORY != 0;
    Ok(FileInfo {
        volume: u64::from(raw.dwVolumeSerialNumber),
        index: u64::from(raw.nFileIndexHigh) << 32 | u64::from(raw.nFileIndexLow),
        links: u64::from(raw.nNumberOfLinks),
        is_file: !dir,
        is_dir: dir,
        reparse: raw.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
    })
}

/// Prueft das geoeffnete Handle selbst (nicht den Pfad): normale Datei, kein
/// Reparse-Punkt, zum Schreiben nicht mit einem anderen Namen verlinkt (ein
/// Hardlink wuerde die andere Datei mitschreiben). Das gilt unter Unix und unter
/// Windows, und es gilt vor jeder Aenderung (auch vor dem Abschneiden).
fn checked(file: File, write: bool) -> io::Result<File> {
    let info = file_info(&file)?;
    if info.reparse || !info.is_file || (write && info.links > 1) {
        return Err(refusal());
    }
    Ok(file)
}

/// Oeffnet eine normale Datei zum Lesen, ohne einem Link zu folgen.
pub(crate) fn open_read(path: &Path) -> io::Result<File> {
    checked(opened(options().read(true).open(path), path)?, false)
}

/// Oeffnet eine vorhandene normale Datei zum Anhaengen (Fortsetzung einer
/// Teildatei), ohne einem Link zu folgen.
pub(crate) fn open_append(path: &Path) -> io::Result<File> {
    checked(opened(options().append(true).open(path), path)?, true)
}

/// Legt eine normale Datei neu an oder leert eine vorhandene. Geleert wird erst
/// nach der Pruefung des Handles: eine abgelehnte Datei bleibt unveraendert.
pub(crate) fn open_truncated(path: &Path) -> io::Result<File> {
    let file = checked(
        opened(options().write(true).create(true).open(path), path)?,
        true,
    )?;
    file.set_len(0)?;
    Ok(file)
}

/// Loescht die normale Datei an `path`; fehlt sie, ist das kein Fehler. Ein Link
/// oder etwas anderes als eine normale Datei wird abgelehnt und bleibt liegen.
pub(crate) fn remove(path: &Path) -> io::Result<()> {
    if plain_len(path)?.is_none() {
        return Ok(());
    }
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// [`remove`] ohne Fehlerrueckgabe: fuer Aufraeumen nach einem Fehler.
pub(crate) fn discard(path: &Path) {
    let _ = remove(path);
}

#[cfg(test)]
mod tests;
