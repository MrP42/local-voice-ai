//! Tauri-Befehle der Dokumentenablage (D1, D2). Die Ablage liegt unter `<Datenordner>/ablage/`, der
//! Datenordner kommt wie bei den Projekten aus `portable::app_data_dir`. Fehlertexte enthalten nie Namen,
//! Pfade oder Inhalte; an die Oberfläche geht `<code>: <text>`.
//!
//! D2: Nach Änderungen am Bestand wird der Index sofort abgeglichen (Papierkorb, Verschieben, Löschen
//! wirken ohne Verzögerung auf die Suche) und der Textarbeiter angestoßen. Er meldet jedes Ergebnis mit
//! dem Ereignis [`EREIGNIS_TEXT`]; die Seite lädt dann den Stand neu.

use super::freigabe::KiStufe;
use super::index::{AblageIndexBericht, AblageSuchtreffer};
use super::speicher::{
    Ablage, AblageArt, AblageEinstellungen, AblageFassung, AblageImportZeile, AblageKollisionswahl,
    AblageLoeschBericht, AblageSicht,
};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

/// Ereignis an die Oberfläche: eine Textfassung ist fertig (oder fehlgeschlagen).
pub const EREIGNIS_TEXT: &str = "ablage-text";

/// Ereignis an die Oberfläche: ein Abgleich der Ablage ist gelaufen (D4); Stand und Sync-Zustand neu laden.
pub const EREIGNIS_SYNC: &str = "ablage-sync";

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

/// Wie [`blockierend`], danach (auch nach einem Fehler) Index abgleichen und Textarbeiter wecken.
/// `ausdruecklich` (Import, Aufnehmen, Neuaufbau): hebt ein Anhalten nach einem Abbruch auf; sonst nur
/// Wiederanlauf (ein abgebrochener Lauf bleibt angehalten).
async fn mit_nachlauf<T: Send + 'static>(
    app: &AppHandle,
    ausdruecklich: bool,
    f: impl FnOnce(Ablage) -> super::Res<T> + Send + 'static,
) -> Antwort<T> {
    let a = ablage(app)?;
    let melder = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let r = f(a.clone());
        nachlauf(&a, melder, ausdruecklich);
        r
    })
    .await
    .map_err(|_| "ablage.speicher: Die Ablage ist gerade nicht verfügbar.".to_string())?
    .map_err(String::from)
}

fn nachlauf(a: &Ablage, app: AppHandle, ausdruecklich: bool) {
    if let Err(e) = super::index::abgleichen(a) {
        log::warn!("ablage: Indexabgleich: {}", e.code);
    }
    super::textfassung::melden_setzen(Box::new(move || {
        let _ = app.emit(EREIGNIS_TEXT, ());
    }));
    if ausdruecklich {
        super::textfassung::anstossen(a.dir().to_path_buf());
    } else {
        super::textfassung::wiederanlauf(a.dir().to_path_buf());
    }
}

/// Gesamtstand für die Seite „Ablage“ (führt zuvor Wiederanlauf und Papierkorb-Aufräumen aus).
#[tauri::command]
#[specta::specta]
pub async fn ablage_stand(app: AppHandle) -> Antwort<AblageSicht> {
    let melder = app.clone();
    let sicht = blockierend(&app, |a| a.ansicht()).await?;
    // Wiederanlauf der Textfassungen (offen ist, was im Stand fehlt, ohne Rückstellungen dieses
    // Prozesses); nicht nach einem Abbruch (angehalten bis zum nächsten ausdrücklichen Anstoß).
    if sicht.lage.is_none() && !super::textfassung::angehalten() {
        let a = ablage(&app)?;
        let offen = tauri::async_runtime::spawn_blocking(move || {
            super::textfassung::offen(&a).then(|| a.dir().to_path_buf())
        })
        .await
        .ok()
        .flatten();
        if let Some(dir) = offen {
            super::textfassung::melden_setzen(Box::new(move || {
                let _ = melder.emit(EREIGNIS_TEXT, ());
            }));
            super::textfassung::wiederanlauf(dir);
        }
    }
    Ok(sicht)
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
    mit_nachlauf(&app, false, move |a| {
        a.ordner_verschieben(&id, eltern.as_deref())
    })
    .await
}

