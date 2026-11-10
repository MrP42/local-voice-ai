//! Startet den Extraktor `lva-extrakt` als eigenen Unterprozess mit Grenzen (Spec §4.1, ADR-0029 E9).
//!
//! * Zeit: nach [`Grenzen::zeit`] (Standard 60 s) wird der Prozess beendet.
//! * Speicher: Linux (sonstiges Unix) `RLIMIT_AS` im Kind (Standard 1 GiB) und eigene Prozessgruppe, die
//!   bei Zeitablauf oder Abbruch als Ganzes `SIGKILL` erhält; Windows Job-Objekt mit Speichergrenze
//!   (`process_guard::ProcessGuard`, `JOB_OBJECT_LIMIT_JOB_MEMORY`, `KILL_ON_JOB_CLOSE`), angehalten
//!   gestartet und erst nach der Zuordnung fortgesetzt.
//!   macOS: `RLIMIT_AS` ist dort nicht setzbar (lva#462). Jeder Prozess hat unter macOS rund 32 GiB
//!   virtuellen Adressraum, und `setrlimit(RLIMIT_AS)` scheitert im Kind mit `EINVAL`, sobald das Limit
//!   unter dieser Größe liegt (gemessen: Schwelle = virtuelle Größe des Elternprozesses; ein Limit von
//!   1 GiB lässt sich nie setzen, der Start schlüge mit `extrakt.start` fehl). Die Grenze gilt dort
//!   stattdessen im Elternprozess: dieselbe Warteschleife, die auf das Ende des Kindes wartet, liest alle
//!   20 ms den Speicherbedarf des Kindes (`proc_pid_rusage`, größerer Wert aus physischem Fußabdruck und
//!   resident size) und beendet bei Überschreitung die Prozessgruppe (`SIGKILL`, Fehlercode wie beim
//!   Absturz am Limit: `extrakt.abbruch`). Kein eigener Thread: der Wächter kann das Kind nicht überleben
//!   und nie eine schon geerntete, womöglich neu vergebene PID treffen. Lässt sich der Speicherbedarf des
//!   lebenden Kindes mehrfach hintereinander nicht lesen, endet der Lauf geschlossen mit
//!   `extrakt.schutz` (wie unter Windows, wenn das Job-Objekt fehlt). Rest-Risiko: gemessen wird nur das
//!   Kind selbst, nicht dessen Enkel (der Extraktor startet keine; die Zeitgrenze fängt den Rest ab).
//!   Weiche Grenze: anders als `RLIMIT_AS` greift der Wächter erst bei der nächsten Messung; ein sehr
//!   schnell belegendes Kind kann die Grenze um Belegungsrate × 20 ms überschreiten (bis einige hundert
//!   MiB). Außerdem misst macOS physischen Speicher, Linux virtuellen; die 1-GiB-Grenze ist am Mac also laxer.
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
        #[cfg(not(target_os = "macos"))]
        let bytes = grenzen.speicher_mb.saturating_mul(1024 * 1024);
        let sekunden = grenzen.zeit.as_secs().saturating_add(5);
        // SAFETY: zwischen fork und exec nur async-signal-sichere Aufrufe (setrlimit), keine Allokation.
        unsafe {
            cmd.pre_exec(move || {
                // macOS: kein `RLIMIT_AS` (EINVAL, siehe Kopf der Datei); dort liest der Elternprozess.
                #[cfg(not(target_os = "macos"))]
                {
                    let as_lim = libc::rlimit {
                        rlim_cur: bytes as libc::rlim_t,
                        rlim_max: bytes as libc::rlim_t,
                    };
                    if libc::setrlimit(libc::RLIMIT_AS, &as_lim) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
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
    #[cfg(target_os = "macos")]
    let mut waechter = mac_waechter::Waechter::neu(grenzen.speicher_mb);
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
        // macOS: Speichergrenze im Elternprozess. Das Kind lebt hier noch und ist nicht geerntet
        // (`try_wait` eben `None`), seine PID ist also nicht neu vergeben.
        #[cfg(target_os = "macos")]
        match waechter.pruefen(child.id()) {
            mac_waechter::Urteil::Ok => {}
            urteil => {
                beenden(&mut child);
                let _ = leser.join();
                return Err(match urteil {
                    mac_waechter::Urteil::Zuviel => "extrakt.abbruch".into(),
                    _ => "extrakt.schutz".into(),
                });
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

/// Speicherwächter für macOS (lva#462), siehe Kopf der Datei.
#[cfg(target_os = "macos")]
mod mac_waechter {
    /// So viele aufeinanderfolgende Fehlmessungen bei lebendem Kind (je ein Durchlauf der Warteschleife,
    /// rund 20 ms) gelten als „nicht messbar“; erst dann bricht der Lauf geschlossen ab. Eine einzelne
    /// Fehlmessung ist unschädlich (z. B. vorübergehender Fehler der API); ein zwischen `try_wait` und
    /// Messung beendetes Kind ist als Zombie weiter messbar (`proc_pid_rusage` liefert dann den Endstand).
    pub(super) const MAX_FEHLMESSUNGEN: u32 = 10;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Urteil {
        Ok,
        /// Speicherbedarf über der Grenze: Prozessgruppe beenden, `extrakt.abbruch`.
        Zuviel,
        /// Speicherbedarf des lebenden Kindes dauerhaft nicht lesbar: geschlossen abbrechen,
        /// `extrakt.schutz` (vorübergehend, wiederholbar).
        Unmessbar,
    }

    pub(super) struct Waechter {
        grenze: u64,
        fehlmessungen: u32,
    }

    impl Waechter {
        pub(super) fn neu(speicher_mb: u64) -> Self {
            Self {
                grenze: speicher_mb.saturating_mul(1024 * 1024),
                fehlmessungen: 0,
            }
        }

        /// Misst das (nicht geerntete, lebende) Kind und urteilt. Keine Allokation.
        pub(super) fn pruefen(&mut self, pid: u32) -> Urteil {
            urteil(belegt(pid), self.grenze, &mut self.fehlmessungen)
        }
    }

    /// Entscheidung aus einer Messung; rein, damit sie ohne Prozess prüfbar ist.
    pub(super) fn urteil(messung: Option<u64>, grenze: u64, fehlmessungen: &mut u32) -> Urteil {
        match messung {
            Some(bytes) => {
                *fehlmessungen = 0;
                if bytes > grenze {
                    Urteil::Zuviel
                } else {
                    Urteil::Ok
                }
            }
            None => {
                *fehlmessungen = fehlmessungen.saturating_add(1);
                if *fehlmessungen >= MAX_FEHLMESSUNGEN {
                    Urteil::Unmessbar
                } else {
                    Urteil::Ok
                }
            }
        }
    }

    /// Speicherbedarf des Prozesses in Bytes: der größere Wert aus physischem Fußabdruck (zählt auch
    /// komprimierte Seiten, die `resident size` nicht mehr enthält) und `resident size`.
    /// `None`, wenn der Prozess nicht (mehr) lesbar ist (beendet, geerntet, keine Berechtigung).
    pub(super) fn belegt(pid: u32) -> Option<u64> {
        let pid = libc::c_int::try_from(pid).ok()?;
        // SAFETY: `proc_pid_rusage` füllt höchstens `size_of::<rusage_info_v2>()` Bytes des Puffers, den
        // wir mit passender Größe und Ausrichtung bereitstellen; ein Fehler wird am Rückgabewert erkannt
        // und der Puffer dann nicht gelesen.
        let info = unsafe {
            let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
            let rc = libc::proc_pid_rusage(
                pid,
                libc::RUSAGE_INFO_V2,
                info.as_mut_ptr() as *mut libc::rusage_info_t,
            );
            if rc != 0 {
                return None;
            }
            info.assume_init()
        };
        Some(info.ri_phys_footprint.max(info.ri_resident_size))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const MB: u64 = 1024 * 1024;

        #[test]
        fn urteil_vergleicht_mit_der_grenze_und_setzt_fehlmessungen_zurueck() {
            let mut f = 5;
            assert_eq!(urteil(Some(100 * MB), 100 * MB, &mut f), Urteil::Ok);
            assert_eq!(f, 0);
            assert_eq!(urteil(Some(100 * MB + 1), 100 * MB, &mut f), Urteil::Zuviel);
            assert_eq!(urteil(Some(0), 0, &mut f), Urteil::Ok);
            assert_eq!(urteil(Some(1), 0, &mut f), Urteil::Zuviel);
        }

        #[test]
        fn einzelne_fehlmessungen_sind_normal_erst_die_serie_bricht_ab() {
            let mut f = 0;
            for i in 1..MAX_FEHLMESSUNGEN {
                assert_eq!(urteil(None, MB, &mut f), Urteil::Ok, "{i}");
            }
            assert_eq!(urteil(None, MB, &mut f), Urteil::Unmessbar);
            assert_eq!(urteil(None, MB, &mut f), Urteil::Unmessbar);
            // Eine gelungene Messung dazwischen beginnt die Zählung neu.
            let mut f = MAX_FEHLMESSUNGEN - 1;
            assert_eq!(urteil(Some(0), MB, &mut f), Urteil::Ok);
            assert_eq!(urteil(None, MB, &mut f), Urteil::Ok);
            // Kein Überlauf der Zählung.
            let mut f = u32::MAX;
            assert_eq!(urteil(None, MB, &mut f), Urteil::Unmessbar);
        }

        #[test]
        fn messung_folgt_dem_speicher_eines_kindes_geerntete_pid_ist_none() {
            let b = belegt(std::process::id()).expect("eigener Prozess muss messbar sein");
            assert!(b > MB && b < (1 << 40), "{b}");
            // Ein Kind mit rund 120 MiB berührtem Speicher (je Prozess eigen, daher unabhängig von
            // parallel laufenden Tests): die Messung muss das sehen.
            let mut k = std::process::Command::new("/usr/bin/perl")
                .args(["-e", "my $x = 'a' x (120 * 1024 * 1024); sleep 30;"])
                .spawn()
                .unwrap();
            let pid = k.id();
            let start = std::time::Instant::now();
            let mut gemessen = 0;
            while gemessen <= 100 * MB && start.elapsed() < std::time::Duration::from_secs(20) {
                gemessen = belegt(pid).unwrap_or(0);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let _ = k.kill();
            k.wait().unwrap();
            assert!(gemessen > 100 * MB, "gemessen: {gemessen}");
            // Geerntetes Kind: nicht mehr lesbar (None), kein Absturz, kein Fremdprozess.
            assert_eq!(belegt(pid), None);
            // Ungültige PIDs.
            assert_eq!(belegt(0x7fff_fff0), None);
            assert_eq!(belegt(u32::MAX), None);
        }

        #[test]
        fn waechter_urteilt_ueber_ein_lebendes_kind() {
            let mut k = std::process::Command::new("/bin/sleep")
                .arg("5")
                .spawn()
                .unwrap();
            let mut w = Waechter::neu(1024);
            assert_eq!(w.pruefen(k.id()), Urteil::Ok);
            let mut klein = Waechter::neu(0);
            assert_eq!(klein.pruefen(k.id()), Urteil::Zuviel);
            let _ = k.kill();
            let _ = k.wait();
        }
    }
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
