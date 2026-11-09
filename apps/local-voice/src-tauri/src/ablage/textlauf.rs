//! Startet den Extraktor `lva-extrakt` als eigenen Unterprozess mit Grenzen (Spec §4.1, ADR-0029 E9).
//!
//! * Zeit: nach [`Grenzen::zeit`] (Standard 60 s) wird der Prozess beendet.
//! * Speicher: Unix `RLIMIT_AS` im Kind (Standard 1 GiB) und eigene Prozessgruppe, die bei Zeitablauf
//!   oder Abbruch als Ganzes `SIGKILL` erhält; Windows Job-Objekt mit Speichergrenze
//!   (`process_guard::ProcessGuard`, `JOB_OBJECT_LIMIT_JOB_MEMORY`, `KILL_ON_JOB_CLOSE`), angehalten
//!   gestartet und erst nach der Zuordnung fortgesetzt.
//!   macOS nimmt `RLIMIT_AS` hin, setzt es aber nicht verlässlich durch; dort begrenzen Zeitgrenze und
//!   Prozessgruppe (offen: Speicherwächter je Kind, falls nötig).
//! * Kein Netz: der Extraktor enthält keinen Netzcode und bekommt nur den Pfad der einen Fassung. Eine
//!   Netzsperre des Betriebssystems (Namensräume, Firewall-Regel) setzt diese Stufe nicht.
//! * Ausgabe: stdout höchstens [`MAX_AUSGABE`] (mehr → `extrakt.zu_gross`, der Prozess wird sofort
//!   beendet); stderr wird verworfen (nie Inhalt im Log).
//! * Aufruf: zufälliges Kennzeichen in der Umgebung und als erste Zeile auf stdin, dazu der Ablageordner
//!   ([`super::extrakt::zugelassen`]); ohne beides antwortet der Extraktor nicht.
//! * Jeder Ausgang nach dem Start beendet und erntet das Kind (samt Prozessgruppe).
//!
//! Programm: `lva-extrakt[.exe]` neben dem App-Programm, sonst das App-Programm selbst mit
//! [`super::extrakt::SCHALTER`] (gebündelte App ohne eigenes Hilfsprogramm).

use super::extrakt::{Antwort, Textfassung, ENV_KENNZEICHEN, ENV_WURZEL, SCHALTER};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const MAX_AUSGABE: u64 = 96 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grenzen {
    pub zeit: Duration,
    pub speicher_mb: u64,
}

pub const STANDARD: Grenzen = Grenzen {
    zeit: Duration::from_secs(60),
    speicher_mb: 1024,
};

/// Programm und feste Argumente vor `<pfad> <sha256> <typ>`.
#[derive(Debug, Clone)]
pub struct Programm {
    pub pfad: PathBuf,
    pub vorweg: Vec<OsString>,
    pub umgebung: Vec<(OsString, OsString)>,
}

impl Programm {
    /// Eigenes Hilfsprogramm neben dem App-Programm, sonst das App-Programm mit Schalter.
    pub fn finden() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|_| "extrakt.programm".to_string())?;
        let name = if cfg!(windows) {
            "lva-extrakt.exe"
        } else {
            "lva-extrakt"
        };
        if let Some(neben) = exe.parent().map(|d| d.join(name)) {
            if neben.is_file() {
                return Ok(Self {
                    pfad: neben,
                    vorweg: vec![],
                    umgebung: vec![],
                });
            }
        }
        Ok(Self {
            pfad: exe,
            vorweg: vec![SCHALTER.into()],
            umgebung: vec![],
        })
    }
}

fn kennzeichen() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    super::extrakt::hex(&b)
}

