//! Entwicklungskanal: Testversionen ohne Zutun installieren.
//!
//! Lokale Abnahme-Bauten (`scripts/build-abnahme.sh`) landen in einem der
//! Update-Ordner aus `local_update`. Bisher bot die App sie nur an; wer sie
//! einspielen wollte, musste klicken oder die App von Hand beenden. Hier
//! schaut ein Hintergrund-Thread regelmaessig nach und spielt eine neuere
//! Testversion selbst ein, wie das Online-Update: App beenden, Installer
//! passiv, neue App starten.
//!
//! Vertrauensgrenzen (Codex-Review 06.11.2026):
//! - Eigener ENTWICKLUNGS-Schluessel. Sein Pubkey wird nur in Abnahme-Bauten
//!   eingebacken (`LVA_DEV_UPDATE_PUBKEY`, setzt allein `build-abnahme.sh`);
//!   Release-Bauten haben keinen und kennen den Kanal gar nicht. Der
//!   Produktionsschluessel signiert nie agentengebaute Testversionen.
//! - Geprueft wird eine private Kopie (Staging im App-Datenordner), gestartet
//!   wird genau diese Kopie. Symlinks/Junctions im Update-Ordner zaehlen nicht.
//! - Gegen Downgrade und Wiederholung: der signierte Zeitstempel (trusted
//!   comment der Minisign-Signatur) muss groesser sein als der zuletzt
//!   akzeptierte, und der signierte Dateiname muss zum Installer passen.
//! - Nie mitten in einer Arbeit (Diktat, Aufnahme, Nachverarbeitung,
//!   Import-Warteschlange, schwerer Rechenauftrag, Vorlesen); vorher ein
//!   Hinweis mit „Später“.
//! - Dauerhafter Status: genau ein Versuch je Installer. Startet danach nicht
//!   die neue Version, wird ihr Hash gesperrt. Der zuletzt erfolgreich
//!   eingespielte Installer und eine Sicherung der Einstellungen/Datenbanken
//!   bleiben im Staging-Ordner fuer die Rueckkehr von Hand.

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

/// Pubkey des Entwicklungsschluessels (Tauri-Form: Base64 der Minisign-
/// Textform). Nur Abnahme-Bauten haben ihn; ohne ihn gibt es keinen Kanal.
const DEV_PUBKEY: Option<&str> = option_env!("LVA_DEV_UPDATE_PUBKEY");

/// Abstand zwischen zwei Blicken in die Update-Ordner.
const INTERVALL: Duration = Duration::from_secs(20);
/// So lange steht der Hinweis mit „Später“, bevor die App sich beendet.
const VORLAUF: Duration = Duration::from_secs(15);
/// „Später“ schiebt so lange auf.
const AUFSCHUB: Duration = Duration::from_secs(15 * 60);
/// Groesste Signaturdatei, die gelesen wird (echte sind ~0,5 KB).
const MAX_SIG_BYTES: u64 = 8 * 1024;
/// Obergrenze der Datensicherung vor dem Einspielen.
const MAX_SICHERUNG_BYTES: u64 = 512 * 1024 * 1024;
/// Eine Sperre, die aelter ist, gilt als liegengeblieben.
const SPERRE_MAX_ALTER: Duration = Duration::from_secs(10 * 60);

/// Ereignis an die Oberflaeche (Hinweis unten rechts).
pub const EVENT: &str = "lokales-testupdate";

/// Bis wann „Später“ gilt (Unix-Sekunden).
static AUFGESCHOBEN_BIS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Type)]
pub struct TestupdateHinweis {
    pub version: String,
    /// `wartet` (Arbeit laeuft noch, Grund in `grund`) oder `startet`.
    pub phase: String,
    pub grund: Option<String>,
    /// Sekunden bis zum Beenden (nur bei `startet`).
    pub sekunden: u32,
}

/// Hat dieser Bau den Entwicklungskanal (Abnahme-Bau)?
#[tauri::command]
#[specta::specta]
pub fn dev_update_available() -> bool {
    DEV_PUBKEY.is_some()
}

