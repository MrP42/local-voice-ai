//! In-Memory-Hub fuer Tests: setzt die Hub-Regeln aus E3.0 bis E3.3 und E3.8 um, ohne Netz.
//!
//! Der Hub sieht nur Steuerfelder, Chiffrat, Verzeichnis und gewrappte Schluessel. Alle Zugriffe laufen
//! unter einem Mutex; dadurch gewinnt bei gleichzeitigem Vergleichsschreiben genau ein Schreiber.
//!
//! Mit [`MemHub::boese`] spielt der Hub einen Angreifer (E3.8): Durchsetzung aus, Umsortieren,
//! Wiederholen, alte Fassungen veraenderlicher Objekte mit neuer Revision, Vorspulen des Cursors,
//! Zurueckhalten, Einschleusen von Verzeichniseintraegen und Schluesselpaketen. Mit
//! [`MemHub::schnappschuss`] und [`MemHub::zuruecksetzen`] rollt er seinen ganzen Stand zurueck.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use super::hub::{AuthGeraet, HubFehler, HubSpeicher, LeseFilter, Schreiben};
use super::kennung::Hex128;
use super::paket::Schluesselpaket;
use super::umschlag::{Sammlung, Umschlag, ZielTyp};
use super::verzeichnis::{Verzeichnis, Verzeichniseintrag};

#[derive(Clone, Default)]
struct Zustand {
    revision: u64,
    zugelassen: BTreeSet<String>,
    objekte: BTreeMap<(Sammlung, Hex128), Umschlag>,
    /// Alle Fassungen veraenderlicher Objekte (fuer den Angriff "alte Fassung").
    fassungen: BTreeMap<(Sammlung, Hex128), Vec<Umschlag>>,
    grab_unterhaltungen: BTreeSet<Hex128>,
    grab_objekte: BTreeSet<Hex128>,
    schreibversuche: u64,
    verzeichnis: Option<Verzeichnis>,
    pakete: Vec<Schluesselpaket>,
}

impl Zustand {
    fn epoche(&self) -> u64 {
        self.verzeichnis.as_ref().map_or(1, |v| v.epoche().max(1))
    }

    fn widerrufen(&self, geraet: &str) -> bool {
        self.verzeichnis
            .as_ref()
            .and_then(|v| v.stand().mitglied(geraet))
            .is_some_and(|m| !m.zugelassen())
    }

    fn zugelassen(&self, geraet: &str) -> bool {
        self.zugelassen.contains(geraet)
            || self
                .verzeichnis
                .as_ref()
                .is_some_and(|v| v.stand().zugelassen(geraet))
    }
}

/// Angriffe des Hubs (E3.8). Standard: ehrlich.
#[derive(Clone, Debug, Default)]
pub struct Boese {
    /// Keine Hubregel wird durchgesetzt (Widerrufene, alte Epochen, fremde `geraet_id`, Grabsteine).
    pub ohne_durchsetzung: bool,
    /// `lesen` liefert in umgekehrter Reihenfolge.
    pub umsortieren: bool,
    /// `lesen` liefert jedes Objekt doppelt.
    pub wiederholen: bool,
    /// Veraenderliche Objekte kommen in ihrer ersten Fassung, aber mit der aktuellen Revision.
    pub alte_fassung: bool,
    /// Auf jede gelieferte Revision wird dieser Wert aufgeschlagen (Cursor vorspulen).
    pub revision_aufschlag: u64,
    /// Diese Objekte werden nie geliefert.
    pub zurueckhalten: BTreeSet<Hex128>,
    /// Verzeichnis wird nur bis zu dieser Laenge geliefert (Widerruf vorenthalten).
    pub verzeichnis_bis: Option<u64>,
    /// Wird an jede Verzeichnisantwort angehaengt (eingeschleuste Eintraege).
    pub verzeichnis_zusatz: Vec<Verzeichniseintrag>,
    /// Wird an jede Paketantwort angehaengt (eingeschleuste Schluessel).
    pub pakete_zusatz: Vec<Schluesselpaket>,
    /// `fassungen_lesen` liefert keine Zwischenbelege (Ketten-Luecke erzwingen).
    pub ohne_fassungen: bool,
}

/// Gesicherter Hubstand fuer den Rollback-Angriff.
#[derive(Clone)]
pub struct Schnappschuss(Zustand);

/// Hub im Arbeitsspeicher.
#[derive(Default)]
pub struct MemHub {
    z: Mutex<Zustand>,
    boese: Mutex<Boese>,
}

