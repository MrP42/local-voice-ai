//! Buch-Ordner-Ablage (lva#365, Paket P2a): Kernlogik ohne Tauri.
//!
//! Verbindlich ist die Spec `docs/superpowers/specs/2026-11-07-buch-ordner-und-projekte.md`,
//! Abschnitt 4. Bisher liegt jede Vorlesen-Seite unter `projects/<seite>/`, auch wenn sie
//! eine Fassung eines Buchs ist. Ab Layout 2 liegt die Seite einer Fassung unter
//! `library/<buch>/fassungen/<seite>/`; lose Texte bleiben unter `projects/`.
//!
//! Alles hier arbeitet allein über den Datenordner (`&Path`), damit es ohne App testbar ist.
//! Die Sperren (Seiten, Bibliothek, Sync-Lauf) hält der Aufrufer, siehe
//! `commands/library_layout.rs`.
//!
//! Bausteine:
//! - **Fundortkarte** (4.3): Seiten-ID → mögliche Orte, aus Bestandsaufnahme und `buch.json`.
//! - **Kandidaten-Auflösung** für `page_path`: alle Kandidaten prüfen, nie einen wählen,
//!   wenn mehrere existieren („Seite doppelt vorhanden“).
//! - **Journal** `library/.migration-buch-ordner.json`, atomar mit fsync geschrieben.
//! - **Sicherung** mit `manifest.json` (relativer Pfad, Größe, SHA-256), vollständig geprüft,
//!   bevor die erste Änderung geschieht.
//! - **Migration**, **Wiederanlauf** nach der Tabelle in 4.3 und **Rückfall** auf Wunsch.
//! - **Namensregeln** (4.4) und die Seiten, die ein Buch beim Gang in den Papierkorb mitnimmt.
//!
//! Ein vorhandenes Ziel wird nie überschrieben: Umbenennen und Kopieren prüfen das Ziel vorher
//! und brechen ab, statt zu ersetzen.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

/// Ordner der Vorlesen-Seiten (lose Texte und, vor Layout 2, auch Buchfassungen).
pub const PROJEKTE: &str = "projects";
/// Ordner der Bibliothek.
pub const BIBLIOTHEK: &str = "library";
/// Unterordner eines Buchs mit je einer Seite pro Fassung.
pub const FASSUNGEN: &str = "fassungen";
/// Seitenindex unter `projects/`.
pub const INDEX: &str = "index.json";
/// Metadaten eines Buchs.
pub const BUCH_JSON: &str = "buch.json";
/// Journal der Migration, liegt unter `library/`.
pub const JOURNAL: &str = ".migration-buch-ordner.json";
/// Kennzeichen des Layouts, liegt unter `library/`.
pub const KENNZEICHEN: &str = ".layout.json";
/// Ordner der Sicherungen im Datenordner.
pub const SICHERUNGEN: &str = "sicherungen";
/// Namensanfang einer Sicherung dieser Migration.
pub const SICHERUNG_PRAEFIX: &str = "buch-ordner-";
/// Inhaltsverzeichnis einer Sicherung.
pub const MANIFEST: &str = "manifest.json";
/// Papierkorb für Ziele, die beim Zurückspielen einer Sicherung weichen.
pub const PAPIERKORB: &str = ".papierkorb";
/// Stempel mit dem Zeitpunkt, zu dem etwas in den Papierkorb kam (wie in der Bibliothek).
const PAPIERKORB_STEMPEL: &str = ".entfernt";

const JOURNAL_VERSION: u32 = 1;
const MANIFEST_VERSION: u32 = 1;
/// Höchstlänge eines bereinigten Dateinamens in Zeichen (4.4).
pub const MAX_NAME: usize = 120;

// ---------------------------------------------------------------------------
// Kennungen und Pfade
// ---------------------------------------------------------------------------

/// Seiten-Kennung, wie `pages_create` sie vergibt: `page_` und ASCII-Buchstaben/Ziffern.
/// Dieselbe Regel wie `pages::checked_id`; sie entscheidet, ob ein Ordner eine Seite ist.
pub fn gueltige_seite(id: &str) -> bool {
    id.strip_prefix("page_")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// Buch-Kennung wie in `library::checked_id` (iPhone vergibt `book_<ms>_<hex>`).
pub fn gueltiges_buch(id: &str) -> bool {
    id.strip_prefix("book_").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 64
            && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Bisheriger Ort einer Seite, relativ zum Datenordner.
pub fn rel_von(seite: &str) -> String {
    format!("{PROJEKTE}/{seite}")
}

/// Ort einer Seite im Buch-Ordner, relativ zum Datenordner.
pub fn rel_nach(buch: &str, seite: &str) -> String {
    format!("{BIBLIOTHEK}/{buch}/{FASSUNGEN}/{seite}")
}

/// Relativen Pfad (mit `/`) unter einer Wurzel auflösen.
pub fn absolut(wurzel: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .filter(|teil| !teil.is_empty())
        .fold(wurzel.to_path_buf(), |pfad, teil| pfad.join(teil))
}

/// Pfad relativ zum Datenordner, mit `/` getrennt (für Meldungen, Journal und Manifest).
pub fn relativ(data: &Path, pfad: &Path) -> String {
    match pfad.strip_prefix(data) {
        Ok(rest) => rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => pfad.to_string_lossy().into_owned(),
    }
}

/// Ein relativer Pfad aus einer Datei (Manifest), der unter seiner Wurzel bleibt:
/// keine Wurzel, kein Laufwerk, kein `.`/`..`, kein Rückstrich.
fn sicherer_rel(rel: &str) -> Result<(), String> {
    let schlecht = rel.is_empty()
        || rel.starts_with('/')
        || rel.contains(['\\', ':', '\0'])
        || rel
            .split('/')
            .any(|teil| teil.is_empty() || teil == "." || teil == "..");
    if schlecht {
        return Err(format!("Unzulässiger Pfad in der Sicherung: {rel}"));
    }
    let nur_normale = Path::new(rel)
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if nur_normale {
        Ok(())
    } else {
        Err(format!("Unzulässiger Pfad in der Sicherung: {rel}"))
    }
}

pub fn jetzt_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Namen der echten Unterordner (keine Verknüpfungen), sortiert.
fn unterordner(dir: &Path) -> Vec<String> {
    let Ok(eintraege) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut namen: Vec<String> = eintraege
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    namen.sort();
    namen
}

/// Gibt es an diesem Pfad etwas (Ordner, Datei, auch eine Verknüpfung)?
fn vorhanden(pfad: &Path) -> bool {
    fs::symlink_metadata(pfad).is_ok()
}

// ---------------------------------------------------------------------------
// Atomar schreiben
// ---------------------------------------------------------------------------

/// Über eine Temp-Datei schreiben, die Daten mit fsync auf die Platte zwingen, dann
/// umbenennen und (unter Unix) den Ordner synchronisieren. Ein Absturz hinterlässt
/// die alte oder die neue Datei, nie eine halbe.
pub fn schreibe_atomar(pfad: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = pfad
        .parent()
        .ok_or_else(|| format!("Pfad ohne Ordner: {}", pfad.display()))?;
    fs::create_dir_all(dir).map_err(|e| format!("Ordner {} nicht anlegbar: {e}", dir.display()))?;
    let tmp = pfad.with_extension(format!("tmp-{}", std::process::id()));
    let geschrieben = (|| -> std::io::Result<()> {
        let mut datei = fs::File::create(&tmp)?;
        datei.write_all(bytes)?;
        datei.sync_all()
    })();
    if let Err(e) = geschrieben {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{} nicht schreibbar: {e}", pfad.display()));
    }
    if let Err(e) = fs::rename(&tmp, pfad) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{} nicht ersetzbar: {e}", pfad.display()));
    }
    ordner_sync(dir)?;
    Ok(())
}

/// Ordnereinträge (Umbenennen, Anlegen, Löschen) dauerhaft machen. Unter Unix per fsync
/// auf den Ordner, unter Windows per `FlushFileBuffers` auf ein Ordner-Handle (nur mit
/// `FILE_FLAG_BACKUP_SEMANTICS` zu öffnen).
///
/// `Ok(true)`: gesichert. `Ok(false)`: das Dateisystem kennt das nicht (manche Netz- und
/// Cloud-Laufwerke); das wird ohne Inhalt geloggt, und wer sich darauf stützen würde,
/// behält seine Rückfallkopien (der Papierkorb des Zurückspielens wird nie geleert).
/// Jeder andere Fehler, auch ein verweigerter Zugriff, ist ein Fehler.
fn ordner_sync(dir: &Path) -> Result<bool, String> {
    #[cfg(test)]
    if OHNE_SYNC.with(|o| o.get()) {
        return Ok(false);
    }
    let ergebnis = ordner_sync_roh(dir);
    match ergebnis {
        Ok(()) => Ok(true),
        Err(e) if sync_nicht_unterstuetzt(&e) => {
            log::warn!("Buch-Ordner: Ordner-Sync vom Dateisystem nicht unterstützt ({e})");
            Ok(false)
        }
        Err(e) => Err(format!(
            "{} ließ sich nicht dauerhaft sichern: {e}",
            dir.display()
        )),
    }
}

#[cfg(unix)]
fn ordner_sync_roh(dir: &Path) -> std::io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

#[cfg(windows)]
fn ordner_sync_roh(dir: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    fs::OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)?
        .sync_all()
}

