//! Antwortregister des Versandtors (lva#365 P4c-1, Entscheidung E2).
//!
//! Der Verlauf eines Projektgesprächs kommt bei jedem Turn als Text vom Frontend. Der Kern sieht ihm nicht
//! an, aus welchem Projektinhalt eine frühere Antwort stammt. Darum trägt das Tor jede **freigegebene**
//! Antwort mit ihrer Herkunft ins Register ein und schlägt sie beim nächsten Turn wieder nach: Eine Antwort
//! bleibt nur im Verlauf, wenn alle ihre Quellen noch gelten. Unbekannte Antworten gelten als entzogen.
//!
//! Datei `projekte/ki-herkunft.json` (gerätelokal):
//! - Je Antwort ein gesalzenes Merkmal (`HMAC-SHA256(salz, normalisierter Text)`), nie Klartext, Titel
//!   oder Namen. Das Salz gilt je Datei.
//! - Schreiben atomar (eigene Temp-Datei, fsync, Umbenennen) und unter einer Prozess-Sperre. Ein Fehler
//!   beim Schreiben lässt die bestehende Datei unverändert.
//! - Fehlt die Datei, ist das Register leer. Ist sie **defekt** (nicht lesbar, kaputt, neuere Version),
//!   wird sie als `ki-herkunft.json.defekt-<ms>` beiseitegelegt (nie gelöscht) und [`Lage::Defekt`]
//!   gemeldet; das Tor setzt dann den Verlauf zurück. Alle alten Antworten sind danach unbekannt.
//! - Der Bestand ist begrenzt ([`MAX_EINTRAEGE`]); die ältesten Einträge fallen zuerst heraus. Das ist
//!   sicher: Was nicht im Register steht, gilt als entzogen.

use crate::ablage::zusammenfuehren::Klasse as AblageKlasse;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
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
    /// Marken `eintrag:<id>#<h16>` der gelieferten Projektinhalte.
    pub inhalte: Vec<String>,
    /// Marken `dokument:<dat>@<fas>#<textrev>` der gelieferten Ablage-Auszüge.
    pub ablage: Vec<String>,
    /// Höchste wirksame Klasse der gelieferten Projektinhalte (mit Projektklasse und Art.-9-Kennzeichen).
    pub klasse_projekt: Klasse,
    /// Höchste Klasse der gelieferten Ablage-Dateien.
    #[serde(default)]
    pub klasse_ablage: Option<AblageKlasse>,
    pub zeit: i64,
}

/// Angaben zu einer neu freigegebenen Antwort.
pub struct Neu<'a> {
    pub gespraech: &'a str,
    pub antwort: &'a str,
    pub projekt: &'a str,
    pub stand: &'a str,
    pub inhalte: &'a [String],
    pub ablage: &'a [String],
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

fn neues_salz() -> String {
    format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    )
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

