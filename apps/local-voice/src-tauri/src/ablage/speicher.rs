//! Lokale Ablage: `ablage.json` (Journal + Baum), Originale, Papierkorb (Spec §2, ADR-0029 E3/E4/E9).
//!
//! Aufbau unter `<Datenordner>/ablage/`:
//!
//! ```text
//! ablage.json                   Version, Epoche, Gerät, Ordner, Dateien, Journal, Gefundenes, Begrabenes
//! dateien/<dat>/<fas>.<typ>     Originale, unveränderlich, nach Kennung benannt
//! text/<dat>/…                  Textfassungen (D2; hier nur beim endgültigen Löschen mitentfernt)
//! papierkorb/                   Zwischenspeicher beim endgültigen Löschen, liegengebliebene Zwischendateien
//! freigabe.json                 gerätelokal (siehe `freigabe`)
//! ```
//!
//! Ablauf je Datei (wie `projekte/dateien.rs`): `dateien/<dat>/.teil-<fas>` (`create_new`) vollständig
//! kopieren und `sync_all`, Größe und SHA-256 gegen die (beim Lesen unveränderte) Quelle prüfen, ohne
//! Ersetzen in `<fas>.<typ>` umbenennen, im Journal als `abgelegt` vermerken, dann den Baum schreiben und
//! den Journaleintrag entfernen (ein Schreibvorgang). Weil Originale nach Kennung heißen, ersetzt nie eine
//! Ablage eine andere; Namenskollisionen betreffen nur den Baum (neue Fassung oder beide behalten).
//!
//! Wiederanlauf ([`Ablage::wartung`]): `abgelegt` ohne Baumeintrag wird nachgetragen, liegengebliebene
//! Zwischendateien gehen in den Papierkorb, eine Datei im Ordner ohne Journal und Baum wird als `gefunden`
//! angeboten, nie still aufgenommen; ein angefangenes Löschen wird zu Ende geführt.
//!
//! Lesbar heißt: `ablage.json` gelesen, oder sie fehlt und `dateien/`/`papierkorb/` fehlen oder sind
//! nachweislich leer. Jeder andere Fall ist unklar und blockiert, bis die Bedienung „Bestand neu aufbauen“
//! ([`Ablage::neu_aufbauen`]) ein fehlendes oder defektes Journal ersetzt. Nie Namen oder Inhalte im Log.

// `original_pfad`, `fassungen_aufraeumen` und `klasse_aus_text` sind für D2 bis D4 vorbereitet.
#![allow(dead_code)]

use super::freigabe::{self, KiStufe, KiUnklar};
use super::zusammenfuehren::{self as zf, Datei, Eltern, Fassung, Klasse, Ordner, Register};
use super::{
    datei_id_pruefen, id_gueltig, ist_umweg, jetzt_ms, neue_id, ordner_id_pruefen, schreibe_atomar,
    sperren, Fehler, Res, PRAEFIX_DATEI, PRAEFIX_FASSUNG, PRAEFIX_GERAET, PRAEFIX_ORDNER,
};
use crate::buch_dateien::{sha256_datei, sha256_stabil, verschieben_ohne_ersetzen};
use crate::library_layout::{bereinigter_name, vergleichsschluessel, MAX_NAME};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const DATEIEN: &str = "dateien";
pub const TEXT: &str = "text";
pub const PAPIERKORB: &str = "papierkorb";
pub const JOURNAL: &str = "ablage.json";
const TEIL: &str = ".teil-";
const LOESCHEN: &str = ".loeschen-";
const VERSION: u32 = 1;
/// Papierkorb: so lange bleibt ein Eintrag wiederherstellbar.
pub const PAPIERKORB_TAGE: i64 = 30;
const TAG_MS: i64 = 24 * 60 * 60 * 1000;
/// Standardordner beim ersten Start.
pub const STANDARD_ORDNER: &str = "Allgemein";
const MAX_TIEFE: usize = 24;
const MAX_DATEIEN_JE_LAUF: usize = 20_000;

/// Fehlercode des künstlichen Abbruchs in Tests (kein Aufräumen, wie bei einem Absturz).
#[cfg(test)]
pub const TEST_ABBRUCH: &str = "ablage.abbruch_test";

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Schritt {
    NachZwischendatei,
    NachHash,
    NachUmbenennen,
    NachJournal,
    LoeschenNachJournal,
}

