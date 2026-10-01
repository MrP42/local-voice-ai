//! Handle-basierter Dateizugriff der Sandbox (B20, QG5).
//!
//! Grundsatz: Wer einmal geprueft hat, wo etwas liegt, darf danach nicht noch einmal ueber den
//! PFAD darauf zugreifen. Zwischen Pruefung und Zugriff kann ein anderer Prozess mit Schreibrecht
//! im Baum einen Ordner gegen eine Junction tauschen; der zweite Pfadzugriff landete dann
//! ausserhalb der Wurzel (Lesen privater Dateien, Ueberschreiben, Loeschen beim Aufraeumen).
//! Deshalb gilt hier:
//!
//! - **Ordner festhalten** ([`PinnedDir`]): der Ordner wird EINMAL geoeffnet (Verknuepfungen
//!   werden dabei aufgeloest) und sein tatsaechlicher Pfad wird am HANDLE abgelesen
//!   (`GetFinalPathNameByHandleW`). Unter Windows wird er ohne `FILE_SHARE_DELETE` geoeffnet: solange
//!   das Handle lebt, laesst er sich weder umbenennen noch loeschen, also auch nicht durch eine
//!   Junction ersetzen.
//! - **Nur Neues anlegen** ([`create_new_in`]): `create_new`, nie `truncate`, nie ein Oeffnen
//!   einer vorhandenen Datei. Gearbeitet wird danach nur ueber dieses Handle; ob die Datei wirklich
//!   im festgehaltenen Ordner liegt, prueft [`final_path`] des Datei-Handles, BEVOR etwas
//!   geschrieben wird.
//! - **Aufraeumen nur der eigenen Datei** ([`Created::discard`]): Windows loescht ueber das Handle
//!   (Disposition „beim Schliessen loeschen“), Unix nur nach Vergleich von Geraet und Inode mit dem
//!   Handle. Was inzwischen unter dem Pfad liegt, wird nie angefasst.
//!
//! Unix: Der tatsaechliche Pfad eines Handles kennt nur Linux (`/proc/self/fd`); auf macOS bleibt
//! es bei `canonicalize` des Pfads (Restrisiko: dort ist die Pruefung nicht handle-genau).

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// Ein festgehaltener Ordner (siehe Moduldoku).
pub(super) struct PinnedDir {
    _handle: File,
    path: PathBuf,
}

impl PinnedDir {
    /// Oeffnet `dir` und liest dessen tatsaechlichen Pfad am Handle ab. Ob er unter der Wurzel
    /// liegt, prueft der Aufrufer (`Sandbox::contains`).
    pub(super) fn open(dir: &Path) -> io::Result<Self> {
        // Haelt gerade ein anderer Prozess den Ordner mit Loeschrecht offen (Dateimanager beim
        // Umbenennen, Synchronisierung), scheitert das Festhalten kurz: ein paar kurze Versuche.
        let mut attempt = 0;
        let handle = loop {
            match sys::open_dir_pinned(dir) {
                Ok(h) => break h,
                Err(e) if sys::is_sharing_violation(&e) && attempt < 4 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(40));
                }
                Err(e) => return Err(e),
            }
        };
        if !handle.metadata()?.is_dir() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "kein Ordner"));
        }
        let path = sys::final_path(&handle, dir)?;
        Ok(Self {
            _handle: handle,
            path,
        })
    }

    /// Der tatsaechliche (kanonische) Pfad des Ordners.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

/// Eine Datei, die WIR angelegt haben, samt ihrem Handle.
pub(super) struct Created {
    pub(super) file: File,
    pub(super) path: PathBuf,
}

impl Created {
    /// Entfernt genau diese Datei (ueber das Handle), nie das, was inzwischen unter dem Pfad liegt.
    pub(super) fn discard(self) {
        sys::delete_created(&self.file, &self.path);
    }
}

/// Legt `name` NEU im festgehaltenen Ordner an (`create_new`). `Ok(None)`: gibt es schon.
pub(super) fn create_new_in(dir: &PinnedDir, name: &str) -> io::Result<Option<Created>> {
    let path = dir.path.join(name);
    match sys::create_new(&path) {
        Ok(file) => Ok(Some(Created { file, path })),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(e),
    }
}

