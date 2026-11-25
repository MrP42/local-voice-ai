//! Nur synthetische Daten; kein Systemproxy und kein externer Netzverkehr.
use super::*;
use futures_util::stream;
use std::collections::VecDeque;

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Fake(std::sync::Mutex<VecDeque<Vec<u8>>>);

#[async_trait::async_trait]
impl Quelle for Fake {
    async fn get(&self, _: &str) -> Result<Antwort, InstallFehler> {
        let bytes = self.0.lock().unwrap().pop_front().expect("unerwartete Anfrage");
        Ok(Antwort {
            total: Some(bytes.len() as u64),
            chunks: Box::pin(stream::iter(vec![Ok(bytes)])),
        })
    }
}

fn fake(data: Vec<Vec<u8>>) -> Fake {
    Fake(std::sync::Mutex::new(data.into()))
}

fn claude_data(binary: &[u8], checksum: &str) -> Vec<Vec<u8>> {
    vec![
        b"2.1.50\n".to_vec(),
        serde_json::to_vec(&serde_json::json!({"platforms": {
            "win32-x64": {"checksum": checksum}
        }})).unwrap(),
        binary.to_vec(),
    ]
}

#[tokio::test]
async fn checksum_falsch_entfernt_temp_und_bewahrt_altes_ziel() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("claude.exe");
    std::fs::write(&target, b"altes Programm").unwrap();
    let result = provision(
        Cli::Claude, "x86_64", &target, &fake(claude_data(b"MZneu", &"0".repeat(64))),
        &Abbruch::default(), &|| false, &|_, _, _| {},
    ).await;
    assert_eq!(result, Err(InstallFehler::Pruefsumme));
    assert_eq!(std::fs::read(&target).unwrap(), b"altes Programm");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    // Gegenprobe mit korrekter Summe: neues Ziel wird veroeffentlicht.
    provision(Cli::Claude, "x86_64", &target,
        &fake(claude_data(b"MZneu", &sha(b"MZneu"))), &Abbruch::default(),
        &|| false, &|_, _, _| {}).await.unwrap();
    assert_eq!(std::fs::read(target).unwrap(), b"MZneu");
}

#[tokio::test]
async fn fehlender_zielordner_wird_erstellt_und_richtlinie_sperrt_vor_netz() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("noch/nicht/claude.exe");
    let result = provision(Cli::Claude, "x86_64", &target, &fake(vec![]),
        &Abbruch::default(), &|| true, &|_, _, _| {}).await;
    assert_eq!(result, Err(InstallFehler::Richtlinie));
    assert!(!target.parent().unwrap().exists());
    provision(Cli::Claude, "x86_64", &target,
        &fake(claude_data(b"MZneu", &sha(b"MZneu"))), &Abbruch::default(),
        &|| false, &|_, _, _| {}).await.unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"MZneu");
    assert_eq!(std::fs::read_dir(target.parent().unwrap()).unwrap().count(), 1);
}

