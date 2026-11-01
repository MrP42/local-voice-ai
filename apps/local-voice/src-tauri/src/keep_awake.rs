//! Bildschirm wach halten, solange der Nutzer es im Vorlesen eingeschaltet
//! hat (Sonnen-Knopf). Das Betriebssystem soll die Anzeige weder abschalten
//! noch dimmen, solange man liest oder zuhört.
//!
//! - Windows: `SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED |
//!   ES_SYSTEM_REQUIRED)`. Der Zustand gilt je Thread, deshalb hält ihn ein
//!   eigener Thread, bis er freigegeben wird.
//! - macOS: `caffeinate -d -i -w <pid>` als Kindprozess; endet spätestens mit
//!   der App.
//! - Sonst: nichts (die WebView-Wake-Lock-API der Oberfläche bleibt).
//!
//! Der Panel-Schutz eines OLED-Monitors ist davon unabhängig und nur im
//! Monitormenü abschaltbar; dagegen verschiebt die Oberfläche den Inhalt.

use std::sync::Mutex;

enum Holder {
    #[cfg(windows)]
    Thread(std::sync::mpsc::Sender<()>),
    #[cfg(target_os = "macos")]
    Child(std::process::Child),
}

static HOLDER: Mutex<Option<Holder>> = Mutex::new(None);

/// Einschalten (idempotent) oder freigeben. `Ok(true)`, wenn das
/// Betriebssystem die Anzeige jetzt wach hält.
pub fn set(on: bool) -> Result<bool, String> {
    let mut slot = HOLDER.lock().map_err(|e| e.to_string())?;
    if !on {
        release(slot.take());
        return Ok(false);
    }
    if alive(slot.as_mut()) {
        return Ok(true);
    }
    release(slot.take());
    *slot = acquire()?;
    Ok(slot.is_some())
}

/// Hält der bestehende Halter noch? Ein von außen beendetes `caffeinate`
/// wird so bemerkt und neu gestartet.
fn alive(holder: Option<&mut Holder>) -> bool {
    match holder {
        #[cfg(target_os = "macos")]
        Some(Holder::Child(child)) => matches!(child.try_wait(), Ok(None)),
        #[cfg(windows)]
        Some(Holder::Thread(_)) => true,
        #[allow(unreachable_patterns)]
        Some(_) => false,
        None => false,
    }
}

#[cfg(windows)]
fn acquire() -> Result<Option<Holder>, String> {
    use windows::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
    };
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let (ok_tx, ok_rx) = std::sync::mpsc::channel::<bool>();
    std::thread::Builder::new()
        .name("keep-awake".into())
        .spawn(move || {
            // SAFETY: reine Zustandsmeldung an den Energiemanager, ohne Zeiger.
            let prev = unsafe {
                SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED)
            };
            let _ = ok_tx.send(prev.0 != 0);
            // Warten, bis freigegeben wird (oder der Sender wegfällt).
            let _ = rx.recv();
            // SAFETY: wie oben; setzt den Thread-Zustand zurück.
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        })
        .map_err(|e| e.to_string())?;
    match ok_rx.recv() {
        Ok(true) => Ok(Some(Holder::Thread(tx))),
        _ => {
            let _ = tx.send(());
            Err("SetThreadExecutionState fehlgeschlagen".into())
        }
    }
}

#[cfg(target_os = "macos")]
fn acquire() -> Result<Option<Holder>, String> {
    let child = std::process::Command::new("/usr/bin/caffeinate")
        .args(["-d", "-i", "-w", &std::process::id().to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(Some(Holder::Child(child)))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn acquire() -> Result<Option<Holder>, String> {
    Ok(None)
}

fn release(holder: Option<Holder>) {
    match holder {
        #[cfg(windows)]
        Some(Holder::Thread(tx)) => {
            let _ = tx.send(());
        }
        #[cfg(target_os = "macos")]
        Some(Holder::Child(mut child)) => {
            let _ = child.kill();
            let _ = child.wait();
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ein_und_aus_sind_idempotent() {
        // Auf Linux ohne Wirkung, aber ohne Fehler; zweimal an, zweimal aus.
        assert!(super::set(true).is_ok());
        assert!(super::set(true).is_ok());
        assert_eq!(super::set(false), Ok(false));
        assert_eq!(super::set(false), Ok(false));
    }
}
