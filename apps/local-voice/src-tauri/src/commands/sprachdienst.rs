//! Kommandos des Sprachdienstes (ADR-0021, Paket S2): ein- und ausschalten, gekoppelte Apps
//! verwalten, Kopplungsanfragen bestätigen.
//!
//! Jedes Kommando ist ein Schritt des Nutzers in der Oberfläche. Es gibt keinen Weg dorthin über
//! die Pipe oder den Socket des Dienstes: Wer koppeln will, kann den Code nicht selbst
//! bestätigen. Den Code einer offenen Anfrage kennt nur dieses Fenster.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, State};
use wai_voice_dienst::msg::Recht;

use crate::sprachdienst::app::EREIGNIS_ZUGAENGE;
use crate::sprachdienst::dienst::Sprachdienst;
use crate::sprachdienst::zugang::Zugang;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SprachdienstStatus {
    /// Vom Nutzer eingeschaltet (Opt-in, Standard aus).
    pub aktiv: bool,
    pub laeuft: bool,
    /// Name der Pipe bzw. Pfad des Sockets.
    pub endpunkt: Option<String>,
    pub fehler: Option<String>,
}

/// Eine gekoppelte App, ohne Fingerabdruck.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SprachZugang {
    pub zugang: String,
    pub client: String,
    pub anzeige: String,
    pub hoeren: bool,
    pub sprechen: bool,
    pub erstellt_ms: i64,
    pub zuletzt_ms: Option<i64>,
    pub widerrufen_ms: Option<i64>,
}

/// Eine Kopplungsanfrage, die auf die Bestätigung wartet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SprachKopplung {
    pub id: String,
    pub anzeige: String,
    /// Sechsstelliger Code, den die anfragende App zeigt; nur ein Vergleich für den Nutzer.
    pub code: String,
    /// Ablauf der Frist (ms seit 1970, UTC).
    pub bis_ms: i64,
}

impl From<Zugang> for SprachZugang {
    fn from(z: Zugang) -> Self {
        Self {
            hoeren: z.hat_recht(Recht::Hoeren),
            sprechen: z.hat_recht(Recht::Sprechen),
            zugang: z.zugang,
            client: z.client,
            anzeige: z.anzeige,
            erstellt_ms: z.erstellt_ms,
            zuletzt_ms: z.zuletzt_ms,
            widerrufen_ms: z.widerrufen_ms,
        }
    }
}

fn jetzt_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn status_von(dienst: &Sprachdienst) -> SprachdienstStatus {
    let s = dienst.status();
    SprachdienstStatus {
        aktiv: dienst.kontext().zugaenge.aktiv(),
        laeuft: s.laeuft,
        endpunkt: s.endpunkt,
        fehler: s.fehler,
    }
}

fn geaendert(app: &AppHandle) {
    let _ = app.emit(EREIGNIS_ZUGAENGE, ());
}

fn recht_aus(name: &str) -> Result<Recht, String> {
    match name {
        "hoeren" => Ok(Recht::Hoeren),
        "sprechen" => Ok(Recht::Sprechen),
        _ => Err("Dieses Recht lässt sich nicht vergeben.".to_string()),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_status(
    dienst: State<'_, Arc<Sprachdienst>>,
) -> Result<SprachdienstStatus, String> {
    Ok(status_von(&dienst))
}

/// Schaltet den Dienst ein oder aus. Aus: Der Endpunkt verschwindet, laufende Verbindungen enden.
#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_aktiv_setzen(
    app: AppHandle,
    dienst: State<'_, Arc<Sprachdienst>>,
    aktiv: bool,
) -> Result<SprachdienstStatus, String> {
    dienst
        .kontext()
        .zugaenge
        .aktiv_setzen(aktiv)
        .map_err(|e| e.to_string())?;
    if aktiv {
        // Ein Startfehler steht im Status und blockiert die App nicht.
        let _ = dienst.starten().await;
    } else {
        dienst.stoppen();
    }
    geaendert(&app);
    Ok(status_von(&dienst))
}

#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_zugaenge(
    dienst: State<'_, Arc<Sprachdienst>>,
) -> Result<Vec<SprachZugang>, String> {
    Ok(dienst
        .kontext()
        .zugaenge
        .liste()
        .into_iter()
        .map(SprachZugang::from)
        .collect())
}

/// Setzt `hoeren` oder `sprechen` für eine gekoppelte App. Ein Entzug wirkt sofort auch in
/// laufenden Verbindungen.
#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_recht_setzen(
    app: AppHandle,
    dienst: State<'_, Arc<Sprachdienst>>,
    zugang: String,
    recht: String,
    an: bool,
) -> Result<SprachZugang, String> {
    let r = recht_aus(&recht)?;
    let z = dienst
        .kontext()
        .zugaenge
        .recht_setzen(&zugang, r, an)
        .map_err(|e| e.to_string())?;
    if !an {
        dienst.kontext().verbindungen_beenden(&zugang);
    }
    geaendert(&app);
    Ok(z.into())
}

/// Widerruft eine App: Rechte weg, laufende Verbindungen enden sofort, die Zeile bleibt als Spur.
#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_widerrufen(
    app: AppHandle,
    dienst: State<'_, Arc<Sprachdienst>>,
    zugang: String,
) -> Result<SprachZugang, String> {
    let z = dienst
        .kontext()
        .zugaenge
        .widerrufen(&zugang, jetzt_ms())
        .map_err(|e| e.to_string())?;
    dienst.kontext().verbindungen_beenden(&zugang);
    geaendert(&app);
    Ok(z.into())
}

/// Entfernt eine App ganz aus der Liste.
#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_entfernen(
    app: AppHandle,
    dienst: State<'_, Arc<Sprachdienst>>,
    zugang: String,
) -> Result<(), String> {
    dienst
        .kontext()
        .zugaenge
        .entfernen(&zugang)
        .map_err(|e| e.to_string())?;
    dienst.kontext().verbindungen_beenden(&zugang);
    geaendert(&app);
    Ok(())
}

/// Offene Kopplungsanfragen (mit Code), neueste zuletzt.
#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_kopplungen(
    dienst: State<'_, Arc<Sprachdienst>>,
) -> Result<Vec<SprachKopplung>, String> {
    Ok(dienst
        .kontext()
        .kopplungen
        .offen(jetzt_ms())
        .into_iter()
        .map(|k| SprachKopplung {
            id: k.id,
            anzeige: k.anzeige,
            code: k.code,
            bis_ms: k.bis_ms,
        })
        .collect())
}

/// Bestätigt oder lehnt eine Kopplungsanfrage ab. `false` in der Antwort: die Anfrage gab es
/// nicht mehr (Frist abgelaufen oder die App hat aufgelegt).
#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_kopplung_entscheiden(
    dienst: State<'_, Arc<Sprachdienst>>,
    id: String,
    bestaetigen: bool,
) -> Result<bool, String> {
    Ok(dienst
        .kontext()
        .kopplungen
        .entscheiden(&id, bestaetigen, jetzt_ms()))
}
