//! Tauri-Befehle der Suite-Projekte und der Recherche (ADR-0026 PR-7).
//!
//! Mandant und Person kommen aus der lokalen Sitzung (`lokaler_principal`), nie aus den Argumenten.
//! Fehlertexte enthalten weder Frage noch Anfragen noch Quellen.

use super::dateien::JournalLage;
use super::dienst::{
    klasse_aus_text, EintragSicht, GefundeneDatei, LoeschErgebnis, SuiteProjekt, SyncBericht,
};
use super::dokument::{self, DokumentAuftrag, DokumentErgebnis, WegVersand};
use super::kontext::{KontextForm, KontextUebersicht};
use super::recherche::{
    FreigabeAktion, FreigabeInfo, LaufAntwort, RechercheDienst, RechercheKurz, RechercheSicht,
    RechercheStatus,
};
use super::zustand::{modell_bereit, AppEntwurf, ProjekteZustand};
use crate::voice_call::projekt::LlmWeg;
use std::sync::Arc;
use tauri::{AppHandle, State};
use wai_recherche::Datenklasse;

type Antwort<T> = Result<T, String>;

const MODELL: &str = "app";

/// Stand der Sperre der KI im Projekt (lva#365 P4c). Die Oberfläche richtet „Gespräch im Projekt
/// beginnen“ danach aus. Es ist die feste Sperre aus `dienst.rs`, kein Laufzeitschalter.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjektKiStatus {
    pub freigegeben: bool,
}

#[tauri::command]
#[specta::specta]
pub fn projekt_ki_status() -> ProjektKiStatus {
    ProjektKiStatus {
        freigegeben: super::dienst::ki_freigegeben(),
    }
}

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
    Ok(z.dienste()
        .await?
        .projekte
        .ordner_loesen(&ordner_id)
        .await?)
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
pub async fn projekt_seite_zuordnen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    seite_id: String,
) -> Antwort<()> {
    Ok(z.dienste()
        .await?
        .projekte
        .seite_zuordnen(&suite_id, &seite_id)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_seite_entfernen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    seite_id: String,
) -> Antwort<bool> {
    Ok(z.dienste()
        .await?
        .projekte
        .seite_entfernen(&suite_id, &seite_id)
        .await?)
}

/// Kopiert die gewählten Dateien in den Projektordner (Prüfung per Größe und SHA-256) und trägt
/// sie danach ein. Liefert die Datei-IDs.
#[tauri::command]
#[specta::specta]
pub async fn projekt_dateien_hinzufuegen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    paths: Vec<String>,
) -> Antwort<Vec<String>> {
    let quellen: Vec<std::path::PathBuf> = paths.into_iter().map(Into::into).collect();
    Ok(z.dienste()
        .await?
        .projekte
        .dateien_hinzufuegen(&suite_id, &quellen)
        .await?)
}

