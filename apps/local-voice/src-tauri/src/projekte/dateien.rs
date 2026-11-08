//! Eigene Dateien eines Projekts (lva#365 P4, Spec §5.1, Codex-Planprüfung 08.11. Auflage 5).
//!
//! Fremde Dateien werden nach `projekte/<projekt>/dateien/` kopiert. Ablauf je Datei:
//!
//! 1. eigene Zwischendatei `dateien/.teil-<id>` (`create_new`), vollständig kopieren, `sync_all`;
//! 2. Größe und SHA-256 der Kopie gegen die (während des Lesens unveränderte) Quelle prüfen;
//! 3. ohne Ersetzen unter einem freien, bereinigten Namen ablegen (Regeln 4.4);
//! 4. im Journal `projekte/<projekt>/dateien.json` als `abgelegt` vermerken;
//! 5. erst dann trägt der Dienst die Mitgliedschaft ein und vermerkt `eingetragen`.
//!
//! Wiederanlauf ([`wiederanlauf`]): `abgelegt` ohne Mitgliedschaft wird erneut eingetragen; eine
//! Datei im Ordner ohne Journaleintrag (Absturz zwischen 3 und 4) wird als `gefunden` vermerkt und
//! angeboten, nie still aufgenommen; liegengebliebene Zwischendateien gehen in den Papierkorb.
//!
//! Entfernen ([`entfernen_beginnen`]): erst `entfernen` mit Ziel im Papierkorb, Größe und SHA-256
//! vermerken, dann die Datei ohne Ersetzen unter einen privaten Prüfnamen im Papierkorb legen, dort
//! prüfen und nur bei gleicher Identität ans Ziel legen (sonst zurück nach `dateien/`, wo sie als
//! `gefunden` angeboten wird), dann `papierkorb`. Vor dem Hashen wird der Abdruck (Datei-Kennung
//! dev/ino bzw. FILE_ID_INFO, Größe, Änderungszeit) gemerkt und unmittelbar vor dem Verschieben neu
//! gemessen; weicht er ab oder ist er nicht messbar, wird nicht verschoben. Grenze (wie beim Löschen
//! von Sicherungen): kein gezielt austauschender Prozess im eigenen Konto; ein Fenster von einem
//! Systemaufruf zwischen letzter Messung und Verschieben bleibt. Danach nimmt der Dienst die
//! Mitgliedschaft weg (bzw. der Abgleich, weil die Herkunft dann nicht mehr besteht). Ein Abbruch
//! dazwischen wird beim Wiederanlauf fortgesetzt. Nichts wird überschrieben oder endgültig gelöscht.
//!
//! Lesbar heißt: Journal gelesen, oder es fehlt und `dateien/`/`papierkorb/` fehlen oder sind
//! nachweislich leer. Jeder andere Fall ist unklar und blockiert, bis die Bedienung „Bestand neu
//! aufbauen“ ([`neu_aufbauen`]) ein fehlendes oder defektes Journal ersetzt.
//!
//! Projekt- und Datei-IDs werden geprüft; jeder Pfad unter dem Projektordner wird gegen
//! Verknüpfungen geprüft. Keine Namen oder Inhalte im Log oder Fehlertext.

use super::herkunft::{id_gueltig, suite_id_gueltig};
use super::kontext::{hinweis, Form};
use crate::buch_dateien::{
    ablegen_frei, ohne_verknuepfung, sha256_datei, sha256_stabil, verschieben_ohne_ersetzen,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const DATEIEN: &str = "dateien";
pub const PAPIERKORB: &str = "papierkorb";
pub const JOURNAL: &str = "dateien.json";
/// Papierkorb gelöschter Projekte im Projektordner der App.
pub const PROJEKT_PAPIERKORB: &str = ".papierkorb";
const TEIL: &str = ".teil-";
const VERSION: u32 = 1;
/// Größter Text, der als Kontext gelesen wird (4 MB).
const MAX_KONTEXT_BYTES: u64 = 4 * 1024 * 1024;

static SPERRE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn sperren() -> std::sync::MutexGuard<'static, ()> {
    SPERRE.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Abgelegt,
    Eingetragen,
    /// Im Ordner, ohne Journaleintrag gefunden (Absturz): wird angeboten, nicht aufgenommen.
    Gefunden,
    Entfernen,
    Papierkorb,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Datei {
    pub name: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default)]
    pub groesse: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eintrag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub papierkorb: Option<String>,
    /// Beim Entfernen: Identität der zu verschiebenden Datei (SHA-256, Größe), vorab vermerkt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entfernen_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entfernen_groesse: Option<u64>,
    /// Entfernen abgelehnt: die Datei konnte nicht sicher in den Papierkorb gelegt werden (andere
    /// Datei, verändert, keine verlässliche Kennung). Sie liegt weiter im Projektordner und wird als
    /// `gefunden` angeboten.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub nicht_sicher: bool,
    pub ms: i64,
}

