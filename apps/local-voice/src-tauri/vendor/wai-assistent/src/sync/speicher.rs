//! Dauerhafter Geraetezustand (ADR-0028 E3.8, Must-fix 1, 3, 6).
//!
//! Der Kern legt seinen ganzen Zustand (Hoechststaende, Mindestepoche ueber das Verzeichnis,
//! Grabsteine, Loeschplatzhalter, Schluessel, offener Widerruf, Unterhaltungen) als **einen** Block ab. Der
//! Host speichert ihn ueber [`Zustandsspeicher`] atomar: ganz oder gar nicht (z. B. SQLite-Transaktion,
//! Datei mit Umbenennen). Der Block enthaelt Geheimnisse; der Host legt ihn geschuetzt ab (Schluesselbund
//! des Betriebssystems bzw. verschluesselter Speicher).

use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

/// Speicherfehler ohne Inhalte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeicherFehler;

/// Host-Schnittstelle fuer den dauerhaften Zustand.
pub trait Zustandsspeicher: Send + Sync {
    /// Ersetzt den gesicherten Block atomar. Kehrt erst zurueck, wenn er dauerhaft liegt.
    fn sichern(&self, daten: &[u8]) -> Result<(), SpeicherFehler>;

    /// Zuletzt erfolgreich gesicherter Block (`None`: noch nie gesichert).
    fn laden(&self) -> Result<Option<Zeroizing<Vec<u8>>>, SpeicherFehler>;
}

#[derive(Default)]
struct Inneres {
    block: Option<Vec<u8>>,
    sicherungen: u64,
    /// Nur noch so viele Sicherungen gelingen (Absturz-Simulation); `None` = unbegrenzt.
    noch_erlaubt: Option<u64>,
}

/// Speicher im Arbeitsspeicher fuer Tests; Klone teilen denselben Block (Neustart-Simulation).
#[derive(Clone, Default)]
pub struct MemSpeicher {
    i: Arc<Mutex<Inneres>>,
}

impl MemSpeicher {
    pub fn new() -> Self {
        Self::default()
    }

    fn sperre(&self) -> std::sync::MutexGuard<'_, Inneres> {
        self.i.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ab jetzt gelingen nur noch `n` Sicherungen, danach scheitert jede (bis [`MemSpeicher::heilen`]).
    pub fn scheitern_nach(&self, n: u64) {
        self.sperre().noch_erlaubt = Some(n);
    }

    pub fn heilen(&self) {
        self.sperre().noch_erlaubt = None;
    }

    /// Anzahl erfolgreicher Sicherungen.
    pub fn sicherungen(&self) -> u64 {
        self.sperre().sicherungen
    }
}

impl Zustandsspeicher for MemSpeicher {
    fn sichern(&self, daten: &[u8]) -> Result<(), SpeicherFehler> {
        let mut i = self.sperre();
        match i.noch_erlaubt {
            Some(0) => return Err(SpeicherFehler),
            Some(n) => i.noch_erlaubt = Some(n - 1),
            None => {}
        }
        i.block = Some(daten.to_vec());
        i.sicherungen += 1;
        Ok(())
    }

    fn laden(&self) -> Result<Option<Zeroizing<Vec<u8>>>, SpeicherFehler> {
        Ok(self.sperre().block.clone().map(Zeroizing::new))
    }
}
