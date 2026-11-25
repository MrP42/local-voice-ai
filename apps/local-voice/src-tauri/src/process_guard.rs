//! Schutz vor einem Systemausfall durch eigene Kindprozesse.
//!
//! Am 15.09.2026 fror der Rechner ein, als Fish Speech mit `--compile`
//! startete: Torch Inductor legt so viele Compile-Prozesse an wie logische
//! Kerne (hier 32), jeder importiert Torch, und bei 44 GB bereits belegtem
//! RAM kippte Windows in die Auslagerung — kein Taskmanager, kein Neustart,
//! nur der Netzschalter. Das darf durch diese App nie wieder passieren.
//!
//! Drei Schichten, jede für sich wirksam:
//! 1. **Start-Gate:** Ein Server startet nur, wenn genug RAM frei ist.
//! 2. **Job-Objekt (Windows):** Jeder Kindprozess samt seiner Kinder bekommt
//!    einen harten Speicher-Deckel, einen CPU-Deckel und niedrige Prioritaet.
//!    Überschreitet der Baum den Deckel, scheitert SEINE Allokation — nicht
//!    die des Systems. Schliesst die App (auch durch Absturz), sterben die
//!    Kinder mit.
//! 3. **Speicherwächter:** Faellt der freie RAM unter die Notgrenze, stoppt
//!    die App ihre Server, bevor das Betriebssystem unbedienbar wird.

use std::time::Duration;

/// RAM, den die App dem System hoechstens laesst (MB). Darunter wird kein
/// Server gestartet, und ein Kindprozess darf nie mehr als (frei - Reserve)
/// belegen. Die wirksame Reserve liefert [`ram_reserve_mb`].
pub const RAM_RESERVE_MB: u64 = 6 * 1024;
/// Untergrenze der Reserve (MB), auch auf kleinen Rechnern.
pub const RAM_RESERVE_MIN_MB: u64 = 3 * 1024;
/// Notgrenze des Wächters (MB): darunter werden laufende Server gestoppt.
pub const RAM_EMERGENCY_MB: u64 = 2 * 1024;
/// Unter diesem Deckel lohnt kein Start: das Modell passt dann ohnehin nicht.
pub const RAM_MIN_LIMIT_MB: u64 = 4 * 1024;
/// Anteil der CPU, den ein Kindprozessbaum hoechstens bekommt (Prozent).
pub const CPU_CAP_PERCENT: u32 = 75;
pub const WATCH_INTERVAL: Duration = Duration::from_secs(5);

/// M7-P7b (QG4): Testschalter fuer knappen Speicher, ohne den Rechner
/// wirklich zu fuellen. `LVA_TEST_FREE_RAM_MB=<n>` deckelt den gemessenen
/// freien RAM auf n MB. Er kann Tore nur SCHLIESSEN, nie oeffnen (Minimum
/// mit dem Messwert); ungueltige Werte werden ignoriert.
pub const TEST_FREE_RAM_ENV: &str = "LVA_TEST_FREE_RAM_MB";

/// Reserve fuer einen Rechner mit `total_mb` RAM: ein Fuenftel, zwischen
/// [`RAM_RESERVE_MIN_MB`] und [`RAM_RESERVE_MB`]. Feste 6 GB verlangten auf
/// dem 16-GB-Zielgeraet (docs/PORTABLE.md) 8,6 GB frei fuer das 2,6-GB-Modell;
/// ab 32 GB bleibt es bei 6 GB. `0` heisst "nicht messbar": dann die volle
/// Reserve.
pub fn reserve_for_total_mb(total_mb: u64) -> u64 {
    if total_mb == 0 {
        return RAM_RESERVE_MB;
    }
    (total_mb / 5).clamp(RAM_RESERVE_MIN_MB, RAM_RESERVE_MB)
}

/// Wirksame Reserve dieses Rechners (MB); der Gesamtspeicher aendert sich
/// zur Laufzeit nicht und wird einmal gemessen.
pub fn ram_reserve_mb() -> u64 {
    static RESERVE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *RESERVE.get_or_init(|| {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        reserve_for_total_mb(sys.total_memory() / (1024 * 1024))
    })
}

/// Freier physischer Speicher in MB (0, wenn nicht messbar).
pub fn available_ram_mb() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let measured = sys.available_memory() / (1024 * 1024);
    let cap = std::env::var(TEST_FREE_RAM_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok());
    capped_free_mb(measured, cap)
}

/// Messwert unter dem Testdeckel: nie mehr als gemessen, und "nicht
/// messbar" (0) wird mit Deckel zum Deckel (strenger, nicht offener).
fn capped_free_mb(measured: u64, cap: Option<u64>) -> u64 {
    // 0 hiesse "nicht messbar" und liesse alles durch: der Deckel ist >= 1.
    match cap.map(|c| c.max(1)) {
        Some(cap) if measured == 0 => cap,
        Some(cap) => measured.min(cap),
        None => measured,
    }
}

pub fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Torch-Inductor-Compile-Prozesse: 2 bis 4 statt "ein Prozess je Kern".
/// Vier reichen fuer den ~60-s-Compile; 32 waren der Ausloeser des Ausfalls.
///
/// Auf Windows immer 1: jeder Wert > 1 laesst Torch (2.8) einen
/// `SubprocPool` starten, der `pass_fds` nutzt — das gibt es unter Windows
/// nicht, der Server bricht mit "pass_fds not supported on Windows" ab
/// (21.09.2026). Torch selbst nimmt auf win32 ebenfalls 1.
pub fn compile_threads_for(cpus: usize) -> usize {
    if cfg!(windows) {
        return 1;
    }
    (cpus / 8).clamp(2, 4)
}

/// Rechen-Threads (OMP/MKL) fuer ein Modell: die Haelfte der Kerne, damit
/// die Oberflaeche des Rechners bedienbar bleibt.
pub fn cpu_threads_for(cpus: usize) -> usize {
    (cpus / 2).clamp(4, 16)
}

