//! In-Memory-Hub fuer Tests: setzt die Hub-Regeln aus E3.0 bis E3.3 um, ohne Netz.
//!
//! Der Hub sieht nur Steuerfelder und Chiffrat. Alle Zugriffe laufen unter einem Mutex; dadurch gewinnt
//! bei gleichzeitigem Vergleichsschreiben genau ein Schreiber.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use super::hub::{AuthGeraet, HubFehler, HubSpeicher, LeseFilter, Schreiben};
use super::kennung::Hex128;
use super::umschlag::{Sammlung, Umschlag, ZielTyp};

#[derive(Default)]
struct Zustand {
    revision: u64,
    epoche: u64,
    zugelassen: BTreeSet<String>,
    objekte: BTreeMap<(Sammlung, Hex128), Umschlag>,
    widerrufen: BTreeSet<String>,
    mindest_epoche: u64,
    grab_unterhaltungen: BTreeSet<Hex128>,
    grab_objekte: BTreeSet<Hex128>,
    schreibversuche: u64,
    vorgaenge: BTreeMap<Hex128, (String, u64)>,
}

/// Hub im Arbeitsspeicher.
#[derive(Default)]
pub struct MemHub {
    z: Mutex<Zustand>,
}

impl MemHub {
    /// Leerer Hub in Epoche 1 ohne zugelassene Geraete (siehe [`MemHub::zulassen`]).
    pub fn new() -> Self {
        let hub = Self::default();
        hub.sperre().epoche = 1;
        hub
    }

    /// Laesst ein Geraet zu (Kopplung); nur zugelassene Geraete schreiben und lesen.
    pub fn zulassen(&self, geraet: &str) {
        self.sperre().zugelassen.insert(geraet.to_owned());
    }

    fn sperre(&self) -> MutexGuard<'_, Zustand> {
        self.z.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Anzahl aller Schreibversuche (`schreiben` und `cas`), auch abgewiesener.
    pub fn schreibversuche(&self) -> u64 {
        self.sperre().schreibversuche
    }