impl Datei {
    /// Besteht als Herkunft eines Eintrags.
    pub fn lebt(&self) -> bool {
        matches!(self.status, Status::Abgelegt | Status::Eingetragen)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Journal {
    v: u32,
    #[serde(default)]
    dateien: BTreeMap<String, Datei>,
}

fn jetzt_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn neue_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "pd_{nanos:x}{:x}{:x}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// Projektordner `<ablage>/<projekt>` nach Prüfung der ID und ohne Verknüpfung.
pub fn projekt_dir(ablage: &Path, projekt: &str) -> Result<PathBuf, String> {
    if !suite_id_gueltig(projekt) {
        return Err("Kennung des Projekts ungültig.".into());
    }
    ohne_verknuepfung(ablage, projekt)?;
    ohne_verknuepfung(ablage, &format!("{projekt}/{DATEIEN}"))?;
    ohne_verknuepfung(ablage, &format!("{projekt}/{PAPIERKORB}"))?;
    ohne_verknuepfung(ablage, &format!("{projekt}/{JOURNAL}"))?;
    Ok(ablage.join(projekt))
}

/// Ein einzelner, zulässiger Dateiname: unverändert durch die Bereinigung (Regeln 4.4), also ohne
/// Pfadtrenner, Laufwerks- oder Windows-Pfadsyntax, ohne `.`/`..`, nicht versteckt.
pub fn name_ok(name: &str) -> bool {
    !name.starts_with('.') && crate::library_layout::bereinigter_name(name).is_ok_and(|b| b == name)
}

/// Liegt in `dateien/` oder `papierkorb/` etwas? Leer ist nur, was fehlt (`NotFound`) oder
/// nachweislich leer gelesen wurde; jeder andere Lesefehler ist ein Fehler (= unklar).
fn bestand_vorhanden(dir: &Path) -> Result<bool, String> {
    for u in [DATEIEN, PAPIERKORB] {
        match fs::read_dir(dir.join(u)) {
            Ok(mut e) => match e.next() {
                None => {}
                Some(Ok(_)) => return Ok(true),
                Some(Err(_)) => return Err("Projektordner nicht lesbar; nichts angefasst.".into()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Projektordner nicht lesbar; nichts angefasst.".into()),
        }
    }
    Ok(false)
}

/// Warum ein Journal nicht geladen werden kann.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum JournalLage {
    /// Fehlt, obwohl Dateien vorhanden sind.
    Fehlt,
    /// Kaputt oder mit ungültigen Namen.
    Defekt,
    /// Von einer neueren App-Version: nicht lesbar und nie neu aufbaubar (die Version wird vor der
    /// übrigen Form geprüft).
    Neuer,
    /// Vorübergehend nicht lesbar (Rechte, Sperre, Datenträger): später erneut.
    NichtLesbar,
}

impl JournalLage {
    fn text(self) -> &'static str {
        match self {
            Self::Fehlt => {
                "Das Dateijournal fehlt, obwohl Dateien vorhanden sind; nichts angefasst."
            }
            Self::Defekt => "Das Dateijournal ist nicht lesbar; nichts angefasst.",
            Self::Neuer => "Das Dateijournal stammt von einer neueren App-Version.",
            Self::NichtLesbar => {
                "Dateijournal oder Projektordner gerade nicht lesbar; nichts angefasst."
            }
        }
    }
}

fn journal_laden(dir: &Path) -> Result<Journal, String> {
    journal_lage(dir).map_err(|l| l.text().to_string())
}

fn journal_lage(dir: &Path) -> Result<Journal, JournalLage> {
    let j = match fs::read(dir.join(JOURNAL)) {
        Ok(roh) => {
            // Erst nur die Version: ein Journal einer neueren App ist nie „defekt“ (und damit nie
            // neu aufbaubar), auch wenn seine übrige Form hier unbekannt ist.
            #[derive(Deserialize)]
            struct NurVersion {
                v: u32,
            }
            match serde_json::from_slice::<NurVersion>(&roh) {
                Ok(n) if n.v > VERSION => return Err(JournalLage::Neuer),
                Ok(_) => {}
                Err(_) => return Err(JournalLage::Defekt),
            }
            match serde_json::from_slice::<Journal>(&roh) {
                Ok(j) => j,
                Err(_) => return Err(JournalLage::Defekt),
            }
        }
        // Ohne Journal, aber mit Dateien: unklar, nicht leer (das Journal wird vor der ersten
        // Ablage geschrieben; fehlt es jetzt, ist es verloren gegangen).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            match bestand_vorhanden(dir) {
                Ok(true) => return Err(JournalLage::Fehlt),
                Ok(false) => {}
                Err(_) => return Err(JournalLage::NichtLesbar),
            }
            Journal {
                v: VERSION,
                dateien: BTreeMap::new(),
            }
        }
        Err(_) => return Err(JournalLage::NichtLesbar),
    };
    let gueltig = j.dateien.iter().all(|(id, d)| {
        id_gueltig(id) && name_ok(&d.name) && d.papierkorb.as_deref().is_none_or(name_ok)
    });
    if !gueltig {
        return Err(JournalLage::Defekt);
    }
    Ok(j)
}

/// Zustand des Dateijournals eines Projekts: `None` = lesbar.
pub fn lage(ablage: &Path, projekt: &str) -> Result<Option<JournalLage>, String> {
    let _g = sperren();
    let dir = projekt_dir(ablage, projekt)?;
    Ok(journal_lage(&dir).err())
}

/// Ausdrücklicher Neuaufbau eines fehlenden oder defekten Journals (Bedienung „Bestand neu
/// aufbauen“). Ein defektes Journal wird als `dateien.json.defekt-<ms>` beiseitegelegt, nie
/// gelöscht. Jede Datei in `dateien/` wird als `gefunden` vermerkt und angeboten, nicht still
/// aufgenommen; der Papierkorb bleibt unberührt. Ein Journal einer neueren Version oder ein nur
/// vorübergehend nicht lesbares wird nicht angefasst. Liefert die Zahl der gefundenen Dateien.
pub fn neu_aufbauen(ablage: &Path, projekt: &str) -> Result<u32, String> {
    let _g = sperren();
    let dir = projekt_dir(ablage, projekt)?;
    match journal_lage(&dir) {
        Ok(_) => return Err("Das Dateijournal ist lesbar; nichts neu aufzubauen.".into()),
        Err(JournalLage::Neuer) | Err(JournalLage::NichtLesbar) => {
            return Err("Das Dateijournal wird so nicht neu aufgebaut; nichts angefasst.".into())
        }
        Err(JournalLage::Defekt) => {
            let beiseite = crate::library_layout::freier_name(
                &dir,
                &format!("{JOURNAL}.defekt-{}", jetzt_ms()),
            )?;
            verschieben_ohne_ersetzen(&dir.join(JOURNAL), &dir.join(beiseite))
                .map_err(|e| format!("Dateijournal nicht beiseitelegbar: {e}"))?;
        }
        Err(JournalLage::Fehlt) => {}
    }
    let mut j = Journal {
        v: VERSION,
        dateien: BTreeMap::new(),
    };
    let mut n = 0;
    let lesen = fs::read_dir(dir.join(DATEIEN));
    let eintraege = match lesen {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            journal_sichern(&dir, &j)?;
            return Ok(0);
        }
        Err(_) => return Err("Projektordner nicht lesbar; nichts angefasst.".into()),
    };
    for e in eintraege {
        let e = e.map_err(|_| "Projektordner nicht lesbar; nichts angefasst.".to_string())?;
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(meta) = fs::symlink_metadata(e.path()) else {
            continue;
        };
        if !meta.is_file() || !name_ok(&name) {
            continue;
        }
        j.dateien.insert(
            neue_id(),
            Datei {
                name,
                status: Status::Gefunden,
                sha256: None,
                groesse: meta.len(),
                eintrag: None,
                papierkorb: None,
                entfernen_sha: None,
                entfernen_groesse: None,
                nicht_sicher: false,
                ms: jetzt_ms(),
            },
        );
        n += 1;
    }
    journal_sichern(&dir, &j)?;
    Ok(n)
}

fn journal_sichern(dir: &Path, j: &Journal) -> Result<(), String> {
    let roh = serde_json::to_vec_pretty(j).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".{JOURNAL}.{}.tmp", neue_id()));
    let schreiben = (|| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(&roh)?;
        f.sync_all()?;
        fs::rename(&tmp, dir.join(JOURNAL))
    })();
    if let Err(e) = schreiben {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Dateijournal nicht speicherbar: {e}"));
    }
    Ok(())
}

/// Alle Dateien eines Projekts laut Journal.
pub fn liste(ablage: &Path, projekt: &str) -> Result<BTreeMap<String, Datei>, String> {
    let _g = sperren();
    let dir = projekt_dir(ablage, projekt)?;
    Ok(journal_laden(&dir)?.dateien)
}

/// Bestand der Projektdateien aller Projekte (für die Herkunft).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Bestand {
    /// Lebende Dateien `(projekt, datei-id)`.
    pub lebend: Vec<(String, String)>,
    /// Projekte, deren Journal vollständig gelesen wurde (fehlendes Journal = leer).
    pub gelesen: Vec<String>,
    /// Projekte mit nicht lesbarem Journal: ihre Verweise bleiben unbeurteilt.
    pub unklar: Vec<String>,
    /// Der Projektordner der App selbst war nicht vollständig lesbar: nicht gelistete Projekte
    /// sind dann nicht „leer“, sondern unbeurteilt.
    pub unvollstaendig: bool,
}

