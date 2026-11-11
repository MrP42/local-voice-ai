//! Geprüfte Ordner-Handles (lva#507, Befunde 2 bis 4): Wurzel und Zwischenverzeichnisse werden einmal geöffnet,
//! am Handle beurteilt und dann GEHALTEN; jede Ebene darunter, jedes Auflisten und jedes Öffnen einer Datei
//! läuft über dieses Handle, nie über einen Pfad, der inzwischen ein anderes Ziel haben könnte.
//!
//! * Unix: echte relative Öffnung (`openat` mit `O_NOFOLLOW | O_DIRECTORY`, `fstatat` mit
//!   `AT_SYMLINK_NOFOLLOW`, `fdopendir` auf einem frischen `openat(fd, ".")`). Eine Junction oder ein Symlink,
//!   der ein Zwischenglied ersetzt, ist für den gehaltenen Ordner ohne Wirkung.
//! * Windows: Handle-Kette. Jedes Glied wird mit `FILE_READ_ATTRIBUTES`, `FILE_FLAG_OPEN_REPARSE_POINT` und
//!   `FILE_FLAG_OPEN_NO_RECALL` geöffnet, OHNE `FILE_SHARE_DELETE`: solange wir das Handle halten, kann kein
//!   anderer Prozess den Ordner umbenennen, löschen oder durch eine Junction ersetzen (alle Ahnen sind
//!   ebenfalls gehalten). Jedes Glied wird am Handle beurteilt (Reparse-Bit, Cloud-Bits, Verzeichnis) und sein
//!   endgültiger Pfad (`GetFinalPathNameByHandleW`) muss der des gehaltenen Elternhandles plus Name sein; ein
//!   Pfadglied, das vor dem Halten ausgetauscht wurde, fällt dadurch auf. Statt `NtCreateFile` mit
//!   `RootDirectory` (breite, undokumentierte Oberfläche) bleibt es bei dokumentierten Win32-Aufrufen; das
//!   Ergebnis ist dasselbe, weil die Eltern gehalten und geprüft sind. Ordner werden gleich mit Listenrecht
//!   geöffnet (`ReOpenFile` auf Ordner-Handles scheitert mit ERROR_ACCESS_DENIED) und über dasselbe Handle
//!   aufgelistet (`FileIdBothDirectoryInfo`), nachdem es beurteilt wurde; keine Find-Abfrage per Pfad.
//!
//! Aufzählen ist iterativ: der Aufrufer bekommt jeden Eintrag einzeln und bricht mit `ControlFlow::Break` ab
//! (Budget); es gibt keine vollständige Liste im Speicher.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};

#[cfg(windows)]
use super::ATTR_VERZEICHNIS;
use super::{
    abruf_sperren, oeffnen_io, offen_daten, zweistufig, Eintrag, OeffnenArt, OeffnenFehler, Urteil,
    UrteilFn,
};

/// Ein geprüftes, gehaltenes Verzeichnis (siehe Moduldoku).
pub struct Ordner {
    pfad: PathBuf,
    urteil: UrteilFn,
    sys: sys::Dir,
}

/// Ein einzelner, gewöhnlicher Name (nicht leer, kein `.`/`..`, kein Trenner, Laufwerk oder Präfix; unter
/// Windows außerdem kein Datenstrom `:` und kein abschließender Punkt oder Leerzeichen).
fn schlicht(name: &OsStr) -> bool {
    let mut c = Path::new(name).components();
    if !matches!((c.next(), c.next()), (Some(Component::Normal(n)), None) if n == name) {
        return false;
    }
    #[cfg(windows)]
    {
        let t = name.to_string_lossy();
        if t.contains(':') || t.ends_with('.') || t.ends_with(' ') {
            return false;
        }
    }
    true
}

fn umweg() -> OeffnenFehler {
    OeffnenFehler::Urteil(Urteil::Umweg)
}

/// Urteil über das gehaltene Ordner-Handle (nicht über den Pfad). Attribute nicht bestimmbar: Umweg.
fn dir_urteil(f: &fs::File, p: &Path, urteil: UrteilFn) -> Result<(), OeffnenFehler> {
    let Some((attribute, tag)) = offen_daten(f, p) else {
        return Err(umweg());
    };
    #[cfg(windows)]
    if attribute & ATTR_VERZEICHNIS == 0 {
        return Err(umweg());
    }
    #[cfg(test)]
    if let Some((a, t)) = super::naht::find_daten(p) {
        match urteil(t, a) {
            Urteil::Lesbar => {}
            u => return Err(OeffnenFehler::Urteil(u)),
        }
    }
    match urteil(tag, attribute) {
        Urteil::Lesbar => Ok(()),
        u => Err(OeffnenFehler::Urteil(u)),
    }
}

