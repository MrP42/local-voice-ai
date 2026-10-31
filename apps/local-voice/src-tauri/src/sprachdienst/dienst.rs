//! Start, Stopp und Zustand des Sprachdienstes.
//!
//! Der Dienst ist **standardmäßig aus** (Opt-in): Die App ruft [`Sprachdienst::starten`] nur, wenn
//! der Nutzer ihn eingeschaltet hat. `starten` und `stoppen` sind idempotent. Fehler beim Start
//! (kein sicherer Laufzeitordner, Pipe belegt) bleiben für die Oberfläche lesbar und lassen die
//! App normal weiterlaufen.

use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::kontext::Kontext;

/// Zustand für die Oberfläche.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub laeuft: bool,
    /// Pipe-Name bzw. Socket-Pfad (kein Geheimnis).
    pub endpunkt: Option<String>,
    /// Warum er nicht läuft.
    pub fehler: Option<String>,
}

struct Lauf {
    stopp: watch::Sender<bool>,
    _task: JoinHandle<()>,
    endpunkt: String,
}

pub struct Sprachdienst {
    ctx: Arc<Kontext>,
    lauf: Mutex<Option<Lauf>>,
    fehler: Mutex<Option<String>>,
}

impl Sprachdienst {
    pub fn neu(ctx: Arc<Kontext>) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            lauf: Mutex::new(None),
            fehler: Mutex::new(None),
        })
    }

    pub fn kontext(&self) -> &Arc<Kontext> {
        &self.ctx
    }

    pub fn status(&self) -> Status {
        let lauf = self.lauf.lock().unwrap_or_else(|e| e.into_inner());
        Status {
            laeuft: lauf.is_some(),
            endpunkt: lauf.as_ref().map(|l| l.endpunkt.clone()),
            fehler: self
                .fehler
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        }
    }

    fn fehler_merken(&self, f: Option<String>) {
        *self.fehler.lock().unwrap_or_else(|e| e.into_inner()) = f;
    }

    fn eintragen(&self, lauf: Lauf) -> String {
        let ep = lauf.endpunkt.clone();
        *self.lauf.lock().unwrap_or_else(|e| e.into_inner()) = Some(lauf);
        self.fehler_merken(None);
        ep
    }

    fn laeuft(&self) -> Option<String> {
        self.lauf
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|l| l.endpunkt.clone())
    }

    /// Beginnt zu lauschen, am Ort des Betriebssystems. Rückgabe: Endpunkt.
    pub async fn starten(&self) -> Result<String, String> {
        if let Some(ep) = self.laeuft() {
            return Ok(ep);
        }
        let r = self.starten_plattform().await;
        match &r {
            Ok(_) => {}
            Err(e) => {
                log::warn!("sprachdienst: Start fehlgeschlagen: {e}");
                self.fehler_merken(Some(e.clone()));
            }
        }
        r
    }

    #[cfg(unix)]
    async fn starten_plattform(&self) -> Result<String, String> {
        let laufzeit = wai_voice_dienst::endpunkt::laufzeitordner().ok_or_else(|| {
            "Es gibt keinen sicheren Laufzeitordner für den Benutzer (XDG_RUNTIME_DIR bzw. TMPDIR).".to_string()
        })?;
        self.starten_unix(&laufzeit)
    }

    /// Wie `starten`, mit einem Laufzeitordner (Tests).
    #[cfg(unix)]
    pub fn starten_unix(&self, laufzeit: &std::path::Path) -> Result<String, String> {
        if let Some(ep) = self.laeuft() {
            return Ok(ep);
        }
        let g = super::unix::binden(laufzeit)
            .map_err(|e| format!("Der Socket ließ sich nicht anlegen: {e}"))?;
        let endpunkt = g.pfad.display().to_string();
        let (tx, rx) = watch::channel(false);
        let uid = g.uid;
        let task = tokio::spawn(super::unix::annehmen(g, uid, self.ctx.clone(), rx));
        Ok(self.eintragen(Lauf {
            stopp: tx,
            _task: task,
            endpunkt,
        }))
    }

    #[cfg(windows)]
    async fn starten_plattform(&self) -> Result<String, String> {
        let g =
            super::pipe::binden().map_err(|e| format!("Die Pipe ließ sich nicht anlegen: {e}"))?;
        let endpunkt = g.name.clone();
        let (tx, rx) = watch::channel(false);
        let task = tokio::spawn(super::pipe::annehmen(g, self.ctx.clone(), rx));
        Ok(self.eintragen(Lauf {
            stopp: tx,
            _task: task,
            endpunkt,
        }))
    }

    #[cfg(not(any(unix, windows)))]
    async fn starten_plattform(&self) -> Result<String, String> {
        Err("Der Sprachdienst gibt es auf diesem System nicht.".to_string())
    }

    /// Hört auf zu lauschen und beendet die laufenden Verbindungen.
    pub fn stoppen(&self) {
        let lauf = self.lauf.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(l) = lauf {
            let _ = l.stopp.send(true);
        }
        self.fehler_merken(None);
    }
}
