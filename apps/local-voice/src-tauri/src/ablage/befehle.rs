//! Tauri-Befehle der Dokumentenablage (D1, D2, D3-Vorbau: Cloud-Ausnahme, KI-Entwurf). Die Ablage liegt
//! unter `<Datenordner>/ablage/`, der Datenordner kommt wie bei den Projekten aus
//! `portable::app_data_dir`. Fehlertexte enthalten nie Namen, Pfade oder Inhalte; an die Oberfläche geht
//! `<code>: <text>`.
//!
//! D2: Nach Änderungen am Bestand wird der Index sofort abgeglichen (Papierkorb, Verschieben, Löschen
//! wirken ohne Verzögerung auf die Suche) und der Textarbeiter angestoßen. Er meldet jedes Ergebnis mit
//! dem Ereignis [`EREIGNIS_TEXT`]; die Seite lädt dann den Stand neu.
//!
//! D7: verknüpfte Ordner ([`ablage_verknuepfen`], [`ablage_verknuepfung_loesen`],
//! [`ablage_verknuepft_abtasten`]); der Dateibeobachter startet mit der App ([`beobachter_einrichten`]) und meldet
//! übernommene Änderungen wie der Textarbeiter mit [`EREIGNIS_TEXT`].
//!
//! D6: Schalter und Modell der semantischen Suche ([`ablage_semantik_stand`], [`ablage_semantik_setzen`],
//! [`ablage_semantik_laden`]); das Ende des Nachziehens der Vektoren meldet [`EREIGNIS_SEMANTIK`].

use super::freigabe::KiStufe;
use super::index::{AblageIndexBericht, AblageSuchtreffer};
use super::semantik;
use super::speicher::{
    Ablage, AblageArt, AblageEinstellungen, AblageEntwurf, AblageFassung, AblageImportZeile,
    AblageKollisionswahl, AblageLoeschBericht, AblageSicht,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};

/// Ereignis an die Oberfläche: eine Textfassung ist fertig (oder fehlgeschlagen).
pub const EREIGNIS_TEXT: &str = "ablage-text";
/// Ereignis an die Oberfläche: Zustand der semantischen Suche hat sich geändert (Vektoren fertig).
pub const EREIGNIS_SEMANTIK: &str = "ablage-semantik";

/// Ereignis an die Oberfläche: ein Abgleich der Ablage ist gelaufen (D4); Stand und Sync-Zustand neu laden.
pub const EREIGNIS_SYNC: &str = "ablage-sync";

type Antwort<T> = Result<T, String>;

fn ablage(app: &AppHandle) -> Antwort<Ablage> {
    let dir = crate::portable::app_data_dir(app)
        .map_err(|e| e.to_string())?
        .join("ablage");
    Ok(Ablage::neu(dir))
}

/// Nach einer Änderung, die Freigaben, Klassen oder den Bestand der Ablage verengen kann: laufende
/// Projektläufe mit Ablage-Inhalten abbrechen (lva#365 P4c-1, Widerruf). Auch nach einem Fehler, weil
/// eine halb ausgeführte Änderung den Bestand schon verändert haben kann. Welche Befehle das tun
/// müssen, steht in `ABLAGE_WIDERRUF`; der Quelltest `ablage_aenderungen_widerrufen_laufende_projektlaeufe`
/// (tests_p4c) prüft, dass jeder davon [`widerrufen`] ruft.
fn widerrufen<T>(app: &AppHandle, ergebnis: Antwort<T>) -> Antwort<T> {
    crate::projekte::zustand::ablage_geaendert(app);
    ergebnis
}

