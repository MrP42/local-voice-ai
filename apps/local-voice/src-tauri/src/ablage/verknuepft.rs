//! D7 (Spec §8): verknüpfte Ordner. Gerätelokal, nur lesen, nie hochladen, nie synchronisieren.
//!
//! Ein Ordner des Geräts wird über den Ordnerdialog verknüpft; seine Dateien bleiben, wo sie sind. Die Ablage
//! liest sie nur: Bestand (relativer Pfad, Größe, Änderungszeit), Textfassung über eine geprüfte
//! Momentkopie im vorhandenen Unterprozess `lva-extrakt`, lokale OCR für Bild-PDFs und Bilder, Index und
//! Suche. Datenklasse (Vorgabe `confidential`) und KI-Freigabe (`freigabe.json`) gelten wie bei Ordnern.
//!
//! Dateien (alle gerätelokal unter `ablage/`, nie unter `dateien/` oder `text/`, damit Wartung, Papierkorb und
//! Sync der eigenen Ablage nichts davon sehen):
//!
//! * `verknuepft.json`: die Liste (`ord_<ULID>`, kanonischer Pfad, Name, Klasse), Temp + Umbenennen.
//! * `verknuepft/<ord>/bestand.json`: je Datei `dat_<ULID>`, Größe, Änderungszeit, Typ, Stand der Textfassung.
//! * `verknuepft/text/<dat>/<textrev>.json`: Textfassungen (Markenformat v1).
//! * `verknuepft/zwischen/`: Momentkopien, nur während einer Extraktion.
//!
//! Schutz: der verknüpfte Pfad ist kanonisch (kein Glied eine Verknüpfung) und wird vor jedem Abtasten erneut
//! geprüft; im Baum werden Verknüpfungen (Symlink, Junction, unbekannte Reparse Points) nie gefolgt,
//! Hartlinks abgewiesen, Ausschlüsse (`~$*`, `*.tmp`, Systemordner) übersprungen. Windows: Cloud-Platzhalter
//! (OneDrive) gelten nur dann nicht als Verknüpfung, wenn die Datei auf dem Gerät liegt. Liegt sie nur online
//! (`RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN` oder `OFFLINE`), erscheint sie mit Name, Pfad und Größe und dem
//! Stand [`NUR_ONLINE`] (kein Text, kein Index, keine Kopie), ein solcher Ordner wird nicht betreten; nie wird
//! ein Download ausgelöst. Attribute und Tag kommen unter Windows nur aus `FindFirstFileExW` ([`eintrag`]),
//! nie aus `fs::symlink_metadata`. Die Momentkopie prüft vor dem Öffnen
//! die Wurzel und jede Komponente des relativen Pfades (nicht nur die letzte), öffnet die Datei ohne Folgen
//! von Verknüpfungen, prüft am offenen Handle erneut auf „nur online“ (dann wird nicht gelesen), vergleicht
//! Größe und Änderungszeit am offenen Handle mit dem Bestand, hasht beim
//! Kopieren, prüft die Quelle nach dem Kopieren erneut und hasht die Kopie noch einmal. Bleibt nach einem
//! Absturz eine Kopie in `zwischen/` liegen, räumen Beobachter, Textarbeiter und das Lösen sie weg. Nie Namen,
//! Pfade oder Inhalte im Log.
//!
//! Dateibeobachter: ein Hintergrundfaden tastet die verknüpften Ordner im Takt [`TAKT`] ab und übernimmt eine
//! Änderung erst, wenn sie über die Ruhezeit [`RUHE`] stabil bleibt ([`Entpreller`]); dann Bestand, Index und
//! Textarbeiter. Abfrage statt Systembenachrichtigung: keine neue Abhängigkeit, und Netz- oder Cloud-Laufwerke
//! melden Änderungen oft gar nicht.

use super::eintrag::{self, Gattung, Urteil};
use super::extrakt;
use super::freigabe::{self, KiStufe};
use super::ocr::{self, Kennung};
use super::speicher::{Ablage, AblageDatei, AblageKi, AblageOrdner, IndexQuelle, TrefferInfo};
use super::textlauf::{self, Grenzen, Programm};
use super::{
    datei_hartverknuepft, id_gueltig, ist_umweg, jetzt_ms, neue_id, ordner_sync, schreibe_atomar,
    Fehler, Res, PRAEFIX_DATEI, PRAEFIX_ORDNER,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub const LISTE: &str = "verknuepft.json";
pub const ORDNER: &str = "verknuepft";
const BESTAND: &str = "bestand.json";
const TEXT: &str = "text";
const ZWISCHEN: &str = "zwischen";
const VERSION: u32 = 1;
pub const STANDARD_KLASSE: &str = "confidential";
const KLASSEN: [&str; 4] = ["public", "internal", "confidential", "personal"];
pub const MAX_TIEFE: usize = 24;
pub const MAX_DATEIEN: usize = 20_000;
pub const MAX_VERKNUEPFT: usize = 32;
/// Abfragetakt des Dateibeobachters.
pub const TAKT: Duration = Duration::from_secs(20);
/// Ruhezeit der Entprellung: eine Änderung gilt erst, wenn die nächste Abtastung danach gleich aussieht.
pub const RUHE: Duration = Duration::from_secs(3);
/// Höchstens so oft wird „zuletzt abgetastet“ ohne Änderung in den Bestand geschrieben (ms).
const STEMPEL_MS: i64 = 60_000;

/// Gespeicherte Stände eines Eintrags (`stand`).
pub const AUSSTEHEND: &str = "ausstehend";
pub const KEIN_FORMAT: &str = "kein_format";
/// Hartlink (erst beim Lesen erkannt): wie ausgeschlossen, nie gelesen, nicht im Index.
pub const ABGEWIESEN: &str = "abgewiesen";
/// Windows: die Datei liegt nur online (OneDrive „Dateien bei Bedarf“). Sie ist in der Liste sichtbar (Name,
/// Pfad, Größe aus den Find-Daten), wird aber nie gelesen, geöffnet oder kopiert: kein Text, kein Index.
pub const NUR_ONLINE: &str = "nur_online";

static SPERRE: Mutex<()> = Mutex::new(());

/// Sperre der Dateien dieses Moduls (Liste, Bestände, Texte). Reihenfolge: erst die Sperre der Ablage
/// ([`super::sperren`]), dann diese; nie umgekehrt.
fn sperren() -> std::sync::MutexGuard<'static, ()> {
    SPERRE.lock().unwrap_or_else(|e| e.into_inner())
}

fn fehler_defekt() -> Fehler {
    Fehler::neu(
        "ablage.verknuepft_defekt",
        "Die Liste der verknüpften Ordner ist nicht lesbar; nichts geändert.",
    )
}

fn fehler_nur_online() -> Fehler {
    Fehler::neu(
        "ablage.nur_online",
        "Der Ordner liegt nur online und wird nicht gelesen.",
    )
}

fn nicht_gefunden() -> Fehler {
    Fehler::neu("ablage.nicht_gefunden", "Nicht gefunden.")
}

pub(super) fn nur_lesen() -> Fehler {
    Fehler::neu(
        "ablage.verknuepft_nur_lesen",
        "Verknüpfte Ordner sind nur lesbar und bleiben auf diesem Gerät.",
    )
}

// ---------------------------------------------------------------------------
// Liste und Bestand
// ---------------------------------------------------------------------------

/// Ein verknüpfter Ordner (gerätelokal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verknuepfung {
    pub id: String,
    /// Kanonischer Pfad (ohne Verknüpfung in einem Glied).
    pub pfad: String,
    pub name: String,
    pub klasse: String,
    pub angelegt: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Liste {
    v: u32,
    #[serde(default)]
    ordner: Vec<Verknuepfung>,
}

/// Eine Datei im verknüpften Ordner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eintrag {
    pub dat: String,
    pub groesse: u64,
    /// Änderungszeit (ms seit 1970).
    pub zeit: i64,
    pub typ: String,
    /// Prüfsumme der letzten Momentkopie.
    #[serde(default)]
    pub sha256: Option<String>,
    /// `textrev` der gespeicherten Textfassung (nur bei `ok`).
    #[serde(default)]
    pub textrev: Option<String>,
    /// `ausstehend`, `ok`, `kein_text`, `fehler`, `kein_format`, `abgewiesen`, `nur_online`.
    pub stand: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub versuche: u32,
    /// OCR-Extraktor, mit dem schon gelesen wurde (`windows-ocr@1`): kein zweiter Versuch derselben OCR.
    #[serde(default)]
    pub ocr: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bestand {
    #[serde(default)]
    pub v: u32,
    /// Relativer Pfad (mit `/`) → Eintrag.
    #[serde(default)]
    pub dateien: BTreeMap<String, Eintrag>,
    /// Übersprungen: Verknüpfungen, Hartlinks, Geräte, zu tief, nicht als UTF-8 lesbar.
    #[serde(default)]
    pub ausgelassen: u32,
    /// Unterordner, die nur online liegen und deshalb nicht betreten wurden (die Dateien darin zählen nicht
    /// einzeln; Dateien, die nur online liegen, stehen mit dem Stand `nur_online` in `dateien`).
    #[serde(default)]
    pub nur_online_ordner: u32,
    #[serde(default)]
    pub zu_viele: bool,
    /// Der Ordner war beim letzten Abtasten nicht erreichbar (entfernt, getauscht, Laufwerk fehlt).
    #[serde(default)]
    pub nicht_erreichbar: bool,
    #[serde(default)]
    pub abgetastet: i64,
}

