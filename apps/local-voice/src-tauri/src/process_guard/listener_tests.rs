use super::*;
use std::path::Path;

#[test]
fn listener_paths_require_existing_component_exact_paths() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    let app2 = dir.path().join("app2");
    let data = dir.path().join("data");
    for folder in [&app, &app2, &data] {
        std::fs::create_dir(folder).unwrap();
        std::fs::write(folder.join("server.exe"), b"synthetic").unwrap();
    }
    let paths = ListenerPaths::new(Some(app.clone()), Some(data.clone()), Path::new(""));
    assert!(paths.allows(Some(&app.join("server.exe"))));
    assert!(paths.allows(Some(&data.join("server.exe"))));
    assert!(!paths.allows(Some(&app2.join("server.exe"))), "kein String-Praefix");
    assert!(!paths.allows(Some(&app.join("missing.exe"))), "keine lexikalische Freigabe");
    assert!(!paths.allows(None), "unlesbar ist kein eigener Prozess");
    let upper = app.join("server.exe").to_string_lossy().to_ascii_uppercase();
    assert!(paths.allows(Some(Path::new(&upper))));
    let canonical = std::fs::canonicalize(app.join("server.exe")).unwrap();
    assert!(paths.allows(Some(&canonical)), "auch mit Windows-Langpfadpraefix");
}

#[test]
fn configured_venv_allows_only_its_starter_and_base_executable() {
    let dir = tempfile::tempdir().unwrap();
    let fish = dir.path().join("fish");
    let base = dir.path().join("external-python");
    std::fs::create_dir_all(fish.join(".venv/Scripts")).unwrap();
    std::fs::create_dir_all(fish.join("tools")).unwrap();
    std::fs::create_dir(&base).unwrap();
    let starter = fish.join(".venv/Scripts/python.exe");
    let python = base.join("python.exe");
    let unrelated = base.join("other.exe");
    for file in [&starter, &python, &unrelated, &fish.join("tools/api_server.py")] {
        std::fs::write(file, b"synthetic").unwrap();
    }
    let config = fish.join(".venv/pyvenv.cfg");
    std::fs::write(&config, format!("home = {}\ninclude-system-site-packages = false\n", base.display())).unwrap();
    let paths = ListenerPaths::new(None, None, &fish);
    assert!(paths.allows(Some(&starter)), "gebuendelter/konfigurierter Starter");
    assert!(paths.allows(Some(&python)), "venv-Kind liegt ausserhalb des Fish-Ordners");
    assert!(!paths.allows(Some(&unrelated)), "kein ganzer Python-Ordner");
    std::fs::write(&config, "home = relative\n").unwrap();
    assert!(!ListenerPaths::new(None, None, &fish).allows(Some(&python)));
    std::fs::write(&config, format!("home = {}\nhome = {}\n", base.display(), base.display())).unwrap();
    assert!(!ListenerPaths::new(None, None, &fish).allows(Some(&python)), "mehrdeutige Konfiguration");
    std::fs::remove_file(&config).unwrap();
    assert!(!ListenerPaths::new(None, None, &fish).allows(Some(&python)));
    assert!(ListenerPaths::new(None, None, &fish).allows(Some(&starter)));
}

#[test]
fn directory_link_cannot_widen_the_allowed_folder() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    let outside = dir.path().join("outside");
    std::fs::create_dir(&app).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("server.exe"), b"synthetic").unwrap();
    // Ein Verzeichnis-Symlink prueft die Kanonisierungsgrenze ohne Hilfsprogramm.
    if std::os::windows::fs::symlink_dir(&outside, app.join("link")).is_err() {
        eprintln!("Linkprobe ausgelassen: kein Verzeichnis-Linkrecht");
        return; // Ohne Entwickler-Modus/Linkrecht bleibt die Laufzeitprobe separat offen.
    }
    let paths = ListenerPaths::new(Some(app), None, Path::new(""));
    assert!(!paths.allows(Some(&dir.path().join("app/link/server.exe"))));
}

#[test]
fn unreadable_image_handle_does_not_authorize_a_listener() {
    let image = crate::prozessliste::image_path(windows::Win32::Foundation::HANDLE::default());
    assert!(image.is_err());
    let paths = ListenerPaths::new(std::env::current_exe().unwrap().parent().map(Path::to_path_buf), None, Path::new(""));
    assert!(!paths.allows(image.ok().as_deref()));
    assert!(paths.allows(Some(&std::env::current_exe().unwrap())), "Gegenprobe: lesbarer eigener Pfad");
    assert!(kill_listener(0, 0, &paths).unwrap_err().contains("anderes Programm"),
        "auch ein nicht oeffenbarer Portbesitzer ist nicht bestaetigt");
}

#[test]
#[ignore = "nur als synthetisches Listener-Kind starten"]
fn listener_child() {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    println!("{}", listener.local_addr().unwrap().port());
    std::io::stdout().flush().unwrap();
    let _ = std::io::stdin().read_line(&mut String::new());
    drop(listener);
}

struct TestChild(std::process::Child);

impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn foreign_listener_survives_and_confirmed_listener_is_killed() {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    let mut child = TestChild(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "process_guard::listener_tests::listener_child", "--ignored", "--nocapture"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().unwrap());
    // Liest nur die synthetische Portnummer, keine Prozessdaten.
    let output = child.0.stdout.take().unwrap();
    let (send, recv) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(output).lines().map_while(Result::ok) {
            if let Ok(port) = line.trim().parse::<u16>() {
                let _ = send.send(port);
                break;
            }
        }
    });
    let port = recv.recv_timeout(Duration::from_secs(30)).expect("Listener bereit");
    let dir = tempfile::tempdir().unwrap();
    let foreign = ListenerPaths::new(Some(dir.path().to_path_buf()), None, Path::new(""));
    assert_eq!(listening_pid(port).unwrap(), Some(child.0.id()));
    let error = kill_listener(child.0.id(), port, &foreign).unwrap_err();
    assert!(error.contains("anderes Programm"));
    assert!(child.0.try_wait().unwrap().is_none(), "fremdes Kind bleibt am Leben");
    assert_eq!(listening_pid(port).unwrap(), Some(child.0.id()));
    // Dasselbe lebende Kind, nun mit bestaetigtem Programmordner: muss sterben.
    let own = ListenerPaths::new(std::env::current_exe().unwrap().parent().map(Path::to_path_buf), None, Path::new(""));
    kill_listener(child.0.id(), port, &own).unwrap();
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::WaitForSingleObject;
    assert_eq!(unsafe { WaitForSingleObject(HANDLE(child.0.as_raw_handle().cast()), 30_000) }, WAIT_OBJECT_0);
}