fn lesen_roh(ordner: &Path) -> (Lage, Option<Datei>) {
    match fs::read(pfad(ordner)) {
        Ok(roh) => match serde_json::from_slice::<Datei>(&roh) {
            Ok(d) if d.v == VERSION && !d.salz.is_empty() => (Lage::Gelesen, Some(d)),
            _ => (Lage::Defekt, None),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Lage::Fehlt, None),
        Err(_) => (Lage::Defekt, None),
    }
}

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
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
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

    /// Liest den Bestand. Eine defekte Datei wird beiseitegelegt und als [`Lage::Defekt`] gemeldet.
    pub fn laden(&self) -> Bestand {
        let _g = sperren();
        match lesen_roh(&self.ordner) {
            (Lage::Gelesen, Some(d)) => Bestand {
                lage: Lage::Gelesen,
                salz: d.salz,
                eintraege: d.eintraege,
            },
            (Lage::Fehlt, _) => Bestand::leer(Lage::Fehlt),
            _ => {
                beiseitelegen(&self.ordner);
                Bestand::leer(Lage::Defekt)
            }
        }
    }

    /// Liest zum Ändern; fehlt die Datei oder ist sie defekt (dann beiseitegelegt), beginnt ein neuer
    /// Bestand mit neuem Salz. Kann eine defekte Datei nicht beiseitegelegt werden, ist das ein Fehler.
    fn zum_aendern(&self) -> Result<Datei, String> {
        match lesen_roh(&self.ordner) {
            (Lage::Gelesen, Some(d)) => Ok(d),
            (Lage::Fehlt, _) => Ok(Datei {
                v: VERSION,
                salz: neues_salz(),
                eintraege: Vec::new(),
            }),
            _ => {
                if !beiseitelegen(&self.ordner) {
                    return Err("Antwortregister: defekte Datei nicht beiseitelegbar.".into());
                }
                Ok(Datei {
                    v: VERSION,
                    salz: neues_salz(),
                    eintraege: Vec::new(),
                })
            }
        }
    }

    /// Trägt eine freigegebene Antwort ein. Bei einem Fehler bleibt die bestehende Datei unverändert.
    pub fn eintragen(&self, n: Neu<'_>) -> Result<(), String> {
        let _g = sperren();
        let mut d = self.zum_aendern()?;
        let m = merkmal(&d.salz, n.antwort);
        d.eintraege
            .retain(|e| !(e.gespraech == n.gespraech && e.merkmal == m));
        d.eintraege.push(Eintrag {
            gespraech: n.gespraech.to_string(),
            merkmal: m,
            projekt: n.projekt.to_string(),
            stand: n.stand.to_string(),
            inhalte: n.inhalte.to_vec(),
            ablage: n.ablage.to_vec(),
            klasse_projekt: n.klasse_projekt,
            klasse_ablage: n.klasse_ablage,
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

    /// Entfernt alle Einträge eines Projekts (Projekt gelöscht). Liefert ihre Zahl.
    pub fn projekt_vergessen(&self, projekt: &str) -> Result<usize, String> {
        self.entfernen(|e| e.projekt == projekt)
    }

    fn entfernen(&self, weg: impl Fn(&Eintrag) -> bool) -> Result<usize, String> {
        let _g = sperren();
        // Fehlt die Datei oder ist sie defekt, gibt es nichts zu entfernen (beim Lesen für das nächste
        // Tor-Verfahren wird sie beiseitegelegt).
        let (Lage::Gelesen, Some(mut d)) = lesen_roh(&self.ordner) else {
            return Ok(0);
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

    fn neu<'a>(g: &'a str, antwort: &'a str, p: &'a str) -> Neu<'a> {
        Neu {
            gespraech: g,
            antwort,
            projekt: p,
            stand: "s",
            inhalte: &[],
            ablage: &[],
            klasse_projekt: Klasse::Internal,
            klasse_ablage: None,
        }
    }

    #[test]
    fn eintragen_und_finden_normalisiert_leerraum() {
        let d = tmp();
        let r = Register::neu(d.path());
        assert_eq!(r.laden().lage, Lage::Fehlt);
        r.eintragen(neu("g1", "Das  ist\n eine Antwort", "prj_a"))
            .unwrap();
        let b = r.laden();
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
        assert_eq!(r.laden().lage, Lage::Defekt);
        assert!(!d.path().join(DATEI).exists());
        let beiseite: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{DATEI}.defekt-"))
            })
            .collect();
        assert_eq!(beiseite.len(), 1);
        assert_eq!(fs::read(beiseite[0].path()).unwrap(), b"{kaputt");
        // Danach beginnt ein neuer Bestand.
        r.eintragen(neu("g1", "a", "prj_a")).unwrap();
        assert_eq!(r.laden().lage, Lage::Gelesen);
    }

    #[test]
    fn neuere_version_gilt_als_defekt() {
        let d = tmp();
        fs::write(
            d.path().join(DATEI),
            br#"{"v":99,"salz":"abc","eintraege":[]}"#,
        )
        .unwrap();
        assert_eq!(Register::neu(d.path()).laden().lage, Lage::Defekt);
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
        let b = r.laden();
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
        assert_eq!(r.laden().anzahl(), 1);
        assert_eq!(r.projekt_vergessen("prj_b").unwrap(), 1);
        assert_eq!(r.laden().anzahl(), 0);
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
        let b = r.laden();
        assert_eq!(b.anzahl(), 16);
        for i in 0..16 {
            assert!(b.finden("g1", &format!("Antwort {i}")).is_some());
        }
    }
}
