//! Unix-Socket des Sprachdienstes (macOS und Linux), nur im Laufzeitordner des Benutzers.
//!
//! Sicherheitsmerkmale, jedes mit Test:
//! - **Ordner `0700`, Socket `0600`**: `<Laufzeitordner>/wai/sprache.sock`. Der Laufzeitordner
//!   selbst (`XDG_RUNTIME_DIR`, unter macOS `TMPDIR`) darf für Gruppe und andere nicht schreibbar
//!   sein; ein gemeinsames `/tmp` wird abgelehnt.
//! - **Besitzer**: Ordner und Socket gehören dem Benutzer, dem der Laufzeitordner gehört. Die
//!   eigene Kennung wird aus dem frisch angelegten Socket gelesen (keine `libc`-Abhängigkeit).
//! - **Peer-Prüfung je Verbindung**: `SO_PEERCRED` bzw. `getpeereid` (über `peer_cred`) muss
//!   dieselbe Benutzerkennung liefern, sonst wird die Verbindung ohne ein Wort geschlossen.
//! - **Kein Überschreiben**: Ein vorhandener Socket wird nur ersetzt, wenn niemand mehr darauf
//!   hört; liegt dort etwas anderes als ein Socket, bricht der Start ab.

use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use wai_voice_dienst::endpunkt::socket_pfad;

use super::kontext::Kontext;
use super::verbindung::verbindung;

pub struct Gebunden {
    pub listener: UnixListener,
    pub pfad: PathBuf,
    /// Benutzerkennung, der die Gegenstellen gehören müssen.
    pub uid: u32,
}

fn verweigert(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, msg.to_string())
}

/// Legt Ordner und Socket an und prüft Besitzer und Modi. Muss im Kontext einer
/// tokio-Laufzeit laufen.
pub fn binden(laufzeit: &Path) -> io::Result<Gebunden> {
    let meta = std::fs::metadata(laufzeit)?;
    if !meta.is_dir() {
        return Err(verweigert("Der Laufzeitordner ist kein Ordner."));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(verweigert(
            "Der Laufzeitordner ist für andere beschreibbar.",
        ));
    }
    let besitzer = meta.uid();

    let ordner = laufzeit.join("wai");
    match std::fs::symlink_metadata(&ordner) {
        Ok(m) => {
            if !m.is_dir() || m.uid() != besitzer {
                return Err(verweigert("Der Ordner „wai“ gehört nicht dem Benutzer."));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new().mode(0o700).create(&ordner)?;
        }
        Err(e) => return Err(e),
    }
    std::fs::set_permissions(&ordner, std::fs::Permissions::from_mode(0o700))?;

    let pfad = socket_pfad(laufzeit);
    if let Ok(m) = std::fs::symlink_metadata(&pfad) {
        if !m.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Am Socket-Pfad liegt etwas anderes als ein Socket.",
            ));
        }
        if std::os::unix::net::UnixStream::connect(&pfad).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "Der Sprachdienst läuft schon (Socket belegt).",
            ));
        }
        std::fs::remove_file(&pfad)?;
    }

    let listener = UnixListener::bind(&pfad)?;
    std::fs::set_permissions(&pfad, std::fs::Permissions::from_mode(0o600))?;
    let sm = std::fs::symlink_metadata(&pfad)?;
    if sm.uid() != besitzer {
        let _ = std::fs::remove_file(&pfad);
        return Err(verweigert(
            "Der Socket gehört nicht dem Benutzer des Laufzeitordners.",
        ));
    }
    Ok(Gebunden {
        listener,
        pfad,
        uid: sm.uid(),
    })
}

/// Nimmt Verbindungen an, bis `stopp` ausgelöst wird, und entfernt dann den Socket.
pub async fn annehmen(
    g: Gebunden,
    erwartete_uid: u32,
    ctx: Arc<Kontext>,
    mut stopp: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            r = g.listener.accept() => match r {
                Ok((s, _)) => einlassen(s, erwartete_uid, &ctx, &stopp),
                Err(e) => {
                    log::warn!("sprachdienst: accept fehlgeschlagen: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            },
            _ = stopp.changed() => break,
        }
    }
    let _ = std::fs::remove_file(&g.pfad);
}

fn einlassen(s: UnixStream, erwartete_uid: u32, ctx: &Arc<Kontext>, stopp: &watch::Receiver<bool>) {
    let gleicher_benutzer = s
        .peer_cred()
        .map(|c| c.uid() == erwartete_uid)
        .unwrap_or(false);
    if !gleicher_benutzer {
        log::warn!("sprachdienst: Gegenstelle eines anderen Benutzers abgewiesen");
        return;
    }
    // Je Gegenstelle (Prozess) höchstens `je_gegenstelle` Verbindungen schon vor dem Handschlag;
    // ohne lesbare Prozess-ID gibt es keine Zählung, nur die Gesamtgrenze.
    let pid = s
        .peer_cred()
        .ok()
        .and_then(|c| c.pid())
        .and_then(|p| u32::try_from(p).ok());
    let gegenstelle = match pid {
        Some(pid) => match ctx.gegenstelle(pid) {
            Some(m) => Some(m),
            None => {
                log::warn!("sprachdienst: zu viele Verbindungen einer Gegenstelle abgewiesen");
                return;
            }
        },
        None => None,
    };
    let Some(platz) = ctx.platz() else {
        log::warn!("sprachdienst: alle Verbindungsplätze belegt");
        return;
    };
    let (ctx, stopp) = (ctx.clone(), stopp.clone());
    tokio::spawn(async move {
        let _platz = platz;
        let _gegenstelle = gegenstelle;
        verbindung(s, ctx, stopp).await;
    });
}