impl MemHub {
    /// Leerer Hub ohne zugelassene Geraete (Zulassung ueber das Verzeichnis oder [`MemHub::zulassen`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Laesst ein Geraet auf Transportebene zu (nur fuer Tests ohne Verzeichnis).
    pub fn zulassen(&self, geraet: &str) {
        self.sperre().zugelassen.insert(geraet.to_owned());
    }

    fn sperre(&self) -> MutexGuard<'_, Zustand> {
        self.z.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn b(&self) -> Boese {
        self.boese.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Setzt das Angriffsverhalten.
    pub fn boese(&self, b: Boese) {
        *self.boese.lock().unwrap_or_else(|e| e.into_inner()) = b;
    }

    /// Wieder ehrlich.
    pub fn ehrlich(&self) {
        self.boese(Boese::default());
    }

    pub fn schnappschuss(&self) -> Schnappschuss {
        Schnappschuss(self.sperre().clone())
    }

    /// Spielt einen frueheren Stand zurueck (Rollback-Angriff).
    pub fn zuruecksetzen(&self, s: &Schnappschuss) {
        *self.sperre() = s.0.clone();
    }

    /// Anzahl aller Schreibversuche (`schreiben` und `cas`), auch abgewiesener.
    pub fn schreibversuche(&self) -> u64 {
        self.sperre().schreibversuche
    }

    /// Alle gespeicherten Umschlaege (Sicht des Hubs, z. B. fuer die Klartextsuche).
    pub fn alle_umschlaege(&self) -> Vec<Umschlag> {
        self.sperre().objekte.values().cloned().collect()
    }

    /// Alle Fassungen aller veraenderlichen Objekte (fuer Wiedereinspielen).
    pub fn alle_fassungen(&self) -> Vec<Umschlag> {
        self.sperre()
            .fassungen
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    pub fn anzahl(&self, sammlung: Sammlung) -> usize {
        self.sperre()
            .objekte
            .keys()
            .filter(|(s, _)| *s == sammlung)
            .count()
    }

    /// Anzahl gespeicherter Objekte einer Unterhaltung (ohne Loeschmarken).
    pub fn anzahl_je_unterhaltung(&self, r: &Hex128) -> usize {
        self.sperre()
            .objekte
            .values()
            .filter(|u| u.sammlung != Sammlung::Loeschmarke && &u.unterhaltung_ref == r)
            .count()
    }

    /// Lesen ohne Geraetepruefung: das, was ein Angreifer mit Hubzugang sieht.
    pub fn roh_lesen(&self, nach_revision: u64) -> Vec<Umschlag> {
        let z = self.sperre();
        sortiert(
            z.objekte
                .values()
                .filter(|u| u.revision.unwrap_or(0) > nach_revision),
        )
    }

    /// Gespeicherte Verzeichniseintraege (Hubsicht).
    pub fn verzeichnis(&self) -> Vec<Verzeichniseintrag> {
        self.sperre()
            .verzeichnis
            .as_ref()
            .map(|v| v.eintraege().to_vec())
            .unwrap_or_default()
    }

    /// Gespeicherte Schluesselpakete (Hubsicht).
    pub fn pakete(&self) -> Vec<Schluesselpaket> {
        self.sperre().pakete.clone()
    }

    /// Aktuelle Epoche laut Verzeichnis (Hubsicht).
    pub fn epoche(&self) -> u64 {
        self.sperre().epoche()
    }

    /// Schreibt ohne jede Pruefung (Angriff: eingeschleustes Objekt).
    pub fn einschleusen(&self, u: &Umschlag) -> u64 {
        Self::speichern(&mut self.sperre(), u)
    }

    fn pruefen_auth(&self, z: &Zustand, auth: &AuthGeraet) -> Result<(), HubFehler> {
        if self.b().ohne_durchsetzung {
            return Ok(());
        }
        if z.widerrufen(auth.id()) {
            return Err(HubFehler::Widerrufen);
        }
        if !z.zugelassen(auth.id()) {
            return Err(HubFehler::NichtZugelassen);
        }
        Ok(())
    }

    fn pruefen_schreiben(
        &self,
        z: &Zustand,
        auth: &AuthGeraet,
        u: &Umschlag,
    ) -> Result<(), HubFehler> {
        self.pruefen_auth(z, auth)?;
        if self.b().ohne_durchsetzung {
            return Ok(());
        }
        if u.geraet_id != auth.id() {
            return Err(HubFehler::GeraetStimmtNichtUeberein);
        }
        u.pruefen().map_err(|_| HubFehler::Vertrag("umschlag"))?;
        let epoche = z.epoche();
        if u.key_epoch < epoche {
            return Err(HubFehler::EpocheVeraltet { mindest: epoche });
        }
        if u.key_epoch > epoche {
            return Err(HubFehler::EpocheVoraus { aktuell: epoche });
        }
        if u.sammlung != Sammlung::Loeschmarke
            && (z.grab_unterhaltungen.contains(&u.unterhaltung_ref)
                || z.grab_objekte.contains(&u.objekt_id))
        {
            return Err(HubFehler::GrabsteinAbgewiesen);
        }
        Ok(())
    }

    fn speichern(z: &mut Zustand, u: &Umschlag) -> u64 {
        z.revision += 1;
        let rev = z.revision;
        if let (Sammlung::Loeschmarke, Some(ziel)) = (u.sammlung, &u.ziel) {
            match ziel.typ {
                ZielTyp::Unterhaltung => {
                    z.grab_unterhaltungen.insert(ziel.wert.clone());
                    z.objekte.retain(|(s, _), o| {
                        *s == Sammlung::Loeschmarke || o.unterhaltung_ref != ziel.wert
                    });
                }
                ZielTyp::Objekt => {
                    z.grab_objekte.insert(ziel.wert.clone());
                    z.objekte
                        .retain(|(s, id), _| *s == Sammlung::Loeschmarke || *id != ziel.wert);
                }
            }
        }
        let mit = u.mit_revision(rev);
        if u.sammlung.veraenderlich() {
            z.fassungen
                .entry((u.sammlung, u.objekt_id.clone()))
                .or_default()
                .push(mit.clone());
        }
        z.objekte.insert((u.sammlung, u.objekt_id.clone()), mit);
        rev
    }

    /// Wendet Lese-Angriffe auf ein einzelnes Objekt an.
    fn fassung(&self, z: &Zustand, u: &Umschlag, b: &Boese) -> Umschlag {
        let mut aus = u.clone();
        if b.alte_fassung
            && let Some(erste) = z
                .fassungen
                .get(&(u.sammlung, u.objekt_id.clone()))
                .and_then(|f| f.first())
        {
            aus = erste.mit_revision(u.revision.unwrap_or(0));
        }
        if b.revision_aufschlag > 0 {
            aus.revision = aus.revision.map(|r| r.saturating_add(b.revision_aufschlag));
        }
        aus
    }
}

fn sortiert<'a>(it: impl Iterator<Item = &'a Umschlag>) -> Vec<Umschlag> {
    let mut v: Vec<Umschlag> = it.cloned().collect();
    v.sort_by_key(|u| u.revision);
    v
}

impl HubSpeicher for MemHub {
    fn schreiben(&self, auth: &AuthGeraet, u: &Umschlag) -> Result<Schreiben, HubFehler> {
        let mut z = self.sperre();
        z.schreibversuche += 1;
        self.pruefen_schreiben(&z, auth, u)?;
        if let Some(alt) = z.objekte.get(&(u.sammlung, u.objekt_id.clone())) {
            let rev = alt.revision.unwrap_or(0);
            return if u.sammlung.veraenderlich() {
                Err(HubFehler::RevisionKonflikt { aktuell: Some(rev) })
            } else {
                Ok(Schreiben::Vorhanden(rev))
            };
        }
        Ok(Schreiben::Neu(Self::speichern(&mut z, u)))
    }

