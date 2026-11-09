//! D3: Sicht der KI auf den Bestand und das einzige schreibende Werkzeug (Spec §5, Epic lva#395).
//!
//! * [`Ablage::ki_bestand`]: frische, nur lesende Momentaufnahme (Freigabe, Ordner, Dateien mit der
//!   Fassung, die die KI nutzen darf). Grundlage für Abruf und Widerrufsprüfung in [`super::super::ki`].
//! * [`Ablage::entwurf_anlegen`]: `ablage_entwurf_anlegen` (§5.5). Neue Datei in „KI-Entwürfe“ oder in
//!   einem Zielordner mit `zielordner_ki`; Klasse = Maximum aus Zielordner und allen Quellen; ist eine
//!   Quelle „nur dieses Gerät“, landet der Entwurf in „KI-Entwürfe (nur dieses Gerät)“. Klasse und
//!   „nur dieses Gerät“ stehen schon im Journaleintrag, damit ein Wiederanlauf sie nie verliert. Es gibt
//!   kein Überschreiben, Verschieben oder Löschen.
//! * [`Ablage::cloud_ausnahme_setzen`]: Cloud-Ausnahme je (Ordner, Anbieter), nur für `confidential`.
//! * [`Ablage::verweise`]: Leserecht und Klasse je Ablage-Eintrag für die Projekt-Herkunft.
//!
//! Keine Namen oder Inhalte in Log oder Fehlertext.

use super::{
    aktuelle_fassung, bereinigen, erwartete_textrev, nicht_gefunden, vergleichsschluessel,
    AblageKollisionswahl, AblageLage, Eltern, Stand,
};
use crate::ablage::freigabe;
use crate::ablage::zusammenfuehren::{self as zf, Klasse, Register};
use crate::ablage::{id_gueltig, ordner_id_pruefen, sperren, Fehler, Res, PRAEFIX_DATEI};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use super::Ablage;

/// Name des Ordners für KI-Entwürfe (oberste Ebene).
pub const ENTWURF_ORDNER: &str = "KI-Entwürfe";
/// Name des Ordners für KI-Entwürfe aus Quellen „nur dieses Gerät“ (oberste Ebene, selbst nur Gerät).
pub const ENTWURF_ORDNER_GERAET: &str = "KI-Entwürfe (nur dieses Gerät)";
/// Größter Inhalt eines Entwurfs (Markdown).
pub const MAX_ENTWURF: usize = 1024 * 1024;

/// Eine Datei aus Sicht der KI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KiDatei {
    pub dat: String,
    pub name: String,
    pub ordner: String,
    /// Ordnerkette (IDs) vom Ordner der Datei bis zur obersten Ebene.
    pub kette: Vec<String>,
    /// Gespeicherte Elternkette unter „Wiederhergestellt“ (zusätzliche Sperre, sonst leer).
    pub zusatz: Vec<String>,
    pub klasse: Klasse,
    pub nur_geraet: bool,
    /// Nicht im Papierkorb (auch nicht über einen Ordner).
    pub lebend: bool,
    /// Mehr als ein Kopf.
    pub konflikt: bool,
    /// Fassung, die die KI nutzt: der einzige Kopf; im Konflikt die jüngste gemeinsame Vorgängerin;
    /// ohne gemeinsame Fassung `None` (die Datei wird nicht genutzt).
    pub ki_fas: Option<String>,
    /// `textrev` (Stand `ok`) der `ki_fas`; `None` = (noch) kein Text.
    pub ki_textrev: Option<String>,
    /// Das Format der `ki_fas` hat eine Textfassung.
    pub mit_text: bool,
    /// Alle lebenden Fassungen (für die Widerrufsprüfung).
    pub fassungen: BTreeSet<String>,
}

/// Ein Ordner aus Sicht der KI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KiOrdner {
    pub id: String,
    pub name: String,
    pub kette: Vec<String>,
    pub lebend: bool,
    pub zielordner_ki: bool,
    pub klasse: Klasse,
    pub nur_geraet: bool,
}

/// Momentaufnahme für Abruf und Widerrufsprüfung (frisch gelesen, nie zwischengespeichert).
#[derive(Debug, Clone)]
pub struct KiBestand {
    pub freigabe: freigabe::Lage,
    pub ordner: BTreeMap<String, KiOrdner>,
    pub dateien: BTreeMap<String, KiDatei>,
}