/// Befehle, die nach ihrer Ausführung [`widerrufen`] rufen müssen.
#[cfg(test)]
pub(crate) const ABLAGE_WIDERRUF: [&str; 9] = [
    "ablage_ordner_verschieben",
    "ablage_ordner_einstellungen",
    "ablage_ki_setzen",
    "ablage_cloud_ausnahme_setzen",
    "ablage_freigaben_neu_festlegen",
    "ablage_in_papierkorb",
    "ablage_endgueltig_loeschen",
    "ablage_papierkorb_leeren",
    "ablage_datei_verschieben",
];

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
    // Ein früher fehlgeschlagener Indexabgleich (Marke) wird beim Öffnen der Ablage wiederholt.
    let _ = blockierend(&app, |a| {
        if let Err(e) = super::index::nachholen(&a) {
            log::warn!("ablage: Indexabgleich (Wiederholung): {}", e.code);
        }
        Ok(())
    })
    .await;
    let sicht = blockierend(&app, |a| a.ansicht()).await?;
    // D7: Dateibeobachter der verknüpften Ordner (einmal je Prozess).
    if sicht.lage.is_none() && !sicht.verknuepft.is_empty() {
        let a = ablage(&app)?;
        let melder = app.clone();
        super::verknuepft::beobachter_starten(
            a.dir().to_path_buf(),
            Box::new(move || {
                let _ = melder.emit(EREIGNIS_TEXT, ());
            }),
        );
    }
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
    let r = mit_nachlauf(&app, false, move |a| {
        a.ordner_verschieben(&id, eltern.as_deref())
    })
    .await;
    widerrufen(&app, r)
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
    let r = mit_nachlauf(&app, false, move |a| {
        a.ordner_einstellungen(&id, &einstellungen)?;
        // Wieder synchron (D4): Rückweg lokal → sync sofort; scheitert er, setzt der Sync-Takt fort.
        if let Err(e) = a.rueckweg(&super::speicher::ProjektVerweise::neu(projekte)) {
            log::warn!("ablage: Rückweg lokal → sync: {}", e.code);
        }
        Ok(())
    })
    .await;
    widerrufen(&app, r)
}

/// KI-Freigabe eines Ordners (gerätelokal); `null` = erbt wieder.
#[tauri::command]
#[specta::specta]
pub async fn ablage_ki_setzen(app: AppHandle, id: String, stufe: Option<KiStufe>) -> Antwort<()> {
    let r = mit_nachlauf(&app, false, move |a| a.ki_setzen(&id, stufe)).await;
    widerrufen(&app, r)
}

/// Cloud-Ausnahme je (Ordner, Anbieter) für einen vertraulichen Ordner erteilen (`an`) oder entziehen
/// (D3, Spec §5.1). Gerätelokal in `freigabe.json`.
#[tauri::command]
#[specta::specta]
pub async fn ablage_cloud_ausnahme_setzen(
    app: AppHandle,
    ordner: String,
    anbieter: String,
    an: bool,
) -> Antwort<()> {
    // Erteilen nur für Anbieter, die Regelwerk und Datenklasse jetzt zulassen (die Liste kommt aus den
    // Einstellungen, nie aus der Oberfläche); Entziehen geht immer.
    let zulaessig = !an || {
        let s = crate::settings::get_settings(&app);
        super::anbieter::zulaessig(&s, &crate::managers::compliance::today(), &anbieter)
    };
    let r = blockierend(&app, move |a| {
        if !zulaessig {
            return Err(super::Fehler::neu(
                "ablage.anbieter_nicht_zugelassen",
                "Für diesen Anbieter ist keine Cloud-Ausnahme möglich.",
            ));
        }
        a.cloud_ausnahme_setzen(&ordner, &anbieter, an)
    })
    .await;
    widerrufen(&app, r)
}

/// Anbieterliste für die Cloud-Ausnahme (D7-Rest): konfigurierte Verbindungen, gefiltert nach Regelwerk
/// und Datenklasse; nicht wählbare stehen mit Grund darin.
#[tauri::command]
#[specta::specta]
pub fn ablage_cloud_anbieter(app: AppHandle) -> super::anbieter::AblageAnbieterListe {
    let s = crate::settings::get_settings(&app);
    super::anbieter::anbieterliste(&s, &crate::managers::compliance::today())
}

