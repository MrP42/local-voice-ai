//! Antwortregister des Versandtors (lva#365 P4c-1, Entscheidung E2).
//!
//! Der Verlauf eines Projektgesprächs kommt bei jedem Turn als Text vom Frontend. Der Kern sieht ihm nicht
//! an, aus welchem Projektinhalt eine frühere Antwort stammt. Darum trägt das Tor jede **freigegebene**
//! Antwort mit ihrer Herkunft ins Register ein und schlägt sie beim nächsten Turn wieder nach: Eine Antwort
//! bleibt nur im Verlauf, wenn alle ihre Quellen noch gelten. Unbekannte Antworten gelten als entzogen.
//!
//! Datei `projekte/ki-herkunft.json` (gerätelokal, unter Unix mit Rechten 0600):
//! - Je Antwort ein gesalzenes Merkmal (`HMAC-SHA256(salz, normalisierter Text)`), nie Klartext, Titel
//!   oder Namen. Das Salz gilt je Datei und kommt vom Zufallsgenerator des Betriebssystems.
//! - Schreiben atomar (eigene Temp-Datei, fsync, Umbenennen) und unter einer Prozess-Sperre. Ein Fehler
//!   beim Schreiben lässt die bestehende Datei unverändert.
//! - Fehlt die Datei, ist das Register leer. Ist ihr **Inhalt** defekt (kaputt, neuere Version), wird
//!   sie als `ki-herkunft.json.defekt-<ms>` beiseitegelegt (nie gelöscht) und [`Lage::Defekt`] gemeldet;
//!   das Tor setzt dann den Verlauf zurück. Alle alten Antworten sind danach unbekannt.
//! - Ein **Lesefehler** (außer „Datei fehlt“: Zugriff verweigert, Datenträger, Pfad ist ein Ordner) ist
//!   etwas anderes: Die Datei kann in Ordnung sein. Dann wird nichts umbenannt, `laden` und `eintragen`
//!   scheitern, und das Tor bricht den Turn ab (fail-closed, das Register bleibt, wie es ist).
//! - Trägt dieselbe Antwort ein zweites Mal ein, wird ihre Herkunft **vereinigt**, nie ersetzt: Sie gilt
//!   nur, solange alle Quellen aus beiden Läufen gelten.
//! - Der Bestand ist begrenzt ([`MAX_EINTRAEGE`]); die ältesten Einträge fallen zuerst heraus. Das ist
//!   sicher: Was nicht im Register steht, gilt als entzogen.
//! - Alle Aufrufe blockieren (Datei, Sperre); das Tor ruft sie über `spawn_blocking`.

use crate::ablage::zusammenfuehren::Klasse as AblageKlasse;
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use wai_projekte::Klasse;

/// Dateiname im Projektordner der App.
pub const DATEI: &str = "ki-herkunft.json";
const VERSION: u32 = 1;
/// Höchstzahl der Einträge; darüber fallen die ältesten heraus (Startwert, nicht gemessen).
pub const MAX_EINTRAEGE: usize = 4096;

static SPERRE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn sperren() -> std::sync::MutexGuard<'static, ()> {
    SPERRE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Herkunft einer freigegebenen Antwort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eintrag {
    pub gespraech: String,
    pub merkmal: String,
    pub projekt: String,
    pub stand: String,
    /// Marken `eintrag:<id>#<h32>` der gelieferten Projektinhalte, vereinigt mit denen der im Verlauf
    /// behaltenen Antworten, auf die sich die Antwort stützt.
    pub inhalte: Vec<String>,
    /// Marken `dokument:<dat>@<fas>#<textrev>` der gelieferten Ablage-Auszüge, vereinigt wie `inhalte`.
    pub ablage: Vec<String>,
    /// Höchste wirksame Klasse der gelieferten Projektinhalte (mit Projektklasse und Art.-9-Kennzeichen).
    pub klasse_projekt: Klasse,
    /// Höchste Klasse der gelieferten Ablage-Dateien.
    #[serde(default)]
    pub klasse_ablage: Option<AblageKlasse>,
    pub zeit: i64,
}

