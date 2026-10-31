//! Gemeinsamer Zustand des Sprachdienstes: Zugänge, offene Kopplungen, Engines, Grenzen.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, OwnedSemaphorePermit, Semaphore};
use wai_voice_dienst::grenzen;

use super::engine::Sprache;
use super::kopplung::Kopplungen;
use super::zugang::ZugangStore;

/// Zeiten und Größen des Dienstes. [`Konfig::standard`] gibt die Werte des Vertrags; Tests
/// verkürzen die Zeiten.
#[derive(Clone, Debug)]
pub struct Konfig {
    /// Name in `willkommen`.
    pub dienst: String,
    pub version: String,
    /// So lange wartet der Dienst auf die Bestätigung einer Kopplung (`KOPPELN_BESTAETIGUNG_S`).
    pub bestaetigung: Duration,
    /// Bis zum ersten `hallo`/`koppeln` nach dem `gruss`.
    pub anfang: Duration,
    /// Ohne Audio und ohne Steuerrahmen endet ein Strom nach dieser Zeit (`LEERLAUF_S`).
    pub leerlauf_strom: Duration,
    /// Eine angemeldete Verbindung ganz ohne Ströme und ohne Rahmen endet nach dieser Zeit.
    pub leerlauf_verbindung: Duration,
    /// Frist für das Schreiben eines Rahmens an eine Gegenstelle, die nicht liest.
    pub schreiben: Duration,
    /// Längste Äußerung beim Erkennen (`AEUSSERUNG_MAX_S`), in Sekunden.
    pub aeusserung_max_s: u32,
    /// Neue Audio-Bytes, nach denen beim Diktat ein weiteres Teilergebnis gerechnet wird.
    pub teil_abstand: usize,
    /// Gleichzeitige Verbindungen.
    pub max_verbindungen: usize,
    /// Takt für Fristen, Guthaben-Rückgabe, Teilergebnisse und Audio heraus.
    pub takt: Duration,
}

impl Konfig {
    pub fn standard(version: &str) -> Self {
        Self {
            dienst: "local-voice-ai".to_string(),
            version: version.to_string(),
            bestaetigung: Duration::from_secs(u64::from(grenzen::KOPPELN_BESTAETIGUNG_S)),
            anfang: Duration::from_secs(u64::from(grenzen::LEERLAUF_S)),
            leerlauf_strom: Duration::from_secs(u64::from(grenzen::LEERLAUF_S)),
            leerlauf_verbindung: Duration::from_secs(300),
            schreiben: Duration::from_secs(10),
            aeusserung_max_s: grenzen::AEUSSERUNG_MAX_S,
            // 1,5 s PCM16 mono 16 kHz.
            teil_abstand: 48_000,
            max_verbindungen: 8,
            takt: Duration::from_millis(100),
        }
    }
}

/// Was die Oberfläche erfahren soll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Meldung {
    /// Eine Kopplungsanfrage wartet auf die Bestätigung des Nutzers.
    KopplungOffen,
    /// Zugänge haben sich geändert (neu gekoppelt).
    ZugaengeGeaendert,
}

pub struct Kontext {
    pub konfig: Konfig,
    pub zugaenge: Arc<ZugangStore>,
    pub kopplungen: Kopplungen,
    pub engine: Arc<dyn Sprache>,
    /// Uhr in Millisekunden (UTC); Tests und die App setzen sie.
    pub uhr: fn() -> i64,
    meldung: Box<dyn Fn(Meldung) + Send + Sync>,
    widerruf: broadcast::Sender<String>,
    plaetze: Arc<Semaphore>,
}

impl Kontext {
    pub fn new(
        konfig: Konfig,
        zugaenge: Arc<ZugangStore>,
        engine: Arc<dyn Sprache>,
        uhr: fn() -> i64,
        meldung: Box<dyn Fn(Meldung) + Send + Sync>,
    ) -> Arc<Self> {
        let (widerruf, _) = broadcast::channel(32);
        let plaetze = Arc::new(Semaphore::new(konfig.max_verbindungen));
        Arc::new(Self {
            konfig,
            zugaenge,
            kopplungen: Kopplungen::new(),
            engine,
            uhr,
            meldung,
            widerruf,
            plaetze,
        })
    }

    pub fn melden(&self, m: Meldung) {
        (self.meldung)(m);
    }

    /// Alle laufenden Verbindungen dieses Zugangs beenden (Widerruf wirkt sofort).
    pub fn verbindungen_beenden(&self, zugang: &str) {
        let _ = self.widerruf.send(zugang.to_string());
    }

    pub(super) fn abonnieren(&self) -> broadcast::Receiver<String> {
        self.widerruf.subscribe()
    }

    /// Platz für eine weitere Verbindung; `None`, wenn alle belegt sind.
    pub fn platz(&self) -> Option<OwnedSemaphorePermit> {
        self.plaetze.clone().try_acquire_owned().ok()
    }
}
