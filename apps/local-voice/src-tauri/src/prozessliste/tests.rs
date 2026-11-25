use super::*;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};

#[test]
fn one_fresh_snapshot_per_level_with_all_candidate_handles_held() {
    use std::cell::Cell;
    let entry = |pid, ppid| Entry { pid, ppid, name: "synthetic.exe".into() };
    // Die Handles gehoeren nur dem Test selbst, die Baumdaten sind synthetisch.
    let process = |pid, created| OwnedProcess {
        pid, created, handle: open(std::process::id()).unwrap(),
    };
    let rows = vec![entry(2, 1), entry(3, 1), entry(4, 1), entry(5, 1), entry(6, 2)];
    let opened = Cell::new(0);
    let snapshots = Cell::new(0);
    let tree = own_tree_with(
        process(1, 100),
        &rows,
        |pid| {
            opened.set(opened.get() + 1);
            // Kind 4 ist aelter als die Wurzel, Kind 5 hat eine wiedervergebene PID.
            Ok(process(pid, if pid == 4 { 99 } else { 110 }))
        },
        |_| Ok(0),
        || {
            snapshots.set(snapshots.get() + 1);
            assert_eq!(opened.get(), if snapshots.get() == 1 { 4 } else { 5 });
            Ok(vec![entry(2, 1), entry(3, 1), entry(4, 1), entry(5, 99), entry(6, 2)])
        },
    ).unwrap();
    assert_eq!(tree.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![1, 2, 3, 6]);
    assert_eq!(snapshots.get(), 2);
}

#[test]
fn failed_level_snapshot_is_not_a_partial_tree() {
    let process = |pid| OwnedProcess {
        pid, created: 100, handle: open(std::process::id()).unwrap(),
    };
    let rows = [Entry { pid: 2, ppid: 1, name: "synthetic.exe".into() }];
    assert!(own_tree_with(process(1), &rows, |pid| Ok(process(pid)), |_| Ok(0),
        || Err(windows::core::Error::from_win32())).is_err());
}

#[test]
fn parent_exit_after_opening_candidates_excludes_late_children() {
    let process = |pid, created| OwnedProcess {
        pid, created, handle: open(std::process::id()).unwrap(),
    };
    let rows = [Entry { pid: 2, ppid: 1, name: "synthetic.exe".into() }];
    let tree = own_tree_with(process(1, 100), &rows, |pid| Ok(process(pid, 110)),
        |_| Ok(105), || Ok(vec![Entry {
            pid: 2, ppid: 1, name: "synthetic.exe".into(),
        }])).unwrap();
    assert_eq!(tree.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![1]);
}

struct TestChild(Child);

impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "wird nur als synthetisches Kind gestartet"]
fn child_waits() {
    if std::env::var_os("LVA_PROCESS_LIST_CHILD").is_none() {
        return;
    }
    if std::env::var("LVA_PROCESS_LIST_CHILD").as_deref() == Ok("exit259") {
        std::process::exit(259);
    }
    std::io::stdout().write_all(b"READY\n").unwrap();
    std::io::stdout().flush().unwrap();
    let _ = std::io::stdin().read(&mut [0u8]);
}

#[test]
fn exitcode_259_is_not_confused_with_a_running_process() {
    let mut child = TestChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "prozessliste::tests::child_waits", "--ignored"])
            .env("LVA_PROCESS_LIST_CHILD", "exit259")
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert_eq!(status.code(), Some(259));
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Kind endet rechtzeitig");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!pid_alive(child.0.id()));
}

#[test]
fn own_path_is_found_but_a_missing_or_wrong_path_is_not() {
    let rows = snapshot().unwrap();
    let exe = std::env::current_exe().unwrap();
    assert!(path_alive(&rows, &exe));
    assert!(!path_alive(&rows, &exe.with_file_name("lva-missing-synthetic.exe")));
    // Gleicher Dateiname reicht nicht; insbesondere kein Namens-Fallback.
    assert!(!path_alive(&rows, &exe.join(exe.file_name().unwrap())));
    assert!(!path_alive(&[], &exe));
    let inaccessible = Entry { pid: 0, ppid: 0, name: exe.file_name().unwrap().to_os_string() };
    assert!(!path_alive(&[inaccessible], &exe));
}

#[test]
fn old_children_and_exited_parents_are_rejected() {
    assert!(child_belongs(100, 0, 101));
    assert!(!child_belongs(100, 0, 99));
    assert!(child_belongs(100, 120, 119));
    assert!(!child_belongs(100, 120, 121));
}

#[test]
fn a_pid_reused_between_snapshot_and_open_is_rejected() {
    let entry = |pid, ppid| Entry { pid, ppid, name: OsString::from("synthetic.exe") };
    assert!(same_candidate(&entry(20, 10), &entry(20, 10)));
    assert!(!same_candidate(&entry(20, 10), &entry(20, 99)));
    assert!(!same_candidate(&entry(20, 10), &entry(21, 10)));
}

#[test]
fn started_child_is_in_own_tree_and_has_memory() {
    let mut child = TestChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "prozessliste::tests::child_waits", "--ignored", "--nocapture"])
            .env("LVA_PROCESS_LIST_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    // Erst messen, wenn das Kind seinen Arbeitssatz aufgebaut hat.
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        use std::io::BufRead;
        let output = std::io::BufReader::new(stdout);
        for line in output.lines() {
            if line.unwrap().contains("READY") {
                let _ = tx.send(());
                break;
            }
        }
    });
    rx.recv_timeout(std::time::Duration::from_secs(10)).expect("Kind startet rechtzeitig");
    reader.join().unwrap();
    let pid = child.0.id();
    let tree = own_tree().unwrap();
    let process = tree.iter().find(|p| p.pid == pid).expect("eigenes Kind");
    assert!(snapshot().unwrap().iter().any(|p| p.pid == pid && p.ppid == std::process::id()));
    assert!(process.memory_bytes().unwrap() > 0);
    assert!(memory_bytes(pid).unwrap().unwrap() > 0);
    assert!(pid_alive(pid));
    assert!(memory_bytes(0).unwrap().is_none());
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert!(!pid_alive(pid));
    assert!(memory_bytes(pid).unwrap().is_none());
}