/// Prueft vor einem Serverstart, ob genug RAM frei ist. `need_mb` ist der
/// Bedarf des Servers; die Reserve kommt obendrauf.
pub fn check_ram_for_start(need_mb: u64) -> Result<u64, String> {
    let free = available_ram_mb();
    if free == 0 {
        return Ok(0); // nicht messbar: nicht blockieren
    }
    // lva#654: dieselbe Logik und dieselbe Grenze wie der Speicherschutz (`memory_guard`),
    // keine zweite feste Reserve: bei ausgeschaltetem Schutz sperrt nichts, kleine Modelle
    // laden mit Warnung, sonst gilt die Grenze der Einstellung (Standard 98 %).
    let (enabled, percent) = crate::memory_guard::current_config();
    if !enabled || crate::memory_guard::allow_active() {
        return Ok(free);
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let total = sys.total_memory() / (1024 * 1024);
    // Unter dem Testdeckel ist `free` kleiner als gemessen: `used` folgt dem.
    let used = total.saturating_sub(free);
    // Grosse Modelle: der Waechter beendet Prozesse unter `RAM_EMERGENCY_MB` frei; ein Start, der
    // dort enden wuerde, wird von vornherein abgewiesen (statt gestartet und gleich abgebrochen).
    let leaves_too_little = need_mb > crate::memory_guard::SMALL_MODEL_MB
        && free < need_mb.saturating_add(RAM_EMERGENCY_MB);
    let verdict = if leaves_too_little {
        crate::memory_guard::Verdict::Block
    } else {
        crate::memory_guard::judge(used, need_mb, total, percent)
    };
    match verdict {
        crate::memory_guard::Verdict::Block => Err(format!(
            "Zu wenig freier Arbeitsspeicher: {:.1} GB frei von {:.1} GB, gebraucht werden etwa {:.1} GB; die Grenze des Speicherschutzes liegt bei {percent} %. Schließen Sie andere Programme oder erhöhen Sie die Grenze in den Einstellungen.",
            free as f64 / 1024.0,
            total as f64 / 1024.0,
            need_mb as f64 / 1024.0,
        )),
        _ => Ok(free),
    }
}

/// Speicher-Deckel fuer einen Kindprozessbaum: alles, was frei ist, minus
/// die Reserve — nie unter `RAM_MIN_LIMIT_MB`.
pub fn memory_limit_mb(free_mb: u64) -> u64 {
    free_mb
        .saturating_sub(ram_reserve_mb())
        .max(RAM_MIN_LIMIT_MB)
}

/// Beendet Windows gerade die Sitzung (Herunterfahren, Neustart, Abmelden)?
///
/// Dann startet kein neuer Prozess mehr: jeder scheitert beim Laden seiner
/// DLLs mit 0xc0000142, und Windows zeigt dafuer ein Fehlerfenster. So kam
/// "NETSTAT.EXE - Anwendungsfehler" bei jedem Herunterfahren zustande
/// (beobachtet 18. bis 27.09.2026): `stop_server` startete beim Beenden
/// immer `netstat`. Die Aufraeumwege starten in dieser Lage deshalb keine
/// Hilfsprogramme. Die eigenen Kindprozesse beendet das Job-Objekt
/// (KILL_ON_JOB_CLOSE), alles Uebrige beendet Windows ohnehin.
pub fn session_ending() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SHUTTINGDOWN};
        // SAFETY: GetSystemMetrics liest nur einen Systemwert und nimmt keine Zeiger.
        unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Haelt das Job-Objekt eines Kindprozesses. Beim Drop wird das Job-Objekt
/// geschlossen, und mit `KILL_ON_JOB_CLOSE` stirbt der ganze Prozessbaum —
/// also nur droppen, wenn der Prozess ohnehin beendet werden soll.
pub struct ProcessGuard {
    #[cfg(windows)]
    job: windows::Win32::Foundation::HANDLE,
}

// SAFETY: ein Job-Handle ist ein Kernel-Objekt ohne Thread-Bindung.
unsafe impl Send for ProcessGuard {}
unsafe impl Sync for ProcessGuard {}

impl ProcessGuard {
    /// Uebernimmt einen angehalten gestarteten Prozess. Windows erlaubt seit
    /// Windows 8 geschachtelte Jobs: der geerbte Job bleibt bestehen. Verweigert
    /// dessen Richtlinie die Zuordnung, laeuft das Kind nie ungeschuetzt an.
    pub fn attach_suspended(
        child: &mut std::process::Child,
        memory_limit_mb: Option<u64>,
        cpu_percent: u32,
    ) -> Result<Option<Self>, String> {
        let guard = Self::attach(child, memory_limit_mb, cpu_percent);
        #[cfg(windows)]
        {
            let resumed = if guard.is_some() {
                resume_suspended(child)
            } else {
                Err("Prozessschutz (Job-Objekt) nicht verfuegbar".to_string())
            };
            if let Err(e) = resumed {
                kill_tree(child, guard);
                return Err(e);
            }
        }
        Ok(guard)
    }

