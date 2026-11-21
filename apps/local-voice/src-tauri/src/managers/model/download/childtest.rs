//! Tests, die haengen koennten, laufen in einem Kindprozess mit harter Frist.
//!
//! Ein Thread mit `recv_timeout` laesst sich nicht beenden: haengt der gepruefte Aufruf (etwa
//! das Oeffnen einer FIFO ohne Gegenseite), bleibt der Thread und mit ihm der Testlauf
//! haengen, und ein unbegrenztes `join` waere dasselbe Problem. Hier startet der Test
//! stattdessen die eigene Test-Binaerdatei neu, nur mit diesem einen Test
//! (`--exact <Name>`), und wartet mit einer Frist. Ist sie um, wird der Kindprozess
//! beendet (`kill`) und eingesammelt (`wait`); der Elternprozess haengt nie.
//!
//! Aufbau eines solchen Tests:
//!
//! ```ignore
//! #[test]
//! fn my_test() {
//!     let Some(work) = childtest::child_dir() else {
//!         return childtest::run(module_path!(), "my_test", Duration::from_secs(20));
//!     };
//!     // ... der eigentliche Test, arbeitet nur in `work` ...
//!     childtest::done(&work);
//! }
//! ```
//!
//! Der Elternprozess legt den Arbeitsordner an und gibt ihn dem Kind ueber eine
//! Umgebungsvariable (sie ist zugleich der Schalter "ich bin das Kind"); so raeumt der
//! Elternprozess auch dann auf, wenn er das Kind beenden musste. Das Kind schreibt am
//! Ende eine Marke ([`done`]): ohne sie gilt der Lauf als gescheitert, auch bei
//! Erfolgsstatus. Sonst bestaende ein Test, dessen Name nicht stimmt (`0 tests`), ohne
//! etwas geprueft zu haben.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Umgebungsvariable: Arbeitsordner, den der Elternprozess dem Kind gibt.
const CHILD_DIR_VAR: &str = "LVA_TEST_CHILD_DIR";
/// Name der Marke, die das Kind nach getaner Arbeit im Arbeitsordner anlegt.
const DONE_MARK: &str = ".kind-fertig";

/// Wie ein Kindlauf endete.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Erfolgsstatus UND Marke gesetzt.
    Passed,
    /// Kein Erfolgsstatus, oder keine Marke (der Test lief nicht bis zum Ende oder gar
    /// nicht); mit der Ausgabe des Kindes.
    Failed(String),
    /// Die Frist ist abgelaufen; das Kind wurde beendet und eingesammelt.
    TimedOut { pid: u32 },
}

/// Der Arbeitsordner, wenn dieser Prozess das Kind ist; im Elternprozess `None`.
pub(crate) fn child_dir() -> Option<PathBuf> {
    std::env::var_os(CHILD_DIR_VAR).map(PathBuf::from)
}

/// Das Kind meldet: der Test ist bis zum Ende gelaufen.
pub(crate) fn done(work: &Path) {
    std::fs::write(work.join(DONE_MARK), b"").expect("Marke schreiben");
}

/// Der Name, unter dem libtest den Test der Binaerdatei findet: `module_path!()` ohne den
/// Namen der Kiste, dazu der Name der Funktion.
fn test_name(module: &str, function: &str) -> String {
    let inside = module.split_once("::").map_or("", |(_, rest)| rest);
    if inside.is_empty() {
        function.to_string()
    } else {
        format!("{inside}::{function}")
    }
}