/// Angaben zu einer neu freigegebenen Antwort. Besitzend, damit der Aufruf in `spawn_blocking` laufen
/// kann.
#[derive(Debug, Clone)]
pub struct Neu {
    pub gespraech: String,
    pub antwort: String,
    pub projekt: String,
    pub stand: String,
    pub inhalte: Vec<String>,
    pub ablage: Vec<String>,
    pub klasse_projekt: Klasse,
    pub klasse_ablage: Option<AblageKlasse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Datei {
    v: u32,
    salz: String,
    #[serde(default)]
    eintraege: Vec<Eintrag>,
}

/// Zustand des Registers beim Lesen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lage {
    /// Noch keine Datei: leer.
    Fehlt,
    /// Gelesen.
    Gelesen,
    /// Nicht lesbar oder kaputt (die Datei wurde, soweit möglich, beiseitegelegt): nichts ist bekannt.
    Defekt,
}

/// Gelesener Bestand.
#[derive(Debug, Clone)]
pub struct Bestand {
    pub lage: Lage,
    salz: String,
    eintraege: Vec<Eintrag>,
}

/// Normalisierter Antworttext: Leerraum zusammengefasst, damit ein vom Frontend gekürzter oder
/// umgebrochener Text dasselbe Merkmal behält.
pub fn normalisieren(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn merkmal(salz: &str, antwort: &str) -> String {
    // HMAC nimmt Schlüssel beliebiger Länge; `new_from_slice` scheitert dafür nie.
    let Ok(mut m) = Hmac::<Sha256>::new_from_slice(salz.as_bytes()) else {
        return String::new();
    };
    m.update(normalisieren(antwort).as_bytes());
    m.finalize().into_bytes()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn jetzt_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 256 Bit vom Zufallsgenerator des Betriebssystems. Scheitert der Generator, gibt es kein Salz und
/// damit keinen Eintrag (fail-closed).
fn neues_salz() -> Result<String, String> {
    let mut b = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut b)
        .map_err(|_| "Antwortregister: kein Zufall für das Salz.".to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn vereinigt(a: Vec<String>, b: Vec<String>) -> Vec<String> {
    a.into_iter()
        .chain(b)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

impl Bestand {
    fn leer(lage: Lage) -> Self {
        Self {
            lage,
            salz: String::new(),
            eintraege: Vec::new(),
        }
    }

    /// Der Eintrag zu dieser Antwort in diesem Gespräch, falls sie das Tor freigegeben hat.
    pub fn finden(&self, gespraech: &str, antwort: &str) -> Option<&Eintrag> {
        if self.salz.is_empty() {
            return None;
        }
        let m = merkmal(&self.salz, antwort);
        self.eintraege
            .iter()
            .find(|e| e.gespraech == gespraech && e.merkmal == m)
    }

    /// Einträge eines Gesprächs in einem Projekt.
    pub fn eintraege_von<'a>(
        &'a self,
        gespraech: &'a str,
        projekt: &'a str,
    ) -> impl Iterator<Item = &'a Eintrag> + 'a {
        self.eintraege
            .iter()
            .filter(move |e| e.gespraech == gespraech && e.projekt == projekt)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn anzahl(&self) -> usize {
        self.eintraege.len()
    }
}

/// Zugang zur Registerdatei eines Projektordners. Ohne eigenen Zustand: jeder Aufruf liest die Datei.
#[derive(Debug, Clone)]
pub struct Register {
    ordner: PathBuf,
}

fn pfad(ordner: &Path) -> PathBuf {
    ordner.join(DATEI)
}

/// Ergebnis des rohen Lesens. `Unlesbar` ist ein Lesefehler und kein Urteil über den Inhalt.
enum Roh {
    Fehlt,
    Gelesen(Datei),
    /// Gelesen, aber nicht auswertbar (kaputt, neuere Version, ohne Salz).
    Kaputt,
    /// Lesefehler außer „fehlt“: Die Datei wird nicht angefasst.
    Unlesbar,
}

fn lesen_roh(ordner: &Path) -> Roh {
    match fs::read(pfad(ordner)) {
        Ok(roh) => match serde_json::from_slice::<Datei>(&roh) {
            Ok(d) if d.v == VERSION && !d.salz.is_empty() => Roh::Gelesen(d),
            _ => Roh::Kaputt,
        },
        // „Fehlt“ heißt hier nur, dass das Betriebssystem `NotFound` meldet. Unter Windows gilt das auch,
        // wenn ein Elternglied eine Datei ist (ERROR_PATH_NOT_FOUND). Das bleibt fail-closed: Ein leeres
        // Register kennt keine Antwort, Antworten im Verlauf werden ersetzt, und das spätere Schreiben
        // scheitert mit `RegisterDefekt`.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Roh::Fehlt,
        Err(_) => Roh::Unlesbar,
    }
}

const UNLESBAR: &str = "Antwortregister: nicht lesbar.";

/// Legt eine defekte Datei beiseite (nie löschen). `true`, wenn sie danach nicht mehr im Weg liegt.
fn beiseitelegen(ordner: &Path) -> bool {
    let ziel = ordner.join(format!("{DATEI}.defekt-{}", jetzt_ms()));
    fs::rename(pfad(ordner), ziel).is_ok()
}

fn schreiben(ordner: &Path, d: &Datei) -> Result<(), String> {
    fs::create_dir_all(ordner)
        .map_err(|_| "Antwortregister: Ordner nicht anlegbar.".to_string())?;
    let roh = serde_json::to_vec(d).map_err(|_| "Antwortregister: nicht serialisierbar.")?;
    let tmp = ordner.join(format!(
        ".{DATEI}.{}-{:016x}.tmp",
        std::process::id(),
        rand::random::<u64>()
    ));
    let ergebnis = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut optionen = fs::OpenOptions::new();
        optionen.write(true).create_new(true);
        // Nur die eigene Person liest das Register (Merkmale, Marken, Klassen).
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut optionen, 0o600);
        let mut f = optionen.open(&tmp)?;
        f.write_all(&roh)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, pfad(ordner))?;
        // Den Ordnereintrag des Umbenennens sichern (unter Windows genügt das Durchschreiben des
        // Umbenennens).
        #[cfg(unix)]
        fs::File::open(ordner)?.sync_all()?;
        Ok(())
    })();
    if ergebnis.is_err() {
        let _ = fs::remove_file(&tmp);
        return Err("Antwortregister: nicht speicherbar.".into());
    }
    Ok(())
}

