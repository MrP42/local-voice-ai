//! Das Tor fuer schwere Schritte, das die vorhandene Import-Warteschlange und die Aufnahme
//! kennt (B3, QG5). Ein `HeavyGate` im Sinne von `heavy.rs`; die Engine bekommt es statt des
//! einfachen `LocalHeavyGate` (`hub::WorkflowHub::start`).
//!
//! Regeln, in dieser Reihenfolge; keine blockiert, jede ist ein Warten (`HeavyWait`), nie ein
//! Fehler (Rueckstau statt Abbruch):
//! 1. **Eine Aufnahme laeuft**: nichts Schweres beginnt. Die Aufnahme hat Vorrang (wie in der
//!    Import-Warteschlange selbst); Live-Transkription und GPU gehoeren ihr.
//! 2. **Die Import-Warteschlange arbeitet** (eine Datei laeuft oder wartet): nichts Schweres
//!    beginnt. Zwei gleichzeitig ausgeloeste Dateien laufen so nacheinander (jeder
//!    `meeting.import`-Schritt reiht erst ein, wenn die Warteschlange leer ist), und KI-Notizen
//!    einer Besprechung beginnen erst, wenn die Transkription fertig ist.
//! 3. Sonst entscheidet das innere Tor: ein Platz je Prozess und das RAM-Start-Tor von
//!    `process_guard` (bei vollem RAM wartet der Schritt, der Rechner bleibt bedienbar).
//!
//! `try_enter` blockiert nie; es fragt nur Zaehler ab (`QueueProbe`).

use std::sync::Arc;

use super::action::HeavyNeed;
use super::heavy::{HeavyGate, HeavyPermit, HeavyWait, HeavyWaitReason};

/// Wartezeit, wenn eine Aufnahme laeuft.
pub const RECORDING_RETRY_MS: u64 = 10_000;
/// Wartezeit, wenn die Import-Warteschlange arbeitet.
pub const QUEUE_RETRY_MS: u64 = 5_000;

/// Was das Tor ueber die App wissen muss.
pub trait QueueProbe: Send + Sync {
    /// Laeuft eine Live-Aufnahme?
    fn recording(&self) -> bool;
    /// Wartet oder laeuft eine Datei der Import-Warteschlange?
    fn import_busy(&self) -> bool;
}

pub struct QueueAwareGate {
    inner: Arc<dyn HeavyGate>,
    probe: Arc<dyn QueueProbe>,
}

impl QueueAwareGate {
    pub fn new(inner: Arc<dyn HeavyGate>, probe: Arc<dyn QueueProbe>) -> Self {
        Self { inner, probe }
    }
}

impl HeavyGate for QueueAwareGate {
    fn try_enter(&self, need: &HeavyNeed) -> Result<HeavyPermit, HeavyWait> {
        if self.probe.recording() {
            return Err(HeavyWait {
                reason: HeavyWaitReason::Other,
                message: format!("Wartet: eine Aufnahme läuft ({}).", need.label),
                retry_after_ms: RECORDING_RETRY_MS,
            });
        }
        if self.probe.import_busy() {
            return Err(HeavyWait {
                reason: HeavyWaitReason::Other,
                message: format!(
                    "Wartet: die Import-Warteschlange arbeitet ({}).",
                    need.label
                ),
                retry_after_ms: QUEUE_RETRY_MS,
            });
        }
        self.inner.try_enter(need)
    }
}

#[cfg(test)]
mod tests;