    fn cas(
        &self,
        auth: &AuthGeraet,
        sammlung: Sammlung,
        objekt_id: &Hex128,
        erwartete_revision: Option<u64>,
        neuer: &Umschlag,
    ) -> Result<u64, HubFehler> {
        let mut z = self.sperre();
        z.schreibversuche += 1;
        if !sammlung.veraenderlich() {
            return Err(HubFehler::NichtVeraenderlich);
        }
        if neuer.sammlung != sammlung || &neuer.objekt_id != objekt_id {
            return Err(HubFehler::Vertrag("sammlung/objekt_id"));
        }
        self.pruefen_schreiben(&z, auth, neuer)?;
        let aktuell = z.objekte.get(&(sammlung, objekt_id.clone()));
        let aktuelle_revision = aktuell.and_then(|o| o.revision);
        if aktuelle_revision != erwartete_revision && !self.b().ohne_durchsetzung {
            return Err(HubFehler::RevisionKonflikt {
                aktuell: aktuelle_revision,
            });
        }
        if aktuell.is_some_and(|o| o.unterhaltung_ref != neuer.unterhaltung_ref) {
            return Err(HubFehler::Vertrag(
                "unterhaltung_ref darf sich nicht aendern",
            ));
        }
        Ok(Self::speichern(&mut z, neuer))
    }

    fn holen(
        &self,
        auth: &AuthGeraet,
        sammlung: Sammlung,
        objekt_id: &Hex128,
    ) -> Result<Option<Umschlag>, HubFehler> {
        let z = self.sperre();
        self.pruefen_auth(&z, auth)?;
        let b = self.b();
        if b.zurueckhalten.contains(objekt_id) {
            return Ok(None);
        }
        Ok(z.objekte
            .get(&(sammlung, objekt_id.clone()))
            .map(|u| self.fassung(&z, u, &b)))
    }

