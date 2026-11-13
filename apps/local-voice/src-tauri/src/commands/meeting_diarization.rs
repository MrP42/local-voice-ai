//! Modelle der Sprechertrennung (Katalog `Purpose::Diarization`): Stand
//! abfragen und auf Knopfdruck laden.
//!
//! Bis 05.11.2026 gab es dafuer keinen Weg in der App: das Modell wird
//! bewusst nicht mitgeliefert (NVIDIA Open Model License, 139 MB, jedes Update
//! waere sonst so viel groesser), aber es liess sich auch nirgends laden. Jede
//! Besprechung zeigte deshalb nur "Modell zur Sprechertrennung ist nicht
//! installiert". Abgelegt wird es dort, wo `diarize::resolve_model_path` sucht.
//!
//! N1: Ein Eintrag kann aus mehreren Dateien bestehen (Nemotron: ONNX-Graph und
//! Gewichte, die nebeneinander liegen muessen). Der Ladeweg arbeitet je Kennung
//! ([`download_model`], [`model_status`]); die Tauri-Befehle bleiben auf dem
//! Standardmodell (Sortformer, unveraenderte Signatur).
//!
//! Fehlerfaelle des Ladens:
//! - Abbruch/Absturz mitten in einer Datei: sie liegt als `<name>.partial`
//!   neben dem Ziel und wird beim naechsten Aufruf fortgesetzt; umbenannt wird
//!   erst nach bestandener Groessen- und SHA-256-Pruefung, das Ziel ist also nie
//!   halb geschrieben;
//! - vorhandene Dateien (abgeschnitten, beschaedigt, von Hand hineinkopiert):
//!   das Bereitstellen ([`download_model`], damit auch `--provision`) prueft JEDE
//!   Datei des Eintrags nach Groesse und SHA-256 und ersetzt jede ungueltige
//!   durch einen verifizierten Download (Umbenennen ueber das Ziel, die alte
//!   Datei bleibt bis dahin liegen). Nur wenn alle gueltig sind, wird nichts
//!   geladen. Die Statusabfrage der Oberflaeche hasht dagegen nie (bei jedem
//!   Oeffnen waeren das 120 MB Lesen): sie prueft Vorhandensein und exakte
//!   Groesse, das faengt abgeschnittene Dateien. Gleich grosse, aber
//!   beschaedigte Bytes findet erst das Bereitstellen; eine Pruefmarke haette
//!   dieselbe Luecke (sie belegt nur, dass die Datei einmal gut war) und liesse
//!   Altbestaende ohne Marke als „nicht installiert“ erscheinen;
//! - Reihenfolge: die Datei, die der Motor oeffnet (die erste des Eintrags),
//!   kommt zuletzt; sie liegt im Normalfall nur da, wenn alle anderen zuvor erfolgreich
//!   geprueft wurden. Muss eine andere Datei ersetzt werden, wird sie vorher
//!   zurueckgezogen (nicht geloescht, sondern als `.partial` abgelegt, damit eine
//!   gueltige Datei nicht neu geladen werden muss), der Motor bekommt also im Normalfall
//!   (ohne gleichzeitigen Eingriff eines anderen Prozesses) nie einen Graphen neben
//!   ungeprueften oder defekten Gewichten, auch nicht, wenn die Reparatur scheitert. Die
//!   Reihenfolge der Umbenennungen ist keine Sicherheitsgrenze (das ist die Pruefung
//!   beim Laden, siehe unten), sie haelt nur den Normalfall sauber;
//! - Bedeutung von `Done` ([`Fetch::Done`]): ein abgeschlossener Bereitstellungsversuch
//!   mit erfolgreichen Lesepruefungen. Das ist ein Hinweis und KEINE Integritaetszusage
//!   fuer den Plattenbestand danach und keine gemeinsame Momentaufnahme des Paars (die
//!   Dateien werden nacheinander gelesen). Die Pruefung vor dem Erfolg ([`confirm`], und
//!   ebenso die der vorhandenen Dateien am Anfang) ist `anchor::check_set`: alle Dateien
//!   des Eintrags zugleich als Handle offen, Groesse und SHA-256 ueber die Handles,
//!   danach Name gegen Handle verglichen (ein Tausch durch eine andere Datei gleicher
//!   Groesse ist `Changed`, nie Erfolg). Mismatch, Abbruch oder Lesefehler: kein Erfolg,
//!   Fehler `verification_failed` bzw. der Grund, der Graph wird zurueckgezogen. Zuletzt
//!   liegt der Ordner noch dort, wohin der Pfad fuehrt (`Anchor::still_at`), sonst
//!   `unsafe_path`. Nicht erkannt wird, was `check_set` nicht sehen kann: eine
//!   In-place-Aenderung einer schon gelesenen Datei (gleicher Inode, gleiche Groesse)
//!   und jede Aenderung NACH dem Erfolg (zwischen Bereitstellen und Laden);
//! - Integritaet garantiert ausschliesslich der Lader
//!   (`nemotron::engine::OnnxBlockModel::load_verified`): er liest die Bytes selbst ueber
//!   Handles, prueft sie gegen den Katalog und baut die Sitzung aus genau diesen Bytes im
//!   Speicher, ohne einen Pfad ein zweites Mal zu oeffnen. Jeder weitere Verbraucher der
//!   Dateien und jeder Paketexport muss die Bytes, die er tatsaechlich verwendet, selbst
//!   verifizieren;
//! - Verknuepfungen und Ordnertausch: das Bereitstellen oeffnet VOR dem ersten
//!   Netzzugriff die Kette Datenwurzel -> Modellordner Komponente fuer Komponente
//!   ohne Link-Folgen und haelt den Modellordner als offenes Handle bis zum Ende;
//!   anlegen, anhaengen, abschneiden, lesen, umbenennen und loeschen laufen alle
//!   ueber dieses Handle und einen Dateinamen (`download::anchor`), nie ueber einen
//!   Pfad. Wird der Ordner im Dateisystem durch einen Link auf einen fremden Ordner
//!   ersetzt, landen alle Operationen weiter im alten Ordner (unter Windows laesst
//!   sich der Ordner nicht ersetzen, solange das Handle offen ist). Jeder Ordner der
//!   Kette wird nach dem Oeffnen gegen seinen Namen verglichen (Kennung), ein im
//!   letzten Moment durch einen Link ersetzter Ordner ist abgelehnt. Ein Symlink (unter
//!   Windows jeder Reparse-Punkt, also auch Junctions) als Ziel oder als `.partial`, ein
//!   Ordner dazwischen, der ein Link ist, eine Datei mit mehreren Namen (Hardlink,
//!   unter Windows `nNumberOfLinks`) als Teildatei oder etwas, das keine normale
//!   Datei ist, wird mit dem Fehler `unsafe_path` abgelehnt, bevor geschrieben,
//!   abgeschnitten, umbenannt oder geloescht wird. Die Wurzel selbst darf ein Link
//!   sein (verschobener Datenordner). Die Grenze, in diesen Worten: „Keine Umlenkung
//!   von App-Operationen auf unabhängige fremde Dateiobjekte. Zusätzliche Namen und
//!   Inhaltsmanipulationen an App-Dateiobjekten durch gleichberechtigte Prozesse
//!   desselben Benutzers sind ausgeklammert.“ Linkzahlpruefungen sind Momentpruefungen,
//!   kein Herkunftsnachweis; Vertrauensannahme: eine vorgefundene reguläre, einfach
//!   verlinkte `.partial` im Modellordner gilt als App-Datei (Genaueres im Kopf von
//!   `download::anchor`). Ein Katalogwert, der aus dem Modellordner
//!   hinausfuehren koennte (absoluter Pfad, `..`, Backslash, Laufwerksbuchstabe,
//!   Windows-Geraetename, zwei Namen nur in der Schreibweise verschieden), ergibt
//!   `catalog_invalid`;
//! - Datentraeger ohne Datei-Kennung: liefert er keine 128-Bit-Kennung fuer den
//!   Modellordner (Windows `FileIdInfo`; typisch FAT, je nach Treiber exFAT und manche
//!   Netzfreigaben), laesst sich weder ein Ordner- noch ein Namenstausch belegen. Das
//!   Bereitstellen meldet dann `volume_unsupported` (nicht `unsafe_path`: es ist kein Link
//!   im Weg), der Lader einen entsprechenden Ladefehler. Das Modell wird dort weder
//!   bereitgestellt noch geladen (fail closed, kein schwaecherer Ersatz); der Datenordner
//!   gehoert auf einen NTFS- oder ReFS-Datentraeger (`download::anchor`);
//! - voller Datentraeger: der Schreibfehler bricht ab, die Teildatei bleibt fuer
//!   die Fortsetzung, bereits fertige Dateien bleiben liegen;
//! - zwei Aufrufe gleichzeitig: einer laeuft, der andere meldet `download_running`.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use hf_hub::api::tokio::CancellationToken;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter};

use crate::catalog::{TtsCatalogEntry, TtsCatalogFile};
use crate::managers::meetings::diarize;
use crate::managers::model::download::anchor::{self, Anchor, Check, Wanted};
use crate::managers::model::download::{is_refused, safe, HttpDownloadEvent, HttpDownloadOutcome};
use crate::managers::model::ModelManager;

#[derive(Debug, Clone, Serialize, Type)]
pub struct DiarizationModelStatus {
    /// Katalogkennung (fuer die Fortschrittsereignisse `model-download-*`).
    pub model_id: String,
    // Nur Anzeige: alle Dateien liegen mit der Groesse aus dem Katalog da. Es wurde
    // nichts gelesen; das ist kein Integritaetsnachweis. (Bewusst kein Doc-Kommentar:
    // der landet in `bindings.ts`.)
    pub model_ready: bool,
    pub downloading: bool,
    pub size_mb: u32,
}

static DOWNLOADING: AtomicBool = AtomicBool::new(false);
/// Abbruch des laufenden Downloads (Knopf „Abbrechen“).
static CANCEL: std::sync::Mutex<Option<CancellationToken>> = std::sync::Mutex::new(None);

fn models_root(app: &AppHandle) -> PathBuf {
    crate::portable::app_data_dir(app)
        .map(|d| d.join("models"))
        .unwrap_or_default()
}

fn catalog_entry(id: &str) -> Option<TtsCatalogEntry> {
    crate::catalog::tts_entries(crate::catalog::Purpose::Diarization)
        .into_iter()
        .find(|e| e.id == id)
}

/// Die Dateien eines Eintrags mit ihrem Zielpfad, in Katalogreihenfolge.
/// `catalog_invalid`, wenn ein Katalogwert (Dateiname, Kennung) aus dem
/// Modellordner hinausfuehren koennte.
fn targets(
    models_root: &Path,
    entry: &TtsCatalogEntry,
) -> Result<Vec<(TtsCatalogFile, PathBuf)>, String> {
    let dir = diarize::catalog_model_dir(models_root, entry)
        .ok_or_else(|| "catalog_invalid".to_string())?;
    // Ohne Pruefsumme und Groesse gaebe es nichts, wogegen eine Datei zu verifizieren
    // waere: bereitgestellt (und spaeter geladen) wird nur, was sich pruefen laesst.
    if entry
        .files
        .iter()
        .any(|f| f.size_bytes == 0 || f.sha256.as_deref().is_none_or(|h| !is_sha256(h)))
    {
        return Err("catalog_invalid".to_string());
    }
    Ok(entry
        .files
        .iter()
        .map(|f| (f.clone(), dir.join(&f.filename)))
        .collect())
}