fn sperre_pruefen() -> Result<super::AbrufSperre, OeffnenFehler> {
    let sperre = abruf_sperren();
    if sperre.fehlt() {
        return Err(OeffnenFehler::Io(io::Error::other(
            "Abrufsperre nicht setzbar",
        )));
    }
    Ok(sperre)
}

impl Ordner {
    /// Öffnet die Wurzel `p` (kanonisch, vom Aufrufer vorgeprüft) und urteilt am Handle. Nicht „lesbar“ oder
    /// nicht bestimmbar: Fehler, aufgelistet wird dann nie (fail-closed). Laufwerkswurzeln gehen denselben Weg:
    /// auch sie bekommen ein positives Urteil vom Handle.
    pub fn wurzel(p: &Path, urteil: UrteilFn) -> Result<Ordner, OeffnenFehler> {
        let _sperre = sperre_pruefen()?;
        let sys = sys::wurzel(p)?;
        dir_urteil(sys.datei(), p, urteil)?;
        Ok(Ordner {
            pfad: p.to_path_buf(),
            urteil,
            sys,
        })
    }

    /// Öffnet den Unterordner `name` relativ zu diesem Ordner und urteilt am neuen Handle.
    pub fn unter(&self, name: &OsStr) -> Result<Ordner, OeffnenFehler> {
        if !schlicht(name) {
            return Err(umweg());
        }
        let _sperre = sperre_pruefen()?;
        let pfad = self.pfad.join(name);
        let sys = sys::unter(&self.sys, &pfad, name)?;
        dir_urteil(sys.datei(), &pfad, self.urteil)?;
        Ok(Ordner {
            pfad,
            urteil: self.urteil,
            sys,
        })
    }

    pub fn pfad(&self) -> &Path {
        &self.pfad
    }

    /// Zählt die Einträge einzeln auf (ohne `.` und `..`). `f` bricht mit `Break` ab; dann ist das Ergebnis
    /// `Ok(false)` (unvollständig). `Ok(true)`: bis zum Ende gelesen. Ein Fehler mitten im Auflisten ist ein
    /// Fehler des ganzen Ordners (`Io`, bei einem Abruf-Verweigern `Urteil(NurOnline)`). Vor dem Auflisten wird
    /// das Handle erneut beurteilt.
    pub fn auflisten(
        &self,
        mut f: impl FnMut(Eintrag) -> ControlFlow<()>,
    ) -> Result<bool, OeffnenFehler> {
        let _sperre = sperre_pruefen()?;
        dir_urteil(self.sys.datei(), &self.pfad, self.urteil)?;
        sys::auflisten(&self.sys, &self.pfad, &mut |mut e: Eintrag| {
            #[cfg(test)]
            if let Some(r) = e.roh.as_mut() {
                super::naht::anwenden(&self.pfad.join(&e.name), r);
            }
            #[cfg(not(test))]
            let _ = &mut e;
            f(e)
        })
    }

    /// Zweistufiges Öffnen der Datei `name` in diesem Ordner zum Lesen ([`super::oeffnen_ohne_abruf`]), aber
    /// relativ zum gehaltenen Ordner und ohne zweiten Pfadzugriff beim Hochstufen. `urteil` beurteilt die
    /// Datei (der Ordner behält sein eigenes).
    pub fn datei_oeffnen(
        &self,
        name: &OsStr,
        art: OeffnenArt,
        urteil: UrteilFn,
    ) -> Result<fs::File, OeffnenFehler> {
        if !schlicht(name) {
            return Err(umweg());
        }
        let _sperre = sperre_pruefen()?;
        let pfad = self.pfad.join(name);
        let eins = sys::datei_stufe1(&self.sys, &pfad, name, art)?;
        zweistufig(eins, &pfad, urteil, &|e| {
            sys::hochstufen_in(&self.sys, e, &pfad, name, art)
        })
    }
}

#[cfg(unix)]
mod sys {
    use super::{oeffnen_io, Eintrag, OeffnenArt, OeffnenFehler, Urteil};
    use crate::ablage::eintrag::{Gattung, Roh};
    use std::ffi::{CStr, CString, OsStr, OsString};
    use std::fs;
    use std::io;
    use std::ops::ControlFlow;
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub struct Dir {
        f: fs::File,
    }