fn liste_pfad(dir: &Path) -> PathBuf {
    dir.join(LISTE)
}

fn ordner_dir(dir: &Path) -> PathBuf {
    dir.join(ORDNER)
}

fn bestand_pfad(dir: &Path, id: &str) -> PathBuf {
    ordner_dir(dir).join(id).join(BESTAND)
}

/// Textfassung einer Datei aus einem verknüpften Ordner.
pub fn text_pfad(dir: &Path, dat: &str, textrev: &str) -> PathBuf {
    ordner_dir(dir)
        .join(TEXT)
        .join(dat)
        .join(format!("{textrev}.json"))
}

/// Ist kein Glied von `rel` unter `dir` eine Verknüpfung? (Fehlende Glieder beenden die Prüfung.)
fn ohne_umweg(dir: &Path, rel: &[&str]) -> Res<()> {
    let mut p = dir.to_path_buf();
    for t in rel {
        p.push(t);
        match fs::symlink_metadata(&p) {
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

fn json_lesen<T: for<'de> Deserialize<'de> + Default>(p: &Path) -> Result<T, ()> {
    match fs::symlink_metadata(p) {
        Ok(m) if ist_umweg(&m) || !m.is_file() => return Err(()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(_) => return Err(()),
    }
    let roh = fs::read(p).map_err(|_| ())?;
    serde_json::from_slice(&roh).map_err(|_| ())
}

fn liste_lesen(dir: &Path) -> Res<Liste> {
    let l: Liste = json_lesen(&liste_pfad(dir)).map_err(|_| fehler_defekt())?;
    if l.v > VERSION || l.ordner.iter().any(|o| !id_gueltig(PRAEFIX_ORDNER, &o.id)) {
        return Err(fehler_defekt());
    }
    Ok(l)
}

fn liste_schreiben(dir: &Path, l: &Liste) -> Res<()> {
    fs::create_dir_all(dir).map_err(|_| Fehler::speicher("Ablageordner nicht anlegbar."))?;
    ohne_umweg(dir, &[LISTE])?;
    let roh =
        serde_json::to_vec_pretty(l).map_err(|_| Fehler::speicher("Liste nicht speicherbar."))?;
    schreibe_atomar(&liste_pfad(dir), &roh)
}

/// Die verknüpften Ordner (fehlende Datei = keine). Defekt: Fehler (nichts wird darüber geschrieben).
pub fn liste(dir: &Path) -> Res<Vec<Verknuepfung>> {
    let _g = sperren();
    Ok(liste_lesen(dir)?.ordner)
}

/// Ist `id` ein verknüpfter Ordner?
pub fn ist_verknuepft(dir: &Path, id: &str) -> bool {
    id_gueltig(PRAEFIX_ORDNER, id)
        && liste(dir)
            .map(|l| l.iter().any(|v| v.id == id))
            .unwrap_or(false)
}

fn bestand_lesen(dir: &Path, id: &str) -> Bestand {
    let b: Bestand = json_lesen(&bestand_pfad(dir, id)).unwrap_or_default();
    if b.dateien
        .values()
        .any(|e| !id_gueltig(PRAEFIX_DATEI, &e.dat))
    {
        // Fremde Kennungen: neu abtasten statt vertrauen.
        return Bestand::default();
    }
    b
}

fn bestand_schreiben(dir: &Path, id: &str, b: &Bestand) -> Res<()> {
    ohne_umweg(dir, &[ORDNER, id])?;
    fs::create_dir_all(ordner_dir(dir).join(id))
        .map_err(|_| Fehler::speicher("Ordner nicht anlegbar."))?;
    ohne_umweg(dir, &[ORDNER, id, BESTAND])?;
    let mut b = b.clone();
    b.v = VERSION;
    let roh = serde_json::to_vec(&b).map_err(|_| Fehler::speicher("Bestand nicht speicherbar."))?;
    schreibe_atomar(&bestand_pfad(dir, id), &roh)
}

/// Bestand eines verknüpften Ordners (Tests, Sicht).
pub fn bestand(dir: &Path, id: &str) -> Bestand {
    let _g = sperren();
    bestand_lesen(dir, id)
}

fn texte_entfernen(dir: &Path, dat: &str) {
    if id_gueltig(PRAEFIX_DATEI, dat) && ohne_umweg(dir, &[ORDNER, TEXT, dat]).is_ok() {
        let p = ordner_dir(dir).join(TEXT).join(dat);
        if let Ok(m) = fs::symlink_metadata(&p) {
            if !ist_umweg(&m) && m.is_dir() {
                let _ = fs::remove_dir_all(&p);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Verknüpfen und lösen
// ---------------------------------------------------------------------------

fn gleich(a: &Path, b: &Path) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

fn liegt_in(innen: &Path, aussen: &Path) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        let i = PathBuf::from(innen.to_string_lossy().to_lowercase());
        let a = PathBuf::from(aussen.to_string_lossy().to_lowercase());
        i.starts_with(a)
    } else {
        innen.starts_with(aussen)
    }
}

/// Prüft den Pfad eines (neuen oder bestehenden) verknüpften Ordners: Ordner, keine Verknüpfung in einem
/// Glied (kanonisch gleich), keine Laufwerkswurzel. Liefert den kanonischen Pfad.
fn wurzel_pruefen(pfad: &Path) -> Res<PathBuf> {
    let unerreichbar = || {
        Fehler::neu(
            "ablage.nicht_erreichbar",
            "Der verknüpfte Ordner ist nicht erreichbar.",
        )
    };
    // Attribute zuerst und ohne zu öffnen (Windows: `FindFirstFileExW`); `canonicalize` öffnet den Ordner.
    let r = eintrag::roh(pfad).map_err(|_| unerreichbar())?;
    match r.urteil() {
        Urteil::Umweg => {
            return Err(Fehler::neu(
                "ablage.verknuepfung",
                "Der Pfad führt über eine Verknüpfung; nichts angefasst.",
            ))
        }
        Urteil::NurOnline => return Err(fehler_nur_online()),
        Urteil::Lesbar => {}
    }
    if !r.ist_ordner() {
        return Err(Fehler::neu("ablage.kein_ordner", "Das ist kein Ordner."));
    }
    let k = fs::canonicalize(pfad).map_err(|_| unerreichbar())?;
    if !gleich(&k, pfad) {
        // Ein Glied weiter oben ist eine Verknüpfung (oder der Pfad ist nicht absolut).
        return Err(Fehler::neu(
            "ablage.verknuepfung",
            "Der Pfad führt über eine Verknüpfung; nichts angefasst.",
        ));
    }
    if k.parent().is_none() {
        return Err(Fehler::neu(
            "ablage.verknuepft_wurzel",
            "Ein ganzes Laufwerk lässt sich nicht verknüpfen.",
        ));
    }
    Ok(k)
}

/// Verknüpft den Ordner `pfad` (Ordnerdialog). Liefert die Kennung (`ord_<ULID>`). Danach einmal
/// [`abtasten`]; das übernimmt der Befehl.
pub fn verknuepfen(a: &Ablage, pfad: &Path) -> Res<String> {
    let vorbereitet = super::speicher::quelle_vorbereiten(pfad)
        .map_err(|code| Fehler::neu(code, "Dieser Pfad lässt sich nicht verknüpfen."))?;
    let k = wurzel_pruefen(&vorbereitet)?;
    let pfad_text = k
        .to_str()
        .ok_or_else(|| Fehler::neu("ablage.quelle_ungueltig", "Der Pfad ist nicht lesbar."))?
        .to_string();
    // Weder im Datenordner der App noch darüber (sonst läse die Ablage sich selbst).
    let daten = a.dir().parent().unwrap_or(a.dir());
    if let Ok(d) = fs::canonicalize(daten) {
        if liegt_in(&k, &d) || liegt_in(&d, &k) {
            return Err(Fehler::neu(
                "ablage.verknuepft_datenordner",
                "Der Datenordner der App lässt sich nicht verknüpfen.",
            ));
        }
    }
    let name = k
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| super::speicher::bereinigen(n).ok())
        .unwrap_or_else(|| "Verknüpfter Ordner".to_string());
    let _ga = super::sperren();
    let _g = sperren();
    let mut l = liste_lesen(a.dir())?;
    for v in &l.ordner {
        let p = PathBuf::from(&v.pfad);
        if liegt_in(&k, &p) || liegt_in(&p, &k) {
            return Err(Fehler::neu(
                "ablage.verknuepft_doppelt",
                "Dieser Ordner ist schon verknüpft (oder liegt in einem verknüpften Ordner).",
            ));
        }
    }
    if l.ordner.len() >= MAX_VERKNUEPFT {
        return Err(Fehler::neu(
            "ablage.zu_viele",
            "Zu viele verknüpfte Ordner.",
        ));
    }
    let id = neue_id(PRAEFIX_ORDNER);
    l.v = VERSION;
    l.ordner.push(Verknuepfung {
        id: id.clone(),
        pfad: pfad_text,
        name,
        klasse: STANDARD_KLASSE.to_string(),
        angelegt: jetzt_ms(),
    });
    liste_schreiben(a.dir(), &l)?;
    Ok(id)
}

/// Löst die Verknüpfung: Liste, Bestand, Textfassungen und Freigabe-Eintrag dieses Geräts verschwinden; der
/// Ordner selbst und seine Dateien bleiben unberührt. Den Index gleicht der Befehl danach ab.
pub fn loesen(a: &Ablage, id: &str) -> Res<()> {
    super::ordner_id_pruefen(id)?;
    let _ga = super::sperren();
    let _g = sperren();
    let mut l = liste_lesen(a.dir())?;
    let Some(pos) = l.ordner.iter().position(|v| v.id == id) else {
        return Err(nicht_gefunden());
    };
    let b = bestand_lesen(a.dir(), id);
    l.ordner.remove(pos);
    liste_schreiben(a.dir(), &l)?;
    for e in b.dateien.values() {
        texte_entfernen(a.dir(), &e.dat);
    }
    if ohne_umweg(a.dir(), &[ORDNER, id]).is_ok() {
        let p = ordner_dir(a.dir()).join(id);
        if let Ok(m) = fs::symlink_metadata(&p) {
            if !ist_umweg(&m) && m.is_dir() {
                let _ = fs::remove_dir_all(&p);
            }
        }
    }
    let _ = freigabe::vergessen(a.dir(), &[id.to_string()]);
    zwischen_raeumen(a.dir());
    Ok(())
}

/// Datenklasse eines verknüpften Ordners (`public`, `internal`, `confidential`, `personal`).
pub fn klasse_setzen(a: &Ablage, id: &str, klasse: &str) -> Res<()> {
    if !KLASSEN.contains(&klasse) {
        return Err(Fehler::ungueltig("Unbekannte Datenklasse."));
    }
    let _g = sperren();
    let mut l = liste_lesen(a.dir())?;
    let v = l
        .ordner
        .iter_mut()
        .find(|v| v.id == id)
        .ok_or_else(nicht_gefunden)?;
    if v.klasse == klasse {
        return Ok(());
    }
    v.klasse = klasse.to_string();
    liste_schreiben(a.dir(), &l)?;
    // Cloud-Ausnahmen gelten nur für `confidential` und verfallen beim Klassenwechsel (wie bei Ordnern).
    freigabe::ausnahmen_entziehen(a.dir(), &[id.to_string()])
}

/// Einstellungen eines verknüpften Ordners: nur die Klasse; „nur dieses Gerät“ gilt immer, Entwürfe der KI
/// landen nie hier.
pub fn einstellungen(a: &Ablage, id: &str, e: &super::speicher::AblageEinstellungen) -> Res<()> {
    if e.nur_geraet == Some(false) || e.zielordner_ki == Some(true) {
        return Err(nur_lesen());
    }
    match e.datenklasse.as_deref() {
        // „Erben“ gibt es oben nicht: zurück auf die Vorgabe.
        Some("erbt") => klasse_setzen(a, id, STANDARD_KLASSE),
        Some(k) => klasse_setzen(a, id, k),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Abtasten
// ---------------------------------------------------------------------------

/// Ausschlüsse (Spec §8): Office-Sperrdateien `~$*`, `*.tmp`, LibreOffice-Sperren, versteckte Einträge (`.*`)
/// und Systemordner/-dateien.
pub fn ausgeschlossen(name: &str, ordner: bool) -> bool {
    let klein = name.to_lowercase();
    if name.starts_with("~$") || name.starts_with('.') || name.is_empty() {
        return true;
    }
    if ordner {
        return matches!(
            klein.as_str(),
            "$recycle.bin"
                | "recycler"
                | "system volume information"
                | "$windows.~bt"
                | "$windows.~ws"
                | "$sysreset"
                | "__macosx"
        );
    }
    klein.ends_with(".tmp")
        || klein.ends_with(".partial")
        || klein.ends_with(".crdownload")
        || matches!(klein.as_str(), "thumbs.db" | "desktop.ini" | "ehthumbs.db")
}

fn zeit_ms(m: &fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Endung als Typ (klein, nur Buchstaben und Ziffern, höchstens 10 Zeichen), sonst leer.
pub fn typ_aus(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stamm, e))
            if !stamm.is_empty()
                && !e.is_empty()
                && e.len() <= 10
                && e.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            e.to_ascii_lowercase()
        }
        _ => String::new(),
    }
}

/// Ergebnis einer Abtastung (noch nicht übernommen).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Abtastung {
    /// Relativer Pfad → (Größe, Änderungszeit, Typ).
    pub dateien: BTreeMap<String, (u64, i64, String)>,
    /// Davon die Dateien, die nur online liegen (nie gelesen, geöffnet oder kopiert).
    pub online: BTreeSet<String>,
    pub ausgelassen: u32,
    /// Unterordner, die nur online liegen und nicht betreten wurden.
    pub nur_online_ordner: u32,
    pub zu_viele: bool,
    /// Unterordner (relativer Pfad), die sich nicht lesen ließen: ihre bekannten Dateien bleiben im Bestand
    /// (nicht lesbar heißt nicht gelöscht).
    pub unlesbar: Vec<String>,
}

impl Abtastung {
    /// Fingerabdruck für die Entprellung (Pfad, Größe, Zeit, Online-Marke, Ausgelassene).
    pub fn fingerabdruck(&self) -> String {
        let mut h = Sha256::new();
        for (rel, (g, z, _)) in &self.dateien {
            h.update(rel.as_bytes());
            h.update([0u8]);
            h.update(g.to_le_bytes());
            h.update(z.to_le_bytes());
            h.update([u8::from(self.online.contains(rel))]);
        }
        h.update(self.ausgelassen.to_le_bytes());
        h.update(self.nur_online_ordner.to_le_bytes());
        h.update([u8::from(self.zu_viele)]);
        extrakt::hex(&h.finalize())
    }
}

impl Bestand {
    fn fingerabdruck(&self) -> String {
        Abtastung {
            dateien: self
                .dateien
                .iter()
                .map(|(r, e)| (r.clone(), (e.groesse, e.zeit, e.typ.clone())))
                .collect(),
            online: self
                .dateien
                .iter()
                .filter(|(_, e)| e.stand == NUR_ONLINE)
                .map(|(r, _)| r.clone())
                .collect(),
            ausgelassen: self.ausgelassen,
            nur_online_ordner: self.nur_online_ordner,
            zu_viele: self.zu_viele,
            unlesbar: Vec::new(),
        }
        .fingerabdruck()
    }
}

fn verzeichnis_lesen(p: &Path) -> std::io::Result<Vec<eintrag::Eintrag>> {
    #[cfg(test)]
    if tests_haken::DIR_FEHLER.with(|d| d.borrow().iter().any(|x| x == p)) {
        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    }
    eintrag::verzeichnis(p)
}

/// Tastet den Baum unter `v.pfad` ab, ohne Verknüpfungen zu folgen und ohne eine Datei zu öffnen. Attribute,
/// Größe und Zeit kommen aus [`eintrag`] (Windows: `FindFirstFileExW`). Dateien, die nur online liegen, werden
/// mit Name, Pfad und Größe aufgenommen (`online`), Ordner, die nur online liegen, nicht betreten. Fehler:
/// Ordner nicht (mehr) erreichbar oder durch eine Verknüpfung ersetzt.
pub fn abtasten_roh(v: &Verknuepfung) -> Res<Abtastung> {
    let wurzel = PathBuf::from(&v.pfad);
    let k = wurzel_pruefen(&wurzel)?;
    if !gleich(&k, &wurzel) {
        return Err(Fehler::neu(
            "ablage.nicht_erreichbar",
            "Der verknüpfte Ordner ist nicht erreichbar.",
        ));
    }
    let mut ab = Abtastung::default();
    let mut stapel: Vec<(PathBuf, String, usize)> = vec![(wurzel, String::new(), 0)];
    while let Some((dir, rel, tiefe)) = stapel.pop() {
        let Ok(inhalt) = verzeichnis_lesen(&dir) else {
            if rel.is_empty() {
                // Die Wurzel selbst ist nicht lesbar: nichts abtasten, der alte Bestand bleibt.
                return Err(Fehler::neu(
                    "ablage.nicht_erreichbar",
                    "Der verknüpfte Ordner ist nicht erreichbar.",
                ));
            }
            ab.ausgelassen += 1;
            ab.unlesbar.push(rel);
            continue;
        };
        for e in inhalt {
            let Some(name) = e.name.to_str().map(str::to_string) else {
                ab.ausgelassen += 1;
                continue;
            };
            let Some(r) = e.roh else {
                ab.ausgelassen += 1;
                continue;
            };
            let urteil = r.urteil();
            if urteil == Urteil::Umweg {
                ab.ausgelassen += 1;
                continue;
            }
            if ausgeschlossen(&name, r.ist_ordner()) || r.versteckt() {
                continue;
            }
            let rel_neu = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if urteil == Urteil::NurOnline {
                if r.ist_ordner() {
                    // Auflisten kann eine Synchronisierung anstoßen: nicht betreten. Bekannte Dateien darin
                    // bleiben im Bestand (nicht lesbar heißt nicht gelöscht).
                    ab.nur_online_ordner += 1;
                    ab.unlesbar.push(rel_neu);
                } else if r.ist_datei() {
                    if ab.dateien.len() >= MAX_DATEIEN {
                        ab.zu_viele = true;
                        return Ok(ab);
                    }
                    ab.online.insert(rel_neu.clone());
                    ab.dateien
                        .insert(rel_neu, (r.groesse, r.zeit, typ_aus(&name)));
                } else {
                    ab.ausgelassen += 1;
                }
                continue;
            }
            if r.ist_ordner() {
                if tiefe + 1 >= MAX_TIEFE {
                    ab.ausgelassen += 1;
                } else {
                    stapel.push((dir.join(&e.name), rel_neu, tiefe + 1));
                }
                continue;
            }
            if !r.ist_datei() || r.nlink > 1 {
                ab.ausgelassen += 1;
                continue;
            }
            if ab.dateien.len() >= MAX_DATEIEN {
                ab.zu_viele = true;
                return Ok(ab);
            }
            ab.dateien
                .insert(rel_neu, (r.groesse, r.zeit, typ_aus(&name)));
        }
    }
    Ok(ab)
}

fn anfangsstand(typ: &str) -> &'static str {
    if extrakt::format_aus_typ(typ).is_some() || ocr::ist_bild(typ) {
        AUSSTEHEND
    } else {
        KEIN_FORMAT
    }
}

/// Übernimmt eine Abtastung in den Bestand: gleiche Dateien bleiben, geänderte werden neu gelesen (Kennung
/// bleibt), neue bekommen eine Kennung, verschwundene fallen mit ihren Textfassungen weg. `true` = geändert.
pub fn uebernehmen(a: &Ablage, id: &str, ab: Option<&Abtastung>) -> Res<bool> {
    let _g = sperren();
    let l = liste_lesen(a.dir())?;
    if !l.ordner.iter().any(|v| v.id == id) {
        return Err(nicht_gefunden());
    }
    let alt = bestand_lesen(a.dir(), id);
    let Some(ab) = ab else {
        if alt.nicht_erreichbar {
            return Ok(false);
        }
        let mut b = alt;
        b.nicht_erreichbar = true;
        bestand_schreiben(a.dir(), id, &b)?;
        return Ok(true);
    };
    let mut neu = Bestand {
        v: VERSION,
        dateien: BTreeMap::new(),
        ausgelassen: ab.ausgelassen,
        nur_online_ordner: ab.nur_online_ordner,
        zu_viele: ab.zu_viele,
        nicht_erreichbar: false,
        abgetastet: jetzt_ms(),
    };
    // Dateien, deren Online-Zustand sich geändert hat: Rückstellungen dieses Prozesses entfallen (eine
    // inzwischen hydrierte Datei wird nicht erst nach einem Neustart gelesen), Textfassungen einer Datei,
    // die wieder nur online liegt, entfallen (kein Text, kein Index).
    let mut gewechselt: Vec<&str> = Vec::new();
    for (rel, (groesse, zeit, typ)) in &ab.dateien {
        let online = ab.online.contains(rel);
        let stand = || -> String {
            if online {
                NUR_ONLINE.into()
            } else {
                anfangsstand(typ).into()
            }
        };
        let e = match alt.dateien.get(rel) {
            Some(e)
                if e.groesse == *groesse
                    && e.zeit == *zeit
                    && &e.typ == typ
                    && (e.stand == NUR_ONLINE) == online =>
            {
                e.clone()
            }
            Some(e) => {
                if (e.stand == NUR_ONLINE) != online {
                    gewechselt.push(e.dat.as_str());
                }
                Eintrag {
                    dat: e.dat.clone(),
                    groesse: *groesse,
                    zeit: *zeit,
                    typ: typ.clone(),
                    sha256: None,
                    textrev: None,
                    stand: stand(),
                    code: None,
                    versuche: 0,
                    ocr: None,
                }
            }
            None => Eintrag {
                dat: neue_id(PRAEFIX_DATEI),
                groesse: *groesse,
                zeit: *zeit,
                typ: typ.clone(),
                sha256: None,
                textrev: None,
                stand: stand(),
                code: None,
                versuche: 0,
                ocr: None,
            },
        };
        neu.dateien.insert(rel.clone(), e);
    }
    if !gewechselt.is_empty() {
        if let Ok(mut z) = ZURUECK.lock() {
            z.retain(|(d, _, _)| !gewechselt.contains(&d.as_str()));
        }
        for dat in &gewechselt {
            let jetzt_online = neu
                .dateien
                .values()
                .any(|e| e.dat == *dat && e.stand == NUR_ONLINE);
            if jetzt_online {
                texte_entfernen(a.dir(), dat);
            }
        }
    }
    for (rel, e) in &alt.dateien {
        let unlesbar = ab.unlesbar.iter().any(|u| {
            rel.strip_prefix(u.as_str())
                .is_some_and(|r| r.starts_with('/'))
        });
        if unlesbar && !neu.dateien.contains_key(rel) {
            neu.dateien.insert(rel.clone(), e.clone());
        }
    }
    let lebend: BTreeSet<&str> = neu.dateien.values().map(|e| e.dat.as_str()).collect();
    for e in alt.dateien.values() {
        if !lebend.contains(e.dat.as_str()) {
            texte_entfernen(a.dir(), &e.dat);
        }
    }
    let geaendert = alt.dateien != neu.dateien
        || alt.ausgelassen != neu.ausgelassen
        || alt.nur_online_ordner != neu.nur_online_ordner
        || alt.zu_viele != neu.zu_viele
        || alt.nicht_erreichbar;
    // Jede übernommene Abtastung schreibt den Zeitpunkt („zuletzt abgetastet“) mit.
    bestand_schreiben(a.dir(), id, &neu)?;
    Ok(geaendert)
}

/// Tastet einen verknüpften Ordner ab und übernimmt das Ergebnis (`true` = Bestand geändert).
pub fn abtasten(a: &Ablage, id: &str) -> Res<bool> {
    let v = liste(a.dir())?
        .into_iter()
        .find(|v| v.id == id)
        .ok_or_else(nicht_gefunden)?;
    let ab = abtasten_roh(&v).ok();
    uebernehmen(a, id, ab.as_ref())
}

/// Tastet alle verknüpften Ordner ab (`true` = irgendein Bestand geändert).
pub fn alle_abtasten(a: &Ablage) -> Res<bool> {
    let mut geaendert = false;
    for v in liste(a.dir())? {
        geaendert |= abtasten(a, &v.id).unwrap_or(false);
    }
    Ok(geaendert)
}

/// „Neu einlesen“: setzt OCR- und Fehlermarken der Dateien eines (oder aller) verknüpften Ordner zurück, damit
/// die Textfassung (und bei einem Bild-PDF die OCR) noch einmal versucht wird; auch die Rückstellungen dieses
/// Prozesses entfallen. Fertige Textfassungen bleiben.
pub fn marken_zuruecksetzen(a: &Ablage, id: Option<&str>) -> Res<()> {
    let _g = sperren();
    let l = liste_lesen(a.dir())?;
    if id.is_some_and(|i| !l.ordner.iter().any(|v| v.id == i)) {
        return Err(nicht_gefunden());
    }
    let mut dats: BTreeSet<String> = BTreeSet::new();
    for v in l.ordner.iter().filter(|v| id.is_none_or(|i| v.id == i)) {
        let mut b = bestand_lesen(a.dir(), &v.id);
        let mut geaendert = false;
        for e in b.dateien.values_mut() {
            let fehler = e.stand == "fehler";
            let ocr_marke = e.stand == "kein_text" && e.ocr.is_some();
            if !fehler && !ocr_marke {
                continue;
            }
            if fehler || ocr::ist_bild(&e.typ) {
                e.stand = anfangsstand(&e.typ).into();
            }
            e.code = None;
            e.versuche = 0;
            e.ocr = None;
            dats.insert(e.dat.clone());
            geaendert = true;
        }
        if geaendert {
            bestand_schreiben(a.dir(), &v.id, &b)?;
        }
    }
    if let Ok(mut z) = ZURUECK.lock() {
        z.retain(|(d, _, _)| !dats.contains(d));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Textfassungen: Momentkopie, Unterprozess, OCR
// ---------------------------------------------------------------------------

/// Art des Auftrags: gewöhnliche Textfassung (mit OCR-Nachlauf bei einem PDF ohne Text) oder gleich OCR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Art {
    Text,
    Ocr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Auftrag {
    pub ordner: String,
    pub rel: String,
    pub dat: String,
    pub typ: String,
    pub groesse: u64,
    pub zeit: i64,
    /// Verknüpfter Ordner (Wurzel); `quelle` ist `wurzel` plus `rel`.
    pub wurzel: PathBuf,
    pub quelle: PathBuf,
    pub art: Art,
    pub versuche: u32,
}

fn ocr_marke(k: Kennung) -> String {
    format!("{}@{}", k.0, k.1)
}

/// Fehler der OCR, die nicht am Inhalt liegen müssen (Zeitgrenze, Speichergrenze des Unterprozesses): sie
/// werden wie die anderen vorübergehenden Fehler höchstens [`super::speicher::MAX_VERSUCHE`]-mal versucht,
/// statt die Datei sofort endgültig als „nicht lesbar“ zu markieren.
const OCR_VORUEBERGEHEND: &[&str] = &["extrakt.zeit", "extrakt.abbruch"];

fn ocr_vorlaeufig_code(code: Option<&str>) -> bool {
    code.is_some_and(|c| OCR_VORUEBERGEHEND.contains(&c))
}

/// Die OCR dieser Datei ist vorübergehend gescheitert und hat noch Versuche übrig.
fn ocr_wiederholbar(e: &Eintrag) -> bool {
    e.ocr.is_some()
        && ocr_vorlaeufig_code(e.code.as_deref())
        && e.versuche < super::speicher::MAX_VERSUCHE
}

fn wiederholbar(e: &Eintrag) -> bool {
    if e.stand != "fehler" || e.versuche >= super::speicher::MAX_VERSUCHE {
        return false;
    }
    e.code
        .as_deref()
        .is_some_and(|c| super::speicher::VORUEBERGEHEND.contains(&c) || c == "verknuepft.lesen")
        || ocr_wiederholbar(e)
}

fn auftrag_art(e: &Eintrag, ocr: Option<Kennung>) -> Option<Art> {
    let bild = ocr::ist_bild(&e.typ);
    let schon_ocr = ocr.is_some_and(|k| e.ocr.as_deref() == Some(ocr_marke(k).as_str()));
    match e.stand.as_str() {
        AUSSTEHEND if bild => ocr.map(|_| Art::Ocr),
        AUSSTEHEND => Some(Art::Text),
        // Bild-PDF: Text war leer, die OCR dieser Plattform hat es noch nicht gelesen.
        "kein_text" if e.typ == "pdf" && ocr.is_some() && (!schon_ocr || ocr_wiederholbar(e)) => {
            Some(Art::Ocr)
        }
        "fehler" if wiederholbar(e) => Some(if bild { Art::Ocr } else { Art::Text }),
        _ => None,
    }
}

/// Offene Aufträge aller erreichbaren verknüpften Ordner.
pub fn auftraege(a: &Ablage, ocr: Option<Kennung>) -> Res<Vec<Auftrag>> {
    let _g = sperren();
    let l = liste_lesen(a.dir())?;
    let mut aus = Vec::new();
    for v in &l.ordner {
        let b = bestand_lesen(a.dir(), &v.id);
        if b.nicht_erreichbar {
            continue;
        }
        let wurzel = PathBuf::from(&v.pfad);
        for (rel, e) in &b.dateien {
            let Some(art) = auftrag_art(e, ocr) else {
                continue;
            };
            // Ein Bestand mit `..`, leeren Gliedern, Laufwerk oder absolutem Pfad ist nicht vertrauenswürdig.
            let Some(quelle) = quellpfad(&wurzel, rel) else {
                continue;
            };
            aus.push(Auftrag {
                wurzel: wurzel.clone(),
                ordner: v.id.clone(),
                rel: rel.clone(),
                dat: e.dat.clone(),
                typ: e.typ.clone(),
                groesse: e.groesse,
                zeit: e.zeit,
                quelle,
                art,
                versuche: e.versuche,
            });
        }
    }
    Ok(aus)
}

/// Öffnet eine Datei ohne Folgen einer Verknüpfung (Unix `O_NOFOLLOW`, Windows `FILE_FLAG_OPEN_REPARSE_POINT`),
/// Löschen und Umbenennen der Quelle bleiben möglich. Zweistufig ([`eintrag::oeffnen_ohne_abruf`]): erst
/// ein Handle nur mit `FILE_READ_ATTRIBUTES` und das Urteil daran; nur bei „lesbar“ folgt das Öffnen mit
/// Lesezugriff, dann wieder Urteil und Datei-Kennung. Das Öffnen mit Lesezugriff löst bei einem Platzhalter
/// einen Abruf aus, der Schutz ist also das Urteil davor (Restrisiko: Auslagern im Fenster zwischen den
/// Stufen, Millisekunden). Die Fehler tragen den Code, den der Aufrufer meldet.
fn oeffnen_ohne_folgen(p: &Path) -> Result<fs::File, &'static str> {
    use eintrag::OeffnenFehler;
    let art = eintrag::OeffnenArt {
        folgen: false,
        teilen: 0x1 | 0x2 | 0x4,
    };
    eintrag::oeffnen_ohne_abruf(p, art, eintrag::reparse_urteil).map_err(|e| match e {
        OeffnenFehler::Urteil(Urteil::NurOnline) => "verknuepft.cloud",
        OeffnenFehler::Urteil(_) => "ablage.verknuepfung",
        OeffnenFehler::Getauscht => "verknuepft.geaendert",
        OeffnenFehler::Io(_) => "verknuepft.lesen",
    })
}

fn sha_datei(p: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut puffer = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut puffer)?;
        if n == 0 {
            break;
        }
        h.update(&puffer[..n]);
    }
    Ok(extrakt::hex(&h.finalize()))
}

/// Pfad der Quelle unter `wurzel`: jedes Glied von `rel` (mit `/` getrennt) muss ein gewöhnlicher Name sein
/// (nicht leer, nicht `.` oder `..`, kein Laufwerk oder Präfix, kein absoluter Pfad, kein Trenner).
fn quellpfad(wurzel: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = wurzel.to_path_buf();
    for t in rel.split('/') {
        let mut c = Path::new(t).components();
        match (c.next(), c.next()) {
            (Some(Component::Normal(n)), None) if n == std::ffi::OsStr::new(t) => {}
            _ => return None,
        }
        p.push(t);
    }
    Some(p)
}

/// Prüft die Wurzel erneut und jedes Glied des relativen Pfades (auch die Zwischenverzeichnisse): keine
/// Verknüpfung, nichts, was nur online liegt (`verknuepft.cloud`). Die Attribute kommen aus [`eintrag::roh`]
/// (Windows: `FindFirstFileExW`, ohne zu öffnen). Liefert den Pfad der Quelle.
fn quelle_pruefen(wurzel: &Path, rel: &str) -> Result<PathBuf, &'static str> {
    let ziel = quellpfad(wurzel, rel).ok_or("ablage.verknuepfung")?;
    wurzel_pruefen(wurzel).map_err(|f| match f.code.as_str() {
        "ablage.verknuepfung" => "ablage.verknuepfung",
        "ablage.nur_online" => "verknuepft.cloud",
        _ => "verknuepft.lesen",
    })?;
    let mut p = wurzel.to_path_buf();
    for t in rel.split('/') {
        p.push(t);
        let r = eintrag::roh(&p).map_err(|_| "verknuepft.lesen")?;
        match r.urteil() {
            Urteil::Umweg => return Err("ablage.verknuepfung"),
            Urteil::NurOnline => return Err("verknuepft.cloud"),
            Urteil::Lesbar => {}
        }
    }
    Ok(ziel)
}