    /// Haengt `child` in ein neues Job-Objekt mit Speicher-, CPU- und
    /// Prioritaetsdeckel. `None`, wenn das Betriebssystem das nicht kann;
    /// der Prozess laeuft dann ohne Deckel weiter (Gate und Wächter bleiben).
    #[cfg(windows)]
    ///
    /// `memory_limit_mb = None`: kein Speicherdeckel, nur CPU, Prioritaet und
    /// KILL_ON_JOB_CLOSE. Noetig fuer GPU-Prozesse: Fish Speech s2-pro belegt
    /// beim Laden kurzzeitig 48 GB Commit (CUDA-Adressraum, Modell geht erst
    /// durch den RAM) — mit einem Deckel von 22 GB starb er mit 0xc0000005
    /// (gemessen 20.09.2026). Start-Gate und Speicherwaechter bleiben.
    pub fn attach(
        child: &std::process::Child,
        memory_limit_mb: Option<u64>,
        cpu_percent: u32,
    ) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        Self::attach_handle(
            HANDLE(child.as_raw_handle() as *mut core::ffi::c_void),
            memory_limit_mb,
            cpu_percent,
        )
    }

    #[cfg(windows)]
    fn attach_handle(
        process: windows::Win32::Foundation::HANDLE,
        memory_limit_mb: Option<u64>,
        cpu_percent: u32,
    ) -> Option<Self> {
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectCpuRateControlInformation,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_CPU_RATE_CONTROL_INFORMATION, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE,
            JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP, JOB_OBJECT_LIMIT_JOB_MEMORY,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PRIORITY_CLASS,
        };
        use windows::Win32::System::Threading::BELOW_NORMAL_PRIORITY_CLASS;

        // SAFETY: Win32-Aufrufe mit korrekt dimensionierten, initialisierten
        // Strukturen; das Handle wird im Drop wieder geschlossen.
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;

            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PRIORITY_CLASS;
            limits.BasicLimitInformation.PriorityClass = BELOW_NORMAL_PRIORITY_CLASS.0;
            if let Some(mb) = memory_limit_mb {
                limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
                limits.JobMemoryLimit = (mb as usize) * 1024 * 1024;
            }
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .is_err()
            {
                let _ = windows::Win32::Foundation::CloseHandle(job);
                return None;
            }

            let cpu = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
                ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE
                    | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
                Anonymous: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0 {
                    // Hundertstel Prozent der Gesamt-CPU.
                    CpuRate: cpu_percent.clamp(10, 100) * 100,
                },
            };
            // Ein fehlender CPU-Deckel ist kein Grund, den RAM-Deckel wegzuwerfen.
            if let Err(e) = SetInformationJobObject(
                job,
                JobObjectCpuRateControlInformation,
                &cpu as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as u32,
            ) {
                log::warn!("process guard: CPU cap not applied: {e}");
            }

            if let Err(e) = AssignProcessToJobObject(job, process) {
                log::warn!("process guard: could not assign process to job: {e}");
                let _ = windows::Win32::Foundation::CloseHandle(job);
                return None;
            }
            log::info!(
                "process guard: pid {} limited to {} RAM, {}% CPU, below-normal priority",
                windows::Win32::System::Threading::GetProcessId(process),
                memory_limit_mb
                    .map(|mb| format!("{mb} MB"))
                    .unwrap_or_else(|| "no cap (GPU process)".to_string()),
                cpu_percent
            );
            Some(Self { job })
        }
    }

    // Gleiche Signatur wie die Windows-Variante: sonst bricht nur der
    // macOS-Build (app-v0.20.0), der Windows-Build merkt davon nichts.
    #[cfg(not(windows))]
    pub fn attach(
        _child: &std::process::Child,
        _memory_limit_mb: Option<u64>,
        _cpu_percent: u32,
    ) -> Option<Self> {
        None
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: das Handle stammt aus CreateJobObjectW und wird genau einmal geschlossen.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

/// Nur eigene Kind-Handles und deren Job verwenden, niemals eine neu vergebene PID.
pub fn kill_tree(child: &mut std::process::Child, guard: Option<ProcessGuard>) {
    drop(guard);
    if !matches!(child.try_wait(), Ok(Some(_))) {
        if let Err(e) = child.kill() {
            log::warn!("process guard: could not stop child: {e}");
        }
    }
    if let Err(e) = child.wait() {
        log::warn!("process guard: could not reap child: {e}");
    }
}

/// Win32-Handle mit eindeutigem Besitz (auch auf Fehlerwegen).
#[cfg(windows)]
pub(crate) struct OwnedHandle(pub(crate) windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: nur erfolgreich geoeffnete Handles, genau ein Besitzer.
        unsafe { let _ = windows::Win32::Foundation::CloseHandle(self.0); }
    }
}

/// Oeffnet einen Pfad oder eine feste Einstellungs-URI ueber die Shell-API.
#[cfg(windows)]
pub fn shell_open(target: &std::ffi::OsStr) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let wide: Vec<u16> = target.encode_wide().chain(Some(0)).collect();
    if wide[..wide.len() - 1].contains(&0) {
        return Err("Oeffnen: ungueltiges Ziel".into());
    }
    // SAFETY: nullterminierter Pfad, keine Parameter oder Befehlszeile.
    let result = unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), None, None, SW_SHOWNORMAL)
    };
    if result.0 as isize <= 32 {
        return Err(format!("Oeffnen fehlgeschlagen ({})", result.0 as isize));
    }
    Ok(())
}

/// Nur IPv4-Loopback-Listener wie beim bisherigen TTS-Portabgleich.
#[cfg(windows)]
pub fn listening_pid(port: u16) -> Result<Option<u32>, String> {
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows::Win32::Networking::WinSock::AF_INET;
    let mut size = 0u32;
    // SAFETY: erste Abfrage liefert die benoetigte Puffergroesse.
    let mut status = unsafe {
        GetExtendedTcpTable(None, &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)
    };
    for _ in 0..8 {
        if status != ERROR_INSUFFICIENT_BUFFER.0 {
            return Err(format!("TCP-Tabelle nicht lesbar ({status})"));
        }
        // DWORD-Ausrichtung; die Tabelle kann zwischen beiden Aufrufen wachsen.
        let mut buf = vec![0u32; (size as usize).div_ceil(4).max(1)];
        let capacity = buf.len() * 4;
        status = unsafe {
            GetExtendedTcpTable(Some(buf.as_mut_ptr().cast()), &mut size, false,
                AF_INET.0 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)
        };
        if status == NO_ERROR.0 {
            let count = buf[0] as usize;
            let row_size = std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
            if size as usize > capacity || size < 4 || count > (size as usize - 4) / row_size {
                return Err("TCP-Tabelle ungueltig".into());
            }
            for n in 0..count {
                // SAFETY: Anzahl und Puffergroesse oben geprueft; DWORD-ausgerichtet.
                let row = unsafe { &*(buf.as_ptr().add(1).cast::<MIB_TCPROW_OWNER_PID>().add(n)) };
                if let Some(pid) = tcp_listener_pid(row, port) {
                    return Ok(Some(pid));
                }
            }
            return Ok(None);
        }
    }
    Err("TCP-Tabelle aendert sich zu schnell".into())
}

#[cfg(windows)]
fn tcp_listener_pid(
    row: &windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW_OWNER_PID,
    port: u16,
) -> Option<u32> {
    use windows::Win32::NetworkManagement::IpHelper::MIB_TCP_STATE_LISTEN;
    (row.dwState == MIB_TCP_STATE_LISTEN.0 as u32
        && row.dwLocalAddr.to_ne_bytes() == [127, 0, 0, 1]
        && u16::from_be(row.dwLocalPort as u16) == port)
        .then_some(row.dwOwningPid)
}