/// Ergebnis von `ablage_entwurf_anlegen`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageEntwurf {
    pub dat: String,
    pub fas: String,
    pub ordner: String,
    pub name: String,
    /// Wirksame Klasse des Entwurfs (`public` … `personal`).
    pub klasse: String,
    pub nur_geraet: bool,
}

/// `textrev` (Stand `ok`) einer Fassung: die des aktuellen Extraktors, sonst die letzte brauchbare
/// ältere (dieselbe Regel wie der Index).
fn ok_textrev(f: &zf::Fassung) -> Option<String> {
    let erwartet = erwartete_textrev(f);
    f.texte
        .iter()
        .find(|t| Some(&t.textrev) == erwartet.as_ref() && t.stand == "ok")
        .or_else(|| f.texte.iter().rev().find(|t| t.stand == "ok"))
        .map(|t| t.textrev.clone())
}

/// Klasse und „nur dieses Gerät“ eines Entwurfs: Maximum aus Zielordner und allen Quellen. Eine
/// unbekannte Quelle oder Klasse ist ein Fehler (nie zu niedrig einstufen).
pub(super) fn entwurf_regel(
    st: &Stand,
    eltern: &BTreeMap<String, Eltern>,
    ordner: &str,
    quellen: &[String],
) -> Res<(Klasse, bool)> {
    let mut klasse = zf::wirksame_klasse(&st.ordner, eltern, ordner);
    for d in quelldateien(st, quellen)? {
        klasse = klasse.max(zf::wirksame_klasse_datei(&st.ordner, eltern, d));
    }
    let nur = quellen_nur_geraet(st, eltern, quellen)?;
    if klasse == Klasse::Unbekannt {
        return Err(Fehler::neu(
            "ablage.entwurf_klasse",
            "Die Datenklasse des Entwurfs ist unklar; nichts angelegt.",
        ));
    }
    Ok((klasse, nur))
}

fn quelldateien<'a>(st: &'a Stand, quellen: &[String]) -> Res<Vec<&'a zf::Datei>> {
    let (offen, _) = st.offen_geloescht();
    quellen
        .iter()
        .map(|q| {
            st.dateien
                .get(q)
                .filter(|_| !offen.contains(q.as_str()))
                .ok_or_else(|| {
                    Fehler::neu(
                        "ablage.entwurf_quelle",
                        "Eine Quelle des Entwurfs ist nicht (mehr) vorhanden; nichts angelegt.",
                    )
                })
        })
        .collect()
}

/// Ist eine der Quellen „nur dieses Gerät“?
fn quellen_nur_geraet(
    st: &Stand,
    eltern: &BTreeMap<String, Eltern>,
    quellen: &[String],
) -> Res<bool> {
    Ok(quelldateien(st, quellen)?
        .iter()
        .any(|d| d.nur_geraet_min || zf::wirksam_nur_geraet(&st.ordner, eltern, &d.ordner.wert)))
}