/// Momentkopien, die gerade in Arbeit sind (nicht wegräumen); die Sperre schützt Anlegen und Räumen.
static KOPIEN: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

fn kopie_abmelden(p: &Path) {
    if let Ok(mut k) = KOPIEN.lock() {
        k.remove(p);
    }
}

/// Entfernt liegengebliebene Momentkopien (Klartext vertraulicher Dateien nach einem Absturz) aus
/// `verknuepft/zwischen/`; Kopien eines laufenden Auftrags bleiben.
pub fn zwischen_raeumen(dir: &Path) {
    if ohne_umweg(dir, &[ORDNER, ZWISCHEN]).is_err() {
        return;
    }
    let Ok(in_arbeit) = KOPIEN.lock() else {
        return;
    };
    let Ok(inhalt) = fs::read_dir(ordner_dir(dir).join(ZWISCHEN)) else {
        return;
    };
    for e in inhalt.flatten() {
        let p = e.path();
        if in_arbeit.contains(&p) {
            continue;
        }
        if fs::symlink_metadata(&p).is_ok_and(|m| !m.is_dir()) {
            let _ = fs::remove_file(&p);
        }
    }
}

/// Geprüfte Momentkopie (Spec §8): Wurzel und alle Glieder des Pfades ohne Verknüpfung und nicht nur online
/// (ohne zu öffnen), Quelle ohne Folgen von Verknüpfungen öffnen, am Handle erneut auf „nur online“
/// prüfen (dann schließen, ohne zu lesen), kein Hartlink, Größe und Änderungszeit am offenen Handle wie beim
/// Abtasten;
/// Kopie mit `create_new` unter `verknuepft/zwischen/`, beim Kopieren gehasht, Quelle danach unverändert,
/// Kopie erneut gehasht. Liefert Kopie und Prüfsumme; Fehler als Code.
pub fn momentkopie(a: &Ablage, auf: &Auftrag) -> Result<(PathBuf, String), &'static str> {
    let quelle = quelle_pruefen(&auf.wurzel, &auf.rel)?;
    // Vor dem Öffnen: Attribute der Quelle ohne zu öffnen (Windows: `FindFirstFileExW`).
    let vor = eintrag::roh(&quelle).map_err(|_| "verknuepft.lesen")?;
    match vor.urteil() {
        Urteil::Lesbar => {}
        Urteil::NurOnline => return Err("verknuepft.cloud"),
        Urteil::Umweg => return Err("ablage.verknuepfung"),
    }
    if vor.gattung != Gattung::Datei {
        return Err("ablage.verknuepfung");
    }
    if (vor.groesse, vor.zeit) != (auf.groesse, auf.zeit) {
        return Err("verknuepft.geaendert");
    }
    if vor.groesse > extrakt::MAX_EINGABE {
        return Err("extrakt.zu_gross");
    }
    let mut von = oeffnen_ohne_folgen(&quelle)?;
    // Ab hier zählt, was das Handle sagt (nicht mehr der Pfad): „nur online“ und Umwege hat das zweistufige
    // Öffnen schon vor dem Lesezugriff abgewiesen.
    let offen = von.metadata().map_err(|_| "verknuepft.lesen")?;
    if !offen.is_file() {
        return Err("ablage.verknuepfung");
    }
    if (offen.len(), zeit_ms(&offen)) != (auf.groesse, auf.zeit) {
        return Err("verknuepft.geaendert");
    }
    if datei_hartverknuepft(&von) {
        return Err("ablage.hartlink");
    }
    ohne_umweg(a.dir(), &[ORDNER, ZWISCHEN]).map_err(|_| "ablage.verknuepfung")?;
    let zwischen = ordner_dir(a.dir()).join(ZWISCHEN);
    fs::create_dir_all(&zwischen).map_err(|_| "verknuepft.speicher")?;
    ohne_umweg(a.dir(), &[ORDNER, ZWISCHEN]).map_err(|_| "ablage.verknuepfung")?;
    let endung = if auf.typ.is_empty() { "bin" } else { &auf.typ };
    let kopie = zwischen.join(format!("{}.{endung}", ulid::Ulid::new()));
    let mut nach = {
        let mut in_arbeit = KOPIEN.lock().map_err(|_| "verknuepft.speicher")?;
        let f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&kopie)
            .map_err(|_| "verknuepft.speicher")?;
        in_arbeit.insert(kopie.clone());
        f
    };
    let (quelle_r, kopie_r) = (&quelle, &kopie);
    let ergebnis = (move || -> Result<String, &'static str> {
        let mut h = Sha256::new();
        let mut puffer = vec![0u8; 1024 * 1024];
        let mut n_ges: u64 = 0;
        loop {
            let n = von.read(&mut puffer).map_err(|_| "verknuepft.lesen")?;
            if n == 0 {
                break;
            }
            n_ges += n as u64;
            if n_ges > extrakt::MAX_EINGABE {
                return Err("extrakt.zu_gross");
            }
            h.update(&puffer[..n]);
            nach.write_all(&puffer[..n])
                .map_err(|_| "verknuepft.speicher")?;
        }
        nach.sync_all().map_err(|_| "verknuepft.speicher")?;
        drop(nach);
        #[cfg(test)]
        tests_haken::nach_kopie(quelle_r);
        let sha = extrakt::hex(&h.finalize());
        let nachher = eintrag::roh(quelle_r).map_err(|_| "verknuepft.geaendert")?;
        let nachher_offen = von.metadata().map_err(|_| "verknuepft.geaendert")?;
        if nachher.urteil() != Urteil::Lesbar
            || eintrag::offen_urteil(&von, quelle_r) != Urteil::Lesbar
            || (nachher.groesse, nachher.zeit) != (auf.groesse, auf.zeit)
            || (nachher_offen.len(), zeit_ms(&nachher_offen)) != (auf.groesse, auf.zeit)
            || n_ges != auf.groesse
        {
            return Err("verknuepft.geaendert");
        }
        if sha_datei(kopie_r).map_err(|_| "verknuepft.speicher")? != sha {
            return Err("ablage.pruefsumme");
        }
        Ok(sha)
    })();
    match ergebnis {
        Ok(sha) => Ok((kopie, sha)),
        Err(c) => {
            let _ = fs::remove_file(&kopie);
            kopie_abmelden(&kopie);
            Err(c)
        }
    }
}

