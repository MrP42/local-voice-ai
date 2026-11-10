//! Lokale Ablage: `ablage.json` (Journal + Baum), Originale, Papierkorb (Spec §2, ADR-0029 E3/E4/E9).
//!
//! Aufbau unter `<Datenordner>/ablage/`:
//!
//! ```text
//! ablage.json                   Version, Epoche, Gerät, Ordner, Dateien, Journal, Gefundenes, Begrabenes
//! dateien/<dat>/<fas>.<typ>     Originale, unveränderlich, nach Kennung benannt
//! text/<dat>/<textrev>.json     Textfassungen (D2, Markenformat v1; siehe `text_schreiben`)
//! index.db                      abgeleiteter Suchindex (D2, siehe `index`), löschbar
//! papierkorb/                   Zwischenspeicher beim endgültigen Löschen, liegengebliebene Zwischendateien
//! freigabe.json                 gerätelokal (siehe `freigabe`)
//! uebergang-<ULID>.json         Rückweg lokal → sync, nur während er läuft (siehe `speicher_uebergang`)
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

// `original_pfad`, `fassungen_aufraeumen` und `klasse_aus_text` sind für D3/D4 vorbereitet.
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

#[path = "speicher_sync.rs"]
mod abgleich_zugriff;
pub use abgleich_zugriff::{Eingang, SyncBestand};
#[path = "speicher_uebergang.rs"]
mod uebergang;
pub use uebergang::ProjektVerweise;
#[path = "speicher_ki.rs"]
mod ki_zugriff;
pub use ki_zugriff::{AblageEntwurf, KiBestand, KiDatei};

pub const DATEIEN: &str = "dateien";
pub const TEXT: &str = "text";
pub const PAPIERKORB: &str = "papierkorb";
pub const JOURNAL: &str = "ablage.json";
const TEIL: &str = ".teil-";
const LOESCHEN: &str = ".loeschen-";
/// Vom Hub begrabene Inhalte im Papierkorb (`grab-<ms>-<wurzel>-<dat>`), D4.
const GRAB: &str = "grab-";
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
    /// Beim endgültigen Löschen liefert das Lesen der Metadaten einen Fehler (Rechte, Datenträger).
    LoeschenLesefehler,
    /// Beim endgültigen Löschen scheitert das Entfernen des Zwischenorts im Papierkorb.
    LoeschenEntfernfehler,
    /// Rückweg lokal → sync (D4): nach dem Schreiben des Übergangsjournals (`begonnen`).
    UebergangBegonnen,
    /// … nachdem die Originale umgezogen sind, vor den Textfassungen und dem Stand.
    UebergangDateien,
    /// … nach Stand und Journalschritt `umgezogen`.
    UebergangUmgezogen,
    /// … nach dem Umschreiben der KI-Freigaben, vor Projektverweisen und Index.
    UebergangFreigaben,
    /// … nach dem Journalschritt `umgeschrieben`.
    UebergangUmgeschrieben,
    /// … nach dem Journalschritt `fertig`, bevor das Journal entfernt ist.
    UebergangFertig,
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
        /// D3 KI-Entwurf: Mindestklasse der neuen Datei (Maximum aus Zielordner und Quellen).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        klasse_min: Option<String>,
        /// D3 KI-Entwurf: eine Quelle ist „nur dieses Gerät“.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        nur_geraet_min: bool,
    },
    /// Endgültiges Löschen läuft. `aufheben`: Grabstein vom Hub (D4) – Inhalte gehen für
    /// [`PAPIERKORB_TAGE`] in den Papierkorb (`papierkorb/grab-<ms>-…`), statt sofort entfernt zu werden.
    Loeschen {
        dat: Vec<String>,
        ordner: Vec<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        aufheben: bool,
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
    /// Vorgängerin, wenn das Journal sie vermerkt hatte; sonst unbekannt (nie erfunden).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vorgaenger: Option<String>,
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
    /// D4: Kennungen, die beim Wechsel zu „nur dieses Gerät“ (oder `personal`/`confidential`) als
    /// Momentaufnahme festgehalten wurden (Write-ahead, vor jedem Push). Sie verlassen das Gerät nie
    /// wieder; ein Grabstein vom Hub wird auf sie nie angewendet (auch nicht nach Verlust von
    /// `sync.json` oder Kontowechsel).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    lokal: Vec<String>,
    /// D4: Einträge, die nach dem Umschalten von einem anderen Gerät unter einen festgehaltenen Ordner
    /// kamen: sie bleiben synchron (der Ordner fehlt für sie, wie unter „Wiederhergestellt“) und werden
    /// nie begraben.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    befreit: Vec<String>,
    /// D4: von diesem Gerät hochgeladene Blobs (Hub-Kennung → Zeit, Konto) für die Wartung
    /// „Blobs abgleichen“ (der Hub bietet keine Blob-Liste).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sync_blobs: BTreeMap<String, SyncBlob>,
    /// D4 Rückweg lokal → sync: alte Kennung → neue Kennung (Deep Links, Projektverweise). Stände vom
    /// Hub zu alten Kennungen werden nie wieder aufgenommen.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    weiterleitung: BTreeMap<String, String>,
    /// Teilnahme beim Laden (nur im Speicher): Grundlage der Momentaufnahme in [`Ablage::sichern`].
    #[serde(skip)]
    vorher: Option<Vorher>,
}

/// Ein hochgeladener Blob (D4, Wartung „Blobs abgleichen“).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncBlob {
    pub ms: i64,
    /// Kontokennung des Abgleichs (Benutzer, Schlüsselkennung, Hub).
    pub konto: String,
}

