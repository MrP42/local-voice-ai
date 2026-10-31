//! Gemeinsamer Zustand des Sprachdienstes: Zugänge, offene Kopplungen, Engines, Grenzen.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{broadcast, OwnedSemaphorePermit, Semaphore};
use wai_voice_dienst::grenzen;
use wai_voice_dienst::msg::Recht;

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
    /// Gleichzeitige Engine-Aufträge im ganzen Dienst (Erkennen und Sprechen).
    pub engine_global: usize,
    /// Gleichzeitige Engine-Aufträge je Verbindung.
    pub engine_je_verbindung: usize,
    /// Gleichzeitig laufende Teilergebnis-Läufe im ganzen Dienst.
    pub teil_global: usize,
    /// Verbindungen je `client`-Kennung (nach dem Handschlag) und je Gegenstelle (davor).
    pub je_gegenstelle: usize,
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
            engine_global: 2,
            engine_je_verbindung: 2,
            teil_global: 1,
            je_gegenstelle: 2,
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

/// Was laufenden Verbindungen eines Zugangs widerfährt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entzug {
    /// Widerruf oder Entfernen: die Verbindungen enden mit `schluessel_widerrufen`.
    Zugang(String),
    /// Ein Recht wurde entzogen: nur die Ströme dieses Rechts enden mit `recht_fehlt`.
    Recht(String, Recht),
}

/// Zählt gleichzeitige Verbindungen je Schlüssel (Gegenstelle oder `client`-Kennung). Die Marke
/// gibt ihren Platz beim Ablegen frei.
pub struct Zaehler<K: Hash + Eq + Clone + Send + 'static> {
    grenze: usize,
    stand: Arc<Mutex<HashMap<K, usize>>>,
}

pub struct Marke<K: Hash + Eq + Clone + Send + 'static> {
    schluessel: K,
    stand: Arc<Mutex<HashMap<K, usize>>>,
}

impl<K: Hash + Eq + Clone + Send + 'static> Zaehler<K> {
    pub fn new(grenze: usize) -> Self {
        Self {
            grenze,
            stand: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Ein Platz für `schluessel`; `None`, wenn schon `grenze` belegt sind.
    pub fn nehmen(&self, schluessel: K) -> Option<Marke<K>> {
        let mut m = self.stand.lock().unwrap_or_else(|e| e.into_inner());
        let n = m.entry(schluessel.clone()).or_insert(0);
        if *n >= self.grenze {
            return None;
        }
        *n += 1;
        Some(Marke {
            schluessel,
            stand: self.stand.clone(),
        })
    }
}

impl<K: Hash + Eq + Clone + Send + 'static> Drop for Marke<K> {
    fn drop(&mut self) {
        let mut m = self.stand.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = m.get_mut(&self.schluessel) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                m.remove(&self.schluessel);
            }
        }
    }
}

pub struct Kontext {
    pub konfig: Konfig,
    pub zugaenge: Arc<ZugangStore>,
    pub kopplungen: Kopplungen,
    pub engine: Arc<dyn Sprache>,
    /// Uhr in Millisekunden (UTC); Tests und die App setzen sie.
    pub uhr: fn() -> i64,
    meldung: Box<dyn Fn(Meldung) + Send + Sync>,
    widerruf: broadcast::Sender<Entzug>,
    plaetze: Arc<Semaphore>,
    engine_plaetze: Arc<Semaphore>,
    teil_plaetze: Arc<Semaphore>,
    gegenstellen: Zaehler<u32>,
    clients: Zaehler<String>,
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
        let engine_plaetze = Arc::new(Semaphore::new(konfig.engine_global));
        let teil_plaetze = Arc::new(Semaphore::new(konfig.teil_global));
        let gegenstellen = Zaehler::new(konfig.je_gegenstelle);
        let clients = Zaehler::new(konfig.je_gegenstelle);
        Arc::new(Self {
            konfig,
            zugaenge,
            kopplungen: Kopplungen::new(),
            engine,
            uhr,
            meldung,
            widerruf,
            plaetze,
            engine_plaetze,
            teil_plaetze,
            gegenstellen,
            clients,
        })
    }

    pub fn melden(&self, m: Meldung) {
        (self.meldung)(m);
    }

    /// Widerruf oder Entfernen: alle laufenden Verbindungen dieses Zugangs enden sofort.
    pub fn verbindungen_beenden(&self, zugang: &str) {
        let _ = self.widerruf.send(Entzug::Zugang(zugang.to_string()));
    }

    /// Rechteentzug: nur die laufenden Ströme dieses Rechts enden (mit `recht_fehlt`).
    pub fn recht_entzogen(&self, zugang: &str, recht: Recht) {
        let _ = self.widerruf.send(Entzug::Recht(zugang.to_string(), recht));
    }

    pub(super) fn abonnieren(&self) -> broadcast::Receiver<Entzug> {
        self.widerruf.subscribe()
    }

    /// Platz für eine weitere Verbindung; `None`, wenn alle belegt sind.
    pub fn platz(&self) -> Option<OwnedSemaphorePermit> {
        self.plaetze.clone().try_acquire_owned().ok()
    }

    /// Platz für eine Verbindung dieser Gegenstelle (Prozess-ID) vor dem Handschlag.
    pub fn gegenstelle(&self, pid: u32) -> Option<Marke<u32>> {
        self.gegenstellen.nehmen(pid)
    }

    /// Platz für eine Verbindung dieser `client`-Kennung nach dem Handschlag.
    pub(super) fn client_platz(&self, client: &str) -> Option<Marke<String>> {
        self.clients.nehmen(client.to_string())
    }

    pub(super) fn engine_plaetze(&self) -> Arc<Semaphore> {
        self.engine_plaetze.clone()
    }

    pub(super) fn teil_plaetze(&self) -> Arc<Semaphore> {
        self.teil_plaetze.clone()
    }
}
