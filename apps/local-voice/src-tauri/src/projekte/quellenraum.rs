//! Quellenraum und gerätelokale Quellenversionen (lva#622).
//! Gespeichert werden nur Verweise, Hashes und Zähler, niemals Quelltexte.

use super::kontext::{formen, Form, KontextForm, KontextUebersicht, Mitglied, Sorte, Wahl};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use unicode_normalization::UnicodeNormalization;

const DATEI: &str = "quellenversionen.json";
static SPERRE: Mutex<()> = Mutex::new(());

fn sperren() -> MutexGuard<'static, ()> {
    SPERRE.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Serialize, Deserialize)]
struct Datei {
    v: u32,
    quellen: BTreeMap<String, Quelle>,
}

#[derive(Default, Serialize, Deserialize)]
struct Quelle {
    versionen: Vec<Version>,
}

#[derive(Serialize, Deserialize)]
struct Version {
    nr: u32,
    hash: String,
    ms: i64,
}

fn jetzt_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn lesen_roh(ablage: &Path) -> Result<Option<Datei>, ()> {
    match fs::read(ablage.join(DATEI)) {
        Ok(roh) => match serde_json::from_slice::<Datei>(&roh) {
            Ok(d) if d.v == 1 => Ok(Some(d)),
            _ => Err(()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

fn sichern(ablage: &Path, d: &Datei) -> Result<(), String> {
    fs::create_dir_all(ablage).map_err(|_| "Projektordner nicht anlegbar.".to_string())?;
    let roh = serde_json::to_vec_pretty(d)
        .map_err(|_| "Quellenversionen nicht kodierbar.".to_string())?;
    let epoche = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = ablage.join(format!(".{DATEI}.{}-{epoche}.tmp", std::process::id()));
    let schreiben = (|| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(&roh)?;
        f.sync_all()?;
        fs::rename(&tmp, ablage.join(DATEI))
    })();
    if schreiben.is_err() {
        let _ = fs::remove_file(&tmp);
        return Err("Quellenversionen nicht speicherbar.".into());
    }
    Ok(())
}

/// Erfasst nur Änderungen gegenüber der letzten Version; höchstens 50 bleiben erhalten.
pub fn erfassen(ablage: &Path, link: &str, hash: &str) -> Result<(u32, u32), String> {
    let _g = sperren();
    let mut d = match lesen_roh(ablage) {
        Ok(Some(d)) => d,
        lage => {
            if lage.is_err() {
                // Eine vorhandene Sicherung wird auch bei mehreren Defekten nicht überschrieben.
                let mut ms = jetzt_ms();
                let mut ziel = ablage.join(format!("{DATEI}.defekt-{ms}"));
                while ziel.exists() {
                    ms = ms.checked_add(1).ok_or("Sicherung nicht anlegbar.")?;
                    ziel = ablage.join(format!("{DATEI}.defekt-{ms}"));
                }
                fs::rename(ablage.join(DATEI), ziel)
                    .map_err(|_| "Defekte Quellenversionen nicht beiseitelegbar.".to_string())?;
            }
            Datei {
                v: 1,
                quellen: BTreeMap::new(),
            }
        }
    };
    let q = d.quellen.entry(link.into()).or_default();
    if let Some(v) = q.versionen.last() {
        if v.hash == hash {
            return Ok((v.nr, q.versionen.len() as u32));
        }
    }
    let nr = match q.versionen.last() {
        Some(v) => v.nr.checked_add(1).ok_or("Versionszähler erschöpft.")?,
        None => 1,
    };
    q.versionen.push(Version {
        nr,
        hash: hash.into(),
        ms: jetzt_ms(),
    });
    if q.versionen.len() > 50 {
        q.versionen.drain(..q.versionen.len() - 50);
    }
    let anzahl = q.versionen.len() as u32;
    sichern(ablage, &d)?;
    Ok((nr, anzahl))
}

/// Liest ohne Änderungen; ein defekter Bestand liefert keine Version.
pub fn lesen(ablage: &Path, link: &str) -> Option<(u32, u32)> {
    let _g = sperren();
    let d = lesen_roh(ablage).ok()??;
    let q = d.quellen.get(link)?;
    Some((q.versionen.last()?.nr, q.versionen.len() as u32))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Inhaltsmerkmal über NFC, auf 128 Bit in kleingeschriebener Hexdarstellung gekürzt.
pub fn inhalt_hash(text: &str) -> String {
    let normalisiert: String = text.nfc().collect();
    hex(&Sha256::digest(normalisiert.as_bytes()))[..32].to_string()
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuellenZustand {
    Bereit,
    KeinText,
    NichtVerfuegbar,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuellenArt {
    Buch,
    Aufnahme,
    Ablage,
    Text,
    Datei,
    Recherche,
    Sonstige,
}

fn art(link: &str) -> QuellenArt {
    match link
        .strip_prefix("wai://lva/")
        .and_then(|s| s.split('/').next())
    {
        Some("buch") => QuellenArt::Buch,
        Some("besprechung") => QuellenArt::Aufnahme,
        Some("ablage") => QuellenArt::Ablage,
        Some("seite") => QuellenArt::Text,
        Some("projektdatei") => QuellenArt::Datei,
        Some("recherche") => QuellenArt::Recherche,
        _ => QuellenArt::Sonstige,
    }
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuellenZeile {
    pub id: String,
    pub kind: String,
    pub art: QuellenArt,
    pub title: Option<String>,
    pub link: Option<String>,
    pub state: QuellenZustand,
    pub selected: bool,
    pub form: Option<KontextForm>,
    pub version: Option<u32>,
    pub version_count: u32,
    pub version_hash: Option<String>,
    pub tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuellenraumSicht {
    pub sources: Vec<QuellenZeile>,
    pub selected_count: u32,
    pub total: u32,
    pub stand: String,
    pub reset: bool,
}

/// Baut die Sicht; gelesene Texte verlassen diese Funktion nur als Inhaltsmerkmal.
pub fn sicht_bauen(
    ablage: &Path,
    mitglieder: &[Mitglied],
    uebersicht: &KontextUebersicht,
    lesen: &mut dyn FnMut(&Mitglied, &Wahl) -> Form,
) -> QuellenraumSicht {
    let mut sources = Vec::with_capacity(mitglieder.len());
    for m in mitglieder {
        let row = uebersicht.rows.iter().find(|r| r.id == m.id);
        let form = row.map_or(formen(m.sorte)[0], |r| r.form);
        let mut z = QuellenZeile {
            id: m.id.clone(),
            kind: m.art.clone(),
            art: art(&m.link),
            title: (!m.titel.is_empty()).then(|| m.titel.clone()),
            link: (m.lesbar && !m.link.is_empty()).then(|| m.link.clone()),
            state: QuellenZustand::NichtVerfuegbar,
            selected: m.lesbar && row.is_some_and(|r| r.on),
            form: Some(form),
            version: None,
            version_count: 0,
            version_hash: None,
            tokens: row.and_then(|r| r.tokens),
        };
        if m.lesbar {
            let inhalt = if m.sorte == Sorte::Ablage {
                Form::Auszuege
            } else {
                lesen(
                    m,
                    &Wahl {
                        an: true,
                        form,
                        fassung: None,
                    },
                )
            };
            z.state = match inhalt {
                Form::Text(t) if !t.trim().is_empty() => {
                    let hash = inhalt_hash(&t);
                    if let Ok((nr, anzahl)) = erfassen(ablage, &m.link, &hash) {
                        z.version = Some(nr);
                        z.version_count = anzahl;
                        z.version_hash = Some(hash[..8].to_string());
                    }
                    QuellenZustand::Bereit
                }
                Form::Auszuege => QuellenZustand::Bereit,
                Form::Text(_) | Form::Fehlt(_) => QuellenZustand::KeinText,
            };
        }
        sources.push(z);
    }
    QuellenraumSicht {
        selected_count: sources.iter().filter(|z| z.selected).count() as u32,
        total: sources.len() as u32,
        sources,
        stand: uebersicht.stand.clone(),
        reset: uebersicht.reset,
    }
}