impl Register {
    pub fn neu(ordner: impl Into<PathBuf>) -> Self {
        Self {
            ordner: ordner.into(),
        }
    }

    /// Liest den Bestand. Eine Datei mit defektem Inhalt wird beiseitegelegt und als [`Lage::Defekt`]
    /// gemeldet. Ein Lesefehler ist ein `Err`: nichts wird umbenannt, der Turn muss abbrechen.
    pub fn laden(&self) -> Result<Bestand, String> {
        let _g = sperren();
        match lesen_roh(&self.ordner) {
            Roh::Gelesen(d) => Ok(Bestand {
                lage: Lage::Gelesen,
                salz: d.salz,
                eintraege: d.eintraege,
            }),
            Roh::Fehlt => Ok(Bestand::leer(Lage::Fehlt)),
            Roh::Kaputt => {
                beiseitelegen(&self.ordner);
                Ok(Bestand::leer(Lage::Defekt))
            }
            Roh::Unlesbar => Err(UNLESBAR.into()),
        }
    }

    /// Liest zum Ändern; fehlt die Datei oder ist ihr Inhalt defekt (dann beiseitegelegt), beginnt ein
    /// neuer Bestand mit neuem Salz. Kann eine defekte Datei nicht beiseitegelegt werden, oder ist sie
    /// nicht lesbar, ist das ein Fehler.
    fn zum_aendern(&self) -> Result<Datei, String> {
        let neu = || -> Result<Datei, String> {
            Ok(Datei {
                v: VERSION,
                salz: neues_salz()?,
                eintraege: Vec::new(),
            })
        };
        match lesen_roh(&self.ordner) {
            Roh::Gelesen(d) => Ok(d),
            Roh::Fehlt => neu(),
            Roh::Kaputt => {
                if !beiseitelegen(&self.ordner) {
                    return Err("Antwortregister: defekte Datei nicht beiseitelegbar.".into());
                }
                neu()
            }
            Roh::Unlesbar => Err(UNLESBAR.into()),
        }
    }

