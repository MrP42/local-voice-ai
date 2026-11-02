//! Tauri-Befehle der Suite-Projekte und der Recherche (ADR-0026 PR-7).
//!
//! Mandant und Person kommen aus der lokalen Sitzung (`lokaler_principal`), nie aus den Argumenten.
//! Fehlertexte enthalten weder Frage noch Anfragen noch Quellen.

use super::dienst::{EintragSicht, LoeschErgebnis, SuiteProjekt, SyncBericht, klasse_aus_text};
use super::recherche::{
    FreigabeAktion, FreigabeInfo, LaufAntwort, RechercheKurz, RechercheSicht, RechercheStatus,
};
use super::zustand::{ProjekteZustand, modell_bereit};
use tauri::{AppHandle, State};
use wai_recherche::Datenklasse;

type Antwort<T> = Result<T, String>;

const MODELL: &str = "app";

#[tauri::command]
#[specta::specta]
pub async fn projekte_liste(z: State<'_, ProjekteZustand>) -> Antwort<Vec<SuiteProjekt>> {
    Ok(z.dienste().await?.projekte.projekte().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_anlegen(
    z: State<'_, ProjekteZustand>,
    titel: String,
    data_class: String,
    special_category: bool,
) -> Antwort<SuiteProjekt> {
    let klasse = klasse_aus_text(&data_class).ok_or("Unbekannte Datenklasse.")?;
    Ok(z.dienste()
        .await?
        .projekte
        .anlegen(&titel, klasse, special_category)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_web_freigabe(
    z: State<'_, ProjekteZustand>,
    id: String,
    an: bool,
) -> Antwort<SuiteProjekt> {
    Ok(z.dienste().await?.projekte.web_freigabe(&id, an).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_loeschen(
    z: State<'_, ProjekteZustand>,
    id: String,
) -> Antwort<LoeschErgebnis> {
    Ok(z.dienste().await?.projekte.loeschen(&id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_eintraege(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
) -> Antwort<Vec<EintragSicht>> {
    Ok(z.dienste().await?.projekte.eintraege(&suite_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_ordner_verknuepfen(
    z: State<'_, ProjekteZustand>,
    ordner_id: String,
    suite_id: String,
) -> Antwort<SyncBericht> {
    Ok(z.dienste()
        .await?
        .projekte
        .ordner_verknuepfen(&ordner_id, &suite_id)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_ordner_loesen(
    z: State<'_, ProjekteZustand>,
    ordner_id: String,
) -> Antwort<u32> {
    Ok(z.dienste().await?.projekte.ordner_loesen(&ordner_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_abgleichen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
) -> Antwort<SyncBericht> {
    Ok(z.dienste()
        .await?
        .projekte
        .projekt_abgleichen(&suite_id)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_buch_zuordnen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    buch_id: String,
) -> Antwort<()> {
    Ok(z.dienste()
        .await?
        .projekte
        .buch_zuordnen(&suite_id, &buch_id)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_buch_entfernen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    buch_id: String,
) -> Antwort<bool> {
    Ok(z.dienste()
        .await?
        .projekte
        .buch_entfernen(&suite_id, &buch_id)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_besprechung_zuordnen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    meeting_id: String,
) -> Antwort<()> {
    Ok(z.dienste()
        .await?
        .projekte
        .besprechung_zuordnen(&suite_id, &meeting_id)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_besprechung_entfernen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    meeting_id: String,
) -> Antwort<bool> {
    Ok(z.dienste()
        .await?
        .projekte
        .besprechung_entfernen(&suite_id, &meeting_id)
        .await?)
}

/// Suite-Projekte, die fuer eine Besprechung in Frage kommen (Projekte ihrer Ordner).
#[tauri::command]
#[specta::specta]
pub async fn projekte_der_besprechung(
    z: State<'_, ProjekteZustand>,
    meeting_id: String,
) -> Antwort<Vec<SuiteProjekt>> {
    let d = z.dienste().await?;
    let ids = d.projekte.projekte_der_besprechung(&meeting_id).await?;
    let alle = d.projekte.projekte().await?;
    Ok(ids
        .iter()
        .filter_map(|id| alle.iter().find(|p| &p.id == id).cloned())
        .collect())
}

// ---------------------------------------------------------------- Recherche

#[tauri::command]
#[specta::specta]
pub async fn recherche_status(z: State<'_, ProjekteZustand>) -> Antwort<RechercheStatus> {
    let d = z.dienste().await?;
    Ok(d.recherche.status(d.recherche.principal()))
}

/// Setzt oder entfernt die Adresse des Suchdienstes (nur Administration, streng geprueft).
#[tauri::command]
#[specta::specta]
pub async fn recherche_adresse_setzen(
    z: State<'_, ProjekteZustand>,
    adresse: Option<String>,
) -> Antwort<RechercheStatus> {
    let d = z.dienste().await?;
    let p = d.recherche.principal().clone();
    Ok(d.recherche.konfigurieren(&p, adresse.as_deref()).await?)
}

/// Chat-Werkzeug "recherche" (Modus Schnell). Der Kontext einer Besprechung ist mindestens
/// `PERSONAL`; die Klasse des Projekts hebt sie an. Ab `PERSONAL` haelt der Lauf an
/// (`status` = "freigabe") und wartet auf die Entscheidung in `recherche_freigabe_entscheiden`.
#[tauri::command]
#[specta::specta]
pub async fn recherche_starten(
    app: AppHandle,
    z: State<'_, ProjekteZustand>,
    frage: String,
    suite_projekt: Option<String>,
    besprechung: Option<String>,
) -> Antwort<LaufAntwort> {
    let d = z.dienste().await?;
    if let Err(e) = modell_bereit(&app) {
        return Ok(LaufAntwort {
            status: "fehler".into(),
            recherche_id: None,
            message: Some(e.message),
            code: Some("kein_modell".into()),
            halt_bleibt: false,
        });
    }
    Ok(d.recherche
        .starten_fuer(
            besprechung.as_deref(),
            &frage,
            MODELL,
            Datenklasse::Personal,
            suite_projekt,
        )
        .await)
}

#[tauri::command]
#[specta::specta]
pub async fn recherche_freigaben(z: State<'_, ProjekteZustand>) -> Antwort<Vec<FreigabeInfo>> {
    Ok(z.dienste().await?.recherche.freigaben())
}

#[tauri::command]
#[specta::specta]
pub async fn recherche_freigabe_entscheiden(
    z: State<'_, ProjekteZustand>,
    id: String,
    auswahl: Vec<String>,
    aktion: FreigabeAktion,
) -> Antwort<LaufAntwort> {
    Ok(z.dienste()
        .await?
        .recherche
        .freigabe_entscheiden(&id, &auswahl, aktion)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn recherche_liste(
    z: State<'_, ProjekteZustand>,
    suite_projekt: Option<String>,
) -> Antwort<Vec<RechercheKurz>> {
    Ok(z.dienste()
        .await?
        .recherche
        .liste(suite_projekt.as_deref())
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn recherche_lesen(
    z: State<'_, ProjekteZustand>,
    id: String,
) -> Antwort<RechercheSicht> {
    Ok(z.dienste().await?.recherche.sicht(&id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn recherche_loeschen(z: State<'_, ProjekteZustand>, id: String) -> Antwort<()> {
    Ok(z.dienste().await?.recherche.loeschen(&id).await?)
}
