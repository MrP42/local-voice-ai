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

/// Der Datentraeger liefert keine brauchbare Datei-Kennung (Windows: `FileIdInfo` nicht
/// lesbar oder nur Nullen/`0xFF`; typisch FAT, je nach Treiber exFAT und manche
/// Netzfreigaben). Ohne Kennung laesst sich weder ein Ordnertausch noch ein Namenstausch
/// belegen; der verankerte Ladeweg lehnt dort ab (fail closed), statt auf etwas
/// Schwaecheres zurueckzufallen. Das ist KEINE Ablehnung wegen eines Links
/// ([`Refused`], `unsafe_path`), sondern eine Eigenschaft des Datentraegers
/// (`volume_unsupported`). Der Text nennt keinen Pfad.
#[derive(Debug)]
pub(crate) struct VolumeUnsupported;

impl std::fmt::Display for VolumeUnsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("volume_unsupported: the volume gives no usable file ID")
    }
}

impl std::error::Error for VolumeUnsupported {}

pub(super) fn volume_unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, VolumeUnsupported)
}

/// Ist `err` die Meldung "Datentraeger ohne Datei-Kennung" ([`VolumeUnsupported`])?
pub(crate) fn is_volume_unsupported(err: &io::Error) -> bool {
    err.get_ref()
        .is_some_and(|inner| inner.is::<VolumeUnsupported>())
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
        // O_NONBLOCK: eine FIFO am Namen laesst das Oeffnen nicht haengen; `checked`
        // lehnt sie am Handle ab. Fuer normale Dateien aendert das Flag nichts.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
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

/// Kennung einer Datei: Datentraeger und 128-Bit-Kennung auf ihm. Unter Unix
/// (Geraet, Inode, auf 128 Bit erweitert), unter Windows (`VolumeSerialNumber`,
/// `FileId` aus `FileIdInfo`). Eine 64-Bit-Kennung (`nFileIndex` aus
/// `GetFileInformationByHandle`) gibt es hier bewusst nicht: ReFS vergibt 128-Bit-IDs,
/// deren untere 64 Bit zusammenfallen koennen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Identity {
    pub(crate) volume: u64,
    pub(crate) id: u128,
}

/// Die Kennung aus den Rohwerten von `FileIdInfo`. Nur Nullen und nur `0xFF` sind
/// keine Kennung (so antworten Dateisysteme, die keine vergeben), dann ist sie
/// unbekannt (`None`). Rein; die Windows-Abfrage reicht ihre Werte hierher.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) fn identity_from_raw(volume: u64, id: [u8; 16]) -> Option<Identity> {
    if id.iter().all(|b| *b == 0) || id.iter().all(|b| *b == 0xFF) {
        return None;
    }
    Some(Identity {
        volume,
        id: u128::from_le_bytes(id),
    })
}

/// Sind das dieselben Dateien? Nur wenn BEIDE Kennungen bekannt und gleich sind.
/// Eine unbekannte Kennung ist nie gleich (auch nicht mit sich selbst): ein
/// Vergleich, der nichts belegen kann, lehnt ab, statt auf eine schwaechere
/// Kennung zurueckzufallen.
pub(crate) fn same_identity(a: Option<Identity>, b: Option<Identity>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if x == y)
}

/// Was ein geoeffnetes Handle ueber seine Datei sagt (nicht der Pfad, den man
/// zum Oeffnen nahm): Kennung, Zahl der Namen, Art.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileInfo {
    /// `None`: unbekannt (Windows: `FileIdInfo` nicht lesbar oder keine Kennung).
    pub(crate) identity: Option<Identity>,
    /// Zahl der Verzeichniseintraege der Datei (Hardlinks); 1 bei einer gewoehnlichen.
    pub(crate) links: u64,
    pub(crate) is_file: bool,
    pub(crate) is_dir: bool,
    /// Reparse-Punkt (nur unter Windows; ein Unix-Handle zeigt nie auf einen Link).
    pub(crate) reparse: bool,
}

impl FileInfo {
    /// Dieselbe Datei wie `other` ([`same_identity`]); unbekannt ist nie gleich.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub(crate) fn same_file_as(&self, other: &FileInfo) -> bool {
        same_identity(self.identity, other.identity)
    }
}

/// Test-Naht: laesst `file_info` auf diesem Thread keine Kennung liefern, so wie ein
/// Datentraeger ohne Datei-Kennung (unter Windows nicht herstellbar, unter Unix liefert
/// jedes Dateisystem eine). Gilt nur fuer den Thread, der den Wachposten haelt; die
/// Tests laufen parallel.
#[cfg(all(unix, test))]
pub(crate) mod inject {
    use std::cell::Cell;

    thread_local! {
        static UNKNOWN: Cell<bool> = const { Cell::new(false) };
    }

    /// Solange er lebt, ist jede Kennung auf diesem Thread unbekannt.
    pub(crate) struct UnknownIdentities;

    impl UnknownIdentities {
        pub(crate) fn on() -> Self {
            UNKNOWN.set(true);
            Self
        }
    }

    impl Drop for UnknownIdentities {
        fn drop(&mut self) {
            UNKNOWN.set(false);
        }
    }

