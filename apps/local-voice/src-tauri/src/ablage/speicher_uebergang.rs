//! D4: Rückweg lokal → sync (Spec §6, Tests D4a).
//!
//! Festgehaltene Kennungen (`lokal`) wurden am Hub begraben (Grabsteine sind endgültig) oder werden es
//! gleich. Wird ein Eintrag wieder synchron („nur dieses Gerät“ aus, Klasse wieder teilbar), bekommt er
//! deshalb eine neue Kennung, der ganze festgehaltene Teilbaum mit ihm. Der Wechsel läuft über ein
//! eigenes Journal `ablage/uebergang-<ULID>.json` und ist nach jedem Schritt absturzsicher:
//!
//! 1. `begonnen`: Zuordnung alt → neu steht fest (vor jeder Änderung geschrieben).
//! 2. Originale und Textfassungen ziehen um (`dateien/<alt>` → `dateien/<neu>`, `text/…`), dann wird der
//!    Stand in einem Schreibvorgang umgeschrieben: Kennungen, Ordner- und Elternverweise (mit neuem
//!    Register, damit befreite Einträge den Umzug am Hub weitergeben), `lokal` und die Weiterleitung
//!    alt → neu (Deep Links). Danach `umgezogen`. Die Wartung führt diesen lokalen Teil beim Start zu
//!    Ende, bevor sie nach Funden sucht.
//! 3. KI-Freigaben (`freigabe.json`), Projektverweise (`projekte/kontext.json`) und Suchindex werden
//!    umgeschrieben, dann `umgeschrieben`.
//! 4. `fertig`, das Journal wird entfernt.
//!
//! Jeder Schritt ist wiederholbar; ein Wiederanlauf setzt beim letzten vermerkten Schritt fort. Die alten
//! Kennungen bleiben nur als Weiterleitung; lebt eine am Hub noch, sendet der Abgleich ihren Grabstein
//! (`sync::ablage`), und Stände vom Hub zu alten Kennungen werden nie wieder aufgenommen.
//!
//! Dateien, deren Original hier fehlt, bleiben festgehalten (sonst ginge mit dem alten Grabstein die
//! einzige Kopie); ihr Ordnerverweis folgt trotzdem dem Umzug.

use super::{meta_ohne_folgen, ohne_umweg, original_name, Ablage, Eintrag, Stand, DATEIEN, TEXT};
use crate::ablage::zusammenfuehren::Register;
use crate::ablage::{
    freigabe, id_gueltig, jetzt_ms, neue_id, schreibe_atomar, sperren, Fehler, Res, PRAEFIX_DATEI,
    PRAEFIX_ORDNER,
};
use crate::sync::ablage::Teilnahme;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
use super::{haken, Schritt};

/// Dateiname des Übergangsjournals: `uebergang-<ULID>.json`.
pub const UEBERGANG: &str = "uebergang-";
const VERSION: u32 = 1;
/// Längste Weiterleitungskette, der gefolgt wird.
const MAX_WEITER: usize = 16;

