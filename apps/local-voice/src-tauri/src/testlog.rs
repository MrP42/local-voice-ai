//! Ein gemeinsamer Log-Fänger für Tests (nur `cfg(test)`). `log` erlaubt einen Logger je Prozess;
//! mehrere Tests, die prüfen, dass keine Inhalte im Log stehen, teilen sich deshalb diesen.

use std::sync::Mutex;

struct Fang;

static ZEILEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl log::Log for Fang {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        ZEILEN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(format!("{} {}", r.target(), r.args()));
    }
    fn flush(&self) {}
}

static FANG: Fang = Fang;

/// Installiert den Fänger (einmal je Prozess; weitere Aufrufe sind folgenlos) auf Stufe Trace.
pub fn installieren() {
    let _ = log::set_logger(&FANG);
    log::set_max_level(log::LevelFilter::Trace);
}

/// Alle bisher gefangenen Zeilen (aller Tests dieses Prozesses).
pub fn zeilen() -> Vec<String> {
    ZEILEN.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
