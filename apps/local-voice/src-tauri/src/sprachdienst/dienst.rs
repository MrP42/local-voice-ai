//! Start, Stopp und Zustand des Sprachdienstes.
//!
//! Der Dienst ist **standardmäßig aus** (Opt-in): Die App ruft [`Sprachdienst::starten`] nur, wenn
//! der Nutzer ihn eingeschaltet hat. `starten` und `stoppen` sind idempotent. Fehler beim Start
//! (kein sicherer Laufzeitordner, Pipe belegt) bleiben für die Oberfläche lesbar und lassen die
//! App normal weiterlaufen.
//!
//! Start und Stopp laufen hintereinander (ein Mutex für beide). Ein Neustart wartet, bis die alte
//! Annahme-Aufgabe beendet ist und ihren Endpunkt freigegeben hat; sonst könnte der Stopp des
//! alten Dienstes den frisch gebundenen Socket des neuen entfernen.

use std::sync::{Arc, Mutex};
use std::time::Duration;

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

/// So lange wartet `stoppen` auf das Ende der Annahme-Aufgabe.
const STOPP_WARTEN: Duration = Duration::from_secs(5);

struct Lauf {
    stopp: watch::Sender<bool>,
    task: JoinHandle<()>,
    endpunkt: String,
}

pub struct Sprachdienst {
    ctx: Arc<Kontext>,
    lauf: Mutex<Option<Lauf>>,
    fehler: Mutex<Option<String>>,
    /// Serialisiert Start und Stopp.
    steuerung: tokio::sync::Mutex<()>,
}

impl Sprachdienst {
    pub fn neu(ctx: Arc<Kontext>) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            lauf: Mutex::new(None),
            fehler: Mutex::new(None),
            steuerung: tokio::sync::Mutex::new(()),
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
        let _steuerung = self.steuerung.lock().await;
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
        self.binden_unix(&laufzeit)
    }

    /// Wie `starten`, mit einem Laufzeitordner (Tests).
    #[cfg(unix)]
    pub async fn starten_unix(&self, laufzeit: &std::path::Path) -> Result<String, String> {
        let _steuerung = self.steuerung.lock().await;
        if let Some(ep) = self.laeuft() {
            return Ok(ep);
        }
        let r = self.binden_unix(laufzeit);
        if let Err(e) = &r {
            self.fehler_merken(Some(e.clone()));
        }
        r
    }

    #[cfg(unix)]
    fn binden_unix(&self, laufzeit: &std::path::Path) -> Result<String, String> {
        let g = super::unix::binden(laufzeit)
            .map_err(|e| format!("Der Socket ließ sich nicht anlegen: {e}"))?;
        let endpunkt = g.pfad.display().to_string();
        let (tx, rx) = watch::channel(false);
        let uid = g.uid;
        let task = tokio::spawn(super::unix::annehmen(g, uid, self.ctx.clone(), rx));
        Ok(self.eintragen(Lauf {
            stopp: tx,
            task,
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
            task,
            endpunkt,
        }))
    }

    #[cfg(not(any(unix, windows)))]
    async fn starten_plattform(&self) -> Result<String, String> {
        Err("Der Sprachdienst gibt es auf diesem System nicht.".to_string())
    }

    /// Hört auf zu lauschen, beendet die laufenden Verbindungen und wartet, bis die
    /// Annahme-Aufgabe ihren Endpunkt freigegeben hat.
    pub async fn stoppen(&self) {
        let _steuerung = self.steuerung.lock().await;
        let lauf = self.lauf.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(l) = lauf {
            let _ = l.stopp.send(true);
            if tokio::time::timeout(STOPP_WARTEN, l.task).await.is_err() {
                log::warn!("sprachdienst: Annahme endete nicht rechtzeitig");
            }
        }
        self.fehler_merken(None);
    }
}
