use super::*;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const FIXTURE: &str = "audiobook_worker::safety::tests::synthetic_child";

fn command(path: &Path, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", FIXTURE, "--ignored"])
        .current_dir(path.parent().unwrap())
        .env("LVA_SAFETY_READY", path).env("LVA_SAFETY_MODE", mode)
        .env("LVA_AUDIOBOOK_PARENT_PID", std::process::id().to_string())
        .stdout(Stdio::null()).stderr(Stdio::null());
    command
}

#[test]
#[ignore = "nur als eigener synthetischer Kindprozess gestartet"]
fn synthetic_child() {
    let path = PathBuf::from(std::env::var_os("LVA_SAFETY_READY").unwrap());
    let mode = std::env::var("LVA_SAFETY_MODE").unwrap();
    if mode.starts_with("parent-") {
        // stdin bleibt beim Enkel offen, auch wenn dieser Parent stirbt.
        let child_mode = mode.strip_prefix("parent-").unwrap();
        let mut child = command(&path, child_mode).stdin(Stdio::inherit()).spawn().unwrap();
        let _ = child.wait();
        return;
    }
    if mode == "crash" {
        // Nur vor Nutzdaten: Gegenprobe soll nicht von schon geerbten Limits
        // verdeckt werden. Die Policy-Assertions stoppen Mutanten vor SIGABRT.
        let mut baseline = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut baseline) }, 0);
        if baseline.rlim_max > 0 {
            baseline.rlim_cur = baseline.rlim_max;
            assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_CORE, &baseline) }, 0);
        }
        #[cfg(target_os = "linux")]
        assert_eq!(unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 1, 0, 0, 0) }, 0);
    }
    if mode != "unprotected" { initialize().unwrap(); }
    #[cfg(target_os = "linux")]
    if mode == "watcher-only" {
        // Gegenprobe zum Kernel-Signal: auch der unabhaengige Thread muss wirken.
        assert_eq!(unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, 0, 0, 0, 0) }, 0);
    }
    if mode == "crash" {
        let mut limit = libc::rlimit { rlim_cur: 1, rlim_max: 1 };
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) }, 0);
        assert_eq!((limit.rlim_cur, limit.rlim_max), (0, 0));
        #[cfg(target_os = "linux")]
        assert_eq!(unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) }, 0);
        let marker = std::hint::black_box(vec![b"synthetic-private-marker".to_vec(); 128]);
        std::fs::write(&path, std::process::id().to_string()).unwrap();
        unsafe { libc::raise(libc::SIGABRT); }
        std::hint::black_box(marker);
        panic!("native abort returned");
    }
    std::fs::write(&path, std::process::id().to_string()).unwrap();
    loop { std::thread::sleep(Duration::from_millis(100)); }
}

struct Fixture { child: Child, worker: Option<libc::pid_t> }
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(pid) = self.worker { unsafe { libc::kill(pid, libc::SIGKILL); } }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn ready(path: &Path) -> libc::pid_t {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(pid) = text.parse() { return pid; }
        }
        assert!(Instant::now() < deadline, "synthetic child not ready");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn alive(pid: libc::pid_t) -> bool {
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // Ein verwaister Zombie haelt keine CPU/Modelle mehr; init reapt ihn.
        if stat.rsplit_once(") ").is_some_and(|(_, rest)| rest.starts_with('Z')) { return false; }
    }
    unsafe { libc::kill(pid, 0) == 0 }
}

fn orphan_probe(mode: &str, parent_dies: bool, protected: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ready");
    let child = command(&path, mode).stdin(Stdio::piped()).spawn().unwrap();
    let mut fixture = Fixture { child, worker: None };
    let pid = ready(&path);
    fixture.worker = Some(pid);
    assert!(alive(pid));
    // Child::wait schliesst sonst stdin und wuerde den Parent-PID-Test maskieren.
    let _held_input = if parent_dies { fixture.child.stdin.take() } else { None };
    if parent_dies {
        fixture.child.kill().unwrap();
        fixture.child.wait().unwrap();
    } else {
        // Selbst ungelesene Nutzbytes duerfen EOF nicht vor dem Waechter verbergen.
        use std::io::Write;
        fixture.child.stdin.as_mut().unwrap().write_all(b"synthetic").unwrap();
        fixture.child.stdin.take();
    }
    let deadline = Instant::now() + if protected { Duration::from_secs(5) } else { Duration::from_millis(200) };
    let mut running = true;
    while running && Instant::now() < deadline {
        running = alive(pid) && (parent_dies || fixture.child.try_wait().unwrap().is_none());
        if running { std::thread::sleep(Duration::from_millis(10)); }
    }
    assert_eq!(running, !protected);
}

#[test]
fn parent_death_ends_a_hung_worker_even_with_an_open_stdin_writer() {
    orphan_probe("parent-protected", true, true);
    orphan_probe("parent-unprotected", true, false);
    #[cfg(target_os = "linux")]
    orphan_probe("parent-watcher-only", true, true);
}

#[test]
fn stdin_eof_ends_a_hung_worker_while_its_parent_lives() {
    orphan_probe("protected", false, true);
    orphan_probe("unprotected", false, false);
}

#[test]
fn native_crash_after_synthetic_payload_cannot_dump_core() {
    use std::os::unix::process::ExitStatusExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ready");
    let child = command(&path, "crash").stdin(Stdio::piped()).spawn().unwrap();
    let mut fixture = Fixture { child, worker: None };
    ready(&path);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = fixture.child.try_wait().unwrap() { break status; }
        assert!(Instant::now() < deadline, "native crash did not exit");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.signal(), Some(libc::SIGABRT));
    assert!(!status.core_dumped());
    // Ohne suppress_dumps scheitern bereits die Policy-Assertions im Kind,
    // auch auf Hosts, auf denen Core-Dateien ohnehin abgeschaltet sind.
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| entry.unwrap().file_name() == "ready"));
}