    /// Trägt eine freigegebene Antwort ein. Bei einem Fehler bleibt die bestehende Datei unverändert.
    /// Gibt es die Antwort in diesem Gespräch schon, wird ihre Herkunft vereinigt und ihre Klasse zum
    /// Maximum.
    pub fn eintragen(&self, n: Neu) -> Result<(), String> {
        let _g = sperren();
        let mut d = self.zum_aendern()?;
        let m = merkmal(&d.salz, &n.antwort);
        let (alt, rest): (Vec<Eintrag>, Vec<Eintrag>) = d
            .eintraege
            .into_iter()
            .partition(|e| e.gespraech == n.gespraech && e.merkmal == m);
        d.eintraege = rest;
        let (mut inhalte, mut ablage) = (n.inhalte, n.ablage);
        let (mut klasse_projekt, mut klasse_ablage) = (n.klasse_projekt, n.klasse_ablage);
        for a in alt {
            inhalte = vereinigt(inhalte, a.inhalte);
            ablage = vereinigt(ablage, a.ablage);
            klasse_projekt = klasse_projekt.max(a.klasse_projekt);
            klasse_ablage = klasse_ablage.max(a.klasse_ablage);
        }
        d.eintraege.push(Eintrag {
            gespraech: n.gespraech,
            merkmal: m,
            projekt: n.projekt,
            stand: n.stand,
            inhalte: vereinigt(inhalte, Vec::new()),
            ablage: vereinigt(ablage, Vec::new()),
            klasse_projekt,
            klasse_ablage,
            zeit: jetzt_ms(),
        });
        if d.eintraege.len() > MAX_EINTRAEGE {
            let zu_viel = d.eintraege.len() - MAX_EINTRAEGE;
            d.eintraege.drain(..zu_viel);
        }
        schreiben(&self.ordner, &d)
    }

    /// Entfernt alle Einträge eines Gesprächs (Gespräch gelöscht). Liefert ihre Zahl.
    pub fn gespraech_vergessen(&self, gespraech: &str) -> Result<usize, String> {
        self.entfernen(|e| e.gespraech == gespraech)
    }

    /// Entfernt den Eintrag dieser Antwort in diesem Gespräch (Abbruch nach dem Eintragen). Liefert ihre
    /// Zahl. Wurde die Antwort mit einem früheren, gleichlautenden Eintrag zusammengeführt, geht auch
    /// dieser verloren: Die Antwort gilt dann im Verlauf als unbekannt und wird verworfen (fail-closed).
    pub fn antwort_vergessen(&self, gespraech: &str, antwort: &str) -> Result<usize, String> {
        let _g = sperren();
        let mut d = match lesen_roh(&self.ordner) {
            Roh::Gelesen(d) => d,
            Roh::Fehlt | Roh::Kaputt => return Ok(0),
            Roh::Unlesbar => return Err(UNLESBAR.into()),
        };
        let m = merkmal(&d.salz, antwort);
        let vorher = d.eintraege.len();
        d.eintraege
            .retain(|e| !(e.gespraech == gespraech && e.merkmal == m));
        let entfernt = vorher - d.eintraege.len();
        if entfernt > 0 {
            schreiben(&self.ordner, &d)?;
        }
        Ok(entfernt)
    }