impl Ablage {
    /// Frische Momentaufnahme für die KI (nur lesend; legt nie einen Stand an).
    pub fn ki_bestand(&self) -> Res<KiBestand> {
        let _g = sperren();
        let freigabe = freigabe::laden(&self.dir);
        let (st, frisch) = self.lage().map_err(AblageLage::fehler)?;
        if frisch {
            return Ok(KiBestand {
                freigabe,
                ordner: BTreeMap::new(),
                dateien: BTreeMap::new(),
            });
        }
        let eltern = st.eltern();
        let (offen_dat, offen_ord) = st.offen_geloescht();
        let ordner = st
            .ordner
            .values()
            .filter(|o| !offen_ord.contains(o.id.as_str()))
            .map(|o| {
                (
                    o.id.clone(),
                    KiOrdner {
                        id: o.id.clone(),
                        name: o.name.wert.clone(),
                        kette: zf::kette(&eltern, &o.id),
                        lebend: !st.ordner_im_korb(&eltern, &o.id),
                        zielordner_ki: o.zielordner_ki.wert,
                        klasse: zf::wirksame_klasse(&st.ordner, &eltern, &o.id),
                        nur_geraet: zf::wirksam_nur_geraet(&st.ordner, &eltern, &o.id),
                    },
                )
            })
            .collect();
        let dateien = st
            .dateien
            .values()
            .filter(|d| !offen_dat.contains(d.id.as_str()))
            .map(|d| {
                let konflikt = zf::konflikt(d);
                let ki_fas = if konflikt {
                    zf::gemeinsame_vorgaengerin(d)
                } else {
                    aktuelle_fassung(d).map(|f| f.fas.clone())
                };
                let f = ki_fas
                    .as_ref()
                    .and_then(|id| d.fassungen.iter().find(|f| &f.fas == id));
                (
                    d.id.clone(),
                    KiDatei {
                        dat: d.id.clone(),
                        name: d.name.wert.clone(),
                        ordner: d.ordner.wert.clone(),
                        kette: zf::kette(&eltern, &d.ordner.wert),
                        zusatz: zf::zusatzkette(&st.ordner, &eltern, &d.ordner.wert),
                        klasse: zf::wirksame_klasse_datei(&st.ordner, &eltern, d),
                        nur_geraet: d.nur_geraet_min
                            || zf::wirksam_nur_geraet(&st.ordner, &eltern, &d.ordner.wert),
                        lebend: st.datei_lebt(&eltern, d),
                        konflikt,
                        ki_textrev: f.and_then(ok_textrev),
                        mit_text: f.is_some_and(|f| erwartete_textrev(f).is_some()),
                        ki_fas,
                        fassungen: d.fassungen.iter().map(|f| f.fas.clone()).collect(),
                    },
                )
            })
            .collect();
        Ok(KiBestand {
            freigabe,
            ordner,
            dateien,
        })
    }

    /// Leserecht und Klasse aller Ablage-Einträge (Ordner und Dateien) für die Projekt-Herkunft: `(id,
    /// lebend, klasse)`. Nur lesend; eine fehlende Ablage liefert nichts.
    pub fn verweise(&self) -> Res<Vec<(String, bool, Klasse)>> {
        let b = self.ki_bestand()?;
        Ok(b.ordner
            .values()
            .map(|o| (o.id.clone(), o.lebend, o.klasse))
            .chain(
                b.dateien
                    .values()
                    .map(|d| (d.dat.clone(), d.lebend, d.klasse)),
            )
            .collect())
    }

    /// Cloud-Ausnahme je (Ordner, Anbieter) erteilen oder entziehen. Erteilen nur für einen Ordner mit
    /// wirksamer Klasse `confidential` (Spec §5.1).
    pub fn cloud_ausnahme_setzen(&self, ordner: &str, anbieter: &str, an: bool) -> Res<()> {
        ordner_id_pruefen(ordner)?;
        let _g = sperren();
        let st = self.laden()?;
        let eltern = st.eltern();
        self.ordner_lebend(&st, &eltern, ordner)?;
        if an && zf::wirksame_klasse(&st.ordner, &eltern, ordner) != Klasse::Confidential {
            return Err(Fehler::neu(
                "ablage.ausnahme_klasse",
                "Cloud-Ausnahmen gibt es nur für vertrauliche Ordner.",
            ));
        }
        freigabe::ausnahme_setzen(&self.dir, ordner, anbieter, an).map(|_| ())
    }

    /// Findet oder legt einen Ordner der obersten Ebene für Entwürfe an (lebend, nicht im Papierkorb).
    /// `nur_geraet`: der Ordner trägt „nur dieses Gerät“ (wird gesetzt, falls nicht).
    fn entwurfsordner(&self, st: &mut Stand, nur_geraet: bool) -> Res<(String, bool)> {
        let name = if nur_geraet {
            ENTWURF_ORDNER_GERAET
        } else {
            ENTWURF_ORDNER
        };
        let eltern = st.eltern();
        let schluessel = vergleichsschluessel(name);
        let gefunden = st
            .kinder(&eltern, None)
            .into_iter()
            .find(|o| {
                o.papierkorb.wert.is_none() && vergleichsschluessel(&o.name.wert) == schluessel
            })
            .map(|o| o.id.clone());
        let (z, g) = (st.zaehler(), st.geraet().to_string());
        let (id, mut geaendert) = match gefunden {
            Some(id) => (id, false),
            None => {
                let o = super::neuer_ordner(&g, None, name, z);
                let id = o.id.clone();
                st.ordner.insert(id.clone(), o);
                (id, true)
            }
        };
        if nur_geraet {
            let o = st.ordner.get_mut(&id).ok_or_else(nicht_gefunden)?;
            if !o.nur_geraet.wert {
                o.nur_geraet = Register::neu(true, z, &g);
                geaendert = true;
            }
        }
        Ok((id, geaendert))
    }

