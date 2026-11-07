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

fn jetzt_ms() -> u64 {
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
    ordner_sync(dir);
    Ok(())
}

/// Ordnereintrag dauerhaft machen. Unter Windows lässt sich ein Ordner so nicht öffnen;
/// dort genügt das fsync der Datei.
fn ordner_sync(dir: &Path) {
    #[cfg(unix)]
    {
        if let Ok(f) = fs::File::open(dir) {
            let _ = f.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
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

/// Ein Schritt des Tauschs beim Zurückspielen, für die Rücknahme bei einem Fehler.
enum Tausch {
    /// Ordner aus dem Datenordner in den Papierkorb gelegt.
    Weg { ort: PathBuf, korb: PathBuf },
    /// Ordner aus dem Zwischenordner an seinen Ort gelegt.
    Hin { stufe: PathBuf, ort: PathBuf },
    /// Datei ersetzt; die alte Fassung liegt als Kopie im Papierkorb (`None`: es gab keine).
    Ersetzt { ort: PathBuf, alt: Option<PathBuf> },
}

fn tausch_zuruecknehmen(log: &[Tausch]) -> Vec<String> {
    let mut fehler = Vec::new();
    for t in log.iter().rev() {
        let r = match t {
            Tausch::Hin { stufe, ort } => fs::rename(ort, stufe).map_err(|e| e.to_string()),
            Tausch::Weg { ort, korb } => fs::rename(korb, ort).map_err(|e| e.to_string()),
            Tausch::Ersetzt {
                ort,
                alt: Some(alt),
            } => fs::read(alt)
                .map_err(|e| e.to_string())
                .and_then(|b| schreibe_atomar(ort, &b)),
            Tausch::Ersetzt { ort, alt: None } => fs::remove_file(ort).map_err(|e| e.to_string()),
        };
        if let Err(e) = r {
            fehler.push(e);
        }
    }
    fehler
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
/// 4. Scheitert ein Schritt, wird der Tausch zurückgenommen.
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
    let manifest = sicherung_pruefen(&dir)?;
    let mut einheiten = BTreeSet::new();
    for rel in manifest
        .ordner
        .iter()
        .chain(manifest.dateien.iter().map(|d| &d.pfad))
    {
        einheiten.insert(einheit(rel)?);
    }
    let seiten: BTreeMap<String, Option<String>> = einheiten
        .iter()
        .filter_map(|e| seite_der_einheit(e))
        .collect();
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
    let gesichert = if einheiten.contains(&index_rel) {
        Some(
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(absolut(&dir, &index_rel)).map_err(|e| e.to_string())?,
            )
            .map_err(|e| format!("Seitenindex der Sicherung unlesbar: {e}"))?,
        )
    } else {
        None
    };
    let index_neu = index_zusammenfuehren(aktuell, gesichert, &seiten);

    // 2. Zwischenordner
    let stufe = neuer_ordner(&data.join(SICHERUNGEN), ".zurueck-")?;
    let vorbereitet = (|| -> Result<(), String> {
        for rel in &manifest.ordner {
            fs::create_dir_all(absolut(&stufe, rel)).map_err(|e| format!("{rel}: {e}"))?;
        }
        for datei in &manifest.dateien {
            if datei.pfad == index_rel {
                continue;
            }
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
        Ok(())
    })();
    if let Err(e) = vorbereitet {
        let _ = fs::remove_dir_all(&stufe);
        return Err(format!("Zurückspielen nicht begonnen: {e}"));
    }

    // 3. Was weichen muss: vorhandene Ordner-Einheiten und weitere Orte derselben Seiten.
    let karte = Fundortkarte::aufbauen(data);
    let mut weichen: BTreeSet<String> = einheiten
        .iter()
        .filter(|e| !datei_einheit(e) && vorhanden(&absolut(data, e)))
        .cloned()
        .collect();
    for seite in seiten.keys() {
        let mut orte = karte.orte(seite);
        orte.push(rel_von(seite));
        for ort in orte {
            if vorhanden(&absolut(data, &ort)) {
                weichen.insert(ort);
            }
        }
    }
    let dateien_ersetzen: Vec<&String> = einheiten
        .iter()
        .filter(|e| datei_einheit(e) && vorhanden(&absolut(&stufe, e)))
        .collect();
    let braucht_korb = !weichen.is_empty()
        || dateien_ersetzen
            .iter()
            .any(|e| vorhanden(&absolut(data, e)));
    let korb = if braucht_korb {
        let korb = match neuer_ordner(&data.join(SICHERUNGEN).join(PAPIERKORB), "") {
            Ok(k) => k,
            Err(e) => {
                let _ = fs::remove_dir_all(&stufe);
                return Err(e);
            }
        };
        let _ = fs::write(korb.join(PAPIERKORB_STEMPEL), jetzt_ms().to_string());
        Some(korb)
    } else {
        None
    };

    let mut log: Vec<Tausch> = Vec::new();
    let getauscht = (|| -> Result<(), String> {
        let korb_von = |rel: &str| -> Result<PathBuf, String> {
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
        for rel in &weichen {
            let ort = absolut(data, rel);
            let nach = korb_von(rel)?;
            fs::rename(&ort, &nach)
                .map_err(|e| format!("{rel} ließ sich nicht in den Papierkorb legen: {e}"))?;
            log.push(Tausch::Weg { ort, korb: nach });
        }
        for rel in einheiten.iter().filter(|e| !datei_einheit(e)) {
            let von = absolut(&stufe, rel);
            if !von.is_dir() {
                continue;
            }
            let ort = absolut(data, rel);
            if vorhanden(&ort) {
                return Err(format!("{rel} ist noch belegt; nichts überschrieben."));
            }
            if let Some(eltern) = ort.parent() {
                fs::create_dir_all(eltern).map_err(|e| format!("{rel}: {e}"))?;
            }
            fs::rename(&von, &ort).map_err(|e| format!("{rel} ließ sich nicht anlegen: {e}"))?;
            log.push(Tausch::Hin { stufe: von, ort });
        }
        // Dateien zuletzt; der Index als allerletzte, damit er erst auf vollständige
        // Seiten zeigt.
        let mut dateien: Vec<&&String> = dateien_ersetzen.iter().collect();
        dateien.sort_by_key(|e| **e == &index_rel);
        for rel in dateien {
            let ort = absolut(data, rel);
            let alt = if vorhanden(&ort) {
                let nach = korb_von(rel)?;
                kopieren_mit_pruefsumme(&ort, &nach)?;
                Some(nach)
            } else {
                None
            };
            if let Some(eltern) = ort.parent() {
                fs::create_dir_all(eltern).map_err(|e| format!("{rel}: {e}"))?;
            }
            fs::rename(absolut(&stufe, rel), &ort)
                .map_err(|e| format!("{rel} ließ sich nicht ersetzen: {e}"))?;
            ordner_sync(ort.parent().unwrap_or(data));
            log.push(Tausch::Ersetzt { ort, alt });
        }
        Ok(())
    })();
    if let Err(e) = getauscht {
        let offen = tausch_zuruecknehmen(&log);
        if offen.is_empty() {
            let _ = fs::remove_dir_all(&stufe);
            if let Some(korb) = &korb {
                let _ = fs::remove_dir_all(korb);
            }
            return Err(format!(
                "Zurückspielen abgebrochen ({e}); der vorige Stand ist wiederhergestellt."
            ));
        }
        return Err(format!(
            "Zurückspielen abgebrochen ({e}). Nicht alles ließ sich zurücknehmen ({}); \
             Zwischenstand unter {} und Papierkorb {} bleiben erhalten.",
            offen.join("; "),
            relativ(data, &stufe),
            korb.as_ref().map(|k| relativ(data, k)).unwrap_or_default()
        ));
    }
    let _ = fs::remove_dir_all(&stufe);
    Ok(Wiederherstellung {
        dateien: manifest.dateien.len(),
        papierkorb: korb.as_ref().map(|k| relativ(data, k)),
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
fn schritt_geloescht(data: &Path, index: Option<&serde_json::Value>, schritt: &Schritt) -> bool {
    index_hat_seite(index, &schritt.seite) == Some(false)
        && !vorhanden(&absolut(data, &schritt.von))
        && !vorhanden(&absolut(data, &schritt.nach))
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
        if schritt_geloescht(data, index.as_ref(), schritt) {
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
        if schritt_geloescht(data, index.as_ref(), &journal.schritte[i]) {
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
    journal.phase = Phase::Fertig;
    journal_schreiben(data, &journal)?;
    layout_setzen(data, 2)?;
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
        if schritt_geloescht(data, index.as_ref(), &journal.schritte[i]) {
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
    journal.phase = Phase::Zurueckgenommen;
    journal_schreiben(data, &journal)?;
    layout_setzen(data, 1)?;
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
        Phase::Fertig | Phase::Zurueckgenommen => Ok(Bericht {
            phase: Some(journal.phase),
            ..Default::default()
        }),
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
            // Noch nichts verschoben: nur abschließen.
            journal.phase = Phase::Zurueckgenommen;
            journal_schreiben(data, &journal)?;
            layout_setzen(data, 1)?;
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
    let fassungen = buch_dir.join(FASSUNGEN);
    let seiten: Vec<String> = unterordner(&fassungen)
        .into_iter()
        .filter(|s| gueltige_seite(s))
        .collect();
    if seiten.is_empty() {
        return Ok(0);
    }
    if aktives_journal(data).is_some() {
        return Err(
            "Die Neuordnung der Bibliothek ist nicht abgeschlossen. Bitte sie erst \
                    abschließen oder zurücknehmen, dann erneut versuchen."
                .into(),
        );
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
        let data = standardlage("zurueck-fehler");
        migrieren(&data, &optionen()).unwrap();
        let name = sicherungen_auflisten(&data)[0].name.clone();
        // Ein Ordner statt buch.json: das Ersetzen der Datei scheitert nach dem Tausch der
        // Seitenordner.
        let buch_json = data.join("library/book_1/buch.json");
        let buch_roh = fs::read(&buch_json).unwrap();
        fs::remove_file(&buch_json).unwrap();
        fs::create_dir_all(&buch_json).unwrap();
        let index_vorher = index_roh(&data);
        let fehler = sicherung_zurueckspielen(&data, &name, None).unwrap_err();
        assert!(fehler.contains("vorige Stand"), "{fehler}");
        assert_eq!(index_roh(&data), index_vorher);
        assert!(data
            .join("library/book_1/fassungen/page_1/state.json")
            .is_file());
        assert!(!data.join("projects/page_1").exists());
        assert!(!data
            .join(SICHERUNGEN)
            .join(PAPIERKORB)
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false));
        assert!(!unterordner(&data.join(SICHERUNGEN))
            .iter()
            .any(|n| n.starts_with(".zurueck-")));
        fs::remove_dir(&buch_json).unwrap();
        fs::write(&buch_json, buch_roh).unwrap();
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