/// Test: Haken nach dem Kopieren (Quelle während der Momentkopie ändern).
#[cfg(test)]
pub(crate) mod tests_haken {
    use std::cell::RefCell;
    use std::path::Path;

    pub type Haken = Option<Box<dyn Fn(&Path)>>;

    thread_local! {
        pub static NACH_KOPIE: RefCell<Haken> = const { RefCell::new(None) };
        /// Ordner, deren `read_dir` im Test scheitert (auch als Administrator).
        pub static DIR_FEHLER: RefCell<Vec<std::path::PathBuf>> = const { RefCell::new(Vec::new()) };
    }

    pub fn nach_kopie(p: &Path) {
        NACH_KOPIE.with(|h| {
            if let Some(f) = h.borrow().as_ref() {
                f(p);
            }
        });
    }
}

/// Ausgang eines Auftrags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ausgang {
    /// Eingetragen mit diesem Stand (`ok`, `kein_text`, `fehler`, `abgewiesen`).
    Eingetragen(String),
    /// Nichts eingetragen; in diesem Prozess nicht erneut versuchen.
    Uebersprungen,
    /// Quelle hat sich geändert: der Beobachter tastet neu ab, nichts eingetragen.
    Geaendert,
    Abgebrochen,
    /// Datei inzwischen weg oder anders: nichts geschrieben.
    Verworfen,
}