/// Oeffnet eine vorhandene Datei zum Lesen (folgt Verknuepfungen; wohin das Handle wirklich zeigt,
/// sagt [`final_path`]).
pub(super) fn open_read(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Der tatsaechliche Pfad, auf den `file` zeigt (`hint`: nur fuer Systeme ohne Handle-Abfrage).
pub(super) fn final_path(file: &File, hint: &Path) -> io::Result<PathBuf> {
    sys::final_path(file, hint)
}

#[cfg(windows)]
mod sys {
    use std::ffi::{c_void, OsString};
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};

    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const FILE_SHARE_DELETE: u32 = 0x4;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const DELETE: u32 = 0x0001_0000;
    /// `FILE_INFO_BY_HANDLE_CLASS::FileDispositionInfo`.
    const FILE_DISPOSITION_INFO_CLASS: i32 = 4;

    extern "system" {
        fn GetFinalPathNameByHandleW(
            file: *mut c_void,
            path: *mut u16,
            len: u32,
            flags: u32,
        ) -> u32;
        fn SetFileInformationByHandle(
            file: *mut c_void,
            class: i32,
            info: *const c_void,
            size: u32,
        ) -> i32;
    }

    /// Ordner mit Leserecht und OHNE `FILE_SHARE_DELETE`: er laesst sich nicht umbenennen oder
    /// loeschen, solange das Handle lebt. (Ein Handle ohne jedes Datenrecht, wie es `canonicalize`
    /// benutzt, zaehlt bei der Pruefung der Freigabemodi nicht mit und haelt nichts fest.)
    pub fn open_dir_pinned(dir: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)
    }

    /// Neue Datei, nur wenn es sie nicht gibt. `DELETE` erlaubt das Loeschen ueber das Handle;
    /// `FILE_SHARE_DELETE` laesst ein Umbenennen der noch offenen Zwischendatei zu.
    /// `FILE_FLAG_OPEN_REPARSE_POINT`: ein am Namen vorbereiteter Symlink wird nicht verfolgt (sonst
    /// legte `CREATE_NEW` sein Ziel an), sondern zaehlt als „gibt es schon“.
    pub fn create_new(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    pub fn is_sharing_violation(e: &io::Error) -> bool {
        e.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
    }

    pub fn final_path(file: &File, _hint: &Path) -> io::Result<PathBuf> {
        let handle = file.as_raw_handle();
        let mut buf = vec![0u16; 512];
        loop {
            // SAFETY: gueltiges Handle, Puffer der angegebenen Laenge; Flags 0 =
            // FILE_NAME_NORMALIZED | VOLUME_NAME_DOS (dieselbe Form wie `fs::canonicalize`).
            let n = unsafe {
                GetFinalPathNameByHandleW(handle as *mut c_void, buf.as_mut_ptr(), buf.len() as u32, 0)
            };
            if n == 0 {
                return Err(io::Error::last_os_error());
            }
            if n as usize > buf.len() {
                buf.resize(n as usize, 0);
                continue;
            }
            return Ok(PathBuf::from(OsString::from_wide(&buf[..n as usize])));
        }
    }

    pub fn delete_created(file: &File, path: &Path) {
        let delete: u8 = 1; // FILE_DISPOSITION_INFO { DeleteFile: TRUE }
        // SAFETY: gueltiges Handle mit DELETE-Recht, Zeiger auf eine lokale 1-Byte-Struktur.
        let ok = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle() as *mut c_void,
                FILE_DISPOSITION_INFO_CLASS,
                &delete as *const u8 as *const c_void,
                1,
            )
        };
        if ok == 0 {
            log::warn!(
                "folder: eigene Datei „{}“ liess sich nicht entfernen: {}",
                path.display(),
                io::Error::last_os_error()
            );
        }
    }
}

#[cfg(unix)]
mod sys {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    pub fn open_dir_pinned(dir: &Path) -> io::Result<File> {
        File::open(dir)
    }

    pub fn is_sharing_violation(_e: &io::Error) -> bool {
        false
    }

    pub fn create_new(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
    }

    #[cfg(target_os = "linux")]
    pub fn final_path(file: &File, _hint: &Path) -> io::Result<PathBuf> {
        use std::os::fd::AsRawFd;
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
    }

    #[cfg(not(target_os = "linux"))]
    pub fn final_path(_file: &File, hint: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(hint)
    }

    /// Nur entfernen, wenn unter dem Pfad noch DIESELBE Datei liegt (Geraet und Inode).
    pub fn delete_created(file: &File, path: &Path) {
        let same = match (file.metadata(), std::fs::symlink_metadata(path)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        };
        if same {
            let _ = std::fs::remove_file(path);
        }
    }
}
