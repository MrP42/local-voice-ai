//! D7: Einträge im verknüpften Baum lesen, ohne sie zu öffnen, und über sie urteilen (fail-closed).
//!
//! Unter Windows liegen OneDrive-Dateien, die nur online sind, als Platzhalter (Reparse-Point mit Cloud-Tag) auf
//! dem Datenträger. Wer sie öffnet oder liest, löst einen Download aus. `fs::symlink_metadata` liefert dafür
//! nicht die echten Attribute: Rust öffnet einen Reparse-Point, der kein Symlink ist, noch einmal ohne
//! `FILE_FLAG_OPEN_REPARSE_POINT` und meldet dann `0x500020` statt `0x501620` (ohne `REPARSE_POINT` und
//! `OFFLINE`). Deshalb liest dieses Modul unter Windows Attribute, Größe und Zeit nur über `FindFirstFileExW`
//! (`FindExInfoBasic`): Die Funktion öffnet die Datei nicht und liefert `dwFileAttributes` samt `dwReserved0`
//! (Reparse-Tag bei gesetztem `REPARSE_POINT`). Die Metadaten von `DirEntry` werden nicht benutzt.
//!
//! Urteil ([`reparse_urteil`], rein und plattformneutral): zuerst „nur online“ (`RECALL_ON_DATA_ACCESS`,
//! `RECALL_ON_OPEN`, `OFFLINE`), unabhängig vom Reparse-Bit und bei Dateien wie bei Ordnern; ein solcher
//! Eintrag wird nicht gelesen, geöffnet oder betreten (auch das Auflisten eines Ordners kann eine
//! Synchronisierung anstoßen). Danach der Tag: Symlink, Mount-Point und unbekannte Tags werden abgewiesen,
//! Cloud-Tags nur bei hydrierten Einträgen zugelassen.
//!
//! Fehlerfälle und Absicherung:
//! * Datei wird zwischen Abtastung und Lesen (oder zwischen Prüfung und Öffnen) wieder online: Prüfung am
//!   offenen Handle vor dem ersten Lesen ([`offen_urteil`]), Öffnen mit `FILE_FLAG_OPEN_NO_RECALL`; ein
//!   solches Handle wird ohne zu lesen geschlossen.
//! * Attribute, Tag oder Handle nicht bestimmbar (Fehler der Windows-Aufrufe): als Umweg abgewiesen.
//! * Auflisten bricht mittendrin ab (Laufwerk weg, Zugriff verweigert): der ganze Ordner gilt als nicht
//!   lesbar, nie eine Teilliste (sonst fielen bekannte Dateien aus dem Bestand).
//! * Unbekannte oder künftige Tags und Attribute: Tags nur über die Cloud-Maske zugelassen, jedes andere
//!   Reparse-Tag abgewiesen; jedes der drei Online-Bits genügt.
//!
//! Plattformneutral getestet wird über die Test-Naht [`naht`], die Attribute und Tags einspeist.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

/// Verzeichnis (nur Windows liefert Attribute).
#[cfg_attr(not(windows), allow(dead_code))]
pub const ATTR_VERZEICHNIS: u32 = 0x10;
/// Versteckt.
pub const ATTR_VERSTECKT: u32 = 0x2;
/// System.
pub const ATTR_SYSTEM: u32 = 0x4;
/// Reparse-Point (Symlink, Junction, Cloud-Platzhalter, …).
pub const ATTR_REPARSE_POINT: u32 = 0x400;
/// Daten nicht sofort verfügbar (Offline).
pub const ATTR_OFFLINE: u32 = 0x1000;
/// Das Öffnen löst einen Abruf aus.
pub const ATTR_RECALL_ON_OPEN: u32 = 0x4_0000;
/// Der Datenzugriff löst einen Abruf aus (OneDrive: nur online).
pub const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
/// Jedes dieser Bits heißt: nur online, nie lesen, öffnen oder betreten.
pub const ATTR_NUR_ONLINE: u32 = ATTR_OFFLINE | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS;

/// Reparse-Tag: Symlink.
pub const REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
/// Reparse-Tag: Mount-Point (Junction).
pub const REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
/// Basis der Cloud-Tags (`IO_REPARSE_TAG_CLOUD`); `IO_REPARSE_TAG_CLOUD_1` bis `_F` setzen die Bits 12 bis 15.
const REPARSE_TAG_CLOUD: u32 = 0x9000_001A;
const REPARSE_TAG_CLOUD_MASKE: u32 = 0xFFFF_0FFF;

/// Ist `tag` ein Cloud-Tag (OneDrive und andere Cloud-Dateien-Anbieter)?
pub fn reparse_tag_cloud(tag: u32) -> bool {
    tag & REPARSE_TAG_CLOUD_MASKE == REPARSE_TAG_CLOUD
}