#[cfg(not(any(unix, windows)))]
fn ordner_sync_roh(_dir: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Fehler, die nur sagen „dieses Dateisystem kann Ordner nicht synchronisieren“.
fn sync_nicht_unterstuetzt(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::Unsupported {
        return true;
    }
    #[cfg(unix)]
    {
        // EINVAL, ENOTSUP/EOPNOTSUPP (Linux 95, macOS 45 und 102)
        matches!(e.raw_os_error(), Some(22 | 95 | 45 | 102))
    }
    #[cfg(windows)]
    {
        // ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_INVALID_PARAMETER
        matches!(e.raw_os_error(), Some(1 | 50 | 87))
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

// ---------------------------------------------------------------------------
// Bibliothek lesen (nur, was die Ablage braucht)
// ---------------------------------------------------------------------------

/// Kennungen aller Buch-Ordner unter `library/` (der Papierkorb `.papierkorb` zählt nicht).
pub fn buecher(data: &Path) -> Vec<String> {
    unterordner(&data.join(BIBLIOTHEK))
        .into_iter()
        .filter(|id| gueltiges_buch(id))
        .collect()
}

/// Seiten eines Buchs laut `buch.json`: `text.page_id` und `editions[].text.page_id`.
/// Ungültige Kennungen werden übergangen; ein unlesbares `buch.json` liefert nichts.
pub fn buch_seiten(data: &Path, buch: &str) -> Vec<String> {
    if !gueltiges_buch(buch) {
        return Vec::new();
    }
    let pfad = data.join(BIBLIOTHEK).join(buch).join(BUCH_JSON);
    let Ok(roh) = fs::read_to_string(&pfad) else {
        return Vec::new();
    };
    let Ok(wert) = serde_json::from_str::<serde_json::Value>(&roh) else {
        return Vec::new();
    };
    let seite_von = |text: Option<&serde_json::Value>| -> Option<String> {
        text?
            .get("page_id")?
            .as_str()
            .filter(|id| gueltige_seite(id))
            .map(str::to_owned)
    };
    let mut seiten = BTreeSet::new();
    if let Some(id) = seite_von(wert.get("text")) {
        seiten.insert(id);
    }
    if let Some(fassungen) = wert.get("editions").and_then(|e| e.as_array()) {
        for fassung in fassungen {
            if let Some(id) = seite_von(fassung.get("text")) {
                seiten.insert(id);
            }
        }
    }
    seiten.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Fundortkarte
// ---------------------------------------------------------------------------

/// Seiten-ID → bekannte Fundorte (relativ zum Datenordner), unabhängig vom Index.
///
/// Zwei Quellen: die Bestandsaufnahme (`library/*/fassungen/*` und `projects/*`) und die
/// Buch-Metadaten (jede `page_id` in `text` und `editions[].text` ergibt den erwarteten Ort
/// `library/<buch>/fassungen/<seite>`). Die Karte ist ein Cache: ob ein Ort existiert, wird
/// bei jeder Auflösung auf der Platte geprüft.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Fundortkarte {
    orte: BTreeMap<String, BTreeSet<String>>,
}

impl Fundortkarte {
    pub fn aufbauen(data: &Path) -> Self {
        let mut karte = Self::default();
        for seite in unterordner(&data.join(PROJEKTE)) {
            if gueltige_seite(&seite) {
                let ort = rel_von(&seite);
                karte.eintragen(&seite, ort);
            }
        }
        for buch in buecher(data) {
            let fassungen = data.join(BIBLIOTHEK).join(&buch).join(FASSUNGEN);
            for seite in unterordner(&fassungen) {
                if gueltige_seite(&seite) {
                    let ort = rel_nach(&buch, &seite);
                    karte.eintragen(&seite, ort);
                }
            }
            for seite in buch_seiten(data, &buch) {
                let ort = rel_nach(&buch, &seite);
                karte.eintragen(&seite, ort);
            }
        }
        karte
    }

    fn eintragen(&mut self, seite: &str, ort: String) {
        self.orte.entry(seite.to_string()).or_default().insert(ort);
    }

    /// Bekannte Orte einer Seite (relativ), sortiert.
    pub fn orte(&self, seite: &str) -> Vec<String> {
        self.orte
            .get(seite)
            .map(|o| o.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Alle Seiten, die die Karte kennt.
    #[allow(dead_code)] // Oberfläche „Datei fehlt / Suchen …“ (P2b)
    pub fn seiten(&self) -> Vec<String> {
        self.orte.keys().cloned().collect()
    }
}

/// Alle Seitenordner auf der Platte samt Buch (für den Neuaufbau eines zerstörten Index):
/// `projects/<seite>` ohne Buch, `library/<buch>/fassungen/<seite>` mit Buch. Liegt eine
/// Seite an mehreren Orten, erscheint sie einmal; `page_path` meldet die Doppelung.
pub fn seiten_bestand(data: &Path) -> Vec<(String, Option<String>)> {
    let mut bestand: BTreeMap<String, Option<String>> = BTreeMap::new();
    for seite in unterordner(&data.join(PROJEKTE)) {
        if gueltige_seite(&seite) {
            bestand.entry(seite).or_insert(None);
        }
    }
    for buch in buecher(data) {
        for seite in unterordner(&data.join(BIBLIOTHEK).join(&buch).join(FASSUNGEN)) {
            if gueltige_seite(&seite) {
                bestand.entry(seite).or_insert_with(|| Some(buch.clone()));
            }
        }
    }
    bestand.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Journal
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Journal mit allen Schritten geschrieben, Sicherung noch nicht bestätigt.
    Geplant,
    /// Sicherung erstellt und geprüft; die Schritte laufen.
    Gesichert,
    /// Alle Schritte eingetragen, Kennzeichen `layout: 2` gesetzt.
    Fertig,
    /// Rückfall auf Wunsch läuft (dieselben Schritte rückwärts).
    Rueckfall,
    /// Rückfall abgeschlossen, Kennzeichen `layout: 1`.
    Zurueckgenommen,
}

impl Phase {
    /// Abgeschlossen heißt: `page_path` braucht das Journal nicht mehr.
    pub fn abgeschlossen(self) -> bool {
        matches!(self, Phase::Fertig | Phase::Zurueckgenommen)
    }

    pub fn name(self) -> &'static str {
        match self {
            Phase::Geplant => "geplant",
            Phase::Gesichert => "gesichert",
            Phase::Fertig => "fertig",
            Phase::Rueckfall => "rueckfall",
            Phase::Zurueckgenommen => "zurueckgenommen",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SchrittStatus {
    Offen,
    Verschoben,
    Eingetragen,
    /// Nur im Rückfall: die Seite liegt wieder unter `von`, der Index trägt noch das Buch.
    Zurueckverschoben,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Schritt {
    pub seite: String,
    pub buch: String,
    /// `projects/<seite>`
    pub von: String,
    /// `library/<buch>/fassungen/<seite>`
    pub nach: String,
    pub status: SchrittStatus,
}

impl Schritt {
    pub fn neu(seite: &str, buch: &str) -> Self {
        Self {
            seite: seite.to_string(),
            buch: buch.to_string(),
            von: rel_von(seite),
            nach: rel_nach(buch, seite),
            status: SchrittStatus::Offen,
        }
    }

    /// Ein Schritt aus der Datei ist nur gültig, wenn `von` und `nach` genau aus
    /// Seite und Buch folgen. Ein manipuliertes Journal verschiebt so nie etwas anderes.
    fn pruefen(&self) -> Result<(), String> {
        let stimmig = gueltige_seite(&self.seite)
            && gueltiges_buch(&self.buch)
            && self.von == rel_von(&self.seite)
            && self.nach == rel_nach(&self.buch, &self.seite);
        if stimmig {
            Ok(())
        } else {
            Err(format!(
                "Das Journal der Buch-Ordner-Migration enthält einen ungültigen Schritt ({})",
                self.seite
            ))
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Journal {
    pub version: u32,
    pub phase: Phase,
    /// Ordner der Sicherung relativ zum Datenordner, leer bis Phase `gesichert`.
    #[serde(default)]
    pub sicherung: String,
    pub schritte: Vec<Schritt>,
}

pub fn journal_pfad(data: &Path) -> PathBuf {
    data.join(BIBLIOTHEK).join(JOURNAL)
}

/// Journal lesen. Fehlt es, `None`. Ist es unlesbar oder enthält es ungültige Schritte,
/// ein Fehler: dann wird nichts verschoben.
pub fn journal_laden(data: &Path) -> Result<Option<Journal>, String> {
    let roh = match fs::read(journal_pfad(data)) {
        Ok(roh) => roh,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "Das Journal der Buch-Ordner-Migration ist nicht lesbar: {e}"
            ))
        }
    };
    let journal: Journal = serde_json::from_slice(&roh)
        .map_err(|e| format!("Das Journal der Buch-Ordner-Migration ist unlesbar: {e}"))?;
    if journal.version != JOURNAL_VERSION {
        return Err(format!(
            "Das Journal der Buch-Ordner-Migration hat eine unbekannte Version ({})",
            journal.version
        ));
    }
    for schritt in &journal.schritte {
        schritt.pruefen()?;
    }
    Ok(Some(journal))
}

/// Journal, das `page_path` berücksichtigen muss: vorhanden, lesbar, nicht abgeschlossen.
/// Ein unlesbares Journal fällt hier still weg; `von` und `nach` sind ohnehin Kandidaten
/// (`projects/<seite>` und die Fundortkarte aus `buch.json`).
pub fn aktives_journal(data: &Path) -> Option<Journal> {
    journal_laden(data)
        .ok()
        .flatten()
        .filter(|j| !j.phase.abgeschlossen())
}

fn journal_schreiben(data: &Path, journal: &Journal) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(journal).map_err(|e| e.to_string())?;
    schreibe_atomar(&journal_pfad(data), &bytes)
}

/// Ein abgeschlossenes Journal vor einem neuen Lauf als Nachweis beiseitelegen
/// (`.migration-buch-ordner-<ms>.json`), nie überschreiben.
fn journal_archivieren(data: &Path) -> Result<(), String> {
    let pfad = journal_pfad(data);
    if !vorhanden(&pfad) {
        return Ok(());
    }
    let mut ms = jetzt_ms();
    loop {
        let ziel = data
            .join(BIBLIOTHEK)
            .join(format!(".migration-buch-ordner-{ms}.json"));
        if !vorhanden(&ziel) {
            return fs::rename(&pfad, &ziel)
                .map_err(|e| format!("Altes Journal nicht archivierbar: {e}"));
        }
        ms += 1;
    }
}

// ---------------------------------------------------------------------------
// Kennzeichen
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Kennzeichen {
    layout: u32,
}

/// Layout des Datenordners: 2 nach erfolgreicher Migration, sonst 1.
pub fn layout(data: &Path) -> u32 {
    fs::read(data.join(BIBLIOTHEK).join(KENNZEICHEN))
        .ok()
        .and_then(|roh| serde_json::from_slice::<Kennzeichen>(&roh).ok())
        .map(|k| k.layout)
        .unwrap_or(1)
}

fn layout_setzen(data: &Path, wert: u32) -> Result<(), String> {
    let pfad = data.join(BIBLIOTHEK).join(KENNZEICHEN);
    if vorhanden(&pfad) && layout(data) == wert {
        return Ok(());
    }
    let bytes =
        serde_json::to_vec_pretty(&Kennzeichen { layout: wert }).map_err(|e| e.to_string())?;
    schreibe_atomar(&pfad, &bytes)
}

// ---------------------------------------------------------------------------
// Seitenindex (nur das Feld `buch`, alles andere bleibt unangetastet)
// ---------------------------------------------------------------------------

fn index_pfad(data: &Path) -> PathBuf {
    data.join(PROJEKTE).join(INDEX)
}

fn index_laden(data: &Path) -> Result<Option<serde_json::Value>, String> {
    let roh = match fs::read(index_pfad(data)) {
        Ok(roh) => roh,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Seitenindex nicht lesbar: {e}")),
    };
    serde_json::from_slice(&roh)
        .map(Some)
        .map_err(|e| format!("Seitenindex unlesbar: {e}"))
}

/// Feld `buch` des Index-Eintrags einer Seite (nur gültige Buch-Kennungen).
fn index_buch_in(index: &serde_json::Value, seite: &str) -> Option<String> {
    index
        .get("pages")?
        .as_array()?
        .iter()
        .find(|p| p.get("id").and_then(|i| i.as_str()) == Some(seite))?
        .get("buch")?
        .as_str()
        .filter(|b| gueltiges_buch(b))
        .map(str::to_owned)
}

/// Feld `buch` des Index-Eintrags lesen.
pub fn index_buch(data: &Path, seite: &str) -> Result<Option<String>, String> {
    Ok(index_laden(data)?.and_then(|i| index_buch_in(&i, seite)))
}

/// Feld `buch` setzen (`Some`) oder leeren (`None`) und den Index atomar schreiben.
/// Ohne Index oder ohne Eintrag für die Seite gibt es nichts zu tun (`false`).
fn index_buch_setzen(data: &Path, seite: &str, buch: Option<&str>) -> Result<bool, String> {
    let Some(mut index) = index_laden(data)? else {
        return Ok(false);
    };
    let Some(eintrag) = index
        .get_mut("pages")
        .and_then(|p| p.as_array_mut())
        .and_then(|p| {
            p.iter_mut()
                .find(|e| e.get("id").and_then(|i| i.as_str()) == Some(seite))
        })
        .and_then(|e| e.as_object_mut())
    else {
        return Ok(false);
    };
    let bisher = eintrag
        .get("buch")
        .and_then(|b| b.as_str())
        .map(str::to_owned);
    if bisher.as_deref() == buch {
        return Ok(true);
    }
    match buch {
        Some(b) => {
            eintrag.insert("buch".into(), serde_json::Value::String(b.to_string()));
        }
        None => {
            eintrag.remove("buch");
        }
    }
    let bytes = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
    schreibe_atomar(&index_pfad(data), &bytes)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Auflösung einer Seite (page_path)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aufloesung {
    /// Genau ein Kandidat existiert.
    Gefunden(PathBuf),
    /// Zwei oder mehr verschiedene Kandidaten existieren (Reihenfolge wie in 4.3).
    Doppelt(Vec<PathBuf>),
    /// Kein Kandidat existiert.
    Fehlt,
}

/// Ort laut Index-Eintrag: `buch` gesetzt → Buch-Ordner, sonst `projects/<seite>`.
fn index_ort(seite: &str, index_buch: Option<&str>) -> String {
    match index_buch.filter(|b| gueltiges_buch(b)) {
        Some(buch) => rel_nach(buch, seite),
        None => rel_von(seite),
    }
}

/// Alle verschiedenen Kandidaten in der Reihenfolge von 4.3 (die Reihenfolge dient nur
/// der Meldung): Index-Ort, Orte der Fundortkarte, Journal `nach` und `von`,
/// `projects/<seite>`. Gleiche Pfade zählen einmal.
pub fn kandidaten(
    data: &Path,
    seite: &str,
    index_buch: Option<&str>,
    karte: &Fundortkarte,
    journal: Option<&Journal>,
) -> Vec<PathBuf> {
    let mut rel: Vec<String> = vec![index_ort(seite, index_buch)];
    rel.extend(karte.orte(seite));
    if let Some(journal) = journal {
        for schritt in journal.schritte.iter().filter(|s| s.seite == seite) {
            rel.push(schritt.nach.clone());
            rel.push(schritt.von.clone());
        }
    }
    rel.push(rel_von(seite));
    let mut gesehen = BTreeSet::new();
    rel.into_iter()
        .filter(|r| gesehen.insert(r.clone()))
        .map(|r| absolut(data, &r))
        .collect()
}

/// Kandidaten auf Existenz prüfen; erst danach entscheiden.
pub fn aufloesen_mit(
    data: &Path,
    seite: &str,
    index_buch: Option<&str>,
    karte: &Fundortkarte,
    journal: Option<&Journal>,
) -> Aufloesung {
    let mut da: Vec<PathBuf> = kandidaten(data, seite, index_buch, karte, journal)
        .into_iter()
        .filter(|p| vorhanden(p))
        .collect();
    match da.len() {
        0 => Aufloesung::Fehlt,
        1 => Aufloesung::Gefunden(da.remove(0)),
        _ => Aufloesung::Doppelt(da),
    }
}

/// Auflösung wie `page_path`: Solange ein Journal ohne Abschluss existiert, wird es je
/// Aufruf geladen und seine Schritte zählen mit (Schritt 2 in 4.3).
pub fn aufloesen(
    data: &Path,
    seite: &str,
    index_buch: Option<&str>,
    karte: &Fundortkarte,
) -> Result<Aufloesung, String> {
    if !gueltige_seite(seite) {
        return Err(format!("Ungültige Seiten-Kennung: {seite}"));
    }
    let journal = aktives_journal(data);
    Ok(aufloesen_mit(
        data,
        seite,
        index_buch,
        karte,
        journal.as_ref(),
    ))
}

pub fn meldung_doppelt(data: &Path, seite: &str, pfade: &[PathBuf]) -> String {
    let liste: Vec<String> = pfade.iter().map(|p| relativ(data, p)).collect();
    format!("Seite doppelt vorhanden: {seite} ({})", liste.join(", "))
}

pub fn meldung_fehlt(seite: &str) -> String {
    format!("Seite nicht gefunden: {seite}")
}

/// Wie `page_path`: genau ein Ort oder ein Fehler mit allen Pfaden.
#[allow(dead_code)] // `pages::page_path` löst über `aufloesen` auf; das hier dient den Tests
pub fn seite_pfad(
    data: &Path,
    seite: &str,
    index_buch: Option<&str>,
    karte: &Fundortkarte,
) -> Result<PathBuf, String> {
    match aufloesen(data, seite, index_buch, karte)? {
        Aufloesung::Gefunden(pfad) => Ok(pfad),
        Aufloesung::Doppelt(pfade) => Err(meldung_doppelt(data, seite, &pfade)),
        Aufloesung::Fehlt => Err(meldung_fehlt(seite)),
    }
}

// ---------------------------------------------------------------------------
// Probleme und Berichte
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProblemArt {
    /// Zwei oder mehr Kandidaten existieren.
    Doppelt,
    /// Weder unter `von` noch unter `nach`: aus der Sicherung anbieten.
    Fehlt,
    /// Genau ein Kandidat, aber weder `von` noch `nach`.
    Unerwartet,
    /// Dateisystem- oder Lesefehler.
    Fehler,
}

impl ProblemArt {
    pub fn name(self) -> &'static str {
        match self {
            ProblemArt::Doppelt => "doppelt",
            ProblemArt::Fehlt => "fehlt",
            ProblemArt::Unerwartet => "unerwartet",
            ProblemArt::Fehler => "fehler",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub seite: String,
    pub art: ProblemArt,
    pub meldung: String,
    /// Betroffene Pfade relativ zum Datenordner.
    pub pfade: Vec<String>,
}

impl Problem {
    fn doppelt(data: &Path, seite: &str, pfade: &[PathBuf]) -> Self {
        Self {
            seite: seite.to_string(),
            art: ProblemArt::Doppelt,
            meldung: meldung_doppelt(data, seite, pfade),
            pfade: pfade.iter().map(|p| relativ(data, p)).collect(),
        }
    }

    fn fehlt(schritt: &Schritt, sicherung: &str) -> Self {
        let hinweis = if sicherung.is_empty() {
            String::new()
        } else {
            format!(" Sie kann aus der Sicherung {sicherung} zurückgeholt werden.")
        };
        Self {
            seite: schritt.seite.clone(),
            art: ProblemArt::Fehlt,
            meldung: format!(
                "Seite fehlt: {} liegt weder unter {} noch unter {}.{hinweis}",
                schritt.seite, schritt.von, schritt.nach
            ),
            pfade: vec![schritt.von.clone(), schritt.nach.clone()],
        }
    }

    fn unerwartet(data: &Path, schritt: &Schritt, ort: &Path) -> Self {
        let rel = relativ(data, ort);
        Self {
            seite: schritt.seite.clone(),
            art: ProblemArt::Unerwartet,
            meldung: format!(
                "Seite {} liegt an einem unerwarteten Ort ({rel}).",
                schritt.seite
            ),
            pfade: vec![rel],
        }
    }

    fn fehler(seite: &str, meldung: String) -> Self {
        Self {
            seite: seite.to_string(),
            art: ProblemArt::Fehler,
            meldung,
            pfade: Vec::new(),
        }
    }
}

/// Ergebnis eines Laufs (Migration, Wiederanlauf, Rückfall).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bericht {
    /// Phase des Journals nach dem Lauf (`None`: es gibt kein Journal).
    pub phase: Option<Phase>,
    /// Schritte, die in diesem Lauf etwas geändert haben.
    pub erledigt: usize,
    /// Angehalten: es wurde an dieser Stelle nichts weiter verschoben.
    pub angehalten: bool,
    pub probleme: Vec<Problem>,
    pub hinweise: Vec<String>,
}

impl Bericht {
    fn angehalten(phase: Phase, erledigt: usize, probleme: Vec<Problem>) -> Self {
        Self {
            phase: Some(phase),
            erledigt,
            angehalten: true,
            probleme,
            hinweise: Vec::new(),
        }
    }
}

/// Was die Migration braucht, aber nicht selbst bestimmen soll.
pub struct Optionen<'a> {
    /// Freier Platz auf dem Datenträger des Datenordners in Bytes (`None`: unbekannt).
    pub platz_frei: &'a dyn Fn(&Path) -> Option<u64>,
    /// Nur für Tests: wird zwischen Erstellen und Prüfen der Sicherung aufgerufen.
    pub nach_sicherung: Option<&'a dyn Fn(&Path)>,
}

// ---------------------------------------------------------------------------
// Sicherung
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManifestDatei {
    /// Pfad relativ zum Datenordner (und gleich relativ zum Sicherungsordner).
    pub pfad: String,
    pub groesse: u64,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: u32,
    pub erstellt_ms: u64,
    /// Seitenordner, die beim Zurückspielen auch leer wieder entstehen.
    #[serde(default)]
    pub ordner: Vec<String>,
    pub dateien: Vec<ManifestDatei>,
}

/// Dateien unter einem Ordner, relativ zum Datenordner. Verknüpfungen werden nicht verfolgt.
fn dateien_unter(data: &Path, dir: &Path, aus: &mut Vec<String>) -> Result<(), String> {
    let eintraege =
        fs::read_dir(dir).map_err(|e| format!("{} nicht lesbar: {e}", relativ(data, dir)))?;
    for eintrag in eintraege {
        let eintrag = eintrag.map_err(|e| e.to_string())?;
        let art = eintrag.file_type().map_err(|e| e.to_string())?;
        let pfad = eintrag.path();
        if art.is_dir() {
            dateien_unter(data, &pfad, aus)?;
        } else if art.is_file() {
            aus.push(relativ(data, &pfad));
        }
    }
    Ok(())
}

/// Umfang der Sicherung (4.2): jede betroffene Seite vollständig, `projects/index.json`,
/// jedes betroffene `buch.json`. Audio wird nicht bewegt und nicht gesichert.
fn sicherung_umfang(
    data: &Path,
    schritte: &[Schritt],
) -> Result<(Vec<String>, Vec<String>), String> {
    let mut dateien = Vec::new();
    let mut ordner = Vec::new();
    let mut buecher = BTreeSet::new();
    for schritt in schritte {
        buecher.insert(schritt.buch.clone());
        let von = absolut(data, &schritt.von);
        let nach = absolut(data, &schritt.nach);
        let ort = if von.is_dir() {
            von
        } else if nach.is_dir() {
            nach
        } else {
            continue;
        };
        ordner.push(relativ(data, &ort));
        dateien_unter(data, &ort, &mut dateien)?;
    }
    if index_pfad(data).is_file() {
        dateien.push(format!("{PROJEKTE}/{INDEX}"));
    }
    for buch in buecher {
        let rel = format!("{BIBLIOTHEK}/{buch}/{BUCH_JSON}");
        if absolut(data, &rel).is_file() {
            dateien.push(rel);
        }
    }
    dateien.sort();
    dateien.dedup();
    Ok((dateien, ordner))
}

fn summe_bytes(data: &Path, dateien: &[String]) -> u64 {
    dateien
        .iter()
        .filter_map(|rel| fs::metadata(absolut(data, rel)).ok())
        .map(|m| m.len())
        .sum()
}

/// Platzbedarf der Sicherung: Daten plus zehn Prozent und ein MiB Reserve.
pub fn platz_bedarf(bytes: u64) -> u64 {
    bytes + bytes / 10 + (1 << 20)
}

fn platz_pruefen(bytes: u64, frei: Option<u64>) -> Result<(), String> {
    let noetig = platz_bedarf(bytes);
    match frei {
        Some(frei) if frei < noetig => Err(format!(
            "Zu wenig freier Speicher für die Sicherung: nötig {} MB, frei {} MB. \
             Die Migration beginnt nicht.",
            noetig.div_ceil(1 << 20),
            frei / (1 << 20)
        )),
        _ => Ok(()),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Datei kopieren und dabei Größe und SHA-256 der Quelle bestimmen. Das Ziel darf nicht
/// existieren (`create_new`), es wird nie überschrieben; die Kopie wird mit fsync gesichert.
fn kopieren_mit_pruefsumme(von: &Path, nach: &Path) -> Result<(u64, String), String> {
    if let Some(dir) = nach.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Ordner nicht anlegbar: {e}"))?;
    }
    let mut quelle =
        fs::File::open(von).map_err(|e| format!("{} nicht lesbar: {e}", von.display()))?;
    let mut ziel = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(nach)
        .map_err(|e| format!("{} nicht anlegbar: {e}", nach.display()))?;
    // Ab hier gehört die Zieldatei diesem Aufruf: scheitert etwas, wird nur sie entfernt.
    let kopiert = (|| -> Result<(u64, String), String> {
        let mut hasher = Sha256::new();
        let mut puffer = vec![0u8; 64 * 1024];
        let mut groesse = 0u64;
        loop {
            let n = quelle
                .read(&mut puffer)
                .map_err(|e| format!("{} nicht lesbar: {e}", von.display()))?;
            if n == 0 {
                break;
            }
            hasher.update(&puffer[..n]);
            ziel.write_all(&puffer[..n])
                .map_err(|e| format!("{} nicht schreibbar: {e}", nach.display()))?;
            groesse += n as u64;
        }
        ziel.sync_all()
            .map_err(|e| format!("{} nicht schreibbar: {e}", nach.display()))?;
        Ok((groesse, hex(&hasher.finalize())))
    })();
    if kopiert.is_err() {
        drop(ziel);
        let _ = fs::remove_file(nach);
    }
    kopiert
}

fn pruefsumme(pfad: &Path) -> Result<(u64, String), String> {
    let mut datei = fs::File::open(pfad).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut puffer = vec![0u8; 64 * 1024];
    let mut groesse = 0u64;
    loop {
        let n = datei.read(&mut puffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&puffer[..n]);
        groesse += n as u64;
    }
    Ok((groesse, hex(&hasher.finalize())))
}

/// Neuen Sicherungsordner `sicherungen/buch-ordner-<ms>` anlegen (nie einen vorhandenen
/// wiederverwenden), die Dateien hineinkopieren und das Manifest schreiben. Scheitert
/// etwas, wird der eben angelegte Ordner wieder entfernt.
pub fn sicherung_erstellen(
    data: &Path,
    dateien: &[String],
    ordner: &[String],
    zeit_ms: u64,
) -> Result<PathBuf, String> {
    let basis = data.join(SICHERUNGEN);
    fs::create_dir_all(&basis).map_err(|e| format!("Sicherungsordner nicht anlegbar: {e}"))?;
    let mut ms = zeit_ms;
    let dir = loop {
        let dir = basis.join(format!("{SICHERUNG_PRAEFIX}{ms}"));
        match fs::create_dir(&dir) {
            Ok(()) => break dir,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ms += 1,
            Err(e) => return Err(format!("Sicherungsordner nicht anlegbar: {e}")),
        }
    };
    let erstellt = (|| -> Result<(), String> {
        let mut eintraege = Vec::with_capacity(dateien.len());
        for rel in dateien {
            sicherer_rel(rel)?;
            let (groesse, sha256) =
                kopieren_mit_pruefsumme(&absolut(data, rel), &absolut(&dir, rel))?;
            eintraege.push(ManifestDatei {
                pfad: rel.clone(),
                groesse,
                sha256,
            });
        }
        for rel in ordner {
            sicherer_rel(rel)?;
        }
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            erstellt_ms: zeit_ms,
            ordner: ordner.to_vec(),
            dateien: eintraege,
        };
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        schreibe_atomar(&dir.join(MANIFEST), &bytes)
    })();
    match erstellt {
        Ok(()) => Ok(dir),
        Err(e) => {
            let _ = fs::remove_dir_all(&dir);
            Err(format!("Sicherung nicht erstellt: {e}"))
        }
    }
}

/// Jede Datei der Sicherung gegen das Manifest prüfen: vorhanden, gleiche Größe,
/// gleiche SHA-256.
pub fn sicherung_pruefen(dir: &Path) -> Result<Manifest, String> {
    let roh =
        fs::read(dir.join(MANIFEST)).map_err(|e| format!("Manifest der Sicherung fehlt: {e}"))?;
    let manifest: Manifest = serde_json::from_slice(&roh)
        .map_err(|e| format!("Manifest der Sicherung unlesbar: {e}"))?;
    if manifest.version != MANIFEST_VERSION {
        return Err(format!(
            "Manifest der Sicherung hat eine unbekannte Version ({})",
            manifest.version
        ));
    }
    for rel in &manifest.ordner {
        sicherer_rel(rel)?;
    }
    for datei in &manifest.dateien {
        sicherer_rel(&datei.pfad)?;
        let pfad = absolut(dir, &datei.pfad);
        let (groesse, sha256) = pruefsumme(&pfad)
            .map_err(|_| format!("Sicherung unvollständig: {} fehlt", datei.pfad))?;
        if groesse != datei.groesse {
            return Err(format!(
                "Sicherung beschädigt: {} hat die falsche Größe",
                datei.pfad
            ));
        }
        if sha256 != datei.sha256 {
            return Err(format!(
                "Sicherung beschädigt: Prüfsumme von {} stimmt nicht",
                datei.pfad
            ));
        }
    }
    Ok(manifest)
}

/// Sicherung für die Schritte anlegen und prüfen. Erst danach darf sich etwas ändern.
fn sicherung_anlegen(
    data: &Path,
    schritte: &[Schritt],
    optionen: &Optionen,
) -> Result<String, String> {
    let (dateien, ordner) = sicherung_umfang(data, schritte)?;
    platz_pruefen(summe_bytes(data, &dateien), (optionen.platz_frei)(data))?;
    let dir = sicherung_erstellen(data, &dateien, &ordner, jetzt_ms())?;
    if let Some(eingriff) = optionen.nach_sicherung {
        eingriff(&dir);
    }
    if let Err(e) = sicherung_pruefen(&dir) {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!(
            "Die Sicherung ließ sich nicht bestätigen ({e}). Die Migration beginnt nicht."
        ));
    }
    Ok(relativ(data, &dir))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SicherungInfo {
    /// Ordnername unter `sicherungen/`.
    pub name: String,
    pub erstellt_ms: u64,
    pub dateien: usize,
    pub bytes: u64,
}

/// Sicherungen dieser Migration mit lesbarem Manifest, neueste zuerst. Die volle Prüfung
/// der Prüfsummen geschieht erst beim Zurückspielen.
pub fn sicherungen_auflisten(data: &Path) -> Vec<SicherungInfo> {
    let mut liste: Vec<SicherungInfo> = unterordner(&data.join(SICHERUNGEN))
        .into_iter()
        .filter(|name| name.starts_with(SICHERUNG_PRAEFIX))
        .filter_map(|name| {
            let roh = fs::read(data.join(SICHERUNGEN).join(&name).join(MANIFEST)).ok()?;
            let manifest: Manifest = serde_json::from_slice(&roh).ok()?;
            Some(SicherungInfo {
                name,
                erstellt_ms: manifest.erstellt_ms,
                dateien: manifest.dateien.len(),
                bytes: manifest.dateien.iter().map(|d| d.groesse).sum(),
            })
        })
        .collect();
    liste.sort_by_key(|s| std::cmp::Reverse(s.erstellt_ms));
    liste
}

/// Aufbewahrung einer Sicherung, bevor die Oberfläche nach dem Löschen fragt (4.2).
pub const AUFBEWAHRUNG_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// Eine Sicherung auf ausdrücklichen Wunsch löschen. Abgelehnt, solange ein Zurückspielen
/// unterbrochen ist oder eine offene Neuordnung (Journal ohne Abschluss) auf genau diese
/// Sicherung zählt; ebenso, wenn ihr Weg über eine Verknüpfung führt.
pub fn sicherung_loeschen(data: &Path, name: &str) -> Result<(), String> {
    if !ziffern_nach(name, SICHERUNG_PRAEFIX) {
        return Err(format!("Ungültiger Name einer Sicherung: {name}"));
    }
    let rel = format!("{SICHERUNGEN}/{name}");
    let dir = absolut(data, &rel);
    if !dir.is_dir() {
        return Err(format!("Sicherung nicht gefunden: {name}"));
    }
    if vorhanden(&tauschplan_pfad(data)) {
        return Err(
            "Ein Zurückspielen ist noch nicht abgeschlossen; bitte die App neu starten.".into(),
        );
    }
    let gebraucht =
        journal_laden(data)?.is_some_and(|j| !j.phase.abgeschlossen() && j.sicherung == rel);
    if gebraucht {
        return Err("Die laufende Neuordnung braucht diese Sicherung; sie bleibt erhalten.".into());
    }
    ohne_umweg(data, &rel)?;
    let wurzel = ordner_kennung(data)?;
    let eltern = ordner_kennung(&data.join(SICHERUNGEN))?;
    let selbst = ordner_kennung(&dir)?;
    let manifest = fs::read(dir.join(MANIFEST))
        .map_err(|e| format!("Manifest der Sicherung nicht lesbar: {e}; nichts gelöscht."))?;
    // Datenwurzel und `sicherungen` müssen dieselben Ordner bleiben, die eben gemessen
    // wurden; sonst könnte ein Pfad über eine neu entstandene Verknüpfung woandershin führen.
    let gleich = |ort: &Path, erwartet: Kennung| -> Result<(), String> {
        let wurzel_jetzt = ordner_kennung(data)?;
        let eltern_jetzt = ordner_kennung(&data.join(SICHERUNGEN))?;
        let ort_jetzt = ordner_kennung(ort)?;
        if wurzel_jetzt == wurzel && eltern_jetzt == eltern && ort_jetzt == erwartet {
            Ok(())
        } else {
            Err(format!(
                "{} ist nicht mehr der eben geprüfte Ordner; nichts gelöscht.",
                ort.display()
            ))
        }
    };
    // Erst in einen privaten Ordner mit nicht erratbarem Namen umbenennen (das Umbenennen
    // nimmt eine Verknüpfung selbst mit, nie ihr Ziel), dann am neuen Ort prüfen, eine
    // Löschmarke schreiben und erst danach löschen.
    let privat = privater_loeschort(data);
    gleich(&dir, selbst)?;
    #[cfg(test)]
    VOR_UMBENENNEN.with(|h| h.take().map(|f| f(data)));
    fs::rename(&dir, &privat).map_err(|e| format!("Sicherung nicht löschbar: {e}"))?;
    let geprueft = gleich(&privat, selbst).and_then(|()| {
        loeschbar(&privat)?;
        let jetzt = fs::read(privat.join(MANIFEST))
            .map_err(|e| format!("Manifest der Sicherung nicht lesbar: {e}; nichts gelöscht."))?;
        if jetzt != manifest {
            return Err("Die Sicherung hat sich beim Löschen verändert; nichts gelöscht.".into());
        }
        let name_privat = privat.file_name().and_then(|n| n.to_str()).unwrap_or("");
        fs::write(
            privat.join(LOESCHMARKE),
            loeschmarke(name_privat, &manifest),
        )
        .map_err(|e| format!("Löschmarke nicht schreibbar: {e}; nichts gelöscht."))?;
        gleich(&privat, selbst)
    });
    if let Err(e) = geprueft {
        // Zurück an den alten Namen, aber nur, wenn am privaten Namen noch genau die
        // gemessene Sicherung liegt. Sonst (etwa ein anderes Objekt, das ein Sync-Client
        // eingesetzt hat) wird nichts verändert und der Ort gemeldet.
        match ordner_kennung(&privat) {
            Ok(k) if k == selbst => {}
            Ok(_) => {
                log::error!(
                    "Buch-Ordner: abgebrochenes Löschen, privater Ort hat eine andere Kennung"
                );
                return Err(format!(
                    "{e} Am privaten Ort {} liegt nicht mehr die geprüfte Sicherung; dort wurde \
                     nichts verändert. Bitte den Ordner prüfen.",
                    privat.display()
                ));
            }
            Err(f) => {
                log::error!(
                    "Buch-Ordner: abgebrochenes Löschen, Kennung am privaten Ort nicht lesbar"
                );
                return Err(format!(
                    "{e} Die Kennung am privaten Ort ließ sich nicht prüfen ({f}); dort wurde \
                     nichts verändert."
                ));
            }
        }
        // Zurückbenennen, unmittelbar nach der Prüfung; die Marke erst danach im
        // zurückbenannten Ordner entfernen, und nur, wenn dort die Sicherung liegt.
        if let Err(zurueck) = fs::rename(&privat, &dir) {
            log::error!(
                "Buch-Ordner: Sicherung nach abgebrochenem Löschen nicht zurückbenannt ({zurueck})"
            );
            return Err(format!(
                "{e} Die Sicherung ließ sich nicht an ihren Platz zurückbringen ({zurueck}); \
                 sie liegt unter {}.",
                privat.display()
            ));
        }
        match ordner_kennung(&dir) {
            Ok(k) if k == selbst => {}
            Ok(_) => {
                log::error!("Buch-Ordner: zurückbenannter Ordner hat eine andere Kennung");
                return Err(format!(
                    "{e} Unter {} liegt nach dem Zurückbenennen nicht die geprüfte Sicherung; \
                     dort wurde nichts weiter verändert. Bitte den Ordner prüfen.",
                    dir.display()
                ));
            }
            Err(f) => {
                log::error!("Buch-Ordner: Kennung nach dem Zurückbenennen nicht lesbar");
                return Err(format!(
                    "{e} Nach dem Zurückbenennen ließ sich die Kennung nicht prüfen ({f}); \
                     dort wurde nichts weiter verändert."
                ));
            }
        }
        if let Err(marke) = fs::remove_file(dir.join(LOESCHMARKE)) {
            if marke.kind() != std::io::ErrorKind::NotFound {
                log::error!("Buch-Ordner: Löschmarke nicht entfernt ({marke})");
                return Err(format!(
                    "{e} Die Löschmarke in {} ließ sich nicht entfernen ({marke}).",
                    dir.display()
                ));
            }
        }
        return Err(e);
    }
    fs::remove_dir_all(&privat).map_err(|e| format!("Sicherung nicht löschbar: {e}"))?;
    reste_loeschen(data, wurzel, eltern);
    let _ = ordner_sync(&data.join(SICHERUNGEN))?;
    Ok(())
}

/// Präfix des privaten Ordners, in den eine Sicherung vor dem Löschen umbenannt wird.
const LOESCHEN_PRAEFIX: &str = ".loeschen-";
/// Löschmarke im privaten Ordner: nur ein Rest mit passender Marke wird später entfernt.
const LOESCHMARKE: &str = ".loeschen";

#[cfg(test)]
thread_local! {
    /// Test: Eingriff unmittelbar vor dem Umbenennen (Fall „Elternordner ausgetauscht“).
    static VOR_UMBENENNEN: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

/// Freier Name `sicherungen/.loeschen-<32 Hex>` mit zufälligem Teil (nicht angelegt: das
/// Umbenennen legt ihn an). Der Zufall stammt aus der zufällig geseedeten `RandomState`.
fn privater_loeschort(data: &Path) -> PathBuf {
    use std::hash::{BuildHasher, Hasher};
    let zufall = |runde: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(jetzt_ms());
        h.write_u64(runde);
        h.write_u32(std::process::id());
        h.finish()
    };
    let mut runde = 0;
    loop {
        let ort = data.join(SICHERUNGEN).join(format!(
            "{LOESCHEN_PRAEFIX}{:016x}{:016x}",
            zufall(runde),
            zufall(runde + 1)
        ));
        if !vorhanden(&ort) {
            return ort;
        }
        runde += 2;
    }
}

/// Inhalt der Löschmarke: bindet den privaten Namen an das Manifest der Sicherung.
fn loeschmarke(name: &str, manifest: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(name.as_bytes());
    h.update([0]);
    h.update(manifest);
    format!("{:x}", h.finalize())
}

/// Dateisystem-Kennung eines Ordners (Unix: Gerät und Inode; Windows: 64-Bit-Seriennummer
/// des Volumes und 128-Bit-FileId aus FILE_ID_INFO, gelesen am Eintrag selbst, ohne einer
/// Verknüpfung zu folgen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Kennung(u64, u128);

/// Warum keine Kennung ermittelt wurde: das Laufwerk taugt nicht (Netzlaufwerk, Dateisystem
/// ohne verlässliche Datei-Kennung) oder ein gewöhnlicher Ein-/Ausgabefehler.
#[derive(Debug)]
enum KennungFehler {
    #[cfg_attr(not(windows), allow(dead_code))]
    Laufwerk(String),
    Io(std::io::Error),
}

impl From<std::io::Error> for KennungFehler {
    fn from(e: std::io::Error) -> Self {
        KennungFehler::Io(e)
    }
}

fn laufwerk_meldung(grund: &str) -> String {
    format!(
        "Löschen von Sicherungen wird auf diesem Laufwerk nicht unterstützt (Netzlaufwerk \
         oder Dateisystem ohne verlässliche Datei-Kennung, etwa FAT/exFAT). Bitte die \
         Sicherung lokal löschen. (Ursache: {grund})"
    )
}

/// Kennung eines gewöhnlichen Ordners; eine Verknüpfung oder ein Nicht-Ordner ist ein Fehler.
/// Unter Windows wird das Laufwerk zuerst geprüft, damit auch ein Lesefehler auf einem
/// Netzlaufwerk den verständlichen Hinweis bekommt.
fn ordner_kennung(pfad: &Path) -> Result<Kennung, String> {
    #[cfg(windows)]
    if let Some(grund) = netzlaufwerk(pfad) {
        return Err(laufwerk_meldung(&grund));
    }
    let meta =
        fs::symlink_metadata(pfad).map_err(|e| format!("{} nicht lesbar: {e}", pfad.display()))?;
    if ist_umweg(&meta) || !meta.is_dir() {
        return Err(format!(
            "{} ist kein gewöhnlicher Ordner; nichts gelöscht.",
            pfad.display()
        ));
    }
    kennung_roh(pfad, &meta).map_err(|f| match f {
        KennungFehler::Laufwerk(grund) => laufwerk_meldung(&grund),
        KennungFehler::Io(e) => format!("{} nicht lesbar: {e}", pfad.display()),
    })
}

#[cfg(unix)]
fn kennung_roh(_pfad: &Path, meta: &fs::Metadata) -> Result<Kennung, KennungFehler> {
    use std::os::unix::fs::MetadataExt;
    Ok(Kennung(meta.dev(), u128::from(meta.ino())))
}

/// Unter Windows nur auf NTFS und ReFS: dort sind FileIds über ein Umbenennen stabil und
/// eindeutig. Auf FAT/exFAT oder Netzlaufwerken ohne verlässliche FileId wird das Löschen
/// einer Sicherung verweigert (dokumentierter Abbruch, nichts verändert).
#[cfg(windows)]
fn kennung_roh(pfad: &Path, _meta: &fs::Metadata) -> Result<Kennung, KennungFehler> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ID_INFO_KLASSE: i32 = 18;
    #[repr(C)]
    struct FileIdInfo {
        volume: u64,
        id: [u8; 16],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandleEx(
            datei: *mut std::ffi::c_void,
            klasse: i32,
            info: *mut std::ffi::c_void,
            groesse: u32,
        ) -> i32;
        fn GetVolumeInformationByHandleW(
            datei: *mut std::ffi::c_void,
            name: *mut u16,
            name_laenge: u32,
            seriennummer: *mut u32,
            max_laenge: *mut u32,
            merkmale: *mut u32,
            dateisystem: *mut u16,
            dateisystem_laenge: u32,
        ) -> i32;
    }
    const FILE_REMOTE_PROTOCOL_INFO_KLASSE: i32 = 13;
    let laufwerk =
        |was: &str| KennungFehler::Laufwerk(format!("{was}: {}", std::io::Error::last_os_error()));
    let datei = fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(pfad)?;
    let handle = datei.as_raw_handle();
    // Am Handle selbst: liefert FileRemoteProtocolInfo etwas, liegt der Ordner auf einem
    // Netz-Dateisystem, gleich über welche Pfadform (subst, verbundenes Laufwerk, UNC).
    let mut fernprotokoll = [0u32; 29];
    // SAFETY: gültiges Handle, Puffer in der Größe von FILE_REMOTE_PROTOCOL_INFO (116 Bytes).
    let fern = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FILE_REMOTE_PROTOCOL_INFO_KLASSE,
            fernprotokoll.as_mut_ptr().cast(),
            std::mem::size_of_val(&fernprotokoll) as u32,
        )
    };
    if fern != 0 {
        return Err(KennungFehler::Laufwerk(
            "Netz-Dateisystem (FileRemoteProtocolInfo)".into(),
        ));
    }
    let mut fs_name = [0u16; 32];
    // SAFETY: gültiges Handle; nur der Puffer für den Dateisystemnamen wird beschrieben.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            handle,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    if ok == 0 {
        return Err(laufwerk("Dateisystem nicht bestimmbar"));
    }
    let ende = fs_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(fs_name.len());
    let fs_name = String::from_utf16_lossy(&fs_name[..ende]);
    if !matches!(fs_name.as_str(), "NTFS" | "ReFS") {
        return Err(KennungFehler::Laufwerk(format!(
            "Dateisystem {fs_name} ohne verlässliche Datei-Kennung"
        )));
    }
    let mut info = FileIdInfo {
        volume: 0,
        id: [0; 16],
    };
    // SAFETY: gültiges Handle, Puffer in der Größe von FILE_ID_INFO.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FILE_ID_INFO_KLASSE,
            (&mut info as *mut FileIdInfo).cast(),
            std::mem::size_of::<FileIdInfo>() as u32,
        )
    };
    if ok == 0 {
        return Err(laufwerk("FILE_ID_INFO nicht lesbar"));
    }
    let id = u128::from_le_bytes(info.id);
    if id == 0 || info.volume == 0 {
        return Err(KennungFehler::Laufwerk(
            "Datei-Kennung oder Volume-Nummer ist 0".into(),
        ));
    }
    Ok(Kennung(info.volume, id))
}

/// Grund, warum dieser Pfad als Netzlaufwerk (oder unklare Pfadform) gilt. Der Pfad wird
/// erst aufgelöst (`canonicalize` löst `subst`, verbundene Laufwerke und relative Pfade auf;
/// scheitert das, gilt der absolute Pfad). UNC-Pfade und Laufwerke mit DRIVE_REMOTE sind
/// Netzlaufwerke; jede andere Pfadform als Laufwerksbuchstabe wird nicht unterstützt.
#[cfg(windows)]
fn netzlaufwerk(pfad: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Prefix;
    const DRIVE_REMOTE: u32 = 4;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDriveTypeW(wurzel: *const u16) -> u32;
    }
    // Ein UNC-Pfad wird ohne Zugriff erkannt (kein Warten auf einen fernen Server).
    if let Some(Component::Prefix(p)) = pfad.components().next() {
        if matches!(p.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..)) {
            return Some("UNC-Pfad (Netzfreigabe)".into());
        }
    }
    let voll = fs::canonicalize(pfad)
        .or_else(|_| std::path::absolute(pfad))
        .unwrap_or_else(|_| pfad.to_path_buf());
    match voll.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) => Some("UNC-Pfad (Netzfreigabe)".into()),
            Prefix::Disk(b) | Prefix::VerbatimDisk(b) => {
                let wurzel: Vec<u16> = std::ffi::OsStr::new(&format!("{}:\\", b as char))
                    .encode_wide()
                    .chain([0])
                    .collect();
                // SAFETY: nullterminierte Zeichenkette, nur gelesen.
                let typ = unsafe { GetDriveTypeW(wurzel.as_ptr()) };
                (typ == DRIVE_REMOTE).then(|| {
                    format!(
                        "Laufwerk {}: ist ein Netzlaufwerk (DRIVE_REMOTE)",
                        b as char
                    )
                })
            }
            andere => Some(format!("Pfadform {andere:?} wird nicht unterstützt")),
        },
        _ => Some("Pfad ohne Laufwerk wird nicht unterstützt".into()),
    }
}

#[cfg(not(any(unix, windows)))]
fn kennung_roh(_pfad: &Path, _meta: &fs::Metadata) -> Result<Kennung, KennungFehler> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported).into())
}

/// Ein gewöhnlicher Ordner, in dem auf keiner Ebene eine Verknüpfung (Symlink, Junction,
/// anderer Namensersatz-Reparse-Punkt) oder ein Sondereintrag liegt.
fn loeschbar(dir: &Path) -> Result<(), String> {
    let meta =
        fs::symlink_metadata(dir).map_err(|e| format!("{} nicht lesbar: {e}", dir.display()))?;
    if ist_umweg(&meta) || !meta.is_dir() {
        return Err(format!(
            "{} ist kein gewöhnlicher Ordner; nichts gelöscht.",
            dir.display()
        ));
    }
    let eintraege =
        fs::read_dir(dir).map_err(|e| format!("{} nicht lesbar: {e}", dir.display()))?;
    for eintrag in eintraege {
        let pfad = eintrag
            .map_err(|e| format!("{} nicht lesbar: {e}", dir.display()))?
            .path();
        let meta = fs::symlink_metadata(&pfad)
            .map_err(|e| format!("{} nicht lesbar: {e}", pfad.display()))?;
        if ist_umweg(&meta) {
            return Err(format!(
                "{} ist eine Verknüpfung; nichts gelöscht.",
                pfad.display()
            ));
        }
        if meta.is_dir() {
            loeschbar(&pfad)?;
        } else if !meta.is_file() {
            return Err(format!(
                "{} ist keine gewöhnliche Datei; nichts gelöscht.",
                pfad.display()
            ));
        }
    }
    Ok(())
}