/// Historischer TTS-Sonderweg: einen bestaetigten Listener samt Baum beenden.
/// Handles halten die Identitaet fest; Zeitstempel schliessen wiedervergebene
/// Eltern-PIDs aus. Bereits existierende Kinder muessen einzeln in den Job.
#[cfg(windows)]
pub fn kill_listener(pid: u32, port: u16) -> Result<(), String> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{ERROR_INVALID_PARAMETER, FILETIME, WAIT_OBJECT_0};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::JobObjects::{AssignProcessToJobObject, IsProcessInJob, TerminateJobObject};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, TerminateProcess, WaitForSingleObject,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };
    fn opened(pid: u32) -> Result<Option<(OwnedHandle, u64)>, String> {
        unsafe {
            let h = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA
                | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) {
                Ok(h) => OwnedHandle(h),
                Err(e) if pid != 0 && e.code() == ERROR_INVALID_PARAMETER.to_hresult() => return Ok(None),
                Err(e) => return Err(format!("Prozess nicht oeffenbar: {e}")),
            };
            let (mut created, mut exit, mut kernel, mut user) =
                (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            GetProcessTimes(h.0, &mut created, &mut exit, &mut kernel, &mut user)
                .map_err(|e| format!("Prozessidentitaet nicht lesbar: {e}"))?;
            Ok(Some((h, (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))))
        }
    }
    fn rows() -> Result<Vec<(u32, u32)>, String> {
        let snap = OwnedHandle(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|e| format!("Prozessliste nicht lesbar: {e}"))?);
        let mut rows = Vec::new();
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        unsafe {
            Process32FirstW(snap.0, &mut entry).map_err(|e| format!("Prozessliste leer: {e}"))?;
            loop {
                rows.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if let Err(e) = Process32NextW(snap.0, &mut entry) {
                    if e.code() != windows::Win32::Foundation::ERROR_NO_MORE_FILES.to_hresult() {
                        return Err(format!("Prozessliste unvollstaendig: {e}"));
                    }
                    break;
                }
            }
        }
        Ok(rows)
    }
    let (root, created) = opened(pid)?.ok_or("Port-Besitzer bereits beendet")?;
    if listening_pid(port)? != Some(pid) {
        return Err("Port-Besitzer hat sich geaendert".into());
    }
    let job = ProcessGuard::attach_handle(root.0, None, 100)
        .ok_or("Prozessbaum nicht einem Job zuordenbar")?;
    let mut errors = drain_listener_tree((pid, root, created), rows, opened, |h| {
        let mut in_job = BOOL::default();
        unsafe { IsProcessInJob(h.0, Some(job.job), &mut in_job) }
            .map_err(|e| format!("Kindprozess-Job nicht lesbar: {e}"))?;
        if !in_job.as_bool() {
            unsafe { AssignProcessToJobObject(job.job, h.0) }
                .map_err(|e| format!("Kindprozess nicht einem Job zuordenbar: {e}"))?;
        }
        Ok(())
    }, |h| {
        // Auch nicht zuordenbare Kinder mit gehaltenem Handle einzeln beenden.
        if unsafe { WaitForSingleObject(h.0, 0) } == WAIT_OBJECT_0 { return Ok(()); }
        unsafe { TerminateProcess(h.0, 1) }.or_else(|e| {
            if unsafe { WaitForSingleObject(h.0, 0) } == WAIT_OBJECT_0 { Ok(()) } else { Err(e) }
        }).map_err(|e| format!("Prozess nicht beendet: {e}"))
    });
    if let Err(e) = unsafe { TerminateJobObject(job.job, 1) } {
        errors.push(format!("Prozessbaum nicht beendet: {e}"));
    }
    errors.sort();
    errors.dedup();
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

/// Teilfehler sammeln; kein bereits erfasstes Kind verhindert das Aufraeumen
/// der uebrigen Kinder. Handles bleiben bis zum letzten Beenden gehalten.
#[cfg(any(windows, test))]
fn drain_listener_tree<H>(
    root: (u32, H, u64),
    mut rows: impl FnMut() -> Result<Vec<(u32, u32)>, String>,
    mut opened: impl FnMut(u32) -> Result<Option<(H, u64)>, String>,
    mut attach: impl FnMut(&H) -> Result<(), String>,
    mut stop: impl FnMut(&H) -> Result<(), String>,
) -> Vec<String> {
    let mut held = vec![root];
    let mut errors = Vec::new();
    // Neue Kinder bereits zugeordneter Prozesse erben den Job. Mehrere Runden
    // erfassen auch Kinder, die vor der Zuordnung ihres Elternteils entstanden.
    for round in 0..8 {
        let mut added = false;
        let snapshot = match rows() {
            Ok(rows) => rows,
            Err(e) => { errors.push(e); break; }
        };
        for (child_pid, parent_pid) in snapshot {
            if held.iter().any(|(p, _, _)| *p == child_pid) { continue; }
            let Some((_, _, parent_time)) = held.iter().find(|(p, _, _)| *p == parent_pid) else { continue; };
            let (h, time) = match opened(child_pid) {
                Ok(Some(child)) => child,
                Ok(None) => continue, // Seit der Momentaufnahme beendet.
                Err(e) => { errors.push(e); continue; }
            };
            if time < *parent_time { continue; }
            // Noch einmal mit gehaltenem Handle pruefen: zwischen Snapshot und
            // OpenProcess koennte die Kind-PID schon neu vergeben worden sein.
            match rows() {
                Ok(rows) if rows.contains(&(child_pid, parent_pid)) => {}
                Ok(_) => continue,
                Err(e) => { errors.push(e); continue; }
            }
            if let Err(e) = attach(&h) { errors.push(e); }
            held.push((child_pid, h, time));
            added = true;
        }
        if !added {
            break;
        }
        if round == 7 { errors.push("Prozessbaum aendert sich zu schnell".into()); }
    }
    for (_, h, _) in held.iter().rev() {
        if let Err(e) = stop(h) { errors.push(e); }
    }
    errors
}

