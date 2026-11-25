//! Deterministische Bewertung nach Konzept §5. Keine Zeitzonenannahmen:
//! Kernzeit wird vom Adapter fuer das ganze Zeitfenster bestimmt.

use super::modell::{normalisiere_adresse, Antwort, Quelle, Umfrage, Wert};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Verfuegbarkeit {
    Frei,
    Vorlaeufig,
    Belegt,
    Abwesend,
    Anderswo,
    Unbekannt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct KalenderWert {
    pub person: String,
    pub vorschlag_nr: u8,
    /// Fuer das ganze Zeitfenster aggregiert, nicht nur dessen Beginn.
    pub wert: Verfuegbarkeit,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FreiBelegt {
    pub eintraege: Vec<KalenderWert>,
    /// Nur sicher bekannte Kernzeiten der einladenden Person (+1 pro Vorschlag).
    pub kernzeit_vorschlaege: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Rang {
    pub nr: u8,
    pub punkte: i32,
    pub ausgeschlossen: bool,
    pub begruendung: String,
}

/// Juengste Stimme je Person/Vorschlag. Eine neuere unsichere Stimme entwertet
/// die alte, statt sie unbemerkt weiter zu zaehlen. Gleichzeitiger Widerspruch
/// bleibt offen, unabhaengig von der Eingabereihenfolge.
fn letzte_antwort<'a>(antworten: impl Iterator<Item = &'a Antwort>) -> Option<Wert> {
    let mut zeit = None;
    let mut wert = Wert::Offen;
    for a in antworten {
        let w = if a.sicher { a.wert } else { Wert::Offen };
        match zeit {
            Some(t) if a.eingegangen < t => {}
            Some(t) if a.eingegangen == t => {
                if w != wert {
                    wert = Wert::Offen;
                }
            }
            _ => {
                zeit = Some(a.eingegangen);
                wert = w;
            }
        }
    }
    zeit.map(|_| wert)
}

fn kalenderwert<'a>(eintraege: impl Iterator<Item = &'a KalenderWert>) -> Wert {
    let mut wert = None;
    for k in eintraege {
        let w = match k.wert {
            Verfuegbarkeit::Frei | Verfuegbarkeit::Anderswo => Wert::Passt,
            Verfuegbarkeit::Vorlaeufig => Wert::Notfalls,
            Verfuegbarkeit::Belegt | Verfuegbarkeit::Abwesend => Wert::PasstNicht,
            Verfuegbarkeit::Unbekannt => Wert::Offen,
        };
        if wert.is_some_and(|v| v != w) {
            return Wert::Offen;
        }
        wert = Some(w);
    }
    wert.unwrap_or(Wert::Offen)
}

/// Explizite Antworten haben Vorrang vor Kalenderdaten. Externe werden nie
/// anhand von Kalenderdaten bewertet. Ungueltiges Modell liefert keine Rangliste.
pub fn bewerte(umfrage: &Umfrage, antworten: &[Antwort], frei_belegt: &FreiBelegt) -> Vec<Rang> {
    if umfrage.validiere().is_err() {
        return vec![];
    }
    let mut raenge = Vec::new();
    for v in &umfrage.vorschlaege {
        let mut zusage_punkte = 0;
        let mut notfalls = 0;
        let mut pflicht_koennen = 0;
        let mut pflicht_fehlen = 0;
        let mut optional_fehlen = 0;
        let mut offen = 0;
        for p in &umfrage.teilnehmende {
            let adresse = normalisiere_adresse(&p.adresse);
            let wert = letzte_antwort(antworten.iter().filter(|a| {
                a.vorschlag_nr == v.nr
                    && normalisiere_adresse(&a.person) == adresse
                    && a.quelle != Quelle::Kalender
            }))
            .unwrap_or_else(|| {
                if p.intern {
                    letzte_antwort(antworten.iter().filter(|a| {
                        a.vorschlag_nr == v.nr
                            && normalisiere_adresse(&a.person) == adresse
                            && a.quelle == Quelle::Kalender
                    }))
                    .unwrap_or_else(|| {
                        kalenderwert(frei_belegt.eintraege.iter().filter(|k| {
                            k.vorschlag_nr == v.nr && normalisiere_adresse(&k.person) == adresse
                        }))
                    })
                } else {
                    Wert::Offen
                }
            });
            match wert {
                Wert::Passt => {
                    zusage_punkte += if p.pflicht { 6 } else { 3 };
                    if p.pflicht {
                        pflicht_koennen += 1;
                    }
                }
                Wert::Notfalls => {
                    notfalls += 1;
                    if p.pflicht {
                        pflicht_koennen += 1;
                    }
                }
                Wert::PasstNicht => {
                    if p.pflicht {
                        pflicht_fehlen += 1;
                    } else {
                        optional_fehlen += 1;
                    }
                }
                Wert::Offen => offen += 1,
            }
        }
        let kernzeit = i32::from(frei_belegt.kernzeit_vorschlaege.contains(&v.nr));
        let punkte = zusage_punkte + notfalls + kernzeit;
        let ausgeschlossen = pflicht_fehlen > 0;
        let begruendung = format!(
            "{pflicht_koennen} Pflicht-Teilnehmende können (einschließlich notfalls), {pflicht_fehlen} fehlen; \
             {optional_fehlen} optionale Teilnehmende fehlen, {offen} offen (0 Punkte); \
             Zusagen (Pflicht je 6, optional je 3), notfalls (je 1) und Kernzeit (0 oder 1) \
             ergeben {zusage_punkte} + {notfalls} + {kernzeit} = {punkte} Punkte. {}",
            if ausgeschlossen { "Wegen fehlender Pflicht-Teilnehmender ausgeschlossen." }
            else { "Kein bekannter Pflicht-Konflikt; offene Antworten bleiben unsicher." });
        raenge.push((
            v.start,
            Rang {
                nr: v.nr,
                punkte,
                ausgeschlossen,
                begruendung,
            },
        ));
    }
    raenge.sort_by(|(ta, a), (tb, b)| {
        a.ausgeschlossen
            .cmp(&b.ausgeschlossen)
            .then_with(|| b.punkte.cmp(&a.punkte))
            .then_with(|| ta.cmp(tb))
            .then_with(|| a.nr.cmp(&b.nr))
    });
    raenge.into_iter().map(|(_, r)| r).collect()
}