/// Reste eines abgebrochenen Löschens (Absturz zwischen Umbenennen und Löschen) entfernen,
/// aber nur mit passender Löschmarke, ohne Verknüpfung darin und solange Datenwurzel und
/// `sicherungen` die eben gemessenen Ordner sind. Alles andere bleibt liegen.
fn reste_loeschen(data: &Path, wurzel: Kennung, eltern: Kennung) {
    for name in unterordner(&data.join(SICHERUNGEN)) {
        let hex = name.strip_prefix(LOESCHEN_PRAEFIX).unwrap_or("");
        if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let ort = data.join(SICHERUNGEN).join(&name);
        // Kennung vor der Inhaltsprüfung erfassen, unmittelbar vor dem Löschen vergleichen.
        let Ok(rest) = ordner_kennung(&ort) else {
            continue;
        };
        let marke_passt = match (
            fs::read(ort.join(LOESCHMARKE)),
            fs::read(ort.join(MANIFEST)),
        ) {
            (Ok(marke), Ok(manifest)) => marke == loeschmarke(&name, &manifest).as_bytes(),
            _ => false,
        };
        if !marke_passt || loeschbar(&ort).is_err() {
            continue;
        }
        let am_platz = ordner_kennung(data).is_ok_and(|k| k == wurzel)
            && ordner_kennung(&data.join(SICHERUNGEN)).is_ok_and(|k| k == eltern)
            && ordner_kennung(&ort).is_ok_and(|k| k == rest);
        if am_platz {
            if let Err(e) = fs::remove_dir_all(&ort) {
                log::warn!("Buch-Ordner: Rest eines Löschens nicht entfernt ({e})");
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wiederherstellung {
    /// Zurückgeschriebene Dateien.
    pub dateien: usize,
    /// Papierkorb mit den Zielen, die vorher weichen mussten (relativ), falls es welche gab.
    pub papierkorb: Option<String>,
}

/// Einheit, die beim Zurückspielen als Ganzes weicht: der Seitenordner oder die Datei.
fn einheit(rel: &str) -> Result<String, String> {
    let teile: Vec<&str> = rel.split('/').collect();
    match teile.as_slice() {
        [PROJEKTE, INDEX] => Ok(rel.to_string()),
        [PROJEKTE, seite, ..] if gueltige_seite(seite) => Ok(rel_von(seite)),
        [BIBLIOTHEK, buch, BUCH_JSON] if gueltiges_buch(buch) => Ok(rel.to_string()),
        [BIBLIOTHEK, buch, FASSUNGEN, seite, ..]
            if gueltiges_buch(buch) && gueltige_seite(seite) =>
        {
            Ok(rel_nach(buch, seite))
        }
        _ => Err(format!("Sicherung enthält einen unzulässigen Pfad: {rel}")),
    }
}

/// Seite, die eine Einheit der Sicherung trägt, samt Buch (Buch-Ordner) oder `None`
/// (`projects/<seite>`). Index und `buch.json` tragen keine Seite.
fn seite_der_einheit(einheit: &str) -> Option<(String, Option<String>)> {
    let teile: Vec<&str> = einheit.split('/').collect();
    match teile.as_slice() {
        [PROJEKTE, seite] if gueltige_seite(seite) => Some((seite.to_string(), None)),
        [BIBLIOTHEK, buch, FASSUNGEN, seite] => Some((seite.to_string(), Some(buch.to_string()))),
        _ => None,
    }
}

/// Einheiten, die eine Datei sind (Index, `buch.json`): sie werden ersetzt, ohne je zu
/// fehlen (Kopie in den Papierkorb, dann Umbenennen über das Ziel).
fn datei_einheit(einheit: &str) -> bool {
    einheit.ends_with(".json")
}

/// Neuen Ordner `<basis>/<praefix><ms>` anlegen, nie einen vorhandenen wiederverwenden.
fn neuer_ordner(basis: &Path, praefix: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(basis).map_err(|e| format!("{} nicht anlegbar: {e}", basis.display()))?;
    let mut ms = jetzt_ms();
    loop {
        let dir = basis.join(format!("{praefix}{ms}"));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ms += 1,
            Err(e) => return Err(format!("{} nicht anlegbar: {e}", dir.display())),
        }
    }
}

/// Seitenindex nach dem Zurückspielen: der aktuelle Index bleibt die Grundlage (Seiten, die
/// nach der Sicherung entstanden sind, bleiben drin, sonst hielte der Sync sie für gelöscht).
/// Nur die Einträge der Seiten aus der Sicherung kommen aus der Sicherung; ihr Feld `buch`
/// folgt dem Ort, an dem die Seite danach liegt. Ohne aktuellen Index gilt der gesicherte.
fn index_zusammenfuehren(
    aktuell: Option<serde_json::Value>,
    gesichert: Option<serde_json::Value>,
    seiten: &BTreeMap<String, Option<String>>,
) -> Option<serde_json::Value> {
    let eintraege = |v: &Option<serde_json::Value>| -> Vec<serde_json::Value> {
        v.as_ref()
            .and_then(|v| v.get("pages"))
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default()
    };
    let id_von = |e: &serde_json::Value| e.get("id").and_then(|i| i.as_str()).map(str::to_owned);
    let gesichert_liste = eintraege(&gesichert);
    let mut basis = match aktuell.or(gesichert) {
        Some(v) => v,
        None if seiten.is_empty() => return None,
        None => serde_json::json!({ "pages": [] }),
    };
    if !basis.is_object() {
        basis = serde_json::json!({ "pages": [] });
    }
    if !basis.get("pages").is_some_and(|p| p.is_array()) {
        basis["pages"] = serde_json::Value::Array(Vec::new());
    }
    let liste = basis["pages"].as_array_mut().expect("pages ist ein Array");
    for (seite, buch) in seiten {
        let aus_sicherung = gesichert_liste
            .iter()
            .find(|e| id_von(e).as_deref() == Some(seite.as_str()))
            .cloned();
        let pos = liste
            .iter()
            .position(|e| id_von(e).as_deref() == Some(seite.as_str()));
        let mut eintrag = match (aus_sicherung, pos) {
            (Some(e), _) => e,
            (None, Some(p)) => liste[p].clone(),
            (None, None) => serde_json::json!({ "id": seite, "title": seite }),
        };
        if let Some(obj) = eintrag.as_object_mut() {
            match buch {
                Some(b) => {
                    obj.insert("buch".into(), serde_json::Value::String(b.clone()));
                }
                None => {
                    obj.remove("buch");
                }
            }
        }
        match pos {
            Some(p) => liste[p] = eintrag,
            None => liste.push(eintrag),
        }
    }
    Some(basis)
}

/// Plan des Tauschs beim Zurückspielen. Er liegt dauerhaft unter
/// `sicherungen/.zurueckspielen.json`, bevor sich etwas ändert, und hält fest, wie weit
/// der Tausch ist ([`TauschStand`]). Findet der Start einen Plan vor, wurde das
/// Zurückspielen unterbrochen: [`zurueckspielen_wiederanlauf`] nimmt es zurück oder räumt
/// nur noch auf, noch bevor Bibliothek und Sync lesen.
///
/// Jeder Ordner ist an seinen [`Abdruck`] gebunden: verschoben oder gelöscht wird nur, was
/// genau so aussieht wie geplant. Ein veränderter oder untergeschobener Plan kann so nie
/// fremde Daten in den Zwischenordner (und damit ins Aufräumen) ziehen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Tauschplan {
    pub version: u32,
    #[serde(default)]
    pub stand: TauschStand,
    /// Die Sicherung (`buch-ordner-<ms>`), aus der zurückgespielt wird. Alles, was
    /// eingesetzt wird, muss laut ihrem Manifest daraus stammen.
    pub sicherung: String,
    /// Zwischenordner `sicherungen/.zurueck-<ms>`.
    pub stufe: String,
    /// Papierkorb `sicherungen/.papierkorb/<ms>`, falls etwas weichen muss.
    pub korb: Option<String>,
    /// Seitenordner, die vom Datenordner in den Papierkorb gehen.
    pub weichen: Vec<PlanOrdner>,
    /// Seitenordner, die aus dem Zwischenordner an ihren Ort gehen.
    pub hin: Vec<PlanOrdner>,
    /// Dateien (Index zuletzt), die über ihr Ziel umbenannt werden.
    pub dateien: Vec<PlanDatei>,
}

/// Wie weit der Tausch ist. Jeder Wechsel steht auf der Platte, bevor aufgeräumt wird:
/// fehlende Zwischenstände dürfen nie als ausgeführte Schritte gedeutet werden.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TauschStand {
    /// Der Tausch kann teilweise geschehen sein: zurücknehmen.
    #[default]
    Laeuft,
    /// Der Tausch ist vollständig: nur noch den Zwischenordner entfernen.
    Fertig,
    /// Die Rücknahme ist vollständig: nur noch den Zwischenordner entfernen (der
    /// Papierkorb bleibt als Rückfall).
    Zurueckgenommen,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PlanDatei {
    pub pfad: String,
    /// SHA-256 der neuen Fassung (im Zwischenordner).
    pub neu_sha256: String,
    /// SHA-256 der bisherigen Fassung, falls es sie gab; ihre Kopie liegt dann im
    /// Papierkorb.
    pub alt_sha256: Option<String>,
}

/// Ein Seitenordner des Plans mit seinem Abdruck zur Planungszeit.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PlanOrdner {
    pub pfad: String,
    pub abdruck: Abdruck,
}

/// Abdruck eines Ordners: alle Dateien darunter (relativ, sortiert) mit Größe und
/// SHA-256. Leere Unterordner zählen nicht.
pub type Abdruck = Vec<ManifestDatei>;

const TAUSCHPLAN: &str = ".zurueckspielen.json";
const TAUSCHPLAN_VERSION: u32 = 3;
const STUFE_PRAEFIX: &str = ".zurueck-";

fn tauschplan_pfad(data: &Path) -> PathBuf {
    data.join(SICHERUNGEN).join(TAUSCHPLAN)
}

/// `<praefix><ziffern>` (Ordnernamen, die nur das Zurückspielen anlegt).
fn ziffern_nach(name: &str, praefix: &str) -> bool {
    name.strip_prefix(praefix)
        .is_some_and(|z| !z.is_empty() && z.chars().all(|c| c.is_ascii_digit()))
}

/// Seitenordner als Einheit: `projects/<seite>` oder `library/<buch>/fassungen/<seite>`.
fn ordner_einheit(rel: &str) -> bool {
    sicherer_rel(rel).is_ok()
        && einheit(rel).is_ok_and(|e| e == rel)
        && seite_der_einheit(rel).is_some()
}

/// Datei als Einheit: `projects/index.json` oder `library/<buch>/buch.json`.
fn datei_einheit_genau(rel: &str) -> bool {
    sicherer_rel(rel).is_ok()
        && einheit(rel).is_ok_and(|e| e == rel)
        && seite_der_einheit(rel).is_none()
}

fn sha_gueltig(sha: &str) -> bool {
    sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit())
}

fn abdruck_gueltig(abdruck: &Abdruck) -> bool {
    abdruck
        .iter()
        .all(|d| sicherer_rel(&d.pfad).is_ok() && sha_gueltig(&d.sha256))
        && abdruck.windows(2).all(|w| w[0].pfad < w[1].pfad)
}

/// Ist dieser Eintrag eine Verknüpfung? Unter Windows zählen dazu auch Junctions und
/// andere Namensersatz-Reparse-Punkte (so deutet die Standardbibliothek `is_symlink`).
fn ist_umweg(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// Abdruck eines Ordners bilden (gestreamt). Eine Verknüpfung darin, ein Eintrag, der
/// weder Datei noch Ordner ist, und jeder Lesefehler sind Fehler: dann wird nichts
/// verschoben oder gelöscht.
fn abdruck(dir: &Path) -> Result<Abdruck, String> {
    fn sammeln(wurzel: &Path, dir: &Path, aus: &mut Abdruck) -> Result<(), String> {
        let eintraege =
            fs::read_dir(dir).map_err(|e| format!("{} nicht lesbar: {e}", dir.display()))?;
        for eintrag in eintraege {
            let pfad = eintrag
                .map_err(|e| format!("{} nicht lesbar: {e}", dir.display()))?
                .path();
            let meta = fs::symlink_metadata(&pfad)
                .map_err(|e| format!("{} nicht lesbar: {e}", pfad.display()))?;
            if ist_umweg(&meta) {
                return Err(format!("{} ist eine Verknüpfung", pfad.display()));
            }
            if meta.is_dir() {
                sammeln(wurzel, &pfad, aus)?;
            } else if meta.is_file() {
                let (groesse, sha256) = pruefsumme(&pfad)
                    .map_err(|e| format!("{} nicht lesbar: {e}", pfad.display()))?;
                aus.push(ManifestDatei {
                    pfad: relativ(wurzel, &pfad),
                    groesse,
                    sha256,
                });
            } else {
                return Err(format!("{} ist keine gewöhnliche Datei", pfad.display()));
            }
        }
        Ok(())
    }
    let meta =
        fs::symlink_metadata(dir).map_err(|e| format!("{} nicht lesbar: {e}", dir.display()))?;
    if ist_umweg(&meta) || !meta.is_dir() {
        return Err(format!("{} ist kein gewöhnlicher Ordner", dir.display()));
    }
    let mut aus = Vec::new();
    sammeln(dir, dir, &mut aus)?;
    aus.sort_by(|a, b| a.pfad.cmp(&b.pfad));
    Ok(aus)
}

/// Kein Glied von `rel` unter `wurzel` ist eine Verknüpfung (Junction, Symlink). Fehlt
/// ein Glied, ist der Rest unbedenklich.
fn ohne_umweg(wurzel: &Path, rel: &str) -> Result<(), String> {
    let mut pfad = wurzel.to_path_buf();
    for teil in rel.split('/').filter(|t| !t.is_empty()) {
        pfad.push(teil);
        match fs::symlink_metadata(&pfad) {
            Ok(meta) if ist_umweg(&meta) => {
                return Err(format!(
                    "{} ist eine Verknüpfung; nichts angefasst.",
                    pfad.display()
                ))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("{} nicht lesbar: {e}", pfad.display())),
        }
    }
    Ok(())
}

/// Alle Wege des Plans frei von Verknüpfungen: Zwischenordner, Papierkorb und jede
/// Einheit an ihrem Ort, im Zwischenordner und im Papierkorb.
fn plan_ohne_umweg(data: &Path, plan: &Tauschplan) -> Result<(), String> {
    let einheiten = plan
        .weichen
        .iter()
        .chain(&plan.hin)
        .map(|o| o.pfad.as_str())
        .chain(plan.dateien.iter().map(|d| d.pfad.as_str()));
    let mut wege = vec![
        plan.stufe.clone(),
        format!("{SICHERUNGEN}/{}", plan.sicherung),
    ];
    wege.extend(plan.korb.clone());
    if let Some(korb) = &plan.korb {
        wege.push(format!("{korb}/{NEU_IM_KORB}"));
    }
    for rel in einheiten {
        wege.push(rel.to_string());
        wege.push(format!("{}/{rel}", plan.stufe));
        wege.push(format!("{SICHERUNGEN}/{}/{rel}", plan.sicherung));
        if let Some(korb) = &plan.korb {
            wege.push(format!("{korb}/{rel}"));
            wege.push(format!("{korb}/{NEU_IM_KORB}/{rel}"));
        }
    }
    wege.iter().try_for_each(|w| ohne_umweg(data, w))
}

/// Umbenennen und beide Elternketten bis zur Datenwurzel dauerhaft machen (auch eben mit
/// `create_dir_all` angelegte Ordner).
fn umbenennen(data: &Path, von: &Path, nach: &Path) -> Result<(), String> {
    fs::rename(von, nach).map_err(|e| e.to_string())?;
    kette_sichern(data, nach)?;
    kette_sichern(data, von)
}

/// Für Tausch und Rücknahme ist „vom Dateisystem nicht unterstützt“ (`Ok(false)`) kein
/// Erfolg: nicht dauerhaft gesichert heißt, nichts überschreiben, löschen oder abschließen.
fn dauerhaft(r: Result<bool, String>, pfad: &Path) -> Result<(), String> {
    match r? {
        true => Ok(()),
        false => Err(format!(
            "{} lässt sich auf diesem Dateisystem nicht dauerhaft sichern; nichts \
             überschrieben oder gelöscht.",
            pfad.display()
        )),
    }
}

/// Eine Datei (falls `pfad` eine ist) und jeden Ordner von ihrem Elternordner bis
/// einschließlich `wurzel` dauerhaft machen. So ist auch eine neu angelegte Ordnerkette
/// im bestehenden Vorfahren verankert. Streng: auch `Ok(false)` ist ein Fehler.
fn kette_sichern(wurzel: &Path, pfad: &Path) -> Result<(), String> {
    if fs::symlink_metadata(pfad).is_ok_and(|m| m.is_file()) {
        dauerhaft(datei_sync(pfad), pfad)?;
    }
    let mut dir = pfad.parent();
    while let Some(d) = dir {
        // Ein fehlender Ordner hat keine Einträge zu sichern; sein Vorfahr schon.
        if vorhanden(d) {
            dauerhaft(ordner_sync(d), d)?;
        }
        if d == wurzel || !d.starts_with(wurzel) {
            break;
        }
        dir = d.parent();
    }
    Ok(())
}

/// Alle Ordner unter `dir` (ohne Verknüpfungen zu folgen) und die Kette bis `wurzel`
/// dauerhaft machen; die Dateien darin sind beim Kopieren schon mit fsync gesichert.
fn baum_sichern(wurzel: &Path, dir: &Path) -> Result<(), String> {
    fn ordner(dir: &Path) -> Result<(), String> {
        let eintraege =
            fs::read_dir(dir).map_err(|e| format!("{} nicht lesbar: {e}", dir.display()))?;
        for eintrag in eintraege {
            let pfad = eintrag
                .map_err(|e| format!("{} nicht lesbar: {e}", dir.display()))?
                .path();
            let meta = fs::symlink_metadata(&pfad).map_err(|e| e.to_string())?;
            if meta.is_dir() && !ist_umweg(&meta) {
                ordner(&pfad)?;
            }
        }
        dauerhaft(ordner_sync(dir), dir)
    }
    ordner(dir)?;
    kette_sichern(wurzel, dir)
}

/// Dateiinhalt dauerhaft machen (unter Windows braucht `FlushFileBuffers` Schreibzugriff).
fn datei_sync(pfad: &Path) -> Result<bool, String> {
    let mut optionen = fs::OpenOptions::new();
    #[cfg(windows)]
    optionen.write(true);
    #[cfg(not(windows))]
    optionen.read(true);
    match optionen.open(pfad).and_then(|f| f.sync_all()) {
        Ok(()) => Ok(true),
        Err(e) if sync_nicht_unterstuetzt(&e) => {
            log::warn!("Buch-Ordner: Datei-Sync vom Dateisystem nicht unterstützt ({e})");
            Ok(false)
        }
        Err(e) => Err(format!(
            "{} ließ sich nicht dauerhaft sichern: {e}",
            pfad.display()
        )),
    }
}

/// Prüfsumme einer Datei, `None` wenn es sie nicht gibt. Jeder andere Fehler (auch ein
/// Ordner an ihrer Stelle) ist ein Fehler und wird nie als „fehlt“ gedeutet.
fn pruefsumme_opt(pfad: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(pfad) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
        Ok(meta) if !meta.is_file() => Err("keine gewöhnliche Datei".to_string()),
        Ok(_) => pruefsumme(pfad).map(|(_, sha)| Some(sha)),
    }
}

impl Tauschplan {
    /// Ein Plan von der Platte ist nicht vertrauenswürdig: Zwischenordner und Papierkorb
    /// haben genau die Form, die nur das Zurückspielen anlegt (nie eine Sicherung), jede
    /// Rolle passt zu ihrer Einheit, und Prüfsummen und Abdrücke sind wohlgeformt.
    fn pruefen(&self) -> Result<(), String> {
        let fehler = |was: &str| Err(format!("Tauschplan abgewiesen: {was}"));
        if self.version != TAUSCHPLAN_VERSION {
            return fehler("unbekannte Version");
        }
        if !ziffern_nach(&self.sicherung, SICHERUNG_PRAEFIX) {
            return fehler("Sicherung");
        }
        let stufe_ok = match self.stufe.split('/').collect::<Vec<_>>().as_slice() {
            [SICHERUNGEN, name] => ziffern_nach(name, STUFE_PRAEFIX),
            _ => false,
        };
        if !stufe_ok || sicherer_rel(&self.stufe).is_err() {
            return fehler("Zwischenordner");
        }
        if let Some(korb) = &self.korb {
            let korb_ok = match korb.split('/').collect::<Vec<_>>().as_slice() {
                [SICHERUNGEN, PAPIERKORB, name] => ziffern_nach(name, ""),
                _ => false,
            };
            if !korb_ok || sicherer_rel(korb).is_err() {
                return fehler("Papierkorb");
            }
        }
        let ordner_ok = |o: &PlanOrdner| ordner_einheit(&o.pfad) && abdruck_gueltig(&o.abdruck);
        if !self.weichen.iter().all(ordner_ok) {
            return fehler("weichende Ordner");
        }
        if !self.hin.iter().all(ordner_ok) {
            return fehler("eingesetzte Ordner");
        }
        let dateien_ok = self.dateien.iter().all(|d| {
            datei_einheit_genau(&d.pfad)
                && sha_gueltig(&d.neu_sha256)
                && d.alt_sha256.as_deref().is_none_or(sha_gueltig)
        });
        if !dateien_ok {
            return fehler("Dateien");
        }
        let braucht_korb =
            !self.weichen.is_empty() || self.dateien.iter().any(|d| d.alt_sha256.is_some());
        if braucht_korb && self.korb.is_none() {
            return fehler("Papierkorb fehlt");
        }
        Ok(())
    }

    /// Alles, was eingesetzt wird, stammt laut Manifest unverändert aus der Sicherung.
    /// Dann verliert das Zurücknehmen und Aufräumen eingesetzter Ordner nie etwas, das
    /// nicht in der Sicherung noch einmal liegt.
    fn gegen_sicherung_pruefen(&self, data: &Path) -> Result<(), String> {
        let roh = fs::read(data.join(SICHERUNGEN).join(&self.sicherung).join(MANIFEST))
            .map_err(|e| format!("Tauschplan abgewiesen: Manifest der Sicherung: {e}"))?;
        let manifest: Manifest = serde_json::from_slice(&roh)
            .map_err(|e| format!("Tauschplan abgewiesen: Manifest der Sicherung: {e}"))?;
        let bekannt: BTreeMap<&str, (u64, &str)> = manifest
            .dateien
            .iter()
            .map(|d| (d.pfad.as_str(), (d.groesse, d.sha256.as_str())))
            .collect();
        for ordner in &self.hin {
            for datei in &ordner.abdruck {
                let pfad = format!("{}/{}", ordner.pfad, datei.pfad);
                if bekannt.get(pfad.as_str()) != Some(&(datei.groesse, datei.sha256.as_str())) {
                    return Err(format!(
                        "Tauschplan abgewiesen: {pfad} stammt nicht aus der Sicherung"
                    ));
                }
            }
        }
        Ok(())
    }
}

fn tauschplan_laden(data: &Path) -> Result<Option<Tauschplan>, String> {
    let roh = match fs::read(tauschplan_pfad(data)) {
        Ok(roh) => roh,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Tauschplan nicht lesbar: {e}")),
    };
    let plan: Tauschplan =
        serde_json::from_slice(&roh).map_err(|e| format!("Tauschplan unlesbar: {e}"))?;
    plan.pruefen()?;
    plan.gegen_sicherung_pruefen(data)?;
    Ok(Some(plan))
}

fn tauschplan_schreiben(data: &Path, plan: &Tauschplan) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(plan).map_err(|e| e.to_string())?;
    schreibe_atomar(&tauschplan_pfad(data), &bytes)?;
    #[cfg(test)]
    if PLAN_SYNC_FEHLER.with(|f| f.replace(false)) {
        return Err("Sync des Plans gescheitert (Test)".into());
    }
    kette_sichern(data, &tauschplan_pfad(data))
}