    /// `ablage_entwurf_anlegen(ordner, name, inhalt_md, quellen)` (Spec §5.5). `ordner`: Zielordner mit
    /// `zielordner_ki`; ohne Angabe „KI-Entwürfe“. `quellen`: Kennungen der Quelldateien (`dat_…`).
    /// Nie überschreibend: bei gleichem Namen entsteht ein freier Name.
    pub fn entwurf_anlegen(
        &self,
        ordner: Option<&str>,
        name: &str,
        inhalt_md: &str,
        quellen: &[String],
    ) -> Res<AblageEntwurf> {
        if inhalt_md.len() > MAX_ENTWURF {
            return Err(Fehler::neu(
                "ablage.entwurf_zu_gross",
                "Der Entwurf ist zu groß; nichts angelegt.",
            ));
        }
        if let Some(o) = ordner {
            ordner_id_pruefen(o)?;
        }
        let mut quellen: Vec<String> = quellen.to_vec();
        quellen.sort();
        quellen.dedup();
        if quellen.iter().any(|q| !id_gueltig(PRAEFIX_DATEI, q)) {
            return Err(Fehler::ungueltig("Kennung einer Quelle ungültig."));
        }
        let mut name = bereinigen(name)?;
        if !name.to_lowercase().ends_with(".md") {
            name = bereinigen(&format!("{name}.md"))?;
        }
        // Schritt 1: Zielordner bestimmen (und Entwurfsordner bei Bedarf anlegen).
        let ziel = {
            let _g = sperren();
            let mut st = self.laden()?;
            let eltern = st.eltern();
            let nur = quellen_nur_geraet(&st, &eltern, &quellen)?;
            match ordner {
                Some(o) if !nur || zf::wirksam_nur_geraet(&st.ordner, &eltern, o) => {
                    self.ordner_lebend(&st, &eltern, o)?;
                    if !st.ordner.get(o).is_some_and(|x| x.zielordner_ki.wert) {
                        return Err(Fehler::neu(
                            "ablage.kein_zielordner",
                            "Dieser Ordner ist kein Zielordner für die KI; nichts angelegt.",
                        ));
                    }
                    o.to_string()
                }
                _ => {
                    let (id, geaendert) = self.entwurfsordner(&mut st, nur)?;
                    if geaendert {
                        self.sichern(&mut st)?;
                    }
                    id
                }
            }
        };
        // Schritt 2: über eine eigene Zwischendatei sicher ablegen (Klasse unter der Sperre neu bestimmt).
        let arbeit = self.dir.join(format!(".entwurf-{}", ulid::Ulid::new()));
        fs::create_dir(&arbeit).map_err(|_| Fehler::speicher("Entwurf nicht anlegbar."))?;
        let quelle = arbeit.join(&name);
        let erg = (|| {
            super::super::schreibe_atomar(&quelle, inhalt_md.as_bytes())?;
            self.datei_ablegen(
                &quelle,
                &ziel,
                Some(AblageKollisionswahl::BeideBehalten),
                Some(&quellen),
            )
        })();
        let _ = fs::remove_dir_all(&arbeit);
        let zeile = erg?;
        let (dat, fas) = match (zeile.dat, zeile.fas) {
            (Some(d), Some(f)) => (d, f),
            _ => return Err(Fehler::speicher("Entwurf nicht ablegbar.")),
        };
        let b = self.ki_bestand()?;
        let d = b.dateien.get(&dat).ok_or_else(nicht_gefunden)?;
        Ok(AblageEntwurf {
            dat: dat.clone(),
            fas,
            ordner: d.ordner.clone(),
            name: zeile.name.unwrap_or_default(),
            klasse: d.klasse.text().into(),
            nur_geraet: d.nur_geraet,
        })
    }
}
