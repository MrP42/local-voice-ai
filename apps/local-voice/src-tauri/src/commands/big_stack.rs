//! Lange Auftraege auf einem eigenen Thread mit grossem Stack (Hotfix 0.21.1, #70/#73).
//!
//! WARUM: Ein `#[tauri::command] async fn` baut sein Future im Aufruf des Webview auf
//! (`Invoke`), und das laeuft auf dem Haupt-Thread der App. Dort hat Windows nur 1 MiB
//! Stack. Das Future eines Auftrags (Uebersetzen, KI-Notizen, Protokoll) ist aber gross:
//! im Release 0.21.0 waren es 335 KiB (Uebersetzen), 575 KiB (KI-Notizen) und 161 KiB
//! (Protokoll), und die erzeugte Huelle des Commands haelt davon mehrere Kopien auf dem
//! Stack (837 KiB, 1,4 MiB, 402 KiB) plus die Kopie fuer `spawn`. Das passte nicht in
//! 1 MiB: `STATUS_STACK_OVERFLOW` (0xc00000fd) beim Klick, ohne eine Logzeile, weil der
//! Auftrag nie anlief. Belegt durch die Absturzabbilder (`__chkstk` mit 343 240 Byte
//! Rahmen im Haupt-Thread).
//!
//! WIE: Der Command haelt nur noch ein winziges Future (den Empfangskanal). Den Auftrag
//! baut und treibt ein eigener Thread mit [`JOB_STACK_BYTES`] Stack; er lebt genau so
//! lange wie der Auftrag. Ein Future, das den Thread nie verlaesst, braucht kein `Send`.
//!
//! Der Stack wird nur RESERVIERT (Windows `STACK_SIZE_PARAM_IS_A_RESERVATION`, Linux und
//! macOS mmap): belegt wird, was der Auftrag beruehrt. Bei vollem Arbeitsspeicher aendert
//! sich also nichts gegenueber vorher; scheitert der Start des Threads, kommt der Code
//! [`CODE_THREAD`] zurueck statt eines Absturzes.

use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Stack des Auftragsthreads. Reserviert, nicht belegt (siehe oben).
pub const JOB_STACK_BYTES: usize = 64 * 1024 * 1024;

/// Fehlercode, wenn der Thread nicht startete oder der Auftrag abstuerzte (Panik).
pub const CODE_THREAD: &str = "job_thread_failed";

/// Fuehrt `make()` und das Future, das es liefert, auf einem eigenen Thread mit grossem
/// Stack aus. Das zurueckgegebene Future ist klein, egal wie gross der Auftrag ist, und
/// darf deshalb als Future eines Commands durch den Haupt-Thread gereicht werden.
///
/// Der Thread startet sofort (beim Aufruf, nicht erst beim ersten `poll`). Wird das
/// zurueckgegebene Future fallen gelassen, laeuft der Auftrag zu Ende (wie ein
/// gespawnter Task) und sein Ergebnis wird verworfen.
pub fn run<T, M, Fut>(name: &'static str, make: M) -> impl Future<Output = Result<T, String>> + Send
where
    T: Send + 'static,
    M: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = T>,
{
    run_with_stack(name, JOB_STACK_BYTES, make)
}

/// [`run`] mit frei gewaehltem Stack (Tests: Thread-Start scheitert, kleiner Stack).
fn run_with_stack<T, M, Fut>(
    name: &'static str,
    stack_bytes: usize,
    make: M,
) -> impl Future<Output = Result<T, String>> + Send
where
    T: Send + 'static,
    M: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = T>,
{
    let (tx, rx) = tokio::sync::oneshot::channel::<std::thread::Result<T>>();
    let started = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(stack_bytes)
        .spawn(move || {
            // Der Auftrag baut sein Future HIER und treibt es auf diesem Thread: weder
            // Konstruktion noch `poll` beruehren den Stack des Aufrufers.
            let result = catch_unwind(AssertUnwindSafe(|| tauri::async_runtime::block_on(make())));
            let _ = tx.send(result);
        })
        .map(drop);
    async move {
        if let Err(e) = started {
            log::error!("Auftragsthread '{name}' startete nicht: {e}");
            return Err(CODE_THREAD.to_string());
        }
        match rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => {
                // Kein Panik-Text ins Log oder an die Oberflaeche: er koennte Inhalt tragen.
                log::error!("Auftragsthread '{name}' abgestuerzt (Panik)");
                Err(CODE_THREAD.to_string())
            }
            Err(_) => {
                log::error!("Auftragsthread '{name}' endete ohne Ergebnis");
                Err(CODE_THREAD.to_string())
            }
        }
    }
}

