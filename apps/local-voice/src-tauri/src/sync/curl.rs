//! Hub-POSTs über das System-curl (Windows-Standard seit 06.11.2026).
//!
//! Befund: Vom Windows-PC lehnt das IONOS-Frontend POSTs aus reqwest/hyper
//! sofort mit HTTP 500 ab, bevor PHP läuft; dieselbe Anfrage (gleiche
//! Kopfzeilen, gleicher Body) über `curl.exe` geht durch. Bis die Ursache
//! gefunden ist, gehen Push und Datei-Stücke unter Windows über curl.
//!
//! Sicherheitsregeln:
//! - nur `%SystemRoot%\System32\curl.exe` aus `GetSystemDirectoryW`, nie über PATH;
//! - `-q` als erstes Argument (keine curlrc), `--globoff`;
//! - Konfiguration (URL, Kopfzeilen inkl. Token) nur über stdin, nie als Argument;
//! - Body und Antwort in zufällig benannten Temp-Dateien, die beim Verlassen
//!   gelöscht werden;
//! - Exit-Code und HTTP-Status werden streng geprüft; stderr und Antwort
//!   landen nie im Log;
//! - das Kind wird bei Fehlern, Zeitüberschreitung (Watchdog) und Drop beendet.
//! - Auf reqwest zurückgefallen wird nur, wenn curl nicht vorhanden ist.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
pub enum CurlError {
    /// curl ist nicht vorhanden: Aufrufer darf auf reqwest zurückfallen.
    Missing,
    /// Jeder andere Fehler; nie über einen anderen Weg wiederholen.
    Failed(String),
}

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

/// Vertrauenswürdiger Pfad zum System-curl.
pub fn system_curl() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
        let mut buf = [0u16; 512];
        // SAFETY: Puffer gültig, Länge übergeben.
        let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
        if n == 0 || n >= buf.len() {
            return None;
        }
        let dir = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
        Some(dir.join("curl.exe"))
    }
    #[cfg(not(windows))]
    {
        ["/usr/bin/curl", "/bin/curl"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
    }
}