/// Liest die Journale aller Projektordner unter `ablage`.
pub fn bestand(ablage: &Path) -> Bestand {
    let _g = sperren();
    let mut b = Bestand::default();
    let eintraege = match fs::read_dir(ablage) {
        Ok(e) => e,
        Err(e) => {
            b.unvollstaendig = e.kind() != std::io::ErrorKind::NotFound;
            return b;
        }
    };
    for e in eintraege {
        let Ok(e) = e else {
            b.unvollstaendig = true;
            continue;
        };
        let projekt = e.file_name().to_string_lossy().into_owned();
        if !suite_id_gueltig(&projekt) {
            continue;
        }
        match projekt_dir(ablage, &projekt).and_then(|d| journal_laden(&d)) {
            Ok(j) => {
                b.lebend.extend(
                    j.dateien
                        .into_iter()
                        .filter(|(_, d)| d.lebt())
                        .map(|(id, _)| (projekt.clone(), id)),
                );
                b.gelesen.push(projekt);
            }
            Err(_) => b.unklar.push(projekt),
        }
    }
    b
}

/// Schritte 1 bis 4: kopieren, prüfen, ohne Ersetzen ablegen, als `abgelegt` vermerken. Liefert
/// `(datei-id, name)`. Bei jedem Fehler bleibt kein Journaleintrag und keine eigene Zwischendatei.
pub fn hinzufuegen(
    ablage: &Path,
    projekt: &str,
    quelle: &Path,
) -> Result<(String, String), String> {
    let meta = fs::symlink_metadata(quelle).map_err(|e| format!("Quelle nicht lesbar: {e}"))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("Die Quelle ist keine gewöhnliche Datei.".into());
    }
    let wunsch = quelle
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("Die Quelle hat keinen Dateinamen.")?;
    let _g = sperren();
    let dir = projekt_dir(ablage, projekt)?;
    let ziel = dir.join(DATEIEN);
    fs::create_dir_all(&ziel).map_err(|e| format!("Projektordner nicht anlegbar: {e}"))?;
    let mut j = journal_laden(&dir)?;
    // Das Journal besteht vor der ersten Ablage: fehlt es später, ist der Bestand unklar.
    if !dir.join(JOURNAL).exists() {
        journal_sichern(&dir, &j)?;
    }
    let id = neue_id();
    let tmp = ziel.join(format!("{TEIL}{id}"));
    let kopiert = (|| -> Result<(String, u64), String> {
        let (sha_quelle, meta) = sha256_stabil(quelle)?;
        let mut nach = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| format!("Zwischendatei nicht anlegbar: {e}"))?;
        let mut von = fs::File::open(quelle).map_err(|e| format!("Quelle nicht lesbar: {e}"))?;
        let n = std::io::copy(&mut von, &mut nach).map_err(|e| format!("Nicht kopiert: {e}"))?;
        nach.sync_all().map_err(|e| format!("Nicht kopiert: {e}"))?;
        drop(nach);
        let sha_kopie = sha256_datei(&tmp)?;
        if n != meta.len() || sha_kopie != sha_quelle {
            return Err(
                "Die Kopie stimmt nicht mit der Quelle überein; nichts aufgenommen.".into(),
            );
        }
        Ok((sha_kopie, n))
    })();
    let (sha, groesse) = match kopiert {
        Ok(x) => x,
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    };
    let name = match ablegen_frei(&tmp, &ziel, &wunsch) {
        Ok(n) => n,
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    };
    j.dateien.insert(
        id.clone(),
        Datei {
            name: name.clone(),
            status: Status::Abgelegt,
            sha256: Some(sha),
            groesse,
            eintrag: None,
            papierkorb: None,
            entfernen_sha: None,
            entfernen_groesse: None,
            nicht_sicher: false,
            ms: jetzt_ms(),
        },
    );
    // Scheitert das Vermerken, liegt die Kopie ohne Journal im Ordner: der Wiederanlauf findet sie.
    journal_sichern(&dir, &j)?;
    Ok((id, name))
}

/// Schritt 5: Mitgliedschaft eingetragen.
pub fn eingetragen(ablage: &Path, projekt: &str, id: &str, eintrag: &str) -> Result<(), String> {
    aendern(ablage, projekt, id, |d| {
        if d.lebt() {
            d.status = Status::Eingetragen;
            d.eintrag = Some(eintrag.to_string());
        }
        Ok(())
    })
}

fn aendern(
    ablage: &Path,
    projekt: &str,
    id: &str,
    f: impl FnOnce(&mut Datei) -> Result<(), String>,
) -> Result<(), String> {
    if !id_gueltig(id) {
        return Err("Kennung der Datei ungültig.".into());
    }
    let _g = sperren();
    let dir = projekt_dir(ablage, projekt)?;
    let mut j = journal_laden(&dir)?;
    let d = j.dateien.get_mut(id).ok_or("Datei nicht gefunden.")?;
    f(d)?;
    journal_sichern(&dir, &j)
}

/// Entfernen: Ziel im Papierkorb und Identität der Datei (Größe, SHA-256) vorab vermerken, dann
/// genau diese Datei ohne Ersetzen dorthin legen, dann `papierkorb`. Die Mitgliedschaft nimmt danach
/// der Dienst weg. Liefert die Eintrags-ID, falls bekannt.
pub fn entfernen_beginnen(
    ablage: &Path,
    projekt: &str,
    id: &str,
) -> Result<Option<String>, String> {
    if !id_gueltig(id) {
        return Err("Kennung der Datei ungültig.".into());
    }
    {
        let _g = sperren();
        let dir = projekt_dir(ablage, projekt)?;
        let mut j = journal_laden(&dir)?;
        let d = j.dateien.get_mut(id).ok_or("Datei nicht gefunden.")?;
        if matches!(d.status, Status::Papierkorb) {
            return Ok(d.eintrag.clone());
        }
        if d.status != Status::Entfernen {
            let quelle = dir.join(DATEIEN).join(&d.name);
            ohne_verknuepfung(&dir, &format!("{DATEIEN}/{}", d.name))?;
            let (sha, groesse) = match fs::symlink_metadata(&quelle) {
                Ok(m) if m.is_file() => {
                    let (sha, m) = sha256_stabil(&quelle)?;
                    (Some(sha), Some(m.len()))
                }
                _ => (None, None),
            };
            d.status = Status::Entfernen;
            d.entfernen_sha = sha;
            d.entfernen_groesse = groesse;
            d.papierkorb = korb_ziel(&dir, &d.name)?;
            journal_sichern(&dir, &j)?;
        }
    }
    entfernen_fortsetzen(ablage, projekt, id)
}

/// Freier Name im Papierkorb des Projekts (Ordner wird angelegt).
fn korb_ziel(dir: &Path, name: &str) -> Result<Option<String>, String> {
    ohne_verknuepfung(dir, PAPIERKORB)?;
    let korb = dir.join(PAPIERKORB);
    fs::create_dir_all(&korb).map_err(|e| format!("Papierkorb nicht anlegbar: {e}"))?;
    crate::library_layout::freier_name(&korb, name).map(Some)
}

/// Hat die Datei die vermerkte Identität?
fn gleiche_datei(pfad: &Path, sha: &Option<String>, groesse: Option<u64>) -> bool {
    let (Some(sha), Some(groesse)) = (sha, groesse) else {
        return false;
    };
    match fs::symlink_metadata(pfad) {
        Ok(m) if m.is_file() && m.len() == groesse => sha256_datei(pfad).is_ok_and(|s| &s == sha),
        _ => false,
    }
}