/// Wie [`run`] fuer Auftraege, die selbst `Result<T, String>` liefern: ein Fehler des
/// Threads wird zu `Err(CODE_THREAD)`, ein Fehler des Auftrags bleibt, wie er ist.
pub fn run_result<T, M, Fut>(
    name: &'static str,
    make: M,
) -> impl Future<Output = Result<T, String>> + Send
where
    T: Send + 'static,
    M: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, String>>,
{
    let job = run(name, make);
    async move { job.await.and_then(|inner| inner) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Ein Future mit `N` Byte Zustand, wie ein Auftrag mit grossen Puffern/Kopien.
    async fn heavy_job<const N: usize>(value: u32) -> u32 {
        let mut buffer = [0u8; N];
        // Ueber einen Wartepunkt hinweg lebendig: der Puffer gehoert zum Zustand des Futures.
        tokio::task::yield_now().await;
        buffer[N - 1] = 1;
        std::hint::black_box(&mut buffer);
        value + u32::from(buffer[N - 1])
    }

    #[test]
    fn the_future_handed_to_the_caller_stays_tiny_however_big_the_job_is() {
        // Vorbedingung: der Auftrag selbst ist gross (hier 128 KiB, mehr passt nicht auf den
        // Testthread); im Auftragsthread unten sind es 1 MiB.
        let big = heavy_job::<{ 128 * 1024 }>(1);
        assert!(std::mem::size_of_val(&big) >= 128 * 1024);
        drop(big);
        const JOB: usize = 1024 * 1024;
        let handed = run("probe-job", || heavy_job::<JOB>(1));
        assert!(
            std::mem::size_of_val(&handed) <= 256,
            "Command-Future: {} Byte",
            std::mem::size_of_val(&handed)
        );
        let handed_result = run_result("probe-job-result", || async { Ok::<u32, String>(1) });
        assert!(std::mem::size_of_val(&handed_result) <= 256);
        // Aufraeumen: die Threads laufen zu Ende, ihr Ergebnis ist egal.
        let _ = tauri::async_runtime::block_on(handed);
    }

    #[test]
    fn the_job_runs_on_a_thread_with_the_big_stack_and_returns_its_result() {
        // Mehr als der Standard-Stack (2 MiB) und das 1 MiB des Windows-Haupt-Threads.
        let result = tauri::async_runtime::block_on(run("big-stack-probe", || {
            heavy_job::<{ 8 * 1024 * 1024 }>(41)
        }));
        assert_eq!(result, Ok(42));
        let named = tauri::async_runtime::block_on(run("named-probe", || async {
            std::thread::current().name().map(str::to_string)
        }));
        assert_eq!(named, Ok(Some("named-probe".to_string())));
    }

    #[test]
    fn an_error_of_the_job_is_passed_on_and_a_panic_becomes_the_thread_code() {
        let ok: Result<u32, String> =
            tauri::async_runtime::block_on(run_result("ok-probe", || async {
                Ok::<u32, String>(7)
            }));
        assert_eq!(ok, Ok(7));
        let err: Result<u32, String> =
            tauri::async_runtime::block_on(run_result("err-probe", || async {
                Err::<u32, String>("translate_empty".to_string())
            }));
        assert_eq!(
            err,
            Err("translate_empty".to_string()),
            "Auftragsfehler bleiben unveraendert"
        );
        let panicked: Result<u32, String> =
            tauri::async_runtime::block_on(run_result("panic-probe", || async {
                if std::hint::black_box(true) {
                    panic!("absichtlich");
                }
                Ok::<u32, String>(1)
            }));
        assert_eq!(
            panicked,
            Err(CODE_THREAD.to_string()),
            "eine Panik haengt nie den Command auf"
        );
    }

    #[test]
    fn dropping_the_handed_future_lets_the_job_finish_and_nothing_hangs_or_panics() {
        // Fenster neu geladen / Command abgebrochen: der Auftrag laeuft zu Ende (wie ein
        // gespawnter Task), sein Ergebnis wird verworfen, und niemand wartet auf ihn.
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&done);
        let handed = run("drop-probe", move || async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        drop(handed);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "der Auftrag lief nicht zu Ende"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn a_thread_that_cannot_start_gives_the_code_instead_of_a_crash_or_a_hang() {
        // Kein Speicher fuer den Stack (oder keine Threads mehr): der Command bekommt einen
        // Code, der Auftrag lief nie an.
        let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&ran);
        let result: Result<u32, String> = tauri::async_runtime::block_on(run_with_stack(
            "no-start-probe",
            usize::MAX / 4,
            move || async move {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                7
            },
        ));
        assert_eq!(result, Err(CODE_THREAD.to_string()));
        assert!(
            !ran.load(std::sync::atomic::Ordering::SeqCst),
            "der Auftrag darf nicht angelaufen sein"
        );
    }

    #[test]
    fn a_job_that_holds_the_registry_slot_frees_it_when_it_panics() {
        use crate::managers::meetings::job;
        let meeting = format!("panic-slot-{}", std::process::id());
        let emit: job::EmitFn = std::sync::Arc::new(|_| {});
        let probe = meeting.clone();
        let result: Result<(), String> =
            tauri::async_runtime::block_on(run_result("slot-probe", move || async move {
                let _guard = job::global()
                    .try_start(&probe, emit)
                    .map_err(|e| e.to_string())?;
                if std::hint::black_box(true) {
                    panic!("absichtlich");
                }
                Ok(())
            }));
        assert_eq!(result, Err(CODE_THREAD.to_string()));
        assert!(
            !job::global().is_running(&meeting),
            "der Platz der Besprechung ist wieder frei"
        );
    }

    // -- Der Absturz selbst ------------------------------------------------------------
    //
    // Ein Stack-Ueberlauf beendet den ganzen Prozess; deshalb laeuft der Beleg in einem
    // Kindprozess (dieselbe Test-EXE mit gesetzter Umgebungsvariable).

    const CHILD: &str = "LV_BIG_STACK_CHILD";

    const CHILD_JOB: usize = 2 * 1024 * 1024;

    /// Vor 0.21.1: das Future wird auf dem Thread gebaut und getrieben, der den Command aufruft.
    /// 1 MiB: der Haupt-Thread unter Windows. (Eigene Funktion: in einem Debug-Build hat jede
    /// Variable ihren eigenen Platz im Rahmen, ein zweiter Zweig mit dem 2-MiB-Future
    /// im selben Rahmen liesse auch den "reparierten" Weg ueberlaufen.)
    fn legacy_on_main_thread_sized_stack() -> u32 {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                let future = heavy_job::<CHILD_JOB>(1);
                tauri::async_runtime::block_on(future)
            })
            .unwrap()
            .join()
            .unwrap()
    }

    /// Dasselbe ueber `run`: der Thread des Aufrufers baut nur den Empfangskanal.
    fn fixed_on_main_thread_sized_stack() -> u32 {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                tauri::async_runtime::block_on(run("child-job", || heavy_job::<CHILD_JOB>(1)))
                    .unwrap()
            })
            .unwrap()
            .join()
            .unwrap()
    }

    fn child_body(legacy: bool) {
        let value = if legacy {
            legacy_on_main_thread_sized_stack()
        } else {
            fixed_on_main_thread_sized_stack()
        };
        println!("CHILD_RESULT={value}");
    }

    fn run_child(test: &str, legacy: bool) -> std::process::Output {
        Command::new(std::env::current_exe().unwrap())
            .args([test, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, if legacy { "legacy" } else { "fixed" })
            .output()
            .unwrap()
    }

    #[test]
    fn a_job_future_built_on_a_one_mib_thread_overflows_the_stack() {
        if let Ok(mode) = std::env::var(CHILD) {
            if mode == "legacy" {
                child_body(true);
            }
            return;
        }
        let out = run_child("commands::big_stack::tests::a_job_future_built_on_a_one_mib_thread_overflows_the_stack", true);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "der Kindprozess muss abstuerzen");
        assert!(
            stderr.contains("overflowed its stack")
                || stderr.contains("stack overflow")
                || out.status.code() == Some(0xc000_00fdu32 as i32),
            "kein Stack-Ueberlauf: {:?} / {stderr}",
            out.status
        );
    }

    #[test]
    fn the_same_job_through_run_survives_on_the_one_mib_thread() {
        if let Ok(mode) = std::env::var(CHILD) {
            if mode == "fixed" {
                child_body(false);
            }
            return;
        }
        let out = run_child(
            "commands::big_stack::tests::the_same_job_through_run_survives_on_the_one_mib_thread",
            false,
        );
        assert!(
            out.status.success(),
            "{:?} / {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("CHILD_RESULT=2"));
    }
}