/// „Später“ im Hinweis: das Einspielen ruht eine Weile.
#[tauri::command]
#[specta::specta]
pub fn dev_update_postpone() {
    AUFGESCHOBEN_BIS.store(jetzt() + AUFSCHUB.as_secs(), Ordering::Release);
    log::info!(
        "dev update: aufgeschoben um {} min",
        AUFSCHUB.as_secs() / 60
    );
}

/// Ist der Kanal fuer diese Einstellung aktiv? Nur in Abnahme-Bauten; dort
/// ist er ohne Wahl an, ein gespeichertes „aus“ bleibt aus.
pub fn aktiv(kanal_vorhanden: bool, einstellung: Option<bool>) -> bool {
    kanal_vorhanden && einstellung.unwrap_or(true)
}

fn jetzt() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sig_path(installer: &Path) -> PathBuf {
    let mut name = installer.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

fn b64_text(s: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| format!("Base64: {e}"))?;
    String::from_utf8(bytes).map_err(|_| "kein UTF-8".to_string())
}

/// Symlink oder (unter Windows) Reparse-Punkt wie eine Junction?
fn ist_umleitung(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return true;
    };
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Signierte Angaben aus dem trusted comment (`timestamp:N\tfile:NAME`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Signiert {
    pub zeit: u64,
    pub datei: String,
    /// Neuere Tauri-CLIs signieren auch die App-Version mit.
    pub version: Option<String>,
}

pub(crate) fn signierte_angaben(trusted: &str) -> Option<Signiert> {
    let mut zeit = None;
    let mut datei = None;
    let mut version = None;
    for teil in trusted.split('\t') {
        if let Some(v) = teil.strip_prefix("timestamp:") {
            zeit = v.trim().parse().ok();
        } else if let Some(v) = teil.strip_prefix("file:") {
            datei = Some(v.trim().to_string());
        } else if let Some(v) = teil.strip_prefix("version:") {
            version = Some(v.trim().to_string());
        }
    }
    Some(Signiert {
        zeit: zeit?,
        datei: datei.filter(|d| !d.is_empty())?,
        version: version.filter(|v| !v.is_empty()),
    })
}

/// Ergebnis einer erfolgreichen Pruefung.
#[derive(Debug)]
pub(crate) struct Geprueft {
    pub signiert: Signiert,
    pub sha256: String,
}

/// Kopiert `quelle` nach `ziel` und prueft dabei die Signatur `sig_b64`
/// gegen `pubkey_b64`; eine mitsignierte Version muss `version` sein. Gilt nur, wenn alles passt; sonst wird `ziel` entfernt.
/// Die Signatur wird VOR dem Kopieren gelesen und gilt fuer genau die Bytes,
/// die in `ziel` stehen — was spaeter im Update-Ordner passiert, zaehlt nicht.
pub(crate) fn kopieren_und_pruefen(
    quelle: &Path,
    sig_b64: &str,
    pubkey_b64: &str,
    version: &str,
    ziel: &Path,
) -> Result<Geprueft, String> {
    let public_key =
        PublicKey::decode(&b64_text(pubkey_b64)?).map_err(|e| format!("Schluessel: {e}"))?;
    let signature = Signature::decode(&b64_text(sig_b64)?).map_err(|e| format!("Signatur: {e}"))?;
    // Tauri signiert vorgehasht (Strom-Pruefung). Fuer eine alte, nicht
    // vorgehashte Signatur wird nach dem Kopieren die private Kopie im
    // Ganzen geprueft.
    let mut verifier = match public_key.verify_stream(&signature) {
        Ok(v) => Some(v),
        Err(minisign_verify::Error::UnsupportedLegacyMode) => None,
        Err(e) => return Err(format!("Signatur passt nicht: {e}")),
    };
    let ergebnis = (|| {
        let mut ein = std::fs::File::open(quelle).map_err(|e| format!("Installer: {e}"))?;
        let mut aus = std::fs::File::create(ziel).map_err(|e| format!("Staging: {e}"))?;
        let mut sha = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = ein.read(&mut buf).map_err(|e| format!("Installer: {e}"))?;
            if n == 0 {
                break;
            }
            if let Some(v) = verifier.as_mut() {
                v.update(&buf[..n]);
            }
            sha.update(&buf[..n]);
            aus.write_all(&buf[..n])
                .map_err(|e| format!("Staging: {e}"))?;
        }
        aus.sync_all().map_err(|e| format!("Staging: {e}"))?;
        match verifier.as_mut() {
            Some(v) => v.finalize(),
            None => {
                drop(aus);
                let daten = std::fs::read(ziel).map_err(|e| format!("Staging: {e}"))?;
                public_key.verify(&daten, &signature, true)
            }
        }
        .map_err(|e| format!("Signatur ungueltig: {e}"))?;
        let signiert = signierte_angaben(signature.trusted_comment())
            .ok_or("Signatur ohne Zeitstempel/Dateiname")?;
        let name = quelle
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if signiert.datei != name {
            return Err(format!(
                "Signatur gehoert zu {}, nicht zu {name}",
                signiert.datei
            ));
        }
        if signiert.version.as_deref().is_some_and(|v| v != version) {
            return Err(format!(
                "signierte Version {:?} passt nicht zu {version}",
                signiert.version
            ));
        }
        Ok(Geprueft {
            signiert,
            sha256: format!("{:x}", sha.finalize()),
        })
    })();
    if ergebnis.is_err() {
        let _ = std::fs::remove_file(ziel);
    }
    ergebnis
}

