//! Windows-Prozessabfragen ohne Lesezugriff auf fremden Prozessspeicher (lva#873).
//! Die Liste kommt aus Toolhelp. Nur passende Pfadkandidaten und moegliche
//! eigene Nachfahren werden mit eingeschraenktem Abfragerecht geoeffnet.

use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows::core::{Result, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Dieser Wrapper besitzt genau ein erfolgreich geoeffnetes Handle.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn open(pid: u32) -> Result<Handle> {
    // SAFETY: Keine Zeiger, keine Vererbung; niemals ein Versuch mit mehr Rechten.
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map(Handle) }
}

/// Eine Toolhelp-Zeile. Fuer diese Daten wird kein Prozess geoeffnet.
pub(crate) struct Entry {
    pub pid: u32,
    pub ppid: u32,
    pub name: OsString,
}

/// Vollstaendige Momentaufnahme; ein Abbruch der Iteration ist kein Teilerfolg.
pub(crate) fn snapshot() -> Result<Vec<Entry>> {
    // SAFETY: Gueltige lokale Struktur mit vorgeschriebener Groesse; der
    // Snapshot bleibt bis zum Ende der Iteration offen und wird per RAII geschlossen.
    unsafe {
        let handle = Handle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut next = Process32FirstW(handle.0, &mut entry);
        let mut rows = Vec::new();
        loop {
            match next {
                Ok(()) => {
                    let end = entry.szExeFile.iter().position(|c| *c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    rows.push(Entry {
                        pid: entry.th32ProcessID,
                        ppid: entry.th32ParentProcessID,
                        name: OsString::from_wide(&entry.szExeFile[..end]),
                    });
                }
                Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => return Ok(rows),
                Err(error) => return Err(error),
            }
            next = Process32NextW(handle.0, &mut entry);
        }
    }
}

fn image_path(handle: &Handle) -> Result<PathBuf> {
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    // SAFETY: Schreibbarer UTF-16-Puffer mit der uebergebenen Kapazitaet.
    unsafe {
        QueryFullProcessImageNameW(handle.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)?;
    }
    Ok(PathBuf::from(OsString::from_wide(&buf[..len as usize])))
}

fn norm_path(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_ascii_lowercase()
}

/// Pfadvergleich nur nach passendem EXE-Namen. Ein unlesbarer Pfad ist kein Treffer.
pub(crate) fn path_alive(rows: &[Entry], path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let name = name.to_string_lossy();
    let want = norm_path(path);
    rows.iter()
        .filter(|row| row.name.to_string_lossy().eq_ignore_ascii_case(&name))
        .any(|row| {
            let Ok(handle) = open(row.pid) else {
                return false;
            };
            running(&handle).unwrap_or(false)
                && image_path(&handle).is_ok_and(|actual| norm_path(&actual) == want)
        })
}

fn running(handle: &Handle) -> Result<bool> {
    let mut code = 0;
    // SAFETY: Gueltiges Handle und Zeiger auf eine lokale Zahl.
    unsafe { GetExitCodeProcess(handle.0, &mut code)? };
    // Ein Prozess darf auch mit Exitcode 259 enden; dann ist seine Endzeit gesetzt.
    Ok(code == 259 && times(handle)?.1 == 0) // STILL_ACTIVE
}

/// Lebt eine bekannte Kind-PID noch? Fehler oder ein beendeter Prozess sind kein Ja.
#[cfg(test)]
pub(crate) fn pid_alive(pid: u32) -> bool {
    open(pid).and_then(|handle| running(&handle)).unwrap_or(false)
}

fn times(handle: &Handle) -> Result<(u64, u64)> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: Vier gueltige, getrennte Ausgabepuffer.
    unsafe { GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user)? };
    let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    Ok((ticks(created), ticks(exited)))
}

fn child_belongs(parent_start: u64, parent_end: u64, child_start: u64) -> bool {
    child_start >= parent_start && (parent_end == 0 || child_start <= parent_end)
}

fn same_candidate(before: &Entry, now: &Entry) -> bool {
    before.pid == now.pid && before.ppid == now.ppid
}

/// Gepruefter eigener Prozess. Das Handle haelt seine Identitaet bis zur Messung
/// fest: eine zwischenzeitlich wiedervergebene PID wird nicht erneut geoeffnet.
pub(crate) struct OwnedProcess {
    pub pid: u32,
    handle: Handle,
    created: u64,
}

impl OwnedProcess {
    /// Arbeitssatz in Bytes. Kein Rueckfall auf weitergehende Zugriffsrechte.
    pub(crate) fn memory_bytes(&self) -> Result<u64> {
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        // SAFETY: Das Handle stammt aus own_tree; Struktur und Groesse passen.
        unsafe {
            K32GetProcessMemoryInfo(
                self.handle.0,
                &mut counters,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ).ok()?;
        }
        Ok(counters.WorkingSetSize as u64)
    }
}

/// Nur der eigene Baum. Eltern-PIDs aus Toolhelp sind blosse Kandidaten:
/// Startzeiten muessen zur Lebenszeit des per Handle festgehaltenen Elternteils
/// passen. Unlesbare Kandidaten werden ausgeschlossen, nie mit Nullzeit akzeptiert.
pub(crate) fn own_tree() -> Result<Vec<OwnedProcess>> {
    let rows = snapshot()?;
    let root = std::process::id();
    own_tree_with(
        open_owned(root)?,
        &rows,
        open_owned,
        |process| times(&process.handle).map(|(_, end)| end),
        snapshot,
    )
}

fn open_owned(pid: u32) -> Result<OwnedProcess> {
    let handle = open(pid)?;
    let (created, _) = times(&handle)?;
    Ok(OwnedProcess { pid, handle, created })
}

fn own_tree_with(
    root: OwnedProcess,
    rows: &[Entry],
    mut open: impl FnMut(u32) -> Result<OwnedProcess>,
    mut end_time: impl FnMut(&OwnedProcess) -> Result<u64>,
    mut snapshot: impl FnMut() -> Result<Vec<Entry>>,
) -> Result<Vec<OwnedProcess>> {
    let mut seen = HashSet::from([root.pid]);
    let mut result = vec![root];
    let mut index = 0;
    while index < result.len() {
        let level_end = result.len();
        let mut candidates = Vec::new();
        for parent_index in index..level_end {
            let parent = &result[parent_index];
            for row in rows.iter().filter(|row| row.ppid == parent.pid) {
                if !seen.insert(row.pid) {
                    continue;
                }
                if let Ok(process) = open(row.pid) {
                    candidates.push((parent_index, row, process));
                }
            }
        }
        if !candidates.is_empty() {
            // Erst alle Handles festhalten, dann genau eine frische Pruefung
            // fuer die Ebene. Auch Eltern-Handles bleiben bis zur Messung offen.
            let fresh = snapshot()?;
            let ends: Vec<u64> = result[index..level_end].iter()
                .map(&mut end_time).collect::<Result<_>>()?;
            for (parent_index, row, process) in candidates {
                let parent = &result[parent_index];
                if fresh.iter().any(|now| same_candidate(row, now))
                    && child_belongs(parent.created, ends[parent_index - index], process.created)
                {
                    result.push(process);
                }
            }
        }
        index = level_end;
    }
    Ok(result)
}

/// Messung einer PID nur, wenn sie zum jetzt geprueften eigenen Baum gehoert.
pub(crate) fn memory_bytes(pid: u32) -> Result<Option<u64>> {
    own_tree()?
        .iter()
        .find(|process| process.pid == pid)
        .map(OwnedProcess::memory_bytes)
        .transpose()
}

#[cfg(test)]
mod tests;
