//! Hub-POSTs über das System-curl (Windows-Standard seit 06.11.2026).
//!
//! Befund: Vom Windows-PC lehnt das IONOS-Frontend POSTs aus reqwest/hyper
//! sofort mit HTTP 500 ab, bevor PHP läuft; dieselbe Anfrage (gleiche
//! Kopfzeilen, gleicher Body) über `curl.exe` geht durch. Bis die Ursache
//! gefunden ist, gehen Push und Datei-Stücke unter Windows über curl.
//!
//! Sicherheitsregeln:
//! - nur `System32\curl.exe` aus `GetSystemDirectoryW` (Unix: `/usr/bin/curl`), nie über PATH;
//! - `-q` als erstes Argument (keine curlrc), `--globoff`;
//! - Konfiguration (URL, Kopfzeilen inkl. Token) nur über stdin, nie als Argument;
//! - Body (Chiffrat) in einer Datei mit Zufallsnamen in einem eigenen Verzeichnis,
//!   unter Windows mit geschützter DACL (nur Besitzer und SYSTEM); die Antwort
//!   kommt über stdout, nicht über eine Datei;
//! - Reste aus abgebrochenen Läufen räumt der nächste Aufruf weg;
//! - Exit-Code und HTTP-Status werden streng geprüft; stderr verworfen;
//! - Watchdog ab dem Start des Kindes (inklusive Konfigurationsübergabe);
//!   das Kind wird bei Fehler, Zeitüberschreitung und Drop beendet;
//! - auf reqwest zurückgefallen wird nur, wenn der Start NotFound meldet.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, PartialEq, Eq)]
pub enum CurlError {
    /// curl ist nicht vorhanden (Start meldet NotFound): Rückfall auf reqwest erlaubt.
    Missing,
    /// Jeder andere Fehler; nie über einen anderen Weg wiederholen.
    Failed(String),
}

fn fail(m: &str) -> CurlError {
    CurlError::Failed(m.to_string())
}

/// Trenner zwischen Antwort und HTTP-Status auf stdout.
const MARK: &str = "\n@@LV-HTTP-STATUS@@";
/// Name des eigenen Verzeichnisses im Benutzer-Temp.
const DIR_NAME: &str = "local-voice-curl";
/// Body-Dateien, die älter sind, stammen aus abgebrochenen Läufen.
const STALE: Duration = Duration::from_secs(3600);

/// POSTs zum Hub über curl? `LV_SYNC_CURL=0` schaltet ab, jeder andere Wert
/// ein; ohne Angabe nur unter Windows.
pub fn enabled() -> bool {
    enabled_for(std::env::var("LV_SYNC_CURL").ok().as_deref(), cfg!(windows))
}

pub fn enabled_for(var: Option<&str>, windows: bool) -> bool {
    match var {
        Some("0") => false,
        Some(_) => true,
        None => windows,
    }
}

/// Vertrauenswürdiger Pfad zum System-curl. Kein Dateisystemtest: ob es da
/// ist, entscheidet allein der Start.
pub fn system_curl() -> Result<PathBuf, CurlError> {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
        let mut buf = [0u16; 512];
        // SAFETY: Puffer gültig, Länge wird übergeben.
        let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
        if n == 0 || n >= buf.len() {
            return Err(fail("Systemverzeichnis nicht ermittelbar"));
        }
        Ok(PathBuf::from(String::from_utf16_lossy(&buf[..n])).join("curl.exe"))
    }
    #[cfg(not(windows))]
    {
        Ok(PathBuf::from("/usr/bin/curl"))
    }
}

/// Eigenes Verzeichnis für Body-Dateien; unter Windows mit geschützter DACL.
fn private_dir(base: &Path) -> Result<PathBuf, CurlError> {
    let dir = base.join(DIR_NAME);
    match std::fs::create_dir(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(fail("Temp-Verzeichnis nicht anlegbar")),
    }
    let meta = std::fs::symlink_metadata(&dir).map_err(|_| fail("Temp-Verzeichnis unlesbar"))?;
    if !meta.is_dir() {
        return Err(fail("Temp-Verzeichnis ist kein Verzeichnis"));
    }
    restrict(&dir)?;
    Ok(dir)
}

#[cfg(windows)]
fn restrict(dir: &Path) -> Result<(), CurlError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    };
    // Geschützt (keine Vererbung von oben): Vollzugriff nur Besitzer und SYSTEM,
    // vererbt auf Dateien im Verzeichnis.
    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)\0"
        .encode_utf16()
        .collect();
    let path: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: Zeichenketten sind nullterminiert; sd wird mit LocalFree freigegeben.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut sd,
            None,
        )
        .map_err(|_| fail("DACL nicht erzeugbar"))?;
        let ok = SetFileSecurityW(
            PCWSTR(path.as_ptr()),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            sd,
        );
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        if !ok.as_bool() {
            return Err(fail("DACL nicht setzbar"));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn restrict(dir: &Path) -> Result<(), CurlError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| fail("Rechte nicht setzbar"))
}