    /// Entfernt alle Einträge eines Projekts (Projekt gelöscht). Liefert ihre Zahl.
    pub fn projekt_vergessen(&self, projekt: &str) -> Result<usize, String> {
        self.entfernen(|e| e.projekt == projekt)
    }

    fn entfernen(&self, weg: impl Fn(&Eintrag) -> bool) -> Result<usize, String> {
        let _g = sperren();
        // Fehlt die Datei oder ist ihr Inhalt defekt, gibt es nichts zu entfernen (beim Lesen für das
        // nächste Tor-Verfahren wird sie beiseitegelegt). Ein Lesefehler ist ein Fehler.
        let mut d = match lesen_roh(&self.ordner) {
            Roh::Gelesen(d) => d,
            Roh::Fehlt | Roh::Kaputt => return Ok(0),
            Roh::Unlesbar => return Err(UNLESBAR.into()),
        };
        let vorher = d.eintraege.len();
        d.eintraege.retain(|e| !weg(e));
        let entfernt = vorher - d.eintraege.len();
        if entfernt > 0 {
            schreiben(&self.ordner, &d)?;
        }
        Ok(entfernt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn neu(g: &str, antwort: &str, p: &str) -> Neu {
        Neu {
            gespraech: g.into(),
            antwort: antwort.into(),
            projekt: p.into(),
            stand: "s".into(),
            inhalte: vec![],
            ablage: vec![],
            klasse_projekt: Klasse::Internal,
            klasse_ablage: None,
        }
    }

    fn beiseite(ordner: &Path) -> Vec<fs::DirEntry> {
        fs::read_dir(ordner)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{DATEI}.defekt-"))
            })
            .collect()
    }

    #[test]
    fn eintragen_und_finden_normalisiert_leerraum() {
        let d = tmp();
        let r = Register::neu(d.path());
        assert_eq!(r.laden().unwrap().lage, Lage::Fehlt);
        r.eintragen(neu("g1", "Das  ist\n eine Antwort", "prj_a"))
            .unwrap();
        let b = r.laden().unwrap();
        assert_eq!(b.lage, Lage::Gelesen);
        assert!(b.finden("g1", "Das ist eine Antwort ").is_some());
        assert!(b.finden("g2", "Das ist eine Antwort").is_none());
        assert!(b.finden("g1", "Eine andere").is_none());
    }

    #[test]
    fn datei_enthaelt_keinen_klartext() {
        let d = tmp();
        let r = Register::neu(d.path());
        r.eintragen(neu("g1", "KONTROLLTEXT-reg-1", "prj_a"))
            .unwrap();
        let roh = fs::read_to_string(d.path().join(DATEI)).unwrap();
        assert!(!roh.contains("KONTROLLTEXT"));
    }

    #[test]
    fn defekte_datei_wird_beiseitegelegt_nicht_geloescht() {
        let d = tmp();
        fs::write(d.path().join(DATEI), b"{kaputt").unwrap();
        let r = Register::neu(d.path());
        assert_eq!(r.laden().unwrap().lage, Lage::Defekt);
        assert!(!d.path().join(DATEI).exists());
        let b = beiseite(d.path());
        assert_eq!(b.len(), 1);
        assert_eq!(fs::read(b[0].path()).unwrap(), b"{kaputt");
        // Danach beginnt ein neuer Bestand.
        r.eintragen(neu("g1", "a", "prj_a")).unwrap();
        assert_eq!(r.laden().unwrap().lage, Lage::Gelesen);
    }

    #[test]
    fn neuere_version_gilt_als_defekt() {
        let d = tmp();
        fs::write(
            d.path().join(DATEI),
            br#"{"v":99,"salz":"abc","eintraege":[]}"#,
        )
        .unwrap();
        assert_eq!(Register::neu(d.path()).laden().unwrap().lage, Lage::Defekt);
    }

