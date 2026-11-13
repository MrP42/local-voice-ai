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
//!   Handles (normale Datei, unter Unix zum Schreiben nicht mehrfach verlinkt).
//!   Das schliesst das Zeitfenster zwischen Pruefung und Oeffnen fuer die letzte
//!   Pfadkomponente; ein Abschneiden geschieht erst nach dieser Pruefung (`set_len`),
//!   nie schon beim Oeffnen;
//! - Ordner: [`dir_inside`] laeuft den Pfad unterhalb der Wurzel Komponente fuer
//!   Komponente ab (kein Link, nur Ordner) und verlangt zusaetzlich, dass der
//!   kanonische Ordner unter der kanonischen Wurzel liegt. Die Wurzel selbst darf
//!   ein Link sein (ein verschobener Datenordner), alles darunter nicht.
//!
//! Nicht abgedeckt: ein Link, der zwischen Ordnerpruefung und Oeffnen in einen
//! Zwischenordner gelegt wird (dafuer fehlt mit `std` ein `openat`); wer das
//! kann, kann dieselben Dateien auch direkt ueberschreiben. Der Modul braucht nur
//! `std` (und unter Unix `libc`), damit die Windows-Zweige auch ohne Windows auf
//! Typfehler geprueft werden koennen.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Component, Path};

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

fn refusal() -> io::Error {
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

/// Prueft das geoeffnete Handle selbst (nicht den Pfad): normale Datei, kein
/// Reparse-Punkt, zum Schreiben unter Unix nicht mit einem anderen Namen verlinkt
/// (ein Hardlink wuerde die andere Datei mitschreiben).
fn checked(file: File, write: bool) -> io::Result<File> {
    let meta = file.metadata()?;
    if is_link(&meta) || !meta.is_file() {
        return Err(refusal());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if write && meta.nlink() > 1 {
            return Err(refusal());
        }
    }
    #[cfg(not(unix))]
    let _ = write;
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

/// Benennt `from` ueber `to` um. Beide Enden muessen normale Dateien sein
/// (`to` darf fehlen); ein Link an einem der Enden wird abgelehnt.
pub(crate) fn rename_over(from: &Path, to: &Path) -> io::Result<()> {
    if plain_len(from)?.is_none() {
        return Err(io::Error::from(io::ErrorKind::NotFound));
    }
    plain_len(to)?;
    fs::rename(from, to)
}

/// Liegt `dir` unterhalb von `root`, ohne dass ein Link oder Reparse-Punkt
/// dazwischen liegt, und zeigt der kanonische Pfad nicht aus `root` heraus?
/// Mit `create` werden fehlende Ordner unterhalb der Wurzel angelegt (die Wurzel
/// selbst auch); ohne `create` ist ein fehlender Rest kein Fehler (es gibt dort
/// nichts, dem man folgen koennte). Ein Verstoss ist eine Ablehnung
/// ([`is_refusal`]).
pub(crate) fn dir_inside(root: &Path, dir: &Path, create: bool) -> io::Result<()> {
    let rel = dir.strip_prefix(root).map_err(|_| refusal())?;
    if create {
        fs::create_dir_all(root)?;
    }
    let canonical_root = match fs::canonicalize(root) {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound && !create => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut current = root.to_path_buf();
    for part in rel.components() {
        let Component::Normal(name) = part else {
            return Err(refusal());
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(meta) if is_link(&meta) || !meta.is_dir() => return Err(refusal()),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if !create {
                    return Ok(());
                }
                match fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        let plain = fs::symlink_metadata(&current)
                            .is_ok_and(|m| m.is_dir() && !is_link(&m));
                        if !plain {
                            return Err(refusal());
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
    if fs::canonicalize(&current)?.starts_with(&canonical_root) {
        Ok(())
    } else {
        Err(refusal())
    }
}

#[cfg(test)]
mod tests;