/// Privater Name im Papierkorb, unter dem eine Datei vor dem Ablegen geprüft wird.
fn pruefname(id: &str) -> String {
    format!(".pruefen-{id}")
}

/// Liegt die Datei unter dem privaten Prüfnamen, wird sie dort geprüft: gleiche Identität → an
/// ihr Ziel im Papierkorb; andere Identität → zurück nach `dateien/` (unter ihrem Namen oder,
/// falls belegt, einem freien), wo der Wiederanlauf sie als `gefunden` anbietet. Liefert, ob die
/// vermerkte Datei abgelegt wurde.
fn pruefen_und_ablegen(
    dir: &Path,
    d: &Datei,
    id: &str,
    ziel: &str,
) -> Result<Option<String>, String> {
    let korb = dir.join(PAPIERKORB);
    let privat = korb.join(pruefname(id));
    // Abdruck (Kennung, Größe, Zeit) vor dem Hashen merken und unmittelbar vor dem Verschieben neu
    // messen: abweichend oder nicht messbar → nicht ablegen, sondern zurücklegen.
    let vor = abdruck(&privat);
    let gleich = vor.is_ok() && gleiche_datei(&privat, &d.entfernen_sha, d.entfernen_groesse);
    #[cfg(test)]
    nach_pruefung(&privat);
    let gleich_geblieben = gleich
        && vor
            .as_ref()
            .is_ok_and(|v| abdruck(&privat).as_ref() == Ok(v));
    if gleich_geblieben {
        verschieben_ohne_ersetzen(&privat, &korb.join(ziel))
            .map_err(|e| format!("Datei nicht in den Papierkorb verschiebbar: {e}"))?;
        return Ok(None);
    }
    // Nicht die vermerkte, eine veränderte oder eine nicht sicher kennbare Datei: zurück, nie
    // ersetzen. Das ist eine abgeschlossene Ablehnung; der Aufrufer bietet sie als `gefunden` an.
    let ordner = dir.join(DATEIEN);
    if verschieben_ohne_ersetzen(&privat, &ordner.join(&d.name)).is_ok() {
        return Ok(Some(d.name.clone()));
    }
    let frei = crate::library_layout::freier_name(&ordner, &d.name)?;
    verschieben_ohne_ersetzen(&privat, &ordner.join(&frei))
        .map_err(|e| format!("Datei nicht zurücklegbar: {e}"))?;
    Ok(Some(frei))
}

/// Abdruck (Kennung, Größe, Zeit) einer Datei; im Test lässt sich eine fehlende Kennung erzwingen.
fn abdruck(p: &Path) -> Result<crate::library_layout::DateiAbdruck, String> {
    #[cfg(test)]
    if KENNUNG_FEHLT.with(|k| k.get()) {
        return Err("Keine verlässliche Datei-Kennung (Test).".into());
    }
    crate::library_layout::datei_abdruck(p)
}

#[cfg(test)]
thread_local! {
    static KENNUNG_FEHLT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
type Haken = Box<dyn Fn(&Path)>;

#[cfg(test)]
thread_local! {
    /// Testhaken zwischen Prüfung und Verschieben.
    static NACH_PRUEFUNG: std::cell::RefCell<Option<Haken>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn nach_pruefung(p: &Path) {
    NACH_PRUEFUNG.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f(p)
        }
    });
}

fn entfernen_fortsetzen(ablage: &Path, projekt: &str, id: &str) -> Result<Option<String>, String> {
    let _g = sperren();
    let dir = projekt_dir(ablage, projekt)?;
    let mut j = journal_laden(&dir)?;
    let Some(d) = j.dateien.get(id).cloned() else {
        return Ok(None);
    };
    if d.status != Status::Entfernen {
        return Ok(d.eintrag);
    }
    let quelle = dir.join(DATEIEN).join(&d.name);
    ohne_verknuepfung(&dir, &format!("{DATEIEN}/{}", d.name))?;
    ohne_verknuepfung(&dir, PAPIERKORB)?;
    let korb = dir.join(PAPIERKORB);
    fs::create_dir_all(&korb).map_err(|e| format!("Papierkorb nicht anlegbar: {e}"))?;
    let privat = korb.join(pruefname(id));
    let (sha, groesse) = (&d.entfernen_sha, d.entfernen_groesse);
    let mut ziel = d.papierkorb.clone();
    // Ziel fehlt oder ist von etwas anderem belegt: erst ein neues freies Ziel vermerken.
    let erledigt = ziel
        .as_ref()
        .is_some_and(|z| gleiche_datei(&korb.join(z), sha, groesse));
    if !erledigt && ziel.as_ref().is_none_or(|z| korb.join(z).exists()) {
        ziel = korb_ziel(&dir, &d.name)?;
        if let Some(x) = j.dateien.get_mut(id) {
            x.papierkorb = ziel.clone();
        }
        journal_sichern(&dir, &j)?;
    }
    let z = ziel.clone().ok_or("Kein Ziel im Papierkorb.")?;
    // (abgelegt, zurückgelegt unter Name)
    let (abgelegt, zurueck) = if erledigt {
        (true, None)
    } else if fs::symlink_metadata(&privat).is_ok() {
        // Abbruch nach dem Verschieben unter den Prüfnamen.
        let r = pruefen_und_ablegen(&dir, &d, id, &z)?;
        (r.is_none(), r)
    } else if fs::symlink_metadata(&quelle).is_ok_and(|m| m.is_file()) {
        // Erst unter den privaten Namen, dort prüfen: was nach einer Prüfung unter dem Quellnamen
        // auftaucht, wird so nie abgelegt, sondern zurückgelegt.
        verschieben_ohne_ersetzen(&quelle, &privat)
            .map_err(|e| format!("Datei nicht in den Papierkorb verschiebbar: {e}"))?;
        let r = pruefen_und_ablegen(&dir, &d, id, &z)?;
        (r.is_none(), r)
    } else {
        (false, None)
    };
    // Zurückgelegt: gleich als `gefunden` anbieten (nie still aufnehmen).
    if let Some(name) = &zurueck {
        let schluessel = crate::library_layout::vergleichsschluessel(name);
        let bekannt = j.dateien.iter().any(|(k, x)| {
            k != id
                && x.status != Status::Papierkorb
                && crate::library_layout::vergleichsschluessel(&x.name) == schluessel
        });
        let groesse = fs::symlink_metadata(dir.join(DATEIEN).join(name))
            .map(|m| m.len())
            .unwrap_or_default();
        if !bekannt {
            j.dateien.insert(
                neue_id(),
                Datei {
                    name: name.clone(),
                    status: Status::Gefunden,
                    sha256: None,
                    groesse,
                    eintrag: None,
                    papierkorb: None,
                    entfernen_sha: None,
                    entfernen_groesse: None,
                    nicht_sicher: false,
                    ms: jetzt_ms(),
                },
            );
        }
    }
    let x = j.dateien.get_mut(id).ok_or("Datei nicht gefunden.")?;
    // Wurde die vermerkte Datei nicht gefunden, ist was jetzt unter dem Namen liegt eine andere
    // Datei; sie bleibt unberührt und wird als `gefunden` angeboten.
    x.nicht_sicher = zurueck.is_some();
    x.papierkorb = if abgelegt { ziel } else { None };
    x.status = Status::Papierkorb;
    x.ms = jetzt_ms();
    let eintrag = x.eintrag.clone();
    journal_sichern(&dir, &j)?;
    Ok(eintrag)
}