    /// Ein Lesefehler ist kein Urteil über den Inhalt: nichts wird beiseitegelegt, `laden`, `eintragen`
    /// und `entfernen` scheitern, die Datei bleibt, wie sie ist. (Hier: der Pfad ist ein Ordner; unter
    /// Unix scheitert das Lesen mit `EISDIR`, unter Windows mit „Zugriff verweigert“.)
    #[test]
    fn lesefehler_bricht_ab_ohne_umbenennen() {
        let d = tmp();
        let pfad = d.path().join(DATEI);
        fs::create_dir(&pfad).unwrap();
        fs::write(pfad.join("inhalt"), b"x").unwrap();
        let r = Register::neu(d.path());
        assert!(r.laden().is_err());
        assert!(r.eintragen(neu("g1", "a", "prj_a")).is_err());
        assert!(r.gespraech_vergessen("g1").is_err());
        assert!(r.projekt_vergessen("prj_a").is_err());
        assert!(pfad.is_dir(), "nicht umbenannt, nicht ersetzt");
        assert_eq!(fs::read(pfad.join("inhalt")).unwrap(), b"x");
        assert!(beiseite(d.path()).is_empty(), "nichts beiseitegelegt");
    }

    #[test]
    fn begrenzung_wirft_die_aeltesten_heraus() {
        let d = tmp();
        let r = Register::neu(d.path());
        // Direkt über die Datei füllen: MAX_EINTRAEGE Schreibvorgänge wären zu langsam.
        r.eintragen(neu("g1", "erste", "prj_a")).unwrap();
        let mut datei: Datei =
            serde_json::from_slice(&fs::read(d.path().join(DATEI)).unwrap()).unwrap();
        let vorlage = datei.eintraege[0].clone();
        for i in 0..MAX_EINTRAEGE {
            let mut e = vorlage.clone();
            e.merkmal = format!("m{i}");
            datei.eintraege.push(e);
        }
        fs::write(d.path().join(DATEI), serde_json::to_vec(&datei).unwrap()).unwrap();
        r.eintragen(neu("g1", "neueste", "prj_a")).unwrap();
        let b = r.laden().unwrap();
        assert_eq!(b.anzahl(), MAX_EINTRAEGE);
        assert!(b.finden("g1", "neueste").is_some());
        assert!(
            b.finden("g1", "erste").is_none(),
            "älteste fällt zuerst heraus"
        );
    }

    #[test]
    fn vergessen_nach_gespraech_und_projekt() {
        let d = tmp();
        let r = Register::neu(d.path());
        r.eintragen(neu("g1", "a", "prj_a")).unwrap();
        r.eintragen(neu("g1", "b", "prj_a")).unwrap();
        r.eintragen(neu("g2", "c", "prj_b")).unwrap();
        assert_eq!(r.gespraech_vergessen("g1").unwrap(), 2);
        assert_eq!(r.laden().unwrap().anzahl(), 1);
        assert_eq!(r.projekt_vergessen("prj_b").unwrap(), 1);
        assert_eq!(r.laden().unwrap().anzahl(), 0);
        assert_eq!(r.projekt_vergessen("prj_b").unwrap(), 0);
    }

    #[test]
    fn schreibfehler_laesst_bestehende_datei_unveraendert() {
        let d = tmp();
        let r = Register::neu(d.path());
        r.eintragen(neu("g1", "a", "prj_a")).unwrap();
        let vorher = fs::read(d.path().join(DATEI)).unwrap();
        // Ordner nicht beschreibbar: ein Ordner, der unter einer Datei liegen soll, ist nicht anlegbar.
        let datei_im_weg = d.path().join("datei");
        fs::write(&datei_im_weg, b"x").unwrap();
        let kaputt = Register::neu(datei_im_weg.join("unter"));
        assert!(kaputt.eintragen(neu("g1", "b", "prj_a")).is_err());
        assert_eq!(fs::read(d.path().join(DATEI)).unwrap(), vorher);
    }