/// Einziges schreibendes KI-Werkzeug (D3, Spec §5.5): legt einen Entwurf (Markdown) als neue Datei in
/// „KI-Entwürfe“ oder in einem Zielordner mit `zielordner_ki` ab. Klasse = Maximum aus Zielordner und
/// Quellen; ist eine Quelle „nur dieses Gerät“, landet er in „KI-Entwürfe (nur dieses Gerät)“. `quellen`:
/// Herkunftsmarken (`dokument:<dat>@<fas>#<textrev>`), Verweise oder `dat_…`. `mindestklasse`
/// (`public` … `personal`): untere Schranke der Klasse; der Assistent übergibt das Maximum aller im
/// Gespräch gelieferten Auszüge, weil die vom Modell benannten `quellen` unvollständig sein können.
#[tauri::command]
#[specta::specta]
pub async fn ablage_entwurf_anlegen(
    app: AppHandle,
    ordner: Option<String>,
    name: String,
    inhalt_md: String,
    quellen: Vec<String>,
    mindestklasse: Option<String>,
) -> Antwort<AblageEntwurf> {
    mit_nachlauf(&app, false, move |a| {
        let mindest = match mindestklasse.as_deref() {
            None => None,
            Some(t) => Some(super::zusammenfuehren::Klasse::aus_text(t).ok_or_else(|| {
                super::Fehler::ungueltig("Unbekannte Mindestklasse des Entwurfs.")
            })?),
        };
        super::ki::entwurf_anlegen(&a, ordner.as_deref(), &name, &inhalt_md, &quellen, mindest)
    })
    .await
}

/// Legt einen KI-Entwurf für die Projekt-KI an (lva#365 P4c-3, KI-Dokument). Nur aus Rust erreichbar, kein
/// Befehl: Die Mindestklasse kommt vom Versandtor und ist hier ein fester Parameter, den keine
/// Oberfläche setzt oder senkt. Wie der Befehl gleicht der Aufruf danach den Index ab und weckt den
/// Textarbeiter. Der Fehler ist `<code>: <text>` der Ablage, ohne Namen, Pfade oder Inhalte.
pub(crate) async fn entwurf_fuer_projekt(
    app: &AppHandle,
    ordner: Option<String>,
    name: String,
    inhalt_md: String,
    quellen: Vec<String>,
    mindestklasse: super::zusammenfuehren::Klasse,
) -> Antwort<AblageEntwurf> {
    mit_nachlauf(app, false, move |a| {
        super::ki::entwurf_anlegen(
            &a,
            ordner.as_deref(),
            &name,
            &inhalt_md,
            &quellen,
            Some(mindestklasse),
        )
    })
    .await
}

/// „Freigaben neu festlegen“: nur bei unklarer (defekter) Freigabe; alle Ordner auf „aus“.
#[tauri::command]
#[specta::specta]
pub async fn ablage_freigaben_neu_festlegen(app: AppHandle) -> Antwort<()> {
    let r = mit_nachlauf(&app, false, |a| a.freigaben_neu_festlegen()).await;
    widerrufen(&app, r)
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
    let r = mit_nachlauf(&app, false, move |a| a.in_papierkorb(art, &id)).await;
    widerrufen(&app, r)
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
    let r = mit_nachlauf(&app, false, move |a| a.endgueltig_loeschen(art, &id)).await;
    widerrufen(&app, r)
}