/// Urteil über einen Eintrag im verknüpften Baum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urteil {
    /// Gewöhnlich oder hydrierter Cloud-Platzhalter: darf gelesen und betreten werden.
    Lesbar,
    /// Nur online: weder lesen noch öffnen noch betreten (sonst Download oder Synchronisierung).
    NurOnline,
    /// Verknüpfung (Symlink, Junction) oder unbekannter Reparse-Point: nie folgen, nie lesen.
    Umweg,
}

/// Verlangen die Attribute einen Abruf (nur online)? Gilt für Dateien und Ordner, unabhängig vom
/// Reparse-Bit.
pub fn online_urteil(attribute: u32) -> bool {
    attribute & ATTR_NUR_ONLINE != 0
}

/// Wertet Attribute und Reparse-Tag in dieser Reihenfolge: (1) nur online, unabhängig vom Reparse-Bit und
/// vom Tag; (2) ohne Reparse-Bit ist der Tag belanglos: lesbar; (3) Symlink, Mount-Point und jedes nicht
/// bekannte Tag: Umweg; Cloud-Tags sind hier nur noch hydriert, also lesbar.
pub fn reparse_urteil(tag: u32, attribute: u32) -> Urteil {
    if online_urteil(attribute) {
        return Urteil::NurOnline;
    }
    if attribute & ATTR_REPARSE_POINT == 0 {
        return Urteil::Lesbar;
    }
    if tag == REPARSE_TAG_SYMLINK || tag == REPARSE_TAG_MOUNT_POINT || !reparse_tag_cloud(tag) {
        return Urteil::Umweg;
    }
    Urteil::Lesbar
}

/// Art eines Eintrags (ohne Verknüpfungen zu folgen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gattung {
    Datei,
    Ordner,
    /// Symlink (nur außerhalb von Windows; dort entscheidet der Reparse-Tag).
    #[cfg_attr(windows, allow(dead_code))]
    Link,
    /// Gerät, Pipe, Socket, …
    #[cfg_attr(windows, allow(dead_code))]
    Anderes,
}

/// Rohdaten eines Eintrags, ohne ihn zu öffnen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Roh {
    pub gattung: Gattung,
    /// Windows-Dateiattribute (`dwFileAttributes`); außerhalb von Windows 0.
    pub attribute: u32,
    /// Reparse-Tag bei gesetztem `REPARSE_POINT`, sonst 0.
    pub tag: u32,
    pub groesse: u64,
    /// Änderungszeit (ms seit 1970).
    pub zeit: i64,
    /// Anzahl der Verzeichniseinträge (nur Unix); sonst 1.
    pub nlink: u64,
}

impl Roh {
    /// Das Urteil über diesen Eintrag.
    pub fn urteil(&self) -> Urteil {
        if self.gattung == Gattung::Link {
            return Urteil::Umweg;
        }
        reparse_urteil(self.tag, self.attribute)
    }

    pub fn ist_datei(&self) -> bool {
        self.gattung == Gattung::Datei
    }

    pub fn ist_ordner(&self) -> bool {
        self.gattung == Gattung::Ordner
    }

    /// Windows: versteckt oder System (wird übersprungen).
    pub fn versteckt(&self) -> bool {
        self.attribute & (ATTR_VERSTECKT | ATTR_SYSTEM) != 0
    }
}

/// Ein Eintrag eines Verzeichnisses: Name und, wenn lesbar, die Rohdaten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eintrag {
    pub name: OsString,
    /// `None`: Eintrag nicht lesbar (zählt als ausgelassen).
    pub roh: Option<Roh>,
}

/// Rohdaten eines Pfades, ohne ihn zu öffnen und ohne einer Verknüpfung zu folgen.
pub fn roh(p: &Path) -> io::Result<Roh> {
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut r = system::eins(p)?;
    #[cfg(test)]
    naht::anwenden(p, &mut r);
    Ok(r)
}

/// Einträge eines Verzeichnisses mit Rohdaten (ohne `.` und `..`). Ein Fehler mitten im Auflisten ist ein
/// Fehler des ganzen Verzeichnisses.
pub fn verzeichnis(p: &Path) -> io::Result<Vec<Eintrag>> {
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut v = system::alle(p)?;
    #[cfg(test)]
    for e in &mut v {
        if let Some(r) = e.roh.as_mut() {
            naht::anwenden(&p.join(&e.name), r);
        }
    }
    Ok(v)
}