    pub(super) fn unknown() -> bool {
        UNKNOWN.get()
    }
}

/// [`FileInfo`] des Handles `file`.
#[cfg(unix)]
pub(crate) fn file_info(file: &File) -> io::Result<FileInfo> {
    use std::os::unix::fs::MetadataExt;
    let meta = file.metadata()?;
    #[cfg(test)]
    let known = !inject::unknown();
    #[cfg(not(test))]
    let known = true;
    Ok(FileInfo {
        identity: known.then(|| Identity {
            volume: meta.dev(),
            id: u128::from(meta.ino()),
        }),
        links: meta.nlink(),
        is_file: meta.is_file(),
        is_dir: meta.is_dir(),
        reparse: false,
    })
}

/// Win32-Fehlercode in einem `HRESULT` der Form `HRESULT_FROM_WIN32` (`0x8007xxxx`);
/// jeder andere `HRESULT` ist kein Win32-Fehler (`None`). Rein.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn win32_code(hresult: i32) -> Option<u32> {
    let bits = hresult as u32;
    (bits & 0xFFFF_0000 == 0x8007_0000).then_some(bits & 0xFFFF)
}

/// Heisst dieser Fehler der Abfrage `FileIdInfo` "das kann dieser Datentraeger (dieser
/// Treiber) nicht"? Nur `ERROR_INVALID_FUNCTION` (1), `ERROR_NOT_SUPPORTED` (50) und
/// `ERROR_INVALID_PARAMETER` (87) sagen das: so antworten Dateisysteme ohne 128-Bit-Kennung
/// (FAT, je nach Treiber exFAT und Netzfreigaben). Alles andere (Zugriff verweigert,
/// ungueltiges Handle, E/A-Fehler, zu wenig Speicher, ...) ist KEIN Befund ueber den
/// Datentraeger und wird als gewoehnlicher Fehler weitergereicht; eine Platte mit
/// Kennung wuerde sonst bei einem voruebergehenden Fehler als "nicht unterstuetzt"
/// gemeldet. Rein; `hresult` ist der Fehler aus dem `windows`-Crate (`HRESULT`).
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn id_info_unsupported(hresult: i32) -> bool {
    const ERROR_INVALID_FUNCTION: u32 = 1;
    const ERROR_NOT_SUPPORTED: u32 = 50;
    const ERROR_INVALID_PARAMETER: u32 = 87;
    matches!(
        win32_code(hresult),
        Some(ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED | ERROR_INVALID_PARAMETER)
    )
}

/// Der Fehler der Abfrage als `io::Error` mit richtigem Win32-Code (damit `kind()`
/// stimmt); ein `HRESULT`, der kein Win32-Fehler ist, bleibt, wie er ist.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn id_info_error(hresult: i32) -> io::Error {
    io::Error::from_raw_os_error(win32_code(hresult).map_or(hresult, |code| code as i32))
}

/// [`FileInfo`] des Handles `file`: Attribute und Zahl der Namen aus
/// `GetFileInformationByHandle`, die Kennung aus `GetFileInformationByHandleEx`
/// (`FileIdInfo`, 128 Bit). Antwortet die zweite Abfrage mit einem der Fehler, die "nicht
/// unterstuetzt" heissen ([`id_info_unsupported`]: Datentraeger ohne Datei-Kennung,
/// typisch FAT), ist die Kennung unbekannt (`None`). Das ist kein stilles Weitermachen:
/// jeder Vergleich lehnt eine unbekannte Kennung ab ([`same_identity`]), und der
/// verankerte Ladeweg meldet sie als [`VolumeUnsupported`] (`volume_unsupported`),
/// nicht als Link-Ablehnung. JEDER andere Fehler der Abfrage wird als `io::Error`
/// weitergereicht, nicht als Befund ueber den Datentraeger gedeutet.
#[cfg(windows)]
pub(crate) fn file_info(file: &File) -> io::Result<FileInfo> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO,
    };
    const DIRECTORY: u32 = 0x0000_0010;
    let handle = HANDLE(file.as_raw_handle());
    let mut raw = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` haelt ein gueltiges Handle fuer die Dauer des Aufrufs, `raw` ist
    // ein beschreibbarer Zielpuffer der verlangten Groesse.
    unsafe { GetFileInformationByHandle(handle, &mut raw) }.map_err(io::Error::from)?;
    let mut id = FILE_ID_INFO::default();
    // SAFETY: gleiches Handle; `id` ist ein beschreibbarer Puffer genau in der Groesse
    // von `FILE_ID_INFO`, die Klasse `FileIdInfo` fuellt ihn.
    let id_known = match unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            std::ptr::addr_of_mut!(id).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    } {
        Ok(()) => true,
        Err(e) if id_info_unsupported(e.code().0) => false,
        Err(e) => return Err(id_info_error(e.code().0)),
    };
    let dir = raw.dwFileAttributes & DIRECTORY != 0;
    Ok(FileInfo {
        identity: if id_known {
            identity_from_raw(id.VolumeSerialNumber, id.FileId.Identifier)
        } else {
            None
        },
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