#[cfg(test)]
thread_local! {
    /// Nur für Tests: beim Schritt mit dieser Nummer „stürzt der Prozess ab“ (der Lauf
    /// endet ohne weitere Rücknahme, der Plan bleibt liegen). Gezählt wird je Lauf über
    /// Tausch, Rücknahme und Aufräumen.
    static ABSTURZ_NACH: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static SCHRITTE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Nur für Tests: dieser Schritt scheitert gewöhnlich (ohne Absturz).
    static FEHLER_BEI: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Nur für Tests: das Dateisystem kann Ordner nicht synchronisieren.
    static OHNE_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Nur für Tests: der Sync nach dem nächsten Schreiben des Plans scheitert (einmal).
    static PLAN_SYNC_FEHLER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
const ABSTURZ: &str = "Absturz (Test)";

fn absturz_probe() -> Result<(), String> {
    #[cfg(test)]
    {
        let n = SCHRITTE.with(|s| {
            let n = s.get();
            s.set(n + 1);
            n
        });
        if ABSTURZ_NACH.with(|a| a.get()) == Some(n) {
            return Err(ABSTURZ.to_string());
        }
        if FEHLER_BEI.with(|f| f.get()) == Some(n) {
            return Err("Fehler (Test)".to_string());
        }
    }
    Ok(())
}

fn ist_absturz(_e: &str) -> bool {
    #[cfg(test)]
    if _e == ABSTURZ {
        return true;
    }
    false
}

/// Unterordner des Papierkorbs für neue Fassungen, die eine Rücknahme verdrängt.
const NEU_IM_KORB: &str = ".neu";

/// Die neue Fassung am Ort gestreamt in den Papierkorb kopieren (idempotent: eine schon
/// liegende Kopie zählt nur, wenn sie hashgleich ist).
fn neue_fassung_aufheben(data: &Path, ort: &Path, ziel: &Path, neu: &str) -> Result<(), String> {
    match pruefsumme_opt(ziel)? {
        // Schon da (früherer Versuch): erst nach erneutem Sync von Datei und Kette gilt sie.
        Some(sha) if sha == neu => return kette_sichern(data, ziel),
        Some(_) => return Err("im Papierkorb liegt schon eine andere neue Fassung".into()),
        None => {}
    }
    let (_, sha) = kopieren_mit_pruefsumme(ort, ziel)?;
    if sha != neu {
        let _ = fs::remove_file(ziel);
        return Err("die neue Fassung hat sich beim Aufheben geändert".into());
    }
    kette_sichern(data, ziel)
}

/// Eine Datei ohne alte Fassung darf die Rücknahme nur entfernen, wenn ihre neue Fassung
/// aus der Sicherung nachweisbar ist: `buch.json` hashgleich in der Sicherung, der Index
/// genau so, wie das Zurückspielen ihn ohne vorherigen Index aus der geprüften Sicherung
/// bildet.
fn neue_fassung_nachweisen(
    data: &Path,
    plan: &Tauschplan,
    datei: &PlanDatei,
) -> Result<(), String> {
    let dir = data.join(SICHERUNGEN).join(&plan.sicherung);
    let in_sicherung = |rel: &str| format!("{SICHERUNGEN}/{}/{rel}", plan.sicherung);
    ohne_umweg(data, &in_sicherung(MANIFEST))?;
    ohne_umweg(data, &in_sicherung(&datei.pfad))?;
    let sha = if datei.pfad == format!("{PROJEKTE}/{INDEX}") {
        let manifest = sicherung_pruefen(&dir)?;
        let (_, seiten, gesichert) = sicherung_inhalt(&dir, &manifest)?;
        let index = index_zusammenfuehren(None, gesichert, &seiten)
            .ok_or("die Sicherung ergibt keinen Index")?;
        let bytes = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
        hex(&Sha256::digest(&bytes))
    } else {
        pruefsumme(&absolut(&dir, &datei.pfad))?.1
    };
    if sha == datei.neu_sha256 {
        Ok(())
    } else {
        Err("ohne alte Fassung und nicht aus der Sicherung nachweisbar; angehalten".into())
    }
}

/// Die alte Fassung einer Datei aus dem Papierkorb einsetzen: gestreamt in eine
/// Temp-Datei neben dem Ziel kopieren, die Prüfsumme vergleichen, mit fsync sichern und
/// erst dann über das Ziel umbenennen.
fn alte_fassung_einsetzen(data: &Path, kopie: &Path, ort: &Path, alt: &str) -> Result<(), String> {
    let name = ort
        .file_name()
        .ok_or("Pfad ohne Namen")?
        .to_string_lossy()
        .into_owned();
    // Eine eigene, eindeutige Temp-Datei (`create_new`); eine vorhandene gleichen Namens
    // wird nie angefasst. Entfernt wird nur, was dieser Aufruf selbst angelegt hat.
    let tmp = (0..)
        .map(|n| ort.with_file_name(format!(".{name}.zurueck-{}-{n}", std::process::id())))
        .find(|p| !vorhanden(p))
        .ok_or("kein freier Name für die Temp-Datei")?;
    let (_, sha) = kopieren_mit_pruefsumme(kopie, &tmp)?;
    let eingesetzt = if sha == alt {
        umbenennen(data, &tmp, ort)
    } else {
        Err("Kopie im Papierkorb stimmt nicht".to_string())
    };
    if eingesetzt.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    eingesetzt
}

/// Den Tausch anhand des Plattenstands zurücknehmen (idempotent, solange Zwischenordner
/// und Papierkorb unangetastet sind, und das sind sie bis zum Stand `zurueckgenommen`).
/// Ersetzte Dateien bekommen ihre alte Fassung nur zurück, wenn am Ort genau die neue
/// liegt; eingesetzte Ordner gehen nur in den Zwischenordner, wenn sie genau ihrem
/// Abdruck entsprechen, gewichene nur dann aus dem Papierkorb an ihren Ort, nie über
/// etwas Vorhandenes. Was nicht passt oder sich nicht prüfen lässt, bleibt liegen und
/// steht in der Fehlerliste. `Err` beim simulierten Absturz oder bei einer Verknüpfung
/// auf einem der Wege.
fn tausch_zuruecknehmen(data: &Path, plan: &Tauschplan) -> Result<Vec<String>, String> {
    plan_ohne_umweg(data, plan)?;
    let stufe = absolut(data, &plan.stufe);
    let korb = plan.korb.as_ref().map(|k| absolut(data, k));
    let mut fehler = Vec::new();
    for datei in plan.dateien.iter().rev() {
        absturz_probe()?;
        let ort = absolut(data, &datei.pfad);
        if vorhanden(&absolut(&stufe, &datei.pfad)) {
            continue; // nicht ersetzt
        }
        let am_ort = match pruefsumme_opt(&ort) {
            Ok(sha) => sha,
            Err(e) => {
                fehler.push(format!("{} nicht prüfbar: {e}", datei.pfad));
                continue;
            }
        };
        if am_ort.as_deref() != Some(datei.neu_sha256.as_str()) {
            // Schon zurückgenommen (oder seither geändert): nichts anfassen.
            if am_ort.is_some() && am_ort != datei.alt_sha256 {
                fehler.push(format!(
                    "{} wurde seither geändert; unverändert gelassen",
                    datei.pfad
                ));
            }
            continue;
        }
        let r = match (&datei.alt_sha256, &korb) {
            // Die verdrängte neue Fassung bleibt im Papierkorb, bevor die alte zurückkommt.
            (Some(alt), Some(korb)) => neue_fassung_aufheben(
                data,
                &ort,
                &absolut(&korb.join(NEU_IM_KORB), &datei.pfad),
                &datei.neu_sha256,
            )
            .and_then(|()| alte_fassung_einsetzen(data, &absolut(korb, &datei.pfad), &ort, alt)),
            (Some(_), None) => Err("Papierkorb fehlt".to_string()),
            // Ohne alte Fassung wird nur entfernt, was nachweislich aus der Sicherung
            // stammt; sonst angehalten.
            (None, _) => neue_fassung_nachweisen(data, plan, datei).and_then(|()| {
                fs::remove_file(&ort).map_err(|e| e.to_string())?;
                kette_sichern(data, &ort)
            }),
        };
        if let Err(e) = r {
            fehler.push(format!("{}: {e}", datei.pfad));
        }
    }
    for ordner in plan.hin.iter().rev() {
        absturz_probe()?;
        let rel = &ordner.pfad;
        let gestuft = absolut(&stufe, rel);
        let ort = absolut(data, rel);
        if vorhanden(&gestuft) || !vorhanden(&ort) {
            continue;
        }
        match abdruck(&ort) {
            Ok(a) if a == ordner.abdruck => {}
            Ok(_) => {
                fehler.push(format!(
                    "{rel} entspricht nicht dem Plan; unverändert gelassen"
                ));
                continue;
            }
            Err(e) => {
                fehler.push(format!("{rel} nicht prüfbar: {e}"));
                continue;
            }
        }
        let r = gestuft
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .map_err(|e| e.to_string())
            .and_then(|()| umbenennen(data, &ort, &gestuft));
        if let Err(e) = r {
            fehler.push(format!("{rel}: {e}"));
        }
    }
    if let Some(korb) = &korb {
        for ordner in plan.weichen.iter().rev() {
            absturz_probe()?;
            let rel = &ordner.pfad;
            let gewichen = absolut(korb, rel);
            let ort = absolut(data, rel);
            if !vorhanden(&gewichen) {
                continue;
            }
            if vorhanden(&ort) {
                fehler.push(format!(
                    "{rel} ist belegt; die alte Fassung bleibt im Papierkorb"
                ));
                continue;
            }
            match abdruck(&gewichen) {
                Ok(a) if a == ordner.abdruck => {}
                Ok(_) => {
                    fehler.push(format!(
                        "{rel} im Papierkorb entspricht nicht dem Plan; er bleibt dort"
                    ));
                    continue;
                }
                Err(e) => {
                    fehler.push(format!("{rel} im Papierkorb nicht prüfbar: {e}"));
                    continue;
                }
            }
            let r = ort
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .map_err(|e| e.to_string())
                .and_then(|()| umbenennen(data, &gewichen, &ort));
            if let Err(e) = r {
                fehler.push(format!("{rel}: {e}"));
            }
        }
    }
    if fehler.is_empty() {
        fehler = zurueckgenommen_pruefen(data, plan);
    }
    if fehler.is_empty() {
        fehler = zuruecknahme_sichern(data, plan);
    }
    Ok(fehler)
}

/// Vor dem Stand `zurueckgenommen` alles, was die Rücknahme berührt, erneut dauerhaft
/// machen, auch Schritte, die ein früherer Versuch schon sichtbar ausgeführt hatte, deren
/// Sync aber gescheitert sein kann: Dateien und Ketten am Ort, im Zwischenordner und im
/// Papierkorb (samt `.neu`).
fn zuruecknahme_sichern(data: &Path, plan: &Tauschplan) -> Vec<String> {
    let stufe = absolut(data, &plan.stufe);
    let korb = plan.korb.as_ref().map(|k| absolut(data, k));
    let mut wege: Vec<PathBuf> = Vec::new();
    for datei in &plan.dateien {
        wege.push(absolut(data, &datei.pfad));
        wege.push(absolut(&stufe, &datei.pfad));
        if let Some(korb) = &korb {
            wege.push(absolut(korb, &datei.pfad));
            wege.push(absolut(&korb.join(NEU_IM_KORB), &datei.pfad));
        }
    }
    for ordner in plan.hin.iter().chain(&plan.weichen) {
        wege.push(absolut(data, &ordner.pfad));
        wege.push(absolut(&stufe, &ordner.pfad));
        if let Some(korb) = &korb {
            wege.push(absolut(korb, &ordner.pfad));
        }
    }
    wege.iter()
        .filter_map(|w| kette_sichern(data, w).err())
        .collect()
}

/// Nach der Rücknahme vom Datenträger nachlesen, ob genau der vorige Stand vorliegt:
/// gewichene Ordner mit ihrem Abdruck am Ort, eingesetzte fort, Dateien in alter Fassung.
fn zurueckgenommen_pruefen(data: &Path, plan: &Tauschplan) -> Vec<String> {
    let mut fehler = Vec::new();
    let gewichen: BTreeSet<&str> = plan.weichen.iter().map(|o| o.pfad.as_str()).collect();
    for ordner in &plan.weichen {
        if abdruck(&absolut(data, &ordner.pfad)).ok().as_ref() != Some(&ordner.abdruck) {
            fehler.push(format!("{} ist nicht wieder hergestellt", ordner.pfad));
        }
    }
    for ordner in &plan.hin {
        if !gewichen.contains(ordner.pfad.as_str()) && vorhanden(&absolut(data, &ordner.pfad)) {
            fehler.push(format!("{} liegt noch am Ort", ordner.pfad));
        }
    }
    for datei in &plan.dateien {
        match pruefsumme_opt(&absolut(data, &datei.pfad)) {
            Ok(sha) if sha == datei.alt_sha256 => {}
            Ok(_) => fehler.push(format!("{} ist nicht die alte Fassung", datei.pfad)),
            Err(e) => fehler.push(format!("{} nicht prüfbar: {e}", datei.pfad)),
        }
    }
    fehler
}

/// Nach dem Tausch vom Datenträger nachlesen, ob genau der neue Stand vorliegt.
fn getauscht_pruefen(data: &Path, plan: &Tauschplan) -> Result<(), String> {
    let eingesetzt: BTreeSet<&str> = plan.hin.iter().map(|o| o.pfad.as_str()).collect();
    for ordner in &plan.hin {
        if abdruck(&absolut(data, &ordner.pfad))? != ordner.abdruck {
            return Err(format!("{} ist nicht vollständig eingesetzt", ordner.pfad));
        }
    }
    for ordner in &plan.weichen {
        if !eingesetzt.contains(ordner.pfad.as_str()) && vorhanden(&absolut(data, &ordner.pfad)) {
            return Err(format!("{} liegt noch am Ort", ordner.pfad));
        }
    }
    for datei in &plan.dateien {
        if pruefsumme_opt(&absolut(data, &datei.pfad))?.as_deref()
            != Some(datei.neu_sha256.as_str())
        {
            return Err(format!("{} ist nicht die neue Fassung", datei.pfad));
        }
    }
    Ok(())
}

/// Einen Ordner nur entfernen, wenn er ausschließlich erwartete Dateien enthält (Pfad →
/// SHA-256; `None` = jeder Inhalt). Sonst bleibt er liegen, und das Aufräumen endet mit
/// einem Fehler.
fn entfernen_wenn_erwartet(
    dir: &Path,
    erwartet: &BTreeMap<String, Option<String>>,
    was: &str,
) -> Result<(), String> {
    if !vorhanden(dir) {
        return Ok(());
    }
    for datei in abdruck(dir).map_err(|e| format!("{was}: {e}"))? {
        let passt = match erwartet.get(&datei.pfad) {
            Some(None) => true,
            Some(Some(sha)) => *sha == datei.sha256,
            None => false,
        };
        if !passt {
            return Err(format!(
                "{was} enthält Unerwartetes ({}); er bleibt erhalten.",
                datei.pfad
            ));
        }
    }
    fs::remove_dir_all(dir).map_err(|e| format!("{was}: {e}"))?;
    ordner_sync(dir.parent().unwrap_or(dir))?;
    Ok(())
}

/// Aufräumen nach dem festgehaltenen Stand und zuletzt den Plan entfernen. Entfernt wird
/// nur der Zwischenordner: bei `fertig` darf er keine Datei mehr enthalten, bei
/// `zurueckgenommen` nur eingesetzte Ordner, deren jede Datei hashgleich in der Sicherung
/// liegt (nachgelesen, nicht nur laut Manifest), und nie live gewesene neue
/// Dateifassungen. Der Papierkorb wird nie geleert: er hält die Rückfallkopien (alte und
/// verdrängte neue Fassungen). Ein Abbruch hier wiederholt beim nächsten Start nur das
/// Aufräumen.
fn tausch_aufraeumen(data: &Path, plan: &Tauschplan) -> Result<(), String> {
    absturz_probe()?;
    plan_ohne_umweg(data, plan)?;
    let mut in_stufe = BTreeMap::new();
    if plan.stand == TauschStand::Zurueckgenommen {
        let sicherung = data.join(SICHERUNGEN).join(&plan.sicherung);
        for ordner in &plan.hin {
            for datei in &ordner.abdruck {
                let rel = format!("{}/{}", ordner.pfad, datei.pfad);
                // Die ganze Kette in der Sicherung ohne Verknüpfung, sonst könnte die
                // Kopie im Zwischenordner sich selbst bestätigen.
                ohne_umweg(data, &format!("{SICHERUNGEN}/{}/{rel}", plan.sicherung))?;
                let in_sicherung = pruefsumme(&absolut(&sicherung, &rel)).ok();
                if in_sicherung == Some((datei.groesse, datei.sha256.clone())) {
                    in_stufe.insert(rel, Some(datei.sha256.clone()));
                }
            }
        }
        for datei in &plan.dateien {
            in_stufe.insert(datei.pfad.clone(), Some(datei.neu_sha256.clone()));
        }
    }
    entfernen_wenn_erwartet(&absolut(data, &plan.stufe), &in_stufe, "Zwischenordner")?;
    absturz_probe()?;
    fs::remove_file(tauschplan_pfad(data)).map_err(|e| format!("Tauschplan: {e}"))?;
    ordner_sync(&data.join(SICHERUNGEN))?;
    Ok(())
}

/// Zurücknehmen, den Stand `zurueckgenommen` festhalten, aufräumen.
fn zuruecknehmen_und_aufraeumen(data: &Path, mut plan: Tauschplan) -> Result<(), String> {
    if plan.stand == TauschStand::Laeuft {
        let fehler = tausch_zuruecknehmen(data, &plan)?;
        if !fehler.is_empty() {
            return Err(format!(
                "Nicht alles ließ sich zurücknehmen ({}); angehalten. Zwischenordner, \
                 Papierkorb und Tauschplan bleiben erhalten, der nächste Start versucht es \
                 erneut.",
                fehler.join("; ")
            ));
        }
        absturz_probe()?;
        plan.stand = TauschStand::Zurueckgenommen;
        tauschplan_schreiben(data, &plan)?;
    }
    tausch_aufraeumen(data, &plan)
}

/// Beim Start, vor Bibliothek, Sync und dem Wiederanlauf der Migration: ein
/// unterbrochenes Zurückspielen abschließen. Stand `laeuft`: zurücknehmen; `fertig` oder
/// `zurueckgenommen`: nur aufräumen. `Ok(true)`: es gab eines. Ein Fehler lässt alles
/// liegen (der nächste Start versucht es erneut); dann soll nichts weiter umgebaut werden.
pub fn zurueckspielen_wiederanlauf(data: &Path) -> Result<bool, String> {
    let Some(plan) = tauschplan_laden(data)? else {
        return Ok(false);
    };
    zuruecknehmen_und_aufraeumen(data, plan)?;
    Ok(true)
}

/// Den Tausch nach Plan ausführen. Jeder Schritt prüft sein Ziel; ein weichender Ordner
/// geht nur, wenn er noch seinem Abdruck entspricht, und überschrieben wird nur eine
/// Datei-Einheit, deren alte Fassung schon geprüft als Kopie im Papierkorb liegt. Am Ende
/// wird der neue Stand vom Datenträger nachgelesen.
fn tausch_ausfuehren(data: &Path, plan: &Tauschplan) -> Result<(), String> {
    plan_ohne_umweg(data, plan)?;
    let stufe = absolut(data, &plan.stufe);
    let korb = plan.korb.as_ref().map(|k| absolut(data, k));
    let korb_ziel = |rel: &str| -> Result<PathBuf, String> {
        let korb = korb.as_ref().ok_or("Papierkorb fehlt")?;
        let nach = absolut(korb, rel);
        if vorhanden(&nach) {
            return Err(format!(
                "Im Papierkorb liegt schon {rel}; nichts überschrieben."
            ));
        }
        if let Some(eltern) = nach.parent() {
            fs::create_dir_all(eltern).map_err(|e| format!("Papierkorb: {e}"))?;
        }
        Ok(nach)
    };
    for ordner in &plan.weichen {
        absturz_probe()?;
        let rel = &ordner.pfad;
        let ort = absolut(data, rel);
        if abdruck(&ort)? != ordner.abdruck {
            return Err(format!(
                "{rel} hat sich seit der Planung geändert; nichts verschoben."
            ));
        }
        let nach = korb_ziel(rel)?;
        umbenennen(data, &ort, &nach)
            .map_err(|e| format!("{rel} ließ sich nicht in den Papierkorb legen: {e}"))?;
    }
    for ordner in &plan.hin {
        absturz_probe()?;
        let rel = &ordner.pfad;
        let ort = absolut(data, rel);
        if vorhanden(&ort) {
            return Err(format!("{rel} ist noch belegt; nichts überschrieben."));
        }
        if abdruck(&absolut(&stufe, rel))? != ordner.abdruck {
            return Err(format!(
                "{rel} im Zwischenordner entspricht nicht dem Plan; nichts eingesetzt."
            ));
        }
        if let Some(eltern) = ort.parent() {
            fs::create_dir_all(eltern).map_err(|e| format!("{rel}: {e}"))?;
        }
        umbenennen(data, &absolut(&stufe, rel), &ort)
            .map_err(|e| format!("{rel} ließ sich nicht anlegen: {e}"))?;
    }
    for datei in &plan.dateien {
        absturz_probe()?;
        let ort = absolut(data, &datei.pfad);
        match &datei.alt_sha256 {
            Some(alt) => {
                let kopie = korb_ziel(&datei.pfad)?;
                let (_, sha) = kopieren_mit_pruefsumme(&ort, &kopie)?;
                kette_sichern(data, &kopie)?;
                if sha != *alt {
                    return Err(format!(
                        "{} hat sich seit der Planung geändert; nichts ersetzt.",
                        datei.pfad
                    ));
                }
            }
            None if vorhanden(&ort) => {
                return Err(format!(
                    "{} ist inzwischen belegt; nichts überschrieben.",
                    datei.pfad
                ));
            }
            None => {}
        }
        if let Some(eltern) = ort.parent() {
            fs::create_dir_all(eltern).map_err(|e| format!("{}: {e}", datei.pfad))?;
        }
        umbenennen(data, &absolut(&stufe, &datei.pfad), &ort)
            .map_err(|e| format!("{} ließ sich nicht ersetzen: {e}", datei.pfad))?;
    }
    absturz_probe()?;
    getauscht_pruefen(data, plan)
}

/// Einheiten, Seiten (mit Buch nach Ort) und gesicherter Index einer geprüften Sicherung.
#[allow(clippy::type_complexity)]
fn sicherung_inhalt(
    dir: &Path,
    manifest: &Manifest,
) -> Result<
    (
        BTreeSet<String>,
        BTreeMap<String, Option<String>>,
        Option<serde_json::Value>,
    ),
    String,
> {
    let mut einheiten = BTreeSet::new();
    for rel in manifest
        .ordner
        .iter()
        .chain(manifest.dateien.iter().map(|d| &d.pfad))
    {
        einheiten.insert(einheit(rel)?);
    }
    let seiten = einheiten
        .iter()
        .filter_map(|e| seite_der_einheit(e))
        .collect();
    let index_rel = format!("{PROJEKTE}/{INDEX}");
    let gesichert = if einheiten.contains(&index_rel) {
        Some(
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(absolut(dir, &index_rel)).map_err(|e| e.to_string())?,
            )
            .map_err(|e| format!("Seitenindex der Sicherung unlesbar: {e}"))?,
        )
    } else {
        None
    };
    Ok((einheiten, seiten, gesichert))
}

/// Sicherung zurückspielen (4.2), so dass nie ein halber Stand entsteht:
///
/// 1. Die Sicherung vollständig gegen das Manifest prüfen; den Platz prüfen.
/// 2. Alles in einen Zwischenordner `sicherungen/.zurueck-<ms>/` kopieren und dort erneut
///    prüfen. Der Seitenindex wird zusammengeführt statt ersetzt (siehe
///    [`index_zusammenfuehren`]).
/// 3. Tauschen: vorhandene Ziele und weitere Orte derselben Seiten (sonst läge die Seite
///    danach doppelt vor) in den Papierkorb `sicherungen/.papierkorb/<ms>/`, dann die
///    Einheiten aus dem Zwischenordner an ihren Ort. Index und `buch.json` werden über das
///    Ziel umbenannt, nachdem ihre alte Fassung in den Papierkorb kopiert ist; sie fehlen
///    also nie, auch nicht bei einem Absturz.
/// 4. Vor der ersten Änderung steht der Plan dauerhaft auf der Platte
///    ([`Tauschplan`]); scheitert ein Schritt oder bricht der Prozess ab, wird nach Plan
///    zurückgenommen (sofort bzw. beim nächsten Start).
///
/// Nichts wird überschrieben oder endgültig gelöscht.
pub fn sicherung_zurueckspielen(
    data: &Path,
    name: &str,
    platz_frei: Option<u64>,
) -> Result<Wiederherstellung, String> {
    let gueltiger_name = name.starts_with(SICHERUNG_PRAEFIX)
        && !name.contains(['/', '\\', ':'])
        && !name.contains("..");
    if !gueltiger_name {
        return Err(format!("Ungültiger Name einer Sicherung: {name}"));
    }
    let dir = data.join(SICHERUNGEN).join(name);
    if !dir.is_dir() {
        return Err(format!("Sicherung nicht gefunden: {name}"));
    }
    // Ein unterbrochenes Zurückspielen zuerst zurücknehmen.
    zurueckspielen_wiederanlauf(data)?;
    let manifest = sicherung_pruefen(&dir)?;
    let (einheiten, seiten, gesichert) = sicherung_inhalt(&dir, &manifest)?;
    let bytes: u64 = manifest.dateien.iter().map(|d| d.groesse).sum();
    if let Some(frei) = platz_frei {
        let noetig = platz_bedarf(bytes);
        if frei < noetig {
            return Err(format!(
                "Zu wenig freier Speicher zum Zurückspielen: nötig {} MB, frei {} MB. \
                 Es wurde nichts geändert.",
                noetig.div_ceil(1 << 20),
                frei / (1 << 20)
            ));
        }
    }
    // Vor jeder Änderung: ein unlesbarer aktueller Index bricht ab.
    let aktuell = index_laden(data)?;
    let index_rel = format!("{PROJEKTE}/{INDEX}");
    let index_neu = index_zusammenfuehren(aktuell, gesichert, &seiten);

    // 2. Zwischenordner
    let stufe = neuer_ordner(&data.join(SICHERUNGEN), STUFE_PRAEFIX)?;
    let vorbereitet = (|| -> Result<(), String> {
        for rel in &manifest.ordner {
            fs::create_dir_all(absolut(&stufe, rel)).map_err(|e| format!("{rel}: {e}"))?;
        }
        for datei in &manifest.dateien {
            if datei.pfad == index_rel {
                continue;
            }
            ohne_umweg(data, &format!("{SICHERUNGEN}/{name}/{}", datei.pfad))?;
            let (groesse, sha256) = kopieren_mit_pruefsumme(
                &absolut(&dir, &datei.pfad),
                &absolut(&stufe, &datei.pfad),
            )?;
            if groesse != datei.groesse || sha256 != datei.sha256 {
                return Err(format!(
                    "{} wurde beim Zurückspielen nicht unverändert kopiert",
                    datei.pfad
                ));
            }
        }
        if let Some(index) = &index_neu {
            let bytes = serde_json::to_vec_pretty(index).map_err(|e| e.to_string())?;
            schreibe_atomar(&absolut(&stufe, &index_rel), &bytes)?;
        }
        baum_sichern(data, &stufe)
    })();
    if let Err(e) = vorbereitet {
        let _ = fs::remove_dir_all(&stufe);
        return Err(format!("Zurückspielen nicht begonnen: {e}"));
    }

    // 3. Plan: was weicht (vorhandene Ordner-Einheiten und weitere Orte derselben
    // Seiten), was eingesetzt wird, welche Dateien ersetzt werden (Index zuletzt).
    let karte = Fundortkarte::aufbauen(data);
    let mut weichen_orte: BTreeSet<String> = einheiten
        .iter()
        .filter(|e| !datei_einheit(e) && vorhanden(&absolut(data, e)))
        .cloned()
        .collect();
    for seite in seiten.keys() {
        let mut orte = karte.orte(seite);
        orte.push(rel_von(seite));
        for ort in orte {
            if vorhanden(&absolut(data, &ort)) {
                weichen_orte.insert(ort);
            }
        }
    }
    let abdruecke = |wurzel: &Path, orte: Vec<&String>| -> Result<Vec<PlanOrdner>, String> {
        orte.into_iter()
            .map(|rel| {
                ohne_umweg(data, rel)?;
                Ok(PlanOrdner {
                    pfad: rel.clone(),
                    abdruck: abdruck(&absolut(wurzel, rel))?,
                })
            })
            .collect()
    };
    let geplant = abdruecke(data, weichen_orte.iter().collect()).and_then(|weichen| {
        let hin = abdruecke(
            &stufe,
            einheiten
                .iter()
                .filter(|e| !datei_einheit(e) && absolut(&stufe, e).is_dir())
                .collect(),
        )?;
        Ok((weichen, hin))
    });
    let (weichen, hin) = match geplant {
        Ok(geplant) => geplant,
        Err(e) => {
            let _ = fs::remove_dir_all(&stufe);
            return Err(format!("Zurückspielen nicht begonnen: {e}"));
        }
    };
    let mut dateien: Vec<PlanDatei> = Vec::new();
    for e in einheiten
        .iter()
        .filter(|e| datei_einheit(e) && absolut(&stufe, e).is_file())
    {
        let neu = pruefsumme(&absolut(&stufe, e)).map(|(_, sha)| sha);
        let alt = if vorhanden(&absolut(data, e)) {
            pruefsumme(&absolut(data, e)).map(|(_, sha)| Some(sha))
        } else {
            Ok(None)
        };
        match (neu, alt) {
            (Ok(neu_sha256), Ok(alt_sha256)) => dateien.push(PlanDatei {
                pfad: e.clone(),
                neu_sha256,
                alt_sha256,
            }),
            (Err(f), _) | (_, Err(f)) => {
                let _ = fs::remove_dir_all(&stufe);
                return Err(format!("Zurückspielen nicht begonnen: {e}: {f}"));
            }
        }
    }
    dateien.sort_by_key(|d| d.pfad == index_rel);
    let korb = if !weichen.is_empty() || dateien.iter().any(|d| d.alt_sha256.is_some()) {
        match neuer_ordner(&data.join(SICHERUNGEN).join(PAPIERKORB), "") {
            Ok(korb) => {
                let _ = fs::write(korb.join(PAPIERKORB_STEMPEL), jetzt_ms().to_string());
                Some(korb)
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&stufe);
                return Err(e);
            }
        }
    } else {
        None
    };
    let mut plan = Tauschplan {
        version: TAUSCHPLAN_VERSION,
        stand: TauschStand::Laeuft,
        sicherung: name.to_string(),
        stufe: relativ(data, &stufe),
        korb: korb.as_ref().map(|k| relativ(data, k)),
        weichen,
        hin,
        dateien,
    };
    // Erst der Plan auf der Platte, dann die erste Änderung.
    if let Err(e) = plan
        .pruefen()
        .and_then(|()| plan.gegen_sicherung_pruefen(data))
    {
        let _ = fs::remove_dir_all(&stufe);
        if let Some(korb) = &korb {
            let _ = fs::remove_dir_all(korb);
        }
        return Err(format!("Zurückspielen nicht begonnen: {e}"));
    }
    if let Err(e) = tauschplan_schreiben(data, &plan) {
        // Der Plan kann schon sichtbar sein (Umbenennen gelungen, Sync gescheitert): dann
        // gilt er als veröffentlicht, und nur die Rücknahme nach Plan darf aufräumen.
        if vorhanden(&tauschplan_pfad(data)) {
            return match zuruecknehmen_und_aufraeumen(data, plan) {
                Ok(()) => Err(format!("Zurückspielen nicht begonnen: {e}")),
                Err(f) => Err(format!("Zurückspielen nicht begonnen ({e}). {f}")),
            };
        }
        let _ = fs::remove_dir_all(&stufe);
        if let Some(korb) = &korb {
            let _ = fs::remove_dir_all(korb);
        }
        return Err(format!("Zurückspielen nicht begonnen: {e}"));
    }

    // 4. Tauschen; scheitert ein Schritt, wird nach Plan zurückgenommen.
    if let Err(e) = tausch_ausfuehren(data, &plan) {
        if ist_absturz(&e) {
            return Err(e);
        }
        return match zuruecknehmen_und_aufraeumen(data, plan) {
            Ok(()) => Err(format!(
                "Zurückspielen abgebrochen ({e}); der vorige Stand ist wiederhergestellt."
            )),
            Err(f) if ist_absturz(&f) => Err(f),
            Err(f) => Err(format!("Zurückspielen abgebrochen ({e}). {f}")),
        };
    }
    // Abschluss: erst `fertig` auf der Platte, dann nur noch aufräumen.
    plan.stand = TauschStand::Fertig;
    tauschplan_schreiben(data, &plan)?;
    let korb = plan.korb.clone();
    if let Err(e) = tausch_aufraeumen(data, &plan) {
        if ist_absturz(&e) {
            return Err(e);
        }
        log::warn!("Buch-Ordner: Aufräumen nach dem Zurückspielen unvollständig: {e}");
    }
    Ok(Wiederherstellung {
        dateien: manifest.dateien.len(),
        papierkorb: korb,
    })
}

// ---------------------------------------------------------------------------
// Planung
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub schritte: Vec<Schritt>,
    /// Hindern den Start (Doppelungen).
    pub probleme: Vec<Problem>,
    /// Übersprungene Seiten mit Grund; hindern den Start nicht.
    pub hinweise: Vec<String>,
    /// Bytes, die die Sicherung umfassen wird.
    pub bytes: u64,
}

/// Seiten, die laut `buch.json` (`text` und `editions[].text`) zu genau einem Buch gehören
/// und noch nicht vollständig im Buch-Ordner eingetragen sind.
pub fn planen(data: &Path) -> Result<Plan, String> {
    let karte = Fundortkarte::aufbauen(data);
    let index = index_laden(data)?;
    let mut zuordnung: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for buch in buecher(data) {
        for seite in buch_seiten(data, &buch) {
            zuordnung.entry(seite).or_default().insert(buch.clone());
        }
    }
    let mut plan = Plan::default();
    for (seite, zu) in zuordnung {
        if zu.len() > 1 {
            let liste: Vec<&str> = zu.iter().map(String::as_str).collect();
            plan.hinweise.push(format!(
                "Seite {seite} gehört zu mehreren Büchern ({}); sie bleibt, wo sie ist.",
                liste.join(", ")
            ));
            continue;
        }
        let Some(buch) = zu.into_iter().next() else {
            continue;
        };
        let im_index = index.as_ref().and_then(|i| index_buch_in(i, &seite));
        if let Some(anderes) = im_index.as_deref().filter(|b| *b != buch) {
            plan.hinweise.push(format!(
                "Seite {seite} ist im Index dem Buch {anderes} zugeordnet, laut Bibliothek \
                 aber {buch}; sie bleibt, wo sie ist."
            ));
            continue;
        }
        let schritt = Schritt::neu(&seite, &buch);
        let von = absolut(data, &schritt.von);
        let nach = absolut(data, &schritt.nach);
        match aufloesen_mit(data, &seite, im_index.as_deref(), &karte, None) {
            Aufloesung::Fehlt => plan.hinweise.push(format!(
                "Seite {seite} aus Buch {buch} ist nicht vorhanden; sie wird übersprungen."
            )),
            Aufloesung::Doppelt(pfade) => {
                plan.probleme.push(Problem::doppelt(data, &seite, &pfade))
            }
            Aufloesung::Gefunden(ort) if ort == von => plan.schritte.push(schritt),
            Aufloesung::Gefunden(ort) if ort == nach => {
                // Liegt schon im Buch-Ordner; fehlt nur der Index-Eintrag, wird er nachgetragen.
                if im_index.as_deref() != Some(buch.as_str()) {
                    plan.schritte.push(schritt);
                }
            }
            Aufloesung::Gefunden(ort) => plan.hinweise.push(format!(
                "Seite {seite} liegt an einem unerwarteten Ort ({}); sie wird übersprungen.",
                relativ(data, &ort)
            )),
        }
    }
    let (dateien, _) = sicherung_umfang(data, &plan.schritte)?;
    plan.bytes = summe_bytes(data, &dateien);
    Ok(plan)
}

// ---------------------------------------------------------------------------
// Migration, Wiederanlauf, Rückfall
// ---------------------------------------------------------------------------

/// Steht die Seite im Index? `None`, wenn der Index fehlt oder keine Seitenliste hat
/// (dann lässt sich „gelöscht“ nicht feststellen).
fn index_hat_seite(index: Option<&serde_json::Value>, seite: &str) -> Option<bool> {
    let seiten = index?.get("pages")?.as_array()?;
    Some(
        seiten
            .iter()
            .any(|e| e.get("id").and_then(|i| i.as_str()) == Some(seite)),
    )
}

/// Wurde die Seite eines Schritts inzwischen gelöscht (Nutzer oder Geräte-Sync, während
/// die Migration angehalten war)? Nicht mehr im lesbaren Index und weder unter `von` noch
/// unter `nach`. Solche Schritte werden mit Hinweis übersprungen; sonst hielte das Journal
/// für immer an.
fn schritt_geloescht(
    data: &Path,
    index: Option<&serde_json::Value>,
    karte: &Fundortkarte,
    journal: &Journal,
    schritt: &Schritt,
) -> bool {
    // Erst alle Kandidaten (Fundortkarte, Journal, `projects/`): liegt die Seite
    // irgendwo, ist sie nicht gelöscht, auch nicht bei mehreren Fundorten.
    index_hat_seite(index, &schritt.seite) == Some(false)
        && aufloesen_mit(data, &schritt.seite, None, karte, Some(journal)) == Aufloesung::Fehlt
}

fn hinweis_geloescht(seite: &str) -> String {
    format!("Seite {seite} wurde inzwischen gelöscht; übersprungen.")
}

/// Doppelungsprüfung (Vorrang vor allen Zeilen der Tabelle) und Befund je Schritt,
/// bevor irgendetwas geändert wird.
fn vorpruefen(data: &Path, journal: &Journal, karte: &Fundortkarte) -> Vec<Problem> {
    let index = match index_laden(data) {
        Ok(index) => index,
        Err(e) => return vec![Problem::fehler("", e)],
    };
    let mut probleme = Vec::new();
    for schritt in &journal.schritte {
        if schritt_geloescht(data, index.as_ref(), karte, journal, schritt) {
            continue;
        }
        let im_index = index
            .as_ref()
            .and_then(|i| index_buch_in(i, &schritt.seite));
        match aufloesen_mit(
            data,
            &schritt.seite,
            im_index.as_deref(),
            karte,
            Some(journal),
        ) {
            Aufloesung::Doppelt(pfade) => {
                probleme.push(Problem::doppelt(data, &schritt.seite, &pfade))
            }
            Aufloesung::Fehlt => probleme.push(Problem::fehlt(schritt, &journal.sicherung)),
            Aufloesung::Gefunden(ort) => {
                if ort != absolut(data, &schritt.von) && ort != absolut(data, &schritt.nach) {
                    probleme.push(Problem::unerwartet(data, schritt, &ort));
                }
            }
        }
    }
    probleme
}

/// Den einen vorhandenen Ort eines Schritts bestimmen (mit Doppelungsprüfung).
fn ort_des_schritts(
    data: &Path,
    journal: &Journal,
    i: usize,
    karte: &Fundortkarte,
) -> Result<PathBuf, Problem> {
    let schritt = &journal.schritte[i];
    let im_index =
        index_buch(data, &schritt.seite).map_err(|e| Problem::fehler(&schritt.seite, e))?;
    match aufloesen_mit(
        data,
        &schritt.seite,
        im_index.as_deref(),
        karte,
        Some(journal),
    ) {
        Aufloesung::Gefunden(ort) => Ok(ort),
        Aufloesung::Doppelt(pfade) => Err(Problem::doppelt(data, &schritt.seite, &pfade)),
        Aufloesung::Fehlt => Err(Problem::fehlt(schritt, &journal.sicherung)),
    }
}