/// Ergebnis eines Laufs (Zähler).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Lauf {
    pub ok: u32,
    pub kein_text: u32,
    pub fehler: u32,
    pub abgebrochen: bool,
}

struct Ergebnis {
    stand: String,
    code: Option<String>,
    text: Option<(String, Vec<u8>)>,
    ocr: Option<String>,
}

fn ergebnis_aus(t: extrakt::Textfassung, ocr: Option<String>) -> Ergebnis {
    if t.abschnitte.is_empty() {
        return Ergebnis {
            stand: "kein_text".into(),
            code: None,
            text: None,
            ocr,
        };
    }
    match serde_json::to_vec(&t) {
        Ok(json) => Ergebnis {
            stand: "ok".into(),
            code: None,
            text: Some((t.textrev, json)),
            ocr,
        },
        Err(_) => Ergebnis {
            stand: "fehler".into(),
            code: Some("verknuepft.speicher".into()),
            text: None,
            ocr,
        },
    }
}

/// Ein Auftrag: Momentkopie, Unterprozess (Text, bei einem PDF ohne Text danach OCR), Kopie löschen,
/// Ergebnis eintragen.
pub fn bearbeiten(
    a: &Ablage,
    auf: &Auftrag,
    prog: &Programm,
    grenzen: Grenzen,
    abbrechen: &AtomicBool,
    ocr: Option<Kennung>,
) -> Res<Ausgang> {
    let (kopie, sha) = match momentkopie(a, auf) {
        Ok(x) => x,
        Err("verknuepft.geaendert") => return Ok(Ausgang::Geaendert),
        // Cloud-Platzhalter ohne Daten auf dem Gerät: nie ein Download, in diesem Prozess nicht erneut.
        Err("verknuepft.speicher" | "verknuepft.cloud") => return Ok(Ausgang::Uebersprungen),
        Err(c @ ("ablage.hartlink" | "ablage.verknuepfung")) => {
            return eintragen(
                a,
                auf,
                Ergebnis {
                    stand: ABGEWIESEN.into(),
                    code: Some(c.into()),
                    text: None,
                    ocr: None,
                },
                None,
            )
        }
        Err(c) => {
            return eintragen(
                a,
                auf,
                Ergebnis {
                    stand: "fehler".into(),
                    code: Some(c.into()),
                    text: None,
                    ocr: None,
                },
                None,
            )
        }
    };
    let ocr_grenzen = Grenzen {
        zeit: grenzen.zeit.saturating_mul(3),
        ..grenzen
    };
    let ocr_lauf = |k: Kennung| -> Result<extrakt::Textfassung, String> {
        let t = textlauf::ausfuehren(
            prog,
            a.dir(),
            &kopie,
            &sha,
            &format!("{}{}", ocr::PRAEFIX, auf.typ),
            ocr_grenzen,
            abbrechen,
        )?;
        if t.textrev != ocr::textrev(&sha, k) {
            return Err("extrakt.antwort".into());
        }
        Ok(t)
    };
    let ergebnis = match (auf.art, ocr) {
        (Art::Ocr, Some(k)) => match ocr_lauf(k) {
            Ok(t) => Ok(ergebnis_aus(t, Some(ocr_marke(k)))),
            Err(c) => Err(c),
        },
        (Art::Ocr, None) => Err("extrakt.ocr_nicht_verfuegbar".to_string()),
        (Art::Text, _) => {
            match textlauf::ausfuehren(prog, a.dir(), &kopie, &sha, &auf.typ, grenzen, abbrechen) {
                Ok(t) if t.abschnitte.is_empty() && auf.typ == "pdf" && ocr.is_some() => {
                    // Bild-PDF: gleich mit derselben Momentkopie die OCR.
                    let k = ocr.unwrap_or(("", ""));
                    match ocr_lauf(k) {
                        Ok(o) => Ok(ergebnis_aus(o, Some(ocr_marke(k)))),
                        Err(c) if c == "extrakt.abgebrochen" || c == "extrakt.start" => Err(c),
                        Err(c) => Ok(Ergebnis {
                            stand: "kein_text".into(),
                            code: Some(c),
                            text: None,
                            ocr: Some(ocr_marke(k)),
                        }),
                    }
                }
                Ok(t) => Ok(ergebnis_aus(t, None)),
                Err(c) => Err(c),
            }
        }
    };
    let _ = fs::remove_file(&kopie);
    kopie_abmelden(&kopie);
    let ergebnis = match ergebnis {
        Ok(e) => e,
        Err(c) if c == "extrakt.abgebrochen" => return Ok(Ausgang::Abgebrochen),
        Err(c) if c == "extrakt.start" => return Ok(Ausgang::Uebersprungen),
        Err(c) => Ergebnis {
            // Ein Bild-PDF, dessen OCR scheitert, behält seinen Text-Stand `kein_text`.
            stand: if auf.art == Art::Ocr && auf.typ == "pdf" {
                "kein_text".into()
            } else {
                "fehler".into()
            },
            code: Some(c.chars().take(40).collect()),
            text: None,
            ocr: (auf.art == Art::Ocr).then(|| ocr.map(ocr_marke)).flatten(),
        },
    };
    // Vorübergehende OCR-Fehler tragen einen Versuch ein, bleiben aber offen; in diesem Prozess nicht erneut.
    let vorlaeufig = ergebnis.stand == "kein_text"
        && ergebnis.ocr.is_some()
        && ocr_vorlaeufig_code(ergebnis.code.as_deref());
    let r = eintragen(a, auf, ergebnis, Some(sha));
    if vorlaeufig {
        zurueckstellen(auf);
    }
    r
}