/// Was der Wiederanlauf vorfand.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Wiederanlauf {
    /// `abgelegt` ohne Mitgliedschaft: der Dienst trägt sie (erneut) ein.
    pub einzutragen: Vec<String>,
    /// Neu gefundene Dateien ohne Journaleintrag.
    pub gefunden: u32,
    /// Fortgesetzte Entfernungen.
    pub entfernt: u32,
    /// Liegengebliebene eigene Zwischendateien, in den Papierkorb gelegt.
    pub teile: u32,
}

/// Wiederanlauf eines Projekts: siehe Moduldoku.
pub fn wiederanlauf(ablage: &Path, projekt: &str) -> Result<Wiederanlauf, String> {
    let mut w = Wiederanlauf::default();
    let offen: Vec<String> = liste(ablage, projekt)?
        .into_iter()
        .filter(|(_, d)| d.status == Status::Entfernen)
        .map(|(id, _)| id)
        .collect();
    // Ein einzelnes Entfernen, das gerade nicht weiterkommt, hält die Fundsuche nicht auf.
    for id in offen {
        if entfernen_fortsetzen(ablage, projekt, &id).is_ok() {
            w.entfernt += 1;
        }
    }
    let _g = sperren();
    let dir = projekt_dir(ablage, projekt)?;
    let mut j = journal_laden(&dir)?;
    let bekannt: std::collections::HashSet<String> = j
        .dateien
        .values()
        .filter(|d| d.status != Status::Papierkorb)
        .map(|d| crate::library_layout::vergleichsschluessel(&d.name))
        .collect();
    let ordner = dir.join(DATEIEN);
    let mut geaendert = false;
    if let Ok(eintraege) = fs::read_dir(&ordner) {
        for e in eintraege.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(meta) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            if !meta.is_file() || meta.file_type().is_symlink() {
                continue;
            }
            if name.starts_with(TEIL) {
                let korb = dir.join(PAPIERKORB);
                fs::create_dir_all(&korb).map_err(|e| format!("Papierkorb nicht anlegbar: {e}"))?;
                ablegen_frei(&e.path(), &korb, &format!("unvollstaendig{}", &name[1..]))?;
                w.teile += 1;
                continue;
            }
            if !name_ok(&name)
                || bekannt.contains(&crate::library_layout::vergleichsschluessel(&name))
            {
                continue;
            }
            j.dateien.insert(
                neue_id(),
                Datei {
                    name,
                    status: Status::Gefunden,
                    sha256: None,
                    groesse: meta.len(),
                    eintrag: None,
                    papierkorb: None,
                    entfernen_sha: None,
                    entfernen_groesse: None,
                    nicht_sicher: false,
                    ms: jetzt_ms(),
                },
            );
            w.gefunden += 1;
            geaendert = true;
        }
    }
    if geaendert {
        journal_sichern(&dir, &j)?;
    }
    w.einzutragen = j
        .dateien
        .iter()
        .filter(|(_, d)| d.status == Status::Abgelegt)
        .map(|(id, _)| id.clone())
        .collect();
    Ok(w)
}

/// Eine gefundene Datei bewusst aufnehmen: Identität vermerken, `abgelegt`; danach trägt der Dienst
/// sie ein.
pub fn gefundene_aufnehmen(ablage: &Path, projekt: &str, id: &str) -> Result<String, String> {
    if !id_gueltig(id) {
        return Err("Kennung der Datei ungültig.".into());
    }
    let _g = sperren();
    let dir = projekt_dir(ablage, projekt)?;
    let mut j = journal_laden(&dir)?;
    let d = j.dateien.get_mut(id).ok_or("Datei nicht gefunden.")?;
    if d.status != Status::Gefunden {
        return Err("Die Datei ist nicht als gefunden vermerkt.".into());
    }
    ohne_verknuepfung(&dir, &format!("{DATEIEN}/{}", d.name))?;
    let (sha, meta) = sha256_stabil(&dir.join(DATEIEN).join(&d.name))?;
    d.sha256 = Some(sha);
    d.groesse = meta.len();
    d.status = Status::Abgelegt;
    let name = d.name.clone();
    journal_sichern(&dir, &j)?;
    Ok(name)
}