#[tauri::command]
#[specta::specta]
pub async fn ablage_papierkorb_leeren(app: AppHandle) -> Antwort<AblageLoeschBericht> {
    let r = mit_nachlauf(&app, false, |a| a.papierkorb_leeren()).await;
    widerrufen(&app, r)
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
    let r = mit_nachlauf(&app, false, move |a| a.datei_verschieben(&id, &ordner)).await;
    widerrufen(&app, r)
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

/// D7: verknüpft einen Ordner des Geräts (nur lesen, nur dieses Gerät, nie hochgeladen). Liefert die
/// Kennung; danach wird er gleich abgetastet, der Index abgeglichen und der Textarbeiter angestoßen.
#[tauri::command]
#[specta::specta]
pub async fn ablage_verknuepfen(app: AppHandle, pfad: String) -> Antwort<String> {
    let quelle = PathBuf::from(pfad);
    let id = mit_nachlauf(&app, true, move |a| {
        let id = super::verknuepft::verknuepfen(&a, &quelle)?;
        if let Err(e) = super::verknuepft::abtasten(&a, &id) {
            log::warn!("ablage: verknüpfter Ordner nicht abgetastet: {}", e.code);
        }
        Ok(id)
    })
    .await?;
    let a = ablage(&app)?;
    let melder = app.clone();
    super::verknuepft::beobachter_starten(
        a.dir().to_path_buf(),
        Box::new(move || {
            let _ = melder.emit(EREIGNIS_TEXT, ());
        }),
    );
    Ok(id)
}

/// D7: löst eine Verknüpfung. Der Ordner und seine Dateien bleiben unberührt; Textfassungen, Index und
/// Freigabe dieses Geräts zu ihm verschwinden.
#[tauri::command]
#[specta::specta]
pub async fn ablage_verknuepfung_loesen(app: AppHandle, id: String) -> Antwort<()> {
    mit_nachlauf(&app, false, move |a| super::verknuepft::loesen(&a, &id)).await
}

/// D7: „Neu einlesen“: einen (oder ohne Kennung alle) verknüpften Ordner sofort abtasten.
#[tauri::command]
#[specta::specta]
pub async fn ablage_verknuepft_abtasten(app: AppHandle, id: Option<String>) -> Antwort<bool> {
    mit_nachlauf(&app, true, move |a| match id {
        Some(id) => {
            super::ordner_id_pruefen(&id)?;
            super::verknuepft::marken_zuruecksetzen(&a, Some(&id))?;
            super::verknuepft::abtasten(&a, &id)
        }
        None => {
            super::verknuepft::marken_zuruecksetzen(&a, None)?;
            super::verknuepft::alle_abtasten(&a)
        }
    })
    .await
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
    let bericht = blockierend(&app, |a| super::index::neu_aufbauen(&a)).await?;
    // D6: Stücke, die beim Neuaufbau ohne Vektor blieben (Nachziehen lief parallel), nachholen.
    if crate::settings::get_settings(&app).ablage_semantische_suche {
        nachziehen(&app);
    }
    Ok(bericht)
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
    // Ohne lesbaren Schlüssel gilt das Konto als nicht verbunden (der Lauf liefe ebenso nicht).
    let konto = crate::sync::account::load(&app).and_then(|cfg| {
        cfg.key()
            .ok()
            .map(|k| crate::sync::ablage::konto_kennung(cfg.user_id, &k, &cfg.url))
    });
    blockierend(&app, move |a| {
        crate::sync::ablage::stand(&a, konto.as_deref())
    })
    .await
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
/// Fortschritt des laufenden Downloads über alle Dateien (0–100), auch nach Neuaufbau der Seite.
static PROZENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Ereignis mit dem Gesamtfortschritt (Prozent) des Modell-Downloads.
const EREIGNIS_FORTSCHRITT: &str = "ablage-semantik-fortschritt";
/// Abbruch des laufenden Downloads ([`ablage_semantik_abbrechen`]).
static ABBRUCH: std::sync::Mutex<Option<hf_hub::api::tokio::CancellationToken>> =
    std::sync::Mutex::new(None);

fn abbruch() -> std::sync::MutexGuard<'static, Option<hf_hub::api::tokio::CancellationToken>> {
    ABBRUCH.lock().unwrap_or_else(|e| e.into_inner())
}

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
    /// Fortschritt des Downloads über alle Dateien (0–100, nur bei `laedt`).
    pub prozent: u32,
    /// Halbe Downloads liegen im Modellordner (können entfernt werden).
    pub reste: bool,
    /// Downloadgröße in MB.
    pub groesse_mb: u32,
    /// Das Modell ließ sich nicht laden (lexikalische Suche aktiv).
    pub ladefehler: bool,
    /// Vektoren werden gerade erstellt.
    pub nachziehen: bool,
    /// Stücke ohne Vektor (bis zum Ende des Nachziehens wirkt nur die lexikalische Suche).
    pub vektoren_fehlen: u32,
    /// Der Index liegt noch nicht im Vektorraum des Modells (das Nachziehen übernimmt ihn).
    pub neuaufbau: bool,
    /// Der Zustand des Suchindex ließ sich nicht lesen.
    pub index_fehler: bool,
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
    let (neu, fehlen, index_fehler) = match super::index::vektor_stand(a, &emb) {
        Ok((neu, fehlen)) => (neu, fehlen, false),
        Err(_) => (false, 0, true),
    };
    let nachziehen = semantik::nachziehen_laeuft();
    let laedt = LAEDT.load(Ordering::Relaxed);
    Ok(AblageSemantik {
        an,
        standard: semantik::SEMANTIK_STANDARD,
        modell_bereit,
        laedt,
        prozent: if laedt {
            PROZENT.load(Ordering::Relaxed).min(100)
        } else {
            0
        },
        reste: !laedt && semantik::reste_vorhanden(&root),
        groesse_mb: groesse.div_ceil(1_000_000).min(u64::from(u32::MAX)) as u32,
        ladefehler: an && modell_bereit && semantik::ladefehler(),
        nachziehen,
        vektoren_fehlen: fehlen,
        neuaufbau: emb.ist_semantisch() && neu,
        index_fehler: emb.ist_semantisch() && index_fehler,
        aktiv: emb.ist_semantisch() && !index_fehler && !neu && fehlen == 0 && !nachziehen,
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
    let root = models_root(&app)?;
    // Ausschalten gibt das Modell frei (rund 500 MB): nicht auf dem async-Thread.
    tauri::async_runtime::spawn_blocking(move || semantik::einrichten(&root, an))
        .await
        .map_err(|_| "ablage.semantik: Der Zustand ist gerade nicht verfügbar.".to_string())?;
    // Der Abfrage-Zwischenspeicher trägt die Kennung der Einbettung im Schlüssel: eine Rangfolge der
    // anderen Suchart kommt nicht wieder, Leeren ist nicht nötig.
    let a = ablage(&app)?;
    let h = app.clone();
    let stand = tauri::async_runtime::spawn_blocking(move || semantik_stand(&h, &a))
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
    let eintrag = semantik::katalog()
        .ok_or_else(|| "semantik.katalog: Das Modell steht nicht im Katalog.".to_string())?;
    let root = models_root(&app)?;
    let ordner = semantik::modell_ordner(&root);
    if LAEDT.swap(true, Ordering::SeqCst) {
        return Err("semantik.laedt: Das Modell wird bereits geladen.".into());
    }
    /// Räumt den Download-Zustand auf und meldet das Ende (auch einer neu geöffneten Seite, die nicht
    /// auf diesen Aufruf wartet), bei Erfolg, Fehler und Abbruch.
    struct Zurueck(AppHandle);
    impl Drop for Zurueck {
        fn drop(&mut self) {
            *abbruch() = None;
            LAEDT.store(false, Ordering::SeqCst);
            PROZENT.store(0, Ordering::Relaxed);
            let _ = self.0.emit(EREIGNIS_SEMANTIK, ());
        }
    }
    let _zurueck = Zurueck(app.clone());
    let cancel = hf_hub::api::tokio::CancellationToken::new();
    *abbruch() = Some(cancel.clone());
    PROZENT.store(0, Ordering::Relaxed);
    let gesamt: u64 = eintrag
        .files
        .iter()
        .map(|f| f.size_bytes)
        .sum::<u64>()
        .max(1);
    let mut fertig_bytes: u64 = 0;
    std::fs::create_dir_all(&ordner).map_err(|_| {
        "semantik.speichern: Das Modell konnte nicht gespeichert werden.".to_string()
    })?;
    for datei in &eintrag.files {
        let ziel = ordner.join(&datei.filename);
        if ziel.is_file() {
            fertig_bytes += datei.size_bytes;
            continue;
        }
        let teil = PathBuf::from(format!("{}.partial", ziel.display()));
        let handle = app.clone();
        let basis = fertig_bytes;
        let emit = move |ev: HttpDownloadEvent<'_>| {
            if let HttpDownloadEvent::Progress(p) = ev {
                let neu = ((basis + p.downloaded).min(gesamt) * 100 / gesamt) as u32;
                // Nur bei Änderung melden (ganze Prozent).
                if PROZENT.swap(neu, Ordering::Relaxed) != neu {
                    let _ = handle.emit(EREIGNIS_FORTSCHRITT, neu);
                }
                let _ = handle.emit("model-download-progress", p);
            }
        };
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
            return Err("semantik.abgebrochen: Das Laden wurde abgebrochen.".into());
        }
        std::fs::rename(&teil, &ziel).map_err(|_| {
            "semantik.speichern: Das Modell konnte nicht gespeichert werden.".to_string()
        })?;
        fertig_bytes += datei.size_bytes;
    }
    let an = crate::settings::get_settings(&app).ablage_semantische_suche;
    let r = root.clone();
    tauri::async_runtime::spawn_blocking(move || semantik::einrichten(&r, an))
        .await
        .map_err(|_| "ablage.semantik: Der Zustand ist gerade nicht verfügbar.".to_string())?;
    let _ = app.emit("model-download-complete", semantik::MODELL_ID);
    drop(_zurueck);
    if an {
        nachziehen(&app);
    }
    let a = ablage(&app)?;
    let h = app.clone();
    tauri::async_runtime::spawn_blocking(move || semantik_stand(&h, &a))
        .await
        .map_err(|_| "ablage.semantik: Der Zustand ist gerade nicht verfügbar.".to_string())?
}