    impl Dir {
        pub fn datei(&self) -> &fs::File {
            &self.f
        }
    }

    fn cstr(b: &[u8]) -> Result<CString, OeffnenFehler> {
        CString::new(b).map_err(|_| OeffnenFehler::Io(io::Error::from(io::ErrorKind::InvalidInput)))
    }

    fn open_at(dirfd: RawFd, name: &CStr, flags: libc::c_int) -> io::Result<fs::File> {
        loop {
            // SAFETY: `name` ist NUL-terminiert und lebt über den Aufruf; `dirfd` ist ein offenes Handle
            // (oder `AT_FDCWD`).
            let fd = unsafe { libc::openat(dirfd, name.as_ptr(), flags) };
            if fd >= 0 {
                // SAFETY: frischer, gültiger Deskriptor, den sonst niemand besitzt.
                return Ok(unsafe { fs::File::from_raw_fd(fd) });
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
    }

    const DIR_FLAGS: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;

    /// Rohdaten aus `fstatat` (ohne Symlink zu folgen, ohne zu öffnen).
    fn stat_at(dirfd: RawFd, name: &CStr) -> io::Result<Roh> {
        // SAFETY: `libc::stat` ist ein reiner Zahlenpuffer; Nullen sind gültig.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: `name` NUL-terminiert, `st` beschreibbar, `dirfd` offen.
        let r = unsafe { libc::fstatat(dirfd, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
        let art = st.st_mode & libc::S_IFMT;
        let gattung = if art == libc::S_IFLNK {
            Gattung::Link
        } else if art == libc::S_IFDIR {
            Gattung::Ordner
        } else if art == libc::S_IFREG {
            Gattung::Datei
        } else {
            Gattung::Anderes
        };
        #[cfg(target_os = "macos")]
        let attribute = crate::ablage::eintrag::attribute_aus_stat_flags(st.st_flags);
        #[cfg(not(target_os = "macos"))]
        let attribute = 0;
        Ok(Roh {
            gattung,
            attribute,
            tag: 0,
            groesse: st.st_size as u64,
            zeit: i64::from(st.st_mtime) * 1000 + i64::from(st.st_mtime_nsec) / 1_000_000,
            nlink: st.st_nlink as u64,
        })
    }

    pub fn wurzel(p: &Path) -> Result<Dir, OeffnenFehler> {
        let c = cstr(p.as_os_str().as_bytes())?;
        let f = open_at(libc::AT_FDCWD, &c, DIR_FLAGS | libc::O_NOFOLLOW).map_err(oeffnen_io)?;
        Ok(Dir { f })
    }

    pub fn unter(d: &Dir, pfad: &Path, name: &OsStr) -> Result<Dir, OeffnenFehler> {
        let c = cstr(name.as_bytes())?;
        // Vorab, ohne zu öffnen: Art und (macOS) Platzhalter-Flag des Eintrags im gehaltenen Ordner.
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut roh = stat_at(d.f.as_raw_fd(), &c).map_err(oeffnen_io)?;
        #[cfg(test)]
        crate::ablage::eintrag::naht::anwenden(pfad, &mut roh);
        #[cfg(not(test))]
        let _ = pfad;
        match roh.urteil() {
            Urteil::Lesbar => {}
            u => return Err(OeffnenFehler::Urteil(u)),
        }
        if !roh.ist_ordner() {
            return Err(OeffnenFehler::Urteil(Urteil::Umweg));
        }
        let f = open_at(d.f.as_raw_fd(), &c, DIR_FLAGS | libc::O_NOFOLLOW).map_err(oeffnen_io)?;
        Ok(Dir { f })
    }

    /// `errno` zurücksetzen, damit das Ende von `readdir` vom Fehler zu unterscheiden ist. `false`: auf dieser
    /// Plattform nicht möglich (dann gilt jedes `NULL` als Ende).
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn errno_null() -> bool {
        // SAFETY: schreibt nur den Thread-lokalen `errno`.
        unsafe { *libc::__errno_location() = 0 };
        true
    }

    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    fn errno_null() -> bool {
        // SAFETY: schreibt nur den Thread-lokalen `errno`.
        unsafe { *libc::__error() = 0 };
        true
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd"
    )))]
    fn errno_null() -> bool {
        false
    }

    struct Dirp(*mut libc::DIR);

    impl Drop for Dirp {
        fn drop(&mut self) {
            // SAFETY: gültiger `DIR*` aus `fdopendir`, wird genau einmal geschlossen (schließt den Deskriptor mit).
            unsafe { libc::closedir(self.0) };
        }
    }

    pub fn auflisten(
        d: &Dir,
        _pfad: &Path,
        f: &mut dyn FnMut(Eintrag) -> ControlFlow<()>,
    ) -> Result<bool, OeffnenFehler> {
        // Frisches Handle desselben Ordners (eigene Leseposition): das „Hochstufen“ zum Auflisten.
        let punkt = cstr(b".")?;
        let lf = open_at(d.f.as_raw_fd(), &punkt, DIR_FLAGS).map_err(oeffnen_io)?;
        let fd = lf.into_raw_fd();
        // SAFETY: `fd` ist ein offener Ordner-Deskriptor; bei Erfolg gehört er dem `DIR*`.
        let dirp = unsafe { libc::fdopendir(fd) };
        if dirp.is_null() {
            let e = io::Error::last_os_error();
            // SAFETY: `fd` gehört noch uns (fdopendir ist fehlgeschlagen).
            unsafe { libc::close(fd) };
            return Err(oeffnen_io(e));
        }
        let dirp = Dirp(dirp);
        // SAFETY: gültiger `DIR*`.
        let dfd = unsafe { libc::dirfd(dirp.0) };
        let kann_errno = errno_null();
        loop {
            if kann_errno {
                errno_null();
            }
            // SAFETY: gültiger `DIR*`; der Rückgabewert gilt bis zum nächsten `readdir` (wird sofort kopiert).
            let ent = unsafe { libc::readdir(dirp.0) };
            if ent.is_null() {
                let e = io::Error::last_os_error();
                if kann_errno && e.raw_os_error().is_some_and(|n| n != 0) {
                    return Err(oeffnen_io(e));
                }
                return Ok(true);
            }
            // SAFETY: `d_name` ist ein NUL-terminierter Name im Eintrag.
            let name_c = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) };
            let nb = name_c.to_bytes();
            if nb == b"." || nb == b".." {
                continue;
            }
            let name = OsString::from(OsStr::from_bytes(nb));
            let roh = stat_at(dfd, name_c).ok();
            if let ControlFlow::Break(()) = f(Eintrag { name, roh }) {
                return Ok(false);
            }
        }
    }

    fn datei_flags(art: OeffnenArt, stufe1: bool) -> libc::c_int {
        let _ = stufe1;
        let mut fl = libc::O_RDONLY | libc::O_CLOEXEC;
        if !art.folgen {
            fl |= libc::O_NOFOLLOW | libc::O_NONBLOCK;
        }
        #[cfg(target_os = "macos")]
        if stufe1 {
            fl |= libc::O_EVTONLY;
        }
        fl
    }

    pub fn datei_stufe1(
        d: &Dir,
        _pfad: &Path,
        name: &OsStr,
        art: OeffnenArt,
    ) -> Result<fs::File, OeffnenFehler> {
        let c = cstr(name.as_bytes())?;
        open_at(d.f.as_raw_fd(), &c, datei_flags(art, true)).map_err(oeffnen_io)
    }

    /// Hochstufen im geprüften Ordner: wo Stufe 1 schon lesbar ist, ein Duplikat desselben Handles; macOS
    /// (Stufe 1 nur `O_EVTONLY`) öffnet den Namen noch einmal RELATIV zum gehaltenen Ordner.
    pub fn hochstufen_in(
        d: &Dir,
        eins: &fs::File,
        _pfad: &Path,
        name: &OsStr,
        art: OeffnenArt,
    ) -> io::Result<fs::File> {
        #[cfg(target_os = "macos")]
        {
            let _ = eins;
            let c = CString::new(name.as_bytes())
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            open_at(d.f.as_raw_fd(), &c, datei_flags(art, false))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (d, name, art);
            eins.try_clone()
        }
    }
}