/// Text einer Projektdatei als Kontext: nur `.txt`/`.md`, UTF-8, bis 4 MB.
pub fn kontext_text(ablage: &Path, projekt: &str, id: &str) -> Form {
    let Ok(alle) = liste(ablage, projekt) else {
        return Form::Fehlt(hinweis::NICHT_LESBAR);
    };
    let Some(d) = alle.get(id).filter(|d| d.lebt()) else {
        return Form::Fehlt(hinweis::FEHLT);
    };
    let endung = Path::new(&d.name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !matches!(endung.as_str(), "txt" | "md") {
        return Form::Fehlt(hinweis::FORMAT);
    }
    let rel = format!("{projekt}/{DATEIEN}/{}", d.name);
    if ohne_verknuepfung(ablage, &rel).is_err() {
        return Form::Fehlt(hinweis::NICHT_LESBAR);
    }
    let pfad = ablage.join(&rel);
    let Ok(f) = fs::File::open(&pfad) else {
        return Form::Fehlt(hinweis::FEHLT);
    };
    let mut roh = Vec::new();
    if f.take(MAX_KONTEXT_BYTES + 1).read_to_end(&mut roh).is_err()
        || roh.len() as u64 > MAX_KONTEXT_BYTES
    {
        return Form::Fehlt(hinweis::NICHT_LESBAR);
    }
    match String::from_utf8(roh) {
        Ok(t) => Form::Text(t),
        Err(_) => Form::Fehlt(hinweis::FORMAT),
    }
}

/// Projekt löschen: der ganze Projektordner geht ohne Ersetzen in `<ablage>/.papierkorb/`.
pub fn projekt_in_papierkorb(ablage: &Path, projekt: &str) -> Result<bool, String> {
    let _g = sperren();
    let dir = projekt_dir(ablage, projekt)?;
    if !dir.exists() {
        return Ok(false);
    }
    let korb = ablage.join(PROJEKT_PAPIERKORB);
    ohne_verknuepfung(ablage, PROJEKT_PAPIERKORB)?;
    fs::create_dir_all(&korb).map_err(|e| format!("Papierkorb nicht anlegbar: {e}"))?;
    for n in 0..100u32 {
        let ziel = korb.join(format!("{projekt}-{}-{n}", jetzt_ms()));
        match verschieben_ohne_ersetzen(&dir, &ziel) {
            Ok(()) => return Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Projektordner nicht verschiebbar: {e}")),
        }
    }
    Err("Kein freier Platz im Papierkorb.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "prj_01HZZZZZZZZZZZZZZZZZZZZZZZ";

    fn tmp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("lva-projdateien-{}", neue_id()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn quelle(dir: &Path, name: &str, inhalt: &str) -> PathBuf {
        let q = dir.join("quelle");
        fs::create_dir_all(&q).unwrap();
        let p = q.join(name);
        fs::write(&p, inhalt).unwrap();
        p
    }

    #[test]
    fn kopie_mit_pruefung_und_namen_ohne_ueberschreiben() {
        let a = tmp();
        let (id1, n1) = hinzufuegen(&a, P, &quelle(&a, "Bericht.pdf", "eins")).unwrap();
        let (_, n2) = hinzufuegen(&a, P, &quelle(&a, "bericht.PDF", "zwei")).unwrap();
        let (_, n3) = hinzufuegen(&a, P, &quelle(&a, "CON.txt", "drei")).unwrap();
        assert_eq!(n1, "Bericht.pdf");
        assert_ne!(n2.to_lowercase(), n1.to_lowercase());
        assert_ne!(n3, "CON.txt");
        let d = a.join(P).join(DATEIEN);
        assert_eq!(fs::read_to_string(d.join(&n1)).unwrap(), "eins");
        assert_eq!(fs::read_to_string(d.join(&n2)).unwrap(), "zwei");
        let l = liste(&a, P).unwrap();
        assert_eq!(l[&id1].status, Status::Abgelegt);
        assert!(fs::read_dir(&d)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().starts_with(TEIL)));
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn ungueltige_kennungen_und_ausbruch_abgewiesen() {
        let a = tmp();
        let q = quelle(&a, "x.txt", "x");
        assert!(hinzufuegen(&a, "../x", &q).is_err());
        assert!(hinzufuegen(&a, "prj_ILOU", &q).is_err());
        assert!(eingetragen(&a, P, "../a", "e").is_err());
        let (_, n) = hinzufuegen(&a, P, &quelle(&a, "..\\x", "x")).unwrap();
        assert!(!n.contains('\\') && n != "..");
        fs::remove_dir_all(&a).ok();
    }

    #[cfg(unix)]
    #[test]
    fn verknuepfter_projektordner_wird_nicht_angefasst() {
        let a = tmp();
        let fremd = a.join("fremd");
        fs::create_dir_all(&fremd).unwrap();
        std::os::unix::fs::symlink(&fremd, a.join(P)).unwrap();
        assert!(hinzufuegen(&a, P, &quelle(&a, "x.txt", "x")).is_err());
        assert_eq!(fs::read_dir(&fremd).unwrap().count(), 0);
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn absturz_nach_umbenennen_wird_gefunden_nicht_still_aufgenommen() {
        let a = tmp();
        let (id, _) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "a")).unwrap();
        // Absturz zwischen Ablegen und Journal: Datei ohne Eintrag, plus liegengebliebener Teil.
        let d = a.join(P).join(DATEIEN);
        fs::write(d.join("verwaist.txt"), "v").unwrap();
        fs::write(d.join(format!("{TEIL}x")), "halb").unwrap();
        let w = wiederanlauf(&a, P).unwrap();
        assert_eq!(w.gefunden, 1);
        assert_eq!(w.teile, 1);
        assert_eq!(
            w.einzutragen,
            vec![id.clone()],
            "abgelegt ohne Mitgliedschaft"
        );
        let l = liste(&a, P).unwrap();
        let (gid, g) = l.iter().find(|(_, x)| x.name == "verwaist.txt").unwrap();
        assert_eq!(g.status, Status::Gefunden);
        assert!(!g.lebt());
        // Teil liegt im Papierkorb, nichts gelöscht.
        assert_eq!(fs::read_dir(a.join(P).join(PAPIERKORB)).unwrap().count(), 1);
        // Zweiter Lauf: nichts Neues.
        let w2 = wiederanlauf(&a, P).unwrap();
        assert_eq!((w2.gefunden, w2.teile), (0, 0));
        assert_eq!(gefundene_aufnehmen(&a, P, gid).unwrap(), "verwaist.txt");
        eingetragen(&a, P, &id, "e1").unwrap();
        let w3 = wiederanlauf(&a, P).unwrap();
        assert_eq!(w3.einzutragen, vec![gid.clone()]);
        fs::remove_dir_all(&a).ok();
    }

    /// Abbruch nach dem Vermerken (Ziel + Identität), vor dem Verschieben.
    fn vermerken(a: &Path, id: &str) {
        let dir = a.join(P);
        let name = liste(a, P).unwrap()[id].name.clone();
        let (sha, m) = sha256_stabil(&dir.join(DATEIEN).join(&name)).unwrap();
        let ziel = korb_ziel(&dir, &name).unwrap();
        aendern(a, P, id, |d| {
            d.status = Status::Entfernen;
            d.entfernen_sha = Some(sha);
            d.entfernen_groesse = Some(m.len());
            d.papierkorb = ziel;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn entfernen_in_papierkorb_und_fortsetzen_nach_abbruch() {
        let a = tmp();
        let (id, name) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "a")).unwrap();
        eingetragen(&a, P, &id, "e1").unwrap();
        vermerken(&a, &id);
        assert!(
            !liste(&a, P).unwrap()[&id].lebt(),
            "Herkunft gilt schon als weg"
        );
        let w = wiederanlauf(&a, P).unwrap();
        assert_eq!(w.entfernt, 1);
        let d = &liste(&a, P).unwrap()[&id];
        assert_eq!(d.status, Status::Papierkorb);
        let korb = a
            .join(P)
            .join(PAPIERKORB)
            .join(d.papierkorb.as_ref().unwrap());
        assert_eq!(fs::read_to_string(korb).unwrap(), "a");
        assert!(!a.join(P).join(DATEIEN).join(&name).exists());
        assert_eq!(entfernen_beginnen(&a, P, &id).unwrap(), Some("e1".into()));
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn absturz_nach_verschieben_trifft_keine_neue_gleichnamige_datei() {
        let a = tmp();
        let (id, name) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "alt")).unwrap();
        vermerken(&a, &id);
        // Verschieben geschehen, Absturz vor dem Sichern von `papierkorb`.
        let dir = a.join(P);
        let z = liste(&a, P).unwrap()[&id].papierkorb.clone().unwrap();
        fs::rename(dir.join(DATEIEN).join(&name), dir.join(PAPIERKORB).join(&z)).unwrap();
        // Danach legt jemand eine neue Datei gleichen Namens ab.
        fs::write(dir.join(DATEIEN).join(&name), "neu").unwrap();
        wiederanlauf(&a, P).unwrap();
        assert_eq!(liste(&a, P).unwrap()[&id].status, Status::Papierkorb);
        assert_eq!(
            fs::read_to_string(dir.join(DATEIEN).join(&name)).unwrap(),
            "neu",
            "neue Datei bleibt unberührt"
        );
        assert_eq!(
            fs::read_to_string(dir.join(PAPIERKORB).join(&z)).unwrap(),
            "alt"
        );
        // Sie wird als gefunden angeboten, nicht still aufgenommen.
        let l = liste(&a, P).unwrap();
        assert!(l
            .values()
            .any(|x| x.name == name && x.status == Status::Gefunden));
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn ersetzte_quelle_wird_nicht_verschoben() {
        let a = tmp();
        let (id, name) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "alt")).unwrap();
        vermerken(&a, &id);
        // Vor dem Verschieben wird die Datei durch eine andere gleichen Namens ersetzt.
        let f = a.join(P).join(DATEIEN).join(&name);
        fs::write(&f, "anders").unwrap();
        wiederanlauf(&a, P).unwrap();
        let d = &liste(&a, P).unwrap()[&id];
        assert_eq!(d.status, Status::Papierkorb);
        assert!(d.papierkorb.is_none());
        assert_eq!(fs::read_to_string(&f).unwrap(), "anders");
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn belegtes_korbziel_wird_nicht_ueberschrieben() {
        let a = tmp();
        let (id, _) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "alt")).unwrap();
        vermerken(&a, &id);
        let z = liste(&a, P).unwrap()[&id].papierkorb.clone().unwrap();
        fs::write(a.join(P).join(PAPIERKORB).join(&z), "fremd").unwrap();
        wiederanlauf(&a, P).unwrap();
        let d = &liste(&a, P).unwrap()[&id];
        assert_eq!(d.status, Status::Papierkorb);
        let z2 = d.papierkorb.clone().unwrap();
        assert_ne!(z2, z);
        let korb = a.join(P).join(PAPIERKORB);
        assert_eq!(fs::read_to_string(korb.join(&z)).unwrap(), "fremd");
        assert_eq!(fs::read_to_string(korb.join(&z2)).unwrap(), "alt");
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn nach_der_pruefung_untergeschobene_datei_wird_nicht_abgelegt() {
        let a = tmp();
        let (id, name) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "alt")).unwrap();
        vermerken(&a, &id);
        let dir = a.join(P);
        // Zwischen Prüfung und Verschieben gelangt extern eine andere Datei unter den Namen; sie
        // landet unter dem Prüfnamen (Absturz dort).
        fs::remove_file(dir.join(DATEIEN).join(&name)).unwrap();
        fs::write(dir.join(PAPIERKORB).join(pruefname(&id)), "fremd").unwrap();
        wiederanlauf(&a, P).unwrap();
        let d = &liste(&a, P).unwrap()[&id];
        assert_eq!(d.status, Status::Papierkorb);
        assert!(
            d.papierkorb.is_none(),
            "nichts als die vermerkte Datei abgelegt"
        );
        assert_eq!(
            fs::read_to_string(dir.join(DATEIEN).join(&name)).unwrap(),
            "fremd",
            "zurückgelegt, nicht in den Papierkorb"
        );
        assert!(!dir.join(PAPIERKORB).join(pruefname(&id)).exists());
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn nach_der_pruefung_ersetzte_datei_wird_nicht_abgelegt() {
        let a = tmp();
        let (id, name) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "alt")).unwrap();
        vermerken(&a, &id);
        let dir = a.join(P);
        // Gleicher Inhalt (gleicher Hash, gleiche Größe), aber eine andere Datei: nach der Prüfung
        // unter den Prüfnamen getauscht.
        let ersatz = a.join("ersatz.txt");
        NACH_PRUEFUNG.with(|h| {
            let e = ersatz.clone();
            *h.borrow_mut() = Some(Box::new(move |p: &Path| {
                fs::write(&e, "alt").unwrap();
                fs::rename(&e, p).unwrap();
            }));
        });
        let r = entfernen_beginnen(&a, P, &id);
        NACH_PRUEFUNG.with(|h| *h.borrow_mut() = None);
        r.unwrap();
        let d = &liste(&a, P).unwrap()[&id];
        assert_eq!(d.status, Status::Papierkorb);
        assert!(d.papierkorb.is_none(), "nicht abgelegt");
        assert_eq!(
            fs::read_to_string(dir.join(DATEIEN).join(&name)).unwrap(),
            "alt",
            "zurückgelegt und als gefunden angeboten"
        );
        let korb: Vec<_> = fs::read_dir(dir.join(PAPIERKORB))
            .unwrap()
            .flatten()
            .collect();
        assert!(korb.is_empty(), "nichts im Papierkorb");
        wiederanlauf(&a, P).unwrap();
        assert!(liste(&a, P)
            .unwrap()
            .values()
            .any(|x| x.name == name && x.status == Status::Gefunden));
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn ohne_kennung_wird_abgelehnt_abgeschlossen_und_angeboten() {
        let a = tmp();
        let (id, name) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "alt")).unwrap();
        KENNUNG_FEHLT.with(|k| k.set(true));
        let r = entfernen_beginnen(&a, P, &id);
        KENNUNG_FEHLT.with(|k| k.set(false));
        r.unwrap();
        let l = liste(&a, P).unwrap();
        let d = &l[&id];
        assert_eq!(d.status, Status::Papierkorb, "Journal abgeschlossen");
        assert!(d.nicht_sicher);
        assert!(d.papierkorb.is_none());
        let dir = a.join(P);
        assert_eq!(
            fs::read_to_string(dir.join(DATEIEN).join(&name)).unwrap(),
            "alt"
        );
        assert!(
            l.values()
                .any(|x| x.name == name && x.status == Status::Gefunden),
            "sofort als gefunden angeboten"
        );
        // Wiederanlauf: nichts offen, nichts doppelt.
        let w = wiederanlauf(&a, P).unwrap();
        assert_eq!((w.entfernt, w.gefunden), (0, 0));
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn ohne_kennung_abgelehnt_taucht_als_gefunden_auf_und_wird_wieder_aufgenommen() {
        let a = tmp();
        let (id, name) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "alt")).unwrap();
        eingetragen(&a, P, &id, "e1").unwrap();
        KENNUNG_FEHLT.with(|k| k.set(true));
        let r = entfernen_beginnen(&a, P, &id);
        // Auch ein Wiederanlauf ohne Kennung bietet sie (einmal) als gefunden an.
        let w = wiederanlauf(&a, P);
        KENNUNG_FEHLT.with(|k| k.set(false));
        r.unwrap();
        assert_eq!(w.unwrap().gefunden, 0, "schon beim Entfernen angeboten");
        let l = liste(&a, P).unwrap();
        assert!(l[&id].nicht_sicher && !l[&id].lebt());
        let (gid, g) = l
            .iter()
            .find(|(_, x)| x.status == Status::Gefunden)
            .expect("als gefunden angeboten");
        assert_eq!(g.name, name);
        // Kennung wieder verfügbar: bewusst erneut aufnehmen, dann regulär entfernbar.
        assert_eq!(gefundene_aufnehmen(&a, P, gid).unwrap(), name);
        let d = &liste(&a, P).unwrap()[gid];
        assert!(d.lebt() && d.sha256.is_some());
        eingetragen(&a, P, gid, "e2").unwrap();
        assert_eq!(entfernen_beginnen(&a, P, gid).unwrap(), Some("e2".into()));
        let d = &liste(&a, P).unwrap()[gid];
        assert!(!d.nicht_sicher && d.papierkorb.is_some());
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn neueres_journal_mit_unbekanntem_feld_wird_nie_neu_aufgebaut() {
        let a = tmp();
        hinzufuegen(&a, P, &quelle(&a, "a.txt", "a")).unwrap();
        let pfad = a.join(P).join(JOURNAL);
        let roh = br#"{"v":2,"dateien":{"x":{"neu":true}},"zusatz":[1,2]}"#;
        fs::write(&pfad, roh).unwrap();
        assert_eq!(lage(&a, P).unwrap(), Some(JournalLage::Neuer));
        assert_eq!(bestand(&a).unklar, vec![P.to_string()]);
        assert!(neu_aufbauen(&a, P).is_err());
        assert_eq!(fs::read(&pfad).unwrap(), roh, "unverändert");
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn absturz_unter_dem_pruefnamen_wird_fortgesetzt() {
        let a = tmp();
        let (id, name) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "alt")).unwrap();
        vermerken(&a, &id);
        let dir = a.join(P);
        fs::rename(
            dir.join(DATEIEN).join(&name),
            dir.join(PAPIERKORB).join(pruefname(&id)),
        )
        .unwrap();
        // Unterdessen eine neue Datei gleichen Namens: bleibt, wo sie ist.
        fs::write(dir.join(DATEIEN).join(&name), "neu").unwrap();
        wiederanlauf(&a, P).unwrap();
        let d = &liste(&a, P).unwrap()[&id];
        let z = d.papierkorb.clone().unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(PAPIERKORB).join(z)).unwrap(),
            "alt"
        );
        assert_eq!(
            fs::read_to_string(dir.join(DATEIEN).join(&name)).unwrap(),
            "neu"
        );
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn nicht_lesbarer_ordner_ist_nicht_leer() {
        let a = tmp();
        let dir = a.join(P);
        fs::create_dir_all(&dir).unwrap();
        // `dateien` ist kein Ordner: Lesefehler, kein NotFound.
        fs::write(dir.join(DATEIEN), "x").unwrap();
        assert_eq!(bestand(&a).unklar, vec![P.to_string()]);
        assert_eq!(lage(&a, P).unwrap(), Some(JournalLage::NichtLesbar));
        assert!(neu_aufbauen(&a, P).is_err(), "nur bei fehlt/defekt");
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn bestand_neu_aufbauen_legt_defektes_beiseite_und_bietet_an() {
        let a = tmp();
        hinzufuegen(&a, P, &quelle(&a, "a.txt", "a")).unwrap();
        let dir = a.join(P);
        assert!(neu_aufbauen(&a, P).is_err(), "lesbares Journal bleibt");
        fs::write(dir.join(JOURNAL), b"{kaputt").unwrap();
        assert_eq!(lage(&a, P).unwrap(), Some(JournalLage::Defekt));
        assert_eq!(neu_aufbauen(&a, P).unwrap(), 1);
        assert_eq!(lage(&a, P).unwrap(), None);
        let l = liste(&a, P).unwrap();
        assert!(l
            .values()
            .all(|d| d.status == Status::Gefunden && !d.lebt()));
        let beiseite: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(&format!("{JOURNAL}.defekt-")))
            .collect();
        assert_eq!(beiseite.len(), 1);
        assert_eq!(fs::read(dir.join(&beiseite[0])).unwrap(), b"{kaputt");
        // Fehlendes Journal ebenso.
        fs::remove_file(dir.join(JOURNAL)).unwrap();
        assert_eq!(lage(&a, P).unwrap(), Some(JournalLage::Fehlt));
        assert_eq!(neu_aufbauen(&a, P).unwrap(), 1);
        // Neuere Version: nie neu aufbauen.
        fs::write(dir.join(JOURNAL), br#"{"v":99,"dateien":{}}"#).unwrap();
        assert_eq!(lage(&a, P).unwrap(), Some(JournalLage::Neuer));
        assert!(neu_aufbauen(&a, P).is_err());
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn fehlendes_journal_mit_dateien_ist_unklar() {
        let a = tmp();
        hinzufuegen(&a, P, &quelle(&a, "a.txt", "a")).unwrap();
        fs::remove_file(a.join(P).join(JOURNAL)).unwrap();
        let b = bestand(&a);
        assert!(b.lebend.is_empty());
        assert_eq!(b.unklar, vec![P.to_string()]);
        assert!(wiederanlauf(&a, P).is_err());
        assert!(a.join(P).join(DATEIEN).join("a.txt").exists());
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn ungueltiger_name_im_journal_macht_unklar() {
        for boese in [
            "../x.txt",
            "..",
            "a/b.txt",
            "a\\b.txt",
            "C:x.txt",
            ".versteckt",
        ] {
            let a = tmp();
            let (id, _) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "a")).unwrap();
            let pfad = a.join(P).join(JOURNAL);
            let mut v: serde_json::Value =
                serde_json::from_slice(&fs::read(&pfad).unwrap()).unwrap();
            v["dateien"][&id]["name"] = serde_json::Value::String(boese.into());
            fs::write(&pfad, serde_json::to_vec(&v).unwrap()).unwrap();
            assert_eq!(bestand(&a).unklar, vec![P.to_string()], "{boese}");
            assert!(entfernen_beginnen(&a, P, &id).is_err(), "{boese}");
            assert_eq!(kontext_text(&a, P, &id), Form::Fehlt(hinweis::NICHT_LESBAR));
            fs::remove_dir_all(&a).ok();
        }
    }

    #[test]
    fn papierkorb_ueberschreibt_nicht() {
        let a = tmp();
        let (id1, _) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "eins")).unwrap();
        entfernen_beginnen(&a, P, &id1).unwrap();
        let (id2, _) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "zwei")).unwrap();
        entfernen_beginnen(&a, P, &id2).unwrap();
        let korb = a.join(P).join(PAPIERKORB);
        let mut inhalte: Vec<String> = fs::read_dir(&korb)
            .unwrap()
            .flatten()
            .map(|e| fs::read_to_string(e.path()).unwrap())
            .collect();
        inhalte.sort();
        assert_eq!(inhalte, vec!["eins", "zwei"]);
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn kontext_nur_txt_md_und_hinweis_sonst() {
        let a = tmp();
        let (t, _) = hinzufuegen(&a, P, &quelle(&a, "n.md", "# Notiz")).unwrap();
        let (p, _) = hinzufuegen(&a, P, &quelle(&a, "b.pdf", "%PDF")).unwrap();
        assert_eq!(kontext_text(&a, P, &t), Form::Text("# Notiz".into()));
        assert_eq!(kontext_text(&a, P, &p), Form::Fehlt(hinweis::FORMAT));
        assert_eq!(kontext_text(&a, P, "pd_fehlt"), Form::Fehlt(hinweis::FEHLT));
        let name = liste(&a, P).unwrap()[&t].name.clone();
        fs::remove_file(a.join(P).join(DATEIEN).join(name)).unwrap();
        assert_eq!(kontext_text(&a, P, &t), Form::Fehlt(hinweis::FEHLT));
        assert!(
            liste(&a, P).unwrap()[&t].lebt(),
            "fehlende Datei bleibt Mitglied"
        );
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn bestand_und_projekt_loeschen() {
        let a = tmp();
        let (id, _) = hinzufuegen(&a, P, &quelle(&a, "a.txt", "a")).unwrap();
        let b = bestand(&a);
        assert_eq!(b.lebend, vec![(P.to_string(), id)]);
        assert_eq!(b.gelesen, vec![P.to_string()]);
        assert!(b.unklar.is_empty());
        assert!(projekt_in_papierkorb(&a, P).unwrap());
        assert!(!a.join(P).exists());
        assert_eq!(fs::read_dir(a.join(PROJEKT_PAPIERKORB)).unwrap().count(), 1);
        assert!(bestand(&a).lebend.is_empty());
        fs::remove_dir_all(&a).ok();
    }

    #[test]
    fn kaputtes_journal_macht_projekt_unklar_und_nichts_wird_angefasst() {
        let a = tmp();
        hinzufuegen(&a, P, &quelle(&a, "a.txt", "a")).unwrap();
        fs::write(a.join(P).join(JOURNAL), b"{kaputt").unwrap();
        let b = bestand(&a);
        assert!(b.lebend.is_empty());
        assert_eq!(b.unklar, vec![P.to_string()]);
        assert!(hinzufuegen(&a, P, &quelle(&a, "b.txt", "b")).is_err());
        assert!(wiederanlauf(&a, P).is_err());
        fs::remove_dir_all(&a).ok();
    }
}
