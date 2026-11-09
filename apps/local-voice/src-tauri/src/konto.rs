//! Suite-Profil (ADR-0024, Paket 1): Tauri-Befehle über dem gemeinsamen Kontospeicher
//! `wai-konto` (vendort unter `vendor/wai-konto`). Nur lokales Profil, keine Anmeldung.
//!
//! Fehlertexte stammen aus `KontoFehler` und enthalten nie Profilinhalte; hier wird
//! nichts protokolliert, was zu einem Profil gehört.

use std::sync::Mutex;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use wai_konto::{
    Beobachter, KontoFehler, KontoSpeicher, Profil, ProfilAenderung, ProfilDatei, ProfilEingabe,
};

/// Ereignis an das Frontend, sobald sich der Kontospeicher geändert hat (auch durch
/// andere Apps der Suite).
pub const EVENT_GEAENDERT: &str = "konto-geaendert";

/// Ein Slot, der erst beim Gebrauch gefüllt wird (lazy, idempotent). Scheitert das
/// Öffnen oder Beobachten, bleibt der Slot leer und der nächste Befehl versucht es
/// erneut – genau einmal je Befehl, ohne Schleife.
struct Slot<S, B> {
    speicher: Option<S>,
    beobachter: Option<B>,
}

impl<S, B> Default for Slot<S, B> {
    fn default() -> Self {
        Self {
            speicher: None,
            beobachter: None,
        }
    }
}

impl<S: Clone, B> Slot<S, B> {
    /// Liefert den Speicher; holt Öffnen und Beobachten nach, soweit sie fehlen. Ein
    /// Beobachtungsfehler macht den Speicher nicht unbenutzbar.
    fn sicherstellen(
        &mut self,
        oeffnen: impl FnOnce() -> Result<S, String>,
        beobachten: impl FnOnce(&S) -> Result<B, String>,
    ) -> Result<S, String> {
        if self.speicher.is_none() {
            self.speicher = Some(oeffnen()?);
        }
        let speicher = self.speicher.clone().ok_or_else(speicher_fehlt)?;
        if self.beobachter.is_none() {
            self.beobachter = beobachten(&speicher).ok();
        }
        Ok(speicher)
    }
}

fn speicher_fehlt() -> String {
    "Der Kontospeicher ist nicht verfügbar.".to_string()
}