#[tokio::test]
async fn abbruch_mitten_im_laden_entfernt_reste() {
    struct Langsam;
    #[async_trait::async_trait]
    impl Quelle for Langsam {
        async fn get(&self, url: &str) -> Result<Antwort, InstallFehler> {
            if !url.ends_with(".exe") {
                let data = if url.ends_with("/latest") { b"2.1.50".to_vec() } else {
                    serde_json::to_vec(&serde_json::json!({"platforms": {
                        "win32-x64": {"checksum": sha(b"MZanfang")}
                    }})).unwrap()
                };
                return fake(vec![data]).get(url).await;
            }
            Ok(Antwort { total: None, chunks: Box::pin(
                stream::iter(vec![Ok(b"MZanfang".to_vec())])
                    .chain(stream::pending())
            ) })
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let cancel = Abbruch::default();
    let received = std::sync::atomic::AtomicBool::new(false);
    let result = provision(Cli::Claude, "x86_64", &dir.path().join("claude.exe"),
        &Langsam, &cancel, &|| false, &|phase, count, _| {
            if phase == "laden" && count > 0 {
                received.store(true, Ordering::SeqCst);
                cancel.cancel();
            }
        }).await;
    assert_eq!(result, Err(InstallFehler::Abgebrochen));
    assert!(received.load(Ordering::SeqCst), "Abbruch muss nach dem ersten Datenstueck passieren");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    // Gegenprobe: ohne Abbruch kann derselbe erste Block vollstaendig geladen werden.
    let mut temp = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    assert!(laden(&fake(vec![b"MZanfang".to_vec()]),
        "https://downloads.claude.ai/test", &mut temp, MAX_BINARY,
        &Abbruch::default(), &|_, _| {}).await.is_ok());
}

#[tokio::test]
async fn fremde_umleitung_wird_vor_weiterer_anfrage_abgewiesen() {
    for host in ["fremd.invalid", "release-assets.githubusercontent.com"] {
        let mut requests = Vec::new();
        let mut responses = VecDeque::from([
            reqwest::Response::from(http::Response::builder().status(302)
                .header("location", format!("https://{host}/asset?token=synthetisch"))
                .body(reqwest::Body::from(Vec::new())).unwrap()),
            reqwest::Response::from(http::Response::builder().status(200)
                .body(reqwest::Body::from(b"synthetisch".to_vec())).unwrap()),
        ]);
        let result = http_get(Cli::Codex, "https://github.com/openai/codex/asset", |url| {
            requests.push(url.host_str().unwrap().to_string());
            std::future::ready(Ok(responses.pop_front().unwrap()))
        }).await;
        if host == "fremd.invalid" {
            assert!(matches!(result, Err(InstallFehler::NetzGesperrt { ziel }) if ziel == host));
            assert_eq!(requests, vec!["github.com"]);
        } else {
            assert!(result.is_ok());
            assert_eq!(requests, vec!["github.com", host]);
        }
    }
}

#[test]
fn redirects_und_https_bleiben_in_der_anbieter_allowlist() {
    for host in HOSTS_CODEX {
        assert!(erlaubte_url(Cli::Codex, &format!("https://{host}/asset")).is_ok());
    }
    for url in ["https://fremd.invalid/asset", "http://github.com/asset",
        "https://github.com.evil.invalid/asset", "https://github.com:444/asset",
        "https://token@github.com/asset", "https://downloads.claude.ai/a"] {
        assert!(erlaubte_url(Cli::Codex, url).is_err());
    }
    assert!(erlaubte_url(Cli::Claude, "https://downloads.claude.ai/a").is_ok());
}

#[test]
fn manifest_und_release_fuer_beide_architekturen() {
    for (arch, platform, triple) in [
        ("x86_64", "win32-x64", "x86_64-pc-windows-msvc"),
        ("aarch64", "win32-arm64", "aarch64-pc-windows-msvc"),
    ] {
        let hash = sha(b"synthetisch");
        let manifest = serde_json::json!({"platforms": {platform: {"checksum": hash}}});
        assert_eq!(claude_asset("2.1.50", &manifest, arch).unwrap().checksum, hash);
        assert!(claude_asset("../2", &manifest, arch).is_err());
        let release = serde_json::json!({"assets": [{
            "name": format!("codex-{triple}.zip"), "digest": format!("sha256:{hash}"),
            "browser_download_url": format!("https://github.com/openai/codex/releases/download/rust-v1/codex-{triple}.zip")
        }]});
        let asset = codex_asset(&release, arch).unwrap();
        assert_eq!(asset.checksum, hash);
        assert_eq!(asset.member.as_deref(), Some(format!("codex-{triple}.exe").as_str()));
        let mut bad = release.clone();
        bad["assets"][0]["digest"] = serde_json::Value::Null;
        assert!(codex_asset(&bad, arch).is_err());
        bad = release.clone();
        bad["assets"][0]["browser_download_url"] = "https://fremd.invalid/a".into();
        assert!(codex_asset(&bad, arch).is_err());
    }
    assert!(plattform("riscv64").is_err());
}

#[test]
fn speicher_und_blockaden_werden_ohne_rohe_meldungen_abgebildet() {
    for code in [28, 39, 112] {
        assert_eq!(io_fehler(std::io::Error::from_raw_os_error(code)), InstallFehler::Speicher);
    }
    for code in [5, 32, 33, 225, 226, 1260] {
        assert_eq!(io_fehler(std::io::Error::from_raw_os_error(code)), InstallFehler::Blockiert);
    }
    assert_eq!(io_fehler(std::io::Error::from(std::io::ErrorKind::NotFound)), InstallFehler::Pfad);
}

#[test]
fn nur_ein_download_je_anbieter_bis_guard_faellt() {
    // Eigene Registry: keine Abhaengigkeit von parallel laufenden Tests.
    let registry = std::sync::Mutex::new(Vec::new());
    let first = Auftrag::begin_in(Cli::Claude, &registry).unwrap();
    assert!(matches!(Auftrag::begin_in(Cli::Claude, &registry), Err(InstallFehler::Beschaeftigt)));
    let other = Auftrag::begin_in(Cli::Codex, &registry).unwrap();
    first.cancel.cancel();
    assert!(first.cancel.check().is_err());
    drop(first);
    assert!(Auftrag::begin_in(Cli::Claude, &registry).is_ok());
    drop(other);
}

#[test]
fn zip_entnimmt_nur_den_exakten_programmnamen() {
    use zip::write::SimpleFileOptions;
    let dir = tempfile::tempdir().unwrap();
    for member in ["../codex.exe", "dir/codex.exe", "codex.exe"] {
        let mut archive = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        {
            let mut zip = zip::ZipWriter::new(archive.as_file_mut());
            zip.start_file(member, SimpleFileOptions::default()).unwrap();
            zip.write_all(b"MZsynthetisch").unwrap();
            zip.finish().unwrap();
        }
        let mut exe = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        let result = entpacken(&mut archive, &mut exe, "codex.exe", &Abbruch::default());
        assert_eq!(result.is_ok(), member == "codex.exe");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

#[test]
fn programmpfad_prueft_dateiname_dateityp_kopf_und_lokalitaet() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("codex.exe");
    assert!(valid_path(Cli::Codex, &path).is_err());
    std::fs::create_dir(&path).unwrap();
    assert!(valid_path(Cli::Codex, &path).is_err());
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, b"kein Programm").unwrap();
    assert!(valid_path(Cli::Codex, &path).is_err());
    std::fs::write(&path, b"MZsynthetisch").unwrap();
    assert!(valid_path(Cli::Codex, &path).is_ok());
    assert!(valid_path(Cli::Claude, &path).is_err());
    for text in [r"\\synthetisch.invalid\share\codex.exe", "//synthetisch.invalid/share/codex.exe", "codex.exe"] {
        assert!(valid_path(Cli::Codex, Path::new(text)).is_err());
    }
}

#[tokio::test]
async fn nachtraegliche_richtlinie_verhindert_ablegen_und_raeuemt_auf() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("claude.exe");
    let disabled = AtomicBool::new(false);
    let result = provision(Cli::Claude, "x86_64", &target,
        &fake(claude_data(b"MZneu", &sha(b"MZneu"))), &Abbruch::default(),
        &|| disabled.load(Ordering::SeqCst), &|phase, _, _| {
            if phase == "ablegen" { disabled.store(true, Ordering::SeqCst); }
        }).await;
    assert_eq!(result, Err(InstallFehler::Richtlinie));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    // Gegenprobe: das gleiche Programm kann ohne Sperre abgelegt werden.
    provision(Cli::Claude, "x86_64", &target,
        &fake(claude_data(b"MZneu", &sha(b"MZneu"))), &Abbruch::default(),
        &|| false, &|_, _, _| {}).await.unwrap();
    assert!(target.is_file());
}

#[tokio::test]
async fn groessenlimit_gilt_auch_fuer_kleine_synthetische_downloads() {
    let dir = tempfile::tempdir().unwrap();
    let mut temp = NamedTempFile::new_in(dir.path()).unwrap();
    let result = laden(&fake(vec![b"MZ123".to_vec()]), "https://downloads.claude.ai/test",
        &mut temp, 4, &Abbruch::default(), &|_, _| {}).await;
    assert_eq!(result, Err(InstallFehler::Ungueltig));
    assert_eq!(temp.as_file().metadata().unwrap().len(), 0);
    let result = laden(&fake(vec![b"MZ123".to_vec()]), "https://downloads.claude.ai/test",
        &mut temp, 5, &Abbruch::default(), &|_, _| {}).await;
    assert_eq!(result, Ok(sha(b"MZ123")));
}

#[cfg(windows)]
#[tokio::test]
async fn gesperrte_exe_bleibt_bei_rename_fehler_erhalten() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("claude.exe");
    std::fs::write(&target, b"MZalt").unwrap();
    let locked = std::fs::OpenOptions::new().read(true).share_mode(1).open(&target).unwrap();
    let result = provision(Cli::Claude, "x86_64", &target,
        &fake(claude_data(b"MZneu", &sha(b"MZneu"))), &Abbruch::default(),
        &|| false, &|_, _, _| {}).await;
    assert_eq!(result, Err(InstallFehler::Blockiert));
    assert_eq!(std::fs::read(&target).unwrap(), b"MZalt");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    drop(locked);
    provision(Cli::Claude, "x86_64", &target,
        &fake(claude_data(b"MZneu", &sha(b"MZneu"))), &Abbruch::default(),
        &|| false, &|_, _, _| {}).await.unwrap();
    assert_eq!(std::fs::read(target).unwrap(), b"MZneu");
}