/// Prozessstart-Flag `CREATE_SUSPENDED`: der Hauptthread läuft erst nach
/// [`resume_suspended`]. So lässt sich ein Prozess einem Job-Objekt
/// zuordnen, bevor er Kinder starten oder sich beenden kann.
#[cfg(windows)]
pub const CREATE_SUSPENDED: u32 = 0x0000_0004;

/// Setzt die Threads eines mit [`CREATE_SUSPENDED`] gestarteten Prozesses
/// fort (bei diesem Start genau der Hauptthread). `std::process::Child`
/// gibt das Thread-Handle nicht heraus, darum über eine Thread-Momentaufnahme.
/// Fehler, wenn kein Thread fortgesetzt werden konnte.
#[cfg(windows)]
pub fn resume_suspended(child: &std::process::Child) -> Result<(), String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    let pid = child.id();
    // SAFETY: Momentaufnahme und Thread-Handles werden hier geöffnet und
    // genau einmal geschlossen; THREADENTRY32 ist mit dwSize initialisiert.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
            .map_err(|e| format!("Thread-Momentaufnahme: {e}"))?;
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut resumed = 0usize;
        let mut ok = Thread32First(snap, &mut entry).is_ok();
        while ok {
            if entry.th32OwnerProcessID == pid {
                if let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) {
                    let previous = ResumeThread(thread);
                    let _ = CloseHandle(thread);
                    // 0: lief bereits; > 1: bleibt angehalten. Beides ist kein
                    // mit CREATE_SUSPENDED frisch gestarteter Hauptthread.
                    if previous == 1 {
                        resumed += 1;
                    } else {
                        let _ = CloseHandle(snap);
                        return Err("Prozess nicht genau einmal angehalten gestartet".into());
                    }
                }
            }
            ok = Thread32Next(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        if resumed == 0 {
            return Err(format!("kein Thread von Prozess {pid} fortgesetzt"));
        }
    }
    Ok(())
}

/// Wächter: stoppt die eigenen Server, sobald der freie RAM unter die
/// Notgrenze faellt. `stop_all` wird einmal je Notfall gerufen; danach erst
/// wieder, wenn der Speicher sich erholt hat.
pub fn spawn_memory_watchdog(stop_all: impl Fn(u64) + Send + 'static) {
    std::thread::Builder::new()
        .name("memory-watchdog".into())
        .spawn(move || {
            let mut tripped = false;
            loop {
                std::thread::sleep(WATCH_INTERVAL);
                let free = available_ram_mb();
                if free == 0 {
                    continue;
                }
                if free < RAM_EMERGENCY_MB {
                    if !tripped {
                        tripped = true;
                        log::error!(
                            "memory watchdog: only {free} MB free — stopping own servers to keep the system alive"
                        );
                        stop_all(free);
                    }
                } else if tripped && free > RAM_EMERGENCY_MB * 2 {
                    tripped = false;
                    log::info!("memory watchdog: {free} MB free again");
                }
            }
        })
        .expect("memory watchdog thread");
}

/// Kann diese Datei als Programm gestartet werden, ohne dass Windows einen Systemdialog zeigt?
///
/// Ein `.exe` ohne gültigen PE-Kopf (Platzhalter, abgebrochener Download, beschädigte Datei)
/// würde Windows als 16-Bit-Anwendung deuten und den modalen Dialog „Nicht unterstützte
/// 16 Bit-Anwendung“ öffnen – mitten in einer Synthese oder einem Testlauf. Deshalb wird
/// vor jedem Start eigener Hilfsprogramme der Kopf geprüft. Nur `.exe` auf Windows; Skripte
/// (`.cmd`, `.bat`) und andere Systeme bleiben unberührt (dort scheitert ein falsches Format
/// still mit einem Fehlercode).
pub fn check_launchable(path: &std::path::Path) -> Result<(), String> {
    suppress_error_dialogs();
    let is_exe = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    if !(cfg!(windows) && is_exe) {
        return Ok(());
    }
    let mut head = [0u8; 4096];
    let n = {
        use std::io::Read;
        let mut f = std::fs::File::open(path)
            .map_err(|e| format!("{} lässt sich nicht öffnen: {e}", path.display()))?;
        let mut filled = 0;
        while filled < head.len() {
            match f.read(&mut head[filled..]) {
                Ok(0) => break,
                Ok(k) => filled += k,
                Err(e) => return Err(format!("{} ist nicht lesbar: {e}", path.display())),
            }
        }
        filled
    };
    if looks_like_pe(&head[..n]) {
        Ok(())
    } else {
        Err(format!(
            "{} ist kein gültiges Windows-Programm (beschädigt oder unvollständig); bitte neu einrichten.",
            path.display()
        ))
    }
}

/// PE-Kopf: `MZ`, Verweis auf `PE\0\0` innerhalb der ersten 4 KiB, bekannte Maschine
/// (x64, ARM64, x86).
pub(crate) fn looks_like_pe(head: &[u8]) -> bool {
    if head.len() < 0x40 || &head[..2] != b"MZ" {
        return false;
    }
    let e_lfanew = u32::from_le_bytes([head[0x3C], head[0x3D], head[0x3E], head[0x3F]]) as usize;
    let Some(end) = e_lfanew.checked_add(6) else {
        return false;
    };
    if e_lfanew < 0x40 || end > head.len() || &head[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        return false;
    }
    let machine = u16::from_le_bytes([head[e_lfanew + 4], head[e_lfanew + 5]]);
    matches!(machine, 0x8664 | 0xAA64 | 0x014C)
}