/// Nur ein Rückweg zur Zeit (Befehl und Sync-Takt können gleichzeitig anstoßen).
static RUECKWEG: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stufe {
    Begonnen,
    Umgezogen,
    Umgeschrieben,
    Fertig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Uebergang {
    v: u32,
    id: String,
    schritt: Stufe,
    zuordnung: BTreeMap<String, String>,
    ms: i64,
}

impl Uebergang {
    fn gueltig(&self) -> bool {
        let neu: BTreeSet<&String> = self.zuordnung.values().collect();
        self.v == VERSION
            && ulid::Ulid::from_string(&self.id).is_ok()
            && neu.len() == self.zuordnung.len()
            && self
                .zuordnung
                .iter()
                .all(|(a, n)| gleiche_art(a, n) && a != n)
    }
}

/// Beide Kennungen gültig und von derselben Art (Ordner oder Datei).
pub(super) fn gleiche_art(alt: &str, neu: &str) -> bool {
    [PRAEFIX_ORDNER, PRAEFIX_DATEI]
        .iter()
        .any(|p| id_gueltig(p, alt) && id_gueltig(p, neu))
}

/// Verweise außerhalb der Ablage, die beim Rückweg der neuen Kennung folgen müssen.
pub trait UebergangVerweise {
    fn umschreiben(&self, zuordnung: &BTreeMap<String, String>) -> Result<(), String>;
}

/// Projektverweise im Schalterbestand der Projekte (`<Datenordner>/projekte/kontext.json`).
///
/// Die Projektdatenbank (vendored `wai-projekte`) kennt noch keinen Verweistyp für Ablage-Einträge;
/// kommt er, gehört sein Umschreiben hierher.
pub struct ProjektVerweise {
    dir: PathBuf,
}

impl ProjektVerweise {
    pub fn neu(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl UebergangVerweise for ProjektVerweise {
    fn umschreiben(&self, zuordnung: &BTreeMap<String, String>) -> Result<(), String> {
        use crate::projekte::herkunft::ablage_link;
        let paare: Vec<(String, String)> = zuordnung
            .iter()
            .map(|(a, n)| (ablage_link(a), ablage_link(n)))
            .collect();
        crate::projekte::kontext::links_umschreiben(&self.dir, &paare).map(|_| ())
    }
}

#[cfg(not(test))]
#[derive(Clone, Copy)]
enum Schritt {
    UebergangBegonnen,
    UebergangDateien,
    UebergangUmgezogen,
    UebergangFreigaben,
    UebergangUmgeschrieben,
    UebergangFertig,
}

#[cfg(not(test))]
fn haken(_: Schritt) -> Res<()> {
    Ok(())
}

fn uebergang_fehler() -> Fehler {
    Fehler::speicher("Rückweg zu „synchron“ nicht möglich; wird beim nächsten Lauf fortgesetzt.")
}

/// Stand umschreiben (rein, wiederholbar). `true`, wenn sich etwas geändert hat.
fn stand_umziehen(st: &mut Stand, m: &BTreeMap<String, String>) -> Res<bool> {
    let (z, g) = (st.zaehler()?, st.geraet.clone());
    let mut geaendert = false;
    for (alt, neu) in m {
        if let Some(mut o) = st.ordner.remove(alt) {
            o.id = neu.clone();
            st.ordner.entry(neu.clone()).or_insert(o);
            geaendert = true;
        }
        if let Some(mut d) = st.dateien.remove(alt) {
            d.id = neu.clone();
            st.dateien.entry(neu.clone()).or_insert(d);
            geaendert = true;
        }
    }
    // Neue Kennungen kennt sonst niemand: ihre Verweise behalten den Rang (ein später gezogener Stand
    // zur alten Kennung entscheidet dann wie gewohnt nach Rang). Übrige Einträge (befreit, synchron)
    // bekommen ein neues Register, damit der Umzug am Hub weitergeht.
    let neue: BTreeSet<&String> = m.values().collect();
    for o in st.ordner.values_mut() {
        if let Some(n) = o.eltern.wert.as_ref().and_then(|p| m.get(p)) {
            o.eltern = if neue.contains(&o.id) {
                Register::neu(Some(n.clone()), o.eltern.zaehler, &o.eltern.geraet)
            } else {
                Register::neu(Some(n.clone()), z, &g)
            };
            geaendert = true;
        }
    }
    for d in st.dateien.values_mut() {
        if let Some(n) = m.get(&d.ordner.wert) {
            d.ordner = if neue.contains(&d.id) {
                Register::neu(n.clone(), d.ordner.zaehler, &d.ordner.geraet)
            } else {
                Register::neu(n.clone(), z, &g)
            };
            geaendert = true;
        }
    }
    let abbilden = |v: &mut String| {
        if let Some(n) = m.get(v.as_str()) {
            *v = n.clone();
            true
        } else {
            false
        }
    };
    for e in st.journal.iter_mut() {
        match e {
            Eintrag::Abgelegt { dat, ordner, .. } => {
                geaendert |= abbilden(dat);
                geaendert |= abbilden(ordner);
            }
            Eintrag::Loeschen { dat, ordner, .. } => {
                for x in dat.iter_mut().chain(ordner.iter_mut()) {
                    geaendert |= abbilden(x);
                }
            }
        }
    }
    let vorher = st.lokal.len();
    st.lokal.retain(|i| !m.contains_key(i));
    geaendert |= st.lokal.len() != vorher;
    for b in st.befreit.iter_mut() {
        geaendert |= abbilden(b);
    }
    for v in st.weiterleitung.values_mut() {
        geaendert |= abbilden(v);
    }
    for (alt, neu) in m {
        if st.weiterleitung.get(alt) != Some(neu) {
            st.weiterleitung.insert(alt.clone(), neu.clone());
            geaendert = true;
        }
    }
    Ok(geaendert)
}

impl Stand {
    /// Folgt der Weiterleitung alter Kennungen (Rückweg lokal → sync).
    pub(super) fn aufgeloest(&self, id: &str) -> String {
        let mut aktuell = id.to_string();
        for _ in 0..MAX_WEITER {
            match self.weiterleitung.get(&aktuell) {
                Some(n) => aktuell = n.clone(),
                None => break,
            }
        }
        aktuell
    }
}

impl Ablage {
    fn uebergaenge_lesen(&self) -> Vec<(PathBuf, Uebergang)> {
        let Ok(r) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut aus: Vec<(PathBuf, Uebergang)> = r
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let id = name.strip_prefix(UEBERGANG)?.strip_suffix(".json")?;
                let p = e.path();
                if meta_ohne_folgen(&p).ok().is_none_or(|m| !m.is_file()) {
                    return None;
                }
                let u: Uebergang = serde_json::from_slice(&fs::read(&p).ok()?).ok()?;
                if !u.gueltig() || u.id != id {
                    log::warn!("ablage: Übergangsjournal unlesbar, bleibt liegen");
                    return None;
                }
                Some((p, u))
            })
            .collect();
        aus.sort_by(|a, b| a.1.id.cmp(&b.1.id));
        aus
    }

    fn uebergang_schreiben(&self, p: &Path, u: &Uebergang) -> Res<()> {
        let roh = serde_json::to_vec_pretty(u).map_err(|_| uebergang_fehler())?;
        schreibe_atomar(p, &roh)
    }

    /// `dateien/<alt>` bzw. `text/<alt>` nach `<neu>` (wiederholbar; nie ersetzen, nie Verknüpfungen
    /// folgen).
    fn ordner_umziehen(&self, basis: &str, alt: &str, neu: &str) -> Res<()> {
        ohne_umweg(&self.dir, basis)?;
        let (a, n) = (
            self.dir.join(basis).join(alt),
            self.dir.join(basis).join(neu),
        );
        let ma = match meta_ohne_folgen(&a) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(uebergang_fehler()),
        };
        if !ma.is_dir() {
            return Err(uebergang_fehler());
        }
        match meta_ohne_folgen(&n) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return fs::rename(&a, &n).map_err(|_| uebergang_fehler());
            }
            Ok(m) if m.is_dir() => {}
            _ => return Err(uebergang_fehler()),
        }
        // Beide da (nach dem Umzug wieder angelegt): Fehlendes nachziehen, Doppeltes bleibt am Ziel.
        for e in fs::read_dir(&a).map_err(|_| uebergang_fehler())?.flatten() {
            let ziel = n.join(e.file_name());
            if meta_ohne_folgen(&ziel).is_err() {
                fs::rename(e.path(), &ziel).map_err(|_| uebergang_fehler())?;
            } else if e.path().is_file() {
                let _ = fs::remove_file(e.path());
            }
        }
        fs::remove_dir(&a).map_err(|_| uebergang_fehler())
    }

    /// Lokaler Teil (unter der Sperre): Originale und Texte umziehen, Stand umschreiben, `umgezogen`.
    fn umziehen(&self, st: &mut Stand, p: &Path, u: &mut Uebergang) -> Res<()> {
        let dateien: Vec<(&String, &String)> = u
            .zuordnung
            .iter()
            .filter(|(a, _)| id_gueltig(PRAEFIX_DATEI, a))
            .collect();
        for (alt, neu) in &dateien {
            self.ordner_umziehen(DATEIEN, alt, neu)?;
        }
        haken(Schritt::UebergangDateien)?;
        for (alt, neu) in &dateien {
            self.ordner_umziehen(TEXT, alt, neu)?;
        }
        st.vorher = None;
        if stand_umziehen(st, &u.zuordnung)? {
            self.sichern(st)?;
        }
        u.schritt = Stufe::Umgezogen;
        self.uebergang_schreiben(p, u)?;
        haken(Schritt::UebergangUmgezogen)
    }

    /// Wartung: begonnene Übergänge lokal zu Ende führen (vor der Suche nach Funden). Liefert
    /// (geändert, alles erledigt).
    pub(super) fn uebergaenge_lokal(&self, st: &mut Stand) -> (bool, bool) {
        let mut geaendert = false;
        let mut ok = true;
        for (p, mut u) in self.uebergaenge_lesen() {
            if u.schritt != Stufe::Begonnen {
                continue;
            }
            let epoche = st.epoche;
            if let Err(e) = self.umziehen(st, &p, &mut u) {
                log::warn!("ablage: Rückweg nicht fortgesetzt: {}", e.code);
                ok = false;
            }
            geaendert |= st.epoche != epoche;
        }
        (geaendert, ok)
    }

    /// Festgehaltene Kennungen, die wieder synchron wären und umziehen können.
    fn rueckweg_kandidaten(&self, st: &Stand) -> Vec<String> {
        if st.lokal.is_empty() {
            return Vec::new();
        }
        let t = crate::sync::ablage::teilnahmen(
            &st.ordner,
            &st.dateien,
            &BTreeSet::new(),
            &st.befreit_menge(),
        );
        let (od, oo) = st.offen_geloescht();
        st.lokal
            .iter()
            .filter(|id| t.get(*id) == Some(&Teilnahme::Sync))
            .filter(|id| {
                !st.begraben.contains(*id) && !od.contains(id.as_str()) && !oo.contains(id.as_str())
            })
            .filter(|id| match st.dateien.get(*id) {
                Some(d) => d.fassungen.iter().all(|f| {
                    let p = self
                        .dir
                        .join(DATEIEN)
                        .join(&d.id)
                        .join(original_name(&f.fas, &f.typ));
                    meta_ohne_folgen(&p).is_ok_and(|m| m.is_file())
                }),
                None => st.ordner.contains_key(*id),
            })
            .cloned()
            .collect()
    }

    /// Rückweg lokal → sync: setzt offene Übergänge fort und beginnt einen neuen für alle festgehaltenen
    /// Einträge, die wieder synchron wären. Liefert die Zahl neu vergebener Kennungen.
    pub fn rueckweg(&self, verweise: &dyn UebergangVerweise) -> Res<u32> {
        let _r = RUECKWEG.lock().unwrap_or_else(|e| e.into_inner());
        let (offen, n) = {
            let _g = sperren();
            let mut st = self.laden()?;
            self.offene_zuerst(&mut st)?;
            let mut offen = self.uebergaenge_lesen();
            for (p, u) in offen.iter_mut() {
                if u.schritt == Stufe::Begonnen {
                    self.umziehen(&mut st, p, u)?;
                }
            }
            let kandidaten = self.rueckweg_kandidaten(&st);
            let n = kandidaten.len() as u32;
            if !kandidaten.is_empty() {
                let zuordnung = kandidaten
                    .into_iter()
                    .map(|alt| {
                        let p = if alt.starts_with(PRAEFIX_DATEI) {
                            PRAEFIX_DATEI
                        } else {
                            PRAEFIX_ORDNER
                        };
                        (alt, neue_id(p))
                    })
                    .collect();
                let mut u = Uebergang {
                    v: VERSION,
                    id: ulid::Ulid::new().to_string(),
                    schritt: Stufe::Begonnen,
                    zuordnung,
                    ms: jetzt_ms(),
                };
                let p = self.dir.join(format!("{UEBERGANG}{}.json", u.id));
                self.uebergang_schreiben(&p, &u)?;
                haken(Schritt::UebergangBegonnen)?;
                self.umziehen(&mut st, &p, &mut u)?;
                offen.push((p, u));
            }
            (offen, n)
        };
        for (p, mut u) in offen {
            self.verweise_umschreiben(&p, &mut u, verweise)?;
        }
        Ok(n)
    }

    /// Freigaben, Projektverweise und Index folgen den neuen Kennungen; dann `fertig` und weg.
    fn verweise_umschreiben(
        &self,
        p: &Path,
        u: &mut Uebergang,
        verweise: &dyn UebergangVerweise,
    ) -> Res<()> {
        if u.schritt < Stufe::Umgeschrieben {
            {
                let _g = sperren();
                freigabe::umschreiben(&self.dir, &u.zuordnung)?;
            }
            haken(Schritt::UebergangFreigaben)?;
            verweise.umschreiben(&u.zuordnung).map_err(|_| {
                Fehler::speicher("Projektverweise nicht umschreibbar; wird fortgesetzt.")
            })?;
            if let Err(e) = crate::ablage::index::abgleichen(self) {
                log::warn!("ablage: Indexabgleich nach Rückweg: {}", e.code);
            }
            u.schritt = Stufe::Umgeschrieben;
            self.uebergang_schreiben(p, u)?;
            haken(Schritt::UebergangUmgeschrieben)?;
        }
        if u.schritt < Stufe::Fertig {
            u.schritt = Stufe::Fertig;
            self.uebergang_schreiben(p, u)?;
            haken(Schritt::UebergangFertig)?;
        }
        match fs::remove_file(p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(uebergang_fehler()),
            _ => Ok(()),
        }
    }

    /// Aktuelle Kennung zu einer (womöglich alten) Kennung, z. B. aus einem Deep Link.
    pub fn aufloesen(&self, id: &str) -> Res<String> {
        let _g = sperren();
        let st = self.laden()?;
        Ok(st.aufgeloest(id))
    }

    /// Offene Übergangsjournale (Prüfungen).
    pub fn uebergaenge_offen(&self) -> usize {
        self.uebergaenge_lesen().len()
    }
}
