//! Fake-`ptts` für die Phonon-Tests (nur Tests, Paket P2).
//!
//! Die Tests brauchen ein ECHTES Programm: nur so lässt sich prüfen, dass der
//! Adapter den Text unter Windows-Quoting unverändert übergibt, dass die
//! Umgebung stimmt und dass ein Abbruch den Kindprozess wirklich beendet.
//! Ein `.cmd`/`.bat` taugt dafür nicht (Rust verweigert Sonderzeichen in
//! Argumenten von Batchdateien, `cmd.exe` zerlegt Anführungszeichen anders
//! als ein Programm). Kein Python, kein heruntergeladenes Binary.
//!
//! Darum wird ein winziges Rust-Programm ([`FAKE_SRC`], nur `std`) einmal je
//! Quelltext mit `rustc` übersetzt (Ablage im Temp-Ordner, Name nach dem
//! Hash des Quelltexts) und je Testfall samt `fake.cfg` in einen eigenen
//! Ordner als `ptts[.exe]` kopiert. Das Verhalten steuert die `fake.cfg`
//! (`Schlüssel=Wert`, `\n` im Wert = Zeilenumbruch):
//!
//! | Schlüssel | Wirkung |
//! |---|---|
//! | `log` | Datei, in die jeder Aufruf Argumente (`arg=…`) und Umgebung (`env=NAME=Wert`) schreibt |
//! | `version_out` | Ausgabe von `--version` (Standard `ptts 0.4.2`) |
//! | `version_code`, `version_delay_ms` | `--version` endet mit diesem Code bzw. schläft vorher (danach entsteht der Marker) |
//! | `voices_out` | Ausgabe von `--list-voices` |
//! | `delay_ms` | so lange schlafen, bevor etwas geschrieben wird |
//! | `marker` | Datei, die NACH der Verzögerung entsteht (fehlt sie, wurde der Prozess vorher beendet) |
//! | `fail_code`, `fail_stderr` | mit diesem Code und dieser Meldung auf stderr enden |
//! | `mode` | `no_output` (Erfolg ohne Datei), `bad_wav` (Müll), `empty_wav` (Kopf ohne Daten), `streaming_header` (Längenfelder `0xFFFFFFFF`) |
//!
//! Der Fake verhält sich wie ein `clap`-CLI: ein Argument mit führendem `-`
//! außerhalb eines Optionswerts ist ein Fehler (Exit 2) — so fällt ein
//! Adapter durch, der `-foo` als Text durchreicht.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tempfile::TempDir;

use super::phonon::fnv1a64;

/// Quelltext des Fake-Programms (nur `std`, Edition 2021).
const FAKE_SRC: &str = r####"
use std::io::Write;

fn env_or_unset(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| "unset".to_string())
}