    fn fassungen_lesen(
        &self,
        auth: &AuthGeraet,
        sammlung: Sammlung,
        objekt_id: &Hex128,
    ) -> Result<Vec<Umschlag>, HubFehler> {
        let z = self.sperre();
        self.pruefen_auth(&z, auth)?;
        if self.b().ohne_fassungen {
            return Ok(Vec::new());
        }
        Ok(z.fassungen
            .get(&(sammlung, objekt_id.clone()))
            .cloned()
            .unwrap_or_default())
    }

    fn lesen(&self, auth: &AuthGeraet, filter: &LeseFilter) -> Result<Vec<Umschlag>, HubFehler> {
        let z = self.sperre();
        self.pruefen_auth(&z, auth)?;
        let b = self.b();
        let mut v: Vec<Umschlag> = sortiert(z.objekte.values().filter(|u| {
            u.revision.unwrap_or(0) > filter.nach_revision
                && filter.sammlung.is_none_or(|s| s == u.sammlung)
                && filter
                    .unterhaltung_ref
                    .as_ref()
                    .is_none_or(|r| r == &u.unterhaltung_ref)
        }))
        .into_iter()
        .filter(|u| !b.zurueckhalten.contains(&u.objekt_id))
        .map(|u| self.fassung(&z, &u, &b))
        .collect();
        if b.wiederholen {
            v = v.iter().flat_map(|u| [u.clone(), u.clone()]).collect();
        }
        if b.umsortieren {
            v.reverse();
        }
        Ok(v)
    }

    fn verzeichnis_anhaengen(
        &self,
        auth: &AuthGeraet,
        e: &Verzeichniseintrag,
    ) -> Result<(), HubFehler> {
        let mut z = self.sperre();
        let laenge = z.verzeichnis.as_ref().map_or(0, |v| v.laenge());
        let gruendung = laenge == 0 && e.nummer == 0 && e.unterzeichner == auth.id();
        if !gruendung {
            self.pruefen_auth(&z, auth)?;
        }
        if e.nummer > laenge {
            return Err(HubFehler::VerzeichnisKonflikt { laenge });
        }
        let mut v = z
            .verzeichnis
            .clone()
            .unwrap_or_else(|| Verzeichnis::gepinnt(e.konto.clone()));
        match v.anhaengen(e.clone()) {
            Ok(_) => {
                z.verzeichnis = Some(v);
                Ok(())
            }
            Err(super::fehler::SyncFehler::VerzeichnisGabelung) => {
                Err(HubFehler::VerzeichnisKonflikt { laenge })
            }
            Err(_) => Err(HubFehler::Vertrag("verzeichniseintrag")),
        }
    }

    fn verzeichnis_lesen(
        &self,
        auth: &AuthGeraet,
        ab: u64,
    ) -> Result<Vec<Verzeichniseintrag>, HubFehler> {
        let z = self.sperre();
        self.pruefen_auth(&z, auth)?;
        let b = self.b();
        let mut v: Vec<Verzeichniseintrag> = z
            .verzeichnis
            .as_ref()
            .map(|v| v.eintraege().to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.nummer >= ab && b.verzeichnis_bis.is_none_or(|bis| e.nummer < bis))
            .collect();
        v.extend(b.verzeichnis_zusatz.iter().cloned());
        Ok(v)
    }

    fn pakete_ablegen(
        &self,
        auth: &AuthGeraet,
        pakete: &[Schluesselpaket],
    ) -> Result<(), HubFehler> {
        let mut z = self.sperre();
        self.pruefen_auth(&z, auth)?;
        for p in pakete {
            if p.absender != auth.id() && !self.b().ohne_durchsetzung {
                return Err(HubFehler::GeraetStimmtNichtUeberein);
            }
            if !z.pakete.contains(p) {
                z.pakete.push(p.clone());
            }
        }
        Ok(())
    }

    fn pakete_lesen(&self, auth: &AuthGeraet) -> Result<Vec<Schluesselpaket>, HubFehler> {
        let z = self.sperre();
        self.pruefen_auth(&z, auth)?;
        let mut v: Vec<Schluesselpaket> = z
            .pakete
            .iter()
            .filter(|p| p.empfaenger == auth.id())
            .cloned()
            .collect();
        v.extend(
            self.b()
                .pakete_zusatz
                .iter()
                .filter(|p| p.empfaenger == auth.id())
                .cloned(),
        );
        Ok(v)
    }
}
