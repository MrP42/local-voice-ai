//! Tauri-Befehle der Dokumentenablage (D1). Die Ablage liegt unter `<Datenordner>/ablage/`, der
//! Datenordner kommt wie bei den Projekten aus `portable::app_data_dir`. Fehlertexte enthalten nie Namen,
//! Pfade oder Inhalte; an die Oberfläche geht `<code>: <text>`.

use super::freigabe::KiStufe;
use super::speicher::{
    Ablage, AblageArt, AblageEinstellungen, AblageFassung, AblageImportZeile, AblageKollisionswahl,
    AblageLoeschBericht, AblageSicht,
};
use std::path::PathBuf;
use tauri::AppHandle;

type Antwort<T> = Result<T, String>;

fn ablage(app: &AppHandle) -> Antwort<Ablage> {
    let dir = crate::portable::app_data_dir(app)
        .map_err(|e| e.to_string())?
        .join("ablage");
    Ok(Ablage::neu(dir))
}

/// Führt eine blockierende Ablageoperation außerhalb des Async-Kerns aus.
async fn blockierend<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce(Ablage) -> super::Res<T> + Send + 'static,
) -> Antwort<T> {
    let a = ablage(app)?;
    tauri::async_runtime::spawn_blocking(move || f(a))
        .await
        .map_err(|_| "ablage.speicher: Die Ablage ist gerade nicht verfügbar.".to_string())?
        .map_err(String::from)
}

/// Gesamtstand für die Seite „Ablage“ (führt zuvor Wiederanlauf und Papierkorb-Aufräumen aus).
#[tauri::command]
#[specta::specta]
pub async fn ablage_stand(app: AppHandle) -> Antwort<AblageSicht> {
    blockierend(&app, |a| a.ansicht()).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_ordner_anlegen(
    app: AppHandle,
    eltern: Option<String>,
    name: String,
) -> Antwort<String> {
    blockierend(&app, move |a| a.ordner_anlegen(eltern.as_deref(), &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_ordner_umbenennen(app: AppHandle, id: String, name: String) -> Antwort<()> {
    blockierend(&app, move |a| a.ordner_umbenennen(&id, &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_ordner_verschieben(
    app: AppHandle,
    id: String,
    eltern: Option<String>,
) -> Antwort<()> {
    blockierend(&app, move |a| a.ordner_verschieben(&id, eltern.as_deref())).await
}

/// Datenklasse, „Nur dieses Gerät“ und „KI-Entwürfe“ eines Ordners.
#[tauri::command]
#[specta::specta]
pub async fn ablage_ordner_einstellungen(
    app: AppHandle,
    id: String,
    einstellungen: AblageEinstellungen,
) -> Antwort<()> {
    blockierend(&app, move |a| a.ordner_einstellungen(&id, &einstellungen)).await
}

/// KI-Freigabe eines Ordners (gerätelokal); `null` = erbt wieder.
#[tauri::command]
#[specta::specta]
pub async fn ablage_ki_setzen(app: AppHandle, id: String, stufe: Option<KiStufe>) -> Antwort<()> {
    blockierend(&app, move |a| a.ki_setzen(&id, stufe)).await
}

/// „Freigaben neu festlegen“: nur bei unklarer (defekter) Freigabe; alle Ordner auf „aus“.
#[tauri::command]
#[specta::specta]
pub async fn ablage_freigaben_neu_festlegen(app: AppHandle) -> Antwort<()> {
    blockierend(&app, |a| a.freigaben_neu_festlegen()).await
}

/// Legt eine Datei oder einen Ordner (mit Struktur) in einem Ordner der Ablage ab. Bei Namenskollision
/// ohne `wahl` wird nichts geschrieben (Zeile `kollision`).
#[tauri::command]
#[specta::specta]
pub async fn ablage_importieren(
    app: AppHandle,
    ordner: String,
    pfad: String,
    wahl: Option<AblageKollisionswahl>,
) -> Antwort<Vec<AblageImportZeile>> {
    let quelle = PathBuf::from(pfad);
    blockierend(&app, move |a| a.importieren(&quelle, &ordner, wahl)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_in_papierkorb(app: AppHandle, art: AblageArt, id: String) -> Antwort<()> {
    blockierend(&app, move |a| a.in_papierkorb(art, &id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_wiederherstellen(app: AppHandle, art: AblageArt, id: String) -> Antwort<()> {
    blockierend(&app, move |a| a.wiederherstellen(art, &id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_endgueltig_loeschen(
    app: AppHandle,
    art: AblageArt,
    id: String,
) -> Antwort<AblageLoeschBericht> {
    blockierend(&app, move |a| a.endgueltig_loeschen(art, &id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_papierkorb_leeren(app: AppHandle) -> Antwort<AblageLoeschBericht> {
    blockierend(&app, |a| a.papierkorb_leeren()).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_fassungen(app: AppHandle, datei: String) -> Antwort<Vec<AblageFassung>> {
    blockierend(&app, move |a| a.fassungen(&datei)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_datei_umbenennen(app: AppHandle, id: String, name: String) -> Antwort<()> {
    blockierend(&app, move |a| a.datei_umbenennen(&id, &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_datei_verschieben(app: AppHandle, id: String, ordner: String) -> Antwort<()> {
    blockierend(&app, move |a| a.datei_verschieben(&id, &ordner)).await
}

/// „Bestand neu aufbauen“: nur bei fehlendem oder defektem Ablagejournal. Liefert die Zahl der Funde.
#[tauri::command]
#[specta::specta]
pub async fn ablage_bestand_neu_aufbauen(app: AppHandle) -> Antwort<u32> {
    blockierend(&app, |a| a.neu_aufbauen()).await
}

/// Nimmt gefundene Originale einer Datei in einem Ordner auf.
#[tauri::command]
#[specta::specta]
pub async fn ablage_gefundene_aufnehmen(
    app: AppHandle,
    datei: String,
    ordner: String,
) -> Antwort<()> {
    blockierend(&app, move |a| a.gefundene_aufnehmen(&datei, &ordner)).await
}
