//! lva#806 P1b: eigener Prozess; Pipe-I/O darf den Steuerthread nie blockieren.

use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

pub use crate::managers::meetings::audiobook::AudiobookTempo;
use super::{protocol::{self, Reply, Request, WorkerError}, InferenceWorker};

/// Win32 Creation-Flags: IDLE bzw. BELOW_NORMAL schon vor dem ersten Thread.
pub fn priority_flags(tempo: AudiobookTempo) -> u32 {
    match tempo { AudiobookTempo::Gentle => 0x40, _ => 0x4000 }
}

struct ManagedChild {
    child: Option<Child>,
    #[cfg(windows)]
    _job: Job,
}

impl ManagedChild {
    fn spawn(command: &mut Command, tempo: AudiobookTempo) -> Result<Self, WorkerError> {
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(priority_flags(tempo) | 0x08000000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let _ = tempo;
            // SAFETY: nach fork nur async-signal-safe nice; keine Allokation,
            // Sperre oder App-Zustaende im Kind vor exec.
            unsafe {
                command.pre_exec(|| {
                    if libc::nice(10) == -1 { return Err(std::io::Error::last_os_error()); }
                    Ok(())
                });
            }
        }
        let child = command.spawn().map_err(|_| WorkerError::SpawnFailed)?;
        #[cfg(windows)]
        let job = match Job::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self { child: Some(child), #[cfg(windows)] _job: job })
    }

    fn terminate(&mut self) -> Result<(), WorkerError> {
        let Some(child) = self.child.as_mut() else { return Ok(()); };
        if child.try_wait().map_err(|_| WorkerError::Io)?.is_some() { return Ok(()); }
        child.kill().map_err(|_| WorkerError::Io)?;
        // Kein unbeschraenktes wait auf dem Parent-Steuerthread. Vier Kinder
        // teilen die 2-s-Frist; auch das anschliessende Reaping bleibt kurz.
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            if child.try_wait().map_err(|_| WorkerError::Io)?.is_some() { return Ok(()); }
            if Instant::now() >= deadline { return Err(WorkerError::Timeout); }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            // Nach SIGKILL/TerminateProcess nur noch aufraeumen; der Parent
            // wartet hier auch bei einem Betriebssystemfehler nicht mit.
            let _ = std::thread::Builder::new().name("audiobook-reap".into()).spawn(move || { let _ = child.wait(); });
        }
    }
}

#[cfg(windows)]
struct Job(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
// SAFETY: exklusiv besessenes Kernel-Handle ohne Thread-Bindung.
unsafe impl Send for Job {}

#[cfg(windows)]
impl Job {
    fn attach(child: &Child) -> Result<Self, WorkerError> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, System::JobObjects::*};
        // SAFETY: initialisierte Struktur; das exklusive Handle schliesst Drop.
        unsafe {
            let job = Self(CreateJobObjectW(None, None).map_err(|_| WorkerError::ContainmentFailed)?);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0, JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                std::mem::size_of_val(&limits) as u32,
            ).map_err(|_| WorkerError::ContainmentFailed)?;
            AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle() as *mut core::ffi::c_void))
                .map_err(|_| WorkerError::ContainmentFailed)?;
            Ok(job)
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: Handle stammt aus CreateJobObjectW, genau ein Besitzer.
        unsafe { let _ = windows::Win32::Foundation::CloseHandle(self.0); }
    }
}

pub struct ProcessWorker {
    process: ManagedChild,
    commands: Option<SyncSender<Request>>,
    replies: Option<Receiver<Result<Reply, WorkerError>>>,
}