/// Stand beim Laden: welche Einträge am Abgleich teilnahmen und wo befreite Einträge lagen.
#[derive(Debug, Clone, Default)]
struct Vorher {
    sync: BTreeSet<String>,
    befreit_lage: BTreeMap<String, Option<String>>,
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
            lokal: Vec::new(),
            befreit: Vec::new(),
            sync_blobs: BTreeMap::new(),
            weiterleitung: BTreeMap::new(),
            vorher: None,
        }
    }

    fn lokal_menge(&self) -> BTreeSet<String> {
        self.lokal.iter().cloned().collect()
    }

    fn befreit_menge(&self) -> BTreeSet<String> {
        self.befreit.iter().cloned().collect()
    }

    /// Teilnahme am Abgleich je Kennung (D4, [`crate::sync::ablage::teilnahmen`]).
    fn teilnahmen(&self) -> BTreeMap<String, crate::sync::ablage::Teilnahme> {
        crate::sync::ablage::teilnahmen(
            &self.ordner,
            &self.dateien,
            &self.lokal_menge(),
            &self.befreit_menge(),
        )
    }

    /// Lage eines befreiten Eintrags (Ordner der Datei bzw. Eltern des Ordners).
    fn lage_von(&self, id: &str) -> Option<Option<String>> {
        if let Some(d) = self.dateien.get(id) {
            return Some(Some(d.ordner.wert.clone()));
        }
        self.ordner.get(id).map(|o| o.eltern.wert.clone())
    }

    fn vorher_berechnen(&self) -> Vorher {
        use crate::sync::ablage::Teilnahme;
        Vorher {
            sync: self
                .teilnahmen()
                .into_iter()
                .filter(|(_, t)| *t == Teilnahme::Sync)
                .map(|(id, _)| id)
                .collect(),
            befreit_lage: self
                .befreit
                .iter()
                .filter_map(|id| self.lage_von(id).map(|l| (id.clone(), l)))
                .collect(),
        }
    }

    /// Momentaufnahme (D4): was durch eine Handlung auf diesem Gerät von „synchron“ zu „nur dieses
    /// Gerät“ wechselt, wird festgehalten, bevor irgendein Abgleich davon erfährt. Ein befreiter
    /// Eintrag, den der Nutzer hier verschiebt, verliert die Befreiung (der Ordner gilt dann wieder).
    fn festhalten(&mut self, v: &Vorher) {
        use crate::sync::ablage::Teilnahme;
        let bewegt: Vec<String> = v
            .befreit_lage
            .iter()
            .filter(|(id, l)| self.lage_von(id).is_some_and(|neu| neu != **l))
            .map(|(id, _)| id.clone())
            .collect();
        self.befreit.retain(|b| !bewegt.contains(b));
        let jetzt = self.teilnahmen();
        let mut neu = false;
        for (id, t) in jetzt {
            if t == Teilnahme::Lokal && v.sync.contains(&id) && !self.lokal.contains(&id) {
                self.lokal.push(id.clone());
                self.befreit.retain(|b| *b != id);
                neu = true;
            }
        }
        if neu {
            self.lokal.sort();
            self.lokal.dedup();
        }
        let lebend: BTreeSet<&String> = self.ordner.keys().chain(self.dateien.keys()).collect();
        self.befreit.retain(|b| lebend.contains(b));
    }

    /// Nächster Lamport-Zähler: größter bekannter Wert + 1, höchstens `ZAEHLER_MAX`. Ist die Kette
    /// erschöpft, gibt es einen Fehler und es wird nichts geschrieben (nie ein Umlauf auf 0).
    fn zaehler(&self) -> Res<u64> {
        zf::naechster_zaehler(&self.ordner, &self.dateien).ok_or_else(zaehler_erschoepft)
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

    /// Dateien und Ordner, die ein offener Löschauftrag im Journal noch entfernen wird.
    fn offen_geloescht(&self) -> (BTreeSet<&str>, BTreeSet<&str>) {
        let mut dat = BTreeSet::new();
        let mut ord = BTreeSet::new();
        for e in &self.journal {
            if let Eintrag::Loeschen {
                dat: d, ordner: o, ..
            } = e
            {
                dat.extend(d.iter().map(String::as_str));
                ord.extend(o.iter().map(String::as_str));
            }
        }
        (dat, ord)
    }

    /// Weist ab, wenn ein offener Löschauftrag die Datei oder den Ordner noch entfernen wird. Sonst
    /// würde eine Änderung (Verschieben, Wiederherstellen, Ablegen) beim nächsten Wiederanlauf mitgelöscht.
    fn nicht_offen_geloescht(&self, dat: Option<&str>, ordner: Option<&str>) -> Res<()> {
        let (d, o) = self.offen_geloescht();
        if dat.is_some_and(|x| d.contains(x)) || ordner.is_some_and(|x| o.contains(x)) {
            return Err(loeschen_offen());
        }
        Ok(())
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
            Eintrag::Loeschen { dat, ordner, .. } => {
                dat.iter().all(|d| id_gueltig(PRAEFIX_DATEI, d))
                    && ordner.iter().all(|o| id_gueltig(PRAEFIX_ORDNER, o))
            }
        })
        && s.lokal
            .iter()
            .chain(s.befreit.iter())
            .all(|i| id_gueltig(PRAEFIX_DATEI, i) || id_gueltig(PRAEFIX_ORDNER, i))
        && s.weiterleitung
            .iter()
            .all(|(a, n)| uebergang::gleiche_art(a, n))
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

/// Bereitet einen vom Nutzer gewählten Quellpfad für den Import vor.
///
/// Windows: Ein nicht verbatim angegebener Pfad, dessen letztes Glied ein reservierter Gerätename
/// ist (`CON`, `NUL`, `AUX`, `PRN`, `COM1`…, `LPT1`…, `CONIN$`, `CONOUT$`, auch mit Endung oder
/// angehängten Punkten/Leerzeichen), wird als Gerät abgewiesen (`ablage.geraet`), ebenso
/// Gerätenamensräume (`\\.\…`, `\\?\GLOBALROOT…`). Ein `:` im letzten Glied adressiert einen
/// alternativen Datenstrom und wird abgewiesen (`ablage.datenstrom`). Alles andere wird absolut
/// gemacht und verbatim (`\\?\C:\…` bzw. `\\?\UNC\…`) zurückgegeben; Kinder aus `read_dir`
/// bleiben damit ebenfalls verbatim. Ein schon verbatim angegebener Pfad meint immer die echte
/// Datei (auch `CON.txt`) und wird nur auf Datenströme geprüft. Andere Systeme: unverändert.
pub(super) fn quelle_vorbereiten(quelle: &Path) -> Result<PathBuf, &'static str> {
    #[cfg(windows)]
    {
        windows_quelle(quelle)
    }
    #[cfg(not(windows))]
    {
        Ok(quelle.to_path_buf())
    }
}

/// Reservierter Windows-Gerätename (ohne Rücksicht auf Groß/Klein, Endung, Punkte, Leerzeichen).
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn ist_geraetename(name: &str) -> bool {
    let stamm = name.split('.').next().unwrap_or("");
    let stamm = stamm.trim_end_matches([' ', '.']).to_ascii_uppercase();
    matches!(
        stamm.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((stamm.starts_with("COM") || stamm.starts_with("LPT"))
        && stamm.len() == 4
        && matches!(stamm.as_bytes()[3], b'1'..=b'9'))
        || stamm == "COM\u{b9}"
        || stamm == "COM\u{b2}"
        || stamm == "COM\u{b3}"
        || stamm == "LPT\u{b9}"
        || stamm == "LPT\u{b2}"
        || stamm == "LPT\u{b3}"
}

#[cfg(windows)]
fn windows_quelle(quelle: &Path) -> Result<PathBuf, &'static str> {
    use std::path::{Component, Prefix};
    let letzter = quelle
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let praefix = match quelle.components().next() {
        Some(Component::Prefix(p)) => Some(p.kind()),
        _ => None,
    };
    let verbatim = match praefix {
        Some(Prefix::DeviceNS(_)) => return Err("ablage.geraet"),
        Some(Prefix::Verbatim(_)) => return Err("ablage.geraet"),
        Some(Prefix::VerbatimDisk(_)) | Some(Prefix::VerbatimUNC(..)) => true,
        _ => false,
    };
    if letzter.contains(':') {
        return Err("ablage.datenstrom");
    }
    if verbatim {
        return Ok(quelle.to_path_buf());
    }
    if ist_geraetename(&letzter) {
        return Err("ablage.geraet");
    }
    // Absolut machen (GetFullPathNameW), dann verbatim voranstellen.
    let abs = std::path::absolute(quelle).map_err(|_| "ablage.quelle_nicht_lesbar")?;
    let mut teile = abs.components();
    let neu = match teile.next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(b) => format!("\\\\?\\{}:", b as char),
            Prefix::UNC(server, share) => format!(
                "\\\\?\\UNC\\{}\\{}",
                server.to_string_lossy(),
                share.to_string_lossy()
            ),
            Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(..) => return Ok(abs),
            _ => return Err("ablage.geraet"),
        },
        _ => return Err("ablage.quelle_nicht_lesbar"),
    };
    let mut pfad = PathBuf::from(neu);
    for c in teile {
        match c {
            Component::RootDir => pfad.push("\\"),
            Component::Normal(n) => pfad.push(n),
            _ => return Err("ablage.quelle_nicht_lesbar"),
        }
    }
    Ok(pfad)
}