/// Eine SHA-256 in Hex: 64 Zeichen.
fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Fehlertext eines E/A-Fehlers; eine Ablehnung wegen Verknuepfung heisst
/// `unsafe_path`, ein Datentraeger ohne Datei-Kennung `volume_unsupported` (kein Pfad im
/// Text).
fn io_msg(e: &std::io::Error) -> String {
    if safe::is_refusal(e) {
        "unsafe_path".to_string()
    } else if safe::is_volume_unsupported(e) {
        "volume_unsupported".to_string()
    } else {
        e.to_string()
    }
}

/// Wie [`io_msg`] fuer Fehler des Downloaders.
fn net_msg(e: &anyhow::Error) -> String {
    if is_refused(e) {
        "unsafe_path".to_string()
    } else {
        e.to_string()
    }
}

/// Bereit (fuer die Anzeige) heisst: alle Dateien des Eintrags liegen an ihrem
/// Platz als normale Dateien (kein Link) und haben genau die Groesse aus dem
/// Katalog. Nur `stat`, kein Lesen der Inhalte, also KEIN Integritaetsnachweis; die
/// Pruefung mit SHA-256 macht [`provision`] (selbst nur ein Hinweis, siehe
/// [`Fetch::Done`]), die Integritaetszusage der Lader.
fn looks_complete(targets: &[(TtsCatalogFile, PathBuf)]) -> bool {
    !targets.is_empty()
        && targets
            .iter()
            .all(|(file, path)| safe::plain_len(path).is_ok_and(|len| len == Some(file.size_bytes)))
}

fn status_for(app: &AppHandle, id: &str) -> Result<DiarizationModelStatus, String> {
    let entry = catalog_entry(id).ok_or_else(|| "model_missing".to_string())?;
    let size: u64 = entry.files.iter().map(|f| f.size_bytes).sum();
    Ok(DiarizationModelStatus {
        model_id: entry.id.clone(),
        model_ready: looks_complete(&targets(&models_root(app), &entry)?),
        downloading: DOWNLOADING.load(Ordering::Relaxed),
        size_mb: size.div_ceil(1_000_000).min(u64::from(u32::MAX)) as u32,
    })
}

/// Stand des Modells `id` (Kennung aus dem Katalog, Zweck Sprechertrennung).
pub fn model_status(app: &AppHandle, id: &str) -> Result<DiarizationModelStatus, String> {
    status_for(app, id)
}

#[tauri::command]
#[specta::specta]
pub fn meeting_diarization_model_status(app: AppHandle) -> Result<DiarizationModelStatus, String> {
    status_for(&app, diarize::DEFAULT_MODEL_ID)
}

/// Wie ein Download endete: Abbruch ist ein Ergebnis, kein Fehler (die
/// Teildatei bleibt fuer die Fortsetzung).
#[derive(Debug, PartialEq, Eq)]
enum Fetch {
    /// Ein abgeschlossener Bereitstellungsversuch mit erfolgreichen Lesepruefungen
    /// (`confirm`, `Anchor::still_at`). Ein Hinweis, keine Integritaetszusage fuer den
    /// Plattenbestand danach und keine gemeinsame Momentaufnahme des Paars; den
    /// Nachweis fuehrt der Lader.
    Done,
    Cancelled,
}

/// Name der fortsetzbaren Teildatei neben der Datei `name`.
fn partial_name(name: &str) -> String {
    format!("{name}.partial")
}

/// Zustand einer Datei am Ziel gegenueber dem Katalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileState {
    Valid,
    Missing,
    /// Liegt da, hat aber nicht die Groesse oder die SHA-256 aus dem Katalog (oder
    /// wurde waehrend der Pruefung getauscht).
    Invalid,
}

impl From<Check> for FileState {
    fn from(check: Check) -> Self {
        match check {
            Check::Valid => Self::Valid,
            Check::Missing => Self::Missing,
            Check::Size(_) | Check::Hash | Check::Changed => Self::Invalid,
        }
    }
}

/// Die Ordner von `root` bis zum Ordner der Ziele (alle Ziele liegen im selben
/// Ordner). Ein Ziel ausserhalb von `root` oder mit einem Bestandteil, der kein
/// Name ist (`..`, absolut), ist `unsafe_path`.
fn place(root: &Path, targets: &[(TtsCatalogFile, PathBuf)]) -> Result<Vec<String>, String> {
    let dir = targets
        .first()
        .and_then(|(_, dest)| dest.parent())
        .ok_or_else(|| "model_missing".to_string())?;
    if targets.iter().any(|(_, dest)| dest.parent() != Some(dir)) {
        return Err("catalog_invalid".into());
    }
    let rel = dir
        .strip_prefix(root)
        .map_err(|_| "unsafe_path".to_string())?;
    rel.components()
        .map(|part| match part {
            Component::Normal(name) => name
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "catalog_invalid".to_string()),
            _ => Err("unsafe_path".to_string()),
        })
        .collect()
}

