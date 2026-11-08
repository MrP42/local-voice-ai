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
//! Entfernen ([`entfernen_beginnen`]): erst `entfernen` vermerken, dann die Datei ohne Ersetzen
//! nach `projekte/<projekt>/papierkorb/` legen, dann `papierkorb`. Danach nimmt der Dienst die
//! Mitgliedschaft weg (bzw. der Abgleich, weil die Herkunft dann nicht mehr besteht). Ein Abbruch
//! dazwischen wird beim Wiederanlauf fortgesetzt. Nichts wird überschrieben oder endgültig gelöscht.
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

/// Liegt in `dateien/` oder `papierkorb/` etwas?
fn bestand_vorhanden(dir: &Path) -> bool {
    [DATEIEN, PAPIERKORB].iter().any(|u| {
        fs::read_dir(dir.join(u))
            .map(|mut e| e.next().is_some())
            .unwrap_or(false)
    })
}

fn journal_laden(dir: &Path) -> Result<Journal, String> {
    let j = match fs::read(dir.join(JOURNAL)) {
        Ok(roh) => match serde_json::from_slice::<Journal>(&roh) {
            Ok(j) if j.v <= VERSION => j,
            Ok(_) => return Err("Das Dateijournal stammt von einer neueren App-Version.".into()),
            Err(_) => return Err("Das Dateijournal ist nicht lesbar; nichts angefasst.".into()),
        },
        // Ohne Journal, aber mit Dateien: unklar, nicht leer (das Journal wird vor der ersten
        // Ablage geschrieben; fehlt es jetzt, ist es verloren gegangen).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if bestand_vorhanden(dir) {
                return Err(
                    "Das Dateijournal fehlt, obwohl Dateien vorhanden sind; nichts angefasst."
                        .into(),
                );
            }
            Journal {
                v: VERSION,
                dateien: BTreeMap::new(),
            }
        }
        Err(e) => return Err(format!("Dateijournal nicht lesbar: {e}")),
    };
    let gueltig = j.dateien.iter().all(|(id, d)| {
        id_gueltig(id) && name_ok(&d.name) && d.papierkorb.as_deref().is_none_or(name_ok)
    });
    if !gueltig {
        return Err("Das Dateijournal enthält ungültige Namen; nichts angefasst.".into());
    }
    Ok(j)
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
}

/// Liest die Journale aller Projektordner unter `ablage`.
pub fn bestand(ablage: &Path) -> Bestand {
    let _g = sperren();
    let mut b = Bestand::default();
    let Ok(eintraege) = fs::read_dir(ablage) else {
        return b;
    };
    for e in eintraege.flatten() {
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
    let (sha, groesse) = (&d.entfernen_sha, d.entfernen_groesse);
    // Liegt am vermerkten Ziel schon genau diese Datei, ist das Verschieben geschehen.
    let erledigt = d
        .papierkorb
        .as_ref()
        .is_some_and(|z| gleiche_datei(&korb.join(z), sha, groesse));
    let mut ziel = d.papierkorb.clone();
    if !erledigt {
        if gleiche_datei(&quelle, sha, groesse) {
            // Ziel fehlt oder ist inzwischen von etwas anderem belegt: erst ein neues freies Ziel
            // vermerken, dann verschieben.
            if ziel.as_ref().is_none_or(|z| korb.join(z).exists()) {
                ziel = korb_ziel(&dir, &d.name)?;
                if let Some(x) = j.dateien.get_mut(id) {
                    x.papierkorb = ziel.clone();
                }
                journal_sichern(&dir, &j)?;
            }
            let z = ziel.clone().ok_or("Kein Ziel im Papierkorb.")?;
            verschieben_ohne_ersetzen(&quelle, &korb.join(z))
                .map_err(|e| format!("Datei nicht in den Papierkorb verschiebbar: {e}"))?;
        } else {
            // Die vermerkte Datei ist nicht mehr da; was jetzt unter dem Namen liegt, ist eine
            // andere Datei und bleibt unberührt (der Wiederanlauf bietet sie als gefunden an).
            ziel = None;
        }
    }
    let x = j.dateien.get_mut(id).ok_or("Datei nicht gefunden.")?;
    x.papierkorb = ziel;
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
    for id in offen {
        entfernen_fortsetzen(ablage, projekt, &id)?;
        w.entfernt += 1;
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