/// Dauerhafter Zustand im Staging-Ordner (`status.json`).
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Status {
    /// Signierter Zeitstempel des zuletzt erfolgreich eingespielten Baus.
    #[serde(default)]
    pub zuletzt_zeit: u64,
    /// Hashes, die nicht (wieder) eingespielt werden.
    #[serde(default)]
    pub gesperrt: Vec<String>,
    /// Laufender Versuch: wird beim naechsten Start aufgeloest.
    #[serde(default)]
    pub laufend: Option<Versuch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Versuch {
    pub version: String,
    pub zeit: u64,
    pub sha256: String,
}

/// Ergebnis beim Start: was aus dem letzten Versuch wurde.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Ausgang {
    Keiner,
    Erfolg,
    Fehlschlag,
}

impl Status {
    /// Beim Start: lief die Zielversion an, ist der Versuch geglueckt;
    /// sonst wird sein Hash gesperrt (genau ein Versuch je Installer).
    pub(crate) fn aufloesen(&mut self, laufende_version: &str) -> Ausgang {
        let Some(v) = self.laufend.take() else {
            return Ausgang::Keiner;
        };
        if v.version == laufende_version {
            self.zuletzt_zeit = self.zuletzt_zeit.max(v.zeit);
            Ausgang::Erfolg
        } else {
            if !self.gesperrt.contains(&v.sha256) {
                self.gesperrt.push(v.sha256);
            }
            Ausgang::Fehlschlag
        }
    }

    /// Darf ein geprueftes Artefakt eingespielt werden?
    pub(crate) fn zulassen(&self, g: &Geprueft) -> Result<(), String> {
        if self.gesperrt.contains(&g.sha256) {
            return Err("Installer ist gesperrt (frueherer Fehlschlag)".into());
        }
        if g.signiert.zeit <= self.zuletzt_zeit {
            return Err(format!(
                "signierter Zeitstempel {} nicht neuer als {} (Downgrade/Wiederholung)",
                g.signiert.zeit, self.zuletzt_zeit
            ));
        }
        Ok(())
    }
}

/// Fingerabdruck eines Funds: Pfad, Groesse und Zeit von Installer und Signatur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stand {
    path: PathBuf,
    len: u64,
    mtime: Option<SystemTime>,
    sig_len: Option<u64>,
    sig_mtime: Option<SystemTime>,
}

impl Stand {
    pub(crate) fn lesen(installer: &Path) -> Option<Self> {
        let meta = std::fs::metadata(installer).ok()?;
        let sig = std::fs::metadata(sig_path(installer)).ok();
        Some(Self {
            path: installer.to_path_buf(),
            len: meta.len(),
            mtime: meta.modified().ok(),
            sig_len: sig.as_ref().map(|m| m.len()),
            sig_mtime: sig.and_then(|m| m.modified().ok()),
        })
    }
}