#[cfg(windows)]
mod sys {
    use super::{oeffnen_io, umweg, Eintrag, OeffnenArt, OeffnenFehler};
    use crate::ablage::eintrag::system::{FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT};
    use crate::ablage::eintrag::{Gattung, Roh, ATTR_REPARSE_POINT, ATTR_VERZEICHNIS};
    use std::ffi::{c_void, OsStr, OsString};
    use std::fs;
    use std::io;
    use std::ops::ControlFlow;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows::Win32::Foundation::{ERROR_NO_MORE_FILES, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandleEx,
        GetFinalPathNameByHandleW, FILE_ID_BOTH_DIR_INFO, FILE_NAME_NORMALIZED,
    };

    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_LIST_DIRECTORY: u32 = 0x1;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    /// Lesen und Schreiben freigeben, NICHT löschen/umbenennen: ein gehaltener Ordner bleibt, wo er ist.
    const TEILEN_OHNE_LOESCHEN: u32 = 0x1 | 0x2;
    const TEILEN_ALLES: u32 = 0x1 | 0x2 | 0x4;
    /// 100-ns-Schritte zwischen 1601-01-01 und 1970-01-01.
    const FILETIME_BIS_1970: u64 = 116_444_736_000_000_000;

    pub struct Dir {
        f: fs::File,
        /// Endgültiger Pfad des gehaltenen Handles, normalisiert ([`norm`]).
        endgueltig: String,
    }