/// Datenklasse, „Nur dieses Gerät“ und „KI-Entwürfe“ eines Ordners.
#[tauri::command]
#[specta::specta]
pub async fn ablage_ordner_einstellungen(
    app: AppHandle,
    id: String,
    einstellungen: AblageEinstellungen,
) -> Antwort<()> {
    let projekte = crate::portable::app_data_dir(&app)
        .map_err(|e| e.to_string())?
        .join("projekte");
    mit_nachlauf(&app, false, move |a| {
        a.ordner_einstellungen(&id, &einstellungen)?;
        // Wieder synchron (D4): Rückweg lokal → sync sofort; scheitert er, setzt der Sync-Takt fort.
        if let Err(e) = a.rueckweg(&super::speicher::ProjektVerweise::neu(projekte)) {
            log::warn!("ablage: Rückweg lokal → sync: {}", e.code);
        }
        Ok(())
    })
    .await
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
    mit_nachlauf(&app, true, move |a| a.importieren(&quelle, &ordner, wahl)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_in_papierkorb(app: AppHandle, art: AblageArt, id: String) -> Antwort<()> {
    mit_nachlauf(&app, false, move |a| a.in_papierkorb(art, &id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_wiederherstellen(app: AppHandle, art: AblageArt, id: String) -> Antwort<()> {
    mit_nachlauf(&app, false, move |a| a.wiederherstellen(art, &id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_endgueltig_loeschen(
    app: AppHandle,
    art: AblageArt,
    id: String,
) -> Antwort<AblageLoeschBericht> {
    mit_nachlauf(&app, false, move |a| a.endgueltig_loeschen(art, &id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_papierkorb_leeren(app: AppHandle) -> Antwort<AblageLoeschBericht> {
    mit_nachlauf(&app, false, |a| a.papierkorb_leeren()).await
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
    mit_nachlauf(&app, false, move |a| a.datei_verschieben(&id, &ordner)).await
}

/// „Bestand neu aufbauen“: nur bei fehlendem oder defektem Ablagejournal. Liefert die Zahl der Funde.
#[tauri::command]
#[specta::specta]
pub async fn ablage_bestand_neu_aufbauen(app: AppHandle) -> Antwort<u32> {
    mit_nachlauf(&app, true, |a| a.neu_aufbauen()).await
}

/// Nimmt gefundene Originale einer Datei in einem Ordner auf.
#[tauri::command]
#[specta::specta]
pub async fn ablage_gefundene_aufnehmen(
    app: AppHandle,
    datei: String,
    ordner: String,
) -> Antwort<()> {
    mit_nachlauf(&app, true, move |a| a.gefundene_aufnehmen(&datei, &ordner)).await
}

/// Volltextsuche in der Ablage (ohne KI): Fundstellen mit Auszug und Anker. Die Namenssuche läuft in der
/// Oberfläche; beide Ergebnisse zeigt die Seite zusammen.
#[tauri::command]
#[specta::specta]
pub async fn ablage_suchen(app: AppHandle, anfrage: String) -> Antwort<Vec<AblageSuchtreffer>> {
    blockierend(&app, move |a| super::index::suchen(&a, &anfrage)).await
}

/// „Suchindex neu aufbauen“: `index.db` löschen und aus den Textfassungen neu erzeugen.
#[tauri::command]
#[specta::specta]
pub async fn ablage_index_neu_aufbauen(app: AppHandle) -> Antwort<AblageIndexBericht> {
    blockierend(&app, |a| super::index::neu_aufbauen(&a)).await
}

/// Bricht die laufende Texterzeugung ab (bis zum nächsten Anstoß).
#[tauri::command]
#[specta::specta]
pub async fn ablage_text_abbrechen() -> Antwort<()> {
    super::textfassung::abbrechen();
    Ok(())
}

/// Sync-Zustand je Ordner und Datei, Kontingent und letzter Fehler (D4). Ohne Konto: `verbunden = false`.
#[tauri::command]
#[specta::specta]
pub async fn ablage_sync_stand(app: AppHandle) -> Antwort<crate::sync::ablage::AblageSyncStand> {
    let verbunden = crate::sync::account::load(&app).is_some();
    blockierend(&app, move |a| crate::sync::ablage::stand(&a, verbunden)).await
}
