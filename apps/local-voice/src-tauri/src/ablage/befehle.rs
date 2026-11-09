//! Tauri-Befehle der Dokumentenablage (D1, D2). Die Ablage liegt unter `<Datenordner>/ablage/`, der
//! Datenordner kommt wie bei den Projekten aus `portable::app_data_dir`. Fehlertexte enthalten nie Namen,
//! Pfade oder Inhalte; an die Oberfläche geht `<code>: <text>`.
//!
//! D2: Nach Änderungen am Bestand wird der Index sofort abgeglichen (Papierkorb, Verschieben, Löschen
//! wirken ohne Verzögerung auf die Suche) und der Textarbeiter angestoßen. Er meldet jedes Ergebnis mit
//! dem Ereignis [`EREIGNIS_TEXT`]; die Seite lädt dann den Stand neu.
//!
//! D6: Schalter und Modell der semantischen Suche ([`ablage_semantik_stand`], [`ablage_semantik_setzen`],
//! [`ablage_semantik_laden`]); das Ende des Nachziehens der Vektoren meldet [`EREIGNIS_SEMANTIK`].

use super::freigabe::KiStufe;
use super::index::{AblageIndexBericht, AblageSuchtreffer};
use super::semantik;
use super::speicher::{
    Ablage, AblageArt, AblageEinstellungen, AblageFassung, AblageImportZeile, AblageKollisionswahl,
    AblageLoeschBericht, AblageSicht,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};