/// Was der Thread mit einem Fund tun soll.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Schritt {
    /// Neu oder noch in Bewegung: beim naechsten Mal wieder ansehen.
    Abwarten,
    /// Signatur fehlt oder Pruefung schlug fehl: liegen lassen (Klick geht weiter).
    Ablehnen,
    /// Unveraendert seit der letzten Runde: kopieren, pruefen, einspielen.
    Weiter,
}

#[derive(Debug, Default)]
pub(crate) struct Gedaechtnis {
    zuletzt: Option<Stand>,
    abgelehnt: Option<Stand>,
    wartet_gemeldet: Option<Stand>,
}

impl Gedaechtnis {
    pub(crate) fn schritt(&mut self, stand: Stand) -> Schritt {
        if self.zuletzt.as_ref() != Some(&stand) {
            self.zuletzt = Some(stand);
            return Schritt::Abwarten;
        }
        if self.abgelehnt.as_ref() == Some(&stand) || stand.sig_len.is_none() {
            return Schritt::Ablehnen;
        }
        Schritt::Weiter
    }
}

fn staging_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_local_data_dir().ok()?.join("testupdate");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn status_lesen(dir: &Path) -> Status {
    std::fs::read_to_string(dir.join("status.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Atomar schreiben (temp + rename), damit ein Absturz keinen halben Status hinterlaesst.
fn status_schreiben(dir: &Path, status: &Status) -> Result<(), String> {
    let tmp = dir.join("status.json.tmp");
    let json = serde_json::to_string_pretty(status).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, json).map_err(|e| format!("Status: {e}"))?;
    std::fs::rename(&tmp, dir.join("status.json")).map_err(|e| format!("Status: {e}"))
}

/// Gemeinsame Installationssperre (auch fuer eine zweite Instanz).
struct Sperre(PathBuf);

impl Sperre {
    fn nehmen(dir: &Path) -> Option<Self> {
        let path = dir.join("install.lock");
        if let Ok(meta) = std::fs::metadata(&path) {
            let alt = meta
                .modified()
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|a| a > SPERRE_MAX_ALTER);
            if !alt {
                return None;
            }
            let _ = std::fs::remove_file(&path);
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ok()?;
        let _ = writeln!(f, "{}", std::process::id());
        Some(Self(path))
    }

    /// Erfolgreich uebergeben: die Sperre bleibt bis zum naechsten Start liegen.
    fn behalten(self) {
        std::mem::forget(self);
    }
}

impl Drop for Sperre {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Sicherung der Einstellungen und Datenbanken (oberste Ebene des
/// Datenordners) vor dem Einspielen; eine Generation, bis 512 MB.
fn sichern(app: &AppHandle, staging: &Path) -> Result<(), String> {
    let Ok(daten) = app.path().app_data_dir() else {
        return Ok(());
    };
    let ziel = staging.join("sicherung");
    let _ = std::fs::remove_dir_all(&ziel);
    std::fs::create_dir_all(&ziel).map_err(|e| format!("Sicherung: {e}"))?;
    let mut summe = 0u64;
    for entry in std::fs::read_dir(&daten)
        .map_err(|e| format!("Sicherung: {e}"))?
        .flatten()
    {
        let path = entry.path();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !matches!(
            ext.as_str(),
            "json" | "db" | "sqlite" | "sqlite3" | "db-wal"
        ) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        summe += meta.len();
        if summe > MAX_SICHERUNG_BYTES {
            log::warn!("dev update: Sicherung bei 512 MB abgebrochen, Rest nicht gesichert");
            break;
        }
        std::fs::copy(&path, ziel.join(entry.file_name()))
            .map_err(|e| format!("Sicherung {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Woran die App gerade arbeitet; `None` = frei fuer einen Neustart.
fn beschaeftigt(app: &AppHandle) -> Option<&'static str> {
    use crate::managers;
    use managers::meetings::{queue::ImportQueue, recorder::MeetingRecorderManager};
    if app
        .try_state::<Arc<managers::audio::AudioRecordingManager>>()
        .is_some_and(|m| m.is_recording())
    {
        return Some("Diktat");
    }
    if let Some(r) = app.try_state::<Arc<MeetingRecorderManager>>() {
        if r.is_recording() {
            return Some("Besprechungsaufnahme");
        }
        if r.final_jobs_active() {
            return Some("Nachverarbeitung");
        }
    }
    if app
        .try_state::<Arc<ImportQueue>>()
        .is_some_and(|q| !q.wait_idle(Duration::ZERO))
    {
        return Some("Import-Warteschlange");
    }
    if managers::workflows::heavy::LocalHeavyGate::shared().is_busy() {
        return Some("Rechenauftrag");
    }
    if app
        .try_state::<Arc<managers::tts::TtsManager>>()
        .is_some_and(|m| m.status().phase == managers::tts::state::TtsPhase::Speaking)
    {
        return Some("Vorlesen");
    }
    None
}

fn melden(app: &AppHandle, version: &str, phase: &str, grund: Option<&str>, sekunden: u32) {
    let _ = app.emit(
        EVENT,
        TestupdateHinweis {
            version: version.to_string(),
            phase: phase.to_string(),
            grund: grund.map(str::to_string),
            sekunden,
        },
    );
}

fn aufgeschoben() -> bool {
    jetzt() < AUFGESCHOBEN_BIS.load(Ordering::Acquire)
}

/// Ein Blick in die Ordner. `true`, wenn der Installer gestartet ist.
fn runde(app: &AppHandle, pubkey: &str, staging: &Path, g: &mut Gedaechtnis) -> bool {
    let settings = crate::settings::get_settings(app);
    if !aktiv(true, settings.local_update_auto_install) || aufgeschoben() {
        return false;
    }
    let Some(update) = crate::local_update::find_newest(app) else {
        return false;
    };
    let installer = PathBuf::from(&update.path);
    let ordner = installer
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    if ist_umleitung(&installer) || ist_umleitung(&ordner) || ist_umleitung(&sig_path(&installer)) {
        if g.abgelehnt.is_none() {
            log::warn!(
                "dev update: {} ist Symlink/Junction, abgelehnt",
                update.file_name
            );
        }
        g.abgelehnt = Stand::lesen(&installer);
        return false;
    }
    let Some(stand) = Stand::lesen(&installer) else {
        return false;
    };
    match g.schritt(stand.clone()) {
        Schritt::Abwarten => return false,
        Schritt::Ablehnen => {
            if g.abgelehnt.as_ref() != Some(&stand) {
                log::warn!(
                    "dev update: {} ohne Signatur, nur Angebot per Klick",
                    update.file_name
                );
                g.abgelehnt = Some(stand);
            }
            return false;
        }
        Schritt::Weiter => {}
    }
    if let Some(grund) = beschaeftigt(app) {
        if g.wartet_gemeldet.as_ref() != Some(&stand) {
            log::info!("dev update: {} wartet ({grund})", update.version);
            melden(app, &update.version, "wartet", Some(grund), 0);
            g.wartet_gemeldet = Some(stand);
        }
        return false;
    }
    let Some(sperre) = Sperre::nehmen(staging) else {
        return false;
    };
    let mut status = status_lesen(staging);
    // Signatur vor dem Kopieren lesen; danach gilt nur noch die private Kopie.
    let sig = match std::fs::metadata(sig_path(&installer)) {
        Ok(m) if m.len() <= MAX_SIG_BYTES => std::fs::read_to_string(sig_path(&installer)).ok(),
        _ => None,
    };
    let Some(sig) = sig else {
        g.abgelehnt = Some(stand);
        return false;
    };
    let kandidat = staging.join("kandidat-setup.exe");
    let geprueft = kopieren_und_pruefen(&installer, &sig, pubkey, &update.version, &kandidat)
        .and_then(|gp| status.zulassen(&gp).map(|()| gp));
    let geprueft = match geprueft {
        Ok(gp) => gp,
        Err(e) => {
            log::warn!("dev update: {} abgelehnt: {e}", update.file_name);
            let _ = std::fs::remove_file(&kandidat);
            g.abgelehnt = Some(stand);
            return false;
        }
    };
    log::info!(
        "dev update: {} geprueft (signiert {}, sha256 {})",
        update.version,
        geprueft.signiert.zeit,
        geprueft.sha256
    );

    let sekunden = VORLAUF.as_secs() as u32;
    melden(app, &update.version, "startet", None, sekunden);
    std::thread::sleep(VORLAUF);
    // Unmittelbar vor dem Beenden erneut: Aufschub, neue Arbeit, Einstellung.
    let settings = crate::settings::get_settings(app);
    if aufgeschoben()
        || !aktiv(true, settings.local_update_auto_install)
        || beschaeftigt(app).is_some()
    {
        log::info!(
            "dev update: {} abgebrochen (Aufschub/Arbeit)",
            update.version
        );
        let _ = std::fs::remove_file(&kandidat);
        return false;
    }
    if let Err(e) = sichern(app, staging) {
        log::warn!("dev update: Sicherung gescheitert, kein Update: {e}");
        let _ = std::fs::remove_file(&kandidat);
        return false;
    }
    status.laufend = Some(Versuch {
        version: update.version.clone(),
        zeit: geprueft.signiert.zeit,
        sha256: geprueft.sha256.clone(),
    });
    if let Err(e) = status_schreiben(staging, &status) {
        log::warn!("dev update: {e}; kein Update");
        let _ = std::fs::remove_file(&kandidat);
        return false;
    }
    log::info!("dev update: installiere {} ohne Rueckfrage", update.version);
    match crate::local_update::install_staged(app, &kandidat) {
        Ok(()) => {
            sperre.behalten();
            true
        }
        Err(e) => {
            log::warn!(
                "dev update: Start von {} gescheitert: {e}",
                update.file_name
            );
            status.laufend = None;
            if !status.gesperrt.contains(&geprueft.sha256) {
                status.gesperrt.push(geprueft.sha256);
            }
            let _ = status_schreiben(staging, &status);
            let _ = std::fs::remove_file(&kandidat);
            g.abgelehnt = Some(stand);
            false
        }
    }
}

/// Beim Start: Ausgang des letzten Versuchs festhalten, Sperre loesen.
fn nach_start(app: &AppHandle, staging: &Path) {
    let mut status = status_lesen(staging);
    let version = app.package_info().version.to_string();
    match status.aufloesen(&version) {
        Ausgang::Keiner => {}
        Ausgang::Erfolg => {
            log::info!("dev update: {version} laeuft, Versuch erfolgreich");
            // Fuer die Rueckkehr von Hand: der zuletzt gute Installer.
            let _ = std::fs::rename(
                staging.join("kandidat-setup.exe"),
                staging.join("letzte-gute-setup.exe"),
            );
        }
        Ausgang::Fehlschlag => {
            log::warn!("dev update: nach dem Versuch laeuft weiter {version}; Installer gesperrt");
            let _ = std::fs::remove_file(staging.join("kandidat-setup.exe"));
        }
    }
    if let Err(e) = status_schreiben(staging, &status) {
        log::warn!("dev update: {e}");
    }
    let _ = std::fs::remove_file(staging.join("install.lock"));
}

/// Startet den Hintergrund-Thread. Nur Windows (nur dort gibt es den
/// Installer-Helfer; macOS folgt) und nur in Bauten mit Entwicklungsschluessel.
pub fn start(app: AppHandle) {
    if !cfg!(target_os = "windows") {
        return;
    }
    let Some(pubkey) = DEV_PUBKEY.map(str::to_string) else {
        return;
    };
    let Some(staging) = staging_dir(&app) else {
        log::warn!("dev update: kein Staging-Ordner, Entwicklungskanal aus");
        return;
    };
    nach_start(&app, &staging);
    log::info!(
        "dev update: Entwicklungskanal aktiv, Staging {}",
        staging.display()
    );
    let spawned = std::thread::Builder::new()
        .name("dev-update".into())
        .spawn(move || {
            let mut g = Gedaechtnis::default();
            loop {
                std::thread::sleep(INTERVALL);
                if runde(&app, &pubkey, &staging, &mut g) {
                    break;
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("dev update: Thread nicht gestartet: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Testvektor aus minisign-verify (vorgehasht, Daten "test", file:test), in
    // die Tauri-Form gebracht: Base64 der Minisign-Textform.
    const PK: &str = "untrusted comment: minisign public key E7620F1842B4E81F\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    const SIG: &str = "untrusted comment: signature from minisign secret key\nRUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\ntrusted comment: timestamp:1556193335\tfile:test\ny/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==";
    const ZEIT: u64 = 1556193335;

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    /// Der signierte Dateiname im Vektor ist `test`.
    fn quelle(dir: &Path, data: &[u8]) -> PathBuf {
        let p = dir.join("test");
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn gueltige_signatur_ergibt_kopie_und_angaben() {
        let dir = tempfile::tempdir().unwrap();
        let q = quelle(dir.path(), b"test");
        let ziel = dir.path().join("kandidat");
        let g = kopieren_und_pruefen(&q, &b64(SIG), &b64(PK), "0.99.1", &ziel).unwrap();
        assert_eq!(
            g.signiert,
            Signiert {
                zeit: ZEIT,
                datei: "test".into(),
                version: None
            }
        );
        assert_eq!(std::fs::read(&ziel).unwrap(), b"test");
        assert_eq!(
            g.sha256,
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }

    #[test]
    fn alte_nicht_vorgehashte_signatur_geht_auch() {
        const LEGACY: &str = "untrusted comment: signature from minisign secret key\nRWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\ntrusted comment: timestamp:1555779966\tfile:test\nQtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";
        let dir = tempfile::tempdir().unwrap();
        let q = quelle(dir.path(), b"test");
        let ziel = dir.path().join("kandidat");
        let g = kopieren_und_pruefen(&q, &b64(LEGACY), &b64(PK), "0.99.1", &ziel).unwrap();
        assert_eq!(g.signiert.zeit, 1555779966);
        let q2 = quelle(dir.path(), b"Test");
        assert!(kopieren_und_pruefen(&q2, &b64(LEGACY), &b64(PK), "0.99.1", &ziel).is_err());
        assert!(!ziel.exists());
    }

    #[test]
    fn veraenderter_installer_wird_abgelehnt_und_kopie_entfernt() {
        let dir = tempfile::tempdir().unwrap();
        let q = quelle(dir.path(), b"Test");
        let ziel = dir.path().join("kandidat");
        assert!(kopieren_und_pruefen(&q, &b64(SIG), &b64(PK), "0.99.1", &ziel).is_err());
        assert!(!ziel.exists());
    }

    #[test]
    fn anderer_dateiname_als_signiert_wird_abgelehnt() {
        let dir = tempfile::tempdir().unwrap();
        let q = dir.path().join("Local Voice AI_0.99.1_x64-setup.exe");
        std::fs::write(&q, b"test").unwrap();
        let ziel = dir.path().join("kandidat");
        let e = kopieren_und_pruefen(&q, &b64(SIG), &b64(PK), "0.99.1", &ziel).unwrap_err();
        assert!(e.contains("gehoert zu test"), "{e}");
    }

    #[test]
    fn produktionsschluessel_nimmt_nichts_an() {
        let dir = tempfile::tempdir().unwrap();
        let q = quelle(dir.path(), b"test");
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let prod = conf["plugins"]["updater"]["pubkey"].as_str().unwrap();
        assert!(
            kopieren_und_pruefen(&q, &b64(SIG), prod, "0.99.1", &dir.path().join("k")).is_err()
        );
    }

    #[test]
    fn trusted_comment_wird_gelesen() {
        assert_eq!(
            signierte_angaben("timestamp:42\tfile:a b_1.0.0_x64-setup.exe"),
            Some(Signiert {
                zeit: 42,
                datei: "a b_1.0.0_x64-setup.exe".into(),
                version: None
            })
        );
        assert_eq!(
            signierte_angaben("timestamp:42\tfile:x\tversion:0.23.5").and_then(|s| s.version),
            Some("0.23.5".to_string())
        );
        assert_eq!(signierte_angaben("timestamp:42"), None);
        assert_eq!(signierte_angaben("file:x"), None);
    }

    fn geprueft(zeit: u64, sha: &str) -> Geprueft {
        Geprueft {
            signiert: Signiert {
                zeit,
                datei: "x".into(),
                version: None,
            },
            sha256: sha.into(),
        }
    }

    #[test]
    fn downgrade_wiederholung_und_sperre() {
        let mut s = Status {
            zuletzt_zeit: 100,
            ..Default::default()
        };
        assert!(s.zulassen(&geprueft(100, "a")).is_err(), "Wiederholung");
        assert!(s.zulassen(&geprueft(99, "a")).is_err(), "Downgrade");
        assert!(s.zulassen(&geprueft(101, "a")).is_ok());
        s.gesperrt.push("a".into());
        assert!(s.zulassen(&geprueft(200, "a")).is_err(), "gesperrt");
    }

    #[test]
    fn genau_ein_versuch_je_installer() {
        let mut s = Status {
            zuletzt_zeit: 100,
            laufend: Some(Versuch {
                version: "0.23.5".into(),
                zeit: 150,
                sha256: "h".into(),
            }),
            ..Default::default()
        };
        // Neue Version laeuft: Erfolg, Zeitstempel ruecken nach.
        let mut ok = s.clone();
        assert_eq!(ok.aufloesen("0.23.5"), Ausgang::Erfolg);
        assert_eq!(ok.zuletzt_zeit, 150);
        assert!(ok.laufend.is_none());
        // Alte Version laeuft weiter: Hash gesperrt, kein zweiter Versuch.
        assert_eq!(s.aufloesen("0.23.4"), Ausgang::Fehlschlag);
        assert_eq!(s.gesperrt, vec!["h".to_string()]);
        assert_eq!(s.zuletzt_zeit, 100);
        assert_eq!(s.aufloesen("0.23.4"), Ausgang::Keiner);
    }

    #[test]
    fn status_ueberlebt_schreiben_und_lesen() {
        let dir = tempfile::tempdir().unwrap();
        let s = Status {
            zuletzt_zeit: 7,
            gesperrt: vec!["x".into()],
            laufend: None,
        };
        status_schreiben(dir.path(), &s).unwrap();
        assert_eq!(status_lesen(dir.path()), s);
        assert_eq!(status_lesen(&dir.path().join("fehlt")), Status::default());
    }

    #[test]
    fn sperre_ist_exklusiv() {
        let dir = tempfile::tempdir().unwrap();
        let a = Sperre::nehmen(dir.path()).expect("erste");
        assert!(Sperre::nehmen(dir.path()).is_none(), "zweite");
        drop(a);
        assert!(Sperre::nehmen(dir.path()).is_some(), "nach Freigabe");
    }

    #[test]
    fn erst_unveraendert_dann_weiter() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Local Voice AI_0.99.1_x64-setup.exe");
        std::fs::write(&p, b"test").unwrap();
        std::fs::write(sig_path(&p), b64(SIG)).unwrap();
        let mut g = Gedaechtnis::default();
        let s = Stand::lesen(&p).unwrap();
        assert_eq!(g.schritt(s.clone()), Schritt::Abwarten);
        assert_eq!(g.schritt(s), Schritt::Weiter);
        std::fs::write(&p, b"test-und-mehr").unwrap();
        let s2 = Stand::lesen(&p).unwrap();
        assert_eq!(g.schritt(s2.clone()), Schritt::Abwarten);
        assert_eq!(g.schritt(s2), Schritt::Weiter);
    }

    #[test]
    fn ohne_signatur_nie_automatisch() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Local Voice AI_0.99.1_x64-setup.exe");
        std::fs::write(&p, b"test").unwrap();
        let mut g = Gedaechtnis::default();
        let s = Stand::lesen(&p).unwrap();
        assert_eq!(g.schritt(s.clone()), Schritt::Abwarten);
        assert_eq!(g.schritt(s), Schritt::Ablehnen);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_zaehlt_als_umleitung() {
        let dir = tempfile::tempdir().unwrap();
        let p = quelle(dir.path(), b"test");
        let l = dir.path().join("link");
        std::os::unix::fs::symlink(&p, &l).unwrap();
        assert!(ist_umleitung(&l));
        assert!(!ist_umleitung(&p));
    }

    #[test]
    fn kanal_nur_mit_schluessel_und_aus_bleibt_aus() {
        assert!(!aktiv(false, Some(true)), "Release kann nicht einschalten");
        assert!(aktiv(true, None));
        assert!(!aktiv(true, Some(false)));
    }
}