#[cfg(test)]
thread_local! {
    pub static ABBRUCH: std::cell::Cell<Option<Schritt>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn haken(s: Schritt) -> Res<()> {
    if ABBRUCH.with(|a| a.get()) == Some(s) {
        return Err(Fehler::neu(TEST_ABBRUCH, "Abbruch (Test)"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Stand auf der Platte
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "art", rename_all = "snake_case")]
enum Eintrag {
    /// Original liegt (Größe und SHA-256 geprüft); der Baum folgt.
    Abgelegt {
        dat: String,
        fas: String,
        ordner: String,
        name: String,
        neue_datei: bool,
        vorgaenger: Option<String>,
        sha256: String,
        groesse: u64,
        typ: String,
        zeit: i64,
    },
    /// Endgültiges Löschen läuft.
    Loeschen {
        dat: Vec<String>,
        ordner: Vec<String>,
    },
}

/// Datei im Ordner, ohne Journal und Baum gefunden: wird angeboten, nicht aufgenommen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Gefunden {
    dat: String,
    fas: String,
    typ: String,
    groesse: u64,
    ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stand {
    v: u32,
    geraet: String,
    epoche: u64,
    #[serde(default)]
    ordner: BTreeMap<String, Ordner>,
    #[serde(default)]
    dateien: BTreeMap<String, Datei>,
    #[serde(default)]
    journal: Vec<Eintrag>,
    #[serde(default)]
    gefunden: Vec<Gefunden>,
    /// Endgültig gelöschte Kennungen (Grabsteine; D4 reicht sie weiter).
    #[serde(default)]
    begraben: Vec<String>,
}

impl Stand {
    fn neu() -> Self {
        Self {
            v: VERSION,
            geraet: neue_id(PRAEFIX_GERAET),
            epoche: 0,
            ordner: BTreeMap::new(),
            dateien: BTreeMap::new(),
            journal: Vec::new(),
            gefunden: Vec::new(),
            begraben: Vec::new(),
        }
    }

    /// Nächster Lamport-Zähler: größter bekannter Wert + 1.
    fn zaehler(&self) -> u64 {
        zf::groesster_zaehler(&self.ordner, &self.dateien) + 1
    }

    fn eltern(&self) -> BTreeMap<String, Eltern> {
        zf::wirksame_eltern(&self.ordner)
    }

    fn geraet(&self) -> &str {
        &self.geraet
    }

    /// Ordner oder ein Vorfahr liegt im Papierkorb.
    fn ordner_im_korb(&self, eltern: &BTreeMap<String, Eltern>, id: &str) -> bool {
        zf::kette(eltern, id).iter().any(|g| {
            self.ordner
                .get(g)
                .is_none_or(|o| o.papierkorb.wert.is_some())
        })
    }

    fn datei_lebt(&self, eltern: &BTreeMap<String, Eltern>, d: &Datei) -> bool {
        d.papierkorb.wert.is_none() && !self.ordner_im_korb(eltern, &d.ordner.wert)
    }

    /// Lebende Geschwister (Ordner) unter `eltern_id`.
    fn kinder(&self, eltern: &BTreeMap<String, Eltern>, eltern_id: Option<&str>) -> Vec<&Ordner> {
        self.ordner
            .values()
            .filter(|o| match (eltern.get(&o.id), eltern_id) {
                (Some(Eltern::Wurzel), None) => true,
                (Some(Eltern::Ordner(p)), Some(e)) => p == e,
                _ => false,
            })
            .collect()
    }

    fn namen_im_ordner(&self, ordner: &str, ohne: Option<&str>) -> BTreeSet<String> {
        self.dateien
            .values()
            .filter(|d| d.ordner.wert == ordner && d.papierkorb.wert.is_none())
            .filter(|d| Some(d.id.as_str()) != ohne)
            .map(|d| vergleichsschluessel(&d.name.wert))
            .collect()
    }

    fn pfad_text(&self, eltern: &BTreeMap<String, Eltern>, ordner: &str) -> String {
        let mut namen: Vec<&str> = zf::kette(eltern, ordner)
            .iter()
            .filter_map(|g| self.ordner.get(g).map(|o| o.name.wert.as_str()))
            .collect();
        namen.reverse();
        namen.join(" › ")
    }
}

/// Warum die Ablage nicht geladen werden kann.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AblageLage {
    /// Fehlt, obwohl Dateien vorhanden sind.
    Fehlt,
    /// Kaputt oder mit ungültigen Kennungen.
    Defekt,
    /// Von einer neueren App-Version: nicht lesbar und nie neu aufbaubar.
    Neuer,
    /// Vorübergehend nicht lesbar (Rechte, Sperre, Datenträger): später erneut.
    NichtLesbar,
}

impl AblageLage {
    fn fehler(self) -> Fehler {
        let (code, text) = match self {
            Self::Fehlt => (
                "ablage.journal_fehlt",
                "Das Ablagejournal fehlt, obwohl Dateien vorhanden sind; nichts angefasst.",
            ),
            Self::Defekt => (
                "ablage.journal_defekt",
                "Das Ablagejournal ist nicht lesbar; nichts angefasst.",
            ),
            Self::Neuer => (
                "ablage.journal_neuer",
                "Das Ablagejournal stammt von einer neueren App-Version.",
            ),
            Self::NichtLesbar => (
                "ablage.journal_nicht_lesbar",
                "Ablagejournal oder Ablageordner gerade nicht lesbar; nichts angefasst.",
            ),
        };
        Fehler::neu(code, text)
    }
}

/// Zeichen, die in einem Namen nichts zu suchen haben (er berührt nie das Dateisystem, die Originale
/// heißen nach Kennung).
fn name_lesbar(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 255 && !name.chars().any(|c| c.is_control())
}

fn typ_ok(typ: &str) -> bool {
    typ.len() <= 10 && typ.chars().all(|c| c.is_ascii_alphanumeric())
}

fn stand_gueltig(s: &Stand) -> bool {
    id_gueltig(PRAEFIX_GERAET, &s.geraet)
        && s.ordner.iter().all(|(k, o)| {
            id_gueltig(PRAEFIX_ORDNER, k)
                && *k == o.id
                && name_lesbar(&o.name.wert)
                && o.eltern
                    .wert
                    .as_deref()
                    .is_none_or(|p| id_gueltig(PRAEFIX_ORDNER, p))
        })
        && s.dateien.iter().all(|(k, d)| {
            id_gueltig(PRAEFIX_DATEI, k)
                && *k == d.id
                && name_lesbar(&d.name.wert)
                && id_gueltig(PRAEFIX_ORDNER, &d.ordner.wert)
                && d.fassungen
                    .iter()
                    .all(|f| id_gueltig(PRAEFIX_FASSUNG, &f.fas) && typ_ok(&f.typ))
        })
        && s.journal.iter().all(|e| match e {
            Eintrag::Abgelegt {
                dat,
                fas,
                ordner,
                name,
                typ,
                ..
            } => {
                id_gueltig(PRAEFIX_DATEI, dat)
                    && id_gueltig(PRAEFIX_FASSUNG, fas)
                    && id_gueltig(PRAEFIX_ORDNER, ordner)
                    && name_lesbar(name)
                    && typ_ok(typ)
            }
            Eintrag::Loeschen { dat, ordner } => {
                dat.iter().all(|d| id_gueltig(PRAEFIX_DATEI, d))
                    && ordner.iter().all(|o| id_gueltig(PRAEFIX_ORDNER, o))
            }
        })
        && s.gefunden.iter().all(|g| {
            id_gueltig(PRAEFIX_DATEI, &g.dat)
                && id_gueltig(PRAEFIX_FASSUNG, &g.fas)
                && typ_ok(&g.typ)
        })
}

// ---------------------------------------------------------------------------
// Namen
// ---------------------------------------------------------------------------

/// Stamm und Endung (mit Punkt); ein führender Punkt gehört zum Stamm.
fn teilen(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// Endung für den Dateinamen des Originals: Kleinbuchstaben und Ziffern, höchstens 10 Zeichen.
fn typ_aus(name: &str) -> String {
    let (stamm, endung) = teilen(name);
    let e = endung.trim_start_matches('.');
    if stamm.is_empty() || e.is_empty() || !typ_ok(e) {
        return String::new();
    }
    e.to_ascii_lowercase()
}

fn original_name(fas: &str, typ: &str) -> String {
    if typ.is_empty() {
        fas.to_string()
    } else {
        format!("{fas}.{typ}")
    }
}

/// Freier Name im Sinne von `Name (2).ext`; `belegt` enthält Vergleichsschlüssel.
fn freier_name(belegt: &BTreeSet<String>, wunsch: &str) -> String {
    if !belegt.contains(&vergleichsschluessel(wunsch)) {
        return wunsch.to_string();
    }
    let (stamm, endung) = teilen(wunsch);
    for n in 2..10_000u32 {
        let zusatz = format!(" ({n})");
        let platz = MAX_NAME
            .saturating_sub(zusatz.chars().count() + endung.chars().count())
            .max(1);
        let kurz: String = stamm.chars().take(platz).collect();
        let kandidat = format!("{}{zusatz}{endung}", kurz.trim_end());
        if !belegt.contains(&vergleichsschluessel(&kandidat)) {
            return kandidat;
        }
    }
    format!("{wunsch} ({})", neue_id(""))
}

fn bereinigen(name: &str) -> Res<String> {
    bereinigter_name(name).map_err(|_| Fehler::neu("ablage.name_ungueltig", "Ungültiger Name."))
}

/// Kein Glied von `rel` unter `wurzel` ist eine Verknüpfung oder ein Reparse-Point; ein fehlendes Glied
/// beendet die Prüfung.
fn ohne_umweg(wurzel: &Path, rel: &str) -> Res<()> {
    let mut pfad = wurzel.to_path_buf();
    for teil in rel.split('/').filter(|t| !t.is_empty()) {
        pfad.push(teil);
        match fs::symlink_metadata(&pfad) {
            Ok(m) if ist_umweg(&m) => {
                return Err(Fehler::neu(
                    "ablage.verknuepfung",
                    "Der Pfad führt über eine Verknüpfung; nichts angefasst.",
                ))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(Fehler::speicher("Pfad nicht lesbar; nichts angefasst.")),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Oberfläche: Sichten
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageKi {
    /// `an`, `aus`, `gesperrt` oder `unklar`.
    pub zustand: String,
    /// Ordner, von dem der Zustand stammt, wenn geerbt.
    pub von: Option<String>,
    /// Name dieses Ordners (für „geerbt von …“).
    pub von_name: Option<String>,
    /// Kein Eintrag in der Kette (Vorgabe aus).
    pub vorgabe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageOrdner {
    pub id: String,
    pub name: String,
    /// Wirksamer Elternordner; `null` = oberste Ebene.
    pub eltern: Option<String>,
    /// Zyklus oder begrabener Elternordner: vorläufig unter „Wiederhergestellt“.
    pub wiederhergestellt: bool,
    /// Gespeicherte Datenklasse (`erbt`, `public`, …).
    pub datenklasse: String,
    /// Wirksame Klasse (`public` … `personal`, `unbekannt`).
    pub klasse: String,
    pub nur_geraet: bool,
    pub nur_geraet_wirksam: bool,
    pub zielordner_ki: bool,
    pub ki: AblageKi,
    pub dateien: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageDatei {
    pub id: String,
    pub name: String,
    pub ordner: String,
    pub typ: String,
    pub groesse: u64,
    pub geaendert: i64,
    pub fassungen: u32,
    pub konflikt: bool,
    pub klasse: String,
    pub ki: AblageKi,
    pub nur_geraet: bool,
    /// Stand der jüngsten Textfassung (`ok`, `kein_text`, `fehler`); `null` = noch keine (D2).
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AblageArt {
    Datei,
    Ordner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageKorb {
    pub art: AblageArt,
    pub id: String,
    pub name: String,
    pub typ: String,
    pub geloescht: i64,
    pub tage_rest: i64,
    pub ursprung: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageGefunden {
    pub dat: String,
    pub typ: String,
    pub groesse: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageSicht {
    /// `null` = lesbar; sonst blockiert alles bis zur Bedienung „Bestand neu aufbauen“ (falls möglich).
    pub lage: Option<AblageLage>,
    /// Freigabe unklar: die ganze Ablage ist für die KI gesperrt.
    pub freigabe_unklar: Option<KiUnklar>,
    pub ordner: Vec<AblageOrdner>,
    pub dateien: Vec<AblageDatei>,
    pub papierkorb: Vec<AblageKorb>,
    pub gefunden: Vec<AblageGefunden>,
    /// Summe der Größen aller Fassungen (lokal belegt).
    pub belegt: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageFassung {
    pub fas: String,
    pub nummer: u32,
    pub vorgaenger: Option<String>,
    pub groesse: u64,
    pub typ: String,
    pub zeit: i64,
    pub aktuell: bool,
    pub kopf: bool,
    pub abgeloest: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AblageKollisionswahl {
    NeueFassung,
    BeideBehalten,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AblageImportStand {
    Abgelegt,
    NeueFassung,
    /// Name vorhanden: nichts geschrieben, die Bedienung wählt.
    Kollision,
    Uebersprungen,
    Fehler,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageImportZeile {
    pub quelle: String,
    pub stand: AblageImportStand,
    /// Zielordner dieser Datei (bei Ordnerimport der Unterordner): für die Wiederholung mit Wahl.
    pub ordner: Option<String>,
    pub dat: Option<String>,
    pub fas: Option<String>,
    pub name: Option<String>,
    pub code: Option<String>,
}

impl AblageImportZeile {
    fn fehler(quelle: &Path, e: &Fehler) -> Self {
        Self {
            quelle: quelle.to_string_lossy().into_owned(),
            stand: AblageImportStand::Fehler,
            ordner: None,
            dat: None,
            fas: None,
            name: None,
            code: Some(e.code.clone()),
        }
    }

    fn uebersprungen(quelle: &Path, code: &str) -> Self {
        Self {
            quelle: quelle.to_string_lossy().into_owned(),
            stand: AblageImportStand::Uebersprungen,
            ordner: None,
            dat: None,
            fas: None,
            name: None,
            code: Some(code.to_string()),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, specta::Type)]
pub struct AblageEinstellungen {
    pub datenklasse: Option<String>,
    pub nur_geraet: Option<bool>,
    pub zielordner_ki: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageLoeschBericht {
    pub dateien: u32,
    pub ordner: u32,
}

const KLASSEN: [&str; 5] = ["erbt", "public", "internal", "confidential", "personal"];

fn ki_sicht(st: &Stand, lage: &freigabe::Lage, kette: &[String], zusatz: &[String]) -> AblageKi {
    let mut w = freigabe::wirksam(lage, kette);
    // Unter „Wiederhergestellt“ gilt zusätzlich eine Sperre der gespeicherten Elternkette.
    let z = freigabe::wirksam(lage, zusatz);
    if z.stufe == KiStufe::Gesperrt && w.stufe != KiStufe::Gesperrt {
        w = z;
    }
    AblageKi {
        zustand: if w.unklar {
            "unklar".into()
        } else {
            match w.stufe {
                KiStufe::An => "an",
                KiStufe::Aus => "aus",
                KiStufe::Gesperrt => "gesperrt",
            }
            .into()
        },
        von_name: w
            .von
            .as_ref()
            .and_then(|v| st.ordner.get(v))
            .map(|o| o.name.wert.clone()),
        von: w.von,
        vorgabe: w.vorgabe,
    }
}

// ---------------------------------------------------------------------------
// Die Ablage
// ---------------------------------------------------------------------------

/// Die lokale Ablage unter `dir` (= `<Datenordner>/ablage`). Jede Methode sperrt und liest den Stand
/// frisch von der Platte; es gibt keinen zwischengespeicherten Zustand, der veralten könnte.
#[derive(Debug, Clone)]
pub struct Ablage {
    dir: PathBuf,
}

fn gescannt() -> &'static std::sync::Mutex<BTreeSet<PathBuf>> {
    static G: std::sync::OnceLock<std::sync::Mutex<BTreeSet<PathBuf>>> = std::sync::OnceLock::new();
    G.get_or_init(|| std::sync::Mutex::new(BTreeSet::new()))
}

impl Ablage {
    pub fn neu(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Test: so tun, als wäre der Prozess neu gestartet (die Suche nach Funden läuft wieder).
    #[cfg(test)]
    pub fn prozess_neustart(&self) {
        if let Ok(mut g) = gescannt().lock() {
            g.remove(&self.dir);
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn journal_pfad(&self) -> PathBuf {
        self.dir.join(JOURNAL)
    }

    /// Liegt in `dateien/` oder `papierkorb/` etwas? Leer ist nur, was fehlt oder nachweislich leer
    /// gelesen wurde; jeder andere Lesefehler ist ein Fehler (= unklar).
    fn bestand_vorhanden(&self) -> Result<bool, ()> {
        for u in [DATEIEN, PAPIERKORB] {
            match fs::read_dir(self.dir.join(u)) {
                Ok(mut e) => match e.next() {
                    None => {}
                    Some(Ok(_)) => return Ok(true),
                    Some(Err(_)) => return Err(()),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(()),
            }
        }
        Ok(false)
    }

    fn lage(&self) -> Result<(Stand, bool), AblageLage> {
        match fs::symlink_metadata(&self.dir) {
            Ok(m) if ist_umweg(&m) => return Err(AblageLage::NichtLesbar),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(AblageLage::NichtLesbar),
        }
        let roh = match fs::read(self.journal_pfad()) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return match self.bestand_vorhanden() {
                    Ok(true) => Err(AblageLage::Fehlt),
                    Ok(false) => Ok((Stand::neu(), true)),
                    Err(()) => Err(AblageLage::NichtLesbar),
                };
            }
            Err(_) => return Err(AblageLage::NichtLesbar),
        };
        #[derive(Deserialize)]
        struct NurVersion {
            v: u32,
        }
        match serde_json::from_slice::<NurVersion>(&roh) {
            Ok(n) if n.v > VERSION => return Err(AblageLage::Neuer),
            Ok(_) => {}
            Err(_) => return Err(AblageLage::Defekt),
        }
        match serde_json::from_slice::<Stand>(&roh) {
            Ok(s) if stand_gueltig(&s) => Ok((s, false)),
            _ => Err(AblageLage::Defekt),
        }
    }

    fn laden(&self) -> Res<Stand> {
        let (mut s, frisch) = self.lage().map_err(AblageLage::fehler)?;
        // Ein frischer Stand wird sofort gespeichert, damit die Gerätekennung bestehen bleibt.
        if frisch {
            self.sichern(&mut s)?;
        }
        Ok(s)
    }

    fn sichern(&self, st: &mut Stand) -> Res<()> {
        st.epoche += 1;
        fs::create_dir_all(&self.dir)
            .map_err(|_| Fehler::speicher("Ablageordner nicht anlegbar."))?;
        let roh = serde_json::to_vec_pretty(st)
            .map_err(|_| Fehler::speicher("Ablagejournal nicht speicherbar."))?;
        schreibe_atomar(&self.journal_pfad(), &roh)
    }

    // --- Wiederanlauf und Wartung -------------------------------------------------

    /// Öffnet die Ablage: legt Ordner an, führt das Journal zu Ende, räumt Zwischendateien weg, sucht
    /// (einmal je Prozess) nach Dateien ohne Eintrag, löscht abgelaufene Papierkorb-Einträge und legt beim
    /// allerersten Start den Ordner „Allgemein“ an. Liefert die Lage, wenn die Ablage nicht lesbar ist.
    pub fn wartung(&self) -> Option<AblageLage> {
        let _g = sperren();
        self.wartung_ohne_sperre()
    }

    fn wartung_ohne_sperre(&self) -> Option<AblageLage> {
        let (mut st, frisch) = match self.lage() {
            Ok(x) => x,
            Err(l) => return Some(l),
        };
        let mut geaendert = false;
        let _ = fs::create_dir_all(self.dir.join(DATEIEN));
        let _ = fs::create_dir_all(self.dir.join(PAPIERKORB));
        geaendert |= self.zwischendateien_raeumen();
        geaendert |= self.journal_abarbeiten(&mut st).unwrap_or(true);
        let erster_lauf = gescannt()
            .lock()
            .map(|mut g| g.insert(self.dir.clone()))
            .unwrap_or(false);
        if erster_lauf {
            geaendert |= self.gefundenes_suchen(&mut st);
        }
        geaendert |= self.abgelaufene_loeschen(&mut st, jetzt_ms());
        self.teildateien_loeschen(Some(jetzt_ms()));
        if st.ordner.is_empty() && st.dateien.is_empty() && st.begraben.is_empty() {
            let z = st.zaehler();
            let o = neuer_ordner(st.geraet(), None, STANDARD_ORDNER, z);
            st.ordner.insert(o.id.clone(), o);
            geaendert = true;
        }
        if (geaendert || frisch) && self.sichern(&mut st).is_err() {
            return Some(AblageLage::NichtLesbar);
        }
        None
    }

    /// Liegengebliebene Zwischendateien (`.teil-*`) in `dateien/<dat>/` gehen in den Papierkorb.
    fn zwischendateien_raeumen(&self) -> bool {
        let mut bewegt = false;
        let Ok(dirs) = fs::read_dir(self.dir.join(DATEIEN)) else {
            return false;
        };
        for d in dirs.flatten() {
            let dat = d.file_name().to_string_lossy().into_owned();
            if !id_gueltig(PRAEFIX_DATEI, &dat) {
                continue;
            }
            let Ok(inhalt) = fs::read_dir(d.path()) else {
                continue;
            };
            for f in inhalt.flatten() {
                let n = f.file_name().to_string_lossy().into_owned();
                let Some(rest) = n.strip_prefix(TEIL) else {
                    continue;
                };
                if !id_gueltig(PRAEFIX_FASSUNG, rest) {
                    continue;
                }
                let Ok(m) = fs::symlink_metadata(f.path()) else {
                    continue;
                };
                if !m.is_file() {
                    continue;
                }
                let ziel = self.dir.join(PAPIERKORB).join(format!("teil-{rest}"));
                if verschieben_ohne_ersetzen(&f.path(), &ziel).is_ok() {
                    bewegt = true;
                }
            }
            // Ein leeres Verzeichnis ohne Original (Abbruch vor der Ablage) wird entfernt.
            let _ = fs::remove_dir(d.path());
        }
        bewegt
    }

    /// Verschobene Zwischendateien (`papierkorb/teil-*`) endgültig entfernen: mit `None` alle (Papierkorb
    /// leeren), mit `Some(jetzt)` nur die, die älter als die Papierkorbfrist sind (Spec §10).
    fn teildateien_loeschen(&self, jetzt: Option<i64>) {
        let Ok(rest) = fs::read_dir(self.dir.join(PAPIERKORB)) else {
            return;
        };
        for e in rest.flatten() {
            if !e.file_name().to_string_lossy().starts_with("teil-") {
                continue;
            }
            let Ok(m) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            if !m.is_file() {
                continue;
            }
            if let Some(jetzt) = jetzt {
                let alt = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .is_some_and(|t| jetzt - t >= PAPIERKORB_TAGE * TAG_MS);
                if !alt {
                    continue;
                }
            }
            let _ = fs::remove_file(e.path());
        }
    }

    /// Journal zu Ende führen. Liefert, ob etwas geändert wurde. Einträge gehen nie verloren: ein
    /// fehlgeschlagener Löscheintrag und alle danach bleiben im Journal (für den nächsten Lauf); ein
    /// `Abgelegt`-Eintrag mit fehlgeschlagener Prüfung wird als `gefunden` angeboten, das Original bleibt.
    fn journal_abarbeiten(&self, st: &mut Stand) -> Res<bool> {
        if st.journal.is_empty() {
            return Ok(false);
        }
        let eintraege = std::mem::take(&mut st.journal);
        let mut rest: Vec<Eintrag> = Vec::new();
        let mut fehler: Option<Fehler> = None;
        let mut it = eintraege.into_iter();
        while let Some(e) = it.next() {
            match &e {
                Eintrag::Abgelegt {
                    dat,
                    fas,
                    typ,
                    sha256,
                    groesse,
                    ..
                } => {
                    let schon = st
                        .dateien
                        .get(dat)
                        .is_some_and(|d| d.fassungen.iter().any(|f| &f.fas == fas));
                    if schon {
                        continue;
                    }
                    let pfad = self
                        .dir
                        .join(DATEIEN)
                        .join(dat)
                        .join(original_name(fas, typ));
                    let ok = ohne_umweg(&self.dir, &format!("{DATEIEN}/{dat}")).is_ok()
                        && fs::symlink_metadata(&pfad)
                            .is_ok_and(|m| m.is_file() && m.len() == *groesse)
                        && sha256_datei(&pfad).is_ok_and(|s| &s == sha256);
                    if ok {
                        anwenden_abgelegt(st, &e);
                        continue;
                    }
                    let vorhanden = fs::symlink_metadata(&pfad)
                        .ok()
                        .filter(|m| m.is_file() && !ist_umweg(m));
                    let bekannt =
                        st.gefunden.iter().any(|g| &g.fas == fas) || st.begraben.contains(dat);
                    if let (Some(m), false) = (vorhanden, bekannt) {
                        st.gefunden.push(Gefunden {
                            dat: dat.clone(),
                            fas: fas.clone(),
                            typ: typ.clone(),
                            groesse: m.len(),
                            ms: jetzt_ms(),
                        });
                    }
                }
                Eintrag::Loeschen { dat, ordner } => {
                    if let Err(x) = self.loeschen_ausfuehren(st, dat, ordner) {
                        fehler = Some(x);
                        rest.push(e);
                        rest.extend(it);
                        break;
                    }
                }
            }
        }
        st.journal = rest;
        match fehler {
            Some(x) => Err(x),
            None => Ok(true),
        }
    }

    /// Dateien in `dateien/<dat>/`, die weder im Baum noch im Journal stehen: als `gefunden` vermerken.
    fn gefundenes_suchen(&self, st: &mut Stand) -> bool {
        let Ok(dirs) = fs::read_dir(self.dir.join(DATEIEN)) else {
            return false;
        };
        let mut neu = false;
        for d in dirs.flatten() {
            let dat = d.file_name().to_string_lossy().into_owned();
            if !id_gueltig(PRAEFIX_DATEI, &dat) {
                continue;
            }
            let Ok(m) = fs::symlink_metadata(d.path()) else {
                continue;
            };
            if ist_umweg(&m) || !m.is_dir() {
                continue;
            }
            let Ok(inhalt) = fs::read_dir(d.path()) else {
                continue;
            };
            for f in inhalt.flatten() {
                let n = f.file_name().to_string_lossy().into_owned();
                let (fas, typ) = match n.split_once('.') {
                    Some((a, b)) => (a.to_string(), b.to_string()),
                    None => (n.clone(), String::new()),
                };
                if !id_gueltig(PRAEFIX_FASSUNG, &fas) || !typ_ok(&typ) {
                    continue;
                }
                let Ok(fm) = fs::symlink_metadata(f.path()) else {
                    continue;
                };
                if ist_umweg(&fm) || !fm.is_file() {
                    continue;
                }
                let bekannt = st.dateien.get(&dat).is_some_and(|x| {
                    x.fassungen.iter().any(|f| f.fas == fas) || x.entfernt.contains(&fas)
                }) || st.gefunden.iter().any(|g| g.fas == fas)
                    || st.begraben.contains(&dat);
                if bekannt {
                    continue;
                }
                st.gefunden.push(Gefunden {
                    dat: dat.clone(),
                    fas,
                    typ,
                    groesse: fm.len(),
                    ms: jetzt_ms(),
                });
                neu = true;
            }
        }
        neu
    }

    fn abgelaufene_loeschen(&self, st: &mut Stand, jetzt: i64) -> bool {
        let eltern = st.eltern();
        let abgelaufen = |p: &Register<Option<i64>>| {
            p.wert
                .is_some_and(|t| jetzt - t >= PAPIERKORB_TAGE * TAG_MS)
        };
        let dat: Vec<String> = st
            .dateien
            .values()
            .filter(|d| abgelaufen(&d.papierkorb) && !st.ordner_im_korb(&eltern, &d.ordner.wert))
            .map(|d| d.id.clone())
            .collect();
        let ord: Vec<String> = st
            .ordner
            .values()
            .filter(|o| {
                abgelaufen(&o.papierkorb)
                    && match eltern.get(&o.id) {
                        Some(Eltern::Ordner(p)) => !st.ordner_im_korb(&eltern, p),
                        _ => true,
                    }
            })
            .map(|o| o.id.clone())
            .collect();
        let mut getan = false;
        for d in dat {
            if self.endgueltig_intern(st, AblageArt::Datei, &d).is_ok() {
                getan = true;
            }
        }
        for o in ord {
            if st.ordner.contains_key(&o)
                && self.endgueltig_intern(st, AblageArt::Ordner, &o).is_ok()
            {
                getan = true;
            }
        }
        getan
    }

    // --- Ansicht -----------------------------------------------------------------

    /// Gesamtstand für die Oberfläche (führt vorher die Wartung aus).
    pub fn ansicht(&self) -> Res<AblageSicht> {
        let _g = sperren();
        if let Some(l) = self.wartung_ohne_sperre() {
            return Ok(AblageSicht {
                lage: Some(l),
                freigabe_unklar: None,
                ordner: vec![],
                dateien: vec![],
                papierkorb: vec![],
                gefunden: vec![],
                belegt: 0,
            });
        }
        let st = self.laden()?;
        let lage = freigabe::laden(&self.dir);
        let eltern = st.eltern();
        let jetzt = jetzt_ms();
        let mut ordner: Vec<AblageOrdner> = st
            .ordner
            .values()
            .filter(|o| !st.ordner_im_korb(&eltern, &o.id))
            .map(|o| {
                let kette = zf::kette(&eltern, &o.id);
                let e = eltern.get(&o.id);
                AblageOrdner {
                    id: o.id.clone(),
                    name: o.name.wert.clone(),
                    eltern: match e {
                        Some(Eltern::Ordner(p)) => Some(p.clone()),
                        _ => None,
                    },
                    wiederhergestellt: matches!(e, Some(Eltern::Wiederhergestellt)),
                    datenklasse: o.datenklasse.wert.clone(),
                    klasse: zf::wirksame_klasse(&st.ordner, &eltern, &o.id)
                        .text()
                        .into(),
                    nur_geraet: o.nur_geraet.wert,
                    nur_geraet_wirksam: zf::wirksam_nur_geraet(&st.ordner, &eltern, &o.id),
                    zielordner_ki: o.zielordner_ki.wert,
                    ki: ki_sicht(
                        &st,
                        &lage,
                        &kette,
                        &zf::zusatzkette(&st.ordner, &eltern, &o.id),
                    ),
                    dateien: st
                        .dateien
                        .values()
                        .filter(|d| d.ordner.wert == o.id && d.papierkorb.wert.is_none())
                        .count() as u32,
                }
            })
            .collect();
        ordner.sort_by_key(|o| vergleichsschluessel(&o.name));
        let mut dateien: Vec<AblageDatei> = st
            .dateien
            .values()
            .filter(|d| st.datei_lebt(&eltern, d))
            .map(|d| datei_sicht(&st, &eltern, &lage, d))
            .collect();
        dateien.sort_by_key(|d| vergleichsschluessel(&d.name));
        let mut korb = Vec::new();
        for d in st.dateien.values() {
            if let Some(t) = d.papierkorb.wert {
                if !st.ordner_im_korb(&eltern, &d.ordner.wert) {
                    korb.push(AblageKorb {
                        art: AblageArt::Datei,
                        id: d.id.clone(),
                        name: d.name.wert.clone(),
                        typ: aktuelle_fassung(d)
                            .map(|f| f.typ.clone())
                            .unwrap_or_default(),
                        geloescht: t,
                        tage_rest: tage_rest(t, jetzt),
                        ursprung: st.pfad_text(&eltern, &d.ordner.wert),
                    });
                }
            }
        }
        for o in st.ordner.values() {
            let Some(t) = o.papierkorb.wert else { continue };
            let eltern_im_korb = match eltern.get(&o.id) {
                Some(Eltern::Ordner(p)) => st.ordner_im_korb(&eltern, p),
                _ => false,
            };
            if !eltern_im_korb {
                korb.push(AblageKorb {
                    art: AblageArt::Ordner,
                    id: o.id.clone(),
                    name: o.name.wert.clone(),
                    typ: String::new(),
                    geloescht: t,
                    tage_rest: tage_rest(t, jetzt),
                    ursprung: match eltern.get(&o.id) {
                        Some(Eltern::Ordner(p)) => st.pfad_text(&eltern, p),
                        _ => String::new(),
                    },
                });
            }
        }
        korb.sort_by(|a, b| b.geloescht.cmp(&a.geloescht).then_with(|| a.id.cmp(&b.id)));
        let belegt = st
            .dateien
            .values()
            .flat_map(|d| d.fassungen.iter().map(|f| f.size))
            .sum();
        Ok(AblageSicht {
            lage: None,
            freigabe_unklar: match &lage {
                freigabe::Lage::Unklar(u) => Some(*u),
                _ => None,
            },
            ordner,
            dateien,
            papierkorb: korb,
            gefunden: st
                .gefunden
                .iter()
                .map(|g| AblageGefunden {
                    dat: g.dat.clone(),
                    typ: g.typ.clone(),
                    groesse: g.groesse,
                })
                .collect(),
            belegt,
        })
    }

    /// Fassungsliste einer Datei, neueste zuerst.
    pub fn fassungen(&self, dat: &str) -> Res<Vec<AblageFassung>> {
        datei_id_pruefen(dat)?;
        let _g = sperren();
        let st = self.laden()?;
        let d = st
            .dateien
            .get(dat)
            .ok_or_else(|| Fehler::neu("ablage.nicht_gefunden", "Datei nicht gefunden."))?;
        let koepfe: BTreeSet<String> = zf::koepfe(d).into_iter().collect();
        let abgel = zf::abgeloest(d);
        let aktuell = zf::aktuell(d);
        let mut chrono: Vec<&Fassung> = d.fassungen.iter().collect();
        chrono.sort_by(|a, b| a.zeit.cmp(&b.zeit).then_with(|| a.fas.cmp(&b.fas)));
        let mut aus: Vec<AblageFassung> = chrono
            .iter()
            .enumerate()
            .map(|(i, f)| AblageFassung {
                fas: f.fas.clone(),
                nummer: i as u32 + 1,
                vorgaenger: f.vorgaenger.clone(),
                groesse: f.size,
                typ: f.typ.clone(),
                zeit: f.zeit,
                aktuell: aktuell.as_deref() == Some(f.fas.as_str()),
                kopf: koepfe.contains(&f.fas),
                abgeloest: abgel.contains(&f.fas),
            })
            .collect();
        aus.reverse();
        Ok(aus)
    }

    // --- Ordner ------------------------------------------------------------------

    pub fn ordner_anlegen(&self, eltern_id: Option<&str>, name: &str) -> Res<String> {
        let name = bereinigen(name)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        if let Some(e) = eltern_id {
            ordner_id_pruefen(e)?;
            self.ordner_lebend(&st, &eltern, e)?;
        }
        gleichnamig_pruefen(&st, &eltern, eltern_id, &name, None)?;
        let z = st.zaehler();
        let o = neuer_ordner(st.geraet(), eltern_id, &name, z);
        let id = o.id.clone();
        st.ordner.insert(id.clone(), o);
        self.sichern(&mut st)?;
        Ok(id)
    }

    fn ordner_lebend(&self, st: &Stand, eltern: &BTreeMap<String, Eltern>, id: &str) -> Res<()> {
        if !st.ordner.contains_key(id) {
            return Err(Fehler::neu(
                "ablage.nicht_gefunden",
                "Ordner nicht gefunden.",
            ));
        }
        if st.ordner_im_korb(eltern, id) {
            return Err(Fehler::neu(
                "ablage.ordner_im_papierkorb",
                "Der Ordner liegt im Papierkorb.",
            ));
        }
        Ok(())
    }

    pub fn ordner_umbenennen(&self, id: &str, name: &str) -> Res<()> {
        ordner_id_pruefen(id)?;
        let name = bereinigen(name)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, id)?;
        let e = match eltern.get(id) {
            Some(Eltern::Ordner(p)) => Some(p.clone()),
            _ => None,
        };
        gleichnamig_pruefen(&st, &eltern, e.as_deref(), &name, Some(id))?;
        let (z, g) = (st.zaehler(), st.geraet().to_string());
        let o = st.ordner.get_mut(id).ok_or_else(nicht_gefunden)?;
        o.name = Register::neu(name, z, &g);
        self.sichern(&mut st)
    }

    /// Verschiebt einen Ordner (`None` = oberste Ebene). Ziele im eigenen Unterbaum werden abgewiesen.
    pub fn ordner_verschieben(&self, id: &str, neu_eltern: Option<&str>) -> Res<()> {
        ordner_id_pruefen(id)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, id)?;
        if let Some(e) = neu_eltern {
            ordner_id_pruefen(e)?;
            self.ordner_lebend(&st, &eltern, e)?;
            if zf::kette(&eltern, e).iter().any(|g| g == id) {
                return Err(Fehler::neu(
                    "ablage.zyklus",
                    "Ein Ordner lässt sich nicht in sich selbst verschieben.",
                ));
            }
        }
        let name = st
            .ordner
            .get(id)
            .ok_or_else(nicht_gefunden)?
            .name
            .wert
            .clone();
        gleichnamig_pruefen(&st, &eltern, neu_eltern, &name, Some(id))?;
        let (z, g) = (st.zaehler(), st.geraet().to_string());
        let o = st.ordner.get_mut(id).ok_or_else(nicht_gefunden)?;
        o.eltern = Register::neu(neu_eltern.map(str::to_string), z, &g);
        self.sichern(&mut st)
    }

    /// Datenklasse, „Nur dieses Gerät“ und „KI-Entwürfe“ eines Ordners.
    pub fn ordner_einstellungen(&self, id: &str, e: &AblageEinstellungen) -> Res<()> {
        ordner_id_pruefen(id)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, id)?;
        if let Some(k) = &e.datenklasse {
            if !KLASSEN.contains(&k.as_str()) {
                return Err(Fehler::ungueltig("Unbekannte Datenklasse."));
            }
        }
        if e.nur_geraet == Some(false) {
            let kette = zf::strenge_kette(&st.ordner, &eltern, id);
            let darueber = kette
                .iter()
                .skip(1)
                .any(|g| st.ordner.get(g).is_some_and(|o| o.nur_geraet.wert));
            if darueber {
                return Err(Fehler::neu(
                    "ablage.nur_geraet_geerbt",
                    "„Nur dieses Gerät“ gilt hier durch einen übergeordneten Ordner.",
                ));
            }
        }
        let (z, g) = (st.zaehler(), st.geraet().to_string());
        let o = st.ordner.get_mut(id).ok_or_else(nicht_gefunden)?;
        if let Some(k) = &e.datenklasse {
            if &o.datenklasse.wert != k {
                o.datenklasse = Register::neu(k.clone(), z, &g);
            }
        }
        if let Some(n) = e.nur_geraet {
            if o.nur_geraet.wert != n {
                o.nur_geraet = Register::neu(n, z, &g);
            }
        }
        if let Some(k) = e.zielordner_ki {
            if o.zielordner_ki.wert != k {
                o.zielordner_ki = Register::neu(k, z, &g);
            }
        }
        self.sichern(&mut st)
    }

    /// KI-Freigabe eines Ordners (gerätelokal, `freigabe.json`); `None` = erbt wieder.
    pub fn ki_setzen(&self, id: &str, stufe: Option<KiStufe>) -> Res<()> {
        ordner_id_pruefen(id)?;
        let _g = sperren();
        let st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, id)?;
        freigabe::setzen(&self.dir, id, stufe).map(|_| ())
    }

    /// „Freigaben neu festlegen“: nur bei unklarer, defekter Freigabe; alle Ordner auf `aus`.
    pub fn freigaben_neu_festlegen(&self) -> Res<()> {
        let _g = sperren();
        let st = self.laden()?;
        let alle: Vec<String> = st.ordner.keys().cloned().collect();
        freigabe::neu_festlegen(&self.dir, &alle).map(|_| ())
    }

    // --- Dateien -----------------------------------------------------------------

    pub fn datei_umbenennen(&self, id: &str, name: &str) -> Res<()> {
        datei_id_pruefen(id)?;
        let name = bereinigen(name)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let d = st.dateien.get(id).ok_or_else(nicht_gefunden)?;
        if st
            .namen_im_ordner(&d.ordner.wert, Some(id))
            .contains(&vergleichsschluessel(&name))
        {
            return Err(Fehler::neu(
                "ablage.name_vorhanden",
                "Der Name ist in diesem Ordner schon vergeben.",
            ));
        }
        let (z, g) = (st.zaehler(), st.geraet().to_string());
        let d = st.dateien.get_mut(id).ok_or_else(nicht_gefunden)?;
        d.name = Register::neu(name, z, &g);
        self.sichern(&mut st)
    }

    pub fn datei_verschieben(&self, id: &str, ordner: &str) -> Res<()> {
        datei_id_pruefen(id)?;
        ordner_id_pruefen(ordner)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, ordner)?;
        let d = st.dateien.get(id).ok_or_else(nicht_gefunden)?;
        if st
            .namen_im_ordner(ordner, Some(id))
            .contains(&vergleichsschluessel(&d.name.wert))
        {
            return Err(Fehler::neu(
                "ablage.name_vorhanden",
                "Der Name ist in diesem Ordner schon vergeben.",
            ));
        }
        let (z, g) = (st.zaehler(), st.geraet().to_string());
        let d = st.dateien.get_mut(id).ok_or_else(nicht_gefunden)?;
        d.ordner = Register::neu(ordner.to_string(), z, &g);
        self.sichern(&mut st)
    }

    /// Pfad des Originals der aktuellen Fassung (ohne Folgen von Verknüpfungen geprüft). Bei Konflikt
    /// die jüngste Fassung.
    pub fn original_pfad(&self, dat: &str) -> Res<PathBuf> {
        datei_id_pruefen(dat)?;
        let _g = sperren();
        let st = self.laden()?;
        let d = st.dateien.get(dat).ok_or_else(nicht_gefunden)?;
        let f = aktuelle_fassung(d).ok_or_else(nicht_gefunden)?;
        ohne_umweg(
            &self.dir,
            &format!("{DATEIEN}/{dat}/{}", original_name(&f.fas, &f.typ)),
        )?;
        Ok(self
            .dir
            .join(DATEIEN)
            .join(dat)
            .join(original_name(&f.fas, &f.typ)))
    }

    // --- Import ------------------------------------------------------------------

    /// Legt `quelle` (Datei oder Ordner) in `ordner` ab. Ordner behalten ihre Struktur. Bei Namenskollision
    /// ohne `wahl` bleibt die Zeile `kollision` und nichts wird geschrieben. Nichts wird überschrieben.
    pub fn importieren(
        &self,
        quelle: &Path,
        ordner: &str,
        wahl: Option<AblageKollisionswahl>,
    ) -> Res<Vec<AblageImportZeile>> {
        ordner_id_pruefen(ordner)?;
        let meta = match fs::symlink_metadata(quelle) {
            Ok(m) => m,
            Err(_) => {
                return Ok(vec![AblageImportZeile::fehler(
                    quelle,
                    &Fehler::neu("ablage.quelle_nicht_lesbar", "Quelle nicht lesbar."),
                )])
            }
        };
        if ist_umweg(&meta) {
            return Ok(vec![AblageImportZeile::uebersprungen(
                quelle,
                "ablage.verknuepfung",
            )]);
        }
        if meta.is_file() {
            return Ok(vec![self.datei_ablegen_zeile(quelle, ordner, wahl)]);
        }
        if !meta.is_dir() {
            return Ok(vec![AblageImportZeile::uebersprungen(
                quelle,
                "ablage.keine_datei",
            )]);
        }
        let mut zeilen = Vec::new();
        let mut zaehler = 0usize;
        self.ordner_importieren(quelle, ordner, wahl, 0, &mut zaehler, &mut zeilen)?;
        Ok(zeilen)
    }

    fn ordner_importieren(
        &self,
        quelle: &Path,
        ziel: &str,
        wahl: Option<AblageKollisionswahl>,
        tiefe: usize,
        zaehler: &mut usize,
        zeilen: &mut Vec<AblageImportZeile>,
    ) -> Res<()> {
        let name = quelle
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| Fehler::neu("ablage.name_ungueltig", "Ungültiger Name."))?;
        if tiefe >= MAX_TIEFE {
            zeilen.push(AblageImportZeile::uebersprungen(quelle, "ablage.zu_tief"));
            return Ok(());
        }
        let unter = match self.unterordner_finden_oder_anlegen(ziel, &name) {
            Ok(id) => id,
            Err(e) => {
                zeilen.push(AblageImportZeile::fehler(quelle, &e));
                return Ok(());
            }
        };
        let Ok(inhalt) = fs::read_dir(quelle) else {
            zeilen.push(AblageImportZeile::fehler(
                quelle,
                &Fehler::neu("ablage.quelle_nicht_lesbar", "Quelle nicht lesbar."),
            ));
            return Ok(());
        };
        let mut eintraege: Vec<PathBuf> = inhalt.flatten().map(|e| e.path()).collect();
        eintraege.sort();
        for p in eintraege {
            if *zaehler >= MAX_DATEIEN_JE_LAUF {
                zeilen.push(AblageImportZeile::uebersprungen(&p, "ablage.zu_viele"));
                return Ok(());
            }
            let Ok(m) = fs::symlink_metadata(&p) else {
                zeilen.push(AblageImportZeile::uebersprungen(
                    &p,
                    "ablage.quelle_nicht_lesbar",
                ));
                continue;
            };
            if ist_umweg(&m) {
                zeilen.push(AblageImportZeile::uebersprungen(&p, "ablage.verknuepfung"));
            } else if m.is_dir() {
                self.ordner_importieren(&p, &unter, wahl, tiefe + 1, zaehler, zeilen)?;
            } else if m.is_file() {
                *zaehler += 1;
                zeilen.push(self.datei_ablegen_zeile(&p, &unter, wahl));
            } else {
                zeilen.push(AblageImportZeile::uebersprungen(&p, "ablage.keine_datei"));
            }
        }
        Ok(())
    }

    fn unterordner_finden_oder_anlegen(&self, eltern_id: &str, name: &str) -> Res<String> {
        let name = bereinigen(name)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, eltern_id)?;
        let schluessel = vergleichsschluessel(&name);
        if let Some(o) = st.kinder(&eltern, Some(eltern_id)).into_iter().find(|o| {
            o.papierkorb.wert.is_none() && vergleichsschluessel(&o.name.wert) == schluessel
        }) {
            return Ok(o.id.clone());
        }
        let z = st.zaehler();
        let o = neuer_ordner(st.geraet(), Some(eltern_id), &name, z);
        let id = o.id.clone();
        st.ordner.insert(id.clone(), o);
        self.sichern(&mut st)?;
        Ok(id)
    }

    fn datei_ablegen_zeile(
        &self,
        quelle: &Path,
        ordner: &str,
        wahl: Option<AblageKollisionswahl>,
    ) -> AblageImportZeile {
        match self.datei_ablegen(quelle, ordner, wahl) {
            Ok(z) => z,
            Err(e) => AblageImportZeile::fehler(quelle, &e),
        }
    }

    /// Schritte 1 bis 4 der sicheren Ablage für genau eine Datei.
    fn datei_ablegen(
        &self,
        quelle: &Path,
        ordner: &str,
        wahl: Option<AblageKollisionswahl>,
    ) -> Res<AblageImportZeile> {
        let meta = fs::symlink_metadata(quelle)
            .map_err(|_| Fehler::neu("ablage.quelle_nicht_lesbar", "Quelle nicht lesbar."))?;
        if ist_umweg(&meta) || !meta.is_file() {
            return Err(Fehler::neu(
                "ablage.quelle_ungueltig",
                "Die Quelle ist keine gewöhnliche Datei.",
            ));
        }
        let wunsch = quelle
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| {
                Fehler::neu("ablage.name_ungueltig", "Die Quelle hat keinen Dateinamen.")
            })?;
        let name = bereinigen(&wunsch)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, ordner)?;
        // Namenskollision im Baum (ohne Groß-/Kleinschreibung, nach NFC).
        let schluessel = vergleichsschluessel(&name);
        let vorhanden = st
            .dateien
            .values()
            .find(|d| {
                d.ordner.wert == ordner
                    && d.papierkorb.wert.is_none()
                    && vergleichsschluessel(&d.name.wert) == schluessel
            })
            .map(|d| d.id.clone());
        let (dat, endname, vorgaenger, neue_datei) = match (&vorhanden, wahl) {
            (Some(id), None) => {
                return Ok(AblageImportZeile {
                    quelle: quelle.to_string_lossy().into_owned(),
                    stand: AblageImportStand::Kollision,
                    ordner: Some(ordner.to_string()),
                    dat: Some(id.clone()),
                    fas: None,
                    name: Some(name),
                    code: None,
                })
            }
            (Some(id), Some(AblageKollisionswahl::NeueFassung)) => {
                let d = st.dateien.get(id).ok_or_else(nicht_gefunden)?;
                let kopf = zf::aktuell(d).ok_or_else(|| {
                    Fehler::neu(
                        "ablage.konflikt",
                        "Die Datei ist im Konflikt; zuerst eine Fassung wählen.",
                    )
                })?;
                (id.clone(), d.name.wert.clone(), Some(kopf), false)
            }
            (Some(_), Some(AblageKollisionswahl::BeideBehalten)) => {
                let frei = freier_name(&st.namen_im_ordner(ordner, None), &name);
                (neue_id(PRAEFIX_DATEI), frei, None, true)
            }
            (None, _) => (neue_id(PRAEFIX_DATEI), name.clone(), None, true),
        };
        let fas = neue_id(PRAEFIX_FASSUNG);
        let typ = typ_aus(&endname);
        ohne_umweg(&self.dir, DATEIEN)?;
        ohne_umweg(&self.dir, &format!("{DATEIEN}/{dat}"))?;
        let dir_dat = self.dir.join(DATEIEN).join(&dat);
        let angelegt = !dir_dat.exists();
        fs::create_dir_all(&dir_dat)
            .map_err(|_| Fehler::speicher("Ablageordner nicht anlegbar."))?;
        ohne_umweg(&self.dir, &format!("{DATEIEN}/{dat}"))?;
        let tmp = dir_dat.join(format!("{TEIL}{fas}"));
        let ziel = dir_dat.join(original_name(&fas, &typ));
        let ergebnis = self.kopieren_und_ablegen(quelle, &tmp, &ziel);
        let (sha, groesse) = match ergebnis {
            Ok(x) => x,
            Err(e) => {
                #[cfg(test)]
                if e.code == TEST_ABBRUCH {
                    return Err(e);
                }
                let _ = fs::remove_file(&tmp);
                if angelegt {
                    let _ = fs::remove_dir(&dir_dat);
                }
                return Err(e);
            }
        };
        let zeit = jetzt_ms();
        let eintrag = Eintrag::Abgelegt {
            dat: dat.clone(),
            fas: fas.clone(),
            ordner: ordner.to_string(),
            name: endname.clone(),
            neue_datei,
            vorgaenger,
            sha256: sha,
            groesse,
            typ,
            zeit,
        };
        st.journal.push(eintrag.clone());
        // Scheitert das Vermerken, liegt das Original ohne Eintrag da: der Wiederanlauf findet es.
        self.sichern(&mut st)?;
        haken_journal()?;
        anwenden_abgelegt(&mut st, &eintrag);
        st.journal.retain(|e| e != &eintrag);
        self.sichern(&mut st)?;
        Ok(AblageImportZeile {
            quelle: quelle.to_string_lossy().into_owned(),
            stand: if neue_datei {
                AblageImportStand::Abgelegt
            } else {
                AblageImportStand::NeueFassung
            },
            ordner: Some(ordner.to_string()),
            dat: Some(dat),
            fas: Some(fas),
            name: Some(endname),
            code: None,
        })
    }

    /// Zwischendatei schreiben, prüfen, ohne Ersetzen umbenennen. Liefert SHA-256 und Größe.
    fn kopieren_und_ablegen(&self, quelle: &Path, tmp: &Path, ziel: &Path) -> Res<(String, u64)> {
        let (sha_quelle, meta) = sha256_stabil(quelle)
            .map_err(|_| Fehler::neu("ablage.quelle_nicht_lesbar", "Quelle nicht lesbar."))?;
        let mut nach = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(tmp)
            .map_err(|_| Fehler::speicher("Zwischendatei nicht anlegbar."))?;
        let mut von = fs::File::open(quelle)
            .map_err(|_| Fehler::neu("ablage.quelle_nicht_lesbar", "Quelle nicht lesbar."))?;
        let n = std::io::copy(&mut von, &mut nach)
            .map_err(|_| Fehler::speicher("Datei nicht kopiert."))?;
        nach.sync_all()
            .map_err(|_| Fehler::speicher("Datei nicht kopiert."))?;
        drop(nach);
        haken_nach_zwischendatei()?;
        let sha_kopie = sha256_datei(tmp).map_err(|_| Fehler::speicher("Kopie nicht lesbar."))?;
        if n != meta.len() || sha_kopie != sha_quelle {
            return Err(Fehler::neu(
                "ablage.pruefsumme",
                "Die Kopie stimmt nicht mit der Quelle überein; nichts aufgenommen.",
            ));
        }
        haken_nach_hash()?;
        verschieben_ohne_ersetzen(tmp, ziel)
            .map_err(|_| Fehler::speicher("Datei nicht ablegbar."))?;
        haken_nach_umbenennen()?;
        Ok((sha_kopie, n))
    }

    // --- Papierkorb --------------------------------------------------------------

    pub fn in_papierkorb(&self, art: AblageArt, id: &str) -> Res<()> {
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        let (z, g, jetzt) = (st.zaehler(), st.geraet().to_string(), jetzt_ms());
        match art {
            AblageArt::Datei => {
                datei_id_pruefen(id)?;
                let d = st.dateien.get_mut(id).ok_or_else(nicht_gefunden)?;
                if d.papierkorb.wert.is_some() {
                    return Err(Fehler::neu(
                        "ablage.schon_im_papierkorb",
                        "Schon im Papierkorb.",
                    ));
                }
                d.papierkorb = Register::neu(Some(jetzt), z, &g);
            }
            AblageArt::Ordner => {
                ordner_id_pruefen(id)?;
                self.ordner_lebend(&st, &eltern, id)?;
                let o = st.ordner.get_mut(id).ok_or_else(nicht_gefunden)?;
                o.papierkorb = Register::neu(Some(jetzt), z, &g);
            }
        }
        self.sichern(&mut st)
    }

    pub fn wiederherstellen(&self, art: AblageArt, id: &str) -> Res<()> {
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        let (z, g) = (st.zaehler(), st.geraet().to_string());
        match art {
            AblageArt::Datei => {
                datei_id_pruefen(id)?;
                let d = st.dateien.get(id).ok_or_else(nicht_gefunden)?;
                if d.papierkorb.wert.is_none() {
                    return Err(nicht_im_korb());
                }
                self.ordner_lebend(&st, &eltern, &d.ordner.wert)?;
                let frei = freier_name(&st.namen_im_ordner(&d.ordner.wert, Some(id)), &d.name.wert);
                let d = st.dateien.get_mut(id).ok_or_else(nicht_gefunden)?;
                if frei != d.name.wert {
                    d.name = Register::neu(frei, z, &g);
                }
                d.papierkorb = Register::neu(None, z, &g);
            }
            AblageArt::Ordner => {
                ordner_id_pruefen(id)?;
                let o = st.ordner.get(id).ok_or_else(nicht_gefunden)?;
                if o.papierkorb.wert.is_none() {
                    return Err(nicht_im_korb());
                }
                let e = match eltern.get(id) {
                    Some(Eltern::Ordner(p)) => {
                        self.ordner_lebend(&st, &eltern, p)?;
                        Some(p.clone())
                    }
                    _ => None,
                };
                let belegt: BTreeSet<String> = st
                    .kinder(&eltern, e.as_deref())
                    .into_iter()
                    .filter(|k| k.papierkorb.wert.is_none() && k.id != id)
                    .map(|k| vergleichsschluessel(&k.name.wert))
                    .collect();
                let frei = freier_name(&belegt, &o.name.wert);
                let o = st.ordner.get_mut(id).ok_or_else(nicht_gefunden)?;
                if frei != o.name.wert {
                    o.name = Register::neu(frei, z, &g);
                }
                o.papierkorb = Register::neu(None, z, &g);
            }
        }
        self.sichern(&mut st)
    }

    /// Entfernt einen Eintrag des Papierkorbs endgültig: Original, Textfassungen, Zwischenspeicher.
    pub fn endgueltig_loeschen(&self, art: AblageArt, id: &str) -> Res<AblageLoeschBericht> {
        let _g = sperren();
        let mut st = self.laden()?;
        let b = self.endgueltig_intern(&mut st, art, id)?;
        self.sichern(&mut st)?;
        Ok(b)
    }

    /// Leert den Papierkorb (alle Einträge der obersten Ebene).
    pub fn papierkorb_leeren(&self) -> Res<AblageLoeschBericht> {
        let _g = sperren();
        let mut st = self.laden()?;
        self.teildateien_loeschen(None);
        let eltern = st.eltern();
        let dat: Vec<String> = st
            .dateien
            .values()
            .filter(|d| d.papierkorb.wert.is_some() && !st.ordner_im_korb(&eltern, &d.ordner.wert))
            .map(|d| d.id.clone())
            .collect();
        let ord: Vec<String> = st
            .ordner
            .values()
            .filter(|o| {
                o.papierkorb.wert.is_some()
                    && match eltern.get(&o.id) {
                        Some(Eltern::Ordner(p)) => !st.ordner_im_korb(&eltern, p),
                        _ => true,
                    }
            })
            .map(|o| o.id.clone())
            .collect();
        let mut summe = AblageLoeschBericht::default();
        for d in dat {
            if st.dateien.contains_key(&d) {
                let b = self.endgueltig_intern(&mut st, AblageArt::Datei, &d)?;
                summe.dateien += b.dateien;
                summe.ordner += b.ordner;
            }
        }
        for o in ord {
            if st.ordner.contains_key(&o) {
                let b = self.endgueltig_intern(&mut st, AblageArt::Ordner, &o)?;
                summe.dateien += b.dateien;
                summe.ordner += b.ordner;
            }
        }
        self.sichern(&mut st)?;
        Ok(summe)
    }

    fn endgueltig_intern(
        &self,
        st: &mut Stand,
        art: AblageArt,
        id: &str,
    ) -> Res<AblageLoeschBericht> {
        let eltern = st.eltern();
        let (dat, ord): (Vec<String>, Vec<String>) = match art {
            AblageArt::Datei => {
                datei_id_pruefen(id)?;
                let d = st.dateien.get(id).ok_or_else(nicht_gefunden)?;
                if d.papierkorb.wert.is_none() && !st.ordner_im_korb(&eltern, &d.ordner.wert) {
                    return Err(nicht_im_korb());
                }
                (vec![id.to_string()], vec![])
            }
            AblageArt::Ordner => {
                ordner_id_pruefen(id)?;
                let o = st.ordner.get(id).ok_or_else(nicht_gefunden)?;
                if o.papierkorb.wert.is_none() {
                    return Err(nicht_im_korb());
                }
                let unterbaum = unterbaum_von(st, &eltern, id);
                let dat = st
                    .dateien
                    .values()
                    .filter(|d| unterbaum.contains(&d.ordner.wert))
                    .map(|d| d.id.clone())
                    .collect();
                (dat, unterbaum.into_iter().collect())
            }
        };
        // Journal zuerst: ein Abbruch wird beim Wiederanlauf zu Ende geführt.
        st.journal.push(Eintrag::Loeschen {
            dat: dat.clone(),
            ordner: ord.clone(),
        });
        self.sichern(st)?;
        haken_loeschen()?;
        self.loeschen_ausfuehren(st, &dat, &ord)?;
        st.journal.retain(
            |e| !matches!(e, Eintrag::Loeschen { dat: d, ordner: o } if *d == dat && *o == ord),
        );
        Ok(AblageLoeschBericht {
            dateien: dat.len() as u32,
            ordner: ord.len() as u32,
        })
    }

    /// Entfernt Originale und Texte (über einen Zwischenort im Papierkorb), dann die Einträge im Baum.
    /// Wiederholbar: was schon fehlt, wird übersprungen.
    fn loeschen_ausfuehren(&self, st: &mut Stand, dat: &[String], ord: &[String]) -> Res<()> {
        for d in dat {
            if !id_gueltig(PRAEFIX_DATEI, d) {
                continue;
            }
            for wurzel in [DATEIEN, TEXT] {
                let quelle = self.dir.join(wurzel).join(d);
                let Ok(m) = fs::symlink_metadata(&quelle) else {
                    continue;
                };
                if ist_umweg(&m) {
                    // Eine Verknüpfung wird nie gefolgt: nur der Verweis selbst verschwindet.
                    let _ = fs::remove_file(&quelle).or_else(|_| fs::remove_dir(&quelle));
                    continue;
                }
                fs::create_dir_all(self.dir.join(PAPIERKORB))
                    .map_err(|_| Fehler::speicher("Papierkorb nicht anlegbar."))?;
                let zwischen = self
                    .dir
                    .join(PAPIERKORB)
                    .join(format!("{d}{LOESCHEN}{}", neue_id("")));
                fs::rename(&quelle, &zwischen)
                    .map_err(|_| Fehler::speicher("Datei nicht entfernbar; nichts verloren."))?;
                fs::remove_dir_all(&zwischen)
                    .map_err(|_| Fehler::speicher("Datei nicht vollständig entfernt."))?;
            }
        }
        // Reste früherer, abgebrochener Läufe.
        if let Ok(rest) = fs::read_dir(self.dir.join(PAPIERKORB)) {
            for e in rest.flatten() {
                if e.file_name().to_string_lossy().contains(LOESCHEN) {
                    let _ = fs::remove_dir_all(e.path());
                }
            }
        }
        for d in dat {
            if st.dateien.remove(d).is_some() || !st.begraben.contains(d) {
                st.begraben.push(d.clone());
            }
        }
        for o in ord {
            st.ordner.remove(o);
            if !st.begraben.contains(o) {
                st.begraben.push(o.clone());
            }
        }
        st.gefunden.retain(|g| !dat.contains(&g.dat));
        st.begraben.sort();
        st.begraben.dedup();
        let _ = freigabe::vergessen(&self.dir, ord);
        Ok(())
    }

    // --- Bestand neu aufbauen, Gefundenes ------------------------------------------

    /// „Bestand neu aufbauen“: nur bei fehlendem oder defektem Journal. Ein defektes wird als
    /// `ablage.json.defekt-<ms>` beiseitegelegt, nie gelöscht. Jede Datei in `dateien/<dat>/` wird als
    /// `gefunden` vermerkt und angeboten, nicht still aufgenommen. Liefert die Zahl der Funde.
    pub fn neu_aufbauen(&self) -> Res<u32> {
        let _g = sperren();
        match self.lage() {
            Ok(_) => {
                return Err(Fehler::neu(
                    "ablage.journal_lesbar",
                    "Das Ablagejournal ist lesbar; nichts neu aufzubauen.",
                ))
            }
            Err(AblageLage::Neuer) | Err(AblageLage::NichtLesbar) => {
                return Err(Fehler::neu(
                    "ablage.nicht_neu_aufbaubar",
                    "Das Ablagejournal wird so nicht neu aufgebaut; nichts angefasst.",
                ))
            }
            Err(AblageLage::Defekt) => {
                let ziel = self.dir.join(format!("{JOURNAL}.defekt-{}", jetzt_ms()));
                verschieben_ohne_ersetzen(&self.journal_pfad(), &ziel)
                    .map_err(|_| Fehler::speicher("Ablagejournal nicht beiseitelegbar."))?;
            }
            Err(AblageLage::Fehlt) => {}
        }
        let mut st = Stand::neu();
        let _ = fs::create_dir_all(self.dir.join(DATEIEN));
        self.zwischendateien_raeumen();
        self.gefundenes_suchen(&mut st);
        if let Ok(mut g) = gescannt().lock() {
            g.insert(self.dir.clone());
        }
        let n = st.gefunden.len() as u32;
        self.sichern(&mut st)?;
        Ok(n)
    }

    /// Nimmt gefundene Originale einer `dat` in `ordner` auf (nach Bestätigung durch die Bedienung). Der
    /// Name geht beim Verlust des Journals verloren; die Datei heißt „Gefundene Datei <Kürzel>“. Gehört
    /// die `dat` schon zu einer Datei im Baum, werden die Funde als neue Fassungen angehängt.
    pub fn gefundene_aufnehmen(&self, dat: &str, ordner: &str) -> Res<()> {
        datei_id_pruefen(dat)?;
        ordner_id_pruefen(ordner)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, ordner)?;
        let mut funde: Vec<Gefunden> = st
            .gefunden
            .iter()
            .filter(|g| g.dat == dat)
            .cloned()
            .collect();
        if funde.is_empty() {
            return Err(nicht_gefunden());
        }
        funde.sort_by(|a, b| a.fas.cmp(&b.fas));
        ohne_umweg(&self.dir, &format!("{DATEIEN}/{dat}"))?;
        let mut neue: Vec<Fassung> = Vec::new();
        for g in &funde {
            let pfad = self
                .dir
                .join(DATEIEN)
                .join(dat)
                .join(original_name(&g.fas, &g.typ));
            let m = fs::symlink_metadata(&pfad)
                .map_err(|_| Fehler::neu("ablage.nicht_gefunden", "Datei nicht gefunden."))?;
            if ist_umweg(&m) || !m.is_file() {
                return Err(Fehler::neu(
                    "ablage.quelle_ungueltig",
                    "Keine gewöhnliche Datei.",
                ));
            }
            let sha = sha256_datei(&pfad).map_err(|_| Fehler::speicher("Datei nicht lesbar."))?;
            neue.push(Fassung {
                fas: g.fas.clone(),
                vorgaenger: None,
                blob: sha.clone(),
                size: m.len(),
                original_sha256: sha,
                typ: g.typ.clone(),
                geraet: st.geraet().to_string(),
                zeit: g.ms,
                texte: vec![],
            });
        }
        let (z, ger) = (st.zaehler(), st.geraet().to_string());
        match st.dateien.get_mut(dat) {
            Some(d) => {
                // Im Konflikt (kein einzelner aktueller Kopf): der jüngste Kopf nach (zeit, id).
                let zeit_von = |d: &Datei, id: &str| {
                    d.fassungen
                        .iter()
                        .find(|f| f.fas == id)
                        .map_or(0, |f| f.zeit)
                };
                let mut vor = zf::aktuell(d).or_else(|| {
                    zf::koepfe(d)
                        .into_iter()
                        .max_by(|a, b| zeit_von(d, a).cmp(&zeit_von(d, b)).then_with(|| a.cmp(b)))
                });
                for mut f in neue {
                    f.vorgaenger = vor.clone();
                    vor = Some(f.fas.clone());
                    d.fassungen.push(f);
                }
                d.blob_refs = zf::blob_refs_aus(&d.fassungen);
            }
            None => {
                let kurz: String = dat
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let typ = funde[0].typ.clone();
                let endung = if typ.is_empty() {
                    String::new()
                } else {
                    format!(".{typ}")
                };
                let wunsch = format!("Gefundene Datei {kurz}{endung}");
                let name = freier_name(&st.namen_im_ordner(ordner, None), &wunsch);
                let mut vor: Option<String> = None;
                let mut fassungen = Vec::new();
                for mut f in neue {
                    f.vorgaenger = vor.clone();
                    vor = Some(f.fas.clone());
                    fassungen.push(f);
                }
                st.dateien.insert(
                    dat.to_string(),
                    Datei {
                        id: dat.to_string(),
                        ordner: Register::neu(ordner.to_string(), z, &ger),
                        name: Register::neu(name, z, &ger),
                        klasse_min: Register::neu("public".into(), z, &ger),
                        nur_geraet_min: false,
                        blob_refs: zf::blob_refs_aus(&fassungen),
                        fassungen,
                        wahlen: vec![],
                        entfernt: vec![],
                        papierkorb: Register::neu(None, z, &ger),
                    },
                );
            }
        }
        st.gefunden.retain(|g| g.dat != dat);
        self.sichern(&mut st)
    }

    /// Entfernt die Fassungen, die die Aufbewahrungsregel nicht mehr hält (Original wird gelöscht, `fas`
    /// kommt nach `entfernt`). Liefert die Zahl der entfernten Fassungen.
    pub fn fassungen_aufraeumen(&self, dat: &str, jetzt: i64) -> Res<u32> {
        datei_id_pruefen(dat)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let d = st.dateien.get(dat).ok_or_else(nicht_gefunden)?;
        let weg = super::aufbewahrung::zu_entfernen(d, jetzt);
        if weg.is_empty() {
            return Ok(0);
        }
        let typen: BTreeMap<String, String> = d
            .fassungen
            .iter()
            .map(|f| (f.fas.clone(), f.typ.clone()))
            .collect();
        let d = st.dateien.get_mut(dat).ok_or_else(nicht_gefunden)?;
        d.fassungen.retain(|f| !weg.contains(&f.fas));
        d.entfernt.extend(weg.iter().cloned());
        d.entfernt.sort();
        d.entfernt.dedup();
        d.blob_refs = zf::blob_refs_aus(&d.fassungen);
        self.sichern(&mut st)?;
        for f in &weg {
            let typ = typen.get(f).cloned().unwrap_or_default();
            let p = self
                .dir
                .join(DATEIEN)
                .join(dat)
                .join(original_name(f, &typ));
            if ohne_umweg(&self.dir, &format!("{DATEIEN}/{dat}")).is_ok() {
                let _ = fs::remove_file(p);
            }
        }
        Ok(weg.len() as u32)
    }
}

// ---------------------------------------------------------------------------
// Hilfen ohne Zustand
// ---------------------------------------------------------------------------

fn nicht_gefunden() -> Fehler {
    Fehler::neu("ablage.nicht_gefunden", "Nicht gefunden.")
}

fn nicht_im_korb() -> Fehler {
    Fehler::neu(
        "ablage.nicht_im_papierkorb",
        "Der Eintrag liegt nicht im Papierkorb.",
    )
}

fn tage_rest(geloescht: i64, jetzt: i64) -> i64 {
    let rest = PAPIERKORB_TAGE * TAG_MS - (jetzt - geloescht);
    (rest.max(0) + TAG_MS - 1) / TAG_MS
}

fn neuer_ordner(geraet: &str, eltern: Option<&str>, name: &str, z: u64) -> Ordner {
    Ordner {
        id: neue_id(PRAEFIX_ORDNER),
        eltern: Register::neu(eltern.map(str::to_string), z, geraet),
        name: Register::neu(name.to_string(), z, geraet),
        datenklasse: Register::neu("erbt".into(), z, geraet),
        nur_geraet: Register::neu(false, z, geraet),
        zielordner_ki: Register::neu(false, z, geraet),
        papierkorb: Register::neu(None, z, geraet),
    }
}

fn gleichnamig_pruefen(
    st: &Stand,
    eltern: &BTreeMap<String, Eltern>,
    eltern_id: Option<&str>,
    name: &str,
    ausser: Option<&str>,
) -> Res<()> {
    let k = vergleichsschluessel(name);
    let doppelt = st.kinder(eltern, eltern_id).into_iter().any(|o| {
        o.papierkorb.wert.is_none()
            && Some(o.id.as_str()) != ausser
            && vergleichsschluessel(&o.name.wert) == k
    });
    if doppelt {
        return Err(Fehler::neu(
            "ablage.name_vorhanden",
            "Der Name ist an dieser Stelle schon vergeben.",
        ));
    }
    Ok(())
}

/// Der Ordner und alle Ordner darunter nach der WIRKSAMEN Zuordnung (Ordner unter „Wiederhergestellt“
/// sind keine Kinder ihres gespeicherten Elternordners).
fn unterbaum_von(st: &Stand, eltern: &BTreeMap<String, Eltern>, id: &str) -> BTreeSet<String> {
    let mut menge: BTreeSet<String> = BTreeSet::new();
    menge.insert(id.to_string());
    loop {
        let vorher = menge.len();
        for o in st.ordner.values() {
            let wirksam = matches!(eltern.get(&o.id), Some(Eltern::Ordner(p)) if menge.contains(p));
            if wirksam {
                menge.insert(o.id.clone());
            }
        }
        if menge.len() == vorher {
            return menge;
        }
    }
}

/// Die Fassung, die als aktuell gilt (einziger Kopf), sonst die jüngste.
fn aktuelle_fassung(d: &Datei) -> Option<&Fassung> {
    let id = zf::aktuell(d);
    match id {
        Some(i) => d.fassungen.iter().find(|f| f.fas == i),
        None => d
            .fassungen
            .iter()
            .max_by(|a, b| a.zeit.cmp(&b.zeit).then_with(|| a.fas.cmp(&b.fas))),
    }
}

fn datei_sicht(
    st: &Stand,
    eltern: &BTreeMap<String, Eltern>,
    lage: &freigabe::Lage,
    d: &Datei,
) -> AblageDatei {
    let f = aktuelle_fassung(d);
    let kette = zf::kette(eltern, &d.ordner.wert);
    AblageDatei {
        id: d.id.clone(),
        name: d.name.wert.clone(),
        ordner: d.ordner.wert.clone(),
        typ: f.map(|f| f.typ.clone()).unwrap_or_default(),
        groesse: f.map(|f| f.size).unwrap_or(0),
        geaendert: f.map(|f| f.zeit).unwrap_or(0),
        fassungen: d.fassungen.len() as u32,
        konflikt: zf::konflikt(d),
        klasse: zf::wirksame_klasse_datei(&st.ordner, eltern, d)
            .text()
            .into(),
        ki: ki_sicht(
            st,
            lage,
            &kette,
            &zf::zusatzkette(&st.ordner, eltern, &d.ordner.wert),
        ),
        nur_geraet: d.nur_geraet_min || zf::wirksam_nur_geraet(&st.ordner, eltern, &d.ordner.wert),
        text: f.and_then(|f| f.texte.last().map(|t| t.stand.clone())),
    }
}

/// Trägt eine abgelegte Fassung in den Baum ein (idempotent).
fn anwenden_abgelegt(st: &mut Stand, e: &Eintrag) {
    let Eintrag::Abgelegt {
        dat,
        fas,
        ordner,
        name,
        vorgaenger,
        sha256,
        groesse,
        typ,
        zeit,
        ..
    } = e
    else {
        return;
    };
    let z = st.zaehler();
    let g = st.geraet().to_string();
    let fassung = Fassung {
        fas: fas.clone(),
        vorgaenger: vorgaenger.clone(),
        blob: sha256.clone(),
        size: *groesse,
        original_sha256: sha256.clone(),
        typ: typ.clone(),
        geraet: g.clone(),
        zeit: *zeit,
        texte: vec![],
    };
    match st.dateien.get_mut(dat) {
        Some(d) => {
            if !d.fassungen.iter().any(|f| &f.fas == fas) {
                d.fassungen.push(fassung);
                d.blob_refs = zf::blob_refs_aus(&d.fassungen);
            }
        }
        None => {
            // Existiert der Zielordner nicht mehr, bleibt die Datei als Fund erhalten.
            if !st.ordner.contains_key(ordner) {
                st.gefunden.push(Gefunden {
                    dat: dat.clone(),
                    fas: fas.clone(),
                    typ: typ.clone(),
                    groesse: *groesse,
                    ms: *zeit,
                });
                return;
            }
            st.dateien.insert(
                dat.clone(),
                Datei {
                    id: dat.clone(),
                    ordner: Register::neu(ordner.clone(), z, &g),
                    name: Register::neu(name.clone(), z, &g),
                    klasse_min: Register::neu("public".into(), z, &g),
                    nur_geraet_min: false,
                    blob_refs: zf::blob_refs_aus(std::slice::from_ref(&fassung)),
                    fassungen: vec![fassung],
                    wahlen: vec![],
                    entfernt: vec![],
                    papierkorb: Register::neu(None, z, &g),
                },
            );
        }
    }
}

#[cfg(test)]
fn haken_nach_zwischendatei() -> Res<()> {
    haken(Schritt::NachZwischendatei)
}
#[cfg(test)]
fn haken_nach_hash() -> Res<()> {
    haken(Schritt::NachHash)
}
#[cfg(test)]
fn haken_nach_umbenennen() -> Res<()> {
    haken(Schritt::NachUmbenennen)
}
#[cfg(test)]
fn haken_journal() -> Res<()> {
    haken(Schritt::NachJournal)
}
#[cfg(test)]
fn haken_loeschen() -> Res<()> {
    haken(Schritt::LoeschenNachJournal)
}
#[cfg(not(test))]
fn haken_nach_zwischendatei() -> Res<()> {
    Ok(())
}
#[cfg(not(test))]
fn haken_nach_hash() -> Res<()> {
    Ok(())
}
#[cfg(not(test))]
fn haken_nach_umbenennen() -> Res<()> {
    Ok(())
}
#[cfg(not(test))]
fn haken_journal() -> Res<()> {
    Ok(())
}
#[cfg(not(test))]
fn haken_loeschen() -> Res<()> {
    Ok(())
}

/// Klasse einer Datei für Aufrufer außerhalb (z. B. D3): `Unbekannt` heißt kein Abruf.
pub fn klasse_aus_text(t: &str) -> Option<Klasse> {
    Klasse::aus_text(t)
}