/// Legt die Kopie in den Papierkorb des Projekts und nimmt sie aus dem Projekt. `false`: konnte nicht
/// sicher entfernt werden; die Datei liegt weiter im Projektordner und wird zum Aufnehmen angeboten.
#[tauri::command]
#[specta::specta]
pub async fn projekt_datei_entfernen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    datei_id: String,
) -> Antwort<bool> {
    Ok(z.dienste()
        .await?
        .projekte
        .datei_entfernen(&suite_id, &datei_id)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_dateien_gefunden(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
) -> Antwort<Vec<GefundeneDatei>> {
    Ok(z.dienste()
        .await?
        .projekte
        .gefundene_dateien(&suite_id)
        .await?)
}

/// Zustand des Dateijournals: `null` = lesbar, sonst `fehlt`/`defekt`/`neuer`/`nicht_lesbar`.
#[tauri::command]
#[specta::specta]
pub async fn projekt_dateien_lage(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
) -> Antwort<Option<JournalLage>> {
    Ok(z.dienste().await?.projekte.dateien_lage(&suite_id).await?)
}

/// „Bestand neu aufbauen“: nur bei fehlendem oder defektem Dateijournal.
#[tauri::command]
#[specta::specta]
pub async fn projekt_dateien_neu_aufbauen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
) -> Antwort<u32> {
    Ok(z.dienste()
        .await?
        .projekte
        .dateien_neu_aufbauen(&suite_id)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn projekt_datei_gefunden_aufnehmen(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    datei_id: String,
) -> Antwort<()> {
    Ok(z.dienste()
        .await?
        .projekte
        .gefundene_datei_aufnehmen(&suite_id, &datei_id)
        .await?)
}

/// Übersicht „Als Kontext“ (ohne Inhalte): Schalter, Form, Token je Eintrag, Summe, Stand.
#[tauri::command]
#[specta::specta]
pub async fn projekt_kontext_uebersicht(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
) -> Antwort<KontextUebersicht> {
    Ok(z.dienste()
        .await?
        .projekte
        .kontext_uebersicht(&suite_id)
        .await?)
}

/// Schaltet „Als Kontext“ eines Eintrags. `form` ändert die Form, `edition` die Fassung eines
/// Buchs (leer = Hauptfassung); fehlt eines, bleibt es, wie es ist. `expected_stand`: der `stand` der
/// angezeigten Übersicht; hat sich die Auswahl seitdem geändert, kommt `projekte.stand_veraltet` und es
/// ändert sich nichts (P4c-1).
#[tauri::command]
#[specta::specta]
pub async fn projekt_kontext_schalten(
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    eintrag_id: String,
    on: bool,
    form: Option<KontextForm>,
    edition: Option<String>,
    expected_stand: Option<String>,
) -> Antwort<KontextUebersicht> {
    let fassung = edition.map(|e| (!e.is_empty()).then_some(e));
    Ok(z.dienste()
        .await?
        .projekte
        .kontext_schalten_mit_stand(
            &suite_id,
            &eintrag_id,
            on,
            form,
            fassung,
            expected_stand.as_deref(),
        )
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
///
/// `mit_kontext` (lva#365 P4c-3): Die Recherche nimmt den Kontext des Projekts hinzu. Den liest das
/// Versandtor frisch, ebenso Klassen und Modell; die Oberflaeche bestimmt davon nichts. Der Lauf haelt
/// dann immer vor dem ersten Webzugriff an. Bis zur Freischaltung (P4c-4) antwortet das Tor mit
/// `projekt.ki_gesperrt`.
#[tauri::command]
#[specta::specta]
pub async fn recherche_starten(
    app: AppHandle,
    z: State<'_, ProjekteZustand>,
    frage: String,
    suite_projekt: Option<String>,
    besprechung: Option<String>,
    mit_kontext: Option<bool>,
) -> Antwort<LaufAntwort> {
    let d = z.dienste().await?;
    let mit_kontext = mit_kontext.unwrap_or(false);
    // Mit Kontext bestimmt das Tor das Modell; die Einstellung der Besprechungs-Recherche gilt dann nicht.
    if !mit_kontext {
        if let Err(e) = modell_bereit(&app) {
            return Ok(LaufAntwort {
                status: "fehler".into(),
                recherche_id: None,
                message: Some(e.message),
                code: Some("kein_modell".into()),
                halt_bleibt: false,
            });
        }
    }
    Ok(recherche_mit(
        &d.recherche,
        &frage,
        suite_projekt,
        besprechung.as_deref(),
        mit_kontext,
    )
    .await)
}

/// Klasse der Frage beim Aufruf aus der Oberflaeche. Eine Frage aus einer Besprechung ist mindestens
/// `PERSONAL` (ihr Chat kennt das Transkript); ohne Besprechung traegt sie ohne Projektkontext
/// ebenfalls `PERSONAL` (unveraendert). Mit Projektkontext und ohne Besprechung traegt die Frage selbst
/// keine Klasse: Dann gelten die Klassen des Projekts und seiner Inhalte, und das Tor bestimmt das Ziel
/// danach. Ein Internal-Projekt laeuft so mit einem Cloud-Ziel; ein hoeher eingestuftes scheitert vor
/// dem Halt mit `projekt.klasse_ziel`, nicht erst im Modell.
pub(super) fn frage_klasse(besprechung: Option<&str>, mit_kontext: bool) -> Datenklasse {
    if mit_kontext && besprechung.is_none() {
        Datenklasse::Public
    } else {
        Datenklasse::Personal
    }
}

/// Der Weg des Befehls `recherche_starten` ab der Pruefung des Modells: Klasse der Frage bestimmen und
/// die Recherche starten (mit oder ohne Projektkontext).
pub(super) async fn recherche_mit(
    recherche: &RechercheDienst,
    frage: &str,
    suite_projekt: Option<String>,
    besprechung: Option<&str>,
    mit_kontext: bool,
) -> LaufAntwort {
    recherche
        .starten_fuer_mit(
            besprechung,
            frage,
            MODELL,
            frage_klasse(besprechung, mit_kontext),
            suite_projekt,
            mit_kontext,
        )
        .await
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
pub async fn recherche_lesen(z: State<'_, ProjekteZustand>, id: String) -> Antwort<RechercheSicht> {
    Ok(z.dienste().await?.recherche.sicht(&id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn recherche_loeschen(z: State<'_, ProjekteZustand>, id: String) -> Antwort<()> {
    Ok(z.dienste().await?.recherche.loeschen(&id).await?)
}

// ---------------------------------------------------------------- KI-Dokument (lva#365 P4c-3)

/// Die KI schreibt aus dem Kontext des Projekts ein Dokument und legt es als Entwurf in der Ablage ab
/// (Ablauf und Regeln: `dokument.rs`). Kontext, Klassen und Modell bestimmt das Versandtor; der Befehl
/// nimmt bewusst **keine** Mindestklasse und keine Quellen entgegen, die Oberflaeche kann die Klasse des
/// Entwurfs also nicht senken. Der Entwurf ist im Projekt ausgeschaltet. Bis zur Freischaltung (P4c-4)
/// antwortet das Tor mit `projekt.ki_gesperrt`.
#[tauri::command]
#[specta::specta]
pub async fn projekt_ki_dokument_erstellen(
    app: AppHandle,
    z: State<'_, ProjekteZustand>,
    suite_id: String,
    auftrag: String,
    ordner: Option<String>,
    name: String,
    expected_stand: Option<String>,
) -> Antwort<DokumentErgebnis> {
    let d = z.dienste().await?;
    Ok(dokument::erstellen(
        &d.tor,
        &WegVersand(Arc::new(LlmWeg)),
        &d.projekte,
        &AppEntwurf(app),
        DokumentAuftrag {
            projekt: suite_id,
            auftrag,
            ordner,
            name,
            erwarteter_stand: expected_stand,
        },
    )
    .await?)
}