/// Windows: Fehlerdialoge des Systems für diesen Prozess abschalten (kritische Fehler,
/// Absturzdialog). Kindprozesse erben den Modus; Fehler kommen dann als Rückgabewert statt
/// als modales Fenster. Einmal je Prozess.
pub fn suppress_error_dialogs() {
    #[cfg(windows)]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            use windows::Win32::System::Diagnostics::Debug::{
                SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
            };
            // SAFETY: setzt nur ein Prozess-Flag, keine Zeiger.
            unsafe {
                SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanished_listener_children_do_not_hide_living_siblings() {
        let mut stopped = Vec::new();
        let errors = drain_listener_tree((1, 1, 10),
            || Ok(vec![(2, 1), (3, 1), (4, 3), (5, 1)]),
            |pid| Ok(if pid == 2 { None } else { Some((pid, if pid == 5 { 9 } else { 11 })) }),
            |_| Ok(()), |h| { stopped.push(*h); Ok(()) });
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(stopped, vec![4, 3, 1]);
        // Gegenprobe: alte/recycelte PID und verschwundenes Kind nie beenden.
        assert!(!stopped.contains(&2) && !stopped.contains(&5));
    }

    #[test]
    fn listener_partial_failures_still_visit_descendants_and_stop_every_handle() {
        let mut stopped = Vec::new();
        let errors = drain_listener_tree((1, 1, 10),
            || Ok(vec![(2, 1), (3, 1), (4, 3), (5, 1)]),
            |pid| if pid == 2 { Err("open".into()) } else { Ok(Some((pid, 11))) },
            |h| if *h == 3 { Err("attach".into()) } else { Ok(()) },
            |h| { stopped.push(*h); if *h == 4 { Err("stop".into()) } else { Ok(()) } });
        assert_eq!(stopped, vec![5, 4, 3, 1]);
        for e in ["open", "attach", "stop"] { assert!(errors.iter().any(|s| s == e)); }
        // Ein Teilfehler darf weder still gelingen noch das letzte Kind auslassen.
        assert!(!errors.is_empty());
        assert!(stopped.contains(&5));
    }

    #[test]
    fn broken_listener_snapshot_still_stops_the_held_root() {
        let mut stopped = Vec::new();
        let errors = drain_listener_tree((1, 1, 10), || Err("snapshot".into()),
            |_| panic!("kein Snapshot"), |_| Ok(()), |h| { stopped.push(*h); Ok(()) });
        assert_eq!(errors, vec!["snapshot"]);
        assert_eq!(stopped, vec![1]);
    }

    #[test]
    fn an_unstable_listener_tree_reports_the_limit_after_cleanup() {
        let mut generation = 1;
        let mut stopped = Vec::new();
        let errors = drain_listener_tree((1, 1, 10), || {
            generation += 1;
            // Je neuer Momentaufnahme ein Kind mehr, als bislang erfasst.
            Ok((2..=generation).map(|pid| (pid, 1)).collect())
        }, |pid| Ok(Some((pid, 11))), |_| Ok(()), |h| { stopped.push(*h); Ok(()) });
        assert!(errors.iter().any(|e| e.contains("aendert sich zu schnell")));
        assert_eq!(stopped.last(), Some(&1));
        assert!(stopped.len() > 8, "auch am Rundenlimit alle gehaltenen Kinder beenden");
    }

    #[cfg(windows)]
    #[test]
    fn attach_suspended_rejects_a_running_start_and_reaps_it() {
        use std::os::windows::process::CommandExt;
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("running");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process_guard::tests::e3_tree_fixture", "--nocapture"])
            .env("LVA_E3_TREE_FIXTURE", &marker).env("LVA_E3_GRANDCHILD", "1")
            .creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .spawn().unwrap();
        let running = wait_for(Duration::from_secs(30), || marker.exists());
        if !running { kill_tree(&mut child, None); panic!("Fixture nicht angelaufen"); }
        let result = ProcessGuard::attach_suspended(&mut child, None, 100);
        if let Ok(guard) = result { kill_tree(&mut child, guard); panic!("laufender Start akzeptiert"); }
        assert!(child.try_wait().unwrap().is_some());
        // Gegenprobe mit CREATE_SUSPENDED: suspended_start_captures_the_first_grandchild_even_in_a_parent_job.
    }

    #[cfg(windows)]
    fn fixture_marker(path: &std::path::Path, value: &str) {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, value).unwrap();
        std::fs::rename(tmp, path).unwrap();
    }

    /// Nur der eigens gestartete Testprozess liest diese synthetische Fixture.
    #[cfg(windows)]
    #[test]
    fn e3_tree_fixture() {
        let Some(path) = std::env::var_os("LVA_E3_TREE_FIXTURE") else { return; };
        let _listener = if std::env::var_os("LVA_E3_GRANDCHILD").is_none() {
            std::env::var_os("LVA_E3_LISTENER").map(|port_file| {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                fixture_marker(std::path::Path::new(&port_file), &listener.local_addr().unwrap().port().to_string());
                listener
            })
        } else { None };
        if std::env::var_os("LVA_E3_GRANDCHILD").is_some() {
            fixture_marker(std::path::Path::new(&path), &std::process::id().to_string());
        } else {
            let _child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "process_guard::tests::e3_tree_fixture", "--nocapture"])
                .env("LVA_E3_GRANDCHILD", "1")
                .spawn().unwrap();
        }
        std::thread::sleep(Duration::from_secs(120));
    }

    #[cfg(windows)]
    #[test]
    fn suspended_start_captures_the_first_grandchild_even_in_a_parent_job() {
        use std::os::windows::{io::AsRawHandle, process::CommandExt};
        use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("grandchild");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process_guard::tests::e3_tree_fixture", "--nocapture"])
            .env("LVA_E3_TREE_FIXTURE", &marker)
            .creation_flags(0x0800_0000 | CREATE_SUSPENDED)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .spawn().unwrap();
        // Der zweite Job verschachtelt sich im ersten (wie eine verwaltete App).
        let outer = ProcessGuard::attach(&child, None, 100).unwrap();
        assert!(!marker.exists(), "vor der Zuordnung laeuft kein Kindcode");
        let guard = ProcessGuard::attach_suspended(&mut child, None, 100).unwrap().unwrap();
        assert!(wait_for(Duration::from_secs(30), || marker.exists()));
        let pid: u32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
        let grandchild = OwnedHandle(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.unwrap());
        assert_eq!(unsafe { WaitForSingleObject(grandchild.0, 0) }, WAIT_TIMEOUT,
            "Gegenprobe: der Enkel lebt vor dem Schliessen");
        kill_tree(&mut child, Some(guard));
        assert_eq!(unsafe { WaitForSingleObject(grandchild.0, 30_000) }, WAIT_OBJECT_0);
        assert_eq!(unsafe { WaitForSingleObject(HANDLE(child.as_raw_handle().cast()), 0) }, WAIT_OBJECT_0);
        drop(outer);
    }

    #[cfg(windows)]
    #[test]
    fn tcp_table_finds_only_the_exact_loopback_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_eq!(listening_pid(port).unwrap(), Some(std::process::id()));
        drop(listener);
        assert_eq!(listening_pid(port).unwrap(), None);
    }

    #[cfg(windows)]
    #[test]
    fn tcp_rows_exclude_wildcard_connections_and_other_ports() {
        use windows::Win32::NetworkManagement::IpHelper::{MIB_TCPROW_OWNER_PID, MIB_TCP_STATE_LISTEN};
        let mut row = MIB_TCPROW_OWNER_PID {
            dwState: MIB_TCP_STATE_LISTEN.0 as u32,
            dwLocalAddr: u32::from_ne_bytes([127, 0, 0, 1]),
            dwLocalPort: 8080u16.to_be() as u32,
            dwOwningPid: 42,
            ..Default::default()
        };
        assert_eq!(tcp_listener_pid(&row, 8080), Some(42));
        assert_eq!(tcp_listener_pid(&row, 8081), None);
        row.dwLocalAddr = 0;
        assert_eq!(tcp_listener_pid(&row, 8080), None);
        row.dwLocalAddr = u32::from_ne_bytes([127, 0, 0, 1]);
        row.dwState = 5; // ESTABLISHED statt LISTEN
        assert_eq!(tcp_listener_pid(&row, 8080), None);
    }

    #[cfg(windows)]
    #[test]
    fn changed_listener_is_never_killed() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        // Eigene PID, aber ein anderer Port: muss vor jeder Job-Zuordnung abbrechen.
        let port = listener.local_addr().unwrap().port();
        assert!(kill_listener(std::process::id(), 0).is_err());
        assert_eq!(listening_pid(port).unwrap(), Some(std::process::id()));
    }

    #[cfg(windows)]
    #[test]
    fn existing_listener_tree_is_killed_without_system_tools() {
        use std::os::windows::process::CommandExt;
        use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("grandchild");
        let port_file = dir.path().join("port");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process_guard::tests::e3_tree_fixture", "--nocapture"])
            .env("LVA_E3_TREE_FIXTURE", &marker).env("LVA_E3_LISTENER", &port_file)
            .creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .spawn().unwrap();
        assert!(wait_for(Duration::from_secs(30), || marker.exists() && port_file.exists()));
        let grandchild: u32 = std::fs::read_to_string(marker).unwrap().parse().unwrap();
        let port: u16 = std::fs::read_to_string(port_file).unwrap().parse().unwrap();
        let handle = OwnedHandle(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, grandchild) }.unwrap());
        assert_eq!(unsafe { WaitForSingleObject(handle.0, 0) }, WAIT_TIMEOUT);
        assert_eq!(listening_pid(port).unwrap(), Some(child.id()));
        kill_listener(child.id(), port).unwrap();
        assert_eq!(unsafe { WaitForSingleObject(handle.0, 30_000) }, WAIT_OBJECT_0);
        assert!(!child.wait().unwrap().success());
    }

    #[cfg(windows)]
    #[test]
    fn shell_rejects_embedded_nul_without_opening_anything() {
        assert!(shell_open(std::ffi::OsStr::new("synthetisch\0suffix")).is_err());
    }

    fn pe(machine: u16) -> Vec<u8> {
        let mut v = vec![0u8; 0x200];
        v[..2].copy_from_slice(b"MZ");
        v[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        v[0x80..0x84].copy_from_slice(b"PE\0\0");
        v[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        v
    }

    #[test]
    fn nur_echte_pe_programme_gelten_als_startbar() {
        assert!(looks_like_pe(&pe(0x8664)));
        assert!(looks_like_pe(&pe(0xAA64)));
        assert!(!looks_like_pe(b"fixture"), "Platzhalter aus Tests");
        assert!(!looks_like_pe(b"MZ"), "nur Signatur");
        let mut dos = vec![0x90u8; 300_000];
        dos[..2].copy_from_slice(b"MZ");
        assert!(!looks_like_pe(&dos), "DOS-Kopf ohne PE");
        assert!(!looks_like_pe(&pe(0x0200)), "Itanium/unbekannt");
        let mut far = pe(0x8664);
        far[0x3C..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!looks_like_pe(&far), "Verweis hinter dem Kopf");
    }

    #[test]
    fn platzhalter_exe_wird_vor_dem_start_abgelehnt() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("piper.exe");
        std::fs::write(&fake, b"fixture").unwrap();
        let script = dir.path().join("fake.cmd");
        std::fs::write(&script, b"@echo off").unwrap();
        assert!(
            check_launchable(&script).is_ok(),
            "Skripte bleiben startbar"
        );
        if cfg!(windows) {
            assert!(check_launchable(&fake)
                .unwrap_err()
                .contains("kein gültiges"));
            let real = dir.path().join("echt.exe");
            std::fs::write(&real, pe(0x8664)).unwrap();
            assert!(check_launchable(&real).is_ok());
        } else {
            assert!(
                check_launchable(&fake).is_ok(),
                "nur Windows prüft den PE-Kopf"
            );
        }
    }

    #[test]
    #[cfg(not(windows))]
    fn compile_threads_never_scale_with_cores() {
        assert_eq!(compile_threads_for(32), 4, "32 Kerne waren der Ausloeser");
        assert_eq!(compile_threads_for(8), 2);
        assert_eq!(compile_threads_for(4), 2);
        assert_eq!(compile_threads_for(64), 4);
    }

    #[test]
    #[cfg(windows)]
    fn compile_threads_are_one_on_windows() {
        // > 1 startet einen SubprocPool mit pass_fds — auf Windows ein Absturz.
        for cpus in [4, 8, 32, 64] {
            assert_eq!(compile_threads_for(cpus), 1);
        }
    }

    #[test]
    fn cpu_threads_leave_half_the_machine() {
        assert_eq!(cpu_threads_for(32), 16);
        assert_eq!(cpu_threads_for(4), 4);
        assert_eq!(cpu_threads_for(64), 16);
    }

    #[test]
    fn memory_limit_keeps_the_reserve() {
        assert_eq!(memory_limit_mb(44 * 1024), 38 * 1024);
        assert_eq!(memory_limit_mb(5 * 1024), RAM_MIN_LIMIT_MB);
        assert_eq!(memory_limit_mb(0), RAM_MIN_LIMIT_MB);
    }

    /// Praxisbeweis fuer den Deckel: ein Python-Kind will unter einem 1-GB-
    /// Job 3 GB anfordern und muss daran scheitern (MemoryError, Exit != 0),
    /// statt den Rechner in die Auslagerung zu treiben. Braucht Windows und
    /// ein Python auf dem PATH: `cargo test job_limit -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn job_limit_stops_a_runaway_child() {
        let child = std::process::Command::new("python")
            .args([
                "-c",
                "b = bytearray(3 * 1024 * 1024 * 1024); print('ALLOCATED', len(b))",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("python auf dem PATH");
        let guard = ProcessGuard::attach(&child, Some(1024), 50).expect("Job-Objekt");
        let out = child.wait_with_output().expect("wait");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        eprintln!(
            "exit={:?} stdout={stdout} stderr={}",
            out.status.code(),
            stderr.lines().last().unwrap_or("")
        );
        drop(guard);
        assert!(
            !stdout.contains("ALLOCATED"),
            "3 GB unter 1-GB-Deckel duerfen nicht gelingen"
        );
        assert!(!out.status.success());
    }

    /// Wartet auf ein Ereignis, hoechstens `limit`. Die Frist ist nur die Grenze,
    /// ab der etwas als haengend gilt, nicht Teil der Aussage der Tests: Kinder
    /// laufen im Job mit BELOW_NORMAL-Prioritaet und verhungern auf einem
    /// ausgelasteten Rechner (parallele Builds) sekundenlang; gemessen brauchte
    /// ein `cmd /c echo` 11 bis 21 s statt 0,1 s.
    #[cfg(windows)]
    fn wait_for(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + limit;
        loop {
            if cond() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Beweis fuer "kein Kindprozess ueberlebt die App": wird das Job-Objekt
    /// geschlossen (Drop, App-Ende, Absturz), sterben Kind UND Enkel. Das
    /// Kind ist `cmd`, der Enkel ein `ping` -- wie llama-server oder
    /// Fish-Speech mit ihren Worker-Prozessen.
    #[test]
    #[cfg(windows)]
    fn closing_the_job_kills_child_and_grandchild() {
        use crate::managers::llm::app_usage::{process_rows, tree_pids};
        // Der Enkel lebt ~5 Minuten, laenger als jede Frist unten: nur das
        // Schliessen des Job-Objekts kann ihn rechtzeitig beenden.
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 300 127.0.0.1 >nul"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("cmd");
        let guard = ProcessGuard::attach(&child, None, 50).expect("Job-Objekt");
        // Warten, bis cmd den Enkel gestartet hat.
        let mut grandchildren = Vec::new();
        let started = wait_for(Duration::from_secs(90), || {
            let pids = tree_pids(child.id(), &process_rows());
            grandchildren = pids.into_iter().filter(|p| *p != child.id()).collect();
            !grandchildren.is_empty()
        });
        assert!(started, "cmd hat keinen Enkel gestartet");
        assert!(
            child.try_wait().unwrap().is_none(),
            "Kind lebt vor dem Schliessen"
        );

        drop(guard); // KILL_ON_JOB_CLOSE

        let dead = wait_for(Duration::from_secs(90), || {
            child.try_wait().unwrap().is_some()
        });
        assert!(dead, "das Kind lebt nach dem Schliessen des Job-Objekts");
        let mut alive: Vec<u32> = Vec::new();
        let gone = wait_for(Duration::from_secs(90), || {
            alive = process_rows()
                .iter()
                .map(|r| r.pid)
                .filter(|p| grandchildren.contains(p))
                .collect();
            alive.is_empty()
        });
        assert!(gone, "Enkel leben noch: {alive:?}");
    }

    #[test]
    fn session_is_not_ending_during_a_test_run() {
        // Waehrend eines Testlaufs faehrt Windows nicht herunter. Meldete die
        // Abfrage hier "ja", uebersprangen Stopp und Beenden ihr Aufraeumen.
        assert!(!session_ending());
    }

    #[test]
    fn the_reserve_follows_the_size_of_the_machine() {
        assert_eq!(reserve_for_total_mb(8 * 1024), RAM_RESERVE_MIN_MB);
        // Zielgeraet 16 GB: Windows meldet etwas weniger als 16 384 MB.
        assert_eq!(reserve_for_total_mb(15_900), 3_180);
        assert_eq!(reserve_for_total_mb(32 * 1024), RAM_RESERVE_MB);
        assert_eq!(reserve_for_total_mb(64 * 1024), RAM_RESERVE_MB);
        // Nicht messbar: die volle Reserve, nicht die kleine.
        assert_eq!(reserve_for_total_mb(0), RAM_RESERVE_MB);
    }

    #[test]
    fn test_cap_only_ever_lowers_the_free_ram() {
        assert_eq!(capped_free_mb(20_000, None), 20_000);
        assert_eq!(capped_free_mb(20_000, Some(5_000)), 5_000);
        // Ein Deckel ueber dem Messwert oeffnet nichts.
        assert_eq!(capped_free_mb(3_000, Some(50_000)), 3_000);
        // Nicht messbar blockiert sonst nie; mit Deckel gilt der Deckel.
        assert_eq!(capped_free_mb(0, None), 0);
        assert_eq!(capped_free_mb(0, Some(1_500)), 1_500);
        // Deckel 0 darf nicht als "nicht messbar" alle Tore oeffnen.
        assert_eq!(capped_free_mb(20_000, Some(0)), 1);
    }

    #[test]
    fn start_gate_needs_need_plus_reserve() {
        // Nicht messbar (0) blockiert nie; das testet available_ram_mb nicht,
        // sondern die Regel selbst über memory_limit/Reserve.
        assert!(RAM_RESERVE_MIN_MB > RAM_EMERGENCY_MB);
        assert!(RAM_RESERVE_MIN_MB <= RAM_RESERVE_MB);
    }
}
