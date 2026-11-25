//! Prozessschutz vor dem ersten Nutzdaten-Frame; keine App-Zustaende im Worker.

use super::protocol::WorkerError;

pub(super) fn initialize() -> Result<(), WorkerError> {
    suppress_dumps()?;
    #[cfg(unix)]
    watch_parent()?;
    Ok(())
}

fn suppress_dumps() -> Result<(), WorkerError> {
    #[cfg(unix)]
    {
        let limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        // SAFETY: gueltige Struktur; betrifft ausschliesslich diesen Prozess.
        if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } != 0 {
            return Err(WorkerError::ContainmentFailed);
        }
    }
    #[cfg(target_os = "linux")]
    // SAFETY: skalare prctl-Argumente; auch bei pipe-basiertem core_pattern.
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
        return Err(WorkerError::ContainmentFailed);
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::{Diagnostics::Debug::*, ErrorReporting::*};
        // SAFETY: nur prozesslokal, keine Registry-/systemweite WER-Ausnahme.
        unsafe {
            SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
            WerSetFlags(WER_FAULT_REPORTING_FLAG_NOHEAP)
                .map_err(|_| WorkerError::ContainmentFailed)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn watch_parent() -> Result<(), WorkerError> {
    let parent = std::env::var("LVA_AUDIOBOOK_PARENT_PID").ok()
        .and_then(|value| value.parse::<libc::pid_t>().ok())
        .filter(|pid| *pid > 1).ok_or(WorkerError::ContainmentFailed)?;
    #[cfg(target_os = "linux")]
    // SAFETY: SIGKILL braucht keinen Handler und erzeugt keinen Core-Dump.
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) } != 0 {
        return Err(WorkerError::ContainmentFailed);
    }
    // Race-Pruefung nach prctl, auch fuer Tod vor dem Worker-Start.
    if unsafe { libc::getppid() } != parent { return Err(WorkerError::ContainmentFailed); }
    std::thread::Builder::new().name("audiobook-parent".into()).spawn(move || {
        let mut pipe = libc::pollfd { fd: libc::STDIN_FILENO, events: 0, revents: 0 };
        loop {
            // Kein Read: der Protokollthread bleibt einziger Leser. HUP wird
            // auch bei ungelesenen Bytes erkannt, unabhaengig von der Inferenz.
            let result = unsafe { libc::poll(&mut pipe, 1, 50) };
            if unsafe { libc::getppid() } != parent
                || pipe.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
                || (result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted)
            {
                // SAFETY: sofort beenden, keine Destruktoren/nativen Locks.
                unsafe { libc::_exit(65); }
            }
        }
    }).map_err(|_| WorkerError::ContainmentFailed)?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use windows::Win32::System::{Diagnostics::Debug::*, ErrorReporting::*, Threading::GetCurrentProcess};

    #[test]
    #[ignore = "nur als eigener synthetischer Kindprozess gestartet"]
    fn synthetic_policy_child() {
        // Gegenprobe: Schutz absichtlich weglassen, Ausgangswerte vorher leeren.
        unsafe {
            SetErrorMode(THREAD_ERROR_MODE(0));
            WerSetFlags(WER_FAULT_REPORTING(0)).unwrap();
        }
        if std::env::var_os("LVA_SKIP_DUMP_POLICY").is_none() { initialize().unwrap(); }
        unsafe {
            assert_ne!(GetErrorMode() & SEM_NOGPFAULTERRORBOX.0, 0);
            assert_ne!(WerGetFlags(GetCurrentProcess()).unwrap().0 & WER_FAULT_REPORTING_FLAG_NOHEAP.0, 0);
        }
    }

    #[test]
    fn dump_policy_is_confined_to_the_worker_process() {
        use std::process::{Command, Stdio};
        let before = unsafe { (GetErrorMode(), WerGetFlags(GetCurrentProcess()).unwrap().0) };
        for skip in [false, true] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "audiobook_worker::safety::windows_tests::synthetic_policy_child", "--ignored"])
                .env_remove("LVA_SKIP_DUMP_POLICY").stdout(Stdio::null()).stderr(Stdio::null());
            if skip { command.env("LVA_SKIP_DUMP_POLICY", "1"); }
            let mut child = command.spawn().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() { break status; }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("synthetic policy child timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            assert_eq!(status.success(), !skip);
        }
        assert_eq!(unsafe { (GetErrorMode(), WerGetFlags(GetCurrentProcess()).unwrap().0) }, before);
    }
}
