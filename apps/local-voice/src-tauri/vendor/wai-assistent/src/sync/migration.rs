//! Migration bestehender lokaler Unterhaltungen (ADR-0022, ADR-0028 E11).
//!
//! - **Stabile IDs:** `unterhaltung_ref` aus (Datenbestand-ID, lokale ID), Hub-Objekt-IDs aus (`unterhaltung_ref`, Beitrags-ID)
//!   ([`super::kennung::Hex128::abgeleitet`]), nicht neu gewuerfelt. Die Datenbestand-ID gibt der Aufrufer
//!   vor und speichert sie in der lokalen Datenbank (nicht die Geraete-ID: Neuinstallation oder neue
//!   Geraete-ID derselben Altdaten ergibt keine Dopplung). Wiederholung erzeugt dieselben Objekte
//!   (Hub-Idempotenz); lokale IDs verschiedener Datenbestaende oder Unterhaltungen kollidieren nie.
//! - **Keine erfundenen Angaben:** Fehlende Zeit-, Modell-, Modus- oder Autorangaben werden `unbekannt`;
//!   fehlende Datenklasse gilt fail-closed als `restricted`; Altbeitraege tragen Generation 0.
//! - **Wiederholbar und idempotent:** Die Eingabe wird nie veraendert (Rueckweg: Altdaten bleiben bis zur
//!   Bestaetigung erhalten; die Sicherung vor dem Lauf ist Sache des Aufrufers). Ein zweiter Lauf
//!   dupliziert nichts und ueberschreibt lokale Aenderungen nicht.
//! - **Migrationsmarke je Geraet** haelt Version und migrierte lokale IDs.
//! - Die Migration laedt nichts hoch; Hochladen erst nach der Sync-Freigabe (E3.6).

use std::collections::BTreeSet;

use super::abgleich::{Abgleich, Kopf, Unterhaltung};
use super::freigabe::Einschraenkung;
use super::verlauf::{Beitrag, Datenklasse, Einfuegen, Modus};

/// Version des Migrationsverfahrens.
pub const MIGRATIONSVERSION: u32 = 1;

const UNBEKANNT: &str = "unbekannt";

/// Eintrag einer bestehenden lokalen Unterhaltung (`conversations.db` bzw. iOS `Entry`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AltEintrag {
    pub id: String,
    /// Elternbeitrag; fehlt er, gilt der vorherige Eintrag der Liste (lineares Altgespraech).
    pub eltern_id: Option<String>,
    pub autor: Option<String>,
    pub modus: Option<Modus>,
    pub text: String,
    pub zeit: Option<String>,
    pub modell: Option<String>,
    pub datenklasse: Option<Datenklasse>,
}

/// Bestehende lokale Unterhaltung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AltUnterhaltung {
    pub id: String,
    pub titel: Option<String>,
    pub bereich: Option<String>,
    pub geraet_lokal: bool,
    pub modell_lokal: bool,
    /// In Reihenfolge der Entstehung.
    pub eintraege: Vec<AltEintrag>,
}

/// Migrationsmarke je Geraet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Migrationsmarke {
    pub version: u32,
    pub unterhaltungen: BTreeSet<String>,
}

/// Ergebnis eines Migrationslaufs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Migrationsbericht {
    pub unterhaltungen_neu: usize,
    pub unterhaltungen_bekannt: usize,
    pub beitraege_neu: usize,
    pub beitraege_bekannt: usize,
    /// Gleiche ID, anderer Inhalt: lokal Vorhandenes bleibt, der Fall wird gemeldet.
    pub konflikte: usize,
    /// Wegen eines Grabsteins nicht (wieder) angelegt.
    pub geloescht: usize,
}

fn angabe(wert: &Option<String>) -> String {
    wert.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(UNBEKANNT)
        .to_owned()
}

impl Abgleich {
    pub fn migrationsmarke(&self) -> &Migrationsmarke {
        &self.marke
    }

    /// Uebernimmt bestehende lokale Unterhaltungen. `datenbestand_id` ist die stabile, in der lokalen
    /// Datenbank gespeicherte Kennung dieses Datenbestands. Siehe Moduldoku.
    pub fn migrieren(
        &mut self,
        datenbestand_id: &str,
        alt: &[AltUnterhaltung],
    ) -> Migrationsbericht {
        let mut bericht = Migrationsbericht::default();
        for a in alt {
            let r = self.ref_aus_lokaler_id(datenbestand_id, &a.id);
            if self.grabsteine.unterhaltung_geloescht(&r) {
                bericht.geloescht += 1;
                continue;
            }
            if self.unterhaltungen.contains_key(&r) {
                bericht.unterhaltungen_bekannt += 1;
            } else {
                let kopf = Kopf {
                    titel: angabe(&a.titel),
                    bereich: angabe(&a.bereich),
                    modellwunsch: UNBEKANNT.to_owned(),
                    angeheftet: false,
                };
                let einschraenkung = Einschraenkung {
                    geraet_lokal: a.geraet_lokal,
                    modell_lokal: a.modell_lokal,
                };
                self.unterhaltungen
                    .insert(r.clone(), Unterhaltung::neu(kopf, einschraenkung));
                bericht.unterhaltungen_neu += 1;
            }
            let mut vorheriger: Option<&str> = None;
            for e in &a.eintraege {
                let oid = self.beitrag_objekt_id(&r, &e.id);
                if self.grabsteine.schlaegt(&r, &oid) {
                    bericht.geloescht += 1;
                    vorheriger = Some(&e.id);
                    continue;
                }
                let beitrag = Beitrag {
                    id: e.id.clone(),
                    eltern_id: e
                        .eltern_id
                        .clone()
                        .or_else(|| vorheriger.map(str::to_owned)),
                    lease_generation: 0,
                    autor: angabe(&e.autor),
                    modus: e.modus.unwrap_or(Modus::Unbekannt),
                    text: e.text.clone(),
                    modell: angabe(&e.modell),
                    quellen: Vec::new(),
                    anzeigezeit: angabe(&e.zeit),
                    datenklasse: e.datenklasse.unwrap_or(Datenklasse::Restricted),
                };
                vorheriger = Some(&e.id);
                let Some(u) = self.unterhaltungen.get_mut(&r) else {
                    continue;
                };
                match u.verlauf.einfuegen(beitrag) {
                    Einfuegen::Neu => bericht.beitraege_neu += 1,
                    Einfuegen::Doppelt => bericht.beitraege_bekannt += 1,
                    Einfuegen::Konflikt => bericht.konflikte += 1,
                    Einfuegen::Geloescht => bericht.geloescht += 1,
                }
            }
            self.marke.unterhaltungen.insert(a.id.clone());
        }
        self.marke.version = MIGRATIONSVERSION;
        bericht
    }
}