pub(super) fn bereinigen(name: &str) -> Res<String> {
    bereinigter_name(name).map_err(|_| Fehler::neu("ablage.name_ungueltig", "Ungültiger Name."))
}

/// Kein Glied von `rel` unter `wurzel` ist eine Verknüpfung oder ein Reparse-Point; ein fehlendes Glied
/// beendet die Prüfung.
fn ohne_umweg(wurzel: &Path, rel: &str) -> Res<()> {
    let mut pfad = wurzel.to_path_buf();
    for teil in rel.split('/').filter(|t| !t.is_empty()) {
        pfad.push(teil);
        match fs::symlink_metadata(&pfad) {
            Ok(m) if ist_umweg(&m) => return Err(verknuepfung()),
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
    /// D7: verknüpfter Ordner (nur lesen, nur dieses Gerät).
    pub verknuepft: bool,
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
    /// Stand der Textfassung der aktuellen Fassung: `ausstehend`, `ok`, `kein_text` oder `fehler`;
    /// `null` = Format ohne Textfassung (z. B. Bilder).
    pub text: Option<String>,
    /// Fehlercode bei `fehler` (z. B. `extrakt.zeit`, `extrakt.zip_grenze`); nie Inhalt.
    pub text_code: Option<String>,
    /// D7: Datei eines verknüpften Ordners (nur lesen; Name = Pfad im verknüpften Ordner).
    pub verknuepft: bool,
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
    /// KI-Freigaben wurden automatisch aus der Sicherung wiederhergestellt: Ordner ohne eigenen oder
    /// geerbten Eintrag sind gesperrt, bis die Bedienung sie neu setzt oder den Stand bestätigt.
    pub freigabe_wiederhergestellt: bool,
    /// D7: verknüpfte Ordner (gerätelokal, nur lesen) und ihre Dateien.
    pub verknuepft: Vec<super::verknuepft::AblageVerknuepft>,
    pub verknuepft_dateien: Vec<AblageDatei>,
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
    // Unter „Wiederhergestellt“ gilt zusätzlich eine Sperre der gespeicherten Elternkette.
    let w = freigabe::wirksam_mit_zusatz(lage, kette, zusatz);
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
        s.vorher = Some(s.vorher_berechnen());
        Ok(s)
    }

    fn sichern(&self, st: &mut Stand) -> Res<()> {
        if let Some(v) = st.vorher.take() {
            st.festhalten(&v);
            st.vorher = Some(st.vorher_berechnen());
        }
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
        self.entwurfsreste_raeumen();
        geaendert |= self.journal_abarbeiten(&mut st).unwrap_or(true);
        // Ein begonnener Rückweg (D4) zieht erst fertig um; sonst sähe die Suche unten die umgezogenen
        // Originale als Funde.
        let (umgezogen, uebergang_ok) = self.uebergaenge_lokal(&mut st);
        geaendert |= umgezogen;
        let erster_lauf = uebergang_ok
            && gescannt()
                .lock()
                .map(|mut g| g.insert(self.dir.clone()))
                .unwrap_or(false);
        if erster_lauf {
            geaendert |= self.gefundenes_suchen(&mut st);
        }
        geaendert |= self.abgelaufene_loeschen(&mut st, jetzt_ms());
        self.teildateien_loeschen(Some(jetzt_ms()));
        self.graeber_loeschen(jetzt_ms());
        self.texte_raeumen(&st);
        if st.ordner.is_empty() && st.dateien.is_empty() && st.begraben.is_empty() {
            // Ohne Ordner und Dateien ist der größte Zähler 0: der Überlauf ist hier ausgeschlossen.
            let z = st.zaehler().unwrap_or(1);
            let o = neuer_ordner(st.geraet(), None, STANDARD_ORDNER, z);
            st.ordner.insert(o.id.clone(), o);
            geaendert = true;
        }
        if (geaendert || frisch) && self.sichern(&mut st).is_err() {
            return Some(AblageLage::NichtLesbar);
        }
        None
    }

    /// Räumt liegengebliebene Arbeitsordner `.entwurf-<ulid>` (D3) im Ablagewurzelordner weg, wenn sie
    /// über eine Stunde alt sind (ein laufender Entwurf legt sie außerhalb der Sperre an). Verknüpfungen
    /// werden nie gefolgt.
    fn entwurfsreste_raeumen(&self) {
        let Ok(inhalt) = fs::read_dir(&self.dir) else {
            return;
        };
        let grenze = std::time::Duration::from_secs(3600);
        for e in inhalt.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if !n
                .strip_prefix(".entwurf-")
                .is_some_and(|r| r.len() == 26 && ulid::Ulid::from_string(r).is_ok())
            {
                continue;
            }
            let Ok(m) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            let alt = m
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|d| d >= grenze);
            if m.is_dir() && !ist_umweg(&m) && alt {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }

    /// Liegengebliebene Zwischendateien (`.teil-*`) in `dateien/<dat>/` gehen in den Papierkorb.
    /// Verknüpfungen werden nie gefolgt: führt `dateien/`, `papierkorb/` oder ein `dateien/<dat>/` über
    /// eine Verknüpfung, bleibt es unangetastet.
    fn zwischendateien_raeumen(&self) -> bool {
        let mut bewegt = false;
        if ohne_umweg(&self.dir, DATEIEN).is_err() || ohne_umweg(&self.dir, PAPIERKORB).is_err() {
            return false;
        }
        let Ok(dirs) = fs::read_dir(self.dir.join(DATEIEN)) else {
            return false;
        };
        for d in dirs.flatten() {
            let dat = d.file_name().to_string_lossy().into_owned();
            if !id_gueltig(PRAEFIX_DATEI, &dat) {
                continue;
            }
            match fs::symlink_metadata(d.path()) {
                Ok(m) if !ist_umweg(&m) && m.is_dir() => {}
                _ => continue,
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
                if ist_umweg(&m) || !m.is_file() {
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
        if ohne_umweg(&self.dir, PAPIERKORB).is_err() {
            return;
        }
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
            if ist_umweg(&m) || !m.is_file() {
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

    /// Vom Hub begrabene Inhalte (`papierkorb/grab-<ms>-…`) nach [`PAPIERKORB_TAGE`] endgültig entfernen.
    fn graeber_loeschen(&self, jetzt: i64) {
        if ohne_umweg(&self.dir, PAPIERKORB).is_err() {
            return;
        }
        let Ok(rest) = fs::read_dir(self.dir.join(PAPIERKORB)) else {
            return;
        };
        for e in rest.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(ms) = name
                .strip_prefix(GRAB)
                .and_then(|r| r.split('-').next())
                .and_then(|m| m.parse::<i64>().ok())
            else {
                continue;
            };
            if jetzt - ms < PAPIERKORB_TAGE * TAG_MS {
                continue;
            }
            match fs::symlink_metadata(e.path()) {
                Ok(m) if ist_umweg(&m) => {}
                Ok(m) if m.is_dir() => {
                    let _ = ordner_entfernen(&e.path());
                }
                Ok(_) => {
                    let _ = fs::remove_file(e.path());
                }
                Err(_) => {}
            }
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
                    vorgaenger,
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
                        if let Err(x) = anwenden_abgelegt(st, &e) {
                            fehler = Some(x);
                            rest.push(e);
                            rest.extend(it);
                            break;
                        }
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
                            vorgaenger: vorgaenger.clone(),
                        });
                    }
                }
                Eintrag::Loeschen {
                    dat,
                    ordner,
                    aufheben,
                } => {
                    if let Err(x) = self.loeschen_ausfuehren(st, dat, ordner, *aufheben) {
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
        if ohne_umweg(&self.dir, DATEIEN).is_err() {
            return false;
        }
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
                if ist_umweg(&fm) || !fm.is_file() || super::pfad_hartverknuepft(&f.path()) {
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
                    vorgaenger: None,
                });
                neu = true;
            }
        }
        neu
    }

    fn abgelaufene_loeschen(&self, st: &mut Stand, jetzt: i64) -> bool {
        // Erst offene Löschaufträge zu Ende führen; sonst entstünden doppelte Aufträge.
        if st
            .journal
            .iter()
            .any(|e| matches!(e, Eintrag::Loeschen { .. }))
        {
            return false;
        }
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
                freigabe_wiederhergestellt: false,
                verknuepft: vec![],
                verknuepft_dateien: vec![],
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
                    verknuepft: false,
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
        let (verknuepft, verknuepft_dateien) = super::verknuepft::sicht(self, &lage);
        Ok(AblageSicht {
            lage: None,
            verknuepft,
            verknuepft_dateien,
            freigabe_unklar: match &lage {
                freigabe::Lage::Unklar(u) => Some(*u),
                _ => None,
            },
            freigabe_wiederhergestellt: matches!(
                &lage,
                freigabe::Lage::Gueltig(f) if f.wiederhergestellt
            ),
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
        let z = st.zaehler()?;
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
        st.nicht_offen_geloescht(None, Some(id))?;
        if st.ordner_im_korb(eltern, id) {
            return Err(Fehler::neu(
                "ablage.ordner_im_papierkorb",
                "Der Ordner liegt im Papierkorb.",
            ));
        }
        Ok(())
    }

    /// Eine Datei, die bearbeitet werden darf: vorhanden, kein offener Löschauftrag, nicht im Papierkorb
    /// (auch nicht über ihren Ordner).
    fn datei_lebend(&self, st: &Stand, eltern: &BTreeMap<String, Eltern>, id: &str) -> Res<()> {
        let d = st.dateien.get(id).ok_or_else(nicht_gefunden)?;
        st.nicht_offen_geloescht(Some(id), None)?;
        if !st.datei_lebt(eltern, d) {
            return Err(Fehler::neu(
                "ablage.datei_im_papierkorb",
                "Die Datei liegt im Papierkorb.",
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
        let (z, g) = (st.zaehler()?, st.geraet().to_string());
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
        let (z, g) = (st.zaehler()?, st.geraet().to_string());
        let o = st.ordner.get_mut(id).ok_or_else(nicht_gefunden)?;
        o.eltern = Register::neu(neu_eltern.map(str::to_string), z, &g);
        self.sichern(&mut st)
    }

    /// Datenklasse, „Nur dieses Gerät“ und „KI-Entwürfe“ eines Ordners.
    pub fn ordner_einstellungen(&self, id: &str, e: &AblageEinstellungen) -> Res<()> {
        ordner_id_pruefen(id)?;
        let _g = sperren();
        // D7: verknüpfter Ordner: nur die Datenklasse ist einstellbar.
        if super::verknuepft::ist_verknuepft(&self.dir, id) {
            return super::verknuepft::einstellungen(self, id, e);
        }
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
        let (z, g) = (st.zaehler()?, st.geraet().to_string());
        let o = st.ordner.get_mut(id).ok_or_else(nicht_gefunden)?;
        let mut klasse_gewechselt = false;
        if let Some(k) = &e.datenklasse {
            if &o.datenklasse.wert != k {
                o.datenklasse = Register::neu(k.clone(), z, &g);
                klasse_gewechselt = true;
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
        self.sichern(&mut st)?;
        if klasse_gewechselt {
            // Cloud-Ausnahmen gelten nur für `confidential`: bei jedem Klassenwechsel verfallen sie für
            // den Ordner und alle Unterordner (die wirksame Klasse kann sich dort mitändern).
            let eltern = st.eltern();
            let betroffen: Vec<String> = st
                .ordner
                .keys()
                .filter(|k| zf::kette(&eltern, k).iter().any(|x| x == id))
                .cloned()
                .collect();
            freigabe::ausnahmen_entziehen(&self.dir, &betroffen)?;
        }
        Ok(())
    }

    /// KI-Freigabe eines Ordners (gerätelokal, `freigabe.json`); `None` = erbt wieder.
    pub fn ki_setzen(&self, id: &str, stufe: Option<KiStufe>) -> Res<()> {
        ordner_id_pruefen(id)?;
        let _g = sperren();
        // D7: verknüpfte Ordner tragen ihre Freigabe wie Ordner der Ablage.
        if super::verknuepft::ist_verknuepft(&self.dir, id) {
            return freigabe::setzen(&self.dir, id, stufe).map(|_| ());
        }
        let st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, id)?;
        freigabe::setzen(&self.dir, id, stufe).map(|_| ())
    }

    /// „Freigaben neu festlegen“: nur bei unklarer, defekter Freigabe; alle Ordner auf `aus`.
    pub fn freigaben_neu_festlegen(&self) -> Res<()> {
        let _g = sperren();
        let st = self.laden()?;
        let mut alle: Vec<String> = st.ordner.keys().cloned().collect();
        alle.extend(super::verknuepft::kennungen(&self.dir));
        freigabe::neu_festlegen(&self.dir, &alle).map(|_| ())
    }

    // --- Dateien -----------------------------------------------------------------

    pub fn datei_umbenennen(&self, id: &str, name: &str) -> Res<()> {
        datei_id_pruefen(id)?;
        let name = bereinigen(name)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        self.datei_lebend(&st, &eltern, id)?;
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
        let (z, g) = (st.zaehler()?, st.geraet().to_string());
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
        self.datei_lebend(&st, &eltern, id)?;
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
        let (z, g) = (st.zaehler()?, st.geraet().to_string());
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
        // Alte Kennung nach dem Rückweg lokal → sync (D4): der Weiterleitung folgen.
        let dat = st.aufgeloest(dat);
        let dat = dat.as_str();
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
        // Windows: Geräte, Datenströme und Gerätenamensräume abweisen; echte Dateien werden ab hier
        // nur noch verbatim (`\\?\`) adressiert, damit `Datei.`, `Datei `, `CON.txt` und lange Pfade
        // genau die Datei treffen, die im Ordner liegt.
        let quelle_pfad = match quelle_vorbereiten(quelle) {
            Ok(p) => p,
            Err(code) => return Ok(vec![AblageImportZeile::uebersprungen(quelle, code)]),
        };
        let quelle = quelle_pfad.as_path();
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
        let z = st.zaehler()?;
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
        match self.datei_ablegen(quelle, ordner, wahl, None) {
            Ok(z) => z,
            Err(e) => AblageImportZeile::fehler(quelle, &e),
        }
    }

    /// Schritte 1 bis 4 der sicheren Ablage für genau eine Datei. `entwurf` (D3): Quellen eines
    /// KI-Entwurfs; Klasse und „nur dieses Gerät“ werden unter der Sperre bestimmt und stehen schon im
    /// Journaleintrag.
    fn datei_ablegen(
        &self,
        quelle: &Path,
        ordner: &str,
        wahl: Option<AblageKollisionswahl>,
        entwurf: Option<(&[String], Klasse)>,
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
        let entwurf = match entwurf {
            Some((q, mindest)) => {
                let (k, nur) = ki_zugriff::entwurf_regel(&st, &eltern, ordner, q, mindest)?;
                if nur && !zf::wirksam_nur_geraet(&st.ordner, &eltern, ordner) {
                    return Err(Fehler::neu(
                        "ablage.entwurf_geaendert",
                        "Die Quellen haben sich geändert; bitte erneut versuchen.",
                    ));
                }
                Some((k, nur))
            }
            None => None,
        };
        if entwurf.is_some() && wahl != Some(AblageKollisionswahl::BeideBehalten) {
            return Err(Fehler::ungueltig("Ein Entwurf überschreibt nie."));
        }
        // Namenskollision im Baum (ohne Groß-/Kleinschreibung, nach NFC).
        let schluessel = vergleichsschluessel(&name);
        let (offen_dat, _) = st.offen_geloescht();
        let vorhanden = st
            .dateien
            .values()
            .find(|d| {
                d.ordner.wert == ordner
                    && d.papierkorb.wert.is_none()
                    && !offen_dat.contains(d.id.as_str())
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
        // Vor dem ersten Schreiben: ohne nächsten Zähler kommt nichts auf die Platte (fail-closed).
        st.zaehler()?;
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
            klasse_min: entwurf.map(|(k, _)| k.text().to_string()),
            nur_geraet_min: entwurf.is_some_and(|(_, n)| n),
        };
        st.journal.push(eintrag.clone());
        // Scheitert das Vermerken, liegt das Original ohne Eintrag da: der Wiederanlauf findet es.
        self.sichern(&mut st)?;
        haken_journal()?;
        anwenden_abgelegt(&mut st, &eintrag)?;
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
        let (z, g, jetzt) = (st.zaehler()?, st.geraet().to_string(), jetzt_ms());
        match art {
            AblageArt::Datei => {
                datei_id_pruefen(id)?;
                st.nicht_offen_geloescht(Some(id), None)?;
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
        let (z, g) = (st.zaehler()?, st.geraet().to_string());
        match art {
            AblageArt::Datei => {
                datei_id_pruefen(id)?;
                st.nicht_offen_geloescht(Some(id), None)?;
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
                st.nicht_offen_geloescht(None, Some(id))?;
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
        self.offene_zuerst(&mut st)?;
        if !st.dateien.contains_key(id)
            && !st.ordner.contains_key(id)
            && st.begraben.iter().any(|b| b == id)
        {
            // Ein offener Auftrag hat den Eintrag eben entfernt.
            self.sichern(&mut st)?;
            return Ok(AblageLoeschBericht::default());
        }
        let b = self.endgueltig_intern(&mut st, art, id)?;
        self.sichern(&mut st)?;
        Ok(b)
    }

    /// Leert den Papierkorb (alle Einträge der obersten Ebene).
    pub fn papierkorb_leeren(&self) -> Res<AblageLoeschBericht> {
        let _g = sperren();
        let mut st = self.laden()?;
        self.offene_zuerst(&mut st)?;
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

    /// Führt offene Journaleinträge (vor allem Löschaufträge) zu Ende, bevor ein neuer Auftrag entsteht.
    /// Scheitert einer, bleibt er im Journal und der Fehler geht an die Bedienung.
    fn offene_zuerst(&self, st: &mut Stand) -> Res<()> {
        if st.journal.is_empty() {
            return Ok(());
        }
        match self.journal_abarbeiten(st) {
            Ok(_) => Ok(()),
            Err(e) => {
                let _ = self.sichern(st);
                Err(e)
            }
        }
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
            aufheben: false,
        });
        self.sichern(st)?;
        haken_loeschen()?;
        self.loeschen_ausfuehren(st, &dat, &ord, false)?;
        st.journal.retain(
            |e| !matches!(e, Eintrag::Loeschen { dat: d, ordner: o, .. } if *d == dat && *o == ord),
        );
        Ok(AblageLoeschBericht {
            dateien: dat.len() as u32,
            ordner: ord.len() as u32,
        })
    }

    /// Entfernt Originale und Texte (über einen Zwischenort im Papierkorb), dann die Einträge im Baum.
    /// Wiederholbar: was schon fehlt, wird übersprungen.
    ///
    /// Jedes Glied von der Ablage abwärts (`dateien/`, `text/`, `papierkorb/`, `<dat>`) wird ohne Folgen
    /// geprüft: eine Verknüpfung oder ein Reparse-Point (Junction) bricht mit Fehler ab, nichts wird
    /// gefolgt. Nur „nicht vorhanden“ gilt als schon gelöscht; jeder andere Lesefehler bricht ab, der
    /// Löschauftrag und die Einträge im Baum bleiben für den nächsten Lauf.
    fn loeschen_ausfuehren(
        &self,
        st: &mut Stand,
        dat: &[String],
        ord: &[String],
        aufheben: bool,
    ) -> Res<()> {
        let jetzt = jetzt_ms();
        for wurzel in [DATEIEN, TEXT, PAPIERKORB] {
            ohne_umweg(&self.dir, wurzel)?;
        }
        for d in dat {
            if !id_gueltig(PRAEFIX_DATEI, d) {
                continue;
            }
            for wurzel in [DATEIEN, TEXT] {
                let quelle = self.dir.join(wurzel).join(d);
                let m = match meta_ohne_folgen(&quelle) {
                    Ok(m) => m,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => {
                        return Err(Fehler::speicher(
                            "Datei nicht lesbar; nichts gelöscht, der Auftrag bleibt.",
                        ))
                    }
                };
                if ist_umweg(&m) {
                    return Err(verknuepfung());
                }
                fs::create_dir_all(self.dir.join(PAPIERKORB))
                    .map_err(|_| Fehler::speicher("Papierkorb nicht anlegbar."))?;
                ohne_umweg(&self.dir, PAPIERKORB)?;
                if aufheben {
                    // Grabstein vom Hub: aufheben, die Wartung entfernt es nach der Frist.
                    let ziel = self
                        .dir
                        .join(PAPIERKORB)
                        .join(format!("{GRAB}{jetzt}-{wurzel}-{d}"));
                    if meta_ohne_folgen(&ziel).is_ok() {
                        continue; // schon aufgehoben (Wiederanlauf in derselben Millisekunde)
                    }
                    fs::rename(&quelle, &ziel).map_err(|_| {
                        Fehler::speicher("Datei nicht entfernbar; nichts verloren.")
                    })?;
                    continue;
                }
                let zwischen = self
                    .dir
                    .join(PAPIERKORB)
                    .join(format!("{d}{LOESCHEN}{}", neue_id("")));
                fs::rename(&quelle, &zwischen)
                    .map_err(|_| Fehler::speicher("Datei nicht entfernbar; nichts verloren."))?;
                ordner_entfernen(&zwischen)
                    .map_err(|_| Fehler::speicher("Datei nicht vollständig entfernt."))?;
            }
        }
        // Reste früherer, abgebrochener Läufe (nie einer Verknüpfung folgen). Zwischenorte, die zu
        // diesem Auftrag gehören, müssen nachweislich weg sein, sonst bleiben Auftrag und Baum
        // stehen; fremde Reste werden nach Möglichkeit geräumt.
        let rest = match fs::read_dir(self.dir.join(PAPIERKORB)) {
            Ok(r) => Some(r),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                return Err(Fehler::speicher(
                    "Papierkorb nicht lesbar; der Auftrag bleibt.",
                ))
            }
        };
        for e in rest.into_iter().flatten() {
            let e =
                e.map_err(|_| Fehler::speicher("Papierkorb nicht lesbar; der Auftrag bleibt."))?;
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(pos) = name.find(LOESCHEN) else {
                continue;
            };
            let eigen = dat.iter().any(|d| d.as_str() == &name[..pos]);
            let ergebnis = match meta_ohne_folgen(&e.path()) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(Fehler::speicher(
                    "Rest im Papierkorb nicht lesbar; der Auftrag bleibt.",
                )),
                Ok(m) if ist_umweg(&m) => Err(verknuepfung()),
                Ok(m) if m.is_dir() => ordner_entfernen(&e.path()).map_err(|_| {
                    Fehler::speicher("Rest im Papierkorb nicht entfernt; der Auftrag bleibt.")
                }),
                Ok(_) => fs::remove_file(e.path()).map_err(|_| {
                    Fehler::speicher("Rest im Papierkorb nicht entfernt; der Auftrag bleibt.")
                }),
            };
            if eigen {
                ergebnis?;
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
    /// die `dat` schon zu einer Datei im Baum, werden die Funde als weitere Fassungen aufgenommen. Ohne
    /// vermerkte Vorgängerin ist jeder Fund eine eigene Wurzel (weiterer Kopf); nichts wird verkettet.
    pub fn gefundene_aufnehmen(&self, dat: &str, ordner: &str) -> Res<()> {
        datei_id_pruefen(dat)?;
        ordner_id_pruefen(ordner)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, ordner)?;
        st.nicht_offen_geloescht(Some(dat), None)?;
        if let Some(d) = st.dateien.get(dat) {
            if !st.datei_lebt(&eltern, d) {
                return Err(Fehler::neu(
                    "ablage.datei_im_papierkorb",
                    "Die Datei liegt im Papierkorb.",
                ));
            }
        }
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
            if ist_umweg(&m) || !m.is_file() || super::pfad_hartverknuepft(&pfad) {
                return Err(Fehler::neu(
                    "ablage.quelle_ungueltig",
                    "Keine gewöhnliche Datei.",
                ));
            }
            let sha = sha256_datei(&pfad).map_err(|_| Fehler::speicher("Datei nicht lesbar."))?;
            neue.push(Fassung {
                fas: g.fas.clone(),
                vorgaenger: g.vorgaenger.clone(),
                blob: sha.clone(),
                size: m.len(),
                original_sha256: sha,
                typ: g.typ.clone(),
                geraet: st.geraet().to_string(),
                zeit: g.ms,
                texte: vec![],
            });
        }
        // Keine erfundene Abstammung: eine Vorgängerin bleibt nur, wenn das Journal sie vermerkt hatte und
        // sie bekannt ist. Sonst ist der Fund eine eigene Wurzel und erscheint als weiterer Kopf (Konflikt),
        // zwischen dem die Bedienung ausdrücklich wählt.
        let mut bekannt: BTreeSet<String> = neue.iter().map(|f| f.fas.clone()).collect();
        if let Some(d) = st.dateien.get(dat) {
            bekannt.extend(d.fassungen.iter().map(|f| f.fas.clone()));
        }
        for f in neue.iter_mut() {
            f.vorgaenger = f
                .vorgaenger
                .take()
                .filter(|v| *v != f.fas && bekannt.contains(v));
        }
        let (z, ger) = (st.zaehler()?, st.geraet().to_string());
        match st.dateien.get_mut(dat) {
            Some(d) => {
                d.fassungen.extend(neue);
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
                let fassungen = neue;
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
                        extra: Default::default(),
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
        st.nicht_offen_geloescht(Some(dat), None)?;
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
// D2: Textfassungen und Index (Spec §4)
// ---------------------------------------------------------------------------

/// Eine fehlende Textfassung: aktuelle Fassung einer lebenden Datei, Format mit Extraktor, keine
/// Textfassung mit der erwarteten `textrev`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextAuftrag {
    pub dat: String,
    pub fas: String,
    pub typ: String,
    pub sha256: String,
    pub textrev: String,
    pub pfad: PathBuf,
    /// Bisherige Versuche (vorübergehender Fehler), sonst 0.
    pub versuche: u32,
    /// Bisherige Zeitüberschreitungen (eigener Zähler, siehe [`MAX_VERSUCHE_ZEIT`]).
    pub versuche_zeit: u32,
}

/// Ausgang von [`Ablage::text_ablegen`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextAblage {
    /// Eingetragen wie zurückgegeben (mit Blob und Größe).
    Eingetragen(zf::Text),
    /// Schon eine endgültige Textfassung mit dieser `textrev`.
    Unveraendert,
    /// Datei oder Fassung gibt es nicht mehr (endgültig gelöscht): nichts geschrieben, Rest geräumt.
    Verschwunden,
    /// Textdatei nicht schreibbar (Datenträger, Verknüpfung): nichts eingetragen.
    NichtSpeicherbar,
}

/// Fehlercodes, die nicht am Inhalt liegen müssen (Start, Schutz, Lesen): sie werden höchstens
/// [`MAX_VERSUCHE`]-mal versucht, jeweils frühestens beim nächsten Programmstart.
///
/// Bewusst nicht dabei:
/// * `extrakt.abbruch` (Speichergrenze, Absturz oder Signal des Kindes): Giftdatei, sofort endgültig
///   nicht extrahierbar; jeder weitere Versuch risikiert erneut Speicher und Rechenzeit.
/// * `extrakt.zeit` (Zeitgrenze): genau ein späterer Wiederholversuch ([`MAX_VERSUCHE_ZEIT`]) mit nach
///   Dateigröße vergrößerter Zeitgrenze ([`super::textfassung::zeitgrenze`]), danach endgültig.
///
/// Ein Abbruch durch die Bedienung (`extrakt.abgebrochen`) trägt nichts ein und ist davon nicht berührt.
pub const VORUEBERGEHEND: &[&str] = &[
    "extrakt.start",
    "extrakt.programm",
    "extrakt.schutz",
    "extrakt.lesen",
];
pub const MAX_VERSUCHE: u32 = 3;
/// Zeitgrenze überschritten: der erste Versuch plus genau ein Wiederholversuch.
pub const MAX_VERSUCHE_ZEIT: u32 = 2;

/// Ein vorübergehender Fehler mit Versuchen übrig: der Auftrag bleibt offen.
pub fn text_wiederholbar(t: &zf::Text) -> bool {
    if t.stand != "fehler" {
        return false;
    }
    match t.code.as_deref() {
        Some("extrakt.zeit") => t.versuche_zeit < MAX_VERSUCHE_ZEIT,
        Some(c) => VORUEBERGEHEND.contains(&c) && t.versuche < MAX_VERSUCHE,
        None => false,
    }
}

/// Was der Index zu einer Datei wissen muss (auch im Papierkorb: dort gefiltert, nicht gelöscht).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexQuelle {
    pub dat: String,
    pub fas: String,
    /// `textrev` der indizierten Textfassung (`stand == "ok"`), sonst `None`.
    pub textrev: Option<String>,
    pub stand: String,
    /// Ordnerkette (IDs) vom Ordner der Datei bis zur obersten Ebene.
    pub ordnerkette: Vec<String>,
    pub klasse: String,
    /// Wirksame KI-Stufe des Ordners der Datei (`an`, `aus`, `gesperrt`, `unklar`). Geht in den Stempel
    /// ein, damit D3 bei einer Freigabeänderung genau die betroffenen Dateien neu bewertet.
    pub ki: String,
    pub nur_geraet: bool,
    pub papierkorb: bool,
    /// D7: Datei eines verknüpften Ordners (Textfassung unter `verknuepft/text/`).
    pub verknuepft: bool,
}

/// Name und Ort einer lebenden Datei für die Trefferliste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrefferInfo {
    pub name: String,
    pub ordner: String,
    pub typ: String,
}

/// Erwartete `textrev` der Fassung mit dem aktuellen Extraktor; `None` = Format ohne Textfassung.
pub fn erwartete_textrev(f: &Fassung) -> Option<String> {
    let format = super::extrakt::format_aus_typ(&f.typ)?;
    let (id, version) = super::extrakt::extraktor(format);
    Some(super::extrakt::textrev(
        &f.original_sha256,
        id,
        version,
        super::extrakt::MARKEN_VERSION,
    ))
}

/// Textstand für die Oberfläche: Stand der erwarteten Textfassung, sonst `ausstehend` (auch während
/// eine ältere Extraktorversion noch gilt); `None` = Format ohne Textfassung.
fn textstand(f: &Fassung) -> (Option<String>, Option<String>) {
    let Some(rev) = erwartete_textrev(f) else {
        return (None, None);
    };
    match f.texte.iter().find(|t| t.textrev == rev) {
        Some(t) => (Some(t.stand.clone()), t.code.clone()),
        None => (Some("ausstehend".into()), None),
    }
}

impl Ablage {
    pub fn text_pfad(&self, dat: &str, textrev: &str) -> PathBuf {
        self.dir
            .join(TEXT)
            .join(dat)
            .join(format!("{textrev}.json"))
    }

    /// Fehlende Textfassungen (Wiederanlauf: alles, was noch fehlt, steht hier; es gibt keine eigene
    /// Warteschlangendatei). Endgültige Fehler werden nicht wiederholt, vorübergehende bis
    /// [`MAX_VERSUCHE`] ([`text_wiederholbar`]).
    pub fn text_auftraege(&self) -> Res<Vec<TextAuftrag>> {
        let _g = sperren();
        let st = self.laden()?;
        let eltern = st.eltern();
        let (offen, _) = st.offen_geloescht();
        let mut aus = Vec::new();
        for d in st.dateien.values() {
            if !st.datei_lebt(&eltern, d) || offen.contains(d.id.as_str()) {
                continue;
            }
            let Some(f) = aktuelle_fassung(d) else {
                continue;
            };
            let Some(rev) = erwartete_textrev(f) else {
                continue;
            };
            let vorhanden = f.texte.iter().find(|t| t.textrev == rev);
            if vorhanden.is_some_and(|t| !text_wiederholbar(t)) {
                continue;
            }
            let name = original_name(&f.fas, &f.typ);
            if ohne_umweg(&self.dir, &format!("{DATEIEN}/{}/{name}", d.id)).is_err() {
                continue;
            }
            // D4: Eine Fassung von einem anderen Gerät, deren Original noch nicht hier liegt, wartet auf
            // ihren Text (oder ihr Original); ein Lauf ohne Datei würde einen Fehler eintragen.
            if !self.dir.join(DATEIEN).join(&d.id).join(&name).is_file() {
                continue;
            }
            aus.push(TextAuftrag {
                dat: d.id.clone(),
                fas: f.fas.clone(),
                typ: f.typ.clone(),
                sha256: f.original_sha256.clone(),
                textrev: rev,
                pfad: self.dir.join(DATEIEN).join(&d.id).join(name),
                versuche: vorhanden.map(|t| t.versuche).unwrap_or(0),
                versuche_zeit: vorhanden.map(|t| t.versuche_zeit).unwrap_or(0),
            });
        }
        Ok(aus)
    }

    /// Schreibt eine Textfassung atomar nach `text/<dat>/<textrev>.json` (ohne Folgen von Verknüpfungen).
    /// Liefert die Blob-Kennung (SHA-256 der Bytes) und die Größe.
    pub fn text_schreiben(&self, dat: &str, textrev: &str, json: &[u8]) -> Res<(String, u64)> {
        datei_id_pruefen(dat)?;
        if textrev.len() != 64 || !textrev.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Fehler::ungueltig("Textfassung ungültig."));
        }
        ohne_umweg(&self.dir, &format!("{TEXT}/{dat}"))?;
        fs::create_dir_all(self.dir.join(TEXT).join(dat))
            .map_err(|_| Fehler::speicher("Textordner nicht anlegbar."))?;
        ohne_umweg(&self.dir, &format!("{TEXT}/{dat}/{textrev}.json"))?;
        schreibe_atomar(&self.text_pfad(dat, textrev), json)?;
        Ok((super::extrakt::hex(&sha2_digest(json)), json.len() as u64))
    }

    /// Legt eine Textfassung (`json`, falls vorhanden) ab und trägt sie (oder ihren Fehler) bei der
    /// Fassung ein: alles unter der Ablage-Sperre, damit ein gleichzeitiges endgültiges Löschen keine
    /// Reste hinterlässt. Ist Datei oder Fassung inzwischen weg, wird nichts geschrieben und ein leerer
    /// `text/<dat>/` entfernt. Ein vorübergehender Fehler wird durch das neue Ergebnis ersetzt.
    pub fn text_ablegen(
        &self,
        dat: &str,
        fas: &str,
        json: Option<&[u8]>,
        mut text: zf::Text,
    ) -> Res<TextAblage> {
        datei_id_pruefen(dat)?;
        let _g = sperren();
        let mut st = self.laden()?;
        let (offen, _) = st.offen_geloescht();
        let geloescht = offen.contains(dat);
        let Some(d) = st.dateien.get_mut(dat).filter(|_| !geloescht) else {
            self.textordner_leer_entfernen(dat);
            return Ok(TextAblage::Verschwunden);
        };
        let Some(f) = d.fassungen.iter_mut().find(|f| f.fas == fas) else {
            self.textordner_leer_entfernen(dat);
            return Ok(TextAblage::Verschwunden);
        };
        let alt = f.texte.iter().position(|t| t.textrev == text.textrev);
        if alt.is_some_and(|i| !text_wiederholbar(&f.texte[i])) {
            return Ok(TextAblage::Unveraendert);
        }
        let mut geschrieben = None;
        if let Some(json) = json {
            match self.text_schreiben(dat, &text.textrev, json) {
                Ok((blob, size)) => {
                    text.blob = blob;
                    text.size = size;
                    geschrieben = Some(self.text_pfad(dat, &text.textrev));
                }
                Err(_) => {
                    self.textordner_leer_entfernen(dat);
                    return Ok(TextAblage::NichtSpeicherbar);
                }
            }
        }
        let eingetragen = text.clone();
        match alt {
            Some(i) => f.texte[i] = text,
            None => f.texte.push(text),
        }
        d.blob_refs = zf::blob_refs_aus(&d.fassungen);
        if let Err(e) = self.sichern(&mut st) {
            if let Some(p) = geschrieben {
                let _ = fs::remove_file(p);
            }
            self.textordner_leer_entfernen(dat);
            return Err(e);
        }
        Ok(TextAblage::Eingetragen(eingetragen))
    }

    /// Entfernt `text/<dat>/`, wenn es leer ist (nie über eine Verknüpfung).
    fn textordner_leer_entfernen(&self, dat: &str) {
        if ohne_umweg(&self.dir, &format!("{TEXT}/{dat}")).is_ok() {
            let _ = fs::remove_dir(self.dir.join(TEXT).join(dat));
        }
    }

    /// Wartung: `text/<dat>/` ohne Datei im Bestand (weder lebend noch im Papierkorb) wird entfernt.
    /// Verknüpfungen werden nie gefolgt und bleiben stehen.
    fn texte_raeumen(&self, st: &Stand) {
        if ohne_umweg(&self.dir, TEXT).is_err() {
            return;
        }
        let Ok(dirs) = fs::read_dir(self.dir.join(TEXT)) else {
            return;
        };
        let (offen, _) = st.offen_geloescht();
        for e in dirs.flatten() {
            let dat = e.file_name().to_string_lossy().into_owned();
            if !id_gueltig(PRAEFIX_DATEI, &dat)
                || (st.dateien.contains_key(&dat) && !offen.contains(dat.as_str()))
            {
                continue;
            }
            match fs::symlink_metadata(e.path()) {
                Ok(m) if !ist_umweg(&m) && m.is_dir() => {
                    let _ = fs::remove_dir_all(e.path());
                }
                _ => {}
            }
        }
    }

    /// Alle Dateien für den Index (lebend und im Papierkorb; nicht endgültig gelöschte).
    pub fn index_quellen(&self) -> Res<Vec<IndexQuelle>> {
        let _g = sperren();
        let st = self.laden()?;
        let eltern = st.eltern();
        let (offen, _) = st.offen_geloescht();
        let lage = freigabe::laden(&self.dir);
        let mut aus = Vec::new();
        for d in st.dateien.values() {
            if offen.contains(d.id.as_str()) {
                continue;
            }
            let Some(f) = aktuelle_fassung(d) else {
                continue;
            };
            let erwartet = erwartete_textrev(f);
            // Die Textfassung des aktuellen Extraktors, sonst die letzte brauchbare ältere (sie gilt bis
            // zum Neuaufbau, Spec §4.2).
            let gewaehlt = f
                .texte
                .iter()
                .find(|t| Some(&t.textrev) == erwartet.as_ref() && t.stand != "fehler")
                .or_else(|| f.texte.iter().rev().find(|t| t.stand == "ok"));
            let stand = match (gewaehlt, &erwartet) {
                (Some(t), _) => t.stand.clone(),
                (None, Some(_)) => f
                    .texte
                    .iter()
                    .find(|t| Some(&t.textrev) == erwartet.as_ref())
                    .map(|t| t.stand.clone())
                    .unwrap_or_else(|| "ausstehend".into()),
                (None, None) => "kein_format".into(),
            };
            let kette = zf::kette(&eltern, &d.ordner.wert);
            let zusatz = zf::zusatzkette(&st.ordner, &eltern, &d.ordner.wert);
            let w = freigabe::wirksam_mit_zusatz(&lage, &kette, &zusatz);
            let ki = if w.unklar {
                "unklar"
            } else {
                match w.stufe {
                    KiStufe::An => "an",
                    KiStufe::Aus => "aus",
                    KiStufe::Gesperrt => "gesperrt",
                }
            };
            aus.push(IndexQuelle {
                dat: d.id.clone(),
                fas: f.fas.clone(),
                textrev: gewaehlt
                    .filter(|t| t.stand == "ok")
                    .map(|t| t.textrev.clone()),
                stand,
                ordnerkette: kette,
                klasse: zf::wirksame_klasse_datei(&st.ordner, &eltern, d)
                    .text()
                    .into(),
                ki: ki.into(),
                nur_geraet: d.nur_geraet_min
                    || zf::wirksam_nur_geraet(&st.ordner, &eltern, &d.ordner.wert),
                papierkorb: !st.datei_lebt(&eltern, d),
                verknuepft: false,
            });
        }
        Ok(aus)
    }

    /// Name, Ordner und Typ lebender Dateien (Dateien im Papierkorb oder unbekannte fehlen).
    pub fn treffer_infos(&self, dats: &[String]) -> Res<BTreeMap<String, TrefferInfo>> {
        let _g = sperren();
        let st = self.laden()?;
        let eltern = st.eltern();
        Ok(dats
            .iter()
            .filter_map(|id| {
                let d = st.dateien.get(id)?;
                if !st.datei_lebt(&eltern, d) {
                    return None;
                }
                Some((
                    id.clone(),
                    TrefferInfo {
                        name: d.name.wert.clone(),
                        ordner: d.ordner.wert.clone(),
                        typ: aktuelle_fassung(d)
                            .map(|f| f.typ.clone())
                            .unwrap_or_default(),
                    },
                ))
            })
            .collect())
    }
}

fn sha2_digest(b: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    sha2::Sha256::digest(b).to_vec()
}

// ---------------------------------------------------------------------------
// Hilfen ohne Zustand
// ---------------------------------------------------------------------------

fn zaehler_erschoepft() -> Fehler {
    Fehler::neu(
        "ablage.zaehler_erschoepft",
        "Die Ablage kann gerade keine Änderungen mehr aufnehmen.",
    )
}

fn nicht_gefunden() -> Fehler {
    Fehler::neu("ablage.nicht_gefunden", "Nicht gefunden.")
}

fn verknuepfung() -> Fehler {
    Fehler::neu(
        "ablage.verknuepfung",
        "Der Pfad führt über eine Verknüpfung; nichts angefasst.",
    )
}

fn loeschen_offen() -> Fehler {
    Fehler::neu(
        "ablage.loeschen_offen",
        "Ein endgültiges Löschen ist noch nicht abgeschlossen; nichts geändert.",
    )
}

/// `remove_dir_all`, in Tests mit einstellbarem Fehler.
fn ordner_entfernen(p: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if ABBRUCH.with(|a| a.get()) == Some(Schritt::LoeschenEntfernfehler) {
        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    }
    fs::remove_dir_all(p)
}

/// `symlink_metadata`, in Tests mit einstellbarem Lesefehler.
fn meta_ohne_folgen(p: &Path) -> std::io::Result<fs::Metadata> {
    #[cfg(test)]
    if ABBRUCH.with(|a| a.get()) == Some(Schritt::LoeschenLesefehler) {
        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    }
    fs::symlink_metadata(p)
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
        extra: Default::default(),
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
        // Befreite Einträge (D4) bleiben synchron, auch unter einem Ordner „nur dieses Gerät“.
        nur_geraet: !st.befreit.contains(&d.id)
            && (d.nur_geraet_min || zf::wirksam_nur_geraet(&st.ordner, eltern, &d.ordner.wert)),
        text: f.and_then(|f| textstand(f).0),
        text_code: f.and_then(|f| textstand(f).1),
        verknuepft: false,
    }
}

/// Trägt eine abgelegte Fassung in den Baum ein (idempotent).
fn anwenden_abgelegt(st: &mut Stand, e: &Eintrag) -> Res<()> {
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
        klasse_min,
        nur_geraet_min,
        ..
    } = e
    else {
        return Ok(());
    };
    let z = st.zaehler()?;
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
                    vorgaenger: vorgaenger.clone(),
                });
                return Ok(());
            }
            st.dateien.insert(
                dat.clone(),
                Datei {
                    id: dat.clone(),
                    ordner: Register::neu(ordner.clone(), z, &g),
                    name: Register::neu(name.clone(), z, &g),
                    klasse_min: Register::neu(
                        klasse_min.clone().unwrap_or_else(|| "public".into()),
                        z,
                        &g,
                    ),
                    nur_geraet_min: *nur_geraet_min,
                    blob_refs: zf::blob_refs_aus(std::slice::from_ref(&fassung)),
                    fassungen: vec![fassung],
                    wahlen: vec![],
                    entfernt: vec![],
                    papierkorb: Register::neu(None, z, &g),
                    extra: Default::default(),
                },
            );
        }
    }
    Ok(())
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