fn eintragen(a: &Ablage, auf: &Auftrag, erg: Ergebnis, sha: Option<String>) -> Res<Ausgang> {
    let _g = sperren();
    if !liste_lesen(a.dir())?
        .ordner
        .iter()
        .any(|v| v.id == auf.ordner)
    {
        return Ok(Ausgang::Verworfen);
    }
    let mut b = bestand_lesen(a.dir(), &auf.ordner);
    let Some(e) = b.dateien.get_mut(&auf.rel) else {
        return Ok(Ausgang::Verworfen);
    };
    if e.dat != auf.dat || e.groesse != auf.groesse || e.zeit != auf.zeit {
        return Ok(Ausgang::Verworfen);
    }
    let mut textrev = None;
    if let Some((rev, json)) = &erg.text {
        if rev.len() != 64 || !rev.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(Ausgang::Uebersprungen);
        }
        let geschrieben = (|| -> Res<()> {
            ohne_umweg(a.dir(), &[ORDNER, TEXT, &auf.dat])?;
            fs::create_dir_all(ordner_dir(a.dir()).join(TEXT).join(&auf.dat))
                .map_err(|_| Fehler::speicher("Textordner nicht anlegbar."))?;
            let datei = format!("{rev}.json");
            ohne_umweg(a.dir(), &[ORDNER, TEXT, &auf.dat, &datei])?;
            schreibe_atomar(&text_pfad(a.dir(), &auf.dat, rev), json)
        })();
        if geschrieben.is_err() {
            return Ok(Ausgang::Uebersprungen);
        }
        textrev = Some(rev.clone());
    }
    // Ältere Textfassungen dieser Datei fallen weg (die Quelle hat genau einen Stand).
    let tdir = ordner_dir(a.dir()).join(TEXT).join(&auf.dat);
    if ohne_umweg(a.dir(), &[ORDNER, TEXT, &auf.dat]).is_ok() {
        if let Ok(rd) = fs::read_dir(&tdir) {
            for x in rd.flatten() {
                let n = x.file_name().to_string_lossy().into_owned();
                if textrev.as_ref().is_none_or(|r| n != format!("{r}.json")) {
                    let _ = fs::remove_file(x.path());
                }
            }
        }
        if textrev.is_none() {
            let _ = fs::remove_dir(&tdir);
        }
        ordner_sync(&tdir);
    }
    let ocr_vorlaeufig =
        erg.stand == "kein_text" && erg.ocr.is_some() && ocr_vorlaeufig_code(erg.code.as_deref());
    let versuche = if erg.stand == "fehler" || ocr_vorlaeufig {
        auf.versuche.saturating_add(1)
    } else {
        0
    };
    e.sha256 = sha.or(e.sha256.take());
    e.textrev = textrev;
    e.stand = erg.stand.clone();
    e.code = erg.code;
    e.versuche = versuche;
    if erg.ocr.is_some() {
        e.ocr = erg.ocr;
    }
    bestand_schreiben(a.dir(), &auf.ordner, &b)?;
    Ok(Ausgang::Eingetragen(erg.stand))
}