/// Ergebnis mit stabilem Fehlercode (`extrakt.zeit`, `extrakt.abbruch`, `extrakt.zip_grenze`, …).
/// `wurzel` ist der Ablageordner; `datei` muss darunter liegen.
pub fn ausfuehren(
    prog: &Programm,
    wurzel: &Path,
    datei: &Path,
    sha256: &str,
    typ: &str,
    grenzen: Grenzen,
    abbrechen: &AtomicBool,
) -> Result<Textfassung, String> {
    crate::process_guard::check_launchable(&prog.pfad).map_err(|_| "extrakt.programm")?;
    let kennung = kennzeichen();
    let mut cmd = Command::new(&prog.pfad);
    cmd.args(&prog.vorweg)
        .arg(datei)
        .arg(sha256)
        .arg(typ)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env(ENV_KENNZEICHEN, &kennung)
        .env(ENV_WURZEL, wurzel);
    for (k, v) in &prog.umgebung {
        cmd.env(k, v);
    }
    // Arbeitsordner ist weder der Ordner der Datei noch die Ablagewurzel: unter Windows hält das
    // Arbeitsverzeichnis eines Prozesses seinen Ordner ohne FILE_SHARE_DELETE offen, und ein
    // gleichzeitiges Löschen oder Umbenennen (auch der Wurzel) schlüge dort fehl. Die Wurzel kommt
    // über die Umgebung ([`ENV_WURZEL`]).
    cmd.current_dir(std::env::temp_dir());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW und angehalten: erst nach der Job-Zuordnung läuft der Extraktor an.
        cmd.creation_flags(0x0800_0000 | crate::process_guard::CREATE_SUSPENDED);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        let bytes = grenzen.speicher_mb.saturating_mul(1024 * 1024);
        let sekunden = grenzen.zeit.as_secs().saturating_add(5);
        // SAFETY: zwischen fork und exec nur async-signal-sichere Aufrufe (setrlimit), keine Allokation.
        unsafe {
            cmd.pre_exec(move || {
                let as_lim = libc::rlimit {
                    rlim_cur: bytes as libc::rlim_t,
                    rlim_max: bytes as libc::rlim_t,
                };
                if libc::setrlimit(libc::RLIMIT_AS, &as_lim) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // Rückhalt zur Zeitgrenze: CPU-Zeit (der Elternprozess beendet ohnehin nach `zeit`).
                let cpu = libc::rlimit {
                    rlim_cur: sekunden as libc::rlim_t,
                    rlim_max: sekunden as libc::rlim_t,
                };
                let _ = libc::setrlimit(libc::RLIMIT_CPU, &cpu);
                // Keine Kerndateien mit Inhalt.
                let kern = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                let _ = libc::setrlimit(libc::RLIMIT_CORE, &kern);
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn().map_err(|_| "extrakt.start".to_string())?;
    #[cfg(windows)]
    let _job = {
        let job = crate::process_guard::ProcessGuard::attach(
            &child,
            Some(grenzen.speicher_mb),
            crate::process_guard::CPU_CAP_PERCENT,
        );
        let fortgesetzt = match job {
            Some(_) => crate::process_guard::resume_suspended(&child),
            None => Err("kein Job-Objekt".to_string()),
        };
        if fortgesetzt.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("extrakt.schutz".into());
        }
        job
    };
    // Ab hier beendet und erntet jeder Ausgang das Kind (auch ein früher Fehler).
    let (Some(mut stdin), Some(mut stdout)) = (child.stdin.take(), child.stdout.take()) else {
        beenden(&mut child);
        return Err("extrakt.start".into());
    };
    // Kennzeichen als erste Zeile; danach stdin schließen (65 Bytes passen in jeden Pipe-Puffer).
    let geschrieben = stdin
        .write_all(format!("{kennung}\n").as_bytes())
        .and_then(|_| stdin.flush());
    drop(stdin);
    if geschrieben.is_err() {
        beenden(&mut child);
        return Err("extrakt.start".into());
    }
    let zu_viel = std::sync::Arc::new(AtomicBool::new(false));
    let zu_viel_leser = zu_viel.clone();
    let leser = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let r = (&mut stdout).take(MAX_AUSGABE + 1).read_to_end(&mut buf);
        if buf.len() as u64 > MAX_AUSGABE {
            // Sofort melden; stdout wird mit dem Ende des Threads geschlossen, das Kind nicht abgewartet.
            zu_viel_leser.store(true, Ordering::Relaxed);
            return (false, Vec::new());
        }
        (r.is_ok(), buf)
    });
    let ende = Instant::now() + grenzen.zeit;
    let status = loop {
        if zu_viel.load(Ordering::Relaxed) {
            beenden(&mut child);
            let _ = leser.join();
            return Err("extrakt.zu_gross".into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {}
            Err(_) => {
                beenden(&mut child);
                let _ = leser.join();
                return Err("extrakt.programm".into());
            }
        }
        let abgebrochen = abbrechen.load(Ordering::Relaxed);
        if abgebrochen || Instant::now() >= ende {
            beenden(&mut child);
            let _ = leser.join();
            return Err(if abgebrochen {
                "extrakt.abgebrochen".into()
            } else {
                "extrakt.zeit".into()
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Das Kind ist beendet und geerntet (kein Signal mehr an seine Kennung: sie kann neu vergeben sein).
    let (ok, ausgabe) = leser.join().unwrap_or((false, Vec::new()));
    if zu_viel.load(Ordering::Relaxed) || ausgabe.len() as u64 > MAX_AUSGABE {
        return Err("extrakt.zu_gross".into());
    }
    if !ok {
        return Err(unklarer_ausgang(&status).into());
    }
    let antwort = antwort_lesen(&ausgabe);
    match (status.code(), antwort) {
        (Some(0), Some(Antwort::Ok(t))) => pruefen(t, sha256),
        (Some(2), Some(Antwort::Fehler { fehler })) if code_ok(&fehler) => {
            Err(format!("extrakt.{fehler}"))
        }
        _ => Err(unklarer_ausgang(&status).into()),
    }
}

/// Fehlercode für ein Ende ohne gültige Antwort. Endgültig (`extrakt.abbruch`, Giftdatei) nur bei
/// Speicherlimit, Absturz oder Signal (SIGKILL, SIGSEGV, SIGABRT, SIGBUS, SIGILL, SIGFPE, SIGXCPU;
/// Rust-Panic 101 und `abort` 134; Windows: NTSTATUS-Absturzcodes) und beim Thread-Panic (70).
/// Alles andere (nicht zugelassen 64, Thread-Start 71, stdout 74, unklares Ende) liegt nicht am Inhalt:
/// vorübergehend (`extrakt.start` bzw. `extrakt.programm`).
pub(crate) fn unklarer_ausgang(status: &std::process::ExitStatus) -> &'static str {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            const ABSTURZ: [i32; 7] = [4, 6, 7, 8, 9, 11, 24];
            return if ABSTURZ.contains(&sig) {
                "extrakt.abbruch"
            } else {
                "extrakt.programm"
            };
        }
    }
    match status.code() {
        Some(70 | 101 | 134) => "extrakt.abbruch",
        Some(71 | 74) => "extrakt.start",
        #[cfg(windows)]
        Some(c) if (c as u32) >= 0xC000_0000 => "extrakt.abbruch",
        _ => "extrakt.programm",
    }
}

fn code_ok(c: &str) -> bool {
    !c.is_empty() && c.len() <= 32 && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

/// Der Extraktor muss die erwartete Prüfsumme bestätigen und eine passende `textrev` liefern.
fn pruefen(t: Textfassung, sha256: &str) -> Result<Textfassung, String> {
    let rev = super::extrakt::textrev(
        &t.original_sha256,
        &t.extraktor,
        &t.extraktor_version,
        t.marken_version,
    );
    if !t.original_sha256.eq_ignore_ascii_case(sha256)
        || rev != t.textrev
        || t.marken_version != super::extrakt::MARKEN_VERSION
    {
        return Err("extrakt.antwort".into());
    }
    Ok(t)
}

fn antwort_lesen(b: &[u8]) -> Option<Antwort> {
    // Im Testlauf schreibt der Test-Harness des Kindes eine Kopfzeile vor das JSON.
    #[cfg(test)]
    let b = &b[b.iter().position(|&x| x == b'{')?..];
    let mut it = serde_json::Deserializer::from_slice(b).into_iter::<Antwort>();
    it.next()?.ok()
}

fn beenden(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // Ganze Prozessgruppe (das Kind ist deren Anführer, `process_group(0)`).
        // SAFETY: kill mit negativer PID adressiert nur die eigene Gruppe des Kindes.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}