    #[test]
    fn gleichzeitiges_eintragen_verliert_nichts() {
        let d = tmp();
        let r = Register::neu(d.path());
        std::thread::scope(|s| {
            for i in 0..16 {
                let r = r.clone();
                s.spawn(move || {
                    r.eintragen(neu("g1", &format!("Antwort {i}"), "prj_a"))
                        .unwrap();
                });
            }
        });
        let b = r.laden().unwrap();
        assert_eq!(b.anzahl(), 16);
        for i in 0..16 {
            assert!(b.finden("g1", &format!("Antwort {i}")).is_some());
        }
    }

    #[test]
    fn erneutes_eintragen_vereinigt_die_herkunft() {
        let d = tmp();
        let r = Register::neu(d.path());
        let mut a = neu("g1", "Ja.", "prj_a");
        a.inhalte = vec!["eintrag:x#1".into()];
        a.ablage = vec!["dokument:1".into()];
        a.klasse_projekt = Klasse::Personal;
        a.klasse_ablage = Some(AblageKlasse::Internal);
        r.eintragen(a).unwrap();
        let mut b = neu("g1", "Ja.", "prj_a");
        b.inhalte = vec!["eintrag:y#2".into(), "eintrag:x#1".into()];
        b.ablage = vec!["dokument:2".into()];
        b.klasse_projekt = Klasse::Internal;
        b.klasse_ablage = Some(AblageKlasse::Confidential);
        r.eintragen(b).unwrap();
        let bestand = r.laden().unwrap();
        assert_eq!(bestand.anzahl(), 1, "ein Eintrag, nicht zwei");
        let e = bestand.finden("g1", "Ja.").unwrap();
        assert_eq!(e.inhalte, vec!["eintrag:x#1", "eintrag:y#2"]);
        assert_eq!(e.ablage, vec!["dokument:1", "dokument:2"]);
        assert_eq!(e.klasse_projekt, Klasse::Personal, "Maximum");
        assert_eq!(e.klasse_ablage, Some(AblageKlasse::Confidential), "Maximum");
        // Ohne Klasse beim zweiten Mal bleibt die erste.
        let mut c = neu("g1", "Ja.", "prj_a");
        c.klasse_ablage = None;
        r.eintragen(c).unwrap();
        let bestand = r.laden().unwrap();
        let e = bestand.finden("g1", "Ja.").unwrap();
        assert_eq!(e.klasse_ablage, Some(AblageKlasse::Confidential));
        assert_eq!(e.inhalte.len(), 2);
    }

    #[test]
    fn salz_ist_lang_und_je_datei_verschieden() {
        let (d1, d2) = (tmp(), tmp());
        for d in [&d1, &d2] {
            Register::neu(d.path())
                .eintragen(neu("g1", "a", "prj_a"))
                .unwrap();
        }
        let lies = |d: &tempfile::TempDir| -> Datei {
            serde_json::from_slice(&fs::read(d.path().join(DATEI)).unwrap()).unwrap()
        };
        let (s1, s2) = (lies(&d1).salz, lies(&d2).salz);
        assert_eq!(s1.len(), 64, "256 Bit als Hex");
        assert!(s1.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(s1, s2);
        // Dieselbe Antwort ergibt in zwei Dateien verschiedene Merkmale.
        assert_ne!(merkmal(&s1, "a"), merkmal(&s2, "a"));
    }

    #[cfg(unix)]
    #[test]
    fn datei_hat_unter_unix_die_rechte_0600() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp();
        let r = Register::neu(d.path());
        r.eintragen(neu("g1", "a", "prj_a")).unwrap();
        let modus = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(modus(&d.path().join(DATEI)), 0o600);
        // Auch nach dem zweiten Schreiben (neue Temp-Datei, Umbenennen).
        r.eintragen(neu("g1", "b", "prj_a")).unwrap();
        assert_eq!(modus(&d.path().join(DATEI)), 0o600);
        // Keine Temp-Datei bleibt liegen.
        let reste = fs::read_dir(d.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(reste, 0);
    }
}
