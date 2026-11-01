//! Named Pipe des Sprachdienstes (Windows): `\\.\pipe\wai-sprache-<Benutzer-SID>`.
//!
//! Die Rechte sind die der Agentenbrücke (`agent_bridge::pipe`), deren Bausteine hier benutzt werden:
//! - **Nur der aktuelle Benutzer**: eigene DACL mit der SID des Benutzers (Lesen/Schreiben) und
//!   ausdrücklichem Verbot für Netzwerk-Anmeldungen; kein `Everyone`, keine Gruppen.
//! - **Keine Fernzugriffe**: `PIPE_REJECT_REMOTE_CLIENTS`.
//! - **Kein Vorbelegen**: Die erste Instanz entsteht mit `FILE_FLAG_FIRST_PIPE_INSTANCE`; belegt
//!   ein anderer Prozess den Namen, scheitert der Start laut.
//! - **Prüfung der Gegenstelle je Verbindung**: Die Benutzer-SID des Client-Prozesses muss die
//!   eigene sein; ist sie anders oder nicht lesbar (etwa ein Prozess mit erhöhten Rechten), wird
//!   die Verbindung ohne ein Wort geschlossen.
//!
//! Dieses Modul lässt sich nur unter Windows übersetzen und prüfen.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::watch;
use wai_voice_dienst::endpunkt::pipe_name;

use super::kontext::Kontext;
use super::verbindung::verbindung;
use crate::agent_bridge::pipe::{
    client_pid, create_instance, current_user_sid, pid_belongs_to, OwnedSid, PipeSecurity,
};

pub struct Gebunden {
    pub name: String,
    sicherheit: PipeSecurity,
    erste: NamedPipeServer,
    eigene: OwnedSid,
}

/// Name der Pipe dieses Benutzers.
pub fn name() -> io::Result<String> {
    let sid = current_user_sid()?.to_sid_string();
    pipe_name(&sid).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Die Benutzer-SID hat keine gültige Form.",
        )
    })
}

/// Legt die erste Pipe-Instanz an. Muss im Kontext einer tokio-Laufzeit laufen.
pub fn binden() -> io::Result<Gebunden> {
    let name = name()?;
    let mut sicherheit = PipeSecurity::only(current_user_sid()?)?;
    let erste = create_instance(&name, &mut sicherheit, true)?;
    Ok(Gebunden {
        name,
        sicherheit,
        erste,
        eigene: current_user_sid()?,
    })
}

pub async fn annehmen(g: Gebunden, ctx: Arc<Kontext>, mut stopp: watch::Receiver<bool>) {
    let Gebunden {
        name,
        mut sicherheit,
        erste,
        eigene,
    } = g;
    let mut wartend: Option<NamedPipeServer> = Some(erste);
    loop {
        // Immer eine wartende Instanz bereithalten, damit Clients nie „keine Pipe“ sehen.
        let aktuell = match wartend.take() {
            Some(l) => l,
            None => match create_instance(&name, &mut sicherheit, false) {
                Ok(l) => l,
                Err(e) => {
                    log::warn!("sprachdienst: keine neue Pipe-Instanz: {e}");
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
                        _ = stopp.changed() => break,
                    }
                }
            },
        };
        let verbunden = tokio::select! {
            r = aktuell.connect() => r,
            _ = stopp.changed() => break,
        };
        if let Err(e) = verbunden {
            log::debug!("sprachdienst: Verbindung beim Annehmen abgebrochen: {e}");
            continue;
        }
        match create_instance(&name, &mut sicherheit, false) {
            Ok(n) => wartend = Some(n),
            Err(e) => log::warn!("sprachdienst: keine neue Pipe-Instanz: {e}"),
        }
        let pid = client_pid(&aktuell);
        match pid
            .as_ref()
            .map_err(|e| e.to_string())
            .and_then(|pid| pid_belongs_to(*pid, &eigene).map_err(|e| e.to_string()))
        {
            Ok(true) => {
                // Je Gegenstelle (Prozess) höchstens `je_gegenstelle` Verbindungen vor dem Handschlag.
                let Some(gegenstelle) = pid.ok().and_then(|p| ctx.gegenstelle(p)) else {
                    log::warn!("sprachdienst: zu viele Verbindungen einer Gegenstelle abgewiesen");
                    continue;
                };
                let Some(platz) = ctx.platz() else {
                    log::warn!("sprachdienst: alle Verbindungsplätze belegt");
                    continue;
                };
                let (ctx, stopp) = (ctx.clone(), stopp.clone());
                tokio::spawn(async move {
                    let _platz = platz;
                    let _gegenstelle = gegenstelle;
                    verbindung(aktuell, ctx, stopp).await;
                });
            }
            Ok(false) => log::warn!("sprachdienst: Gegenstelle eines anderen Benutzers abgewiesen"),
            Err(e) => log::warn!("sprachdienst: Gegenstelle nicht prüfbar, abgewiesen: {e}"),
        }
    }
}