/// In diesem Prozess nicht erneut versuchen: `(dat, groesse, zeit)`.
static ZURUECK: Mutex<BTreeSet<(String, u64, i64)>> = Mutex::new(BTreeSet::new());

fn zurueckstellen(auf: &Auftrag) {
    if let Ok(mut z) = ZURUECK.lock() {
        z.insert((auf.dat.clone(), auf.groesse, auf.zeit));
    }
}

fn zurueckgestellt(auf: &Auftrag) -> bool {
    ZURUECK
        .lock()
        .map(|z| z.contains(&(auf.dat.clone(), auf.groesse, auf.zeit)))
        .unwrap_or(false)
}

/// Gibt es Aufträge, die dieser Prozess noch bearbeiten würde?
pub fn offen(a: &Ablage, ocr: Option<Kennung>) -> bool {
    auftraege(a, ocr)
        .map(|v| v.iter().any(|x| !zurueckgestellt(x)))
        .unwrap_or(false)
}

/// Arbeitet die offenen Aufträge der verknüpften Ordner einmal ab (die Warteschlange der Ablage ruft das
/// nach den eigenen Aufträgen). `melden` nach jedem eingetragenen Ergebnis.
pub fn lauf(
    a: &Ablage,
    prog: &Programm,
    grenzen: Grenzen,
    abbrechen: &AtomicBool,
    melden: &dyn Fn(),
    ocr: Option<Kennung>,
) -> Res<Lauf> {
    let mut l = Lauf::default();
    let Ok(auftraege) = auftraege(a, ocr) else {
        return Ok(l);
    };
    for auf in auftraege {
        if zurueckgestellt(&auf) {
            continue;
        }
        if abbrechen.load(Ordering::Relaxed) {
            l.abgebrochen = true;
            break;
        }
        match bearbeiten(a, &auf, prog, grenzen, abbrechen, ocr)? {
            Ausgang::Eingetragen(s) => {
                match s.as_str() {
                    "ok" => l.ok += 1,
                    "kein_text" => l.kein_text += 1,
                    _ => {
                        l.fehler += 1;
                        zurueckstellen(&auf);
                    }
                }
                let _ = super::index::abgleichen(a);
                melden();
            }
            Ausgang::Uebersprungen => {
                l.fehler += 1;
                zurueckstellen(&auf);
            }
            Ausgang::Geaendert | Ausgang::Verworfen => {}
            Ausgang::Abgebrochen => {
                l.abgebrochen = true;
                break;
            }
        }
    }
    Ok(l)
}

/// Test: Rückstellungen vergessen (neuer Prozess).
#[cfg(test)]
pub fn rueckstellungen_vergessen() {
    if let Ok(mut z) = ZURUECK.lock() {
        z.clear();
    }
}

// ---------------------------------------------------------------------------
// Index, Suche, Sicht
// ---------------------------------------------------------------------------

fn ki_zustand(lage: &freigabe::Lage, id: &str) -> (String, bool) {
    let w = freigabe::wirksam(lage, &[id.to_string()]);
    let z = if w.unklar {
        "unklar"
    } else {
        match w.stufe {
            KiStufe::An => "an",
            KiStufe::Aus => "aus",
            KiStufe::Gesperrt => "gesperrt",
        }
    };
    (z.to_string(), w.vorgabe)
}

fn klasse_wirksam(v: &Verknuepfung) -> &str {
    if KLASSEN.contains(&v.klasse.as_str()) {
        &v.klasse
    } else {
        "unbekannt"
    }
}

/// Quellen des Index aus allen verknüpften Ordnern. Der Aufrufer hält die Sperre der Ablage nicht.
pub fn index_quellen(a: &Ablage) -> Res<Vec<IndexQuelle>> {
    let lage = {
        let _ga = super::sperren();
        freigabe::laden(a.dir())
    };
    let _g = sperren();
    let Ok(l) = liste_lesen(a.dir()) else {
        // Defekte Liste: nichts aus verknüpften Ordnern im Index (fail-closed).
        return Ok(vec![]);
    };
    let mut aus = Vec::new();
    for v in &l.ordner {
        let (ki, _) = ki_zustand(&lage, &v.id);
        let b = bestand_lesen(a.dir(), &v.id);
        for e in b.dateien.values() {
            // Hartlinks und Dateien, die nur online liegen: nie im Index.
            if e.stand == ABGEWIESEN || e.stand == NUR_ONLINE {
                continue;
            }
            aus.push(IndexQuelle {
                dat: e.dat.clone(),
                fas: e.sha256.clone().unwrap_or_else(|| "-".into()),
                textrev: e.textrev.clone().filter(|_| e.stand == "ok"),
                stand: e.stand.clone(),
                ordnerkette: vec![v.id.clone()],
                klasse: klasse_wirksam(v).to_string(),
                ki: ki.clone(),
                nur_geraet: true,
                papierkorb: false,
                verknuepft: true,
            });
        }
    }
    Ok(aus)
}