/// Attribute und Reparse-Tag einer schon geöffneten Datei, vom Handle (`GetFileInformationByHandleEx` mit
/// `FileAttributeTagInfo`), nicht vom Pfad. `None`: nicht bestimmbar. `p` ist nur für die Test-Naht.
pub fn offen_daten(f: &fs::File, p: &Path) -> Option<(u32, u32)> {
    let _ = p;
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut d = system::offen(f).ok();
    #[cfg(test)]
    if let Some(x) = naht::handle(p) {
        d = Some(x);
    }
    d
}

/// Urteil über eine schon geöffnete Datei, vom Handle (siehe [`offen_daten`]). Nicht bestimmbar: Umweg (im
/// Zweifel ablehnen).
pub fn offen_urteil(f: &fs::File, p: &Path) -> Urteil {
    match offen_daten(f, p) {
        Some((attribute, tag)) => reparse_urteil(tag, attribute),
        None => Urteil::Umweg,
    }
}

#[cfg(not(windows))]
mod system {
    use super::{Eintrag, Gattung, Roh};
    use std::fs;
    use std::io;
    use std::path::Path;

    fn aus_meta(m: &fs::Metadata) -> Roh {
        let t = m.file_type();
        let gattung = if t.is_symlink() {
            Gattung::Link
        } else if t.is_dir() {
            Gattung::Ordner
        } else if t.is_file() {
            Gattung::Datei
        } else {
            Gattung::Anderes
        };
        #[cfg(unix)]
        let nlink = {
            use std::os::unix::fs::MetadataExt;
            m.nlink()
        };
        #[cfg(not(unix))]
        let nlink = 1;
        Roh {
            gattung,
            attribute: 0,
            tag: 0,
            groesse: m.len(),
            zeit: m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64),
            nlink,
        }
    }

    pub fn eins(p: &Path) -> io::Result<Roh> {
        fs::symlink_metadata(p).map(|m| aus_meta(&m))
    }

    pub fn alle(p: &Path) -> io::Result<Vec<Eintrag>> {
        let mut aus = Vec::new();
        for e in fs::read_dir(p)? {
            let Ok(e) = e else {
                aus.push(Eintrag {
                    name: std::ffi::OsString::new(),
                    roh: None,
                });
                continue;
            };
            let roh = fs::symlink_metadata(e.path()).ok().map(|m| aus_meta(&m));
            aus.push(Eintrag {
                name: e.file_name(),
                roh,
            });
        }
        Ok(aus)
    }

    pub fn offen(_f: &fs::File) -> io::Result<(u32, u32)> {
        Ok((0, 0))
    }
}

