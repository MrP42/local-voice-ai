//! Die Sprach-Engines hinter dem Sprachdienst (ADR-0021, Paket S2).
//!
//! Der Dienst kennt die Engines der App nur über [`Sprache`]. Die App-Anbindung steht in
//! `app.rs` (Erkennung über den `TranscriptionManager`, Stimmen über den `TtsManager`); Tests
//! benutzen Attrappen. Alle Aufrufe blockieren und laufen im Dienst auf eigenen Threads
//! (`spawn_blocking`), nie auf der Laufzeit der Verbindungen.
//!
//! Keine Inhalte im Log: weder Audio noch Text. Fehler tragen feste Meldungen, nie Auszüge
//! aus dem Gesprochenen.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use wai_voice_dienst::msg::{code, Engines};

/// Fehler einer Engine: ein stabiler Code des Vertrags und eine feste, inhaltsfreie Meldung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineFehler {
    pub code: &'static str,
    pub nachricht: String,
}

impl EngineFehler {
    /// Es gibt keine Engine (kein Modell gewählt, Stimme nicht installiert).
    pub fn fehlt(nachricht: impl Into<String>) -> Self {
        Self {
            code: code::ENGINE_FEHLT,
            nachricht: nachricht.into(),
        }
    }

    /// Der Auftrag wurde abgebrochen (Abbruch der App, Widerruf, Verbindungsende).
    pub fn abgebrochen() -> Self {
        Self {
            code: code::ABGEBROCHEN,
            nachricht: "Der Auftrag wurde abgebrochen.".to_string(),
        }
    }

    /// Die Engine lief, lieferte aber nichts.
    pub fn fehlgeschlagen(nachricht: impl Into<String>) -> Self {
        Self {
            code: code::FEHLGESCHLAGEN,
            nachricht: nachricht.into(),
        }
    }
}

/// Hören und Sprechen, ohne Denken (`SpeechIO` in `@wai/voice`).
pub trait Sprache: Send + Sync + 'static {
    /// Namen der laufenden Engines und ob beide auf diesem Rechner laufen. Die Souveränität
    /// `nur_lokal` verlangt `lokal == true`, sonst antwortet der Dienst mit `nicht_lokal`.
    fn engines(&self) -> Engines;

    /// Kann der Dienst jetzt Aufträge annehmen (Erkennung vorhanden)?
    fn bereit(&self) -> bool {
        true
    }

    /// Erkennt eine Äußerung: PCM16 LE, 16 kHz, mono. Blockiert. `abbruch` setzen Abbruch der App,
    /// Widerruf und das Ende der Verbindung; die Engine prüft es vor und zwischen ihren
    /// Teilschritten (ein laufender Erkennungsdurchgang selbst lässt sich nicht unterbrechen).
    fn erkennen(
        &self,
        pcm: &[u8],
        bereich: &str,
        abbruch: &Arc<AtomicBool>,
    ) -> Result<String, EngineFehler>;

    /// Spricht `text`; Rückgabe: vollständige WAV-Stücke in Sprechreihenfolge (je Satz eines).
    /// Blockiert, schreibt keine Datei, spielt nichts ab und benutzt keinen Cache. `abbruch` wie
    /// bei `erkennen`.
    fn sprechen(
        &self,
        text: &str,
        stimme: Option<&str>,
        abbruch: &Arc<AtomicBool>,
    ) -> Result<Vec<Vec<u8>>, EngineFehler>;

    /// Korrektursignal (Recht `lernen`). Local Voice AI nimmt es in Paket S2 noch nicht an; der
    /// Dienst vergibt das Recht deshalb nicht.
    fn rueckmeldung(&self, _bereich: &str, _paare: usize) -> Result<(), EngineFehler> {
        Err(EngineFehler::fehlt(
            "Lernen ist in dieser Version nicht verfügbar.",
        ))
    }
}

/// PCM16 LE nach f32 im Bereich -1,0 bis 1,0 (das Format der Erkennung der App). Ein
/// übriges ungerades Byte wird verworfen.
pub fn pcm_zu_f32(pcm: &[u8]) -> Vec<f32> {
    pcm.chunks_exact(2)
        .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_wird_normiert() {
        let pcm = [0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0xFF];
        let f = pcm_zu_f32(&pcm);
        assert_eq!(f.len(), 3, "das übrige Byte entfällt");
        assert_eq!(f[0], 0.0);
        assert_eq!(f[1], 0.5);
        assert_eq!(f[2], -1.0);
    }
}