    impl Dir {
        pub fn datei(&self) -> &fs::File {
            &self.f
        }
    }

    /// Ohne `\\?\`-Präfix (`\\?\UNC\` wird `\\`), klein, ohne abschließenden `\`.
    fn norm(s: &str) -> String {
        let t = if let Some(r) = s.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{r}")
        } else if let Some(r) = s.strip_prefix(r"\\?\") {
            r.to_string()
        } else {
            s.to_string()
        };
        t.to_lowercase().trim_end_matches('\\').to_string()
    }

    fn endgueltig_von(f: &fs::File) -> io::Result<String> {
        let mut puffer = vec![0u16; 1024];
        loop {
            // SAFETY: gültiges, offenes Handle; `puffer` ist ein beschreibbarer Puffer der angegebenen Länge.
            let n = unsafe {
                GetFinalPathNameByHandleW(
                    HANDLE(f.as_raw_handle()),
                    &mut puffer,
                    // 0 = `VOLUME_NAME_DOS | FILE_NAME_NORMALIZED`
                    FILE_NAME_NORMALIZED,
                )
            } as usize;
            if n == 0 {
                return Err(io::Error::last_os_error());
            }
            if n >= puffer.len() {
                puffer.resize(n + 1, 0);
                continue;
            }
            return Ok(norm(&String::from_utf16_lossy(&puffer[..n])));
        }
    }

    /// Ordner gleich mit Listenrecht öffnen: `ReOpenFile` auf einem Ordner-Handle scheitert immer mit
    /// ERROR_ACCESS_DENIED (auf Windows gemessen). Listenrecht liest keine Dateidaten, hydriert also nichts;
    /// das Urteil am Handle fällt vor dem ersten Auflisten.
    fn dir_oeffnen(p: &Path) -> io::Result<fs::File> {
        fs::OpenOptions::new()
            .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
            .share_mode(TEILEN_OHNE_LOESCHEN)
            .custom_flags(
                FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_REPARSE_POINT
                    | FILE_FLAG_OPEN_NO_RECALL,
            )
            .open(p)
    }

    pub fn wurzel(p: &Path) -> Result<Dir, OeffnenFehler> {
        let f = dir_oeffnen(p).map_err(oeffnen_io)?;
        let endgueltig = endgueltig_von(&f).map_err(OeffnenFehler::Io)?;
        if endgueltig != norm(&p.to_string_lossy()) {
            // Ein Glied des Pfades führt woandershin (Junction, Symlink, Umbenennung vor dem Halten).
            return Err(umweg());
        }
        Ok(Dir { f, endgueltig })
    }

    pub fn unter(d: &Dir, pfad: &Path, name: &OsStr) -> Result<Dir, OeffnenFehler> {
        let f = dir_oeffnen(pfad).map_err(oeffnen_io)?;
        let endgueltig = endgueltig_von(&f).map_err(OeffnenFehler::Io)?;
        if endgueltig
            != format!(
                r"{}\{}",
                d.endgueltig,
                name.to_string_lossy().to_lowercase()
            )
        {
            return Err(umweg());
        }
        Ok(Dir { f, endgueltig })
    }

    pub fn auflisten(
        d: &Dir,
        _pfad: &Path,
        f: &mut dyn FnMut(Eintrag) -> ControlFlow<()>,
    ) -> Result<bool, OeffnenFehler> {
        // Das gehaltene Handle trägt schon Listenrecht ([`dir_oeffnen`]); der Aufrufer hat es vorher beurteilt.
        // Die Aufzählung beginnt mit `FileIdBothDirectoryRestartInfo` immer von vorn.
        let liste = &d.f;
        // 8-Byte-ausgerichteter Puffer (64 KiB).
        let mut puffer = vec![0u64; 8192];
        let bytes = puffer.len() * 8;
        let mut klasse = FileIdBothDirectoryRestartInfo;
        loop {
            // SAFETY: gültiges Handle; `puffer` ist ein beschreibbarer, ausgerichteter Puffer der Größe `bytes`.
            let r = unsafe {
                GetFileInformationByHandleEx(
                    HANDLE(liste.as_raw_handle()),
                    klasse,
                    puffer.as_mut_ptr().cast::<c_void>(),
                    bytes as u32,
                )
            };
            klasse = FileIdBothDirectoryInfo;
            if let Err(e) = r {
                if e.code() == ERROR_NO_MORE_FILES.to_hresult() {
                    return Ok(true);
                }
                return Err(OeffnenFehler::Io(crate::ablage::eintrag::system::win_io(e)));
            }
            let basis = puffer.as_ptr().cast::<u8>();
            let name_ab = std::mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            let mut ab = 0usize;
            loop {
                if ab + name_ab > bytes {
                    return Err(OeffnenFehler::Io(io::Error::other("Verzeichnispuffer")));
                }
                // SAFETY: `ab` liegt im Puffer (geprüft), Einträge sind 8-Byte-ausgerichtet; Lesen als Kopie.
                let kopf = unsafe {
                    std::ptr::read_unaligned(basis.add(ab).cast::<FILE_ID_BOTH_DIR_INFO>())
                };
                let n = kopf.FileNameLength as usize / 2;
                if ab + name_ab + n * 2 > bytes {
                    return Err(OeffnenFehler::Io(io::Error::other("Verzeichnispuffer")));
                }
                // SAFETY: Name liegt im Puffer (geprüft), `name_ab` und `ab` sind gerade (u16-ausgerichtet).
                let breit =
                    unsafe { std::slice::from_raw_parts(basis.add(ab + name_ab).cast::<u16>(), n) };
                let name = OsString::from_wide(breit);
                if name != "." && name != ".." {
                    let attribute = kopf.FileAttributes;
                    let schreib = kopf.LastWriteTime as u64;
                    let roh = Roh {
                        gattung: if attribute & ATTR_VERZEICHNIS != 0 {
                            Gattung::Ordner
                        } else {
                            Gattung::Datei
                        },
                        attribute,
                        // Bei gesetztem `REPARSE_POINT` trägt `EaSize` das Reparse-Tag (wie `dwReserved0`
                        // bei `FindFirstFileExW`, dasselbe Systemverhalten).
                        tag: if attribute & ATTR_REPARSE_POINT != 0 {
                            kopf.EaSize
                        } else {
                            0
                        },
                        groesse: kopf.EndOfFile.max(0) as u64,
                        zeit: schreib
                            .checked_sub(FILETIME_BIS_1970)
                            .map_or(0, |x| (x / 10_000) as i64),
                        nlink: 1,
                    };
                    if let ControlFlow::Break(()) = f(Eintrag {
                        name,
                        roh: Some(roh),
                    }) {
                        return Ok(false);
                    }
                }
                if kopf.NextEntryOffset == 0 {
                    break;
                }
                ab += kopf.NextEntryOffset as usize;
            }
        }
    }

    pub fn datei_stufe1(
        d: &Dir,
        pfad: &Path,
        name: &OsStr,
        _art: OeffnenArt,
    ) -> Result<fs::File, OeffnenFehler> {
        let f = fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(TEILEN_ALLES)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL)
            .open(pfad)
            .map_err(oeffnen_io)?;
        let endgueltig = endgueltig_von(&f).map_err(OeffnenFehler::Io)?;
        if endgueltig
            != format!(
                r"{}\{}",
                d.endgueltig,
                name.to_string_lossy().to_lowercase()
            )
        {
            return Err(umweg());
        }
        Ok(f)
    }

    pub fn hochstufen_in(
        _d: &Dir,
        eins: &fs::File,
        pfad: &Path,
        _name: &OsStr,
        art: OeffnenArt,
    ) -> io::Result<fs::File> {
        crate::ablage::eintrag::system::hochstufen(eins, pfad, art)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::ablage::eintrag::{self, naht, reparse_urteil, ATTR_OFFLINE};
    use std::ffi::OsString;
    use std::os::unix::fs::symlink;

    struct Sauber;
    impl Drop for Sauber {
        fn drop(&mut self) {
            naht::leeren();
        }
    }
    fn sauber() -> Sauber {
        naht::leeren();
        Sauber
    }

    fn art() -> OeffnenArt {
        OeffnenArt {
            folgen: false,
            teilen: 7,
        }
    }

    fn namen(o: &Ordner) -> Vec<String> {
        let mut v = Vec::new();
        o.auflisten(|e| {
            v.push(e.name.to_string_lossy().into_owned());
            ControlFlow::Continue(())
        })
        .unwrap();
        v.sort();
        v
    }

    fn wurzel() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let w = fs::canonicalize(d.path()).unwrap();
        (d, w)
    }

    #[test]
    fn ein_ersetztes_zwischenverzeichnis_aendert_nichts_am_gehaltenen_ordner() {
        let _n = sauber();
        let (_d, w) = wurzel();
        fs::create_dir_all(w.join("unter")).unwrap();
        fs::write(w.join("unter/innen.txt"), b"innen").unwrap();
        fs::create_dir_all(w.join("extern")).unwrap();
        fs::write(w.join("extern/aussen.txt"), b"aussen").unwrap();
        let root = Ordner::wurzel(&w, reparse_urteil).unwrap();
        let unter = root.unter(OsStr::new("unter")).unwrap();
        // Nach der Prüfung: `unter` wird durch eine Verknüpfung auf `extern` ersetzt.
        fs::rename(w.join("unter"), w.join("unter_alt")).unwrap();
        symlink(w.join("extern"), w.join("unter")).unwrap();
        // Das gehaltene Handle zeigt weiter auf das geprüfte Objekt.
        assert_eq!(namen(&unter), ["innen.txt"]);
        let mut f = unter
            .datei_oeffnen(OsStr::new("innen.txt"), art(), reparse_urteil)
            .unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut f, &mut s).unwrap();
        assert_eq!(s, "innen");
        assert!(unter
            .datei_oeffnen(OsStr::new("aussen.txt"), art(), reparse_urteil)
            .is_err());
        // Ein neuer Weg über die Wurzel betritt die Verknüpfung nicht.
        assert!(matches!(
            root.unter(OsStr::new("unter")),
            Err(OeffnenFehler::Io(_)) | Err(OeffnenFehler::Urteil(Urteil::Umweg))
        ));
    }

    #[test]
    fn verknuepfte_unterordner_und_unsaubere_namen_werden_abgewiesen() {
        let _n = sauber();
        let (_d, w) = wurzel();
        fs::create_dir_all(w.join("echt")).unwrap();
        symlink(w.join("echt"), w.join("link")).unwrap();
        let root = Ordner::wurzel(&w, reparse_urteil).unwrap();
        assert!(root.unter(OsStr::new("link")).is_err());
        for schlecht in ["..", ".", "a/b", "", "/etc"] {
            assert!(
                root.unter(OsStr::new(schlecht)).is_err(),
                "{schlecht:?} muss abgewiesen werden"
            );
        }
        // Verknüpfte Wurzel: nicht öffnen.
        assert!(Ordner::wurzel(&w.join("link"), reparse_urteil).is_err());
    }

    #[test]
    fn ohne_positives_urteil_am_handle_wird_nie_aufgelistet() {
        let _n = sauber();
        let (_d, w) = wurzel();
        fs::write(w.join("a.txt"), b"a").unwrap();
        // Attribute am Handle nicht bestimmbar: fail-closed.
        naht::handle_ausfall_setzen(&w);
        assert!(matches!(
            Ordner::wurzel(&w, reparse_urteil),
            Err(OeffnenFehler::Urteil(Urteil::Umweg))
        ));
        naht::leeren();
        // Nur online am Handle.
        naht::handle_setzen(&w, ATTR_OFFLINE, 0);
        assert!(matches!(
            Ordner::wurzel(&w, reparse_urteil),
            Err(OeffnenFehler::Urteil(Urteil::NurOnline))
        ));
        naht::leeren();
        // Wird der Ordner nach dem Halten ausgelagert, listet auch das Auflisten nicht mehr.
        let o = Ordner::wurzel(&w, reparse_urteil).unwrap();
        naht::handle_setzen(&w, ATTR_OFFLINE, 0);
        let mut gesehen = 0;
        let r = o.auflisten(|_| {
            gesehen += 1;
            ControlFlow::Continue(())
        });
        assert!(matches!(r, Err(OeffnenFehler::Urteil(Urteil::NurOnline))));
        assert_eq!(
            gesehen, 0,
            "kein Eintrag eines nur online liegenden Ordners"
        );
    }

    #[test]
    fn das_aufzaehlen_bricht_im_budget_ab_und_laedt_nichts_vorab() {
        let _n = sauber();
        let (_d, w) = wurzel();
        for i in 0..50 {
            fs::write(w.join(format!("d{i:02}.txt")), b"x").unwrap();
        }
        let o = Ordner::wurzel(&w, reparse_urteil).unwrap();
        let mut besucht = 0;
        let fertig = o
            .auflisten(|_| {
                besucht += 1;
                if besucht == 7 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap();
        assert!(!fertig, "Abbruch heißt unvollständig");
        assert_eq!(besucht, 7, "nach dem Abbruch kein weiterer Eintrag");
        let mut alle = 0;
        assert!(o
            .auflisten(|_| {
                alle += 1;
                ControlFlow::Continue(())
            })
            .unwrap());
        assert_eq!(alle, 50);
    }

    #[test]
    fn das_hochstufen_oeffnet_den_pfad_nicht_noch_einmal() {
        let _n = sauber();
        let (_d, w) = wurzel();
        let p = w.join("brief.txt");
        fs::write(&p, b"original").unwrap();
        fs::write(w.join("fremd.txt"), b"fremd").unwrap();
        // Zwischen Urteil und Hochstufen wird der Pfad auf ein anderes Objekt gelenkt.
        let ziel = p.clone();
        let fremd = w.join("fremd.txt");
        naht::vor_hochstufen_haken(Some(Box::new(move |q: &Path| {
            if q == ziel {
                fs::rename(&fremd, &ziel).unwrap();
            }
        })));
        let r = eintrag::oeffnen_ohne_abruf(&p, art(), reparse_urteil);
        // macOS: Stufe 1 ist nur `O_EVTONLY`; Stufe 2 öffnet per `openat` im geprüften Elternordner und
        // gleicht die Kennung ab. Ein Tausch im selben Ordner wird dort sicher abgelehnt statt gelesen.
        #[cfg(target_os = "macos")]
        assert!(
            matches!(r, Err(eintrag::OeffnenFehler::Getauscht)),
            "der getauschte Eintrag wird abgelehnt"
        );
        #[cfg(not(target_os = "macos"))]
        {
            let mut f = r.unwrap();
            let mut s = String::new();
            std::io::Read::read_to_string(&mut f, &mut s).unwrap();
            assert_eq!(s, "original", "das hochgestufte Handle ist dasselbe Objekt");
        }
        assert_eq!(
            fs::read_to_string(&p).unwrap(),
            "fremd",
            "der Pfad zeigt jetzt woanders hin"
        );
    }

    #[test]
    fn das_urteil_wird_vor_dem_hochstufen_am_selben_handle_wiederholt() {
        let _n = sauber();
        let (_d, w) = wurzel();
        let p = w.join("platzhalter.txt");
        fs::write(&p, b"x").unwrap();
        // Erste Antwort lesbar, zweite (unmittelbar vor dem Hochstufen) nur online: nie hochgestuft.
        naht::handle_folge(&p, &[(0x20, 0), (0x50_1620, 0x9000_701A)]);
        let r = eintrag::oeffnen_ohne_abruf(&p, art(), reparse_urteil);
        assert!(matches!(r, Err(OeffnenFehler::Urteil(Urteil::NurOnline))));
        assert_eq!(naht::stufe2_aufrufe(&p), 0, "Lesezugriff nie erreicht");
    }

    #[test]
    fn ein_gehaltener_ordner_tauscht_keine_namen_gegen_pfade() {
        let _n = sauber();
        let (_d, w) = wurzel();
        fs::write(w.join("a.txt"), b"a").unwrap();
        let root = Ordner::wurzel(&w, reparse_urteil).unwrap();
        // Ein Name mit Trenner erreicht nie einen anderen Ordner.
        let name = OsString::from("../a.txt");
        assert!(root.datei_oeffnen(&name, art(), reparse_urteil).is_err());
    }
}
