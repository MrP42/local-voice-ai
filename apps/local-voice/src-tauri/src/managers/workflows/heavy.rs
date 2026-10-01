//! Das Tor fuer schwere Schritte (STT, Sprachmodell, TTS): seriell und mit
//! RAM-Pruefung (B1, QG5).
//!
//! Die Engine holt vor jedem schweren Baustein (`Action::heavy`) einen Platz mit
//! [`HeavyGate::try_enter`]. Gibt es keinen, WARTET der Lauf (er geht mit
//! `next_run_at` zurueck in die Warteschlange, `wait_reason` nennt den Grund), er
//! scheitert nicht und verbraucht keinen Versuch: Rueckstau statt Abbruch.
//!
//! [`LocalHeavyGate`] ist die Standardumsetzung: EIN Platz je Prozess (alle Engines
//! teilen [`LocalHeavyGate::shared`]) und das RAM-Start-Tor von `process_guard`
//! (Bedarf plus Systemreserve). Bei vollem RAM passiert also: der Schritt wartet, der
//! Rechner bleibt bedienbar, nach 30 s wird erneut geprueft.
//!
//! **Schnittstelle fuer B3/B4:** wer die vorhandene schwere Warteschlange der
//! Besprechungen (`meetings::queue`, Aufnahme hat Vorrang, gleichzeitige
//! Transkriptionen, GPU-Tor) einbinden will, schreibt einen eigenen `HeavyGate`, der
//! deren Belegung abfragt (`try_enter` darf nie blockieren), und gibt ihn der Engine
//! mit. An den Aufrufern aendert sich nichts.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use once_cell::sync::Lazy;

use super::action::HeavyNeed;

/// Warum ein schwerer Schritt jetzt nicht beginnt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeavyWaitReason {
    /// Ein anderer schwerer Schritt laeuft.
    Slot,
    /// Zu wenig freier Arbeitsspeicher.
    Memory,
    /// Etwas anderes hat Vorrang (z. B. eine laufende Aufnahme).
    Other,
}

impl HeavyWaitReason {
    /// Wert fuer `workflow_runs.wait_reason`.
    pub fn code(self) -> &'static str {
        match self {
            HeavyWaitReason::Slot => "heavy_slot",
            HeavyWaitReason::Memory => "heavy_memory",
            HeavyWaitReason::Other => "heavy_other",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeavyWait {
    pub reason: HeavyWaitReason,
    /// Klartext fuer die Oberflaeche ("wartet auf Arbeitsspeicher").
    pub message: String,
    pub retry_after_ms: u64,
}

/// Der gehaltene Platz; gibt ihn beim Loslassen frei (auch bei Panik).
pub struct HeavyPermit {
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl HeavyPermit {
    pub fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }
}

impl Drop for HeavyPermit {
    fn drop(&mut self) {
        if let Some(f) = self.release.take() {
            f();
        }
    }
}

pub trait HeavyGate: Send + Sync {
    /// Versucht, JETZT zu beginnen. Blockiert nie.
    fn try_enter(&self, need: &HeavyNeed) -> Result<HeavyPermit, HeavyWait>;
}

/// Pruefung des freien Arbeitsspeichers: `Err(text)`, wenn der Bedarf nicht passt.
pub type RamProbe = Box<dyn Fn(u64) -> Result<(), String> + Send + Sync>;

/// Wartezeit, wenn ein anderer schwerer Schritt laeuft.
pub const SLOT_RETRY_MS: u64 = 3_000;
/// Wartezeit, wenn der Arbeitsspeicher nicht reicht.
pub const MEMORY_RETRY_MS: u64 = 30_000;

pub struct LocalHeavyGate {
    busy: Arc<AtomicBool>,
    ram: RamProbe,
}

static SHARED: Lazy<Arc<LocalHeavyGate>> = Lazy::new(|| Arc::new(LocalHeavyGate::new()));

impl LocalHeavyGate {
    /// Mit dem RAM-Start-Tor von `process_guard`.
    pub fn new() -> Self {
        Self::with_probe(Box::new(|need| {
            crate::process_guard::check_ram_for_start(need).map(|_| ())
        }))
    }

    pub fn with_probe(ram: RamProbe) -> Self {
        Self {
            busy: Arc::new(AtomicBool::new(false)),
            ram,
        }
    }

    /// Das prozessweit geteilte Tor (ein Platz fuer alle Engines).
    pub fn shared() -> Arc<LocalHeavyGate> {
        SHARED.clone()
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
}

impl Default for LocalHeavyGate {
    fn default() -> Self {
        Self::new()
    }
}

impl HeavyGate for LocalHeavyGate {
    fn try_enter(&self, need: &HeavyNeed) -> Result<HeavyPermit, HeavyWait> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(HeavyWait {
                reason: HeavyWaitReason::Slot,
                message: format!("Wartet: ein anderer schwerer Schritt läuft ({}).", need.label),
                retry_after_ms: SLOT_RETRY_MS,
            });
        }
        let busy = self.busy.clone();
        // Der Platz gehoert ab hier dem Permit; scheitert das RAM-Tor, gibt er ihn frei.
        let permit = HeavyPermit::new(move || busy.store(false, Ordering::Release));
        match (self.ram)(need.ram_mb) {
            Ok(()) => Ok(permit),
            Err(text) => Err(HeavyWait {
                reason: HeavyWaitReason::Memory,
                message: format!("Wartet auf Arbeitsspeicher: {text}"),
                retry_after_ms: MEMORY_RETRY_MS,
            }),
        }
    }
}