/// Ein Wert für die curl-Konfigurationsdatei: in Anführungszeichen, Steuerzeichen abgelehnt.
fn quote(s: &str) -> Result<String, CurlError> {
    if s.chars().any(|c| c.is_control()) {
        return Err(CurlError::Failed("Steuerzeichen in der Anfrage".into()));
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
    out_path: &Path,
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
    cfg.push_str(&format!(
        "output = {}\n",
        quote(&out_path.display().to_string())?
    ));
    cfg.push_str("write-out = \"%{http_code}\"\n");
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

/// HTTP-Status aus `write-out`: genau drei Ziffern, 100..=599.
fn parse_status(stdout: &[u8]) -> Option<u16> {
    let s = std::str::from_utf8(stdout).ok()?.trim();
    if s.len() != 3 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let code: u16 = s.parse().ok()?;
    (100..=599).contains(&code).then_some(code)
}

/// Anfrage über das System-curl senden.
pub fn send(
    req: &reqwest::Request,
    timeout: Duration,
) -> Result<http::Response<Vec<u8>>, CurlError> {
    let exe = system_curl().ok_or(CurlError::Missing)?;
    send_with(&exe, req, timeout)
}

pub fn send_with(
    exe: &Path,
    req: &reqwest::Request,
    timeout: Duration,
) -> Result<http::Response<Vec<u8>>, CurlError> {
    let fail = |m: &str| CurlError::Failed(m.to_string());
    if !exe.is_file() {
        return Err(CurlError::Missing);
    }
    let body: &[u8] = match req.body() {
        None => &[],
        Some(b) => b
            .as_bytes()
            .ok_or_else(|| fail("Body ist kein Bytepuffer"))?,
    };
    // Zufällige Namen im Benutzer-Temp (unter Windows nur für den Benutzer
    // lesbar); TempPath löscht beim Verlassen, auch bei Fehlern.
    let mut body_file =
        tempfile::NamedTempFile::new().map_err(|_| fail("Temp-Datei nicht anlegbar"))?;
    body_file
        .write_all(body)
        .and_then(|_| body_file.flush())
        .map_err(|_| fail("Temp-Datei nicht schreibbar"))?;
    let body_path = body_file.into_temp_path();
    let out_path = tempfile::NamedTempFile::new()
        .map_err(|_| fail("Temp-Datei nicht anlegbar"))?
        .into_temp_path();
    let cfg = config(req, &body_path, &out_path, timeout)?;

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
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(CurlError::Missing),
        Err(_) => return Err(fail("curl nicht startbar")),
    };
    let mut reap = Reap(Some(child));
    {
        let child = reap.0.as_mut().ok_or_else(|| fail("curl weg"))?;
        let mut stdin = child.stdin.take().ok_or_else(|| fail("curl ohne stdin"))?;
        stdin
            .write_all(cfg.as_bytes())
            .map_err(|_| fail("curl-Konfiguration nicht übergeben"))?;
        // stdin schließen, damit curl die Konfiguration zu Ende liest.
    }
    drop(cfg);

    // Watchdog: curl hat max-time; hängt es darüber hinaus, wird es beendet.
    let deadline = Instant::now() + timeout + Duration::from_secs(15);
    loop {
        let child = reap.0.as_mut().ok_or_else(|| fail("curl weg"))?;
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                return Err(fail("curl hängt, abgebrochen"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return Err(fail("curl-Status unlesbar")),
        }
    }
    let child = reap.0.take().ok_or_else(|| fail("curl weg"))?;
    let out = child
        .wait_with_output()
        .map_err(|_| fail("curl-Ausgabe unlesbar"))?;
    if !out.status.success() {
        let code = out.status.code().map_or("?".into(), |c| c.to_string());
        return Err(CurlError::Failed(format!("curl Exit {code}")));
    }
    let status = parse_status(&out.stdout).ok_or_else(|| fail("curl ohne gültigen HTTP-Status"))?;
    let bytes = std::fs::read(&out_path).map_err(|_| fail("Antwort nicht lesbar"))?;
    http::Response::builder()
        .status(status)
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
        let cfg = config(
            &r,
            Path::new("/t/b"),
            Path::new("/t/o"),
            Duration::from_secs(9),
        )
        .unwrap();
        assert!(cfg.contains("authorization: Bearer wai_pat_GEHEIM"));
        assert!(cfg.contains("max-time = 9"));
        assert!(cfg.contains("data-binary = \"@/t/b\""));
    }

    #[test]
    fn steuerzeichen_werden_abgelehnt() {
        assert!(quote("a\nb").is_err());
        assert!(quote("a\"b\\c").unwrap() == "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn status_streng() {
        assert_eq!(parse_status(b"200"), Some(200));
        assert_eq!(parse_status(b"200\n"), Some(200));
        assert_eq!(parse_status(b"000"), None);
        assert_eq!(parse_status(b"20"), None);
        assert_eq!(parse_status(b"2000"), None);
        assert_eq!(parse_status(b"abc"), None);
        assert_eq!(parse_status(b"700"), None);
    }

    #[test]
    fn fehlendes_curl_ist_missing() {
        let r = req(b"x");
        assert_eq!(
            send_with(Path::new("/gibt/es/nicht/curl"), &r, Duration::from_secs(1)).unwrap_err(),
            CurlError::Missing
        );
    }

    /// Unix-Attrappen für curl: prüfen Argumente, Body-Bytegleichheit,
    /// Exit-Code-Behandlung, Watchdog und dass keine Temp-Dateien bleiben.
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

        /// Liest die Konfiguration von stdin, kopiert den Body in die Antwort
        /// und meldet die Argumente in einer Protokolldatei.
        const ECHO: &str = r#"
[ "$1" = "-q" ] || exit 91
[ "$2" = "--globoff" ] || exit 92
cfg=$(cat)
body=$(printf '%s\n' "$cfg" | sed -n 's/^data-binary = "@\(.*\)"$/\1/p')
out=$(printf '%s\n' "$cfg" | sed -n 's/^output = "\(.*\)"$/\1/p')
printf '%s\n' "$*" > "$(dirname "$0")/args"
printf '%s\n' "$body" > "$(dirname "$0")/bodypath"
cp "$body" "$out"
printf 200
"#;

        #[test]
        fn body_bytegleich_token_nicht_in_argumenten_temp_geloescht() {
            let dir = tempfile::tempdir().unwrap();
            let exe = script(dir.path(), ECHO);
            let payload: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
            let r = req(&payload);
            let resp = send_with(&exe, &r, Duration::from_secs(10)).unwrap();
            assert_eq!(resp.status(), 200);
            assert_eq!(resp.body(), &payload);
            let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
            assert!(!args.contains("GEHEIM"));
            assert_eq!(args.trim(), "-q --globoff -K -");
            let body_path = std::fs::read_to_string(dir.path().join("bodypath")).unwrap();
            assert!(!Path::new(body_path.trim()).exists(), "Temp-Body gelöscht");
        }

        #[test]
        fn exit_ungleich_null_trotz_200_ist_fehler() {
            let dir = tempfile::tempdir().unwrap();
            let exe = script(dir.path(), "cat >/dev/null; printf 200; exit 28");
            let err = send_with(&exe, &req(b"x"), Duration::from_secs(5)).unwrap_err();
            assert_eq!(err, CurlError::Failed("curl Exit 28".into()));
        }

        #[test]
        fn status_401_500_und_ungueltig() {
            let dir = tempfile::tempdir().unwrap();
            for (code, ok) in [("401", true), ("500", true), ("000", false)] {
                let exe = script(
                    dir.path(),
                    &format!(
                        "cfg=$(cat); out=$(printf '%s\\n' \"$cfg\" | sed -n 's/^output = \"\\(.*\\)\"$/\\1/p'); printf '<html>' > \"$out\"; printf {code}"
                    ),
                );
                let r = send_with(&exe, &req(b"x"), Duration::from_secs(5));
                match ok {
                    true => assert_eq!(r.unwrap().status().as_u16().to_string(), code),
                    false => assert!(matches!(r, Err(CurlError::Failed(_)))),
                }
            }
        }

        #[test]
        fn watchdog_beendet_haengendes_curl() {
            let dir = tempfile::tempdir().unwrap();
            let exe = script(dir.path(), "cat >/dev/null; exec sleep 60");
            // timeout 1 s + 15 s Zuschlag: hier über eine kleine Zeit prüfen,
            // dass nicht 60 s gewartet wird.
            let t = Instant::now();
            let err = send_with(&exe, &req(b"x"), Duration::from_secs(1)).unwrap_err();
            assert_eq!(err, CurlError::Failed("curl hängt, abgebrochen".into()));
            assert!(t.elapsed() < Duration::from_secs(30));
        }

        #[test]
        fn parallele_aufrufe() {
            let dir = tempfile::tempdir().unwrap();
            let exe = script(dir.path(), "cfg=$(cat); body=$(printf '%s\\n' \"$cfg\" | sed -n 's/^data-binary = \"@\\(.*\\)\"$/\\1/p'); out=$(printf '%s\\n' \"$cfg\" | sed -n 's/^output = \"\\(.*\\)\"$/\\1/p'); cp \"$body\" \"$out\"; printf 200");
            let handles: Vec<_> = (0..8u8)
                .map(|i| {
                    let exe = exe.clone();
                    std::thread::spawn(move || {
                        let r = send_with(&exe, &req(&[i; 1000]), Duration::from_secs(10)).unwrap();
                        assert_eq!(r.body(), &vec![i; 1000]);
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
        }
    }
}