/// Ereignis an die Oberfläche: eine Textfassung ist fertig (oder fehlgeschlagen).
pub const EREIGNIS_TEXT: &str = "ablage-text";
/// Ereignis an die Oberfläche: Zustand der semantischen Suche hat sich geändert (Vektoren fertig).
pub const EREIGNIS_SEMANTIK: &str = "ablage-semantik";

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
    if sicht.lage.is_none() && crate::settings::get_settings(&app).ablage_semantische_suche {
        nachziehen(&app);
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
    mit_nachlauf(&app, false, move |a| {
        a.ordner_einstellungen(&id, &einstellungen)
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

// ---------------------------------------------------------------------------
// D6: semantische Suche
// ---------------------------------------------------------------------------

fn models_root(app: &AppHandle) -> Antwort<PathBuf> {
    Ok(crate::portable::app_data_dir(app)
        .map_err(|e| e.to_string())?
        .join("models"))
}

/// Startet das Nachziehen der Vektoren im Hintergrund (nur wenn eingeschaltet).
fn nachziehen(app: &AppHandle) {
    let Ok(a) = ablage(app) else {
        return;
    };
    let melder = app.clone();
    semantik::nachziehen_anstossen(
        a.dir().to_path_buf(),
        Box::new(move || {
            let _ = melder.emit(EREIGNIS_SEMANTIK, ());
        }),
    );
}

static LAEDT: AtomicBool = AtomicBool::new(false);

/// Zustand der semantischen Suche für die Seite „Ablage“.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageSemantik {
    /// Einstellung „Semantische Suche“.
    pub an: bool,
    /// Ist die semantische Suche Standard (Eval-Tor bestanden)?
    pub standard: bool,
    /// Modell und Zerleger liegen vor.
    pub modell_bereit: bool,
    /// Download läuft.
    pub laedt: bool,
    /// Downloadgröße in MB.
    pub groesse_mb: u32,
    /// Das Modell ließ sich nicht laden (lexikalische Suche aktiv).
    pub ladefehler: bool,
    /// Vektoren werden gerade erstellt.
    pub nachziehen: bool,
    /// Stücke ohne Vektor (bis zum Ende des Nachziehens wirkt nur die lexikalische Suche).
    pub vektoren_fehlen: u32,
    /// Die Suche nutzt gerade Vektoren.
    pub aktiv: bool,
}

fn semantik_stand(app: &AppHandle, a: &super::speicher::Ablage) -> Antwort<AblageSemantik> {
    let an = crate::settings::get_settings(app).ablage_semantische_suche;
    let root = models_root(app)?;
    let groesse: u64 = semantik::katalog()
        .map(|e| e.files.iter().map(|f| f.size_bytes).sum())
        .unwrap_or(0);
    let modell_bereit = semantik::modell_bereit(&root);
    let emb = if an && modell_bereit {
        semantik::aktuell()
    } else {
        semantik::AblageEinbettung::Keine
    };
    let (neu, fehlen) = super::index::vektor_stand(a, &emb).unwrap_or((true, 0));
    let nachziehen = semantik::nachziehen_laeuft();
    Ok(AblageSemantik {
        an,
        standard: semantik::SEMANTIK_STANDARD,
        modell_bereit,
        laedt: LAEDT.load(Ordering::Relaxed),
        groesse_mb: groesse.div_ceil(1_000_000).min(u64::from(u32::MAX)) as u32,
        ladefehler: an && modell_bereit && semantik::ladefehler(),
        nachziehen,
        vektoren_fehlen: fehlen,
        aktiv: emb.ist_semantisch() && !neu && fehlen == 0 && !nachziehen,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_semantik_stand(app: AppHandle) -> Antwort<AblageSemantik> {
    let a = ablage(&app)?;
    let h = app.clone();
    tauri::async_runtime::spawn_blocking(move || semantik_stand(&h, &a))
        .await
        .map_err(|_| "ablage.semantik: Der Zustand ist gerade nicht verfügbar.".to_string())?
}

/// Einstellung „Semantische Suche“. Einschalten bettet die vorhandenen Stücke im Hintergrund ein (nur
/// mit geladenem Modell); Ausschalten gibt das Modell frei, die Suche ist sofort wieder lexikalisch.
#[tauri::command]
#[specta::specta]
pub async fn ablage_semantik_setzen(app: AppHandle, an: bool) -> Antwort<AblageSemantik> {
    let mut settings = crate::settings::get_settings(&app);
    settings.ablage_semantische_suche = an;
    crate::settings::write_settings(&app, settings);
    semantik::einrichten(&models_root(&app)?, an);
    let a = ablage(&app)?;
    let h = app.clone();
    let stand = tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = super::index::zwischenspeicher_leeren(&a) {
            log::warn!("ablage: Zwischenspeicher: {}", e.code);
        }
        semantik_stand(&h, &a)
    })
    .await
    .map_err(|_| "ablage.semantik: Der Zustand ist gerade nicht verfügbar.".to_string())??;
    if an {
        nachziehen(&app);
    }
    Ok(stand)
}

/// Lädt Modell und Zerleger (auf Knopfdruck), je Datei mit Größe und SHA-256 aus dem Katalog.
/// Fortschritt über `model-download-progress` (Kennung [`semantik::MODELL_ID`]).
#[tauri::command]
#[specta::specta]
pub async fn ablage_semantik_laden(app: AppHandle) -> Antwort<AblageSemantik> {
    use crate::managers::model::download::{HttpDownloadEvent, HttpDownloadOutcome};
    use crate::managers::model::ModelManager;
    let eintrag = semantik::katalog().ok_or_else(|| "semantik.katalog".to_string())?;
    let root = models_root(&app)?;
    let ordner = semantik::modell_ordner(&root);
    if LAEDT.swap(true, Ordering::SeqCst) {
        return Err("semantik.laedt: Das Modell wird bereits geladen.".into());
    }
    struct Zurueck;
    impl Drop for Zurueck {
        fn drop(&mut self) {
            LAEDT.store(false, Ordering::SeqCst);
        }
    }
    let _zurueck = Zurueck;
    std::fs::create_dir_all(&ordner).map_err(|_| "semantik.ordner".to_string())?;
    for datei in &eintrag.files {
        let ziel = ordner.join(&datei.filename);
        if ziel.is_file() {
            continue;
        }
        let teil = PathBuf::from(format!("{}.partial", ziel.display()));
        let handle = app.clone();
        let emit = move |ev: HttpDownloadEvent<'_>| {
            if let HttpDownloadEvent::Progress(p) = ev {
                let _ = handle.emit("model-download-progress", p);
            }
        };
        let cancel = hf_hub::api::tokio::CancellationToken::new();
        let ergebnis = ModelManager::download_http_resumable_with_events(
            semantik::MODELL_ID,
            &datei.url,
            &teil,
            Some(datei.size_bytes),
            datei.sha256.as_deref(),
            &cancel,
            &emit,
        )
        .await
        .map_err(|_| "semantik.download: Das Modell konnte nicht geladen werden.".to_string())?;
        if !matches!(ergebnis, HttpDownloadOutcome::Completed) {
            return Err("semantik.download: Das Laden wurde abgebrochen.".into());
        }
        std::fs::rename(&teil, &ziel).map_err(|_| "semantik.ordner".to_string())?;
    }
    let an = crate::settings::get_settings(&app).ablage_semantische_suche;
    semantik::einrichten(&root, an);
    let _ = app.emit("model-download-complete", semantik::MODELL_ID);
    drop(_zurueck);
    if an {
        nachziehen(&app);
    }
    let a = ablage(&app)?;
    let h = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Die Suchart kann gewechselt haben: keine Rangfolge der bisherigen wiederverwenden.
        let _ = super::index::zwischenspeicher_leeren(&a);
        semantik_stand(&h, &a)
    })
    .await
    .map_err(|_| "ablage.semantik: Der Zustand ist gerade nicht verfügbar.".to_string())?
}

/// Prozesszustand beim Start: Modellordner und Schalter (ohne das Modell schon zu laden).
pub fn semantik_einrichten(app: &AppHandle) {
    if let Ok(root) = models_root(app) {
        semantik::einrichten(
            &root,
            crate::settings::get_settings(app).ablage_semantische_suche,
        );
    }
}