/// Bricht einen laufenden Modell-Download ab (halbe Dateien bleiben für die Fortsetzung liegen).
#[tauri::command]
#[specta::specta]
pub async fn ablage_semantik_abbrechen() -> Antwort<()> {
    if let Some(c) = abbruch().as_ref() {
        c.cancel();
    }
    Ok(())
}

/// Entfernt das Modell vom Gerät (auch halbe Downloads). Die Suche ist danach lexikalisch; die
/// Einstellung bleibt, ein erneutes Laden stellt die semantische Suche wieder her.
#[tauri::command]
#[specta::specta]
pub async fn ablage_semantik_loeschen(app: AppHandle) -> Antwort<AblageSemantik> {
    if LAEDT.load(Ordering::SeqCst) {
        return Err("semantik.laedt: Das Modell wird gerade geladen.".into());
    }
    let root = models_root(&app)?;
    let a = ablage(&app)?;
    let h = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        semantik::modell_entfernen(&root).map_err(|code| match code.as_str() {
            "semantik.belegt" => {
                "semantik.belegt: Der Suchindex wird gerade erweitert. Bitte später erneut versuchen."
                    .to_string()
            }
            _ => "semantik.entfernen: Das Modell konnte nicht entfernt werden.".to_string(),
        })?;
        semantik_stand(&h, &a)
    })
    .await
    .map_err(|_| "ablage.semantik: Der Zustand ist gerade nicht verfügbar.".to_string())?
}

/// Beim App-Start: den Dateibeobachter der verknüpften Ordner starten, wenn es welche gibt (sonst erst
/// beim Verknüpfen des ersten Ordners), statt erst nach dem ersten Öffnen der Seite „Ablage“.
pub fn beobachter_einrichten(app: &AppHandle) {
    let Ok(a) = ablage(app) else {
        return;
    };
    if super::verknuepft::kennungen(a.dir()).is_empty() {
        return;
    }
    let melder = app.clone();
    super::verknuepft::beobachter_starten(
        a.dir().to_path_buf(),
        Box::new(move || {
            let _ = melder.emit(EREIGNIS_TEXT, ());
        }),
    );
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
