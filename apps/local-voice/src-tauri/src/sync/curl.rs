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
//!   je Aufruf atomar neu angelegt (Windows: CreateDirectoryW mit geschützter
//!   DACL nur für die Benutzer-SID, Besitzer = Benutzer; Unix: 0700), ein
//!   vorhandenes Verzeichnis wird nie übernommen; die Antwort kommt über stdout
//!   (höchstens 1 MiB), nicht über eine Datei;
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
/// Präfix der Arbeitsverzeichnisse im Benutzer-Temp (eines je Aufruf).
const DIR_PREFIX: &str = "lv-curl-";
/// Arbeitsverzeichnisse, die älter sind, stammen aus abgebrochenen Läufen.
const STALE: Duration = Duration::from_secs(3600);
/// Höchstgröße der Antwort auf stdout (Push/Stück antworten mit wenigen KB).
const MAX_OUT: usize = 1024 * 1024;

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

/// Zufälliger Verzeichnisname (Prozess-Zufallsschlüssel, Zeit, Zähler).
fn random_name() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(N.fetch_add(1, Ordering::Relaxed));
    h.write_u128(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.write_u32(std::process::id());
    format!("{DIR_PREFIX}{:016x}", h.finish())
}

/// Arbeitsverzeichnis nur für diesen Aufruf: atomar neu angelegt, schon beim
/// Anlegen nur für den aktuellen Benutzer zugänglich (Windows: DACL mit der
/// Benutzer-SID aus dem Prozess-Token, Besitzer = Benutzer; Unix: 0700).
/// Ein vorhandenes Verzeichnis wird nie übernommen. Wird beim Drop entfernt.
struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn work_dir(base: &Path) -> Result<WorkDir, CurlError> {
    for _ in 0..8 {
        let path = base.join(random_name());
        if create_dir_private(&path)? {
            return Ok(WorkDir(path));
        }
    }
    Err(fail("Temp-Verzeichnis nicht anlegbar"))
}

/// Legt `path` neu an. `Ok(false)`, wenn es schon existiert (nie übernehmen).
#[cfg(windows)]
fn create_dir_private(path: &Path) -> Result<bool, CurlError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, ERROR_ALREADY_EXISTS, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::CreateDirectoryW;
    let sid = user_sid()?;
    // Besitzer = Benutzer; geschützte DACL (keine Vererbung von oben), Vollzugriff
    // nur für diesen Benutzer, vererbt auf die Body-Datei.
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;OICI;FA;;;{sid})\0")
        .encode_utf16()
        .collect();
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: nullterminierte Zeichenketten; sd wird mit LocalFree freigegeben.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut sd,
            None,
        )
        .map_err(|_| fail("DACL nicht erzeugbar"))?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let r = CreateDirectoryW(PCWSTR(wide.as_ptr()), Some(&sa));
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        match r {
            Ok(()) => Ok(true),
            Err(e) if e.code() == ERROR_ALREADY_EXISTS.to_hresult() => Ok(false),
            Err(_) => Err(fail("Temp-Verzeichnis nicht anlegbar")),
        }
    }
}

/// SID des Prozess-Benutzers als Zeichenkette (S-1-5-21-…).
#[cfg(windows)]
fn user_sid() -> Result<String, CurlError> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: Puffer ist ausreichend groß und 8-Byte-ausgerichtet; Handles und
    // Zeichenketten werden freigegeben.
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok)
            .map_err(|_| fail("Prozess-Token nicht lesbar"))?;
        let mut len = 0u32;
        let _ = GetTokenInformation(tok, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        let r = GetTokenInformation(tok, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len);
        let _ = CloseHandle(tok);
        r.map_err(|_| fail("Benutzer-SID nicht lesbar"))?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s).map_err(|_| fail("SID nicht lesbar"))?;
        let sid = s.to_string();
        let _ = LocalFree(Some(HLOCAL(s.0.cast())));
        let sid = sid.map_err(|_| fail("SID nicht lesbar"))?;
        if !sid.starts_with("S-") || !sid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(fail("SID ungültig"));
        }
        Ok(sid)
    }
}

#[cfg(not(windows))]
fn create_dir_private(path: &Path) -> Result<bool, CurlError> {
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    match b.create(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(_) => Err(fail("Temp-Verzeichnis nicht anlegbar")),
    }
}