    /// Alle gespeicherten Umschlaege (Sicht des Hubs, z. B. fuer die Klartextsuche).
    pub fn alle_umschlaege(&self) -> Vec<Umschlag> {
        self.sperre().objekte.values().cloned().collect()
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

    fn pruefen_auth(z: &Zustand, auth: &AuthGeraet) -> Result<(), HubFehler> {
        if z.widerrufen.contains(auth.id()) {
            return Err(HubFehler::Widerrufen);
        }
        if !z.zugelassen.contains(auth.id()) {
            return Err(HubFehler::NichtZugelassen);
        }
        Ok(())
    }

    fn pruefen_schreiben(z: &Zustand, auth: &AuthGeraet, u: &Umschlag) -> Result<(), HubFehler> {
        Self::pruefen_auth(z, auth)?;
        if u.geraet_id != auth.id() {
            return Err(HubFehler::GeraetStimmtNichtUeberein);
        }
        u.pruefen().map_err(|_| HubFehler::Vertrag("umschlag"))?;
        if u.key_epoch < z.mindest_epoche {
            return Err(HubFehler::EpocheVeraltet {
                mindest: z.mindest_epoche,
            });
        }
        if u.key_epoch > z.epoche {
            return Err(HubFehler::EpocheVoraus { aktuell: z.epoche });
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
        z.objekte
            .insert((u.sammlung, u.objekt_id.clone()), u.mit_revision(rev));
        rev
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
        Self::pruefen_schreiben(&z, auth, u)?;
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
        Self::pruefen_schreiben(&z, auth, neuer)?;
        let aktuell = z.objekte.get(&(sammlung, objekt_id.clone()));
        let aktuelle_revision = aktuell.and_then(|o| o.revision);
        if aktuelle_revision != erwartete_revision {
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
        Self::pruefen_auth(&z, auth)?;
        Ok(z.objekte.get(&(sammlung, objekt_id.clone())).cloned())
    }

    fn lesen(&self, auth: &AuthGeraet, filter: &LeseFilter) -> Result<Vec<Umschlag>, HubFehler> {
        let z = self.sperre();
        Self::pruefen_auth(&z, auth)?;
        Ok(sortiert(z.objekte.values().filter(|u| {
            u.revision.unwrap_or(0) > filter.nach_revision
                && filter.sammlung.is_none_or(|s| s == u.sammlung)
                && filter
                    .unterhaltung_ref
                    .as_ref()
                    .is_none_or(|r| r == &u.unterhaltung_ref)
        })))
    }

    fn widerrufen(
        &self,
        auth: &AuthGeraet,
        widerrufenes_geraet: &str,
        erwartete_epoche: u64,
        vorgang: &Hex128,
    ) -> Result<u64, HubFehler> {
        let mut z = self.sperre();
        Self::pruefen_auth(&z, auth)?;
        if let Some((geraet, epoche)) = z.vorgaenge.get(vorgang) {
            return if geraet == widerrufenes_geraet {
                Ok(*epoche)
            } else {
                Err(HubFehler::Vertrag(
                    "vorgang gehoert zu einem anderen widerruf",
                ))
            };
        }
        if z.epoche != erwartete_epoche {
            return Err(HubFehler::EpocheKonflikt { aktuell: z.epoche });
        }
        let neu = z
            .epoche
            .checked_add(1)
            .ok_or(HubFehler::Vertrag("epoche ueberlaeuft"))?;
        z.widerrufen.insert(widerrufenes_geraet.to_owned());
        z.epoche = neu;
        z.mindest_epoche = neu;
        z.vorgaenge
            .insert(vorgang.clone(), (widerrufenes_geraet.to_owned(), neu));
        Ok(neu)
    }

    fn epoche(&self, auth: &AuthGeraet) -> Result<u64, HubFehler> {
        let z = self.sperre();
        Self::pruefen_auth(&z, auth)?;
        Ok(z.epoche)
    }

    fn zugelassene(&self, auth: &AuthGeraet) -> Result<Vec<String>, HubFehler> {
        let z = self.sperre();
        Self::pruefen_auth(&z, auth)?;
        Ok(z.zugelassen.difference(&z.widerrufen).cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::schluessel::Schluesselbund;
    use crate::sync::umschlag::{Steuerfelder, Ziel, versiegeln};
    use serde_json::json;

    fn bund() -> Schluesselbund {
        Schluesselbund::zufaellig().unwrap()
    }

    fn hub() -> MemHub {
        let h = MemHub::new();
        for g in ["pc", "tablet", "iphone"] {
            h.zulassen(g);
        }
        h
    }

    fn a(n: &str) -> AuthGeraet {
        AuthGeraet::new(n)
    }

    fn objekt(
        b: &Schluesselbund,
        s: Sammlung,
        id: &Hex128,
        r: &Hex128,
        geraet: &str,
        epoche: u64,
    ) -> Umschlag {
        versiegeln(
            b,
            &Steuerfelder::inhalt(s, id.clone(), r.clone(), geraet, epoche),
            &json!({"x": 1}),
        )
        .unwrap()
    }

    fn zufall() -> Hex128 {
        Hex128::zufall().unwrap()
    }

    #[test]
    fn erstschreiben_ist_idempotent_und_revision_steigt() {
        let (hub, b) = (hub(), bund());
        let (id, r) = (zufall(), zufall());
        let u = objekt(&b, Sammlung::Beitrag, &id, &r, "pc", 1);
        assert_eq!(hub.schreiben(&a("pc"), &u), Ok(Schreiben::Neu(1)));
        assert_eq!(hub.schreiben(&a("pc"), &u), Ok(Schreiben::Vorhanden(1)));
        let u2 = objekt(&b, Sammlung::Beitrag, &zufall(), &r, "pc", 1);
        assert_eq!(hub.schreiben(&a("pc"), &u2), Ok(Schreiben::Neu(2)));
        assert_eq!(hub.anzahl(Sammlung::Beitrag), 2);
        assert_eq!(hub.schreibversuche(), 3);
    }

    #[test]
    fn schreiber_wird_authentifiziert() {
        let (hub, b) = (hub(), bund());
        let r = zufall();
        let als_pc = objekt(&b, Sammlung::Beitrag, &zufall(), &r, "pc", 1);
        assert_eq!(
            hub.schreiben(&a("tablet"), &als_pc),
            Err(HubFehler::GeraetStimmtNichtUeberein),
            "fremde geraet_id im Umschlag"
        );
        assert_eq!(
            hub.schreiben(&a("fremd"), &als_pc),
            Err(HubFehler::NichtZugelassen)
        );
        let fremd = objekt(&b, Sammlung::Beitrag, &zufall(), &r, "fremd", 1);
        assert_eq!(
            hub.schreiben(&a("fremd"), &fremd),
            Err(HubFehler::NichtZugelassen)
        );
        assert_eq!(
            hub.lesen(&a("fremd"), &LeseFilter::default()),
            Err(HubFehler::NichtZugelassen)
        );
        assert_eq!(hub.epoche(&a("fremd")), Err(HubFehler::NichtZugelassen));
        assert_eq!(hub.anzahl(Sammlung::Beitrag), 0);
    }

    #[test]
    fn vergleichsschreiben_nur_mit_passender_revision() {
        let (hub, b) = (hub(), bund());
        let (id, r) = (zufall(), zufall());
        let u = objekt(&b, Sammlung::Sitzung, &id, &r, "pc", 1);
        let pc = a("pc");
        let falsch = hub.cas(&pc, Sammlung::Sitzung, &id, Some(5), &u);
        assert_eq!(falsch, Err(HubFehler::RevisionKonflikt { aktuell: None }));
        let rev = hub.cas(&pc, Sammlung::Sitzung, &id, None, &u).unwrap();
        let doppelt = hub.cas(&pc, Sammlung::Sitzung, &id, None, &u);
        assert_eq!(
            doppelt,
            Err(HubFehler::RevisionKonflikt { aktuell: Some(rev) })
        );
        let zu_neu = hub.cas(&pc, Sammlung::Sitzung, &id, Some(rev + 1), &u);
        assert_eq!(
            zu_neu,
            Err(HubFehler::RevisionKonflikt { aktuell: Some(rev) })
        );
        let neu = hub.cas(&pc, Sammlung::Sitzung, &id, Some(rev), &u).unwrap();
        assert!(neu > rev);
        assert_eq!(
            hub.schreiben(&pc, &u),
            Err(HubFehler::RevisionKonflikt { aktuell: Some(neu) })
        );
        let umgehaengt = objekt(&b, Sammlung::Sitzung, &id, &zufall(), "pc", 1);
        assert!(matches!(
            hub.cas(&pc, Sammlung::Sitzung, &id, Some(neu), &umgehaengt),
            Err(HubFehler::Vertrag(_))
        ));
        let beitrag = objekt(&b, Sammlung::Beitrag, &id, &r, "pc", 1);
        assert_eq!(
            hub.cas(&pc, Sammlung::Beitrag, &id, None, &beitrag),
            Err(HubFehler::NichtVeraenderlich)
        );
    }

    #[test]
    fn widerruf_ist_vergleichsschreiben_auf_die_epoche() {
        let (hub, b) = (hub(), bund());
        let r = zufall();
        hub.schreiben(
            &a("tablet"),
            &objekt(&b, Sammlung::Beitrag, &zufall(), &r, "tablet", 1),
        )
        .unwrap();
        assert_eq!(
            hub.widerrufen(&a("pc"), "tablet", 7, &zufall()),
            Err(HubFehler::EpocheKonflikt { aktuell: 1 })
        );
        assert_eq!(hub.widerrufen(&a("pc"), "tablet", 1, &zufall()), Ok(2));
        assert_eq!(
            hub.widerrufen(&a("iphone"), "uhr", 1, &zufall()),
            Err(HubFehler::EpocheKonflikt { aktuell: 2 }),
            "gleichzeitiger zweiter Widerruf bekommt nie dieselbe Nummer"
        );
        let mut b2 = b.clone();
        b2.epoche_aufnehmen(2, crate::sync::schluessel::Schluessel::zufall().unwrap())
            .unwrap();
        let neu_tablet = objekt(&b2, Sammlung::Beitrag, &zufall(), &r, "tablet", 2);
        assert_eq!(
            hub.schreiben(&a("tablet"), &neu_tablet),
            Err(HubFehler::Widerrufen)
        );
        assert_eq!(
            hub.lesen(&a("tablet"), &LeseFilter::default()),
            Err(HubFehler::Widerrufen)
        );
        assert_eq!(
            hub.holen(&a("tablet"), Sammlung::Beitrag, &r),
            Err(HubFehler::Widerrufen)
        );
        let alt = objekt(&b, Sammlung::Beitrag, &zufall(), &r, "pc", 1);
        assert_eq!(
            hub.schreiben(&a("pc"), &alt),
            Err(HubFehler::EpocheVeraltet { mindest: 2 })
        );
        let ok = objekt(&b2, Sammlung::Beitrag, &zufall(), &r, "pc", 2);
        assert!(hub.schreiben(&a("pc"), &ok).is_ok());
        b2.epoche_aufnehmen(3, crate::sync::schluessel::Schluessel::zufall().unwrap())
            .unwrap();
        let zu_neu = objekt(&b2, Sammlung::Beitrag, &zufall(), &r, "pc", 3);
        assert_eq!(
            hub.schreiben(&a("pc"), &zu_neu),
            Err(HubFehler::EpocheVoraus { aktuell: 2 })
        );
        assert_eq!(
            hub.widerrufen(&a("tablet"), "pc", 2, &zufall()),
            Err(HubFehler::Widerrufen),
            "Widerrufene widerrufen niemanden"
        );
        assert_eq!(
            hub.widerrufen(&a("fremd"), "pc", 2, &zufall()),
            Err(HubFehler::NichtZugelassen)
        );
        let mut liste = hub.zugelassene(&a("pc")).unwrap();
        liste.sort();
        assert_eq!(
            liste,
            vec!["iphone", "pc"],
            "widerrufene stehen nicht in der Empfaengerliste"
        );
    }

    #[test]
    fn widerruf_mit_gleicher_vorgangskennung_ist_idempotent() {
        let hub = hub();
        let v = zufall();
        assert_eq!(hub.widerrufen(&a("pc"), "tablet", 1, &v), Ok(2));
        assert_eq!(
            hub.widerrufen(&a("pc"), "tablet", 1, &v),
            Ok(2),
            "verlorene Antwort: Wiederholung liefert dieselbe Epoche"
        );
        assert_eq!(hub.epoche(&a("pc")), Ok(2), "kein zweiter Epochensprung");
        assert!(matches!(
            hub.widerrufen(&a("pc"), "iphone", 1, &v),
            Err(HubFehler::Vertrag(_))
        ));
        assert_eq!(
            hub.widerrufen(&a("iphone"), "uhr", 1, &zufall()),
            Err(HubFehler::EpocheKonflikt { aktuell: 2 }),
            "andere Kennung bleibt ein Konflikt"
        );
    }

    #[test]
    fn grabstein_loescht_und_weist_spaetere_objekte_ab() {
        let (hub, b) = (hub(), bund());
        let (r, andere) = (zufall(), zufall());
        hub.schreiben(
            &a("pc"),
            &objekt(&b, Sammlung::Beitrag, &zufall(), &r, "pc", 1),
        )
        .unwrap();
        hub.schreiben(
            &a("pc"),
            &objekt(&b, Sammlung::Beitrag, &zufall(), &andere, "pc", 1),
        )
        .unwrap();
        let grab = versiegeln(
            &b,
            &Steuerfelder::loeschmarke(
                zufall(),
                r.clone(),
                "pc",
                1,
                Ziel {
                    typ: ZielTyp::Unterhaltung,
                    wert: r.clone(),
                },
            ),
            &json!({}),
        )
        .unwrap();
        hub.schreiben(&a("pc"), &grab).unwrap();
        assert_eq!(hub.anzahl_je_unterhaltung(&r), 0);
        assert_eq!(
            hub.anzahl_je_unterhaltung(&andere),
            1,
            "andere Unterhaltungen bleiben"
        );
        let spaeter = objekt(&b, Sammlung::Beitrag, &zufall(), &r, "tablet", 1);
        assert_eq!(
            hub.schreiben(&a("tablet"), &spaeter),
            Err(HubFehler::GrabsteinAbgewiesen)
        );
        let u = objekt(&b, Sammlung::Sitzung, &zufall(), &r, "tablet", 1);
        assert_eq!(
            hub.cas(&a("tablet"), Sammlung::Sitzung, &u.objekt_id, None, &u),
            Err(HubFehler::GrabsteinAbgewiesen)
        );
        assert_eq!(hub.anzahl(Sammlung::Loeschmarke), 1, "Grabsteine bleiben");
    }

    #[test]
    fn lesen_filtert_nach_revision_unterhaltung_und_sammlung() {
        let (hub, b) = (hub(), bund());
        let (r1, r2) = (zufall(), zufall());
        for (r, s) in [
            (&r1, Sammlung::Beitrag),
            (&r2, Sammlung::Beitrag),
            (&r1, Sammlung::Hinweis),
        ] {
            hub.schreiben(&a("pc"), &objekt(&b, s, &zufall(), r, "pc", 1))
                .unwrap();
        }
        let pc = a("pc");
        assert_eq!(hub.lesen(&pc, &LeseFilter::default()).unwrap().len(), 3);
        let nach = LeseFilter {
            nach_revision: 1,
            ..Default::default()
        };
        assert_eq!(hub.lesen(&pc, &nach).unwrap().len(), 2);
        let nur_r1 = LeseFilter {
            unterhaltung_ref: Some(r1.clone()),
            ..Default::default()
        };
        assert_eq!(hub.lesen(&pc, &nur_r1).unwrap().len(), 2);
        let hinweise = LeseFilter {
            sammlung: Some(Sammlung::Hinweis),
            ..Default::default()
        };
        assert_eq!(hub.lesen(&pc, &hinweise).unwrap().len(), 1);
        let revs: Vec<_> = hub
            .lesen(&pc, &LeseFilter::default())
            .unwrap()
            .iter()
            .map(|u| u.revision.unwrap())
            .collect();
        assert_eq!(revs, vec![1, 2, 3], "aufsteigend");
    }

    #[test]
    fn vertragsverletzung_wird_abgewiesen() {
        let (hub, b) = (hub(), bund());
        let mut u = objekt(&b, Sammlung::Beitrag, &zufall(), &zufall(), "pc", 1);
        u.art = Sammlung::Hinweis;
        assert!(matches!(
            hub.schreiben(&a("pc"), &u),
            Err(HubFehler::Vertrag(_))
        ));
        assert_eq!(hub.anzahl(Sammlung::Beitrag), 0);
    }
}