/// Prueft alle Dateien ueber den Anker (`anchor::check_set`: alle Handles zugleich
/// offen, Groesse und SHA-256 ueber die Handles, Name gegen Handle), im
/// Hintergrundthread. `None`: abgebrochen. Ein Link oder eine Nicht-Datei ist ein
/// Fehler (Ablehnung), nie eine Datei, der gefolgt wird.
async fn verify_all(
    anchor: &Arc<Anchor>,
    targets: &[(TtsCatalogFile, PathBuf)],
    cancel: &CancellationToken,
) -> Result<Option<Vec<FileState>>, String> {
    let anchor = anchor.clone();
    let owned: Vec<(String, u64, Option<String>)> = targets
        .iter()
        .map(|(f, _)| (f.filename.clone(), f.size_bytes, f.sha256.clone()))
        .collect();
    let token = cancel.clone();
    let checked = tokio::task::spawn_blocking(move || {
        let wanted: Vec<Wanted<'_>> = owned
            .iter()
            .map(|(name, size, sha)| Wanted {
                name,
                size: *size,
                sha256: sha.as_deref(),
            })
            .collect();
        anchor::check_set(&anchor, &wanted, &|| token.is_cancelled(), None)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| io_msg(&e))?;
    Ok(checked.map(|states| states.into_iter().map(FileState::from).collect()))
}

/// Zieht die Datei `name` zurueck, die der Motor oeffnet (die erste des Eintrags),
/// damit er sie nicht neben ungeprueften oder defekten Dateien findet. Ungueltig:
/// geloescht; fehlend: nichts zu tun. Gueltig oder unbekannt (`None`, etwa nach
/// Abbruch oder Lesefehler): als `.partial` ablegen (kommt ohne Netz zurueck, der
/// Downloader prueft sie dann erneut); geht das nicht (ein Link, ein Hindernis,
/// fehlende Rechte), den Eintrag selbst entfernen ([`Anchor::unlink`]: ein Link
/// verschwindet, sein Ziel bleibt). Scheitert auch das, ist es ein Fehler: der
/// Aufrufer meldet dann keinen Erfolg. Alles ueber den Anker, nie ueber einen Pfad.
fn withdraw_graph(anchor: &Anchor, name: &str, state: Option<FileState>) -> Result<(), String> {
    match state {
        Some(FileState::Missing) => Ok(()),
        Some(FileState::Invalid) => anchor.remove(name).map_err(|e| io_msg(&e)),
        Some(FileState::Valid) | None => match anchor.rename_over(name, &partial_name(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => anchor.unlink(name).map_err(|e| io_msg(&e)),
        },
    }
}

/// Das Ende eines Erfolgs: der Ordner liegt noch dort, wohin der Pfad fuehrt.
fn finish(anchor: &Anchor) -> Result<Fetch, String> {
    anchor.still_at().map_err(|e| io_msg(&e))?;
    Ok(Fetch::Done)
}

/// Die letzte Pruefung vor dem Erfolg: alle Dateien des Eintrags erneut, jetzt,
/// nachdem alles an seinem Platz liegt, ueber denselben Weg, den der Lader geht
/// ([`verify_all`]). Stimmt eine nicht (getauscht, beschaedigt, durch einen Link
/// ersetzt) oder wird die Pruefung abgebrochen, gibt es keinen Erfolg und der Graph
/// wird zurueckgezogen ([`withdraw_graph`]). `verification_failed`, wenn der Inhalt
/// nicht stimmt.
async fn confirm(
    anchor: &Arc<Anchor>,
    targets: &[(TtsCatalogFile, PathBuf)],
    cancel: &CancellationToken,
) -> Result<Fetch, String> {
    let outcome = verify_all(anchor, targets, cancel).await;
    if let Ok(Some(states)) = &outcome {
        if states.iter().all(|s| *s == FileState::Valid) {
            return finish(anchor);
        }
    }
    let graph_state = match &outcome {
        Ok(Some(states)) => states.first().copied(),
        _ => None,
    };
    let reason = match &outcome {
        Ok(None) => None,
        Ok(Some(_)) => Some("verification_failed".to_string()),
        Err(e) => Some(e.clone()),
    };
    if let Err(w) = withdraw_graph(anchor, &targets[0].0.filename, graph_state) {
        return Err(format!(
            "{}; graph_not_withdrawn: {w}",
            reason.as_deref().unwrap_or("cancelled")
        ));
    }
    match reason {
        None => Ok(Fetch::Cancelled),
        Some(reason) => Err(reason),
    }
}

/// Laedt EINE Datei nach `<name>.partial` und benennt sie erst nach bestandener
/// Pruefung ueber das Ziel um. Die Teildatei wird VOR der Anfrage ueber den Anker
/// geoeffnet (Link, Hardlink und Nicht-Datei sind dann schon abgelehnt) und der
/// Downloader schreibt in dieses Handle: er folgt keinem Pfad. Ein leer gebliebenes
/// Handle (Fehler vor dem ersten Byte) raeumt die Teildatei wieder ab.
async fn fetch_one(
    anchor: &Anchor,
    model_id: &str,
    file: &TtsCatalogFile,
    cancel: &CancellationToken,
    emit: &(dyn Fn(HttpDownloadEvent<'_>) + Send + Sync),
) -> Result<Fetch, String> {
    let partial = partial_name(&file.filename);
    let mut handle = anchor.open_partial(&partial).map_err(|e| io_msg(&e))?;
    let outcome = ModelManager::download_http_into_file(
        model_id,
        &file.url,
        &mut handle,
        Some(file.size_bytes),
        file.sha256.as_deref(),
        cancel,
        emit,
    )
    .await;
    let remove_if_empty = |handle: std::fs::File| {
        if handle.metadata().is_ok_and(|m| m.len() == 0) {
            drop(handle);
            let _ = anchor.remove(&partial);
        }
    };
    match outcome {
        Ok(HttpDownloadOutcome::Completed) => {
            // Der Name zeigt noch auf die Datei, in die geschrieben wurde.
            if !anchor.names(&partial, &handle).map_err(|e| io_msg(&e))? {
                return Err("unsafe_path".into());
            }
            drop(handle);
            anchor
                .rename_over(&partial, &file.filename)
                .map_err(|e| io_msg(&e))?;
            Ok(Fetch::Done)
        }
        Ok(HttpDownloadOutcome::Cancelled) => {
            remove_if_empty(handle);
            Ok(Fetch::Cancelled)
        }
        Err(e) => {
            remove_if_empty(handle);
            Err(net_msg(&e))
        }
    }
}

/// Stellt alle Dateien des Eintrags bereit: jede Datei wird vor dem Umbenennen ueber
/// das Ziel nach Groesse und SHA-256 geprueft. Der Erfolg ([`Fetch::Done`]) ist ein
/// Hinweis, keine Integritaetszusage fuer den Bestand danach (siehe Modulkopf).
///
/// 0. Die Kette von `root` bis zum Ordner der Ziele wird VOR dem ersten Netzzugriff
///    Komponente fuer Komponente ohne Link-Folgen geoeffnet (fehlende Ordner werden
///    angelegt) und als Handle bis zum Ende gehalten ([`Anchor`]); jede weitere
///    Dateioperation laeuft ueber dieses Handle (`unsafe_path`, wenn ein Link oder
///    eine Nicht-Datei im Weg ist).
/// 1. Jede vorhandene Datei wird nach Groesse und SHA-256 geprueft ([`verify_all`]).
///    Sind alle gueltig, ist nichts zu tun (kein Netz).
/// 2. Muss eine andere als die erste Datei ersetzt werden, wird die erste (die
///    der Motor oeffnet) zurueckgezogen: als `<ziel>.partial` abgelegt, ein
///    gueltiger Graph wird dadurch ohne Netz wieder uebernommen.
/// 3. Jede fehlende oder ungueltige Datei wird nach `<ziel>.partial` geladen
///    (fortsetzbar, Groesse und SHA-256 aus dem Katalog geprueft) und erst
///    dann ueber das Ziel umbenannt; die erste Datei kommt zuletzt.
/// 4. Zuletzt pruefen alle Dateien noch einmal ([`confirm`]); nur dann Erfolg, und
///    nur wenn der Ordner noch dort liegt, wohin der Pfad fuehrt. Das belegt die
///    Lesepruefungen dieses Versuchs, nicht den Bestand danach.
async fn provision(
    root: &Path,
    model_id: &str,
    targets: &[(TtsCatalogFile, PathBuf)],
    cancel: &CancellationToken,
    emit: &(dyn Fn(HttpDownloadEvent<'_>) + Send + Sync),
) -> Result<Fetch, String> {
    if targets.is_empty() {
        return Err("model_missing".into());
    }
    let parts = place(root, targets)?;
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let anchor = Anchor::open(root, &parts, true)
        .map_err(|e| io_msg(&e))?
        .ok_or_else(|| "model_missing".to_string())?;
    let anchor = Arc::new(anchor);

    let Some(mut states) = verify_all(&anchor, targets, cancel).await? else {
        return Ok(Fetch::Cancelled);
    };
    if states.iter().all(|s| *s == FileState::Valid) {
        return finish(&anchor);
    }
    // Die erste Datei oeffnet der Motor: sie darf nicht neben Dateien liegen, die
    // gleich ersetzt werden. Eine gueltige wandert als Teildatei beiseite (kommt
    // ohne Netz zurueck), eine ungueltige wird ohnehin neu geladen.
    let graph = &targets[0].0.filename;
    let others_broken = states[1..].iter().any(|s| *s != FileState::Valid);
    if others_broken && states[0] != FileState::Missing {
        withdraw_graph(&anchor, graph, Some(states[0]))?;
        states[0] = FileState::Missing;
    }
    for ((file, _), state) in targets.iter().zip(&states).rev() {
        if *state == FileState::Valid {
            continue;
        }
        if cancel.is_cancelled() {
            return Ok(Fetch::Cancelled);
        }
        if fetch_one(&anchor, model_id, file, cancel, emit).await? == Fetch::Cancelled {
            return Ok(Fetch::Cancelled);
        }
    }
    confirm(&anchor, targets, cancel).await
}

/// Stellt das Modell `id` bereit (alle seine Dateien), nur auf Knopfdruck oder
/// `--provision`: prueft jede vorhandene Datei nach Groesse und SHA-256, ersetzt
/// ungueltige und laedt fehlende (siehe [`provision`]). Fortschritt ueber
/// `model-download-progress`, Ende ueber `model-download-complete`. Wirkt ab der
/// naechsten Sprechertrennung.
pub async fn download_model(app: &AppHandle, id: &str) -> Result<(), String> {
    let entry = catalog_entry(id).ok_or_else(|| "model_missing".to_string())?;
    let root = models_root(app);
    if root.as_os_str().is_empty() {
        return Err("data_dir_missing".into());
    }
    let files = targets(&root, &entry)?;
    if files.is_empty() {
        return Err("model_missing".into());
    }
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Err("download_running".into());
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            *CANCEL.lock().unwrap_or_else(|e| e.into_inner()) = None;
            DOWNLOADING.store(false, Ordering::SeqCst);
        }
    }
    let _reset = Reset;
    let handle = app.clone();
    let emit = move |event: HttpDownloadEvent<'_>| {
        if let HttpDownloadEvent::Progress(p) = event {
            let _ = handle.emit("model-download-progress", p);
        }
    };
    let cancel = CancellationToken::new();
    *CANCEL.lock().unwrap_or_else(|e| e.into_inner()) = Some(cancel.clone());
    match provision(&root, &entry.id, &files, &cancel, &emit).await? {
        Fetch::Done => {
            let _ = app.emit("model-download-complete", entry.id.as_str());
            Ok(())
        }
        Fetch::Cancelled => Err("cancelled".into()),
    }
}

/// Laedt das Standardmodell (Sortformer), nur auf Knopfdruck (oder
/// `--provision`). Eine vorhandene Datei wird vorher nach Groesse und SHA-256
/// geprueft und bei Abweichung ersetzt. Fortschritt ueber
/// `model-download-progress`, Ende ueber `model-download-complete`. Wirkt ab der
/// naechsten Sprechertrennung (Stopp, Import, Neu-Transkription).
#[tauri::command]
#[specta::specta]
pub async fn meeting_diarization_model_download(app: AppHandle) -> Result<(), String> {
    download_model(&app, diarize::DEFAULT_MODEL_ID).await
}

/// Bricht einen laufenden Download ab; die Teildatei bleibt fuer die
/// Fortsetzung liegen. Ohne laufenden Download ohne Wirkung.
#[tauri::command]
#[specta::specta]
pub fn meeting_diarization_model_cancel() -> Result<(), String> {
    if let Some(token) = CANCEL.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        token.cancel();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Die Kennung, die der Knopf laedt, ist genau die, die die
    /// Sprechertrennung sucht, und sie steht mit Datei und Pruefsumme im Katalog.
    #[test]
    fn the_downloadable_entry_is_the_one_diarization_looks_for() {
        let entry = catalog_entry(diarize::DEFAULT_MODEL_ID).expect("Sortformer steht im Katalog");
        let file = entry.files.first().expect("eine Datei");
        assert!(file.sha256.is_some(), "Download braucht eine Pruefsumme");
        assert!(file.size_bytes > 0);
        let root = std::path::Path::new("models");
        assert_eq!(
            diarize::catalog_model_path(root, &entry.id),
            Some(diarize::resolve_model_path(root, None))
        );
    }

    /// Nemotron: beide Dateien in einem eigenen Ordner, der Graph (die Datei,
    /// die der Motor oeffnet) ist die erste, damit kommt er beim Laden zuletzt.
    #[test]
    fn the_nemotron_entry_has_two_targets_in_its_own_folder_graph_first() {
        let root = Path::new("models");
        let entry = catalog_entry(diarize::nemotron::MODEL_ID).expect("Nemotron im Katalog");
        let t = targets(root, &entry).unwrap();
        assert_eq!(t.len(), 2);
        let dir = root.join("diarization").join(&entry.id);
        assert_eq!(t[0].1, dir.join("model_quantized.onnx"));
        assert_eq!(t[1].1, dir.join("model_quantized.onnx_data"));
        assert_eq!(
            diarize::catalog_model_path(root, &entry.id).as_ref(),
            Some(&t[0].1)
        );
        // Sortformer bleibt, wo es war (Altbestand).
        let old = catalog_entry(diarize::DEFAULT_MODEL_ID).unwrap();
        let t = targets(root, &old).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].1.parent().unwrap(), root.join("diarization"));
    }

    // ---- Ladeweg gegen einen lokalen Server --------------------------------

    fn sha_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Kleiner HTTP-Server: antwortet je Pfad mit dem Inhalt oder 500, und
    /// merkt sich die Reihenfolge der Anfragen.
    struct Server {
        base: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    async fn serve(routes: Vec<(&'static str, Option<Vec<u8>>)>) -> Server {
        serve_with(routes, Arc::new(|_| {})).await
    }

    /// Wie [`serve`]; `on_request` laeuft bei jeder Anfrage, bevor geantwortet wird
    /// (Tests schauen darin in den Ordner).
    async fn serve_with(
        routes: Vec<(&'static str, Option<Vec<u8>>)>,
        on_request: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (routes, log, on_request) = (routes.clone(), log.clone(), on_request.clone());
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if sock.read_exact(&mut byte).await.is_err() {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    log.lock().unwrap().push(path.clone());
                    on_request(&path);
                    let body = routes
                        .iter()
                        .find(|(p, _)| *p == path)
                        .and_then(|(_, b)| b.clone());
                    let response = match body {
                        Some(b) => {
                            let mut r = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                b.len()
                            )
                            .into_bytes();
                            r.extend(b);
                            r
                        }
                        None => b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec(),
                    };
                    let _ = sock.write_all(&response).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Server { base, requests }
    }

    const GRAPH: &[u8] = b"graph-bytes";
    const WEIGHTS: &[u8] = b"weights-bytes-weights-bytes";

    /// Zwei Dateien wie bei Nemotron: Graph zuerst, Gewichte danach.
    fn two_files(server: &Server, weights_sha: Option<String>) -> Vec<(TtsCatalogFile, PathBuf)> {
        let dir = PathBuf::from("zielordner");
        let file = |name: &str, body: &[u8], sha: Option<String>| TtsCatalogFile {
            filename: name.to_string(),
            url: format!("{}/{name}", server.base),
            size_bytes: body.len() as u64,
            sha256: Some(sha.unwrap_or_else(|| sha_hex(body))),
        };
        vec![
            (file("model.onnx", GRAPH, None), dir.join("model.onnx")),
            (
                file("model.onnx_data", WEIGHTS, weights_sha),
                dir.join("model.onnx_data"),
            ),
        ]
    }

    fn rebase(t: Vec<(TtsCatalogFile, PathBuf)>, dir: &Path) -> Vec<(TtsCatalogFile, PathBuf)> {
        t.into_iter()
            .map(|(f, p)| {
                let dest = dir.join(p.file_name().unwrap());
                (f, dest)
            })
            .collect()
    }

    /// Der Anker auf einen schon vorhandenen Ordner (ohne Ordner anzulegen).
    fn anchor_of(dir: &Path) -> Arc<Anchor> {
        Arc::new(Anchor::open(dir, &[], false).unwrap().unwrap())
    }

    /// Die Teildatei neben der Datei `path`.
    fn partial_path(path: &Path) -> PathBuf {
        path.with_file_name(partial_name(path.file_name().unwrap().to_str().unwrap()))
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn both_files_arrive_graph_last_and_no_partial_is_left() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        assert!(!looks_complete(&files));
        let out = provision(
            dir.path(),
            "test/nemotron",
            &files,
            &CancellationToken::new(),
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(looks_complete(&files));
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
        // Die Gewichte zuerst, der Graph (der, den der Motor oeffnet) zuletzt.
        assert_eq!(
            *server.requests.lock().unwrap(),
            ["/model.onnx_data", "/model.onnx"]
        );
    }

    #[tokio::test]
    async fn a_second_run_asks_the_server_for_nothing_and_resumes_after_a_failed_file() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        let asked = server.requests.lock().unwrap().len();
        provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(
            server.requests.lock().unwrap().len(),
            asked,
            "Nachladen trotz vollstaendiger Dateien"
        );

        // Eine fehlende Datei wird einzeln nachgeladen, die vorhandene bleibt.
        std::fs::remove_file(&files[0].1).unwrap();
        provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(server.requests.lock().unwrap().len(), asked + 1);
        assert_eq!(
            *server.requests.lock().unwrap().last().unwrap(),
            "/model.onnx"
        );
        assert!(looks_complete(&files));
    }

    /// Faellt die Gewichte-Datei aus (Server 500), gibt es den Graphen nicht:
    /// das Modell gilt nicht als installiert, und der Motor bekommt nie einen
    /// Graphen ohne Gewichte.
    #[tokio::test]
    async fn a_failing_weights_download_leaves_no_graph_behind() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", None),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        let err = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap_err();
        assert!(!err.is_empty());
        assert!(!files[0].1.exists(), "Graph ohne Gewichte");
        assert!(!looks_complete(&files));
        assert!(!leftovers(dir.path()).iter().any(|n| n == "model.onnx_data"));
    }

    /// Falsche Pruefsumme: die Datei wird nicht umbenannt, der Graph nicht geladen.
    #[tokio::test]
    async fn a_checksum_mismatch_installs_nothing() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, Some("0".repeat(64))), dir.path());
        assert!(
            provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .is_err()
        );
        assert!(!files[0].1.exists() && !files[1].1.exists());
        assert!(!looks_complete(&files));
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx_data"]);
    }

    #[tokio::test]
    async fn cancelling_before_the_start_downloads_nothing() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        let cancel = CancellationToken::new();
        cancel.cancel();
        let out = provision(dir.path(), "t", &files, &cancel, &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Cancelled);
        assert!(server.requests.lock().unwrap().is_empty());
        assert!(!looks_complete(&files));
    }

    /// Eine abgebrochene Teildatei (Absturz mitten im Laden) bleibt als
    /// `.partial` liegen und zaehlt nicht als installiert; ein vollstaendig
    /// geschriebener, aber nie umbenannter Rest wird geprueft und uebernommen.
    #[tokio::test]
    async fn a_leftover_partial_is_not_an_installed_file_and_a_full_one_is_adopted() {
        let server = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        // Nach einem Absturz: Gewichte komplett als .partial (Rename fehlte), Graph nicht begonnen.
        std::fs::write(dir.path().join("model.onnx_data.partial"), WEIGHTS).unwrap();
        assert!(
            !looks_complete(&files),
            "Teildatei darf nicht als installiert gelten"
        );
        provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert!(looks_complete(&files));
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        // Die volle Teildatei wurde nur geprueft, nicht erneut geladen.
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx"]);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    // ---- Vorhandene Dateien: pruefen, ersetzen, nie Graph neben defekten Gewichten

    /// Die Dateien so, wie sie nach erfolgreichem Laden liegen.
    fn install(files: &[(TtsCatalogFile, PathBuf)]) {
        std::fs::write(&files[0].1, GRAPH).unwrap();
        std::fs::write(&files[1].1, WEIGHTS).unwrap();
    }

    fn both_routes() -> Vec<(&'static str, Option<Vec<u8>>)> {
        vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", Some(WEIGHTS.to_vec())),
        ]
    }

    /// Alles vorhanden und gueltig: kein Netzzugriff, nichts angefasst.
    #[tokio::test]
    async fn valid_existing_files_are_verified_and_nothing_is_downloaded() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        assert!(looks_complete(&files));
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(server.requests.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    /// Abgeschnittene Gewichte neben vollstaendigem Graph: die Statusabfrage
    /// sieht es an der Groesse, das Bereitstellen repariert nur die Gewichte, und
    /// waehrend sie geladen werden, liegt der Graph nicht am Ziel.
    #[tokio::test]
    async fn truncated_weights_next_to_a_complete_graph_are_repaired_without_exposing_the_graph() {
        let dir = tempfile::tempdir().unwrap();
        let graph = dir.path().join("model.onnx");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hook = {
            let seen = seen.clone();
            Arc::new(move |path: &str| {
                seen.lock()
                    .unwrap()
                    .push((path.to_string(), graph.exists()))
            })
        };
        let server = serve_with(both_routes(), hook).await;
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        std::fs::write(&files[1].1, &WEIGHTS[..5]).unwrap();
        assert!(
            !looks_complete(&files),
            "abgeschnitten gilt nicht als bereit"
        );

        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(looks_complete(&files));
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        // Nur die Gewichte aus dem Netz; der gueltige Graph kam aus der Rueckstellung.
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx_data"]);
        assert_eq!(
            *seen.lock().unwrap(),
            [("/model.onnx_data".to_string(), false)],
            "Graph lag waehrend des Ladens der Gewichte am Ziel"
        );
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    /// Graph fehlt, Gewichte haben die richtige Groesse, aber falsche Bytes (nur
    /// die SHA-256 sieht das): Gewichte ersetzt, dann der Graph.
    #[tokio::test]
    async fn corrupt_weights_of_the_right_size_are_replaced_then_the_graph_follows() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        std::fs::write(&files[1].1, vec![b'x'; WEIGHTS.len()]).unwrap();
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(
            *server.requests.lock().unwrap(),
            ["/model.onnx_data", "/model.onnx"]
        );
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    /// Der Vertrag zwischen Anzeige und Bereitstellen: die Anzeige schaut nur auf
    /// die Groesse (kein Hashlauf je Abfrage), das Bereitstellen auf den Inhalt.
    #[tokio::test]
    async fn the_status_checks_sizes_and_provisioning_checks_content() {
        assert!(!looks_complete(&[]), "ohne Dateien nie bereit");
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        std::fs::write(&files[1].1, vec![b'x'; WEIGHTS.len()]).unwrap();
        assert!(looks_complete(&files), "die Anzeige liest keine Inhalte");
        provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx_data"]);
    }

    /// Scheitert die Reparatur der Gewichte, bleibt der Graph zurueckgezogen
    /// (nicht am Ziel); die naechste Runde holt nur die Gewichte nach.
    #[tokio::test]
    async fn a_failed_repair_never_leaves_the_graph_next_to_defective_weights() {
        let dir = tempfile::tempdir().unwrap();
        let broken = serve(vec![
            ("/model.onnx", Some(GRAPH.to_vec())),
            ("/model.onnx_data", None),
        ])
        .await;
        let files = rebase(two_files(&broken, None), dir.path());
        install(&files);
        std::fs::write(&files[1].1, &WEIGHTS[..5]).unwrap();
        assert!(
            provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .is_err()
        );
        assert!(!files[0].1.exists(), "Graph neben defekten Gewichten");
        assert!(!looks_complete(&files));
        assert_eq!(
            std::fs::read(dir.path().join("model.onnx.partial")).unwrap(),
            GRAPH,
            "der gueltige Graph liegt als Teildatei bereit"
        );

        let healthy = serve(both_routes()).await;
        let files = rebase(two_files(&healthy, None), dir.path());
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert_eq!(*healthy.requests.lock().unwrap(), ["/model.onnx_data"]);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    /// Eine einzelne Datei (Sortformer): eine ungueltige wird ersetzt, aber erst
    /// wenn die neue verifiziert ist; scheitert das Laden, bleibt die alte liegen
    /// (nicht schlechter als vorher).
    #[tokio::test]
    async fn a_single_invalid_file_is_replaced_only_by_a_verified_download() {
        let dir = tempfile::tempdir().unwrap();
        let one = |server: &Server| rebase(two_files(server, None), dir.path()).remove(0);
        let down = serve(vec![("/model.onnx", None)]).await;
        let files = vec![one(&down)];
        std::fs::write(&files[0].1, b"alte-datei-bytes").unwrap();
        assert!(!looks_complete(&files));
        assert!(
            provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&files[0].1).unwrap(), b"alte-datei-bytes");

        let up = serve(both_routes()).await;
        let files = vec![one(&up)];
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert!(looks_complete(&files));
        assert_eq!(leftovers(dir.path()), ["model.onnx"]);
    }

    /// Abbruch vor dem Start laesst vorhandene Dateien unberuehrt.
    #[tokio::test]
    async fn cancelling_leaves_existing_files_untouched() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        std::fs::write(&files[1].1, &WEIGHTS[..5]).unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let out = provision(dir.path(), "t", &files, &cancel, &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Cancelled);
        assert!(server.requests.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), &WEIGHTS[..5]);
    }
    // ---- Ende nur nach bestandener Paarpruefung (ein Hinweis, kein Nachweis) ----

    /// Gewichte gleicher Groesse mit falschen Bytes: nur die SHA-256 sieht es.
    fn same_size_wrong() -> Vec<u8> {
        vec![b'x'; WEIGHTS.len()]
    }

    /// Hook, der bei der Anfrage `trigger` die Datei `victim` durch gleich grosse
    /// falsche Bytes ersetzt (ein Tausch mitten im Bereitstellen).
    fn swap_on(trigger: &'static str, victim: PathBuf) -> Arc<dyn Fn(&str) + Send + Sync> {
        Arc::new(move |path: &str| {
            if path == trigger {
                std::fs::write(&victim, same_size_wrong()).unwrap();
            }
        })
    }

    /// Gewichte gueltig, Graph fehlt; waehrend der Graph geladen wird, werden die
    /// Gewichte gegen gleich grosse falsche Bytes getauscht. Das ist kein Erfolg:
    /// Fehler, der Graph liegt nicht am Ziel (er wartet als verifizierte Teildatei),
    /// die Anzeige sagt "nicht bereit"; die naechste Runde holt nur die Gewichte.
    #[tokio::test]
    async fn a_swap_of_the_weights_while_the_graph_downloads_is_no_success() {
        let dir = tempfile::tempdir().unwrap();
        let hook = swap_on("/model.onnx", dir.path().join("model.onnx_data"));
        let server = serve_with(both_routes(), hook).await;
        let files = rebase(two_files(&server, None), dir.path());
        std::fs::write(&files[1].1, WEIGHTS).unwrap();

        let err = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap_err();
        assert_eq!(err, "verification_failed");
        assert_eq!(*server.requests.lock().unwrap(), ["/model.onnx"]);
        assert!(!files[0].1.exists(), "Graph neben getauschten Gewichten");
        assert_eq!(
            std::fs::read(&files[1].1).unwrap(),
            same_size_wrong(),
            "Testannahme: der Tausch hat stattgefunden"
        );
        assert!(
            !looks_complete(&files),
            "Anzeige: bereit trotz fehlendem Graph"
        );
        assert_eq!(
            std::fs::read(dir.path().join("model.onnx.partial")).unwrap(),
            GRAPH,
            "der verifizierte Graph wartet als Teildatei"
        );

        let healthy = serve(both_routes()).await;
        let files = rebase(two_files(&healthy, None), dir.path());
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert_eq!(*healthy.requests.lock().unwrap(), ["/model.onnx_data"]);
        assert_eq!(std::fs::read(&files[0].1).unwrap(), GRAPH);
        assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);
    }

    /// Dasselbe bei frischer Installation: die Gewichte liegen gerade erst
    /// verifiziert am Ziel, als sie waehrend der Graph-Anfrage getauscht werden.
    #[tokio::test]
    async fn a_swap_after_a_fresh_weights_install_is_caught_too() {
        let dir = tempfile::tempdir().unwrap();
        let hook = swap_on("/model.onnx", dir.path().join("model.onnx_data"));
        let server = serve_with(both_routes(), hook).await;
        let files = rebase(two_files(&server, None), dir.path());

        let err = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap_err();
        assert_eq!(err, "verification_failed");
        assert_eq!(
            *server.requests.lock().unwrap(),
            ["/model.onnx_data", "/model.onnx"]
        );
        assert!(!files[0].1.exists());
        assert!(!looks_complete(&files));
    }

    /// Die letzte Pruefung allein: alles gueltig ist Erfolg und fasst nichts an;
    /// sonst kein Erfolg, und der Graph ist je nach Befund zurueckgezogen
    /// (gueltig: als Teildatei, ungueltig: weg).
    #[tokio::test]
    async fn the_final_check_passes_only_a_verified_pair_and_withdraws_the_graph_otherwise() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        let none = CancellationToken::new();
        let anchor = anchor_of(dir.path());

        assert_eq!(confirm(&anchor, &files, &none).await.unwrap(), Fetch::Done);
        assert_eq!(leftovers(dir.path()), ["model.onnx", "model.onnx_data"]);

        // Gewichte getauscht, Graph gut: Graph als Teildatei beiseite.
        std::fs::write(&files[1].1, same_size_wrong()).unwrap();
        assert_eq!(
            confirm(&anchor, &files, &none).await.unwrap_err(),
            "verification_failed"
        );
        assert!(!files[0].1.exists());
        assert_eq!(
            std::fs::read(dir.path().join("model.onnx.partial")).unwrap(),
            GRAPH
        );
        assert_eq!(std::fs::read(&files[1].1).unwrap(), same_size_wrong());

        // Graph getauscht (gleich gross, falsche Bytes): weg, auch keine Teildatei.
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        std::fs::write(&files[0].1, vec![b'y'; GRAPH.len()]).unwrap();
        let anchor = anchor_of(dir.path());
        assert_eq!(
            confirm(&anchor, &files, &none).await.unwrap_err(),
            "verification_failed"
        );
        assert_eq!(leftovers(dir.path()), ["model.onnx_data"]);
    }

    /// Abbruch in der letzten Pruefung ist kein Erfolg und laesst keinen
    /// ungeprueften Graphen am Ziel liegen.
    #[tokio::test]
    async fn cancelling_the_final_check_is_no_success_and_withdraws_the_graph() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let files = rebase(two_files(&server, None), dir.path());
        install(&files);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let anchor = anchor_of(dir.path());
        assert_eq!(
            confirm(&anchor, &files, &cancel).await.unwrap(),
            Fetch::Cancelled
        );
        assert!(!files[0].1.exists());
        assert_eq!(
            std::fs::read(dir.path().join("model.onnx.partial")).unwrap(),
            GRAPH
        );
        assert!(!looks_complete(&files));
        // Die naechste Runde stellt ohne Netz wieder her.
        let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(server.requests.lock().unwrap().is_empty());
        assert!(looks_complete(&files));
    }

    /// Fehlende Ordner unterhalb der Wurzel legt das Bereitstellen an.
    #[tokio::test]
    async fn provisioning_creates_the_folders_below_the_root() {
        let server = serve(both_routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("models");
        let sub = root.join("diarization").join("id");
        let files = rebase(two_files(&server, None), &sub);
        let out = provision(&root, "t", &files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap();
        assert_eq!(out, Fetch::Done);
        assert!(looks_complete(&files));
        assert_eq!(leftovers(&sub), ["model.onnx", "model.onnx_data"]);
    }

    // ---- Katalogwerte, die aus dem Modellordner hinausfuehren -----------------

    #[test]
    fn targets_refuse_catalog_values_that_could_leave_the_model_folder() {
        let root = Path::new("models");
        let entry = catalog_entry(diarize::nemotron::MODEL_ID).unwrap();
        assert_eq!(targets(root, &entry).unwrap().len(), 2);
        let single = catalog_entry(diarize::DEFAULT_MODEL_ID).unwrap();
        assert_eq!(targets(root, &single).unwrap().len(), 1);
        for bad in ["..", "../x", "a/b", "/abs", "a\\b", "C:\\x", "C:", ""] {
            let mut by_id = entry.clone();
            by_id.id = bad.to_string();
            assert_eq!(
                targets(root, &by_id).unwrap_err(),
                "catalog_invalid",
                "{bad:?}"
            );
            let mut by_name = entry.clone();
            by_name.files[1].filename = bad.to_string();
            assert_eq!(
                targets(root, &by_name).unwrap_err(),
                "catalog_invalid",
                "{bad:?}"
            );
            let mut one = single.clone();
            one.files[0].filename = bad.to_string();
            assert_eq!(
                targets(root, &one).unwrap_err(),
                "catalog_invalid",
                "{bad:?}"
            );
        }
    }

    /// Ein Eintrag ohne Pruefsumme (oder mit einer, die keine SHA-256 ist) oder ohne
    /// Groesse wird weder bereitgestellt noch angezeigt: es gaebe nichts zu pruefen. Alle
    /// Eintraege des Katalogs erfuellen das.
    #[test]
    fn targets_refuse_files_without_a_checksum_or_a_size() {
        let root = Path::new("models");
        for entry in crate::catalog::tts_entries(crate::catalog::Purpose::Diarization) {
            assert!(targets(root, &entry).is_ok(), "{}", entry.id);
        }
        let entry = catalog_entry(diarize::nemotron::MODEL_ID).unwrap();
        for victim in 0..entry.files.len() {
            for bad in [
                None,
                Some(String::new()),
                Some("0".repeat(63)),
                Some("g".repeat(64)),
            ] {
                let mut e = entry.clone();
                e.files[victim].sha256 = bad.clone();
                assert_eq!(targets(root, &e).unwrap_err(), "catalog_invalid", "{bad:?}");
            }
            let mut e = entry.clone();
            e.files[victim].size_bytes = 0;
            assert_eq!(targets(root, &e).unwrap_err(), "catalog_invalid");
        }
    }

    // ---- Verknuepfungen: nur eigene, normale Dateien im Modellordner --------

    const FOREIGN: &[u8] = b"ABCDE";

    /// Prueft, dass `provision` mit `unsafe_path` ablehnt, ohne das Netz zu fragen.
    async fn assert_refused_without_network(
        root: &Path,
        files: &[(TtsCatalogFile, PathBuf)],
        server: &Server,
    ) {
        let err = provision(root, "t", files, &CancellationToken::new(), &|_| {})
            .await
            .unwrap_err();
        assert_eq!(err, "unsafe_path");
        assert!(
            server.requests.lock().unwrap().is_empty(),
            "Netzzugriff trotz Ablehnung"
        );
    }

    /// Jede Datei unter `dir` mit ihren Bytes, nach Pfad sortiert (Ordner als Eintrag
    /// ohne Bytes); Links werden nicht verfolgt, es gibt in diesen Baeumen keine.
    fn tree(dir: &Path) -> Vec<(String, Option<Vec<u8>>)> {
        fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Option<Vec<u8>>)>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if entry.file_type().unwrap().is_dir() {
                    out.push((rel, None));
                    walk(base, &path, out);
                } else {
                    out.push((rel, Some(std::fs::read(&path).unwrap())));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    mod links {
        use super::*;
        use crate::managers::meetings::diarize::nemotron::engine::OnnxBlockModel;
        use crate::managers::meetings::diarize::DiarizeError;
        use crate::managers::model::download::linktest::{
            folder_link, is_link, swap_must_have_happened, symlink_dir, symlink_file,
        };
        use std::sync::atomic::{AtomicBool, AtomicUsize};

        fn foreign(dir: &Path, bytes: &[u8]) -> PathBuf {
            let path = dir.join("fremd.bin");
            std::fs::write(&path, bytes).unwrap();
            path
        }

        /// Ein Ordner-Link in beiden Formen: ohne Rechte (Unix: Symlink, Windows:
        /// Junction) und, nur unter Windows, als echter Symlink (rot ohne Rechte, ausser
        /// `LVA_SKIP_SYMLINK_TESTS=1`). `false`: nicht angelegt (erlaubtes Ueberspringen).
        fn dir_link(real_symlink: bool, link: &Path, target: &Path) -> bool {
            if real_symlink {
                symlink_dir(link, target)
            } else {
                folder_link(link, target);
                true
            }
        }

        /// Die Formen, die dieses System kennt: unter Unix ist beides derselbe Symlink.
        fn link_forms() -> &'static [bool] {
            if cfg!(windows) {
                &[false, true]
            } else {
                &[false]
            }
        }

        /// Ein Link als Ziel wird abgelehnt, auch wenn dahinter genau die richtigen
        /// Bytes liegen: die fremde Datei wird weder als "gueltig" gelesen noch
        /// ersetzt, noch umbenannt, noch geloescht.
        #[tokio::test]
        async fn a_symlinked_target_is_refused_even_when_it_holds_the_right_bytes() {
            for victim in [1usize, 0] {
                let server = serve(both_routes()).await;
                let dir = tempfile::tempdir().unwrap();
                let outside = tempfile::tempdir().unwrap();
                let files = rebase(two_files(&server, None), dir.path());
                install(&files);
                let right = if victim == 1 { WEIGHTS } else { GRAPH };
                let target = foreign(outside.path(), right);
                std::fs::remove_file(&files[victim].1).unwrap();
                if !symlink_file(&files[victim].1, &target) {
                    return;
                }
                assert!(
                    !looks_complete(&files),
                    "Anzeige: bereit trotz Link an Stelle {victim}"
                );

                assert_refused_without_network(dir.path(), &files, &server).await;

                assert_eq!(
                    std::fs::read(&target).unwrap(),
                    right,
                    "fremde Datei veraendert"
                );
                assert!(is_link(&files[victim].1), "Link ersetzt");
                let other = 1 - victim;
                assert!(!is_link(&files[other].1) && files[other].1.is_file());
            }
        }

        /// Das Gegenstueck ohne Rechte: ein Ordner-Link (Windows: Junction) an der
        /// Stelle des Ziels. Auch er wird abgelehnt, der fremde Ordner bleibt leer.
        #[tokio::test]
        async fn a_folder_link_as_target_is_refused_and_the_foreign_folder_stays_as_it_is() {
            for victim in [1usize, 0] {
                let server = serve(both_routes()).await;
                let dir = tempfile::tempdir().unwrap();
                let outside = tempfile::tempdir().unwrap();
                let files = rebase(two_files(&server, None), dir.path());
                install(&files);
                std::fs::remove_file(&files[victim].1).unwrap();
                folder_link(&files[victim].1, outside.path());
                std::fs::write(outside.path().join("fremd.bin"), FOREIGN).unwrap();
                let before = tree(outside.path());

                assert_refused_without_network(dir.path(), &files, &server).await;

                assert_eq!(tree(outside.path()), before);
                assert!(is_link(&files[victim].1), "Link ersetzt");
            }
        }

        /// Ein Link als Teildatei: die fremde Datei dahinter (kuerzer als die
        /// Gewichte) bekommt keine Bytes angehaengt, das Netz wird nie gefragt.
        #[tokio::test]
        async fn a_symlinked_partial_is_refused_and_the_foreign_file_stays_byte_identical() {
            let server = serve(both_routes()).await;
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let files = rebase(two_files(&server, None), dir.path());
            let target = foreign(outside.path(), FOREIGN);
            let partial = partial_path(&files[1].1);
            if !symlink_file(&partial, &target) {
                return;
            }

            assert_refused_without_network(dir.path(), &files, &server).await;

            assert_eq!(std::fs::read(&target).unwrap(), FOREIGN);
            assert!(is_link(&partial));
            assert!(!files[0].1.exists() && !files[1].1.exists());
        }

        /// Ein Link ins Leere als Teildatei: hinter dem Link entsteht nichts.
        #[tokio::test]
        async fn a_dangling_symlinked_partial_creates_nothing() {
            let server = serve(both_routes()).await;
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let files = rebase(two_files(&server, None), dir.path());
            let behind = outside.path().join("noch-nicht-da.bin");
            if !symlink_file(&partial_path(&files[1].1), &behind) {
                return;
            }

            assert_refused_without_network(dir.path(), &files, &server).await;
            assert!(!behind.exists());
        }

        /// Ein Ordner-Link (Windows: Junction) als Teildatei, dahinter ein fremder
        /// Ordner mit einer Datei: nichts darin wird veraendert.
        #[tokio::test]
        async fn a_folder_link_as_partial_is_refused_and_the_foreign_folder_stays_as_it_is() {
            let server = serve(both_routes()).await;
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let files = rebase(two_files(&server, None), dir.path());
            std::fs::write(outside.path().join("fremd.bin"), FOREIGN).unwrap();
            let before = tree(outside.path());
            folder_link(&partial_path(&files[1].1), outside.path());

            assert_refused_without_network(dir.path(), &files, &server).await;

            assert_eq!(tree(outside.path()), before);
        }

        /// Ein Zwischenordner, der ein Link aus dem Modellordner heraus ist (Unix:
        /// Symlink; Windows: Junction und, mit Rechten, Symlink): es wird weder dahinter
        /// etwas angelegt noch geschrieben, und das Netz nicht gefragt.
        #[tokio::test]
        async fn a_linked_folder_pointing_outside_the_model_root_is_refused() {
            for &real_symlink in link_forms() {
                for leaf_only in [false, true] {
                    let server = serve(both_routes()).await;
                    let dir = tempfile::tempdir().unwrap();
                    let outside = tempfile::tempdir().unwrap();
                    let root = dir.path().join("models");
                    std::fs::create_dir_all(&root).unwrap();
                    let sub = root.join("diarization").join("id");
                    let made = if leaf_only {
                        std::fs::create_dir_all(root.join("diarization")).unwrap();
                        dir_link(real_symlink, &sub, outside.path())
                    } else {
                        dir_link(real_symlink, &root.join("diarization"), outside.path())
                    };
                    if !made {
                        return;
                    }
                    let files = rebase(two_files(&server, None), &sub);

                    assert_refused_without_network(&root, &files, &server).await;

                    assert!(
                        std::fs::read_dir(outside.path()).unwrap().next().is_none(),
                        "ausserhalb des Modellordners wurde etwas angelegt \
                         (leaf_only={leaf_only}, symlink={real_symlink})"
                    );
                }
            }
        }

        /// Gegenprobe: ein verschobener Datenordner (die Wurzel ist ein Link; unter
        /// Windows eine Junction) ist kein Verstoss, alles darunter liegt dann dort,
        /// wohin die Wurzel zeigt.
        #[tokio::test]
        async fn a_model_root_that_is_itself_a_link_still_works() {
            let server = serve(both_routes()).await;
            let dir = tempfile::tempdir().unwrap();
            let real = dir.path().join("platte-d");
            std::fs::create_dir_all(&real).unwrap();
            let root = dir.path().join("models");
            folder_link(&root, &real);
            let sub = root.join("diarization").join("id");
            let files = rebase(two_files(&server, None), &sub);

            let out = provision(&root, "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .unwrap();
            assert_eq!(out, Fetch::Done);
            assert!(looks_complete(&files));
            assert_eq!(
                std::fs::read(real.join("diarization").join("id").join("model.onnx")).unwrap(),
                GRAPH
            );
        }

        /// Ein Link als Gewichte-Ziel, der erst in der letzten Pruefung auffaellt
        /// (eingeschoben waehrend des Ladens): kein Erfolg, Graph zurueckgezogen,
        /// fremde Datei unberuehrt.
        #[tokio::test]
        async fn a_link_slipped_in_during_the_download_is_caught_by_the_final_check() {
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let target = foreign(outside.path(), WEIGHTS);
            let weights = dir.path().join("model.onnx_data");
            // Vorab pruefen, ob sich hier ein Symlink anlegen laesst (unter Windows
            // nur mit Rechten); im Haken waere ein Ueberspringen nicht moeglich.
            let probe = outside.path().join("probe");
            if !symlink_file(&probe, &target) {
                return;
            }
            std::fs::remove_file(&probe).unwrap();
            let hook: Arc<dyn Fn(&str) + Send + Sync> = {
                let (weights, target) = (weights.clone(), target.clone());
                Arc::new(move |path: &str| {
                    if path == "/model.onnx" {
                        std::fs::remove_file(&weights).unwrap();
                        assert!(symlink_file(&weights, &target));
                    }
                })
            };
            let server = serve_with(both_routes(), hook).await;
            let files = rebase(two_files(&server, None), dir.path());
            std::fs::write(&files[1].1, WEIGHTS).unwrap();

            let err = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .unwrap_err();
            assert_eq!(err, "unsafe_path");
            assert!(!files[0].1.exists(), "Graph neben einem Link als Gewichte");
            assert_eq!(std::fs::read(&target).unwrap(), WEIGHTS);
            assert!(is_link(&weights));
        }

        /// Aufbau fuer den Ordnertausch: unter `outside` liegt ein fremder Baum
        /// `fremd/id` mit einer Wachposten-Datei und fremden Teildateien, genau dort, wo
        /// das Bereitstellen sie aufnehmen wuerde, fuhre ein Pfad dorthin.
        fn foreign_tree(outside: &Path) -> PathBuf {
            let root = outside.join("fremd");
            let leaf = root.join("id");
            std::fs::create_dir_all(&leaf).unwrap();
            std::fs::write(leaf.join("fremd.bin"), FOREIGN).unwrap();
            std::fs::write(leaf.join("model.onnx_data.partial"), FOREIGN).unwrap();
            std::fs::write(leaf.join("model.onnx.partial"), FOREIGN).unwrap();
            root
        }

        /// Der Fehlerfall, um den es in dieser Runde geht: der Modellordner wird NACH
        /// der Ordnerpruefung und VOR den Dateioperationen (waehrend der erste Abruf
        /// laeuft) durch einen Link auf einen fremden Ordner ersetzt, der Wachposten und
        /// fremde Teildateien enthaelt. Alle Dateioperationen laufen ueber das geoeffnete
        /// Ordner-Handle: im fremden Baum entsteht, aendert, verschwindet nichts, und die
        /// Bereitstellung meldet keinen Erfolg (`unsafe_path`, der Ordner liegt nicht mehr
        /// dort, wohin der Pfad fuehrt). Unter Windows laesst sich der Ordner solange das
        /// Handle offen ist gar nicht ersetzen: dann gibt es den Tausch nicht, und die
        /// Bereitstellung gelingt im richtigen Ordner. Getauscht wird einmal der
        /// Modellordner selbst, einmal der Ordner davor; als Link und, unter Windows, auch
        /// als Junction.
        #[tokio::test]
        async fn replacing_the_model_folder_by_a_link_mid_download_touches_nothing_outside() {
            for &real_symlink in link_forms() {
                for swap_parent in [false, true] {
                    let dir = tempfile::tempdir().unwrap();
                    let outside = tempfile::tempdir().unwrap();
                    let foreign_root = foreign_tree(outside.path());
                    let before = tree(&foreign_root);

                    let root = dir.path().join("models");
                    let parent = root.join("diarization");
                    let sub = parent.join("id");
                    // Der Haken tauscht beim ersten Abruf; das Ordner-Handle liegt dann
                    // schon (der Anker wird vor dem ersten Netzzugriff geoeffnet).
                    let (victim, moved, link_to) = if swap_parent {
                        (
                            parent.clone(),
                            root.join("diarization.alt"),
                            foreign_root.clone(),
                        )
                    } else {
                        (sub.clone(), parent.join("id.alt"), foreign_root.join("id"))
                    };
                    let tried = Arc::new(AtomicBool::new(false));
                    let swapped = Arc::new(AtomicBool::new(false));
                    let hook: Arc<dyn Fn(&str) + Send + Sync> = {
                        let (tried, swapped) = (tried.clone(), swapped.clone());
                        let (victim, moved) = (victim.clone(), moved.clone());
                        Arc::new(move |_: &str| {
                            if tried.swap(true, Ordering::SeqCst) {
                                return;
                            }
                            if std::fs::rename(&victim, &moved).is_err() {
                                return;
                            }
                            if dir_link(real_symlink, &victim, &link_to) {
                                swapped.store(true, Ordering::SeqCst);
                            } else {
                                // Kein Link anlegbar (erlaubtes Ueberspringen): der Ordner
                                // kommt zurueck, sonst fehlte er im Nicht-getauscht-Zweig.
                                std::fs::rename(&moved, &victim)
                                    .expect("weg benannten Ordner zurueckbenennen");
                            }
                        })
                    };
                    let server = serve_with(both_routes(), hook).await;
                    let files = rebase(two_files(&server, None), &sub);
                    // Der Link, den der Haken anlegt, muss sich anlegen lassen (Windows:
                    // Symlink nur mit Rechten); vorab pruefen, im Haken geht kein Skip.
                    if !dir_link(real_symlink, &dir.path().join("probe-link"), outside.path()) {
                        return;
                    }

                    let out =
                        provision(&root, "t", &files, &CancellationToken::new(), &|_| {}).await;

                    assert_eq!(
                        tree(&foreign_root),
                        before,
                        "im fremden Baum wurde etwas angelegt, geaendert oder geloescht \
                         (parent={swap_parent}, symlink={real_symlink})"
                    );
                    if swapped.load(Ordering::SeqCst) {
                        assert_eq!(out.unwrap_err(), "unsafe_path");
                        let new_home = if swap_parent {
                            moved.join("id")
                        } else {
                            moved.clone()
                        };
                        // Die Arbeit landete im ALTEN Ordner (Handle), nicht im Link.
                        assert_eq!(std::fs::read(new_home.join("model.onnx")).unwrap(), GRAPH);
                        assert_eq!(
                            std::fs::read(new_home.join("model.onnx_data")).unwrap(),
                            WEIGHTS
                        );
                    } else {
                        swap_must_have_happened();
                        assert_eq!(out.unwrap(), Fetch::Done);
                        assert!(looks_complete(&files));
                    }
                }
            }
        }

        /// Zweiter Abruf: der Tausch geschieht erst, wenn die Gewichte schon liegen und
        /// der Graph geholt wird. Auch dann landet nichts im fremden Baum, und der
        /// Graph wird nicht im fremden Ordner abgelegt.
        #[tokio::test]
        async fn replacing_the_model_folder_during_the_second_fetch_touches_nothing_outside() {
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let foreign_root = foreign_tree(outside.path());
            let before = tree(&foreign_root);
            let root = dir.path().join("models");
            let sub = root.join("diarization").join("id");
            let moved = root.join("diarization").join("id.alt");
            let link_to = foreign_root.join("id");
            let swapped = Arc::new(AtomicBool::new(false));
            let hook: Arc<dyn Fn(&str) + Send + Sync> = {
                let (sub, moved, link_to, swapped) =
                    (sub.clone(), moved.clone(), link_to.clone(), swapped.clone());
                Arc::new(move |path: &str| {
                    if path == "/model.onnx" && std::fs::rename(&sub, &moved).is_ok() {
                        folder_link(&sub, &link_to);
                        swapped.store(true, Ordering::SeqCst);
                    }
                })
            };
            let server = serve_with(both_routes(), hook).await;
            let files = rebase(two_files(&server, None), &sub);

            let out = provision(&root, "t", &files, &CancellationToken::new(), &|_| {}).await;

            assert_eq!(tree(&foreign_root), before);
            if swapped.load(Ordering::SeqCst) {
                assert_eq!(out.unwrap_err(), "unsafe_path");
                assert_eq!(
                    std::fs::read(moved.join("model.onnx_data")).unwrap(),
                    WEIGHTS
                );
            } else {
                swap_must_have_happened();
                assert_eq!(out.unwrap(), Fetch::Done);
            }
        }

        /// Die Teildatei wird waehrend des Abrufs durch eine andere Datei ersetzt (das
        /// Schreibhandle bleibt beim urspruenglichen Inode, der Name zeigt auf die neue
        /// Datei), beim Abruf der Gewichte wie beim Abruf des Graphen. Das darf nie als
        /// Erfolg gelten, die Ersatzdatei bleibt Byte fuer Byte unveraendert (es wird nie
        /// in sie hineingeschrieben) und darf nicht unter dem Katalognamen landen.
        #[tokio::test]
        async fn replacing_the_partial_file_mid_download_is_no_success_and_leaves_the_replacement_untouched(
        ) {
            const REPLACEMENT: &[u8] = b"ersatz-statt-teildatei";
            for route in ["/model.onnx_data", "/model.onnx"] {
                let dir = tempfile::tempdir().unwrap();
                let root = dir.path().join("models");
                let sub = root.join("diarization").join("id");
                let name = route.trim_start_matches('/');
                let swapped = Arc::new(AtomicBool::new(false));
                let hook: Arc<dyn Fn(&str) + Send + Sync> = {
                    let (sub, swapped) = (sub.clone(), swapped.clone());
                    Arc::new(move |path: &str| {
                        if path != route {
                            return;
                        }
                        let spare = sub.join("ersatz.tmp");
                        let partial = partial_path(&sub.join(name));
                        if std::fs::write(&spare, REPLACEMENT).is_ok()
                            && std::fs::rename(&spare, &partial).is_ok()
                        {
                            swapped.store(true, Ordering::SeqCst);
                        }
                    })
                };
                let server = serve_with(both_routes(), hook).await;
                let files = rebase(two_files(&server, None), &sub);

                let out = provision(&root, "t", &files, &CancellationToken::new(), &|_| {}).await;

                if swapped.load(Ordering::SeqCst) {
                    assert_eq!(out.unwrap_err(), "unsafe_path", "{route}");
                    assert!(
                        !sub.join(name).exists(),
                        "die fremde Datei liegt unter dem Katalognamen ({route})"
                    );
                    assert_eq!(
                        std::fs::read(partial_path(&sub.join(name))).unwrap(),
                        REPLACEMENT,
                        "die fremde Datei wurde veraendert ({route})"
                    );
                } else {
                    swap_must_have_happened();
                    assert_eq!(out.unwrap(), Fetch::Done);
                }
            }
        }

        // ---- Datentraeger ohne Datei-Kennung ------------------------------------------

        /// Die Abbildung auf den Code der Oberflaeche: ein Datentraeger ohne Datei-Kennung
        /// heisst `volume_unsupported`, eine Link-Ablehnung `unsafe_path`, alles andere
        /// bleibt sein Text. Auf jedem System, ohne eingespielte Kennung.
        #[test]
        fn io_errors_map_to_their_codes() {
            let unsupported =
                std::io::Error::new(std::io::ErrorKind::Unsupported, safe::VolumeUnsupported);
            assert_eq!(io_msg(&unsupported), "volume_unsupported");
            let refused = std::io::Error::new(std::io::ErrorKind::PermissionDenied, safe::Refused);
            assert_eq!(io_msg(&refused), "unsafe_path");
            assert_eq!(io_msg(&std::io::Error::other("x")), "x");
            // Ein gewoehnlicher Unsupported-Fehler ist noch kein Datentraegerbefund.
            let plain = std::io::Error::from(std::io::ErrorKind::Unsupported);
            assert_ne!(io_msg(&plain), "volume_unsupported");
        }

        /// Liefert der Datentraeger keine Datei-Kennung (hier eingespielt), ist das
        /// Ergebnis `volume_unsupported` und nicht `unsafe_path`: es ist kein Link im Weg.
        /// Es geht kein Netzzugriff voraus und es entsteht keine Modelldatei. Mit Kennung
        /// gelingt dieselbe Bereitstellung danach (die Unbekanntheit haengt nur am Wachposten).
        #[cfg(unix)]
        #[tokio::test]
        async fn a_volume_without_file_ids_is_volume_unsupported_and_provisions_nothing() {
            use crate::managers::model::download::safe::inject::UnknownIdentities;
            let server = serve(both_routes()).await;
            let dir = tempfile::tempdir().unwrap();
            let files = rebase(two_files(&server, None), dir.path());
            let err = {
                let _unknown = UnknownIdentities::on();
                provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                    .await
                    .unwrap_err()
            };
            assert_eq!(err, "volume_unsupported");
            assert!(
                server.requests.lock().unwrap().is_empty(),
                "Netzzugriff trotz fehlender Kennung"
            );
            assert!(!files[0].1.exists() && !files[1].1.exists());

            let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .unwrap();
            assert_eq!(out, Fetch::Done);
        }

        // ---- Dateien mit mehreren Namen (Hardlink), auf jedem System ---------------

        /// Eine Teildatei, die ein Hardlink auf eine fremde Datei ist: Fortsetzung
        /// (fremde Datei kuerzer als die Gewichte: ein Range-Abruf waere der Fall) und
        /// frischer Abruf (leere fremde Datei) werden vor dem ersten Netzzugriff und vor
        /// jeder Aenderung abgelehnt; die fremde Datei bleibt Byte fuer Byte gleich.
        #[tokio::test]
        async fn a_hard_linked_partial_is_refused_for_resume_and_fresh_download() {
            for content in [FOREIGN, &b""[..]] {
                let server = serve(both_routes()).await;
                let dir = tempfile::tempdir().unwrap();
                let outside = tempfile::tempdir().unwrap();
                let files = rebase(two_files(&server, None), dir.path());
                let sentinel = foreign(outside.path(), content);
                std::fs::hard_link(&sentinel, partial_path(&files[1].1)).unwrap();

                assert_refused_without_network(dir.path(), &files, &server).await;

                assert_eq!(
                    std::fs::read(&sentinel).unwrap(),
                    content,
                    "fremde Datei veraendert (Laenge {})",
                    content.len()
                );
                assert!(!files[0].1.exists() && !files[1].1.exists());
            }
        }

        /// Dasselbe fuer die Teildatei des Graphen, die erst nach den Gewichten
        /// drankommt: die Gewichte liegen dann, der Graph nie, die fremde Datei bleibt.
        #[tokio::test]
        async fn a_hard_linked_graph_partial_is_refused_and_no_graph_appears() {
            let server = serve(both_routes()).await;
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let files = rebase(two_files(&server, None), dir.path());
            let sentinel = foreign(outside.path(), FOREIGN);
            std::fs::hard_link(&sentinel, partial_path(&files[0].1)).unwrap();

            let err = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                .await
                .unwrap_err();

            assert_eq!(err, "unsafe_path");
            assert_eq!(std::fs::read(&sentinel).unwrap(), FOREIGN);
            assert!(!files[0].1.exists(), "Graph trotz abgelehnter Teildatei");
            assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
        }

        /// Ein Ziel, das ein Hardlink auf eine fremde Datei ist: gleich gross mit
        /// richtigen Bytes ist es gueltig (nur gelesen), mit falschen Bytes wird der
        /// EINTRAG ersetzt (Umbenennen), nie in die Datei hineingeschrieben.
        #[tokio::test]
        async fn a_hard_linked_target_is_only_read_or_replaced_by_name() {
            for (content, expect_fetch) in [(WEIGHTS.to_vec(), false), (same_size_wrong(), true)] {
                let server = serve(both_routes()).await;
                let dir = tempfile::tempdir().unwrap();
                let outside = tempfile::tempdir().unwrap();
                let files = rebase(two_files(&server, None), dir.path());
                std::fs::write(&files[0].1, GRAPH).unwrap();
                let sentinel = foreign(outside.path(), &content);
                std::fs::hard_link(&sentinel, &files[1].1).unwrap();

                let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                    .await
                    .unwrap();

                assert_eq!(out, Fetch::Done);
                assert_eq!(
                    std::fs::read(&sentinel).unwrap(),
                    content,
                    "fremde Datei veraendert"
                );
                assert_eq!(std::fs::read(&files[1].1).unwrap(), WEIGHTS);
                assert_eq!(
                    server.requests.lock().unwrap().is_empty(),
                    !expect_fetch,
                    "Netzzugriff passt nicht zum Befund"
                );
            }
        }

        // ---- Die letzte Pruefung ist genau der Weg des Laders -----------------------

        /// Nach der Pruefung des Graphen (`Point::Checked(0)`) werden die noch
        /// UNGEPRUEFTEN Gewichte getauscht: einmal in die Datei hinein (gleich grosse
        /// falsche Bytes), einmal durch eine andere Datei unter demselben Namen. Beides
        /// ist kein Erfolg: der Graph liegt nicht am Ziel, die Anzeige sagt "nicht
        /// bereit". (Den bereits GEPRUEFTEN Graphen tauscht
        /// `an_in_place_overwrite_of_the_checked_graph_...`.) Unter Windows laesst sich
        /// eine geoeffnete Datei nicht tauschen; dann gibt es den Tausch nicht und die
        /// Bereitstellung gelingt.
        #[tokio::test]
        async fn a_same_size_swap_of_the_not_yet_checked_weights_is_no_success() {
            for by_rename in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let server = serve(both_routes()).await;
                let files = rebase(two_files(&server, None), dir.path());
                let weights = files[1].1.clone();
                let swapped = Arc::new(AtomicBool::new(false));
                let seen = Arc::new(AtomicUsize::new(0));
                let _guard = anchor::hook::set(dir.path(), {
                    let (swapped, seen) = (swapped.clone(), seen.clone());
                    move |point| {
                        if point != anchor::Point::Checked(0) {
                            return;
                        }
                        if seen.fetch_add(1, Ordering::SeqCst) != 0 {
                            return;
                        }
                        let ok = if by_rename {
                            let other = weights.with_extension("neu");
                            std::fs::write(&other, same_size_wrong()).is_ok()
                                && std::fs::rename(&other, &weights).is_ok()
                        } else {
                            std::fs::write(&weights, same_size_wrong()).is_ok()
                        };
                        swapped.store(ok, Ordering::SeqCst);
                    }
                });

                let out =
                    provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {}).await;

                if swapped.load(Ordering::SeqCst) {
                    assert_eq!(
                        out.unwrap_err(),
                        "verification_failed",
                        "rename={by_rename}"
                    );
                    assert!(!files[0].1.exists(), "Graph neben getauschten Gewichten");
                    assert!(!looks_complete(&files));
                    assert_eq!(
                        std::fs::read(partial_path(&files[0].1)).unwrap(),
                        GRAPH,
                        "der verifizierte Graph wartet als Teildatei"
                    );
                } else {
                    swap_must_have_happened();
                    assert_eq!(out.unwrap(), Fetch::Done);
                }
            }
        }

        /// Die Wanted-Liste des Laders fuer ein Paar aus Graph und Gewichten.
        fn loader_wanted(files: &[(TtsCatalogFile, PathBuf)]) -> [Wanted<'_>; 2] {
            fn one(target: &(TtsCatalogFile, PathBuf)) -> Wanted<'_> {
                Wanted {
                    name: &target.0.filename,
                    size: target.0.size_bytes,
                    sha256: target.0.sha256.as_deref(),
                }
            }
            [one(&files[0]), one(&files[1])]
        }

        /// Der Lader (Speicherweg) auf dem Ordner; die Fehlermeldung, wenn er ablehnt.
        /// Er lehnt vor dem Sitzungsbau ab (die Fake-Bytes sind kein ONNX), die Meldung
        /// nennt eine Datei, aber nie einen Pfad.
        fn loader_refusal(dir: &Path, files: &[(TtsCatalogFile, PathBuf)]) -> String {
            let anchor = anchor_of(dir);
            match OnnxBlockModel::load_verified(&anchor, loader_wanted(files), dir, 1) {
                Err(DiarizeError::Load(msg)) => {
                    assert!(
                        !msg.contains(dir.to_str().unwrap()),
                        "Pfad in der Meldung: {msg}"
                    );
                    msg
                }
                Err(other) => panic!("falscher Fehler: {other:?}"),
                Ok(_) => panic!("der Lader haette ablehnen muessen"),
            }
        }

        /// Beleg der Loader-Garantie: ein gueltiges Paar wird bereitgestellt (`Done`),
        /// danach wird eine der Dateien IN PLACE gleich gross ueberschrieben. Das merkt
        /// die Bereitstellung nicht mehr (sie ist vorbei), der naechste Lader aber lehnt
        /// ab, vor dem Sitzungsbau, per SHA-256 ueber die Bytes, die er selbst gelesen hat.
        #[tokio::test]
        async fn after_done_an_in_place_overwrite_is_refused_by_the_next_loader() {
            for victim in [0usize, 1] {
                let dir = tempfile::tempdir().unwrap();
                let server = serve(both_routes()).await;
                let files = rebase(two_files(&server, None), dir.path());

                let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                    .await
                    .unwrap();
                assert_eq!(out, Fetch::Done);
                assert!(looks_complete(&files));

                let same_size = vec![b'y'; files[victim].0.size_bytes as usize];
                std::fs::write(&files[victim].1, &same_size).unwrap();
                assert!(
                    looks_complete(&files),
                    "die Anzeige sieht nur die Groesse und bleibt auf bereit"
                );

                let msg = loader_refusal(dir.path(), &files);
                assert!(msg.contains("SHA-256"), "{msg}");
                assert!(msg.contains(&files[victim].0.filename), "{msg}");
            }
        }

        /// Dokumentiertes Verhalten der Bereitstellung, ehrlich festgehalten: die Datei
        /// wird IN PLACE (gleicher Inode, gleiche Groesse) ueberschrieben, NACHDEM die
        /// Pruefung sie gelesen hat (`Point::Checked(0)`, der Graph). Die Pruefung kann
        /// das nicht sehen (Handle und Name zeigen auf dieselbe Datei), die Bereitstellung
        /// meldet `Done`: ein Hinweis, keine Integritaetszusage. Der naechste Lader
        /// lehnt den Graphen ab. Einmal im Bestandsweg (alle Dateien liegen schon da, die
        /// Pruefung am Anfang), einmal in der Abschlusspruefung nach einer frischen
        /// Installation. Unter Windows scheitert das Schreiben (die Pruefung haelt die
        /// Datei mit `FILE_SHARE_READ` allein offen); dann gibt es die Aenderung nicht.
        #[tokio::test]
        async fn an_in_place_overwrite_of_the_checked_graph_is_done_here_but_refused_by_the_loader()
        {
            for existing in [true, false] {
                let dir = tempfile::tempdir().unwrap();
                let server = serve(both_routes()).await;
                let files = rebase(two_files(&server, None), dir.path());
                if existing {
                    install(&files);
                }
                let graph = files[0].1.clone();
                let changed = Arc::new(AtomicBool::new(false));
                let seen = Arc::new(AtomicUsize::new(0));
                let guard = anchor::hook::set(dir.path(), {
                    let (changed, seen) = (changed.clone(), seen.clone());
                    move |point| {
                        if point != anchor::Point::Checked(0) {
                            return;
                        }
                        if seen.fetch_add(1, Ordering::SeqCst) != 0 {
                            return;
                        }
                        let done = std::fs::write(&graph, vec![b'y'; GRAPH.len()]);
                        changed.store(done.is_ok(), Ordering::SeqCst);
                    }
                });

                let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                    .await
                    .unwrap();
                drop(guard);

                assert_eq!(out, Fetch::Done, "existing={existing}");
                assert_eq!(
                    seen.load(Ordering::SeqCst),
                    1,
                    "der Graph wird genau einmal gelesen (existing={existing})"
                );
                if changed.load(Ordering::SeqCst) {
                    assert_eq!(std::fs::read(&files[0].1).unwrap(), vec![b'y'; GRAPH.len()]);
                    let msg = loader_refusal(dir.path(), &files);
                    assert!(msg.contains("SHA-256"), "{msg}");
                    assert!(msg.contains("model.onnx"), "{msg}");
                } else {
                    swap_must_have_happened();
                }
            }
        }

        /// Der Erfolg folgt einer vollstaendigen Pruefung ueber Handles NACH dem letzten
        /// Abruf (alle Dateien offen, dann jede gelesen und gehasht), und die Pruefung der
        /// schon vorhandenen Dateien am Anfang geht denselben Weg: eine Pruefung, wenn
        /// alles vorhanden ist; bei frischer Installation eine leere am Anfang (nichts zu
        /// lesen) und eine volle am Ende.
        #[tokio::test]
        async fn success_follows_a_full_handle_check_after_the_last_download() {
            for existing in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let server = serve(both_routes()).await;
                let files = rebase(two_files(&server, None), dir.path());
                if existing {
                    install(&files);
                }
                let events = Arc::new(Mutex::new(Vec::<(anchor::Point, usize)>::new()));
                let _guard = anchor::hook::set(dir.path(), {
                    let (events, requests) = (events.clone(), server.requests.clone());
                    move |point| {
                        let asked = requests.lock().unwrap().len();
                        events.lock().unwrap().push((point, asked));
                    }
                });

                let out = provision(dir.path(), "t", &files, &CancellationToken::new(), &|_| {})
                    .await
                    .unwrap();

                assert_eq!(out, Fetch::Done);
                let events = events.lock().unwrap().clone();
                let total = server.requests.lock().unwrap().len();
                let opened = events
                    .iter()
                    .filter(|(p, _)| *p == anchor::Point::Opened)
                    .count();
                assert_eq!(opened, if existing { 1 } else { 2 }, "{events:?}");
                // Die letzte Pruefung beginnt und endet nach dem letzten Abruf und hat
                // beide Dateien gelesen.
                let last_open = events
                    .iter()
                    .rposition(|(p, _)| *p == anchor::Point::Opened)
                    .unwrap();
                let tail = &events[last_open..];
                assert!(tail.iter().all(|(_, asked)| *asked == total), "{events:?}");
                assert_eq!(
                    tail.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
                    [
                        anchor::Point::Opened,
                        anchor::Point::Checked(0),
                        anchor::Point::Checked(1)
                    ]
                );
            }
        }
    }
}