#[cfg(windows)]
mod system {
    use super::{Eintrag, Gattung, Roh, ATTR_REPARSE_POINT, ATTR_VERZEICHNIS};
    use std::ffi::{c_void, OsStr, OsString};
    use std::fs;
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_NO_MORE_FILES, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
        FindNextFileW, GetFileInformationByHandleEx, FILE_ATTRIBUTE_TAG_INFO,
        FIND_FIRST_EX_LARGE_FETCH, WIN32_FIND_DATAW,
    };

    /// 100-ns-Schritte zwischen 1601-01-01 und 1970-01-01.
    const FILETIME_BIS_1970: u64 = 116_444_736_000_000_000;

    /// Schließt das Suchhandle (auch bei einem frühen Rückkehr).
    struct Suche(HANDLE);

    impl Drop for Suche {
        fn drop(&mut self) {
            // SAFETY: gültiges Suchhandle aus `FindFirstFileExW`, wird genau einmal geschlossen.
            let _ = unsafe { FindClose(self.0) };
        }
    }

    fn breit(p: &OsStr) -> io::Result<Vec<u16>> {
        let mut v: Vec<u16> = p.encode_wide().collect();
        if v.contains(&0) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        v.push(0);
        Ok(v)
    }

    fn roh_aus(d: &WIN32_FIND_DATAW) -> Roh {
        let attribute = d.dwFileAttributes;
        let tag = if attribute & ATTR_REPARSE_POINT != 0 {
            d.dwReserved0
        } else {
            0
        };
        let ft = (u64::from(d.ftLastWriteTime.dwHighDateTime) << 32)
            | u64::from(d.ftLastWriteTime.dwLowDateTime);
        Roh {
            gattung: if attribute & ATTR_VERZEICHNIS != 0 {
                Gattung::Ordner
            } else {
                Gattung::Datei
            },
            attribute,
            tag,
            groesse: (u64::from(d.nFileSizeHigh) << 32) | u64::from(d.nFileSizeLow),
            zeit: ft
                .checked_sub(FILETIME_BIS_1970)
                .map_or(0, |x| (x / 10_000) as i64),
            nlink: 1,
        }
    }

    fn name_aus(d: &WIN32_FIND_DATAW) -> OsString {
        let n = d
            .cFileName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(d.cFileName.len());
        OsString::from_wide(&d.cFileName[..n])
    }

    /// Ein Eintrag über `FindFirstFileExW`: öffnet die Datei nicht, folgt keiner Verknüpfung.
    pub fn eins(p: &Path) -> io::Result<Roh> {
        let w = breit(p.as_os_str())?;
        let mut d = WIN32_FIND_DATAW::default();
        // SAFETY: `w` ist NUL-terminiert und lebt über den Aufruf; `d` ist ein beschreibbarer
        // `WIN32_FIND_DATAW` (bei `FindExInfoBasic` ist das der erwartete Puffer).
        let h = unsafe {
            FindFirstFileExW(
                PCWSTR(w.as_ptr()),
                FindExInfoBasic,
                std::ptr::addr_of_mut!(d).cast::<c_void>(),
                FindExSearchNameMatch,
                None,
                FIND_FIRST_EX_LARGE_FETCH,
            )
        }
        .map_err(io::Error::other)?;
        let _suche = Suche(h);
        Ok(roh_aus(&d))
    }

    /// Alle Einträge eines Ordners über `FindFirstFileExW` und `FindNextFileW`.
    pub fn alle(p: &Path) -> io::Result<Vec<Eintrag>> {
        let w = breit(p.join("*").as_os_str())?;
        let mut d = WIN32_FIND_DATAW::default();
        // SAFETY: wie in `eins`.
        let h = unsafe {
            FindFirstFileExW(
                PCWSTR(w.as_ptr()),
                FindExInfoBasic,
                std::ptr::addr_of_mut!(d).cast::<c_void>(),
                FindExSearchNameMatch,
                None,
                FIND_FIRST_EX_LARGE_FETCH,
            )
        }
        .map_err(io::Error::other)?;
        let suche = Suche(h);
        let mut aus = Vec::new();
        loop {
            let name = name_aus(&d);
            if name != "." && name != ".." {
                aus.push(Eintrag {
                    name,
                    roh: Some(roh_aus(&d)),
                });
            }
            // SAFETY: gültiges Suchhandle, `d` ist ein beschreibbarer `WIN32_FIND_DATAW`.
            match unsafe { FindNextFileW(suche.0, &mut d) } {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_NO_MORE_FILES.to_hresult() => break,
                Err(e) => return Err(io::Error::other(e)),
            }
        }
        Ok(aus)
    }

    /// Attribute und Reparse-Tag am offenen Handle (`FileAttributeTagInfo`).
    pub fn offen(f: &fs::File) -> io::Result<(u32, u32)> {
        let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
        // SAFETY: gültiges, offenes Handle; `info` ist ein beschreibbarer Puffer der angegebenen Größe.
        unsafe {
            GetFileInformationByHandleEx(
                HANDLE(f.as_raw_handle()),
                FileAttributeTagInfo,
                std::ptr::addr_of_mut!(info).cast::<c_void>(),
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        }
        .map_err(io::Error::other)?;
        Ok((info.FileAttributes, info.ReparseTag))
    }
}

/// Test-Naht: speist Attribute und Tags ein, wie Windows sie für OneDrive-Dateien liefert (die Art des
/// Eintrags, Größe und Zeit bleiben die echten). Gilt nur für den eigenen Test-Faden.
#[cfg(test)]
pub(crate) mod naht {
    use super::Roh;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    type Karte = RefCell<BTreeMap<PathBuf, (u32, u32)>>;

    thread_local! {
        /// Pfad → (Attribute, Tag) der Find-Daten.
        static FIND: Karte = const { RefCell::new(BTreeMap::new()) };
        /// Pfad → (Attribute, Tag) am offenen Handle.
        static HANDLE: Karte = const { RefCell::new(BTreeMap::new()) };
    }

    /// Find-Daten dieses Pfades (Attribute, Tag) vortäuschen.
    pub fn setzen(p: &Path, attribute: u32, tag: u32) {
        FIND.with(|k| k.borrow_mut().insert(p.to_path_buf(), (attribute, tag)));
    }

    /// Das Handle dieser Datei meldet (Attribute, Tag).
    pub fn handle_setzen(p: &Path, attribute: u32, tag: u32) {
        HANDLE.with(|k| k.borrow_mut().insert(p.to_path_buf(), (attribute, tag)));
    }

    pub fn leeren() {
        FIND.with(|k| k.borrow_mut().clear());
        HANDLE.with(|k| k.borrow_mut().clear());
    }

    pub(super) fn anwenden(p: &Path, r: &mut Roh) {
        if let Some((attribute, tag)) = FIND.with(|k| k.borrow().get(p).copied()) {
            r.attribute = attribute;
            r.tag = tag;
        }
    }

    pub(super) fn handle(p: &Path) -> Option<(u32, u32)> {
        HANDLE.with(|k| k.borrow().get(p).copied())
    }
}
