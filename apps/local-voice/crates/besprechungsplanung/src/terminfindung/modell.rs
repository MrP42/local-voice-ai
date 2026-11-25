//! Datentypen. Zeitpunkte sind wie im Kalender der App Millisekunden UTC.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Entwurf,
    Offen,
    Entschieden,
    Abgebrochen,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Person {
    pub adresse: String,
    pub name: String,
    pub intern: bool,
    pub pflicht: bool,
}

/// Lokale Anzeigezeit des Vorschlags, vom Adapter in der Umfrage-Zeitzone
/// berechnet und gegen UTC geprueft. Ohne diese Angabe raet der Parser keine
/// Zeitzone aus UTC. Der Versatz gilt je Vorschlag (Sommer-/Winterzeit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Ortszeit {
    pub jahr: u16,
    pub monat: u8,
    pub tag: u8,
    /// ISO-Wochentag, Montag = 1, Sonntag = 7.
    pub wochentag: u8,
    pub stunde: u8,
    pub minute: u8,
    /// Ortszeit minus UTC in Minuten. Pflichtangabe, kein stiller UTC-Default.
    pub utc_versatz_min: i16,
}

impl Ortszeit {
    fn gueltig(&self) -> bool {
        let schaltjahr = self.jahr.is_multiple_of(4)
            && (!self.jahr.is_multiple_of(100) || self.jahr.is_multiple_of(400));
        let tage = match self.monat {
            2 => {
                if schaltjahr {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => return false,
        };
        let versatz = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let jahr = i32::from(self.jahr) - i32::from(self.monat < 3);
        let sonntag_null = (jahr + jahr / 4 - jahr / 100
            + jahr / 400
            + versatz[usize::from(self.monat - 1)]
            + i32::from(self.tag))
        .rem_euclid(7);
        let wochentag = (sonntag_null + 6) % 7 + 1;
        (1..=9999).contains(&self.jahr)
            && (1..=tage).contains(&self.tag)
            && i32::from(self.wochentag) == wochentag
            && self.stunde < 24
            && self.minute < 60
            && (-840..=840).contains(&self.utc_versatz_min)
    }

    fn passt_zu_start(&self, start: i64) -> bool {
        if !self.gueltig() {
            return false;
        }
        let jahr = i64::from(self.jahr) - 1;
        let schaltjahr = self.jahr.is_multiple_of(4)
            && (!self.jahr.is_multiple_of(100) || self.jahr.is_multiple_of(400));
        let monate: [i64; 12] = [
            31,
            if schaltjahr { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        let tage = jahr * 365 + jahr / 4 - jahr / 100 + jahr / 400
            + monate[..usize::from(self.monat - 1)].iter().sum::<i64>()
            + i64::from(self.tag)
            - 1
            - 719_162;
        // Alle Operanden sind durch gueltig() begrenzt. Exakter Vergleich:
        // Sekunden/Millisekunden ohne Gegenstueck in Ortszeit nicht verbergen.
        let minuten = tage * 1440 + i64::from(self.stunde) * 60 + i64::from(self.minute)
            - i64::from(self.utc_versatz_min);
        minuten * 60_000 == start
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Vorschlag {
    pub nr: u8,
    pub start: i64,
    pub ende: i64,
    #[serde(default)]
    pub ortszeit: Option<Ortszeit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Umfrage {
    pub id: String,
    pub plan_id: Option<String>,
    pub titel: String,
    pub titel_an_externe: bool,
    pub dauer_min: u32,
    pub frist: i64,
    pub status: Status,
    pub vorschlaege: Vec<Vorschlag>,
    pub teilnehmende: Vec<Person>,
}

impl Umfrage {
    /// Prueft auch Entwuerfe mit einem Vorschlag; die UI fordert beim Versand 2-8.
    pub fn validiere(&self) -> Result<(), &'static str> {
        if !gueltige_id(&self.id) {
            return Err("umfrage_id");
        }
        if self.titel.len() > 800 || self.dauer_min == 0 || self.dauer_min > 1440 {
            return Err("umfrage_daten");
        }
        if self.vorschlaege.is_empty() || self.vorschlaege.len() > 8 {
            return Err("vorschlaege_anzahl");
        }
        let mut nummern = std::collections::BTreeSet::new();
        for v in &self.vorschlaege {
            if v.nr == 0
                || v.nr > 8
                || !nummern.insert(v.nr)
                || v.ende.checked_sub(v.start) != Some(i64::from(self.dauer_min) * 60_000)
                || v.ortszeit.is_some_and(|z| !z.passt_zu_start(v.start))
            {
                return Err("vorschlag_daten");
            }
        }
        if self.teilnehmende.len() > 256 {
            return Err("teilnehmende_anzahl");
        }
        let mut adressen = std::collections::BTreeSet::new();
        for p in &self.teilnehmende {
            if !gueltige_adresse(&p.adresse) || !adressen.insert(normalisiere_adresse(&p.adresse)) {
                return Err("teilnehmende_adresse");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Wert {
    Passt,
    Notfalls,
    PasstNicht,
    Offen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Quelle {
    Knopf,
    Auswahl,
    Freitext,
    Kalender,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Antwort {
    /// Normalisierte E-Mail-Adresse; kein Anzeigename.
    pub person: String,
    pub vorschlag_nr: u8,
    pub wert: Wert,
    pub quelle: Quelle,
    /// Bei Freitext erst nach ausdruecklicher Bestaetigung setzen.
    pub sicher: bool,
    pub eingegangen: i64,
}

pub fn gueltige_id(id: &str) -> bool {
    id.len() == 6 && id.starts_with("T-") && gueltiger_kurzcode(&id[2..])
}

pub(crate) fn gueltiger_kurzcode(code: &str) -> bool {
    code.len() == 4
        && code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

pub fn normalisiere_adresse(adresse: &str) -> String {
    adresse.trim().to_lowercase()
}

/// Bewusst nur einzelne einfache addr-spec-Adressen, keine Anzeigenamen,
/// Verteiler, URI-Parameter oder Header. Internationale Domains vorher als IDNA.
pub(crate) fn gueltige_adresse(adresse: &str) -> bool {
    let a = adresse.trim();
    let Some((lokal, domain)) = a.split_once('@') else {
        return false;
    };
    a.len() <= 254
        && !lokal.is_empty()
        && !domain.is_empty()
        && !lokal.starts_with('.')
        && !lokal.ends_with('.')
        && !lokal.contains("..")
        && lokal
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&b))
        && domain.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