/// Startet die eigene Test-Binaerdatei nur mit `module::function` und wartet hoechstens
/// `deadline`. Haengt oder stirbt das Kind: nach der Frist `kill` und `wait`.
pub(crate) fn run_child(module: &str, function: &str, deadline: Duration) -> Outcome {
    let work = tempfile::tempdir().expect("Arbeitsordner");
    let log = tempfile::NamedTempFile::new().expect("Ausgabedatei");
    let exe = std::env::current_exe().expect("eigene Binaerdatei");
    // Ein Handle, zweimal dupliziert: stdout und stderr teilen sich die Schreibposition.
    let stdout = log.reopen().expect("Ausgabe");
    let stderr = stdout.try_clone().expect("Ausgabe");
    let mut child = Command::new(exe)
        .args(["--exact", &test_name(module, function), "--test-threads=1"])
        .env(CHILD_DIR_VAR, work.path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("Kindprozess starten");
    let pid = child.id();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("Kindstatus") {
            break Some(status);
        }
        if started.elapsed() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        return Outcome::TimedOut { pid };
    };
    if status.success() && work.path().join(DONE_MARK).exists() {
        return Outcome::Passed;
    }
    let output = std::fs::read_to_string(log.path()).unwrap_or_default();
    Outcome::Failed(format!(
        "Status {:?}, Marke {}: {output}",
        status.code(),
        if work.path().join(DONE_MARK).exists() {
            "gesetzt"
        } else {
            "fehlt (Test nicht gelaufen oder nicht zu Ende)"
        }
    ))
}

/// [`run_child`], als Aussage des Elternprozesses: bestanden, sonst Fehlschlag mit der
/// Ausgabe des Kindes beziehungsweise dem Hinweis auf die Frist.
pub(crate) fn run(module: &str, function: &str, deadline: Duration) {
    match run_child(module, function, deadline) {
        Outcome::Passed => {}
        Outcome::Failed(why) => panic!("Kindlauf gescheitert: {why}"),
        Outcome::TimedOut { pid } => panic!(
            "Kindlauf haengt: nach {deadline:?} beendet (Prozess {pid}); der gepruefte Aufruf blockiert"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gegenstueck des Kindes im Test unten: haengt endlos, wenn es als Kind laeuft, und
    /// ist im Elternlauf (gewoehnlicher Testlauf) ohne Wirkung.
    #[test]
    fn a_child_that_hangs_forever() {
        if child_dir().is_none() {
            return;
        }
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    /// Gegenstueck des Kindes: laeuft bis zum Ende und meldet es.
    #[test]
    fn a_child_that_finishes() {
        if let Some(work) = child_dir() {
            done(&work);
        }
    }

    /// Ein Kind, das haengt, wird nach der Frist beendet und eingesammelt: der Elternlauf
    /// kehrt in der Frist plus Spielraum zurueck, meldet `TimedOut`, und der Prozess
    /// gibt es nicht mehr.
    #[test]
    fn a_hanging_child_is_killed_at_the_deadline() {
        let started = Instant::now();
        let outcome = run_child(
            module_path!(),
            "a_child_that_hangs_forever",
            Duration::from_secs(2),
        );
        let Outcome::TimedOut { pid } = outcome else {
            panic!("erwartet TimedOut, war {outcome:?}");
        };
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "der Elternlauf wartete weit ueber die Frist"
        );
        // Eingesammelt: kein Prozess dieser Nummer mehr (kill mit Signal 0 prueft nur).
        // SAFETY: Signal 0 sendet nichts, es prueft nur, ob der Prozess existiert.
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        assert!(!alive, "der Kindprozess lebt noch");
    }

    /// Ein Kind, das zu Ende laeuft, ist bestanden.
    #[test]
    fn a_finishing_child_passes() {
        assert_eq!(
            run_child(
                module_path!(),
                "a_child_that_finishes",
                Duration::from_secs(60)
            ),
            Outcome::Passed
        );
    }

    /// Ein Name, den libtest nicht findet (`0 tests`, Erfolgsstatus), ist KEIN Bestehen:
    /// die Marke fehlt.
    #[test]
    fn a_child_that_runs_no_test_is_a_failure() {
        let outcome = run_child(
            module_path!(),
            "es_gibt_diesen_test_nicht",
            Duration::from_secs(60),
        );
        let Outcome::Failed(why) = outcome else {
            panic!("erwartet Failed, war {outcome:?}");
        };
        assert!(why.contains("Marke fehlt"), "{why}");
    }

    #[test]
    fn the_test_name_drops_the_crate_and_keeps_the_modules() {
        assert_eq!(
            test_name("local_voice_ai_lib::a::b::tests", "t"),
            "a::b::tests::t"
        );
        assert_eq!(test_name("kiste", "t"), "t");
    }
}