/// Verwalteter Zustand. Der Mutex wird nur kurz und nie über ein `await` gehalten;
/// alle Dateizugriffe laufen in `spawn_blocking`. Ohne Speicher (Datenordner nicht
/// ermittelbar) bleibt die App benutzbar; die Befehle melden dann einen Fehler und
/// versuchen es beim nächsten Aufruf erneut.
#[derive(Default)]
pub struct KontoState {
    slot: Mutex<Slot<KontoSpeicher, Beobachter>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KontoAnmeldungDto {
    pub issuer: String,
    pub subject: String,
    pub seit_ms: u64,
    pub idp_felder: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KontoProfilDto {
    pub id: String,
    pub anzeigename: String,
    pub vorname: Option<String>,
    pub nachname: Option<String>,
    pub email: Option<String>,
    pub telefon: Option<String>,
    pub mobil: Option<String>,
    pub position: Option<String>,
    pub abteilung: Option<String>,
    pub organisation: Option<String>,
    pub standort: Option<String>,
    pub farbe: u8,
    pub hat_bild: bool,
    pub gesperrt: bool,
    /// Zeitpunkt der letzten Änderung des Profils (auch von Bild setzen/entfernen); das
    /// Frontend lädt das Bild nur nach, wenn sich dieser Wert ändert.
    pub geaendert_ms: u64,
    pub anmeldung: Option<KontoAnmeldungDto>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KontoStatus {
    pub aktiv: KontoProfilDto,
    pub weitere: Vec<KontoProfilDto>,
    pub sitzungsrevision: u64,
}

/// Teiländerung: fehlende oder `null`-Felder bleiben unverändert, ein leerer Text leert
/// ein optionales Feld.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KontoAenderung {
    pub anzeigename: Option<String>,
    pub vorname: Option<String>,
    pub nachname: Option<String>,
    pub email: Option<String>,
    pub telefon: Option<String>,
    pub mobil: Option<String>,
    pub position: Option<String>,
    pub abteilung: Option<String>,
    pub organisation: Option<String>,
    pub standort: Option<String>,
    pub farbe: Option<u8>,
}

impl From<Profil> for KontoProfilDto {
    fn from(p: Profil) -> Self {
        Self {
            id: p.id,
            anzeigename: p.anzeigename,
            vorname: p.vorname,
            nachname: p.nachname,
            email: p.email,
            telefon: p.telefon,
            mobil: p.mobil,
            position: p.position,
            abteilung: p.abteilung,
            organisation: p.organisation,
            standort: p.standort,
            farbe: p.farbe,
            hat_bild: p.hat_bild,
            gesperrt: p.gesperrt,
            geaendert_ms: p.geaendert_ms,
            anmeldung: p.anmeldung.map(|a| KontoAnmeldungDto {
                issuer: a.issuer,
                subject: a.subject,
                seit_ms: a.seit_ms,
                idp_felder: a.idp_felder,
            }),
        }
    }
}

impl From<KontoAenderung> for ProfilAenderung {
    fn from(a: KontoAenderung) -> Self {
        Self {
            anzeigename: a.anzeigename,
            vorname: a.vorname,
            nachname: a.nachname,
            email: a.email,
            telefon: a.telefon,
            mobil: a.mobil,
            position: a.position,
            abteilung: a.abteilung,
            organisation: a.organisation,
            standort: a.standort,
            farbe: a.farbe,
        }
    }
}

fn status_aus(datei: ProfilDatei) -> Result<KontoStatus, String> {
    let ProfilDatei {
        aktiv,
        profile,
        sitzungsrevision,
        ..
    } = datei;
    let mut aktives = None;
    let mut weitere = Vec::new();
    for p in profile {
        if p.id == aktiv {
            aktives = Some(KontoProfilDto::from(p));
        } else {
            weitere.push(KontoProfilDto::from(p));
        }
    }
    let aktiv = aktives.ok_or_else(|| "Aktives Profil nicht gefunden".to_string())?;
    Ok(KontoStatus {
        aktiv,
        weitere,
        sitzungsrevision,
    })
}

/// Meldung, wenn das gezeigte Profil nicht mehr das aktive ist (eine andere Suite-App
/// hat gewechselt). Die Prüfung macht wai-konto unter derselben Dateisperre wie das
/// Schreiben (`*_wenn_aktiv`).
const NICHT_AKTIV: &str =
    "Das aktive Profil hat sich geändert. Die Änderung wurde nicht übernommen.";

fn fehler(e: KontoFehler) -> String {
    match e {
        KontoFehler::NichtAktiv(_) => NICHT_AKTIV.to_string(),
        e => e.to_string(),
    }
}

/// Nur das Hauptfenster darf das Suite-Profil lesen und ändern. Die Fähigkeiten der
/// Fenster `recording_overlay` und `meeting_prompt` sind dieselben wie die des
/// Hauptfensters; ohne diese Prüfung könnte Code in einem davon Profile ändern.
fn fenster_erlaubt(label: &str) -> Result<(), String> {
    if label == "main" {
        Ok(())
    } else {
        Err("Das Suite-Profil lässt sich nur im Hauptfenster ändern.".to_string())
    }
}

fn nur_hauptfenster(fenster: &WebviewWindow) -> Result<(), String> {
    fenster_erlaubt(fenster.label())
}

impl KontoState {
    /// Blockierend (Dateisperre, Retry-Schlaf): nur aus `spawn_blocking` aufrufen.
    fn speicher(&self, app: &AppHandle) -> Result<KontoSpeicher, String> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.sicherstellen(
            || {
                KontoSpeicher::oeffnen_standard().map_err(|e| {
                    log::error!("Kontospeicher nicht verfügbar: {e}");
                    speicher_fehlt()
                })
            },
            |s| {
                let handle = app.clone();
                s.beobachten(move |_| {
                    let _ = handle.emit(EVENT_GEAENDERT, ());
                })
                .map_err(|e| {
                    log::warn!("Kontobeobachtung nicht gestartet: {e}");
                    e.to_string()
                })
            },
        )
    }
}

/// Legt den Zustand an. Der Speicher wird erst beim ersten Befehl geöffnet (und bei
/// Misserfolg beim nächsten erneut); der Start blockiert nicht auf Dateizugriffe.
pub fn init(app: &AppHandle) {
    app.manage(KontoState::default());
}

fn melden(app: &AppHandle) {
    let _ = app.emit(EVENT_GEAENDERT, ());
}

/// Führt `arbeit` mit dem Speicher auf einem Blocking-Thread aus.
async fn blockierend<T, F>(app: &AppHandle, arbeit: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&KontoSpeicher) -> Result<T, String> + Send + 'static,
{
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let speicher = app.state::<KontoState>().speicher(&app)?;
        arbeit(&speicher)
    })
    .await
    .map_err(|_| "Der Kontozugriff wurde abgebrochen.".to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn konto_status(fenster: WebviewWindow, app: AppHandle) -> Result<KontoStatus, String> {
    nur_hauptfenster(&fenster)?;
    blockierend(&app, |s| status_aus(s.lesen().map_err(fehler)?)).await
}

/// `id` ist das Profil, das die Oberfläche bearbeitet; es muss noch das aktive sein.
#[tauri::command]
#[specta::specta]
pub async fn konto_aendern(
    fenster: WebviewWindow,
    app: AppHandle,
    id: String,
    aenderung: KontoAenderung,
) -> Result<KontoProfilDto, String> {
    nur_hauptfenster(&fenster)?;
    let profil = blockierend(&app, move |s| {
        s.profil_aendern_wenn_aktiv(&id, aenderung.into())
            .map_err(fehler)
    })
    .await?;
    melden(&app);
    Ok(profil.into())
}

#[tauri::command]
#[specta::specta]
pub async fn konto_aktivieren(
    fenster: WebviewWindow,
    app: AppHandle,
    id: String,
) -> Result<KontoStatus, String> {
    nur_hauptfenster(&fenster)?;
    let status = blockierend(&app, move |s| {
        s.aktivieren(&id).map_err(fehler)?;
        status_aus(s.lesen().map_err(fehler)?)
    })
    .await?;
    melden(&app);
    Ok(status)
}

/// Legt ein Profil an und macht es in derselben Sperre zum aktiven Profil.
#[tauri::command]
#[specta::specta]
pub async fn konto_anlegen(
    fenster: WebviewWindow,
    app: AppHandle,
    anzeigename: String,
) -> Result<KontoProfilDto, String> {
    nur_hauptfenster(&fenster)?;
    let profil = blockierend(&app, move |s| {
        s.profil_anlegen_und_aktivieren(ProfilEingabe {
            anzeigename,
            ..ProfilEingabe::default()
        })
        .map_err(fehler)
    })
    .await?;
    melden(&app);
    Ok(profil.into())
}

/// Maximale Länge des Base64-Textes: 256 KB Bild sind höchstens ~342 KB Base64.
const BILD_BASE64_MAX: usize = 256 * 1024 / 3 * 4 + 8;

fn bild_dekodieren(bytes: &str) -> Result<Vec<u8>, String> {
    // Vor dem Dekodieren begrenzen.
    if bytes.len() > BILD_BASE64_MAX {
        return Err("Das Profilbild ist größer als 256 KB.".to_string());
    }
    B64.decode(bytes.trim())
        .map_err(|_| "Das Profilbild ist nicht lesbar.".to_string())
}

/// Bild als Base64 (PNG, höchstens 256 KB; die Prüfung macht `wai-konto`). `id` muss
/// das aktive Profil sein.
#[tauri::command]
#[specta::specta]
pub async fn konto_bild_setzen(
    fenster: WebviewWindow,
    app: AppHandle,
    id: String,
    bytes: String,
) -> Result<(), String> {
    nur_hauptfenster(&fenster)?;
    let png = bild_dekodieren(&bytes)?;
    drop(bytes);
    blockierend(&app, move |s| {
        s.bild_setzen_wenn_aktiv(&id, &png).map_err(fehler)
    })
    .await?;
    melden(&app);
    Ok(())
}

/// `id` muss das aktive Profil sein.
#[tauri::command]
#[specta::specta]
pub async fn konto_bild_entfernen(
    fenster: WebviewWindow,
    app: AppHandle,
    id: String,
) -> Result<(), String> {
    nur_hauptfenster(&fenster)?;
    blockierend(&app, move |s| {
        s.bild_entfernen_wenn_aktiv(&id).map_err(fehler)
    })
    .await?;
    melden(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn konto_bild_lesen(
    fenster: WebviewWindow,
    app: AppHandle,
    id: String,
) -> Result<Option<String>, String> {
    nur_hauptfenster(&fenster)?;
    let bild = blockierend(&app, move |s| s.bild_lesen(&id).map_err(fehler)).await?;
    Ok(bild.map(|b| B64.encode(b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn slot_versucht_oeffnen_erneut_nach_fehler() {
        let mut slot: Slot<u8, u8> = Slot::default();
        let n = Cell::new(0);
        assert!(slot
            .sicherstellen(
                || {
                    n.set(n.get() + 1);
                    Err("x".into())
                },
                |_| Ok(1)
            )
            .is_err());
        assert!(slot.speicher.is_none());
        assert_eq!(
            slot.sicherstellen(
                || {
                    n.set(n.get() + 1);
                    Ok(7)
                },
                |_| Ok(1)
            ),
            Ok(7)
        );
        assert_eq!(n.get(), 2);
        // Idempotent: kein erneutes Oeffnen, kein erneutes Beobachten.
        assert_eq!(
            slot.sicherstellen(|| panic!("nicht erneut"), |_| panic!("nicht erneut")),
            Ok(7)
        );
    }

    #[test]
    fn beobachtungsfehler_blockiert_nicht_und_wird_nachgeholt() {
        let mut slot: Slot<u8, u8> = Slot::default();
        assert_eq!(slot.sicherstellen(|| Ok(7), |_| Err("b".into())), Ok(7));
        assert!(slot.beobachter.is_none());
        assert_eq!(
            slot.sicherstellen(|| panic!("nicht erneut"), |_| Ok(3)),
            Ok(7)
        );
        assert_eq!(slot.beobachter, Some(3));
    }

    #[test]
    fn nur_hauptfenster() {
        assert!(fenster_erlaubt("main").is_ok());
        assert!(fenster_erlaubt("recording_overlay").is_err());
        assert!(fenster_erlaubt("meeting_prompt").is_err());
        assert!(fenster_erlaubt("").is_err());
    }

    #[test]
    fn nicht_aktiv_wird_verstaendlich_gemeldet() {
        assert_eq!(fehler(KontoFehler::NichtAktiv("p1".into())), NICHT_AKTIV);
    }

    #[test]
    fn bild_dekodieren_grenzen() {
        assert_eq!(
            bild_dekodieren(&B64.encode([1u8, 2, 3])).unwrap(),
            vec![1, 2, 3]
        );
        assert!(bild_dekodieren("!!!").is_err());
        assert!(bild_dekodieren(&"A".repeat(BILD_BASE64_MAX + 1))
            .unwrap_err()
            .contains("256 KB"));
    }
}