fn status_setzen(
    data: &Path,
    journal: &mut Journal,
    i: usize,
    status: SchrittStatus,
) -> Result<(), Problem> {
    journal.schritte[i].status = status;
    let seite = journal.schritte[i].seite.clone();
    journal_schreiben(data, journal).map_err(|e| Problem::fehler(&seite, e))
}

/// Ein Schritt vorwärts: `rename(von, nach)` → `verschoben` → Index `buch` → `eingetragen`.
fn schritt_vorwaerts(
    data: &Path,
    journal: &mut Journal,
    i: usize,
    karte: &Fundortkarte,
) -> Result<bool, Problem> {
    let schritt = journal.schritte[i].clone();
    if schritt.status == SchrittStatus::Eingetragen {
        return Ok(false);
    }
    let von = absolut(data, &schritt.von);
    let nach = absolut(data, &schritt.nach);
    let ort = ort_des_schritts(data, journal, i, karte)?;
    match schritt.status {
        SchrittStatus::Offen if ort == von => {
            // Nie ein vorhandenes Ziel überschreiben (die Doppelungsprüfung deckt das schon ab;
            // hier unmittelbar vor dem Umbenennen noch einmal).
            if vorhanden(&nach) {
                return Err(Problem::doppelt(data, &schritt.seite, &[von, nach]));
            }
            let buch_dir = data.join(BIBLIOTHEK).join(&schritt.buch);
            if !buch_dir.join(BUCH_JSON).is_file() {
                return Err(Problem::fehler(
                    &schritt.seite,
                    format!("Buch {} ist nicht (mehr) vorhanden.", schritt.buch),
                ));
            }
            if let Some(eltern) = nach.parent() {
                fs::create_dir_all(eltern).map_err(|e| {
                    Problem::fehler(&schritt.seite, format!("Ordner nicht anlegbar: {e}"))
                })?;
            }
            fs::rename(&von, &nach).map_err(|e| {
                Problem::fehler(
                    &schritt.seite,
                    format!("Seite {} ließ sich nicht verschieben: {e}", schritt.seite),
                )
            })?;
            status_setzen(data, journal, i, SchrittStatus::Verschoben)?;
        }
        // Tabelle: Schritt `offen`, Seite unter `nach`, nicht unter `von` → als `verschoben` werten.
        SchrittStatus::Offen if ort == nach => {
            status_setzen(data, journal, i, SchrittStatus::Verschoben)?;
        }
        SchrittStatus::Verschoben if ort == nach => {}
        _ => return Err(Problem::unerwartet(data, &schritt, &ort)),
    }
    // Tabelle: Schritt `verschoben` → Index-Eintrag setzen, `eingetragen`.
    index_buch_setzen(data, &schritt.seite, Some(&schritt.buch))
        .map_err(|e| Problem::fehler(&schritt.seite, e))?;
    status_setzen(data, journal, i, SchrittStatus::Eingetragen)?;
    Ok(true)
}

/// Phase `gesichert`: alle Schritte ausführen, dann `fertig` und `layout: 2`.
fn ausfuehren(data: &Path, mut journal: Journal) -> Result<Bericht, String> {
    let karte = Fundortkarte::aufbauen(data);
    let probleme = vorpruefen(data, &journal, &karte);
    if !probleme.is_empty() {
        return Ok(Bericht::angehalten(journal.phase, 0, probleme));
    }
    let index = index_laden(data)?;
    let mut erledigt = 0;
    let mut hinweise = Vec::new();
    for i in 0..journal.schritte.len() {
        if schritt_geloescht(data, index.as_ref(), &karte, &journal, &journal.schritte[i]) {
            hinweise.push(hinweis_geloescht(&journal.schritte[i].seite));
            continue;
        }
        match schritt_vorwaerts(data, &mut journal, i, &karte) {
            Ok(true) => erledigt += 1,
            Ok(false) => {}
            Err(problem) => {
                let mut b = Bericht::angehalten(journal.phase, erledigt, vec![problem]);
                b.hinweise = hinweise;
                return Ok(b);
            }
        }
    }
    // Erst das Kennzeichen, dann der Abschluss: ein Abbruch dazwischen lässt das
    // Journal offen, und der Wiederanlauf schließt es ab.
    layout_setzen(data, 2)?;
    journal.phase = Phase::Fertig;
    journal_schreiben(data, &journal)?;
    Ok(Bericht {
        phase: Some(Phase::Fertig),
        erledigt,
        hinweise,
        ..Default::default()
    })
}

/// Phase `geplant`: Sicherung (neu) beginnen, dann ausführen. Schlägt die Sicherung fehl,
/// beginnt die Migration nicht: das Journal wird entfernt, sonst ändert sich nichts.
fn sichern_und_ausfuehren(
    data: &Path,
    mut journal: Journal,
    optionen: &Optionen,
) -> Result<Bericht, String> {
    let karte = Fundortkarte::aufbauen(data);
    let probleme = vorpruefen(data, &journal, &karte);
    if !probleme.is_empty() {
        return Ok(Bericht::angehalten(journal.phase, 0, probleme));
    }
    let sicherung = match sicherung_anlegen(data, &journal.schritte, optionen) {
        Ok(sicherung) => sicherung,
        Err(e) => {
            let _ = fs::remove_file(journal_pfad(data));
            return Err(e);
        }
    };
    journal.sicherung = sicherung;
    journal.phase = Phase::Gesichert;
    journal_schreiben(data, &journal)?;
    ausfuehren(data, journal)
}

/// Einen Schritt rückwärts: `rename(nach, von)` → `zurueckverschoben` → Index `buch`
/// leeren → `offen`.
fn schritt_rueckwaerts(
    data: &Path,
    journal: &mut Journal,
    i: usize,
    karte: &Fundortkarte,
) -> Result<bool, Problem> {
    let schritt = journal.schritte[i].clone();
    let von = absolut(data, &schritt.von);
    let nach = absolut(data, &schritt.nach);
    let ort = ort_des_schritts(data, journal, i, karte)?;
    let mut geaendert = false;
    if ort == nach {
        if vorhanden(&von) {
            return Err(Problem::doppelt(data, &schritt.seite, &[nach, von]));
        }
        if let Some(eltern) = von.parent() {
            fs::create_dir_all(eltern).map_err(|e| {
                Problem::fehler(&schritt.seite, format!("Ordner nicht anlegbar: {e}"))
            })?;
        }
        fs::rename(&nach, &von).map_err(|e| {
            Problem::fehler(
                &schritt.seite,
                format!(
                    "Seite {} ließ sich nicht zurückverschieben: {e}",
                    schritt.seite
                ),
            )
        })?;
        // Leeren Ordner `fassungen/` aufräumen; ist er nicht leer, bleibt er.
        if let Some(fassungen) = nach.parent() {
            let _ = fs::remove_dir(fassungen);
        }
        status_setzen(data, journal, i, SchrittStatus::Zurueckverschoben)?;
        geaendert = true;
    } else if ort == von {
        if matches!(
            schritt.status,
            SchrittStatus::Verschoben | SchrittStatus::Eingetragen
        ) {
            // Abbruch zwischen Umbenennen und Journal: liegt schon wieder unter `von`.
            status_setzen(data, journal, i, SchrittStatus::Zurueckverschoben)?;
        }
    } else {
        return Err(Problem::unerwartet(data, &schritt, &ort));
    }
    if journal.schritte[i].status == SchrittStatus::Zurueckverschoben {
        let im_index =
            index_buch(data, &schritt.seite).map_err(|e| Problem::fehler(&schritt.seite, e))?;
        if im_index.as_deref() == Some(schritt.buch.as_str()) {
            index_buch_setzen(data, &schritt.seite, None)
                .map_err(|e| Problem::fehler(&schritt.seite, e))?;
        }
        status_setzen(data, journal, i, SchrittStatus::Offen)?;
        geaendert = true;
    }
    Ok(geaendert)
}

/// Phase `rueckfall`: alle Schritte rückwärts (letzter zuerst), dann `zurueckgenommen`
/// und `layout: 1`.
fn rueckwaerts(data: &Path, mut journal: Journal) -> Result<Bericht, String> {
    let karte = Fundortkarte::aufbauen(data);
    let probleme = vorpruefen(data, &journal, &karte);
    if !probleme.is_empty() {
        return Ok(Bericht::angehalten(journal.phase, 0, probleme));
    }
    let index = index_laden(data)?;
    let mut erledigt = 0;
    let mut hinweise = Vec::new();
    for i in (0..journal.schritte.len()).rev() {
        if schritt_geloescht(data, index.as_ref(), &karte, &journal, &journal.schritte[i]) {
            hinweise.push(hinweis_geloescht(&journal.schritte[i].seite));
            continue;
        }
        match schritt_rueckwaerts(data, &mut journal, i, &karte) {
            Ok(true) => erledigt += 1,
            Ok(false) => {}
            Err(problem) => {
                let mut b = Bericht::angehalten(journal.phase, erledigt, vec![problem]);
                b.hinweise = hinweise;
                return Ok(b);
            }
        }
    }
    // Erst das Kennzeichen, dann der Abschluss: ein Abbruch dazwischen lässt das
    // Journal offen, und der Wiederanlauf schließt es ab.
    layout_setzen(data, 1)?;
    journal.phase = Phase::Zurueckgenommen;
    journal_schreiben(data, &journal)?;
    Ok(Bericht {
        phase: Some(Phase::Zurueckgenommen),
        erledigt,
        hinweise,
        ..Default::default()
    })
}

fn fortsetzen(data: &Path, journal: Journal, optionen: &Optionen) -> Result<Bericht, String> {
    match journal.phase {
        // Tabelle: `geplant` → Sicherung neu beginnen.
        Phase::Geplant => sichern_und_ausfuehren(data, journal, optionen),
        Phase::Gesichert => ausfuehren(data, journal),
        Phase::Rueckfall => rueckwaerts(data, journal),
        // Abgeschlossenes Journal vervollständigen: das Kennzeichen muss zur Phase
        // passen (ein Stand vor dieser Reihenfolge schrieb es erst danach).
        Phase::Fertig | Phase::Zurueckgenommen => {
            let wert = if journal.phase == Phase::Fertig { 2 } else { 1 };
            layout_setzen(data, wert)?;
            Ok(Bericht {
                phase: Some(journal.phase),
                ..Default::default()
            })
        }
    }
}

/// Wiederanlauf beim Start (vor dem ersten Lesen der Bibliothek): ein offenes Journal nach
/// der Tabelle in 4.3 auflösen. Bei Doppelung oder Fehlen wird angehalten; dann ist nichts
/// verschoben und nichts im Index geändert.
pub fn wiederanlauf(data: &Path, optionen: &Optionen) -> Result<Bericht, String> {
    match journal_laden(data)? {
        None => Ok(Bericht::default()),
        Some(journal) => fortsetzen(data, journal, optionen),
    }
}

/// Migration auf Wunsch starten (oder ein offenes Journal fortsetzen).
pub fn migrieren(data: &Path, optionen: &Optionen) -> Result<Bericht, String> {
    if let Some(journal) = journal_laden(data)? {
        if journal.phase == Phase::Rueckfall {
            return Err(
                "Ein Rückfall ist noch nicht abgeschlossen; erst den Rückfall beenden.".into(),
            );
        }
        if !journal.phase.abgeschlossen() {
            return fortsetzen(data, journal, optionen);
        }
    }
    let plan = planen(data)?;
    if !plan.probleme.is_empty() {
        return Ok(Bericht {
            phase: journal_laden(data)?.map(|j| j.phase),
            angehalten: true,
            probleme: plan.probleme,
            hinweise: plan.hinweise,
            ..Default::default()
        });
    }
    if plan.schritte.is_empty() {
        layout_setzen(data, 2)?;
        return Ok(Bericht {
            phase: journal_laden(data)?.map(|j| j.phase),
            hinweise: plan.hinweise,
            ..Default::default()
        });
    }
    journal_archivieren(data)?;
    let journal = Journal {
        version: JOURNAL_VERSION,
        phase: Phase::Geplant,
        sicherung: String::new(),
        schritte: plan.schritte,
    };
    journal_schreiben(data, &journal)?;
    let mut bericht = sichern_und_ausfuehren(data, journal, optionen)?;
    let mut hinweise = plan.hinweise;
    hinweise.append(&mut bericht.hinweise);
    bericht.hinweise = hinweise;
    Ok(bericht)
}

/// Rückfall auf Wunsch: dieselben Schritte rückwärts, ebenfalls im Journal.
pub fn rueckfall(data: &Path) -> Result<Bericht, String> {
    let Some(mut journal) = journal_laden(data)? else {
        return Err("Es gibt keine Migration, die sich zurücknehmen ließe.".into());
    };
    match journal.phase {
        Phase::Zurueckgenommen => {
            return Ok(Bericht {
                phase: Some(journal.phase),
                ..Default::default()
            })
        }
        Phase::Geplant => {
            // Noch nichts verschoben: nur abschließen (Kennzeichen zuerst).
            layout_setzen(data, 1)?;
            journal.phase = Phase::Zurueckgenommen;
            journal_schreiben(data, &journal)?;
            return Ok(Bericht {
                phase: Some(journal.phase),
                ..Default::default()
            });
        }
        Phase::Rueckfall => return rueckwaerts(data, journal),
        Phase::Gesichert | Phase::Fertig => {}
    }
    // Vor der ersten Änderung prüfen; bei einem Problem bleibt auch die Phase, wie sie ist.
    let probleme = vorpruefen(data, &journal, &Fundortkarte::aufbauen(data));
    if !probleme.is_empty() {
        return Ok(Bericht::angehalten(journal.phase, 0, probleme));
    }
    journal.phase = Phase::Rueckfall;
    journal_schreiben(data, &journal)?;
    rueckwaerts(data, journal)
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutStatus {
    pub layout: u32,
    /// Phase des Journals, falls es eines gibt.
    pub phase: Option<Phase>,
    pub probleme: Vec<Problem>,
    pub hinweise: Vec<String>,
    /// Seiten, die noch zu verschieben (bzw. im Rückfall zurückzuverschieben) sind.
    pub zu_verschieben: usize,
    /// Nötiger Platz für die Sicherung in Bytes (0, wenn keine nötig ist).
    pub platz_benoetigt: u64,
    pub platz_frei: Option<u64>,
}

pub fn status(data: &Path, platz_frei: &dyn Fn(&Path) -> Option<u64>) -> LayoutStatus {
    let mut status = LayoutStatus {
        layout: layout(data),
        platz_frei: platz_frei(data),
        ..Default::default()
    };
    let journal = match journal_laden(data) {
        Ok(journal) => journal,
        Err(e) => {
            status.probleme.push(Problem::fehler("", e));
            return status;
        }
    };
    status.phase = journal.as_ref().map(|j| j.phase);
    if let Some(journal) = journal.filter(|j| !j.phase.abgeschlossen()) {
        let karte = Fundortkarte::aufbauen(data);
        status.probleme = vorpruefen(data, &journal, &karte);
        status.zu_verschieben = journal
            .schritte
            .iter()
            .filter(|s| match journal.phase {
                Phase::Rueckfall => s.status != SchrittStatus::Offen,
                _ => s.status != SchrittStatus::Eingetragen,
            })
            .count();
        if journal.phase == Phase::Geplant {
            if let Ok((dateien, _)) = sicherung_umfang(data, &journal.schritte) {
                status.platz_benoetigt = platz_bedarf(summe_bytes(data, &dateien));
            }
        }
        return status;
    }
    match planen(data) {
        Ok(plan) => {
            status.zu_verschieben = plan.schritte.len();
            status.platz_benoetigt = if plan.schritte.is_empty() {
                0
            } else {
                platz_bedarf(plan.bytes)
            };
            status.probleme = plan.probleme;
            status.hinweise = plan.hinweise;
        }
        Err(e) => status.probleme.push(Problem::fehler("", e)),
    }
    status
}

// ---------------------------------------------------------------------------
// Löschkaskade und Namensregeln (4.3, 4.4)
// ---------------------------------------------------------------------------

/// Seiten (Fassungen), die ein Buch beim Gang in den Papierkorb mitnimmt: alles unter
/// `library/<buch>/fassungen/`. Der Papierkorb der Bibliothek verschiebt den ganzen
/// Buch-Ordner; Wiederherstellen bringt die Fassungen mit zurück. Danach ist die
/// Fundortkarte neu aufzubauen.
#[allow(dead_code)] // wird in P2b an `library_trash`/`library_restore` angebunden
pub fn kaskade_seiten(data: &Path, buch: &str) -> Vec<String> {
    if !gueltiges_buch(buch) {
        return Vec::new();
    }
    unterordner(&data.join(BIBLIOTHEK).join(buch).join(FASSUNGEN))
        .into_iter()
        .filter(|s| gueltige_seite(s))
        .collect()
}

/// Bevor die Bibliothek einen Buch-Ordner löscht, zusammenführt oder in den Papierkorb legt:
/// seine Fassungen (Seiten unter `<buch_dir>/fassungen/`) nach `projects/<seite>` zurücklegen
/// und im Index das Feld `buch` dieser Seiten leeren. Wie vor Layout 2 bleibt die
/// Vorlesen-Seite also erhalten; die Kaskade in den Papierkorb (4.3) folgt in P2b zusammen
/// mit ihrer Behandlung im Geräte-Sync.
///
/// `buch_dir` darf auch ein Eintrag im Papierkorb der Bibliothek sein. Nichts wird
/// überschrieben: liegt unter `projects/<seite>` schon etwas, bricht es ab, bevor etwas
/// bewegt ist; scheitert ein Umbenennen, kommen die schon bewegten Seiten zurück. Solange
/// ein Journal offen ist, wird nichts bewegt. Liefert die Zahl der verschobenen Seiten.
/// Der Aufrufer hält Bibliotheks- und Seitensperre.
pub fn fassungen_auslagern(data: &Path, buch_dir: &Path) -> Result<usize, String> {
    buch_ordner_bewegbar(data)?;
    let fassungen = buch_dir.join(FASSUNGEN);
    let seiten: Vec<String> = unterordner(&fassungen)
        .into_iter()
        .filter(|s| gueltige_seite(s))
        .collect();
    if seiten.is_empty() {
        return Ok(0);
    }
    // Den Index vorher lesen: ist er unlesbar, wird nichts bewegt.
    let index = index_laden(data)?;
    let belegt: Vec<String> = seiten
        .iter()
        .map(|s| rel_von(s))
        .filter(|rel| vorhanden(&absolut(data, rel)))
        .collect();
    if !belegt.is_empty() {
        return Err(format!(
            "Seiten des Buchs liegen schon unter {}; nichts verschoben.",
            belegt.join(", ")
        ));
    }
    fs::create_dir_all(data.join(PROJEKTE))
        .map_err(|e| format!("Ordner {PROJEKTE} nicht anlegbar: {e}"))?;
    let mut bewegt: Vec<(PathBuf, PathBuf)> = Vec::new();
    for seite in &seiten {
        let von = fassungen.join(seite);
        let nach = absolut(data, &rel_von(seite));
        if let Err(e) = fs::rename(&von, &nach) {
            for (v, n) in bewegt.iter().rev() {
                let _ = fs::rename(n, v);
            }
            return Err(format!(
                "Seite {seite} ließ sich nicht aus dem Buch-Ordner lösen: {e}"
            ));
        }
        bewegt.push((von, nach));
    }
    let _ = fs::remove_dir(&fassungen);
    if let Some(mut index) = index {
        let mut geaendert = false;
        if let Some(eintraege) = index.get_mut("pages").and_then(|p| p.as_array_mut()) {
            for eintrag in eintraege.iter_mut() {
                let betroffen = eintrag
                    .get("id")
                    .and_then(|i| i.as_str())
                    .is_some_and(|id| seiten.iter().any(|s| s == id));
                if let (true, Some(obj)) = (betroffen, eintrag.as_object_mut()) {
                    geaendert |= obj.remove("buch").is_some();
                }
            }
        }
        if geaendert {
            let bytes = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
            schreibe_atomar(&index_pfad(data), &bytes)?;
        }
    }
    Ok(seiten.len())
}

/// Darf ein Buch-Ordner gerade verschwinden (Papierkorb, Löschen, Zusammenlegen)? Solange
/// eine Neuordnung offen ist (auch wenn dieses Buch noch keine Fassung im Buch-Ordner hat),
/// nein: sonst scheitert ihr nächster Schritt am fehlenden `buch.json`. Ein unlesbares
/// Journal hält ebenfalls an.
pub fn buch_ordner_bewegbar(data: &Path) -> Result<(), String> {
    let offen = journal_laden(data)
        .map_err(|e| format!("Neuordnung der Bibliothek nicht prüfbar ({e}); nichts geändert."))?
        .is_some_and(|j| !j.phase.abgeschlossen());
    if offen {
        return Err(
            "Die Neuordnung der Bibliothek ist nicht abgeschlossen. Bitte sie erst \
                    abschließen oder zurücknehmen, dann erneut versuchen."
                .into(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Löschkaskade (P2b-2): Papierkorb und endgültiges Löschen samt Fassungen
// ---------------------------------------------------------------------------

/// Name des Papierkorbs der Bibliothek (`library/.papierkorb/<buch>`).
pub const BIBLIOTHEK_PAPIERKORB: &str = ".papierkorb";

/// Liegt dieses Buch im Papierkorb der Bibliothek (und nicht zugleich in der Bibliothek)?
/// Seiten eines solchen Buchs sind „mit dem Buch entfernt“: die Seitenliste blendet sie
/// aus, der Sync hält sie zurück (weder hoch- noch als gelöscht gemeldet), bis das Buch
/// zurückkommt oder endgültig gelöscht wird.
pub fn buch_im_papierkorb(data: &Path, buch: &str) -> bool {
    if !gueltiges_buch(buch) {
        return false;
    }
    let bibliothek = data.join(BIBLIOTHEK);
    !vorhanden(&bibliothek.join(buch))
        && bibliothek
            .join(BIBLIOTHEK_PAPIERKORB)
            .join(buch)
            .join(BUCH_JSON)
            .is_file()
}

/// Seiten-Kennungen unter `<buch_dir>/fassungen/` (nur gültige, nur Ordner).
fn fassungen_von(buch_dir: &Path) -> Vec<String> {
    unterordner(&buch_dir.join(FASSUNGEN))
        .into_iter()
        .filter(|s| gueltige_seite(s))
        .collect()
}

/// Einträge dieser Seiten aus dem Seitenindex nehmen (atomar). Ohne Index nichts zu tun.
/// Liefert die Zahl der ausgetragenen Einträge. Der Aufrufer hält die Seitensperre.
fn seiten_austragen(data: &Path, seiten: &[String]) -> Result<usize, String> {
    if seiten.is_empty() {
        return Ok(0);
    }
    let Some(mut index) = index_laden(data)? else {
        return Ok(0);
    };
    let Some(eintraege) = index.get_mut("pages").and_then(|p| p.as_array_mut()) else {
        return Ok(0);
    };
    let vorher = eintraege.len();
    eintraege.retain(|e| {
        !e.get("id")
            .and_then(|i| i.as_str())
            .is_some_and(|id| seiten.iter().any(|s| s == id))
    });
    let weg = vorher - eintraege.len();
    if weg > 0 {
        let bytes = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
        schreibe_atomar(&index_pfad(data), &bytes)?;
    }
    Ok(weg)
}

/// Ein Buch endgültig löschen (aus dem Papierkorb oder direkt), samt seinen Fassungen:
/// erst den Ordner entfernen, dann genau die Seiten aus dem Index nehmen, deren Ordner
/// danach wirklich fehlt. Scheitert das Entfernen mittendrin, bleiben die übrigen Seiten
/// im Index (und damit vom Sync zurückgehalten statt als gelöscht gemeldet). Ein
/// fehlender Ordner ist kein Fehler. Der Aufrufer hält Bibliotheks- und Seitensperre.
/// Liefert die Zahl der ausgetragenen Seiten.
pub fn buch_endgueltig_loeschen(data: &Path, buch_dir: &Path) -> Result<usize, String> {
    let meta = match fs::symlink_metadata(buch_dir) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("{} nicht lesbar: {e}", buch_dir.display())),
    };
    if ist_umweg(&meta) || !meta.is_dir() {
        return Err(format!(
            "{} ist kein gewöhnlicher Ordner; nichts gelöscht.",
            buch_dir.display()
        ));
    }
    let seiten = fassungen_von(buch_dir);
    let geloescht = fs::remove_dir_all(buch_dir);
    let weg: Vec<String> = seiten
        .into_iter()
        .filter(|s| !vorhanden(&buch_dir.join(FASSUNGEN).join(s)))
        .collect();
    let ausgetragen = seiten_austragen(data, &weg)?;
    geloescht.map_err(|e| format!("Buch nicht vollständig gelöscht: {e}"))?;
    Ok(ausgetragen)
}

// ---------------------------------------------------------------------------
// Neue Seiten direkt in den Buch-Ordner (P2b-2)
// ---------------------------------------------------------------------------

/// Nach dem Speichern eines Buchs: seine Seiten (Text, Transkript, Sprachfassungen laut
/// `buch.json`), die noch unter `projects/<seite>` liegen, in `library/<buch>/fassungen/`
/// holen und im Index das Feld `buch` setzen. So landen Import, Transkript, Zusammenführen
/// und Übersetzung direkt im Buch-Ordner, wie es die Migration für den Bestand tut.
///
/// Nur im neuen Layout (Kennzeichen ≥ 2) und ohne offene Neuordnung; sonst bleibt alles,
/// wie es ist (die Seite wird weiter unter `projects/` gefunden). Übergangen wird eine
/// Seite, die ein anderes Buch ebenfalls nennt, deren Ziel schon belegt ist oder deren
/// Weg über eine Verknüpfung führt. Nichts wird überschrieben. Liefert die verschobenen
/// Seiten. Der Aufrufer hält Bibliotheks- und Seitensperre.
pub fn seiten_einlagern(data: &Path, buch: &str) -> Result<Vec<String>, String> {
    if !gueltiges_buch(buch) || layout(data) < 2 || buch_ordner_bewegbar(data).is_err() {
        return Ok(Vec::new());
    }
    let buch_rel = format!("{BIBLIOTHEK}/{buch}");
    if !absolut(data, &buch_rel).join(BUCH_JSON).is_file() {
        return Ok(Vec::new());
    }
    let karte = Fundortkarte::aufbauen(data);
    let mut bewegt = Vec::new();
    for seite in buch_seiten(data, buch) {
        let von_rel = rel_von(&seite);
        let nach_rel = rel_nach(buch, &seite);
        let nach = absolut(data, &nach_rel);
        if vorhanden(&nach) {
            // Schon im Buch-Ordner: nur das Indexfeld nachziehen.
            if index_buch(data, &seite)?.as_deref() != Some(buch) {
                index_buch_setzen(data, &seite, Some(buch))?;
            }
            continue;
        }
        let von = absolut(data, &von_rel);
        let fremd = karte
            .orte(&seite)
            .iter()
            .any(|o| o != &von_rel && o != &nach_rel);
        if fremd || !von.is_dir() || ohne_umweg(data, &von_rel).is_err() {
            continue;
        }
        if ohne_umweg(data, &nach_rel).is_err() {
            continue;
        }
        let eltern = nach
            .parent()
            .expect("fassungen/<seite> hat einen Elternordner");
        fs::create_dir_all(eltern)
            .map_err(|e| format!("{} nicht anlegbar: {e}", eltern.display()))?;
        umbenennen(data, &von, &nach)
            .map_err(|e| format!("Seite {seite} nicht in den Buch-Ordner verschoben: {e}"))?;
        if let Err(e) = index_buch_setzen(data, &seite, Some(buch)) {
            // Index nicht schreibbar: Seite zurück, damit Index und Platte zusammenpassen.
            let _ = fs::rename(&nach, &von);
            return Err(e);
        }
        bewegt.push(seite);
    }
    Ok(bewegt)
}

// ---------------------------------------------------------------------------
// Dateien im Buch-Ordner (Buchdetail, P2b-2)
// ---------------------------------------------------------------------------

/// Ein Eintrag der Ansicht „Dateien im Buch-Ordner“.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuchEintrag {
    /// Pfad relativ zum Buch-Ordner, mit `/`.
    pub rel: String,
    /// Ebene unter dem Buch-Ordner (0 = direkt darin).
    pub tiefe: u32,
    pub ordner: bool,
    /// Größe in Bytes (bei Ordnern: alles darunter).
    pub bytes: u64,
    /// Zahl der Dateien darunter (bei Dateien 1).
    pub dateien: u64,
    /// Ordner, dessen Inhalt nicht einzeln gezeigt wird (zu tief oder zu viele Dateien).
    pub zusammengefasst: bool,
}

/// Bis zu dieser Ebene werden Ordner aufgeklappt (Buch-Ordner = 0).
const DATEIEN_TIEFE: u32 = 3;
/// Mehr Einträge in einem Ordner werden zusammengefasst (etwa 35 Hörbuch-Teile).
const DATEIEN_JE_ORDNER: usize = 12;

/// Inhalt eines Buch-Ordners für die Anzeige: Ordner zuerst, dann Dateien, je nach Name.
/// Versteckte Einträge (Punkt am Anfang) und Verknüpfungen werden nicht gezeigt und nicht
/// betreten. Ein Weg über eine Verknüpfung oder ein fehlendes Buch ist ein Fehler.
pub fn buch_dateien(data: &Path, buch: &str) -> Result<Vec<BuchEintrag>, String> {
    if !gueltiges_buch(buch) {
        return Err(format!("Ungültige Buch-Kennung: {buch}"));
    }
    let rel = format!("{BIBLIOTHEK}/{buch}");
    ohne_umweg(data, &rel)?;
    let dir = absolut(data, &rel);
    if !dir.join(BUCH_JSON).is_file() {
        return Err("Das Buch ist nicht in der Bibliothek.".into());
    }
    fn sichtbar(dir: &Path) -> Vec<(String, fs::Metadata)> {
        let Ok(eintraege) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut liste: Vec<(String, fs::Metadata)> = eintraege
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_owned();
                let meta = fs::symlink_metadata(e.path()).ok()?;
                (!name.starts_with('.') && !ist_umweg(&meta) && (meta.is_dir() || meta.is_file()))
                    .then_some((name, meta))
            })
            .collect();
        liste.sort_by(|a, b| b.1.is_dir().cmp(&a.1.is_dir()).then_with(|| a.0.cmp(&b.0)));
        liste
    }
    fn summe(dir: &Path) -> (u64, u64) {
        sichtbar(dir)
            .into_iter()
            .fold((0, 0), |(bytes, dateien), (name, meta)| {
                if meta.is_dir() {
                    let (b, d) = summe(&dir.join(name));
                    (bytes + b, dateien + d)
                } else {
                    (bytes + meta.len(), dateien + 1)
                }
            })
    }
    fn sammeln(dir: &Path, praefix: &str, tiefe: u32, aus: &mut Vec<BuchEintrag>) {
        for (name, meta) in sichtbar(dir) {
            let rel = format!("{praefix}{name}");
            if meta.is_file() {
                aus.push(BuchEintrag {
                    rel,
                    tiefe,
                    ordner: false,
                    bytes: meta.len(),
                    dateien: 1,
                    zusammengefasst: false,
                });
                continue;
            }
            let pfad = dir.join(&name);
            let (bytes, dateien) = summe(&pfad);
            let zusammengefasst =
                tiefe + 1 >= DATEIEN_TIEFE || sichtbar(&pfad).len() > DATEIEN_JE_ORDNER;
            aus.push(BuchEintrag {
                rel: rel.clone(),
                tiefe,
                ordner: true,
                bytes,
                dateien,
                zusammengefasst,
            });
            if !zusammengefasst {
                sammeln(&pfad, &format!("{rel}/"), tiefe + 1, aus);
            }
        }
    }
    let mut aus = Vec::new();
    sammeln(&dir, "", 0, &mut aus);
    Ok(aus)
}

