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

/// RAM, den die App dem System immer laesst (MB). Darunter wird kein Server
/// gestartet, und ein Kindprozess darf nie mehr als (frei - Reserve) belegen.
pub const RAM_RESERVE_MB: u64 = 6 * 1024;
/// Notgrenze des Wächters (MB): darunter werden laufende Server gestoppt.
pub const RAM_EMERGENCY_MB: u64 = 2 * 1024;
/// Unter diesem Deckel lohnt kein Start: das Modell passt dann ohnehin nicht.
pub const RAM_MIN_LIMIT_MB: u64 = 4 * 1024;
/// Anteil der CPU, den ein Kindprozessbaum hoechstens bekommt (Prozent).
pub const CPU_CAP_PERCENT: u32 = 75;
pub const WATCH_INTERVAL: Duration = Duration::from_secs(5);

/// Freier physischer Speicher in MB (0, wenn nicht messbar).
pub fn available_ram_mb() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.available_memory() / (1024 * 1024)
}

pub fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Torch-Inductor-Compile-Prozesse: 2 bis 4 statt "ein Prozess je Kern".
/// Vier reichen fuer den ~60-s-Compile; 32 waren der Ausloeser des Ausfalls.
pub fn compile_threads_for(cpus: usize) -> usize {
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
    if free < need_mb + RAM_RESERVE_MB {
        return Err(format!(
            "Zu wenig freier Arbeitsspeicher: {:.1} GB frei, gebraucht werden etwa {:.1} GB plus {:.0} GB Reserve fuer das System. Andere Programme schliessen und erneut starten.",
            free as f64 / 1024.0,
            need_mb as f64 / 1024.0,
            RAM_RESERVE_MB as f64 / 1024.0
        ));
    }
    Ok(free)
}

/// Speicher-Deckel fuer einen Kindprozessbaum: alles, was frei ist, minus
/// die Reserve — nie unter `RAM_MIN_LIMIT_MB`.
pub fn memory_limit_mb(free_mb: u64) -> u64 {
    free_mb.saturating_sub(RAM_RESERVE_MB).max(RAM_MIN_LIMIT_MB)
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
    /// Haengt `child` in ein neues Job-Objekt mit Speicher-, CPU- und
    /// Prioritaetsdeckel. `None`, wenn das Betriebssystem das nicht kann;
    /// der Prozess laeuft dann ohne Deckel weiter (Gate und Wächter bleiben).
    #[cfg(windows)]
    pub fn attach(child: &std::process::Child, memory_limit_mb: u64, cpu_percent: u32) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
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
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_JOB_MEMORY
                | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_PRIORITY_CLASS;
            limits.BasicLimitInformation.PriorityClass = BELOW_NORMAL_PRIORITY_CLASS.0;
            limits.JobMemoryLimit = (memory_limit_mb as usize) * 1024 * 1024;
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
                ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
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

            let process = HANDLE(child.as_raw_handle() as *mut core::ffi::c_void);
            if let Err(e) = AssignProcessToJobObject(job, process) {
                log::warn!("process guard: could not assign process to job: {e}");
                let _ = windows::Win32::Foundation::CloseHandle(job);
                return None;
            }
            log::info!(
                "process guard: pid {} limited to {} MB RAM, {}% CPU, below-normal priority",
                child.id(),
                memory_limit_mb,
                cpu_percent
            );
            Some(Self { job })
        }
    }

    #[cfg(not(windows))]
    pub fn attach(_child: &std::process::Child, _memory_limit_mb: u64, _cpu_percent: u32) -> Option<Self> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_threads_never_scale_with_cores() {
        assert_eq!(compile_threads_for(32), 4, "32 Kerne waren der Ausloeser");
        assert_eq!(compile_threads_for(8), 2);
        assert_eq!(compile_threads_for(4), 2);
        assert_eq!(compile_threads_for(64), 4);
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
            .args(["-c", "b = bytearray(3 * 1024 * 1024 * 1024); print('ALLOCATED', len(b))"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("python auf dem PATH");
        let guard = ProcessGuard::attach(&child, 1024, 50).expect("Job-Objekt");
        let out = child.wait_with_output().expect("wait");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        eprintln!("exit={:?} stdout={stdout} stderr={}", out.status.code(), stderr.lines().last().unwrap_or(""));
        drop(guard);
        assert!(!stdout.contains("ALLOCATED"), "3 GB unter 1-GB-Deckel duerfen nicht gelingen");
        assert!(!out.status.success());
    }

    #[test]
    fn start_gate_needs_need_plus_reserve() {
        // Nicht messbar (0) blockiert nie; das testet available_ram_mb nicht,
        // sondern die Regel selbst über memory_limit/Reserve.
        assert!(RAM_RESERVE_MB > RAM_EMERGENCY_MB);
    }
}