fn main() {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().to_path_buf();
    let cfg = std::fs::read_to_string(dir.join("fake.cfg")).unwrap_or_default();
    let get = |k: &str| -> Option<String> {
        let prefix = format!("{k}=");
        cfg.lines()
            .find_map(|l| l.strip_prefix(prefix.as_str()).map(|v| v.replace("\\n", "\n")))
    };
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    if let Some(log) = get("log") {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(f, "--- pid={}", std::process::id());
            for a in &args {
                let _ = writeln!(f, "arg={a}");
            }
            for k in ["HF_HUB_OFFLINE", "HF_HOME"] {
                let _ = writeln!(f, "env={k}={}", env_or_unset(k));
            }
        }
    }

    if args.iter().any(|a| a == "--version") {
        if let Some(ms) = get("version_delay_ms") {
            std::thread::sleep(std::time::Duration::from_millis(ms.parse().unwrap_or(0)));
            if let Some(m) = get("marker") {
                let _ = std::fs::write(m, "done");
            }
        }
        println!("{}", get("version_out").unwrap_or_else(|| "ptts 0.4.2".to_string()));
        if let Some(c) = get("version_code") {
            eprintln!("version failed");
            std::process::exit(c.parse().unwrap_or(1));
        }
        return;
    }
    if args.iter().any(|a| a == "--list-voices") {
        print!("{}", get("voices_out").unwrap_or_else(|| "Ava de\nBen en\n".to_string()));
        return;
    }

    let with_value = ["--model", "--lang", "--quant", "--voice", "--threads", "-o"];
    let mut out: Option<String> = None;
    let mut text: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if with_value.contains(&a.as_str()) {
            if a == "-o" {
                out = args.get(i + 1).cloned();
            }
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            eprintln!("error: unexpected argument '{a}' found");
            std::process::exit(2);
        }
        if text.is_some() {
            eprintln!("error: unexpected extra positional argument '{a}'");
            std::process::exit(2);
        }
        text = Some(a.clone());
        i += 1;
    }
    let Some(text) = text else {
        eprintln!("error: the following required arguments were not provided: <TEXT>");
        std::process::exit(2);
    };
    let Some(out) = out else {
        eprintln!("error: missing -o");
        std::process::exit(2);
    };

    if let Some(ms) = get("delay_ms") {
        std::thread::sleep(std::time::Duration::from_millis(ms.parse().unwrap_or(0)));
    }
    if let Some(m) = get("marker") {
        let _ = std::fs::write(m, "done");
    }
    if let Some(code) = get("fail_code") {
        eprintln!("{}", get("fail_stderr").unwrap_or_default());
        std::process::exit(code.parse().unwrap_or(1));
    }

    let mode = get("mode").unwrap_or_default();
    if mode == "no_output" {
        return;
    }
    if mode == "bad_wav" {
        let _ = std::fs::write(&out, b"das ist keine WAV-Datei, nur Text");
        return;
    }

    // 24 kHz, mono, 16 Bit; 40 ms je Zeichen, Sinus.
    let chars = text.trim().chars().count().max(1);
    let frames = if mode == "empty_wav" { 0 } else { 24 * 40 * chars };
    let mut pcm: Vec<u8> = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let v = ((i as f32 * 0.05).sin() * 8000.0) as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    let streaming = mode == "streaming_header";
    let (riff, data) = if streaming {
        (u32::MAX, u32::MAX)
    } else {
        (36 + pcm.len() as u32, pcm.len() as u32)
    };
    let mut wav: Vec<u8> = b"RIFF".to_vec();
    wav.extend(riff.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(24_000u32.to_le_bytes());
    wav.extend((24_000u32 * 2).to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data.to_le_bytes());
    wav.extend(pcm);
    if let Err(e) = std::fs::write(&out, wav) {
        eprintln!("error: cannot write {out}: {e}");
        std::process::exit(3);
    }
}
"####;

/// Ordner mit Fake-`ptts`, Konfiguration, Protokoll und Marker.
pub(crate) struct FakePtts {
    /// Wird mit dem Wert gelöscht.
    pub dir: TempDir,
    pub exe: PathBuf,
    pub log: PathBuf,
    pub marker: PathBuf,
}

/// Verhalten des Fakes (siehe Tabelle in der Moduldoku).
#[derive(Default, Clone)]
pub(crate) struct FakeOpts {
    pub delay_ms: Option<u64>,
    pub fail_code: Option<i32>,
    pub fail_stderr: Option<String>,
    pub version_out: Option<String>,
    pub version_code: Option<i32>,
    pub version_delay_ms: Option<u64>,
    pub voices_out: Option<String>,
    pub mode: Option<String>,
}

impl FakeOpts {
    pub fn slow() -> Self {
        Self {
            delay_ms: Some(1500),
            ..Default::default()
        }
    }

    pub fn failing(code: i32, stderr: &str) -> Self {
        Self {
            fail_code: Some(code),
            fail_stderr: Some(stderr.to_string()),
            ..Default::default()
        }
    }

    pub fn mode(mode: &str) -> Self {
        Self {
            mode: Some(mode.to_string()),
            ..Default::default()
        }
    }
}

fn rustc_path() -> PathBuf {
    if let Some(p) = std::env::var_os("RUSTC") {
        return PathBuf::from(p);
    }
    if let Some(cargo) = std::env::var_os("CARGO") {
        let sibling =
            Path::new(&cargo).with_file_name(if cfg!(windows) { "rustc.exe" } else { "rustc" });
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from("rustc")
}

/// Das übersetzte Fake-Programm (einmal je Prozess und Quelltext).
fn compiled_template() -> &'static PathBuf {
    static TEMPLATE: OnceLock<PathBuf> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let base = std::env::temp_dir().join(format!("lva-fake-ptts-{:016x}", fnv1a64(FAKE_SRC)));
        std::fs::create_dir_all(&base).expect("Temp-Ordner für das Fake-ptts");
        let name = if cfg!(windows) {
            "fake_ptts.exe"
        } else {
            "fake_ptts"
        };
        let exe = base.join(name);
        if exe.is_file() {
            return exe;
        }
        let pid = std::process::id();
        let src = base.join(format!("fake_ptts_{pid}.rs"));
        std::fs::write(&src, FAKE_SRC).unwrap();
        let tmp = base.join(if cfg!(windows) {
            format!("fake_ptts_{pid}.exe")
        } else {
            format!("fake_ptts_{pid}")
        });
        let out = std::process::Command::new(rustc_path())
            .args(["--edition", "2021", "-C", "debuginfo=0", "-o"])
            .arg(&tmp)
            .arg(&src)
            .output()
            .unwrap_or_else(|e| {
                panic!("rustc für das Fake-ptts nicht startbar ({e}); RUSTC setzen oder rustc in den PATH legen")
            });
        assert!(
            out.status.success(),
            "Fake-ptts lässt sich nicht übersetzen:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        // Zwei Testprozesse dürfen gleichzeitig übersetzen; der zweite Rename
        // scheitert dann harmlos, die Datei ist ja da.
        let _ = std::fs::rename(&tmp, &exe);
        let _ = std::fs::remove_file(&src);
        if !exe.is_file() {
            return tmp;
        }
        let _ = std::fs::remove_file(&tmp);
        exe
    })
}