const VERBOTENE_ZEICHEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
const RESERVIERTE_NAMEN: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Stamm und Endung (`.ext` samt Punkt; leer, wenn es keine gibt).
fn stamm_und_endung(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(pos) if pos > 0 => (&name[..pos], &name[pos..]),
        _ => (name, ""),
    }
}

/// Auf `max` Zeichen kürzen, die Endung bleibt (wenn sie nicht absurd lang ist).
fn kuerzen(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    let (stamm, endung) = stamm_und_endung(name);
    let endung_zeichen = endung.chars().count();
    if endung_zeichen * 2 > max {
        return name.chars().take(max).collect();
    }
    let gekuerzt: String = stamm.chars().take(max - endung_zeichen).collect();
    format!("{}{endung}", gekuerzt.trim_end_matches(['.', ' ']))
}

/// Dateiname nach 4.4 bereinigen: keine `/ \ : * ? " < > |`, keine Steuerzeichen (jeweils
/// durch `_` ersetzt), kein `.`/`..`, kein reservierter Windows-Name (bekommt `_` davor),
/// keine Endung mit Punkt oder Leerzeichen, höchstens 120 Zeichen, Unicode-NFC.
#[allow(dead_code)] // Anhänge, Notizen, Projektdateien (P2b/P3)
pub fn bereinigter_name(name: &str) -> Result<String, String> {
    let ersetzt: String = name
        .nfc()
        .map(|c| {
            if VERBOTENE_ZEICHEN.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut bereinigt = ersetzt
        .trim_start()
        .trim_end_matches(['.', ' '])
        .to_string();
    if bereinigt.is_empty() || bereinigt == "." || bereinigt == ".." {
        return Err(format!("Ungültiger Dateiname: {name}"));
    }
    let stamm = bereinigt.split('.').next().unwrap_or("").trim_end();
    if RESERVIERTE_NAMEN
        .iter()
        .any(|r| r.eq_ignore_ascii_case(stamm))
    {
        bereinigt = format!("_{bereinigt}");
    }
    let gekuerzt = kuerzen(&bereinigt, MAX_NAME);
    let gekuerzt = gekuerzt.trim_end_matches(['.', ' ']).to_string();
    if gekuerzt.is_empty() {
        return Err(format!("Ungültiger Dateiname: {name}"));
    }
    Ok(gekuerzt)
}

/// Vergleichsschlüssel: Unicode-NFC, ohne Beachtung der Groß-/Kleinschreibung.
fn vergleichsschluessel(name: &str) -> String {
    name.nfc()
        .collect::<String>()
        .to_lowercase()
        .nfc()
        .collect()
}

/// Freien, bereinigten Namen in `dir` finden: bei Kollision (ohne Groß-/Kleinschreibung,
/// nach NFC) `Name (2).ext`, `Name (3).ext` … Ein vorhandener Name wird nie zurückgegeben.
#[allow(dead_code)] // Anhänge, Notizen, Projektdateien (P2b/P3)
pub fn freier_name(dir: &Path, wunsch: &str) -> Result<String, String> {
    let name = bereinigter_name(wunsch)?;
    let belegt: BTreeSet<String> = fs::read_dir(dir)
        .map(|eintraege| {
            eintraege
                .flatten()
                .map(|e| vergleichsschluessel(&e.file_name().to_string_lossy()))
                .collect()
        })
        .unwrap_or_default();
    if !belegt.contains(&vergleichsschluessel(&name)) {
        return Ok(name);
    }
    let (stamm, endung) = stamm_und_endung(&name);
    for n in 2..10_000u32 {
        let zusatz = format!(" ({n})");
        let platz = MAX_NAME
            .saturating_sub(zusatz.chars().count() + endung.chars().count())
            .max(1);
        let kurz: String = stamm.chars().take(platz).collect();
        let kandidat = format!("{}{zusatz}{endung}", kurz.trim_end());
        if !belegt.contains(&vergleichsschluessel(&kandidat)) {
            return Ok(kandidat);
        }
    }
    Err(format!("Kein freier Name für {name} gefunden"))
}

// ---------------------------------------------------------------------------
// Tests (nur synthetische Daten)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static ZAEHLER: AtomicU32 = AtomicU32::new(0);

    /// Frischer Datenordner unter dem Temp-Verzeichnis.
    fn datenordner(name: &str) -> PathBuf {
        let n = ZAEHLER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("lva-buchordner-{}-{name}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(PROJEKTE)).unwrap();
        fs::create_dir_all(dir.join(BIBLIOTHEK)).unwrap();
        dir
    }

    fn seite_anlegen(ordner: &Path, inhalt: &str) {
        fs::create_dir_all(ordner.join("ki").join("dokumente")).unwrap();
        fs::write(
            ordner.join("state.json"),
            format!("{{\"text\":\"{inhalt}\"}}"),
        )
        .unwrap();
        fs::write(ordner.join("ki").join("dokumente").join("d1.json"), "{}").unwrap();
    }

    fn buch_anlegen(data: &Path, buch: &str, text: &str, fassungen: &[&str]) {
        let dir = data.join(BIBLIOTHEK).join(buch);
        fs::create_dir_all(&dir).unwrap();
        let editions: Vec<serde_json::Value> = fassungen
            .iter()
            .enumerate()
            .map(|(i, s)| serde_json::json!({ "id": format!("ed_{i}"), "text": { "page_id": s, "origin": "document" } }))
            .collect();
        let wert = serde_json::json!({
            "id": buch, "title": "Synthetisches Buch", "schema": 2,
            "text": { "page_id": text, "origin": "document" },
            "editions": editions,
        });
        fs::write(
            dir.join(BUCH_JSON),
            serde_json::to_vec_pretty(&wert).unwrap(),
        )
        .unwrap();
    }

    fn index_schreiben(data: &Path, eintraege: &[(&str, Option<&str>)]) {
        let pages: Vec<serde_json::Value> = eintraege
            .iter()
            .map(|(id, buch)| {
                let mut e = serde_json::json!({ "id": id, "title": format!("Titel {id}") });
                if let Some(b) = buch {
                    e["buch"] = serde_json::Value::String(b.to_string());
                }
                e
            })
            .collect();
        fs::write(
            data.join(PROJEKTE).join(INDEX),
            serde_json::to_vec_pretty(&serde_json::json!({ "pages": pages })).unwrap(),
        )
        .unwrap();
    }

    fn index_roh(data: &Path) -> Vec<u8> {
        fs::read(data.join(PROJEKTE).join(INDEX)).unwrap()
    }

    fn ohne_platzgrenze(_: &Path) -> Option<u64> {
        None
    }

    fn optionen() -> Optionen<'static> {
        Optionen {
            platz_frei: &ohne_platzgrenze,
            nach_sicherung: None,
        }
    }

    fn pfad(data: &Path, seite: &str) -> Result<PathBuf, String> {
        let buch = index_buch(data, seite).unwrap();
        seite_pfad(data, seite, buch.as_deref(), &Fundortkarte::aufbauen(data))
    }

    /// Standardlage: ein Buch mit zwei Fassungen, eine lose Seite.
    fn standardlage(name: &str) -> PathBuf {
        let data = datenordner(name);
        buch_anlegen(&data, "book_1", "page_1", &["page_1", "page_2"]);
        seite_anlegen(&data.join(PROJEKTE).join("page_1"), "eins");
        seite_anlegen(&data.join(PROJEKTE).join("page_2"), "zwei");
        seite_anlegen(&data.join(PROJEKTE).join("page_9"), "lose");
        index_schreiben(
            &data,
            &[("page_1", None), ("page_2", None), ("page_9", None)],
        );
        data
    }

    fn journal_mit(data: &Path, phase: Phase, schritte: Vec<Schritt>) {
        let j = Journal {
            version: 1,
            phase,
            sicherung: "sicherungen/buch-ordner-1".into(),
            schritte,
        };
        journal_schreiben(data, &j).unwrap();
    }

    fn schritt(seite: &str, buch: &str, status: SchrittStatus) -> Schritt {
        Schritt {
            status,
            ..Schritt::neu(seite, buch)
        }
    }

    fn verschieben(data: &Path, seite: &str, buch: &str) {
        let nach = absolut(data, &rel_nach(buch, seite));
        fs::create_dir_all(nach.parent().unwrap()).unwrap();
        fs::rename(absolut(data, &rel_von(seite)), nach).unwrap();
    }

    // ----- Migration im Ganzen ---------------------------------------------

    #[test]
    fn migration_verschiebt_buchseiten_und_laesst_lose_seiten() {
        let data = standardlage("ganz");
        let b = migrieren(&data, &optionen()).unwrap();
        assert!(!b.angehalten, "{:?}", b.probleme);
        assert_eq!(b.phase, Some(Phase::Fertig));
        assert_eq!(b.erledigt, 2);
        assert!(data
            .join("library/book_1/fassungen/page_1/state.json")
            .is_file());
        assert!(data
            .join("library/book_1/fassungen/page_2/ki/dokumente/d1.json")
            .is_file());
        assert!(!data.join("projects/page_1").exists());
        assert!(data.join("projects/page_9").is_dir(), "lose Seite bleibt");
        assert_eq!(
            index_buch(&data, "page_1").unwrap().as_deref(),
            Some("book_1")
        );
        assert_eq!(index_buch(&data, "page_9").unwrap(), None);
        assert_eq!(layout(&data), 2);
        let j = journal_laden(&data).unwrap().unwrap();
        assert_eq!(j.phase, Phase::Fertig, "Journal bleibt als Nachweis");
        assert!(j
            .schritte
            .iter()
            .all(|s| s.status == SchrittStatus::Eingetragen));
        assert_eq!(
            pfad(&data, "page_2").unwrap(),
            data.join("library/book_1/fassungen/page_2")
        );
        // Sicherung liegt vor und ist gültig.
        let liste = sicherungen_auflisten(&data);
        assert_eq!(liste.len(), 1);
        let m = sicherung_pruefen(&data.join(SICHERUNGEN).join(&liste[0].name)).unwrap();
        assert!(m
            .dateien
            .iter()
            .any(|d| d.pfad == "projects/page_1/state.json"));
        assert!(m.dateien.iter().any(|d| d.pfad == "projects/index.json"));
        assert!(m
            .dateien
            .iter()
            .any(|d| d.pfad == "library/book_1/buch.json"));
        assert!(!m
            .dateien
            .iter()
            .any(|d| d.pfad.starts_with("projects/page_9")));
    }

    #[test]
    fn zweiter_lauf_nach_erfolg_aendert_nichts() {
        let data = standardlage("zweiter");
        migrieren(&data, &optionen()).unwrap();
        let index_vorher = index_roh(&data);
        let journal_vorher = fs::read(journal_pfad(&data)).unwrap();
        let b = migrieren(&data, &optionen()).unwrap();
        assert_eq!(b.erledigt, 0);
        assert!(!b.angehalten);
        assert_eq!(index_roh(&data), index_vorher);
        assert_eq!(fs::read(journal_pfad(&data)).unwrap(), journal_vorher);
        assert_eq!(
            sicherungen_auflisten(&data).len(),
            1,
            "keine zweite Sicherung"
        );
        let w = wiederanlauf(&data, &optionen()).unwrap();
        assert_eq!(w.erledigt, 0);
    }

    // ----- Sicherung ---------------------------------------------------------

    #[test]
    fn manipulierte_sicherung_bricht_vor_der_ersten_aenderung_ab() {
        let data = standardlage("manipuliert");
        let index_vorher = index_roh(&data);
        let manipulieren = |dir: &Path| {
            fs::write(dir.join("projects/page_1/state.json"), "verfälscht").unwrap();
        };
        let opt = Optionen {
            platz_frei: &ohne_platzgrenze,
            nach_sicherung: Some(&manipulieren),
        };
        let fehler = migrieren(&data, &opt).unwrap_err();
        assert!(fehler.contains("nicht bestätigen"), "{fehler}");
        assert!(data.join("projects/page_1/state.json").is_file());
        assert!(!data.join("library/book_1/fassungen").exists());
        assert_eq!(index_roh(&data), index_vorher);
        assert_eq!(layout(&data), 1);
        assert!(
            !journal_pfad(&data).exists(),
            "Migration hat nicht begonnen"
        );
        assert!(
            sicherungen_auflisten(&data).is_empty(),
            "kaputte Sicherung entfernt"
        );
    }

    #[test]
    fn voller_datentraeger_verhindert_den_start() {
        let data = standardlage("voll");
        let index_vorher = index_roh(&data);
        let knapp = |_: &Path| Some(10u64);
        let opt = Optionen {
            platz_frei: &knapp,
            nach_sicherung: None,
        };
        let fehler = migrieren(&data, &opt).unwrap_err();
        assert!(fehler.contains("Zu wenig freier Speicher"), "{fehler}");
        assert!(data.join("projects/page_1").is_dir());
        assert_eq!(index_roh(&data), index_vorher);
        assert!(!journal_pfad(&data).exists());
    }

    #[test]
    fn sicherungspruefung_erkennt_fehlende_und_falsche_dateien() {
        let data = standardlage("pruefung");
        let dateien = vec!["projects/page_1/state.json".to_string()];
        let dir = sicherung_erstellen(&data, &dateien, &[], 1).unwrap();
        assert!(sicherung_pruefen(&dir).is_ok());
        fs::write(dir.join("projects/page_1/state.json"), "x").unwrap();
        assert!(sicherung_pruefen(&dir).unwrap_err().contains("Größe"));
        fs::remove_file(dir.join("projects/page_1/state.json")).unwrap();
        assert!(sicherung_pruefen(&dir).unwrap_err().contains("fehlt"));
        // Ein Manifest mit Ausbruchspfad wird abgewiesen.
        let boese = Manifest {
            version: 1,
            erstellt_ms: 1,
            ordner: vec![],
            dateien: vec![ManifestDatei {
                pfad: "../aussen.txt".into(),
                groesse: 0,
                sha256: String::new(),
            }],
        };
        fs::write(dir.join(MANIFEST), serde_json::to_vec(&boese).unwrap()).unwrap();
        assert!(sicherung_pruefen(&dir)
            .unwrap_err()
            .contains("Unzulässiger Pfad"));
    }

    #[test]
    fn sicherung_ueberschreibt_keinen_vorhandenen_ordner() {
        let data = standardlage("zeit");
        let a = sicherung_erstellen(&data, &[], &[], 5).unwrap();
        let b = sicherung_erstellen(&data, &[], &[], 5).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn zurueckspielen_legt_ziele_in_den_papierkorb_und_ueberschreibt_nichts() {
        let data = standardlage("zurueck");
        migrieren(&data, &optionen()).unwrap();
        let name = sicherungen_auflisten(&data)[0].name.clone();
        // Nach der Sicherung entsteht eine neue Seite: sie bleibt im Index.
        seite_anlegen(&data.join(PROJEKTE).join("page_neu"), "neu");
        let mut index: serde_json::Value = serde_json::from_slice(&index_roh(&data)).unwrap();
        index["pages"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "id": "page_neu", "title": "Neu" }));
        fs::write(
            data.join(PROJEKTE).join(INDEX),
            serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
        let index_nach_migration = index_roh(&data);
        let w = sicherung_zurueckspielen(&data, &name, None).unwrap();
        assert!(w.dateien >= 5);
        // Seiten wieder unter projects/, im Index ohne `buch`.
        assert_eq!(
            fs::read_to_string(data.join("projects/page_1/state.json")).unwrap(),
            "{\"text\":\"eins\"}"
        );
        assert_eq!(index_buch(&data, "page_1").unwrap(), None);
        assert_eq!(index_buch(&data, "page_2").unwrap(), None);
        // Die Seite liegt genau einmal vor; die Fassung im Buch-Ordner liegt im Papierkorb.
        assert_eq!(pfad(&data, "page_1").unwrap(), data.join("projects/page_1"));
        let korb = absolut(&data, w.papierkorb.as_deref().unwrap());
        assert!(korb
            .join("library/book_1/fassungen/page_1/state.json")
            .is_file());
        // Der alte Index liegt als Kopie im Papierkorb, nicht überschrieben.
        assert_eq!(
            fs::read(korb.join("projects/index.json")).unwrap(),
            index_nach_migration
        );
        assert!(korb.join("library/book_1/buch.json").is_file());
        // Die neue Seite und die lose Seite bleiben im Index.
        let roh = String::from_utf8(index_roh(&data)).unwrap();
        assert!(roh.contains("page_neu"), "{roh}");
        assert!(roh.contains("page_9"), "{roh}");
        // Kein Zwischenordner bleibt liegen.
        assert!(!unterordner(&data.join(SICHERUNGEN))
            .iter()
            .any(|n| n.starts_with(".zurueck-")));
    }

    #[test]
    fn zurueckspielen_nimmt_den_tausch_bei_einem_fehler_zurueck() {
        let mut geprueft = 0;
        for n in 0..80 {
            let data = standardlage(&format!("zurueck-fehler-{n}"));
            migrieren(&data, &optionen()).unwrap();
            let name = sicherungen_auflisten(&data)[0].name.clone();
            let vorher = stand(&data);
            FEHLER_BEI.with(|f| f.set(Some(n)));
            SCHRITTE.with(|z| z.set(0));
            let r = sicherung_zurueckspielen(&data, &name, None);
            FEHLER_BEI.with(|f| f.set(None));
            let Err(fehler) = r else { break };
            geprueft += 1;
            assert!(fehler.contains("vorige Stand"), "Schritt {n}: {fehler}");
            assert_eq!(stand(&data), vorher, "Schritt {n}");
            assert!(!tauschplan_pfad(&data).exists());
            assert!(!unterordner(&data.join(SICHERUNGEN))
                .iter()
                .any(|d| d.starts_with(STUFE_PRAEFIX)));
            // Der Papierkorb bleibt als Rückfall erhalten; er hält nur Kopien.
            for korb in unterordner(&data.join(SICHERUNGEN).join(PAPIERKORB)) {
                let korb = data.join(SICHERUNGEN).join(PAPIERKORB).join(korb);
                assert!(korb.join(PAPIERKORB_STEMPEL).is_file(), "Schritt {n}");
            }
        }
        assert!(geprueft >= 5, "{geprueft} Fehlerstellen");
    }

    /// Ohne vorherigen Index (frische Installation) ist der eingesetzte Index aus der
    /// Sicherung nachweisbar: die Rücknahme entfernt ihn und hält nicht an.
    #[test]
    fn zuruecknahme_ohne_vorherigen_index_haelt_nicht_an() {
        let mut geprueft = 0;
        for n in 0..80 {
            let data = standardlage(&format!("ohne-index-{n}"));
            migrieren(&data, &optionen()).unwrap();
            fs::remove_file(data.join(PROJEKTE).join(INDEX)).unwrap();
            let name = sicherungen_auflisten(&data)[0].name.clone();
            let vorher = stand(&data);
            FEHLER_BEI.with(|f| f.set(Some(n)));
            SCHRITTE.with(|z| z.set(0));
            let r = sicherung_zurueckspielen(&data, &name, None);
            FEHLER_BEI.with(|f| f.set(None));
            let Err(fehler) = r else { break };
            geprueft += 1;
            assert!(fehler.contains("vorige Stand"), "Schritt {n}: {fehler}");
            assert_eq!(stand(&data), vorher, "Schritt {n}");
            keine_zwischenstaende(&data);
        }
        assert!(geprueft >= 5, "{geprueft} Fehlerstellen");
    }

    /// Ein Dateisystem ohne Ordner-Sync: das Zurückspielen beginnt gar nicht erst.
    #[test]
    fn ohne_sync_beginnt_das_zurueckspielen_nicht() {
        let data = standardlage("ohne-sync");
        migrieren(&data, &optionen()).unwrap();
        let name = sicherungen_auflisten(&data)[0].name.clone();
        let vorher = stand(&data);
        OHNE_SYNC.with(|o| o.set(true));
        let r = sicherung_zurueckspielen(&data, &name, None);
        OHNE_SYNC.with(|o| o.set(false));
        let e = r.unwrap_err();
        assert!(
            e.contains("nicht begonnen") && e.contains("dauerhaft"),
            "{e}"
        );
        assert_eq!(stand(&data), vorher);
        keine_zwischenstaende(&data);
    }

    /// Der Plan ist sichtbar, sein Sync scheitert: kein Aufräumen am Plan vorbei, sondern
    /// Rücknahme nach Plan (hier sofort erfolgreich).
    #[test]
    fn plan_sichtbar_aber_nicht_gesichert_wird_nach_plan_zurueckgenommen() {
        let data = standardlage("plan-sync");
        migrieren(&data, &optionen()).unwrap();
        let name = sicherungen_auflisten(&data)[0].name.clone();
        let vorher = stand(&data);
        PLAN_SYNC_FEHLER.with(|f| f.set(true));
        let r = sicherung_zurueckspielen(&data, &name, None);
        PLAN_SYNC_FEHLER.with(|f| f.set(false));
        let e = r.unwrap_err();
        assert!(e.contains("Sync des Plans"), "{e}");
        assert_eq!(stand(&data), vorher);
        keine_zwischenstaende(&data);
        assert!(sicherung_pruefen(&data.join(SICHERUNGEN).join(&name)).is_ok());
    }

    /// Alle Dateien außerhalb von `sicherungen/` samt Inhalt.
    fn stand(data: &Path) -> BTreeMap<String, Vec<u8>> {
        fn sammeln(data: &Path, dir: &Path, aus: &mut BTreeMap<String, Vec<u8>>) {
            for e in fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                // Das Journal nennt die Sicherung mit Zeitstempel: es zählt nicht mit.
                if p == data.join(SICHERUNGEN) || p == journal_pfad(data) {
                    continue;
                }
                if p.is_dir() {
                    sammeln(data, &p, aus);
                } else {
                    aus.insert(relativ(data, &p), fs::read(&p).unwrap());
                }
            }
        }
        let mut aus = BTreeMap::new();
        sammeln(data, data, &mut aus);
        aus
    }

    fn absturz_bei(schritt: Option<usize>) {
        ABSTURZ_NACH.with(|a| a.set(schritt));
        SCHRITTE.with(|s| s.set(0));
    }

    fn keine_zwischenstaende(data: &Path) {
        assert!(!tauschplan_pfad(data).exists(), "Plan entfernt");
        assert!(!unterordner(&data.join(SICHERUNGEN))
            .iter()
            .any(|n| n.starts_with(STUFE_PRAEFIX)));
    }

    /// Abbruch an jeder Stelle des Tauschs, dann beim Start an jeder Stelle der
    /// Rücknahme bzw. des Aufräumens, bis ein Start durchläuft: am Ende liegt genau der
    /// vorige Stand vor (Abbruch vor `fertig`) oder genau der zurückgespielte (danach).
    #[test]
    fn zurueckspielen_ist_an_jeder_stelle_absturzfest() {
        let referenz = standardlage("absturz-ref");
        migrieren(&referenz, &optionen()).unwrap();
        let name = sicherungen_auflisten(&referenz)[0].name.clone();
        sicherung_zurueckspielen(&referenz, &name, None).unwrap();
        let zurueckgespielt = stand(&referenz);
        keine_zwischenstaende(&referenz);

        let mut abbrueche = 0;
        let mut nach_fertig = 0;
        for n in 0..80 {
            let data = standardlage(&format!("absturz-{n}"));
            migrieren(&data, &optionen()).unwrap();
            let name = sicherungen_auflisten(&data)[0].name.clone();
            let vorher = stand(&data);
            absturz_bei(Some(n));
            let r = sicherung_zurueckspielen(&data, &name, None);
            absturz_bei(None);
            match r {
                Ok(_) => {
                    assert_eq!(stand(&data), zurueckgespielt);
                    keine_zwischenstaende(&data);
                    break;
                }
                Err(e) if e == ABSTURZ => {}
                Err(e) => panic!("Schritt {n}: {e}"),
            }
            abbrueche += 1;
            let plan = tauschplan_laden(&data).unwrap().expect("Plan liegt");
            let erwartet = if plan.stand == TauschStand::Fertig {
                nach_fertig += 1;
                &zurueckgespielt
            } else {
                &vorher
            };
            let mut fertig = false;
            for k in 0..80 {
                absturz_bei(Some(k));
                let w = zurueckspielen_wiederanlauf(&data);
                absturz_bei(None);
                match w {
                    Ok(true) => {
                        fertig = true;
                        break;
                    }
                    Err(e) if e == ABSTURZ => {}
                    other => panic!("Schritt {n}/{k}: {other:?}"),
                }
            }
            assert!(fertig, "Schritt {n}: Wiederanlauf endet");
            assert_eq!(&stand(&data), erwartet, "Schritt {n}");
            keine_zwischenstaende(&data);
            assert!(!zurueckspielen_wiederanlauf(&data).unwrap());
            // Die Sicherung selbst ist unversehrt.
            assert!(sicherung_pruefen(&data.join(SICHERUNGEN).join(&name)).is_ok());
        }
        assert!(abbrueche >= 8, "{abbrueche} Abbruchstellen");
        assert!(nach_fertig >= 1, "auch nach dem Abschluss abgebrochen");
    }

    fn ordner(pfad: &str, abdruck: Abdruck) -> PlanOrdner {
        PlanOrdner {
            pfad: pfad.into(),
            abdruck,
        }
    }

    #[test]
    fn manipulierter_tauschplan_wird_abgewiesen() {
        let data = standardlage("plan-boese");
        migrieren(&data, &optionen()).unwrap();
        let sicherung = sicherungen_auflisten(&data)[0].name.clone();
        let index_vorher = index_roh(&data);
        let sha = |c: char| c.to_string().repeat(64);
        let gut = Tauschplan {
            version: TAUSCHPLAN_VERSION,
            stand: TauschStand::Laeuft,
            sicherung: sicherung.clone(),
            stufe: "sicherungen/.zurueck-1".into(),
            korb: Some("sicherungen/.papierkorb/1".into()),
            weichen: vec![ordner("projects/page_9", vec![])],
            hin: vec![],
            dateien: vec![],
        };
        assert!(gut.pruefen().is_ok());
        let boese = [
            Tauschplan {
                korb: Some(format!("sicherungen/{sicherung}")),
                ..gut.clone()
            },
            Tauschplan {
                korb: Some("sicherungen/.papierkorb/../buch-ordner-1".into()),
                ..gut.clone()
            },
            Tauschplan {
                stufe: format!("sicherungen/{sicherung}"),
                ..gut.clone()
            },
            Tauschplan {
                stufe: "sicherungen/.papierkorb/1".into(),
                ..gut.clone()
            },
            Tauschplan {
                weichen: vec![ordner("projects/index.json", vec![])],
                ..gut.clone()
            },
            Tauschplan {
                hin: vec![ordner("einstellungen.json", vec![])],
                ..gut.clone()
            },
            Tauschplan {
                sicherung: "../buch-ordner-1".into(),
                ..gut.clone()
            },
            Tauschplan {
                sicherung: "buch-ordner-1".into(),
                ..gut.clone()
            },
            Tauschplan {
                weichen: vec![ordner(
                    "projects/page_9",
                    vec![ManifestDatei {
                        pfad: "../x".into(),
                        groesse: 1,
                        sha256: sha('a'),
                    }],
                )],
                ..gut.clone()
            },
            Tauschplan {
                weichen: vec![ordner(
                    "projects/page_9",
                    vec![ManifestDatei {
                        pfad: "x".into(),
                        groesse: 1,
                        sha256: "kurz".into(),
                    }],
                )],
                ..gut.clone()
            },
            Tauschplan {
                dateien: vec![PlanDatei {
                    pfad: "projects/page_9".into(),
                    neu_sha256: sha('a'),
                    alt_sha256: None,
                }],
                ..gut.clone()
            },
            Tauschplan {
                dateien: vec![PlanDatei {
                    pfad: "projects/index.json".into(),
                    neu_sha256: "kurz".into(),
                    alt_sha256: None,
                }],
                ..gut.clone()
            },
            Tauschplan {
                korb: None,
                ..gut.clone()
            },
        ];
        fs::create_dir_all(data.join(SICHERUNGEN)).unwrap();
        for plan in &boese {
            fs::write(tauschplan_pfad(&data), serde_json::to_vec(plan).unwrap()).unwrap();
            assert!(zurueckspielen_wiederanlauf(&data).is_err(), "{plan:?}");
        }
        // Formal gültig, aber der Index am Ort ist nicht die neue Fassung des Plans: er
        // bleibt unangetastet, obwohl die Stufendatei fehlt.
        let plan = Tauschplan {
            weichen: vec![],
            dateien: vec![PlanDatei {
                pfad: "projects/index.json".into(),
                neu_sha256: sha('b'),
                alt_sha256: None,
            }],
            ..gut.clone()
        };
        fs::write(tauschplan_pfad(&data), serde_json::to_vec(&plan).unwrap()).unwrap();
        let _ = zurueckspielen_wiederanlauf(&data);
        assert_eq!(index_roh(&data), index_vorher);
        // Der aktuelle Index als „neue Fassung ohne alte“, Stufendatei fehlt: er ist nicht
        // aus der Sicherung nachweisbar, also wird angehalten statt gelöscht.
        let plan = Tauschplan {
            weichen: vec![],
            dateien: vec![PlanDatei {
                pfad: "projects/index.json".into(),
                neu_sha256: hex(&Sha256::digest(&index_vorher)),
                alt_sha256: None,
            }],
            ..gut.clone()
        };
        fs::write(tauschplan_pfad(&data), serde_json::to_vec(&plan).unwrap()).unwrap();
        let e = zurueckspielen_wiederanlauf(&data).unwrap_err();
        assert!(e.contains("nicht aus der Sicherung nachweisbar"), "{e}");
        assert_eq!(index_roh(&data), index_vorher);
        assert!(data.join("library/book_1/fassungen/page_1").is_dir());
        assert!(sicherung_pruefen(&data.join(SICHERUNGEN).join(&sicherung)).is_ok());
    }

    /// Ein Plan, der einen fremden Seitenordner als „eingesetzt“ führt, darf ihn weder in
    /// den Zwischenordner ziehen noch wegräumen: weder mit falschem noch mit echtem
    /// Abdruck (der stammt dann nicht aus der Sicherung).
    #[test]
    fn untergeschobener_hin_ordner_bleibt_unangetastet() {
        let data = standardlage("plan-hin");
        migrieren(&data, &optionen()).unwrap();
        let sicherung = sicherungen_auflisten(&data)[0].name.clone();
        let seite = "library/book_1/fassungen/page_1";
        let vorher = stand(&data);
        let echt = abdruck(&data.join(seite)).unwrap();
        assert!(!echt.is_empty());
        for (abdruck, stand_) in [
            (vec![], TauschStand::Laeuft),
            (echt.clone(), TauschStand::Laeuft),
            (echt.clone(), TauschStand::Zurueckgenommen),
        ] {
            let plan = Tauschplan {
                version: TAUSCHPLAN_VERSION,
                stand: stand_,
                sicherung: sicherung.clone(),
                stufe: "sicherungen/.zurueck-1".into(),
                korb: None,
                weichen: vec![],
                hin: vec![ordner(seite, abdruck)],
                dateien: vec![],
            };
            fs::write(tauschplan_pfad(&data), serde_json::to_vec(&plan).unwrap()).unwrap();
            assert!(zurueckspielen_wiederanlauf(&data).is_err(), "{plan:?}");
            assert_eq!(stand(&data), vorher);
            assert!(!data.join("sicherungen/.zurueck-1").exists());
        }
    }

    /// Ein Abbruch nach dem Ersetzen des Index; beim Start ist der Index unlesbar (hier:
    /// ein Ordner an seiner Stelle). Das darf nie als „fehlt“ gelten: kein Abschluss, kein
    /// Aufräumen, die alte Fassung bleibt im Papierkorb.
    #[test]
    fn lesefehler_am_ort_verhindert_abschluss_und_aufraeumen() {
        let index = format!("{PROJEKTE}/{INDEX}");
        let mut gefunden = None;
        for n in 0..80 {
            let data = standardlage(&format!("lesefehler-{n}"));
            migrieren(&data, &optionen()).unwrap();
            let name = sicherungen_auflisten(&data)[0].name.clone();
            absturz_bei(Some(n));
            let r = sicherung_zurueckspielen(&data, &name, None);
            absturz_bei(None);
            if r.is_ok() {
                break;
            }
            let plan = tauschplan_laden(&data).unwrap().unwrap();
            let ersetzt = plan.stand == TauschStand::Laeuft
                && plan
                    .dateien
                    .iter()
                    .any(|d| d.pfad == index && d.alt_sha256.is_some())
                && !absolut(&data, &plan.stufe).join(&index).exists()
                && data.join(&index).is_file();
            if ersetzt {
                gefunden = Some((data, plan));
                break;
            }
        }
        let (data, plan) = gefunden.expect("Abbruch nach dem Ersetzen des Index");
        let kopie = absolut(&absolut(&data, plan.korb.as_ref().unwrap()), &index);
        assert!(kopie.is_file());
        let neu = fs::read(data.join(&index)).unwrap();
        fs::remove_file(data.join(&index)).unwrap();
        fs::create_dir(data.join(&index)).unwrap();
        let e = zurueckspielen_wiederanlauf(&data).unwrap_err();
        assert!(e.contains("nicht prüfbar"), "{e}");
        assert!(tauschplan_pfad(&data).exists(), "Plan bleibt");
        assert!(kopie.is_file(), "alte Fassung bleibt im Papierkorb");
        assert!(
            absolut(&data, &plan.stufe).is_dir(),
            "Zwischenordner bleibt"
        );
        // Ist der Ort wieder lesbar, gelingt der Wiederanlauf.
        fs::remove_dir(data.join(&index)).unwrap();
        fs::write(data.join(&index), &neu).unwrap();
        // Fremde Dateien mit Temp-Namen bleiben unangetastet.
        let koeder = [
            data.join("projects/.index.json.zurueck"),
            data.join(format!(
                "projects/.index.json.zurueck-{}-0",
                std::process::id()
            )),
        ];
        for k in &koeder {
            fs::write(k, b"fremd").unwrap();
        }
        assert!(zurueckspielen_wiederanlauf(&data).unwrap());
        keine_zwischenstaende(&data);
        for k in &koeder {
            assert_eq!(fs::read(k).unwrap(), b"fremd");
        }
        // Die verdrängte neue Fassung liegt im Papierkorb.
        let korb = absolut(&data, plan.korb.as_ref().unwrap());
        assert_eq!(
            fs::read(absolut(&korb.join(NEU_IM_KORB), &index)).unwrap(),
            neu
        );
    }

    /// Ein Papierkorb hinter einer Verknüpfung wird weder zurückgeholt noch geleert.
    #[cfg(unix)]
    #[test]
    fn papierkorb_hinter_einer_verknuepfung_wird_abgewiesen() {
        let data = standardlage("plan-link");
        migrieren(&data, &optionen()).unwrap();
        let sicherung = sicherungen_auflisten(&data)[0].name.clone();
        let fremd = datenordner("plan-link-fremd");
        seite_anlegen(&fremd.join("1/projects/page_8"), "fremd");
        let fremd_abdruck = abdruck(&fremd.join("1/projects/page_8")).unwrap();
        fs::create_dir_all(data.join(SICHERUNGEN)).unwrap();
        std::os::unix::fs::symlink(&fremd, data.join(SICHERUNGEN).join(PAPIERKORB)).unwrap();
        for stand_ in [TauschStand::Laeuft, TauschStand::Zurueckgenommen] {
            let plan = Tauschplan {
                version: TAUSCHPLAN_VERSION,
                stand: stand_,
                sicherung: sicherung.clone(),
                stufe: "sicherungen/.zurueck-1".into(),
                korb: Some("sicherungen/.papierkorb/1".into()),
                weichen: vec![ordner("projects/page_8", fremd_abdruck.clone())],
                hin: vec![],
                dateien: vec![],
            };
            fs::write(tauschplan_pfad(&data), serde_json::to_vec(&plan).unwrap()).unwrap();
            let e = zurueckspielen_wiederanlauf(&data).unwrap_err();
            assert!(e.contains("Verknüpfung"), "{e}");
            assert!(!data.join("projects/page_8").exists());
            assert_eq!(
                abdruck(&fremd.join("1/projects/page_8")).unwrap(),
                fremd_abdruck
            );
        }
    }

    /// Zeigt ein Weg in der Sicherung per Verknüpfung in den Zwischenordner, bestätigte
    /// sich die Kopie selbst: das Aufräumen hält an und löscht nichts.
    #[cfg(unix)]
    #[test]
    fn sicherung_mit_verknuepfung_bestaetigt_keine_kopie() {
        let data = datenordner("sicherung-link");
        let stufe = data.join("sicherungen/.zurueck-1");
        seite_anlegen(&stufe.join("projects/page_7"), "kopie");
        let a = abdruck(&stufe.join("projects/page_7")).unwrap();
        let sicherung = data.join("sicherungen/buch-ordner-5");
        fs::create_dir_all(&sicherung).unwrap();
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            erstellt_ms: 5,
            ordner: vec![],
            dateien: a
                .iter()
                .map(|d| ManifestDatei {
                    pfad: format!("projects/page_7/{}", d.pfad),
                    ..d.clone()
                })
                .collect(),
        };
        fs::write(
            sicherung.join(MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(stufe.join("projects"), sicherung.join("projects")).unwrap();
        let plan = Tauschplan {
            version: TAUSCHPLAN_VERSION,
            stand: TauschStand::Zurueckgenommen,
            sicherung: "buch-ordner-5".into(),
            stufe: "sicherungen/.zurueck-1".into(),
            korb: None,
            weichen: vec![],
            hin: vec![ordner("projects/page_7", a.clone())],
            dateien: vec![],
        };
        fs::write(tauschplan_pfad(&data), serde_json::to_vec(&plan).unwrap()).unwrap();
        let e = zurueckspielen_wiederanlauf(&data).unwrap_err();
        assert!(e.contains("Verknüpfung"), "{e}");
        assert_eq!(abdruck(&stufe.join("projects/page_7")).unwrap(), a);
        assert!(tauschplan_pfad(&data).exists());
    }

    /// `Papierkorb/.neu` hinter einer Verknüpfung: keine Kopie nach draußen, nichts ersetzt.
    #[cfg(unix)]
    #[test]
    fn neu_im_papierkorb_hinter_einer_verknuepfung_wird_abgewiesen() {
        let data = standardlage("neu-link");
        migrieren(&data, &optionen()).unwrap();
        let sicherung = sicherungen_auflisten(&data)[0].name.clone();
        let index_vorher = index_roh(&data);
        let korb = data.join("sicherungen/.papierkorb/1");
        fs::create_dir_all(korb.join("projects")).unwrap();
        fs::write(korb.join("projects/index.json"), b"{}").unwrap();
        let fremd = datenordner("neu-link-fremd");
        std::os::unix::fs::symlink(&fremd, korb.join(NEU_IM_KORB)).unwrap();
        let plan = Tauschplan {
            version: TAUSCHPLAN_VERSION,
            stand: TauschStand::Laeuft,
            sicherung,
            stufe: "sicherungen/.zurueck-1".into(),
            korb: Some("sicherungen/.papierkorb/1".into()),
            weichen: vec![],
            hin: vec![],
            dateien: vec![PlanDatei {
                pfad: "projects/index.json".into(),
                neu_sha256: hex(&Sha256::digest(&index_vorher)),
                alt_sha256: Some(hex(&Sha256::digest(b"{}"))),
            }],
        };
        fs::write(tauschplan_pfad(&data), serde_json::to_vec(&plan).unwrap()).unwrap();
        let e = zurueckspielen_wiederanlauf(&data).unwrap_err();
        assert!(e.contains("Verknüpfung"), "{e}");
        assert_eq!(index_roh(&data), index_vorher);
        assert!(!fremd.join("projects/index.json").exists());
    }

    /// Ein weichender Ordner, der sich seit der Planung geändert hat, geht nicht in den
    /// Papierkorb; aus dem Papierkorb kommt nur zurück, was seinem Abdruck entspricht.
    #[test]
    fn abdruck_bindet_weichende_ordner() {
        let data = standardlage("abdruck-weichen");
        seite_anlegen(&data.join("projects/page_9"), "vorher");
        let a = abdruck(&data.join("projects/page_9")).unwrap();
        seite_anlegen(&data.join("projects/page_9"), "nachher");
        assert_ne!(abdruck(&data.join("projects/page_9")).unwrap(), a);
        let korb = data.join("sicherungen/.papierkorb/1");
        fs::create_dir_all(&korb).unwrap();
        let plan = Tauschplan {
            version: TAUSCHPLAN_VERSION,
            stand: TauschStand::Laeuft,
            sicherung: "buch-ordner-1".into(),
            stufe: "sicherungen/.zurueck-1".into(),
            korb: Some("sicherungen/.papierkorb/1".into()),
            weichen: vec![ordner("projects/page_9", a.clone())],
            hin: vec![],
            dateien: vec![],
        };
        let e = tausch_ausfuehren(&data, &plan).unwrap_err();
        assert!(e.contains("seit der Planung"), "{e}");
        assert!(data.join("projects/page_9").is_dir());
        // Im Papierkorb liegt etwas anderes als geplant: es bleibt dort.
        fs::rename(data.join("projects/page_9"), korb.join("projects_page_9")).unwrap();
        fs::create_dir_all(korb.join("projects")).unwrap();
        fs::rename(korb.join("projects_page_9"), korb.join("projects/page_9")).unwrap();
        let fehler = tausch_zuruecknehmen(&data, &plan).unwrap();
        assert!(
            fehler.iter().any(|f| f.contains("Papierkorb")),
            "{fehler:?}"
        );
        assert!(korb.join("projects/page_9").is_dir());
        assert!(!data.join("projects/page_9").exists());
    }

    #[test]
    fn sicherung_loeschen_nur_wenn_nichts_sie_braucht() {
        let data = standardlage("loeschen");
        migrieren(&data, &optionen()).unwrap();
        let name = sicherungen_auflisten(&data)[0].name.clone();
        assert!(sicherung_loeschen(&data, "../projects").is_err());
        assert!(sicherung_loeschen(&data, "buch-ordner-x").is_err());
        // Offene Neuordnung, die genau diese Sicherung nennt: bleibt.
        let mut j = journal_laden(&data).unwrap().unwrap();
        let roh = fs::read(journal_pfad(&data)).unwrap();
        j.phase = Phase::Gesichert;
        journal_schreiben(&data, &j).unwrap();
        assert!(sicherung_loeschen(&data, &name).is_err());
        fs::write(journal_pfad(&data), &roh).unwrap();
        // Unterbrochenes Zurückspielen: bleibt.
        fs::write(tauschplan_pfad(&data), b"{}").unwrap();
        assert!(sicherung_loeschen(&data, &name).is_err());
        fs::remove_file(tauschplan_pfad(&data)).unwrap();
        sicherung_loeschen(&data, &name).unwrap();
        assert!(sicherungen_auflisten(&data).is_empty());
        assert!(data.join("library/book_1/fassungen/page_1").is_dir());
        assert!(unterordner(&data.join(SICHERUNGEN))
            .iter()
            .all(|n| !n.starts_with(LOESCHEN_PRAEFIX)));
    }

    #[cfg(unix)]
    #[test]
    fn sicherung_loeschen_folgt_keiner_verknuepfung() {
        let data = standardlage("loeschen-umweg");
        migrieren(&data, &optionen()).unwrap();
        let name = sicherungen_auflisten(&data)[0].name.clone();
        let fremd = data.parent().unwrap().join("loeschen-umweg-fremd");
        let _ = fs::remove_dir_all(&fremd);
        fs::create_dir_all(&fremd).unwrap();
        fs::write(fremd.join("wichtig.txt"), b"bleibt").unwrap();
        // Verknüpfung tief in der Sicherung: abgelehnt, Sicherung und Ziel bleiben.
        let sicherung = data.join(SICHERUNGEN).join(&name);
        std::os::unix::fs::symlink(&fremd, sicherung.join("umweg")).unwrap();
        let fehler = sicherung_loeschen(&data, &name).unwrap_err();
        assert!(fehler.contains("Verknüpfung"), "{fehler}");
        assert!(sicherung.join(MANIFEST).is_file());
        assert!(fremd.join("wichtig.txt").is_file());
        fs::remove_file(sicherung.join("umweg")).unwrap();
        // Die Sicherung selbst als Verknüpfung auf einen fremden Ordner: abgelehnt.
        let echt = data.join("echt");
        fs::rename(&sicherung, &echt).unwrap();
        std::os::unix::fs::symlink(&fremd, &sicherung).unwrap();
        assert!(sicherung_loeschen(&data, &name).is_err());
        assert!(fremd.join("wichtig.txt").is_file());
        // Rest eines abgebrochenen Löschens mit Verknüpfung bleibt liegen, ohne Folgen.
        fs::remove_file(&sicherung).unwrap();
        fs::rename(&echt, &sicherung).unwrap();
        let rest = data
            .join(SICHERUNGEN)
            .join(format!("{LOESCHEN_PRAEFIX}{}", "a".repeat(32)));
        std::os::unix::fs::symlink(&fremd, &rest).unwrap();
        sicherung_loeschen(&data, &name).unwrap();
        assert!(fremd.join("wichtig.txt").is_file());
        assert!(sicherungen_auflisten(&data).is_empty());
    }

    /// Fall C: `sicherungen` wird zwischen Prüfung und Umbenennen gegen eine Verknüpfung
    /// auf einen fremden Ordner getauscht, der eine gleichnamige Sicherung mit gleichem
    /// Manifest enthält. Nichts wird gelöscht. Weil am privaten Ort nicht die gemessene
    /// Sicherung liegt, wird dort auch nichts zurückbenannt oder verändert; die Meldung
    /// nennt den Ort.
    fn fall_c(kennung: &str, tauschen: fn(&Path)) {
        let data = standardlage(kennung);
        migrieren(&data, &optionen()).unwrap();
        let name = sicherungen_auflisten(&data)[0].name.clone();
        let fremd = fall_c_fremd(&data);
        let _ = fs::remove_dir_all(&fremd);
        fs::create_dir_all(fremd.join(&name)).unwrap();
        fs::copy(
            data.join(SICHERUNGEN).join(&name).join(MANIFEST),
            fremd.join(&name).join(MANIFEST),
        )
        .unwrap();
        fs::write(fremd.join(&name).join("wichtig.txt"), b"bleibt").unwrap();
        VOR_UMBENENNEN.with(|h| h.set(Some(tauschen)));
        let fehler = sicherung_loeschen(&data, &name).unwrap_err();
        VOR_UMBENENNEN.with(|h| h.set(None));
        assert!(fehler.contains("nichts gelöscht"), "{fehler}");
        assert!(fehler.contains("nichts verändert"), "{fehler}");
        let eintraege = unterordner(&fremd);
        assert_eq!(eintraege.len(), 1, "{eintraege:?}");
        let ort = fremd.join(&eintraege[0]);
        assert!(ort.join("wichtig.txt").is_file());
        assert!(ort.join(MANIFEST).is_file());
        assert!(!ort.join(LOESCHMARKE).exists());
        assert!(data
            .join("sicherungen-echt")
            .join(&name)
            .join(MANIFEST)
            .is_file());
    }

    fn fall_c_fremd(data: &Path) -> PathBuf {
        data.parent().unwrap().join(format!(
            "{}-fremd",
            data.file_name().unwrap().to_str().unwrap()
        ))
    }

    #[cfg(unix)]
    #[test]
    fn sicherung_loeschen_bemerkt_ausgetauschten_elternordner() {
        fn tauschen(data: &Path) {
            fs::rename(data.join(SICHERUNGEN), data.join("sicherungen-echt")).unwrap();
            std::os::unix::fs::symlink(fall_c_fremd(data), data.join(SICHERUNGEN)).unwrap();
        }
        fall_c("loeschen-fall-c", tauschen);
    }

    /// Dasselbe mit einer echten Junction (`mklink /J`).
    #[cfg(windows)]
    #[test]
    fn sicherung_loeschen_bemerkt_ausgetauschte_junction() {
        fn tauschen(data: &Path) {
            fs::rename(data.join(SICHERUNGEN), data.join("sicherungen-echt")).unwrap();
            let status = std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(data.join(SICHERUNGEN))
                .arg(fall_c_fremd(data))
                .status()
                .unwrap();
            assert!(status.success());
        }
        fall_c("loeschen-fall-c-junction", tauschen);
    }

    /// Pfadformen unter Windows: lokale Ordner (auch relativ) sind kein Netzlaufwerk;
    /// UNC und unklare Präfixe werden mit Ursache abgelehnt.
    #[cfg(windows)]
    #[test]
    fn netzlaufwerk_erkennt_pfadformen() {
        let data = standardlage("netz-pfadformen");
        assert_eq!(netzlaufwerk(&data), None);
        assert_eq!(netzlaufwerk(Path::new(".")), None);
        let unc = netzlaufwerk(Path::new(r"\\server.example\freigabe\daten")).unwrap();
        assert!(unc.contains("UNC"), "{unc}");
        let geraet = netzlaufwerk(Path::new(r"\\.\PhysicalDrive0")).unwrap();
        assert!(geraet.contains("Pfadform"), "{geraet}");
        let meldung = ordner_kennung(Path::new(r"\\server.example\freigabe")).unwrap_err();
        assert!(
            meldung.contains("Bitte die Sicherung lokal löschen"),
            "{meldung}"
        );
        assert!(meldung.contains("Ursache: UNC"), "{meldung}");
        ordner_kennung(&data).unwrap();
    }

    /// Ein Rest wird nur mit passender Löschmarke entfernt; ohne Marke, mit falscher
    /// Marke oder mit Marke zu einem anderen Manifest bleibt er liegen.
    #[test]
    fn rest_nur_mit_passender_loeschmarke_entfernt() {
        let data = standardlage("loeschen-rest");
        migrieren(&data, &optionen()).unwrap();
        let name = sicherungen_auflisten(&data)[0].name.clone();
        let rest = |n: char, marke: Option<&str>| {
            let rname = format!("{LOESCHEN_PRAEFIX}{}", n.to_string().repeat(32));
            let ort = data.join(SICHERUNGEN).join(&rname);
            fs::create_dir_all(&ort).unwrap();
            fs::write(ort.join(MANIFEST), b"{\"x\":1}").unwrap();
            fs::write(ort.join("inhalt.txt"), b"x").unwrap();
            if let Some(m) = marke {
                fs::write(ort.join(LOESCHMARKE), m).unwrap();
            }
            (rname, ort)
        };
        let (_, ohne) = rest('a', None);
        let (_, falsch) = rest('b', Some("0000"));
        let (_, fremd_name) = rest('c', Some(&loeschmarke("anderer", b"{\"x\":1}")));
        let passend_name = format!("{LOESCHEN_PRAEFIX}{}", "d".repeat(32));
        let (_, passend) = rest('d', Some(&loeschmarke(&passend_name, b"{\"x\":1}")));
        // Regulärer Sicherungsname mit stehengebliebener, zu ihm passender Marke: nie Rest.
        let regulaer_name = format!("{SICHERUNG_PRAEFIX}1");
        let regulaer = data.join(SICHERUNGEN).join(&regulaer_name);
        fs::create_dir_all(&regulaer).unwrap();
        fs::write(regulaer.join(MANIFEST), b"{\"x\":1}").unwrap();
        fs::write(
            regulaer.join(LOESCHMARKE),
            loeschmarke(&regulaer_name, b"{\"x\":1}"),
        )
        .unwrap();
        // Privates Präfix, aber keine 32 Hex: nie Rest, auch mit passender Marke.
        let kurz_name = format!("{LOESCHEN_PRAEFIX}123");
        let kurz = data.join(SICHERUNGEN).join(&kurz_name);
        fs::create_dir_all(&kurz).unwrap();
        fs::write(kurz.join(MANIFEST), b"{\"x\":1}").unwrap();
        fs::write(
            kurz.join(LOESCHMARKE),
            loeschmarke(&kurz_name, b"{\"x\":1}"),
        )
        .unwrap();
        sicherung_loeschen(&data, &name).unwrap();
        assert!(ohne.join("inhalt.txt").is_file());
        assert!(regulaer.join(MANIFEST).is_file());
        assert!(kurz.join(MANIFEST).is_file());
        assert!(falsch.join("inhalt.txt").is_file());
        assert!(fremd_name.join("inhalt.txt").is_file());
        assert!(!passend.exists());
    }

    #[test]
    fn abgeschlossenes_journal_vervollstaendigt_das_kennzeichen() {
        let data = standardlage("kennzeichen");
        migrieren(&data, &optionen()).unwrap();
        // Abbruch zwischen Journal-Abschluss und Kennzeichen (älterer Stand).
        layout_setzen(&data, 1).unwrap();
        wiederanlauf(&data, &optionen()).unwrap();
        assert_eq!(layout(&data), 2);
    }

    #[test]
    fn zurueckspielen_prueft_den_platz_vorher() {
        let data = standardlage("zurueck-platz");
        migrieren(&data, &optionen()).unwrap();
        let name = sicherungen_auflisten(&data)[0].name.clone();
        let index_vorher = index_roh(&data);
        let fehler = sicherung_zurueckspielen(&data, &name, Some(10)).unwrap_err();
        assert!(fehler.contains("Zu wenig freier Speicher"), "{fehler}");
        assert_eq!(index_roh(&data), index_vorher);
        assert!(data.join("library/book_1/fassungen/page_1").is_dir());
    }

    #[test]
    fn index_zusammenfuehren_behaelt_neue_seiten_und_setzt_buch_nach_ort() {
        let aktuell = serde_json::json!({ "pages": [
            { "id": "page_1", "title": "Jetzt", "buch": "book_1" },
            { "id": "page_3", "title": "Neu" },
        ]});
        let gesichert = serde_json::json!({ "pages": [
            { "id": "page_1", "title": "Damals" },
            { "id": "page_2", "title": "Zwei" },
        ]});
        let mut seiten = BTreeMap::new();
        seiten.insert("page_1".to_string(), None);
        seiten.insert("page_2".to_string(), Some("book_1".to_string()));
        let neu = index_zusammenfuehren(Some(aktuell), Some(gesichert), &seiten).unwrap();
        let p = neu["pages"].as_array().unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0]["title"], "Damals");
        assert!(p[0].get("buch").is_none());
        assert_eq!(p[1]["id"], "page_3");
        assert_eq!(p[2]["id"], "page_2");
        assert_eq!(p[2]["buch"], "book_1");
        assert!(index_zusammenfuehren(None, None, &BTreeMap::new()).is_none());
    }

    #[test]
    fn geloeschte_seite_haelt_das_journal_nicht_an() {
        let data = standardlage("geloescht");
        verschieben(&data, "page_1", "book_1");
        index_schreiben(&data, &[("page_1", Some("book_1")), ("page_9", None)]);
        // page_2 ist gelöscht: nicht im Index, kein Ordner.
        fs::remove_dir_all(data.join("projects/page_2")).unwrap();
        journal_mit(
            &data,
            Phase::Gesichert,
            vec![
                schritt("page_1", "book_1", SchrittStatus::Eingetragen),
                schritt("page_2", "book_1", SchrittStatus::Offen),
            ],
        );
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert!(!b.angehalten, "{:?}", b.probleme);
        assert_eq!(b.phase, Some(Phase::Fertig));
        assert!(
            b.hinweise.iter().any(|h| h.contains("page_2")),
            "{:?}",
            b.hinweise
        );
        assert!(aktives_journal(&data).is_none());
        // Steht die Seite noch im Index, fehlt sie aber, bleibt es ein Problem.
        let data = standardlage("geloescht-nicht");
        fs::remove_dir_all(data.join("projects/page_2")).unwrap();
        journal_mit(
            &data,
            Phase::Gesichert,
            vec![schritt("page_2", "book_1", SchrittStatus::Offen)],
        );
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert!(b.angehalten);
        assert_eq!(b.probleme[0].art, ProblemArt::Fehlt);
    }

    #[test]
    fn papierkorb_kaskade_blendet_seiten_aus_und_loescht_sie_endgueltig() {
        let data = standardlage("kaskade");
        migrieren(&data, &optionen()).unwrap();
        assert!(!buch_im_papierkorb(&data, "book_1"));
        // In den Papierkorb (wie library::trash_locked mit Kaskade): Ordner samt Fassungen.
        let korb = data.join(BIBLIOTHEK).join(BIBLIOTHEK_PAPIERKORB);
        fs::create_dir_all(&korb).unwrap();
        fs::rename(data.join("library/book_1"), korb.join("book_1")).unwrap();
        assert!(buch_im_papierkorb(&data, "book_1"));
        assert!(!buch_im_papierkorb(&data, "../book_1"));
        assert_eq!(
            fassungen_von(&korb.join("book_1")),
            vec!["page_1", "page_2"]
        );
        // Die Seiten bleiben im Index, sind aber nicht auffindbar (Sync hält sie zurück).
        let karte = Fundortkarte::default();
        assert_eq!(
            aufloesen(&data, "page_1", Some("book_1"), &karte).unwrap(),
            Aufloesung::Fehlt
        );
        assert_eq!(
            index_buch(&data, "page_1").unwrap().as_deref(),
            Some("book_1")
        );
        // Endgültig: Ordner weg, genau diese Seiten aus dem Index, lose Seite bleibt.
        assert_eq!(
            buch_endgueltig_loeschen(&data, &korb.join("book_1")).unwrap(),
            2
        );
        assert!(!korb.join("book_1").exists());
        let index: serde_json::Value =
            serde_json::from_slice(&fs::read(index_pfad(&data)).unwrap()).unwrap();
        let ids: Vec<&str> = index["pages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p["id"].as_str())
            .collect();
        assert_eq!(ids, vec!["page_9"]);
        assert!(data.join("projects/page_9").is_dir());
        // Fehlender Ordner: nichts zu tun.
        assert_eq!(
            buch_endgueltig_loeschen(&data, &korb.join("book_1")).unwrap(),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn endgueltig_loeschen_folgt_keiner_verknuepfung() {
        let data = standardlage("kaskade-umweg");
        let fremd = data.parent().unwrap().join("kaskade-umweg-fremd");
        let _ = fs::remove_dir_all(&fremd);
        fs::create_dir_all(fremd.join("fassungen/page_1")).unwrap();
        let korb = data.join(BIBLIOTHEK).join(BIBLIOTHEK_PAPIERKORB);
        fs::create_dir_all(&korb).unwrap();
        std::os::unix::fs::symlink(&fremd, korb.join("book_7")).unwrap();
        assert!(buch_endgueltig_loeschen(&data, &korb.join("book_7")).is_err());
        assert!(fremd.join("fassungen/page_1").is_dir());
        assert_eq!(index_buch(&data, "page_1").unwrap(), None);
        assert!(index_laden(&data).unwrap().unwrap()["pages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "page_1"));
    }

    #[test]
    fn neue_seiten_kommen_direkt_in_den_buch_ordner() {
        let data = standardlage("einlagern");
        // Altes Layout: nichts bewegen.
        assert!(seiten_einlagern(&data, "book_1").unwrap().is_empty());
        assert!(data.join("projects/page_1").is_dir());
        migrieren(&data, &optionen()).unwrap();
        assert_eq!(layout(&data), 2);
        // Neues Buch mit neuer Textseite (wie der Import nach library_save).
        buch_anlegen(&data, "book_2", "page_5", &["page_5", "page_6"]);
        seite_anlegen(&data.join(PROJEKTE).join("page_5"), "neu");
        seite_anlegen(&data.join(PROJEKTE).join("page_6"), "fassung");
        index_schreiben(
            &data,
            &[
                ("page_1", Some("book_1")),
                ("page_2", Some("book_1")),
                ("page_9", None),
                ("page_5", None),
                ("page_6", None),
            ],
        );
        assert_eq!(
            seiten_einlagern(&data, "book_2").unwrap(),
            vec!["page_5", "page_6"]
        );
        assert!(data
            .join("library/book_2/fassungen/page_5/state.json")
            .is_file());
        assert!(!data.join("projects/page_5").exists());
        assert_eq!(
            index_buch(&data, "page_5").unwrap().as_deref(),
            Some("book_2")
        );
        // Zweiter Aufruf: nichts mehr zu tun.
        assert!(seiten_einlagern(&data, "book_2").unwrap().is_empty());
        // Lose Seite bleibt; ungültiges Buch: nichts.
        assert!(data.join("projects/page_9").is_dir());
        assert!(seiten_einlagern(&data, "../book_2").unwrap().is_empty());
    }

    #[test]
    fn einlagern_uebergeht_geteilte_belegte_und_offene_faelle() {
        let data = standardlage("einlagern-grenzen");
        migrieren(&data, &optionen()).unwrap();
        // page_7 wird von zwei Büchern genannt: bleibt in projects/.
        buch_anlegen(&data, "book_3", "page_7", &["page_7"]);
        buch_anlegen(&data, "book_4", "page_7", &["page_7"]);
        seite_anlegen(&data.join(PROJEKTE).join("page_7"), "geteilt");
        // page_8: Ziel im Buch-Ordner ist schon belegt; nichts überschrieben.
        buch_anlegen(&data, "book_5", "page_8", &["page_8"]);
        seite_anlegen(&data.join(PROJEKTE).join("page_8"), "alt");
        seite_anlegen(&data.join("library/book_5/fassungen/page_8"), "belegt");
        index_schreiben(
            &data,
            &[("page_7", None), ("page_8", None), ("page_9", None)],
        );
        assert!(seiten_einlagern(&data, "book_3").unwrap().is_empty());
        assert!(data.join("projects/page_7").is_dir());
        assert!(seiten_einlagern(&data, "book_5").unwrap().is_empty());
        assert!(data.join("projects/page_8").is_dir());
        assert_eq!(
            index_buch(&data, "page_8").unwrap().as_deref(),
            Some("book_5")
        );
        // Offene Neuordnung: nichts bewegen.
        buch_anlegen(&data, "book_6", "page_4", &["page_4"]);
        seite_anlegen(&data.join(PROJEKTE).join("page_4"), "wartet");
        journal_mit(&data, Phase::Gesichert, vec![]);
        assert!(seiten_einlagern(&data, "book_6").unwrap().is_empty());
        assert!(data.join("projects/page_4").is_dir());
    }

    #[test]
    fn dateien_im_buch_ordner_als_baum() {
        let data = standardlage("dateien");
        migrieren(&data, &optionen()).unwrap();
        let buch = data.join("library/book_1");
        fs::create_dir_all(buch.join("audio")).unwrap();
        for i in 0..20 {
            fs::write(buch.join(format!("audio/teil-{i:02}.mp3")), b"abc").unwrap();
        }
        fs::write(buch.join("cover.jpg"), b"12345").unwrap();
        fs::write(buch.join(".entfernt"), b"0").unwrap();
        let liste = buch_dateien(&data, "book_1").unwrap();
        let rels: Vec<&str> = liste.iter().map(|e| e.rel.as_str()).collect();
        // Ordner zuerst, dann Dateien; versteckte Einträge fehlen.
        assert_eq!(rels[0], "audio");
        assert!(rels.contains(&"fassungen"));
        assert!(rels.contains(&"fassungen/page_1"));
        assert!(rels.contains(&"fassungen/page_1/state.json"));
        assert!(rels.contains(&"buch.json"));
        assert!(!rels.iter().any(|r| r.starts_with('.') || r.contains("/.")));
        let audio = &liste[0];
        assert!(audio.ordner && audio.zusammengefasst);
        assert_eq!((audio.dateien, audio.bytes), (20, 60));
        assert!(!rels.iter().any(|r| r.starts_with("audio/")));
        // Ebene 3 (fassungen/page_1/ki) ist zusammengefasst.
        let ki = liste
            .iter()
            .find(|e| e.rel == "fassungen/page_1/ki")
            .unwrap();
        assert!(ki.zusammengefasst && ki.tiefe == 2 && ki.dateien == 1);
        assert!(buch_dateien(&data, "../book_1").is_err());
        assert!(buch_dateien(&data, "book_404").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn dateien_im_buch_ordner_folgt_keiner_verknuepfung() {
        let data = standardlage("dateien-umweg");
        let fremd = data.parent().unwrap().join("dateien-umweg-fremd");
        let _ = fs::remove_dir_all(&fremd);
        fs::create_dir_all(&fremd).unwrap();
        fs::write(fremd.join("geheim.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(&fremd, data.join("library/book_1/umweg")).unwrap();
        let liste = buch_dateien(&data, "book_1").unwrap();
        assert!(!liste.iter().any(|e| e.rel.starts_with("umweg")));
        fs::rename(data.join("library/book_1"), data.join("library-echt")).unwrap();
        std::os::unix::fs::symlink(data.join("library-echt"), data.join("library/book_1")).unwrap();
        assert!(buch_dateien(&data, "book_1").is_err());
    }

    #[test]
    fn papierkorb_wartet_auf_offene_neuordnung() {
        let data = standardlage("kaskade-journal");
        buch_ordner_bewegbar(&data).unwrap();
        journal_mit(&data, Phase::Gesichert, vec![]);
        assert!(buch_ordner_bewegbar(&data).is_err());
    }

    #[test]
    fn fassungen_auslagern_rettet_seiten_vor_dem_loeschen_des_buchs() {
        let data = standardlage("auslagern");
        migrieren(&data, &optionen()).unwrap();
        let buch_dir = data.join("library/book_1");
        assert_eq!(fassungen_auslagern(&data, &buch_dir).unwrap(), 2);
        assert!(data.join("projects/page_1/state.json").is_file());
        assert!(data.join("projects/page_2/ki/dokumente/d1.json").is_file());
        assert!(!buch_dir.join(FASSUNGEN).exists());
        assert_eq!(index_buch(&data, "page_1").unwrap(), None);
        assert_eq!(pfad(&data, "page_2").unwrap(), data.join("projects/page_2"));
        // Ein zweiter Aufruf hat nichts mehr zu tun.
        assert_eq!(fassungen_auslagern(&data, &buch_dir).unwrap(), 0);
    }

    #[test]
    fn fassungen_auslagern_haelt_bei_offenem_oder_kaputtem_journal_auch_ohne_fassungen() {
        let data = standardlage("auslagern-leer");
        // Journal offen, Buch noch ohne Fassungen im Buch-Ordner.
        journal_mit(
            &data,
            Phase::Gesichert,
            vec![schritt("page_1", "book_1", SchrittStatus::Offen)],
        );
        let fehler = fassungen_auslagern(&data, &data.join("library/book_1")).unwrap_err();
        assert!(fehler.contains("nicht abgeschlossen"), "{fehler}");
        fs::write(journal_pfad(&data), "{kaputt").unwrap();
        let fehler = fassungen_auslagern(&data, &data.join("library/book_1")).unwrap_err();
        assert!(fehler.contains("nicht prüfbar"), "{fehler}");
    }

    #[test]
    fn seite_in_einem_anderen_buch_gilt_nicht_als_geloescht() {
        let data = standardlage("anderswo");
        buch_anlegen(&data, "book_2", "page_8", &[]);
        // page_2 liegt (unerwartet) im Buch-Ordner von book_2, nicht im Index.
        let ziel = data.join("library/book_2/fassungen/page_2");
        fs::create_dir_all(ziel.parent().unwrap()).unwrap();
        fs::rename(data.join("projects/page_2"), &ziel).unwrap();
        index_schreiben(&data, &[("page_1", None), ("page_9", None)]);
        journal_mit(
            &data,
            Phase::Gesichert,
            vec![schritt("page_2", "book_1", SchrittStatus::Offen)],
        );
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert!(b.angehalten, "nicht übersprungen: {:?}", b.hinweise);
        assert_eq!(b.probleme[0].art, ProblemArt::Unerwartet);
        assert!(ziel.is_dir());
    }

    #[test]
    fn fassungen_auslagern_ueberschreibt_nichts_und_wartet_auf_das_journal() {
        let data = standardlage("auslagern-belegt");
        migrieren(&data, &optionen()).unwrap();
        let buch_dir = data.join("library/book_1");
        fs::create_dir_all(data.join("projects/page_2")).unwrap();
        let index_vorher = index_roh(&data);
        let fehler = fassungen_auslagern(&data, &buch_dir).unwrap_err();
        assert!(fehler.contains("projects/page_2"), "{fehler}");
        assert!(buch_dir.join("fassungen/page_1").is_dir(), "nichts bewegt");
        assert!(!data.join("projects/page_1").exists());
        assert_eq!(index_roh(&data), index_vorher);

        let data = standardlage("auslagern-journal");
        migrieren(&data, &optionen()).unwrap();
        let mut j = journal_laden(&data).unwrap().unwrap();
        j.phase = Phase::Rueckfall;
        journal_schreiben(&data, &j).unwrap();
        let fehler = fassungen_auslagern(&data, &data.join("library/book_1")).unwrap_err();
        assert!(fehler.contains("nicht abgeschlossen"), "{fehler}");
        assert!(data.join("library/book_1/fassungen/page_1").is_dir());
    }

    #[test]
    fn zurueckspielen_verweigert_eine_beschaedigte_sicherung() {
        let data = standardlage("zurueck-kaputt");
        migrieren(&data, &optionen()).unwrap();
        let name = sicherungen_auflisten(&data)[0].name.clone();
        fs::write(
            data.join(SICHERUNGEN)
                .join(&name)
                .join("projects/page_1/state.json"),
            "anders",
        )
        .unwrap();
        let index_vorher = index_roh(&data);
        assert!(sicherung_zurueckspielen(&data, &name, None).is_err());
        assert_eq!(index_roh(&data), index_vorher);
        assert!(!data.join(SICHERUNGEN).join(PAPIERKORB).exists());
        assert!(sicherung_zurueckspielen(&data, "../projects", None).is_err());
    }

    // ----- Wiederanlauf: je Zeile der Tabelle ------------------------------

    #[test]
    fn tabelle_doppelung_hat_vorrang_bei_offen() {
        let data = standardlage("dopp-offen");
        // Seite liegt an beiden Orten (z. B. aus der Sicherung zurückgespielt).
        seite_anlegen(&data.join("library/book_1/fassungen/page_1"), "kopie");
        journal_mit(
            &data,
            Phase::Gesichert,
            vec![
                schritt("page_1", "book_1", SchrittStatus::Offen),
                schritt("page_2", "book_1", SchrittStatus::Offen),
            ],
        );
        let index_vorher = index_roh(&data);
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert!(b.angehalten);
        assert_eq!(b.erledigt, 0);
        assert_eq!(b.probleme[0].art, ProblemArt::Doppelt);
        assert_eq!(b.probleme[0].pfade.len(), 2);
        // Nichts verschoben, auch nicht die unbeteiligte page_2, Index unverändert.
        assert!(data.join("projects/page_2").is_dir());
        assert!(data.join("projects/page_1").is_dir());
        assert_eq!(index_roh(&data), index_vorher);
        assert!(pfad(&data, "page_1")
            .unwrap_err()
            .starts_with("Seite doppelt vorhanden"));
    }

    #[test]
    fn tabelle_doppelung_hat_vorrang_bei_verschoben() {
        let data = standardlage("dopp-verschoben");
        verschieben(&data, "page_1", "book_1");
        seite_anlegen(&data.join("projects/page_1"), "kopie");
        journal_mit(
            &data,
            Phase::Gesichert,
            vec![schritt("page_1", "book_1", SchrittStatus::Verschoben)],
        );
        let index_vorher = index_roh(&data);
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert!(b.angehalten);
        assert_eq!(b.probleme[0].art, ProblemArt::Doppelt);
        assert_eq!(index_roh(&data), index_vorher);
        assert!(data.join("projects/page_1").is_dir());
        assert!(data.join("library/book_1/fassungen/page_1").is_dir());
    }

    #[test]
    fn tabelle_doppelung_ohne_journal() {
        let data = standardlage("dopp-ohne");
        seite_anlegen(&data.join("library/book_1/fassungen/page_1"), "kopie");
        let index_vorher = index_roh(&data);
        let fehler = pfad(&data, "page_1").unwrap_err();
        assert!(fehler.starts_with("Seite doppelt vorhanden"));
        // Auch die Migration beginnt dann nicht.
        let b = migrieren(&data, &optionen()).unwrap();
        assert!(b.angehalten);
        assert_eq!(b.probleme[0].art, ProblemArt::Doppelt);
        assert!(!journal_pfad(&data).exists());
        assert_eq!(index_roh(&data), index_vorher);
        assert!(sicherungen_auflisten(&data).is_empty());
    }

    #[test]
    fn tabelle_geplant_beginnt_die_sicherung_neu() {
        let data = standardlage("geplant");
        journal_mit(
            &data,
            Phase::Geplant,
            vec![
                schritt("page_1", "book_1", SchrittStatus::Offen),
                schritt("page_2", "book_1", SchrittStatus::Offen),
            ],
        );
        // Ein Rest einer abgebrochenen Sicherung bleibt unberührt.
        fs::create_dir_all(data.join("sicherungen/buch-ordner-1")).unwrap();
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert_eq!(b.phase, Some(Phase::Fertig), "{:?}", b.probleme);
        let j = journal_laden(&data).unwrap().unwrap();
        assert_ne!(j.sicherung, "sicherungen/buch-ordner-1");
        assert!(sicherung_pruefen(&absolut(&data, &j.sicherung)).is_ok());
        assert!(data.join("library/book_1/fassungen/page_2").is_dir());
    }

    #[test]
    fn tabelle_offen_unter_von_wird_normal_fortgesetzt() {
        let data = standardlage("offen-von");
        journal_mit(
            &data,
            Phase::Gesichert,
            vec![schritt("page_1", "book_1", SchrittStatus::Offen)],
        );
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert_eq!(b.phase, Some(Phase::Fertig));
        assert_eq!(b.erledigt, 1);
        assert!(data.join("library/book_1/fassungen/page_1").is_dir());
        assert_eq!(
            index_buch(&data, "page_1").unwrap().as_deref(),
            Some("book_1")
        );
    }

    /// Zwischenfenster Migration: Abbruch nach `rename`, vor dem Journal und dem Index.
    #[test]
    fn tabelle_offen_unter_nach_gilt_als_verschoben() {
        let data = standardlage("offen-nach");
        journal_mit(
            &data,
            Phase::Gesichert,
            vec![schritt("page_1", "book_1", SchrittStatus::Offen)],
        );
        verschieben(&data, "page_1", "book_1");
        // page_path findet die Seite schon am neuen Ort, obwohl der Index noch nichts weiß.
        assert_eq!(
            pfad(&data, "page_1").unwrap(),
            data.join("library/book_1/fassungen/page_1")
        );
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert_eq!(b.phase, Some(Phase::Fertig));
        assert_eq!(
            index_buch(&data, "page_1").unwrap().as_deref(),
            Some("book_1")
        );
        let j = journal_laden(&data).unwrap().unwrap();
        assert_eq!(j.schritte[0].status, SchrittStatus::Eingetragen);
    }

    /// Zwischenfenster Migration: Abbruch nach `verschoben`, vor dem Index-Schreiben.
    #[test]
    fn tabelle_verschoben_setzt_den_index_nach() {
        let data = standardlage("verschoben");
        verschieben(&data, "page_1", "book_1");
        journal_mit(
            &data,
            Phase::Gesichert,
            vec![schritt("page_1", "book_1", SchrittStatus::Verschoben)],
        );
        assert!(pfad(&data, "page_1").is_ok());
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert_eq!(b.phase, Some(Phase::Fertig));
        assert_eq!(b.erledigt, 1);
        assert_eq!(
            index_buch(&data, "page_1").unwrap().as_deref(),
            Some("book_1")
        );
    }

    #[test]
    fn tabelle_weder_von_noch_nach_haelt_an_und_bietet_die_sicherung_an() {
        let data = standardlage("weder");
        fs::remove_dir_all(data.join("projects/page_1")).unwrap();
        journal_mit(
            &data,
            Phase::Gesichert,
            vec![
                schritt("page_1", "book_1", SchrittStatus::Offen),
                schritt("page_2", "book_1", SchrittStatus::Offen),
            ],
        );
        let index_vorher = index_roh(&data);
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert!(b.angehalten);
        assert_eq!(b.probleme.len(), 1);
        assert_eq!(b.probleme[0].art, ProblemArt::Fehlt);
        assert!(b.probleme[0].meldung.contains("sicherungen/buch-ordner-1"));
        assert!(data.join("projects/page_2").is_dir(), "nichts verschoben");
        assert_eq!(index_roh(&data), index_vorher);
        assert_eq!(
            pfad(&data, "page_1").unwrap_err(),
            "Seite nicht gefunden: page_1"
        );
    }

    #[test]
    fn wiederanlauf_ohne_journal_tut_nichts() {
        let data = standardlage("ohne");
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert_eq!(b, Bericht::default());
        assert!(data.join("projects/page_1").is_dir());
    }

    #[test]
    fn manipuliertes_journal_verschiebt_nichts() {
        let data = standardlage("boese");
        let roh = r#"{"version":1,"phase":"gesichert","sicherung":"","schritte":[
            {"seite":"page_1","buch":"book_1","von":"projects/page_1","nach":"../aussen","status":"offen"}]}"#;
        fs::write(journal_pfad(&data), roh).unwrap();
        assert!(wiederanlauf(&data, &optionen()).is_err());
        assert!(data.join("projects/page_1").is_dir());
        // page_path ignoriert das unlesbare Journal und findet die Seite weiter.
        assert!(pfad(&data, "page_1").is_ok());
    }

    // ----- page_path ---------------------------------------------------------

    #[test]
    fn page_path_findet_lose_und_buchseiten() {
        let data = standardlage("pfad");
        assert_eq!(pfad(&data, "page_9").unwrap(), data.join("projects/page_9"));
        assert_eq!(
            pfad(&data, "page_7").unwrap_err(),
            "Seite nicht gefunden: page_7"
        );
        assert!(pfad(&data, "page_..")
            .unwrap_err()
            .contains("Ungültige Seiten-Kennung"));
    }

    /// Abnahme 9: zurückgesicherter Index (Feld `buch` leer), kein Journal, Seite unter
    /// `projects/` und im Buch-Ordner → doppelt mit beiden Pfaden; Gegenprobe nur im
    /// Buch-Ordner → über die Fundortkarte gefunden.
    #[test]
    fn page_path_zurueckgesetzter_index_mit_leerem_buch() {
        let data = standardlage("index-leer");
        migrieren(&data, &optionen()).unwrap();
        index_schreiben(
            &data,
            &[("page_1", None), ("page_2", None), ("page_9", None)],
        );
        fs::remove_file(journal_pfad(&data)).unwrap();
        // Gegenprobe: nur im Buch-Ordner.
        assert_eq!(
            pfad(&data, "page_1").unwrap(),
            data.join("library/book_1/fassungen/page_1")
        );
        // Zusätzlich unter projects/: doppelt mit beiden Pfaden.
        seite_anlegen(&data.join("projects/page_1"), "alt");
        let index_vorher = index_roh(&data);
        let fehler = pfad(&data, "page_1").unwrap_err();
        assert_eq!(
            fehler,
            "Seite doppelt vorhanden: page_1 (projects/page_1, library/book_1/fassungen/page_1)"
        );
        assert_eq!(index_roh(&data), index_vorher);
    }

    #[test]
    fn page_path_findet_seite_ueber_buch_metadaten_ohne_bestandsaufnahme() {
        let data = standardlage("metadaten");
        // Karte aus der Zeit vor dem Umzug (veralteter Cache) – nur buch.json kennt den Ort.
        let karte_alt = Fundortkarte::aufbauen(&data);
        verschieben(&data, "page_2", "book_1");
        assert_eq!(
            seite_pfad(&data, "page_2", None, &karte_alt).unwrap(),
            data.join("library/book_1/fassungen/page_2")
        );
    }

    #[test]
    fn page_path_waehrend_eines_rueckfalls_beide_richtungen() {
        let data = standardlage("pfad-rueckfall");
        migrieren(&data, &optionen()).unwrap();
        // Rückfall mitten im Schritt: zurückverschoben, Index trägt noch das Buch.
        let mut j = journal_laden(&data).unwrap().unwrap();
        j.phase = Phase::Rueckfall;
        journal_schreiben(&data, &j).unwrap();
        let von = data.join("projects/page_1");
        fs::rename(data.join("library/book_1/fassungen/page_1"), &von).unwrap();
        assert_eq!(
            index_buch(&data, "page_1").unwrap().as_deref(),
            Some("book_1")
        );
        assert_eq!(pfad(&data, "page_1").unwrap(), von);
    }

    // ----- Rückfall ----------------------------------------------------------

    #[test]
    fn rueckfall_stellt_den_alten_stand_her() {
        let data = standardlage("rueckfall");
        let state_vorher = fs::read(data.join("projects/page_2/state.json")).unwrap();
        migrieren(&data, &optionen()).unwrap();
        let b = rueckfall(&data).unwrap();
        assert_eq!(b.phase, Some(Phase::Zurueckgenommen));
        assert_eq!(b.erledigt, 2);
        assert!(data.join("projects/page_1").is_dir());
        assert!(
            !data.join("library/book_1/fassungen").exists(),
            "leerer Ordner aufgeräumt"
        );
        assert_eq!(
            fs::read(data.join("projects/page_2/state.json")).unwrap(),
            state_vorher
        );
        assert_eq!(index_buch(&data, "page_1").unwrap(), None);
        assert_eq!(layout(&data), 1);
        let j = journal_laden(&data).unwrap().unwrap();
        assert!(j.schritte.iter().all(|s| s.status == SchrittStatus::Offen));
        // Zweiter Rückfall ändert nichts.
        assert_eq!(rueckfall(&data).unwrap().erledigt, 0);
        // Danach ist eine neue Migration möglich; das alte Journal wird archiviert.
        let neu = migrieren(&data, &optionen()).unwrap();
        assert_eq!(neu.phase, Some(Phase::Fertig));
        let archiv = fs::read_dir(data.join(BIBLIOTHEK))
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".migration-buch-ordner-")
            })
            .count();
        assert_eq!(archiv, 1);
    }

    /// Zwischenfenster Rückfall: Abbruch nach `rename(nach, von)`, vor Journal und Index.
    #[test]
    fn rueckfall_zwischenfenster_wird_beim_start_abgeschlossen() {
        let data = standardlage("rueckfall-fenster");
        migrieren(&data, &optionen()).unwrap();
        let mut j = journal_laden(&data).unwrap().unwrap();
        j.phase = Phase::Rueckfall;
        journal_schreiben(&data, &j).unwrap();
        fs::rename(
            data.join("library/book_1/fassungen/page_2"),
            data.join("projects/page_2"),
        )
        .unwrap();
        let b = wiederanlauf(&data, &optionen()).unwrap();
        assert_eq!(b.phase, Some(Phase::Zurueckgenommen), "{:?}", b.probleme);
        assert!(data.join("projects/page_1").is_dir());
        assert!(data.join("projects/page_2").is_dir());
        assert_eq!(index_buch(&data, "page_2").unwrap(), None);
    }

    #[test]
    fn rueckfall_ueberschreibt_kein_vorhandenes_ziel() {
        let data = standardlage("rueckfall-ziel");
        migrieren(&data, &optionen()).unwrap();
        // Unter projects/ ist inzwischen etwas Gleichnamiges entstanden.
        seite_anlegen(&data.join("projects/page_1"), "neu");
        let b = rueckfall(&data).unwrap();
        assert!(b.angehalten);
        assert_eq!(b.probleme[0].art, ProblemArt::Doppelt);
        assert_eq!(
            fs::read_to_string(data.join("projects/page_1/state.json")).unwrap(),
            "{\"text\":\"neu\"}"
        );
        assert!(data.join("library/book_1/fassungen/page_1").is_dir());
        assert!(
            data.join("library/book_1/fassungen/page_2").is_dir(),
            "nichts bewegt"
        );
        assert_eq!(journal_laden(&data).unwrap().unwrap().phase, Phase::Fertig);
    }

    #[test]
    fn migration_ueberschreibt_kein_vorhandenes_ziel() {
        let data = standardlage("ziel");
        let fremd = data.join("library/book_1/fassungen/page_1");
        seite_anlegen(&fremd, "fremd");
        let b = migrieren(&data, &optionen()).unwrap();
        assert!(b.angehalten);
        assert_eq!(
            fs::read_to_string(fremd.join("state.json")).unwrap(),
            "{\"text\":\"fremd\"}"
        );
        assert!(data.join("projects/page_1").is_dir());
    }

    #[test]
    fn rueckfall_ohne_journal_ist_ein_fehler() {
        let data = standardlage("rueckfall-leer");
        assert!(rueckfall(&data).is_err());
    }

    // ----- Status, Planung, Index ------------------------------------------

    #[test]
    fn status_zeigt_anzahl_und_platz() {
        let data = standardlage("status");
        let s = status(&data, &|_| Some(1 << 40));
        assert_eq!(s.layout, 1);
        assert_eq!(s.phase, None);
        assert_eq!(s.zu_verschieben, 2);
        assert!(s.platz_benoetigt > 1 << 20);
        assert_eq!(s.platz_frei, Some(1 << 40));
        migrieren(&data, &optionen()).unwrap();
        let s = status(&data, &|_| None);
        assert_eq!(s.layout, 2);
        assert_eq!(s.phase, Some(Phase::Fertig));
        assert_eq!(s.zu_verschieben, 0);
        assert_eq!(s.platz_benoetigt, 0);
    }

    #[test]
    fn planung_ueberspringt_fehlende_und_mehrfach_zugeordnete_seiten() {
        let data = standardlage("plan");
        buch_anlegen(&data, "book_2", "page_2", &[]);
        buch_anlegen(&data, "book_3", "page_77", &[]);
        let plan = planen(&data).unwrap();
        let seiten: Vec<&str> = plan.schritte.iter().map(|s| s.seite.as_str()).collect();
        assert_eq!(seiten, vec!["page_1"]);
        assert_eq!(plan.hinweise.len(), 2, "{:?}", plan.hinweise);
        assert!(plan.probleme.is_empty());
    }

    #[test]
    fn index_behaelt_alle_anderen_felder() {
        let data = standardlage("felder");
        fs::write(
            data.join("projects/index.json"),
            r#"{"pages":[{"id":"page_1","title":"T","modified_ms":5.0,"preview":"x","neu":1}]}"#,
        )
        .unwrap();
        assert!(index_buch_setzen(&data, "page_1", Some("book_1")).unwrap());
        let v: serde_json::Value = serde_json::from_slice(&index_roh(&data)).unwrap();
        assert_eq!(v["pages"][0]["title"], "T");
        assert_eq!(v["pages"][0]["neu"], 1);
        assert_eq!(v["pages"][0]["buch"], "book_1");
        assert!(index_buch_setzen(&data, "page_1", None).unwrap());
        let v: serde_json::Value = serde_json::from_slice(&index_roh(&data)).unwrap();
        assert!(v["pages"][0].get("buch").is_none());
        assert!(!index_buch_setzen(&data, "page_5", Some("book_1")).unwrap());
    }

    #[test]
    fn bestand_fuer_den_index_neuaufbau() {
        let data = standardlage("bestand");
        migrieren(&data, &optionen()).unwrap();
        let bestand = seiten_bestand(&data);
        assert!(bestand.contains(&("page_1".into(), Some("book_1".into()))));
        assert!(bestand.contains(&("page_9".into(), None)));
    }

    #[test]
    fn atomar_geschrieben_ohne_reste() {
        let data = datenordner("atomar");
        let pfad = data.join(BIBLIOTHEK).join(JOURNAL);
        schreibe_atomar(&pfad, b"eins").unwrap();
        schreibe_atomar(&pfad, b"zwei").unwrap();
        assert_eq!(fs::read(&pfad).unwrap(), b"zwei");
        let reste = fs::read_dir(data.join(BIBLIOTHEK))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("tmp-"))
            .count();
        assert_eq!(reste, 0);
    }

    // ----- Löschkaskade und Namen -------------------------------------------

    #[test]
    fn papierkorb_nimmt_die_fassungen_mit_und_bringt_sie_zurueck() {
        let data = standardlage("kaskade");
        migrieren(&data, &optionen()).unwrap();
        assert_eq!(kaskade_seiten(&data, "book_1"), vec!["page_1", "page_2"]);
        assert!(kaskade_seiten(&data, "../x").is_empty());
        // So legt die Bibliothek ein Buch in den Papierkorb: der ganze Ordner wandert.
        let korb = data.join("library/.papierkorb");
        fs::create_dir_all(&korb).unwrap();
        fs::rename(data.join("library/book_1"), korb.join("book_1")).unwrap();
        assert_eq!(
            pfad(&data, "page_1").unwrap_err(),
            "Seite nicht gefunden: page_1"
        );
        assert!(Fundortkarte::aufbauen(&data).orte("page_1").is_empty());
        fs::rename(korb.join("book_1"), data.join("library/book_1")).unwrap();
        assert_eq!(
            pfad(&data, "page_1").unwrap(),
            data.join("library/book_1/fassungen/page_1")
        );
    }

    #[test]
    fn namen_werden_bereinigt() {
        assert_eq!(bereinigter_name("Bericht.pdf").unwrap(), "Bericht.pdf");
        assert_eq!(bereinigter_name("CON.txt").unwrap(), "_CON.txt");
        assert_eq!(bereinigter_name("nul").unwrap(), "_nul");
        assert_eq!(bereinigter_name("a:b*c?.txt").unwrap(), "a_b_c_.txt");
        assert_eq!(bereinigter_name("Ende. . ").unwrap(), "Ende");
        assert_eq!(bereinigter_name("zeile\nzwei").unwrap(), "zeile_zwei");
        assert!(bereinigter_name("..").is_err());
        assert!(bereinigter_name(".").is_err());
        assert!(bereinigter_name("  ").is_err());
        let lang = format!("{}.pdf", "x".repeat(300));
        let kurz = bereinigter_name(&lang).unwrap();
        assert_eq!(kurz.chars().count(), MAX_NAME);
        assert!(kurz.ends_with(".pdf"));
        // Kein Ausbruch: das Ergebnis ist genau eine normale Pfadkomponente.
        for boese in ["..\\x", "../x", "a/../../b", "C:\\Windows\\x"] {
            let n = bereinigter_name(boese).unwrap();
            let teile: Vec<_> = Path::new(&n).components().collect();
            assert_eq!(teile.len(), 1, "{boese} → {n}");
            assert!(matches!(teile[0], Component::Normal(_)), "{boese} → {n}");
        }
    }

    #[test]
    fn namenskollision_ohne_gross_klein_und_nach_nfc() {
        let data = datenordner("namen");
        let dir = data.join("anhaenge");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Bericht.pdf"), "a").unwrap();
        assert_eq!(freier_name(&dir, "bericht.PDF").unwrap(), "bericht (2).PDF");
        fs::write(dir.join("bericht (2).PDF"), "b").unwrap();
        assert_eq!(freier_name(&dir, "BERICHT.pdf").unwrap(), "BERICHT (3).pdf");
        // NFC: „Müller“ zusammengesetzt und zerlegt sind derselbe Name.
        fs::write(dir.join("M\u{00fc}ller.txt"), "c").unwrap();
        assert_eq!(
            freier_name(&dir, "Mu\u{0308}ller.txt").unwrap(),
            "M\u{00fc}ller (2).txt"
        );
        assert_eq!(freier_name(&dir, "frei.txt").unwrap(), "frei.txt");
        // Vorhandene Dateien bleiben unberührt.
        assert_eq!(fs::read_to_string(dir.join("Bericht.pdf")).unwrap(), "a");
        // Ein langer Name behält auch mit Zusatz die Höchstlänge.
        let lang = format!("{}.txt", "y".repeat(200));
        let erster = bereinigter_name(&lang).unwrap();
        fs::write(dir.join(&erster), "d").unwrap();
        let zweiter = freier_name(&dir, &lang).unwrap();
        assert!(zweiter.chars().count() <= MAX_NAME);
        assert!(zweiter.ends_with(" (2).txt"));
    }
}
