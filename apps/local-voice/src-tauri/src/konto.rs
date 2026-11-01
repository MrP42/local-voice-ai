//! Suite-Profil (ADR-0024, Paket 1): Tauri-Befehle über dem gemeinsamen Kontospeicher
//! `wai-konto` (vendort unter `vendor/wai-konto`). Nur lokales Profil, keine Anmeldung.
//!
//! Fehlertexte stammen aus `KontoFehler` und enthalten nie Profilinhalte; hier wird
//! nichts protokolliert, was zu einem Profil gehört.

use std::sync::Mutex;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager, State};
use wai_konto::{
    Beobachter, KontoFehler, KontoSpeicher, Profil, ProfilAenderung, ProfilDatei, ProfilEingabe,
};

/// Ereignis an das Frontend, sobald sich der Kontospeicher geändert hat (auch durch
/// andere Apps der Suite).
pub const EVENT_GEAENDERT: &str = "konto-geaendert";

/// Verwalteter Zustand. Ohne Speicher (Datenordner nicht ermittelbar) bleibt die App
/// benutzbar; die Befehle melden dann einen Fehler.
pub struct KontoState {
    speicher: Option<KontoSpeicher>,
    _beobachter: Mutex<Option<Beobachter>>,
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

fn fehler(e: KontoFehler) -> String {
    e.to_string()
}

impl KontoState {
    fn speicher(&self) -> Result<&KontoSpeicher, String> {
        self.speicher
            .as_ref()
            .ok_or_else(|| "Der Kontospeicher ist nicht verfügbar.".to_string())
    }
}

/// Öffnet den Kontospeicher, hält ihn als verwalteten Zustand und meldet Änderungen
/// (auch anderer Apps) als Ereignis `konto-geaendert`.
pub fn init(app: &AppHandle) {
    let speicher = match KontoSpeicher::oeffnen_standard() {
        Ok(s) => Some(s),
        Err(e) => {
            log::error!("Kontospeicher nicht verfügbar: {e}");
            None
        }
    };
    let beobachter = speicher.as_ref().and_then(|s| {
        let handle = app.clone();
        match s.beobachten(move |_| {
            let _ = handle.emit(EVENT_GEAENDERT, ());
        }) {
            Ok(b) => Some(b),
            Err(e) => {
                log::warn!("Kontobeobachtung nicht gestartet: {e}");
                None
            }
        }
    });
    app.manage(KontoState {
        speicher,
        _beobachter: Mutex::new(beobachter),
    });
}

fn melden(app: &AppHandle) {
    let _ = app.emit(EVENT_GEAENDERT, ());
}

#[tauri::command]
#[specta::specta]
pub fn konto_status(state: State<'_, KontoState>) -> Result<KontoStatus, String> {
    status_aus(state.speicher()?.lesen().map_err(fehler)?)
}

#[tauri::command]
#[specta::specta]
pub fn konto_aendern(
    app: AppHandle,
    state: State<'_, KontoState>,
    id: String,
    aenderung: KontoAenderung,
) -> Result<KontoProfilDto, String> {
    let profil = state
        .speicher()?
        .profil_aendern(&id, aenderung.into())
        .map_err(fehler)?;
    melden(&app);
    Ok(profil.into())
}

#[tauri::command]
#[specta::specta]
pub fn konto_aktivieren(
    app: AppHandle,
    state: State<'_, KontoState>,
    id: String,
) -> Result<KontoStatus, String> {
    let speicher = state.speicher()?;
    speicher.aktivieren(&id).map_err(fehler)?;
    melden(&app);
    status_aus(speicher.lesen().map_err(fehler)?)
}

#[tauri::command]
#[specta::specta]
pub fn konto_anlegen(
    app: AppHandle,
    state: State<'_, KontoState>,
    anzeigename: String,
) -> Result<KontoProfilDto, String> {
    let profil = state
        .speicher()?
        .profil_anlegen(ProfilEingabe {
            anzeigename,
            ..ProfilEingabe::default()
        })
        .map_err(fehler)?;
    melden(&app);
    Ok(profil.into())
}

/// Bild als Base64 (PNG, höchstens 256 KB; die Prüfung macht `wai-konto`).
#[tauri::command]
#[specta::specta]
pub fn konto_bild_setzen(
    app: AppHandle,
    state: State<'_, KontoState>,
    id: String,
    bytes: String,
) -> Result<(), String> {
    let png = B64
        .decode(bytes.trim())
        .map_err(|_| "Das Profilbild ist nicht lesbar.".to_string())?;
    state.speicher()?.bild_setzen(&id, &png).map_err(fehler)?;
    melden(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn konto_bild_entfernen(
    app: AppHandle,
    state: State<'_, KontoState>,
    id: String,
) -> Result<(), String> {
    state.speicher()?.bild_entfernen(&id).map_err(fehler)?;
    melden(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn konto_bild_lesen(
    state: State<'_, KontoState>,
    id: String,
) -> Result<Option<String>, String> {
    let bild = state.speicher()?.bild_lesen(&id).map_err(fehler)?;
    Ok(bild.map(|b| B64.encode(b)))
}