#[cfg(not(any(windows, unix)))]
fn restrict(_dir: &Path) -> Result<(), CurlError> {
    Ok(())
}

/// Body-Dateien abgebrochener Läufe entfernen (älter als `STALE`).
fn sweep(dir: &Path, older_than: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for e in entries.flatten() {
        let name = e.file_name();
        if !name.to_string_lossy().starts_with("body-") {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= older_than);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Ein Wert für die curl-Konfiguration: in Anführungszeichen, Steuerzeichen abgelehnt.
fn quote(s: &str) -> Result<String, CurlError> {
    if s.chars().any(|c| c.is_control()) {
        return Err(fail("Steuerzeichen in der Anfrage"));
    }
    Ok(format!(
        "\"{}\"",
        s.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

/// Konfiguration für `curl -K -`. Enthält das Token; nur über stdin geben.
fn config(
    req: &reqwest::Request,
    body_path: &Path,
    timeout: Duration,
) -> Result<String, CurlError> {
    let mut cfg = String::new();
    cfg.push_str(&format!("url = {}\n", quote(req.url().as_str())?));
    cfg.push_str(&format!("request = {}\n", quote(req.method().as_str())?));
    cfg.push_str(&format!(
        "data-binary = {}\n",
        quote(&format!("@{}", body_path.display()))?
    ));
    for (k, v) in req.headers() {
        let v = v
            .to_str()
            .map_err(|_| CurlError::Failed(format!("Kopfzeile {k} nicht darstellbar")))?;
        cfg.push_str(&format!("header = {}\n", quote(&format!("{k}: {v}"))?));
    }
    cfg.push_str(&format!(
        "user-agent = {}\n",
        quote(concat!("local-voice-ai/", env!("CARGO_PKG_VERSION")))?
    ));
    cfg.push_str("output = \"-\"\n");
    cfg.push_str("write-out = \"\\n@@LV-HTTP-STATUS@@%{http_code}\"\n");
    cfg.push_str(&format!("max-time = {}\n", timeout.as_secs().max(1)));
    cfg.push_str("silent\nshow-error\n");
    Ok(cfg)
}

/// Beendet und sammelt das Kind ein, falls es noch läuft.
struct Reap(Option<Child>);

impl Drop for Reap {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// stdout in Antwort und HTTP-Status zerlegen; Status genau drei Ziffern, 100..=599.
fn split_output(out: &[u8]) -> Option<(Vec<u8>, u16)> {
    let mark = MARK.as_bytes();
    let pos = out.windows(mark.len()).rposition(|w| w == mark)?;
    let code = std::str::from_utf8(&out[pos + mark.len()..]).ok()?;
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let code: u16 = code.parse().ok()?;
    (100..=599)
        .contains(&code)
        .then(|| (out[..pos].to_vec(), code))
}

/// Anfrage über das System-curl senden.
pub fn send(
    req: &reqwest::Request,
    timeout: Duration,
) -> Result<http::Response<Vec<u8>>, CurlError> {
    let exe = system_curl()?;
    send_with(&exe, &std::env::temp_dir(), req, timeout)
}

pub fn send_with(
    exe: &Path,
    temp_base: &Path,
    req: &reqwest::Request,
    timeout: Duration,
) -> Result<http::Response<Vec<u8>>, CurlError> {
    let body: &[u8] = match req.body() {
        None => &[],
        Some(b) => b
            .as_bytes()
            .ok_or_else(|| fail("Body ist kein Bytepuffer"))?,
    };
    let dir = private_dir(temp_base)?;
    sweep(&dir, STALE);
    let mut file = tempfile::Builder::new()
        .prefix("body-")
        .tempfile_in(&dir)
        .map_err(|_| fail("Temp-Datei nicht anlegbar"))?;
    file.write_all(body)
        .and_then(|_| file.flush())
        .map_err(|_| fail("Temp-Datei nicht schreibbar"))?;
    let body_path = file.into_temp_path();
    let cfg = config(req, &body_path, timeout)?;

    let mut cmd = Command::new(exe);
    // -q muss das erste Argument sein (keine curlrc).
    cmd.args(["-q", "--globoff", "-K", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let started = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(CurlError::Missing),
        Err(_) => return Err(fail("curl nicht startbar")),
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let mut reap = Reap(Some(child));
    let (mut stdin, mut stdout) = match (stdin, stdout) {
        (Some(i), Some(o)) => (i, o),
        _ => return Err(fail("curl ohne Ein-/Ausgabe")),
    };
    // Konfiguration und Ausgabe in eigenen Threads: der Watchdog unten gilt ab
    // dem Start, auch wenn das Kind stdin nicht liest oder stdout nicht leert.
    let writer = std::thread::spawn(move || stdin.write_all(cfg.as_bytes()).is_ok());
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).map(|_| buf)
    });

    let deadline = started + timeout + Duration::from_secs(15);
    let status = loop {
        let child = reap.0.as_mut().ok_or_else(|| fail("curl weg"))?;
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                drop(reap); // beendet das Kind; Writer und Reader enden dadurch
                let _ = writer.join();
                let _ = reader.join();
                return Err(fail("curl hängt, abgebrochen"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return Err(fail("curl-Status unlesbar")),
        }
    };
    reap.0 = None; // bereits beendet und eingesammelt
    let wrote = writer.join().unwrap_or(false);
    let out = reader
        .join()
        .map_err(|_| fail("curl-Ausgabe unlesbar"))?
        .map_err(|_| fail("curl-Ausgabe unlesbar"))?;
    if !wrote {
        return Err(fail("curl-Konfiguration nicht übergeben"));
    }
    if !status.success() {
        let code = status.code().map_or("?".into(), |c| c.to_string());
        return Err(CurlError::Failed(format!("curl Exit {code}")));
    }
    let (bytes, code) = split_output(&out).ok_or_else(|| fail("curl ohne gültigen HTTP-Status"))?;
    drop(body_path);
    http::Response::builder()
        .status(code)
        .body(bytes)
        .map_err(|_| fail("Antwort nicht aufbaubar"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(body: &[u8]) -> reqwest::Request {
        reqwest::Client::new()
            .post("https://hub.example/api/v1/voice/sync/push")
            .bearer_auth("wai_pat_GEHEIM")
            .header("content-type", "application/json")
            .body(body.to_vec())
            .build()
            .unwrap()
    }

    #[test]
    fn schalter_windows_standard_und_aus() {
        assert!(enabled_for(None, true));
        assert!(!enabled_for(None, false));
        assert!(!enabled_for(Some("0"), true));
        assert!(enabled_for(Some("1"), false));
    }

    #[test]
    fn konfiguration_enthaelt_token_nur_im_stdin_text() {
        let r = req(b"{}");
        let cfg = config(&r, Path::new("/t/b"), Duration::from_secs(9)).unwrap();
        assert!(cfg.contains("authorization: Bearer wai_pat_GEHEIM"));
        assert!(cfg.contains("max-time = 9"));
        assert!(cfg.contains("data-binary = \"@/t/b\""));
        assert!(cfg.contains("output = \"-\""));
    }

    #[test]
    fn steuerzeichen_werden_abgelehnt() {
        assert!(quote("a\nb").is_err());
        assert!(quote("a\rb").is_err());
        assert_eq!(quote("a\"b\\c").unwrap(), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn ausgabe_und_status_streng() {
        let ok = format!("{{\"a\":1}}{MARK}200");
        assert_eq!(
            split_output(ok.as_bytes()),
            Some((b"{\"a\":1}".to_vec(), 200))
        );
        let leer = format!("{MARK}401");
        assert_eq!(split_output(leer.as_bytes()), Some((Vec::new(), 401)));
        for bad in ["000", "20", "2000", "abc", "700"] {
            assert_eq!(
                split_output(format!("x{MARK}{bad}").as_bytes()),
                None,
                "{bad}"
            );
        }
        assert_eq!(split_output(b"200"), None);
    }

    #[test]
    fn fehlendes_curl_ist_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let r = req(b"x");
        let exe = tmp.path().join("gibt-es-nicht");
        assert_eq!(
            send_with(&exe, tmp.path(), &r, Duration::from_secs(1)).unwrap_err(),
            CurlError::Missing
        );
    }

    #[test]
    fn alte_reste_werden_geraeumt_frische_nicht() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = private_dir(tmp.path()).unwrap();
        std::fs::write(dir.join("body-alt"), b"x").unwrap();
        std::fs::write(dir.join("anderes"), b"x").unwrap();
        sweep(&dir, Duration::from_secs(3600));
        assert!(dir.join("body-alt").exists(), "frisch bleibt");
        sweep(&dir, Duration::ZERO);
        assert!(!dir.join("body-alt").exists(), "alt wird geräumt");
        assert!(dir.join("anderes").exists(), "fremde Namen bleiben");
    }

    /// Unix-Attrappen für curl: Argumente, Body-Bytegleichheit, Exit-Code,
    /// Watchdog, Parallelität und dass keine Body-Dateien bleiben.
    #[cfg(unix)]
    mod fake {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn script(dir: &Path, body: &str) -> PathBuf {
            let p = dir.join("curl");
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }

        /// Liest die Konfiguration, gibt den Body als Antwort zurück, Status 200.
        const ECHO: &str = r#"
[ "$1" = "-q" ] || exit 91
[ "$2" = "--globoff" ] || exit 92
cfg=$(cat)
body=$(printf '%s\n' "$cfg" | sed -n 's/^data-binary = "@\(.*\)"$/\1/p')
printf '%s\n' "$*" > "$(dirname "$0")/args"
printf '%s\n' "$body" > "$(dirname "$0")/bodypath"
cat "$body"
printf '\n@@LV-HTTP-STATUS@@200'
"#;

        fn rest(base: &Path) -> usize {
            std::fs::read_dir(base.join(DIR_NAME)).map_or(0, |d| d.count())
        }

        #[test]
        fn body_bytegleich_token_nicht_in_argumenten_datei_geloescht() {
            let tmp = tempfile::tempdir().unwrap();
            let exe = script(tmp.path(), ECHO);
            let payload: Vec<u8> = (0..=255u8).cycle().take(200_000).collect();
            let resp =
                send_with(&exe, tmp.path(), &req(&payload), Duration::from_secs(10)).unwrap();
            assert_eq!(resp.status(), 200);
            assert_eq!(resp.body(), &payload, "auch Antworten > Pipe-Puffer");
            let args = std::fs::read_to_string(tmp.path().join("args")).unwrap();
            assert!(!args.contains("GEHEIM"));
            assert_eq!(args.trim(), "-q --globoff -K -");
            let body_path = std::fs::read_to_string(tmp.path().join("bodypath")).unwrap();
            assert!(body_path.contains(DIR_NAME));
            assert!(!Path::new(body_path.trim()).exists(), "Body-Datei gelöscht");
            assert_eq!(rest(tmp.path()), 0);
            let mode = std::fs::metadata(tmp.path().join(DIR_NAME))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }

        #[test]
        fn exit_ungleich_null_trotz_200_ist_fehler() {
            let tmp = tempfile::tempdir().unwrap();
            let exe = script(
                tmp.path(),
                "cat >/dev/null; printf '\\n@@LV-HTTP-STATUS@@200'; exit 28",
            );
            let err = send_with(&exe, tmp.path(), &req(b"x"), Duration::from_secs(5)).unwrap_err();
            assert_eq!(err, CurlError::Failed("curl Exit 28".into()));
            assert_eq!(rest(tmp.path()), 0);
        }

        #[test]
        fn status_401_500_und_ungueltig() {
            let tmp = tempfile::tempdir().unwrap();
            for (code, ok) in [("401", true), ("500", true), ("000", false)] {
                let exe = script(
                    tmp.path(),
                    &format!("cat >/dev/null; printf '<html>\\n@@LV-HTTP-STATUS@@{code}'"),
                );
                let r = send_with(&exe, tmp.path(), &req(b"x"), Duration::from_secs(5));
                if ok {
                    let r = r.unwrap();
                    assert_eq!(r.status().as_u16().to_string(), code);
                    assert_eq!(r.body(), b"<html>");
                } else {
                    assert!(matches!(r, Err(CurlError::Failed(_))));
                }
            }
        }

        #[test]
        fn nicht_ausfuehrbar_ist_kein_missing() {
            let tmp = tempfile::tempdir().unwrap();
            let p = tmp.path().join("curl");
            std::fs::write(&p, "x").unwrap(); // ohne Ausführrecht → PermissionDenied
            let r = send_with(&p, tmp.path(), &req(b"x"), Duration::from_secs(1));
            assert_eq!(
                r.unwrap_err(),
                CurlError::Failed("curl nicht startbar".into())
            );
        }

        #[test]
        fn watchdog_auch_wenn_kind_stdin_nicht_liest() {
            let tmp = tempfile::tempdir().unwrap();
            let exe = script(tmp.path(), "exec sleep 60");
            let t = Instant::now();
            let err = send_with(&exe, tmp.path(), &req(b"x"), Duration::from_secs(1)).unwrap_err();
            assert_eq!(err, CurlError::Failed("curl hängt, abgebrochen".into()));
            assert!(t.elapsed() < Duration::from_secs(30));
            assert_eq!(rest(tmp.path()), 0);
        }

        #[test]
        fn parallele_aufrufe() {
            let tmp = tempfile::tempdir().unwrap();
            let exe = script(tmp.path(), ECHO);
            let base = tmp.path().to_path_buf();
            let handles: Vec<_> = (0..8u8)
                .map(|i| {
                    let (exe, base) = (exe.clone(), base.clone());
                    std::thread::spawn(move || {
                        let r = send_with(&exe, &base, &req(&[i; 1000]), Duration::from_secs(10))
                            .unwrap();
                        assert_eq!(r.body(), &vec![i; 1000]);
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
            assert_eq!(rest(tmp.path()), 0);
        }
    }
}