/// Arbeitsverzeichnisse abgebrochener Läufe entfernen (älter als `older_than`).
/// `remove_dir_all` folgt keinen Verknüpfungen.
fn sweep(base: &Path, older_than: Duration) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let now = SystemTime::now();
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().starts_with(DIR_PREFIX) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= older_than);
        if old && meta.is_dir() {
            let _ = std::fs::remove_dir_all(e.path());
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
    sweep(temp_base, STALE);
    let dir = work_dir(temp_base)?;
    let body_path = dir.0.join("body");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&body_path)
        .and_then(|mut f| f.write_all(body).and_then(|_| f.flush()))
        .map_err(|_| fail("Temp-Datei nicht schreibbar"))?;
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
    // Ausgabe begrenzt: mehr als MAX_OUT bricht ab (Kind wird beendet).
    let too_big = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = too_big.clone();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let r = (&mut stdout).take(MAX_OUT as u64 + 1).read_to_end(&mut buf);
        if buf.len() > MAX_OUT {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        r.map(|_| buf)
    });

    let deadline = started + timeout + Duration::from_secs(15);
    let status = loop {
        let child = reap.0.as_mut().ok_or_else(|| fail("curl weg"))?;
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(_) if too_big.load(std::sync::atomic::Ordering::SeqCst) => {
                drop(reap);
                let _ = writer.join();
                let _ = reader.join();
                return Err(fail("curl-Antwort zu groß, abgebrochen"));
            }
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
    // Vor dem Exit-Code: das Kind stirbt nach dem Abbruch des Lesens oft an SIGPIPE.
    if out.len() > MAX_OUT {
        return Err(fail("curl-Antwort zu groß, abgebrochen"));
    }
    if !wrote {
        return Err(fail("curl-Konfiguration nicht übergeben"));
    }
    if !status.success() {
        let code = status.code().map_or("?".into(), |c| c.to_string());
        return Err(CurlError::Failed(format!("curl Exit {code}")));
    }
    let (bytes, code) = split_output(&out).ok_or_else(|| fail("curl ohne gültigen HTTP-Status"))?;
    drop(dir);
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

    fn lv_dirs(base: &Path) -> usize {
        std::fs::read_dir(base).map_or(0, |d| {
            d.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(DIR_PREFIX))
                .count()
        })
    }

    #[test]
    fn alte_reste_werden_geraeumt_frische_nicht() {
        let tmp = tempfile::tempdir().unwrap();
        let alt = work_dir(tmp.path()).unwrap();
        std::fs::write(alt.0.join("body"), b"x").unwrap();
        let alt_pfad = alt.0.clone();
        std::mem::forget(alt); // simuliert einen abgebrochenen Lauf
        std::fs::create_dir(tmp.path().join("anderes")).unwrap();
        std::fs::write(tmp.path().join("lv-curl-datei"), b"x").unwrap();
        sweep(tmp.path(), Duration::from_secs(3600));
        assert!(alt_pfad.exists(), "frisch bleibt");
        sweep(tmp.path(), Duration::ZERO);
        assert!(!alt_pfad.exists(), "alt wird geräumt");
        assert!(tmp.path().join("anderes").exists(), "fremde Namen bleiben");
        assert!(
            tmp.path().join("lv-curl-datei").exists(),
            "nur Verzeichnisse"
        );
    }

    #[test]
    fn vorhandenes_verzeichnis_wird_nie_uebernommen() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("lv-curl-vorher");
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("fremd"), b"x").unwrap();
        assert_eq!(create_dir_private(&p), Ok(false));
        assert!(p.join("fremd").exists(), "nicht angefasst");
        // Auch eine Datei unter dem Namen gilt als vorhanden.
        let f = tmp.path().join("lv-curl-datei");
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(create_dir_private(&f), Ok(false));
        // Neu anlegen klappt, zweites Anlegen desselben Pfads nicht.
        let n = tmp.path().join("lv-curl-neu");
        assert_eq!(create_dir_private(&n), Ok(true));
        assert_eq!(create_dir_private(&n), Ok(false));
    }

    #[test]
    fn arbeitsverzeichnis_je_aufruf_eindeutig_und_wird_geraeumt() {
        let tmp = tempfile::tempdir().unwrap();
        let a = work_dir(tmp.path()).unwrap();
        let b = work_dir(tmp.path()).unwrap();
        assert_ne!(a.0, b.0);
        assert!(a
            .0
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(DIR_PREFIX));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a.0).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        drop(a);
        drop(b);
        assert_eq!(lv_dirs(tmp.path()), 0);
    }

    /// Windows: DACL geschützt, nur die Benutzer-SID, Besitzer = Benutzer.
    #[cfg(windows)]
    #[test]
    fn windows_dacl_nur_benutzer() {
        let sid = user_sid().unwrap();
        assert!(sid.starts_with("S-1-"));
        let tmp = tempfile::tempdir().unwrap();
        let d = work_dir(tmp.path()).unwrap();
        let out = std::process::Command::new("icacls")
            .arg(&d.0)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        // Nur ein ACE-Eintrag (eine Zeile mit „:(“), keine Vererbung von oben.
        let aces = text.lines().filter(|l| l.contains(":(")).count();
        assert_eq!(aces, 1, "{text}");
        assert!(!text.contains("(I)"), "keine geerbten Einträge: {text}");
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
            lv_dirs(base)
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
            assert!(body_path.contains(DIR_PREFIX));
            assert!(!Path::new(body_path.trim()).exists(), "Body-Datei gelöscht");
            assert_eq!(rest(tmp.path()), 0);
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
        fn antwort_ueber_limit_bricht_ab() {
            let tmp = tempfile::tempdir().unwrap();
            // 2 MiB Ausgabe, danach ein gültiger Status: muss trotzdem scheitern.
            let exe = script(
                tmp.path(),
                "cat >/dev/null; head -c 2097152 /dev/zero; printf '\\n@@LV-HTTP-STATUS@@200'",
            );
            let err = send_with(&exe, tmp.path(), &req(b"x"), Duration::from_secs(10)).unwrap_err();
            assert_eq!(
                err,
                CurlError::Failed("curl-Antwort zu groß, abgebrochen".into())
            );
            assert_eq!(rest(tmp.path()), 0);
        }

        #[test]
        fn antwort_ueber_limit_endlos_wird_beendet() {
            let tmp = tempfile::tempdir().unwrap();
            let exe = script(tmp.path(), "cat >/dev/null; exec cat /dev/zero");
            let t = Instant::now();
            let err = send_with(&exe, tmp.path(), &req(b"x"), Duration::from_secs(30)).unwrap_err();
            assert_eq!(
                err,
                CurlError::Failed("curl-Antwort zu groß, abgebrochen".into())
            );
            assert!(t.elapsed() < Duration::from_secs(10), "vor dem Watchdog");
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