impl ProcessWorker {
    /// Ein fehlender EXE-Modus, Ladefehler und Lade-Timeout sind unterscheidbar.
    /// Der spaetere Queue-Aufrufer entscheidet ueber den sichtbaren P1a-Rueckfall.
    pub fn load(load: Request, tempo: AudiobookTempo, timeout: Duration) -> Result<Self, WorkerError> {
        if !matches!(&load, Request::Load { .. }) { return Err(WorkerError::Invalid); }
        load.validate()?;
        let exe = std::env::current_exe().map_err(|_| WorkerError::SpawnFailed)?;
        let mut command = Command::new(exe);
        command.arg("--lva-audiobook-worker");
        let mut worker = Self::spawn(&mut command, tempo)?;
        worker.send(load)?;
        match worker.replies.as_ref().ok_or(WorkerError::Stopped)?.recv_timeout(timeout) {
            Ok(Ok(Reply::Loaded)) => Ok(worker),
            Ok(Ok(Reply::Error { code })) => Err(code),
            Ok(Err(error)) => Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(WorkerError::Timeout),
            _ => Err(WorkerError::Crashed),
        }
    }

    fn spawn(command: &mut Command, tempo: AudiobookTempo) -> Result<Self, WorkerError> {
        let mut process = ManagedChild::spawn(command, tempo)?;
        let child = process.child.as_mut().ok_or(WorkerError::SpawnFailed)?;
        let mut input = child.stdin.take().ok_or(WorkerError::Io)?;
        let mut output = child.stdout.take().ok_or(WorkerError::Io)?;
        let (tx, commands) = mpsc::sync_channel::<Request>(1);
        let (replies, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new().name("audiobook-pipe".into()).spawn(move || {
            while let Ok(request) = commands.recv() {
                let shutdown = matches!(request, Request::Shutdown);
                let result = protocol::write(&mut input, &request).and_then(|_| {
                    protocol::read(&mut output)?.ok_or(WorkerError::Crashed)
                });
                let failed = result.is_err();
                if replies.send(result).is_err() || failed || shutdown { break; }
            }
        }).map_err(|_| WorkerError::Io)?;
        Ok(Self { process, commands: Some(tx), replies: Some(rx) })
    }

    fn send(&self, request: Request) -> Result<(), WorkerError> {
        self.commands.as_ref().ok_or(WorkerError::Stopped)?.try_send(request)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => WorkerError::Busy,
                mpsc::TrySendError::Disconnected(_) => WorkerError::Crashed,
            })
    }
}

impl InferenceWorker for ProcessWorker {
    fn start(&mut self, generation: u64, samples: Vec<f32>) -> Result<(), WorkerError> {
        let request = Request::Infer { generation, samples };
        request.validate()?;
        self.send(request)
    }

    fn poll(&mut self) -> Result<Option<Reply>, WorkerError> {
        match self.replies.as_ref().ok_or(WorkerError::Stopped)?.try_recv() {
            Ok(result) => result.map(Some),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(WorkerError::Crashed),
        }
    }

    fn terminate(&mut self) -> Result<(), WorkerError> {
        self.commands.take();
        self.replies.take();
        self.process.terminate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_creation_flags_do_not_raise_the_app_priority() {
        assert_eq!(priority_flags(AudiobookTempo::Gentle), 0x40);
        assert_eq!(priority_flags(AudiobookTempo::Balanced), 0x4000);
        assert_eq!(priority_flags(AudiobookTempo::Fast), 0x4000);
        assert_ne!(priority_flags(AudiobookTempo::Gentle), priority_flags(AudiobookTempo::Fast));
    }

    #[test]
    #[ignore = "nur als eigener synthetischer Kindprozess gestartet"]
    fn hanging_child() {
        loop { std::thread::sleep(Duration::from_millis(100)); }
    }

    #[test]
    fn four_real_hanging_children_are_killed_in_under_five_seconds() {
        let mut children = Vec::new();
        for _ in 0..4 {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "audiobook_worker::process::tests::hanging_child", "--ignored"]);
            children.push(ManagedChild::spawn(&mut command, AudiobookTempo::Gentle).unwrap());
        }
        // Gegenprobe: ohne Kill laufen dieselben synthetischen Kinder weiter.
        for child in &mut children { assert!(child.child.as_mut().unwrap().try_wait().unwrap().is_none()); }
        let start = Instant::now();
        for child in &mut children { child.terminate().unwrap(); }
        assert!(start.elapsed() < Duration::from_secs(5));
        for child in &mut children { assert!(child.child.as_mut().unwrap().try_wait().unwrap().is_some()); }
    }
}