/// Ein Fake-`ptts` mit eigenem Ordner anlegen.
pub(crate) fn fake_ptts(opts: FakeOpts) -> FakePtts {
    let dir = TempDir::new().expect("Temp-Ordner");
    let name = if cfg!(windows) { "ptts.exe" } else { "ptts" };
    let exe = dir.path().join(name);
    std::fs::copy(compiled_template(), &exe).expect("Fake-ptts kopieren");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let log = dir.path().join("calls.log");
    let marker = dir.path().join("finished.marker");
    let mut cfg = format!("log={}\nmarker={}\n", log.display(), marker.display());
    let mut put = |k: &str, v: String| cfg.push_str(&format!("{k}={}\n", v.replace('\n', "\\n")));
    if let Some(v) = opts.delay_ms {
        put("delay_ms", v.to_string());
    }
    if let Some(v) = opts.fail_code {
        put("fail_code", v.to_string());
    }
    if let Some(v) = opts.fail_stderr {
        put("fail_stderr", v);
    }
    if let Some(v) = opts.version_out {
        put("version_out", v);
    }
    if let Some(v) = opts.version_code {
        put("version_code", v.to_string());
    }
    if let Some(v) = opts.version_delay_ms {
        put("version_delay_ms", v.to_string());
    }
    if let Some(v) = opts.voices_out {
        put("voices_out", v);
    }
    if let Some(v) = opts.mode {
        put("mode", v);
    }
    std::fs::write(dir.path().join("fake.cfg"), cfg).unwrap();
    FakePtts {
        dir,
        exe,
        log,
        marker,
    }
}

/// Ein protokollierter Aufruf des Fakes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Call {
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

impl Call {
    /// Wert nach einer Option (`--model` → `models/x`).
    pub fn value_of(&self, flag: &str) -> Option<&str> {
        self.args
            .iter()
            .position(|a| a == flag)
            .and_then(|i| self.args.get(i + 1))
            .map(String::as_str)
    }

    pub fn has(&self, flag: &str) -> bool {
        self.args.iter().any(|a| a == flag)
    }

    /// Das Positionsargument (der Text): das einzige Argument, das weder
    /// Option noch Optionswert ist.
    pub fn text(&self) -> Option<&str> {
        let with_value = ["--model", "--lang", "--quant", "--voice", "--threads", "-o"];
        let mut i = 0;
        while i < self.args.len() {
            let a = self.args[i].as_str();
            if with_value.contains(&a) {
                i += 2;
            } else if a.starts_with('-') {
                i += 1;
            } else {
                return Some(a);
            }
        }
        None
    }
}

impl FakePtts {
    /// Alle bisherigen Aufrufe (leer, wenn der Fake nie lief).
    pub fn calls(&self) -> Vec<Call> {
        let Ok(text) = std::fs::read_to_string(&self.log) else {
            return Vec::new();
        };
        let mut calls: Vec<Call> = Vec::new();
        for line in text.lines() {
            if line.starts_with("--- pid=") {
                calls.push(Call::default());
            } else if let Some(c) = calls.last_mut() {
                if let Some(a) = line.strip_prefix("arg=") {
                    c.args.push(a.to_string());
                } else if let Some(e) = line.strip_prefix("env=") {
                    if let Some((k, v)) = e.split_once('=') {
                        c.env.insert(k.to_string(), v.to_string());
                    }
                }
            }
        }
        calls
    }

    /// Nur die Syntheseaufrufe (ohne `--version`/`--list-voices`).
    pub fn synth_calls(&self) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| !c.has("--version") && !c.has("--list-voices"))
            .collect()
    }
}
