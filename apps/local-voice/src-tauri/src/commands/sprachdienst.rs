//! Kommandos des Sprachdienstes (ADR-0021, Paket S2): ein- und ausschalten, gekoppelte Apps
//! verwalten, Kopplungsanfragen bestätigen.
//!
//! Jedes Kommando ist ein Schritt des Nutzers in der Oberfläche. Es gibt keinen Weg dorthin über
//! die Pipe oder den Socket des Dienstes: Wer koppeln will, kann den Code nicht selbst
//! bestätigen. Den Code einer offenen Anfrage kennt nur dieses Fenster.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, State, WebviewWindow};
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
    /// Kennung der App (`client` im Vertrag), neben dem frei gewählten Namen anzuzeigen.
    pub client: String,
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

/// Nur das Hauptfenster darf den Sprachdienst steuern. Die Fähigkeiten der Fenster
/// `recording_overlay` und `meeting_prompt` sind dieselben wie die des Hauptfensters; ohne diese
/// Prüfung könnte Code in einem davon Apps koppeln oder die Kopplung bestätigen.
fn nur_hauptfenster(fenster: &WebviewWindow) -> Result<(), String> {
    if fenster.label() == "main" {
        Ok(())
    } else {
        Err("Der Sprachdienst lässt sich nur im Hauptfenster steuern.".to_string())
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
    fenster: WebviewWindow,
    dienst: State<'_, Arc<Sprachdienst>>,
) -> Result<SprachdienstStatus, String> {
    nur_hauptfenster(&fenster)?;
    Ok(status_von(&dienst))
}

/// Schaltet den Dienst ein oder aus. Aus: Der Endpunkt verschwindet, laufende Verbindungen enden.
#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_aktiv_setzen(
    fenster: WebviewWindow,
    app: AppHandle,
    dienst: State<'_, Arc<Sprachdienst>>,
    aktiv: bool,
) -> Result<SprachdienstStatus, String> {
    nur_hauptfenster(&fenster)?;
    dienst
        .kontext()
        .zugaenge
        .aktiv_setzen(aktiv)
        .map_err(|e| e.to_string())?;
    if aktiv {
        // Ein Startfehler steht im Status und blockiert die App nicht.
        let _ = dienst.starten().await;
    } else {
        dienst.stoppen().await;
    }
    geaendert(&app);
    Ok(status_von(&dienst))
}

#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_zugaenge(
    fenster: WebviewWindow,
    dienst: State<'_, Arc<Sprachdienst>>,
) -> Result<Vec<SprachZugang>, String> {
    nur_hauptfenster(&fenster)?;
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
    fenster: WebviewWindow,
    app: AppHandle,
    dienst: State<'_, Arc<Sprachdienst>>,
    zugang: String,
    recht: String,
    an: bool,
) -> Result<SprachZugang, String> {
    nur_hauptfenster(&fenster)?;
    let r = recht_aus(&recht)?;
    let z = dienst
        .kontext()
        .zugaenge
        .recht_setzen(&zugang, r, an)
        .map_err(|e| e.to_string())?;
    if !an {
        // Nur die Ströme dieses Rechts enden (mit `recht_fehlt`), nicht die Verbindung.
        dienst.kontext().recht_entzogen(&zugang, r);
    }
    geaendert(&app);
    Ok(z.into())
}

/// Widerruft eine App: Rechte weg, laufende Verbindungen enden sofort, die Zeile bleibt als Spur.
#[tauri::command]
#[specta::specta]
pub async fn sprachdienst_widerrufen(
    fenster: WebviewWindow,
    app: AppHandle,
    dienst: State<'_, Arc<Sprachdienst>>,
    zugang: String,
) -> Result<SprachZugang, String> {
    nur_hauptfenster(&fenster)?;
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
    fenster: WebviewWindow,
    app: AppHandle,
    dienst: State<'_, Arc<Sprachdienst>>,
    zugang: String,
) -> Result<(), String> {
    nur_hauptfenster(&fenster)?;
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
    fenster: WebviewWindow,
    dienst: State<'_, Arc<Sprachdienst>>,
) -> Result<Vec<SprachKopplung>, String> {
    nur_hauptfenster(&fenster)?;
    Ok(dienst
        .kontext()
        .kopplungen
        .offen(jetzt_ms())
        .into_iter()
        .map(|k| SprachKopplung {
            id: k.id,
            client: k.client,
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
    fenster: WebviewWindow,
    dienst: State<'_, Arc<Sprachdienst>>,
    id: String,
    bestaetigen: bool,
) -> Result<bool, String> {
    nur_hauptfenster(&fenster)?;
    Ok(dienst
        .kontext()
        .kopplungen
        .entscheiden(&id, bestaetigen, jetzt_ms()))
}