/// Name (relativer Pfad), Ordner und Typ für die Trefferliste.
pub fn treffer_infos(a: &Ablage, dats: &[String]) -> BTreeMap<String, TrefferInfo> {
    let gesucht: BTreeSet<&str> = dats.iter().map(String::as_str).collect();
    let _g = sperren();
    let Ok(l) = liste_lesen(a.dir()) else {
        return BTreeMap::new();
    };
    let mut aus = BTreeMap::new();
    for v in &l.ordner {
        for (rel, e) in bestand_lesen(a.dir(), &v.id).dateien {
            if gesucht.contains(e.dat.as_str()) && e.stand != ABGEWIESEN && e.stand != NUR_ONLINE {
                aus.insert(
                    e.dat.clone(),
                    TrefferInfo {
                        name: rel,
                        ordner: v.id.clone(),
                        typ: e.typ.clone(),
                    },
                );
            }
        }
    }
    aus
}

/// Ein verknüpfter Ordner für die Oberfläche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageVerknuepft {
    pub ordner: AblageOrdner,
    pub erreichbar: bool,
    /// Übersprungene Einträge (Verknüpfungen, Hartlinks, zu tief).
    pub ausgelassen: u32,
    /// Dateien und Ordner, die nur online liegen (OneDrive „Dateien bei Bedarf“): die Dateien stehen mit Name
    /// und Größe in der Liste, werden aber nie gelesen; Ordner werden nicht betreten.
    pub nur_online: u32,
    /// Mehr als [`MAX_DATEIEN`] Dateien: nur die ersten sind erfasst.
    pub zu_viele: bool,
    /// Letztes Abtasten (ms), 0 = noch nie.
    pub abgetastet: i64,
}

fn textstand(e: &Eintrag, ocr: Option<Kennung>) -> Option<String> {
    match e.stand.as_str() {
        KEIN_FORMAT => None,
        AUSSTEHEND if ocr::ist_bild(&e.typ) && ocr.is_none() => None,
        s => Some(s.to_string()),
    }
}

/// Sicht der verknüpften Ordner und ihrer Dateien. Der Aufrufer darf die Sperre der Ablage halten.
pub fn sicht(a: &Ablage, lage: &freigabe::Lage) -> (Vec<AblageVerknuepft>, Vec<AblageDatei>) {
    let ocr = ocr::extraktor();
    let _g = sperren();
    let Ok(l) = liste_lesen(a.dir()) else {
        return (vec![], vec![]);
    };
    let mut ordner = Vec::new();
    let mut dateien = Vec::new();
    for v in &l.ordner {
        let b = bestand_lesen(a.dir(), &v.id);
        let (zustand, vorgabe) = ki_zustand(lage, &v.id);
        let ki = AblageKi {
            zustand,
            von: None,
            von_name: None,
            vorgabe,
        };
        let klasse = klasse_wirksam(v).to_string();
        let mut abgewiesen = 0u32;
        let mut anzahl = 0u32;
        let mut nur_online = b.nur_online_ordner;
        for (rel, e) in &b.dateien {
            if e.stand == ABGEWIESEN {
                abgewiesen += 1;
                continue;
            }
            if e.stand == NUR_ONLINE {
                nur_online += 1;
            }
            anzahl += 1;
            dateien.push(AblageDatei {
                id: e.dat.clone(),
                name: rel.clone(),
                ordner: v.id.clone(),
                typ: e.typ.clone(),
                groesse: e.groesse,
                geaendert: e.zeit,
                fassungen: 1,
                konflikt: false,
                klasse: klasse.clone(),
                ki: ki.clone(),
                nur_geraet: true,
                text: textstand(e, ocr),
                text_code: e.code.clone(),
                verknuepft: true,
            });
        }
        ordner.push(AblageVerknuepft {
            ordner: AblageOrdner {
                id: v.id.clone(),
                name: v.name.clone(),
                eltern: None,
                wiederhergestellt: false,
                datenklasse: v.klasse.clone(),
                klasse,
                nur_geraet: true,
                nur_geraet_wirksam: true,
                zielordner_ki: false,
                ki,
                dateien: anzahl,
                verknuepft: true,
            },
            erreichbar: !b.nicht_erreichbar,
            ausgelassen: b.ausgelassen.saturating_add(abgewiesen),
            nur_online,
            zu_viele: b.zu_viele,
            abgetastet: b.abgetastet,
        });
    }
    dateien.sort_by_key(|d| crate::library_layout::vergleichsschluessel(&d.name));
    (ordner, dateien)
}

/// Kennungen aller verknüpften Ordner (für „Freigaben neu festlegen“).
pub fn kennungen(dir: &Path) -> Vec<String> {
    liste(dir)
        .map(|l| l.into_iter().map(|v| v.id).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Dateibeobachter mit Entprellung
// ---------------------------------------------------------------------------

/// Entprellung je Ordner: eine Änderung des Fingerabdrucks wird erst übernommen, wenn sie mindestens
/// `ruhe_ms` lang (über mindestens zwei Abtastungen) gleich bleibt. Rein, ohne Uhr: die Zeit kommt herein.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entpreller {
    bekannt: Option<String>,
    kandidat: Option<(String, i64)>,
    ruhe_ms: i64,
    /// Zeitpunkt, an dem „zuletzt abgetastet“ im Bestand zuletzt geschrieben wurde.
    gestempelt: i64,
}

impl Entpreller {
    pub fn neu(bekannt: Option<String>, ruhe: Duration) -> Self {
        Self {
            bekannt,
            kandidat: None,
            ruhe_ms: ruhe.as_millis() as i64,
            gestempelt: 0,
        }
    }

    /// Meldet eine Abtastung; `true` = jetzt übernehmen.
    pub fn melden(&mut self, fp: &str, jetzt: i64) -> bool {
        if self.bekannt.as_deref() == Some(fp) {
            self.kandidat = None;
            return false;
        }
        match &self.kandidat {
            Some((k, seit)) if k == fp => {
                if jetzt - seit >= self.ruhe_ms {
                    self.bekannt = Some(fp.to_string());
                    self.kandidat = None;
                    true
                } else {
                    false
                }
            }
            _ => {
                self.kandidat = Some((fp.to_string(), jetzt));
                false
            }
        }
    }

    /// Wartet eine Änderung auf ihre Ruhezeit?
    pub fn wartet(&self) -> bool {
        self.kandidat.is_some()
    }
}

/// Ein Takt des Beobachters über alle verknüpften Ordner: liefert, ob ein Bestand übernommen wurde, und ob
/// eine Änderung noch auf ihre Ruhezeit wartet.
pub fn beobachten(
    a: &Ablage,
    entpreller: &mut BTreeMap<String, Entpreller>,
    jetzt: i64,
    ruhe: Duration,
) -> (bool, bool) {
    let Ok(l) = liste(a.dir()) else {
        return (false, false);
    };
    entpreller.retain(|id, _| l.iter().any(|v| &v.id == id));
    let mut uebernommen = false;
    let mut wartet = false;
    for v in &l {
        let e = entpreller.entry(v.id.clone()).or_insert_with(|| {
            let b = bestand(a.dir(), &v.id);
            let fp = (b.abgetastet != 0 && !b.nicht_erreichbar).then(|| b.fingerabdruck());
            Entpreller::neu(fp, ruhe)
        });
        match abtasten_roh(v) {
            Ok(ab) => {
                if e.melden(&ab.fingerabdruck(), jetzt) {
                    uebernommen |= uebernehmen(a, &v.id, Some(&ab)).unwrap_or(false);
                    e.gestempelt = jetzt;
                } else if !e.wartet() && jetzt - e.gestempelt >= STEMPEL_MS {
                    // Unverändert: nur „zuletzt abgetastet“ nachführen (nicht bei jedem Takt schreiben).
                    let _ = uebernehmen(a, &v.id, Some(&ab));
                    e.gestempelt = jetzt;
                }
                wartet |= e.wartet();
            }
            Err(_) => {
                // Nicht erreichbar: sofort vermerken (kein Lesen möglich), der Fingerabdruck gilt als neu.
                e.bekannt = None;
                uebernommen |= uebernehmen(a, &v.id, None).unwrap_or(false);
            }
        }
    }
    (uebernommen, wartet)
}

/// Startet den Dateibeobachter für die Ablage unter `dir` (einmal je Prozess; weitere Aufrufe ersetzen nur
/// den Rückruf). `geaendert` läuft nach jeder Übernahme (Index ist dann abgeglichen).
pub fn beobachter_starten(dir: PathBuf, geaendert: Box<dyn Fn() + Send>) {
    static RUECKRUF: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);
    static GESTARTET: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if let Ok(mut r) = RUECKRUF.lock() {
        *r = Some(geaendert);
    }
    GESTARTET.get_or_init(|| {
        let a = Ablage::neu(dir);
        let gestartet = std::thread::Builder::new()
            .name("ablage-verknuepft".into())
            .spawn(move || {
                let mut entpreller = BTreeMap::new();
                zwischen_raeumen(a.dir());
                loop {
                    let (uebernommen, wartet) = beobachten(&a, &mut entpreller, jetzt_ms(), RUHE);
                    if uebernommen {
                        if let Err(e) = super::index::abgleichen(&a) {
                            log::warn!("ablage: Indexabgleich (verknüpft): {}", e.code);
                        }
                        super::textfassung::wiederanlauf(a.dir().to_path_buf());
                        if let Ok(r) = RUECKRUF.lock() {
                            if let Some(f) = r.as_ref() {
                                f();
                            }
                        }
                    }
                    std::thread::sleep(if wartet { RUHE } else { TAKT });
                }
            });
        if gestartet.is_err() {
            log::warn!("ablage: Beobachter verknüpfter Ordner nicht gestartet");
        }
    });
}
